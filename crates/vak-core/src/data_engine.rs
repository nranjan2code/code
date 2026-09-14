//! In-Memory Polyglot Data Engine.
//!
//! Provides deterministic in-memory querying, filtering, projection,
//! grouping, sorting, and statistical aggregation over tabular data
//! (CSV, JSON arrays, or Markdown tables) with zero subprocess overhead.

use std::collections::BTreeMap;

use serde_json::Value;

pub struct DataQueryTool;

#[derive(Debug, Clone)]
struct TableData {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl TableData {
    fn from_csv(text: &str) -> Result<Self, String> {
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if lines.is_empty() {
            return Err("empty CSV data".into());
        }
        let headers = parse_csv_line(lines[0]);
        let mut rows = Vec::new();
        for (idx, line) in lines.iter().skip(1).enumerate() {
            let cells = parse_csv_line(line);
            if cells.len() != headers.len() {
                return Err(format!(
                    "CSV row {} has {} columns, expected {}",
                    idx + 1,
                    cells.len(),
                    headers.len()
                ));
            }
            rows.push(cells);
        }
        Ok(TableData { headers, rows })
    }

    fn from_json(val: &Value) -> Result<Self, String> {
        let arr = val.as_array().ok_or_else(|| "JSON data must be an array of objects".to_string())?;
        if arr.is_empty() {
            return Ok(TableData { headers: Vec::new(), rows: Vec::new() });
        }
        let mut header_set = Vec::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                for k in obj.keys() {
                    if !header_set.contains(k) {
                        header_set.push(k.clone());
                    }
                }
            }
        }
        let mut rows = Vec::new();
        for item in arr {
            let mut row = Vec::new();
            if let Some(obj) = item.as_object() {
                for h in &header_set {
                    let cell = match obj.get(h) {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(other) => other.to_string(),
                    };
                    row.push(cell);
                }
            }
            rows.push(row);
        }
        Ok(TableData { headers: header_set, rows })
    }

    fn from_markdown(text: &str) -> Result<Self, String> {
        let mut table_lines = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
                table_lines.push(trimmed);
            } else if !table_lines.is_empty() {
                break;
            }
        }
        if table_lines.len() < 2 {
            return Err("invalid markdown table (needs header and separator)".into());
        }

        fn parse_md_cells(l: &str) -> Vec<String> {
            l.trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_string())
                .collect()
        }

        let headers = parse_md_cells(table_lines[0]);
        let mut rows = Vec::new();
        for (i, line) in table_lines.iter().skip(2).enumerate() {
            let cells = parse_md_cells(line);
            if cells.len() != headers.len() {
                return Err(format!(
                    "markdown table row {} has {} columns, expected {}",
                    i + 1,
                    cells.len(),
                    headers.len()
                ));
            }
            rows.push(cells);
        }
        Ok(TableData { headers, rows })
    }

    fn col_index(&self, name: &str) -> Option<usize> {
        self.headers.iter().position(|h| h.eq_ignore_ascii_case(name))
    }

    fn to_markdown(&self) -> String {
        if self.headers.is_empty() {
            return String::new();
        }
        let mut out = format!("| {} |\n", self.headers.join(" | "));
        let sep: Vec<String> = self.headers.iter().map(|_| "---".to_string()).collect();
        out.push_str(&format!("| {} |\n", sep.join(" | ")));
        for row in &self.rows {
            out.push_str(&format!("| {} |\n", row.join(" | ")));
        }
        out
    }

    fn to_json(&self) -> Value {
        let mut arr = Vec::new();
        for row in &self.rows {
            let mut obj = serde_json::Map::new();
            for (idx, header) in self.headers.iter().enumerate() {
                let cell = row.get(idx).cloned().unwrap_or_default();
                if let Ok(num) = cell.parse::<f64>() {
                    if num.fract() == 0.0 && num >= i64::MIN as f64 && num <= i64::MAX as f64 {
                        obj.insert(header.clone(), Value::from(num as i64));
                    } else {
                        obj.insert(header.clone(), Value::from(num));
                    }
                } else if cell.eq_ignore_ascii_case("true") {
                    obj.insert(header.clone(), Value::Bool(true));
                } else if cell.eq_ignore_ascii_case("false") {
                    obj.insert(header.clone(), Value::Bool(false));
                } else {
                    obj.insert(header.clone(), Value::String(cell));
                }
            }
            arr.push(Value::Object(obj));
        }
        Value::Array(arr)
    }
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' => {
                in_quotes = true;
            }
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            other => current.push(other),
        }
    }
    fields.push(current.trim().to_string());
    fields
}

#[async_trait::async_trait]
impl vak_tools::Tool for DataQueryTool {
    fn name(&self) -> &str {
        "data_query"
    }

    fn description(&self) -> &str {
        "Query and analyze structured data (CSV, JSON arrays, or Markdown tables) in memory. \
         Supports column projection (select), row filtering (filter), grouping & aggregation \
         (count, sum, avg, min, max), sorting, and row limits. Produces verified outputs."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "string",
                    "description": "Raw tabular string data: CSV text, JSON array of objects, or Markdown table"
                },
                "format": {
                    "type": "string",
                    "enum": ["csv", "json", "markdown"],
                    "description": "Optional format hint ('csv', 'json', 'markdown'). Auto-detected if omitted."
                },
                "select": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of columns to include in the output"
                },
                "filter": {
                    "type": "object",
                    "properties": {
                        "column": { "type": "string" },
                        "op": { "type": "string", "enum": ["eq", "neq", "gt", "lt", "contains"] },
                        "value": { "type": "string" }
                    },
                    "required": ["column", "op", "value"],
                    "description": "Optional row filter predicate"
                },
                "group_by": {
                    "type": "string",
                    "description": "Optional column to group by before aggregation"
                },
                "aggregate": {
                    "type": "object",
                    "properties": {
                        "column": { "type": "string" },
                        "fn": { "type": "string", "enum": ["count", "sum", "avg", "min", "max"] }
                    },
                    "required": ["column", "fn"],
                    "description": "Optional aggregation function"
                },
                "sort_by": {
                    "type": "object",
                    "properties": {
                        "column": { "type": "string" },
                        "descending": { "type": "boolean" }
                    },
                    "required": ["column"],
                    "description": "Optional column to sort rows by"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of rows to return"
                }
            },
            "required": ["data"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(raw_data) = args.get("data").and_then(Value::as_str).map(str::trim) else {
            return vak_tools::ToolOutput::error("missing required argument 'data'");
        };
        if raw_data.is_empty() {
            return vak_tools::ToolOutput::error("'data' must not be empty");
        }

        let format = args.get("format").and_then(Value::as_str);
        let table = if format == Some("json") || (format.is_none() && raw_data.starts_with('[')) {
            match serde_json::from_str::<Value>(raw_data) {
                Ok(v) => TableData::from_json(&v),
                Err(e) => return vak_tools::ToolOutput::error(format!("invalid JSON data: {e}")),
            }
        } else if format == Some("markdown") || (format.is_none() && raw_data.starts_with('|')) {
            TableData::from_markdown(raw_data)
        } else {
            TableData::from_csv(raw_data)
        };

        let mut table = match table {
            Ok(t) => t,
            Err(e) => return vak_tools::ToolOutput::error(format!("could not parse data: {e}")),
        };

        // 1. Filtering
        if let Some(filter) = args.get("filter") {
            let col = filter.get("column").and_then(Value::as_str).unwrap_or("");
            let op = filter.get("op").and_then(Value::as_str).unwrap_or("eq");
            let target_val = filter.get("value").and_then(Value::as_str).unwrap_or("");

            if let Some(idx) = table.col_index(col) {
                let target_num = target_val.parse::<f64>().ok();
                table.rows.retain(|row| {
                    let cell = row.get(idx).map(|s| s.as_str()).unwrap_or("");
                    let cell_num = cell.parse::<f64>().ok();
                    match op {
                        "eq" => cell.eq_ignore_ascii_case(target_val),
                        "neq" => !cell.eq_ignore_ascii_case(target_val),
                        "contains" => cell.to_ascii_lowercase().contains(&target_val.to_ascii_lowercase()),
                        "gt" => match (cell_num, target_num) {
                            (Some(c), Some(t)) => c > t,
                            _ => cell > target_val,
                        },
                        "lt" => match (cell_num, target_num) {
                            (Some(c), Some(t)) => c < t,
                            _ => cell < target_val,
                        },
                        _ => true,
                    }
                });
            } else {
                return vak_tools::ToolOutput::error(format!("filter column '{col}' not found"));
            }
        }

        // 2. Group By & Aggregation
        if let Some(agg) = args.get("aggregate") {
            let agg_col = agg.get("column").and_then(Value::as_str).unwrap_or("");
            let agg_fn = agg.get("fn").and_then(Value::as_str).unwrap_or("count");
            let agg_idx = match table.col_index(agg_col) {
                Some(i) => i,
                None => return vak_tools::ToolOutput::error(format!("aggregate column '{agg_col}' not found")),
            };

            if let Some(group_by_col) = args.get("group_by").and_then(Value::as_str) {
                let group_idx = match table.col_index(group_by_col) {
                    Some(i) => i,
                    None => return vak_tools::ToolOutput::error(format!("group_by column '{group_by_col}' not found")),
                };

                let mut groups: BTreeMap<String, Vec<f64>> = BTreeMap::new();
                for row in &table.rows {
                    let key = row.get(group_idx).cloned().unwrap_or_default();
                    let val = row.get(agg_idx).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    groups.entry(key).or_default().push(val);
                }

                let mut agg_rows = Vec::new();
                for (k, vals) in groups {
                    let result = match agg_fn {
                        "count" => vals.len() as f64,
                        "sum" => vals.iter().sum(),
                        "avg" => if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / vals.len() as f64 },
                        "min" => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                        "max" => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                        _ => vals.len() as f64,
                    };
                    agg_rows.push(vec![k, format!("{result:.2}").trim_end_matches(".00").to_string()]);
                }

                table.headers = vec![group_by_col.to_string(), format!("{agg_fn}_{agg_col}")];
                table.rows = agg_rows;
            } else {
                let vals: Vec<f64> = table
                    .rows
                    .iter()
                    .map(|r| r.get(agg_idx).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0))
                    .collect();
                let result = match agg_fn {
                    "count" => vals.len() as f64,
                    "sum" => vals.iter().sum(),
                    "avg" => if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / vals.len() as f64 },
                    "min" => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                    "max" => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    _ => vals.len() as f64,
                };
                table.headers = vec![format!("{agg_fn}_{agg_col}")];
                table.rows = vec![vec![format!("{result:.2}").trim_end_matches(".00").to_string()]];
            }
        }

        // 3. Sorting
        if let Some(sort) = args.get("sort_by") {
            if let Some(sort_col) = sort.get("column").and_then(Value::as_str) {
                let desc = sort.get("descending").and_then(Value::as_bool).unwrap_or(false);
                if let Some(idx) = table.col_index(sort_col) {
                    table.rows.sort_by(|a, b| {
                        let val_a = a.get(idx).map(|s| s.as_str()).unwrap_or("");
                        let val_b = b.get(idx).map(|s| s.as_str()).unwrap_or("");
                        let num_a = val_a.parse::<f64>().ok();
                        let num_b = val_b.parse::<f64>().ok();
                        let ord = match (num_a, num_b) {
                            (Some(na), Some(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
                            _ => val_a.cmp(val_b),
                        };
                        if desc { ord.reverse() } else { ord }
                    });
                }
            }
        }

        // 4. Projection (Select)
        if let Some(select_cols) = args.get("select").and_then(Value::as_array) {
            let requested: Vec<&str> = select_cols.iter().filter_map(Value::as_str).collect();
            if !requested.is_empty() {
                let indices: Vec<Option<usize>> = requested.iter().map(|c| table.col_index(c)).collect();
                let mut new_rows = Vec::new();
                for row in &table.rows {
                    let new_row = indices
                        .iter()
                        .map(|opt| opt.and_then(|i| row.get(i).cloned()).unwrap_or_default())
                        .collect();
                    new_rows.push(new_row);
                }
                table.headers = requested.into_iter().map(String::from).collect();
                table.rows = new_rows;
            }
        }

        // 5. Limit
        if let Some(limit) = args.get("limit").and_then(Value::as_u64) {
            table.rows.truncate(limit as usize);
        }

        let md = table.to_markdown();
        let json_val = table.to_json();
        let response_text = format!(
            "Query Result ({} rows, {} columns):\n\n{}\n```json\n{}\n```",
            table.rows.len(),
            table.headers.len(),
            md,
            serde_json::to_string_pretty(&json_val).unwrap_or_default()
        );

        vak_tools::ToolOutput::ok(response_text)
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vak_tools::Tool;

    #[tokio::test]
    async fn data_query_csv_filter_sort_aggregate() {
        let tool = DataQueryTool;
        let ctx = vak_tools::ToolContext {
            cwd: std::path::PathBuf::from("/tmp"),
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
            sandbox_sink: None,
        };

        let csv = "name,dept,salary\nAlice,Eng,120000\nBob,Sales,85000\nCharlie,Eng,140000\nDave,Sales,90000\nEve,Eng,110000\n";

        // 1. Filter and sort
        let args = serde_json::json!({
            "data": csv,
            "filter": { "column": "dept", "op": "eq", "value": "Eng" },
            "sort_by": { "column": "salary", "descending": true },
            "select": ["name", "salary"]
        });
        let out = tool.execute(&args, &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("Charlie"), "{}", out.content);
        assert!(!out.content.contains("Bob"), "{}", out.content);

        // 2. Group by and aggregation
        let agg_args = serde_json::json!({
            "data": csv,
            "group_by": "dept",
            "aggregate": { "column": "salary", "fn": "avg" }
        });
        let agg_out = tool.execute(&agg_args, &ctx).await;
        assert!(!agg_out.is_error, "{}", agg_out.content);
        assert!(agg_out.content.contains("Eng"), "{}", agg_out.content);
        assert!(agg_out.content.contains("123333.33") || agg_out.content.contains("123333"), "{}", agg_out.content);
    }
}
