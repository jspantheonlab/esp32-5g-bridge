#pragma once

#include "esp_err.h"
#include "esp_netif.h"

esp_err_t hotspot_init(void);

/* Route hotspot clients through uplink (NAPT) and hand out its DNS server. */
esp_err_t hotspot_set_uplink(esp_netif_t *uplink);
void hotspot_clear_uplink(void);
