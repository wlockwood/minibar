//! Bounded, deterministic strategic experiment execution.
//!
//! Parallelism is across independent [`World`] instances. Results are reduced
//! in seed order so Rayon scheduling cannot perturb streaming quantiles or
//! retained output. By default only fixed-size online aggregates are kept;
//! retaining every trial is an explicit debugging choice.

use std::fmt;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::model::DT;
use crate::score::{
    AnalysisError, AnalyzedRun, AnalyzedSide, CapabilityFlags, CompositeWeights, ExperimentSummary,
    FailureReason, OnlineExperimentAggregate, OnlinePairedAggregate, PairedSummary,
    ResourceConservation, RunMetadata,
};
use crate::strategy::{SideId, Strategy, StrategyCadence, StrategyController};
use crate::world::{World, WorldError};

/// Whether an experiment keeps per-trial state after updating its aggregates.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialRetention {
    /// Fixed-size storage independent of the number of trials.
    #[default]
    AggregateOnly,
    /// Keep trials in ascending seed order for replay and debugging.
    All,
}

/// Metadata shared by all trials. The runner fills in the trial seed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExperimentMetadata {
    pub simulator_version: String,
    pub scenario_version: String,
    pub scenario_hash: String,
    pub catalog_hash: String,
    pub map_hash: String,
    pub capability_flags: CapabilityFlags,
    #[serde(default = "crate::score::declared_approximation_warnings")]
    pub calibration_warnings: Vec<String>,
}

impl ExperimentMetadata {
    fn for_seed(&self, seed: u64) -> RunMetadata {
        RunMetadata {
            simulator_version: self.simulator_version.clone(),
            scenario_version: self.scenario_version.clone(),
            scenario_hash: self.scenario_hash.clone(),
            catalog_hash: self.catalog_hash.clone(),
            map_hash: self.map_hash.clone(),
            capability_flags: self.capability_flags,
            seed,
            calibration_warnings: self.calibration_warnings.clone(),
        }
    }
}

/// Bounds and analysis settings for a strategic batch.
#[derive(Clone, Debug)]
pub struct StrategicExperimentConfig {
    pub trials: usize,
    pub first_seed: u64,
    pub max_ticks: u64,
    /// Maximum number of completed trials temporarily held before aggregation.
    pub parallel_batch_size: usize,
    pub conservation_tolerance: f64,
    pub retention: TrialRetention,
    pub policy_a_cadence: StrategyCadence,
    pub policy_b_cadence: StrategyCadence,
    pub weights: CompositeWeights,
    pub metadata: ExperimentMetadata,
}

impl StrategicExperimentConfig {
    fn validate(&self) -> Result<(), ExperimentError> {
        if self.parallel_batch_size == 0 {
            return Err(ExperimentError::ZeroParallelBatchSize);
        }
        if !self.conservation_tolerance.is_finite() || self.conservation_tolerance < 0.0 {
            return Err(ExperimentError::InvalidConservationTolerance);
        }
        let trial_span = u64::try_from(self.trials.saturating_sub(1))
            .map_err(|_| ExperimentError::SeedRangeOverflow)?;
        if self.trials > 0 && self.first_seed.checked_add(trial_span).is_none() {
            return Err(ExperimentError::SeedRangeOverflow);
        }
        self.weights.validate().map_err(ExperimentError::Analysis)
    }

    fn seed(&self, index: usize) -> u64 {
        // Checked once for the complete range in `validate`.
        self.first_seed + index as u64
    }
}

/// Identifies one seating. Policy A is the candidate in paired analysis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrialContext {
    pub seed: u64,
    pub policy_a_side: SideId,
}

/// Supplied to each strategy factory once per trial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StrategyInstanceContext {
    pub seed: u64,
    pub side: SideId,
}

/// A scenario-defined terminal result. Returning `None` means keep stepping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrialConclusion {
    Winner(SideId),
    Draw,
}

impl TrialConclusion {
    fn winner(self) -> Option<SideId> {
        match self {
            Self::Winner(side) => Some(side),
            Self::Draw => None,
        }
    }
}

/// Optional per-trial output. This is only accumulated with `TrialRetention::All`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StrategicTrialResult {
    pub policy_a_side: SideId,
    pub final_state_hash: u64,
    pub conservation: [ResourceConservation; 2],
    pub analysis: AnalyzedRun,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StrategicExperimentReport {
    pub summary: ExperimentSummary,
    pub retained_trials: Option<Vec<StrategicTrialResult>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PairedStrategicExperimentReport {
    /// Summary across both seatings (two runs per logical seed).
    pub all_runs: ExperimentSummary,
    /// Candidate-minus-baseline deltas after removing fixed side bias.
    pub paired: PairedSummary,
    /// Ordered by seed, then candidate on side 0 followed by candidate on side 1.
    pub retained_trials: Option<Vec<StrategicTrialResult>>,
}

#[derive(Debug)]
pub enum ExperimentError {
    ZeroParallelBatchSize,
    InvalidConservationTolerance,
    SeedRangeOverflow,
    InvalidTerminalSide(SideId),
    World {
        seed: u64,
        policy_a_side: SideId,
        source: WorldError,
    },
    Analysis(AnalysisError),
}

impl fmt::Display for ExperimentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroParallelBatchSize => {
                formatter.write_str("parallel experiment batch size must be non-zero")
            }
            Self::InvalidConservationTolerance => formatter
                .write_str("resource conservation tolerance must be finite and non-negative"),
            Self::SeedRangeOverflow => formatter.write_str("experiment seed range overflows u64"),
            Self::InvalidTerminalSide(side) => {
                write!(formatter, "terminal condition returned invalid side {side}")
            }
            Self::World {
                seed,
                policy_a_side,
                source,
            } => write!(
                formatter,
                "strategic world failed for seed {seed} with policy A on side {policy_a_side}: {source}"
            ),
            Self::Analysis(source) => write!(formatter, "experiment analysis failed: {source}"),
        }
    }
}

impl std::error::Error for ExperimentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::World { source, .. } => Some(source),
            Self::Analysis(source) => Some(source),
            _ => None,
        }
    }
}

/// Runs policy A on side 0 and policy B on side 1 for each scheduled seed.
///
/// The factories are called inside the worker for every trial, so stateful
/// strategies are never shared between trials.
pub fn run_strategic_experiment<WF, AF, BF, A, B, TC>(
    config: &StrategicExperimentConfig,
    world_factory: WF,
    policy_a_factory: AF,
    policy_b_factory: BF,
    terminal: TC,
) -> Result<StrategicExperimentReport, ExperimentError>
where
    WF: Fn(TrialContext) -> Result<World, WorldError> + Sync,
    AF: Fn(StrategyInstanceContext) -> A + Sync,
    BF: Fn(StrategyInstanceContext) -> B + Sync,
    A: Strategy,
    B: Strategy,
    TC: Fn(&World) -> Option<TrialConclusion> + Sync,
{
    config.validate()?;
    let mut aggregate = OnlineExperimentAggregate::default();
    let mut retained = retained_storage(config.retention, config.trials);
    run_bounded_parallel(
        config,
        |index| {
            run_trial(
                config,
                TrialContext {
                    seed: config.seed(index),
                    policy_a_side: 0,
                },
                &world_factory,
                &policy_a_factory,
                &policy_b_factory,
                &terminal,
            )
        },
        |trial| {
            aggregate
                .record(&trial.analysis)
                .map_err(ExperimentError::Analysis)?;
            if let Some(retained) = &mut retained {
                retained.push(trial);
            }
            Ok(())
        },
    )?;
    Ok(StrategicExperimentReport {
        summary: aggregate.summary(),
        retained_trials: retained,
    })
}

/// Runs a common-random-number pair per seed, swapping policy A between sides.
pub fn run_side_swapped_experiment<WF, AF, BF, A, B, TC>(
    config: &StrategicExperimentConfig,
    world_factory: WF,
    policy_a_factory: AF,
    policy_b_factory: BF,
    terminal: TC,
) -> Result<PairedStrategicExperimentReport, ExperimentError>
where
    WF: Fn(TrialContext) -> Result<World, WorldError> + Sync,
    AF: Fn(StrategyInstanceContext) -> A + Sync,
    BF: Fn(StrategyInstanceContext) -> B + Sync,
    A: Strategy,
    B: Strategy,
    TC: Fn(&World) -> Option<TrialConclusion> + Sync,
{
    config.validate()?;
    let mut aggregate = OnlineExperimentAggregate::default();
    let mut paired = OnlinePairedAggregate::default();
    let retained_capacity = config.trials.saturating_mul(2);
    let mut retained = retained_storage(config.retention, retained_capacity);
    run_bounded_parallel(
        config,
        |index| {
            let seed = config.seed(index);
            let first = run_trial(
                config,
                TrialContext {
                    seed,
                    policy_a_side: 0,
                },
                &world_factory,
                &policy_a_factory,
                &policy_b_factory,
                &terminal,
            )?;
            let swapped = run_trial(
                config,
                TrialContext {
                    seed,
                    policy_a_side: 1,
                },
                &world_factory,
                &policy_a_factory,
                &policy_b_factory,
                &terminal,
            )?;
            Ok((first, swapped))
        },
        |(first, swapped)| {
            aggregate
                .record(&first.analysis)
                .map_err(ExperimentError::Analysis)?;
            aggregate
                .record(&swapped.analysis)
                .map_err(ExperimentError::Analysis)?;
            paired
                .record_side_swapped(&first.analysis, &swapped.analysis)
                .map_err(ExperimentError::Analysis)?;
            if let Some(retained) = &mut retained {
                retained.push(first);
                retained.push(swapped);
            }
            Ok(())
        },
    )?;
    Ok(PairedStrategicExperimentReport {
        all_runs: aggregate.summary(),
        paired: paired.summary(),
        retained_trials: retained,
    })
}

fn retained_storage<T>(retention: TrialRetention, capacity: usize) -> Option<Vec<T>> {
    match retention {
        TrialRetention::AggregateOnly => None,
        TrialRetention::All => Some(Vec::with_capacity(capacity)),
    }
}

fn run_bounded_parallel<T, Produce, Consume>(
    config: &StrategicExperimentConfig,
    produce: Produce,
    mut consume: Consume,
) -> Result<(), ExperimentError>
where
    T: Send,
    Produce: Fn(usize) -> Result<T, ExperimentError> + Sync,
    Consume: FnMut(T) -> Result<(), ExperimentError>,
{
    let mut start = 0;
    while start < config.trials {
        let end = start
            .saturating_add(config.parallel_batch_size)
            .min(config.trials);
        let batch: Vec<_> = (start..end).into_par_iter().map(&produce).collect();
        // Indexed collection preserves seed order. Aggregation stays sequential
        // because the bounded P² estimators are intentionally order-sensitive.
        for result in batch {
            consume(result?)?;
        }
        start = end;
    }
    Ok(())
}

fn run_trial<WF, AF, BF, A, B, TC>(
    config: &StrategicExperimentConfig,
    context: TrialContext,
    world_factory: &WF,
    policy_a_factory: &AF,
    policy_b_factory: &BF,
    terminal: &TC,
) -> Result<StrategicTrialResult, ExperimentError>
where
    WF: Fn(TrialContext) -> Result<World, WorldError> + Sync,
    AF: Fn(StrategyInstanceContext) -> A + Sync,
    BF: Fn(StrategyInstanceContext) -> B + Sync,
    A: Strategy,
    B: Strategy,
    TC: Fn(&World) -> Option<TrialConclusion> + Sync,
{
    let mut world = world_factory(context).map_err(|source| ExperimentError::World {
        seed: context.seed,
        policy_a_side: context.policy_a_side,
        source,
    })?;
    let policy_b_side = 1 - context.policy_a_side;
    let mut policy_a = StrategyController::new(
        context.policy_a_side,
        config.policy_a_cadence,
        policy_a_factory(StrategyInstanceContext {
            seed: context.seed,
            side: context.policy_a_side,
        }),
    );
    let mut policy_b = StrategyController::new(
        policy_b_side,
        config.policy_b_cadence,
        policy_b_factory(StrategyInstanceContext {
            seed: context.seed,
            side: policy_b_side,
        }),
    );

    let mut conclusion = checked_conclusion(terminal(&world))?;
    let mut failure = None;
    while conclusion.is_none() && world.tick < config.max_ticks {
        let policy_a_observation =
            world
                .observation(policy_a.side())
                .map_err(|source| ExperimentError::World {
                    seed: context.seed,
                    policy_a_side: context.policy_a_side,
                    source,
                })?;
        let policy_b_observation =
            world
                .observation(policy_b.side())
                .map_err(|source| ExperimentError::World {
                    seed: context.seed,
                    policy_a_side: context.policy_a_side,
                    source,
                })?;
        world.enqueue_actions(policy_a.poll(&policy_a_observation));
        world.enqueue_actions(policy_b.poll(&policy_b_observation));
        world.step();
        if world
            .action_decisions
            .iter()
            .any(|decision| decision.result.is_err())
        {
            failure = Some(FailureReason::InvalidCommand);
            break;
        }
        conclusion = checked_conclusion(terminal(&world))?;
    }
    let timed_out = conclusion.is_none() && failure.is_none() && world.tick >= config.max_ticks;
    let winner = conclusion.and_then(TrialConclusion::winner);

    let metrics = [
        world
            .strategic_metrics(0)
            .map_err(|source| ExperimentError::World {
                seed: context.seed,
                policy_a_side: context.policy_a_side,
                source,
            })?,
        world
            .strategic_metrics(1)
            .map_err(|source| ExperimentError::World {
                seed: context.seed,
                policy_a_side: context.policy_a_side,
                source,
            })?,
    ];
    let ledgers = [
        world
            .resource_ledger(0)
            .map_err(|source| ExperimentError::World {
                seed: context.seed,
                policy_a_side: context.policy_a_side,
                source,
            })?,
        world
            .resource_ledger(1)
            .map_err(|source| ExperimentError::World {
                seed: context.seed,
                policy_a_side: context.policy_a_side,
                source,
            })?,
    ];
    let conservation = [
        ledgers[0].conservation(&metrics[0], config.conservation_tolerance),
        ledgers[1].conservation(&metrics[1], config.conservation_tolerance),
    ];
    if failure.is_none() && conservation.iter().any(|check| !check.closes) {
        failure = Some(FailureReason::ResourceConservation);
    }
    let sides = [
        AnalyzedSide {
            metrics: metrics[0],
            composite_score: config
                .weights
                .try_score(&metrics[0])
                .map_err(ExperimentError::Analysis)?,
        },
        AnalyzedSide {
            metrics: metrics[1],
            composite_score: config
                .weights
                .try_score(&metrics[1])
                .map_err(ExperimentError::Analysis)?,
        },
    ];
    Ok(StrategicTrialResult {
        policy_a_side: context.policy_a_side,
        final_state_hash: world.state_hash(),
        conservation,
        analysis: AnalyzedRun {
            winner,
            timed_out,
            duration: world.tick as f64 * f64::from(DT),
            sides,
            failure,
            metadata: config.metadata.for_seed(context.seed),
        },
    })
}

fn checked_conclusion(
    conclusion: Option<TrialConclusion>,
) -> Result<Option<TrialConclusion>, ExperimentError> {
    if let Some(TrialConclusion::Winner(side)) = conclusion
        && side >= 2
    {
        return Err(ExperimentError::InvalidTerminalSide(side));
    }
    Ok(conclusion)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::map::{CompiledMap, HelloMapOptions, MapSource};
    use crate::model::{MovementDef, StrategicDef, UnitDef, Vec2};
    use crate::placement::BuildFacing;
    use crate::score::{MetricWeight, StrategicMetric};
    use crate::strategy::{ActionEmitter, StrategyAction, StrategyObservation};
    use crate::world::{UnitDefId, WorldCatalog, WorldConfig};

    #[derive(Default)]
    struct CountingStrategy {
        updates: Arc<AtomicUsize>,
    }

    impl Strategy for CountingStrategy {
        fn update(&mut self, _observation: &StrategyObservation, _actions: &mut ActionEmitter) {
            self.updates.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct InvalidStrategy;

    impl Strategy for InvalidStrategy {
        fn update(&mut self, _observation: &StrategyObservation, actions: &mut ActionEmitter) {
            actions.emit(StrategyAction::Move {
                entities: Vec::new(),
                destination: Vec2::new(1.0, 1.0),
                mode: Default::default(),
            });
        }
    }

    fn map() -> Arc<CompiledMap> {
        let json = r#"{
            "map_name":"experiment", "map_size":[128,128], "water_level":0,
            "terrain":{"heights":[[0,0],[0,0]],
              "movement_classes":{"BOT":[[true,true],[true,true]]}},
            "metal_spots":[], "geothermal_spots":[]
        }"#;
        Arc::new(
            CompiledMap::compile(
                MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap(),
            )
            .unwrap(),
        )
    }

    fn catalog() -> Arc<WorldCatalog> {
        let unit = UnitDef {
            name: "marker".into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 10.0,
            energy_cost: 0.0,
            strategic: StrategicDef {
                is_mobile: true,
                ..Default::default()
            },
            movement: MovementDef {
                radius: 4.0,
                height: 8.0,
                ..Default::default()
            },
            weapons: Vec::new(),
        };
        Arc::new(WorldCatalog::new(vec![unit]).unwrap())
    }

    fn world() -> Result<World, WorldError> {
        World::new(map(), catalog(), WorldConfig::default())
    }

    fn config(trials: usize, retention: TrialRetention) -> StrategicExperimentConfig {
        StrategicExperimentConfig {
            trials,
            first_seed: 40,
            max_ticks: 5,
            parallel_batch_size: 3,
            conservation_tolerance: 1e-9,
            retention,
            policy_a_cadence: StrategyCadence::new(2, 0).unwrap(),
            policy_b_cadence: StrategyCadence::new(2, 0).unwrap(),
            weights: CompositeWeights {
                weights: vec![MetricWeight {
                    metric: StrategicMetric::ArmyValue,
                    weight: 1.0,
                }],
            },
            metadata: ExperimentMetadata {
                simulator_version: "test".into(),
                scenario_version: "1".into(),
                scenario_hash: "scenario".into(),
                catalog_hash: "catalog".into(),
                map_hash: "map".into(),
                capability_flags: CapabilityFlags::ECONOMY,
                calibration_warnings: Vec::new(),
            },
        }
    }

    #[test]
    fn fresh_strategies_follow_cadence_and_default_storage_is_bounded() {
        let a_updates = Arc::new(AtomicUsize::new(0));
        let b_updates = Arc::new(AtomicUsize::new(0));
        let report = run_strategic_experiment(
            &config(7, TrialRetention::AggregateOnly),
            |_| world(),
            |_| CountingStrategy {
                updates: Arc::clone(&a_updates),
            },
            |_| CountingStrategy {
                updates: Arc::clone(&b_updates),
            },
            |_| None,
        )
        .unwrap();
        assert_eq!(report.summary.runs, 7);
        assert_eq!(report.summary.timeouts, 7);
        assert_eq!(a_updates.load(Ordering::Relaxed), 21);
        assert_eq!(b_updates.load(Ordering::Relaxed), 21);
        assert!(report.retained_trials.is_none());
    }

    #[test]
    fn retained_results_and_streaming_quantiles_are_schedule_stable() {
        let run = |batch_size| {
            let mut config = config(12, TrialRetention::All);
            config.parallel_batch_size = batch_size;
            run_strategic_experiment(
                &config,
                |_| world(),
                |_| CountingStrategy::default(),
                |_| CountingStrategy::default(),
                |world| (world.tick == 3).then_some(TrialConclusion::Draw),
            )
            .unwrap()
        };
        let one_at_a_time = run(1);
        let many_at_a_time = run(7);
        assert_eq!(one_at_a_time, many_at_a_time);
        let trials = one_at_a_time.retained_trials.unwrap();
        assert_eq!(trials.len(), 12);
        assert!(
            trials
                .windows(2)
                .all(|pair| { pair[0].analysis.metadata.seed < pair[1].analysis.metadata.seed })
        );
        assert!(trials.iter().all(|trial| {
            !trial.analysis.timed_out
                && trial.analysis.duration == 3.0 * f64::from(DT)
                && trial.conservation.iter().all(|check| check.closes)
        }));
    }

    #[test]
    fn rejected_commands_and_broken_resource_ledgers_are_failures() {
        let invalid = run_strategic_experiment(
            &config(1, TrialRetention::All),
            |_| world(),
            |_| InvalidStrategy,
            |_| CountingStrategy::default(),
            |_| None,
        )
        .unwrap();
        assert_eq!(
            invalid.summary.failure_count(FailureReason::InvalidCommand),
            1
        );
        assert!(!invalid.retained_trials.unwrap()[0].analysis.timed_out);

        let conservation = run_strategic_experiment(
            &config(1, TrialRetention::All),
            |_| {
                let mut world = world()?;
                world.teams[0].economy.metal.stored += 1.0;
                Ok(world)
            },
            |_| CountingStrategy::default(),
            |_| CountingStrategy::default(),
            |_| Some(TrialConclusion::Draw),
        )
        .unwrap();
        assert_eq!(
            conservation
                .summary
                .failure_count(FailureReason::ResourceConservation),
            1
        );
        assert!(!conservation.retained_trials.unwrap()[0].conservation[0].closes);
    }

    #[test]
    fn paired_runs_swap_candidate_side_and_preserve_seed_order() {
        let report = run_side_swapped_experiment(
            &config(4, TrialRetention::All),
            |context| {
                let mut world = world()?;
                world.spawn_active(
                    context.policy_a_side,
                    UnitDefId(0),
                    Vec2::new(32.0, 32.0),
                    BuildFacing::South,
                )?;
                Ok(world)
            },
            |_| CountingStrategy::default(),
            |_| CountingStrategy::default(),
            |world| {
                let owner = world
                    .entities
                    .iter()
                    .next()
                    .map(|(_, entity)| entity.owner)?;
                Some(TrialConclusion::Winner(owner))
            },
        )
        .unwrap();
        assert_eq!(report.all_runs.runs, 8);
        assert_eq!(report.all_runs.side_wins, [4, 4]);
        assert_eq!(report.paired.accepted_pairs, 4);
        assert_eq!(report.paired.composite_delta.mean, 10.0);
        let trials = report.retained_trials.unwrap();
        for (index, pair) in trials.chunks_exact(2).enumerate() {
            assert_eq!(pair[0].analysis.metadata.seed, 40 + index as u64);
            assert_eq!(pair[1].analysis.metadata.seed, 40 + index as u64);
            assert_eq!(pair[0].policy_a_side, 0);
            assert_eq!(pair[1].policy_a_side, 1);
        }
    }

    #[test]
    fn invalid_bounds_are_rejected_before_starting_workers() {
        let mut invalid = config(1, TrialRetention::AggregateOnly);
        invalid.parallel_batch_size = 0;
        let error = run_strategic_experiment(
            &invalid,
            |_| world(),
            |_| CountingStrategy::default(),
            |_| CountingStrategy::default(),
            |_| None,
        )
        .unwrap_err();
        assert!(matches!(error, ExperimentError::ZeroParallelBatchSize));

        invalid.parallel_batch_size = 1;
        invalid.first_seed = u64::MAX;
        invalid.trials = 2;
        let error = run_strategic_experiment(
            &invalid,
            |_| world(),
            |_| CountingStrategy::default(),
            |_| CountingStrategy::default(),
            |_| None,
        )
        .unwrap_err();
        assert!(matches!(error, ExperimentError::SeedRangeOverflow));
    }
}
