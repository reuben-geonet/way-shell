//! Saved shortcuts and panel popup lifecycle on the private Sway/Niri display.
use adw::prelude::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use way_shell::{
    resources,
    services::{
        settings,
        shortcuts::{ShortcutsService, Status},
        theme::{Theme, ThemeService},
        wm::WindowManager,
    },
    ui::shortcuts::ShortcutsSheet,
};

fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Shortcut smoke condition timed out"
        );
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}
fn settle() {
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(250)));
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(std::env::var("WAY_SHELL_TEST_BUS").as_deref(), Ok("1"));
    assert_eq!(std::env::var("GSETTINGS_BACKEND").as_deref(), Ok("memory"));
    adw::init()?;
    resources::get();
    let display = gtk::gdk::Display::default().unwrap();
    let niri = std::env::var_os("NIRI_SOCKET").is_some();
    let manager = if niri {
        WindowManager::niri()?
    } else {
        WindowManager::sway()?
    };
    wait(|| manager.is_connected());
    let settings = settings::open("org.ldelossa.way-shell.window-manager")?;
    let key = if niri {
        "niri-config-path"
    } else {
        "sway-config-path"
    };
    let directory =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap()).join("shortcuts-fixture");
    std::fs::create_dir_all(&directory)?;
    let root = directory.join("config");
    let part = directory.join("part");
    std::fs::write(
        &root,
        if niri {
            "include \"part\"\n"
        } else {
            "include part\n"
        },
    )?;
    let binding = |workspace: &str| {
        if niri {
            format!("binds {{ Mod+1 {{ focus-workspace \"{workspace}\"; }}; }}\n")
        } else {
            format!("bindsym Mod4+1 workspace \"{workspace}\"\n")
        }
    };
    std::fs::write(&part, binding("first"))?;
    settings.set_string(key, root.to_str().unwrap())?;
    let service = ShortcutsService::new(manager.clone(), settings.clone());
    assert_eq!(service.data().status, Status::Loading);
    let sheet = ShortcutsSheet::new(service.clone(), manager.clone())?;
    sheet.show()?;
    sheet.show()?;
    wait(|| sheet.popup().window().is_mapped() && service.data().status == Status::Current);
    assert_eq!(service.data().rows.len(), 1);
    let monitor = sheet.popup().monitor().unwrap();
    assert!(sheet.popup().window().width() <= monitor.geometry().width());
    assert!(sheet.popup().window().height() < monitor.geometry().height());
    assert!(
        sheet
            .popup()
            .underlays()
            .windows()
            .iter()
            .all(|w| w.window().is_visible())
    );
    let temporary = directory.join("replacement");
    std::fs::write(&temporary, binding("replacement"))?;
    std::fs::rename(&temporary, &part)?;
    wait(|| {
        service
            .data()
            .rows
            .iter()
            .any(|r| r.description.contains("replacement"))
    });
    // An invalid save retains the previous complete source, then recovers.
    std::fs::write(&part, if niri { "binds {" } else { "mode resize {" })?;
    wait(|| service.data().status == Status::Stale);
    assert!(service.data().rows[0].description.contains("replacement"));
    std::fs::write(
        &part,
        binding(&"A long workspace name 日本語 & <plain text> ".repeat(10)),
    )?;
    wait(|| service.data().status == Status::Current);
    settle();
    assert!(sheet.popup().window().width() <= monitor.geometry().width());
    let theme = ThemeService::with_settings(
        settings::open("org.ldelossa.way-shell.system")?,
        directory.join("theme"),
    );
    theme.attach_display(&display);
    for selected in [Theme::Light, Theme::Dark] {
        theme.set_theme(selected)?;
        settle();
    }
    if !niri {
        let run = |args: &[&str]| {
            let output = std::process::Command::new("swaymsg")
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["create_output"]);
        wait(|| display.monitors().n_items() == 2);
        let second = (0..display.monitors().n_items())
            .filter_map(|i| {
                display
                    .monitors()
                    .item(i)
                    .and_downcast::<gtk::gdk::Monitor>()
            })
            .find(|m| *m != monitor)
            .unwrap();
        sheet.toggle_on(&second)?;
        assert_eq!(sheet.popup().monitor().as_ref(), Some(&second));
        assert!(sheet.popup().is_visible());
        sheet.toggle_on(&monitor)?;
        assert_eq!(sheet.popup().monitor().as_ref(), Some(&monitor));
        sheet.toggle_on(&second)?;
        run(&["output", second.connector().unwrap().as_str(), "disable"]);
        wait(|| !second.is_valid());
        assert!(!sheet.popup().is_visible());
        sheet.toggle_on(&monitor)?;
    }
    // Click-away and Escape release underlays and the keyboard surface.
    sheet.popup().underlays().buttons()[0].emit_clicked();
    assert!(!sheet.popup().is_visible());
    assert!(
        sheet
            .popup()
            .underlays()
            .windows()
            .iter()
            .all(|w| !w.window().is_visible())
    );
    sheet.show()?;
    let controllers = sheet.popup().window().observe_controllers();
    let keys = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .unwrap();
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Escape,
            &0u32,
            &gtk::gdk::ModifierType::empty(),
        ],
    );
    assert!(!sheet.popup().is_visible());
    sheet.toggle_on(&monitor)?;
    assert!(sheet.popup().is_visible());
    sheet.toggle_on(&monitor)?;
    assert!(!sheet.popup().is_visible());
    std::fs::write(&part, binding("hidden-change"))?;
    settle();
    sheet.show()?;
    wait(|| service.data().rows[0].description.contains("hidden-change"));
    // Changing source must clear old rows; creation of a missing file recovers.
    let missing = directory.join("missing/subdir/config");
    settings.set_string(key, missing.to_str().unwrap())?;
    wait(|| service.data().status == Status::Unavailable);
    assert!(service.data().rows.is_empty());
    assert!(sheet.popup().is_visible());
    std::fs::create_dir_all(missing.parent().unwrap())?;
    std::fs::write(&missing, binding("recovered"))?;
    wait(|| service.data().status == Status::Current);
    for index in 0..8 {
        std::fs::write(&missing, binding(&format!("rapid-{index}")))?;
    }
    wait(|| service.data().rows[0].description.contains("rapid-7"));
    let retained = sheet.popup().window().clone();
    let buttons = sheet.popup().underlays().buttons();
    service.retry();
    sheet.close();
    service.stop();
    for button in buttons {
        button.emit_clicked();
    }
    settle();
    assert!(!retained.is_visible());
    assert!(sheet.show().is_err());
    manager.stop();
    println!("shortcut sheet refresh, sizing, dismissal, recovery and cleanup passed");
    Ok(())
}
