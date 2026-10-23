#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// COBS(payload || CRC16-CCITT-FALSE big endian), delimited by zero bytes.
#define SERIAL_PAYLOAD_MAX 26U
#define SERIAL_ENCODED_MAX (SERIAL_PAYLOAD_MAX + 3U)
#define SERIAL_FRAME_MAX (SERIAL_ENCODED_MAX + 2U)

static inline uint16_t serial_crc16(const uint8_t *data, size_t length)
{
    uint16_t crc = 0xffff;
    for (size_t i = 0; i < length; ++i) {
        crc ^= (uint16_t)data[i] << 8;
        for (unsigned bit = 0; bit < 8; ++bit) {
            crc = (crc & 0x8000) ? (uint16_t)((crc << 1) ^ 0x1021)
                                : (uint16_t)(crc << 1);
        }
    }
    return crc;
}

static inline size_t serial_frame_encode(const uint8_t *packet, size_t length,
                                         uint8_t frame[SERIAL_FRAME_MAX])
{
    if (length == 0 || length > SERIAL_PAYLOAD_MAX) {
        return 0;
    }
    uint8_t raw[SERIAL_PAYLOAD_MAX + 2U];
    for (size_t i = 0; i < length; ++i) {
        raw[i] = packet[i];
    }
    const uint16_t crc = serial_crc16(packet, length);
    raw[length] = (uint8_t)(crc >> 8);
    raw[length + 1] = (uint8_t)crc;
    frame[0] = 0;
    size_t code_index = 1, output = 2;
    uint8_t code = 1;
    for (size_t i = 0; i < length + 2; ++i) {
        if (raw[i] == 0) {
            frame[code_index] = code;
            code_index = output++;
            code = 1;
        } else {
            frame[output++] = raw[i];
            ++code;
        }
    }
    frame[code_index] = code;
    frame[output++] = 0;
    return output;
}

static inline size_t serial_frame_decode(const uint8_t *encoded, size_t length,
                                         uint8_t packet[SERIAL_PAYLOAD_MAX])
{
    uint8_t raw[SERIAL_PAYLOAD_MAX + 2U];
    size_t input = 0, output = 0;
    while (input < length) {
        const uint8_t code = encoded[input++];
        if (code == 0 || input + code - 1 > length) {
            return 0;
        }
        for (unsigned i = 1; i < code; ++i) {
            if (output >= sizeof(raw)) {
                return 0;
            }
            raw[output++] = encoded[input++];
        }
        if (code != 255 && input < length) {
            if (output >= sizeof(raw)) {
                return 0;
            }
            raw[output++] = 0;
        }
    }
    if (output < 3) {
        return 0;
    }
    const size_t payload_length = output - 2;
    const uint16_t crc = ((uint16_t)raw[payload_length] << 8) | raw[payload_length + 1];
    if (serial_crc16(raw, payload_length) != crc) {
        return 0;
    }
    for (size_t i = 0; i < payload_length; ++i) {
        packet[i] = raw[i];
    }
    return payload_length;
}
