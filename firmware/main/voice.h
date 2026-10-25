#pragma once

#include "esp_err.h"

typedef enum {
    CALL_IDLE,
    CALL_INCOMING,
    CALL_DIALING,
    CALL_ALERTING,
    CALL_ACTIVE,
    CALL_HELD,
    CALL_WAITING,
} call_state_t;

void voice_init(void);
void voice_on_urc(const char *line);

/* Re-send the current call state to the phone (e.g. after it reconnects). */
void voice_resend_state(void);

esp_err_t voice_dial(const char *number);
esp_err_t voice_answer(void);
esp_err_t voice_hangup(void);
esp_err_t voice_dtmf(char digit);
