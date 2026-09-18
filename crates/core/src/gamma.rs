//! Gamma ramps using the existing gammastep whitepoint table and interpolation.
use crate::whitepoints::WHITEPOINTS;

/// Limit compositor-supplied ramp sizes to six MiB of channel data.
pub const MAX_RAMP_SIZE: usize = 1 << 20;

pub fn is_supported(size: usize, temperature: u32) -> bool {
    size > 0 && size <= MAX_RAMP_SIZE && (1000..=25000).contains(&temperature)
}

pub fn apply(red: &mut [u16], green: &mut [u16], blue: &mut [u16], temperature: u32) -> bool {
    if red.is_empty()
        || red.len() > MAX_RAMP_SIZE
        || green.len() != red.len()
        || blue.len() != red.len()
        || !(1000..=25000).contains(&temperature)
    {
        return false;
    }
    let index = ((temperature - 1000) / 100) as usize;
    let alpha = (f64::from(temperature % 100) / 100.0) as f32;
    for (channel, ramp) in [red, green, blue].into_iter().enumerate() {
        let white = ((1.0 - f64::from(alpha)) * f64::from(WHITEPOINTS[index][channel])
            + f64::from(alpha * WHITEPOINTS[index + 1][channel])) as f32;
        for value in ramp {
            *value = (f64::from(*value) * f64::from(white)) as u16;
        }
    }
    true
}

pub fn ramp(size: usize, temperature: u32) -> Option<Vec<u16>> {
    if size == 0 || size > MAX_RAMP_SIZE || !(1000..=25000).contains(&temperature) {
        return None;
    }
    let total = size.checked_mul(3)?;
    let mut ramp = Vec::new();
    ramp.try_reserve_exact(total).ok()?;
    for i in 0..total {
        ramp.push((((i % size) as f64 / size as f64) * 65536.0) as u16);
    }
    let (red, other) = ramp.split_at_mut(size);
    let (green, blue) = other.split_at_mut(size);
    if !apply(red, green, blue, temperature) {
        return None;
    }
    Some(ramp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_gamma_golden_values() {
        for line in include_str!("../../../tests/fixtures/gamma.tsv").lines() {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let expected: Vec<u16> = values[1..].iter().map(|v| *v as u16).collect();
            assert_eq!(ramp(4, values[0]).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_invalid_dimensions_and_temperatures() {
        assert!(ramp(0, 6500).is_none());
        assert!(ramp(256, 999).is_none());
        assert!(ramp(256, 25001).is_none());
        assert!(ramp(usize::MAX, 6500).is_none());
        assert_eq!(ramp(1, 25000).unwrap(), [0, 0, 0]);
    }
}
