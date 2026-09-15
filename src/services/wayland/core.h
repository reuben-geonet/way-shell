#pragma once
#include <adwaita.h>

/* Compatibility identifiers only. Protocol objects stay in the Rust service. */
enum WaylandType { WL_REGISTRY, WL_SEAT, WL_OUTPUT, WLR_FOREIGN_TOPLEVEL, WLR_GAMMA_CONTROL };
typedef struct _WaylandHeader { enum WaylandType type; } WaylandHeader;

G_BEGIN_DECLS
#define WAYLAND_CORE_SERVICE_TYPE wayland_core_service_get_type()
G_DECLARE_FINAL_TYPE(WaylandCoreService, wayland_core_service, WAYLAND, CORE_SERVICE, GObject);
G_END_DECLS

int wayland_core_service_global_init(void);
WaylandCoreService *wayland_core_service_get_global(void);
