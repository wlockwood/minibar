//! End-to-end: train a tiny council, then evaluate, explain and watch it, all
//! through the command line the way a user (or scripts/evolve.py) would.

use std::{fs, path::PathBuf, process::Command};

fn exe() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"));
    command.args([
        "--catalog",
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/pawn-hello.json"),
        "--unit",
        "armpw",
    ]);
    command
}

fn run(mut command: Command) -> String {
    let output = command.output().expect("simulator runs");
    assert!(
        output.status.success(),
        "{:?} failed:\n{}",
        command,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 output")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("minibar-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn train_evaluate_watch_and_explain_a_tiny_council() {
    let dir = temp_dir("learn-cli");
    let config = dir.join("config.json");
    fs::write(
        &config,
        r#"{
            "name": "cli-smoke", "hidden": 4, "generations": 2,
            "es": {"population": 8, "seed": 5},
            "seeds_per_battle": 1, "validation_seeds": 2, "validation_every": 1,
            "opponents": ["charge", "kite"], "max_time": 15,
            "scenarios": [
              {"name": "1v1", "learner": [{"unit": "armpw", "count": 1}], "enemy": [{"unit": "armpw", "count": 1}]},
              {"name": "2v3", "learner": [{"unit": "armpw", "count": 2}], "enemy": [{"unit": "armpw", "count": 3}]}
            ]
        }"#,
    )
    .unwrap();

    // Train.
    let mut train = exe();
    train.args([
        "--train",
        config.to_str().unwrap(),
        "--train-out",
        dir.join("out").to_str().unwrap(),
    ]);
    let summary: serde_json::Value = serde_json::from_str(&run(train)).expect("summary is JSON");
    assert_eq!(summary["name"], "cli-smoke");
    let brain = dir.join("out").join("brain.best.json");
    assert!(brain.exists());
    let resolved = dir.join("out").join("train_config.json");

    // Evaluate on fresh seeds: deterministic, and honest about the tallies.
    let evaluate = || {
        let mut c = exe();
        c.args([
            "--evaluate",
            brain.to_str().unwrap(),
            "--train-config",
            resolved.to_str().unwrap(),
            "--eval-seeds",
            "2",
            "--eval-seed-base",
            "42",
        ]);
        run(c)
    };
    let first = evaluate();
    assert_eq!(first, evaluate());
    let evaluation: serde_json::Value = serde_json::from_str(&first).unwrap();
    let total = &evaluation["total"];
    let parts: u64 = ["wins", "draws", "timeouts", "losses"]
        .iter()
        .map(|k| total[k].as_u64().unwrap())
        .sum();
    assert_eq!(parts, total["battles"].as_u64().unwrap());
    assert_eq!(evaluation["rows"].as_array().unwrap().len(), 2 * 2);

    // Watch: JSON for the live server, and a standalone page.
    let mut battle = exe();
    battle.args([
        "--watch-battle",
        brain.to_str().unwrap(),
        "--train-config",
        resolved.to_str().unwrap(),
        "--watch-scenario",
        "1",
        "--watch-opponent",
        "0",
        "--watch-seed",
        "3",
        "--watch-side",
        "1",
    ]);
    let battle: serde_json::Value = serde_json::from_str(&run(battle)).expect("battle is JSON");
    assert_eq!(battle["learner_side"], 1);
    assert_eq!(battle["units"].as_array().unwrap().len(), 5);
    assert!(
        battle["frames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| !f["d"].as_array().unwrap().is_empty())
    );

    let page = dir.join("watch.html");
    let mut watch = exe();
    watch.args([
        "--watch",
        brain.to_str().unwrap(),
        "--train-config",
        resolved.to_str().unwrap(),
        "--watch-out",
        page.to_str().unwrap(),
    ]);
    run(watch);
    let html = fs::read_to_string(&page).unwrap();
    assert!(html.contains("\"learner_side\":0") && !html.contains("/*__WATCH_DATA__*/null"));

    // Explain.
    let mut explain = exe();
    explain.args([
        "--explain",
        brain.to_str().unwrap(),
        "--train-config",
        resolved.to_str().unwrap(),
    ]);
    let report = run(explain);
    assert!(report.contains("## `spacing`") && report.contains("| timeout |"));

    // A trained council is a first-class policy for ordinary duels.
    let mut duel = exe();
    duel.args([
        "--left",
        &format!("brain:{}", brain.display()),
        "--right",
        "kite",
        "--runs",
        "4",
        "--seed",
        "1",
    ]);
    let duel: serde_json::Value = serde_json::from_str(&run(duel)).unwrap();
    assert_eq!(duel["summary"]["runs"], 4);

    let _ = fs::remove_dir_all(&dir);
}
