//! Tactical training: evolve a [`Brain`] against baseline (or earlier-brain)
//! opponents over a declared set of scenarios.
//!
//! * Every scenario is built twice, with the learner on side 0 and on side 1,
//!   so an asymmetric roster (an underdog fight) is never confounded with a
//!   starting side.
//! * Within a generation all candidates see identical layouts and seeds
//!   (common random numbers), so differences between candidates are signal.
//! * Layouts are re-jittered each generation so the council cannot memorise
//!   coordinates; validation uses one fixed, un-jittered layout and a fixed
//!   seed range, and decides which checkpoint is "best".

use super::brain::{Brain, BrainError};
use super::es::{EsConfig, EvolutionStrategy, GenerationStats};
use super::matchups::{self, MatchupConfig};
use super::objective::{ObjectiveKind, ObjectiveWeights, Tally, assess_objective, robust_fitness};
use super::rng::Rng;
use super::tactical::{self, TacticalPolicy};
use crate::catalog::UnitCatalog;
use crate::engine::{Scenario, Simulator};
use crate::map::{CompiledMap, MapLoadOptions};
use crate::model::{Spawn, Terrain, UnitDef, Vec2};
use crate::policy::{Policy, parse_policy};
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const VALIDATION_SEED_BASE: u64 = 1_000_000_000;
const TRAIN_SEED_BASE: u64 = 1_000;
/// Offset keeping the progress evaluation off the seeds candidates trained on.
const PROGRESS_SEED_OFFSET: u64 = 500_000;
/// Seed of the fixed batch of generated matchups used for validation, for
/// `--explain` and for the viewer. Training draws a new batch every generation.
pub const MATCHUP_VALIDATION_SEED: u64 = 7_777_777;
const UNIT_GAP: f32 = 8.0;
const MAX_PER_COLUMN: usize = 6;
/// Space between two platoons of one side at the start.
const PLATOON_GAP: f32 = 140.0;
/// Distance from a side's last column to its objective.
const TARGET_DEPTH: f32 = 140.0;

#[derive(Debug)]
pub struct TrainError(pub String);

impl fmt::Display for TrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TrainError {}

impl From<BrainError> for TrainError {
    fn from(error: BrainError) -> Self {
        Self(error.to_string())
    }
}

impl From<String> for TrainError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub unit: String,
    #[serde(default = "one")]
    pub count: usize,
    /// Which starting group of its side the unit belongs to. Groups start
    /// apart from each other; the default keeps a side in one group.
    #[serde(default)]
    pub platoon: usize,
}

fn one() -> usize {
    1
}

/// A target one side owns and the other must destroy. The owner is the learner
/// for a defence and the enemy for an attack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveSpec {
    pub kind: ObjectiveKind,
    /// Catalog name of the immobile target, placed behind its owner's platoons.
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faction: Option<String>,
    pub name: String,
    pub learner: Vec<RosterEntry>,
    pub enemy: Vec<RosterEntry>,
    /// None: a plain fight to the finish.
    #[serde(default)]
    pub objective: Option<ObjectiveSpec>,
    /// Distance between the two formations' front lines at the start.
    #[serde(default = "default_separation")]
    pub separation: f32,
}

fn default_separation() -> f32 {
    300.0
}

/// Where training begins when no saved brain is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    /// The hand-authored council from [`tactical::template`]: it already
    /// closes to range, fights, retreats only when hopeless and spreads around
    /// the enemy, and training refines it.
    #[default]
    Template,
    /// Small random weights: training must discover fighting from nothing.
    Random,
}

/// Optional captured-map arena; a single movement mask is used for this curriculum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrainingArenaSpec {
    pub map: PathBuf,
    pub movement_class: String,
    #[serde(default = "default_map_cell")]
    pub cell_size: f32,
    #[serde(default)]
    pub center: Option<[f32; 2]>,
}

fn default_map_cell() -> f32 {
    32.0
}

struct PreparedArena {
    terrain: Terrain,
    center: Vec2,
    movement_class: String,
}

fn prepare_arena(config: &TrainConfig) -> Result<Option<PreparedArena>, TrainError> {
    let Some(spec) = &config.arena else {
        return Ok(None);
    };
    let (map, _) = CompiledMap::load_path(&spec.map, MapLoadOptions::default())
        .map_err(|error| TrainError(format!("arena {}: {error}", spec.map.display())))?;
    let terrain = map
        .to_tactical_terrain(&spec.movement_class, spec.cell_size)
        .map_err(|error| TrainError(format!("arena {}: {error}", spec.map.display())))?;
    let center = spec.center.unwrap_or([map.width * 0.5, map.depth * 0.5]);
    Ok(Some(PreparedArena {
        terrain,
        center: Vec2::new(center[0], center[1]),
        movement_class: spec.movement_class.clone(),
    }))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainConfig {
    pub name: String,
    /// Versioned tactical wiring. Existing configs retain v3.
    pub brain_kind: String,
    /// Frozen policy for units from the other movement domain in mixed fights.
    pub companion_brain: Option<PathBuf>,
    /// None keeps the legacy unlimited firing supply; Some bounds each side.
    pub resource_supply: Option<crate::model::StartingResources>,
    /// Generated records are carried into reports; users normally leave this empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matchup_records: Vec<matchups::MatchupRecord>,
    pub arena: Option<TrainingArenaSpec>,
    /// Hidden units per module when starting from scratch.
    pub hidden: usize,
    /// Continue from an earlier brain file instead of a fresh start.
    pub init_from: Option<PathBuf>,
    /// The fresh start used when `init_from` is not set.
    pub start: StartKind,
    /// Modules allowed to change; empty means all. Others stay frozen.
    pub train_modules: Vec<String>,
    pub generations: u64,
    pub es: EsConfig,
    pub objective: ObjectiveWeights,
    /// Unit definition names to flag as key units (for example a commander),
    /// on both sides. Pair with `objective.key_unit_bonus` / `key_unit_penalty`.
    pub key_units: Vec<String>,
    /// Shot-randomness seeds per (scenario, opponent, learner side).
    pub seeds_per_battle: usize,
    pub validation_seeds: usize,
    pub validation_every: u64,
    /// `charge`, `kite`, `orbit`, `hold`, or `brain:PATH` for an earlier brain.
    pub opponents: Vec<String>,
    pub max_time: f32,
    /// Random per-unit position offset (world units) re-drawn each generation.
    pub layout_jitter: f32,
    pub scenarios: Vec<ScenarioSpec>,
    /// Randomly generated platoon fights, added to `scenarios`. Training draws
    /// a fresh batch every generation; validation uses one fixed batch.
    pub matchups: Option<MatchupConfig>,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            name: "tactical".into(),
            brain_kind: tactical::KIND.into(),
            companion_brain: None,
            resource_supply: None,
            matchup_records: Vec::new(),
            arena: None,
            hidden: 6,
            init_from: None,
            start: StartKind::Template,
            train_modules: Vec::new(),
            generations: 100,
            es: EsConfig::default(),
            objective: ObjectiveWeights::default(),
            key_units: Vec::new(),
            seeds_per_battle: 4,
            validation_seeds: 24,
            validation_every: 10,
            opponents: vec!["charge".into(), "kite".into(), "orbit".into()],
            max_time: 60.0,
            layout_jitter: 30.0,
            scenarios: Vec::new(),
            matchups: None,
        }
    }
}

impl TrainConfig {
    /// Even fights, an outnumbered fight, and an advantaged one, all with one
    /// unit type: a sensible first curriculum.
    pub fn default_for_unit(unit: &str) -> Self {
        let roster = |count| {
            vec![RosterEntry {
                unit: unit.to_owned(),
                count,
                platoon: 0,
            }]
        };
        let scenario = |name: &str, ours, theirs| ScenarioSpec {
            faction: None,
            name: name.into(),
            learner: roster(ours),
            enemy: roster(theirs),
            objective: None,
            separation: default_separation(),
        };
        Self {
            scenarios: vec![
                scenario("1v1", 1, 1),
                scenario("3v3", 3, 3),
                scenario("3v5 underdog", 3, 5),
                scenario("5v3 favourite", 5, 3),
            ],
            ..Self::default()
        }
    }

    pub fn load(path: &Path) -> Result<Self, TrainError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| TrainError(format!("{}: {e}", path.display())))?;
        let mut config: Self = serde_json::from_str(&text)
            .map_err(|e| TrainError(format!("{}: {e}", path.display())))?;
        let directory = std::env::current_dir()
            .map_err(|e| TrainError(e.to_string()))?
            .join(path.parent().unwrap_or_else(|| Path::new(".")));
        let resolve = |value: &mut PathBuf| {
            if value.is_relative() {
                let local = directory.join(&*value);
                // Older configs use paths relative to the working directory.
                // Prefer an existing config-local file, then preserve that legacy path.
                *value = if !local.exists() && value.exists() {
                    std::env::current_dir()
                        .expect("working directory was resolved")
                        .join(&*value)
                } else {
                    local
                };
            }
        };
        if let Some(arena) = &mut config.arena {
            resolve(&mut arena.map);
        }
        if let Some(initial) = &mut config.init_from {
            resolve(initial);
        }
        if let Some(companion) = &mut config.companion_brain {
            resolve(companion);
        }
        for opponent in &mut config.opponents {
            if let Some(brain) = opponent.strip_prefix("brain:") {
                let mut file = PathBuf::from(brain);
                resolve(&mut file);
                *opponent = format!("brain:{}", file.display());
            }
        }
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), TrainError> {
        if self.resource_supply.is_some_and(|s| {
            !s.metal.is_finite() || !s.energy.is_finite() || s.metal < 0.0 || s.energy < 0.0
        }) {
            return Err(TrainError(
                "resource_supply needs finite nonnegative metal and energy".into(),
            ));
        }
        self.es.validate()?;
        tactical::specs_for_kind(&self.brain_kind, self.hidden)?;
        if let Some(arena) = &self.arena {
            if arena.movement_class.is_empty()
                || !arena.cell_size.is_finite()
                || arena.cell_size <= 0.0
                || arena
                    .center
                    .is_some_and(|center| center.iter().any(|coordinate| !coordinate.is_finite()))
            {
                return Err(TrainError(
                    "arena needs a movement class, positive cell size and finite center".into(),
                ));
            }
        }
        self.objective.validate()?;
        if self.scenarios.is_empty() && self.matchups.is_none() {
            return Err(TrainError(
                "training needs at least one scenario or a matchups section".into(),
            ));
        }
        if self.opponents.is_empty() {
            return Err(TrainError("training needs at least one opponent".into()));
        }
        if self.start == StartKind::Template && self.init_from.is_none() && self.hidden < 2 {
            return Err(TrainError(
                "the template council needs at least 2 hidden units per module".into(),
            ));
        }
        if self.hidden == 0 || self.seeds_per_battle == 0 || self.validation_seeds == 0 {
            return Err(TrainError(
                "hidden, seeds_per_battle and validation_seeds must be non-zero".into(),
            ));
        }
        if !(self.max_time.is_finite() && self.max_time > 0.0)
            || !(self.layout_jitter.is_finite() && self.layout_jitter >= 0.0)
        {
            return Err(TrainError(
                "max_time must be positive and jitter non-negative".into(),
            ));
        }
        for scenario in &self.scenarios {
            for side in [&scenario.learner, &scenario.enemy] {
                if side.is_empty() || side.iter().any(|entry| entry.count == 0) {
                    return Err(TrainError(format!(
                        "scenario {:?}: each side needs at least one unit",
                        scenario.name
                    )));
                }
            }
            if !(scenario.separation.is_finite() && scenario.separation > 0.0) {
                return Err(TrainError(format!(
                    "scenario {:?}: separation must be positive",
                    scenario.name
                )));
            }
        }
        Ok(())
    }

    /// The config with its generated matchups for `seed` appended to the fixed
    /// scenarios, so everything downstream deals with one concrete list.
    pub fn resolved(&self, catalog: &UnitCatalog, seed: u64) -> Result<Self, TrainError> {
        let mut resolved = self.clone();
        if let Some(matchups) = resolved.matchups.take() {
            let mut records = matchups::generate_records(&matchups, catalog, seed)?;
            for record in &mut records {
                record.resource_supply = self.resource_supply;
            }
            resolved
                .scenarios
                .extend(records.iter().map(|r| r.scenario.clone()));
            resolved.matchup_records.extend(records);
        }
        Ok(resolved)
    }

    /// The concrete config validation, `--explain` and the viewer use.
    pub fn validation_view(&self, catalog: &UnitCatalog) -> Result<Self, TrainError> {
        self.resolved(catalog, MATCHUP_VALIDATION_SEED)
    }
}

// --- Arena ---------------------------------------------------------------------

struct ArenaEntry {
    scenario: usize,
    learner_side: usize,
    simulator: Simulator,
}

/// Opponent with the label used in reports.
pub struct Opponent {
    pub label: String,
    pub policy: Box<dyn Policy>,
}

fn load_opponents(specs: &[String]) -> Result<Vec<Opponent>, TrainError> {
    specs
        .iter()
        .map(|spec| {
            parse_policy(spec)
                .map(|policy| Opponent {
                    label: spec.clone(),
                    policy,
                })
                .map_err(TrainError)
        })
        .collect()
}

fn lookup<'a>(catalog: &'a UnitCatalog, name: &str) -> Result<&'a UnitDef, TrainError> {
    catalog
        .get(name)
        .ok_or_else(|| TrainError(format!("unit {name:?} is not in the catalog")))
}

/// Column formation facing `heading`, front rank at `front_x`. Each platoon of
/// the roster is its own block of columns; blocks sit side by side across the
/// front, apart from each other, centred on the arena's middle.
fn formation(
    roster: &[RosterEntry],
    catalog: &UnitCatalog,
    side: u8,
    front_x: f32,
    towards_positive_x: bool,
    jitter: f32,
    key_units: &[String],
    target: Option<&str>,
    rng: &mut Rng,
) -> Result<Vec<Spawn>, TrainError> {
    let platoons = roster.iter().map(|e| e.platoon).max().map_or(0, |m| m + 1);
    let mut groups: Vec<Vec<&UnitDef>> = vec![Vec::new(); platoons];
    for entry in roster {
        let definition = lookup(catalog, &entry.unit)?;
        groups[entry.platoon].extend(std::iter::repeat_n(definition, entry.count));
    }
    groups.retain(|g| !g.is_empty());
    let pitch_of = |group: &[&UnitDef]| {
        group
            .iter()
            .map(|d| d.movement.radius)
            .fold(1.0f32, f32::max)
            * 2.0
            + UNIT_GAP
    };
    let width_of = |group: &[&UnitDef]| group.len().min(MAX_PER_COLUMN) as f32 * pitch_of(group);
    let total_width: f32 =
        groups.iter().map(|g| width_of(g)).sum::<f32>() + PLATOON_GAP * (groups.len() as f32 - 1.0);
    let backwards = if towards_positive_x { -1.0 } else { 1.0 };
    let mut spawns = Vec::new();
    let mut edge = 500.0 - total_width * 0.5;
    for group in &groups {
        let pitch = pitch_of(group);
        let centre = edge + width_of(group) * 0.5;
        for (index, definition) in group.iter().enumerate() {
            let column = index / MAX_PER_COLUMN;
            let in_column = (group.len() - column * MAX_PER_COLUMN).min(MAX_PER_COLUMN);
            let row = index % MAX_PER_COLUMN;
            let z = centre + (row as f32 - (in_column as f32 - 1.0) * 0.5) * pitch;
            let x = front_x + backwards * column as f32 * pitch;
            spawns.push(Spawn {
                side,
                definition: {
                    let mut definition = (*definition).clone();
                    definition.strategic.key_unit |= key_units.contains(&definition.name);
                    definition
                },
                position: Vec2::new(x + rng.signed() * jitter, z + rng.signed() * jitter),
                heading: if towards_positive_x { 0.0 } else { PI },
            });
        }
        edge += width_of(group) + PLATOON_GAP;
    }
    if let Some(name) = target {
        // The objective sits behind the deepest column of its owner's platoons.
        let mut definition = lookup(catalog, name)?.clone();
        definition.strategic.key_unit = true;
        let depth = groups
            .iter()
            .map(|g| g.len().div_ceil(MAX_PER_COLUMN) as f32 * pitch_of(g))
            .fold(0.0f32, f32::max);
        spawns.push(Spawn {
            side,
            position: Vec2::new(front_x + backwards * (depth + TARGET_DEPTH), 500.0),
            heading: if towards_positive_x { 0.0 } else { PI },
            definition,
        });
    }
    Ok(spawns)
}

/// One scenario with the learner on `learner_side`. `layout_seed` and `jitter`
/// control the per-unit position offsets.
fn build_simulator(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    index: usize,
    learner_side: usize,
    layout_seed: u64,
    jitter: f32,
) -> Result<Simulator, TrainError> {
    let prepared = prepare_arena(config)?;
    build_simulator_prepared(
        config,
        catalog,
        index,
        learner_side,
        layout_seed,
        jitter,
        prepared.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_simulator_prepared(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    index: usize,
    learner_side: usize,
    layout_seed: u64,
    jitter: f32,
    prepared: Option<&PreparedArena>,
) -> Result<Simulator, TrainError> {
    let spec = &config.scenarios[index];
    let mut rng = Rng::for_stream(layout_seed, (index as u64) << 1 | learner_side as u64);
    let width = (spec.separation + 1_000.0).max(2_000.0);
    let centre = width * 0.5;
    let (left_roster, right_roster) = if learner_side == 0 {
        (&spec.learner, &spec.enemy)
    } else {
        (&spec.enemy, &spec.learner)
    };
    // Who owns the objective target: the learner defends its own; an attack is
    // on the enemy's.
    let owner_side = spec.objective.as_ref().map(|o| match o.kind {
        ObjectiveKind::Defend => learner_side,
        ObjectiveKind::Attack => 1 - learner_side,
    });
    let target_for = |side: usize| {
        spec.objective
            .as_ref()
            .filter(|_| owner_side == Some(side))
            .map(|o| o.target.as_str())
    };
    let mut units = formation(
        left_roster,
        catalog,
        0,
        centre - spec.separation * 0.5,
        true,
        jitter,
        &config.key_units,
        target_for(0),
        &mut rng,
    )?;
    units.extend(formation(
        right_roster,
        catalog,
        1,
        centre + spec.separation * 0.5,
        false,
        jitter,
        &config.key_units,
        target_for(1),
        &mut rng,
    )?);
    let terrain = if let Some(arena) = prepared {
        let translation = arena.center - Vec2::new(centre, 500.0);
        for spawn in &mut units {
            spawn.position = spawn.position + translation;
            if spawn.definition.flight.is_none()
                && spawn.definition.movement.max_speed > 0.0
                && spawn
                    .definition
                    .strategic
                    .movement_class
                    .as_deref()
                    .is_some_and(|class| class != arena.movement_class)
            {
                return Err(TrainError(format!(
                    "scenario {:?}: unit {} movement class differs from arena {}",
                    spec.name, spawn.definition.name, arena.movement_class
                )));
            }
            if spawn.definition.flight.is_none()
                && !arena
                    .terrain
                    .is_passable_with_radius(spawn.position, spawn.definition.movement.radius)
            {
                return Err(TrainError(format!(
                    "scenario {:?}: blocked or out-of-bounds spawn {} at ({}, {})",
                    spec.name, spawn.definition.name, spawn.position.x, spawn.position.z
                )));
            }
        }
        arena.terrain.clone()
    } else {
        Terrain::flat(width, 1_000.0, 32.0)
    };
    let scenario = Scenario {
        terrain,
        units,
        max_time: config.max_time,
        trace_resources: Vec::new(),
    };
    Simulator::try_new(scenario)
        .map(|sim| {
            if let Some(supply) = config.resource_supply {
                sim.with_starting_resources([supply; 2])
            } else {
                sim
            }
        })
        .map_err(|missing| {
            TrainError(format!(
                "scenario {:?} needs unsupported mechanics: {missing:?}",
                spec.name
            ))
        })
}

fn build_arena(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    layout_seed: u64,
    jitter: f32,
) -> Result<Vec<ArenaEntry>, TrainError> {
    let prepared = prepare_arena(config)?;
    build_arena_prepared(config, catalog, layout_seed, jitter, prepared.as_ref())
}

fn build_arena_prepared(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    layout_seed: u64,
    jitter: f32,
    prepared: Option<&PreparedArena>,
) -> Result<Vec<ArenaEntry>, TrainError> {
    let mut arena = Vec::new();
    for index in 0..config.scenarios.len() {
        for learner_side in 0..2usize {
            arena.push(ArenaEntry {
                scenario: index,
                learner_side,
                simulator: build_simulator_prepared(
                    config,
                    catalog,
                    index,
                    learner_side,
                    layout_seed,
                    jitter,
                    prepared,
                )?,
            });
        }
    }
    Ok(arena)
}

/// The simulator for one scenario and side, laid out with `layout_seed`'s
/// jitter, for tools that replay a single battle.
pub fn watch_simulator(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    scenario: usize,
    learner_side: usize,
    layout_seed: u64,
) -> Result<Simulator, TrainError> {
    if scenario >= config.scenarios.len() || learner_side > 1 {
        return Err(TrainError(format!(
            "no scenario {scenario} / side {learner_side} (config has {} scenarios)",
            config.scenarios.len()
        )));
    }
    build_simulator(
        config,
        catalog,
        scenario,
        learner_side,
        layout_seed,
        config.layout_jitter,
    )
}

// --- Evaluation ----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvaluationRow {
    pub scenario: String,
    pub opponent: String,
    pub tally: Tally,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    pub total: Tally,
    pub rows: Vec<EvaluationRow>,
    /// The score training maximises and checkpoints are chosen on: the mean
    /// over (scenario, opponent) rows blended with their worst quarter. See
    /// [`ObjectiveWeights::worst_case_weight`].
    pub robust_fitness: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matchup_records: Vec<matchups::MatchupRecord>,
    #[serde(default)]
    pub families: std::collections::BTreeMap<String, f64>,
}

pub(crate) fn family_fitness(
    rows: &[EvaluationRow],
    records: &[matchups::MatchupRecord],
    weights: &ObjectiveWeights,
) -> std::collections::BTreeMap<String, f64> {
    let mut names: std::collections::BTreeMap<String, std::collections::BTreeSet<&str>> =
        Default::default();
    for record in records {
        names
            .entry(record.family.clone())
            .or_default()
            .insert(record.scenario.name.as_str());
    }
    names
        .into_iter()
        .map(|(family, names)| {
            let fitness = robust_fitness(
                rows.iter()
                    .filter(|r| names.contains(r.scenario.as_str()))
                    .map(|r| &r.tally),
                weights,
            );
            (family, fitness)
        })
        .collect()
}

/// Runs the learner against every opponent in every arena entry.
pub fn evaluate(
    config: &TrainConfig,
    arena_entries: &[(usize, usize, &Simulator)],
    learner: &dyn Policy,
    opponents: &[Opponent],
    seeds: usize,
    first_seed: u64,
) -> Evaluation {
    let mut rows: Vec<EvaluationRow> = config
        .scenarios
        .iter()
        .flat_map(|scenario| {
            opponents.iter().map(|opponent| EvaluationRow {
                scenario: scenario.name.clone(),
                opponent: opponent.label.clone(),
                tally: Tally::default(),
            })
        })
        .collect();
    let mut total = Tally::default();
    for &(scenario, learner_side, simulator) in arena_entries {
        for (opponent_index, opponent) in opponents.iter().enumerate() {
            let policies: [&dyn Policy; 2] = if learner_side == 0 {
                [learner, opponent.policy.as_ref()]
            } else {
                [opponent.policy.as_ref(), learner]
            };
            let row = &mut rows[scenario * opponents.len() + opponent_index];
            for result in simulator.run_parallel_results(policies, seeds, first_seed) {
                let objective = config.scenarios[scenario]
                    .objective
                    .as_ref()
                    .map(|o| o.kind);
                let assessment =
                    assess_objective(&result, learner_side, &config.objective, objective);
                row.tally.add(&assessment);
                total.add(&assessment);
            }
        }
    }
    let robust_fitness = robust_fitness(rows.iter().map(|row| &row.tally), &config.objective);
    Evaluation {
        families: family_fitness(&rows, &config.matchup_records, &config.objective),
        total,
        rows,
        robust_fitness,
        matchup_records: config
            .matchup_records
            .iter()
            .flat_map(|record| {
                opponents.iter().flat_map(move |opponent| {
                    (0..2).map(move |side| {
                        let mut record = record.clone();
                        record.opponent_identity = opponent.label.clone();
                        record.learner_side = side;
                        record
                    })
                })
            })
            .collect(),
    }
}

fn evaluate_arena(
    config: &TrainConfig,
    arena: &[ArenaEntry],
    learner: &dyn Policy,
    opponents: &[Opponent],
    seeds: usize,
    first_seed: u64,
) -> Evaluation {
    let entries: Vec<_> = arena
        .iter()
        .map(|entry| (entry.scenario, entry.learner_side, &entry.simulator))
        .collect();
    evaluate(config, &entries, learner, opponents, seeds, first_seed)
}

/// Fixed-layout, fixed-seed evaluation used for validation and for explaining
/// a finished brain.
pub fn evaluate_validation(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    learner: &dyn Policy,
    opponents: &[Opponent],
) -> Result<Evaluation, TrainError> {
    let config = &config.validation_view(catalog)?;
    let arena = build_arena(config, catalog, 0, 0.0)?;
    Ok(evaluate_arena(
        config,
        &arena,
        learner,
        opponents,
        config.validation_seeds,
        VALIDATION_SEED_BASE,
    ))
}

/// Evaluation on a layout and seed range of the caller's choosing. The layout
/// jitter is drawn from `first_seed`, so each seed base is a fresh sample;
/// two brains evaluated with the same arguments face identical battles.
pub fn evaluate_at(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    learner: &dyn Policy,
    opponents: &[Opponent],
    seeds: usize,
    first_seed: u64,
) -> Result<Evaluation, TrainError> {
    evaluate_on_layout(
        config, catalog, learner, opponents, seeds, first_seed, first_seed,
    )
}
pub fn evaluate_on_layout(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    learner: &dyn Policy,
    opponents: &[Opponent],
    seeds: usize,
    first_seed: u64,
    layout_seed: u64,
) -> Result<Evaluation, TrainError> {
    let config = &config.resolved(catalog, layout_seed)?;
    let arena = build_arena(config, catalog, layout_seed, config.layout_jitter)?;
    Ok(evaluate_arena(
        config, &arena, learner, opponents, seeds, first_seed,
    ))
}

pub fn opponents_from_config(config: &TrainConfig) -> Result<Vec<Opponent>, TrainError> {
    let Some(path) = &config.companion_brain else {
        return load_opponents(&config.opponents);
    };
    let companion = Brain::load(path)?;
    config
        .opponents
        .iter()
        .map(|spec| {
            if let Some(path) = spec.strip_prefix("brain:") {
                let brain = Brain::load(Path::new(path))?;
                let label = format!(
                    "brain:{}",
                    super::content_hash::sha256(brain.to_json().as_bytes())
                );
                Ok(Opponent {
                    label,
                    policy: domain_policy(brain, Some(companion.clone()))?,
                })
            } else {
                Ok(Opponent {
                    label: spec.clone(),
                    policy: parse_policy(spec).map_err(TrainError)?,
                })
            }
        })
        .collect()
}

/// Builds the simulators for a config's scenarios, for tools that need to
/// watch the council act (such as `explain`).
pub fn validation_simulators(
    config: &TrainConfig,
    catalog: &UnitCatalog,
) -> Result<Vec<(usize, usize, Simulator)>, TrainError> {
    Ok(
        build_arena(&config.validation_view(catalog)?, catalog, 0, 0.0)?
            .into_iter()
            .map(|entry| (entry.scenario, entry.learner_side, entry.simulator))
            .collect(),
    )
}

// --- Training loop ----------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub generation: u64,
    pub es: GenerationStats,
    /// The updated mean parameters on seeds no candidate trained on.
    pub center: Tally,
    pub validation: Option<Tally>,
    pub new_best: bool,
    /// Battles simulated for this generation (population plus the progress
    /// check; validation excluded), the simulated time they covered, and the
    /// wall-clock time the generation took.
    #[serde(default)]
    pub battles: u64,
    #[serde(default)]
    pub sim_seconds: f64,
    #[serde(default)]
    pub wall_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrainSummary {
    pub name: String,
    pub parameters_trained: usize,
    pub initial: Evaluation,
    pub best_generation: u64,
    pub best: Evaluation,
    pub last: Evaluation,
    pub output_dir: Option<PathBuf>,
}

fn initial_brain(config: &TrainConfig) -> Result<Brain, TrainError> {
    match &config.init_from {
        Some(path) => {
            let brain = Brain::load(path)?;
            brain.check_against(
                &config.brain_kind,
                &tactical::specs_for_kind(&config.brain_kind, 1)?,
            )?;
            Ok(brain)
        }
        None => match config.start {
            StartKind::Template => Ok(tactical::template_for_kind(
                &config.brain_kind,
                config.hidden,
            )?),
            StartKind::Random => Ok(Brain::random(
                &config.brain_kind,
                tactical::specs_for_kind(&config.brain_kind, config.hidden)?,
                &mut Rng::new(config.es.seed ^ 0xB1A1_B1A1),
                0.5,
            )?),
        },
    }
}

/// Construct a domain-specific policy, optionally with a frozen companion.
pub fn policy_for_brain(config: &TrainConfig, brain: Brain) -> Result<Box<dyn Policy>, TrainError> {
    if brain.kind != config.brain_kind {
        return Err(TrainError(
            "candidate brain kind differs from training config".into(),
        ));
    }
    let companion = config
        .companion_brain
        .as_ref()
        .map(|p| Brain::load(p))
        .transpose()?;
    domain_policy(brain, companion)
}

pub(crate) fn domain_policy(
    brain: Brain,
    companion: Option<Brain>,
) -> Result<Box<dyn Policy>, TrainError> {
    match companion {
        Some(companion) => Ok(Box::new(super::air::DomainPolicy::new(brain, companion)?)),
        None => Ok(Box::new(TacticalPolicy::new(brain)?)),
    }
}

fn policy_for(
    base: &Brain,
    mask: &[bool],
    theta: &[f32],
    companion: Option<&Brain>,
) -> Box<dyn Policy> {
    let brain = base
        .with_flat(mask, theta)
        .expect("optimizer keeps the parameter count fixed");
    domain_policy(brain, companion.cloned()).expect("derived from validated domain brains")
}

pub fn train(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    output_dir: Option<&Path>,
    progress: &mut dyn FnMut(&Progress),
) -> Result<TrainSummary, TrainError> {
    config.validate()?;
    // Load once before evaluating populations; a companion is frozen for the run.
    let companion = config
        .companion_brain
        .as_ref()
        .map(|p| Brain::load(p))
        .transpose()?;
    if let Some(other) = &companion {
        let base = initial_brain(config)?;
        super::air::DomainPolicy::new(base, other.clone())?;
    }
    let opponents = opponents_from_config(config)?;
    let base = initial_brain(config)?;
    let mask = base.trainable_mask(&config.train_modules)?;
    let mut es = EvolutionStrategy::new(base.flatten(&mask), config.es)?;
    let validation_config = config.validation_view(catalog)?;
    let prepared = prepare_arena(config)?;
    let validation_arena =
        build_arena_prepared(&validation_config, catalog, 0, 0.0, prepared.as_ref())?;

    if let Some(dir) = output_dir {
        std::fs::create_dir_all(dir).map_err(|e| TrainError(format!("{}: {e}", dir.display())))?;
        let resolved = serde_json::to_string_pretty(config).expect("config serializes");
        write(dir, "train_config.json", &resolved)?;
    }

    let validate = |theta: &[f32]| {
        evaluate_arena(
            &validation_config,
            &validation_arena,
            policy_for(&base, &mask, theta, companion.as_ref()).as_ref(),
            &opponents,
            config.validation_seeds,
            VALIDATION_SEED_BASE,
        )
    };

    let initial = validate(es.theta());
    let mut best = initial.clone();
    let mut best_generation = 0;
    let mut last = initial.clone();
    save_checkpoint(output_dir, "brain.initial.json", &base, &mask, es.theta())?;
    save_checkpoint(output_dir, "brain.best.json", &base, &mask, es.theta())?;

    let mut log = String::new();
    for _ in 0..config.generations {
        let generation = es.generation();
        let layout_seed = config.es.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ generation;
        let generation_config = config.resolved(catalog, layout_seed)?;
        let arena = build_arena_prepared(
            &generation_config,
            catalog,
            layout_seed,
            config.layout_jitter,
            prepared.as_ref(),
        )?;
        let seed0 = TRAIN_SEED_BASE + generation * 1_000;

        let started = Instant::now();
        let battles = AtomicU64::new(0);
        let sim_millis = AtomicU64::new(0);
        let count = |total: &Tally| {
            battles.fetch_add(total.battles as u64, Ordering::Relaxed);
            sim_millis.fetch_add((total.duration_sum * 1000.0) as u64, Ordering::Relaxed);
        };

        let stats = es.step_masked(&base.parameter_activity(&mask), |theta| {
            let evaluation = evaluate_arena(
                &generation_config,
                &arena,
                policy_for(&base, &mask, theta, companion.as_ref()).as_ref(),
                &opponents,
                config.seeds_per_battle,
                seed0,
            );
            count(&evaluation.total);
            evaluation.robust_fitness
        })?;

        let center = evaluate_arena(
            &generation_config,
            &arena,
            policy_for(&base, &mask, es.theta(), companion.as_ref()).as_ref(),
            &opponents,
            config.seeds_per_battle,
            seed0 + PROGRESS_SEED_OFFSET,
        )
        .total;
        count(&center);
        let wall_seconds = started.elapsed().as_secs_f64();

        let finished = es.generation();
        let is_last = finished == config.generations;
        let mut validation_tally = None;
        let mut new_best = false;
        if finished % config.validation_every.max(1) == 0 || is_last {
            let evaluation = validate(es.theta());
            validation_tally = Some(evaluation.total);
            if evaluation.robust_fitness > best.robust_fitness {
                best = evaluation.clone();
                best_generation = finished;
                new_best = true;
                save_checkpoint(output_dir, "brain.best.json", &base, &mask, es.theta())?;
            }
            last = evaluation;
            save_checkpoint(output_dir, "brain.latest.json", &base, &mask, es.theta())?;
        }

        let report = Progress {
            generation: finished,
            es: stats,
            center,
            validation: validation_tally,
            new_best,
            battles: battles.load(Ordering::Relaxed),
            sim_seconds: sim_millis.load(Ordering::Relaxed) as f64 / 1000.0,
            wall_seconds,
        };
        log.push_str(&serde_json::to_string(&report).expect("progress serializes"));
        log.push('\n');
        progress(&report);
    }

    let summary = TrainSummary {
        name: config.name.clone(),
        parameters_trained: es.theta().len(),
        initial,
        best_generation,
        best,
        last,
        output_dir: output_dir.map(Path::to_path_buf),
    };
    if let Some(dir) = output_dir {
        write(dir, "training_log.jsonl", &log)?;
        let text = serde_json::to_string_pretty(&summary).expect("summary serializes");
        write(dir, "summary.json", &text)?;
    }
    Ok(summary)
}

fn write(dir: &Path, name: &str, text: &str) -> Result<(), TrainError> {
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(|e| TrainError(format!("{}: {e}", path.display())))
}

fn save_checkpoint(
    dir: Option<&Path>,
    name: &str,
    base: &Brain,
    mask: &[bool],
    theta: &[f32],
) -> Result<(), TrainError> {
    if let Some(dir) = dir {
        let brain = base.with_flat(mask, theta)?;
        brain.save(&dir.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::tactical::tests::skirmisher;

    fn catalog() -> UnitCatalog {
        let mut catalog = UnitCatalog::default();
        catalog.units.insert("skirm".into(), skirmisher("skirm"));
        catalog
    }

    fn tiny_config() -> TrainConfig {
        TrainConfig {
            generations: 3,
            es: EsConfig {
                population: 8,
                ..Default::default()
            },
            seeds_per_battle: 2,
            validation_seeds: 3,
            validation_every: 2,
            opponents: vec!["hold".into(), "charge".into()],
            max_time: 20.0,
            ..TrainConfig::default_for_unit("skirm")
        }
    }

    #[test]
    fn arena_builds_each_scenario_with_the_learner_on_both_sides() {
        let config = tiny_config();
        let arena = build_arena(&config, &catalog(), 1, 10.0).unwrap();
        assert_eq!(arena.len(), config.scenarios.len() * 2);
        assert_eq!(arena[0].learner_side, 0);
        assert_eq!(arena[1].learner_side, 1);
    }

    #[test]
    fn the_target_belongs_to_the_defender_and_is_a_key_unit() {
        let mut catalog = catalog();
        let mut tower = crate::learn::tactical::tests::skirmisher("tower");
        tower.movement.max_speed = 0.0;
        catalog.units.insert("tower".into(), tower);
        for (kind, learner_owns) in [
            (ObjectiveKind::Defend, true),
            (ObjectiveKind::Attack, false),
        ] {
            let mut config = tiny_config();
            config.scenarios.truncate(1);
            config.scenarios[0].objective = Some(ObjectiveSpec {
                kind,
                target: "tower".into(),
            });
            for learner_side in 0..2usize {
                let simulator =
                    build_simulator(&config, &catalog, 0, learner_side, 1, 0.0).unwrap();
                let result = simulator.run(
                    [
                        &crate::policy::BuiltinPolicy::Hold,
                        &crate::policy::BuiltinPolicy::Hold,
                    ],
                    1,
                );
                let owner = if learner_owns {
                    learner_side
                } else {
                    1 - learner_side
                };
                assert_eq!(
                    result.sides[owner].key_units_started, 1,
                    "{kind:?} side {learner_side}"
                );
                assert_eq!(result.sides[1 - owner].key_units_started, 0);
            }
        }
    }

    #[test]
    fn charging_attackers_win_an_attack_by_destroying_the_target() {
        let mut catalog = catalog();
        let mut tower = crate::learn::tactical::tests::skirmisher("tower");
        tower.movement.max_speed = 0.0;
        tower.weapons.clear();
        tower.health = 100.0;
        catalog.units.insert("tower".into(), tower);
        let mut config = tiny_config();
        config.max_time = 60.0;
        config.scenarios.truncate(1);
        config.scenarios[0].learner[0].count = 4;
        config.scenarios[0].enemy[0].count = 1;
        config.scenarios[0].objective = Some(ObjectiveSpec {
            kind: ObjectiveKind::Attack,
            target: "tower".into(),
        });
        let opponents = load_opponents(&["hold".to_owned()]).unwrap();
        let evaluation = evaluate_validation(
            &config,
            &catalog,
            &crate::policy::BuiltinPolicy::Charge,
            &opponents,
        )
        .unwrap();
        assert_eq!(
            evaluation.total.wins, evaluation.total.battles,
            "{:?}",
            evaluation.total
        );
    }

    #[test]
    fn underdog_scenarios_are_reported_as_such() {
        // 3v5 at equal unit cost: the learner is the underdog on both sides.
        let config = tiny_config();
        let evaluation = evaluate_validation(
            &config,
            &catalog(),
            &crate::policy::BuiltinPolicy::Hold,
            &load_opponents(&["hold".to_owned()]).unwrap(),
        )
        .unwrap();
        let row = |name: &str| evaluation.rows.iter().find(|r| r.scenario == name).unwrap();
        assert!(row("3v5 underdog").tally.underdog_battles > 0);
        assert_eq!(row("3v3").tally.underdog_battles, 0);
        assert_eq!(row("5v3 favourite").tally.underdog_battles, 0);
    }

    #[test]
    fn evaluation_at_a_seed_base_is_repeatable_and_fresh_per_base() {
        let config = tiny_config();
        let opponents = load_opponents(&["charge".to_owned()]).unwrap();
        let policy = crate::policy::BuiltinPolicy::Kite;
        let run = |base| evaluate_at(&config, &catalog(), &policy, &opponents, 3, base).unwrap();
        assert_eq!(run(5).total, run(5).total);
        // Jittered layouts differ between bases, so some statistic moves.
        assert!((0..4).any(|k| run(5).total != run(100 + k).total));
    }

    #[test]
    fn key_units_flow_from_the_config_through_the_engine_into_fitness() {
        let opponents = load_opponents(&["hold".to_owned()]).unwrap();
        let charge = crate::policy::BuiltinPolicy::Charge;
        let score = |key_units: Vec<String>| {
            let config = TrainConfig {
                key_units,
                // Bonus only: with equal weights and mutual kills the terms
                // cancel exactly, which is the intended behaviour.
                objective: ObjectiveWeights {
                    key_unit_penalty: 0.0,
                    ..ObjectiveWeights::default()
                },
                ..tiny_config()
            };
            evaluate_validation(&config, &catalog(), &charge, &opponents)
                .unwrap()
                .total
        };
        // Charging a stationary opponent kills it, so with every unit flagged
        // as a key unit the learner earns the bonus; with none flagged, nothing.
        let flagged = score(vec!["skirm".into()]);
        assert!(flagged.key_term_sum > 0.0, "{flagged:?}");
        assert_eq!(score(vec![]).key_term_sum, 0.0);
    }

    #[test]
    fn the_authored_council_wins_every_favourite_fight_without_any_training() {
        // The complaint this guards against: five units running from three.
        let config = TrainConfig {
            opponents: vec![
                "charge".into(),
                "kite".into(),
                "orbit".into(),
                "hold".into(),
            ],
            max_time: 90.0,
            validation_seeds: 6,
            ..TrainConfig::default_for_unit("skirm")
        };
        let policy = TacticalPolicy::new(tactical::template(6).unwrap()).unwrap();
        let opponents = opponents_from_config(&config).unwrap();
        let evaluation = evaluate_validation(&config, &catalog(), &policy, &opponents).unwrap();
        for row in evaluation
            .rows
            .iter()
            .filter(|r| r.scenario.starts_with("5v3"))
        {
            let t = &row.tally;
            assert!(
                t.wins == t.battles,
                "5v3 vs {}: {} wins, {} timeouts, {} losses of {}",
                row.opponent,
                t.wins,
                t.timeouts,
                t.losses,
                t.battles
            );
        }
    }

    #[test]
    fn unknown_units_and_empty_scenarios_fail_cleanly() {
        let mut config = tiny_config();
        config.scenarios[0].learner[0].unit = "nope".into();
        assert!(build_arena(&config, &catalog(), 0, 0.0).is_err());
        config.scenarios.clear();
        assert!(config.validate().is_err());
    }

    #[test]
    fn training_is_deterministic_and_writes_self_describing_checkpoints() {
        let dir = std::env::temp_dir().join(format!("minibar-train-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = tiny_config();
        let mut generations = Vec::new();
        let first = train(&config, &catalog(), Some(&dir), &mut |p| {
            generations.push(p.generation)
        })
        .unwrap();
        assert_eq!(generations, vec![1, 2, 3]);
        let best = Brain::load(&dir.join("brain.best.json")).unwrap();
        assert_eq!(best.kind, tactical::KIND);
        assert!(dir.join("training_log.jsonl").exists());
        assert!(dir.join("summary.json").exists());
        assert_eq!(first.parameters_trained, best.param_count());

        let second = train(&config, &catalog(), None, &mut |_| {}).unwrap();
        assert_eq!(first.last.total, second.last.total);
        assert_eq!(first.best_generation, second.best_generation);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frozen_modules_do_not_change() {
        let dir = std::env::temp_dir().join(format!("minibar-freeze-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = tiny_config();
        config.train_modules = vec![tactical::SPACING.into()];
        train(&config, &catalog(), Some(&dir), &mut |_| {}).unwrap();
        let before = Brain::load(&dir.join("brain.initial.json")).unwrap();
        let after = Brain::load(&dir.join("brain.latest.json")).unwrap();
        for (a, b) in before.modules.iter().zip(&after.modules) {
            if a.spec.name == tactical::SPACING {
                assert_ne!(a.net, b.net);
            } else {
                assert_eq!(a.net, b.net, "{} should be frozen", a.spec.name);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn v4_config_selects_trainable_kind_and_old_configs_default_v3() {
        let legacy: TrainConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.brain_kind, tactical::KIND);
        let mut config = tiny_config();
        config.brain_kind = tactical::KIND_V4.into();
        let brain = initial_brain(&config).unwrap();
        assert_eq!(brain.kind, tactical::KIND_V4);
        assert!(
            brain
                .modules
                .iter()
                .any(|m| m.spec.name == tactical::MISSION)
        );
    }
    #[test]
    fn mapped_arena_rejects_blocked_spawn_and_resolves_relative_path() {
        let dir = std::env::temp_dir().join(format!("minibar-map-training-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let map_path = dir.join("blocked.map_source.json");
        let source = serde_json::json!({"source_version":1,"map_name":"blocked","content_hash":"fixture","width":1000.0,"depth":1000.0,"water_level":0.0,
            "heights":{"cols":2,"rows":2,"origin_x":0.0,"origin_z":0.0,"step_x":1000.0,"step_z":1000.0,"values":[42.0,42.0,42.0,42.0]},
            "passability":[{"movement_class":"BOT","cols":2,"rows":2,"cell_size_x":500.0,"cell_size_z":500.0,"passable":[false,false,false,false]}],"continuous_metal":null});
        std::fs::write(&map_path, serde_json::to_vec(&source).unwrap()).unwrap();
        let mut config = tiny_config();
        config.arena = Some(TrainingArenaSpec {
            map: PathBuf::from("blocked.map_source.json"),
            movement_class: "BOT".into(),
            cell_size: 100.0,
            center: Some([500.0, 500.0]),
        });
        let config_path = dir.join("config.json");
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let loaded = TrainConfig::load(&config_path).unwrap();
        assert_eq!(loaded.arena.as_ref().unwrap().map, map_path);
        let error = build_arena(&loaded, &catalog(), 1, 0.0).err().unwrap();
        assert!(error.0.contains("spawn"), "{}", error.0);
        let mut legal = source.clone();
        legal["passability"][0]["passable"] = serde_json::json!([true, true, true, true]);
        std::fs::write(&map_path, serde_json::to_vec(&legal).unwrap()).unwrap();
        let prepared = prepare_arena(&loaded).unwrap().unwrap();
        assert_eq!(prepared.terrain.height_at(Vec2::new(500.0, 500.0)), 42.0);
        std::fs::remove_file(&map_path).unwrap();
        // Prepared map survives disappearance of its source: battles do not reread it.
        let arena = build_arena_prepared(&loaded, &catalog(), 1, 0.0, Some(&prepared)).unwrap();
        assert_eq!(arena.len(), loaded.scenarios.len() * 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn config_resolves_saved_brains_relative_to_config_directory() {
        let dir =
            std::env::temp_dir().join(format!("minibar-relative-brains-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = tiny_config();
        config.init_from = Some(PathBuf::from("baseline.json"));
        config.opponents = vec!["hold".into(), "brain:opponent.json".into()];
        let path = dir.join("config.json");
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let loaded = TrainConfig::load(&path).unwrap();
        assert_eq!(loaded.init_from, Some(dir.join("baseline.json")));
        assert_eq!(
            loaded.opponents[1],
            format!("brain:{}", dir.join("opponent.json").display())
        );
        assert_eq!(loaded.opponents[0], "hold");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
