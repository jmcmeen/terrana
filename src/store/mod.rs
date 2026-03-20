pub mod loader;

use crate::error::AppError;
use serde_json::Value;
use std::collections::HashMap;

const NULL_INT: i64 = i64::MIN;
const NULL_TEXT: u32 = u32::MAX;

pub enum Column {
    Int(Vec<i64>),    // NULL_INT = null
    Float(Vec<f64>),  // NaN = null
    Text(Vec<u32>),   // index into StringTable, NULL_TEXT = null
}

pub struct StringTable {
    pub values: Vec<String>,
    index: HashMap<String, u32>,
}

impl StringTable {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.index.get(s) {
            return idx;
        }
        let idx = self.values.len() as u32;
        self.values.push(s.to_string());
        self.index.insert(s.to_string(), idx);
        idx
    }
}

pub struct DataTable {
    pub col_names: Vec<String>,
    pub col_types: Vec<String>,
    pub columns: Vec<Column>,
    pub strings: StringTable,
    pub row_count: usize,
}

impl DataTable {
    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.col_names.iter().position(|n| n == name)
    }

    pub fn get_f64(&self, col_idx: usize, row_idx: usize) -> Option<f64> {
        match &self.columns[col_idx] {
            Column::Float(v) => {
                let val = v[row_idx];
                if val.is_nan() {
                    None
                } else {
                    Some(val)
                }
            }
            Column::Int(v) => {
                let val = v[row_idx];
                if val == NULL_INT {
                    None
                } else {
                    Some(val as f64)
                }
            }
            _ => None,
        }
    }

    fn cell_to_value(&self, col_idx: usize, row_idx: usize) -> Value {
        match &self.columns[col_idx] {
            Column::Int(v) => {
                let val = v[row_idx];
                if val == NULL_INT {
                    Value::Null
                } else {
                    Value::Number(val.into())
                }
            }
            Column::Float(v) => {
                let val = v[row_idx];
                if val.is_nan() {
                    Value::Null
                } else {
                    serde_json::Number::from_f64(val)
                        .map(Value::Number)
                        .unwrap_or(Value::Null)
                }
            }
            Column::Text(v) => {
                let idx = v[row_idx];
                if idx == NULL_TEXT {
                    Value::Null
                } else {
                    Value::String(self.strings.values[idx as usize].clone())
                }
            }
        }
    }

    fn row_to_value(&self, row_idx: usize) -> Value {
        let mut map = serde_json::Map::with_capacity(self.col_names.len());
        for (col_idx, name) in self.col_names.iter().enumerate() {
            map.insert(name.clone(), self.cell_to_value(col_idx, row_idx));
        }
        Value::Object(map)
    }

    fn row_to_value_select(&self, row_idx: usize, col_indices: &[usize]) -> Value {
        let mut map = serde_json::Map::with_capacity(col_indices.len());
        for &col_idx in col_indices {
            map.insert(
                self.col_names[col_idx].clone(),
                self.cell_to_value(col_idx, row_idx),
            );
        }
        Value::Object(map)
    }

    fn cell_matches(&self, col_idx: usize, row_idx: usize, val: &str) -> bool {
        match &self.columns[col_idx] {
            Column::Int(v) => {
                let cell = v[row_idx];
                if cell == NULL_INT {
                    return false;
                }
                val.parse::<i64>().map_or(false, |target| cell == target)
            }
            Column::Float(v) => {
                let cell = v[row_idx];
                if cell.is_nan() {
                    return false;
                }
                val.parse::<f64>().map_or(false, |target| cell == target)
            }
            Column::Text(v) => {
                let idx = v[row_idx];
                if idx == NULL_TEXT {
                    return false;
                }
                self.strings.values[idx as usize] == val
            }
        }
    }

    fn cell_group_key(&self, col_idx: usize, row_idx: usize) -> String {
        match &self.columns[col_idx] {
            Column::Int(v) => {
                let val = v[row_idx];
                if val == NULL_INT {
                    "null".to_string()
                } else {
                    val.to_string()
                }
            }
            Column::Float(v) => {
                let val = v[row_idx];
                if val.is_nan() {
                    "null".to_string()
                } else {
                    val.to_string()
                }
            }
            Column::Text(v) => {
                let idx = v[row_idx];
                if idx == NULL_TEXT {
                    "null".to_string()
                } else {
                    self.strings.values[idx as usize].clone()
                }
            }
        }
    }

    pub fn get_rows_by_ids(&self, ids: &[i64]) -> Vec<Value> {
        ids.iter()
            .filter_map(|&id| {
                let idx = (id - 1) as usize;
                if idx < self.row_count {
                    Some(self.row_to_value(idx))
                } else {
                    None
                }
            })
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
        let where_col_indices: Vec<(usize, String)> = where_clauses
            .iter()
            .filter_map(|(col, val)| self.col_index(col).map(|idx| (idx, val.clone())))
            .collect();

        let select_indices: Option<Vec<usize>> = select_cols.map(|cols| {
            cols.iter()
                .filter_map(|c| self.col_index(c))
                .collect()
        });

        // Aggregation path — stream with O(num_groups) memory
        if let (Some(gb), Some(a)) = (group_by, agg) {
            let gb_idx = self
                .col_index(gb)
                .ok_or_else(|| AppError::BadRequest(format!("Column '{}' not found", gb)))?;
            let agg_col_idx = parse_agg_col(a).and_then(|col| self.col_index(col));
            return Ok(self.aggregate(ids, &where_col_indices, gb_idx, gb, a, agg_col_idx, limit));
        }

        // Non-aggregation path — stream up to limit
        let mut result = Vec::new();

        let iter: Box<dyn Iterator<Item = usize>> = match ids {
            Some(ids) => Box::new(ids.iter().filter_map(|&id| {
                let idx = (id - 1) as usize;
                if idx < self.row_count {
                    Some(idx)
                } else {
                    None
                }
            })),
            None => Box::new(0..self.row_count),
        };

        for row_idx in iter {
            if !where_col_indices
                .iter()
                .all(|(ci, v)| self.cell_matches(*ci, row_idx, v))
            {
                continue;
            }
            let row = match &select_indices {
                Some(indices) => self.row_to_value_select(row_idx, indices),
                None => self.row_to_value(row_idx),
            };
            result.push(row);
            if result.len() >= limit {
                break;
            }
        }

        Ok(result)
    }

    fn aggregate(
        &self,
        ids: Option<&[i64]>,
        where_col_indices: &[(usize, String)],
        gb_idx: usize,
        gb_name: &str,
        agg: &str,
        agg_col_idx: Option<usize>,
        limit: usize,
    ) -> Vec<Value> {
        struct Acc {
            count: usize,
            sum: f64,
            min: f64,
            max: f64,
        }

        let mut groups: HashMap<String, Acc> = HashMap::new();

        let iter: Box<dyn Iterator<Item = usize>> = match ids {
            Some(ids) => Box::new(ids.iter().filter_map(|&id| {
                let idx = (id - 1) as usize;
                if idx < self.row_count {
                    Some(idx)
                } else {
                    None
                }
            })),
            None => Box::new(0..self.row_count),
        };

        for row_idx in iter {
            if !where_col_indices
                .iter()
                .all(|(ci, v)| self.cell_matches(*ci, row_idx, v))
            {
                continue;
            }
            let key = self.cell_group_key(gb_idx, row_idx);
            let acc = groups.entry(key).or_insert(Acc {
                count: 0,
                sum: 0.0,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
            });
            acc.count += 1;
            if let Some(ci) = agg_col_idx {
                if let Some(v) = self.get_f64(ci, row_idx) {
                    acc.sum += v;
                    if v < acc.min {
                        acc.min = v;
                    }
                    if v > acc.max {
                        acc.max = v;
                    }
                }
            }
        }

        groups
            .into_iter()
            .take(limit)
            .map(|(key, acc)| {
                let mut map = serde_json::Map::new();
                map.insert(gb_name.to_string(), Value::String(key));

                if agg == "count" {
                    map.insert("count".to_string(), serde_json::json!(acc.count));
                } else if let Some(col) = agg.strip_prefix("sum:") {
                    map.insert(format!("sum_{}", col), serde_json::json!(acc.sum));
                } else if let Some(col) = agg.strip_prefix("avg:") {
                    let avg = if acc.count > 0 {
                        acc.sum / acc.count as f64
                    } else {
                        0.0
                    };
                    map.insert(format!("avg_{}", col), serde_json::json!(avg));
                } else if let Some(col) = agg.strip_prefix("min:") {
                    map.insert(format!("min_{}", col), serde_json::json!(acc.min));
                } else if let Some(col) = agg.strip_prefix("max:") {
                    map.insert(format!("max_{}", col), serde_json::json!(acc.max));
                } else {
                    map.insert("count".to_string(), serde_json::json!(acc.count));
                }

                Value::Object(map)
            })
            .collect()
    }
}

fn parse_agg_col(agg: &str) -> Option<&str> {
    agg.strip_prefix("sum:")
        .or_else(|| agg.strip_prefix("avg:"))
        .or_else(|| agg.strip_prefix("min:"))
        .or_else(|| agg.strip_prefix("max:"))
}

/// Validate a column name from user input.
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
