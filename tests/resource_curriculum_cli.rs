use serde_json::{Value, json};
use std::{fs, process::Command};

#[test]
fn paired_promotion_protects_a_family_hidden_by_another_familys_gain() {
    use visigoth_simulator::learn::{campaign_eval::PlanReport, campaign_promotion};
    let report = |a: f64, b: f64| -> PlanReport {
        serde_json::from_value(json!({"fitness":(a+b)/2.0,"stages":[{
            "name":"mixed", "difficulty":1,"weight":1.0,"fitness":(a+b)/2.0,
            "details":{"families":{"heavy_swarm":a,"bomber_aa":b}}
        }]}))
        .unwrap()
    };
    let decision = campaign_promotion::assess_with_t(
        vec![report(1.0, -0.5); 4],
        vec![report(0.0, 0.0); 4],
        0.001,
        0.15,
        2.0,
    )
    .unwrap();
    assert!(decision.mean_gain > 0.0);
    assert!(
        !decision.accepted,
        "family regression must veto aggregate gain"
    );
}

#[test]
fn preview_uses_native_resource_values_and_explicit_supply() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("minibar-resource-preview-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let config = dir.join("train.json");
    fs::write(
        &config,
        json!({
            "name":"resource preview", "resource_supply":{"metal":1000000,"energy":1000000},
            "matchups":{"count":1,"types":[],"max_units":96,
              "anchors":[{"name":"40 Pawns","family":"heavy_swarm",
                "learner":[{"unit":"armpw","count":1}],
                "enemy":[{"unit":"armpw","count":1}],"max_gap":0.05}]}
        })
        .to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
        .current_dir(root)
        .args([
            "--catalog",
            "examples/t1-catalog.json",
            "--preview-curriculum",
        ])
        .arg(&config)
        .args(["--seed", "123"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["seed"], 123);
    let row = &report["matchup_records"][0];
    assert_eq!(row["family"], "heavy_swarm");
    assert_eq!(row["absolute_gap"], 0.0);
    assert_eq!(row["resource_supply"]["energy"], 1000000.0);
    assert!(row["combat_value"][0].as_f64().unwrap() > 54.0);
    assert_eq!(report["scenarios"][0], row["scenario"]);
    fs::remove_dir_all(dir).unwrap();
}
