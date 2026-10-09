#pragma once

#include <stdint.h>

#include "driver/i2c_master.h"
#include "esp_err.h"

esp_err_t mt6701_init(int sda_gpio,
                      int scl_gpio,
                      uint32_t clock_hz,
                      i2c_master_bus_handle_t *bus_handle,
                      i2c_master_dev_handle_t *device_handle);

esp_err_t mt6701_read_angle(i2c_master_dev_handle_t device_handle,
                            uint16_t *angle);
