//! A controls card's values (docs/design/96-answer-projection-gaps.md, P1):
//! each input's value as a number and each output computed from them in
//! order. The host checks the card's worked example with this, and computes
//! what a person set before their values start a turn.

use crate::expression;
use serde_json::Value;
use std::collections::BTreeMap;

/// The kinds of input a controls card draws.
pub const INPUT_KINDS: &[&str] = &["number", "range", "choice", "toggle", "date"];

/// Days since 1970-01-01 for a `YYYY-MM-DD` date, so dates subtract.
pub fn days(date: &str) -> Option<f64> {
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) as f64)
}

/// An input's value as the expressions read it.
pub fn input_value(input: &Value) -> Option<f64> {
    match input["kind"].as_str()? {
        "toggle" => input["value"]
            .as_bool()
            .map(|on| if on { 1.0 } else { 0.0 }),
        "date" => days(input["value"].as_str()?),
        _ => input["value"].as_f64(),
    }
}

/// Every output of a stored card, computed in order from its inputs, with
/// `set` in place of the inputs it names.
pub fn compute(payload: &Value, set: &BTreeMap<String, f64>) -> Result<Vec<(String, f64)>, String> {
    let mut values = BTreeMap::new();
    for input in payload["inputs"].as_array().into_iter().flatten() {
        let name = input["name"].as_str().unwrap_or_default().to_string();
        let value = match set.get(&name) {
            Some(value) => *value,
            None => input_value(input).ok_or_else(|| format!("input `{name}` has no value"))?,
        };
        values.insert(name, value);
    }
    let mut outputs = Vec::new();
    for output in payload["outputs"].as_array().into_iter().flatten() {
        let name = output["name"].as_str().unwrap_or_default().to_string();
        let source = output["expression"].as_str().unwrap_or_default();
        let expr =
            expression::parse(source).map_err(|error| format!("output `{name}`: {error}"))?;
        let value = expression::evaluate(&expr, &values)
            .map_err(|error| format!("output `{name}`: {error}"))?;
        values.insert(name.clone(), value);
        outputs.push((name, value));
    }
    Ok(outputs)
}

/// A number written to `decimals` places.
pub fn shown(value: f64, decimals: u64) -> String {
    format!("{value:.*}", decimals.min(12) as usize)
}

/// Whether `computed` is `stated` at the precision it was stated to.
pub fn agrees(computed: f64, stated: f64, decimals: u64) -> bool {
    (computed - stated).abs() <= 0.5 * 10_f64.powi(-(decimals.min(12) as i32)) + 1e-9
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn dates_count_days() {
        assert_eq!(days("1970-01-01"), Some(0.0));
        assert_eq!(
            days("2026-10-17")
                .zip(days("2026-10-10"))
                .map(|(a, b)| a - b),
            Some(7.0)
        );
        assert_eq!(
            days("2024-03-01")
                .zip(days("2024-02-28"))
                .map(|(a, b)| a - b),
            Some(2.0)
        );
        assert_eq!(days("2026-13-01"), None);
    }

    #[test]
    fn outputs_read_inputs_and_earlier_outputs() {
        let card = serde_json::json!({
            "inputs": [
                {"name": "bill", "kind": "number", "value": 1200},
                {"name": "tip", "kind": "choice", "value": 10},
                {"name": "round_up", "kind": "toggle", "value": true}
            ],
            "outputs": [
                {"name": "total", "expression": "bill * (1 + tip / 100)"},
                {"name": "each", "expression": "if(round_up, ceil(total / 4), total / 4)"}
            ]
        });
        let outputs = compute(&card, &BTreeMap::new()).expect("computes");
        assert_eq!(
            outputs,
            vec![("total".into(), 1320.0), ("each".into(), 330.0)]
        );
        let set = BTreeMap::from([("bill".to_string(), 1000.0)]);
        assert_eq!(compute(&card, &set).expect("computes")[1].1, 275.0);
        assert!(agrees(8884.8789, 8884.88, 2));
        assert!(!agrees(8884.8789, 8884.0, 0));
    }
}
