//! Zombie movement. Same chase-with-jitter behavior as Python's
//! `Zombie.step`, minus per-tick allocation and global-RNG contention.

use crate::config::{ZOMBIE_JITTER, ZOMBIE_SPEED_MPS};
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
}
