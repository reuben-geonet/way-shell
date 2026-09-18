//! Audio calculations; WirePlumber binding types stay in the shell service.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Linear,
    Cubic,
}

pub fn from_linear(volume: f32, scale: Scale) -> f64 {
    if volume <= 0.0 {
        0.0
    } else if scale == Scale::Cubic {
        f64::from(volume).cbrt()
    } else {
        f64::from(volume)
    }
}

pub fn to_linear(volume: f64, scale: Scale) -> f32 {
    if volume <= 0.0 {
        0.0
    } else if scale == Scale::Cubic {
        (volume * volume * volume) as f32
    } else {
        volume as f32
    }
}

pub const CHANNELS: [&str; 12] = [
    "RL", "RR", "FL", "FR", "C", "LFE", "SL", "SR", "RHL", "RHR", "TFL", "TFR",
];

pub fn channel_index(channel: &str) -> Option<usize> {
    CHANNELS.iter().position(|name| *name == channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_matches_c_characterization() {
        assert_eq!(from_linear(-1.0, Scale::Cubic), 0.0);
        assert_eq!(to_linear(-1.0, Scale::Cubic), 0.0);
        assert_eq!(from_linear(0.125, Scale::Cubic), 0.5);
        assert_eq!(to_linear(0.5, Scale::Cubic), 0.125);
        assert_eq!(from_linear(0.5, Scale::Linear), 0.5);
        assert_eq!(to_linear(0.5, Scale::Linear), 0.5);
        for i in 0..=100 {
            let value = f64::from(i) / 100.0;
            assert!(
                (from_linear(to_linear(value, Scale::Cubic), Scale::Cubic) - value).abs() < 1e-6
            );
        }
    }

    #[test]
    fn channel_mapping_matches_c_layout() {
        for (index, channel) in CHANNELS.into_iter().enumerate() {
            assert_eq!(channel_index(channel), Some(index));
        }
        assert_eq!(channel_index("MONO"), None);
        assert_eq!(channel_index(""), None);
    }
}
