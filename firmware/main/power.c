/*
 * Battery monitoring for the custom board (docs/board-design.md): MAX17048 fuel gauge and
 * BQ25895 charger on I2C. Disabled automatically when no gauge answers (e.g. a DevKit on USB).
 * The charger's register defaults (2 A, 4.208 V) match the board design, so it is only read.
 * Same logic as firmware-rs/firmware/src/power.rs.
 */
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/i2c_master.h"
#include "esp_log.h"
#include "esp_sleep.h"
#include "app.h"
#include "modem.h"
#include "power.h"

#define MAX17048_ADDR      0x36
#define MAX17048_REG_SOC   0x04
#define BQ25895_ADDR       0x6A
#define BQ25895_REG_STATUS 0x0B
#define CRITICAL_PERCENT   3
#define POLL_MS            30000
#define I2C_TIMEOUT_MS     50

static const char *TAG = "power";

static i2c_master_dev_handle_t s_gauge;
static i2c_master_dev_handle_t s_charger;
static power_status_t s_status;
static portMUX_TYPE s_mux = portMUX_INITIALIZER_UNLOCKED;
static void (*s_on_change)(void);

void power_get_status(power_status_t *out)
{
    taskENTER_CRITICAL(&s_mux);
    *out = s_status;
    taskEXIT_CRITICAL(&s_mux);
}

static bool read_reg(i2c_master_dev_handle_t dev, uint8_t reg, uint8_t *buf, size_t len)
{
    return i2c_master_transmit_receive(dev, &reg, 1, buf, len, I2C_TIMEOUT_MS) == ESP_OK;
}

static void shutdown(int percent)
{
    ESP_LOGW(TAG, "Battery critical (%d%%), shutting down", percent);
    cJSON *msg = cJSON_CreateObject();
    cJSON_AddStringToObject(msg, "t", "err");
    cJSON_AddStringToObject(msg, "ctx", "power");
    cJSON_AddStringToObject(msg, "msg", "battery critical, shutting down");
    app_emit(msg);
    modem_at("AT+QPOWD=1", NULL, 0, 5000);
    vTaskDelay(pdMS_TO_TICKS(3000));
    esp_sleep_enable_ext0_wakeup(CONFIG_BRIDGE_POWER_BUTTON_GPIO, 0);
    esp_deep_sleep_start();
}

static void power_task(void *arg)
{
    for (;;) {
        power_status_t st = { 0 };
        uint8_t soc[2], reg0b;
        if (read_reg(s_gauge, MAX17048_REG_SOC, soc, sizeof(soc))) {
            st.present = true;
            st.percent = soc[0] > 100 ? 100 : soc[0];   /* MSB = whole percent */
        }
        if (s_charger && read_reg(s_charger, BQ25895_REG_STATUS, &reg0b, 1)) {
            int chrg_stat = (reg0b >> 3) & 0x3;           /* 1 = pre-charge, 2 = fast charge */
            st.charger = true;
            st.charging = chrg_stat == 1 || chrg_stat == 2;
            st.usb = reg0b & 0x04;                        /* PG_STAT */
        }

        taskENTER_CRITICAL(&s_mux);
        bool changed = st.present != s_status.present || st.percent != s_status.percent ||
                       st.charger != s_status.charger || st.charging != s_status.charging ||
                       st.usb != s_status.usb;
        s_status = st;
        taskEXIT_CRITICAL(&s_mux);
        if (changed && s_on_change) {
            s_on_change();
        }

        if (st.present && st.percent <= CRITICAL_PERCENT && !st.usb) {
            shutdown(st.percent);
        }
        vTaskDelay(pdMS_TO_TICKS(POLL_MS));
    }
}

void power_init(void (*on_change)(void))
{
    s_on_change = on_change;

    i2c_master_bus_config_t bus_cfg = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = CONFIG_BRIDGE_I2C_SDA_GPIO,
        .scl_io_num = CONFIG_BRIDGE_I2C_SCL_GPIO,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
        .flags.enable_internal_pullup = true,
    };
    i2c_master_bus_handle_t bus;
    if (i2c_new_master_bus(&bus_cfg, &bus) != ESP_OK) {
        ESP_LOGW(TAG, "I2C bus init failed; battery reporting disabled");
        return;
    }
    if (i2c_master_probe(bus, MAX17048_ADDR, I2C_TIMEOUT_MS) != ESP_OK) {
        ESP_LOGI(TAG, "No battery gauge found; battery reporting disabled");
        i2c_del_master_bus(bus);
        return;
    }

    i2c_device_config_t dev_cfg = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = MAX17048_ADDR,
        .scl_speed_hz = 100000,
    };
    i2c_master_bus_add_device(bus, &dev_cfg, &s_gauge);
    if (i2c_master_probe(bus, BQ25895_ADDR, I2C_TIMEOUT_MS) == ESP_OK) {
        dev_cfg.device_address = BQ25895_ADDR;
        i2c_master_bus_add_device(bus, &dev_cfg, &s_charger);
    }
    ESP_LOGI(TAG, "Battery gauge found%s", s_charger ? " (with charger)" : "");
    xTaskCreate(power_task, "power", 4096, NULL, 3, NULL);
}
