set(transport_sources "transport_espnow.c" "espnow_radio.c")
set(transport_requires esp_wifi esp_event nvs_flash)
if(NOT CONFIG_SENSOR_ESPNOW)
    set(transport_sources "transport_udp.c")
    list(APPEND transport_requires esp_netif lwip)
endif()

idf_component_register(
    SRCS
        "main.c"
        "mt6701.c"
        "sensor_protocol.c"
        ${transport_sources}
    INCLUDE_DIRS "."
    REQUIRES
        esp_driver_gpio
        esp_driver_i2c
        esp_hw_support
        freertos
        log
        esp_timer
        ${transport_requires}
)
