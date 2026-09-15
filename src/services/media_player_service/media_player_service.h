#pragma once

#include <adwaita.h>

typedef struct _MediaPlayer {
    // Reserved compatibility slots. Rust owns the native proxies privately.
    gpointer player;
    gpointer proxy;
    gchar *identity;
    gchar *name;
    gchar *playback_status;
    gchar *art_url;
    gchar *album;
    gchar *artist;
    gchar *title;
} MediaPlayer;

G_BEGIN_DECLS

struct _MediaPlayerService;
#define MEDIA_PLAYER_SERVICE_TYPE media_player_service_get_type()
G_DECLARE_FINAL_TYPE(MediaPlayerService, media_player_service, MEDIA_PLAYER,
                     SERVICE, GObject);

G_END_DECLS

int media_player_service_global_init(void);

MediaPlayerService *media_player_service_get_global();

// Borrowed array and records, valid until the next service update. Do not free.
GPtrArray *media_player_service_get_players(MediaPlayerService *self);

// Commands copy the well-known org.mpris.MediaPlayer2.* name before returning.
void media_player_service_player_play(MediaPlayerService *self, gchar *name);

void media_player_service_player_pause(MediaPlayerService *self, gchar *name);

void media_player_service_player_playpause(MediaPlayerService *self,
                                           gchar *name);

void media_player_service_player_stop(MediaPlayerService *self, gchar *name);

void media_player_service_player_next(MediaPlayerService *self, gchar *name);

void media_player_service_player_previous(MediaPlayerService *self,
                                          gchar *name);

void media_player_service_player_raise(MediaPlayerService *self, gchar *name);
