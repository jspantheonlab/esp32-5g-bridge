#include <stdlib.h>
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/queue.h"
#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "nvs_flash.h"
#include "cJSON.h"
#include "app.h"
#include "audio_bridge.h"
#include "ble_link.h"
#include "hotspot.h"
#include "modem.h"
#include "power.h"
#include "sms.h"
#include "voice.h"

static const char *TAG = "app";

typedef enum { EV_LINE, EV_READY } app_event_type_t;

typedef struct {
    app_event_type_t type;
    char *line;
} app_event_t;

static QueueHandle_t s_events;

void app_emit(cJSON *msg)
{
    char *json = cJSON_PrintUnformatted(msg);
    cJSON_Delete(msg);
    if (!json) {
        return;
    }
    size_t n = strlen(json);
    char *line = realloc(json, n + 2);
    if (!line) {
        free(json);
        return;
    }
    line[n] = '\n';
    line[n + 1] = '\0';
    ble_link_send(line, n + 1);
    free(line);
}

static void emit_status(const modem_status_t *st)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "status");
    cJSON_AddBoolToObject(msg, "sim", st->sim_ready);
    cJSON_AddBoolToObject(msg, "reg", st->registered);
    cJSON_AddBoolToObject(msg, "data", st->data_up);
    cJSON_AddStringToObject(msg, "op", st->operator_name);
    cJSON_AddStringToObject(msg, "rat", modem_rat_name(st->act));
    if (st->rssi_dbm) {
        cJSON_AddNumberToObject(msg, "rssi", st->rssi_dbm);
    }
    cJSON_AddStringToObject(msg, "ip", st->ip);
    char ssid[33];
    hotspot_get_credentials(ssid, sizeof(ssid), NULL, 0);
    cJSON_AddStringToObject(msg, "ssid", ssid);
    power_status_t power;
    power_get_status(&power);
    if (power.present) {
        cJSON_AddNumberToObject(msg, "bat", power.percent);
    }
    if (power.charger) {
        cJSON_AddBoolToObject(msg, "chg", power.charging);
        cJSON_AddBoolToObject(msg, "usb", power.usb);
    }
    app_emit(msg);
}

static void emit_error(const char *context, esp_err_t err)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "err");
    cJSON_AddStringToObject(msg, "ctx", context);
    cJSON_AddStringToObject(msg, "msg", esp_err_to_name(err));
    app_emit(msg);
}

static void emit_current_status(void)
{
    modem_status_t st;
    modem_get_status(&st);
    emit_status(&st);
}

static void emit_wifi(void)
{
    char ssid[33], pass[64];
    hotspot_get_credentials(ssid, sizeof(ssid), pass, sizeof(pass));
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "wifi");
    cJSON_AddStringToObject(msg, "ssid", ssid);
    cJSON_AddStringToObject(msg, "pass", pass);
    app_emit(msg);
}

static const char *str_field(const cJSON *msg, const char *key)
{
    return cJSON_GetStringValue(cJSON_GetObjectItemCaseSensitive(msg, key));
}

static void handle_command(const char *line)
{
    cJSON *msg = cJSON_Parse(line);
    const char *type = msg ? str_field(msg, "t") : NULL;
    if (!type) {
        ESP_LOGW(TAG, "Bad command: %s", line);
        cJSON_Delete(msg);
        return;
    }

    esp_err_t err = ESP_OK;
    if (strcmp(type, "dial") == 0) {
        err = voice_dial(str_field(msg, "num"));
    } else if (strcmp(type, "answer") == 0) {
        err = voice_answer();
    } else if (strcmp(type, "hangup") == 0) {
        err = voice_hangup();
    } else if (strcmp(type, "dtmf") == 0) {
        const char *d = str_field(msg, "d");
        err = d ? voice_dtmf(d[0]) : ESP_ERR_INVALID_ARG;
    } else if (strcmp(type, "sms_send") == 0) {
        sms_request_send(str_field(msg, "ref"), str_field(msg, "to"), str_field(msg, "body"));
    } else if (strcmp(type, "sms_ack") == 0) {
        const cJSON *id = cJSON_GetObjectItemCaseSensitive(msg, "id");
        if (cJSON_IsNumber(id)) {
            sms_request_delete(id->valueint);
        }
    } else if (strcmp(type, "sms_sync") == 0) {
        sms_request_sync();
    } else if (strcmp(type, "wifi_get") == 0) {
        emit_wifi();
    } else if (strcmp(type, "wifi_set") == 0) {
        const char *problem = hotspot_set_credentials(str_field(msg, "ssid"), str_field(msg, "pass"));
        if (problem) {
            cJSON *reply = cJSON_CreateObject();
            cJSON_AddStringToObject(reply, "t", "err");
            cJSON_AddStringToObject(reply, "ctx", "wifi_set");
            cJSON_AddStringToObject(reply, "msg", problem);
            app_emit(reply);
        } else {
            emit_wifi();
            emit_current_status();
        }
    } else if (strcmp(type, "status") == 0) {
        modem_status_t st;
        modem_get_status(&st);
        emit_status(&st);
    } else {
        ESP_LOGW(TAG, "Unknown command: %s", type);
    }

    if (err != ESP_OK) {
        emit_error(type, err);
    }
    cJSON_Delete(msg);
}

/* BLE callbacks run in the NimBLE host task; defer all work to the app task. */
static void on_ble_line(char *line)
{
    app_event_t ev = { .type = EV_LINE, .line = line };
    if (xQueueSend(s_events, &ev, 0) != pdTRUE) {
        free(line);
    }
}

static void on_ble_ready(void)
{
    app_event_t ev = { .type = EV_READY };
    xQueueSend(s_events, &ev, 0);
}

static void app_task(void *arg)
{
    app_event_t ev;
    for (;;) {
        if (xQueueReceive(s_events, &ev, portMAX_DELAY) != pdTRUE) {
            continue;
        }
        if (ev.type == EV_LINE) {
            handle_command(ev.line);
            free(ev.line);
        } else if (ev.type == EV_READY) {
            modem_status_t st;
            modem_get_status(&st);
            emit_status(&st);
            voice_resend_state();
            sms_request_sync();
        }
    }
}

void app_main(void)
{
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    ESP_ERROR_CHECK(err);
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());

    s_events = xQueueCreate(16, sizeof(app_event_t));
    xTaskCreate(app_task, "app", 6144, NULL, 5, NULL);

    ESP_ERROR_CHECK(hotspot_init());
    ESP_ERROR_CHECK(audio_bridge_init());

    const ble_link_callbacks_t ble_cbs = { .on_line = on_ble_line, .on_ready = on_ble_ready };
    ESP_ERROR_CHECK(ble_link_init(&ble_cbs));

    modem_set_status_handler(emit_status);
    ESP_ERROR_CHECK(modem_init());
    sms_init();
    voice_init();
    power_init(emit_current_status);

    ESP_LOGI(TAG, "5G bridge started");
}
