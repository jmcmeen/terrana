//! Geodesic measurements: point-to-point distance and bearing, the bounding box
//! of a geometry, and centroids.
//!
//! Distance and bearing use the WGS 84 [`Geodesic`] (ellipsoidal, high precision)
//! — never the haversine/spherical approximation used on the query path.

use geo::{Bearing, BoundingRect, Centroid, Distance, Geodesic, GeodesicArea};
use geo_types::{Geometry, LineString, Point, Polygon};
use serde::Serialize;

/// Geodesic distance (several units) and forward bearing between two points.
#[derive(Debug, Clone, Serialize)]
pub struct DistanceResult {
    pub distance_m: f64,
    pub distance_km: f64,
    pub distance_mi: f64,
    pub bearing_deg: f64,
}

/// The axis-aligned bounding box of a geometry, with its envelope polygon and
/// geodesic dimensions.
#[derive(Debug, Clone)]
pub struct BoundsResult {
    /// `[min_lat, min_lon, max_lat, max_lon]`.
    pub bbox: [f64; 4],
    /// The bounding box as a closed rectangle polygon.
    pub envelope: Polygon<f64>,
    pub width_km: f64,
    pub height_km: f64,
    pub area_km2: f64,
}

/// Compute the geodesic distance and forward bearing from `from` to `to`.
///
/// Points are `(lon, lat)` as in GeoJSON.
///
/// ```
/// use geo_types::Point;
/// use terrana::geometry::measure::geodesic_distance;
///
/// // One degree of longitude at the equator is ~111 km.
/// let d = geodesic_distance(Point::new(0.0, 0.0), Point::new(1.0, 0.0));
/// assert!((d.distance_km - 111.3).abs() < 1.0);
/// assert!((d.bearing_deg - 90.0).abs() < 0.1);
/// ```
pub fn geodesic_distance(from: Point<f64>, to: Point<f64>) -> DistanceResult {
    let distance_m = Geodesic.distance(from, to);
    let bearing_deg = Geodesic.bearing(from, to);
    DistanceResult {
        distance_m,
        distance_km: distance_m / 1000.0,
        distance_mi: distance_m / 1609.344,
        bearing_deg,
    }
}

/// Compute the bounding box of `geom`. Returns `None` when the geometry is empty
/// and has no bounding rectangle.
///
/// ```
/// use geo_types::{polygon, Geometry};
/// use terrana::geometry::measure::bounding_box;
///
/// let square = polygon![
///     (x: 0.0, y: 0.0), (x: 1.0, y: 0.0), (x: 1.0, y: 1.0), (x: 0.0, y: 1.0), (x: 0.0, y: 0.0),
/// ];
/// let bounds = bounding_box(&Geometry::Polygon(square)).unwrap();
/// assert_eq!(bounds.bbox, [0.0, 0.0, 1.0, 1.0]);
/// assert!(bounds.width_km > 0.0 && bounds.height_km > 0.0);
/// ```
pub fn bounding_box(geom: &Geometry<f64>) -> Option<BoundsResult> {
    let rect = geom.bounding_rect()?;
    let min_lat = rect.min().y;
    let min_lon = rect.min().x;
    let max_lat = rect.max().y;
    let max_lon = rect.max().x;

    let width_m = Geodesic.distance(Point::new(min_lon, min_lat), Point::new(max_lon, min_lat));
    let height_m = Geodesic.distance(Point::new(min_lon, min_lat), Point::new(min_lon, max_lat));

    let envelope = Polygon::new(
        LineString::from(vec![
            (min_lon, min_lat),
            (max_lon, min_lat),
            (max_lon, max_lat),
            (min_lon, max_lat),
            (min_lon, min_lat),
        ]),
        vec![],
    );
    let area_m2 = envelope.geodesic_area_unsigned();

    Some(BoundsResult {
        bbox: [min_lat, min_lon, max_lat, max_lon],
        envelope,
        width_km: width_m / 1000.0,
        height_km: height_m / 1000.0,
        area_km2: area_m2 / 1_000_000.0,
    })
}

/// Compute the centroid of any geometry, or `None` if it has no centroid
/// (e.g. an empty geometry).
///
/// ```
/// use geo_types::{polygon, Geometry};
/// use terrana::geometry::measure::centroid;
///
/// let square = polygon![
///     (x: 0.0, y: 0.0), (x: 2.0, y: 0.0), (x: 2.0, y: 2.0), (x: 0.0, y: 2.0), (x: 0.0, y: 0.0),
/// ];
/// let c = centroid(&Geometry::Polygon(square)).unwrap();
/// assert_eq!((c.x(), c.y()), (1.0, 1.0));
/// ```
pub fn centroid(geom: &Geometry<f64>) -> Option<Point<f64>> {
    geom.centroid()
}
