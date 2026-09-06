//! Tunables. Same values / semantics as the Python originals
//! (`app/cyberdeck.py`, `app/zombie_cyberdeck.py`).

/// Meters represented by one terminal character cell.
pub const SCALE_M_PER_CELL: f64 = 3.0;
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

/// Horde manager: hard cap on concurrent zombies (perf + fairness).
pub const MAX_ZOMBIES: usize = 12;
/// Horde manager: reinforcements spawn while below this count.
pub const MIN_ZOMBIES: usize = 6;
/// A zombie farther than this is "out of range" (past typical view).
pub const DESPAWN_RANGE_M: f64 = 150.0;
/// Out-of-range zombies despawn after this many seconds away.
pub const DESPAWN_AFTER_S: f64 = 10.0;
/// Max reinforcement rate (spawns/second). 0.5 = one every 2s.
pub const MAX_SPAWN_PER_S: f64 = 0.5;
/// Nearest zombie beyond this = player is "out of the zombie area" -> send
/// reinforcements (rate-limited, capped). Must exceed SPAWN_MAX_M so the
/// opening horde doesn't instantly flood to MAX_ZOMBIES.
pub const SPAWN_TRIGGER_M: f64 = 100.0;

/// Max interpolation steps per road segment (perf safety cap).
pub const MAP_LINE_STEP_CAP: i32 = 100;
/// How close you must be for a POI name to show in the status bar, meters.
pub const POI_CALLOUT_RADIUS_M: f64 = 30.0;

/// Player is a bit faster than a zombie (sim mode), as a speed multiplier.
pub const PLAYER_SPEED_MULT: f64 = 1.6;
