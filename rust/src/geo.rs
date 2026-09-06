//! Coordinate math. Mirrors `latlon_to_local` / `world_to_screen` from the
//! Python original, with the hot-loop improvement of hoisting
//! `cos(lat0)` (Python recomputed it per point).

/// Mean Earth radius, meters.
pub const EARTH_R_M: f64 = 6_371_000.0;

/// Equirectangular approximation -> local meters (x = east, y = north).
/// Accurate enough for play areas up to a few km across.
#[inline]
pub fn latlon_to_local(lat: f64, lon: f64, lat0: f64, lon0: f64) -> (f64, f64) {
    let cos_lat0 = lat0.to_radians().cos();
    latlon_to_local_with_cos(lat, lon, lat0, lon0, cos_lat0)
}

/// Same as [`latlon_to_local`] but reuses a precomputed `cos(lat0)` — use this
/// when projecting many points (map load) to skip one trig call per point.
#[inline]
pub fn latlon_to_local_with_cos(
    lat: f64,
    lon: f64,
    lat0: f64,
    lon0: f64,
    cos_lat0: f64,
) -> (f64, f64) {
    let x = (lon - lon0).to_radians() * EARTH_R_M * cos_lat0;
    let y = (lat - lat0).to_radians() * EARTH_R_M;
    (x, y)
}

/// Inverse of [`latlon_to_local`]: local meters -> lat/lon.
#[inline]
pub fn local_to_latlon(x: f64, y: f64, lat0: f64, lon0: f64) -> (f64, f64) {
    let lat = lat0 + (y / EARTH_R_M).to_degrees();
    let lon = lon0 + (x / (EARTH_R_M * lat0.to_radians().cos())).to_degrees();
    (lat, lon)
}

/// World meters -> terminal cell. Same formula as Python's `world_to_screen`.
#[inline]
pub fn world_to_screen(
    wx: f64,
    wy: f64,
    player_x: f64,
    player_y: f64,
    cx: i32,
    cy: i32,
    scale_m_per_cell: f64,
) -> (i32, i32) {
    let gx = cx + ((wx - player_x) / scale_m_per_cell).round() as i32;
    let gy = cy - ((wy - player_y) / scale_m_per_cell).round() as i32;
    (gx, gy)
}

/// Squared Euclidean distance (skip the sqrt until you need meters).
#[inline]
pub fn dist2(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

/// Great-circle distance in meters (for GPS outlier gating across lat/lon).
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_R_M * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_zero() {
        let (x, y) = latlon_to_local(10.0, 76.3, 10.0, 76.3);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
    }

    #[test]
    fn one_degree_lat_is_about_111km() {
        let (_, y) = latlon_to_local(11.0, 76.3, 10.0, 76.3);
        assert!((y - 111_194.9).abs() < 1.0, "y={y}");
    }

    #[test]
    fn local_roundtrip() {
        let (x, y) = latlon_to_local(10.001, 76.301, 10.0, 76.3);
        let (lat, lon) = local_to_latlon(x, y, 10.0, 76.3);
        assert!((lat - 10.001).abs() < 1e-9);
        assert!((lon - 76.301).abs() < 1e-9);
    }

    #[test]
    fn screen_centers_player() {
        let (gx, gy) = world_to_screen(5.0, 5.0, 5.0, 5.0, 40, 12, 3.0);
        assert_eq!((gx, gy), (40, 12));
    }
}
