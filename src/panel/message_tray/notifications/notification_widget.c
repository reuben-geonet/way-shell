#include "notification_widget.h"

#include <adwaita.h>
#include <string.h>

#include "../../../services/media_player_service/media_player_service.h"
#include "../message_tray.h"
#include "glib-object.h"
#include "glib.h"
#include "gtk/gtk.h"
#include "notification_osd.h"

enum signals { notification_expanded, notification_collapsed, signals_n };

typedef struct _NotificationWidget {
    GObject parent_instance;
    guint32 id;
    AdwAnimation *expand_animation;
    GtkEventControllerMotion *ctrl;
    GtkBox *container;
    GtkBox *notification_container;
    GtkBox *stack_effect_box1;
    GtkBox *stack_effect_box2;

    // header
    GtkCenterBox *header;
    GtkImage *header_app_icon;
    GtkLabel *header_app_name;
    GtkLabel *header_timer;
    GtkButton *header_expand;
    GtkButton *header_dismiss;

    // notification button
    GtkBox *button_container;
    GtkBox *button_contents;
    GtkButton *button;
    AdwAvatar *avatar;
    GtkImage *icon;
    // button text
    GtkBox *text;
    GtkLabel *summary;
    GtkLabel *body;

    // media player buttons, if notification is a media player.
    GtkBox *media_buttons;
    GtkButton *play_pause;
    GtkButton *previous;
    GtkButton *next;

    // action revealer
    GtkRevealer *action_revealer;
    // container which holds action buttons
    GtkBox *action_container;
    GtkButton *osd_hide_button;
    // count of action buttons used for applying the correct css.
    int actions_buttons_n;

    // properties
    gboolean expanded;
    GDateTime *created_on;
    guint32 timer_id;
    // mpris media player name, if null, notification is not a media player.
    gchar *media_player_name;
    GCancellable *artwork_cancellable;
    guint64 artwork_generation;
    gboolean disposed;
    NotificationsOSD *osd;
} NotificationWidget;

static guint notification_widget_signals[signals_n] = {0};
G_DEFINE_TYPE(NotificationWidget, notification_widget, G_TYPE_OBJECT);

void on_pointer_enter(GtkEventControllerMotion *ctrl, double x, double y,
                      NotificationWidget *self) {
    g_debug("notification_widget.c:on_pointer_enter() called");
    if (self->action_revealer) {
        gtk_revealer_set_reveal_child(self->action_revealer, true);
    }

    adw_timed_animation_set_reverse(ADW_TIMED_ANIMATION(self->expand_animation),
                                    false);

    adw_animation_reset(self->expand_animation);
    adw_animation_play(self->expand_animation);
}

void on_pointer_leave(GtkEventControllerMotion *ctrl, double x, double y,
                      NotificationWidget *self) {
    if (self->action_revealer) {
        gtk_revealer_set_reveal_child(self->action_revealer, false);
    }

    adw_timed_animation_set_reverse(ADW_TIMED_ANIMATION(self->expand_animation),
                                    true);

    adw_animation_reset(self->expand_animation);
    adw_animation_play(self->expand_animation);
}

void on_dismiss_clicked(GtkButton *button, NotificationWidget *self) {
    g_debug("notification_widget.c:on_dismiss_clicked() called");

    NotificationsService *service = notifications_service_get_global();
    notifications_service_closed_notification(
        service, self->id, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
}

void on_notification_clicked(GtkButton *button, NotificationWidget *self) {
    g_debug("notification_widget.c:on_notification_clicked() called");

    NotificationsService *service = notifications_service_get_global();
    notifications_service_invoke_action(service, self->id, "default");
    // dimiss it after click
    notifications_service_closed_notification(
        service, self->id, NOTIFICATIONS_CLOSED_REASON_REQUESTED);
}

static void on_message_tray_will_hide(MessageTray *tray,
                                      NotificationWidget *self);

static void cancel_media_artwork(NotificationWidget *self) {
    self->artwork_generation++;
    if (self->artwork_cancellable)
        g_cancellable_cancel(self->artwork_cancellable);
    g_clear_object(&self->artwork_cancellable);
}
static void load_artwork(NotificationWidget *self, const gchar *location);

// stub out dispose, finalize, class_init and init methods.
static void notification_widget_dispose(GObject *gobject) {
    NotificationWidget *self = NOTIFICATION_WIDGET(gobject);

    // debug with pointer value as hex
    g_debug("notification_widget.c:notification_widget_dispose() called: %p",
            self);

    MessageTray *mt = message_tray_get_global();
    if (mt)
        g_signal_handlers_disconnect_by_func(mt, on_message_tray_will_hide, self);

    self->disposed = TRUE;
    cancel_media_artwork(self);

    // kill timer
    if (self->timer_id) {
        g_source_remove(self->timer_id);
        self->timer_id = 0;
    }

    // unref our ref'd datetime.
    g_clear_pointer(&self->created_on, g_date_time_unref);
    g_clear_pointer(&self->media_player_name, g_free);
    if (self->osd) {
        g_object_remove_weak_pointer(G_OBJECT(self->osd), (gpointer *)&self->osd);
        self->osd = NULL;
    }
    if (self->expand_animation) {
        g_signal_handlers_disconnect_by_data(self->expand_animation, self);
        adw_animation_pause(self->expand_animation);
        g_clear_object(&self->expand_animation);
    }
    if (self->container)
        g_object_set_data(G_OBJECT(self->container), "self", NULL);
    g_clear_object(&self->container);

    // Chain-up
    G_OBJECT_CLASS(notification_widget_parent_class)->dispose(gobject);
};

static void notification_widget_finalize(GObject *gobject) {
    // Chain-up
    G_OBJECT_CLASS(notification_widget_parent_class)->finalize(gobject);
};

static void notification_widget_class_init(NotificationWidgetClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = notification_widget_dispose;
    object_class->finalize = notification_widget_finalize;

    notification_widget_signals[notification_expanded] =
        g_signal_new("notification-expanded", G_TYPE_FROM_CLASS(klass),
                     G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);

    notification_widget_signals[notification_collapsed] =
        g_signal_new("notification-collapsed", G_TYPE_FROM_CLASS(klass),
                     G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
};

static void expand_animation_cb(double value, GtkLabel *body) {
    gtk_label_set_lines(body, value);
}

static void on_expand_button_clicked(GtkButton *button,
                                     NotificationWidget *self);

static void on_expand_animation_done(AdwAnimation *animation,
                                     NotificationWidget *self) {
    g_debug("notification_widget.c:on_expand_animation_done() called");

    gboolean reverse =
        adw_timed_animation_get_reverse(ADW_TIMED_ANIMATION(animation));

    if (reverse) {
        g_debug(
            "notification_widget.c:on_expand_animation_done() called: "
            "reverse");
        g_signal_emit(self, notification_widget_signals[notification_collapsed],
                      0);
    } else {
        g_debug(
            "notification_widget.c:on_expand_animation_done() called: "
            "not reverse");
        g_signal_emit(self, notification_widget_signals[notification_expanded],
                      0);
    }
}

static void on_message_tray_will_hide(MessageTray *tray,
                                      NotificationWidget *self) {
    g_debug("notification_widget.c:on_message_tray_will_hide() called");
    if (self->expanded) {
        gtk_button_set_icon_name(self->header_expand, "go-down-symbolic");
        adw_timed_animation_set_reverse(
            ADW_TIMED_ANIMATION(self->expand_animation), true);
        adw_animation_play(self->expand_animation);
        if (self->action_revealer)
            gtk_revealer_set_reveal_child(self->action_revealer, false);
        self->expanded = !self->expanded;
    }
}

static void on_action_button_clicked(GtkButton *button,
                                     NotificationWidget *self) {
    g_debug("notification_widget.c:on_action_button_clicked() called");

    NotificationsService *service = notifications_service_get_global();

    gchar *action = g_object_get_data(G_OBJECT(button), "action");
    if (!action) return;

    notifications_service_invoke_action(service, self->id, action);
}

static void configure_action_revealer(NotificationWidget *self) {
    self->action_revealer = GTK_REVEALER(gtk_revealer_new());
    gtk_revealer_set_transition_type(self->action_revealer,
                                     GTK_REVEALER_TRANSITION_TYPE_SLIDE_DOWN);

    GtkCenterBox *center = GTK_CENTER_BOX(gtk_center_box_new());

    // hexpand center box
    gtk_widget_set_hexpand(GTK_WIDGET(center), true);

    self->action_container =
        GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));

    // hexpand action container
    gtk_widget_set_hexpand(GTK_WIDGET(self->action_container), true);

    // set container as child of revealer
    gtk_revealer_set_child(self->action_revealer, GTK_WIDGET(center));

    gtk_center_box_set_center_widget(center,
                                     GTK_WIDGET(self->action_container));

    // append revealer to notification container
    gtk_box_append(self->notification_container,
                   GTK_WIDGET(self->action_revealer));
}

static void notification_widget_from_notification_action_buttons(
    Notification *n, NotificationWidget *self) {
    for (guint i = 0; n->actions && n->actions[i] && n->actions[i + 1]; i += 2) {
        // ignore the 'default' action, this will always be called by clicking
        // the notification body.
        if (n->actions[i] && strcmp(n->actions[i], "default") == 0) {
            continue;
        }

        // ensure we have a user-facing string to display.
        // see:
        // https://specifications.freedesktop.org/notification-spec/notification-spec-latest.html
        if (!n->actions[i + 1] || strlen(n->actions[i + 1]) == 0) {
            continue;
        }

        // create GtkRevealer and GtkBox if we need to.
        if (!self->action_revealer) {
            configure_action_revealer(self);
        }

        // Add action button
        GtkButton *action_button =
            GTK_BUTTON(gtk_button_new_with_label(n->actions[i + 1]));

        // hexpand button
        gtk_widget_set_hexpand(GTK_WIDGET(action_button), true);

        gtk_widget_add_css_class(GTK_WIDGET(action_button),
                                 "notification-widget-action-button");

        g_object_set_data_full(G_OBJECT(action_button), "action",
                               g_strdup(n->actions[i]), g_free);

        g_signal_connect_object(action_button, "clicked",
                                G_CALLBACK(on_action_button_clicked), self, 0);

        gtk_box_append(self->action_container, GTK_WIDGET(action_button));
        self->actions_buttons_n++;

    }
}

static void notification_widget_init_layout(NotificationWidget *self) {
    // main container for widget
    // Keep children alive for asynchronous artwork callbacks while their
    // controller lives, including a containing window being rebuilt.
    self->container = g_object_ref_sink(GTK_BOX(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0)));
    gtk_widget_add_css_class(GTK_WIDGET(self->container),
                             "notification-widget-container");

    self->notification_container =
        GTK_BOX(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->notification_container),
                             "notification-widget");

    // set size request
    gtk_widget_set_size_request(GTK_WIDGET(self->container), 400, 120);

    // motion controller to get mouse in/out events
    self->ctrl = GTK_EVENT_CONTROLLER_MOTION(gtk_event_controller_motion_new());
    gtk_widget_add_controller(GTK_WIDGET(self->container),
                              GTK_EVENT_CONTROLLER(self->ctrl));

    // setup header
    self->header = GTK_CENTER_BOX(gtk_center_box_new());
    gtk_widget_add_css_class(GTK_WIDGET(self->header),
                             "notification-widget-header");
    GtkBox *header_left = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_set_valign(GTK_WIDGET(header_left), GTK_ALIGN_CENTER);

    GtkBox *header_right = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_set_valign(GTK_WIDGET(header_right), GTK_ALIGN_CENTER);

    gtk_center_box_set_start_widget(self->header, GTK_WIDGET(header_left));
    gtk_center_box_set_end_widget(self->header, GTK_WIDGET(header_right));

    self->header_app_icon = GTK_IMAGE(gtk_image_new());
    gtk_widget_add_css_class(GTK_WIDGET(self->header_app_icon),
                             "notification-widget-app-icon");
    gtk_image_set_pixel_size(self->header_app_icon, 18);
    gtk_widget_set_halign(GTK_WIDGET(self->header_app_icon), GTK_ALIGN_START);
    gtk_widget_add_css_class(GTK_WIDGET(self->header_app_icon),
                             "notification-widget-icon");

    self->header_app_name = GTK_LABEL(gtk_label_new(""));
    gtk_widget_add_css_class(GTK_WIDGET(self->header_app_name),
                             "notification-widget-app-name");
    gtk_widget_set_valign(GTK_WIDGET(self->header_app_name), GTK_ALIGN_CENTER);

    self->header_timer = GTK_LABEL(gtk_label_new("Just now"));
    gtk_widget_add_css_class(GTK_WIDGET(self->header_timer),
                             "notification-widget-timer");
    gtk_widget_set_valign(GTK_WIDGET(self->header_timer), GTK_ALIGN_END);

    self->header_expand =
        GTK_BUTTON(gtk_button_new_from_icon_name("go-down-symbolic"));
    gtk_widget_add_css_class(GTK_WIDGET(self->header_expand), "circular");
    gtk_widget_add_css_class(GTK_WIDGET(self->header_expand),
                             "notification-widget-expand-button");
    gtk_widget_set_visible(GTK_WIDGET(self->header_expand), false);
    g_signal_connect_object(self->header_expand, "clicked",
                            G_CALLBACK(on_expand_button_clicked), self, 0);

    GtkImage *header_dismiss_icon =
        GTK_IMAGE(gtk_image_new_from_icon_name("window-close-symbolic"));
    gtk_image_set_pixel_size(header_dismiss_icon, 18);
    self->header_dismiss = GTK_BUTTON(gtk_button_new());
    gtk_button_set_child(self->header_dismiss, GTK_WIDGET(header_dismiss_icon));
    gtk_widget_add_css_class(GTK_WIDGET(self->header_dismiss), "circular");
    gtk_widget_add_css_class(GTK_WIDGET(self->header_dismiss),
                             "notification-widget-dismiss-button");

    gtk_box_append(header_left, GTK_WIDGET(self->header_app_icon));
    gtk_box_append(header_left, GTK_WIDGET(self->header_app_name));
    gtk_box_append(header_left, GTK_WIDGET(self->header_timer));
    gtk_box_append(header_right, GTK_WIDGET(self->header_expand));
    gtk_box_append(header_right, GTK_WIDGET(self->header_dismiss));

    self->button_container =
        GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));

    self->button_contents = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));

    self->button = GTK_BUTTON(gtk_button_new());
    gtk_button_set_child(self->button, GTK_WIDGET(self->button_contents));
    gtk_widget_add_css_class(GTK_WIDGET(self->button),
                             "notification-widget-button");

    gtk_box_append(self->button_container, GTK_WIDGET(self->button));

    self->avatar = ADW_AVATAR(adw_avatar_new(48, "", false));
    gtk_widget_add_css_class(GTK_WIDGET(self->avatar),
                             "notification-widget-icon");
    adw_avatar_set_icon_name(self->avatar,
                             "preferences-system-notifications-symbolic");
    gtk_widget_set_valign(GTK_WIDGET(self->avatar), GTK_ALIGN_START);

    self->text = GTK_BOX(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
    gtk_widget_set_valign(GTK_WIDGET(self->text), GTK_ALIGN_CENTER);

    // setup summary and body labels
    self->summary = GTK_LABEL(gtk_label_new(""));
    gtk_widget_add_css_class(GTK_WIDGET(self->summary), "summary");
    gtk_widget_set_halign(GTK_WIDGET(self->summary), GTK_ALIGN_START);
    gtk_label_set_max_width_chars(self->summary, 40);
    gtk_label_set_ellipsize(self->summary, PANGO_ELLIPSIZE_END);
    gtk_label_set_lines(self->summary, 1);
    gtk_label_set_wrap(self->summary, true);
    gtk_widget_set_size_request(GTK_WIDGET(self->summary), 380, -1);
    gtk_label_set_xalign(self->summary, 0.0);

    self->body = GTK_LABEL(gtk_label_new(""));
    gtk_widget_add_css_class(GTK_WIDGET(self->body), "body");
    gtk_widget_set_halign(GTK_WIDGET(self->body), GTK_ALIGN_START);
    gtk_label_set_max_width_chars(self->body, 200);
    gtk_label_set_ellipsize(self->body, PANGO_ELLIPSIZE_END);
    gtk_label_set_lines(self->body, 1);
    gtk_label_set_wrap(self->body, true);
    gtk_widget_set_size_request(GTK_WIDGET(self->body), 380, -1);
    gtk_label_set_xalign(self->body, 0.0);

    gtk_box_append(self->text, GTK_WIDGET(self->summary));
    gtk_box_append(self->text, GTK_WIDGET(self->body));

    gtk_box_append(self->button_contents, GTK_WIDGET(self->avatar));
    gtk_box_append(self->button_contents, GTK_WIDGET(self->text));

    // set body expand animation
    AdwAnimationTarget *target = adw_callback_animation_target_new(
        (AdwAnimationTargetFunc)expand_animation_cb, self->body, NULL);
    self->expand_animation =
        adw_timed_animation_new(GTK_WIDGET(self->body), 1, 10, 200, target);
    adw_timed_animation_set_easing(ADW_TIMED_ANIMATION(self->expand_animation),
                                   ADW_LINEAR);

    g_signal_connect_object(self->expand_animation, "done",
                            G_CALLBACK(on_expand_animation_done), self, 0);

    gtk_box_append(self->notification_container, GTK_WIDGET(self->header));
    gtk_box_append(self->notification_container,
                   GTK_WIDGET(self->button_container));

    gtk_box_append(self->container, GTK_WIDGET(self->notification_container));

    // we can make a 'stacked notifications' effect by being clever with some
    // extra boxes in the notification widget.
    self->stack_effect_box1 = GTK_BOX(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->stack_effect_box1),
                             "notification-group-stack-effect-box1");
    GtkLabel *label1 = GTK_LABEL(gtk_label_new(""));
    // set width request to 1, allows us to set margins on surrounding box
    gtk_widget_set_size_request(GTK_WIDGET(label1), 1, -1);
    gtk_box_append(self->stack_effect_box1, GTK_WIDGET(label1));
    gtk_widget_set_margin_start(GTK_WIDGET(self->stack_effect_box1), 5);
    gtk_widget_set_margin_end(GTK_WIDGET(self->stack_effect_box1), 5);

    self->stack_effect_box2 = GTK_BOX(gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->stack_effect_box2),
                             "notification-group-stack-effect-box2");
    GtkLabel *label2 = GTK_LABEL(gtk_label_new(""));
    gtk_widget_set_size_request(GTK_WIDGET(label2), 1, -1);
    gtk_box_append(self->stack_effect_box2, GTK_WIDGET(label2));
    gtk_widget_set_margin_start(GTK_WIDGET(self->stack_effect_box2), 10);
    gtk_widget_set_margin_end(GTK_WIDGET(self->stack_effect_box2), 10);

    gtk_box_append(self->container, GTK_WIDGET(self->stack_effect_box1));
    gtk_box_append(self->container, GTK_WIDGET(self->stack_effect_box2));

    // start stack effect boxes as hidden
    gtk_widget_set_visible(GTK_WIDGET(self->stack_effect_box1), false);
    gtk_widget_set_visible(GTK_WIDGET(self->stack_effect_box2), false);
}

static void notification_widget_init(NotificationWidget *self) {
    self->id = 0;
    self->expanded = false;
    self->expand_animation = NULL;
}

static gboolean avatar_from_img_data(NotificationWidget *self,
                                     NotificationImageData *image) {
    guint channels = image->has_alpha ? 4 : 3;
    if (!image->data || !image->width || !image->height ||
        image->width > G_MAXINT || image->height > G_MAXINT ||
        image->bits_per_sample != 8 || image->channels != channels)
        return FALSE;
    gsize row_bytes = (gsize)image->width * channels;
    if (image->rowstride < row_bytes ||
        (gsize)(image->height - 1) > (G_MAXSIZE - row_bytes) / image->rowstride)
        return FALSE;
    gsize length = (gsize)(image->height - 1) * image->rowstride + row_bytes;
    // The service validates the pixel buffer's length. Copy its borrowed data
    // before replacement/removal releases the notification snapshot.
    g_autoptr(GBytes) pixels = g_bytes_new(image->data, length);
    g_autoptr(GdkTexture) texture = gdk_memory_texture_new(
        image->width, image->height,
        image->has_alpha ? GDK_MEMORY_R8G8B8A8 : GDK_MEMORY_R8G8B8,
        pixels, image->rowstride);
    adw_avatar_set_custom_image(self->avatar, GDK_PAINTABLE(texture));
    return TRUE;
}

static GIcon *app_icon_for_id(const gchar *app_id) {
    if (!app_id || !*app_id) return NULL;
    g_autofree gchar *lower_app_id = g_utf8_strdown(app_id, -1);
    GList *apps = g_app_info_get_all();
    GIcon *result = NULL;
    for (GList *item = apps; item; item = item->next) {
        GAppInfo *info = item->data;
        const gchar *id = g_app_info_get_id(info);
        if (!id) continue;
        g_autofree gchar *lower_id = g_utf8_strdown(id, -1);
        if (g_strrstr(lower_id, lower_app_id)) {
            GIcon *icon = g_app_info_get_icon(info);
            if (icon) result = g_object_ref(icon);
            break;
        }
    }
    g_list_free_full(apps, g_object_unref);
    return result;
}

static void icon_from_app_id(GtkImage *image, gchar *app_id) {
    g_autoptr(GIcon) icon = app_icon_for_id(app_id);
    if (icon) gtk_image_set_from_gicon(image, icon);
}

static void avatar_from_app_id(NotificationWidget *self, gchar *app_id) {
    g_autoptr(GIcon) icon = app_icon_for_id(app_id);
    if (!icon) return;
    GtkIconTheme *theme = gtk_icon_theme_get_for_display(gdk_display_get_default());
    g_autoptr(GtkIconPaintable) paintable = gtk_icon_theme_lookup_by_gicon(
        theme, icon, 48, 1, GTK_TEXT_DIR_RTL, 0);
    adw_avatar_set_custom_image(self->avatar, GDK_PAINTABLE(paintable));
}

static gboolean icon_is_file(const gchar *name) {
    return name && (g_path_is_absolute(name) || strstr(name, "://"));
}

static void set_notification_icon(NotificationWidget *self, Notification *n) {
    cancel_media_artwork(self);
    adw_avatar_set_custom_image(self->avatar, NULL);
    adw_avatar_set_icon_name(self->avatar, "preferences-system-notifications-symbolic");
    if (n->img_data.data && avatar_from_img_data(self, &n->img_data)) return;
    if (n->image_path && *n->image_path) {
        load_artwork(self, n->image_path);
    } else if (n->app_icon && *n->app_icon) {
        if (icon_is_file(n->app_icon))
            load_artwork(self, n->app_icon);
        else
            adw_avatar_set_icon_name(self->avatar, n->app_icon);
    } else if (n->app_name && *n->app_name) {
        avatar_from_app_id(self, n->app_name);
    } else if (n->desktop_entry && *n->desktop_entry) {
        avatar_from_app_id(self, n->desktop_entry);
    }
}

static void set_text_with_markup(GtkLabel *label, const gchar *text) {
    g_autoptr(GError) error = NULL;
    if (pango_parse_markup(text, -1, 0, NULL, NULL, NULL, &error))
        gtk_label_set_markup(label, text);
    else
        gtk_label_set_text(label, text);
}

static void set_notification_text(NotificationWidget *self, Notification *n) {
    g_autofree gchar *summary = g_strdup(n->summary ? n->summary : "");
    g_autofree gchar *body = g_strdup(n->body ? n->body : "");
    gtk_label_set_text(self->summary, g_strdelimit(g_strstrip(summary), "\n", ' '));
    set_text_with_markup(self->body, g_strdelimit(g_strstrip(body), "\n", ' '));
}

static void set_notification_app_icon(NotificationWidget *self, Notification *n) {
    gtk_image_set_from_icon_name(self->header_app_icon,
                                 "preferences-system-notifications-symbolic");
    if (n->app_icon && *n->app_icon) {
        if (icon_is_file(n->app_icon)) {
            g_autoptr(GFile) file = g_file_new_for_commandline_arg(n->app_icon);
            g_autoptr(GIcon) icon = g_file_icon_new(file);
            gtk_image_set_from_gicon(self->header_app_icon, icon);
        } else {
            gtk_image_set_from_icon_name(self->header_app_icon, n->app_icon);
        }
    } else if (n->app_name && *n->app_name) {
        icon_from_app_id(self->header_app_icon, n->app_name);
    } else if (n->desktop_entry && *n->desktop_entry) {
        icon_from_app_id(self->header_app_icon, n->desktop_entry);
    }
}

static void on_expand_button_clicked(GtkButton *button,
                                     NotificationWidget *self) {
    g_debug("notification_widget.c:on_expand_button_clicked() called");

    if (self->expanded) {
        gtk_button_set_icon_name(self->header_expand, "go-down-symbolic");
        adw_timed_animation_set_reverse(
            ADW_TIMED_ANIMATION(self->expand_animation), true);
        adw_animation_play(self->expand_animation);

        if (self->action_revealer)
            gtk_revealer_set_reveal_child(self->action_revealer, false);
    } else {
        gtk_button_set_icon_name(self->header_expand, "go-up-symbolic");
        adw_timed_animation_set_reverse(
            ADW_TIMED_ANIMATION(self->expand_animation), false);
        adw_animation_play(self->expand_animation);

        if (self->action_revealer)
            gtk_revealer_set_reveal_child(self->action_revealer, true);
    }

    self->expanded = !self->expanded;
}

static gboolean update_timer(NotificationWidget *self) {
    if (self->disposed || !self->created_on) return G_SOURCE_REMOVE;
    g_autoptr(GDateTime) now = g_date_time_new_now_local();
    GTimeSpan span = g_date_time_difference(now, self->created_on);
    gint days = span / G_TIME_SPAN_DAY;
    gint hours = (span % G_TIME_SPAN_DAY) / G_TIME_SPAN_HOUR;
    gint minutes = (span % G_TIME_SPAN_HOUR) / G_TIME_SPAN_MINUTE;
    g_autofree gchar *age = NULL;
    if (days > 0)
        age = g_strdup_printf(days == 1 ? "%d day ago" : "%d days ago", days);
    else if (hours > 0)
        age = g_strdup_printf(hours == 1 ? "%d hour ago" : "%d hours ago", hours);
    else if (minutes > 0)
        age = g_strdup_printf(minutes == 1 ? "%d minute ago" : "%d minutes ago", minutes);
    gtk_label_set_text(self->header_timer, age ? age : "Just now");
    return G_SOURCE_CONTINUE;
}

static void action_button_css_reset(GtkWidget *child) {
    gtk_widget_remove_css_class(child, "first");
    gtk_widget_remove_css_class(child, "center");
    gtk_widget_remove_css_class(child, "last");
    gtk_widget_remove_css_class(child, "only");
}

static void set_action_button_css(NotificationWidget *self) {
    if (!self->actions_buttons_n) return;

    GtkWidget *child =
        gtk_widget_get_first_child(GTK_WIDGET(self->action_container));

    if (self->actions_buttons_n == 1) {
        // add .only class
        action_button_css_reset(child);
        gtk_widget_add_css_class(child, "only");
        return;
    }

    int i = 1;

    while (child) {
        action_button_css_reset(child);
        if (i == 1) {
            gtk_widget_add_css_class(child, "first");
        } else if (i != self->actions_buttons_n) {
            gtk_widget_add_css_class(child, "center");
        } else {
            gtk_widget_add_css_class(child, "last");
        }
        i++;
        child = gtk_widget_get_next_sibling(child);
    }
}

void notification_widget_set_notification(NotificationWidget *self, Notification *n) {
    if (self->disposed || !n) return;
    self->id = n->id;
    g_object_set_data(G_OBJECT(self->container), "notification-id", GUINT_TO_POINTER(n->id));

    if (self->action_container) {
        GtkWidget *child = gtk_widget_get_first_child(GTK_WIDGET(self->action_container));
        while (child) {
            GtkWidget *next = gtk_widget_get_next_sibling(child);
            if (child != GTK_WIDGET(self->osd_hide_button)) {
                g_signal_handlers_disconnect_by_data(child, self);
                gtk_box_remove(self->action_container, child);
            }
            child = next;
        }
    }
    self->actions_buttons_n = self->osd_hide_button ? 1 : 0;
    notification_widget_from_notification_action_buttons(n, self);
    if (self->osd_hide_button) {
        GtkWidget *last = gtk_widget_get_last_child(GTK_WIDGET(self->action_container));
        if (last != GTK_WIDGET(self->osd_hide_button))
            gtk_box_reorder_child_after(self->action_container, GTK_WIDGET(self->osd_hide_button), last);
    }
    set_action_button_css(self);
    if (self->action_revealer) {
        gtk_widget_set_visible(GTK_WIDGET(self->action_revealer), self->actions_buttons_n > 0);
        gtk_revealer_set_reveal_child(self->action_revealer, self->expanded);
    }

    if (n->urgency == 2)
        gtk_widget_add_css_class(GTK_WIDGET(self->button), "notification-widget-button-critical");
    else
        gtk_widget_remove_css_class(GTK_WIDGET(self->button), "notification-widget-button-critical");
    gtk_label_set_text(self->header_app_name, n->app_name ? n->app_name : "");
    set_notification_app_icon(self, n);
    set_notification_icon(self, n);
    set_notification_text(self, n);
    GDateTime *created = n->created_on ? g_date_time_ref(n->created_on) : g_date_time_new_now_local();
    g_clear_pointer(&self->created_on, g_date_time_unref);
    self->created_on = created;
    update_timer(self);
}

NotificationWidget *notification_widget_from_notification(
    Notification *n, gboolean expand_on_enter) {
    NotificationWidget *self = g_object_new(NOTIFICATION_WIDGET_TYPE, NULL);
    notification_widget_init_layout(self);
    notification_widget_set_notification(self, n);

    // wire up notification click
    g_signal_connect_object(self->button, "clicked",
                            G_CALLBACK(on_notification_clicked), self, 0);

    // wire up dissmiss click
    g_signal_connect_object(self->header_dismiss, "clicked",
                            G_CALLBACK(on_dismiss_clicked), self, 0);

    // wire up motion controller
    if (expand_on_enter) {
        g_signal_connect_object(self->ctrl, "enter", G_CALLBACK(on_pointer_enter),
                                self, 0);
        g_signal_connect_object(self->ctrl, "leave", G_CALLBACK(on_pointer_leave),
                                self, 0);
    } else {
        // set expander button visible as visible if we aren't expanding on
        // cursor enter
        gtk_widget_set_visible(GTK_WIDGET(self->header_expand), true);
    }

    if (!expand_on_enter) {
        MessageTray *mt = message_tray_get_global();
        if (mt)
            g_signal_connect_object(mt, "message-tray-will-hide",
                                    G_CALLBACK(on_message_tray_will_hide), self, 0);
    }

    // give the main container a pointer to ourselves.
    g_object_set_data(G_OBJECT(self->container), "self", self);

    // setup timer for ever minute to update header_timer
    self->timer_id = g_timeout_add_seconds(60, (GSourceFunc)update_timer, self);

    return self;
}

GtkWidget *notification_widget_get_widget(NotificationWidget *self) {
    return GTK_WIDGET(self->container);
}

guint32 notification_widget_get_id(NotificationWidget *self) {
    return self->id;
}

GtkLabel *notification_widget_get_summary(NotificationWidget *self) {
    return self->summary;
}

GtkLabel *notification_widget_get_body(NotificationWidget *self) {
    return self->body;
}

void notification_widget_set_stack_effect(NotificationWidget *self,
                                          gboolean show) {
    gtk_widget_set_visible(GTK_WIDGET(self->header_expand), !show);
    gtk_widget_set_visible(GTK_WIDGET(self->stack_effect_box1), show);
    gtk_widget_set_visible(GTK_WIDGET(self->stack_effect_box2), show);
}

void notification_widget_dismiss_notification(NotificationWidget *self) {
    NotificationsService *service = notifications_service_get_global();
    notifications_service_closed_notification(
        service, self->id, NOTIFICATIONS_CLOSED_REASON_DISMISSED);
}

// Media Player Notification Widget

static void media_player_widget_on_playpause_clicked(GtkButton *button,
                                                     gpointer user_data) {
    NotificationWidget *self = (NotificationWidget *)user_data;
    MediaPlayerService *srv = media_player_service_get_global();
    media_player_service_player_playpause(srv, self->media_player_name);
}

static void media_player_widget_on_previous_clicked(GtkButton *button,
                                                    gpointer user_data) {
    NotificationWidget *self = (NotificationWidget *)user_data;
    MediaPlayerService *srv = media_player_service_get_global();
    media_player_service_player_previous(srv, self->media_player_name);
}

static void media_player_widget_on_next_clicked(GtkButton *button,
                                                gpointer user_data) {
    NotificationWidget *self = (NotificationWidget *)user_data;
    MediaPlayerService *srv = media_player_service_get_global();
    media_player_service_player_next(srv, self->media_player_name);
}

static void media_player_widget_on_raise(GtkButton *button,
                                         gpointer user_data) {
    NotificationWidget *self = (NotificationWidget *)user_data;
    MediaPlayerService *srv = media_player_service_get_global();
    media_player_service_player_raise(srv, self->media_player_name);
}

typedef struct {
    GWeakRef owner;
    GCancellable *cancellable;
    guint64 generation;
} MediaArtworkRequest;

static void media_artwork_request_free(MediaArtworkRequest *request) {
    g_weak_ref_clear(&request->owner);
    g_clear_object(&request->cancellable);
    g_free(request);
}

static NotificationWidget *media_artwork_request_owner(MediaArtworkRequest *request) {
    NotificationWidget *self = g_weak_ref_get(&request->owner);
    if (self && (self->disposed || self->artwork_generation != request->generation ||
                 self->artwork_cancellable != request->cancellable))
        g_clear_object(&self);
    return self;
}

static void media_artwork_error(GError *error) {
    if (error && !g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        g_message("Could not load artwork: %s", error->message);
}

static void on_media_player_img_decoded(GObject *obj, GAsyncResult *res,
                                        gpointer user_data) {
    MediaArtworkRequest *request = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GdkPixbuf) pixbuf = gdk_pixbuf_new_from_stream_finish(res, &error);
    g_autoptr(NotificationWidget) self = media_artwork_request_owner(request);
    if (self) {
        g_clear_object(&self->artwork_cancellable);
        if (pixbuf) {
            g_autoptr(GBytes) pixels = gdk_pixbuf_read_pixel_bytes(pixbuf);
            GdkMemoryFormat format = gdk_pixbuf_get_has_alpha(pixbuf)
                ? GDK_MEMORY_R8G8B8A8 : GDK_MEMORY_R8G8B8;
            g_autoptr(GdkTexture) texture = gdk_memory_texture_new(
                gdk_pixbuf_get_width(pixbuf), gdk_pixbuf_get_height(pixbuf),
                format, pixels, gdk_pixbuf_get_rowstride(pixbuf));
            adw_avatar_set_custom_image(self->avatar, GDK_PAINTABLE(texture));
        } else {
            media_artwork_error(error);
        }
    }
    media_artwork_request_free(request);
}

static void on_media_player_img_loaded(GObject *obj, GAsyncResult *res,
                                       gpointer user_data) {
    MediaArtworkRequest *request = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GFileInputStream) stream = g_file_read_finish(G_FILE(obj), res, &error);
    g_autoptr(NotificationWidget) self = media_artwork_request_owner(request);
    if (!self || !stream) {
        if (self) {
            g_clear_object(&self->artwork_cancellable);
            media_artwork_error(error);
        }
        media_artwork_request_free(request);
        return;
    }

    // Decode off the GTK thread and retain only the avatar-sized image.
    // The decoder owns the stream until its asynchronous result is finished.
    gdk_pixbuf_new_from_stream_at_scale_async(G_INPUT_STREAM(stream), 48, 48, TRUE,
                                             request->cancellable,
                                             on_media_player_img_decoded, request);
}

static void load_artwork(NotificationWidget *self, const gchar *location) {
    MediaArtworkRequest *request = g_new0(MediaArtworkRequest, 1);
    g_weak_ref_init(&request->owner, self);
    self->artwork_cancellable = g_cancellable_new();
    request->cancellable = g_object_ref(self->artwork_cancellable);
    request->generation = self->artwork_generation;
    g_autoptr(GFile) file = g_file_new_for_commandline_arg(location);
    g_file_read_async(file, G_PRIORITY_DEFAULT, request->cancellable,
                      on_media_player_img_loaded, request);
}

NotificationWidget *notification_widget_set_media_player(
    NotificationWidget *self, MediaPlayer *player) {
    if (self->disposed) return self;
    cancel_media_artwork(self);
    adw_avatar_set_custom_image(self->avatar, NULL);
    if (player->art_url && *player->art_url)
        load_artwork(self, player->art_url);

    // update play/pause icon depending on playback state
    if (g_strcmp0(player->playback_status, "Playing") == 0) {
        gtk_button_set_icon_name(self->play_pause,
                                 "media-playback-pause-symbolic");
    } else {
        gtk_button_set_icon_name(self->play_pause,
                                 "media-playback-start-symbolic");
    }

    gtk_label_set_text(self->header_app_name, player->identity);
    gtk_label_set_text(self->summary, player->artist);
    gtk_label_set_text(self->body, player->title);
    return self;
}

NotificationWidget *notification_widget_from_media_player(MediaPlayer *player) {
    NotificationWidget *self = g_object_new(NOTIFICATION_WIDGET_TYPE, NULL);

    self->media_player_name = g_strdup(player->name);

    // use almost all of the normal notification's layout, we'll tweak it below.
    notification_widget_init_layout(self);

    // hide dismiss button
    gtk_widget_set_visible(GTK_WIDGET(self->header_dismiss), false);
    // hide header's timer
    gtk_widget_set_visible(GTK_WIDGET(self->header_timer), false);

    // add media-player class to container to css can apply select styling
    gtk_widget_add_css_class(GTK_WIDGET(self->container), "media-player");

    // if we have an identity of the player we can display it in the
    // notification header.
    if (player->identity)
        icon_from_app_id(self->header_app_icon, player->identity);

    // reset some label layout details since media player buttons will be
    // present next to the labels.
    gtk_widget_set_size_request(GTK_WIDGET(self->summary), -1, -1);
    gtk_widget_set_size_request(GTK_WIDGET(self->body), -1, -1);

    gtk_label_set_width_chars(self->summary, 34);
    gtk_label_set_max_width_chars(self->summary, 34);

    gtk_label_set_width_chars(self->body, 34);
    gtk_label_set_max_width_chars(self->body, 34);

    // create the player's media buttons.
    self->media_buttons = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->media_buttons),
                             "notification-widget-media-buttons-container");
    gtk_widget_set_halign(GTK_WIDGET(self->media_buttons), GTK_ALIGN_END);

    self->play_pause = GTK_BUTTON(
        gtk_button_new_from_icon_name("media-playback-pause-symbolic"));
    gtk_widget_add_css_class(GTK_WIDGET(self->play_pause),
                             "notification-widget-media-button");

    self->previous =
        GTK_BUTTON(gtk_button_new_from_icon_name("go-previous-symbolic"));
    gtk_widget_add_css_class(GTK_WIDGET(self->previous),
                             "notification-widget-media-button");

    self->next = GTK_BUTTON(gtk_button_new_from_icon_name("go-next-symbolic"));
    gtk_widget_add_css_class(GTK_WIDGET(self->next),
                             "notification-widget-media-button");

    // wire them up
    g_signal_connect_object(self->button, "clicked",
                            G_CALLBACK(media_player_widget_on_raise), self, 0);
    g_signal_connect_object(self->play_pause, "clicked",
                            G_CALLBACK(media_player_widget_on_playpause_clicked),
                            self, 0);
    g_signal_connect_object(self->previous, "clicked",
                            G_CALLBACK(media_player_widget_on_previous_clicked), self, 0);
    g_signal_connect_object(self->next, "clicked",
                            G_CALLBACK(media_player_widget_on_next_clicked), self, 0);

    gtk_box_append(self->media_buttons, GTK_WIDGET(self->previous));
    gtk_box_append(self->media_buttons, GTK_WIDGET(self->play_pause));
    gtk_box_append(self->media_buttons, GTK_WIDGET(self->next));

    // append media player buttons next to notification button
    gtk_box_append(self->button_container, GTK_WIDGET(self->media_buttons));

    // give the main container a pointer to ourselves.
    g_object_set_data(G_OBJECT(self->container), "self", self);

    return self;
}

gchar *notification_widget_get_media_player_name(NotificationWidget *self) {
    return self->media_player_name;
}

static void on_hide_button_clicked(GtkButton *button,
                                   NotificationWidget *self) {
    g_debug("notification_widget.c:on_hide_button_clicked() called");
    if (self->osd) notification_osd_hide(self->osd);
}

void notification_widget_set_osd(NotificationWidget *self,
                                 NotificationsOSD *osd) {
    if (self->disposed) return;
    if (self->osd)
        g_object_remove_weak_pointer(G_OBJECT(self->osd), (gpointer *)&self->osd);
    self->osd = osd;
    if (self->osd)
        g_object_add_weak_pointer(G_OBJECT(self->osd), (gpointer *)&self->osd);
    if (self->osd_hide_button) return;
    // for the creation of a synethic 'hide' action button if this notification
    // has an OSD attached, which hides the notification for later viewing in
    // the NotificationList
    if (!self->action_revealer) {
        configure_action_revealer(self);
    }

    // Add action button
    GtkButton *action_button = GTK_BUTTON(gtk_button_new_with_label("Hide"));
    self->osd_hide_button = action_button;

    // hexpand button
    gtk_widget_set_hexpand(GTK_WIDGET(action_button), true);

    gtk_widget_add_css_class(GTK_WIDGET(action_button),
                             "notification-widget-action-button");

    g_signal_connect_object(action_button, "clicked",
                            G_CALLBACK(on_hide_button_clicked), self, 0);

    gtk_box_append(self->action_container, GTK_WIDGET(action_button));
    self->actions_buttons_n++;

    // we added a button, so reset css
    set_action_button_css(self);

    gtk_widget_set_visible(GTK_WIDGET(self->action_revealer), TRUE);
    gtk_revealer_set_reveal_child(self->action_revealer, self->expanded);
}

NotificationsOSD *notification_widget_get_osd(NotificationWidget *self) {
    return self->osd;
}
