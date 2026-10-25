#pragma once

/*
 * Thin C layer over esp_modem so the Rust firmware never has to replicate its C/C++
 * config macros or struct layouts. All state lives here; there is one modem.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"
#include "esp_netif.h"

typedef struct {
    int uart_port;
    int tx_gpio;
    int rx_gpio;
    int rts_gpio;
    int cts_gpio;
    int baud_rate;
    const char *apn;   /* copied */
} modem_shim_config_t;

/* Called from the esp_modem DTE task with raw received bytes (not line-split). */
typedef void (*modem_shim_rx_cb_t)(const uint8_t *data, size_t len);

/* Creates the PPP netif and the esp_modem DCE (UART, HW flow control). */
esp_err_t modem_shim_init(const modem_shim_config_t *config);

/* Registers the unsolicited-result-code byte callback. */
esp_err_t modem_shim_set_urc_handler(modem_shim_rx_cb_t cb);

/* Size the `out` buffer of modem_shim_at_raw must have. */
size_t modem_shim_resp_max(void);

/* Sends raw bytes (caller adds \r / Ctrl-Z), waits for pass/fail phrase. */
esp_err_t modem_shim_at_raw(const char *data, const char *pass, const char *fail,
                            int timeout_ms, char *out, size_t out_len);

/* Plain "AT" handshake. */
esp_err_t modem_shim_sync(void);

/* Changes the local UART baud rate (the modem side is changed with AT+IPR). */
esp_err_t modem_shim_set_local_baud(int baud);

/* Enter CMUX (PPP on one channel, AT on another) / return to plain command mode. */
esp_err_t modem_shim_enter_cmux(void);
esp_err_t modem_shim_enter_command(void);

/* PPP netif handle (for NAPT routing). */
esp_netif_t *modem_shim_ppp_netif(void);

/* Writes the PPP IPv4 address as text and returns true if the data session has an address. */
bool modem_shim_ppp_ip(char *out, size_t out_len);
