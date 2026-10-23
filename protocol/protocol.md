# Sensor Protocol, Version 3

This document defines the only supported wire protocol between ESP32 sensor nodes and the Rust Sensor Server. Packets travel either through ESP-NOW and the USB serial gateway or directly over Wi-Fi / UDP. Receivers accept version 3 only; version 2 packets are rejected.

## Transport

| Setting | Value |
| --- | --- |
| Default transport | ESP-NOW to gateway, then framed serial |
| Optional transport | Wi-Fi / UDP over IPv4 |
| ESP-NOW channel | 1 by default; sensor and gateway must match |
| Sample target | 1,000 packets/s per node |
| Sample packet | 24 bytes |
| Sync request | 18 bytes |
| Sync response | 30 bytes |
| Sync completion report | 26 bytes |
| Byte order | Big-endian for multi-byte fields |
| Retransmission | None |

ESP-NOW uses unencrypted broadcast on a fixed channel. Broadcast submission is not a receiver acknowledgement. The gateway checks each sensor packet's embedded MAC against its radio source MAC before forwarding it. One gateway/server should be active per channel.

### Serial framing

The bidirectional serial envelope is:

```text
00 | COBS(payload | CRC16_BE) | 00
```

CRC-16/CCITT-FALSE covers the payload: polynomial `0x1021`, initial value `0xFFFF`, no reflection, no final XOR, checksum high byte first. For `123456789`, the CRC is `0x29B1`. Payload `49 00 50` encodes to `00 02 49 04 50 15 55 00`.

The largest protocol payload is the 30-byte sync response. The maximum encoded frame is 35 bytes including delimiters. Receivers buffer across reads, process multiple frames per read, reject malformed, CRC-invalid, or oversized frames, and recover at the next zero delimiter. Application logs must use a separate console.

## Common header

Every packet begins with this four-byte header:

| Offset | Length | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 2 | magic | Unsigned 16-bit big-endian, `0x4950` |
| 2 | 1 | version | `3` |
| 3 | 1 | packet_type | Sample `0x01`, sync request `0x02`, sync response `0x03`, sync completion report `0x04` |

Unknown magic, versions, packet types, or packet lengths are rejected. No V2 compatibility path is supported.

## Sample packet

The sample packet is encoded field by field, without compiler padding.

| Offset | Length | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 2 | magic | `0x4950` |
| 2 | 1 | version | `3` |
| 3 | 1 | packet_type | Sample `0x01` |
| 4 | 6 | device_mac | Raw station MAC octets |
| 10 | 4 | sequence | Unsigned 32-bit sampling-slot counter |
| 14 | 4 | timestamp_us | Unsigned 32-bit wrapping device microseconds |
| 18 | 2 | angle_status | State in the upper two bits; raw angle in the lower 14 bits |
| 20 | 4 | sync_token | Current server-process token; zero means unsynchronized |

Total sample length is **24 bytes**.

The MT6701 angle is a 14-bit unsigned count. For a valid reading, `angle_status[15:14]` is `00` and `angle_status[13:0]` contains 0 through 16,383. Convert it to degrees with `raw_angle * 360.0 / 16384.0`. State `01` means I2C error and requires all lower angle bits to be zero. States `10` and `11` are reserved and rejected.

The MAC identifies the sensor and maps to a logical device name in server configuration. The 32-bit sequence begins at zero after boot and advances for each sampling slot, including slots whose sample is not transmitted. The server uses modular differences to estimate packet loss; it cannot distinguish network loss from samples omitted by firmware.

`timestamp_us` is the low 32 bits of the ESP32 monotonic microsecond timer, captured immediately before the MT6701 I2C read. It wraps every 2^32 microseconds (about 71 minutes 35 seconds). The latency metric includes I2C acquisition after this timestamp, firmware queueing, transport and server receive scheduling; it does not model MT6701 internal measurement delay.

The nonzero 32-bit `sync_token` changes when the server process starts. The server stores each device's offset and RTT after a completed time-sync exchange. A sample contributes latency only when its token matches the server's current token and a valid offset has been recorded. Token zero or a stale token is counted as unsynchronized. This also prevents buffered samples from a previous server session from being interpreted using a new offset.

## Time synchronization

The server replies only to requests from configured device MACs. The exchange uses four timestamps: device `t1` before sending the request, server `t2` on request receipt, server `t3` immediately before responding, and device `t4` on response receipt. The sensor then sends a completion report so the server can calculate and retain the mapping. Exchanges occur about once per second.

### Request: sensor to server

| Offset | Length | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 4 | Header | Version 3, type `0x02` |
| 4 | 6 | device_mac | Raw station MAC octets |
| 10 | 4 | sync_sequence | Unsigned 32-bit sequence |
| 14 | 4 | device_t1_us | Device timestamp before request encoding/send |

Total request length is **18 bytes**.

### Response: server to sensor

| Offset | Length | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 4 | Header | Version 3, type `0x03` |
| 4 | 6 | device_mac | Echoed station MAC |
| 10 | 4 | sync_sequence | Echoed request sequence |
| 14 | 4 | device_t1_us | Echoed device timestamp |
| 18 | 4 | server_t2_us | Server timestamp when request was received |
| 22 | 4 | server_t3_us | Server timestamp before response encoding/send |
| 26 | 4 | sync_token | Current nonzero server-process token |

Total response length is **30 bytes**. The sensor accepts only a response matching its MAC, pending sequence, and request timestamp, and captures `device_t4_us` when the response arrives.

### Completion report: sensor to server

| Offset | Length | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 4 | Header | Version 3, type `0x04` |
| 4 | 6 | device_mac | Raw station MAC octets |
| 10 | 4 | sync_sequence | Echoed request sequence |
| 14 | 4 | device_t1_us | Echoed request timestamp |
| 18 | 4 | device_t4_us | Device timestamp when matching response arrived |
| 22 | 4 | sync_token | Token echoed from the response |

Total report length is **26 bytes**. The server accepts a report only if it matches the outstanding request for that MAC and the server's current token.

All time calculations use modulo-2^32 differences. Forward intervals must be less than 2^31 microseconds. The server computes:

```text
device_elapsed = (device_t4_us - device_t1_us) mod 2^32
server_elapsed = (server_t3_us - server_t2_us) mod 2^32
round_trip_us = device_elapsed - server_elapsed

offset_a = signed_modular_difference(server_t2_us, device_t1_us)
offset_b = signed_modular_difference(server_t3_us, device_t4_us)
clock_offset_us = (offset_a + offset_b) / 2
```

The server rejects exchanges with ambiguous elapsed intervals, a negative inferred network round trip, or RTT above one second. It smooths offset updates within a server session and records the latest accepted RTT per device.

## Latency calculation

For a sample whose `sync_token` matches the active server session and whose device has a valid offset:

```text
mapped_sample_us = (timestamp_us + clock_offset_us) mod 2^32
latency_us = (server_receive_us - mapped_sample_us) mod 2^32
```

`server_receive_us` is captured when the transport returns the packet, after UDP receipt or serial frame decoding. Values at least 2^31 microseconds are treated as clock-ahead or ambiguous and excluded. Offset error and path asymmetry limit one-way latency accuracy.

The server reports P50, P75, P90, and P99 per configured device and statistics interval using nearest-rank percentiles. It retains at most 50,000 latency samples per device per interval; larger intervals use a bounded reservoir sample.

## Scope

The protocol defines sensor samples and clock synchronization only. It has no acknowledgements, remote configuration, or start/stop commands.
