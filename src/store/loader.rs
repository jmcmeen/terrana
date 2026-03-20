use crate::error::AppError;
use crate::store::DataTable;
use serde_json::Value;
use std::path::Path;
use tracing::info;

pub fn load_file(path: &Path) -> Result<DataTable, AppError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    info!(ext = %extension, "loading file");

    match extension.as_str() {
        "csv" => load_csv(path),
        "geojson" | "json" => load_geojson(path),
        _ => Err(AppError::BadRequest(format!(
            "Unsupported file type: {}",
            extension
        ))),
    }
}

fn load_csv(path: &Path) -> Result<DataTable, AppError> {
    let mut reader = csv::ReaderBuilder::new()
        .from_path(path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV read error: {}", e)))?;

    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV header error: {}", e)))?
        .iter()
        .map(|h| h.to_string())
        .collect();

    let mut rows = Vec::new();
    let mut type_hints: Vec<Option<&str>> = vec![None; headers.len()];

    for (i, result) in reader.records().enumerate() {
        let record =
            result.map_err(|e| AppError::Internal(anyhow::anyhow!("CSV row error: {}", e)))?;
        let mut map = serde_json::Map::new();
        map.insert("rowid".to_string(), Value::Number((i as i64 + 1).into()));

        for (j, field) in record.iter().enumerate() {
            if j >= headers.len() {
                continue;
            }
            let val = parse_csv_field(field);
            if type_hints[j].is_none() && !val.is_null() {
                type_hints[j] = Some(match &val {
                    Value::Number(n) if n.is_i64() => "INTEGER",
                    Value::Number(_) => "DOUBLE",
                    Value::Bool(_) => "BOOLEAN",
                    _ => "VARCHAR",
                });
            }
            map.insert(headers[j].clone(), val);
        }

        rows.push(Value::Object(map));

        if (i + 1) % 1_000_000 == 0 {
            info!(rows_loaded = i + 1, "loading CSV");
        }
    }

    let row_count = rows.len() as i64;
    info!(rows = row_count, "file loaded");

    let mut columns = vec![("rowid".to_string(), "INTEGER".to_string())];
    for (j, name) in headers.iter().enumerate() {
        let dtype = type_hints[j].unwrap_or("VARCHAR").to_string();
        columns.push((name.clone(), dtype));
    }

    Ok(DataTable {
        columns,
        rows,
        row_count,
    })
}

fn parse_csv_field(field: &str) -> Value {
    if field.is_empty() {
        return Value::Null;
    }
    if let Ok(i) = field.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = field.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    match field {
        "true" | "TRUE" => return Value::Bool(true),
        "false" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    Value::String(field.to_string())
}

fn load_geojson(path: &Path) -> Result<DataTable, AppError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("GeoJSON read error: {}", e)))?;
    let geojson: geojson::GeoJson = content
        .parse()
        .map_err(|e| AppError::BadRequest(format!("Invalid GeoJSON: {}", e)))?;

    let features = match geojson {
        geojson::GeoJson::FeatureCollection(fc) => fc.features,
        _ => return Err(AppError::BadRequest("Expected a FeatureCollection".into())),
    };

    let mut rows = Vec::new();
    let mut all_keys: Vec<String> = Vec::new();
    let mut key_set = std::collections::HashSet::new();

    for (i, feat) in features.iter().enumerate() {
        let mut map = serde_json::Map::new();
        map.insert("rowid".to_string(), Value::Number((i as i64 + 1).into()));

        if let Some(props) = &feat.properties {
            for (k, v) in props {
                if !key_set.contains(k) {
                    key_set.insert(k.clone());
                    all_keys.push(k.clone());
                }
                map.insert(k.clone(), v.clone());
            }
        }

        if let Some(geom) = &feat.geometry {
            use geo::Centroid;
            use std::convert::TryInto;
            if let Ok(geo_geom) = TryInto::<geo_types::Geometry<f64>>::try_into(geom.clone()) {
                if let Some(c) = geo_geom.centroid() {
                    map.insert("latitude".to_string(), serde_json::json!(c.y()));
                    map.insert("longitude".to_string(), serde_json::json!(c.x()));
                }
            }
        }

        rows.push(Value::Object(map));
    }

    let row_count = rows.len() as i64;
    info!(rows = row_count, "GeoJSON loaded");

    let mut columns = vec![("rowid".to_string(), "INTEGER".to_string())];
    for key in &all_keys {
        columns.push((key.clone(), "VARCHAR".to_string()));
    }
    let has_geometry = features.iter().any(|f| f.geometry.is_some());
    if has_geometry && !key_set.contains("latitude") {
        columns.push(("latitude".to_string(), "DOUBLE".to_string()));
        columns.push(("longitude".to_string(), "DOUBLE".to_string()));
    }

    Ok(DataTable {
        columns,
        rows,
        row_count,
    })
}

const LAT_CANDIDATES: &[&str] = &["latitude", "lat", "y", "ylat", "geo_lat"];
const LON_CANDIDATES: &[&str] = &[
    "longitude",
    "lon",
    "lng",
    "x",
    "xlon",
    "xlong",
    "geo_lon",
    "geo_lng",
];

pub fn detect_lat_lon(
    table: &DataTable,
    lat_override: Option<&str>,
    lon_override: Option<&str>,
) -> Result<(String, String), AppError> {
    let col_names: Vec<String> = table.columns.iter().map(|c| c.0.clone()).collect();
    let col_lower: Vec<String> = col_names.iter().map(|c| c.to_lowercase()).collect();

    let lat_col = if let Some(l) = lat_override {
        if !col_lower.contains(&l.to_lowercase()) {
            return Err(AppError::ColumnNotFound(format!(
                "'{}'. Available: {}",
                l,
                col_names.join(", ")
            )));
        }
        l.to_string()
    } else {
        detect_column(&col_names, &col_lower, LAT_CANDIDATES, "latitude")?
    };

    let lon_col = if let Some(l) = lon_override {
        if !col_lower.contains(&l.to_lowercase()) {
            return Err(AppError::ColumnNotFound(format!(
                "'{}'. Available: {}",
                l,
                col_names.join(", ")
            )));
        }
        l.to_string()
    } else {
        detect_column(&col_names, &col_lower, LON_CANDIDATES, "longitude")?
    };

    Ok((lat_col, lon_col))
}

fn detect_column(
    columns: &[String],
    col_lower: &[String],
    candidates: &[&str],
    label: &str,
) -> Result<String, AppError> {
    for candidate in candidates {
        if let Some(idx) = col_lower.iter().position(|c| c == candidate) {
            return Ok(columns[idx].clone());
        }
    }
    Err(AppError::BadRequest(format!(
        "Could not auto-detect {} column. Available columns: {}. Use --lat/--lon to specify.",
        label,
        columns.join(", ")
    )))
}
