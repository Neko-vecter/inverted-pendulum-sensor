#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "../src/serial_frame.h"

int main(void)
{
    const uint8_t payload[] = {0x49, 0, 0x50};
    const uint8_t expected[] = {0, 2, 0x49, 4, 0x50, 0x15, 0x55, 0};
    uint8_t frame[SERIAL_FRAME_MAX], decoded[SERIAL_PAYLOAD_MAX];
    assert(serial_crc16((const uint8_t *)"123456789", 9) == 0x29b1);
    assert(serial_frame_encode(payload, sizeof(payload), frame) == sizeof(expected));
    assert(memcmp(frame, expected, sizeof(expected)) == 0);
    assert(serial_frame_decode(frame + 1, sizeof(expected) - 3, decoded) == 0);
    frame[2] ^= 1;
    assert(serial_frame_decode(frame + 1, sizeof(expected) - 2, decoded) == 0);
    // Exercise all lengths, zeros, nonzero bytes and deterministic mixed data.
    uint32_t random = 1;
    for (size_t length = 1; length <= SERIAL_PAYLOAD_MAX; ++length) {
        for (unsigned pattern = 0; pattern < 256; ++pattern) {
            uint8_t packet[SERIAL_PAYLOAD_MAX];
            for (size_t i = 0; i < length; ++i) {
                random = random * 1664525U + 1013904223U;
                packet[i] = pattern < 3 ? (uint8_t)(pattern == 2 ? 255 : pattern)
                                       : (uint8_t)(random >> 24);
            }
            const size_t size = serial_frame_encode(packet, length, frame);
            assert(size <= SERIAL_FRAME_MAX && frame[0] == 0 && frame[size - 1] == 0);
            assert(serial_frame_decode(frame + 1, size - 2, decoded) == length);
            assert(memcmp(packet, decoded, length) == 0);
        }
    }
    uint8_t invalid[] = {255, 1};
    assert(serial_frame_decode(invalid, sizeof(invalid), decoded) == 0);
    assert(serial_frame_decode(invalid, 0, decoded) == 0);
    puts("serial framing: wire vector, CRC, bounds and 7680 round trips passed");
    return 0;
}
