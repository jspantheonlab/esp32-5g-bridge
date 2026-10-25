#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_log.h"
#include "app.h"
#include "audio_bridge.h"
#include "modem.h"
#include "voice.h"

#define POLL_INTERVAL_MS 500

static const char *TAG = "voice";

/* Call state is owned by the voice task; everything else only kicks it. */
static TaskHandle_t s_task;
static call_state_t s_state = CALL_IDLE;
static char s_number[32];
static bool s_outgoing;
static atomic_bool s_resend;

static const char *state_name(call_state_t s)
{
    switch (s) {
    case CALL_INCOMING: return "incoming";
    case CALL_DIALING:  return "dialing";
    case CALL_ALERTING: return "alerting";
    case CALL_ACTIVE:   return "active";
    case CALL_HELD:     return "held";
    case CALL_WAITING:  return "waiting";
    default:            return "idle";
    }
}

static void emit_state(void)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "call");
    cJSON_AddStringToObject(msg, "state", state_name(s_state));
    cJSON_AddStringToObject(msg, "num", s_number);
    cJSON_AddStringToObject(msg, "dir", s_outgoing ? "out" : "in");
    app_emit(msg);
}

static void set_state(call_state_t state, const char *number, bool outgoing)
{
    if (state == s_state && (!number || strcmp(number, s_number) == 0)) {
        return;
    }
    ESP_LOGI(TAG, "%s -> %s", state_name(s_state), state_name(state));
    s_state = state;
    s_outgoing = outgoing;
    if (number) {
        strlcpy(s_number, number, sizeof(s_number));
    }

    if (state == CALL_ACTIVE) {
        audio_bridge_start();
    } else if (state == CALL_IDLE) {
        audio_bridge_stop();
    }
    emit_state();
    if (state == CALL_IDLE) {
        s_number[0] = '\0';
    }
}

static call_state_t map_clcc_stat(int stat)
{
    switch (stat) {
    case 0: return CALL_ACTIVE;
    case 1: return CALL_HELD;
    case 2: return CALL_DIALING;
    case 3: return CALL_ALERTING;
    case 4: return CALL_INCOMING;
    case 5: return CALL_WAITING;
    default: return CALL_IDLE;
    }
}

static int state_rank(call_state_t s)
{
    switch (s) {
    case CALL_ACTIVE: return 3;
    case CALL_INCOMING: case CALL_DIALING: case CALL_ALERTING: return 2;
    case CALL_HELD: case CALL_WAITING: return 1;
    default: return 0;
    }
}

/* +CLCC: <id>,<dir>,<stat>,<mode>,<mpty>[,"<number>",<type>] */
static void poll_clcc(void)
{
    char out[512];
    if (modem_at("AT+CLCC", out, sizeof(out), 2000) != ESP_OK) {
        return;
    }

    call_state_t best = CALL_IDLE;
    char number[32] = "";
    bool outgoing = false;

    for (char *p = out; (p = strstr(p, "+CLCC:")) != NULL; p += 6) {
        int id, dir, stat, mode, mpty;
        if (sscanf(p, "+CLCC: %d,%d,%d,%d,%d", &id, &dir, &stat, &mode, &mpty) != 5 || mode != 0) {
            continue;
        }
        call_state_t st = map_clcc_stat(stat);
        if (state_rank(st) <= state_rank(best)) {
            continue;
        }
        best = st;
        outgoing = dir == 0;
        number[0] = '\0';
        char *eol = strpbrk(p, "\r\n");
        char *q1 = strchr(p, '"');
        char *q2 = q1 ? strchr(q1 + 1, '"') : NULL;
        if (q1 && q2 && (!eol || q2 < eol)) {
            size_t n = q2 - q1 - 1;
            if (n >= sizeof(number)) {
                n = sizeof(number) - 1;
            }
            memcpy(number, q1 + 1, n);
            number[n] = '\0';
        }
    }
    set_state(best, best == CALL_IDLE ? NULL : number, outgoing);
}

static void voice_task(void *arg)
{
    for (;;) {
        TickType_t wait = s_state == CALL_IDLE ? portMAX_DELAY : pdMS_TO_TICKS(POLL_INTERVAL_MS);
        ulTaskNotifyTake(pdTRUE, wait);
        poll_clcc();
        if (atomic_exchange(&s_resend, false)) {
            emit_state();
        }
    }
}

static void kick(void)
{
    if (s_task) {
        xTaskNotifyGive(s_task);
    }
}

void voice_on_urc(const char *line)
{
    static const char *const triggers[] = {
        "RING", "+CLIP:", "+CRING:", "NO CARRIER", "BUSY", "NO ANSWER", "^DSCI:", "+QIND: \"ccinfo\"",
    };
    for (size_t i = 0; i < sizeof(triggers) / sizeof(triggers[0]); i++) {
        if (strncmp(line, triggers[i], strlen(triggers[i])) == 0) {
            kick();
            return;
        }
    }
}

void voice_resend_state(void)
{
    atomic_store(&s_resend, true);
    kick();
}

esp_err_t voice_dial(const char *number)
{
    size_t n = number ? strlen(number) : 0;
    if (n == 0 || n > 20) {
        return ESP_ERR_INVALID_ARG;
    }
    for (size_t i = 0; i < n; i++) {
        if (!strchr("0123456789*#+", number[i])) {
            return ESP_ERR_INVALID_ARG;
        }
    }
    char cmd[32];
    snprintf(cmd, sizeof(cmd), "ATD%s;", number);
    esp_err_t err = modem_at(cmd, NULL, 0, 15000);
    kick();
    return err;
}

esp_err_t voice_answer(void)
{
    esp_err_t err = modem_at("ATA", NULL, 0, 10000);
    kick();
    return err;
}

esp_err_t voice_hangup(void)
{
    esp_err_t err = modem_at("AT+CHUP", NULL, 0, 5000);
    if (err != ESP_OK) {
        err = modem_at("ATH", NULL, 0, 5000);
    }
    kick();
    return err;
}

esp_err_t voice_dtmf(char digit)
{
    if (!strchr("0123456789*#ABCD", digit)) {
        return ESP_ERR_INVALID_ARG;
    }
    char cmd[16];
    snprintf(cmd, sizeof(cmd), "AT+VTS=%c", digit);
    return modem_at(cmd, NULL, 0, 3000);
}

void voice_init(void)
{
    xTaskCreate(voice_task, "voice", 4096, NULL, 5, &s_task);
    modem_add_urc_handler(voice_on_urc);
}
