#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"

typedef struct {
    bool sim_ready;
    bool registered;
    bool data_up;
    int rssi_dbm;           /* 0 = unknown */
    int act;                /* 3GPP access technology from +COPS, -1 = unknown */
    char operator_name[32];
    char ip[16];
} modem_status_t;

typedef void (*modem_urc_cb_t)(const char *line);
typedef void (*modem_status_cb_t)(const modem_status_t *status);

esp_err_t modem_init(void);

void modem_add_urc_handler(modem_urc_cb_t cb);
void modem_set_status_handler(modem_status_cb_t cb);
void modem_get_status(modem_status_t *out);
const char *modem_rat_name(int act);

/* Hold the AT channel across a multi-command sequence (recursive). */
void modem_lock(void);
void modem_unlock(void);

/* Send "cmd\r", wait for OK / ERROR. out receives the full raw response. */
esp_err_t modem_at(const char *cmd, char *out, size_t out_len, uint32_t timeout_ms);

/* Send raw bytes (caller adds terminators), wait for pass / fail phrase. */
esp_err_t modem_at_raw(const char *raw, const char *pass, const char *fail,
                       char *out, size_t out_len, uint32_t timeout_ms);
