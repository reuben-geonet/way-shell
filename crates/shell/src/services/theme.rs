use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Theme {
    Light = 0,
    Dark = 1,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ThemeSource {
    File(PathBuf),
    Resource(&'static str),
}

impl Theme {
    pub fn argument(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Light => "way-shell-light.css",
            Self::Dark => "way-shell-dark.css",
        }
    }
    pub fn resource_path(self) -> &'static str {
        match self {
            Self::Light => "/org/ldelossa/way-shell/data/theme/way-shell-light.css",
            Self::Dark => "/org/ldelossa/way-shell/data/theme/way-shell-dark.css",
        }
    }
    pub fn source(self, directory: &Path) -> ThemeSource {
        let path = directory.join(self.file_name());
        if path.exists() {
            ThemeSource::File(path)
        } else {
            ThemeSource::Resource(self.resource_path())
        }
    }
}

pub fn run_hook(directory: &Path, theme: Theme) -> Result<(), glib::Error> {
    let script = directory.join("on_theme_changed.sh");
    if script.exists() {
        // g_spawn_command_line_async parses arguments without an intervening
        // shell. Quote the path for its parser; Bash receives a script filename.
        let command = format!(
            "bash {} {}",
            glib::shell_quote(script.as_os_str()).to_string_lossy(),
            theme.argument()
        );
        glib::spawn_command_line_async(command)?;
    }
    Ok(())
}

pub(super) struct ThemeStyle {
    display: gtk::gdk::Display,
    provider: gtk::CssProvider,
}

impl Drop for ThemeStyle {
    fn drop(&mut self) {
        gtk::style_context_remove_provider_for_display(&self.display, &self.provider);
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct ThemeService {
        pub settings: RefCell<Option<(gio::Settings, glib::SignalHandlerId)>>,
        pub directory: OnceCell<PathBuf>,
        pub theme: Cell<Option<Theme>>,
        pub(super) style: RefCell<Option<ThemeStyle>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for ThemeService {
        const NAME: &'static str = "WayShellThemeService";
        type Type = super::ThemeService;
    }
    impl ObjectImpl for ThemeService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("theme-changed")
                        .param_types([i32::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            if let Some((settings, handler)) = self.settings.borrow_mut().take() {
                settings.disconnect(handler);
            }
            self.style.borrow_mut().take();
        }
    }
}

glib::wrapper! { pub struct ThemeService(ObjectSubclass<imp::ThemeService>); }

impl ThemeService {
    pub fn new() -> Result<Self, glib::BoolError> {
        Ok(Self::with_settings(
            super::settings::open("org.ldelossa.way-shell.system")?,
            glib::user_config_dir().join("way-shell"),
        ))
    }

    pub fn with_settings(settings: gio::Settings, directory: PathBuf) -> Self {
        let service: Self = glib::Object::new();
        service.imp().directory.set(directory).unwrap();
        let weak = service.downgrade();
        let handler = settings.connect_changed(Some("light-theme"), move |_, _| {
            if let Some(service) = weak.upgrade() {
                service.refresh();
            }
        });
        service.imp().settings.replace(Some((settings, handler)));
        service.refresh();
        service
    }

    pub fn theme(&self) -> Theme {
        self.imp().theme.get().unwrap_or(Theme::Dark)
    }

    pub fn set_theme(&self, theme: Theme) -> Result<(), glib::BoolError> {
        let settings = self
            .imp()
            .settings
            .borrow()
            .as_ref()
            .map(|(settings, _)| settings.clone())
            .ok_or_else(|| glib::bool_error!("Theme service has stopped"))?;
        let unchanged = self.theme() == theme;
        settings.set_boolean("light-theme", theme == Theme::Light)?;
        self.refresh();
        // Repeating a command still reloads user CSS, as in the C service.
        if unchanged {
            self.apply();
        }
        Ok(())
    }

    pub fn attach_display(&self, display: &gtk::gdk::Display) {
        crate::resources::get();
        let provider = gtk::CssProvider::new();
        provider.connect_parsing_error(|_, section, error| {
            let location = section.start_location();
            glib::g_warning!(
                "way-shell",
                "Theme CSS at {}:{}: {error}",
                location.lines() + 1,
                location.line_chars() + 1
            )
        });
        // Remove the previous provider before installing the replacement.
        self.imp().style.borrow_mut().take();
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_THEME,
        );
        self.imp().style.replace(Some(ThemeStyle {
            display: display.clone(),
            provider,
        }));
        self.load_css();
    }

    fn refresh(&self) {
        let theme = match self.imp().settings.borrow().as_ref() {
            Some((settings, _)) if settings.boolean("light-theme") => Theme::Light,
            Some(_) => Theme::Dark,
            None => return,
        };
        if self.imp().theme.replace(Some(theme)) != Some(theme) {
            self.apply();
        }
    }

    fn load_css(&self) {
        let style = self.imp().style.borrow();
        if let Some(style) = style.as_ref() {
            // Keep native GTK/libadwaita controls in step with the shell CSS.
            adw::StyleManager::for_display(&style.display).set_color_scheme(match self.theme() {
                Theme::Light => adw::ColorScheme::ForceLight,
                Theme::Dark => adw::ColorScheme::ForceDark,
            });
            match self.theme().source(self.imp().directory.get().unwrap()) {
                ThemeSource::File(path) => {
                    style.provider.load_from_file(&gio::File::for_path(path))
                }
                ThemeSource::Resource(path) => style.provider.load_from_resource(path),
            }
        }
    }

    fn apply(&self) {
        self.load_css();
        self.emit_by_name::<()>("theme-changed", &[&(self.theme() as i32)]);
        if let Err(error) = run_hook(self.imp().directory.get().unwrap(), self.theme()) {
            glib::g_warning!("way-shell", "Could not run theme hook: {error}");
        }
    }
}
