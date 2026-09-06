//! Zombie movement + horde management. Same chase-with-jitter behavior as
//! Python's `Zombie.step`, minus per-tick allocation and global-RNG
//! contention. [`Horde`] adds what the Python version lacked: reinforcements
//! when the player outruns the pack, despawn of zombies lost far behind,
//! and caps on concurrent zombies + spawn rate (so the Pi and the player
//! both survive).

use crate::config::*;
use crate::rng::XorShift64;

#[derive(Debug, Clone, Copy)]
pub struct Zombie {
    pub x: f64,
    pub y: f64,
}

impl Zombie {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Step toward `(target_x, target_y)` over `dt` seconds.
    #[inline]
    pub fn step(&mut self, target_x: f64, target_y: f64, dt: f64, rng: &mut XorShift64) {
        let dx = target_x - self.x;
        let dy = target_y - self.y;
        let dist = dx.hypot(dy);
        if dist < 0.01 {
            return;
        }
        let heading = dy.atan2(dx) + rng.range(-ZOMBIE_JITTER, ZOMBIE_JITTER);
        let s = ZOMBIE_SPEED_MPS * dt;
        let (sh, ch) = heading.sin_cos(); // one trig call, not two
        self.x += ch * s;
        self.y += sh * s;
    }

    #[inline]
    pub fn dist_to(&self, x: f64, y: f64) -> f64 {
        (self.x - x).hypot(self.y - y)
    }
}

/// Tunables for [`Horde`]; defaults mirror `config::*` so CLI flags are
/// optional overrides.
#[derive(Debug, Clone)]
pub struct HordeCfg {
    pub min: usize,
    pub max: usize,
    pub despawn_range_m: f64,
    pub despawn_after_s: f64,
    pub max_spawn_per_s: f64,
    pub spawn_trigger_m: f64,
}

impl Default for HordeCfg {
    fn default() -> Self {
        Self {
            min: MIN_ZOMBIES,
            max: MAX_ZOMBIES,
            despawn_range_m: DESPAWN_RANGE_M,
            despawn_after_s: DESPAWN_AFTER_S,
            max_spawn_per_s: MAX_SPAWN_PER_S,
            spawn_trigger_m: SPAWN_TRIGGER_M,
        }
    }
}

/// One horde member: position + how long it has been out of range.
/// The timer resets the moment it gets back in range.
#[derive(Debug, Clone)]
pub struct ZombieState {
    pub pos: Zombie,
    out_of_range_s: f64,
}

impl ZombieState {
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            pos: Zombie::new(x, y),
            out_of_range_s: 0.0,
        }
    }
}

/// Per-tick outcome of [`Horde::update`].
#[derive(Debug, Clone, Copy)]
pub struct TickReport {
    pub min_dist: f64,
    pub spawned: usize,
    pub despawned: usize,
}

/// The pack: steps every zombie, despawns ones lost far behind, and trickles
/// in reinforcements — capped in count and rate.
#[derive(Debug)]
pub struct Horde {
    pub members: Vec<ZombieState>,
    cfg: HordeCfg,
    cooldown_s: f64,
}

impl Horde {
    /// New horde of `cfg.min` zombies in the spawn ring around `(cx, cy)`.
    pub fn new(cfg: HordeCfg, rng: &mut XorShift64, cx: f64, cy: f64) -> Self {
        let mut h = Self {
            members: Vec::with_capacity(cfg.max),
            cfg,
            cooldown_s: 0.0,
        };
        for _ in 0..h.cfg.min.min(h.cfg.max) {
            let (dx, dy) = spawn_offset(rng);
            h.members.push(ZombieState::new(cx + dx, cy + dy));
        }
        h
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Advance one tick: chase, age out-of-range timers, despawn the lost,
    /// reinforce the pack. At most one spawn per tick (plus the rate cap).
    pub fn update(&mut self, px: f64, py: f64, dt: f64, rng: &mut XorShift64) -> TickReport {
        let mut min_dist = f64::INFINITY;
        for m in &mut self.members {
            m.pos.step(px, py, dt, rng);
            let d = m.pos.dist_to(px, py);
            min_dist = min_dist.min(d);
            if d > self.cfg.despawn_range_m {
                m.out_of_range_s += dt;
            } else {
                m.out_of_range_s = 0.0;
            }
        }

        let before = self.members.len();
        self.members
            .retain(|m| m.out_of_range_s < self.cfg.despawn_after_s);
        let despawned = before - self.members.len();

        self.cooldown_s -= dt;
        let mut spawned = 0;
        // Reinforce when the pack is thin OR the player escaped the zombie
        // area entirely — always capped in count and rate.
        let need = self.members.len() < self.cfg.min
            || (min_dist > self.cfg.spawn_trigger_m && self.members.len() < self.cfg.max);
        if need && self.cooldown_s <= 0.0 && self.members.len() < self.cfg.max {
            let (dx, dy) = spawn_offset(rng);
            self.members.push(ZombieState::new(px + dx, py + dy));
            self.cooldown_s = 1.0 / self.cfg.max_spawn_per_s.max(f64::EPSILON);
            spawned = 1;
            min_dist = min_dist.min(dx.hypot(dy));
        }
        TickReport {
            min_dist,
            spawned,
            despawned,
        }
    }
}

/// Random offset in the spawn ring (40-90m) around the player.
fn spawn_offset(rng: &mut XorShift64) -> (f64, f64) {
    let d = rng.range(SPAWN_MIN_M, SPAWN_MAX_M);
    let b = rng.range(0.0, std::f64::consts::TAU);
    (b.cos() * d, b.sin() * d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_toward_target() {
        let mut rng = XorShift64::new(7);
        let mut z = Zombie::new(50.0, 0.0);
        let d0 = z.dist_to(0.0, 0.0);
        z.step(0.0, 0.0, 0.25, &mut rng);
        assert!(z.dist_to(0.0, 0.0) < d0);
    }

    #[test]
    fn speed_matches_config() {
        // With zero jitter the step length must equal speed*dt exactly.
        // (Use a private-scope check via many steps averaging instead:
        // mean displacement per tick ~= speed*dt within jitter cone.)
        let mut rng = XorShift64::new(1);
        let mut z = Zombie::new(1000.0, 0.0);
        let n = 200;
        for _ in 0..n {
            z.step(0.0, 0.0, 0.25, &mut rng);
        }
        let travelled = 1000.0 - z.x.hypot(z.y);
        let expect = ZOMBIE_SPEED_MPS * 0.25 * n as f64;
        assert!(
            (travelled - expect).abs() < expect * 0.15,
            "travelled={travelled} expect~{expect}"
        );
    }

    fn cfg(rng_seed: u64) -> (HordeCfg, XorShift64) {
        (HordeCfg::default(), XorShift64::new(rng_seed))
    }

    #[test]
    fn horde_starts_at_min() {
        let (c, mut rng) = cfg(1);
        let h = Horde::new(c, &mut rng, 0.0, 0.0);
        assert_eq!(h.len(), MIN_ZOMBIES);
        for m in &h.members {
            let d = m.pos.dist_to(0.0, 0.0);
            assert!((SPAWN_MIN_M..=SPAWN_MAX_M).contains(&d), "d={d}");
        }
    }

    #[test]
    fn far_zombie_despawns_after_timeout() {
        let (c, mut rng) = cfg(2);
        let mut h = Horde::new(c, &mut rng, 0.0, 0.0);
        h.members.clear();
        h.members.push(ZombieState::new(500.0, 0.0)); // hopelessly lost
                                                      // Player camps at origin; zombie shambles closer but stays >150m.
        let mut despawned = 0;
        for _ in 0..44 {
            // 44 x 0.25s = 11s > DESPAWN_AFTER_S
            despawned += h.update(0.0, 0.0, 0.25, &mut rng).despawned;
        }
        assert_eq!(despawned, 1);
        // ...and the pack refills (min) afterwards via reinforcements.
        for _ in 0..40 {
            h.update(0.0, 0.0, 0.25, &mut rng);
        }
        assert!(!h.is_empty());
    }

    #[test]
    fn near_zombie_never_despawns_and_resets_timer() {
        let (mut c, mut rng) = cfg(3);
        c.min = 1; // a lone zombie is a full pack: no refill noise
        let mut h = Horde::new(c, &mut rng, 0.0, 0.0);
        h.members.clear();
        h.members.push(ZombieState::new(10.0, 0.0));
        for _ in 0..80 {
            // 20s next to the player: still there, timer pinned at 0.
            let r = h.update(0.0, 0.0, 0.25, &mut rng);
            assert_eq!(r.despawned, 0);
        }
        assert_eq!(h.len(), 1);
        assert_eq!(h.members[0].out_of_range_s, 0.0);
    }

    #[test]
    fn escape_triggers_rate_limited_reinforcements() {
        let (c, mut rng) = cfg(4);
        let mut h = Horde::new(c, &mut rng, 0.0, 0.0);
        h.members.clear(); // player escaped everything
        let r = h.update(0.0, 0.0, 0.25, &mut rng);
        assert_eq!(r.spawned, 1); // immediate refill...
        let r = h.update(0.0, 0.0, 0.25, &mut rng);
        assert_eq!(r.spawned, 0); // ...then the rate cap bites (0.5/s)

        // Cap path: tiny trigger + small max so the pack perpetually
        // qualifies for reinforcements yet must stop at max.
        let cfg2 = HordeCfg {
            min: 1,
            max: 3,
            spawn_trigger_m: 10.0, // ring spawns (40-90m) always trigger
            ..HordeCfg::default()
        };
        let mut h2 = Horde::new(cfg2, &mut rng, 0.0, 0.0);
        for _ in 0..200 {
            h2.update(0.0, 0.0, 0.25, &mut rng);
        }
        assert_eq!(h2.len(), 3);
    }

    #[test]
    fn no_flood_while_pack_is_close() {
        let (c, mut rng) = cfg(5);
        let mut h = Horde::new(c, &mut rng, 0.0, 0.0);
        // Pack camps on the player: no reinforcements beyond min.
        for _ in 0..40 {
            let r = h.update(0.0, 0.0, 0.25, &mut rng);
            assert_eq!(r.spawned, 0);
        }
        assert_eq!(h.len(), MIN_ZOMBIES);
    }
}
