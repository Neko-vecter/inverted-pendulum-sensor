# Sensor UDP Protocol, Version 2

This document defines the UDP sample and time-synchronization datagrams between
each ESP32-C6 sensor node and the Rust Sensor Server.

## Transport

| Setting | Value |
|---|---|
| Network | Wi-Fi station |
| Transport | UDP over IPv4 |
| Destination | Server IPv4 and UDP port from firmware configuration |
| Default destination port | 5000 |
| Sample target | 1,000 datagrams per second per node |
| Version 1 sample payload | 26 bytes; accepted for compatibility, no latency estimate |
| Version 2 sample payload | 46 bytes |
| Byte order | Big-endian for multi-byte integers |
| Retransmission | None |

Version 2 sends one sample datagram per sampling slot and a small time-sync
request/response about once per second. UDP remains best effort. Queue overflow
and Wi-Fi outages can produce sequence gaps; samples are not replayed afterward.

## Common header

Every datagram begins with this four-byte header:

| Offset | Length | Field | Encoding |
|---:|---:|---|---|
| 0 | 2 | magic | Unsigned 16-bit big-endian; 0x4950 (IP) |
| 2 | 1 | version | Protocol version: 1 or 2 |
| 3 | 1 | packet_type | Sample 0x01, time-sync request 0x02, response 0x03 |

Version 1 defines only 26-byte sample datagrams. Version 2 defines 46-byte
sample datagrams and the time-sync messages below. Receivers reject unknown
magic, versions, packet types, and lengths.

## Version 2 sample frame

The packet is encoded field by field; it is not a native C structure and has no
compiler-dependent padding.

| Offset | Length | Field | Encoding |
|---:|---:|---|---|
| 0 | 2 | magic | 0x4950 |
| 2 | 1 | version | 2 |
| 3 | 1 | packet_type | Sample 0x01 |
| 4 | 6 | device_mac | Raw Wi-Fi station MAC octets, display order |
| 10 | 4 | sequence | Unsigned 32-bit big-endian |
| 14 | 8 | timestamp_us | ESP32 monotonic microseconds at start of MT6701 I2C read |
| 22 | 2 | raw_angle | Unsigned 16-bit; valid range 0–16,383 |
| 24 | 1 | status | Status field described below |
| 25 | 1 | reserved | Zero; receiver ignores it |
| 26 | 8 | sync_session_id | Server session identifier; zero means unsynchronized |
| 34 | 8 | clock_offset_us | Signed 64-bit two's complement; server clock minus device clock |
| 42 | 4 | sync_round_trip_us | Round-trip time from the latest accepted sync exchange |

Total version 2 sample length is **46 bytes**. The sample timestamp remains in
the device's monotonic clock domain. The server uses the supplied offset to map
it into the current server session's monotonic clock domain.

### Device identity and mapping

device_mac is the six-byte Wi-Fi station MAC read from the ESP32-C6. For
example, AA:BB:CC:DD:EE:FF is transmitted as AA BB CC DD EE FF. The MAC is
inside the payload because an IP receiver cannot use the original Wi-Fi link
source MAC after packets pass through an access point.

The firmware has no per-node numeric ID. The server maps each MAC to a logical
sensor name or role in its configuration file. Unknown MAC addresses are
counted and are not assigned to a configured device.

### Sequence

sequence starts at zero for the first sampling task wake after boot and
advances for each subsequent 1 ms sampling slot, including slots whose sample
is not transmitted. It wraps modulo 2^32. Samples omitted during a Wi-Fi outage,
when the sampler is late, or when the local queue is full appear as gaps. The
server's loss estimate cannot distinguish network loss from samples the
firmware omitted before transmission.

### Timestamp and latency

timestamp_us is esp_timer_get_time() in microseconds, captured immediately
before the firmware starts the MT6701 I2C read. It is monotonic, local to that
ESP32 boot, and is not a Unix timestamp. The latency metric starts at this
timestamp and does not model or subtract MT6701 internal measurement delay.

For a synchronized sample whose sync_session_id matches the server's current
session, the server estimates sample-to-receive latency as:

    latency_us = server_receive_monotonic_us
               - (timestamp_us + clock_offset_us)

server_receive_monotonic_us is captured when the server's recv_from call
returns. This measures from the firmware timestamp point through I2C
acquisition, firmware queueing, UDP/Wi-Fi delivery, and server receive
scheduling. Clock-offset error and path asymmetry limit one-way latency
accuracy. Samples without a valid current-session offset, or with a negative
computed latency, are excluded from percentiles and counted separately.

The server reports P50, P75, P90, and P99 per configured device and statistics
interval, using the nearest-rank percentile definition. It retains at most
50,000 latency samples per device per interval; larger intervals use a bounded
reservoir sample, while the log reports both the total and retained sample
counts. Version 1 samples have no sync metadata and do not contribute to
latency percentiles.

### Angle and status

The MT6701 provides a 14-bit absolute angle. The firmware reads register 0x03
then 0x04, and combines Angle[13:6] from the first byte with Angle[5:0] from
bits 7 through 2 of the second byte. Convert the transmitted integer to degrees:

    degrees = raw_angle * 360.0 / 16384.0

| Bit | Mask | Meaning |
|---:|---:|---|
| 0 | 0x01 | Angle valid; both I2C reads succeeded |
| 1 | 0x02 | MT6701 I2C read failed |
| 2–7 | — | Reserved; zero |

The sender emits exactly 0x01 on success and 0x02 on an I2C failure. For
status 0x01, raw_angle is 0–16,383. For status 0x02, raw_angle is 0xFFFF.
Receivers reject other status values, including both defined bits being set.

## Time synchronization messages

The device and server exchange four timestamps over the same configured UDP
port. Only MAC addresses present in the server's device mapping receive a
response.

### Request: device to server

| Offset | Length | Field | Encoding |
|---:|---:|---|---|
| 0 | 4 | Common header | Version 2, type 0x02 |
| 4 | 6 | device_mac | Raw station MAC octets |
| 10 | 4 | sync_sequence | Unsigned 32-bit big-endian |
| 14 | 8 | device_t1_us | Device monotonic time captured immediately before request encoding/send |

Total request length is **22 bytes**.

### Response: server to device

| Offset | Length | Field | Encoding |
|---:|---:|---|---|
| 0 | 4 | Common header | Version 2, type 0x03 |
| 4 | 6 | device_mac | Echoed station MAC |
| 10 | 4 | sync_sequence | Echoed request sequence |
| 14 | 8 | device_t1_us | Echoed device timestamp |
| 22 | 8 | server_t2_us | Server monotonic time when request was received |
| 30 | 8 | server_t3_us | Server monotonic time captured immediately before response encoding/send |
| 38 | 8 | server_session_id | Nonzero ID that changes when the server process restarts |

Total response length is **46 bytes**. On receipt of the matching response, the
device records device_t4_us from its monotonic clock and estimates:

    round_trip_us = (device_t4_us - device_t1_us) - (server_t3_us - server_t2_us)
    clock_offset_us = ((server_t2_us - device_t1_us)
                     + (server_t3_us - device_t4_us)) / 2

The offset convention is server clock minus device clock; adding it to
timestamp_us maps the sample into server time. Firmware refreshes synchronization
periodically and marks it unavailable after responses become stale. A zero
session ID means the sample is unsynchronized. After server restart, the session
ID changes, preventing an old offset from producing valid-looking latency data.

## Version 1 compatibility

The server accepts version 1 sample datagrams of exactly 26 bytes with the
original fields and offsets. Version 1 timestamps are device-local monotonic
values and are not latency-measured. New firmware sends version 2.

## Scope

The protocol defines sample reports and time synchronization only. It has no
acknowledgements, remote configuration, or start/stop commands.
