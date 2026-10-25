idf_component_register(
    SRCS
        "main.c"
        "modem.c"
        "hotspot.c"
        "ble_link.c"
        "sms.c"
        "ucs2.c"
        "voice.c"
        "audio_bridge.c"
        "power.c"
        "reset_button.c"
    INCLUDE_DIRS "."
    PRIV_REQUIRES nvs_flash esp_wifi esp_netif esp_event bt json driver lwip
)
