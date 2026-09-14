#include "window_manager_service_sway.h"
#include "../rust_adapter.h"

/* Temporary GObject signal/vtable adapter. State, I/O and actions live in Rust. */
struct _WMServiceSway {
    GObject parent_instance;
    void *rust;
};
enum { workspaces_changed, outputs_changed, signals_n };
static guint service_signals[signals_n];
G_DEFINE_TYPE(WMServiceSway, wm_service_sway, G_TYPE_OBJECT);

static void wm_service_sway_dispose(GObject *object) {
    WMServiceSway *self = WM_SERVICE_SWAY(object);
    g_clear_pointer(&self->rust, way_shell_wm_free);
    G_OBJECT_CLASS(wm_service_sway_parent_class)->dispose(object);
}

static void wm_service_sway_class_init(WMServiceSwayClass *klass) {
    G_OBJECT_CLASS(klass)->dispose = wm_service_sway_dispose;
    service_signals[workspaces_changed] = g_signal_new(
        "workspaces-changed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
        0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_PTR_ARRAY);
    service_signals[outputs_changed] = g_signal_new(
        "outputs-changed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
        0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_PTR_ARRAY);
}

static void on_workspaces(void *data, GPtrArray *snapshot) {
    g_signal_emit(data, service_signals[workspaces_changed], 0, snapshot);
}
static void on_outputs(void *data, GPtrArray *snapshot) {
    g_signal_emit(data, service_signals[outputs_changed], 0, snapshot);
}
static void wm_service_sway_init(WMServiceSway *self) {
    self->rust = way_shell_wm_new_sway(on_workspaces, on_outputs, self);
}
static GPtrArray *get_workspaces(WindowManager *wm) {
    return way_shell_wm_workspaces(WM_SERVICE_SWAY(wm->private)->rust);
}
static GPtrArray *get_outputs(WindowManager *wm) {
    return way_shell_wm_outputs(WM_SERVICE_SWAY(wm->private)->rust);
}
static int focus_workspace(WindowManager *wm, WMWorkspace *workspace) {
    if (!workspace) return -1;
    return way_shell_wm_workspace_action(WM_SERVICE_SWAY(wm->private)->rust,
        0, workspace->id, workspace->num, workspace->name);
}
static int move_window(WindowManager *wm, WMWorkspace *workspace) {
    if (!workspace) return -1;
    return way_shell_wm_workspace_action(WM_SERVICE_SWAY(wm->private)->rust,
        1, workspace->id, workspace->num, workspace->name);
}
static int rename_workspace(WindowManager *wm, const char *name) {
    return way_shell_wm_named_action(WM_SERVICE_SWAY(wm->private)->rust, 0, name);
}
static int move_workspace(WindowManager *wm, WMOutput *output) {
    if (!output) return -1;
    return way_shell_wm_named_action(WM_SERVICE_SWAY(wm->private)->rust, 1, output->name);
}
static guint register_workspaces(WindowManager *wm, wm_on_workspaces_changed callback, void *data) {
    return g_signal_connect_swapped(wm->private, "workspaces-changed", G_CALLBACK(callback), data);
}
static guint unregister_workspaces(WindowManager *wm, wm_on_workspaces_changed callback, void *data) {
    return g_signal_handlers_disconnect_by_func(wm->private, callback, data);
}
static guint register_outputs(WindowManager *wm, wm_on_outputs_changed callback, void *data) {
    return g_signal_connect_swapped(wm->private, "outputs-changed", G_CALLBACK(callback), data);
}
static guint unregister_outputs(WindowManager *wm, wm_on_outputs_changed callback, void *data) {
    return g_signal_handlers_disconnect_by_func(wm->private, callback, data);
}

WindowManager *wm_service_sway_window_manager_init(void) {
    WMServiceSway *self = g_object_new(WM_SERVICE_SWAY_TYPE, NULL);
    if (!self->rust) { g_object_unref(self); return NULL; }
    WindowManager *wm = g_new0(WindowManager, 1);
    *wm = (WindowManager) {
        .private = self, .get_workspaces = get_workspaces, .get_outputs = get_outputs,
        .focus_workspace = focus_workspace, .rename_workspace = rename_workspace,
        .current_ws_to_output = move_workspace, .current_app_to_workspace = move_window,
        .register_on_workspaces_changed = register_workspaces,
        .unregister_on_workspaces_changed = unregister_workspaces,
        .register_on_outputs_changed = register_outputs,
        .unregister_on_outputs_changed = unregister_outputs,
    };
    return wm;
}
