/* Temporary C volume/PulseAudio controls. Rust owns the audio inventory,
 * WirePlumber connection, plugins and C-compatible records/signals. */
#include "wireplumber_service.h"
#include <pulse/pulseaudio.h>
#include <pulse/glib-mainloop.h>

extern int way_shell_audio_global_init(void);
extern gboolean way_shell_audio_available(WirePlumberService *self);
/* Full reference; release after the call. */
extern GObject *way_shell_audio_ref_mixer(WirePlumberService *self);

typedef struct _Routing Routing;
typedef struct {
    Routing *routing;
    guint32 stream_id, target_id;
    gchar *target_name;
    pa_operation *query;
    gboolean matched;
} RouteRequest;
struct _Routing {
    GWeakRef owner;
    pa_context *pa_ctx;
    pa_glib_mainloop *pa_loop;
    GPtrArray *requests;
};
static void route_request_free(gpointer data) {
    RouteRequest *request = data;
    if (request->query) {
        pa_operation_cancel(request->query);
        pa_operation_unref(request->query);
    }
    g_free(request->target_name);
    g_free(request);
}
static void route_request_finished(RouteRequest *request) {
    if (!request) return;
    /* The end callback is already running: unref without cancelling it. */
    pa_operation *operation = request->query;
    request->query = NULL;
    if (operation) pa_operation_unref(operation);
    g_ptr_array_remove(request->routing->requests, request);
}
static void routing_cancel_requests(Routing *routing) {
    g_clear_pointer(&routing->requests, g_ptr_array_unref);
}
static void routing_free(gpointer data) {
    Routing *routing = data;
    routing_cancel_requests(routing);
    if (routing->pa_ctx) {
        pa_context_set_state_callback(routing->pa_ctx, NULL, NULL);
        pa_context_disconnect(routing->pa_ctx);
        pa_context_unref(routing->pa_ctx);
    }
    if (routing->pa_loop) pa_glib_mainloop_free(routing->pa_loop);
    g_weak_ref_clear(&routing->owner);
    g_free(routing);
}
static void routing_state(pa_context *context, void *data) {
    pa_context_state_t state = pa_context_get_state(context);
    /* Context teardown cancels operations without invoking their query
     * callbacks, so release their owned callback data here as well. */
    if (state == PA_CONTEXT_FAILED || state == PA_CONTEXT_TERMINATED)
        routing_cancel_requests(data);
    if (state == PA_CONTEXT_FAILED)
        g_message("Stream routing is unavailable: %s", pa_strerror(pa_context_errno(context)));
}
static void routing_available(WirePlumberService *self, gboolean available, gpointer data) {
    Routing *routing = g_object_get_data(G_OBJECT(self), "way-shell-pulse-routing");
    if (!available) { g_object_set_data(G_OBJECT(self), "way-shell-pulse-routing", NULL); return; }
    if (routing) return;
    routing = g_new0(Routing, 1);
    routing->requests = g_ptr_array_new_with_free_func(route_request_free);
    g_weak_ref_init(&routing->owner, self);
    routing->pa_loop = pa_glib_mainloop_new(g_main_context_default());
    routing->pa_ctx = pa_context_new(pa_glib_mainloop_get_api(routing->pa_loop), "org.ldelossa.way-shell");
    g_object_set_data_full(G_OBJECT(self), "way-shell-pulse-routing", routing, routing_free);
    pa_context_set_state_callback(routing->pa_ctx, routing_state, routing);
    if (pa_context_connect(routing->pa_ctx, NULL, PA_CONTEXT_NOFLAGS, NULL) < 0)
        g_message("Stream routing connection failed: %s", pa_strerror(pa_context_errno(routing->pa_ctx)));
}
int wire_plumber_service_global_init(void) {
    if (way_shell_audio_global_init() != 0) return -1;
    WirePlumberService *self = wire_plumber_service_get_global();
    g_signal_connect_object(self, "availability-changed", G_CALLBACK(routing_available), G_OBJECT(self), 0);
    routing_available(self, way_shell_audio_available(self), NULL);
    return 0;
}

static const char *route_target(RouteRequest *request, const pa_proplist *properties) {
    if (!properties) return NULL;
    const char *value = pa_proplist_gets(properties, "object.id");
    if (!value || request->matched || g_ascii_strtoull(value, NULL, 10) != request->stream_id) return NULL;
    g_autoptr(WirePlumberService) self = g_weak_ref_get(&request->routing->owner);
    if (!self) return NULL;
    WirePlumberServiceNode *node = g_hash_table_lookup(wire_plumber_service_get_db(self), GUINT_TO_POINTER(request->target_id));
    if (!node || g_strcmp0(node->proper_name, request->target_name) != 0) return NULL;
    request->matched = TRUE;
    return request->target_name;
}
void sink_input_info_cb(pa_context *context, const pa_sink_input_info *info, int eol, void *data) {
    RouteRequest *request = data;
    if (!request) return;
    if (eol != 0 || !info) { route_request_finished(request); return; }
    const char *target = route_target(request, info->proplist);
    if (!target) return;
    pa_operation *operation = pa_context_move_sink_input_by_name(context, info->index, target, NULL, NULL);
    if (operation) pa_operation_unref(operation);
}
void source_output_info_cb(pa_context *context, const pa_source_output_info *info, int eol, void *data) {
    RouteRequest *request = data;
    if (!request) return;
    if (eol != 0 || !info) { route_request_finished(request); return; }
    const char *target = route_target(request, info->proplist);
    if (!target) return;
    pa_operation *operation = pa_context_move_source_output_by_name(context, info->index, target, NULL, NULL);
    if (operation) pa_operation_unref(operation);
}

void wire_plumber_service_set_link(WirePlumberService *self,
                                   WirePlumberServiceNodeHeader *output,
                                   WirePlumberServiceNodeHeader *input) {
    g_debug("wireplumber_service.c:wire_plumber_service_set_link() called");

    Routing *routing = g_object_get_data(G_OBJECT(self), "way-shell-pulse-routing");
    if (!routing || pa_context_get_state(routing->pa_ctx) != PA_CONTEXT_READY || !output || !input) {
        g_message("Stream routing is not ready"); return;
    }
    // determine which one is our stream
    WirePlumberServiceNode *node = NULL;
    WirePlumberServiceAudioStream *stream = NULL;

    switch (output->type) {
        case WIRE_PLUMBER_SERVICE_TYPE_SINK:
        case WIRE_PLUMBER_SERVICE_TYPE_SOURCE:
            node = (WirePlumberServiceNode *)output;
            break;
        case WIRE_PLUMBER_SERVICE_TYPE_INPUT_AUDIO_STREAM:
        case WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM:
            stream = (WirePlumberServiceAudioStream *)output;
            break;
        default:
            break;
    }

    switch (input->type) {
        case WIRE_PLUMBER_SERVICE_TYPE_SINK:
        case WIRE_PLUMBER_SERVICE_TYPE_SOURCE:
            node = (WirePlumberServiceNode *)input;
            break;
        case WIRE_PLUMBER_SERVICE_TYPE_INPUT_AUDIO_STREAM:
        case WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM:
            stream = (WirePlumberServiceAudioStream *)input;
            break;
        default:
            break;
    }

    if (!node || !stream) return;

    if (!routing->requests || !node->proper_name) return;
    if ((stream->type == WIRE_PLUMBER_SERVICE_TYPE_INPUT_AUDIO_STREAM && node->type != WIRE_PLUMBER_SERVICE_TYPE_SOURCE) ||
        (stream->type == WIRE_PLUMBER_SERVICE_TYPE_OUTPUT_AUDIO_STREAM && node->type != WIRE_PLUMBER_SERVICE_TYPE_SINK)) return;
    RouteRequest *request = g_new0(RouteRequest, 1);
    request->routing = routing;
    request->stream_id = stream->id;
    request->target_id = node->id;
    request->target_name = g_strdup(node->proper_name);
    g_ptr_array_add(routing->requests, request);
    if (stream->type == WIRE_PLUMBER_SERVICE_TYPE_INPUT_AUDIO_STREAM)
        request->query = pa_context_get_source_output_info_list(routing->pa_ctx, source_output_info_cb, request);
    else
        request->query = pa_context_get_sink_input_info_list(routing->pa_ctx, sink_input_info_cb, request);
    if (!request->query) g_ptr_array_remove(routing->requests, request);
}


void wire_plumber_service_set_volume(WirePlumberService *self,
                                     const WirePlumberServiceNode *node,
                                     double volume) {
    g_debug(
        "wireplumber_service.c:wire_plumber_service_set_volume() called: %f",
        volume);

    if (!node) return;
    g_autoptr(GObject) mixer = way_shell_audio_ref_mixer(self);
    if (!mixer) return;

    g_auto(GVariantBuilder) b = G_VARIANT_BUILDER_INIT(G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) variant = NULL;
    gboolean res = FALSE;

    // our volume is in a linear scale for ease of use, but mixer-api wants it
    // in a cubic scale, to take the cube of our linear volume before encoding
    g_variant_builder_add(
        &b, "{sv}", "volume",
        g_variant_new_double(volume_to_linear(volume, SCALE_CUBIC)));
    variant = g_variant_ref_sink(g_variant_builder_end(&b));

    g_signal_emit_by_name(mixer, "set-volume", node->id, variant,
                          &res);
    g_debug(
        "wireplumber_service.c:wire_plumber_service_set_volume() id: %d, "
        "res: %d",
        node->id, res);
}

void wire_plumber_service_volume_up(WirePlumberService *self,
                                    const WirePlumberServiceNode *node) {
    g_debug("wireplumber_service.c:wire_plumber_service_volume_up() called");

    if (!node) return;
    g_autoptr(GObject) mixer = way_shell_audio_ref_mixer(self);
    if (!mixer) return;

    if (node->volume >= 1.0) {
        g_debug(
            "wireplumber_service.c:wire_plumber_service_volume_up() volume is "
            "already at max");
        return;
    }
    double volume = MIN(node->volume + .05, 1.0);
    g_debug("wireplumber_service.c:wire_plumber_service_volume_up() volume: %f",
            volume);
    wire_plumber_service_set_volume(self, node, volume);
}

void wire_plumber_service_volume_down(WirePlumberService *self,
                                      const WirePlumberServiceNode *node) {
    g_debug("wireplumber_service.c:wire_plumber_service_volume_down() called");

    if (!node) return;
    g_autoptr(GObject) mixer = way_shell_audio_ref_mixer(self);
    if (!mixer) return;

    if (node->volume <= 0.0) {
        g_debug(
            "wireplumber_service.c:wire_plumber_service_volume_down() volume "
            "is already at min");
        return;
    }
    double volume = node->volume - .05;
    g_debug(
        "wireplumber_service.c:wire_plumber_service_volume_down() volume: %f",
        volume);
    wire_plumber_service_set_volume(self, node, volume);
}

void wire_plumber_service_volume_mute(WirePlumberService *self,
                                      WirePlumberServiceNode *node) {
    gboolean res = FALSE;

    if (!node) return;
    g_autoptr(GObject) mixer = way_shell_audio_ref_mixer(self);
    if (!mixer) return;

    g_debug("wireplumber_service.c:wire_plumber_service_volume_mute() called");

    g_auto(GVariantBuilder) b = G_VARIANT_BUILDER_INIT(G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) variant = NULL;

    g_variant_builder_add(&b, "{sv}", "mute", g_variant_new_boolean(true));
    variant = g_variant_ref_sink(g_variant_builder_end(&b));

    g_signal_emit_by_name(mixer, "set-volume", node->id, variant,
                          &res);

    g_debug(
        "wireplumber_service.c:wire_plumber_service_volume_mute() id: %d, res: "
        "%d",
        node->id, res);
}

void wire_plumber_service_volume_unmute(WirePlumberService *self,
                                        const WirePlumberServiceNode *node) {
    g_debug(
        "wireplumber_service.c:wire_plumber_service_volume_unmute() called");

    if (!node) return;
    g_autoptr(GObject) mixer = way_shell_audio_ref_mixer(self);
    if (!mixer) return;

    gboolean res = FALSE;

    g_debug("wireplumber_service.c:wire_plumber_service_volume_mute() called");

    g_auto(GVariantBuilder) b = G_VARIANT_BUILDER_INIT(G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) variant = NULL;

    g_variant_builder_add(&b, "{sv}", "mute", g_variant_new_boolean(false));
    variant = g_variant_ref_sink(g_variant_builder_end(&b));

    g_signal_emit_by_name(mixer, "set-volume", node->id, variant,
                          &res);

    g_debug(
        "wireplumber_service.c:wire_plumber_service_volume_mute() id: %d, res: "
        "%d",
        node->id, res);
}

char *wire_plumber_service_map_source_vol_icon(float vol, gboolean mute) {
    if (mute) {
        return "microphone-sensitivity-muted-symbolic";
    }
    if (vol < 0.25) {
        return "microphone-sensitivity-low-symbolic";
    }
    if (vol >= 0.25 && vol < 0.5) {
        return "microphone-sensitivity-medium-symbolic";
    }
    return "microphone-sensitivity-high-symbolic";
}

char *wire_plumber_service_map_sink_vol_icon(float vol, gboolean mute) {
    if (mute) {
        return "audio-volume-muted-symbolic";
    }
    if (vol < 0.25) {
        return "audio-volume-low-symbolic";
    }
    if (vol >= 0.25 && vol < 0.5) {
        return "audio-volume-medium-symbolic";
    }
    return "audio-volume-high-symbolic";
}
