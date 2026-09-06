//! `zdeck-run` — main launcher (Rust port of `run.py`).
//!
//! One binary to rule them all: check sibling binaries, ensure `map_data.json`
//! (fetching it with `zdeck-fetch` if needed), then launch `zdeck-game`.
//!
//! ```sh
//! zdeck-run                 # interactive (recommended)
//! zdeck-run --sim           # skip prompts, sim mode
//! zdeck-run --gps /dev/rfcomm0 --baud 9600
//! zdeck-run --gpio 16 --baud 9600
//! ```
//!
//! Unlike `run.py` there is nothing to `pip install`: the only "dependencies"
//! are the sibling `zdeck-*` binaries (looked up next to this executable,
//! via `--bin-dir`, or on `PATH`). `--headless/--ticks` are forwarded to
//! `zdeck-game` for testing without a terminal.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "zdeck-run",
    about = "Zombie Deck launcher: configure and run the game"
)]
struct Args {
    /// Launch directly in sim (WASD) mode
    #[arg(long)]
    sim: bool,
    /// Launch directly in serial GPS mode (e.g. /dev/rfcomm0)
    #[arg(long, value_name = "PORT")]
    gps: Option<String>,
    /// Launch directly in GPIO bit-bang mode (e.g. 16)
    #[arg(long, value_name = "PIN")]
    gpio: Option<u32>,
    /// Baud rate
    #[arg(long, default_value_t = 9600)]
    baud: u32,
    /// Path to map_data.json
    #[arg(long, value_name = "PATH")]
    map: Option<String>,
    /// Fetch map lat (non-interactive)
    #[arg(long)]
    lat: Option<f64>,
    /// Fetch map lon (non-interactive)
    #[arg(long)]
    lon: Option<f64>,
    /// Fetch map radius, meters
    #[arg(long, default_value_t = 300.0)]
    radius: f64,
    /// Non-interactive, auto-accept defaults
    #[arg(long)]
    yes: bool,
    /// Directory containing the zdeck-* binaries (default: this exe's dir, then PATH)
    #[arg(long, value_name = "DIR")]
    bin_dir: Option<String>,
    /// Test hook: run the game without a TUI
    #[arg(long)]
    headless: bool,
    /// Test hook: ticks for headless game run
    #[arg(long, default_value_t = 40)]
    ticks: u64,
}

// ---- colors (same palette as run.py) ----
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GRN: &str = "\x1b[92m";
const RED: &str = "\x1b[91m";
const CYA: &str = "\x1b[96m";
const YEL: &str = "\x1b[93m";
const RST: &str = "\x1b[0m";

fn banner() {
    println!(
        "{GRN}{BOLD}
  ███████╗         ██████╗ ███████╗ ██████╗██╗  ██╗
  ╚══███╔╝         ██╔══██╗██╔════╝██╔════╝██║ ██╔╝
    ███╔╝  █████╗  ██║  ██║█████╗  ██║     █████╔╝
   ███╔╝   ╚════╝  ██║  ██║██╔══╝  ██║     ██╔═██╗
  ███████╗         ██████╔╝███████╗╚██████╗██║  ██╗
  ╚══════╝         ╚═════╝ ╚══════╝ ╚═════╝╚═╝  ╚═╝
{RST}{YEL}{BOLD}                    Z  —  D E C K{RST}{DIM}  — Bit By Bit — Run for your life.{RST}"
    );
    // Linger so the TFT / terminal actually reads the logo.
    std::thread::sleep(std::time::Duration::from_millis(2200));
}

// ---- binary discovery (replaces run.py's ensure_deps: no pip needed) ----

struct Bins {
    game: PathBuf,
    gps: PathBuf,
    fetch: PathBuf,
}

fn find_bin(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    // Fall back to PATH lookup.
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    })
}

fn ensure_bins(args: &Args) -> Result<Bins> {
    println!("\n{BOLD}[1/3] Checking binaries...{RST}");
    println!("  {DIM}installer: none needed (static binaries, no pip){RST}");
    let dir = match &args.bin_dir {
        Some(d) => PathBuf::from(d),
        None => std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let mut bins = Bins {
        game: PathBuf::new(),
        gps: PathBuf::new(),
        fetch: PathBuf::new(),
    };
    let mut missing = Vec::new();
    for (slot, name) in [
        (&mut bins.game, "zdeck-game"),
        (&mut bins.gps, "zdeck-gps"),
        (&mut bins.fetch, "zdeck-fetch"),
    ] {
        match find_bin(&dir, name) {
            Some(p) => {
                println!("  {GRN}✓ {name} ({}){RST}", p.display());
                *slot = p;
            }
            None => missing.push(name),
        }
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "missing binaries: {} -- build with `cargo build --release` (looked in {} and PATH)",
            missing.join(", "),
            dir.display()
        );
    }
    Ok(bins)
}

// ---- prompts (same UX as run.py, EOF-safe) ----

fn read_line() -> Option<String> {
    let mut s = String::new();
    io::stdout().flush().ok()?;
    match io::stdin().read_line(&mut s) {
        Ok(0) => None, // EOF
        Ok(_) => Some(s.trim().to_string()),
        Err(_) => None,
    }
}

fn ask_choice(prompt: &str, options: &[(&str, &str)], default: Option<char>) -> char {
    let opt_str: Vec<String> = options
        .iter()
        .map(|(k, _)| format!("{YEL}{k}{RST}"))
        .collect();
    let hint = default.map(|d| format!(" [{d}]")).unwrap_or_default();
    loop {
        println!(
            "\n{BOLD}{prompt}{RST} {DIM}({}{hint}){RST}",
            opt_str.join("/")
        );
        for (k, label) in options {
            println!("  {YEL}{k}{RST}) {label}");
        }
        print!("> ");
        match read_line() {
            None => {
                println!(
                    "\n{DIM}No input — using default {}{RST}",
                    default.unwrap_or('?')
                );
                if let Some(d) = default {
                    return d.to_ascii_lowercase();
                }
                std::process::exit(0);
            }
            Some(ans) => {
                let ans = ans.to_lowercase();
                if ans.is_empty() {
                    if let Some(d) = default {
                        return d.to_ascii_lowercase();
                    }
                }
                for (k, _) in options {
                    if ans == k.to_lowercase() {
                        return k.to_lowercase().chars().next().unwrap();
                    }
                }
                println!("{RED}  Invalid choice.{RST}");
            }
        }
    }
}

fn ask_float(prompt: &str, default: Option<f64>) -> f64 {
    loop {
        let hint = default.map(|d| format!(" [{d}]")).unwrap_or_default();
        print!("{prompt}{hint}: ");
        match read_line() {
            None => {
                if let Some(d) = default {
                    println!("\n{DIM}No input — using {d}{RST}");
                    return d;
                }
                std::process::exit(0);
            }
            Some(ans) => {
                if ans.is_empty() {
                    if let Some(d) = default {
                        return d;
                    }
                } else if let Ok(v) = ans.parse() {
                    return v;
                }
                println!("{RED}  Enter a number.{RST}");
            }
        }
    }
}

fn ask_str(prompt: &str, default: &str) -> String {
    let hint = if default.is_empty() {
        String::new()
    } else {
        format!(" [{default}]")
    };
    print!("{prompt}{hint}: ");
    match read_line() {
        None => {
            println!("\n{DIM}No input — using {default}{RST}");
            default.to_string()
        }
        Some(ans) => {
            if ans.is_empty() {
                default.to_string()
            } else {
                ans
            }
        }
    }
}

// ---- map data (mirrors run.py ensure_map) ----

fn map_summary(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let count = |k: &str| {
        v.get(k)
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    };
    Some(format!(
        "origin {},{} radius {}m, {} ways, {} POIs, {} roads, {} areas",
        v.get("origin_lat")?,
        v.get("origin_lon")?,
        v.get("radius_m")?,
        count("ways"),
        count("pois"),
        count("roads"),
        count("areas"),
    ))
}

fn default_map_path(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    // run.py keeps map_data.json next to the app dir; accept either layout.
    for cand in ["map_data.json", "app/map_data.json"] {
        if Path::new(cand).is_file() {
            return PathBuf::from(cand);
        }
    }
    PathBuf::from("map_data.json")
}

fn ensure_map(bins: &Bins, args: &Args, interactive: bool) -> Result<PathBuf> {
    println!("\n{BOLD}[2/3] Map data{RST}");
    let path = default_map_path(args.map.as_deref());

    if path.is_file() {
        match map_summary(&path) {
            Some(s) => println!("  {GRN}✓ Found {} — {s}{RST}", path.display()),
            None => println!(
                "  {YEL}○ {} exists but is corrupt, will re-fetch{RST}",
                path.display()
            ),
        }
        if map_summary(&path).is_some() {
            if !interactive || args.yes {
                return Ok(path);
            }
            if ask_choice(
                "Map found. What to do?",
                &[("K", "Keep existing"), ("F", "Fetch new map")],
                Some('K'),
            ) == 'k'
            {
                return Ok(path);
            }
        }
    }

    // Need to fetch (or user asked for a fresh one).
    let (lat, lon, radius) = if interactive && !args.yes {
        println!("\n  Fetch map from OpenStreetMap (needs internet once, then offline).");
        println!("  {DIM}Find your lat/lon on https://www.openstreetmap.org (right-click → Show address){RST}");
        (
            ask_float(
                "  Center latitude  (e.g. 9.9649)",
                args.lat.or(Some(9.9649)),
            ),
            ask_float(
                "  Center longitude (e.g. 76.2868)",
                args.lon.or(Some(76.2868)),
            ),
            ask_float("  Radius meters", Some(args.radius)),
        )
    } else if let (Some(lat), Some(lon)) = (args.lat, args.lon) {
        (lat, lon, args.radius)
    } else {
        println!("  {YEL}○ No map file. Run with --lat/--lon or interactively to fetch.{RST}");
        return Ok(path);
    };

    let status = Command::new(&bins.fetch)
        .args([
            "--lat",
            &lat.to_string(),
            "--lon",
            &lon.to_string(),
            "--radius",
            &radius.to_string(),
            "--out",
        ])
        .arg(&path)
        .status()
        .with_context(|| format!("failed to run {}", bins.fetch.display()))?;
    if status.success() {
        println!("  {GRN}✓ Map saved to {}{RST}", path.display());
    } else {
        println!("  {RED}✗ fetch failed{DIM} — try again with hotspot, or copy an existing map_data.json next to zdeck-game{RST}");
    }
    Ok(path)
}

// ---- launch ----

fn forward_test_flags(cmd: &mut Command, args: &Args) {
    if args.headless {
        cmd.arg("--headless")
            .arg("--ticks")
            .arg(args.ticks.to_string());
    }
}

fn launch_game(
    bins: &Bins,
    args: &Args,
    mode: &str,
    gps_port: Option<&str>,
    gpio_pin: u32,
    baud: u32,
    map: &Path,
) {
    println!("\n{BOLD}[3/3] Launching game...{RST}");
    let mut cmd = Command::new(&bins.game);
    cmd.arg("--map").arg(map);
    forward_test_flags(&mut cmd, args);
    match mode {
        "sim" => {
            cmd.arg("--sim");
            println!("  {CYA}Mode: SIM (WASD to move){RST}");
            println!("  {DIM}Controls: W/A/S/D move, Q quit. Zombies spawn 40-90m away.{RST}");
        }
        "serial" => {
            let port = gps_port.unwrap_or("/dev/rfcomm0");
            cmd.args(["--gps-bin"]).arg(&bins.gps).args([
                "--gps-source",
                "serial",
                "--gps-port",
                port,
                "--baud",
                &baud.to_string(),
            ]);
            println!("  {CYA}Mode: GPS via {port} @ {baud} baud{RST}");
        }
        _ => {
            cmd.args(["--gps-bin"]).arg(&bins.gps).args([
                "--gps-source",
                "gpio",
                "--gpio",
                &gpio_pin.to_string(),
                "--baud",
                &baud.to_string(),
            ]);
            println!("  {CYA}Mode: GPS GPIO bit-bang (GPIO{gpio_pin}, {baud} baud){RST}");
            println!("  {DIM}Needs pigpiod: sudo systemctl start pigpiod{RST}");
        }
    }
    println!("{DIM}$ {cmd:?}{RST}\n");
    match cmd.status() {
        Ok(_) => {}
        Err(e) => println!("  {RED}✗ failed to launch: {e}{RST}"),
    }
}

// ---- main flows (mirror run.py) ----

fn main() -> Result<()> {
    let args = Args::parse();
    banner();

    // Quick non-interactive paths.
    if args.sim || args.gps.is_some() || args.gpio.is_some() || args.yes {
        let bins = ensure_bins(&args)?;
        let mp = ensure_map(&bins, &args, false)?;
        if args.sim {
            launch_game(&bins, &args, "sim", None, 16, args.baud, &mp);
        } else if let Some(port) = &args.gps {
            launch_game(&bins, &args, "serial", Some(port), 16, args.baud, &mp);
        } else {
            launch_game(
                &bins,
                &args,
                "gpio",
                None,
                args.gpio.unwrap_or(16),
                args.baud,
                &mp,
            );
        }
        return Ok(());
    }

    // Interactive flow.
    let bins = ensure_bins(&args)?;
    let mp = ensure_map(&bins, &args, true)?;

    println!("\n{BOLD}How do you want to play?{RST}");
    match ask_choice(
        "Select mode",
        &[
            ("1", "Simulation  — WASD keys, no hardware (indoor testing)"),
            (
                "2",
                "GPS Serial  — NEO-6M via /dev/rfcomm0 / /dev/ttyUSB0 / /dev/ttyAMA0",
            ),
            (
                "3",
                "GPS GPIO    — bit-banged on GPIO pin (TFT occupies UART, pigpio)",
            ),
        ],
        Some('1'),
    ) {
        '1' => launch_game(&bins, &args, "sim", None, 16, args.baud, &mp),
        '2' => {
            println!("\n{BOLD}GPS Serial setup{RST}");
            let port = ask_str("  Serial port", "/dev/rfcomm0");
            let baud = ask_float("  Baud rate", Some(args.baud as f64)) as u32;
            if !Path::new(&port).exists() {
                println!("  {YEL}⚠ {port} not found — will still try (Bluetooth may create it on connect){RST}");
            }
            launch_game(&bins, &args, "serial", Some(&port), 16, baud, &mp);
        }
        _ => {
            println!("\n{BOLD}GPS GPIO bit-bang setup{RST} {DIM}(when TFT uses pins 8/10){RST}");
            let pin = ask_float("  GPIO pin (BCM numbering)", Some(16.0)) as u32;
            let baud = ask_float("  Baud rate", Some(args.baud as f64)) as u32;
            launch_game(&bins, &args, "gpio", None, pin, baud, &mp);
        }
    }
    Ok(())
}
