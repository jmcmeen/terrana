//! Geometry simplification via Douglas–Peucker or Visvalingam–Whyatt.
//!
//! `tolerance` is in degrees. When `preserve_topology` is true the
//! topology-preserving Visvalingam–Whyatt variant ([`SimplifyVw`]) is used;
//! otherwise plain Douglas–Peucker ([`Simplify`]). Geometry types other than
//! `Polygon`, `MultiPolygon`, and `LineString` are returned unchanged.

use geo::{Simplify, SimplifyVw};
use geo_types::Geometry;

/// Simplify `geom`, returning a geometry of the same variant.
///
/// ```
/// use geo_types::{line_string, Geometry};
/// use terrana::geometry::simplify::simplify_geometry;
///
/// let line = line_string![
///     (x: 0.0, y: 0.0), (x: 0.5, y: 0.01), (x: 1.0, y: 0.0),
/// ];
/// let simplified = simplify_geometry(Geometry::LineString(line), 0.1, false);
/// if let Geometry::LineString(ls) = simplified {
///     assert_eq!(ls.0.len(), 2); // the near-collinear midpoint is dropped
/// } else {
///     panic!("expected a LineString");
/// }
/// ```
pub fn simplify_geometry(
    geom: Geometry<f64>,
    tolerance: f64,
    preserve_topology: bool,
) -> Geometry<f64> {
    match geom {
        Geometry::Polygon(p) => {
            if preserve_topology {
                Geometry::Polygon(p.simplify_vw(tolerance))
            } else {
                Geometry::Polygon(p.simplify(tolerance))
            }
        }
        Geometry::MultiPolygon(mp) => {
            if preserve_topology {
                Geometry::MultiPolygon(mp.simplify_vw(tolerance))
            } else {
                Geometry::MultiPolygon(mp.simplify(tolerance))
            }
        }
        Geometry::LineString(ls) => {
            if preserve_topology {
                Geometry::LineString(ls.simplify_vw(tolerance))
            } else {
                Geometry::LineString(ls.simplify(tolerance))
            }
        }
        other => other,
    }
}
