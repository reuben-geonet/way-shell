#include "indicator_widget.h"

#include <adwaita.h>

#include "../../services/status_notifier_service/status_notifier_service.h"

struct _IndicatorWidget {
    GObject parent_instance;
    GtkBox *container;
    GtkBox *box;
    GtkImage *icon;
    GtkButton *button;
    GtkPopoverMenu *menu;
    StatusNotifierItem *sni;
    GWeakRef service;
    gboolean disposed;
};
G_DEFINE_TYPE(IndicatorWidget, indicator_widget, G_TYPE_OBJECT);

static void on_sni_menu_updated(StatusNotifierService *s,
                                StatusNotifierItem *sni, IndicatorWidget *self);
static void on_sni_property_update(StatusNotifierService *s,
                                   StatusNotifierItem *item,
                                   IndicatorWidget *self);
static void clear_menu(IndicatorWidget *self) {
    GtkPopoverMenu *menu = self->menu;
    self->menu = NULL;
    if (menu) {
        gtk_widget_insert_action_group(GTK_WIDGET(menu), SNI_GACTION_PREFIX, NULL);
        gtk_popover_popdown(GTK_POPOVER(menu));
        gtk_widget_unparent(GTK_WIDGET(menu));
    }
}

static void disconnect_service(IndicatorWidget *self) {
    g_autoptr(GObject) source = g_weak_ref_get(&self->service);
    if (source) g_signal_handlers_disconnect_by_data(source, self);
    g_weak_ref_set(&self->service, NULL);
}

static void indicator_widget_dispose(GObject *gobject) {
    IndicatorWidget *self = INDICATOR_WIDGET(gobject);
    self->disposed = TRUE;
    disconnect_service(self);
    if (self->button) g_signal_handlers_disconnect_by_data(self->button, self);
    clear_menu(self);
    self->sni = NULL;
    g_clear_object(&self->container);
    self->button = NULL;
    self->icon = NULL;
    G_OBJECT_CLASS(indicator_widget_parent_class)->dispose(gobject);
};

static void indicator_widget_finalize(GObject *gobject) {
    IndicatorWidget *self = INDICATOR_WIDGET(gobject);
    g_weak_ref_clear(&self->service);
    G_OBJECT_CLASS(indicator_widget_parent_class)->finalize(gobject);
};

static void indicator_widget_class_init(IndicatorWidgetClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = indicator_widget_dispose;
    object_class->finalize = indicator_widget_finalize;
};

static void on_button_clicked(GtkButton *button, IndicatorWidget *self) {
    if (self->disposed || !self->sni) return;
    if (self->menu) {
        gtk_popover_popup(GTK_POPOVER(self->menu));
        status_notifier_item_about_to_show(self->sni, 0);
        return;
    }
    status_notifier_item_activate(self->sni, 0, 0);
}

static void indicator_widget_init_layout(IndicatorWidget *self) {
    self->container = g_object_ref_sink(GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0)));
    gtk_widget_set_name(GTK_WIDGET(self->container),
                        "panel-indicator-bar-widget");

    self->box = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->box),
                             "panel-indicator-bar-widget-box");

    self->icon = GTK_IMAGE(gtk_image_new_from_icon_name("image-missing"));

    self->button = GTK_BUTTON(gtk_button_new());
    gtk_button_set_child(self->button, GTK_WIDGET(self->icon));
    g_signal_connect_object(self->button, "clicked", G_CALLBACK(on_button_clicked), self, 0);

    // wire it up
    gtk_box_append(self->box, GTK_WIDGET(self->button));
    gtk_box_append(self->container, GTK_WIDGET(self->box));
}

static void indicator_widget_init(IndicatorWidget *self) {
    g_weak_ref_init(&self->service, NULL);
    indicator_widget_init_layout(self);
}

GtkWidget *indicator_widget_get_widget(IndicatorWidget *self) {
    return GTK_WIDGET(self->container);
}

static void indicator_widget_set_icon(IndicatorWidget *self) {
    const gchar *icon_name = status_notifier_item_get_icon_name(self->sni);
    if (self->sni->icon_pixmap_from_theme) {
        g_autoptr(GdkTexture) t =
            gdk_texture_new_for_pixbuf(self->sni->icon_pixmap_from_theme);
        gtk_image_set_from_paintable(self->icon, GDK_PAINTABLE(t));
    } else if (icon_name && strlen(icon_name) > 0) {
        gtk_image_set_from_icon_name(self->icon, icon_name);
    } else {
        GdkPixbuf *pix = status_notifier_item_get_icon_pixmap(self->sni);
        if (pix) {
            g_autoptr(GdkTexture) t = gdk_texture_new_for_pixbuf(pix);
            gtk_image_set_from_paintable(self->icon, GDK_PAINTABLE(t));
        } else {
            gtk_image_set_from_icon_name(self->icon, "image-missing");
        }
    }
}

static void on_sni_property_update(StatusNotifierService *s,
                                   StatusNotifierItem *item,
                                   IndicatorWidget *self) {
    if (self->disposed || item != self->sni) return;
    indicator_widget_set_icon(self);
}

static void on_sni_menu_updated(StatusNotifierService *s,
                                StatusNotifierItem *sni,
                                IndicatorWidget *self) {
    g_debug("indicator_widget.c:on_sni_menu_updated() called");
    if (self->disposed || sni != self->sni) return;
    if (!sni->menu_model) {
        clear_menu(self);
        return;
    }
    if (!self->menu) {
        self->menu = GTK_POPOVER_MENU(gtk_popover_menu_new_from_model_full(
            G_MENU_MODEL(sni->menu_model), GTK_POPOVER_MENU_NESTED));
        gtk_widget_set_parent(GTK_WIDGET(self->menu), GTK_WIDGET(self->button));
    }
    gtk_widget_insert_action_group(GTK_WIDGET(self->menu), SNI_GACTION_PREFIX,
                                   sni->action_group);
    gtk_popover_menu_set_menu_model(self->menu, G_MENU_MODEL(sni->menu_model));
}

void indicator_widget_set_sni(IndicatorWidget *self, StatusNotifierItem *sni) {
    if (self->disposed || !sni) return;
    disconnect_service(self);
    self->sni = sni;

    indicator_widget_set_icon(self);

    StatusNotifierService *s = status_notifier_service_get_global();
    on_sni_menu_updated(s, sni, self);
    g_weak_ref_set(&self->service, s);
    if (s) {
        g_signal_connect_object(s, "status-notifier-item-properties-changed",
                                G_CALLBACK(on_sni_property_update), self, 0);
        g_signal_connect_object(s, "status-notifier-item-menu-updated",
                                G_CALLBACK(on_sni_menu_updated), self, 0);
    }
}

StatusNotifierItem *indicator_widget_get_sni(IndicatorWidget *self) {
    return self->sni;
}
