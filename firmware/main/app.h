#pragma once

#include "cJSON.h"

/* Send a message to the iPhone over the BLE control link. Takes ownership of msg. */
void app_emit(cJSON *msg);
