#include <glib.h>
#include <json-glib/json-glib.h>
#include <sys/socket.h>
#include <unistd.h>
#include "../src/services/window_manager_service/niri/niri_client.h"

static GIOChannel *fixture(int *peer, const char *reply) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets), ==, 0);
    *peer = sockets[1];
    g_assert_cmpint(write(*peer, reply, strlen(reply)), ==, strlen(reply));
    g_assert_cmpint(write(*peer, "\n", 1), ==, 1);
    GIOChannel *channel = g_io_channel_unix_new(sockets[0]);
    g_io_channel_set_close_on_unref(channel, TRUE);
    return channel;
}
static void workspaces(void) {
    g_autofree char *reply = NULL;
    g_assert_true(g_file_get_contents("tests/fixtures/niri-workspaces.json", &reply, NULL, NULL));
    /* IPC replies are one physical line. */
    for (char *p = reply; *p; p++) if (*p == '\n') *p = ' ';
    int peer;
    GIOChannel *channel = fixture(&peer, reply);
    GPtrArray *items = niri_client_get_workspaces(channel);
    g_assert_nonnull(items);
    g_assert_cmpuint(items->len, ==, 2);
    WMWorkspace *named = NULL;
    for (guint i = 0; i < items->len; i++) {
        WMWorkspace *item = g_ptr_array_index(items, i);
        if (item->id == 4294967313ULL) named = item;
    }
    g_assert_nonnull(named);
    g_assert_cmpint(named->num, ==, 2);
    g_assert_false(named->focused);
    g_assert_true(named->visible);
    g_assert_true(named->urgent);
    g_assert_cmpstr(named->name, ==, "2: 日本語");
    g_ptr_array_unref(items);
    g_io_channel_unref(channel);
    close(peer);
}
static void outputs(void) {
    g_autofree char *reply = NULL;
    g_assert_true(g_file_get_contents("tests/fixtures/niri-outputs.json", &reply, NULL, NULL));
    for (char *p = reply; *p; p++) if (*p == '\n') *p = ' ';
    int peer;
    GIOChannel *channel = fixture(&peer, reply);
    GPtrArray *items = niri_client_get_outputs(channel);
    g_assert_nonnull(items);
    g_assert_cmpuint(items->len, ==, 2);
    WMOutput *item = g_ptr_array_index(items, 0);
    g_assert_cmpstr(item->name, !=, "Ok");
    g_assert_nonnull(item->make);
    g_ptr_array_unref(items);
    g_io_channel_unref(channel);
    close(peer);
}
static void actions(void) {
    const char *names[] = {"2: 日本語", "a \"quote\" and \\slash", "7"};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        int peer;
        GIOChannel *channel = fixture(&peer, "{\"Ok\":\"Handled\"}");
        g_assert_cmpint(niri_client_focus_workspace(channel, names[i]), ==, 0);
        char bytes[1024] = {0};
        g_assert_cmpint(read(peer, bytes, sizeof(bytes) - 1), >, 0);
        g_autoptr(JsonParser) parser = json_parser_new();
        g_assert_true(json_parser_load_from_data(parser, bytes, -1, NULL));
        JsonObject *object = json_node_get_object(json_parser_get_root(parser));
        object = json_object_get_object_member(object, "Action");
        object = json_object_get_object_member(object, "FocusWorkspace");
        object = json_object_get_object_member(object, "reference");
        if (i == 2) g_assert_cmpint(json_object_get_int_member(object, "Index"), ==, 7);
        else g_assert_cmpstr(json_object_get_string_member(object, "Name"), ==, names[i]);
        g_io_channel_unref(channel);
        close(peer);
    }
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/niri/workspaces", workspaces);
    g_test_add_func("/niri/outputs", outputs);
    g_test_add_func("/niri/actions", actions);
    return g_test_run();
}
