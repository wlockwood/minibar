//! Compilation of declarative strategic scenarios into runnable worlds.
//!
//! Selectors are deliberately resolved once, against entities spawned by the
//! scenario's initial formations. Units constructed by an opening cannot be
//! named by a later static selector; strategies that need those units must
//! discover them from observations or other runtime state.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::economy::Resources;
use crate::entity::EntityId;
use crate::map::CompiledMap;
use crate::model::{UnitDef, Vec2};
use crate::placement::BuildFacing;
use crate::strategy::{
    AttackTarget, CancelTarget, DeclarativeScenario, EntitySelector, FactoryQueueOperation,
    ObservationPolicy, OpeningInstruction, OrderMode, ScenarioKind, ScenarioSide, ScriptedStrategy,
    SideId, StrategyCadence, StrategyController, TimedAction,
};
use crate::world::{World, WorldCatalog, WorldError};

/// Runtime and sensor settings applied while compiling a declarative scenario.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenarioCompileOptions {
    pub world: crate::world::WorldConfig,
    pub cadence: StrategyCadence,
    pub line_of_sight_range: f32,
    pub radar_range: f32,
    pub radar_position_resolution: f32,
    /// Bounds untrusted declarative input before materializing formations.
    pub max_initial_entities: usize,
}

impl Default for ScenarioCompileOptions {
    fn default() -> Self {
        Self {
            world: crate::world::WorldConfig::default(),
            cadence: StrategyCadence::EVERY_TICK,
            line_of_sight_range: 500.0,
            radar_range: 1_200.0,
            radar_position_resolution: 32.0,
            max_initial_entities: 1_000_000,
        }
    }
}

impl From<crate::world::WorldConfig> for ScenarioCompileOptions {
    fn from(world: crate::world::WorldConfig) -> Self {
        Self {
            world,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorUse {
    Actor,
    AttackTarget,
    AssistTarget,
}

/// Stable entity lookup built from initial formation declarations.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioEntityIndex {
    sides: BTreeMap<SideId, SideEntityIndex>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEntityIndex {
    pub all: Vec<EntityId>,
    pub labels: BTreeMap<String, Vec<EntityId>>,
    pub definitions: BTreeMap<String, Vec<EntityId>>,
    pub builders: Vec<EntityId>,
    pub factories: Vec<EntityId>,
    pub combat_units: Vec<EntityId>,
}

impl ScenarioEntityIndex {
    pub fn side(&self, side: SideId) -> Option<&SideEntityIndex> {
        self.sides.get(&side)
    }

    /// Resolves an actor selector within one side. An empty result means the
    /// selector did not match any initially spawned entity.
    pub fn resolve_for_side(&self, side: SideId, selector: &EntitySelector) -> Vec<EntityId> {
        self.sides
            .get(&side)
            .map_or_else(Vec::new, |index| index.resolve(selector).to_vec())
    }

    fn resolve_globally(&self, selector: &EntitySelector) -> Vec<EntityId> {
        let mut matches = Vec::new();
        for index in self.sides.values() {
            matches.extend_from_slice(index.resolve(selector));
        }
        matches
    }

    fn insert(&mut self, side: SideId, label: &str, definition: &UnitDef, entity: EntityId) {
        let index = self.sides.entry(side).or_default();
        index.all.push(entity);
        index
            .labels
            .entry(label.to_owned())
            .or_default()
            .push(entity);
        index
            .definitions
            .entry(definition.name.clone())
            .or_default()
            .push(entity);
        if definition.strategic.is_builder {
            index.builders.push(entity);
        }
        if definition.strategic.is_factory {
            index.factories.push(entity);
        }
        if !definition.weapons.is_empty() {
            index.combat_units.push(entity);
        }
    }
}

impl SideEntityIndex {
    pub fn resolve(&self, selector: &EntitySelector) -> &[EntityId] {
        match selector {
            EntitySelector::Label { label } => self.labels.get(label).map_or(&[], Vec::as_slice),
            EntitySelector::Definition { definition } => {
                self.definitions.get(definition).map_or(&[], Vec::as_slice)
            }
            EntitySelector::AllBuilders => &self.builders,
            EntitySelector::AllFactories => &self.factories,
            EntitySelector::AllCombatUnits => &self.combat_units,
        }
    }
}

#[derive(Debug)]
pub struct CompiledScenarioSide {
    pub specification: ScenarioSide,
    pub controller: StrategyController<ScriptedStrategy>,
}

/// A fully initialized deterministic world plus one scripted controller per
/// declared side. Controllers are ordered by side ID.
pub struct DeclarativeScenarioRuntime {
    pub name: String,
    pub kind: ScenarioKind,
    pub seed: u64,
    pub max_ticks: u64,
    pub world: World,
    pub entity_index: ScenarioEntityIndex,
    pub sides: Vec<CompiledScenarioSide>,
}

impl fmt::Debug for DeclarativeScenarioRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeclarativeScenarioRuntime")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("seed", &self.seed)
            .field("max_ticks", &self.max_ticks)
            .field("tick", &self.world.tick)
            .field("entity_index", &self.entity_index)
            .field("sides", &self.sides)
            .finish_non_exhaustive()
    }
}

impl DeclarativeScenarioRuntime {
    pub fn is_complete(&self) -> bool {
        self.world.tick >= self.max_ticks
    }

    /// Polls each side in stable side order, then advances one world tick.
    /// Returns `false` without advancing after `max_ticks`.
    pub fn step(&mut self) -> Result<bool, WorldError> {
        if self.is_complete() {
            return Ok(false);
        }
        for side in &mut self.sides {
            let observation = self.world.observation(side.specification.side)?;
            self.world
                .enqueue_actions(side.controller.poll(&observation));
        }
        self.world.step();
        Ok(true)
    }
}

impl crate::live::StepWorld for DeclarativeScenarioRuntime {
    fn tick(&self) -> u64 {
        self.world.tick
    }

    fn enqueue_action(&mut self, action: crate::strategy::QueuedAction) {
        self.world.enqueue_action(action);
    }

    fn step(&mut self) {
        // Side IDs and observation policy were validated at compilation, so a
        // live driver cannot introduce a stepping error here. Once max_ticks
        // is reached the authoritative tick simply remains fixed.
        Self::step(self).expect("compiled scenario remains valid while stepping");
    }

    fn state_hash(&self) -> u64 {
        self.world.state_hash()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScenarioCompileError {
    Json(String),
    EmptyName,
    ZeroMaxTicks,
    MapMismatch {
        requested: String,
        loaded: String,
    },
    NoSides,
    InvalidSide(SideId),
    DuplicateSide(SideId),
    UnsupportedTeamMapping {
        side: SideId,
        team: u8,
    },
    InvalidStartingResources {
        side: SideId,
    },
    InvalidStartPosition {
        side: SideId,
        position: Vec2,
    },
    InvalidSensorRange(&'static str),
    InvalidEntityLimit,
    TooManyInitialEntities {
        requested: usize,
        limit: usize,
    },
    UnknownFormationSide {
        formation: usize,
        side: SideId,
    },
    EmptyFormation {
        formation: usize,
    },
    InvalidFormationHeading {
        formation: usize,
    },
    InvalidFormationPattern {
        formation: usize,
        reason: &'static str,
    },
    EmptyMemberLabel {
        formation: usize,
        member: usize,
    },
    ZeroMemberCount {
        formation: usize,
        member: usize,
    },
    UnknownDefinition {
        definition: String,
    },
    InvalidFormationPosition {
        formation: usize,
        spawn: usize,
        position: Vec2,
    },
    UnknownMovementClass {
        formation: usize,
        spawn: usize,
        movement_class: String,
    },
    ImpassableFormationPosition {
        formation: usize,
        spawn: usize,
        movement_class: String,
        position: Vec2,
    },
    SpawnFailed {
        formation: usize,
        spawn: usize,
        source: WorldError,
    },
    EmptyOpeningName {
        opening: usize,
    },
    DuplicateOpeningName {
        side: SideId,
        name: String,
    },
    UnknownOpeningSide {
        opening: usize,
        side: SideId,
    },
    StepOutsideDuration {
        opening: usize,
        step: usize,
        at_tick: u64,
        max_ticks: u64,
    },
    UnknownInitialSelector {
        opening: usize,
        step: usize,
        side: Option<SideId>,
        usage: SelectorUse,
        selector: EntitySelector,
    },
    AmbiguousSelector {
        opening: usize,
        step: usize,
        usage: SelectorUse,
        selector: EntitySelector,
        matches: Vec<EntityId>,
    },
    CapabilityMissing {
        opening: usize,
        step: usize,
        entity: EntityId,
        capability: &'static str,
    },
    InvalidTargetSide {
        opening: usize,
        step: usize,
        actor_side: SideId,
        target_side: SideId,
        usage: SelectorUse,
    },
    InvalidOpeningPosition {
        opening: usize,
        step: usize,
        position: Vec2,
    },
    ImpassableOpeningPosition {
        opening: usize,
        step: usize,
        entity: EntityId,
        movement_class: String,
        position: Vec2,
    },
    InvalidReclaimRadius {
        opening: usize,
        step: usize,
    },
    InvalidQueueCount {
        opening: usize,
        step: usize,
        definition: String,
    },
    BuildOptionUnavailable {
        opening: usize,
        step: usize,
        actor: EntityId,
        definition: String,
    },
    World(WorldError),
}

impl fmt::Display for ScenarioCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => {
                write!(formatter, "invalid declarative scenario JSON: {message}")
            }
            Self::EmptyName => formatter.write_str("scenario name must not be empty"),
            Self::ZeroMaxTicks => formatter.write_str("scenario max_ticks must be non-zero"),
            Self::MapMismatch { requested, loaded } => write!(
                formatter,
                "scenario requests map {requested:?}, but loaded map is {loaded:?}"
            ),
            Self::NoSides => formatter.write_str("scenario must declare at least one side"),
            Self::InvalidSide(side) => write!(formatter, "scenario side {side} is unsupported"),
            Self::DuplicateSide(side) => write!(formatter, "scenario side {side} is duplicated"),
            Self::UnsupportedTeamMapping { side, team } => write!(
                formatter,
                "side {side} maps to team {team}; the current two-side world requires team == side"
            ),
            Self::InvalidStartingResources { side } => {
                write!(formatter, "side {side} has invalid starting resources")
            }
            Self::InvalidStartPosition { side, position } => write!(
                formatter,
                "side {side} start position ({}, {}) is outside the map",
                position.x, position.z
            ),
            Self::InvalidSensorRange(name) => write!(formatter, "invalid {name} sensor range"),
            Self::InvalidEntityLimit => {
                formatter.write_str("max_initial_entities must be non-zero")
            }
            Self::TooManyInitialEntities { requested, limit } => write!(
                formatter,
                "scenario requests {requested} initial entities, exceeding limit {limit}"
            ),
            Self::UnknownFormationSide { formation, side } => write!(
                formatter,
                "formation {formation} refers to undeclared side {side}"
            ),
            Self::EmptyFormation { formation } => {
                write!(formatter, "formation {formation} has no members")
            }
            Self::InvalidFormationHeading { formation } => {
                write!(formatter, "formation {formation} has a non-finite heading")
            }
            Self::InvalidFormationPattern { formation, reason } => {
                write!(
                    formatter,
                    "formation {formation} has invalid pattern: {reason}"
                )
            }
            Self::EmptyMemberLabel { formation, member } => write!(
                formatter,
                "formation {formation} member {member} has an empty label"
            ),
            Self::ZeroMemberCount { formation, member } => write!(
                formatter,
                "formation {formation} member {member} has zero count"
            ),
            Self::UnknownDefinition { definition } => {
                write!(formatter, "unknown unit definition {definition:?}")
            }
            Self::InvalidFormationPosition {
                formation,
                spawn,
                position,
            } => write!(
                formatter,
                "formation {formation} spawn {spawn} position ({}, {}) is outside the map",
                position.x, position.z
            ),
            Self::UnknownMovementClass {
                formation,
                spawn,
                movement_class,
            } => write!(
                formatter,
                "formation {formation} spawn {spawn} uses unknown movement class {movement_class:?}"
            ),
            Self::ImpassableFormationPosition {
                formation,
                spawn,
                movement_class,
                position,
            } => write!(
                formatter,
                "formation {formation} spawn {spawn} cannot use {movement_class:?} at ({}, {})",
                position.x, position.z
            ),
            Self::SpawnFailed {
                formation,
                spawn,
                source,
            } => write!(
                formatter,
                "formation {formation} spawn {spawn} could not be created: {source}"
            ),
            Self::EmptyOpeningName { opening } => {
                write!(formatter, "opening {opening} has an empty name")
            }
            Self::DuplicateOpeningName { side, name } => {
                write!(formatter, "side {side} has duplicate opening name {name:?}")
            }
            Self::UnknownOpeningSide { opening, side } => {
                write!(
                    formatter,
                    "opening {opening} refers to undeclared side {side}"
                )
            }
            Self::StepOutsideDuration {
                opening,
                step,
                at_tick,
                max_ticks,
            } => write!(
                formatter,
                "opening {opening} step {step} runs at tick {at_tick}, outside max_ticks {max_ticks}"
            ),
            Self::UnknownInitialSelector {
                opening,
                step,
                usage,
                selector,
                ..
            } => write!(
                formatter,
                "opening {opening} step {step} {usage:?} selector {selector:?} matches no initial formation entity; constructed-later entities require a runtime strategy"
            ),
            Self::AmbiguousSelector {
                opening,
                step,
                usage,
                selector,
                matches,
            } => write!(
                formatter,
                "opening {opening} step {step} {usage:?} selector {selector:?} requires one entity but matched {}",
                matches.len()
            ),
            Self::CapabilityMissing {
                opening,
                step,
                entity,
                capability,
            } => write!(
                formatter,
                "opening {opening} step {step} entity {entity:?} lacks {capability}"
            ),
            Self::InvalidTargetSide {
                opening,
                step,
                actor_side,
                target_side,
                usage,
            } => write!(
                formatter,
                "opening {opening} step {step} {usage:?} target side {target_side} is invalid for actor side {actor_side}"
            ),
            Self::InvalidOpeningPosition {
                opening,
                step,
                position,
            } => write!(
                formatter,
                "opening {opening} step {step} position ({}, {}) is outside the map",
                position.x, position.z
            ),
            Self::ImpassableOpeningPosition {
                opening,
                step,
                entity,
                movement_class,
                position,
            } => write!(
                formatter,
                "opening {opening} step {step} entity {entity:?} cannot move with {movement_class:?} to ({}, {})",
                position.x, position.z
            ),
            Self::InvalidReclaimRadius { opening, step } => write!(
                formatter,
                "opening {opening} step {step} reclaim radius must be finite and positive"
            ),
            Self::InvalidQueueCount {
                opening,
                step,
                definition,
            } => write!(
                formatter,
                "opening {opening} step {step} queue count for {definition:?} must be non-zero"
            ),
            Self::BuildOptionUnavailable {
                opening,
                step,
                actor,
                definition,
            } => write!(
                formatter,
                "opening {opening} step {step} entity {actor:?} cannot build {definition:?}"
            ),
            Self::World(source) => {
                write!(formatter, "could not initialize strategic world: {source}")
            }
        }
    }
}

impl std::error::Error for ScenarioCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SpawnFailed { source, .. } | Self::World(source) => Some(source),
            _ => None,
        }
    }
}

/// Parses and compiles one scenario JSON document.
pub fn compile_declarative_scenario_json(
    json: &str,
    map: Arc<CompiledMap>,
    catalog: Arc<WorldCatalog>,
    options: ScenarioCompileOptions,
) -> Result<DeclarativeScenarioRuntime, ScenarioCompileError> {
    let scenario = serde_json::from_str(json)
        .map_err(|error| ScenarioCompileError::Json(error.to_string()))?;
    compile_declarative_scenario(&scenario, map, catalog, options)
}

/// Validates and materializes an initial strategic world and its opening plans.
pub fn compile_declarative_scenario(
    scenario: &DeclarativeScenario,
    map: Arc<CompiledMap>,
    catalog: Arc<WorldCatalog>,
    options: ScenarioCompileOptions,
) -> Result<DeclarativeScenarioRuntime, ScenarioCompileError> {
    validate_header(scenario, &map, options)?;

    let mut side_specs = BTreeMap::new();
    for side in &scenario.sides {
        validate_side(side, &map)?;
        if side_specs.insert(side.side, side.clone()).is_some() {
            return Err(ScenarioCompileError::DuplicateSide(side.side));
        }
    }

    let mut requested_entities = 0_usize;
    for (formation_index, formation) in scenario.formations.iter().enumerate() {
        if !side_specs.contains_key(&formation.side) {
            return Err(ScenarioCompileError::UnknownFormationSide {
                formation: formation_index,
                side: formation.side,
            });
        }
        validate_formation(formation_index, formation, &catalog)?;
        for member in &formation.members {
            requested_entities = requested_entities.saturating_add(member.count as usize);
            if requested_entities > options.max_initial_entities {
                return Err(ScenarioCompileError::TooManyInitialEntities {
                    requested: requested_entities,
                    limit: options.max_initial_entities,
                });
            }
        }
    }

    let policy = ObservationPolicy::from_rules(
        scenario.observation,
        options.line_of_sight_range,
        options.radar_range,
        options.radar_position_resolution,
    );
    let mut world_config = options.world;
    world_config.observation_policy = policy;
    let mut world =
        World::new_with_static_features(Arc::clone(&map), Arc::clone(&catalog), world_config)
            .map_err(ScenarioCompileError::World)?;
    for side in side_specs.values() {
        world
            .set_team_starting_resources(
                side.side,
                Resources::new(side.starting_metal, side.starting_energy),
            )
            .map_err(ScenarioCompileError::World)?;
    }

    let mut entity_index = ScenarioEntityIndex::default();
    for (formation_index, formation) in scenario.formations.iter().enumerate() {
        let facing = facing_from_heading(formation.heading);
        for (spawn_index, (label, definition_name, position)) in
            formation.materialize().into_iter().enumerate()
        {
            if !map.contains(position.x, position.z) {
                return Err(ScenarioCompileError::InvalidFormationPosition {
                    formation: formation_index,
                    spawn: spawn_index,
                    position,
                });
            }
            let (definition_id, definition) =
                catalog.get_by_name(&definition_name).ok_or_else(|| {
                    ScenarioCompileError::UnknownDefinition {
                        definition: definition_name.clone(),
                    }
                })?;
            validate_spawn_passability(formation_index, spawn_index, position, definition, &map)?;
            let entity = world
                .spawn_active(formation.side, definition_id, position, facing)
                .map_err(|source| ScenarioCompileError::SpawnFailed {
                    formation: formation_index,
                    spawn: spawn_index,
                    source,
                })?;
            entity_index.insert(formation.side, &label, definition, entity);
        }
    }

    let mut actions_by_side: BTreeMap<SideId, Vec<TimedAction>> = side_specs
        .keys()
        .copied()
        .map(|side| (side, Vec::new()))
        .collect();
    let mut opening_names = BTreeSet::new();
    for (opening_index, opening) in scenario.openings.iter().enumerate() {
        if !side_specs.contains_key(&opening.side) {
            return Err(ScenarioCompileError::UnknownOpeningSide {
                opening: opening_index,
                side: opening.side,
            });
        }
        if opening.name.trim().is_empty() {
            return Err(ScenarioCompileError::EmptyOpeningName {
                opening: opening_index,
            });
        }
        if !opening_names.insert((opening.side, opening.name.clone())) {
            return Err(ScenarioCompileError::DuplicateOpeningName {
                side: opening.side,
                name: opening.name.clone(),
            });
        }
        for (step_index, step) in opening.steps.iter().enumerate() {
            if step.at_tick >= scenario.max_ticks {
                return Err(ScenarioCompileError::StepOutsideDuration {
                    opening: opening_index,
                    step: step_index,
                    at_tick: step.at_tick,
                    max_ticks: scenario.max_ticks,
                });
            }
            validate_selector_definition(&step.actor, &catalog)?;
            let actors = entity_index.resolve_for_side(opening.side, &step.actor);
            if actors.is_empty() {
                return Err(ScenarioCompileError::UnknownInitialSelector {
                    opening: opening_index,
                    step: step_index,
                    side: Some(opening.side),
                    usage: SelectorUse::Actor,
                    selector: step.actor.clone(),
                });
            }
            let actions = compile_instruction(
                opening_index,
                step_index,
                opening.side,
                &step.actor,
                &actors,
                &step.instruction,
                &entity_index,
                &world,
                &catalog,
                &map,
            )?;
            actions_by_side
                .get_mut(&opening.side)
                .expect("validated side")
                .extend(actions.into_iter().map(|action| TimedAction {
                    at_tick: step.at_tick,
                    action,
                }));
        }
    }

    let sides = side_specs
        .into_values()
        .map(|specification| {
            let actions = actions_by_side
                .remove(&specification.side)
                .expect("initialized side action list");
            CompiledScenarioSide {
                controller: StrategyController::new(
                    specification.side,
                    options.cadence,
                    ScriptedStrategy::new(actions),
                ),
                specification,
            }
        })
        .collect();

    Ok(DeclarativeScenarioRuntime {
        name: scenario.name.clone(),
        kind: scenario.kind,
        seed: scenario.seed,
        max_ticks: scenario.max_ticks,
        world,
        entity_index,
        sides,
    })
}

fn validate_header(
    scenario: &DeclarativeScenario,
    map: &CompiledMap,
    options: ScenarioCompileOptions,
) -> Result<(), ScenarioCompileError> {
    if scenario.name.trim().is_empty() {
        return Err(ScenarioCompileError::EmptyName);
    }
    if scenario.max_ticks == 0 {
        return Err(ScenarioCompileError::ZeroMaxTicks);
    }
    if scenario.map != map.map_name {
        return Err(ScenarioCompileError::MapMismatch {
            requested: scenario.map.clone(),
            loaded: map.map_name.clone(),
        });
    }
    if scenario.sides.is_empty() {
        return Err(ScenarioCompileError::NoSides);
    }
    validate_sensor_range("line_of_sight", options.line_of_sight_range)?;
    validate_sensor_range("radar", options.radar_range)?;
    validate_sensor_range(
        "radar_position_resolution",
        options.radar_position_resolution,
    )?;
    if options.max_initial_entities == 0 {
        return Err(ScenarioCompileError::InvalidEntityLimit);
    }
    Ok(())
}

fn validate_sensor_range(name: &'static str, value: f32) -> Result<(), ScenarioCompileError> {
    if !value.is_finite() || value < 0.0 {
        Err(ScenarioCompileError::InvalidSensorRange(name))
    } else {
        Ok(())
    }
}

fn validate_side(side: &ScenarioSide, map: &CompiledMap) -> Result<(), ScenarioCompileError> {
    if side.side >= 2 {
        return Err(ScenarioCompileError::InvalidSide(side.side));
    }
    if side.team != side.side {
        return Err(ScenarioCompileError::UnsupportedTeamMapping {
            side: side.side,
            team: side.team,
        });
    }
    if !side.starting_metal.is_finite()
        || side.starting_metal < 0.0
        || !side.starting_energy.is_finite()
        || side.starting_energy < 0.0
    {
        return Err(ScenarioCompileError::InvalidStartingResources { side: side.side });
    }
    if !map.contains(side.start_position.x, side.start_position.z) {
        return Err(ScenarioCompileError::InvalidStartPosition {
            side: side.side,
            position: side.start_position,
        });
    }
    Ok(())
}

fn validate_formation(
    formation_index: usize,
    formation: &crate::strategy::FormationSpec,
    catalog: &WorldCatalog,
) -> Result<(), ScenarioCompileError> {
    if formation.members.is_empty() {
        return Err(ScenarioCompileError::EmptyFormation {
            formation: formation_index,
        });
    }
    if !formation.heading.is_finite() {
        return Err(ScenarioCompileError::InvalidFormationHeading {
            formation: formation_index,
        });
    }
    let mut expanded_count = 0_usize;
    for (member_index, member) in formation.members.iter().enumerate() {
        if member.label.trim().is_empty() {
            return Err(ScenarioCompileError::EmptyMemberLabel {
                formation: formation_index,
                member: member_index,
            });
        }
        if member.count == 0 {
            return Err(ScenarioCompileError::ZeroMemberCount {
                formation: formation_index,
                member: member_index,
            });
        }
        if catalog.id(&member.definition).is_none() {
            return Err(ScenarioCompileError::UnknownDefinition {
                definition: member.definition.clone(),
            });
        }
        expanded_count = expanded_count.saturating_add(member.count as usize);
    }
    use crate::strategy::FormationPattern;
    match &formation.pattern {
        FormationPattern::Line { spacing } if !finite_nonnegative(*spacing) => invalid_pattern(
            formation_index,
            "line spacing must be finite and non-negative",
        ),
        FormationPattern::Grid {
            columns,
            column_spacing,
            row_spacing,
        } if *columns == 0
            || !finite_nonnegative(*column_spacing)
            || !finite_nonnegative(*row_spacing) =>
        {
            invalid_pattern(
                formation_index,
                "grid columns must be non-zero and spacing finite and non-negative",
            )
        }
        FormationPattern::Ring { radius } if !finite_nonnegative(*radius) => invalid_pattern(
            formation_index,
            "ring radius must be finite and non-negative",
        ),
        FormationPattern::Custom { offsets }
            if offsets.len() != expanded_count
                || offsets
                    .iter()
                    .any(|offset| !offset.x.is_finite() || !offset.z.is_finite()) =>
        {
            invalid_pattern(
                formation_index,
                "custom offsets must be finite and exactly match expanded member count",
            )
        }
        _ => Ok(()),
    }
}

fn invalid_pattern(formation: usize, reason: &'static str) -> Result<(), ScenarioCompileError> {
    Err(ScenarioCompileError::InvalidFormationPattern { formation, reason })
}

fn finite_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn validate_spawn_passability(
    formation: usize,
    spawn: usize,
    position: Vec2,
    definition: &UnitDef,
    map: &CompiledMap,
) -> Result<(), ScenarioCompileError> {
    if !definition.strategic.is_mobile {
        return Ok(());
    }
    let Some(movement_class) = &definition.strategic.movement_class else {
        // Air and otherwise unrestricted units commonly have no MoveDef.
        return Ok(());
    };
    if map.movement_layer(movement_class).is_none() {
        return Err(ScenarioCompileError::UnknownMovementClass {
            formation,
            spawn,
            movement_class: movement_class.clone(),
        });
    }
    if !map.is_passable(movement_class, position.x, position.z) {
        return Err(ScenarioCompileError::ImpassableFormationPosition {
            formation,
            spawn,
            movement_class: movement_class.clone(),
            position,
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compile_instruction(
    opening: usize,
    step: usize,
    side: SideId,
    actor_selector: &EntitySelector,
    actors: &[EntityId],
    instruction: &OpeningInstruction,
    index: &ScenarioEntityIndex,
    world: &World,
    catalog: &WorldCatalog,
    map: &CompiledMap,
) -> Result<Vec<crate::strategy::StrategyAction>, ScenarioCompileError> {
    use crate::strategy::{ReclaimTarget, StrategyAction};

    let require_capability = |entity: EntityId,
                              capability: &'static str,
                              predicate: fn(&UnitDef) -> bool|
     -> Result<(), ScenarioCompileError> {
        let definition = entity_definition(world, catalog, entity);
        if predicate(definition) {
            Ok(())
        } else {
            Err(ScenarioCompileError::CapabilityMissing {
                opening,
                step,
                entity,
                capability,
            })
        }
    };
    match instruction {
        OpeningInstruction::Move { destination } => {
            validate_opening_position(opening, step, *destination, map)?;
            for &actor in actors {
                require_capability(actor, "movement", |definition| {
                    definition.strategic.is_mobile
                })?;
                let definition = entity_definition(world, catalog, actor);
                if let Some(movement_class) = &definition.strategic.movement_class
                    && !map.is_passable(movement_class, destination.x, destination.z)
                {
                    return Err(ScenarioCompileError::ImpassableOpeningPosition {
                        opening,
                        step,
                        entity: actor,
                        movement_class: movement_class.clone(),
                        position: *destination,
                    });
                }
            }
            Ok(vec![StrategyAction::Move {
                entities: actors.to_vec(),
                destination: *destination,
                mode: OrderMode::Replace,
            }])
        }
        OpeningInstruction::Attack { target } => {
            validate_selector_definition(target, catalog)?;
            for &actor in actors {
                require_capability(actor, "attack", |definition| !definition.weapons.is_empty())?;
            }
            let target = resolve_one(
                opening,
                step,
                None,
                SelectorUse::AttackTarget,
                target,
                index.resolve_globally(target),
            )?;
            let target_side = world.entities.get(target).expect("indexed entity").owner;
            if target_side == side {
                return Err(ScenarioCompileError::InvalidTargetSide {
                    opening,
                    step,
                    actor_side: side,
                    target_side,
                    usage: SelectorUse::AttackTarget,
                });
            }
            Ok(vec![StrategyAction::Attack {
                entities: actors.to_vec(),
                target: AttackTarget::Entity { entity: target },
                mode: OrderMode::Replace,
            }])
        }
        OpeningInstruction::Build {
            definition,
            position,
            facing,
        } => {
            let actor = require_one_actor(opening, step, actor_selector, actors)?;
            validate_opening_position(opening, step, *position, map)?;
            require_capability(actor, "build", |definition| definition.strategic.is_builder)?;
            catalog
                .id(definition)
                .ok_or_else(|| ScenarioCompileError::UnknownDefinition {
                    definition: definition.clone(),
                })?;
            validate_build_option(opening, step, actor, definition, world, catalog)?;
            Ok(vec![StrategyAction::Build {
                builder: actor,
                definition: definition.clone(),
                position: *position,
                facing: *facing,
                mode: OrderMode::Replace,
            }])
        }
        OpeningInstruction::Assist { target } => {
            validate_selector_definition(target, catalog)?;
            for &actor in actors {
                require_capability(actor, "assist", |definition| {
                    definition.strategic.is_builder && definition.strategic.can_assist
                })?;
            }
            let target = resolve_one(
                opening,
                step,
                Some(side),
                SelectorUse::AssistTarget,
                target,
                index.resolve_for_side(side, target),
            )?;
            Ok(vec![StrategyAction::Assist {
                entities: actors.to_vec(),
                target,
                mode: OrderMode::Replace,
            }])
        }
        OpeningInstruction::FactoryQueue { items } => {
            let actor = require_one_actor(opening, step, actor_selector, actors)?;
            require_capability(actor, "factory queue", |definition| {
                definition.strategic.is_factory
            })?;
            for item in items {
                if item.count == 0 {
                    return Err(ScenarioCompileError::InvalidQueueCount {
                        opening,
                        step,
                        definition: item.definition.clone(),
                    });
                }
                catalog.id(&item.definition).ok_or_else(|| {
                    ScenarioCompileError::UnknownDefinition {
                        definition: item.definition.clone(),
                    }
                })?;
                validate_build_option(opening, step, actor, &item.definition, world, catalog)?;
            }
            Ok(vec![StrategyAction::FactoryQueue {
                factory: actor,
                operation: FactoryQueueOperation::Replace {
                    items: items.clone(),
                },
            }])
        }
        OpeningInstruction::SetActivation { active } => {
            for &actor in actors {
                require_capability(actor, "activation", |definition| {
                    definition.strategic.onoffable
                })?;
            }
            Ok(vec![StrategyAction::SetActivation {
                entities: actors.to_vec(),
                active: *active,
            }])
        }
        OpeningInstruction::ReclaimArea { position, radius } => {
            validate_opening_position(opening, step, *position, map)?;
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(ScenarioCompileError::InvalidReclaimRadius { opening, step });
            }
            for &actor in actors {
                require_capability(actor, "reclaim", |definition| {
                    definition.strategic.is_builder
                })?;
            }
            Ok(vec![StrategyAction::Reclaim {
                entities: actors.to_vec(),
                target: ReclaimTarget::Area {
                    position: *position,
                    radius: *radius,
                },
                mode: OrderMode::Replace,
            }])
        }
        OpeningInstruction::Cancel => {
            let actor = require_one_actor(opening, step, actor_selector, actors)?;
            Ok(vec![StrategyAction::Cancel {
                target: CancelTarget::Orders { entity: actor },
            }])
        }
    }
}

fn entity_definition<'a>(
    world: &World,
    catalog: &'a WorldCatalog,
    entity: EntityId,
) -> &'a UnitDef {
    let definition = world
        .entities
        .get(entity)
        .expect("indexed entity")
        .definition;
    catalog.get(definition).expect("indexed definition")
}

fn require_one_actor(
    opening: usize,
    step: usize,
    selector: &EntitySelector,
    actors: &[EntityId],
) -> Result<EntityId, ScenarioCompileError> {
    if actors.len() == 1 {
        Ok(actors[0])
    } else {
        Err(ScenarioCompileError::AmbiguousSelector {
            opening,
            step,
            usage: SelectorUse::Actor,
            selector: selector.clone(),
            matches: actors.to_vec(),
        })
    }
}

fn resolve_one(
    opening: usize,
    step: usize,
    side: Option<SideId>,
    usage: SelectorUse,
    selector: &EntitySelector,
    matches: Vec<EntityId>,
) -> Result<EntityId, ScenarioCompileError> {
    match matches.as_slice() {
        [entity] => Ok(*entity),
        [] => Err(ScenarioCompileError::UnknownInitialSelector {
            opening,
            step,
            side,
            usage,
            selector: selector.clone(),
        }),
        _ => Err(ScenarioCompileError::AmbiguousSelector {
            opening,
            step,
            usage,
            selector: selector.clone(),
            matches,
        }),
    }
}

fn validate_selector_definition(
    selector: &EntitySelector,
    catalog: &WorldCatalog,
) -> Result<(), ScenarioCompileError> {
    if let EntitySelector::Definition { definition } = selector
        && catalog.id(definition).is_none()
    {
        return Err(ScenarioCompileError::UnknownDefinition {
            definition: definition.clone(),
        });
    }
    Ok(())
}

fn validate_opening_position(
    opening: usize,
    step: usize,
    position: Vec2,
    map: &CompiledMap,
) -> Result<(), ScenarioCompileError> {
    if map.contains(position.x, position.z) {
        Ok(())
    } else {
        Err(ScenarioCompileError::InvalidOpeningPosition {
            opening,
            step,
            position,
        })
    }
}

fn validate_build_option(
    opening: usize,
    step: usize,
    actor: EntityId,
    target_name: &str,
    world: &World,
    catalog: &WorldCatalog,
) -> Result<(), ScenarioCompileError> {
    let actor_definition = entity_definition(world, catalog, actor);
    if actor_definition.strategic.build_options.is_empty()
        || actor_definition
            .strategic
            .build_options
            .iter()
            .any(|option| option == target_name)
    {
        Ok(())
    } else {
        Err(ScenarioCompileError::BuildOptionUnavailable {
            opening,
            step,
            actor,
            definition: target_name.to_owned(),
        })
    }
}

fn facing_from_heading(heading: f32) -> BuildFacing {
    let forward = Vec2::from_heading(heading);
    if forward.x.abs() > forward.z.abs() {
        if forward.x >= 0.0 {
            BuildFacing::East
        } else {
            BuildFacing::West
        }
    } else if forward.z >= 0.0 {
        BuildFacing::South
    } else {
        BuildFacing::North
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{MAP_SOURCE_VERSION, MapSource, PassabilitySource, ScalarGridSource};
    use crate::model::{MovementDef, StrategicDef, UnitDef, WeaponDef};
    use crate::strategy::{ObservationRules, StrategyAction};

    fn map() -> Arc<CompiledMap> {
        Arc::new(
            CompiledMap::compile(MapSource {
                source_version: MAP_SOURCE_VERSION,
                map_name: "test-map".into(),
                content_hash: "test".into(),
                width: 256.0,
                depth: 256.0,
                water_level: 0.0,
                heights: ScalarGridSource {
                    cols: 2,
                    rows: 2,
                    origin_x: 0.0,
                    origin_z: 0.0,
                    step_x: 256.0,
                    step_z: 256.0,
                    values: vec![0.0; 4],
                },
                passability: vec![PassabilitySource {
                    movement_class: "bot".into(),
                    cols: 2,
                    rows: 2,
                    cell_size_x: 128.0,
                    cell_size_z: 128.0,
                    passable: vec![true; 4],
                }],
                metal_nodes: Vec::new(),
                continuous_metal: None,
                geothermal_nodes: Vec::new(),
                static_blockers: Vec::new(),
                wind_min: None,
                wind_max: None,
                tidal: None,
                provenance: Default::default(),
            })
            .unwrap(),
        )
    }

    fn unit(name: &str) -> UnitDef {
        UnitDef {
            name: name.into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 10.0,
            energy_cost: 0.0,
            strategic: StrategicDef::default(),
            movement: MovementDef::default(),
            weapons: Vec::new(),
        }
    }

    fn catalog() -> Arc<WorldCatalog> {
        let mut pawn = unit("pawn");
        pawn.strategic.is_mobile = true;
        pawn.strategic.movement_class = Some("bot".into());
        pawn.weapons.push(WeaponDef {
            name: "laser".into(),
            damage: 10.0,
            ..Default::default()
        });
        let mut builder = unit("builder");
        builder.strategic.is_mobile = true;
        builder.strategic.is_builder = true;
        builder.strategic.can_assist = true;
        builder.strategic.movement_class = Some("bot".into());
        builder.strategic.build_options = vec!["factory".into()];
        let mut factory = unit("factory");
        factory.strategic.is_building = true;
        factory.strategic.is_factory = true;
        factory.strategic.footprint = [2, 2];
        factory.strategic.build_options = vec!["pawn".into()];
        factory.strategic.build_constraints.yardmap = Some("oooo".into());
        factory.movement.radius = 16.0;
        factory.movement.height = 20.0;
        factory.movement.blocks_movement = true;
        Arc::new(WorldCatalog::new(vec![pawn, builder, factory]).unwrap())
    }

    fn scenario_json() -> String {
        r#"{
          "name":"opening smoke", "kind":"opening", "map":"test-map",
          "max_ticks":20, "seed":42,
          "observation":{"line_of_sight":true,"radar":false,"remember_last_seen":false},
          "sides":[
            {"side":0,"team":0,"start_position":{"x":32.0,"z":32.0},"starting_metal":125.0,"starting_energy":250.0},
            {"side":1,"team":1,"start_position":{"x":224.0,"z":224.0},"starting_metal":25.0,"starting_energy":50.0}
          ],
          "formations":[
            {"side":0,"anchor":{"x":48.0,"z":48.0},"heading":0.0,
             "members":[{"label":"raiders","definition":"pawn","count":2}],
             "pattern":{"type":"line","spacing":16.0}},
            {"side":0,"anchor":{"x":32.0,"z":80.0},"heading":0.0,
             "members":[{"label":"constructor","definition":"builder","count":1}],
             "pattern":{"type":"line","spacing":1.0}},
            {"side":1,"anchor":{"x":208.0,"z":208.0},"heading":0.0,
             "members":[{"label":"victim","definition":"pawn","count":1}],
             "pattern":{"type":"line","spacing":1.0}}
          ],
          "openings":[
            {"name":"raid","side":0,"steps":[
              {"at_tick":0,"actor":{"type":"label","label":"raiders"},
               "instruction":{"type":"move","destination":{"x":160.0,"z":160.0}}},
              {"at_tick":1,"actor":{"type":"label","label":"raiders"},
               "instruction":{"type":"attack","target":{"type":"label","label":"victim"}}},
              {"at_tick":2,"actor":{"type":"label","label":"constructor"},
               "instruction":{"type":"build","definition":"factory","position":{"x":96.0,"z":96.0},"facing":"South"}}
            ]}
          ]
        }"#
        .into()
    }

    #[test]
    fn json_compiles_and_runs_with_stable_groups_and_resources() {
        let mut runtime = compile_declarative_scenario_json(
            &scenario_json(),
            map(),
            catalog(),
            ScenarioCompileOptions {
                line_of_sight_range: 256.0,
                radar_range: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(runtime.seed, 42);
        assert_eq!(runtime.sides.len(), 2);
        assert_eq!(
            runtime
                .entity_index
                .resolve_for_side(
                    0,
                    &EntitySelector::Label {
                        label: "raiders".into()
                    }
                )
                .len(),
            2
        );
        assert_eq!(runtime.world.teams[0].economy.metal.stored, 125.0);
        assert_eq!(runtime.world.teams[1].economy.energy.stored, 50.0);
        assert_eq!(
            runtime.world.resource_ledger(0).unwrap().starting_metal,
            125.0
        );
        assert!(matches!(
            runtime.world.observation_policy(),
            ObservationPolicy::Filtered {
                line_of_sight_range: 256.0,
                radar_range: 0.0,
                ..
            }
        ));

        assert!(runtime.step().unwrap());
        assert!(matches!(
            &runtime.world.action_decisions[0].queued.action,
            StrategyAction::Move { entities, .. } if entities.len() == 2
        ));
        assert!(runtime.step().unwrap());
        assert!(matches!(
            runtime.world.action_decisions[0].queued.action,
            StrategyAction::Attack { .. }
        ));
        assert!(runtime.step().unwrap());
        assert!(matches!(
            runtime.world.action_decisions[0].queued.action,
            StrategyAction::Build { .. }
        ));
    }

    #[test]
    fn selector_for_single_target_reports_ambiguity() {
        let mut scenario: DeclarativeScenario = serde_json::from_str(&scenario_json()).unwrap();
        if let OpeningInstruction::Attack { target } =
            &mut scenario.openings[0].steps[1].instruction
        {
            *target = EntitySelector::Definition {
                definition: "pawn".into(),
            };
        }
        let error = compile_declarative_scenario(
            &scenario,
            map(),
            catalog(),
            ScenarioCompileOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ScenarioCompileError::AmbiguousSelector {
                usage: SelectorUse::AttackTarget,
                matches,
                ..
            } if matches.len() == 3
        ));
    }

    #[test]
    fn selector_cannot_refer_to_a_constructed_later_entity() {
        let mut scenario: DeclarativeScenario = serde_json::from_str(&scenario_json()).unwrap();
        scenario.openings[0]
            .steps
            .push(crate::strategy::OpeningStep {
                at_tick: 3,
                actor: EntitySelector::Label {
                    label: "future_factory".into(),
                },
                instruction: OpeningInstruction::FactoryQueue {
                    items: vec![crate::strategy::FactoryQueueItem {
                        definition: "pawn".into(),
                        count: 1,
                    }],
                },
            });
        let error = compile_declarative_scenario(
            &scenario,
            map(),
            catalog(),
            ScenarioCompileOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ScenarioCompileError::UnknownInitialSelector {
                usage: SelectorUse::Actor,
                ..
            }
        ));
        assert!(error.to_string().contains("constructed-later"));
    }

    #[test]
    fn malformed_geometry_is_rejected_before_materialization() {
        let mut scenario: DeclarativeScenario = serde_json::from_str(&scenario_json()).unwrap();
        scenario.formations[0].pattern = crate::strategy::FormationPattern::Custom {
            offsets: vec![Vec2::default()],
        };
        assert!(matches!(
            compile_declarative_scenario(
                &scenario,
                map(),
                catalog(),
                ScenarioCompileOptions::default()
            ),
            Err(ScenarioCompileError::InvalidFormationPattern { .. })
        ));
    }

    #[test]
    fn observation_memory_is_enabled_by_the_scenario() {
        let mut scenario: DeclarativeScenario = serde_json::from_str(&scenario_json()).unwrap();
        scenario.observation = ObservationRules {
            remember_last_seen: true,
            ..scenario.observation
        };
        let runtime = compile_declarative_scenario(
            &scenario,
            map(),
            catalog(),
            ScenarioCompileOptions::default(),
        )
        .unwrap();
        assert!(matches!(
            runtime.world.observation_policy(),
            ObservationPolicy::Filtered {
                remember_last_seen: true,
                ..
            }
        ));
    }

    #[test]
    fn equal_tick_steps_keep_opening_and_declaration_order() {
        let mut scenario: DeclarativeScenario = serde_json::from_str(&scenario_json()).unwrap();
        scenario.openings.push(crate::strategy::OpeningPlan {
            name: "second".into(),
            side: 0,
            steps: vec![crate::strategy::OpeningStep {
                at_tick: 0,
                actor: EntitySelector::Label {
                    label: "raiders".into(),
                },
                instruction: OpeningInstruction::Move {
                    destination: Vec2::new(200.0, 200.0),
                },
            }],
        });
        let mut runtime = compile_declarative_scenario(
            &scenario,
            map(),
            catalog(),
            ScenarioCompileOptions::default(),
        )
        .unwrap();
        runtime.step().unwrap();
        assert_eq!(runtime.world.action_decisions.len(), 2);
        let destinations = runtime
            .world
            .action_decisions
            .iter()
            .map(|decision| match decision.queued.action {
                StrategyAction::Move { destination, .. } => destination,
                _ => panic!("expected move"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            destinations,
            vec![Vec2::new(160.0, 160.0), Vec2::new(200.0, 200.0)]
        );
    }

    #[test]
    fn map_definition_side_and_position_errors_are_distinct() {
        let base: DeclarativeScenario = serde_json::from_str(&scenario_json()).unwrap();

        let mut wrong_map = base.clone();
        wrong_map.map = "somewhere-else".into();
        assert!(matches!(
            compile_declarative_scenario(
                &wrong_map,
                map(),
                catalog(),
                ScenarioCompileOptions::default()
            ),
            Err(ScenarioCompileError::MapMismatch { .. })
        ));

        let mut unknown_definition = base.clone();
        unknown_definition.formations[0].members[0].definition = "missing".into();
        assert!(matches!(
            compile_declarative_scenario(
                &unknown_definition,
                map(),
                catalog(),
                ScenarioCompileOptions::default()
            ),
            Err(ScenarioCompileError::UnknownDefinition { definition }) if definition == "missing"
        ));

        let mut invalid_side = base.clone();
        invalid_side.sides[0].side = 2;
        invalid_side.sides[0].team = 2;
        assert!(matches!(
            compile_declarative_scenario(
                &invalid_side,
                map(),
                catalog(),
                ScenarioCompileOptions::default()
            ),
            Err(ScenarioCompileError::InvalidSide(2))
        ));

        let mut invalid_position = base;
        invalid_position.formations[0].anchor = Vec2::new(-1.0, 32.0);
        assert!(matches!(
            compile_declarative_scenario(
                &invalid_position,
                map(),
                catalog(),
                ScenarioCompileOptions::default()
            ),
            Err(ScenarioCompileError::InvalidFormationPosition { .. })
        ));
    }
}
