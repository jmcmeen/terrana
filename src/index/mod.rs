pub mod build;

use rstar::{RTreeObject, AABB};

#[derive(Clone, Debug)]
pub struct SpatialPoint {
    pub rowid: i64,
    pub lat: f64,
    pub lon: f64,
}

impl RTreeObject for SpatialPoint {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point([self.lon, self.lat]) // lon first (x), lat second (y)
    }
}

impl rstar::PointDistance for SpatialPoint {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let dx = self.lon - point[0];
        let dy = self.lat - point[1];
        dx * dx + dy * dy
    }
}
