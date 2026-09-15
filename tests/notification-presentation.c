#include <adwaita.h>

#include "../src/panel/message_tray/notifications/notification_widget.c"

typedef struct { GObject parent_instance; } FixtureServices;
typedef struct { GObjectClass parent_class; } FixtureServicesClass;
G_DEFINE_TYPE(FixtureServices, fixture_services, G_TYPE_OBJECT)
static GObject *services;
static GPtrArray *actions;
static guint last_id, closed, hidden;
static void fixture_services_class_init(FixtureServicesClass *klass) {
    g_signal_new("message-tray-will-hide", G_TYPE_FROM_CLASS(klass),
                 G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_services_init(FixtureServices *self) {}
MessageTray *message_tray_get_global(void) { return (MessageTray *)services; }
NotificationsService *notifications_service_get_global(void) { return (NotificationsService *)services; }
int notifications_service_closed_notification(NotificationsService *self, guint32 id,
                                              enum NotifcationsClosedReason reason) {
    last_id = id; closed++; return 0;
}
int notifications_service_invoke_action(NotificationsService *self, guint32 id, char *action) {
    last_id = id; g_ptr_array_add(actions, g_strdup(action)); return 0;
}
void notification_osd_hide(NotificationsOSD *self) { hidden++; }

static void setup(void) {
    services = g_object_new(fixture_services_get_type(), NULL);
    actions = g_ptr_array_new_with_free_func(g_free);
    last_id = closed = hidden = 0;
}
static void teardown(void) {
    g_object_unref(services);
    g_ptr_array_unref(actions);
}
static Notification notification(void) {
    return (Notification){.id = 42, .summary = g_strdup("Fixture summary"),
        .body = g_strdup("Fixture body"), .created_on = g_date_time_new_now_local()};
}
static void notification_clear(Notification *n) {
    g_free(n->summary); g_free(n->body); g_date_time_unref(n->created_on);
}

static void action_widgets_have_one_valid_parent(void) {
    setup();
    Notification n = notification();
    gchar *choices[] = {"default", "Open", "reply", "Reply", "dismiss", "Dismiss", NULL};
    n.actions = choices;
    NotificationWidget *widget = notification_widget_from_notification(&n, FALSE);
    GtkWidget *container = g_object_ref_sink(notification_widget_get_widget(widget));
    GtkWidget *center = gtk_revealer_get_child(widget->action_revealer);
    g_assert_true(GTK_IS_CENTER_BOX(center));
    g_assert_true(gtk_widget_get_parent(center) == GTK_WIDGET(widget->action_revealer));
    g_assert_true(gtk_widget_get_parent(GTK_WIDGET(widget->action_container)) == center);
    g_assert_true(gtk_widget_get_parent(GTK_WIDGET(widget->action_revealer)) ==
                  GTK_WIDGET(widget->notification_container));
    g_assert_cmpuint(widget->actions_buttons_n, ==, 2);
    GtkWidget *reply = gtk_widget_get_first_child(GTK_WIDGET(widget->action_container));
    GtkWidget *dismiss = gtk_widget_get_next_sibling(reply);
    g_assert_cmpstr(gtk_button_get_label(GTK_BUTTON(reply)), ==, "Reply");
    g_assert_cmpstr(gtk_button_get_label(GTK_BUTTON(dismiss)), ==, "Dismiss");
    g_assert_null(gtk_widget_get_next_sibling(dismiss));
    g_signal_emit_by_name(reply, "clicked");
    g_assert_cmpuint(actions->len, ==, 1);
    g_assert_cmpstr(g_ptr_array_index(actions, 0), ==, "reply");
    g_assert_cmpuint(last_id, ==, 42);
    g_object_unref(widget);
    g_object_unref(container);
    notification_clear(&n);
    teardown();
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/notification-presentation/action-parentage", action_widgets_have_one_valid_parent);
    return g_test_run();
}
