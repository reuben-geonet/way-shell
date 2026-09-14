#include <adwaita.h>
#include <fcntl.h>
#include <unistd.h>
#include "../src/services/logind_service/logind_manager_dbus.h"
#include "../src/services/logind_service/logind_session_dbus.h"
static DbusLogin1Manager *manager_fixture;
static DbusLogin1Session *own_session, *other_session;
static GSettings *settings_fixture;
static gboolean deny, invalid_fd;
static int reader;
static DbusLogin1Manager *manager_proxy(GDBusConnection *connection, GDBusProxyFlags flags,
    const char *name, const char *path, GCancellable *cancel, GError **error) {
    return g_object_ref(manager_fixture);
}
static DbusLogin1Session *session_proxy(GDBusConnection *connection, GDBusProxyFlags flags,
    const char *name, const char *path, GCancellable *cancel, GError **error) {
    return g_object_ref(g_str_has_suffix(path, "/own") ? own_session : other_session);
}
static gboolean list_sessions(DbusLogin1Manager *manager, GVariant **sessions,
    GCancellable *cancel, GError **error) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a(susso)"));
    g_variant_builder_add(&builder, "(susso)", "other", (guint32)getuid() + 1, "other", "seat1", "/org/freedesktop/login1/session/other");
    g_variant_builder_add(&builder, "(susso)", "own", (guint32)getuid(), "own", "seat0", "/org/freedesktop/login1/session/own");
    *sessions = g_variant_ref_sink(g_variant_builder_end(&builder));
    return TRUE;
}
static GSettings *settings_new(const gchar *schema) { return g_object_ref(settings_fixture); }
static GVariant *inhibit_call(GDBusConnection *connection, const char *name,
    const char *path, const char *interface, const char *method, GVariant *parameters,
    const GVariantType *reply, GDBusCallFlags flags, gint timeout, GUnixFDList *input,
    GUnixFDList **output, GCancellable *cancel, GError **error) {
    if (deny) { g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED, "fixture denied inhibitor"); return NULL; }
    int fds[2];
    g_assert_cmpint(pipe(fds), ==, 0);
    reader = fds[0];
    *output = g_unix_fd_list_new();
    int index = g_unix_fd_list_append(*output, fds[1], NULL);
    close(fds[1]);
    return g_variant_ref_sink(g_variant_new("(h)", invalid_fd ? 77 : index));
}
#define dbus_login1_manager_proxy_new_sync manager_proxy
#define dbus_login1_session_proxy_new_sync session_proxy
#define dbus_login1_manager_call_list_sessions_sync list_sessions
#define g_settings_new settings_new
#define g_dbus_connection_call_with_unix_fd_list_sync inhibit_call
#include "../src/services/logind_service/logind_service.c"
#undef g_settings_new
DBUSService *dbus_service_get_global(void) { return NULL; }
GDBusConnection *dbus_service_get_system_bus(DBUSService *self) { return NULL; }
static LogindService *fixture(gboolean initial) {
    manager_fixture = dbus_login1_manager_skeleton_new();
    own_session = dbus_login1_session_skeleton_new();
    other_session = dbus_login1_session_skeleton_new();
    dbus_login1_session_set_state(own_session, "active");
    dbus_login1_session_set_state(other_session, "active");
    settings_fixture = g_settings_new("org.ldelossa.way-shell.system");
    g_settings_set_boolean(settings_fixture, "idle-inhibitor", initial);
    deny = invalid_fd = FALSE;
    return g_object_new(LOGIND_SERVICE_TYPE, NULL);
}
static void cleanup(LogindService *service) {
    g_object_unref(service);
    g_object_unref(manager_fixture);
    g_object_unref(own_session);
    g_object_unref(other_session);
    g_object_unref(settings_fixture);
}
static void selects_own_session(void) {
    LogindService *service = fixture(FALSE);
    g_assert_true(service->session == own_session);
    cleanup(service);
}
static void releases_fd_on_dispose(void) {
    LogindService *service = fixture(FALSE);
    g_assert_true(logind_service_set_idle_inhibit(service, TRUE));
    int fd = service->idle_inhibitor_fd;
    cleanup(service);
    g_assert_cmpint(fcntl(fd, F_GETFD), ==, -1);
    g_assert_cmpint(errno, ==, EBADF);
    char byte;
    g_assert_cmpint(read(reader, &byte, 1), ==, 0);
    close(reader);
}
static void handles_denied_request(void) {
    LogindService *service = fixture(FALSE);
    deny = TRUE;
    g_test_expect_message(NULL, G_LOG_LEVEL_WARNING, "*fixture denied inhibitor*");
    g_assert_false(logind_service_set_idle_inhibit(service, TRUE));
    g_test_assert_expected_messages();
    g_assert_false(logind_service_get_idle_inhibit(service));
    cleanup(service);
}
static void rejects_invalid_descriptor_index(void) {
    LogindService *service = fixture(FALSE);
    invalid_fd = TRUE;
    g_test_expect_message(NULL, G_LOG_LEVEL_WARNING, "*invalid descriptor index*");
    g_assert_false(logind_service_set_idle_inhibit(service, TRUE));
    g_test_assert_expected_messages();
    g_assert_false(logind_service_get_idle_inhibit(service));
    char byte;
    g_assert_cmpint(read(reader, &byte, 1), ==, 0);
    close(reader);
    cleanup(service);
}
static void applies_initial_setting(void) {
    LogindService *service = fixture(TRUE);
    g_assert_true(logind_service_get_idle_inhibit(service));
    g_settings_set_boolean(settings_fixture, "idle-inhibitor", FALSE);
    g_assert_false(logind_service_get_idle_inhibit(service));
    cleanup(service);
    close(reader);
}
int main(int argc, char **argv) {
    g_setenv("GSETTINGS_BACKEND", "memory", TRUE);
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/logind/own-session", selects_own_session);
    g_test_add_func("/logind/dispose-inhibitor", releases_fd_on_dispose);
    g_test_add_func("/logind/denied-inhibitor", handles_denied_request);
    g_test_add_func("/logind/initial-setting", applies_initial_setting);
    g_test_add_func("/logind/invalid-descriptor", rejects_invalid_descriptor_index);
    return g_test_run();
}
