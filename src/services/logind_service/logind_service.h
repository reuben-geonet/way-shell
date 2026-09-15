#pragma once

#include <adwaita.h>

G_BEGIN_DECLS

struct _LogindService;
#define LOGIND_SERVICE_TYPE logind_service_get_type()
G_DECLARE_FINAL_TYPE(LogindService, logind_service, LOGIND, SERVICE, GObject);

G_END_DECLS

int logind_service_global_init(void);

LogindService *logind_service_get_global();

gboolean logind_service_get_enabled(LogindService *self);
gboolean logind_service_get_session_enabled(LogindService *self);

gboolean logind_service_can_reboot(LogindService *self);

void logind_service_reboot(LogindService *self);

gboolean logind_service_can_power_off(LogindService *self);

void logind_service_power_off(LogindService *self);

gboolean logind_service_can_suspend(LogindService *self);

void logind_service_suspend(LogindService *self);

gboolean logind_service_can_hibernate(LogindService *self);

void logind_service_hibernate(LogindService *self);

gboolean logind_service_can_hybrid_sleep(LogindService *self);

void logind_service_hybrid_sleep(LogindService *self);

gboolean logind_service_can_suspendthenhibernate(LogindService *self);

void logind_service_suspendthenhibernate(LogindService *self);

void logind_service_kill_session(LogindService *self);

// Strings are copied before return; error is borrowed during completion.
// destroy(data) runs exactly once after completion, including cancellation.
typedef void (*LogindBrightnessDone)(gboolean success, const gchar *error, gpointer data);
void logind_service_session_set_brightness_async(LogindService *self,
    const gchar *subsystem, const gchar *name, guint brightness,
    LogindBrightnessDone done, gpointer data, GDestroyNotify destroy);

gboolean logind_service_set_idle_inhibit(LogindService *self, gboolean enable);

gboolean logind_service_get_idle_inhibit(LogindService *self);
