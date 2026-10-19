//! Stateful, engine-independent strategy commands and declarative experiments.
//!
//! Strategies observe the last completed tick and emit actions only on their
//! configured cadence. The world owns validation and execution: this module
//! supplies deterministic ordering, stable rejection codes, and serializable
//! descriptions without coupling strategies to tactical or economy internals.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::entity::EntityId;
use crate::model::Vec2;
use crate::placement::BuildFacing;

pub type SideId = u8;
pub type ActionSequence = u64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderMode {
    #[default]
    Replace,
    Append,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttackTarget {
    Entity { entity: EntityId },
    Ground { position: Vec2 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReclaimTarget {
    Entity { entity: EntityId },
    Area { position: Vec2, radius: f32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactoryQueueItem {
    pub definition: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum FactoryQueueOperation {
    Append { item: FactoryQueueItem },
    Prepend { item: FactoryQueueItem },
    Replace { items: Vec<FactoryQueueItem> },
    Cancel { index: usize },
    Clear,
    SetRepeat { enabled: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CancelTarget {
    Orders { entity: EntityId },
    Project { entity: EntityId },
    FactoryCurrent { entity: EntityId },
}

/// Complete set of strategy-issued commands. Execution remains a world phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StrategyAction {
    Move {
        entities: Vec<EntityId>,
        destination: Vec2,
        mode: OrderMode,
    },
    Attack {
        entities: Vec<EntityId>,
        target: AttackTarget,
        mode: OrderMode,
    },
    Build {
        builder: EntityId,
        definition: String,
        position: Vec2,
        facing: BuildFacing,
        mode: OrderMode,
    },
    Assist {
        entities: Vec<EntityId>,
        target: EntityId,
        mode: OrderMode,
    },
    Repair {
        entities: Vec<EntityId>,
        target: EntityId,
        mode: OrderMode,
    },
    FactoryQueue {
        factory: EntityId,
        operation: FactoryQueueOperation,
    },
    SetActivation {
        entities: Vec<EntityId>,
        active: bool,
    },
    SetCloak {
        entities: Vec<EntityId>,
        cloaked: bool,
    },
    Reclaim {
        entities: Vec<EntityId>,
        target: ReclaimTarget,
        mode: OrderMode,
    },
    Cancel {
        target: CancelTarget,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedAction {
    pub side: SideId,
    pub sequence: ActionSequence,
    /// First tick on which validation may consume this action.
    pub apply_at_tick: u64,
    pub action: StrategyAction,
}

/// Transport-neutral pending action queue. Ready actions are always returned
/// in `(side, sequence)` order, independent of producer or insertion order.
#[derive(Clone, Debug, Default)]
pub struct ActionInbox {
    pending: VecDeque<QueuedAction>,
}

impl ActionInbox {
    pub fn push(&mut self, action: QueuedAction) {
        self.pending.push_back(action);
    }

    pub fn extend(&mut self, actions: impl IntoIterator<Item = QueuedAction>) {
        self.pending.extend(actions);
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn drain_ready(&mut self, tick: u64) -> Vec<QueuedAction> {
        let mut ready = Vec::new();
        let mut waiting = VecDeque::with_capacity(self.pending.len());
        while let Some(action) = self.pending.pop_front() {
            if action.apply_at_tick <= tick {
                ready.push(action);
            } else {
                waiting.push_back(action);
            }
        }
        self.pending = waiting;
        ready.sort_by_key(|action| (action.side, action.sequence));
        ready
    }
}

/// Stable public rejection contract. Numeric values are append-only and may be
/// persisted in experiment output; variants must never be reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
#[serde(rename_all = "snake_case")]
pub enum ActionRejection {
    InvalidSide = 1,
    SequenceAlreadySeen = 2,
    EmptySelection = 3,
    DuplicateEntity = 4,
    UnknownEntity = 5,
    EntityDestroyed = 6,
    NotOwned = 7,
    CapabilityMissing = 8,
    InvalidPosition = 9,
    UnknownDefinition = 10,
    BuildOptionUnavailable = 11,
    InvalidTarget = 12,
    FriendlyTarget = 13,
    TargetNotObserved = 14,
    InvalidQuantity = 15,
    QueueIndexOutOfRange = 16,
    PlacementRejected = 17,
}

impl ActionRejection {
    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidSide => "invalid_side",
            Self::SequenceAlreadySeen => "sequence_already_seen",
            Self::EmptySelection => "empty_selection",
            Self::DuplicateEntity => "duplicate_entity",
            Self::UnknownEntity => "unknown_entity",
            Self::EntityDestroyed => "entity_destroyed",
            Self::NotOwned => "not_owned",
            Self::CapabilityMissing => "capability_missing",
            Self::InvalidPosition => "invalid_position",
            Self::UnknownDefinition => "unknown_definition",
            Self::BuildOptionUnavailable => "build_option_unavailable",
            Self::InvalidTarget => "invalid_target",
            Self::FriendlyTarget => "friendly_target",
            Self::TargetNotObserved => "target_not_observed",
            Self::InvalidQuantity => "invalid_quantity",
            Self::QueueIndexOutOfRange => "queue_index_out_of_range",
            Self::PlacementRejected => "placement_rejected",
        }
    }
}

impl fmt::Display for ActionRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCapabilities {
    pub move_order: bool,
    pub attack: bool,
    pub build: bool,
    pub assist: bool,
    pub repair: bool,
    pub factory_queue: bool,
    pub activation: bool,
    #[serde(default)]
    pub cloak: bool,
    pub reclaim: bool,
    pub cancel: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyEntityKind {
    /// Sensor contact whose concrete unit role has not been identified.
    Unknown,
    Mobile,
    Structure,
    Nanoframe,
    Wall,
    Factory,
    Feature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionEntity {
    pub side: SideId,
    pub kind: StrategyEntityKind,
    pub alive: bool,
    pub capabilities: ActionCapabilities,
}

/// Minimal authoritative query seam needed to validate strategy actions.
/// Placement legality deliberately delegates to the world so map, footprint,
/// reservation, resource-site, and build-range rules have one implementation.
pub trait ActionWorld {
    fn is_valid_side(&self, side: SideId) -> bool;
    fn entity(&self, entity: EntityId) -> Option<ActionEntity>;
    fn is_position_valid(&self, position: Vec2) -> bool;
    fn is_observed(&self, side: SideId, entity: EntityId) -> bool;
    fn definition_exists(&self, definition: &str) -> bool;
    fn builder_can_build(&self, builder: EntityId, definition: &str) -> bool;
    fn factory_can_build(&self, factory: EntityId, definition: &str) -> bool;
    fn factory_queue_len(&self, factory: EntityId) -> usize;

    fn placement_rejection(
        &self,
        _side: SideId,
        _builder: EntityId,
        _definition: &str,
        _position: Vec2,
        _facing: BuildFacing,
    ) -> Option<ActionRejection> {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionDecision {
    pub queued: QueuedAction,
    pub result: Result<(), ActionRejection>,
}

/// Remembers consumed sequences to make reconnect/replay duplicates harmless.
#[derive(Clone, Debug, Default)]
pub struct ActionValidator {
    last_sequence: BTreeMap<SideId, ActionSequence>,
}

impl ActionValidator {
    pub fn last_sequence(&self, side: SideId) -> Option<ActionSequence> {
        self.last_sequence.get(&side).copied()
    }

    pub fn validate_ready(
        &mut self,
        tick: u64,
        inbox: &mut ActionInbox,
        world: &impl ActionWorld,
    ) -> Vec<ActionDecision> {
        inbox
            .drain_ready(tick)
            .into_iter()
            .map(|queued| {
                let result = self.validate_envelope(&queued, world);
                ActionDecision { queued, result }
            })
            .collect()
    }

    pub fn validate_envelope(
        &mut self,
        queued: &QueuedAction,
        world: &impl ActionWorld,
    ) -> Result<(), ActionRejection> {
        if !world.is_valid_side(queued.side) {
            return Err(ActionRejection::InvalidSide);
        }
        if self
            .last_sequence
            .get(&queued.side)
            .is_some_and(|last| queued.sequence <= *last)
        {
            return Err(ActionRejection::SequenceAlreadySeen);
        }
        // A syntactically valid sequence is consumed even when its action is
        // rejected, preventing a client from changing and replaying history.
        self.last_sequence.insert(queued.side, queued.sequence);
        validate_action(queued.side, &queued.action, world)
    }
}

pub fn validate_action(
    side: SideId,
    action: &StrategyAction,
    world: &impl ActionWorld,
) -> Result<(), ActionRejection> {
    match action {
        StrategyAction::Move {
            entities,
            destination,
            ..
        } => {
            validate_entities(side, entities, |capability| capability.move_order, world)?;
            validate_position(*destination, world)
        }
        StrategyAction::Attack {
            entities, target, ..
        } => {
            validate_entities(side, entities, |capability| capability.attack, world)?;
            match target {
                AttackTarget::Ground { position } => validate_position(*position, world),
                AttackTarget::Entity { entity } => {
                    let target = validate_target(*entity, world)?;
                    if target.side == side {
                        return Err(ActionRejection::FriendlyTarget);
                    }
                    if !world.is_observed(side, *entity) {
                        return Err(ActionRejection::TargetNotObserved);
                    }
                    Ok(())
                }
            }
        }
        StrategyAction::Build {
            builder,
            definition,
            position,
            facing,
            ..
        } => {
            validate_entity(side, *builder, |capability| capability.build, world)?;
            validate_position(*position, world)?;
            validate_definition(definition, world)?;
            if !world.builder_can_build(*builder, definition) {
                return Err(ActionRejection::BuildOptionUnavailable);
            }
            if let Some(rejection) =
                world.placement_rejection(side, *builder, definition, *position, *facing)
            {
                return Err(rejection);
            }
            Ok(())
        }
        StrategyAction::Assist {
            entities, target, ..
        } => {
            validate_entities(side, entities, |capability| capability.assist, world)?;
            validate_friendly_target(side, *target, world)
        }
        StrategyAction::Repair {
            entities, target, ..
        } => {
            validate_entities(side, entities, |capability| capability.repair, world)?;
            validate_friendly_target(side, *target, world)
        }
        StrategyAction::FactoryQueue { factory, operation } => {
            validate_entity(side, *factory, |capability| capability.factory_queue, world)?;
            validate_factory_operation(*factory, operation, world)
        }
        StrategyAction::SetActivation { entities, .. } => {
            validate_entities(side, entities, |capability| capability.activation, world)
        }
        StrategyAction::SetCloak { entities, .. } => {
            validate_entities(side, entities, |capability| capability.cloak, world)
        }
        StrategyAction::Reclaim {
            entities, target, ..
        } => {
            validate_entities(side, entities, |capability| capability.reclaim, world)?;
            match target {
                ReclaimTarget::Entity { entity } => validate_target(*entity, world).map(|_| ()),
                ReclaimTarget::Area { position, radius } => {
                    validate_position(*position, world)?;
                    if !radius.is_finite() || *radius <= 0.0 {
                        return Err(ActionRejection::InvalidQuantity);
                    }
                    Ok(())
                }
            }
        }
        StrategyAction::Cancel { target } => {
            let entity = match target {
                CancelTarget::Orders { entity }
                | CancelTarget::Project { entity }
                | CancelTarget::FactoryCurrent { entity } => *entity,
            };
            validate_entity(side, entity, |capability| capability.cancel, world)
        }
    }
}

fn validate_factory_operation(
    factory: EntityId,
    operation: &FactoryQueueOperation,
    world: &impl ActionWorld,
) -> Result<(), ActionRejection> {
    match operation {
        FactoryQueueOperation::Append { item } | FactoryQueueOperation::Prepend { item } => {
            validate_queue_item(factory, item, world)
        }
        FactoryQueueOperation::Replace { items } => {
            for item in items {
                validate_queue_item(factory, item, world)?;
            }
            Ok(())
        }
        FactoryQueueOperation::Cancel { index } => {
            if *index >= world.factory_queue_len(factory) {
                Err(ActionRejection::QueueIndexOutOfRange)
            } else {
                Ok(())
            }
        }
        FactoryQueueOperation::Clear | FactoryQueueOperation::SetRepeat { .. } => Ok(()),
    }
}

fn validate_queue_item(
    factory: EntityId,
    item: &FactoryQueueItem,
    world: &impl ActionWorld,
) -> Result<(), ActionRejection> {
    if item.count == 0 {
        return Err(ActionRejection::InvalidQuantity);
    }
    validate_definition(&item.definition, world)?;
    if !world.factory_can_build(factory, &item.definition) {
        return Err(ActionRejection::BuildOptionUnavailable);
    }
    Ok(())
}

fn validate_entities(
    side: SideId,
    entities: &[EntityId],
    has_capability: impl Fn(ActionCapabilities) -> bool,
    world: &impl ActionWorld,
) -> Result<(), ActionRejection> {
    if entities.is_empty() {
        return Err(ActionRejection::EmptySelection);
    }
    let mut ordered = entities.to_vec();
    ordered.sort_unstable();
    if ordered.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ActionRejection::DuplicateEntity);
    }
    // Sort before querying so the first rejected entity does not depend on UI
    // selection order.
    for entity in ordered {
        validate_entity(side, entity, &has_capability, world)?;
    }
    Ok(())
}

fn validate_entity(
    side: SideId,
    entity: EntityId,
    has_capability: impl Fn(ActionCapabilities) -> bool,
    world: &impl ActionWorld,
) -> Result<(), ActionRejection> {
    let Some(found) = world.entity(entity) else {
        return Err(ActionRejection::UnknownEntity);
    };
    if !found.alive {
        return Err(ActionRejection::EntityDestroyed);
    }
    if found.side != side {
        return Err(ActionRejection::NotOwned);
    }
    if !has_capability(found.capabilities) {
        return Err(ActionRejection::CapabilityMissing);
    }
    Ok(())
}

fn validate_target(
    entity: EntityId,
    world: &impl ActionWorld,
) -> Result<ActionEntity, ActionRejection> {
    let target = world.entity(entity).ok_or(ActionRejection::InvalidTarget)?;
    if !target.alive {
        return Err(ActionRejection::InvalidTarget);
    }
    Ok(target)
}

fn validate_friendly_target(
    side: SideId,
    entity: EntityId,
    world: &impl ActionWorld,
) -> Result<(), ActionRejection> {
    let target = validate_target(entity, world)?;
    if target.side != side {
        return Err(ActionRejection::InvalidTarget);
    }
    Ok(())
}

fn validate_definition(definition: &str, world: &impl ActionWorld) -> Result<(), ActionRejection> {
    if definition.is_empty() || !world.definition_exists(definition) {
        Err(ActionRejection::UnknownDefinition)
    } else {
        Ok(())
    }
}

fn validate_position(position: Vec2, world: &impl ActionWorld) -> Result<(), ActionRejection> {
    if !position.x.is_finite() || !position.z.is_finite() || !world.is_position_valid(position) {
        Err(ActionRejection::InvalidPosition)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct StrategyCadence {
    interval_ticks: u32,
    phase_tick: u32,
}

impl StrategyCadence {
    pub const EVERY_TICK: Self = Self {
        interval_ticks: 1,
        phase_tick: 0,
    };

    pub const fn new(interval_ticks: u32, phase_tick: u32) -> Result<Self, StrategyConfigError> {
        if interval_ticks == 0 {
            Err(StrategyConfigError::ZeroCadence)
        } else if phase_tick >= interval_ticks {
            Err(StrategyConfigError::PhaseOutsideCadence)
        } else {
            Ok(Self {
                interval_ticks,
                phase_tick,
            })
        }
    }

    pub const fn interval_ticks(self) -> u32 {
        self.interval_ticks
    }

    pub const fn phase_tick(self) -> u32 {
        self.phase_tick
    }
}

impl<'de> Deserialize<'de> for StrategyCadence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawCadence {
            interval_ticks: u32,
            phase_tick: u32,
        }

        let raw = RawCadence::deserialize(deserializer)?;
        Self::new(raw.interval_ticks, raw.phase_tick).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrategyConfigError {
    ZeroCadence,
    PhaseOutsideCadence,
}

impl fmt::Display for StrategyConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroCadence => "strategy cadence must be at least one tick",
            Self::PhaseOutsideCadence => "strategy cadence phase must be less than its interval",
        })
    }
}

impl std::error::Error for StrategyConfigError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceObservation {
    pub stored: f64,
    pub capacity: f64,
    pub income_per_second: f64,
    pub expense_per_second: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationVisibility {
    Own,
    LineOfSight,
    Radar,
    Sonar,
    /// An immobile seen in LOS. Position and identity are retained, never
    /// current health; the structure may have been destroyed out of sight.
    Ghost,
    Remembered,
}

/// Controls how a strategic side observes enemy entities. The default world
/// uses `Omniscient`, preserving the original zero-filtering behavior. Sensor
/// work is only performed when `Filtered` is requested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ObservationPolicy {
    #[default]
    Omniscient,
    Filtered {
        /// Maximum horizontal range at which an owned entity can identify an
        /// enemy, subject to terrain. Units do not occlude sensor coverage.
        line_of_sight_range: f32,
        /// Fallback horizontal radar range. Ground radar has terrain shadows;
        /// concrete catalog radii override fallbacks.
        radar_range: f32,
        /// Radar error magnitude in elmos (legacy field name). Positions drift
        /// smoothly rather than snapping to a grid. Recoil default is96.
        /// Zero explicitly disables positional error for a controlled fixture.
        #[serde(default = "default_radar_error", alias = "radar_error_size")]
        radar_position_resolution: f32,
        /// Retain optional mobile last-seen memory after contact loss. It is
        /// stale and cannot be targeted as a current entity contact. Building
        /// ghosts always persist independently of this switch.
        #[serde(default)]
        remember_last_seen: bool,
    },
}

fn default_radar_error() -> f32 {
    96.0
}

impl ObservationPolicy {
    /// Builds a filtered world policy from a scenario's sensor switches.
    pub fn from_rules(
        rules: ObservationRules,
        line_of_sight_range: f32,
        radar_range: f32,
        radar_position_resolution: f32,
    ) -> Self {
        Self::Filtered {
            line_of_sight_range: if rules.line_of_sight {
                finite_nonnegative(line_of_sight_range)
            } else {
                0.0
            },
            radar_range: if rules.radar {
                finite_nonnegative(radar_range)
            } else {
                0.0
            },
            radar_position_resolution: finite_nonnegative(radar_position_resolution),
            remember_last_seen: rules.remember_last_seen,
        }
    }
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedEntity {
    pub id: EntityId,
    pub side: SideId,
    pub definition: Option<String>,
    pub kind: StrategyEntityKind,
    pub position: Vec2,
    pub health_fraction: Option<f32>,
    pub visibility: ObservationVisibility,
    /// Simulation seconds since a sensor last detected this entity; zero for
    /// contacts detected now.
    #[serde(default)]
    pub age_seconds: f32,
    /// Elmos around `position` within which the entity is known to be: zero
    /// for exact contacts and building ghosts, the radar error for radar and
    /// sonar blips, and for remembered mobile contacts that error plus the
    /// fastest the entity could have moved since (capped at the map diagonal).
    #[serde(default)]
    pub position_error: f32,
}

/// One issued attack intent for symbolic viewers. The endpoint is derived
/// from this side's disclosed target position, so it does not reveal hidden
/// movement behind radar quantization or last-seen memory.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedFireIntent {
    pub source: EntityId,
    pub target_position: Vec2,
    /// The current intent is prevented by an owned wall before the tactical
    /// engine can emit a round. Other obstruction states remain deliberately
    /// unspecified so observations do not leak hidden units.
    #[serde(default)]
    pub blocked_by_friendly_wall: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedResourceSite {
    pub id: u32,
    pub position: Vec2,
    pub metal: f32,
    pub geothermal: bool,
    pub occupied_by: Option<SideId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyObservation {
    pub tick: u64,
    pub side: SideId,
    pub map_size: Vec2,
    pub metal: ResourceObservation,
    pub energy: ResourceObservation,
    /// Must be emitted in entity-ID order by the world.
    pub entities: Vec<ObservedEntity>,
    /// Current attack orders for owned, observed source entities.
    #[serde(default)]
    pub fire_intents: Vec<ObservedFireIntent>,
    /// Must be emitted in site-ID order by the world.
    pub resource_sites: Vec<ObservedResourceSite>,
}

impl StrategyObservation {
    pub fn entity(&self, id: EntityId) -> Option<&ObservedEntity> {
        self.entities.iter().find(|entity| entity.id == id)
    }

    pub fn nearest_enemy(&self, position: Vec2) -> Option<&ObservedEntity> {
        self.entities
            .iter()
            .filter(|entity| entity.side != self.side)
            .min_by(|left, right| {
                position
                    .distance_squared(left.position)
                    .total_cmp(&position.distance_squared(right.position))
                    .then_with(|| left.id.cmp(&right.id))
            })
    }
}

pub trait Strategy: Send {
    fn update(&mut self, observation: &StrategyObservation, actions: &mut ActionEmitter);
}

#[derive(Clone, Debug)]
pub struct ActionEmitter {
    side: SideId,
    apply_at_tick: u64,
    next_sequence: ActionSequence,
    emitted: Vec<QueuedAction>,
}

impl ActionEmitter {
    fn new(side: SideId, apply_at_tick: u64, next_sequence: ActionSequence) -> Self {
        Self {
            side,
            apply_at_tick,
            next_sequence,
            emitted: Vec::new(),
        }
    }

    pub fn emit(&mut self, action: StrategyAction) -> ActionSequence {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("strategy action sequence exhausted");
        self.emitted.push(QueuedAction {
            side: self.side,
            sequence,
            apply_at_tick: self.apply_at_tick,
            action,
        });
        sequence
    }
}

/// Owns one stateful strategy, its cadence clock, and its sequence allocator.
#[derive(Debug)]
pub struct StrategyController<S> {
    side: SideId,
    cadence: StrategyCadence,
    next_update_tick: u64,
    next_sequence: ActionSequence,
    strategy: S,
}

impl<S: Strategy> StrategyController<S> {
    pub fn new(side: SideId, cadence: StrategyCadence, strategy: S) -> Self {
        Self {
            side,
            cadence,
            next_update_tick: u64::from(cadence.phase_tick),
            next_sequence: 1,
            strategy,
        }
    }

    pub fn side(&self) -> SideId {
        self.side
    }

    pub fn strategy(&self) -> &S {
        &self.strategy
    }

    pub fn strategy_mut(&mut self) -> &mut S {
        &mut self.strategy
    }

    pub fn poll(&mut self, observation: &StrategyObservation) -> Vec<QueuedAction> {
        if observation.side != self.side || observation.tick < self.next_update_tick {
            return Vec::new();
        }
        let mut emitter = ActionEmitter::new(self.side, observation.tick, self.next_sequence);
        self.strategy.update(observation, &mut emitter);
        self.next_sequence = emitter.next_sequence;

        let interval = u64::from(self.cadence.interval_ticks);
        // Preserve cadence alignment when a caller jumps over several ticks.
        let elapsed = observation.tick - u64::from(self.cadence.phase_tick);
        self.next_update_tick = u64::from(self.cadence.phase_tick)
            .saturating_add((elapsed / interval + 1).saturating_mul(interval));
        emitter.emitted
    }
}

// --- Declarative experiment data -----------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioKind {
    Formation,
    Opening,
    Expansion,
    Raid,
    WallDefense,
    EconomyRace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeScenario {
    pub name: String,
    pub kind: ScenarioKind,
    pub map: String,
    pub max_ticks: u64,
    pub seed: u64,
    pub observation: ObservationRules,
    pub sides: Vec<ScenarioSide>,
    pub formations: Vec<FormationSpec>,
    pub openings: Vec<OpeningPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationRules {
    pub line_of_sight: bool,
    pub radar: bool,
    pub remember_last_seen: bool,
}

impl Default for ObservationRules {
    fn default() -> Self {
        Self {
            line_of_sight: true,
            radar: true,
            remember_last_seen: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioSide {
    pub side: SideId,
    pub team: u8,
    pub start_position: Vec2,
    pub starting_metal: f64,
    pub starting_energy: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormationMember {
    pub label: String,
    pub definition: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormationSpec {
    pub side: SideId,
    pub anchor: Vec2,
    pub heading: f32,
    pub members: Vec<FormationMember>,
    pub pattern: FormationPattern,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FormationPattern {
    Line {
        spacing: f32,
    },
    Grid {
        columns: u32,
        column_spacing: f32,
        row_spacing: f32,
    },
    Ring {
        radius: f32,
    },
    Custom {
        offsets: Vec<Vec2>,
    },
}

impl FormationSpec {
    /// Returns `(label, definition, position)` in declaration order.
    pub fn materialize(&self) -> Vec<(String, String, Vec2)> {
        let expanded = self
            .members
            .iter()
            .flat_map(|member| {
                (0..member.count).map(move |_| (member.label.clone(), member.definition.clone()))
            })
            .collect::<Vec<_>>();
        let count = expanded.len();
        expanded
            .into_iter()
            .enumerate()
            .map(|(index, (label, definition))| {
                let offset = formation_offset(index, count, &self.pattern).rotate(self.heading);
                (label, definition, self.anchor + offset)
            })
            .collect()
    }
}

fn formation_offset(index: usize, count: usize, pattern: &FormationPattern) -> Vec2 {
    match pattern {
        FormationPattern::Line { spacing } => Vec2::new(
            (index as f32 - (count.saturating_sub(1)) as f32 * 0.5) * spacing,
            0.0,
        ),
        FormationPattern::Grid {
            columns,
            column_spacing,
            row_spacing,
        } => {
            let columns = (*columns).max(1) as usize;
            let rows = count.div_ceil(columns).max(1);
            let column = index % columns;
            let row = index / columns;
            Vec2::new(
                (column as f32 - (columns - 1) as f32 * 0.5) * column_spacing,
                (row as f32 - (rows - 1) as f32 * 0.5) * row_spacing,
            )
        }
        FormationPattern::Ring { radius } => {
            if count == 0 {
                Vec2::default()
            } else {
                let angle = std::f32::consts::TAU * index as f32 / count as f32;
                Vec2::from_heading(angle) * *radius
            }
        }
        FormationPattern::Custom { offsets } => offsets.get(index).copied().unwrap_or_default(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntitySelector {
    Label { label: String },
    Definition { definition: String },
    AllBuilders,
    AllFactories,
    AllCombatUnits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpeningPlan {
    pub name: String,
    pub side: SideId,
    pub steps: Vec<OpeningStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpeningStep {
    pub at_tick: u64,
    pub actor: EntitySelector,
    pub instruction: OpeningInstruction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpeningInstruction {
    Move {
        destination: Vec2,
    },
    Attack {
        target: EntitySelector,
    },
    Build {
        definition: String,
        position: Vec2,
        facing: BuildFacing,
    },
    Assist {
        target: EntitySelector,
    },
    FactoryQueue {
        items: Vec<FactoryQueueItem>,
    },
    SetActivation {
        active: bool,
    },
    ReclaimArea {
        position: Vec2,
        radius: f32,
    },
    Cancel,
}

// --- Reference geometry and a scriptable strategy ------------------------

pub fn kite_destination(unit: Vec2, threat: Vec2, retreat_distance: f32) -> Vec2 {
    unit + (unit - threat).normalized() * retreat_distance.max(0.0)
}

pub fn orbit_destination(unit: Vec2, target: Vec2, clockwise: bool, distance: f32) -> Vec2 {
    let radial = (unit - target).normalized();
    let tangent = if clockwise {
        Vec2::new(radial.z, -radial.x)
    } else {
        Vec2::new(-radial.z, radial.x)
    };
    unit + tangent * distance.max(0.0)
}

pub fn envelopment_positions(center: Vec2, radius: f32, count: usize) -> Vec<Vec2> {
    if count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / count as f32;
            center + Vec2::from_heading(angle) * radius.max(0.0)
        })
        .collect()
}

pub fn wall_segment_positions(from: Vec2, to: Vec2, spacing: f32) -> Vec<Vec2> {
    let distance = from.distance(to);
    let spacing = spacing.max(f32::EPSILON);
    let segments = (distance / spacing).ceil().max(1.0) as usize;
    (0..=segments)
        .map(|index| from + (to - from) * (index as f32 / segments as f32))
        .collect()
}

/// Reference expansion choice: nearest unoccupied site, with site ID as the
/// deterministic tie-break. `geothermal` selects geo versus metal sites.
pub fn nearest_open_resource_site(
    observation: &StrategyObservation,
    from: Vec2,
    geothermal: bool,
) -> Option<&ObservedResourceSite> {
    observation
        .resource_sites
        .iter()
        .filter(|site| site.geothermal == geothermal && site.occupied_by.is_none())
        .min_by(|left, right| {
            from.distance_squared(left.position)
                .total_cmp(&from.distance_squared(right.position))
                .then_with(|| left.id.cmp(&right.id))
        })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimedAction {
    pub at_tick: u64,
    pub action: StrategyAction,
}

/// Useful as a reference opening and as a deterministic command-log fixture.
#[derive(Clone, Debug, Default)]
pub struct ScriptedStrategy {
    actions: Vec<TimedAction>,
    next: usize,
}

impl ScriptedStrategy {
    pub fn new(mut actions: Vec<TimedAction>) -> Self {
        // Stable sort preserves declaration order for equal-tick actions.
        actions.sort_by_key(|action| action.at_tick);
        Self { actions, next: 0 }
    }

    pub fn is_complete(&self) -> bool {
        self.next == self.actions.len()
    }
}

impl Strategy for ScriptedStrategy {
    fn update(&mut self, observation: &StrategyObservation, actions: &mut ActionEmitter) {
        while let Some(action) = self.actions.get(self.next) {
            if action.at_tick > observation.tick {
                break;
            }
            actions.emit(action.action.clone());
            self.next += 1;
        }
    }
}

/// Closed-loop adaptive controller that coordinates mobile combat units to
/// envelop enemy forces from multiple flanking angles.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopmentStrategy {
    pub envelopment_radius: f32,
    pub combat_range: f32,
    pub retreat_health_fraction: f32,
    pub default_destination: Option<Vec2>,
    pub target_entity: Option<EntityId>,
}

impl EnvelopmentStrategy {
    pub fn new(envelopment_radius: f32, combat_range: f32) -> Self {
        Self {
            envelopment_radius,
            combat_range,
            retreat_health_fraction: 0.25,
            default_destination: None,
            target_entity: None,
        }
    }

    pub fn with_retreat_health_fraction(mut self, fraction: f32) -> Self {
        self.retreat_health_fraction = fraction.clamp(0.0, 1.0);
        self
    }

    pub fn with_default_destination(mut self, destination: Vec2) -> Self {
        self.default_destination = Some(destination);
        self
    }
}

impl Strategy for EnvelopmentStrategy {
    fn update(&mut self, observation: &StrategyObservation, actions: &mut ActionEmitter) {
        let mut combat_units = observation
            .entities
            .iter()
            .filter(|e| e.side == observation.side && e.kind == StrategyEntityKind::Mobile)
            .collect::<Vec<_>>();
        if combat_units.is_empty() {
            return;
        }
        combat_units.sort_by_key(|e| e.id);

        let enemies = observation
            .entities
            .iter()
            .filter(|e| e.side != observation.side)
            .collect::<Vec<_>>();

        if enemies.is_empty() {
            self.target_entity = None;
            if let Some(destination) = self.default_destination {
                let ids = combat_units.iter().map(|u| u.id).collect();
                actions.emit(StrategyAction::Move {
                    entities: ids,
                    destination,
                    mode: OrderMode::Replace,
                });
            }
            return;
        }

        // Verify current target is still visible and alive
        if let Some(target_id) = self.target_entity
            && !enemies.iter().any(|e| e.id == target_id)
        {
            self.target_entity = None;
        }

        // Select nearest enemy to centroid of combat units if no active target
        let target = if let Some(target_id) = self.target_entity {
            enemies.iter().find(|e| e.id == target_id).copied()
        } else {
            let centroid = combat_units
                .iter()
                .fold(Vec2::default(), |acc, u| acc + u.position)
                * (1.0 / combat_units.len() as f32);
            let nearest = enemies.iter().min_by(|a, b| {
                centroid
                    .distance_squared(a.position)
                    .total_cmp(&centroid.distance_squared(b.position))
                    .then_with(|| a.id.cmp(&b.id))
            });
            if let Some(t) = nearest {
                self.target_entity = Some(t.id);
            }
            nearest.copied()
        };

        let Some(target) = target else {
            return;
        };

        let count = combat_units.len();
        let flank_positions =
            envelopment_positions(target.position, self.envelopment_radius, count);

        for (i, unit) in combat_units.into_iter().enumerate() {
            let hp_frac = unit.health_fraction.unwrap_or(1.0);
            if hp_frac <= self.retreat_health_fraction {
                let retreat_pos = kite_destination(unit.position, target.position, 120.0);
                actions.emit(StrategyAction::Move {
                    entities: vec![unit.id],
                    destination: retreat_pos,
                    mode: OrderMode::Replace,
                });
            } else {
                let dist = unit.position.distance(target.position);
                if dist <= self.combat_range {
                    actions.emit(StrategyAction::Attack {
                        entities: vec![unit.id],
                        target: AttackTarget::Entity { entity: target.id },
                        mode: OrderMode::Replace,
                    });
                } else if let Some(&flank_pos) = flank_positions.get(i) {
                    actions.emit(StrategyAction::Move {
                        entities: vec![unit.id],
                        destination: flank_pos,
                        mode: OrderMode::Replace,
                    });
                }
            }
        }
    }
}

/// Closed-loop adaptive defensive controller that establishes a perimeter of
/// wall segments and positions defending forces behind the fortification.
#[derive(Clone, Debug, PartialEq)]
pub struct WallDefenseStrategy {
    pub wall_definition: String,
    pub anchor_from: Vec2,
    pub anchor_to: Vec2,
    pub segment_spacing: f32,
    pub combat_standoff: f32,
    pub combat_range: f32,
}

impl WallDefenseStrategy {
    pub fn new(wall_definition: impl Into<String>, anchor_from: Vec2, anchor_to: Vec2) -> Self {
        Self {
            wall_definition: wall_definition.into(),
            anchor_from,
            anchor_to,
            segment_spacing: 32.0,
            combat_standoff: 48.0,
            combat_range: 240.0,
        }
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.segment_spacing = spacing.max(8.0);
        self
    }

    pub fn with_combat_standoff(mut self, standoff: f32) -> Self {
        self.combat_standoff = standoff.max(0.0);
        self
    }

    pub fn with_combat_range(mut self, range: f32) -> Self {
        self.combat_range = range.max(10.0);
        self
    }
}

impl Strategy for WallDefenseStrategy {
    fn update(&mut self, observation: &StrategyObservation, actions: &mut ActionEmitter) {
        let wall_positions =
            wall_segment_positions(self.anchor_from, self.anchor_to, self.segment_spacing);

        let owned_entities = observation
            .entities
            .iter()
            .filter(|e| e.side == observation.side)
            .collect::<Vec<_>>();

        let enemies = observation
            .entities
            .iter()
            .filter(|e| e.side != observation.side)
            .collect::<Vec<_>>();

        // Builder detection
        let builder = owned_entities.iter().find(|e| {
            e.kind == StrategyEntityKind::Mobile
                && e.definition
                    .as_deref()
                    .is_some_and(|d| d.contains("con") || d.contains("builder") || d.contains("ck"))
        });

        // Find unbuilt wall segments
        let existing_walls = owned_entities
            .iter()
            .filter(|e| {
                e.kind == StrategyEntityKind::Wall || e.kind == StrategyEntityKind::Nanoframe
            })
            .collect::<Vec<_>>();

        let missing_segment = wall_positions.iter().find(|pos| {
            !existing_walls
                .iter()
                .any(|w| w.position.distance_squared(**pos) <= (self.segment_spacing * 0.6).powi(2))
        });

        if let (Some(builder), Some(missing_pos)) = (builder, missing_segment) {
            let builder_threatened = enemies
                .iter()
                .any(|e| e.position.distance_squared(builder.position) < 180.0_f32.powi(2));
            if !builder_threatened {
                actions.emit(StrategyAction::Build {
                    builder: builder.id,
                    definition: self.wall_definition.clone(),
                    position: *missing_pos,
                    facing: BuildFacing::South,
                    mode: OrderMode::Replace,
                });
            } else if let Some(nearest_threat) = enemies.iter().min_by(|a, b| {
                builder
                    .position
                    .distance_squared(a.position)
                    .total_cmp(&builder.position.distance_squared(b.position))
                    .then_with(|| a.id.cmp(&b.id))
            }) {
                let retreat_pos =
                    kite_destination(builder.position, nearest_threat.position, 100.0);
                actions.emit(StrategyAction::Move {
                    entities: vec![builder.id],
                    destination: retreat_pos,
                    mode: OrderMode::Replace,
                });
            }
        }

        // Combat mobiles (non-builders)
        let combat_units = owned_entities
            .iter()
            .filter(|e| {
                e.kind == StrategyEntityKind::Mobile && builder.is_none_or(|b| b.id != e.id)
            })
            .collect::<Vec<_>>();

        if combat_units.is_empty() {
            return;
        }

        let wall_mid = (self.anchor_from + self.anchor_to) * 0.5;
        let delta = self.anchor_to - self.anchor_from;
        let wall_dir = if delta.length_squared() > f32::EPSILON {
            delta.normalized()
        } else {
            Vec2::new(1.0, 0.0)
        };
        let normal1 = Vec2::new(wall_dir.z, -wall_dir.x);
        let normal2 = Vec2::new(-wall_dir.z, wall_dir.x);

        let enemy_center = if !enemies.is_empty() {
            enemies
                .iter()
                .fold(Vec2::default(), |acc, e| acc + e.position)
                * (1.0 / enemies.len() as f32)
        } else {
            Vec2::new(observation.map_size.x * 0.5, observation.map_size.z * 0.5)
        };
        let defense_normal = if (wall_mid + normal1 * 50.0).distance_squared(enemy_center)
            > (wall_mid + normal2 * 50.0).distance_squared(enemy_center)
        {
            normal1
        } else {
            normal2
        };

        let defense_anchor = wall_mid + defense_normal * self.combat_standoff;

        if let Some(target) = enemies.iter().min_by(|a, b| {
            wall_mid
                .distance_squared(a.position)
                .total_cmp(&wall_mid.distance_squared(b.position))
                .then_with(|| a.id.cmp(&b.id))
        }) {
            for unit in &combat_units {
                let dist = unit.position.distance(target.position);
                if dist <= self.combat_range {
                    actions.emit(StrategyAction::Attack {
                        entities: vec![unit.id],
                        target: AttackTarget::Entity { entity: target.id },
                        mode: OrderMode::Replace,
                    });
                } else {
                    actions.emit(StrategyAction::Move {
                        entities: vec![unit.id],
                        destination: defense_anchor,
                        mode: OrderMode::Replace,
                    });
                }
            }
        } else {
            let combat_ids = combat_units.iter().map(|u| u.id).collect();
            actions.emit(StrategyAction::Move {
                entities: combat_ids,
                destination: defense_anchor,
                mode: OrderMode::Replace,
            });
        }
    }
}

/// Closed-loop adaptive economic expansion controller that directs builders to
/// claim discrete resource sites and construct necessary power generation.
#[derive(Clone, Debug, PartialEq)]
pub struct ExpansionStrategy {
    pub mex_definition: String,
    pub energy_definition: String,
    pub threat_radius: f32,
    pub energy_reserve_threshold: f64,
}

impl ExpansionStrategy {
    pub fn new(mex_definition: impl Into<String>, energy_definition: impl Into<String>) -> Self {
        Self {
            mex_definition: mex_definition.into(),
            energy_definition: energy_definition.into(),
            threat_radius: 200.0,
            energy_reserve_threshold: 100.0,
        }
    }

    pub fn with_threat_radius(mut self, radius: f32) -> Self {
        self.threat_radius = radius.max(0.0);
        self
    }

    pub fn with_energy_reserve_threshold(mut self, threshold: f64) -> Self {
        self.energy_reserve_threshold = threshold.max(0.0);
        self
    }
}

impl Strategy for ExpansionStrategy {
    fn update(&mut self, observation: &StrategyObservation, actions: &mut ActionEmitter) {
        let owned_entities = observation
            .entities
            .iter()
            .filter(|e| e.side == observation.side)
            .collect::<Vec<_>>();

        let enemies = observation
            .entities
            .iter()
            .filter(|e| e.side != observation.side)
            .collect::<Vec<_>>();

        let builder = owned_entities
            .iter()
            .find(|e| {
                e.kind == StrategyEntityKind::Mobile
                    && e.definition.as_deref().is_some_and(|d| {
                        d.contains("con") || d.contains("builder") || d.contains("ck")
                    })
            })
            .or_else(|| {
                owned_entities
                    .iter()
                    .find(|e| e.kind == StrategyEntityKind::Mobile)
            });

        let Some(builder) = builder else {
            return;
        };

        let threat = enemies
            .iter()
            .filter(|e| builder.position.distance_squared(e.position) <= self.threat_radius.powi(2))
            .min_by(|a, b| {
                builder
                    .position
                    .distance_squared(a.position)
                    .total_cmp(&builder.position.distance_squared(b.position))
                    .then_with(|| a.id.cmp(&b.id))
            });

        if let Some(enemy) = threat {
            let safe_dest = kite_destination(builder.position, enemy.position, 120.0);
            actions.emit(StrategyAction::Move {
                entities: vec![builder.id],
                destination: safe_dest,
                mode: OrderMode::Replace,
            });
            return;
        }

        let needs_energy = observation.energy.stored < self.energy_reserve_threshold
            || observation.energy.income_per_second < observation.energy.expense_per_second + 2.0;

        if needs_energy {
            let power_pos = builder.position + Vec2::new(32.0, 0.0);
            actions.emit(StrategyAction::Build {
                builder: builder.id,
                definition: self.energy_definition.clone(),
                position: power_pos,
                facing: BuildFacing::South,
                mode: OrderMode::Replace,
            });
            return;
        }

        if let Some(site) = nearest_open_resource_site(observation, builder.position, false) {
            actions.emit(StrategyAction::Build {
                builder: builder.id,
                definition: self.mex_definition.clone(),
                position: site.position,
                facing: BuildFacing::South,
                mode: OrderMode::Replace,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(slot: u32) -> EntityId {
        EntityId::new(slot, 1)
    }

    fn observation(tick: u64) -> StrategyObservation {
        StrategyObservation {
            tick,
            side: 0,
            map_size: Vec2::new(1_000.0, 1_000.0),
            metal: ResourceObservation::default(),
            energy: ResourceObservation::default(),
            entities: Vec::new(),
            fire_intents: Vec::new(),
            resource_sites: Vec::new(),
        }
    }

    #[derive(Default)]
    struct CountingStrategy(u32);

    impl Strategy for CountingStrategy {
        fn update(&mut self, _observation: &StrategyObservation, actions: &mut ActionEmitter) {
            self.0 += 1;
            actions.emit(StrategyAction::Cancel {
                target: CancelTarget::Orders { entity: id(1) },
            });
        }
    }

    #[test]
    fn cadence_is_aligned_and_sequences_are_monotonic() {
        let cadence = StrategyCadence::new(3, 2).unwrap();
        let mut controller = StrategyController::new(0, cadence, CountingStrategy::default());
        assert!(controller.poll(&observation(0)).is_empty());
        let first = controller.poll(&observation(2));
        assert_eq!(first[0].sequence, 1);
        assert!(controller.poll(&observation(2)).is_empty());
        assert!(controller.poll(&observation(4)).is_empty());
        let second = controller.poll(&observation(8));
        assert_eq!(second[0].sequence, 2);
        assert_eq!(controller.strategy().0, 2);
        assert!(controller.poll(&observation(10)).is_empty());
        assert_eq!(controller.poll(&observation(11))[0].sequence, 3);
    }

    #[test]
    fn invalid_cadences_are_explicit() {
        assert_eq!(
            StrategyCadence::new(0, 0),
            Err(StrategyConfigError::ZeroCadence)
        );
        assert_eq!(
            StrategyCadence::new(4, 4),
            Err(StrategyConfigError::PhaseOutsideCadence)
        );
        assert!(
            serde_json::from_str::<StrategyCadence>(r#"{"interval_ticks":0,"phase_tick":0}"#)
                .is_err()
        );
    }

    fn cancel(side: SideId, sequence: u64, tick: u64) -> QueuedAction {
        QueuedAction {
            side,
            sequence,
            apply_at_tick: tick,
            action: StrategyAction::Cancel {
                target: CancelTarget::Orders { entity: id(1) },
            },
        }
    }

    #[test]
    fn inbox_orders_ready_actions_by_side_then_sequence() {
        let mut inbox = ActionInbox::default();
        inbox.extend([cancel(1, 9, 0), cancel(0, 4, 0), cancel(0, 2, 0)]);
        inbox.push(cancel(0, 1, 8));
        let ready = inbox.drain_ready(0);
        assert_eq!(
            ready
                .iter()
                .map(|action| (action.side, action.sequence))
                .collect::<Vec<_>>(),
            vec![(0, 2), (0, 4), (1, 9)]
        );
        assert_eq!(inbox.len(), 1);
    }

    #[derive(Default)]
    struct TestWorld {
        entities: BTreeMap<EntityId, ActionEntity>,
        queue_len: usize,
        reject_placement: bool,
    }

    impl ActionWorld for TestWorld {
        fn is_valid_side(&self, side: SideId) -> bool {
            side < 2
        }

        fn entity(&self, entity: EntityId) -> Option<ActionEntity> {
            self.entities.get(&entity).copied()
        }

        fn is_position_valid(&self, position: Vec2) -> bool {
            position.x >= 0.0 && position.z >= 0.0 && position.x <= 1_000.0 && position.z <= 1_000.0
        }

        fn is_observed(&self, _side: SideId, _entity: EntityId) -> bool {
            true
        }

        fn definition_exists(&self, definition: &str) -> bool {
            matches!(definition, "pawn" | "wall")
        }

        fn builder_can_build(&self, _builder: EntityId, definition: &str) -> bool {
            definition == "wall"
        }

        fn factory_can_build(&self, _factory: EntityId, definition: &str) -> bool {
            definition == "pawn"
        }

        fn factory_queue_len(&self, _factory: EntityId) -> usize {
            self.queue_len
        }

        fn placement_rejection(
            &self,
            _side: SideId,
            _builder: EntityId,
            _definition: &str,
            _position: Vec2,
            _facing: BuildFacing,
        ) -> Option<ActionRejection> {
            self.reject_placement
                .then_some(ActionRejection::PlacementRejected)
        }
    }

    fn capable(side: SideId) -> ActionEntity {
        ActionEntity {
            side,
            kind: StrategyEntityKind::Mobile,
            alive: true,
            capabilities: ActionCapabilities {
                move_order: true,
                attack: true,
                build: true,
                assist: true,
                repair: true,
                factory_queue: true,
                activation: true,
                cloak: true,
                reclaim: true,
                cancel: true,
            },
        }
    }

    fn world() -> TestWorld {
        let mut world = TestWorld {
            queue_len: 2,
            ..Default::default()
        };
        world.entities.insert(id(1), capable(0));
        world.entities.insert(id(2), capable(0));
        world.entities.insert(id(3), capable(1));
        world
    }

    #[test]
    fn every_action_family_has_an_engine_independent_valid_form() {
        let world = world();
        let actions = [
            StrategyAction::Move {
                entities: vec![id(1)],
                destination: Vec2::new(20.0, 20.0),
                mode: OrderMode::Replace,
            },
            StrategyAction::Attack {
                entities: vec![id(1)],
                target: AttackTarget::Entity { entity: id(3) },
                mode: OrderMode::Replace,
            },
            StrategyAction::Build {
                builder: id(1),
                definition: "wall".into(),
                position: Vec2::new(30.0, 30.0),
                facing: BuildFacing::South,
                mode: OrderMode::Append,
            },
            StrategyAction::Assist {
                entities: vec![id(1)],
                target: id(2),
                mode: OrderMode::Replace,
            },
            StrategyAction::Repair {
                entities: vec![id(1)],
                target: id(2),
                mode: OrderMode::Replace,
            },
            StrategyAction::FactoryQueue {
                factory: id(1),
                operation: FactoryQueueOperation::Append {
                    item: FactoryQueueItem {
                        definition: "pawn".into(),
                        count: 2,
                    },
                },
            },
            StrategyAction::SetActivation {
                entities: vec![id(1)],
                active: false,
            },
            StrategyAction::Reclaim {
                entities: vec![id(1)],
                target: ReclaimTarget::Area {
                    position: Vec2::new(40.0, 40.0),
                    radius: 50.0,
                },
                mode: OrderMode::Replace,
            },
            StrategyAction::Cancel {
                target: CancelTarget::Orders { entity: id(1) },
            },
        ];
        for action in actions {
            assert_eq!(validate_action(0, &action, &world), Ok(()), "{action:?}");
        }
    }

    #[test]
    fn validation_precedence_is_independent_of_selection_order() {
        let world = world();
        let duplicate_and_unknown = StrategyAction::Move {
            entities: vec![id(99), id(1), id(1)],
            destination: Vec2::new(f32::NAN, 0.0),
            mode: OrderMode::Replace,
        };
        assert_eq!(
            validate_action(0, &duplicate_and_unknown, &world),
            Err(ActionRejection::DuplicateEntity)
        );

        let unknown_and_bad_position = StrategyAction::Move {
            entities: vec![id(99)],
            destination: Vec2::new(f32::NAN, 0.0),
            mode: OrderMode::Replace,
        };
        assert_eq!(
            validate_action(0, &unknown_and_bad_position, &world),
            Err(ActionRejection::UnknownEntity)
        );
    }

    #[test]
    fn rejected_actions_still_consume_their_sequence() {
        let world = world();
        let invalid = QueuedAction {
            side: 0,
            sequence: 7,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: Vec::new(),
                destination: Vec2::new(0.0, 0.0),
                mode: OrderMode::Replace,
            },
        };
        let mut validator = ActionValidator::default();
        assert_eq!(
            validator.validate_envelope(&invalid, &world),
            Err(ActionRejection::EmptySelection)
        );
        assert_eq!(
            validator.validate_envelope(&invalid, &world),
            Err(ActionRejection::SequenceAlreadySeen)
        );
        assert_eq!(validator.last_sequence(0), Some(7));
    }

    #[test]
    fn factory_validation_has_stable_reason_codes() {
        let world = world();
        let zero_count = StrategyAction::FactoryQueue {
            factory: id(1),
            operation: FactoryQueueOperation::Append {
                item: FactoryQueueItem {
                    definition: "pawn".into(),
                    count: 0,
                },
            },
        };
        assert_eq!(
            validate_action(0, &zero_count, &world),
            Err(ActionRejection::InvalidQuantity)
        );
        let out_of_range = StrategyAction::FactoryQueue {
            factory: id(1),
            operation: FactoryQueueOperation::Cancel { index: 2 },
        };
        assert_eq!(
            validate_action(0, &out_of_range, &world),
            Err(ActionRejection::QueueIndexOutOfRange)
        );
        assert_eq!(ActionRejection::QueueIndexOutOfRange.code(), 16);
    }

    #[test]
    fn formation_and_reference_geometry_are_deterministic() {
        let formation = FormationSpec {
            side: 0,
            anchor: Vec2::new(100.0, 100.0),
            heading: std::f32::consts::FRAC_PI_2,
            members: vec![FormationMember {
                label: "wing".into(),
                definition: "pawn".into(),
                count: 3,
            }],
            pattern: FormationPattern::Line { spacing: 10.0 },
        };
        let positions = formation.materialize();
        assert_eq!(positions.len(), 3);
        assert!((positions[0].2.z - 90.0).abs() < 1e-4);
        assert!((positions[1].2.z - 100.0).abs() < 1e-4);
        assert!((positions[2].2.z - 110.0).abs() < 1e-4);

        assert_eq!(
            kite_destination(Vec2::new(5.0, 0.0), Vec2::new(0.0, 0.0), 5.0),
            Vec2::new(10.0, 0.0)
        );
        let envelope = envelopment_positions(Vec2::new(0.0, 0.0), 10.0, 4);
        assert_eq!(envelope.len(), 4);
        assert!((envelope[2].x + 10.0).abs() < 1e-4);
        assert_eq!(
            wall_segment_positions(Vec2::new(0.0, 0.0), Vec2::new(25.0, 0.0), 10.0).len(),
            4
        );

        let mut observation = observation(0);
        observation.resource_sites = vec![
            ObservedResourceSite {
                id: 2,
                position: Vec2::new(5.0, 0.0),
                metal: 2.0,
                geothermal: false,
                occupied_by: None,
            },
            ObservedResourceSite {
                id: 1,
                position: Vec2::new(-5.0, 0.0),
                metal: 1.0,
                geothermal: false,
                occupied_by: None,
            },
        ];
        assert_eq!(
            nearest_open_resource_site(&observation, Vec2::default(), false).map(|site| site.id),
            Some(1)
        );
    }

    #[test]
    fn scripted_strategy_catches_up_without_reordering_equal_ticks() {
        let first = StrategyAction::Cancel {
            target: CancelTarget::Orders { entity: id(1) },
        };
        let second = StrategyAction::Cancel {
            target: CancelTarget::Orders { entity: id(2) },
        };
        let script = ScriptedStrategy::new(vec![
            TimedAction {
                at_tick: 5,
                action: first.clone(),
            },
            TimedAction {
                at_tick: 5,
                action: second.clone(),
            },
        ]);
        let mut controller = StrategyController::new(0, StrategyCadence::EVERY_TICK, script);
        let emitted = controller.poll(&observation(7));
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].action, first);
        assert_eq!(emitted[1].action, second);
        assert!(controller.strategy().is_complete());
    }

    #[test]
    fn declarative_scenarios_round_trip_as_json() {
        let scenario = DeclarativeScenario {
            name: "wall hold".into(),
            kind: ScenarioKind::WallDefense,
            map: "flat-1k".into(),
            max_ticks: 9_000,
            seed: 4,
            observation: ObservationRules::default(),
            sides: vec![ScenarioSide {
                side: 0,
                team: 0,
                start_position: Vec2::new(100.0, 100.0),
                starting_metal: 1_000.0,
                starting_energy: 1_000.0,
            }],
            formations: Vec::new(),
            openings: Vec::new(),
        };
        let json = serde_json::to_string(&scenario).unwrap();
        assert_eq!(
            serde_json::from_str::<DeclarativeScenario>(&json).unwrap(),
            scenario
        );
    }

    #[test]
    fn envelopment_strategy_attacks_in_range_flanks_out_of_range_and_kites_on_low_hp() {
        let mut strategy = EnvelopmentStrategy::new(100.0, 150.0)
            .with_retreat_health_fraction(0.3)
            .with_default_destination(Vec2::new(500.0, 500.0));

        let enemy_id = id(10);
        let mut obs = observation(1);
        obs.entities = vec![
            // Healthy unit in combat range
            ObservedEntity {
                id: id(1),
                side: 0,
                definition: Some("armpw".into()),
                kind: StrategyEntityKind::Mobile,
                position: Vec2::new(100.0, 100.0),
                health_fraction: Some(1.0),
                visibility: ObservationVisibility::Own,
                age_seconds: 0.0,
                position_error: 0.0,
            },
            // Healthy unit outside combat range
            ObservedEntity {
                id: id(2),
                side: 0,
                definition: Some("armpw".into()),
                kind: StrategyEntityKind::Mobile,
                position: Vec2::new(300.0, 300.0),
                health_fraction: Some(1.0),
                visibility: ObservationVisibility::Own,
                age_seconds: 0.0,
                position_error: 0.0,
            },
            // Low health unit (should kite)
            ObservedEntity {
                id: id(3),
                side: 0,
                definition: Some("armpw".into()),
                kind: StrategyEntityKind::Mobile,
                position: Vec2::new(120.0, 100.0),
                health_fraction: Some(0.2),
                visibility: ObservationVisibility::Own,
                age_seconds: 0.0,
                position_error: 0.0,
            },
            // Enemy target
            ObservedEntity {
                id: enemy_id,
                side: 1,
                definition: Some("corak".into()),
                kind: StrategyEntityKind::Mobile,
                position: Vec2::new(100.0, 180.0),
                health_fraction: Some(1.0),
                visibility: ObservationVisibility::LineOfSight,
                age_seconds: 0.0,
                position_error: 0.0,
            },
        ];

        let mut emitter = ActionEmitter::new(0, 1, 1);
        strategy.update(&obs, &mut emitter);

        assert_eq!(emitter.emitted.len(), 3);
        // Unit 1 attacks (dist is 80 <= 150)
        assert!(matches!(
            &emitter.emitted[0].action,
            StrategyAction::Attack { entities, target: AttackTarget::Entity { entity }, .. }
                if entities == &[id(1)] && *entity == enemy_id
        ));
        // Unit 2 moves to flank position
        assert!(matches!(
            &emitter.emitted[1].action,
            StrategyAction::Move { entities, .. } if entities == &[id(2)]
        ));
        // Unit 3 kites away
        assert!(matches!(
            &emitter.emitted[2].action,
            StrategyAction::Move { entities, .. } if entities == &[id(3)]
        ));

        // When enemies are eliminated, units navigate towards default_destination
        obs.entities.retain(|e| e.side == 0);
        let mut idle_emitter = ActionEmitter::new(0, 2, 10);
        strategy.update(&obs, &mut idle_emitter);
        assert_eq!(idle_emitter.emitted.len(), 1);
        assert!(matches!(
            &idle_emitter.emitted[0].action,
            StrategyAction::Move { destination, .. } if *destination == Vec2::new(500.0, 500.0)
        ));
    }

    #[test]
    fn wall_defense_strategy_builds_missing_walls_and_maneuvers_combat_behind_wall() {
        let from = Vec2::new(100.0, 100.0);
        let to = Vec2::new(100.0, 164.0);
        let mut strategy = WallDefenseStrategy::new("armdrag", from, to)
            .with_spacing(32.0)
            .with_combat_standoff(30.0)
            .with_combat_range(120.0);

        let mut obs = observation(1);
        let builder_id = id(1);
        let combat_id = id(2);
        obs.entities = vec![
            ObservedEntity {
                id: builder_id,
                side: 0,
                definition: Some("armck".into()),
                kind: StrategyEntityKind::Mobile,
                position: Vec2::new(80.0, 100.0),
                health_fraction: Some(1.0),
                visibility: ObservationVisibility::Own,
                age_seconds: 0.0,
                position_error: 0.0,
            },
            ObservedEntity {
                id: combat_id,
                side: 0,
                definition: Some("armpw".into()),
                kind: StrategyEntityKind::Mobile,
                position: Vec2::new(60.0, 130.0),
                health_fraction: Some(1.0),
                visibility: ObservationVisibility::Own,
                age_seconds: 0.0,
                position_error: 0.0,
            },
        ];

        let mut emitter = ActionEmitter::new(0, 1, 1);
        strategy.update(&obs, &mut emitter);

        // Builder should issue build order for the first missing wall segment
        assert!(emitter.emitted.iter().any(|a| matches!(
            &a.action,
            StrategyAction::Build { builder, definition, .. }
                if *builder == builder_id && definition == "armdrag"
        )));

        // Combat unit should move to defense anchor behind the wall
        assert!(emitter.emitted.iter().any(|a| matches!(
            &a.action,
            StrategyAction::Move { entities, .. } if entities == &[combat_id]
        )));
    }

    #[test]
    fn expansion_strategy_prioritizes_open_mex_and_builds_energy_when_depleted() {
        let mut strategy = ExpansionStrategy::new("armmex", "armsolar")
            .with_threat_radius(100.0)
            .with_energy_reserve_threshold(200.0);

        let mut obs = observation(1);
        let builder_id = id(1);
        obs.entities = vec![ObservedEntity {
            id: builder_id,
            side: 0,
            definition: Some("armck".into()),
            kind: StrategyEntityKind::Mobile,
            position: Vec2::new(50.0, 50.0),
            health_fraction: Some(1.0),
            visibility: ObservationVisibility::Own,
            age_seconds: 0.0,
            position_error: 0.0,
        }];
        obs.resource_sites = vec![ObservedResourceSite {
            id: 1,
            position: Vec2::new(100.0, 50.0),
            metal: 2.0,
            geothermal: false,
            occupied_by: None,
        }];
        // Plenty of energy -> builds mex
        obs.energy.stored = 500.0;
        obs.energy.income_per_second = 20.0;
        obs.energy.expense_per_second = 5.0;

        let mut emitter = ActionEmitter::new(0, 1, 1);
        strategy.update(&obs, &mut emitter);
        assert_eq!(emitter.emitted.len(), 1);
        assert!(matches!(
            &emitter.emitted[0].action,
            StrategyAction::Build { builder, definition, position, .. }
                if *builder == builder_id && definition == "armmex" && *position == Vec2::new(100.0, 50.0)
        ));

        // Energy is depleted -> builds solar
        obs.energy.stored = 50.0;
        obs.energy.income_per_second = 2.0;
        obs.energy.expense_per_second = 10.0;
        let mut solar_emitter = ActionEmitter::new(0, 2, 2);
        strategy.update(&obs, &mut solar_emitter);
        assert_eq!(solar_emitter.emitted.len(), 1);
        assert!(matches!(
            &solar_emitter.emitted[0].action,
            StrategyAction::Build { builder, definition, .. }
                if *builder == builder_id && definition == "armsolar"
        ));

        // Threat approaches builder -> retreats
        obs.entities.push(ObservedEntity {
            id: id(20),
            side: 1,
            definition: Some("corak".into()),
            kind: StrategyEntityKind::Mobile,
            position: Vec2::new(80.0, 50.0), // 30 elmos away <= 100
            health_fraction: Some(1.0),
            visibility: ObservationVisibility::LineOfSight,
            age_seconds: 0.0,
            position_error: 0.0,
        });
        let mut threat_emitter = ActionEmitter::new(0, 3, 3);
        strategy.update(&obs, &mut threat_emitter);
        assert_eq!(threat_emitter.emitted.len(), 1);
        assert!(matches!(
            &threat_emitter.emitted[0].action,
            StrategyAction::Move { entities, .. } if entities == &[builder_id]
        ));
    }
}
