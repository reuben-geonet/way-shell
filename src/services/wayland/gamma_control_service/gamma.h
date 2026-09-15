#pragma once
#include <adwaita.h>

G_BEGIN_DECLS
#define WAYLAND_GAMMA_CONTROL_SERVICE_TYPE wayland_gamma_control_service_get_type()
G_DECLARE_FINAL_TYPE(WaylandGammaControlService, wayland_gamma_control_service,
                     WAYLAND, GAMMA_CONTROL_SERVICE, GObject);
G_END_DECLS

WaylandGammaControlService *wayland_gamma_control_service_get_global(void);
void wayland_gamma_control_service_set_temperature(WaylandGammaControlService *self,
                                                   double temperature);
void wayland_gamma_control_service_destroy(WaylandGammaControlService *self);
gboolean wayland_gamma_control_service_enabled(WaylandGammaControlService *self);
gboolean wayland_gamma_control_service_available(WaylandGammaControlService *self);
