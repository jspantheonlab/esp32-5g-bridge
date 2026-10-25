#pragma once

#include "esp_err.h"

/*
 * Bridges modem PCM audio (I2S, 16-bit mono) to the iPhone over UDP.
 * Packet: 8-byte header {'5','G', seq u16 LE, timestamp u32 LE} + one 20 ms frame of int16 LE samples.
 * The iPhone's address is learned from the first packet it sends to CONFIG_BRIDGE_AUDIO_UDP_PORT.
 */
esp_err_t audio_bridge_init(void);
void audio_bridge_start(void);
void audio_bridge_stop(void);
