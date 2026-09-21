//! Application-owned Wayland state; no protocol objects escape this module's service.
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Eq, PartialEq, glib::Boxed)]
#[boxed_type(name = "WayShellOutput")]
pub struct Output {
    pub id: u32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub make: String,
    pub model: String,
    pub initialized: bool,
    pub scale: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, glib::Boxed)]
#[boxed_type(name = "WayShellSeat")]
pub struct Seat {
    pub id: u32,
    pub name: Option<String>,
    pub capabilities: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, glib::Boxed)]
#[boxed_type(name = "WayShellToplevel")]
pub struct Toplevel {
    pub id: u64,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub outputs: BTreeSet<u32>,
    pub parent: Option<u64>,
    pub maximized: bool,
    pub minimized: bool,
    pub active: bool,
    pub fullscreen: bool,
    /// Compatibility event flags for the just-completed protocol batch.
    pub activation_event: bool,
    pub entered_event: bool,
}

impl Toplevel {
    pub(super) fn apply_state(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if !bytes.len().is_multiple_of(4) {
            return Err("Foreign toplevel state is not an array of 32-bit values");
        }
        self.maximized = false;
        self.minimized = false;
        self.active = false;
        self.fullscreen = false;
        for value in bytes.as_chunks::<4>().0 {
            match u32::from_ne_bytes(*value) {
                0 => self.maximized = true,
                1 => self.minimized = true,
                2 => self.active = true,
                3 => self.fullscreen = true,
                _ => {}
            }
        }
        self.activation_event = self.active;
        Ok(())
    }

    pub(super) fn visible(
        &self,
        ignored_apps: &BTreeSet<String>,
        ignored_titles: &BTreeSet<String>,
    ) -> bool {
        self.app_id
            .as_ref()
            .is_some_and(|id| !ignored_apps.contains(id))
            && self
                .title
                .as_ref()
                .is_some_and(|title| !ignored_titles.contains(title))
    }
}

pub(super) fn ignored(value: &str) -> BTreeSet<String> {
    if value.is_empty() {
        BTreeSet::new()
    } else {
        value.split(':').map(String::from).collect()
    }
}

pub(super) fn version(advertised: u32, maximum: u32) -> Option<u32> {
    (advertised > 0).then_some(advertised.min(maximum))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_counted_as_u32_values_and_unknown_states_are_ignored() {
        let mut top = Toplevel::default();
        top.apply_state(&2_u32.to_ne_bytes()).unwrap();
        assert!(top.active && top.activation_event);
        assert!(!top.maximized);
        let bytes: Vec<u8> = [0_u32, 1, 3, u32::MAX]
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect();
        top.apply_state(&bytes).unwrap();
        assert!(top.maximized && top.minimized && top.fullscreen);
        assert!(!top.active && !top.activation_event);
        top.apply_state(&[]).unwrap();
        assert!(!top.maximized && !top.minimized && !top.fullscreen);
    }

    #[test]
    fn malformed_state_does_not_partially_replace_valid_state() {
        let mut top = Toplevel {
            active: true,
            ..Default::default()
        };
        assert!(top.apply_state(&[2, 0, 0]).is_err());
        assert!(top.active);
    }

    #[test]
    fn advertised_versions_are_capped_and_zero_is_rejected() {
        assert_eq!(version(0, 4), None);
        assert_eq!(version(1, 4), Some(1));
        assert_eq!(version(99, 4), Some(4));
        assert_eq!(version(u32::MAX, 1), Some(1));
    }

    #[test]
    fn incomplete_and_ignored_toplevels_are_filtered_by_exact_name() {
        let apps = ignored("hidden:org.example.Ignore");
        let titles = ignored("Secrets:ignored 日本語");
        let mut top = Toplevel::default();
        assert!(!top.visible(&apps, &titles));
        top.app_id = Some("org.example.Visible".into());
        top.title = Some("Visible 日本語".into());
        assert!(top.visible(&apps, &titles));
        top.title = Some("Secrets".into());
        assert!(!top.visible(&apps, &titles));
        top.title = Some("Other".into());
        top.app_id = Some("hidden".into());
        assert!(!top.visible(&apps, &titles));
        assert!(top.visible(&ignored(""), &ignored("")));
    }
}
