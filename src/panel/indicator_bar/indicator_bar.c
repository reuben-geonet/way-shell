#include "indicator_bar.h"

#include <adwaita.h>

#include "../../services/status_notifier_service/status_notifier_service.h"
#include "../panel.h"
#include "indicator_widget.h"

struct _IndicatorBar {
    GObject parent_instance;
    GtkBox *container;
    GtkBox *list;
    Panel *panel;
    GHashTable *indicators;
    GWeakRef service;
};
G_DEFINE_TYPE(IndicatorBar, indicator_bar, G_TYPE_OBJECT);

static void on_status_notifier_item_added(StatusNotifierService *sn,
                                          GHashTable *items,
                                          StatusNotifierItem *sni,
                                          IndicatorBar *self);

static void on_status_notifier_item_removed(StatusNotifierService *sn,
                                            GHashTable *items,
                                            StatusNotifierItem *sni,
                                            IndicatorBar *self);

// stub out dispose, finalize, class_init, and init methods
static void indicator_bar_dispose(GObject *gobject) {
    IndicatorBar *self = INDICATOR_BAR(gobject);

    g_autoptr(GObject) source = g_weak_ref_get(&self->service);
    if (source) g_signal_handlers_disconnect_by_data(source, self);
    g_weak_ref_set(&self->service, NULL);
    if (self->list) {
        GtkWidget *child;
        while ((child = gtk_widget_get_first_child(GTK_WIDGET(self->list))))
            gtk_box_remove(self->list, child);
    }
    g_clear_pointer(&self->indicators, g_hash_table_unref);
    g_clear_object(&self->container);
    self->list = NULL;
    G_OBJECT_CLASS(indicator_bar_parent_class)->dispose(gobject);
};

static void indicator_bar_finalize(GObject *gobject) {
    IndicatorBar *self = INDICATOR_BAR(gobject);
    g_weak_ref_clear(&self->service);
    G_OBJECT_CLASS(indicator_bar_parent_class)->finalize(gobject);
};

static void indicator_bar_class_init(IndicatorBarClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = indicator_bar_dispose;
    object_class->finalize = indicator_bar_finalize;
};

static void on_status_notifier_item_added(StatusNotifierService *sn,
                                          GHashTable *items,
                                          StatusNotifierItem *sni,
                                          IndicatorBar *self) {
    g_debug("indicator_bar.c:on_status_notifier_item_added");

    if (!self->indicators || !sni) return;
    g_autofree gchar *key = g_strconcat(sni->bus_name, sni->obj_name, NULL);
    IndicatorWidget *w = g_hash_table_lookup(self->indicators, key);
    if (w) {
        indicator_widget_set_sni(w, sni);
        return;
    }
    w = g_object_new(INDICATOR_WIDGET_TYPE, NULL);
    indicator_widget_set_sni(w, sni);

    g_hash_table_insert(self->indicators, g_steal_pointer(&key), w);
    gtk_box_append(self->list, indicator_widget_get_widget(w));
}

static void on_status_notifier_item_removed(StatusNotifierService *sn,
                                            GHashTable *items,
                                            StatusNotifierItem *sni,
                                            IndicatorBar *self) {
    g_debug("indicator_bar.c:on_status_notifier_item_removed");

    if (!self->indicators || !sni) return;
    g_autofree gchar *key = g_strconcat(sni->bus_name, sni->obj_name, NULL);
    IndicatorWidget *i = g_hash_table_lookup(self->indicators, key);
    if (!i || indicator_widget_get_sni(i) != sni) return;

    GtkWidget *w = indicator_widget_get_widget(i);
    gtk_box_remove(self->list, w);

    g_hash_table_remove(self->indicators, key);
}

static void indicator_bar_init_layout(IndicatorBar *self) {
    g_debug("indicator_bar.c:indicator_bar_init_layout() called.");
    self->container = g_object_ref_sink(GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0)));
    gtk_widget_set_name(GTK_WIDGET(self->container), "panel-indicator-bar");

    self->list = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->list),
                             "panel-indicator-bar-list");

    gtk_box_append(self->container, GTK_WIDGET(self->list));

    StatusNotifierService *sn = status_notifier_service_get_global();
    if (!sn) return;
    g_weak_ref_set(&self->service, sn);
    g_signal_connect_object(sn, "status-notifier-item-added",
                            G_CALLBACK(on_status_notifier_item_added), self, 0);
    g_signal_connect_object(sn, "status-notifier-item-removed",
                            G_CALLBACK(on_status_notifier_item_removed), self, 0);

    // seed indicators
    GHashTable *items = status_notifier_service_get_items(sn);
    GList *values = g_hash_table_get_values(items);
    for (GList *l = values; l; l = l->next) {
        StatusNotifierItem *sni = l->data;
        on_status_notifier_item_added(sn, items, sni, self);
    }
    g_list_free(values);
}

static void indicator_bar_init(IndicatorBar *self) {
    g_debug("indicator_bar.c:indicator_bar_init() called.");
    g_weak_ref_init(&self->service, NULL);
    self->indicators = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_object_unref);
    indicator_bar_init_layout(self);
}

GtkWidget *indicator_bar_get_widget(IndicatorBar *self) {
    return GTK_WIDGET(self->container);
}

void indicator_bar_set_panel(IndicatorBar *self, Panel *panel) {
    self->panel = panel;
}
