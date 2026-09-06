//! `zdeck-gps` — GPS preprocessing binary (the SWAPPABLE half).
//!
//! Reads raw NMEA from ONE hardware source, validates + smooths + throttles,
//! and prints normalized [`zdeck::proto::GpsFix`] NDJSON lines on stdout.
//!
//! ```text
//!   NEO-6M --(UART NMEA)--> [ zdeck-gps ] --(NDJSON fix)--> [ zdeck-game ]
//!   swap this binary ^^^      stdout contract (see zdeck::proto)   untouched
//! ```
//!
//! Sources:
//! * `serial` — plain `/dev/tty*` device file (baud set via `stty`, best
//!   effort; works for NEO-6M on ttyAMA0/ttyUSB0/rfcomm0).
//! * `gpio`   — bit-banged serial on a plain GPIO pin via the pigpiod socket
//!   interface (TCP 127.0.0.1:8888, no C deps). Same wiring as the Python
//!   version: GPS TX -> GPIO pin, e.g. GPIO16 when the TFT owns pins 8/10.
//! * `stdin`  — NMEA lines on stdin (replay logs, USB-serial helpers...).
//! * `sim`    — synthetic fixes for pipeline testing (with `--sim-speed`
//!   eastward drift so `zdeck-game --stdin` can chase something).
//!
//! Preprocessing (all cheap integer/float ops, O(1) memory):
//! 1. checksum gate + GGA/RMC parse (zdeck::nmea),
//! 2. fix-quality gate (`fix > 0`),
//! 3. outlier gate (drop jumps faster than `--max-speed` m/s — GPS glitches),
//! 4. optional moving-average smoothing (`--smooth N`),
//! 5. emit throttle (moved > `--throttle-m` OR 1s elapsed).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};

use zdeck::geo::haversine_m;
use zdeck::nmea::{parse_line, NmeaFix};
use zdeck::proto::{now_ms, GpsFix};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Source {
    Serial,
    Gpio,
    Stdin,
    Sim,
}

#[derive(Parser, Debug)]
#[command(
    name = "zdeck-gps",
    about = "GPS preprocessor: NMEA in, NDJSON fixes out"
)]
struct Args {
    /// NMEA source to read from
    #[arg(long, value_enum, default_value = "gpio")]
    source: Source,
    /// Serial device (source=serial), e.g. /dev/ttyUSB0
    #[arg(long)]
    port: Option<String>,
    /// GPIO pin (BCM) wired to GPS TX (source=gpio)
    #[arg(long, default_value_t = 16)]
    gpio: u32,
    /// Baud rate (sources serial/gpio)
    #[arg(long, default_value_t = 9600)]
    baud: u32,
    /// pigpiod host (source=gpio)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// pigpiod TCP port (source=gpio)
    #[arg(long, default_value_t = 8888)]
    pigpio_port: u16,
    /// Moving-average window over fixes (1 = off)
    #[arg(long, default_value_t = 1)]
    smooth: usize,
    /// Emit when moved more than this (meters) or 1s elapsed
    #[arg(long, default_value_t = 0.5)]
    throttle_m: f64,
    /// Drop jumps faster than this (m/s) as GPS glitches
    #[arg(long, default_value_t = 60.0)]
    max_speed: f64,
    /// Sim origin latitude (source=sim)
    #[arg(long, default_value_t = 10.0)]
    lat: f64,
    /// Sim origin longitude (source=sim)
    #[arg(long, default_value_t = 76.3)]
    lon: f64,
    /// Sim drift speed east, m/s (source=sim)
    #[arg(long, default_value_t = 0.0)]
    sim_speed: f64,
}

// ---------------------------------------------------------------- pipeline

/// O(1) preprocessing state shared by all sources.
struct Pipe {
    smooth: usize,
    buf: VecDeque<(f64, f64)>,
    last_accept: Option<(f64, f64, u64)>,
    last_emit: Option<(f64, f64, u64)>,
    throttle_m: f64,
    max_speed: f64,
}

impl Pipe {
    fn new(args: &Args) -> Self {
        Self {
            smooth: args.smooth.max(1),
            buf: VecDeque::with_capacity(args.smooth.max(1)),
            last_accept: None,
            last_emit: None,
            throttle_m: args.throttle_m,
            max_speed: args.max_speed,
        }
    }

    /// Feed one parsed fix; writes an NDJSON line only when it passes the
    /// outlier gate AND the emit throttle.
    fn push(&mut self, f: NmeaFix, out: &mut impl Write) -> Result<()> {
        let now = now_ms();
        if let Some((plat, plon, pt)) = self.last_accept {
            let dt = (now.saturating_sub(pt) as f64) / 1000.0;
            if dt > 0.0 && haversine_m(plat, plon, f.lat, f.lon) / dt > self.max_speed {
                return Ok(()); // teleport => glitch, drop it
            }
        }
        self.last_accept = Some((f.lat, f.lon, now));

        self.buf.push_back((f.lat, f.lon));
        if self.buf.len() > self.smooth {
            self.buf.pop_front();
        }
        let n = self.buf.len() as f64;
        let (slat, slon) = self
            .buf
            .iter()
            .fold((0.0, 0.0), |(a, b), &(x, y)| (a + x, b + y));
        let (slat, slon) = (slat / n, slon / n);

        let due = match self.last_emit {
            None => true,
            Some((elat, elon, et)) => {
                haversine_m(elat, elon, slat, slon) > self.throttle_m
                    || now.saturating_sub(et) >= 1000
            }
        };
        if due {
            let fix = GpsFix {
                lat: slat,
                lon: slon,
                fix: f.quality,
                t_ms: now,
            };
            writeln!(out, "{}", fix.encode())?;
            out.flush()?;
            self.last_emit = Some((slat, slon, now));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- sources

/// Blocking iterator of raw NMEA lines, one impl per hardware source.
trait Lines {
    fn next_line(&mut self) -> Option<String>;
}

struct ReaderLines<R: BufRead> {
    inner: R,
    buf: String,
}

impl<R: BufRead> Lines for ReaderLines<R> {
    fn next_line(&mut self) -> Option<String> {
        self.buf.clear();
        match self.inner.read_line(&mut self.buf) {
            Ok(0) => None, // EOF
            Ok(_) => Some(self.buf.clone()),
            Err(_) => None,
        }
    }
}

/// Synthetic GGA sentences (with valid checksums) for pipeline testing.
struct SimLines {
    lat: f64,
    lon: f64,
    speed: f64,
    step: u64,
}

impl SimLines {
    fn synth_gga(&self) -> String {
        // Decimal deg -> ddmm.mmmm / dddmm.mmmm.
        let la = self.lat.abs();
        let lo = self.lon.abs();
        let lad = la.floor() as u32;
        let lam = (la - lad as f64) * 60.0;
        let lod = lo.floor() as u32;
        let lom = (lo - lod as f64) * 60.0;
        let payload = format!(
            "GPGGA,120000,{:02}{:06.3},{},{:03}{:06.3},{},1,08,0.9,0.0,M,0.0,M,,",
            lad,
            lam,
            if self.lat >= 0.0 { "N" } else { "S" },
            lod,
            lom,
            if self.lon >= 0.0 { "E" } else { "W" },
        );
        let mut x: u8 = 0;
        for b in payload.bytes() {
            x ^= b;
        }
        format!("${payload}*{x:02X}")
    }
}

impl Lines for SimLines {
    fn next_line(&mut self) -> Option<String> {
        use zdeck::geo::EARTH_R_M;
        std::thread::sleep(Duration::from_millis(200)); // ~5Hz raw, GPS-like
                                                        // Drift east at sim_speed m/s.
        self.step += 1;
        self.lon += (self.speed * 0.2 / (EARTH_R_M * self.lat.to_radians().cos())).to_degrees();
        Some(self.synth_gga())
    }
}

// --- pigpiod socket client (bit-bang serial, pure std, no C deps) ---
//
// pigpiod listens on TCP 8888. Request = 4x u32 LE (cmd, p1, p2, ext_len)
// + ext bytes. Response = 16-byte echo with signed result in the last word,
// followed by `result` bytes for commands that return data.
// Opcodes: MODES=0, BSRO=118 (bb_serial_read_open), BSR=119
// (bb_serial_read), BSRC=120 (bb_serial_read_close). Needs pigpio V71+.

const PI_CMD_MODES: u32 = 0;
const PI_CMD_BSRO: u32 = 118;
const PI_CMD_BSR: u32 = 119;
const PI_CMD_BSRC: u32 = 120;
const PI_INPUT: u32 = 1;

struct Pigpio {
    s: TcpStream,
}

impl Pigpio {
    fn connect(host: &str, port: u16) -> Result<Self> {
        let s = TcpStream::connect((host, port)).with_context(|| {
            format!(
                "can't connect to pigpiod at {host}:{port} -- is it running? \
                 (sudo systemctl start pigpiod)"
            )
        })?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        Ok(Self { s })
    }

    fn cmd(&mut self, cmd: u32, p1: u32, p2: u32, ext: &[u8]) -> Result<(i32, Vec<u8>)> {
        use std::io::{Read, Write};
        let mut req = [0u8; 16];
        req[0..4].copy_from_slice(&cmd.to_le_bytes());
        req[4..8].copy_from_slice(&p1.to_le_bytes());
        req[8..12].copy_from_slice(&p2.to_le_bytes());
        req[12..16].copy_from_slice(&(ext.len() as u32).to_le_bytes());
        self.s.write_all(&req)?;
        self.s.write_all(ext)?;
        let mut res = [0u8; 16];
        self.s.read_exact(&mut res)?;
        let code = i32::from_le_bytes(res[12..16].try_into().unwrap());
        let mut data = Vec::new();
        if code > 0 {
            let n = (code as usize).min(65536);
            data.resize(n, 0);
            self.s.read_exact(&mut data)?;
        }
        Ok((code, data))
    }
}

struct GpioLines {
    pi: Pigpio,
    gpio: u32,
    buf: Vec<u8>,
}

impl GpioLines {
    fn open(host: &str, port: u16, gpio: u32, baud: u32) -> Result<Self> {
        let mut pi = Pigpio::connect(host, port)?;
        let (rc, _) = pi.cmd(PI_CMD_MODES, gpio, PI_INPUT, &[])?;
        if rc != 0 {
            bail!("pigpiod set_mode(gpio{gpio}) failed, rc={rc}");
        }
        let (rc, _) = pi.cmd(PI_CMD_BSRO, gpio, baud, &8u32.to_le_bytes())?;
        if rc != 0 {
            bail!("pigpiod bb_serial_read_open(gpio{gpio}, {baud}) failed, rc={rc}");
        }
        Ok(Self {
            pi,
            gpio,
            buf: Vec::with_capacity(4096),
        })
    }
}

impl Drop for GpioLines {
    fn drop(&mut self) {
        let _ = self.pi.cmd(PI_CMD_BSRC, self.gpio, 0, &[]);
    }
}

impl Lines for GpioLines {
    fn next_line(&mut self) -> Option<String> {
        // ~20Hz poll, plenty for a 1Hz GPS (mirrors the Python version).
        std::thread::sleep(Duration::from_millis(50));
        let (rc, data) = self.pi.cmd(PI_CMD_BSR, self.gpio, 4000, &[]).ok()?;
        if rc > 0 {
            self.buf.extend_from_slice(&data);
            if self.buf.len() > 65536 {
                self.buf.clear(); // desync safety
            }
        }
        let nl = self.buf.iter().position(|&b| b == b'\n')?;
        let raw: Vec<u8> = self.buf.drain(..=nl).collect();
        Some(String::from_utf8_lossy(&raw).into_owned())
    }
}

// ---------------------------------------------------------------- main

fn open_serial(port: &str, baud: u32) -> Result<ReaderLines<BufReader<File>>> {
    // Best effort: configure baud with stty (GPS dongles usually enumerate
    // at 9600 already; failure here is non-fatal).
    let _ = Command::new("stty")
        .args(["-F", port, &baud.to_string(), "raw", "-echo"])
        .status();
    let f = File::open(port)
        .with_context(|| format!("can't open serial port {port} (try --port /dev/ttyUSB0)"))?;
    Ok(ReaderLines {
        inner: BufReader::new(f),
        buf: String::with_capacity(128),
    })
}

fn main() -> Result<()> {
    let args = Args::parse();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut pipe = Pipe::new(&args);

    // Monomorphize per source so the hot loop stays a tight static call.
    match args.source {
        Source::Stdin => {
            let stdin = io::stdin();
            let mut src = ReaderLines {
                inner: stdin.lock(),
                buf: String::with_capacity(128),
            };
            pump(&mut src, &mut pipe, &mut out);
        }
        Source::Serial => {
            let port = args.port.clone().unwrap_or_else(|| "/dev/ttyAMA0".into());
            let mut src = open_serial(&port, args.baud)?;
            pump(&mut src, &mut pipe, &mut out);
        }
        Source::Gpio => {
            let mut src = GpioLines::open(&args.host, args.pigpio_port, args.gpio, args.baud)?;
            pump(&mut src, &mut pipe, &mut out);
        }
        Source::Sim => {
            let mut src = SimLines {
                lat: args.lat,
                lon: args.lon,
                speed: args.sim_speed,
                step: 0,
            };
            pump(&mut src, &mut pipe, &mut out);
        }
    }
    Ok(())
}

/// Shared hot loop: bytes in -> validated, gated, throttled fixes out.
fn pump(src: &mut impl Lines, pipe: &mut Pipe, out: &mut impl Write) {
    while let Some(line) = src.next_line() {
        // Fast reject: only GGA/RMC sentences can yield fixes.
        if !(line.contains("GGA") || line.contains("RMC")) {
            continue;
        }
        if let Some(fix) = parse_line(&line) {
            if pipe.push(fix, out).is_err() {
                break; // stdout closed (game quit) => exit quietly
            }
        }
    }
}
