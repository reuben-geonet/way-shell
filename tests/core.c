#include <math.h>
#include <glib.h>
#include <string.h>

#include "../lib/cmd_tree/include/cmd_tree.h"
#include "../src/services/wireplumber_service.h"
#include "../src/services/wayland/gamma_control_service/colorramp.h"
#include "../src/services/window_manager_service/sway/sway_client.h"

static void command_tree(void) {
    cmd_tree_node_t root = {0}, volume = {.name = "volume"},
                    set = {.name = "set"}, mute = {.name = "mute"};
    cmd_tree_node_t *found = NULL;
    char *args[] = {"volume", "set", "0.5"};
    g_assert_cmpint(cmd_tree_node_add_child(NULL, &set), ==, -1);
    g_assert_cmpint(cmd_tree_node_add_child(&root, &volume), ==, 1);
    g_assert_cmpint(cmd_tree_node_add_child(&volume, &set), ==, 1);
    g_assert_cmpint(cmd_tree_node_add_child(&volume, &mute), ==, 1);
    g_assert_cmpint(cmd_tree_search(&root, 3, args, &found), ==, 1);
    g_assert_true(found == &set);
    g_assert_cmpuint(found->argc, ==, 1);
    g_assert_cmpstr(found->argv[0], ==, "0.5");
    args[1] = "mute";
    g_assert_cmpint(cmd_tree_search(&root, 2, args, &found), ==, 1);
    g_assert_true(found == &mute);
    g_assert_cmpuint(found->argc, ==, 0);
    g_assert_cmpint(cmd_tree_search(&root, 0, NULL, &found), ==, 1);
    g_assert_true(found == &root);
    g_assert_cmpint(cmd_tree_search(NULL, 0, NULL, &found), ==, -1);
}

static void volume_scaling(void) {
    g_assert_cmpfloat(volume_from_linear(-1, SCALE_CUBIC), ==, 0);
    g_assert_cmpfloat(volume_to_linear(-1, SCALE_CUBIC), ==, 0);
    g_assert_cmpfloat(volume_from_linear(0.125f, SCALE_CUBIC), ==, 0.5);
    g_assert_cmpfloat(volume_to_linear(0.5, SCALE_CUBIC), ==, 0.125f);
    g_assert_cmpfloat(volume_from_linear(0.5f, SCALE_LINEAR), ==, 0.5);
    g_assert_cmpfloat(volume_to_linear(0.5, SCALE_LINEAR), ==, 0.5f);
    for (int i = 0; i <= 100; i++) {
        double value = i / 100.0;
        g_assert_cmpfloat_with_epsilon(
            volume_from_linear(volume_to_linear(value, SCALE_CUBIC), SCALE_CUBIC),
            value, 1e-6);
    }
}

static void channels(void) {
    const char *names[] = {"RL", "RR", "FL", "FR", "C", "LFE", "SL", "SR",
                           "RHL", "RHR", "TFL", "TFR"};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++)
        g_assert_cmpint(wire_plumber_service_map_port(names[i]), ==, i);
    g_assert_cmpint((int)wire_plumber_service_map_port(NULL), ==, -1);
    g_assert_cmpint((int)wire_plumber_service_map_port("MONO"), ==, -1);
}

static void gamma_golden(void) {
    gchar *contents = NULL;
    g_assert_true(g_file_get_contents("tests/fixtures/gamma.tsv", &contents, NULL, NULL));
    g_auto(GStrv) lines = g_strsplit(contents, "\n", -1);
    g_free(contents);
    for (guint row = 0; lines[row] && *lines[row]; row++) {
        int temperature;
        unsigned expected[12];
        g_assert_cmpint(sscanf(lines[row], "%d %u %u %u %u %u %u %u %u %u %u %u %u",
            &temperature, &expected[0], &expected[1], &expected[2], &expected[3],
            &expected[4], &expected[5], &expected[6], &expected[7], &expected[8],
            &expected[9], &expected[10], &expected[11]), ==, 13);
        uint16_t r[] = {0, 16384, 32768, 49152};
        uint16_t g[] = {0, 16384, 32768, 49152};
        uint16_t b[] = {0, 16384, 32768, 49152};
        colorramp_fill(r, g, b, 4, temperature);
        for (guint i = 0; i < 4; i++) {
            g_assert_cmpuint(r[i], ==, expected[i]);
            g_assert_cmpuint(g[i], ==, expected[i + 4]);
            g_assert_cmpuint(b[i], ==, expected[i + 8]);
        }
    }
}

static sway_client_ipc_msg fixture(const char *path) {
    sway_client_ipc_msg msg = {0};
    gsize size = 0;
    g_assert_true(g_file_get_contents(path, &msg.payload, &size, NULL));
    msg.size = size;
    return msg;
}

static void sway_workspaces(void) {
    sway_client_ipc_msg msg = fixture("tests/fixtures/sway-workspaces.json");
    GPtrArray *items = sway_client_ipc_get_workspaces_resp(&msg);
    g_assert_nonnull(items);
    g_assert_cmpuint(items->len, ==, 2);
    WMWorkspace *first = items->pdata[0], *second = items->pdata[1];
    g_assert_cmpuint(first->id, ==, 17);
    g_assert_cmpstr(first->name, ==, "1: web");
    g_assert_cmpstr(first->output, ==, "eDP-1");
    g_assert_true(first->focused);
    g_assert_false(first->urgent);
    g_assert_cmpint(second->num, ==, -1);
    g_assert_cmpstr(second->name, ==, "日本語");
    g_assert_true(second->urgent);
    g_ptr_array_unref(items);
}

static void sway_outputs(void) {
    sway_client_ipc_msg msg = fixture("tests/fixtures/sway-outputs.json");
    GPtrArray *items = sway_client_ipc_get_outputs_resp(&msg);
    g_assert_nonnull(items);
    g_assert_cmpuint(items->len, ==, 2);
    WMOutput *first = items->pdata[0], *second = items->pdata[1];
    g_assert_cmpstr(first->name, ==, "eDP-1");
    g_assert_cmpstr(first->current_workspace, ==, "1: web");
    g_assert_cmpstr(second->serial, ==, "1234");
    g_assert_null(second->current_workspace);
    g_ptr_array_unref(items);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/commands/lookup-and-consumption", command_tree);
    g_test_add_func("/audio/volume-scaling", volume_scaling);
    g_test_add_func("/audio/channel-map", channels);
    g_test_add_func("/gamma/golden", gamma_golden);
    g_test_add_func("/sway/workspaces", sway_workspaces);
    g_test_add_func("/sway/outputs", sway_outputs);
    return g_test_run();
}
