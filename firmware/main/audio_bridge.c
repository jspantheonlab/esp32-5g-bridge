#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/event_groups.h"
#include "driver/i2s_std.h"
#include "lwip/sockets.h"
#include "esp_log.h"
#include "audio_bridge.h"

#define SAMPLE_RATE    CONFIG_BRIDGE_PCM_SAMPLE_RATE
#define FRAME_SAMPLES  (SAMPLE_RATE / 50)          /* 20 ms */
#define FRAME_BYTES    (FRAME_SAMPLES * 2)
#define JITTER_FRAMES  8
#define JITTER_PREFILL 3
#define RUN_BIT        BIT0

typedef struct __attribute__((packed)) {
    uint8_t magic[2];
    uint16_t seq;
    uint32_t ts;
} pkt_hdr_t;

static const char *TAG = "audio";

static i2s_chan_handle_t s_tx;
static i2s_chan_handle_t s_rx;
static int s_sock = -1;
static EventGroupHandle_t s_events;

/* Only touched by the bridge task. */
static int16_t s_jitter[JITTER_FRAMES][FRAME_SAMPLES];
static int s_jhead;
static int s_jcount;
static struct sockaddr_in s_peer;
static bool s_has_peer;

static void jitter_reset(void)
{
    s_jhead = 0;
    s_jcount = 0;
}

static void jitter_push(const void *pcm)
{
    if (s_jcount == JITTER_FRAMES) {
        s_jhead = (s_jhead + 1) % JITTER_FRAMES;   /* drop oldest */
        s_jcount--;
    }
    memcpy(s_jitter[(s_jhead + s_jcount) % JITTER_FRAMES], pcm, FRAME_BYTES);
    s_jcount++;
}

static bool jitter_pop(int16_t *pcm)
{
    if (s_jcount == 0) {
        return false;
    }
    memcpy(pcm, s_jitter[s_jhead], FRAME_BYTES);
    s_jhead = (s_jhead + 1) % JITTER_FRAMES;
    s_jcount--;
    return true;
}

static void drain_socket(void)
{
    uint8_t buf[sizeof(pkt_hdr_t) + FRAME_BYTES];
    for (;;) {
        struct sockaddr_in from;
        socklen_t from_len = sizeof(from);
        int n = recvfrom(s_sock, buf, sizeof(buf), MSG_DONTWAIT, (struct sockaddr *)&from, &from_len);
        if (n < 0) {
            return;
        }
        if (n < (int)sizeof(pkt_hdr_t) || buf[0] != '5' || buf[1] != 'G') {
            continue;
        }
        s_peer = from;
        s_has_peer = true;
        if (n == (int)sizeof(buf)) {
            jitter_push(buf + sizeof(pkt_hdr_t));
        }
    }
}

static void bridge_task(void *arg)
{
    int16_t speaker[FRAME_SAMPLES];
    uint8_t pkt[sizeof(pkt_hdr_t) + FRAME_BYTES];
    uint16_t seq = 0;
    uint32_t ts = 0;

    for (;;) {
        xEventGroupWaitBits(s_events, RUN_BIT, pdFALSE, pdTRUE, portMAX_DELAY);
        ESP_LOGI(TAG, "Call audio started");
        jitter_reset();
        s_has_peer = false;
        bool playing = false;
        i2s_channel_enable(s_tx);
        i2s_channel_enable(s_rx);

        while (xEventGroupGetBits(s_events) & RUN_BIT) {
            size_t got = 0;
            /* I2S read paces the loop at the PCM clock (20 ms per frame). */
            if (i2s_channel_read(s_rx, pkt + sizeof(pkt_hdr_t), FRAME_BYTES, &got, pdMS_TO_TICKS(100)) != ESP_OK ||
                got != FRAME_BYTES) {
                drain_socket();
                continue;
            }
            drain_socket();

            if (s_has_peer) {
                pkt_hdr_t hdr = { .magic = { '5', 'G' }, .seq = seq, .ts = ts };
                memcpy(pkt, &hdr, sizeof(hdr));
                sendto(s_sock, pkt, sizeof(pkt), 0, (struct sockaddr *)&s_peer, sizeof(s_peer));
            }
            seq++;
            ts += FRAME_SAMPLES;

            if (!playing && s_jcount >= JITTER_PREFILL) {
                playing = true;
            }
            if (!playing || !jitter_pop(speaker)) {
                memset(speaker, 0, sizeof(speaker));
                playing = false;
            }
            size_t written;
            i2s_channel_write(s_tx, speaker, FRAME_BYTES, &written, pdMS_TO_TICKS(100));
        }

        i2s_channel_disable(s_tx);
        i2s_channel_disable(s_rx);
        ESP_LOGI(TAG, "Call audio stopped");
    }
}

esp_err_t audio_bridge_init(void)
{
    s_events = xEventGroupCreate();

#ifdef CONFIG_BRIDGE_I2S_MASTER
    i2s_chan_config_t chan_cfg = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_MASTER);
#else
    i2s_chan_config_t chan_cfg = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_SLAVE);
#endif
    chan_cfg.dma_frame_num = FRAME_SAMPLES;
    ESP_ERROR_CHECK(i2s_new_channel(&chan_cfg, &s_tx, &s_rx));

    /* PCM short-frame sync, 16-bit, modem audio in the first slot. */
    i2s_std_config_t std_cfg = {
        .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(SAMPLE_RATE),
        .slot_cfg = I2S_STD_PCM_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_16BIT, I2S_SLOT_MODE_MONO),
        .gpio_cfg = {
            .mclk = I2S_GPIO_UNUSED,
            .bclk = CONFIG_BRIDGE_PCM_BCLK_GPIO,
            .ws = CONFIG_BRIDGE_PCM_WS_GPIO,
            .dout = CONFIG_BRIDGE_PCM_DOUT_GPIO,
            .din = CONFIG_BRIDGE_PCM_DIN_GPIO,
        },
    };
    ESP_ERROR_CHECK(i2s_channel_init_std_mode(s_tx, &std_cfg));
    ESP_ERROR_CHECK(i2s_channel_init_std_mode(s_rx, &std_cfg));

    s_sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (s_sock < 0) {
        ESP_LOGE(TAG, "socket() failed");
        return ESP_FAIL;
    }
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(CONFIG_BRIDGE_AUDIO_UDP_PORT),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (bind(s_sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        ESP_LOGE(TAG, "bind() failed");
        return ESP_FAIL;
    }

    xTaskCreate(bridge_task, "audio", 6144, NULL, 10, NULL);
    return ESP_OK;
}

void audio_bridge_start(void) { xEventGroupSetBits(s_events, RUN_BIT); }
void audio_bridge_stop(void)  { xEventGroupClearBits(s_events, RUN_BIT); }
