//! `zdeck-game` — main app binary (the STABLE half).
//!
//! Renders the zombie chase TUI and owns the game simulation. It NEVER talks
//! to GPS hardware directly: position fixes arrive either from an internal
//! WASD simulator (`--sim`), from NDJSON on stdin (`--stdin`), or from a
//! spawned `zdeck-gps` child process whose stdout is the ONLY coupling.
//!
//! ```text
//!   zdeck-game --sim                                        # indoor testing
//!   zdeck-game --stdin            < fixes.ndjson            # any producer
//!   zdeck-gps --source serial --port /dev/ttyUSB0 | zdeck-game --stdin
//!   zdeck-game --gps-source gpio --gpio 16                 # spawns zdeck-gps
//! ```
//!
//! Hardware swap rule: to change GPS module / baud / GPIO / protocol quirks,
//! replace ONLY the `zdeck-gps` binary (or point `--gps-bin` at the new one).
//! This binary stays untouched as long as NDJSON fixes keep flowing.

use std::io::{BufRead, BufReader, Stdout};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::Alignment,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Terminal,
};

use zdeck::{
    config::*,
    geo::*,
    map::{fill_poly, AreaKind, MapData, Segment, Viewport},
    proto::GpsFix,
    rng::XorShift64,
    zombie::{Horde, HordeCfg},
};

#[derive(Parser, Debug)]
#[command(
    name = "zdeck-game",
    about = "Zombie Deck: GPS zombie chase on your terminal"
)]
struct Args {
    /// Use keyboard-simulated GPS (WASD) for indoor testing
    #[arg(long)]
    sim: bool,
    /// Read NDJSON fixes from stdin (pairs with any `zdeck-gps ...` producer)
    #[arg(long)]
    stdin: bool,
    /// GPS preprocessor binary to spawn (swap this file to change GPS hardware)
    #[arg(long, default_value = "zdeck-gps")]
    gps_bin: String,
    /// Source passed to the spawned zdeck-gps: gpio | serial | sim
    #[arg(long, default_value = "gpio")]
    gps_source: String,
    /// Serial port passed to zdeck-gps (gps_source=serial)
    #[arg(long)]
    gps_port: Option<String>,
    /// GPIO pin (BCM) passed to zdeck-gps (gps_source=gpio)
    #[arg(long, default_value_t = 16)]
    gpio: u32,
    /// Baud rate passed to zdeck-gps
    #[arg(long, default_value_t = 9600)]
    baud: u32,
    /// Path to offline map file produced by fetch_map.py / zdeck-fetch
    #[arg(long, default_value = "map_data.json")]
    map: String,
    /// Run without TUI (testing / benchmarks); sim auto-walks east
    #[arg(long)]
    headless: bool,
    /// Ticks to run in headless mode
    #[arg(long, default_value_t = 40)]
    ticks: u64,
    /// Min zombies: reinforcements spawn while below this
    #[arg(long, default_value_t = MIN_ZOMBIES)]
    min_zombies: usize,
    /// Max concurrent zombies (hard cap)
    #[arg(long, default_value_t = MAX_ZOMBIES)]
    max_zombies: usize,
    /// Zombies farther than this are out of range (meters)
    #[arg(long, default_value_t = DESPAWN_RANGE_M)]
    despawn_range: f64,
    /// Out-of-range zombies despawn after this many seconds away
    #[arg(long, default_value_t = DESPAWN_AFTER_S)]
    despawn_after: f64,
    /// Max reinforcement spawns per second
    #[arg(long, default_value_t = MAX_SPAWN_PER_S)]
    spawn_per_s: f64,
}

// ---------------------------------------------------------------- TUI guard

struct Tui {
    term: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    fn new() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut out = std::io::stdout();
        execute!(out, EnterAlternateScreen, cursor::Hide)?;
        Ok(Self {
            term: Terminal::new(CrosstermBackend::new(out))?,
        })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(self.term.backend_mut(), LeaveAlternateScreen, cursor::Show);
    }
}

// ---------------------------------------------------------------- rendering

/// Cell attribute = curses color-pair number from the Python original, so map
/// styling stays familiar: 1 player, 2 zombie/alert, 3 status, 4 roads,
/// 5 school, 6 worship, 7 food, 8 park/other. 0 = background.
/// v2 additions: 9 water, 10 buildings, 11 green areas, 12 arterial roads,
/// 13 supply POIs, 14 culture POIs, 15 shelter POIs.
fn style_for(attr: u8) -> Style {
    match attr {
        1 => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        2 => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        3 => Style::default().fg(Color::White),
        4 => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::DIM),
        5 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        6 => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        7 => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        8 => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        9 => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        10 => Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
        11 => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::DIM),
        12 => Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
        13 => Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
        14 => Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::Cyan),
    }
}

fn poi_attr(category: &str) -> u8 {
    match category {
        "school" => 5,
        "worship" => 6,
        "hospital" => 2,
        "food" => 7,
        "supply" => 13,
        "culture" => 14,
        "shelter" => 15,
        "water" => 9,
        "fuel" | "bank" | "civic" => 3,
        "police" => 2,
        _ => 8,
    }
}

/// Reused character + attribute grid: zero per-frame allocation in steady
/// state (resized only when the terminal itself is resized).
struct FrameBuf {
    chars: Vec<char>,
    attr: Vec<u8>,
    w: usize,
    h: usize,
}

impl FrameBuf {
    fn new() -> Self {
        Self {
            chars: Vec::new(),
            attr: Vec::new(),
            w: 0,
            h: 0,
        }
    }

    fn resize(&mut self, w: usize, h: usize) {
        if w != self.w || h != self.h {
            self.chars = vec![' '; w * h];
            self.attr = vec![0; w * h];
            self.w = w;
            self.h = h;
        } else {
            self.chars.fill(' ');
            self.attr.fill(0);
        }
    }

    #[inline]
    fn put(&mut self, gx: i32, gy: i32, ch: char, a: u8) {
        if gx >= 0 && gy >= 0 && (gx as usize) < self.w && (gy as usize) < self.h {
            let i = gy as usize * self.w + gx as usize;
            self.chars[i] = ch;
            self.attr[i] = a;
        }
    }
}

/// Plot roads / POIs / player / zombies, then flush as run-compressed spans.
fn draw(tui: &mut Tui, fb: &FrameBuf, status: &str) -> Result<()> {
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(fb.h + 1);
    for y in 0..fb.h {
        let base = y * fb.w;
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut cur_a = fb.attr[base];
        let mut cur = String::new();
        for x in 0..fb.w {
            let a = fb.attr[base + x];
            if a != cur_a && !cur.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut cur), style_for(cur_a)));
                cur_a = a;
            }
            cur.push(fb.chars[base + x]);
        }
        if !cur.is_empty() {
            spans.push(Span::styled(cur, style_for(cur_a)));
        }
        lines.push(Line::from(spans));
    }
    let width = tui.term.size().map(|s| s.width as usize).unwrap_or(80);
    let status: String = status.chars().take(width.saturating_sub(1)).collect();
    lines.push(Line::from(Span::styled(status, style_for(3))));
    tui.term.draw(|f| {
        f.render_widget(Paragraph::new(lines), f.area());
    })?;
    Ok(())
}

/// Max area-fill cells per frame (shared budget across all polygons).
const FILL_BUDGET_PER_FRAME: usize = 1500;

/// Draw one projected segment with capped interpolation (perf safety cap).
fn draw_seg(fb: &mut FrameBuf, view: &Viewport, s: &Segment, ch: char, attr: u8) {
    let (gx1, gy1) = world_to_screen(
        s.x1,
        s.y1,
        view.player_x,
        view.player_y,
        view.cx,
        view.cy,
        SCALE_M_PER_CELL,
    );
    let (gx2, gy2) = world_to_screen(
        s.x2,
        s.y2,
        view.player_x,
        view.player_y,
        view.cx,
        view.cy,
        SCALE_M_PER_CELL,
    );
    let margin = 5;
    let max_x = fb.w as i32;
    let max_y = fb.h as i32 + 1; // +1: last grid row is second-to-last screen row
    if (gx1 < -margin && gx2 < -margin) || (gx1 > max_x + margin && gx2 > max_x + margin) {
        return;
    }
    if (gy1 < -margin && gy2 < -margin) || (gy1 > max_y + margin && gy2 > max_y + margin) {
        return;
    }
    let steps = (gx2 - gx1)
        .abs()
        .max((gy2 - gy1).abs())
        .clamp(1, MAP_LINE_STEP_CAP);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let gx = (gx1 as f64 + (gx2 - gx1) as f64 * t).round() as i32;
        let gy = (gy1 as f64 + (gy2 - gy1) as f64 * t).round() as i32;
        fb.put(gx, gy, ch, attr);
    }
}

fn plot_world(fb: &mut FrameBuf, map: &MapData, px: f64, py: f64, horde: &Horde) {
    let view = Viewport {
        player_x: px,
        player_y: py,
        cx: fb.w as i32 / 2,
        cy: fb.h as i32 / 2,
        scale_m_per_cell: SCALE_M_PER_CELL,
        w: fb.w as i32,
        h: fb.h as i32,
    };

    // Areas first (background): budgeted polygon fill.
    let mut budget = FILL_BUDGET_PER_FRAME;
    for area in &map.areas {
        let (ch, attr) = match area.kind {
            AreaKind::Water => ('~', 9),
            AreaKind::Building => ('#', 10),
            AreaKind::Green => (':', 11),
        };
        fill_poly(&area.poly, &view, &mut budget, |gx, gy| {
            fb.put(gx, gy, ch, attr)
        });
        if budget == 0 {
            break;
        }
    }

    // Legacy (unnamed) road geometry, then typed roads (arterials brighter).
    for s in &map.segments {
        draw_seg(fb, &view, s, '.', 4);
    }
    for road in &map.roads {
        let attr = if road.major { 12 } else { 4 };
        for s in &road.segs {
            draw_seg(fb, &view, s, '.', attr);
        }
    }

    for poi in &map.pois {
        let (gx, gy) = world_to_screen(poi.x, poi.y, px, py, view.cx, view.cy, SCALE_M_PER_CELL);
        let a = poi_attr(&poi.category);
        for (i, ch) in poi.name.chars().enumerate() {
            fb.put(gx + i as i32, gy, ch, a);
        }
    }

    for m in &horde.members {
        let (gx, gy) =
            world_to_screen(m.pos.x, m.pos.y, px, py, view.cx, view.cy, SCALE_M_PER_CELL);
        fb.put(gx, gy, 'Z', 2);
    }
    fb.put(view.cx, view.cy, '@', 1);
}

fn death_screen(tui: &mut Tui) -> Result<()> {
    tui.term.draw(|f| {
        let msg = vec![
            Line::from(""),
            Line::from(Span::styled("YOU DIED", style_for(2))),
            Line::from(""),
            Line::from(Span::styled("press q to quit", style_for(3))),
        ];
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center), f.area());
    })?;
    loop {
        if let Event::Key(k) = event::read()? {
            if matches!(k.code, KeyCode::Char('q' | 'Q') | KeyCode::Esc) {
                break;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- GPS feeds

/// WASD-driven fake GPS (indoor testing). Same math as Python's SimulatedGPS.
struct SimGps {
    origin_lat: f64,
    origin_lon: f64,
    x: f64,
    y: f64,
}

impl SimGps {
    fn latlon(&self) -> (f64, f64) {
        local_to_latlon(self.x, self.y, self.origin_lat, self.origin_lon)
    }
}

fn feed_from_stdin() -> Receiver<GpsFix> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if let Some(f) = GpsFix::decode(&line) {
                if tx.send(f).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

fn feed_from_pipe(out: std::process::ChildStdout) -> Receiver<GpsFix> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(f) = GpsFix::decode(&line) {
                if tx.send(f).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

// ---------------------------------------------------------------- main

fn main() -> Result<()> {
    let args = Args::parse();
    if args.sim && args.stdin {
        bail!("--sim and --stdin are exclusive");
    }
    if !args.sim && !args.stdin && !matches!(args.gps_source.as_str(), "gpio" | "serial" | "sim") {
        bail!(
            "--gps-source must be gpio|serial|sim (got '{}')",
            args.gps_source
        );
    }

    let map = MapData::load(&args.map, 10.0, 76.3);
    if map.segments.is_empty() {
        eprintln!(
            "No map data at '{}' -- blank background. Run zdeck-fetch first.",
            args.map
        );
    } else if args.headless {
        println!(
            "map: origin {},{} | {} segments, {} POIs, {} roads, {} areas",
            map.origin_lat,
            map.origin_lon,
            map.segments.len(),
            map.pois.len(),
            map.roads.len(),
            map.areas.len()
        );
    }

    // ---- GPS feed (the only hardware coupling in this binary) ----
    let mut sim = args.sim.then_some(SimGps {
        origin_lat: map.origin_lat,
        origin_lon: map.origin_lon,
        x: 0.0,
        y: 0.0,
    });
    let mut child: Option<Child> = None;
    let rx: Option<Receiver<GpsFix>> = if args.stdin {
        Some(feed_from_stdin())
    } else if !args.sim {
        let mut cmd = Command::new(&args.gps_bin);
        cmd.arg("--source")
            .arg(&args.gps_source)
            .arg("--baud")
            .arg(args.baud.to_string())
            .arg("--gpio")
            .arg(args.gpio.to_string());
        if let Some(p) = &args.gps_port {
            cmd.arg("--port").arg(p);
        }
        if args.gps_source == "sim" {
            // Parity with Python's SimulatedGPS(map_origin_*): the synthetic
            // fix must start where the map is, not at a compiled-in default.
            cmd.arg("--lat").arg(map.origin_lat.to_string());
            cmd.arg("--lon").arg(map.origin_lon.to_string());
        }
        let mut c = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to spawn '{}' -- build it (cargo build --bin zdeck-gps) \
                     or pass --gps-bin <path> / use --sim",
                    args.gps_bin
                )
            })?;
        // Split roles: the reader thread owns ONLY the stdout pipe (EOF is
        // our disconnect signal); main keeps the Child handle for kill/wait.
        let out = c.stdout.take().context("child stdout not piped")?;
        child = Some(c);
        Some(feed_from_pipe(out))
    } else {
        None
    };

    let mut tui: Option<Tui> = if args.headless {
        None
    } else {
        Some(Tui::new()?)
    };
    let mut fb = FrameBuf::new();
    let mut rng = if args.headless {
        XorShift64::new(1234)
    } else {
        XorShift64::from_time()
    };
    let mut horde: Option<Horde> = None;
    let horde_cfg = HordeCfg {
        min: args.min_zombies,
        max: args.max_zombies.max(1),
        despawn_range_m: args.despawn_range,
        despawn_after_s: args.despawn_after.max(0.0),
        max_spawn_per_s: args.spawn_per_s,
        spawn_trigger_m: SPAWN_TRIGGER_M,
    };
    let mut fix: Option<GpsFix> = None;
    let mut feed_dead = false;

    let frame_dt = Duration::from_micros(1_000_000 / TICK_HZ);
    let step_len = ZOMBIE_SPEED_MPS * PLAYER_SPEED_MULT / TICK_HZ as f64;
    let max_ticks = if args.headless {
        args.ticks.max(1)
    } else {
        u64::MAX
    };
    let mut tick = 0u64;
    let mut work_ns: u128 = 0;

    loop {
        let t0 = Instant::now();

        // ---- input ----
        if tui.is_some() {
            while event::poll(Duration::from_millis(0))? {
                if let Event::Key(k) = event::read()? {
                    if let KeyCode::Char(c) = k.code {
                        match c.to_ascii_lowercase() {
                            'q' => return Ok(()),
                            'w' if sim.is_some() => sim.as_mut().unwrap().y += step_len,
                            's' if sim.is_some() => sim.as_mut().unwrap().y -= step_len,
                            'a' if sim.is_some() => sim.as_mut().unwrap().x -= step_len,
                            'd' if sim.is_some() => sim.as_mut().unwrap().x += step_len,
                            _ => {}
                        }
                    } else if matches!(k.code, KeyCode::Esc) {
                        return Ok(());
                    }
                }
            }
        }

        // ---- latest fix (drain to newest; never block the tick) ----
        if let Some(s) = sim.as_mut() {
            if args.headless {
                s.x += step_len; // auto-walk east so headless runs exercise motion
            }
            let (lat, lon) = s.latlon();
            fix = Some(GpsFix::new(lat, lon, 1));
        } else if let Some(r) = rx.as_ref() {
            loop {
                match r.try_recv() {
                    Ok(f) => fix = Some(f),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        feed_dead = true;
                        break;
                    }
                }
            }
        }

        let Some(cur) = fix else {
            if feed_dead {
                bail!("GPS feed ended before the first fix (is the module attached?)");
            }
            if let Some(t) = tui.as_mut() {
                let (w, h) = (
                    t.term.size()?.width as usize,
                    t.term.size()?.height as usize,
                );
                fb.resize(w, h.saturating_sub(1));
                draw(t, &fb, "Waiting for GPS fix... (q=quit)")?;
            } else {
                std::thread::sleep(Duration::from_millis(50));
            }
            continue;
        };

        // Lazily center the opening horde on the first real fix (the player
        // may be far from the map origin on real hardware).
        let (px, py) = latlon_to_local(cur.lat, cur.lon, map.origin_lat, map.origin_lon);
        let h = horde.get_or_insert_with(|| Horde::new(horde_cfg.clone(), &mut rng, px, py));

        let dt = 1.0 / TICK_HZ as f64;
        let rep = h.update(px, py, dt, &mut rng);
        let min_dist = rep.min_dist;
        if args.headless && (rep.spawned > 0 || rep.despawned > 0) {
            println!(
                "tick {tick}: horde +{} -{} (z:{})",
                rep.spawned,
                rep.despawned,
                h.len()
            );
        }

        let near = map.nearest_poi(px, py, POI_CALLOUT_RADIUS_M);
        let road = map.nearest_road(px, py, 12.0);
        // Nearby-place detail: name + opening hours (or address) when known.
        let mut detail = String::new();
        if let Some(p) = near {
            detail.push_str("near: ");
            detail.push_str(&p.name);
            let extra = p.hours.as_deref().or(p.addr.as_deref());
            if let Some(e) = extra {
                detail.push_str(" (");
                detail.extend(e.chars().take(24));
                detail.push(')');
            }
            detail.push_str("  ");
        }
        if let Some(r) = road {
            detail.push_str("on: ");
            detail.extend(r.chars().take(30));
            detail.push_str("  ");
        }
        let dist_txt = if h.is_empty() {
            "--".to_string()
        } else {
            format!("{min_dist:5.1}m")
        };
        let status = format!(
            "nearest zombie: {dist_txt} (z:{})  {detail}{}q=quit",
            h.len(),
            if sim.is_some() { "wasd to move, " } else { "" },
        );

        if let Some(t) = tui.as_mut() {
            let (w, hgt) = (
                t.term.size()?.width as usize,
                t.term.size()?.height as usize,
            );
            fb.resize(w, hgt.saturating_sub(1));
            plot_world(&mut fb, &map, px, py, h);
            draw(t, &fb, &status)?;
        } else {
            // Headless still rasterizes into a scratch buffer: exercises the
            // area/road render path, and its cost shows in tick-work timing.
            fb.resize(80, 23);
            plot_world(&mut fb, &map, px, py, h);
            if tick.is_multiple_of(10) {
                println!("tick {tick}: {status}");
            }
        }

        if min_dist <= CATCH_RADIUS_M {
            if let Some(t) = tui.as_mut() {
                death_screen(t)?;
            } else {
                println!("tick {tick}: YOU DIED ({status})");
            }
            break;
        }

        tick += 1;
        if tick >= max_ticks {
            break;
        }
        work_ns += t0.elapsed().as_nanos();
        if !args.headless {
            let elapsed = t0.elapsed();
            if elapsed < frame_dt {
                std::thread::sleep(frame_dt - elapsed);
            }
        }
    }

    if let Some(mut c) = child {
        let _ = c.kill();
        let _ = c.wait();
    }
    if args.headless {
        let avg_us = if tick > 0 {
            work_ns / tick as u128 / 1000
        } else {
            0
        };
        println!("done: {tick} ticks, avg tick work {avg_us}us");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zdeck::zombie::ZombieState;

    #[test]
    fn plot_centers_player_and_draws_roads() {
        let map = MapData::from_str(
            r#"{"origin_lat": 10.0, "origin_lon": 76.3, "radius_m": 300,
                "ways": [[[10.0, 76.3], [10.0, 76.301]]],
                "pois": [{"lat": 10.0, "lon": 76.3, "name": "Kada", "category": "food"}]}"#,
            0.0,
            0.0,
        );
        let mut fb = FrameBuf::new();
        fb.resize(80, 23);
        let mut horde = Horde::new(HordeCfg::default(), &mut XorShift64::new(9), 0.0, 0.0);
        horde.members.clear();
        horde.members.push(ZombieState::new(3.0, 0.0));
        plot_world(&mut fb, &map, 0.0, 0.0, &horde);
        // player at center
        let c = 11 * 80 + 40;
        assert_eq!((fb.chars[c], fb.attr[c]), ('@', 1));
        // POI name overwrote road start with category color (food -> 7)
        assert_eq!(fb.chars[c], '@'); // player wins over POI at same cell
                                      // zombie one cell east (3m / 3m-per-cell)
        assert_eq!((fb.chars[11 * 80 + 41], fb.attr[11 * 80 + 41]), ('Z', 2));
        // road extends east: some '.' must exist on the center row
        assert!((0..80).any(|x| fb.chars[11 * 80 + x] == '.'));
    }

    #[test]
    fn framebuf_reuse_does_not_reallocate() {
        let mut fb = FrameBuf::new();
        fb.resize(80, 24);
        let cap = fb.chars.capacity();
        fb.put(0, 0, 'X', 4);
        fb.resize(80, 24);
        assert_eq!(fb.chars.capacity(), cap);
        assert_eq!(fb.chars[0], ' '); // cleared, not reallocated
    }
}
