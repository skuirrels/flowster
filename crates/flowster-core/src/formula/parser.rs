//! ShapeSheet formula lexer and parser (SPEC §7.3), universal syntax:
//! `.` decimal separator and `,` list separator, as stored in VSDX `F` attributes.

use super::units::unit_factor;

#[derive(Clone, Debug, PartialEq)]
pub enum SheetRef {
    /// `Sheet.5!` or a name with id such as `Process.12!`.
    Id(u32),
    ThePage,
    TheDoc,
    /// `Pages[…]!`, `Masters[…]!`, `Styles[…]!`: parsed, not yet resolvable.
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellRef {
    pub sheet: Option<SheetRef>,
    /// Cell reference as written, e.g. `Width`, `Geometry1.X3`, `Prop.Cost.Label`.
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrLit {
    Ref,
    Name,
    Value,
    Div0,
    Na,
    Num,
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A number already converted to internal units (inches, radians, days).
    Num(f64),
    Str(String),
    Bool(bool),
    Err(ErrLit),
    Ref(CellRef),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// Function name upper-cased.
    Call(String, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub pos: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.message, self.pos)
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Err(ErrLit),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Bang,
    Percent,
    Eof,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c >= 0x80
}

impl<'a> Lexer<'a> {
    fn err(&self, message: &str) -> ParseError {
        ParseError { pos: self.pos, message: message.to_owned() }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn next(&mut self) -> Result<(usize, Tok), ParseError> {
        self.skip_ws();
        let start = self.pos;
        let Some(&c) = self.src.get(self.pos) else { return Ok((start, Tok::Eof)) };
        let tok = match c {
            b'0'..=b'9' | b'.' if c != b'.' || self.src.get(self.pos + 1).is_some_and(u8::is_ascii_digit) => {
                let mut end = self.pos;
                while end < self.src.len() && (self.src[end].is_ascii_digit() || self.src[end] == b'.') {
                    end += 1;
                }
                // Exponent: 1E-05, 2.5e3.
                if end < self.src.len() && (self.src[end] == b'e' || self.src[end] == b'E') {
                    let mut e = end + 1;
                    if e < self.src.len() && (self.src[e] == b'+' || self.src[e] == b'-') {
                        e += 1;
                    }
                    if e < self.src.len() && self.src[e].is_ascii_digit() {
                        while e < self.src.len() && self.src[e].is_ascii_digit() {
                            e += 1;
                        }
                        end = e;
                    }
                }
                let text = std::str::from_utf8(&self.src[self.pos..end]).unwrap_or("");
                let n = text.parse::<f64>().map_err(|_| self.err("bad number"))?;
                self.pos = end;
                Tok::Num(n)
            }
            b'"' => {
                let mut s = Vec::new();
                self.pos += 1;
                loop {
                    match self.src.get(self.pos) {
                        None => return Err(self.err("unterminated string")),
                        Some(b'"') if self.src.get(self.pos + 1) == Some(&b'"') => {
                            s.push(b'"');
                            self.pos += 2;
                        }
                        Some(b'"') => {
                            self.pos += 1;
                            break;
                        }
                        Some(&b) => {
                            s.push(b);
                            self.pos += 1;
                        }
                    }
                }
                Tok::Str(String::from_utf8_lossy(&s).into_owned())
            }
            b'#' => {
                let rest = std::str::from_utf8(&self.src[self.pos..]).unwrap_or("").to_ascii_uppercase();
                let table = [
                    ("#REF!", ErrLit::Ref),
                    ("#NAME?", ErrLit::Name),
                    ("#VALUE!", ErrLit::Value),
                    ("#DIV/0!", ErrLit::Div0),
                    ("#N/A", ErrLit::Na),
                    ("#NUM!", ErrLit::Num),
                    ("#NULL!", ErrLit::Null),
                ];
                let (lit, kind) =
                    table.iter().find(|(l, _)| rest.starts_with(l)).ok_or_else(|| self.err("unknown error literal"))?;
                self.pos += lit.len();
                Tok::Err(*kind)
            }
            c if is_ident_start(c) => {
                let mut end = self.pos;
                while end < self.src.len() && is_ident(self.src[end]) {
                    end += 1;
                }
                // Bracketed sheet names: Pages[Page-1], Masters["Name"].
                if end < self.src.len() && self.src[end] == b'[' {
                    while end < self.src.len() && self.src[end] != b']' {
                        end += 1;
                    }
                    end = (end + 1).min(self.src.len());
                }
                let text = String::from_utf8_lossy(&self.src[self.pos..end]).into_owned();
                self.pos = end;
                Tok::Ident(text)
            }
            b'(' => {
                self.pos += 1;
                Tok::LParen
            }
            b')' => {
                self.pos += 1;
                Tok::RParen
            }
            b',' | b';' => {
                self.pos += 1;
                Tok::Comma
            }
            b'!' => {
                self.pos += 1;
                Tok::Bang
            }
            b'%' => {
                self.pos += 1;
                Tok::Percent
            }
            _ => {
                let two = self.src.get(self.pos..self.pos + 2);
                let op: &'static str = match two {
                    Some(b"<>") => "<>",
                    Some(b"<=") => "<=",
                    Some(b">=") => ">=",
                    _ => match c {
                        b'+' => "+",
                        b'-' => "-",
                        b'*' => "*",
                        b'/' => "/",
                        b'^' => "^",
                        b'&' => "&",
                        b'=' => "=",
                        b'<' => "<",
                        b'>' => ">",
                        _ => return Err(self.err("unexpected character")),
                    },
                };
                self.pos += op.len();
                Tok::Op(op)
            }
        };
        Ok((start, tok))
    }
}

struct Parser<'a> {
    lex: Lexer<'a>,
    tok: Tok,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Result<Self, ParseError> {
        let mut lex = Lexer { src: src.as_bytes(), pos: 0 };
        let (pos, tok) = lex.next()?;
        Ok(Parser { lex, tok, pos })
    }

    fn bump(&mut self) -> Result<Tok, ParseError> {
        let (pos, next) = self.lex.next()?;
        self.pos = pos;
        Ok(std::mem::replace(&mut self.tok, next))
    }

    fn err(&self, message: &str) -> ParseError {
        ParseError { pos: self.pos, message: message.to_owned() }
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(&self.tok, Tok::Op(o) if *o == op)
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.concat()?;
        loop {
            let op = match &self.tok {
                Tok::Op("=") => BinOp::Eq,
                Tok::Op("<>") => BinOp::Ne,
                Tok::Op("<") => BinOp::Lt,
                Tok::Op(">") => BinOp::Gt,
                Tok::Op("<=") => BinOp::Le,
                Tok::Op(">=") => BinOp::Ge,
                _ => return Ok(lhs),
            };
            self.bump()?;
            let rhs = self.concat()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn concat(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.additive()?;
        while self.is_op("&") {
            self.bump()?;
            let rhs = self.additive()?;
            lhs = Expr::Bin(BinOp::Concat, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.term()?;
        loop {
            let op = if self.is_op("+") {
                BinOp::Add
            } else if self.is_op("-") {
                BinOp::Sub
            } else {
                return Ok(lhs);
            };
            self.bump()?;
            let rhs = self.term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.power()?;
        loop {
            let op = if self.is_op("*") {
                BinOp::Mul
            } else if self.is_op("/") {
                BinOp::Div
            } else {
                return Ok(lhs);
            };
            self.bump()?;
            let rhs = self.power()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn power(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        while self.is_op("^") {
            self.bump()?;
            let rhs = self.unary()?;
            lhs = Expr::Bin(BinOp::Pow, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.is_op("-") {
            self.bump()?;
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.is_op("+") {
            self.bump()?;
            return self.unary();
        }
        let mut e = self.primary()?;
        while self.tok == Tok::Percent {
            self.bump()?;
            e = Expr::Percent(Box::new(e));
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.bump()? {
            Tok::Num(n) => {
                // Optional unit: "2 in", "30 deg", "0.19685DL".
                if let Tok::Ident(u) = &self.tok
                    && let Some(f) = unit_factor(u)
                {
                    self.bump()?;
                    return Ok(Expr::Num(n * f));
                }
                Ok(Expr::Num(n))
            }
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Err(e) => Ok(Expr::Err(e)),
            Tok::LParen => {
                let e = self.expr()?;
                if self.bump()? != Tok::RParen {
                    return Err(self.err("expected )"));
                }
                Ok(e)
            }
            Tok::Ident(id) => {
                let upper = id.to_ascii_uppercase();
                if self.tok == Tok::LParen && !id.contains('.') {
                    self.bump()?;
                    let mut args = Vec::new();
                    if self.tok != Tok::RParen {
                        loop {
                            // Visio allows empty arguments: IF(x,,1).
                            if matches!(self.tok, Tok::Comma | Tok::RParen) {
                                args.push(Expr::Str(String::new()));
                            } else {
                                args.push(self.expr()?);
                            }
                            if self.tok == Tok::Comma {
                                self.bump()?;
                                continue;
                            }
                            break;
                        }
                    }
                    if self.bump()? != Tok::RParen {
                        return Err(self.err("expected ) after arguments"));
                    }
                    return Ok(Expr::Call(upper, args));
                }
                if upper == "TRUE" {
                    return Ok(Expr::Bool(true));
                }
                if upper == "FALSE" {
                    return Ok(Expr::Bool(false));
                }
                if self.tok == Tok::Bang {
                    self.bump()?;
                    let sheet = sheet_ref(&id);
                    let Tok::Ident(cell) = self.bump()? else { return Err(self.err("expected cell after !")) };
                    // Chained sheets (Pages[x]!Sheet.1!Width): keep the innermost.
                    if self.tok == Tok::Bang {
                        self.bump()?;
                        let Tok::Ident(inner) = self.bump()? else { return Err(self.err("expected cell")) };
                        return Ok(Expr::Ref(CellRef { sheet: Some(sheet_ref(&cell)), name: inner }));
                    }
                    return Ok(Expr::Ref(CellRef { sheet: Some(sheet), name: cell }));
                }
                Ok(Expr::Ref(CellRef { sheet: None, name: id }))
            }
            other => Err(self.err(&format!("unexpected {other:?}"))),
        }
    }
}

fn sheet_ref(id: &str) -> SheetRef {
    let upper = id.to_ascii_uppercase();
    match upper.as_str() {
        "THEPAGE" => return SheetRef::ThePage,
        "THEDOC" => return SheetRef::TheDoc,
        _ => {}
    }
    // Sheet.12, Process.12, Dynamic connector.3 (spaces are not identifier chars, so names are single tokens).
    if let Some((_, tail)) = id.rsplit_once('.')
        && let Ok(n) = tail.parse::<u32>()
    {
        return SheetRef::Id(n);
    }
    SheetRef::Other(id.to_owned())
}

/// Parses a formula (with or without a leading `=`).
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let src = src.trim();
    let src = src.strip_prefix('=').unwrap_or(src);
    let mut p = Parser::new(src)?;
    let e = p.expr()?;
    if p.tok != Tok::Eof {
        return Err(p.err("unexpected trailing input"));
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64) -> Box<Expr> {
        Box::new(Expr::Num(x))
    }

    #[test]
    fn precedence_and_units() {
        assert_eq!(
            parse("1+2*3").unwrap(),
            Expr::Bin(BinOp::Add, n(1.0), Box::new(Expr::Bin(BinOp::Mul, n(2.0), n(3.0))))
        );
        // Unary minus binds tighter than ^ (Visio/Excel): -2^2 = 4.
        assert_eq!(parse("-2^2").unwrap(), Expr::Bin(BinOp::Pow, Box::new(Expr::Neg(n(2.0))), n(2.0)));
        // Same rounding as Visio, which also reports 0.9999999999999999 here.
        assert_eq!(parse("2.54 cm").unwrap(), Expr::Num(2.54 * (1.0 / 2.54)));
        assert_eq!(parse("50%").unwrap(), Expr::Percent(n(50.0)));
        assert_eq!(parse("1E-05").unwrap(), Expr::Num(1e-5));
    }

    #[test]
    fn references_and_calls() {
        assert_eq!(
            parse("Sheet.13!Width*0.5").unwrap(),
            Expr::Bin(
                BinOp::Mul,
                Box::new(Expr::Ref(CellRef { sheet: Some(SheetRef::Id(13)), name: "Width".into() })),
                n(0.5)
            )
        );
        assert_eq!(
            parse("PAR(PNT(Sheet.3!Connections.X2,Sheet.3!Connections.Y2))").unwrap(),
            Expr::Call(
                "PAR".into(),
                vec![Expr::Call(
                    "PNT".into(),
                    vec![
                        Expr::Ref(CellRef { sheet: Some(SheetRef::Id(3)), name: "Connections.X2".into() }),
                        Expr::Ref(CellRef { sheet: Some(SheetRef::Id(3)), name: "Connections.Y2".into() }),
                    ]
                )]
            )
        );
        assert_eq!(
            parse(r#"IF(Prop.Cost>0,"a""b",FALSE)"#).unwrap(),
            Expr::Call(
                "IF".into(),
                vec![
                    Expr::Bin(
                        BinOp::Gt,
                        Box::new(Expr::Ref(CellRef { sheet: None, name: "Prop.Cost".into() })),
                        n(0.0)
                    ),
                    Expr::Str("a\"b".into()),
                    Expr::Bool(false),
                ]
            )
        );
        assert!(parse("GUARD(EndX-BeginX)").is_ok());
        assert!(parse("THEMEGUARD(THEMEVAL(\"BackgroundColor\")+1)").is_ok());
        assert!(parse("_WALKGLUE(BegTrigger,EndTrigger,WalkPreference)").is_ok());
        assert!(parse("1+").is_err());
        assert!(parse("(1").is_err());
    }
}
