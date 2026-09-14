/* Characterize wl_array's byte length before replacing the C protocol reader. */
#include "../src/services/wayland/foreign_toplevel_service/foreign_toplevel.c"

static void state_array(void) {
    WaylandForeignToplevelService service = {0};
    WaylandWLRForeignTopLevel top = {0};
    service.toplevels = g_hash_table_new(g_direct_hash, g_direct_equal);
    void *handle = &top;
    g_hash_table_insert(service.toplevels, handle, &top);
    uint32_t *bytes = g_new(uint32_t, 1);
    *bytes = ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_ACTIVATED;
    struct wl_array state = {.size = sizeof(uint32_t), .alloc = sizeof(uint32_t), .data = bytes};
    toplevel_handle_state(&service, handle, &state);
    g_assert_true(top.activated);
    *bytes = ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_MINIMIZED;
    toplevel_handle_state(&service, handle, &state);
    g_assert_false(top.activated);
    state.size = 0;
    toplevel_handle_state(&service, handle, &state);
    g_assert_false(top.activated);
    g_free(bytes);
    g_hash_table_unref(service.toplevels);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/wayland/state-array-byte-length", state_array);
    return g_test_run();
}
