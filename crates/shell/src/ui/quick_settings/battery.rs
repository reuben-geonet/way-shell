//! Battery presentation and independently testable warning/recharge state.
use crate::services::power::{DeviceKind, PowerDevice};
use way_shell_core::notifications::{Hints, NotificationRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    Fatal,
    Critical,
    Low,
}
impl Warning {
    pub fn request(self, percentage: f64) -> NotificationRequest {
        NotificationRequest {
            app_name: "Way-Shell".into(),
            app_icon: match self {
                Self::Fatal => "battery-level-0-symbolic",
                Self::Critical => "battery-level-10-symbolic",
                Self::Low => "battery-level-20-symbolic",
            }
            .into(),
            summary: if self == Self::Fatal {
                "Battery is critically low"
            } else {
                "Battery is low"
            }
            .into(),
            body: format!("Battery is at {percentage:.0}% power."),
            hints: Hints {
                urgency: if self == Self::Low { 0 } else { 2 },
                ..Hints::default()
            },
            ..NotificationRequest::default()
        }
    }
}

#[derive(Clone, Default)]
pub struct WarningTracker {
    device: Option<(String, String)>,
    sent: [bool; 3],
}
impl WarningTracker {
    /// Failed delivery leaves the warning pending. Recovery re-arms each threshold.
    pub fn next(&mut self, device: Option<&PowerDevice>, available: bool) -> Option<Warning> {
        let key = device.map(|device| (device.path.clone(), device.serial.clone()));
        if self.device != key {
            self.device = key;
            self.sent = [false; 3];
        }
        let device = device?;
        let percentage = device
            .percentage
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))?;
        for (sent, threshold) in self.sent.iter_mut().zip([5.0, 15.0, 20.0]) {
            if percentage > threshold {
                *sent = false;
            }
        }
        if !available || !device.rechargeable || device.state.is_charging() {
            return None;
        }
        [Warning::Fatal, Warning::Critical, Warning::Low]
            .into_iter()
            .zip([5.0, 15.0, 20.0])
            .zip(self.sent)
            .find_map(|((warning, threshold), sent)| {
                (percentage <= threshold && !sent).then_some(warning)
            })
    }
    pub fn acknowledge(&mut self, warning: Warning) {
        // A severe warning already tells the user about every crossed threshold.
        let start = match warning {
            Warning::Fatal => 0,
            Warning::Critical => 1,
            Warning::Low => 2,
        };
        self.sent[start..].fill(true);
    }
}

pub struct BatteryPresentation {
    pub button: String,
    pub percentage: String,
    pub time: String,
    pub value: f64,
    pub icon: &'static str,
    pub available: bool,
}
impl BatteryPresentation {
    pub fn from_device(device: Option<&PowerDevice>) -> Self {
        let Some(device) = device else {
            return Self {
                button: "—".into(),
                percentage: "—".into(),
                time: "Power information is unavailable".into(),
                value: 0.0,
                icon: "battery-missing-symbolic",
                available: false,
            };
        };
        let icon = device.preferred_icon_name();
        let mut result = Self {
            button: if device.kind == DeviceKind::Battery {
                "—"
            } else {
                "AC"
            }
            .into(),
            percentage: String::new(),
            time: "AC power".into(),
            value: 100.0,
            icon,
            available: true,
        };
        if !device.rechargeable {
            return result;
        }
        let Some(value) = device
            .percentage
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        else {
            result.time = "Battery percentage is unavailable".into();
            result.percentage = "—".into();
            result.value = 0.0;
            return result;
        };
        result.percentage = format!("{value:.0}%");
        if device.kind == DeviceKind::Battery {
            result.button.clone_from(&result.percentage);
        }
        result.value = value;
        let seconds = if device.state.is_charging() {
            device.time_to_full
        } else {
            device.time_to_empty
        }
        .max(0);
        let minutes = seconds / 60 % 60;
        let hours = seconds / 3600;
        let suffix = if device.state.is_charging() {
            "Until Full"
        } else {
            "Remaining"
        };
        result.time = if hours > 0 {
            format!("{hours} Hours {minutes} Minutes {suffix}")
        } else {
            format!("{minutes} Minutes {suffix}")
        };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::power::DeviceState;
    fn device(percentage: f64) -> PowerDevice {
        PowerDevice {
            path: "/battery/1".into(),
            native_path: String::new(),
            vendor: String::new(),
            model: String::new(),
            serial: "one".into(),
            kind: DeviceKind::Battery,
            state: DeviceState::Discharging,
            power_supply: true,
            present: true,
            rechargeable: true,
            online: false,
            percentage: Some(percentage),
            time_to_empty: 3660,
            time_to_full: 0,
            icon_name: String::new(),
        }
    }
    fn send(tracker: &mut WarningTracker, device: &PowerDevice) -> Option<Warning> {
        let warning = tracker.next(Some(device), true);
        if let Some(warning) = warning {
            tracker.acknowledge(warning);
        }
        warning
    }
    #[test]
    fn one_warning_at_five_then_recharge_rearms_each_threshold() {
        let mut tracker = WarningTracker::default();
        assert_eq!(send(&mut tracker, &device(5.0)), Some(Warning::Fatal));
        for _ in 0..5 {
            assert_eq!(send(&mut tracker, &device(5.0)), None);
        }
        assert_eq!(send(&mut tracker, &device(10.0)), None);
        assert_eq!(send(&mut tracker, &device(5.0)), Some(Warning::Fatal));
        assert_eq!(send(&mut tracker, &device(18.0)), None);
        assert_eq!(send(&mut tracker, &device(15.0)), Some(Warning::Critical));
        assert_eq!(send(&mut tracker, &device(21.0)), None);
        assert_eq!(send(&mut tracker, &device(20.0)), Some(Warning::Low));
        assert_eq!(send(&mut tracker, &device(15.0)), Some(Warning::Critical));
        assert_eq!(send(&mut tracker, &device(5.0)), Some(Warning::Fatal));
    }
    #[test]
    fn charging_missing_delivery_and_replacement_preserve_pending_warning() {
        let mut tracker = WarningTracker::default();
        let mut battery = device(5.0);
        battery.state = DeviceState::Charging;
        assert_eq!(send(&mut tracker, &battery), None);
        battery.state = DeviceState::Discharging;
        assert_eq!(tracker.next(Some(&battery), false), None);
        assert_eq!(tracker.next(Some(&battery), true), Some(Warning::Fatal));
        assert_eq!(send(&mut tracker, &battery), Some(Warning::Fatal));
        battery.serial = "replacement".into();
        assert_eq!(send(&mut tracker, &battery), Some(Warning::Fatal));
        assert_eq!(tracker.next(None, true), None);
        assert_eq!(send(&mut tracker, &battery), Some(Warning::Fatal));
        battery.percentage = Some(f64::NAN);
        assert_eq!(send(&mut tracker, &battery), None);
    }
    #[test]
    fn presentation_keeps_time_labels_and_handles_missing_percentage() {
        let mut battery = device(99.6);
        let display = BatteryPresentation::from_device(Some(&battery));
        assert_eq!(display.button, "100%");
        assert_eq!(display.time, "1 Hours 1 Minutes Remaining");
        battery.state = DeviceState::Charging;
        battery.time_to_full = 120;
        assert_eq!(
            BatteryPresentation::from_device(Some(&battery)).time,
            "2 Minutes Until Full"
        );
        battery.percentage = None;
        assert_eq!(
            BatteryPresentation::from_device(Some(&battery)).percentage,
            "—"
        );
        assert!(!BatteryPresentation::from_device(None).available);
    }
}
