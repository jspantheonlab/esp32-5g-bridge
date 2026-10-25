#include <stdio.h>
#include <string.h>
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_netif_ppp.h"
#include "driver/uart.h"
#include "esp_modem_api.h"
#include "modem_shim.h"

#ifdef CONFIG_ESP_MODEM_C_API_STR_MAX
#define RESP_MAX CONFIG_ESP_MODEM_C_API_STR_MAX
#else
#define RESP_MAX 128
#endif

static const char *TAG = "modem_shim";

static esp_modem_dce_t *s_dce;
static esp_netif_t *s_ppp;
static int s_uart_port;
static char s_apn[64];
static modem_shim_rx_cb_t s_rx_cb;

esp_err_t modem_shim_init(const modem_shim_config_t *config)
{
    if (s_dce) {
        return ESP_ERR_INVALID_STATE;
    }
    strlcpy(s_apn, config->apn ? config->apn : "", sizeof(s_apn));
    s_uart_port = config->uart_port;

    esp_netif_config_t ppp_cfg = ESP_NETIF_DEFAULT_PPP();
    s_ppp = esp_netif_new(&ppp_cfg);
    if (!s_ppp) {
        return ESP_FAIL;
    }

    esp_modem_dte_config_t dte = ESP_MODEM_DTE_DEFAULT_CONFIG();
    dte.uart_config.port_num = config->uart_port;
    dte.uart_config.tx_io_num = config->tx_gpio;
    dte.uart_config.rx_io_num = config->rx_gpio;
    dte.uart_config.rts_io_num = config->rts_gpio;
    dte.uart_config.cts_io_num = config->cts_gpio;
    dte.uart_config.flow_control = ESP_MODEM_FLOW_CONTROL_HW;
    dte.uart_config.baud_rate = config->baud_rate;
    dte.uart_config.rx_buffer_size = 16384;
    dte.uart_config.tx_buffer_size = 4096;
    dte.uart_config.event_queue_size = 32;
    dte.task_stack_size = 6144;
    dte.task_priority = 6;
    dte.dte_buffer_size = 8192;

    esp_modem_dce_config_t dce = ESP_MODEM_DCE_DEFAULT_CONFIG(s_apn);
    s_dce = esp_modem_new_dev(ESP_MODEM_DCE_GENERIC, &dte, &dce, s_ppp);
    if (!s_dce) {
        ESP_LOGE(TAG, "esp_modem_new_dev failed");
        return ESP_FAIL;
    }
    return ESP_OK;
}

static esp_err_t urc_trampoline(uint8_t *data, size_t len)
{
    if (s_rx_cb) {
        s_rx_cb(data, len);
    }
    return ESP_OK;
}

esp_err_t modem_shim_set_urc_handler(modem_shim_rx_cb_t cb)
{
    if (!s_dce) {
        return ESP_ERR_INVALID_STATE;
    }
    s_rx_cb = cb;
    return esp_modem_set_urc(s_dce, urc_trampoline);
}

size_t modem_shim_resp_max(void)
{
    return RESP_MAX;
}

esp_err_t modem_shim_at_raw(const char *data, const char *pass, const char *fail,
                            int timeout_ms, char *out, size_t out_len)
{
    if (!s_dce) {
        return ESP_ERR_INVALID_STATE;
    }
    if (!out || out_len < RESP_MAX) {
        return ESP_ERR_INVALID_SIZE;
    }
    out[0] = '\0';
    return esp_modem_at_raw(s_dce, data, out, pass, fail, timeout_ms);
}

esp_err_t modem_shim_set_apn(const char *apn)
{
    if (!s_dce || !apn) {
        return ESP_ERR_INVALID_STATE;
    }
    strlcpy(s_apn, apn, sizeof(s_apn));
    return esp_modem_set_apn(s_dce, s_apn);
}

esp_err_t modem_shim_sync(void)
{
    return s_dce ? esp_modem_sync(s_dce) : ESP_ERR_INVALID_STATE;
}

esp_err_t modem_shim_set_local_baud(int baud)
{
    return uart_set_baudrate(s_uart_port, baud);
}

esp_err_t modem_shim_enter_cmux(void)
{
    return s_dce ? esp_modem_set_mode(s_dce, ESP_MODEM_MODE_CMUX) : ESP_ERR_INVALID_STATE;
}

esp_err_t modem_shim_enter_command(void)
{
    return s_dce ? esp_modem_set_mode(s_dce, ESP_MODEM_MODE_COMMAND) : ESP_ERR_INVALID_STATE;
}

esp_netif_t *modem_shim_ppp_netif(void)
{
    return s_ppp;
}

bool modem_shim_ppp_ip(char *out, size_t out_len)
{
    esp_netif_ip_info_t info;
    if (!s_ppp || esp_netif_get_ip_info(s_ppp, &info) != ESP_OK || info.ip.addr == 0) {
        if (out && out_len) {
            out[0] = '\0';
        }
        return false;
    }
    snprintf(out, out_len, IPSTR, IP2STR(&info.ip));
    return true;
}
