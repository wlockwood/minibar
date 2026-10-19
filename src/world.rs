//! Deterministic closed-loop strategic world.
//!
//! This module joins catalog definitions, generational entities, placement,
//! layered occupancy, routing, streamed economy, construction, builders, and
//! factories. Tactical combat is opt-in: ordinary economy experiments avoid
//! its setup cost, while combat-enabled worlds retain aim, burst, projectile,
//! EMP, and death state across strategic ticks.

use crate::catalog::{CapabilityRequirement, IMPLEMENTED_CAPABILITIES, UnitCatalog};
use crate::construction::{
    BuildItem, BuilderContribution, ConstructionProject, ConstructionSystem, Factory, FactoryEvent,
    FactoryState, ProjectId, ProjectSpec, ProjectState, factory_exit_geometry,
};
use crate::economy::{ResourceDemand, ResourcePool, Resources, TeamEconomy};
use crate::engine::{
    CombatCommand, CombatDefinitionId, CombatError, CombatSpawn, CombatStepper, CombatantUpdate,
    TargetObservation,
};
use crate::entity::{EntityId, EntityStore};
use crate::map::{CompiledMap, MapError, StaticBlocker};
use crate::model::{DT, MovementDef, ReclaimDef, StartingResources, StrategicDef, UnitDef, Vec2};
use crate::occupancy::{CellLayers, OccupancyGrid, OccupantId};
use crate::placement::{
    BuildFacing, BuilderContext, PlacementRejection, PlacementRequest, PlacementResolution,
    execute_placement, snap_position, validate_placement,
};
use crate::routing::{GridRouter, Route, RouteRequest, RouteStatus};
use crate::score::{
    MilestoneCondition, MilestoneDeclaration, ResourceLedger, StrategicMetric,
    StrategicMetricVector,
};
use crate::sensors::{self, Detection, SensorUnit};
use crate::strategy::{
    ActionCapabilities, ActionDecision, ActionEntity, ActionInbox, ActionRejection,
    ActionValidator, ActionWorld, AttackTarget, CancelTarget, FactoryQueueOperation,
    ObservationPolicy, ObservationVisibility, ObservedEntity, ObservedFireIntent,
    ObservedResourceSite, OrderMode, QueuedAction, ReclaimTarget, ResourceObservation,
    StrategyAction, StrategyEntityKind, StrategyObservation,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

const UPKEEP_DEMAND_BIT: u64 = 1_u64 << 63;
const REPAIR_DEMAND_BIT: u64 = 1_u64 << 62;
const WALL_ROUTE_DELAY_CAP_SECONDS: f64 = 300.0;
const WALL_LOCAL_EXCHANGE_RADIUS: f32 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UnitDefId(pub u32);

#[derive(Clone, Debug)]
pub struct WorldCatalog {
    definitions: Arc<[UnitDef]>,
    names: BTreeMap<String, UnitDefId>,
}

impl WorldCatalog {
    pub fn new(definitions: Vec<UnitDef>) -> Result<Self, WorldError> {
        let mut names = BTreeMap::new();
        for (index, definition) in definitions.iter().enumerate() {
            let id = UnitDefId(u32::try_from(index).map_err(|_| WorldError::TooManyDefinitions)?);
            if names.insert(definition.name.clone(), id).is_some() {
                return Err(WorldError::DuplicateDefinition(definition.name.clone()));
            }
        }
        Ok(Self {
            definitions: definitions.into(),
            names,
        })
    }

    pub fn id(&self, name: &str) -> Option<UnitDefId> {
        self.names.get(name).copied()
    }

    pub fn get(&self, id: UnitDefId) -> Option<&UnitDef> {
        self.definitions.get(id.0 as usize)
    }

    pub fn get_by_name(&self, name: &str) -> Option<(UnitDefId, &UnitDef)> {
        let id = self.id(name)?;
        Some((id, self.get(id)?))
    }

    pub fn definitions(&self) -> &[UnitDef] {
        &self.definitions
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityRole {
    Mobile,
    Structure,
    Factory,
    Feature,
    Nanoframe { project_id: ProjectId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityLifecycle {
    Active,
    UnderConstruction,
    PendingActivation { tick: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldEntity {
    pub definition: UnitDefId,
    pub owner: u8,
    pub position: Vec2,
    #[serde(default)]
    pub heading: f32,
    pub health: f64,
    pub enabled: bool,
    pub powered: bool,
    pub role: EntityRole,
    pub lifecycle: EntityLifecycle,
    pub reclaim: Option<ReclaimState>,
}

/// A non-mutating placement result for the live human driver.  It is a
/// snapshot of the authoritative world at the displayed tick; a later build
/// command is still revalidated when it reaches the action phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildPreview {
    pub definition: String,
    pub requested_position: Vec2,
    pub snapped_position: Vec2,
    pub footprint: [u16; 2],
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection: Option<String>,
}

impl WorldEntity {
    pub fn is_active(&self) -> bool {
        self.lifecycle == EntityLifecycle::Active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReclaimState {
    pub progress: f64,
    pub reclaim_time: f64,
    pub metal: f64,
    pub energy: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuilderTaskState {
    Travelling,
    WaitingForSite,
    Constructing,
    Assisting,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuilderTask {
    pub project_id: ProjectId,
    pub state: BuilderTaskState,
    pub waypoints: VecDeque<Vec2>,
    pub route_epoch: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldMetrics {
    pub units_produced: u64,
    pub projects_cancelled: u64,
    pub projects_destroyed: u64,
    pub stalled_build_power_seconds: f64,
    pub idle_builder_seconds: f64,
    pub idle_factory_seconds: f64,
    pub health_repaired: f64,
    pub metal_reclaimed: f64,
    pub energy_reclaimed: f64,
    pub production_value: f64,
    pub sunk_cost: f64,
    pub metal_income_integral: f64,
    pub energy_income_integral: f64,
    pub site_occupancy_integral: f64,
    /// Time integral of the fraction of discrete resource sites occupied.
    pub map_control_integral: f64,
    /// Added routed travel time caused by this side's tracked wall occupancy.
    pub wall_route_delay_seconds: f64,
    /// Number of this side's tracked wall sections destroyed after placement.
    pub wall_breaches: u64,
    /// Actual hit-point damage absorbed by this side's tracked walls.
    pub wall_intercepted_damage: f64,
    /// Hostile mobiles that entered a destroyed wall section's footprint.
    pub wall_leaks: u64,
    /// Attacker-seconds with explicit fire intent geometrically blocked by a
    /// friendly tracked wall.
    pub wall_friendly_obstruction_seconds: f64,
    /// Enemy damage-value minus friendly damage-value within 256 elmos of
    /// this side's tracked walls.
    pub wall_local_exchange_value: f64,
    pub milestones_reached: u64,
    pub milestone_score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldTeam {
    pub economy: TeamEconomy,
    pub metrics: WorldMetrics,
    pub base_income: Resources,
    pub base_storage: Resources,
    /// Per-side baseline used by conservation reporting.
    pub starting_resources: Resources,
}

impl WorldTeam {
    fn new(config: &WorldConfig) -> Self {
        let starting_resources = Resources::new(
            config
                .starting_resources
                .metal
                .max(0.0)
                .min(config.base_storage.metal.max(0.0)),
            config
                .starting_resources
                .energy
                .max(0.0)
                .min(config.base_storage.energy.max(0.0)),
        );
        Self {
            economy: TeamEconomy::new(
                ResourcePool::new(
                    starting_resources.metal,
                    config.base_storage.metal,
                    config.base_income.metal,
                ),
                ResourcePool::new(
                    starting_resources.energy,
                    config.base_storage.energy,
                    config.base_income.energy,
                ),
            ),
            metrics: WorldMetrics::default(),
            base_income: config.base_income,
            base_storage: config.base_storage,
            starting_resources,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldConfig {
    pub starting_resources: Resources,
    pub base_storage: Resources,
    pub base_income: Resources,
    /// Captured map-mean output per unit of `wind_generator`.
    pub wind_output: f64,
    /// Captured map tidal output per unit of `tidal_generator`.
    pub tidal_output: f64,
    pub occupancy_cell_size: f32,
    pub observation_policy: ObservationPolicy,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            starting_resources: Resources::new(1_000.0, 1_000.0),
            base_storage: Resources::new(1_000.0, 1_000.0),
            base_income: Resources::ZERO,
            wind_output: 0.0,

            tidal_output: 0.0,
            occupancy_cell_size: 16.0,
            observation_policy: ObservationPolicy::Omniscient,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorldError {
    InvalidTeam(u8),
    DuplicateDefinition(String),
    UnknownDefinition(String),
    TooManyDefinitions,
    InvalidEntity(EntityId),
    NotBuilder(EntityId),
    NotFactory(EntityId),
    BuildOptionDenied {
        builder: EntityId,
        definition: String,
    },
    ProjectUnavailable(ProjectId),
    Placement(PlacementRejection),
    Occupancy(String),
    SimulationAlreadyStarted,
    CombatAlreadyEnabled,
    UnsupportedCombatCapabilities(Vec<CapabilityRequirement>),
    CombatTerrain(MapError),
    Combat(CombatError),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for WorldError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WorldEvent {
    ProjectOrdered {
        project_id: ProjectId,
        builder: EntityId,
    },
    ProjectPlaced {
        project_id: ProjectId,
        entity: EntityId,
    },
    PlacementRejected {
        project_id: ProjectId,
        reason: String,
    },
    ProjectCompleted {
        project_id: ProjectId,
        entity: EntityId,
    },
    EntityActivated {
        entity: EntityId,
    },
    FactoryExitBlocked {
        factory: EntityId,
        project_id: ProjectId,
    },
    FactoryReleased {
        factory: EntityId,
        entity: EntityId,
    },
    WreckCreated {
        source: EntityId,
        wreck: EntityId,
    },
    Reclaimed {
        entity: EntityId,
        side: u8,
    },
    EntityDestroyed {
        entity: EntityId,
    },
    CombatDamage {
        entity: EntityId,
        amount: f64,
        remaining_health: f64,
    },
}

#[derive(Clone, Debug)]
struct ProjectRuntime {
    definition: UnitDefId,
    owner: u8,
    position: Vec2,
    facing: BuildFacing,
    is_wall: bool,
    factory: Option<EntityId>,
    entity: Option<EntityId>,
    placement: Option<PlacementResolution>,
    priority: i32,
}

#[derive(Clone, Debug)]
struct FactoryRuntime {
    factory: Factory,
    facing: BuildFacing,
    force_exit_blocked: bool,
}

struct WorldCombatRuntime {
    stepper: CombatStepper,
    known: BTreeSet<EntityId>,
    scratch: Vec<EntityId>,
    /// Effective entity targets selected for this tactical tick, including
    /// transient local auto-acquisition. This is presentation-only state used
    /// by observations; it never changes combat resolution.
    targets: BTreeMap<EntityId, EntityId>,
    fractional_ticks: f64,
}

#[derive(Clone, Copy, Debug)]
struct WallRecord {
    owner: u8,
    position: Vec2,
    bounds: (f32, f32, f32, f32),
}

#[derive(Clone, Debug)]
struct WallBreachRecord {
    owner: u8,
    bounds: (f32, f32, f32, f32),
    entrants: BTreeSet<EntityId>,
}

#[derive(Clone, Debug, Serialize)]
struct ContactMemory {
    last: ObservedEntity,
    ghost: Option<ObservedEntity>,
    continuous: bool,
    uncertainty: f32,
    /// `sensor_seconds` of the last tick a sensor detected this entity.
    seen_at: f64,
}

#[derive(Clone, Debug)]
struct CloakState {
    wanted: bool,
    cloaked: bool,
    blocked_until: f64,
    next_check: f64,
    last_position: Vec2,
}

/// One deterministic strategic trial. Parallelism belongs around `World`, not
/// within `step`, so action and resource ordering remain reproducible.
pub struct World {
    pub tick: u64,
    pub map: Arc<CompiledMap>,
    pub catalog: Arc<WorldCatalog>,
    pub occupancy: OccupancyGrid,
    pub entities: EntityStore<WorldEntity>,
    pub teams: [WorldTeam; 2],
    pub construction: ConstructionSystem,
    pub events: Vec<WorldEvent>,
    pub action_decisions: Vec<ActionDecision>,
    config: WorldConfig,
    router: GridRouter,
    builder_tasks: BTreeMap<EntityId, BuilderTask>,
    projects: BTreeMap<ProjectId, ProjectRuntime>,
    factories: BTreeMap<EntityId, FactoryRuntime>,
    action_inbox: ActionInbox,
    action_validator: ActionValidator,
    movement_orders: BTreeMap<EntityId, VecDeque<Vec2>>,
    movement_goals: BTreeMap<EntityId, VecDeque<Vec2>>,
    movement_route_epochs: BTreeMap<EntityId, u64>,
    attack_orders: BTreeMap<EntityId, AttackTarget>,
    repair_orders: BTreeMap<EntityId, EntityId>,
    reclaim_orders: BTreeMap<EntityId, ReclaimTarget>,
    observation_policy: ObservationPolicy,
    observation_memory: RefCell<[BTreeMap<EntityId, ContactMemory>; 2]>,
    observation_memory_policy: RefCell<[Option<ObservationPolicy>; 2]>,
    cloak_states: BTreeMap<EntityId, CloakState>,
    sensor_seconds: f64,
    next_project_id: ProjectId,
    combat: Option<WorldCombatRuntime>,
    walls: BTreeMap<EntityId, WallRecord>,
    wall_breach_records: Vec<WallBreachRecord>,
    pub milestones: Vec<MilestoneDeclaration>,
    pub milestones_achieved: [BTreeSet<usize>; 2],
}

impl World {
    pub fn new(
        map: Arc<CompiledMap>,
        catalog: Arc<WorldCatalog>,
        config: WorldConfig,
    ) -> Result<Self, WorldError> {
        let occupancy = OccupancyGrid::new(map.width, map.depth, config.occupancy_cell_size)
            .map_err(WorldError::Occupancy)?;
        let mut config = config;
        if config.wind_output == 0.0 {
            config.wind_output = map.average_wind() as f64;
        }
        if config.tidal_output == 0.0 {
            config.tidal_output = map.tidal as f64;
        }
        Ok(Self {
            tick: 0,
            map,
            catalog,
            occupancy,
            entities: EntityStore::new(),
            teams: std::array::from_fn(|_| WorldTeam::new(&config)),
            construction: ConstructionSystem::default(),
            events: Vec::new(),
            action_decisions: Vec::new(),
            config,
            router: GridRouter::new(),
            builder_tasks: BTreeMap::new(),
            projects: BTreeMap::new(),
            factories: BTreeMap::new(),
            action_inbox: ActionInbox::default(),
            action_validator: ActionValidator::default(),
            movement_orders: BTreeMap::new(),
            movement_goals: BTreeMap::new(),
            movement_route_epochs: BTreeMap::new(),
            attack_orders: BTreeMap::new(),
            repair_orders: BTreeMap::new(),
            reclaim_orders: BTreeMap::new(),
            observation_policy: config.observation_policy,
            observation_memory: RefCell::new(std::array::from_fn(|_| BTreeMap::new())),
            observation_memory_policy: RefCell::new([None, None]),
            cloak_states: BTreeMap::new(),
            sensor_seconds: 0.0,
            next_project_id: 1,
            combat: None,
            walls: BTreeMap::new(),
            wall_breach_records: Vec::new(),
            milestones: Vec::new(),
            milestones_achieved: [BTreeSet::new(), BTreeSet::new()],
        })
    }

    /// Creates a world and materializes every captured map blocker as an
    /// independently removable feature entity. Existing catalog IDs remain
    /// stable; private proxy definitions are appended in deterministic capture
    /// order solely to provide collision and observation metadata.
    pub fn new_with_static_features(
        map: Arc<CompiledMap>,
        catalog: Arc<WorldCatalog>,
        config: WorldConfig,
    ) -> Result<Self, WorldError> {
        if map.static_blockers.is_empty() {
            return Self::new(map, catalog, config);
        }

        let mut definitions = catalog.definitions().to_vec();
        let mut proxies = BTreeMap::new();
        let mut proxy_ids = Vec::with_capacity(map.static_blockers.len());
        for blocker in &map.static_blockers {
            let width = blocker.max_x - blocker.min_x;
            let depth = blocker.max_z - blocker.min_z;
            let key = (
                blocker.definition.as_str(),
                blocker.blocks_movement,
                blocker.blocks_fire,
                blocker.fire_height.to_bits(),
                width.max(depth).to_bits(),
            );
            if let Some(&definition) = proxies.get(&key) {
                proxy_ids.push(definition);
                continue;
            }
            let definition = UnitDefId(
                u32::try_from(definitions.len()).map_err(|_| WorldError::TooManyDefinitions)?,
            );
            let mut name = format!(
                "__map_feature/{}/{}/{}",
                proxies.len(),
                blocker.feature_id,
                blocker.definition
            );
            while definitions.iter().any(|definition| definition.name == name) {
                name.insert(0, '_');
            }
            definitions.push(static_feature_definition(name, blocker));
            proxies.insert(key, definition);
            proxy_ids.push(definition);
        }
        drop(proxies);
        let catalog = Arc::new(WorldCatalog::new(definitions)?);
        let blockers = map.static_blockers.clone();
        let mut world = Self::new(map, catalog, config)?;
        for (definition, blocker) in proxy_ids.into_iter().zip(&blockers) {
            world.insert_static_feature(definition, blocker)?;
        }
        Ok(world)
    }

    /// Enables the persistent tactical kernel for this world. The selected
    /// movement class supplies baseline terrain passability, while definitions
    /// with distinct movement classes receive dedicated passability masks.
    pub fn enable_combat(
        &mut self,
        seed: u64,
        movement_class: &str,
        terrain_cell_size: f32,
    ) -> Result<(), WorldError> {
        if self.combat.is_some() {
            return Err(WorldError::CombatAlreadyEnabled);
        }
        let tactical_catalog = UnitCatalog {
            units: self
                .catalog
                .definitions()
                .iter()
                .cloned()
                .map(|definition| (definition.name.clone(), definition))
                .collect(),
            ..Default::default()
        };
        if let Err(requirements) =
            tactical_catalog.validate_capabilities(IMPLEMENTED_CAPABILITIES.iter().copied())
        {
            return Err(WorldError::UnsupportedCombatCapabilities(requirements));
        }
        let terrain = self
            .map
            .to_tactical_terrain(movement_class, terrain_cell_size)
            .map_err(WorldError::CombatTerrain)?;
        let definitions = self
            .catalog
            .definitions()
            .iter()
            .cloned()
            .map(Arc::new)
            .collect();
        let mut stepper = CombatStepper::new(terrain, definitions, seed);
        let mut class_masks: std::collections::BTreeMap<&str, Arc<[bool]>> =
            std::collections::BTreeMap::new();
        for (def_index, def) in self.catalog.definitions().iter().enumerate() {
            if let Some(ref class) = def.strategic.movement_class
                && !class.is_empty()
                && self.map.movement_layer(class).is_some()
            {
                let mask = match class_masks.get(class.as_str()) {
                    Some(cached) => Arc::clone(cached),
                    None => {
                        let raster = self
                            .map
                            .tactical_passability_mask(class, terrain_cell_size)
                            .map_err(WorldError::CombatTerrain)?;
                        let arc_mask: Arc<[bool]> = Arc::from(raster);
                        class_masks.insert(class.as_str(), Arc::clone(&arc_mask));
                        arc_mask
                    }
                };
                stepper.set_definition_movement_mask(CombatDefinitionId(def_index), mask);
            }
        }
        let reserve_combatants = self.entities.len().saturating_add(256);
        stepper.reserve(
            reserve_combatants,
            3_600,
            reserve_combatants.saturating_mul(16).max(1_024),
        );
        let mut known = BTreeSet::new();
        for (id, entity) in self.entities.iter() {
            let collision_height_scale = match entity.role {
                EntityRole::Nanoframe { project_id } => self
                    .construction
                    .get(project_id)
                    .map_or(0.0, |project| project.progress as f32),
                _ => 1.0,
            };
            stepper
                .insert(CombatSpawn {
                    key: id.to_raw(),
                    side: entity.owner,
                    definition: CombatDefinitionId(entity.definition.0 as usize),
                    position: entity.position,
                    heading: entity.heading,
                    hp: Some(entity.health as f32),
                })
                .map_err(WorldError::Combat)?;
            if collision_height_scale < 1.0 {
                stepper
                    .update(
                        id.to_raw(),
                        CombatantUpdate {
                            collision_height_scale: Some(collision_height_scale),
                            ..Default::default()
                        },
                    )
                    .map_err(WorldError::Combat)?;
            }
            known.insert(id);
        }
        self.combat = Some(WorldCombatRuntime {
            stepper,
            known,
            scratch: Vec::with_capacity(reserve_combatants),
            targets: BTreeMap::new(),
            fractional_ticks: 0.0,
        });
        Ok(())
    }

    pub fn combat_enabled(&self) -> bool {
        self.combat.is_some()
    }

    pub fn pending_combat_impacts(&self) -> usize {
        self.combat
            .as_ref()
            .map_or(0, |combat| combat.stepper.pending_impacts())
    }

    pub fn combat_state_hash(&self) -> Option<u64> {
        self.combat
            .as_ref()
            .map(|combat| combat.stepper.deterministic_state_hash())
    }

    /// Current yaw of the entity's primary turret, when tactical combat is
    /// enabled. This is presentation state only; fixed weapons use the hull
    /// heading and therefore have no separate turret angle.
    pub fn primary_turret_heading(&self, entity: EntityId) -> Option<f32> {
        self.combat
            .as_ref()?
            .stepper
            .primary_turret_heading(entity.to_raw())
    }

    /// Sets one side's initial resource baseline before the first simulation
    /// step. Values are clamped to the side's current storage capacities.
    pub fn set_team_starting_resources(
        &mut self,
        side: u8,
        resources: Resources,
    ) -> Result<(), WorldError> {
        self.validate_team(side)?;
        if self.tick != 0 {
            return Err(WorldError::SimulationAlreadyStarted);
        }
        let team = &mut self.teams[side as usize];
        let starting = Resources::new(
            finite_nonnegative(resources.metal).min(team.economy.metal.capacity),
            finite_nonnegative(resources.energy).min(team.economy.energy.capacity),
        );
        team.economy.metal.stored = starting.metal;
        team.economy.energy.stored = starting.energy;
        team.starting_resources = starting;
        Ok(())
    }

    fn insert_static_feature(
        &mut self,
        definition: UnitDefId,
        blocker: &StaticBlocker,
    ) -> Result<EntityId, WorldError> {
        let entity = self.entities.insert(WorldEntity {
            definition,
            owner: u8::MAX,
            position: Vec2::new(blocker.x, blocker.z),
            heading: 0.0,
            health: 1.0,
            enabled: false,
            powered: false,
            role: EntityRole::Feature,
            lifecycle: EntityLifecycle::Active,
            reclaim: blocker.reclaimable.then_some(ReclaimState {
                progress: 0.0,
                reclaim_time: blocker.reclaim_time as f64,
                metal: blocker.metal as f64,
                energy: blocker.energy as f64,
            }),
        });
        let cell_size = self.occupancy.cell_size();
        let left = (blocker.min_x / cell_size).floor() as u32;
        let top = (blocker.min_z / cell_size).floor() as u32;
        let right = ((blocker.max_x / cell_size).ceil() as u32)
            .saturating_sub(1)
            .min(self.occupancy.cols().saturating_sub(1));
        let bottom = ((blocker.max_z / cell_size).ceil() as u32)
            .saturating_sub(1)
            .min(self.occupancy.rows().saturating_sub(1));
        let layers = CellLayers {
            blocks_movement: blocker.blocks_movement,
            fire_height: blocker.blocks_fire.then_some(blocker.fire_height),
            construction_reservation: blocker.blocks_movement,
            factory_yard: false,
        };
        let contributions =
            (top..=bottom).flat_map(|row| (left..=right).map(move |col| ((col, row), layers)));
        if !self
            .occupancy
            .add_occupant(occupant(entity), None, contributions)
        {
            self.entities.remove(entity);
            return Err(WorldError::Occupancy(
                "failed to register initial map feature".into(),
            ));
        }
        Ok(entity)
    }

    pub fn spawn_active(
        &mut self,
        owner: u8,
        definition: UnitDefId,
        position: Vec2,
        facing: BuildFacing,
    ) -> Result<EntityId, WorldError> {
        self.validate_team(owner)?;
        let unit = self.definition(definition)?.clone();
        let role = entity_role(&unit);
        let entity = self.entities.insert(WorldEntity {
            definition,
            owner,
            position,
            heading: facing.heading(),
            health: unit.health.max(0.0) as f64,
            enabled: true,
            powered: true,
            role,
            lifecycle: EntityLifecycle::Active,
            reclaim: None,
        });
        if unit.strategic.is_building {
            let mut request =
                PlacementRequest::from_unit_def(occupant(entity), &unit, position, facing);
            request.owner = Some(owner);
            let resolution = match validate_placement(
                &self.map,
                &self.occupancy,
                &request,
                completed_layers(&unit),
            ) {
                Ok(resolution) => resolution,
                Err(error) => {
                    self.entities.remove(entity);
                    return Err(WorldError::Placement(error));
                }
            };
            if !execute_placement(&mut self.occupancy, &resolution) {
                self.entities.remove(entity);
                return Err(WorldError::Occupancy(
                    "failed to register active structure".into(),
                ));
            }
            if unit.strategic.is_factory
                && let Some(entity_state) = self.entities.get_mut(entity)
            {
                entity_state.position = resolution.snapped_position;
            }
            if unit.strategic.is_wall {
                self.walls.insert(
                    entity,
                    WallRecord {
                        owner,
                        position: resolution.snapped_position,
                        bounds: resolution.bounds,
                    },
                );
            }
        } else if unit.strategic.is_wall {
            let radius = unit.movement.radius.max(0.0);
            self.walls.insert(
                entity,
                WallRecord {
                    owner,
                    position,
                    bounds: (
                        position.x - radius,
                        position.z - radius,
                        position.x + radius,
                        position.z + radius,
                    ),
                },
            );
        }
        if unit.strategic.is_factory {
            self.factories.insert(
                entity,
                FactoryRuntime {
                    factory: Factory::new(entity.to_raw(), unit.strategic.build_speed as f64, 0),
                    facing,
                    force_exit_blocked: false,
                },
            );
        }
        Ok(entity)
    }

    /// Explicitly registers an existing entity as a tracked wall for metric analysis.
    pub fn register_wall(&mut self, entity: EntityId) -> Result<(), WorldError> {
        let entry = self
            .entities
            .get(entity)
            .ok_or(WorldError::InvalidEntity(entity))?;
        let definition = self.definition(entry.definition)?;
        let radius = definition.movement.radius.max(0.0);
        let bounds = (
            entry.position.x - radius,
            entry.position.z - radius,
            entry.position.x + radius,
            entry.position.z + radius,
        );
        self.walls.insert(
            entity,
            WallRecord {
                owner: entry.owner,
                position: entry.position,
                bounds,
            },
        );
        Ok(())
    }

    pub fn issue_build(
        &mut self,
        builder: EntityId,
        definition: UnitDefId,
        position: Vec2,
        facing: BuildFacing,
        priority: i32,
    ) -> Result<ProjectId, WorldError> {
        let is_wall = self
            .definition(definition)
            .is_ok_and(|def| def.strategic.is_wall);
        self.issue_build_internal(builder, definition, position, facing, priority, is_wall)
    }

    /// Returns the current routed waypoints for an owned entity.  This is
    /// deliberately a copied, presentation-only view so live rendering cannot
    /// retain or mutate the router's internal command state.
    pub fn planned_route(&self, entity: EntityId) -> Vec<Vec2> {
        self.movement_orders
            .get(&entity)
            .map(|waypoints| waypoints.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Returns construction progress for the nanoframe represented by an
    /// entity, if any.
    pub fn construction_progress(&self, entity: EntityId) -> Option<f32> {
        self.projects.iter().find_map(|(&project_id, runtime)| {
            (runtime.entity == Some(entity))
                .then(|| {
                    self.construction
                        .get(project_id)
                        .map(|project| project.progress as f32)
                })
                .flatten()
        })
    }

    /// Performs the same static placement checks used by action validation,
    /// without allocating an entity, project, reservation, or route.
    pub fn preview_build(
        &self,
        side: u8,
        builder: EntityId,
        definition_name: &str,
        position: Vec2,
        facing: BuildFacing,
    ) -> BuildPreview {
        let fallback = BuildPreview {
            definition: definition_name.to_owned(),
            requested_position: position,
            snapped_position: position,
            footprint: [0, 0],
            valid: false,
            rejection: None,
        };
        let Some(builder_entity) = self.entities.get(builder) else {
            return BuildPreview {
                rejection: Some("unknown builder".into()),
                ..fallback
            };
        };
        if builder_entity.owner != side {
            return BuildPreview {
                rejection: Some("builder is not owned by this side".into()),
                ..fallback
            };
        }
        let Some(builder_def) = self.catalog.get(builder_entity.definition) else {
            return BuildPreview {
                rejection: Some("builder definition is unavailable".into()),
                ..fallback
            };
        };
        let Some((_, definition)) = self.catalog.get_by_name(definition_name) else {
            return BuildPreview {
                rejection: Some("unknown build definition".into()),
                ..fallback
            };
        };
        let snapped_position = snap_position(position, definition.strategic.footprint, facing);
        let mut preview = BuildPreview {
            definition: definition.name.clone(),
            requested_position: position,
            snapped_position,
            footprint: definition.strategic.footprint,
            valid: false,
            rejection: None,
        };
        if !builder_entity.is_active() || !builder_def.strategic.is_builder {
            preview.rejection = Some("selected entity is not an active builder".into());
            return preview;
        }
        if !definition.strategic.is_building {
            preview.rejection = Some("definition is not a placeable building".into());
            return preview;
        }
        if !builder_def.strategic.build_options.is_empty()
            && !builder_def
                .strategic
                .build_options
                .iter()
                .any(|option| option == &definition.name)
        {
            preview.rejection = Some("definition is not in the builder's build options".into());
            return preview;
        }
        let mut request = PlacementRequest::from_unit_def(
            OccupantId(u64::MAX - u64::from(side)),
            definition,
            position,
            facing,
        );
        request.owner = Some(side);
        match validate_placement(
            &self.map,
            &self.occupancy,
            &request,
            CellLayers {
                blocks_movement: definition.movement.blocks_movement,
                fire_height: definition.movement.blocks_fire.then_some(0.0),
                construction_reservation: true,
                factory_yard: false,
            },
        ) {
            Ok(resolution) => {
                preview.snapped_position = resolution.snapped_position;
                preview.valid = true;
            }
            Err(error) => preview.rejection = Some(error.to_string()),
        }
        preview
    }

    pub fn issue_wall(
        &mut self,
        builder: EntityId,
        definition: UnitDefId,
        position: Vec2,
        facing: BuildFacing,
        priority: i32,
    ) -> Result<ProjectId, WorldError> {
        self.issue_build_internal(builder, definition, position, facing, priority, true)
    }

    fn issue_build_internal(
        &mut self,
        builder: EntityId,
        definition: UnitDefId,
        position: Vec2,
        facing: BuildFacing,
        priority: i32,
        is_wall: bool,
    ) -> Result<ProjectId, WorldError> {
        let builder_entity = self
            .entities
            .get(builder)
            .ok_or(WorldError::InvalidEntity(builder))?;
        let builder_def = self.definition(builder_entity.definition)?;
        if !builder_entity.is_active() || !builder_def.strategic.is_builder {
            return Err(WorldError::NotBuilder(builder));
        }
        let builder_owner = builder_entity.owner;
        let target_def = self.definition(definition)?;
        if !builder_def.strategic.build_options.is_empty()
            && !builder_def
                .strategic
                .build_options
                .iter()
                .any(|name| name == &target_def.name)
        {
            return Err(WorldError::BuildOptionDenied {
                builder,
                definition: target_def.name.clone(),
            });
        }
        let project_id = self.allocate_project_id();
        self.projects.insert(
            project_id,
            ProjectRuntime {
                definition,
                owner: builder_owner,
                position,
                facing,
                is_wall,
                factory: None,
                entity: None,
                placement: None,
                priority,
            },
        );
        let task = self.route_task(builder, project_id, position, BuilderTaskState::Travelling)?;
        self.builder_tasks.insert(builder, task);
        self.events.push(WorldEvent::ProjectOrdered {
            project_id,
            builder,
        });
        Ok(project_id)
    }

    pub fn assist(&mut self, builder: EntityId, project_id: ProjectId) -> Result<(), WorldError> {
        let project = self
            .projects
            .get(&project_id)
            .ok_or(WorldError::ProjectUnavailable(project_id))?;
        if project.entity.is_none() {
            return Err(WorldError::ProjectUnavailable(project_id));
        }
        let builder_entity = self
            .entities
            .get(builder)
            .ok_or(WorldError::InvalidEntity(builder))?;
        let definition = self.definition(builder_entity.definition)?;
        if !builder_entity.is_active()
            || !definition.strategic.is_builder
            || !definition.strategic.can_assist
        {
            return Err(WorldError::NotBuilder(builder));
        }
        let task = self.route_task(
            builder,
            project_id,
            project.position,
            BuilderTaskState::Assisting,
        )?;
        self.builder_tasks.insert(builder, task);
        Ok(())
    }

    pub fn queue_factory(
        &mut self,
        factory: EntityId,
        definition: UnitDefId,
    ) -> Result<(), WorldError> {
        let item = BuildItem::from_unit(self.definition(definition)?);
        self.factories
            .get_mut(&factory)
            .ok_or(WorldError::NotFactory(factory))?
            .factory
            .append(item);
        Ok(())
    }

    /// Debug/test override that forces the selected factory's exit closed.
    /// Normal simulations derive blockage from terrain, occupancy, and nearby
    /// entity collision circles automatically.
    pub fn set_factory_exit_blocked(
        &mut self,
        factory: EntityId,
        blocked: bool,
    ) -> Result<(), WorldError> {
        self.factories
            .get_mut(&factory)
            .ok_or(WorldError::NotFactory(factory))?
            .force_exit_blocked = blocked;
        Ok(())
    }

    pub fn factory_state(&self, factory: EntityId) -> Option<&FactoryState> {
        self.factories
            .get(&factory)
            .map(|runtime| &runtime.factory.state)
    }

    pub fn cancel_project(&mut self, project_id: ProjectId) -> bool {
        let Some(project) = self.projects.remove(&project_id) else {
            return false;
        };
        if let Some(construction) = self.construction.get(project_id) {
            self.teams[project.owner as usize].metrics.sunk_cost +=
                project_spent_value(construction);
        }
        if let Some(construction) = self.construction.get_mut(project_id) {
            construction.cancel();
        }
        self.construction.remove(project_id);
        if let Some(entity) = project.entity {
            self.forget_observed_removal(entity);
            self.walls.remove(&entity);
            self.occupancy.remove_occupant(occupant(entity));
            self.entities.remove(entity);
        }
        self.builder_tasks
            .retain(|_, task| task.project_id != project_id);
        self.teams[project.owner as usize]
            .metrics
            .projects_cancelled += 1;
        true
    }

    fn forget_observed_removal(&mut self, entity: EntityId) {
        // A death observed in LOS or radar is known. Only an entirely unseen
        // building death can leave a dead ghost (UnitDrawerData::canSeeGhost).
        let policies = *self.observation_memory_policy.borrow();
        for side in 0..2 {
            let observed = policies[side].is_some_and(|policy| {
                let Some(target) = self.entities.get(entity) else {
                    return false;
                };
                match policy {
                    ObservationPolicy::Omniscient => true,
                    ObservationPolicy::Filtered {
                        line_of_sight_range,
                        radar_range,
                        ..
                    } => {
                        let units = self.sensor_snapshots(line_of_sight_range, radar_range);
                        let target =
                            self.sensor_snapshot(entity, target, line_of_sight_range, radar_range);
                        sensors::detect(side as u8, &target, &units, |a, b, c, d| {
                            self.sensor_terrain_clear(a, b, c, d)
                        })
                        .is_some()
                    }
                }
            });
            if observed {
                self.observation_memory.get_mut()[side as usize].remove(&entity);
            }
        }
    }

    pub fn destroy_entity(&mut self, entity: EntityId) -> bool {
        self.forget_observed_removal(entity);
        let Some(removed) = self.entities.remove(entity) else {
            return false;
        };
        let breached_wall = self.walls.remove(&entity);
        if let Some(wall) = breached_wall
            && wall.owner < 2
        {
            self.teams[wall.owner as usize].metrics.wall_breaches += 1;
            self.wall_breach_records.push(WallBreachRecord {
                owner: wall.owner,
                bounds: wall.bounds,
                entrants: BTreeSet::new(),
            });
        }
        if let Some(combat) = &mut self.combat {
            combat.known.remove(&entity);
            let _ = combat.stepper.remove(entity.to_raw());
        }
        let corpse = (removed.role != EntityRole::Feature)
            .then(|| {
                self.catalog
                    .get(removed.definition)
                    .and_then(|definition| definition.strategic.corpse.clone())
            })
            .flatten();
        self.occupancy.remove_occupant(occupant(entity));
        self.builder_tasks.remove(&entity);
        self.movement_orders.remove(&entity);
        self.movement_goals.remove(&entity);
        self.movement_route_epochs.remove(&entity);
        self.factories.remove(&entity);
        let linked_project = self
            .projects
            .iter()
            .find_map(|(id, project)| (project.entity == Some(entity)).then_some(*id));
        if let Some(project_id) = linked_project {
            if let Some(construction) = self.construction.get(project_id) {
                self.teams[removed.owner as usize].metrics.sunk_cost +=
                    project_spent_value(construction);
            }
            self.construction.remove(project_id);
            self.projects.remove(&project_id);
            self.builder_tasks
                .retain(|_, task| task.project_id != project_id);
            self.teams[removed.owner as usize]
                .metrics
                .projects_destroyed += 1;
        }
        self.events.push(WorldEvent::EntityDestroyed { entity });
        if let Some(corpse) = corpse.filter(|corpse| corpse.reclaim.reclaimable) {
            let definition = self.catalog.id(&corpse.name).unwrap_or(removed.definition);
            let wreck = self.entities.insert(WorldEntity {
                definition,
                owner: u8::MAX,
                position: removed.position,
                heading: removed.heading,
                health: 1.0,
                enabled: false,
                powered: false,
                role: EntityRole::Feature,
                lifecycle: EntityLifecycle::Active,
                reclaim: Some(ReclaimState {
                    progress: 0.0,
                    reclaim_time: corpse.reclaim.reclaim_time.max(DT) as f64,
                    metal: corpse.reclaim.metal.max(0.0) as f64,
                    energy: corpse.reclaim.energy.max(0.0) as f64,
                }),
            });
            self.events.push(WorldEvent::WreckCreated {
                source: entity,
                wreck,
            });
        }
        true
    }

    pub fn builder_task(&self, builder: EntityId) -> Option<&BuilderTask> {
        self.builder_tasks.get(&builder)
    }

    pub fn project_entity(&self, project_id: ProjectId) -> Option<EntityId> {
        self.projects.get(&project_id)?.entity
    }

    pub fn enqueue_action(&mut self, action: QueuedAction) {
        self.action_inbox.push(action);
    }

    pub fn enqueue_actions(&mut self, actions: impl IntoIterator<Item = QueuedAction>) {
        self.action_inbox.extend(actions);
    }

    pub fn pending_action_count(&self) -> usize {
        self.action_inbox.len()
    }

    pub fn attack_order(&self, entity: EntityId) -> Option<AttackTarget> {
        self.attack_orders.get(&entity).copied()
    }

    pub fn observation_policy(&self) -> ObservationPolicy {
        self.observation_policy
    }

    /// Desired cloak and actual cloak differ when enemies, EMP, firing or
    /// energy costs prevent cloaking. This method reports actual visibility.
    pub fn entity_is_cloaked(&self, id: EntityId) -> bool {
        let Some(entity) = self.entities.get(id) else {
            return false;
        };
        let Some(definition) = self.catalog.get(entity.definition) else {
            return false;
        };
        if !definition.strategic.can_cloak || !entity.is_active() {
            return false;
        }
        // initCloaked requests cloak; actual cloak begins only after admission.
        self.cloak_states.get(&id).is_some_and(|c| c.cloaked)
    }

    pub fn set_cloak(&mut self, id: EntityId, wanted: bool) -> Result<(), WorldError> {
        let entity = self.entities.get(id).ok_or(WorldError::InvalidEntity(id))?;
        let definition = self.catalog.get(entity.definition).expect("catalog ID");
        if !definition.strategic.can_cloak {
            return Err(WorldError::InvalidEntity(id));
        }
        if self
            .cloak_states
            .get(&id)
            .is_some_and(|state| state.wanted == wanted)
        {
            return Ok(());
        }
        let state = self.cloak_states.entry(id).or_insert(CloakState {
            wanted: definition.strategic.start_cloaked,
            cloaked: false,
            blocked_until: 0.0,
            next_check: self.sensor_seconds,
            last_position: entity.position,
        });
        state.wanted = wanted;
        if !wanted {
            state.cloaked = false;
        }
        state.next_check = self.sensor_seconds;
        self.advance_cloaks(0.0);
        Ok(())
    }

    fn advance_cloaks(&mut self, seconds: f64) {
        if seconds > 0.5 {
            let mut remaining = seconds;
            while remaining > 0.5 {
                self.advance_cloaks(0.5);
                remaining -= 0.5;
            }
            if remaining > 0.0 {
                self.advance_cloaks(remaining);
            }
            return;
        }
        self.sensor_seconds += seconds;
        self.cloak_states
            .retain(|id, _| self.entities.contains(*id));
        let units = self
            .entities
            .iter()
            .filter_map(|(id, e)| {
                let d = self.catalog.get(e.definition)?;
                d.strategic
                    .can_cloak
                    .then_some((id, e.position, e.owner, e.is_active(), d))
            })
            .collect::<Vec<_>>();
        for (id, position, side, active, definition) in units {
            let d = &definition.strategic;
            let state = self.cloak_states.entry(id).or_insert(CloakState {
                wanted: d.start_cloaked,
                cloaked: false,
                blocked_until: 0.0,
                next_check: 0.0,
                last_position: position,
            });
            if self.sensor_seconds + 1e-9 < state.next_check {
                state.last_position = position;
                continue;
            }
            let dt = seconds.max(1.0 / 30.0);
            let speed = self
                .combat
                .as_ref()
                .and_then(|c| c.stepper.combatant(id.to_raw()))
                .map_or_else(
                    || position.distance(state.last_position) / dt as f32,
                    |u| u.speed.abs(),
                );
            let stunned = self
                .combat
                .as_ref()
                .and_then(|c| c.stepper.combatant(id.to_raw()))
                .is_some_and(|u| u.stunned);
            let proximity = d.min_cloak_distance > 0.0
                && self.entities.iter().any(|(other_id, e)| {
                    if other_id == id
                        || e.owner >= 2
                        || e.owner == side
                        || e.role == EntityRole::Feature
                    {
                        return false;
                    }
                    let mut distance = position.distance_squared(e.position);
                    if d.decloak_spherical {
                        let target_def = self.catalog.get(e.definition).expect("catalog ID");
                        let y = self.map.ground_height(position.x, position.z)
                            + definition.movement.collision_mid_y();
                        let other_y = self.map.ground_height(e.position.x, e.position.z)
                            + target_def.movement.collision_mid_y();
                        distance += (y - other_y).powi(2);
                    }
                    let radius = d.min_cloak_distance
                        + if d.decloak_spherical {
                            self.catalog
                                .get(e.definition)
                                .expect("catalog ID")
                                .movement
                                .radius
                                .max(0.0)
                        } else {
                            0.0
                        };
                    distance <= radius * radius
                });
            let allowed = state.wanted
                && active
                && !stunned
                && !proximity
                && self.sensor_seconds + 1e-9 >= state.blocked_until
                && side < 2;
            let cost = if speed > 0.2_f32.sqrt() * crate::model::SIM_HZ {
                d.cloak_cost_moving.unwrap_or(d.cloak_cost)
            } else {
                d.cloak_cost
            };
            let charge = f64::from(cost.max(0.0)) * 0.5;
            state.cloaked =
                allowed && self.teams[side.min(1) as usize].economy.energy.stored + 1e-12 >= charge;
            if state.cloaked {
                self.teams[side as usize].economy.energy.spend(charge);
            }
            // BAR's cloak gadget charges each successful half-second check.
            // Proximity and insufficient energy do not add a firing cooldown.
            state.next_check = self.sensor_seconds + 0.5;
            state.last_position = position;
        }
    }

    fn update_sensor_memory(&self) {
        if let ObservationPolicy::Filtered {
            line_of_sight_range,
            radar_range,
            radar_position_resolution,
            remember_last_seen,
        } = self.observation_policy
        {
            for side in 0..2 {
                self.filtered_entities(
                    side,
                    line_of_sight_range,
                    radar_range,
                    radar_position_resolution,
                    remember_last_seen,
                );
            }
        }
    }

    /// Selects the policy used by [`Self::observation`] and attack-target
    /// validation. Omniscient remains the default for existing callers.
    pub fn set_observation_policy(&mut self, policy: ObservationPolicy) {
        if self.observation_policy != policy {
            self.observation_memory
                .get_mut()
                .iter_mut()
                .for_each(BTreeMap::clear);
            *self.observation_memory_policy.get_mut() = [None, None];
        }
        self.observation_policy = policy;
    }

    pub fn observation(&self, side: u8) -> Result<StrategyObservation, WorldError> {
        self.observation_with_policy(side, self.observation_policy)
    }

    /// Produces one observation under an explicit policy without changing the
    /// world's configured command-validation policy.
    pub fn observation_with_policy(
        &self,
        side: u8,
        policy: ObservationPolicy,
    ) -> Result<StrategyObservation, WorldError> {
        self.validate_team(side)?;
        let team = &self.teams[side as usize];
        let entities = match policy {
            ObservationPolicy::Omniscient => self
                .entities
                .iter()
                .map(|(id, entity)| {
                    self.exact_entity_observation(
                        id,
                        entity,
                        if entity.owner == side {
                            ObservationVisibility::Own
                        } else {
                            ObservationVisibility::LineOfSight
                        },
                    )
                })
                .collect(),
            ObservationPolicy::Filtered {
                line_of_sight_range,
                radar_range,
                radar_position_resolution,
                remember_last_seen,
            } => self.filtered_entities(
                side,
                line_of_sight_range,
                radar_range,
                radar_position_resolution,
                remember_last_seen,
            ),
        };
        let identified_entities = match policy {
            ObservationPolicy::Omniscient => None,
            ObservationPolicy::Filtered { .. } => Some(
                entities
                    .iter()
                    .filter(|entity| {
                        matches!(
                            entity.visibility,
                            ObservationVisibility::Own | ObservationVisibility::LineOfSight
                        )
                    })
                    .map(|entity| entity.id)
                    .collect::<BTreeSet<_>>(),
            ),
        };
        // A live viewer needs the target that combat is actually attempting
        // to engage, not only explicit player orders. In-range defenders use
        // transient local targets, so omitting these made turret and idle-unit
        // fire appear to stop even while the tactical stepper kept working.
        let mut intent_positions = BTreeMap::new();
        for (&source, target) in &self.attack_orders {
            let target = match target {
                AttackTarget::Ground { position } => Some((*position, None)),
                AttackTarget::Entity { entity } => entities
                    .iter()
                    .find(|seen| seen.id == *entity)
                    .map(|seen| (seen.position, Some(*entity))),
            };
            if let Some(target) = target {
                intent_positions.insert(source, target);
            }
        }
        if let Some(combat) = &self.combat {
            for (&source, &target) in &combat.targets {
                if let Some(target) = entities.iter().find(|seen| seen.id == target) {
                    intent_positions.insert(source, (target.position, Some(target.id)));
                }
            }
        }
        let fire_intents = intent_positions
            .into_iter()
            .filter_map(|(source, (target_position, target_id))| {
                let source_entity = self.entities.get(source)?;
                // An enemy's active weapon intent is a visible combat cue
                // only when its source is in LOS. Radar and remembered
                // contacts remain opaque.
                let source_disclosed = source_entity.owner == side
                    || identified_entities
                        .as_ref()
                        .is_none_or(|identified| identified.contains(&source));
                let blocked_by_friendly_wall = target_id
                    .filter(|id| {
                        entities.iter().any(|e| {
                            e.id == *id
                                && matches!(
                                    e.visibility,
                                    ObservationVisibility::Own | ObservationVisibility::LineOfSight
                                )
                        })
                    })
                    .and_then(|target_id| {
                        self.entities
                            .get(target_id)
                            .map(|target| (target_id, target))
                    })
                    .is_some_and(|(_target_id, target)| {
                        self.catalog
                            .get(source_entity.definition)
                            .is_some_and(|definition| {
                                definition.weapons.iter().any(|weapon| {
                                    weapon.avoid_friendly
                                        && !weapon.manual_fire
                                        && source_entity.position.distance_squared(target.position)
                                            <= weapon.range.max(0.0).powi(2)
                                        && self.weapon_line_blocked_by_owned_wall(
                                            source,
                                            source_entity,
                                            target,
                                            definition,
                                            weapon,
                                        )
                                })
                            })
                    });
                source_disclosed.then_some(ObservedFireIntent {
                    source,
                    target_position,
                    blocked_by_friendly_wall,
                })
            })
            .collect();
        let observed_site_owner = |occupant_id: OccupantId| {
            let owner = self.occupancy.occupant_owner(occupant_id)?;
            if identified_entities.as_ref().is_none_or(|identified| {
                owner == side || identified.contains(&EntityId::from_raw(occupant_id.0))
            }) {
                Some(owner)
            } else {
                None
            }
        };
        let mut resource_sites =
            Vec::with_capacity(self.map.metal_nodes.len() + self.map.geothermal_nodes.len());
        for (index, node) in self.map.metal_nodes.iter().enumerate() {
            resource_sites.push(ObservedResourceSite {
                id: index as u32,
                position: Vec2::new(node.x, node.z),
                metal: node.yield_value,
                geothermal: false,
                occupied_by: self
                    .occupancy
                    .is_metal_spot_reserved(index)
                    .and_then(observed_site_owner),
            });
        }
        let metal_count = self.map.metal_nodes.len() as u32;
        for (index, node) in self.map.geothermal_nodes.iter().enumerate() {
            resource_sites.push(ObservedResourceSite {
                id: metal_count + index as u32,
                position: Vec2::new(node.x, node.z),
                metal: 0.0,
                geothermal: true,
                occupied_by: self
                    .occupancy
                    .is_geothermal_reserved(index)
                    .and_then(observed_site_owner),
            });
        }
        Ok(StrategyObservation {
            tick: self.tick,
            side,
            map_size: Vec2::new(self.map.width, self.map.depth),
            metal: pool_observation(&team.economy.metal),
            energy: pool_observation(&team.economy.energy),
            entities,
            fire_intents,
            resource_sites,
        })
    }

    fn exact_entity_observation(
        &self,
        id: EntityId,
        entity: &WorldEntity,
        visibility: ObservationVisibility,
    ) -> ObservedEntity {
        let definition = self.catalog.get(entity.definition).expect("catalog ID");
        ObservedEntity {
            id,
            side: entity.owner,
            definition: Some(definition.name.clone()),
            kind: self.strategy_entity_kind(id, entity),
            position: entity.position,
            health_fraction: Some(
                (entity.health as f32 / definition.health.max(f32::EPSILON)).clamp(0.0, 1.0),
            ),
            visibility,
            age_seconds: 0.0,
            position_error: 0.0,
        }
    }

    fn sensor_snapshot(
        &self,
        id: EntityId,
        entity: &WorldEntity,
        los: f32,
        radar: f32,
    ) -> SensorUnit {
        let definition = self.catalog.get(entity.definition).expect("catalog ID");
        let mut unit = SensorUnit::from_definition(
            id.to_raw(),
            entity.owner,
            entity.position,
            self.map.ground_height(entity.position.x, entity.position.z) - self.map.water_level,
            definition,
        );
        unit.active = entity.role != EntityRole::Feature && entity.is_active();
        unit.being_built = !entity.is_active();
        let stunned = self
            .combat
            .as_ref()
            .and_then(|c| c.stepper.combatant(id.to_raw()))
            .is_some_and(|u| u.stunned);
        // Recoil deactivation/stun disables electronic coverage; an energy
        // shortage alone does not implicitly toggle a radar or jammer off.
        unit.enabled = entity.enabled && !stunned;
        if definition.strategic.sight_distance.is_none() {
            unit.sight = los;
            unit.air_sight = definition.strategic.air_sight_distance.unwrap_or(los * 1.5);
        }
        if definition.strategic.radar_radius.is_none() {
            unit.radar = radar;
        }
        unit.cloaked = self.entity_is_cloaked(id);
        unit
    }

    fn sensor_snapshots(&self, los: f32, radar: f32) -> Vec<SensorUnit> {
        self.entities
            .iter()
            .map(|(id, e)| self.sensor_snapshot(id, e, los, radar))
            .collect()
    }

    fn sensor_terrain_clear(&self, from: Vec2, from_y: f32, to: Vec2, to_y: f32) -> bool {
        sampled_terrain_line_of_sight(
            &self.map,
            from,
            from_y + self.map.water_level,
            to,
            to_y + self.map.water_level,
        )
    }

    fn filtered_entities(
        &self,
        side: u8,
        line_of_sight_range: f32,
        radar_range: f32,
        radar_position_resolution: f32,
        remember_last_seen: bool,
    ) -> Vec<ObservedEntity> {
        self.observation_memory_policy.borrow_mut()[side as usize] =
            Some(ObservationPolicy::Filtered {
                line_of_sight_range,
                radar_range,
                radar_position_resolution,
                remember_last_seen,
            });
        let units = self.sensor_snapshots(line_of_sight_range, radar_range);
        let clear = |a, b, c, d| self.sensor_terrain_clear(a, b, c, d);
        let error = sensors::radar_error_size(side, &units, radar_position_resolution);
        let frame = (self.sensor_seconds * f64::from(crate::model::SIM_HZ)).round() as u64;
        let mut all_memory = self.observation_memory.borrow_mut();
        let memory = &mut all_memory[side as usize];
        let mut entities = Vec::new();
        let mut current_ids = BTreeSet::new();
        for unit in &units {
            let id = EntityId::from_raw(unit.key);
            let entity = self.entities.get(id).expect("snapshot entity");
            if entity.role == EntityRole::Feature {
                // BAR modrules featureVisibility=3: rocks and wrecks are
                // globally disclosed, independently of unit sensor maps.
                memory.remove(&id);
                entities.push(self.exact_entity_observation(
                    id,
                    entity,
                    ObservationVisibility::LineOfSight,
                ));
                continue;
            }
            if entity.owner == side {
                memory.remove(&id);
                entities.push(self.exact_entity_observation(
                    id,
                    entity,
                    ObservationVisibility::Own,
                ));
                continue;
            }
            let detection = sensors::detect(side, unit, &units, clear);
            let Some(detection) = detection else {
                continue;
            };
            let previous = memory.get(&id);
            let observed = if detection == Detection::Los {
                self.exact_entity_observation(id, entity, ObservationVisibility::LineOfSight)
            } else {
                let known = previous
                    .and_then(|m| m.ghost.as_ref().or_else(|| m.continuous.then_some(&m.last)));
                ObservedEntity {
                    id,
                    side: entity.owner,
                    definition: known.and_then(|m| m.definition.clone()),
                    kind: known.map_or(StrategyEntityKind::Unknown, |m| m.kind),
                    position: previous.and_then(|m| m.ghost.as_ref()).map_or_else(
                        || sensors::radar_position(unit.key, frame, entity.position, error),
                        |g| g.position,
                    ),
                    health_fraction: None,
                    visibility: if detection == Detection::Sonar {
                        ObservationVisibility::Sonar
                    } else {
                        ObservationVisibility::Radar
                    },
                    age_seconds: 0.0,
                    // A ghost pins the position of a building exactly.
                    position_error: if previous.is_some_and(|m| m.ghost.is_some()) {
                        0.0
                    } else {
                        error
                    },
                }
            };
            let mut ghost = previous.and_then(|m| m.ghost.clone());
            if detection == Detection::Los && unit.leaves_ghost {
                let mut g = observed.clone();
                g.visibility = ObservationVisibility::Ghost;
                g.health_fraction = None;
                ghost = Some(g);
            }
            memory.insert(
                id,
                ContactMemory {
                    last: observed.clone(),
                    ghost,
                    continuous: true,
                    uncertainty: if detection == Detection::Los {
                        0.0
                    } else {
                        error
                    },
                    seen_at: self.sensor_seconds,
                },
            );
            current_ids.insert(id);
            entities.push(observed);
        }
        memory.retain(|id, m| {
            if current_ids.contains(id) {
                return true;
            }
            m.continuous = false;
            if !remember_last_seen && m.ghost.is_none() {
                return false;
            }
            let remembered = m.ghost.as_ref().unwrap_or(&m.last);
            // Recoil live ghosts survive cloak even in visible terrain. Dead
            // ghosts are removed only by a position-LOS recheck, never radar.
            let live_ghost = m.ghost.is_some() && self.entities.contains(*id);
            let known = remembered
                .definition
                .as_deref()
                .and_then(|name| self.catalog.get_by_name(name).map(|(_, d)| d));
            let fallback = self
                .catalog
                .definitions()
                .first()
                .expect("remembered catalog");
            let mut target = SensorUnit::from_definition(
                id.to_raw(),
                remembered.side,
                remembered.position,
                self.map
                    .ground_height(remembered.position.x, remembered.position.z)
                    - self.map.water_level,
                known.unwrap_or(fallback),
            );
            // Geometry follows disclosed knowledge, never the live hidden
            // unit's mode. Anonymous stale contacts require ground LOS.
            target.use_air_los = known.is_some_and(|d| d.strategic.use_air_los);
            let uncertainty = if m.ghost.is_some() {
                0.0
            } else {
                m.uncertainty
            };
            live_ghost
                || !units.iter().any(|u| {
                    u.side == side
                        && u.position.distance(target.position) + uncertainty <= u.sight
                        && sensors::visual_coverage(u, &target, &clear)
                })
        });
        {
            let fastest = self.fastest_mobile_speed();
            let diagonal = (self.map.width as f32).hypot(self.map.depth as f32);
            entities.extend(
                memory
                    .iter()
                    .filter(|(id, m)| {
                        !current_ids.contains(id) && (remember_last_seen || m.ghost.is_some())
                    })
                    .map(|(_, m)| {
                        let age = (self.sensor_seconds - m.seen_at).max(0.0) as f32;
                        if let Some(ghost) = &m.ghost {
                            // A building cannot have moved: only its existence
                            // is in doubt, which the Ghost visibility says.
                            let mut ghost = ghost.clone();
                            ghost.age_seconds = age;
                            ghost.position_error = 0.0;
                            ghost
                        } else {
                            let mut last = m.last.clone();
                            last.visibility = ObservationVisibility::Remembered;
                            last.age_seconds = age;
                            let speed = self.contact_speed_bound(&last, fastest);
                            last.position_error = (m.uncertainty + speed * age).min(diagonal);
                            last
                        }
                    }),
            );
        }
        entities.sort_unstable_by_key(|e| e.id);
        entities
    }

    /// The fastest any mobile unit in the catalog can move: the worst case
    /// for a contact whose identity was never disclosed.
    fn fastest_mobile_speed(&self) -> f32 {
        self.catalog
            .definitions()
            .iter()
            .filter(|definition| definition.strategic.is_mobile)
            .map(|definition| definition.movement.max_speed)
            .fold(0.0, f32::max)
    }

    /// Upper bound on how fast a remembered contact can have moved, from only
    /// what has been disclosed about it.
    fn contact_speed_bound(&self, contact: &ObservedEntity, fastest: f32) -> f32 {
        match contact.kind {
            StrategyEntityKind::Mobile => contact
                .definition
                .as_deref()
                .and_then(|name| self.catalog.get_by_name(name))
                .map_or(fastest, |(_, definition)| definition.movement.max_speed),
            StrategyEntityKind::Unknown => fastest,
            StrategyEntityKind::Structure
            | StrategyEntityKind::Nanoframe
            | StrategyEntityKind::Wall
            | StrategyEntityKind::Factory
            | StrategyEntityKind::Feature => 0.0,
        }
    }

    pub fn entity_sensor_ranges(
        &self,
        entity_id: EntityId,
        entity: &WorldEntity,
    ) -> (f32, f32, f32) {
        let los = match self.observation_policy {
            ObservationPolicy::Filtered {
                line_of_sight_range,
                ..
            } => line_of_sight_range,
            _ => 0.0,
        };
        let unit = self.sensor_snapshot(entity_id, entity, los, 0.0);
        if !unit.active || unit.being_built {
            return (0.0, 0.0, 0.0);
        }
        (
            unit.sight,
            if unit.enabled { unit.radar } else { 0.0 },
            if unit.enabled { unit.jammer } else { 0.0 },
        )
    }

    pub fn target_is_jammed(&self, target: &WorldEntity) -> bool {
        self.position_is_jammed(target.owner, target.position)
    }

    pub fn position_is_jammed(&self, side: u8, position: Vec2) -> bool {
        let units = self.sensor_snapshots(0.0, 0.0);
        units.iter().any(|u| {
            u.side == side
                && u.active
                && u.enabled
                && !u.being_built
                && u.jammer > 0.0
                && u.position.distance_squared(position) <= u.jammer * u.jammer
        })
    }

    fn visibility_from_sensors(
        &self,
        _sensors: &[(EntityId, &WorldEntity)],
        target_id: EntityId,
        target: &WorldEntity,
        los_range_squared: f32,
        radar_range_squared: f32,
    ) -> Option<ObservationVisibility> {
        let units = self.sensor_snapshots(los_range_squared.sqrt(), radar_range_squared.sqrt());
        let unit = self.sensor_snapshot(
            target_id,
            target,
            los_range_squared.sqrt(),
            radar_range_squared.sqrt(),
        );
        let side = _sensors.first().map(|(_, e)| e.owner)?;
        sensors::detect(side, &unit, &units, |a, b, c, d| {
            self.sensor_terrain_clear(a, b, c, d)
        })
        .map(|v| match v {
            Detection::Los => ObservationVisibility::LineOfSight,
            Detection::Radar => ObservationVisibility::Radar,
            Detection::Sonar => ObservationVisibility::Sonar,
        })
    }

    /// Produces the fixed strategic score vector directly from authoritative
    /// world state. Scenario-specific weighting remains in `CompositeWeights`.
    pub fn strategic_metrics(&self, side: u8) -> Result<StrategicMetricVector, WorldError> {
        self.validate_team(side)?;
        let team = &self.teams[side as usize];
        let mut metrics = StrategicMetricVector::default();
        for (_, entity) in self.entities.iter() {
            if entity.owner != side || entity.role == EntityRole::Feature {
                continue;
            }
            let definition = self.catalog.get(entity.definition).expect("catalog ID");
            let health_fraction = if definition.health > f32::EPSILON {
                (entity.health / definition.health as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let value = definition.resource_value() as f64 * health_fraction;
            let strategic = &definition.strategic;
            if strategic.is_factory {
                metrics.add(StrategicMetric::FactoryValue, value);
            } else if strategic.is_builder {
                metrics.add(StrategicMetric::BuilderValue, value);
            } else if strategic.is_mobile {
                metrics.add(StrategicMetric::ArmyValue, value);
            }
            if strategic.extracts_metal > 0.0
                || strategic.metal_make > 0.0
                || strategic.energy_make > 0.0
                || strategic.wind_generator > 0.0
                || strategic.tidal_generator > 0.0
                || strategic.metal_storage > 0.0
                || strategic.energy_storage > 0.0
            {
                metrics.add(StrategicMetric::EconomyValue, value);
            }
        }
        metrics.set(
            StrategicMetric::MetalGenerated,
            team.economy.metal.stats.generated,
        );
        metrics.set(
            StrategicMetric::EnergyGenerated,
            team.economy.energy.stats.generated,
        );
        metrics.set(StrategicMetric::MetalSpent, team.economy.metal.stats.spent);
        metrics.set(
            StrategicMetric::EnergySpent,
            team.economy.energy.stats.spent,
        );
        metrics.set(
            StrategicMetric::MetalOverflow,
            team.economy.metal.stats.overflow,
        );
        metrics.set(
            StrategicMetric::EnergyOverflow,
            team.economy.energy.stats.overflow,
        );
        metrics.set(
            StrategicMetric::MetalIncomeIntegral,
            team.metrics.metal_income_integral,
        );
        metrics.set(
            StrategicMetric::EnergyIncomeIntegral,
            team.metrics.energy_income_integral,
        );
        metrics.set(
            StrategicMetric::MetalStallSeconds,
            team.economy.metal.stats.stalled_demand_seconds,
        );
        metrics.set(
            StrategicMetric::EnergyStallSeconds,
            team.economy.energy.stats.stalled_demand_seconds,
        );
        metrics.set(
            StrategicMetric::BuilderIdleSeconds,
            team.metrics.idle_builder_seconds,
        );
        metrics.set(
            StrategicMetric::FactoryIdleSeconds,
            team.metrics.idle_factory_seconds,
        );
        metrics.set(
            StrategicMetric::UnitsProduced,
            team.metrics.units_produced as f64,
        );
        metrics.set(
            StrategicMetric::ProductionValue,
            team.metrics.production_value,
        );
        metrics.set(StrategicMetric::SunkCost, team.metrics.sunk_cost);
        metrics.set(
            StrategicMetric::MilestonesReached,
            team.metrics.milestones_reached as f64,
        );
        metrics.set(
            StrategicMetric::MilestoneScore,
            team.metrics.milestone_score,
        );
        metrics.set(
            StrategicMetric::SiteOccupancyIntegral,
            team.metrics.site_occupancy_integral,
        );
        metrics.set(
            StrategicMetric::MapControlIntegral,
            team.metrics.map_control_integral,
        );
        metrics.set(
            StrategicMetric::WallRouteDelaySeconds,
            team.metrics.wall_route_delay_seconds,
        );
        metrics.set(
            StrategicMetric::WallBreaches,
            team.metrics.wall_breaches as f64,
        );
        metrics.set(
            StrategicMetric::WallInterceptedDamage,
            team.metrics.wall_intercepted_damage,
        );
        metrics.set(StrategicMetric::WallLeaks, team.metrics.wall_leaks as f64);
        metrics.set(
            StrategicMetric::WallFriendlyObstructionSeconds,
            team.metrics.wall_friendly_obstruction_seconds,
        );
        metrics.set(
            StrategicMetric::WallLocalExchangeValue,
            team.metrics.wall_local_exchange_value,
        );
        Ok(metrics)
    }

    pub fn resource_ledger(&self, side: u8) -> Result<ResourceLedger, WorldError> {
        self.validate_team(side)?;
        let team = &self.teams[side as usize];
        Ok(ResourceLedger {
            starting_metal: team.starting_resources.metal,
            ending_metal: team.economy.metal.stored,
            starting_energy: team.starting_resources.energy,
            ending_energy: team.economy.energy.stored,
        })
    }

    pub fn step(&mut self) {
        self.step_seconds(DT as f64);
    }

    pub fn step_seconds(&mut self, seconds: f64) {
        let seconds = finite_seconds(seconds);
        self.events.clear();
        self.activate_due_entities();
        self.process_actions();
        self.replan_stale_movement_orders();
        self.advance_factories_pre(seconds);
        if self.combat.is_none() {
            self.advance_ordered_movement(seconds);
        }
        self.advance_builders(seconds);
        self.place_ready_projects();
        self.refresh_team_economies(seconds);
        self.advance_construction(seconds);
        self.advance_repairs_and_reclaim(seconds);
        if self.combat.is_none() {
            self.advance_cloaks(seconds);
            self.update_sensor_memory();
        }
        self.advance_combat(seconds);
        self.record_wall_leaks();
        self.finalize_construction();
        self.record_idle_time(seconds);
        self.tick = self.tick.wrapping_add(1);
        self.evaluate_milestones();
    }

    pub fn set_milestones(&mut self, milestones: Vec<MilestoneDeclaration>) {
        self.milestones = milestones;
        self.milestones_achieved = [BTreeSet::new(), BTreeSet::new()];
    }

    pub fn milestones(&self) -> &[MilestoneDeclaration] {
        &self.milestones
    }

    fn evaluate_milestones(&mut self) {
        if self.milestones.is_empty() {
            return;
        }
        for side in 0..2 {
            let mut achieved_indices = Vec::new();
            for (idx, milestone) in self.milestones.iter().enumerate() {
                if self.milestones_achieved[side].contains(&idx) {
                    continue;
                }
                let reached = match &milestone.condition {
                    MilestoneCondition::MetalProduced { threshold } => {
                        self.teams[side].economy.metal.stats.generated >= *threshold
                    }
                    MilestoneCondition::EnergyProduced { threshold } => {
                        self.teams[side].economy.energy.stats.generated >= *threshold
                    }
                    MilestoneCondition::UnitsProduced { count } => {
                        self.teams[side].metrics.units_produced >= *count
                    }
                    MilestoneCondition::UnitCompleted { definition } => {
                        self.entities.iter().any(|(_, e)| {
                            e.owner == side as u8
                                && e.is_active()
                                && self
                                    .catalog
                                    .get(e.definition)
                                    .is_some_and(|d| d.name == *definition)
                        })
                    }
                    MilestoneCondition::SitesOccupied { count } => {
                        let occupied = self
                            .map
                            .metal_nodes
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| {
                                self.occupancy
                                    .is_metal_spot_reserved(*i)
                                    .and_then(|occ| self.occupancy.occupant_owner(occ))
                                    == Some(side as u8)
                            })
                            .count();
                        occupied >= *count
                    }
                    MilestoneCondition::MetalStored { threshold } => {
                        self.teams[side].economy.metal.stored >= *threshold
                    }
                    MilestoneCondition::EnergyStored { threshold } => {
                        self.teams[side].economy.energy.stored >= *threshold
                    }
                    MilestoneCondition::ArmyValue { threshold } => {
                        let army_val: f64 = self
                            .entities
                            .iter()
                            .filter(|(_, e)| {
                                e.owner == side as u8
                                    && e.is_active()
                                    && e.role != EntityRole::Feature
                            })
                            .filter_map(|(_, e)| {
                                let def = self.catalog.get(e.definition)?;
                                def.strategic.is_mobile.then(|| def.resource_value() as f64)
                            })
                            .sum();
                        army_val >= *threshold
                    }
                    MilestoneCondition::TickReached { tick } => self.tick >= *tick,
                };
                if reached {
                    achieved_indices.push((idx, milestone.points));
                }
            }
            for (idx, points) in achieved_indices {
                self.milestones_achieved[side].insert(idx);
                self.teams[side].metrics.milestones_reached += 1;
                self.teams[side].metrics.milestone_score += points;
            }
        }
    }

    fn process_actions(&mut self) {
        let mut inbox = std::mem::take(&mut self.action_inbox);
        let mut validator = std::mem::take(&mut self.action_validator);
        let decisions = validator.validate_ready(self.tick, &mut inbox, self);
        self.action_inbox = inbox;
        self.action_validator = validator;
        for decision in &decisions {
            if decision.result.is_ok() {
                self.execute_action(&decision.queued);
            }
        }
        self.action_decisions = decisions;
    }

    fn execute_action(&mut self, queued: &QueuedAction) {
        match &queued.action {
            StrategyAction::Move {
                entities,
                destination,
                mode,
            } => {
                for entity in entities {
                    if *mode == OrderMode::Replace {
                        self.attack_orders.remove(entity);
                    }
                    self.set_movement_order(*entity, *destination, *mode);
                }
            }
            StrategyAction::Attack {
                entities,
                target,
                mode,
            } => {
                for entity in entities {
                    if *mode == OrderMode::Replace {
                        self.movement_orders.remove(entity);
                        self.movement_goals.remove(entity);
                        self.movement_route_epochs.remove(entity);
                    }
                    self.attack_orders.insert(*entity, *target);
                }
            }
            StrategyAction::Build {
                builder,
                definition,
                position,
                facing,
                mode,
            } => {
                if *mode == OrderMode::Replace {
                    self.builder_tasks.remove(builder);
                }
                if let Some(definition) = self.catalog.id(definition) {
                    let _ = self.issue_build(*builder, definition, *position, *facing, 0);
                }
            }
            StrategyAction::Assist {
                entities, target, ..
            } => {
                if let Some(project_id) = self.project_for_entity(*target) {
                    for entity in entities {
                        let _ = self.assist(*entity, project_id);
                    }
                }
            }
            StrategyAction::Repair {
                entities, target, ..
            } => {
                for entity in entities {
                    self.repair_orders.insert(*entity, *target);
                }
            }
            StrategyAction::FactoryQueue { factory, operation } => {
                self.apply_factory_queue_operation(*factory, operation);
            }
            StrategyAction::SetActivation { entities, active } => {
                for entity in entities {
                    if let Some(entity) = self.entities.get_mut(*entity) {
                        entity.enabled = *active;
                        entity.powered = *active;
                    }
                }
            }
            StrategyAction::SetCloak { entities, cloaked } => {
                for entity in entities {
                    let _ = self.set_cloak(*entity, *cloaked);
                }
            }
            StrategyAction::Reclaim {
                entities, target, ..
            } => {
                for entity in entities {
                    self.reclaim_orders.insert(*entity, *target);
                }
            }
            StrategyAction::Cancel { target } => match target {
                CancelTarget::Orders { entity } => {
                    self.movement_orders.remove(entity);
                    self.movement_goals.remove(entity);
                    self.movement_route_epochs.remove(entity);
                    self.attack_orders.remove(entity);
                    self.repair_orders.remove(entity);
                    self.reclaim_orders.remove(entity);
                    self.builder_tasks.remove(entity);
                }
                CancelTarget::Project { entity } => {
                    if let Some(project) = self.project_for_entity(*entity) {
                        self.cancel_project(project);
                    }
                }
                CancelTarget::FactoryCurrent { entity } => {
                    if let Some(runtime) = self.factories.get_mut(entity) {
                        runtime.factory.cancel_current(&mut self.construction);
                    }
                }
            },
        }
    }

    fn set_movement_order(&mut self, entity: EntityId, destination: Vec2, mode: OrderMode) {
        if mode == OrderMode::Replace {
            self.movement_goals
                .insert(entity, VecDeque::from([destination]));
            self.rebuild_movement_order(entity);
            return;
        }
        if self
            .movement_route_epochs
            .get(&entity)
            .is_some_and(|epoch| *epoch != self.occupancy.movement_epoch())
        {
            self.rebuild_movement_order(entity);
        }
        self.movement_goals
            .entry(entity)
            .or_default()
            .push_back(destination);
        self.rebuild_movement_order(entity);
    }

    fn replan_stale_movement_orders(&mut self) {
        let epoch = self.occupancy.movement_epoch();
        let stale: Vec<_> = self
            .movement_goals
            .keys()
            .copied()
            .filter(|entity| self.movement_route_epochs.get(entity) != Some(&epoch))
            .collect();
        for entity in stale {
            self.rebuild_movement_order(entity);
        }
    }

    fn rebuild_movement_order(&mut self, entity_id: EntityId) {
        let Some(entity) = self.entities.get(entity_id) else {
            self.movement_orders.remove(&entity_id);
            self.movement_goals.remove(&entity_id);
            self.movement_route_epochs.remove(&entity_id);
            return;
        };
        let Some(definition) = self.catalog.get(entity.definition) else {
            return;
        };
        let owner = entity.owner;
        let speed = definition.movement.max_speed.max(0.0);
        let radius = definition.movement.radius.max(0.0);
        let movement_class = definition.strategic.movement_class.clone();
        let mut segment_start = entity.position;
        let goals = self
            .movement_goals
            .get(&entity_id)
            .cloned()
            .unwrap_or_default();
        let mut compiled = VecDeque::new();

        for goal in goals {
            let Some(movement_class) = &movement_class else {
                compiled.push_back(goal);
                segment_start = goal;
                continue;
            };
            let request = RouteRequest::new(segment_start, goal, movement_class.clone())
                .with_radius(radius)
                .with_allow_partial(true);
            let route = self.router.find_route(&self.map, &self.occupancy, &request);
            self.record_wall_route_delay(owner, speed, &request, &route);
            let terminal = route.status;
            compiled.extend(route.waypoints);
            if matches!(terminal, RouteStatus::Complete | RouteStatus::AlreadyAtGoal) {
                segment_start = goal;
            } else {
                break;
            }
        }

        self.movement_orders.insert(entity_id, compiled);
        self.movement_route_epochs
            .insert(entity_id, self.occupancy.movement_epoch());
    }

    fn record_wall_route_delay(
        &mut self,
        mover_owner: u8,
        speed: f32,
        request: &RouteRequest,
        actual: &Route,
    ) {
        if mover_owner >= 2 || speed <= f32::EPSILON {
            return;
        }
        let wall_owner = 1 - mover_owner;
        if !self.walls.values().any(|wall| wall.owner == wall_owner) {
            return;
        }
        let mut without_enemy_walls = self.occupancy.clone();
        for (entity, wall) in &self.walls {
            if wall.owner == wall_owner {
                without_enemy_walls.remove_occupant(occupant(*entity));
            }
        }
        let mut counterfactual_router = GridRouter::new();
        let counterfactual =
            counterfactual_router.find_route(&self.map, &without_enemy_walls, request);
        let Some(counterfactual_seconds) = complete_route_seconds(&counterfactual, speed) else {
            return;
        };
        let actual_seconds =
            complete_route_seconds(actual, speed).unwrap_or(WALL_ROUTE_DELAY_CAP_SECONDS);
        self.teams[wall_owner as usize]
            .metrics
            .wall_route_delay_seconds +=
            (actual_seconds - counterfactual_seconds).clamp(0.0, WALL_ROUTE_DELAY_CAP_SECONDS);
    }

    fn apply_factory_queue_operation(
        &mut self,
        factory: EntityId,
        operation: &FactoryQueueOperation,
    ) {
        let Some(runtime) = self.factories.get_mut(&factory) else {
            return;
        };
        match operation {
            FactoryQueueOperation::Append { item } => {
                if let Some(definition) = self.catalog.get_by_name(&item.definition).map(|(_, d)| d)
                {
                    for _ in 0..item.count {
                        runtime.factory.append(BuildItem::from_unit(definition));
                    }
                }
            }
            FactoryQueueOperation::Prepend { item } => {
                if let Some(definition) = self.catalog.get_by_name(&item.definition).map(|(_, d)| d)
                {
                    for _ in 0..item.count {
                        runtime.factory.prepend(BuildItem::from_unit(definition));
                    }
                }
            }
            FactoryQueueOperation::Replace { items } => {
                let definitions: Vec<_> = items
                    .iter()
                    .flat_map(|item| {
                        self.catalog
                            .get_by_name(&item.definition)
                            .map(|(_, definition)| {
                                std::iter::repeat_n(
                                    BuildItem::from_unit(definition),
                                    item.count as usize,
                                )
                            })
                            .into_iter()
                            .flatten()
                    })
                    .collect();
                runtime.factory.replace_queue(definitions);
            }
            FactoryQueueOperation::Cancel { index } => {
                runtime.factory.cancel_queued(*index);
            }
            FactoryQueueOperation::Clear => runtime.factory.clear_queue(),
            FactoryQueueOperation::SetRepeat { enabled } => runtime.factory.repeat = *enabled,
        }
    }

    /// Stable FNV-1a digest of strategic state, including sensor knowledge that
    /// affects future orders. It excludes geometric caches and transient events.
    pub fn state_hash(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325_u64;
        hash_u64(&mut hash, self.tick);
        hash_u64(&mut hash, self.sensor_seconds.to_bits());
        for side in self.observation_memory.borrow().iter() {
            for (id, memory) in side {
                hash_u64(&mut hash, id.to_raw());
                for byte in serde_json::to_vec(memory).expect("finite sensor memory") {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
            hash_u64(&mut hash, u64::MAX);
        }
        for (id, state) in &self.cloak_states {
            hash_u64(&mut hash, id.to_raw());
            hash_u64(&mut hash, state.wanted as u64);
            hash_u64(&mut hash, state.cloaked as u64);
            hash_u64(&mut hash, state.blocked_until.to_bits());
            hash_u64(&mut hash, state.next_check.to_bits());
            hash_u64(&mut hash, state.last_position.x.to_bits() as u64);
            hash_u64(&mut hash, state.last_position.z.to_bits() as u64);
        }
        for (id, entity) in self.entities.iter() {
            hash_u64(&mut hash, id.to_raw());
            hash_u64(&mut hash, entity.definition.0 as u64);
            hash_u64(&mut hash, entity.owner as u64);
            hash_u64(&mut hash, entity.position.x.to_bits() as u64);
            hash_u64(&mut hash, entity.position.z.to_bits() as u64);
            hash_u64(&mut hash, entity.heading.to_bits() as u64);
            hash_u64(&mut hash, entity.health.to_bits());
            hash_u64(&mut hash, entity.enabled as u64);
            hash_u64(&mut hash, entity.powered as u64);
            hash_u64(&mut hash, role_discriminant(entity.role));
            hash_u64(&mut hash, lifecycle_discriminant(entity.lifecycle));
            if let Some(reclaim) = entity.reclaim {
                hash_u64(&mut hash, reclaim.progress.to_bits());
                hash_u64(&mut hash, reclaim.reclaim_time.to_bits());
                hash_u64(&mut hash, reclaim.metal.to_bits());
                hash_u64(&mut hash, reclaim.energy.to_bits());
            }
        }
        for team in &self.teams {
            hash_u64(&mut hash, team.economy.metal.stored.to_bits());
            hash_u64(&mut hash, team.economy.energy.stored.to_bits());
            hash_u64(&mut hash, team.economy.metal.stats.generated.to_bits());
            hash_u64(&mut hash, team.economy.energy.stats.generated.to_bits());
            hash_u64(&mut hash, team.metrics.wall_route_delay_seconds.to_bits());
            hash_u64(&mut hash, team.metrics.wall_breaches);
            hash_u64(&mut hash, team.metrics.wall_intercepted_damage.to_bits());
            hash_u64(&mut hash, team.metrics.wall_leaks);
            hash_u64(
                &mut hash,
                team.metrics.wall_friendly_obstruction_seconds.to_bits(),
            );
            hash_u64(&mut hash, team.metrics.wall_local_exchange_value.to_bits());
        }
        for (id, wall) in &self.walls {
            hash_u64(&mut hash, id.to_raw());
            hash_u64(&mut hash, wall.owner as u64);
            hash_u64(&mut hash, wall.position.x.to_bits() as u64);
            hash_u64(&mut hash, wall.position.z.to_bits() as u64);
            for bound in [wall.bounds.0, wall.bounds.1, wall.bounds.2, wall.bounds.3] {
                hash_u64(&mut hash, bound.to_bits() as u64);
            }
        }
        for project in self.construction.projects() {
            hash_u64(&mut hash, project.id);
            hash_u64(&mut hash, project.progress.to_bits());
            hash_u64(&mut hash, project.health.to_bits());
            hash_u64(&mut hash, project.state as u64);
        }
        if let Some(combat) = &self.combat {
            hash_u64(&mut hash, combat.stepper.deterministic_state_hash());
            hash_u64(&mut hash, combat.fractional_ticks.to_bits());
        }
        hash
    }

    fn definition(&self, id: UnitDefId) -> Result<&UnitDef, WorldError> {
        self.catalog
            .get(id)
            .ok_or_else(|| WorldError::UnknownDefinition(format!("definition #{}", id.0)))
    }

    fn validate_team(&self, owner: u8) -> Result<(), WorldError> {
        if owner < 2 {
            Ok(())
        } else {
            Err(WorldError::InvalidTeam(owner))
        }
    }

    fn allocate_project_id(&mut self) -> ProjectId {
        let id = self.next_project_id;
        self.next_project_id = self.next_project_id.wrapping_add(1).max(1);
        id
    }

    fn route_task(
        &mut self,
        builder: EntityId,
        project_id: ProjectId,
        destination: Vec2,
        final_state: BuilderTaskState,
    ) -> Result<BuilderTask, WorldError> {
        let entity = self
            .entities
            .get(builder)
            .ok_or(WorldError::InvalidEntity(builder))?;
        let definition = self.definition(entity.definition)?;
        let owner = entity.owner;
        let start = entity.position;
        let speed = definition.movement.max_speed.max(0.0);
        let radius = definition.movement.radius;
        let movement_class = definition.strategic.movement_class.clone();
        let mut waypoints = VecDeque::new();
        if let Some(movement_class) = movement_class {
            let request = RouteRequest::new(start, destination, movement_class)
                .with_radius(radius)
                .with_allow_partial(true);
            let route = self.router.find_route(&self.map, &self.occupancy, &request);
            self.record_wall_route_delay(owner, speed, &request, &route);
            if route.status != RouteStatus::Unreachable {
                waypoints.extend(route.waypoints);
            }
        }
        if waypoints.is_empty() {
            waypoints.push_back(destination);
        }
        Ok(BuilderTask {
            project_id,
            state: if start.distance_squared(destination) <= 1.0 {
                final_state
            } else {
                BuilderTaskState::Travelling
            },
            waypoints,
            route_epoch: self.occupancy.movement_epoch(),
        })
    }

    fn project_for_entity(&self, entity: EntityId) -> Option<ProjectId> {
        self.projects.iter().find_map(|(project_id, runtime)| {
            (runtime.entity == Some(entity)).then_some(*project_id)
        })
    }

    fn strategy_entity_kind(&self, _id: EntityId, entity: &WorldEntity) -> StrategyEntityKind {
        match entity.role {
            EntityRole::Mobile => StrategyEntityKind::Mobile,
            EntityRole::Structure => StrategyEntityKind::Structure,
            EntityRole::Factory => StrategyEntityKind::Factory,
            EntityRole::Feature => StrategyEntityKind::Feature,
            EntityRole::Nanoframe { project_id } => {
                if self
                    .projects
                    .get(&project_id)
                    .is_some_and(|project| project.is_wall)
                {
                    StrategyEntityKind::Wall
                } else {
                    StrategyEntityKind::Nanoframe
                }
            }
        }
    }

    fn advance_ordered_movement(&mut self, seconds: f64) {
        let ids: Vec<_> = self.movement_orders.keys().copied().collect();
        for id in ids {
            if self.builder_tasks.contains_key(&id) {
                continue;
            }
            let Some(entity) = self.entities.get(id) else {
                self.movement_orders.remove(&id);
                continue;
            };
            if !entity.is_active() {
                continue;
            }
            let definition = self.catalog.get(entity.definition).expect("catalog ID");
            let mut remaining = definition.movement.max_speed.max(0.0) * seconds as f32;
            let mut position = entity.position;
            if let Some(orders) = self.movement_orders.get_mut(&id) {
                while remaining > 0.0 {
                    let Some(destination) = orders.front().copied() else {
                        break;
                    };
                    let delta = destination - position;
                    let distance = delta.length();
                    if distance <= remaining + 1e-5 {
                        position = destination;
                        remaining -= distance;
                        orders.pop_front();
                    } else {
                        position = position + delta * (remaining / distance.max(1e-6));
                        break;
                    }
                }
            }
            if let Some(entity) = self.entities.get_mut(id) {
                if entity.position.distance_squared(position) > f32::EPSILON {
                    entity.heading = (position - entity.position).heading();
                }
                entity.position = position;
            }
            let goals_finished = self.movement_goals.get_mut(&id).is_some_and(|goals| {
                while goals
                    .front()
                    .is_some_and(|goal| position.distance_squared(*goal) < 0.25)
                {
                    goals.pop_front();
                }
                goals.is_empty()
            });
            if goals_finished {
                self.movement_goals.remove(&id);
                self.movement_route_epochs.remove(&id);
            }
            if self
                .movement_orders
                .get(&id)
                .is_some_and(VecDeque::is_empty)
            {
                self.movement_orders.remove(&id);
            }
        }
    }

    fn advance_builders(&mut self, seconds: f64) {
        let builders: Vec<_> = self.builder_tasks.keys().copied().collect();
        for builder in builders {
            let Some(entity) = self.entities.get(builder) else {
                self.builder_tasks.remove(&builder);
                continue;
            };
            if !entity.is_active() {
                continue;
            }
            let Some(definition) = self.catalog.get(entity.definition) else {
                continue;
            };
            let speed = definition.movement.max_speed.max(0.0) * seconds as f32;
            let build_distance = definition.strategic.build_distance.max(0.0);
            let Some(project) = self
                .builder_tasks
                .get(&builder)
                .and_then(|task| self.projects.get(&task.project_id))
            else {
                continue;
            };
            let target = project.position;
            let target_def = self.catalog.get(project.definition).expect("catalog ID");
            let footprint_reach = target_def.strategic.footprint[0]
                .max(target_def.strategic.footprint[1]) as f32
                * 8.0;
            let within_range = entity.position.distance(target) <= build_distance + footprint_reach;
            if within_range {
                if let Some(task) = self.builder_tasks.get_mut(&builder) {
                    task.state = if project.entity.is_some() {
                        BuilderTaskState::Constructing
                    } else {
                        BuilderTaskState::WaitingForSite
                    };
                }
                continue;
            }
            if speed <= 0.0 {
                continue;
            }
            let mut remaining = speed;
            let mut position = entity.position;
            if let Some(task) = self.builder_tasks.get_mut(&builder) {
                task.state = BuilderTaskState::Travelling;
                while remaining > 0.0 {
                    let Some(waypoint) = task.waypoints.front().copied() else {
                        break;
                    };
                    let delta = waypoint - position;
                    let distance = delta.length();
                    if distance <= remaining + 1e-5 {
                        position = waypoint;
                        remaining -= distance;
                        task.waypoints.pop_front();
                    } else {
                        position = position + delta * (remaining / distance.max(1e-6));
                        break;
                    }
                }
            }
            if let Some(entity) = self.entities.get_mut(builder) {
                if entity.position.distance_squared(position) > f32::EPSILON {
                    entity.heading = (position - entity.position).heading();
                }
                entity.position = position;
            }
        }
    }

    fn place_ready_projects(&mut self) {
        let ready: Vec<_> = self
            .builder_tasks
            .iter()
            .filter_map(|(builder, task)| {
                (task.state == BuilderTaskState::WaitingForSite)
                    .then_some((*builder, task.project_id))
            })
            .collect();
        for (builder, project_id) in ready {
            if let Err(error) = self.place_project(builder, project_id) {
                self.events.push(WorldEvent::PlacementRejected {
                    project_id,
                    reason: error.to_string(),
                });
            }
        }
    }

    fn place_project(
        &mut self,
        builder: EntityId,
        project_id: ProjectId,
    ) -> Result<(), WorldError> {
        let runtime = self
            .projects
            .get(&project_id)
            .cloned()
            .ok_or(WorldError::ProjectUnavailable(project_id))?;
        if runtime.entity.is_some() {
            return Ok(());
        }
        let definition = self.definition(runtime.definition)?.clone();
        let builder_entity = self
            .entities
            .get(builder)
            .ok_or(WorldError::InvalidEntity(builder))?;
        let builder_definition = self.definition(builder_entity.definition)?;
        let builder_position = builder_entity.position;
        let builder_distance = builder_definition.strategic.build_distance;
        let entity = self.entities.insert(WorldEntity {
            definition: runtime.definition,
            owner: runtime.owner,
            position: runtime.position,
            heading: runtime.facing.heading(),
            health: 0.0,
            enabled: false,
            powered: false,
            role: EntityRole::Nanoframe { project_id },
            lifecycle: EntityLifecycle::UnderConstruction,
            reclaim: None,
        });
        let mut placement = None;
        if definition.strategic.is_building {
            let mut request = PlacementRequest::from_unit_def(
                occupant(entity),
                &definition,
                runtime.position,
                runtime.facing,
            );
            request.owner = Some(runtime.owner);
            request.builder = Some(BuilderContext {
                position: builder_position,
                build_distance: builder_distance,
            });
            let layers = CellLayers {
                blocks_movement: definition.movement.blocks_movement,
                fire_height: definition.movement.blocks_fire.then_some(0.0),
                construction_reservation: true,
                factory_yard: false,
            };
            let resolution = match validate_placement(&self.map, &self.occupancy, &request, layers)
            {
                Ok(resolution) => resolution,
                Err(error) => {
                    self.entities.remove(entity);
                    return Err(WorldError::Placement(error));
                }
            };
            if !execute_placement(&mut self.occupancy, &resolution) {
                self.entities.remove(entity);
                return Err(WorldError::Occupancy("failed to register nanoframe".into()));
            }
            if let Some(entity_state) = self.entities.get_mut(entity) {
                entity_state.position = resolution.snapped_position;
            }
            placement = Some(resolution);
        }
        let spec = if runtime.is_wall {
            ProjectSpec::from_wall_unit(&definition)
        } else {
            ProjectSpec::from_unit(&definition)
        };
        self.construction.insert(ConstructionProject::nanoframe(
            project_id,
            spec,
            runtime.priority,
        ));
        if runtime.is_wall {
            let radius = definition.movement.radius.max(0.0);
            let position = self.entities.get(entity).expect("new wall entity").position;
            let bounds = placement.as_ref().map_or(
                (
                    position.x - radius,
                    position.z - radius,
                    position.x + radius,
                    position.z + radius,
                ),
                |resolution| resolution.bounds,
            );
            self.walls.insert(
                entity,
                WallRecord {
                    owner: runtime.owner,
                    position,
                    bounds,
                },
            );
        }
        if let Some(project) = self.projects.get_mut(&project_id) {
            project.entity = Some(entity);
            project.placement = placement;
        }
        if let Some(task) = self.builder_tasks.get_mut(&builder) {
            task.state = BuilderTaskState::Constructing;
        }
        self.events
            .push(WorldEvent::ProjectPlaced { project_id, entity });
        Ok(())
    }

    fn activate_due_entities(&mut self) {
        let due: Vec<_> = self
            .entities
            .iter()
            .filter_map(|(id, entity)| match entity.lifecycle {
                EntityLifecycle::PendingActivation { tick } if tick <= self.tick => Some(id),
                _ => None,
            })
            .collect();
        for entity_id in due {
            let completed_project = self.project_for_entity(entity_id);
            let facing = completed_project
                .and_then(|project_id| self.projects.get(&project_id))
                .map_or(BuildFacing::South, |project| project.facing);
            let Some(definition_id) = self.entities.get(entity_id).map(|entity| entity.definition)
            else {
                continue;
            };
            let definition = self.catalog.get(definition_id).expect("catalog ID");
            if let Some(entity) = self.entities.get_mut(entity_id) {
                entity.lifecycle = EntityLifecycle::Active;
                entity.role = entity_role(definition);
                entity.enabled = definition.strategic.activate_when_built;
                entity.powered = entity.enabled;
                entity.health = definition.health.max(0.0) as f64;
            }
            if definition.strategic.is_factory {
                self.factories
                    .entry(entity_id)
                    .or_insert_with(|| FactoryRuntime {
                        factory: Factory::new(
                            entity_id.to_raw(),
                            definition.strategic.build_speed as f64,
                            0,
                        ),
                        facing,
                        force_exit_blocked: false,
                    });
            }
            self.teams[self.entities.get(entity_id).expect("live").owner as usize]
                .metrics
                .units_produced += 1;
            self.teams[self.entities.get(entity_id).expect("live").owner as usize]
                .metrics
                .production_value += definition.resource_value() as f64;
            self.events
                .push(WorldEvent::EntityActivated { entity: entity_id });
            if let Some(project_id) = completed_project {
                self.construction.remove(project_id);
                self.projects.remove(&project_id);
            }
        }
    }

    fn advance_factories_pre(&mut self, seconds: f64) {
        let ids: Vec<_> = self.factories.keys().copied().collect();
        for factory_id in ids {
            if self
                .entities
                .get(factory_id)
                .is_none_or(|entity| !entity.is_active() || !entity.enabled || !entity.powered)
            {
                continue;
            }
            let forced_block = self
                .factories
                .get(&factory_id)
                .is_some_and(|runtime| runtime.force_exit_blocked);
            let exit_clear = !forced_block && self.advance_factory_release(factory_id, seconds);
            let needs_project = self.factories.get(&factory_id).is_some_and(|runtime| {
                matches!(runtime.factory.state, FactoryState::Starting { .. })
            });
            let candidate = if needs_project {
                self.allocate_project_id()
            } else {
                self.next_project_id
            };
            let runtime = self.factories.get_mut(&factory_id).expect("factory exists");
            let events =
                runtime
                    .factory
                    .pre_construction(&mut self.construction, exit_clear, || candidate);
            for event in events {
                self.apply_factory_event(factory_id, event);
            }
        }
    }

    /// Advances a completed product through the factory's physical exit. A
    /// product remains an inert nanoframe until its collision circle is clear
    /// of the rotated factory footprint.
    fn advance_factory_release(&mut self, factory_id: EntityId, seconds: f64) -> bool {
        let Some(runtime) = self.factories.get(&factory_id) else {
            return false;
        };
        let project_id = match runtime.factory.state {
            FactoryState::Releasing { project_id, .. }
            | FactoryState::ExitBlocked { project_id, .. } => project_id,
            _ => return true,
        };
        let facing = runtime.facing;
        let Some(project) = self.projects.get(&project_id) else {
            return false;
        };
        let Some(product_id) = project.entity else {
            return false;
        };
        let Some(factory_entity) = self.entities.get(factory_id) else {
            return false;
        };
        let Some(product) = self.entities.get(product_id) else {
            return false;
        };
        let factory_definition = self
            .catalog
            .get(factory_entity.definition)
            .expect("catalog ID");
        let product_definition = self.catalog.get(product.definition).expect("catalog ID");
        let geometry = factory_exit_geometry(
            factory_entity.position,
            factory_definition.strategic.footprint,
            facing,
            factory_definition
                .strategic
                .build_constraints
                .yardmap
                .as_deref(),
            product_definition.movement.radius,
        );
        let position = product.position;
        let distance = position.distance(geometry.goal);
        if distance <= 1e-4 {
            return self.factory_release_position_clear(
                factory_id,
                product_id,
                position,
                product_definition.movement.radius,
                product_definition.strategic.movement_class.as_deref(),
            );
        }
        let travel = product_definition.movement.max_speed.max(0.0) * seconds as f32;
        if travel <= 0.0 {
            return false;
        }
        let proposed = if distance <= travel {
            geometry.goal
        } else {
            position + (geometry.goal - position) * (travel / distance)
        };
        if !self.factory_release_sweep_clear(
            factory_id,
            product_id,
            position,
            proposed,
            product_definition.movement.radius,
            product_definition.strategic.movement_class.as_deref(),
        ) {
            return false;
        }
        if let Some(product) = self.entities.get_mut(product_id) {
            product.position = proposed;
        }
        proposed.distance_squared(geometry.goal) <= 1e-8
    }

    fn factory_release_sweep_clear(
        &self,
        factory_id: EntityId,
        product_id: EntityId,
        start: Vec2,
        end: Vec2,
        radius: f32,
        movement_class: Option<&str>,
    ) -> bool {
        let distance = start.distance(end);
        let interval = radius
            .max(1.0)
            .min((self.occupancy.cell_size() * 0.5).max(1.0));
        let steps = (distance / interval).ceil().max(1.0) as u32;
        (1..=steps).all(|step| {
            let fraction = step as f32 / steps as f32;
            let position = start + (end - start) * fraction;
            self.factory_release_position_clear(
                factory_id,
                product_id,
                position,
                radius,
                movement_class,
            )
        })
    }

    fn factory_release_position_clear(
        &self,
        factory_id: EntityId,
        product_id: EntityId,
        position: Vec2,
        radius: f32,
        movement_class: Option<&str>,
    ) -> bool {
        let radius = radius.max(0.0);
        if position.x - radius < 0.0
            || position.z - radius < 0.0
            || position.x + radius > self.map.width
            || position.z + radius > self.map.depth
        {
            return false;
        }
        let perimeter = [
            Vec2::new(0.0, 0.0),
            Vec2::new(radius, 0.0),
            Vec2::new(-radius, 0.0),
            Vec2::new(0.0, radius),
            Vec2::new(0.0, -radius),
        ];
        if let Some(movement_class) = movement_class
            && perimeter.iter().any(|offset| {
                !self
                    .map
                    .is_passable(movement_class, position.x + offset.x, position.z + offset.z)
            })
        {
            return false;
        }

        let cell_size = self.occupancy.cell_size();
        let min_col = ((position.x - radius) / cell_size).floor().max(0.0) as u32;
        let max_col = ((position.x + radius) / cell_size)
            .floor()
            .min(self.occupancy.cols().saturating_sub(1) as f32) as u32;
        let min_row = ((position.z - radius) / cell_size).floor().max(0.0) as u32;
        let max_row = ((position.z + radius) / cell_size)
            .floor()
            .min(self.occupancy.rows().saturating_sub(1) as f32) as u32;
        for row in min_row..=max_row {
            for col in min_col..=max_col {
                if self.occupancy.cell(col, row).is_some_and(|cell| {
                    cell.contributions.iter().any(|contribution| {
                        contribution.occupant_id != occupant(factory_id)
                            && contribution.layers.blocks_movement
                    })
                }) {
                    return false;
                }
            }
        }

        for (other_id, other) in self.entities.iter() {
            if other_id == factory_id || other_id == product_id {
                continue;
            }
            let Some(definition) = self.catalog.get(other.definition) else {
                continue;
            };
            let separation = radius + definition.movement.radius.max(0.0);
            if separation > 0.0
                && position.distance_squared(other.position) < separation * separation
            {
                return false;
            }
        }
        true
    }

    fn apply_factory_event(&mut self, factory_id: EntityId, event: FactoryEvent) {
        match event {
            FactoryEvent::Starting { .. } => {}
            FactoryEvent::ProjectPlaced {
                definition,
                project_id,
            } => {
                let Some((definition_id, unit)) = self.catalog.get_by_name(&definition) else {
                    return;
                };
                let Some(factory_entity) = self.entities.get(factory_id) else {
                    return;
                };
                let factory_position = factory_entity.position;
                let owner = factory_entity.owner;
                let factory_definition = self
                    .catalog
                    .get(factory_entity.definition)
                    .expect("catalog ID");
                let facing = self
                    .factories
                    .get(&factory_id)
                    .map_or(BuildFacing::South, |runtime| runtime.facing);
                let geometry = factory_exit_geometry(
                    factory_position,
                    factory_definition.strategic.footprint,
                    facing,
                    factory_definition
                        .strategic
                        .build_constraints
                        .yardmap
                        .as_deref(),
                    unit.movement.radius,
                );
                let position = geometry.pad;
                let entity = self.entities.insert(WorldEntity {
                    definition: definition_id,
                    owner,
                    position,
                    heading: facing.heading(),
                    health: 0.0,
                    enabled: false,
                    powered: false,
                    role: EntityRole::Nanoframe { project_id },
                    lifecycle: EntityLifecycle::UnderConstruction,
                    reclaim: None,
                });
                self.projects.insert(
                    project_id,
                    ProjectRuntime {
                        definition: definition_id,
                        owner,
                        position,
                        facing,
                        is_wall: false,
                        factory: Some(factory_id),
                        entity: Some(entity),
                        placement: None,
                        priority: 0,
                    },
                );
                debug_assert_eq!(unit.name, definition);
                self.events
                    .push(WorldEvent::ProjectPlaced { project_id, entity });
            }
            FactoryEvent::ExitBecameBlocked { project_id, .. } => {
                self.events.push(WorldEvent::FactoryExitBlocked {
                    factory: factory_id,
                    project_id,
                });
            }
            FactoryEvent::Released { project_id, .. } => {
                if let Some(project) = self.projects.remove(&project_id)
                    && let Some(entity_id) = project.entity
                {
                    if let Some(entity) = self.entities.get_mut(entity_id) {
                        let definition = self.catalog.get(entity.definition).expect("catalog ID");
                        entity.lifecycle = EntityLifecycle::Active;
                        entity.role = entity_role(definition);
                        entity.enabled = definition.strategic.activate_when_built;
                        entity.powered = entity.enabled;
                        entity.health = definition.health.max(0.0) as f64;
                    }
                    self.teams[project.owner as usize].metrics.units_produced += 1;
                    self.teams[project.owner as usize].metrics.production_value +=
                        self.catalog
                            .get(project.definition)
                            .expect("catalog ID")
                            .resource_value() as f64;
                    self.events.push(WorldEvent::FactoryReleased {
                        factory: factory_id,
                        entity: entity_id,
                    });
                }
            }
        }
    }

    fn refresh_team_economies(&mut self, seconds: f64) {
        let mut incomes = [self.teams[0].base_income, self.teams[1].base_income];
        let mut capacities = [self.teams[0].base_storage, self.teams[1].base_storage];
        for (_, entity) in self.entities.iter() {
            if !entity.is_active() || !entity.enabled || !entity.powered || entity.owner >= 2 {
                continue;
            }
            let definition = self.catalog.get(entity.definition).expect("catalog ID");
            let strategic = &definition.strategic;
            let side = entity.owner as usize;
            capacities[side].metal += strategic.metal_storage as f64;
            capacities[side].energy += strategic.energy_storage as f64;
            // BAR uses negative upkeep as production for definitions such as
            // solar generators. Positive upkeep is allocated later as demand.
            incomes[side].metal +=
                strategic.metal_make as f64 + (-strategic.metal_upkeep).max(0.0) as f64;
            incomes[side].energy +=
                strategic.energy_make as f64 + (-strategic.energy_upkeep).max(0.0) as f64;
            incomes[side].energy += strategic.wind_generator as f64 * self.config.wind_output;
            incomes[side].energy += strategic.tidal_generator as f64 * self.config.tidal_output;
            if strategic.extracts_metal > 0.0 {
                incomes[side].metal += strategic.extracts_metal as f64
                    * self.map.metal_yield_at(
                        entity.position.x,
                        entity.position.z,
                        strategic.extract_range,
                    ) as f64;
            }
        }
        for side in 0..2 {
            set_pool_capacity(&mut self.teams[side].economy.metal, capacities[side].metal);
            set_pool_capacity(
                &mut self.teams[side].economy.energy,
                capacities[side].energy,
            );
            self.teams[side].economy.metal.income_per_second = incomes[side].metal.max(0.0);
            self.teams[side].economy.energy.income_per_second = incomes[side].energy.max(0.0);
            self.teams[side].metrics.metal_income_integral +=
                self.teams[side].economy.metal.income_per_second * seconds;
            self.teams[side].metrics.energy_income_integral +=
                self.teams[side].economy.energy.income_per_second * seconds;
            self.teams[side].economy.gather_income(seconds);
        }
        let total_sites = self.map.metal_nodes.len() + self.map.geothermal_nodes.len();
        if total_sites > 0 {
            for side in 0..2_u8 {
                let metal_sites = (0..self.map.metal_nodes.len())
                    .filter(|index| {
                        self.occupancy
                            .is_metal_spot_reserved(*index)
                            .and_then(|occupant| self.occupancy.occupant_owner(occupant))
                            == Some(side)
                    })
                    .count();
                let geothermal_sites = (0..self.map.geothermal_nodes.len())
                    .filter(|index| {
                        self.occupancy
                            .is_geothermal_reserved(*index)
                            .and_then(|occupant| self.occupancy.occupant_owner(occupant))
                            == Some(side)
                    })
                    .count();
                let occupied = (metal_sites + geothermal_sites) as f64;
                self.teams[side as usize].metrics.site_occupancy_integral += occupied * seconds;
                self.teams[side as usize].metrics.map_control_integral +=
                    occupied / total_sites as f64 * seconds;
            }
        }
    }

    fn advance_construction(&mut self, seconds: f64) {
        let mut contributions: [Vec<BuilderContribution>; 2] = std::array::from_fn(|_| Vec::new());
        for (builder, task) in &self.builder_tasks {
            if !matches!(
                task.state,
                BuilderTaskState::Constructing | BuilderTaskState::Assisting
            ) {
                continue;
            }
            let Some(entity) = self.entities.get(*builder) else {
                continue;
            };
            let Some(project) = self.projects.get(&task.project_id) else {
                continue;
            };
            let definition = self.catalog.get(entity.definition).expect("catalog ID");
            if !entity.is_active()
                || !entity.enabled
                || !entity.powered
                || entity.owner != project.owner
                || entity.position.distance(project.position)
                    > definition.strategic.build_distance.max(0.0)
                        + self
                            .catalog
                            .get(project.definition)
                            .expect("catalog ID")
                            .strategic
                            .footprint[0]
                            .max(
                                self.catalog
                                    .get(project.definition)
                                    .expect("catalog ID")
                                    .strategic
                                    .footprint[1],
                            ) as f32
                            * 8.0
            {
                continue;
            }
            contributions[entity.owner as usize].push(BuilderContribution {
                builder_id: builder.to_raw(),
                project_id: task.project_id,
                build_power: definition.strategic.build_speed.max(0.0) as f64,
            });
        }
        for (factory_id, runtime) in &self.factories {
            if let Some(contribution) = runtime.factory.builder_contribution()
                && let Some(entity) = self.entities.get(*factory_id)
                && entity.is_active()
                && entity.enabled
                && entity.powered
            {
                contributions[entity.owner as usize].push(contribution);
            }
        }

        for (side, side_contributions) in contributions.iter().enumerate() {
            let plan = self.construction.plan(seconds, side_contributions);
            let mut demands = plan.demands.clone();
            let mut upkeep_entities = BTreeMap::new();
            for (entity_id, entity) in self.entities.iter() {
                if entity.owner as usize != side || !entity.is_active() || !entity.enabled {
                    continue;
                }
                let definition = self.catalog.get(entity.definition).expect("catalog ID");
                let requested = Resources::new(
                    definition.strategic.metal_upkeep.max(0.0) as f64 * seconds,
                    definition.strategic.energy_upkeep.max(0.0) as f64 * seconds,
                );
                if requested.is_zero() {
                    continue;
                }
                let demand_id = UPKEEP_DEMAND_BIT | (entity_id.to_raw() & !UPKEEP_DEMAND_BIT);
                upkeep_entities.insert(demand_id, entity_id);
                demands.push(ResourceDemand {
                    id: demand_id,
                    priority: 100,
                    requested,
                });
            }
            let allocations = self.teams[side].economy.allocate(&demands, seconds);
            let supplied_upkeep: BTreeSet<_> = allocations
                .iter()
                .filter_map(|allocation| {
                    (allocation.id & UPKEEP_DEMAND_BIT != 0 && allocation.fraction >= 1.0 - 1e-12)
                        .then_some(allocation.id)
                })
                .collect();
            for (demand_id, entity_id) in upkeep_entities {
                if let Some(entity) = self.entities.get_mut(entity_id) {
                    entity.powered = supplied_upkeep.contains(&demand_id);
                }
            }
            let result = self.construction.apply_allocations(plan, &allocations);
            self.teams[side].metrics.stalled_build_power_seconds +=
                result.stalled_build_power_seconds;
            for receipt in result.receipts {
                let Some(runtime) = self.projects.get(&receipt.project_id) else {
                    continue;
                };
                let Some(entity_id) = runtime.entity else {
                    continue;
                };
                let Some(project) = self.construction.get(receipt.project_id) else {
                    continue;
                };
                if let Some(entity) = self.entities.get_mut(entity_id) {
                    entity.health = project.health;
                }
                if runtime.placement.is_some() {
                    let height = self
                        .catalog
                        .get(runtime.definition)
                        .expect("catalog ID")
                        .movement
                        .collision_height()
                        * project.progress as f32;
                    self.occupancy
                        .update_fire_height(occupant(entity_id), height);
                }
            }
        }
    }

    fn record_wall_friendly_obstruction(&mut self, seconds: f64) {
        if seconds <= 0.0 || self.walls.is_empty() {
            return;
        }
        let mut obstructed = [0_u64; 2];
        for (shooter_id, target_order) in &self.attack_orders {
            let AttackTarget::Entity { entity: target_id } = target_order else {
                continue;
            };
            let (Some(shooter), Some(target)) = (
                self.entities.get(*shooter_id),
                self.entities.get(*target_id),
            ) else {
                continue;
            };
            if shooter.owner >= 2 || shooter.owner == target.owner || !shooter.is_active() {
                continue;
            }
            let Some(definition) = self.catalog.get(shooter.definition) else {
                continue;
            };
            if definition.weapons.iter().any(|weapon| {
                weapon.avoid_friendly
                    && !weapon.manual_fire
                    && shooter.position.distance_squared(target.position)
                        <= weapon.range.max(0.0).powi(2)
                    && self.weapon_line_blocked_by_owned_wall(
                        *shooter_id,
                        shooter,
                        target,
                        definition,
                        weapon,
                    )
            }) {
                obstructed[shooter.owner as usize] += 1;
            }
        }
        for (side, count) in obstructed.into_iter().enumerate() {
            self.teams[side].metrics.wall_friendly_obstruction_seconds += count as f64 * seconds;
        }
    }

    fn weapon_line_blocked_by_owned_wall(
        &self,
        shooter_id: EntityId,
        shooter: &WorldEntity,
        target: &WorldEntity,
        shooter_definition: &UnitDef,
        weapon: &crate::model::WeaponDef,
    ) -> bool {
        let local_origin = weapon.firing_origin.unwrap_or([
            0.0,
            shooter_definition.movement.collision_mid_y(),
            0.0,
        ]);
        let forward = Vec2::from_heading(shooter.heading);
        let right = Vec2::new(-forward.z, forward.x);
        let origin = shooter.position + right * local_origin[0] + forward * local_origin[2];
        let origin_y = self
            .map
            .ground_height(shooter.position.x, shooter.position.z)
            + local_origin[1];
        let target_definition = self.catalog.get(target.definition).expect("catalog ID");
        let target_y = self.map.ground_height(target.position.x, target.position.z)
            + target_definition.movement.collision_mid_y();
        let ray = target.position - origin;
        let ray_length_squared = ray.length_squared();
        if ray_length_squared <= f32::EPSILON {
            return false;
        }

        self.walls.iter().any(|(wall_id, wall)| {
            if *wall_id == shooter_id || wall.owner != shooter.owner {
                return false;
            }
            let Some(entity) = self.entities.get(*wall_id) else {
                return false;
            };
            let definition = self.catalog.get(entity.definition).expect("catalog ID");
            if !definition.movement.blocks_fire {
                return false;
            }
            let offset = entity.position - origin;
            let along = (offset.x * ray.x + offset.z * ray.z) / ray_length_squared;
            if !(0.0 < along && along < 1.0) {
                return false;
            }
            let nearest = origin + ray * along;
            if nearest.distance_squared(entity.position)
                > definition.movement.radius.max(0.0).powi(2)
            {
                return false;
            }
            let ground = self.map.ground_height(entity.position.x, entity.position.z);
            let (bottom, full_top) = definition.movement.collision_vertical_bounds(ground);
            let scale = match entity.role {
                EntityRole::Nanoframe { project_id } => self
                    .construction
                    .get(project_id)
                    .map_or(0.0, |project| project.progress as f32),
                _ => 1.0,
            };
            let top = bottom + (full_top - bottom) * scale;
            let ray_y = origin_y + (target_y - origin_y) * along;
            bottom <= ray_y && ray_y <= top
        })
    }

    fn record_wall_leaks(&mut self) {
        if self.wall_breach_records.is_empty() {
            return;
        }
        let entities = &self.entities;
        let catalog = &self.catalog;
        let mut leaks = [0_u64; 2];
        for breach in &mut self.wall_breach_records {
            for (entity_id, entity) in entities.iter() {
                if entity.owner >= 2
                    || entity.owner == breach.owner
                    || !entity.is_active()
                    || breach.entrants.contains(&entity_id)
                {
                    continue;
                }
                let definition = catalog.get(entity.definition).expect("catalog ID");
                if !definition.strategic.is_mobile {
                    continue;
                }
                let radius = definition.movement.radius.max(0.0);
                let (min_x, min_z, max_x, max_z) = breach.bounds;
                if entity.position.x + radius >= min_x
                    && entity.position.x - radius <= max_x
                    && entity.position.z + radius >= min_z
                    && entity.position.z - radius <= max_z
                {
                    breach.entrants.insert(entity_id);
                    leaks[breach.owner as usize] += 1;
                }
            }
        }
        for (side, count) in leaks.into_iter().enumerate() {
            self.teams[side].metrics.wall_leaks += count;
        }
    }

    fn advance_combat(&mut self, seconds: f64) {
        let tactical_ticks = {
            let Some(combat) = &mut self.combat else {
                return;
            };
            combat.fractional_ticks += seconds * f64::from(crate::model::SIM_HZ);
            let ticks = (combat.fractional_ticks + 1e-9).floor() as usize;
            combat.fractional_ticks -= ticks as f64;
            ticks
        };

        for _ in 0..tactical_ticks {
            self.sync_combatants();
            self.advance_cloaks(f64::from(DT));
            self.update_sensor_memory();
            self.prepare_combat_commands();
            self.record_wall_friendly_obstruction(f64::from(DT));
            let before = [
                StartingResources {
                    metal: self.teams[0].economy.metal.stored,
                    energy: self.teams[0].economy.energy.stored,
                },
                StartingResources {
                    metal: self.teams[1].economy.metal.stored,
                    energy: self.teams[1].economy.energy.stored,
                },
            ];
            let cloak_shots: Vec<_> = self
                .cloak_states
                .iter()
                .filter(|(_, c)| c.wanted)
                .filter_map(|(id, _)| {
                    self.combat
                        .as_ref()?
                        .stepper
                        .combatant(id.to_raw())
                        .map(|u| (*id, u.weapons.iter().map(|w| w.shot_sequence).sum::<u64>()))
                })
                .collect();
            let after = {
                let combat = self.combat.as_mut().expect("combat remains enabled");
                combat.stepper.set_resources(Some(before));
                combat.stepper.step();
                combat
                    .stepper
                    .resources()
                    .expect("finite resources were set")
            };
            for (id, before_shots) in cloak_shots {
                let fired = self
                    .combat
                    .as_ref()
                    .and_then(|c| c.stepper.combatant(id.to_raw()))
                    .is_some_and(|u| {
                        u.definition.strategic.decloak_on_fire
                            && u.weapons.iter().map(|w| w.shot_sequence).sum::<u64>()
                                != before_shots
                    });
                if fired && let Some(cloak) = self.cloak_states.get_mut(&id) {
                    cloak.cloaked = false;
                    cloak.blocked_until =
                        self.sensor_seconds + 128.0 / f64::from(crate::model::SIM_HZ);
                    cloak.next_check = self.sensor_seconds;
                }
            }
            for side in 0..2 {
                self.teams[side]
                    .economy
                    .metal
                    .spend((before[side].metal - after[side].metal).max(0.0));
                self.teams[side]
                    .economy
                    .energy
                    .spend((before[side].energy - after[side].energy).max(0.0));
            }
            self.apply_combat_state();
            self.update_sensor_memory();
        }
    }

    fn sync_combatants(&mut self) {
        let combat = self.combat.as_mut().expect("combat is enabled");
        combat.scratch.clear();
        combat.scratch.extend(
            combat
                .known
                .iter()
                .copied()
                .filter(|id| !self.entities.contains(*id)),
        );
        for id in combat.scratch.iter().copied() {
            let _ = combat.stepper.remove(id.to_raw());
            combat.known.remove(&id);
        }

        for (id, entity) in self.entities.iter() {
            let collision_height_scale = match entity.role {
                EntityRole::Nanoframe { project_id } => self
                    .construction
                    .get(project_id)
                    .map_or(0.0, |project| project.progress as f32),
                _ => 1.0,
            };
            if combat.known.insert(id) {
                combat
                    .stepper
                    .insert(CombatSpawn {
                        key: id.to_raw(),
                        side: entity.owner,
                        definition: CombatDefinitionId(entity.definition.0 as usize),
                        position: entity.position,
                        heading: entity.heading,
                        hp: Some(entity.health as f32),
                    })
                    .expect("world catalog IDs are registered with the combat stepper");
                if collision_height_scale < 1.0 {
                    combat
                        .stepper
                        .update(
                            id.to_raw(),
                            CombatantUpdate {
                                collision_height_scale: Some(collision_height_scale),
                                ..Default::default()
                            },
                        )
                        .expect("new combatant remains addressable");
                }
            } else {
                combat
                    .stepper
                    .update(
                        id.to_raw(),
                        CombatantUpdate {
                            position: Some(entity.position),
                            heading: Some(entity.heading),
                            hp: Some(entity.health as f32),
                            collision_height_scale: Some(collision_height_scale),
                            ..Default::default()
                        },
                    )
                    .expect("known combatant remains addressable");
            }
        }
    }

    fn prepare_combat_commands(&mut self) {
        let observations: [BTreeMap<EntityId, ObservedEntity>; 2] = std::array::from_fn(|side| {
            let entities = match self.observation_policy {
                ObservationPolicy::Filtered {
                    line_of_sight_range,
                    radar_range,
                    radar_position_resolution,
                    remember_last_seen,
                } => self.filtered_entities(
                    side as u8,
                    line_of_sight_range,
                    radar_range,
                    radar_position_resolution,
                    remember_last_seen,
                ),
                ObservationPolicy::Omniscient => self
                    .entities
                    .iter()
                    .map(|(id, e)| {
                        self.exact_entity_observation(
                            id,
                            e,
                            if e.owner == side as u8 {
                                ObservationVisibility::Own
                            } else {
                                ObservationVisibility::LineOfSight
                            },
                        )
                    })
                    .collect(),
            };
            entities.into_iter().map(|e| (e.id, e)).collect()
        });
        let combat = self.combat.as_mut().expect("combat is enabled");
        for (side, contacts) in observations.iter().enumerate() {
            let disclosed: Vec<_> = contacts
                .values()
                .filter(|e| {
                    e.side != side as u8
                        && matches!(
                            e.visibility,
                            ObservationVisibility::LineOfSight
                                | ObservationVisibility::Radar
                                | ObservationVisibility::Sonar
                        )
                })
                .filter_map(|e| {
                    let actual = self.entities.get(e.id)?;
                    let exact = e.visibility == ObservationVisibility::LineOfSight;
                    let definition = self.catalog.get(actual.definition)?;
                    Some(TargetObservation {
                        target: e.id.to_raw(),
                        position: e.position,
                        height: self.map.ground_height(e.position.x, e.position.z)
                            + if exact {
                                definition.movement.collision_mid_y()
                            } else {
                                0.5
                            },
                        velocity: if exact {
                            combat
                                .stepper
                                .combatant(e.id.to_raw())
                                .map(|u| u.velocity())
                        } else {
                            None
                        },
                        exact,
                    })
                })
                .collect();
            combat
                .stepper
                .set_target_observations(side as u8, &disclosed)
                .expect("current contacts are synchronized combatants");
        }
        combat.targets.clear();
        for id in combat.known.iter().copied() {
            let Some(entity) = self.entities.get(id) else {
                continue;
            };
            let can_act = entity.owner < 2
                && entity.is_active()
                && entity.enabled
                && entity.powered
                && !self.builder_tasks.contains_key(&id);
            if !can_act {
                combat
                    .stepper
                    .clear_command(id.to_raw())
                    .expect("known combatant");
                continue;
            }
            let contacts = &observations[entity.owner as usize];
            let ordered_contact = self.attack_orders.get(&id).and_then(|order| match order {
                AttackTarget::Entity { entity } => contacts.get(entity),
                _ => None,
            });
            let ground_target = match self.attack_orders.get(&id) {
                Some(AttackTarget::Ground { position }) => Some(*position),
                _ => ordered_contact
                    .filter(|e| e.visibility == ObservationVisibility::Ghost)
                    .map(|e| e.position),
            };
            let ordered_target = ordered_contact
                .filter(|e| {
                    matches!(
                        e.visibility,
                        ObservationVisibility::LineOfSight
                            | ObservationVisibility::Radar
                            | ObservationVisibility::Sonar
                    )
                })
                .map(|e| e.id);
            // Never reacquire an unseen unit merely because it still exists in
            // authoritative state. Ghosts are explicit position fire only.
            let should_acquire = ground_target.is_none()
                && (self.attack_orders.get(&id).is_none()
                    || matches!(self.attack_orders.get(&id),Some(AttackTarget::Entity{entity})
                    if !self.entities.contains(*entity) && ordered_contact.is_none()));
            let target_entity = ordered_target.or_else(|| {
                should_acquire
                    .then(|| {
                        Self::automatic_target(&self.entities, &self.catalog, id, entity, contacts)
                    })
                    .flatten()
            });
            if let Some(target) = target_entity {
                combat.targets.insert(id, target);
            }
            let mut destination = self
                .movement_orders
                .get(&id)
                .and_then(|w| w.front().copied());
            let mut throttle = if destination.is_some() { 1.0 } else { 0.0 };
            if destination.is_none() {
                let target_position = target_entity
                    .and_then(|target| contacts.get(&target).map(|e| e.position))
                    .or(ground_target)
                    .or_else(|| ordered_contact.map(|e| e.position))
                    .or_else(|| match self.attack_orders.get(&id) {
                        Some(AttackTarget::Entity { entity: target }) => {
                            self.observation_memory.borrow()[entity.owner as usize]
                                .get(target)
                                .map(|m| m.last.position)
                        }
                        _ => None,
                    });
                if let (Some(position), Some(definition)) =
                    (target_position, self.catalog.get(entity.definition))
                {
                    let range = definition
                        .weapons
                        .iter()
                        .map(|w| w.range.max(0.0))
                        .fold(0.0_f32, f32::max);
                    if range > 0.0 && entity.position.distance_squared(position) > range * range {
                        destination = Some(position);
                        throttle = 1.0;
                    } else if definition.weapons.iter().any(|w| !w.turret) {
                        destination = Some(position);
                    }
                }
            }
            combat
                .stepper
                .set_command(
                    id.to_raw(),
                    CombatCommand {
                        target: target_entity.map(EntityId::to_raw),
                        destination,
                        throttle,
                    },
                )
                .expect("validated combat targets");
            combat
                .stepper
                .set_ground_target(id.to_raw(), ground_target)
                .expect("known ground order");
        }
    }

    /// Automatic acquisition uses current shared contacts, never stale memory.
    fn automatic_target(
        entities: &EntityStore<WorldEntity>,
        catalog: &WorldCatalog,
        source_id: EntityId,
        source: &WorldEntity,
        contacts: &BTreeMap<EntityId, ObservedEntity>,
    ) -> Option<EntityId> {
        let source_definition = catalog.get(source.definition)?;
        contacts
            .iter()
            .filter_map(|(id, contact)| {
                if *id == source_id
                    || contact.side >= 2
                    || contact.side == source.owner
                    || !matches!(
                        contact.visibility,
                        ObservationVisibility::LineOfSight
                            | ObservationVisibility::Radar
                            | ObservationVisibility::Sonar
                    )
                {
                    return None;
                }
                let target = entities.get(*id)?;
                let definition = catalog.get(target.definition)?;
                let distance = source.position.distance_squared(contact.position);
                source_definition
                    .weapons
                    .iter()
                    .any(|weapon| {
                        !weapon.manual_fire
                            && weapon.range > 0.0
                            && distance <= weapon.range * weapon.range
                            && (weapon.only_target_categories.is_empty()
                                || weapon.only_target_categories.iter().any(|allowed| {
                                    definition
                                        .strategic
                                        .target_categories
                                        .iter()
                                        .any(|category| category.eq_ignore_ascii_case(allowed))
                                }))
                    })
                    .then_some((*id, distance))
            })
            .min_by(|(a, ad), (b, bd)| ad.total_cmp(bd).then(a.cmp(b)))
            .map(|(id, _)| id)
    }

    fn apply_combat_state(&mut self) {
        let mut resolved = {
            let combat = self.combat.as_mut().expect("combat is enabled");
            combat.scratch.clear();
            for (key, state) in combat.stepper.combatants() {
                let id = EntityId::from_raw(key);
                let Some(entity) = self.entities.get_mut(id) else {
                    continue;
                };
                let previous_health = entity.health;
                entity.position = state.position;
                entity.heading = state.heading;
                entity.health = f64::from(state.hp.max(0.0));
                if previous_health > entity.health + 1e-9 {
                    let damage = previous_health - entity.health;
                    self.events.push(WorldEvent::CombatDamage {
                        entity: id,
                        amount: damage,
                        remaining_health: entity.health,
                    });
                    if let Some(wall) = self.walls.get(&id)
                        && wall.owner < 2
                    {
                        self.teams[wall.owner as usize]
                            .metrics
                            .wall_intercepted_damage += damage;
                    }
                    if entity.owner < 2 {
                        let definition = self.catalog.get(entity.definition).expect("catalog ID");
                        let damage_value = damage * definition.resource_value() as f64
                            / f64::from(definition.health.max(1.0));
                        for side in 0_u8..2 {
                            if self.walls.values().any(|wall| {
                                wall.owner == side
                                    && wall.position.distance_squared(entity.position)
                                        <= WALL_LOCAL_EXCHANGE_RADIUS * WALL_LOCAL_EXCHANGE_RADIUS
                            }) {
                                let sign = if entity.owner == side { -1.0 } else { 1.0 };
                                self.teams[side as usize].metrics.wall_local_exchange_value +=
                                    damage_value * sign;
                            }
                        }
                    }
                }
                if let EntityRole::Nanoframe { project_id } = entity.role
                    && previous_health > entity.health
                    && let Some(project) = self.construction.get_mut(project_id)
                {
                    project.take_damage(previous_health - entity.health);
                    entity.health = project.health;
                }
                if self
                    .movement_orders
                    .get(&id)
                    .and_then(|waypoints| waypoints.front())
                    .is_some_and(|destination| {
                        entity.position.distance_squared(*destination) < 0.25
                    })
                {
                    combat.scratch.push(id);
                }
                let goals_finished = self.movement_goals.get_mut(&id).is_some_and(|goals| {
                    while goals
                        .front()
                        .is_some_and(|goal| entity.position.distance_squared(*goal) < 0.25)
                    {
                        goals.pop_front();
                    }
                    goals.is_empty()
                });
                if goals_finished {
                    self.movement_goals.remove(&id);
                    self.movement_route_epochs.remove(&id);
                }
            }
            combat.scratch.extend(
                combat
                    .stepper
                    .deaths()
                    .iter()
                    .map(|death| EntityId::from_raw(death.key)),
            );
            combat.scratch.sort_unstable();
            combat.scratch.dedup();
            std::mem::take(&mut combat.scratch)
        };

        for id in resolved.iter().copied() {
            if self
                .entities
                .get(id)
                .is_some_and(|entity| entity.health <= 0.0)
            {
                self.destroy_entity(id);
            } else {
                let finished = self.movement_orders.get_mut(&id).is_some_and(|waypoints| {
                    waypoints.pop_front();
                    waypoints.is_empty()
                });
                if finished {
                    self.movement_orders.remove(&id);
                }
            }
        }
        resolved.clear();
        self.combat
            .as_mut()
            .expect("combat remains enabled")
            .scratch = resolved;
    }

    fn finalize_construction(&mut self) {
        let completed: Vec<_> = self
            .projects
            .iter()
            .filter_map(|(project_id, runtime)| {
                let entity_is_nanoframe = runtime.entity.is_some_and(|entity| {
                    self.entities
                        .get(entity)
                        .is_some_and(|state| state.lifecycle == EntityLifecycle::UnderConstruction)
                });
                (entity_is_nanoframe
                    && self
                        .construction
                        .get(*project_id)
                        .is_some_and(|project| project.state == ProjectState::Complete))
                .then_some((*project_id, runtime.entity, runtime.factory))
            })
            .collect();
        for (project_id, entity, factory) in completed {
            let Some(entity_id) = entity else {
                continue;
            };
            if factory.is_none() {
                let definition_id = self.entities.get(entity_id).expect("live").definition;
                let definition = self.catalog.get(definition_id).expect("catalog ID").clone();
                self.promote_occupancy(entity_id, &definition);
                if let Some(entity) = self.entities.get_mut(entity_id) {
                    entity.lifecycle = EntityLifecycle::PendingActivation {
                        tick: self.tick + 1,
                    };
                    entity.health = definition.health.max(0.0) as f64;
                }
                self.builder_tasks
                    .retain(|_, task| task.project_id != project_id);
                self.events.push(WorldEvent::ProjectCompleted {
                    project_id,
                    entity: entity_id,
                });
            }
        }
        for runtime in self.factories.values_mut() {
            runtime.factory.post_construction(&self.construction);
        }
    }

    fn advance_repairs_and_reclaim(&mut self, seconds: f64) {
        self.advance_repairs(seconds);
        self.advance_reclaim(seconds);
    }

    fn advance_repairs(&mut self, seconds: f64) {
        let mut by_side: [Vec<(u64, EntityId, f64)>; 2] = std::array::from_fn(|_| Vec::new());
        for (builder_id, target_id) in &self.repair_orders {
            let (Some(builder), Some(target)) = (
                self.entities.get(*builder_id),
                self.entities.get(*target_id),
            ) else {
                continue;
            };
            if builder.owner >= 2
                || builder.owner != target.owner
                || !builder.is_active()
                || !builder.enabled
                || !builder.powered
                || !target.is_active()
            {
                continue;
            }
            let builder_definition = self.catalog.get(builder.definition).expect("catalog ID");
            let target_definition = self.catalog.get(target.definition).expect("catalog ID");
            let max_health = target_definition.health.max(0.0) as f64;
            let missing = (max_health - target.health).max(0.0);
            if missing <= f64::EPSILON {
                continue;
            }
            let range = builder_definition.strategic.build_distance.max(0.0)
                + target_definition.movement.radius.max(0.0);
            if builder.position.distance_squared(target.position) > range * range {
                continue;
            }
            let repair = (max_health * builder_definition.strategic.build_speed.max(0.0) as f64
                / target_definition.strategic.build_time.max(1.0) as f64
                * seconds)
                .min(missing);
            if repair > f64::EPSILON {
                let demand_id = REPAIR_DEMAND_BIT | (builder_id.to_raw() & (REPAIR_DEMAND_BIT - 1));
                by_side[builder.owner as usize].push((demand_id, *target_id, repair));
            }
        }
        for (side, repairs) in by_side.iter().enumerate() {
            let demands: Vec<_> = repairs
                .iter()
                .map(|(demand_id, target_id, repair)| {
                    let target = self.entities.get(*target_id).expect("repair target");
                    let definition = self.catalog.get(target.definition).expect("catalog ID");
                    let health_fraction = repair / definition.health.max(1.0) as f64;
                    ResourceDemand {
                        id: *demand_id,
                        priority: -10,
                        requested: Resources::new(
                            definition.metal_cost.max(0.0) as f64 * health_fraction,
                            definition.energy_cost.max(0.0) as f64 * health_fraction,
                        ),
                    }
                })
                .collect();
            let allocations = self.teams[side].economy.allocate(&demands, seconds);
            let fractions: BTreeMap<_, _> = allocations
                .iter()
                .map(|allocation| (allocation.id, allocation.fraction))
                .collect();
            for (demand_id, target_id, requested_repair) in repairs {
                let applied = requested_repair * fractions.get(demand_id).copied().unwrap_or(0.0);
                if let Some(target) = self.entities.get_mut(*target_id) {
                    let max_health = self
                        .catalog
                        .get(target.definition)
                        .expect("catalog ID")
                        .health
                        .max(0.0) as f64;
                    target.health = (target.health + applied).min(max_health);
                }
                self.teams[side].metrics.health_repaired += applied;
            }
        }
    }

    fn advance_reclaim(&mut self, seconds: f64) {
        let orders: Vec<_> = self
            .reclaim_orders
            .iter()
            .map(|(builder, target)| (*builder, *target))
            .collect();
        for (builder_id, order) in orders {
            let Some(builder) = self.entities.get(builder_id) else {
                self.reclaim_orders.remove(&builder_id);
                continue;
            };
            if builder.owner >= 2 || !builder.is_active() || !builder.enabled || !builder.powered {
                continue;
            }
            let builder_owner = builder.owner;
            let builder_definition_id = builder.definition;
            let builder_position = builder.position;
            let target_id = match order {
                ReclaimTarget::Entity { entity } => entity,
                ReclaimTarget::Area { position, radius } => {
                    let radius_squared = radius.max(0.0) * radius.max(0.0);
                    let Some(entity) = self.entities.iter().find_map(|(id, entity)| {
                        (entity.role == EntityRole::Feature
                            && entity.position.distance_squared(position) <= radius_squared)
                            .then_some(id)
                    }) else {
                        continue;
                    };
                    entity
                }
            };
            let Some(target) = self.entities.get(target_id) else {
                continue;
            };
            let Some(mut reclaim) = target.reclaim else {
                continue;
            };
            let definition = self.catalog.get(builder_definition_id).expect("catalog ID");
            let range = definition.strategic.build_distance.max(0.0);
            if builder_position.distance_squared(target.position) > range * range {
                continue;
            }
            reclaim.progress += definition.strategic.build_speed.max(0.0) as f64 * seconds
                / reclaim.reclaim_time.max(f64::EPSILON);
            if reclaim.progress < 1.0 - 1e-12 {
                if let Some(target) = self.entities.get_mut(target_id) {
                    target.reclaim = Some(reclaim);
                }
                continue;
            }
            self.forget_observed_removal(target_id);
            self.entities.remove(target_id);
            self.occupancy.remove_occupant(occupant(target_id));
            deposit_pool(
                &mut self.teams[builder_owner as usize].economy.metal,
                reclaim.metal,
            );
            deposit_pool(
                &mut self.teams[builder_owner as usize].economy.energy,
                reclaim.energy,
            );
            self.teams[builder_owner as usize].metrics.metal_reclaimed += reclaim.metal;
            self.teams[builder_owner as usize].metrics.energy_reclaimed += reclaim.energy;
            self.events.push(WorldEvent::Reclaimed {
                entity: target_id,
                side: builder_owner,
            });
            self.reclaim_orders.remove(&builder_id);
        }
    }

    fn promote_occupancy(&mut self, entity_id: EntityId, definition: &UnitDef) {
        let Some(runtime) = self
            .projects
            .values_mut()
            .find(|runtime| runtime.entity == Some(entity_id))
        else {
            return;
        };
        let Some(mut resolution) = runtime.placement.clone() else {
            return;
        };
        self.occupancy.remove_occupant(occupant(entity_id));
        for (_, layers) in &mut resolution.cells {
            layers.construction_reservation = false;
            layers.blocks_movement = definition.movement.blocks_movement;
            layers.fire_height = definition
                .movement
                .blocks_fire
                .then_some(definition.movement.collision_height());
        }
        execute_placement(&mut self.occupancy, &resolution);
        runtime.placement = Some(resolution);
    }

    fn record_idle_time(&mut self, seconds: f64) {
        for (entity_id, entity) in self.entities.iter() {
            if !entity.is_active() || entity.owner >= 2 {
                continue;
            }
            let definition = self.catalog.get(entity.definition).expect("catalog ID");
            if definition.strategic.is_builder
                && !self.builder_tasks.contains_key(&entity_id)
                && !self.repair_orders.contains_key(&entity_id)
                && !self.reclaim_orders.contains_key(&entity_id)
            {
                self.teams[entity.owner as usize]
                    .metrics
                    .idle_builder_seconds += seconds;
            }
            if definition.strategic.is_factory
                && self
                    .factories
                    .get(&entity_id)
                    .is_some_and(|factory| matches!(factory.factory.state, FactoryState::Idle))
            {
                self.teams[entity.owner as usize]
                    .metrics
                    .idle_factory_seconds += seconds;
            }
        }
    }
}

impl ActionWorld for World {
    fn is_valid_side(&self, side: u8) -> bool {
        side < 2
    }

    fn entity(&self, entity: EntityId) -> Option<ActionEntity> {
        let state = self.entities.get(entity)?;
        let definition = self.catalog.get(state.definition)?;
        let strategic = &definition.strategic;
        Some(ActionEntity {
            side: state.owner,
            kind: self.strategy_entity_kind(entity, state),
            alive: true,
            capabilities: ActionCapabilities {
                move_order: strategic.is_mobile && state.is_active(),
                attack: !definition.weapons.is_empty() && state.is_active(),
                build: strategic.is_builder && state.is_active(),
                assist: strategic.is_builder && strategic.can_assist && state.is_active(),
                repair: strategic.is_builder && state.is_active(),
                factory_queue: strategic.is_factory && state.is_active(),
                activation: strategic.onoffable && state.is_active(),
                cloak: strategic.can_cloak && state.is_active(),
                reclaim: strategic.is_builder && state.is_active(),
                cancel: true,
            },
        })
    }

    fn is_position_valid(&self, position: Vec2) -> bool {
        self.map.contains(position.x, position.z)
    }

    fn is_observed(&self, side: u8, entity: EntityId) -> bool {
        if side >= 2 {
            return false;
        }
        let Some(target) = self.entities.get(entity) else {
            return false;
        };
        if target.role == EntityRole::Feature {
            return true;
        }
        if target.owner == side {
            return true;
        }
        let ObservationPolicy::Filtered {
            line_of_sight_range,
            radar_range,
            ..
        } = self.observation_policy
        else {
            return true;
        };
        let sensors: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, candidate)| {
                candidate.owner == side
                    && candidate.is_active()
                    && candidate.role != EntityRole::Feature
            })
            .collect();
        self.visibility_from_sensors(
            &sensors,
            entity,
            target,
            finite_sensor_range_squared(line_of_sight_range),
            finite_sensor_range_squared(radar_range),
        )
        .is_some()
    }

    fn definition_exists(&self, definition: &str) -> bool {
        self.catalog.id(definition).is_some()
    }

    fn builder_can_build(&self, builder: EntityId, definition: &str) -> bool {
        let Some(entity) = self.entities.get(builder) else {
            return false;
        };
        let Some(builder_definition) = self.catalog.get(entity.definition) else {
            return false;
        };
        builder_definition.strategic.is_builder
            && (builder_definition.strategic.build_options.is_empty()
                || builder_definition
                    .strategic
                    .build_options
                    .iter()
                    .any(|option| option == definition))
    }

    fn factory_can_build(&self, factory: EntityId, definition: &str) -> bool {
        let Some(entity) = self.entities.get(factory) else {
            return false;
        };
        let Some(factory_definition) = self.catalog.get(entity.definition) else {
            return false;
        };
        factory_definition.strategic.is_factory
            && factory_definition
                .strategic
                .build_options
                .iter()
                .any(|option| option == definition)
    }

    fn factory_queue_len(&self, factory: EntityId) -> usize {
        self.factories
            .get(&factory)
            .map_or(0, |runtime| runtime.factory.queue().len())
    }

    fn placement_rejection(
        &self,
        side: u8,
        _builder: EntityId,
        definition: &str,
        position: Vec2,
        facing: BuildFacing,
    ) -> Option<ActionRejection> {
        let (_, definition) = self.catalog.get_by_name(definition)?;
        if !definition.strategic.is_building {
            return None;
        }
        let mut request = PlacementRequest::from_unit_def(
            OccupantId(u64::MAX - side as u64),
            definition,
            position,
            facing,
        );
        request.owner = Some(side);
        validate_placement(
            &self.map,
            &self.occupancy,
            &request,
            CellLayers {
                blocks_movement: definition.movement.blocks_movement,
                fire_height: definition.movement.blocks_fire.then_some(0.0),
                construction_reservation: true,
                factory_yard: false,
            },
        )
        .err()
        .map(|_| ActionRejection::PlacementRejected)
    }
}

fn entity_role(definition: &UnitDef) -> EntityRole {
    if definition.strategic.is_factory {
        EntityRole::Factory
    } else if definition.strategic.is_building {
        EntityRole::Structure
    } else {
        EntityRole::Mobile
    }
}

fn static_feature_definition(name: String, blocker: &StaticBlocker) -> UnitDef {
    let width = blocker.max_x - blocker.min_x;
    let depth = blocker.max_z - blocker.min_z;
    let strategic = StrategicDef {
        reclaim: ReclaimDef {
            reclaimable: blocker.reclaimable,
            reclaim_time: blocker.reclaim_time,
            metal: blocker.metal,
            energy: blocker.energy,
        },
        ..StrategicDef::default()
    };
    UnitDef {
        flight: None,
        name,
        icon_type: Some("feature".into()),
        health: 1.0,
        metal_cost: 0.0,
        energy_cost: 0.0,
        strategic,
        movement: MovementDef {
            radius: width.max(depth) * 0.5,
            height: blocker.fire_height,
            blocks_movement: blocker.blocks_movement,
            blocks_fire: blocker.blocks_fire,
            ..MovementDef::default()
        },
        weapons: Vec::new(),
    }
}

fn occupant(entity: EntityId) -> OccupantId {
    OccupantId(entity.to_raw())
}

fn project_spent_value(project: &ConstructionProject) -> f64 {
    (project.spec.cost.metal + project.spec.cost.energy / 60.0) * project.progress
}

fn completed_layers(definition: &UnitDef) -> CellLayers {
    CellLayers {
        blocks_movement: definition.movement.blocks_movement,
        fire_height: definition
            .movement
            .blocks_fire
            .then_some(definition.movement.collision_height()),
        construction_reservation: false,
        factory_yard: false,
    }
}

fn set_pool_capacity(pool: &mut ResourcePool, capacity: f64) {
    pool.capacity = capacity.max(0.0);
    if pool.stored > pool.capacity {
        pool.stats.overflow += pool.stored - pool.capacity;
        pool.stored = pool.capacity;
    }
}

fn deposit_pool(pool: &mut ResourcePool, amount: f64) {
    let amount = if amount.is_finite() {
        amount.max(0.0)
    } else {
        0.0
    };
    let accepted = amount.min((pool.capacity - pool.stored).max(0.0));
    pool.stored += accepted;
    pool.stats.generated += amount;
    pool.stats.overflow += amount - accepted;
}

fn finite_seconds(seconds: f64) -> f64 {
    if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

fn complete_route_seconds(route: &Route, speed: f32) -> Option<f64> {
    (speed > f32::EPSILON
        && matches!(
            route.status,
            RouteStatus::Complete | RouteStatus::AlreadyAtGoal
        ))
    .then(|| f64::from(route.path_length) / f64::from(speed))
}

fn finite_nonnegative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_sensor_range_squared(range: f32) -> f32 {
    if range.is_finite() && range > 0.0 {
        range * range
    } else {
        0.0
    }
}

fn sampled_terrain_line_of_sight(
    map: &CompiledMap,
    from: Vec2,
    from_y: f32,
    to: Vec2,
    to_y: f32,
) -> bool {
    let delta = to - from;
    let distance = delta.length();
    if distance <= f32::EPSILON {
        return true;
    }
    // Sampling at half the smaller source-grid interval cannot skip a complete
    // heightfield cell. This path is intentionally reserved for filtered
    // observations rather than the default benchmark hot path.
    let spacing = (map.heights.step_x.min(map.heights.step_z) * 0.5).max(0.5);
    let steps = (distance / spacing).ceil().max(1.0) as u32;
    for step in 1..steps {
        let along = step as f32 / steps as f32;
        let position = from + delta * along;
        let sight_y = from_y + (to_y - from_y) * along;
        if map.ground_height(position.x, position.z) >= sight_y - 1e-4 {
            return false;
        }
    }
    true
}

fn pool_observation(pool: &ResourcePool) -> ResourceObservation {
    ResourceObservation {
        stored: pool.stored,
        capacity: pool.capacity,
        income_per_second: pool.income_per_second,
        expense_per_second: 0.0,
    }
}

fn hash_u64(hash: &mut u64, value: u64) {
    for byte in value.to_le_bytes() {
        *hash ^= byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn role_discriminant(role: EntityRole) -> u64 {
    match role {
        EntityRole::Mobile => 0,
        EntityRole::Structure => 1,
        EntityRole::Factory => 2,
        EntityRole::Feature => 3,
        EntityRole::Nanoframe { project_id } => 4 ^ project_id.rotate_left(7),
    }
}

fn lifecycle_discriminant(lifecycle: EntityLifecycle) -> u64 {
    match lifecycle {
        EntityLifecycle::Active => 0,
        EntityLifecycle::UnderConstruction => 1,
        EntityLifecycle::PendingActivation { tick } => 2 ^ tick.rotate_left(11),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{HelloMapOptions, MapSource, StaticBlocker};
    use crate::model::{CorpseDef, MovementDef, ReclaimDef, StrategicDef, WeaponDef};

    fn map() -> Arc<CompiledMap> {
        let json = r#"{
            "map_name":"world", "map_size":[128,128], "water_level":0,
            "terrain":{"heights":[[0,0],[0,0]],
              "movement_classes":{"BOT":[[true,true],[true,true]]}},
            "metal_spots":[{"x":96,"z":96,"worth":2}],
            "geothermal_spots":[]
        }"#;
        Arc::new(
            CompiledMap::compile(
                MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap(),
            )
            .unwrap(),
        )
    }

    fn map_with_metal_and_geothermal() -> Arc<CompiledMap> {
        let json = r#"{
            "map_name":"resource-sites", "map_size":[128,128], "water_level":0,
            "terrain":{"heights":[[0,0],[0,0]],
              "movement_classes":{"BOT":[[true,true],[true,true]]}},
            "metal_spots":[{"x":32,"z":32,"worth":2}],
            "geothermal_spots":[{"x":96,"z":96,"radius":16}]
        }"#;
        Arc::new(
            CompiledMap::compile(
                MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap(),
            )
            .unwrap(),
        )
    }

    fn map_with_static_blocker() -> Arc<CompiledMap> {
        let json = r#"{
            "map_name":"features", "map_size":[128,128], "water_level":0,
            "terrain":{"heights":[[0,0],[0,0]],
              "movement_classes":{"BOT":[[true,true],[true,true]]}},
            "metal_spots":[], "geothermal_spots":[]
        }"#;
        let mut source = MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap();
        source.static_blockers.push(StaticBlocker {
            feature_id: 42,
            definition: "reclaimable_rock".into(),
            x: 40.0,
            y: 0.0,
            z: 24.0,
            min_x: 32.0,
            min_z: 16.0,
            max_x: 48.0,
            max_z: 32.0,
            blocks_movement: true,
            blocks_fire: true,
            fire_height: 12.0,
            crushable: false,
            reclaimable: true,
            reclaim_time: 50.0,
            metal: 25.0,
            energy: 10.0,
            geothermal: false,
        });
        Arc::new(CompiledMap::compile(source).unwrap())
    }

    fn map_with_factory_exit_blocker() -> Arc<CompiledMap> {
        let json = r#"{
            "map_name":"factory-exit-feature", "map_size":[128,128], "water_level":0,
            "terrain":{"heights":[[0,0],[0,0]],
              "movement_classes":{"BOT":[[true,true],[true,true]]}},
            "metal_spots":[], "geothermal_spots":[]
        }"#;
        let mut source = MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap();
        source.static_blockers.push(StaticBlocker {
            feature_id: 77,
            definition: "factory_exit_rock".into(),
            x: 64.0,
            y: 0.0,
            z: 88.0,
            min_x: 56.0,
            min_z: 80.0,
            max_x: 72.0,
            max_z: 96.0,
            blocks_movement: true,
            blocks_fire: false,
            fire_height: 0.0,
            crushable: false,
            reclaimable: true,
            reclaim_time: 10.0,
            metal: 5.0,
            energy: 0.0,
            geothermal: false,
        });
        Arc::new(CompiledMap::compile(source).unwrap())
    }

    fn visibility_map(with_ridge: bool) -> Arc<CompiledMap> {
        let middle = if with_ridge { "48" } else { "0" };
        let json = format!(
            r#"{{
                "map_name":"visibility", "map_size":[256,64], "water_level":0,
                "terrain":{{
                    "heights":[[0,0],[0,0],[{middle},{middle}],[0,0],[0,0]],
                    "movement_classes":{{"BOT":[[true,true],[true,true],[true,true],[true,true],[true,true]]}}
                }},
                "metal_spots":[], "geothermal_spots":[]
            }}"#
        );
        Arc::new(
            CompiledMap::compile(
                MapSource::from_hello_json(&json, HelloMapOptions::default()).unwrap(),
            )
            .unwrap(),
        )
    }

    fn unit(name: &str) -> UnitDef {
        UnitDef {
            flight: None,
            name: name.into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 0.0,
            energy_cost: 0.0,
            // World fixtures assert base damage and resource conservation.
            strategic: StrategicDef {
                flanking: crate::model::FlankingDef {
                    mode: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            movement: MovementDef {
                radius: 4.0,
                height: 8.0,
                ..Default::default()
            },
            weapons: vec![],
        }
    }

    fn catalog() -> Arc<WorldCatalog> {
        let mut builder = unit("builder");
        builder.strategic.is_builder = true;
        builder.strategic.is_mobile = true;
        builder.strategic.build_speed = 100.0;
        builder.strategic.build_distance = 48.0;
        builder.strategic.movement_class = Some("BOT".into());
        builder.strategic.can_assist = true;
        builder.strategic.build_options = vec!["wall".into(), "factory".into()];
        builder.movement.max_speed = 64.0;

        let mut wall = unit("wall");
        wall.health = 1_000.0;
        wall.strategic.is_building = true;
        wall.strategic.is_wall = true;
        wall.strategic.build_time = 100.0;
        wall.strategic.footprint = [1, 1];
        wall.movement.height = 40.0;

        let mut factory = unit("factory");
        factory.strategic.is_building = true;
        factory.strategic.is_factory = true;
        factory.strategic.build_speed = 100.0;
        factory.strategic.build_time = 100.0;
        factory.strategic.footprint = [2, 2];
        factory.strategic.build_options = vec!["pawn".into()];

        let mut pawn = unit("pawn");
        pawn.metal_cost = 50.0;
        pawn.strategic.build_time = 100.0;
        pawn.strategic.is_mobile = true;
        pawn.movement.max_speed = 64.0;

        Arc::new(WorldCatalog::new(vec![builder, wall, factory, pawn]).unwrap())
    }

    fn resource_catalog() -> Arc<WorldCatalog> {
        let mut builder = unit("builder");
        builder.strategic.is_builder = true;
        builder.strategic.is_mobile = true;
        builder.strategic.build_speed = 100.0;
        builder.strategic.build_distance = 48.0;
        builder.strategic.movement_class = Some("BOT".into());
        builder.strategic.build_options = vec!["mex".into(), "geo".into()];

        let mut mex = unit("mex");
        mex.strategic.is_building = true;
        mex.strategic.footprint = [1, 1];
        mex.strategic.build_time = 100.0;
        mex.strategic.extracts_metal = 1.0;
        mex.strategic.extract_range = 16.0;

        let mut geo = unit("geo");
        geo.strategic.is_building = true;
        geo.strategic.footprint = [1, 1];
        geo.strategic.build_time = 100.0;
        geo.strategic.need_geo = true;
        geo.strategic.energy_make = 20.0;

        Arc::new(WorldCatalog::new(vec![builder, mex, geo]).unwrap())
    }

    fn world() -> World {
        World::new(
            map(),
            catalog(),
            WorldConfig {
                starting_resources: Resources::new(10_000.0, 10_000.0),
                base_storage: Resources::new(10_000.0, 10_000.0),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn combat_world(energy: f64) -> World {
        let mut gunner = unit("gunner");
        gunner.strategic.is_mobile = true;
        gunner.strategic.movement_class = Some("BOT".into());
        gunner.movement.max_speed = 24.0;
        gunner.movement.acceleration = 96.0;
        gunner.weapons.push(WeaponDef {
            name: "test_laser".into(),
            damage: 10.0,
            range: 112.0,
            reload: 0.5,
            hitscan: true,
            energy_cost: 10.0,
            firing_origin: Some([0.0, 6.0, 0.0]),
            ..Default::default()
        });

        let mut target = unit("target");
        target.strategic.is_mobile = true;
        target.strategic.movement_class = Some("BOT".into());

        let mut wall = unit("combat_wall");
        wall.health = 1_000.0;
        wall.metal_cost = 100.0;
        wall.strategic.is_building = true;
        wall.strategic.is_wall = true;
        wall.movement.radius = 6.0;
        wall.movement.height = 32.0;

        let catalog = Arc::new(WorldCatalog::new(vec![gunner, target, wall]).unwrap());
        World::new(
            map(),
            catalog,
            WorldConfig {
                starting_resources: Resources::new(100.0, energy),
                base_storage: Resources::new(100.0, energy),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn order_attack(world: &mut World, attacker: EntityId, target: EntityId) {
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: world.tick,
            action: StrategyAction::Attack {
                entities: vec![attacker],
                target: AttackTarget::Entity { entity: target },
                mode: OrderMode::Replace,
            },
        });
    }

    #[test]
    fn builder_places_physical_wall_and_activates_next_tick() {
        let mut world = world();
        let builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 24.0), BuildFacing::South)
            .unwrap();
        let project = world
            .issue_wall(
                builder,
                UnitDefId(1),
                Vec2::new(40.0, 24.0),
                BuildFacing::South,
                0,
            )
            .unwrap();

        world.step_seconds(0.5);
        let wall = world.project_entity(project).expect("nanoframe placed");
        assert!(world.occupancy.is_movement_blocked(40.0, 24.0));
        assert!(
            world
                .entities
                .get(wall)
                .is_some_and(|entity| entity.lifecycle == EntityLifecycle::UnderConstruction)
        );

        world.step_seconds(0.5);
        assert!(matches!(
            world.entities.get(wall).unwrap().lifecycle,
            EntityLifecycle::PendingActivation { .. }
        ));
        world.step_seconds(0.5);
        assert!(world.entities.get(wall).unwrap().is_active());
        assert_eq!(world.occupancy.effective_fire_height_at(40.0, 24.0), 40.0);
    }

    #[test]
    fn assistant_build_power_is_removed_when_the_assistant_dies() {
        let base_catalog = catalog();
        let mut definitions = (0..4)
            .map(|id| base_catalog.get(UnitDefId(id)).unwrap().clone())
            .collect::<Vec<_>>();
        definitions[0].strategic.build_speed = 50.0;
        let mut world = World::new(
            map(),
            Arc::new(WorldCatalog::new(definitions).unwrap()),
            WorldConfig {
                starting_resources: Resources::new(10_000.0, 10_000.0),
                base_storage: Resources::new(10_000.0, 10_000.0),
                ..Default::default()
            },
        )
        .unwrap();
        let primary = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 24.0), BuildFacing::South)
            .unwrap();
        let assistant = world
            .spawn_active(0, UnitDefId(0), Vec2::new(40.0, 24.0), BuildFacing::South)
            .unwrap();
        let project = world
            .issue_wall(
                primary,
                UnitDefId(1),
                Vec2::new(40.0, 24.0),
                BuildFacing::South,
                0,
            )
            .unwrap();

        // The first step turns the primary builder's in-range task into a
        // physical nanoframe, after which a second builder may assist.
        world.step_seconds(0.5);
        world.assist(assistant, project).unwrap();
        world.step_seconds(0.25);
        assert!(
            (world.construction.get(project).unwrap().progress - 0.5).abs() < 1e-9,
            "progress was {}",
            world.construction.get(project).unwrap().progress
        );

        assert!(world.destroy_entity(assistant));
        assert!(world.builder_task(assistant).is_none());
        world.step_seconds(0.25);
        assert!((world.construction.get(project).unwrap().progress - 0.625).abs() < 1e-9);

        world.step_seconds(0.75);
        let wall = world.project_entity(project).unwrap();
        assert!(matches!(
            world.entities.get(wall).unwrap().lifecycle,
            EntityLifecycle::PendingActivation { .. }
        ));
        world.step_seconds(0.25);
        assert!(world.entities.get(wall).unwrap().is_active());
    }

    #[test]
    fn completed_mex_and_geo_from_compiled_map_sites_produce_income() {
        let mut world = World::new(
            map_with_metal_and_geothermal(),
            resource_catalog(),
            WorldConfig {
                starting_resources: Resources::ZERO,
                base_storage: Resources::new(100.0, 100.0),
                ..Default::default()
            },
        )
        .unwrap();
        let mex_builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let geo_builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(96.0, 96.0), BuildFacing::South)
            .unwrap();
        world
            .issue_build(
                mex_builder,
                UnitDefId(1),
                Vec2::new(32.0, 32.0),
                BuildFacing::South,
                0,
            )
            .unwrap();
        world
            .issue_build(
                geo_builder,
                UnitDefId(2),
                Vec2::new(96.0, 96.0),
                BuildFacing::South,
                0,
            )
            .unwrap();

        // Place, construct, complete, then activate and collect one second of
        // income from the compiled metal/geo sites. Site-control occupancy
        // begins with the physical reservation, before activation.
        for _ in 0..4 {
            world.step_seconds(0.5);
        }

        assert!(
            (world.teams[0].economy.metal.stored - 2.0).abs() < 1e-9,
            "metal was {} and energy was {}",
            world.teams[0].economy.metal.stored,
            world.teams[0].economy.energy.stored
        );
        assert!((world.teams[0].economy.energy.stored - 20.0).abs() < 1e-9);
        let metrics = world.strategic_metrics(0).unwrap();
        assert_eq!(metrics.get(StrategicMetric::MetalIncomeIntegral), 2.0);
        assert_eq!(metrics.get(StrategicMetric::EnergyIncomeIntegral), 20.0);
        assert_eq!(metrics.get(StrategicMetric::SiteOccupancyIntegral), 4.0);
        assert_eq!(metrics.get(StrategicMetric::MapControlIntegral), 2.0);
    }

    #[test]
    fn factory_stalls_releases_and_respects_blocked_exit() {
        let mut world = world();
        let factory = world
            .spawn_active(0, UnitDefId(2), Vec2::new(64.0, 64.0), BuildFacing::South)
            .unwrap();
        world.queue_factory(factory, UnitDefId(3)).unwrap();
        world.set_factory_exit_blocked(factory, true).unwrap();

        world.step_seconds(0.5); // idle -> starting
        world.step_seconds(0.5); // project placed and half built
        world.step_seconds(0.5); // complete -> releasing
        world.step_seconds(0.5); // exit blocked
        assert!(matches!(
            world.factory_state(factory),
            Some(FactoryState::ExitBlocked { .. })
        ));
        let product = world
            .projects
            .values()
            .find(|project| project.factory == Some(factory))
            .and_then(|project| project.entity)
            .unwrap();
        assert!(!world.entities.get(product).unwrap().is_active());

        world.set_factory_exit_blocked(factory, false).unwrap();
        world.step_seconds(0.5);
        assert!(world.entities.get(product).unwrap().is_active());
        assert_eq!(world.teams[0].metrics.units_produced, 1);
        assert_eq!(world.teams[0].metrics.production_value, 50.0);
    }

    #[test]
    fn factory_release_detects_dynamic_blocker_and_uses_facing() {
        let mut world = world();
        let factory = world
            .spawn_active(0, UnitDefId(2), Vec2::new(64.0, 64.0), BuildFacing::East)
            .unwrap();
        let blocker = world
            .spawn_active(0, UnitDefId(3), Vec2::new(84.01, 64.0), BuildFacing::South)
            .unwrap();
        world.queue_factory(factory, UnitDefId(3)).unwrap();

        for _ in 0..4 {
            world.step_seconds(0.5);
        }
        assert!(matches!(
            world.factory_state(factory),
            Some(FactoryState::ExitBlocked { .. })
        ));
        let product = world
            .projects
            .values()
            .find(|project| project.factory == Some(factory))
            .and_then(|project| project.entity)
            .unwrap();
        assert_eq!(
            world.entities.get(product).unwrap().position,
            Vec2::new(64.0, 64.0)
        );

        assert!(world.destroy_entity(blocker));
        world.step_seconds(0.5);
        let emerged = world.entities.get(product).unwrap();
        assert!(emerged.is_active());
        assert!(emerged.position.x - 4.0 > 80.0);
        assert_eq!(emerged.position.z, 64.0);
    }

    #[test]
    fn reclaimed_exit_blocker_resumes_repeat_queue_without_losing_occupancy() {
        let mut world = World::new_with_static_features(
            map_with_factory_exit_blocker(),
            catalog(),
            WorldConfig {
                starting_resources: Resources::new(10_000.0, 10_000.0),
                base_storage: Resources::new(10_000.0, 10_000.0),
                ..Default::default()
            },
        )
        .unwrap();
        let feature = world
            .entities
            .iter()
            .find_map(|(id, entity)| (entity.role == EntityRole::Feature).then_some(id))
            .unwrap();
        let factory = world
            .spawn_active(0, UnitDefId(2), Vec2::new(64.0, 64.0), BuildFacing::South)
            .unwrap();
        let builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(64.0, 112.0), BuildFacing::South)
            .unwrap();
        world.factories.get_mut(&factory).unwrap().factory.repeat = true;
        world.queue_factory(factory, UnitDefId(3)).unwrap();

        for _ in 0..4 {
            world.step_seconds(0.5);
        }
        assert!(matches!(
            world.factory_state(factory),
            Some(FactoryState::ExitBlocked { .. })
        ));
        let first = world
            .projects
            .values()
            .find(|project| project.factory == Some(factory))
            .and_then(|project| project.entity)
            .unwrap();

        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: world.tick,
            action: StrategyAction::Reclaim {
                entities: vec![builder],
                target: ReclaimTarget::Entity { entity: feature },
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(0.5); // reclaim completes after the blocked release phase
        assert!(world.entities.get(feature).is_none());
        world.step_seconds(0.5); // the now-clear product emerges
        assert!(world.entities.get(first).unwrap().is_active());
        assert!(world.occupancy.is_movement_blocked(64.0, 64.0));
        assert!(!world.occupancy.is_movement_blocked(64.0, 88.0));
        assert_eq!(
            world.factories.get(&factory).unwrap().factory.queue().len(),
            1
        );

        world.set_movement_order(first, Vec2::new(64.0, 120.0), OrderMode::Replace);
        // Four phases: repeat item enters Starting, is placed, completes, and
        // releases. A fifth step would correctly begin the next repeat item.
        for _ in 0..4 {
            world.step_seconds(0.5);
        }
        assert_eq!(world.teams[0].metrics.units_produced, 2);
        assert!(matches!(
            world.factory_state(factory),
            Some(FactoryState::Idle)
        ));
        assert!(world.occupancy.is_movement_blocked(64.0, 64.0));
        let factory_cell = world.occupancy.cell(4, 4).unwrap();
        assert_eq!(
            factory_cell
                .contributions
                .iter()
                .filter(|contribution| contribution.occupant_id == occupant(factory))
                .count(),
            1
        );
    }

    #[test]
    fn active_entities_drive_income_and_destroyed_generators_stop_it() {
        let mut generator = unit("generator");
        generator.strategic.is_building = false;
        generator.strategic.energy_make = 20.0;
        let catalog = Arc::new(WorldCatalog::new(vec![generator]).unwrap());
        let mut world = World::new(
            map(),
            catalog,
            WorldConfig {
                starting_resources: Resources::ZERO,
                base_storage: Resources::new(100.0, 100.0),
                ..Default::default()
            },
        )
        .unwrap();
        let generator = world
            .spawn_active(0, UnitDefId(0), Vec2::new(16.0, 16.0), BuildFacing::South)
            .unwrap();
        world.step_seconds(1.0);
        assert!((world.teams[0].economy.energy.stored - 20.0).abs() < 1e-9);
        let metrics = world.strategic_metrics(0).unwrap();
        assert_eq!(metrics.get(StrategicMetric::EnergyGenerated), 20.0);
        assert_eq!(metrics.get(StrategicMetric::EnergyIncomeIntegral), 20.0);
        assert!(
            world
                .resource_ledger(0)
                .unwrap()
                .conservation(&metrics, 1e-9)
                .closes
        );
        assert!(world.destroy_entity(generator));
        let stored = world.teams[0].economy.energy.stored;
        world.step_seconds(1.0);
        assert_eq!(world.teams[0].economy.energy.stored, stored);
    }

    #[test]
    fn negative_upkeep_is_income_while_positive_upkeep_remains_a_demand() {
        let mut generator = unit("solar");
        generator.strategic.energy_upkeep = -20.0;
        generator.strategic.metal_upkeep = -2.0;
        let catalog = Arc::new(WorldCatalog::new(vec![generator]).unwrap());
        let mut world = World::new(
            map(),
            catalog,
            WorldConfig {
                starting_resources: Resources::ZERO,
                base_storage: Resources::new(100.0, 100.0),
                ..Default::default()
            },
        )
        .unwrap();
        world
            .spawn_active(0, UnitDefId(0), Vec2::new(16.0, 16.0), BuildFacing::South)
            .unwrap();

        world.step_seconds(1.0);

        assert_eq!(world.teams[0].economy.metal.stored, 2.0);
        assert_eq!(world.teams[0].economy.energy.stored, 20.0);
        assert_eq!(world.teams[0].economy.metal.stats.spent, 0.0);
        assert_eq!(world.teams[0].economy.energy.stats.spent, 0.0);
    }

    #[test]
    fn filtered_observation_distinguishes_los_radar_and_unseen_contacts() {
        let mut world =
            World::new(visibility_map(true), catalog(), WorldConfig::default()).unwrap();
        let mut definitions = world.catalog.definitions().to_vec();
        definitions[3].strategic.radar_emit_height = Some(200.0);
        world.catalog = Arc::new(WorldCatalog::new(definitions).unwrap());
        let own = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let visible = world
            .spawn_active(1, UnitDefId(3), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let radar = world
            .spawn_active(1, UnitDefId(3), Vec2::new(166.0, 32.0), BuildFacing::South)
            .unwrap();
        let unseen = world
            .spawn_active(1, UnitDefId(3), Vec2::new(240.0, 32.0), BuildFacing::South)
            .unwrap();
        world.entities.get_mut(own).unwrap().health = 25.0;

        let observation = world
            .observation_with_policy(
                0,
                ObservationPolicy::Filtered {
                    line_of_sight_range: 180.0,
                    radar_range: 180.0,
                    radar_position_resolution: 32.0,
                    remember_last_seen: false,
                },
            )
            .unwrap();

        let own_contact = observation.entity(own).unwrap();
        assert_eq!(own_contact.visibility, ObservationVisibility::Own);
        assert_eq!(own_contact.definition.as_deref(), Some("pawn"));
        assert_eq!(own_contact.health_fraction, Some(0.25));
        assert_eq!(
            observation.entity(visible).unwrap().visibility,
            ObservationVisibility::LineOfSight
        );
        let radar_contact = observation.entity(radar).unwrap();
        assert_eq!(radar_contact.visibility, ObservationVisibility::Radar);
        assert_eq!(radar_contact.definition, None);
        assert_eq!(radar_contact.kind, StrategyEntityKind::Unknown);
        assert_eq!(radar_contact.health_fraction, None);
        assert_eq!(
            radar_contact.position,
            sensors::radar_position(radar.to_raw(), 0, Vec2::new(166.0, 32.0), 32.0)
        );
        assert!(observation.entity(unseen).is_none());
        assert!(
            observation
                .entities
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
        );
    }

    #[test]
    fn last_seen_memory_preserves_disclosure_without_leaking_current_position() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        let sensor = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(3), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 0.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };

        let first = world.observation_with_policy(0, policy).unwrap();
        assert_eq!(
            first.entity(target).unwrap().visibility,
            ObservationVisibility::LineOfSight
        );

        world.entities.get_mut(sensor).unwrap().position = Vec2::new(224.0, 32.0);
        world.entities.get_mut(target).unwrap().position = Vec2::new(32.0, 32.0);
        world.entities.get_mut(target).unwrap().health = 10.0;
        let remembered = world.observation_with_policy(0, policy).unwrap();
        let contact = remembered.entity(target).unwrap();
        assert_eq!(contact.visibility, ObservationVisibility::Remembered);
        assert_eq!(contact.position, Vec2::new(80.0, 32.0));
        assert_eq!(contact.health_fraction, Some(1.0));
    }

    #[test]
    fn radar_memory_stays_anonymous_and_keeps_its_last_disclosed_error() {
        let mut world =
            World::new(visibility_map(true), catalog(), WorldConfig::default()).unwrap();
        let mut definitions = world.catalog.definitions().to_vec();
        definitions[3].strategic.radar_emit_height = Some(200.0);
        world.catalog = Arc::new(WorldCatalog::new(definitions).unwrap());
        let sensor = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(3), Vec2::new(166.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 180.0,
            radar_range: 180.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };

        let first = world.observation_with_policy(0, policy).unwrap();
        let radar = first.entity(target).unwrap();
        assert_eq!(radar.visibility, ObservationVisibility::Radar);
        assert_eq!(
            radar.position,
            sensors::radar_position(target.to_raw(), 0, Vec2::new(166.0, 32.0), 32.0)
        );

        world.entities.get_mut(sensor).unwrap().position = Vec2::new(224.0, 32.0);
        world.entities.get_mut(target).unwrap().position = Vec2::new(32.0, 32.0);
        let hidden_policy = ObservationPolicy::Filtered {
            line_of_sight_range: 40.0,
            radar_range: 40.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };
        let remembered = world.observation_with_policy(0, hidden_policy).unwrap();
        let contact = remembered.entity(target).unwrap();
        assert_eq!(contact.visibility, ObservationVisibility::Remembered);
        assert_eq!(
            contact.position,
            sensors::radar_position(target.to_raw(), 0, Vec2::new(166.0, 32.0), 32.0)
        );
        assert_eq!(contact.definition, None);
        assert_eq!(contact.kind, StrategyEntityKind::Unknown);
        assert_eq!(contact.health_fraction, None);
    }

    #[test]
    fn identity_learned_in_los_survives_a_radar_only_contact() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(3), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 180.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };

        let first = world.observation_with_policy(0, policy).unwrap();
        let seen = first.entity(target).unwrap();
        assert_eq!(seen.visibility, ObservationVisibility::LineOfSight);
        let (definition, kind) = (seen.definition.clone(), seen.kind);
        assert_eq!(definition.as_deref(), Some("pawn"));

        // Out of LOS (134 > 64) but inside radar (134 <= 180): the blip is
        // imprecise, yet it is the unit we already identified.
        world.entities.get_mut(target).unwrap().position = Vec2::new(166.0, 32.0);
        let second = world.observation_with_policy(0, policy).unwrap();
        let blip = second.entity(target).unwrap();
        assert_eq!(blip.visibility, ObservationVisibility::Radar);
        assert_eq!(
            blip.position,
            sensors::radar_position(target.to_raw(), 0, Vec2::new(166.0, 32.0), 32.0)
        );
        assert_eq!(blip.definition, definition);
        assert_eq!(blip.kind, kind);
    }

    #[test]
    fn identity_learned_in_los_survives_radar_then_memory() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        let sensor = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(3), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 180.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };
        let first = world.observation_with_policy(0, policy).unwrap();
        let kind = first.entity(target).unwrap().kind;

        // Radar-only tick: this used to overwrite the stored identity.
        world.entities.get_mut(target).unwrap().position = Vec2::new(166.0, 32.0);
        let radar = world.observation_with_policy(0, policy).unwrap();
        assert_eq!(
            radar.entity(target).unwrap().visibility,
            ObservationVisibility::Radar
        );

        // Every sensor now loses the target; only the memory is left.
        world.entities.get_mut(sensor).unwrap().position = Vec2::new(240.0, 32.0);
        world.entities.get_mut(target).unwrap().position = Vec2::new(32.0, 32.0);
        let hidden = ObservationPolicy::Filtered {
            line_of_sight_range: 40.0,
            radar_range: 40.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };
        let remembered = world.observation_with_policy(0, hidden).unwrap();
        let ghost = remembered.entity(target).unwrap();
        assert_eq!(ghost.visibility, ObservationVisibility::Remembered);
        assert_eq!(
            ghost.position,
            sensors::radar_position(target.to_raw(), 0, Vec2::new(166.0, 32.0), 32.0)
        );
        assert_eq!(ghost.definition.as_deref(), Some("pawn"));
        assert_eq!(ghost.kind, kind);
    }

    #[test]
    fn remembered_mobile_contact_grows_position_error_with_age_up_to_the_map_diagonal() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        let sensor = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(3), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 0.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };
        let speed = world.catalog.get(UnitDefId(3)).unwrap().movement.max_speed;
        assert!(speed > 0.0);

        let seen = world.observation_with_policy(0, policy).unwrap();
        let live = seen.entity(target).unwrap();
        assert_eq!(live.visibility, ObservationVisibility::LineOfSight);
        assert_eq!((live.age_seconds, live.position_error), (0.0, 0.0));

        world.entities.get_mut(sensor).unwrap().position = Vec2::new(224.0, 32.0);
        world.step_seconds(2.0);
        let soon = world.observation_with_policy(0, policy).unwrap();
        let contact = soon.entity(target).unwrap();
        assert_eq!(contact.visibility, ObservationVisibility::Remembered);
        assert!((contact.age_seconds - 2.0).abs() < 1e-3);
        assert!((contact.position_error - speed * 2.0).abs() < 1e-2);

        world.step_seconds(60.0);
        let late = world.observation_with_policy(0, policy).unwrap();
        let contact = late.entity(target).unwrap();
        assert!((contact.age_seconds - 62.0).abs() < 1e-3);
        let diagonal = (world.map.width as f32).hypot(world.map.depth as f32);
        assert_eq!(contact.position_error, diagonal);
    }

    #[test]
    fn building_ghost_ages_but_keeps_an_exact_position() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        let sensor = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let wall = world
            .spawn_active(1, UnitDefId(1), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 0.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };
        let seen = world.observation_with_policy(0, policy).unwrap();
        assert_eq!(
            seen.entity(wall).unwrap().visibility,
            ObservationVisibility::LineOfSight
        );

        world.entities.get_mut(sensor).unwrap().position = Vec2::new(224.0, 32.0);
        world.step_seconds(30.0);
        let later = world.observation_with_policy(0, policy).unwrap();
        let ghost = later.entity(wall).unwrap();
        assert_eq!(ghost.visibility, ObservationVisibility::Ghost);
        assert!((ghost.age_seconds - 30.0).abs() < 1e-3);
        assert_eq!(ghost.position_error, 0.0);
        assert_eq!(ghost.position, Vec2::new(80.0, 32.0));
    }

    #[test]
    fn live_contacts_have_zero_age_and_radar_reports_its_error_bound() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let blip = world
            .spawn_active(1, UnitDefId(3), Vec2::new(166.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 180.0,
            radar_position_resolution: 32.0,
            remember_last_seen: true,
        };
        world.step_seconds(5.0);
        let observation = world.observation_with_policy(0, policy).unwrap();
        let contact = observation.entity(blip).unwrap();
        assert_eq!(contact.visibility, ObservationVisibility::Radar);
        assert_eq!(contact.age_seconds, 0.0);
        assert_eq!(contact.position_error, 32.0);
    }

    #[test]
    fn unseen_destruction_does_not_clear_memory_until_the_position_is_observed() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        let sensor = world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(3), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let policy = ObservationPolicy::Filtered {
            line_of_sight_range: 64.0,
            radar_range: 0.0,
            radar_position_resolution: 0.0,
            remember_last_seen: true,
        };
        world.observation_with_policy(0, policy).unwrap();

        world.entities.get_mut(sensor).unwrap().position = Vec2::new(224.0, 32.0);
        world.destroy_entity(target);
        assert_eq!(
            world
                .observation_with_policy(0, policy)
                .unwrap()
                .entity(target)
                .unwrap()
                .visibility,
            ObservationVisibility::Remembered
        );

        world.entities.get_mut(sensor).unwrap().position = Vec2::new(80.0, 32.0);
        assert!(
            world
                .observation_with_policy(0, policy)
                .unwrap()
                .entity(target)
                .is_none()
        );
    }

    #[test]
    fn fire_blocking_unit_cylinder_does_not_occlude_sensor_maps() {
        let mut world =
            World::new(visibility_map(false), catalog(), WorldConfig::default()).unwrap();
        world
            .spawn_active(0, UnitDefId(3), Vec2::new(32.0, 32.0), BuildFacing::South)
            .unwrap();
        let wall = world
            .spawn_active(1, UnitDefId(1), Vec2::new(80.0, 32.0), BuildFacing::South)
            .unwrap();
        let behind_wall = world
            .spawn_active(1, UnitDefId(3), Vec2::new(120.0, 32.0), BuildFacing::South)
            .unwrap();

        let observation = world
            .observation_with_policy(
                0,
                ObservationPolicy::Filtered {
                    line_of_sight_range: 200.0,
                    radar_range: 0.0,
                    radar_position_resolution: 0.0,
                    remember_last_seen: false,
                },
            )
            .unwrap();

        assert_eq!(
            observation.entity(wall).unwrap().visibility,
            ObservationVisibility::LineOfSight
        );
        assert_eq!(
            observation.entity(behind_wall).unwrap().visibility,
            ObservationVisibility::LineOfSight
        );
    }

    #[test]
    fn identical_worlds_produce_identical_terminal_hashes() {
        let mut left = world();
        let mut right = world();
        for world in [&mut left, &mut right] {
            let builder = world
                .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 24.0), BuildFacing::South)
                .unwrap();
            world
                .issue_wall(
                    builder,
                    UnitDefId(1),
                    Vec2::new(40.0, 24.0),
                    BuildFacing::South,
                    0,
                )
                .unwrap();
            for _ in 0..4 {
                world.step_seconds(0.5);
            }
        }
        assert_eq!(left.state_hash(), right.state_hash());
    }

    #[test]
    fn combat_world_applies_orders_damage_and_weapon_resource_costs() {
        let mut world = combat_world(15.0);
        let attacker = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(1), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();
        world.enable_combat(7, "BOT", 16.0).unwrap();
        order_attack(&mut world, attacker, target);

        world.step_seconds(0.5);

        assert_eq!(world.action_decisions[0].result, Ok(()));
        assert_eq!(world.entities.get(target).unwrap().health, 90.0);
        assert_eq!(world.teams[0].economy.energy.stored, 5.0);
        assert_eq!(
            world.observation(0).unwrap().fire_intents,
            vec![ObservedFireIntent {
                source: attacker,
                target_position: Vec2::new(80.0, 32.0),
                blocked_by_friendly_wall: false,
            }]
        );
        assert!(world.events.iter().any(|event| matches!(
            event,
            WorldEvent::CombatDamage {
                entity,
                amount,
                remaining_health
            } if *entity == target && *amount == 10.0 && *remaining_health == 90.0
        )));
    }

    #[test]
    fn idle_combatants_engage_a_valid_enemy_already_in_weapon_range() {
        let mut world = combat_world(100.0);
        let left = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let right = world
            .spawn_active(1, UnitDefId(0), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();
        world.enable_combat(7, "BOT", 16.0).unwrap();

        world.step_seconds(0.5);

        // Local automatic targets are intentionally transient rather than a
        // hidden permanent attack-move order.
        assert_eq!(world.attack_order(left), None);
        assert_eq!(world.attack_order(right), None);
        assert!(world.entities.get(left).unwrap().health < 100.0);
        assert!(world.entities.get(right).unwrap().health < 100.0);
        let intents = world.observation(0).unwrap().fire_intents;
        assert!(intents.iter().any(|intent| intent.source == left));
        assert!(intents.iter().any(|intent| intent.source == right));
    }

    #[test]
    fn replace_orders_clear_incompatible_move_and_attack_state() {
        let mut world = combat_world(100.0);
        let attacker = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(0), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();
        let move_order = |mode| QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: vec![attacker],
                destination: Vec2::new(160.0, 32.0),
                mode,
            },
        };
        world.execute_action(&move_order(OrderMode::Replace));
        assert!(world.movement_goals.contains_key(&attacker));

        world.execute_action(&QueuedAction {
            side: 0,
            sequence: 2,
            apply_at_tick: 0,
            action: StrategyAction::Attack {
                entities: vec![attacker],
                target: AttackTarget::Entity { entity: target },
                mode: OrderMode::Replace,
            },
        });
        assert!(!world.movement_goals.contains_key(&attacker));
        assert_eq!(
            world.attack_order(attacker),
            Some(AttackTarget::Entity { entity: target })
        );

        world.execute_action(&move_order(OrderMode::Replace));
        assert_eq!(world.attack_order(attacker), None);
    }

    #[test]
    fn automatic_targeting_can_attack_an_unpowered_or_disabled_enemy_structure() {
        let mut world = combat_world(100.0);
        let attacker = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(1), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();
        let target_entity = world.entities.get_mut(target).unwrap();
        target_entity.enabled = false;
        target_entity.powered = false;
        world.enable_combat(7, "BOT", 16.0).unwrap();

        world.step_seconds(0.5);

        assert_eq!(world.attack_order(attacker), None);
        assert!(world.entities.get(target).unwrap().health < 100.0);
    }

    #[test]
    fn visible_enemy_fire_intent_is_disclosed_without_revealing_hidden_contacts() {
        let mut world = combat_world(15.0);
        let friendly = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let enemy = world
            .spawn_active(1, UnitDefId(0), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();
        world.enable_combat(7, "BOT", 16.0).unwrap();
        world.enqueue_action(QueuedAction {
            side: 1,
            sequence: 1,
            apply_at_tick: world.tick,
            action: StrategyAction::Attack {
                entities: vec![enemy],
                target: AttackTarget::Entity { entity: friendly },
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(0.25);

        let observation = world
            .observation_with_policy(
                0,
                ObservationPolicy::Filtered {
                    line_of_sight_range: 200.0,
                    radar_range: 0.0,
                    radar_position_resolution: 0.0,
                    remember_last_seen: false,
                },
            )
            .unwrap();
        assert!(observation.fire_intents.iter().any(
            |intent| intent.source == enemy && intent.target_position == Vec2::new(24.0, 32.0)
        ));
    }

    #[test]
    fn combat_world_preserves_spawn_facing_as_tactical_heading() {
        let mut world = combat_world(100.0);
        let unit = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::North)
            .unwrap();
        world.enable_combat(3, "BOT", 16.0).unwrap();

        let strategic_heading = world.entities.get(unit).unwrap().heading;
        let tactical_heading = world
            .combat
            .as_ref()
            .unwrap()
            .stepper
            .combatant(unit.to_raw())
            .unwrap()
            .heading;
        assert_eq!(strategic_heading, BuildFacing::North.heading());
        assert_eq!(tactical_heading, strategic_heading);
    }

    #[test]
    fn combat_attack_order_closes_to_weapon_range() {
        let mut world = combat_world(100.0);
        let attacker = world
            .spawn_active(0, UnitDefId(0), Vec2::new(8.0, 32.0), BuildFacing::East)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(1), Vec2::new(124.0, 32.0), BuildFacing::West)
            .unwrap();
        world.enable_combat(5, "BOT", 16.0).unwrap();
        order_attack(&mut world, attacker, target);

        world.step_seconds(0.5);

        assert!(world.entities.get(attacker).unwrap().position.x > 8.0);
        assert!(world.entities.get(target).unwrap().health < 100.0);
    }

    #[test]
    fn combat_world_wall_blocks_fire_until_destroyed() {
        let mut world = combat_world(100.0);
        let attacker = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let wall = world
            .spawn_active(0, UnitDefId(2), Vec2::new(52.0, 32.0), BuildFacing::South)
            .unwrap();
        let target = world
            .spawn_active(1, UnitDefId(1), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();
        world.enable_combat(11, "BOT", 16.0).unwrap();
        order_attack(&mut world, attacker, target);

        world.step_seconds(0.25);
        assert_eq!(world.entities.get(target).unwrap().health, 100.0);
        assert_eq!(world.teams[0].economy.energy.stored, 100.0);

        assert!(world.destroy_entity(wall));
        world.step_seconds(0.25);
        assert_eq!(world.entities.get(target).unwrap().health, 90.0);
        assert_eq!(world.teams[0].economy.energy.stored, 90.0);
    }

    #[test]
    fn combat_enabled_worlds_are_seed_deterministic() {
        let mut worlds = [combat_world(100.0), combat_world(100.0)];
        for world in &mut worlds {
            let attacker = world
                .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
                .unwrap();
            let target = world
                .spawn_active(1, UnitDefId(1), Vec2::new(80.0, 32.0), BuildFacing::West)
                .unwrap();
            world.enable_combat(17, "BOT", 16.0).unwrap();
            order_attack(world, attacker, target);
            world.step_seconds(1.0);
        }

        assert_eq!(worlds[0].state_hash(), worlds[1].state_hash());
        assert_eq!(worlds[0].combat_state_hash(), worlds[1].combat_state_hash());
    }

    #[test]
    fn combat_world_rejects_unsupported_catalog_mechanics() {
        let mut unsupported = unit("unsupported");
        unsupported.strategic.required_capabilities = vec!["unsupported_test_mechanic".into()];
        let catalog = Arc::new(WorldCatalog::new(vec![unsupported]).unwrap());
        let mut world = World::new(map(), catalog, WorldConfig::default()).unwrap();

        let error = world.enable_combat(1, "BOT", 16.0).unwrap_err();
        assert!(matches!(
            error,
            WorldError::UnsupportedCombatCapabilities(requirements)
                if requirements.len() == 1
                    && requirements[0].capability == "unsupported_test_mechanic"
        ));
    }

    #[test]
    fn action_inbox_validates_orders_and_rejects_replayed_sequences() {
        let mut world = world();
        let pawn = world
            .spawn_active(0, UnitDefId(3), Vec2::new(16.0, 16.0), BuildFacing::South)
            .unwrap();
        for (sequence, destination) in [(2, Vec2::new(64.0, 16.0)), (1, Vec2::new(32.0, 16.0))] {
            world.enqueue_action(QueuedAction {
                side: 0,
                sequence,
                apply_at_tick: 0,
                action: StrategyAction::Move {
                    entities: vec![pawn],
                    destination,
                    mode: OrderMode::Append,
                },
            });
        }
        world.step_seconds(0.25);
        assert_eq!(
            world
                .action_decisions
                .iter()
                .map(|decision| decision.queued.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(
            world
                .action_decisions
                .iter()
                .all(|decision| decision.result.is_ok())
        );
        assert_eq!(
            world.entities.get(pawn).unwrap().position,
            Vec2::new(32.0, 16.0)
        );

        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 2,
            apply_at_tick: world.tick,
            action: StrategyAction::Move {
                entities: vec![pawn],
                destination: Vec2::new(80.0, 16.0),
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(0.25);
        assert_eq!(
            world.action_decisions[0].result,
            Err(ActionRejection::SequenceAlreadySeen)
        );
    }

    #[test]
    fn strategy_build_action_enters_the_same_construction_pipeline() {
        let mut world = world();
        let builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 24.0), BuildFacing::South)
            .unwrap();
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Build {
                builder,
                definition: "wall".into(),
                position: Vec2::new(40.0, 24.0),
                facing: BuildFacing::South,
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(0.25);
        assert_eq!(world.action_decisions[0].result, Ok(()));
        assert!(world.occupancy.is_movement_blocked(40.0, 24.0));
        let observation = world.observation(0).unwrap();
        assert!(observation.entities.iter().any(|entity| {
            entity.kind == StrategyEntityKind::Nanoframe || entity.kind == StrategyEntityKind::Wall
        }));
    }

    #[test]
    fn repair_and_reclaim_close_the_resource_loop() {
        let mut builder = unit("builder");
        builder.strategic.is_builder = true;
        builder.strategic.is_mobile = true;
        builder.strategic.build_speed = 100.0;
        builder.strategic.build_distance = 64.0;

        let mut victim = unit("victim");
        victim.metal_cost = 100.0;
        victim.strategic.build_time = 100.0;
        victim.strategic.corpse = Some(CorpseDef {
            name: "wreck".into(),
            reclaim: ReclaimDef {
                reclaimable: true,
                reclaim_time: 100.0,
                metal: 25.0,
                energy: 10.0,
            },
            heap: None,
        });
        let wreck_definition = unit("wreck");
        let catalog = Arc::new(WorldCatalog::new(vec![builder, victim, wreck_definition]).unwrap());
        let mut world = World::new(
            map(),
            catalog,
            WorldConfig {
                starting_resources: Resources::new(100.0, 100.0),
                base_storage: Resources::new(1_000.0, 1_000.0),
                ..Default::default()
            },
        )
        .unwrap();
        let builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(16.0, 16.0), BuildFacing::South)
            .unwrap();
        let victim = world
            .spawn_active(0, UnitDefId(1), Vec2::new(24.0, 16.0), BuildFacing::South)
            .unwrap();
        world.entities.get_mut(victim).unwrap().health = 50.0;
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Repair {
                entities: vec![builder],
                target: victim,
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(0.5);
        assert_eq!(world.entities.get(victim).unwrap().health, 100.0);
        assert_eq!(world.teams[0].metrics.health_repaired, 50.0);

        assert!(world.destroy_entity(victim));
        let wreck = world
            .entities
            .iter()
            .find_map(|(id, entity)| (entity.role == EntityRole::Feature).then_some(id))
            .expect("death created reclaimable wreck");
        let metal_before = world.teams[0].economy.metal.stored;
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 2,
            apply_at_tick: world.tick,
            action: StrategyAction::Reclaim {
                entities: vec![builder],
                target: ReclaimTarget::Entity { entity: wreck },
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(1.0);
        assert!(!world.entities.contains(wreck));
        assert_eq!(world.teams[0].economy.metal.stored, metal_before + 25.0);
        assert_eq!(world.teams[0].metrics.energy_reclaimed, 10.0);
        let metrics = world.strategic_metrics(0).unwrap();
        assert!(
            world
                .resource_ledger(0)
                .unwrap()
                .conservation(&metrics, 1e-9)
                .closes
        );
    }

    #[test]
    fn map_features_materialize_as_removable_counted_occupancy() {
        let mut world = World::new_with_static_features(
            map_with_static_blocker(),
            catalog(),
            WorldConfig {
                starting_resources: Resources::new(100.0, 100.0),
                base_storage: Resources::new(1_000.0, 1_000.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(world.catalog.id("builder"), Some(UnitDefId(0)));
        let feature = world
            .entities
            .iter()
            .find_map(|(id, entity)| (entity.role == EntityRole::Feature).then_some(id))
            .unwrap();
        assert!(world.occupancy.is_movement_blocked(40.0, 24.0));
        assert!(world.occupancy.is_reserved_at(40.0, 24.0));
        assert_eq!(world.occupancy.effective_fire_height_at(40.0, 24.0), 12.0);

        let builder = world
            .spawn_active(0, UnitDefId(0), Vec2::new(16.0, 24.0), BuildFacing::South)
            .unwrap();
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Reclaim {
                entities: vec![builder],
                target: ReclaimTarget::Entity { entity: feature },
                mode: OrderMode::Replace,
            },
        });
        world.step_seconds(1.0);
        assert!(!world.entities.contains(feature));
        assert!(!world.occupancy.is_movement_blocked(40.0, 24.0));
        assert!(!world.occupancy.is_reserved_at(40.0, 24.0));
        assert_eq!(world.occupancy.effective_fire_height_at(40.0, 24.0), 0.0);
        assert_eq!(world.teams[0].metrics.metal_reclaimed, 25.0);
    }

    #[test]
    fn per_team_starting_resources_reset_the_conservation_baseline() {
        let mut world = world();
        world
            .set_team_starting_resources(1, Resources::new(123.0, 456.0))
            .unwrap();
        let ledger = world.resource_ledger(1).unwrap();
        assert_eq!(ledger.starting_metal, 123.0);
        assert_eq!(ledger.ending_metal, 123.0);
        assert_eq!(ledger.starting_energy, 456.0);
        assert_eq!(ledger.ending_energy, 456.0);
        assert!(
            ledger
                .conservation(&world.strategic_metrics(1).unwrap(), 1e-9)
                .closes
        );
        world.step_seconds(0.0);
        assert_eq!(
            world.set_team_starting_resources(1, Resources::ZERO),
            Err(WorldError::SimulationAlreadyStarted)
        );
    }

    #[test]
    fn prebuilt_scenario_walls_are_tracked_and_breached() {
        let mut world = world();
        let wall = world
            .spawn_active(0, UnitDefId(1), Vec2::new(40.0, 24.0), BuildFacing::South)
            .unwrap();
        assert!(world.walls.contains_key(&wall));
        assert_eq!(world.teams[0].metrics.wall_breaches, 0);

        let non_wall = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 24.0), BuildFacing::South)
            .unwrap();
        assert!(!world.walls.contains_key(&non_wall));
        assert!(world.register_wall(non_wall).is_ok());
        assert!(world.walls.contains_key(&non_wall));

        assert!(world.destroy_entity(wall));
        assert_eq!(world.teams[0].metrics.wall_breaches, 1);
        assert_eq!(world.wall_breach_records.len(), 1);
    }

    #[test]
    fn wall_fire_over_and_fire_into_intercepts_damage() {
        let mut low_wall = unit("low_wall");
        low_wall.health = 500.0;
        low_wall.strategic.is_building = true;
        low_wall.strategic.is_wall = true;
        low_wall.movement.radius = 6.0;
        low_wall.movement.height = 8.0;

        let mut high_wall = unit("high_wall");
        high_wall.health = 500.0;
        high_wall.strategic.is_building = true;
        high_wall.strategic.is_wall = true;
        high_wall.movement.radius = 6.0;
        high_wall.movement.height = 40.0;

        let mut elevated_gunner = unit("elevated_gunner");
        elevated_gunner.strategic.is_mobile = true;
        elevated_gunner.strategic.movement_class = Some("BOT".into());
        elevated_gunner.weapons.push(WeaponDef {
            name: "laser".into(),
            damage: 20.0,
            range: 120.0,
            reload: 0.5,
            hitscan: true,
            firing_origin: Some([0.0, 30.0, 0.0]),
            ..Default::default()
        });

        let mut ground_gunner = unit("ground_gunner");
        ground_gunner.strategic.is_mobile = true;
        ground_gunner.strategic.movement_class = Some("BOT".into());
        ground_gunner.weapons.push(WeaponDef {
            name: "ground_laser".into(),
            damage: 20.0,
            range: 120.0,
            reload: 0.5,
            hitscan: true,
            firing_origin: Some([0.0, 4.0, 0.0]),
            ..Default::default()
        });

        let mut target = unit("target");
        target.strategic.is_mobile = true;
        target.strategic.movement_class = Some("BOT".into());

        let catalog = Arc::new(
            WorldCatalog::new(vec![
                elevated_gunner,
                ground_gunner,
                target,
                low_wall,
                high_wall,
            ])
            .unwrap(),
        );

        // Case A: Elevated shooter fires over low wall and hits target
        {
            let mut world = World::new(map(), catalog.clone(), WorldConfig::default()).unwrap();
            let shooter = world
                .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
                .unwrap();
            let wall = world
                .spawn_active(1, UnitDefId(3), Vec2::new(52.0, 32.0), BuildFacing::South)
                .unwrap();
            let enemy_target = world
                .spawn_active(1, UnitDefId(2), Vec2::new(80.0, 32.0), BuildFacing::West)
                .unwrap();
            world.enable_combat(1, "BOT", 16.0).unwrap();
            order_attack(&mut world, shooter, enemy_target);
            world.step_seconds(0.25);

            assert_eq!(world.entities.get(wall).unwrap().health, 500.0);
            assert_eq!(world.teams[1].metrics.wall_intercepted_damage, 0.0);
            assert_eq!(world.entities.get(enemy_target).unwrap().health, 80.0);
        }

        // Case B: Ground shooter fires directly into tall wall; wall intercepts damage
        {
            let mut world = World::new(map(), catalog, WorldConfig::default()).unwrap();
            let shooter = world
                .spawn_active(0, UnitDefId(1), Vec2::new(24.0, 32.0), BuildFacing::East)
                .unwrap();
            let wall = world
                .spawn_active(1, UnitDefId(4), Vec2::new(52.0, 32.0), BuildFacing::South)
                .unwrap();
            let enemy_target = world
                .spawn_active(1, UnitDefId(2), Vec2::new(80.0, 32.0), BuildFacing::West)
                .unwrap();
            world.enable_combat(2, "BOT", 16.0).unwrap();
            order_attack(&mut world, shooter, enemy_target);
            world.step_seconds(0.25);

            assert_eq!(world.entities.get(enemy_target).unwrap().health, 100.0);
            assert_eq!(world.entities.get(wall).unwrap().health, 480.0);
            assert_eq!(world.teams[1].metrics.wall_intercepted_damage, 20.0);
        }
    }

    #[test]
    fn wall_friendly_obstruction_accumulates_seconds() {
        let mut shooter = unit("friendly_shooter");
        shooter.strategic.is_mobile = true;
        shooter.strategic.movement_class = Some("BOT".into());
        shooter.weapons.push(WeaponDef {
            name: "friendly_fire_avoiding_laser".into(),
            damage: 20.0,
            range: 120.0,
            reload: 0.5,
            hitscan: true,
            avoid_friendly: true,
            firing_origin: Some([0.0, 4.0, 0.0]),
            ..Default::default()
        });

        let mut wall = unit("tall_friendly_wall");
        wall.health = 1_000.0;
        wall.strategic.is_building = true;
        wall.strategic.is_wall = true;
        wall.movement.radius = 6.0;
        wall.movement.height = 40.0;

        let mut target = unit("enemy_target");
        target.strategic.is_mobile = true;
        target.strategic.movement_class = Some("BOT".into());

        let catalog = Arc::new(WorldCatalog::new(vec![shooter, wall, target]).unwrap());
        let mut world = World::new(map(), catalog, WorldConfig::default()).unwrap();

        let friendly_shooter = world
            .spawn_active(0, UnitDefId(0), Vec2::new(24.0, 32.0), BuildFacing::East)
            .unwrap();
        let _friendly_wall = world
            .spawn_active(0, UnitDefId(1), Vec2::new(52.0, 32.0), BuildFacing::South)
            .unwrap();
        let enemy_target = world
            .spawn_active(1, UnitDefId(2), Vec2::new(80.0, 32.0), BuildFacing::West)
            .unwrap();

        world.enable_combat(3, "BOT", 16.0).unwrap();
        order_attack(&mut world, friendly_shooter, enemy_target);

        world.step_seconds(0.5);

        assert_eq!(world.entities.get(enemy_target).unwrap().health, 100.0);
        assert!(world.teams[0].metrics.wall_friendly_obstruction_seconds >= 0.4);
    }

    #[test]
    fn wall_breach_and_leak_detection_tracks_penetrating_mobiles() {
        let mut world = world();
        let wall = world
            .spawn_active(0, UnitDefId(1), Vec2::new(40.0, 24.0), BuildFacing::South)
            .unwrap();

        let mobile = world
            .spawn_active(1, UnitDefId(3), Vec2::new(20.0, 24.0), BuildFacing::East)
            .unwrap();

        assert_eq!(world.teams[0].metrics.wall_breaches, 0);
        assert_eq!(world.teams[0].metrics.wall_leaks, 0);

        assert!(world.destroy_entity(wall));
        assert_eq!(world.teams[0].metrics.wall_breaches, 1);
        assert_eq!(world.wall_breach_records.len(), 1);

        world.enqueue_action(QueuedAction {
            side: 1,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: vec![mobile],
                destination: Vec2::new(60.0, 24.0),
                mode: OrderMode::Replace,
            },
        });

        // Step until mobile traverses through (40.0, 24.0)
        world.step_seconds(0.5);
        assert_eq!(world.teams[0].metrics.wall_leaks, 1);

        // Subsequent steps do not re-count the same entrant
        world.step_seconds(0.5);
        assert_eq!(world.teams[0].metrics.wall_leaks, 1);
    }

    #[test]
    fn wall_local_exchange_scoring_tracks_vicinity_value() {
        let mut gunner = unit("gunner");
        gunner.strategic.is_mobile = true;
        gunner.strategic.movement_class = Some("BOT".into());
        gunner.movement.max_speed = 24.0;
        gunner.weapons.push(WeaponDef {
            name: "test_laser".into(),
            damage: 10.0,
            range: 112.0,
            reload: 0.5,
            hitscan: true,
            energy_cost: 10.0,
            firing_origin: Some([0.0, 6.0, 0.0]),
            ..Default::default()
        });

        let mut target = unit("target");
        target.metal_cost = 100.0;
        target.strategic.is_mobile = true;
        target.strategic.movement_class = Some("BOT".into());

        let mut wall = unit("combat_wall");
        wall.health = 1_000.0;
        wall.metal_cost = 100.0;
        wall.strategic.is_building = true;
        wall.strategic.is_wall = true;
        wall.movement.radius = 6.0;
        wall.movement.height = 32.0;

        let catalog = Arc::new(WorldCatalog::new(vec![gunner, target, wall]).unwrap());
        let mut world = World::new(
            map(),
            catalog,
            WorldConfig {
                starting_resources: Resources::new(100.0, 100.0),
                base_storage: Resources::new(100.0, 100.0),
                ..Default::default()
            },
        )
        .unwrap();

        let _wall = world
            .spawn_active(0, UnitDefId(2), Vec2::new(48.0, 32.0), BuildFacing::South)
            .unwrap();
        let attacker = world
            .spawn_active(0, UnitDefId(0), Vec2::new(40.0, 48.0), BuildFacing::East)
            .unwrap();
        let enemy_target = world
            .spawn_active(1, UnitDefId(1), Vec2::new(56.0, 48.0), BuildFacing::West)
            .unwrap();

        world.enable_combat(9, "BOT", 16.0).unwrap();
        order_attack(&mut world, attacker, enemy_target);

        world.step_seconds(0.5);

        assert!(world.entities.get(enemy_target).unwrap().health < 100.0);
        assert!(world.teams[0].metrics.wall_local_exchange_value > 0.0);
    }

    #[test]
    fn wall_splash_and_occlusion() {
        let mut artillery = unit("artillery");
        artillery.strategic.is_mobile = true;
        artillery.strategic.movement_class = Some("BOT".into());
        artillery.weapons.push(WeaponDef {
            name: "explosive_cannon".into(),
            damage: 50.0,
            area_of_effect: 32.0,
            range: 120.0,
            reload: 0.5,
            hitscan: false,
            projectile_speed: 150.0,
            firing_origin: Some([0.0, 8.0, 0.0]),
            ..Default::default()
        });

        let mut wall = unit("heavy_wall");
        wall.health = 500.0;
        wall.strategic.is_building = true;
        wall.strategic.is_wall = true;
        wall.movement.radius = 8.0;
        wall.movement.height = 30.0;

        let mut target = unit("rear_target");
        target.strategic.is_mobile = true;
        target.strategic.movement_class = Some("BOT".into());

        let catalog = Arc::new(WorldCatalog::new(vec![artillery, wall, target]).unwrap());
        let mut world = World::new(map(), catalog, WorldConfig::default()).unwrap();

        let shooter = world
            .spawn_active(0, UnitDefId(0), Vec2::new(20.0, 32.0), BuildFacing::East)
            .unwrap();
        let wall_entity = world
            .spawn_active(1, UnitDefId(1), Vec2::new(50.0, 32.0), BuildFacing::South)
            .unwrap();
        let rear_target = world
            .spawn_active(1, UnitDefId(2), Vec2::new(90.0, 32.0), BuildFacing::West)
            .unwrap();

        world.enable_combat(13, "BOT", 16.0).unwrap();
        order_attack(&mut world, shooter, rear_target);

        // Projectile travels and impacts enemy wall
        world.step_seconds(0.5);

        assert!(world.entities.get(wall_entity).unwrap().health < 500.0);
        assert!(world.teams[1].metrics.wall_intercepted_damage >= 50.0);
        // Target at 90.0 is beyond the 32.0 AOE of the impact at 50.0
        assert_eq!(world.entities.get(rear_target).unwrap().health, 100.0);
    }

    #[test]
    fn wall_route_delay_and_replan_on_breach() {
        let mut world = world();
        // Wall line at x = 40.0
        let wall1 = world
            .spawn_active(0, UnitDefId(1), Vec2::new(40.0, 16.0), BuildFacing::South)
            .unwrap();
        let wall2 = world
            .spawn_active(0, UnitDefId(1), Vec2::new(40.0, 32.0), BuildFacing::South)
            .unwrap();
        let wall3 = world
            .spawn_active(0, UnitDefId(1), Vec2::new(40.0, 48.0), BuildFacing::South)
            .unwrap();

        let mover = world
            .spawn_active(1, UnitDefId(0), Vec2::new(16.0, 32.0), BuildFacing::East)
            .unwrap();

        world.enqueue_action(QueuedAction {
            side: 1,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: vec![mover],
                destination: Vec2::new(64.0, 32.0),
                mode: OrderMode::Replace,
            },
        });

        world.step_seconds(0.0);

        // Wall route delay should be recorded on team 0
        assert!(world.teams[0].metrics.wall_route_delay_seconds > 0.0);

        // Waypoints detour around the wall line (z != 32.0)
        let initial_waypoints = world.movement_orders.get(&mover).cloned().unwrap();
        assert!(initial_waypoints.iter().any(|wp| (wp.z - 32.0).abs() > 4.0));

        // Destroy the middle wall to create a breach
        assert!(world.destroy_entity(wall2));
        assert_eq!(world.teams[0].metrics.wall_breaches, 1);

        // Stepping replans stale movement orders through the new breach
        world.step_seconds(0.1);
        let replanned_waypoints = world.movement_orders.get(&mover).cloned().unwrap();
        // The direct path through the breach stays close to z = 32.0
        assert!(
            replanned_waypoints
                .iter()
                .all(|wp| (wp.z - 32.0).abs() <= 8.0)
        );

        let _ = (wall1, wall3);
    }

    #[test]
    fn world_derives_wind_and_tidal_output_from_map() {
        let json = r#"{
            "map_name":"wind_tidal_map", "map_size":[128,128], "water_level":0,
            "wind_min": 6.0, "wind_max": 14.0, "tidal": 25.0,
            "terrain":{"heights":[[0,0],[0,0]],
              "movement_classes":{"BOT":[[true,true],[true,true]]}},
            "metal_spots":[], "geothermal_spots":[]
        }"#;
        let map = Arc::new(
            CompiledMap::compile(
                MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap(),
            )
            .unwrap(),
        );
        let world = World::new(map, catalog(), WorldConfig::default()).unwrap();
        assert_eq!(world.config.wind_output, 10.0);
        assert_eq!(world.config.tidal_output, 25.0);
    }

    #[test]
    fn per_unit_sight_and_radar_sensors_respect_catalog_limits() {
        let mut scout = unit("scout");
        scout.strategic.is_mobile = true;
        scout.strategic.sight_distance = Some(30.0);
        scout.strategic.radar_radius = Some(60.0);

        let mut blind_radar = unit("radar_dish");
        blind_radar.strategic.sight_distance = Some(5.0);
        blind_radar.strategic.radar_radius = Some(150.0);

        let mut target = unit("enemy_target");
        target.strategic.is_mobile = true;

        let catalog = Arc::new(WorldCatalog::new(vec![scout, blind_radar, target]).unwrap());
        let mut world = World::new(
            map(),
            catalog,
            WorldConfig {
                observation_policy: ObservationPolicy::Filtered {
                    line_of_sight_range: 10.0,
                    radar_range: 10.0,
                    radar_position_resolution: 0.0,
                    remember_last_seen: false,
                },
                ..Default::default()
            },
        )
        .unwrap();

        let scout_id = world
            .spawn_active(
                0,
                world.catalog.id("scout").unwrap(),
                Vec2::new(32.0, 32.0),
                BuildFacing::South,
            )
            .unwrap();
        let enemy1 = world
            .spawn_active(
                1,
                world.catalog.id("enemy_target").unwrap(),
                Vec2::new(52.0, 32.0),
                BuildFacing::South,
            )
            .unwrap();
        let enemy2 = world
            .spawn_active(
                1,
                world.catalog.id("enemy_target").unwrap(),
                Vec2::new(80.0, 32.0),
                BuildFacing::South,
            )
            .unwrap();
        let enemy3 = world
            .spawn_active(
                1,
                world.catalog.id("enemy_target").unwrap(),
                Vec2::new(110.0, 32.0),
                BuildFacing::South,
            )
            .unwrap();

        let obs = world
            .observation_with_policy(0, world.observation_policy)
            .unwrap();
        let e1 = obs.entity(enemy1).unwrap();
        assert_eq!(e1.visibility, ObservationVisibility::LineOfSight);

        let e2 = obs.entity(enemy2).unwrap();
        assert_eq!(e2.visibility, ObservationVisibility::Radar);

        assert!(obs.entity(enemy3).is_none());

        let _radar_id = world
            .spawn_active(
                0,
                world.catalog.id("radar_dish").unwrap(),
                Vec2::new(32.0, 32.0),
                BuildFacing::South,
            )
            .unwrap();
        let obs2 = world
            .observation_with_policy(0, world.observation_policy)
            .unwrap();
        let e3 = obs2.entity(enemy3).unwrap();
        assert_eq!(e3.visibility, ObservationVisibility::Radar);

        let _ = scout_id;
    }

    #[test]
    fn world_milestones_track_and_score_progress() {
        let mut world = World::new(map(), catalog(), WorldConfig::default()).unwrap();
        world.set_milestones(vec![
            MilestoneDeclaration {
                name: "metal_10".into(),
                condition: MilestoneCondition::MetalProduced { threshold: 10.0 },
                points: 5.0,
            },
            MilestoneDeclaration {
                name: "tick_2".into(),
                condition: MilestoneCondition::TickReached { tick: 2 },
                points: 2.5,
            },
            MilestoneDeclaration {
                name: "unit_produced".into(),
                condition: MilestoneCondition::UnitsProduced { count: 1 },
                points: 15.0,
            },
        ]);

        assert_eq!(world.teams[0].metrics.milestones_reached, 0);
        assert_eq!(world.teams[0].metrics.milestone_score, 0.0);

        // Step 1: tick 0 -> 1, neither tick_2 nor metal_10 reached yet
        world.step();
        assert_eq!(world.teams[0].metrics.milestones_reached, 0);

        // Step 2: tick 1 -> 2, tick_2 reached!
        world.step();
        assert_eq!(world.teams[0].metrics.milestones_reached, 1);
        assert_eq!(world.teams[0].metrics.milestone_score, 2.5);

        // Step 3: tick 2 -> 3, tick_2 should not be double-counted
        world.step();
        assert_eq!(world.teams[0].metrics.milestones_reached, 1);
        assert_eq!(world.teams[0].metrics.milestone_score, 2.5);

        // Simulate metal production on team 0
        world.teams[0].economy.metal.stats.generated = 12.0;
        world.step();
        assert_eq!(world.teams[0].metrics.milestones_reached, 2);
        assert_eq!(world.teams[0].metrics.milestone_score, 7.5);

        // Simulate units produced on team 0
        world.teams[0].metrics.units_produced = 1;
        world.step();
        assert_eq!(world.teams[0].metrics.milestones_reached, 3);
        assert_eq!(world.teams[0].metrics.milestone_score, 22.5);

        // Verify StrategicMetric population matches
        let metrics = world.strategic_metrics(0).unwrap();
        assert_eq!(metrics.get(StrategicMetric::MilestonesReached), 3.0);
        assert_eq!(metrics.get(StrategicMetric::MilestoneScore), 22.5);
    }

    #[test]
    fn combat_world_supports_mixed_movement_classes() {
        let json = r#"{
            "map_name": "mixed_classes", "map_size": [64, 64], "water_level": 0,
            "terrain": {
                "heights": [[0, 0], [0, 0]],
                "movement_classes": {
                    "BOT": [[true, true], [false, false]],
                    "HOVER": [[true, true], [true, true]]
                }
            },
            "metal_spots": [], "geothermal_spots": []
        }"#;
        let map = Arc::new(
            CompiledMap::compile(
                MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap(),
            )
            .unwrap(),
        );

        let mut bot = unit("bot_unit");
        bot.strategic.is_mobile = true;
        bot.strategic.movement_class = Some("BOT".into());
        bot.movement.max_speed = 32.0;
        bot.movement.acceleration = 128.0;

        let mut hover = unit("hover_unit");
        hover.strategic.is_mobile = true;
        hover.strategic.movement_class = Some("HOVER".into());
        hover.movement.max_speed = 32.0;
        hover.movement.acceleration = 128.0;

        let catalog = Arc::new(WorldCatalog::new(vec![bot, hover]).unwrap());
        let mut world = World::new(map, catalog, WorldConfig::default()).unwrap();

        let bot_id = world
            .spawn_active(0, UnitDefId(0), Vec2::new(8.0, 16.0), BuildFacing::East)
            .unwrap();
        let hover_id = world
            .spawn_active(0, UnitDefId(1), Vec2::new(8.0, 48.0), BuildFacing::East)
            .unwrap();

        // Enable combat with cell size 32.0; BOT is baseline, but HOVER receives its own mask
        world.enable_combat(42, "BOT", 32.0).unwrap();

        // Give both units a move order across to the right side (x = 56.0)
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 1,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: vec![bot_id],
                destination: Vec2::new(56.0, 16.0),
                mode: OrderMode::Replace,
            },
        });
        world.enqueue_action(QueuedAction {
            side: 0,
            sequence: 2,
            apply_at_tick: 0,
            action: StrategyAction::Move {
                entities: vec![hover_id],
                destination: Vec2::new(56.0, 48.0),
                mode: OrderMode::Replace,
            },
        });

        // Step 2 seconds of combat simulation
        world.step_seconds(2.0);

        let bot_pos = world.entities.get(bot_id).unwrap().position;
        let hover_pos = world.entities.get(hover_id).unwrap().position;

        // BOT cannot enter col 1 (x in [32, 64]), so it stops before the boundary
        assert!(
            bot_pos.x <= 32.0,
            "BOT should be blocked by impassable column, but reached x={}",
            bot_pos.x
        );

        // HOVER traverses freely into col 1 toward destination (56.0)
        assert!(
            hover_pos.x > 32.0,
            "HOVER should cross the column freely, but is at x={}",
            hover_pos.x
        );
    }

    #[test]
    fn jammer_shields_friendly_units_from_radar_but_not_los() {
        let map = map();
        let mut radar_tower = unit("radar_tower");
        radar_tower.strategic.sight_distance = Some(50.0);
        radar_tower.strategic.radar_radius = Some(200.0);

        let mut scout = unit("scout");
        scout.strategic.sight_distance = Some(150.0);

        let mut jammer_bot = unit("jammer_bot");
        jammer_bot.strategic.sight_distance = Some(50.0);
        jammer_bot.strategic.jammer_radius = Some(80.0);

        let mut combatant = unit("combatant");
        combatant.strategic.sight_distance = Some(50.0);

        let catalog =
            Arc::new(WorldCatalog::new(vec![radar_tower, scout, jammer_bot, combatant]).unwrap());
        let mut world = World::new(
            map,
            catalog,
            WorldConfig {
                observation_policy: ObservationPolicy::Filtered {
                    line_of_sight_range: 0.0,
                    radar_range: 0.0,
                    radar_position_resolution: 1.0,
                    remember_last_seen: false,
                },
                ..Default::default()
            },
        )
        .unwrap();

        // Side 0 has a radar tower at (20, 20) with radar radius 200, sight 50
        world
            .spawn_active(0, UnitDefId(0), Vec2::new(20.0, 20.0), BuildFacing::North)
            .unwrap();

        // Side 1 has a jammer bot at (100, 20) with jammer radius 80
        let jammer_id = world
            .spawn_active(1, UnitDefId(2), Vec2::new(100.0, 20.0), BuildFacing::North)
            .unwrap();

        // Side 1 has a combatant at (120, 20) - distance to radar is 100 <= 200 (in radar range),
        // and distance to jammer is 20 <= 80 (inside jammer field!)
        let target_id = world
            .spawn_active(1, UnitDefId(3), Vec2::new(120.0, 20.0), BuildFacing::North)
            .unwrap();

        // Observation for Side 0: combatant and jammer are jammed, so radar CANNOT detect them!
        let obs_side_0 = world.observation(0).unwrap();
        assert!(obs_side_0.entities.iter().all(|e| e.id != target_id));

        // Now Side 0 moves a scout close enough for LOS: at (120, 40), distance to target (120, 20) is 20 <= 150
        let scout_id = world
            .spawn_active(0, UnitDefId(1), Vec2::new(120.0, 40.0), BuildFacing::North)
            .unwrap();
        let obs_side_0_los = world.observation(0).unwrap();
        let observed_target = obs_side_0_los
            .entities
            .iter()
            .find(|e| e.id == target_id)
            .unwrap();
        assert_eq!(
            observed_target.visibility,
            ObservationVisibility::LineOfSight
        );

        // Remove scout so it's not in LOS
        world.entities.remove(scout_id);

        // If jammer is disabled (turned off), radar can now detect target as radar contact!
        world.entities.get_mut(jammer_id).unwrap().enabled = false;
        let obs_side_0_unlocked = world.observation(0).unwrap();
        let radar_blip = obs_side_0_unlocked
            .entities
            .iter()
            .find(|e| e.id == target_id)
            .unwrap();
        assert_eq!(radar_blip.visibility, ObservationVisibility::Radar);
    }

    #[test]
    fn entity_sensor_ranges_respects_nanoframes_and_disabled_state() {
        let map = map();
        let mut jammer_radar = unit("jammer_radar");
        jammer_radar.strategic.sight_distance = Some(300.0);
        jammer_radar.strategic.radar_radius = Some(1500.0);
        jammer_radar.strategic.jammer_radius = Some(500.0);

        let catalog = Arc::new(WorldCatalog::new(vec![jammer_radar]).unwrap());
        let mut world = World::new(map, catalog, WorldConfig::default()).unwrap();

        let id = world
            .spawn_active(0, UnitDefId(0), Vec2::new(32.0, 32.0), BuildFacing::North)
            .unwrap();
        let entity = world.entities.get(id).unwrap();

        // Fully active and completed:
        let (los, radar, jammer) = world.entity_sensor_ranges(id, entity);
        assert_eq!(los, 300.0);
        assert_eq!(radar, 1500.0);
        assert_eq!(jammer, 500.0);

        // Disabled (toggled off): radar and jammer are 0.0, sight remains
        world.entities.get_mut(id).unwrap().enabled = false;
        let entity_disabled = world.entities.get(id).unwrap();
        let (los2, radar2, jammer2) = world.entity_sensor_ranges(id, entity_disabled);
        assert_eq!(los2, 300.0);
        assert_eq!(radar2, 0.0);
        assert_eq!(jammer2, 0.0);
    }
}
