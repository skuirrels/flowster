//! Formula conformance against the Visio oracle (SPEC §22.3).
//!
//! `tests/fixtures/formulas.json` is produced by
//! `tools/visio-oracle/formula-fixtures.ps1`: each case is a formula from
//! `tools/visio-oracle/formulas.txt` with the result Visio computed for it.
//! Numbers must match within 1e-9; text, colours and errors exactly.

use std::collections::BTreeMap;

use flowster_core::formula::{CellRef, Env, ErrKind, SheetRef, Value, evaluate};
use serde_json::Value as Json;

/// Functions Flowster deliberately leaves to cached values for now.
const KNOWN_UNSUPPORTED: &[&str] = &["SHADE(", "TINT(RGB(200,100,50),-30)"];

struct Context(BTreeMap<String, Json>);

impl Env for Context {
    fn cell(&self, r: &CellRef) -> Value {
        let key = match &r.sheet {
            Some(SheetRef::Id(n)) => format!("sheet.{n}!{}", r.name.to_ascii_lowercase()),
            Some(_) => return Value::Err(ErrKind::Ref),
            None => r.name.to_ascii_lowercase(),
        };
        match self.0.get(&key) {
            Some(Json::Number(n)) => Value::Num(n.as_f64().unwrap()),
            Some(Json::String(s)) => Value::Str(s.clone()),
            _ => Value::Err(ErrKind::Ref),
        }
    }
}

fn load() -> Json {
    let raw = include_str!("fixtures/formulas.json");
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).expect("fixture JSON")
}

#[test]
fn matches_visio() {
    let fixture = load();
    let ctx = Context(
        fixture["context"].as_object().unwrap().iter().map(|(k, v)| (k.to_ascii_lowercase(), v.clone())).collect(),
    );
    let mut failures = Vec::new();
    let mut unsupported = Vec::new();
    let cases = fixture["cases"].as_array().unwrap();
    for case in cases {
        let formula = case["formula"].as_str().unwrap();
        let got = evaluate(formula, &ctx);
        if got == Value::Err(ErrKind::Unsupported) {
            unsupported.push(formula.to_owned());
            continue;
        }
        let visio_error = case.get("parseError").is_some() || case["error"].as_i64().unwrap_or(0) != 0;
        let ok = if visio_error {
            got.is_err()
        } else {
            let want_str = case["str"].as_str().unwrap_or("");
            let want_num = case["value"].as_f64().unwrap_or(f64::NAN);
            match &got {
                Value::Str(s) => s == want_str,
                Value::Color(_) => got.text().ok().as_deref() == Some(want_str),
                Value::Err(_) => false,
                other => match other.num() {
                    Ok(n) => (n - want_num).abs() <= 1e-9 * want_num.abs().max(1.0),
                    Err(_) => false,
                },
            }
        };
        if !ok {
            failures.push(format!("{formula}: got {got:?}, Visio {} / {:?}", case["value"], case["str"]));
        }
    }
    for u in &unsupported {
        assert!(KNOWN_UNSUPPORTED.iter().any(|k| u.starts_with(k)), "unexpectedly unsupported: {u}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Visio:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
