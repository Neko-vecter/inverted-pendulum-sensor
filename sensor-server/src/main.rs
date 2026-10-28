mod config;
mod protocol;
mod serial_frame;
mod state;
mod transport;
mod websocket;

use crate::config::{LogLevel, ServerConfig};
use crate::protocol::{
    DecodedDatagram, decode_datagram, encode_registration_response, encode_time_sync_response,
};
use crate::state::{
    DeviceRegistry, DeviceState, ServerMetrics, format_rate, latency_percentiles_us,
};
use crate::transport::Transport;
use crate::websocket::{WebSocketHub, start_from_registry};
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, trace, warn};

fn main() -> ExitCode {
    let server_start = Instant::now();
    let config_path = match config_path_from_args() {
        Ok(Some(path)) => path,
        Ok(None) => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("error: {error}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    let config = match config::load(&config_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("sensor-server: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (_logging_guard, log_error_counter) = init_logging(config.log_level);

    match run(config, server_start, log_error_counter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(%error, "server stopped");
            ExitCode::FAILURE
        }
    }
}

fn init_logging(
    level: LogLevel,
) -> (
    tracing_appender::non_blocking::WorkerGuard,
    tracing_appender::non_blocking::ErrorCounter,
) {
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(2048)
        .lossy(true)
        .finish(std::io::stderr());
    let error_counter = writer.error_counter();
    let max_level = match level {
        LogLevel::Error => tracing_subscriber::filter::LevelFilter::ERROR,
        LogLevel::Warn => tracing_subscriber::filter::LevelFilter::WARN,
        LogLevel::Info => tracing_subscriber::filter::LevelFilter::INFO,
        LogLevel::Debug => tracing_subscriber::filter::LevelFilter::DEBUG,
        LogLevel::Trace => tracing_subscriber::filter::LevelFilter::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(max_level)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .expect("failed to initialize logging subscriber");
    (guard, error_counter)
}

fn config_path_from_args() -> Result<Option<PathBuf>, String> {
    let mut args = env::args().skip(1);
    let mut config_path = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(None),
            "-c" | "--config" => {
                if config_path.is_some() {
                    return Err("specify --config only once".to_owned());
                }
                let Some(path) = args.next() else {
                    return Err("--config requires a path".to_owned());
                };
                config_path = Some(PathBuf::from(path));
            }
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    Ok(Some(
        config_path.unwrap_or_else(|| PathBuf::from("config.toml")),
    ))
}

fn print_usage() {
    println!("Usage: sensor-server [--config PATH]\n       sensor-server --help");
    println!("Default config path: config.toml in the current directory");
}

fn run(
    config: ServerConfig,
    server_start: Instant,
    log_error_counter: tracing_appender::non_blocking::ErrorCounter,
) -> Result<(), String> {
    let session_entropy = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    let server_sync_token = ((session_entropy as u32) ^ ((session_entropy >> 32) as u32)).max(1);
    let mut transport = Transport::open(&config.transport)?;
    info!(
        "protocol v3, {} configured device(s)",
        config.devices_by_mac.len()
    );
    info!(
        time_sync_token = server_sync_token,
        "time synchronization initialized"
    );
    info!(
        stats_interval_secs = config.stats_interval.as_secs(),
        "statistics interval configured"
    );
    info!("server started; press Ctrl+C to stop");

    let mut devices = DeviceRegistry::new(config.devices_by_mac, server_sync_token);
    let websocket = start_from_registry(&devices, server_start, config.websocket_bind_address)?;
    info!(
        websocket_bind_address = %config.websocket_bind_address,
        "local WebSocket API listening"
    );
    let mut metrics = ServerMetrics::default();
    let mut interval_started = Instant::now();
    let mut dropped_log_lines = 0;

    loop {
        match transport.receive() {
            Ok(Some(packet)) => {
                let received_at = packet.received_at;
                let server_now_us = monotonic_us(server_start, received_at) as u32;
                handle_datagram(
                    &packet.data,
                    packet.sender,
                    received_at,
                    server_now_us,
                    server_start,
                    server_sync_token,
                    &mut transport,
                    &mut devices,
                    &mut metrics,
                    &websocket,
                );
            }
            Ok(None) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                metrics.record_receive_error();
                if matches!(transport, Transport::Serial { .. }) {
                    return Err(format!(
                        "ESP-NOW serial receive failed: {error}; reconnect the gateway and restart"
                    ));
                }
                warn!(%error, "UDP receive error");
                thread::sleep(Duration::from_millis(100));
            }
        }

        let now = Instant::now();
        let elapsed = now.duration_since(interval_started);
        if elapsed >= config.stats_interval {
            log_stats(elapsed, now, &mut devices, &mut metrics);
            let total_dropped_log_lines = log_error_counter.dropped_lines();
            if total_dropped_log_lines > dropped_log_lines {
                warn!(
                    dropped_log_lines = total_dropped_log_lines - dropped_log_lines,
                    dropped_log_lines_total = total_dropped_log_lines,
                    "non-blocking log queue dropped output"
                );
                dropped_log_lines = total_dropped_log_lines;
            }
            if matches!(transport, Transport::Serial { .. }) {
                info!(
                    serial_framing_errors_total = transport.framing_errors(),
                    "serial framing statistics"
                );
            }
            interval_started = now;
        }
    }
}

fn handle_datagram(
    packet: &[u8],
    sender: Option<SocketAddr>,
    received_at: Instant,
    server_now_us: u32,
    server_start: Instant,
    server_sync_token: u32,
    transport: &mut Transport,
    devices: &mut DeviceRegistry,
    metrics: &mut ServerMetrics,
    websocket: &WebSocketHub,
) {
    let datagram = match decode_datagram(packet) {
        Ok(datagram) => datagram,
        Err(error) => {
            metrics.record_malformed(error);
            return;
        }
    };

    trace!(
        packet_type = packet[3],
        packet_bytes = packet.len(),
        sender = ?sender,
        "valid protocol packet received"
    );

    match datagram {
        DecodedDatagram::Sample(sample) => {
            metrics.record_decoded();
            if devices.record(sample, received_at, server_now_us) {
                metrics.record_mapped();
                if let Some(name) = devices.name_for_id(sample.device_id)
                    && let Some(latest) = devices.by_name.get(name).and_then(|state| state.latest)
                    && latest.received_at == received_at
                {
                    websocket.publish(name, latest.sample.angle_degrees(), received_at);
                }
            } else {
                metrics.record_unknown(sample.device_id);
            }
        }
        DecodedDatagram::RegistrationRequest(request) => {
            let Some(device_id) = devices.id_for_mac(request.device_mac) else {
                debug!(sensor_mac = %request.device_mac, "registration request from unconfigured sensor");
                return;
            };
            debug!(sensor_mac = %request.device_mac, device_id, "sensor registration requested");
            let response = encode_registration_response(request, device_id);
            if let Err(error) = transport.respond(&response, sender) {
                warn!(%error, "registration response failed");
            }
        }
        DecodedDatagram::TimeSyncRequest(request) => {
            if !devices.contains_id(request.device_id) {
                return;
            }
            let server_t3_us = monotonic_us(server_start, Instant::now()) as u32;
            devices.begin_sync(request, server_now_us, server_t3_us);
            let response =
                encode_time_sync_response(request, server_now_us, server_t3_us, server_sync_token);
            if let Err(error) = transport.respond(&response, sender) {
                warn!(%error, "time-sync response failed");
            }
        }
        DecodedDatagram::TimeSyncReport(report) => {
            let _ = devices.complete_sync(report);
        }
    }
}

fn monotonic_us(start: Instant, now: Instant) -> u64 {
    now.saturating_duration_since(start)
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

fn log_stats(
    elapsed: Duration,
    now: Instant,
    devices: &mut DeviceRegistry,
    metrics: &mut ServerMetrics,
) {
    let interval = metrics.take_interval();
    info!(
        elapsed_secs = elapsed.as_secs_f64(),
        samples = interval.decoded,
        mapped = interval.mapped,
        unknown = interval.unknown,
        malformed = interval.malformed.values().sum::<u64>(),
        receive_errors_total = metrics.receive_errors_total,
        decoded_total = metrics.decoded_total,
        mapped_total = metrics.mapped_total,
        unknown_total = metrics.unknown_total,
        malformed_total = metrics.malformed_total,
        "server statistics"
    );

    if !interval.malformed.is_empty() {
        let details = interval
            .malformed
            .iter()
            .map(|(kind, count)| format!("{}={count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        warn!(malformed = %details, "rejected malformed packets");
    }
    for (id, count) in interval.unknown_by_id {
        warn!(
            device_id = format_args!("0x{id:04X}"),
            count, "unassigned device ID received"
        );
    }
    if interval.unknown_unlisted > 0 {
        warn!(
            count = interval.unknown_unlisted,
            "additional datagrams from unlisted device IDs"
        );
    }

    for state in devices.by_name.values_mut() {
        log_device_stats(state, elapsed, now);
    }
}

fn log_device_stats(state: &mut DeviceState, elapsed: Duration, now: Instant) {
    let interval_received = state.take_interval_received();
    let rate = format_rate(interval_received, elapsed);
    let latency = state.take_latency_interval();
    let latency_total_samples = latency.total_samples;
    let percentile_sample_count = latency.samples_us.len();
    let sync_rtt = latency
        .latest_sync_rtt_us
        .map_or_else(|| "n/a".to_owned(), |rtt| rtt.to_string());
    let latency_unsynced = latency.unsynced;
    let latency_clock_ahead = latency.clock_ahead;
    let latency_percentiles = latency_percentiles_us(latency.samples_us);
    let latency_summary = match latency_percentiles {
        Some([p50, p75, p90, p99]) => format!(
            "latency_ms p50={:.3} p75={:.3} p90={:.3} p99={:.3} samples={} percentile_samples={} sync_rtt_us={} unsynced={} clock_ahead={}",
            p50 as f64 / 1000.0,
            p75 as f64 / 1000.0,
            p90 as f64 / 1000.0,
            p99 as f64 / 1000.0,
            latency_total_samples,
            percentile_sample_count,
            sync_rtt,
            latency_unsynced,
            latency_clock_ahead
        ),
        None => format!(
            "latency_ms n/a samples={} sync_rtt_us={} unsynced={} clock_ahead={}",
            latency_total_samples, sync_rtt, latency_unsynced, latency_clock_ahead
        ),
    };
    let loss = state
        .loss_percent()
        .map(|percent| format!("{percent:.3}%"))
        .unwrap_or_else(|| "n/a".to_owned());

    let latest = match state.latest {
        Some(latest) => {
            let age_ms = now
                .saturating_duration_since(latest.received_at)
                .as_millis();
            match latest.sample.angle_degrees() {
                Some(degrees) => format!(
                    "angle={degrees:.3}deg raw={} seq={} ts={}us age={}ms",
                    latest.sample.raw_angle,
                    latest.sample.sequence,
                    latest.sample.timestamp_us,
                    age_ms
                ),
                None => format!(
                    "angle=invalid(i2c_error) seq={} ts={}us age={}ms",
                    latest.sample.sequence, latest.sample.timestamp_us, age_ms
                ),
            }
        }
        None => "latest=none".to_owned(),
    };

    info!(
        sensor = %state.logical_name,
        mac = %state.mac,
        rx_per_sec = rate,
        total = state.received_total,
        seq_gaps = state.sequence_gaps,
        lost_est = state.estimated_lost,
        loss = %loss,
        duplicates = state.duplicates,
        out_of_order = state.out_of_order,
        restarts = state.restarts,
        i2c_errors = state.sensor_errors,
        latency = %latency_summary,
        latest = %latest,
        "sensor statistics"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MacAddress;
    use std::collections::HashMap;
    use std::net::UdpSocket;

    #[test]
    fn udp_sync_exchange_and_framed_sample_use_existing_device_state() {
        let mac = MacAddress::from_octets([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 1]);
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let address = socket.local_addr().unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut transport = Transport::Udp(socket);
        let start = Instant::now();
        let mut devices = DeviceRegistry::new(HashMap::from([(mac, "sensor_1".to_owned())]), 7);
        let websocket = WebSocketHub::new(["sensor_1".to_owned()], start);
        let mut metrics = ServerMetrics::default();
        let mut registration = vec![0x49, 0x50, 3, 5];
        registration.extend(mac.octets());
        registration.extend(7_u32.to_be_bytes());
        client.send_to(&registration, address).unwrap();
        let received = transport.receive().unwrap().unwrap();
        handle_datagram(
            &received.data,
            received.sender,
            received.received_at,
            0,
            start,
            7,
            &mut transport,
            &mut devices,
            &mut metrics,
            &websocket,
        );
        let mut assigned = [0; 64];
        let assigned_size = client.recv(&mut assigned).unwrap();
        assert_eq!(assigned_size, protocol::REGISTRATION_RESPONSE_SIZE);
        assert_eq!(&assigned[14..16], &0_u16.to_be_bytes());
        let id = 0_u16;
        let mut request = vec![0x49, 0x50, 3, 2];
        request.extend(id.to_be_bytes());
        request.extend(42_u32.to_be_bytes());
        request.extend(1000_u32.to_be_bytes());
        client.send_to(&request, address).unwrap();
        let received = transport.receive().unwrap().unwrap();
        let server_t2 = monotonic_us(start, received.received_at) as u32;
        handle_datagram(
            &received.data,
            received.sender,
            received.received_at,
            server_t2,
            start,
            7,
            &mut transport,
            &mut devices,
            &mut metrics,
            &websocket,
        );
        let mut response = [0; 64];
        let size = client.recv(&mut response).unwrap();
        assert_eq!(size, protocol::TIME_SYNC_RESPONSE_SIZE);
        assert_eq!(
            &response[..14],
            &[&request[..3], &[3], &request[4..]].concat()
        );
        let response_t2 = u32::from_be_bytes(response[14..18].try_into().unwrap());
        let response_t3 = u32::from_be_bytes(response[18..22].try_into().unwrap());
        assert_eq!(response_t2, server_t2);
        assert_eq!(&response[22..26], &7_u32.to_be_bytes());

        let device_t4 = 1000_u32
            .wrapping_add(response_t3.wrapping_sub(response_t2))
            .wrapping_add(200);
        let mut report = vec![0x49, 0x50, 3, 4];
        report.extend(id.to_be_bytes());
        report.extend(42_u32.to_be_bytes());
        report.extend(1000_u32.to_be_bytes());
        report.extend(device_t4.to_be_bytes());
        report.extend(7_u32.to_be_bytes());
        handle_datagram(
            &report,
            Some(client.local_addr().unwrap()),
            Instant::now(),
            server_t2.wrapping_add(100),
            start,
            7,
            &mut transport,
            &mut devices,
            &mut metrics,
            &websocket,
        );

        // A gateway-framed sample reaches the same state/latency path as UDP.
        let mut sample = vec![0x49, 0x50, 3, 1];
        sample.extend(id.to_be_bytes());
        sample.extend(1_u32.to_be_bytes());
        sample.extend(1000_u32.to_be_bytes());
        sample.extend(4096_u16.to_be_bytes());
        sample.extend(7_u32.to_be_bytes());
        let mut decoder = serial_frame::Decoder::default();
        let packet = serial_frame::encode(&sample)
            .into_iter()
            .find_map(|byte| decoder.push(byte))
            .unwrap();
        handle_datagram(
            &packet,
            None,
            Instant::now(),
            server_t2.wrapping_add(100),
            start,
            7,
            &mut transport,
            &mut devices,
            &mut metrics,
            &websocket,
        );
        let state = &devices.by_name["sensor_1"];
        assert_eq!(state.received_total, 1);
        assert_eq!(state.latest.unwrap().sample.angle_degrees(), Some(90.0));
        assert_eq!(state.latency_samples_us, [200]);
        assert_eq!(metrics.mapped_total, 1);
    }
}
