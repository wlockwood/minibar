//! Data for the council viewer (`viewer/brain_watch.html`).
//!
//! One traced battle is the usual [`RunTrace`] (positions, health, aim lines)
//! plus, for every sampled frame, the full reasoning of each of the learner's
//! units: every module's inputs, hidden activations and outputs, and how they
//! combined. The viewer re-draws the perceptron graph from those numbers and
//! from the weights in the brain file, so what it shows is exactly what ran.

use super::brain::Brain;
use super::objective::{Outcome, assess_objective};
use super::tactical::{self, DecisionTrace, TacticalPolicy};
use super::train::{Opponent, TrainConfig, TrainError, watch_simulator};
use crate::catalog::UnitCatalog;
use crate::model::{Command, SIM_HZ, UnitId, UnitState, WorldView};
use crate::policy::Policy;
use crate::trace::{AimState, TraceConfig};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Frames are sampled every 3 ticks (10 Hz); the viewer interpolates.
pub const SAMPLE_EVERY_TICKS: u64 = 3;

fn r3(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}

fn r4(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

fn r2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

/// One run of one module. Only the inputs are sent: the viewer recomputes
/// hidden activations and outputs from them and the weights in the brain file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WatchRun {
    /// Index into the brain's module list.
    pub module: usize,
    /// For the target selector: which enemy this run scored.
    pub candidate: Option<UnitId>,
    /// Inputs to 4 decimals.
    pub inputs: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WatchDecision {
    pub unit: UnitId,
    pub target: Option<UnitId>,
    pub runs: Vec<WatchRun>,
    /// Combined movement vector (world frame) and throttle after the fixed
    /// formula and doctrine.
    pub movement: [f32; 2],
    pub throttle: f32,
    /// How clearly the whole platoon outguns the enemy side (0..1).
    pub commit: f32,
    /// The module outputs behind the movement: advance, orbit, regroup,
    /// retreat, bloom. The viewer recomputes them itself; scripts read these.
    pub out: [f32; 5],
}

/// Static facts about a unit; per-frame data refers to it by id.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WatchUnit {
    pub id: UnitId,
    pub side: u8,
    /// Catalog name of the unit's definition.
    pub name: String,
    pub max_hp: f32,
    pub radius: f32,
}

/// One sampled instant. Short keys keep standalone files small.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WatchSample {
    pub tick: u64,
    pub time: f32,
    /// `[id, x, z, heading, hp]` per unit; hp 0 means dead.
    pub u: Vec<[f32; 5]>,
    /// Actual weapon yaws per unit: `[id, [yaw, ...]]`, independently of hull yaw.
    pub w: Vec<(UnitId, Vec<Option<f32>>)>,
    /// Shots fired this sample: `[shooter, origin_x, origin_z, aim_x, aim_z, weapon]`.
    pub f: Vec<[f32; 6]>,
    /// Reasoning of each council unit that decided on this tick.
    pub d: Vec<WatchDecision>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WatchBattle {
    pub scenario: usize,
    pub opponent: usize,
    pub seed: u64,
    pub learner_side: usize,
    /// `win`, `loss`, `timeout` or `wipe-out`, from the learner's side.
    pub outcome: &'static str,
    pub fitness: f64,
    pub exchange: f64,
    /// Primary weapon range of the learner's first unit (for a range ring).
    pub range: f32,
    pub sample_every_ticks: u64,
    pub units: Vec<WatchUnit>,
    pub frames: Vec<WatchSample>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScenarioInfo {
    pub name: String,
    pub learner: String,
    pub enemy: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct WatchMeta {
    pub title: String,
    /// The brain file contents (modules, roles, labels, every weight).
    pub brain: serde_json::Value,
    pub input_ranges: BTreeMap<String, [f32; 2]>,
    pub scenarios: Vec<ScenarioInfo>,
    pub opponents: Vec<String>,
    pub tick_hz: f32,
}

fn roster_text(roster: &[super::train::RosterEntry]) -> String {
    roster
        .iter()
        .map(|entry| format!("{}× {}", entry.count, entry.unit))
        .collect::<Vec<_>>()
        .join(" + ")
}

pub fn meta(brain: &Brain, config: &TrainConfig, title: &str) -> WatchMeta {
    let mut input_ranges = BTreeMap::new();
    for module in &brain.modules {
        for input in &module.spec.inputs {
            let (low, high) = tactical::input_range(input);
            input_ranges.insert(input.clone(), [low, high]);
        }
    }
    WatchMeta {
        title: title.to_owned(),
        brain: serde_json::from_str(&brain.to_json()).expect("brain JSON re-parses"),
        input_ranges,
        scenarios: config
            .scenarios
            .iter()
            .map(|s| ScenarioInfo {
                name: s.name.clone(),
                learner: roster_text(&s.learner),
                enemy: roster_text(&s.enemy),
            })
            .collect(),
        opponents: config.opponents.clone(),
        tick_hz: SIM_HZ,
    }
}

struct DecisionRecorder<'a> {
    policy: &'a TacticalPolicy,
    companion_router: Option<Box<dyn Policy>>,
    frames: Mutex<BTreeMap<u64, Vec<WatchDecision>>>,
}

impl DecisionRecorder<'_> {
    fn convert(&self, trace: DecisionTrace) -> WatchDecision {
        let brain = self.policy.brain();
        let runs = trace
            .modules
            .iter()
            .map(|run| WatchRun {
                module: brain
                    .module_index(&run.module)
                    .expect("trace names a module of this brain"),
                candidate: run.candidate,
                inputs: run.inputs.iter().map(|(_, v)| r4(*v)).collect(),
            })
            .collect();
        WatchDecision {
            unit: trace.unit,
            target: trace.chosen_target,
            runs,
            movement: [r3(trace.movement[0]), r3(trace.movement[1])],
            throttle: r3(trace.throttle),
            commit: r3(trace.commit),
            out: [
                r2(trace.advance),
                r2(trace.orbit),
                r2(trace.regroup),
                r2(trace.retreat),
                r2(trace.bloom),
            ],
        }
    }
}

impl Policy for DecisionRecorder<'_> {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        if unit.definition.flight.is_some() != (self.policy.brain().kind == super::air::KIND) {
            if let Some(router) = &self.companion_router {
                return router.command(world, unit);
            }
        }
        let (command, trace) = self.policy.explain(world, unit);
        let tick = (world.time * SIM_HZ).round() as u64;
        if tick % SAMPLE_EVERY_TICKS == 0 {
            if let Some(trace) = trace {
                let decision = self.convert(trace);
                self.frames
                    .lock()
                    .expect("recorder lock")
                    .entry(tick)
                    .or_default()
                    .push(decision);
            }
        }
        command
    }
}

/// Plays one battle of `scenario` against `opponents[opponent]` and records
/// everything the viewer needs. `seed` drives both shot randomness and the
/// layout jitter, so the same arguments always give the same battle.
pub fn battle(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    policy: &TacticalPolicy,
    opponents: &[Opponent],
    scenario: usize,
    opponent: usize,
    seed: u64,
    learner_side: usize,
) -> Result<WatchBattle, TrainError> {
    let opponent_policy = opponents
        .get(opponent)
        .ok_or_else(|| TrainError(format!("no opponent {opponent}")))?;
    let simulator = watch_simulator(config, catalog, scenario, learner_side, seed)?;
    let recorder = DecisionRecorder {
        policy,
        companion_router: if config.companion_brain.is_some() {
            Some(super::train::policy_for_brain(
                config,
                policy.brain().clone(),
            )?)
        } else {
            None
        },
        frames: Mutex::new(BTreeMap::new()),
    };
    let policies: [&dyn Policy; 2] = if learner_side == 0 {
        [&recorder, opponent_policy.policy.as_ref()]
    } else {
        [opponent_policy.policy.as_ref(), &recorder]
    };
    let (result, trace) = simulator.run_traced(
        policies,
        seed,
        TraceConfig {
            sample_every_ticks: SAMPLE_EVERY_TICKS,
            max_frames: 0,
        },
    );

    let mut by_tick = recorder.frames.into_inner().expect("recorder lock");
    let units = trace
        .frames
        .first()
        .map(|frame| {
            frame
                .units
                .iter()
                .map(|u| WatchUnit {
                    id: u.id,
                    side: u.side,
                    name: u.definition.clone(),
                    max_hp: u.max_hp,
                    radius: u.radius,
                })
                .collect()
        })
        .unwrap_or_default();
    let frames = trace
        .frames
        .iter()
        .map(|frame| WatchSample {
            tick: frame.tick,
            time: r3(frame.time),
            u: frame
                .units
                .iter()
                .map(|u| {
                    let hp = if u.alive {
                        ((u.hp * 10.0).round() / 10.0).max(0.1)
                    } else {
                        0.0
                    };
                    [
                        u.id as f32,
                        r2(u.position[0]),
                        r2(u.position[1]),
                        r3(u.heading),
                        hp,
                    ]
                })
                .collect(),
            w: frame
                .units
                .iter()
                .map(|u| {
                    (
                        u.id,
                        u.weapon_headings.iter().map(|yaw| yaw.map(r3)).collect(),
                    )
                })
                .collect(),
            f: frame
                .aim_attempts
                .iter()
                .filter(|aim| aim.state == AimState::Fired)
                .map(|aim| {
                    [
                        aim.shooter as f32,
                        r2(aim.origin[0]),
                        r2(aim.origin[1]),
                        r2(aim.aim_point[0]),
                        r2(aim.aim_point[1]),
                        aim.weapon as f32,
                    ]
                })
                .collect(),
            d: by_tick.remove(&frame.tick).unwrap_or_default(),
        })
        .collect();

    let objective = config.scenarios[scenario]
        .objective
        .as_ref()
        .map(|o| o.kind);
    let assessment = assess_objective(&result, learner_side, &config.objective, objective);
    let range = config.scenarios[scenario]
        .learner
        .first()
        .and_then(|entry| catalog.get(&entry.unit))
        .and_then(|def| def.weapons.first())
        .map_or(0.0, |w| w.range);

    Ok(WatchBattle {
        scenario,
        opponent,
        seed,
        learner_side,
        outcome: match assessment.outcome {
            Outcome::Win => "win",
            Outcome::Loss => "loss",
            Outcome::Timeout => "timeout",
            Outcome::Draw => "wipe-out",
        },
        fitness: assessment.fitness,
        exchange: assessment.exchange,
        range,
        sample_every_ticks: SAMPLE_EVERY_TICKS,
        units,
        frames,
    })
}

const VIEWER_HTML: &str = include_str!("../../viewer/brain_watch.html");

/// Marker the template replaces with embedded data (`null` when served live).
const DATA_MARKER: &str = "/*__WATCH_DATA__*/null";

#[derive(Serialize)]
struct Embedded<'a> {
    meta: &'a WatchMeta,
    battles: &'a [WatchBattle],
}

/// One standalone HTML file: no server, every embedded battle selectable.
pub fn standalone_html(meta: &WatchMeta, battles: &[WatchBattle]) -> String {
    let json = serde_json::to_string(&Embedded { meta, battles })
        .expect("watch data serializes")
        .replace('<', "\\u003c");
    VIEWER_HTML.replace(DATA_MARKER, &json)
}

/// The viewer without embedded data; it then talks to `scripts/watch.py`.
pub fn live_html() -> &'static str {
    VIEWER_HTML
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::rng::Rng;
    use crate::learn::tactical::tests::skirmisher;
    use crate::learn::train::opponents_from_config;

    fn setup() -> (TrainConfig, UnitCatalog, TacticalPolicy) {
        let mut catalog = UnitCatalog::default();
        catalog.units.insert("skirm".into(), skirmisher("skirm"));
        let config = TrainConfig {
            opponents: vec!["charge".into(), "kite".into()],
            max_time: 20.0,
            ..TrainConfig::default_for_unit("skirm")
        };
        let brain =
            Brain::random(tactical::KIND, tactical::specs(5), &mut Rng::new(3), 1.0).unwrap();
        (config, catalog, TacticalPolicy::new(brain).unwrap())
    }

    #[test]
    fn every_frame_carries_the_reasoning_of_each_living_learner_unit() {
        let (config, catalog, policy) = setup();
        let opponents = opponents_from_config(&config).unwrap();
        // Scenario 1 is 3v3; put the learner on side 1 to exercise that path too.
        let b = battle(&config, &catalog, &policy, &opponents, 1, 0, 7, 1).unwrap();
        assert!(b.frames.len() > 10);
        assert_eq!(b.units.len(), 6);
        for frame in &b.frames {
            let side_of = |id: f32| b.units.iter().find(|u| u.id as f32 == id).unwrap().side;
            let alive_learner = frame
                .u
                .iter()
                .filter(|row| side_of(row[0]) == 1 && row[4] > 0.0)
                .count();
            // Decisions are made before this tick's damage lands, so a unit that
            // dies on the tick may still have decided.
            assert!(frame.d.len() >= alive_learner, "tick {}", frame.tick);
            for decision in &frame.d {
                let selector_runs = decision.runs.iter().filter(|r| r.module == 0).count();
                assert!((1..=tactical::TARGET_CANDIDATES).contains(&selector_runs));
                assert!(
                    decision
                        .runs
                        .iter()
                        .filter(|r| r.module == 0)
                        .all(|r| r.candidate.is_some())
                );
                assert!(decision.target.is_some());
            }
        }
    }

    #[test]
    fn battles_are_reproducible_and_seeds_change_the_layout() {
        let (config, catalog, policy) = setup();
        let opponents = opponents_from_config(&config).unwrap();
        let run = |seed| {
            serde_json::to_string(
                &battle(&config, &catalog, &policy, &opponents, 0, 1, seed, 0).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(run(4), run(4));
        assert_ne!(run(4), run(5));
    }

    #[test]
    fn compact_battles_keep_independent_weapon_headings_for_both_sides() {
        let (config, catalog, policy) = setup();
        let opponents = opponents_from_config(&config).unwrap();
        let battle = battle(&config, &catalog, &policy, &opponents, 1, 0, 7, 1).unwrap();
        let json = serde_json::to_value(&battle).unwrap();
        let mut independent_aim = false;
        for (sample, wire) in battle.frames.iter().zip(json["frames"].as_array().unwrap()) {
            let headings = wire["w"]
                .as_array()
                .expect("weapon yaw must reach the council viewer");
            assert_eq!(headings.len(), sample.u.len());
            for (unit, weapons) in sample.u.iter().zip(headings) {
                assert_eq!(weapons[0].as_u64().unwrap(), unit[0] as u64);
                let yaw = weapons[1][0].as_f64().unwrap();
                independent_aim |= (yaw - unit[3] as f64).abs() > 0.1;
            }
        }
        assert!(
            independent_aim,
            "fixture must distinguish weapon yaw from hull yaw"
        );
    }

    #[test]
    fn viewer_embeds_data_and_escapes_script_terminators() {
        let (config, catalog, policy) = setup();
        let opponents = opponents_from_config(&config).unwrap();
        let b = battle(&config, &catalog, &policy, &opponents, 0, 0, 1, 0).unwrap();
        let m = meta(policy.brain(), &config, "a </script> title");
        let html = standalone_html(&m, std::slice::from_ref(&b));
        assert!(!html.contains(DATA_MARKER));
        assert!(!html.contains("a </script> title"));
        assert!(html.contains("\"learner_side\":0"));
        assert!(live_html().contains(DATA_MARKER));
        // Meta carries labelled inputs and a display range for each.
        assert_eq!(m.input_ranges["target_approaching"], [-1.0, 1.0]);
        assert_eq!(m.scenarios[2].name, "3v5 underdog");
        assert_eq!(m.scenarios[2].learner, "3× skirm");
    }
}
