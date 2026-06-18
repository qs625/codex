use std::time::Duration;

use rand::Rng;

const INITIAL_DELAY_MS: u64 = 200;
const BACKOFF_FACTOR: f64 = 2.0;

/// Return an exponential retry delay with a small random jitter.
///
/// The first retry window is centered around 200ms and later attempts double.
pub fn backoff(attempt: u64) -> Duration {
    let exp = BACKOFF_FACTOR.powi(attempt.saturating_sub(1) as i32);
    let base = (INITIAL_DELAY_MS as f64 * exp) as u64;
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_preserves_initial_window_for_zero_and_first_attempt() {
        let attempt_zero = backoff(0);
        let attempt_one = backoff(1);

        assert!((180..=220).contains(&attempt_zero.as_millis()));
        assert!((180..=220).contains(&attempt_one.as_millis()));
    }

    #[test]
    fn backoff_doubles_each_attempt_with_jitter_range() {
        let attempt_two = backoff(2);
        let attempt_three = backoff(3);

        assert!((360..=440).contains(&attempt_two.as_millis()));
        assert!((720..=880).contains(&attempt_three.as_millis()));
    }
}
