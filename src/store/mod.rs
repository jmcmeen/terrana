pub mod loader;

use crate::error::AppError;
use serde_json::Value;
use std::collections::HashMap;

pub struct DataTable {
    /// Column metadata: (name, type_string) pairs
    pub columns: Vec<(String, String)>,
    /// Rows as JSON objects, index 0 = rowid 1
    pub rows: Vec<Value>,
    pub row_count: i64,
}

impl DataTable {
    pub fn get_rows_by_ids(&self, ids: &[i64]) -> Vec<Value> {
        ids.iter()
            .filter_map(|&id| self.rows.get((id - 1) as usize).cloned())
            .collect()
    }

    pub fn query(
        &self,
        ids: Option<&[i64]>,
        where_clauses: &[(String, String)],
        select_cols: Option<&[String]>,
        group_by: Option<&str>,
        agg: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, AppError> {
        let source: Box<dyn Iterator<Item = &Value>> = match ids {
            Some(ids) => Box::new(ids.iter().filter_map(|&id| self.rows.get((id - 1) as usize))),
            None => Box::new(self.rows.iter()),
        };

        let filtered: Vec<&Value> = source
            .filter(|row| matches_where(row, where_clauses))
            .collect();

        if let (Some(gb), Some(a)) = (group_by, agg) {
            return Ok(apply_group_by(&filtered, gb, a, limit));
        }

        let result: Vec<Value> = filtered
            .into_iter()
            .take(limit)
            .map(|row| match select_cols {
                Some(cols) if !cols.is_empty() => select_columns(row, cols),
                _ => row.clone(),
            })
            .collect();

        Ok(result)
    }
}

fn matches_where(row: &Value, clauses: &[(String, String)]) -> bool {
    clauses.iter().all(|(col, val)| {
        row.get(col).map_or(false, |v| match v {
            Value::String(s) => s == val,
            Value::Number(n) => n.to_string() == *val,
            Value::Bool(b) => b.to_string() == *val,
            _ => false,
        })
    })
}

fn select_columns(row: &Value, cols: &[String]) -> Value {
    if let Some(obj) = row.as_object() {
        let filtered: serde_json::Map<String, Value> = cols
            .iter()
            .filter_map(|c| obj.get(c).map(|v| (c.clone(), v.clone())))
            .collect();
        Value::Object(filtered)
    } else {
        row.clone()
    }
}

fn apply_group_by(rows: &[&Value], group_col: &str, agg: &str, limit: usize) -> Vec<Value> {
    let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();

    for row in rows {
        let key = row
            .get(group_col)
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "null".to_string());
        groups.entry(key).or_default().push(row);
    }

    groups
        .into_iter()
        .take(limit)
        .map(|(key, group_rows)| {
            let mut map = serde_json::Map::new();
            map.insert(group_col.to_string(), Value::String(key));

            if agg == "count" {
                map.insert("count".to_string(), serde_json::json!(group_rows.len()));
            } else if let Some(col) = agg.strip_prefix("sum:") {
                let sum: f64 = group_rows
                    .iter()
                    .filter_map(|r| r.get(col).and_then(|v| v.as_f64()))
                    .sum();
                map.insert(format!("sum_{}", col), serde_json::json!(sum));
            } else if let Some(col) = agg.strip_prefix("avg:") {
                let vals: Vec<f64> = group_rows
                    .iter()
                    .filter_map(|r| r.get(col).and_then(|v| v.as_f64()))
                    .collect();
                let avg = if vals.is_empty() {
                    0.0
                } else {
                    vals.iter().sum::<f64>() / vals.len() as f64
                };
                map.insert(format!("avg_{}", col), serde_json::json!(avg));
            } else if let Some(col) = agg.strip_prefix("min:") {
                let min = group_rows
                    .iter()
                    .filter_map(|r| r.get(col).and_then(|v| v.as_f64()))
                    .fold(f64::INFINITY, f64::min);
                map.insert(format!("min_{}", col), serde_json::json!(min));
            } else if let Some(col) = agg.strip_prefix("max:") {
                let max = group_rows
                    .iter()
                    .filter_map(|r| r.get(col).and_then(|v| v.as_f64()))
                    .fold(f64::NEG_INFINITY, f64::max);
                map.insert(format!("max_{}", col), serde_json::json!(max));
            } else {
                map.insert("count".to_string(), serde_json::json!(group_rows.len()));
            }

            Value::Object(map)
        })
        .collect()
}

/// Validate a column name from user input (rejects anything that isn't alphanumeric/underscore).
pub fn validate_column_name(name: &str) -> Result<&str, AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest("Empty column name not allowed".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return Err(AppError::BadRequest(format!(
            "Invalid column name: '{}'. Only alphanumeric characters and underscores are allowed.",
            name
        )));
    }
    Ok(name)
}
