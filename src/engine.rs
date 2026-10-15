use crate::catalog::{
    CAPABILITY_HOMING_PROJECTILES, CAPABILITY_INTERCEPTORS, CAPABILITY_SHIELDS,
    CapabilityRequirement, IMPLEMENTED_CAPABILITIES,
};
use crate::model::Spawn;
use crate::model::{
    Command, DT, StartingResources, Terrain, UnitState, Vec2, WeaponDef, WorldView, angle_delta,
    turn_toward,
};
use crate::policy::Policy;
use crate::score::{BatchSummary, BattleResult, SideScore};
use crate::spatial::SpatialGrid;
use crate::trace::{
    AimAttempt, AimState, ResourceMarker, RunTrace, TraceConfig, TraceMap, TraceOutcome,
    TraceRecorder,
};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Instant;

/// A single small capsule radius for all direct-fire line tests. Unit and
/// terrain geometry dominate these interactions; per-projectile colvols do not.
const PROJECTILE_CLEARANCE: f32 = 0.5;
/// BAR uses Recoil's default global EMP decline period. A weapon's
/// `paralyze_time` limits how far above the stun threshold damage may stack;
/// it does not replace this global decay rate.
const PARALYZE_DECLINE_TIME: f32 = 40.0;

#[derive(Clone, Copy, Debug)]
struct Impact {
    shooter_id: u32,
    shooter_side: u8,
    target_id: u32,
    point: Vec2,
    damage: f32,
    /// Compact indices recover per-armour damage without copying a damage
    /// table into every projectile. `u16::MAX` is used only by focused tests.
    weapon_definition_index: u32,
    weapon_index: u16,
    damage_fraction: f32,
    radius: f32,
    edge_effectiveness: f32,
    paralyzer: bool,
    paralyze_time: f32,
    hits_allies: bool,
    collide_non_target: bool,
    track_target: bool,
    requires_source: bool,
}

/// Per-tick state exists only for guided rounds. Ordinary ballistic rounds
/// remain a single entry in [`ImpactQueue`] and pay no steering cost.
#[derive(Clone, Copy, Debug)]
struct GuidedProjectile {
    impact: Impact,
    position: Vec2,
    height: f32,
    heading: f32,
    vertical_speed: f32,
    speed: f32,
    turn_rate: f32,
    remaining_ticks: u32,
    damage_pulses: u32,
}

/// Flight geometry for an unguided shot. Straight paths retain the existing
/// zero-cost calculation. Recoil weapons with gravity use an analytical arc,
/// but still collapse to one scheduled impact rather than allocating a
/// per-tick projectile object.
#[derive(Clone, Copy, Debug)]
enum FlightPath {
    Linear {
        from_y: f32,
        to_y: f32,
        flight_time: f32,
    },
    Ballistic {
        origin_y: f32,
        flight_time: f32,
        horizontal_speed: f32,
        vertical_speed: f32,
        gravity: f32,
    },
}

impl FlightPath {
    fn flight_time(self) -> f32 {
        match self {
            Self::Linear { flight_time, .. } | Self::Ballistic { flight_time, .. } => flight_time,
        }
    }

    fn height_at_fraction(self, fraction: f32) -> f32 {
        match self {
            Self::Linear { from_y, to_y, .. } => from_y + (to_y - from_y) * fraction,
            Self::Ballistic {
                origin_y,
                flight_time,
                vertical_speed,
                gravity,
                ..
            } => {
                let time = flight_time * fraction;
                origin_y + vertical_speed * time - 0.5 * gravity * time * time
            }
        }
    }

    fn launch_pitch(self, horizontal_distance: f32, from_y: f32, to_y: f32) -> f32 {
        match self {
            Self::Linear { .. } => (to_y - from_y).atan2(horizontal_distance),
            Self::Ballistic {
                horizontal_speed,
                vertical_speed,
                ..
            } => vertical_speed.atan2(horizontal_speed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DamageEvent {
    target_index: usize,
    source_side: u8,
    damage: f32,
    paralyzer: bool,
    paralyze_time: f32,
}

struct ImpactQueue {
    /// Absolute-tick intrusive queues. Two i32 arrays are substantially more
    /// compact than one Vec header per tick, while tails preserve shot order.
    heads: Vec<i32>,
    tails: Vec<i32>,
    nodes: Vec<ImpactNode>,
    pending: usize,
    maximum_tick: u64,
    free_head: i32,
}

#[derive(Clone, Copy)]
struct ImpactNode {
    impact: Impact,
    next: i32,
}

impl Default for ImpactQueue {
    fn default() -> Self {
        Self {
            heads: Vec::new(),
            tails: Vec::new(),
            nodes: Vec::new(),
            pending: 0,
            maximum_tick: 0,
            free_head: -1,
        }
    }
}

impl ImpactQueue {
    fn new(maximum_tick: u64) -> Self {
        Self {
            maximum_tick,
            ..Self::default()
        }
    }

    fn schedule(&mut self, due_tick: u64, impact: Impact) {
        if due_tick > self.maximum_tick {
            return;
        }
        let index = due_tick as usize;
        if index >= self.heads.len() {
            self.heads.resize(index + 1, -1);
            self.tails.resize(index + 1, -1);
        }
        let node_index = if self.free_head >= 0 {
            let reused = self.free_head;
            self.free_head = self.nodes[reused as usize].next;
            self.nodes[reused as usize] = ImpactNode { impact, next: -1 };
            reused
        } else {
            let appended = self.nodes.len() as i32;
            self.nodes.push(ImpactNode { impact, next: -1 });
            appended
        };
        if self.tails[index] >= 0 {
            self.nodes[self.tails[index] as usize].next = node_index;
        } else {
            self.heads[index] = node_index;
        }
        self.tails[index] = node_index;
        self.pending += 1;
    }

    fn is_empty(&self) -> bool {
        self.pending == 0
    }

    fn reset(&mut self, maximum_tick: u64) {
        self.heads.fill(-1);
        self.tails.fill(-1);
        self.nodes.clear();
        self.pending = 0;
        self.maximum_tick = maximum_tick;
        self.free_head = -1;
    }
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub terrain: Terrain,
    pub units: Vec<Spawn>,
    pub max_time: f32,
    /// Optional map annotations copied only into human-readable traces.
    pub trace_resources: Vec<ResourceMarker>,
}

impl Scenario {
    pub fn duel(unit: crate::model::UnitDef, separation: f32) -> Self {
        let width = (separation + 1_000.0).max(2_000.0);
        let centre = width * 0.5;
        Self {
            terrain: Terrain::flat(width, 1_000.0, 32.0),
            units: vec![
                Spawn {
                    side: 0,
                    position: Vec2::new(centre - separation * 0.5, 500.0),
                    heading: 0.0,
                    definition: unit.clone(),
                },
                Spawn {
                    side: 1,
                    position: Vec2::new(centre + separation * 0.5, 500.0),
                    heading: PI,
                    definition: unit,
                },
            ],
            max_time: 120.0,
            trace_resources: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Simulator {
    scenario: Scenario,
    definitions: Vec<Arc<crate::model::UnitDef>>,
    terrain_all_passable: bool,
    paralysis_enabled: bool,
    /// `None` keeps historical tactical runs resource-unlimited. Experiments
    /// can opt into finite per-side ammunition resources without coupling this
    /// hot loop to the strategic economy scheduler.
    combat_resources: Option<[StartingResources; 2]>,
}

/// Opaque identity supplied by a strategic world. Tactical IDs remain an
/// implementation detail and are never reused while a stepper is alive, so an
/// in-flight projectile cannot accidentally acquire a newly inserted target.
pub type CombatKey = u64;

/// Side sentinel for map features and other neutral blockers. Neutral objects
/// cannot fire and never index two-side score/resource arrays, but ordinary
/// projectile collision and damage still apply to them.
pub const NEUTRAL_COMBAT_SIDE: u8 = u8::MAX;

/// Index into a stepper's immutable, shared unit-definition table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CombatDefinitionId(pub usize);

/// Initial runtime state for a combatant, building, wall, or other cylindrical
/// blocker. A stationary definition with `blocks_movement`/`blocks_fire` set is
/// sufficient to model a wall; no special wall path exists in the hot loop.
#[derive(Clone, Copy, Debug)]
pub struct CombatSpawn {
    pub key: CombatKey,
    pub side: u8,
    pub definition: CombatDefinitionId,
    pub position: Vec2,
    pub heading: f32,
    /// `None` starts at the definition's full health.
    pub hp: Option<f32>,
}

/// Persistent order consumed by every tick until replaced or cleared.
#[derive(Clone, Copy, Debug, Default)]
pub struct CombatCommand {
    pub target: Option<CombatKey>,
    pub destination: Option<Vec2>,
    pub throttle: f32,
}

/// Sparse external synchronization patch. Changing definitions resets weapon
/// state because reload/burst state cannot be transferred between loadouts.
#[derive(Clone, Copy, Debug, Default)]
pub struct CombatantUpdate {
    pub side: Option<u8>,
    pub definition: Option<CombatDefinitionId>,
    pub position: Option<Vec2>,
    pub heading: Option<f32>,
    pub speed: Option<f32>,
    pub hp: Option<f32>,
    pub collision_height_scale: Option<f32>,
    pub paralyze_damage: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatDeath {
    pub key: CombatKey,
    pub side: u8,
    pub position: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatStepSummary {
    /// Tick just resolved.
    pub tick: u64,
    /// Simulation time after resolving that tick.
    pub time: f32,
    pub deaths: usize,
    pub pending_impacts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatError {
    DuplicateKey(CombatKey),
    UnknownKey(CombatKey),
    UnknownTarget(CombatKey),
    UnknownDefinition(usize),
    InvalidSide(u8),
}

/// Long-lived tactical state for integration with the strategic `World`.
///
/// Definitions are reference-counted once at registration; combatants share
/// them without cloning catalog strings or weapon arrays. All per-tick scratch
/// vectors are retained. Call [`CombatStepper::reserve`] before a bounded run
/// to remove capacity growth from the measured tick loop.
pub struct CombatStepper {
    terrain: Terrain,
    terrain_all_passable: bool,
    definitions: Vec<Arc<crate::model::UnitDef>>,
    movement_masks: Vec<Option<Arc<[bool]>>>,
    paralysis_enabled: bool,
    units: Vec<UnitState>,
    keys: Vec<Option<CombatKey>>,
    indices: HashMap<CombatKey, usize>,
    commands: Vec<Command>,
    impacts: ImpactQueue,
    guided_projectiles: Vec<GuidedProjectile>,
    damage_events: Vec<DamageEvent>,
    spatial_candidates: Vec<usize>,
    spatial: SpatialGrid,
    aim_attempts: Vec<AimAttempt>,
    deaths: Vec<CombatDeath>,
    was_alive: Vec<bool>,
    battle: BattleState,
    seed: u64,
    tick: u64,
    time: f32,
}

/// Allocation-owning state retained across trials by each Rayon worker.
/// Scenario and definition data are immutable; only these runtime buffers need
/// to be reset between seeds.
struct TrialWorkspace {
    units: Vec<UnitState>,
    impacts: ImpactQueue,
    guided_projectiles: Vec<GuidedProjectile>,
    damage_events: Vec<DamageEvent>,
    spatial_candidates: Vec<usize>,
    spatial: SpatialGrid,
    commands: Vec<Command>,
}

impl TrialWorkspace {
    fn new(simulator: &Simulator) -> Self {
        let units = spawn_units(&simulator.scenario.units, &simulator.definitions);
        let maximum_tick = (simulator.scenario.max_time / DT).ceil() as u64;
        Self {
            commands: vec![Command::default(); units.len()],
            units,
            impacts: ImpactQueue::new(maximum_tick),
            guided_projectiles: Vec::new(),
            damage_events: Vec::new(),
            spatial_candidates: Vec::new(),
            spatial: SpatialGrid::new(&simulator.scenario.terrain, 128.0),
        }
    }

    fn reset(&mut self, simulator: &Simulator) {
        reset_units(&mut self.units, &simulator.scenario.units);
        self.impacts
            .reset((simulator.scenario.max_time / DT).ceil() as u64);
        self.guided_projectiles.clear();
        self.damage_events.clear();
        self.spatial_candidates.clear();
        self.commands.resize(self.units.len(), Command::default());
        self.commands.fill(Command::default());
    }
}

struct BattleState {
    scores: [SideScore; 2],
    alive_counts: [usize; 2],
    combat_resources: Option<[StartingResources; 2]>,
}

impl Simulator {
    /// Checked constructor for callers that did not validate a complete
    /// catalog at their experiment boundary. `new` remains available for old
    /// fixtures that intentionally exercise partial mechanics.
    pub fn try_new(scenario: Scenario) -> Result<Self, Vec<CapabilityRequirement>> {
        let unsupported = unsupported_scenario_capabilities(&scenario);
        if unsupported.is_empty() {
            Ok(Self::new(scenario))
        } else {
            Err(unsupported)
        }
    }

    pub fn new(mut scenario: Scenario) -> Self {
        scenario.terrain.refresh_derived();
        let terrain_all_passable = scenario.terrain.passable.iter().all(|passable| *passable);
        let definitions = scenario
            .units
            .iter()
            .map(|spawn| Arc::new(spawn.definition.clone()))
            .collect();
        let paralysis_enabled = scenario.units.iter().any(|spawn| {
            spawn
                .definition
                .weapons
                .iter()
                .any(|weapon| weapon.paralyzer)
        });
        Self {
            scenario,
            definitions,
            terrain_all_passable,
            paralysis_enabled,
            combat_resources: None,
        }
    }

    /// Enables finite tactical weapon resources. Costs are charged per emitted
    /// salvo, or when manufacture begins for an automatically queued stockpile
    /// round. The default remains unlimited for backwards-compatible duels.
    pub fn with_starting_resources(mut self, resources: [StartingResources; 2]) -> Self {
        self.combat_resources = Some(resources);
        self
    }

    pub fn run(&self, policies: [&dyn Policy; 2], seed: u64) -> BattleResult {
        let mut workspace = TrialWorkspace::new(self);
        self.run_with_workspace::<false>(policies, seed, None, &mut workspace)
            .0
    }

    /// Records one sampled run for the viewer. The ordinary `run` method uses
    /// a const-specialized path in which trace branches and allocations vanish.
    pub fn run_traced(
        &self,
        policies: [&dyn Policy; 2],
        seed: u64,
        config: TraceConfig,
    ) -> (BattleResult, RunTrace) {
        let recorder = self.trace_recorder(config, seed);
        let mut workspace = TrialWorkspace::new(self);
        let (result, trace) =
            self.run_with_workspace::<true>(policies, seed, Some(recorder), &mut workspace);
        (
            result,
            trace.expect("traced specialization always returns a trace"),
        )
    }

    /// Records multiple representative seeds in parallel for heatmap viewers.
    /// This is intentionally opt-in and separate from the allocation-light
    /// headless batch path.
    pub fn run_traced_parallel(
        &self,
        policies: [&dyn Policy; 2],
        runs: usize,
        seed: u64,
        config: TraceConfig,
    ) -> Vec<(BattleResult, RunTrace)> {
        (0..runs)
            .into_par_iter()
            .map_init(
                || TrialWorkspace::new(self),
                |workspace, index| {
                    let trial_seed = seed.wrapping_add(index as u64);
                    let recorder = self.trace_recorder(config, trial_seed);
                    let (result, trace) = self.run_with_workspace::<true>(
                        policies,
                        trial_seed,
                        Some(recorder),
                        workspace,
                    );
                    (
                        result,
                        trace.expect("traced specialization always returns a trace"),
                    )
                },
            )
            .collect()
    }

    fn trace_recorder(&self, config: TraceConfig, seed: u64) -> TraceRecorder {
        let terrain = &self.scenario.terrain;
        let map = TraceMap {
            width: terrain.cols as f32 * terrain.cell_size,
            depth: terrain.rows as f32 * terrain.cell_size,
            cols: terrain.cols,
            rows: terrain.rows,
            cell_size: terrain.cell_size,
            heights: terrain.heights.clone(),
            water_depth: terrain
                .heights
                .iter()
                .map(|height| (terrain.water_level - height).max(0.0))
                .collect(),
            passable: terrain.passable.clone(),
            resources: self.scenario.trace_resources.clone(),
        };
        TraceRecorder::new(config, seed, crate::model::SIM_HZ, map)
    }

    fn run_with_workspace<const TRACE: bool>(
        &self,
        policies: [&dyn Policy; 2],
        seed: u64,
        mut recorder: Option<TraceRecorder>,
        workspace: &mut TrialWorkspace,
    ) -> (BattleResult, Option<RunTrace>) {
        workspace.reset(self);
        let TrialWorkspace {
            units,
            impacts,
            guided_projectiles,
            damage_events,
            spatial_candidates,
            spatial,
            commands,
        } = workspace;
        let mut battle = BattleState {
            scores: initial_scores(units),
            alive_counts: count_live_sides(units),
            combat_resources: self.combat_resources,
        };
        let mut time = 0.0;
        let mut tick = 0_u64;

        // Let already-fired projectiles and damage-over-time pulses land after
        // one army dies; otherwise long flight time would create false wins.
        while time < self.scenario.max_time
            && ((battle.alive_counts[0] > 0 && battle.alive_counts[1] > 0)
                || !impacts.is_empty()
                || !guided_projectiles.is_empty())
        {
            if self.paralysis_enabled {
                advance_paralysis(units);
            }
            let view = WorldView {
                time,
                terrain: &self.scenario.terrain,
                units,
            };
            for (command, unit) in commands.iter_mut().zip(units.iter()) {
                *command = if unit.alive && unit.side < 2 {
                    policies[unit.side as usize].command(&view, unit)
                } else {
                    Command::default()
                };
            }

            update_movement(
                units,
                commands,
                &self.scenario.terrain,
                self.terrain_all_passable,
                &[],
            );
            let indexed_collisions = units.len() >= 64;
            if indexed_collisions {
                spatial.rebuild(units);
            }
            separate_collisions(
                units,
                &self.scenario.terrain,
                spatial,
                spatial_candidates,
                indexed_collisions,
                self.terrain_all_passable,
                &[],
            );
            // Collision resolution moves centres, so weapon/AOE queries need
            // a fresh index. Epoch stamping makes this O(units), not O(cells).
            spatial.rebuild(units);
            let trace_tick = TRACE
                && recorder
                    .as_ref()
                    .is_some_and(|recorder| recorder.should_sample(tick));
            let mut aim_attempts = Vec::new();
            update_weapons::<TRACE>(
                units,
                commands,
                time,
                tick,
                impacts,
                guided_projectiles,
                &mut battle,
                seed,
                &self.scenario.terrain,
                &self.definitions,
                spatial,
                spatial_candidates,
                trace_tick,
                &mut aim_attempts,
            );
            advance_guided_projectiles(
                units,
                tick,
                guided_projectiles,
                impacts,
                &self.scenario.terrain,
                spatial,
                spatial_candidates,
            );
            resolve_impacts(
                units,
                tick,
                impacts,
                damage_events,
                spatial,
                spatial_candidates,
                &mut battle,
                &self.definitions,
            );
            if TRACE && trace_tick {
                let view = WorldView {
                    time,
                    terrain: &self.scenario.terrain,
                    units,
                };
                recorder
                    .as_mut()
                    .expect("traced specialization has recorder")
                    .record_world(tick, &view, aim_attempts);
            }
            time += DT;
            tick += 1;
        }

        finish_scores(units, &mut battle.scores, time);
        let alive = [battle.alive_counts[0] > 0, battle.alive_counts[1] > 0];
        let result = BattleResult {
            winner: match alive {
                [true, false] => Some(0),
                [false, true] => Some(1),
                _ => None,
            },
            timed_out: time >= self.scenario.max_time && alive[0] && alive[1],
            duration: time.min(self.scenario.max_time),
            sides: battle.scores,
        };
        let trace = recorder.map(|recorder| {
            recorder.finish(TraceOutcome {
                winner: result.winner,
                duration: result.duration,
            })
        });
        (result, trace)
    }

    pub fn run_parallel(&self, policies: [&dyn Policy; 2], runs: usize, seed: u64) -> BatchSummary {
        let started = Instant::now();
        let results = self.run_parallel_results(policies, runs, seed);
        BatchSummary::from_results(&results, started.elapsed().as_secs_f64())
    }

    /// Retain every seeded result for experiment-specific scoring or
    /// distribution analysis. Ordering always matches increasing seed index.
    pub fn run_parallel_results(
        &self,
        policies: [&dyn Policy; 2],
        runs: usize,
        seed: u64,
    ) -> Vec<BattleResult> {
        (0..runs)
            .into_par_iter()
            .map_init(
                || TrialWorkspace::new(self),
                |workspace, index| {
                    self.run_with_workspace::<false>(
                        policies,
                        seed.wrapping_add(index as u64),
                        None,
                        workspace,
                    )
                    .0
                },
            )
            .collect()
    }

    /// Run each seed twice with policy sides reversed, then normalize results
    /// so score slot 0 always belongs to `policies[0]`.
    pub fn run_paired_parallel(
        &self,
        policies: [&dyn Policy; 2],
        pairs: usize,
        seed: u64,
    ) -> BatchSummary {
        let started = Instant::now();
        let results: Vec<BattleResult> = (0..pairs)
            .into_par_iter()
            .map_init(
                || TrialWorkspace::new(self),
                |workspace, index| {
                    let trial_seed = seed.wrapping_add(index as u64);
                    let first = self
                        .run_with_workspace::<false>(policies, trial_seed, None, workspace)
                        .0;
                    let second = normalize_swapped_result(
                        self.run_with_workspace::<false>(
                            [policies[1], policies[0]],
                            trial_seed,
                            None,
                            workspace,
                        )
                        .0,
                    );
                    [first, second]
                },
            )
            .flat_map_iter(|pair| pair)
            .collect();
        BatchSummary::from_results(&results, started.elapsed().as_secs_f64())
    }
}

fn unsupported_scenario_capabilities(scenario: &Scenario) -> Vec<CapabilityRequirement> {
    let supported: BTreeSet<&str> = IMPLEMENTED_CAPABILITIES.iter().copied().collect();
    let mut requirements = BTreeSet::new();
    for spawn in &scenario.units {
        for capability in &spawn.definition.strategic.required_capabilities {
            if !supported.contains(capability.as_str()) {
                requirements.insert(CapabilityRequirement {
                    capability: capability.clone(),
                    unit: Some(spawn.definition.name.clone()),
                    weapon: None,
                });
            }
        }
        for weapon in &spawn.definition.weapons {
            let mut add = |capability: &str| {
                if !supported.contains(capability) {
                    requirements.insert(CapabilityRequirement {
                        capability: capability.to_owned(),
                        unit: Some(spawn.definition.name.clone()),
                        weapon: (!weapon.name.is_empty()).then(|| weapon.name.clone()),
                    });
                }
            };
            for capability in &weapon.required_capabilities {
                add(capability);
            }
            if weapon.tracks && !weapon.hitscan {
                add(CAPABILITY_HOMING_PROJECTILES);
            }
            if weapon.is_shield {
                add(CAPABILITY_SHIELDS);
            }
            if weapon.interceptor != 0 || weapon.targetable != 0 {
                add(CAPABILITY_INTERCEPTORS);
            }
        }
    }
    requirements.into_iter().collect()
}

impl CombatStepper {
    pub fn new(
        mut terrain: Terrain,
        definitions: Vec<Arc<crate::model::UnitDef>>,
        seed: u64,
    ) -> Self {
        terrain.refresh_derived();
        let terrain_all_passable = terrain.passable.iter().all(|passable| *passable);
        let paralysis_enabled = definitions
            .iter()
            .any(|definition| definition.weapons.iter().any(|weapon| weapon.paralyzer));
        let spatial = SpatialGrid::new(&terrain, 128.0);
        let movement_masks = vec![None; definitions.len()];
        Self {
            terrain,
            terrain_all_passable,
            definitions,
            movement_masks,
            paralysis_enabled,
            units: Vec::new(),
            keys: Vec::new(),
            indices: HashMap::new(),
            commands: Vec::new(),
            impacts: ImpactQueue::new(u64::MAX),
            guided_projectiles: Vec::new(),
            damage_events: Vec::new(),
            spatial_candidates: Vec::new(),
            spatial,
            aim_attempts: Vec::new(),
            deaths: Vec::new(),
            was_alive: Vec::new(),
            battle: BattleState {
                scores: [
                    SideScore {
                        side: 0,
                        ..Default::default()
                    },
                    SideScore {
                        side: 1,
                        ..Default::default()
                    },
                ],
                alive_counts: [0; 2],
                combat_resources: None,
            },
            seed,
            tick: 0,
            time: 0.0,
        }
    }

    pub fn terrain(&self) -> &Terrain {
        &self.terrain
    }

    pub fn tick_index(&self) -> u64 {
        self.tick
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn pending_impacts(&self) -> usize {
        self.impacts.pending + self.guided_projectiles.len()
    }

    pub fn definitions(&self) -> &[Arc<crate::model::UnitDef>] {
        &self.definitions
    }

    pub fn register_definition(
        &mut self,
        definition: Arc<crate::model::UnitDef>,
    ) -> CombatDefinitionId {
        self.paralysis_enabled |= definition.weapons.iter().any(|weapon| weapon.paralyzer);
        let id = CombatDefinitionId(self.definitions.len());
        self.definitions.push(definition);
        self.movement_masks.push(None);
        id
    }

    /// Associates an optional per-definition passability mask with a combat
    /// definition. Footprint checks for this definition will query this mask
    /// instead of the baseline terrain's passability layer.
    pub fn set_definition_movement_mask(
        &mut self,
        definition: CombatDefinitionId,
        mask: Arc<[bool]>,
    ) {
        if definition.0 >= self.movement_masks.len() {
            self.movement_masks.resize(definition.0 + 1, None);
        }
        self.movement_masks[definition.0] = Some(mask);
    }

    /// Reserve all capacity that can grow in the tick loop. `additional_ticks`
    /// should cover the intended bounded run plus its maximum projectile flight
    /// time. Mutations beyond the reserved unit/impact capacities may allocate,
    /// but steady-state stepping will not.
    pub fn reserve(
        &mut self,
        additional_combatants: usize,
        additional_ticks: usize,
        impact_nodes: usize,
    ) {
        self.units.reserve(additional_combatants);
        self.keys.reserve(additional_combatants);
        self.indices.reserve(additional_combatants);
        self.commands.reserve(additional_combatants);
        self.was_alive.reserve(additional_combatants);
        self.spatial_candidates.reserve(additional_combatants);
        self.deaths
            .reserve(self.units.len().saturating_add(additional_combatants));
        self.damage_events.reserve(impact_nodes);
        self.impacts.nodes.reserve(impact_nodes);
        self.guided_projectiles.reserve(impact_nodes);
        let final_tick = self
            .tick
            .saturating_add(additional_ticks as u64)
            .min((usize::MAX - 1) as u64) as usize;
        if self.impacts.heads.len() <= final_tick {
            self.impacts.heads.resize(final_tick + 1, -1);
            self.impacts.tails.resize(final_tick + 1, -1);
        }
        // Allocate the broad phase's map and unit arrays outside the measured
        // loop once the current combatants are known.
        self.spatial.rebuild(&self.units);
    }

    pub fn set_resources(&mut self, resources: Option<[StartingResources; 2]>) {
        self.battle.combat_resources = resources;
    }

    pub fn resources(&self) -> Option<[StartingResources; 2]> {
        self.battle.combat_resources
    }

    pub fn insert(&mut self, spawn: CombatSpawn) -> Result<(), CombatError> {
        if self.indices.contains_key(&spawn.key) {
            return Err(CombatError::DuplicateKey(spawn.key));
        }
        if spawn.side >= 2 && spawn.side != NEUTRAL_COMBAT_SIDE {
            return Err(CombatError::InvalidSide(spawn.side));
        }
        let definition = self
            .definitions
            .get(spawn.definition.0)
            .ok_or(CombatError::UnknownDefinition(spawn.definition.0))?;
        let index = self.units.len();
        let hp = spawn.hp.unwrap_or(definition.health).max(0.0);
        let alive = hp > 0.0;
        self.units.push(new_unit_state(
            index as u32 + 1,
            spawn.side,
            Arc::clone(definition),
            spawn.definition.0,
            spawn.position,
            spawn.heading,
            hp,
        ));
        self.units[index].alive = alive;
        self.keys.push(Some(spawn.key));
        self.commands.push(Command::default());
        self.indices.insert(spawn.key, index);
        if alive && spawn.side < 2 {
            self.battle.alive_counts[spawn.side as usize] += 1;
            self.battle.scores[spawn.side as usize].starting_value += definition.resource_value();
        }
        Ok(())
    }

    /// Remove an entity without producing a death explosion. Ordinary rounds
    /// already in flight remain scheduled; source-dependent beams stop.
    pub fn remove(&mut self, key: CombatKey) -> Result<(), CombatError> {
        let index = self
            .indices
            .remove(&key)
            .ok_or(CombatError::UnknownKey(key))?;
        let unit = &mut self.units[index];
        if unit.alive && unit.side < 2 {
            self.battle.alive_counts[unit.side as usize] -= 1;
        }
        unit.alive = false;
        unit.speed = 0.0;
        self.keys[index] = None;
        self.commands[index] = Command::default();
        Ok(())
    }

    /// Restore contiguous tactical IDs after explicit removals. Compaction is
    /// refused while any projectile or delayed pulse still carries an old ID;
    /// call again after `pending_impacts()` reaches zero. Combat-killed units
    /// remain addressable until the strategic owner explicitly removes them.
    pub fn compact_removed(&mut self) -> bool {
        if !self.impacts.is_empty() || !self.guided_projectiles.is_empty() {
            return false;
        }
        let mut remap = vec![0_u32; self.units.len()];
        let mut write = 0;
        for (read, new_id) in remap.iter_mut().enumerate() {
            if self.keys[read].is_none() {
                continue;
            }
            *new_id = write as u32 + 1;
            if read != write {
                self.units.swap(write, read);
                self.keys.swap(write, read);
                self.commands.swap(write, read);
            }
            self.units[write].id = write as u32 + 1;
            write += 1;
        }
        self.units.truncate(write);
        self.keys.truncate(write);
        self.commands.truncate(write);
        for command in &mut self.commands {
            command.target = command
                .target
                .and_then(|id| remap.get(id.saturating_sub(1) as usize).copied())
                .filter(|id| *id != 0);
        }
        self.indices.clear();
        for (index, key) in self.keys.iter().enumerate() {
            self.indices
                .insert(key.expect("compaction retained present keys"), index);
        }
        self.spatial.rebuild(&self.units);
        true
    }

    pub fn update(&mut self, key: CombatKey, update: CombatantUpdate) -> Result<(), CombatError> {
        let index = *self.indices.get(&key).ok_or(CombatError::UnknownKey(key))?;
        if let Some(side) = update.side
            && side >= 2
            && side != NEUTRAL_COMBAT_SIDE
        {
            return Err(CombatError::InvalidSide(side));
        }
        let replacement_definition = match update.definition {
            Some(definition_id) => Some(
                self.definitions
                    .get(definition_id.0)
                    .cloned()
                    .ok_or(CombatError::UnknownDefinition(definition_id.0))?,
            ),
            None => None,
        };
        let old_side = self.units[index].side;
        let old_alive = self.units[index].alive;
        if let Some(side) = update.side {
            self.units[index].side = side;
        }
        if let Some(position) = update.position {
            self.units[index].position = position;
        }
        if let Some(heading) = update.heading {
            self.units[index].heading = heading;
            self.units[index].forward = Vec2::from_heading(heading);
        }
        if let (Some(definition_id), Some(definition)) = (update.definition, replacement_definition)
        {
            self.units[index].weapons =
                initial_weapon_states(&definition, self.units[index].heading);
            self.units[index].definition = definition;
            self.units[index].definition_index = definition_id.0;
        }
        if let Some(speed) = update.speed {
            self.units[index].speed = speed;
        }
        if let Some(hp) = update.hp {
            self.units[index].hp = hp;
            self.units[index].alive = hp > 0.0;
            if !self.units[index].alive {
                self.units[index].speed = 0.0;
            }
        }
        if let Some(scale) = update.collision_height_scale {
            self.units[index].collision_height_scale = if scale.is_finite() {
                scale.clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
        if let Some(paralyze_damage) = update.paralyze_damage {
            self.units[index].paralyze_damage = paralyze_damage.max(0.0);
        }
        self.units[index].stunned =
            self.units[index].paralyze_damage > self.units[index].definition.health.max(0.0);
        let new_side = self.units[index].side;
        let new_alive = self.units[index].alive;
        if old_alive && old_side < 2 {
            self.battle.alive_counts[old_side as usize] -= 1;
        }
        if new_alive && new_side < 2 {
            self.battle.alive_counts[new_side as usize] += 1;
        }
        Ok(())
    }

    pub fn set_command(
        &mut self,
        key: CombatKey,
        command: CombatCommand,
    ) -> Result<(), CombatError> {
        let index = *self.indices.get(&key).ok_or(CombatError::UnknownKey(key))?;
        let target = match command.target {
            Some(target) => Some(
                self.indices
                    .get(&target)
                    .map(|index| self.units[*index].id)
                    .ok_or(CombatError::UnknownTarget(target))?,
            ),
            None => None,
        };
        self.commands[index] = Command {
            target,
            destination: command.destination,
            throttle: command.throttle,
        };
        Ok(())
    }

    pub fn clear_command(&mut self, key: CombatKey) -> Result<(), CombatError> {
        let index = *self.indices.get(&key).ok_or(CombatError::UnknownKey(key))?;
        self.commands[index] = Command::default();
        Ok(())
    }

    /// Returns runtime state for live or combat-killed entities. Explicitly
    /// removed entities are no longer addressable by their external key.
    pub fn combatant(&self, key: CombatKey) -> Option<&UnitState> {
        self.indices.get(&key).map(|index| &self.units[*index])
    }

    /// Current yaw of the first turret mount, for symbolic tactical viewers.
    /// Fixed mounts deliberately return `None`: their aim is the hull heading
    /// and drawing a second identical indicator would add noise.
    pub fn primary_turret_heading(&self, key: CombatKey) -> Option<f32> {
        let unit = self.combatant(key)?;
        unit.definition
            .weapons
            .iter()
            .zip(&unit.weapons)
            .find_map(|(weapon, state)| weapon.turret.then_some(state.aim_heading))
    }

    /// Current charge of a shield weapon. Returns `None` for unknown entities,
    /// ordinary weapons, or an out-of-range mount index.
    pub fn shield_charge(&self, key: CombatKey, weapon_index: usize) -> Option<f32> {
        let unit = self.combatant(key)?;
        unit.definition
            .weapons
            .get(weapon_index)?
            .is_shield
            .then(|| unit.weapons[weapon_index].shield_charge)
    }

    /// Current ready rounds for a stockpiled weapon. Useful for strategic UI
    /// and deterministic defensive-system tests without exposing mutable state.
    pub fn stockpiled_rounds(&self, key: CombatKey, weapon_index: usize) -> Option<u16> {
        let unit = self.combatant(key)?;
        unit.definition
            .weapons
            .get(weapon_index)?
            .stockpile
            .then(|| unit.weapons[weapon_index].stockpiled_rounds)
    }

    pub fn combatants(&self) -> impl Iterator<Item = (CombatKey, &UnitState)> {
        self.keys
            .iter()
            .zip(&self.units)
            .filter_map(|(key, unit)| key.map(|key| (key, unit)))
    }

    pub fn deaths(&self) -> &[CombatDeath] {
        &self.deaths
    }

    pub fn step(&mut self) -> CombatStepSummary {
        self.deaths.clear();
        if self.paralysis_enabled {
            advance_paralysis(&mut self.units);
        }
        update_movement(
            &mut self.units,
            &self.commands,
            &self.terrain,
            self.terrain_all_passable,
            &self.movement_masks,
        );
        let indexed_collisions = self.units.len() >= 64;
        if indexed_collisions {
            self.spatial.rebuild(&self.units);
        }
        separate_collisions(
            &mut self.units,
            &self.terrain,
            &self.spatial,
            &mut self.spatial_candidates,
            indexed_collisions,
            self.terrain_all_passable,
            &self.movement_masks,
        );
        self.spatial.rebuild(&self.units);
        self.aim_attempts.clear();
        update_weapons::<false>(
            &mut self.units,
            &self.commands,
            self.time,
            self.tick,
            &mut self.impacts,
            &mut self.guided_projectiles,
            &mut self.battle,
            self.seed,
            &self.terrain,
            &self.definitions,
            &self.spatial,
            &mut self.spatial_candidates,
            false,
            &mut self.aim_attempts,
        );
        advance_guided_projectiles(
            &self.units,
            self.tick,
            &mut self.guided_projectiles,
            &mut self.impacts,
            &self.terrain,
            &self.spatial,
            &mut self.spatial_candidates,
        );
        self.was_alive.resize(self.units.len(), false);
        for (was_alive, unit) in self.was_alive.iter_mut().zip(&self.units) {
            *was_alive = unit.alive;
        }
        resolve_impacts(
            &mut self.units,
            self.tick,
            &mut self.impacts,
            &mut self.damage_events,
            &self.spatial,
            &mut self.spatial_candidates,
            &mut self.battle,
            &self.definitions,
        );
        for (index, unit) in self.units.iter().enumerate() {
            if self.was_alive[index]
                && !unit.alive
                && let Some(key) = self.keys[index]
            {
                self.deaths.push(CombatDeath {
                    key,
                    side: unit.side,
                    position: unit.position,
                });
            }
        }
        let resolved_tick = self.tick;
        self.tick = self.tick.saturating_add(1);
        self.time += DT;
        CombatStepSummary {
            tick: resolved_tick,
            time: self.time,
            deaths: self.deaths.len(),
            pending_impacts: self.pending_impacts(),
        }
    }

    /// Stable digest of externally visible and retained tactical runtime state.
    /// Intended for replay validation, not cryptographic use.
    pub fn deterministic_state_hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        hash_value(&mut hash, self.seed);
        hash_value(&mut hash, self.tick);
        hash_value(&mut hash, u64::from(self.time.to_bits()));
        for (index, unit) in self.units.iter().enumerate() {
            hash_value(&mut hash, self.keys[index].unwrap_or(u64::MAX));
            hash_value(&mut hash, u64::from(unit.id));
            hash_value(&mut hash, u64::from(unit.side));
            hash_value(&mut hash, u64::from(unit.position.x.to_bits()));
            hash_value(&mut hash, u64::from(unit.position.z.to_bits()));
            hash_value(&mut hash, u64::from(unit.heading.to_bits()));
            hash_value(&mut hash, u64::from(unit.speed.to_bits()));
            hash_value(&mut hash, u64::from(unit.hp.to_bits()));
            hash_value(&mut hash, u64::from(unit.collision_height_scale.to_bits()));
            hash_value(&mut hash, u64::from(unit.paralyze_damage.to_bits()));
            hash_value(&mut hash, unit.alive as u64);
            for weapon in &unit.weapons {
                hash_value(&mut hash, u64::from(weapon.aim_heading.to_bits()));
                hash_value(&mut hash, u64::from(weapon.aim_pitch.to_bits()));
                hash_value(&mut hash, u64::from(weapon.reload_until.to_bits()));
                hash_value(&mut hash, u64::from(weapon.next_shot_at.to_bits()));
                hash_value(&mut hash, u64::from(weapon.shots_left));
                hash_value(&mut hash, weapon.shot_sequence);
                hash_value(&mut hash, u64::from(weapon.predict_speed_mod.to_bits()));
                hash_value(
                    &mut hash,
                    u64::from(weapon.next_prediction_update.to_bits()),
                );
                hash_value(&mut hash, u64::from(weapon.stockpiled_rounds));
                hash_value(&mut hash, u64::from(weapon.stockpile_ready_at.to_bits()));
                hash_value(&mut hash, weapon.stockpile_in_progress as u64);
                hash_value(&mut hash, u64::from(weapon.shield_charge.to_bits()));
                hash_value(&mut hash, u64::from(weapon.shield_regen_after.to_bits()));
            }
            let command = self.commands[index];
            hash_value(&mut hash, u64::from(command.target.unwrap_or(0)));
            hash_value(
                &mut hash,
                u64::from(command.destination.unwrap_or_default().x.to_bits()),
            );
            hash_value(
                &mut hash,
                u64::from(command.destination.unwrap_or_default().z.to_bits()),
            );
            hash_value(&mut hash, command.destination.is_some() as u64);
            hash_value(&mut hash, u64::from(command.throttle.to_bits()));
        }
        if let Some(resources) = self.battle.combat_resources {
            hash_value(&mut hash, 1);
            for side in resources {
                hash_value(&mut hash, side.metal.to_bits());
                hash_value(&mut hash, side.energy.to_bits());
            }
        } else {
            hash_value(&mut hash, 0);
        }
        hash_value(&mut hash, self.impacts.pending as u64);
        for (due_tick, head) in self.impacts.heads.iter().copied().enumerate() {
            let mut node = head;
            while node >= 0 {
                let impact_node = self.impacts.nodes[node as usize];
                let impact = impact_node.impact;
                hash_value(&mut hash, due_tick as u64);
                hash_value(&mut hash, u64::from(impact.shooter_id));
                hash_value(&mut hash, u64::from(impact.target_id));
                hash_value(&mut hash, u64::from(impact.point.x.to_bits()));
                hash_value(&mut hash, u64::from(impact.point.z.to_bits()));
                hash_value(&mut hash, u64::from(impact.damage.to_bits()));
                node = impact_node.next;
            }
        }
        hash_value(&mut hash, self.guided_projectiles.len() as u64);
        for projectile in &self.guided_projectiles {
            hash_value(&mut hash, u64::from(projectile.impact.shooter_id));
            hash_value(&mut hash, u64::from(projectile.impact.shooter_side));
            hash_value(&mut hash, u64::from(projectile.impact.target_id));
            hash_value(&mut hash, u64::from(projectile.impact.point.x.to_bits()));
            hash_value(&mut hash, u64::from(projectile.impact.point.z.to_bits()));
            hash_value(&mut hash, u64::from(projectile.impact.damage.to_bits()));
            hash_value(&mut hash, u64::from(projectile.position.x.to_bits()));
            hash_value(&mut hash, u64::from(projectile.position.z.to_bits()));
            hash_value(&mut hash, u64::from(projectile.height.to_bits()));
            hash_value(&mut hash, u64::from(projectile.heading.to_bits()));
            hash_value(&mut hash, u64::from(projectile.vertical_speed.to_bits()));
            hash_value(&mut hash, u64::from(projectile.speed.to_bits()));
            hash_value(&mut hash, u64::from(projectile.turn_rate.to_bits()));
            hash_value(&mut hash, u64::from(projectile.remaining_ticks));
            hash_value(&mut hash, u64::from(projectile.damage_pulses));
        }
        hash
    }
}

#[inline]
fn hash_value(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
}

fn normalize_swapped_result(mut result: BattleResult) -> BattleResult {
    result.winner = result.winner.map(|side| 1 - side);
    result.sides.swap(0, 1);
    result.sides[0].side = 0;
    result.sides[1].side = 1;
    result
}

fn spawn_units(spawns: &[Spawn], definitions: &[Arc<crate::model::UnitDef>]) -> Vec<UnitState> {
    spawns
        .iter()
        .zip(definitions)
        .enumerate()
        .map(|(index, (spawn, definition))| {
            new_unit_state(
                index as u32 + 1,
                spawn.side,
                Arc::clone(definition),
                index,
                spawn.position,
                spawn.heading,
                spawn.definition.health,
            )
        })
        .collect()
}

fn new_unit_state(
    id: u32,
    side: u8,
    definition: Arc<crate::model::UnitDef>,
    definition_index: usize,
    position: Vec2,
    heading: f32,
    hp: f32,
) -> UnitState {
    UnitState {
        id,
        side,
        hp,
        position,
        heading,
        forward: Vec2::from_heading(heading),
        speed: 0.0,
        collision_height_scale: 1.0,
        paralyze_damage: 0.0,
        stunned: false,
        alive: hp > 0.0,
        weapons: initial_weapon_states(&definition, heading),
        definition,
        definition_index,
    }
}

fn initial_weapon_states(
    definition: &crate::model::UnitDef,
    heading: f32,
) -> Vec<crate::model::WeaponState> {
    definition
        .weapons
        .iter()
        .map(|weapon| crate::model::WeaponState {
            reload_until: 0.0,
            shots_left: 0,
            shot_sequence: 0,
            next_shot_at: 0.0,
            aim_heading: heading,
            aim_pitch: 0.0,
            predict_speed_mod: 1.0,
            next_prediction_update: 0.0,
            stockpiled_rounds: 0,
            stockpile_ready_at: 0.0,
            stockpile_in_progress: false,
            shield_charge: if weapon.is_shield {
                weapon
                    .shield_starting_power
                    .clamp(0.0, weapon.shield_power.max(0.0))
            } else {
                0.0
            },
            shield_regen_after: 0.0,
        })
        .collect()
}

fn reset_units(units: &mut [UnitState], spawns: &[Spawn]) {
    debug_assert_eq!(units.len(), spawns.len());
    for (index, (unit, spawn)) in units.iter_mut().zip(spawns).enumerate() {
        unit.id = index as u32 + 1;
        unit.side = spawn.side;
        unit.hp = spawn.definition.health;
        unit.position = spawn.position;
        unit.heading = spawn.heading;
        unit.forward = Vec2::from_heading(spawn.heading);
        unit.speed = 0.0;
        unit.paralyze_damage = 0.0;
        unit.stunned = false;
        unit.alive = true;
        unit.definition_index = index;
        debug_assert_eq!(unit.weapons.len(), spawn.definition.weapons.len());
        for (weapon_index, weapon) in unit.weapons.iter_mut().enumerate() {
            weapon.reload_until = 0.0;
            weapon.shots_left = 0;
            weapon.shot_sequence = 0;
            weapon.next_shot_at = 0.0;
            weapon.aim_heading = spawn.heading;
            weapon.aim_pitch = 0.0;
            weapon.predict_speed_mod = 1.0;
            weapon.next_prediction_update = 0.0;
            weapon.stockpiled_rounds = 0;
            weapon.stockpile_ready_at = 0.0;
            weapon.stockpile_in_progress = false;
            let definition = &spawn.definition.weapons[weapon_index];
            weapon.shield_charge = if definition.is_shield {
                definition
                    .shield_starting_power
                    .clamp(0.0, definition.shield_power.max(0.0))
            } else {
                0.0
            };
            weapon.shield_regen_after = 0.0;
        }
    }
}

fn initial_scores(units: &[UnitState]) -> [SideScore; 2] {
    let mut scores = [
        SideScore {
            side: 0,
            ..Default::default()
        },
        SideScore {
            side: 1,
            ..Default::default()
        },
    ];
    for unit in units.iter().filter(|unit| unit.side < 2) {
        scores[unit.side as usize].starting_value += unit.definition.resource_value();
    }
    scores
}

fn finish_scores(units: &[UnitState], scores: &mut [SideScore; 2], duration: f32) {
    for (side, score) in scores.iter_mut().enumerate() {
        score.duration = duration;
        score.won = side_alive(units, side as u8) && !side_alive(units, (1 - side) as u8);
        score.surviving_value = units
            .iter()
            .filter(|unit| unit.alive && unit.side as usize == side)
            .map(|unit| unit.definition.resource_value())
            .sum();
        score.remaining_hp_value = units
            .iter()
            .filter(|unit| unit.alive && unit.side as usize == side)
            .map(|unit| {
                unit.definition.resource_value()
                    * (unit.hp / unit.definition.health).clamp(0.0, 1.0)
            })
            .sum();
    }
    let starting = [scores[0].starting_value, scores[1].starting_value];
    let surviving = [scores[0].surviving_value, scores[1].surviving_value];
    for (side, score) in scores.iter_mut().enumerate() {
        score.enemy_value_destroyed = starting[1 - side] - surviving[1 - side];
    }
}

fn side_alive(units: &[UnitState], side: u8) -> bool {
    units.iter().any(|unit| unit.alive && unit.side == side)
}

fn count_live_sides(units: &[UnitState]) -> [usize; 2] {
    let mut counts = [0; 2];
    for unit in units {
        if unit.alive && unit.side < 2 {
            counts[unit.side as usize] += 1;
        }
    }
    counts
}

/// Recoil decays EMP damage against maximum health. BAR's default decline
/// period is forty seconds, so damage equal to 2.5% max health represents one
/// second of additional stun once the threshold has been crossed.
fn advance_paralysis(units: &mut [UnitState]) {
    for unit in units.iter_mut().filter(|unit| unit.alive) {
        if unit.paralyze_damage > 0.0 {
            let decline = unit.definition.health.max(0.0) / PARALYZE_DECLINE_TIME * DT;
            unit.paralyze_damage = (unit.paralyze_damage - decline).max(0.0);
        }
        unit.stunned = unit.paralyze_damage > unit.definition.health.max(0.0);
        if !unit.stunned {
            continue;
        }

        unit.speed = 0.0;
        // Recoil increments weapon reloadStatus while stunned. Moving the
        // absolute deadlines forward by one tick is the equivalent operation
        // in this engine and avoids adding a second countdown representation.
        for weapon in &mut unit.weapons {
            weapon.reload_until += DT;
            weapon.next_shot_at += DT;
            if weapon.stockpile_in_progress {
                weapon.stockpile_ready_at += DT;
            }
        }
    }
}

fn update_movement(
    units: &mut [UnitState],
    commands: &[Command],
    terrain: &Terrain,
    terrain_all_passable: bool,
    movement_masks: &[Option<Arc<[bool]>>],
) {
    for (unit, command) in units.iter_mut().zip(commands) {
        if !unit.alive {
            continue;
        }
        if unit.stunned {
            unit.speed = 0.0;
            continue;
        }
        let mask = movement_masks
            .get(unit.definition_index)
            .and_then(|mask| mask.as_deref());
        let movement = &unit.definition.movement;
        let Some(destination) = command.destination else {
            unit.speed = approach(unit.speed, 0.0, movement.braking * DT);
            unit.position = accepted_position(
                terrain,
                unit.position,
                unit.position + unit.velocity() * DT,
                movement.radius,
                terrain_all_passable,
                mask,
            );
            continue;
        };
        let desired_vector = destination - unit.position;
        if desired_vector.length_squared() < 0.25 {
            continue;
        }
        let travel_heading = desired_vector.heading();
        let forward_turn = angle_delta(unit.heading, travel_heading).abs();
        // Reversing saves a large hull turn and preserves turret aim when the
        // destination lies behind the unit. This is deterministic and needs no
        // policy/API expansion: units without reverse speed retain old behavior.
        let reversing = movement.reverse_speed > 0.0 && forward_turn > PI * 0.5;
        let desired_heading = if reversing {
            travel_heading + PI
        } else {
            travel_heading
        };
        let turn = angle_delta(unit.heading, desired_heading).abs();
        let turn_rate = movement.turn_rate.to_radians() * DT;
        unit.heading = turn_toward(unit.heading, desired_heading, turn_rate);
        unit.forward = Vec2::from_heading(unit.heading);

        let must_brake_to_turn =
            movement.turn_in_place && turn.to_degrees() > movement.turn_in_place_angle_limit;
        let desired_speed = if must_brake_to_turn {
            0.0
        } else if reversing {
            -movement.reverse_speed * command.throttle.clamp(0.0, 1.0)
        } else {
            movement.max_speed * command.throttle.clamp(0.0, 1.0)
        };
        let changing_direction = desired_speed.signum() != unit.speed.signum()
            && desired_speed.abs() > 1e-6
            && unit.speed.abs() > 1e-6;
        let acceleration_target = if changing_direction {
            0.0
        } else {
            desired_speed
        };
        let rate = if !changing_direction && desired_speed.abs() > unit.speed.abs() {
            movement.acceleration
        } else {
            movement.braking
        };
        unit.speed = approach(unit.speed, acceleration_target, rate * DT);
        let candidate = unit.position + unit.velocity() * DT;
        unit.position = accepted_position(
            terrain,
            unit.position,
            candidate,
            movement.radius,
            terrain_all_passable,
            mask,
        );
    }
}

fn accepted_position(
    terrain: &Terrain,
    old: Vec2,
    candidate: Vec2,
    radius: f32,
    terrain_all_passable: bool,
    passable_mask: Option<&[bool]>,
) -> Vec2 {
    if footprint_is_passable(
        terrain,
        candidate,
        radius,
        terrain_all_passable,
        passable_mask,
    ) {
        candidate
    } else {
        old
    }
}

#[inline]
fn footprint_is_passable(
    terrain: &Terrain,
    candidate: Vec2,
    radius: f32,
    terrain_all_passable: bool,
    passable_mask: Option<&[bool]>,
) -> bool {
    let radius = radius.max(0.0);
    let within_bounds = candidate.x - radius >= 0.0
        && candidate.z - radius >= 0.0
        && candidate.x + radius <= terrain.cols as f32 * terrain.cell_size
        && candidate.z + radius <= terrain.rows as f32 * terrain.cell_size;
    within_bounds
        && match passable_mask {
            Some(mask) => terrain.is_passable_with_radius_and_mask(candidate, radius, mask),
            None => terrain_all_passable || terrain.is_passable_with_radius(candidate, radius),
        }
}

fn separate_collisions(
    units: &mut [UnitState],
    terrain: &Terrain,
    spatial: &SpatialGrid,
    candidates: &mut Vec<usize>,
    indexed: bool,
    terrain_all_passable: bool,
    movement_masks: &[Option<Arc<[bool]>>],
) {
    for left in 0..units.len() {
        if !units[left].alive || !units[left].definition.movement.blocks_movement {
            continue;
        }
        if indexed {
            // The index describes pre-separation positions. Extra padding
            // keeps candidates that can enter contact through an earlier pair
            // correction in this deterministic single pass.
            let expansion = units[left].definition.movement.radius + spatial.maximum_radius() * 5.0;
            let position = units[left].position;
            spatial.query_aabb(
                Vec2::new(position.x - expansion, position.z - expansion),
                Vec2::new(position.x + expansion, position.z + expansion),
                candidates,
            );
            candidates.sort_unstable();
            for right in candidates.iter().copied().filter(|right| *right > left) {
                separate_unit_pair(
                    units,
                    terrain,
                    left,
                    right,
                    terrain_all_passable,
                    movement_masks,
                );
            }
        } else {
            for right in left + 1..units.len() {
                separate_unit_pair(
                    units,
                    terrain,
                    left,
                    right,
                    terrain_all_passable,
                    movement_masks,
                );
            }
        }
    }
}

fn separate_unit_pair(
    units: &mut [UnitState],
    terrain: &Terrain,
    left: usize,
    right: usize,
    terrain_all_passable: bool,
    movement_masks: &[Option<Arc<[bool]>>],
) {
    if !units[right].alive || !units[right].definition.movement.blocks_movement {
        return;
    }
    let delta = units[right].position - units[left].position;
    let minimum = units[left].definition.movement.radius + units[right].definition.movement.radius;
    let distance_squared = delta.length_squared();
    if distance_squared >= minimum * minimum || distance_squared <= 1e-10 {
        return;
    }
    let distance = distance_squared.sqrt();
    let left_mobile = units[left].definition.movement.max_speed > 0.0;
    let right_mobile = units[right].definition.movement.max_speed > 0.0;
    let overlap = minimum - distance;
    let direction = delta * distance.recip();
    let (left_share, right_share) = match (left_mobile, right_mobile) {
        (true, true) => (0.5, 0.5),
        (true, false) => (1.0, 0.0),
        (false, true) => (0.0, 1.0),
        (false, false) => (0.0, 0.0),
    };
    let left_position = units[left].position - direction * (overlap * left_share);
    let right_position = units[right].position + direction * (overlap * right_share);
    let left_radius = units[left].definition.movement.radius;
    let right_radius = units[right].definition.movement.radius;
    let left_mask = movement_masks
        .get(units[left].definition_index)
        .and_then(|mask| mask.as_deref());
    let right_mask = movement_masks
        .get(units[right].definition_index)
        .and_then(|mask| mask.as_deref());
    if left_mobile
        && footprint_is_passable(
            terrain,
            left_position,
            left_radius,
            terrain_all_passable,
            left_mask,
        )
    {
        units[left].position = left_position;
    }
    if right_mobile
        && footprint_is_passable(
            terrain,
            right_position,
            right_radius,
            terrain_all_passable,
            right_mask,
        )
    {
        units[right].position = right_position;
    }
}

#[allow(clippy::too_many_arguments)]
fn update_weapons<const TRACE: bool>(
    units: &mut [UnitState],
    commands: &[Command],
    time: f32,
    tick: u64,
    impacts: &mut ImpactQueue,
    guided_projectiles: &mut Vec<GuidedProjectile>,
    battle: &mut BattleState,
    seed: u64,
    terrain: &Terrain,
    definitions: &[Arc<crate::model::UnitDef>],
    spatial: &SpatialGrid,
    spatial_candidates: &mut Vec<usize>,
    trace_tick: bool,
    aim_attempts: &mut Vec<AimAttempt>,
) {
    advance_stockpiles(units, definitions, time, &mut battle.combat_resources);
    for index in 0..units.len() {
        if !units[index].alive || units[index].stunned || units[index].side >= 2 {
            continue;
        }
        let Some(target_id) = commands[index].target else {
            continue;
        };
        let Some(target_index) = live_unit_index(units, target_id) else {
            continue;
        };
        if units[target_index].side == units[index].side {
            continue;
        }
        let unit_position = units[index].position;
        let body_heading = units[index].heading;
        let moving = units[index].speed.abs() > 0.5;
        let side = units[index].side;
        let shooter_id = units[index].id;
        let definition = &definitions[units[index].definition_index];

        for (weapon_index, weapon) in definition.weapons.iter().enumerate() {
            if weapon.manual_fire
                || weapon.is_shield
                || weapon.interceptor != 0
                || !weapon_can_target(weapon, &units[target_index].definition)
                || weapon.damage_against(&units[target_index].definition.strategic.armor_class)
                    <= 0.0
            {
                continue;
            }
            let local_origin =
                weapon
                    .firing_origin
                    .unwrap_or([0.0, definition.movement.collision_mid_y(), 0.0]);
            let forward = units[index].forward;
            let right = Vec2::new(-forward.z, forward.x);
            let origin = unit_position + right * local_origin[0] + forward * local_origin[2];
            let origin_y = terrain.height_at(unit_position) + local_origin[1];
            let target_position = units[target_index].position;
            let target_y = unit_collision_mid_y(&units[target_index], terrain);
            let target_velocity = units[target_index].velocity();
            let predict_speed_mod = {
                let state = &mut units[index].weapons[weapon_index];
                if time + 1e-6 >= state.next_prediction_update {
                    // Recoil refreshes this imperfect velocity estimate during its
                    // slow update. A half-second cadence captures the behavior
                    // without coupling it to rendering or allocating projectile state.
                    state.predict_speed_mod = 1.0
                        + keyed_symmetric(seed, tick, shooter_id, weapon_index as u64, 0, 0, 1.0);
                    state.next_prediction_update = time + 0.5;
                }
                state.predict_speed_mod
            };
            let aim_point = predicted_target_position_3d(
                origin,
                origin_y,
                target_position,
                target_y,
                target_velocity,
                weapon,
                predict_speed_mod,
            );
            if origin.distance_squared(aim_point) > weapon.range * weapon.range {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    units[index].id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::Tracking,
                );
                continue;
            }
            let Some(flight_path) =
                weapon_flight_path(origin, origin_y, aim_point, target_y, weapon)
            else {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    units[index].id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::Tracking,
                );
                continue;
            };
            let desired_aim = (aim_point - origin).heading();
            let aim_rate = if weapon.turret {
                weapon
                    .turret_yaw_speed
                    .map_or(f32::INFINITY, f32::to_radians)
            } else {
                definition.movement.turn_rate.to_radians()
            };
            let tolerance = if weapon.fire_tolerance > 0.0 {
                weapon.fire_tolerance
            } else {
                5_f32.to_radians()
            };
            // Most catalog weapons expose no vertical script speed and retain
            // the zero-cost instant-pitch approximation. When the bridge does
            // provide one, terrain and firing-origin height participate in the
            // same readiness gate as turret yaw.
            let desired_pitch = weapon
                .turret_pitch_speed
                .map(|_| flight_path.launch_pitch(origin.distance(aim_point), origin_y, target_y));
            let (aim_ready, shot_ready) = {
                let state = &mut units[index].weapons[weapon_index];
                state.aim_heading = if aim_rate.is_infinite() {
                    desired_aim
                } else {
                    turn_toward(state.aim_heading, desired_aim, aim_rate * DT)
                };
                if !weapon.turret {
                    state.aim_heading = body_heading;
                }
                let yaw_ready = angle_delta(state.aim_heading, desired_aim).abs() <= tolerance;
                let pitch_ready = desired_pitch.is_none_or(|desired_pitch| {
                    let maximum_delta = weapon
                        .turret_pitch_speed
                        .expect("desired pitch requires a pitch speed")
                        .max(0.0)
                        .to_radians()
                        * DT;
                    let delta = desired_pitch - state.aim_pitch;
                    state.aim_pitch += delta.clamp(-maximum_delta, maximum_delta);
                    (desired_pitch - state.aim_pitch).abs() <= tolerance
                });
                let aim_ready = yaw_ready && pitch_ready;
                let ammunition_ready = !weapon.stockpile || state.stockpiled_rounds > 0;
                if aim_ready
                    && ammunition_ready
                    && state.shots_left == 0
                    && time + 1e-6 >= state.reload_until
                {
                    state.shots_left = weapon.burst.max(1);
                    state.next_shot_at = time;
                    state.reload_until = time + weapon.reload.max(DT);
                    if weapon.stockpile {
                        state.stockpiled_rounds -= 1;
                    }
                }
                let shot_ready =
                    aim_ready && state.shots_left > 0 && time + 1e-6 >= state.next_shot_at;
                (aim_ready, shot_ready)
            };
            if !aim_ready {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    shooter_id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::Tracking,
                );
                continue;
            }
            // A headless run needs obstruction work only when a shot can be
            // emitted. Traced runs retain blocked/reloading intent precedence
            // for the viewer.
            if !TRACE && !shot_ready {
                continue;
            }
            if !terrain_clear_for_flight_path(
                terrain,
                origin,
                aim_point,
                flight_path,
                PROJECTILE_CLEARANCE,
            ) {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    units[index].id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::BlockedTerrain,
                );
                continue;
            }

            // Recoil can withhold a shot when an ally occupies the line of
            // fire. A non-target enemy instead intercepts an ordinary round.
            let blocker = first_unit_blocker(
                units,
                terrain,
                index,
                target_index,
                origin,
                aim_point,
                flight_path,
                PROJECTILE_CLEARANCE,
                spatial,
                spatial_candidates,
            );
            if blocker.is_some_and(|blocker_index| {
                units[blocker_index].side == side && weapon.avoid_friendly
            }) {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    units[index].id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::BlockedFriendly,
                );
                continue;
            }
            if !shot_ready {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    shooter_id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::Reloading,
                );
                continue;
            }
            // Guided rounds retain their commanded target and perform swept
            // blocker collision while in flight. Ordinary rounds keep the
            // cheaper launch-time interception path.
            let effective_target_index = if weapon.tracks && !weapon.hitscan {
                target_index
            } else {
                blocker
                    .filter(|&blocker_index| {
                        units[blocker_index].side != side
                            && weapon.collide_enemy
                            && weapon.collide_non_target
                    })
                    .unwrap_or(target_index)
            };
            let effective_target_id = units[effective_target_index].id;
            let effective_target_position = units[effective_target_index].position;
            let effective_target_speed = units[effective_target_index].speed.abs();

            if !weapon.stockpile
                && !try_consume_weapon_resources(
                    &mut battle.combat_resources,
                    side,
                    weapon.energy_cost,
                    weapon.metal_cost,
                )
            {
                record_aim::<TRACE>(
                    trace_tick,
                    aim_attempts,
                    shooter_id,
                    weapon_index,
                    origin,
                    aim_point,
                    target_id,
                    AimState::Reloading,
                );
                continue;
            }

            let shot_sequence = {
                let state = &mut units[index].weapons[weapon_index];
                state.shots_left -= 1;
                state.next_shot_at += weapon.burst_rate.max(DT);
                let sequence = state.shot_sequence;
                state.shot_sequence = state.shot_sequence.wrapping_add(1);
                sequence
            };
            let effective_target_y = unit_collision_mid_y(&units[effective_target_index], terrain);
            fire_shot(
                origin,
                origin_y,
                shooter_id,
                effective_target_id,
                if effective_target_index == target_index {
                    aim_point
                } else {
                    effective_target_position
                },
                effective_target_y,
                effective_target_speed,
                moving,
                side,
                units[index].definition_index,
                weapon_index,
                weapon,
                tick,
                shot_sequence,
                impacts,
                guided_projectiles,
                &mut battle.scores[side as usize],
                units,
                &mut battle.combat_resources,
                time,
                seed,
            );
            record_aim::<TRACE>(
                trace_tick,
                aim_attempts,
                shooter_id,
                weapon_index,
                origin,
                aim_point,
                target_id,
                AimState::Fired,
            );
        }
    }
}

fn advance_stockpiles(
    units: &mut [UnitState],
    definitions: &[Arc<crate::model::UnitDef>],
    time: f32,
    resources: &mut Option<[StartingResources; 2]>,
) {
    for unit in units
        .iter_mut()
        .filter(|unit| unit.alive && !unit.stunned && unit.side < 2)
    {
        let definition = &definitions[unit.definition_index];
        for (weapon, state) in definition.weapons.iter().zip(&mut unit.weapons) {
            if weapon.is_shield {
                let maximum = weapon.shield_power.max(0.0);
                if time + 1e-6 >= state.shield_regen_after
                    && state.shield_charge + 1e-6 < maximum
                    && weapon.shield_power_regen > 0.0
                {
                    let recovered =
                        (weapon.shield_power_regen * DT).min(maximum - state.shield_charge);
                    let energy =
                        weapon.shield_power_regen_energy * (recovered / weapon.shield_power_regen);
                    if try_consume_weapon_resources(resources, unit.side, energy, 0.0) {
                        state.shield_charge += recovered;
                    }
                }
                continue;
            }
            if !weapon.stockpile {
                continue;
            }
            if state.stockpile_in_progress && time + 1e-6 >= state.stockpile_ready_at {
                state.stockpile_in_progress = false;
                state.stockpiled_rounds = 1;
            }
            if state.stockpiled_rounds == 0
                && !state.stockpile_in_progress
                && try_consume_weapon_resources(
                    resources,
                    unit.side,
                    weapon.energy_cost,
                    weapon.metal_cost,
                )
            {
                state.stockpile_in_progress = true;
                state.stockpile_ready_at = time + weapon.stockpile_time.max(DT);
            }
        }
    }
}

#[inline]
fn try_consume_weapon_resources(
    resources: &mut Option<[StartingResources; 2]>,
    side: u8,
    energy_cost: f32,
    metal_cost: f32,
) -> bool {
    let Some(resources) = resources else {
        return true;
    };
    let Some(pool) = resources.get_mut(side as usize) else {
        return false;
    };
    let energy_cost = f64::from(energy_cost.max(0.0));
    let metal_cost = f64::from(metal_cost.max(0.0));
    if pool.energy + 1e-9 < energy_cost || pool.metal + 1e-9 < metal_cost {
        return false;
    }
    pool.energy -= energy_cost;
    pool.metal -= metal_cost;
    true
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn record_aim<const TRACE: bool>(
    trace_tick: bool,
    attempts: &mut Vec<AimAttempt>,
    shooter: u32,
    weapon: usize,
    origin: Vec2,
    aim_point: Vec2,
    target: u32,
    state: AimState,
) {
    if TRACE && trace_tick {
        attempts.push(AimAttempt {
            shooter,
            weapon: weapon as u16,
            origin: [origin.x, origin.z],
            aim_point: [aim_point.x, aim_point.z],
            target: Some(target),
            state,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn unit_collision_vertical_bounds(unit: &UnitState, terrain: &Terrain) -> (f32, f32) {
    let (bottom, full_top) = unit
        .definition
        .movement
        .collision_vertical_bounds(terrain.height_at(unit.position));
    let top = bottom + (full_top - bottom) * unit.collision_height_scale;
    (bottom, top)
}

fn unit_collision_mid_y(unit: &UnitState, terrain: &Terrain) -> f32 {
    let (bottom, top) = unit_collision_vertical_bounds(unit, terrain);
    (bottom + top) * 0.5
}

/// Solves the requested gravity trajectory that reaches a fixed point with a
/// fixed muzzle speed. Recoil cannon rounds use this analytical path; it has
/// no per-tick cost for ordinary unguided projectiles.
fn ballistic_flight_path(
    origin: Vec2,
    origin_y: f32,
    target: Vec2,
    target_y: f32,
    speed: f32,
    gravity: f32,
    high_trajectory: bool,
) -> Option<FlightPath> {
    let horizontal_distance = origin.distance(target);
    if horizontal_distance <= 1e-5 || speed <= 1e-5 || gravity <= 1e-5 {
        return None;
    }
    let speed_squared = speed * speed;
    let delta_y = target_y - origin_y;
    let discriminant = speed_squared * speed_squared
        - gravity
            * (gravity * horizontal_distance * horizontal_distance + 2.0 * delta_y * speed_squared);
    if !discriminant.is_finite() || discriminant < 0.0 {
        return None;
    }
    // The smaller tangent is the direct/low trajectory. A unit's currently
    // selected high-trajectory state uses the other valid root instead.
    let root = discriminant.sqrt();
    let numerator = if high_trajectory {
        speed_squared + root
    } else {
        speed_squared - root
    };
    let tangent = numerator / (gravity * horizontal_distance);
    if !tangent.is_finite() {
        return None;
    }
    let horizontal_speed = speed / (1.0 + tangent * tangent).sqrt();
    if !horizontal_speed.is_finite() || horizontal_speed <= 1e-5 {
        return None;
    }
    let flight_time = horizontal_distance / horizontal_speed;
    let vertical_speed = horizontal_speed * tangent;
    (flight_time.is_finite() && vertical_speed.is_finite() && flight_time >= 0.0).then_some(
        FlightPath::Ballistic {
            origin_y,
            flight_time,
            horizontal_speed,
            vertical_speed,
            gravity,
        },
    )
}

fn weapon_flight_path(
    origin: Vec2,
    origin_y: f32,
    target: Vec2,
    target_y: f32,
    weapon: &WeaponDef,
) -> Option<FlightPath> {
    if weapon.hitscan || weapon.projectile_speed <= 1e-5 {
        return Some(FlightPath::Linear {
            from_y: origin_y,
            to_y: target_y,
            flight_time: 0.0,
        });
    }
    if !weapon.tracks && weapon.projectile_gravity > 1e-5 {
        return ballistic_flight_path(
            origin,
            origin_y,
            target,
            target_y,
            weapon.projectile_speed,
            weapon.projectile_gravity,
            weapon.high_trajectory,
        );
    }
    Some(FlightPath::Linear {
        from_y: origin_y,
        to_y: target_y,
        flight_time: origin.distance(target) / weapon.projectile_speed,
    })
}

fn terrain_clear_for_flight_path(
    terrain: &Terrain,
    origin: Vec2,
    target: Vec2,
    path: FlightPath,
    clearance: f32,
) -> bool {
    if matches!(path, FlightPath::Linear { .. }) {
        return terrain.has_line_of_sight_3d(
            origin,
            path.height_at_fraction(0.0),
            target,
            path.height_at_fraction(1.0),
            clearance,
        );
    }

    let distance = origin.distance(target);
    if distance <= f32::EPSILON {
        return terrain.height_at(origin) + clearance < path.height_at_fraction(0.0);
    }
    // Half a height-grid cell is sufficient to see any sampled ridge cell and
    // keeps arc validation off the ordinary straight-shot hot path.
    let spacing = (terrain.cell_size * 0.5).max(0.5);
    let steps = (distance / spacing).ceil().max(1.0) as u32;
    for step in 1..steps {
        let fraction = step as f32 / steps as f32;
        let position = origin + (target - origin) * fraction;
        if terrain.height_at(position) + clearance >= path.height_at_fraction(fraction) {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn first_unit_blocker(
    units: &[UnitState],
    terrain: &Terrain,
    shooter_index: usize,
    target_index: usize,
    from: Vec2,
    to: Vec2,
    path: FlightPath,
    projectile_radius: f32,
    spatial: &SpatialGrid,
    candidates: &mut Vec<usize>,
) -> Option<usize> {
    if !spatial.is_indexed() {
        return first_unit_blocker_in(
            units,
            terrain,
            shooter_index,
            target_index,
            from,
            to,
            path,
            projectile_radius,
            0..units.len(),
        );
    }
    let expansion = spatial.maximum_radius() + projectile_radius.max(0.0);
    spatial.query_aabb(
        Vec2::new(from.x.min(to.x) - expansion, from.z.min(to.z) - expansion),
        Vec2::new(from.x.max(to.x) + expansion, from.z.max(to.z) + expansion),
        candidates,
    );
    candidates.sort_unstable();
    first_unit_blocker_in(
        units,
        terrain,
        shooter_index,
        target_index,
        from,
        to,
        path,
        projectile_radius,
        candidates.iter().copied(),
    )
}

#[allow(clippy::too_many_arguments)]
fn first_unit_blocker_in<I>(
    units: &[UnitState],
    terrain: &Terrain,
    shooter_index: usize,
    target_index: usize,
    from: Vec2,
    to: Vec2,
    path: FlightPath,
    projectile_radius: f32,
    indices: I,
) -> Option<usize>
where
    I: IntoIterator<Item = usize>,
{
    let segment = to - from;
    let length_squared = segment.x * segment.x + segment.z * segment.z;
    if length_squared <= 1e-6 {
        return None;
    }
    let length = length_squared.sqrt();
    let mut first: Option<(usize, f32)> = None;
    for index in indices {
        let unit = &units[index];
        if index == shooter_index
            || index == target_index
            || !unit.alive
            || !unit.definition.movement.blocks_fire
        {
            continue;
        }
        let offset = unit.position - from;
        let centre_t = (offset.x * segment.x + offset.z * segment.z) / length_squared;
        if !(0.0..=1.0).contains(&centre_t) {
            continue;
        }
        let closest = from + segment * centre_t;
        let expanded_radius = unit.definition.movement.radius + projectile_radius.max(0.0);
        let perpendicular_squared = {
            let delta = closest - unit.position;
            delta.x * delta.x + delta.z * delta.z
        };
        if perpendicular_squared > expanded_radius * expanded_radius {
            continue;
        }
        let half_t = (expanded_radius * expanded_radius - perpendicular_squared)
            .max(0.0)
            .sqrt()
            / length;
        let enter_t = (centre_t - half_t).clamp(0.0, 1.0);
        let exit_t = (centre_t + half_t).clamp(0.0, 1.0);
        if exit_t <= 0.0 || enter_t >= 1.0 {
            continue;
        }
        let enter_y = path.height_at_fraction(enter_t);
        let exit_y = path.height_at_fraction(exit_t);
        let (bottom, top) = unit_collision_vertical_bounds(unit, terrain);
        let ray_bottom = enter_y.min(exit_y) - projectile_radius.max(0.0);
        let ray_top = enter_y.max(exit_y) + projectile_radius.max(0.0);
        if ray_top < bottom || ray_bottom > top {
            continue;
        }
        if first.is_none_or(|(_, previous_t)| enter_t < previous_t) {
            first = Some((index, enter_t));
        }
    }
    first.map(|(index, _)| index)
}

#[allow(clippy::too_many_arguments)]
fn fire_shot(
    origin: Vec2,
    origin_y: f32,
    shooter_id: u32,
    target_id: u32,
    aim_point: Vec2,
    target_y: f32,
    target_speed: f32,
    moving: bool,
    side: u8,
    weapon_definition_index: usize,
    weapon_index: usize,
    weapon: &WeaponDef,
    current_tick: u64,
    shot_sequence: u64,
    impacts: &mut ImpactQueue,
    guided_projectiles: &mut Vec<GuidedProjectile>,
    score: &mut SideScore,
    units: &mut [UnitState],
    resources: &mut Option<[StartingResources; 2]>,
    time: f32,
    seed: u64,
) {
    let distance = origin.distance(aim_point);
    let move_error = random_disc(
        seed,
        current_tick,
        shooter_id,
        weapon_index as u64,
        shot_sequence,
        1,
    ) * (weapon.target_move_error * target_speed);
    let base_aim = aim_point + move_error;
    let spread = if moving {
        weapon.moving_accuracy
    } else {
        weapon.accuracy
    };
    let duration_ticks = (weapon.damage_duration * crate::model::SIM_HZ)
        .ceil()
        .max(1.0) as u32;
    let per_tick_damage = weapon.damage / duration_ticks as f32;

    for projectile in 0..weapon.projectiles.max(1) {
        let projectile = projectile as u64;
        let error = keyed_symmetric(
            seed,
            current_tick,
            shooter_id,
            weapon_index as u64,
            shot_sequence,
            3 + projectile * 2,
            spread,
        ) + keyed_symmetric(
            seed,
            current_tick,
            shooter_id,
            weapon_index as u64,
            shot_sequence,
            4 + projectile * 2,
            weapon.spray_angle,
        );
        let point = origin + (base_aim - origin).rotate(error);
        let Some(flight_path) = weapon_flight_path(origin, origin_y, point, target_y, weapon)
        else {
            // A spread sample can fall outside the weapon's physical
            // ballistic envelope. Recoil would let it fall short; the fast
            // kernel records no target impact rather than teleporting it.
            continue;
        };
        let flight_time = flight_path.flight_time();
        score.projectiles_fired += 1;
        // Recoil interceptors launch their own projectiles and may miss. The
        // fast kernel instead resolves a targetable round at launch against
        // the first ready enemy interceptor (dense unit ID, then weapon
        // index) whose coverage contains its intended impact point. Ordinary
        // non-targetable rounds take only this zero-mask branch.
        if !weapon.hitscan
            && weapon.targetable != 0
            && try_intercept_projectile(units, side, weapon.targetable, point, time, resources)
        {
            continue;
        }
        if weapon.tracks && !weapon.hitscan {
            let projectile_speed = weapon.projectile_speed.max(0.0);
            let nominal_flight_time = if projectile_speed > 0.0 {
                flight_time
            } else {
                weapon.projectile_lifetime.max(DT)
            };
            let lifetime = if weapon.projectile_lifetime > 0.0 {
                weapon.projectile_lifetime
            } else if projectile_speed > 0.0 {
                weapon.range.max(distance) / projectile_speed
            } else {
                DT
            }
            .max(DT);
            guided_projectiles.push(GuidedProjectile {
                impact: Impact {
                    shooter_id,
                    shooter_side: side,
                    target_id,
                    point,
                    damage: per_tick_damage,
                    weapon_definition_index: weapon_definition_index as u32,
                    weapon_index: weapon_index as u16,
                    damage_fraction: 1.0 / duration_ticks as f32,
                    radius: weapon.area_of_effect,
                    edge_effectiveness: weapon.edge_effectiveness,
                    paralyzer: weapon.paralyzer,
                    paralyze_time: weapon.paralyze_time,
                    hits_allies: false,
                    collide_non_target: weapon.collide_enemy && weapon.collide_non_target,
                    track_target: false,
                    requires_source: duration_ticks > 1,
                },
                position: origin,
                height: origin_y,
                heading: (point - origin).heading(),
                vertical_speed: (target_y - origin_y) / nominal_flight_time.max(DT),
                speed: projectile_speed,
                turn_rate: weapon.projectile_turn_rate.max(0.0).to_radians(),
                remaining_ticks: (lifetime * crate::model::SIM_HZ).ceil().max(1.0) as u32,
                damage_pulses: duration_ticks,
            });
            continue;
        }
        for pulse in 0..duration_ticks {
            let delay = flight_time + pulse as f32 * DT;
            let due_tick = current_tick + delay_to_ticks(delay);
            impacts.schedule(
                due_tick,
                Impact {
                    shooter_id,
                    shooter_side: side,
                    target_id,
                    point,
                    damage: per_tick_damage,
                    weapon_definition_index: weapon_definition_index as u32,
                    weapon_index: weapon_index as u16,
                    damage_fraction: 1.0 / duration_ticks as f32,
                    radius: weapon.area_of_effect,
                    edge_effectiveness: weapon.edge_effectiveness,
                    paralyzer: weapon.paralyzer,
                    paralyze_time: weapon.paralyze_time,
                    hits_allies: false,
                    collide_non_target: weapon.collide_enemy && weapon.collide_non_target,
                    track_target: weapon.hitscan && duration_ticks > 1,
                    requires_source: duration_ticks > 1,
                },
            );
        }
    }
}

fn try_intercept_projectile(
    units: &mut [UnitState],
    projectile_side: u8,
    targetable: u32,
    impact_point: Vec2,
    time: f32,
    resources: &mut Option<[StartingResources; 2]>,
) -> bool {
    debug_assert_ne!(targetable, 0);
    for unit in units.iter_mut() {
        if !unit.alive || unit.stunned || unit.side >= 2 || unit.side == projectile_side {
            continue;
        }
        for (weapon, state) in unit.definition.weapons.iter().zip(&mut unit.weapons) {
            if weapon.interceptor & targetable == 0
                || unit.position.distance_squared(impact_point)
                    > weapon.coverage_range * weapon.coverage_range
            {
                continue;
            }
            if weapon.stockpile {
                if state.stockpiled_rounds == 0 {
                    continue;
                }
                state.stockpiled_rounds -= 1;
                return true;
            }
            if time + 1e-6 < state.reload_until
                || !try_consume_weapon_resources(
                    resources,
                    unit.side,
                    weapon.energy_cost,
                    weapon.metal_cost,
                )
            {
                continue;
            }
            state.reload_until = time + weapon.reload.max(DT);
            return true;
        }
    }
    false
}

/// Recoil's ordinary direct-fire lead uses distance / projectile speed and a
/// periodically noisy estimate of target velocity. `predictBoost` blends that
/// estimate toward 1.0; it is not itself a lead multiplier.
#[cfg(test)]
fn predicted_target_position(
    origin: Vec2,
    target_position: Vec2,
    target_velocity: Vec2,
    weapon: &WeaponDef,
    predict_speed_mod: f32,
) -> Vec2 {
    predicted_target_position_3d(
        origin,
        0.0,
        target_position,
        0.0,
        target_velocity,
        weapon,
        predict_speed_mod,
    )
}

/// Lead an airborne round by repeatedly solving its actual travel time. The
/// fixed-point iterations are intentionally bounded: they run only while a
/// weapon is aiming, not for every projectile tick.
fn predicted_target_position_3d(
    origin: Vec2,
    origin_y: f32,
    target_position: Vec2,
    target_y: f32,
    target_velocity: Vec2,
    weapon: &WeaponDef,
    predict_speed_mod: f32,
) -> Vec2 {
    if weapon.hitscan || weapon.projectile_speed <= 0.0 {
        return target_position;
    }
    let boost = weapon.predict_boost.clamp(0.0, 1.0);
    let prediction_multiplier = predict_speed_mod * (1.0 - boost) + boost;
    let mut lead = if !weapon.tracks && weapon.projectile_gravity > 1e-5 {
        let mut predicted = target_position;
        for _ in 0..3 {
            let Some(path) = weapon_flight_path(origin, origin_y, predicted, target_y, weapon)
            else {
                break;
            };
            let next =
                target_position + target_velocity * (path.flight_time() * prediction_multiplier);
            if next.distance_squared(predicted) <= 1e-6 {
                predicted = next;
                break;
            }
            predicted = next;
        }
        predicted - target_position
    } else {
        // Preserve the established Recoil-style nominal-time lead for
        // gravity-free and guided paths. Only a real arc needs fixed-point
        // travel-time refinement.
        target_velocity
            * (origin.distance(target_position) / weapon.projectile_speed * prediction_multiplier)
    };
    if weapon.lead_limit >= 0.0 {
        let length_squared = lead.length_squared();
        let limit_squared = weapon.lead_limit * weapon.lead_limit;
        if length_squared > limit_squared && length_squared > 1e-12 {
            lead = lead * (weapon.lead_limit / length_squared.sqrt());
        }
    }
    target_position + lead
}

#[inline]
fn weapon_can_target(weapon: &WeaponDef, target: &crate::model::UnitDef) -> bool {
    weapon.only_target_categories.is_empty()
        || weapon.only_target_categories.iter().any(|allowed| {
            target
                .strategic
                .target_categories
                .iter()
                .any(|category| category == allowed)
        })
}

#[allow(clippy::too_many_arguments)]
fn advance_guided_projectiles(
    units: &[UnitState],
    tick: u64,
    projectiles: &mut Vec<GuidedProjectile>,
    impacts: &mut ImpactQueue,
    terrain: &Terrain,
    spatial: &SpatialGrid,
    spatial_candidates: &mut Vec<usize>,
) {
    if projectiles.is_empty() {
        return;
    }
    let mut write = 0;
    for read in 0..projectiles.len() {
        let mut projectile = projectiles[read];
        if let Some(target_index) = live_unit_index(units, projectile.impact.target_id) {
            let target_delta = units[target_index].position - projectile.position;
            if target_delta.length_squared() > 1e-8 {
                projectile.heading = turn_toward(
                    projectile.heading,
                    target_delta.heading(),
                    projectile.turn_rate * DT,
                );
            }
            let target_y = unit_collision_mid_y(&units[target_index], terrain);
            let remaining_time = (target_delta.length() / projectile.speed.max(1e-6)).max(DT);
            projectile.vertical_speed = (target_y - projectile.height) / remaining_time;
        }

        let start = projectile.position;
        let start_y = projectile.height;
        let end = start + Vec2::from_heading(projectile.heading) * (projectile.speed * DT);
        let end_y = start_y + projectile.vertical_speed * DT;
        let collision = first_guided_collision(
            units,
            &projectile,
            start,
            start_y,
            end,
            end_y,
            terrain,
            spatial,
            spatial_candidates,
        );
        let terrain_clear =
            terrain.has_line_of_sight_3d(start, start_y, end, end_y, PROJECTILE_CLEARANCE);

        if let Some((target_index, hit_t)) = collision {
            let mut impact = projectile.impact;
            impact.target_id = units[target_index].id;
            impact.point = start + (end - start) * hit_t;
            schedule_guided_impact(tick, projectile.damage_pulses, impact, impacts);
            continue;
        }
        if !terrain_clear {
            let mut impact = projectile.impact;
            impact.target_id = 0;
            impact.point = end;
            schedule_guided_impact(tick, projectile.damage_pulses, impact, impacts);
            continue;
        }

        projectile.position = end;
        projectile.height = end_y;
        projectile.remaining_ticks = projectile.remaining_ticks.saturating_sub(1);
        if projectile.remaining_ticks == 0 {
            continue;
        }
        projectiles[write] = projectile;
        write += 1;
    }
    projectiles.truncate(write);
}

fn schedule_guided_impact(
    tick: u64,
    damage_pulses: u32,
    impact: Impact,
    impacts: &mut ImpactQueue,
) {
    for pulse in 0..damage_pulses {
        impacts.schedule(tick.saturating_add(u64::from(pulse)), impact);
    }
}

#[allow(clippy::too_many_arguments)]
fn first_guided_collision(
    units: &[UnitState],
    projectile: &GuidedProjectile,
    from: Vec2,
    from_y: f32,
    to: Vec2,
    to_y: f32,
    terrain: &Terrain,
    spatial: &SpatialGrid,
    candidates: &mut Vec<usize>,
) -> Option<(usize, f32)> {
    if spatial.is_indexed() {
        let expansion = spatial.maximum_radius() + PROJECTILE_CLEARANCE;
        spatial.query_aabb(
            Vec2::new(from.x.min(to.x) - expansion, from.z.min(to.z) - expansion),
            Vec2::new(from.x.max(to.x) + expansion, from.z.max(to.z) + expansion),
            candidates,
        );
        candidates.sort_unstable();
        first_guided_collision_in(
            units,
            projectile,
            from,
            from_y,
            to,
            to_y,
            terrain,
            candidates.iter().copied(),
        )
    } else {
        first_guided_collision_in(
            units,
            projectile,
            from,
            from_y,
            to,
            to_y,
            terrain,
            0..units.len(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn first_guided_collision_in<I>(
    units: &[UnitState],
    projectile: &GuidedProjectile,
    from: Vec2,
    from_y: f32,
    to: Vec2,
    to_y: f32,
    terrain: &Terrain,
    indices: I,
) -> Option<(usize, f32)>
where
    I: IntoIterator<Item = usize>,
{
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared <= 1e-8 {
        return None;
    }
    let mut first = None;
    for index in indices {
        let unit = &units[index];
        let designated_target = unit.id == projectile.impact.target_id;
        if !unit.alive
            || unit.id == projectile.impact.shooter_id
            || (unit.side == projectile.impact.shooter_side && !projectile.impact.hits_allies)
            || (!designated_target && !projectile.impact.collide_non_target)
        {
            continue;
        }
        let offset = unit.position - from;
        let centre_t = (offset.x * segment.x + offset.z * segment.z) / length_squared;
        let closest_t = centre_t.clamp(0.0, 1.0);
        let closest = from + segment * closest_t;
        let radius = unit.definition.movement.radius + PROJECTILE_CLEARANCE;
        let perpendicular_squared = closest.distance_squared(unit.position);
        if perpendicular_squared > radius * radius {
            continue;
        }
        let half_t =
            (radius * radius - perpendicular_squared).max(0.0).sqrt() / length_squared.sqrt();
        let enter_t = (centre_t - half_t).clamp(0.0, 1.0);
        let hit_y = from_y + (to_y - from_y) * enter_t;
        let (bottom, top) = unit_collision_vertical_bounds(unit, terrain);
        if hit_y + PROJECTILE_CLEARANCE < bottom || hit_y - PROJECTILE_CLEARANCE > top {
            continue;
        }
        if first.is_none_or(|(_, previous_t)| enter_t < previous_t) {
            first = Some((index, enter_t));
        }
    }
    first
}

#[allow(clippy::too_many_arguments)]
fn resolve_impacts(
    units: &mut [UnitState],
    tick: u64,
    impacts: &mut ImpactQueue,
    damage_events: &mut Vec<DamageEvent>,
    spatial: &SpatialGrid,
    spatial_candidates: &mut Vec<usize>,
    battle: &mut BattleState,
    definitions: &[Arc<crate::model::UnitDef>],
) {
    damage_events.clear();
    let Some(head) = impacts.heads.get_mut(tick as usize) else {
        return;
    };
    let mut node_index = std::mem::replace(head, -1);
    impacts.tails[tick as usize] = -1;
    while node_index >= 0 {
        let node = impacts.nodes[node_index as usize];
        let impact = node.impact;
        impacts.nodes[node_index as usize].next = impacts.free_head;
        impacts.free_head = node_index;
        node_index = node.next;
        impacts.pending -= 1;
        // Channeled beam/DoT ticks cease with their source. Ordinary
        // projectiles remain live after their shooter is destroyed.
        if impact.requires_source && live_unit_index(units, impact.shooter_id).is_none() {
            continue;
        }
        let point = if impact.track_target {
            live_unit_index(units, impact.target_id)
                .map_or(impact.point, |index| units[index].position)
        } else {
            impact.point
        };
        if try_absorb_impact_with_shield(units, definitions, &impact, point, tick as f32 * DT) {
            continue;
        }
        // The overwhelmingly common direct hit can resolve by dense target ID
        // without touching the spatial index or scanning the army. A miss still
        // falls through to incidental collision at the sampled impact point.
        let direct_hit = (impact.radius <= 0.0)
            .then(|| collect_direct_target_hit(units, definitions, &impact, point, damage_events))
            .flatten();
        let landed = if let Some(landed) = direct_hit {
            landed
        } else if spatial.is_indexed() {
            let expansion = impact.radius.max(0.0) + spatial.maximum_radius();
            spatial.query_aabb(
                Vec2::new(point.x - expansion, point.z - expansion),
                Vec2::new(point.x + expansion, point.z + expansion),
                spatial_candidates,
            );
            spatial_candidates.sort_unstable();
            collect_impact_hits(
                units,
                definitions,
                &impact,
                point,
                spatial_candidates.iter().copied(),
                damage_events,
            )
        } else {
            collect_impact_hits(
                units,
                definitions,
                &impact,
                point,
                0..units.len(),
                damage_events,
            )
        };
        if landed && impact.shooter_side < 2 {
            battle.scores[impact.shooter_side as usize].damage_events_hit += 1;
        }
    }
    // All ordinary damage due on a frame is accumulated before deaths are
    // removed. EMP uses its own pool and can stun immediately without
    // consuming hit points.
    for event in damage_events.iter().copied() {
        let unit = &mut units[event.target_index];
        if event.paralyzer {
            let threshold = unit.definition.health.max(0.0);
            let multiplier = unit.definition.strategic.paralyze_multiplier.max(0.0);
            let maximum =
                threshold + threshold * event.paralyze_time.max(0.0) / PARALYZE_DECLINE_TIME;
            unit.paralyze_damage = (unit.paralyze_damage + event.damage * multiplier).min(maximum);
            unit.stunned = unit.paralyze_damage > threshold;
            if unit.stunned {
                unit.speed = 0.0;
            }
            continue;
        }

        let actual = event.damage.min(unit.hp.max(0.0));
        let overkill = (event.damage - actual).max(0.0);
        unit.hp -= event.damage;
        if event.source_side < 2 && event.source_side != unit.side {
            battle.scores[event.source_side as usize].damage_dealt += actual;
            battle.scores[event.source_side as usize].overkill_damage += overkill;
        }
        if unit.side < 2 {
            battle.scores[unit.side as usize].damage_received += actual;
        }
    }
    // Only units touched this tick can have died. Repeated indices are safe
    // because `alive` flips exactly once after all simultaneous damage lands.
    for event in damage_events.iter().copied() {
        let index = event.target_index;
        let unit = &mut units[index];
        if unit.alive && unit.hp <= 0.0 {
            unit.alive = false;
            if unit.side < 2 {
                battle.alive_counts[unit.side as usize] -= 1;
            }
            if let Some(explosion) = &unit.definition.strategic.death_explosion
                && explosion.damage > 0.0
                && explosion.radius > 0.0
            {
                // Death is finalized after this tick's simultaneous damage.
                // Queueing for the next tick keeps the intrusive calendar
                // valid and gives chain reactions a stable entity-ID order.
                impacts.schedule(
                    tick.saturating_add(1),
                    Impact {
                        shooter_id: unit.id,
                        shooter_side: unit.side,
                        target_id: 0,
                        point: unit.position,
                        damage: explosion.damage,
                        weapon_definition_index: u32::MAX,
                        weapon_index: u16::MAX,
                        damage_fraction: 1.0,
                        radius: explosion.radius,
                        edge_effectiveness: explosion.edge_effectiveness,
                        paralyzer: false,
                        paralyze_time: 0.0,
                        hits_allies: true,
                        collide_non_target: true,
                        track_target: false,
                        requires_source: false,
                    },
                );
            }
        }
    }
}

fn try_absorb_impact_with_shield(
    units: &mut [UnitState],
    definitions: &[Arc<crate::model::UnitDef>],
    impact: &Impact,
    point: Vec2,
    time: f32,
) -> bool {
    let Some(projectile_weapon) = definitions
        .get(impact.weapon_definition_index as usize)
        .and_then(|definition| definition.weapons.get(impact.weapon_index as usize))
    else {
        return false;
    };
    let shield_class = projectile_weapon.intercepted_by_shield_type;
    if shield_class == 0 {
        return false;
    }
    let shield_damage = projectile_weapon
        .damage_by_armor
        .get("shields")
        .copied()
        .unwrap_or(projectile_weapon.damage)
        * impact.damage_fraction;
    if shield_damage <= 0.0 {
        return false;
    }

    // Stable dense-unit/weapon order resolves overlapping shields without RNG.
    // `exterior_shield` is the only ownership distinction represented: a
    // non-exterior shield covers its generator, while an exterior one covers
    // any allied impact point in its radius.
    for unit in units.iter_mut() {
        if !unit.alive || unit.stunned || unit.side >= 2 || unit.side == impact.shooter_side {
            continue;
        }
        for (weapon, state) in unit.definition.weapons.iter().zip(&mut unit.weapons) {
            if !weapon.is_shield
                || weapon.shield_intercept_type & shield_class == 0
                || (!weapon.exterior_shield && unit.id != impact.target_id)
                || unit.position.distance_squared(point)
                    > weapon.shield_radius * weapon.shield_radius
                || state.shield_charge + 1e-6 < shield_damage
            {
                continue;
            }
            state.shield_charge = (state.shield_charge - shield_damage).max(0.0);
            state.shield_regen_after = time + weapon.shield_recharge_delay.max(0.0);
            return true;
        }
    }
    false
}

fn collect_direct_target_hit(
    units: &[UnitState],
    definitions: &[Arc<crate::model::UnitDef>],
    impact: &Impact,
    point: Vec2,
    damage_events: &mut Vec<DamageEvent>,
) -> Option<bool> {
    let index = live_unit_index(units, impact.target_id)?;
    let target = &units[index];
    if target.side == impact.shooter_side && !impact.hits_allies {
        return None;
    }
    let radius = target.definition.movement.radius;
    if point.distance_squared(target.position) > radius * radius {
        return None;
    }
    damage_events.push(DamageEvent {
        target_index: index,
        source_side: impact.shooter_side,
        damage: impact_damage_against(impact, target, definitions),
        paralyzer: impact.paralyzer,
        paralyze_time: impact.paralyze_time,
    });
    Some(true)
}

fn collect_impact_hits<I>(
    units: &[UnitState],
    definitions: &[Arc<crate::model::UnitDef>],
    impact: &Impact,
    point: Vec2,
    indices: I,
    damage_events: &mut Vec<DamageEvent>,
) -> bool
where
    I: IntoIterator<Item = usize>,
{
    let mut landed = false;
    for index in indices {
        let unit = &units[index];
        if !unit.alive
            || unit.id == impact.shooter_id
            || (unit.side == impact.shooter_side && !impact.hits_allies)
        {
            continue;
        }
        let distance_squared = point.distance_squared(unit.position);
        let hit_radius = if impact.radius > 0.0 {
            impact.radius + unit.definition.movement.radius
        } else {
            unit.definition.movement.radius
        };
        if distance_squared > hit_radius * hit_radius {
            continue;
        }
        let factor = if impact.radius <= 0.0 {
            1.0
        } else {
            let distance = distance_squared.sqrt();
            let normalized = (distance / impact.radius).clamp(0.0, 1.0);
            impact.edge_effectiveness.clamp(0.0, 1.0)
                + (1.0 - impact.edge_effectiveness.clamp(0.0, 1.0)) * (1.0 - normalized)
        };
        damage_events.push(DamageEvent {
            target_index: index,
            source_side: impact.shooter_side,
            damage: impact_damage_against(impact, unit, definitions) * factor,
            paralyzer: impact.paralyzer,
            paralyze_time: impact.paralyze_time,
        });
        landed = true;
    }
    landed
}

#[inline]
fn impact_damage_against(
    impact: &Impact,
    target: &UnitState,
    definitions: &[Arc<crate::model::UnitDef>],
) -> f32 {
    definitions
        .get(impact.weapon_definition_index as usize)
        .and_then(|definition| definition.weapons.get(impact.weapon_index as usize))
        .map_or(impact.damage, |weapon| {
            if weapon.damage_by_armor.is_empty() {
                impact.damage
            } else {
                weapon.damage_against(&target.definition.strategic.armor_class)
                    * impact.damage_fraction
            }
        })
}

#[inline]
fn delay_to_ticks(delay: f32) -> u64 {
    (((delay - 1e-6).max(0.0) / DT).ceil()) as u64
}

#[inline]
fn live_unit_index(units: &[UnitState], id: u32) -> Option<usize> {
    let index = id.checked_sub(1)? as usize;
    units
        .get(index)
        .is_some_and(|unit| unit.alive && unit.id == id)
        .then_some(index)
}

fn approach(value: f32, target: f32, amount: f32) -> f32 {
    if value < target {
        (value + amount).min(target)
    } else {
        (value - amount).max(target)
    }
}

fn random_disc(seed: u64, tick: u64, entity: u32, weapon: u64, shot: u64, stream: u64) -> Vec2 {
    let radius = keyed_f32(seed, tick, entity, weapon, shot, stream).sqrt();
    let heading = keyed_f32(seed, tick, entity, weapon, shot, stream + 1) * PI * 2.0;
    Vec2::from_heading(heading) * radius
}

#[inline]
fn keyed_symmetric(
    seed: u64,
    tick: u64,
    entity: u32,
    weapon: u64,
    shot: u64,
    stream: u64,
    magnitude: f32,
) -> f32 {
    (keyed_f32(seed, tick, entity, weapon, shot, stream) * 2.0 - 1.0) * magnitude
}

#[inline]
fn keyed_f32(seed: u64, tick: u64, entity: u32, weapon: u64, shot: u64, stream: u64) -> f32 {
    let mut value = seed ^ 0x9E37_79B9_7F4A_7C15;
    value = mix64(value ^ tick.wrapping_mul(0xD1B5_4A32_D192_ED03));
    value = mix64(value ^ u64::from(entity).wrapping_mul(0x94D0_49BB_1331_11EB));
    value = mix64(value ^ weapon.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    value = mix64(value ^ shot.wrapping_mul(0xDB4F_0B91_75AE_2165));
    value = mix64(value ^ stream.wrapping_mul(0xA24B_AED4_963E_E407));
    (value >> 40) as f32 / (1_u32 << 24) as f32
}

#[inline]
fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod lead_tests {
    use super::*;

    fn projectile() -> WeaponDef {
        WeaponDef {
            projectile_speed: 100.0,
            predict_boost: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn perfect_prediction_leads_by_nominal_flight_time() {
        let aim = predicted_target_position(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(0.0, 10.0),
            &projectile(),
            0.25,
        );
        assert!((aim.x - 100.0).abs() < 1e-5);
        assert!((aim.z - 10.0).abs() < 1e-5);
    }

    #[test]
    fn random_samples_are_keyed_instead_of_consumption_ordered() {
        let expected = keyed_f32(7, 30, 42, 2, 5, 9);
        let unrelated = keyed_f32(7, 30, 99, 2, 5, 9);
        assert_eq!(keyed_f32(7, 30, 42, 2, 5, 9), expected);
        assert_ne!(unrelated, expected);
        assert_ne!(keyed_f32(7, 31, 42, 2, 5, 9), expected);
        assert_ne!(keyed_f32(7, 30, 42, 2, 6, 9), expected);
    }

    #[test]
    fn weapon_target_categories_are_enforced() {
        let mut weapon = WeaponDef {
            only_target_categories: vec!["VTOL".into()],
            ..Default::default()
        };
        let mut target = crate::model::UnitDef {
            name: "target".into(),
            icon_type: None,
            health: 1.0,
            metal_cost: 0.0,
            energy_cost: 0.0,
            strategic: crate::model::StrategicDef::default(),
            movement: crate::model::MovementDef::default(),
            weapons: Vec::new(),
        };
        target.strategic.target_categories = vec!["MOBILE".into(), "VTOL".into()];
        assert!(weapon_can_target(&weapon, &target));
        weapon.only_target_categories = vec!["SURFACE".into()];
        assert!(!weapon_can_target(&weapon, &target));
        weapon.only_target_categories.clear();
        assert!(weapon_can_target(&weapon, &target));
    }

    #[test]
    fn lead_limit_caps_prediction_displacement() {
        let weapon = WeaponDef {
            lead_limit: 4.0,
            ..projectile()
        };
        let aim = predicted_target_position(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(0.0, 10.0),
            &weapon,
            1.0,
        );
        assert!((aim.z - 4.0).abs() < 1e-5);
    }

    #[test]
    fn hitscan_does_not_lead() {
        let weapon = WeaponDef {
            hitscan: true,
            ..projectile()
        };
        let target = Vec2::new(100.0, 20.0);
        assert_eq!(
            predicted_target_position(
                Vec2::new(0.0, 0.0),
                target,
                Vec2::new(0.0, 10.0),
                &weapon,
                1.0,
            ),
            target
        );
    }

    #[test]
    fn impact_delays_are_bucketed_on_the_first_due_tick() {
        assert_eq!(delay_to_ticks(0.0), 0);
        assert_eq!(delay_to_ticks(DT), 1);
        assert_eq!(delay_to_ticks(DT + 1e-4), 2);

        let mut queue = ImpactQueue::new(20);
        let impact = Impact {
            shooter_id: 1,
            shooter_side: 0,
            target_id: 2,
            point: Vec2::default(),
            damage: 1.0,
            weapon_definition_index: u32::MAX,
            weapon_index: u16::MAX,
            damage_fraction: 1.0,
            radius: 0.0,
            edge_effectiveness: 0.0,
            paralyzer: false,
            paralyze_time: 0.0,
            hits_allies: false,
            collide_non_target: true,
            track_target: false,
            requires_source: false,
        };
        queue.schedule(7, impact);
        queue.schedule(7, impact);
        queue.schedule(11, impact);
        assert_eq!(queue.pending, 3);
        assert!(queue.heads[..7].iter().all(|head| *head == -1));
        assert_ne!(queue.heads[7], -1);
        assert_ne!(queue.heads[11], -1);
        let first = queue.heads[7] as usize;
        assert_ne!(queue.nodes[first].next, -1);
        assert_eq!(queue.nodes[queue.nodes[first].next as usize].next, -1);
    }

    #[test]
    fn direct_impact_uses_target_slot_and_miss_falls_through() {
        let definition = crate::model::UnitDef {
            name: "target".into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 1.0,
            energy_cost: 0.0,
            strategic: Default::default(),
            movement: crate::model::MovementDef {
                radius: 10.0,
                ..Default::default()
            },
            weapons: Vec::new(),
        };
        let spawns = vec![Spawn {
            side: 1,
            definition: definition.clone(),
            position: Vec2::new(20.0, 30.0),
            heading: 0.0,
        }];
        let definitions = vec![Arc::new(definition)];
        let units = spawn_units(&spawns, &definitions);
        let impact = Impact {
            shooter_id: 2,
            shooter_side: 0,
            target_id: 1,
            point: spawns[0].position,
            damage: 7.0,
            weapon_definition_index: u32::MAX,
            weapon_index: u16::MAX,
            damage_fraction: 1.0,
            radius: 0.0,
            edge_effectiveness: 0.0,
            paralyzer: false,
            paralyze_time: 0.0,
            hits_allies: false,
            collide_non_target: true,
            track_target: false,
            requires_source: false,
        };
        let mut damage_events = Vec::new();

        assert_eq!(
            collect_direct_target_hit(
                &units,
                &definitions,
                &impact,
                impact.point,
                &mut damage_events,
            ),
            Some(true)
        );
        assert_eq!(
            damage_events,
            vec![DamageEvent {
                target_index: 0,
                source_side: 0,
                damage: 7.0,
                paralyzer: false,
                paralyze_time: 0.0,
            }]
        );
        damage_events.clear();
        assert_eq!(
            collect_direct_target_hit(
                &units,
                &definitions,
                &impact,
                Vec2::new(100.0, 100.0),
                &mut damage_events,
            ),
            None
        );
        assert!(damage_events.is_empty());
    }

    fn tactical_unit(
        name: &str,
        movement: crate::model::MovementDef,
        weapons: Vec<WeaponDef>,
    ) -> crate::model::UnitDef {
        crate::model::UnitDef {
            name: name.into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 1.0,
            energy_cost: 0.0,
            strategic: Default::default(),
            movement,
            weapons,
        }
    }

    #[test]
    fn unit_reverses_toward_destination_behind_hull() {
        let definition = tactical_unit(
            "reverser",
            crate::model::MovementDef {
                max_speed: 60.0,
                reverse_speed: 30.0,
                acceleration: 30.0,
                braking: 60.0,
                turn_rate: 90.0,
                turn_in_place: false,
                radius: 1.0,
                ..Default::default()
            },
            Vec::new(),
        );
        let spawns = vec![Spawn {
            side: 0,
            definition: definition.clone(),
            position: Vec2::new(100.0, 100.0),
            heading: 0.0,
        }];
        let definitions = vec![Arc::new(definition)];
        let mut units = spawn_units(&spawns, &definitions);
        let commands = [Command {
            destination: Some(Vec2::new(0.0, 100.0)),
            throttle: 1.0,
            ..Default::default()
        }];

        update_movement(
            &mut units,
            &commands,
            &Terrain::flat(200.0, 200.0, 32.0),
            true,
            &[],
        );

        assert!(units[0].speed < 0.0);
        assert!(units[0].position.x < 100.0);
        assert!(angle_delta(0.0, units[0].heading).abs() < 1e-5);
    }

    #[test]
    fn impact_uses_targets_armor_class_override() {
        let weapon = WeaponDef {
            damage: 10.0,
            damage_by_armor: [("heavy".to_owned(), 25.0)].into_iter().collect(),
            ..Default::default()
        };
        let shooter = tactical_unit("shooter", Default::default(), vec![weapon]);
        let mut target = tactical_unit(
            "target",
            crate::model::MovementDef {
                radius: 10.0,
                ..Default::default()
            },
            Vec::new(),
        );
        target.strategic.armor_class = "heavy".into();
        let spawns = vec![
            Spawn {
                side: 0,
                definition: shooter.clone(),
                position: Vec2::new(0.0, 0.0),
                heading: 0.0,
            },
            Spawn {
                side: 1,
                definition: target.clone(),
                position: Vec2::new(20.0, 30.0),
                heading: 0.0,
            },
        ];
        let definitions = vec![Arc::new(shooter), Arc::new(target)];
        let units = spawn_units(&spawns, &definitions);
        let impact = Impact {
            shooter_id: 1,
            shooter_side: 0,
            target_id: 2,
            point: spawns[1].position,
            damage: 10.0,
            weapon_definition_index: 0,
            weapon_index: 0,
            damage_fraction: 1.0,
            radius: 0.0,
            edge_effectiveness: 0.0,
            paralyzer: false,
            paralyze_time: 0.0,
            hits_allies: false,
            collide_non_target: true,
            track_target: false,
            requires_source: false,
        };
        let mut damage_events = Vec::new();

        assert_eq!(
            collect_direct_target_hit(
                &units,
                &definitions,
                &impact,
                impact.point,
                &mut damage_events,
            ),
            Some(true)
        );
        assert_eq!(
            damage_events,
            vec![DamageEvent {
                target_index: 1,
                source_side: 0,
                damage: 25.0,
                paralyzer: false,
                paralyze_time: 0.0,
            }]
        );
    }

    #[test]
    fn stockpile_manufacture_charges_once_and_gates_round() {
        let definition = tactical_unit(
            "stockpiler",
            Default::default(),
            vec![WeaponDef {
                damage: 100.0,
                stockpile: true,
                stockpile_time: 1.0,
                energy_cost: 20.0,
                metal_cost: 2.0,
                ..Default::default()
            }],
        );
        let spawns = vec![Spawn {
            side: 0,
            definition: definition.clone(),
            position: Vec2::new(10.0, 10.0),
            heading: 0.0,
        }];
        let definitions = vec![Arc::new(definition)];
        let mut units = spawn_units(&spawns, &definitions);
        let mut resources = Some([
            StartingResources {
                metal: 2.0,
                energy: 20.0,
            },
            StartingResources::default(),
        ]);

        advance_stockpiles(&mut units, &definitions, 0.0, &mut resources);
        assert!(units[0].weapons[0].stockpile_in_progress);
        assert_eq!(units[0].weapons[0].stockpiled_rounds, 0);
        assert_eq!(
            resources.expect("finite resources")[0],
            StartingResources::default()
        );

        advance_stockpiles(&mut units, &definitions, 0.5, &mut resources);
        assert_eq!(units[0].weapons[0].stockpiled_rounds, 0);
        advance_stockpiles(&mut units, &definitions, 1.0, &mut resources);
        assert_eq!(units[0].weapons[0].stockpiled_rounds, 1);
        assert!(!units[0].weapons[0].stockpile_in_progress);
    }

    #[test]
    fn finite_weapon_resources_are_atomic_across_both_resource_types() {
        let mut resources = Some([
            StartingResources {
                metal: 5.0,
                energy: 9.0,
            },
            StartingResources::default(),
        ]);
        assert!(!try_consume_weapon_resources(&mut resources, 0, 10.0, 2.0));
        assert_eq!(resources.expect("finite resources")[0].metal, 5.0);
        assert_eq!(resources.expect("finite resources")[0].energy, 9.0);
        assert!(try_consume_weapon_resources(&mut resources, 0, 9.0, 2.0));
        assert_eq!(resources.expect("finite resources")[0].metal, 3.0);
        assert_eq!(resources.expect("finite resources")[0].energy, 0.0);
    }

    #[test]
    fn finite_resources_gate_weapon_emission_in_a_battle() {
        let definition = tactical_unit(
            "costly-shot",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            vec![WeaponDef {
                damage: 100.0,
                range: 500.0,
                reload: 1.0,
                hitscan: true,
                energy_cost: 10.0,
                ..Default::default()
            }],
        );
        let simulator =
            Simulator::new(Scenario::duel(definition, 100.0)).with_starting_resources([
                StartingResources::default(),
                StartingResources {
                    metal: 0.0,
                    energy: 10.0,
                },
            ]);
        let hold = crate::policy::BuiltinPolicy::Hold;

        let result = simulator.run([&hold, &hold], 7);

        assert_eq!(result.winner, Some(1));
        assert_eq!(result.sides[0].projectiles_fired, 0);
        assert_eq!(result.sides[1].projectiles_fired, 1);
    }

    #[test]
    fn paralyzer_stuns_without_hp_damage_then_recovers() {
        let shooter = tactical_unit(
            "emp",
            Default::default(),
            vec![WeaponDef {
                damage: 125.0,
                paralyzer: true,
                paralyze_time: 10.0,
                ..Default::default()
            }],
        );
        let target = tactical_unit(
            "target",
            crate::model::MovementDef {
                radius: 10.0,
                ..Default::default()
            },
            Vec::new(),
        );
        let spawns = vec![
            Spawn {
                side: 0,
                definition: shooter.clone(),
                position: Vec2::new(10.0, 10.0),
                heading: 0.0,
            },
            Spawn {
                side: 1,
                definition: target.clone(),
                position: Vec2::new(30.0, 10.0),
                heading: 0.0,
            },
        ];
        let definitions = vec![Arc::new(shooter), Arc::new(target)];
        let mut units = spawn_units(&spawns, &definitions);
        let mut impacts = ImpactQueue::new(1_000);
        impacts.schedule(
            0,
            Impact {
                shooter_id: 1,
                shooter_side: 0,
                target_id: 2,
                point: spawns[1].position,
                damage: 125.0,
                weapon_definition_index: 0,
                weapon_index: 0,
                damage_fraction: 1.0,
                radius: 0.0,
                edge_effectiveness: 0.0,
                paralyzer: true,
                paralyze_time: 10.0,
                hits_allies: false,
                collide_non_target: true,
                track_target: false,
                requires_source: false,
            },
        );
        let terrain = Terrain::flat(100.0, 100.0, 10.0);
        let mut spatial = SpatialGrid::new(&terrain, 64.0);
        spatial.rebuild(&units);
        let mut damage_events = Vec::new();
        let mut candidates = Vec::new();
        let mut battle = BattleState {
            scores: initial_scores(&units),
            alive_counts: count_live_sides(&units),
            combat_resources: None,
        };

        resolve_impacts(
            &mut units,
            0,
            &mut impacts,
            &mut damage_events,
            &spatial,
            &mut candidates,
            &mut battle,
            &definitions,
        );

        assert_eq!(units[1].hp, 100.0);
        assert_eq!(units[1].paralyze_damage, 125.0);
        assert!(units[1].stunned);
        for _ in 0..299 {
            advance_paralysis(&mut units);
        }
        assert!(units[1].stunned);
        advance_paralysis(&mut units);
        advance_paralysis(&mut units);
        assert!(!units[1].stunned);
        assert!(units[1].paralyze_damage <= units[1].definition.health);
    }

    #[test]
    fn emp_immunity_multiplier_prevents_stun() {
        let mut target = tactical_unit("immune", Default::default(), Vec::new());
        target.strategic.paralyze_multiplier = 0.0;
        let definitions = vec![Arc::new(target.clone())];
        let spawns = vec![Spawn {
            side: 1,
            definition: target,
            position: Vec2::new(10.0, 10.0),
            heading: 0.0,
        }];
        let mut units = spawn_units(&spawns, &definitions);
        let mut impacts = ImpactQueue::new(1);
        impacts.schedule(
            0,
            Impact {
                shooter_id: 99,
                shooter_side: 0,
                target_id: 1,
                point: spawns[0].position,
                damage: 10_000.0,
                weapon_definition_index: u32::MAX,
                weapon_index: u16::MAX,
                damage_fraction: 1.0,
                radius: 0.0,
                edge_effectiveness: 0.0,
                paralyzer: true,
                paralyze_time: 35.0,
                hits_allies: false,
                collide_non_target: true,
                track_target: false,
                requires_source: false,
            },
        );
        let terrain = Terrain::flat(100.0, 100.0, 10.0);
        let mut spatial = SpatialGrid::new(&terrain, 64.0);
        spatial.rebuild(&units);
        let mut battle = BattleState {
            scores: initial_scores(&units),
            alive_counts: count_live_sides(&units),
            combat_resources: None,
        };
        let mut events = Vec::new();
        let mut candidates = Vec::new();

        resolve_impacts(
            &mut units,
            0,
            &mut impacts,
            &mut events,
            &spatial,
            &mut candidates,
            &mut battle,
            &definitions,
        );

        assert!(!units[0].stunned);
        assert_eq!(units[0].paralyze_damage, 0.0);
        assert_eq!(units[0].hp, 100.0);
    }

    #[test]
    fn stunned_unit_cannot_move_and_weapon_deadlines_are_frozen() {
        let definition = tactical_unit(
            "stunned",
            crate::model::MovementDef {
                max_speed: 60.0,
                acceleration: 60.0,
                braking: 60.0,
                radius: 2.0,
                ..Default::default()
            },
            vec![WeaponDef {
                stockpile: true,
                stockpile_time: 5.0,
                ..Default::default()
            }],
        );
        let spawns = vec![Spawn {
            side: 0,
            definition: definition.clone(),
            position: Vec2::new(50.0, 50.0),
            heading: 0.0,
        }];
        let definitions = vec![Arc::new(definition)];
        let mut units = spawn_units(&spawns, &definitions);
        units[0].speed = 20.0;
        units[0].paralyze_damage = 150.0;
        units[0].weapons[0].reload_until = 2.0;
        units[0].weapons[0].next_shot_at = 3.0;
        units[0].weapons[0].stockpile_ready_at = 4.0;
        units[0].weapons[0].stockpile_in_progress = true;
        let origin = units[0].position;

        advance_paralysis(&mut units);
        update_movement(
            &mut units,
            &[Command {
                destination: Some(Vec2::new(90.0, 50.0)),
                throttle: 1.0,
                ..Default::default()
            }],
            &Terrain::flat(100.0, 100.0, 10.0),
            true,
            &[],
        );

        assert!(units[0].stunned);
        assert_eq!(units[0].position, origin);
        assert_eq!(units[0].speed, 0.0);
        assert!((units[0].weapons[0].reload_until - (2.0 + DT)).abs() < 1e-6);
        assert!((units[0].weapons[0].next_shot_at - (3.0 + DT)).abs() < 1e-6);
        assert!((units[0].weapons[0].stockpile_ready_at - (4.0 + DT)).abs() < 1e-6);
    }

    #[test]
    fn death_explosions_damage_allies_and_chain_in_tick_order() {
        let killer = tactical_unit(
            "killer",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            vec![WeaponDef {
                damage: 100.0,
                range: 500.0,
                reload: 100.0,
                hitscan: true,
                ..Default::default()
            }],
        );
        let mut first = tactical_unit(
            "first-volatile",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            Vec::new(),
        );
        first.strategic.death_explosion = Some(crate::model::DeathExplosionDef {
            damage: 100.0,
            radius: 50.0,
            edge_effectiveness: 1.0,
        });
        let mut second = tactical_unit(
            "second-volatile",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            Vec::new(),
        );
        second.health = 50.0;
        second.strategic.death_explosion = Some(crate::model::DeathExplosionDef {
            damage: 100.0,
            radius: 150.0,
            edge_effectiveness: 1.0,
        });
        let scenario = Scenario {
            terrain: Terrain::flat(1_000.0, 500.0, 32.0),
            units: vec![
                Spawn {
                    side: 0,
                    definition: killer,
                    position: Vec2::new(400.0, 250.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: first,
                    position: Vec2::new(500.0, 250.0),
                    heading: PI,
                },
                Spawn {
                    side: 1,
                    definition: second,
                    position: Vec2::new(540.0, 250.0),
                    heading: PI,
                },
            ],
            max_time: 5.0,
            trace_resources: Vec::new(),
        };

        let result = Simulator::new(scenario).run(
            [
                &crate::policy::BuiltinPolicy::Hold,
                &crate::policy::BuiltinPolicy::Hold,
            ],
            17,
        );

        assert_eq!(result.winner, None);
        assert!(!result.timed_out);
        assert_eq!(result.sides[0].damage_dealt, 100.0);
        assert_eq!(result.sides[1].damage_dealt, 100.0);
        assert_eq!(result.sides[0].damage_received, 100.0);
        assert_eq!(result.sides[1].damage_received, 150.0);
    }

    fn stepper_weapon(damage: f32) -> Arc<crate::model::UnitDef> {
        Arc::new(tactical_unit(
            "stepper-weapon",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            vec![WeaponDef {
                damage,
                range: 500.0,
                reload: 10.0,
                hitscan: true,
                ..Default::default()
            }],
        ))
    }

    fn stepper_target() -> Arc<crate::model::UnitDef> {
        Arc::new(tactical_unit(
            "stepper-target",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            Vec::new(),
        ))
    }

    fn insert_stepper_pair(stepper: &mut CombatStepper) {
        stepper
            .insert(CombatSpawn {
                key: 100,
                side: 0,
                definition: CombatDefinitionId(0),
                position: Vec2::new(100.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper
            .insert(CombatSpawn {
                key: 200,
                side: 1,
                definition: CombatDefinitionId(1),
                position: Vec2::new(200.0, 100.0),
                heading: PI,
                hp: None,
            })
            .unwrap();
        stepper
            .set_command(
                100,
                CombatCommand {
                    target: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    #[test]
    fn combat_stepper_retains_burst_state_across_ticks() {
        let mut weapon = (*stepper_weapon(10.0)).clone();
        weapon.weapons[0].burst = 3;
        weapon.weapons[0].burst_rate = DT;
        let definitions = vec![Arc::new(weapon), stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 3);
        insert_stepper_pair(&mut stepper);

        stepper.step();
        assert_eq!(stepper.combatant(200).unwrap().hp, 90.0);
        stepper.step();
        assert_eq!(stepper.combatant(200).unwrap().hp, 80.0);
        stepper.step();
        assert_eq!(stepper.combatant(200).unwrap().hp, 70.0);
    }

    #[test]
    fn ballistic_arc_clears_a_ridge_that_blocks_a_straight_shot() {
        let origin = Vec2::new(8.0, 8.0);
        let target = Vec2::new(120.0, 8.0);
        let mut terrain = Terrain::flat(128.0, 32.0, 16.0);
        terrain.heights[3] = 10.0;
        terrain.refresh_derived();
        let path = ballistic_flight_path(origin, 5.0, target, 5.0, 100.0, 80.0, false)
            .expect("the cannon can reach the target");

        assert!(!terrain.has_line_of_sight_3d(origin, 5.0, target, 5.0, 0.0));
        assert!(terrain_clear_for_flight_path(
            &terrain, origin, target, path, 0.0,
        ));
        assert!(path.height_at_fraction(0.5) > 10.0);
        assert!(ballistic_flight_path(origin, 5.0, target, 5.0, 30.0, 80.0, false).is_none());
    }

    #[test]
    fn high_ballistic_trajectory_uses_the_second_valid_arc() {
        let origin = Vec2::new(8.0, 8.0);
        let target = Vec2::new(120.0, 8.0);
        let low = ballistic_flight_path(origin, 5.0, target, 5.0, 100.0, 80.0, false)
            .expect("the low cannon arc is reachable");
        let high = ballistic_flight_path(origin, 5.0, target, 5.0, 100.0, 80.0, true)
            .expect("the high cannon arc is reachable");

        assert!(high.flight_time() > low.flight_time());
        assert!(high.height_at_fraction(0.5) > low.height_at_fraction(0.5));
        assert!((high.height_at_fraction(1.0) - 5.0).abs() < 0.01);
    }

    #[test]
    fn ballistic_cannon_arc_clears_a_low_wall_and_lands_on_time() {
        let mut cannon = (*stepper_weapon(25.0)).clone();
        cannon.weapons[0].hitscan = false;
        cannon.weapons[0].projectile_speed = 100.0;
        cannon.weapons[0].projectile_gravity = 80.0;
        let low_wall = Arc::new(tactical_unit(
            "low-wall",
            crate::model::MovementDef {
                radius: 6.0,
                height: 6.0,
                blocks_movement: true,
                blocks_fire: true,
                ..Default::default()
            },
            Vec::new(),
        ));
        let definitions = vec![Arc::new(cannon), stepper_target(), low_wall];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 14);
        insert_stepper_pair(&mut stepper);
        stepper
            .insert(CombatSpawn {
                key: 300,
                side: NEUTRAL_COMBAT_SIDE,
                definition: CombatDefinitionId(2),
                position: Vec2::new(150.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();

        let fired = stepper.step();
        assert!(fired.pending_impacts > 0);
        assert_eq!(stepper.combatant(300).unwrap().hp, 100.0);
        for _ in 0..40 {
            stepper.step();
        }
        assert_eq!(stepper.combatant(300).unwrap().hp, 100.0);
        assert_eq!(stepper.combatant(200).unwrap().hp, 75.0);
    }

    #[test]
    fn tracking_beam_follows_a_moving_target_and_stops_with_its_source() {
        let beam = Arc::new(tactical_unit(
            "tracking-beam",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            vec![WeaponDef {
                damage: 30.0,
                range: 500.0,
                reload: 10.0,
                hitscan: true,
                damage_duration: 0.1,
                ..Default::default()
            }],
        ));

        let mut tracking = CombatStepper::new(
            Terrain::flat(500.0, 300.0, 16.0),
            vec![Arc::clone(&beam), stepper_target()],
            13,
        );
        insert_stepper_pair(&mut tracking);

        tracking.step();
        assert_eq!(tracking.combatant(200).unwrap().hp, 90.0);
        // The two remaining pulses were aimed at the original position. Move
        // the tiny target far enough that an untracked pulse would miss.
        tracking
            .update(
                200,
                CombatantUpdate {
                    position: Some(Vec2::new(200.0, 140.0)),
                    ..Default::default()
                },
            )
            .unwrap();
        tracking.step();
        tracking
            .update(
                200,
                CombatantUpdate {
                    position: Some(Vec2::new(200.0, 180.0)),
                    ..Default::default()
                },
            )
            .unwrap();
        tracking.step();
        assert_eq!(tracking.combatant(200).unwrap().hp, 70.0);

        let mut interrupted = CombatStepper::new(
            Terrain::flat(500.0, 300.0, 16.0),
            vec![beam, stepper_target()],
            13,
        );
        insert_stepper_pair(&mut interrupted);
        interrupted.step();
        assert_eq!(interrupted.combatant(200).unwrap().hp, 90.0);
        interrupted.remove(100).unwrap();
        interrupted.step();
        interrupted.step();
        assert_eq!(interrupted.combatant(200).unwrap().hp, 90.0);
    }

    #[test]
    fn projectile_lands_after_stepper_shooter_is_removed() {
        let mut weapon = (*stepper_weapon(25.0)).clone();
        weapon.weapons[0].hitscan = false;
        weapon.weapons[0].projectile_speed = 100.0;
        let definitions = vec![Arc::new(weapon), stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 7);
        insert_stepper_pair(&mut stepper);

        let fired = stepper.step();
        assert!(fired.pending_impacts > 0);
        stepper.remove(100).unwrap();
        for _ in 0..crate::model::SIM_HZ as usize + 1 {
            stepper.step();
        }

        assert_eq!(stepper.combatant(200).unwrap().hp, 75.0);
    }

    #[test]
    fn combat_stepper_supports_dynamic_insertion_and_wall_blocking() {
        let definitions = vec![stepper_weapon(25.0), stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(500.0, 200.0, 16.0), definitions, 11);
        stepper
            .insert(CombatSpawn {
                key: 100,
                side: 0,
                definition: CombatDefinitionId(0),
                position: Vec2::new(100.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper.step();
        stepper
            .insert(CombatSpawn {
                key: 300,
                side: NEUTRAL_COMBAT_SIDE,
                definition: CombatDefinitionId(1),
                position: Vec2::new(200.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper
            .insert(CombatSpawn {
                key: 200,
                side: 1,
                definition: CombatDefinitionId(1),
                position: Vec2::new(300.0, 100.0),
                heading: PI,
                hp: None,
            })
            .unwrap();
        stepper
            .set_command(
                100,
                CombatCommand {
                    target: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();

        stepper.step();

        // A neutral stationary cylinder intercepts the shot but cannot fire or
        // access either side's score/resource slot.
        assert_eq!(stepper.combatant(200).unwrap().hp, 100.0);
        assert_eq!(stepper.combatant(300).unwrap().hp, 75.0);
        assert_eq!(stepper.combatants().count(), 3);
    }

    #[test]
    fn partial_nanoframe_height_allows_fire_over_while_retaining_footprint() {
        let definitions = vec![stepper_weapon(25.0), stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(500.0, 200.0, 16.0), definitions, 12);
        stepper
            .insert(CombatSpawn {
                key: 100,
                side: 0,
                definition: CombatDefinitionId(0),
                position: Vec2::new(100.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper
            .insert(CombatSpawn {
                key: 300,
                side: NEUTRAL_COMBAT_SIDE,
                definition: CombatDefinitionId(1),
                position: Vec2::new(200.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper
            .update(
                300,
                CombatantUpdate {
                    collision_height_scale: Some(0.1),
                    ..Default::default()
                },
            )
            .unwrap();
        stepper
            .insert(CombatSpawn {
                key: 200,
                side: 1,
                definition: CombatDefinitionId(1),
                position: Vec2::new(300.0, 100.0),
                heading: PI,
                hp: None,
            })
            .unwrap();
        stepper
            .set_command(
                100,
                CombatCommand {
                    target: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();

        stepper.step();

        assert_eq!(stepper.combatant(300).unwrap().hp, 100.0);
        assert_eq!(stepper.combatant(200).unwrap().hp, 75.0);
    }

    #[test]
    fn neutral_static_blocker_prevents_movement() {
        let mobile = Arc::new(tactical_unit(
            "mobile",
            crate::model::MovementDef {
                max_speed: 60.0,
                acceleration: 600.0,
                braking: 600.0,
                radius: 5.0,
                ..Default::default()
            },
            Vec::new(),
        ));
        let definitions = vec![mobile, stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(500.0, 200.0, 16.0), definitions, 1);
        stepper
            .insert(CombatSpawn {
                key: 1,
                side: 0,
                definition: CombatDefinitionId(0),
                position: Vec2::new(180.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper
            .insert(CombatSpawn {
                key: 2,
                side: NEUTRAL_COMBAT_SIDE,
                definition: CombatDefinitionId(1),
                position: Vec2::new(200.0, 100.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        stepper
            .set_command(
                1,
                CombatCommand {
                    destination: Some(Vec2::new(300.0, 100.0)),
                    throttle: 1.0,
                    ..Default::default()
                },
            )
            .unwrap();

        for _ in 0..60 {
            stepper.step();
        }

        assert!(stepper.combatant(1).unwrap().position.x <= 190.001);
        assert_eq!(stepper.combatant(2).unwrap().position.x, 200.0);
    }

    #[test]
    fn combat_stepper_reports_death_by_external_key() {
        let definitions = vec![stepper_weapon(150.0), stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 4);
        insert_stepper_pair(&mut stepper);

        let summary = stepper.step();

        assert_eq!(summary.deaths, 1);
        assert_eq!(stepper.deaths()[0].key, 200);
        assert!(!stepper.combatant(200).unwrap().alive);
    }

    #[test]
    fn combat_stepper_state_is_deterministic() {
        fn configured() -> CombatStepper {
            let definitions = vec![stepper_weapon(7.0), stepper_target()];
            let mut stepper =
                CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 12345);
            insert_stepper_pair(&mut stepper);
            stepper.reserve(4, 100, 100);
            stepper
        }

        let mut left = configured();
        let mut right = configured();
        for _ in 0..60 {
            assert_eq!(left.step(), right.step());
            assert_eq!(
                left.deterministic_state_hash(),
                right.deterministic_state_hash()
            );
        }
        assert_eq!(
            left.combatant(200).unwrap().hp,
            right.combatant(200).unwrap().hp
        );
        assert!(left.combatant(200).unwrap().hp < 100.0);
    }

    #[test]
    fn stepper_compacts_removed_slots_only_without_pending_impacts() {
        let definitions = vec![stepper_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(200.0, 200.0, 16.0), definitions, 2);
        for (key, x) in [(10, 40.0), (20, 80.0), (30, 120.0)] {
            stepper
                .insert(CombatSpawn {
                    key,
                    side: 0,
                    definition: CombatDefinitionId(0),
                    position: Vec2::new(x, 100.0),
                    heading: 0.0,
                    hp: None,
                })
                .unwrap();
        }
        stepper.remove(20).unwrap();

        assert!(stepper.compact_removed());
        assert_eq!(stepper.combatant(30).unwrap().id, 2);
        assert_eq!(
            stepper.combatants().map(|(key, _)| key).collect::<Vec<_>>(),
            [10, 30]
        );
    }

    fn guided_test_projectile(
        heading: f32,
        turn_rate_degrees: f32,
        remaining_ticks: u32,
    ) -> GuidedProjectile {
        GuidedProjectile {
            impact: Impact {
                shooter_id: 1,
                shooter_side: 0,
                target_id: 2,
                point: Vec2::default(),
                damage: 10.0,
                weapon_definition_index: u32::MAX,
                weapon_index: u16::MAX,
                damage_fraction: 1.0,
                radius: 0.0,
                edge_effectiveness: 0.0,
                paralyzer: false,
                paralyze_time: 0.0,
                hits_allies: false,
                collide_non_target: true,
                track_target: false,
                requires_source: false,
            },
            position: Vec2::new(100.0, 50.0),
            height: 5.0,
            heading,
            vertical_speed: 0.0,
            speed: 30.0,
            turn_rate: turn_rate_degrees.to_radians(),
            remaining_ticks,
            damage_pulses: 1,
        }
    }

    #[test]
    fn guided_projectile_turn_is_finite_and_tick_exact() {
        let target = stepper_target();
        let units = vec![
            new_unit_state(1, 0, target.clone(), 0, Vec2::new(20.0, 20.0), 0.0, 100.0),
            new_unit_state(2, 1, target, 0, Vec2::new(100.0, 150.0), PI, 100.0),
        ];
        let terrain = Terrain::flat(300.0, 300.0, 16.0);
        let mut spatial = SpatialGrid::new(&terrain, 64.0);
        spatial.rebuild(&units);
        let mut candidates = Vec::new();
        let mut impacts = ImpactQueue::new(100);
        let mut projectiles = vec![guided_test_projectile(0.0, 90.0, 30)];

        advance_guided_projectiles(
            &units,
            0,
            &mut projectiles,
            &mut impacts,
            &terrain,
            &spatial,
            &mut candidates,
        );

        let projectile = projectiles[0];
        assert!((projectile.heading - 3_f32.to_radians()).abs() < 1e-6);
        assert!((projectile.position.x - 100.99863).abs() < 1e-5);
        assert!((projectile.position.z - 50.05234).abs() < 1e-5);
        assert_eq!(projectile.remaining_ticks, 29);
        assert!(impacts.is_empty());
    }

    #[test]
    fn insufficient_turn_rate_expires_as_a_missed_intercept() {
        let target = stepper_target();
        let units = vec![
            new_unit_state(1, 0, target.clone(), 0, Vec2::new(20.0, 20.0), 0.0, 100.0),
            new_unit_state(2, 1, target, 0, Vec2::new(100.0, 150.0), PI, 100.0),
        ];
        let terrain = Terrain::flat(300.0, 300.0, 16.0);
        let mut spatial = SpatialGrid::new(&terrain, 64.0);
        spatial.rebuild(&units);
        let mut candidates = Vec::new();
        let mut impacts = ImpactQueue::new(100);
        let mut projectiles = vec![guided_test_projectile(0.0, 0.0, 3)];

        for tick in 0..3 {
            advance_guided_projectiles(
                &units,
                tick,
                &mut projectiles,
                &mut impacts,
                &terrain,
                &spatial,
                &mut candidates,
            );
        }

        assert!(projectiles.is_empty());
        assert!(impacts.is_empty());
        assert_eq!(units[1].hp, 100.0);
    }

    #[test]
    fn guided_projectile_state_is_seed_deterministic() {
        fn configured() -> CombatStepper {
            let mut shooter = (*stepper_weapon(10.0)).clone();
            shooter.weapons[0].hitscan = false;
            shooter.weapons[0].tracks = true;
            shooter.weapons[0].projectile_speed = 60.0;
            shooter.weapons[0].projectile_turn_rate = 75.0;
            shooter.weapons[0].projectile_lifetime = 4.0;
            let mut target = (*stepper_target()).clone();
            target.movement.max_speed = 30.0;
            target.movement.acceleration = 120.0;
            target.movement.braking = 120.0;
            let definitions = vec![Arc::new(shooter), Arc::new(target)];
            let mut stepper =
                CombatStepper::new(Terrain::flat(400.0, 300.0, 16.0), definitions, 99);
            insert_stepper_pair(&mut stepper);
            stepper
                .set_command(
                    200,
                    CombatCommand {
                        destination: Some(Vec2::new(200.0, 250.0)),
                        throttle: 1.0,
                        ..Default::default()
                    },
                )
                .unwrap();
            stepper
        }

        let mut left = configured();
        let mut right = configured();
        for _ in 0..90 {
            assert_eq!(left.step(), right.step());
            assert_eq!(
                left.deterministic_state_hash(),
                right.deterministic_state_hash()
            );
        }
        assert_eq!(
            left.combatant(200).unwrap().hp,
            right.combatant(200).unwrap().hp
        );
    }

    fn shielded_target() -> Arc<crate::model::UnitDef> {
        Arc::new(tactical_unit(
            "shielded-target",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            vec![WeaponDef {
                name: "shield".into(),
                is_shield: true,
                shield_radius: 80.0,
                shield_power: 100.0,
                shield_starting_power: 100.0,
                shield_power_regen: 30.0,
                shield_power_regen_energy: 30.0,
                shield_recharge_delay: 1.0,
                shield_intercept_type: 1,
                ..Default::default()
            }],
        ))
    }

    #[test]
    fn shield_absorbs_depletes_and_energy_gates_recharge() {
        let mut shooter = (*stepper_weapon(50.0)).clone();
        shooter.weapons[0].projectiles = 3;
        shooter.weapons[0].intercepted_by_shield_type = 1;
        let definitions = vec![Arc::new(shooter), shielded_target()];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 17);
        insert_stepper_pair(&mut stepper);
        stepper.set_resources(Some([
            StartingResources::default(),
            StartingResources::default(),
        ]));

        stepper.step();

        // Two rounds exactly exhaust the shield; the third reaches the unit.
        assert_eq!(stepper.shield_charge(200, 0), Some(0.0));
        assert_eq!(stepper.combatant(200).unwrap().hp, 50.0);
        for _ in 0..31 {
            stepper.step();
        }
        assert_eq!(stepper.shield_charge(200, 0), Some(0.0));

        stepper.set_resources(Some([
            StartingResources::default(),
            StartingResources {
                metal: 0.0,
                energy: 1.0,
            },
        ]));
        stepper.step();
        assert!((stepper.shield_charge(200, 0).unwrap() - 1.0).abs() < 1e-5);
        assert_eq!(stepper.resources().unwrap()[1].energy, 0.0);
    }

    fn interceptor_target(energy_cost: f32) -> Arc<crate::model::UnitDef> {
        Arc::new(tactical_unit(
            "interceptor-target",
            crate::model::MovementDef {
                radius: 5.0,
                height: 10.0,
                ..Default::default()
            },
            vec![WeaponDef {
                name: "interceptor".into(),
                interceptor: 1,
                coverage_range: 150.0,
                reload: 10.0,
                energy_cost,
                ..Default::default()
            }],
        ))
    }

    fn targetable_shooter(guided: bool) -> Arc<crate::model::UnitDef> {
        let mut shooter = (*stepper_weapon(50.0)).clone();
        shooter.weapons[0].hitscan = false;
        shooter.weapons[0].projectile_speed = 100.0;
        shooter.weapons[0].targetable = 1;
        shooter.weapons[0].tracks = guided;
        if guided {
            shooter.weapons[0].projectile_turn_rate = 90.0;
            shooter.weapons[0].projectile_lifetime = 3.0;
        }
        Arc::new(shooter)
    }

    #[test]
    fn interceptor_covers_ordinary_and_guided_projectile_paths() {
        for guided in [false, true] {
            let definitions = vec![targetable_shooter(guided), interceptor_target(0.0)];
            let mut stepper =
                CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 21);
            insert_stepper_pair(&mut stepper);

            let summary = stepper.step();

            assert_eq!(summary.pending_impacts, 0, "guided={guided}");
            assert_eq!(stepper.combatant(200).unwrap().hp, 100.0);
            assert!(stepper.combatant(200).unwrap().weapons[0].reload_until > 0.0);
        }
    }

    #[test]
    fn interceptor_resource_gate_allows_unopposed_projectile() {
        let definitions = vec![targetable_shooter(false), interceptor_target(5.0)];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 22);
        insert_stepper_pair(&mut stepper);
        stepper.set_resources(Some([
            StartingResources::default(),
            StartingResources {
                metal: 0.0,
                energy: 4.0,
            },
        ]));

        let summary = stepper.step();

        assert_eq!(summary.pending_impacts, 1);
        assert_eq!(stepper.resources().unwrap()[1].energy, 4.0);
        assert_eq!(stepper.combatant(200).unwrap().weapons[0].reload_until, 0.0);
    }

    #[test]
    fn interceptor_stockpile_manufacture_and_launch_are_resource_gated() {
        let mut interceptor = (*interceptor_target(5.0)).clone();
        interceptor.weapons[0].stockpile = true;
        interceptor.weapons[0].stockpile_time = DT;
        let definitions = vec![targetable_shooter(false), Arc::new(interceptor)];
        let mut stepper = CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 24);
        insert_stepper_pair(&mut stepper);
        stepper.clear_command(100).unwrap();
        stepper.set_resources(Some([
            StartingResources::default(),
            StartingResources {
                metal: 0.0,
                energy: 5.0,
            },
        ]));

        stepper.step();
        assert_eq!(stepper.resources().unwrap()[1].energy, 0.0);
        assert_eq!(stepper.stockpiled_rounds(200, 0), Some(0));
        stepper.step();
        assert_eq!(stepper.stockpiled_rounds(200, 0), Some(1));
        stepper
            .set_command(
                100,
                CombatCommand {
                    target: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
        let summary = stepper.step();
        assert_eq!(summary.pending_impacts, 0);
        assert_eq!(stepper.stockpiled_rounds(200, 0), Some(0));
    }

    #[test]
    fn overlapping_interceptors_use_stable_unit_then_weapon_order() {
        fn configured() -> CombatStepper {
            let definitions = vec![
                targetable_shooter(false),
                interceptor_target(0.0),
                stepper_target(),
            ];
            let mut stepper =
                CombatStepper::new(Terrain::flat(400.0, 200.0, 16.0), definitions, 23);
            for (key, side, definition, position) in [
                (100, 0, 0, Vec2::new(100.0, 100.0)),
                (201, 1, 1, Vec2::new(180.0, 50.0)),
                (202, 1, 1, Vec2::new(180.0, 150.0)),
                (200, 1, 2, Vec2::new(200.0, 100.0)),
            ] {
                stepper
                    .insert(CombatSpawn {
                        key,
                        side,
                        definition: CombatDefinitionId(definition),
                        position,
                        heading: 0.0,
                        hp: None,
                    })
                    .unwrap();
            }
            stepper
                .set_command(
                    100,
                    CombatCommand {
                        target: Some(200),
                        ..Default::default()
                    },
                )
                .unwrap();
            stepper
        }

        let mut left = configured();
        let mut right = configured();
        assert_eq!(left.step(), right.step());
        assert_eq!(
            left.deterministic_state_hash(),
            right.deterministic_state_hash()
        );
        assert!(left.combatant(201).unwrap().weapons[0].reload_until > 0.0);
        assert_eq!(left.combatant(202).unwrap().weapons[0].reload_until, 0.0);
    }

    #[test]
    fn checked_simulator_accepts_homing_shields_and_interceptors() {
        let mut definition = (*stepper_weapon(5.0)).clone();
        definition.name = "guided".into();
        definition.weapons[0].name = "missile".into();
        definition.weapons[0].hitscan = false;
        definition.weapons[0].tracks = true;
        definition.weapons[0].projectile_speed = 50.0;
        definition.weapons[0].projectile_turn_rate = 90.0;

        Simulator::try_new(Scenario::duel(definition.clone(), 100.0))
            .expect("finite-turn-rate homing is implemented");

        definition.weapons[0].required_capabilities = vec![
            crate::catalog::CAPABILITY_SHIELDS.into(),
            crate::catalog::CAPABILITY_INTERCEPTORS.into(),
        ];
        Simulator::try_new(Scenario::duel(definition.clone(), 100.0))
            .expect("simplified shields and interceptors are implemented");

        definition.weapons[0].required_capabilities = vec!["projectile_portals".into()];
        let error = Simulator::try_new(Scenario::duel(definition, 100.0))
            .expect_err("unknown defensive variants remain explicit");
        assert_eq!(error[0].capability, "projectile_portals");
    }
}
