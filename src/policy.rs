use crate::model::{Command, UnitState, Vec2, WorldView};

/// A tactic is read-only and stateless from the simulator's point of view, so
/// one instance can safely be shared by every Rayon worker.
pub trait Policy: Sync {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command;
}

/// Resolves a policy name used by the CLI and by training configs: a built-in
/// tactic (`charge`, `kite`, `orbit`, `hold`) or `brain:PATH` for a trained
/// council saved by `--train`.
pub fn parse_policy(spec: &str) -> Result<Box<dyn Policy>, String> {
    if let Some(path) = spec.strip_prefix("brain:") {
        let brain = crate::learn::Brain::load(std::path::Path::new(path))
            .map_err(|error| error.to_string())?;
        let policy = crate::learn::TacticalPolicy::new(brain).map_err(|error| error.to_string())?;
        return Ok(Box::new(policy));
    }
    if let Some(baseline) = crate::learn::baselines::Baseline::parse(spec) {
        return Ok(Box::new(baseline));
    }
    BuiltinPolicy::parse(spec)
        .map(|builtin| Box::new(builtin) as Box<dyn Policy>)
        .ok_or_else(|| {
            format!(
                "unknown policy {spec:?}; expected charge, kite, orbit, hold, surround, strike, guard or brain:PATH"
            )
        })
}

#[derive(Clone, Copy, Debug)]
pub enum BuiltinPolicy {
    Charge,
    Kite,
    Orbit,
    Hold,
    Surround,
}

impl BuiltinPolicy {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "charge" => Some(Self::Charge),
            "kite" => Some(Self::Kite),
            "orbit" => Some(Self::Orbit),
            "hold" => Some(Self::Hold),
            "surround" => Some(Self::Surround),
            _ => None,
        }
    }
}

impl Policy for BuiltinPolicy {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        let enemy = if matches!(self, Self::Surround) {
            surround_target(world, unit)
        } else {
            world.nearest_enemy(unit)
        };
        let Some(enemy) = enemy else {
            return if matches!(self, Self::Hold) {
                Command::default()
            } else {
                reconnaissance(world, unit)
            };
        };
        let toward = (enemy.position - unit.position).normalized();
        let distance = unit.position.distance(enemy.position);
        let range = unit.primary_range();
        let target = Some(enemy.id);
        match self {
            Self::Surround => surround(world, unit, enemy),
            Self::Charge => Command {
                target,
                destination: Some(enemy.position),
                throttle: 1.0,
            },
            Self::Hold => Command {
                target,
                destination: None,
                throttle: 0.0,
            },
            Self::Kite => {
                // Back away while reloading, plant briefly to aim/fire, and
                // close only when the opponent has left effective range.
                if unit.reload_remaining(world.time) > 0.0 && distance < range * 1.05 {
                    Command {
                        target,
                        destination: Some(unit.position - toward * 500.0),
                        throttle: 1.0,
                    }
                } else if distance > range * 0.92 {
                    Command {
                        target,
                        destination: Some(enemy.position),
                        throttle: 1.0,
                    }
                } else {
                    Command {
                        target,
                        destination: None,
                        throttle: 0.0,
                    }
                }
            }
            Self::Orbit => {
                let tangent = Vec2::new(-toward.z, toward.x);
                let radial = if distance < range * 0.75 {
                    toward * -0.6
                } else {
                    toward * 0.2
                };
                Command {
                    target,
                    destination: Some(unit.position + (tangent + radial).normalized() * 500.0),
                    throttle: 1.0,
                }
            }
        }
    }
}

/// Search using current heading and known map bounds only. In particular this
/// does not target an undisclosed enemy or stop permanently at the map centre.
pub(crate) fn reconnaissance(world: &WorldView<'_>, unit: &UnitState) -> Command {
    if !unit.alive || unit.definition.movement.max_speed <= 0.0 {
        return Command::default();
    }
    let width = world.terrain.cols as f32 * world.terrain.cell_size;
    let depth = world.terrain.rows as f32 * world.terrain.cell_size;
    let margin = unit.definition.movement.radius + 4.0;
    let ahead = unit.position + Vec2::from_heading(unit.heading) * 500.0;
    let destination = if ahead.x >= margin
        && ahead.x <= width - margin
        && ahead.z >= margin
        && ahead.z <= depth - margin
    {
        ahead
    } else {
        Vec2::new(width * 0.5, depth * 0.5)
    };
    Command {
        target: None,
        destination: Some(destination),
        throttle: 1.0,
    }
}

/// Assign distinct firing slots to combatants pursuing the same visible enemy.
/// Approach far-side slots around the target rather than driving through it.
/// No hidden information, future state or learned weights are used.
fn surround(world: &WorldView<'_>, unit: &UnitState, enemy: &UnitState) -> Command {
    use std::f32::consts::{PI, TAU};
    if unit.definition.movement.max_speed <= 0.0 {
        return Command {
            target: Some(enemy.id),
            ..Default::default()
        };
    }
    let mut allies: Vec<_> = world
        .units
        .iter()
        .filter(|other| {
            other.alive
                && other.side == unit.side
                && other.definition.has_automatic_weapon()
                && other.definition.movement.max_speed > 0.0
                && other.definition.flight.is_some() == unit.definition.flight.is_some()
                && surround_can_attack(other, enemy)
        })
        .collect();
    allies.sort_by_key(|other| other.id);
    let rank = allies
        .iter()
        .position(|other| other.id == unit.id)
        .unwrap_or(0);
    let spacing = allies
        .iter()
        .map(|other| other.definition.movement.radius)
        .fold(unit.definition.movement.radius, f32::max)
        * 2.0
        + 6.0;
    // Use the shortest participating weapon range so mixed forces agree on slots.
    let range = allies
        .iter()
        .map(|other| other.primary_range())
        .fold(unit.primary_range(), f32::min);
    let minimum = enemy.definition.movement.radius + unit.definition.movement.radius + 6.0;
    let mut radius = (range * 0.82).max(minimum);
    let outer_radius = radius;
    let mut ring = 0;
    let mut offset = 0;
    let (slot, count) = loop {
        let capacity = ((TAU * radius / spacing).floor() as usize).max(1);
        let count = capacity.min(allies.len().saturating_sub(offset)).max(1);
        if rank < offset + count {
            break (rank - offset, count);
        }
        offset += count;
        ring += 1;
        radius = if radius <= outer_radius && radius - spacing >= minimum {
            radius - spacing
        } else {
            // Overflow waits outside the firing ring rather than sharing slots.
            radius.max(outer_radius) + spacing
        };
    };
    let angle = (TAU * slot as f32 / count as f32 + ring as f32 * 2.399_963_1).rem_euclid(TAU);
    let goal = enemy.position + Vec2::new(angle.cos(), angle.sin()) * radius;
    let radial = unit.position - enemy.position;
    let current = radial.z.atan2(radial.x);
    let delta = (angle - current + PI).rem_euclid(TAU) - PI;
    let destination = if delta.abs() > 0.3 && radial.length() < radius * 1.7 {
        let step = current + delta.clamp(-0.3, 0.3);
        enemy.position + Vec2::new(step.cos(), step.sin()) * radial.length().max(radius)
    } else {
        goal
    };
    let settled = unit.position.distance(goal) < unit.definition.movement.radius.max(4.0) * 0.5;
    Command {
        target: Some(enemy.id),
        destination: (!settled).then_some(destination),
        throttle: if settled { 0.0 } else { 1.0 },
    }
}

fn surround_target<'a>(world: &'a WorldView<'_>, unit: &UnitState) -> Option<&'a UnitState> {
    // Shared focus for compatible units, without nested nearest-target scans.
    let mut centre = Vec2::default();
    let mut count = 0;
    for other in world.units.iter().filter(|other| {
        other.alive
            && other.side == unit.side
            && other.definition.has_automatic_weapon()
            && other.definition.flight.is_some() == unit.definition.flight.is_some()
    }) {
        centre = centre + other.position;
        count += 1;
    }
    centre = if count > 0 {
        centre * (1.0 / count as f32)
    } else {
        unit.position
    };
    world
        .units
        .iter()
        .filter(|enemy| {
            enemy.alive
                && enemy.side < 2
                && enemy.side < 2
                && enemy.side != unit.side
                && surround_can_attack(unit, enemy)
        })
        .min_by(|a, b| {
            centre
                .distance_squared(a.position)
                .total_cmp(&centre.distance_squared(b.position))
                .then(a.id.cmp(&b.id))
        })
}

fn surround_can_attack(unit: &UnitState, enemy: &UnitState) -> bool {
    unit.definition.weapons.iter().any(|w| {
        !w.manual_fire
            && !w.is_shield
            && w.interceptor == 0
            && crate::engine::weapon_can_target(w, &enemy.definition)
            && w.damage_against(&enemy.definition.strategic.armor_class) > 0.0
    })
}

#[cfg(test)]
mod reconnaissance_tests {
    use super::*;
    use crate::engine::{Scenario, Simulator};
    use crate::learn::{TacticalPolicy, tactical};
    use std::sync::Mutex;

    struct Probe<'a> {
        policy: &'a dyn Policy,
        commands: Mutex<Vec<Command>>,
    }
    impl Policy for Probe<'_> {
        fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
            assert!(
                !world.units.iter().any(|other| other.side == 1),
                "Hidden enemies must stay hidden"
            );
            let command = self.policy.command(world, unit);
            self.commands.lock().unwrap().push(command);
            command
        }
    }

    #[test]
    fn reconnaissance_crosses_the_map_centre_and_finds_a_stationary_hidden_enemy() {
        let mut definition = tactical::tests::skirmisher("scout");
        definition.strategic.sight_distance = Some(100.0);
        let mut scenario = Scenario::duel(definition, 1800.0);
        scenario.max_time = 30.0;
        scenario.units[1].definition.movement.max_speed = 0.0;
        scenario.units[1].definition.weapons[0].damage = 0.0;
        let brain = tactical::template_for_kind(tactical::KIND_V4, 4).unwrap();
        let policy = TacticalPolicy::new(brain).unwrap();
        let result = Simulator::new(scenario).run([&policy, &BuiltinPolicy::Hold], 1);
        assert!(
            result.sides[0].projectiles_fired > 0,
            "The search must continue past the midpoint to acquire the stationary enemy"
        );
    }

    #[test]
    fn mobile_policies_seek_contacts_at_distant_starts_without_hidden_targets() {
        let mut definition = tactical::tests::skirmisher("scout");
        definition.strategic.sight_distance = Some(100.0);
        let mut scenario = Scenario::duel(definition, 1800.0);
        scenario.max_time = 1.0;
        scenario.units[1].definition.movement.max_speed = 0.0;
        let sim = Simulator::new(scenario);
        let mut policies: Vec<(&str, Box<dyn Policy>)> = vec![];
        for name in ["charge", "kite", "orbit", "surround"] {
            policies.push((name, parse_policy(name).unwrap()));
        }
        for kind in [tactical::KIND, tactical::KIND_V4] {
            let brain = tactical::template_for_kind(kind, 4).unwrap();
            policies.push((kind, Box::new(TacticalPolicy::new(brain).unwrap())));
        }
        for (name, policy) in policies {
            let probe = Probe {
                policy: policy.as_ref(),
                commands: Mutex::new(vec![]),
            };
            sim.run([&probe, &BuiltinPolicy::Hold], 1);
            let commands = probe.commands.lock().unwrap();
            assert!(!commands.is_empty());
            assert!(commands.iter().all(|c| c.target.is_none()), "{name}");
            assert!(
                commands
                    .iter()
                    .any(|c| c.throttle > 0.0 && c.destination.is_some()),
                "{name} failed to scout"
            );
        }
    }
}
