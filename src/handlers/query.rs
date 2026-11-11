//! `GET /query` — radius, bounding-box, and nearest-neighbor spatial queries,
//! plus the shared `select`/`where`/`group_by`/`agg`/`limit`/`format` parameters.

use crate::db;
use crate::db::filter;
use crate::db::query::{DEFAULT_LIMIT, MAX_RESULT_LIMIT};
use crate::error::AppError;
use crate::output;
use crate::server::AppState;
use axum::extract::{Query, RawQuery, State};
use axum::http::{header, HeaderValue};
use axum::response::Response;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub radius: Option<String>,
    pub bbox: Option<String>,
    pub nearest: Option<usize>,
    pub select: Option<String>,
    pub group_by: Option<String>,
    pub agg: Option<String>,
    pub limit: Option<usize>,
    pub format: Option<String>,
}

/// Dispatch a `GET /query` request to the bbox, nearest, radius, or plain-table path
/// based on which parameters are present, then serialize to the requested format.
///
/// Results are paged: `offset` skips rows of the (stably ordered) result, and when
/// more rows follow the response carries a `Link: <...>; rel="next"` header.
pub async fn query(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    // Every `where` is kept (they AND together); other repeated keys keep the last.
    let filters = filter::parse_where(
        pairs
            .iter()
            .filter(|(k, _)| k == "where")
            .map(|(_, v)| v.as_str()),
    )?;
    let params: HashMap<String, String> = pairs.into_iter().collect();
    let qp = parse_query_params(&params)?;
    let offset = parse_offset(params.get("offset"))?;
    let limit = qp.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_RESULT_LIMIT);
    let format = qp.format.as_deref().unwrap_or("json");
    let snapshot = state.snapshot();
    let columns: Vec<(&str, &str)> = snapshot
        .schema
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.dtype.as_str()))
        .collect();
    let conditions = filter::to_conditions(&filters, &columns, &state.db)?;
    let select_cols = parse_select(qp.select.as_deref());
    // Every other column name must be a column too. Each is sent as typed: DuckDB
    // matches quoted names whatever their case, and the `agg` alias keeps it.
    let names: Vec<&str> = columns.iter().map(|(n, _)| *n).collect();
    // `agg` is used only with `group_by`, and checked only then.
    let agg_col = match (qp.group_by.as_deref(), qp.agg.as_deref()) {
        (Some(_), Some(a)) => db::query::parse_agg(a)?.1,
        _ => None,
    };
    for name in select_cols
        .iter()
        .flatten()
        .map(String::as_str)
        .chain(qp.group_by.as_deref())
        .chain(agg_col)
    {
        db::resolve_column(name, &names)?;
    }

    // (spatial filter, extra select, order by, page size) for each kind of query.
    let (spatial, extra, order_by, page_size) = if let Some(bbox_str) = &qp.bbox {
        let bbox = parse_bbox(bbox_str)?;
        // Ranges, not the R-tree's ST_Intersects: the page is sorted by rowid.
        let spatial = db::query::bbox_point_filter(bbox.0, bbox.1, bbox.2, bbox.3);
        (Some(spatial), None, None, limit)
    } else if let (Some(lat), Some(lon), Some(nearest)) = (qp.lat, qp.lon, qp.nearest) {
        // Nearest neighbor: the K nearest, or with `offset` the next K.
        let extra = db::query::distance_select(lat, lon);
        (
            Some("geom IS NOT NULL".to_string()),
            Some(extra),
            Some("_distance_km ASC"),
            nearest.min(MAX_RESULT_LIMIT),
        )
    } else if let (Some(lat), Some(lon), Some(radius_str)) = (qp.lat, qp.lon, &qp.radius) {
        let radius_m = parse_radius(radius_str)?;
        let spatial = db::query::radius_filter(lat, lon, radius_m);
        let extra = db::query::distance_select(lat, lon);
        (Some(spatial), Some(extra), Some("_distance_km ASC"), limit)
    } else {
        // No spatial filter — plain table query
        (None, None, None, limit)
    };

    if page_size == 0 {
        return output::format_response(&[], format, &state);
    }

    // One row past the page says whether another page follows.
    let mut rows = db::query::query_page(
        &state.db,
        spatial.as_deref(),
        &conditions,
        select_cols.as_deref(),
        qp.group_by.as_deref(),
        qp.agg.as_deref(),
        page_size + 1,
        offset,
        extra.as_deref(),
        order_by,
    )?;
    let has_next = rows.len() > page_size;
    rows.truncate(page_size);

    let mut response = output::format_response(&rows, format, &state)?;
    if has_next {
        let link = next_link(raw_query.as_deref(), offset.saturating_add(page_size));
        let value = HeaderValue::from_str(&link)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Link header error: {}", e)))?;
        response.headers_mut().insert(header::LINK, value);
    }
    Ok(response)
}

/// `offset` must be a non-negative integer when given; absent means 0.
fn parse_offset(value: Option<&String>) -> Result<usize, AppError> {
    match value {
        None => Ok(0),
        // DuckDB's OFFSET is a signed 64-bit integer.
        Some(v) => match v.trim().parse::<usize>() {
            Ok(n) if i64::try_from(n).is_ok() => Ok(n),
            _ => Err(AppError::BadRequest(format!(
                "Invalid offset: '{}'. Use a non-negative integer",
                v
            ))),
        },
    }
}

/// `Link` header value for the next page: the request's own query string (kept
/// as sent) with `offset` replaced. The target is the relative reference `?...`,
/// which resolves against the request's own path, so it stays right behind a
/// proxy that mounts the API under a prefix. Bytes a header can't carry (raw
/// non-ASCII, spaces, controls) are percent-encoded.
fn next_link(raw_query: Option<&str>, next_offset: usize) -> String {
    let mut parts: Vec<&str> = raw_query
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty() && !is_offset_param(p))
        .collect();
    let offset_part = format!("offset={}", next_offset);
    parts.push(&offset_part);
    format!(
        "<?{}>; rel=\"next\"",
        encode_header_unsafe(&parts.join("&"))
    )
}

/// Whether a raw `key=value` pair sets `offset`, however the key is encoded.
fn is_offset_param(pair: &str) -> bool {
    percent_decode(pair.split('=').next().unwrap_or("")) == b"offset"
}

/// Decode `%XX` escapes (and `+` as space), as form-encoded query keys are.
fn percent_decode(s: &str) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    out
}

/// Percent-encode every byte outside visible ASCII, plus `<` and `>` (they
/// delimit the target in a `Link` header).
fn encode_header_unsafe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if (0x21..=0x7e).contains(&b) && b != b'<' && b != b'>' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn parse_query_params(params: &HashMap<String, String>) -> Result<QueryParams, AppError> {
    Ok(QueryParams {
        lat: params.get("lat").and_then(|v| v.parse().ok()),
        lon: params.get("lon").and_then(|v| v.parse().ok()),
        radius: params.get("radius").cloned(),
        bbox: params.get("bbox").cloned(),
        nearest: params.get("nearest").and_then(|v| v.parse().ok()),
        select: params.get("select").cloned(),
        group_by: params.get("group_by").cloned(),
        agg: params.get("agg").cloned(),
        limit: params.get("limit").and_then(|v| v.parse().ok()),
        format: params.get("format").cloned(),
    })
}

fn parse_bbox(s: &str) -> Result<(f64, f64, f64, f64), AppError> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            AppError::BadRequest(
                "Invalid bbox format. Expected: minlat,minlon,maxlat,maxlon".into(),
            )
        })?;
    if parts.len() != 4 {
        return Err(AppError::BadRequest(
            "bbox requires exactly 4 values: minlat,minlon,maxlat,maxlon".into(),
        ));
    }
    let (min_lat, min_lon, max_lat, max_lon) = (parts[0], parts[1], parts[2], parts[3]);
    if !(-90.0..=90.0).contains(&min_lat) || !(-90.0..=90.0).contains(&max_lat) {
        return Err(AppError::BadRequest(
            "bbox latitudes must be within [-90, 90]".into(),
        ));
    }
    if !(-180.0..=180.0).contains(&min_lon) || !(-180.0..=180.0).contains(&max_lon) {
        return Err(AppError::BadRequest(
            "bbox longitudes must be within [-180, 180]".into(),
        ));
    }
    if min_lat > max_lat || min_lon > max_lon {
        return Err(AppError::BadRequest(
            "bbox requires minlat<=maxlat and minlon<=maxlon".into(),
        ));
    }
    Ok((min_lat, min_lon, max_lat, max_lon))
}

fn parse_radius(s: &str) -> Result<f64, AppError> {
    let s = s.trim();
    if let Some(val) = s.strip_suffix("km") {
        val.trim().parse::<f64>().map(|v| v * 1000.0)
    } else if let Some(val) = s.strip_suffix("mi") {
        val.trim().parse::<f64>().map(|v| v * 1609.344)
    } else if let Some(val) = s.strip_suffix("ft") {
        val.trim().parse::<f64>().map(|v| v * 0.3048)
    } else if let Some(val) = s.strip_suffix('m') {
        val.trim().parse::<f64>()
    } else {
        s.parse::<f64>()
    }
    .map_err(|_| {
        AppError::BadRequest(format!(
            "Invalid radius: '{}'. Use e.g. 10km, 5000m, 3mi",
            s
        ))
    })
}

fn parse_select(select: Option<&str>) -> Option<Vec<String>> {
    select.map(|s| s.split(',').map(|c| c.trim().to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_parses_valid_input() {
        assert_eq!(
            parse_bbox("35.0,-84.0,37.0,-81.0").unwrap(),
            (35.0, -84.0, 37.0, -81.0)
        );
    }

    #[test]
    fn bbox_rejects_out_of_range_and_inverted() {
        assert!(parse_bbox("35.0,-84.0,37.0").is_err()); // too few
        assert!(parse_bbox("-100.0,0.0,10.0,1.0").is_err()); // lat < -90
        assert!(parse_bbox("0.0,-200.0,1.0,1.0").is_err()); // lon < -180
        assert!(parse_bbox("37.0,-81.0,35.0,-84.0").is_err()); // min > max
        assert!(parse_bbox("a,b,c,d").is_err()); // non-numeric
    }

    #[test]
    fn radius_parses_units() {
        assert_eq!(parse_radius("10km").unwrap(), 10_000.0);
        assert_eq!(parse_radius("5000m").unwrap(), 5000.0);
        assert_eq!(parse_radius("1mi").unwrap(), 1609.344);
        assert_eq!(parse_radius("100").unwrap(), 100.0);
        assert!(parse_radius("abc").is_err());
    }

    #[test]
    fn offset_defaults_to_zero_and_rejects_bad_values() {
        assert_eq!(parse_offset(None).unwrap(), 0);
        assert_eq!(parse_offset(Some(&"25".to_string())).unwrap(), 25);
        assert!(parse_offset(Some(&"-1".to_string())).is_err());
        assert!(parse_offset(Some(&"ten".to_string())).is_err());
        assert!(parse_offset(Some(&u64::MAX.to_string())).is_err());
    }

    #[test]
    fn next_link_replaces_offset_and_keeps_the_rest() {
        assert_eq!(
            next_link(Some("bbox=35,-84,37,-81&offset=10&limit=10"), 20),
            "<?bbox=35,-84,37,-81&limit=10&offset=20>; rel=\"next\""
        );
        assert_eq!(
            next_link(Some("where=species:Eastern%20Bluebird"), 1000),
            "<?where=species:Eastern%20Bluebird&offset=1000>; rel=\"next\""
        );
        assert_eq!(next_link(None, 5), "<?offset=5>; rel=\"next\"");
        // A raw non-ASCII byte can't go into a header; it is encoded.
        assert_eq!(
            next_link(Some("where=species:Mésange"), 1),
            "<?where=species:M%C3%A9sange&offset=1>; rel=\"next\""
        );
        // An encoded `offset` key is still the offset.
        assert_eq!(
            next_link(Some("off%73et=5&limit=2"), 7),
            "<?limit=2&offset=7>; rel=\"next\""
        );
    }

    #[test]
    fn select_trims_columns() {
        assert_eq!(
            parse_select(Some("species, observed_on")),
            Some(vec!["species".to_string(), "observed_on".to_string()])
        );
        assert_eq!(parse_select(None), None);
    }
}
