//! Owned notification state and deterministic expiration, independent of D-Bus.
use std::{collections::BTreeMap, error::Error, fmt, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    External,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub key: String,
    pub label: String,
}
impl Action {
    /// D-Bus sends alternating keys and labels. Discard an incomplete final
    /// pair, as the compatibility parser does, while owning every string.
    pub fn from_pairs(values: impl IntoIterator<Item = String>) -> Vec<Self> {
        let mut values = values.into_iter();
        let mut actions = Vec::new();
        while let (Some(key), Some(label)) = (values.next(), values.next()) {
            actions.push(Self { key, label });
        }
        actions
    }
}

/// An owned, validated 8-bit RGB or RGBA image. The last row does not need
/// trailing padding; pixels never borrow the incoming D-Bus message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rowstride: u32,
    has_alpha: bool,
    data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    InvalidDimensions,
    InvalidStride,
    UnsupportedFormat,
    SizeOverflow,
    Truncated { required: usize, actual: usize },
}
impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("Image dimensions must be positive"),
            Self::InvalidStride => f.write_str("Image row stride is too small"),
            Self::UnsupportedFormat => f.write_str("Image must contain 8-bit RGB or RGBA pixels"),
            Self::SizeOverflow => f.write_str("Image layout exceeds addressable memory"),
            Self::Truncated { required, actual } => {
                write!(f, "Image needs {required} bytes but contains {actual}")
            }
        }
    }
}
impl Error for ImageError {}

impl Image {
    pub fn new(
        width: i32,
        height: i32,
        rowstride: i32,
        has_alpha: bool,
        bits_per_sample: i32,
        channels: i32,
        mut data: Vec<u8>,
    ) -> Result<Self, ImageError> {
        if width <= 0 || height <= 0 {
            return Err(ImageError::InvalidDimensions);
        }
        if rowstride <= 0 {
            return Err(ImageError::InvalidStride);
        }
        if bits_per_sample != 8 || channels != if has_alpha { 4 } else { 3 } {
            return Err(ImageError::UnsupportedFormat);
        }
        let row_bytes = (width as usize)
            .checked_mul(channels as usize)
            .ok_or(ImageError::SizeOverflow)?;
        if (rowstride as usize) < row_bytes {
            return Err(ImageError::InvalidStride);
        }
        let required = (height as usize - 1)
            .checked_mul(rowstride as usize)
            .and_then(|size| size.checked_add(row_bytes))
            .ok_or(ImageError::SizeOverflow)?;
        if data.len() < required {
            return Err(ImageError::Truncated {
                required,
                actual: data.len(),
            });
        }
        data.truncate(required);
        Ok(Self {
            width: width as u32,
            height: height as u32,
            rowstride: rowstride as u32,
            has_alpha,
            data,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn rowstride(&self) -> u32 {
        self.rowstride
    }
    pub fn has_alpha(&self) -> bool {
        self.has_alpha
    }
    pub fn bits_per_sample(&self) -> u32 {
        8
    }
    pub fn channels(&self) -> u32 {
        if self.has_alpha { 4 } else { 3 }
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hints {
    pub category: Option<String>,
    pub desktop_entry: Option<String>,
    pub image_path: Option<String>,
    pub image: Option<Image>,
    pub action_icons: bool,
    pub resident: bool,
    pub transient: bool,
    /// Preserve the original byte and its existing default of zero.
    pub urgency: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationRequest {
    pub app_name: String,
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<Action>,
    pub hints: Hints,
    /// Positive milliseconds expire. Nonpositive values preserve the existing
    /// notification history, including the protocol's default value of -1.
    pub expire_timeout: i32,
    pub origin: Origin,
}
impl Default for NotificationRequest {
    fn default() -> Self {
        Self {
            app_name: String::new(),
            replaces_id: 0,
            app_icon: String::new(),
            summary: String::new(),
            body: String::new(),
            actions: Vec::new(),
            hints: Hints::default(),
            expire_timeout: -1,
            origin: Origin::External,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub id: u32,
    pub request: NotificationRequest,
    /// Wall-clock microseconds supplied by the caller for presentation. A
    /// replacement has the new content's timestamp; expiration is monotonic.
    pub created_on_us: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyResult {
    pub id: u32,
    pub index: usize,
    pub replaced: Option<Notification>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdsExhausted;
impl fmt::Display for IdsExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("All nonzero notification IDs are in use")
    }
}
impl Error for IdsExhausted {}

/// Insertion-ordered notification history with one current deadline per ID.
/// Callers can schedule `next_expiration` on their own main loop and then pass
/// the current monotonic time to `expire`; stale callbacks cannot close a
/// replacement whose current deadline moved or was cancelled.
pub struct NotificationStore {
    notifications: Vec<Notification>,
    deadlines: BTreeMap<u32, Duration>,
    next_id: u32,
}
impl Default for NotificationStore {
    fn default() -> Self {
        Self {
            notifications: Vec::new(),
            deadlines: BTreeMap::new(),
            next_id: 1,
        }
    }
}
impl NotificationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notifications(&self) -> &[Notification] {
        &self.notifications
    }

    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.notifications.iter().find(|item| item.id == id)
    }

    pub fn action(&self, id: u32, key: &str) -> Option<&Action> {
        let notification = self.get(id)?;
        if notification.request.origin == Origin::Internal {
            return None;
        }
        notification
            .request
            .actions
            .iter()
            .find(|action| action.key == key)
    }

    pub fn notify(
        &mut self,
        request: NotificationRequest,
        now: Duration,
        created_on_us: i64,
    ) -> Result<NotifyResult, IdsExhausted> {
        let index = self
            .notifications
            .iter()
            .position(|item| item.id == request.replaces_id);
        let id = match index {
            Some(index) => self.notifications[index].id,
            None => self.allocate_id()?,
        };
        self.deadlines.remove(&id);
        if request.expire_timeout > 0 {
            let timeout = Duration::from_millis(request.expire_timeout as u64);
            self.deadlines.insert(id, now.saturating_add(timeout));
        }
        let notification = Notification {
            id,
            request,
            created_on_us,
        };
        let (index, replaced) = if let Some(index) = index {
            (
                index,
                Some(std::mem::replace(
                    &mut self.notifications[index],
                    notification,
                )),
            )
        } else {
            let index = self.notifications.len();
            self.notifications.push(notification);
            (index, None)
        };
        Ok(NotifyResult {
            id,
            index,
            replaced,
        })
    }

    fn allocate_id(&mut self) -> Result<u32, IdsExhausted> {
        let start = self.next_id;
        loop {
            let candidate = self.next_id;
            self.next_id = candidate.wrapping_add(1).max(1);
            if self.get(candidate).is_none() {
                return Ok(candidate);
            }
            if self.next_id == start {
                return Err(IdsExhausted);
            }
        }
    }

    /// Remove a record in place, preserving the relative order of survivors.
    pub fn close(&mut self, id: u32) -> Option<Notification> {
        let index = self.notifications.iter().position(|item| item.id == id)?;
        self.deadlines.remove(&id);
        Some(self.notifications.remove(index))
    }

    pub fn next_expiration(&self) -> Option<Duration> {
        self.deadlines.values().min().copied()
    }

    /// Select the next due record without removing it. Services can close one
    /// item, notify observers, and then recheck so a reentrant replacement or
    /// dismissal is applied before choosing another expired record.
    pub fn next_expired(&self, now: Duration) -> Option<u32> {
        self.notifications
            .iter()
            .find(|item| {
                self.deadlines
                    .get(&item.id)
                    .is_some_and(|deadline| *deadline <= now)
            })
            .map(|item| item.id)
    }

    /// Return expired records in inventory order. Their owned data remains
    /// available to callers while emitting removal and close notifications.
    pub fn expire(&mut self, now: Duration) -> Vec<Notification> {
        let expired = self
            .notifications
            .iter()
            .filter(|item| {
                self.deadlines
                    .get(&item.id)
                    .is_some_and(|deadline| *deadline <= now)
            })
            .map(|item| item.id)
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|id| self.close(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn request(summary: &str, replaces_id: u32, timeout: i32) -> NotificationRequest {
        NotificationRequest {
            app_name: "Fixture".into(),
            summary: summary.into(),
            replaces_id,
            expire_timeout: timeout,
            ..Default::default()
        }
    }

    #[test]
    fn additions_and_replacements_keep_owned_records_and_inventory_position() {
        let mut store = NotificationStore::default();
        let added = store
            .notify(request("First", 0, -1), Duration::ZERO, 50)
            .unwrap();
        assert_ne!(added.id, 0);
        assert_eq!(added.index, 0);
        assert!(added.replaced.is_none());
        let original = store.get(added.id).unwrap().clone();
        let other = store
            .notify(request("Second", 0, 0), Duration::ZERO, 60)
            .unwrap();
        assert_ne!(added.id, other.id);
        let replacement = store
            .notify(request("Updated", added.id, -1), Duration::from_secs(1), 70)
            .unwrap();
        assert_eq!(replacement.id, added.id);
        assert_eq!(replacement.index, 0);
        assert_eq!(replacement.replaced, Some(original.clone()));
        assert_eq!(store.notifications().len(), 2);
        assert_eq!(store.notifications()[1].id, other.id);
        assert_eq!(store.get(added.id).unwrap().request.summary, "Updated");
        assert_eq!(store.get(added.id).unwrap().created_on_us, 70);
        assert_eq!(original.request.summary, "First");
        assert_eq!(original.created_on_us, 50);
    }

    #[test]
    fn unknown_replacement_allocates_a_fresh_id_and_wrap_skips_live_ids() {
        let mut store = NotificationStore::default();
        let first = store
            .notify(request("First", 0, -1), Duration::ZERO, 0)
            .unwrap();
        let missing = store
            .notify(request("Unknown replacement", 500, -1), Duration::ZERO, 0)
            .unwrap();
        assert_ne!(missing.id, 0);
        assert_ne!(missing.id, 500);
        assert_ne!(missing.id, first.id);
        assert!(missing.replaced.is_none());
        store.next_id = u32::MAX;
        let last = store
            .notify(request("Last ID", 0, -1), Duration::ZERO, 0)
            .unwrap();
        assert_eq!(last.id, u32::MAX);
        let wrapped = store
            .notify(request("Wrapped", 0, -1), Duration::ZERO, 0)
            .unwrap();
        assert_eq!(wrapped.id, 3);
        assert_eq!(
            store
                .notifications()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [1, 2, u32::MAX, 3]
        );
    }

    #[test]
    fn controlled_expiration_is_inclusive_and_keeps_persistent_history() {
        let mut store = NotificationStore::default();
        let start = Duration::from_secs(10);
        let zero = store.notify(request("Persistent", 0, 0), start, 0).unwrap();
        let default = store.notify(request("Default", 0, -1), start, 0).unwrap();
        let timed = store.notify(request("Timed", 0, 25), start, 0).unwrap();
        let later = store.notify(request("Later", 0, 50), start, 0).unwrap();
        assert_eq!(
            store.next_expiration(),
            Some(start + Duration::from_millis(25))
        );
        assert!(store.expire(start + Duration::from_millis(24)).is_empty());
        let expired = store.expire(start + Duration::from_millis(25));
        assert_eq!(
            expired.iter().map(|item| item.id).collect::<Vec<_>>(),
            [timed.id]
        );
        assert!(store.get(timed.id).is_none());
        assert_eq!(expired[0].request.summary, "Timed");
        assert_eq!(
            store.next_expiration(),
            Some(start + Duration::from_millis(50))
        );
        assert_eq!(store.expire(Duration::MAX)[0].id, later.id);
        assert_eq!(
            store
                .notifications()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [zero.id, default.id]
        );
        assert_eq!(store.next_expiration(), None);
    }

    #[test]
    fn replacement_reschedules_or_cancels_the_previous_expiration() {
        let mut store = NotificationStore::default();
        let id = store
            .notify(request("Old", 0, 100), Duration::ZERO, 0)
            .unwrap()
            .id;
        store
            .notify(request("New", id, 200), Duration::from_millis(50), 1)
            .unwrap();
        assert!(store.expire(Duration::from_millis(100)).is_empty());
        assert_eq!(store.next_expiration(), Some(Duration::from_millis(250)));
        store
            .notify(request("History", id, -1), Duration::from_millis(150), 2)
            .unwrap();
        assert!(store.expire(Duration::MAX).is_empty());
        assert_eq!(store.next_expiration(), None);
        assert_eq!(store.get(id).unwrap().request.summary, "History");
        store
            .notify(request("Expiring again", id, 1), Duration::from_secs(1), 3)
            .unwrap();
        assert_eq!(
            store.expire(Duration::from_millis(1001))[0].request.summary,
            "Expiring again"
        );
    }

    #[test]
    fn explicit_close_returns_owned_state_and_cancels_expiration() {
        let mut store = NotificationStore::default();
        let first = store
            .notify(request("First", 0, 10), Duration::ZERO, 0)
            .unwrap();
        let second = store
            .notify(request("Second", 0, -1), Duration::ZERO, 0)
            .unwrap();
        let third = store
            .notify(request("Third", 0, -1), Duration::ZERO, 0)
            .unwrap();
        let closed = store.close(first.id).unwrap();
        assert_eq!(closed.request.summary, "First");
        assert!(store.close(first.id).is_none());
        assert!(store.close(0).is_none());
        assert!(store.expire(Duration::MAX).is_empty());
        assert_eq!(
            store
                .notifications()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [second.id, third.id]
        );
    }

    #[test]
    fn incremental_expiration_rechecks_reentrant_replacements_and_closes() {
        let mut store = NotificationStore::new();
        let first = store
            .notify(request("First", 0, 10), Duration::ZERO, 0)
            .unwrap()
            .id;
        let second = store
            .notify(request("Second", 0, 10), Duration::ZERO, 0)
            .unwrap()
            .id;
        let third = store
            .notify(request("Third", 0, 10), Duration::ZERO, 0)
            .unwrap()
            .id;
        let now = Duration::from_millis(10);
        assert_eq!(store.next_expired(now), Some(first));
        store.close(first);
        // These changes model observers reacting to the first removal.
        store
            .notify(request("Replacement", second, 20), now, 1)
            .unwrap();
        store.close(third);
        assert_eq!(store.next_expired(now), None);
        assert_eq!(store.get(second).unwrap().request.summary, "Replacement");
        assert_eq!(store.next_expired(Duration::from_millis(30)), Some(second));
    }

    #[test]
    fn actions_are_complete_owned_pairs_and_internal_actions_are_suppressed() {
        let mut source = vec![
            "default".into(),
            "Open".into(),
            "reply".into(),
            "Reply".into(),
            "unpaired".into(),
        ];
        let actions = Action::from_pairs(source.clone());
        source[1] = "Changed source".into();
        assert_eq!(
            actions,
            [
                Action {
                    key: "default".into(),
                    label: "Open".into()
                },
                Action {
                    key: "reply".into(),
                    label: "Reply".into()
                }
            ]
        );
        let mut store = NotificationStore::default();
        let mut external = request("External", 0, -1);
        external.actions = actions.clone();
        let external = store.notify(external, Duration::ZERO, 0).unwrap();
        assert_eq!(store.action(external.id, "reply").unwrap().label, "Reply");
        assert!(store.action(external.id, "missing").is_none());
        assert!(store.action(99, "default").is_none());
        let mut internal = request("Internal", 0, -1);
        internal.origin = Origin::Internal;
        internal.actions = actions;
        let internal = store.notify(internal, Duration::ZERO, 0).unwrap();
        assert!(store.action(internal.id, "default").is_none());
        assert_eq!(
            store.close(internal.id).unwrap().request.origin,
            Origin::Internal
        );
        store.close(external.id);
        assert!(store.action(external.id, "reply").is_none());
    }

    #[test]
    fn expiration_overflow_stays_pending_until_the_maximum_instant() {
        let mut store = NotificationStore::default();
        let id = store
            .notify(
                request("Overflow", 0, 2),
                Duration::MAX - Duration::from_millis(1),
                i64::MAX,
            )
            .unwrap()
            .id;
        assert_eq!(store.next_expiration(), Some(Duration::MAX));
        assert!(
            store
                .expire(Duration::MAX - Duration::from_nanos(1))
                .is_empty()
        );
        assert_eq!(store.expire(Duration::MAX)[0].id, id);
    }

    #[test]
    fn validated_images_own_pixels_and_allow_missing_final_row_padding() {
        let pixels = (0..14).collect::<Vec<u8>>();
        let image = Image::new(2, 2, 8, false, 8, 3, pixels.clone()).unwrap();
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 2);
        assert_eq!(image.rowstride(), 8);
        assert!(!image.has_alpha());
        assert_eq!(image.bits_per_sample(), 8);
        assert_eq!(image.channels(), 3);
        assert_eq!(image.data(), pixels);
        let rgba = Image::new(2, 2, 8, true, 8, 4, vec![255; 20]).unwrap();
        assert!(rgba.has_alpha());
        assert_eq!(rgba.channels(), 4);
        assert_eq!(rgba.data().len(), 16);
        assert_eq!(
            Image::new(1, 1, 1024, false, 8, 3, vec![1, 2, 3])
                .unwrap()
                .data(),
            [1, 2, 3]
        );
    }

    #[test]
    fn malformed_image_dimensions_stride_format_and_length_are_rejected() {
        for (width, height, stride, alpha, bits, channels, length) in [
            (-1, 2, 6, false, 8, 3, 16),
            (2, 0, 6, false, 8, 3, 16),
            (0, 2, 6, false, 8, 3, 16),
            (2, 2, -6, false, 8, 3, 16),
            (2, 2, 5, false, 8, 3, 16),
            (2, 2, 6, false, 16, 3, 16),
            (2, 2, 8, true, 8, 3, 16),
            (2, 2, 8, false, 8, 4, 16),
            (2, 2, 8, true, 8, 4, 15),
            (2, 2, 8, false, 8, 3, 13),
            (i32::MAX, i32::MAX, i32::MAX, true, 8, 4, 16),
            (1, i32::MAX, i32::MAX, false, 8, 3, 16),
        ] {
            assert!(
                Image::new(
                    width,
                    height,
                    stride,
                    alpha,
                    bits,
                    channels,
                    vec![0; length]
                )
                .is_err(),
                "accepted invalid image {width}x{height}, stride {stride}"
            );
        }
    }
}
