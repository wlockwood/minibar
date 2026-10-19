//! `tactical_v3`: per-unit movement and targeting as a council of six tiny
//! perceptrons, plus a small fixed doctrine.
//!
//! ```text
//!   situation ──► target_selector ──► chosen enemy
//!                      │ (scores each of the 3 nearest enemies, highest wins)
//!                      ▼
//!   spacing    ─► advance  ┐
//!   strafe     ─► orbit    │
//!   cohesion   ─► regroup  ├─► fight = advance·toward + orbit·tangent + regroup·ally_dir
//!   encircle   ─► bloom   ─┘             + gate·bloom·around_enemy
//!   engagement ─► retreat ──► flee = clamp(retreat,0,1)·(1 − commit)
//!                             move = (1−flee)·fight + flee·away_from_enemy_force
//!   doctrine: if out of contact, move = (1−pull)·move + pull·toward_target
//!                 pull = commit · clamp((distance_over_range − 0.8) / 0.4, 0, 1)
//! ```
//!
//! The combining formula is fixed code, not learned, so every behaviour can be
//! attributed to exactly one named module output.
//!
//! **Doctrine.** `commit` measures how clearly the *whole platoon* (not just
//! this unit's neighbourhood) outguns the enemy side: the ratio of
//! `DPS × HP` between the two sides, 0 at 1.5× and 1 at 2.5×. A platoon that
//! is clearly ahead does not retreat, and a unit that has drifted out of
//! contact is pulled back to the fight. This is deliberately not learned:
//! a brain that has found a way to score by running must not be able to leave
//! units idle while their side is winning.
//!
//! **Force inputs.** Following the approach published for Supreme Commander 2
//! (Robbins, *Game AI Pro* ch. 30) and visible in Planetary Annihilation's AI
//! vocabulary, the council reads *ratios* between the friendly and enemy
//! force rather than raw counts: each statistic enters as `ours/theirs` and
//! `theirs/ours`, both clamped to `0..1`, so the network can see the balance
//! whichever side is ahead. Local statistics cover three weapon ranges around
//! the unit; platoon statistics cover the whole side.
//!
//! **Encirclement.** `encircle` sees the unit's bearing around the enemy force
//! relative to the enemy's line of advance, and the angular gaps to its nearest
//! friendly neighbours on each side. Sliding toward the larger gap spreads
//! friends into an even ring; sliding away from the enemy's front puts them on
//! its flanks and rear.

use super::brain::{Brain, BrainError, Module, ModuleSpec};
use super::rng::Rng;
use crate::model::{Command, UnitId, UnitState, Vec2, WorldView};
use crate::policy::Policy;
use serde::Serialize;
use std::f32::consts::TAU;

pub const KIND: &str = "tactical_v3";
pub const KIND_V4: &str = "tactical_v4";
pub const MISSION: &str = "mission";

pub const TARGET_SELECTOR: &str = "target_selector";
pub const SPACING: &str = "spacing";
pub const STRAFE: &str = "strafe";
pub const COHESION: &str = "cohesion";
pub const ENGAGEMENT: &str = "engagement";
pub const ENCIRCLE: &str = "encircle";

/// Enemies the target selector may choose between (nearest first).
pub const TARGET_CANDIDATES: usize = 3;
/// How far ahead of the unit the move command points. Matches the built-in
/// kite and orbit baselines.
const MOVE_LOOKAHEAD: f32 = 500.0;
/// Combined movement weaker than this means "stand still and shoot".
const MOVE_DEADZONE: f32 = 0.05;
/// Used for units without a weapon so range-relative inputs stay finite.
const FALLBACK_RANGE: f32 = 200.0;
/// Friend/enemy force statistics are gathered within this many of the unit's
/// own weapon ranges.
const LOCAL_RADIUS_RANGES: f32 = 3.0;
/// Weapon range bands, as multiples of the unit's own range: below
/// `SHORT_BAND` is short, up to `MID_BAND` is mid, beyond that is long.
const SHORT_BAND: f32 = 0.75;
const MID_BAND: f32 = 1.5;
/// Platoon power ratio (ours / theirs) at which commitment starts, and where
/// it is complete. 5 against 3 identical units is 2.8.
const COMMIT_FROM: f32 = 1.5;
const COMMIT_FULL: f32 = 2.5;
/// The doctrine pull starts this far (in own weapon ranges) from the target and
/// is complete 0.4 beyond it, so a pulled unit settles inside weapon range.
const PULL_START: f32 = 0.8;
const PULL_SPAN: f32 = 0.4;
/// How strongly a unit out of contact is drawn back when a friend is already
/// fighting its target, whatever the platoon balance (1 = as if clearly ahead).
const ENGAGED_ALLY_DRIVE: f32 = 1.0;
/// Objective doctrine. An attacking unit heads for the enemy's key unit with at
/// least this much of its movement, rising to all of it with commitment, and
/// stops closing once the target is within this fraction of its weapon range.
const OBJECTIVE_BASE_PULL: f32 = 0.4;
const OBJECTIVE_REACH: f32 = 0.9;
/// Encirclement strength when the sides are evenly matched; it rises to 1 with
/// commitment.
const BLOOM_BASE_GATE: f32 = 0.2;
/// An enemy force moving slower than this fraction of its mean top speed has no
/// line of advance; its "front" is then the side facing us.
const ADVANCE_MIN_SPEED: f32 = 0.15;

/// Statistics compared between the friendly and enemy force.
pub const FORCE_STATS: [&str; 8] = [
    "count",
    "hp",
    "dps",
    "value",
    "speed",
    "dps_short",
    "dps_mid",
    "dps_long",
];
const STAT_COUNT: usize = 0;
const STAT_DPS: usize = 2;

pub const TARGET_INPUTS: [&str; 6] = [
    "distance_over_range",
    "enemy_hp_fraction",
    "enemy_range_over_mine",
    "enemy_threat_over_mine",
    "enemy_value_over_mine",
    "is_nearest",
];
pub const SPACING_INPUTS: [&str; 11] = [
    "distance_over_range",
    "my_reload_remaining",
    "my_hp_fraction",
    "target_range_over_mine",
    "target_hp_fraction",
    "target_approaching",
    "ours_over_theirs_dps",
    "theirs_over_ours_dps",
    "my_range_rank",
    "platoon_ours_over_theirs_power",
    "platoon_theirs_over_ours_power",
];
pub const STRAFE_INPUTS: [&str; 8] = [
    "distance_over_range",
    "my_reload_remaining",
    "target_approaching",
    "target_range_over_mine",
    "my_hp_fraction",
    "ours_over_theirs_dps",
    "theirs_over_ours_dps",
    "my_speed_rank",
];
pub const COHESION_INPUTS: [&str; 5] = [
    "ally_centroid_distance_over_range",
    "nearest_ally_distance_over_range",
    "my_hp_fraction",
    "ours_over_theirs_count",
    "distance_over_range",
];
pub const ENGAGEMENT_INPUTS: [&str; 22] = [
    "ours_over_theirs_count",
    "theirs_over_ours_count",
    "ours_over_theirs_hp",
    "theirs_over_ours_hp",
    "ours_over_theirs_dps",
    "theirs_over_ours_dps",
    "ours_over_theirs_value",
    "theirs_over_ours_value",
    "ours_over_theirs_speed",
    "theirs_over_ours_speed",
    "ours_over_theirs_dps_short",
    "theirs_over_ours_dps_short",
    "ours_over_theirs_dps_mid",
    "theirs_over_ours_dps_mid",
    "ours_over_theirs_dps_long",
    "theirs_over_ours_dps_long",
    "my_hp_fraction",
    "my_speed_rank",
    "my_range_rank",
    "my_hp_rank",
    "platoon_ours_over_theirs_power",
    "platoon_theirs_over_ours_power",
];
pub const ENCIRCLE_INPUTS: [&str; 5] = [
    "enemy_front_alignment",
    "enemy_side",
    "gap_ccw",
    "gap_cw",
    "ring_distance_over_range",
];

/// Natural display range of a named module input, used by viewers for what-if
/// sliders. These are the clamps the encoder applies.
pub fn input_range(name: &str) -> (f32, f32) {
    if name.contains("ours_over_theirs")
        || name.contains("theirs_over_ours")
        || name.ends_with("_rank")
    {
        return (0.0, 1.0);
    }
    match name {
        "target_approaching"
        | "enemy_front_alignment"
        | "enemy_side"
        | "speed_balance"
        | "closing_speed"
        | "objective_alignment"
        | "local_damage_balance" => (-1.0, 1.0),
        "enemy_threat_over_mine" | "enemy_value_over_mine" => (0.0, 4.0),
        "is_nearest"
        | "my_reload_remaining"
        | "my_hp_fraction"
        | "enemy_hp_fraction"
        | "target_hp_fraction"
        | "gap_ccw"
        | "gap_cw" => (0.0, 1.0),
        // Everything else is a distance or range measured in own weapon ranges.
        _ => (0.0, 3.0),
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// The six module contracts. `hidden` is the width of the single hidden layer.
pub fn specs(hidden: usize) -> Vec<ModuleSpec> {
    let hidden = vec![hidden];
    vec![
        ModuleSpec {
            name: TARGET_SELECTOR.into(),
            role: "Scores one enemy at a time (same weights for every candidate); the \
                   highest-scoring of the three nearest enemies becomes the target."
                .into(),
            inputs: strings(&TARGET_INPUTS),
            outputs: strings(&["score"]),
            hidden: hidden.clone(),
        },
        ModuleSpec {
            name: SPACING.into(),
            role: "Owns range: close on the target (+1) or back away from it (-1) along \
                   the line between them."
                .into(),
            inputs: strings(&SPACING_INPUTS),
            outputs: strings(&["advance"]),
            hidden: hidden.clone(),
        },
        ModuleSpec {
            name: STRAFE.into(),
            role: "Owns sideways motion: circle the target counter-clockwise (+1) or \
                   clockwise (-1), e.g. to dodge a projectile weapon."
                .into(),
            inputs: strings(&STRAFE_INPUTS),
            outputs: strings(&["orbit"]),
            hidden: hidden.clone(),
        },
        ModuleSpec {
            name: COHESION.into(),
            role: "Owns group shape: drift toward the centre of friendly units (+1) or \
                   spread away from it (-1). Ignored when the unit has no allies."
                .into(),
            inputs: strings(&COHESION_INPUTS),
            outputs: strings(&["regroup"]),
            hidden: hidden.clone(),
        },
        ModuleSpec {
            name: ENGAGEMENT.into(),
            role: "Owns fight or flight: reads the friend/enemy force balance and, above \
                   zero, pulls the unit out of the fight by that fraction, running away \
                   from the enemy force. At or below zero it fights. Never retreats \
                   while the platoon is clearly ahead (the doctrine)."
                .into(),
            inputs: strings(&ENGAGEMENT_INPUTS),
            outputs: strings(&["retreat"]),
            hidden: hidden.clone(),
        },
        ModuleSpec {
            name: ENCIRCLE.into(),
            role: "Owns the formation around the enemy force: slides the unit around it \
                   (+1 counter-clockwise) toward the larger gap between itself and its \
                   neighbours, so friends spread into an even ring, and away from the \
                   enemy's line of advance, so they flank instead of bunching in front. \
                   Scaled up as the platoon gets further ahead."
                .into(),
            inputs: strings(&ENCIRCLE_INPUTS),
            outputs: strings(&["bloom"]),
            hidden,
        },
    ]
}

/// Versioned contracts leave existing v3 files and their execution unchanged.
pub fn specs_for_kind(kind: &str, hidden: usize) -> Result<Vec<ModuleSpec>, BrainError> {
    if kind == super::air::KIND {
        return Ok(super::air::specs(hidden));
    }
    let mut modules = specs(hidden);
    match kind {
        KIND => return Ok(modules),
        KIND_V4 => {}
        _ => {
            return Err(BrainError::KindMismatch {
                expected: format!("{KIND} or {KIND_V4}"),
                found: kind.into(),
            });
        }
    }
    modules[0].inputs.extend(strings(&[
        "can_damage",
        "enemy_can_damage_me",
        "target_is_key",
        "speed_balance",
    ]));
    modules[0].role = "Scores eligible contacts plus the nearest eligible enemy key asset; legality is enforced independently.".into();
    modules[1].inputs.extend(strings(&[
        "speed_balance",
        "time_to_enemy_range",
        "closing_speed",
    ]));
    modules.push(ModuleSpec {
        name: MISSION.into(),
        role: "Balances advancing on an enemy key asset (+1) against intercepting threats to our key asset (-1); zero preserves local combat.".into(),
        inputs: strings(&["own_key_present", "enemy_key_present", "own_key_distance", "enemy_key_distance", "own_key_hp", "enemy_key_hp", "enemy_key_arrival_time", "threat_to_own_key_distance", "threat_to_own_key_arrival_time", "can_damage_enemy_key", "local_damage_balance", "my_hp_fraction", "objective_alignment", "own_key_under_threat"]),
        outputs: strings(&["objective_drive"]), hidden: vec![hidden],
    });
    Ok(modules)
}

/// Research-informed authored initialization, not imitation-learned weights.
pub fn template_for_kind(kind: &str, hidden: usize) -> Result<Brain, BrainError> {
    if kind == super::air::KIND {
        return super::air::template(hidden);
    }
    if kind == KIND {
        return template(hidden);
    }
    let contracts = specs_for_kind(kind, hidden)?;
    let old = template(hidden)?;
    let mut brain = Brain::random(kind, contracts, &mut Rng::new(0), 0.0)?;
    for module in &mut brain.modules {
        module.net.params_mut().fill(0.0);
        if let Some(previous) = old.modules.iter().find(|m| m.spec.name == module.spec.name) {
            // Lift the old single-hidden-layer weights by label; extra sensors start at zero.
            let old_inputs = previous.spec.inputs.len();
            let inputs = module.spec.inputs.len();
            for h in 0..hidden {
                for (i, label) in previous.spec.inputs.iter().enumerate() {
                    let new_i = module.spec.inputs.iter().position(|n| n == label).unwrap();
                    module.net.params_mut()[h * inputs + new_i] =
                        previous.net.params()[h * old_inputs + i];
                }
            }
            module.net.params_mut()[hidden * inputs..]
                .copy_from_slice(&previous.net.params()[hidden * old_inputs..]);
        }
        let mut wiring = Wiring { module };
        match wiring.module.spec.name.as_str() {
            TARGET_SELECTOR => {
                wiring
                    .reads(1, "target_is_key", 2.0)
                    .reads(1, "can_damage", 0.5)
                    .feeds_output(1, 1.2);
            }
            SPACING => {
                wiring
                    .reads(1, "speed_balance", -1.0)
                    .reads(1, "closing_speed", 0.5);
            }
            MISSION => {
                wiring
                    .reads(0, "enemy_key_present", 1.4)
                    .reads(0, "can_damage_enemy_key", 1.0)
                    .reads(0, "enemy_key_arrival_time", -0.3)
                    .feeds_output(0, 1.2);
                wiring
                    .reads(1, "own_key_under_threat", 2.5)
                    .feeds_output(1, -1.2);
            }
            _ => {}
        }
    }
    Ok(brain)
}

fn eligible_damage_rate(unit: &UnitState, target: &UnitState) -> f32 {
    unit.definition
        .weapons
        .iter()
        .filter(|weapon| {
            !weapon.manual_fire
                && !weapon.is_shield
                && weapon.interceptor == 0
                && (weapon.only_target_categories.is_empty()
                    || weapon.only_target_categories.iter().any(|allowed| {
                        target
                            .definition
                            .strategic
                            .target_categories
                            .contains(allowed)
                    }))
        })
        .map(|weapon| {
            weapon.damage_against(&target.definition.strategic.armor_class)
                * f32::from(weapon.burst.max(1))
                * f32::from(weapon.projectiles.max(1))
                / weapon.reload.max(0.05)
        })
        .sum()
}

fn eligible_weapon(unit: &UnitState, target: &UnitState) -> Option<usize> {
    unit.definition
        .weapons
        .iter()
        .enumerate()
        .filter(|(_, weapon)| {
            !weapon.manual_fire
                && !weapon.is_shield
                && weapon.interceptor == 0
                && weapon.damage_against(&target.definition.strategic.armor_class) > 0.0
                && (weapon.only_target_categories.is_empty()
                    || weapon.only_target_categories.iter().any(|allowed| {
                        target
                            .definition
                            .strategic
                            .target_categories
                            .contains(allowed)
                    }))
        })
        .max_by(|(_, a), (_, b)| a.range.total_cmp(&b.range))
        .map(|(index, _)| index)
}

// --- Situation encoding ------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    pub id: UnitId,
    pub position: Vec2,
    pub distance_over_range: f32,
    pub hp_fraction: f32,
    pub range_over_mine: f32,
    pub threat_over_mine: f32,
    pub value_over_mine: f32,
    pub approaching: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct Situation {
    pub hp_fraction: f32,
    pub reload_remaining: f32,
    /// `[ours/theirs, theirs/ours]` for each of [`FORCE_STATS`], in order, each
    /// clamped to `0..1`, over the local radius.
    pub force: [f32; 2 * FORCE_STATS.len()],
    /// The same pair for `DPS x HP` over the whole side, not just locally.
    pub platoon_power: (f32, f32),
    /// 0..1: how clearly the whole platoon outguns the enemy side.
    pub commit: f32,
    /// This unit's speed, range and maximum health relative to the best
    /// friendly combatant (1 for the best, and for a lone or uniform force).
    pub speed_rank: f32,
    pub range_rank: f32,
    pub hp_rank: f32,
    /// Centre of the enemy combatants inside the local radius, if any.
    pub enemy_centroid: Option<Vec2>,
    /// Geometry around that centre, for `encircle`.
    pub front_alignment: f32,
    pub side: f32,
    pub gap_ccw: f32,
    pub gap_cw: f32,
    pub ring_distance_over_range: f32,
    /// Unit vector along which positive `bloom` moves this unit.
    pub bloom_tangent: Option<Vec2>,
    pub ally_centroid: Option<Vec2>,
    pub ally_centroid_distance_over_range: f32,
    pub nearest_ally_distance_over_range: f32,
    pub candidates: [Option<Candidate>; TARGET_CANDIDATES],
    /// Friendly combatants already in weapon range of this unit's nearest enemy.
    pub allies_engaged: usize,
    /// The nearest enemy key unit (something to destroy): id, position and distance.
    pub enemy_objective: Option<(UnitId, Vec2, f32)>,
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn clamp_ratio(value: f32, maximum: f32) -> f32 {
    finite(value).clamp(0.0, maximum)
}

fn paper_dps(unit: &UnitState) -> f32 {
    unit.definition.weapons.iter().map(|w| w.paper_dps()).sum()
}

fn is_combatant(unit: &UnitState) -> bool {
    !unit.definition.weapons.is_empty()
}

/// Running totals for one side's force.
#[derive(Clone, Copy, Debug, Default)]
struct Force {
    count: f32,
    hp: f32,
    dps: f32,
    value: f32,
    speed_sum: f32,
    /// DPS by weapon range band: short, mid, long.
    bands: [f32; 3],
}

impl Force {
    fn add(&mut self, unit: &UnitState, own_range: f32) {
        self.count += 1.0;
        self.hp += unit.hp.max(0.0);
        self.dps += paper_dps(unit);
        self.value += unit.definition.resource_value();
        self.speed_sum += unit.definition.movement.max_speed;
        for weapon in &unit.definition.weapons {
            let relative = weapon.range / own_range;
            let band = if relative < SHORT_BAND {
                0
            } else if relative <= MID_BAND {
                1
            } else {
                2
            };
            self.bands[band] += weapon.paper_dps();
        }
    }

    /// In [`FORCE_STATS`] order.
    fn stats(&self) -> [f32; FORCE_STATS.len()] {
        let speed = if self.count > 0.0 {
            self.speed_sum / self.count
        } else {
            0.0
        };
        [
            self.count,
            self.hp,
            self.dps,
            self.value,
            speed,
            self.bands[0],
            self.bands[1],
            self.bands[2],
        ]
    }

    /// Lanchester-flavoured strength: damage rate times staying power.
    fn power(&self) -> f32 {
        self.dps * self.hp
    }
}

/// `(ours/theirs, theirs/ours)`, each clamped to `0..1`. When neither side has
/// any of the statistic both are zero; when only one side does, the other
/// side's ratio is zero and this side's is one.
pub(crate) fn ratio_pair(ours: f32, theirs: f32) -> (f32, f32) {
    let (ours, theirs) = (finite(ours).max(0.0), finite(theirs).max(0.0));
    if ours <= 0.0 && theirs <= 0.0 {
        (0.0, 0.0)
    } else if theirs <= 0.0 {
        (1.0, 0.0)
    } else if ours <= 0.0 {
        (0.0, 1.0)
    } else {
        ((ours / theirs).min(1.0), (theirs / ours).min(1.0))
    }
}

/// How committed the platoon is to the fight, from the raw `ours / theirs`
/// power ratio: 0 at [`COMMIT_FROM`] or below, 1 at [`COMMIT_FULL`] or above.
/// An enemy side with no power at all is a walkover.
pub(crate) fn commitment(ours_power: f32, theirs_power: f32) -> f32 {
    if ours_power <= 0.0 {
        0.0
    } else if theirs_power <= 0.0 {
        1.0
    } else {
        ((ours_power / theirs_power - COMMIT_FROM) / (COMMIT_FULL - COMMIT_FROM)).clamp(0.0, 1.0)
    }
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.z - a.z * b.x
}

fn dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.z * b.z
}

/// Signed angle from `front` to `r`, positive in the direction `Vec2::rotate`
/// turns by a positive angle.
fn bearing(front: Vec2, r: Vec2) -> f32 {
    cross(front, r).atan2(dot(front, r))
}

impl Situation {
    /// A few passes over the world; no allocation.
    pub(crate) fn observe(world: &WorldView<'_>, me: &UnitState) -> Option<Self> {
        let range = match me.primary_range() {
            r if r > 1.0 => r,
            _ => FALLBACK_RANGE,
        };
        let local_radius_squared = (range * LOCAL_RADIUS_RANGES).powi(2);
        let my_dps = paper_dps(me);
        let my_value = me.definition.resource_value().max(1e-3);

        let mut nearest: [Option<(f32, &UnitState)>; TARGET_CANDIDATES] = [None; TARGET_CANDIDATES];
        let mut allies = 0usize;
        let mut ally_sum = Vec2::default();
        let mut nearest_ally = f32::INFINITY;
        let mut ours = Force::default();
        let mut theirs = Force::default();
        let mut platoon_ours = Force::default();
        let mut platoon_theirs = Force::default();
        let mut ours_position_sum = Vec2::default();
        let mut enemy_objective: Option<(f32, UnitId, Vec2)> = None;
        let mut enemy_sum = Vec2::default();
        let mut enemy_velocity_sum = Vec2::default();
        // Best friendly combatant, over the whole side (not just locally).
        let (mut best_speed, mut best_range, mut best_health) = (0.0f32, 0.0f32, 0.0f32);

        if is_combatant(me) {
            ours.add(me, range);
            platoon_ours.add(me, range);
            ours_position_sum = me.position;
            best_speed = me.definition.movement.max_speed;
            best_range = me.primary_range();
            best_health = me.definition.health;
        }

        for other in world.units {
            if !other.alive || other.side >= 2 || other.id == me.id {
                continue;
            }
            let d2 = me.position.distance_squared(other.position);
            let local = d2 <= local_radius_squared;
            if other.definition.strategic.key_unit {
                if other.side != me.side && enemy_objective.is_none_or(|(held, _, _)| d2 < held) {
                    enemy_objective = Some((d2, other.id, other.position));
                }
            }
            if other.side == me.side {
                if is_combatant(other) {
                    platoon_ours.add(other, range);
                    if local {
                        ours.add(other, range);
                        ours_position_sum = ours_position_sum + other.position;
                    }
                    best_speed = best_speed.max(other.definition.movement.max_speed);
                    best_range = best_range.max(other.primary_range());
                    best_health = best_health.max(other.definition.health);
                }
                allies += 1;
                ally_sum = ally_sum + other.position;
                nearest_ally = nearest_ally.min(d2.sqrt());
            } else {
                if is_combatant(other) {
                    platoon_theirs.add(other, range);
                    if local {
                        theirs.add(other, range);
                        enemy_sum = enemy_sum + other.position;
                        enemy_velocity_sum = enemy_velocity_sum + other.velocity();
                    }
                }
                // Insert into the sorted top-K by squared distance.
                let mut incoming = Some((d2, other));
                for slot in nearest.iter_mut() {
                    match (*slot, incoming) {
                        (None, Some(_)) => {
                            *slot = incoming;
                            incoming = None;
                        }
                        (Some((held, _)), Some((new, _))) if new < held => {
                            std::mem::swap(slot, &mut incoming);
                        }
                        _ => {}
                    }
                }
            }
        }

        nearest[0]?;

        let mut candidates = [None; TARGET_CANDIDATES];
        for (slot, found) in candidates.iter_mut().zip(nearest) {
            let Some((d2, enemy)) = found else { continue };
            let distance = d2.sqrt();
            let enemy_range = enemy.primary_range();
            let enemy_speed = enemy.definition.movement.max_speed;
            let toward_me = (me.position - enemy.position).normalized();
            let approaching = if enemy_speed > 1e-3 {
                (enemy.velocity().x * toward_me.x + enemy.velocity().z * toward_me.z) / enemy_speed
            } else {
                0.0
            };
            *slot = Some(Candidate {
                id: enemy.id,
                position: enemy.position,
                distance_over_range: clamp_ratio(distance / range, 3.0),
                hp_fraction: clamp_ratio(enemy.hp / enemy.definition.health.max(1e-3), 1.0),
                range_over_mine: clamp_ratio(enemy_range / range, 3.0),
                threat_over_mine: if my_dps > 1e-3 {
                    clamp_ratio(paper_dps(enemy) / my_dps, 4.0)
                } else {
                    1.0
                },
                value_over_mine: clamp_ratio(enemy.definition.resource_value() / my_value, 4.0),
                approaching: finite(approaching).clamp(-1.0, 1.0),
            });
        }

        let nearest_enemy_position = nearest[0].map(|(_, enemy)| enemy.position);
        let allies_engaged = nearest_enemy_position.map_or(0, |at| {
            world
                .units
                .iter()
                .filter(|a| {
                    a.alive
                        && a.side == me.side
                        && a.id != me.id
                        && is_combatant(a)
                        && a.position.distance_squared(at) <= (a.primary_range() * 1.1).powi(2)
                })
                .count()
        });
        let (ally_centroid, centroid_distance) = if allies > 0 {
            let centroid = ally_sum * (1.0 / allies as f32);
            (Some(centroid), me.position.distance(centroid))
        } else {
            (None, 0.0)
        };
        let reload = me.definition.weapons.first().map_or(0.0, |w| w.reload);

        let (our_stats, their_stats) = (ours.stats(), theirs.stats());
        let mut force = [0.0; 2 * FORCE_STATS.len()];
        for stat in 0..FORCE_STATS.len() {
            let (a, b) = ratio_pair(our_stats[stat], their_stats[stat]);
            force[2 * stat] = a;
            force[2 * stat + 1] = b;
        }
        let (power_ours, power_theirs) = (platoon_ours.power(), platoon_theirs.power());
        let rank = |mine: f32, best: f32| {
            if best > 1e-3 {
                clamp_ratio(mine / best, 1.0)
            } else {
                1.0
            }
        };

        // Encirclement geometry: where am I around the enemy force, relative to
        // its line of advance, and how much room do I have on each side?
        let enemy_centroid = (theirs.count > 0.0).then(|| enemy_sum * (1.0 / theirs.count));
        let (mut front_alignment, mut side, mut gap_ccw, mut gap_cw) = (0.0, 0.0, 1.0, 1.0);
        let mut ring_distance_over_range = 0.0;
        let mut bloom_tangent = None;
        if let Some(centre) = enemy_centroid {
            let mean_velocity = enemy_velocity_sum * (1.0 / theirs.count);
            let mean_top_speed = theirs.speed_sum / theirs.count;
            let ours_centre = if ours.count > 0.0 {
                ours_position_sum * (1.0 / ours.count)
            } else {
                me.position
            };
            let mut front = if mean_velocity.length() > ADVANCE_MIN_SPEED * mean_top_speed {
                mean_velocity.normalized()
            } else {
                (ours_centre - centre).normalized()
            };
            if front.length_squared() < 0.5 {
                front = Vec2::new(1.0, 0.0);
            }
            let radial = me.position - centre;
            let mine = bearing(front, radial);
            front_alignment = mine.cos();
            side = mine.sin();
            ring_distance_over_range = clamp_ratio(radial.length() / range, 3.0);
            if radial.length_squared() > 1.0 {
                bloom_tangent = Some(Vec2::new(-radial.z, radial.x).normalized());
            }
            // Gaps to the nearest friendly combatants either way round.
            let (mut nearest_ccw, mut nearest_cw) = (TAU, TAU);
            for other in world.units {
                if !other.alive
                    || other.side != me.side
                    || other.id == me.id
                    || !is_combatant(other)
                    || other.position.distance_squared(me.position) > local_radius_squared
                {
                    continue;
                }
                let delta = (bearing(front, other.position - centre) - mine).rem_euclid(TAU);
                nearest_ccw = nearest_ccw.min(delta);
                nearest_cw = nearest_cw.min(TAU - delta);
            }
            gap_ccw = nearest_ccw / TAU;
            gap_cw = nearest_cw / TAU;
        }

        Some(Self {
            hp_fraction: clamp_ratio(me.hp / me.definition.health.max(1e-3), 1.0),
            reload_remaining: if reload > 1e-3 {
                clamp_ratio(me.reload_remaining(world.time) / reload, 1.0)
            } else {
                0.0
            },
            force,
            platoon_power: ratio_pair(power_ours, power_theirs),
            commit: commitment(power_ours, power_theirs),
            speed_rank: rank(me.definition.movement.max_speed, best_speed),
            range_rank: rank(me.primary_range(), best_range),
            hp_rank: rank(me.definition.health, best_health),
            enemy_centroid,
            front_alignment: finite(front_alignment),
            side: finite(side),
            gap_ccw,
            gap_cw,
            ring_distance_over_range,
            bloom_tangent,
            ally_centroid,
            ally_centroid_distance_over_range: clamp_ratio(centroid_distance / range, 3.0),
            nearest_ally_distance_over_range: if nearest_ally.is_finite() {
                clamp_ratio(nearest_ally / range, 3.0)
            } else {
                0.0
            },
            candidates,
            allies_engaged,
            enemy_objective: enemy_objective.map(|(d2, id, position)| (id, position, d2.sqrt())),
        })
    }

    /// `(ours/theirs, theirs/ours)` for one of the local force statistics.
    fn pair(&self, stat: usize) -> (f32, f32) {
        (self.force[2 * stat], self.force[2 * stat + 1])
    }

    fn target_input(&self, candidate: &Candidate, rank: usize) -> [f32; 6] {
        [
            candidate.distance_over_range,
            candidate.hp_fraction,
            candidate.range_over_mine,
            candidate.threat_over_mine,
            candidate.value_over_mine,
            if rank == 0 { 1.0 } else { 0.0 },
        ]
    }

    fn spacing_input(&self, target: &Candidate) -> [f32; 11] {
        let (ours_dps, theirs_dps) = self.pair(STAT_DPS);
        [
            target.distance_over_range,
            self.reload_remaining,
            self.hp_fraction,
            target.range_over_mine,
            target.hp_fraction,
            target.approaching,
            ours_dps,
            theirs_dps,
            self.range_rank,
            self.platoon_power.0,
            self.platoon_power.1,
        ]
    }

    fn strafe_input(&self, target: &Candidate) -> [f32; 8] {
        let (ours_dps, theirs_dps) = self.pair(STAT_DPS);
        [
            target.distance_over_range,
            self.reload_remaining,
            target.approaching,
            target.range_over_mine,
            self.hp_fraction,
            ours_dps,
            theirs_dps,
            self.speed_rank,
        ]
    }

    fn cohesion_input(&self, target: &Candidate) -> [f32; 5] {
        [
            self.ally_centroid_distance_over_range,
            self.nearest_ally_distance_over_range,
            self.hp_fraction,
            self.pair(STAT_COUNT).0,
            target.distance_over_range,
        ]
    }

    fn engagement_input(&self) -> [f32; 22] {
        let mut input = [0.0; 22];
        input[..2 * FORCE_STATS.len()].copy_from_slice(&self.force);
        input[16] = self.hp_fraction;
        input[17] = self.speed_rank;
        input[18] = self.range_rank;
        input[19] = self.hp_rank;
        input[20] = self.platoon_power.0;
        input[21] = self.platoon_power.1;
        input
    }

    fn encircle_input(&self) -> [f32; 5] {
        [
            self.front_alignment,
            self.side,
            self.gap_ccw,
            self.gap_cw,
            self.ring_distance_over_range,
        ]
    }
}

// --- Explanation records -----------------------------------------------------

/// One run of one module with everything labelled.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModuleTrace {
    pub module: String,
    /// Which candidate this run scored (target selector only).
    pub label: Option<String>,
    /// Unit id of that candidate, for tools that need more than the label.
    pub candidate: Option<UnitId>,
    pub inputs: Vec<(String, f32)>,
    /// Hidden-layer activations, first hidden layer first.
    pub hidden: Vec<Vec<f32>>,
    pub outputs: Vec<(String, f32)>,
}

/// Why a unit did what it did on one tick.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionTrace {
    pub unit: UnitId,
    pub time: f32,
    pub position: [f32; 2],
    pub chosen_target: Option<UnitId>,
    /// Target selector runs (one per candidate), then spacing, strafe,
    /// cohesion, engagement, encircle.
    pub modules: Vec<ModuleTrace>,
    pub advance: f32,
    pub orbit: f32,
    pub regroup: f32,
    pub retreat: f32,
    pub bloom: f32,
    /// How clearly the whole platoon outguns the enemy side (0..1). Above
    /// zero it suppresses retreat and pulls units out of contact back in.
    pub commit: f32,
    /// The final world-frame movement vector after the formula and doctrine.
    pub movement: [f32; 2],
    pub throttle: f32,
}

// --- Policy ------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TacticalPolicy {
    brain: Brain,
    air: Option<super::air::AirPolicy>,
    target: usize,
    spacing: usize,
    strafe: usize,
    cohesion: usize,
    engagement: usize,
    encircle: usize,
    mission: Option<usize>,
}

impl TacticalPolicy {
    pub fn new(brain: Brain) -> Result<Self, BrainError> {
        if brain.kind == super::air::KIND {
            return Ok(Self {
                air: Some(super::air::AirPolicy::new(brain.clone())?),
                brain,
                target: 0,
                spacing: 0,
                strafe: 0,
                cohesion: 0,
                engagement: 0,
                encircle: 0,
                mission: None,
            });
        }
        // Hidden width is free; the labelled inputs/outputs are not.
        brain.check_against(&brain.kind, &specs_for_kind(&brain.kind, 1)?)?;
        Ok(Self {
            air: None,
            target: brain.module_index(TARGET_SELECTOR)?,
            spacing: brain.module_index(SPACING)?,
            strafe: brain.module_index(STRAFE)?,
            cohesion: brain.module_index(COHESION)?,
            engagement: brain.module_index(ENGAGEMENT)?,
            encircle: brain.module_index(ENCIRCLE)?,
            mission: if brain.kind == KIND_V4 {
                Some(brain.module_index(MISSION)?)
            } else {
                None
            },
            brain,
        })
    }

    pub fn brain(&self) -> &Brain {
        &self.brain
    }

    /// The command plus a full account of how it was reached.
    pub fn explain(
        &self,
        world: &WorldView<'_>,
        unit: &UnitState,
    ) -> (Command, Option<DecisionTrace>) {
        if let Some(air) = &self.air {
            return air.explain(world, unit);
        }
        let mut trace = DecisionTrace {
            unit: unit.id,
            time: world.time,
            position: [unit.position.x, unit.position.z],
            chosen_target: None,
            modules: Vec::new(),
            advance: 0.0,
            orbit: 0.0,
            regroup: 0.0,
            retreat: 0.0,
            bloom: 0.0,
            commit: 0.0,
            movement: [0.0, 0.0],
            throttle: 0.0,
        };
        let command = self.think(world, unit, Some(&mut trace));
        if trace.chosen_target.is_some() || !trace.modules.is_empty() {
            (command, Some(trace))
        } else {
            (command, None)
        }
    }

    fn run(
        &self,
        module: usize,
        input: &[f32],
        output: &mut [f32],
        candidate: Option<(UnitId, usize)>,
        trace: &mut Option<&mut DecisionTrace>,
    ) {
        let module = &self.brain.modules[module];
        match trace {
            None => module.net.forward(input, output),
            Some(record) => {
                let layers = module.net.trace(input).layers;
                output.copy_from_slice(layers.last().expect("output layer"));
                record.modules.push(ModuleTrace {
                    module: module.spec.name.clone(),
                    label: candidate.map(|(id, rank)| format!("enemy #{id} (rank {rank})")),
                    candidate: candidate.map(|(id, _)| id),
                    inputs: module
                        .spec
                        .inputs
                        .iter()
                        .cloned()
                        .zip(input.iter().copied())
                        .collect(),
                    hidden: layers[1..layers.len() - 1].to_vec(),
                    outputs: module
                        .spec
                        .outputs
                        .iter()
                        .cloned()
                        .zip(output.iter().copied())
                        .collect(),
                });
            }
        }
    }

    fn think_v4(
        &self,
        world: &WorldView<'_>,
        unit: &UnitState,
        mut trace: Option<&mut DecisionTrace>,
    ) -> Command {
        let Some(mut situation) = Situation::observe(world, unit) else {
            return crate::policy::reconnaissance(world, unit);
        };
        let mut candidates = [None; TARGET_CANDIDATES + 1];
        for (slot, candidate) in candidates.iter_mut().zip(situation.candidates) {
            *slot = candidate;
        }
        let enemy_key = world
            .units
            .iter()
            .filter(|other| {
                other.alive
                    && other.side < 2
                    && other.side != unit.side
                    && other.definition.strategic.key_unit
                    && eligible_weapon(unit, other).is_some()
            })
            .min_by(|a, b| {
                unit.position
                    .distance_squared(a.position)
                    .total_cmp(&unit.position.distance_squared(b.position))
            });
        if let Some(key) = enemy_key {
            if !candidates
                .iter()
                .flatten()
                .any(|candidate| candidate.id == key.id)
            {
                candidates[TARGET_CANDIDATES] = Some(Candidate {
                    id: key.id,
                    position: key.position,
                    distance_over_range: 0.0,
                    hp_fraction: clamp_ratio(key.hp / key.definition.health.max(1e-3), 1.0),
                    range_over_mine: 0.0,
                    threat_over_mine: 0.0,
                    value_over_mine: clamp_ratio(
                        key.definition.resource_value()
                            / unit.definition.resource_value().max(1e-3),
                        4.0,
                    ),
                    approaching: 0.0,
                });
            }
        }
        let mut selected = None;
        let mut first_threat = None;
        for (rank, held) in candidates.iter_mut().enumerate() {
            let Some(candidate) = held else {
                continue;
            };
            let Some(enemy) = world.unit(candidate.id) else {
                continue;
            };
            let mount = eligible_weapon(unit, enemy);
            let range = mount.map_or(FALLBACK_RANGE, |index| {
                unit.definition.weapons[index].range.max(1.0)
            });
            let incoming = eligible_weapon(enemy, unit);
            candidate.distance_over_range =
                clamp_ratio(unit.position.distance(enemy.position) / range, 3.0);
            candidate.range_over_mine = clamp_ratio(
                incoming.map_or(0.0, |i| enemy.definition.weapons[i].range) / range,
                3.0,
            );
            let own_dps = eligible_damage_rate(unit, enemy);
            let enemy_dps = eligible_damage_rate(enemy, unit);
            candidate.threat_over_mine = clamp_ratio(enemy_dps / own_dps.max(1e-3), 4.0);
            let sum_speed = (unit.definition.movement.max_speed
                + enemy.definition.movement.max_speed)
                .max(1e-3);
            let speed_balance = ((unit.definition.movement.max_speed
                - enemy.definition.movement.max_speed)
                / sum_speed)
                .clamp(-1.0, 1.0);
            let toward = (enemy.position - unit.position).normalized();
            let approaching = dot(enemy.velocity(), toward * -1.0)
                / enemy.definition.movement.max_speed.max(1e-3);
            candidate.approaching = approaching.clamp(-1.0, 1.0);
            let mut input = [0.0; 10];
            input[..6].copy_from_slice(&situation.target_input(candidate, rank));
            input[6] = f32::from(mount.is_some());
            input[7] = f32::from(incoming.is_some());
            input[8] = f32::from(enemy.definition.strategic.key_unit);
            input[9] = speed_balance;
            let mut score = [0.0];
            self.run(
                self.target,
                &input,
                &mut score,
                Some((candidate.id, rank)),
                &mut trace,
            );
            first_threat.get_or_insert(*candidate);
            if mount.is_some() && selected.is_none_or(|(held, _)| finite(score[0]) > held) {
                selected = Some((finite(score[0]), *candidate));
            }
        }
        let Some(chosen) = selected.map(|(_, candidate)| candidate).or(first_threat) else {
            return Command::default();
        };
        let enemy = world.unit(chosen.id).expect("candidate is live");
        let mount = eligible_weapon(unit, enemy);
        let own_range = mount.map_or(FALLBACK_RANGE, |i| {
            unit.definition.weapons[i].range.max(1.0)
        });
        situation.reload_remaining = mount.map_or(0.0, |i| {
            clamp_ratio(
                (unit.weapons[i].reload_until - world.time).max(0.0)
                    / unit.definition.weapons[i].reload.max(1e-3),
                1.0,
            )
        });
        // Condition the damage balance on what can hit this target and this chassis.
        let radius_squared = (own_range * LOCAL_RADIUS_RANGES).powi(2);
        let (mut ours, mut theirs) = (0.0, 0.0);
        for other in world.units.iter().filter(|other| {
            other.alive
                && other.side < 2
                && other.position.distance_squared(unit.position) <= radius_squared
        }) {
            if other.side == unit.side {
                ours += eligible_damage_rate(other, enemy);
            } else {
                theirs += eligible_damage_rate(other, unit);
            }
        }
        let pair = ratio_pair(ours, theirs);
        situation.force[2 * STAT_DPS] = pair.0;
        situation.force[2 * STAT_DPS + 1] = pair.1;
        // The v3 whole-side doctrine cannot cancel retreat from an unanswerable threat.
        situation.platoon_power = pair;
        situation.commit = 0.0;
        let speeds =
            (unit.definition.movement.max_speed + enemy.definition.movement.max_speed).max(1e-3);
        let toward = (chosen.position - unit.position).normalized();
        let closing = dot(unit.velocity() - enemy.velocity(), toward) / speeds;
        let speed_balance =
            ((unit.definition.movement.max_speed - enemy.definition.movement.max_speed) / speeds)
                .clamp(-1.0, 1.0);
        let mut spacing = [0.0; 14];
        spacing[..11].copy_from_slice(&situation.spacing_input(&chosen));
        spacing[11] = speed_balance;
        spacing[12] = clamp_ratio(
            (unit.position.distance(enemy.position) - chosen.range_over_mine * own_range).max(0.0)
                / (enemy.definition.movement.max_speed.max(1.0) * 10.0),
            1.0,
        );
        spacing[13] = closing.clamp(-1.0, 1.0);
        let (mut advance, mut orbit, mut regroup, mut retreat, mut bloom) =
            ([0.0], [0.0], [0.0], [0.0], [0.0]);
        self.run(self.spacing, &spacing, &mut advance, None, &mut trace);
        self.run(
            self.strafe,
            &situation.strafe_input(&chosen),
            &mut orbit,
            None,
            &mut trace,
        );
        self.run(
            self.engagement,
            &situation.engagement_input(),
            &mut retreat,
            None,
            &mut trace,
        );
        if situation.ally_centroid.is_some() {
            self.run(
                self.cohesion,
                &situation.cohesion_input(&chosen),
                &mut regroup,
                None,
                &mut trace,
            );
        }
        if situation.bloom_tangent.is_some() {
            self.run(
                self.encircle,
                &situation.encircle_input(),
                &mut bloom,
                None,
                &mut trace,
            );
        }
        let own_key = world
            .units
            .iter()
            .filter(|other| {
                other.alive
                    && other.side == unit.side
                    && other.definition.strategic.key_unit
                    && other.id != unit.id
            })
            .min_by(|a, b| {
                unit.position
                    .distance_squared(a.position)
                    .total_cmp(&unit.position.distance_squared(b.position))
            });
        let threat = own_key.and_then(|asset| {
            world
                .units
                .iter()
                .filter(|other| {
                    other.alive
                        && other.side < 2
                        && other.side != unit.side
                        && eligible_weapon(other, asset).is_some()
                })
                .min_by(|a, b| {
                    asset
                        .position
                        .distance_squared(a.position)
                        .total_cmp(&asset.position.distance_squared(b.position))
                })
        });
        let mut mission = [0.0; 14];
        mission[0] = f32::from(own_key.is_some());
        mission[1] = f32::from(enemy_key.is_some());
        if let Some(asset) = own_key {
            mission[2] = clamp_ratio(unit.position.distance(asset.position) / own_range, 3.0);
            mission[4] = clamp_ratio(asset.hp / asset.definition.health.max(1e-3), 1.0);
            if let Some(hostile) = threat {
                let distance = asset.position.distance(hostile.position);
                mission[7] = clamp_ratio(distance / own_range, 3.0);
                mission[8] = clamp_ratio(
                    distance / (hostile.definition.movement.max_speed.max(1.0) * 30.0),
                    1.0,
                );
                let enemy_range = eligible_weapon(hostile, asset)
                    .map_or(0.0, |i| hostile.definition.weapons[i].range);
                mission[13] = (1.0
                    - (distance - enemy_range).max(0.0)
                        / (hostile.definition.movement.max_speed.max(1.0) * 10.0))
                    .clamp(0.0, 1.0);
            }
        }
        if let Some(asset) = enemy_key {
            let distance = unit.position.distance(asset.position);
            mission[3] = clamp_ratio(distance / own_range, 3.0);
            mission[5] = clamp_ratio(asset.hp / asset.definition.health.max(1e-3), 1.0);
            mission[6] = clamp_ratio(
                distance / (unit.definition.movement.max_speed.max(1.0) * 30.0),
                1.0,
            );
            mission[9] = 1.0;
            mission[12] =
                dot(toward, (asset.position - unit.position).normalized()).clamp(-1.0, 1.0);
        }
        mission[10] = (ours - theirs) / (ours + theirs).max(1e-3);
        mission[11] = situation.hp_fraction;
        let mut drive = [0.0];
        self.run(
            self.mission.expect("v4 mission module"),
            &mission,
            &mut drive,
            None,
            &mut trace,
        );
        let (advance, orbit, regroup, retreat, bloom) = (
            finite(advance[0]),
            finite(orbit[0]),
            finite(regroup[0]),
            finite(retreat[0]).clamp(0.0, 1.0),
            finite(bloom[0]),
        );
        let mut fight = toward * advance + Vec2::new(-toward.z, toward.x) * orbit;
        if let Some(centre) = situation.ally_centroid {
            fight = fight + (centre - unit.position).normalized() * regroup;
        }
        if let Some(tangent) = situation.bloom_tangent {
            fight = fight + tangent * (bloom * BLOOM_BASE_GATE);
        }
        let away =
            (unit.position - situation.enemy_centroid.unwrap_or(chosen.position)).normalized();
        let mut movement = fight * (1.0 - retreat) + away * retreat;
        let drive = finite(drive[0]).clamp(-1.0, 1.0);
        let objective_direction = if drive >= 0.0 {
            enemy_key.map(|asset| (asset.position - unit.position).normalized())
        } else {
            own_key.map(|asset| {
                let at = threat.map_or(asset.position, |hostile| {
                    asset.position + (hostile.position - asset.position).normalized() * own_range
                });
                (at - unit.position).normalized()
            })
        };
        if let Some(direction) = objective_direction {
            let amount = drive.abs();
            movement = movement * (1.0 - amount) + direction * amount;
        }
        let strength = movement.length();
        let throttle = if strength < MOVE_DEADZONE {
            0.0
        } else {
            strength.min(1.0)
        };
        let target = selected.map(|(_, candidate)| candidate.id);
        if let Some(record) = trace {
            record.chosen_target = target;
            record.advance = advance;
            record.orbit = orbit;
            record.regroup = regroup;
            record.retreat = retreat;
            record.bloom = bloom;
            record.movement = [movement.x, movement.z];
            record.throttle = throttle;
        }
        Command {
            target,
            destination: (throttle > 0.0)
                .then(|| unit.position + movement.normalized() * MOVE_LOOKAHEAD),
            throttle,
        }
    }

    fn think(
        &self,
        world: &WorldView<'_>,
        unit: &UnitState,
        mut trace: Option<&mut DecisionTrace>,
    ) -> Command {
        if let Some(air) = &self.air {
            return air.command(world, unit);
        }
        if unit.definition.flight.is_some() {
            return Command::default();
        }
        if self.mission.is_some() {
            return self.think_v4(world, unit, trace);
        }
        let Some(situation) = Situation::observe(world, unit) else {
            return crate::policy::reconnaissance(world, unit);
        };

        // 1. Who? Score each candidate with the same weights; strict `>` keeps
        //    the nearer enemy on ties.
        let mut best: Option<(f32, usize)> = None;
        for (rank, candidate) in situation.candidates.iter().enumerate() {
            let Some(candidate) = candidate else { continue };
            let mut score = [0.0f32];
            self.run(
                self.target,
                &situation.target_input(candidate, rank),
                &mut score,
                Some((candidate.id, rank)),
                &mut trace,
            );
            let score = finite(score[0]);
            if best.is_none_or(|(held, _)| score > held) {
                best = Some((score, rank));
            }
        }
        let chosen = situation.candidates[best.expect("observe guarantees one candidate").1]
            .expect("chosen rank is populated");

        // 2. How to move relative to them, whether to stay in, and where to stand.
        let mut advance = [0.0f32];
        let mut orbit = [0.0f32];
        let mut regroup = [0.0f32];
        let mut retreat = [0.0f32];
        let mut bloom = [0.0f32];
        self.run(
            self.spacing,
            &situation.spacing_input(&chosen),
            &mut advance,
            None,
            &mut trace,
        );
        self.run(
            self.strafe,
            &situation.strafe_input(&chosen),
            &mut orbit,
            None,
            &mut trace,
        );
        if situation.ally_centroid.is_some() {
            self.run(
                self.cohesion,
                &situation.cohesion_input(&chosen),
                &mut regroup,
                None,
                &mut trace,
            );
        }
        self.run(
            self.engagement,
            &situation.engagement_input(),
            &mut retreat,
            None,
            &mut trace,
        );
        if situation.bloom_tangent.is_some() {
            self.run(
                self.encircle,
                &situation.encircle_input(),
                &mut bloom,
                None,
                &mut trace,
            );
        }
        let (advance, orbit, regroup, retreat, bloom) = (
            finite(advance[0]),
            finite(orbit[0]),
            finite(regroup[0]),
            finite(retreat[0]),
            finite(bloom[0]),
        );

        // 3. Fixed, readable composition.
        let toward = (chosen.position - unit.position).normalized();
        let tangent = Vec2::new(-toward.z, toward.x);
        let mut fight = toward * advance + tangent * orbit;
        if let Some(centroid) = situation.ally_centroid {
            fight = fight + (centroid - unit.position).normalized() * regroup;
        }
        if let Some(around) = situation.bloom_tangent {
            let gate = BLOOM_BASE_GATE + (1.0 - BLOOM_BASE_GATE) * situation.commit;
            fight = fight + around * (bloom * gate);
        }
        // Run from the local enemy force, or from the target if none is near. A
        // platoon that is clearly ahead does not retreat.
        let flee_from = situation.enemy_centroid.unwrap_or(chosen.position);
        let away = (unit.position - flee_from).normalized();
        let flee = retreat.clamp(0.0, 1.0) * (1.0 - situation.commit);
        let mut movement = fight * (1.0 - flee) + away * flee;
        // Doctrine: a unit out of contact rejoins when its platoon is ahead, and
        // also whenever a friend is already in weapon range of its target: it
        // never sits out a fight its fellows are in.
        let drive = if situation.allies_engaged > 0 {
            situation.commit.max(ENGAGED_ALLY_DRIVE)
        } else {
            situation.commit
        };
        let pull = drive * ((chosen.distance_over_range - PULL_START) / PULL_SPAN).clamp(0.0, 1.0);
        if pull > 0.0 {
            movement = movement * (1.0 - pull) + toward * pull;
        }
        // Doctrine: objectives. An attacker drives for the enemy's key unit and
        // shoots it once in reach (destroying it wins, whatever else stands).
        // Defenders get no rule: a leash was tried and removed (see
        // docs/FINDINGS.md); the fight belongs where the enemy is, which can be
        // beyond any fixed radius when the enemy outranges the target.
        let range = match unit.primary_range() {
            r if r > 1.0 => r,
            _ => FALLBACK_RANGE,
        };
        let mut target = chosen.id;
        if let Some((id, position, distance)) = situation.enemy_objective {
            if distance <= range * OBJECTIVE_REACH {
                target = id;
            } else {
                let strike = OBJECTIVE_BASE_PULL + (1.0 - OBJECTIVE_BASE_PULL) * situation.commit;
                movement =
                    movement * (1.0 - strike) + (position - unit.position).normalized() * strike;
            }
        }
        let strength = movement.length();
        let throttle = strength.min(1.0);

        if let Some(record) = trace {
            record.chosen_target = Some(target);
            record.advance = advance;
            record.orbit = orbit;
            record.regroup = regroup;
            record.retreat = retreat;
            record.bloom = bloom;
            record.commit = situation.commit;
            record.movement = [movement.x, movement.z];
            record.throttle = if strength < MOVE_DEADZONE {
                0.0
            } else {
                throttle
            };
        }

        if strength < MOVE_DEADZONE {
            Command {
                target: Some(target),
                destination: None,
                throttle: 0.0,
            }
        } else {
            Command {
                target: Some(target),
                destination: Some(unit.position + movement.normalized() * MOVE_LOOKAHEAD),
                throttle,
            }
        }
    }
}

impl Policy for TacticalPolicy {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        self.think(world, unit, None)
    }
}

// --- The authored starting council ----------------------------------------------

/// Edits one single-hidden-layer module by input name, so a starting council
/// can be written down as the rules it encodes rather than as numbers.
struct Wiring<'a> {
    module: &'a mut Module,
}

impl Wiring<'_> {
    fn dims(&self) -> (usize, usize) {
        (self.module.spec.inputs.len(), self.module.spec.hidden[0])
    }

    /// Hidden unit `unit` adds `weight * input`.
    fn reads(&mut self, unit: usize, input: &str, weight: f32) -> &mut Self {
        let (inputs, hidden) = self.dims();
        assert!(
            unit < hidden,
            "authored unit {unit} needs hidden >= {}",
            unit + 1
        );
        let column = self
            .module
            .spec
            .inputs
            .iter()
            .position(|name| name == input)
            .unwrap_or_else(|| panic!("module {} has no input {input}", self.module.spec.name));
        self.module.net.params_mut()[unit * inputs + column] = weight;
        self
    }

    fn bias(&mut self, unit: usize, bias: f32) -> &mut Self {
        let (inputs, hidden) = self.dims();
        assert!(unit < hidden);
        self.module.net.params_mut()[hidden * inputs + unit] = bias;
        self
    }

    /// The (single) output's own bias.
    fn output_bias(&mut self, bias: f32) -> &mut Self {
        let (inputs, hidden) = self.dims();
        self.module.net.params_mut()[hidden * inputs + hidden + hidden] = bias;
        self
    }

    /// The output adds `weight * hidden[unit]`.
    fn feeds_output(&mut self, unit: usize, weight: f32) -> &mut Self {
        let (inputs, hidden) = self.dims();
        assert!(unit < hidden);
        self.module.net.params_mut()[hidden * inputs + hidden + unit] = weight;
        self
    }
}

/// A hand-written council that already behaves sensibly, so training starts
/// from "fights well" instead of from noise and refines it. Every rule is a
/// few weights on named inputs:
///
/// * `target_selector`: prefer near, wounded and dangerous enemies.
/// * `spacing`: close to about 0.85 weapon range, and kite (back away while
///   reloading).
/// * `engagement`: retreat only when the platoon is hopelessly outgunned.
/// * `encircle`: slide toward the larger gap and away from the enemy's front.
/// * `strafe`, `cohesion`: silent; left for training to discover.
///
/// `hidden` must be at least 3.
pub fn template(hidden: usize) -> Result<Brain, BrainError> {
    let mut brain = Brain::random(KIND, specs(hidden), &mut Rng::new(0), 0.0)?;
    for module in &mut brain.modules {
        module.net.params_mut().fill(0.0);
    }
    for module in &mut brain.modules {
        let mut wiring = Wiring { module };
        match wiring.module.spec.name.as_str() {
            TARGET_SELECTOR => {
                wiring
                    .reads(0, "distance_over_range", -0.9)
                    .reads(0, "enemy_hp_fraction", -0.7)
                    .reads(0, "enemy_threat_over_mine", 0.3)
                    .reads(0, "is_nearest", 0.6)
                    .feeds_output(0, 1.0);
            }
            SPACING => {
                // Hold near 0.85 of weapon range: unit 0 is positive when too
                // far and negative when too close.
                wiring
                    .reads(0, "distance_over_range", 3.0)
                    .bias(0, -2.55)
                    .feeds_output(0, 1.2);
                // Kite: back away while the gun reloads, the sooner the closer
                // the enemy is (active when reload_remaining > distance - 0.5).
                // Standing still loses to a circle-strafer; kiting beats it.
                // A tanh unit that is not active sits at -1, not 0, so the
                // output bias cancels that resting contribution.
                wiring
                    .reads(1, "my_reload_remaining", 40.0)
                    .reads(1, "distance_over_range", -40.0)
                    .bias(1, 20.0)
                    .feeds_output(1, -1.2)
                    .output_bias(-1.2);
            }
            ENGAGEMENT => {
                // retreat > 0 only when theirs/ours greatly exceeds ours/theirs.
                wiring
                    .reads(0, "platoon_ours_over_theirs_power", -4.0)
                    .reads(0, "platoon_theirs_over_ours_power", 4.0)
                    .bias(0, -2.8)
                    .feeds_output(0, 1.5);
            }
            ENCIRCLE => {
                // Toward the bigger gap (units 0), away from the front (unit 1).
                wiring
                    .reads(0, "gap_ccw", 8.0)
                    .reads(0, "gap_cw", -8.0)
                    .reads(1, "enemy_side", 2.5)
                    .feeds_output(0, 1.2)
                    .feeds_output(1, 0.8);
            }
            _ => {}
        }
    }
    Ok(brain)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::{Scenario, Simulator};
    use crate::learn::rng::Rng;
    use crate::model::{MovementDef, Spawn, StrategicDef, Terrain, UnitDef, WeaponDef};
    use std::sync::Mutex;

    pub(crate) fn skirmisher(name: &str) -> UnitDef {
        UnitDef {
            name: name.into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 50.0,
            energy_cost: 0.0,
            strategic: StrategicDef::default(),
            flight: None,
            movement: MovementDef {
                max_speed: 87.0,
                acceleration: 372.6,
                braking: 621.0,
                turn_rate: 194.3,
                radius: 11.0,
                ..Default::default()
            },
            weapons: vec![WeaponDef {
                name: "gun".into(),
                damage: 20.0,
                range: 180.0,
                reload: 1.0,
                hitscan: true,
                turret_yaw_speed: Some(300.0),
                ..Default::default()
            }],
        }
    }

    fn spawn(side: u8, x: f32, z: f32) -> Spawn {
        Spawn {
            side,
            definition: skirmisher(if side == 0 { "a" } else { "b" }),
            position: Vec2::new(x, z),
            heading: if side == 0 { 0.0 } else { std::f32::consts::PI },
        }
    }

    fn scenario(units: Vec<Spawn>) -> Scenario {
        Scenario {
            terrain: Terrain::flat(2_000.0, 1_000.0, 32.0),
            units,
            max_time: 0.1,
            trace_resources: vec![],
        }
    }

    /// Records the first command (and situation) each unit produces.
    struct Probe<'a> {
        inner: &'a dyn Policy,
        first: Mutex<Vec<(UnitId, Command, Option<Situation>)>>,
    }

    impl Policy for Probe<'_> {
        fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
            let command = self.inner.command(world, unit);
            let mut first = self.first.lock().unwrap();
            if !first.iter().any(|(id, ..)| *id == unit.id) {
                first.push((unit.id, command, Situation::observe(world, unit)));
            }
            command
        }
    }

    /// Spacing, strafe and engagement output constants; the rest are silent.
    fn constant_brain(advance: f32, orbit: f32) -> Brain {
        constant_brain_with(advance, orbit, 0.0)
    }

    fn constant_brain_with(advance: f32, orbit: f32, retreat: f32) -> Brain {
        constant_brain_full(advance, orbit, retreat, 0.0)
    }

    fn constant_brain_full(advance: f32, orbit: f32, retreat: f32, bloom: f32) -> Brain {
        let mut brain = Brain::random(KIND, specs(4), &mut Rng::new(1), 1.0).unwrap();
        for module in &mut brain.modules {
            let bias = match module.spec.name.as_str() {
                SPACING => atanh(advance),
                STRAFE => atanh(orbit),
                ENGAGEMENT => atanh(retreat),
                ENCIRCLE => atanh(bloom),
                _ => 0.0,
            };
            let params = module.net.params_mut();
            params.fill(0.0);
            // Last parameter is the output bias.
            *params.last_mut().unwrap() = bias;
        }
        brain
    }

    fn atanh(x: f32) -> f32 {
        0.5 * ((1.0 + x) / (1.0 - x)).ln()
    }

    fn first_commands(
        brain: Brain,
        units: Vec<Spawn>,
    ) -> Vec<(UnitId, Command, Option<Situation>)> {
        let policy = TacticalPolicy::new(brain).unwrap();
        let probe = Probe {
            inner: &policy,
            first: Mutex::new(Vec::new()),
        };
        let hold = crate::policy::BuiltinPolicy::Hold;
        Simulator::new(scenario(units)).run([&probe, &hold], 1);
        let mut first = probe.first.into_inner().unwrap();
        first.sort_by_key(|(id, ..)| *id);
        first
    }

    #[test]
    fn positive_advance_heads_for_the_target_and_negative_retreats() {
        let units = vec![spawn(0, 800.0, 500.0), spawn(1, 1_000.0, 500.0)];
        let forward = first_commands(constant_brain(0.9, 0.0), units.clone());
        let (_, command, _) = &forward[0];
        let destination = command.destination.expect("moves");
        assert!(destination.x > 800.0 && (destination.z - 500.0).abs() < 1e-3);
        assert!((command.throttle - 0.9).abs() < 1e-3);

        let backward = first_commands(constant_brain(-0.9, 0.0), units);
        assert!(backward[0].1.destination.unwrap().x < 800.0);
    }

    #[test]
    fn orbit_moves_perpendicular_to_the_target_line() {
        let units = vec![spawn(0, 800.0, 500.0), spawn(1, 1_000.0, 500.0)];
        let result = first_commands(constant_brain(0.0, 0.8), units);
        let destination = result[0].1.destination.unwrap();
        assert!((destination.x - 800.0).abs() < 1e-2, "{destination:?}");
        assert!(destination.z > 500.0);
    }

    #[test]
    fn silent_modules_mean_stand_and_shoot_the_nearest_enemy() {
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn(1, 1_300.0, 500.0),
            spawn(1, 900.0, 520.0),
        ];
        let result = first_commands(constant_brain(0.0, 0.0), units);
        let (_, command, _) = &result[0];
        assert!(command.destination.is_none());
        assert_eq!(command.throttle, 0.0);
        // Ties keep the nearest enemy (unit ids are 1-based, in spawn order; the enemy at x=900 is the third spawn).
        assert_eq!(command.target, Some(3));
    }

    #[test]
    fn situation_reports_labelled_geometry_in_range_units() {
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn(0, 800.0, 600.0),
            spawn(1, 980.0, 500.0), // exactly one weapon range away
            spawn(1, 1_700.0, 500.0),
        ];
        let result = first_commands(constant_brain(0.0, 0.0), units);
        let situation = result[0].2.clone().expect("has enemies");
        let nearest = situation.candidates[0].unwrap();
        assert!((nearest.distance_over_range - 1.0).abs() < 1e-3);
        assert!((situation.nearest_ally_distance_over_range - 100.0 / 180.0).abs() < 1e-3);
        // Local radius is 3 weapon ranges (540): we and our ally (2) face only
        // the enemy at 180 away; the one at 900 is outside it.
        assert_eq!(situation.pair(STAT_COUNT), (1.0, 0.5));
        assert_eq!(situation.pair(STAT_DPS), (1.0, 0.5));
        // Third candidate slot is empty with only two enemies.
        assert!(situation.candidates[2].is_none());
    }

    #[test]
    fn ratio_pairs_show_the_balance_from_both_sides() {
        assert_eq!(ratio_pair(0.0, 0.0), (0.0, 0.0));
        assert_eq!(ratio_pair(5.0, 0.0), (1.0, 0.0));
        assert_eq!(ratio_pair(0.0, 5.0), (0.0, 1.0));
        assert_eq!(ratio_pair(2.0, 4.0), (0.5, 1.0));
        assert_eq!(ratio_pair(4.0, 2.0), (1.0, 0.5));
        assert_eq!(ratio_pair(3.0, 3.0), (1.0, 1.0));
        assert_eq!(ratio_pair(f32::NAN, 1.0), (0.0, 1.0));
    }

    fn with_range(mut definition: UnitDef, range: f32) -> UnitDef {
        definition.weapons[0].range = range;
        definition
    }

    fn spawn_def(side: u8, x: f32, z: f32, definition: UnitDef) -> Spawn {
        Spawn {
            definition,
            ..spawn(side, x, z)
        }
    }

    #[test]
    fn dps_is_split_by_range_band_relative_to_the_units_own_range() {
        // We have a 180-range gun. One enemy has a 400-range gun (2.2x: long
        // band) and one has a 100-range gun (0.56x: short band).
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn_def(1, 1_000.0, 500.0, with_range(skirmisher("long"), 400.0)),
            spawn_def(1, 1_000.0, 540.0, with_range(skirmisher("short"), 100.0)),
        ];
        let situation = first_commands(constant_brain(0.0, 0.0), units)[0]
            .2
            .clone()
            .unwrap();
        let band = |name: &str| {
            let stat = FORCE_STATS.iter().position(|s| *s == name).unwrap();
            situation.pair(stat)
        };
        // Our own gun is mid band; neither enemy has one.
        assert_eq!(band("dps_mid"), (1.0, 0.0));
        assert_eq!(band("dps_long"), (0.0, 1.0));
        assert_eq!(band("dps_short"), (0.0, 1.0));
    }

    #[test]
    fn role_ranks_compare_a_unit_with_the_best_friendly_combatant() {
        let mut fast = skirmisher("fast");
        fast.movement.max_speed = 174.0;
        let mut tank = skirmisher("tank");
        tank.health = 400.0;
        // Far apart on purpose: ranks use the whole side, not the local radius.
        let units = vec![
            spawn(0, 200.0, 500.0),
            spawn_def(0, 1_800.0, 500.0, fast),
            spawn_def(0, 1_800.0, 700.0, tank),
            spawn(1, 400.0, 500.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), units);
        let me = rows[0].2.clone().unwrap();
        assert!((me.speed_rank - 0.5).abs() < 1e-6, "{}", me.speed_rank);
        assert!((me.hp_rank - 0.25).abs() < 1e-6, "{}", me.hp_rank);
        assert_eq!(me.range_rank, 1.0);
        // The fast unit is the best at speed.
        assert_eq!(rows[1].2.clone().unwrap().speed_rank, 1.0);
    }

    #[test]
    fn retreat_blends_the_fight_toward_running_from_the_enemy_force() {
        let units = vec![spawn(0, 800.0, 500.0), spawn(1, 1_000.0, 500.0)];

        // Full retreat: run directly away from the enemy at 0.9 throttle.
        let flee = first_commands(constant_brain_with(0.0, 0.0, 0.9), units.clone());
        let command = &flee[0].1;
        assert!(command.destination.unwrap().x < 800.0);
        assert!(
            (command.throttle - 0.9).abs() < 1e-3,
            "{}",
            command.throttle
        );

        // Non-positive retreat means fight: the advance output rules.
        let fight = first_commands(constant_brain_with(0.9, 0.0, -0.9), units.clone());
        assert!(fight[0].1.destination.unwrap().x > 800.0);

        // A partial retreat that outweighs a modest advance still runs, and
        // slower than a full retreat: 0.2*0.9 toward - 0.8 away = 0.62 away.
        let mixed = first_commands(constant_brain_with(0.9, 0.0, 0.8), units);
        let command = &mixed[0].1;
        assert!(command.destination.unwrap().x < 800.0);
        assert!(
            (command.throttle - 0.62).abs() < 1e-3,
            "{}",
            command.throttle
        );
    }

    #[test]
    fn retreat_runs_from_the_enemy_force_not_just_the_chosen_target() {
        // Two enemies close by, on opposite sides in z; the force centroid is
        // directly ahead, so fleeing is straight back along x.
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn(1, 1_000.0, 450.0),
            spawn(1, 1_000.0, 550.0),
        ];
        let result = first_commands(constant_brain_with(0.0, 0.0, 0.9), units);
        let destination = result[0].1.destination.unwrap();
        assert!(
            destination.x < 800.0 && (destination.z - 500.0).abs() < 1.0,
            "{destination:?}"
        );
    }

    #[test]
    fn commitment_rises_from_even_to_overwhelming() {
        assert_eq!(commitment(1.0, 1.0), 0.0);
        assert_eq!(commitment(1.5, 1.0), 0.0);
        assert!((commitment(2.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(commitment(2.5, 1.0), 1.0);
        assert_eq!(commitment(10.0, 1.0), 1.0);
        assert_eq!(commitment(1.0, 4.0), 0.0);
        assert_eq!(commitment(1.0, 0.0), 1.0); // nothing left to fight
        assert_eq!(commitment(0.0, 0.0), 0.0);
    }

    #[test]
    fn five_against_three_is_a_clear_advantage_and_four_against_three_is_not_yet() {
        let five_v_three = vec![
            spawn(0, 800.0, 400.0),
            spawn(0, 800.0, 450.0),
            spawn(0, 800.0, 500.0),
            spawn(0, 800.0, 550.0),
            spawn(0, 800.0, 600.0),
            spawn(1, 1_000.0, 450.0),
            spawn(1, 1_000.0, 500.0),
            spawn(1, 1_000.0, 550.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), five_v_three);
        assert!((rows[0].2.clone().unwrap().commit - 1.0).abs() < 1e-6);
        let four_v_three = vec![
            spawn(0, 800.0, 400.0),
            spawn(0, 800.0, 450.0),
            spawn(0, 800.0, 500.0),
            spawn(0, 800.0, 550.0),
            spawn(1, 1_000.0, 450.0),
            spawn(1, 1_000.0, 500.0),
            spawn(1, 1_000.0, 550.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), four_v_three);
        let commit = rows[0].2.clone().unwrap().commit;
        assert!(commit > 0.0 && commit < 0.5, "{commit}");
    }

    #[test]
    fn a_platoon_that_is_ahead_never_retreats_and_idle_units_rejoin() {
        // A brain that only wants to run: advance -0.9, retreat 0.9.
        let runner = || constant_brain_with(-0.9, 0.0, 0.9);

        // Three of ours (one far from the fight) against one enemy: clearly ahead.
        let units = vec![
            spawn(0, 300.0, 500.0), // far away
            spawn(0, 900.0, 480.0),
            spawn(0, 900.0, 520.0),
            spawn(1, 1_100.0, 500.0),
        ];
        let rows = first_commands(runner(), units);
        let far = &rows[0].1;
        assert!(far.destination.unwrap().x > 300.0, "{:?}", far.destination);
        assert!(
            far.throttle > 0.9,
            "rejoining at full speed, got {}",
            far.throttle
        );

        // The same brain with evenly matched forces does what it wants: flee.
        let even = vec![spawn(0, 300.0, 500.0), spawn(1, 1_100.0, 500.0)];
        let rows = first_commands(runner(), even);
        assert!(rows[0].1.destination.unwrap().x < 300.0);
    }

    #[test]
    fn a_unit_already_in_range_is_not_dragged_forward_by_the_doctrine() {
        // In range (100 away, 0.55 of range) and clearly ahead: retreat is
        // suppressed, so with advance 0 the unit simply holds and shoots.
        let units = vec![
            spawn(0, 900.0, 480.0),
            spawn(0, 900.0, 520.0),
            spawn(0, 900.0, 500.0),
            spawn(1, 1_000.0, 500.0),
        ];
        let rows = first_commands(constant_brain_with(0.0, 0.0, 0.9), units);
        assert!(rows[0].1.destination.is_none() && rows[0].1.throttle == 0.0);
    }

    #[test]
    fn a_unit_out_of_contact_joins_a_friend_who_is_already_fighting_even_when_behind() {
        // Two enemies against one of ours: the platoon is behind (commit 0). The
        // straggler 400 away would stand still with silent modules; because its
        // friend already has the target in weapon range, it heads in.
        let stand_alone = vec![
            spawn(0, 500.0, 500.0),
            spawn(1, 900.0, 500.0),
            spawn(1, 900.0, 540.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), stand_alone);
        assert!(rows[0].1.destination.is_none(), "nobody is fighting yet");
        let with_friend = vec![
            spawn(0, 500.0, 500.0),
            spawn(0, 800.0, 500.0),
            spawn(1, 900.0, 500.0),
            spawn(1, 900.0, 540.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), with_friend);
        let destination = rows[0].1.destination.expect("straggler heads in");
        assert!(destination.x > 600.0, "{destination:?}");
        assert_eq!(rows[0].2.as_ref().unwrap().commit, 0.0);
    }

    #[test]
    fn friends_are_given_angular_gaps_around_the_enemy_force() {
        // Hold-still enemy at (1000, 500). Its front is the side facing us, i.e.
        // toward our centroid. We stand at bearing 0 (straight in front); two
        // friends sit at +/-90 degrees. Three friends at 0, 90, -90 degrees: the
        // gaps either way round from the unit at 0 are a quarter circle each.
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn(0, 1_000.0, 300.0),
            spawn(0, 1_000.0, 700.0),
            spawn(1, 1_000.0, 500.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), units);
        let me = rows[0].2.clone().unwrap();
        assert!((me.gap_ccw - 0.25).abs() < 0.02, "{}", me.gap_ccw);
        assert!((me.gap_cw - 0.25).abs() < 0.02, "{}", me.gap_cw);
        // Our centroid is roughly in front of them, and we are right in it.
        assert!(me.front_alignment > 0.9, "{}", me.front_alignment);
        assert!(me.side.abs() < 0.2, "{}", me.side);

        // A neighbour hugging one side makes that gap small.
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn(0, 802.0, 540.0),
            spawn(1, 1_000.0, 500.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), units);
        let me = rows[0].2.clone().unwrap();
        assert!(me.gap_ccw.min(me.gap_cw) < 0.05);
        assert!(
            (me.gap_ccw + me.gap_cw - 1.0).abs() < 1e-3,
            "two friends split the circle"
        );
    }

    #[test]
    fn bloom_slides_the_unit_around_the_enemy_not_toward_it() {
        // Two of ours against one of theirs (clearly ahead, so the gate is open),
        // standing inside weapon range so the doctrine's pull stays out of it.
        let units = vec![
            spawn(0, 800.0, 500.0),
            spawn(0, 800.0, 560.0),
            spawn(1, 940.0, 500.0),
        ];
        let positive = first_commands(constant_brain_full(0.0, 0.0, 0.0, 0.8), units.clone());
        let destination = positive[0].1.destination.unwrap();
        // Tangent to the circle around (940, 500) at (800, 500) is along z.
        assert!((destination.x - 800.0).abs() < 1.0, "{destination:?}");
        let negative = first_commands(constant_brain_full(0.0, 0.0, 0.0, -0.8), units);
        let opposite = negative[0].1.destination.unwrap();
        assert!(
            (destination.z - 500.0) * (opposite.z - 500.0) < 0.0,
            "opposite directions"
        );
    }

    fn tower(side: u8, x: f32, z: f32) -> Spawn {
        let mut definition = skirmisher("tower");
        definition.movement.max_speed = 0.0;
        definition.weapons.clear();
        definition.strategic.key_unit = true;
        spawn_def(side, x, z, definition)
    }

    #[test]
    fn an_attacker_heads_for_the_enemy_key_unit_and_shoots_it_once_in_reach() {
        // The decoy (300 away, off to the side) is the nearest enemy; the tower is
        // 600 away straight ahead. With every module silent the unit would stand
        // still, so any movement along +x is the objective doctrine.
        let far = vec![
            spawn(0, 500.0, 500.0),
            spawn(1, 500.0, 800.0),
            tower(1, 1_100.0, 500.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), far);
        let destination = rows[0].1.destination.expect("attacker moves");
        assert!(
            destination.x > 600.0 && (destination.z - 500.0).abs() < 1.0,
            "{destination:?}"
        );
        assert_ne!(
            rows[0].1.target,
            Some(rows[0].0 + 2),
            "too far to shoot yet"
        );
        // Within 0.9 of its range (180) it shoots the tower instead of the nearer decoy.
        let near = vec![
            spawn(0, 500.0, 500.0),
            spawn(1, 500.0, 520.0),
            tower(1, 640.0, 500.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), near);
        assert_eq!(rows[0].1.target, Some(rows[0].0 + 2));
    }

    #[test]
    fn a_defender_is_not_tethered_to_its_key_unit() {
        // Far from its tower with an enemy in view and every module silent, the
        // defender stands still: nothing drags it back onto the target.
        let stray = vec![
            tower(0, 100.0, 500.0),
            spawn(0, 1_200.0, 500.0),
            spawn(1, 1_600.0, 500.0),
        ];
        let rows = first_commands(constant_brain(0.0, 0.0), stray);
        assert!(rows[1].1.destination.is_none());
    }

    #[test]
    fn the_authored_template_encodes_the_rules_it_claims() {
        let brain = template(6).unwrap();
        brain.check_against(KIND, &specs(1)).unwrap();
        let policy = TacticalPolicy::new(brain).unwrap();
        // Closes on a distant lone enemy, holds near 0.85 range, backs off when
        // too close.
        let at = |enemy_x: f32| {
            let units = vec![spawn(0, 800.0, 500.0), spawn(1, enemy_x, 500.0)];
            let probe = Probe {
                inner: &policy,
                first: Mutex::new(Vec::new()),
            };
            let hold = crate::policy::BuiltinPolicy::Hold;
            Simulator::new(scenario(units)).run([&probe, &hold], 1);
            let first = probe.first.into_inner().unwrap();
            first.iter().find(|(id, ..)| *id == 1).unwrap().1
        };
        let far = at(1_300.0); // 500 away, 2.8 ranges
        assert!(
            far.destination.unwrap().x > 800.0 && far.throttle > 0.7,
            "{far:?}"
        );
        let held = at(953.0); // 153 away = 0.85 of 180
        assert!(
            held.throttle < 0.3,
            "should hold near 0.85 range, got {held:?}"
        );
        let close = at(870.0); // 70 away
        assert!(close.destination.unwrap().x < 800.0, "{close:?}");
    }

    #[test]
    fn template_needs_enough_hidden_units_to_hold_its_rules() {
        assert!(std::panic::catch_unwind(|| template(1)).is_err());
    }

    #[test]
    fn force_inputs_are_all_unit_range_and_named_in_the_engagement_spec() {
        let spec = &specs(2)[4];
        assert_eq!(spec.name, ENGAGEMENT);
        assert_eq!(spec.inputs.len(), 22);
        for name in &spec.inputs {
            let (low, high) = input_range(name);
            assert!(low == 0.0 && high == 1.0, "{name}: {low}..{high}");
        }
        // The first sixteen names are exactly the ratio pairs, in stat order.
        for (i, stat) in FORCE_STATS.iter().enumerate() {
            assert_eq!(spec.inputs[2 * i], format!("ours_over_theirs_{stat}"));
            assert_eq!(spec.inputs[2 * i + 1], format!("theirs_over_ours_{stat}"));
        }
    }

    #[test]
    fn explanation_matches_the_fast_path_exactly() {
        let brain = Brain::random(KIND, specs(6), &mut Rng::new(5), 1.0).unwrap();
        let policy = TacticalPolicy::new(brain).unwrap();
        let sim_units = vec![
            spawn(0, 800.0, 500.0),
            spawn(0, 820.0, 560.0),
            spawn(1, 1_000.0, 500.0),
            spawn(1, 1_050.0, 540.0),
        ];

        struct Both<'a>(&'a TacticalPolicy, Mutex<usize>);
        impl Policy for Both<'_> {
            fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
                let fast = self.0.command(world, unit);
                let (slow, trace) = self.0.explain(world, unit);
                assert_eq!(fast.target, slow.target);
                assert_eq!(fast.destination, slow.destination);
                assert_eq!(fast.throttle, slow.throttle);
                let trace = trace.expect("enemies exist");
                assert_eq!(trace.chosen_target, slow.target);
                assert!(trace.modules.iter().any(|m| m.module == TARGET_SELECTOR));
                *self.1.lock().unwrap() += 1;
                fast
            }
        }
        let both = Both(&policy, Mutex::new(0));
        let hold = crate::policy::BuiltinPolicy::Hold;
        let mut config = scenario(sim_units);
        config.max_time = 2.0;
        Simulator::new(config).run([&both, &hold], 3);
        assert!(*both.1.lock().unwrap() > 20);
    }

    #[test]
    fn foreign_brains_are_rejected() {
        let mut specs = specs(3);
        specs[1].inputs.pop();
        let brain = Brain::random(KIND, specs, &mut Rng::new(1), 1.0).unwrap();
        assert!(TacticalPolicy::new(brain).is_err());
    }
    fn v4_silent() -> Brain {
        let mut brain = template_for_kind(KIND_V4, 6).unwrap();
        for module in &mut brain.modules {
            module.net.params_mut().fill(0.0);
        }
        brain
    }
    #[test]
    fn v4_preserves_v3_json_loading_and_commands() {
        let brain = template(6).unwrap();
        let loaded = Brain::from_json(&brain.to_json()).unwrap();
        assert_eq!(loaded.kind, KIND);
        let units = vec![spawn(0, 800.0, 500.0), spawn(1, 1_000.0, 500.0)];
        let before = first_commands(brain, units.clone());
        let after = first_commands(loaded, units);
        assert_eq!(before[0].1.destination, after[0].1.destination);
        assert_eq!(before[0].1.target, after[0].1.target);
    }
    #[test]
    fn v4_can_select_key_unit_outside_three_nearest() {
        let mut brain = v4_silent();
        let selector = brain
            .modules
            .iter_mut()
            .find(|m| m.spec.name == TARGET_SELECTOR)
            .unwrap();
        Wiring { module: selector }
            .reads(0, "target_is_key", 5.0)
            .feeds_output(0, 2.0);
        let mut units = vec![spawn(0, 800.0, 500.0)];
        for x in [950.0, 1_000.0, 1_050.0, 1_500.0] {
            units.push(spawn(1, x, 500.0));
        }
        units.last_mut().unwrap().definition.strategic.key_unit = true;
        let result = first_commands(brain, units);
        assert_eq!(result[0].1.target, Some(5));
    }
    #[test]
    fn v4_rejects_incompatible_firing_target() {
        let mut ours = spawn(0, 800.0, 500.0);
        ours.definition.weapons[0].only_target_categories = vec!["SURFACE".into()];
        let mut enemy = spawn(1, 1_000.0, 500.0);
        enemy.definition.strategic.target_categories = vec!["VTOL".into()];
        let result = first_commands(v4_silent(), vec![ours, enemy]);
        assert_eq!(result[0].1.target, None);
    }
    #[test]
    fn v4_mission_weights_change_actual_movement() {
        let silent = v4_silent();
        let mut driven = silent.clone();
        let mission = driven
            .modules
            .iter_mut()
            .find(|m| m.spec.name == MISSION)
            .unwrap();
        *mission.net.params_mut().last_mut().unwrap() = atanh(0.8);
        let mut objective = spawn(1, 1_500.0, 700.0);
        objective.definition.strategic.key_unit = true;
        let units = vec![spawn(0, 800.0, 500.0), spawn(1, 1_000.0, 500.0), objective];
        let stopped = first_commands(silent, units.clone());
        let moving = first_commands(driven, units);
        assert_eq!(stopped[0].1.destination, None);
        assert!(moving[0].1.destination.unwrap().z > 500.0);
    }
    #[test]
    fn air_contract_is_independent_and_rejects_ground_checkpoint() {
        let contracts = specs_for_kind("tactical-air-v1", 3).expect("air contract is supported");
        assert!(
            contracts
                .iter()
                .any(|m| m.inputs.iter().any(|i| i == "altitude_difference"))
        );
        let brain = template_for_kind("tactical-air-v1", 3).unwrap();
        assert_eq!(brain.kind, "tactical-air-v1");
        assert!(brain.check_against(KIND, &specs(3)).is_err());
        let restored = Brain::from_json(&brain.to_json()).unwrap();
        assert_eq!(restored, brain);
        assert!(TacticalPolicy::new(restored).is_ok());
    }
}
