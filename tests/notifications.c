/* Request and image validation before replacing the C notification service.
 * Transport is captured locally; the real request/hint handlers run below. */
#include <adwaita.h>
#include <stdarg.h>

static GVariant *reply;
static void fixture_reply(GDBusMethodInvocation *invocation, GVariant *value) {
    g_clear_pointer(&reply, g_variant_unref);
    reply = g_variant_ref_sink(value);
}
static void fixture_emit(gpointer instance, guint signal, GQuark detail, ...) {}

#define g_dbus_method_invocation_return_value fixture_reply
#define g_signal_emit fixture_emit
#include "../src/services/notifications_service/notifications_service.c"
#undef g_dbus_method_invocation_return_value
#undef g_signal_emit

static NotificationsService fixture_service(void) {
    return (NotificationsService){
        .notifications = g_ptr_array_new(),
        .internal_ids = g_hash_table_new(g_direct_hash, g_direct_equal),
    };
}
static void clear_service(NotificationsService *service) {
    for (guint i = 0; i < service->notifications->len; i++)
        free_notification(g_ptr_array_index(service->notifications, i));
    g_ptr_array_unref(service->notifications);
    g_hash_table_unref(service->internal_ids);
    g_clear_pointer(&reply, g_variant_unref);
}
static GVariant *hint(const gchar *name, GVariant *value) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", name, value);
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}
static void empty_name_without_desktop_entry(void) {
    NotificationsService service = fixture_service();
    g_autoptr(GVariant) hints = g_variant_ref_sink(
        g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    const gchar *actions[] = {NULL};
    g_assert_true(on_handle_notify(NULL, NULL, "", 0, "", "Title", "Body",
                                  actions, hints, -1, &service));
    g_assert_cmpuint(service.notifications->len, ==, 1);
    Notification *notification = g_ptr_array_index(service.notifications, 0);
    g_assert_cmpstr(notification->app_name, ==, "");
    g_assert_cmpstr(notification->summary, ==, "Title");
    g_assert_nonnull(notification->created_on);
    clear_service(&service);
}
static void internal_creation_time(void) {
    NotificationsService service = fixture_service();
    Notification source = {.app_name = "Way-Shell", .summary = "Battery low",
                           .body = "Battery is at 5% power.",
                           .app_icon = "battery-level-0-symbolic", .urgency = 2};
    notifications_service_send_notification(&service, &source);
    g_assert_cmpuint(service.notifications->len, ==, 1);
    Notification *notification = g_ptr_array_index(service.notifications, 0);
    g_assert_nonnull(notification->created_on);
    g_assert_true(notification->is_internal);
    g_assert_cmpstr(notification->body, ==, source.body);
    g_assert_true(notification->body != source.body);
    clear_service(&service);
}
static void complete_action_pairs(void) {
    NotificationsService service = fixture_service();
    const gchar *actions[] = {"default", "Open", "reply", "Reply", NULL};
    g_assert_true(on_handle_notify(NULL, NULL, "Example", 0, "", "Title", "Body",
                                  actions, NULL, -1, &service));
    Notification *notification = g_ptr_array_index(service.notifications, 0);
    g_assert_cmpuint(g_strv_length(notification->actions), ==, 4);
    g_assert_cmpstr(notification->actions[2], ==, "reply");
    g_assert_true(notification->actions[2] != actions[2]);
    clear_service(&service);
}
static void incomplete_action_pair(void) {
    NotificationsService service = fixture_service();
    const gchar *actions[] = {"reply", "Reply", "dangling", NULL};
    g_assert_true(on_handle_notify(NULL, NULL, "Example", 0, "", "Title", "Body",
                                  actions, NULL, -1, &service));
    Notification *notification = g_ptr_array_index(service.notifications, 0);
    g_assert_cmpuint(g_strv_length(notification->actions), ==, 2);
    g_assert_cmpstr(notification->actions[0], ==, "reply");
    g_assert_cmpstr(notification->actions[1], ==, "Reply");
    clear_service(&service);
}
static GVariant *image(gint width, gint height, gint rowstride,
                       gboolean alpha, gint bits, gint channels, gsize length) {
    guchar bytes[64];
    for (gsize i = 0; i < sizeof(bytes); i++) bytes[i] = i;
    g_assert_cmpuint(length, <=, sizeof(bytes));
    return g_variant_new("(iiibii@ay)", width, height, rowstride, alpha,
                         bits, channels, g_variant_new_fixed_array(
                             G_VARIANT_TYPE_BYTE, bytes, length, 1));
}
static void short_image(void) {
    g_autoptr(GVariant) hints = hint("image-data", image(2, 2, 6, FALSE, 8, 3, 1));
    Notification *notification = g_new0(Notification, 1);
    parse_notify_hints(hints, notification);
    g_assert_null(notification->img_data.data);
    free_notification(notification);
}
static void invalid_image_layouts(void) {
    const gint layouts[][6] = {
        {-1, 2, 6, FALSE, 8, 3}, {2, 0, 6, FALSE, 8, 3},
        {2, 2, 5, FALSE, 8, 3}, {2, 2, 6, FALSE, 16, 3},
        {2, 2, 8, TRUE, 8, 3}, {2, 2, 8, FALSE, 8, 4},
        {G_MAXINT, G_MAXINT, G_MAXINT, TRUE, 8, 4},
    };
    for (guint i = 0; i < G_N_ELEMENTS(layouts); i++) {
        const gint *layout = layouts[i];
        g_autoptr(GVariant) hints = hint("image-data", image(
            layout[0], layout[1], layout[2], layout[3], layout[4], layout[5], 16));
        Notification *notification = g_new0(Notification, 1);
        parse_notify_hints(hints, notification);
        g_assert_null(notification->img_data.data);
        free_notification(notification);
    }
}
static void wrong_hint_types(void) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    const gchar *strings[] = {"category", "desktop-entry", "image-path", NULL};
    const gchar *booleans[] = {"action-icons", "resident", "transient", NULL};
    for (guint i = 0; strings[i]; i++)
        g_variant_builder_add(&builder, "{sv}", strings[i], g_variant_new_int32(5));
    for (guint i = 0; booleans[i]; i++)
        g_variant_builder_add(&builder, "{sv}", booleans[i], g_variant_new_string("false"));
    g_variant_builder_add(&builder, "{sv}", "urgency", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&builder, "{sv}", "image-data", g_variant_new_string("bad image"));
    g_autoptr(GVariant) hints = g_variant_ref_sink(g_variant_builder_end(&builder));
    Notification *notification = g_new0(Notification, 1);
    parse_notify_hints(hints, notification);
    g_assert_null(notification->category);
    g_assert_null(notification->desktop_entry);
    g_assert_null(notification->image_path);
    g_assert_null(notification->img_data.data);
    g_assert_false(notification->resident);
    g_assert_false(notification->transient);
    g_assert_false(notification->action_icons);
    free_notification(notification);
}
typedef struct { gpointer bytes; guint releases; } ImageStorage;
static void release_storage(gpointer data) {
    ImageStorage *storage = data;
    storage->releases++;
    g_free(storage->bytes);
}
static void image_storage_and_last_row(void) {
    g_autoptr(GVariant) original = hint("image-data", image(2, 2, 8, FALSE, 8, 3, 14));
    gsize size = g_variant_get_size(original);
    ImageStorage storage = {.bytes = g_malloc(size)};
    g_variant_store(original, storage.bytes);
    GVariant *hints = g_variant_ref_sink(g_variant_new_from_data(
        G_VARIANT_TYPE_VARDICT, storage.bytes, size, TRUE, release_storage, &storage));
    Notification *notification = g_new0(Notification, 1);
    parse_notify_hints(hints, notification);
    g_assert_nonnull(notification->img_data.data);
    g_variant_unref(hints);
    g_assert_cmpuint(storage.releases, ==, 1);
    g_assert_cmpuint(notification->img_data.width, ==, 2);
    g_assert_cmpuint(notification->img_data.rowstride, ==, 8);
    for (guint i = 0; i < 14; i++)
        g_assert_cmpuint((guchar)notification->img_data.data[i], ==, i);
    free_notification(notification);
}
static void image_aliases(void) {
    const gchar *names[] = {"image-data", "image_data", "icon_data", NULL};
    for (guint i = 0; names[i]; i++) {
        g_autoptr(GVariant) hints = hint(names[i], image(2, 2, 8, TRUE, 8, 4, 16));
        Notification *notification = g_new0(Notification, 1);
        parse_notify_hints(hints, notification);
        g_assert_nonnull(notification->img_data.data);
        g_assert_true(notification->img_data.has_alpha);
        g_assert_cmpuint(notification->img_data.channels, ==, 4);
        free_notification(notification);
    }
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/notifications/empty-name", empty_name_without_desktop_entry);
    g_test_add_func("/notifications/internal-time", internal_creation_time);
    g_test_add_func("/notifications/actions-valid", complete_action_pairs);
    g_test_add_func("/notifications/actions-incomplete", incomplete_action_pair);
    g_test_add_func("/notifications/image-short", short_image);
    g_test_add_func("/notifications/image-layout", invalid_image_layouts);
    g_test_add_func("/notifications/hints-wrong-types", wrong_hint_types);
    g_test_add_func("/notifications/image-ownership", image_storage_and_last_row);
    g_test_add_func("/notifications/image-aliases", image_aliases);
    return g_test_run();
}
