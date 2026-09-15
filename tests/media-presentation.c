#include <adwaita.h>
#include <glib/gstdio.h>

static void fixture_file_read_async(GFile *file, int priority, GCancellable *cancel,
                                    GAsyncReadyCallback callback, gpointer data);
static GFileInputStream *fixture_file_read_finish(GFile *file, GAsyncResult *result,
                                                GError **error);
static GdkPixbuf *fixture_pixbuf_finish(GAsyncResult *result, GError **error);
static void fixture_pixbuf_async(GInputStream *stream, int width, int height,
                                 gboolean aspect, GCancellable *cancel,
                                 GAsyncReadyCallback callback, gpointer data);
#define g_file_read_async fixture_file_read_async
#define g_file_read_finish fixture_file_read_finish
#define gdk_pixbuf_new_from_stream_finish fixture_pixbuf_finish
#define gdk_pixbuf_new_from_stream_at_scale_async fixture_pixbuf_async
#include "../src/panel/message_tray/notifications/notification_widget.c"
#undef g_file_read_async
#undef g_file_read_finish
#undef gdk_pixbuf_new_from_stream_finish
#undef gdk_pixbuf_new_from_stream_at_scale_async

typedef struct { GObject parent_instance; } FixtureServices;
typedef struct { GObjectClass parent_class; } FixtureServicesClass;
G_DEFINE_TYPE(FixtureServices, fixture_services, G_TYPE_OBJECT)
static GObject *services;
static guint commands;
static void fixture_services_class_init(FixtureServicesClass *klass) {
    g_signal_new("message-tray-will-hide", G_TYPE_FROM_CLASS(klass),
                 G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_services_init(FixtureServices *self) {}
MessageTray *message_tray_get_global(void) { return (MessageTray *)services; }
MediaPlayerService *media_player_service_get_global(void) { return (MediaPlayerService *)services; }
NotificationsService *notifications_service_get_global(void) { return (NotificationsService *)services; }
#define COMMAND(name) \
    void name(MediaPlayerService *self, gchar *player) { \
        g_assert_cmpstr(player, ==, "org.mpris.MediaPlayer2.fixture"); commands++; \
    }
COMMAND(media_player_service_player_playpause)
COMMAND(media_player_service_player_previous)
COMMAND(media_player_service_player_next)
COMMAND(media_player_service_player_raise)
int notifications_service_closed_notification(NotificationsService *self, guint32 id,
                                              enum NotifcationsClosedReason reason) { return 0; }
int notifications_service_invoke_action(NotificationsService *self, guint32 id, char *action) { return 0; }
void notification_osd_hide(NotificationsOSD *self) {}

typedef struct { GTask *task; } ReadRequest;
static GPtrArray *reads;
static guint files_destroyed, streams_created, streams_destroyed;
static guint pixbufs_created, pixbufs_destroyed, reads_finished;
static guint cancellables_created, cancellables_destroyed;
static gboolean defer_decode;
static guint decodes_ready;
static GAsyncReadyCallback delayed_callback;
static gpointer delayed_data;
static GObject *delayed_source;
static GAsyncResult *delayed_result;
static gchar *directory, *red_file, *blue_file, *red_uri, *blue_uri;

static void counted(gpointer data, GObject *object) { (*(guint *)data)++; }
static void flagged(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void read_request_free(gpointer data) {
    ReadRequest *request = data;
    g_clear_object(&request->task);
    g_free(request);
}
static void fixture_file_read_async(GFile *file, int priority, GCancellable *cancel,
                                    GAsyncReadyCallback callback, gpointer data) {
    ReadRequest *request = g_new0(ReadRequest, 1);
    request->task = g_task_new(file, cancel, callback, data);
    // Deliberately return some successful reads after cancellation. The widget
    // must also reject stale generations and weak owners at callback time.
    g_task_set_check_cancellable(request->task, FALSE);
    g_object_weak_ref(G_OBJECT(file), counted, &files_destroyed);
    if (cancel) {
        cancellables_created++;
        g_object_weak_ref(G_OBJECT(cancel), counted, &cancellables_destroyed);
    }
    g_ptr_array_add(reads, request);
}
static GFileInputStream *fixture_file_read_finish(GFile *file, GAsyncResult *result,
                                                GError **error) {
    g_assert_true(g_task_get_source_object(G_TASK(result)) == file);
    reads_finished++;
    return g_task_propagate_pointer(G_TASK(result), error);
}
static GdkPixbuf *fixture_pixbuf_finish(GAsyncResult *result, GError **error) {
    GdkPixbuf *pixbuf = gdk_pixbuf_new_from_stream_finish(result, error);
    if (pixbuf) {
        pixbufs_created++;
        g_object_weak_ref(G_OBJECT(pixbuf), counted, &pixbufs_destroyed);
    }
    return pixbuf;
}
static void capture_decode(GObject *source, GAsyncResult *result, gpointer data) {
    g_assert_null(delayed_result);
    g_assert_true(G_IS_TASK(result));
    // A completed decode may win the cancellation race. Deliver its real
    // pixels later to verify the widget checks ownership and generation too.
    g_task_set_check_cancellable(G_TASK(result), FALSE);
    delayed_source = source ? g_object_ref(source) : NULL;
    delayed_result = g_object_ref(result);
    decodes_ready++;
}
static void fixture_pixbuf_async(GInputStream *stream, int width, int height,
                                 gboolean aspect, GCancellable *cancel,
                                 GAsyncReadyCallback callback, gpointer data) {
    if (defer_decode) {
        g_assert_null(delayed_callback);
        delayed_callback = callback;
        delayed_data = data;
        gdk_pixbuf_new_from_stream_at_scale_async(stream, width, height, aspect,
                                                 cancel, capture_decode, NULL);
    } else {
        gdk_pixbuf_new_from_stream_at_scale_async(stream, width, height, aspect,
                                                 cancel, callback, data);
    }
}
static void release_decode(void) {
    g_assert_nonnull(delayed_result);
    GAsyncReadyCallback callback = delayed_callback;
    gpointer data = delayed_data;
    GObject *source = g_steal_pointer(&delayed_source);
    GAsyncResult *result = g_steal_pointer(&delayed_result);
    delayed_callback = NULL;
    delayed_data = NULL;
    callback(source, result, data);
    g_clear_object(&source);
    g_object_unref(result);
}
static void complete_read(guint index, gboolean fail) {
    ReadRequest *request = g_ptr_array_index(reads, index);
    GTask *task = g_steal_pointer(&request->task);
    g_assert_nonnull(task);
    if (fail) {
        g_task_return_new_error(task, G_IO_ERROR, G_IO_ERROR_FAILED, "Fixture read failure");
    } else {
        g_autoptr(GError) error = NULL;
        GFileInputStream *stream = g_file_read(g_task_get_source_object(task), NULL, &error);
        g_assert_no_error(error);
        streams_created++;
        g_object_weak_ref(G_OBJECT(stream), counted, &streams_destroyed);
        g_task_return_pointer(task, stream, g_object_unref);
    }
    g_object_unref(task);
}
static void wait_count(guint *counter, guint expected) {
    gint64 deadline = g_get_monotonic_time() + 5 * G_TIME_SPAN_SECOND;
    while (*counter < expected && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        if (*counter < expected) g_usleep(1000);
    }
    g_assert_cmpuint(*counter, ==, expected);
}
static void wait_image(NotificationWidget *widget) {
    gint64 deadline = g_get_monotonic_time() + 5 * G_TIME_SPAN_SECOND;
    while (!adw_avatar_get_custom_image(widget->avatar) && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        if (!adw_avatar_get_custom_image(widget->avatar)) g_usleep(1000);
    }
    g_assert_nonnull(adw_avatar_get_custom_image(widget->avatar));
}
static void assert_color(NotificationWidget *widget, gboolean blue) {
    GdkPaintable *image = adw_avatar_get_custom_image(widget->avatar);
    g_assert_true(GDK_IS_TEXTURE(image));
    g_autoptr(GdkTextureDownloader) download = gdk_texture_downloader_new(GDK_TEXTURE(image));
    gdk_texture_downloader_set_format(download, GDK_MEMORY_R8G8B8A8);
    gsize stride;
    g_autoptr(GBytes) pixels = gdk_texture_downloader_download_bytes(download, &stride);
    const guchar *pixel = g_bytes_get_data(pixels, NULL);
    g_assert_cmpuint(pixel[0], ==, blue ? 0 : 255);
    g_assert_cmpuint(pixel[1], ==, 0);
    g_assert_cmpuint(pixel[2], ==, blue ? 255 : 0);
}
static void setup(void) {
    services = g_object_new(fixture_services_get_type(), NULL);
    reads = g_ptr_array_new_with_free_func(read_request_free);
    commands = files_destroyed = streams_created = streams_destroyed = 0;
    pixbufs_created = pixbufs_destroyed = reads_finished = 0;
    cancellables_created = cancellables_destroyed = 0;
    defer_decode = FALSE;
    decodes_ready = 0;
    g_autoptr(GError) error = NULL;
    directory = g_dir_make_tmp("way-shell-media-images.XXXXXX", &error);
    g_assert_no_error(error);
    red_file = g_build_filename(directory, "red.png", NULL);
    blue_file = g_build_filename(directory, "blue.png", NULL);
    g_autoptr(GdkPixbuf) pixbuf = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, 4, 4);
    gdk_pixbuf_fill(pixbuf, 0xff0000ff);
    g_assert_true(gdk_pixbuf_save(pixbuf, red_file, "png", &error, NULL));
    g_assert_no_error(error);
    gdk_pixbuf_fill(pixbuf, 0x0000ffff);
    g_assert_true(gdk_pixbuf_save(pixbuf, blue_file, "png", &error, NULL));
    g_assert_no_error(error);
    red_uri = g_filename_to_uri(red_file, NULL, &error);
    g_assert_no_error(error);
    blue_uri = g_filename_to_uri(blue_file, NULL, &error);
    g_assert_no_error(error);
}
static void teardown(void) {
    g_assert_null(delayed_callback);
    g_assert_null(delayed_result);
    for (guint i = 0; i < reads->len; i++)
        g_assert_null(((ReadRequest *)g_ptr_array_index(reads, i))->task);
    g_ptr_array_unref(reads);
    g_object_unref(services);
    g_remove(red_file); g_remove(blue_file); g_rmdir(directory);
    g_free(red_file); g_free(blue_file); g_free(directory);
    g_free(red_uri); g_free(blue_uri);
}
static MediaPlayer player(void) {
    return (MediaPlayer){.name = "org.mpris.MediaPlayer2.fixture",
        .playback_status = "Playing", .artist = "Fixture artist", .title = "First track"};
}

static void media_dispose_is_idempotent_and_buttons_are_weak(void) {
    setup();
    MediaPlayer media = player();
    NotificationWidget *widget = notification_widget_from_media_player(&media);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    GtkButton *buttons[] = {widget->button, widget->play_pause, widget->previous,
                           widget->next, widget->header_expand};
    for (guint i = 0; i < 4; i++)
        g_signal_emit_by_name(buttons[i], "clicked");
    g_assert_cmpuint(commands, ==, 4);
    commands = 0;
    gboolean gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget), flagged, &gone);
    g_object_run_dispose(G_OBJECT(widget));
    g_object_run_dispose(G_OBJECT(widget));
    gpointer owner = widget;
    g_object_unref(widget);
    g_assert_true(gone);
    g_assert_null(g_object_get_data(G_OBJECT(container), "self"));
    for (guint i = 0; i < G_N_ELEMENTS(buttons); i++) {
        g_assert_cmpuint(g_signal_handlers_block_matched(buttons[i], G_SIGNAL_MATCH_DATA,
                          0, 0, NULL, NULL, owner), ==, 0);
        g_signal_emit_by_name(buttons[i], "clicked");
    }
    g_assert_cmpuint(commands, ==, 0);
    g_object_unref(container);
    teardown();
}

static void missing_art_clears_the_previous_track(void) {
    setup();
    MediaPlayer media = player();
    media.art_url = red_uri;
    NotificationWidget *widget = notification_widget_from_media_player(&media);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    notification_widget_set_media_player(widget, &media);
    complete_read(0, FALSE);
    wait_image(widget);
    assert_color(widget, FALSE);
    media.art_url = NULL;
    media.artist = NULL;
    media.title = NULL;
    notification_widget_set_media_player(widget, &media);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));
    g_assert_cmpstr(gtk_label_get_text(widget->summary), ==, "");
    g_assert_cmpstr(gtk_label_get_text(widget->body), ==, "");
    g_object_unref(widget);
    g_object_unref(container);
    wait_count(&files_destroyed, 1);
    wait_count(&streams_destroyed, streams_created);
    wait_count(&pixbufs_destroyed, pixbufs_created);
    teardown();
}

static void newer_art_and_owner_removal_win_over_late_reads(void) {
    setup();
    MediaPlayer media = player();
    NotificationWidget *widget = notification_widget_from_media_player(&media);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    media.art_url = red_uri;
    notification_widget_set_media_player(widget, &media);
    media.art_url = blue_uri;
    notification_widget_set_media_player(widget, &media);
    complete_read(1, FALSE);
    wait_image(widget);
    assert_color(widget, TRUE);
    complete_read(0, FALSE);
    wait_count(&reads_finished, 2);
    assert_color(widget, TRUE);

    media.art_url = red_uri;
    notification_widget_set_media_player(widget, &media);
    media.art_url = NULL;
    notification_widget_set_media_player(widget, &media);
    complete_read(2, FALSE);
    wait_count(&reads_finished, 3);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));

    media.art_url = blue_uri;
    notification_widget_set_media_player(widget, &media);
    gboolean gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget), flagged, &gone);
    g_object_unref(widget);
    g_assert_true(gone);
    complete_read(3, FALSE);
    wait_count(&reads_finished, 4);
    g_object_unref(container);
    wait_count(&files_destroyed, 4);
    wait_count(&streams_destroyed, streams_created);
    wait_count(&pixbufs_destroyed, pixbufs_created);
    g_assert_cmpuint(cancellables_created, ==, 4);
    wait_count(&cancellables_destroyed, cancellables_created);
    teardown();
}

static void cleared_art_and_owner_removal_win_over_queued_decodes(void) {
    setup();
    defer_decode = TRUE;
    MediaPlayer media = player();
    NotificationWidget *widget = notification_widget_from_media_player(&media);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    AdwAvatar *avatar = widget->avatar;
    media.art_url = red_uri;
    g_assert_true(notification_widget_set_media_player(widget, &media) == widget);
    complete_read(0, FALSE);
    wait_count(&decodes_ready, 1);
    media.art_url = NULL;
    notification_widget_set_media_player(widget, &media);
    release_decode();
    g_assert_null(adw_avatar_get_custom_image(avatar));

    media.art_url = blue_uri;
    notification_widget_set_media_player(widget, &media);
    complete_read(1, FALSE);
    wait_count(&decodes_ready, 2);
    gboolean gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget), flagged, &gone);
    g_object_unref(widget);
    g_assert_true(gone);
    release_decode();
    g_assert_null(adw_avatar_get_custom_image(avatar));
    g_object_unref(container);
    wait_count(&files_destroyed, 2);
    wait_count(&streams_destroyed, streams_created);
    g_assert_cmpuint(pixbufs_created, ==, 2);
    wait_count(&pixbufs_destroyed, pixbufs_created);
    wait_count(&cancellables_destroyed, cancellables_created);
    teardown();
}

static void artwork_errors_release_resources_and_allow_recovery(void) {
    setup();
    MediaPlayer media = player();
    NotificationWidget *widget = notification_widget_from_media_player(&media);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    media.art_url = red_uri;
    notification_widget_set_media_player(widget, &media);
    complete_read(0, TRUE);
    wait_count(&cancellables_destroyed, 1);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));

    g_autoptr(GError) error = NULL;
    g_assert_true(g_file_set_contents(blue_file, "Not an image", -1, &error));
    g_assert_no_error(error);
    media.art_url = blue_uri;
    notification_widget_set_media_player(widget, &media);
    complete_read(1, FALSE);
    wait_count(&cancellables_destroyed, 2);
    g_assert_null(adw_avatar_get_custom_image(widget->avatar));

    media.art_url = red_uri;
    notification_widget_set_media_player(widget, &media);
    complete_read(2, FALSE);
    wait_image(widget);
    assert_color(widget, FALSE);
    g_object_unref(widget);
    g_object_unref(container);
    wait_count(&files_destroyed, 3);
    wait_count(&streams_destroyed, streams_created);
    wait_count(&pixbufs_destroyed, pixbufs_created);
    wait_count(&cancellables_destroyed, 3);
    teardown();
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/media-presentation/dispose", media_dispose_is_idempotent_and_buttons_are_weak);
    g_test_add_func("/media-presentation/clear-art", missing_art_clears_the_previous_track);
    g_test_add_func("/media-presentation/late-art", newer_art_and_owner_removal_win_over_late_reads);
    g_test_add_func("/media-presentation/queued-decode", cleared_art_and_owner_removal_win_over_queued_decodes);
    g_test_add_func("/media-presentation/errors-recovery", artwork_errors_release_resources_and_allow_recovery);
    return g_test_run();
}
