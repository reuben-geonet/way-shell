//! Shared quick-settings menu layout; service controllers own their options.
use gtk::prelude::*;

#[derive(Clone)]
pub struct Menu {
    root: gtk::Box,
    heading: gtk::Box,
    title: gtk::Label,
    icon: gtk::Image,
    banner: gtk::Revealer,
    options: gtk::Box,
}

impl Menu {
    pub fn new(title: &str, icon: &str, scrolling: bool) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("quick-settings-menu");
        root.set_vexpand(true);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_widget_name("container");
        container.set_vexpand(true);
        root.append(&container);

        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        heading.set_widget_name("title-container");
        let icon_container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_container.set_widget_name("icon-container");
        let icon = gtk::Image::from_icon_name(icon);
        icon.set_pixel_size(24);
        icon_container.append(&icon);
        heading.append(&icon_container);
        let title = gtk::Label::new(Some(title));
        heading.append(&title);
        container.append(&heading);

        let banner = gtk::Revealer::new();
        banner.set_widget_name("banner");
        banner.set_transition_type(gtk::RevealerTransitionType::SwingDown);
        banner.set_transition_duration(400);
        container.append(&banner);

        let options = gtk::Box::new(gtk::Orientation::Vertical, 0);
        options.set_widget_name("options-container");
        if scrolling {
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_vexpand(true);
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_child(Some(&options));
            container.append(&scroll);
        } else {
            container.append(&options);
        }
        Self {
            root,
            heading,
            title,
            icon,
            banner,
            options,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn options(&self) -> &gtk::Box {
        &self.options
    }
    pub fn heading(&self) -> &gtk::Box {
        &self.heading
    }
    pub fn banner(&self) -> &gtk::Revealer {
        &self.banner
    }
    pub fn set_title(&self, title: &str) {
        self.title.set_label(title);
    }
    pub fn set_icon(&self, icon: &str) {
        self.icon.set_icon_name(Some(icon));
    }
}
