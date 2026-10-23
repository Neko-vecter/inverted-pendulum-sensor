set(SENSOR_CONFIG_HEADER "${CMAKE_CURRENT_LIST_DIR}/config.h")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${SENSOR_CONFIG_HEADER}")
if(NOT EXISTS "${SENSOR_CONFIG_HEADER}")
    message(FATAL_ERROR "Missing src/config.h. Copy src/config.example.h to src/config.h and edit it.")
endif()
file(STRINGS "${SENSOR_CONFIG_HEADER}" sensor_transport_setting
    REGEX "^[ \t]*#[ \t]*define[ \t]+SENSOR_USE_ESPNOW[ \t]+[01]([ \t]|$)")
if(NOT sensor_transport_setting)
    message(FATAL_ERROR "config.h must define SENSOR_USE_ESPNOW as 0 or 1")
endif()
if(sensor_transport_setting MATCHES "SENSOR_USE_ESPNOW[ \t]+1")
    set(transport_sources "transport_espnow.c" "espnow_radio.c")
    set(transport_requires esp_wifi esp_event nvs_flash)
else()
    set(transport_sources "transport_udp.c")
    set(transport_requires esp_wifi esp_event nvs_flash esp_netif lwip)
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
