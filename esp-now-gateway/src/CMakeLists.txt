idf_component_register(
    SRCS "main.c" "espnow_radio.c"
    INCLUDE_DIRS "."
    REQUIRES esp_wifi esp_event nvs_flash esp_timer esp_hw_support
             esp_driver_uart esp_driver_usb_serial_jtag freertos log
)
