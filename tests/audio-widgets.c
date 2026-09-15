#include <adwaita.h>
#include "../src/panel/quick_settings/quick_settings_scales/quick_settings_scales.c"
typedef struct { GObject parent_instance; } FixtureAudio;
typedef struct { GObjectClass parent_class; } FixtureAudioClass;
G_DEFINE_TYPE(FixtureAudio, fixture_audio, G_TYPE_OBJECT)
static void fixture_audio_class_init(FixtureAudioClass *klass) {
    GType type = G_TYPE_FROM_CLASS(klass);
    for (unsigned i=0; i<2; i++)
        g_signal_new(i ? "default-source-changed" : "default-sink-changed", type,
          G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);
    g_signal_new("brightness-changed", type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_FLOAT);
    g_signal_new("availability-changed", type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
    g_signal_new("operation-failed", type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_STRING);
}
static void fixture_audio_init(FixtureAudio *self) {}
static GObject *fixture;
static WirePlumberServiceNode *sink, *source;
static unsigned requests;
WirePlumberService *wire_plumber_service_get_global(void) { return (WirePlumberService *)fixture; }
WirePlumberServiceNode *wire_plumber_service_get_default_sink(WirePlumberService *s) { return sink; }
WirePlumberServiceNode *wire_plumber_service_get_default_source(WirePlumberService *s) { return source; }
void wire_plumber_service_set_volume(WirePlumberService *s, const WirePlumberServiceNode *n, double v) { requests++; }
void wire_plumber_service_volume_mute(WirePlumberService *s, WirePlumberServiceNode *n) { requests++; }
void wire_plumber_service_volume_unmute(WirePlumberService *s, const WirePlumberServiceNode *n) { requests++; }
char *wire_plumber_service_map_sink_vol_icon(float v, gboolean m) { return "audio-volume-high-symbolic"; }
char *wire_plumber_service_map_source_vol_icon(float v, gboolean m) { return "microphone-sensitivity-high-symbolic"; }
BrightnessService *brightness_service_get_global(void) { return (BrightnessService *)fixture; }
gboolean brightness_service_has_backlight_brightness(BrightnessService *s) { return FALSE; }
float brightness_service_get_backlight(BrightnessService *s) { return 0; }
void brightness_service_set_backlight(BrightnessService *s, float value) {}
char *brightness_service_map_icon(BrightnessService *s) { return "display-brightness-symbolic"; }
static void lifecycle(void) {
    fixture = g_object_new(fixture_audio_get_type(), NULL);
    WirePlumberServiceNode output = {.id=1, .volume=.4}, input = {.id=2, .volume=.7, .state=WP_NODE_STATE_RUNNING};
    sink = &output; source = &input;
    QuickSettingsScales *scales = g_object_new(QUICK_SETTINGS_SCALES_TYPE, NULL);
    GtkWidget *container = g_object_ref_sink(quick_settings_scales_get_widget(scales));
    g_assert_true(scales->active_source_node == source);
    g_assert_cmpfloat(gtk_range_get_value(GTK_RANGE(scales->default_source_scale)), ==, .7);
    sink = source = NULL;
    g_signal_emit_by_name(fixture, "default-source-changed", NULL);
    g_signal_emit_by_name(fixture, "default-sink-changed", NULL);
    g_assert_null(scales->active_source_node); g_assert_null(scales->default_sink_node);
    g_assert_false(gtk_widget_get_visible(GTK_WIDGET(scales->default_source_container)));
    g_assert_false(gtk_widget_get_sensitive(GTK_WIDGET(scales->default_sink_container)));
    g_signal_emit_by_name(scales->default_sink_button, "clicked");
    g_signal_emit_by_name(scales->default_source_button, "clicked");
    g_assert_cmpuint(requests, ==, 0);
    sink = &output; source = &input;
    g_signal_emit_by_name(fixture, "default-source-changed", source);
    g_signal_emit_by_name(fixture, "default-sink-changed", sink);
    g_assert_true(gtk_widget_get_sensitive(GTK_WIDGET(scales->default_sink_container)));
    g_assert_true(gtk_widget_get_visible(GTK_WIDGET(scales->default_source_container)));
    GtkScale *slider = scales->default_sink_scale; GtkButton *button = scales->default_sink_button;
    gpointer owner = scales; g_object_unref(scales);
    g_assert_cmpuint(g_signal_handlers_block_matched(fixture, G_SIGNAL_MATCH_DATA, 0, 0, NULL, NULL, owner), ==, 0);
    g_assert_cmpuint(g_signal_handlers_block_matched(slider, G_SIGNAL_MATCH_DATA, 0, 0, NULL, NULL, owner), ==, 0);
    gtk_range_set_value(GTK_RANGE(slider), .9); g_signal_emit_by_name(button, "clicked");
    g_assert_cmpuint(requests, ==, 0);
    g_object_unref(container); g_object_unref(fixture);
}
int main(int argc, char **argv) { g_test_init(&argc, &argv, NULL); gtk_init(); g_test_add_func("/audio-controls/removal-recovery-ownership", lifecycle); return g_test_run(); }
