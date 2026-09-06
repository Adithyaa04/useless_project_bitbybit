//! zdeck — shared library = the COMMON INTERFACE between the binaries.
//!
//! * `zdeck-gps` (preprocessor) owns everything hardware-specific: serial
//!   ports, pigpiod bit-bang GPIO, NMEA quirks. It emits normalized fixes.
//! * `zdeck-game` (main app) only consumes [`proto::GpsFix`] NDJSON lines.
//! * `zdeck-fetch` produces the offline `map_data.json` both understand.
//!
//! Swapping GPS hardware (different module, baud, GPIO pin, or even a vendor
//! binary) means replacing ONLY the `zdeck-gps` binary, as long as it keeps
//! printing one [`proto::GpsFix`] JSON object per line on stdout.
//!
//! Performance notes (vs the original Python):
//! * map lat/lon -> meters projection happens ONCE at load, stored flat;
//! * per-frame work is integer screen math + viewport culling, no allocation
//!   in the hot loop (render buffers are reused);
//! * NMEA parsing is a hand-rolled GGA/RMC byte parser (no generic parser,
//!   checksum-gated before any float work);
//! * zombie RNG is an inline xorshift (no global RNG, no GIL).

pub mod config;
pub mod geo;
pub mod map;
pub mod nmea;
pub mod proto;
pub mod rng;
pub mod zombie;
