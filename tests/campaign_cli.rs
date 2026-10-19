use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[test]
fn fresh_screening_and_champion_follower_resume_exactly() {
    let f = Fixture::new("fresh-follow");
    let mut cfg = read_json(&f.config);
    cfg["fresh_screening"] = json!(true);
    cfg["screen_min_gain"] = json!(0.0);
    cfg["validation_every"] = json!(1);
    cfg["tracks"][0]["validation"]["layouts"] = json!(2);
    cfg["tracks"][0]["islands"][0]["follow_champion"] = json!(true);
    cfg["tracks"][0]["islands"][1]["restart"] = json!(false);
    cfg["tracks"][0]["islands"][1]["structural_search"] = json!({"every":30});
    write_json(&f.config, &cfg);
    let full = f.root.join("full");
    let split = f.root.join("split");
    success(f.campaign(&full, 2, false, false));
    success(f.campaign(&split, 1, false, false));
    success(f.campaign(&split, 2, true, false));
    let state = read_json(&full.join("state.json"));
    assert_eq!(state, read_json(&split.join("state.json")));
    assert_eq!(
        state["tracks"][0]["islands"][0]["champion_follows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(state["tracks"][0]["islands"][1]["structural_attempts"], 1);
    for generation in state["history"].as_array().unwrap() {
        for island in generation["tracks"][0]["islands"].as_array().unwrap() {
            assert_eq!(island["decisions"].as_array().unwrap().len(), 2);
            for decision in island["decisions"].as_array().unwrap() {
                assert!(decision["screen_candidate"].is_object());
                assert!(decision["screen_incumbent"].is_object());
                assert!(decision["reason"].is_string());
            }
        }
    }
    let mut forged = read_json(&split.join("state.json"));
    forged["tracks"][0]["islands"][0]["champion_follows"][0]["brain_hash"] = json!("forged");
    write_json(&split.join("state.json"), &forged);
    assert!(!f.campaign(&split, 3, true, false).status.success());
}

#[test]
fn fresh_gate_can_promote_below_old_audit_and_follower_adopts_winning_structure() {
    let f = Fixture::new("fresh-structural-winner");
    let mut cfg = read_json(&f.config);
    cfg["fresh_screening"] = json!(true);
    cfg["min_gain"] = json!(0.005);
    cfg["promotion_seeds"] = json!(3);
    cfg["promotion_min_t"] = json!(2.0);
    cfg["max_stage_regression"] = json!(100.0);
    cfg["validation_every"] = json!(1);
    cfg["tracks"][0]["validation"]["layouts"] = json!(2);
    cfg["tracks"][0]["islands"][0]["follow_champion"] = json!(true);
    cfg["tracks"][0]["islands"][1]["restart"] = json!(false);
    cfg["tracks"][0]["islands"][1]["structural_search"] =
        json!({"every":30,"activation_weight":0,"input_weight":1,"connection_weight":0});
    write_json(&f.config, &cfg);
    let out = f.root.join("external");
    let mut done = false;
    for round in 0..120 {
        let result = success(f.campaign(&out, 2, round > 0, true));
        if result["status"] == "completed" {
            done = true;
            break;
        }
        assert_eq!(result["status"], "waiting_for_workers");
        for entry in fs::read_dir(out.join("work/jobs")).unwrap() {
            let path = entry.unwrap().path();
            let result_path = out.join("work/results").join(path.file_name().unwrap());
            if result_path.exists() {
                continue;
            }
            let job = read_json(&path);
            success(
                native()
                    .arg("--campaign-worker")
                    .arg(&path)
                    .arg("--job-out")
                    .arg(&result_path)
                    .output()
                    .unwrap(),
            );
            let mut result = read_json(&result_path);
            let label = job["label"].as_str().unwrap();
            // Trusted worker fixture: historical audit favours the incumbent,
            // but independent fresh conditions favour the structural centre.
            let score = if job["plan"] == "promotion" {
                if label.contains("/incumbent/") {
                    1.0
                } else {
                    2.0
                }
            } else if label.contains("/screen/") {
                if label.contains("/wide/center/screen/candidate/") {
                    2.0
                } else if label.contains("/incumbent/") {
                    1.0
                } else {
                    0.0
                }
            } else if job["plan"] == "validation" {
                if label.starts_with("generation-") {
                    9.0
                } else {
                    10.0
                }
            } else {
                result["report"]["fitness"].as_f64().unwrap()
            };
            result["report"]["fitness"] = json!(score);
            write_json(&result_path, &result);
        }
    }
    assert!(done);
    let state = read_json(&out.join("state.json"));
    let track = &state["tracks"][0];
    let first_winner = &track["promotion_history"][0];
    assert_eq!(first_winner["island"], "wide");
    let first = &state["history"][0]["tracks"][0]["islands"][1];
    assert_eq!(first["decisions"][0]["reason"], "promoted");
    assert!(
        first["center_validation"]["fitness"].as_f64().unwrap()
            < track["initial_report"]["fitness"].as_f64().unwrap()
    );
    let follower = &track["islands"][0];
    assert_eq!(follower["champion_follows"].as_array().unwrap().len(), 2);
    assert_eq!(
        follower["champion_follows"][1]["brain_hash"],
        first_winner["brain_sha256"]
    );
    let inherited = &track["structure_snapshots"][first_winner["brain_sha256"].as_str().unwrap()];
    assert_eq!(&follower["base"], inherited);
    assert!(
        inherited["modules"]
            .as_array()
            .unwrap()
            .iter()
            .zip(track["base"]["modules"].as_array().unwrap())
            .any(|(a, b)| a["structure"] != b["structure"])
    );
    // Native validation of completed state must accept the recorded adoption.
    success(f.campaign(&out, 2, true, false));
}

#[test]
fn structural_campaign_inherits_champion_preserves_rejections_and_resumes_exactly() {
    let fixture = Fixture::new("structural");
    let mut config: Value = serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
    config["tracks"][0]["islands"][1]["restart"] = json!(false);
    config["tracks"][0]["islands"][1]["structural_search"] =
        json!({"every": 1, "activation_weight":0.0,"input_weight":1.0,"connection_weight":0.0});
    write_json(&fixture.config, &config);
    let full = fixture.root.join("full");
    success(fixture.campaign(&full, 2, false, false));
    let split = fixture.root.join("split");
    success(fixture.campaign(&split, 1, false, false));
    success(fixture.campaign(&split, 2, true, false));
    let a: Value = serde_json::from_slice(&fs::read(full.join("state.json")).unwrap()).unwrap();
    let b: Value = serde_json::from_slice(&fs::read(split.join("state.json")).unwrap()).unwrap();
    assert_eq!(a, b);
    let track = &a["tracks"][0];
    assert_eq!(track["best"], track["base"]);
    assert_eq!(track["structural_history"].as_array().unwrap().len(), 2);
    assert_eq!(track["islands"][1]["structural_attempts"], 2);
    assert_eq!(track["islands"][0]["structural_attempts"], 0);
    assert_ne!(track["islands"][1]["base"], track["base"]);
    let snapshots = track["structure_snapshots"].as_object().unwrap();
    assert!(snapshots.len() >= 3);
    for event in track["structural_history"].as_array().unwrap() {
        assert!(snapshots.contains_key(event["proposal_sha256"].as_str().unwrap()));
        let parent = &snapshots[event["parent_sha256"].as_str().unwrap()];
        let proposal = &snapshots[event["proposal_sha256"].as_str().unwrap()];
        for (m, n) in parent["modules"]
            .as_array()
            .unwrap()
            .iter()
            .zip(proposal["modules"].as_array().unwrap())
        {
            assert_eq!(m["layers"], n["layers"]);
        }
    }
    let mut tampered = b.clone();
    let on = tampered["tracks"][0]["islands"][1]["base"]["modules"][0]["structure"]["inputs"][0]
        .as_bool()
        .unwrap();
    tampered["tracks"][0]["islands"][1]["base"]["modules"][0]["structure"]["inputs"][0] =
        json!(!on);
    write_json(&split.join("state.json"), &tampered);
    let refused = fixture.campaign(&split, 3, true, false);
    assert!(
        !refused.status.success(),
        "tampered island structure must not resume"
    );
    let mut weight_only = b.clone();
    weight_only["tracks"][0]["islands"][0]["base"] = b["tracks"][0]["base"].clone();
    weight_only["tracks"][0]["islands"][0]["base"]["modules"][0]["structure"]["inputs"][0] =
        json!(false);
    write_json(&split.join("state.json"), &weight_only);
    assert!(
        !fixture.campaign(&split, 3, true, false).status.success(),
        "weight-only optimizer must retain its inherited structure"
    );
    let inherited = visigoth_simulator::learn::Brain::from_json(
        &b["tracks"][0]["islands"][1]["base"].to_string(),
    )
    .unwrap();
    let mask = inherited
        .trainable_mask(&["mission".into(), "spacing".into()])
        .unwrap();
    let inactive = inherited
        .parameter_activity(&mask)
        .iter()
        .position(|active| !active)
        .unwrap();
    let mut dormant = b.clone();
    let old = dormant["tracks"][0]["islands"][1]["es"]["theta"][inactive]
        .as_f64()
        .unwrap();
    dormant["tracks"][0]["islands"][1]["es"]["theta"][inactive] = json!(old + 1.0);
    write_json(&split.join("state.json"), &dormant);
    assert!(
        !fixture.campaign(&split, 3, true, false).status.success(),
        "dormant optimizer weights must match inherited values"
    );
    let mut bad_promotion = b.clone();
    bad_promotion["tracks"][0]["promotion_history"] = json!([{"generation":999,"island":"wide","candidate":"sample","parent_sha256":"missing","brain_sha256":"missing"}]);
    write_json(&split.join("state.json"), &bad_promotion);
    assert!(
        !fixture.campaign(&split, 3, true, false).status.success(),
        "promotion history must match committed results"
    );
}

#[test]
fn campaign_console_reports_progress_and_generation_details_without_polluting_json() {
    let fixture = Fixture::new("console-progress");
    let output = fixture.campaign(&fixture.root.join("run"), 1, false, false);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("campaign progress"), "{stderr}");
    assert!(stderr.contains("generation 1/1 committed in"), "{stderr}");
    assert!(stderr.contains("tactical champion validation"), "{stderr}");
    assert!(stderr.contains("local: candidates mean"), "{stderr}");
    assert!(stderr.contains("validation center"), "{stderr}");
    assert!(stderr.contains("promotions: none"), "{stderr}");
    assert_eq!(success(output)["completed_generations"], 1);
}

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    stages: Vec<PathBuf>,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "minibar-campaign-cli-{label}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let catalog = root.join("catalog.json");
        let brain = root.join("baseline.json");
        fs::copy(repo.join("data/brains/from_replays/catalog.json"), &catalog).unwrap();
        fs::copy(
            repo.join("data/brains/from_replays/tactical/baseline.json"),
            &brain,
        )
        .unwrap();
        let mut stages = Vec::new();
        let mut stage_specs = Vec::new();
        for (index, name) in ["easy", "mixed", "objectives"].iter().enumerate() {
            let count = if index == 1 { 2 } else { 1 };
            let mut scenario = json!({"name":name,
                "learner":[{"unit":"armpw","count":count}],
                "enemy":[{"unit":"armpw","count":count}],"separation":200});
            if index == 2 {
                scenario["objective"] = json!({"kind":"defend","target":"armlab"});
            }
            let stage = root.join(format!("{name}.json"));
            write_json(
                &stage,
                &json!({"name":name,"brain_kind":"tactical_v4","hidden":8,
                "max_time":1.0,"layout_jitter":0.0,"opponents":["charge"],
                "seeds_per_battle":1,"validation_seeds":1,"scenarios":[scenario]}),
            );
            stage_specs.push(json!({"name":name,"difficulty":index+1,"weight":index+1,
                "task":{"kind":"tactical","config":stage}}));
            stages.push(stage);
        }
        let plan = json!({"catalog":catalog,"seeds":1,"stages":stage_specs});
        let config = root.join("campaign.json");
        write_json(
            &config,
            &json!({"format":"minibar-campaign","version":1,
            "name":"native CLI regression","generations":3,"seed":9173,
            "validation_every":3,"min_gain":100.0,"max_stage_regression":0.15,
            "restart_after":1,"restart_scale":0.25,"archive_size":2,
            "tracks":[{"name":"tactical","brain":brain,"train_modules":["mission","spacing"],
                "training":plan,"validation":plan,"holdout":null,
                "islands":[
                    {"name":"local","es":{"population":2,"sigma":0.04,"learning_rate":0.01,"seed":11},"restart":false},
                    {"name":"wide","es":{"population":2,"sigma":0.2,"learning_rate":0.01,"seed":29},"restart":true}
                ]}]}),
        );
        Self {
            root,
            config,
            stages,
        }
    }

    fn campaign(&self, out: &Path, generations: u64, resume: bool, external: bool) -> Output {
        let mut command = native();
        command
            .current_dir(&self.root)
            .arg("--train-campaign")
            .arg(&self.config)
            .arg("--train-out")
            .arg(out)
            .arg("--generations")
            .arg(generations.to_string())
            .args(["--threads", "1"]);
        if resume {
            command.arg("--resume");
        }
        if external {
            command.arg("--external");
        }
        command.output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

fn native() -> Command {
    Command::new(env!("CARGO_BIN_EXE_visigoth-simulator"))
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn first_difference(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    if actual == expected {
        return None;
    }
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: object lengths {} vs {}", a.len(), b.len()));
            }
            for (key, value) in a {
                let Some(other) = b.get(key) else {
                    return Some(format!("{path}/{key}: missing key"));
                };
                if let Some(difference) = first_difference(value, other, &format!("{path}/{key}")) {
                    return Some(difference);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: array lengths {} vs {}", a.len(), b.len()));
            }
            for (index, (value, other)) in a.iter().zip(b).enumerate() {
                if let Some(difference) = first_difference(value, other, &format!("{path}/{index}"))
                {
                    return Some(difference);
                }
            }
        }
        _ => return Some(format!("{path}: {actual} vs {expected}")),
    }
    None
}

fn assert_same_json(actual: &Value, expected: &Value, label: &str) {
    if let Some(difference) = first_difference(actual, expected, "") {
        panic!("{label}: {difference}");
    }
}

fn assert_same_bytes(actual: &Path, expected: &Path, label: &str) {
    let a = fs::read(actual).unwrap();
    let b = fs::read(expected).unwrap();
    if a != b {
        let offset = a
            .iter()
            .zip(&b)
            .position(|(a, b)| a != b)
            .unwrap_or(a.len().min(b.len()));
        let excerpt = |bytes: &[u8]| {
            String::from_utf8_lossy(
                &bytes[offset.saturating_sub(70)..(offset + 90).min(bytes.len())],
            )
            .into_owned()
        };
        panic!(
            "{label}: first byte difference {offset}: {:?} vs {:?}",
            excerpt(&a),
            excerpt(&b)
        );
    }
}

#[test]
fn invalid_campaign_is_rejected_before_creating_training_output() {
    let fixture = Fixture::new("invalid");
    fs::write(&fixture.config, "{broken").unwrap();
    let out = fixture.root.join("out");
    let result = fixture.campaign(&out, 1, false, false);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("campaign"));
    assert!(!out.exists());
}

#[test]
fn campaign_limit_flags_require_values_before_starting_a_run() {
    let fixture = Fixture::new("missing-limits");
    for (index, flag) in ["--max-seconds", "--generations", "--threads", "--train-out"]
        .iter()
        .enumerate()
    {
        let out = fixture.root.join(format!("out-{index}"));
        let result = native()
            .arg("--train-campaign")
            .arg(&fixture.config)
            .arg("--train-out")
            .arg(&out)
            .arg(flag)
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "{flag} without a value must not silently start default training"
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("requires a value"));
        assert!(!out.exists());
    }
    let out = fixture.root.join("empty-budget");
    let result = native()
        .arg("--train-campaign")
        .arg(&fixture.config)
        .arg("--train-out")
        .arg(&out)
        .args(["--max-seconds", ""])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!out.exists());
}

#[test]
fn native_resume_matches_uninterrupted_optimizer_history_champions_and_restarts() {
    let fixture = Fixture::new("resume");
    let uninterrupted = fixture.root.join("uninterrupted");
    let resumed = fixture.root.join("resumed");
    assert_eq!(
        success(fixture.campaign(&uninterrupted, 3, false, false))["status"],
        "completed"
    );
    let first = success(fixture.campaign(&resumed, 1, false, false));
    assert_eq!(first["completed_generations"], 1);
    let first_state = read_json(&resumed.join("state.json"));
    assert_eq!(first_state["completed_generations"], 1);
    assert_eq!(
        success(fixture.campaign(&resumed, 3, true, false))["status"],
        "completed"
    );
    let full = read_json(&uninterrupted.join("state.json"));
    let continued = read_json(&resumed.join("state.json"));
    assert_same_json(
        &continued,
        &full,
        "exact resumed state including moments, history, brains and restarts",
    );
    assert_eq!(full["history"].as_array().unwrap().len(), 3);
    let islands = full["tracks"][0]["islands"].as_array().unwrap();
    assert_eq!(islands[0]["restarts"], 0);
    assert!(
        islands[1]["restarts"].as_u64().unwrap() >= 1,
        "wide island must exercise actual module restart"
    );
    for generation in full["history"].as_array().unwrap() {
        for island in generation["tracks"][0]["islands"].as_array().unwrap() {
            let candidates = island["candidate_stage_fitness"].as_array().unwrap();
            assert_eq!(candidates.len(), 2);
            assert!(
                candidates
                    .iter()
                    .all(|scores| scores.as_array().unwrap().len() == 3)
            );
        }
    }
    // Validation cadence must not be changed merely by ending an invocation.
    for island in full["history"][1]["tracks"][0]["islands"]
        .as_array()
        .unwrap()
    {
        assert!(island["center_validation"].is_null());
        assert!(island["sample_validation"].is_null());
    }
    for suffix in [
        "brain.best.json",
        "local/brain.latest.json",
        "wide/brain.latest.json",
    ] {
        assert_same_json(
            &read_json(&uninterrupted.join("tracks/tactical").join(suffix)),
            &read_json(&resumed.join("tracks/tactical").join(suffix)),
            suffix,
        );
    }
    assert_same_bytes(
        &uninterrupted.join("history.jsonl"),
        &resumed.join("history.jsonl"),
        "resumed history",
    );
}

#[test]
fn native_resume_refuses_changed_stage_input_without_advancing_checkpoint() {
    let fixture = Fixture::new("changed-input");
    let out = fixture.root.join("out");
    success(fixture.campaign(&out, 1, false, false));
    let before = fs::read(out.join("state.json")).unwrap();
    let mut stage = read_json(&fixture.stages[0]);
    stage["max_time"] = json!(2.0);
    write_json(&fixture.stages[0], &stage);
    let rejected = fixture.campaign(&out, 3, true, false);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("resume refused"));
    assert_eq!(fs::read(out.join("state.json")).unwrap(), before);
}

#[test]
fn native_external_workers_and_relocated_config_match_local_campaign() {
    external_worker_equivalence(false);
}

#[test]
fn structural_external_workers_preserve_partial_proposals_and_match_local_campaign() {
    external_worker_equivalence(true);
}

fn external_worker_equivalence(structural: bool) {
    let fixture = Fixture::new("workers");
    if structural {
        let mut config = read_json(&fixture.config);
        config["tracks"][0]["islands"][1]["restart"] = json!(false);
        config["tracks"][0]["islands"][1]["structural_search"] = json!({"every":1});
        write_json(&fixture.config, &config);
    }
    let local = fixture.root.join("local");
    let external = fixture.root.join("external");
    success(fixture.campaign(&local, 1, false, false));
    let relocated = fixture.root.join("relocated-campaign.json");
    fs::copy(&fixture.config, &relocated).unwrap();
    let mut worker_count = 0;
    let mut complete = false;
    for round in 0..80 {
        let result = success(fixture.campaign(&external, 1, round > 0, true));
        if result["status"] == "completed" {
            complete = true;
            break;
        }
        assert_eq!(result["status"], "waiting_for_workers");
        assert!(result["waiting_jobs"].as_u64().unwrap() > 0);
        let mut jobs: Vec<_> = fs::read_dir(external.join("work/jobs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .filter(|path| {
                !external
                    .join("work/results")
                    .join(path.file_name().unwrap())
                    .exists()
            })
            .collect();
        assert!(
            !jobs.is_empty(),
            "waiting coordinator must publish missing work"
        );
        jobs.sort();
        jobs.reverse(); // Completion order deliberately differs from candidate order.
        for job_path in jobs {
            let job = read_json(&job_path);
            let result_path = external
                .join("work/results")
                .join(format!("{}.json", job["id"].as_str().unwrap()));
            let mut command = native();
            if worker_count == 0 {
                let mut moved = job.clone();
                moved["campaign"] = json!(fixture.root.join("missing-original-campaign.json"));
                let moved_job = fixture.root.join("relocated-job.json");
                write_json(&moved_job, &moved);
                command
                    .arg("--campaign-worker")
                    .arg(moved_job)
                    .arg("--campaign-config")
                    .arg(&relocated);
            } else {
                command.arg("--campaign-worker").arg(&job_path);
            }
            let worker = success(command.arg("--job-out").arg(&result_path).output().unwrap());
            assert_eq!(worker["job_id"], job["id"]);
            worker_count += 1;
        }
    }
    assert!(
        complete,
        "worker campaign did not converge after bounded stage barriers"
    );
    assert!(worker_count >= 3, "must evaluate all curriculum stages");
    assert_same_json(
        &read_json(&external.join("state.json")),
        &read_json(&local.join("state.json")),
        "external versus local state",
    );
    assert_same_bytes(
        &external.join("history.jsonl"),
        &local.join("history.jsonl"),
        "external versus local history",
    );
}

#[test]
fn native_resume_refuses_tampered_optimizer_settings() {
    let fixture = Fixture::new("tampered-state");
    let out = fixture.root.join("out");
    success(fixture.campaign(&out, 1, false, false));
    let checkpoint = out.join("state.json");
    let mut state = read_json(&checkpoint);
    let input_identity = state["input_identity"].clone();
    state["tracks"][0]["islands"][0]["es"]["config"]["sigma"] = json!(0.08);
    assert_eq!(state["input_identity"], input_identity);
    write_json(&checkpoint, &state);
    let before = fs::read(&checkpoint).unwrap();
    let rejected = fixture.campaign(&out, 3, true, false);
    assert!(
        !rejected.status.success(),
        "valid but altered saved optimizer settings must not be silently accepted"
    );
    assert_eq!(
        fs::read(&checkpoint).unwrap(),
        before,
        "rejected resume must leave checkpoint intact"
    );
}

#[test]
fn native_extension_removes_previous_holdout_until_new_holdout_finishes() {
    let fixture = Fixture::new("holdout-extension");
    let mut config = read_json(&fixture.config);
    config["tracks"][0]["holdout"] = config["tracks"][0]["validation"].clone();
    write_json(&fixture.config, &config);
    let out = fixture.root.join("out");
    success(fixture.campaign(&out, 1, false, false));
    let export = out.join("tracks/tactical/holdout.json");
    assert!(
        export.exists(),
        "first completed run must export its measured holdout"
    );
    let mut reached_new_checkpoint = false;
    for _ in 0..40 {
        let report = success(fixture.campaign(&out, 2, true, true));
        let state = read_json(&out.join("state.json"));
        if state["completed_generations"] == 2 {
            assert_eq!(report["status"], "waiting_for_workers");
            assert!(state["tracks"][0]["holdout"].is_null());
            assert!(
                !export.exists(),
                "an old holdout must not appear current while new holdout jobs are unfinished"
            );
            fs::write(out.join("STOP"), "stop before holdout").unwrap();
            assert_eq!(
                success(fixture.campaign(&out, 2, true, false))["status"],
                "stopped"
            );
            assert!(
                !export.exists(),
                "STOP also applies after final generation, before holdout"
            );
            fs::remove_file(out.join("STOP")).unwrap();
            assert_eq!(
                success(fixture.campaign(&out, 2, true, false))["status"],
                "completed"
            );
            assert!(export.exists());
            reached_new_checkpoint = true;
            break;
        }
        assert_eq!(report["status"], "waiting_for_workers");
        let mut count = 0;
        for entry in fs::read_dir(out.join("work/jobs")).unwrap() {
            let job = entry.unwrap().path();
            if job.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let result = out.join("work/results").join(job.file_name().unwrap());
            if result.exists() {
                continue;
            }
            success(
                native()
                    .arg("--campaign-worker")
                    .arg(job)
                    .arg("--job-out")
                    .arg(result)
                    .output()
                    .unwrap(),
            );
            count += 1;
        }
        assert!(
            count > 0,
            "waiting extension must publish work before its next checkpoint"
        );
    }
    assert!(
        reached_new_checkpoint,
        "extension never reached second generation holdout barrier"
    );
}

#[test]
fn time_budget_applies_before_initial_validation_and_resume_is_exact() {
    let fixture = Fixture::new("initial-budget");
    let out = fixture.root.join("out");
    let stopped = success(
        native()
            .arg("--train-campaign")
            .arg(&fixture.config)
            .arg("--train-out")
            .arg(&out)
            .args(["--max-seconds", "0.000000001", "--threads", "1"])
            .output()
            .unwrap(),
    );
    assert_eq!(stopped["status"], "stopped");
    assert_eq!(stopped["completed_generations"], 0);
    let state = read_json(&out.join("state.json"));
    assert!(
        state["tracks"][0]["initial_report"].is_null(),
        "budget must cover initialization"
    );
    let progress = read_json(&out.join("progress.json"));
    assert_eq!(progress["status"], "stopped");
    assert_eq!(progress["stop_reason"], "max_seconds");
    assert!(
        !out.join("work/jobs").exists(),
        "expired invocation must not start jobs"
    );
    assert_eq!(
        success(fixture.campaign(&out, 1, true, false))["status"],
        "completed"
    );
    let full = fixture.root.join("full");
    success(fixture.campaign(&full, 1, false, false));
    assert_same_json(
        &read_json(&out.join("state.json")),
        &read_json(&full.join("state.json")),
        "budget resume",
    );
}

#[test]
fn stop_during_initialization_preserves_cached_work_and_reports_progress() {
    let fixture = Fixture::new("initial-stop");
    let out = fixture.root.join("out");
    assert_eq!(
        success(fixture.campaign(&out, 1, false, true))["status"],
        "waiting_for_workers"
    );
    let before = fs::read(out.join("state.json")).unwrap();
    let progress = read_json(&out.join("progress.json"));
    assert_eq!(progress["plan"], "validation");
    assert_eq!(progress["stage"], "easy");
    assert_eq!(progress["jobs_waiting"], 1);
    let job = fs::read_dir(out.join("work/jobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let result = out.join("work/results").join(job.file_name().unwrap());
    success(
        native()
            .arg("--campaign-worker")
            .arg(&job)
            .arg("--job-out")
            .arg(&result)
            .output()
            .unwrap(),
    );
    let cached = fs::read(&result).unwrap();
    fs::write(out.join("STOP"), "stop").unwrap();
    assert_eq!(
        success(fixture.campaign(&out, 1, true, false))["status"],
        "stopped"
    );
    assert_eq!(fs::read(out.join("state.json")).unwrap(), before);
    assert_eq!(fs::read(&result).unwrap(), cached);
    let progress = read_json(&out.join("progress.json"));
    assert_eq!(progress["status"], "stopped");
    assert_eq!(progress["stop_reason"], "STOP");
    fs::remove_file(out.join("STOP")).unwrap();
    assert_eq!(
        success(fixture.campaign(&out, 1, true, false))["status"],
        "completed"
    );
    assert_eq!(read_json(&out.join("progress.json"))["status"], "completed");
    let full = fixture.root.join("full");
    success(fixture.campaign(&full, 1, false, false));
    assert_same_json(
        &read_json(&out.join("state.json")),
        &read_json(&full.join("state.json")),
        "STOP cached resume",
    );
}

#[test]
fn fresh_paired_results_gate_fixed_validation_winners_on_external_workers() {
    for accept in [false, true] {
        let fixture = Fixture::new("promotion-gate");
        let mut config = read_json(&fixture.config);
        config["min_gain"] = json!(0.0);
        config["promotion_seeds"] = json!(2);
        // One island keeps the deliberately controlled worker fixture small.
        config["tracks"][0]["islands"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        write_json(&fixture.config, &config);
        let out = fixture.root.join("out");
        let mut promotion_jobs = Vec::new();
        let mut completed = false;
        for round in 0..80 {
            let summary = success(fixture.campaign(&out, 1, round > 0, true));
            if summary["status"] == "completed" {
                completed = true;
                break;
            }
            assert_eq!(summary["status"], "waiting_for_workers");
            let jobs: Vec<_> = fs::read_dir(out.join("work/jobs"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|job| {
                    !out.join("work/results")
                        .join(job.file_name().unwrap())
                        .exists()
                })
                .collect();
            assert!(!jobs.is_empty());
            for path in jobs {
                let job = read_json(&path);
                let result_path = out.join("work/results").join(path.file_name().unwrap());
                success(
                    native()
                        .arg("--campaign-worker")
                        .arg(&path)
                        .arg("--job-out")
                        .arg(&result_path)
                        .output()
                        .unwrap(),
                );
                let mut result = read_json(&result_path);
                if job["plan"] == "validation"
                    && job["label"].as_str().unwrap().starts_with("generation-")
                {
                    // Controlled trusted-worker scores: a clear fixed-suite winner.
                    result["report"]["fitness"] = json!(100.0);
                    write_json(&result_path, &result);
                } else if job["plan"] == "promotion" {
                    // Exercise both acceptance and rejection after the fixed-suite gain.
                    result["report"]["fitness"] =
                        json!(if job["label"].as_str().unwrap().contains("/incumbent/") {
                            1.0
                        } else {
                            if accept { 2.0 } else { 0.0 }
                        });
                    write_json(&result_path, &result);
                    assert!(job["archive"].as_array().unwrap().is_empty());
                    promotion_jobs.push(job);
                }
            }
        }
        assert!(completed);
        let state = read_json(&out.join("state.json"));
        let island = &state["history"][0]["tracks"][0]["islands"][0];
        assert_eq!(
            state["tracks"][0]["best_generation"],
            if accept { 1 } else { 0 }
        );
        assert_eq!(
            island["promoted"].as_array().unwrap().len(),
            usize::from(accept)
        );
        let challenges = island["promotion_challenges"].as_array().unwrap();
        assert_eq!(challenges.len(), if accept { 1 } else { 2 });
        for challenge in challenges {
            assert_eq!(challenge["report"]["accepted"], accept);
            assert_eq!(challenge["report"]["losses"], if accept { 0 } else { 2 });
            assert_eq!(challenge["report"]["seeds"], 2);
        }
        // A rejected center leaves the same incumbent for the sampled challenger;
        // six incumbent stage results must be reused instead of evaluated twice.
        assert_eq!(promotion_jobs.len(), if accept { 12 } else { 18 });
        for job in &promotion_jobs {
            let label = job["label"].as_str().unwrap();
            if label.contains("/candidate/") {
                let incumbent = promotion_jobs
                    .iter()
                    .find(|other| {
                        other["label"].as_str().unwrap().contains("/incumbent/")
                            && other["stage"] == job["stage"]
                            && other["seed_base"] == job["seed_base"]
                    })
                    .unwrap();
                assert_eq!(job["seed_base"], incumbent["seed_base"]);
                assert!(job["seed_base"].as_u64().unwrap() > (1u64 << 61));
            }
        }
    }
}

#[test]
fn promoted_sample_recentres_its_island_only_when_enabled_and_reports_t() {
    for recentre in [false, true] {
        let fixture = Fixture::new("recentre");
        let mut config = read_json(&fixture.config);
        config["min_gain"] = json!(0.0);
        config["promotion_seeds"] = json!(3);
        config["promotion_min_t"] = json!(2.0);
        config["recentre_on_promotion"] = json!(recentre);
        config["tracks"][0]["islands"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        write_json(&fixture.config, &config);
        let out = fixture.root.join("out");
        let mut completed = false;
        for round in 0..80 {
            let summary = success(fixture.campaign(&out, 1, round > 0, true));
            if summary["status"] == "completed" {
                completed = true;
                break;
            }
            assert_eq!(summary["status"], "waiting_for_workers");
            let jobs: Vec<_> = fs::read_dir(out.join("work/jobs"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|job| {
                    !out.join("work/results")
                        .join(job.file_name().unwrap())
                        .exists()
                })
                .collect();
            assert!(!jobs.is_empty());
            for path in jobs {
                let job = read_json(&path);
                let result_path = out.join("work/results").join(path.file_name().unwrap());
                success(
                    native()
                        .arg("--campaign-worker")
                        .arg(&path)
                        .arg("--job-out")
                        .arg(&result_path)
                        .output()
                        .unwrap(),
                );
                let mut result = read_json(&result_path);
                let label = job["label"].as_str().unwrap();
                // Trusted-worker scores: only the sampled candidate beats the
                // incumbent, by a steady margin on every fresh pair.
                if job["plan"] == "validation" && label.starts_with("generation-") {
                    result["report"]["fitness"] = json!(if label.contains("/sample") {
                        100.0
                    } else {
                        0.0
                    });
                    write_json(&result_path, &result);
                } else if job["plan"] == "promotion" {
                    result["report"]["fitness"] = json!(if label.contains("/incumbent/") {
                        1.0
                    } else {
                        2.0
                    });
                    write_json(&result_path, &result);
                }
            }
        }
        assert!(completed);
        let state = read_json(&out.join("state.json"));
        let island = &state["history"][0]["tracks"][0]["islands"][0];
        assert_eq!(island["promoted"], json!(["sample"]));
        assert_eq!(state["tracks"][0]["best_generation"], 1);
        let challenge = &island["promotion_challenges"][0]["report"];
        assert_eq!(challenge["accepted"], true);
        assert!(challenge["t_statistic"].as_f64().unwrap() >= 2.0);
        let saved = &state["tracks"][0]["islands"][0]["es"];
        let moments_cleared = saved["first_moment"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m.as_f64() == Some(0.0));
        assert_eq!(moments_cleared, recentre, "recentre={recentre}");
        // The generation count survives so later noise streams stay fresh.
        assert_eq!(saved["generation"], 1);
    }
}

#[test]
fn stop_preserves_partial_generation_results_and_resume_matches_full_run() {
    let fixture = Fixture::new("partial-stop");
    let out = fixture.root.join("out");
    success(fixture.campaign(&out, 1, false, false));
    assert_eq!(
        success(fixture.campaign(&out, 2, true, true))["status"],
        "waiting_for_workers"
    );
    let progress = read_json(&out.join("progress.json"));
    assert_eq!(progress["plan"], "training");
    assert_eq!(progress["jobs_total"], 2);
    let job_path = fs::read_dir(out.join("work/jobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let result_path = out.join("work/results").join(job_path.file_name().unwrap());
    success(
        native()
            .arg("--campaign-worker")
            .arg(job_path)
            .arg("--job-out")
            .arg(&result_path)
            .output()
            .unwrap(),
    );
    let checkpoint = fs::read(out.join("state.json")).unwrap();
    let cached = fs::read(&result_path).unwrap();
    fs::write(out.join("STOP"), "stop").unwrap();
    assert_eq!(
        success(fixture.campaign(&out, 2, true, false))["status"],
        "stopped"
    );
    assert_eq!(fs::read(out.join("state.json")).unwrap(), checkpoint);
    assert_eq!(fs::read(&result_path).unwrap(), cached);
    fs::remove_file(out.join("STOP")).unwrap();
    success(fixture.campaign(&out, 2, true, false));
    let full = fixture.root.join("full");
    success(fixture.campaign(&full, 2, false, false));
    assert_same_json(
        &read_json(&out.join("state.json")),
        &read_json(&full.join("state.json")),
        "partial-generation resume",
    );
    assert_eq!(fs::read_dir(out.join("work/jobs")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(out.join("work/results")).unwrap().count(), 0);
}

#[test]
fn quality_state_matches_resumed_and_external_work_with_complete_facets() {
    let fixture = Fixture::new("quality-equivalence");
    let mut cfg = read_json(&fixture.config);
    cfg["frontier_limit"] = json!(8);
    cfg["max_faction_regression"] = json!(0.05);
    cfg["validation_every"] = json!(1);
    cfg["tracks"][0]["reference_brain"] = cfg["tracks"][0]["brain"].clone();
    for purpose in ["training", "validation"] {
        cfg["tracks"][0][purpose]["layouts"] = json!(2);
    }
    write_json(&fixture.config, &cfg);
    // An enabled guard must reject even one unlabelled scenario before creating state.
    let rejected = fixture.campaign(&fixture.root.join("missing-faction"), 1, false, false);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("every scenario"));
    for path in &fixture.stages {
        let mut stage = read_json(path);
        stage["scenarios"][0]["faction"] = json!("armada");
        write_json(path, &stage);
    }
    let local = fixture.root.join("quality-local");
    let resumed = fixture.root.join("quality-resume");
    let external = fixture.root.join("quality-external");
    success(fixture.campaign(&local, 2, false, false));
    success(fixture.campaign(&resumed, 1, false, false));
    fs::write(resumed.join("STOP"), "").unwrap();
    let stopped = success(fixture.campaign(&resumed, 2, true, false));
    assert_eq!(stopped["completed_generations"], 1);
    fs::remove_file(resumed.join("STOP")).unwrap();
    success(fixture.campaign(&resumed, 2, true, false));
    assert_same_json(
        &read_json(&local.join("state.json")),
        &read_json(&resumed.join("state.json")),
        "quality stop/resume",
    );
    let mut complete = false;
    for round in 0..150 {
        let status = success(fixture.campaign(&external, 2, round > 0, true));
        if status["status"] == "completed" {
            complete = true;
            break;
        }
        assert_eq!(status["status"], "waiting_for_workers");
        let mut jobs: Vec<_> = fs::read_dir(external.join("work/jobs"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "json")
                    && !external
                        .join("work/results")
                        .join(p.file_name().unwrap())
                        .exists()
            })
            .collect();
        jobs.sort();
        jobs.reverse();
        for job in jobs {
            let target = external.join("work/results").join(job.file_name().unwrap());
            success(
                native()
                    .arg("--campaign-worker")
                    .arg(job)
                    .arg("--job-out")
                    .arg(target)
                    .output()
                    .unwrap(),
            );
        }
    }
    assert!(complete);
    let state = read_json(&local.join("state.json"));
    assert!(state["tracks"][0]["reference_report"].is_object());
    assert_eq!(state["tracks"][0]["frontier"]["version"], 1);
    assert_same_json(
        &state,
        &read_json(&external.join("state.json")),
        "quality workers",
    );
    let entries = state["tracks"][0]["frontier"]["entries"]
        .as_array()
        .unwrap();
    assert!(!entries.is_empty());
    let hash = entries[0]["sha256"].as_str().unwrap();
    let brain_file = local
        .join("frontier/tactical")
        .join(format!("{hash}.brain.json"));
    let report_file = local
        .join("frontier/tactical")
        .join(format!("{hash}.report.json"));
    fs::remove_file(&brain_file).unwrap();
    fs::remove_file(&report_file).unwrap();
    success(fixture.campaign(&local, 2, true, false));
    assert_eq!(read_json(&brain_file), entries[0]["brain"]);
    assert_eq!(read_json(&report_file), entries[0]["report"]);
    write_json(&report_file, &json!({"tampered":true}));
    let tampered = fixture.campaign(&local, 2, true, false);
    assert!(!tampered.status.success());
    assert!(String::from_utf8_lossy(&tampered.stderr).contains("immutable frontier"));
    for entry in state["tracks"][0]["frontier"]["entries"]
        .as_array()
        .unwrap()
    {
        assert_ne!(entry["brain"], state["tracks"][0]["best"]);
        assert_ne!(entry["brain"], state["tracks"][0]["reference"]);
    }
}
