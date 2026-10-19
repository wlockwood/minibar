//! Initial approach geometry shared by ground and air combat scenarios.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApproachStart {
    pub name: String,
    /// Degrees from facing the opposing formation; positive turns toward local right.
    pub learner_heading_offset_deg: f32,
    pub enemy_heading_offset_deg: f32,
    /// Fraction of each unit's own maximum speed; immobile units remain stopped.
    pub learner_speed_fraction: f32,
    pub enemy_speed_fraction: f32,
}

impl ApproachStart {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            (
                "learner_heading_offset_deg",
                self.learner_heading_offset_deg,
            ),
            ("enemy_heading_offset_deg", self.enemy_heading_offset_deg),
        ] {
            if !value.is_finite() || !(-180.0..=180.0).contains(&value) {
                return Err(format!(
                    "approach {name} must be finite and between -180 and 180 degrees"
                ));
            }
        }
        for (name, value) in [
            ("learner_speed_fraction", self.learner_speed_fraction),
            ("enemy_speed_fraction", self.enemy_speed_fraction),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "approach {name} must be finite and between 0 and 1"
                ));
            }
        }
        Ok(())
    }

    pub fn for_side(&self, side: usize, learner_side: usize) -> (f32, f32) {
        if side == learner_side {
            (
                self.learner_heading_offset_deg.to_radians(),
                self.learner_speed_fraction,
            )
        } else {
            (
                self.enemy_heading_offset_deg.to_radians(),
                self.enemy_speed_fraction,
            )
        }
    }
}
