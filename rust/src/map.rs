//! Offline map data. Reads the same `map_data.json` produced by
//! `fetch_map.py` / `zdeck-fetch`, then pre-projects everything to local
//! meters ONCE (Python did this at load too; here segments are stored flat
//! for cache locality so the per-frame loop is just arithmetic).

use serde::Deserialize;

use crate::geo::latlon_to_local_with_cos;

#[derive(Debug, Deserialize)]
struct RawMap {
    origin_lat: f64,
    origin_lon: f64,
    #[allow(dead_code)]
    radius_m: Option<f64>,
    #[serde(default)]
    ways: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    pois: Vec<RawPoi>,
}

#[derive(Debug, Deserialize)]
struct RawPoi {
    lat: f64,
    lon: f64,
    name: String,
    category: Option<String>,
}

/// One road segment, already projected to local meters.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// Point of interest, already projected to local meters.
#[derive(Debug, Clone)]
pub struct Poi {
    pub x: f64,
    pub y: f64,
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone)]
pub struct MapData {
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub segments: Vec<Segment>,
    pub pois: Vec<Poi>,
}

impl MapData {
    pub fn empty(fallback_lat: f64, fallback_lon: f64) -> Self {
        Self {
            origin_lat: fallback_lat,
            origin_lon: fallback_lon,
            segments: Vec::new(),
            pois: Vec::new(),
        }
    }

    /// Parse + project. Never fails: corrupt input yields an empty map with
    /// the fallback origin (mirrors the Python `load_map` fallback).
    pub fn from_str(s: &str, fallback_lat: f64, fallback_lon: f64) -> Self {
        let raw: RawMap = match serde_json::from_str(s) {
            Ok(r) => r,
            Err(_) => return Self::empty(fallback_lat, fallback_lon),
        };
        let (olat, olon) = (raw.origin_lat, raw.origin_lon);
        let cos_lat0 = olat.to_radians().cos();

        let mut segments = Vec::new();
        for way in &raw.ways {
            if way.len() < 2 {
                continue;
            }
            let mut prev = {
                let (x, y) = latlon_to_local_with_cos(way[0][0], way[0][1], olat, olon, cos_lat0);
                (x, y)
            };
            for pt in &way[1..] {
                let cur = latlon_to_local_with_cos(pt[0], pt[1], olat, olon, cos_lat0);
                segments.push(Segment {
                    x1: prev.0,
                    y1: prev.1,
                    x2: cur.0,
                    y2: cur.1,
                });
                prev = cur;
            }
        }

        let mut pois = Vec::with_capacity(raw.pois.len());
        for poi in &raw.pois {
            let (x, y) = latlon_to_local_with_cos(poi.lat, poi.lon, olat, olon, cos_lat0);
            pois.push(Poi {
                x,
                y,
                name: poi.name.clone(),
                category: poi.category.clone().unwrap_or_else(|| "other".into()),
            });
        }

        Self {
            origin_lat: olat,
            origin_lon: olon,
            segments,
            pois,
        }
    }

    /// Load from a file path. Missing/corrupt file -> empty map (same as Python).
    pub fn load(path: &str, fallback_lat: f64, fallback_lon: f64) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_str(&s, fallback_lat, fallback_lon),
            Err(_) => Self::empty(fallback_lat, fallback_lon),
        }
    }

    /// Closest POI name within `max_dist` meters. Uses squared distances —
    /// no sqrt per POI (Python called `math.hypot` for every POI every frame).
    pub fn nearest_poi(&self, px: f64, py: f64, max_dist: f64) -> Option<&str> {
        let mut best: Option<&str> = None;
        let mut best_d2 = max_dist * max_dist;
        for poi in &self.pois {
            let dx = poi.x - px;
            let dy = poi.y - py;
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                best = Some(poi.name.as_str());
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"origin_lat": 10.0, "origin_lon": 76.3, "radius_m": 300,
        "ways": [[[10.0, 76.3], [10.001, 76.301]]],
        "pois": [{"lat": 10.0, "lon": 76.3, "name": "Chaya Kada", "category": "food"}]}"#;

    #[test]
    fn projects_ways_and_pois() {
        let m = MapData::from_str(SAMPLE, 0.0, 0.0);
        assert_eq!(m.segments.len(), 1);
        assert_eq!(m.pois.len(), 1);
        assert_eq!(m.pois[0].name, "Chaya Kada");
        // origin POI sits at local (0,0)
        assert!(m.pois[0].x.abs() < 1e-9 && m.pois[0].y.abs() < 1e-9);
        // segment starts at origin
        assert!(m.segments[0].x1.abs() < 1e-9);
    }

    #[test]
    fn corrupt_input_falls_back() {
        let m = MapData::from_str("not json", 10.0, 76.3);
        assert!(m.segments.is_empty() && m.pois.is_empty());
        assert_eq!((m.origin_lat, m.origin_lon), (10.0, 76.3));
    }

    #[test]
    fn nearest_poi_respects_radius() {
        let m = MapData::from_str(SAMPLE, 0.0, 0.0);
        assert_eq!(m.nearest_poi(0.0, 0.0, 30.0), Some("Chaya Kada"));
        assert_eq!(m.nearest_poi(1000.0, 1000.0, 30.0), None);
    }
}
