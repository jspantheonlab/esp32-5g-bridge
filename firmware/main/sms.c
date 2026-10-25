#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/queue.h"
#include "esp_log.h"
#include "app.h"
#include "modem.h"
#include "ucs2.h"
#include "sms.h"

#define MAX_GSM_CHARS  160
#define MAX_UCS2_UNITS 70
#define MAX_NUMBER_LEN 20

static const char *TAG = "sms";

typedef enum { JOB_READ, JOB_SYNC, JOB_DELETE, JOB_SEND } sms_job_kind_t;

typedef struct {
    sms_job_kind_t kind;
    int index;
    char *ref;
    char *to;
    char *body;
} sms_job_t;

static QueueHandle_t s_queue;

static void post(sms_job_t job)
{
    if (xQueueSend(s_queue, &job, pdMS_TO_TICKS(100)) != pdTRUE) {
        ESP_LOGW(TAG, "Job queue full, dropping job %d", job.kind);
        free(job.ref);
        free(job.to);
        free(job.body);
    }
}

void sms_request_sync(void)        { post((sms_job_t){ .kind = JOB_SYNC }); }
void sms_request_delete(int index) { post((sms_job_t){ .kind = JOB_DELETE, .index = index }); }

void sms_request_send(const char *ref, const char *to, const char *body)
{
    post((sms_job_t){
        .kind = JOB_SEND,
        .ref = strdup(ref ? ref : ""),
        .to = strdup(to ? to : ""),
        .body = strdup(body ? body : ""),
    });
}

void sms_on_urc(const char *line)
{
    /* +CMTI: "SM",3 */
    if (strncmp(line, "+CMTI:", 6) == 0) {
        const char *comma = strrchr(line, ',');
        if (comma) {
            post((sms_job_t){ .kind = JOB_READ, .index = atoi(comma + 1) });
        }
    }
}

/* Decode a field read while CSCS="UCS2"; fall back to the raw text if it isn't hex. */
static void decode_field(const char *in, char *out, size_t out_len)
{
    if (!ucs2_hex_to_utf8(in, out, out_len)) {
        strlcpy(out, in, out_len);
    }
}

static bool valid_number(const char *num)
{
    size_t n = strlen(num);
    if (n == 0 || n > MAX_NUMBER_LEN) {
        return false;
    }
    for (size_t i = 0; i < n; i++) {
        if (!((num[i] >= '0' && num[i] <= '9') || (i == 0 && num[i] == '+'))) {
            return false;
        }
    }
    return true;
}

/* Reads one stored message and forwards it to the phone. Returns true if a received SMS was found. */
static bool read_index(int index)
{
    static char out[1024];
    static char text[1024];
    char cmd[24];
    snprintf(cmd, sizeof(cmd), "AT+CMGR=%d", index);

    modem_lock();
    modem_at("AT+CSCS=\"UCS2\"", NULL, 0, 1000);
    esp_err_t err = modem_at(cmd, out, sizeof(out), 5000);
    modem_at("AT+CSCS=\"IRA\"", NULL, 0, 1000);
    modem_unlock();

    char *hdr;
    if (err != ESP_OK || !(hdr = strstr(out, "+CMGR:"))) {
        return false;
    }
    char *eol = strpbrk(hdr, "\r\n");
    if (!eol) {
        return false;
    }

    /* +CMGR: "REC UNREAD","<oa>",[<alpha>],"<scts>" */
    char fields[5][96];
    int nf = 0;
    for (char *p = hdr; nf < 5 && (p = strchr(p, '"')) && p < eol;) {
        char *end = strchr(p + 1, '"');
        if (!end || end > eol) {
            break;
        }
        size_t len = end - p - 1;
        if (len >= sizeof(fields[0])) {
            len = sizeof(fields[0]) - 1;
        }
        memcpy(fields[nf], p + 1, len);
        fields[nf][len] = '\0';
        nf++;
        p = end + 1;
    }
    if (nf < 2) {
        return false;
    }

    char stat[24], from[48], ts[48];
    decode_field(fields[0], stat, sizeof(stat));
    if (strncmp(stat, "REC", 3) != 0) {
        return false;   /* stored outgoing draft, not an inbound message */
    }
    decode_field(fields[1], from, sizeof(from));
    decode_field(nf >= 3 ? fields[nf - 1] : "", ts, sizeof(ts));

    char *body = eol;
    while (*body == '\r' || *body == '\n') {
        body++;
    }
    char *body_end = strstr(body, "\r\n");
    if (body_end) {
        *body_end = '\0';
    }
    decode_field(body, text, sizeof(text));

    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "sms");
    cJSON_AddNumberToObject(msg, "id", index);
    cJSON_AddStringToObject(msg, "from", from);
    cJSON_AddStringToObject(msg, "ts", ts);
    cJSON_AddStringToObject(msg, "body", text);
    app_emit(msg);
    return true;
}

static void sync_all(void)
{
    char out[160];
    char *p;
    int used = 0, total = 0;
    if (modem_at("AT+CPMS?", out, sizeof(out), 2000) != ESP_OK || !(p = strstr(out, "+CPMS:")) ||
        sscanf(p, "+CPMS: \"%*[^\"]\",%d,%d", &used, &total) != 2) {
        ESP_LOGW(TAG, "Could not read SMS storage");
        return;
    }
    ESP_LOGI(TAG, "SIM storage: %d/%d", used, total);
    for (int i = 0, found = 0; i <= total && found < used; i++) {
        if (read_index(i)) {
            found++;
        }
    }
}

static void emit_send_result(const char *ref, bool ok, const char *error)
{
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "sms_sent");
    cJSON_AddStringToObject(msg, "ref", ref);
    cJSON_AddBoolToObject(msg, "ok", ok);
    if (error) {
        cJSON_AddStringToObject(msg, "err", error);
    }
    app_emit(msg);
}

static void send_sms(const char *ref, const char *to, const char *body)
{
    static char hex_to[MAX_NUMBER_LEN * 4 + 1];
    static char payload[MAX_UCS2_UNITS * 4 * 2 + 2];
    char cmd[128];
    char out[128];

    if (!valid_number(to)) {
        emit_send_result(ref, false, "invalid number");
        return;
    }

    bool gsm = sms_is_gsm_safe(body);
    if (gsm) {
        if (strlen(body) > MAX_GSM_CHARS) {
            emit_send_result(ref, false, "message longer than 160 characters");
            return;
        }
        snprintf(cmd, sizeof(cmd), "AT+CMGS=\"%s\"\r", to);
        snprintf(payload, sizeof(payload), "%s\x1A", body);
    } else {
        int units = utf8_to_ucs2_hex(body, payload, sizeof(payload) - 1);
        if (units < 0 || units > MAX_UCS2_UNITS) {
            emit_send_result(ref, false, "unicode message longer than 70 characters");
            return;
        }
        strcat(payload, "\x1A");
        utf8_to_ucs2_hex(to, hex_to, sizeof(hex_to));
        snprintf(cmd, sizeof(cmd), "AT+CMGS=\"%s\"\r", hex_to);
    }

    modem_lock();
    modem_at(gsm ? "AT+CSCS=\"IRA\"" : "AT+CSCS=\"UCS2\"", NULL, 0, 1000);
    modem_at(gsm ? "AT+CSMP=17,167,0,0" : "AT+CSMP=17,167,0,8", NULL, 0, 1000);

    esp_err_t err = modem_at_raw(cmd, ">", "ERROR", NULL, 0, 5000);
    if (err == ESP_OK) {
        err = modem_at_raw(payload, "+CMGS:", "ERROR", out, sizeof(out), 60000);
    } else {
        modem_at_raw("\x1B", "OK", "ERROR", NULL, 0, 1000);
    }

    modem_at("AT+CSCS=\"IRA\"", NULL, 0, 1000);
    modem_at("AT+CSMP=17,167,0,0", NULL, 0, 1000);
    modem_unlock();

    ESP_LOGI(TAG, "SMS to %s: %s", to, err == ESP_OK ? "sent" : "failed");
    emit_send_result(ref, err == ESP_OK, err == ESP_OK ? NULL : "modem rejected message");
}

static void sms_task(void *arg)
{
    sms_job_t job;
    for (;;) {
        if (xQueueReceive(s_queue, &job, portMAX_DELAY) != pdTRUE) {
            continue;
        }
        switch (job.kind) {
        case JOB_READ:
            read_index(job.index);
            break;
        case JOB_SYNC:
            sync_all();
            break;
        case JOB_DELETE: {
            char cmd[24];
            snprintf(cmd, sizeof(cmd), "AT+CMGD=%d", job.index);
            modem_at(cmd, NULL, 0, 5000);
            break;
        }
        case JOB_SEND:
            if (job.ref && job.to && job.body) {
                send_sms(job.ref, job.to, job.body);
            }
            break;
        }
        free(job.ref);
        free(job.to);
        free(job.body);
    }
}

void sms_init(void)
{
    s_queue = xQueueCreate(16, sizeof(sms_job_t));
    xTaskCreate(sms_task, "sms", 6144, NULL, 4, NULL);
    modem_add_urc_handler(sms_on_urc);
}
