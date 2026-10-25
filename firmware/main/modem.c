#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/semphr.h"
#include "freertos/queue.h"
#include "freertos/event_groups.h"
#include "esp_log.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "esp_netif_ppp.h"
#include "driver/gpio.h"
#include "driver/uart.h"
#include "esp_modem_api.h"
#include "hotspot.h"
#include "modem.h"

#define PPP_UP_BIT   BIT0
#define PPP_DOWN_BIT BIT1
#define MAX_URC_HANDLERS 4

#ifdef CONFIG_ESP_MODEM_C_API_STR_MAX
#define RESP_MAX CONFIG_ESP_MODEM_C_API_STR_MAX
#else
#define RESP_MAX 1024
#endif

static const char *TAG = "modem";

static esp_modem_dce_t *s_dce;
static esp_netif_t *s_ppp;
static SemaphoreHandle_t s_lock;
static EventGroupHandle_t s_events;
static QueueHandle_t s_urc_queue;
static modem_urc_cb_t s_urc_handlers[MAX_URC_HANDLERS];
static int s_urc_handler_count;
static modem_status_cb_t s_status_cb;
static modem_status_t s_status = { .act = -1 };
static portMUX_TYPE s_status_mux = portMUX_INITIALIZER_UNLOCKED;
static char s_resp[RESP_MAX];

/* ---------- AT channel ---------- */

void modem_lock(void)   { xSemaphoreTakeRecursive(s_lock, portMAX_DELAY); }
void modem_unlock(void) { xSemaphoreGiveRecursive(s_lock); }

esp_err_t modem_at_raw(const char *raw, const char *pass, const char *fail,
                       char *out, size_t out_len, uint32_t timeout_ms)
{
    if (!s_dce) {
        return ESP_ERR_INVALID_STATE;
    }
    modem_lock();
    s_resp[0] = '\0';
    esp_err_t err = esp_modem_at_raw(s_dce, raw, s_resp, pass, fail, timeout_ms);
    if (out && out_len) {
        strlcpy(out, s_resp, out_len);
    }
    modem_unlock();
    return err;
}

esp_err_t modem_at(const char *cmd, char *out, size_t out_len, uint32_t timeout_ms)
{
    char line[256];
    int n = snprintf(line, sizeof(line), "%s\r", cmd);
    if (n <= 0 || n >= (int)sizeof(line)) {
        return ESP_ERR_INVALID_SIZE;
    }
    return modem_at_raw(line, "OK", "ERROR", out, out_len, timeout_ms);
}

/* ---------- URCs ---------- */

static char s_urc_line[256];
static size_t s_urc_len;
static size_t s_seen_len;
static uint64_t s_seen_hash;

static uint64_t fnv1a(const uint8_t *data, size_t len)
{
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < len; i++) {
        h = (h ^ data[i]) * 0x100000001b3ULL;
    }
    return h;
}

/*
 * Runs in the esp_modem DTE task: only assemble lines and hand them off.
 * esp_modem passes its whole unconsumed buffer on every call (in UART mode the same bytes
 * repeat until a command completes), so skip a prefix identical to what was already seen.
 * Same logic as BufferDelta in firmware-rs/bridge-core/src/framing.rs.
 */
static esp_err_t urc_rx(uint8_t *data, size_t len)
{
    size_t start = 0;
    if (s_seen_len > 0 && len >= s_seen_len && fnv1a(data, s_seen_len) == s_seen_hash) {
        start = s_seen_len;
    }
    s_seen_len = len;
    s_seen_hash = fnv1a(data, len);

    for (size_t i = start; i < len; i++) {
        char c = (char)data[i];
        if (c == '\r' || c == '\n') {
            if (s_urc_len) {
                s_urc_line[s_urc_len] = '\0';
                char *line = strdup(s_urc_line);
                if (line && xQueueSend(s_urc_queue, &line, 0) != pdTRUE) {
                    free(line);
                }
                s_urc_len = 0;
            }
        } else if (s_urc_len < sizeof(s_urc_line) - 1) {
            s_urc_line[s_urc_len++] = c;
        }
    }
    return ESP_OK;
}

static void urc_task(void *arg)
{
    char *line;
    for (;;) {
        if (xQueueReceive(s_urc_queue, &line, portMAX_DELAY) == pdTRUE) {
            ESP_LOGD(TAG, "URC: %s", line);
            for (int i = 0; i < s_urc_handler_count; i++) {
                s_urc_handlers[i](line);
            }
            free(line);
        }
    }
}

void modem_add_urc_handler(modem_urc_cb_t cb)
{
    if (s_urc_handler_count < MAX_URC_HANDLERS) {
        s_urc_handlers[s_urc_handler_count++] = cb;
    }
}

/* ---------- status ---------- */

void modem_set_status_handler(modem_status_cb_t cb) { s_status_cb = cb; }

void modem_get_status(modem_status_t *out)
{
    taskENTER_CRITICAL(&s_status_mux);
    *out = s_status;
    taskEXIT_CRITICAL(&s_status_mux);
}

const char *modem_rat_name(int act)
{
    switch (act) {
    case 0: case 1: case 3: return "2G";
    case 2: case 4: case 5: case 6: return "3G";
    case 7: case 9: return "LTE";
    case 10: return "LTE (5GC)";
    case 11: case 12: return "5G SA";
    case 13: return "5G NSA";
    default: return "—";
    }
}

static void refresh_status(void)
{
    char out[160];
    modem_status_t st;
    modem_get_status(&st);

    st.sim_ready = modem_at("AT+CPIN?", out, sizeof(out), 2000) == ESP_OK && strstr(out, "READY");

    int rssi = 99, ber = 99;
    char *p;
    if (modem_at("AT+CSQ", out, sizeof(out), 2000) == ESP_OK && (p = strstr(out, "+CSQ:"))) {
        sscanf(p, "+CSQ: %d,%d", &rssi, &ber);
    }
    st.rssi_dbm = (rssi >= 0 && rssi <= 31) ? -113 + 2 * rssi : 0;

    st.operator_name[0] = '\0';
    st.act = -1;
    if (modem_at("AT+COPS?", out, sizeof(out), 3000) == ESP_OK && (p = strstr(out, "+COPS:"))) {
        char *q1 = strchr(p, '"');
        char *q2 = q1 ? strchr(q1 + 1, '"') : NULL;
        if (q1 && q2) {
            size_t n = q2 - q1 - 1;
            if (n >= sizeof(st.operator_name)) {
                n = sizeof(st.operator_name) - 1;
            }
            memcpy(st.operator_name, q1 + 1, n);
            st.operator_name[n] = '\0';
            sscanf(q2 + 1, ",%d", &st.act);
        }
    }
    st.registered = st.operator_name[0] != '\0';

    taskENTER_CRITICAL(&s_status_mux);
    /* data_up and ip are owned by the PPP event handler */
    st.data_up = s_status.data_up;
    memcpy(st.ip, s_status.ip, sizeof(st.ip));
    s_status = st;
    taskEXIT_CRITICAL(&s_status_mux);

    if (s_status_cb) {
        s_status_cb(&st);
    }
}

static void on_ip_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    if (id == IP_EVENT_PPP_GOT_IP) {
        ip_event_got_ip_t *e = data;
        if (e->esp_netif != s_ppp) {
            return;
        }
        ESP_LOGI(TAG, "PPP up, IP " IPSTR, IP2STR(&e->ip_info.ip));
        taskENTER_CRITICAL(&s_status_mux);
        s_status.data_up = true;
        snprintf(s_status.ip, sizeof(s_status.ip), IPSTR, IP2STR(&e->ip_info.ip));
        taskEXIT_CRITICAL(&s_status_mux);
        hotspot_set_uplink(s_ppp);
        xEventGroupSetBits(s_events, PPP_UP_BIT);
    } else if (id == IP_EVENT_PPP_LOST_IP) {
        ESP_LOGW(TAG, "PPP lost IP");
        taskENTER_CRITICAL(&s_status_mux);
        s_status.data_up = false;
        s_status.ip[0] = '\0';
        taskEXIT_CRITICAL(&s_status_mux);
        hotspot_clear_uplink();
        xEventGroupSetBits(s_events, PPP_DOWN_BIT);
    }
}

/* ---------- bring-up ---------- */

static void pwrkey_pulse(uint32_t ms)
{
#if CONFIG_BRIDGE_MODEM_PWRKEY_GPIO >= 0
#ifdef CONFIG_BRIDGE_MODEM_PWRKEY_ACTIVE_HIGH
    const int on = 1;
#else
    const int on = 0;
#endif
    ESP_LOGI(TAG, "Pulsing PWRKEY");
    gpio_set_level(CONFIG_BRIDGE_MODEM_PWRKEY_GPIO, on);
    vTaskDelay(pdMS_TO_TICKS(ms));
    gpio_set_level(CONFIG_BRIDGE_MODEM_PWRKEY_GPIO, !on);
#endif
}

static bool sync_at(int baud, int attempts)
{
    uart_set_baudrate(CONFIG_BRIDGE_MODEM_UART_PORT, baud);
    for (int i = 0; i < attempts; i++) {
        if (esp_modem_sync(s_dce) == ESP_OK) {
            return true;
        }
        vTaskDelay(pdMS_TO_TICKS(500));
    }
    return false;
}

/* Returns the baud rate the modem answered on, or 0. */
static int find_modem(int attempts)
{
    if (sync_at(CONFIG_BRIDGE_MODEM_BAUD, attempts)) {
        return CONFIG_BRIDGE_MODEM_BAUD;
    }
    if (CONFIG_BRIDGE_MODEM_BAUD != 115200 && sync_at(115200, attempts)) {
        return 115200;
    }
    return 0;
}

static bool power_and_sync(void)
{
    int baud = find_modem(2);
    if (!baud) {
        pwrkey_pulse(600);
        for (int i = 0; i < 10 && !baud; i++) {
            baud = find_modem(3);
        }
    }
    if (!baud) {
        ESP_LOGE(TAG, "Modem not responding");
        return false;
    }
    if (baud != CONFIG_BRIDGE_MODEM_BAUD) {
        char cmd[32];
        snprintf(cmd, sizeof(cmd), "AT+IPR=%d", CONFIG_BRIDGE_MODEM_BAUD);
        modem_at(cmd, NULL, 0, 1000);
        vTaskDelay(pdMS_TO_TICKS(100));
        if (!sync_at(CONFIG_BRIDGE_MODEM_BAUD, 6)) {
            ESP_LOGE(TAG, "Modem lost after baud switch");
            return false;
        }
        modem_at("AT&W", NULL, 0, 1000);
    }
    ESP_LOGI(TAG, "Modem responding at %d baud", CONFIG_BRIDGE_MODEM_BAUD);
    return true;
}

static bool wait_sim_ready(void)
{
    char out[96];
    for (int i = 0; i < 20; i++) {
        if (modem_at("AT+CPIN?", out, sizeof(out), 2000) == ESP_OK) {
            if (strstr(out, "READY")) {
                return true;
            }
            if (strstr(out, "SIM PIN") && strlen(CONFIG_BRIDGE_SIM_PIN) > 0) {
                char cmd[40];
                snprintf(cmd, sizeof(cmd), "AT+CPIN=\"%s\"", CONFIG_BRIDGE_SIM_PIN);
                modem_at(cmd, NULL, 0, 5000);
            } else if (strstr(out, "PUK")) {
                ESP_LOGE(TAG, "SIM is PUK-locked");
                return false;
            }
        }
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return false;
}

static bool wait_registered(int timeout_s)
{
    static const char *const queries[][2] = {
        { "AT+C5GREG?", "+C5GREG:" },
        { "AT+CEREG?", "+CEREG:" },
    };
    char out[128];
    for (int t = 0; t < timeout_s; t += 2) {
        for (size_t i = 0; i < sizeof(queries) / sizeof(queries[0]); i++) {
            char *p;
            int n, stat;
            if (modem_at(queries[i][0], out, sizeof(out), 2000) == ESP_OK &&
                (p = strstr(out, queries[i][1])) &&
                sscanf(p + strlen(queries[i][1]), " %d,%d", &n, &stat) == 2 &&
                (stat == 1 || stat == 5)) {
                ESP_LOGI(TAG, "Registered (%s stat=%d)", queries[i][1], stat);
                return true;
            }
        }
        vTaskDelay(pdMS_TO_TICKS(2000));
    }
    return false;
}

/* Carrier APNs by IMSI prefix (MCC+MNC). Same table as firmware-rs/bridge-core/src/carrier.rs. */
static const struct {
    const char *prefix;
    const char *name;
    const char *apn;
} s_carriers[] = {
    { "46000", "China Mobile", "cmnet" },  { "46002", "China Mobile", "cmnet" },
    { "46004", "China Mobile", "cmnet" },  { "46007", "China Mobile", "cmnet" },
    { "46008", "China Mobile", "cmnet" },  { "46001", "China Unicom", "3gnet" },
    { "46006", "China Unicom", "3gnet" },  { "46009", "China Unicom", "3gnet" },
    { "46003", "China Telecom", "ctnet" }, { "46005", "China Telecom", "ctnet" },
    { "46011", "China Telecom", "ctnet" }, { "46015", "China Broadnet", "cbnet" },
};

/* The Kconfig APN wins if it was changed from the default; otherwise use the SIM's carrier APN. */
static void select_apn(void)
{
    static char apn[64];
    strlcpy(apn, CONFIG_BRIDGE_MODEM_APN, sizeof(apn));

    char out[64];
    if (strcmp(CONFIG_BRIDGE_MODEM_APN, "internet") == 0 && modem_at("AT+CIMI", out, sizeof(out), 2000) == ESP_OK) {
        const char *imsi = out + strspn(out, "\r\n ");
        for (size_t i = 0; i < sizeof(s_carriers) / sizeof(s_carriers[0]); i++) {
            if (strncmp(imsi, s_carriers[i].prefix, strlen(s_carriers[i].prefix)) == 0) {
                strlcpy(apn, s_carriers[i].apn, sizeof(apn));
                ESP_LOGI(TAG, "SIM carrier: %s", s_carriers[i].name);
                break;
            }
        }
    }
    ESP_LOGI(TAG, "Using APN \"%s\"", apn);
    esp_modem_set_apn(s_dce, apn);
}

static esp_err_t bring_up(void)
{
    if (!power_and_sync()) {
        return ESP_FAIL;
    }

    modem_at("ATE0", NULL, 0, 1000);
    modem_at("AT+CMEE=2", NULL, 0, 1000);
    modem_at("AT+CSCS=\"IRA\"", NULL, 0, 1000);
    modem_at("AT+COPS=3,0", NULL, 0, 1000);

    if (!wait_sim_ready()) {
        ESP_LOGE(TAG, "SIM not ready");
        return ESP_ERR_INVALID_STATE;
    }

    select_apn();
    /* Let the modem pick the carrier profile (MBN) that matches the SIM, e.g. for VoLTE. */
    modem_at("AT+QMBNCFG=\"AutoSel\",1", NULL, 0, 2000);
#ifdef CONFIG_BRIDGE_ENABLE_IMS
    modem_at("AT+QCFG=\"ims\",1", NULL, 0, 2000);
#endif
    if (strlen(CONFIG_BRIDGE_PCM_DAI_CMD) > 0) {
        modem_at(CONFIG_BRIDGE_PCM_DAI_CMD, NULL, 0, 2000);
    }
    modem_at("AT+CLIP=1", NULL, 0, 1000);
    modem_at("AT+CMGF=1", NULL, 0, 1000);
    modem_at("AT+CNMI=2,1,0,0,0", NULL, 0, 1000);

    if (!wait_registered(180)) {
        ESP_LOGE(TAG, "Network registration timed out");
        return ESP_ERR_TIMEOUT;
    }
    refresh_status();

    /* CMUX: one virtual channel carries PPP, another keeps AT available for calls/SMS. */
    xEventGroupClearBits(s_events, PPP_UP_BIT | PPP_DOWN_BIT);
    if (esp_modem_set_mode(s_dce, ESP_MODEM_MODE_CMUX) != ESP_OK) {
        ESP_LOGE(TAG, "Failed to enter CMUX/PPP mode");
        return ESP_FAIL;
    }
    EventBits_t bits = xEventGroupWaitBits(s_events, PPP_UP_BIT, pdFALSE, pdFALSE, pdMS_TO_TICKS(30000));
    if (!(bits & PPP_UP_BIT)) {
        ESP_LOGE(TAG, "PPP did not come up");
        return ESP_ERR_TIMEOUT;
    }
    return ESP_OK;
}

static void modem_task(void *arg)
{
    for (;;) {
        if (bring_up() != ESP_OK) {
            esp_modem_set_mode(s_dce, ESP_MODEM_MODE_COMMAND);
            vTaskDelay(pdMS_TO_TICKS(10000));
            continue;
        }
        refresh_status();
        for (;;) {
            EventBits_t bits = xEventGroupWaitBits(s_events, PPP_DOWN_BIT, pdTRUE, pdFALSE, pdMS_TO_TICKS(10000));
            if (bits & PPP_DOWN_BIT) {
                break;
            }
            refresh_status();
        }
        ESP_LOGW(TAG, "Data session lost, restarting");
        esp_modem_set_mode(s_dce, ESP_MODEM_MODE_COMMAND);
        vTaskDelay(pdMS_TO_TICKS(2000));
    }
}

esp_err_t modem_init(void)
{
    s_lock = xSemaphoreCreateRecursiveMutex();
    s_events = xEventGroupCreate();
    s_urc_queue = xQueueCreate(16, sizeof(char *));

#if CONFIG_BRIDGE_MODEM_PWRKEY_GPIO >= 0
    gpio_config_t io = {
        .pin_bit_mask = 1ULL << CONFIG_BRIDGE_MODEM_PWRKEY_GPIO,
        .mode = GPIO_MODE_OUTPUT,
    };
    gpio_config(&io);
#ifdef CONFIG_BRIDGE_MODEM_PWRKEY_ACTIVE_HIGH
    gpio_set_level(CONFIG_BRIDGE_MODEM_PWRKEY_GPIO, 0);
#else
    gpio_set_level(CONFIG_BRIDGE_MODEM_PWRKEY_GPIO, 1);
#endif
#endif

    ESP_ERROR_CHECK(esp_event_handler_register(IP_EVENT, ESP_EVENT_ANY_ID, on_ip_event, NULL));

    esp_netif_config_t ppp_cfg = ESP_NETIF_DEFAULT_PPP();
    s_ppp = esp_netif_new(&ppp_cfg);

    esp_modem_dte_config_t dte_config = ESP_MODEM_DTE_DEFAULT_CONFIG();
    dte_config.uart_config.port_num = CONFIG_BRIDGE_MODEM_UART_PORT;
    dte_config.uart_config.tx_io_num = CONFIG_BRIDGE_MODEM_TX_GPIO;
    dte_config.uart_config.rx_io_num = CONFIG_BRIDGE_MODEM_RX_GPIO;
    dte_config.uart_config.rts_io_num = CONFIG_BRIDGE_MODEM_RTS_GPIO;
    dte_config.uart_config.cts_io_num = CONFIG_BRIDGE_MODEM_CTS_GPIO;
    dte_config.uart_config.flow_control = ESP_MODEM_FLOW_CONTROL_HW;
    dte_config.uart_config.baud_rate = CONFIG_BRIDGE_MODEM_BAUD;
    dte_config.uart_config.rx_buffer_size = 16384;
    dte_config.uart_config.tx_buffer_size = 4096;
    dte_config.uart_config.event_queue_size = 32;
    dte_config.task_stack_size = 6144;
    dte_config.task_priority = 6;
    dte_config.dte_buffer_size = 8192;

    esp_modem_dce_config_t dce_config = ESP_MODEM_DCE_DEFAULT_CONFIG(CONFIG_BRIDGE_MODEM_APN);
    s_dce = esp_modem_new_dev(ESP_MODEM_DCE_GENERIC, &dte_config, &dce_config, s_ppp);
    if (!s_dce) {
        ESP_LOGE(TAG, "esp_modem_new_dev failed");
        return ESP_FAIL;
    }
    esp_modem_set_urc(s_dce, urc_rx);

    xTaskCreate(urc_task, "modem_urc", 4096, NULL, 6, NULL);
    xTaskCreate(modem_task, "modem", 6144, NULL, 5, NULL);
    return ESP_OK;
}
