/*
 * Factory reset by holding the BOOT button (GPIO0, present on the DevKit and the custom board).
 * Hold 5 s for a warning, 10 s to reset: clears settings saved from the app (hotspot name and
 * password) and all Bluetooth pairings, then restarts. SIM contents and modem settings are kept.
 * Same logic as firmware-rs (bridge-core/src/button.rs, firmware/src/reset.rs).
 */
#include <stdio.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/gpio.h"
#include "esp_log.h"
#include "esp_system.h"
#include "nvs.h"
#include "host/ble_hs.h"
#include "app.h"
#include "reset_button.h"

#define POLL_MS         50
#define WARN_AFTER_MS   5000
#define RESET_AFTER_MS  (CONFIG_BRIDGE_RESET_HOLD_SECONDS * 1000)
#define SETTINGS_NVS_NS "bridge"   /* everything set from the app (see hotspot.c) */

static const char *TAG = "reset";

static void notice(const char *text)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "notice");
    cJSON_AddStringToObject(msg, "msg", text);
    app_emit(msg);
}

static void factory_reset(void)
{
    ESP_LOGW(TAG, "Factory reset: clearing saved settings and Bluetooth pairings");
    notice("Factory reset: the bridge is restarting. Forget \"5G-Bridge\" in iPhone Settings → Bluetooth, "
           "then pair again.");
    vTaskDelay(pdMS_TO_TICKS(500));   /* let the notice go out over BLE */

    nvs_handle_t nvs;
    if (nvs_open(SETTINGS_NVS_NS, NVS_READWRITE, &nvs) == ESP_OK) {
        nvs_erase_all(nvs);
        nvs_commit(nvs);
        nvs_close(nvs);
    }
    ble_store_clear();
    esp_restart();
}

static void reset_task(void *arg)
{
    uint32_t held_ms = 0;
    bool warned = false;
    for (;;) {
        /* The BOOT button pulls the pin low when pressed. */
        if (gpio_get_level(CONFIG_BRIDGE_RESET_BUTTON_GPIO) == 0) {
            held_ms += POLL_MS;
            if (held_ms >= RESET_AFTER_MS) {
                factory_reset();
            }
            if (!warned && held_ms >= WARN_AFTER_MS) {
                warned = true;
                char text[96];
                snprintf(text, sizeof(text), "Factory reset in %d s. Release the bridge's button to cancel.",
                         (RESET_AFTER_MS - WARN_AFTER_MS) / 1000);
                ESP_LOGW(TAG, "%s", text);
                notice(text);
            }
        } else {
            if (warned) {
                ESP_LOGW(TAG, "Factory reset cancelled");
                notice("Factory reset cancelled.");
            }
            held_ms = 0;
            warned = false;
        }
        vTaskDelay(pdMS_TO_TICKS(POLL_MS));
    }
}

void reset_button_init(void)
{
    gpio_config_t io = {
        .pin_bit_mask = 1ULL << CONFIG_BRIDGE_RESET_BUTTON_GPIO,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_ENABLE,
    };
    gpio_config(&io);
    xTaskCreate(reset_task, "reset", 3072, NULL, 2, NULL);
}
