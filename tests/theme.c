#include <glib/gstdio.h>
#include "../src/services/theme_service.c"

static void hook_path_is_one_argument(void) {
    GError *error = NULL;
    gchar *root = g_dir_make_tmp("way-shell-theme-' space.XXXXXX", &error);
    g_assert_no_error(error);
    gchar *config = g_build_filename(root, "way-shell", NULL);
    g_assert_cmpint(g_mkdir(config, 0700), ==, 0);
    gchar *script = g_build_filename(config, "on_theme_changed.sh", NULL);
    gchar *result = g_strconcat(script, ".result", NULL);
    g_file_set_contents(script, "#!/bin/sh\nprintf '%s' \"$1\" > \"$0.result\"\n", -1, &error);
    g_assert_no_error(error);
    g_assert_cmpint(g_chmod(script, 0700), ==, 0);
    run_theme_hook(root, "light");
    gchar *contents = NULL;
    for (int i = 0; i < 200; i++) {
        g_free(contents);
        contents = NULL;
        g_file_get_contents(result, &contents, NULL, NULL);
        if (contents && g_str_equal(contents, "light")) break;
        g_usleep(10000);
    }
    g_assert_cmpstr(contents, ==, "light");
    g_free(contents);
    g_remove(result);
    g_remove(script);
    g_rmdir(config);
    g_rmdir(root);
    g_free(result);
    g_free(script);
    g_free(config);
    g_free(root);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/theme/hook-path", hook_path_is_one_argument);
    return g_test_run();
}
