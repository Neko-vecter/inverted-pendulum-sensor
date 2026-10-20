# Sensor Protocol, Version 2

This document defines the wire format between each ESP32 sensor and the Rust Sensor Server, carried either by ESP-NOW through a serial gateway or directly by Wi-Fi / UDP. Version 2 uses 32-bit wrapping microsecond counters for device and server time.

## Transport

| Setting                  | Value |
| ------------------------ | ----- |
| Default transport        | ESP-NOW to gateway, then framed serial to server |
| Optional transport       | Wi-Fi / UDP over IPv4 |
| ESP-NOW channel          | 1 by default; sensor and gateway must match |
| UDP destination          | Configured server IPv4 address and port (UDP mode only) |
| UDP port                 | 5000 by default (UDP mode only) |
| Sample target            | 1,000 packets per second per node |
| Sample packet length     | 38 bytes |
| Byte order               | Big-endian for multi-byte fields |
| Retransmission           | None |

The device sends one sample packet per sampling slot and a time-sync request/response about once per second. Both UDP and ESP-NOW are best effort; queue overflow, radio loss, or Wi-Fi outages can produce sequence gaps. Samples are not replayed later.

### ESP-NOW and serial gateway

ESP-NOW is the default firmware transport. Sensors and the gateway use the same fixed 2.4 GHz channel (default 1), without associating with an access point. All packets use unencrypted ESP-NOW broadcast. There is one active gateway/server per channel; broadcast success is not a receiver acknowledgement.

Sensor-to-gateway radio payloads are exactly the 38-byte sample or 18-byte sync request defined below. The gateway checks that the embedded sensor MAC matches the radio source MAC. Server-to-gateway serial payloads are 34-byte sync responses. The gateway broadcasts these responses, and each sensor accepts only replies matching its MAC, pending sequence and request timestamp.

The bidirectional serial envelope is:

```text
00 | COBS(payload | CRC16_BE) | 00
```

CRC-16/CCITT-FALSE covers the payload: polynomial `0x1021`, initial `0xFFFF`, no reflection, no final XOR, checksum transmitted high byte first. For the check string `123456789`, CRC is `0x29B1`. Payload `49 00 50` encodes to `00 02 49 04 50 15 55 00` (a framing test vector, not a sensor packet).

Maximum payload length is 38 bytes, encoded length is at most 41 bytes, and the complete envelope is at most 43 bytes. Empty delimiters are ignored. Receivers buffer across reads, process multiple frames in one read, bound their buffer sizes, discard malformed/CRC-invalid/oversized frames and recover at the next zero delimiter. Application logs must use a separate console.

The gateway defaults to native USB Serial/JTAG. Optional UART uses 8N1, no flow control and 921600 baud by default; three nodes at 1 kHz need a faster UART or native USB. Serial framing adds five bytes per packet. Sensor samples and sync requests pass to the same server decoder, MAC mapping, state and statistics used for UDP. Sync responses traverse serial and radio in reverse. Latency measures sample-to-host reception including gateway and serial buffering, and is subject to sync-path asymmetry.

These projects build independently and implement this contract locally. This directory contains documentation only.

## Common header

Every protocol packet begins with this four-byte header:

| Offset | Length | Field       | Encoding                                           |
| -----: | -----: | ----------- | -------------------------------------------------- |
|      0 |      2 | magic       | Unsigned 16-bit big-endian; 0x4950                 |
|      2 |      1 | version     | 2                                                  |
|      3 |      1 | packet_type | Sample 0x01, time-sync request 0x02, response 0x03 |

Receivers accept only version 2 and the packet lengths in this document. The earlier 46-byte version 2 sample layout is not supported. Packets with unknown magic, versions, types, or lengths are rejected.

## Sample frame

The packet is encoded field by field. It is not a native C structure and has no compiler-dependent padding.

| Offset | Length | Field              | Encoding                                             |
| -----: | -----: | ------------------ | ---------------------------------------------------- |
|      0 |      2 | magic              | 0x4950                                               |
|      2 |      1 | version            | 2                                                    |
|      3 |      1 | packet_type        | Sample 0x01                                          |
|      4 |      6 | device_mac         | Raw sensor station MAC octets, display order         |
|     10 |      4 | sequence           | Unsigned 32-bit big-endian sample-slot counter       |
|     14 |      4 | timestamp_us       | Unsigned 32-bit wrapping device microsecond counter  |
|     18 |      2 | raw_angle          | Unsigned 16-bit; valid range 0-16,383                |
|     20 |      1 | status             | Status field described below                         |
|     21 |      1 | reserved           | Zero                                                 |
|     22 |      8 | sync_session_id    | Server session identifier; zero means unsynchronized |
|     30 |      4 | clock_offset_us    | Unsigned 32-bit modular server-minus-device offset   |
|     34 |      4 | sync_round_trip_us | Latest accepted sync round-trip time in microseconds |

Total sample length is **38 bytes**.

### Device identity and mapping

device_mac is the six-byte station MAC read from the ESP32 sensor. For example, AA:BB:CC:DD:EE:FF is transmitted as AA BB CC DD EE FF. The embedded MAC identifies the sensor on both transports. It is necessary in UDP mode because an IP receiver cannot use the original Wi-Fi link source MAC after packets pass through an access point; the gateway also checks it against the ESP-NOW source MAC.

The firmware has no per-node numeric ID. The server maps each MAC to a logical sensor name or role in its configuration file. Unknown MAC addresses are counted and are not assigned to a configured device.

### Sequence counter

sequence starts at zero for the first sampling task wake after boot and advances for each subsequent 1 ms sampling slot, including slots whose sample is not transmitted. It wraps modulo 2^32. This counter is independent of timestamp_us. The server uses modular sequence differences to estimate gaps; the estimate cannot distinguish network loss from samples omitted by firmware before transmission.

### Device timestamp and counter wrap

timestamp_us is the low 32 bits of esp_timer_get_time(), captured immediately before the firmware starts the MT6701 I2C read. It is monotonic within a boot, local to that ESP32, and is not a Unix timestamp. It wraps every 2^32 microseconds (about 71 minutes 35 seconds).

All comparisons between wrapping counters use modulo-2^32 differences. A forward interval is unambiguous only when it is less than 2^31 microseconds (about 35 minutes 47 seconds). The firmware rejects sync exchanges whose measured device or server intervals are at least 2^31 microseconds. In normal operation, sync requests are sent about once per second and accepted RTT is at most 1 second.

The server's time origin is Instant::now() captured at the beginning of main, before command-line and configuration processing. Server timestamps are elapsed microseconds from that origin, truncated modulo 2^32 for the wire protocol. Internal statistics continue to use monotonic Instant durations.

The latency metric starts at the device timestamp and does not model or subtract MT6701 internal measurement delay.

### Angle and status

The MT6701 provides a 14-bit absolute angle. The firmware reads register 0x03 then 0x04, and combines Angle[13:6] from the first byte with Angle[5:0] from bits 7 through 2 of the second byte. Convert the transmitted integer to degrees:

```
degrees = raw_angle * 360.0 / 16384.0
```

|  Bit | Mask | Meaning                               |
| ---: | ---: | ------------------------------------- |
|    0 | 0x01 | Angle valid; both I2C reads succeeded |
|    1 | 0x02 | MT6701 I2C read failed                |
|  2-7 |    - | Reserved; zero                        |

The sender emits exactly 0x01 on success and 0x02 on an I2C failure. For status 0x01, raw_angle is 0-16,383. For status 0x02, raw_angle is 0xFFFF. Receivers reject other status values, including both defined bits being set.

## Time synchronization messages

The device and server exchange four timestamps over the selected transport. Only MAC addresses present in the server's device mapping receive a response.

### Request: device to server

| Offset | Length | Field         | Encoding                                             |
| -----: | -----: | ------------- | ---------------------------------------------------- |
|      0 |      4 | Common header | Version 2, type 0x02                                 |
|      4 |      6 | device_mac    | Raw station MAC octets                               |
|     10 |      4 | sync_sequence | Unsigned 32-bit big-endian                           |
|     14 |      4 | device_t1_us  | Device wrapping counter before request encoding/send |

Total request length is **18 bytes**.

### Response: server to device

| Offset | Length | Field             | Encoding                                                 |
| -----: | -----: | ----------------- | -------------------------------------------------------- |
|      0 |      4 | Common header     | Version 2, type 0x03                                     |
|      4 |      6 | device_mac        | Echoed station MAC                                       |
|     10 |      4 | sync_sequence     | Echoed request sequence                                  |
|     14 |      4 | device_t1_us      | Echoed device timestamp                                  |
|     18 |      4 | server_t2_us      | Server wrapping counter when request was received        |
|     22 |      4 | server_t3_us      | Server wrapping counter before response encoding/send    |
|     26 |      8 | server_session_id | Nonzero ID that changes when the server process restarts |

Total response length is **34 bytes**. On receipt of the matching response, the device records device_t4_us from its wrapping counter. All subtractions below are modulo 2^32; signed offset deltas use the nearest signed interpretation in the range [-2^31, 2^31-1]:

```
device_elapsed = (device_t4_us - device_t1_us) mod 2^32
server_elapsed = (server_t3_us - server_t2_us) mod 2^32
round_trip_us = device_elapsed - server_elapsed

offset_a = signed_modular_difference(server_t2_us, device_t1_us)
offset_b = signed_modular_difference(server_t3_us, device_t4_us)
clock_offset_us = (offset_a + offset_b) / 2, represented modulo 2^32
```

The offset convention is server clock minus device clock. Adding it to a device timestamp maps that timestamp into the server's wrapping clock domain. The firmware smooths offset updates within a server session and replaces the estimate when the session ID changes. A zero session ID in a sample means the device is not synchronized. After server restart, a new session ID prevents an old offset from producing valid-looking latency data.

## Latency calculation

For a sample whose sync_session_id matches the server's current session, the server calculates the sample-to-receive delay using 32-bit modular arithmetic:

```
mapped_sample_us = (timestamp_us + clock_offset_us) mod 2^32
latency_us = (server_receive_us - mapped_sample_us) mod 2^32
```

server_receive_us is captured when the server transport returns the packet, after a UDP receive or serial frame read. The server accepts the latency only when it is less than 2^31 microseconds; a larger value is treated as a clock-ahead or ambiguous result and excluded from percentiles. The metric includes I2C acquisition after the timestamp point, firmware queueing, the selected radio/network path, gateway and serial buffering in ESP-NOW mode, and server receive scheduling. Clock offset error and path asymmetry limit one-way latency accuracy.

The server reports P50, P75, P90, and P99 per configured device and statistics interval, using the nearest-rank percentile definition. It retains at most 50,000 latency samples per device per interval; larger intervals use a bounded reservoir sample, while the log reports both total and retained sample counts.

## Scope

The protocol defines sample reports and time synchronization only. It has no acknowledgements, remote configuration, or start/stop commands.
