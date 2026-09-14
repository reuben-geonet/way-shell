#include <glib.h>
#include "../src/services/upower_service.h"

static void icon_at_ninety_percent(void) {
    g_autoptr(UpDevice) device = up_device_new();
    const guint states[] = {UP_DEVICE_STATE_DISCHARGING,
                            UP_DEVICE_STATE_CHARGING,
                            UP_DEVICE_STATE_FULLY_CHARGED};
    const char *expected[] = {"battery-level-90-symbolic",
                              "battery-level-90-charging-symbolic",
                              "battery-full-charging-symbolic"};
    g_object_set(device, "is-rechargeable", TRUE, "percentage", 90.0,
                 "icon-name", "battery-full-symbolic", NULL);
    for (guint i = 0; i < G_N_ELEMENTS(states); i++) {
        g_object_set(device, "state", states[i], NULL);
        g_assert_cmpstr(upower_device_map_icon_name(device), ==, expected[i]);
    }
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/power/icons/ninety-percent", icon_at_ninety_percent);
    return g_test_run();
}
