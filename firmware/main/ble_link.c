#include <stdlib.h>
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "esp_log.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "host/ble_hs.h"
#include "host/util/util.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"
#include "ble_link.h"

#define RX_LINE_MAX 1024
#define NOTIFY_RETRIES 50

/* A5E1000n-5C0E-4B6A-9D3E-6B2F0C5A1E01, little-endian byte order */
#define BRIDGE_UUID(n) \
    BLE_UUID128_INIT(0x01, 0x1e, 0x5a, 0x0c, 0x2f, 0x6b, 0x3e, 0x9d, 0x6a, 0x4b, 0x0e, 0x5c, (n), 0x00, 0xe1, 0xa5)

void ble_store_config_init(void);

static const char *TAG = "ble";

static const ble_uuid128_t s_svc_uuid = BRIDGE_UUID(0x01);
static const ble_uuid128_t s_rx_uuid = BRIDGE_UUID(0x02);
static const ble_uuid128_t s_tx_uuid = BRIDGE_UUID(0x03);

static ble_link_callbacks_t s_cbs;
static SemaphoreHandle_t s_tx_lock;
static uint8_t s_own_addr_type;
static uint16_t s_tx_handle;
static uint16_t s_conn = BLE_HS_CONN_HANDLE_NONE;
static bool s_encrypted;
static bool s_subscribed;
static bool s_ready;

static char s_rx_line[RX_LINE_MAX];
static size_t s_rx_len;

static void advertise(void);

static void feed_rx(const uint8_t *data, size_t len)
{
    for (size_t i = 0; i < len; i++) {
        char c = (char)data[i];
        if (c == '\n') {
            if (s_rx_len) {
                s_rx_line[s_rx_len] = '\0';
                char *line = strdup(s_rx_line);
                if (line) {
                    s_cbs.on_line(line);
                }
            }
            s_rx_len = 0;
        } else if (s_rx_len < RX_LINE_MAX - 1) {
            s_rx_line[s_rx_len++] = c;
        } else {
            ESP_LOGW(TAG, "RX line too long, dropped");
            s_rx_len = 0;
        }
    }
}

static int chr_access(uint16_t conn_handle, uint16_t attr_handle, struct ble_gatt_access_ctxt *ctxt, void *arg)
{
    static uint8_t buf[512];
    switch (ctxt->op) {
    case BLE_GATT_ACCESS_OP_WRITE_CHR: {
        uint16_t len = 0;
        if (ble_hs_mbuf_to_flat(ctxt->om, buf, sizeof(buf), &len) != 0) {
            return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
        }
        feed_rx(buf, len);
        return 0;
    }
    case BLE_GATT_ACCESS_OP_READ_CHR:
        return 0;
    default:
        return BLE_ATT_ERR_UNLIKELY;
    }
}

static const struct ble_gatt_svc_def s_services[] = {
    {
        .type = BLE_GATT_SVC_TYPE_PRIMARY,
        .uuid = &s_svc_uuid.u,
        .characteristics = (struct ble_gatt_chr_def[]) {
            {
                .uuid = &s_rx_uuid.u,
                .access_cb = chr_access,
                .flags = BLE_GATT_CHR_F_WRITE | BLE_GATT_CHR_F_WRITE_NO_RSP | BLE_GATT_CHR_F_WRITE_ENC,
            },
            {
                .uuid = &s_tx_uuid.u,
                .access_cb = chr_access,
                .val_handle = &s_tx_handle,
                .flags = BLE_GATT_CHR_F_NOTIFY | BLE_GATT_CHR_F_READ | BLE_GATT_CHR_F_READ_ENC,
            },
            { 0 },
        },
    },
    { 0 },
};

static void check_ready(void)
{
    bool ready = s_conn != BLE_HS_CONN_HANDLE_NONE && s_encrypted && s_subscribed;
    if (ready && !s_ready) {
        ESP_LOGI(TAG, "Control link ready");
        s_ready = true;
        s_cbs.on_ready();
    } else if (!ready) {
        s_ready = false;
    }
}

static int gap_event(struct ble_gap_event *event, void *arg)
{
    switch (event->type) {
    case BLE_GAP_EVENT_CONNECT:
        if (event->connect.status == 0) {
            s_conn = event->connect.conn_handle;
            s_encrypted = s_subscribed = s_ready = false;
            s_rx_len = 0;
            ESP_LOGI(TAG, "Connected, requesting encryption");
            ble_gap_security_initiate(s_conn);
        } else {
            advertise();
        }
        return 0;

    case BLE_GAP_EVENT_DISCONNECT:
        ESP_LOGI(TAG, "Disconnected (reason 0x%x)", event->disconnect.reason);
        s_conn = BLE_HS_CONN_HANDLE_NONE;
        s_encrypted = s_subscribed = s_ready = false;
        advertise();
        return 0;

    case BLE_GAP_EVENT_ADV_COMPLETE:
        advertise();
        return 0;

    case BLE_GAP_EVENT_ENC_CHANGE:
        s_encrypted = event->enc_change.status == 0;
        if (!s_encrypted) {
            ESP_LOGW(TAG, "Encryption failed (%d), disconnecting", event->enc_change.status);
            ble_gap_terminate(event->enc_change.conn_handle, BLE_ERR_REM_USER_CONN_TERM);
        }
        check_ready();
        return 0;

    case BLE_GAP_EVENT_SUBSCRIBE:
        if (event->subscribe.attr_handle == s_tx_handle) {
            s_subscribed = event->subscribe.cur_notify;
            check_ready();
        }
        return 0;

    case BLE_GAP_EVENT_MTU:
        ESP_LOGI(TAG, "MTU %d", event->mtu.value);
        return 0;

    case BLE_GAP_EVENT_REPEAT_PAIRING: {
        struct ble_gap_conn_desc desc;
        if (ble_gap_conn_find(event->repeat_pairing.conn_handle, &desc) == 0) {
            ble_store_util_delete_peer(&desc.peer_id_addr);
        }
        return BLE_GAP_REPEAT_PAIRING_RETRY;
    }

    case BLE_GAP_EVENT_PASSKEY_ACTION:
        if (event->passkey.params.action == BLE_SM_IOACT_DISP) {
            struct ble_sm_io io = {
                .action = BLE_SM_IOACT_DISP,
                .passkey = CONFIG_BRIDGE_BLE_PASSKEY,
            };
            ble_sm_inject_io(event->passkey.conn_handle, &io);
        }
        return 0;

    default:
        return 0;
    }
}

static void advertise(void)
{
    struct ble_hs_adv_fields fields = { 0 };
    fields.flags = BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP;
    fields.uuids128 = &s_svc_uuid;
    fields.num_uuids128 = 1;
    fields.uuids128_is_complete = 1;
    int rc = ble_gap_adv_set_fields(&fields);
    if (rc != 0) {
        ESP_LOGE(TAG, "adv fields: %d", rc);
        return;
    }

    const char *name = ble_svc_gap_device_name();
    struct ble_hs_adv_fields rsp = { 0 };
    rsp.name = (const uint8_t *)name;
    rsp.name_len = strlen(name);
    rsp.name_is_complete = 1;
    ble_gap_adv_rsp_set_fields(&rsp);

    struct ble_gap_adv_params params = {
        .conn_mode = BLE_GAP_CONN_MODE_UND,
        .disc_mode = BLE_GAP_DISC_MODE_GEN,
    };
    rc = ble_gap_adv_start(s_own_addr_type, NULL, BLE_HS_FOREVER, &params, gap_event, NULL);
    if (rc != 0 && rc != BLE_HS_EALREADY) {
        ESP_LOGE(TAG, "adv start: %d", rc);
    }
}

static void on_sync(void)
{
    ble_hs_util_ensure_addr(0);
    ble_hs_id_infer_auto(0, &s_own_addr_type);
    advertise();
}

static void on_reset(int reason)
{
    ESP_LOGW(TAG, "Host reset: %d", reason);
}

static void host_task(void *arg)
{
    nimble_port_run();
    nimble_port_freertos_deinit();
}

esp_err_t ble_link_send(const char *data, size_t len)
{
    if (!s_ready) {
        return ESP_ERR_INVALID_STATE;
    }
    esp_err_t result = ESP_OK;
    xSemaphoreTake(s_tx_lock, portMAX_DELAY);

    uint16_t conn = s_conn;
    uint16_t mtu = ble_att_mtu(conn);
    size_t chunk = mtu > 3 ? mtu - 3 : 20;

    for (size_t off = 0; off < len;) {
        size_t n = len - off < chunk ? len - off : chunk;
        int rc = BLE_HS_ENOMEM;
        for (int attempt = 0; attempt < NOTIFY_RETRIES && s_ready; attempt++) {
            struct os_mbuf *om = ble_hs_mbuf_from_flat(data + off, n);
            /* notify_custom consumes om, also on failure */
            rc = om ? ble_gatts_notify_custom(conn, s_tx_handle, om) : BLE_HS_ENOMEM;
            if (rc != BLE_HS_ENOMEM && rc != BLE_HS_EBUSY) {
                break;
            }
            vTaskDelay(pdMS_TO_TICKS(20));
        }
        if (rc != 0) {
            ESP_LOGW(TAG, "notify failed: %d", rc);
            result = ESP_FAIL;
            break;
        }
        off += n;
    }

    xSemaphoreGive(s_tx_lock);
    return result;
}

esp_err_t ble_link_init(const ble_link_callbacks_t *cbs)
{
    s_cbs = *cbs;
    s_tx_lock = xSemaphoreCreateMutex();

    esp_err_t err = nimble_port_init();
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "nimble_port_init: %s", esp_err_to_name(err));
        return err;
    }

    ble_hs_cfg.sync_cb = on_sync;
    ble_hs_cfg.reset_cb = on_reset;
    ble_hs_cfg.store_status_cb = ble_store_util_status_rr;

    /* Passkey pairing: the ESP32 "displays" a fixed passkey that the user types on the iPhone. */
    ble_hs_cfg.sm_io_cap = BLE_SM_IO_CAP_DISP_ONLY;
    ble_hs_cfg.sm_bonding = 1;
    ble_hs_cfg.sm_mitm = 1;
    ble_hs_cfg.sm_sc = 1;
    ble_hs_cfg.sm_our_key_dist = BLE_SM_PAIR_KEY_DIST_ENC | BLE_SM_PAIR_KEY_DIST_ID;
    ble_hs_cfg.sm_their_key_dist = BLE_SM_PAIR_KEY_DIST_ENC | BLE_SM_PAIR_KEY_DIST_ID;

    ble_svc_gap_init();
    ble_svc_gatt_init();
    int rc = ble_gatts_count_cfg(s_services);
    if (rc == 0) {
        rc = ble_gatts_add_svcs(s_services);
    }
    if (rc != 0) {
        ESP_LOGE(TAG, "GATT setup failed: %d", rc);
        return ESP_FAIL;
    }
    ble_svc_gap_device_name_set(CONFIG_BRIDGE_BLE_NAME);
    ble_store_config_init();

    nimble_port_freertos_init(host_task);
    return ESP_OK;
}
