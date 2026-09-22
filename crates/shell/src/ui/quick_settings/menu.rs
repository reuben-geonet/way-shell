//! Shared quick-settings menu layout; service controllers own their options.
use gtk::prelude::*;

/// Viewport height used before row measurements are available.
const FALLBACK_MAX_CONTENT_HEIGHT: i32 = 360;
/// Viewport height for the first `max_visible` rows.
///
/// Returns `None` when there is nothing measurable yet so callers keep the
/// current height instead of collapsing the list.
fn capped_height(natural_heights: &[i32], max_visible: usize) -> Option<i32> {
    let height: i32 = natural_heights.iter().take(max_visible).sum();
    (height > 0).then_some(height)
}

#[derive(Clone)]
pub struct Menu {
    root: gtk::Box,
    heading: gtk::Box,
    title: gtk::Label,
    icon: gtk::Image,
    banner: gtk::Revealer,
    options: gtk::Box,
    scroll: gtk::ScrolledWindow,
    list: gtk::Box,
}

impl Menu {
    pub fn new(title: &str, icon: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("quick-settings-menu");
        root.set_vexpand(false);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_widget_name("container");
        container.set_vexpand(false);
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
        options.set_vexpand(false);
        container.append(&options);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(0)
            .max_content_height(FALLBACK_MAX_CONTENT_HEIGHT)
            .propagate_natural_height(true)
            .vexpand(false)
            .child(&list)
            .build();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_visible(false);
        options.append(&scroll);
        Self {
            root,
            heading,
            title,
            icon,
            banner,
            options,
            scroll,
            list,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    /// Scrollable rows. Footers and placeholders stay in `options()`.
    pub fn list(&self) -> &gtk::Box {
        &self.list
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
    /// Fit the viewport to the first `max_visible` rows, scrolling the rest.
    pub fn update_viewport(&self, max_visible: usize) {
        let mut heights = Vec::new();
        let mut count = 0;
        let mut child = self.list.first_child();
        while let Some(widget) = child {
            count += 1;
            if heights.len() < max_visible {
                heights.push(widget.measure(gtk::Orientation::Vertical, -1).1);
            }
            child = widget.next_sibling();
        }
        if let Some(height) = capped_height(&heights, max_visible) {
            self.scroll.set_max_content_height(height);
        } else if count == 0 {
            self.scroll
                .set_max_content_height(FALLBACK_MAX_CONTENT_HEIGHT);
        }
        self.scroll.set_visible(count > 0);
        self.scroll.set_policy(
            gtk::PolicyType::Never,
            if count > max_visible {
                gtk::PolicyType::Automatic
            } else {
                gtk::PolicyType::Never
            },
        );
    }
    pub fn set_title(&self, title: &str) {
        self.title.set_label(title);
    }
    pub fn set_icon(&self, icon: &str) {
        self.icon.set_icon_name(Some(icon));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_grows_with_each_row_up_to_the_limit() {
        assert_eq!(capped_height(&[], 5), None);
        assert_eq!(capped_height(&[0, 0], 5), None);
        assert_eq!(capped_height(&[64], 5), Some(64));
        assert_eq!(capped_height(&[64, 64], 5), Some(128));
    }
    #[test]
    fn viewport_stops_growing_after_five_rows() {
        let heights = [60, 62, 58, 64, 61, 59, 63];
        assert_eq!(capped_height(&heights, 5), Some(60 + 62 + 58 + 64 + 61));
    }
}
