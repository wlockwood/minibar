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
                "unknown policy {spec:?}; expected charge, kite, orbit, hold, strike, guard or brain:PATH"
            )
        })
}

#[derive(Clone, Copy, Debug)]
pub enum BuiltinPolicy {
    Charge,
    Kite,
    Orbit,
    Hold,
}

impl BuiltinPolicy {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "charge" => Some(Self::Charge),
            "kite" => Some(Self::Kite),
            "orbit" => Some(Self::Orbit),
            "hold" => Some(Self::Hold),
            _ => None,
        }
    }
}

impl Policy for BuiltinPolicy {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        let Some(enemy) = world.nearest_enemy(unit) else {
            return Command::default();
        };
        let toward = (enemy.position - unit.position).normalized();
        let distance = unit.position.distance(enemy.position);
        let range = unit.primary_range();
        let target = Some(enemy.id);
        match self {
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
