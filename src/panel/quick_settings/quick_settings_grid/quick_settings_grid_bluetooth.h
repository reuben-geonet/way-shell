#pragma once

#include "quick_settings_grid_button.h"
#include "../../../services/bluetooth_service/bluetooth_service.h"

typedef struct _QuickSettingsGridBluetoothButton QuickSettingsGridBluetoothButton;

QuickSettingsGridBluetoothButton *quick_settings_grid_bluetooth_button_init(BluetoothService *service);
void quick_settings_grid_bluetooth_button_free(QuickSettingsGridBluetoothButton *self);
