/* Exercise the actual menu without issuing any machine power operations. */
#include <adwaita.h>
#include "../src/panel/quick_settings/quick_settings_header/quick_settings_power_menu.c"

typedef struct { GObject parent_instance; } FixtureService;
typedef struct { GObjectClass parent_class; } FixtureServiceClass;
G_DEFINE_TYPE(FixtureService, fixture_service, G_TYPE_OBJECT)
static void fixture_service_class_init(FixtureServiceClass *klass) {
    g_signal_new("quick-settings-hidden", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
                 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_service_init(FixtureService *self) {}
static GObject *settings_fixture;
QuickSettings *quick_settings_get_global(void) { return (QuickSettings *)settings_fixture; }
void quick_settings_set_hidden(QuickSettings *self) {}
LogindService *logind_service_get_global(void) { return NULL; }
DialogOverlay *dialog_overlay_get_global(void) { return NULL; }
void dialog_overlay_present(DialogOverlay *self, gchar *heading, gchar *body, GCallback cb) { g_assert_not_reached(); }
#define ACTION(name) \
gboolean logind_service_can_##name(LogindService *self) { return FALSE; } \
void logind_service_##name(LogindService *self) { g_assert_not_reached(); }
ACTION(reboot)
ACTION(power_off)
ACTION(suspend)
ACTION(hibernate)
ACTION(hybrid_sleep)
ACTION(suspendthenhibernate)
void logind_service_kill_session(LogindService *self) { g_assert_not_reached(); }
static void destroyed(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void lifecycle(void) {
    settings_fixture = g_object_new(fixture_service_get_type(), NULL);
    for (unsigned cycle = 0; cycle < 3; ++cycle) {
        QuickSettingsPowerMenu *menu = g_object_new(QUICK_SETTINGS_POWER_MENU_TYPE, NULL);
        g_assert_true(G_IS_OBJECT(menu));
        gboolean finalized = FALSE;
        g_object_weak_ref(G_OBJECT(menu), destroyed, &finalized);
        GtkWidget *container = g_object_ref_sink(quick_settings_power_menu_get_widget(menu));
        g_assert_cmpuint(menu->revealer->len, ==, 4);
        GtkRevealer *revealer = g_ptr_array_index(menu->revealer, 0);
        gtk_revealer_set_reveal_child(revealer, TRUE);
        g_signal_emit_by_name(settings_fixture, "quick-settings-hidden");
        g_assert_false(gtk_revealer_get_reveal_child(revealer));
        gpointer old_menu = menu;
        g_object_unref(menu);
        g_assert_true(finalized);
        g_assert_cmpuint(g_signal_handlers_block_matched(settings_fixture,
            G_SIGNAL_MATCH_DATA, 0, 0, NULL, NULL, old_menu), ==, 0);
        /* Widgets can outlive the controller without retaining callbacks. */
        gtk_revealer_set_reveal_child(revealer, TRUE);
        g_signal_emit_by_name(settings_fixture, "quick-settings-hidden");
        g_object_unref(container);
    }
    g_object_unref(settings_fixture);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/power-menu/construct-update-destroy", lifecycle);
    return g_test_run();
}
