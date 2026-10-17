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
        if let Some(unknown) = expression::names(&expr)
            .into_iter()
            .find(|read| !values.contains_key(read))
        {
            let known: Vec<&str> = values.keys().map(String::as_str).collect();
            return Err(format!(
                "output `{name}`: `{unknown}` is not an input or an earlier output; the names are {}, and a fixed amount is written as a number",
                known.join(", ")
            ));
        }
        let value = expression::evaluate(&expr, &values)
            .map_err(|error| format!("output `{name}`: {error}"))?;
        values.insert(name.clone(), value);
        outputs.push((name, value));
    }
    Ok(outputs)
}

/// A computed number as JSON: a whole number stays whole.
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
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

/// A series' points: the varied input's value and the output's.
pub type Points = Vec<(f64, f64)>;

/// Published series by name, each with the presentation that published it.
pub type SeriesScope = BTreeMap<String, (Points, String)>;

/// Every series output of a stored card: its expression computed across a
/// range of one of the card's inputs (`vary` from `from` to `to` by
/// `step`), each point the input's value and the output's
/// (docs/design/98 §2).
/// `max_points` is the operator's limit (`[control.limits] series_points`).
pub fn series(payload: &Value, max_points: u64) -> Result<Vec<(String, Points)>, String> {
    let inputs = payload["inputs"].as_array().cloned().unwrap_or_default();
    let outputs = payload["outputs"].as_array().cloned().unwrap_or_default();
    let mut base = BTreeMap::new();
    for input in &inputs {
        let name = input["name"].as_str().unwrap_or_default().to_string();
        base.insert(
            name.clone(),
            input_value(input).ok_or_else(|| format!("input `{name}` has no value"))?,
        );
    }
    for (name, value) in compute(payload, &BTreeMap::new())? {
        base.insert(name, value);
    }
    let parsed = outputs
        .iter()
        .map(|output| expression::parse(output["expression"].as_str().unwrap_or_default()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for (at, output) in outputs.iter().enumerate() {
        let Some(vary) = output["vary"].as_str() else {
            continue;
        };
        let name = output["name"].as_str().unwrap_or_default().to_string();
        let bound = |key: &str| -> Result<f64, String> {
            let source = output[key].as_str().unwrap_or_default();
            let expr = expression::parse(source)
                .map_err(|error| format!("output `{name}` `{key}`: {error}"))?;
            expression::evaluate(&expr, &base)
                .map_err(|error| format!("output `{name}` `{key}`: {error}"))
        };
        let (from, to, step) = (bound("from")?, bound("to")?, bound("step")?);
        if step <= 0.0 {
            return Err(format!(
                "output `{name}`: `step` must come to more than zero"
            ));
        }
        if (to - from) / step > max_points as f64 {
            return Err(format!(
                "output `{name}`: the range gives more than {max_points} points; take a larger step"
            ));
        }
        let mut points = Vec::new();
        let mut index = 0_u32;
        loop {
            let x = from + step * f64::from(index);
            if x > to + step * 1e-9 {
                break;
            }
            let mut values = base.clone();
            values.insert(vary.to_string(), x);
            for (earlier, expr) in outputs.iter().zip(&parsed).take(at + 1) {
                let value = expression::evaluate(expr, &values)
                    .map_err(|error| format!("output `{name}` at {vary} = {x}: {error}"))?;
                values.insert(
                    earlier["name"].as_str().unwrap_or_default().to_string(),
                    value,
                );
            }
            points.push((x, values[&name]));
            index += 1;
        }
        out.push((name, points));
    }
    Ok(out)
}

/// What a controls card publishes to the cards after it in its answer
/// (docs/design/98 §2): each input's value and each output's, by name.
pub fn published(payload: &Value) -> Result<Vec<(String, f64)>, String> {
    let mut out = Vec::new();
    for input in payload["inputs"].as_array().into_iter().flatten() {
        let name = input["name"].as_str().unwrap_or_default().to_string();
        let value = input_value(input).ok_or_else(|| format!("input `{name}` has no value"))?;
        out.push((name, value));
    }
    out.extend(compute(payload, &BTreeMap::new())?);
    Ok(out)
}

/// The concrete paths in `payload` that match the declared patterns, where
/// `*` stands for every index of a list (docs/design/98 §2).
fn value_paths(payload: &Value, patterns: &[Vec<String>]) -> Vec<Vec<Value>> {
    fn expand(value: &Value, pattern: &[String], at: Vec<Value>, out: &mut Vec<Vec<Value>>) {
        let Some((step, rest)) = pattern.split_first() else {
            out.push(at);
            return;
        };
        if step == "*" {
            for (index, item) in value.as_array().into_iter().flatten().enumerate() {
                let mut next = at.clone();
                next.push(Value::from(index));
                expand(item, rest, next, out);
            }
        } else if let Some(item) = value.get(step.as_str()) {
            let mut next = at;
            next.push(Value::from(step.as_str()));
            expand(item, rest, next, out);
        }
    }
    let mut out = Vec::new();
    for pattern in patterns {
        expand(payload, pattern, Vec::new(), &mut out);
    }
    out
}

fn at_path<'a>(value: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    path.iter().try_fold(value, |here, step| match step {
        Value::String(key) => here.get_mut(key.as_str()),
        Value::Number(index) => here.get_mut(usize::try_from(index.as_u64()?).ok()?),
        _ => None,
    })
}

/// Turn each `= expression` field of a card into the value it computes from
/// `scope` (name to value and the presentation that published it), and
/// record the binding in the card's `bound` list so a client recomputes it
/// when those values change. A name nothing earlier published is refused.
/// `followable` is what the card's tool declares may follow published
/// values (`Tool::followable_fields`).
pub fn bind(
    form: &str,
    followable: &[Vec<String>],
    payload: &mut Value,
    scope: &BTreeMap<String, (f64, String)>,
    series_scope: &SeriesScope,
) -> Result<(), String> {
    let mut bound = Vec::new();
    // A chart series that follows a published series takes its points.
    if form == "chart" {
        // `get_mut`, never `payload["series"]`, which would add the key.
        for (at, series) in payload
            .get_mut("series")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(name) = series["follows"].as_str().map(str::to_string) else {
                continue;
            };
            let Some((points, from)) = series_scope.get(&name) else {
                let known: Vec<&str> = series_scope.keys().map(String::as_str).collect();
                return Err(format!(
                    "the chart follows `{name}`, which no controls card before it in the answer publishes as a series{}",
                    if known.is_empty() {
                        String::new()
                    } else {
                        format!(" (series published: {})", known.join(", "))
                    }
                ));
            };
            series["points"] = Value::Array(
                points
                    .iter()
                    .map(|(x, y)| serde_json::json!({"x": number(*x), "y": number(*y)}))
                    .collect(),
            );
            bound.push(serde_json::json!({"path": ["series", at, "points"], "series": name, "from": [from]}));
        }
    }
    for path in value_paths(payload, followable) {
        let Some(field) = at_path(payload, &path) else {
            continue;
        };
        let Some(source) = field
            .as_str()
            .and_then(|text| text.trim().strip_prefix('='))
        else {
            continue;
        };
        let source = source.trim().to_string();
        let expr = expression::parse(&source).map_err(|error| format!("`= {source}`: {error}"))?;
        let names = expression::names(&expr);
        if let Some(unknown) = names.iter().find(|name| !scope.contains_key(*name)) {
            let known: Vec<&str> = scope.keys().map(String::as_str).collect();
            return Err(format!(
                "`= {source}` reads `{unknown}`, which no controls card before this one in the answer publishes{}",
                if known.is_empty() {
                    String::new()
                } else {
                    format!(" (published: {})", known.join(", "))
                }
            ));
        }
        let values: BTreeMap<String, f64> = scope
            .iter()
            .map(|(name, (value, _))| (name.clone(), *value))
            .collect();
        let value = expression::evaluate(&expr, &values)
            .map_err(|error| format!("`= {source}`: {error}"))?;
        *field = number(value);
        let mut from: Vec<String> = names.iter().map(|name| scope[name].1.clone()).collect();
        from.sort();
        from.dedup();
        bound.push(serde_json::json!({"path": path, "expression": source, "from": from}));
    }
    if !bound.is_empty() {
        payload["bound"] = Value::Array(bound);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod binding_tests {
    use super::*;

    #[test]
    fn a_series_varies_an_input_and_a_chart_follows_it() {
        let card = serde_json::json!({"inputs": [
            {"name": "start", "kind": "number", "value": 100},
            {"name": "rate", "kind": "number", "value": 10},
            {"name": "year", "kind": "number", "value": 0}],
            "outputs": [{"name": "grown", "expression": "start * (1 + rate / 100) ^ year",
             "vary": "year", "from": "0", "to": "3", "step": "1"}]});
        let all = series(&card, 10_000).expect("computes");
        assert_eq!(all[0].0, "grown");
        assert_eq!(all[0].1.len(), 4);
        assert!((all[0].1[3].1 - 133.1).abs() < 1e-9);
        let series_scope = BTreeMap::from([(
            "grown".to_string(),
            (all[0].1.clone(), "pres-1".to_string()),
        )]);
        let mut chart = serde_json::json!({"chart_type": "line", "accessible_summary": "Growth.",
            "series": [{"name": "grown", "follows": "grown", "points": []}]});
        bind("chart", &[], &mut chart, &BTreeMap::new(), &series_scope).expect("binds");
        assert_eq!(
            chart["series"][0]["points"][0],
            serde_json::json!({"x": 0, "y": 100})
        );
        assert_eq!(chart["bound"][0]["series"], "grown");
        let mut wide = card.clone();
        wide["outputs"][0]["step"] = serde_json::json!("0.0001");
        assert!(series(&wide, 10_000).is_err());
    }

    #[test]
    fn a_card_binds_its_fields_to_values_published_before_it() {
        let scope = BTreeMap::from([
            ("emi".to_string(), (8884.88, "pres-1".to_string())),
            ("n".to_string(), (12.0, "pres-1".to_string())),
        ]);
        let mut stats = serde_json::json!({"readings": [
            {"label": "Monthly", "value": "= emi"}, {"label": "Fixed", "value": "3"}]});
        bind(
            "stats",
            &[vec!["readings".into(), "*".into(), "value".into()]],
            &mut stats,
            &scope,
            &BTreeMap::new(),
        )
        .expect("binds");
        assert_eq!(stats["readings"][0]["value"], 8884.88);
        assert_eq!(stats["readings"][1]["value"], "3");
        assert_eq!(
            stats["bound"][0]["path"],
            serde_json::json!(["readings", 0, "value"])
        );
        assert_eq!(stats["bound"][0]["from"], serde_json::json!(["pres-1"]));
        let mut table =
            serde_json::json!({"columns": [{"label": "Total"}], "rows": [["= round(emi * n, 2)"]]});
        bind(
            "table",
            &[vec!["rows".into(), "*".into(), "*".into()]],
            &mut table,
            &scope,
            &BTreeMap::new(),
        )
        .expect("binds");
        assert_eq!(table["rows"][0][0], 106618.56);
        let mut unknown = serde_json::json!({"readings": [{"label": "X", "value": "= rate"}]});
        let refused = bind(
            "stats",
            &[vec!["readings".into(), "*".into(), "value".into()]],
            &mut unknown,
            &scope,
            &BTreeMap::new(),
        )
        .expect_err("no such name");
        assert!(
            refused.contains("`rate`") && refused.contains("emi, n"),
            "{refused}"
        );
    }
}
