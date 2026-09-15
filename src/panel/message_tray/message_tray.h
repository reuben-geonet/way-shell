#pragma once

#include <adwaita.h>

G_BEGIN_DECLS

// Temporary lifecycle signals for the Rust message-tray controller.
struct _MessageTray;
#define MESSAGE_TRAY_TYPE message_tray_get_type()
G_DECLARE_FINAL_TYPE(MessageTray, message_tray, MESSAGE_TRAY, TRAY, GObject);

G_END_DECLS

void message_tray_activate(AdwApplication *app, gpointer user_data);

MessageTray *message_tray_get_global(void);

// Opens the MessageTray on the current monitor.
void message_tray_set_visible(MessageTray *self);

// Closes the MessageTray.
void message_tray_set_hidden(MessageTray *self);

void message_tray_shrink(MessageTray *self);

void message_tray_toggle(MessageTray *self);
