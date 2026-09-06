//! Hand-rolled NMEA 0183 parser: GGA (+ RMC fallback) sentences only.
//!
//! The Python version used the generic `pynmea2` parser; here we validate the
//! `*HH` checksum FIRST (cheap integer XOR over bytes — rejects corrupt lines
//! before any float work), then parse only the fields we need with zero
//! intermediate allocations.

/// A validated position fix from one NMEA sentence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NmeaFix {
    pub lat: f64,
    pub lon: f64,
    /// NMEA fix quality (GGA field 6). RMC reports 1 when status is 'A'.
    pub quality: u8,
}

/// Parse one NMEA line (`$GPGGA...`, `$GNGGA...`, `$GPRMC...`, `$GNRMC...`).
/// Returns `None` for bad checksums, empty fields, or void/no-fix sentences.
pub fn parse_line(line: &str) -> Option<NmeaFix> {
    let line = line.trim();
    if !line.starts_with('$') {
        return None;
    }
    // Split payload / checksum and verify BEFORE parsing fields.
    let star = line.rfind('*')?;
    let payload = &line[1..star];
    let check: u8 = u8::from_str_radix(line[star + 1..].trim(), 16).ok()?;
    let mut x: u8 = 0;
    for b in payload.bytes() {
        x ^= b;
    }
    if x != check {
        return None;
    }

    let mut f = payload.split(',');
    let kind = f.next()?;
    // Talker id varies (GP/GN/GL...); match the sentence type suffix.
    if kind.len() < 5 {
        return None;
    }
    let suffix = &kind[kind.len() - 3..];
    match suffix {
        "GGA" => parse_gga(&mut f),
        "RMC" => parse_rmc(&mut f),
        _ => None,
    }
}

fn parse_gga<'a>(f: &mut impl Iterator<Item = &'a str>) -> Option<NmeaFix> {
    let _time = f.next()?;
    let lat_s = f.next()?;
    let ns = f.next()?;
    let lon_s = f.next()?;
    let ew = f.next()?;
    let qual_s = f.next()?;
    let quality: u8 = qual_s.parse().ok()?;
    if quality == 0 {
        return None; // no fix — same gate as Python (`gps_qual > 0`)
    }
    let lat = dm_to_deg(lat_s, false)?;
    let lon = dm_to_deg(lon_s, true)?;
    let lat = match ns {
        "N" => lat,
        "S" => -lat,
        _ => return None,
    };
    let lon = match ew {
        "E" => lon,
        "W" => -lon,
        _ => return None,
    };
    Some(NmeaFix { lat, lon, quality })
}

fn parse_rmc<'a>(f: &mut impl Iterator<Item = &'a str>) -> Option<NmeaFix> {
    let _time = f.next()?;
    if f.next()? != "A" {
        return None; // void
    }
    let lat_s = f.next()?;
    let ns = f.next()?;
    let lon_s = f.next()?;
    let ew = f.next()?;
    let lat = dm_to_deg(lat_s, false)?;
    let lon = dm_to_deg(lon_s, true)?;
    let lat = match ns {
        "N" => lat,
        "S" => -lat,
        _ => return None,
    };
    let lon = match ew {
        "E" => lon,
        "W" => -lon,
        _ => return None,
    };
    Some(NmeaFix {
        lat,
        lon,
        quality: 1,
    })
}

/// `ddmm.mmmm` (lat) or `dddmm.mmmm` (lon) -> decimal degrees.
fn dm_to_deg(s: &str, _is_lon: bool) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let dot = s.find('.')?;
    if dot < 3 {
        return None;
    }
    let deg: f64 = s[..dot - 2].parse().ok()?;
    let min: f64 = s[dot - 2..].parse().ok()?;
    if !(0.0..60.0).contains(&min) {
        return None;
    }
    Some(deg + min / 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real-world style GGA (checksum computed for the exact payload).
    fn gga(payload: &str) -> String {
        let mut x: u8 = 0;
        for b in payload.bytes() {
            x ^= b;
        }
        format!("${payload}*{x:02X}")
    }

    #[test]
    fn parses_gga_fix() {
        let s = gga("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,");
        let fix = parse_line(&s).expect("should parse");
        assert!((fix.lat - 48.1173).abs() < 1e-4, "lat={}", fix.lat);
        assert!((fix.lon - 11.5166_666).abs() < 1e-4, "lon={}", fix.lon);
        assert_eq!(fix.quality, 1);
    }

    #[test]
    fn rejects_bad_checksum() {
        assert_eq!(
            parse_line("$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*00"),
            None
        );
    }

    #[test]
    fn rejects_no_fix_quality_zero() {
        let s = gga("GPGGA,123519,4807.038,N,01131.000,E,0,00,99.9,,,,,,");
        assert_eq!(parse_line(&s), None);
    }

    #[test]
    fn parses_rmc_active() {
        let s = gga("GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W");
        let fix = parse_line(&s).expect("should parse RMC");
        assert!((fix.lat - 48.1173).abs() < 1e-4);
    }

    #[test]
    fn rejects_rmc_void() {
        let s = gga("GPRMC,123519,V,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W");
        assert_eq!(parse_line(&s), None);
    }

    #[test]
    fn southern_western_hemisphere() {
        let s = gga("GPGGA,123519,3351.000,S,15112.000,W,1,08,0.9,0.0,M,0.0,M,,");
        let fix = parse_line(&s).expect("should parse");
        assert!(fix.lat < 0.0 && fix.lon < 0.0);
    }
}
