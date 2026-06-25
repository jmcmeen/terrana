//! Dissolve: group points by an attribute and compute a convex hull per group.
//!
//! Each group with at least 3 points becomes a GeoJSON `Feature` whose geometry
//! is the group's convex hull. Optional `count` and geodesic `area` properties
//! can be attached. Groups with fewer than 3 points are skipped (no valid hull).

use crate::geometry::hull::compute_convex_hull;
use geo_types::Point;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Build one hull `Feature` per group. `by` names the property key under which
/// each group's key is stored. Returns GeoJSON `Feature` objects ready to be
/// wrapped in a `FeatureCollection`.
///
/// ```
/// use std::collections::HashMap;
/// use geo_types::Point;
/// use terrana::geometry::dissolve::dissolve_by;
///
/// let mut groups: HashMap<String, Vec<Point<f64>>> = HashMap::new();
/// groups.insert(
///     "a".to_string(),
///     vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(0.0, 1.0)],
/// );
/// let features = dissolve_by(&groups, "kind", true, true);
/// assert_eq!(features.len(), 1);
/// assert_eq!(features[0]["properties"]["kind"], "a");
/// assert_eq!(features[0]["properties"]["count"], 3);
/// ```
pub fn dissolve_by(
    groups: &HashMap<String, Vec<Point<f64>>>,
    by: &str,
    include_area: bool,
    include_count: bool,
) -> Vec<Value> {
    let mut features = Vec::new();
    for (key, pts) in groups {
        if pts.len() < 3 {
            continue;
        }
        let result = compute_convex_hull(pts);
        let hull_geojson: geojson::Geometry = (&result.hull).into();

        let mut props = serde_json::Map::new();
        props.insert(by.to_string(), json!(key));
        if include_count {
            props.insert("count".to_string(), json!(pts.len()));
        }
        if include_area {
            props.insert("area_m2".to_string(), json!(result.area_m2));
            props.insert("area_km2".to_string(), json!(result.area_km2));
        }

        features.push(json!({
            "type": "Feature",
            "geometry": serde_json::to_value(&hull_geojson).unwrap_or(json!(null)),
            "properties": props,
        }));
    }
    features
}
