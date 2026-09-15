#include <adwaita.h>
#include <glib/gstdio.h>

#include "../src/panel/message_tray/notifications/notification_widget.c"

typedef struct { GObject parent_instance; } FixtureServices;
typedef struct { GObjectClass parent_class; } FixtureServicesClass;
G_DEFINE_TYPE(FixtureServices, fixture_services, G_TYPE_OBJECT)
static GObject *services;
static GPtrArray *actions;
static guint last_id, closed, hidden;
static void fixture_services_class_init(FixtureServicesClass *klass) {
    g_signal_new("message-tray-will-hide", G_TYPE_FROM_CLASS(klass),
                 G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_services_init(FixtureServices *self) {}
MessageTray *message_tray_get_global(void) { return (MessageTray *)services; }
NotificationsService *notifications_service_get_global(void) { return (NotificationsService *)services; }
int notifications_service_closed_notification(NotificationsService *self, guint32 id,
                                              enum NotifcationsClosedReason reason) {
    last_id = id; closed++; return 0;
}
int notifications_service_invoke_action(NotificationsService *self, guint32 id, char *action) {
    last_id = id; g_ptr_array_add(actions, g_strdup(action)); return 0;
}
void notification_osd_hide(NotificationsOSD *self) { hidden++; }

static void setup(void) {
    services = g_object_new(fixture_services_get_type(), NULL);
    actions = g_ptr_array_new_with_free_func(g_free);
    last_id = closed = hidden = 0;
}
static void teardown(void) {
    g_object_unref(services);
    g_ptr_array_unref(actions);
}
static Notification notification(void) {
    return (Notification){.id = 42, .summary = g_strdup("Fixture summary"),
        .body = g_strdup("Fixture body"), .created_on = g_date_time_new_now_local()};
}
static void notification_clear(Notification *n) {
    g_clear_pointer(&n->summary, g_free);
    g_clear_pointer(&n->body, g_free);
    g_clear_pointer(&n->created_on, g_date_time_unref);
}
static GtkWidget *action_named(NotificationWidget *widget, const gchar *label) {
    for (GtkWidget *child = gtk_widget_get_first_child(GTK_WIDGET(widget->action_container));
         child; child = gtk_widget_get_next_sibling(child))
        if (g_strcmp0(gtk_button_get_label(GTK_BUTTON(child)), label) == 0) return child;
    return NULL;
}
static void flagged(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void wait_for_image(NotificationWidget *widget) {
    gint64 deadline = g_get_monotonic_time() + 5 * G_TIME_SPAN_SECOND;
    while (!adw_avatar_get_custom_image(widget->avatar) && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        if (!adw_avatar_get_custom_image(widget->avatar)) g_usleep(1000);
    }
    g_assert_nonnull(adw_avatar_get_custom_image(widget->avatar));
}
static void wait_for_flag(gboolean *flag) {
    gint64 deadline = g_get_monotonic_time() + 5 * G_TIME_SPAN_SECOND;
    while (!*flag && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        if (!*flag) g_usleep(1000);
    }
    g_assert_true(*flag);
}

static void action_widgets_have_one_valid_parent(void) {
    setup();
    Notification n = notification();
    gchar *choices[] = {"default", "Open", "reply", "Reply", "dismiss", "Dismiss", NULL};
    n.actions = choices;
    NotificationWidget *widget = notification_widget_from_notification(&n, FALSE);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    GtkWidget *center = gtk_revealer_get_child(widget->action_revealer);
    g_assert_true(GTK_IS_CENTER_BOX(center));
    g_assert_true(gtk_widget_get_parent(center) == GTK_WIDGET(widget->action_revealer));
    g_assert_true(gtk_widget_get_parent(GTK_WIDGET(widget->action_container)) == center);
    g_assert_true(gtk_widget_get_parent(GTK_WIDGET(widget->action_revealer)) ==
                  GTK_WIDGET(widget->notification_container));
    g_assert_cmpuint(widget->actions_buttons_n, ==, 2);
    GtkWidget *reply = gtk_widget_get_first_child(GTK_WIDGET(widget->action_container));
    GtkWidget *dismiss = gtk_widget_get_next_sibling(reply);
    g_assert_cmpstr(gtk_button_get_label(GTK_BUTTON(reply)), ==, "Reply");
    g_assert_cmpstr(gtk_button_get_label(GTK_BUTTON(dismiss)), ==, "Dismiss");
    g_assert_null(gtk_widget_get_next_sibling(dismiss));
    g_signal_emit_by_name(reply, "clicked");
    g_assert_cmpuint(actions->len, ==, 1);
    g_assert_cmpstr(g_ptr_array_index(actions, 0), ==, "reply");
    g_assert_cmpuint(last_id, ==, 42);
    g_object_unref(widget);
    g_object_unref(container);
    notification_clear(&n);
    teardown();
}

static void replacement_copies_content_and_retains_root_timer_and_callbacks(void) {
    setup();
    Notification n = notification();
    n.urgency = 2;
    n.app_icon = "dialog-warning-symbolic";
    n.is_internal = TRUE;
    gchar *choices[] = {"reply", "Reply", NULL};
    n.actions = choices;
    NotificationWidget *widget = notification_widget_from_notification(&n, FALSE);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    GtkWidget *old_reply = g_object_ref(action_named(widget, "Reply"));
    guint timer = widget->timer_id;
    g_signal_emit_by_name(widget->header_expand, "clicked");
    g_assert_true(widget->expanded);
    g_assert_true(gtk_widget_has_css_class(GTK_WIDGET(widget->button), "notification-widget-button-critical"));

    Notification replacement = notification();
    g_free(replacement.summary); replacement.summary = g_strdup("  Replaced\nsummary  ");
    g_free(replacement.body); replacement.body = g_strdup("  <b>Replacement</b>\nbody  ");
    replacement.app_name = "Replacement application";
    replacement.app_icon = "dialog-information-symbolic";
    gchar *new_choices[] = {"answer", "Answer", "default", "Open", NULL};
    replacement.actions = new_choices;
    GDateTime *old = replacement.created_on;
    replacement.created_on = g_date_time_add_hours(old, -2);
    g_date_time_unref(old);
    gint64 timestamp = g_date_time_to_unix(replacement.created_on);
    for (guint i = 0; i < 5; i++) notification_widget_set_notification(widget, &replacement);
    g_assert_true(notification_widget_get_widget(widget) == container);
    g_assert_cmpuint(widget->timer_id, ==, timer);
    g_assert_true(widget->expanded);
    g_assert_true(gtk_revealer_get_reveal_child(widget->action_revealer));
    g_assert_cmpstr(gtk_label_get_text(widget->summary), ==, "Replaced summary");
    g_assert_cmpstr(gtk_label_get_text(widget->body), ==, "Replacement body");
    g_assert_cmpstr(gtk_label_get_text(widget->header_app_name), ==, "Replacement application");
    g_assert_cmpstr(gtk_image_get_icon_name(widget->header_app_icon), ==, "dialog-information-symbolic");
    g_assert_cmpstr(replacement.summary, ==, "  Replaced\nsummary  ");
    g_assert_cmpstr(replacement.body, ==, "  <b>Replacement</b>\nbody  ");
    g_assert_false(gtk_widget_has_css_class(GTK_WIDGET(widget->button), "notification-widget-button-critical"));
    g_assert_cmpuint(widget->actions_buttons_n, ==, 1);
    g_signal_emit_by_name(old_reply, "clicked");
    g_assert_cmpuint(actions->len, ==, 0);
    GtkWidget *answer = action_named(widget, "Answer");
    g_signal_emit_by_name(answer, "clicked");
    g_assert_cmpuint(actions->len, ==, 1);
    g_assert_cmpstr(g_ptr_array_index(actions, 0), ==, "answer");
    g_signal_emit_by_name(widget->button, "clicked");
    g_assert_cmpuint(actions->len, ==, 2);
    g_assert_cmpuint(closed, ==, 1);
    g_assert_cmpstr(g_ptr_array_index(actions, 1), ==, "default");
    g_clear_pointer(&replacement.created_on, g_date_time_unref);
    g_assert_cmpint(g_date_time_to_unix(widget->created_on), ==, timestamp);
    g_assert_cmpstr(gtk_label_get_text(widget->header_timer), ==, "2 hours ago");

    g_free(replacement.body); replacement.body = g_strdup("value < 3");
    replacement.actions = NULL;
    notification_widget_set_notification(widget, &replacement);
    g_assert_cmpstr(gtk_label_get_text(widget->body), ==, "value < 3");
    g_assert_cmpuint(widget->actions_buttons_n, ==, 0);
    g_assert_false(gtk_widget_get_visible(GTK_WIDGET(widget->action_revealer)));
    g_assert_nonnull(widget->created_on);

    GtkButton *button = widget->button, *dismiss = widget->header_dismiss;
    gpointer owner = widget;
    gboolean gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget), flagged, &gone);
    g_object_run_dispose(G_OBJECT(widget));
    g_assert_cmpuint(widget->timer_id, ==, 0);
    g_assert_null(widget->created_on);
    g_assert_null(g_main_context_find_source_by_id(NULL, timer));
    g_object_unref(widget);
    g_assert_true(gone);
    g_assert_cmpuint(g_signal_handlers_block_matched(button, G_SIGNAL_MATCH_DATA,
                      0, 0, NULL, NULL, owner), ==, 0);
    g_signal_emit_by_name(button, "clicked");
    g_signal_emit_by_name(dismiss, "clicked");
    g_signal_emit_by_name(services, "message-tray-will-hide");
    g_assert_cmpuint(actions->len, ==, 2);
    g_assert_cmpuint(closed, ==, 1);
    g_object_unref(old_reply);
    g_object_unref(container);
    notification_clear(&n); notification_clear(&replacement);
    teardown();
}

static void replacement_owns_pixels_and_resets_missing_icons(void) {
    setup();
    Notification n = notification();
    guchar rgba[] = {255, 0, 0, 255};
    n.img_data = (NotificationImageData){.width = 1, .height = 1, .rowstride = 4,
        .has_alpha = TRUE, .bits_per_sample = 8, .channels = 4, .data = (char *)rgba};
    NotificationWidget *widget = notification_widget_from_notification(&n, TRUE);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    rgba[0] = 0; rgba[2] = 255;
    GdkPaintable *image = adw_avatar_get_custom_image(widget->avatar);
    g_assert_true(GDK_IS_TEXTURE(image));
    g_autoptr(GdkTextureDownloader) download = gdk_texture_downloader_new(GDK_TEXTURE(image));
    gdk_texture_downloader_set_format(download, GDK_MEMORY_R8G8B8A8);
    gsize stride;
    g_autoptr(GBytes) pixels = gdk_texture_downloader_download_bytes(download, &stride);
    const guchar *pixel = g_bytes_get_data(pixels, NULL);
    g_assert_cmpuint(pixel[0], ==, 255);
    g_assert_cmpuint(pixel[2], ==, 0);
    n.img_data.data = NULL;
    n.app_icon = "audio-volume-high-symbolic";
    notification_widget_set_notification(widget, &n);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));
    g_assert_cmpstr(adw_avatar_get_icon_name(widget->avatar), ==, n.app_icon);
    g_assert_cmpstr(gtk_image_get_icon_name(widget->header_app_icon), ==, n.app_icon);
    n.app_icon = NULL;
    n.img_data.width = 0;
    n.img_data.data = (char *)rgba;
    notification_widget_set_notification(widget, &n);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));
    g_assert_cmpstr(adw_avatar_get_icon_name(widget->avatar), ==, "preferences-system-notifications-symbolic");
    g_assert_cmpstr(gtk_image_get_icon_name(widget->header_app_icon), ==, "preferences-system-notifications-symbolic");
    g_object_unref(widget);
    g_object_unref(container);
    notification_clear(&n);
    teardown();
}

static void replacement_preserves_the_osd_hide_button_once(void) {
    setup();
    Notification n = notification();
    gchar *initial[] = {"reply", "Reply", NULL};
    n.actions = initial;
    NotificationWidget *widget = notification_widget_from_notification(&n, FALSE);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    GObject *osd = g_object_new(G_TYPE_OBJECT, NULL);
    notification_widget_set_osd(widget, (NotificationsOSD *)osd);
    GtkWidget *hide = action_named(widget, "Hide");
    g_assert_nonnull(hide);
    notification_widget_set_osd(widget, (NotificationsOSD *)osd);
    g_assert_cmpuint(widget->actions_buttons_n, ==, 2);
    gchar *next[] = {"answer", "Answer", NULL};
    n.actions = next;
    notification_widget_set_notification(widget, &n);
    g_assert_true(action_named(widget, "Hide") == hide);
    g_assert_cmpuint(widget->actions_buttons_n, ==, 2);
    g_assert_true(gtk_widget_get_last_child(GTK_WIDGET(widget->action_container)) == hide);
    g_signal_emit_by_name(hide, "clicked");
    g_assert_cmpuint(hidden, ==, 1);
    n.actions = NULL;
    notification_widget_set_notification(widget, &n);
    g_assert_true(action_named(widget, "Hide") == hide);
    g_assert_cmpuint(widget->actions_buttons_n, ==, 1);
    g_assert_true(gtk_widget_has_css_class(hide, "only"));
    g_object_unref(osd);
    g_signal_emit_by_name(hide, "clicked");
    g_assert_cmpuint(hidden, ==, 1);
    g_object_unref(widget);
    g_signal_emit_by_name(hide, "clicked");
    g_assert_cmpuint(hidden, ==, 1);
    g_object_unref(container);
    notification_clear(&n);
    teardown();
}

static void file_images_replace_clear_and_cancel_on_owner_drop(void) {
    setup();
    g_autoptr(GError) error = NULL;
    g_autofree gchar *directory = g_dir_make_tmp("way-shell-notification-image.XXXXXX", &error);
    g_assert_no_error(error);
    g_autofree gchar *path = g_build_filename(directory, "image.png", NULL);
    guchar rgba[] = {0, 0, 255, 255};
    g_autoptr(GBytes) pixels = g_bytes_new(rgba, sizeof rgba);
    g_autoptr(GdkTexture) texture = gdk_memory_texture_new(1, 1, GDK_MEMORY_R8G8B8A8, pixels, 4);
    g_assert_true(gdk_texture_save_to_png(texture, path));
    Notification n = notification();
    n.image_path = path;
    NotificationWidget *widget = notification_widget_from_notification(&n, FALSE);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    wait_for_image(widget);
    GdkPaintable *image = adw_avatar_get_custom_image(widget->avatar);
    g_assert_true(GDK_IS_TEXTURE(image));
    g_autoptr(GdkTextureDownloader) download = gdk_texture_downloader_new(GDK_TEXTURE(image));
    gdk_texture_downloader_set_format(download, GDK_MEMORY_R8G8B8A8);
    gsize stride;
    g_autoptr(GBytes) decoded = gdk_texture_downloader_download_bytes(download, &stride);
    const guchar *pixel = g_bytes_get_data(decoded, NULL);
    g_assert_cmpuint(pixel[0], ==, 0);
    g_assert_cmpuint(pixel[2], ==, 255);
    n.image_path = NULL;
    notification_widget_set_notification(widget, &n);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));

    g_autofree gchar *uri = g_filename_to_uri(path, NULL, &error);
    g_assert_no_error(error);
    n.app_icon = uri;
    notification_widget_set_notification(widget, &n);
    wait_for_image(widget);
    g_assert_true(G_IS_FILE_ICON(gtk_image_get_gicon(widget->header_app_icon)));
    n.app_icon = NULL;
    n.image_path = path;
    notification_widget_set_notification(widget, &n);
    gboolean released = FALSE, gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget->artwork_cancellable), flagged, &released);
    g_object_weak_ref(G_OBJECT(widget), flagged, &gone);
    g_object_unref(widget);
    g_assert_true(gone);
    wait_for_flag(&released);
    g_object_unref(container);
    notification_clear(&n);
    g_remove(path);
    g_rmdir(directory);
    teardown();
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/notification-presentation/action-parentage", action_widgets_have_one_valid_parent);
    g_test_add_func("/notification-presentation/replacement-content", replacement_copies_content_and_retains_root_timer_and_callbacks);
    g_test_add_func("/notification-presentation/replacement-images", replacement_owns_pixels_and_resets_missing_icons);
    g_test_add_func("/notification-presentation/replacement-osd", replacement_preserves_the_osd_hide_button_once);
    g_test_add_func("/notification-presentation/file-images", file_images_replace_clear_and_cancel_on_owner_drop);
    return g_test_run();
}
