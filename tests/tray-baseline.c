#include <adwaita.h>
#include <stdint.h>

#include "../src/services/status_notifier_service/status_notifier_service.h"
#include "../src/services/status_notifier_service/status_notifier_watcher_dbus.h"

/* Keep the production registration and removal paths, while pausing item
 * discovery at its asynchronous D-Bus boundary. Rust's private-bus fixtures
 * exercise that boundary when discovery migrates. No live session is used. */
typedef struct {
    gchar *bus;
    gchar *path;
    gpointer item;
} PendingItem;
static GPtrArray *pending;
static const gchar *sender = ":1.42";
static guint completed;
static guint owned_name;
static GObject *bus_service;
static guint live_pixel_buffers;

static const gchar *fixture_sender(GDBusMethodInvocation *invocation) {
    return sender;
}
static void fixture_complete(DbusWatcherV0Gen *watcher,
                             GDBusMethodInvocation *invocation) {
    completed++;
}
static void fixture_proxy_new(GDBusConnection *connection,
                              GDBusProxyFlags flags, const gchar *name,
                              const gchar *path, GCancellable *cancellable,
                              GAsyncReadyCallback callback, gpointer data) {
    PendingItem *request = g_new0(PendingItem, 1);
    request->bus = g_strdup(name);
    request->path = g_strdup(path);
    request->item = data;
    g_ptr_array_add(pending, request);
}
static guint fixture_own_name(GDBusConnection *connection, const gchar *name,
                              GBusNameOwnerFlags flags,
                              GBusNameAcquiredCallback acquired,
                              GBusNameLostCallback lost, gpointer data,
                              GDestroyNotify destroy) {
    return ++owned_name;
}

typedef struct {
    GdkPixbufDestroyNotify destroy;
    gpointer data;
} PixelOwner;
static void release_pixels(guchar *pixels, gpointer data) {
    PixelOwner *owner = data;
    if (owner->destroy) {
        owner->destroy(pixels, owner->data);
        live_pixel_buffers--;
    } else {
        /* Clean up the production leak after recording it. */
        g_free(pixels);
    }
    g_free(owner);
}
static GdkPixbuf *fixture_pixbuf_new(const guchar *pixels,
                                    GdkColorspace colorspace, gboolean alpha,
                                    int bits, int width, int height, int stride,
                                    GdkPixbufDestroyNotify destroy,
                                    gpointer data) {
    PixelOwner *owner = g_new0(PixelOwner, 1);
    owner->destroy = destroy;
    owner->data = data;
    live_pixel_buffers++;
    return gdk_pixbuf_new_from_data(pixels, colorspace, alpha, bits, width,
                                    height, stride, release_pixels, owner);
}

#define g_dbus_method_invocation_get_sender fixture_sender
#define dbus_watcher_v0_gen_complete_register_item fixture_complete
#define dbus_item_v0_gen_proxy_new fixture_proxy_new
#define g_bus_own_name_on_connection fixture_own_name
#define gdk_pixbuf_new_from_data fixture_pixbuf_new
#include "../src/services/status_notifier_service/status_notifier_service.c"
#undef g_dbus_method_invocation_get_sender
#undef dbus_watcher_v0_gen_complete_register_item
#undef dbus_item_v0_gen_proxy_new
#undef g_bus_own_name_on_connection
#undef gdk_pixbuf_new_from_data

typedef struct { GObject parent_instance; } FixtureBus;
typedef struct { GObjectClass parent_class; } FixtureBusClass;
G_DEFINE_TYPE(FixtureBus, fixture_bus, G_TYPE_OBJECT)
static void fixture_bus_class_init(FixtureBusClass *klass) {
    g_signal_new("dbus-session-name-lost", G_TYPE_FROM_CLASS(klass),
                 G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 2,
                 G_TYPE_HASH_TABLE, G_TYPE_STRING);
}
static void fixture_bus_init(FixtureBus *self) {}
DBUSService *dbus_service_get_global(void) { return (DBUSService *)bus_service; }
GDBusConnection *dbus_service_get_session_bus(DBUSService *self) { return NULL; }

static GVariant *pixmap(gint width, gint height, const guint8 *bytes, gsize size) {
    return g_variant_new("(ii@ay)", width, height,
        g_variant_new_fixed_array(G_VARIANT_TYPE_BYTE, bytes, size, 1));
}
static GVariant *icons(GVariant *first, GVariant *second) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a(iiay)"));
    if (first) g_variant_builder_add_value(&builder, first);
    if (second) g_variant_builder_add_value(&builder, second);
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}
static void pixmap_selects_largest_and_copies_argb(void) {
    const guint8 small[] = {0xff, 0x01, 0x02, 0x03};
    const guint8 large[] = {0x80, 0x11, 0x22, 0x33, 0x40, 0x44, 0x55, 0x66};
    const guint8 rgba[] = {0x11, 0x22, 0x33, 0x80, 0x44, 0x55, 0x66, 0x40};
    GVariant *data = icons(pixmap(1, 1, small, sizeof small),
                           pixmap(2, 1, large, sizeof large));
    GdkPixbuf *image = pixbuf_from_icon_data(data);
    g_variant_unref(data);
    g_assert_nonnull(image);
    g_assert_cmpint(gdk_pixbuf_get_width(image), ==, 2);
    g_assert_cmpint(gdk_pixbuf_get_height(image), ==, 1);
    g_assert_cmpmem(gdk_pixbuf_get_pixels(image), sizeof rgba, rgba, sizeof rgba);
    g_object_unref(image);
}
static void pixmap_rejects_bad_lengths_and_dimensions(void) {
    const guint8 bytes[] = {0xff, 1, 2, 3};
    GVariant *data = icons(pixmap(2, 1, bytes, sizeof bytes),
                           pixmap(-1, 1, bytes, sizeof bytes));
    g_assert_null(pixbuf_from_icon_data(data));
    g_variant_unref(data);
    data = icons(pixmap(0, 1, bytes, sizeof bytes),
                  pixmap(1, 1, bytes, sizeof bytes - 1));
    g_assert_null(pixbuf_from_icon_data(data));
    g_variant_unref(data);
    g_assert_null(pixbuf_from_icon_data(NULL));
    data = g_variant_ref_sink(g_variant_new_string("invalid pixmap type"));
    g_assert_null(pixbuf_from_icon_data(data));
    g_variant_unref(data);

    /* A malformed larger candidate must not hide the valid fallback. */
    data = icons(pixmap(1, 1, bytes, sizeof bytes),
                  pixmap(2, 2, bytes, sizeof bytes));
    GdkPixbuf *image = pixbuf_from_icon_data(data);
    g_assert_nonnull(image);
    g_assert_cmpint(gdk_pixbuf_get_width(image), ==, 1);
    g_object_unref(image);
    g_variant_unref(data);
}
static void pixmap_rejects_overflow(void) {
    GVariant *data = icons(pixmap(G_MAXINT, 2, NULL, 0), NULL);
    g_assert_null(pixbuf_from_icon_data(data));
    g_variant_unref(data);
    /* The four-byte size expression wraps to 16 on 32-bit arithmetic. */
    const guint8 bytes[16] = {0};
    data = icons(pixmap(1073741825, 4, bytes, sizeof bytes), NULL);
    GdkPixbuf *image = pixbuf_from_icon_data(data);
    g_assert_null(image);
    g_variant_unref(data);
}
static void pixmap_releases_owned_pixels(void) {
    const guint8 bytes[] = {0xff, 1, 2, 3};
    live_pixel_buffers = 0;
    GVariant *data = icons(pixmap(1, 1, bytes, sizeof bytes), NULL);
    GdkPixbuf *image = pixbuf_from_icon_data(data);
    g_assert_nonnull(image);
    g_variant_unref(data);
    g_object_unref(image);
    g_assert_cmpuint(live_pixel_buffers, ==, 0);
}
static void initial_overlay_uses_its_own_pixels(void) {
    const guint8 main_icon[] = {0xff, 1, 2, 3};
    const guint8 overlay[] = {0x80, 4, 5, 6};
    const guint8 expected[] = {4, 5, 6, 0x80};
    DbusItemV0Gen *proxy = g_object_new(DBUS_TYPE_ITEM_V0_GEN_PROXY,
        "g-name", sender, "g-object-path", "/Item", NULL);
    GVariant *main_data = icons(pixmap(1, 1, main_icon, sizeof main_icon), NULL);
    GVariant *overlay_data = icons(pixmap(1, 1, overlay, sizeof overlay), NULL);
    g_dbus_proxy_set_cached_property(G_DBUS_PROXY(proxy), "IconPixmap", main_data);
    g_dbus_proxy_set_cached_property(G_DBUS_PROXY(proxy), "OverlayIconPixmap", overlay_data);
    StatusNotifierItem *item = g_new0(StatusNotifierItem, 1);
    status_notifier_item_init(item, proxy);
    g_variant_unref(main_data);
    g_variant_unref(overlay_data);
    g_assert_nonnull(item->overlay_icon_pixmap);
    g_assert_cmpmem(gdk_pixbuf_get_pixels(item->overlay_icon_pixmap), sizeof expected,
                    expected, sizeof expected);
    status_notifier_item_free(item);
}

static void pending_free(gpointer data) {
    PendingItem *request = data;
    g_free(request->bus);
    g_free(request->path);
    g_free(request);
}
static StatusNotifierService *registry_new(void) {
    pending = g_ptr_array_new_with_free_func(pending_free);
    bus_service = g_object_new(fixture_bus_get_type(), NULL);
    completed = 0;
    global = g_object_new(status_notifier_service_get_type(), NULL);
    return global;
}
static void registry_free(StatusNotifierService *service) {
    /* The C service currently has no destructor. Own fixture resources so the
     * registration tests do not obscure the behavior with unrelated leaks. */
    g_signal_handlers_disconnect_by_data(bus_service, service);
    g_hash_table_remove_all(service->items);
    for (guint i = 0; i < pending->len; i++) {
        PendingItem *request = g_ptr_array_index(pending, i);
        if (request->item) status_notifier_item_free(request->item);
    }
    g_clear_pointer(&service->items, g_hash_table_unref);
    g_clear_object(&service->host);
    g_clear_object(&service->watcher);
    g_object_unref(service);
    g_clear_object(&bus_service);
    g_clear_pointer(&pending, g_ptr_array_unref);
    global = NULL;
}
static void register_item(StatusNotifierService *service, const gchar *name) {
    on_handle_register_item(service->watcher, NULL, (gchar *)name, service);
}
static void registration_routes_both_forms(void) {
    StatusNotifierService *service = registry_new();
    register_item(service, "org.example.Tray");
    register_item(service, "/OtherItem");
    g_assert_cmpuint(completed, ==, 2);
    g_assert_cmpuint(pending->len, ==, 2);
    PendingItem *by_name = g_ptr_array_index(pending, 0);
    PendingItem *by_path = g_ptr_array_index(pending, 1);
    g_assert_cmpstr(by_name->bus, ==, "org.example.Tray");
    g_assert_cmpstr(by_name->path, ==, "/StatusNotifierItem");
    g_assert_cmpstr(by_path->bus, ==, sender);
    g_assert_cmpstr(by_path->path, ==, "/OtherItem");
    registry_free(service);
}
static guint removed;
static void removed_while_record_alive(StatusNotifierService *service,
                                      GHashTable *items, StatusNotifierItem *item,
                                      gpointer data) {
    g_assert_cmpstr(item->bus_name, ==, sender);
    g_assert_cmpstr(item->register_service_name, ==, "/First");
    g_assert_true(g_hash_table_lookup(items, item->bus_name) == item);
    removed++;
}
static void removal_keeps_record_through_signal(void) {
    StatusNotifierService *service = registry_new();
    removed = 0;
    register_item(service, "/First");
    g_signal_connect(service, "status-notifier-item-removed",
                      G_CALLBACK(removed_while_record_alive), NULL);
    on_session_name_lost(NULL, NULL, (gchar *)sender, service);
    ((PendingItem *)g_ptr_array_index(pending, 0))->item = NULL;
    g_assert_cmpuint(removed, ==, 1);
    g_assert_cmpuint(g_hash_table_size(service->items), ==, 0);
    on_session_name_lost(NULL, NULL, (gchar *)sender, service);
    g_assert_cmpuint(removed, ==, 1);
    registry_free(service);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/tray/pixmap/largest-argb", pixmap_selects_largest_and_copies_argb);
    g_test_add_func("/tray/pixmap/invalid-length", pixmap_rejects_bad_lengths_and_dimensions);
    g_test_add_func("/tray/pixmap/overflow", pixmap_rejects_overflow);
    g_test_add_func("/tray/pixmap/ownership", pixmap_releases_owned_pixels);
    g_test_add_func("/tray/pixmap/overlay", initial_overlay_uses_its_own_pixels);
    g_test_add_func("/tray/registration/address-forms", registration_routes_both_forms);
    g_test_add_func("/tray/registration/removal-lifetime", removal_keeps_record_through_signal);
    return g_test_run();
}
