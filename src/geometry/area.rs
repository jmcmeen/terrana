//! Geodesic area and perimeter for polygons.
//!
//! Area uses [`GeodesicArea::geodesic_area_unsigned`] (Karney's algorithm on the
//! WGS 84 ellipsoid) — never planar. Perimeter is the geodesic perimeter.

use geo::GeodesicArea;
use geo_types::Polygon;
use serde::Serialize;

/// Geodesic area (in several units) and perimeter of one or more polygons.
///
/// When multiple polygons are supplied the areas and perimeters are summed,
/// matching the behaviour of a GeoJSON `MultiPolygon`.
#[derive(Debug, Clone, Serialize)]
pub struct AreaResult {
    pub area_m2: f64,
    pub area_km2: f64,
    pub area_ha: f64,
    pub area_acres: f64,
    pub perimeter_m: f64,
}

/// Compute the total geodesic area and perimeter of `polygons`.
///
/// ```
/// use geo_types::polygon;
/// use terrana::geometry::area::compute_area;
///
/// // A 1° × 1° box at the equator is ~12,308 km² on the WGS 84 ellipsoid.
/// let square = polygon![
///     (x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0),
/// ];
/// let result = compute_area(std::slice::from_ref(&square));
/// assert!((12_000.0..12_700.0).contains(&result.area_km2));
/// ```
pub fn compute_area(polygons: &[Polygon<f64>]) -> AreaResult {
    let mut area_m2 = 0.0_f64;
    let mut perimeter_m = 0.0_f64;
    for poly in polygons {
        area_m2 += poly.geodesic_area_unsigned();
        perimeter_m += poly.geodesic_perimeter();
    }
    AreaResult {
        area_m2,
        area_km2: area_m2 / 1_000_000.0,
        area_ha: area_m2 / 10_000.0,
        area_acres: area_m2 / 4_046.8564224,
        perimeter_m,
    }
}
