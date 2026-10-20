#pragma once

#include <stddef.h>
#include <stdint.h>
#include "esp_now.h"

// Connectionless broadcast transport; every node and gateway uses one channel.
esp_err_t espnow_radio_start(uint8_t channel, esp_now_recv_cb_t receive_callback);
esp_err_t espnow_radio_send(const uint8_t *packet, size_t length);
