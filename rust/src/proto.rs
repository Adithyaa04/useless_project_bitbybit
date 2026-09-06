//! Stable IPC contract between `zdeck-gps` and `zdeck-game`.
//!
//! **The swap rule:** any program can replace `zdeck-gps` — different GPS
//! module, different baud, different GPIO, even a vendor binary in another
//! language — as long as it prints one JSON object per line on stdout with
//! this exact schema:
//!
//! ```json
//! {"lat": 9.9649, "lon": 76.2868, "fix": 1, "t_ms": 1720000000000}
//! ```
//!
//! * `lat`/`lon`: decimal degrees (WGS84).
//! * `fix`: NMEA quality (0 = no fix; game ignores `fix == 0` lines... in
//!   practice `zdeck-gps` only emits `fix > 0`).
//! * `t_ms`: unix epoch millis when the fix was produced.
//!
//! Transport: NDJSON on stdout (or a pipe). No shared memory, no sockets, no
//! version lockstep — the game tolerates unknown extra fields and skips
//! malformed lines.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// One normalized GPS position, ready for the game loop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GpsFix {
    pub lat: f64,
    pub lon: f64,
    pub fix: u8,
    pub t_ms: u64,
}

impl GpsFix {
    pub fn new(lat: f64, lon: f64, fix: u8) -> Self {
        Self {
            lat,
            lon,
            fix,
            t_ms: now_ms(),
        }
    }

    /// Serialize to one NDJSON line (no trailing newline included).
    pub fn encode(&self) -> String {
        // Only fails on OOM; fall back to a degraded line rather than panic.
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"lat":{},"lon":{},"fix":{},"t_ms":{}}}"#,
                self.lat, self.lon, self.fix, self.t_ms
            )
        })
    }

    /// Parse one NDJSON line. `None` = skip (malformed / no-fix / wrong shape).
    pub fn decode(line: &str) -> Option<Self> {
        let f: Self = serde_json::from_str(line.trim()).ok()?;
        if !f.lat.is_finite() || !f.lon.is_finite() || f.fix == 0 {
            return None;
        }
        Some(f)
    }
}

/// Current unix time in millis.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = GpsFix::new(9.9649, 76.2868, 1);
        let line = f.encode();
        let back = GpsFix::decode(&line).expect("roundtrip");
        assert_eq!((back.lat, back.lon, back.fix), (f.lat, f.lon, f.fix));
    }

    #[test]
    fn rejects_junk_and_no_fix() {
        assert_eq!(GpsFix::decode("hello"), None);
        assert_eq!(
            GpsFix::decode(r#"{"lat":1.0,"lon":2.0,"fix":0,"t_ms":3}"#),
            None
        );
    }

    #[test]
    fn tolerates_extra_fields() {
        // Forward-compat: a future gps binary may add fields.
        let line = r#"{"lat":1.0,"lon":2.0,"fix":1,"t_ms":3,"sats":9}"#;
        assert!(GpsFix::decode(line).is_some());
    }
}
