use crate::state::DeviceRegistry;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{Request, Response};
use tungstenite::{Message, WebSocket, accept_hdr};

const CLIENT_QUEUE_CAPACITY: usize = 2048;
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone)]
pub struct WebSocketHub {
    shared: Arc<Shared>,
}

struct Shared {
    timestamp_origin: Mutex<Instant>,
    measuring: std::sync::atomic::AtomicBool,
    mode_generation: AtomicU64,
    next_client_id: AtomicU64,
    state: Mutex<HubState>,
    control_sender: mpsc::Sender<ControlCommand>,
    control_receiver: Mutex<mpsc::Receiver<ControlCommand>>,
    controller_id: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlCommand {
    Start,
    Stop,
}

struct HubState {
    sensor_names: BTreeSet<String>,
    latest: BTreeMap<String, SensorValue>,
    clients: HashMap<u64, Client>,
    last_sensor_packet: Instant,
}

struct Client {
    sensors: BTreeSet<String>,
    sender: mpsc::SyncSender<HubUpdate>,
    last_full_update: Instant,
}

#[derive(Clone)]
struct SensorValue {
    angle: Option<f64>,
    received_at: Instant,
}

#[derive(Clone)]
struct SensorUpdate {
    name: String,
    value: SensorValue,
}

#[derive(Clone)]
struct HubUpdate {
    sensors: Vec<SensorUpdate>,
    timestamp: Instant,
    mode_generation: u64,
}

impl WebSocketHub {
    pub fn new(sensor_names: impl IntoIterator<Item = String>, server_start: Instant) -> Self {
        let (control_sender, control_receiver) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                timestamp_origin: Mutex::new(server_start),
                measuring: std::sync::atomic::AtomicBool::new(false),
                mode_generation: AtomicU64::new(0),
                next_client_id: AtomicU64::new(1),
                control_sender,
                control_receiver: Mutex::new(control_receiver),
                controller_id: AtomicU64::new(0),
                state: Mutex::new(HubState {
                    sensor_names: sensor_names.into_iter().collect(),
                    latest: BTreeMap::new(),
                    clients: HashMap::new(),
                    last_sensor_packet: server_start,
                }),
            }),
        }
    }

    pub fn publish(&self, name: &str, angle: Option<f64>, received_at: Instant) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        let value = SensorValue { angle, received_at };
        state.latest.insert(name.to_owned(), value.clone());
        state.last_sensor_packet = received_at;
        let update = HubUpdate {
            sensors: vec![SensorUpdate {
                name: name.to_owned(),
                value,
            }],
            timestamp: received_at,
            mode_generation: self.shared.mode_generation.load(Ordering::Acquire),
        };
        if !self.is_measuring() {
            return;
        }
        state.clients.retain(|_, client| {
            if !client.sensors.contains(name) {
                return true;
            }
            match client.sender.try_send(update.clone()) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_) | mpsc::TrySendError::Disconnected(_)) => false,
            }
        });
    }

    pub fn start_measurement(&self) {
        if let Ok(mut origin) = self.shared.timestamp_origin.lock() {
            *origin = Instant::now();
            self.shared.mode_generation.fetch_add(1, Ordering::AcqRel);
            self.shared.measuring.store(true, Ordering::Release);
            let _ = self.shared.control_sender.send(ControlCommand::Start);
        }
    }

    pub fn stop_measurement(&self) {
        self.shared.measuring.store(false, Ordering::Release);
        self.shared.mode_generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.shared.control_sender.send(ControlCommand::Stop);
    }

    pub fn try_next_control(&self) -> Option<ControlCommand> {
        self.shared.control_receiver.lock().ok()?.try_recv().ok()
    }

    pub fn is_measuring(&self) -> bool {
        self.shared.measuring.load(Ordering::Acquire)
    }

    pub fn publish_stale_snapshot_if_due(&self, now: Instant) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        if now.saturating_duration_since(state.last_sensor_packet) < SNAPSHOT_INTERVAL {
            return;
        }
        let latest = state.latest.clone();
        state.clients.retain(|_, client| {
            if now.saturating_duration_since(client.last_full_update) < SNAPSHOT_INTERVAL {
                return true;
            }
            client.last_full_update = now;
            let sensors = client
                .sensors
                .iter()
                .filter_map(|name| {
                    latest.get(name).cloned().map(|value| SensorUpdate {
                        name: name.clone(),
                        value,
                    })
                })
                .collect::<Vec<_>>();
            match client.sender.try_send(HubUpdate {
                sensors,
                timestamp: now,
                mode_generation: self.shared.mode_generation.load(Ordering::Acquire),
            }) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_) | mpsc::TrySendError::Disconnected(_)) => false,
            }
        });
    }

    pub fn start(&self, bind_address: SocketAddrV4) -> io::Result<()> {
        let listener = TcpListener::bind(bind_address)?;
        listener.set_nonblocking(true)?;
        let hub = self.clone();
        thread::Builder::new()
            .name("sensor-websocket-listener".to_owned())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let hub = hub.clone();
                            let _ = thread::Builder::new()
                                .name("sensor-websocket-client".to_owned())
                                .spawn(move || hub.serve_client(stream));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(20)),
                    }
                }
            })?;
        Ok(())
    }

    fn serve_client(&self, stream: TcpStream) {
        let Ok(mut socket) = accept_hdr(stream, |request: &Request, response: Response| {
            if request.uri().path() == "/ws" {
                Ok(response)
            } else {
                Err(http::Response::builder()
                    .status(404)
                    .body(Some("WebSocket endpoint is /ws".to_owned()))
                    .expect("valid WebSocket rejection response"))
            }
        }) else {
            return;
        };
        if socket.get_ref().set_nonblocking(true).is_err() {
            return;
        }
        let client_id = self.shared.next_client_id.fetch_add(1, Ordering::Relaxed);
        let mut subscription: Option<mpsc::Receiver<HubUpdate>> = None;
        let mut sequence = 0_u64;
        let mut last_activity = Instant::now();
        let mut ping_sent_at: Option<Instant> = None;

        loop {
            let mut did_work = false;
            if let Some(receiver) = &subscription {
                match receiver.try_recv() {
                    Ok(update) => {
                        if update.mode_generation
                            == self.shared.mode_generation.load(Ordering::Acquire)
                        {
                            sequence = sequence.wrapping_add(1);
                            if send_update(
                                &mut socket,
                                sequence,
                                update,
                                &self.shared.timestamp_origin,
                            )
                            .is_err()
                            {
                                break;
                            }
                            did_work = true;
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }

            let now = Instant::now();
            if ping_sent_at
                .is_some_and(|sent| now.saturating_duration_since(sent) >= HEARTBEAT_TIMEOUT)
            {
                break;
            }
            if ping_sent_at.is_none()
                && now.saturating_duration_since(last_activity) >= HEARTBEAT_INTERVAL
            {
                if socket.send(Message::Ping(Vec::new())).is_err() {
                    break;
                }
                ping_sent_at = Some(now);
            }

            match socket.read() {
                Ok(Message::Text(text)) => {
                    did_work = true;
                    last_activity = Instant::now();
                    match serde_json::from_str::<ControlRequest>(&text) {
                        Ok(request) if request.message_type == "subscribe" => {
                            did_work = true;
                            match self
                                .subscribe(client_id, request.sensors.as_deref().unwrap_or(&[]))
                            {
                                Ok((receiver, snapshot)) => {
                                    subscription = Some(receiver);
                                    sequence = sequence.wrapping_add(1);
                                    if send_update(
                                        &mut socket,
                                        sequence,
                                        snapshot,
                                        &self.shared.timestamp_origin,
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                                Err(message) => {
                                    let error = json!({
                                        "type": "error",
                                        "code": "unknown_sensor",
                                        "message": message,
                                    });
                                    if socket.send(Message::Text(error.to_string())).is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(request) if request.message_type == "start" => {
                            self.shared
                                .controller_id
                                .store(client_id, Ordering::Release);
                            self.start_measurement();
                            did_work = true;
                            if send_control_ack(&mut socket, "started").is_err() {
                                break;
                            }
                        }
                        Ok(request) if request.message_type == "stop" => {
                            self.shared
                                .controller_id
                                .store(client_id, Ordering::Release);
                            self.stop_measurement();
                            did_work = true;
                            if send_control_ack(&mut socket, "stopped").is_err() {
                                break;
                            }
                        }
                        Ok(_) => {
                            if send_error(
                                &mut socket,
                                "invalid_request",
                                "type must be subscribe and sensors must be a non-empty array",
                            )
                            .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => {
                            if send_error(
                                &mut socket,
                                "invalid_json",
                                "request must be a valid JSON object",
                            )
                            .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                Ok(Message::Pong(_)) => {
                    ping_sent_at = None;
                    last_activity = Instant::now();
                    did_work = true;
                }
                Ok(Message::Ping(_)) => {
                    last_activity = Instant::now();
                    if socket.flush().is_err() {
                        break;
                    }
                    did_work = true;
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {
                    last_activity = Instant::now();
                    did_work = true;
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    break;
                }
                Err(_) => break,
            }

            if !did_work {
                thread::sleep(Duration::from_millis(1));
            }
        }

        if let Ok(mut state) = self.shared.state.lock() {
            state.clients.remove(&client_id);
        }
        if self
            .shared
            .controller_id
            .compare_exchange(client_id, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.stop_measurement();
        }
    }

    fn subscribe(
        &self,
        client_id: u64,
        requested: &[String],
    ) -> Result<(mpsc::Receiver<HubUpdate>, HubUpdate), String> {
        if requested.is_empty() {
            return Err("sensors must contain at least one configured sensor name".to_owned());
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "sensor state is unavailable".to_owned())?;
        let sensors: BTreeSet<String> = if requested.iter().any(|name| name == "*") {
            state.sensor_names.clone()
        } else {
            requested.iter().cloned().collect()
        };
        if sensors.is_empty() {
            return Err("no sensors are configured".to_owned());
        }
        if let Some(unknown) = sensors
            .iter()
            .find(|name| !state.sensor_names.contains(*name))
        {
            return Err(format!("unknown sensor: {unknown}"));
        }

        let (sender, receiver) = mpsc::sync_channel(CLIENT_QUEUE_CAPACITY);
        let now = Instant::now();
        let snapshot = HubUpdate {
            sensors: sensors
                .iter()
                .filter_map(|name| {
                    state.latest.get(name).cloned().map(|value| SensorUpdate {
                        name: name.clone(),
                        value,
                    })
                })
                .collect(),
            timestamp: now,
            mode_generation: self.shared.mode_generation.load(Ordering::Acquire),
        };
        state.clients.insert(
            client_id,
            Client {
                sensors,
                sender,
                last_full_update: now,
            },
        );
        Ok((receiver, snapshot))
    }
}

#[derive(Deserialize)]
struct ControlRequest {
    #[serde(rename = "type")]
    message_type: String,
    sensors: Option<Vec<String>>,
}

fn send_update(
    socket: &mut WebSocket<TcpStream>,
    sequence: u64,
    update: HubUpdate,
    timestamp_origin: &Mutex<Instant>,
) -> Result<(), tungstenite::Error> {
    let now = Instant::now();
    let mut sensors = serde_json::Map::new();
    for sensor in update.sensors {
        sensors.insert(
            sensor.name,
            json!({
                "angle": sensor.value.angle,
                "age_us": now.saturating_duration_since(sensor.value.received_at).as_micros().min(u128::from(u64::MAX)) as u64,
            }),
        );
    }
    let origin = timestamp_origin
        .lock()
        .map(|origin| *origin)
        .unwrap_or(update.timestamp);
    let timestamp_us = update
        .timestamp
        .saturating_duration_since(origin)
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    let message = json!({
        "event": "update",
        "type": "sensor_data",
        "seq": sequence,
        "timestamp_us": timestamp_us,
        "sensor": Value::Object(sensors),
    });
    socket.send(Message::Text(message.to_string()))
}

fn send_control_ack(
    socket: &mut WebSocket<TcpStream>,
    status: &str,
) -> Result<(), tungstenite::Error> {
    socket.send(Message::Text(
        json!({"type": "control", "status": status}).to_string(),
    ))
}

fn send_error(
    socket: &mut WebSocket<TcpStream>,
    code: &str,
    message: &str,
) -> Result<(), tungstenite::Error> {
    socket.send(Message::Text(
        json!({"type": "error", "code": code, "message": message}).to_string(),
    ))
}

pub fn start_from_registry(
    devices: &DeviceRegistry,
    server_start: Instant,
    bind_address: SocketAddrV4,
) -> Result<WebSocketHub, String> {
    let hub = WebSocketHub::new(devices.by_name.keys().cloned(), server_start);
    hub.start(bind_address)
        .map_err(|error| format!("cannot start WebSocket API on {bind_address}: {error}"))?;
    Ok(hub)
}
