use serde::{Deserialize, Serialize};
use std::array;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Default, Serialize)]
pub struct SideScore {
    pub side: u8,
    pub won: bool,
    pub starting_value: f32,
    pub surviving_value: f32,
    /// Resource value weighted by each survivor's remaining HP fraction.
    pub remaining_hp_value: f32,
    pub enemy_value_destroyed: f32,
    pub damage_dealt: f32,
    pub damage_received: f32,
    pub overkill_damage: f32,
    pub projectiles_fired: u64,
    /// Projectile impacts or damage-over-time pulses that touched a target.
    pub damage_events_hit: u64,
    pub duration: f32,
}

impl SideScore {
    pub fn value_preserved_fraction(&self) -> f32 {
        if self.starting_value > 0.0 {
            self.surviving_value / self.starting_value
        } else {
            0.0
        }
    }
    pub fn value_destroyed_fraction(&self, enemy_starting_value: f32) -> f32 {
        if enemy_starting_value > 0.0 {
            self.enemy_value_destroyed / enemy_starting_value
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleResult {
    pub winner: Option<u8>,
    pub timed_out: bool,
    pub duration: f32,
    pub sides: [SideScore; 2],
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchSummary {
    pub runs: usize,
    pub side_wins: [usize; 2],
    pub side_win_rate: [f64; 2],
    /// Wilson 95% interval for each unconditional win rate (draws are not wins).
    pub side_win_rate_95ci: [[f64; 2]; 2],
    pub draws: usize,
    pub mean_duration: f32,
    pub mean_scores: [MeanSideScore; 2],
    pub simulations_per_second: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MeanSideScore {
    pub side: u8,
    pub starting_value: f64,
    pub surviving_value: f64,
    pub remaining_hp_value: f64,
    pub enemy_value_destroyed: f64,
    pub damage_dealt: f64,
    pub damage_received: f64,
    pub overkill_damage: f64,
    pub projectiles_fired: f64,
    pub damage_events_hit: f64,
}

impl BatchSummary {
    pub fn from_results(results: &[BattleResult], elapsed_seconds: f64) -> Self {
        let mut side_wins = [0, 0];
        let mut draws = 0;
        let mut scores = [
            MeanSideScore {
                side: 0,
                ..Default::default()
            },
            MeanSideScore {
                side: 1,
                ..Default::default()
            },
        ];
        for result in results {
            match result.winner {
                Some(side) if side < 2 => side_wins[side as usize] += 1,
                _ => draws += 1,
            }
            for (total, source) in scores.iter_mut().zip(result.sides.iter()) {
                total.starting_value += source.starting_value as f64;
                total.surviving_value += source.surviving_value as f64;
                total.remaining_hp_value += source.remaining_hp_value as f64;
                total.enemy_value_destroyed += source.enemy_value_destroyed as f64;
                total.damage_dealt += source.damage_dealt as f64;
                total.damage_received += source.damage_received as f64;
                total.overkill_damage += source.overkill_damage as f64;
                total.projectiles_fired += source.projectiles_fired as f64;
                total.damage_events_hit += source.damage_events_hit as f64;
            }
        }
        if !results.is_empty() {
            let count = results.len() as f64;
            for score in &mut scores {
                score.starting_value /= count;
                score.surviving_value /= count;
                score.remaining_hp_value /= count;
                score.enemy_value_destroyed /= count;
                score.damage_dealt /= count;
                score.damage_received /= count;
                score.overkill_damage /= count;
                score.projectiles_fired /= count;
                score.damage_events_hit /= count;
            }
        }
        let side_win_rate = side_wins.map(|wins| wins as f64 / results.len().max(1) as f64);
        Self {
            runs: results.len(),
            side_wins,
            side_win_rate,
            side_win_rate_95ci: side_wins.map(|wins| wilson_interval(wins, results.len())),
            draws,
            mean_duration: results.iter().map(|result| result.duration).sum::<f32>()
                / results.len().max(1) as f32,
            mean_scores: scores,
            simulations_per_second: results.len() as f64 / elapsed_seconds.max(f64::EPSILON),
        }
    }
}

fn wilson_interval(successes: usize, trials: usize) -> [f64; 2] {
    if trials == 0 {
        return [0.0, 0.0];
    }
    let n = trials as f64;
    let p = successes as f64 / n;
    let z = 1.959_963_984_540_054;
    let denominator = 1.0 + z * z / n;
    let centre = (p + z * z / (2.0 * n)) / denominator;
    let margin = z * ((p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt()) / denominator;
    [(centre - margin).max(0.0), (centre + margin).min(1.0)]
}

/// Stable vocabulary for strategic experiment measurements.
///
/// Values are deliberately kept separate from policy-specific weights. A scenario
/// may choose any subset of these measurements when constructing a composite
/// objective; the simulator does not define a universal strategic score.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum StrategicMetric {
    ArmyValue,
    BuilderValue,
    FactoryValue,
    EconomyValue,
    MetalGenerated,
    EnergyGenerated,
    MetalSpent,
    EnergySpent,
    MetalOverflow,
    EnergyOverflow,
    MetalIncomeIntegral,
    EnergyIncomeIntegral,
    MetalStallSeconds,
    EnergyStallSeconds,
    BuilderIdleSeconds,
    FactoryIdleSeconds,
    UnitsProduced,
    ProductionValue,
    SunkCost,
    MilestonesReached,
    MilestoneScore,
    SiteOccupancyIntegral,
    MapControlIntegral,
    WallRouteDelaySeconds,
    WallBreaches,
    WallInterceptedDamage,
    WallLeaks,
    WallFriendlyObstructionSeconds,
    WallLocalExchangeValue,
}

pub const STRATEGIC_METRIC_COUNT: usize = 29;

impl StrategicMetric {
    pub const ALL: [Self; STRATEGIC_METRIC_COUNT] = [
        Self::ArmyValue,
        Self::BuilderValue,
        Self::FactoryValue,
        Self::EconomyValue,
        Self::MetalGenerated,
        Self::EnergyGenerated,
        Self::MetalSpent,
        Self::EnergySpent,
        Self::MetalOverflow,
        Self::EnergyOverflow,
        Self::MetalIncomeIntegral,
        Self::EnergyIncomeIntegral,
        Self::MetalStallSeconds,
        Self::EnergyStallSeconds,
        Self::BuilderIdleSeconds,
        Self::FactoryIdleSeconds,
        Self::UnitsProduced,
        Self::ProductionValue,
        Self::SunkCost,
        Self::MilestonesReached,
        Self::MilestoneScore,
        Self::SiteOccupancyIntegral,
        Self::MapControlIntegral,
        Self::WallRouteDelaySeconds,
        Self::WallBreaches,
        Self::WallInterceptedDamage,
        Self::WallLeaks,
        Self::WallFriendlyObstructionSeconds,
        Self::WallLocalExchangeValue,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::ArmyValue => "army_value",
            Self::BuilderValue => "builder_value",
            Self::FactoryValue => "factory_value",
            Self::EconomyValue => "economy_value",
            Self::MetalGenerated => "metal_generated",
            Self::EnergyGenerated => "energy_generated",
            Self::MetalSpent => "metal_spent",
            Self::EnergySpent => "energy_spent",
            Self::MetalOverflow => "metal_overflow",
            Self::EnergyOverflow => "energy_overflow",
            Self::MetalIncomeIntegral => "metal_income_integral",
            Self::EnergyIncomeIntegral => "energy_income_integral",
            Self::MetalStallSeconds => "metal_stall_seconds",
            Self::EnergyStallSeconds => "energy_stall_seconds",
            Self::BuilderIdleSeconds => "builder_idle_seconds",
            Self::FactoryIdleSeconds => "factory_idle_seconds",
            Self::UnitsProduced => "units_produced",
            Self::ProductionValue => "production_value",
            Self::SunkCost => "sunk_cost",
            Self::MilestonesReached => "milestones_reached",
            Self::MilestoneScore => "milestone_score",
            Self::SiteOccupancyIntegral => "site_occupancy_integral",
            Self::MapControlIntegral => "map_control_integral",
            Self::WallRouteDelaySeconds => "wall_route_delay_seconds",
            Self::WallBreaches => "wall_breaches",
            Self::WallInterceptedDamage => "wall_intercepted_damage",
            Self::WallLeaks => "wall_leaks",
            Self::WallFriendlyObstructionSeconds => "wall_friendly_obstruction_seconds",
            Self::WallLocalExchangeValue => "wall_local_exchange_value",
        }
    }
}

/// Dense metric storage used in every trial. Its allocation-free, fixed-size
/// representation is suitable for worker-local reuse.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct StrategicMetricVector {
    values: [f64; STRATEGIC_METRIC_COUNT],
}

impl Default for StrategicMetricVector {
    fn default() -> Self {
        Self {
            values: [0.0; STRATEGIC_METRIC_COUNT],
        }
    }
}

impl StrategicMetricVector {
    pub const fn from_values(values: [f64; STRATEGIC_METRIC_COUNT]) -> Self {
        Self { values }
    }

    pub const fn values(&self) -> &[f64; STRATEGIC_METRIC_COUNT] {
        &self.values
    }

    pub fn get(&self, metric: StrategicMetric) -> f64 {
        self.values[metric.index()]
    }

    pub fn set(&mut self, metric: StrategicMetric, value: f64) {
        self.values[metric.index()] = value;
    }

    pub fn add(&mut self, metric: StrategicMetric, amount: f64) {
        self.values[metric.index()] += amount;
    }

    pub fn is_finite(&self) -> bool {
        self.values.iter().all(|value| value.is_finite())
    }

    pub fn difference(self, baseline: Self) -> Self {
        Self::from_values(array::from_fn(|index| {
            self.values[index] - baseline.values[index]
        }))
    }

    pub fn scaled(self, scale: f64) -> Self {
        Self::from_values(self.values.map(|value| value * scale))
    }

    pub fn sum(self, other: Self) -> Self {
        Self::from_values(array::from_fn(|index| {
            self.values[index] + other.values[index]
        }))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricWeight {
    pub metric: StrategicMetric,
    pub weight: f64,
}

/// Scenario-owned composite objective. An empty/default declaration produces
/// zero, making the absence of a universal score explicit.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CompositeWeights {
    pub weights: Vec<MetricWeight>,
}

impl CompositeWeights {
    pub fn validate(&self) -> Result<(), AnalysisError> {
        let mut seen = 0_u64;
        for entry in &self.weights {
            if !entry.weight.is_finite() {
                return Err(AnalysisError::NonFiniteWeight(entry.metric));
            }
            let bit = 1_u64 << entry.metric.index();
            if seen & bit != 0 {
                return Err(AnalysisError::DuplicateWeight(entry.metric));
            }
            seen |= bit;
        }
        Ok(())
    }

    pub fn score(&self, metrics: &StrategicMetricVector) -> f64 {
        self.weights
            .iter()
            .map(|entry| metrics.get(entry.metric) * entry.weight)
            .sum()
    }

    pub fn try_score(&self, metrics: &StrategicMetricVector) -> Result<f64, AnalysisError> {
        self.validate()?;
        if !metrics.is_finite() {
            return Err(AnalysisError::NonFiniteMetric);
        }
        let score = self.score(metrics);
        if !score.is_finite() {
            return Err(AnalysisError::NonFiniteComposite);
        }
        Ok(score)
    }
}

/// The inventories required to close the resource-flow equations. Generation
/// includes reclaim and transfers received; spending includes transfers sent.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ResourceLedger {
    pub starting_metal: f64,
    pub ending_metal: f64,
    pub starting_energy: f64,
    pub ending_energy: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ResourceConservation {
    pub metal_error: f64,
    pub energy_error: f64,
    pub tolerance: f64,
    pub closes: bool,
}

impl ResourceLedger {
    pub fn conservation(
        &self,
        metrics: &StrategicMetricVector,
        tolerance: f64,
    ) -> ResourceConservation {
        let tolerance = tolerance.abs();
        let metal_error = self.starting_metal + metrics.get(StrategicMetric::MetalGenerated)
            - metrics.get(StrategicMetric::MetalSpent)
            - metrics.get(StrategicMetric::MetalOverflow)
            - self.ending_metal;
        let energy_error = self.starting_energy + metrics.get(StrategicMetric::EnergyGenerated)
            - metrics.get(StrategicMetric::EnergySpent)
            - metrics.get(StrategicMetric::EnergyOverflow)
            - self.ending_energy;
        ResourceConservation {
            metal_error,
            energy_error,
            tolerance,
            closes: metal_error.abs() <= tolerance && energy_error.abs() <= tolerance,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    Timeout,
    InvalidScenario,
    InvalidCommand,
    NumericalError,
    ResourceConservation,
    MissingCapability,
    InternalError,
}

pub const FAILURE_REASON_COUNT: usize = 7;

impl FailureReason {
    pub const ALL: [Self; FAILURE_REASON_COUNT] = [
        Self::Timeout,
        Self::InvalidScenario,
        Self::InvalidCommand,
        Self::NumericalError,
        Self::ResourceConservation,
        Self::MissingCapability,
        Self::InternalError,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

/// Capability bits recorded with every result. New capabilities should consume
/// a previously-unused bit so old analysis files remain comparable.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CapabilityFlags(pub u64);

impl CapabilityFlags {
    pub const TACTICAL_COMBAT: Self = Self(1 << 0);
    pub const ECONOMY: Self = Self(1 << 1);
    pub const CONSTRUCTION: Self = Self(1 << 2);
    pub const FACTORIES: Self = Self(1 << 3);
    pub const WALLS: Self = Self(1 << 4);
    pub const NATIVE_MAP: Self = Self(1 << 5);
    pub const LOS_RADAR: Self = Self(1 << 6);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MilestoneDeclaration {
    pub name: String,
    pub condition: MilestoneCondition,
    /// Score points awarded when this milestone condition is first satisfied.
    pub points: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MilestoneCondition {
    MetalProduced { threshold: f64 },
    EnergyProduced { threshold: f64 },
    UnitsProduced { count: u64 },
    UnitCompleted { definition: String },
    SitesOccupied { count: usize },
    MetalStored { threshold: f64 },
    EnergyStored { threshold: f64 },
    ArmyValue { threshold: f64 },
    TickReached { tick: u64 },
}

/// Canonical declared calibration approximation and sensitivity warnings.
pub fn declared_approximation_warnings() -> Vec<String> {
    vec![
        "ballistics: constant launch speed gravity arc root; weapon acceleration, bounce, and high trajectory toggling omitted".into(),
        "shields: repulsers absorb rather than physically deflect projectiles".into(),
        "interceptors: instantaneous deterministic resolution at launch; flight and midcourse retargeting omitted".into(),
        "collision: uniform clearance rather than distinct per-unit collision volumes".into(),
        "sensors: 2D horizontal sensor masks for sonar/jammer/radar approximations".into(),
    ]
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunMetadata {
    pub simulator_version: String,
    pub scenario_version: String,
    pub scenario_hash: String,
    pub catalog_hash: String,
    pub map_hash: String,
    pub capability_flags: CapabilityFlags,
    pub seed: u64,
    #[serde(default)]
    pub calibration_warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RunIdentity {
    pub simulator_version: String,
    pub scenario_version: String,
    pub scenario_hash: String,
    pub catalog_hash: String,
    pub map_hash: String,
    pub capability_flags: CapabilityFlags,
    pub calibration_warnings: Vec<String>,
}

impl RunMetadata {
    fn identity(&self) -> RunIdentity {
        RunIdentity {
            simulator_version: self.simulator_version.clone(),
            scenario_version: self.scenario_version.clone(),
            scenario_hash: self.scenario_hash.clone(),
            catalog_hash: self.catalog_hash.clone(),
            map_hash: self.map_hash.clone(),
            capability_flags: self.capability_flags,
            calibration_warnings: self.calibration_warnings.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct AnalyzedSide {
    pub metrics: StrategicMetricVector,
    pub composite_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AnalyzedRun {
    pub winner: Option<u8>,
    pub timed_out: bool,
    pub duration: f64,
    pub sides: [AnalyzedSide; 2],
    pub failure: Option<FailureReason>,
    pub metadata: RunMetadata,
}

impl AnalyzedRun {
    pub fn from_battle(
        battle: &BattleResult,
        metrics: [StrategicMetricVector; 2],
        weights: &CompositeWeights,
        metadata: RunMetadata,
        failure: Option<FailureReason>,
    ) -> Result<Self, AnalysisError> {
        weights.validate()?;
        if !metrics.iter().all(StrategicMetricVector::is_finite) {
            return Err(AnalysisError::NonFiniteMetric);
        }
        let composite_scores = [
            weights.try_score(&metrics[0])?,
            weights.try_score(&metrics[1])?,
        ];
        Ok(Self {
            winner: battle.winner,
            timed_out: battle.timed_out,
            duration: f64::from(battle.duration),
            sides: [
                AnalyzedSide {
                    metrics: metrics[0],
                    composite_score: composite_scores[0],
                },
                AnalyzedSide {
                    metrics: metrics[1],
                    composite_score: composite_scores[1],
                },
            ],
            failure,
            metadata,
        })
    }

    fn validate(&self) -> Result<(), AnalysisError> {
        if !self.duration.is_finite()
            || self.duration < 0.0
            || self
                .sides
                .iter()
                .any(|side| !side.composite_score.is_finite() || !side.metrics.is_finite())
        {
            return Err(AnalysisError::NonFiniteMetric);
        }
        if self.winner.is_some_and(|side| side >= 2) {
            return Err(AnalysisError::InvalidWinner);
        }
        Ok(())
    }

    fn effective_failure(&self) -> Option<FailureReason> {
        self.failure
            .or(self.timed_out.then_some(FailureReason::Timeout))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ScalarSummary {
    pub count: u64,
    pub mean: f64,
    pub standard_deviation: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub p05: f64,
    pub p50: f64,
    pub p95: f64,
}

#[derive(Clone, Copy, Debug)]
struct P2Quantile {
    probability: f64,
    initial: [f64; 5],
    initial_len: u8,
    heights: [f64; 5],
    positions: [i64; 5],
    desired: [f64; 5],
    increments: [f64; 5],
}

impl P2Quantile {
    fn new(probability: f64) -> Self {
        Self {
            probability,
            initial: [0.0; 5],
            initial_len: 0,
            heights: [0.0; 5],
            positions: [0; 5],
            desired: [0.0; 5],
            increments: [
                0.0,
                probability / 2.0,
                probability,
                (1.0 + probability) / 2.0,
                1.0,
            ],
        }
    }

    fn record(&mut self, value: f64) {
        if self.initial_len < 5 {
            self.initial[self.initial_len as usize] = value;
            self.initial_len += 1;
            if self.initial_len == 5 {
                self.initial.sort_by(f64::total_cmp);
                self.heights = self.initial;
                self.positions = [1, 2, 3, 4, 5];
                self.desired = [
                    1.0,
                    1.0 + 2.0 * self.probability,
                    1.0 + 4.0 * self.probability,
                    3.0 + 2.0 * self.probability,
                    5.0,
                ];
            }
            return;
        }

        let bucket = if value < self.heights[0] {
            self.heights[0] = value;
            0
        } else if value < self.heights[1] {
            0
        } else if value < self.heights[2] {
            1
        } else if value < self.heights[3] {
            2
        } else if value <= self.heights[4] {
            3
        } else {
            self.heights[4] = value;
            3
        };
        for position in &mut self.positions[bucket + 1..] {
            *position += 1;
        }
        for (desired, increment) in self.desired.iter_mut().zip(self.increments) {
            *desired += increment;
        }

        for index in 1..4 {
            let displacement = self.desired[index] - self.positions[index] as f64;
            let direction = if displacement >= 1.0
                && self.positions[index + 1] - self.positions[index] > 1
            {
                1
            } else if displacement <= -1.0 && self.positions[index - 1] - self.positions[index] < -1
            {
                -1
            } else {
                continue;
            };
            let candidate = self.parabolic(index, direction);
            self.heights[index] =
                if self.heights[index - 1] < candidate && candidate < self.heights[index + 1] {
                    candidate
                } else {
                    self.linear(index, direction)
                };
            self.positions[index] += direction;
        }
    }

    fn parabolic(&self, index: usize, direction: i64) -> f64 {
        let direction_f = direction as f64;
        let current = self.positions[index];
        let previous = self.positions[index - 1];
        let next = self.positions[index + 1];
        self.heights[index]
            + direction_f / (next - previous) as f64
                * (((current - previous + direction) as f64
                    * (self.heights[index + 1] - self.heights[index])
                    / (next - current) as f64)
                    + ((next - current - direction) as f64
                        * (self.heights[index] - self.heights[index - 1])
                        / (current - previous) as f64))
    }

    fn linear(&self, index: usize, direction: i64) -> f64 {
        let adjacent = (index as i64 + direction) as usize;
        self.heights[index]
            + direction as f64 * (self.heights[adjacent] - self.heights[index])
                / (self.positions[adjacent] - self.positions[index]) as f64
    }

    fn estimate(&self) -> f64 {
        if self.initial_len == 0 {
            return 0.0;
        }
        if self.initial_len < 5 {
            let mut samples = self.initial;
            samples[..self.initial_len as usize].sort_by(f64::total_cmp);
            let rank = self.probability * f64::from(self.initial_len - 1);
            let lower = rank.floor() as usize;
            let upper = rank.ceil() as usize;
            let fraction = rank - lower as f64;
            return samples[lower] * (1.0 - fraction) + samples[upper] * fraction;
        }
        self.heights[2]
    }
}

#[derive(Clone, Copy, Debug)]
struct OnlineScalar {
    count: u64,
    mean: f64,
    squared_deviation: f64,
    minimum: f64,
    maximum: f64,
    p05: P2Quantile,
    p50: P2Quantile,
    p95: P2Quantile,
}

impl Default for OnlineScalar {
    fn default() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            squared_deviation: 0.0,
            minimum: f64::INFINITY,
            maximum: f64::NEG_INFINITY,
            p05: P2Quantile::new(0.05),
            p50: P2Quantile::new(0.5),
            p95: P2Quantile::new(0.95),
        }
    }
}

impl OnlineScalar {
    fn record(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.squared_deviation += delta * (value - self.mean);
        self.minimum = self.minimum.min(value);
        self.maximum = self.maximum.max(value);
        self.p05.record(value);
        self.p50.record(value);
        self.p95.record(value);
    }

    fn summary(&self) -> ScalarSummary {
        ScalarSummary {
            count: self.count,
            mean: self.mean,
            standard_deviation: if self.count > 1 {
                (self.squared_deviation / (self.count - 1) as f64).sqrt()
            } else {
                0.0
            },
            minimum: if self.count == 0 { 0.0 } else { self.minimum },
            maximum: if self.count == 0 { 0.0 } else { self.maximum },
            p05: self.p05.estimate(),
            p50: self.p50.estimate(),
            p95: self.p95.estimate(),
        }
    }
}

#[derive(Clone, Debug)]
struct OnlineMetricVector {
    values: [OnlineScalar; STRATEGIC_METRIC_COUNT],
}

impl Default for OnlineMetricVector {
    fn default() -> Self {
        Self {
            values: array::from_fn(|_| OnlineScalar::default()),
        }
    }
}

impl OnlineMetricVector {
    fn record(&mut self, metrics: &StrategicMetricVector) {
        for (statistic, value) in self.values.iter_mut().zip(metrics.values) {
            statistic.record(value);
        }
    }

    fn summary(&self) -> Vec<MetricSummary> {
        StrategicMetric::ALL
            .into_iter()
            .zip(&self.values)
            .map(|(metric, value)| MetricSummary {
                metric,
                statistics: value.summary(),
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MetricSummary {
    pub metric: StrategicMetric,
    pub statistics: ScalarSummary,
}

#[derive(Clone, Debug, Default)]
struct OnlineMetadata {
    identity: Option<RunIdentity>,
    mismatched_runs: u64,
    minimum_seed: u64,
    maximum_seed: u64,
    seed_xor: u64,
    count: u64,
}

impl OnlineMetadata {
    fn record(&mut self, metadata: &RunMetadata) {
        if let Some(expected) = &self.identity {
            if expected.simulator_version != metadata.simulator_version
                || expected.scenario_version != metadata.scenario_version
                || expected.scenario_hash != metadata.scenario_hash
                || expected.catalog_hash != metadata.catalog_hash
                || expected.map_hash != metadata.map_hash
                || expected.capability_flags != metadata.capability_flags
            {
                self.mismatched_runs += 1;
            }
        } else {
            self.identity = Some(metadata.identity());
            self.minimum_seed = metadata.seed;
            self.maximum_seed = metadata.seed;
        }
        self.minimum_seed = self.minimum_seed.min(metadata.seed);
        self.maximum_seed = self.maximum_seed.max(metadata.seed);
        self.seed_xor ^= metadata.seed;
        self.count += 1;
    }

    fn summary(&self) -> MetadataSummary {
        let warnings = self
            .identity
            .as_ref()
            .map(|id| id.calibration_warnings.clone())
            .unwrap_or_default();
        MetadataSummary {
            identity: self.identity.clone(),
            mismatched_runs: self.mismatched_runs,
            minimum_seed: (self.count > 0).then_some(self.minimum_seed),
            maximum_seed: (self.count > 0).then_some(self.maximum_seed),
            seed_xor: self.seed_xor,
            calibration_warnings: warnings,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MetadataSummary {
    pub identity: Option<RunIdentity>,
    pub mismatched_runs: u64,
    pub minimum_seed: Option<u64>,
    pub maximum_seed: Option<u64>,
    /// Order-independent checksum useful for confirming that two aggregates used
    /// the same seed multiset. It is not a cryptographic digest.
    pub seed_xor: u64,
    pub calibration_warnings: Vec<String>,
}

/// Streaming analysis whose storage is fixed by the metric and failure
/// vocabularies, not by the number of recorded runs.
#[derive(Clone, Debug, Default)]
pub struct OnlineExperimentAggregate {
    runs: u64,
    side_wins: [u64; 2],
    draws: u64,
    timeouts: u64,
    failures: [u64; FAILURE_REASON_COUNT],
    duration: OnlineScalar,
    metrics: [OnlineMetricVector; 2],
    composite_scores: [OnlineScalar; 2],
    metadata: OnlineMetadata,
}

impl OnlineExperimentAggregate {
    pub fn record(&mut self, run: &AnalyzedRun) -> Result<(), AnalysisError> {
        run.validate()?;
        self.runs += 1;
        match run.winner {
            Some(side) => self.side_wins[side as usize] += 1,
            None => self.draws += 1,
        }
        if run.timed_out {
            self.timeouts += 1;
        }
        if let Some(reason) = run.effective_failure() {
            self.failures[reason.index()] += 1;
        }
        self.duration.record(run.duration);
        for index in 0..2 {
            self.metrics[index].record(&run.sides[index].metrics);
            self.composite_scores[index].record(run.sides[index].composite_score);
        }
        self.metadata.record(&run.metadata);
        Ok(())
    }

    pub fn summary(&self) -> ExperimentSummary {
        let metadata = self.metadata.summary();
        let calibration_warnings = metadata.calibration_warnings.clone();
        ExperimentSummary {
            runs: self.runs,
            side_wins: self.side_wins,
            draws: self.draws,
            timeouts: self.timeouts,
            failures: FailureReason::ALL
                .into_iter()
                .zip(self.failures)
                .map(|(reason, count)| FailureCount { reason, count })
                .collect(),
            duration: self.duration.summary(),
            metrics: self.metrics.each_ref().map(OnlineMetricVector::summary),
            composite_scores: self.composite_scores.each_ref().map(OnlineScalar::summary),
            metadata,
            calibration_warnings,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FailureCount {
    pub reason: FailureReason,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExperimentSummary {
    pub runs: u64,
    pub side_wins: [u64; 2],
    pub draws: u64,
    pub timeouts: u64,
    pub failures: Vec<FailureCount>,
    pub duration: ScalarSummary,
    pub metrics: [Vec<MetricSummary>; 2],
    pub composite_scores: [ScalarSummary; 2],
    pub metadata: MetadataSummary,
    pub calibration_warnings: Vec<String>,
}

impl ExperimentSummary {
    pub fn failure_count(&self, reason: FailureReason) -> u64 {
        self.failures[reason.index()].count
    }
}

/// Streaming paired-policy deltas. Positive values favor the candidate policy.
/// `record_side_swapped` removes fixed side bias by averaging candidate-minus-
/// baseline in both seatings before recording one paired observation.
#[derive(Clone, Debug, Default)]
pub struct OnlinePairedAggregate {
    accepted_pairs: u64,
    rejected_pairs: u64,
    failures: [u64; FAILURE_REASON_COUNT],
    metric_deltas: OnlineMetricVector,
    composite_delta: OnlineScalar,
}

impl OnlinePairedAggregate {
    pub fn record_delta(
        &mut self,
        candidate: &AnalyzedSide,
        baseline: &AnalyzedSide,
    ) -> Result<(), AnalysisError> {
        if !candidate.metrics.is_finite()
            || !baseline.metrics.is_finite()
            || !candidate.composite_score.is_finite()
            || !baseline.composite_score.is_finite()
        {
            return Err(AnalysisError::NonFiniteMetric);
        }
        self.accepted_pairs += 1;
        self.metric_deltas
            .record(&candidate.metrics.difference(baseline.metrics));
        self.composite_delta
            .record(candidate.composite_score - baseline.composite_score);
        Ok(())
    }

    /// `first` seats candidate on side 0; `swapped` seats candidate on side 1.
    /// Failed or timed-out constituent runs reject the whole pair.
    pub fn record_side_swapped(
        &mut self,
        first: &AnalyzedRun,
        swapped: &AnalyzedRun,
    ) -> Result<(), AnalysisError> {
        first.validate()?;
        swapped.validate()?;
        let failures = [first.effective_failure(), swapped.effective_failure()];
        if failures.iter().any(Option::is_some) {
            self.rejected_pairs += 1;
            for reason in failures.into_iter().flatten() {
                self.failures[reason.index()] += 1;
            }
            return Ok(());
        }
        let first_delta = first.sides[0].metrics.difference(first.sides[1].metrics);
        let swapped_delta = swapped.sides[1]
            .metrics
            .difference(swapped.sides[0].metrics);
        let metrics = first_delta.sum(swapped_delta).scaled(0.5);
        let composite = ((first.sides[0].composite_score - first.sides[1].composite_score)
            + (swapped.sides[1].composite_score - swapped.sides[0].composite_score))
            * 0.5;
        self.accepted_pairs += 1;
        self.metric_deltas.record(&metrics);
        self.composite_delta.record(composite);
        Ok(())
    }

    pub fn summary(&self) -> PairedSummary {
        PairedSummary {
            accepted_pairs: self.accepted_pairs,
            rejected_pairs: self.rejected_pairs,
            failures: FailureReason::ALL
                .into_iter()
                .zip(self.failures)
                .map(|(reason, count)| FailureCount { reason, count })
                .collect(),
            metric_deltas: self.metric_deltas.summary(),
            composite_delta: self.composite_delta.summary(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PairedSummary {
    pub accepted_pairs: u64,
    pub rejected_pairs: u64,
    pub failures: Vec<FailureCount>,
    pub metric_deltas: Vec<MetricSummary>,
    pub composite_delta: ScalarSummary,
}

impl PairedSummary {
    pub fn failure_count(&self, reason: FailureReason) -> u64 {
        self.failures[reason.index()].count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisError {
    DuplicateWeight(StrategicMetric),
    NonFiniteWeight(StrategicMetric),
    NonFiniteMetric,
    NonFiniteComposite,
    InvalidWinner,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateWeight(metric) => {
                write!(
                    formatter,
                    "duplicate composite weight for {}",
                    metric.name()
                )
            }
            Self::NonFiniteWeight(metric) => {
                write!(
                    formatter,
                    "non-finite composite weight for {}",
                    metric.name()
                )
            }
            Self::NonFiniteMetric => formatter.write_str("non-finite strategic metric"),
            Self::NonFiniteComposite => formatter.write_str("non-finite composite score"),
            Self::InvalidWinner => formatter.write_str("winner side must be zero or one"),
        }
    }
}

impl Error for AnalysisError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(seed: u64) -> RunMetadata {
        RunMetadata {
            simulator_version: "test".into(),
            scenario_version: "1".into(),
            scenario_hash: "scenario".into(),
            catalog_hash: "catalog".into(),
            map_hash: "map".into(),
            capability_flags: CapabilityFlags::TACTICAL_COMBAT,
            seed,
            calibration_warnings: Vec::new(),
        }
    }

    fn run(seed: u64, side_values: [f64; 2]) -> AnalyzedRun {
        let mut sides = [AnalyzedSide::default(); 2];
        for (side, value) in sides.iter_mut().zip(side_values) {
            side.metrics.set(StrategicMetric::ArmyValue, value);
            side.composite_score = value;
        }
        AnalyzedRun {
            winner: Some(if side_values[0] > side_values[1] {
                0
            } else {
                1
            }),
            timed_out: false,
            duration: 10.0,
            sides,
            failure: None,
            metadata: metadata(seed),
        }
    }

    #[test]
    fn declared_composite_is_hand_checkable() {
        let mut metrics = StrategicMetricVector::default();
        metrics.set(StrategicMetric::ArmyValue, 100.0);
        metrics.set(StrategicMetric::MetalOverflow, 8.0);
        metrics.set(StrategicMetric::WallLeaks, 2.0);
        let weights = CompositeWeights {
            weights: vec![
                MetricWeight {
                    metric: StrategicMetric::ArmyValue,
                    weight: 0.5,
                },
                MetricWeight {
                    metric: StrategicMetric::MetalOverflow,
                    weight: -2.0,
                },
                MetricWeight {
                    metric: StrategicMetric::WallLeaks,
                    weight: -5.0,
                },
            ],
        };
        assert_eq!(weights.try_score(&metrics), Ok(24.0));
        assert_eq!(CompositeWeights::default().score(&metrics), 0.0);
    }

    #[test]
    fn resource_conservation_reports_signed_error() {
        let mut metrics = StrategicMetricVector::default();
        metrics.set(StrategicMetric::MetalGenerated, 90.0);
        metrics.set(StrategicMetric::MetalSpent, 60.0);
        metrics.set(StrategicMetric::MetalOverflow, 10.0);
        metrics.set(StrategicMetric::EnergyGenerated, 200.0);
        metrics.set(StrategicMetric::EnergySpent, 175.0);
        let ledger = ResourceLedger {
            starting_metal: 50.0,
            ending_metal: 70.0,
            starting_energy: 25.0,
            ending_energy: 50.000_001,
        };
        let conservation = ledger.conservation(&metrics, 0.000_01);
        assert_eq!(conservation.metal_error, 0.0);
        assert!((conservation.energy_error + 0.000_001).abs() < 1e-10);
        assert!(conservation.closes);
    }

    #[test]
    fn online_statistics_are_bounded_and_hand_checkable() {
        let bytes_before = std::mem::size_of::<OnlineExperimentAggregate>();
        let mut aggregate = OnlineExperimentAggregate::default();
        for seed in 1..=1000 {
            aggregate.record(&run(seed, [seed as f64, 0.0])).unwrap();
        }
        let summary = aggregate.summary();
        assert_eq!(std::mem::size_of_val(&aggregate), bytes_before);
        assert_eq!(summary.runs, 1000);
        assert_eq!(summary.side_wins, [1000, 0]);
        let army = &summary.metrics[0][StrategicMetric::ArmyValue.index()].statistics;
        assert_eq!(army.mean, 500.5);
        assert!((army.p50 - 500.0).abs() < 10.0);
        assert!((army.p95 - 950.0).abs() < 20.0);
        assert_eq!(summary.metadata.minimum_seed, Some(1));
        assert_eq!(summary.metadata.maximum_seed, Some(1000));
    }

    #[test]
    fn identical_side_swapped_policies_have_zero_delta() {
        let first = run(1, [80.0, 60.0]);
        let swapped = run(2, [80.0, 60.0]);
        let mut paired = OnlinePairedAggregate::default();
        paired.record_side_swapped(&first, &swapped).unwrap();
        let summary = paired.summary();
        assert_eq!(summary.accepted_pairs, 1);
        assert_eq!(summary.composite_delta.mean, 0.0);
        assert_eq!(
            summary.metric_deltas[StrategicMetric::ArmyValue.index()]
                .statistics
                .mean,
            0.0
        );
    }

    #[test]
    fn failures_and_metadata_mismatches_are_counted_without_run_storage() {
        let mut aggregate = OnlineExperimentAggregate::default();
        let good = run(4, [2.0, 1.0]);
        aggregate.record(&good).unwrap();
        let mut timeout = run(9, [2.0, 1.0]);
        timeout.timed_out = true;
        timeout.winner = None;
        timeout.metadata.map_hash = "different".into();
        aggregate.record(&timeout).unwrap();
        let summary = aggregate.summary();
        assert_eq!(summary.timeouts, 1);
        assert_eq!(summary.failures[FailureReason::Timeout.index()].count, 1);
        assert_eq!(summary.metadata.mismatched_runs, 1);
    }
}
