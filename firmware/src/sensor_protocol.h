#pragma once

#include <stddef.h>
#include <stdint.h>

#define SENSOR_PROTOCOL_PACKET_SIZE 24U
#define SENSOR_PROTOCOL_SYNC_REQUEST_SIZE 18U
#define SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE 30U
#define SENSOR_PROTOCOL_SYNC_REPORT_SIZE 26U
#define SENSOR_PROTOCOL_MAGIC 0x4950U
#define SENSOR_PROTOCOL_VERSION 3U
#define SENSOR_PROTOCOL_TYPE_SAMPLE 0x01U
#define SENSOR_PROTOCOL_TYPE_TIME_SYNC_REQUEST 0x02U
#define SENSOR_PROTOCOL_TYPE_TIME_SYNC_RESPONSE 0x03U
#define SENSOR_PROTOCOL_TYPE_TIME_SYNC_REPORT 0x04U

#define SENSOR_STATUS_ANGLE_VALID 0x01U
#define SENSOR_STATUS_I2C_ERROR 0x02U

typedef struct {
    uint8_t mac[6];
    uint32_t sequence;
    uint32_t timestamp_us;
    uint16_t raw_angle;
    uint8_t status;
    uint32_t sync_token;
} sensor_sample_t;

typedef struct {
    uint8_t mac[6];
    uint32_t sequence;
    uint32_t device_t1_us;
    uint32_t server_t2_us;
    uint32_t server_t3_us;
    uint32_t sync_token;
} sensor_time_sync_response_t;

size_t sensor_protocol_encode_sample(const sensor_sample_t *sample,
                                     uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE]);
size_t sensor_protocol_encode_time_sync_request(const uint8_t mac[6],
                                               uint32_t sequence,
                                               uint32_t device_t1_us,
                                               uint8_t packet[SENSOR_PROTOCOL_SYNC_REQUEST_SIZE]);
int sensor_protocol_decode_time_sync_response(
    const uint8_t *packet,
    size_t packet_length,
    sensor_time_sync_response_t *response);
size_t sensor_protocol_encode_time_sync_report(
    const uint8_t mac[6],
    uint32_t sequence,
    uint32_t device_t1_us,
    uint32_t device_t4_us,
    uint32_t sync_token,
    uint8_t packet[SENSOR_PROTOCOL_SYNC_REPORT_SIZE]);
