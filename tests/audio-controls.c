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
static GHashTable *test_db;
GHashTable *wire_plumber_service_get_db(WirePlumberService *self) { return test_db; }

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
typedef struct { pa_sink_input_info_cb_t callback; void *data; unsigned cancelled, unrefs; } Query;
static Query queries[8]; static unsigned query_count, moves;
static char *destinations[8];
static pa_context_state_t test_context_state = PA_CONTEXT_READY;
pa_context_state_t pa_context_get_state(const pa_context *context) { return test_context_state; }
int pa_context_errno(const pa_context *context) { return PA_ERR_CONNECTIONTERMINATED; }
pa_operation *pa_context_get_sink_input_info_list(pa_context *context, pa_sink_input_info_cb_t callback, void *data) {
    Query *query = &queries[query_count++]; *query = (Query){.callback=callback, .data=data};
    return (pa_operation *)query;
}
pa_operation *pa_context_move_sink_input_by_name(pa_context *context, uint32_t index, const char *name, pa_context_success_cb_t callback, void *data) {
    destinations[moves++] = g_strdup(name); return NULL;
}
void pa_operation_unref(pa_operation *operation) { ((Query *)operation)->unrefs++; }
void pa_operation_cancel(pa_operation *operation) { ((Query *)operation)->cancelled++; }
static void concurrent_routes_keep_their_targets(void) {
    g_autoptr(GObject) owner = g_object_new(G_TYPE_OBJECT, NULL);
    Routing routing = {.pa_ctx=(pa_context *)owner, .requests=g_ptr_array_new_with_free_func(route_request_free)};
    g_weak_ref_init(&routing.owner, owner);
    g_object_set_data(owner, "way-shell-pulse-routing", &routing);
    WirePlumberServiceNode a = {.type=WIRE_PLUMBER_SERVICE_TYPE_SINK, .id=1, .proper_name="first-output"};
    WirePlumberServiceNode b = {.type=WIRE_PLUMBER_SERVICE_TYPE_SINK, .id=2, .proper_name="second-output"};
    WirePlumberServiceAudioStream first = {.type=WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM, .id=10};
    WirePlumberServiceAudioStream second = {.type=WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM, .id=20};
    test_db = g_hash_table_new(g_direct_hash, g_direct_equal);
    g_hash_table_insert(test_db, GUINT_TO_POINTER(a.id), &a);
    g_hash_table_insert(test_db, GUINT_TO_POINTER(b.id), &b);
    query_count = moves = 0; test_context_state = PA_CONTEXT_READY;
    wire_plumber_service_set_link((WirePlumberService *)owner, (WirePlumberServiceNodeHeader *)&first, (WirePlumberServiceNodeHeader *)&a);
    wire_plumber_service_set_link((WirePlumberService *)owner, (WirePlumberServiceNodeHeader *)&second, (WirePlumberServiceNodeHeader *)&b);
    g_assert_cmpuint(query_count, ==, 2);
    pa_sink_input_info missing_properties = {.index=100};
    queries[0].callback(routing.pa_ctx, &missing_properties, 0, queries[0].data);
    g_assert_cmpuint(moves, ==, 0);
    pa_proplist *properties = pa_proplist_new();
    pa_sink_input_info info = {.index=100, .proplist=properties};
    pa_proplist_sets(properties, "object.id", "10");
    queries[0].callback(routing.pa_ctx, &info, 0, queries[0].data);
    queries[0].callback(routing.pa_ctx, NULL, 1, queries[0].data);
    pa_proplist_sets(properties, "object.id", "20"); info.index=200;
    queries[1].callback(routing.pa_ctx, &info, 0, queries[1].data);
    queries[1].callback(routing.pa_ctx, NULL, 1, queries[1].data);
    g_assert_cmpuint(moves, ==, 2);
    g_assert_cmpstr(destinations[0], ==, "first-output");
    g_assert_cmpstr(destinations[1], ==, "second-output");
    for (unsigned i=0; i<moves; i++) g_free(destinations[i]);
    g_assert_cmpuint(routing.requests->len, ==, 0);
    g_assert_cmpuint(queries[0].cancelled, ==, 0);
    g_assert_cmpuint(queries[0].unrefs, ==, 1);
    /* A pending request is cancelled before its callback data is released. */
    wire_plumber_service_set_link((WirePlumberService *)owner, (WirePlumberServiceNodeHeader *)&first, (WirePlumberServiceNodeHeader *)&a);
    g_assert_cmpuint(routing.requests->len, ==, 1);
    routing_cancel_requests(&routing);
    g_assert_cmpuint(queries[2].cancelled, ==, 1);
    g_assert_cmpuint(queries[2].unrefs, ==, 1);
    pa_proplist_free(properties); g_hash_table_unref(test_db); test_db=NULL;
    g_weak_ref_clear(&routing.owner);
}
static void disconnected_pulse_cancels_queries(void) {
    g_autoptr(GObject) owner = g_object_new(G_TYPE_OBJECT, NULL);
    Routing routing = {.pa_ctx=(pa_context *)owner};
    g_weak_ref_init(&routing.owner, owner);
    g_object_set_data(owner, "way-shell-pulse-routing", &routing);
    WirePlumberServiceNode sink = {.type=WIRE_PLUMBER_SERVICE_TYPE_SINK, .id=1, .proper_name="output"};
    WirePlumberServiceAudioStream stream = {.type=WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM, .id=10};
    query_count = 0;
    const pa_context_state_t terminal_states[] = {PA_CONTEXT_FAILED, PA_CONTEXT_TERMINATED};
    for (unsigned i=0; i<G_N_ELEMENTS(terminal_states); i++) {
        test_context_state = PA_CONTEXT_READY;
        routing.requests = g_ptr_array_new_with_free_func(route_request_free);
        wire_plumber_service_set_link((WirePlumberService *)owner, (WirePlumberServiceNodeHeader *)&stream, (WirePlumberServiceNodeHeader *)&sink);
        g_assert_cmpuint(routing.requests->len, ==, 1);
        test_context_state = terminal_states[i];
        if (test_context_state == PA_CONTEXT_FAILED)
            g_test_expect_message(NULL, G_LOG_LEVEL_MESSAGE, "Stream routing is unavailable: *");
        routing_state(routing.pa_ctx, &routing);
        if (test_context_state == PA_CONTEXT_FAILED) g_test_assert_expected_messages();
        g_assert_null(routing.requests);
        g_assert_cmpuint(queries[i].cancelled, ==, 1);
        g_assert_cmpuint(queries[i].unrefs, ==, 1);
    }
    test_context_state = PA_CONTEXT_READY;
    g_weak_ref_clear(&routing.owner);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/audio-controls/stream-mute-bounds", stream_mute_keeps_record_bounds);
    g_test_add_func("/audio-controls/failed-pulse-query", failed_pulse_queries_are_ignored);
    g_test_add_func("/audio-controls/concurrent-routes", concurrent_routes_keep_their_targets);
    g_test_add_func("/audio-controls/disconnected-pulse-query", disconnected_pulse_cancels_queries);
    return g_test_run();
}
