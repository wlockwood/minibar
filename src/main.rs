use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    env,
    f32::consts::PI,
    fs,
    path::Path,
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use visigoth_simulator::{
    BuildFacing, BuiltinPolicy, CalibrationFixture, DeclarativeScenario, MeasuredObservations,
    Scenario, ScenarioCompileOptions, Simulator, Spawn, Terrain, UnitDef, UnitDefId, Vec2, World,
    WorldCatalog, WorldConfig, catalog, compare_calibration, compile_declarative_scenario,
    economy::Resources,
    live::{LiveDriver, ReplayLog, replay_and_verify},
    live_server::LiveServerConfig,
    map::{CompiledMap, HelloMapOptions, MapInputFormat, MapLoadOptions, MapSource},
    start_scenario_live_server,
    trace::{AggregateTrace, ResourceKind, ResourceMarker, TraceConfig},
    viewer::{IconCatalog, ViewerData, ViewerDocument},
};

const ALLOCATION_SHARDS: usize = 256;
const UNASSIGNED_ALLOCATION_SHARD: usize = usize::MAX;

#[repr(align(64))]
struct AllocationShard {
    calls: AtomicU64,
    bytes: AtomicU64,
}

impl AllocationShard {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }
}

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static NEXT_ALLOCATION_SHARD: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_COUNTERS: [AllocationShard; ALLOCATION_SHARDS] =
    [const { AllocationShard::new() }; ALLOCATION_SHARDS];
static OVERFLOW_ALLOCATION_COUNTER: AllocationShard = AllocationShard::new();

thread_local! {
    static ALLOCATION_SHARD_INDEX: Cell<usize> = const { Cell::new(UNASSIGNED_ALLOCATION_SHARD) };
}

struct CountingAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

// Allocation accounting is deliberately implemented in the binary. Library users pay no
// instrumentation cost, while the benchmark suite can observe allocations made by Rayon workers.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            record_allocation(new_size);
        }
        new_pointer
    }
}

#[inline]
fn record_allocation(bytes: usize) {
    if !COUNT_ALLOCATIONS.load(Ordering::Acquire) {
        return;
    }
    let recorded = ALLOCATION_SHARD_INDEX.try_with(|index| {
        let mut shard_index = index.get();
        if shard_index == UNASSIGNED_ALLOCATION_SHARD {
            shard_index = NEXT_ALLOCATION_SHARD.fetch_add(1, Ordering::Relaxed);
            index.set(shard_index);
        }
        let shard = ALLOCATION_COUNTERS
            .get(shard_index)
            .unwrap_or(&OVERFLOW_ALLOCATION_COUNTER);
        shard.calls.fetch_add(1, Ordering::Relaxed);
        shard.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    });
    if recorded.is_err() {
        OVERFLOW_ALLOCATION_COUNTER
            .calls
            .fetch_add(1, Ordering::Relaxed);
        OVERFLOW_ALLOCATION_COUNTER
            .bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AllocationTotals {
    calls: u64,
    bytes: u64,
}

struct AllocationMeasurement {
    active: bool,
}

impl AllocationMeasurement {
    fn begin() -> Self {
        debug_assert!(!COUNT_ALLOCATIONS.load(Ordering::Relaxed));
        for shard in &ALLOCATION_COUNTERS {
            shard.calls.store(0, Ordering::Relaxed);
            shard.bytes.store(0, Ordering::Relaxed);
        }
        OVERFLOW_ALLOCATION_COUNTER
            .calls
            .store(0, Ordering::Relaxed);
        OVERFLOW_ALLOCATION_COUNTER
            .bytes
            .store(0, Ordering::Relaxed);
        COUNT_ALLOCATIONS.store(true, Ordering::Release);
        Self { active: true }
    }

    fn finish(mut self) -> AllocationTotals {
        // Disable recording before reading. Counter reads therefore cannot change the result even
        // if a future implementation of report construction happens to allocate.
        COUNT_ALLOCATIONS.store(false, Ordering::Release);
        self.active = false;
        let mut totals = AllocationTotals::default();
        for shard in &ALLOCATION_COUNTERS {
            totals.calls += shard.calls.load(Ordering::Acquire);
            totals.bytes += shard.bytes.load(Ordering::Acquire);
        }
        totals.calls += OVERFLOW_ALLOCATION_COUNTER.calls.load(Ordering::Acquire);
        totals.bytes += OVERFLOW_ALLOCATION_COUNTER.bytes.load(Ordering::Acquire);
        totals
    }
}

impl Drop for AllocationMeasurement {
    fn drop(&mut self) {
        if self.active {
            COUNT_ALLOCATIONS.store(false, Ordering::Release);
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "usage:\n  visigoth-simulator --catalog HELLO.json [--assets BAR_SOURCE_ROOT] [--catalog-out CATALOG.json] [--unit armpw] [--map MAP_FILE] [--movement-class NAME] [--map-cell 16] [--runs 10000] [--paired] [--left kite] [--right charge] [--distance 240] [--seed 1] [--trace RUN.json] [--viewer RUN.html] [--aggregate-viewer RUNS.html] [--aggregate-runs 100] [--heat-cols 64] [--heat-rows 64] [--trace-every 3] [--human-side 0]\n  visigoth-simulator --catalog HELLO.json [--assets BAR_SOURCE_ROOT] --run-scenario SCENARIO.json --map MAP_FILE [--movement-class NAME] [--map-cell 16] [--strategic-only] [--los-range 500] [--radar-range 1200] [--radar-resolution 32] [--serve 127.0.0.1:0] [--control-side 0] [--live-max-ticks N] [--live-log SESSION.json]\n  visigoth-simulator --catalog HELLO.json [--assets BAR_SOURCE_ROOT] --run-scenario SCENARIO.json --map MAP_FILE [same scenario options] --replay-live SESSION.json\n  visigoth-simulator --catalog HELLO.json [--assets BAR_SOURCE_ROOT] [--unit armpw] --benchmark-suite [--benchmark-runs 10000] [--seed 1]\n  visigoth-simulator --compile-map MAP_SOURCE_OR_HELLO.json --map-out MAP.vsgmap [--acquisition CAPTURE.acquisition.json] [--content-hash HASH] [--extractor-radius 500] [--extraction-scale 1] [--legacy-movement-class NAME]\n  MAP_FILE accepts a precompiled .vsgmap cache, canonical .map_source.json, or bridge hello JSON.\n  visigoth-simulator --compare-calibration FIXTURE.json --observations MEASURED.json [--report REPORT.json]"
    );
    process::exit(2)
}

fn value(args: &[String], key: &str, default: &str) -> String {
    args.iter()
        .position(|arg| arg == key)
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| default.to_owned())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage();
    }

    let calibration_fixture = value(&args, "--compare-calibration", "");
    if !calibration_fixture.is_empty() {
        compare_calibration_files(&args, &calibration_fixture);
        return;
    }

    let compile_map_path = value(&args, "--compile-map", "");
    if !compile_map_path.is_empty() {
        compile_map(&args, &compile_map_path);
        return;
    }

    let catalog_path = value(&args, "--catalog", "");
    if catalog_path.is_empty() {
        usage();
    }
    let unit_name = value(&args, "--unit", "armpw");
    let runs: usize = value(&args, "--runs", "10000")
        .parse()
        .unwrap_or_else(|_| usage());
    let separation: f32 = value(&args, "--distance", "240")
        .parse()
        .unwrap_or_else(|_| usage());
    let seed: u64 = value(&args, "--seed", "1")
        .parse()
        .unwrap_or_else(|_| usage());
    let left = BuiltinPolicy::parse(&value(&args, "--left", "kite")).unwrap_or_else(|| usage());
    let right = BuiltinPolicy::parse(&value(&args, "--right", "charge")).unwrap_or_else(|| usage());
    let paired = args.iter().any(|arg| arg == "--paired");
    let trace_path = value(&args, "--trace", "");
    let viewer_path = value(&args, "--viewer", "");
    let aggregate_viewer_path = value(&args, "--aggregate-viewer", "");
    let trace_every: u64 = value(&args, "--trace-every", "3")
        .parse()
        .unwrap_or_else(|_| usage());
    let human_side = args
        .iter()
        .position(|arg| arg == "--human-side")
        .map(|index| {
            args.get(index + 1)
                .and_then(|side| side.parse::<u8>().ok())
                .filter(|side| *side < 2)
                .unwrap_or_else(|| usage())
        });

    let asset_root = value(&args, "--assets", "");
    let catalog_out = value(&args, "--catalog-out", "");

    let loaded_catalog = if asset_root.is_empty() {
        catalog::load_catalog_with_capabilities(
            Path::new(&catalog_path),
            catalog::IMPLEMENTED_CAPABILITIES.iter().copied(),
        )
        .unwrap_or_else(|error| {
            eprintln!("error: {error}");
            process::exit(1);
        })
    } else {
        catalog::load_catalog_with_assets(Path::new(&catalog_path), Path::new(&asset_root))
            .unwrap_or_else(|error| {
                eprintln!("error: {error}");
                process::exit(1);
            })
    };

    if !catalog_out.is_empty() {
        catalog::export_catalog_json(
            &loaded_catalog,
            Path::new(&catalog_out),
            Path::new(&catalog_path),
        )
        .unwrap_or_else(|error| {
            eprintln!("error exporting catalog: {error}");
            process::exit(1);
        });
        println!("exported catalog to {catalog_out}");
        if args
            .iter()
            .all(|arg| arg != "--run-scenario" && arg != "--unit" && arg != "--benchmark-suite")
        {
            return;
        }
    }

    let declarative_scenario = value(&args, "--run-scenario", "");
    if !declarative_scenario.is_empty() {
        run_declarative_scenario(&args, &declarative_scenario, &loaded_catalog);
        return;
    }
    let unit = loaded_catalog.get(&unit_name).cloned().unwrap_or_else(|| {
        eprintln!("error: unit {unit_name:?} is absent from {catalog_path}");
        process::exit(1);
    });
    if args.iter().any(|arg| arg == "--benchmark-suite") {
        let benchmark_runs: usize = value(&args, "--benchmark-runs", "10000")
            .parse()
            .unwrap_or_else(|_| usage());
        run_benchmark_suite(&catalog_path, &unit, benchmark_runs, seed);
        return;
    }
    let movement_class = value(
        &args,
        "--movement-class",
        unit.strategic.movement_class.as_deref().unwrap_or(""),
    );
    let mut scenario = Scenario::duel(unit, separation);
    let map_path = value(&args, "--map", "");
    if !map_path.is_empty() {
        if movement_class.is_empty() {
            eprintln!("error: --movement-class is required for this unit/map");
            process::exit(1);
        }
        let cell_size: f32 = value(&args, "--map-cell", "16")
            .parse()
            .unwrap_or_else(|_| usage());
        let map = load_map_or_exit(&map_path);
        scenario.terrain = map
            .to_tactical_terrain(&movement_class, cell_size)
            .unwrap_or_else(|error| {
                eprintln!("error materializing {map_path}: {error}");
                process::exit(1);
            });
        scenario.trace_resources = trace_resources(&map);
        let centre = Vec2::new(map.width * 0.5, map.depth * 0.5);
        scenario.units[0].position = Vec2::new(centre.x - separation * 0.5, centre.z);
        scenario.units[1].position = Vec2::new(centre.x + separation * 0.5, centre.z);
    }
    let simulator = Simulator::new(scenario);

    if !aggregate_viewer_path.is_empty() {
        let aggregate_runs: usize = value(&args, "--aggregate-runs", "100")
            .parse()
            .unwrap_or_else(|_| usage());
        let heat_cols: usize = value(&args, "--heat-cols", "64")
            .parse()
            .unwrap_or_else(|_| usage());
        let heat_rows: usize = value(&args, "--heat-rows", "64")
            .parse()
            .unwrap_or_else(|_| usage());
        let traces = simulator
            .run_traced_parallel(
                [&left, &right],
                aggregate_runs,
                seed,
                TraceConfig {
                    sample_every_ticks: trace_every.max(1),
                    max_frames: 0,
                },
            )
            .into_iter()
            .map(|(_, trace)| trace)
            .collect::<Vec<_>>();
        let aggregate =
            AggregateTrace::from_runs(&traces, heat_cols, heat_rows).unwrap_or_else(|error| {
                eprintln!("error aggregating traces: {error}");
                process::exit(1);
            });
        ViewerDocument {
            title: format!("{unit_name}: aggregate {left:?} vs {right:?}"),
            data: ViewerData::Aggregate(aggregate),
            icons: IconCatalog::default(),
            human_side: None,
        }
        .write_html(Path::new(&aggregate_viewer_path))
        .unwrap_or_else(|error| {
            eprintln!("error writing {aggregate_viewer_path}: {error}");
            process::exit(1);
        });
    }
    if !trace_path.is_empty() || !viewer_path.is_empty() {
        let (_, trace) = simulator.run_traced(
            [&left, &right],
            seed,
            TraceConfig {
                sample_every_ticks: trace_every.max(1),
                max_frames: 0,
            },
        );
        if !trace_path.is_empty() {
            let json = serde_json::to_vec(&trace).expect("run trace serializes");
            fs::write(&trace_path, json).unwrap_or_else(|error| {
                eprintln!("error writing {trace_path}: {error}");
                process::exit(1);
            });
        }
        if !viewer_path.is_empty() {
            let keys = trace
                .frames
                .first()
                .into_iter()
                .flat_map(|frame| frame.units.iter().map(|unit| unit.icon_key.as_str()));
            let icons = if asset_root.is_empty() {
                IconCatalog::default()
            } else {
                IconCatalog::probe_bar_root(Path::new(&asset_root), keys)
            };
            ViewerDocument {
                title: format!("{unit_name}: {left:?} vs {right:?}"),
                data: ViewerData::Run(trace),
                icons,
                human_side,
            }
            .write_html(Path::new(&viewer_path))
            .unwrap_or_else(|error| {
                eprintln!("error writing {viewer_path}: {error}");
                process::exit(1);
            });
        }
    }

    let summary = if paired {
        simulator.run_paired_parallel([&left, &right], runs, seed)
    } else {
        simulator.run_parallel([&left, &right], runs, seed)
    };
    let realtime_multiple = summary.mean_duration as f64 * summary.simulations_per_second;

    let output = serde_json::json!({
        "experiment": {
            "unit": unit_name,
            "left_policy": format!("{left:?}").to_ascii_lowercase(),
            "right_policy": format!("{right:?}").to_ascii_lowercase(),
            "seed": seed,
            "seed_count": runs,
            "paired": paired,
            "starting_separation": separation,
            "calibration_warnings": visigoth_simulator::score::declared_approximation_warnings(),
        },
        "summary": summary,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("summary serializes")
    );
    eprintln!(
        "throughput: {:.0} simulations/s, approximately {:.0}x aggregate realtime",
        summary.simulations_per_second, realtime_multiple
    );
}

fn run_declarative_scenario(
    args: &[String],
    scenario_path: &str,
    source_catalog: &catalog::UnitCatalog,
) {
    let map_path = value(args, "--map", "");
    if map_path.is_empty() {
        eprintln!("error: --map MAP_FILE is required with --run-scenario");
        process::exit(1);
    }
    let scenario_json = fs::read_to_string(scenario_path).unwrap_or_else(|error| {
        eprintln!("error reading scenario {scenario_path}: {error}");
        process::exit(1);
    });
    let scenario: DeclarativeScenario =
        serde_json::from_str(&scenario_json).unwrap_or_else(|error| {
            eprintln!("error parsing scenario {scenario_path}: {error}");
            process::exit(1);
        });
    let map = Arc::new(load_map_or_exit(&map_path));
    let catalog = Arc::new(
        WorldCatalog::new(source_catalog.units.values().cloned().collect()).unwrap_or_else(
            |error| {
                eprintln!("error creating strategic catalog: {error}");
                process::exit(1);
            },
        ),
    );

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
    let mut options = ScenarioCompileOptions::default();
    options.world.base_storage.metal = options.world.base_storage.metal.max(maximum_starting_metal);
    options.world.base_storage.energy = options
        .world
        .base_storage
        .energy
        .max(maximum_starting_energy);
    options.line_of_sight_range = value(args, "--los-range", "500")
        .parse()
        .unwrap_or_else(|_| usage());
    options.radar_range = value(args, "--radar-range", "1200")
        .parse()
        .unwrap_or_else(|_| usage());
    options.radar_position_resolution = value(args, "--radar-resolution", "32")
        .parse()
        .unwrap_or_else(|_| usage());

    let mut runtime = compile_declarative_scenario(&scenario, Arc::clone(&map), catalog, options)
        .unwrap_or_else(|error| {
            eprintln!("error compiling scenario {scenario_path}: {error}");
            process::exit(1);
        });
    if !args.iter().any(|argument| argument == "--strategic-only") {
        let explicit_class = value(args, "--movement-class", "");
        let inferred_classes = runtime
            .world
            .entities
            .iter()
            .filter_map(|(_, entity)| {
                runtime
                    .world
                    .catalog
                    .get(entity.definition)
                    .and_then(|definition| definition.strategic.movement_class.as_deref())
            })
            .collect::<std::collections::BTreeSet<_>>();
        let movement_class = if !explicit_class.is_empty() {
            explicit_class
        } else if let Some(first_inferred) = inferred_classes.first() {
            first_inferred.to_string()
        } else if !map.passability.is_empty() {
            map.passability[0].movement_class.clone()
        } else {
            eprintln!(
                "error: --movement-class is required when scenario units have no inferred movement class and map has no passability layers"
            );
            process::exit(1);
        };
        let cell_size = value(args, "--map-cell", "16")
            .parse()
            .unwrap_or_else(|_| usage());
        runtime
            .world
            .enable_combat(runtime.seed, &movement_class, cell_size)
            .unwrap_or_else(|error| {
                eprintln!("error enabling scenario combat: {error}");
                process::exit(1);
            });
    }

    let replay_path = value(args, "--replay-live", "");
    let serve_address = value(args, "--serve", "");
    if !serve_address.is_empty() {
        let live_max_ticks: u64 = value(args, "--live-max-ticks", "0")
            .parse()
            .unwrap_or_else(|_| usage());
        // Declarative scenarios have a finite offline test horizon. A human
        // session must not silently turn into a no-op when it reaches that
        // horizon, so zero (the default) means run until the operator stops it.
        runtime.max_ticks = if live_max_ticks == 0 {
            u64::MAX
        } else {
            live_max_ticks
        };
    }
    if !replay_path.is_empty() {
        let bytes = fs::read(&replay_path).unwrap_or_else(|error| {
            eprintln!("error reading {replay_path}: {error}");
            process::exit(1);
        });
        let log: ReplayLog = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            eprintln!("error parsing {replay_path}: {error}");
            process::exit(1);
        });
        // A saved interactive session may outlast the scenario's normal
        // offline horizon. Replaying through its recorded terminal tick is
        // necessary for its terminal hash to be meaningful.
        runtime.max_ticks = runtime.max_ticks.max(log.final_tick);
        let verification = replay_and_verify(&mut runtime, &log).unwrap_or_else(|error| {
            eprintln!("live replay verification failed: {error:?}");
            process::exit(1);
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&verification).expect("replay verification serializes")
        );
        return;
    }

    if !serve_address.is_empty() {
        let bind_addr = serve_address.parse().unwrap_or_else(|_| {
            eprintln!("error: invalid --serve socket address {serve_address:?}");
            process::exit(1);
        });
        let control_side: u8 = value(args, "--control-side", "0")
            .parse()
            .ok()
            .filter(|side| *side < 2)
            .unwrap_or_else(|| usage());
        let server = start_scenario_live_server(
            LiveDriver::new(runtime),
            control_side,
            LiveServerConfig {
                bind_addr,
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| {
            eprintln!("error starting live scenario server: {error}");
            process::exit(1);
        });
        eprintln!(
            "live scenario viewer: http://{} (press Enter to stop)",
            server.local_addr()
        );
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let replay_log = server.shutdown_with_replay().unwrap_or_else(|error| {
            eprintln!("error stopping live scenario server: {error}");
            process::exit(1);
        });
        let live_log_path = value(args, "--live-log", "");
        if !live_log_path.is_empty() {
            let bytes = serde_json::to_vec_pretty(&replay_log).expect("live replay serializes");
            fs::write(&live_log_path, bytes).unwrap_or_else(|error| {
                eprintln!("error writing {live_log_path}: {error}");
                process::exit(1);
            });
            eprintln!("saved live replay: {live_log_path}");
        }
        return;
    }

    let mut accepted_actions = 0_u64;
    let mut rejected_actions = 0_u64;
    let mut combat_damage_events = 0_u64;
    let mut destroyed_entities = 0_u64;
    while runtime.step().unwrap_or_else(|error| {
        eprintln!("error stepping scenario: {error}");
        process::exit(1);
    }) {
        for decision in &runtime.world.action_decisions {
            if decision.result.is_ok() {
                accepted_actions += 1;
            } else {
                rejected_actions += 1;
            }
        }
        for event in &runtime.world.events {
            match event {
                visigoth_simulator::world::WorldEvent::CombatDamage { .. } => {
                    combat_damage_events += 1;
                }
                visigoth_simulator::world::WorldEvent::EntityDestroyed { .. } => {
                    destroyed_entities += 1;
                }
                _ => {}
            }
        }
    }

    let sides = runtime
        .sides
        .iter()
        .map(|side| {
            let side_id = side.specification.side;
            let team = &runtime.world.teams[side_id as usize];
            let owned_entities = runtime
                .world
                .entities
                .iter()
                .filter(|(_, entity)| entity.owner == side_id)
                .map(|(_, entity)| entity)
                .collect::<Vec<_>>();
            let live_entities = owned_entities.len();
            let total_health: f64 = owned_entities.iter().map(|entity| entity.health).sum();
            let damaged_entities = owned_entities
                .iter()
                .filter(|entity| {
                    runtime
                        .world
                        .catalog
                        .get(entity.definition)
                        .is_some_and(|definition| {
                            entity.health + 1e-9 < f64::from(definition.health)
                        })
                })
                .count();
            serde_json::json!({
                "side": side_id,
                "live_entities": live_entities,
                "damaged_entities": damaged_entities,
                "total_health": total_health,
                "metal": team.economy.metal.stored,
                "energy": team.economy.energy.stored,
                "metrics": runtime.world.strategic_metrics(side_id).expect("validated side"),
            })
        })
        .collect::<Vec<_>>();
    let output = serde_json::json!({
        "scenario": {
            "name": runtime.name,
            "kind": runtime.kind,
            "seed": runtime.seed,
            "ticks": runtime.world.tick,
            "combat_enabled": runtime.world.combat_enabled(),
        },
        "terminal_state_hash": runtime.world.state_hash(),
        "pending_combat_impacts": runtime.world.pending_combat_impacts(),
        "accepted_actions": accepted_actions,
        "rejected_actions": rejected_actions,
        "combat_damage_events": combat_damage_events,
        "destroyed_entities": destroyed_entities,
        "sides": sides,
        "calibration_warnings": visigoth_simulator::score::declared_approximation_warnings(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("scenario report serializes")
    );
}

fn compare_calibration_files(args: &[String], fixture_path: &str) {
    let observations_path = value(args, "--observations", "");
    if observations_path.is_empty() {
        usage();
    }
    let fixture_bytes = fs::read(fixture_path).unwrap_or_else(|error| {
        eprintln!("error reading calibration fixture {fixture_path}: {error}");
        process::exit(1);
    });
    let fixture: CalibrationFixture =
        serde_json::from_slice(&fixture_bytes).unwrap_or_else(|error| {
            eprintln!("error parsing calibration fixture {fixture_path}: {error}");
            process::exit(1);
        });
    let observation_bytes = fs::read(&observations_path).unwrap_or_else(|error| {
        eprintln!("error reading calibration observations {observations_path}: {error}");
        process::exit(1);
    });
    let observations: MeasuredObservations = serde_json::from_slice(&observation_bytes)
        .unwrap_or_else(|error| {
            eprintln!("error parsing calibration observations {observations_path}: {error}");
            process::exit(1);
        });
    let report = compare_calibration(&fixture, &observations);
    let json = serde_json::to_string_pretty(&report).expect("calibration report serializes");
    let report_path = value(args, "--report", "");
    if !report_path.is_empty() {
        fs::write(&report_path, json.as_bytes()).unwrap_or_else(|error| {
            eprintln!("error writing calibration report {report_path}: {error}");
            process::exit(1);
        });
    }
    println!("{json}");
    if !report.passed {
        process::exit(1);
    }
}

fn source_control_revision() -> String {
    if let Some(rev) = option_env!("VISIGOTH_GIT_REV")
        && !rev.is_empty()
    {
        return rev.to_string();
    }
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        && output.status.success()
    {
        let rev = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !rev.is_empty() {
            return rev;
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn catalog_hash(path: &str) -> String {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return "unavailable".to_string(),
    };
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a:{:016x}", hash)
}

fn fnv_u64(hash: &mut u64, val: u64) {
    for byte in val.to_le_bytes() {
        *hash ^= byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn scenario_hash(scenario: &Scenario) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    fnv_u64(&mut hash, scenario.units.len() as u64);
    for unit in &scenario.units {
        fnv_u64(&mut hash, unit.side as u64);
        fnv_u64(&mut hash, unit.position.x.to_bits() as u64);
        fnv_u64(&mut hash, unit.position.z.to_bits() as u64);
        fnv_u64(&mut hash, unit.heading.to_bits() as u64);
        fnv_u64(&mut hash, unit.definition.health.to_bits() as u64);
        for byte in unit.definition.name.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    fnv_u64(&mut hash, scenario.terrain.cols as u64);
    fnv_u64(&mut hash, scenario.terrain.rows as u64);
    for h in &scenario.terrain.heights {
        fnv_u64(&mut hash, h.to_bits() as u64);
    }
    fnv_u64(&mut hash, scenario.max_time.to_bits() as u64);
    format!("fnv1a:{:016x}", hash)
}

fn run_benchmark_suite(catalog_path: &str, unit: &UnitDef, runs: usize, seed: u64) {
    let runs = runs.max(1);
    // Initialize the global Rayon pool and each worker's local state outside all case timers and
    // allocation measurements. Per-case results still include allocations intrinsic to scheduling
    // and collecting that case's workload.
    rayon::broadcast(|_| std::hint::black_box(()));
    let mut cases = Vec::new();
    cases.push(benchmark_case(
        "duel_1v1",
        Scenario::duel(unit.clone(), 240.0),
        runs,
        seed,
    ));

    let dense_runs = (runs / 100).max(10);
    cases.push(benchmark_case(
        "formation_16v16",
        formation_scenario(unit, 16),
        dense_runs,
        seed,
    ));

    let mut burst = unit.clone();
    if let Some(weapon) = burst.weapons.first_mut() {
        weapon.burst = weapon.burst.max(8);
        weapon.burst_rate = weapon.burst_rate.min(0.05);
        weapon.area_of_effect = weapon.area_of_effect.max(64.0);
    }
    cases.push(benchmark_case(
        "burst_aoe_4v4",
        formation_scenario(&burst, 4),
        (runs / 10).max(10),
        seed,
    ));

    let mut slow = unit.clone();
    if let Some(weapon) = slow.weapons.first_mut() {
        weapon.hitscan = false;
        weapon.projectile_speed = 100.0;
        weapon.range = weapon.range.max(800.0);
    }
    cases.push(benchmark_case(
        "long_flight_1v1",
        Scenario::duel(slow, 600.0),
        runs,
        seed,
    ));

    let mut blocked = unit.clone();
    blocked.movement.max_speed = 0.0;
    blocked.movement.acceleration = 0.0;
    if let Some(weapon) = blocked.weapons.first_mut() {
        weapon.range = weapon.range.max(500.0);
    }
    let mut blocked_scenario = Scenario::duel(blocked, 240.0);
    blocked_scenario.max_time = 5.0;
    let ridge_col = blocked_scenario.terrain.cols / 2;
    for row in 0..blocked_scenario.terrain.rows {
        blocked_scenario.terrain.heights[row * blocked_scenario.terrain.cols + ridge_col] = 100.0;
    }
    cases.push(benchmark_case(
        "terrain_los_blocked_1v1",
        blocked_scenario,
        runs,
        seed,
    ));
    cases.push(benchmark_traced_case(unit, (runs / 1_000).max(10), seed));
    cases.push(benchmark_strategic_case(unit, runs));

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "benchmark_format": 2,
            "unit": unit.name,
            "seed": seed,
            "source_control_revision": source_control_revision(),
            "catalog_hash": catalog_hash(catalog_path),
            "rayon_threads": rayon::current_num_threads(),
            "simulator_version": env!("CARGO_PKG_VERSION"),
            "target": {
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
            },
            "capability_flags": [
                "tactical", "trace", "strategic_world", "economy", "construction",
                "placement", "occupancy", "routing", "live_protocol", "allocation_metrics"
            ],
            "allocation_counter": {
                "scope": "timed_workload_only",
                "reallocation_accounting": "one_call_and_new_size",
                "deallocations_counted": false,
                "thread_shards": ALLOCATION_SHARDS,
            },
            "cases": cases,
        }))
        .expect("benchmark result serializes")
    );
}

fn benchmark_traced_case(unit: &UnitDef, runs: usize, seed: u64) -> serde_json::Value {
    let scenario = Scenario::duel(unit.clone(), 240.0);
    let scen_hash = scenario_hash(&scenario);
    let simulator = Simulator::new(scenario);
    let allocation_measurement = AllocationMeasurement::begin();
    let started = std::time::Instant::now();
    let traced = simulator.run_traced_parallel(
        [&BuiltinPolicy::Kite, &BuiltinPolicy::Charge],
        runs,
        seed,
        TraceConfig {
            sample_every_ticks: 3,
            max_frames: 128,
        },
    );
    let elapsed = started.elapsed().as_secs_f64();
    let allocations = allocation_measurement.finish();
    let frames: usize = traced.iter().map(|(_, trace)| trace.frames.len()).sum();
    let simulated_ticks: f64 = traced
        .iter()
        .map(|(result, _)| result.duration as f64 * visigoth_simulator::model::SIM_HZ as f64)
        .sum();
    let simulated_ticks_per_second = simulated_ticks / elapsed;
    let aggregate_realtime = simulated_ticks_per_second / visigoth_simulator::model::SIM_HZ as f64;
    serde_json::json!({
        "name": "traced_1v1",
        "scenario_hash": scen_hash,
        "runs": runs,
        "frames": frames,
        "simulations_per_second": runs as f64 / elapsed,
        "simulated_ticks_per_second": simulated_ticks_per_second,
        "spawned_unit_ticks_per_second": (simulated_ticks * 2.0) / elapsed,
        "aggregate_realtime": aggregate_realtime,
        "trace_frames_per_second": frames as f64 / elapsed,
        "allocations": allocation_metrics(
            allocations,
            Some(runs as f64),
            simulated_ticks,
            simulated_ticks * 2.0,
            Some(frames as f64),
        ),
    })
}

fn benchmark_strategic_case(unit: &UnitDef, ticks: usize) -> serde_json::Value {
    let json = r#"{
        "map_name":"benchmark", "map_size":[512,512], "water_level":0,
        "terrain":{"heights":[[0,0],[0,0]],
          "movement_classes":{"default":[[true,true],[true,true]]}},
        "metal_spots":[], "geothermal_spots":[]
    }"#;
    let map = Arc::new(
        CompiledMap::compile(
            MapSource::from_hello_json(json, HelloMapOptions::default())
                .expect("benchmark map source"),
        )
        .expect("benchmark map"),
    );
    let mut definition = unit.clone();
    definition.strategic.is_building = false;
    definition.strategic.is_factory = false;
    definition.strategic.is_mobile = true;
    definition.strategic.movement_class = Some("default".into());
    definition.strategic.metal_make = 1.0;
    let catalog = Arc::new(WorldCatalog::new(vec![definition]).expect("benchmark catalog"));
    let mut world = World::new(
        map,
        catalog,
        WorldConfig {
            starting_resources: Resources::new(1_000.0, 1_000.0),
            base_storage: Resources::new(1_000_000.0, 1_000_000.0),
            ..Default::default()
        },
    )
    .expect("benchmark world");
    let entities_per_side = 16;
    for side in 0..2 {
        for index in 0..entities_per_side {
            world
                .spawn_active(
                    side,
                    UnitDefId(0),
                    Vec2::new(64.0 + index as f32 * 16.0, 128.0 + side as f32 * 128.0),
                    BuildFacing::South,
                )
                .expect("benchmark entity");
        }
    }
    let ticks = ticks.max(1);
    let allocation_measurement = AllocationMeasurement::begin();
    let started = std::time::Instant::now();
    for _ in 0..ticks {
        world.step();
    }
    let elapsed = started.elapsed().as_secs_f64();
    let allocations = allocation_measurement.finish();
    let spawned_unit_ticks = ticks as f64 * (entities_per_side * 2) as f64;
    let simulated_ticks_per_second = ticks as f64 / elapsed;
    let spawned_unit_ticks_per_second = spawned_unit_ticks / elapsed;
    let aggregate_realtime = simulated_ticks_per_second / visigoth_simulator::model::SIM_HZ as f64;
    let mut strat_hash = 0xcbf29ce484222325_u64;
    for b in json.as_bytes() {
        strat_hash ^= *b as u64;
        strat_hash = strat_hash.wrapping_mul(0x100000001b3);
    }
    for b in unit.name.as_bytes() {
        strat_hash ^= *b as u64;
        strat_hash = strat_hash.wrapping_mul(0x100000001b3);
    }
    let scen_hash = format!("fnv1a:{:016x}", strat_hash);
    serde_json::json!({
        "name": "strategic_only_16v16",
        "scenario_hash": scen_hash,
        "ticks": ticks,
        "spawned_units": entities_per_side * 2,
        "simulated_ticks_per_second": simulated_ticks_per_second,
        "spawned_unit_ticks_per_second": spawned_unit_ticks_per_second,
        "aggregate_realtime": aggregate_realtime,
        "terminal_state_hash": world.state_hash(),
        "allocations": allocation_metrics(
            allocations,
            None,
            ticks as f64,
            spawned_unit_ticks,
            None,
        ),
    })
}

fn benchmark_case(name: &str, scenario: Scenario, runs: usize, seed: u64) -> serde_json::Value {
    let scen_hash = scenario_hash(&scenario);
    let spawned_units = scenario.units.len();
    let simulator = Simulator::new(scenario);
    let allocation_measurement = AllocationMeasurement::begin();
    let summary =
        simulator.run_parallel([&BuiltinPolicy::Kite, &BuiltinPolicy::Charge], runs, seed);
    let allocations = allocation_measurement.finish();
    let simulated_ticks_per_second = summary.simulations_per_second
        * summary.mean_duration as f64
        * visigoth_simulator::model::SIM_HZ as f64;
    let simulated_ticks = summary.mean_duration as f64
        * visigoth_simulator::model::SIM_HZ as f64
        * summary.runs as f64;
    let spawned_unit_ticks = simulated_ticks * spawned_units as f64;
    let aggregate_realtime = summary.mean_duration as f64 * summary.simulations_per_second;
    serde_json::json!({
        "name": name,
        "scenario_hash": scen_hash,
        "runs": summary.runs,
        "spawned_units": spawned_units,
        "mean_duration": summary.mean_duration,
        "simulations_per_second": summary.simulations_per_second,
        "simulated_ticks_per_second": simulated_ticks_per_second,
        "spawned_unit_ticks_per_second": simulated_ticks_per_second * spawned_units as f64,
        "aggregate_realtime": aggregate_realtime,
        "allocations": allocation_metrics(
            allocations,
            Some(summary.runs as f64),
            simulated_ticks,
            spawned_unit_ticks,
            None,
        ),
    })
}

fn allocation_metrics(
    totals: AllocationTotals,
    runs: Option<f64>,
    simulated_ticks: f64,
    spawned_unit_ticks: f64,
    trace_frames: Option<f64>,
) -> serde_json::Value {
    serde_json::json!({
        "calls": totals.calls,
        "bytes": totals.bytes,
        "calls_per_run": runs.map(|runs| rate(totals.calls, runs)),
        "bytes_per_run": runs.map(|runs| rate(totals.bytes, runs)),
        "calls_per_simulated_tick": rate(totals.calls, simulated_ticks),
        "bytes_per_simulated_tick": rate(totals.bytes, simulated_ticks),
        "calls_per_spawned_unit_tick": rate(totals.calls, spawned_unit_ticks),
        "bytes_per_spawned_unit_tick": rate(totals.bytes, spawned_unit_ticks),
        "calls_per_trace_frame": trace_frames.map(|frames| rate(totals.calls, frames)),
        "bytes_per_trace_frame": trace_frames.map(|frames| rate(totals.bytes, frames)),
    })
}

fn rate(value: u64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        value as f64 / denominator
    } else {
        0.0
    }
}

fn formation_scenario(unit: &UnitDef, per_side: usize) -> Scenario {
    let terrain = Terrain::flat(2_000.0, 2_000.0, 32.0);
    let mut units = Vec::with_capacity(per_side * 2);
    for side in 0..2_u8 {
        for index in 0..per_side {
            let row = index / 4;
            let column = index % 4;
            let base_x = if side == 0 { 750.0 } else { 1_250.0 };
            let direction = if side == 0 { 1.0 } else { -1.0 };
            let x = base_x + direction * column as f32 * 28.0;
            let z = 850.0 + row as f32 * 60.0 + column as f32 * 4.0;
            units.push(Spawn {
                side,
                definition: unit.clone(),
                position: Vec2::new(x, z),
                heading: if side == 0 { 0.0 } else { PI },
            });
        }
    }
    Scenario {
        terrain,
        units,
        max_time: 60.0,
        trace_resources: vec![],
    }
}

fn trace_resources(map: &CompiledMap) -> Vec<ResourceMarker> {
    let mut markers = map
        .metal_nodes
        .iter()
        .map(|node| ResourceMarker {
            kind: ResourceKind::MetalSpot,
            position: [node.x, node.z],
            yield_per_second: node.yield_value,
            radius: 16.0,
        })
        .collect::<Vec<_>>();
    markers.extend(map.geothermal_nodes.iter().map(|node| ResourceMarker {
        kind: ResourceKind::Geothermal,
        position: [node.x, node.z],
        yield_per_second: 0.0,
        radius: node.radius,
    }));
    if let Some(field) = &map.metal_field {
        // A bounded preview for the viewer; extraction queries still use the
        // complete compiled field.
        let cols = (field.density.cols as usize).min(32);
        let rows = (field.density.rows as usize).min(32);
        let step_x = map.width / cols as f32;
        let step_z = map.depth / rows as f32;
        for row in 0..rows {
            for col in 0..cols {
                let x = (col as f32 + 0.5) * step_x;
                let z = (row as f32 + 0.5) * step_z;
                let density = field.density.sample(x, z);
                if density > 0.0 {
                    markers.push(ResourceMarker {
                        kind: ResourceKind::MetalField,
                        position: [x, z],
                        yield_per_second: density,
                        radius: step_x.min(step_z) * 0.35,
                    });
                }
            }
        }
    }
    markers
}

fn compile_map(args: &[String], input_path: &str) {
    let output_path = value(args, "--map-out", "");
    if output_path.is_empty() {
        usage();
    }
    let extractor_radius_arg = value(args, "--extractor-radius", "500");
    let extractor_radius: f32 = extractor_radius_arg.parse().unwrap_or_else(|_| usage());
    let extraction_scale_arg = value(args, "--extraction-scale", "1");
    let extraction_scale: f32 = extraction_scale_arg.parse().unwrap_or_else(|_| usage());
    let legacy = value(args, "--legacy-movement-class", "");
    let options = HelloMapOptions {
        content_hash: value(args, "--content-hash", ""),
        extractor_radius,
        extraction_scale,
        accept_legacy_mask_as: (!legacy.is_empty()).then_some(legacy),
        ..HelloMapOptions::default()
    };
    let json = fs::read_to_string(input_path).unwrap_or_else(|error| {
        eprintln!("error reading {input_path}: {error}");
        process::exit(1);
    });
    let (mut source, input_format) = MapSource::from_json(&json, options).unwrap_or_else(|error| {
        eprintln!("error adapting {input_path}: {error}");
        process::exit(1);
    });
    if input_format == MapInputFormat::NativeSource {
        validate_native_cli_overrides(args, &source, input_path);
        let explicit_acquisition = value(args, "--acquisition", "");
        let acquisition_path = if explicit_acquisition.is_empty() {
            sibling_acquisition_path(Path::new(input_path)).filter(|path| path.is_file())
        } else {
            Some(Path::new(&explicit_acquisition).to_owned())
        };
        if let Some(path) = acquisition_path {
            let metadata = fs::read_to_string(&path).unwrap_or_else(|error| {
                eprintln!("error reading {}: {error}", path.display());
                process::exit(1);
            });
            source
                .apply_acquisition_json(&metadata)
                .unwrap_or_else(|error| {
                    eprintln!("error applying {}: {error}", path.display());
                    process::exit(1);
                });
        } else if !source.provenance.is_complete() {
            eprintln!(
                "warning: native map has no acquisition metadata; cache identity is limited to content_hash (expected sibling *.acquisition.json or --acquisition PATH)"
            );
        }
    }
    let map = CompiledMap::compile(source).unwrap_or_else(|error| {
        eprintln!("error compiling {input_path}: {error}");
        process::exit(1);
    });
    fs::write(&output_path, map.to_cache_bytes()).unwrap_or_else(|error| {
        eprintln!("error writing {output_path}: {error}");
        process::exit(1);
    });
    println!(
        "{}",
        serde_json::json!({
            "map": map.map_name,
            "input_format": match input_format { MapInputFormat::NativeSource => "native_map_source", MapInputFormat::Hello => "bridge_hello" },
            "provenance_complete": map.provenance.is_complete(),
            "content_hash": map.content_hash,
            "width": map.width,
            "depth": map.depth,
            "movement_classes": map.passability.iter().map(|layer| &layer.movement_class).collect::<Vec<_>>(),
            "metal_nodes": map.metal_nodes.len(),
            "continuous_metal": map.metal_field.is_some(),
            "geothermal_nodes": map.geothermal_nodes.len(),
            "static_blockers": map.static_blockers.len(),
            "output": output_path,
        })
    );
}

/// Loads a cache or source description for an interactive/session run. Source
/// descriptions are deliberately compiled only once here, before the world is
/// created, so tactical and strategic ticks retain the same hot path as cached
/// maps.
fn load_map_or_exit(path: &str) -> CompiledMap {
    let (map, format) = CompiledMap::load_path(Path::new(path), MapLoadOptions::default())
        .unwrap_or_else(|error| {
            eprintln!("error loading map {path}: {error}");
            process::exit(1);
        });
    if format != visigoth_simulator::MapLoadFormat::CompiledCache {
        eprintln!(
            "loaded {path} as {format:?}; compiled in memory (use --compile-map for repeat batch runs)"
        );
    }
    map
}

fn sibling_acquisition_path(input: &Path) -> Option<std::path::PathBuf> {
    let file_name = input.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".map_source.json")?;
    Some(input.with_file_name(format!("{stem}.acquisition.json")))
}

fn validate_native_cli_overrides(args: &[String], source: &MapSource, input_path: &str) {
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let content_hash = value(args, "--content-hash", "");
    if has("--content-hash") && content_hash != source.content_hash {
        eprintln!(
            "error: --content-hash conflicts with the canonical value in {input_path}; native identity cannot be overridden"
        );
        process::exit(1);
    }
    if has("--legacy-movement-class") {
        eprintln!("error: --legacy-movement-class applies only to bridge hello JSON");
        process::exit(1);
    }
    if let Some(metal) = &source.continuous_metal {
        if has("--extractor-radius") {
            let value: f32 = value(args, "--extractor-radius", "")
                .parse()
                .unwrap_or_else(|_| usage());
            if (value - metal.extractor_radius).abs() > 1e-4 {
                eprintln!(
                    "error: --extractor-radius conflicts with the canonical value in {input_path}; native extraction rules cannot be overridden"
                );
                process::exit(1);
            }
        }
        if has("--extraction-scale") {
            let value: f32 = value(args, "--extraction-scale", "")
                .parse()
                .unwrap_or_else(|_| usage());
            if (value - metal.extraction_scale).abs() > 1e-6 {
                eprintln!(
                    "error: --extraction-scale conflicts with the canonical value in {input_path}; native extraction rules cannot be overridden"
                );
                process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_measurement_observes_worker_threads() {
        let measurement = AllocationMeasurement::begin();
        let workers = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    let allocation = Vec::<u8>::with_capacity(std::hint::black_box(1_024));
                    std::hint::black_box(allocation);
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("allocation worker succeeds");
        }
        let totals = measurement.finish();

        // Thread spawning may allocate additional runtime bookkeeping, so these are intentionally
        // lower bounds. The important contract is that allocations on non-caller threads appear.
        assert!(totals.calls >= 4);
        assert!(totals.bytes >= 4 * 1_024);
    }

    #[test]
    fn allocation_rates_handle_real_and_empty_denominators() {
        assert_eq!(rate(15, 3.0), 5.0);
        assert_eq!(rate(15, 0.0), 0.0);
        assert_eq!(rate(15, -1.0), 0.0);
    }
}
