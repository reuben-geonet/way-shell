//! MPRIS cards are an owned, stable prefix of the notification list.
use super::notification_card::{CardLayout, CardMode, app_icon_for_id, load_avatar};
use crate::services::media::{MediaAction, MediaPlayer, MediaService, PlaybackStatus};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn play_icon(status: &PlaybackStatus) -> &'static str {
    if *status == PlaybackStatus::Playing {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    }
}
fn can_command(player: &MediaPlayer, action: MediaAction) -> bool {
    let caps = &player.capabilities;
    if action == MediaAction::Raise {
        return caps.can_raise;
    }
    caps.can_control
        && match action {
            MediaAction::Previous => caps.can_go_previous,
            MediaAction::Next => caps.can_go_next,
            MediaAction::PlayPause if player.playback_status == PlaybackStatus::Playing => {
                caps.can_pause
            }
            MediaAction::PlayPause | MediaAction::Play => caps.can_play,
            MediaAction::Pause => caps.can_pause,
            MediaAction::Stop => true,
            MediaAction::Raise => unreachable!(),
        }
}

pub struct MediaCards {
    service: MediaService,
    cards: RefCell<Vec<Rc<MediaCard>>>,
    handler: Cell<Option<glib::SignalHandlerId>>,
    changed: Box<dyn Fn(Vec<gtk::Widget>)>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    closed: Cell<bool>,
}
impl MediaCards {
    pub fn new(service: MediaService, changed: impl Fn(Vec<gtk::Widget>) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            service,
            cards: RefCell::new(Vec::new()),
            handler: Cell::new(None),
            changed: Box::new(changed),
            updating: Cell::new(false),
            pending: Cell::new(false),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.handler.set(Some(this.service.connect_local(
            "changed",
            false,
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.refresh();
                }
                None
            },
        )));
        this.refresh();
        this
    }
    pub fn widgets(&self) -> Vec<gtk::Widget> {
        self.cards
            .borrow()
            .iter()
            .map(|card| card.layout.root.clone().upcast())
            .collect()
    }
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(handler) = self.handler.take() {
            self.service.disconnect(handler);
        }
        let cards = self.cards.take();
        for card in cards {
            card.close();
        }
        (self.changed)(Vec::new());
    }
    fn refresh(self: &Rc<Self>) {
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        while self.pending.replace(false) && !self.closed.get() {
            let state = self.service.state();
            let previous = self.widgets();
            let old = self.cards.borrow().clone();
            let mut cards = Vec::new();
            for card in old {
                if state.available && state.players.iter().any(|player| card.matches(player)) {
                    cards.push(card);
                } else {
                    card.close();
                }
            }
            // Disabling a removed card emits GTK notifications. An observer may
            // close the whole controller there; never restore the saved cards.
            if self.closed.get() {
                break;
            }
            // Existing players retain their position. Newly discovered players
            // are prepended as they were in the original notification list.
            if state.available {
                for player in &state.players {
                    if !cards.iter().any(|card| card.matches(player)) {
                        cards.insert(0, MediaCard::new(self, player));
                    }
                }
            }
            self.cards.replace(cards.clone());
            for card in cards {
                if self.closed.get() {
                    break;
                }
                if let Some(player) = state.players.iter().find(|player| card.matches(player)) {
                    card.update(player);
                }
            }
            let current = self.widgets();
            if !self.closed.get() && previous != current {
                (self.changed)(current);
            }
        }
        self.updating.set(false);
    }
}
impl Drop for MediaCards {
    fn drop(&mut self) {
        self.close();
    }
}

struct MediaCard {
    name: String,
    owner: String,
    player: RefCell<Option<MediaPlayer>>,
    layout: CardLayout,
    previous: gtk::Button,
    play_pause: gtk::Button,
    next: gtk::Button,
    artwork: RefCell<Option<glib::JoinHandle<()>>>,
    generation: Cell<u64>,
    closed: Cell<bool>,
}
impl MediaCard {
    fn new(owner: &Rc<MediaCards>, player: &MediaPlayer) -> Rc<Self> {
        let layout = CardLayout::new(CardMode::Tray);
        layout.root.add_css_class("media-player");
        layout.dismiss.set_visible(false);
        layout.expand.set_visible(false);
        layout.timer_label.set_visible(false);
        layout.action_revealer.set_visible(false);
        for label in [&layout.summary, &layout.body] {
            label.set_size_request(-1, -1);
            label.set_width_chars(34);
            label.set_max_width_chars(34);
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        buttons.add_css_class("notification-widget-media-buttons-container");
        buttons.set_halign(gtk::Align::End);
        let previous = gtk::Button::from_icon_name("go-previous-symbolic");
        let play_pause = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        let next = gtk::Button::from_icon_name("go-next-symbolic");
        for button in [&previous, &play_pause, &next] {
            button.add_css_class("notification-widget-media-button");
            buttons.append(button);
        }
        layout.button_container.append(&buttons);
        let this = Rc::new(Self {
            name: player.name.clone(),
            owner: player.owner.clone(),
            player: RefCell::new(None),
            layout,
            previous,
            play_pause,
            next,
            artwork: RefCell::new(None),
            generation: Cell::new(0),
            closed: Cell::new(false),
        });
        for (button, action) in [
            (&this.layout.button, MediaAction::Raise),
            (&this.previous, MediaAction::Previous),
            (&this.play_pause, MediaAction::PlayPause),
            (&this.next, MediaAction::Next),
        ] {
            let weak = Rc::downgrade(&this);
            let owner_weak = Rc::downgrade(owner);
            button.connect_clicked(move |button| {
                let (Some(this), Some(owner)) = (weak.upgrade(), owner_weak.upgrade()) else {
                    return;
                };
                if this.closed.get() || owner.closed.get() {
                    return;
                }
                let state = owner.service.state();
                let Some(player) = state.players.iter().find(|player| this.matches(player)) else {
                    return;
                };
                if !can_command(player, action) {
                    return;
                }
                button.set_tooltip_text(None);
                let weak = Rc::downgrade(&this);
                let button = button.downgrade();
                owner.service.command(&this.name, action, move |result| {
                    if let Some(this) = weak.upgrade().filter(|this| !this.closed.get())
                        && let Some(button) = button.upgrade()
                        && let Err(error) = result
                    {
                        button.set_tooltip_text(Some(&error.to_string()));
                        glib::g_message!(
                            "way-shell",
                            "Media command for {} failed: {error}",
                            this.name
                        );
                    }
                });
            });
        }
        this
    }
    fn matches(&self, player: &MediaPlayer) -> bool {
        self.name == player.name && self.owner == player.owner
    }
    fn update(self: &Rc<Self>, player: &MediaPlayer) {
        if self.closed.get() || self.player.borrow().as_ref() == Some(player) {
            return;
        }
        self.player.replace(Some(player.clone()));
        self.layout
            .app_name
            .set_label(player.identity.as_deref().unwrap_or(""));
        self.layout.header_icon.clear();
        if let Some(icon) = player.identity.as_deref().and_then(app_icon_for_id) {
            self.layout.header_icon.set_from_gicon(&icon);
        }
        self.layout
            .summary
            .set_label(&player.metadata.artist().unwrap_or_default());
        self.layout
            .body
            .set_label(player.metadata.title.as_deref().unwrap_or(""));
        self.play_pause
            .set_icon_name(play_icon(&player.playback_status));
        for (button, action) in [
            (&self.layout.button, MediaAction::Raise),
            (&self.previous, MediaAction::Previous),
            (&self.play_pause, MediaAction::PlayPause),
            (&self.next, MediaAction::Next),
        ] {
            button.set_sensitive(can_command(player, action));
        }
        self.cancel_artwork();
        self.layout
            .avatar
            .set_custom_image(None::<&gtk::gdk::Paintable>);
        if self.closed.get() {
            return;
        }
        if let Some(location) = player
            .metadata
            .art_url
            .as_ref()
            .filter(|location| !location.is_empty())
        {
            let location = location.clone();
            let weak = Rc::downgrade(self);
            let generation = self.generation.get();
            let task = glib::MainContext::ref_thread_default().spawn_local(async move {
                let result = load_avatar(&location).await;
                if let Some(this) = weak
                    .upgrade()
                    .filter(|this| !this.closed.get() && this.generation.get() == generation)
                {
                    this.artwork.borrow_mut().take();
                    match result {
                        Ok(image) => this.layout.avatar.set_custom_image(Some(&image)),
                        Err(error) if !error.matches(gio::IOErrorEnum::Cancelled) => {
                            glib::g_debug!("way-shell", "Could not load media artwork: {error}")
                        }
                        Err(_) => {}
                    }
                }
            });
            self.artwork.replace(Some(task));
        }
    }
    fn cancel_artwork(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        let task = self.artwork.borrow_mut().take();
        if let Some(task) = task {
            task.abort();
        }
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.cancel_artwork();
        self.layout.root.set_sensitive(false);
    }
}
impl Drop for MediaCard {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::media::{Capabilities, Metadata};
    #[test]
    fn transport_icons_and_capabilities_follow_playback_state() {
        let mut player = MediaPlayer {
            name: "org.mpris.MediaPlayer2.test".into(),
            owner: ":1.42".into(),
            identity: None,
            playback_status: PlaybackStatus::Playing,
            metadata: Metadata::default(),
            capabilities: Capabilities {
                can_control: true,
                can_play: false,
                can_pause: true,
                can_go_next: true,
                can_go_previous: false,
                can_raise: true,
            },
        };
        assert_eq!(
            play_icon(&player.playback_status),
            "media-playback-pause-symbolic"
        );
        assert!(can_command(&player, MediaAction::PlayPause));
        assert!(can_command(&player, MediaAction::Next));
        assert!(!can_command(&player, MediaAction::Previous));
        assert!(can_command(&player, MediaAction::Raise));
        player.playback_status = PlaybackStatus::Paused;
        assert_eq!(
            play_icon(&player.playback_status),
            "media-playback-start-symbolic"
        );
        assert!(!can_command(&player, MediaAction::PlayPause));
        player.capabilities.can_play = true;
        assert!(can_command(&player, MediaAction::PlayPause));
        player.capabilities.can_control = false;
        assert!(!can_command(&player, MediaAction::Next));
    }
}
