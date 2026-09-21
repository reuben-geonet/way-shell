use std::time::Duration;

/// Delay to the next wall-clock minute, rounded up to GLib's millisecond timer
/// resolution. Recompute after every tick so late callbacks do not accumulate.
pub fn until_next_minute(unix_microseconds: i64) -> Duration {
    let remaining = 60_000_000 - unix_microseconds.rem_euclid(60_000_000);
    Duration::from_millis((remaining as u64).div_ceil(1000))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minute_boundaries_and_delayed_callbacks() {
        for (time, milliseconds) in [
            (0, 60_000),
            (59_000_000, 1000),
            (59_999_999, 1),
            (60_000_000, 60_000),
            (61_234_000, 58_766),
            (-1, 1),
            (-60_000_000, 60_000),
        ] {
            assert_eq!(until_next_minute(time), Duration::from_millis(milliseconds));
        }
        for time in [i64::MIN, i64::MAX] {
            assert!(
                (Duration::from_millis(1)..=Duration::from_secs(60))
                    .contains(&until_next_minute(time))
            );
        }
    }
}
