//! Explicit strategic objective; defaults reproduce the legacy objective.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProductionMode {
    #[default]
    Count,
    None,
    Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RewardConfig {
    pub production_mode: ProductionMode,
    pub production_weight: f64,
    pub production_reference_value: f64,
    pub production_cap: Option<f64>,
}
impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            production_mode: ProductionMode::Count,
            production_weight: 0.02,
            production_reference_value: 100.0,
            production_cap: None,
        }
    }
}
impl RewardConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.production_weight.is_finite()
            || self.production_weight < 0.0
            || !self.production_reference_value.is_finite()
            || self.production_reference_value <= 0.0
            || self
                .production_cap
                .is_some_and(|v| !v.is_finite() || v < 0.0)
        {
            return Err("invalid strategic reward configuration".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RewardStats {
    pub own_surviving_value: f64,
    pub enemy_surviving_value: f64,
    pub metal_income_integral: f64,
    pub units_produced: u64,
    pub production_value: f64,
    pub energy_overflow: f64,
    pub metal_overflow: f64,
    pub stalled_build_power_seconds: f64,
    pub duration_seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RewardComponents {
    pub asset_difference: f64,
    pub own_survival: f64,
    pub metal_income: f64,
    pub production: f64,
    pub energy_overflow: f64,
    pub metal_overflow: f64,
    pub stalled_build_power: f64,
    pub total: f64,
}
pub fn score(s: &RewardStats, c: &RewardConfig) -> Result<RewardComponents, String> {
    c.validate()?;
    if s.duration_seconds <= 0.0
        || [
            s.own_surviving_value,
            s.enemy_surviving_value,
            s.metal_income_integral,
            s.production_value,
            s.energy_overflow,
            s.metal_overflow,
            s.stalled_build_power_seconds,
            s.duration_seconds,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err("invalid reward sufficient statistics".into());
    }
    let mut production = c.production_weight
        * match c.production_mode {
            ProductionMode::None => 0.0,
            ProductionMode::Count => s.units_produced as f64,
            ProductionMode::Value => s.production_value / c.production_reference_value,
        };
    if let Some(cap) = c.production_cap {
        production = production.min(cap);
    }
    let mut r = RewardComponents {
        asset_difference: (s.own_surviving_value - s.enemy_surviving_value) / 1000.0,
        own_survival: 0.2 * s.own_surviving_value / 1000.0,
        metal_income: 0.02 * s.metal_income_integral / s.duration_seconds,
        production,
        energy_overflow: -0.001 * s.energy_overflow / s.duration_seconds,
        metal_overflow: -0.1 * s.metal_overflow / s.duration_seconds,
        stalled_build_power: -0.0001 * s.stalled_build_power_seconds / s.duration_seconds,
        total: 0.0,
    };
    r.total = r.asset_difference
        + r.own_survival
        + r.metal_income
        + r.production
        + r.energy_overflow
        + r.metal_overflow
        + r.stalled_build_power;
    if !r.total.is_finite() {
        return Err("nonfinite strategic reward".into());
    }
    Ok(r)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_caps_and_sufficient_statistics() {
        let mut s = RewardStats {
            duration_seconds: 100.0,
            units_produced: 2,
            production_value: 100.0,
            ..Default::default()
        };
        let mut c = RewardConfig::default();
        assert_eq!(score(&s, &c).unwrap().production, 0.04);
        c.production_mode = ProductionMode::Value;
        assert_eq!(score(&s, &c).unwrap().production, 0.02);
        s.production_value = 300.0;
        assert_eq!(score(&s, &c).unwrap().production, 0.06);
        c.production_cap = Some(0.01);
        assert_eq!(score(&s, &c).unwrap().production, 0.01);
        c.production_mode = ProductionMode::None;
        assert_eq!(score(&s, &c).unwrap().production, 0.0);
        c.production_reference_value = 0.0;
        assert!(score(&s, &c).is_err());
    }
    #[test]
    fn legacy_arithmetic_is_preserved() {
        let s = RewardStats {
            own_surviving_value: 1234.5,
            enemy_surviving_value: 888.1,
            metal_income_integral: 610.0,
            units_produced: 17,
            energy_overflow: 1080.0,
            metal_overflow: 11.0,
            stalled_build_power_seconds: 78.0,
            duration_seconds: 300.0,
            ..Default::default()
        };
        let expected = (s.own_surviving_value - s.enemy_surviving_value) / 1000.0
            + 0.2 * s.own_surviving_value / 1000.0
            + 0.02 * s.metal_income_integral / s.duration_seconds
            + 0.02 * s.units_produced as f64
            - 0.001 * s.energy_overflow / s.duration_seconds
            - 0.1 * s.metal_overflow / s.duration_seconds
            - 0.0001 * s.stalled_build_power_seconds / s.duration_seconds;
        assert_eq!(score(&s, &RewardConfig::default()).unwrap().total, expected);
    }
}
