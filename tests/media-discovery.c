/* Exercise the actual MPRIS discovery service against an isolated session bus. */
#include "../src/services/media_player_service/media_player_service.c"

G_DEFINE_AUTOPTR_CLEANUP_FUNC(DbusMediaPlayer2, g_object_unref)
G_DEFINE_AUTOPTR_CLEANUP_FUNC(DbusMediaPlayer2Player, g_object_unref)

static GDBusConnection *fixture_bus;
static GHashTable *fixture_names;

DBUSService *dbus_service_get_global(void) { return NULL; }
GDBusConnection *dbus_service_get_session_bus(DBUSService *service) {
    return fixture_bus;
}
GHashTable *dbus_service_get_bus_names(DBUSService *service, gboolean system) {
    g_assert_false(system);
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        fixture_bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus", "ListNames", NULL, G_VARIANT_TYPE("(as)"),
        G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    g_assert_no_error(error);
    g_auto(GStrv) names = NULL;
    g_variant_get(reply, "(^as)", &names);
    g_hash_table_remove_all(fixture_names);
    for (guint i = 0; names[i]; i++)
        g_hash_table_add(fixture_names, g_strdup(names[i]));
    return fixture_names;
}

typedef struct {
    const gchar *address;
    const gchar *name;
    const gchar *identity;
    guint request_flags;
    GAsyncQueue *ready;
    GMainContext *context;
    GMainLoop *loop;
    GThread *thread;
} PlayerDaemon;

static GDBusConnection *connect_bus(const gchar *address) {
    g_autoptr(GError) error = NULL;
    GDBusConnection *connection = g_dbus_connection_new_for_address_sync(
        address, G_DBUS_CONNECTION_FLAGS_AUTHENTICATION_CLIENT |
                     G_DBUS_CONNECTION_FLAGS_MESSAGE_BUS_CONNECTION,
        NULL, NULL, &error);
    g_assert_no_error(error);
    g_dbus_connection_set_exit_on_close(connection, FALSE);
    return connection;
}

static gpointer serve_player(gpointer data) {
    PlayerDaemon *daemon = data;
    daemon->context = g_main_context_new();
    g_main_context_push_thread_default(daemon->context);
    daemon->loop = g_main_loop_new(daemon->context, FALSE);
    g_autoptr(GDBusConnection) bus = connect_bus(daemon->address);
    g_autoptr(DbusMediaPlayer2) root = dbus_media_player2_skeleton_new();
    g_autoptr(DbusMediaPlayer2Player) player =
        dbus_media_player2_player_skeleton_new();
    dbus_media_player2_set_identity(root, daemon->identity);
    dbus_media_player2_player_set_playback_status(player, "Stopped");
    GVariantBuilder metadata;
    g_variant_builder_init(&metadata, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&metadata, "{sv}", "xesam:title",
                          g_variant_new_string(daemon->identity));
    dbus_media_player2_player_set_metadata(
        player, g_variant_builder_end(&metadata));
    g_autoptr(GError) error = NULL;
    g_assert_true(g_dbus_interface_skeleton_export(
        G_DBUS_INTERFACE_SKELETON(root), bus, "/org/mpris/MediaPlayer2",
        &error));
    g_assert_no_error(error);
    g_assert_true(g_dbus_interface_skeleton_export(
        G_DBUS_INTERFACE_SKELETON(player), bus, "/org/mpris/MediaPlayer2",
        &error));
    g_assert_no_error(error);
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus", "RequestName",
        g_variant_new("(su)", daemon->name, daemon->request_flags),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    g_assert_no_error(error);
    guint result = 0;
    g_variant_get(reply, "(u)", &result);
    g_assert_cmpuint(result, ==, 1);
    g_async_queue_push(daemon->ready, daemon);
    g_main_loop_run(daemon->loop);
    g_dbus_interface_skeleton_unexport(G_DBUS_INTERFACE_SKELETON(player));
    g_dbus_interface_skeleton_unexport(G_DBUS_INTERFACE_SKELETON(root));
    g_assert_true(g_dbus_connection_close_sync(bus, NULL, &error));
    g_assert_no_error(error);
    g_main_loop_unref(daemon->loop);
    g_main_context_pop_thread_default(daemon->context);
    g_main_context_unref(daemon->context);
    return NULL;
}

static void start_player(PlayerDaemon *daemon, const gchar *address,
                          const gchar *name, const gchar *identity,
                          guint flags) {
    daemon->address = address;
    daemon->name = name;
    daemon->identity = identity;
    daemon->request_flags = flags;
    daemon->ready = g_async_queue_new();
    daemon->thread = g_thread_new("mpris-fixture", serve_player, daemon);
    g_assert_true(g_async_queue_timeout_pop(daemon->ready, 5000000) == daemon);
}

static gboolean stop_loop(gpointer data) {
    g_main_loop_quit(data);
    return G_SOURCE_REMOVE;
}

static void stop_player(PlayerDaemon *daemon) {
    g_main_context_invoke(daemon->context, stop_loop, daemon->loop);
    g_thread_join(daemon->thread);
    g_async_queue_unref(daemon->ready);
}

static gboolean wait_for_player(MediaPlayerService *service, const gchar *name,
                                 const gchar *identity) {
    gint64 deadline = g_get_monotonic_time() + 3000000;
    do {
        while (g_main_context_iteration(NULL, FALSE)) {}
        MediaPlayer *player = g_hash_table_lookup(service->players_by_name, name);
        if (player && g_strcmp0(player->identity, identity) == 0)
            return TRUE;
        g_usleep(1000);
    } while (g_get_monotonic_time() < deadline);
    return FALSE;
}

static GTestDBus *setup_bus(void) {
    GTestDBus *bus = g_test_dbus_new(G_TEST_DBUS_NONE);
    g_test_dbus_up(bus);
    fixture_bus = connect_bus(g_test_dbus_get_bus_address(bus));
    fixture_names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    return bus;
}

static void teardown_bus(GTestDBus *bus, MediaPlayerService *service) {
    g_autoptr(GError) error = NULL;
    g_assert_true(g_dbus_connection_close_sync(fixture_bus, NULL, &error));
    g_assert_no_error(error);
    while (g_main_context_iteration(NULL, FALSE)) {}
    g_clear_object(&service);
    g_clear_object(&fixture_bus);
    g_clear_pointer(&fixture_names, g_hash_table_unref);
    g_test_dbus_down(bus);
    g_object_unref(bus);
}

static void count_changed(MediaPlayerService *service, MediaPlayer *player,
                            guint *count) {
    g_assert_cmpstr(player->identity, ==, "Existing");
    ++*count;
}

static void discovers_existing_players(void) {
    GTestDBus *bus = setup_bus();
    PlayerDaemon daemon = {0}, decoy = {0};
    const gchar *name = "org.mpris.MediaPlayer2.fixture";
    start_player(&daemon, g_test_dbus_get_bus_address(bus), name, "Existing", 4);
    start_player(&decoy, g_test_dbus_get_bus_address(bus),
                   "org.mpris.MediaPlayer2Decoy.fixture", "Unrelated", 4);
    MediaPlayerService *service = g_object_new(MEDIA_PLAYER_SERVICE_TYPE, NULL);
    guint changed = 0;
    gulong handler = g_signal_connect(service, "media-player-changed",
                                       G_CALLBACK(count_changed), &changed);
    g_assert_true(wait_for_player(service, name, "Existing"));
    g_assert_cmpuint(changed, ==, 1);
    g_assert_cmpuint(g_hash_table_size(service->players_by_name), ==, 1);
    g_signal_handler_disconnect(service, handler);
    stop_player(&decoy);
    stop_player(&daemon);
    teardown_bus(bus, service);
}

static void count_removed(MediaPlayerService *service, MediaPlayer *player,
                            guint *count) {
    g_assert_cmpstr(player->identity, ==, "First owner");
    ++*count;
}

static void finalized(gpointer data, GObject *object) {
    *(gboolean *)data = TRUE;
}

static void discovers_replacement_owner(void) {
    GTestDBus *bus = setup_bus();
    MediaPlayerService *service = g_object_new(MEDIA_PLAYER_SERVICE_TYPE, NULL);
    PlayerDaemon first = {0}, second = {0};
    const gchar *name = "org.mpris.MediaPlayer2.fixture";
    const gchar *address = g_test_dbus_get_bus_address(bus);
    /* Allow replacement, then replace without releasing the first connection. */
    start_player(&first, address, name, "First owner", 1 | 4);
    g_assert_true(wait_for_player(service, name, "First owner"));
    MediaPlayer *old = g_hash_table_lookup(service->players_by_name, name);
    gboolean old_proxy_finalized = FALSE;
    g_object_weak_ref(G_OBJECT(old->player), finalized, &old_proxy_finalized);
    guint removed = 0;
    gulong handler = g_signal_connect(service, "media-player-removed",
                                       G_CALLBACK(count_removed), &removed);
    start_player(&second, address, name, "Second owner", 2 | 4);
    g_assert_true(wait_for_player(service, name, "Second owner"));
    g_assert_cmpuint(removed, ==, 1);
    g_assert_cmpuint(g_hash_table_size(service->players_by_name), ==, 1);
    g_assert_cmpuint(g_hash_table_size(service->players_by_proxy), ==, 1);
    g_assert_true(old_proxy_finalized);
    g_signal_handler_disconnect(service, handler);
    stop_player(&second);
    stop_player(&first);
    teardown_bus(bus, service);
}

static void drop_before_discovery(void) {
    GTestDBus *bus = setup_bus();
    PlayerDaemon daemon = {0};
    start_player(&daemon, g_test_dbus_get_bus_address(bus),
                   "org.mpris.MediaPlayer2.fixture", "Existing", 4);
    MediaPlayerService *service = g_object_new(MEDIA_PLAYER_SERVICE_TYPE, NULL);
    gboolean service_finalized = FALSE;
    g_object_weak_ref(G_OBJECT(service), finalized, &service_finalized);
    g_object_unref(service);
    g_assert_true(service_finalized);
    /* Dispatch any queued discovery callbacks after drop. */
    while (g_main_context_iteration(NULL, FALSE)) {}

    service = g_object_new(MEDIA_PLAYER_SERVICE_TYPE, NULL);
    g_assert_true(wait_for_player(service, "org.mpris.MediaPlayer2.fixture", "Existing"));
    MediaPlayer *player = g_hash_table_lookup(
        service->players_by_name, "org.mpris.MediaPlayer2.fixture");
    gboolean proxy_finalized = FALSE;
    g_object_weak_ref(G_OBJECT(player->player), finalized, &proxy_finalized);
    g_object_unref(service);
    g_assert_true(proxy_finalized);
    stop_player(&daemon);
    /* Dispatch owner-change callbacks after the populated service is gone. */
    gint64 deadline = g_get_monotonic_time() + 100000;
    while (g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        g_usleep(1000);
    }
    teardown_bus(bus, NULL);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/media-player/discovery-existing", discovers_existing_players);
    g_test_add_func("/media-player/discovery-replacement", discovers_replacement_owner);
    g_test_add_func("/media-player/discovery-drop-before-idle", drop_before_discovery);
    return g_test_run();
}
