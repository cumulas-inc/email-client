//! Delays between reconnection attempts: exponential with full jitter, so
//! many clients cut off at once (a server restart) don't all come back at
//! the same moment.

use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// The ceiling of the first delay; it doubles with every failed attempt.
const BASE: Duration = Duration::from_secs(1);
/// The highest ceiling.
const CAP: Duration = Duration::from_secs(300);
/// The shortest delay, so a server that drops every connection at once
/// isn't hammered.
const FLOOR: Duration = Duration::from_millis(250);
/// A connection that stays up this long counts as healthy: the next failure
/// starts again from [`BASE`].
pub const STABLE: Duration = Duration::from_secs(120);

pub struct Backoff {
    attempt: u32,
    rng: StdRng,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            rng: StdRng::from_os_rng(),
        }
    }

    /// With reproducible jitter, for tests.
    pub fn seeded(seed: u64) -> Self {
        Self {
            attempt: 0,
            rng: StdRng::seed_from_u64(seed),
        }
    }

    /// The upper bound of the next delay.
    pub fn ceiling(&self) -> Duration {
        // 2^9 s already exceeds the cap.
        (BASE * 2u32.pow(self.attempt.min(9))).min(CAP)
    }

    /// The delay before the next attempt: random up to the ceiling, which
    /// then doubles.
    pub fn next_delay(&mut self) -> Duration {
        let ceiling = self.ceiling();
        self.attempt = self.attempt.saturating_add(1);
        let millis = self.rng.random_range(0..=ceiling.as_millis() as u64);
        Duration::from_millis(millis).max(FLOOR)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// A connection ended after `up_for`; one that was stable resets the
    /// backoff.
    pub fn connection_ended(&mut self, up_for: Duration) {
        if up_for >= STABLE {
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_stay_under_a_doubling_ceiling_up_to_the_cap() {
        let mut backoff = Backoff::seeded(7);
        let mut ceilings = Vec::new();
        for _ in 0..12 {
            let ceiling = backoff.ceiling();
            let delay = backoff.next_delay();
            assert!(delay >= FLOOR && delay <= ceiling.max(FLOOR), "{delay:?}");
            ceilings.push(ceiling.as_secs());
        }
        assert_eq!(ceilings, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
    }

    #[test]
    fn jitter_is_reproducible_with_a_seed_and_spread_out() {
        let delays = |seed| {
            let mut backoff = Backoff::seeded(seed);
            (0..8).map(|_| backoff.next_delay()).collect::<Vec<_>>()
        };
        assert_eq!(delays(1), delays(1));
        assert_ne!(delays(1), delays(2));

        // Full jitter: delays at the cap are spread over the whole range.
        let mut backoff = Backoff::seeded(3);
        for _ in 0..10 {
            backoff.next_delay();
        }
        let samples: Vec<Duration> = (0..200).map(|_| backoff.next_delay()).collect();
        assert!(samples.iter().any(|d| *d < CAP / 4));
        assert!(samples.iter().any(|d| *d > CAP * 3 / 4));
        assert!(samples.iter().all(|d| *d <= CAP));
    }

    #[test]
    fn resets_after_a_stable_connection_only() {
        let mut backoff = Backoff::seeded(5);
        for _ in 0..5 {
            backoff.next_delay();
        }
        assert_eq!(backoff.ceiling(), Duration::from_secs(32));
        backoff.connection_ended(STABLE - Duration::from_secs(1));
        assert_eq!(backoff.ceiling(), Duration::from_secs(32));
        backoff.connection_ended(STABLE);
        assert_eq!(backoff.ceiling(), BASE);
        backoff.next_delay();
        backoff.reset();
        assert_eq!(backoff.ceiling(), BASE);
    }
}
