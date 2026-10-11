cmake_minimum_required(VERSION 3.16.0)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)

# The XIAO ESP32-C5 module has 8 MB flash. Add this as an SDK config default
# for C5 builds so the setting is selected by the IDF target, not a local file.
if("${IDF_TARGET}" STREQUAL "esp32c5" OR "$ENV{IDF_TARGET}" STREQUAL "esp32c5")
    set(C5_FLASH_CONFIG_DEFAULTS "${CMAKE_BINARY_DIR}/sdkconfig.c5.flash.defaults")
    file(WRITE "${C5_FLASH_CONFIG_DEFAULTS}" "CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y\n")
    list(APPEND SDKCONFIG_DEFAULTS "${C5_FLASH_CONFIG_DEFAULTS}")
endif()

project(pendulum_sensor)
