//! Replay-informed strategic baseline, executed by MiniBAR's economic World.

use super::brain::{Brain, BrainError, Module, ModuleSpec};
use super::es::{EsConfig, EvolutionStrategy};
use crate::catalog::UnitCatalog;
use crate::construction::FactoryState;
use crate::economy::{ResourceStats, Resources};
use crate::map::{CompiledMap, MapLoadOptions};
use crate::model::Vec2;
use crate::occupancy::{CellLayers, OccupancyGrid, OccupantId};
use crate::placement::{BuildFacing, PlacementRequest, validate_placement};
use crate::routing::{GridRouter, RouteRequest, RouteStatus};
use crate::score::StrategicMetric;
use crate::strategy::{
    ActionDecision, ActionRejection, AttackTarget, FactoryQueueItem, FactoryQueueOperation,
    ObservationPolicy, ObservationVisibility, OrderMode, QueuedAction, StrategyAction,
    StrategyObservation,
};
use crate::world::{World, WorldCatalog, WorldConfig, WorldEvent};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const KIND: &str = "strategic_v1";
pub const ECON_INPUTS: [&str; 20] = [
    "metal_fill",
    "energy_fill",
    "metal_need",
    "energy_need",
    "metal_unmet",
    "energy_unmet",
    "metal_overflow",
    "energy_overflow",
    "candidate_mex",
    "candidate_power",
    "candidate_factory",
    "candidate_count",
    "travel",
    "cost",
    "known_threat",
    "factory_absent",
    "idle_builder_fraction",
    "yield",
    "metal_net",
    "energy_net",
];
const PROD_INPUTS: [&str; 12] = [
    "metal_fill",
    "energy_fill",
    "energy_need",
    "builder_fraction",
    "army_fraction",
    "candidate_builder",
    "candidate_combat",
    "cost",
    "observed_threat",
    "factory_count",
    "metal_net",
    "energy_net",
];
const MISSION_INPUTS: [&str; 10] = [
    "target_distance",
    "target_health",
    "target_factory",
    "target_economy",
    "target_mobile",
    "own_health",
    "own_value",
    "enemy_value",
    "home_distance",
    "target_identified",
];

pub fn specs(hidden: usize) -> Vec<ModuleSpec> {
    [
        (
            "investment",
            "Scores legal economic construction candidates",
            ECON_INPUTS.as_slice(),
        ),
        (
            "production",
            "Scores available factory products",
            PROD_INPUTS.as_slice(),
        ),
        (
            "mission",
            "Scores disclosed enemy objectives for each local combat group",
            MISSION_INPUTS.as_slice(),
        ),
    ]
    .into_iter()
    .map(|(name, role, inputs)| ModuleSpec {
        name: name.into(),
        role: role.into(),
        inputs: inputs.iter().map(|s| (*s).into()).collect(),
        outputs: vec!["utility".into()],
        hidden: vec![hidden],
    })
    .collect()
}

pub const KIND_V2: &str = "strategic_v2";
pub fn specs_for_kind(kind: &str, hidden: usize) -> Result<Vec<ModuleSpec>, BrainError> {
    if kind != KIND && kind != KIND_V2 {
        return Err(BrainError::UnsupportedFormat(kind.into()));
    }
    let mut result = specs(hidden);
    if kind == KIND_V2 {
        result[1].inputs.extend(
            super::strategic_sensors::EXTRA_INPUTS
                .iter()
                .map(|s| s.to_string()),
        );
    }
    Ok(result)
}
pub fn upgrade_v1_to_v2(brain: &Brain) -> Result<Brain, BrainError> {
    let hidden = brain
        .modules
        .first()
        .and_then(|m| m.spec.hidden.first())
        .copied()
        .unwrap_or(8);
    brain.check_against(KIND, &specs(hidden))?;
    let mut value: serde_json::Value =
        serde_json::from_str(&brain.to_json()).map_err(|e| BrainError::Parse(e.to_string()))?;
    value["kind"] = serde_json::json!(KIND_V2);
    let production = &mut value["modules"][1];
    production["inputs"].as_array_mut().unwrap().extend(
        super::strategic_sensors::EXTRA_INPUTS
            .iter()
            .map(|s| serde_json::json!(s)),
    );
    for row in production["layers"][0]["weights"].as_array_mut().unwrap() {
        row.as_array_mut().unwrap().extend(
            super::strategic_sensors::EXTRA_INPUTS
                .iter()
                .map(|_| serde_json::json!(0.0)),
        );
    }
    let count = super::strategic_sensors::EXTRA_INPUTS.len();
    production["structure"]["inputs"]
        .as_array_mut()
        .unwrap()
        .extend(std::iter::repeat_n(serde_json::json!(true), count));
    for row in production["structure"]["connections"][0]
        .as_array_mut()
        .unwrap()
    {
        row.as_array_mut()
            .unwrap()
            .extend(std::iter::repeat_n(serde_json::json!(true), count));
    }
    Brain::from_json(&value.to_string())
}
pub fn template_for_kind(kind: &str, hidden: usize) -> Result<Brain, BrainError> {
    let original = template(hidden)?;
    match kind {
        KIND => Ok(original),
        KIND_V2 => upgrade_v1_to_v2(&original),
        _ => Err(BrainError::UnsupportedFormat(kind.into())),
    }
}

fn wire(module: &mut Module, hidden: usize, input: &str, weight: f32) {
    let col = module
        .spec
        .inputs
        .iter()
        .position(|s| s == input)
        .expect("named sensor");
    module.net.params_mut()[hidden * module.spec.inputs.len() + col] = weight;
}
fn bias(module: &mut Module, hidden: usize, value: f32) {
    let offset = module.spec.inputs.len() * module.spec.hidden[0] + hidden;
    module.net.params_mut()[offset] = value;
}
fn feed(module: &mut Module, hidden: usize, output: usize, weight: f32) {
    let n = module.spec.inputs.len();
    let h = module.spec.hidden[0];
    module.net.params_mut()[n * h + h + output * h + hidden] = weight;
}
fn score(module: &Module, input: &[f32]) -> f32 {
    let mut output = [0.0];
    module.net.forward(input, &mut output);
    output[0]
}

/// Authored weights motivated by replay observations, not imitation-fitted weights.
pub fn template(hidden: usize) -> Result<Brain, BrainError> {
    let hidden = hidden.max(4);
    let mut brain = Brain::random(KIND, specs(hidden), &mut super::rng::Rng::new(1), 0.0)?;
    for module in &mut brain.modules {
        module.net.params_mut().fill(0.0);
    }
    let m = &mut brain.modules[0];
    for (h, flag) in [
        (0, "candidate_mex"),
        (1, "candidate_power"),
        (2, "candidate_factory"),
    ] {
        bias(m, h, -3.0);
        wire(m, h, flag, 4.0);
        wire(m, h, "candidate_count", -3.0);
        wire(m, h, "travel", -0.8);
        wire(m, h, "known_threat", -3.0);
        feed(m, h, 0, 1.0);
    }
    wire(m, 0, "metal_need", 1.0);
    wire(m, 0, "energy_need", -1.5);
    wire(m, 0, "metal_net", -1.0);
    wire(m, 1, "energy_need", 3.0);
    wire(m, 1, "energy_unmet", 3.0);
    wire(m, 1, "energy_net", -2.0);
    wire(m, 2, "factory_absent", 1.0);
    wire(m, 2, "energy_need", -1.0);
    let m = &mut brain.modules[1];
    wire(m, 0, "candidate_builder", 2.0);
    wire(m, 0, "builder_fraction", -6.0);
    bias(m, 0, -1.0);
    feed(m, 0, 0, 1.0);
    wire(m, 1, "candidate_combat", 2.5);
    wire(m, 1, "energy_need", -0.5);
    wire(m, 1, "observed_threat", 1.0);
    bias(m, 1, -1.0);
    feed(m, 1, 0, 1.0);
    let m = &mut brain.modules[2];
    wire(m, 0, "target_distance", -2.0);
    wire(m, 0, "target_health", -0.5);
    wire(m, 0, "target_factory", 1.6);
    wire(m, 0, "target_economy", 1.2);
    wire(m, 0, "target_identified", 0.5);
    feed(m, 0, 0, 1.0);
    Ok(brain)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactionSpec {
    pub name: String,
    pub builder: String,
    pub mex: String,
    pub power: String,
    pub factory: String,
    pub combat: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapSpec {
    pub map: PathBuf,
    pub movement_class: String,
    pub cell_size: f32,
    pub starts: [Vec2; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainConfig {
    pub name: String,
    pub hidden: usize,
    pub init_from: Option<PathBuf>,
    pub train_modules: Vec<String>,
    pub generations: u64,
    pub es: EsConfig,
    pub maps: Vec<MapSpec>,
    pub factions: Vec<FactionSpec>,
    pub max_ticks: u64,
    pub cadence_ticks: u64,
    pub validation_seeds: usize,
    pub train_seeds: usize,
    pub starting_metal: f64,
    pub starting_energy: f64,
    pub base_metal_income: f64,
    pub base_energy_income: f64,
    pub combat: bool,
    pub layout_jitter: f32,
    pub reward: super::strategic_reward::RewardConfig,
    pub brain_kind: String,
}
impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            name: "from_replays_strategic".into(),
            hidden: 8,
            init_from: None,
            train_modules: vec![],
            generations: 3,
            es: EsConfig {
                population: 8,
                ..EsConfig::default()
            },
            maps: vec![],
            factions: vec![],
            max_ticks: 7200,
            cadence_ticks: 30,
            validation_seeds: 1,
            train_seeds: 1,
            starting_metal: 1000.0,
            starting_energy: 1000.0,
            base_metal_income: 2.0,
            base_energy_income: 25.0,
            combat: true,
            layout_jitter: 24.0,
            reward: Default::default(),
            brain_kind: KIND.into(),
        }
    }
}
impl TrainConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut config: Self =
            serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let root = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path.parent().unwrap_or(Path::new(".")));
        for map in &mut config.maps {
            if map.map.is_relative() {
                map.map = root.join(&map.map);
            }
        }
        if let Some(p) = &mut config.init_from {
            if p.is_relative() {
                *p = root.join(&*p);
            }
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.es.validate()?;
        self.reward.validate()?;
        specs_for_kind(&self.brain_kind, self.hidden).map_err(|e| e.to_string())?;
        if !self.layout_jitter.is_finite() || self.layout_jitter < 0.0 {
            return Err("layout_jitter must be finite and nonnegative".into());
        }
        if self.maps.is_empty()
            || self.factions.is_empty()
            || self.max_ticks == 0
            || self.max_ticks > 1_000_000
            || self.cadence_ticks == 0
            || self.validation_seeds == 0
            || self.train_seeds == 0
        {
            return Err("strategic training requires maps, factions, bounded positive ticks, cadence and seeds".into());
        }
        if self.hidden < 4 || self.hidden > 32 {
            return Err("strategic hidden width must be 4..32".into());
        }
        for amount in [
            self.starting_metal,
            self.starting_energy,
            self.base_metal_income,
            self.base_energy_income,
        ] {
            if !amount.is_finite() || amount < 0.0 {
                return Err("strategic resources must be finite and nonnegative".into());
            }
        }
        for m in &self.maps {
            if !m.cell_size.is_finite()
                || m.cell_size <= 0.0
                || m.starts
                    .iter()
                    .any(|p| !p.x.is_finite() || !p.z.is_finite())
            {
                return Err("map cell size and starting positions must be finite".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct ResourceSignals {
    fill: f32,
    requested_net: f32,
    unmet: f32,
    overflow: f32,
}
fn resource_signals(
    stored: f64,
    capacity: f64,
    income: f64,
    expense: f64,
    pull: f64,
    overflow: f64,
) -> ResourceSignals {
    ResourceSignals {
        fill: (stored / capacity.max(1.0)).clamp(0.0, 1.0) as f32,
        requested_net: ((income - pull) / (income + pull + 1.0)).clamp(-1.0, 1.0) as f32,
        unmet: ((pull - expense).max(0.0) / pull.max(1.0)).clamp(0.0, 1.0) as f32,
        overflow: (overflow / income.max(1.0)).clamp(0.0, 1.0) as f32,
    }
}
fn bounded(x: f64, reference: f64) -> f32 {
    (x.max(0.0) / (x.max(0.0) + reference)) as f32
}

fn surviving_value(world: &World, side: u8) -> f64 {
    // Reward surviving assets once. Role metrics deliberately overlap (a
    // constructor can also store energy), so summing them biases this reward.
    world
        .entities
        .iter()
        .filter(|(_, entity)| {
            entity.owner == side && entity.role != crate::world::EntityRole::Feature
        })
        .map(|(_, entity)| {
            let definition = world.catalog.get(entity.definition).expect("catalog ID");
            let health_fraction = if definition.health > f32::EPSILON {
                (entity.health / definition.health as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            definition.resource_value() as f64 * health_fraction
        })
        .sum()
}

/// Placement/routing inputs contain only disclosed structures, never hidden occupancy.
fn known_occupancy(
    world: &World,
    observation: &StrategyObservation,
) -> Result<OccupancyGrid, String> {
    let mut grid = OccupancyGrid::new(
        world.map.width,
        world.map.depth,
        world.occupancy.cell_size(),
    )?;
    for entity in &observation.entities {
        let Some(def) = entity
            .definition
            .as_deref()
            .and_then(|name| world.catalog.get_by_name(name).map(|(_, d)| d))
        else {
            continue;
        };
        if !def.strategic.is_building {
            continue;
        }
        let half_x = def.strategic.footprint[0] as f32 * 8.0;
        let half_z = def.strategic.footprint[1] as f32 * 8.0;
        // Match validate_placement's cell rounding, rather than rasterizing a
        // geometric rectangle with different edge rules. Baseline structures
        // are all issued with South facing.
        let left = ((entity.position.x - half_x) / grid.cell_size())
            .round()
            .max(0.0) as u32;
        let top = ((entity.position.z - half_z) / grid.cell_size())
            .round()
            .max(0.0) as u32;
        let right = left + def.strategic.footprint[0] as u32 - 1;
        let bottom = top + def.strategic.footprint[1] as u32 - 1;
        {
            grid.add_occupant(
                OccupantId(entity.id.to_raw()),
                Some(entity.side),
                (top..=bottom)
                    .flat_map(|z| (left..=right).map(move |x| ((x, z), CellLayers::building(0.0)))),
            );
            if def.strategic.extracts_metal > 0.0 {
                let radius = def.strategic.extract_range;
                if let Some((index, _)) =
                    world.map.metal_nodes.iter().enumerate().find(|(_, node)| {
                        entity.position.distance_squared(Vec2::new(node.x, node.z))
                            <= radius * radius
                    })
                {
                    grid.reserve_metal_spot(OccupantId(entity.id.to_raw()), index);
                } else {
                    grid.reserve_extractor_site(
                        OccupantId(entity.id.to_raw()),
                        entity.position,
                        radius,
                    );
                }
            }
        }
    }
    Ok(grid)
}
fn known_placement(
    world: &World,
    grid: &OccupancyGrid,
    definition: &crate::model::UnitDef,
    position: Vec2,
) -> Option<Vec2> {
    let request = PlacementRequest::from_unit_def(
        OccupantId(u64::MAX),
        definition,
        position,
        BuildFacing::South,
    );
    validate_placement(&world.map, grid, &request, CellLayers::building(0.0))
        .ok()
        .map(|p| p.snapped_position)
}

/// A placement this side's own order was rejected at. A hidden occupant (for example an enemy
/// extractor outside line of sight) is not in the observation, so without this memory the same
/// proposal would repeat at every decision. Only the accept/reject outcome of the side's own
/// order is stored; the rejection reason, which could name a hidden occupant, is never read.
#[derive(Clone, Debug)]
struct RejectedPlacement {
    definition: String,
    position: Vec2,
    rejections: u32,
    retry_at: u64,
}
/// Base wait (30 ticks per second) before a rejected placement is proposed again: 30 s,
/// doubling with each further rejection of the same placement up to 8 times the base.
const REJECTED_PLACEMENT_BASE_TICKS: u64 = 900;
const REJECTED_PLACEMENT_MAX_DOUBLINGS: u32 = 3;
/// Rejections within this distance of an earlier one are the same site.
const REJECTED_PLACEMENT_RADIUS: f32 = 40.0;

fn placement_is_deferred(
    rejected: &[RejectedPlacement],
    tick: u64,
    definition: &str,
    position: Vec2,
) -> bool {
    rejected.iter().any(|entry| {
        entry.definition == definition
            && entry.retry_at > tick
            && entry.position.distance_squared(position)
                <= REJECTED_PLACEMENT_RADIUS * REJECTED_PLACEMENT_RADIUS
    })
}

/// The policy reads exact own telemetry and only disclosed enemy contacts.
pub struct StrategicPolicy {
    brain: Brain,
    faction: FactionSpec,
    side: u8,
    sequence: u64,
    previous: [ResourceStats; 2],
    previous_tick: u64,
    home: Vec2,
    rejected_placements: Vec<RejectedPlacement>,
}
impl StrategicPolicy {
    pub fn new(brain: Brain, faction: FactionSpec, side: u8, home: Vec2) -> Result<Self, String> {
        let hidden = brain
            .modules
            .first()
            .and_then(|m| m.spec.hidden.first())
            .copied()
            .unwrap_or(8);
        brain
            .check_against(
                &brain.kind,
                &specs_for_kind(&brain.kind, hidden).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        if side > 1 {
            return Err("invalid strategic side".into());
        }
        Ok(Self {
            brain,
            faction,
            side,
            sequence: 0,
            previous: [ResourceStats::default(); 2],
            previous_tick: 0,
            home,
            rejected_placements: Vec::new(),
        })
    }
    /// Reports the validator's decisions on this side's own orders. Call after every world step
    /// with `World::action_decisions`; decisions for the other side are ignored. A build rejected for
    /// placement (not for, say, a destroyed builder)
    /// defers that placement for a growing cooldown so the side tries other sites instead of
    /// repeating the order. Only the accept/reject outcome is used, never its reason text.
    pub fn observe_decisions(&mut self, tick: u64, decisions: &[ActionDecision]) {
        for decision in decisions {
            if decision.queued.side != self.side
                || decision.result != Err(ActionRejection::PlacementRejected)
            {
                continue;
            }
            let StrategyAction::Build {
                definition,
                position,
                ..
            } = &decision.queued.action
            else {
                continue;
            };
            let existing = self.rejected_placements.iter_mut().find(|entry| {
                entry.definition == *definition
                    && entry.position.distance_squared(*position)
                        <= REJECTED_PLACEMENT_RADIUS * REJECTED_PLACEMENT_RADIUS
            });
            let entry = match existing {
                Some(entry) => entry,
                None => {
                    self.rejected_placements.push(RejectedPlacement {
                        definition: definition.clone(),
                        position: *position,
                        rejections: 0,
                        retry_at: 0,
                    });
                    self.rejected_placements.last_mut().expect("just pushed")
                }
            };
            entry.rejections = entry.rejections.saturating_add(1);
            let doublings = (entry.rejections - 1).min(REJECTED_PLACEMENT_MAX_DOUBLINGS);
            entry.retry_at = tick + (REJECTED_PLACEMENT_BASE_TICKS << doublings);
        }
    }
    pub fn actions(&mut self, world: &World) -> Result<Vec<QueuedAction>, String> {
        let observation = world.observation(self.side).map_err(|e| e.to_string())?;
        let known_grid = known_occupancy(world, &observation)?;
        let mut router = GridRouter::new();
        let owned: Vec<_> = world
            .entities
            .iter()
            .filter(|(_, e)| e.owner == self.side && e.is_active())
            .collect();
        let enemies: Vec<_> = observation
            .entities
            .iter()
            .filter(|e| e.side != self.side && e.visibility != ObservationVisibility::Remembered)
            .collect();
        let team = &world.teams[self.side as usize];
        let dt = (world.tick.saturating_sub(self.previous_tick) as f64 / 30.0).max(1.0 / 30.0);
        let pools = [&team.economy.metal, &team.economy.energy];
        let signals: Vec<_> = pools
            .iter()
            .zip(self.previous)
            .map(|(p, last)| {
                let spent = (p.stats.spent - last.spent) / dt;
                resource_signals(
                    p.stored,
                    p.capacity,
                    p.income_per_second,
                    spent,
                    spent + (p.stats.unmet - last.unmet) / dt,
                    (p.stats.overflow - last.overflow) / dt,
                )
            })
            .collect();
        self.previous = [pools[0].stats, pools[1].stats];
        self.previous_tick = world.tick;
        let metal = signals[0];
        let energy = signals[1];
        let count = |name: &str| {
            owned
                .iter()
                .filter(|(_, e)| {
                    world
                        .catalog
                        .get(e.definition)
                        .is_some_and(|d| d.name == name)
                })
                .count()
        };
        let builders: Vec<_> = owned
            .iter()
            .filter(|(_, e)| {
                world
                    .catalog
                    .get(e.definition)
                    .is_some_and(|d| d.strategic.is_builder && !d.strategic.is_factory)
            })
            .collect();
        let idle: Vec<_> = builders
            .iter()
            .filter(|(id, _)| world.builder_task(*id).is_none())
            .collect();
        let idle_fraction = idle.len() as f32 / builders.len().max(1) as f32;
        let mut proposed = Vec::new();
        let mut claimed_sites = std::collections::BTreeSet::new();
        for (builder_id, builder) in idle {
            let builder_def = world.catalog.get(builder.definition).unwrap();
            let mut candidates = Vec::new();
            for (kind, name) in [
                (0, &self.faction.mex),
                (1, &self.faction.power),
                (2, &self.faction.factory),
            ] {
                if !builder_def.strategic.build_options.contains(name) {
                    continue;
                }
                let Some((_, definition)) = world.catalog.get_by_name(name) else {
                    continue;
                };
                // One active factory is sufficient for the initial curriculum.
                if kind == 2 && count(name) > 0 {
                    continue;
                }
                let position = if kind == 0 {
                    let mut sites: Vec<_> = observation
                        .resource_sites
                        .iter()
                        .filter(|s| {
                            !s.geothermal
                                && s.occupied_by.is_none()
                                && !claimed_sites.contains(&s.id)
                                && !placement_is_deferred(
                                    &self.rejected_placements,
                                    world.tick,
                                    name,
                                    s.position,
                                )
                        })
                        .filter(|s| {
                            !world.entities.iter().any(|(_, e)| {
                                e.owner == self.side
                                    && e.position.distance_squared(s.position) < 64.0 * 64.0
                                    && world
                                        .catalog
                                        .get(e.definition)
                                        .is_some_and(|d| d.strategic.extracts_metal > 0.0)
                            })
                        })
                        .collect();
                    sites.sort_by(|a, b| {
                        builder
                            .position
                            .distance_squared(a.position)
                            .total_cmp(&builder.position.distance_squared(b.position))
                    });
                    sites.into_iter().take(16).find_map(|s| {
                        let pos = known_placement(world, &known_grid, definition, s.position)?;
                        let class = builder_def.strategic.movement_class.as_deref()?;
                        if builder.position.distance(pos) > builder_def.strategic.build_distance {
                            let route = router.find_route(
                                &world.map,
                                &known_grid,
                                &RouteRequest::new(builder.position, pos, class)
                                    .with_radius(builder_def.movement.radius)
                                    .with_allow_partial(false),
                            );
                            if !matches!(
                                route.status,
                                RouteStatus::Complete | RouteStatus::AlreadyAtGoal
                            ) {
                                return None;
                            }
                        }
                        Some((pos, Some(s.id), s.metal))
                    })
                } else {
                    // Local placement checks occur only in identified own territory.
                    let mut found = None;
                    'placement: for radius in [96.0, 160.0, 224.0, 320.0, 448.0] {
                        for direction in 0..8 {
                            let a = direction as f32 * std::f32::consts::TAU / 8.0;
                            let position =
                                builder.position + Vec2::new(a.cos() * radius, a.sin() * radius);
                            if position.x < 32.0
                                || position.z < 32.0
                                || position.x > world.map.width - 32.0
                                || position.z > world.map.depth - 32.0
                            {
                                continue;
                            }
                            let visible = owned.iter().any(|(_, e)| {
                                e.position.distance_squared(position) < 500.0 * 500.0
                            });
                            if !visible {
                                continue;
                            }
                            if let Some(position) =
                                known_placement(world, &known_grid, definition, position)
                            {
                                if placement_is_deferred(
                                    &self.rejected_placements,
                                    world.tick,
                                    name,
                                    position,
                                ) {
                                    continue;
                                }
                                found = Some((position, None, 0.0));
                                break 'placement;
                            }
                        }
                    }
                    found
                };
                let Some((position, site, yield_value)) = position else {
                    continue;
                };
                let threat = enemies
                    .iter()
                    .filter(|e| e.position.distance_squared(position) < 700.0 * 700.0)
                    .count() as f32;
                let input = [
                    metal.fill,
                    energy.fill,
                    1.0 - metal.fill,
                    1.0 - energy.fill,
                    metal.unmet,
                    energy.unmet,
                    metal.overflow,
                    energy.overflow,
                    (kind == 0) as u8 as f32,
                    (kind == 1) as u8 as f32,
                    (kind == 2) as u8 as f32,
                    bounded(count(name) as f64, 3.0),
                    bounded(builder.position.distance(position) as f64, 1000.0),
                    bounded(definition.metal_cost as f64, 500.0),
                    threat.min(1.0),
                    (count(&self.faction.factory) == 0) as u8 as f32,
                    idle_fraction,
                    bounded(yield_value as f64, 2.0),
                    metal.requested_net,
                    energy.requested_net,
                ];
                candidates.push((
                    score(&self.brain.modules[0], &input),
                    name.clone(),
                    position,
                    site,
                ));
            }
            if let Some((_, name, position, site)) =
                candidates.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))
            {
                if let Some(site) = site {
                    claimed_sites.insert(site);
                }
                proposed.push(StrategyAction::Build {
                    builder: *builder_id,
                    definition: name,
                    position,
                    facing: BuildFacing::South,
                    mode: OrderMode::Replace,
                });
            }
        }
        for (factory_id, factory) in owned.iter().filter(|(_, e)| {
            world
                .catalog
                .get(e.definition)
                .is_some_and(|d| d.strategic.is_factory)
        }) {
            if !matches!(world.factory_state(*factory_id), Some(FactoryState::Idle)) {
                continue;
            }
            let factory_def = world.catalog.get(factory.definition).unwrap();
            let mut candidates = Vec::new();
            for name in std::iter::once(&self.faction.builder).chain(self.faction.combat.iter()) {
                if !factory_def.strategic.build_options.contains(name) {
                    continue;
                }
                let Some((_, def)) = world.catalog.get_by_name(name) else {
                    continue;
                };
                let is_builder = name == &self.faction.builder;
                if is_builder && builders.len() >= 4 {
                    continue;
                }
                let input = [
                    metal.fill,
                    energy.fill,
                    1.0 - energy.fill,
                    bounded(builders.len() as f64, 3.0),
                    bounded(
                        owned
                            .iter()
                            .filter(|(_, entity)| {
                                world.catalog.get(entity.definition).is_some_and(|d| {
                                    d.strategic.is_mobile
                                        && !d.strategic.is_builder
                                        && !d.weapons.is_empty()
                                })
                            })
                            .count() as f64,
                        20.0,
                    ),
                    is_builder as u8 as f32,
                    (!is_builder) as u8 as f32,
                    bounded(def.metal_cost as f64, 500.0),
                    (!enemies.is_empty()) as u8 as f32,
                    bounded(count(&self.faction.factory) as f64, 3.0),
                    metal.requested_net,
                    energy.requested_net,
                ];
                let mut input = input.to_vec();
                if self.brain.kind == KIND_V2 {
                    let identified: Vec<_> = enemies
                        .iter()
                        .filter(|e| {
                            matches!(
                                e.visibility,
                                ObservationVisibility::LineOfSight | ObservationVisibility::Ghost
                            )
                        })
                        .filter_map(|e| {
                            e.definition
                                .as_deref()
                                .and_then(|n| world.catalog.get_by_name(n).map(|(_, d)| d))
                        })
                        .collect();
                    input.extend_from_slice(&super::strategic_sensors::extras(def, &identified));
                }
                candidates.push((score(&self.brain.modules[1], &input), name.clone()));
            }
            if let Some((_, name)) = candidates.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)) {
                proposed.push(StrategyAction::FactoryQueue {
                    factory: *factory_id,
                    operation: FactoryQueueOperation::Append {
                        item: FactoryQueueItem {
                            definition: name,
                            count: 1,
                        },
                    },
                });
            }
        }
        // Each small spatial group receives one mission; no global enemy centroid.
        let mut groups: std::collections::BTreeMap<(i32, i32), Vec<_>> =
            std::collections::BTreeMap::new();
        for (id, entity) in &owned {
            let def = world.catalog.get(entity.definition).unwrap();
            if def.strategic.is_mobile && !def.strategic.is_builder && !def.weapons.is_empty() {
                groups
                    .entry((
                        (entity.position.x / 600.0) as i32,
                        (entity.position.z / 600.0) as i32,
                    ))
                    .or_default()
                    .push((*id, *entity));
            }
        }
        for members in groups.values() {
            let center = members
                .iter()
                .fold(Vec2::default(), |sum, (_, e)| sum + e.position)
                * (1.0 / members.len() as f32);
            let own_health = members.iter().map(|(_, e)| e.health).sum::<f64>()
                / members
                    .iter()
                    .map(|(_, e)| world.catalog.get(e.definition).unwrap().health as f64)
                    .sum::<f64>()
                    .max(1.0);
            let own_value = members
                .iter()
                .map(|(_, e)| world.catalog.get(e.definition).unwrap().resource_value() as f64)
                .sum::<f64>();
            let mut targets = Vec::new();
            for target in enemies.iter().filter(|e| {
                matches!(
                    e.visibility,
                    ObservationVisibility::LineOfSight | ObservationVisibility::Ghost
                )
            }) {
                let Some(def) = target
                    .definition
                    .as_deref()
                    .and_then(|name| world.catalog.get_by_name(name).map(|(_, d)| d))
                else {
                    continue;
                };
                let input = [
                    bounded(center.distance(target.position) as f64, 1000.0),
                    target.health_fraction.unwrap_or(1.0),
                    def.strategic.is_factory as u8 as f32,
                    (def.strategic.extracts_metal > 0.0
                        || def.strategic.energy_make > 0.0
                        || def.strategic.energy_upkeep < 0.0) as u8 as f32,
                    def.strategic.is_mobile as u8 as f32,
                    own_health.clamp(0.0, 1.0) as f32,
                    bounded(own_value, 1000.0),
                    bounded(def.resource_value() as f64, 1000.0),
                    bounded(self.home.distance(target.position) as f64, 1000.0),
                    1.0,
                ];
                targets.push((score(&self.brain.modules[2], &input), *target));
            }
            if let Some((_, target)) = targets.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)) {
                proposed.push(StrategyAction::Attack {
                    entities: members.iter().map(|(id, _)| *id).collect(),
                    target: if target.visibility == ObservationVisibility::Ghost {
                        AttackTarget::Ground {
                            position: target.position,
                        }
                    } else {
                        AttackTarget::Entity { entity: target.id }
                    },
                    mode: OrderMode::Replace,
                });
            } else {
                // A reproducible known-map scouting route; no hidden enemy positions.
                let seed = (world.tick / 300) as usize + self.side as usize * 2;
                let sites: &[Vec2] = &[
                    Vec2::new(world.map.width * 0.5, world.map.depth * 0.5),
                    Vec2::new(world.map.width * 0.25, world.map.depth * 0.75),
                    Vec2::new(world.map.width * 0.75, world.map.depth * 0.25),
                ];
                proposed.push(StrategyAction::Move {
                    entities: members.iter().map(|(id, _)| *id).collect(),
                    destination: sites[seed % sites.len()],
                    mode: OrderMode::Replace,
                });
            }
        }
        Ok(proposed
            .into_iter()
            .map(|action| {
                self.sequence += 1;
                QueuedAction {
                    side: self.side,
                    sequence: self.sequence,
                    apply_at_tick: world.tick,
                    action,
                }
            })
            .collect())
    }
}

pub struct Prepared {
    config: TrainConfig,
    maps: Vec<Arc<CompiledMap>>,
    catalog: Arc<WorldCatalog>,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrialReport {
    pub starting_positions: [Vec2; 2],
    pub map: String,
    pub faction: String,
    pub learner_side: u8,
    pub seed: u64,
    pub layout_seed: u64,
    pub fitness: f64,
    pub projects_completed: u64,
    pub units_produced: u64,
    pub spent_metal: f64,
    pub metal_income: f64,
    pub energy_income: f64,
    pub metal_overflow: f64,
    pub energy_overflow: f64,
    pub unmet_energy: f64,
    pub army_value: f64,
    pub economy_value: f64,
    pub actions_rejected: u64,
    pub rejection_reasons: std::collections::BTreeMap<String, u64>,
    pub state_hash: u64,
    pub reward_stats: super::strategic_reward::RewardStats,
    pub reward_components: super::strategic_reward::RewardComponents,
    pub telemetry: super::strategic_telemetry::TrialTelemetry,
}
#[derive(Clone, Debug, Serialize)]
pub struct Evaluation {
    pub fitness: f64,
    pub trials: Vec<TrialReport>,
}
impl Prepared {
    pub fn new(config: &TrainConfig, catalog: &UnitCatalog) -> Result<Self, String> {
        config.validate()?;
        let mut maps = Vec::new();
        for spec in &config.maps {
            let (map, _) = CompiledMap::load_path(&spec.map, MapLoadOptions::default())
                .map_err(|e| e.to_string())?;
            for start in spec.starts {
                if !map.is_passable(&spec.movement_class, start.x, start.z) {
                    return Err(format!("blocked start on {}", spec.map.display()));
                }
            }
            maps.push(Arc::new(map));
        }
        for faction in &config.factions {
            for name in [
                &faction.builder,
                &faction.mex,
                &faction.power,
                &faction.factory,
            ]
            .into_iter()
            .chain(faction.combat.iter())
            {
                let d = catalog
                    .get(name)
                    .ok_or_else(|| format!("missing faction unit {name}"))?;
                if d.health <= 0.0 || d.strategic.build_time <= 0.0 {
                    return Err(format!("{name} lacks health or construction data"));
                }
            }
            let builder = catalog.get(&faction.builder).unwrap();
            let factory = catalog.get(&faction.factory).unwrap();
            if !builder.strategic.is_builder
                || builder.strategic.is_factory
                || !factory.strategic.is_factory
                || faction.combat.is_empty()
            {
                return Err(format!(
                    "{} requires a mobile builder, factory and combat products",
                    faction.name
                ));
            }
            for name in [&faction.mex, &faction.power, &faction.factory] {
                if !builder.strategic.build_options.contains(name) {
                    return Err(format!("{} cannot build {name}", faction.builder));
                }
            }
            for name in std::iter::once(&faction.builder).chain(faction.combat.iter()) {
                if !factory.strategic.build_options.contains(name) {
                    return Err(format!("{} cannot produce {name}", faction.factory));
                }
            }
        }
        let shared = WorldCatalog::new(catalog.units.values().cloned().collect())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            config: config.clone(),
            maps,
            catalog: Arc::new(shared),
        })
    }
    pub fn rollout(
        &self,
        brain: &Brain,
        map_index: usize,
        faction_index: usize,
        learner_side: u8,
        seed: u64,
    ) -> Result<TrialReport, String> {
        let opponent = template_for_kind(&self.config.brain_kind, self.config.hidden)
            .map_err(|e| e.to_string())?;
        self.rollout_against(
            brain,
            &opponent,
            map_index,
            faction_index,
            learner_side,
            seed,
        )
    }

    /// Runs a mirrored-trial component against an explicit historical brain.
    pub fn rollout_against(
        &self,
        brain: &Brain,
        opponent: &Brain,
        map_index: usize,
        faction_index: usize,
        learner_side: u8,
        seed: u64,
    ) -> Result<TrialReport, String> {
        self.rollout_against_sample(
            brain,
            opponent,
            map_index,
            faction_index,
            learner_side,
            seed,
            seed,
        )
    }
    pub fn rollout_against_sample(
        &self,
        brain: &Brain,
        opponent: &Brain,
        map_index: usize,
        faction_index: usize,
        learner_side: u8,
        seed: u64,
        layout_seed: u64,
    ) -> Result<TrialReport, String> {
        if learner_side > 1
            || map_index >= self.maps.len()
            || faction_index >= self.config.factions.len()
        {
            return Err("strategic rollout side, map or faction index is out of bounds".into());
        }
        let config = &self.config;
        let spec = &config.maps[map_index];
        let faction = &config.factions[faction_index];
        let mut world = World::new_with_static_features(
            Arc::clone(&self.maps[map_index]),
            Arc::clone(&self.catalog),
            WorldConfig {
                starting_resources: Resources::new(config.starting_metal, config.starting_energy),
                base_storage: Resources::new(
                    config.starting_metal.max(1000.0),
                    config.starting_energy.max(1000.0),
                ),
                base_income: Resources::new(config.base_metal_income, config.base_energy_income),
                occupancy_cell_size: 16.0,
                observation_policy: ObservationPolicy::Filtered {
                    line_of_sight_range: 600.0,
                    radar_range: 0.0,
                    radar_position_resolution: 96.0,
                    remember_last_seen: true,
                },
                ..WorldConfig::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let builder = self
            .catalog
            .id(&faction.builder)
            .ok_or("starting builder missing")?;
        let mut starts = spec.starts;
        for side in 0..2 {
            let mut rng = super::rng::Rng::for_stream(layout_seed, side as u64);
            for _ in 0..8 {
                let candidate = spec.starts[side as usize]
                    + Vec2::new(
                        rng.signed() * config.layout_jitter,
                        rng.signed() * config.layout_jitter,
                    );
                if candidate.x > 32.0
                    && candidate.z > 32.0
                    && candidate.x < world.map.width - 32.0
                    && candidate.z < world.map.depth - 32.0
                    && world
                        .map
                        .is_passable(&spec.movement_class, candidate.x, candidate.z)
                {
                    starts[side as usize] = candidate;
                    break;
                }
            }
            world
                .spawn_active(side, builder, starts[side as usize], BuildFacing::South)
                .map_err(|e| e.to_string())?;
        }
        if config.combat {
            world
                .enable_combat(seed, &spec.movement_class, spec.cell_size)
                .map_err(|e| e.to_string())?;
        }
        let incumbent = opponent;
        let mut policies = [
            StrategicPolicy::new(
                if learner_side == 0 {
                    brain.clone()
                } else {
                    incumbent.clone()
                },
                faction.clone(),
                0,
                starts[0],
            )?,
            StrategicPolicy::new(
                if learner_side == 1 {
                    brain.clone()
                } else {
                    incumbent.clone()
                },
                faction.clone(),
                1,
                starts[1],
            )?,
        ];
        let mut ledger = super::strategic_telemetry::TrialLedger::new(&world);
        let mut completed = 0;
        let mut rejected = 0;
        let mut rejection_reasons = std::collections::BTreeMap::new();
        for _ in 0..config.max_ticks {
            if world.tick % config.cadence_ticks == 0 {
                for policy in &mut policies {
                    let actions = policy.actions(&world)?;
                    world.enqueue_actions(actions);
                }
            }
            world.step();
            ledger.observe(&world);
            for policy in &mut policies {
                policy.observe_decisions(world.tick, &world.action_decisions);
            }
            rejected += world
                .action_decisions
                .iter()
                .filter(|d| d.queued.side == learner_side && d.result.is_err())
                .count() as u64;
            for decision in &world.action_decisions {
                if decision.queued.side == learner_side {
                    if let Err(reason) = decision.result {
                        let mut detail = reason.as_str().to_owned();
                        if let StrategyAction::Build {
                            builder,
                            definition,
                            position,
                            facing,
                            ..
                        } = &decision.queued.action
                        {
                            if let Some(rejection) = world
                                .preview_build(
                                    learner_side,
                                    *builder,
                                    definition,
                                    *position,
                                    *facing,
                                )
                                .rejection
                            {
                                detail.push_str(": ");
                                detail.push_str(&rejection);
                            }
                        }
                        *rejection_reasons.entry(detail).or_insert(0) += 1;
                    }
                }
            }
            completed+=world.events.iter().filter(|event|matches!(event,WorldEvent::ProjectCompleted {entity,..} if world.entities.get(*entity).is_some_and(|e|e.owner==learner_side))).count() as u64;
        }
        let metric = world
            .strategic_metrics(learner_side)
            .map_err(|e| e.to_string())?;
        let team = &world.teams[learner_side as usize];
        let econ = &team.economy;
        let army = metric.get(StrategicMetric::ArmyValue);
        let economy = metric.get(StrategicMetric::EconomyValue);
        let useful = surviving_value(&world, learner_side);
        let enemy_useful = surviving_value(&world, 1 - learner_side);
        let duration = config.max_ticks as f64 / 30.0;
        let reward_stats = super::strategic_reward::RewardStats {
            own_surviving_value: useful,
            enemy_surviving_value: enemy_useful,
            metal_income_integral: metric.get(StrategicMetric::MetalIncomeIntegral),
            units_produced: team.metrics.units_produced,
            production_value: ledger.production_value[learner_side as usize],
            energy_overflow: econ.energy.stats.overflow,
            metal_overflow: econ.metal.stats.overflow,
            stalled_build_power_seconds: team.metrics.stalled_build_power_seconds,
            duration_seconds: duration,
        };
        let reward_components = super::strategic_reward::score(&reward_stats, &config.reward)?;
        let fitness = reward_components.total;
        let telemetry = ledger.summary(&world, learner_side, config.combat);
        Ok(TrialReport {
            starting_positions: starts,
            map: world.map.map_name.clone(),
            faction: faction.name.clone(),
            learner_side,
            seed,
            layout_seed,
            fitness,
            projects_completed: completed,
            units_produced: team.metrics.units_produced,
            spent_metal: econ.metal.stats.spent,
            metal_income: econ.metal.income_per_second,
            energy_income: econ.energy.income_per_second,
            metal_overflow: econ.metal.stats.overflow,
            energy_overflow: econ.energy.stats.overflow,
            unmet_energy: econ.energy.stats.unmet,
            army_value: army,
            economy_value: economy,
            actions_rejected: rejected,
            rejection_reasons,
            state_hash: world.state_hash(),
            reward_stats,
            reward_components,
            telemetry,
        })
    }
    pub fn evaluate(
        &self,
        brain: &Brain,
        seeds: usize,
        seed_base: u64,
    ) -> Result<Evaluation, String> {
        let opponent = template_for_kind(&self.config.brain_kind, self.config.hidden)
            .map_err(|e| e.to_string())?;
        self.evaluate_against(brain, &opponent, seeds, seed_base)
    }

    pub fn evaluate_on_layout(
        &self,
        brain: &Brain,
        opponent: Option<&Brain>,
        seeds: usize,
        seed_base: u64,
        layout_seed: u64,
    ) -> Result<Evaluation, String> {
        let default = template_for_kind(&self.config.brain_kind, self.config.hidden)
            .map_err(|e| e.to_string())?;
        self.evaluate_samples(
            brain,
            opponent.unwrap_or(&default),
            seeds,
            seed_base,
            Some(layout_seed),
        )
    }
    pub fn evaluate_against(
        &self,
        brain: &Brain,
        opponent: &Brain,
        seeds: usize,
        seed_base: u64,
    ) -> Result<Evaluation, String> {
        self.evaluate_samples(brain, opponent, seeds, seed_base, None)
    }
    fn evaluate_samples(
        &self,
        brain: &Brain,
        opponent: &Brain,
        seeds: usize,
        seed_base: u64,
        layout: Option<u64>,
    ) -> Result<Evaluation, String> {
        if brain.kind != self.config.brain_kind || opponent.kind != self.config.brain_kind {
            return Err("strategic evaluation brain kind differs from config".into());
        }
        if seeds == 0 {
            return Err("strategic evaluation needs at least one seed".into());
        }
        seed_base
            .checked_add((seeds - 1) as u64)
            .ok_or("strategic seed range overflows u64")?;
        let jobs: Vec<_> = (0..self.maps.len())
            .flat_map(|map| {
                (0..self.config.factions.len()).flat_map(move |faction| {
                    (0..2).flat_map(move |side| {
                        (0..seeds)
                            .map(move |offset| (map, faction, side, seed_base + offset as u64))
                    })
                })
            })
            .collect();
        let trials: Vec<TrialReport> = jobs
            .par_iter()
            .map(|&(map, faction, side, seed)| {
                self.rollout_against_sample(
                    brain,
                    opponent,
                    map,
                    faction,
                    side,
                    seed,
                    layout.unwrap_or(seed),
                )
            })
            .collect::<Result<_, _>>()?;
        Ok(Evaluation {
            fitness: trials.iter().map(|r| r.fitness).sum::<f64>() / trials.len().max(1) as f64,
            trials,
        })
    }
}

pub fn train(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    out: &Path,
) -> Result<serde_json::Value, String> {
    let prepared = Prepared::new(config, catalog)?;
    let base = if let Some(path) = &config.init_from {
        Brain::load(path).map_err(|e| e.to_string())?
    } else {
        template_for_kind(&config.brain_kind, config.hidden).map_err(|e| e.to_string())?
    };
    StrategicPolicy::new(
        base.clone(),
        config.factions[0].clone(),
        0,
        config.maps[0].starts[0],
    )?;
    let mask = base
        .trainable_mask(&config.train_modules)
        .map_err(|e| e.to_string())?;
    let mut es = EvolutionStrategy::new(base.flatten(&mask), config.es)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let write = |name: &str, value: &serde_json::Value| {
        std::fs::write(out.join(name), serde_json::to_string_pretty(value).unwrap())
            .map_err(|e| e.to_string())
    };
    write("train_config.json", &serde_json::to_value(config).unwrap())?;
    let started = std::time::Instant::now();
    let initial = prepared.evaluate(&base, config.validation_seeds, 1_000_000_000)?;
    let mut best = initial.clone();
    let mut latest = initial.clone();
    let mut best_generation = 0;
    base.save(&out.join("brain.initial.json"))
        .map_err(|e| e.to_string())?;
    base.save(&out.join("brain.best.json"))
        .map_err(|e| e.to_string())?;
    base.save(&out.join("brain.latest.json"))
        .map_err(|e| e.to_string())?;
    let mut history = String::new();
    for generation in 0..config.generations {
        let rollout_error = std::sync::Mutex::new(None);
        let stats = es.step_masked(&base.parameter_activity(&mask), |theta| {
            let candidate = base.with_flat(&mask, theta).expect("parameter shape");
            match prepared.evaluate(&candidate, config.train_seeds, 1000 + generation * 1000) {
                Ok(e) => e.fitness,
                Err(error) => {
                    *rollout_error.lock().unwrap() = Some(error);
                    -1e12
                }
            }
        })?;
        if let Some(error) = rollout_error.into_inner().unwrap() {
            return Err(format!("strategic candidate rollout failed: {error}"));
        }
        let candidate = base
            .with_flat(&mask, es.theta())
            .map_err(|e| e.to_string())?;
        latest = prepared.evaluate(&candidate, config.validation_seeds, 1_000_000_000)?;
        candidate
            .save(&out.join("brain.latest.json"))
            .map_err(|e| e.to_string())?;
        if latest.fitness > best.fitness {
            best = latest.clone();
            best_generation = generation + 1;
            candidate
                .save(&out.join("brain.best.json"))
                .map_err(|e| e.to_string())?;
        }
        let row = serde_json::json!({"generation":generation+1,"training":stats,"validation_fitness":latest.fitness,"best_fitness":best.fitness,"elapsed_seconds":started.elapsed().as_secs_f64()});
        eprintln!(
            "strategic generation {}: validation {:.5}, best {:.5}",
            generation + 1,
            latest.fitness,
            best.fitness
        );
        history.push_str(&serde_json::to_string(&row).unwrap());
        history.push('\n');
        std::fs::write(out.join("history.jsonl"), &history).map_err(|e| e.to_string())?;
    }
    let summary = serde_json::json!({"kind":KIND,"generations":config.generations,"best_generation":best_generation,
        "wall_seconds":started.elapsed().as_secs_f64(),"initial":initial,"best":best,"latest":latest,
        "origin":"authored from replay research; ES refinement in approximate MiniBAR; not imitation-fitted"});
    write("summary.json", &summary)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapSource;
    use crate::model::{MovementDef, StrategicDef, UnitDef};

    fn fixture() -> (TrainConfig, UnitCatalog, std::path::PathBuf) {
        let source: MapSource = serde_json::from_value(serde_json::json!({
            "source_version":1,"map_name":"strategic test","content_hash":"fixture",
            "width":512,"depth":512,"water_level":0,
            "heights":{"cols":2,"rows":2,"origin_x":0,"origin_z":0,"step_x":512,"step_z":512,"values":[10,10,10,10]},
            "passability":[{"movement_class":"bot2","cols":8,"rows":8,"cell_size_x":64,"cell_size_z":64,"passable":vec![true;64]}],
            "metal_nodes":[{"x":128,"y":10,"z":128,"yield_value":2},{"x":384,"y":10,"z":384,"yield_value":2}],
            "continuous_metal":null
        })).unwrap();
        let map = CompiledMap::compile(source).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "minibar-strategy-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace(':', "_")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.vsgmap");
        std::fs::write(&path, map.to_cache_bytes()).unwrap();
        let mut units = std::collections::BTreeMap::new();
        for name in ["builder", "mex", "power", "factory", "soldier"] {
            let mobile = matches!(name, "builder" | "soldier");
            let mut def = UnitDef {
                name: name.into(),
                health: 500.0,
                metal_cost: 10.0,
                energy_cost: 10.0,
                movement: MovementDef {
                    max_speed: 80.0,
                    acceleration: 1000.0,
                    braking: 1000.0,
                    turn_rate: 360.0,
                    radius: 6.0,
                    ..MovementDef::default()
                },
                strategic: StrategicDef {
                    build_time: 50.0,
                    build_distance: 160.0,
                    build_speed: 200.0,
                    is_mobile: mobile,
                    is_building: !mobile,
                    is_builder: name == "builder",
                    is_factory: name == "factory",
                    can_assist: true,
                    can_be_assisted: true,
                    footprint: [2, 2],
                    movement_class: mobile.then(|| "bot2".into()),
                    ..StrategicDef::default()
                },
                ..UnitDef::default()
            };
            if name == "builder" {
                def.strategic.build_options = vec!["mex".into(), "power".into(), "factory".into()];
            }
            if name == "factory" {
                def.strategic.build_options = vec!["builder".into(), "soldier".into()];
            }
            if name == "mex" {
                def.strategic.extracts_metal = 1.0;
            }
            if name == "power" {
                def.strategic.energy_make = 20.0;
            }
            units.insert(name.into(), def);
        }
        let config = TrainConfig {
            max_ticks: 600,
            cadence_ticks: 15,
            combat: false,
            maps: vec![MapSpec {
                map: path,
                movement_class: "bot2".into(),
                cell_size: 32.0,
                starts: [Vec2::new(64.0, 64.0), Vec2::new(448.0, 448.0)],
            }],
            factions: vec![FactionSpec {
                name: "fixture".into(),
                builder: "builder".into(),
                mex: "mex".into(),
                power: "power".into(),
                factory: "factory".into(),
                combat: vec!["soldier".into()],
            }],
            ..TrainConfig::default()
        };
        (
            config,
            UnitCatalog {
                units,
                metadata: Default::default(),
            },
            dir,
        )
    }

    #[test]
    fn surviving_value_counts_overlapping_roles_once_and_tracks_damage_and_loss() {
        let (config, mut catalog, _) = fixture();
        for name in ["builder", "factory"] {
            catalog
                .units
                .get_mut(name)
                .unwrap()
                .strategic
                .energy_storage = 50.0;
        }
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig::default(),
        )
        .unwrap();
        let builder = world
            .spawn_active(
                0,
                prepared.catalog.id("builder").unwrap(),
                Vec2::new(64.0, 64.0),
                BuildFacing::South,
            )
            .unwrap();
        let factory = world
            .spawn_active(
                0,
                prepared.catalog.id("factory").unwrap(),
                Vec2::new(224.0, 224.0),
                BuildFacing::South,
            )
            .unwrap();
        world
            .spawn_active(
                1,
                prepared.catalog.id("builder").unwrap(),
                Vec2::new(448.0, 448.0),
                BuildFacing::South,
            )
            .unwrap();
        let builder_def = prepared.catalog.get_by_name("builder").unwrap().1;
        let builder_value = builder_def.resource_value() as f64;
        let factory_value = prepared
            .catalog
            .get_by_name("factory")
            .unwrap()
            .1
            .resource_value() as f64;
        assert_eq!(
            surviving_value(&world, 0),
            builder_value + factory_value,
            "incidental storage must not double a role's replacement value"
        );
        world.entities.get_mut(builder).unwrap().health = builder_def.health as f64 / 2.0;
        assert_eq!(
            surviving_value(&world, 0),
            builder_value / 2.0 + factory_value
        );
        world.entities.get_mut(factory).unwrap().role = crate::world::EntityRole::Feature;
        assert_eq!(
            surviving_value(&world, 0),
            builder_value / 2.0,
            "features do not count as surviving assets"
        );
        assert!(world.destroy_entity(builder));
        assert_eq!(surviving_value(&world, 0), 0.0);
        assert_eq!(surviving_value(&world, 1), builder_value);
    }

    #[test]
    fn baseline_roundtrip_is_trainable_and_executes_real_construction() {
        let (config, catalog, dir) = fixture();
        let brain = template(8).unwrap();
        brain.save(&dir.join("baseline.json")).unwrap();
        let loaded = Brain::load(&dir.join("baseline.json")).unwrap();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let report = prepared.rollout(&loaded, 0, 0, 0, 123).unwrap();
        let again = prepared.rollout(&loaded, 0, 0, 0, 123).unwrap();
        assert_eq!(report.state_hash, again.state_hash);
        assert!(report.projects_completed > 0, "{report:?}");
        assert!(report.units_produced > 0, "{report:?}");
        assert!(report.spent_metal > 0.0, "{report:?}");
        let mask = loaded.trainable_mask(&[]).unwrap();
        assert!(
            EvolutionStrategy::new(
                loaded.flatten(&mask),
                EsConfig {
                    population: 2,
                    ..EsConfig::default()
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn demand_sensor_separates_starvation_from_actual_spending() {
        let input = resource_signals(0.5, 1000.0, 222.5, 222.4, 299.0, 0.0);
        assert!(input.unmet > 0.25);
        assert!(input.requested_net < 0.0);
        assert!(input.fill < 0.001);
    }

    #[test]
    fn explicit_authored_opponent_matches_default_and_mirrors_sides() {
        let (config, catalog, _) = fixture();
        let brain = template(config.hidden).unwrap();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let baseline = prepared.evaluate(&brain, 1, 123).unwrap();
        let explicit = prepared.evaluate_against(&brain, &brain, 1, 123).unwrap();
        assert_eq!(
            serde_json::to_value(&baseline).unwrap(),
            serde_json::to_value(&explicit).unwrap()
        );
        assert_eq!(
            explicit
                .trials
                .iter()
                .map(|trial| trial.learner_side)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        let direct = prepared
            .rollout_against(&brain, &brain, 0, 0, 0, 123)
            .unwrap();
        assert_eq!(direct.state_hash, baseline.trials[0].state_hash);
    }

    #[test]
    fn economic_weights_change_investment_order() {
        let mut brain = template(8).unwrap();
        let mut power = [0.0; ECON_INPUTS.len()];
        power[ECON_INPUTS
            .iter()
            .position(|s| *s == "candidate_power")
            .unwrap()] = 1.0;
        let mut mex = power;
        mex[ECON_INPUTS
            .iter()
            .position(|s| *s == "candidate_power")
            .unwrap()] = 0.0;
        mex[ECON_INPUTS
            .iter()
            .position(|s| *s == "candidate_mex")
            .unwrap()] = 1.0;
        let before = score(&brain.modules[0], &mex) - score(&brain.modules[0], &power);
        let module = &mut brain.modules[0];
        module.net.params_mut().fill(0.0);
        wire(module, 0, "candidate_power", 5.0);
        feed(module, 0, 0, 5.0);
        let after = score(&brain.modules[0], &mex) - score(&brain.modules[0], &power);
        assert!(before > after && after < 0.0);
    }

    #[test]
    fn requested_net_flow_reaches_the_investment_network() {
        let brain = template(8).unwrap();
        let mut surplus = [0.0; ECON_INPUTS.len()];
        surplus[ECON_INPUTS
            .iter()
            .position(|s| *s == "candidate_power")
            .unwrap()] = 1.0;
        let index = ECON_INPUTS.iter().position(|s| *s == "energy_net").unwrap();
        surplus[index] = 1.0;
        let mut deficit = surplus;
        deficit[index] = -1.0;
        assert!(score(&brain.modules[0], &deficit) > score(&brain.modules[0], &surplus));
    }

    #[test]
    fn skips_a_known_blocked_mex_and_executes_weight_selected_investment() {
        let (config, catalog, _) = fixture();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig::default(),
        )
        .unwrap();
        world
            .spawn_active(
                0,
                prepared.catalog.id("builder").unwrap(),
                Vec2::new(64.0, 64.0),
                BuildFacing::South,
            )
            .unwrap();
        world
            .spawn_active(
                0,
                prepared.catalog.id("power").unwrap(),
                Vec2::new(128.0, 128.0),
                BuildFacing::South,
            )
            .unwrap();
        let mut brain = template(8).unwrap();
        let module = &mut brain.modules[0];
        module.net.params_mut().fill(0.0);
        wire(module, 0, "candidate_mex", 5.0);
        feed(module, 0, 0, 5.0);
        let mut policy =
            StrategicPolicy::new(brain, config.factions[0].clone(), 0, Vec2::new(64.0, 64.0))
                .unwrap();
        let actions = policy.actions(&world).unwrap();
        assert!(
            matches!(&actions[0].action,StrategyAction::Build {definition,position,..}
            if definition=="mex" && position.distance(Vec2::new(384.0,384.0))<16.0)
        );
        world.enqueue_actions(actions);
        world.step();
        assert!(
            world
                .events
                .iter()
                .any(|e| matches!(e, WorldEvent::ProjectOrdered { .. }))
        );
    }

    #[test]
    fn rejects_a_catalog_with_missing_build_relationships() {
        let (config, mut catalog, _) = fixture();
        catalog
            .units
            .get_mut("builder")
            .unwrap()
            .strategic
            .build_options
            .clear();
        assert!(
            Prepared::new(&config, &catalog)
                .err()
                .unwrap()
                .contains("cannot build")
        );
    }

    #[test]
    fn policy_footprints_match_authoritative_placement_cells() {
        let (config, mut catalog, _) = fixture();
        catalog
            .units
            .get_mut("power")
            .unwrap()
            .movement
            .blocks_movement = true;
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig {
                occupancy_cell_size: 32.0,
                ..WorldConfig::default()
            },
        )
        .unwrap();
        let builder = world
            .spawn_active(
                0,
                prepared.catalog.id("builder").unwrap(),
                Vec2::new(200.0, 200.0),
                BuildFacing::South,
            )
            .unwrap();
        world
            .spawn_active(
                0,
                prepared.catalog.id("power").unwrap(),
                Vec2::new(256.0, 256.0),
                BuildFacing::South,
            )
            .unwrap();
        let grid = known_occupancy(&world, &world.observation(0).unwrap()).unwrap();
        let power = prepared.catalog.get_by_name("power").unwrap().1;
        for x in (128..384).step_by(16) {
            for z in (128..384).step_by(16) {
                let position = Vec2::new(x as f32, z as f32);
                if let Some(snapped) = known_placement(&world, &grid, power, position) {
                    let preview =
                        world.preview_build(0, builder, "power", snapped, BuildFacing::South);
                    assert!(
                        preview.valid,
                        "policy accepted {position:?} -> {snapped:?}: {preview:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn disclosed_extractors_reserve_their_resource_area_in_policy_grid() {
        let (config, mut catalog, _) = fixture();
        catalog
            .units
            .get_mut("mex")
            .unwrap()
            .strategic
            .extract_range = 120.0;
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig::default(),
        )
        .unwrap();
        world
            .spawn_active(
                0,
                prepared.catalog.id("mex").unwrap(),
                Vec2::new(128.0, 128.0),
                BuildFacing::South,
            )
            .unwrap();
        let observation = world.observation(0).unwrap();
        let grid = known_occupancy(&world, &observation).unwrap();
        let mex = prepared.catalog.get_by_name("mex").unwrap().1;
        // The second footprint is clear but draws on the same discrete spot.
        assert!(known_placement(&world, &grid, mex, Vec2::new(208.0, 128.0)).is_none());
    }

    /// An enemy extractor outside line of sight hides a metal spot's occupant, so the observation
    /// rightly reports it free. The world rejects the order, the builder never moves, and the same
    /// proposal used to repeat on every decision (91 rejections in the retained champion's validation).
    #[test]
    fn a_rejected_site_is_not_re_proposed_every_decision() {
        let (config, catalog, _) = fixture();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new_with_static_features(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig {
                occupancy_cell_size: 16.0,
                observation_policy: ObservationPolicy::Filtered {
                    line_of_sight_range: 100.0,
                    radar_range: 0.0,
                    radar_position_resolution: 96.0,
                    remember_last_seen: true,
                },
                ..WorldConfig::default()
            },
        )
        .unwrap();
        // The builder is nearer to the hidden node (384,384) than to the free one (128,128).
        world
            .spawn_active(
                0,
                prepared.catalog.id("builder").unwrap(),
                Vec2::new(330.0, 200.0),
                BuildFacing::South,
            )
            .unwrap();
        world
            .spawn_active(
                1,
                prepared.catalog.id("mex").unwrap(),
                Vec2::new(384.0, 384.0),
                BuildFacing::South,
            )
            .unwrap();
        let hidden = world.observation(0).unwrap();
        assert!(
            hidden
                .resource_sites
                .iter()
                .all(|site| site.occupied_by.is_none()),
            "the enemy extractor must stay undisclosed"
        );
        let mut policy = StrategicPolicy::new(
            template(8).unwrap(),
            config.factions[0].clone(),
            0,
            Vec2::new(330.0, 200.0),
        )
        .unwrap();
        let (mut at_hidden, mut rejected) = (0, 0);
        for tick in 0..3600u64 {
            if tick % 30 == 0 {
                let actions = policy.actions(&world).unwrap();
                world.enqueue_actions(actions);
            }
            world.step();
            policy.observe_decisions(world.tick, &world.action_decisions);
            for decision in &world.action_decisions {
                if decision.queued.side != 0 {
                    continue;
                }
                if let StrategyAction::Build {
                    definition,
                    position,
                    ..
                } = &decision.queued.action
                {
                    if definition == "mex" && position.distance(Vec2::new(384.0, 384.0)) < 40.0 {
                        at_hidden += 1;
                        rejected += decision.result.is_err() as u32;
                    }
                }
            }
        }
        assert!(
            at_hidden >= 1,
            "the scenario must tempt the policy with the hidden site"
        );
        assert!(
            rejected >= 1,
            "the hidden occupant must make the world reject the order"
        );
        assert!(
            rejected <= 2,
            "{rejected} identical rejected orders for one hidden-blocked site"
        );
    }

    fn build_decision(side: u8, result: Result<(), ActionRejection>) -> ActionDecision {
        ActionDecision {
            queued: QueuedAction {
                side,
                sequence: 1,
                apply_at_tick: 0,
                action: StrategyAction::Build {
                    builder: crate::entity::EntityId::from_raw(1),
                    definition: "mex".into(),
                    position: Vec2::new(384.0, 384.0),
                    facing: BuildFacing::South,
                    mode: OrderMode::Replace,
                },
            },
            result,
        }
    }

    #[test]
    fn only_own_placement_rejections_defer_a_site_with_growing_cooldown() {
        let (config, _, _) = fixture();
        let mut policy = StrategicPolicy::new(
            template(8).unwrap(),
            config.factions[0].clone(),
            0,
            Vec2::new(0.0, 0.0),
        )
        .unwrap();
        let site = Vec2::new(390.0, 380.0);
        // The other side's rejection, an accepted order and a non-placement rejection change nothing.
        policy.observe_decisions(
            100,
            &[
                build_decision(1, Err(ActionRejection::PlacementRejected)),
                build_decision(0, Ok(())),
                build_decision(0, Err(ActionRejection::EntityDestroyed)),
            ],
        );
        assert!(!placement_is_deferred(
            &policy.rejected_placements,
            100,
            "mex",
            site
        ));
        // Base cooldown, then doubling, capped at eight times the base.
        let mut tick = 1000;
        for expected in [900, 1800, 3600, 7200, 7200] {
            policy.observe_decisions(
                tick,
                &[build_decision(0, Err(ActionRejection::PlacementRejected))],
            );
            assert!(placement_is_deferred(
                &policy.rejected_placements,
                tick + expected - 1,
                "mex",
                site
            ));
            assert!(!placement_is_deferred(
                &policy.rejected_placements,
                tick + expected,
                "mex",
                site
            ));
            tick += expected;
        }
        assert_eq!(
            policy.rejected_placements.len(),
            1,
            "nearby rejections are the same site"
        );
    }

    #[test]
    fn a_deferred_placement_is_specific_to_definition_and_place() {
        let rejected = [RejectedPlacement {
            definition: "mex".into(),
            position: Vec2::new(100.0, 100.0),
            rejections: 1,
            retry_at: 500,
        }];
        let near = Vec2::new(120.0, 110.0);
        assert!(placement_is_deferred(&rejected, 499, "mex", near));
        assert!(!placement_is_deferred(&rejected, 500, "mex", near));
        assert!(!placement_is_deferred(&rejected, 0, "power", near));
        assert!(!placement_is_deferred(
            &rejected,
            0,
            "mex",
            Vec2::new(200.0, 100.0)
        ));
    }

    #[test]
    fn evaluation_rejects_zero_seeds() {
        let (config, catalog, _) = fixture();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        assert!(prepared.evaluate(&template(8).unwrap(), 0, 1000).is_err());
        assert!(
            prepared
                .evaluate(&upgrade_v1_to_v2(&template(8).unwrap()).unwrap(), 1, 1000)
                .is_err()
        );
    }

    #[test]
    fn strategic_training_and_validation_use_distinct_seeded_layouts() {
        let (config, catalog, _) = fixture();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let brain = template(8).unwrap();
        let a = prepared.rollout(&brain, 0, 0, 0, 1000).unwrap();
        let b = prepared.rollout(&brain, 0, 0, 0, 1_000_000_000).unwrap();
        assert_ne!(a.starting_positions, b.starting_positions);
    }

    #[test]
    fn short_training_saves_loadable_checkpoints_and_actual_trial_metrics() {
        let (mut config, catalog, dir) = fixture();
        config.generations = 1;
        config.es.population = 2;
        let out = dir.join("training");
        let summary = train(&config, &catalog, &out).unwrap();
        assert_eq!(summary["generations"], 1);
        assert!(
            summary["initial"]["trials"][0]["projects_completed"]
                .as_u64()
                .unwrap()
                > 0
        );
        for name in ["brain.initial.json", "brain.best.json", "brain.latest.json"] {
            let brain = Brain::load(&out.join(name)).unwrap();
            assert_eq!(brain.kind, KIND);
        }
        assert_eq!(
            std::fs::read_to_string(out.join("history.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[test]
    fn structural_warm_start_preserves_dormant_weights_during_training() {
        let (mut config, catalog, dir) = fixture();
        config.generations = 1;
        config.es.population = 2;
        config.es.weight_decay = 1.0;
        config.es.l1 = 1.0;
        let mut brain = template(8).unwrap();
        let module = &mut brain.modules[0];
        module.net.params_mut()[0] = 2.0;
        let mut structure = module.net.structure().clone();
        structure.inputs[0] = false;
        module.net.set_structure(structure).unwrap();
        let path = dir.join("structural-initial.json");
        brain.save(&path).unwrap();
        config.init_from = Some(path);
        let out = dir.join("structural-training");
        train(&config, &catalog, &out).unwrap();
        let latest = Brain::load(&out.join("brain.latest.json")).unwrap();
        assert_eq!(
            latest.modules[0].net.params()[0].to_bits(),
            2.0f32.to_bits()
        );
    }
    #[test]
    fn quality_upgrade_preserves_nonzero_sensor_outputs_and_world_hash() {
        let (mut config, catalog, _) = fixture();
        let before = template(8).unwrap();
        let after = upgrade_v1_to_v2(&before).unwrap();
        let original = &before.modules[1].net;
        let upgraded = &after.modules[1].net;
        let old_inputs = vec![0.37; 12];
        let mut new_inputs = old_inputs.clone();
        new_inputs.extend(vec![
            0.81;
            super::super::strategic_sensors::EXTRA_INPUTS.len()
        ]);
        let (mut a, mut b) = ([0.0], [0.0]);
        original.forward(&old_inputs, &mut a);
        upgraded.forward(&new_inputs, &mut b);
        assert_eq!(a, b);
        let first = Prepared::new(&config, &catalog)
            .unwrap()
            .rollout(&before, 0, 0, 0, 917)
            .unwrap();
        config.brain_kind = KIND_V2.into();
        let second = Prepared::new(&config, &catalog)
            .unwrap()
            .rollout(&after, 0, 0, 0, 917)
            .unwrap();
        assert_eq!(first.state_hash, second.state_hash);
        assert_eq!(first.fitness, second.fitness);
        assert!(first.reward_stats.production_value > 0.0);
        assert_eq!(
            first.reward_stats.production_value,
            first.telemetry.production_value
        );
    }

    #[test]
    fn quality_ledger_counts_releases_and_distinguishes_reclaim_and_nanoframes() {
        use super::super::strategic_telemetry::TrialLedger;
        let (config, catalog, _) = fixture();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig::default(),
        )
        .unwrap();
        let builder = world
            .spawn_active(
                0,
                prepared.catalog.id("builder").unwrap(),
                Vec2::new(64.0, 64.0),
                BuildFacing::South,
            )
            .unwrap();
        let factory = world
            .spawn_active(
                0,
                prepared.catalog.id("factory").unwrap(),
                Vec2::new(224.0, 224.0),
                BuildFacing::South,
            )
            .unwrap();
        let value = prepared
            .catalog
            .get_by_name("builder")
            .unwrap()
            .1
            .resource_value() as f64;
        let mut ledger = TrialLedger::new(&world);
        assert_eq!(ledger.production_value, [0.0, 0.0]);
        world
            .queue_factory(factory, prepared.catalog.id("soldier").unwrap())
            .unwrap();
        for _ in 0..300 {
            world.step();
            ledger.observe(&world);
        }
        assert!(ledger.production_value[0] > 0.0);
        assert_eq!(
            ledger.production_value[0],
            world.teams[0].metrics.production_value
        );
        world.events.clear();
        world.destroy_entity(builder);
        world.events.push(WorldEvent::Reclaimed {
            entity: builder,
            side: 0,
        });
        ledger.observe(&world);
        ledger.observe(&world);
        let summary = ledger.summary(&world, 0, false);
        assert_eq!(summary.completed_entity_value_lost, 0.0);
        assert_eq!(summary.reclaimed_value, value);
        assert_eq!(summary.terminal_outcome, "unresolved");
        world.events.clear();
        world.entities.get_mut(factory).unwrap().lifecycle =
            crate::world::EntityLifecycle::UnderConstruction;
        ledger.observe(&world);
        world.destroy_entity(factory);
        ledger.observe(&world);
        assert_eq!(
            ledger.summary(&world, 0, false).completed_entity_value_lost,
            0.0
        );
    }
    #[test]
    fn quality_ledger_counts_a_product_released_and_destroyed_before_observation() {
        use super::super::strategic_telemetry::TrialLedger;
        let (config, catalog, _) = fixture();
        let prepared = Prepared::new(&config, &catalog).unwrap();
        let mut world = World::new(
            Arc::clone(&prepared.maps[0]),
            Arc::clone(&prepared.catalog),
            WorldConfig::default(),
        )
        .unwrap();
        let factory = world
            .spawn_active(
                0,
                prepared.catalog.id("factory").unwrap(),
                Vec2::new(224.0, 224.0),
                BuildFacing::South,
            )
            .unwrap();
        let mut ledger = TrialLedger::new(&world);
        world
            .queue_factory(factory, prepared.catalog.id("soldier").unwrap())
            .unwrap();
        for _ in 0..300 {
            world.step();
            let released = world.events.iter().find_map(|event| {
                if let WorldEvent::FactoryReleased { entity, .. } = event {
                    Some(*entity)
                } else {
                    None
                }
            });
            if let Some(entity) = released {
                let value = prepared
                    .catalog
                    .get(world.entities.get(entity).unwrap().definition)
                    .unwrap()
                    .resource_value() as f64;
                world.destroy_entity(entity);
                ledger.observe(&world);
                assert_eq!(
                    ledger.summary(&world, 0, true).completed_entity_value_lost,
                    value
                );
                assert_eq!(
                    ledger.production_value[0],
                    world.teams[0].metrics.production_value
                );
                return;
            }
            ledger.observe(&world);
        }
        panic!("fixture must release a real factory product");
    }
}
