use gio::prelude::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use way_shell::services::{
    settings,
    theme::{Theme, ThemeService, ThemeSource, run_hook},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "way-shell-theme-' space.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_settings() -> gio::Settings {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    let schema = source
        .lookup("org.ldelossa.way-shell.system", false)
        .unwrap();
    gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None)
}

#[test]
fn settings_changes_and_owned_callbacks() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let directory = Directory::new();
            let settings = test_settings();
            let service = ThemeService::with_settings(settings.clone(), directory.0.clone());
            assert_eq!(service.theme(), Theme::Dark);
            let updates = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let recorded = updates.clone();
            service.connect_local("theme-changed", false, move |values| {
                recorded.borrow_mut().push(values[1].get::<i32>().unwrap());
                None
            });
            service.set_theme(Theme::Light).unwrap();
            assert!(settings.boolean("light-theme"));
            assert_eq!(service.theme(), Theme::Light);
            settings.set_boolean("light-theme", false).unwrap();
            assert_eq!(service.theme(), Theme::Dark);
            assert_eq!(*updates.borrow(), [0, 1]);
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            settings.set_boolean("light-theme", true).unwrap();
            assert_eq!(*updates.borrow(), [0, 1]);
            for _ in 0..20 {
                let service = ThemeService::with_settings(settings.clone(), directory.0.clone());
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            }
        })
        .unwrap();
}

#[test]
fn local_theme_overrides_and_embedded_resources_match() {
    let directory = Directory::new();
    for (theme, original) in [
        (
            Theme::Dark,
            include_bytes!("../../../data/theme/way-shell-dark.css").as_slice(),
        ),
        (
            Theme::Light,
            include_bytes!("../../../data/theme/way-shell-light.css").as_slice(),
        ),
    ] {
        assert!(matches!(
            theme.source(&directory.0),
            ThemeSource::Resource(_)
        ));
        assert_eq!(
            way_shell::resources::get()
                .lookup_data(theme.resource_path(), gio::ResourceLookupFlags::NONE)
                .unwrap()
                .as_ref(),
            original
        );
        let path = directory.0.join(theme.file_name());
        fs::write(&path, b"window { color: red; }").unwrap();
        assert_eq!(theme.source(&directory.0), ThemeSource::File(path));
    }
    assert!(settings::open("org.ldelossa.way-shell.missing-schema").is_err());
}

#[test]
fn hook_keeps_path_and_theme_as_separate_arguments() {
    let directory = Directory::new();
    let script = directory.0.join("on_theme_changed.sh");
    let result = directory.0.join("on_theme_changed.sh.result");
    fs::write(&script, "#!/bin/sh\nprintf '%s' \"$1\" > \"$0.result\"\n").unwrap();
    for theme in [Theme::Light, Theme::Dark] {
        let _ = fs::remove_file(&result);
        run_hook(&directory.0, theme).unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            if fs::read_to_string(&result).ok().as_deref() == Some(theme.argument()) {
                break;
            }
            assert!(
                Instant::now() < end,
                "hook did not produce its theme argument"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
