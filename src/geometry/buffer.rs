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
    // Bearings increase clockwise (N→E→S→W), so the raw ring is clockwise. We
    // reverse it to counter-clockwise before closing: that satisfies the GeoJSON
    // right-hand rule, and — crucially — keeps `geodesic_area_unsigned` measuring
    // the disk rather than its complement (≈ the whole planet) for a CW ring.
    let mut coords: Vec<Coord<f64>> = (0..segments)
        .map(|i| {
            let bearing = (i as f64) * 360.0 / (segments as f64);
            let dest = Geodesic.destination(center, bearing, distance_m);
            Coord {
                x: dest.x(),
                y: dest.y(),
            }
        })
        .collect();
    coords.reverse();
    coords.push(coords[0]);
    Polygon::new(LineString::from(coords), vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::GeodesicArea;

    #[test]
    fn buffer_area_is_the_disk_not_its_complement() {
        // A 5 km buffer encloses ~78 km², not the rest of the planet. A clockwise
        // ring would yield ~510,000,000 km² (Earth's surface) here.
        let poly = compute_buffer(Point::new(-82.5, 36.5), 5000.0, 64);
        let area_km2 = poly.geodesic_area_unsigned() / 1_000_000.0;
        assert!(
            (70.0..85.0).contains(&area_km2),
            "expected ~78 km², got {area_km2}"
        );
    }
}
