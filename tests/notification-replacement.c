#include <adwaita.h>

/* Exercise the real group/list/OSD controllers. Rename private symbols that
 * would otherwise collide when their implementation files share this fixture. */
#define signals group_signals_enum
#define signals_n group_signals_count
#define on_notification_added group_notification_added
#define on_notification_closed group_notification_closed
#define on_message_tray_will_hide group_tray_will_hide
#include "../src/panel/message_tray/notifications/notification_group.c"
#undef signals
#undef signals_n
#undef on_notification_added
#undef on_notification_closed
#undef on_message_tray_will_hide
#define signals osd_signals_enum
#define signals_n osd_signals_count
#define on_notification_added osd_notification_added
#include "../src/panel/message_tray/notifications/notification_osd.c"
#undef signals
#undef signals_n
#undef on_notification_added
#include "../src/panel/message_tray/notifications/notifications_list.c"

typedef struct { GObject parent_instance; } FixtureServices;
typedef struct { GObjectClass parent_class; } FixtureServicesClass;
G_DEFINE_TYPE(FixtureServices, fixture_services, G_TYPE_OBJECT)
static GObject *fixture_source;
static GPtrArray *fixture_notifications, *fixture_players;
static guint fixture_closes;
static void fixture_services_class_init(FixtureServicesClass *klass) {
    GType type = G_TYPE_FROM_CLASS(klass);
    const gchar *notification_signals[] = {"notification-added", "notification-closed", "notification-replaced"};
    for (guint i = 0; i < G_N_ELEMENTS(notification_signals); i++)
        g_signal_new(notification_signals[i], type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                     G_TYPE_NONE, 3, G_TYPE_PTR_ARRAY, G_TYPE_UINT, G_TYPE_UINT);
    g_signal_new("notification-changed", type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                 G_TYPE_NONE, 1, G_TYPE_PTR_ARRAY);
    const gchar *tray_signals[] = {"message-tray-hidden", "message-tray-will-hide", "message-tray-visible", "message-tray-will-show"};
    for (guint i = 0; i < G_N_ELEMENTS(tray_signals); i++)
        g_signal_new(tray_signals[i], type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
    g_signal_new("media-player-changed", type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);
    g_signal_new("media-player-removed", type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);
}
static void fixture_services_init(FixtureServices *self) {}
NotificationsService *notifications_service_get_global(void) { return (NotificationsService *)fixture_source; }
GPtrArray *notifications_service_get_notifications(NotificationsService *self) { return fixture_notifications; }
MediaPlayerService *media_player_service_get_global(void) { return (MediaPlayerService *)fixture_source; }
GPtrArray *media_player_service_get_players(MediaPlayerService *self) { return fixture_players; }
MessageTray *message_tray_get_global(void) { return (MessageTray *)fixture_source; }
void message_tray_shrink(MessageTray *self) {}
#define MEDIA_COMMAND(name) void name(MediaPlayerService *self, gchar *player) {}
MEDIA_COMMAND(media_player_service_player_playpause)
MEDIA_COMMAND(media_player_service_player_previous)
MEDIA_COMMAND(media_player_service_player_next)
MEDIA_COMMAND(media_player_service_player_raise)
int notifications_service_invoke_action(NotificationsService *self, guint32 id, char *action) { return 0; }

static void free_fixture_notification(gpointer value) {
    Notification *notification = value;
    g_free(notification->app_name);
    g_free(notification->summary);
    g_free(notification->body);
    g_date_time_unref(notification->created_on);
    g_free(notification);
}
static Notification *fixture_notification(guint32 id, const gchar *app, const gchar *summary) {
    Notification *notification = g_new0(Notification, 1);
    notification->id = id;
    notification->app_name = g_strdup(app);
    notification->summary = g_strdup(summary);
    notification->body = g_strdup("Fixture body");
    notification->created_on = g_date_time_new_now_local();
    return notification;
}
static void fixture_add(guint32 id, const gchar *app, const gchar *summary) {
    g_ptr_array_add(fixture_notifications, fixture_notification(id, app, summary));
    g_signal_emit_by_name(fixture_source, "notification-added", fixture_notifications, id, fixture_notifications->len - 1);
    g_signal_emit_by_name(fixture_source, "notification-changed", fixture_notifications);
}
static void fixture_replace(guint32 id, const gchar *app, const gchar *summary) {
    for (guint i = 0; i < fixture_notifications->len; i++) {
        Notification *old = g_ptr_array_index(fixture_notifications, i);
        if ((guint32)old->id != id) continue;
        Notification *replacement = fixture_notification(id, app, summary);
        replacement->replaces_id = id;
        g_ptr_array_index(fixture_notifications, i) = replacement;
        g_signal_emit_by_name(fixture_source, "notification-replaced", fixture_notifications, id, i);
        g_signal_emit_by_name(fixture_source, "notification-changed", fixture_notifications);
        free_fixture_notification(old);
        return;
    }
    g_assert_not_reached();
}
int notifications_service_closed_notification(NotificationsService *self, guint32 id, enum NotifcationsClosedReason reason) {
    for (guint i = 0; i < fixture_notifications->len; i++) {
        Notification *notification = g_ptr_array_index(fixture_notifications, i);
        if ((guint32)notification->id != id) continue;
        fixture_closes++;
        g_signal_emit_by_name(fixture_source, "notification-closed", fixture_notifications, id, i);
        g_ptr_array_remove_index(fixture_notifications, i);
        g_signal_emit_by_name(fixture_source, "notification-changed", fixture_notifications);
        return 0;
    }
    return -1;
}
static void fixture_setup(void) {
    fixture_source = g_object_new(fixture_services_get_type(), NULL);
    fixture_notifications = g_ptr_array_new_with_free_func(free_fixture_notification);
    fixture_players = g_ptr_array_new();
    fixture_closes = 0;
}
static void fixture_teardown(NotificationsList *list) {
    g_object_unref(list);
    g_ptr_array_unref(fixture_notifications);
    g_ptr_array_unref(fixture_players);
    g_object_unref(fixture_source);
}
static NotificationGroup *group(NotificationsList *list, const gchar *app) {
    return g_hash_table_lookup(list->notification_groups, app);
}
static NotificationWidget *widget(NotificationGroup *group, guint32 id) {
    return g_hash_table_lookup(group->notification_widgets, GUINT_TO_POINTER(id));
}
static guint children(GtkWidget *parent) {
    guint count = 0;
    for (GtkWidget *child = gtk_widget_get_first_child(parent); child; child = gtk_widget_get_next_sibling(child)) count++;
    return count;
}
static void flagged(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void iterate_for(guint milliseconds) {
    gint64 deadline = g_get_monotonic_time() + milliseconds * G_TIME_SPAN_MILLISECOND;
    do {
        while (g_main_context_iteration(NULL, FALSE)) {}
        g_usleep(1000);
    } while (g_get_monotonic_time() < deadline);
}

static void same_application_replacement_preserves_widget_and_order(void) {
    fixture_setup();
    g_ptr_array_add(fixture_notifications, fixture_notification(1, "Alpha", "Original"));
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    adw_switch_row_set_active(list->dnd_switch, TRUE);
    NotificationGroup *alpha = group(list, "Alpha");
    NotificationWidget *first = widget(alpha, 1);
    GtkWidget *first_root = notification_widget_get_widget(first);
    fixture_replace(1, "Alpha", "Replacement");
    g_assert_true(widget(alpha, 1) == first);
    g_assert_true(notification_widget_get_widget(first) == first_root);
    g_assert_cmpstr(gtk_label_get_text(notification_widget_get_summary(first)), ==, "Replacement");
    g_assert_cmpuint(children(GTK_WIDGET(alpha->notification_list)), ==, 0);
    fixture_add(2, "Alpha", "Second");
    NotificationWidget *head = alpha->head_notification;
    g_assert_true(head == widget(alpha, 2));
    expand_messages_on_click(NULL, alpha);
    g_assert_true(alpha->expanded);
    fixture_replace(1, "Alpha", "Older replacement");
    g_assert_true(alpha->head_notification == head);
    g_assert_true(alpha->expanded);
    g_assert_true(widget(alpha, 1) == first);
    g_assert_true(gtk_widget_get_first_child(GTK_WIDGET(alpha->notification_list)) == first_root);
    fixture_replace(2, "Alpha", "Head replacement");
    g_assert_true(alpha->head_notification == head);
    g_assert_true(alpha->expanded);
    g_assert_cmpuint(g_hash_table_size(alpha->notification_widgets), ==, 2);
    g_assert_cmpuint(fixture_closes, ==, 0);
    notifications_service_closed_notification((NotificationsService *)fixture_source, 2, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
    g_assert_true(alpha->head_notification == first);
    g_assert_cmpuint(g_hash_table_size(alpha->notification_widgets), ==, 1);
    notifications_service_closed_notification((NotificationsService *)fixture_source, 1, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
    g_assert_cmpuint(g_hash_table_size(list->notification_groups), ==, 0);
    g_assert_true(gtk_widget_get_visible(GTK_WIDGET(list->status)));
    fixture_teardown(list);
}

static void changed_application_moves_between_existing_and_new_groups(void) {
    fixture_setup();
    g_ptr_array_add(fixture_notifications, fixture_notification(1, "Alpha", "First"));
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    adw_switch_row_set_active(list->dnd_switch, TRUE);
    fixture_add(2, "Alpha", "Head");
    fixture_add(3, "Beta", "Other group");
    NotificationGroup *alpha = group(list, "Alpha");
    NotificationGroup *beta = group(list, "Beta");
    fixture_replace(1, "Beta", "Moved older item");
    g_assert_null(widget(alpha, 1));
    g_assert_nonnull(widget(beta, 1));
    g_assert_cmpuint(g_hash_table_size(alpha->notification_widgets), ==, 1);
    g_assert_cmpuint(g_hash_table_size(beta->notification_widgets), ==, 2);
    gboolean alpha_gone = FALSE;
    g_object_weak_ref(G_OBJECT(alpha), flagged, &alpha_gone);
    fixture_replace(2, "Gamma", "Moved last item");
    g_assert_true(alpha_gone);
    g_assert_null(group(list, "Alpha"));
    g_assert_nonnull(widget(group(list, "Gamma"), 2));
    g_assert_cmpuint(g_hash_table_size(list->notification_groups), ==, 2);
    g_assert_cmpuint(children(GTK_WIDGET(list->list)), ==, 2);
    g_assert_cmpuint(fixture_closes, ==, 0);
    notifications_service_closed_notification((NotificationsService *)fixture_source, 2, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
    g_assert_null(group(list, "Gamma"));
    fixture_teardown(list);
}

static void visible_osd_updates_in_place_and_hidden_replacements_stay_hidden(void) {
    fixture_setup();
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    adw_switch_row_set_active(list->dnd_switch, FALSE);
    fixture_add(1, "Alpha", "Visible original");
    NotificationsOSD *osd = list->osd;
    NotificationWidget *original = osd->notification;
    guint old_timeout = osd->timeout_id;
    g_assert_nonnull(original);
    iterate_for(400);
    g_assert_true(gtk_widget_get_visible(GTK_WIDGET(osd->win)));
    g_assert_true(gtk_widget_get_mapped(GTK_WIDGET(osd->win)));
    fixture_replace(1, "Alpha", "Visible replacement");
    g_assert_true(osd->notification == original);
    g_assert_cmpstr(gtk_label_get_text(notification_widget_get_summary(original)), ==, "Visible replacement");
    g_assert_cmpuint(osd->timeout_id, !=, old_timeout);
    g_assert_null(g_main_context_find_source_by_id(NULL, old_timeout));
    g_signal_emit_by_name(fixture_source, "message-tray-will-show");
    g_signal_emit_by_name(fixture_source, "message-tray-visible");
    fixture_replace(1, "Alpha", "Tray-visible replacement");
    g_assert_false(gtk_revealer_get_reveal_child(osd->revealer));
    g_assert_cmpuint(osd->timeout_id, ==, 0);
    g_signal_emit_by_name(fixture_source, "message-tray-hidden");
    notification_osd_hide(osd);
    fixture_replace(1, "Alpha", "Hidden replacement");
    g_assert_false(gtk_revealer_get_reveal_child(osd->revealer));
    g_assert_cmpuint(osd->timeout_id, ==, 0);
    adw_switch_row_set_active(list->dnd_switch, TRUE);
    fixture_add(2, "Beta", "DND notification");
    fixture_replace(2, "Beta", "DND replacement");
    g_assert_false(gtk_revealer_get_reveal_child(osd->revealer));
    g_assert_true(osd->notification == original);
    g_assert_cmpuint(fixture_closes, ==, 0);
    gboolean osd_gone = FALSE;
    g_object_weak_ref(G_OBJECT(osd), flagged, &osd_gone);
    fixture_teardown(list);
    g_assert_true(osd_gone);
}

static void osd_rebuild_and_disposal_release_windows_and_callbacks(void) {
    fixture_setup();
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    adw_switch_row_set_active(list->dnd_switch, FALSE);
    fixture_add(1, "Alpha", "Before rebuild");
    NotificationsOSD *osd = g_object_ref(list->osd);
    gboolean old_window_gone = FALSE, old_notification_gone = FALSE;
    g_object_weak_ref(G_OBJECT(osd->win), flagged, &old_window_gone);
    g_object_weak_ref(G_OBJECT(osd->notification), flagged, &old_notification_gone);
    guint old_timeout = osd->timeout_id;
    notification_osd_reinitialize(osd);
    g_assert_true(old_window_gone);
    g_assert_true(old_notification_gone);
    g_assert_cmpuint(osd->timeout_id, ==, 0);
    g_assert_null(g_main_context_find_source_by_id(NULL, old_timeout));
    g_assert_null(osd->notification);
    g_assert_false(gtk_widget_get_visible(GTK_WIDGET(osd->win)));
    fixture_replace(1, "Alpha", "Replacement after rebuild");
    g_assert_null(osd->notification);

    fixture_add(2, "Beta", "After rebuild");
    g_assert_nonnull(osd->notification);
    g_assert_cmpuint(notification_widget_get_id(osd->notification), ==, 2);
    /* External window destruction (for example, output loss) rebuilds one
     * hidden window and disconnects the old subscriptions and timers. */
    old_timeout = osd->timeout_id;
    old_window_gone = FALSE;
    g_object_weak_ref(G_OBJECT(osd->win), flagged, &old_window_gone);
    gtk_window_destroy(GTK_WINDOW(osd->win));
    g_assert_true(old_window_gone);
    g_assert_nonnull(osd->win);
    g_assert_null(osd->notification);
    g_assert_null(g_main_context_find_source_by_id(NULL, old_timeout));
    fixture_add(3, "Gamma", "Before owner disposal");
    old_timeout = osd->timeout_id;
    g_object_unref(list);
    g_autoptr(NotificationsList) owner = g_weak_ref_get(&osd->list_owner);
    g_assert_null(owner);
    fixture_add(4, "Delta", "No remaining list");
    fixture_replace(4, "Delta", "No remaining list replacement");
    g_assert_cmpuint(notification_widget_get_id(osd->notification), ==, 3);
    gboolean final_window_gone = FALSE;
    g_object_weak_ref(G_OBJECT(osd->win), flagged, &final_window_gone);
    g_object_run_dispose(G_OBJECT(osd));
    g_object_run_dispose(G_OBJECT(osd));
    g_assert_true(final_window_gone);
    g_assert_null(g_main_context_find_source_by_id(NULL, old_timeout));
    g_signal_emit_by_name(fixture_source, "message-tray-visible");
    g_signal_emit_by_name(fixture_source, "message-tray-will-show");
    fixture_replace(4, "Delta", "Disposed OSD stays inert");
    g_object_unref(osd);
    g_ptr_array_unref(fixture_notifications);
    g_ptr_array_unref(fixture_players);
    g_object_unref(fixture_source);
}

static void head_removal_releases_controllers_and_reparented_roots(void) {
    fixture_setup();
    g_ptr_array_add(fixture_notifications, fixture_notification(1, "Alpha", "First"));
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    adw_switch_row_set_active(list->dnd_switch, TRUE);
    NotificationGroup *alpha = group(list, "Alpha");
    gboolean first_gone = FALSE, first_root_gone = FALSE, second_gone = FALSE, second_root_gone = FALSE;
    NotificationWidget *first = widget(alpha, 1);
    g_object_weak_ref(G_OBJECT(first), flagged, &first_gone);
    g_object_weak_ref(G_OBJECT(notification_widget_get_widget(first)), flagged, &first_root_gone);
    fixture_add(2, "Alpha", "Second");
    NotificationWidget *second = widget(alpha, 2);
    g_object_weak_ref(G_OBJECT(second), flagged, &second_gone);
    g_object_weak_ref(G_OBJECT(notification_widget_get_widget(second)), flagged, &second_root_gone);
    notifications_service_closed_notification((NotificationsService *)fixture_source, 2, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
    g_assert_true(second_gone);
    g_assert_true(second_root_gone);
    g_assert_false(first_gone);
    g_assert_true(alpha->head_notification == first);
    notifications_service_closed_notification((NotificationsService *)fixture_source, 1, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
    g_assert_true(first_gone);
    g_assert_true(first_root_gone);
    g_assert_cmpuint(g_hash_table_size(list->notification_groups), ==, 0);
    fixture_teardown(list);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_autofree gchar *schema_directory = g_path_get_dirname(argv[0]);
    g_setenv("GSETTINGS_SCHEMA_DIR", schema_directory, TRUE);
    /* The fixture maps real layer surfaces in a headless compositor. Avoid
     * relying on the host GPU or its Vulkan surface support. */
    g_setenv("GSK_RENDERER", "cairo", TRUE);
    gtk_init();
    g_test_add_func("/notification-replacement/same-app", same_application_replacement_preserves_widget_and_order);
    g_test_add_func("/notification-replacement/changed-app", changed_application_moves_between_existing_and_new_groups);
    g_test_add_func("/notification-replacement/osd", visible_osd_updates_in_place_and_hidden_replacements_stay_hidden);
    g_test_add_func("/notification-replacement/osd-lifecycle", osd_rebuild_and_disposal_release_windows_and_callbacks);
    g_test_add_func("/notification-replacement/head-removal", head_removal_releases_controllers_and_reparented_roots);
    return g_test_run();
}
