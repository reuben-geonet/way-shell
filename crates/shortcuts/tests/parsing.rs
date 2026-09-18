use std::{collections::BTreeMap, path::Path};
use way_shell_shortcuts::*;
fn parse(kind: Compositor, text: &str) -> Snapshot {
    parse_text(kind, Path::new("fixture"), text, &Context::default())
}

#[test]
fn sway_variables_modes_continuations_conditions_and_unbinding() {
    let data = parse(
        Compositor::Sway,
        r#"
set $mod Mod1
set $mode "resize 窗口"
set $shell /opt/bin/way-sh
bindsym $mod+Return exec "$shell" activities \
toggle # comment
bindsym $mod+1 workspace number 1
bindsym --release $mod+1 workspace number 2
bindsym --input-device=keyboard $mod+1 workspace number 3
unbindsym $mod+1
bindcode $mod+38 focus left
mode "$mode" {
    bindsym Escape mode "default"
    bindsym {
        h resize shrink width 10 px
        l resize grow width 10 px
    }
}
bindsym $mod+f [app_id="org.example"] floating toggle; focus left
"#,
    );
    assert!(data.complete(), "{:?}", data.diagnostics);
    assert_eq!(data.bindings.len(), 8);
    assert_eq!(data.bindings[0].trigger, "Alt+Enter");
    assert_eq!(
        data.bindings[0].actions,
        [Action::Shell(
            "\"/opt/bin/way-sh\" activities toggle".into()
        )]
    );
    assert!(data.bindings.iter().any(|b| b.trigger == "Alt+Keycode 38"));
    assert_eq!(
        data.bindings
            .iter()
            .filter(|b| b.mode == "resize 窗口")
            .count(),
        3
    );
    let last = data.bindings.last().unwrap();
    assert_eq!(last.actions.len(), 2);
    assert!(last.conditions.contains(&"[app_id=\"org.example\"]".into()));
}

#[test]
fn niri_kdl_titles_comments_modifiers_and_properties() {
    let text = r#"
input { mod-key "Alt"; mod-key-nested "Super"; }
/-binds { Mod+X { quit; }; }
binds {
    /-Mod+Q { quit; }
    "Mod+Return" hotkey-overlay-title="Open shell" { spawn "/opt/bin/way-sh" "activities" "toggle"; }
    Mod+L allow-when-locked=true { focus-column-right; }
    Mod+H hotkey-overlay-title=null { spawn-sh "way-sh shortcuts toggle"; }
    Mod+1 { focus-workspace "一"; }
}
"#;
    let ctx = Context {
        session: Session::Nested,
        ..Default::default()
    };
    let data = parse_text(Compositor::Niri, Path::new("fixture"), text, &ctx);
    assert!(data.complete(), "{:?}", data.diagnostics);
    assert_eq!(data.bindings.len(), 4);
    assert_eq!(data.bindings[0].trigger, "Super+Enter");
    assert_eq!(data.bindings[1].conditions, ["allow-when-locked=true"]);
    assert_eq!(data.bindings[2].title, Some(None));
    assert!(parse(Compositor::Niri, text).mod_legend);
    assert!(
        !parse(
            Compositor::Niri,
            "binds { Mod+A { quit; }; Mod+A { quit; }; }"
        )
        .complete()
    );
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "way-shortcuts-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn put(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn includes_overrides_cycles_and_missing_dependencies() {
    let dir = Temp::new();
    dir.put(
        "sway",
        "set $mod Mod4\ninclude $PARTS/*.conf\nbindsym $mod+1 workspace 3\n",
    );
    dir.put("a.conf", "bindsym $mod+1 workspace 1\ninclude sway\n");
    dir.put("b.conf", "bindsym $mod+2 workspace 2\n");
    dir.put(".hidden.conf", "bindsym $mod+3 workspace 99\n");
    let ctx = Context {
        environment: BTreeMap::from([("PARTS".into(), dir.0.to_string_lossy().into())]),
        ..Default::default()
    };
    let data = load(Compositor::Sway, &dir.0.join("sway"), &ctx);
    assert_eq!(data.bindings.len(), 2);
    assert_eq!(data.bindings.last().unwrap().original, "workspace 3");
    assert!(
        data.diagnostics
            .iter()
            .any(|d| d.message.contains("Cyclic"))
    );
    dir.put(
        "niri",
        "binds { Mod+A { quit; }; }\ninclude \"part.kdl\"\ninclude optional=true \"missing.kdl\"\n",
    );
    dir.put("part.kdl", "binds { Mod+A { close-window; }; }\n");
    let data = load(Compositor::Niri, &dir.0.join("niri"), &ctx);
    assert!(data.complete(), "{:?}", data.diagnostics);
    assert_eq!(data.bindings.len(), 1);
    assert!(data.bindings[0].original.contains("close-window"));
    assert!(data.files.contains(&dir.0.join("missing.kdl")));
    dir.put("part.kdl", "binds { Mod+A { quit; }; ");
    assert!(!load(Compositor::Niri, &dir.0.join("niri"), &ctx).complete());
}

#[test]
fn physical_keys_aliases_and_limits() {
    assert_eq!(
        normalize_keys("Control+Mod4+Return", false),
        "Ctrl+Super+Enter"
    );
    let data = parse(
        Compositor::Sway,
        "bindsym Mod4+A focus left\nbindsym Mod4+A focus right\n",
    );
    assert_eq!(data.bindings.len(), 1);
    assert!(!parse(Compositor::Sway, "mode \"x\" {\nbindsym x focus left").complete());
    assert!(!parse(Compositor::Niri, &"node { ".repeat(1000)).complete());
    assert!(!parse(Compositor::Sway, &"x".repeat(MAX_FILE_BYTES + 1)).complete());
}

#[test]
fn variable_expansion_is_bounded_and_text_parsing_never_reads_includes() {
    let mut text = "set $a x\n".to_owned();
    for _ in 0..24 {
        text.push_str("set $a $a$a\n");
    }
    let data = parse(Compositor::Sway, &text);
    assert!(!data.complete());
    assert!(
        data.diagnostics
            .iter()
            .any(|d| d.message.contains("expansion limit"))
    );
    let data = parse(
        Compositor::Sway,
        "include /definitely/not/read/*.conf\nbindsym Mod4+A focus left",
    );
    assert!(data.complete());
    assert_eq!(data.bindings.len(), 1);
}

#[test]
fn native_descriptions_preserve_arguments_and_unknown_actions() {
    let data = parse(
        Compositor::Niri,
        "binds { Mod+1 { focus-workspace \"α\"; }; Mod+X { futuristic; }; Mod+T { spawn \"foot\"; }; }",
    );
    assert_eq!(
        describe_native(&data.bindings[0].actions[0]).unwrap().text,
        "Focus workspace α"
    );
    assert_eq!(
        describe_native(&data.bindings[1].actions[0]).unwrap().text,
        "Compositor action: futuristic"
    );
    assert!(describe_native(&data.bindings[2].actions[0]).is_none());
}

#[test]
fn niri_recovers_independent_sections_and_handles_raw_strings() {
    let data = parse(
        Compositor::Niri,
        "binds { Mod+A { focus-workspace r#\"a \\\" { raw }\"#; }; }\nlayout {\n",
    );
    assert!(!data.complete());
    assert_eq!(data.bindings.len(), 1);
    assert_eq!(data.bindings[0].source.line, 1);
    let data = parse(
        Compositor::Niri,
        "/-binds { Mod+A { quit; }; }\nbinds { Mod+B { quit; }; }\ninvalid =\n",
    );
    assert!(!data.complete());
    assert_eq!(data.bindings.len(), 1);
    assert_eq!(data.bindings[0].trigger, "Mod+B");
}

#[test]
fn sway_equivalent_options_and_chords_override_and_unbind() {
    let data = parse(
        Compositor::Sway,
        r#"
bindsym Mod4+a+b focus left
bindsym --input-device=* b+a+Mod4 focus right
bindsym --release --release Mod4+x focus left
unbindsym --release Mod4+x
bindsym --whole-window Mod4+button1 floating toggle
bindsym --border --whole-window Mod4+button1 fullscreen toggle
bindsym --input-device=keyboard Mod4+a+b focus up
bindsym --input-device=keyboard --input-device=* Mod4+a+b focus down
"#,
    );
    assert!(data.complete(), "{:?}", data.diagnostics);
    assert_eq!(data.bindings.len(), 3);
    assert_eq!(data.bindings.last().unwrap().original, "focus down");
    assert!(data.bindings.iter().any(|b| b.original == "focus up"));
    assert!(
        data.bindings
            .iter()
            .any(|b| b.original == "fullscreen toggle")
    );
    assert!(!parse(Compositor::Sway, "bindsym --unknown Mod4+A focus left").complete());
}

#[test]
fn bounded_diagnostics_cannot_hide_parse_errors() {
    let mut text = "bindsym $unknown+A focus left\n".repeat(300);
    text.push_str("mode broken {\n");
    let data = parse(Compositor::Sway, &text);
    assert!(!data.complete());
    assert_eq!(data.diagnostics.len(), 256);
}
