//! `zdeck-fetch` — offline map prep tool (Rust port of `fetch_map.py`).
//!
//! Run ONCE with internet: downloads road/POI geometry around a center point
//! from OpenStreetMap (Overpass API) and saves `map_data.json` that
//! `zdeck-game` reads fully offline in the field.
//!
//! ```sh
//! zdeck-fetch --lat 9.9649 --lon 76.2868 --radius 300
//! ```

use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::{json, Value};

const OVERPASS_URLS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.openstreetmap.ru/api/interpreter",
];

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
    format!(
        "[out:json][timeout:25];\
         (way[\"highway\"](around:{radius_m},{lat},{lon});\
         node[\"name\"][\"amenity\"](around:{radius_m},{lat},{lon});\
         node[\"name\"][\"shop\"](around:{radius_m},{lat},{lon});\
         node[\"name\"][\"tourism\"](around:{radius_m},{lat},{lon});\
         node[\"name\"][\"leisure\"](around:{radius_m},{lat},{lon});\
         );out geom;"
    )
}

/// Same tag -> category mapping as `fetch_map.py`, including its quirk:
/// unmapped values fall back to the raw tag *key* (`amenity`, `shop`, ...).
fn categorize(tags: &serde_json::Map<String, Value>) -> String {
    for key in ["amenity", "shop", "tourism", "leisure"] {
        let val = tags.get(key).and_then(|v| v.as_str()).unwrap_or("");
        if val.is_empty() {
            continue;
        }
        let cat = match val {
            "school" | "college" | "university" | "kindergarten" => "school",
            "place_of_worship" => "worship",
            "hospital" | "clinic" | "pharmacy" | "doctors" => "hospital",
            "restaurant" | "cafe" | "fast_food" | "bar" | "pub" => "food",
            "park" | "garden" | "playground" => "park",
            "fuel" => "fuel",
            "bank" | "atm" => "bank",
            "police" | "fire_station" => "police",
            _ => key, // = POI_TAG_TO_CATEGORY.get(val, key) in Python
        };
        return cat.to_string();
    }
    "other".to_string()
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
        "Querying OpenStreetMap for roads within {:.0}m of ({lat}, {lon})...",
        args.radius
    );
    let raw = fetch(lat, lon, args.radius)?;

    let mut ways: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut pois: Vec<Value> = Vec::new();
    for el in raw
        .get("elements")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
    {
        let typ = el.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if typ == "way" {
            if let Some(geom) = el.get("geometry").and_then(|g| g.as_array()) {
                let pts: Vec<[f64; 2]> = geom
                    .iter()
                    .filter_map(|p| Some([p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?]))
                    .collect();
                if pts.len() >= 2 {
                    ways.push(pts);
                }
            }
        } else if typ == "node" {
            if let Some(tags) = el.get("tags").and_then(|t| t.as_object()) {
                if let Some(name) = tags.get("name").and_then(|n| n.as_str()) {
                    pois.push(json!({
                        "lat": el.get("lat"),
                        "lon": el.get("lon"),
                        "name": name,
                        "category": categorize(tags),
                    }));
                }
            }
        }
    }

    let out = json!({
        "origin_lat": lat,
        "origin_lon": lon,
        "radius_m": args.radius,
        "ways": ways,
        "pois": pois,
    });
    std::fs::write(&args.out, serde_json::to_string(&out)?)?;
    println!(
        "Saved {} road segments and {} named places to {}",
        ways.len(),
        pois.len(),
        args.out
    );
    println!("Copy this file next to zdeck-game on the Pi -- the game loads it offline.");
    Ok(())
}
