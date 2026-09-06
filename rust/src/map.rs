//! Offline map data. Reads the same `map_data.json` produced by
//! `fetch_map.py` / `zdeck-fetch`, then pre-projects everything to local
//! meters ONCE (Python did this at load too; here segments are stored flat
//! for cache locality so the per-frame loop is just arithmetic).
//!
//! Schema v2 is additive: `ways` + `pois{name,category}` keep their v1 shape
//! (old readers load new files fine); `roads`, `areas` and per-POI
//! `hours`/`addr` are optional and default to empty.

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
    /// v2: named/typed streets. Absent in v1 files -> empty.
    #[serde(default)]
    roads: Vec<RawRoad>,
    /// v2: building / water / green polygons. Absent in v1 files -> empty.
    #[serde(default)]
    areas: Vec<RawArea>,
}

#[derive(Debug, Deserialize)]
struct RawPoi {
    lat: f64,
    lon: f64,
    name: String,
    category: Option<String>,
    hours: Option<String>,
    addr: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRoad {
    name: Option<String>,
    highway: Option<String>,
    #[serde(default)]
    pts: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct RawArea {
    kind: Option<String>,
    name: Option<String>,
    #[serde(default)]
    pts: Vec<[f64; 2]>,
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
    /// `opening_hours` tag, if OSM had one.
    pub hours: Option<String>,
    /// Assembled `addr:housenumber, addr:street`, if OSM had them.
    pub addr: Option<String>,
}

/// A street with its OSM name + highway type, projected to local meters.
#[derive(Debug, Clone)]
pub struct Road {
    pub name: Option<String>,
    /// OSM highway class (`residential`, `footway`, `primary`, ...).
    pub highway: String,
    /// Arterials (motorway..tertiary) render brighter than lanes/paths.
    pub major: bool,
    pub segs: Vec<Segment>,
}

/// A filled map feature, projected to local meters.
#[derive(Debug, Clone)]
pub struct Area {
    pub kind: AreaKind,
    pub name: Option<String>,
    /// Closed ring (first == last is fine, not required).
    pub poly: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaKind {
    Building,
    Water,
    Green,
}

impl AreaKind {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "building" => Some(Self::Building),
            "water" => Some(Self::Water),
            "green" => Some(Self::Green),
            _ => None,
        }
    }
}

/// OSM highway classes drawn as bright arterials.
fn is_major(highway: &str) -> bool {
    matches!(
        highway,
        "motorway"
            | "trunk"
            | "primary"
            | "secondary"
            | "tertiary"
            | "motorway_link"
            | "trunk_link"
            | "primary_link"
            | "secondary_link"
            | "tertiary_link"
    )
}

#[derive(Debug, Clone)]
pub struct MapData {
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub segments: Vec<Segment>,
    pub pois: Vec<Poi>,
    pub roads: Vec<Road>,
    pub areas: Vec<Area>,
}

impl MapData {
    pub fn empty(fallback_lat: f64, fallback_lon: f64) -> Self {
        Self {
            origin_lat: fallback_lat,
            origin_lon: fallback_lon,
            segments: Vec::new(),
            pois: Vec::new(),
            roads: Vec::new(),
            areas: Vec::new(),
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
        let proj = |lat: f64, lon: f64| latlon_to_local_with_cos(lat, lon, olat, olon, cos_lat0);

        let mut segments = Vec::new();
        for way in &raw.ways {
            push_way(&mut segments, way, &proj);
        }

        let mut roads = Vec::with_capacity(raw.roads.len());
        for road in &raw.roads {
            let mut segs = Vec::new();
            push_way(&mut segs, &road.pts, &proj);
            if segs.is_empty() {
                continue;
            }
            let highway = road.highway.clone().unwrap_or_else(|| "road".into());
            roads.push(Road {
                name: road.name.clone().filter(|n| !n.is_empty()),
                major: is_major(&highway),
                highway,
                segs,
            });
        }

        let mut areas = Vec::with_capacity(raw.areas.len());
        for area in &raw.areas {
            let Some(kind) = area.kind.as_deref().and_then(AreaKind::from_str) else {
                continue;
            };
            if area.pts.len() < 3 {
                continue;
            }
            areas.push(Area {
                kind,
                name: area.name.clone().filter(|n| !n.is_empty()),
                poly: area.pts.iter().map(|&[la, lo]| proj(la, lo)).collect(),
            });
        }

        let mut pois = Vec::with_capacity(raw.pois.len());
        for poi in &raw.pois {
            let (x, y) = proj(poi.lat, poi.lon);
            pois.push(Poi {
                x,
                y,
                name: poi.name.clone(),
                category: poi.category.clone().unwrap_or_else(|| "other".into()),
                hours: poi.hours.clone().filter(|h| !h.is_empty()),
                addr: poi.addr.clone().filter(|a| !a.is_empty()),
            });
        }

        Self {
            origin_lat: olat,
            origin_lon: olon,
            segments,
            pois,
            roads,
            areas,
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
    pub fn nearest_poi(&self, px: f64, py: f64, max_dist: f64) -> Option<&Poi> {
        let mut best: Option<&Poi> = None;
        let mut best_d2 = max_dist * max_dist;
        for poi in &self.pois {
            let dx = poi.x - px;
            let dy = poi.y - py;
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                best = Some(poi);
            }
        }
        best
    }

    /// Name of the closest NAMED road within `max_dist` meters (for the
    /// "on: Main St" status callout). Point-to-segment, squared, no sqrt.
    pub fn nearest_road(&self, px: f64, py: f64, max_dist: f64) -> Option<&str> {
        let mut best: Option<&str> = None;
        let mut best_d2 = max_dist * max_dist;
        for road in &self.roads {
            let Some(name) = road.name.as_deref() else {
                continue;
            };
            for s in &road.segs {
                let d2 = seg_dist2(px, py, s);
                if d2 < best_d2 {
                    best_d2 = d2;
                    best = Some(name);
                }
            }
        }
        best
    }
}

/// Project a lat/lon way into flat segments (skips degenerate ways).
fn push_way(segments: &mut Vec<Segment>, way: &[[f64; 2]], proj: &impl Fn(f64, f64) -> (f64, f64)) {
    if way.len() < 2 {
        return;
    }
    let mut prev = proj(way[0][0], way[0][1]);
    for pt in &way[1..] {
        let cur = proj(pt[0], pt[1]);
        if (cur.0 - prev.0).abs() + (cur.1 - prev.1).abs() > 1e-9 {
            segments.push(Segment {
                x1: prev.0,
                y1: prev.1,
                x2: cur.0,
                y2: cur.1,
            });
        }
        prev = cur;
    }
}

/// Squared distance from point to segment (no sqrt in the hot loop).
fn seg_dist2(px: f64, py: f64, s: &Segment) -> f64 {
    let dx = s.x2 - s.x1;
    let dy = s.y2 - s.y1;
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        ((px - s.x1) * dx + (py - s.y1) * dy) / len2
    } else {
        0.0
    };
    let t = t.clamp(0.0, 1.0);
    let cx = s.x1 + t * dx - px;
    let cy = s.y1 + t * dy - py;
    cx * cx + cy * cy
}

/// Viewport for projecting local-meters geometry to screen cells.
pub struct Viewport {
    pub player_x: f64,
    pub player_y: f64,
    pub cx: i32,
    pub cy: i32,
    pub scale_m_per_cell: f64,
    pub w: i32,
    pub h: i32,
}

/// Rasterize a local-meters polygon into screen cells.
///
/// `plot(gx, gy)` is called for each covered cell; `budget` caps total cells
/// per frame (shared across polygons) so a huge park can't stall the Pi.
/// Even-odd scanline fill + edge outline. All integer screen math.
pub fn fill_poly(
    poly: &[(f64, f64)],
    view: &Viewport,
    budget: &mut usize,
    mut plot: impl FnMut(i32, i32),
) {
    use crate::geo::world_to_screen;
    if poly.len() < 3 || *budget == 0 {
        return;
    }
    // Project once; track screen-space bbox for viewport culling.
    let mut sx = Vec::with_capacity(poly.len());
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(wx, wy) in poly {
        let (gx, gy) = world_to_screen(
            wx,
            wy,
            view.player_x,
            view.player_y,
            view.cx,
            view.cy,
            view.scale_m_per_cell,
        );
        x0 = x0.min(gx);
        y0 = y0.min(gy);
        x1 = x1.max(gx);
        y1 = y1.max(gy);
        sx.push((gx, gy));
    }
    if x1 < 0 || y1 < 0 || x0 >= view.w || y0 >= view.h {
        return; // fully off-screen
    }
    // Absurdly large polygons (bad OSM ring) -> outline only, no fill.
    let fill = (x1 - x0) <= 300 && (y1 - y0) <= 150;
    let xa = x0.max(0);
    let xb = x1.min(view.w - 1);
    let ya = y0.max(0);
    let yb = y1.min(view.h - 1);
    let n = sx.len();
    // Edge outline (cheap, always drawn).
    for i in 0..n {
        let (ax, ay) = sx[i];
        let (bx, by) = sx[(i + 1) % n];
        let steps = (bx - ax).abs().max((by - ay).abs()).clamp(1, 200);
        for j in 0..=steps {
            let t = j as f64 / steps as f64;
            plot(
                (ax as f64 + (bx - ax) as f64 * t).round() as i32,
                (ay as f64 + (by - ay) as f64 * t).round() as i32,
            );
        }
    }
    if !fill {
        return;
    }
    // Scanline fill.
    for gy in ya..=yb {
        if *budget == 0 {
            return;
        }
        let mut xs: Vec<f64> = Vec::with_capacity(8);
        for i in 0..n {
            let (ax, ay) = sx[i];
            let (bx, by) = sx[(i + 1) % n];
            if (ay <= gy && gy < by) || (by <= gy && gy < ay) {
                let t = (gy - ay) as f64 / (by - ay) as f64;
                xs.push(ax as f64 + (bx - ax) as f64 * t);
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks_exact(2) {
            let (mut a, mut b) = (pair[0].ceil() as i32, pair[1].floor() as i32);
            a = a.max(xa);
            b = b.min(xb);
            for gx in a..=b {
                if *budget == 0 {
                    return;
                }
                *budget -= 1;
                plot(gx, gy);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"origin_lat": 10.0, "origin_lon": 76.3, "radius_m": 300,
        "ways": [[[10.0, 76.3], [10.001, 76.301]]],
        "pois": [{"lat": 10.0, "lon": 76.3, "name": "Chaya Kada", "category": "food"}]}"#;

    const SAMPLE_V2: &str = r#"{"version": 2, "origin_lat": 10.0, "origin_lon": 76.3, "radius_m": 300,
        "ways": [[[10.0, 76.3], [10.001, 76.301]]],
        "pois": [{"lat": 10.0, "lon": 76.3, "name": "City Hospital", "category": "hospital",
                  "hours": "24/7", "addr": "12, Main St"}],
        "roads": [{"name": "Main St", "highway": "primary", "pts": [[10.0, 76.3], [10.0, 76.301]]},
                  {"name": null, "highway": "footway", "pts": [[10.0, 76.3], [10.001, 76.3]]}],
        "areas": [{"kind": "building", "pts": [[10.0, 76.3], [10.0, 76.3001], [10.0001, 76.3001]]},
                  {"kind": "pond", "pts": [[10.0, 76.3], [10.0, 76.3001], [10.0001, 76.3001]]}]}"#;

    #[test]
    fn projects_ways_and_pois() {
        let m = MapData::from_str(SAMPLE, 0.0, 0.0);
        assert_eq!(m.segments.len(), 1);
        assert_eq!(m.pois.len(), 1);
        assert_eq!(m.pois[0].name, "Chaya Kada");
        assert!(m.roads.is_empty() && m.areas.is_empty()); // v1 has no extras
                                                           // origin POI sits at local (0,0)
        assert!(m.pois[0].x.abs() < 1e-9 && m.pois[0].y.abs() < 1e-9);
        // segment starts at origin
        assert!(m.segments[0].x1.abs() < 1e-9);
    }

    #[test]
    fn v2_roads_areas_extras() {
        let m = MapData::from_str(SAMPLE_V2, 0.0, 0.0);
        assert_eq!(m.roads.len(), 2);
        assert_eq!(m.roads[0].name.as_deref(), Some("Main St"));
        assert!(m.roads[0].major); // primary
        assert!(!m.roads[1].major); // footway
        assert_eq!(m.areas.len(), 1); // unknown kind "pond" skipped
        assert_eq!(m.areas[0].kind, AreaKind::Building);
        assert_eq!(m.pois[0].hours.as_deref(), Some("24/7"));
        assert_eq!(m.pois[0].addr.as_deref(), Some("12, Main St"));
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
        assert_eq!(
            m.nearest_poi(0.0, 0.0, 30.0).map(|p| p.name.as_str()),
            Some("Chaya Kada")
        );
        assert!(m.nearest_poi(1000.0, 1000.0, 30.0).is_none());
    }

    #[test]
    fn nearest_road_needs_name_and_range() {
        let m = MapData::from_str(SAMPLE_V2, 0.0, 0.0);
        // Main St runs east along y=0: standing on it names it.
        assert_eq!(m.nearest_road(50.0, 0.0, 12.0), Some("Main St"));
        // 500m away: nothing (unnamed footway never qualifies).
        assert_eq!(m.nearest_road(500.0, 500.0, 12.0), None);
    }

    #[test]
    fn fill_poly_covers_triangle() {
        // 30m right triangle at origin, 3m cells, 40x20 viewport.
        let poly = vec![(0.0, 0.0), (30.0, 0.0), (0.0, 30.0)];
        let view = Viewport {
            player_x: 0.0,
            player_y: 0.0,
            cx: 20,
            cy: 10,
            scale_m_per_cell: 3.0,
            w: 40,
            h: 20,
        };
        let mut cells = Vec::new();
        let mut budget = 10_000;
        fill_poly(&poly, &view, &mut budget, |x, y| cells.push((x, y)));
        assert!(!cells.is_empty());
        // center cell (origin) must be covered...
        assert!(cells.contains(&(20, 10)));
        // ...and nothing may escape the viewport.
        assert!(cells
            .iter()
            .all(|&(x, y)| (0..40).contains(&x) && (0..20).contains(&y)));
        assert!(budget < 10_000);
    }

    #[test]
    fn fill_poly_respects_budget() {
        // 60m square -> 20x20 cells; outline ~84 plots, fill must stop at 50.
        let poly = vec![
            (120.0, 120.0),
            (180.0, 120.0),
            (180.0, 180.0),
            (120.0, 180.0),
        ];
        let view = Viewport {
            player_x: 150.0,
            player_y: 150.0,
            cx: 50,
            cy: 25,
            scale_m_per_cell: 3.0,
            w: 100,
            h: 50,
        };
        let mut n = 0;
        let mut budget = 50;
        fill_poly(&poly, &view, &mut budget, |_, _| n += 1);
        assert!(n <= 50 + 100, "n={n}");
        assert_eq!(budget, 0);
    }
}
