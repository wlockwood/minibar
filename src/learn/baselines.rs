//! Objective-aware baseline opponents.
//!
//! The built-in policies (`charge`, `hold`, `kite`, `orbit`) ignore objectives:
//! a charging attacker never goes for the target on purpose, and nothing
//! defends it. These two do, so attack and defend fights have opponents that
//! take the objective seriously.
//!
//! * `strike`: everyone drives at the nearest enemy key unit and shoots it,
//!   whatever else is around; with no key unit left it charges like `charge`.
//! * `guard`: everyone stays near their own key unit, fights what comes within
//!   a leash of it and otherwise returns to it; with no key unit it holds.

use crate::model::{Command, UnitState, Vec2, WorldView};
use crate::policy::Policy;

/// How far from its key unit a guard will chase, in weapon ranges.
const LEASH_RANGES: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Baseline {
    Strike,
    Guard,
}

impl Baseline {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "strike" => Some(Self::Strike),
            "guard" => Some(Self::Guard),
            _ => None,
        }
    }
}

fn nearest<'a>(
    world: &'a WorldView<'_>,
    from: Vec2,
    keep: impl Fn(&UnitState) -> bool,
) -> Option<&'a UnitState> {
    world
        .units
        .iter()
        .filter(|u| u.alive && keep(u))
        .min_by(|a, b| {
            from.distance_squared(a.position)
                .total_cmp(&from.distance_squared(b.position))
        })
}

impl Policy for Baseline {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        let range = unit.primary_range();
        match self {
            Self::Strike => {
                let goal = nearest(world, unit.position, |u| {
                    u.side != unit.side && u.definition.strategic.key_unit
                })
                .or_else(|| world.nearest_enemy(unit));
                let Some(goal) = goal else {
                    return Command::default();
                };
                let close_enough = unit.position.distance(goal.position) <= range * 0.9;
                Command {
                    target: Some(goal.id),
                    destination: (!close_enough).then_some(goal.position),
                    throttle: if close_enough { 0.0 } else { 1.0 },
                }
            }
            Self::Guard => {
                let home = nearest(world, unit.position, |u| {
                    u.side == unit.side && u.definition.strategic.key_unit
                });
                let Some(home) = home else {
                    return Command {
                        target: world.nearest_enemy(unit).map(|e| e.id),
                        destination: None,
                        throttle: 0.0,
                    };
                };
                let leash = (range * LEASH_RANGES).max(120.0);
                let intruder = nearest(world, home.position, |u| u.side != unit.side)
                    .filter(|e| e.position.distance(home.position) <= leash + range);
                match intruder {
                    Some(enemy) => {
                        let in_range = unit.position.distance(enemy.position) <= range * 0.9;
                        Command {
                            target: Some(enemy.id),
                            destination: (!in_range).then_some(enemy.position),
                            throttle: if in_range { 0.0 } else { 1.0 },
                        }
                    }
                    None => {
                        let away = unit.position.distance(home.position);
                        Command {
                            target: world.nearest_enemy(unit).map(|e| e.id),
                            destination: (away > 60.0).then_some(home.position),
                            throttle: if away > 60.0 { 0.6 } else { 0.0 },
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse() {
        assert_eq!(Baseline::parse("Strike"), Some(Baseline::Strike));
        assert_eq!(Baseline::parse("guard"), Some(Baseline::Guard));
        assert_eq!(Baseline::parse("charge"), None);
    }
}
