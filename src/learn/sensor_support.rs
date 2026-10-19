//! Seeded reconnaissance support. Providers are ordinary, destructible catalog
//! units; selecting a provider never grants omniscient or unjammable vision.
use super::rng::Rng;
use super::train::{RosterEntry, TrainError};
use crate::catalog::UnitCatalog;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SensorSupportConfig {
    pub radars: Vec<String>,
    pub air_scouts: Vec<String>,
    pub jammers: Vec<String>,
    pub jammer_share: f32,
}

impl Default for SensorSupportConfig {
    fn default() -> Self {
        Self {
            radars: Vec::new(),
            air_scouts: Vec::new(),
            jammers: Vec::new(),
            jammer_share: 1.0 / 3.0,
        }
    }
}

/// Relative to learner/enemy, regardless of their physical starting side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorSupport {
    pub radar: Option<String>,
    pub air_scout: Option<String>,
    pub jammer: Option<String>,
}

impl SensorSupport {
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.radar.iter().chain(&self.air_scout).chain(&self.jammer)
    }

    pub fn roster(&self) -> Vec<RosterEntry> {
        self.names()
            .map(|name| RosterEntry {
                unit: name.clone(),
                count: 1,
                platoon: 0,
            })
            .collect()
    }

    pub fn value(&self, catalog: &UnitCatalog) -> f32 {
        self.names()
            .map(|n| catalog.get(n).unwrap().resource_value())
            .sum()
    }
}

impl SensorSupportConfig {
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.radars
            .iter()
            .chain(&self.air_scouts)
            .chain(&self.jammers)
    }

    pub fn validate(&self, catalog: &UnitCatalog) -> Result<(), TrainError> {
        let error = |message: String| TrainError(format!("sensor support: {message}"));
        if self.radars.is_empty() && self.air_scouts.is_empty() {
            return Err(error(
                "needs at least one radar or radar-equipped air scout".into(),
            ));
        }
        if !self.jammer_share.is_finite() || !(0.0..=1.0).contains(&self.jammer_share) {
            return Err(error("jammer_share must be a finite fraction".into()));
        }
        if self.jammer_share > 0.0 && self.jammers.is_empty() {
            return Err(error("positive jammer_share needs a jammer pool".into()));
        }
        for (pool, role) in [
            (&self.radars, "radar"),
            (&self.air_scouts, "air scout"),
            (&self.jammers, "jammer"),
        ] {
            for name in pool {
                let d = catalog
                    .get(name)
                    .ok_or_else(|| error(format!("missing {role} {name}")))?;
                let valid = !d.has_automatic_weapon()
                    && match role {
                        "air scout" => {
                            d.flight.is_some()
                                && d.movement.max_speed > 0.0
                                && d.movement.acceleration > 0.0
                                && d.strategic.sight_distance.unwrap_or(0.0) > 0.0
                                && d.strategic.radar_radius.unwrap_or(0.0) > 0.0
                        }
                        "radar" => {
                            d.flight.is_none()
                                && d.movement.max_speed == 0.0
                                && d.strategic.radar_radius.unwrap_or(0.0) > 0.0
                        }
                        _ => {
                            d.flight.is_none()
                                && d.movement.max_speed == 0.0
                                && d.strategic.jammer_radius.unwrap_or(0.0) > 0.0
                                && d.strategic.radar_radius.unwrap_or(0.0) == 0.0
                        }
                    };
                if !valid || !d.resource_value().is_finite() || d.resource_value() <= 0.0 {
                    return Err(error(format!(
                        "{name} is not a supported unarmed {role} with positive resource value"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn generate(&self, seed: u64, index: usize) -> [SensorSupport; 2] {
        // Separate streams preserve combat compositions and distance draws.
        // Rotation gives exact thirds in batches divisible by three, while
        // rotating short batches across fresh layouts (not a fixed family bias).
        let mut rotation = Rng::for_stream(seed ^ 0x7261_6461_725f_6d69, 0);
        let stratum = (index + (rotation.next_u64() % 3) as usize) % 3;
        let mut rng = Rng::for_stream(seed ^ 0x7363_6f75_745f_6d69, index as u64);
        let owner = (rng.next_u64() % 2) as usize;
        let mut result: [SensorSupport; 2] = Default::default();
        let choose = |pool: &[String], rng: &mut Rng| {
            pool[(rng.next_u64() % pool.len() as u64) as usize].clone()
        };
        for (side, support) in result.iter_mut().enumerate() {
            if stratum == 2 || (stratum == 1 && side == owner) {
                let provider = if self.air_scouts.is_empty() {
                    0
                } else if self.radars.is_empty() {
                    1
                } else {
                    rng.next_u64() % 3
                };
                if provider != 1 {
                    support.radar = Some(choose(&self.radars, &mut rng));
                }
                if provider != 0 {
                    support.air_scout = Some(choose(&self.air_scouts, &mut rng));
                }
            }
            if rng.uniform() < self.jammer_share {
                support.jammer = Some(choose(&self.jammers, &mut rng));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::tactical::tests::skirmisher;
    #[test]
    fn support_pools_reject_invalid_roles_and_probabilities() {
        let mut catalog = UnitCatalog::default();
        let mut radar = skirmisher("radar");
        radar.weapons.clear();
        radar.movement.max_speed = 0.0;
        radar.strategic.radar_radius = Some(2000.0);
        catalog.units.insert("radar".into(), radar.clone());
        let mut config = SensorSupportConfig {
            radars: vec!["radar".into()],
            jammer_share: 0.0,
            ..Default::default()
        };
        config.validate(&catalog).unwrap();
        config.jammer_share = f32::NAN;
        assert!(config.validate(&catalog).is_err());
        config.jammer_share = 0.0;
        config.air_scouts = vec!["radar".into()];
        assert!(config.validate(&catalog).is_err());
        config.air_scouts.clear();
        config.jammers = vec!["radar".into()];
        assert!(config.validate(&catalog).is_err());
        config.jammers.clear();
        config.radars = vec!["missing".into()];
        assert!(config.validate(&catalog).is_err());
    }
}
