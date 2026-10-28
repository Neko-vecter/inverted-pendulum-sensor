#include <string.h>
#include "config.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "espnow_radio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "sensor_protocol.h"
#include "sensor_transport.h"

typedef struct {
    uint8_t packet[SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE];
    size_t length;
    uint32_t received_us;
} sync_packet_t;

static QueueHandle_t s_responses;

static void receive_packet(const esp_now_recv_info_t *info, const uint8_t *data, int length)
{
    if (info == NULL || data == NULL || length < 4 || length > SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE ||
        data[0] != 0x49 || data[1] != 0x50 || data[2] != SENSOR_PROTOCOL_VERSION ||
        !((data[3] == SENSOR_PROTOCOL_TYPE_TIME_SYNC_RESPONSE && length == SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE) ||
          (data[3] == SENSOR_PROTOCOL_TYPE_REGISTRATION_RESPONSE && length == SENSOR_PROTOCOL_REGISTRATION_RESPONSE_SIZE) ||
          (data[3] == SENSOR_PROTOCOL_TYPE_SENSOR_CONTROL && length == SENSOR_PROTOCOL_SENSOR_CONTROL_SIZE))) {
        return;
    }
    sync_packet_t response = {.length = (size_t)length, .received_us = (uint32_t)esp_timer_get_time()};
    memcpy(response.packet, data, response.length);
    (void)xQueueSend(s_responses, &response, 0);
}

esp_err_t sensor_transport_start(void)
{
    s_responses = xQueueCreate(8, sizeof(sync_packet_t));
    if (s_responses == NULL) {
        return ESP_ERR_NO_MEM;
    }
    ESP_LOGI("sensor_espnow", "ESP-NOW broadcast channel=%d", SENSOR_ESPNOW_CHANNEL);
    return espnow_radio_start(SENSOR_ESPNOW_CHANNEL, receive_packet);
}

bool sensor_transport_ready(void)
{
    return true;
}

bool sensor_transport_send(const uint8_t *packet, size_t length)
{
    return espnow_radio_send(packet, length) == ESP_OK;
}

size_t sensor_transport_receive(uint8_t *packet, size_t capacity, uint32_t *received_us)
{
    sync_packet_t response;
    if (xQueueReceive(s_responses, &response, 0) != pdTRUE || capacity < response.length) {
        return 0;
    }
    memcpy(packet, response.packet, response.length);
    *received_us = response.received_us;
    return response.length;
}
