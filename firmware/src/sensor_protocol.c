#include "sensor_protocol.h"

static void write_u16_be(uint8_t *destination, uint16_t value)
{
    destination[0] = (uint8_t)(value >> 8);
    destination[1] = (uint8_t)value;
}

static void write_u32_be(uint8_t *destination, uint32_t value)
{
    destination[0] = (uint8_t)(value >> 24);
    destination[1] = (uint8_t)(value >> 16);
    destination[2] = (uint8_t)(value >> 8);
    destination[3] = (uint8_t)value;
}

static uint16_t read_u16_be(const uint8_t *source)
{
    return (uint16_t)(((uint16_t)source[0] << 8) | source[1]);
}

static uint32_t read_u32_be(const uint8_t *source)
{
    return ((uint32_t)source[0] << 24) |
           ((uint32_t)source[1] << 16) |
           ((uint32_t)source[2] << 8) |
           (uint32_t)source[3];
}

static void write_header(uint8_t *packet, uint8_t packet_type)
{
    write_u16_be(&packet[0], SENSOR_PROTOCOL_MAGIC);
    packet[2] = SENSOR_PROTOCOL_VERSION;
    packet[3] = packet_type;
}

size_t sensor_protocol_encode_sample(const sensor_sample_t *sample,
                                     uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE])
{
    if (sample == NULL || packet == NULL) {
        return 0;
    }

    write_header(packet, SENSOR_PROTOCOL_TYPE_SAMPLE);
    for (size_t index = 0; index < sizeof(sample->mac); ++index) {
        packet[4U + index] = sample->mac[index];
    }
    write_u32_be(&packet[10], sample->sequence);
    write_u32_be(&packet[14], sample->timestamp_us);
    const uint16_t angle_status = sample->status == SENSOR_STATUS_I2C_ERROR
                                      ? 0x4000U
                                      : (uint16_t)(sample->raw_angle & 0x3FFFU);
    write_u16_be(&packet[18], angle_status);
    write_u32_be(&packet[20], sample->sync_token);

    return SENSOR_PROTOCOL_PACKET_SIZE;
}

size_t sensor_protocol_encode_time_sync_request(
    const uint8_t mac[6],
    uint32_t sequence,
    uint32_t device_t1_us,
    uint8_t packet[SENSOR_PROTOCOL_SYNC_REQUEST_SIZE])
{
    if (mac == NULL || packet == NULL) {
        return 0;
    }

    write_header(packet, SENSOR_PROTOCOL_TYPE_TIME_SYNC_REQUEST);
    for (size_t index = 0; index < 6U; ++index) {
        packet[4U + index] = mac[index];
    }
    write_u32_be(&packet[10], sequence);
    write_u32_be(&packet[14], device_t1_us);
    return SENSOR_PROTOCOL_SYNC_REQUEST_SIZE;
}

int sensor_protocol_decode_time_sync_response(
    const uint8_t *packet,
    size_t packet_length,
    sensor_time_sync_response_t *response)
{
    if (packet == NULL || response == NULL ||
        packet_length != SENSOR_PROTOCOL_SYNC_RESPONSE_SIZE ||
        read_u16_be(&packet[0]) != SENSOR_PROTOCOL_MAGIC ||
        packet[2] != SENSOR_PROTOCOL_VERSION ||
        packet[3] != SENSOR_PROTOCOL_TYPE_TIME_SYNC_RESPONSE) {
        return 0;
    }

    for (size_t index = 0; index < 6U; ++index) {
        response->mac[index] = packet[4U + index];
    }
    response->sequence = read_u32_be(&packet[10]);
    response->device_t1_us = read_u32_be(&packet[14]);
    response->server_t2_us = read_u32_be(&packet[18]);
    response->server_t3_us = read_u32_be(&packet[22]);
    response->sync_token = read_u32_be(&packet[26]);
    return 1;
}

size_t sensor_protocol_encode_time_sync_report(
    const uint8_t mac[6],
    uint32_t sequence,
    uint32_t device_t1_us,
    uint32_t device_t4_us,
    uint32_t sync_token,
    uint8_t packet[SENSOR_PROTOCOL_SYNC_REPORT_SIZE])
{
    if (mac == NULL || packet == NULL || sync_token == 0U) {
        return 0;
    }
    write_header(packet, SENSOR_PROTOCOL_TYPE_TIME_SYNC_REPORT);
    for (size_t index = 0; index < 6U; ++index) {
        packet[4U + index] = mac[index];
    }
    write_u32_be(&packet[10], sequence);
    write_u32_be(&packet[14], device_t1_us);
    write_u32_be(&packet[18], device_t4_us);
    write_u32_be(&packet[22], sync_token);
    return SENSOR_PROTOCOL_SYNC_REPORT_SIZE;
}
