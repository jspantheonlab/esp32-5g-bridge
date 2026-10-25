#pragma once

#include <stddef.h>
#include "esp_err.h"
#include "esp_netif.h"

/* Starts the AP with credentials saved from the app (NVS), else the Kconfig defaults. */
esp_err_t hotspot_init(void);

void hotspot_get_credentials(char *ssid, size_t ssid_len, char *pass, size_t pass_len);

/* Validates, applies (clients must rejoin) and persists. Returns NULL on success or an error message. */
const char *hotspot_set_credentials(const char *ssid, const char *pass);

/* Returns NULL if valid, otherwise a human-readable reason. */
const char *hotspot_validate(const char *ssid, const char *pass);

/* Route hotspot clients through uplink (NAPT) and hand out its DNS server. */
esp_err_t hotspot_set_uplink(esp_netif_t *uplink);
void hotspot_clear_uplink(void);
