//! Independent, versioned fixed-wing controller. Ground weights are never
//! interpreted as aircraft weights. Generic Brain supplies activation and wiring evolution.
use super::rng::Rng;
use super::{Brain, BrainError, ModuleSpec};
use crate::model::{Command, UnitState, Vec2, WorldView, angle_delta};
use crate::policy::Policy;

pub const KIND: &str = "tactical-air-v1";
pub const INPUTS: [&str; 12] = [
    "distance_to_range",
    "heading_error",
    "altitude_difference",
    "my_hp_fraction",
    "target_hp_fraction",
    "reload_fraction",
    "my_speed_fraction",
    "target_speed_fraction",
    "target_is_air",
    "attack_run",
    "target_approaching",
    "target_is_key",
];

pub fn specs(hidden: usize) -> Vec<ModuleSpec> {
    [
        ("air_target", vec!["priority"]),
        ("air_flight", vec!["turn", "throttle", "extend_run"]),
    ]
    .into_iter()
    .map(|(name, outputs)| ModuleSpec {
        name: name.into(),
        role: if name == "air_target" {
            "Prioritize legally damageable air or surface contacts"
        } else {
            "Steer an aircraft and choose cruise speed and attack-run extension"
        }
        .into(),
        inputs: INPUTS.iter().map(|s| s.to_string()).collect(),
        outputs: outputs.into_iter().map(String::from).collect(),
        hidden: vec![hidden],
    })
    .collect()
}

/// Explicit air initialization, independent of every ground checkpoint.
pub fn template(hidden: usize) -> Result<Brain, BrainError> {
    Brain::random(KIND, specs(hidden), &mut Rng::new(0x616972), 0.0)
}

#[derive(Clone, Debug)]
pub struct AirPolicy {
    brain: Brain,
    target: usize,
    flight: usize,
}
impl AirPolicy {
    pub fn new(brain: Brain) -> Result<Self, BrainError> {
        brain.check_against(KIND, &specs(1))?;
        Ok(Self {
            target: brain.module_index("air_target")?,
            flight: brain.module_index("air_flight")?,
            brain,
        })
    }
    pub fn brain(&self) -> &Brain {
        &self.brain
    }
    pub fn explain(
        &self,
        world: &WorldView<'_>,
        unit: &UnitState,
    ) -> (Command, Option<super::tactical::DecisionTrace>) {
        let command = self.command(world, unit);
        if unit.definition.flight.is_none() {
            return (command, None);
        }
        let mut modules = Vec::new();
        let mut record = |index: usize, enemy: &UnitState| {
            let module = &self.brain.modules[index];
            let input = self.inputs(unit, enemy, world);
            let layers = module.net.trace(&input).layers;
            modules.push(super::tactical::ModuleTrace {
                module: module.spec.name.clone(),
                label: Some(format!("enemy #{}", enemy.id)),
                candidate: Some(enemy.id),
                inputs: module.spec.inputs.iter().cloned().zip(input).collect(),
                hidden: layers[1..layers.len() - 1].to_vec(),
                outputs: module
                    .spec
                    .outputs
                    .iter()
                    .cloned()
                    .zip(layers.last().unwrap().iter().copied())
                    .collect(),
            });
        };
        for enemy in world
            .units
            .iter()
            .filter(|u| u.alive && u.side < 2 && u.side != unit.side && can_damage(unit, u))
        {
            record(self.target, enemy);
        }
        if let Some(enemy) = command.target.and_then(|id| world.unit(id)) {
            record(self.flight, enemy);
        }
        let delta = command
            .destination
            .map_or(Vec2::default(), |p| (p - unit.position).normalized());
        let trace = super::tactical::DecisionTrace {
            unit: unit.id,
            time: world.time,
            position: [unit.position.x, unit.position.z],
            chosen_target: command.target,
            modules,
            advance: 1.0,
            orbit: 0.0,
            regroup: 0.0,
            retreat: 0.0,
            bloom: 0.0,
            commit: 0.0,
            movement: [delta.x, delta.z],
            throttle: command.throttle,
        };
        (command, Some(trace))
    }
    fn inputs(&self, unit: &UnitState, enemy: &UnitState, world: &WorldView<'_>) -> [f32; 12] {
        let delta = enemy.position - unit.position;
        let range = unit.primary_range().max(1.0);
        let flight = unit.definition.flight.as_ref().unwrap();
        [
            (delta.length() / range).min(4.0) * 0.25,
            angle_delta(unit.heading, delta.heading()) / std::f32::consts::PI,
            ((world.terrain.height_at(enemy.position) + enemy.altitude
                - world.terrain.height_at(unit.position)
                - unit.altitude)
                / range)
                .clamp(-1.0, 1.0),
            (unit.hp / unit.definition.health.max(1.0)).clamp(0.0, 1.0),
            (enemy.hp / enemy.definition.health.max(1.0)).clamp(0.0, 1.0),
            (unit.reload_remaining(world.time)
                / unit
                    .definition
                    .weapons
                    .first()
                    .map_or(1.0, |w| w.reload.max(0.01)))
            .clamp(0.0, 1.0),
            (unit.speed / unit.definition.movement.max_speed.max(1.0)).clamp(0.0, 1.0),
            (enemy.speed.abs() / unit.definition.movement.max_speed.max(1.0)).clamp(0.0, 1.0),
            f32::from(enemy.definition.flight.is_some()),
            f32::from(flight.attack_run),
            ((unit.velocity() - enemy.velocity()).x * delta.normalized().x
                + (unit.velocity() - enemy.velocity()).z * delta.normalized().z)
                / unit.definition.movement.max_speed.max(1.0),
            f32::from(enemy.definition.strategic.key_unit),
        ]
    }
}

pub fn can_damage(unit: &UnitState, enemy: &UnitState) -> bool {
    unit.definition.weapons.iter().any(|w| {
        !w.manual_fire
            && !w.is_shield
            && w.interceptor == 0
            && w.damage_against(&enemy.definition.strategic.armor_class) > 0.0
            && (w.only_target_categories.is_empty()
                || w.only_target_categories
                    .iter()
                    .any(|c| enemy.definition.strategic.target_categories.contains(c)))
    })
}
impl Policy for AirPolicy {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        let Some(flight) = &unit.definition.flight else {
            return Command::default();
        };
        let mut best: Option<(f32, &UnitState, [f32; 12])> = None;
        for enemy in world
            .units
            .iter()
            .filter(|u| u.alive && u.side < 2 && u.side != unit.side && can_damage(unit, u))
        {
            let input = self.inputs(unit, enemy, world);
            let mut score = [0.0];
            self.brain.modules[self.target]
                .net
                .forward(&input, &mut score);
            let priority = score[0] - input[0] * 0.05;
            if best.as_ref().is_none_or(|(s, _, _)| priority > *s) {
                best = Some((priority, enemy, input));
            }
        }
        let Some((_, enemy, input)) = best else {
            return crate::policy::reconnaissance(world, unit);
        };
        let mut output = [0.0; 3];
        self.brain.modules[self.flight]
            .net
            .forward(&input, &mut output);
        let toward = (enemy.position - unit.position).normalized();
        // A bomber extends along its present heading after release instead of
        // reversing at the target. This provides a bounded attack-run orbit.
        let extending =
            flight.attack_run && input[5] > (0.05 + 0.4 * output[2].clamp(-1.0, 1.0)).max(0.0);
        let direction = if extending {
            Vec2::from_heading(unit.heading)
        } else {
            toward.rotate(output[0].clamp(-1.0, 1.0) * 1.2)
        };
        Command {
            target: Some(enemy.id),
            destination: Some(unit.position + direction * 500.0),
            throttle: (0.8 + output[1] * 0.2).clamp(0.0, 1.0),
        }
    }
}

/// An immutable snapshot of both controllers. Evaluating a candidate cannot
/// mutate the companion's weights, wiring or activations.
#[derive(Clone, Debug)]
pub struct DomainPolicy {
    ground: super::tactical::TacticalPolicy,
    air: AirPolicy,
}
impl DomainPolicy {
    pub fn new(candidate: Brain, frozen_companion: Brain) -> Result<Self, BrainError> {
        let (ground, air) = if candidate.kind == KIND {
            (frozen_companion, candidate)
        } else {
            (candidate, frozen_companion)
        };
        if ground.kind == KIND || air.kind != KIND {
            return Err(BrainError::KindMismatch {
                expected: "one ground and one air checkpoint".into(),
                found: format!("{} + {}", ground.kind, air.kind),
            });
        }
        Ok(Self {
            ground: super::tactical::TacticalPolicy::new(ground)?,
            air: AirPolicy::new(air)?,
        })
    }
}
impl Policy for DomainPolicy {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        if unit.definition.flight.is_some() {
            self.air.command(world, unit)
        } else {
            self.ground.command(world, unit)
        }
    }
}
