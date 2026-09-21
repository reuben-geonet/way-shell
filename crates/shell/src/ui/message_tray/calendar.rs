//! Calendar selection and local date updates share the shell clock.
use crate::services::clock::ClockService;
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn date_key(date: &glib::DateTime) -> (i32, i32, i32) {
    (date.year(), date.month(), date.day_of_month())
}
fn format(date: &glib::DateTime, pattern: &str) -> String {
    date.format(pattern).map(Into::into).unwrap_or_default()
}

pub struct Calendar {
    clock: ClockService,
    now: RefCell<glib::DateTime>,
    root: gtk::Box,
    calendar: gtk::Calendar,
    today: gtk::Button,
    today_row: adw::ActionRow,
    month: gtk::Label,
    handler: Cell<Option<glib::SignalHandlerId>>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    closed: Cell<bool>,
}
impl Calendar {
    pub fn new(clock: ClockService) -> Result<Rc<Self>, glib::BoolError> {
        let now = glib::DateTime::now_local()?;
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("calendar-area");
        let today = gtk::Button::new();
        today.set_widget_name("calendar-today");
        today.set_hexpand(true);
        let today_row = adw::ActionRow::new();
        today.set_child(Some(&today_row));
        let selector = gtk::CenterBox::new();
        selector.set_widget_name("calendar-month-selector");
        let back = gtk::Button::from_icon_name("pan-start-symbolic");
        let forward = gtk::Button::from_icon_name("pan-end-symbolic");
        let month = gtk::Label::new(None);
        selector.set_start_widget(Some(&back));
        selector.set_center_widget(Some(&month));
        selector.set_end_widget(Some(&forward));
        let calendar = gtk::Calendar::new();
        calendar.set_size_request(-1, 224);
        calendar.set_show_heading(false);
        calendar.set_show_week_numbers(false);
        calendar.set_hexpand(true);
        calendar.select_day(&now);
        root.append(&today);
        root.append(&selector);
        root.append(&calendar);
        let this = Rc::new(Self {
            clock,
            now: RefCell::new(now),
            root,
            calendar,
            today,
            today_row,
            month,
            handler: Cell::new(None),
            updating: Cell::new(false),
            pending: Cell::new(false),
            closed: Cell::new(false),
        });
        for (button, amount) in [(back, -1), (forward, 1)] {
            let weak = Rc::downgrade(&this);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade().filter(|this| !this.closed.get())
                    && let Ok(date) = this.calendar.date().add_months(amount)
                {
                    this.calendar.select_day(&date);
                    this.refresh();
                }
            });
        }
        let weak = Rc::downgrade(&this);
        this.today.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade().filter(|this| !this.closed.get()) {
                let now = this.now.borrow().clone();
                if date_key(&this.calendar.date()) != date_key(&now) {
                    this.calendar.select_day(&now);
                    this.calendar.clear_marks();
                }
                this.refresh();
            }
        });
        let weak = Rc::downgrade(&this);
        this.calendar.connect_day_selected(move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
        });
        let weak = Rc::downgrade(&this);
        this.handler.set(Some(this.clock.connect_local(
            "tick",
            false,
            move |values| {
                if let Some(this) = weak.upgrade().filter(|this| !this.closed.get())
                    && let Ok(now) = values[1].get::<glib::DateTime>()
                {
                    let changed = date_key(&this.now.borrow()) != date_key(&now);
                    this.now.replace(now);
                    if changed {
                        this.refresh();
                    }
                }
                None
            },
        )));
        this.refresh();
        Ok(this)
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn calendar(&self) -> &gtk::Calendar {
        &self.calendar
    }
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(handler) = self.handler.take() {
            self.clock.disconnect(handler);
        }
        self.root.set_sensitive(false);
    }
    fn refresh(&self) {
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        while self.pending.replace(false) && !self.closed.get() {
            let now = self.now.borrow().clone();
            let selected = self.calendar.date();
            self.month.set_label(&format(&selected, "%B %Y"));
            self.today_row.set_title(&format(&now, "%A"));
            self.today_row.set_subtitle(&format(&now, "%B %d %Y"));
            if date_key(&now) == date_key(&selected) {
                self.today.remove_css_class("calendar-today-dirty");
            } else {
                self.today.add_css_class("calendar-today-dirty");
            }
        }
        self.updating.set(false);
    }
}
impl Drop for Calendar {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_dates_distinguish_year_changes_and_leap_days() {
        let date =
            |year, month, day| glib::DateTime::from_utc(year, month, day, 12, 0, 0.0).unwrap();
        assert_ne!(date_key(&date(2025, 1, 1)), date_key(&date(2026, 1, 1)));
        assert_ne!(date_key(&date(2024, 2, 29)), date_key(&date(2025, 3, 1)));
        assert_eq!(
            date_key(&date(2024, 1, 31).add_months(1).unwrap()),
            (2024, 2, 29)
        );
        assert_eq!(
            date_key(&date(2025, 1, 31).add_months(1).unwrap()),
            (2025, 2, 28)
        );
    }
}
