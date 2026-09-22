//! Compose quick settings from shared service handles and permanent Rust widgets.
use super::{
    QuickSettingsEvent, QuickSettingsWindow,
    audio::AudioControls,
    bluetooth::BluetoothControls,
    controls::{SystemControls, SystemServices},
    grid::Grid,
    header::{MixerSlot, SystemHeader},
    network::NetworkControls,
    power_menu::Confirmation,
};
use crate::services::{
    audio::AudioService, bluetooth::BluetoothService, network::NetworkService,
    notifications::NotificationsService, power::PowerService,
};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

pub struct QuickSettingsServices {
    pub system: SystemServices,
    pub audio: AudioService,
    pub network: NetworkService,
    pub bluetooth: BluetoothService,
    pub power: PowerService,
    pub notifications: NotificationsService,
}

pub struct QuickSettings {
    window: Rc<QuickSettingsWindow>,
    system: Rc<SystemControls>,
    network: Rc<NetworkControls>,
    bluetooth: Rc<BluetoothControls>,
    audio: Rc<AudioControls>,
    header: Rc<SystemHeader>,
    grid: Rc<Grid>,
    stopped: Cell<bool>,
}
impl QuickSettings {
    pub fn new(
        services: QuickSettingsServices,
        system_settings: gio::Settings,
        confirmation: impl Fn(Confirmation) + 'static,
    ) -> Result<Rc<Self>, String> {
        let window = QuickSettingsWindow::new()?;
        let bluetooth =
            BluetoothControls::new(services.bluetooth, system_settings.clone(), &window);
        let system = SystemControls::new(services.system.clone());
        let network = NetworkControls::new(services.network, &window);
        let audio = AudioControls::new(services.audio, &window);
        let weak = Rc::downgrade(&audio);
        let mixer = MixerSlot {
            button: audio.mixer_button().clone(),
            content: audio.menu().widget().clone().upcast(),
            revealed: Rc::new(move |revealed| {
                if let Some(audio) = weak.upgrade() {
                    audio.set_mixer_revealed(revealed);
                }
            }),
        };
        let focus = Rc::downgrade(&window);
        let confirm = Rc::downgrade(&window);
        let header = SystemHeader::new(
            services.power,
            services.notifications,
            services.system.logind,
            system_settings,
            Some(mixer),
            move |focused| {
                if let Some(window) = focus.upgrade() {
                    window.set_focused(focused);
                    window.shrink();
                }
            },
            move |request| {
                if let Some(window) = confirm.upgrade() {
                    window.hide();
                    confirmation(request);
                } else {
                    (request.respond)(false);
                }
            },
        );
        let weak = Rc::downgrade(&window);
        let grid = Grid::new(move |focused| {
            if let Some(window) = weak.upgrade() {
                window.set_focused(focused);
                window.shrink();
            }
        });
        window.content().append(header.widget());
        let scales = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scales.set_widget_name("quick-settings-scales");
        scales.append(audio.scales());
        scales.append(system.brightness_row());
        window.content().append(&scales);
        window.content().append(grid.widget());
        let this = Rc::new(Self {
            window,
            system,
            network,
            bluetooth,
            audio,
            header,
            grid,
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.bluetooth.on_changed(move || {
            if let Some(this) = weak.upgrade() {
                this.update_tiles();
            }
        });
        let weak = Rc::downgrade(&this);
        this.network.on_changed(move || {
            if let Some(this) = weak.upgrade() {
                this.update_tiles();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window.on_event(move |event| {
            if event == QuickSettingsEvent::Hidden
                && let Some(this) = weak.upgrade()
            {
                this.header.hide_menus();
                this.grid.hide_menus();
                this.network.hide_passwords();
                this.window.set_focused(false);
                this.window.shrink();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window
            .visibility_controller()
            .main()
            .on_closed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.close();
                }
            });
        this.update_tiles();
        Ok(this)
    }
    pub fn window(&self) -> &Rc<QuickSettingsWindow> {
        &self.window
    }
    pub fn close(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.header.stop();
        self.grid.stop();
        self.system.stop();
        self.network.stop();
        self.bluetooth.stop();
        self.audio.close();
        self.window.close();
    }
    fn update_tiles(&self) {
        if !self.stopped.get() {
            self.grid.set_buttons(
                self.system.ordered_buttons(
                    self.network
                        .device_buttons()
                        .into_iter()
                        .chain(self.bluetooth.button())
                        .chain(self.network.vpn_button()),
                    Some(self.network.airplane()),
                ),
            );
        }
    }
}
impl Drop for QuickSettings {
    fn drop(&mut self) {
        self.close();
    }
}
