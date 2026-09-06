//! `zdeck-fetch` — offline map prep tool (Rust port of `fetch_map.py`).
//!
//! Run ONCE with internet: downloads road/POI geometry around a center point
//! from OpenStreetMap (Overpass API) and saves `map_data.json` that
//! `zdeck-game` reads fully offline in the field.
//!
//! Schema is ADDITIVE (v2): `ways` + `pois{name,category}` keep their exact v1
//! shape so old readers (e.g. the Python game) load new files untouched.
//! New optional keys: `roads` (named/typed streets), `areas` (building /
//! water / green polygons), and per-POI `hours` + `addr` details.
//!
//! ```sh
//! zdeck-fetch --lat 9.9649 --lon 76.2868 --radius 300
//! ```

use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::{json, Map, Value};

const OVERPASS_URLS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.openstreetmap.ru/api/interpreter",
];

/// File schema version. Readers must tolerate missing new keys (v1 files).
const SCHEMA_VERSION: u32 = 2;

#[derive(Parser, Debug)]
#[command(
    name = "zdeck-fetch",
    about = "Fetch offline OSM map data for zdeck-game"
)]
struct Args {
    /// Center latitude of the play area
    #[arg(long)]
    lat: Option<f64>,
    /// Center longitude of the play area
    #[arg(long)]
    lon: Option<f64>,
    /// Meters around the center to fetch
    #[arg(long, default_value_t = 300.0)]
    radius: f64,
    /// Output file path
    #[arg(long, default_value = "map_data.json")]
    out: String,
}

fn build_query(lat: f64, lon: f64, radius_m: f64) -> String {
    // Roads (+ their tags: name/highway), building footprints, water and
    // green areas, plus named places from a wider tag set than v1
    // (emergency/healthcare/office/craft/historic/natural added).
    format!(
        "[out:json][timeout:25];\
         (way[\"highway\"](around:{radius_m},{lat},{lon});\
          way[\"building\"](around:{radius_m},{lat},{lon});\
          way[\"natural\"=\"water\"](around:{radius_m},{lat},{lon});\
          way[\"leisure\"~\"^(park|garden|playground|pitch)$\"](around:{radius_m},{lat},{lon});\
          way[\"landuse\"~\"^(grass|forest|meadow|recreation_ground|village_green)$\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"amenity\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"emergency\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"healthcare\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"shop\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"tourism\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"leisure\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"office\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"craft\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"historic\"](around:{radius_m},{lat},{lon});\
          node[\"name\"][\"natural\"](around:{radius_m},{lat},{lon});\
         );out geom;"
    )
}

/// Tag value -> short category. Unmapped values fall back to the raw tag
/// *key* (same quirk as `fetch_map.py`: `.get(val, key)`).
fn categorize(tags: &Map<String, Value>) -> String {
    for key in [
        "amenity",
        "emergency",
        "healthcare",
        "shop",
        "tourism",
        "leisure",
        "office",
        "craft",
        "historic",
        "natural",
    ] {
        let val = tags.get(key).and_then(|v| v.as_str()).unwrap_or("");
        if val.is_empty() {
            continue;
        }
        // Grouping keys: any office is civic infrastructure.
        if key == "office" {
            return "civic".to_string();
        }
        let cat = match val {
            "school" | "college" | "university" | "kindergarten" => "school",
            "place_of_worship" => "worship",
            "hospital" | "clinic" | "doctors" | "dentist" | "pharmacy" | "ambulance_station" => {
                "hospital"
            }
            "restaurant" | "cafe" | "fast_food" | "bar" | "pub" | "food_court" | "ice_cream" => {
                "food"
            }
            // survival supplies: groceries, markets, hardware...
            "supermarket" | "convenience" | "mall" | "department_store" | "marketplace"
            | "greengrocer" | "bakery" | "butcher" | "beverages" | "kiosk" | "newsagent"
            | "hardware" => "supply",
            "park" | "garden" | "playground" | "pitch" => "park",
            "fuel" | "charging" => "fuel",
            "bank" | "atm" | "bureau_de_change" => "bank",
            "police" | "fire_station" => "police",
            // civic / admin offices
            "government" | "townhall" | "community_centre" | "courthouse" | "embassy" => "civic",
            // culture / sights
            "museum" | "gallery" | "theatre" | "cinema" | "library" | "arts_centre"
            | "attraction" | "viewpoint" | "artwork" | "historic" => "culture",
            // shelter: zombie-apocalypse-relevant lodging
            "hotel" | "hostel" | "guest_house" | "motel" => "shelter",
            "water" | "spring" => "water",
            _ => key,
        };
        return cat.to_string();
    }
    "other".to_string()
}

/// `addr:housenumber addr:street` -> "12, Main St" (if any part exists).
fn address(tags: &Map<String, Value>) -> Option<String> {
    let num = tags
        .get("addr:housenumber")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let street = tags
        .get("addr:street")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match (num.is_empty(), street.is_empty()) {
        (true, true) => None,
        (false, false) => Some(format!("{num}, {street}")),
        (false, true) => Some(num.to_string()),
        (true, false) => Some(street.to_string()),
    }
}

fn fetch(lat: f64, lon: f64, radius_m: f64) -> Result<Value> {
    let query = build_query(lat, lon, radius_m);
    let mut last_err = String::new();
    for url in OVERPASS_URLS {
        println!("  trying {url} ...");
        let res = ureq::post(url)
            .set(
                "User-Agent",
                "zombie-cyberdeck-map-fetch/1.0 (personal hobby project)",
            )
            .set("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(30))
            .send_form(&[("data", &query)]);
        match res {
            Ok(r) => {
                let body = r.into_string().context("reading Overpass response")?;
                return serde_json::from_str(&body).context("invalid JSON from Overpass");
            }
            Err(e) => {
                println!("  -> failed: {e}");
                last_err = e.to_string();
            }
        }
    }
    bail!(
        "all Overpass mirrors failed ({last_err}). Check internet, or the \
         server is rate-limiting -- wait a minute and retry."
    )
}

fn ask(prompt: &str, default: Option<f64>) -> Result<f64> {
    loop {
        match default {
            Some(d) => print!("{prompt} [{d}]: "),
            None => print!("{prompt}: "),
        }
        io::stdout().flush()?;
        let mut s = String::new();
        io::stdin().read_line(&mut s)?;
        let s = s.trim();
        if s.is_empty() {
            if let Some(d) = default {
                return Ok(d);
            }
        } else if let Ok(v) = s.parse() {
            return Ok(v);
        }
        println!("  Enter a number.");
    }
}

fn geom_pts(el: &Value) -> Vec<[f64; 2]> {
    el.get("geometry")
        .and_then(|g| g.as_array())
        .map(|geom| {
            geom.iter()
                .filter_map(|p| Some([p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?]))
                .collect()
        })
        .unwrap_or_default()
}

fn main() -> Result<()> {
    let args = Args::parse();
    let lat = match args.lat {
        Some(v) => v,
        None => ask("Center latitude (e.g. 9.9649)", None)?,
    };
    let lon = match args.lon {
        Some(v) => v,
        None => ask("Center longitude (e.g. 76.2868)", None)?,
    };

    println!(
        "Querying OpenStreetMap for roads, buildings, water, green areas and \
         named places within {:.0}m of ({lat}, {lon})...",
        args.radius
    );
    let raw = fetch(lat, lon, args.radius)?;

    // `ways`: UNCHANGED v1 shape (highway point lists) for old readers.
    let mut ways: Vec<Vec<[f64; 2]>> = Vec::new();
    // `roads`: same geometry + name/highway tags.
    let mut roads: Vec<Value> = Vec::new();
    // `areas`: building / water / green polygons.
    let mut areas: Vec<Value> = Vec::new();
    let mut pois: Vec<Value> = Vec::new();

    for el in raw
        .get("elements")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
    {
        let typ = el.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if typ == "way" {
            let pts = geom_pts(el);
            if pts.len() < 2 {
                continue;
            }
            let tags = el.get("tags").and_then(|t| t.as_object());
            let tag = |k: &str| {
                tags.and_then(|t| t.get(k))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            };
            if !tag("highway").is_empty() {
                let name = tag("name");
                ways.push(pts.clone()); // v1 compat
                roads.push(json!({
                    "name": if name.is_empty() { Value::Null } else { json!(name) },
                    "highway": tag("highway"),
                    "pts": pts,
                }));
            } else if !tag("building").is_empty() {
                if areas.len() < 2000 {
                    areas.push(json!({"kind": "building", "pts": pts}));
                }
            } else if tag("natural") == "water" {
                let name = tag("name");
                areas.push(json!({
                    "kind": "water",
                    "name": if name.is_empty() { Value::Null } else { json!(name) },
                    "pts": pts,
                }));
            } else if matches!(tag("leisure"), "park" | "garden" | "playground" | "pitch")
                || matches!(
                    tag("landuse"),
                    "grass" | "forest" | "meadow" | "recreation_ground" | "village_green"
                )
            {
                let name = tag("name");
                areas.push(json!({
                    "kind": "green",
                    "name": if name.is_empty() { Value::Null } else { json!(name) },
                    "pts": pts,
                }));
            }
        } else if typ == "node" {
            if let Some(tags) = el.get("tags").and_then(|t| t.as_object()) {
                if let Some(name) = tags.get("name").and_then(|n| n.as_str()) {
                    let mut poi = Map::new();
                    poi.insert("lat".into(), el.get("lat").cloned().unwrap_or(Value::Null));
                    poi.insert("lon".into(), el.get("lon").cloned().unwrap_or(Value::Null));
                    poi.insert("name".into(), json!(name));
                    poi.insert("category".into(), json!(categorize(tags)));
                    if let Some(h) = tags.get("opening_hours").and_then(|v| v.as_str()) {
                        poi.insert("hours".into(), json!(h));
                    }
                    if let Some(a) = address(tags) {
                        poi.insert("addr".into(), json!(a));
                    }
                    pois.push(Value::Object(poi));
                }
            }
        }
    }

    let out = json!({
        "version": SCHEMA_VERSION,
        "origin_lat": lat,
        "origin_lon": lon,
        "radius_m": args.radius,
        "ways": ways,
        "pois": pois,
        "roads": roads,
        "areas": areas,
    });
    std::fs::write(&args.out, serde_json::to_string(&out)?)?;
    println!(
        "Saved {} roads ({} legacy ways), {} areas, {} named places to {}",
        roads.len(),
        ways.len(),
        areas.len(),
        pois.len(),
        args.out
    );
    println!("Copy this file next to zdeck-game on the Pi -- the game loads it offline.");
    Ok(())
}
