set(GATEWAY_CONFIG_HEADER "${CMAKE_CURRENT_LIST_DIR}/config.h")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${GATEWAY_CONFIG_HEADER}")
if(NOT EXISTS "${GATEWAY_CONFIG_HEADER}")
    message(FATAL_ERROR "Missing src/config.h. Copy src/config.example.h to src/config.h and edit it.")
endif()
file(STRINGS "${GATEWAY_CONFIG_HEADER}" gateway_serial_setting
    REGEX "^[ \t]*#[ \t]*define[ \t]+GATEWAY_SERIAL_USB[ \t]+[01]([ \t]|$)")
if(NOT gateway_serial_setting)
    message(FATAL_ERROR "config.h must define GATEWAY_SERIAL_USB as 0 or 1")
endif()
if(gateway_serial_setting MATCHES "GATEWAY_SERIAL_USB[ \t]+1")
    set(gateway_serial_requires esp_driver_usb_serial_jtag)
else()
    set(gateway_serial_requires esp_driver_uart)
endif()

idf_component_register(
    SRCS "main.c" "espnow_radio.c"
    INCLUDE_DIRS "."
    REQUIRES esp_wifi esp_event nvs_flash esp_timer esp_hw_support
             ${gateway_serial_requires} freertos log
)
