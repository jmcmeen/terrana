//! Geodesic buffer: a polygon ring around a center point.
//!
//! Each ring vertex is found by shooting a geodesic ray from the center at an
//! evenly-spaced bearing for `distance_m` metres using
//! [`Destination::destination`] on the WGS 84 [`Geodesic`] — never planar offset.

use geo::{Destination, Geodesic};
use geo_types::{Coord, LineString, Point, Polygon};

/// Build a geodesic buffer ring of `segments` vertices around `center`, each
/// vertex `distance_m` metres from the center. The returned polygon's exterior
/// ring is explicitly closed.
///
/// ```
/// use geo_types::Point;
/// use terrana::geometry::buffer::compute_buffer;
///
/// let poly = compute_buffer(Point::new(0.0, 0.0), 1000.0, 64);
/// // Closed ring: segments + 1 coordinates.
/// assert_eq!(poly.exterior().0.len(), 65);
/// ```
pub fn compute_buffer(center: Point<f64>, distance_m: f64, segments: usize) -> Polygon<f64> {
    let mut coords = Vec::with_capacity(segments + 1);
    for i in 0..segments {
        let bearing = (i as f64) * 360.0 / (segments as f64);
        let dest = Geodesic.destination(center, bearing, distance_m);
        coords.push(Coord {
            x: dest.x(),
            y: dest.y(),
        });
    }
    coords.push(coords[0]);
    Polygon::new(LineString::from(coords), vec![])
}
