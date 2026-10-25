#include <string.h>
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_wifi.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "hotspot.h"

#ifndef DHCPS_OFFER_DNS
#define DHCPS_OFFER_DNS 0x02
#endif

static const char *TAG = "hotspot";
static esp_netif_t *s_ap;

static void on_wifi_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    if (id == WIFI_EVENT_AP_STACONNECTED) {
        wifi_event_ap_staconnected_t *e = data;
        ESP_LOGI(TAG, "Client " MACSTR " joined", MAC2STR(e->mac));
    } else if (id == WIFI_EVENT_AP_STADISCONNECTED) {
        wifi_event_ap_stadisconnected_t *e = data;
        ESP_LOGI(TAG, "Client " MACSTR " left", MAC2STR(e->mac));
    }
}

esp_err_t hotspot_init(void)
{
    if (strlen(CONFIG_BRIDGE_AP_PASSWORD) < 8) {
        ESP_LOGE(TAG, "Hotspot password must be at least 8 characters");
        return ESP_ERR_INVALID_ARG;
    }

    s_ap = esp_netif_create_default_wifi_ap();

    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&cfg));
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, on_wifi_event, NULL));

    wifi_config_t wc = { 0 };
    strlcpy((char *)wc.ap.ssid, CONFIG_BRIDGE_AP_SSID, sizeof(wc.ap.ssid));
    wc.ap.ssid_len = strlen(CONFIG_BRIDGE_AP_SSID);
    strlcpy((char *)wc.ap.password, CONFIG_BRIDGE_AP_PASSWORD, sizeof(wc.ap.password));
    wc.ap.channel = CONFIG_BRIDGE_AP_CHANNEL;
    wc.ap.max_connection = CONFIG_BRIDGE_AP_MAX_CONN;
    wc.ap.authmode = WIFI_AUTH_WPA2_WPA3_PSK;
    wc.ap.sae_pwe_h2e = WPA3_SAE_PWE_BOTH;
    wc.ap.pmf_cfg.required = false;

    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_AP));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &wc));
    ESP_ERROR_CHECK(esp_wifi_start());
    ESP_LOGI(TAG, "Hotspot \"%s\" up on channel %d", CONFIG_BRIDGE_AP_SSID, CONFIG_BRIDGE_AP_CHANNEL);
    return ESP_OK;
}

esp_err_t hotspot_set_uplink(esp_netif_t *uplink)
{
    esp_netif_dns_info_t dns;
    if (esp_netif_get_dns_info(uplink, ESP_NETIF_DNS_MAIN, &dns) == ESP_OK) {
        uint8_t offer = DHCPS_OFFER_DNS;
        esp_netif_dhcps_stop(s_ap);
        esp_netif_dhcps_option(s_ap, ESP_NETIF_OP_SET, ESP_NETIF_DOMAIN_NAME_SERVER, &offer, sizeof(offer));
        esp_netif_set_dns_info(s_ap, ESP_NETIF_DNS_MAIN, &dns);
        esp_netif_dhcps_start(s_ap);
    }
    esp_netif_set_default_netif(uplink);
    esp_err_t err = esp_netif_napt_enable(s_ap);
    ESP_LOGI(TAG, "NAPT %s", err == ESP_OK ? "enabled" : esp_err_to_name(err));
    return err;
}

void hotspot_clear_uplink(void)
{
    esp_netif_napt_disable(s_ap);
}
