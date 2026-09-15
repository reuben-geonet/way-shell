#include <glib.h>
#include <glib/gstdio.h>
#include "../src/services/brightness_service/brightness_service.c"

static guint requested;
LogindService *logind_service_get_global(void) { return NULL; }
int logind_service_session_set_brightness(LogindService *service,
                                          const gchar *subsystem,
                                          const gchar *name,
                                          guint brightness) {
    requested = brightness;
    return -1;
}

static void failed_writes_keep_observed_state(void) {
    g_autofree gchar *directory = g_dir_make_tmp("way-shell-brightness-XXXXXX", NULL);
    g_autofree gchar *path = g_build_filename(directory, "brightness", NULL);
    g_assert_true(g_file_set_contents(path, "2\n", -1, NULL));
    g_autoptr(GSettings) settings = g_settings_new("org.ldelossa.way-shell.system");
    g_autoptr(GFile) device = g_file_new_for_path(directory);
    BrightnessService service = {.backlight_brightness = 2,
                                  .max_backlight_brightness = 24,
                                  .backlight_device_path = device,
                                  .systems_settings = settings,
                                  .keyboard_device_path = device,
                                  .keyboard_brightness = 2,
                                  .keyboard_max_brightness = 3};
    brightness_service_backlight_up(&service);
    g_assert_cmpuint(requested, ==, 4);
    g_assert_cmpuint(service.backlight_brightness, ==, 2);
    brightness_service_backlight_down(&service);
    g_assert_cmpuint(requested, ==, 0);
    g_assert_cmpuint(service.backlight_brightness, ==, 2);
    brightness_service_keyboard_up(&service);
    g_assert_cmpuint(requested, ==, 3);
    g_assert_cmpuint(service.keyboard_brightness, ==, 2);
    brightness_service_keyboard_down(&service);
    g_assert_cmpuint(requested, ==, 1);
    g_assert_cmpuint(service.keyboard_brightness, ==, 2);
    service.max_backlight_brightness = 5;
    brightness_service_backlight_up(&service);
    g_assert_cmpuint(requested, ==, 3);
    brightness_service_backlight_down(&service);
    g_assert_cmpuint(requested, ==, 1);
    g_assert_cmpuint(service.backlight_brightness, ==, 2);
    g_unlink(path);
    g_rmdir(directory);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/brightness/failed-write-keeps-observed-state", failed_writes_keep_observed_state);
    return g_test_run();
}
