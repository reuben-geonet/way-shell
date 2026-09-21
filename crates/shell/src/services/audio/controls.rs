use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    Unavailable,
    NodeNotFound(u32),
    VolumeUnavailable(u32),
    InvalidVolume,
    InvalidDelta,
    Rejected(u32),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("audio service is unavailable"),
            Self::NodeNotFound(id) => write!(f, "audio node {id} is no longer available"),
            Self::VolumeUnavailable(id) => write!(f, "audio node {id} has no volume control"),
            Self::InvalidVolume => f.write_str("volume must be finite and between 0.0 and 1.0"),
            Self::InvalidDelta => f.write_str("volume adjustment must be finite"),
            Self::Rejected(id) => write!(f, "audio node {id} rejected the volume operation"),
        }
    }
}

impl std::error::Error for AudioError {}

impl AudioService {
    /// Set displayed volume in the same 0..=1 range accepted by the CLI.
    /// Success means the mixer accepted the request; changes arrive in the state subscription.
    pub fn set_volume(&self, id: u32, volume: f64) -> Result<(), AudioError> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(AudioError::InvalidVolume);
        }
        let (mixer, _) = self.volume_control(id)?;
        mixer.set_volume(id, volume)
    }

    /// Set mute without changing channel volumes or saving a stale volume for unmute.
    pub fn set_muted(&self, id: u32, muted: bool) -> Result<(), AudioError> {
        let (mixer, _) = self.volume_control(id)?;
        mixer.set_muted(id, muted)
    }

    /// Adjust the observed displayed volume, preserving amplification set by other mixers.
    /// Increases stop at full volume; decreases can reduce an amplified level gradually.
    pub fn change_volume(&self, id: u32, delta: f64) -> Result<(), AudioError> {
        if !delta.is_finite() {
            return Err(AudioError::InvalidDelta);
        }
        let (mixer, observed) = self.volume_control(id)?;
        if delta == 0.0 || (delta > 0.0 && observed.volume >= 1.0) {
            return Ok(());
        }
        let volume = if delta > 0.0 {
            (observed.volume + delta).min(1.0)
        } else {
            (observed.volume + delta).max(0.0)
        };
        mixer.set_volume(id, volume)
    }

    fn volume_control(&self, id: u32) -> Result<(native::Mixer, Volume), AudioError> {
        let volume = {
            let state = self.imp().state.borrow();
            if !state.available {
                return Err(AudioError::Unavailable);
            }
            state
                .node(id)
                .ok_or(AudioError::NodeNotFound(id))?
                .volume
                .clone()
                .ok_or(AudioError::VolumeUnavailable(id))?
        };
        // Retain only the plugin across the call. Native signals may synchronously refresh
        // state, so neither state nor session RefCells may be borrowed during emission.
        let mixer = self
            .imp()
            .session
            .borrow()
            .as_ref()
            .and_then(native::Session::mixer)
            .ok_or(AudioError::Unavailable)?;
        Ok((mixer, volume))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_node_without_mixer_values_disables_controls() {
        // Inventory discovery can precede the first mixer property update.
        let service: AudioService = glib::Object::new();
        service.publish(AudioState {
            available: true,
            nodes: vec![AudioNode {
                id: 12,
                serial: 34,
                kind: NodeKind::Sink,
                name: "pending-output".into(),
                description: String::new(),
                nickname: String::new(),
                application: String::new(),
                media: String::new(),
                state: NodeState::Creating,
                volume: None,
            }],
            ..AudioState::default()
        });
        assert_eq!(
            service.set_volume(12, 0.5),
            Err(AudioError::VolumeUnavailable(12))
        );
        assert_eq!(
            service.set_muted(12, true),
            Err(AudioError::VolumeUnavailable(12))
        );
        assert_eq!(
            service.change_volume(12, 0.05),
            Err(AudioError::VolumeUnavailable(12))
        );
    }
}
