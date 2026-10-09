use crate::protocol::{DecodeError, MacAddress, SensorSample};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

const MAX_REPORTED_UNKNOWN_MACS: usize = 32;
const MAX_LATENCY_PERCENTILE_SAMPLES: usize = 50_000;

#[derive(Debug)]
pub struct DeviceState {
    pub logical_name: String,
    pub mac: MacAddress,
    pub latest: Option<LatestSample>,
    pub received_total: u64,
    pub interval_received: u64,
    pub sequence_slots: u64,
    pub sequence_gaps: u64,
    pub estimated_lost: u64,
    pub duplicates: u64,
    pub out_of_order: u64,
    pub restarts: u64,
    pub sensor_errors: u64,
    pub latency_samples_us: Vec<u64>,
    pub latency_total_samples: u64,
    pub latency_unsynced: u64,
    pub latency_clock_ahead: u64,
    pub latest_sync_rtt_us: Option<u32>,
    last_sequence: Option<u32>,
    last_timestamp_us: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct LatestSample {
    pub sample: SensorSample,
    pub received_at: Instant,
}

impl DeviceState {
    fn new(logical_name: String, mac: MacAddress) -> Self {
        Self {
            logical_name,
            mac,
            latest: None,
            received_total: 0,
            interval_received: 0,
            sequence_slots: 0,
            sequence_gaps: 0,
            estimated_lost: 0,
            duplicates: 0,
            out_of_order: 0,
            restarts: 0,
            sensor_errors: 0,
            latency_samples_us: Vec::new(),
            latency_total_samples: 0,
            latency_unsynced: 0,
            latency_clock_ahead: 0,
            latest_sync_rtt_us: None,
            last_sequence: None,
            last_timestamp_us: None,
        }
    }

    fn record(
        &mut self,
        sample: SensorSample,
        received_at: Instant,
        server_now_us: u64,
        server_session_id: u64,
    ) {
        self.received_total += 1;
        self.interval_received += 1;
        if sample.has_i2c_error() {
            self.sensor_errors += 1;
        }
        self.record_latency(sample, server_now_us, server_session_id);

        let mut advances_sequence = false;
        let mut is_restart = false;
        match (self.last_sequence, self.last_timestamp_us) {
            (None, _) => {
                self.sequence_slots += 1;
                advances_sequence = true;
            }
            (Some(_), Some(last_timestamp_us)) if sample.timestamp_us < last_timestamp_us => {
                self.restarts += 1;
                self.sequence_slots += 1;
                advances_sequence = true;
                is_restart = true;
            }
            (Some(last_sequence), Some(_)) => {
                let delta = sample.sequence.wrapping_sub(last_sequence);
                if delta == 0 {
                    self.duplicates += 1;
                } else if delta < (1_u32 << 31) {
                    self.sequence_slots += u64::from(delta);
                    if delta > 1 {
                        self.sequence_gaps += 1;
                        self.estimated_lost += u64::from(delta - 1);
                    }
                    advances_sequence = true;
                } else {
                    self.out_of_order += 1;
                }
            }
            _ => {
                self.sequence_slots += 1;
                advances_sequence = true;
            }
        }

        if advances_sequence {
            self.last_sequence = Some(sample.sequence);
            self.last_timestamp_us = Some(sample.timestamp_us);
            if is_restart || self.latest.is_none() {
                self.latest = Some(LatestSample {
                    sample,
                    received_at,
                });
            } else if self
                .latest
                .is_some_and(|latest| sample.timestamp_us >= latest.sample.timestamp_us)
            {
                self.latest = Some(LatestSample {
                    sample,
                    received_at,
                });
            }
        }
    }

    fn record_latency(
        &mut self,
        sample: SensorSample,
        server_now_us: u64,
        server_session_id: u64,
    ) {
        let Some(sync) = sample.clock_sync else {
            self.latency_unsynced += 1;
            return;
        };
        if sync.session_id != server_session_id {
            self.latency_unsynced += 1;
            return;
        }

        self.latest_sync_rtt_us = Some(sync.round_trip_us);
        let estimated_sample_server_us = i128::from(sample.timestamp_us)
            + i128::from(sync.offset_us);
        let latency_us = i128::from(server_now_us) - estimated_sample_server_us;
        if latency_us < 0 {
            self.latency_clock_ahead += 1;
        } else if let Ok(latency_us) = u64::try_from(latency_us) {
            self.latency_total_samples += 1;
            if self.latency_samples_us.len() < MAX_LATENCY_PERCENTILE_SAMPLES {
                self.latency_samples_us.push(latency_us);
            } else {
                let slot = reservoir_index(self.latency_total_samples);
                if slot < MAX_LATENCY_PERCENTILE_SAMPLES as u64 {
                    self.latency_samples_us[slot as usize] = latency_us;
                }
            }
        } else {
            self.latency_clock_ahead += 1;
        }
    }

    pub fn take_interval_received(&mut self) -> u64 {
        std::mem::take(&mut self.interval_received)
    }

    pub fn take_latency_interval(&mut self) -> LatencyInterval {
        LatencyInterval {
            samples_us: std::mem::take(&mut self.latency_samples_us),
            total_samples: std::mem::take(&mut self.latency_total_samples),
            unsynced: std::mem::take(&mut self.latency_unsynced),
            clock_ahead: std::mem::take(&mut self.latency_clock_ahead),
            latest_sync_rtt_us: self.latest_sync_rtt_us,
        }
    }

    pub fn loss_percent(&self) -> Option<f64> {
        (self.sequence_slots > 0)
            .then(|| self.estimated_lost as f64 * 100.0 / self.sequence_slots as f64)
    }
}

#[derive(Debug)]
pub struct DeviceRegistry {
    by_mac: HashMap<MacAddress, String>,
    pub by_name: BTreeMap<String, DeviceState>,
}

impl DeviceRegistry {
    pub fn new(devices_by_mac: HashMap<MacAddress, String>) -> Self {
        let by_name = devices_by_mac
            .iter()
            .map(|(mac, name)| (name.clone(), DeviceState::new(name.clone(), *mac)))
            .collect();
        Self {
            by_mac: devices_by_mac,
            by_name,
        }
    }

    pub fn record(
        &mut self,
        sample: SensorSample,
        received_at: Instant,
        server_now_us: u64,
        server_session_id: u64,
    ) -> bool {
        let Some(name) = self.by_mac.get(&sample.device_mac) else {
            return false;
        };
        if let Some(state) = self.by_name.get_mut(name) {
            state.record(sample, received_at, server_now_us, server_session_id);
            true
        } else {
            false
        }
    }

    pub fn contains_mac(&self, mac: MacAddress) -> bool {
        self.by_mac.contains_key(&mac)
    }
}

#[derive(Debug)]
pub struct LatencyInterval {
    pub samples_us: Vec<u64>,
    pub total_samples: u64,
    pub unsynced: u64,
    pub clock_ahead: u64,
    pub latest_sync_rtt_us: Option<u32>,
}

fn reservoir_index(sample_number: u64) -> u64 {
    let mut value = sample_number.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    value % sample_number
}

pub fn latency_percentiles_us(mut samples_us: Vec<u64>) -> Option<[u64; 4]> {
    if samples_us.is_empty() {
        return None;
    }
    samples_us.sort_unstable();
    let len = samples_us.len();
    Some([
        nearest_rank(&samples_us, len, 50),
        nearest_rank(&samples_us, len, 75),
        nearest_rank(&samples_us, len, 90),
        nearest_rank(&samples_us, len, 99),
    ])
}

fn nearest_rank(sorted_samples_us: &[u64], len: usize, percentile: usize) -> u64 {
    let rank = (len * percentile).div_ceil(100).max(1);
    sorted_samples_us[rank - 1]
}

#[derive(Debug, Default)]
pub struct ServerMetrics {
    pub decoded_total: u64,
    pub mapped_total: u64,
    pub unknown_total: u64,
    pub malformed_total: u64,
    pub receive_errors_total: u64,
    decoded_interval: u64,
    mapped_interval: u64,
    unknown_interval: u64,
    malformed_by_kind: BTreeMap<DecodeError, u64>,
    unknown_by_mac: BTreeMap<MacAddress, u64>,
    unknown_unlisted: u64,
}

impl ServerMetrics {
    pub fn record_decoded(&mut self) {
        self.decoded_total += 1;
        self.decoded_interval += 1;
    }

    pub fn record_mapped(&mut self) {
        self.mapped_total += 1;
        self.mapped_interval += 1;
    }

    pub fn record_unknown(&mut self, mac: MacAddress) {
        self.unknown_total += 1;
        self.unknown_interval += 1;
        if let Some(count) = self.unknown_by_mac.get_mut(&mac) {
            *count += 1;
        } else if self.unknown_by_mac.len() < MAX_REPORTED_UNKNOWN_MACS {
            self.unknown_by_mac.insert(mac, 1);
        } else {
            self.unknown_unlisted += 1;
        }
    }

    pub fn record_malformed(&mut self, error: DecodeError) {
        self.malformed_total += 1;
        *self.malformed_by_kind.entry(error).or_default() += 1;
    }

    pub fn record_receive_error(&mut self) {
        self.receive_errors_total += 1;
    }

    pub fn take_interval(&mut self) -> MetricsInterval {
        MetricsInterval {
            decoded: std::mem::take(&mut self.decoded_interval),
            mapped: std::mem::take(&mut self.mapped_interval),
            unknown: std::mem::take(&mut self.unknown_interval),
            malformed: std::mem::take(&mut self.malformed_by_kind),
            unknown_by_mac: std::mem::take(&mut self.unknown_by_mac),
            unknown_unlisted: std::mem::take(&mut self.unknown_unlisted),
        }
    }
}

#[derive(Debug)]
pub struct MetricsInterval {
    pub decoded: u64,
    pub mapped: u64,
    pub unknown: u64,
    pub malformed: BTreeMap<DecodeError, u64>,
    pub unknown_by_mac: BTreeMap<MacAddress, u64>,
    pub unknown_unlisted: u64,
}

pub fn format_rate(count: u64, elapsed: Duration) -> f64 {
    count as f64 / elapsed.as_secs_f64().max(f64::EPSILON)
}
