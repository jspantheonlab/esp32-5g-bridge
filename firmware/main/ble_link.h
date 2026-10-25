#pragma once

#include <stddef.h>
#include "esp_err.h"

/*
 * BLE GATT control link to the iPhone app. Newline-delimited JSON in both directions,
 * fragmented to the ATT MTU. Characteristics require an encrypted, bonded (passkey) link.
 *
 *   Service  A5E10001-5C0E-4B6A-9D3E-6B2F0C5A1E01
 *   RX       A5E10002-...  (write)   iPhone -> ESP32
 *   TX       A5E10003-...  (notify)  ESP32 -> iPhone
 */
typedef struct {
    void (*on_line)(char *line);   /* callee takes ownership (free()) */
    void (*on_ready)(void);        /* link encrypted and TX subscribed */
} ble_link_callbacks_t;

esp_err_t ble_link_init(const ble_link_callbacks_t *cbs);
esp_err_t ble_link_send(const char *data, size_t len);
