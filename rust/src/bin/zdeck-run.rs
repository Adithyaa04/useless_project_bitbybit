//! `zdeck-run` — main launcher (Rust port of `run.py`).
//!
//! Pi 3 64-bit boot flow:
//! 1. Fullscreen splash with the Z-DECK logo (fresh boot every time, never
//!    "continues where it left").
//! 2. Launcher menu — ↑/↓ + Enter: `Start` (saved default mode), `Settings`
//!    (edit + persist defaults to `zdeck.conf` next to the binary), or
//!    `Quit to terminal` (drops to a normal shell; exiting it returns to
//!    the deck). Works with the CardKB arrows, USB keyboards, and 1/2/3.
//! 3. AUTO: wait (as long as needed) for a live GPS fix, reminding the user
//!    to step into the open -> fetch a fresh map around it -> launch
//!    `zdeck-game` in GPS-serial mode. No fallback location: without a lock
//!    the deck keeps waiting, never starts on a stale/default map.
//!    The game itself keeps polling the sensor and reloads areas as you
//!    walk out of them.
//! 4. SIM: indoor WASD testing without hardware.
//!
//! Non-interactive (kiosk/autostart/SSH) flows bypass the menu:
//! ```sh
//! zdeck-run                 # splash + launcher menu (default)
//! zdeck-run --auto          # skip menu, straight to AUTO (kiosk)
//! zdeck-run --sim           # skip prompts, sim mode
//! zdeck-run --gps /dev/ttyAMA0 --baud 9600
//! ```
//!
//! Quit-to-terminal note: under DietPi's "Custom script (foreground)"
//! autostart the menu IS the session, so quitting the shell (or the
//! launcher) returns to a fresh login/launcher — by design. For a
//! persistent shell that survives, Ctrl+C the foreground script.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use zdeck::proto::GpsFix;

/// The ONLY GPS device on the deck.
const DEFAULT_GPS_PORT: &str = "/dev/ttyAMA0";
const DEFAULT_BAUD: u32 = 9600;
/// Fresh play-area radius fetched around every live fix, meters.
const AUTO_RADIUS_M: f64 = 300.0;
/// Seconds between "go out into the open" reminders while AUTO waits.
/// AUTO waits indefinitely for a lock — there is no fallback location.
const DEFAULT_FIX_HINT_S: u64 = 30;
/// Settings file (next to the binary, so ~/zdeck/zdeck.conf on the deck).
const CONFIG_FILE: &str = "zdeck.conf";
/// Modes the launcher can start.
const MODES: [&str; 4] = ["auto", "sim", "serial", "gpio"];

#[derive(Parser, Debug, Clone)]
#[command(
    name = "zdeck-run",
    about = "Zombie Deck launcher: splash, menu, AUTO(GPS)/SIM pick, fetch map, run game"
)]
struct Args {
    /// Launch directly in sim (WASD) mode
    #[arg(long)]
    sim: bool,
    /// Skip the launcher menu and go straight to AUTO (kiosk/autostart)
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
    /// Settings file (default: zdeck.conf next to this exe)
    #[arg(long, value_name = "PATH")]
    config: Option<String>,
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

// ---- persistent settings (zdeck.conf next to the binary) ----

/// Everything the Settings screen can edit. CLI flags always win over
/// these for the current session; the file only feeds the launcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeckConfig {
    /// Which mode `Start` launches: auto | sim | serial | gpio
    default_mode: String,
    /// SIM session centre + map radius (indoor testing, no GPS)
    sim_lat: f64,
    sim_lon: f64,
    sim_radius: f64,
    /// AUTO fetch radius around the live fix (the "map loading radius")
    fetch_radius: f64,
    /// GPS serial port for `serial` mode / AUTO override
    gps_port: String,
    /// GPIO pin (BCM) for `gpio` bit-bang mode
    gpio_pin: u32,
    /// GPS baud rate for every hardware mode
    baud: u32,
    /// Seconds between open-sky reminders while AUTO waits for a lock
    fix_hint: u64,
}

impl Default for DeckConfig {
    fn default() -> Self {
        Self {
            default_mode: "auto".to_string(),
            sim_lat: 9.9649,
            sim_lon: 76.2868,
            sim_radius: AUTO_RADIUS_M,
            fetch_radius: AUTO_RADIUS_M,
            gps_port: DEFAULT_GPS_PORT.to_string(),
            gpio_pin: 16,
            baud: DEFAULT_BAUD,
            fix_hint: DEFAULT_FIX_HINT_S,
        }
    }
}

impl DeckConfig {
    fn sanitise(&mut self) {
        if !MODES.contains(&self.default_mode.as_str()) {
            self.default_mode = "auto".to_string();
        }
        self.sim_lat = self.sim_lat.clamp(-90.0, 90.0);
        self.sim_lon = self.sim_lon.clamp(-180.0, 180.0);
        self.sim_radius = self.sim_radius.clamp(50.0, 2000.0);
        self.fetch_radius = self.fetch_radius.clamp(50.0, 2000.0);
        if self.gpio_pin > 27 {
            self.gpio_pin = 16;
        }
        if self.baud == 0 {
            self.baud = DEFAULT_BAUD;
        }
        if self.fix_hint < 10 {
            self.fix_hint = 10;
        }
        if self.gps_port.trim().is_empty() {
            self.gps_port = DEFAULT_GPS_PORT.to_string();
        }
    }

    /// Next selectable default mode (Enter on the mode row cycles these).
    fn cycle_mode(&mut self) {
        let i = MODES
            .iter()
            .position(|m| *m == self.default_mode)
            .unwrap_or(0);
        self.default_mode = MODES[(i + 1) % MODES.len()].to_string();
    }
}

/// Config path: --config flag, else zdeck.conf next to this exe.
fn config_path(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join(CONFIG_FILE);
        }
    }
    PathBuf::from(CONFIG_FILE)
}

fn load_config(path: &Path) -> DeckConfig {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<DeckConfig>(&text) {
            Ok(mut cfg) => {
                cfg.sanitise();
                cfg
            }
            Err(e) => {
                println!("  {YEL}○ {CONFIG_FILE} invalid ({e}) — using defaults{RST}");
                DeckConfig::default()
            }
        },
        Err(_) => DeckConfig::default(), // first run: no file yet
    }
}

fn save_config(path: &Path, cfg: &DeckConfig) -> Result<()> {
    let text = serde_json::to_string_pretty(cfg)?;
    std::fs::write(path, text)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
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

// ---- launcher menu (↑/↓ + Enter) + settings editor ----

/// What the launcher menu returns.
enum Launch {
    Start,
    Settings,
    Quit,
}

fn raw_on() {
    let _ = crossterm::terminal::enable_raw_mode();
}

fn raw_off() {
    let _ = crossterm::terminal::disable_raw_mode();
}

/// One-line summary of what `Start` will do with this config.
fn describe_start(cfg: &DeckConfig) -> String {
    match cfg.default_mode.as_str() {
        "sim" => format!(
            "SIM · WASD @ {:.4},{:.4} r={:.0}m",
            cfg.sim_lat, cfg.sim_lon, cfg.sim_radius
        ),
        "serial" => format!(
            "GPS serial {} @ {} baud · fresh {:.0}m map",
            cfg.gps_port, cfg.baud, cfg.fetch_radius
        ),
        "gpio" => format!(
            "GPS gpio{} @ {} baud · fresh {:.0}m map",
            cfg.gpio_pin, cfg.baud, cfg.fetch_radius
        ),
        _ => format!(
            "AUTO · GPS {} @ {} baud · fresh {:.0}m map",
            cfg.gps_port, cfg.baud, cfg.fetch_radius
        ),
    }
}

fn draw_launcher(sel: usize, cfg: &DeckConfig) {
    clear_screen();
    println!("{GRN}{BOLD}  Z — D E C K{RST}{DIM} — Bit By Bit{RST}\n");
    let items = [
        format!("Start  ({})", describe_start(cfg)),
        "Settings".to_string(),
        "Quit to terminal".to_string(),
    ];
    for (i, label) in items.iter().enumerate() {
        if i == sel {
            println!("{BOLD}{GRN}  > {label}{RST}");
        } else {
            println!("{DIM}    {label}{RST}");
        }
    }
    println!("\n{DIM}  ↑/↓ or 1-3 · Enter select · Esc quits{RST}");
    let _ = std::io::stdout().flush();
}

/// Arrow-key menu. Returns when the user picks (or Esc/Ctrl+C = Quit).
/// Number keys 1/2/3 and j/k also work (CardKB-friendly).
fn launcher_menu(cfg: &DeckConfig) -> Launch {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};

    raw_on();
    let items = 3;
    let mut sel = 0usize;
    draw_launcher(sel, cfg);
    loop {
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        sel = (sel + items - 1) % items;
                        draw_launcher(sel, cfg);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        sel = (sel + 1) % items;
                        draw_launcher(sel, cfg);
                    }
                    KeyCode::Enter => break,
                    KeyCode::Esc => {
                        sel = 2;
                        break;
                    }
                    KeyCode::Char('1') => {
                        sel = 0;
                        break;
                    }
                    KeyCode::Char('2') => {
                        sel = 1;
                        break;
                    }
                    KeyCode::Char('3') => {
                        sel = 2;
                        break;
                    }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        sel = 2;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    raw_off();
    println!();
    match sel {
        0 => Launch::Start,
        1 => Launch::Settings,
        _ => Launch::Quit,
    }
}

/// Line-input prompt (raw mode off while typing). Empty = keep current.
fn ask_line(prompt: &str, current: &str) -> Option<String> {
    raw_off();
    print!("{BOLD}{prompt}{RST} {DIM}[{current}]{RST}: ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let ok = std::io::stdin().read_line(&mut s).is_ok();
    raw_on();
    if !ok {
        return None;
    }
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn persist(path: &Path, cfg: &DeckConfig, note: &mut String) {
    match save_config(path, cfg) {
        Ok(()) => *note = format!("saved {}", path.display()),
        Err(e) => *note = format!("SAVE FAILED: {e:#}"),
    }
}

fn draw_settings(sel: usize, cfg: &DeckConfig, note: &str) {
    clear_screen();
    println!("{GRN}{BOLD}  Settings{RST}{DIM} — Enter edits · Esc back (edits save instantly){RST}\n");
    let rows = [
        format!("Default mode (Start launches this): {}", cfg.default_mode),
        format!("SIM latitude: {}", cfg.sim_lat),
        format!("SIM longitude: {}", cfg.sim_lon),
        format!("SIM map radius (m): {:.0}", cfg.sim_radius),
        format!("AUTO fetch radius (m): {:.0}", cfg.fetch_radius),
        format!("GPS serial port: {}", cfg.gps_port),
        format!("GPIO pin (BCM): {}", cfg.gpio_pin),
        format!("Baud rate: {}", cfg.baud),
        format!("GPS fix hint every (s): {}", cfg.fix_hint),
        "Reset all to defaults".to_string(),
        "Back".to_string(),
    ];
    for (i, row) in rows.iter().enumerate() {
        if i == sel {
            println!("{BOLD}{GRN}  > {row}{RST}");
        } else {
            println!("{DIM}    {row}{RST}");
        }
    }
    if note.is_empty() {
        println!("\n{DIM}  ↑/↓ move · Enter edit/cycle · Esc back{RST}");
    } else {
        println!("\n{DIM}  {note}{RST}");
    }
    let _ = std::io::stdout().flush();
}

/// Settings editor. Every successful edit is sanitised + saved at once.
fn settings_menu(path: &Path, cfg: &mut DeckConfig) {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};

    const ROWS: usize = 11;
    raw_on();
    let mut sel = 0usize;
    let mut note = String::new();
    loop {
        draw_settings(sel, cfg, &note);
        note.clear();
        let mut back = false;
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        sel = (sel + ROWS - 1) % ROWS;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        sel = (sel + 1) % ROWS;
                    }
                    KeyCode::Esc => break,
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Enter | KeyCode::Char(' ') => match sel {
                        0 => {
                            cfg.cycle_mode();
                            persist(path, cfg, &mut note);
                        }
                        1 => {
                            if let Some(v) = ask_line("SIM latitude (-90..90)", &cfg.sim_lat.to_string())
                            {
                                match parse_f64(&v) {
                                    Some(f) => {
                                        cfg.sim_lat = f.clamp(-90.0, 90.0);
                                        persist(path, cfg, &mut note);
                                    }
                                    None => note = "invalid number — kept old".to_string(),
                                }
                            }
                        }
                        2 => {
                            if let Some(v) = ask_line("SIM longitude (-180..180)", &cfg.sim_lon.to_string())
                            {
                                match parse_f64(&v) {
                                    Some(f) => {
                                        cfg.sim_lon = f.clamp(-180.0, 180.0);
                                        persist(path, cfg, &mut note);
                                    }
                                    None => note = "invalid number — kept old".to_string(),
                                }
                            }
                        }
                        3 => {
                            if let Some(v) = ask_line("SIM map radius, meters (50..2000)", &format!("{:.0}", cfg.sim_radius))
                            {
                                match parse_f64(&v) {
                                    Some(f) => {
                                        cfg.sim_radius = f.clamp(50.0, 2000.0);
                                        persist(path, cfg, &mut note);
                                    }
                                    None => note = "invalid number — kept old".to_string(),
                                }
                            }
                        }
                        4 => {
                            if let Some(v) = ask_line("AUTO fetch radius, meters (50..2000)", &format!("{:.0}", cfg.fetch_radius))
                            {
                                match parse_f64(&v) {
                                    Some(f) => {
                                        cfg.fetch_radius = f.clamp(50.0, 2000.0);
                                        persist(path, cfg, &mut note);
                                    }
                                    None => note = "invalid number — kept old".to_string(),
                                }
                            }
                        }
                        5 => {
                            if let Some(v) = ask_line("GPS serial port", &cfg.gps_port) {
                                if !v.trim().is_empty() {
                                    cfg.gps_port = v.trim().to_string();
                                    persist(path, cfg, &mut note);
                                }
                            }
                        }
                        6 => {
                            if let Some(v) = ask_line("GPIO pin, BCM (0..27)", &cfg.gpio_pin.to_string()) {
                                match v.trim().parse::<u32>() {
                                    Ok(p) if p <= 27 => {
                                        cfg.gpio_pin = p;
                                        persist(path, cfg, &mut note);
                                    }
                                    _ => note = "invalid pin (0..27) — kept old".to_string(),
                                }
                            }
                        }
                        7 => {
                            if let Some(v) = ask_line("Baud rate", &cfg.baud.to_string()) {
                                match v.trim().parse::<u32>() {
                                    Ok(b) if b > 0 => {
                                        cfg.baud = b;
                                        persist(path, cfg, &mut note);
                                    }
                                    _ => note = "invalid baud — kept old".to_string(),
                                }
                            }
                        }
                        8 => {
                            if let Some(v) = ask_line("GPS fix hint every, seconds (>=10)", &cfg.fix_hint.to_string())
                            {
                                match v.trim().parse::<u64>() {
                                    Ok(h) => {
                                        cfg.fix_hint = h.max(10);
                                        persist(path, cfg, &mut note);
                                    }
                                    _ => note = "invalid number — kept old".to_string(),
                                }
                            }
                        }
                        9 => {
                            *cfg = DeckConfig::default();
                            persist(path, cfg, &mut note);
                            note = format!("reset to defaults; {}", note);
                        }
                        _ => back = true,
                    },
                    _ => {}
                }
            }
        }
        if back {
            break;
        }
    }
    raw_off();
    println!();
}

/// Quit to a normal shell. Replaces this process, so the user gets a real
/// terminal; exiting it returns to whatever launched us (deck menu / login).
fn quit_to_shell() -> ! {
    raw_off();
    clear_screen();
    println!("{DIM}Z-DECK closed — normal terminal. `exit`/logout returns to the deck.{RST}");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let err = Command::new(&shell).arg("-l").exec();
        eprintln!("could not start {shell}: {err}");
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    std::process::exit(42);
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

/// Fill session values from the saved config wherever the CLI left a
/// default. (An explicitly passed flag equal to the default resolves the
/// same way, so this is always safe.)
fn apply_config(a: &mut Args, cfg: &DeckConfig) {
    if a.baud == DEFAULT_BAUD {
        a.baud = cfg.baud;
    }
    if a.fix_hint == DEFAULT_FIX_HINT_S {
        a.fix_hint = cfg.fix_hint;
    }
    if a.lat.is_none() {
        a.lat = Some(cfg.sim_lat);
    }
    if a.lon.is_none() {
        a.lon = Some(cfg.sim_lon);
    }
}

/// Session radius: SIM uses the SIM map radius, hardware modes the fetch
/// radius — unless --radius was explicitly changed on the CLI.
fn apply_radius(a: &mut Args, cfg: &DeckConfig, sim: bool) {
    if (a.radius - AUTO_RADIUS_M).abs() < f64::EPSILON {
        a.radius = if sim { cfg.sim_radius } else { cfg.fetch_radius };
    }
}

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

fn run_gpio(bins: &Bins, args: &Args) -> Result<()> {
    let pin = args.gpio.unwrap_or(16);
    let mp = ensure_sim_map(bins, args)?;
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
    forward_test_flags(&mut cmd, args);
    println!("{DIM}$ {cmd:?}{RST}\n");
    let _ = cmd.status();
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    splash();

    // Explicit legacy/direct flags bypass the launcher menu.
    if args.sim {
        let bins = ensure_bins(&args)?;
        let mp = ensure_sim_map(&bins, &args)?;
        launch_sim(&bins, &args, &mp);
        return Ok(());
    }
    if args.gpio.is_some() {
        let bins = ensure_bins(&args)?;
        run_gpio(&bins, &args)?;
        return Ok(());
    }
    if args.gps.is_some() && (args.auto || args.yes) {
        // --gps with --yes/--auto: same AUTO flow (wait for lock, fresh
        // fetch) on the explicit port — no stale/default map.
        let bins = ensure_bins(&args)?;
        run_auto(&bins, &args)?;
        return Ok(());
    }

    if args.auto || args.yes {
        // Non-interactive kiosk: straight to AUTO, no menu.
        let bins = ensure_bins(&args)?;
        println!("\n{BOLD}Mode: AUTO{RST} {DIM}(--auto, skipping menu){RST}");
        return run_auto(&bins, &args);
    }

    // Interactive default: launcher menu. `Start` runs one session with
    // the saved settings, then returns here; direct flags above bypass.
    let cfg_path = config_path(args.config.as_deref());
    let mut cfg = load_config(&cfg_path);
    if cfg_path.is_file() {
        println!("  {DIM}settings: {}{RST}", cfg_path.display());
    }
    let bins = ensure_bins(&args)?;
    loop {
        match launcher_menu(&cfg) {
            Launch::Settings => settings_menu(&cfg_path, &mut cfg),
            Launch::Quit => quit_to_shell(),
            Launch::Start => {
                let mut a = args.clone();
                apply_config(&mut a, &cfg);
                match cfg.default_mode.as_str() {
                    "sim" => {
                        apply_radius(&mut a, &cfg, true);
                        let mp = ensure_sim_map(&bins, &a)?;
                        launch_sim(&bins, &a, &mp);
                    }
                    "serial" => {
                        apply_radius(&mut a, &cfg, false);
                        a.gps = Some(cfg.gps_port.clone());
                        run_auto(&bins, &a)?;
                    }
                    "gpio" => {
                        apply_radius(&mut a, &cfg, false);
                        a.gpio = Some(cfg.gpio_pin);
                        run_gpio(&bins, &a)?;
                    }
                    _ => {
                        apply_radius(&mut a, &cfg, false);
                        run_auto(&bins, &a)?;
                    }
                }
                println!("\n{DIM}session over — back to launcher.{RST}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let mut c = DeckConfig::default();
        c.default_mode = "sim".to_string();
        c.sim_lat = 1.5;
        c.gps_port = "/dev/ttyUSB0".to_string();
        let s = serde_json::to_string(&c).unwrap();
        let d: DeckConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(d.default_mode, "sim");
        assert!((d.sim_lat - 1.5).abs() < 1e-9);
        assert_eq!(d.gps_port, "/dev/ttyUSB0");
    }

    #[test]
    fn config_sanitise_clamps() {
        let mut c = DeckConfig {
            default_mode: "bogus".to_string(),
            sim_lat: 999.0,
            sim_lon: -999.0,
            sim_radius: 5.0,
            fetch_radius: 99999.0,
            gps_port: "   ".to_string(),
            gpio_pin: 99,
            baud: 0,
            fix_hint: 1,
        };
        c.sanitise();
        assert_eq!(c.default_mode, "auto");
        assert_eq!(c.sim_lat, 90.0);
        assert_eq!(c.sim_lon, -180.0);
        assert_eq!(c.sim_radius, 50.0);
        assert_eq!(c.fetch_radius, 2000.0);
        assert_eq!(c.gps_port, DEFAULT_GPS_PORT);
        assert_eq!(c.gpio_pin, 16);
        assert_eq!(c.baud, DEFAULT_BAUD);
        assert_eq!(c.fix_hint, 10);
    }

    #[test]
    fn config_cycle_round_robin() {
        let mut c = DeckConfig::default();
        assert_eq!(c.default_mode, "auto");
        c.cycle_mode();
        assert_eq!(c.default_mode, "sim");
        c.cycle_mode();
        assert_eq!(c.default_mode, "serial");
        c.cycle_mode();
        assert_eq!(c.default_mode, "gpio");
        c.cycle_mode();
        assert_eq!(c.default_mode, "auto");
    }

    #[test]
    fn config_missing_file_gives_defaults() {
        let d = load_config(Path::new("/nonexistent-dir/zdeck.conf"));
        assert_eq!(d.default_mode, "auto");
        assert_eq!(d.baud, DEFAULT_BAUD);
    }

    #[test]
    fn config_invalid_json_gives_defaults() {
        let p = std::env::temp_dir().join("zdeck-test-bad.conf");
        std::fs::write(&p, "{not json").unwrap();
        let d = load_config(&p);
        assert_eq!(d.default_mode, "auto");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn config_save_load_roundtrip_file() {
        let p = std::env::temp_dir().join("zdeck-test.conf");
        let mut c = DeckConfig::default();
        c.sim_radius = 500.0;
        save_config(&p, &c).unwrap();
        let d = load_config(&p);
        assert!((d.sim_radius - 500.0).abs() < 1e-9);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn describe_start_mentions_mode() {
        let mut c = DeckConfig::default();
        assert!(describe_start(&c).starts_with("AUTO"));
        c.default_mode = "sim".to_string();
        assert!(describe_start(&c).starts_with("SIM"));
        c.default_mode = "gpio".to_string();
        assert!(describe_start(&c).contains("gpio16"));
    }
}
