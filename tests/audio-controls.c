#include "../src/services/wireplumber_service.c"
#include <stddef.h>
_Static_assert(sizeof(WirePlumberServiceNode) == 136, "Rust device record ABI");
_Static_assert(sizeof(WirePlumberServiceAudioStream) == 128, "Rust stream record ABI");
_Static_assert(sizeof(WirePlumberServiceLink) == 24, "Rust link record ABI");
_Static_assert(offsetof(WirePlumberServiceNode, volume) == 88, "Rust volume offset");
typedef struct { GObject parent_instance; } FixtureMixer;
typedef struct { GObjectClass parent_class; } FixtureMixerClass;
G_DEFINE_TYPE(FixtureMixer, fixture_mixer, G_TYPE_OBJECT)
static void fixture_mixer_class_init(FixtureMixerClass *klass) {
    g_signal_new("set-volume", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_BOOLEAN, 2, G_TYPE_UINT, G_TYPE_VARIANT);
}
static void fixture_mixer_init(FixtureMixer *self) {}
static unsigned mute_requests, volume_requests;
static GObject *test_mixer;
GObject *way_shell_audio_ref_mixer(WirePlumberService *self) { return test_mixer ? g_object_ref(test_mixer) : NULL; }
GHashTable *wire_plumber_service_get_db(WirePlumberService *self) { return NULL; }

static gboolean mixer_request(GObject *object, guint id, GVariant *value, gpointer data) {
    gboolean mute; gdouble volume;
    if (g_variant_lookup(value, "mute", "b", &mute)) mute_requests++;
    if (g_variant_lookup(value, "volume", "d", &volume)) volume_requests++;
    return TRUE;
}
float volume_to_linear(double volume, int scale) { return volume * volume * volume; }
static void stream_mute_keeps_record_bounds(void) {
    g_autoptr(GObject) mixer = g_object_new(fixture_mixer_get_type(), NULL);
    g_signal_connect(mixer, "set-volume", G_CALLBACK(mixer_request), NULL);
    test_mixer = mixer;
    WirePlumberService *service = (WirePlumberService *)mixer;
    struct { WirePlumberServiceAudioStream stream; gdouble canary; } record = {
      .stream = {.type=WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM, .id=42, .volume=.7}, .canary=.91 };
    mute_requests = volume_requests = 0;
    wire_plumber_service_volume_mute(service, (WirePlumberServiceNode *)&record.stream);
    g_assert_cmpfloat(record.canary, ==, .91);
    wire_plumber_service_volume_unmute(service, (WirePlumberServiceNode *)&record.stream);
    g_assert_cmpfloat(record.canary, ==, .91);
    g_assert_cmpuint(mute_requests, ==, 2);
    g_assert_cmpuint(volume_requests, ==, 0);
}

static void failed_pulse_queries_are_ignored(void) {
    sink_input_info_cb(NULL, NULL, -1, NULL);
    source_output_info_cb(NULL, NULL, -1, NULL);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/audio-controls/stream-mute-bounds", stream_mute_keeps_record_bounds);
    g_test_add_func("/audio-controls/failed-pulse-query", failed_pulse_queries_are_ignored);
    return g_test_run();
}
