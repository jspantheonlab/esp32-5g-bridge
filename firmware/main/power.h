#pragma once

#include <stdbool.h>

typedef struct {
    bool present;      /* fuel gauge found */
    int percent;       /* 0..100 */
    bool charger;      /* charger found (charging/usb are valid) */
    bool charging;
    bool usb;
} power_status_t;

/* Starts battery monitoring (MAX17048 + BQ25895 on I2C). No-op if no gauge answers. */
void power_init(void (*on_change)(void));
void power_get_status(power_status_t *out);
