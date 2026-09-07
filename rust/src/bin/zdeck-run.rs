//! `zdeck-run` — main launcher (Rust port of `run.py`).
//!
//! Pi 3 64-bit kiosk flow:
//! 1. Fullscreen splash with the Z-DECK logo (fresh boot every time, never
//!    "continues where it left").
//! 2. Mode prompt with a 5 s countdown, defaulting to AUTO (GPS on
//!    `/dev/ttyAMA0`, the only GPS device on the deck).
//! 3. AUTO: wait (as long as needed) for a live GPS fix, reminding the user
//!    to step into the open -> fetch a fresh 300 m map around it -> launch
//!    `zdeck-game` in GPS-serial mode. No fallback location: without a lock
//!    the deck keeps waiting, never starts on a stale/default map.
//!    The game itself keeps polling the sensor and reloads areas as you
//!    walk out of them.
//! 4. SIM: indoor WASD testing without hardware.
//!
//! ```sh
//! zdeck-run                 # splash + 5s AUTO/SIM countdown (kiosk default)
//! zdeck-run --auto          # skip countdown, straight to AUTO
//! zdeck-run --sim           # skip prompts, sim mode
//! zdeck-run --gps /dev/ttyAMA0 --baud 9600
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use zdeck::proto::GpsFix;

/// The ONLY GPS device on the deck.
const DEFAULT_GPS_PORT: &str = "/dev/ttyAMA0";
const DEFAULT_BAUD: u32 = 9600;
/// Fresh play-area radius fetched around every live fix, meters.
const AUTO_RADIUS_M: f64 = 300.0;
/// Mode-prompt countdown, seconds.
const MODE_COUNTDOWN_S: u64 = 5;
/// Seconds between "go out into the open" reminders while AUTO waits.
/// AUTO waits indefinitely for a lock — there is no fallback location.
const DEFAULT_FIX_HINT_S: u64 = 30;

#[derive(Parser, Debug)]
#[command(
    name = "zdeck-run",
    about = "Zombie Deck launcher: splash, AUTO(GPS)/SIM pick, fetch map, run game"
)]
struct Args {
    /// Launch directly in sim (WASD) mode
    #[arg(long)]
    sim: bool,
    /// Skip the 5s countdown and go straight to AUTO (kiosk/autostart)
    #[arg(long)]
    auto: bool,
    /// Launch directly in serial GPS mode (e.g. /dev/ttyAMA0)
    #[arg(long, value_name = "PORT")]
    gps: Option<String>,
    /// Launch directly in GPIO bit-bang mode (e.g. 16)
    #[arg(long, value_name = "PIN")]
    gpio: Option<u32>,
    /// Baud rate
    #[arg(long, default_value_t = DEFAULT_BAUD)]
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
    #[arg(long, default_value_t = AUTO_RADIUS_M)]
    radius: f64,
    /// Non-interactive, auto-accept defaults (= AUTO when no other flag)
    #[arg(long)]
    yes: bool,
    /// Seconds between open-sky reminders while AUTO waits for GPS lock
    /// (AUTO waits indefinitely — no fallback location, no SIM switch)
    #[arg(long, default_value_t = DEFAULT_FIX_HINT_S)]
    fix_hint: u64,
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

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = std::io::stdout().flush();
}

/// Fullscreen splash: clear + big logo centered on the TFT, fresh every
/// boot (no resume). Falls back to plain print when the size is unknown.
fn splash() {
    clear_screen();
    const ART: &[&str] = &[
        "███████╗         ██████╗ ███████╗ ██████╗██╗  ██╗",
        "╚══███╔╝         ██╔══██╗██╔════╝██╔════╝██║ ██╔╝",
        "  ███╔╝  █████╗  ██║  ██║█████╗  ██║     █████╔╝",
        " ███╔╝   ╚════╝  ██║  ██║██╔══╝  ██║     ██╔═██╗",
        "███████╗         ██████╔╝███████╗╚██████╗██║  ██╗",
        "╚══════╝         ╚═════╝ ╚══════╝ ╚═════╝╚═╝  ╚═╝",
    ];
    let sub = format!("Z  —  D E C K  — Bit By Bit — Run for your life.");
    let sub2 = format!("PI3-64 // AUTO-BOOT // GPS {DEFAULT_GPS_PORT}");
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let total = ART.len() + 3; // art + blank + 2 sub lines
    for _ in 0..(rows as usize).saturating_sub(total) / 2 {
        println!();
    }
    for line in ART {
        let pad = (cols as usize).saturating_sub(line.chars().count()) / 2;
        println!("{GRN}{BOLD}{:pad$}{line}{RST}", "", pad = pad);
    }
    println!();
    for (line, color) in [(&sub, YEL), (&sub2, DIM)] {
        let pad = (cols as usize).saturating_sub(line.chars().count()) / 2;
        println!("{BOLD}{color}{:pad$}{line}{RST}", "", pad = pad);
    }
    let _ = std::io::stdout().flush();
    // Linger so the TFT actually reads the logo.
    std::thread::sleep(Duration::from_millis(1500));
}

// ---- binary discovery (no pip needed: static binaries) ----

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
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    })
}

fn ensure_bins(args: &Args) -> Result<Bins> {
    println!("\n{BOLD}[1/3] Checking binaries...{RST}");
    println!("  {DIM}installer: none needed (static pi3-arm64 binaries, no pip){RST}");
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
            "missing binaries: {} -- copy binary/pi3-arm64/* next to zdeck-run (looked in {} and PATH)",
            missing.join(", "),
            dir.display()
        );
    }
    Ok(bins)
}

// ---- 5-second AUTO/SIM countdown (default AUTO) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Auto,
    Sim,
}

/// Fullscreen prompt: "AUTO in Ns — A now, S for SIM". Returns the pick.
/// Falls back to AUTO when stdin is not a TTY (kiosk/test pipes).
fn countdown_pick() -> Mode {
    use crossterm::event::{self, Event, KeyCode};

    // Try raw mode; if there is no TTY (piped test), default to AUTO.
    let raw_ok = crossterm::terminal::enable_raw_mode().is_ok();
    let deadline = Instant::now() + Duration::from_secs(MODE_COUNTDOWN_S);
    let mut pick: Option<Mode> = None;

    // We print via stderr-safe println; raw mode keeps it readable enough.
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let secs = left.as_secs() + 1; // ceil-ish display
        print!(
            "\r{BOLD}[AUTO]{RST} GPS {CYA}{DEFAULT_GPS_PORT}{RST} in {YEL}{BOLD}{secs}s{RST}  — press {GRN}{BOLD}A{RST}=AUTO now  {YEL}{BOLD}S{RST}=SIM (WASD)   "
        );
        let _ = std::io::stdout().flush();
        if raw_ok && event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                match k.code {
                    KeyCode::Char('s' | 'S') => {
                        pick = Some(Mode::Sim);
                        break;
                    }
                    KeyCode::Char('a' | 'A') | KeyCode::Enter => {
                        pick = Some(Mode::Auto);
                        break;
                    }
                    KeyCode::Esc => {
                        pick = Some(Mode::Sim);
                        break;
                    }
                    _ => {}
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if raw_ok {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    println!();
    pick.unwrap_or(Mode::Auto)
}

// ---- AUTO helpers: live GPS fix + fresh map ----

/// AUTO GPS lock: spawn `zdeck-gps --source serial` and keep listening
/// until the first valid fix arrives — no timeout, no fallback location.
/// Every `hint_every` seconds without a lock, remind the user to step out
/// into an open place with a clear view of the sky. If the feed itself
/// drops (module detached), it is respawned and the wait continues, so
/// every (re)start re-locks the live position — even when an earlier run
/// never got a lock.
fn wait_for_gps_fix(gps_bin: &Path, port: &str, baud: u32, hint_every: Duration) -> (f64, f64) {
    println!("\n{BOLD}[AUTO]{RST} Scanning GPS on {CYA}{port}{RST} @ {baud} baud ...");
    println!("  {DIM}No fallback — the game starts only on a live fix.{RST}");
    if !Path::new(port).exists() {
        println!("  {YEL}⚠ {port} not present yet — still listening (module may enumerate late){RST}");
    }
    let start = Instant::now();
    let mut last_hint = Instant::now() - hint_every; // first hint shows promptly
    let spinner = ["|", "/", "-", "\\"];
    let mut i = 0u64;
    loop {
        let mut child = match Command::new(gps_bin)
            .args(["--source", "serial", "--port", port, "--baud", &baud.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                println!(
                    "  {RED}✗ can't spawn {}: {e} — retrying...{RST}",
                    gps_bin.display()
                );
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        let out = match child.stdout.take() {
            Some(o) => o,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let mut feed_alive = true;
        while feed_alive {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if let Some(f) = GpsFix::decode(&line) {
                        let _ = child.kill();
                        let _ = child.wait();
                        println!(
                            "\n  {GRN}✓ GPS fix: {:.6},{:.6}{RST} (after {:.0}s)",
                            f.lat,
                            f.lon,
                            start.elapsed().as_secs_f64()
                        );
                        return (f.lat, f.lon);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    print!(
                        "\r  {DIM}waiting for fix... {:>4}s {} {RST}",
                        start.elapsed().as_secs(),
                        spinner[(i % spinner.len() as u64) as usize]
                    );
                    let _ = std::io::stdout().flush();
                    i += 1;
                    if last_hint.elapsed() >= hint_every {
                        println!(
                            "\n  {YEL}○ No GPS lock yet — go OUT into an OPEN place with a clear view of the sky.{RST}"
                        );
                        last_hint = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    println!("\n  {RED}✗ GPS feed lost (module detached?) — respawning, still waiting...{RST}");
                    feed_alive = false;
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Always fetch a FRESH map around the live fix (never "continue where it
/// left"). Returns true on success.
fn fetch_fresh(
    fetch_bin: &Path,
    lat: f64,
    lon: f64,
    radius: f64,
    out: &Path,
) -> Result<bool> {
    println!(
        "\n{BOLD}[AUTO]{RST} Fetching fresh map: {radius:.0}m around {lat:.6},{lon:.6} ..."
    );
    let status = Command::new(fetch_bin)
        .args([
            "--lat",
            &lat.to_string(),
            "--lon",
            &lon.to_string(),
            "--radius",
            &radius.to_string(),
            "--out",
        ])
        .arg(out)
        .status()
        .with_context(|| format!("failed to run {}", fetch_bin.display()))?;
    if status.success() {
        println!("  {GRN}✓ Map saved to {}{RST}", out.display());
        Ok(true)
    } else {
        println!("  {RED}✗ fetch failed{DIM} — check internet/hotspot and retry{RST}");
        Ok(false)
    }
}

// ---- map path ----

fn default_map_path(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    // Prefer a map next to the binaries (kiosk ~/zdeck), else CWD.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("map_data.json");
            // Return it regardless of existence: AUTO always (over)writes it.
            // For SIM we check existence at call time.
            if explicit.is_none() {
                // keep going; exact choice made by caller
                let _ = &cand;
            }
        }
    }
    for cand in ["map_data.json", "app/map_data.json"] {
        if Path::new(cand).is_file() {
            return PathBuf::from(cand);
        }
    }
    // Kiosk default: next to the running binary.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("map_data.json");
        }
    }
    PathBuf::from("map_data.json")
}

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

// ---- launch ----

fn forward_test_flags(cmd: &mut Command, args: &Args) {
    if args.headless {
        cmd.arg("--headless")
            .arg("--ticks")
            .arg(args.ticks.to_string());
    }
}

fn launch_sim(bins: &Bins, args: &Args, map: &Path) {
    println!("\n{BOLD}[3/3] Launching game...{RST}");
    println!("  {CYA}Mode: SIM (WASD to move){RST}");
    let mut cmd = Command::new(&bins.game);
    cmd.arg("--map").arg(map).arg("--sim");
    // SIM sessions also get area reload (uses same fetch binary, harmless).
    cmd.arg("--fetch-bin").arg(&bins.fetch);
    forward_test_flags(&mut cmd, args);
    println!("{DIM}$ {cmd:?}{RST}\n");
    match cmd.status() {
        Ok(_) => {}
        Err(e) => println!("  {RED}✗ failed to launch: {e}{RST}"),
    }
}

fn launch_gps_serial(bins: &Bins, args: &Args, map: &Path, port: &str, baud: u32) {
    println!("\n{BOLD}[3/3] Launching game...{RST}");
    println!("  {CYA}Mode: AUTO GPS via {port} @ {baud} baud + auto area reload{RST}");
    let mut cmd = Command::new(&bins.game);
    cmd.arg("--map")
        .arg(map)
        .args(["--gps-bin"])
        .arg(&bins.gps)
        .args(["--gps-source", "serial", "--gps-port", port, "--baud", &baud.to_string()])
        // Game refetches new 300 m areas itself as you walk out of the map.
        .args(["--fetch-bin"])
        .arg(&bins.fetch)
        .args(["--fetch-radius", &args.radius.to_string()]);
    forward_test_flags(&mut cmd, args);
    println!("{DIM}$ {cmd:?}{RST}\n");
    match cmd.status() {
        Ok(_) => {}
        Err(e) => println!("  {RED}✗ failed to launch: {e}{RST}"),
    }
}

// ---- flows ----

fn run_auto(bins: &Bins, args: &Args) -> Result<()> {
    let port = args.gps.clone().unwrap_or_else(|| DEFAULT_GPS_PORT.to_string());
    let map = default_map_path(args.map.as_deref());
    println!("\n{BOLD}[2/3] AUTO mode{RST} {DIM}(fresh session — old map is discarded){RST}");
    // Every AUTO (re)start re-locks the live position and fetches a fresh
    // area around it — even when an earlier run never got a lock, this keeps
    // checking until it does. No SIM switch, no stale/default map.
    let (lat, lon) = wait_for_gps_fix(
        &bins.gps,
        &port,
        args.baud,
        Duration::from_secs(args.fix_hint.max(10)),
    );
    loop {
        match fetch_fresh(&bins.fetch, lat, lon, args.radius, &map) {
            Ok(true) => break,
            _ => {
                println!(
                    "  {YEL}○ Map fetch failed — check hotspot/internet, retrying in 10 s...{RST}"
                );
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    }
    launch_gps_serial(bins, args, &map, &port, args.baud);
    Ok(())
}

/// SIM map: reuse existing file when valid, else fetch defaults (no prompts
/// in kiosk mode — never block the TFT).
fn ensure_sim_map(bins: &Bins, args: &Args) -> Result<PathBuf> {
    let path = default_map_path(args.map.as_deref());
    if path.is_file() {
        if let Some(s) = map_summary(&path) {
            println!("  {GRN}✓ Found {} — {s}{RST}", path.display());
            return Ok(path);
        }
    }
    let (lat, lon) = (args.lat.unwrap_or(9.9649), args.lon.unwrap_or(76.2868));
    println!("  {DIM}fetching default SIM map {lat},{lon} r={}{RST}", args.radius);
    let status = Command::new(&bins.fetch)
        .args([
            "--lat",
            &lat.to_string(),
            "--lon",
            &lon.to_string(),
            "--radius",
            &args.radius.to_string(),
            "--out",
        ])
        .arg(&path)
        .status();
    match status {
        Ok(s) if s.success() => println!("  {GRN}✓ Map saved to {}{RST}", path.display()),
        _ => println!("  {YEL}○ fetch failed — game will start on blank background{RST}"),
    }
    Ok(path)
}

fn main() -> Result<()> {
    let args = Args::parse();
    splash();

    // Explicit legacy/direct flags bypass the countdown.
    if args.sim {
        let bins = ensure_bins(&args)?;
        let mp = ensure_sim_map(&bins, &args)?;
        launch_sim(&bins, &args, &mp);
        return Ok(());
    }
    if args.gpio.is_some() {
        let bins = ensure_bins(&args)?;
        let mp = ensure_sim_map(&bins, &args)?;
        let pin = args.gpio.unwrap_or(16);
        println!("\n{BOLD}[3/3] Launching game...{RST}");
        println!("  {CYA}Mode: GPS GPIO bit-bang (GPIO{pin}){RST}");
        let mut cmd = Command::new(&bins.game);
        cmd.arg("--map").arg(&mp).args(["--gps-bin"]).arg(&bins.gps).args([
            "--gps-source",
            "gpio",
            "--gpio",
            &pin.to_string(),
            "--baud",
            &args.baud.to_string(),
        ]);
        forward_test_flags(&mut cmd, &args);
        println!("{DIM}$ {cmd:?}{RST}\n");
        let _ = cmd.status();
        return Ok(());
    }
    if args.gps.is_some() && (args.auto || args.yes) {
        // --gps with --yes/--auto: same AUTO flow (wait for lock, fresh
        // fetch) on the explicit port — no stale/default map.
        let bins = ensure_bins(&args)?;
        run_auto(&bins, &args)?;
        return Ok(());
    }

    // Kiosk default: countdown, AUTO wins on timeout.
    let bins = ensure_bins(&args)?;
    let mode = if args.auto || args.yes {
        println!("\n{BOLD}Mode: AUTO{RST} {DIM}(--auto, skipping countdown){RST}");
        Mode::Auto
    } else {
        println!("\n{BOLD}How do you want to play?{RST} {DIM}(default AUTO){RST}");
        println!("  {GRN}A{RST}) AUTO — GPS {DEFAULT_GPS_PORT} @ {} baud, fresh 300 m map", args.baud);
        println!("  {YEL}S{RST}) SIM  — WASD keys, no hardware (indoor testing)");
        countdown_pick()
    };
    match mode {
        Mode::Sim => {
            let mp = ensure_sim_map(&bins, &args)?;
            launch_sim(&bins, &args, &mp);
        }
        Mode::Auto => {
            run_auto(&bins, &args)?;
        }
    }
    Ok(())
}
