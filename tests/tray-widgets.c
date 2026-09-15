#include <adwaita.h>
#include "../src/services/status_notifier_service/status_notifier_service.h"

#include "../src/panel/indicator_bar/indicator_widget.c"
#include "../src/panel/indicator_bar/indicator_bar.c"

typedef struct { GObject parent_instance; } FixtureTray;
typedef struct { GObjectClass parent_class; } FixtureTrayClass;
G_DEFINE_TYPE(FixtureTray, fixture_tray, G_TYPE_OBJECT)
static GObject *source;
static GHashTable *items;
static guint activations, menu_requests;

static void fixture_tray_class_init(FixtureTrayClass *klass) {
    GType type = G_TYPE_FROM_CLASS(klass);
    const gchar *inventory[] = {"status-notifier-item-added", "status-notifier-item-removed"};
    for (guint i = 0; i < G_N_ELEMENTS(inventory); i++)
        g_signal_new(inventory[i], type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                     G_TYPE_NONE, 2, G_TYPE_HASH_TABLE, G_TYPE_POINTER);
    const gchar *updates[] = {"status-notifier-item-properties-changed",
                             "status-notifier-item-menu-will-update", "status-notifier-item-menu-updated"};
    for (guint i = 0; i < G_N_ELEMENTS(updates); i++)
        g_signal_new(updates[i], type, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                     G_TYPE_NONE, 1, G_TYPE_POINTER);
}
static void fixture_tray_init(FixtureTray *self) {}
StatusNotifierService *status_notifier_service_get_global(void) { return (StatusNotifierService *)source; }
GHashTable *status_notifier_service_get_items(StatusNotifierService *self) { return items; }
const gchar *status_notifier_item_get_key(StatusNotifierItem *item) { return item->register_service_name; }
const gchar *status_notifier_item_get_icon_name(StatusNotifierItem *item) { return item->icon_name; }
GdkPixbuf *status_notifier_item_get_icon_pixmap(StatusNotifierItem *item) { return item->icon_pixmap; }
void status_notifier_item_activate(StatusNotifierItem *item, gint32 x, gint32 y) { activations++; }
void status_notifier_item_about_to_show(StatusNotifierItem *item, gint32 id) { menu_requests++; }
static void setup(void) {
    source = g_object_new(fixture_tray_get_type(), NULL);
    items = g_hash_table_new(g_str_hash, g_str_equal);
    activations = menu_requests = 0;
}
static void teardown(void) { g_hash_table_unref(items); g_object_unref(source); }
static StatusNotifierItem item(const gchar *key) {
    return (StatusNotifierItem){.bus_name = ":1.42", .obj_name = strchr(key, '/'),
        .register_service_name = (gchar *)key, .icon_name = "network-wired-symbolic"};
}
static guint children(GtkWidget *parent) {
    guint count = 0;
    for (GtkWidget *child = gtk_widget_get_first_child(parent); child;
         child = gtk_widget_get_next_sibling(child)) count++;
    return count;
}
static void gone(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void painted(GdkFrameClock *clock, gpointer data) { *(gboolean *)data = TRUE; }
static void add(StatusNotifierItem *item) {
    g_hash_table_insert(items, item->register_service_name, item);
    g_signal_emit_by_name(source, "status-notifier-item-added", items, item);
}
static void remove_item(StatusNotifierItem *item) {
    g_signal_emit_by_name(source, "status-notifier-item-removed", items, item);
    g_hash_table_remove(items, item->register_service_name);
}

static void menu_less_items_receive_icon_updates(void) {
    setup();
    StatusNotifierItem first = item(":1.42/First");
    IndicatorWidget *widget = g_object_new(INDICATOR_WIDGET_TYPE, NULL);
    indicator_widget_set_sni(widget, &first);
    g_assert_cmpstr(gtk_image_get_icon_name(widget->icon), ==, first.icon_name);
    first.icon_name = "network-wireless-symbolic";
    g_signal_emit_by_name(source, "status-notifier-item-properties-changed", &first);
    g_assert_cmpstr(gtk_image_get_icon_name(widget->icon), ==, first.icon_name);
    g_signal_emit_by_name(widget->button, "clicked");
    g_assert_cmpuint(activations, ==, 1);
    g_object_unref(widget);
    teardown();
}

static void menus_can_arrive_change_and_disappear(void) {
    setup();
    StatusNotifierItem first = item(":1.42/First");
    IndicatorWidget *widget = g_object_new(INDICATOR_WIDGET_TYPE, NULL);
    indicator_widget_set_sni(widget, &first);
    GtkWindow *window = GTK_WINDOW(gtk_window_new());
    gtk_window_set_child(window, indicator_widget_get_widget(widget));
    gtk_window_present(window);
    GdkFrameClock *clock = gtk_widget_get_frame_clock(GTK_WIDGET(window));
    gboolean frame_presented = FALSE;
    gulong frame_handler = g_signal_connect(clock, "after-paint", G_CALLBACK(painted), &frame_presented);
    gint64 deadline = g_get_monotonic_time() + 2 * G_TIME_SPAN_SECOND;
    while (!frame_presented) {
        g_assert_cmpint(g_get_monotonic_time(), <, deadline);
        g_main_context_iteration(NULL, FALSE);
        g_usleep(1000);
    }
    g_signal_handler_disconnect(clock, frame_handler);
    first.menu_model = g_menu_new();
    g_menu_append(first.menu_model, "Open", "sni.item-clicked");
    first.action_group = G_ACTION_GROUP(g_simple_action_group_new());
    g_signal_emit_by_name(source, "status-notifier-item-menu-updated", &first);
    g_assert_nonnull(widget->menu);
    g_assert_true(gtk_widget_get_parent(GTK_WIDGET(widget->menu)) == GTK_WIDGET(widget->button));
    g_assert_true(gtk_popover_menu_get_menu_model(widget->menu) == G_MENU_MODEL(first.menu_model));
    /* Programmatic fixture clicks have no Wayland input serial for a grab. */
    gtk_popover_set_autohide(GTK_POPOVER(widget->menu), FALSE);
    g_signal_emit_by_name(widget->button, "clicked");
    g_assert_cmpuint(menu_requests, ==, 1);
    g_assert_cmpuint(activations, ==, 0);
    gtk_popover_popdown(GTK_POPOVER(widget->menu));
    GtkPopoverMenu *old = g_object_ref(widget->menu);
    g_clear_object(&first.menu_model);
    first.menu_model = g_menu_new();
    g_menu_append(first.menu_model, "Changed", "sni.item-clicked");
    g_signal_emit_by_name(source, "status-notifier-item-menu-updated", &first);
    g_assert_true(widget->menu == old);
    g_assert_true(gtk_popover_menu_get_menu_model(widget->menu) == G_MENU_MODEL(first.menu_model));
    g_clear_object(&first.menu_model);
    g_clear_object(&first.action_group);
    g_signal_emit_by_name(source, "status-notifier-item-menu-updated", &first);
    g_assert_null(widget->menu);
    g_assert_null(gtk_widget_get_parent(GTK_WIDGET(old)));
    g_signal_emit_by_name(widget->button, "clicked");
    g_assert_cmpuint(activations, ==, 1);
    g_object_unref(old);
    gtk_window_destroy(window);
    g_object_unref(widget);
    teardown();
}

static void duplicate_addition_keeps_one_widget(void) {
    setup();
    StatusNotifierItem first = item(":1.42/First");
    IndicatorBar *bar = g_object_new(INDICATOR_BAR_TYPE, NULL);
    add(&first);
    GtkWidget *original = gtk_widget_get_first_child(GTK_WIDGET(bar->list));
    add(&first);
    g_assert_cmpuint(children(GTK_WIDGET(bar->list)), ==, 1);
    g_assert_true(gtk_widget_get_first_child(GTK_WIDGET(bar->list)) == original);
    remove_item(&first);
    g_assert_cmpuint(children(GTK_WIDGET(bar->list)), ==, 0);
    g_object_unref(bar);
    teardown();
}

static void removal_releases_controller_and_root(void) {
    setup();
    StatusNotifierItem first = item(":1.42/First");
    IndicatorBar *bar = g_object_new(INDICATOR_BAR_TYPE, NULL);
    add(&first);
    GList *values = g_hash_table_get_values(bar->indicators);
    IndicatorWidget *widget = values->data;
    g_list_free(values);
    gboolean controller_gone = FALSE, root_gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget), gone, &controller_gone);
    g_object_weak_ref(G_OBJECT(indicator_widget_get_widget(widget)), gone, &root_gone);
    remove_item(&first);
    g_assert_true(controller_gone);
    g_assert_true(root_gone);
    g_object_unref(bar);
    teardown();
}

static void distinct_paths_and_disposal_preserve_ownership(void) {
    setup();
    StatusNotifierItem first = item(":1.42/First"), second = item(":1.42/Second");
    g_hash_table_insert(items, first.register_service_name, &first);
    g_hash_table_insert(items, second.register_service_name, &second);
    IndicatorBar *bar = g_object_new(INDICATOR_BAR_TYPE, NULL);
    g_assert_cmpuint(g_hash_table_size(bar->indicators), ==, 2);
    GtkWidget *root = g_object_ref(indicator_bar_get_widget(bar));
    g_object_run_dispose(G_OBJECT(bar));
    g_object_run_dispose(G_OBJECT(bar));
    add(&first);
    g_object_unref(bar);
    g_object_unref(root);
    teardown();
}

static void retained_buttons_do_not_call_destroyed_controllers(void) {
    setup();
    StatusNotifierItem first = item(":1.42/First");
    IndicatorWidget *widget = g_object_new(INDICATOR_WIDGET_TYPE, NULL);
    indicator_widget_set_sni(widget, &first);
    GtkButton *button = g_object_ref(widget->button);
    gboolean controller_gone = FALSE;
    g_object_weak_ref(G_OBJECT(widget), gone, &controller_gone);
    g_object_unref(widget);
    g_assert_true(controller_gone);
    g_signal_emit_by_name(button, "clicked");
    g_assert_cmpuint(activations, ==, 0);
    g_object_unref(button);
    teardown();
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_setenv("GSK_RENDERER", "cairo", TRUE);
    gtk_init();
    g_test_add_func("/tray-widgets/menu-less-update", menu_less_items_receive_icon_updates);
    g_test_add_func("/tray-widgets/late-menu", menus_can_arrive_change_and_disappear);
    g_test_add_func("/tray-widgets/duplicate", duplicate_addition_keeps_one_widget);
    g_test_add_func("/tray-widgets/removal", removal_releases_controller_and_root);
    g_test_add_func("/tray-widgets/distinct-paths", distinct_paths_and_disposal_preserve_ownership);
    g_test_add_func("/tray-widgets/retained-button", retained_buttons_do_not_call_destroyed_controllers);
    return g_test_run();
}
