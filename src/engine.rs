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
    /// Terrain-relative elevation of the intended impact.
    altitude: f32,
    terrain_height: f32,
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
    source_id: u32,
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
    sensing_enabled: bool,
    /// `None` keeps historical tactical runs resource-unlimited. Experiments
    /// can opt into finite per-side ammunition resources without coupling this
    /// hot loop to the strategic economy scheduler.
    combat_resources: Option<[StartingResources; 2]>,
    auxiliary_units: BTreeSet<u32>,
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

/// A current contact disclosed to one observing side. A missing velocity is
/// unknown, not permission to recover it from the physical combatant.
#[derive(Clone, Copy, Debug)]
pub struct TargetObservation {
    pub target: CombatKey,
    pub position: Vec2,
    pub height: f32,
    pub velocity: Option<Vec2>,
    pub exact: bool,
}

#[derive(Clone, Copy, Debug)]
struct TargetEstimate {
    position: Vec2,
    height: f32,
    velocity: Vec2,
    exact: bool,
}

#[derive(Default)]
struct TargetingState {
    // None is the explicit legacy oracle mode. Some(empty) denies all entity acquisition.
    contacts: [Option<Vec<Option<TargetEstimate>>>; 2],
    ground: HashMap<u32, Vec2>,
}
impl TargetingState {
    fn estimate(
        &self,
        side: u8,
        target: u32,
        units: &[UnitState],
        terrain: &Terrain,
    ) -> Option<TargetEstimate> {
        if let Some(contacts) = self.contacts.get(side as usize).and_then(Option::as_ref) {
            return contacts
                .get(target.checked_sub(1)? as usize)
                .copied()
                .flatten();
        }
        let index = live_unit_index(units, target)?;
        Some(TargetEstimate {
            position: units[index].position,
            height: unit_collision_mid_y(&units[index], terrain),
            velocity: units[index].velocity(),
            exact: true,
        })
    }
    fn clear(&mut self) {
        self.contacts = [None, None];
        self.ground.clear();
    }
    fn remap(&mut self, remap: &[u32]) {
        for contacts in self.contacts.iter_mut().flatten() {
            let old = std::mem::take(contacts);
            contacts.resize(remap.iter().copied().max().unwrap_or(0) as usize, None);
            for (index, value) in old.into_iter().enumerate() {
                if let Some(id) = remap.get(index).copied().filter(|id| *id > 0) {
                    contacts[id as usize - 1] = value;
                }
            }
        }
        self.ground = self
            .ground
            .drain()
            .filter_map(|(id, point)| {
                remap
                    .get(id as usize - 1)
                    .copied()
                    .filter(|id| *id > 0)
                    .map(|id| (id, point))
            })
            .collect();
    }
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
    InvalidObservation,
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
    targeting: TargetingState,
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

/// Cached, side-limited policy snapshots for stand-alone tactical trials.
/// Legacy fixtures without sensor declarations keep their oracle fast path;
/// once any sensor is declared, undeclared sight falls back to map-wide range.
struct StandaloneSensors {
    targeting: TargetingState,
    views: [Vec<UnitState>; 2],
    units: Vec<crate::sensors::SensorUnit>,
    cloaked: Vec<bool>,
    decloak_until: Vec<u64>,
    next_cloak_check: Vec<u64>,
    previous_shots: Vec<u64>,
    identified: [Vec<bool>; 2],
    tracked: [Vec<bool>; 2],
    known_definitions: [Vec<Option<Arc<crate::model::UnitDef>>>; 2],
    ghosts: [Vec<Option<UnitState>>; 2],
    ghost_targets: [Vec<Option<Vec2>>; 2],
    unknown: Arc<crate::model::UnitDef>,
}
impl StandaloneSensors {
    fn new(len: usize) -> Self {
        let mut unknown = crate::model::UnitDef::default();
        unknown.name = "unknown-contact".into();
        unknown.health = 1.0;
        Self {
            targeting: TargetingState::default(),
            views: [Vec::with_capacity(len), Vec::with_capacity(len)],
            units: Vec::with_capacity(len),
            cloaked: vec![false; len],
            decloak_until: vec![0; len],
            next_cloak_check: vec![0; len],
            previous_shots: vec![0; len],
            identified: [vec![false; len], vec![false; len]],
            tracked: [vec![false; len], vec![false; len]],
            known_definitions: [vec![None; len], vec![None; len]],
            ghosts: [vec![None; len], vec![None; len]],
            ghost_targets: [vec![None; len], vec![None; len]],
            unknown: Arc::new(unknown),
        }
    }
    fn reset(&mut self) {
        self.targeting.clear();
        self.cloaked.fill(false);
        self.decloak_until.fill(0);
        self.next_cloak_check.fill(0);
        self.previous_shots.fill(0);
        for identified in &mut self.identified {
            identified.fill(false);
        }
        for tracked in &mut self.tracked {
            tracked.fill(false);
        }
        for known in &mut self.known_definitions {
            known.fill(None);
        }
        for ghosts in &mut self.ghosts {
            ghosts.fill(None);
        }
        for ghosts in &mut self.ghost_targets {
            ghosts.fill(None);
        }
    }

    fn sanitize_command(&mut self, side: u8, shooter: u32, mut command: Command) -> Command {
        self.targeting.ground.remove(&shooter);
        if let Some(target) = command.target {
            let known = self
                .targeting
                .contacts
                .get(side as usize)
                .and_then(Option::as_ref)
                .and_then(|contacts| {
                    target
                        .checked_sub(1)
                        .and_then(|id| contacts.get(id as usize))
                })
                .copied()
                .flatten();
            if known.is_none() {
                if let Some(point) = self
                    .ghost_targets
                    .get(side as usize)
                    .and_then(|ghosts| target.checked_sub(1).and_then(|id| ghosts.get(id as usize)))
                    .copied()
                    .flatten()
                {
                    self.targeting.ground.insert(shooter, point);
                }
                command.target = None;
            }
        }
        command
    }

    fn refresh(
        &mut self,
        truth: &[UnitState],
        terrain: &Terrain,
        seed: u64,
        tick: u64,
        resources: &mut Option<[StartingResources; 2]>,
    ) {
        self.units.clear();
        for (index, unit) in truth.iter().enumerate() {
            let d = &unit.definition.strategic;
            let ground = terrain.height_at(unit.position);
            let mut sensor = crate::sensors::SensorUnit::from_definition(
                u64::from(unit.id),
                unit.side,
                unit.position,
                ground - terrain.water_level,
                &unit.definition,
            );
            sensor.ground_height += terrain.water_level;
            sensor.target_height += terrain.water_level + unit.altitude;
            sensor.use_air_los = unit.definition.flight.is_some() || d.use_air_los;
            sensor.active = unit.alive;
            sensor.enabled = !unit.stunned;
            sensor.being_built = unit.collision_height_scale < 1.0;
            if d.sight_distance.is_none() {
                sensor.sight = (terrain.cols as f32 + terrain.rows as f32) * terrain.cell_size;
                sensor.air_sight = d.air_sight_distance.unwrap_or(sensor.sight * 1.5);
            }
            let shots = unit
                .weapons
                .iter()
                .map(|mount| mount.shot_sequence)
                .sum::<u64>();
            if d.decloak_on_fire && shots != self.previous_shots[index] {
                self.decloak_until[index] = tick.saturating_add(128);
                self.cloaked[index] = false;
            }
            self.previous_shots[index] = shots;
            let nearby = d.min_cloak_distance > 0.0
                && truth.iter().any(|enemy| {
                    enemy.alive && enemy.side < 2 && enemy.side != unit.side && {
                        let horizontal = unit.position.distance_squared(enemy.position);
                        let vertical = if d.decloak_spherical {
                            let dy = unit_collision_mid_y(unit, terrain)
                                - unit_collision_mid_y(enemy, terrain);
                            dy * dy
                        } else {
                            0.0
                        };
                        let radius = d.min_cloak_distance
                            + if d.decloak_spherical {
                                enemy.definition.movement.radius.max(0.0)
                            } else {
                                0.0
                            };
                        horizontal + vertical <= radius * radius
                    }
                });
            if tick >= self.next_cloak_check[index] {
                let mut cloaked = d.can_cloak
                    && d.start_cloaked
                    && sensor.active
                    && !unit.stunned
                    && tick >= self.decloak_until[index]
                    && !nearby;
                if cloaked && unit.side < 2 {
                    let cost = if unit.speed.abs() > 0.2_f32.sqrt() * crate::model::SIM_HZ {
                        d.cloak_cost_moving.unwrap_or(d.cloak_cost)
                    } else {
                        d.cloak_cost
                    };
                    if let Some(pools) = resources {
                        let energy = &mut pools[unit.side as usize].energy;
                        let expense = f64::from(cost.max(0.0)) * 0.5;
                        if *energy + 1e-12 >= expense {
                            *energy = (*energy - expense).max(0.0);
                        } else {
                            cloaked = false;
                        }
                    }
                }
                self.cloaked[index] = cloaked;
                self.next_cloak_check[index] = tick.saturating_add(15);
            }
            if !unit.alive {
                self.cloaked[index] = false;
            }
            sensor.cloaked = self.cloaked[index];
            self.units.push(sensor);
        }
        for side in 0..2 {
            let error = crate::sensors::radar_error_size(side as u8, &self.units, 96.0);
            let contacts = self.targeting.contacts[side].get_or_insert_with(Vec::new);
            contacts.resize(truth.len(), None);
            contacts.fill(None);
            self.ghost_targets[side].fill(None);
            let mut count = 0;
            for (index, unit) in truth.iter().enumerate() {
                let detected = if !unit.alive {
                    None
                } else if unit.side == side as u8 {
                    Some(crate::sensors::Detection::Los)
                } else {
                    crate::sensors::detect(
                        side as u8,
                        &self.units[index],
                        &self.units,
                        |from, from_y, to, to_y| {
                            terrain.has_line_of_sight_3d(from, from_y, to, to_y, 0.0)
                        },
                    )
                };
                let Some(detected) = detected else {
                    let was_tracked = self.tracked[side][index];
                    self.tracked[side][index] = false;
                    if !unit.alive && was_tracked {
                        self.ghosts[side][index] = None;
                    }
                    self.identified[side][index] = false;
                    self.known_definitions[side][index] = None;
                    if let Some(ghost) = self.ghosts[side][index].as_ref() {
                        let ghost_ground = terrain.height_at(ghost.position);
                        let mut location = crate::sensors::SensorUnit::from_definition(
                            u64::from(ghost.id),
                            ghost.side,
                            ghost.position,
                            ghost_ground - terrain.water_level,
                            &ghost.definition,
                        );
                        location.ground_height += terrain.water_level;
                        location.target_height += terrain.water_level + ghost.altitude;
                        let empty_seen = !unit.alive
                            && self.units.iter().any(|observer| {
                                observer.side == side as u8
                                    && crate::sensors::visual_coverage(
                                        observer,
                                        &location,
                                        &|from, from_y, to, to_y| {
                                            terrain
                                                .has_line_of_sight_3d(from, from_y, to, to_y, 0.0)
                                        },
                                    )
                            });
                        if empty_seen {
                            self.ghosts[side][index] = None;
                        } else {
                            self.ghost_targets[side][index] = Some(ghost.position);
                            let views = &mut self.views[side];
                            if count < views.len() {
                                views[count].clone_from(ghost);
                            } else {
                                views.push(ghost.clone());
                            }
                            count += 1;
                        }
                    }
                    continue;
                };
                self.tracked[side][index] = true;
                let exact = detected == crate::sensors::Detection::Los;
                if exact {
                    self.identified[side][index] = true;
                    self.known_definitions[side][index] = Some(Arc::clone(&unit.definition));
                }
                let remembered_static = self.ghosts[side][index]
                    .as_ref()
                    .map(|ghost| ghost.position);
                let position = if exact {
                    unit.position
                } else if self.units[index].leaves_ghost && remembered_static.is_some() {
                    remembered_static.unwrap()
                } else {
                    crate::sensors::radar_position(
                        u64::from(unit.id) ^ seed.rotate_left(17),
                        tick,
                        unit.position,
                        error,
                    )
                };
                if unit.side != side as u8 {
                    contacts[index] = Some(TargetEstimate {
                        position,
                        height: if exact {
                            self.units[index].target_height
                        } else {
                            terrain.height_at(position) + PROJECTILE_CLEARANCE + 0.01
                        },
                        velocity: if exact {
                            unit.velocity()
                        } else {
                            Vec2::default()
                        },
                        exact,
                    });
                }
                let views = &mut self.views[side];
                if count < views.len() {
                    views[count].clone_from(unit);
                } else {
                    views.push(unit.clone());
                }
                let view = &mut views[count];
                view.position = position;
                if unit.side != side as u8 {
                    // Known definitions disclose capabilities, not live enemy reload,
                    // aim, stockpile, prediction RNG or regeneration deadlines.
                    view.weapons.clear();
                    if !exact {
                        view.definition = self.known_definitions[side][index]
                            .as_ref()
                            .cloned()
                            .or_else(|| {
                                self.ghosts[side][index]
                                    .as_ref()
                                    .map(|ghost| Arc::clone(&ghost.definition))
                            })
                            .unwrap_or_else(|| Arc::clone(&self.unknown));
                        view.definition_index = usize::MAX;
                        view.hp = 0.0;
                        view.speed = 0.0;
                        view.heading = 0.0;
                        view.forward = Vec2::new(1.0, 0.0);
                        view.weapons.clear();
                        view.paralyze_damage = 0.0;
                        view.stunned = false;
                        view.collision_height_scale = 1.0;
                        view.flanking_direction = Vec2::default();
                        view.flanking_mobility = 0.0;
                    } else if self.units[index].leaves_ghost
                        && unit.definition.movement.max_speed <= 0.0
                    {
                        let mut ghost = view.clone();
                        ghost.hp = 0.0;
                        ghost.speed = 0.0;
                        ghost.alive = true;
                        ghost.weapons.clear();
                        ghost.paralyze_damage = 0.0;
                        ghost.stunned = false;
                        self.ghosts[side][index] = Some(ghost);
                    }
                }
                count += 1;
            }
            self.views[side].truncate(count);
        }
    }
}

fn has_sensor_declaration(unit: &crate::model::UnitDef) -> bool {
    let d = &unit.strategic;
    d.sight_distance.is_some()
        || d.air_sight_distance.is_some()
        || d.radar_radius.is_some()
        || d.sonar_radius.is_some()
        || d.jammer_radius.is_some()
        || d.sonar_jammer_radius.is_some()
        || d.stealth
        || d.sonar_stealth
        || d.can_cloak
        || d.use_air_los
        || d.targeting_facility
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
    sensors: StandaloneSensors,
}

impl TrialWorkspace {
    fn new(simulator: &Simulator) -> Self {
        let units = spawn_units(&simulator.scenario.units, &simulator.definitions);
        let maximum_tick = (simulator.scenario.max_time / DT).ceil() as u64;
        Self {
            commands: vec![Command::default(); units.len()],
            sensors: StandaloneSensors::new(units.len()),
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
        self.sensors.reset();
    }
}

struct BattleState {
    armed_units: [BTreeSet<u32>; 2],
    fired_units: [BTreeSet<u32>; 2],
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
        let sensing_enabled = scenario
            .units
            .iter()
            .any(|spawn| has_sensor_declaration(&spawn.definition));
        Self {
            scenario,
            definitions,
            terrain_all_passable,
            paralysis_enabled,
            sensing_enabled,
            combat_resources: None,
            auxiliary_units: BTreeSet::new(),
        }
    }

    /// Enables finite tactical weapon resources. Costs are charged per emitted
    /// salvo, or when manufacture begins for an automatically queued stockpile
    /// round. The default remains unlimited for backwards-compatible duels.
    pub fn with_starting_resources(mut self, resources: [StartingResources; 2]) -> Self {
        self.combat_resources = Some(resources);
        self
    }

    /// Reconnaissance assets remain destructible and fully resource-scored,
    /// but cannot prolong a platoon fight after its main force is eliminated.
    /// Unarmed aircraft use a fixed, sensor-limited area sweep; static support
    /// stays at its site. The empty default preserves ordinary battle semantics.
    pub fn with_auxiliary_units(mut self, ids: impl IntoIterator<Item = u32>) -> Self {
        self.auxiliary_units = ids.into_iter().collect();
        assert!(
            self.auxiliary_units
                .iter()
                .all(|id| *id > 0 && (*id as usize) <= self.scenario.units.len())
        );
        self
    }

    fn victory_alive(&self, units: &[UnitState], battle: &BattleState) -> [bool; 2] {
        if self.auxiliary_units.is_empty() {
            return [battle.alive_counts[0] > 0, battle.alive_counts[1] > 0];
        }
        std::array::from_fn(|side| {
            units.iter().any(|u| {
                u.alive && u.side as usize == side && !self.auxiliary_units.contains(&u.id)
            })
        })
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
            sensors,
        } = workspace;
        let mut battle = BattleState {
            fired_units: Default::default(),
            armed_units: armed_unit_ids(units),
            scores: initial_scores(units),
            alive_counts: count_live_sides(units),
            combat_resources: self.combat_resources,
        };
        let mut time = 0.0;
        let mut tick = 0_u64;
        let mut alive = self.victory_alive(units, &battle);

        // Let already-fired projectiles and damage-over-time pulses land after
        // one army dies; otherwise long flight time would create false wins.
        while time < self.scenario.max_time
            && ((alive[0] && alive[1]) || !impacts.is_empty() || !guided_projectiles.is_empty())
        {
            advance_flanking(units, tick);
            if self.paralysis_enabled {
                advance_paralysis(units);
            }
            if self.sensing_enabled {
                sensors.refresh(
                    units,
                    &self.scenario.terrain,
                    seed,
                    tick,
                    &mut battle.combat_resources,
                );
            }
            for (command, unit) in commands.iter_mut().zip(units.iter()) {
                *command = if unit.alive && unit.side < 2 {
                    let disclosed = if self.sensing_enabled {
                        sensors.views[unit.side as usize].as_slice()
                    } else {
                        units.as_slice()
                    };
                    let view = WorldView {
                        time,
                        terrain: &self.scenario.terrain,
                        units: disclosed,
                    };
                    let result = if self.auxiliary_units.contains(&unit.id) {
                        if unit.definition.flight.is_some() {
                            crate::policy::reconnaissance(&view, unit)
                        } else {
                            Command::default()
                        }
                    } else {
                        policies[unit.side as usize].command(&view, unit)
                    };
                    if self.sensing_enabled {
                        sensors.sanitize_command(unit.side, unit.id, result)
                    } else {
                        result
                    }
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
            // Drain already-fired rounds after main-force elimination without
            // generating an endless stream at surviving auxiliary targets.
            if self.auxiliary_units.is_empty() || (alive[0] && alive[1]) {
                update_weapons::<TRACE>(
                    units,
                    commands,
                    self.sensing_enabled.then_some(&sensors.targeting),
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
            }
            advance_guided_projectiles_observed(
                units,
                tick,
                guided_projectiles,
                impacts,
                &self.scenario.terrain,
                spatial,
                spatial_candidates,
                self.sensing_enabled.then_some(&sensors.targeting),
            );
            resolve_impacts_observed(
                units,
                tick,
                impacts,
                damage_events,
                spatial,
                spatial_candidates,
                &mut battle,
                &self.definitions,
                self.sensing_enabled.then_some(&sensors.targeting),
                Some(&self.scenario.terrain),
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
            alive = self.victory_alive(units, &battle);
            time += DT;
            tick += 1;
        }

        finish_scores(units, &mut battle.scores, time);
        for (side, score) in battle.scores.iter_mut().enumerate() {
            score.won = alive[side] && !alive[1 - side];
        }
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
            targeting: TargetingState::default(),
            impacts: ImpactQueue::new(u64::MAX),
            guided_projectiles: Vec::new(),
            damage_events: Vec::new(),
            spatial_candidates: Vec::new(),
            spatial,
            aim_attempts: Vec::new(),
            deaths: Vec::new(),
            was_alive: Vec::new(),
            battle: BattleState {
                fired_units: Default::default(),
                armed_units: Default::default(),
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
            if definition.has_automatic_weapon() {
                self.battle.armed_units[spawn.side as usize].insert(self.units[index].id);
                self.battle.scores[spawn.side as usize].firing.armed_units += 1;
            }
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
        self.targeting.remap(&remap);
        for tracked in [&mut self.battle.fired_units, &mut self.battle.armed_units] {
            for ids in tracked.iter_mut() {
                *ids = ids
                    .iter()
                    .filter_map(|id| remap.get(id.saturating_sub(1) as usize).copied())
                    .filter(|id| *id != 0)
                    .collect();
            }
        }
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
            self.units[index].flanking_direction = definition.strategic.flanking.direction;
            self.units[index].flanking_mobility =
                definition.strategic.flanking.mobility_add * 1000.0;
            self.units[index].altitude = definition
                .flight
                .as_ref()
                .map_or(0.0, |flight| flight.cruise_altitude);
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
            if self.units[index].definition.has_automatic_weapon()
                && self.battle.armed_units[new_side as usize].insert(self.units[index].id)
            {
                self.battle.scores[new_side as usize].firing.armed_units += 1;
            }
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
        self.targeting.ground.remove(&self.units[index].id);
        self.commands[index] = Command {
            target,
            destination: command.destination,
            throttle: command.throttle,
        };
        Ok(())
    }

    /// Replaces the complete current contact set for a side. Once called, a
    /// missing contact cannot acquire or guide against the hidden true position.
    /// Call every tactical tick, including with an empty slice after contact loss.
    pub fn set_target_observations(
        &mut self,
        side: u8,
        observations: &[TargetObservation],
    ) -> Result<(), CombatError> {
        if side >= 2 {
            return Err(CombatError::InvalidSide(side));
        }
        let mut contacts = vec![None; self.units.len()];
        for observation in observations {
            let index = *self
                .indices
                .get(&observation.target)
                .ok_or(CombatError::UnknownTarget(observation.target))?;
            if !observation.position.x.is_finite()
                || !observation.position.z.is_finite()
                || !observation.height.is_finite()
                || observation
                    .velocity
                    .is_some_and(|v| !v.x.is_finite() || !v.z.is_finite())
                || contacts[index].is_some()
            {
                return Err(CombatError::InvalidObservation);
            }
            contacts[index] = Some(TargetEstimate {
                position: observation.position,
                height: observation.height,
                velocity: observation.velocity.unwrap_or_default(),
                exact: observation.exact,
            });
        }
        self.targeting.contacts[side as usize] = Some(contacts);
        Ok(())
    }

    /// Ground fire is explicit and never acquires a live entity at that point.
    /// Entity commands clear this override; None clears a prior ground order.
    pub fn set_ground_target(
        &mut self,
        key: CombatKey,
        position: Option<Vec2>,
    ) -> Result<(), CombatError> {
        let index = *self.indices.get(&key).ok_or(CombatError::UnknownKey(key))?;
        let id = self.units[index].id;
        if let Some(point) = position {
            if !point.x.is_finite() || !point.z.is_finite() {
                return Err(CombatError::InvalidObservation);
            }
            self.commands[index].target = None;
            self.targeting.ground.insert(id, point);
        } else {
            self.targeting.ground.remove(&id);
        }
        Ok(())
    }

    pub fn clear_command(&mut self, key: CombatKey) -> Result<(), CombatError> {
        let index = *self.indices.get(&key).ok_or(CombatError::UnknownKey(key))?;
        self.commands[index] = Command::default();
        self.targeting.ground.remove(&self.units[index].id);
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
            .find_map(|(weapon, state)| {
                (weapon.turret
                    && !weapon.is_shield
                    && !weapon.manual_fire
                    && weapon.interceptor == 0)
                    .then_some(state.aim_heading)
            })
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
        advance_flanking(&mut self.units, self.tick);
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
            Some(&self.targeting),
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
        advance_guided_projectiles_observed(
            &self.units,
            self.tick,
            &mut self.guided_projectiles,
            &mut self.impacts,
            &self.terrain,
            &self.spatial,
            &mut self.spatial_candidates,
            Some(&self.targeting),
        );
        self.was_alive.resize(self.units.len(), false);
        for (was_alive, unit) in self.was_alive.iter_mut().zip(&self.units) {
            *was_alive = unit.alive;
        }
        resolve_impacts_observed(
            &mut self.units,
            self.tick,
            &mut self.impacts,
            &mut self.damage_events,
            &self.spatial,
            &mut self.spatial_candidates,
            &mut self.battle,
            &self.definitions,
            Some(&self.targeting),
            Some(&self.terrain),
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
            hash_value(&mut hash, u64::from(unit.altitude.to_bits()));
            hash_value(&mut hash, u64::from(unit.hp.to_bits()));
            hash_value(&mut hash, u64::from(unit.collision_height_scale.to_bits()));
            hash_value(&mut hash, u64::from(unit.paralyze_damage.to_bits()));
            hash_value(&mut hash, u64::from(unit.flanking_direction.x.to_bits()));
            hash_value(&mut hash, u64::from(unit.flanking_direction.z.to_bits()));
            hash_value(&mut hash, u64::from(unit.flanking_mobility.to_bits()));
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
        for contacts in &self.targeting.contacts {
            hash_value(&mut hash, contacts.is_some() as u64);
            if let Some(contacts) = contacts {
                for (index, estimate) in contacts.iter().enumerate() {
                    if let Some(estimate) = estimate {
                        hash_value(&mut hash, index as u64 + 1);
                        for value in [
                            estimate.position.x,
                            estimate.position.z,
                            estimate.height,
                            estimate.velocity.x,
                            estimate.velocity.z,
                        ] {
                            hash_value(&mut hash, u64::from(value.to_bits()));
                        }
                        hash_value(&mut hash, estimate.exact as u64);
                    }
                }
            }
        }
        for unit in &self.units {
            if let Some(point) = self.targeting.ground.get(&unit.id) {
                hash_value(&mut hash, u64::from(unit.id));
                hash_value(&mut hash, u64::from(point.x.to_bits()));
                hash_value(&mut hash, u64::from(point.z.to_bits()));
            }
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
                hash_value(&mut hash, u64::from(impact.altitude.to_bits()));
                hash_value(&mut hash, u64::from(impact.terrain_height.to_bits()));
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
            hash_value(&mut hash, u64::from(projectile.impact.altitude.to_bits()));
            hash_value(
                &mut hash,
                u64::from(projectile.impact.terrain_height.to_bits()),
            );
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
        altitude: definition
            .flight
            .as_ref()
            .map_or(0.0, |flight| flight.cruise_altitude),
        collision_height_scale: 1.0,
        paralyze_damage: 0.0,
        flanking_direction: definition.strategic.flanking.direction,
        flanking_mobility: definition.strategic.flanking.mobility_add * 1000.0,
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
        unit.altitude = unit
            .definition
            .flight
            .as_ref()
            .map_or(0.0, |flight| flight.cruise_altitude);
        unit.paralyze_damage = 0.0;
        unit.flanking_direction = unit.definition.strategic.flanking.direction;
        unit.flanking_mobility = unit.definition.strategic.flanking.mobility_add * 1000.0;
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

fn armed_unit_ids(units: &[UnitState]) -> [BTreeSet<u32>; 2] {
    std::array::from_fn(|side| {
        units
            .iter()
            .filter(|unit| unit.side as usize == side && unit.definition.has_automatic_weapon())
            .map(|unit| unit.id)
            .collect()
    })
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
        scores[unit.side as usize].firing.armed_units +=
            u64::from(unit.definition.has_automatic_weapon());
        scores[unit.side as usize].starting_value += unit.definition.resource_value();
        scores[unit.side as usize].key_units_started +=
            u32::from(unit.definition.strategic.key_unit);
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
        score.key_units_surviving = units
            .iter()
            .filter(|unit| {
                unit.alive && unit.side as usize == side && unit.definition.strategic.key_unit
            })
            .count() as u32;
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

#[cfg(test)]
mod firing_diagnostics_tests {
    use super::*;
    use crate::learn::tactical::tests::skirmisher;
    use crate::policy::{BuiltinPolicy, parse_policy};

    fn scene() -> Scenario {
        let mut unit = skirmisher("shooter");
        unit.health = 100_000.0;
        unit.weapons[0].avoid_friendly = true;
        unit.weapons[0].turret_yaw_speed = None;
        Scenario {
            terrain: Terrain::flat(1000.0, 1000.0, 16.0),
            units: [(0, 100.0, 400.0), (0, 140.0, 400.0), (1, 240.0, 400.0)]
                .into_iter()
                .map(|(side, x, z)| Spawn {
                    side,
                    definition: unit.clone(),
                    position: Vec2::new(x, z),
                    heading: 0.0,
                })
                .collect(),
            max_time: 3.0,
            trace_resources: vec![],
        }
    }

    #[test]
    fn firing_participation_remaps_ids_when_combat_slots_are_compacted() {
        let mut stepper = CombatStepper::new(
            Terrain::flat(1000.0, 1000.0, 16.0),
            vec![Arc::new(skirmisher("gun"))],
            1,
        );
        for key in 1..=3 {
            stepper
                .insert(CombatSpawn {
                    key,
                    side: 0,
                    definition: CombatDefinitionId(0),
                    position: Vec2::new(100.0 + key as f32 * 30.0, 400.0),
                    heading: 0.0,
                    hp: None,
                })
                .unwrap();
        }
        stepper.battle.fired_units[0].extend([1, 3]);
        stepper.battle.scores[0].firing.units_fired = 2;
        stepper.remove(1).unwrap();
        assert!(stepper.compact_removed());
        assert_eq!(stepper.battle.fired_units[0], BTreeSet::from([2]));
        assert_eq!(stepper.battle.scores[0].firing.units_fired, 2);
        stepper
            .update(
                2,
                CombatantUpdate {
                    side: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(stepper.battle.scores[1].firing.armed_units, 1);
        stepper
            .update(
                2,
                CombatantUpdate {
                    side: Some(0),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(stepper.battle.scores[0].firing.armed_units, 3);
        let mut unarmed = skirmisher("unarmed");
        unarmed.weapons.clear();
        let unarmed_id = stepper.register_definition(Arc::new(unarmed));
        stepper
            .insert(CombatSpawn {
                key: 4,
                side: 1,
                definition: unarmed_id,
                position: Vec2::new(500.0, 400.0),
                heading: 0.0,
                hp: None,
            })
            .unwrap();
        assert_eq!(stepper.battle.scores[1].firing.armed_units, 1);
        stepper
            .update(
                4,
                CombatantUpdate {
                    definition: Some(CombatDefinitionId(0)),
                    ..Default::default()
                },
            )
            .unwrap();
        stepper
            .update(
                4,
                CombatantUpdate {
                    definition: Some(CombatDefinitionId(0)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(stepper.battle.scores[1].firing.armed_units, 2);
    }

    #[test]
    fn firing_diagnostics_match_traced_and_headless_and_count_unique_shooters() {
        let sim = Simulator::new(scene());
        let hold = BuiltinPolicy::Hold;
        let headless = sim.run([&hold, &hold], 5);
        let (traced, _) = sim.run_traced([&hold, &hold], 5, TraceConfig::default());
        let a = headless.sides[0].firing;
        assert_eq!(a, traced.sides[0].firing);
        assert_eq!(a.armed_units, 2);
        assert_eq!(a.units_fired, 1);
        assert!(a.blocked_friendly > 0);
        assert!(a.shots_fired > 1);
        assert_eq!(
            a.ready_checks,
            a.blocked_friendly + a.blocked_terrain + a.shots_fired + a.blocked_resources
        );
    }

    #[test]
    fn surround_assigns_distinct_angular_routes_and_ignores_dead_and_unarmed_allies() {
        let scenario = scene();
        let sim = Simulator::new(scenario);
        let mut workspace = TrialWorkspace::new(&sim);
        let policy = parse_policy("surround").expect("surround baseline");
        let commands = |units: &[UnitState]| {
            let world = WorldView {
                time: 0.0,
                terrain: &sim.scenario.terrain,
                units,
            };
            [
                policy.command(&world, &units[0]),
                policy.command(&world, &units[1]),
            ]
        };
        let before = commands(&workspace.units);
        assert_ne!(before[0].destination, before[1].destination);
        assert!(
            before
                .iter()
                .all(|c| c.target == Some(workspace.units[2].id))
        );
        let mut extra = workspace.units[0].clone();
        extra.id = 99;
        extra.alive = false;
        workspace.units.push(extra.clone());
        assert_eq!(
            before[0].destination,
            commands(&workspace.units)[0].destination
        );
        extra.id = 100;
        extra.alive = true;
        Arc::make_mut(&mut extra.definition).weapons.clear();
        workspace.units.push(extra);
        assert_eq!(
            before[1].destination,
            commands(&workspace.units)[1].destination
        );
    }
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
        if let Some(flight) = &unit.definition.flight {
            let movement = &unit.definition.movement;
            if let Some(destination) = command.destination {
                let delta = destination - unit.position;
                if delta.length_squared() > 0.25 {
                    unit.heading = turn_toward(
                        unit.heading,
                        delta.heading(),
                        movement.turn_rate.to_radians() * DT,
                    );
                    unit.forward = Vec2::from_heading(unit.heading);
                }
            }
            let desired = (movement.max_speed * command.throttle.clamp(0.0, 1.0))
                .max(flight.minimum_speed)
                .min(movement.max_speed);
            let rate = if desired > unit.speed {
                movement.acceleration
            } else {
                movement.braking
            };
            unit.speed = approach(unit.speed, desired, rate * DT);
            let candidate = unit.position + unit.velocity() * DT;
            // Aircraft fly over terrain and occupants; map boundaries remain physical.
            unit.position = Vec2::new(
                candidate
                    .x
                    .clamp(0.0, terrain.cols as f32 * terrain.cell_size),
                candidate
                    .z
                    .clamp(0.0, terrain.rows as f32 * terrain.cell_size),
            );
            unit.altitude = flight.cruise_altitude;
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
    if units[left].definition.flight.is_some()
        || units[right].definition.flight.is_some()
        || !units[right].alive
        || !units[right].definition.movement.blocks_movement
    {
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
    targeting: Option<&TargetingState>,
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
        let target_id = commands[index].target.unwrap_or(0);
        let target_index = live_unit_index(units, target_id).unwrap_or(units.len());
        let side = units[index].side;
        let ground = targeting
            .and_then(|state| state.ground.get(&units[index].id))
            .copied();
        let estimate = if let Some(point) = ground {
            Some(TargetEstimate {
                position: point,
                height: terrain.height_at(point) + PROJECTILE_CLEARANCE + 0.01,
                velocity: Vec2::default(),
                exact: false,
            })
        } else if target_index < units.len() && units[target_index].side != side {
            targeting.map_or_else(
                || {
                    Some(TargetEstimate {
                        position: units[target_index].position,
                        height: unit_collision_mid_y(&units[target_index], terrain),
                        velocity: units[target_index].velocity(),
                        exact: true,
                    })
                },
                |state| state.estimate(side, target_id, units, terrain),
            )
        } else {
            None
        };
        let Some(estimate) = estimate else {
            continue;
        };
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
                || (target_index < units.len()
                    && (!weapon_can_target(weapon, &units[target_index].definition)
                        || weapon
                            .damage_against(&units[target_index].definition.strategic.armor_class)
                            <= 0.0))
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
            let origin_y =
                terrain.height_at(unit_position) + units[index].altitude + local_origin[1];
            let target_position = estimate.position;
            let target_y = estimate.height;
            let target_velocity = estimate.velocity;
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
            let mut aim_point = predicted_target_position_3d(
                origin,
                origin_y,
                target_position,
                target_y,
                target_velocity,
                weapon,
                predict_speed_mod,
            );
            if weapon.weapon_type.eq_ignore_ascii_case("AircraftBomb") {
                if definition.flight.is_none()
                    || target_y >= origin_y
                    || weapon.projectile_gravity <= 0.0
                {
                    continue;
                }
                let fall_time = (2.0 * (origin_y - target_y) / weapon.projectile_gravity).sqrt();
                aim_point = target_position + target_velocity * fall_time;
                let landing = origin + units[index].velocity() * fall_time;
                let release_radius = units
                    .get(target_index)
                    .map_or(5.0, |u| u.definition.movement.radius)
                    + weapon.area_of_effect * 0.35;
                if landing.distance(aim_point) > release_radius.max(5.0) {
                    continue;
                }
            }
            if origin.distance_squared(aim_point)
                + if definition.flight.is_some()
                    || units
                        .get(target_index)
                        .is_some_and(|u| u.definition.flight.is_some())
                {
                    (target_y - origin_y).powi(2)
                } else {
                    0.0
                }
                > weapon.range * weapon.range
            {
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
            if shot_ready {
                battle.scores[side as usize].firing.ready_checks += 1;
            }
            if !terrain_clear_for_flight_path(
                terrain,
                origin,
                aim_point,
                flight_path,
                PROJECTILE_CLEARANCE,
            ) {
                if shot_ready {
                    battle.scores[side as usize].firing.blocked_terrain += 1;
                }
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
                if shot_ready {
                    battle.scores[side as usize].firing.blocked_friendly += 1;
                }
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
            let effective_target_id = units.get(effective_target_index).map_or(0, |unit| unit.id);
            let effective_target_position = units
                .get(effective_target_index)
                .map_or(aim_point, |unit| unit.position);
            let effective_target_speed = if effective_target_index == target_index {
                estimate.velocity.length()
            } else {
                units
                    .get(effective_target_index)
                    .map_or(0.0, |unit| unit.speed.abs())
            };

            if !weapon.stockpile
                && !try_consume_weapon_resources(
                    &mut battle.combat_resources,
                    side,
                    weapon.energy_cost,
                    weapon.metal_cost,
                )
            {
                battle.scores[side as usize].firing.blocked_resources += 1;
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

            battle.scores[side as usize].firing.shots_fired += 1;
            if battle.fired_units[side as usize].insert(shooter_id) {
                battle.scores[side as usize].firing.units_fired += 1;
            }
            let shot_sequence = {
                let state = &mut units[index].weapons[weapon_index];
                state.shots_left -= 1;
                state.next_shot_at += weapon.burst_rate.max(DT);
                let sequence = state.shot_sequence;
                state.shot_sequence = state.shot_sequence.wrapping_add(1);
                sequence
            };
            let effective_target_y = if effective_target_index == target_index {
                target_y
            } else {
                units
                    .get(effective_target_index)
                    .map_or(target_y, |unit| unit_collision_mid_y(unit, terrain))
            };
            fire_shot(
                terrain,
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
        .collision_vertical_bounds(terrain.height_at(unit.position) + unit.altitude);
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
    if weapon.weapon_type.eq_ignore_ascii_case("AircraftBomb") {
        let drop = origin_y - target_y;
        if drop <= 0.0 || weapon.projectile_gravity <= 0.0 {
            return None;
        }
        let flight_time = (2.0 * drop / weapon.projectile_gravity).sqrt();
        return Some(FlightPath::Ballistic {
            origin_y,
            flight_time,
            horizontal_speed: origin.distance(target) / flight_time,
            vertical_speed: 0.0,
            gravity: weapon.projectile_gravity,
        });
    }
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
    terrain: &Terrain,
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
    let impact_altitude = units
        .iter()
        .find(|u| u.id == target_id)
        .map_or(0.0, |u| u.altitude);
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
        let point = if weapon.weapon_type.eq_ignore_ascii_case("AircraftBomb") {
            let drop_time =
                (2.0 * (origin_y - target_y).max(0.0) / weapon.projectile_gravity.max(1e-5)).sqrt();
            let velocity = units
                .iter()
                .find(|u| u.id == shooter_id)
                .map_or(Vec2::default(), UnitState::velocity);
            origin + velocity.rotate(error) * drop_time + move_error
        } else {
            origin + (base_aim - origin).rotate(error)
        };
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
                    altitude: impact_altitude,
                    terrain_height: terrain.height_at(point),
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
                    altitude: impact_altitude,
                    terrain_height: terrain.height_at(point),
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
pub(crate) fn weapon_can_target(weapon: &WeaponDef, target: &crate::model::UnitDef) -> bool {
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
#[cfg(test)]
fn advance_guided_projectiles(
    units: &[UnitState],
    tick: u64,
    projectiles: &mut Vec<GuidedProjectile>,
    impacts: &mut ImpactQueue,
    terrain: &Terrain,
    spatial: &SpatialGrid,
    spatial_candidates: &mut Vec<usize>,
) {
    advance_guided_projectiles_observed(
        units,
        tick,
        projectiles,
        impacts,
        terrain,
        spatial,
        spatial_candidates,
        None,
    );
}

fn advance_guided_projectiles_observed(
    units: &[UnitState],
    tick: u64,
    projectiles: &mut Vec<GuidedProjectile>,
    impacts: &mut ImpactQueue,
    terrain: &Terrain,
    spatial: &SpatialGrid,
    spatial_candidates: &mut Vec<usize>,
    targeting: Option<&TargetingState>,
) {
    if projectiles.is_empty() {
        return;
    }
    let mut write = 0;
    for read in 0..projectiles.len() {
        let mut projectile = projectiles[read];
        let estimate = targeting.map_or_else(
            || {
                live_unit_index(units, projectile.impact.target_id).map(|index| TargetEstimate {
                    position: units[index].position,
                    height: unit_collision_mid_y(&units[index], terrain),
                    velocity: units[index].velocity(),
                    exact: true,
                })
            },
            |state| {
                state.estimate(
                    projectile.impact.shooter_side,
                    projectile.impact.target_id,
                    units,
                    terrain,
                )
            },
        );
        if let Some(estimate) = estimate {
            let target_delta = estimate.position - projectile.position;
            if target_delta.length_squared() > 1e-8 {
                projectile.heading = turn_toward(
                    projectile.heading,
                    target_delta.heading(),
                    projectile.turn_rate * DT,
                );
            }
            let remaining_time = (target_delta.length() / projectile.speed.max(1e-6)).max(DT);
            projectile.vertical_speed = (estimate.height - projectile.height) / remaining_time;
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
            impact.altitude = units[target_index].altitude;
            impact.terrain_height = terrain.height_at(units[target_index].position);
            impact.point = start + (end - start) * hit_t;
            schedule_guided_impact(tick, projectile.damage_pulses, impact, impacts);
            continue;
        }
        if !terrain_clear {
            let mut impact = projectile.impact;
            impact.target_id = 0;
            impact.altitude = 0.0;
            impact.terrain_height = terrain.height_at(end);
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

/// Recoil adds mobility on each 15-frame SlowUpdate. MiniBAR uses a shared
/// phase instead of Recoil's staggered per-unit SlowUpdate scheduling.
fn advance_flanking(units: &mut [UnitState], tick: u64) {
    if !tick.is_multiple_of(15) {
        return;
    }
    for unit in units.iter_mut().filter(|unit| unit.alive) {
        unit.flanking_mobility += unit.definition.strategic.flanking.mobility_add;
    }
}

fn flanking_multiplier(unit: &mut UnitState, attack_direction: Vec2) -> f32 {
    let definition = unit.definition.strategic.flanking;
    if definition.mode == 0 {
        return 1.0;
    }
    let direction = if definition.mode == 1 {
        attack_direction
    } else {
        let forward = unit.forward;
        Vec2::new(
            -attack_direction.x * forward.z + attack_direction.z * forward.x,
            attack_direction.x * forward.x + attack_direction.z * forward.z,
        )
    };
    if definition.mode <= 2 {
        unit.flanking_direction =
            (unit.flanking_direction + direction * unit.flanking_mobility).normalized();
        unit.flanking_mobility = 0.0;
    }
    let dot = direction.x * unit.flanking_direction.x + direction.z * unit.flanking_direction.z;
    (definition.max_damage + definition.min_damage) * 0.5
        - dot * (definition.max_damage - definition.min_damage) * 0.5
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
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
    resolve_impacts_observed(
        units,
        tick,
        impacts,
        damage_events,
        spatial,
        spatial_candidates,
        battle,
        definitions,
        None,
        None,
    );
}

fn resolve_impacts_observed(
    units: &mut [UnitState],
    tick: u64,
    impacts: &mut ImpactQueue,
    damage_events: &mut Vec<DamageEvent>,
    spatial: &SpatialGrid,
    spatial_candidates: &mut Vec<usize>,
    battle: &mut BattleState,
    definitions: &[Arc<crate::model::UnitDef>],
    targeting: Option<&TargetingState>,
    terrain: Option<&Terrain>,
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
            if let Some(state) = targeting {
                if let Some(contacts) = state
                    .contacts
                    .get(impact.shooter_side as usize)
                    .and_then(Option::as_ref)
                {
                    impact
                        .target_id
                        .checked_sub(1)
                        .and_then(|id| contacts.get(id as usize))
                        .copied()
                        .flatten()
                        .map_or(impact.point, |estimate| estimate.position)
                } else {
                    live_unit_index(units, impact.target_id)
                        .map_or(impact.point, |index| units[index].position)
                }
            } else {
                live_unit_index(units, impact.target_id)
                    .map_or(impact.point, |index| units[index].position)
            }
        } else {
            impact.point
        };
        if try_absorb_impact_with_shield(
            units,
            definitions,
            &impact,
            point,
            tick as f32 * DT,
            terrain,
        ) {
            continue;
        }
        // The overwhelmingly common direct hit can resolve by dense target ID
        // without touching the spatial index or scanning the army. A miss still
        // falls through to incidental collision at the sampled impact point.
        let direct_hit = (impact.radius <= 0.0)
            .then(|| {
                collect_direct_target_hit(
                    units,
                    definitions,
                    &impact,
                    point,
                    damage_events,
                    terrain,
                )
            })
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
                terrain,
            )
        } else {
            collect_impact_hits(
                units,
                definitions,
                &impact,
                point,
                0..units.len(),
                damage_events,
                terrain,
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
        // Recoil uses the attacker's current position, not projectile travel
        // direction. Retained dead slots still identify airborne shot sources.
        let attack_direction = event
            .source_id
            .checked_sub(1)
            .and_then(|index| units.get(index as usize))
            .filter(|source| source.id == event.source_id)
            .map(|source| (source.position - units[event.target_index].position).normalized());
        let unit = &mut units[event.target_index];
        let damage = if event.damage > 0.0 {
            event.damage
                * attack_direction.map_or(1.0, |direction| flanking_multiplier(unit, direction))
        } else {
            event.damage
        };
        if event.paralyzer {
            let threshold = unit.definition.health.max(0.0);
            let multiplier = unit.definition.strategic.paralyze_multiplier.max(0.0);
            let maximum =
                threshold + threshold * event.paralyze_time.max(0.0) / PARALYZE_DECLINE_TIME;
            unit.paralyze_damage = (unit.paralyze_damage + damage * multiplier).min(maximum);
            unit.stunned = unit.paralyze_damage > threshold;
            if unit.stunned {
                unit.speed = 0.0;
            }
            continue;
        }

        let actual = damage.min(unit.hp.max(0.0));
        let overkill = (damage - actual).max(0.0);
        unit.hp -= damage;
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
                        altitude: unit.altitude,
                        terrain_height: terrain
                            .map_or(0.0, |terrain| terrain.height_at(unit.position)),
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
    terrain: Option<&Terrain>,
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
        let vertical_distance_squared = impact_vertical_distance(unit, impact, terrain).powi(2);
        for (weapon, state) in unit.definition.weapons.iter().zip(&mut unit.weapons) {
            if !weapon.is_shield
                || weapon.shield_intercept_type & shield_class == 0
                || (!weapon.exterior_shield && unit.id != impact.target_id)
                || unit.position.distance_squared(point) + vertical_distance_squared
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
    terrain: Option<&Terrain>,
) -> Option<bool> {
    let index = live_unit_index(units, impact.target_id)?;
    let target = &units[index];
    if target.side == impact.shooter_side && !impact.hits_allies {
        return None;
    }
    let radius = target.definition.movement.radius;
    if point.distance_squared(target.position)
        + impact_vertical_distance(target, impact, terrain).powi(2)
        > radius * radius
    {
        return None;
    }
    damage_events.push(DamageEvent {
        target_index: index,
        source_id: impact.shooter_id,
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
    terrain: Option<&Terrain>,
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
        let distance_squared = point.distance_squared(unit.position)
            + impact_vertical_distance(unit, impact, terrain).powi(2);
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
            source_id: impact.shooter_id,
            source_side: impact.shooter_side,
            damage: impact_damage_against(impact, unit, definitions) * factor,
            paralyzer: impact.paralyzer,
            paralyze_time: impact.paralyze_time,
        });
        landed = true;
    }
    landed
}

fn impact_vertical_distance(unit: &UnitState, impact: &Impact, terrain: Option<&Terrain>) -> f32 {
    if unit.definition.flight.is_some() || impact.altitude > 0.0 {
        unit.altitude + terrain.map_or(0.0, |terrain| terrain.height_at(unit.position))
            - impact.altitude
            - impact.terrain_height
    } else {
        0.0
    }
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

    // Exercise the real impact resolver: omitting the flanking multiplier must
    // make opposing fire indistinguishable from concentrated fire.
    fn flanking_fixture() -> (Vec<UnitState>, Vec<Arc<crate::model::UnitDef>>) {
        let mut definition = tactical_unit(
            "flanking-target",
            crate::model::MovementDef {
                radius: 5.0,
                ..Default::default()
            },
            Vec::new(),
        );
        definition.strategic.flanking = Default::default();
        let definition = Arc::new(definition);
        let units = vec![
            new_unit_state(
                1,
                0,
                definition.clone(),
                0,
                Vec2::new(200.0, 300.0),
                0.0,
                100.0,
            ),
            new_unit_state(
                2,
                0,
                definition.clone(),
                0,
                Vec2::new(200.0, 100.0),
                0.0,
                100.0,
            ),
            new_unit_state(
                3,
                1,
                definition.clone(),
                0,
                Vec2::new(200.0, 200.0),
                0.0,
                100.0,
            ),
        ];
        (units, vec![definition])
    }

    fn flanking_hits(
        units: &mut [UnitState],
        definitions: &[Arc<crate::model::UnitDef>],
        shooters: &[u32],
        tick: u64,
    ) -> BattleState {
        flanking_hits_with(units, definitions, shooters, tick, |impact| impact)
    }

    fn flanking_hits_with(
        units: &mut [UnitState],
        definitions: &[Arc<crate::model::UnitDef>],
        shooters: &[u32],
        tick: u64,
        modify: impl Fn(Impact) -> Impact,
    ) -> BattleState {
        let mut impacts = ImpactQueue::new(tick + 2);
        for &shooter_id in shooters {
            impacts.schedule(
                tick,
                modify(Impact {
                    altitude: 0.0,
                    terrain_height: 0.0,
                    shooter_id,
                    shooter_side: 0,
                    target_id: 3,
                    point: units[2].position,
                    damage: 10.0,
                    weapon_definition_index: u32::MAX,
                    weapon_index: u16::MAX,
                    damage_fraction: 1.0,
                    radius: 0.0,
                    edge_effectiveness: 1.0,
                    paralyzer: false,
                    paralyze_time: 0.0,
                    hits_allies: false,
                    collide_non_target: true,
                    track_target: false,
                    requires_source: false,
                }),
            );
        }
        let mut battle = BattleState {
            fired_units: Default::default(),
            armed_units: armed_unit_ids(units),
            scores: initial_scores(units),
            alive_counts: count_live_sides(units),
            combat_resources: None,
        };
        let spatial = SpatialGrid::new(&Terrain::flat(400.0, 400.0, 16.0), 64.0);
        resolve_impacts(
            units,
            tick,
            &mut impacts,
            &mut Vec::new(),
            &spatial,
            &mut Vec::new(),
            &mut battle,
            definitions,
        );
        battle
    }

    #[test]
    fn flanking_opposing_fire_deals_more_damage_than_concentrated_fire() {
        let (units, definitions) = flanking_fixture();
        let mut concentrated = units.clone();
        let mut opposed = units;
        let same = flanking_hits(&mut concentrated, &definitions, &[1, 1], 0);
        let opposite = flanking_hits(&mut opposed, &definitions, &[1, 2], 0);
        assert_eq!(same.scores[0].damage_dealt, 20.0);
        assert!(opposite.scores[0].damage_dealt > 29.9);
        assert!(opposed[2].hp < concentrated[2].hp - 9.9);
        assert_eq!(
            opposite.scores[0].damage_dealt,
            opposite.scores[1].damage_received
        );
    }

    #[test]
    fn flanking_uses_attacker_position_at_impact_independently_of_hull_heading() {
        let (mut units, definitions) = flanking_fixture();
        flanking_hits(&mut units, &definitions, &[1], 0);
        units[2].heading = PI;
        units[2].forward = Vec2::from_heading(PI);
        units[0].position = units[1].position;
        let battle = flanking_hits(&mut units, &definitions, &[1], 0);
        assert!(battle.scores[0].damage_dealt > 19.9);
    }

    #[test]
    fn flanking_recovers_ability_to_face_a_new_attack_direction() {
        let (mut units, definitions) = flanking_fixture();
        flanking_hits(&mut units, &definitions, &[1], 0);
        units[1].position = Vec2::new(300.0, 200.0);
        let mut immediate = units.clone();
        assert_eq!(
            flanking_hits(&mut immediate, &definitions, &[2], 1).scores[0].damage_dealt,
            15.0
        );
        // 100 slow updates add one unit of mobility: a perpendicular attack
        // turns the remembered direction 45 degrees before damage is computed.
        for tick in 1..=1500 {
            advance_flanking(&mut units, tick);
        }
        let recovered = flanking_hits(&mut units, &definitions, &[2], 1500);
        assert!((recovered.scores[0].damage_dealt - 11.464466).abs() < 1e-4);
    }

    #[test]
    fn flanking_applies_to_splash_and_emp_without_inflating_hp_scores() {
        for radius in [0.0, 10.0] {
            for paralyzer in [false, true] {
                let (mut units, definitions) = flanking_fixture();
                flanking_hits(&mut units, &definitions, &[1], 0);
                let battle =
                    flanking_hits_with(&mut units, &definitions, &[2], 0, |impact| Impact {
                        radius,
                        paralyzer,
                        paralyze_time: 10.0,
                        ..impact
                    });
                if paralyzer {
                    assert_eq!(units[2].hp, 90.0);
                    assert_eq!(units[2].paralyze_damage, 20.0);
                    assert_eq!(battle.scores[0].damage_dealt, 0.0);
                } else {
                    assert_eq!(units[2].hp, 70.0);
                    assert_eq!(battle.scores[0].damage_dealt, 20.0);
                }
            }
        }
    }

    #[test]
    fn flanking_disabled_and_missing_attacker_leave_damage_unmodified() {
        let (mut units, definitions) = flanking_fixture();
        Arc::make_mut(&mut units[2].definition)
            .strategic
            .flanking
            .mode = 0;
        assert_eq!(
            flanking_hits(&mut units, &definitions, &[1, 2], 0).scores[0].damage_dealt,
            20.0
        );
        let (mut units, definitions) = flanking_fixture();
        assert_eq!(
            flanking_hits(&mut units, &definitions, &[0], 0).scores[0].damage_dealt,
            10.0
        );
    }

    #[test]
    fn flanking_trial_reset_restores_first_hit_behavior() {
        let (mut units, definitions) = flanking_fixture();
        let spawns: Vec<_> = units
            .iter()
            .map(|unit| Spawn {
                side: unit.side,
                definition: (*unit.definition).clone(),
                position: unit.position,
                heading: unit.heading,
            })
            .collect();
        flanking_hits(&mut units, &definitions, &[2, 1], 0);
        reset_units(&mut units, &spawns);
        assert_eq!(
            flanking_hits(&mut units, &definitions, &[2, 1], 0).scores[0].damage_dealt,
            30.0
        );
    }

    #[test]
    fn flanking_retained_state_changes_replay_hash() {
        let mut stepper = CombatStepper::new(
            Terrain::flat(400.0, 400.0, 16.0),
            vec![stepper_weapon(10.0), stepper_target()],
            7,
        );
        insert_stepper_pair(&mut stepper);
        let initial = stepper.deterministic_state_hash();
        stepper.units[1].flanking_direction = Vec2::new(1.0, 0.0);
        assert_ne!(stepper.deterministic_state_hash(), initial);
        let changed = stepper.deterministic_state_hash();
        stepper.units[1].flanking_mobility = 0.0;
        assert_ne!(stepper.deterministic_state_hash(), changed);
    }

    #[test]
    fn flanking_definition_replacement_initializes_new_armour_facing() {
        let mut replacement = (*stepper_target()).clone();
        replacement.strategic.flanking.mode = 3;
        replacement.strategic.flanking.direction = Vec2::new(1.0, 0.0);
        let mut stepper = CombatStepper::new(
            Terrain::flat(400.0, 400.0, 16.0),
            vec![
                stepper_weapon(10.0),
                stepper_target(),
                Arc::new(replacement),
            ],
            7,
        );
        insert_stepper_pair(&mut stepper);
        // A world-space +Z direction from previous mode-1 combat would be
        // incorrectly read as local forward after changing to fixed armour.
        stepper.units[1].flanking_direction = Vec2::new(0.0, 1.0);
        stepper.units[1].flanking_mobility = 0.0;
        stepper
            .update(
                200,
                CombatantUpdate {
                    definition: Some(CombatDefinitionId(2)),
                    ..Default::default()
                },
            )
            .unwrap();
        // New local +right armour is perpendicular to the incoming attack.
        let damage = flanking_multiplier(&mut stepper.units[1], Vec2::new(-1.0, 0.0));
        assert!((damage - 1.5).abs() < 1e-6);
    }

    #[test]
    fn flanking_absorbed_shots_do_not_turn_damage_facing() {
        let mut shooter = (*stepper_weapon(10.0)).clone();
        shooter.weapons[0].intercepted_by_shield_type = 1;
        let mut target = (*shielded_target()).clone();
        target.strategic.flanking = Default::default();
        let mut stepper = CombatStepper::new(
            Terrain::flat(400.0, 400.0, 16.0),
            vec![Arc::new(shooter), Arc::new(target)],
            7,
        );
        insert_stepper_pair(&mut stepper);
        stepper.step();
        assert_eq!(stepper.shield_charge(200, 0), Some(90.0));
        assert_eq!(stepper.units[1].hp, 100.0);
        assert_eq!(stepper.units[1].flanking_direction, Vec2::new(0.0, 1.0));
        assert!(stepper.units[1].flanking_mobility >= 10.0);
    }

    #[test]
    fn flanking_dead_shooters_still_supply_airborne_damage_direction() {
        let (mut units, definitions) = flanking_fixture();
        flanking_hits(&mut units, &definitions, &[1], 0);
        units[1].alive = false;
        units[1].hp = 0.0;
        let battle = flanking_hits(&mut units, &definitions, &[2], 1);
        assert_eq!(battle.scores[0].damage_dealt, 20.0);
    }

    #[test]
    fn flanking_unit_relative_modes_follow_hull_rotation() {
        for mode in [1, 2, 3] {
            let (mut units, definitions) = flanking_fixture();
            Arc::make_mut(&mut units[2].definition)
                .strategic
                .flanking
                .mode = mode;
            // +X is local forward for a zero-heading MiniBAR unit.
            units[0].position = Vec2::new(300.0, 200.0);
            flanking_hits(&mut units, &definitions, &[1], 0);
            units[2].heading = PI;
            units[2].forward = Vec2::new(-1.0, 0.0);
            let damage = flanking_hits(&mut units, &definitions, &[1], 0).scores[0].damage_dealt;
            if mode == 1 {
                assert!(damage < 10.03);
            } else {
                assert_eq!(damage, 20.0);
            }
        }
    }

    #[test]
    fn flanking_simulator_pooled_trials_are_seed_stable() {
        let mut shooter = (*stepper_weapon(10.0)).clone();
        shooter.weapons[0].range = 500.0;
        shooter.weapons[0].reload = 100.0;
        let mut target = (*stepper_target()).clone();
        target.strategic.flanking = Default::default();
        let scenario = Scenario {
            terrain: Terrain::flat(400.0, 400.0, 16.0),
            units: vec![
                Spawn {
                    side: 0,
                    definition: shooter.clone(),
                    position: Vec2::new(200.0, 100.0),
                    heading: PI * 0.5,
                },
                Spawn {
                    side: 0,
                    definition: shooter,
                    position: Vec2::new(200.0, 300.0),
                    heading: -PI * 0.5,
                },
                Spawn {
                    side: 1,
                    definition: target,
                    position: Vec2::new(200.0, 200.0),
                    heading: 0.0,
                },
            ],
            max_time: 1.0,
            trace_resources: Vec::new(),
        };
        let simulator = Simulator::new(scenario);
        let hold = crate::policy::BuiltinPolicy::Hold;
        let single = simulator.run([&hold, &hold], 7);
        assert_eq!(single.sides[0].damage_dealt, 30.0);
        let batch = simulator.run_parallel_results([&hold, &hold], 64, 7);
        for result in batch {
            assert_eq!(result.sides[0].damage_dealt, 30.0);
            assert_eq!(result.sides[1].damage_received, 30.0);
        }
    }

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
            flight: None,
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
            altitude: 0.0,
            terrain_height: 0.0,
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
            flight: None,
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
            altitude: 0.0,
            terrain_height: 0.0,
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
                None,
            ),
            Some(true)
        );
        assert_eq!(
            damage_events,
            vec![DamageEvent {
                target_index: 0,
                source_id: 2,
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
                None,
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
            // These fixtures isolate base damage, movement and weapon rules.
            // Flanking fixtures explicitly restore BAR's enabled defaults.
            strategic: crate::model::StrategicDef {
                flanking: crate::model::FlankingDef {
                    mode: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            flight: None,
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
            altitude: 0.0,
            terrain_height: 0.0,
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
                None,
            ),
            Some(true)
        );
        assert_eq!(
            damage_events,
            vec![DamageEvent {
                target_index: 1,
                source_id: 1,
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
                altitude: 0.0,
                terrain_height: 0.0,
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
            fired_units: Default::default(),
            armed_units: armed_unit_ids(&units),
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
                altitude: 0.0,
                terrain_height: 0.0,
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
            fired_units: Default::default(),
            armed_units: armed_unit_ids(&units),
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
    fn live_primary_turret_skips_mounts_without_modeled_aim() {
        for excluded in [
            WeaponDef {
                is_shield: true,
                ..Default::default()
            },
            WeaponDef {
                manual_fire: true,
                ..Default::default()
            },
            WeaponDef {
                interceptor: 1,
                ..Default::default()
            },
        ] {
            let mut definition = (*stepper_weapon(10.0)).clone();
            definition.weapons.insert(0, excluded);
            let mut stepper = CombatStepper::new(
                Terrain::flat(400.0, 200.0, 16.0),
                vec![Arc::new(definition), stepper_target()],
                3,
            );
            insert_stepper_pair(&mut stepper);
            // Excluded mounts may retain arbitrary spawn yaw; the real gun is at zero.
            stepper.units[0].weapons[0].aim_heading = 1.0;
            assert_eq!(stepper.primary_turret_heading(100), Some(0.0));
        }
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
                altitude: 0.0,
                terrain_height: 0.0,
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
    #[test]
    fn sensor_contacts_gate_forged_target_and_use_disclosed_aim_point() {
        let mut stepper = CombatStepper::new(
            Terrain::flat(400.0, 240.0, 16.0),
            vec![stepper_weapon(20.0), stepper_target()],
            3,
        );
        insert_stepper_pair(&mut stepper);
        stepper.set_target_observations(0, &[]).unwrap();
        for _ in 0..2 {
            stepper.step();
        }
        assert_eq!(stepper.combatant(200).unwrap().hp, 100.0);
        stepper
            .set_target_observations(
                0,
                &[TargetObservation {
                    target: 200,
                    position: Vec2::new(200.0, 180.0),
                    height: 5.0,
                    velocity: None,
                    exact: false,
                }],
            )
            .unwrap();
        for _ in 0..30 {
            stepper.step();
        }
        assert_eq!(
            stepper.combatant(200).unwrap().hp,
            100.0,
            "radar aiming must not snap back to truth"
        );
    }

    #[test]
    fn sensor_ground_fire_can_hit_without_entity_acquisition() {
        let mut stepper = CombatStepper::new(
            Terrain::flat(400.0, 240.0, 16.0),
            vec![stepper_weapon(20.0), stepper_target()],
            3,
        );
        insert_stepper_pair(&mut stepper);
        stepper.set_target_observations(0, &[]).unwrap();
        stepper.set_command(100, CombatCommand::default()).unwrap();
        stepper
            .set_ground_target(100, Some(Vec2::new(200.0, 100.0)))
            .unwrap();
        for _ in 0..3 {
            stepper.step();
        }
        assert!(stepper.combatant(200).unwrap().hp < 100.0);
    }

    #[test]
    fn sensor_guidance_does_not_track_true_hidden_position() {
        let target = stepper_target();
        let units = vec![
            new_unit_state(1, 0, target.clone(), 0, Vec2::new(20.0, 20.0), 0.0, 100.0),
            new_unit_state(2, 1, target, 0, Vec2::new(100.0, 150.0), PI, 100.0),
        ];
        let terrain = Terrain::flat(300.0, 300.0, 16.0);
        let mut spatial = SpatialGrid::new(&terrain, 64.0);
        spatial.rebuild(&units);
        let mut contacts = TargetingState::default();
        contacts.contacts[0] = Some(Vec::new());
        let mut rounds = vec![guided_test_projectile(0.0, 90.0, 30)];
        let mut impacts = ImpactQueue::new(100);
        let mut candidates = Vec::new();
        advance_guided_projectiles_observed(
            &units,
            0,
            &mut rounds,
            &mut impacts,
            &terrain,
            &spatial,
            &mut candidates,
            Some(&contacts),
        );
        assert_eq!(
            rounds[0].heading, 0.0,
            "lost contact retains course rather than homing on truth"
        );
    }

    #[test]
    fn standalone_sensor_view_hides_unseen_units_and_rejects_forged_orders() {
        struct Forged;
        impl Policy for Forged {
            fn command(&self, view: &WorldView<'_>, unit: &UnitState) -> Command {
                if unit.side == 0 {
                    assert!(view.units.iter().all(|other| other.side == 0));
                    Command {
                        target: Some(2),
                        ..Default::default()
                    }
                } else {
                    Command::default()
                }
            }
        }
        let mut shooter = (*stepper_weapon(20.0)).clone();
        shooter.strategic.sight_distance = Some(30.0);
        let scenario = Scenario {
            terrain: Terrain::flat(400.0, 240.0, 16.0),
            units: vec![
                Spawn {
                    side: 0,
                    definition: shooter,
                    position: Vec2::new(100.0, 100.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: (*stepper_target()).clone(),
                    position: Vec2::new(200.0, 100.0),
                    heading: PI,
                },
            ],
            max_time: 0.2,
            trace_resources: vec![],
        };
        let result = Simulator::new(scenario).run([&Forged, &Forged], 1);
        assert_eq!(result.sides[1].damage_received, 0.0);
    }
    #[test]
    fn sensor_radar_snapshot_censors_health_velocity_and_definition() {
        let mut shooter = (*stepper_weapon(20.0)).clone();
        shooter.strategic.sight_distance = Some(20.0);
        shooter.strategic.radar_radius = Some(300.0);
        let mut enemy = (*stepper_target()).clone();
        enemy.movement.max_speed = 100.0;
        let truth = vec![
            new_unit_state(
                1,
                0,
                Arc::new(shooter),
                0,
                Vec2::new(100.0, 100.0),
                0.0,
                100.0,
            ),
            new_unit_state(2, 1, Arc::new(enemy), 1, Vec2::new(200.0, 100.0), PI, 20.0),
        ];
        let terrain = Terrain::flat(400.0, 240.0, 16.0);
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 3, 0, &mut None);
        let radar = sensors.views[0].iter().find(|unit| unit.id == 2).unwrap();
        assert_eq!(radar.definition.name, "unknown-contact");
        assert_eq!(radar.hp, 0.0);
        assert_eq!(radar.velocity(), Vec2::default());
        assert!(radar.weapons.is_empty());
        assert_ne!(radar.position, truth[1].position);
        let earlier = radar.position;
        sensors.refresh(&truth, &terrain, 3, 64, &mut None);
        assert_ne!(
            sensors.views[0]
                .iter()
                .find(|unit| unit.id == 2)
                .unwrap()
                .position,
            earlier
        );
    }

    #[test]
    fn sensor_start_cloak_uses_energy_and_proximity_but_damage_alone_does_not_break_it() {
        let mut observer = (*stepper_target()).clone();
        observer.strategic.sight_distance = Some(300.0);
        let mut hidden = observer.clone();
        hidden.strategic.can_cloak = true;
        hidden.strategic.start_cloaked = true;
        hidden.strategic.cloak_cost = 30.0;
        hidden.strategic.min_cloak_distance = 50.0;
        let mut truth = vec![
            new_unit_state(
                1,
                0,
                Arc::new(observer),
                0,
                Vec2::new(100.0, 100.0),
                0.0,
                100.0,
            ),
            new_unit_state(
                2,
                1,
                Arc::new(hidden),
                1,
                Vec2::new(200.0, 100.0),
                PI,
                100.0,
            ),
        ];
        let terrain = Terrain::flat(400.0, 240.0, 16.0);
        let mut sensors = StandaloneSensors::new(2);
        let mut resources = Some([
            StartingResources {
                metal: 0.0,
                energy: 0.0,
            },
            StartingResources {
                metal: 0.0,
                energy: 20.0,
            },
        ]);
        sensors.refresh(&truth, &terrain, 1, 0, &mut resources);
        assert!(sensors.cloaked[1]);
        truth[1].hp = 50.0;
        sensors.refresh(&truth, &terrain, 1, 1, &mut resources);
        assert!(sensors.cloaked[1], "damage decloak is not BAR's default");
        sensors.refresh(&truth, &terrain, 1, 14, &mut resources);
        assert!(sensors.cloaked[1]);
        sensors.refresh(&truth, &terrain, 1, 15, &mut resources);
        assert!(!sensors.cloaked[1], "finite energy must limit cloak");
        truth[1].position = Vec2::new(120.0, 100.0);
        sensors.refresh(&truth, &terrain, 1, 30, &mut None);
        assert!(!sensors.cloaked[1]);
        truth[1].position = Vec2::new(200.0, 100.0);
        sensors.refresh(&truth, &terrain, 1, 45, &mut None);
        assert!(
            sensors.cloaked[1],
            "leaving proximity allows the next ordinary check; only firing adds128frames"
        );
    }

    #[test]
    fn sensor_beam_pulse_holds_launch_point_after_contact_loss() {
        let mut gun = (*stepper_weapon(30.0)).clone();
        gun.weapons[0].damage_duration = 0.3;
        let mut stepper = CombatStepper::new(
            Terrain::flat(400.0, 240.0, 16.0),
            vec![Arc::new(gun), stepper_target()],
            3,
        );
        insert_stepper_pair(&mut stepper);
        stepper
            .set_target_observations(
                0,
                &[TargetObservation {
                    target: 200,
                    position: Vec2::new(200.0, 100.0),
                    height: 5.0,
                    velocity: None,
                    exact: true,
                }],
            )
            .unwrap();
        stepper.step();
        let damaged = stepper.combatant(200).unwrap().hp;
        stepper
            .update(
                200,
                CombatantUpdate {
                    position: Some(Vec2::new(200.0, 170.0)),
                    ..Default::default()
                },
            )
            .unwrap();
        stepper.set_target_observations(0, &[]).unwrap();
        for _ in 0..15 {
            stepper.step();
        }
        assert_eq!(
            stepper.combatant(200).unwrap().hp,
            damaged,
            "old beam must not follow hidden movement"
        );
    }
    fn standalone_truth_pair(building: bool) -> (Vec<UnitState>, Terrain) {
        let mut observer = (*stepper_target()).clone();
        observer.strategic.sight_distance = Some(100.0);
        observer.strategic.radar_radius = Some(500.0);
        let mut enemy = (*stepper_target()).clone();
        enemy.name = "known-enemy".into();
        enemy.strategic.leaves_ghost = Some(building);
        enemy.strategic.is_building = building;
        enemy.strategic.is_mobile = !building;
        enemy.movement.max_speed = if building { 0.0 } else { 80.0 };
        (
            vec![
                new_unit_state(
                    1,
                    0,
                    Arc::new(observer),
                    0,
                    Vec2::new(100.0, 100.0),
                    0.0,
                    100.0,
                ),
                new_unit_state(2, 1, Arc::new(enemy), 1, Vec2::new(150.0, 100.0), PI, 100.0),
            ],
            Terrain::flat(900.0, 300.0, 16.0),
        )
    }

    #[test]
    fn standalone_emp_keeps_visual_sight_but_disables_radar() {
        let (mut truth, terrain) = standalone_truth_pair(false);
        truth[0].stunned = true;
        Arc::make_mut(&mut truth[1].definition).weapons = stepper_weapon(1.0).weapons.clone();
        truth[1].weapons = initial_weapon_states(&truth[1].definition, truth[1].heading);
        truth[1].weapons[0].reload_until = 123.0;
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        assert!(sensors.views[0].iter().any(|unit| unit.id == 2));
        assert!(
            sensors.views[0]
                .iter()
                .find(|unit| unit.id == 2)
                .unwrap()
                .weapons
                .is_empty(),
            "LOS discloses a known weapon definition, never private live mount state"
        );
        assert!(sensors.units[0].active);
        assert!(!sensors.units[0].enabled);
        truth[1].position.x = 350.0;
        sensors.refresh(&truth, &terrain, 1, 1, &mut None);
        assert!(sensors.views[0].iter().all(|unit| unit.id != 2));
    }

    #[test]
    fn standalone_mobile_identity_survives_continuous_radar_but_not_a_contact_gap() {
        let (mut truth, terrain) = standalone_truth_pair(false);
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        truth[1].position.x = 350.0;
        sensors.refresh(&truth, &terrain, 1, 1, &mut None);
        let radar = sensors.views[0].iter().find(|unit| unit.id == 2).unwrap();
        assert_eq!(radar.definition.name, "known-enemy");
        assert_eq!(radar.hp, 0.0);
        truth[1].position.x = 800.0;
        sensors.refresh(&truth, &terrain, 1, 2, &mut None);
        assert!(sensors.views[0].iter().all(|unit| unit.id != 2));
        truth[1].position.x = 350.0;
        sensors.refresh(&truth, &terrain, 1, 3, &mut None);
        assert_eq!(
            sensors.views[0]
                .iter()
                .find(|unit| unit.id == 2)
                .unwrap()
                .definition
                .name,
            "unknown-contact"
        );
    }

    #[test]
    fn standalone_cloak_half_second_charges_use_recoil_motion_threshold() {
        let (mut truth, terrain) = standalone_truth_pair(false);
        let d = &mut Arc::make_mut(&mut truth[1].definition).strategic;
        d.can_cloak = true;
        d.start_cloaked = true;
        d.cloak_cost = 30.0;
        d.cloak_cost_moving = Some(120.0);
        truth[1].speed = 1.0;
        let mut sensors = StandaloneSensors::new(2);
        let mut pools = Some([
            StartingResources {
                metal: 0.0,
                energy: 0.0,
            },
            StartingResources {
                metal: 0.0,
                energy: 100.0,
            },
        ]);
        sensors.refresh(&truth, &terrain, 1, 0, &mut pools);
        assert_eq!(pools.unwrap()[1].energy, 85.0);
        sensors.refresh(&truth, &terrain, 1, 14, &mut pools);
        assert_eq!(pools.unwrap()[1].energy, 85.0);
        truth[1].speed = 20.0;
        sensors.refresh(&truth, &terrain, 1, 15, &mut pools);
        assert_eq!(pools.unwrap()[1].energy, 25.0);
    }

    #[test]
    fn standalone_building_ghost_is_frozen_and_cleared_only_by_los_of_empty_position() {
        let (mut truth, terrain) = standalone_truth_pair(true);
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        let remembered = truth[1].position;
        // Lose the contact first; destruction behind fog must not clear this ghost.
        truth[0].position.x = 800.0;
        sensors.refresh(&truth, &terrain, 1, 1, &mut None);
        truth[1].alive = false;
        sensors.refresh(&truth, &terrain, 1, 2, &mut None);
        // Radar coverage of an empty location cannot confirm an already hidden death.
        truth[0].position.x = 450.0;
        sensors.refresh(&truth, &terrain, 1, 3, &mut None);
        let ghost = sensors.views[0]
            .iter()
            .find(|unit| unit.id == 2)
            .expect("radar cannot confirm hidden destruction");
        assert_eq!(ghost.position, remembered);
        assert_eq!(ghost.hp, 0.0);
        assert!(ghost.alive);
        assert!(sensors.targeting.contacts[0].as_ref().unwrap()[1].is_none());
        truth[0].position.x = 100.0;
        sensors.refresh(&truth, &terrain, 1, 4, &mut None);
        assert!(sensors.views[0].iter().all(|unit| unit.id != 2));
    }

    #[test]
    fn standalone_live_cloaked_building_retains_ghost_and_attack_translates_to_ground() {
        let (mut truth, terrain) = standalone_truth_pair(true);
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        let d = &mut Arc::make_mut(&mut truth[1].definition).strategic;
        d.can_cloak = true;
        d.start_cloaked = true;
        d.stealth = true;
        sensors.refresh(&truth, &terrain, 1, 15, &mut None);
        assert!(sensors.views[0].iter().any(|unit| unit.id == 2));
        let command = sensors.sanitize_command(
            0,
            1,
            Command {
                target: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(command.target, None);
        assert_eq!(
            sensors.targeting.ground.get(&1),
            Some(&Vec2::new(150.0, 100.0))
        );
    }
    #[test]
    fn standalone_spherical_decloak_includes_enemy_radius_but_cylinder_does_not() {
        let (mut truth, terrain) = standalone_truth_pair(false);
        truth[0].position.x = 40.0;
        Arc::make_mut(&mut truth[0].definition).movement.radius = 20.0;
        let d = &mut Arc::make_mut(&mut truth[1].definition).strategic;
        d.can_cloak = true;
        d.start_cloaked = true;
        d.min_cloak_distance = 100.0;
        d.decloak_spherical = true;
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        assert!(
            !sensors.cloaked[1],
            "centers110 minus enemy radius20 is inside radius100"
        );
        Arc::make_mut(&mut truth[1].definition)
            .strategic
            .decloak_spherical = false;
        sensors.refresh(&truth, &terrain, 1, 15, &mut None);
        assert!(
            sensors.cloaked[1],
            "cylindrical check uses center distance only"
        );
    }

    #[test]
    fn standalone_radar_tracked_building_death_does_not_create_a_dead_ghost() {
        let (mut truth, terrain) = standalone_truth_pair(true);
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        truth[0].position.x = 450.0;
        sensors.refresh(&truth, &terrain, 1, 1, &mut None);
        assert!(sensors.targeting.contacts[0].as_ref().unwrap()[1].is_some());
        truth[1].alive = false;
        sensors.refresh(&truth, &terrain, 1, 2, &mut None);
        assert!(
            sensors.views[0].iter().all(|unit| unit.id != 2),
            "a currently radar-tracked death leaves no ghost"
        );
    }
    #[test]
    fn standalone_firing_alone_blocks_recloak_for128_frames() {
        let (mut truth, terrain) = standalone_truth_pair(false);
        let definition = Arc::make_mut(&mut truth[1].definition);
        definition.strategic.can_cloak = true;
        definition.strategic.start_cloaked = true;
        definition.weapons = stepper_weapon(1.0).weapons.clone();
        truth[1].weapons = initial_weapon_states(&truth[1].definition, truth[1].heading);
        let mut sensors = StandaloneSensors::new(2);
        sensors.refresh(&truth, &terrain, 1, 0, &mut None);
        assert!(sensors.cloaked[1]);
        truth[1].weapons[0].shot_sequence = 1;
        for tick in 1..129 {
            sensors.refresh(&truth, &terrain, 1, tick, &mut None);
            assert!(!sensors.cloaked[1], "premature recloak at tick{tick}");
        }
        for tick in 129..=135 {
            sensors.refresh(&truth, &terrain, 1, tick, &mut None);
        }
        assert!(
            sensors.cloaked[1],
            "next half-second admission after firing timeout must recloak"
        );
    }
    fn flight_fixture() -> crate::model::UnitDef {
        let ground = tactical_unit(
            "air fixture",
            crate::model::MovementDef {
                max_speed: 120.0,
                acceleration: 120.0,
                braking: 120.0,
                turn_rate: 90.0,
                radius: 2.0,
                ..Default::default()
            },
            vec![],
        );
        let mut json = serde_json::to_value(ground).unwrap();
        json["flight"] =
            serde_json::json!({"cruise_altitude":100.0,"minimum_speed":60.0,"attack_run":true});
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn flight_crosses_blocked_terrain_without_stopping_or_reversing() {
        let definition = flight_fixture();
        let mut terrain = Terrain::flat(512.0, 512.0, 32.0);
        terrain.passable.fill(false);
        let mut units = vec![new_unit_state(
            1,
            0,
            Arc::new(definition),
            0,
            Vec2::new(200.0, 200.0),
            0.0,
            100.0,
        )];
        let commands = [Command {
            destination: Some(Vec2::new(20.0, 200.0)),
            throttle: 0.0,
            ..Default::default()
        }];
        for _ in 0..15 {
            update_movement(&mut units, &commands, &terrain, false, &[]);
        }
        assert!(
            units[0].speed >= 59.9,
            "fixed-wing flight retains cruise speed"
        );
        assert!(units[0].position.distance(Vec2::new(200.0, 200.0)) > 5.0);
        assert!(
            units[0].heading.abs() > 0.2 && units[0].heading.abs() < 1.0,
            "turn rate is bounded"
        );
        assert!(unit_collision_mid_y(&units[0], &terrain) >= 100.0);
    }

    #[test]
    fn flight_ground_occupancy_does_not_displace_aircraft() {
        let mut units = vec![
            new_unit_state(
                1,
                0,
                Arc::new(flight_fixture()),
                0,
                Vec2::new(200.0, 200.0),
                0.0,
                100.0,
            ),
            new_unit_state(
                2,
                1,
                Arc::new(tactical_unit("ground", Default::default(), vec![])),
                1,
                Vec2::new(201.0, 200.0),
                0.0,
                100.0,
            ),
        ];
        let before = units[0].position;
        separate_unit_pair(
            &mut units,
            &Terrain::flat(512.0, 512.0, 32.0),
            0,
            1,
            true,
            &[],
        );
        assert_eq!(units[0].position, before);
    }

    #[test]
    fn aircraft_bomb_falls_under_gravity_with_delayed_impact() {
        let weapon = WeaponDef {
            weapon_type: "AircraftBomb".into(),
            projectile_gravity: 360.0,
            ..Default::default()
        };
        let path = weapon_flight_path(
            Vec2::new(0.0, 0.0),
            180.0,
            Vec2::new(100.0, 0.0),
            0.0,
            &weapon,
        )
        .unwrap();
        assert!(
            (path.flight_time() - 1.0).abs() < 1e-5,
            "fall takes one second"
        );
        assert!((path.height_at_fraction(0.5) - 135.0).abs() < 1e-4);
        assert!(
            weapon_flight_path(
                Vec2::new(0.0, 0.0),
                0.0,
                Vec2::new(100.0, 0.0),
                180.0,
                &weapon
            )
            .is_none()
        );
    }

    #[test]
    fn aircraft_is_not_hit_by_ground_splash_underneath_it() {
        let (mut units, mut definitions) = flanking_fixture();
        let mut aircraft = flight_fixture();
        aircraft.strategic.flanking.mode = 0;
        definitions.push(Arc::new(aircraft));
        units.push(new_unit_state(
            4,
            1,
            Arc::clone(&definitions[1]),
            1,
            units[2].position,
            0.0,
            100.0,
        ));
        flanking_hits_with(&mut units, &definitions, &[1], 0, |mut impact| {
            impact.radius = 20.0;
            impact
        });
        assert!(units[2].hp < 100.0, "ground target takes splash");
        assert_eq!(units[3].hp, 100.0, "aircraft above blast remains unharmed");
    }

    #[test]
    fn bomber_attack_run_releases_inherited_velocity_bombs_against_ground() {
        let mut bomber = flight_fixture();
        bomber.weapons = vec![WeaponDef {
            weapon_type: "AircraftBomb".into(),
            projectile_gravity: 360.0,
            damage: 200.0,
            range: 500.0,
            reload: 2.0,
            area_of_effect: 80.0,
            edge_effectiveness: 0.5,
            ..Default::default()
        }];
        let ground = tactical_unit(
            "ground",
            crate::model::MovementDef {
                radius: 10.0,
                height: 20.0,
                ..Default::default()
            },
            vec![],
        );
        let scenario = Scenario {
            terrain: Terrain::flat(1500.0, 1000.0, 32.0),
            max_time: 8.0,
            trace_resources: vec![],
            units: vec![
                Spawn {
                    side: 0,
                    definition: bomber,
                    position: Vec2::new(200.0, 500.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: ground,
                    position: Vec2::new(500.0, 500.0),
                    heading: 0.0,
                },
            ],
        };
        let sim = Simulator::new(scenario);
        let result = sim.run(
            [
                &crate::policy::BuiltinPolicy::Hold,
                &crate::policy::BuiltinPolicy::Hold,
            ],
            1,
        );
        assert!(result.sides[0].projectiles_fired > 0);
        assert_eq!(result.winner, Some(0));
        assert!(result.duration > 1.0, "bomb release and falling take time");
    }

    #[test]
    fn air_domain_router_uses_frozen_ground_and_independent_air_checkpoints() {
        use crate::learn::{air, tactical};
        let ground = tactical::template(3).unwrap();
        let air_brain = air::template(3).unwrap();
        let snapshot = ground.to_json();
        let router = air::DomainPolicy::new(air_brain.clone(), ground.clone()).unwrap();
        assert!(air::DomainPolicy::new(ground.clone(), ground.clone()).is_err());
        let mut plane = flight_fixture();
        plane.weapons = stepper_weapon(10.0).weapons.clone();
        let units = vec![
            new_unit_state(
                1,
                0,
                Arc::new(plane),
                0,
                Vec2::new(200.0, 200.0),
                0.0,
                100.0,
            ),
            new_unit_state(
                2,
                0,
                stepper_weapon(10.0),
                1,
                Vec2::new(200.0, 250.0),
                0.0,
                100.0,
            ),
            new_unit_state(
                3,
                1,
                stepper_target(),
                2,
                Vec2::new(400.0, 200.0),
                0.0,
                100.0,
            ),
        ];
        let terrain = Terrain::flat(1000.0, 1000.0, 32.0);
        let world = WorldView {
            time: 0.0,
            terrain: &terrain,
            units: &units,
        };
        let air_policy = air::AirPolicy::new(air_brain).unwrap();
        let ground_policy = tactical::TacticalPolicy::new(ground.clone()).unwrap();
        assert_eq!(
            ground_policy.command(&world, &units[0]).target,
            None,
            "ground checkpoint cannot command an aircraft"
        );
        for index in 0..2 {
            let expected = if index == 0 {
                air_policy.command(&world, &units[index])
            } else {
                ground_policy.command(&world, &units[index])
            };
            let actual = router.command(&world, &units[index]);
            assert_eq!(actual.target, expected.target);
            assert_eq!(actual.destination, expected.destination);
            assert_eq!(actual.throttle, expected.throttle);
        }
        let wrapper = tactical::TacticalPolicy::new(air_policy.brain().clone()).unwrap();
        let (_, explanation) = wrapper.explain(&world, &units[0]);
        let explanation = explanation.expect("air decisions expose named module observations");
        assert!(explanation.modules.iter().any(|m| m.module == "air_flight"));
        assert!(explanation.modules.iter().any(|m| {
            m.inputs
                .iter()
                .any(|(name, _)| name == "altitude_difference")
        }));
        assert_eq!(ground.to_json(), snapshot);
    }

    #[test]
    fn aa_uses_altitude_range_and_target_categories() {
        let mut plane = flight_fixture();
        plane.flight.as_mut().unwrap().cruise_altitude = 200.0;
        plane.strategic.target_categories = vec!["VTOL".into()];
        let mut aa = (*stepper_weapon(200.0)).clone();
        aa.weapons[0].only_target_categories = vec!["VTOL".into()];
        aa.weapons[0].range = 150.0;
        let make = |aa: crate::model::UnitDef| {
            Simulator::new(Scenario {
                terrain: Terrain::flat(1000.0, 1000.0, 32.0),
                max_time: 2.0,
                trace_resources: vec![],
                units: vec![
                    Spawn {
                        side: 0,
                        definition: aa,
                        position: Vec2::new(400.0, 500.0),
                        heading: 0.0,
                    },
                    Spawn {
                        side: 1,
                        definition: plane.clone(),
                        position: Vec2::new(500.0, 500.0),
                        heading: 0.0,
                    },
                ],
            })
        };
        let policies: [&dyn Policy; 2] = [
            &crate::policy::BuiltinPolicy::Hold,
            &crate::policy::BuiltinPolicy::Hold,
        ];
        assert_eq!(
            make(aa.clone()).run(policies, 1).sides[0].projectiles_fired,
            0,
            "horizontal range alone cannot authorize AA fire"
        );
        aa.weapons[0].range = 400.0;
        assert_eq!(make(aa.clone()).run(policies, 1).winner, Some(0));
        aa.weapons[0].only_target_categories = vec!["SURFACE".into()];
        assert_eq!(make(aa).run(policies, 1).sides[0].projectiles_fired, 0);
    }
    #[test]
    fn guided_collateral_air_hit_records_collision_altitude() {
        let plane = Arc::new(flight_fixture());
        let units = vec![new_unit_state(
            3,
            1,
            plane,
            0,
            Vec2::new(110.0, 50.0),
            0.0,
            100.0,
        )];
        let terrain = Terrain::flat(400.0, 400.0, 16.0);
        let mut spatial = SpatialGrid::new(&terrain, 64.0);
        spatial.rebuild(&units);
        let mut projectile = guided_test_projectile(0.0, 0.0, 30);
        projectile.impact.target_id = 99;
        projectile.height = 102.0;
        projectile.speed = 300.0;
        let mut projectiles = vec![projectile];
        let mut impacts = ImpactQueue::new(30);
        advance_guided_projectiles(
            &units,
            0,
            &mut projectiles,
            &mut impacts,
            &terrain,
            &spatial,
            &mut vec![],
        );
        assert_eq!(impacts.pending, 1);
        assert_eq!(impacts.nodes[0].impact.target_id, 3);
        assert_eq!(impacts.nodes[0].impact.altitude, 100.0);
    }

    #[test]
    fn ground_blast_on_cliff_does_not_hit_aircraft_below_cliff() {
        let mut shooter = (*stepper_weapon(200.0)).clone();
        shooter.weapons[0].area_of_effect = 150.0;
        shooter.weapons[0].edge_effectiveness = 1.0;
        shooter.weapons[0].only_target_categories = vec!["SURFACE".into()];
        let mut target = (*stepper_target()).clone();
        target.strategic.target_categories = vec!["SURFACE".into()];
        let mut plane = flight_fixture();
        plane.strategic.target_categories = vec!["VTOL".into()];
        let mut terrain = Terrain::flat(1000.0, 1000.0, 32.0);
        for row in 0..terrain.rows {
            for col in 0..13 {
                terrain.heights[row * terrain.cols + col] = 300.0;
            }
        }
        terrain.refresh_derived();
        let scenario = Scenario {
            terrain,
            max_time: 0.5,
            trace_resources: vec![],
            units: vec![
                Spawn {
                    side: 0,
                    definition: shooter,
                    position: Vec2::new(200.0, 500.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: target,
                    position: Vec2::new(400.0, 500.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: plane,
                    position: Vec2::new(440.0, 500.0),
                    heading: 0.0,
                },
            ],
        };
        let result = Simulator::new(scenario).run(
            [
                &crate::policy::BuiltinPolicy::Hold,
                &crate::policy::BuiltinPolicy::Hold,
            ],
            1,
        );
        assert_eq!(
            result.sides[0].damage_dealt, 100.0,
            "blast at elevation 300 misses aircraft at elevation 100"
        );
        assert_eq!(result.winner, None);
    }
}

#[cfg(test)]
mod auxiliary_tests {
    use super::*;
    use crate::learn::tactical::tests::skirmisher;
    use crate::policy::BuiltinPolicy;
    #[test]
    fn auxiliary_survivors_cannot_extend_pending_projectiles_with_new_shots() {
        let mut attacker = skirmisher("attacker");
        attacker.weapons[0].damage = 1000.0;
        attacker.weapons[0].range = 1000.0;
        attacker.weapons[0].reload = 0.1;
        attacker.weapons[0].hitscan = false;
        attacker.weapons[0].projectile_speed = 100.0;
        let mut victim = skirmisher("victim");
        victim.weapons.clear();
        victim.movement.max_speed = 0.0;
        let mut support = victim.clone();
        support.name = "support".into();
        support.health = 1000000.0;
        let scenario = Scenario {
            terrain: Terrain::flat(2000.0, 1000.0, 32.0),
            max_time: 8.0,
            trace_resources: vec![],
            units: vec![
                Spawn {
                    definition: attacker,
                    side: 0,
                    position: Vec2::new(100.0, 500.0),
                    heading: 0.0,
                },
                Spawn {
                    definition: victim,
                    side: 1,
                    position: Vec2::new(200.0, 500.0),
                    heading: PI,
                },
                Spawn {
                    definition: support,
                    side: 1,
                    position: Vec2::new(350.0, 550.0),
                    heading: PI,
                },
            ],
        };
        let result = Simulator::new(scenario)
            .with_auxiliary_units([3])
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 1);
        assert_eq!(result.winner, Some(0));
        assert!(result.duration < 4.0, "{result:?}");
        assert!(result.sides[0].projectiles_fired < 20);
    }

    #[test]
    fn auxiliary_units_have_real_costs_but_cannot_hold_elimination_hostage() {
        let mut attacker = skirmisher("attacker");
        attacker.weapons[0].damage = 10000.0;
        let mut victim = skirmisher("victim");
        victim.weapons.clear();
        let mut support = victim.clone();
        support.name = "radar".into();
        support.metal_cost = 25.0;
        support.energy_cost = 0.0;
        support.movement.max_speed = 0.0;
        let scenario = Scenario {
            terrain: Terrain::flat(3000.0, 1000.0, 32.0),
            max_time: 1.0,
            trace_resources: vec![],
            units: vec![
                Spawn {
                    definition: attacker,
                    side: 0,
                    position: Vec2::new(100.0, 500.0),
                    heading: 0.0,
                },
                Spawn {
                    definition: victim,
                    side: 1,
                    position: Vec2::new(180.0, 500.0),
                    heading: PI,
                },
                Spawn {
                    definition: support,
                    side: 1,
                    position: Vec2::new(2000.0, 500.0),
                    heading: PI,
                },
            ],
        };
        let simulator = Simulator::new(scenario).with_auxiliary_units([3]);
        let result = simulator.run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 1);
        assert_eq!(result.winner, Some(0), "{result:?}");
        assert!(!result.timed_out);
        assert!(result.duration < 1.0);
        assert_eq!(result.sides[1].surviving_value, 25.0);
        assert!(result.sides[0].won);
        assert!(!result.sides[1].won);
        let repeated =
            simulator.run_parallel_results([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 2, 1);
        assert!(
            repeated
                .iter()
                .all(|r| r.winner == Some(0) && r.sides[1].surviving_value == 25.0)
        );
    }
}
