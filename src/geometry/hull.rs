//! Convex hull of a point set, with geodesic area and perimeter of the hull.
//!
//! The hull *shape* is computed in 2D lat/lon space (acceptable per the project's
//! geodesic rules), but the reported area and perimeter are geodesic.

use geo::{ConvexHull, GeodesicArea};
use geo_types::{MultiPoint, Point, Polygon};

/// A convex hull polygon plus its geodesic metrics and the number of input points.
#[derive(Debug, Clone)]
pub struct HullResult {
    pub hull: Polygon<f64>,
    pub area_m2: f64,
    pub area_km2: f64,
    pub area_ha: f64,
    pub perimeter_m: f64,
    pub point_count: usize,
}

/// Compute the convex hull of `points` and its geodesic area/perimeter.
///
/// The caller is responsible for ensuring at least 3 points are supplied; with
/// fewer points the hull degenerates to a line or point with zero area.
///
/// ```
/// use geo_types::Point;
/// use terrana::geometry::hull::compute_convex_hull;
///
/// let pts = [
///     Point::new(0.0, 0.0),
///     Point::new(1.0, 0.0),
///     Point::new(1.0, 1.0),
///     Point::new(0.0, 1.0),
///     Point::new(0.5, 0.5), // interior point — dropped by the hull
/// ];
/// let result = compute_convex_hull(&pts);
/// assert_eq!(result.point_count, 5);
/// assert!(result.area_km2 > 0.0);
/// ```
pub fn compute_convex_hull(points: &[Point<f64>]) -> HullResult {
    let multi_point = MultiPoint::from(points.to_vec());
    let hull = multi_point.convex_hull();
    let area_m2 = hull.geodesic_area_unsigned();
    let perimeter_m = hull.geodesic_perimeter();
    HullResult {
        hull,
        area_m2,
        area_km2: area_m2 / 1_000_000.0,
        area_ha: area_m2 / 10_000.0,
        perimeter_m,
        point_count: points.len(),
    }
}
