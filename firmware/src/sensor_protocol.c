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

static void write_u64_be(uint8_t *destination, uint64_t value)
{
    for (size_t index = 0; index < 8; ++index) {
        destination[index] = (uint8_t)(value >> (56U - (8U * index)));
    }
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

static uint64_t read_u64_be(const uint8_t *source)
{
    uint64_t value = 0;
    for (size_t index = 0; index < 8; ++index) {
        value = (value << 8) | source[index];
    }
    return value;
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
    write_u64_be(&packet[14], sample->timestamp_us);
    write_u16_be(&packet[22], sample->raw_angle);
    packet[24] = sample->status;
    packet[25] = 0;
    write_u64_be(&packet[26], sample->sync_session_id);
    write_u64_be(&packet[34], (uint64_t)sample->clock_offset_us);
    write_u32_be(&packet[42], sample->sync_round_trip_us);

    return SENSOR_PROTOCOL_PACKET_SIZE;
}

size_t sensor_protocol_encode_time_sync_request(
    const uint8_t mac[6],
    uint32_t sequence,
    uint64_t device_t1_us,
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
    write_u64_be(&packet[14], device_t1_us);
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
    response->device_t1_us = read_u64_be(&packet[14]);
    response->server_t2_us = read_u64_be(&packet[22]);
    response->server_t3_us = read_u64_be(&packet[30]);
    response->server_session_id = read_u64_be(&packet[38]);
    return 1;
}
