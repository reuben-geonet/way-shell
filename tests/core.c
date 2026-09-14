#include <math.h>
#include <glib.h>
#include <string.h>

#include "../src/services/wireplumber_service.h"
#include "../src/services/wayland/gamma_control_service/colorramp.h"

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
    g_assert_cmpint(way_shell_gamma_supported(4, 6500), ==, 1);
    g_assert_cmpint(way_shell_gamma_supported(0, 6500), ==, 0);
    g_assert_cmpint(way_shell_gamma_supported(SIZE_MAX, 6500), ==, 0);
    g_assert_cmpint(way_shell_gamma_supported(256, 999), ==, 0);
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
        g_assert_cmpint(colorramp_fill(r, g, b, 4, temperature), ==, 0);
        for (guint i = 0; i < 4; i++) {
            g_assert_cmpuint(r[i], ==, expected[i]);
            g_assert_cmpuint(g[i], ==, expected[i + 4]);
            g_assert_cmpuint(b[i], ==, expected[i + 8]);
        }
    }
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/audio/volume-scaling", volume_scaling);
    g_test_add_func("/audio/channel-map", channels);
    g_test_add_func("/gamma/golden", gamma_golden);
    return g_test_run();
}
