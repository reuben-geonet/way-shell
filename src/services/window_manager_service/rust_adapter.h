#pragma once
#include <glib.h>
#include <stdint.h>

typedef void (*WayShellWmNotify)(void *data, GPtrArray *snapshot);
/* Rust owns the handle; callbacks borrow data until free disconnects them. */
void *way_shell_wm_new_sway(WayShellWmNotify workspaces, WayShellWmNotify outputs, void *data);
void way_shell_wm_free(void *handle);
/* Owned array references; entries stay valid while an array reference is held. */
GPtrArray *way_shell_wm_workspaces(void *handle);
GPtrArray *way_shell_wm_outputs(void *handle);
int way_shell_wm_workspace_action(void *handle, int move_window, uint64_t id, int32_t number, const char *name);
int way_shell_wm_named_action(void *handle, int move_output, const char *name);
