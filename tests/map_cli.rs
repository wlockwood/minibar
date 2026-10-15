use std::{fs, process::Command};
use visigoth_simulator::map::CompiledMap;

#[test]
fn compile_map_accepts_canonical_source_and_discovers_acquisition() {
    let directory = std::env::temp_dir().join(format!(
        "visigoth-map-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let source_path = directory.join("tiny.map_source.json");
    let acquisition_path = directory.join("tiny.acquisition.json");
    let cache_path = directory.join("tiny.vsgmap");
    fs::write(
        &source_path,
        r#"{
            "source_version":1,"map_name":"CLI Native","content_hash":"sha256:cli",
            "width":16.0,"depth":16.0,"water_level":2.0,
            "heights":{"cols":3,"rows":3,"origin_x":0.0,"origin_z":0.0,
                "step_x":8.0,"step_z":8.0,"values":[0,0,0,0,0,0,0,0,0]},
            "passability":[{"movement_class":"Bot","cols":2,"rows":2,
                "cell_size_x":8.0,"cell_size_z":8.0,"passable":[true,true,true,true]}],
            "metal_nodes":[],
            "continuous_metal":{"density":{"cols":2,"rows":2,"origin_x":4.0,
                "origin_z":4.0,"step_x":8.0,"step_z":8.0,"values":[0,1,0,1]},
                "extractor_radius":8.0,"extraction_scale":1.0},
            "geothermal_nodes":[]
        }"#,
    )
    .unwrap();
    fs::write(
        &acquisition_path,
        r#"{
            "format":"visigoth.native_map","format_version":1,"assembler_revision":1,
            "content_hash":"sha256:cli",
            "identity":{"map_name":"CLI Native","map_checksum":"map","game_name":"BAR",
                "game_version":"1","mod_checksum":"mod","engine_version":"Recoil",
                "exporter_revision":1},
            "map":{"extractor_radius":8.0},
            "options":{"map":{"weather":"clear"},"mod":{"startmetal":"1000"}},
            "movedefs":[{"name":"Bot","path_type":1,"max_slope":0.5}],
            "static_blockers":[{"feature_id":7,"definition":"rock","x":4.0,"y":0.0,"z":4.0,
                "min_x":0.0,"min_z":0.0,"max_x":8.0,"max_z":8.0,
                "blocks_movement":true,"blocks_fire":true,"fire_height":6.0,
                "crushable":false,"reclaimable":true,"reclaim_time":10.0,
                "metal":2.0,"energy":0.0,"geothermal":false}]
        }"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--compile-map",
            source_path.to_str().unwrap(),
            "--map-out",
            cache_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["input_format"], "native_map_source");
    assert_eq!(summary["provenance_complete"], true);

    let map = CompiledMap::from_cache_bytes(&fs::read(&cache_path).unwrap()).unwrap();
    assert_eq!(map.heights.origin_x, 0.0);
    assert_eq!(map.provenance.identity.unwrap().map_checksum, "map");
    assert_eq!(map.provenance.mod_options["startmetal"], "1000");
    assert_eq!(map.static_blockers.len(), 1);
    assert_eq!(map.static_blockers[0].definition, "rock");

    fs::remove_file(source_path).unwrap();
    fs::remove_file(acquisition_path).unwrap();
    fs::remove_file(cache_path).unwrap();
    fs::remove_dir(directory).unwrap();
}

#[test]
fn declarative_scenario_cli_runs_shared_combat_to_a_terminal_hash() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cache_path = std::env::temp_dir().join(format!(
        "visigoth-scenario-cli-{}-{}.vsgmap",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let compile = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--compile-map",
            manifest
                .join("examples/flat-pawn-map.json")
                .to_str()
                .unwrap(),
            "--map-out",
            cache_path.to_str().unwrap(),
            "--legacy-movement-class",
            "BOT",
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--catalog",
            manifest.join("examples/pawn-hello.json").to_str().unwrap(),
            "--run-scenario",
            manifest
                .join("examples/pawn-envelopment.json")
                .to_str()
                .unwrap(),
            "--map",
            cache_path.to_str().unwrap(),
            "--movement-class",
            "BOT",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(report["scenario"]["combat_enabled"], true);
    assert_eq!(report["scenario"]["ticks"], 300);
    assert_eq!(report["accepted_actions"], 2);
    assert_eq!(report["rejected_actions"], 0);
    assert!(report["combat_damage_events"].as_u64().unwrap() > 0);
    assert!(report["terminal_state_hash"].as_u64().unwrap() > 0);
    assert!(
        !report["calibration_warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    fs::remove_file(cache_path).unwrap();
}

#[test]
fn declarative_scenarios_execute_all_four_new_scenarios() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cache_path = std::env::temp_dir().join(format!(
        "visigoth-scenarios-all-{}-{}.vsgmap",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let compile = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--compile-map",
            manifest
                .join("examples/flat-pawn-map.json")
                .to_str()
                .unwrap(),
            "--map-out",
            cache_path.to_str().unwrap(),
            "--legacy-movement-class",
            "BOT",
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let scenarios = [
        "examples/wall-defense.json",
        "examples/economy-race.json",
        "examples/pawn-raid.json",
        "examples/complete-opening.json",
    ];

    for scenario_file in scenarios {
        let run = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
            .args([
                "--catalog",
                manifest.join("examples/pawn-hello.json").to_str().unwrap(),
                "--run-scenario",
                manifest.join(scenario_file).to_str().unwrap(),
                "--map",
                cache_path.to_str().unwrap(),
                "--movement-class",
                "BOT",
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "Scenario {} failed: {}",
            scenario_file,
            String::from_utf8_lossy(&run.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
        assert!(
            report["terminal_state_hash"].as_u64().is_some(),
            "Missing terminal_state_hash in {}",
            scenario_file
        );
        assert_eq!(
            report["rejected_actions"], 0,
            "Rejected actions in {}",
            scenario_file
        );
        let warnings = report["calibration_warnings"]
            .as_array()
            .expect("calibration_warnings array");
        assert!(
            !warnings.is_empty(),
            "Calibration warnings empty in {}",
            scenario_file
        );
        assert_eq!(
            report["sides"].as_array().expect("sides array").len(),
            2,
            "Sides count must be 2 in {}",
            scenario_file
        );
    }

    fs::remove_file(cache_path).unwrap();
}

#[test]
fn declarative_scenario_cli_replays_live_log_and_verifies_terminal_state() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp_dir = std::env::temp_dir().join(format!(
        "visigoth-replay-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let cache_path = temp_dir.join("flat.vsgmap");
    let replay_path = temp_dir.join("session.json");

    let compile = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--compile-map",
            manifest
                .join("examples/flat-pawn-map.json")
                .to_str()
                .unwrap(),
            "--map-out",
            cache_path.to_str().unwrap(),
            "--legacy-movement-class",
            "BOT",
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // Load map, catalog, and scenario to construct an authentic runtime session
    let map_bytes = fs::read(&cache_path).unwrap();
    let map = std::sync::Arc::new(CompiledMap::from_cache_bytes(&map_bytes).unwrap());
    let unit_catalog =
        visigoth_simulator::catalog::load_catalog(&manifest.join("examples/pawn-hello.json"))
            .unwrap();
    let catalog = std::sync::Arc::new(
        visigoth_simulator::world::WorldCatalog::new(
            unit_catalog.units.values().cloned().collect(),
        )
        .unwrap(),
    );
    let scenario_json =
        fs::read_to_string(manifest.join("examples/pawn-envelopment.json")).unwrap();
    let scenario: visigoth_simulator::strategy::DeclarativeScenario =
        serde_json::from_str(&scenario_json).unwrap();
    let maximum_starting_metal = scenario
        .sides
        .iter()
        .map(|side| side.starting_metal)
        .fold(0.0_f64, f64::max);
    let maximum_starting_energy = scenario
        .sides
        .iter()
        .map(|side| side.starting_energy)
        .fold(0.0_f64, f64::max);
    let mut options = visigoth_simulator::scenario_compile::ScenarioCompileOptions::default();
    options.world.base_storage.metal = options.world.base_storage.metal.max(maximum_starting_metal);
    options.world.base_storage.energy = options
        .world
        .base_storage
        .energy
        .max(maximum_starting_energy);
    let mut runtime = visigoth_simulator::scenario_compile::compile_declarative_scenario(
        &scenario, map, catalog, options,
    )
    .unwrap();
    runtime
        .world
        .enable_combat(runtime.seed, "BOT", 16.0)
        .unwrap();

    let mut driver = visigoth_simulator::live::LiveDriver::new(runtime);
    let first_entity = driver
        .world()
        .world
        .entities
        .iter()
        .next()
        .map(|(id, _)| id)
        .expect("scenario has entities");

    driver.step_ticks(2).unwrap();
    driver.submit_action(
        2,
        visigoth_simulator::strategy::QueuedAction {
            side: 0,
            sequence: 10,
            apply_at_tick: 2,
            action: visigoth_simulator::strategy::StrategyAction::Move {
                entities: vec![first_entity],
                destination: visigoth_simulator::model::Vec2::new(100.0, 100.0),
                mode: visigoth_simulator::strategy::OrderMode::Replace,
            },
        },
    );
    driver.step_ticks(3).unwrap();
    let log = driver.replay_log();
    assert_eq!(log.actions.len(), 1);
    assert_eq!(log.final_tick, 5);

    fs::write(&replay_path, serde_json::to_vec_pretty(&log).unwrap()).unwrap();

    // Now invoke the CLI with --replay-live
    let run = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--catalog",
            manifest.join("examples/pawn-hello.json").to_str().unwrap(),
            "--run-scenario",
            manifest
                .join("examples/pawn-envelopment.json")
                .to_str()
                .unwrap(),
            "--map",
            cache_path.to_str().unwrap(),
            "--movement-class",
            "BOT",
            "--replay-live",
            replay_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "CLI replay failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let verification: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(verification["final_tick"], 5);
    assert_eq!(verification["actions_replayed"], 1);
    assert_eq!(verification["final_state_hash"], log.final_state_hash);

    fs::remove_file(cache_path).unwrap();
    fs::remove_file(replay_path).unwrap();
    fs::remove_dir(temp_dir).unwrap();
}
