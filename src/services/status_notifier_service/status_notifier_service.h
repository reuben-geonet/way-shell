#pragma once

#include <adwaita.h>

#define SNI_GACTION_PREFIX "sni"
#define SNI_GRACTION_ITEM_CLICKED "sni.item-clicked"
#define SNI_GRACTION_MENU_ABOUT_TO_SHOW "sni.about-to-show"

// Borrowed Rust-owned record, valid through update/removal signal delivery.
// Do not mutate or free fields. All calls run on the application GLib thread.
typedef struct StatusNotifierItem {
    gpointer proxy;      // Reserved ABI slot; always NULL.
    gpointer menu_proxy; // Reserved ABI slot; always NULL.
    GActionGroup *action_group;
    GMenu *menu_model;
    gchar *bus_name;
    gchar *obj_name;
    gchar *register_service_name;

    // item properties
    gchar *category;
    gchar *id;
    gchar *title;
    gchar *status;
    guint window_id;

    GdkPixbuf *icon_pixmap_from_theme;
    gchar *icon_theme_path;
    gchar *icon_name;
    GdkPixbuf *icon_pixmap;

    gchar *overlay_icon_name;
    GdkPixbuf *overlay_icon_pixmap;

    gchar *attention_icon_name;
    GdkPixbuf *attention_icon_pixmap;
    gchar *attention_movie_name;
} StatusNotifierItem;

void status_notifier_item_about_to_show(StatusNotifierItem *self,
                                        gint32 menu_item_id);

const gchar *status_notifier_item_get_key(StatusNotifierItem *self);
void status_notifier_item_activate(StatusNotifierItem *self, gint32 x, gint32 y);
void status_notifier_item_secondary_activate(StatusNotifierItem *self, gint32 x,
                                             gint32 y);
void status_notifier_item_context_menu(StatusNotifierItem *self, gint32 x,
                                       gint32 y);
void status_notifier_item_scroll(StatusNotifierItem *self, gint32 delta,
                                gboolean horizontal);
const gchar *status_notifier_item_get_category(StatusNotifierItem *self);
const gchar *status_notifier_item_get_id(StatusNotifierItem *self);
const gchar *status_notifier_item_get_title(StatusNotifierItem *self);
const gchar *status_notifier_item_get_status(StatusNotifierItem *self);
const int status_notifier_item_get_window_id(StatusNotifierItem *self);
const gchar *status_notifier_item_get_icon_name(StatusNotifierItem *self);
GdkPixbuf *status_notifier_item_get_icon_pixmap(StatusNotifierItem *self);
const gchar *status_notifier_item_get_overlay_icon_name(
    StatusNotifierItem *self);
GdkPixbuf *status_notifier_item_get_overlay_icon_pixmap(
    StatusNotifierItem *self);
const gchar *status_notifier_item_get_attention_icon_name(
    StatusNotifierItem *self);
GdkPixbuf *status_notifier_item_get_attention_icon_pixmap(
    StatusNotifierItem *self);
const gchar *status_notifier_item_get_attention_movie_name(
    StatusNotifierItem *self);
// TODO: get_tooltip
const gboolean status_notifier_item_get_item_is_menu(StatusNotifierItem *self);
const gchar *status_notifier_item_get_menu(StatusNotifierItem *self);

G_BEGIN_DECLS

// Temporary C facade for the Rust StatusNotifier watcher and item service.
struct _StatusNotifierService;
#define STATUS_NOTIFIER_SERVICE_TYPE status_notifier_service_get_type()
G_DECLARE_FINAL_TYPE(StatusNotifierService, status_notifier_service,
                     STATUS_NOTIFIER, SERVICE, GObject);

G_END_DECLS

int status_notifier_service_global_init();

// Borrowed table keyed by unique bus owner plus object path. Do not mutate.
GHashTable *status_notifier_service_get_items(StatusNotifierService *self);

StatusNotifierService *status_notifier_service_get_global();
