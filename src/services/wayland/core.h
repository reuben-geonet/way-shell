#pragma once
#include <adwaita.h>

G_BEGIN_DECLS
#define WAYLAND_CORE_SERVICE_TYPE wayland_core_service_get_type()
G_DECLARE_FINAL_TYPE(WaylandCoreService, wayland_core_service, WAYLAND, CORE_SERVICE, GObject);
G_END_DECLS

int wayland_core_service_global_init(void);
WaylandCoreService *wayland_core_service_get_global(void);
