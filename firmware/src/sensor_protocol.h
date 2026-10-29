#pragma once
#include <stddef.h>
#include <stdint.h>
#define SENSOR_PROTOCOL_PACKET_SIZE 20U
#define SENSOR_PROTOCOL_SYNC_REQUEST_SIZE 14U
#define SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE 26U
#define SENSOR_PROTOCOL_SYNC_REPORT_SIZE 22U
#define SENSOR_PROTOCOL_REGISTRATION_REQUEST_SIZE 14U
#define SENSOR_PROTOCOL_REGISTRATION_RESPONSE_SIZE 16U
#define SENSOR_PROTOCOL_SENSOR_CONTROL_SIZE 15U
#define SENSOR_PROTOCOL_SENSOR_CONTROL_ACK_SIZE 15U
#define SENSOR_PROTOCOL_MAGIC 0x4950U
#define SENSOR_PROTOCOL_VERSION 3U
#define SENSOR_PROTOCOL_TYPE_SAMPLE 0x01U
#define SENSOR_PROTOCOL_TYPE_TIME_SYNC_REQUEST 0x02U
#define SENSOR_PROTOCOL_TYPE_TIME_SYNC_RESPONSE 0x03U
#define SENSOR_PROTOCOL_TYPE_TIME_SYNC_REPORT 0x04U
#define SENSOR_PROTOCOL_TYPE_REGISTRATION_REQUEST 0x05U
#define SENSOR_PROTOCOL_TYPE_REGISTRATION_RESPONSE 0x06U
#define SENSOR_PROTOCOL_TYPE_SENSOR_CONTROL 0x07U
#define SENSOR_PROTOCOL_TYPE_SENSOR_CONTROL_ACK 0x08U
#define SENSOR_CONTROL_START_MEASUREMENT 0x01U
#define SENSOR_CONTROL_STOP_MEASUREMENT 0x02U
#define SENSOR_STATUS_ANGLE_VALID 0x01U
#define SENSOR_STATUS_I2C_ERROR 0x02U
typedef struct { uint16_t device_id; uint32_t sequence; uint32_t timestamp_us; uint16_t raw_angle; uint8_t status; uint32_t sync_token; } sensor_sample_t;
typedef struct { uint16_t device_id; uint32_t sequence; uint32_t device_t1_us; uint32_t server_t2_us; uint32_t server_t3_us; uint32_t sync_token; } sensor_time_sync_response_t;
typedef struct { uint8_t mac[6]; uint32_t sequence; uint16_t device_id; } sensor_registration_response_t;
typedef struct {
    uint16_t device_id;
    uint8_t command;
    uint32_t session_token;
    uint32_t sequence;
} sensor_control_command_t;
typedef sensor_control_command_t sensor_control_ack_t;
size_t sensor_protocol_encode_registration_request(const uint8_t mac[6], uint32_t sequence, uint8_t packet[SENSOR_PROTOCOL_REGISTRATION_REQUEST_SIZE]);
int sensor_protocol_decode_registration_response(const uint8_t *packet, size_t packet_length, sensor_registration_response_t *response);
size_t sensor_protocol_encode_sample(const sensor_sample_t *sample, uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE]);
size_t sensor_protocol_encode_time_sync_request(uint16_t device_id, uint32_t sequence, uint32_t device_t1_us, uint8_t packet[SENSOR_PROTOCOL_SYNC_REQUEST_SIZE]);
int sensor_protocol_decode_time_sync_response(const uint8_t *packet, size_t packet_length, sensor_time_sync_response_t *response);
int sensor_protocol_decode_sensor_control(const uint8_t *packet, size_t packet_length, sensor_control_command_t *command);
size_t sensor_protocol_encode_sensor_control_ack(const sensor_control_ack_t *ack, uint8_t packet[SENSOR_PROTOCOL_SENSOR_CONTROL_ACK_SIZE]);
size_t sensor_protocol_encode_time_sync_report(uint16_t device_id, uint32_t sequence, uint32_t device_t1_us, uint32_t device_t4_us, uint32_t sync_token, uint8_t packet[SENSOR_PROTOCOL_SYNC_REPORT_SIZE]);
