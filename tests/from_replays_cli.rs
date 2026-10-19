use std::{fs, process::Command};

#[test]
fn exported_baselines_have_versioned_trainable_modules() {
    let root = std::env::temp_dir().join(format!("minibar-replay-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for kind in ["tactical_v4", "strategic_v1"] {
        let path = root.join(format!("{kind}.json"));
        let output = Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
            .args(["--seed-brain", kind, "--brain-out"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(value["format"], "minibar-brain");
        assert_eq!(value["kind"], kind);
        assert!(!value["modules"].as_array().unwrap().is_empty());
        assert!(
            value["modules"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["inputs"].as_array().unwrap().len() <= 32)
        );
    }
}

#[test]
fn old_working_directory_relative_training_paths_still_resolve() {
    let dir = std::env::temp_dir().join(format!("minibar-replay-legacy-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    fs::write(&path, r#"{"init_from":"examples/pawn-hello.json"}"#).unwrap();
    let config = visigoth_simulator::learn::train::TrainConfig::load(&path).unwrap();
    assert_eq!(
        config.init_from.unwrap(),
        std::env::current_dir()
            .unwrap()
            .join("examples/pawn-hello.json")
    );
}
