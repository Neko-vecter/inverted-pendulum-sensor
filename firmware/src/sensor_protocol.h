#pragma once

#include <stddef.h>
#include <stdint.h>

#define SENSOR_PROTOCOL_PACKET_SIZE 26U
#define SENSOR_PROTOCOL_MAGIC 0x4950U
#define SENSOR_PROTOCOL_VERSION 1U
#define SENSOR_PROTOCOL_TYPE_SAMPLE 0x01U

#define SENSOR_STATUS_ANGLE_VALID 0x01U
#define SENSOR_STATUS_I2C_ERROR 0x02U

typedef struct {
    uint8_t mac[6];
    uint32_t sequence;
    uint64_t timestamp_us;
    uint16_t raw_angle;
    uint8_t status;
} sensor_sample_t;

size_t sensor_protocol_encode_sample(const sensor_sample_t *sample,
                                     uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE]);
