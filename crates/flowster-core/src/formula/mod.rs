//! The ShapeSheet formula language (SPEC §7): parser, units and evaluator.

pub mod eval;
pub mod parser;
pub mod units;

pub use eval::{Env, ErrKind, Evaluator, Value};
pub use parser::{CellRef, Expr, ParseError, SheetRef, parse};

/// Parses and evaluates `formula` against `env`. Parse errors become `#NAME?`.
pub fn evaluate(formula: &str, env: &dyn Env) -> Value {
    match parse(formula) {
        Ok(e) => Evaluator { env }.eval(&e),
        Err(_) => Value::Err(ErrKind::Name),
    }
}
