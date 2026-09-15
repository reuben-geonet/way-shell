#include <adwaita.h>

#include "../src/panel/message_tray/notifications/notifications_list.c"

typedef struct { GObject parent_instance; } FixtureServices;
typedef struct { GObjectClass parent_class; } FixtureServicesClass;
G_DEFINE_TYPE(FixtureServices, fixture_services, G_TYPE_OBJECT)

static GObject *media_service, *notification_service, *tray;
static GPtrArray *players, *notifications;
static guint widgets_created, widgets_destroyed, widget_updates, groups_destroyed;

static void fixture_services_class_init(FixtureServicesClass *klass) {
    GType type = G_TYPE_FROM_CLASS(klass);
    g_signal_new("media-player-changed", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);
    g_signal_new("media-player-removed", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);
    g_signal_new("notification-added", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 3,
                 G_TYPE_PTR_ARRAY, G_TYPE_UINT, G_TYPE_UINT);
    g_signal_new("message-tray-hidden", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_services_init(FixtureServices *self) {}

MediaPlayerService *media_player_service_get_global(void) {
    return (MediaPlayerService *)media_service;
}
GPtrArray *media_player_service_get_players(MediaPlayerService *self) { return players; }
NotificationsService *notifications_service_get_global(void) {
    return (NotificationsService *)notification_service;
}
GPtrArray *notifications_service_get_notifications(NotificationsService *self) {
    return notifications;
}
MessageTray *message_tray_get_global(void) { return (MessageTray *)tray; }
void message_tray_shrink(MessageTray *self) {}

struct _NotificationWidget {
    GObject parent_instance;
    GtkWidget *container;
    gchar *name;
    gchar *title;
};
G_DEFINE_TYPE(NotificationWidget, notification_widget, G_TYPE_OBJECT)
static void fixture_widget_finalize(GObject *object) {
    NotificationWidget *self = NOTIFICATION_WIDGET(object);
    g_clear_object(&self->container);
    g_free(self->name);
    g_free(self->title);
    widgets_destroyed++;
    G_OBJECT_CLASS(notification_widget_parent_class)->finalize(object);
}
static void notification_widget_class_init(NotificationWidgetClass *klass) {
    G_OBJECT_CLASS(klass)->finalize = fixture_widget_finalize;
}
static void notification_widget_init(NotificationWidget *self) {
    self->container = g_object_ref_sink(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
    widgets_created++;
}
NotificationWidget *notification_widget_from_media_player(MediaPlayer *player) {
    NotificationWidget *self = g_object_new(NOTIFICATION_WIDGET_TYPE, NULL);
    self->name = g_strdup(player->name);
    return self;
}
NotificationWidget *notification_widget_set_media_player(NotificationWidget *self,
                                                         MediaPlayer *player) {
    g_free(self->title);
    self->title = g_strdup(player->title);
    widget_updates++;
    return self;
}
GtkWidget *notification_widget_get_widget(NotificationWidget *self) { return self->container; }
gchar *notification_widget_get_media_player_name(NotificationWidget *self) { return self->name; }

struct _NotificationGroup {
    GObject parent_instance;
    GtkWidget *container;
    gchar *app;
};
G_DEFINE_TYPE(NotificationGroup, notification_group, G_TYPE_OBJECT)
static void fixture_group_finalize(GObject *object) {
    NotificationGroup *self = NOTIFICATION_GROUP(object);
    g_clear_object(&self->container);
    g_free(self->app);
    groups_destroyed++;
    G_OBJECT_CLASS(notification_group_parent_class)->finalize(object);
}
static void notification_group_class_init(NotificationGroupClass *klass) {
    G_OBJECT_CLASS(klass)->finalize = fixture_group_finalize;
    const gchar *names[] = {
        "notification-group-empty", "notification-group-expanded",
        "notification-group-will-expand", "notification-group-collapsed",
        "notification-group-notification-added", "notification-group-notification-closed",
        "notification-group-notification-expanded", "notification-group-notification-collapsed"
    };
    for (guint i = 0; i < G_N_ELEMENTS(names); i++)
        g_signal_new(names[i], G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
                     0, NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void notification_group_init(NotificationGroup *self) {
    self->container = g_object_ref_sink(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
}
GtkWidget *notification_group_get_widget(NotificationGroup *self) { return self->container; }
gchar *notification_group_get_app_name(NotificationGroup *self) { return self->app; }
void notification_group_add_notification(NotificationGroup *self, Notification *notification) {
    g_free(self->app);
    self->app = g_strdup(notification->app_name);
}
void notification_group_dismiss_all(NotificationGroup *self) {
    g_signal_emit_by_name(self, "notification-group-empty");
}

GType notifications_osd_get_type(void) { return G_TYPE_OBJECT; }
void notification_osd_set_notification_list(NotificationsOSD *self, NotificationsList *list) {}

static void setup(void) {
    media_service = g_object_new(fixture_services_get_type(), NULL);
    notification_service = g_object_new(fixture_services_get_type(), NULL);
    tray = g_object_new(fixture_services_get_type(), NULL);
    players = g_ptr_array_new();
    notifications = g_ptr_array_new();
    widgets_created = widgets_destroyed = widget_updates = groups_destroyed = 0;
}
static void teardown(void) {
    g_ptr_array_unref(players);
    g_ptr_array_unref(notifications);
    g_object_unref(media_service);
    g_object_unref(notification_service);
    g_object_unref(tray);
}
static void finalized(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }

static void ready_seed_change_and_remove(void) {
    setup();
    MediaPlayer player = {.name = "org.mpris.MediaPlayer2.fixture", .title = "First track"};
    g_ptr_array_add(players, &player);
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    GtkWidget *container = g_object_ref_sink(notifications_list_get_widget(list));
    g_assert_cmpuint(list->media_players->len, ==, 1);
    g_assert_cmpuint(widgets_created, ==, 1);
    g_assert_false(gtk_widget_get_visible(GTK_WIDGET(list->status)));
    g_assert_true(gtk_widget_get_visible(GTK_WIDGET(list->scroll)));
    NotificationWidget *widget = g_ptr_array_index(list->media_players, 0);
    g_assert_cmpstr(widget->title, ==, "First track");
    player.title = "Next track";
    guint before = widget_updates;
    g_signal_emit_by_name(media_service, "media-player-changed", &player);
    g_assert_cmpuint(widgets_created, ==, 1);
    g_assert_cmpuint(widget_updates, ==, before + 1);
    g_assert_cmpstr(widget->title, ==, "Next track");
    g_ptr_array_set_size(players, 0);
    g_signal_emit_by_name(media_service, "media-player-removed", &player);
    g_assert_cmpuint(list->media_players->len, ==, 0);
    g_assert_cmpuint(widgets_destroyed, ==, 1);
    g_assert_true(gtk_widget_get_visible(GTK_WIDGET(list->status)));
    g_assert_false(gtk_widget_get_visible(GTK_WIDGET(list->scroll)));
    g_object_unref(list);
    g_object_unref(container);
    teardown();
}

static void reinitialize_disconnects_previous_layout_and_sources(void) {
    setup();
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    GtkWidget *old_container = g_object_ref_sink(notifications_list_get_widget(list));
    GtkButton *old_clear = list->clear;
    GSettings *settings = list->settings;
    for (guint i = 0; i < 3; i++) {
        notifications_list_reinitialize(list);
        g_assert_true(list->settings == settings);
    }
    MediaPlayer player = {.name = "org.mpris.MediaPlayer2.fixture", .title = "After rebuild"};
    g_ptr_array_add(players, &player);
    guint before = widget_updates;
    g_signal_emit_by_name(media_service, "media-player-changed", &player);
    g_assert_cmpuint(widget_updates, ==, before + 1);
    g_assert_cmpuint(list->media_players->len, ==, 1);
    g_assert_cmpuint(g_signal_handlers_block_matched(old_clear, G_SIGNAL_MATCH_DATA,
                      0, 0, NULL, NULL, list), ==, 0);

    Notification notification = {.app_name = "Retained group", .id = 8};
    g_ptr_array_add(notifications, &notification);
    g_signal_emit_by_name(notification_service, "notification-added", notifications, 8, 0);
    NotificationGroup *old_group = g_object_ref(
        g_hash_table_lookup(list->notification_groups, notification.app_name));

    // Reconnect against a replacement service object and ensure the old source
    // cannot mutate the new layout, even while that source remains alive.
    GObject *old_service = media_service;
    media_service = g_object_new(fixture_services_get_type(), NULL);
    notifications_list_reinitialize(list);
    g_assert_cmpuint(list->media_players->len, ==, 1);
    g_assert_true(list->settings == settings);
    g_assert_cmpuint(g_signal_handlers_block_matched(old_group, G_SIGNAL_MATCH_DATA,
                      0, 0, NULL, NULL, list), ==, 0);
    g_signal_emit_by_name(old_group, "notification-group-empty");
    g_assert_cmpuint(g_hash_table_size(list->notification_groups), ==, 1);
    g_object_unref(old_group);
    before = widget_updates;
    g_signal_emit_by_name(old_service, "media-player-changed", &player);
    g_assert_cmpuint(widget_updates, ==, before);
    g_assert_cmpuint(g_signal_handlers_block_matched(old_service, G_SIGNAL_MATCH_DATA,
                      0, 0, NULL, NULL, list), ==, 0);
    g_signal_emit_by_name(media_service, "media-player-changed", &player);
    g_assert_cmpuint(widget_updates, ==, before + 1);
    g_object_unref(old_service);
    g_object_unref(list);
    g_object_unref(old_container);
    g_assert_cmpuint(widgets_created, ==, widgets_destroyed);
    teardown();
}

static void disposal_releases_collections_and_disconnects_callbacks(void) {
    setup();
    MediaPlayer player = {.name = "org.mpris.MediaPlayer2.fixture", .title = "Owned track"};
    Notification notification = {.app_name = "Fixture application", .id = 7};
    g_ptr_array_add(players, &player);
    g_ptr_array_add(notifications, &notification);
    NotificationsList *list = g_object_new(NOTIFICATIONS_LIST_TYPE, NULL);
    GtkWidget *container = g_object_ref_sink(notifications_list_get_widget(list));
    GtkButton *clear = list->clear;
    gboolean settings_gone = FALSE, osd_gone = FALSE, owner_gone = FALSE;
    g_object_weak_ref(G_OBJECT(list->settings), finalized, &settings_gone);
    g_object_weak_ref(G_OBJECT(list->osd), finalized, &osd_gone);
    g_object_weak_ref(G_OBJECT(list), finalized, &owner_gone);
    // GObject dispose is explicitly repeatable. Arrays must release each
    // controller once and be safe when dispose is called again at final unref.
    g_object_run_dispose(G_OBJECT(list));
    g_assert_null(list->media_players);
    g_assert_null(list->notification_groups);
    g_assert_true(settings_gone);
    g_assert_true(osd_gone);
    g_assert_cmpuint(widgets_created, ==, widgets_destroyed);
    g_assert_cmpuint(groups_destroyed, ==, 1);
    g_object_run_dispose(G_OBJECT(list));
    gpointer owner = list;
    g_object_unref(list);
    g_assert_true(owner_gone);
    for (guint i = 0; i < 4; i++) {
        GObject *source = ((GObject *[]){media_service, notification_service, tray, G_OBJECT(clear)})[i];
        g_assert_cmpuint(g_signal_handlers_block_matched(source, G_SIGNAL_MATCH_DATA,
                          0, 0, NULL, NULL, owner), ==, 0);
    }
    g_signal_emit_by_name(media_service, "media-player-changed", &player);
    g_signal_emit_by_name(media_service, "media-player-removed", &player);
    g_signal_emit_by_name(notification_service, "notification-added", notifications, 7, 0);
    g_signal_emit_by_name(tray, "message-tray-hidden");
    g_signal_emit_by_name(clear, "clicked");
    g_object_unref(container);
    teardown();
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/media-widgets/ready-seed-change-removal", ready_seed_change_and_remove);
    g_test_add_func("/media-widgets/reinitialize-sources", reinitialize_disconnects_previous_layout_and_sources);
    g_test_add_func("/media-widgets/disposal", disposal_releases_collections_and_disconnects_callbacks);
    return g_test_run();
}
