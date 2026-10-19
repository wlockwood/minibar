use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[test]
fn diagnostic_frontier_retains_tradeoffs_without_accepting_faction_regression() {
    use visigoth_simulator::learn::campaign_eval::PlanReport;
    use visigoth_simulator::learn::campaign_frontier::{FrontierState, protected};
    let report = |a: f64, b: f64| -> PlanReport {
        serde_json::from_value(json!({"fitness":(a+b)/2.0,"stages":[{"name":"easy","difficulty":1,"weight":1,"fitness":a,"details":{"facets_complete":true,"facets":{"armada":a}}},{"name":"objectives","difficulty":2,"weight":1,"fitness":b,"details":{"facets_complete":true,"facets":{"armada":b}}}]})).unwrap()
    };
    let reference = report(0.1, 0.1);
    let mut frontier = FrontierState::default();
    frontier
        .consider(
            json!({"w":1}),
            report(0.2, 0.8),
            &reference,
            json!({"generation":1}),
            8,
            0.05,
        )
        .unwrap();
    frontier
        .consider(
            json!({"w":2}),
            report(0.6, 0.4),
            &reference,
            json!({"generation":2}),
            8,
            0.05,
        )
        .unwrap();
    assert_eq!(frontier.entries.len(), 2);
    assert!(!protected(&report(0.0, 0.9), &reference, 0.05).unwrap());
    let bytes = serde_json::to_vec(&frontier).unwrap();
    frontier
        .consider(
            json!({"w":2}),
            report(0.6, 0.4),
            &reference,
            json!({"generation":3}),
            8,
            0.05,
        )
        .unwrap();
    assert_eq!(bytes, serde_json::to_vec(&frontier).unwrap());
}
#[test]
fn diagnostic_frontier_is_bounded_deterministic_and_records_pruning() {
    use visigoth_simulator::learn::{campaign_eval::PlanReport, campaign_frontier::FrontierState};
    let report = |a: f64, b: f64| -> PlanReport {
        serde_json::from_value(json!({"fitness":(a+b)/2.0,"stages":[{"name":"easy","difficulty":1,"weight":1,"fitness":a,"details":{"facets_complete":true,"facets":{"armada":a,"cortex":b}}}]})).unwrap()
    };
    let reference = report(0.0, 0.0);
    let build = || {
        let mut frontier = FrontierState::default();
        for i in 0..12 {
            frontier
                .consider(
                    json!({"w":i}),
                    report(0.1 + i as f64 * 0.01, 0.22 - i as f64 * 0.01),
                    &reference,
                    json!({"generation":i}),
                    3,
                    0.05,
                )
                .unwrap();
        }
        frontier.validate().unwrap();
        frontier
    };
    let frontier = build();
    assert_eq!(frontier.entries.len(), 3);
    assert!(!frontier.pruning.is_empty());
    assert_eq!(
        serde_json::to_value(&frontier).unwrap(),
        serde_json::to_value(build()).unwrap()
    );
    let mut damaged = frontier.clone();
    damaged.entries[0].brain = json!({"w":"changed"});
    assert!(damaged.validate().is_err());
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_owned()
}
fn scratch() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "minibar-quality-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn run(args: &[&str]) -> Value {
    let p = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args(args)
        .env("RAYON_NUM_THREADS", "1")
        .output()
        .unwrap();
    assert!(p.status.success(), "{}", String::from_utf8_lossy(&p.stderr));
    serde_json::from_slice(&p.stdout).unwrap()
}
#[test]
fn no_production_bonus_reports_auditable_components() {
    let tmp = scratch();
    let repo = root();
    let source = repo.join("examples/campaign/stages/smoke-strategic-1.json");
    let mut cfg: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    for map in cfg["maps"].as_array_mut().unwrap() {
        map["map"] = json!(
            source
                .parent()
                .unwrap()
                .join(map["map"].as_str().unwrap())
                .canonicalize()
                .unwrap()
        );
    }
    cfg["reward"] = json!({"production_mode":"none"});
    let config = tmp.join("stage.json");
    fs::write(&config, serde_json::to_vec(&cfg).unwrap()).unwrap();
    let report = run(&[
        "--catalog",
        repo.join("data/brains/from_replays/catalog.json")
            .to_str()
            .unwrap(),
        "--evaluate-strategy",
        repo.join("data/brains/from_replays/strategic/baseline.json")
            .to_str()
            .unwrap(),
        "--train-config",
        config.to_str().unwrap(),
        "--eval-seeds",
        "1",
    ]);
    for trial in report["trials"].as_array().unwrap() {
        assert_eq!(trial["reward_components"]["production"], 0.0);
        assert_eq!(trial["reward_components"]["total"], trial["fitness"]);
        assert!(trial["telemetry"]["terminal_outcome"].is_string());
    }
    fs::remove_dir_all(tmp).unwrap();
}
#[test]
fn strategic_v2_upgrade_preserves_existing_columns_and_refuses_overwrite() {
    let tmp = scratch();
    let source = root().join("data/brains/from_replays/strategic/baseline.json");
    let target = tmp.join("v2.json");
    run(&[
        "--upgrade-brain",
        source.to_str().unwrap(),
        "--brain-kind",
        "strategic_v2",
        "--brain-out",
        target.to_str().unwrap(),
    ]);
    let before: Value = serde_json::from_slice(&fs::read(source).unwrap()).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
    assert_eq!(after["kind"], "strategic_v2");
    let old = before["modules"][1]["inputs"].as_array().unwrap().len();
    assert!(after["modules"][1]["inputs"].as_array().unwrap().len() > old);
    for (a, b) in before["modules"][1]["layers"][0]["weights"]
        .as_array()
        .unwrap()
        .iter()
        .zip(
            after["modules"][1]["layers"][0]["weights"]
                .as_array()
                .unwrap(),
        )
    {
        assert_eq!(a.as_array().unwrap(), &b.as_array().unwrap()[..old]);
        assert!(
            b.as_array().unwrap()[old..]
                .iter()
                .all(|x| x.as_f64() == Some(0.0))
        );
    }
    let result = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .args([
            "--upgrade-brain",
            target.to_str().unwrap(),
            "--brain-kind",
            "strategic_v2",
            "--brain-out",
            target.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    fs::remove_dir_all(tmp).unwrap();
}
#[test]
fn campaign_evaluation_exposes_multiple_layouts_without_writing_training_state() {
    let report = run(&[
        "--evaluate-campaign",
        root()
            .join("data/brains/from_replays/tactical/baseline.json")
            .to_str()
            .unwrap(),
        "--campaign-config",
        root()
            .join("examples/campaign/smoke.json")
            .to_str()
            .unwrap(),
        "--campaign-track",
        "tactical",
        "--campaign-plan",
        "validation",
        "--eval-layouts",
        "2",
        "--eval-seeds",
        "1",
        "--eval-seed-base",
        "8000000000000",
    ]);
    assert_eq!(
        report["stages"][0]["details"]["layouts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(report["stages"][0]["details"]["mean_layout_fitness"].is_number());
}
