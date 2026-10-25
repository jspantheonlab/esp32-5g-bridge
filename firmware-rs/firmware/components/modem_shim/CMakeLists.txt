idf_component_register(
    SRCS "modem_shim.c"
    INCLUDE_DIRS "include"
    REQUIRES esp_netif
    PRIV_REQUIRES esp_driver_uart
)
