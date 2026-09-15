#include "../src/services/wireplumber_service.c"

static void setup_inventory(WirePlumberService *service) {
    service->om = wp_object_manager_new();
    service->db = g_hash_table_new(g_direct_hash, g_direct_equal);
    service->sinks = g_ptr_array_new(); service->sources = g_ptr_array_new();
    service->streams = g_ptr_array_new(); service->links = g_ptr_array_new();
}
static void destroy_inventory(WirePlumberService *service) {
    g_ptr_array_unref(service->sinks); g_ptr_array_unref(service->sources);
    g_ptr_array_unref(service->streams); g_ptr_array_unref(service->links);
    g_hash_table_unref(service->db); g_object_unref(service->om);
}
static void removed_source_without_sink(void) {
    WirePlumberService service = {0}; setup_inventory(&service);
    WirePlumberServiceNode *source = g_new0(WirePlumberServiceNode, 1);
    source->id = 42; source->type = WIRE_PLUMBER_SERVICE_TYPE_SOURCE;
    g_ptr_array_add(service.sources, source);
    g_hash_table_insert(service.db, GUINT_TO_POINTER(source->id), source);
    wire_plumber_service_prune_db(&service);
    g_assert_cmpuint(service.sources->len, ==, 0);
    g_assert_cmpuint(g_hash_table_size(service.db), ==, 0);
    destroy_inventory(&service);
}
static void removed_default_is_cleared(void) {
    WirePlumberService service = {0}; setup_inventory(&service);
    WirePlumberServiceNode *sink = g_new0(WirePlumberServiceNode, 1);
    sink->id = 43; sink->type = WIRE_PLUMBER_SERVICE_TYPE_SINK;
    g_ptr_array_add(service.sinks, sink); service.default_sink = sink;
    g_hash_table_insert(service.db, GUINT_TO_POINTER(sink->id), sink);
    wire_plumber_service_prune_db(&service);
    g_assert_null(service.default_sink);
    destroy_inventory(&service);
}
static void copied_node_names_are_released(void) {
    WirePlumberServiceNode node = { .name = g_strdup("description"),
        .proper_name = g_strdup("node.name"), .media_class = g_strdup("Audio/Sink"),
        .nick_name = g_strdup("nickname") };
    wire_plumber_service_clean_source_sink_node(&node);
    g_assert_null(node.name); g_assert_null(node.proper_name);
    g_assert_null(node.media_class); g_assert_null(node.nick_name);
    WirePlumberServiceAudioStream stream = { .name = g_strdup("stream"),
        .media_name = g_strdup("playing"), .media_class = g_strdup("Stream/Output/Audio"),
        .app_name = g_strdup("application") };
    wire_plumber_service_clean_audio_node(&stream);
    g_assert_null(stream.name); g_assert_null(stream.media_name);
    g_assert_null(stream.media_class); g_assert_null(stream.app_name);
}
static void mixer_step_and_missing_values(void) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "volume", g_variant_new_double(.125));
    g_variant_builder_add(&builder, "{sv}", "mute", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&builder, "{sv}", "step", g_variant_new_double(.01));
    g_variant_builder_add(&builder, "{sv}", "base", g_variant_new_double(.75));
    g_autoptr(GVariant) values = g_variant_ref_sink(g_variant_builder_end(&builder));
    gdouble volume = -1, step = -1, base = -1; gboolean mute = FALSE;
    read_mixer_values(values, &volume, &mute, &step, &base);
    g_assert_cmpfloat(volume, ==, .125); g_assert_true(mute);
    g_assert_cmpfloat(step, ==, .01); g_assert_cmpfloat(base, ==, .75);
    read_mixer_values(NULL, &volume, &mute, &step, &base);
    g_assert_cmpfloat(volume, ==, 0); g_assert_false(mute);
    g_assert_cmpfloat(step, ==, 0); g_assert_cmpfloat(base, ==, 1);
}
typedef struct { GObject parent_instance; } FixtureMixer;
typedef struct { GObjectClass parent_class; } FixtureMixerClass;
G_DEFINE_TYPE(FixtureMixer, fixture_mixer, G_TYPE_OBJECT)
static void fixture_mixer_class_init(FixtureMixerClass *klass) {
    g_signal_new("set-volume", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_BOOLEAN, 2, G_TYPE_UINT, G_TYPE_VARIANT);
}
static void fixture_mixer_init(FixtureMixer *self) {}
static unsigned mute_requests, volume_requests;
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
    WirePlumberService service = { .mixer_api = (WpPlugin *)mixer };
    struct { WirePlumberServiceAudioStream stream; gdouble canary; } record = {
      .stream = {.type=WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM, .id=42, .volume=.7}, .canary=.91 };
    mute_requests = volume_requests = 0;
    wire_plumber_service_volume_mute(&service, (WirePlumberServiceNode *)&record.stream);
    g_assert_cmpfloat(record.canary, ==, .91);
    wire_plumber_service_volume_unmute(&service, (WirePlumberServiceNode *)&record.stream);
    g_assert_cmpfloat(record.canary, ==, .91);
    g_assert_cmpuint(mute_requests, ==, 2);
    g_assert_cmpuint(volume_requests, ==, 0);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    wp_init(WP_INIT_PIPEWIRE);
    g_test_add_func("/audio-inventory/source-without-sink", removed_source_without_sink);
    g_test_add_func("/audio-inventory/removed-default", removed_default_is_cleared);
    g_test_add_func("/audio-inventory/copied-names", copied_node_names_are_released);
    g_test_add_func("/audio-inventory/mixer-values", mixer_step_and_missing_values);
    g_test_add_func("/audio-inventory/stream-mute-bounds", stream_mute_keeps_record_bounds);
    return g_test_run();
}
