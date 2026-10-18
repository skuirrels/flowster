//! Formula evaluation (SPEC §7.2, §7.4, §7.5).
//!
//! Semantics follow Visio as recorded by the oracle fixtures
//! (`tests/fixtures/formulas.json`): TRUE is 1, comparisons coerce strings to
//! numbers (non-numeric strings count as 0) and use a small tolerance, numeric
//! strings coerce in arithmetic, `ROUND` rounds half away from zero, `RGB()`
//! values are colours, HSL components use the 0–240 scale, list functions are 0-based.
//!
//! Functions that need state Flowster does not compute yet (theme lookups, text
//! metrics, volatile clocks, Visio-internal `_` functions) evaluate to
//! [`ErrKind::Unsupported`]; callers then keep the cell's cached value.

use libm::{acos, asin, atan, atan2, cos, cosh, floor, log, log10, pow, round, sin, sinh, sqrt, tan, tanh};

use super::parser::{BinOp, CellRef, ErrLit, Expr};
use crate::value::Rgba;

const PI: f64 = core::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrKind {
    Ref,
    Name,
    Value,
    Div0,
    Na,
    Num,
    Null,
    /// Not computable by Flowster yet: keep the cached value.
    Unsupported,
}

impl ErrKind {
    pub fn label(self) -> &'static str {
        match self {
            ErrKind::Ref => "#REF!",
            ErrKind::Name => "#NAME?",
            ErrKind::Value => "#VALUE!",
            ErrKind::Div0 => "#DIV/0!",
            ErrKind::Na => "#N/A",
            ErrKind::Num => "#NUM!",
            ErrKind::Null => "#NULL!",
            ErrKind::Unsupported => "#UNSUPPORTED",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
    Color(Rgba),
    /// A point and the sheet whose local coordinates it is in (`None` = current).
    Point {
        x: f64,
        y: f64,
        sheet: Option<u32>,
    },
    Err(ErrKind),
    Empty,
}

/// What a formula can see: cells, coordinate conversions and a few shape facts.
pub trait Env {
    fn cell(&self, r: &CellRef) -> Value;
    /// The sheet a reference points to, used to tag `PNT` results.
    fn sheet_of(&self, _r: &CellRef) -> Option<u32> {
        None
    }
    /// A point in `sheet`'s local coordinates → the current shape's parent coordinates.
    fn par(&self, _sheet: Option<u32>, _p: (f64, f64)) -> Option<(f64, f64)> {
        None
    }
    /// A point in the current shape's parent coordinates → `sheet`'s local coordinates.
    fn loc(&self, _sheet: Option<u32>, _p: (f64, f64)) -> Option<(f64, f64)> {
        None
    }
    fn shape_text(&self) -> Option<String> {
        None
    }
    fn master_name(&self) -> Option<String> {
        None
    }
}

/// Formats a number the way Visio does when it becomes text (`"x"&1.5` → `x1.5`).
pub fn num_to_string(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl Value {
    pub fn is_err(&self) -> bool {
        matches!(self, Value::Err(_))
    }

    /// Number in arithmetic context.
    pub fn num(&self) -> Result<f64, ErrKind> {
        match self {
            Value::Num(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Str(s) => s.trim().parse::<f64>().map_err(|_| ErrKind::Value),
            Value::Empty => Ok(0.0),
            Value::Err(e) => Err(*e),
            Value::Color(_) | Value::Point { .. } => Err(ErrKind::Value),
        }
    }

    /// Number in comparison context: non-numeric strings count as 0.
    fn cmp_num(&self) -> Result<f64, ErrKind> {
        match self {
            Value::Str(s) => Ok(s.trim().parse::<f64>().unwrap_or(0.0)),
            other => other.num(),
        }
    }

    pub fn text(&self) -> Result<String, ErrKind> {
        match self {
            Value::Str(s) => Ok(s.clone()),
            Value::Num(n) => Ok(num_to_string(*n)),
            Value::Bool(b) => Ok(if *b { "TRUE".into() } else { "FALSE".into() }),
            Value::Empty => Ok(String::new()),
            Value::Color(c) => Ok(format!("RGB({}, {}, {})", c.0, c.1, c.2)),
            Value::Point { x, y, .. } => Ok(format!("PNT({}, {})", num_to_string(*x), num_to_string(*y))),
            Value::Err(e) => Err(*e),
        }
    }

    fn color(&self) -> Result<Rgba, ErrKind> {
        match self {
            Value::Color(c) => Ok(*c),
            Value::Err(e) => Err(*e),
            Value::Str(s) => crate::value::parse_color(s, &Default::default()).ok_or(ErrKind::Value),
            _ => Err(ErrKind::Value),
        }
    }

    fn point(&self) -> Result<(f64, f64, Option<u32>), ErrKind> {
        match self {
            Value::Point { x, y, sheet } => Ok((*x, *y, *sheet)),
            Value::Err(e) => Err(*e),
            _ => Err(ErrKind::Value),
        }
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Round half away from zero to `digits` decimals (negative digits round left of the point).
fn round_half_away(x: f64, digits: f64) -> f64 {
    let m = pow(10.0, digits.trunc());
    round(x * m) / m
}

// ---- HSL on Visio's 0–240 scale ----------------------------------------------
// Integer RGB <-> HLS conversion with HLSMAX = 240 and RGBMAX = 255, as in the
// classic Windows colour-dialog algorithm; this matches Visio's results exactly.

const HLSMAX: i64 = 240;
const RGBMAX: i64 = 255;

/// (hue, saturation, luminance), each 0–240.
fn rgb_to_hls(c: Rgba) -> (i64, i64, i64) {
    let (r, g, b) = (i64::from(c.0), i64::from(c.1), i64::from(c.2));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = ((max + min) * HLSMAX + RGBMAX) / (2 * RGBMAX);
    if max == min {
        return (0, 0, l);
    }
    let s = if l <= HLSMAX / 2 {
        ((max - min) * HLSMAX + (max + min) / 2) / (max + min)
    } else {
        ((max - min) * HLSMAX + (2 * RGBMAX - max - min) / 2) / (2 * RGBMAX - max - min)
    };
    let delta = |x: i64| ((max - x) * (HLSMAX / 6) + (max - min) / 2) / (max - min);
    let (rd, gd, bd) = (delta(r), delta(g), delta(b));
    let mut h = if r == max {
        bd - gd
    } else if g == max {
        HLSMAX / 3 + rd - bd
    } else {
        2 * HLSMAX / 3 + gd - rd
    };
    if h < 0 {
        h += HLSMAX;
    }
    if h > HLSMAX {
        h -= HLSMAX;
    }
    (h, s, l)
}

fn hue_to_rgb(n1: i64, n2: i64, mut hue: i64) -> i64 {
    if hue < 0 {
        hue += HLSMAX;
    }
    if hue > HLSMAX {
        hue -= HLSMAX;
    }
    if hue < HLSMAX / 6 {
        n1 + ((n2 - n1) * hue + HLSMAX / 12) / (HLSMAX / 6)
    } else if hue < HLSMAX / 2 {
        n2
    } else if hue < 2 * HLSMAX / 3 {
        n1 + ((n2 - n1) * (2 * HLSMAX / 3 - hue) + HLSMAX / 12) / (HLSMAX / 6)
    } else {
        n1
    }
}

fn hls_to_rgb(h: i64, l: i64, s: i64) -> Rgba {
    let (h, l, s) = (h.rem_euclid(HLSMAX + 1), l.clamp(0, HLSMAX), s.clamp(0, HLSMAX));
    let q = |x: i64| x.clamp(0, RGBMAX) as u8;
    if s == 0 {
        let v = q(l * RGBMAX / HLSMAX);
        return Rgba(v, v, v, 255);
    }
    let m2 =
        if l <= HLSMAX / 2 { (l * (HLSMAX + s) + HLSMAX / 2) / HLSMAX } else { l + s - (l * s + HLSMAX / 2) / HLSMAX };
    let m1 = 2 * l - m2;
    let ch = |hue: i64| q((hue_to_rgb(m1, m2, hue) * RGBMAX + HLSMAX / 2) / HLSMAX);
    Rgba(ch(h + HLSMAX / 3), ch(h), ch(h - HLSMAX / 3), 255)
}

pub struct Evaluator<'e> {
    pub env: &'e dyn Env,
}

macro_rules! num {
    ($e:expr) => {
        match $e.num() {
            Ok(n) => n,
            Err(k) => return Value::Err(k),
        }
    };
}

macro_rules! txt {
    ($e:expr) => {
        match $e.text() {
            Ok(s) => s,
            Err(k) => return Value::Err(k),
        }
    };
}

fn num_result(n: f64) -> Value {
    if n.is_finite() { Value::Num(n) } else { Value::Err(ErrKind::Num) }
}

impl Evaluator<'_> {
    pub fn eval(&self, e: &Expr) -> Value {
        match e {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Err(l) => Value::Err(match l {
                ErrLit::Ref => ErrKind::Ref,
                ErrLit::Name => ErrKind::Name,
                ErrLit::Value => ErrKind::Value,
                ErrLit::Div0 => ErrKind::Div0,
                ErrLit::Na => ErrKind::Na,
                ErrLit::Num => ErrKind::Num,
                ErrLit::Null => ErrKind::Null,
            }),
            Expr::Ref(r) => self.env.cell(r),
            Expr::Neg(x) => Value::Num(-num!(self.eval(x))),
            Expr::Percent(x) => Value::Num(num!(self.eval(x)) / 100.0),
            Expr::Bin(op, a, b) => self.binary(*op, a, b),
            Expr::Call(name, args) => self.call(name, args),
        }
    }

    fn binary(&self, op: BinOp, a: &Expr, b: &Expr) -> Value {
        let (va, vb) = (self.eval(a), self.eval(b));
        if op == BinOp::Concat {
            return Value::Str(txt!(va) + &txt!(vb));
        }
        if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge) {
            let x = match va.cmp_num() {
                Ok(n) => n,
                Err(k) => return Value::Err(k),
            };
            let y = match vb.cmp_num() {
                Ok(n) => n,
                Err(k) => return Value::Err(k),
            };
            let eq = approx_eq(x, y);
            return Value::Bool(match op {
                BinOp::Eq => eq,
                BinOp::Ne => !eq,
                BinOp::Lt => x < y && !eq,
                BinOp::Gt => x > y && !eq,
                BinOp::Le => x < y || eq,
                _ => x > y || eq,
            });
        }
        let (x, y) = (num!(va), num!(vb));
        match op {
            BinOp::Add => num_result(x + y),
            BinOp::Sub => num_result(x - y),
            BinOp::Mul => num_result(x * y),
            BinOp::Div if y == 0.0 => Value::Err(ErrKind::Div0),
            BinOp::Div => num_result(x / y),
            BinOp::Pow => num_result(pow(x, y)),
            _ => unreachable!("handled above"),
        }
    }

    fn call(&self, name: &str, args: &[Expr]) -> Value {
        let n = args.len();
        let arg = |i: usize| self.eval(&args[i]);
        let need = |min: usize, max: usize| n < min || n > max;

        if name.starts_with('_') {
            return Value::Err(ErrKind::Unsupported);
        }
        match name {
            // ---- logic -------------------------------------------------------
            "IF" => {
                if n != 3 {
                    return Value::Err(ErrKind::Name);
                }
                let c = match arg(0) {
                    Value::Str(_) => return Value::Err(ErrKind::Value),
                    v => num!(v),
                };
                if c != 0.0 { arg(1) } else { arg(2) }
            }
            "AND" | "OR" => {
                if n == 0 {
                    return Value::Err(ErrKind::Name);
                }
                let mut acc = name == "AND";
                for i in 0..n {
                    let b = num!(arg(i)) != 0.0;
                    if name == "AND" {
                        acc &= b;
                    } else {
                        acc |= b;
                    }
                }
                Value::Bool(acc)
            }
            "NOT" if n == 1 => Value::Bool(num!(arg(0)) == 0.0),
            "IFERROR" if n == 2 => match arg(0) {
                Value::Err(ErrKind::Unsupported) => Value::Err(ErrKind::Unsupported),
                Value::Err(_) => arg(1),
                v => v,
            },
            "ISERROR" | "ISERR" | "ISERRNA" | "ISERRVALUE" if n == 1 => {
                let v = arg(0);
                Value::Bool(match (name, &v) {
                    (_, Value::Err(ErrKind::Unsupported)) => return Value::Err(ErrKind::Unsupported),
                    ("ISERROR", Value::Err(_)) => true,
                    ("ISERR", Value::Err(k)) => *k != ErrKind::Na,
                    ("ISERRNA", Value::Err(k)) => *k == ErrKind::Na,
                    ("ISERRVALUE", Value::Err(k)) => *k == ErrKind::Value,
                    _ => false,
                })
            }
            "NA" if n == 0 => Value::Err(ErrKind::Na),
            "TRUE" if n == 0 => Value::Bool(true),
            "FALSE" if n == 0 => Value::Bool(false),

            // ---- pass-throughs and actions ------------------------------------
            "GUARD" | "THEMEGUARD" | "SETATREFEXPR" | "SETATREFEVAL" if n == 1 => arg(0),
            "SETATREF" if n >= 1 => arg(0),
            "DEPENDSON" => Value::Num(0.0),
            "SETF" | "OPENTEXTWIN" | "OPENGROUPWIN" | "DOCMD" | "RUNADDON" | "RUNADDONWARGS" | "GOTOPAGE"
            | "HYPERLINK" | "CALLTHIS" | "PLAYSOUND" | "OPENSHEETWIN" | "HELP" => Value::Num(0.0),

            // ---- numbers -----------------------------------------------------
            "ABS" if n == 1 => Value::Num(num!(arg(0)).abs()),
            "SQRT" if n == 1 => {
                let x = num!(arg(0));
                if x < 0.0 { Value::Err(ErrKind::Num) } else { Value::Num(sqrt(x)) }
            }
            "MAX" | "MIN" | "SUM" if n >= 1 => {
                let mut acc = if name == "SUM" { 0.0 } else { num!(arg(0)) };
                for i in 0..n {
                    let x = num!(arg(i));
                    acc = match name {
                        "MAX" => acc.max(x),
                        "MIN" => acc.min(x),
                        _ => acc + x,
                    };
                }
                Value::Num(acc)
            }
            "INT" if n == 1 => Value::Num(floor(num!(arg(0)))),
            "INTUP" if n == 1 => {
                let x = num!(arg(0));
                Value::Num(if x >= 0.0 { libm::ceil(x) } else { floor(x) + if x == floor(x) { 0.0 } else { 1.0 } })
            }
            "ROUND" if n == 2 => Value::Num(round_half_away(num!(arg(0)), num!(arg(1)))),
            // CEILING rounds away from zero, FLOOR towards zero, in steps of `multiple`.
            "CEILING" | "FLOOR" if need(1, 2) => Value::Err(ErrKind::Name),
            "CEILING" | "FLOOR" => {
                let x = num!(arg(0));
                let m = if n == 2 { num!(arg(1)).abs() } else { 1.0 };
                if m == 0.0 {
                    return Value::Num(0.0);
                }
                let q = (x / m).abs();
                let r = if name == "CEILING" { libm::ceil(q - 1e-12) } else { floor(q + 1e-12) };
                Value::Num(r * m * x.signum())
            }
            "SIGN" if n == 1 => {
                let x = num!(arg(0));
                Value::Num(if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                })
            }
            // Floored modulus: the result has the divisor's sign.
            "MODULUS" if n == 2 => {
                let (x, y) = (num!(arg(0)), num!(arg(1)));
                if y == 0.0 { Value::Err(ErrKind::Div0) } else { Value::Num(x - y * floor(x / y)) }
            }
            "POW" if n == 2 => num_result(pow(num!(arg(0)), num!(arg(1)))),
            "LN" if n == 1 => {
                let x = num!(arg(0));
                if x <= 0.0 { Value::Err(ErrKind::Num) } else { Value::Num(log(x)) }
            }
            "LOG10" if n == 1 => {
                let x = num!(arg(0));
                if x <= 0.0 { Value::Err(ErrKind::Num) } else { Value::Num(log10(x)) }
            }
            "PI" if n == 0 => Value::Num(PI),
            "BITAND" | "BITOR" | "BITXOR" if n == 2 => {
                let (a, b) = (num!(arg(0)) as i64, num!(arg(1)) as i64);
                Value::Num(match name {
                    "BITAND" => a & b,
                    "BITOR" => a | b,
                    _ => a ^ b,
                } as f64)
            }
            "BITNOT" if n == 1 => Value::Num(f64::from(!(num!(arg(0)) as i64 as u16))),

            // ---- angles (radians internally) ---------------------------------
            "SIN" if n == 1 => Value::Num(sin(num!(arg(0)))),
            "COS" if n == 1 => Value::Num(cos(num!(arg(0)))),
            "TAN" if n == 1 => Value::Num(tan(num!(arg(0)))),
            "ATAN" if n == 1 => Value::Num(atan(num!(arg(0)))),
            "ATAN2" if n == 2 => Value::Num(atan2(num!(arg(0)), num!(arg(1)))),
            "ASIN" if n == 1 => {
                let x = num!(arg(0));
                if x.abs() > 1.0 { Value::Err(ErrKind::Num) } else { Value::Num(asin(x)) }
            }
            "ACOS" if n == 1 => {
                let x = num!(arg(0));
                if x.abs() > 1.0 { Value::Err(ErrKind::Num) } else { Value::Num(acos(x)) }
            }
            "ANG360" if n == 1 => Value::Num(num!(arg(0)).rem_euclid(2.0 * PI)),
            // DEG(x) only tags x as an angle; RAD(x) reads x as degrees.
            "DEG" if n == 1 => Value::Num(num!(arg(0))),
            "RAD" if n == 1 => Value::Num(num!(arg(0)) * PI / 180.0),
            "COSH" if n == 1 => Value::Num(cosh(num!(arg(0)))),
            "SINH" if n == 1 => Value::Num(sinh(num!(arg(0)))),
            "TANH" if n == 1 => Value::Num(tanh(num!(arg(0)))),

            // ---- strings -------------------------------------------------------
            "LEN" if n == 1 => Value::Num(txt!(arg(0)).chars().count() as f64),
            "FIND" if need(2, 4) => Value::Err(ErrKind::Name),
            "FIND" => {
                let needle = txt!(arg(0));
                let hay = txt!(arg(1));
                let start = if n >= 3 { (num!(arg(2)) as usize).max(1) - 1 } else { 0 };
                let ignore = n == 4 && num!(arg(3)) != 0.0;
                let (needle, hay) = if ignore { (needle.to_lowercase(), hay.to_lowercase()) } else { (needle, hay) };
                let chars: Vec<char> = hay.chars().collect();
                let rest: String = chars.iter().skip(start).collect();
                match rest.find(&needle) {
                    Some(b) => Value::Num((rest[..b].chars().count() + start + 1) as f64),
                    None => Value::Err(ErrKind::Value),
                }
            }
            "STRSAME" if need(2, 3) => Value::Err(ErrKind::Name),
            "STRSAME" => {
                let (a, b) = (txt!(arg(0)), txt!(arg(1)));
                let ignore = n == 3 && num!(arg(2)) != 0.0;
                Value::Bool(if ignore { a.to_lowercase() == b.to_lowercase() } else { a == b })
            }
            "CHAR" if n == 1 => match char::from_u32(num!(arg(0)) as u32) {
                Some(c) => Value::Str(c.to_string()),
                None => Value::Err(ErrKind::Value),
            },
            "LEFT" | "RIGHT" if n == 2 => {
                let s: Vec<char> = txt!(arg(0)).chars().collect();
                let k = (num!(arg(1)).max(0.0) as usize).min(s.len());
                Value::Str(if name == "LEFT" { s[..k].iter().collect() } else { s[s.len() - k..].iter().collect() })
            }
            "MID" if n == 3 => {
                let s: Vec<char> = txt!(arg(0)).chars().collect();
                let start = (num!(arg(1)).max(1.0) as usize - 1).min(s.len());
                let len = num!(arg(2)).max(0.0) as usize;
                Value::Str(s[start..(start + len).min(s.len())].iter().collect())
            }
            "UPPER" if n == 1 => Value::Str(txt!(arg(0)).to_uppercase()),
            "LOWER" if n == 1 => Value::Str(txt!(arg(0)).to_lowercase()),
            "TRIM" if n == 1 => Value::Str(txt!(arg(0)).split_whitespace().collect::<Vec<_>>().join(" ")),
            "SUBSTITUTE" if n == 3 => Value::Str(txt!(arg(0)).replace(&txt!(arg(1)), &txt!(arg(2)))),
            "REPLACE" if n == 4 => {
                let s: Vec<char> = txt!(arg(0)).chars().collect();
                let start = (num!(arg(1)).max(1.0) as usize - 1).min(s.len());
                let len = num!(arg(2)).max(0.0) as usize;
                let mut out: String = s[..start].iter().collect();
                out.push_str(&txt!(arg(3)));
                out.extend(s[(start + len).min(s.len())..].iter());
                Value::Str(out)
            }
            "REPT" if n == 2 => Value::Str(txt!(arg(0)).repeat(num!(arg(1)).max(0.0) as usize)),
            // INDEX(i, "a;b;c"[, delimiter[, error]]): 0-based; out of range gives "" (or `error`).
            "INDEX" if need(2, 4) => Value::Err(ErrKind::Name),
            "INDEX" => {
                let i = num!(arg(0));
                let list = txt!(arg(1));
                let delim = if n >= 3 { txt!(arg(2)) } else { ";".into() };
                let items: Vec<&str> = list.split(delim.as_str()).collect();
                if i >= 0.0 && (i as usize) < items.len() {
                    Value::Str(items[i as usize].to_owned())
                } else if n == 4 {
                    arg(3)
                } else {
                    Value::Str(String::new())
                }
            }
            // LOOKUP("b", "a;b;c"[, delimiter]): 0-based position, -1 if absent.
            "LOOKUP" if need(2, 3) => Value::Err(ErrKind::Name),
            "LOOKUP" => {
                let key = txt!(arg(0));
                let list = txt!(arg(1));
                let delim = if n == 3 { txt!(arg(2)) } else { ";".into() };
                let pos = list.split(delim.as_str()).position(|s| s.eq_ignore_ascii_case(&key));
                Value::Num(pos.map_or(-1.0, |p| p as f64))
            }
            "SHAPETEXT" => self.env.shape_text().map_or(Value::Err(ErrKind::Unsupported), Value::Str),
            "MASTERNAME" => self.env.master_name().map_or(Value::Str(String::new()), Value::Str),

            // ---- points --------------------------------------------------------
            "PNT" if n == 2 => {
                let sheet = [&args[0], &args[1]].iter().find_map(|a| match a {
                    Expr::Ref(r) => self.env.sheet_of(r),
                    _ => None,
                });
                Value::Point { x: num!(arg(0)), y: num!(arg(1)), sheet }
            }
            "PNTX" | "PNTY" if n == 1 => match arg(0).point() {
                Ok((x, y, _)) => Value::Num(if name == "PNTX" { x } else { y }),
                Err(k) => Value::Err(k),
            },
            "PAR" | "LOC" if n == 1 => match arg(0).point() {
                Ok((x, y, sheet)) => {
                    let p = if name == "PAR" { self.env.par(sheet, (x, y)) } else { self.env.loc(sheet, (x, y)) };
                    match p {
                        Some((x, y)) => Value::Point { x, y, sheet: None },
                        None => Value::Err(ErrKind::Unsupported),
                    }
                }
                Err(k) => Value::Err(k),
            },

            // ---- colours (HSL components on the 0–240 scale) -------------------
            "RGB" if n == 3 => {
                let c = |v: f64| v.clamp(0.0, 255.0) as u8;
                Value::Color(Rgba(c(num!(arg(0))), c(num!(arg(1))), c(num!(arg(2))), 255))
            }
            "HSL" if n == 3 => Value::Color(hls_to_rgb(num!(arg(0)) as i64, num!(arg(2)) as i64, num!(arg(1)) as i64)),
            "RED" | "GREEN" | "BLUE" if n == 1 => match arg(0).color() {
                Ok(c) => Value::Num(f64::from(match name {
                    "RED" => c.0,
                    "GREEN" => c.1,
                    _ => c.2,
                })),
                Err(k) => Value::Err(k),
            },
            "HUE" | "SAT" | "LUM" if n == 1 => match arg(0).color() {
                Ok(c) => {
                    let (h, s, l) = rgb_to_hls(c);
                    Value::Num(match name {
                        "HUE" => h,
                        "SAT" => s,
                        _ => l,
                    } as f64)
                }
                Err(k) => Value::Err(k),
            },
            "LUMDIFF" if n == 2 => match (arg(0).color(), arg(1).color()) {
                (Ok(a), Ok(b)) => Value::Num((rgb_to_hls(a).2 - rgb_to_hls(b).2).abs() as f64),
                (Err(k), _) | (_, Err(k)) => Value::Err(k),
            },
            // TINT raises luminance, TONE lowers saturation. Visio's darkening curve
            // (SHADE, negative TINT) is not linear in HSL or RGB: kept as cached values.
            "TINT" | "TONE" if n == 2 => match arg(0).color() {
                Ok(c) => {
                    let amount = num!(arg(1));
                    if name == "TINT" && amount < 0.0 {
                        return Value::Err(ErrKind::Unsupported);
                    }
                    let (h, s, l) = rgb_to_hls(c);
                    let a = amount as i64;
                    Value::Color(if name == "TINT" { hls_to_rgb(h, l + a, s) } else { hls_to_rgb(h, l, s - a) })
                }
                Err(k) => Value::Err(k),
            },

            // ---- needs state Flowster does not compute yet ---------------------
            "SHADE"
            | "THEMEVAL"
            | "THEMEPROP"
            | "THEMECBV"
            | "THEME"
            | "ISTHEMED"
            | "THEMERESTORE"
            | "CELLISTHEMED"
            | "TEXTWIDTH"
            | "TEXTHEIGHT"
            | "NOW"
            | "RAND"
            | "RANDBETWEEN"
            | "BOUND"
            | "NURBS"
            | "POLYLINE"
            | "FORMAT"
            | "FORMATEX"
            | "FIELDPICTURE"
            | "FONT"
            | "FONTTOID"
            | "GRAVITY"
            | "MSOTINT"
            | "MSOSHADE"
            | "USE"
            | "LISTSHEETREF"
            | "CONTAINERSHEETREF"
            | "POINTALONGPATH"
            | "GETREF"
            | "REF"
            | "MAGNITUDE"
            | "PAGENUMBER"
            | "PAGECOUNT"
            | "DOCLASTSAVE"
            | "DOCLASTEDIT"
            | "DOCCREATION"
            | "DATETIME"
            | "DATEVALUE"
            | "DAY"
            | "MONTH"
            | "YEAR"
            | "HOUR"
            | "MINUTE"
            | "SECOND"
            | "WEEKDAY"
            | "DATE"
            | "TIME"
            | "EVALTEXT"
            | "EVALCELL"
            | "ID"
            | "USERUI"
            | "LISTSEP"
            | "FILENAME"
            | "TITLE"
            | "DIRECTORY"
            | "DESCRIPTION"
            | "CREATOR"
            | "BLEND"
            | "FORMULAEXISTS"
            | "LOCALFORMULAEXISTS"
            | "PATHLENGTH"
            | "NEARESTPOINTONPATH"
            | "ANGLETOLOC"
            | "ANGLETOPAR"
            | "LOCTOPAR"
            | "CONTAINERCOUNT"
            | "CONTAINERMEMBERCOUNT"
            | "CALLOUTCOUNT"
            | "CALLOUTTARGETREF"
            | "LISTMEMBERCOUNT"
            | "LISTORDER"
            | "MASTERMAP"
            | "SHAPEBINDING"
            | "CONNECTORBINDING"
            | "LAYOUT"
            | "CFFBINDING"
            | "MEMBEROFGRID"
            | "RECTSECT"
            | "SATDIFF"
            | "AGGCOUNT"
            | "GLUEDSHAPES"
            | "CONNECTEDSHAPES"
            | "SHAPESONPAGE"
            | "FILTERSET"
            | "HASCATEGORY"
            | "ONLAYER"
            | "ROLE"
            | "IS1D" => Value::Err(ErrKind::Unsupported),
            // Known functions with the wrong number of arguments, and unknown functions.
            _ => Value::Err(ErrKind::Name),
        }
    }
}
