#include <string.h>
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_wifi.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "nvs.h"
#include "hotspot.h"

#ifndef DHCPS_OFFER_DNS
#define DHCPS_OFFER_DNS 0x02
#endif

#define NVS_NAMESPACE "bridge"
#define NVS_SSID      "ap_ssid"
#define NVS_PASS      "ap_pass"
#define MAX_SSID_LEN  32
#define MAX_PASS_LEN  63

static const char *TAG = "hotspot";
static esp_netif_t *s_ap;
static esp_netif_t *s_uplink;
static bool s_started;
static char s_ssid[MAX_SSID_LEN + 1];
static char s_pass[MAX_PASS_LEN + 1];

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

/* SSID 1-32 bytes; WPA2 passphrase 8-63 printable ASCII. Mirrors bridge-core/src/wifi.rs. */
const char *hotspot_validate(const char *ssid, const char *pass)
{
    size_t ssid_len = ssid ? strlen(ssid) : 0;
    size_t pass_len = pass ? strlen(pass) : 0;
    if (ssid_len == 0) {
        return "network name is empty";
    }
    if (ssid_len > MAX_SSID_LEN) {
        return "network name is longer than 32 bytes";
    }
    if (pass_len < 8 || pass_len > MAX_PASS_LEN) {
        return "password must be 8 to 63 characters";
    }
    for (size_t i = 0; i < pass_len; i++) {
        if (pass[i] < 0x20 || pass[i] > 0x7E) {
            return "password may only contain printable ASCII characters";
        }
    }
    return NULL;
}

static esp_err_t apply_config(void)
{
    wifi_config_t wc = { 0 };
    memcpy(wc.ap.ssid, s_ssid, strlen(s_ssid));   /* SSID may be exactly 32 bytes, no terminator */
    wc.ap.ssid_len = strlen(s_ssid);
    strlcpy((char *)wc.ap.password, s_pass, sizeof(wc.ap.password));
    wc.ap.channel = CONFIG_BRIDGE_AP_CHANNEL;
    wc.ap.max_connection = CONFIG_BRIDGE_AP_MAX_CONN;
    wc.ap.authmode = WIFI_AUTH_WPA2_WPA3_PSK;
    wc.ap.sae_pwe_h2e = WPA3_SAE_PWE_BOTH;
    wc.ap.pmf_cfg.required = false;

    if (s_started) {
        esp_wifi_stop();
        s_started = false;
    }
    esp_err_t err = esp_wifi_set_config(WIFI_IF_AP, &wc);
    if (err == ESP_OK) {
        err = esp_wifi_start();
    }
    if (err == ESP_OK) {
        s_started = true;
        ESP_LOGI(TAG, "Hotspot \"%s\" up on channel %d", s_ssid, CONFIG_BRIDGE_AP_CHANNEL);
    }
    return err;
}

static void load_credentials(void)
{
    strlcpy(s_ssid, CONFIG_BRIDGE_AP_SSID, sizeof(s_ssid));
    strlcpy(s_pass, CONFIG_BRIDGE_AP_PASSWORD, sizeof(s_pass));

    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    char ssid[MAX_SSID_LEN + 1], pass[MAX_PASS_LEN + 1];
    size_t ssid_size = sizeof(ssid), pass_size = sizeof(pass);
    if (nvs_get_str(nvs, NVS_SSID, ssid, &ssid_size) == ESP_OK &&
        nvs_get_str(nvs, NVS_PASS, pass, &pass_size) == ESP_OK && !hotspot_validate(ssid, pass)) {
        strlcpy(s_ssid, ssid, sizeof(s_ssid));
        strlcpy(s_pass, pass, sizeof(s_pass));
        ESP_LOGI(TAG, "Using hotspot settings saved from the app");
    }
    nvs_close(nvs);
}

esp_err_t hotspot_init(void)
{
    load_credentials();
    const char *problem = hotspot_validate(s_ssid, s_pass);
    if (problem) {
        ESP_LOGE(TAG, "Invalid hotspot settings: %s", problem);
        return ESP_ERR_INVALID_ARG;
    }

    s_ap = esp_netif_create_default_wifi_ap();

    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&cfg));
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, on_wifi_event, NULL));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_AP));
    return apply_config();
}

void hotspot_get_credentials(char *ssid, size_t ssid_len, char *pass, size_t pass_len)
{
    if (ssid) {
        strlcpy(ssid, s_ssid, ssid_len);
    }
    if (pass) {
        strlcpy(pass, s_pass, pass_len);
    }
}

const char *hotspot_set_credentials(const char *ssid, const char *pass)
{
    const char *problem = hotspot_validate(ssid, pass);
    if (problem) {
        return problem;
    }

    char old_ssid[sizeof(s_ssid)], old_pass[sizeof(s_pass)];
    strlcpy(old_ssid, s_ssid, sizeof(old_ssid));
    strlcpy(old_pass, s_pass, sizeof(old_pass));
    strlcpy(s_ssid, ssid, sizeof(s_ssid));
    strlcpy(s_pass, pass, sizeof(s_pass));
    if (apply_config() != ESP_OK) {
        strlcpy(s_ssid, old_ssid, sizeof(s_ssid));
        strlcpy(s_pass, old_pass, sizeof(s_pass));
        apply_config();
        return "could not apply new settings";
    }

    /* Restarting the AP resets DHCP/NAPT state; restore routing if the uplink is up. */
    if (s_uplink) {
        hotspot_set_uplink(s_uplink);
    }

    nvs_handle_t nvs;
    if (nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs) == ESP_OK) {
        nvs_set_str(nvs, NVS_SSID, s_ssid);
        nvs_set_str(nvs, NVS_PASS, s_pass);
        nvs_commit(nvs);
        nvs_close(nvs);
    }
    ESP_LOGI(TAG, "Hotspot settings changed (SSID \"%s\")", s_ssid);
    return NULL;
}

esp_err_t hotspot_set_uplink(esp_netif_t *uplink)
{
    s_uplink = uplink;
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
    s_uplink = NULL;
    esp_netif_napt_disable(s_ap);
}
