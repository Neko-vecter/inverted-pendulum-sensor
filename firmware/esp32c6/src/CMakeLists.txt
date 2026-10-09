idf_component_register(
    SRCS
        "main.c"
        "mt6701.c"
        "sensor_protocol.c"
    INCLUDE_DIRS "."
    REQUIRES
        driver
        esp_event
        esp_hw_support
        esp_netif
        freertos
        log
        esp_timer
        esp_wifi
        lwip
        nvs_flash
)
