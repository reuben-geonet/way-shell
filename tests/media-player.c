/* Baseline MPRIS metadata regressions, independent of a running bus or display. */
#include "../src/services/media_player_service/media_player_service.c"

static void clear_metadata(MediaPlayer *player) {
    g_clear_pointer(&player->album, g_free);
    g_clear_pointer(&player->title, g_free);
    g_clear_pointer(&player->artist, g_free);
    g_clear_pointer(&player->art_url, g_free);
}

static GVariant *track_metadata(void) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "xesam:album", g_variant_new_string("Album"));
    g_variant_builder_add(&builder, "{sv}", "xesam:title", g_variant_new_string("Track"));
    const gchar *artists[] = {"First", "Second", NULL};
    g_variant_builder_add(&builder, "{sv}", "xesam:artist", g_variant_new_strv(artists, -1));
    g_variant_builder_add(&builder, "{sv}", "mpris:artUrl", g_variant_new_string("file:///tmp/cover.png"));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void valid_metadata(void) {
    MediaPlayer player = {0};
    g_autoptr(GVariant) metadata = track_metadata();
    media_player_fill_metadata(metadata, &player);
    g_assert_cmpstr(player.album, ==, "Album");
    g_assert_cmpstr(player.title, ==, "Track");
    g_assert_cmpstr(player.artist, ==, "First, Second");
    g_assert_cmpstr(player.art_url, ==, "file:///tmp/cover.png");
    clear_metadata(&player);
}

static void replacement_clears_omitted_fields(void) {
    MediaPlayer player = {0};
    g_autoptr(GVariant) first = track_metadata();
    media_player_fill_metadata(first, &player);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "xesam:title", g_variant_new_string("Next track"));
    g_autoptr(GVariant) next = g_variant_ref_sink(g_variant_builder_end(&builder));
    media_player_fill_metadata(next, &player);
    g_assert_cmpstr(player.title, ==, "Next track");
    g_assert_null(player.album);
    g_assert_null(player.artist);
    g_assert_null(player.art_url);
    clear_metadata(&player);
}

static void empty_artists(void) {
    MediaPlayer player = {0};
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "xesam:artist", g_variant_new_strv(NULL, 0));
    g_autoptr(GVariant) metadata = g_variant_ref_sink(g_variant_builder_end(&builder));
    media_player_fill_metadata(metadata, &player);
    g_assert_null(player.artist);
    clear_metadata(&player);
}

static void wrong_metadata_types(void) {
    MediaPlayer player = {0};
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "xesam:album", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&builder, "{sv}", "xesam:title", g_variant_new_int32(17));
    g_variant_builder_add(&builder, "{sv}", "xesam:artist", g_variant_new_string("invalid artist array"));
    g_variant_builder_add(&builder, "{sv}", "mpris:artUrl", g_variant_new_uint32(18));
    g_autoptr(GVariant) metadata = g_variant_ref_sink(g_variant_builder_end(&builder));
    media_player_fill_metadata(metadata, &player);
    g_assert_null(player.album);
    g_assert_null(player.title);
    g_assert_null(player.artist);
    g_assert_null(player.art_url);
    clear_metadata(&player);

    g_autoptr(GVariant) wrong_container =
        g_variant_ref_sink(g_variant_new_string("invalid metadata dictionary"));
    media_player_fill_metadata(wrong_container, &player);
    g_assert_null(player.album);
    g_assert_null(player.title);
    g_assert_null(player.artist);
    g_assert_null(player.art_url);
}

static void absent_metadata(void) {
    MediaPlayer player = {0};
    g_autoptr(GVariant) first = track_metadata();
    media_player_fill_metadata(first, &player);
    media_player_fill_metadata(NULL, &player);
    g_assert_null(player.album);
    g_assert_null(player.title);
    g_assert_null(player.artist);
    g_assert_null(player.art_url);
    clear_metadata(&player);
}

typedef struct {
    gpointer bytes;
    guint releases;
} MetadataStorage;

static void metadata_storage_released(gpointer data) {
    MetadataStorage *storage = data;
    storage->releases++;
    g_free(storage->bytes);
}

static void metadata_is_borrowed_without_retaining_it(void) {
    MediaPlayer player = {0};
    g_autoptr(GVariant) original = track_metadata();
    gsize size = g_variant_get_size(original);
    MetadataStorage storage = {.bytes=g_malloc(size)};
    g_variant_store(original, storage.bytes);
    GVariant *metadata = g_variant_ref_sink(g_variant_new_from_data(
        G_VARIANT_TYPE_VARDICT, storage.bytes, size, TRUE,
        metadata_storage_released, &storage));
    media_player_fill_metadata(metadata, &player);
    g_assert_cmpuint(storage.releases, ==, 0);
    g_variant_unref(metadata);
    g_assert_cmpuint(storage.releases, ==, 1);
    /* The copied domain fields remain owned after releasing the variant. */
    g_assert_cmpstr(player.title, ==, "Track");
    g_assert_cmpstr(player.artist, ==, "First, Second");
    clear_metadata(&player);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/media-player/metadata-valid", valid_metadata);
    g_test_add_func("/media-player/metadata-replacement", replacement_clears_omitted_fields);
    g_test_add_func("/media-player/metadata-empty-artists", empty_artists);
    g_test_add_func("/media-player/metadata-wrong-types", wrong_metadata_types);
    g_test_add_func("/media-player/metadata-absent", absent_metadata);
    g_test_add_func("/media-player/metadata-ownership", metadata_is_borrowed_without_retaining_it);
    return g_test_run();
}
