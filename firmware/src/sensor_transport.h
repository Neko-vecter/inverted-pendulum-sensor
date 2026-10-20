#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_err.h"

esp_err_t sensor_transport_start(void);
bool sensor_transport_ready(void);
bool sensor_transport_send(const uint8_t *packet, size_t length);
size_t sensor_transport_receive(uint8_t *packet, size_t capacity, uint32_t *received_us);
