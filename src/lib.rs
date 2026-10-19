pub mod assets;
pub mod calibration;
pub mod catalog;
pub mod construction;
pub mod economy;
pub mod engine;
pub mod entity;
pub mod experiment;
pub mod learn;
pub mod live;
pub mod live_server;
pub mod map;
pub mod model;
pub mod occupancy;
pub mod placement;
pub mod policy;
pub mod routing;
pub mod scenario_compile;
pub mod score;
pub mod sensors;
mod spatial;
pub mod strategy;
pub mod trace;
pub mod viewer;
pub mod viewer_protocol;
pub mod world;
mod wrecks;

pub use calibration::{
    CALIBRATION_FIXTURE_VERSION, CalibrationDiagnostic, CalibrationFixture, CalibrationNotice,
    CalibrationReport, CalibrationWarning, CheckExpectation, CheckObservation, CheckReport,
    EventObservation, ExpectedCheck, FailureCode, FixtureMetadata, MeasuredCheck,
    MeasuredObservations, MechanicReport, NoticeKind, ObservationValue, SnapshotObservation,
    StatisticalExpectation, StatisticalObservation, WarningCode, compare_calibration,
};
pub use engine::{
    CombatCommand, CombatDeath, CombatDefinitionId, CombatError, CombatKey, CombatSpawn,
    CombatStepSummary, CombatStepper, CombatantUpdate, NEUTRAL_COMBAT_SIDE, Scenario, Simulator,
};
pub use entity::{EntityId, EntityStore};
pub use experiment::{
    ExperimentError, ExperimentMetadata, PairedStrategicExperimentReport,
    StrategicExperimentConfig, StrategicExperimentReport, StrategicTrialResult,
    StrategyInstanceContext, TrialConclusion, TrialContext, TrialRetention,
    run_side_swapped_experiment, run_strategic_experiment,
};
pub use live::{
    AdvanceReport, BuildPreviewRequest, ClientEnvelope, ClientMessage, CommandAck,
    CommandRejection, CommandResult, ControlError, LiveDriver, LiveState, LoggedAction,
    ReplayError, ReplayLog, ReplayVerification, ServerEnvelope, ServerError, ServerMessage,
    SideSequence, SimulationSpeed, SpeedError, StepWorld, replay_and_verify,
};
pub use live_server::{
    LiveServerConfig, LiveServerError, LiveServerHandle, ObservationEnvelope, start_live_server,
    start_scenario_live_server, start_world_live_server,
};
pub use map::{
    CompiledMap, ContinuousMetalSource, GeothermalNode, MapError, MapInputFormat, MapLoadFormat,
    MapLoadOptions, MapProvenance, MapSource, NativeMapIdentity, PassabilitySource, ResourceNode,
    ScalarGridSource, StaticBlocker,
};
pub use model::{
    BuildConstraints, CorpseDef, DeathExplosionDef, MovementDef, ReclaimDef, Spawn,
    StartingResources, StrategicDef, Terrain, UnitDef, Vec2, WeaponDef, WorldView,
};
pub use occupancy::{
    CellContribution, CellLayers, ExtractorSite, OccupancyCell, OccupancyGrid, OccupancyLayerKind,
    OccupantId,
};
pub use placement::{
    BUILD_GRID_SIZE, BuildFacing, BuilderContext, HALF_GRID_SIZE, PlacementConstraints,
    PlacementRejection, PlacementRequest, PlacementResolution, ResourceRequirements, Yardmap,
    YardmapStatus, execute_placement, snap_build_coord, snap_position, validate_placement,
};
pub use policy::{BuiltinPolicy, Policy};
pub use routing::{GridRouter, Route, RouteCache, RouteRequest, RouteStatus};
pub use scenario_compile::{
    CompiledScenarioSide, DeclarativeScenarioRuntime, ScenarioCompileError, ScenarioCompileOptions,
    ScenarioEntityIndex, SelectorUse, SideEntityIndex, compile_declarative_scenario,
    compile_declarative_scenario_json,
};
pub use score::{
    AnalysisError, AnalyzedRun, AnalyzedSide, BatchSummary, BattleResult, CapabilityFlags,
    CompositeWeights, ExperimentSummary, FAILURE_REASON_COUNT, FailureCount, FailureReason,
    MeanSideScore, MetadataSummary, MetricSummary, MetricWeight, MilestoneCondition,
    MilestoneDeclaration, OnlineExperimentAggregate, OnlinePairedAggregate, PairedSummary,
    ResourceConservation, ResourceLedger, RunIdentity, RunMetadata, STRATEGIC_METRIC_COUNT,
    ScalarSummary, SideScore, StrategicMetric, StrategicMetricVector,
    declared_approximation_warnings,
};
pub use strategy::{
    ActionCapabilities, ActionDecision, ActionEmitter, ActionInbox, ActionRejection,
    ActionSequence, DeclarativeScenario, EntitySelector, EnvelopmentStrategy, ExpansionStrategy,
    FactoryQueueItem, FormationMember, FormationPattern, FormationSpec, ObservationPolicy,
    ObservationRules, ObservationVisibility, ObservedEntity, ObservedFireIntent,
    ObservedResourceSite, OpeningInstruction, OpeningPlan, OpeningStep, OrderMode, QueuedAction,
    ResourceObservation, ScenarioKind, ScenarioSide, ScriptedStrategy, SideId, Strategy,
    StrategyAction, StrategyCadence, StrategyConfigError, StrategyController, StrategyEntityKind,
    StrategyObservation, TimedAction, WallDefenseStrategy,
};
pub use viewer_protocol::{
    SHARED_VIEWER_HTML, VIEWER_CONFIG_SCHEMA, VIEWER_FRAME_SCHEMA, VIEWER_PROTOCOL_VERSION,
    ViewerAnnotation, ViewerCapabilities, ViewerConfig, ViewerDecision, ViewerEntity, ViewerFrame,
    ViewerFrameHeader, ViewerMap, ViewerMode, ViewerProtocolError,
};
pub use world::{
    BuildPreview, BuilderTask, BuilderTaskState, EntityLifecycle, EntityRole, ReclaimState,
    UnitDefId, World, WorldCatalog, WorldConfig, WorldEntity, WorldError, WorldEvent, WorldMetrics,
    WorldTeam,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str, hp: f32, damage: f32, damage_duration: f32) -> UnitDef {
        UnitDef {
            flight: None,
            name: name.into(),
            icon_type: None,
            health: hp,
            metal_cost: 50.0,
            energy_cost: 0.0,
            strategic: StrategicDef::default(),
            movement: MovementDef {
                max_speed: 87.0,
                acceleration: 372.6,
                braking: 621.0,
                turn_rate: 194.3,
                radius: 11.0,
                ..Default::default()
            },
            weapons: vec![WeaponDef {
                name: "test".into(),
                damage,
                damage_duration,
                range: 180.0,
                reload: 1.0,
                hitscan: true,
                turret_yaw_speed: Some(300.0),
                ..Default::default()
            }],
        }
    }

    fn asymmetric_scenario(left: UnitDef, right: UnitDef) -> Scenario {
        Scenario {
            terrain: Terrain::flat(1_000.0, 500.0, 32.0),
            units: vec![
                Spawn {
                    side: 0,
                    definition: left,
                    position: Vec2::new(400.0, 250.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: right,
                    position: Vec2::new(500.0, 250.0),
                    heading: std::f32::consts::PI,
                },
            ],
            max_time: 10.0,
            trace_resources: vec![],
        }
    }

    #[test]
    fn equal_burst_damage_resolves_simultaneously() {
        let pawn = unit("burst", 100.0, 100.0, 0.0);
        let result = Simulator::new(Scenario::duel(pawn, 100.0))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 1);
        assert_eq!(result.winner, None);
        assert!(!result.timed_out);
    }

    #[test]
    fn burst_and_damage_over_time_are_materially_different() {
        let burst = unit("burst", 50.0, 100.0, 0.0);
        let damage_over_time = unit("dot", 50.0, 100.0, 1.0);
        let result = Simulator::new(asymmetric_scenario(burst, damage_over_time))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 2);
        assert_eq!(result.winner, Some(0));
        assert!(result.sides[0].surviving_value > 0.0);
        assert!(result.sides[1].damage_dealt > 0.0);
    }

    #[test]
    fn parallel_batches_are_seed_stable() {
        let mut inaccurate = unit("inaccurate", 100.0, 40.0, 0.0);
        inaccurate.weapons[0].accuracy = 0.25;
        let simulator = Simulator::new(Scenario::duel(inaccurate, 100.0));
        let first = simulator.run_parallel([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 200, 99);
        let second = simulator.run_parallel([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 200, 99);
        assert_eq!(first.side_wins, second.side_wins);
        assert_eq!(first.draws, second.draws);
        assert_eq!(
            first.mean_scores[0].damage_dealt,
            second.mean_scores[0].damage_dealt
        );
    }

    #[test]
    fn terrain_height_blocks_fire() {
        let pawn = unit("pawn", 100.0, 100.0, 0.0);
        let mut scenario = Scenario::duel(pawn, 100.0);
        let row = (500.0 / scenario.terrain.cell_size) as usize;
        let col = (1_000.0 / scenario.terrain.cell_size) as usize;
        let middle = row * scenario.terrain.cols + col;
        scenario.terrain.heights[middle] = 100.0;
        scenario.max_time = 1.0;
        let result = Simulator::new(scenario).run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 0);
        assert!(result.timed_out);
        assert_eq!(result.sides[0].damage_dealt, 0.0);
    }

    #[test]
    fn paper_dps_is_derived_from_burst_primitives() {
        let weapon = WeaponDef {
            damage: 9.0,
            burst: 3,
            projectiles: 1,
            reload: 0.3,
            ..Default::default()
        };
        assert_eq!(weapon.paper_dps(), 90.0);
    }

    #[test]
    fn catalog_loader_reads_the_bridge_shape() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/pawn-hello.json");
        let pawn = catalog::load_unit(&path, "armpw").expect("reference catalog loads");
        assert_eq!(pawn.health, 370.0);
        assert_eq!(pawn.weapons[0].burst, 3);
        assert_eq!(pawn.weapons[0].damage, 9.0);
        assert_eq!(pawn.weapons[0].paper_dps(), 90.0);
    }

    #[test]
    fn paired_batches_run_both_policy_sides_per_seed() {
        let pawn = unit("pawn", 100.0, 20.0, 0.0);
        let simulator = Simulator::new(Scenario::duel(pawn, 100.0));
        let summary =
            simulator.run_paired_parallel([&BuiltinPolicy::Kite, &BuiltinPolicy::Charge], 20, 8);
        assert_eq!(summary.runs, 40);
        assert_eq!(summary.side_wins.iter().sum::<usize>() + summary.draws, 40);
    }

    #[test]
    fn traced_run_records_predicted_weapon_intent() {
        let mut pawn = unit("pawn", 100.0, 20.0, 0.0);
        pawn.weapons[0].hitscan = false;
        pawn.weapons[0].projectile_speed = 100.0;
        pawn.weapons[0].predict_boost = 1.0;
        let simulator = Simulator::new(Scenario::duel(pawn, 100.0));
        let (_, trace) = simulator.run_traced(
            [&BuiltinPolicy::Charge, &BuiltinPolicy::Charge],
            5,
            trace::TraceConfig {
                sample_every_ticks: 1,
                max_frames: 10,
            },
        );
        assert!(trace.frames.iter().any(|frame| {
            frame.aim_attempts.iter().any(|aim| {
                frame
                    .units
                    .iter()
                    .find(|unit| Some(unit.id) == aim.target)
                    .is_some_and(|target| (aim.aim_point[0] - target.position[0]).abs() > 0.01)
            })
        }));
    }

    #[test]
    fn tracing_does_not_change_battle_semantics() {
        let pawn = unit("pawn", 100.0, 20.0, 0.0);
        let simulator = Simulator::new(Scenario::duel(pawn, 100.0));
        let plain = simulator.run([&BuiltinPolicy::Kite, &BuiltinPolicy::Charge], 41);
        let (traced, _) = simulator.run_traced(
            [&BuiltinPolicy::Kite, &BuiltinPolicy::Charge],
            41,
            trace::TraceConfig::default(),
        );
        assert_eq!(plain.winner, traced.winner);
        assert_eq!(plain.timed_out, traced.timed_out);
        assert_eq!(plain.duration, traced.duration);
        assert_eq!(plain.sides[0].damage_dealt, traced.sides[0].damage_dealt);
        assert_eq!(plain.sides[1].damage_dealt, traced.sides[1].damage_dealt);
        assert_eq!(
            plain.sides[0].projectiles_fired,
            traced.sides[0].projectiles_fired
        );
        assert_eq!(
            plain.sides[1].projectiles_fired,
            traced.sides[1].projectiles_fired
        );
    }

    fn wall(height: f32) -> UnitDef {
        UnitDef {
            flight: None,
            name: "wall".into(),
            icon_type: None,
            health: 1_000.0,
            metal_cost: 10.0,
            energy_cost: 0.0,
            strategic: StrategicDef::default(),
            movement: MovementDef {
                radius: 10.0,
                height,
                collision_mid_y: Some(height * 0.5),
                ..Default::default()
            },
            weapons: vec![],
        }
    }

    fn wall_line_scenario(wall_height: f32) -> Scenario {
        let mut shooter = unit("shooter", 100.0, 25.0, 0.0);
        shooter.weapons[0].firing_origin = Some([0.0, 20.0, 0.0]);
        let mut target = unit("target", 100.0, 0.0, 0.0);
        target.weapons.clear();
        target.movement.height = 30.0;
        target.movement.collision_mid_y = Some(15.0);
        Scenario {
            terrain: Terrain::flat(1_000.0, 500.0, 16.0),
            units: vec![
                Spawn {
                    side: 0,
                    definition: shooter,
                    position: Vec2::new(400.0, 250.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 0,
                    definition: wall(wall_height),
                    position: Vec2::new(450.0, 250.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: target,
                    position: Vec2::new(500.0, 250.0),
                    heading: std::f32::consts::PI,
                },
            ],
            max_time: 0.2,
            trace_resources: vec![],
        }
    }

    #[test]
    fn tall_friendly_wall_blocks_the_firing_origin() {
        let result = Simulator::new(wall_line_scenario(30.0))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 3);
        assert_eq!(result.sides[0].damage_dealt, 0.0);
    }

    #[test]
    fn firing_origin_clears_a_short_wall() {
        let result = Simulator::new(wall_line_scenario(4.0))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 3);
        assert!(result.sides[0].damage_dealt > 0.0);
    }

    fn elevated_target_scenario(pitch_speed: Option<f32>, max_time: f32) -> Scenario {
        let mut shooter = unit("pitch-limited", 100.0, 100.0, 0.0);
        shooter.weapons[0].hitscan = true;
        shooter.weapons[0].range = 200.0;
        shooter.weapons[0].fire_tolerance = 0.01;
        shooter.weapons[0].firing_origin = Some([0.0, 10.0, 0.0]);
        shooter.weapons[0].turret_pitch_speed = pitch_speed;
        let mut target = unit("tall-target", 100.0, 0.0, 0.0);
        target.weapons.clear();
        target.movement.height = 200.0;
        target.movement.collision_mid_y = Some(110.0);
        Scenario {
            terrain: Terrain::flat(1_000.0, 500.0, 16.0),
            units: vec![
                Spawn {
                    side: 0,
                    definition: shooter,
                    position: Vec2::new(400.0, 250.0),
                    heading: 0.0,
                },
                Spawn {
                    side: 1,
                    definition: target,
                    position: Vec2::new(500.0, 250.0),
                    heading: std::f32::consts::PI,
                },
            ],
            max_time,
            trace_resources: vec![],
        }
    }

    #[test]
    fn finite_turret_pitch_delays_fire_at_an_elevated_target() {
        let slow = Simulator::new(elevated_target_scenario(Some(30.0), 1.0))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 4);
        assert_eq!(slow.sides[0].damage_dealt, 0.0);

        let instant = Simulator::new(elevated_target_scenario(None, 1.0))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 4);
        assert!(instant.sides[0].damage_dealt > 0.0);

        let caught_up = Simulator::new(elevated_target_scenario(Some(30.0), 2.0))
            .run([&BuiltinPolicy::Hold, &BuiltinPolicy::Hold], 4);
        assert!(caught_up.sides[0].damage_dealt > 0.0);
    }

    #[test]
    fn replay_preserves_weapon_yaw_and_shots_wait_for_turret_alignment() {
        let mut shooter = unit("yaw-limited", 1000.0, 1.0, 0.0);
        shooter.movement.max_speed = 0.0;
        shooter.weapons[0].turret_yaw_speed = Some(90.0);
        shooter.weapons[0].fire_tolerance = 0.01;
        let mut target = unit("target", 1000.0, 0.0, 0.0);
        target.weapons.clear();
        target.movement.max_speed = 0.0;
        let mut scenario = asymmetric_scenario(shooter, target);
        scenario.units[0].heading = std::f32::consts::FRAC_PI_2;
        scenario.max_time = 2.0;
        let (_, trace) = Simulator::new(scenario).run_traced(
            [&BuiltinPolicy::Hold, &BuiltinPolicy::Hold],
            1,
            crate::trace::TraceConfig {
                sample_every_ticks: 1,
                max_frames: 0,
            },
        );
        let json = serde_json::to_value(&trace).unwrap();
        let yaw = json["frames"][0]["units"][0]["weapon_headings"][0]
            .as_f64()
            .expect("replays must retain actual weapon yaw, separately from hull yaw");
        // One 30 Hz tick at 90 degrees/s turns the turret from 90 to 87 degrees.
        assert!((yaw - 87_f64.to_radians()).abs() < 1e-5);
        let mut fired = 0;
        for (frame, json_frame) in trace.frames.iter().zip(json["frames"].as_array().unwrap()) {
            let turret = json_frame["units"][0]["weapon_headings"][0]
                .as_f64()
                .unwrap();
            assert!((frame.units[0].heading - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
            for aim in &frame.aim_attempts {
                if aim.state == crate::trace::AimState::Fired {
                    fired += 1;
                    assert!(frame.time > 0.9, "shot before turret could turn");
                    let direction = ((aim.aim_point[1] - aim.origin[1]) as f64)
                        .atan2((aim.aim_point[0] - aim.origin[0]) as f64);
                    assert!((turret - direction).abs() <= 0.01001);
                }
            }
        }
        assert!(fired > 0, "fixture must exercise a real shot");
    }

    #[test]
    fn replay_fixed_guns_follow_turning_hulls_and_shields_have_no_barrel() {
        let mut shooter = unit("fixed-gun", 1000.0, 1.0, 0.0);
        shooter.weapons[0].turret = false;
        shooter.weapons[0].range = 1.0;
        shooter.weapons.push(WeaponDef {
            is_shield: true,
            ..Default::default()
        });
        let mut target = unit("target", 1000.0, 0.0, 0.0);
        target.weapons.clear();
        let mut scenario = asymmetric_scenario(shooter, target);
        scenario.units[0].heading = std::f32::consts::FRAC_PI_2;
        scenario.max_time = 0.2;
        let (_, trace) = Simulator::new(scenario).run_traced(
            [&BuiltinPolicy::Charge, &BuiltinPolicy::Hold],
            1,
            crate::trace::TraceConfig {
                sample_every_ticks: 1,
                max_frames: 0,
            },
        );
        assert!((trace.frames[0].units[0].heading - std::f32::consts::FRAC_PI_2).abs() > 0.01);
        let json = serde_json::to_value(&trace).unwrap();
        for (frame, wire) in trace.frames.iter().zip(json["frames"].as_array().unwrap()) {
            let mounts = wire["units"][0]["weapon_headings"].as_array().unwrap();
            assert_eq!(mounts.len(), 2, "weapon indices must stay stable");
            assert!((mounts[0].as_f64().unwrap() - frame.units[0].heading as f64).abs() < 1e-5);
            assert!(
                mounts[1].is_null(),
                "shield slots must not become fictitious barrels"
            );
        }
    }
}
