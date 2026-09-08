//! `zdeck-cardkb` — M5Stack CardKB driver (the Rust replacement for
//! `cardkb/cardkb_keyboard.py`, no Python needed).
//!
//! The CardKB sends fully-formed ASCII over I2C at fixed address `0x5F`.
//! This binary polls it and re-emits each byte as a Linux input event via
//! `/dev/uinput`, so the whole OS sees a real `CardKB-Virtual-Keyboard`.
//!
//! ```text
//!   CardKB --(I2C 0x5F)--> [ zdeck-cardkb ] --(uinput)--> OS / game
//! ```
//!
//! Needs: `/dev/i2c-1` (`dtparam=i2c_arm=on`), `/dev/uinput`
//! (`modprobe uinput` + udev rule, user in `input` group).
//! Run: `zdeck-cardkb`, test by typing in any editor, verify with
//! `cat /proc/bus/input/devices | grep -A5 CardKB`.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use evdev::{AttributeSet, BusType, InputEvent, InputId, KeyCode, KeyEvent};
use evdev::uinput::VirtualDevice;
use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;

#[derive(Parser, Debug)]
#[command(
    name = "zdeck-cardkb",
    about = "M5Stack CardKB I2C driver: 0x5F -> virtual USB keyboard (uinput)"
)]
struct Args {
    /// I2C bus device, e.g. /dev/i2c-1
    #[arg(long, default_value = "/dev/i2c-1")]
    bus: String,
    /// CardKB I2C address (hex 0x5F or decimal 95)
    #[arg(long, default_value = "0x5F", value_parser = parse_addr)]
    addr: u16,
    /// Poll interval in milliseconds
    #[arg(long, default_value_t = 30)]
    poll_ms: u64,
    /// Name of the virtual keyboard device
    #[arg(long, default_value = "CardKB-Virtual-Keyboard")]
    device_name: String,
}

fn parse_addr(s: &str) -> Result<u16, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).map_err(|e| format!("bad hex address {s:?}: {e}"))
    } else if s.chars().all(|c| c.is_ascii_digit()) {
        s.parse::<u16>().map_err(|e| format!("bad address {s:?}: {e}"))
    } else {
        u16::from_str_radix(s, 16).map_err(|e| format!("bad hex address {s:?}: {e}"))
    }
}

/// Map a CardKB byte to (Linux key code, needs_shift).
/// Mirrors `cardkb/cardkb_keyboard.py` (CardKB manual key code tables).
fn map_byte(b: u8) -> Option<(KeyCode, bool)> {
    Some(match b {
        // --- special non-ASCII codes from the CardKB manual ---
        0x08 => (KeyCode::KEY_BACKSPACE, false),
        0x09 => (KeyCode::KEY_TAB, false),
        0x0D => (KeyCode::KEY_ENTER, false),
        0x1B => (KeyCode::KEY_ESC, false),
        0xB4 => (KeyCode::KEY_LEFT, false),
        0xB5 => (KeyCode::KEY_UP, false),
        0xB6 => (KeyCode::KEY_DOWN, false),
        0xB7 => (KeyCode::KEY_RIGHT, false),
        0xB8 => (KeyCode::KEY_DOWN, false), // alias seen on some fw revisions
        0xB9 => (KeyCode::KEY_RIGHT, false), // alias seen on some fw revisions
        // --- printable ASCII ---
        b' ' => (KeyCode::KEY_SPACE, false),
        b'0' => (KeyCode::KEY_0, false),
        b'1' => (KeyCode::KEY_1, false),
        b'2' => (KeyCode::KEY_2, false),
        b'3' => (KeyCode::KEY_3, false),
        b'4' => (KeyCode::KEY_4, false),
        b'5' => (KeyCode::KEY_5, false),
        b'6' => (KeyCode::KEY_6, false),
        b'7' => (KeyCode::KEY_7, false),
        b'8' => (KeyCode::KEY_8, false),
        b'9' => (KeyCode::KEY_9, false),
        b'a' | b'A' => (KeyCode::KEY_A, b.is_ascii_uppercase()),
        b'b' | b'B' => (KeyCode::KEY_B, b.is_ascii_uppercase()),
        b'c' | b'C' => (KeyCode::KEY_C, b.is_ascii_uppercase()),
        b'd' | b'D' => (KeyCode::KEY_D, b.is_ascii_uppercase()),
        b'e' | b'E' => (KeyCode::KEY_E, b.is_ascii_uppercase()),
        b'f' | b'F' => (KeyCode::KEY_F, b.is_ascii_uppercase()),
        b'g' | b'G' => (KeyCode::KEY_G, b.is_ascii_uppercase()),
        b'h' | b'H' => (KeyCode::KEY_H, b.is_ascii_uppercase()),
        b'i' | b'I' => (KeyCode::KEY_I, b.is_ascii_uppercase()),
        b'j' | b'J' => (KeyCode::KEY_J, b.is_ascii_uppercase()),
        b'k' | b'K' => (KeyCode::KEY_K, b.is_ascii_uppercase()),
        b'l' | b'L' => (KeyCode::KEY_L, b.is_ascii_uppercase()),
        b'm' | b'M' => (KeyCode::KEY_M, b.is_ascii_uppercase()),
        b'n' | b'N' => (KeyCode::KEY_N, b.is_ascii_uppercase()),
        b'o' | b'O' => (KeyCode::KEY_O, b.is_ascii_uppercase()),
        b'p' | b'P' => (KeyCode::KEY_P, b.is_ascii_uppercase()),
        b'q' | b'Q' => (KeyCode::KEY_Q, b.is_ascii_uppercase()),
        b'r' | b'R' => (KeyCode::KEY_R, b.is_ascii_uppercase()),
        b's' | b'S' => (KeyCode::KEY_S, b.is_ascii_uppercase()),
        b't' | b'T' => (KeyCode::KEY_T, b.is_ascii_uppercase()),
        b'u' | b'U' => (KeyCode::KEY_U, b.is_ascii_uppercase()),
        b'v' | b'V' => (KeyCode::KEY_V, b.is_ascii_uppercase()),
        b'w' | b'W' => (KeyCode::KEY_W, b.is_ascii_uppercase()),
        b'x' | b'X' => (KeyCode::KEY_X, b.is_ascii_uppercase()),
        b'y' | b'Y' => (KeyCode::KEY_Y, b.is_ascii_uppercase()),
        b'z' | b'Z' => (KeyCode::KEY_Z, b.is_ascii_uppercase()),
        b',' => (KeyCode::KEY_COMMA, false),
        b'.' => (KeyCode::KEY_DOT, false),
        b'/' => (KeyCode::KEY_SLASH, false),
        b'\\' => (KeyCode::KEY_BACKSLASH, false),
        b'\'' => (KeyCode::KEY_APOSTROPHE, false),
        b';' => (KeyCode::KEY_SEMICOLON, false),
        b'`' => (KeyCode::KEY_GRAVE, false),
        b'[' => (KeyCode::KEY_LEFTBRACE, false),
        b']' => (KeyCode::KEY_RIGHTBRACE, false),
        b'-' => (KeyCode::KEY_MINUS, false),
        b'=' => (KeyCode::KEY_EQUAL, false),
        b'!' => (KeyCode::KEY_1, true),
        b'@' => (KeyCode::KEY_2, true),
        b'#' => (KeyCode::KEY_3, true),
        b'$' => (KeyCode::KEY_4, true),
        b'%' => (KeyCode::KEY_5, true),
        b'^' => (KeyCode::KEY_6, true),
        b'&' => (KeyCode::KEY_7, true),
        b'*' => (KeyCode::KEY_8, true),
        b'(' => (KeyCode::KEY_9, true),
        b')' => (KeyCode::KEY_0, true),
        b'{' => (KeyCode::KEY_LEFTBRACE, true),
        b'}' => (KeyCode::KEY_RIGHTBRACE, true),
        b'|' => (KeyCode::KEY_BACKSLASH, true),
        b'~' => (KeyCode::KEY_GRAVE, true),
        b'"' => (KeyCode::KEY_APOSTROPHE, true),
        b':' => (KeyCode::KEY_SEMICOLON, true),
        b'+' => (KeyCode::KEY_EQUAL, true),
        b'_' => (KeyCode::KEY_MINUS, true),
        b'?' => (KeyCode::KEY_SLASH, true),
        b'<' => (KeyCode::KEY_COMMA, true),
        b'>' => (KeyCode::KEY_DOT, true),
        _ => return None,
    })
}

/// Every key code the driver can emit (needed to declare the uinput device).
fn all_keys() -> Vec<KeyCode> {
    vec![
        KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_D, KeyCode::KEY_E, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_H, KeyCode::KEY_I, KeyCode::KEY_J,
        KeyCode::KEY_K, KeyCode::KEY_L, KeyCode::KEY_M, KeyCode::KEY_N, KeyCode::KEY_O, KeyCode::KEY_P, KeyCode::KEY_Q, KeyCode::KEY_R, KeyCode::KEY_S, KeyCode::KEY_T,
        KeyCode::KEY_U, KeyCode::KEY_V, KeyCode::KEY_W, KeyCode::KEY_X, KeyCode::KEY_Y, KeyCode::KEY_Z, KeyCode::KEY_SPACE, KeyCode::KEY_0, KeyCode::KEY_1,
        KeyCode::KEY_2, KeyCode::KEY_3, KeyCode::KEY_4, KeyCode::KEY_5, KeyCode::KEY_6, KeyCode::KEY_7, KeyCode::KEY_8, KeyCode::KEY_9, KeyCode::KEY_COMMA,
        KeyCode::KEY_DOT, KeyCode::KEY_SLASH, KeyCode::KEY_BACKSLASH, KeyCode::KEY_APOSTROPHE, KeyCode::KEY_SEMICOLON,
        KeyCode::KEY_GRAVE, KeyCode::KEY_LEFTBRACE, KeyCode::KEY_RIGHTBRACE, KeyCode::KEY_MINUS, KeyCode::KEY_EQUAL,
        KeyCode::KEY_BACKSPACE, KeyCode::KEY_TAB, KeyCode::KEY_ENTER, KeyCode::KEY_ESC, KeyCode::KEY_LEFT, KeyCode::KEY_UP,
        KeyCode::KEY_DOWN, KeyCode::KEY_RIGHT, KeyCode::KEY_LEFTSHIFT,
    ]
}

fn key_ev(key: KeyCode, value: i32) -> InputEvent {
    InputEvent::from(KeyEvent::new(key, value))
}

fn send_key(dev: &mut VirtualDevice, key: KeyCode, shift: bool) -> Result<()> {
    if shift {
        dev.emit(&[key_ev(KeyCode::KEY_LEFTSHIFT, 1), key_ev(key, 1)])?;
        dev.emit(&[key_ev(key, 0), key_ev(KeyCode::KEY_LEFTSHIFT, 0)])?;
    } else {
        dev.emit(&[key_ev(key, 1)])?;
        dev.emit(&[key_ev(key, 0)])?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut i2c = LinuxI2CDevice::new(&args.bus, args.addr)
        .with_context(|| format!("open I2C {} @ {:#04X}", args.bus, args.addr))?;

    let keys: AttributeSet<KeyCode> = all_keys().into_iter().collect();
    let mut dev = VirtualDevice::builder()
        .context("open /dev/uinput (need `modprobe uinput` + input group)")?
        .name(&args.device_name)
        .input_id(InputId::new(BusType::BUS_USB, 0x1209, 0x005F, 1))
        .with_keys(&keys)
        .context("declare uinput keys")?
        .build()
        .context("create virtual keyboard")?;

    eprintln!(
        "zdeck-cardkb: {} on {} @ {:#04X}, poll {}ms. Ctrl+C to quit.",
        args.device_name, args.bus, args.addr, args.poll_ms
    );
    let poll = Duration::from_millis(args.poll_ms.max(1));
    let mut io_errors: u64 = 0;
    loop {
        match i2c.smbus_read_byte() {
            Err(e) => {
                // NACK / bus hiccup: poll again, complain only periodically.
                io_errors += 1;
                if io_errors == 1 || io_errors % 1000 == 0 {
                    eprintln!("zdeck-cardkb: I2C read error (#{io_errors}): {e:#}");
                }
            }
            Ok(0) => {}
            Ok(data) => {
                io_errors = 0;
                match map_byte(data) {
                    Some((key, shift)) => send_key(&mut dev, key, shift)?,
                    None => eprintln!("zdeck-cardkb: unmapped byte 0x{data:02X}"),
                }
            }
        }
        thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn spot_mappings_match_python_driver() {
        assert_eq!(map_byte(b'a'), Some((KeyCode::KEY_A, false)));
        assert_eq!(map_byte(b'A'), Some((KeyCode::KEY_A, true)));
        assert_eq!(map_byte(b'!'), Some((KeyCode::KEY_1, true)));
        assert_eq!(map_byte(b'?'), Some((KeyCode::KEY_SLASH, true)));
        assert_eq!(map_byte(0x0D), Some((KeyCode::KEY_ENTER, false)));
        assert_eq!(map_byte(0xB5), Some((KeyCode::KEY_UP, false)));
        assert_eq!(map_byte(0x00), None);
    }

    #[test]
    fn every_mapped_key_is_declared_on_uinput_device() {
        // A key missing from all_keys() would silently never emit.
        let declared: HashSet<u16> = all_keys().into_iter().map(|k| k.0).collect();
        assert!(declared.contains(&KeyCode::KEY_LEFTSHIFT.0));
        for b in [0x08u8, 0x09, 0x0D, 0x1B, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9] {
            let (k, _) = map_byte(b).unwrap_or_else(|| panic!("special {b:#04X} unmapped"));
            assert!(declared.contains(&k.0), "special {b:#04X} not declared");
        }
        for b in 0x20u8..0x7Fu8 {
            if let Some((k, _)) = map_byte(b) {
                assert!(declared.contains(&k.0), "byte {b:#04X} not declared");
            }
        }
    }

    #[test]
    fn addr_parser_accepts_hex_and_decimal() {
        assert_eq!(parse_addr("0x5F").unwrap(), 0x5F);
        assert_eq!(parse_addr("5F").unwrap(), 0x5F);
        assert_eq!(parse_addr("95").unwrap(), 95);
    }
}
