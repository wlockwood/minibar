//! Local HTTP/WebSocket adapter for [`crate::live::LiveDriver`].
//!
//! Nothing in this module is initialized unless [`start_live_server`] is
//! called. The server is deliberately synchronous and serializes access to the
//! authoritative driver; simulation state therefore never depends on socket
//! thread scheduling.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tungstenite::protocol::{Message, WebSocketConfig};

use crate::live::{
    BuildPreviewRequest, ClientEnvelope, LIVE_PROTOCOL_VERSION, LiveDriver, LiveState, ReplayLog,
    ServerMessage, StepWorld,
};
use crate::model::Vec2;
use crate::strategy::{
    ActionCapabilities, ActionRejection, ActionWorld, ObservationVisibility, ObservedFireIntent,
    ObservedResourceSite, ResourceObservation, SideId, StrategyEntityKind,
};
use crate::viewer_protocol::{ViewerFrameHeader, ViewerMode, ViewerProvenance};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const DEFAULT_MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Exactly one connected viewer owns simulation-time controls.  Other local
/// viewers remain useful as observers and can issue unit orders, but cannot
/// accidentally multiply the live simulation rate by each sending `Advance`.
#[derive(Default)]
struct LiveControlLease {
    owner: Option<u64>,
}

impl LiveControlLease {
    fn allows(&mut self, client_id: u64, message: &crate::live::ClientMessage) -> bool {
        let changes_time = matches!(
            message,
            crate::live::ClientMessage::SetPaused { .. }
                | crate::live::ClientMessage::SetSpeed { .. }
                | crate::live::ClientMessage::Step { .. }
                | crate::live::ClientMessage::Advance { .. }
        );
        if !changes_time {
            return true;
        }
        match self.owner {
            Some(owner) => owner == client_id,
            None => {
                self.owner = Some(client_id);
                true
            }
        }
    }

    fn release(&mut self, client_id: u64) {
        if self.owner == Some(client_id) {
            self.owner = None;
        }
    }
}

struct LiveControlLeaseGuard {
    lease: Arc<Mutex<LiveControlLease>>,
    client_id: u64,
}

impl LiveControlLeaseGuard {
    fn new(lease: Arc<Mutex<LiveControlLease>>, client_id: u64) -> Self {
        Self { lease, client_id }
    }

    fn allows(&self, message: &crate::live::ClientMessage) -> bool {
        self.lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .allows(self.client_id, message)
    }
}

impl Drop for LiveControlLeaseGuard {
    fn drop(&mut self) {
        self.lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .release(self.client_id);
    }
}

/// Per-WebSocket connection state kept out of the authoritative driver.
struct LiveClientSession {
    replay_log: Arc<Mutex<ReplayLog>>,
    stopping: Arc<AtomicBool>,
    control_lease: LiveControlLeaseGuard,
    /// This is intentionally local to one browser connection. It determines
    /// both the fog-of-war observation and which side may receive its orders.
    view_side: SideId,
    /// Maps a browser's retry token to its authoritative reserved sequence.
    /// This makes a duplicate submission on the same connection idempotent.
    assigned_sequences: BTreeMap<(SideId, u64), u64>,
    human_sequences: Arc<Mutex<LiveHumanSequenceAllocator>>,
}

/// Scripted controllers begin at low sequence numbers. Human commands use a
/// monotonic per-side range above that namespace, allocated centrally for all
/// local browser sessions. The range is so large that a controller cannot
/// realistically reach it, while central allocation avoids two browsers
/// issuing the same sequence after controlling the same side.
const FIRST_HUMAN_ACTION_SEQUENCE: u64 = 1 << 48;

struct LiveHumanSequenceAllocator {
    next_by_side: [u64; 2],
}

impl Default for LiveHumanSequenceAllocator {
    fn default() -> Self {
        Self {
            next_by_side: [FIRST_HUMAN_ACTION_SEQUENCE; 2],
        }
    }
}

impl LiveHumanSequenceAllocator {
    fn allocate(&mut self, side: SideId) -> u64 {
        let next = &mut self.next_by_side[side as usize];
        let sequence = *next;
        *next = next
            .checked_add(1)
            .expect("human action sequence namespace exhausted");
        sequence
    }
}

/// Configuration for the opt-in local viewer server.
#[derive(Clone, Copy, Debug)]
pub struct LiveServerConfig {
    /// Must be a loopback address. Port zero asks the OS for a free port.
    pub bind_addr: SocketAddr,
    /// Emit an observation after at least this many newly observed ticks.
    pub observation_every_ticks: u64,
    /// Upper bound for a JSON WebSocket message.
    pub max_message_bytes: usize,
}

impl Default for LiveServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 0),
            observation_every_ticks: 1,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

#[derive(Debug)]
pub enum LiveServerError {
    NonLoopbackBind(SocketAddr),
    InvalidObservationInterval,
    InvalidMessageLimit,
    InvalidInitialViewSide(SideId),
    Io(io::Error),
    ReplaySerialize(serde_json::Error),
    ServerThreadPanicked,
}

impl fmt::Display for LiveServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonLoopbackBind(address) => {
                write!(
                    formatter,
                    "live server address must be loopback, got {address}"
                )
            }
            Self::InvalidObservationInterval => {
                formatter.write_str("observation interval must be nonzero")
            }
            Self::InvalidMessageLimit => formatter.write_str("message limit must be nonzero"),
            Self::InvalidInitialViewSide(side) => {
                write!(
                    formatter,
                    "initial live viewer side must be 0 or 1, got {side}"
                )
            }
            Self::Io(error) => error.fmt(formatter),
            Self::ReplaySerialize(error) => {
                write!(formatter, "could not serialize replay log: {error}")
            }
            Self::ServerThreadPanicked => formatter.write_str("live server thread panicked"),
        }
    }
}

impl std::error::Error for LiveServerError {}

impl From<io::Error> for LiveServerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// An unsolicited sampled frame sent after the corresponding response.
/// Command responses themselves remain exact [`crate::live::ServerEnvelope`]
/// JSON values.
#[derive(Clone, Debug, Serialize)]
pub struct ObservationEnvelope {
    pub protocol_version: u16,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub state: LiveState,
    pub observation: Value,
}

/// Static map data sent with every live frame. It is intentionally compact
/// enough for a symbolic local viewer, not a replacement for native map data.
#[derive(Clone, Debug, Serialize)]
struct LiveMapFrame {
    width: f32,
    depth: f32,
    cols: u32,
    rows: u32,
    cell_size_x: f32,
    cell_size_z: f32,
    heights: Vec<f32>,
    water_depth: Vec<f32>,
    passable: Vec<bool>,
}

impl LiveMapFrame {
    fn from_map(map: &crate::map::CompiledMap) -> Self {
        let cols = map.heights.cols;
        let rows = map.heights.rows;
        let cell_size_x = map.width / cols.max(1) as f32;
        let cell_size_z = map.depth / rows.max(1) as f32;
        let mut heights = Vec::with_capacity((cols * rows) as usize);
        let mut water_depth = Vec::with_capacity((cols * rows) as usize);
        let mut passable = Vec::with_capacity((cols * rows) as usize);
        let default_layer = map.passability.first();
        for row in 0..rows {
            for col in 0..cols {
                let x = (col as f32 + 0.5) * cell_size_x;
                let z = (row as f32 + 0.5) * cell_size_z;
                let height = map.ground_height(x, z);
                heights.push(height);
                water_depth.push((map.water_level - height).max(0.0));
                passable.push(default_layer.is_none_or(|layer| layer.is_passable(x, z)));
            }
        }
        Self {
            width: map.width,
            depth: map.depth,
            cols,
            rows,
            cell_size_x,
            cell_size_z,
            heights,
            water_depth,
            passable,
        }
    }
}

/// One client-safe entity view. Fields not observable at the selected side are
/// omitted instead of reading the authoritative entity and leaking state.
#[derive(Clone, Debug, Serialize)]
struct LiveEntityFrame {
    id: crate::entity::EntityId,
    side: SideId,
    #[serde(skip_serializing_if = "Option::is_none")]
    definition: Option<String>,
    kind: StrategyEntityKind,
    position: Vec2,
    #[serde(skip_serializing_if = "Option::is_none")]
    health_fraction: Option<f32>,
    visibility: ObservationVisibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    heading: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turret_heading: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_turret: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capabilities: Option<ActionCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    construction_progress: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sensor_coverage: Option<LiveSensorCoverage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    planned_route: Vec<Vec2>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct LiveSensorCoverage {
    pub los: f32,
    pub radar: f32,
    pub jammer: f32,
}

/// The world-phase result of a submitted or scripted action. Transport-level
/// acknowledgement only means the driver queued a command; this record tells
/// a live player whether the authoritative world actually executed it.
#[derive(Clone, Debug, Serialize)]
struct LiveActionResult {
    side: SideId,
    sequence: u64,
    accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    rejection: Option<ActionRejection>,
}

/// Damage that resolved during the just-sampled world tick. It gives the
/// symbolic viewer a concrete impact cue without rendering projectiles.
#[derive(Clone, Debug, Serialize)]
struct LiveDamageFrame {
    entity: crate::entity::EntityId,
    amount: f64,
}

#[derive(Clone, Debug, Serialize)]
struct LiveWorldFrame {
    #[serde(flatten)]
    viewer: ViewerFrameHeader,
    tick: u64,
    side: SideId,
    map_size: Vec2,
    metal: ResourceObservation,
    energy: ResourceObservation,
    map: LiveMapFrame,
    entities: Vec<LiveEntityFrame>,
    fire_intents: Vec<ObservedFireIntent>,
    resource_sites: Vec<ObservedResourceSite>,
    damage_events: Vec<LiveDamageFrame>,
    action_results: Vec<LiveActionResult>,
}

fn world_live_frame(world: &crate::world::World, side: SideId, map: &LiveMapFrame) -> Value {
    let Ok(observation) = world.observation(side) else {
        return serde_json::json!({ "error": "could not produce world observation" });
    };
    let entities: Vec<LiveEntityFrame> = observation
        .entities
        .into_iter()
        .map(|observed| {
            let exact = matches!(
                observed.visibility,
                ObservationVisibility::Own | ObservationVisibility::LineOfSight
            );
            let actual = exact.then(|| world.entities.get(observed.id)).flatten();
            let definition = actual.and_then(|entity| world.catalog.get(entity.definition));
            LiveEntityFrame {
                id: observed.id,
                side: observed.side,
                definition: observed.definition,
                kind: observed.kind,
                position: observed.position,
                health_fraction: observed.health_fraction,
                visibility: observed.visibility,
                heading: actual.zip(definition).and_then(|(entity, unit)| {
                    (unit.strategic.is_mobile || !unit.weapons.is_empty()).then_some(entity.heading)
                }),
                turret_heading: actual.and_then(|_| world.primary_turret_heading(observed.id)),
                is_turret: definition
                    .map(|unit| !unit.strategic.is_mobile && !unit.weapons.is_empty()),
                capabilities: exact
                    .then(|| ActionWorld::entity(world, observed.id))
                    .flatten()
                    .map(|entity| entity.capabilities),
                radius: definition.map(|unit| unit.movement.radius),
                enabled: actual.map(|entity| entity.enabled),
                construction_progress: exact
                    .then(|| world.construction_progress(observed.id))
                    .flatten(),
                sensor_coverage: exact
                    .then(|| {
                        actual.and_then(|entity| {
                            let (los, radar, jammer) = world.entity_sensor_ranges(observed.id, entity);
                            (los > 0.0 || radar > 0.0 || jammer > 0.0).then_some(LiveSensorCoverage {
                                los,
                                radar,
                                jammer,
                            })
                        })
                    })
                    .flatten(),
                planned_route: if observed.side == side {
                    world.planned_route(observed.id)
                } else {
                    Vec::new()
                },
            }
        })
        .collect();
    let observed_ids = entities
        .iter()
        .map(|entity| entity.id)
        .collect::<std::collections::BTreeSet<_>>();
    let damage_events = world
        .events
        .iter()
        .filter_map(|event| match event {
            crate::world::WorldEvent::CombatDamage { entity, amount, .. }
                if observed_ids.contains(entity) =>
            {
                Some(LiveDamageFrame {
                    entity: *entity,
                    amount: *amount,
                })
            }
            _ => None,
        })
        .collect();
    serde_json::to_value(LiveWorldFrame {
        viewer: ViewerFrameHeader::new(
            ViewerMode::LiveSimulation,
            ViewerProvenance {
                source_id: "simulator-world".into(),
                ..ViewerProvenance::default()
            },
        ),
        tick: observation.tick,
        side: observation.side,
        map_size: observation.map_size,
        metal: observation.metal,
        energy: observation.energy,
        map: map.clone(),
        entities,
        fire_intents: observation.fire_intents,
        resource_sites: observation.resource_sites,
        damage_events,
        action_results: world
            .action_decisions
            .iter()
            .map(|decision| LiveActionResult {
                side: decision.queued.side,
                sequence: decision.queued.sequence,
                accepted: decision.result.is_ok(),
                rejection: decision.result.as_ref().err().copied(),
            })
            .collect(),
    })
    .unwrap_or(Value::Null)
}

/// Removes the immutable terrain payload from all but a client's first frame.
/// This deliberately leaves dynamic resource sites in the frame: extractors
/// can claim or reclaim them during play.
fn strip_static_map(observation: &mut Value) {
    if let Some(object) = observation.as_object_mut() {
        object.remove("map");
    }
}

/// Gracefully stops the listener and active client threads when shut down or
/// dropped. Clones are intentionally not supported so ownership is explicit.
pub struct LiveServerHandle {
    local_addr: SocketAddr,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    replay_log: Arc<Mutex<ReplayLog>>,
}

impl LiveServerHandle {
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(mut self) -> Result<(), LiveServerError> {
        self.stop_and_join()
    }

    /// Snapshot of accepted commands and their authoritative terminal hash.
    pub fn replay_log(&self) -> ReplayLog {
        self.replay_log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Stops the server before returning the final immutable replay record.
    pub fn shutdown_with_replay(mut self) -> Result<ReplayLog, LiveServerError> {
        self.stop_and_join()?;
        Ok(self.replay_log())
    }

    /// Persists a formatted replay record without exposing the driver lock.
    pub fn write_replay_log(&self, path: &Path) -> Result<(), LiveServerError> {
        let bytes = serde_json::to_vec_pretty(&self.replay_log())
            .map_err(LiveServerError::ReplaySerialize)?;
        std::fs::write(path, bytes).map_err(LiveServerError::Io)
    }

    fn stop_and_join(&mut self) -> Result<(), LiveServerError> {
        self.stopping.store(true, Ordering::Release);
        // Wake a nonblocking accept loop promptly on platforms with coarse
        // scheduler timers.
        let _ = TcpStream::connect_timeout(&self.local_addr, Duration::from_millis(50));
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| LiveServerError::ServerThreadPanicked)?;
        }
        Ok(())
    }
}

impl Drop for LiveServerHandle {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

/// Starts a loopback-only viewer server around one authoritative driver.
///
/// `sample` runs while the driver lock is held and should be quick. It may
/// return any JSON value; [`start_world_live_server`] supplies the normal
/// strategic observation for a [`crate::world::World`].
pub fn start_live_server<W, S>(
    driver: LiveDriver<W>,
    config: LiveServerConfig,
    sample: S,
) -> Result<LiveServerHandle, LiveServerError>
where
    W: StepWorld + Send + 'static,
    S: Fn(&W) -> Value + Send + Sync + 'static,
{
    start_live_server_with_preview(
        driver,
        config,
        move |world, _| sample(world),
        |_, _| serde_json::json!({ "valid": false, "rejection": "build previews are unavailable for this world" }),
        0,
    )
}

fn start_live_server_with_preview<W, S, P>(
    driver: LiveDriver<W>,
    config: LiveServerConfig,
    sample: S,
    preview: P,
    initial_view_side: SideId,
) -> Result<LiveServerHandle, LiveServerError>
where
    W: StepWorld + Send + 'static,
    S: Fn(&W, SideId) -> Value + Send + Sync + 'static,
    P: Fn(&W, &BuildPreviewRequest) -> Value + Send + Sync + 'static,
{
    if initial_view_side >= 2 {
        return Err(LiveServerError::InvalidInitialViewSide(initial_view_side));
    }
    validate_config(config)?;
    let listener = TcpListener::bind(config.bind_addr)?;
    listener.set_nonblocking(true)?;
    let local_addr = listener.local_addr()?;
    let stopping = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stopping);
    let replay_log = Arc::new(Mutex::new(driver.replay_log()));
    let thread_replay_log = Arc::clone(&replay_log);
    let driver = Arc::new(Mutex::new(driver));
    let sample = Arc::new(sample);
    let preview = Arc::new(preview);
    let control_lease = Arc::new(Mutex::new(LiveControlLease::default()));
    let human_sequences = Arc::new(Mutex::new(LiveHumanSequenceAllocator::default()));

    let thread = thread::Builder::new()
        .name("visigoth-live-server".into())
        .spawn(move || {
            let mut clients = Vec::new();
            let mut next_client_id = 1_u64;
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let driver = Arc::clone(&driver);
                        let sample = Arc::clone(&sample);
                        let preview = Arc::clone(&preview);
                        let replay_log = Arc::clone(&thread_replay_log);
                        let client_stop = Arc::clone(&thread_stop);
                        let control_lease = Arc::clone(&control_lease);
                        let human_sequences = Arc::clone(&human_sequences);
                        let client_id = next_client_id;
                        next_client_id = next_client_id.wrapping_add(1).max(1);
                        clients.push(thread::spawn(move || {
                            let _ = serve_connection(
                                stream,
                                driver,
                                sample,
                                preview,
                                LiveClientSession {
                                    replay_log,
                                    stopping: client_stop,
                                    control_lease: LiveControlLeaseGuard::new(
                                        control_lease,
                                        client_id,
                                    ),
                                    view_side: initial_view_side,
                                    assigned_sequences: BTreeMap::new(),
                                    human_sequences,
                                },
                                config,
                            );
                        }));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
            for client in clients {
                let _ = client.join();
            }
        })?;

    Ok(LiveServerHandle {
        local_addr,
        stopping,
        thread: Some(thread),
        replay_log,
    })
}

/// Starts a server whose samples are the normal per-side world observation.
pub fn start_world_live_server(
    driver: LiveDriver<crate::world::World>,
    side: u8,
    config: LiveServerConfig,
) -> Result<LiveServerHandle, LiveServerError> {
    let map = LiveMapFrame::from_map(&driver.world().map);
    start_live_server_with_preview(
        driver,
        config,
        move |world, view_side| world_live_frame(world, view_side, &map),
        move |world, request| {
            serde_json::to_value(world.preview_build(
                request.side,
                request.builder,
                &request.definition,
                request.position,
                request.facing,
            ))
            .unwrap_or(Value::Null)
        },
        side,
    )
}

/// Starts a server around a compiled declarative scenario. Scripted opening
/// controllers and human commands share the same deterministic world inbox.
pub fn start_scenario_live_server(
    driver: LiveDriver<crate::scenario_compile::DeclarativeScenarioRuntime>,
    side: u8,
    config: LiveServerConfig,
) -> Result<LiveServerHandle, LiveServerError> {
    let map = LiveMapFrame::from_map(&driver.world().world.map);
    start_live_server_with_preview(
        driver,
        config,
        move |runtime, view_side| world_live_frame(&runtime.world, view_side, &map),
        move |runtime, request| {
            serde_json::to_value(runtime.world.preview_build(
                request.side,
                request.builder,
                &request.definition,
                request.position,
                request.facing,
            ))
            .unwrap_or(Value::Null)
        },
        side,
    )
}

fn validate_config(config: LiveServerConfig) -> Result<(), LiveServerError> {
    if !config.bind_addr.ip().is_loopback() {
        return Err(LiveServerError::NonLoopbackBind(config.bind_addr));
    }
    if config.observation_every_ticks == 0 {
        return Err(LiveServerError::InvalidObservationInterval);
    }
    if config.max_message_bytes == 0 {
        return Err(LiveServerError::InvalidMessageLimit);
    }
    Ok(())
}

fn serve_connection<W, S, P>(
    mut stream: TcpStream,
    driver: Arc<Mutex<LiveDriver<W>>>,
    sample: Arc<S>,
    preview: Arc<P>,
    session: LiveClientSession,
    config: LiveServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    W: StepWorld,
    S: Fn(&W, SideId) -> Value,
    P: Fn(&W, &BuildPreviewRequest) -> Value,
{
    // The initial browser connection can take longer than one simulation
    // frame, particularly under a debugger or a cold browser cache. Keep the
    // handshake tolerant; the WebSocket loop restores its short timeout after
    // the upgrade so shutdown remains prompt.
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let header = peek_http_header(&stream)?;
    let first_line = header.lines().next().unwrap_or_default();
    let websocket = header.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("upgrade") && value.trim().eq_ignore_ascii_case("websocket")
        })
    });

    if websocket && first_line.split_whitespace().nth(1) == Some("/ws") {
        serve_websocket(stream, driver, sample, preview, session, config)?;
    } else {
        serve_http(&mut stream, first_line)?;
    }
    Ok(())
}

fn peek_http_header(stream: &TcpStream) -> io::Result<String> {
    let mut bytes = vec![0; MAX_HTTP_HEADER_BYTES];
    loop {
        match stream.peek(&mut bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "empty request",
                ));
            }
            Ok(count) => {
                let text = String::from_utf8_lossy(&bytes[..count]);
                if text.contains("\r\n\r\n") {
                    return Ok(text.into_owned());
                }
                if count == bytes.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HTTP header exceeds limit",
                    ));
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(error);
            }
            Err(error) => return Err(error),
        }
    }
}

fn serve_http(stream: &mut TcpStream, first_line: &str) -> io::Result<()> {
    let mut request = [0_u8; MAX_HTTP_HEADER_BYTES];
    let _ = stream.read(&mut request)?;
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = match path {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", VIEWER_HTML),
        "/health" => ("200 OK", "application/json", "{\"status\":\"ok\"}"),
        _ => ("404 Not Found", "text/plain; charset=utf-8", "not found"),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

fn serve_websocket<W, S, P>(
    stream: TcpStream,
    driver: Arc<Mutex<LiveDriver<W>>>,
    sample: Arc<S>,
    preview: Arc<P>,
    mut session: LiveClientSession,
    config: LiveServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    W: StepWorld,
    S: Fn(&W, SideId) -> Value,
    P: Fn(&W, &BuildPreviewRequest) -> Value,
{
    // `serve_connection` checked the request path before handing the stream to
    // the standards-compliant handshake implementation.
    let websocket_config = WebSocketConfig::default()
        .read_buffer_size(config.max_message_bytes.min(16 * 1024))
        .write_buffer_size(16 * 1024)
        .max_message_size(Some(config.max_message_bytes))
        .max_frame_size(Some(config.max_message_bytes));
    let mut socket = tungstenite::accept_with_config(stream, Some(websocket_config))?;
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut last_sample_tick = None;

    while !session.stopping.load(Ordering::Acquire) {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > config.max_message_bytes {
                    let _ = socket.close(None);
                    break;
                }
                let mut envelope: ClientEnvelope = match serde_json::from_str(&text) {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        socket.send(Message::Text(
                            serde_json::json!({
                                "type": "transport_error",
                                "error": "invalid_json",
                                "detail": error.to_string(),
                            })
                            .to_string()
                            .into(),
                        ))?;
                        continue;
                    }
                };
                let (response_json, observation_json, sampled_tick) = {
                    let mut driver = driver
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let side_changed = match &envelope.message {
                        crate::live::ClientMessage::SelectControlSide { side } if *side < 2 => {
                            let changed = session.view_side != *side;
                            session.view_side = *side;
                            changed
                        }
                        _ => false,
                    };
                    let side_error = match &envelope.message {
                        crate::live::ClientMessage::SelectControlSide { side } if *side >= 2 => {
                            Some(crate::live::ServerError::InvalidControlledSide {
                                requested_side: *side,
                            })
                        }
                        crate::live::ClientMessage::SubmitAction { action, .. }
                            if action.side != session.view_side =>
                        {
                            Some(crate::live::ServerError::ControlledSideMismatch {
                                controlled_side: session.view_side,
                                requested_side: action.side,
                            })
                        }
                        crate::live::ClientMessage::PreviewBuild { request }
                            if request.side != session.view_side =>
                        {
                            Some(crate::live::ServerError::ControlledSideMismatch {
                                controlled_side: session.view_side,
                                requested_side: request.side,
                            })
                        }
                        _ => None,
                    };
                    // The browser's sequence is only a retry token. Assign a
                    // per-side, server-owned value before it reaches the
                    // driver/world so separate human sessions and scripted
                    // controllers cannot share a sequence namespace.
                    if side_error.is_none()
                        && let crate::live::ClientMessage::SubmitAction { action, .. } =
                            &mut envelope.message
                    {
                        let retry_key = (action.side, action.sequence);
                        let sequence =
                            *session
                                .assigned_sequences
                                .entry(retry_key)
                                .or_insert_with(|| {
                                    session
                                        .human_sequences
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                                        .allocate(action.side)
                                });
                        action.sequence = sequence;
                    }
                    let preview_value = if side_error.is_none() {
                        match &envelope.message {
                            crate::live::ClientMessage::PreviewBuild { request } => {
                                Some(preview(driver.world(), request))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let is_preview = preview_value.is_some();
                    // Time controls are a lease, not a broadcast: opening a
                    // second browser tab cannot make the world run twice as
                    // fast.  Non-time commands remain available to observers.
                    let mut response = if let Some(error) = side_error {
                        let mut response = driver.handle(ClientEnvelope {
                            protocol_version: LIVE_PROTOCOL_VERSION,
                            request_id: envelope.request_id,
                            message: crate::live::ClientMessage::Synchronize {
                                last_server_sequence: None,
                            },
                        });
                        response.message = ServerMessage::Error {
                            error,
                            state: driver.state(),
                        };
                        response
                    } else if session.control_lease.allows(&envelope.message) {
                        driver.handle(envelope)
                    } else {
                        driver.handle(ClientEnvelope {
                            protocol_version: LIVE_PROTOCOL_VERSION,
                            request_id: envelope.request_id,
                            message: crate::live::ClientMessage::Synchronize {
                                last_server_sequence: None,
                            },
                        })
                    };
                    if let Some(preview) = preview_value {
                        response.message = ServerMessage::BuildPreview {
                            preview,
                            state: driver.state(),
                        };
                    }
                    if !is_preview {
                        *session
                            .replay_log
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = driver.replay_log();
                    }
                    let response_json = serde_json::to_string(&response)?;
                    let state = driver.state();
                    let sample_due = side_changed
                        || last_sample_tick.is_none_or(|last| {
                            state.displayed_tick.saturating_sub(last)
                                >= config.observation_every_ticks
                        });
                    let observation_json = sample_due
                        .then(|| {
                            let mut observation = sample(driver.world(), session.view_side);
                            // Map topology is immutable for a live session.
                            // Send it once per client and retain it in the
                            // viewer, avoiding a large allocation and JSON
                            // payload every simulation tick.
                            if last_sample_tick.is_some() {
                                strip_static_map(&mut observation);
                            }
                            ObservationEnvelope {
                                protocol_version: LIVE_PROTOCOL_VERSION,
                                kind: "observation",
                                state: state.clone(),
                                observation,
                            }
                        })
                        .map(|observation| serde_json::to_string(&observation))
                        .transpose()?;
                    (response_json, observation_json, state.displayed_tick)
                };
                // Never hold the authoritative driver lock while a slow client
                // applies network backpressure.
                socket.send(Message::Text(response_json.into()))?;
                if let Some(observation_json) = observation_json {
                    socket.send(Message::Text(observation_json.into()))?;
                    last_sample_tick = Some(sampled_tick);
                }
            }
            Ok(Message::Ping(payload)) => socket.send(Message::Pong(payload))?,
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(error) => return Err(error.into()),
        }
    }
    let _ = socket.close(None);
    Ok(())
}

const VIEWER_HTML: &str = crate::viewer_protocol::SHARED_VIEWER_HTML;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[derive(Debug)]
    struct MockWorld {
        tick: u64,
    }

    impl StepWorld for MockWorld {
        fn tick(&self) -> u64 {
            self.tick
        }
        fn enqueue_action(&mut self, _: crate::strategy::QueuedAction) {}
        fn step(&mut self) {
            self.tick += 1;
        }
        fn state_hash(&self) -> u64 {
            self.tick.rotate_left(7)
        }
    }

    #[test]
    fn one_connected_viewer_owns_time_controls_until_it_disconnects() {
        let mut lease = LiveControlLease::default();
        let advance = crate::live::ClientMessage::Advance {
            real_time_quanta: 1,
        };
        assert!(lease.allows(11, &advance));
        assert!(!lease.allows(22, &advance));
        assert!(lease.allows(
            22,
            &crate::live::ClientMessage::Synchronize {
                last_server_sequence: None,
            }
        ));
        lease.release(11);
        assert!(lease.allows(22, &advance));
    }

    #[test]
    fn human_sequences_are_central_monotonic_and_above_controller_sequences() {
        let mut allocator = LiveHumanSequenceAllocator::default();
        assert_eq!(allocator.allocate(0), FIRST_HUMAN_ACTION_SEQUENCE);
        assert_eq!(allocator.allocate(0), FIRST_HUMAN_ACTION_SEQUENCE + 1);
        assert_eq!(allocator.allocate(1), FIRST_HUMAN_ACTION_SEQUENCE);
    }

    #[test]
    fn later_frames_omit_only_the_static_map_payload() {
        let mut observation = serde_json::json!({
            "map": { "width": 128.0 },
            "entities": [{ "id": 7 }],
            "resource_sites": [{ "position": { "x": 1.0, "z": 2.0 } }],
        });
        strip_static_map(&mut observation);
        assert!(observation.get("map").is_none());
        assert_eq!(observation["entities"][0]["id"], 7);
        assert_eq!(observation["resource_sites"][0]["position"]["x"], 1.0);
    }

    #[test]
    fn refuses_non_loopback_bind() {
        let config = LiveServerConfig {
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            ..LiveServerConfig::default()
        };
        let error = start_live_server(LiveDriver::new(MockWorld { tick: 0 }), config, |_| {
            Value::Null
        })
        .err()
        .expect("must reject public bind");
        assert!(matches!(error, LiveServerError::NonLoopbackBind(_)));
    }

    #[test]
    fn serves_embedded_viewer_and_health_check() {
        let server = start_live_server(
            LiveDriver::new(MockWorld { tick: 0 }),
            LiveServerConfig::default(),
            |world| serde_json::json!({ "tick": world.tick }),
        )
        .unwrap();
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.ends_with("{\"status\":\"ok\"}"));

        let mut viewer = TcpStream::connect(server.local_addr()).unwrap();
        viewer
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut viewer_response = String::new();
        viewer.read_to_string(&mut viewer_response).unwrap();
        assert!(viewer_response.starts_with("HTTP/1.1 200 OK"));
        assert!(viewer_response.contains("<canvas id=\"map\"></canvas>"));
        assert!(viewer_response.contains("/ws"));
        assert!(viewer_response.contains("type:'submit_action'"));
        assert!(viewer_response.contains("data-mode=\"attack\""));
        assert!(viewer_response.contains("data-mode=\"build\""));
        assert!(!viewer_response.contains("data-mode=\"select\""));
        assert!(viewer_response.contains("function clearOrder"));
        assert!(viewer_response.contains("button:disabled"));
        assert!(viewer_response.contains("function refreshControls"));
        assert!(viewer_response.contains("modeCapability"));
        assert!(viewer_response.contains("capabilities"));
        assert!(viewer_response.contains("oncontextmenu"));
        assert!(viewer_response.contains("type:'factory_queue'"));
        assert!(viewer_response.contains("id=\"clear-queue\""));
        assert!(viewer_response.contains("kind:'factory_current'"));
        assert!(viewer_response.contains("obs.fire_intents"));
        assert!(viewer_response.contains("damage_events"));
        assert!(viewer_response.contains("turret_heading"));
        assert!(viewer_response.contains("function facing"));
        assert!(viewer_response.contains("outer arrow = hull facing"));
        assert!(viewer_response.contains("Blue = your units"));
        assert!(viewer_response.contains("control-side"));
        assert!(viewer_response.contains("select_control_side"));
        assert!(viewer_response.contains("gold/red lines"));
        assert!(viewer_response.contains("type:'preview_build'"));
        assert!(viewer_response.contains("c.onpointerdown"));
        assert!(viewer_response.contains("selection cleared"));
        assert!(viewer_response.contains("command rejected"));
        assert!(viewer_response.contains("world executed command"));
        assert!(viewer_response.contains("action_results"));
        assert!(viewer_response.contains("pendingSequence"));
        assert!(viewer_response.contains("obs.resource_sites"));
        assert!(viewer_response.contains("planned_route"));
        assert!(viewer_response.contains("construction_progress"));
        assert!(viewer_response.contains("VISIGOTH_SHARED_VIEWER"));
        assert_eq!(VIEWER_HTML, crate::viewer_protocol::SHARED_VIEWER_HTML);
        let replay = server.shutdown_with_replay().unwrap();
        assert_eq!(replay.final_tick, 0);
        assert!(replay.actions.is_empty());
    }

    #[test]
    fn one_browser_can_switch_sides_but_cannot_forge_orders_for_the_other() {
        let server = start_live_server_with_preview(
            LiveDriver::new(MockWorld { tick: 3 }),
            LiveServerConfig::default(),
            |_, side| serde_json::json!({ "view_side": side }),
            |_, _| Value::Null,
            0,
        )
        .unwrap();
        let (mut socket, _) =
            tungstenite::connect(format!("ws://{}/ws", server.local_addr())).unwrap();
        let send = |socket: &mut tungstenite::WebSocket<
            tungstenite::stream::MaybeTlsStream<TcpStream>,
        >,
                    request_id,
                    message| {
            socket
                .send(Message::Text(
                    serde_json::to_string(&ClientEnvelope {
                        protocol_version: LIVE_PROTOCOL_VERSION,
                        request_id,
                        message,
                    })
                    .unwrap()
                    .into(),
                ))
                .unwrap();
        };

        send(
            &mut socket,
            1,
            crate::live::ClientMessage::Synchronize {
                last_server_sequence: None,
            },
        );
        let _: crate::live::ServerEnvelope =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        let initial: Value =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(initial["observation"]["view_side"], 0);

        send(
            &mut socket,
            2,
            crate::live::ClientMessage::SelectControlSide { side: 1 },
        );
        let _: crate::live::ServerEnvelope =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        let switched: Value =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(switched["observation"]["view_side"], 1);

        let forged = crate::strategy::QueuedAction {
            side: 0,
            sequence: 7,
            apply_at_tick: 3,
            action: crate::strategy::StrategyAction::Move {
                entities: Vec::new(),
                destination: Vec2 { x: 0.0, z: 0.0 },
                mode: crate::strategy::OrderMode::Replace,
            },
        };
        send(
            &mut socket,
            3,
            crate::live::ClientMessage::SubmitAction {
                displayed_tick: 3,
                action: forged,
            },
        );
        let rejected: crate::live::ServerEnvelope =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        assert!(matches!(
            rejected.message,
            ServerMessage::Error {
                error: crate::live::ServerError::ControlledSideMismatch {
                    controlled_side: 1,
                    requested_side: 0,
                },
                ..
            }
        ));
        socket.close(None).unwrap();
        assert!(server.shutdown_with_replay().unwrap().actions.is_empty());
    }

    #[test]
    fn websocket_exchanges_envelopes_and_samples_authoritative_state() {
        let server = start_live_server(
            LiveDriver::new(MockWorld { tick: 3 }),
            LiveServerConfig::default(),
            |world| serde_json::json!({ "sample_tick": world.tick }),
        )
        .unwrap();
        let (mut socket, _) =
            tungstenite::connect(format!("ws://{}/ws", server.local_addr())).unwrap();
        let request = ClientEnvelope {
            protocol_version: LIVE_PROTOCOL_VERSION,
            request_id: 9,
            message: crate::live::ClientMessage::Step { ticks: 2 },
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&request).unwrap().into(),
            ))
            .unwrap();
        let response: crate::live::ServerEnvelope =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(response.request_id, 9);
        let observation: Value =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(observation["type"], "observation");
        assert_eq!(observation["state"]["displayed_tick"], 5);
        assert_eq!(observation["observation"]["sample_tick"], 5);
        socket.close(None).unwrap();
        let replay = server.shutdown_with_replay().unwrap();
        assert_eq!(replay.final_tick, 5);
        assert!(replay.actions.is_empty());
    }
}
