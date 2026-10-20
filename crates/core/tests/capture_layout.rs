use way_shell_core::{capture::Rect, niri, sway};
fn json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}
#[test]
fn sway_visible_workspaces_floating_windows_clipping_and_rotation() {
    let layout = sway::capture_layout(
        &json(include_bytes!(
            "../../../tests/fixtures/sway-capture-outputs.json"
        )),
        &json(include_bytes!(
            "../../../tests/fixtures/sway-capture-tree.json"
        )),
    )
    .unwrap();
    assert_eq!(layout.outputs.len(), 2);
    assert_eq!(layout.outputs[1].native_size, (150, 240));
    assert_eq!(
        layout.windows.iter().map(|w| w.id).collect::<Vec<_>>(),
        [1, 3, 5]
    );
    assert_eq!(
        layout.windows[2].bounds,
        Rect {
            x: -10.0,
            y: -160.0,
            width: 110.0,
            height: 90.0
        }
    );
    assert_eq!(layout.focused_output.as_deref(), Some("DP-2"));
}
#[test]
fn niri_active_workspaces_fractional_tiles_and_missing_geometry() {
    let outputs = json(include_bytes!(
        "../../../tests/fixtures/niri-capture-outputs.json"
    ));
    let workspaces = json(include_bytes!(
        "../../../tests/fixtures/niri-capture-workspaces.json"
    ));
    let mut windows = json(include_bytes!(
        "../../../tests/fixtures/niri-capture-windows.json"
    ));
    let layout = niri::capture_layout(&outputs, &workspaces, &windows).unwrap();
    assert_eq!(layout.outputs[1].native_size, (150, 240));
    assert_eq!(
        layout.windows.iter().map(|w| w.id).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(layout.windows[0].bounds.x, -159.5);
    assert_eq!(
        layout.windows[1].bounds,
        Rect {
            x: 0.0,
            y: -144.75,
            width: 49.75,
            height: 50.5
        }
    );
    assert_eq!(layout.launching_output(None), 1);
    for window in windows.as_array_mut().unwrap() {
        window.as_object_mut().unwrap().remove("layout");
    }
    let layout = niri::capture_layout(&outputs, &workspaces, &windows).unwrap();
    assert!(layout.windows.is_empty());
    assert_eq!(layout.outputs.len(), 2);
}
