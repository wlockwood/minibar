//! Transport-independent live simulation control.
//!
//! The driver deliberately knows nothing about sockets, HTTP, or rendering. A
//! later adapter can serialize [`ClientEnvelope`] and [`ServerEnvelope`], sample
//! its own richer frame from [`LiveDriver::world`], and use the authoritative
//! tick in [`LiveState`] when constructing actions.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::entity::EntityId;
use crate::model::Vec2;
use crate::placement::BuildFacing;
use crate::strategy::{ActionSequence, QueuedAction, SideId};

pub const LIVE_PROTOCOL_VERSION: u16 = 1;
pub const MAX_SPEED_MULTIPLIER: u32 = 1_024;
pub const MAX_STEPS_PER_REQUEST: u64 = 1_000_000;

/// Minimum interface required by the live driver and replay verifier.
///
/// Action validation and placement authority remain in the world. The driver
/// only provides tick synchronization and transport-level de-duplication.
pub trait StepWorld {
    fn tick(&self) -> u64;
    fn enqueue_action(&mut self, action: QueuedAction);
    fn step(&mut self);
    fn state_hash(&self) -> u64;
}

impl StepWorld for crate::world::World {
    fn tick(&self) -> u64 {
        self.tick
    }

    fn enqueue_action(&mut self, action: QueuedAction) {
        crate::world::World::enqueue_action(self, action);
    }

    fn step(&mut self) {
        crate::world::World::step(self);
    }

    fn state_hash(&self) -> u64 {
        crate::world::World::state_hash(self)
    }
}

/// Exact rational simulation rate. One input quantum normally represents one
/// real-time simulation tick; the rate determines how many world ticks run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SimulationSpeed {
    numerator: u32,
    denominator: u32,
}

impl SimulationSpeed {
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    pub fn new(numerator: u32, denominator: u32) -> Result<Self, SpeedError> {
        if numerator == 0 || denominator == 0 {
            return Err(SpeedError::ZeroComponent);
        }
        if u64::from(numerator) > u64::from(denominator) * u64::from(MAX_SPEED_MULTIPLIER) {
            return Err(SpeedError::TooFast {
                maximum_multiplier: MAX_SPEED_MULTIPLIER,
            });
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

impl Default for SimulationSpeed {
    fn default() -> Self {
        Self::ONE
    }
}

impl<'de> Deserialize<'de> for SimulationSpeed {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireSpeed {
            numerator: u32,
            denominator: u32,
        }

        let wire = WireSpeed::deserialize(deserializer)?;
        Self::new(wire.numerator, wire.denominator).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SpeedError {
    ZeroComponent,
    TooFast { maximum_multiplier: u32 },
}

impl fmt::Display for SpeedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroComponent => formatter.write_str("speed ratio components must be nonzero"),
            Self::TooFast { maximum_multiplier } => write!(
                formatter,
                "speed exceeds the maximum {maximum_multiplier}x multiplier"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideSequence {
    pub side: SideId,
    pub sequence: ActionSequence,
}

/// Small authoritative state sent with acknowledgements and sampled frames.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveState {
    /// Tick that is displayed and is currently accepting commands. The next
    /// call to `step` applies actions addressed to this tick, then increments it.
    pub displayed_tick: u64,
    pub state_hash: u64,
    pub paused: bool,
    pub speed: SimulationSpeed,
    pub accepted_action_count: u64,
    pub last_action_sequences: Vec<SideSequence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoggedAction {
    pub log_index: u64,
    pub received_at_tick: u64,
    pub displayed_tick: u64,
    /// Preserved byte-for-byte at the data-model level; replay never rebuilds
    /// or normalizes the action payload.
    pub action: QueuedAction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayLog {
    pub protocol_version: u16,
    pub initial_tick: u64,
    pub initial_state_hash: u64,
    pub final_tick: u64,
    pub final_state_hash: u64,
    pub actions: Vec<LoggedAction>,
}

/// A read-only, server-authoritative placement query.  The returned preview is
/// only valid for the displayed tick and never reserves map occupancy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildPreviewRequest {
    pub side: SideId,
    pub builder: EntityId,
    pub definition: String,
    pub position: Vec2,
    pub facing: BuildFacing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CommandRejection {
    StaleDisplayedTick {
        displayed_tick: u64,
        authoritative_tick: u64,
    },
    ApplyTickMismatch {
        displayed_tick: u64,
        apply_at_tick: u64,
    },
    SequenceConflict,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandResult {
    Accepted { log_index: u64 },
    Duplicate { log_index: u64 },
    Rejected { rejection: CommandRejection },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAck {
    pub side: SideId,
    pub sequence: ActionSequence,
    pub result: CommandResult,
    pub state: LiveState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceReport {
    pub from_tick: u64,
    pub to_tick: u64,
    pub steps: u64,
    pub state_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ControlError {
    StepLimitExceeded { requested: u64, maximum: u64 },
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StepLimitExceeded { requested, maximum } => {
                write!(
                    formatter,
                    "requested {requested} steps, maximum is {maximum}"
                )
            }
        }
    }
}

/// JSON-friendly request body for a future HTTP/WebSocket adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Synchronize {
        last_server_sequence: Option<u64>,
    },
    /// Changes which side a local live-viewer observes and is permitted to
    /// command. The socket adapter owns that per-client selection; the core
    /// driver treats this as a read-only synchronization request.
    SelectControlSide {
        side: SideId,
    },
    SubmitAction {
        displayed_tick: u64,
        action: QueuedAction,
    },
    PreviewBuild {
        request: BuildPreviewRequest,
    },
    SetPaused {
        paused: bool,
    },
    SetSpeed {
        speed: SimulationSpeed,
    },
    Step {
        ticks: u32,
    },
    Advance {
        real_time_quanta: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientEnvelope {
    pub protocol_version: u16,
    pub request_id: u64,
    pub message: ClientMessage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Synchronized {
        state: LiveState,
    },
    CommandAck {
        ack: CommandAck,
    },
    StateChanged {
        state: LiveState,
    },
    Advanced {
        report: AdvanceReport,
        state: LiveState,
    },
    /// The core driver has no world-specific placement authority.  The local
    /// live server replaces this with a `BuildPreview` response for worlds
    /// that expose a preview adapter.
    PreviewUnavailable {
        state: LiveState,
    },
    BuildPreview {
        preview: serde_json::Value,
        state: LiveState,
    },
    Error {
        error: ServerError,
        state: LiveState,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ServerError {
    ProtocolMismatch {
        expected: u16,
        received: u16,
    },
    Control(ControlError),
    /// The local viewer tried to submit an order for a side other than the
    /// side it currently controls. This is enforced by the socket adapter,
    /// before the order can enter the deterministic world inbox.
    ControlledSideMismatch {
        controlled_side: SideId,
        requested_side: SideId,
    },
    /// MiniBar currently exposes its two playable sides, zero and one.
    InvalidControlledSide {
        requested_side: SideId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerEnvelope {
    pub protocol_version: u16,
    pub server_sequence: u64,
    pub request_id: u64,
    pub message: ServerMessage,
}

/// Owns one authoritative live world. It is intentionally single-threaded;
/// transports may serialize access while batch parallelism stays outside it.
pub struct LiveDriver<W> {
    world: W,
    paused: bool,
    speed: SimulationSpeed,
    speed_remainder: u64,
    initial_tick: u64,
    initial_state_hash: u64,
    next_server_sequence: u64,
    high_water: BTreeMap<SideId, ActionSequence>,
    action_indices: BTreeMap<(SideId, ActionSequence), usize>,
    actions: Vec<LoggedAction>,
}

impl<W: StepWorld> LiveDriver<W> {
    /// A new live session starts paused so clients can synchronize before any
    /// authoritative tick advances.
    pub fn new(world: W) -> Self {
        let initial_tick = world.tick();
        let initial_state_hash = world.state_hash();
        Self {
            world,
            paused: true,
            speed: SimulationSpeed::ONE,
            speed_remainder: 0,
            initial_tick,
            initial_state_hash,
            next_server_sequence: 1,
            high_water: BTreeMap::new(),
            action_indices: BTreeMap::new(),
            actions: Vec::new(),
        }
    }

    pub fn world(&self) -> &W {
        &self.world
    }

    pub fn into_world(self) -> W {
        self.world
    }

    pub fn state(&self) -> LiveState {
        LiveState {
            displayed_tick: self.world.tick(),
            state_hash: self.world.state_hash(),
            paused: self.paused,
            speed: self.speed,
            accepted_action_count: self.actions.len() as u64,
            last_action_sequences: self
                .high_water
                .iter()
                .map(|(&side, &sequence)| SideSequence { side, sequence })
                .collect(),
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn set_speed(&mut self, speed: SimulationSpeed) {
        self.speed = speed;
        self.speed_remainder = 0;
    }

    /// Inject an action created from the indicated displayed state.
    ///
    /// Exact duplicate `(side, sequence, payload)` submissions are acknowledged
    /// before tick checks, making reconnect retries harmless after time advances.
    pub fn submit_action(&mut self, displayed_tick: u64, action: QueuedAction) -> CommandAck {
        let key = (action.side, action.sequence);
        if let Some(&index) = self.action_indices.get(&key) {
            let logged = &self.actions[index];
            let result = if logged.displayed_tick == displayed_tick && logged.action == action {
                CommandResult::Duplicate {
                    log_index: logged.log_index,
                }
            } else {
                CommandResult::Rejected {
                    rejection: CommandRejection::SequenceConflict,
                }
            };
            return self.acknowledgement(action.side, action.sequence, result);
        }

        if self
            .high_water
            .get(&action.side)
            .is_some_and(|last| action.sequence <= *last)
        {
            return self.acknowledgement(
                action.side,
                action.sequence,
                CommandResult::Rejected {
                    rejection: CommandRejection::SequenceConflict,
                },
            );
        }

        let authoritative_tick = self.world.tick();
        if displayed_tick != authoritative_tick {
            return self.acknowledgement(
                action.side,
                action.sequence,
                CommandResult::Rejected {
                    rejection: CommandRejection::StaleDisplayedTick {
                        displayed_tick,
                        authoritative_tick,
                    },
                },
            );
        }
        if action.apply_at_tick != displayed_tick {
            return self.acknowledgement(
                action.side,
                action.sequence,
                CommandResult::Rejected {
                    rejection: CommandRejection::ApplyTickMismatch {
                        displayed_tick,
                        apply_at_tick: action.apply_at_tick,
                    },
                },
            );
        }

        let log_index = self.actions.len() as u64;
        self.world.enqueue_action(action.clone());
        self.high_water.insert(action.side, action.sequence);
        self.action_indices.insert(key, self.actions.len());
        self.actions.push(LoggedAction {
            log_index,
            received_at_tick: authoritative_tick,
            displayed_tick,
            action: action.clone(),
        });
        self.acknowledgement(
            action.side,
            action.sequence,
            CommandResult::Accepted { log_index },
        )
    }

    /// Advances explicitly even while paused; used for frame-by-frame control.
    pub fn step_ticks(&mut self, ticks: u32) -> Result<AdvanceReport, ControlError> {
        self.run_steps(u64::from(ticks))
    }

    /// Advances according to the exact speed ratio when running. Paused time
    /// does not accumulate and therefore cannot cause a resume-time burst.
    pub fn advance(&mut self, real_time_quanta: u32) -> Result<AdvanceReport, ControlError> {
        if self.paused {
            return Ok(self.report(self.world.tick(), 0));
        }
        let scaled = u128::from(self.speed_remainder)
            + u128::from(real_time_quanta) * u128::from(self.speed.numerator);
        let denominator = u128::from(self.speed.denominator);
        let steps = scaled / denominator;
        let steps = u64::try_from(steps).unwrap_or(u64::MAX);
        if steps > MAX_STEPS_PER_REQUEST {
            return Err(ControlError::StepLimitExceeded {
                requested: steps,
                maximum: MAX_STEPS_PER_REQUEST,
            });
        }
        self.speed_remainder = (scaled % denominator) as u64;
        self.run_steps(steps)
    }

    pub fn replay_log(&self) -> ReplayLog {
        ReplayLog {
            protocol_version: LIVE_PROTOCOL_VERSION,
            initial_tick: self.initial_tick,
            initial_state_hash: self.initial_state_hash,
            final_tick: self.world.tick(),
            final_state_hash: self.world.state_hash(),
            actions: self.actions.clone(),
        }
    }

    pub fn handle(&mut self, envelope: ClientEnvelope) -> ServerEnvelope {
        let request_id = envelope.request_id;
        if envelope.protocol_version != LIVE_PROTOCOL_VERSION {
            return self.response(
                request_id,
                ServerMessage::Error {
                    error: ServerError::ProtocolMismatch {
                        expected: LIVE_PROTOCOL_VERSION,
                        received: envelope.protocol_version,
                    },
                    state: self.state(),
                },
            );
        }

        let message = match envelope.message {
            ClientMessage::Synchronize { .. } | ClientMessage::SelectControlSide { .. } => {
                ServerMessage::Synchronized {
                    state: self.state(),
                }
            }
            ClientMessage::SubmitAction {
                displayed_tick,
                action,
            } => ServerMessage::CommandAck {
                ack: self.submit_action(displayed_tick, action),
            },
            ClientMessage::PreviewBuild { .. } => ServerMessage::PreviewUnavailable {
                state: self.state(),
            },
            ClientMessage::SetPaused { paused } => {
                self.set_paused(paused);
                ServerMessage::StateChanged {
                    state: self.state(),
                }
            }
            ClientMessage::SetSpeed { speed } => {
                self.set_speed(speed);
                ServerMessage::StateChanged {
                    state: self.state(),
                }
            }
            ClientMessage::Step { ticks } => match self.step_ticks(ticks) {
                Ok(report) => ServerMessage::Advanced {
                    report,
                    state: self.state(),
                },
                Err(error) => ServerMessage::Error {
                    error: ServerError::Control(error),
                    state: self.state(),
                },
            },
            ClientMessage::Advance { real_time_quanta } => match self.advance(real_time_quanta) {
                Ok(report) => ServerMessage::Advanced {
                    report,
                    state: self.state(),
                },
                Err(error) => ServerMessage::Error {
                    error: ServerError::Control(error),
                    state: self.state(),
                },
            },
        };
        self.response(request_id, message)
    }

    fn acknowledgement(
        &self,
        side: SideId,
        sequence: ActionSequence,
        result: CommandResult,
    ) -> CommandAck {
        CommandAck {
            side,
            sequence,
            result,
            state: self.state(),
        }
    }

    fn run_steps(&mut self, steps: u64) -> Result<AdvanceReport, ControlError> {
        if steps > MAX_STEPS_PER_REQUEST {
            return Err(ControlError::StepLimitExceeded {
                requested: steps,
                maximum: MAX_STEPS_PER_REQUEST,
            });
        }
        let from_tick = self.world.tick();
        for _ in 0..steps {
            self.world.step();
        }
        Ok(self.report(from_tick, steps))
    }

    fn report(&self, from_tick: u64, steps: u64) -> AdvanceReport {
        AdvanceReport {
            from_tick,
            to_tick: self.world.tick(),
            steps,
            state_hash: self.world.state_hash(),
        }
    }

    fn response(&mut self, request_id: u64, message: ServerMessage) -> ServerEnvelope {
        let server_sequence = self.next_server_sequence;
        self.next_server_sequence = self.next_server_sequence.wrapping_add(1);
        ServerEnvelope {
            protocol_version: LIVE_PROTOCOL_VERSION,
            server_sequence,
            request_id,
            message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayVerification {
    pub final_tick: u64,
    pub final_state_hash: u64,
    pub actions_replayed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ReplayError {
    ProtocolMismatch {
        expected: u16,
        received: u16,
    },
    InitialTickMismatch {
        expected: u64,
        actual: u64,
    },
    InitialHashMismatch {
        expected: u64,
        actual: u64,
    },
    InvalidLogIndex {
        expected: u64,
        actual: u64,
    },
    ActionAfterFinalTick {
        action_tick: u64,
        final_tick: u64,
    },
    ActionTickWentBackwards {
        action_tick: u64,
        current_tick: u64,
    },
    ApplyTickMismatch {
        displayed_tick: u64,
        apply_at_tick: u64,
    },
    FinalHashMismatch {
        expected: u64,
        actual: u64,
    },
}

/// Replays accepted actions into a fresh equivalent world and verifies the
/// terminal authoritative hash. Transport retries and playback controls are
/// intentionally absent because they never mutate simulation state.
pub fn replay_and_verify<W: StepWorld>(
    world: &mut W,
    log: &ReplayLog,
) -> Result<ReplayVerification, ReplayError> {
    if log.protocol_version != LIVE_PROTOCOL_VERSION {
        return Err(ReplayError::ProtocolMismatch {
            expected: LIVE_PROTOCOL_VERSION,
            received: log.protocol_version,
        });
    }
    if world.tick() != log.initial_tick {
        return Err(ReplayError::InitialTickMismatch {
            expected: log.initial_tick,
            actual: world.tick(),
        });
    }
    if world.state_hash() != log.initial_state_hash {
        return Err(ReplayError::InitialHashMismatch {
            expected: log.initial_state_hash,
            actual: world.state_hash(),
        });
    }

    for (expected_index, logged) in log.actions.iter().enumerate() {
        if logged.log_index != expected_index as u64 {
            return Err(ReplayError::InvalidLogIndex {
                expected: expected_index as u64,
                actual: logged.log_index,
            });
        }
        if logged.displayed_tick > log.final_tick {
            return Err(ReplayError::ActionAfterFinalTick {
                action_tick: logged.displayed_tick,
                final_tick: log.final_tick,
            });
        }
        if logged.displayed_tick < world.tick() {
            return Err(ReplayError::ActionTickWentBackwards {
                action_tick: logged.displayed_tick,
                current_tick: world.tick(),
            });
        }
        if logged.action.apply_at_tick != logged.displayed_tick {
            return Err(ReplayError::ApplyTickMismatch {
                displayed_tick: logged.displayed_tick,
                apply_at_tick: logged.action.apply_at_tick,
            });
        }
        while world.tick() < logged.displayed_tick {
            world.step();
        }
        world.enqueue_action(logged.action.clone());
    }
    while world.tick() < log.final_tick {
        world.step();
    }

    let actual_hash = world.state_hash();
    if actual_hash != log.final_state_hash {
        return Err(ReplayError::FinalHashMismatch {
            expected: log.final_state_hash,
            actual: actual_hash,
        });
    }
    Ok(ReplayVerification {
        final_tick: world.tick(),
        final_state_hash: actual_hash,
        actions_replayed: log.actions.len() as u64,
    })
}

const fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityId;
    use crate::map::{CompiledMap, HelloMapOptions, MapSource};
    use crate::model::{MovementDef, StrategicDef, UnitDef, Vec2};
    use crate::placement::BuildFacing;
    use crate::strategy::{CancelTarget, StrategyAction};
    use crate::world::{UnitDefId, World, WorldCatalog, WorldConfig};
    use std::sync::Arc;

    #[derive(Clone, Debug)]
    struct MockWorld {
        tick: u64,
        value: u64,
        pending: Vec<QueuedAction>,
        applied: Vec<(u64, u64)>,
    }

    impl MockWorld {
        fn seeded(value: u64) -> Self {
            Self {
                tick: 0,
                value,
                pending: Vec::new(),
                applied: Vec::new(),
            }
        }
    }

    impl StepWorld for MockWorld {
        fn tick(&self) -> u64 {
            self.tick
        }

        fn enqueue_action(&mut self, action: QueuedAction) {
            self.pending.push(action);
        }

        fn step(&mut self) {
            self.pending
                .sort_by_key(|action| (action.apply_at_tick, action.side, action.sequence));
            let mut waiting = Vec::new();
            for action in self.pending.drain(..) {
                if action.apply_at_tick <= self.tick {
                    self.value = self
                        .value
                        .wrapping_mul(1_099_511_628_211)
                        .wrapping_add(action.sequence)
                        .wrapping_add(u64::from(action.side));
                    self.applied.push((action.sequence, self.tick));
                } else {
                    waiting.push(action);
                }
            }
            self.pending = waiting;
            self.tick += 1;
        }

        fn state_hash(&self) -> u64 {
            self.value ^ self.tick.rotate_left(17)
        }
    }

    fn action(side: u8, sequence: u64, tick: u64) -> QueuedAction {
        QueuedAction {
            side,
            sequence,
            apply_at_tick: tick,
            action: StrategyAction::Cancel {
                target: CancelTarget::Orders {
                    entity: EntityId::new(1, 1),
                },
            },
        }
    }

    #[test]
    fn command_applies_on_the_displayed_authoritative_tick() {
        let mut driver = LiveDriver::new(MockWorld::seeded(7));
        driver.step_ticks(4).unwrap();
        let ack = driver.submit_action(4, action(0, 1, 4));
        assert!(matches!(ack.result, CommandResult::Accepted { .. }));

        driver.step_ticks(1).unwrap();
        assert_eq!(driver.world().applied, vec![(1, 4)]);
        assert_eq!(driver.state().displayed_tick, 5);
    }

    #[test]
    fn reconnect_duplicate_is_acknowledged_without_requeueing() {
        let mut driver = LiveDriver::new(MockWorld::seeded(11));
        let submitted = action(1, 42, 0);
        let first = driver.submit_action(0, submitted.clone());
        assert!(matches!(first.result, CommandResult::Accepted { .. }));
        driver.step_ticks(1).unwrap();

        let retry = driver.submit_action(0, submitted);
        assert_eq!(retry.result, CommandResult::Duplicate { log_index: 0 });
        driver.step_ticks(1).unwrap();
        assert_eq!(driver.world().applied, vec![(42, 0)]);
        assert_eq!(driver.replay_log().actions.len(), 1);
    }

    #[test]
    fn pause_manual_step_and_rational_speed_are_exact() {
        let mut driver = LiveDriver::new(MockWorld::seeded(3));
        assert_eq!(driver.advance(10).unwrap().steps, 0);

        driver.set_speed(SimulationSpeed::new(3, 2).unwrap());
        driver.set_paused(false);
        assert_eq!(driver.advance(1).unwrap().steps, 1);
        assert_eq!(driver.advance(1).unwrap().steps, 2);
        driver.set_paused(true);
        assert_eq!(driver.advance(100).unwrap().steps, 0);
        assert_eq!(driver.step_ticks(2).unwrap().steps, 2);
        assert_eq!(driver.world().tick, 5);
    }

    #[test]
    fn replay_reaches_identical_tick_and_state_hash() {
        let mut driver = LiveDriver::new(MockWorld::seeded(0x1234));
        driver.submit_action(0, action(0, 1, 0));
        driver.step_ticks(1).unwrap();
        driver.submit_action(1, action(1, 7, 1));
        driver.step_ticks(3).unwrap();
        driver.submit_action(4, action(0, 9, 4));
        driver.step_ticks(2).unwrap();
        let log = driver.replay_log();

        let mut replay = MockWorld::seeded(0x1234);
        let verification = replay_and_verify(&mut replay, &log).unwrap();
        assert_eq!(verification.final_tick, driver.world().tick);
        assert_eq!(verification.final_state_hash, driver.world().state_hash());
        assert_eq!(replay.applied, driver.world().applied);
    }

    #[test]
    fn messages_round_trip_as_json() {
        let message = ClientEnvelope {
            protocol_version: LIVE_PROTOCOL_VERSION,
            request_id: 17,
            message: ClientMessage::SubmitAction {
                displayed_tick: 2,
                action: action(0, 3, 2),
            },
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientEnvelope>(&json).unwrap(),
            message
        );
    }

    fn real_world() -> (World, EntityId) {
        let source = MapSource::from_hello_json(
            r#"{
                "map_name":"live-replay", "map_size":[128,128], "water_level":0,
                "terrain":{"heights":[[0,0],[0,0]],
                  "movement_classes":{"BOT":[[true,true],[true,true]]}},
                "metal_spots":[], "geothermal_spots":[]
            }"#,
            HelloMapOptions::default(),
        )
        .unwrap();
        let map = Arc::new(CompiledMap::compile(source).unwrap());
        let unit = UnitDef {
            name: "runner".into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 0.0,
            energy_cost: 0.0,
            strategic: StrategicDef {
                is_mobile: true,
                movement_class: Some("BOT".into()),
                ..StrategicDef::default()
            },
            movement: MovementDef {
                max_speed: 30.0,
                acceleration: 120.0,
                braking: 120.0,
                radius: 4.0,
                ..MovementDef::default()
            },
            weapons: Vec::new(),
        };
        let catalog = Arc::new(WorldCatalog::new(vec![unit]).unwrap());
        let mut world = World::new(map, catalog, WorldConfig::default()).unwrap();
        let entity = world
            .spawn_active(0, UnitDefId(0), Vec2::new(16.0, 16.0), BuildFacing::South)
            .unwrap();
        (world, entity)
    }

    #[test]
    fn replay_verifies_a_real_world_command_log() {
        let (world, entity) = real_world();
        let mut driver = LiveDriver::new(world);
        let queued = QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: vec![entity],
                destination: Vec2::new(96.0, 16.0),
                mode: crate::strategy::OrderMode::Replace,
            },
        };
        assert!(matches!(
            driver.submit_action(0, queued).result,
            CommandResult::Accepted { .. }
        ));
        driver.step_ticks(8).unwrap();
        let log = driver.replay_log();
        let final_position = driver.world().entities.get(entity).unwrap().position;
        assert!(final_position.x > 16.0);

        let (mut replay, replay_entity) = real_world();
        assert_eq!(entity, replay_entity);
        let verified = replay_and_verify(&mut replay, &log).unwrap();
        assert_eq!(verified.final_state_hash, driver.world().state_hash());
        assert_eq!(
            replay.entities.get(entity).unwrap().position,
            final_position
        );
    }
}
