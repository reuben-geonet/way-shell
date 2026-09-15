#pragma once

#include <adwaita.h>

G_BEGIN_DECLS

// The mediator which aggregates and emits signals on behavior of Panels.
// This is useful since multiple Panels can exist, one per GdkMonitor.
// External components can connect to signals the PanelMediator emits rather
// then each Panel.
//
// PanelMediator also acts as a high-level signal router for cross cutting
// needs, such as closing the MessageTray component when the QuickSettings
// component is opened.
struct _PanelMediator;
#define PANEL_MEDIATOR_TYPE panel_mediator_get_type()
G_DECLARE_FINAL_TYPE(PanelMediator, panel_mediator, PANEL, MEDIATOR, GObject);

G_END_DECLS

// Connects the PanelMediator to all other Mediator's signals required.
void panel_mediator_connect(PanelMediator *mediator);
