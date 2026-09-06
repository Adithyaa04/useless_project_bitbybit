//! Tunables. Same values / semantics as the Python originals
//! (`app/cyberdeck.py`, `app/zombie_cyberdeck.py`).

/// Meters represented by one terminal character cell.
pub const SCALE_M_PER_CELL: f64 = 3.0;
/// Zombies spawned per run.
pub const ZOMBIE_COUNT: usize = 6;
/// Zombie walking speed, m/s.
pub const ZOMBIE_SPEED_MPS: f64 = 1.1;
/// Heading randomness (radians) so zombie paths aren't dead straight.
pub const ZOMBIE_JITTER: f64 = 0.35;
/// Distance at which a zombie "gets" you, meters.
pub const CATCH_RADIUS_M: f64 = 2.5;
/// Game update rate, Hz.
pub const TICK_HZ: u64 = 4;
/// Zombie spawn ring around the player, meters.
pub const SPAWN_MIN_M: f64 = 40.0;
/// Zombie spawn ring around the player, meters.
pub const SPAWN_MAX_M: f64 = 90.0;

/// Max interpolation steps per road segment (perf safety cap).
pub const MAP_LINE_STEP_CAP: i32 = 100;
/// How close you must be for a POI name to show in the status bar, meters.
pub const POI_CALLOUT_RADIUS_M: f64 = 30.0;

/// Player is a bit faster than a zombie (sim mode), as a speed multiplier.
pub const PLAYER_SPEED_MULT: f64 = 1.6;
