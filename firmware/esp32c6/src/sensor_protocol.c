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

size_t sensor_protocol_encode_sample(const sensor_sample_t *sample,
                                     uint8_t packet[SENSOR_PROTOCOL_PACKET_SIZE])
{
    if (sample == NULL || packet == NULL) {
        return 0;
    }

    write_u16_be(&packet[0], SENSOR_PROTOCOL_MAGIC);
    packet[2] = SENSOR_PROTOCOL_VERSION;
    packet[3] = SENSOR_PROTOCOL_TYPE_SAMPLE;
    for (size_t index = 0; index < sizeof(sample->mac); ++index) {
        packet[4U + index] = sample->mac[index];
    }
    write_u32_be(&packet[10], sample->sequence);
    write_u64_be(&packet[14], sample->timestamp_us);
    write_u16_be(&packet[22], sample->raw_angle);
    packet[24] = sample->status;
    packet[25] = 0;

    return SENSOR_PROTOCOL_PACKET_SIZE;
}
