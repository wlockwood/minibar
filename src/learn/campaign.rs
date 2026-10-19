//! Resumable staged evolution, with the same work protocol locally and on workers.
use super::{
    brain::Brain,
    campaign_eval::{self, PlanReport, PreparedPlan, StageReport},
    campaign_io::{self, CampaignConfig},
    campaign_promotion::{self, PromotionReport},
    campaign_work::{self, EvaluationJob, EvaluationResult},
    es::{EsState, EvolutionStrategy, GenerationStats},
    rng::Rng,
};
use rayon::prelude::*;
#[path = "campaign_console.rs"]
mod console;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
pub struct CampaignOptions {
    pub resume: bool,
    pub external: bool,
    pub generations: Option<u64>,
    pub max_seconds: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct IslandState {
    name: String,
    es: EsState,
    last_progress: u64,
    restarts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<Value>,
    #[serde(default)]
    structural_attempts: u64,
    #[serde(default)]
    last_structural_generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    champion_follows: Vec<ChampionFollow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChampionFollow {
    generation: u64,
    brain_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrackState {
    #[serde(default)]
    structure_snapshots: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    structural_history: Vec<StructuralAttempt>,
    #[serde(default)]
    promotion_history: Vec<BrainPromotion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference_report: Option<PlanReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frontier: Option<super::campaign_frontier::FrontierState>,
    name: String,
    base: Value,
    best: Value,
    best_report: Option<PlanReport>,
    initial_report: Option<PlanReport>,
    best_generation: u64,
    archive: Vec<Value>,
    islands: Vec<IslandState>,
    holdout: Option<PlanReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StructuralAttempt {
    generation: u64,
    island: String,
    attempt: u64,
    parent_sha256: String,
    proposal_sha256: String,
    edit: super::structural_search::StructuralEdit,
    outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_generation: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrainPromotion {
    generation: u64,
    island: String,
    candidate: String,
    parent_sha256: String,
    brain_sha256: String,
}

fn snapshot(snapshots: &mut std::collections::BTreeMap<String, Value>, value: Value) -> String {
    let hash = campaign_io::digest(&serde_json::to_vec(&value).expect("brain serializes"));
    snapshots.entry(hash.clone()).or_insert(value);
    hash
}

fn structural_seed(seed: u64, track: usize, island: usize, generation: u64) -> u64 {
    Rng::for_stream(
        seed ^ (track as u64).rotate_left(17) ^ (island as u64).rotate_left(41),
        generation,
    )
    .next_u64()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CampaignState {
    format: String,
    version: u32,
    input_identity: String,
    backend_identity: String,
    completed_generations: u64,
    tracks: Vec<TrackState>,
    history: Vec<GenerationRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GenerationRecord {
    generation: u64,
    tracks: Vec<TrackRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrackRecord {
    name: String,
    islands: Vec<IslandRecord>,
    champion_fitness: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagnostic_challengers: Option<usize>,
    best_generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct IslandRecord {
    name: String,
    restarted_module: Option<String>,
    optimizer: GenerationStats,
    candidate_fitness: Vec<f64>,
    candidate_stage_fitness: Vec<Vec<f64>>,
    stage_diagnostics: Vec<StageDiagnostics>,
    center_validation: Option<PlanReport>,
    sample_validation: Option<PlanReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    module_validation: Vec<Value>,
    #[serde(default)]
    promotion_challenges: Vec<PromotionChallenge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    decisions: Vec<CandidateDecision>,
    promoted: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CandidateDecision {
    candidate: String,
    reason: String,
    screen_gain: f64,
    screen_min_gain: f64,
    promotion_min_gain: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_candidate: Option<PlanReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_incumbent: Option<PlanReport>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PromotionChallenge {
    candidate: String,
    report: PromotionReport,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StageDiagnostics {
    name: String,
    mean: f64,
    std_dev: f64,
    minimum: f64,
    maximum: f64,
    unique_fitness: usize,
}
fn stage_diagnostics(reports: &[PlanReport]) -> Vec<StageDiagnostics> {
    (0..reports[0].stages.len())
        .map(|stage| {
            let mut values: Vec<_> = reports.iter().map(|r| r.stages[stage].fitness).collect();
            values.sort_by(f64::total_cmp);
            let minimum = values[0];
            let maximum = *values.last().unwrap();
            let scale = minimum.abs().max(maximum.abs());
            let count = values.len() as f64;
            let mean_scaled = if scale == 0.0 {
                0.0
            } else {
                values
                    .iter()
                    .map(|v| v / scale / count)
                    .sum::<f64>()
                    .clamp(-1.0, 1.0)
            };
            let variance = if scale == 0.0 {
                0.0
            } else {
                (values
                    .iter()
                    .map(|v| (v / scale).powi(2) / count)
                    .sum::<f64>()
                    - mean_scaled.powi(2))
                .clamp(0.0, 1.0)
            };
            values.dedup();
            StageDiagnostics {
                name: reports[0].stages[stage].name.clone(),
                mean: mean_scaled * scale,
                std_dev: variance.sqrt() * scale,
                minimum,
                maximum,
                unique_fitness: values.len(),
            }
        })
        .collect()
}
#[derive(Serialize)]
pub struct CampaignSummary {
    pub status: String,
    pub completed_generations: u64,
    pub target_generations: u64,
    pub waiting_jobs: usize,
    pub tracks: Vec<Value>,
}
struct TrackRuntime {
    reference: Option<Brain>,
    base: Brain,
    mask: Vec<bool>,
    training: PreparedPlan,
    validation: PreparedPlan,
    promotion: PreparedPlan,
    holdout: Option<PreparedPlan>,
}

#[derive(Clone, Default, Serialize)]
struct Progress {
    status: String,
    track: Option<String>,
    plan: Option<String>,
    label: Option<String>,
    stage: Option<String>,
    stage_index: Option<usize>,
    jobs_total: usize,
    jobs_completed: usize,
    jobs_cached: usize,
    jobs_waiting: usize,
    elapsed_seconds: f64,
    stop_reason: Option<String>,
}

/// Coordinator calls this only after a complete stage barrier; validation is
/// independent of the requested stopping generation, allowing exact extension.
fn validation_due(generation: u64, cadence: u64) -> bool {
    generation == 1 || generation % cadence == 0
}
fn stages_safe(candidate: &PlanReport, baseline: &PlanReport, slack: f64) -> bool {
    campaign_eval::families_safe(candidate, baseline, slack)
        && candidate.stages.len() == baseline.stages.len()
        && candidate
            .stages
            .iter()
            .zip(&baseline.stages)
            .all(|(a, b)| a.name == b.name && a.fitness >= b.fitness - slack)
}

fn promotes(
    candidate: &PlanReport,
    champion: &PlanReport,
    initial: &PlanReport,
    gain: f64,
    slack: f64,
) -> bool {
    campaign_eval::families_safe(candidate, champion, slack)
        && campaign_eval::families_safe(candidate, initial, slack)
        && candidate.fitness > champion.fitness + gain
        && candidate.stages.len() == champion.stages.len()
        && candidate.stages.len() == initial.stages.len()
        && candidate
            .stages
            .iter()
            .zip(&champion.stages)
            .zip(&initial.stages)
            .all(|((c, b), i)| {
                c.name == b.name
                    && c.name == i.name
                    && c.fitness >= b.fitness - slack
                    && c.fitness >= i.fitness - slack
            })
}

struct WorkQueue<'a> {
    campaign: &'a Path,
    out: &'a Path,
    input: &'a str,
    backend: &'a str,
    external: bool,
    pending: AtomicUsize,
    used: Mutex<Vec<(PathBuf, PathBuf)>>,
    started: Instant,
    max_seconds: Option<f64>,
    stop_reason: Mutex<Option<&'static str>>,
    progress: Mutex<Progress>,
    console: &'a console::Reporter,
}
impl WorkQueue<'_> {
    fn should_stop(&self) -> Result<bool, String> {
        let mut reason = self.stop_reason.lock().map_err(|_| "stop lock poisoned")?;
        if reason.is_none() {
            *reason = if self.out.join("STOP").exists() {
                Some("STOP")
            } else if self
                .max_seconds
                .is_some_and(|limit| self.started.elapsed().as_secs_f64() >= limit)
            {
                Some("max_seconds")
            } else {
                None
            };
        }
        Ok(reason.is_some())
    }
    fn progress(&self, update: impl FnOnce(&mut Progress)) -> Result<(), String> {
        let mut progress = self.progress.lock().map_err(|_| "progress lock poisoned")?;
        update(&mut progress);
        progress.elapsed_seconds = self.started.elapsed().as_secs_f64();
        self.console.update(&progress);
        campaign_io::atomic_json(&self.out.join("progress.json"), &*progress)
    }
    fn finish(
        &self,
        state: &CampaignState,
        target: u64,
        status: &str,
    ) -> Result<CampaignSummary, String> {
        let reason = *self.stop_reason.lock().map_err(|_| "stop lock poisoned")?;
        self.console.checkpoint(state, target);
        self.progress(|p| {
            p.status = status.into();
            p.stop_reason = reason.map(str::to_owned);
        })?;
        finish(
            self.out,
            state,
            target,
            status,
            self.pending.load(Ordering::Relaxed),
        )
    }
    fn incomplete(&self, state: &CampaignState, target: u64) -> Result<CampaignSummary, String> {
        self.finish(
            state,
            target,
            if self.should_stop()? {
                "stopped"
            } else {
                "waiting_for_workers"
            },
        )
    }
    fn stage(
        &self,
        prepared: &PreparedPlan,
        track: &str,
        plan: &str,
        stage: usize,
        seed_base: u64,
        label: &str,
        brains: &[Brain],
        archive: &[Value],
    ) -> Result<Option<Vec<StageReport>>, String> {
        if self.should_stop()? {
            return Ok(None);
        }
        self.progress(|p| {
            *p = Progress {
                status: "running".into(),
                track: Some(track.into()),
                plan: Some(plan.into()),
                label: Some(label.into()),
                stage: Some(prepared.plan().stages[stage].name.clone()),
                stage_index: Some(stage),
                jobs_total: brains.len(),
                ..Progress::default()
            };
        })?;
        let results: Vec<Result<Option<StageReport>, String>> = brains
            .par_iter()
            .enumerate()
            .map(|(index, brain)| {
                if self.should_stop()? {
                    return Ok(None);
                }
                let mut job = EvaluationJob {
                    format: "minibar-evaluation-job".into(),
                    version: 1,
                    id: String::new(),
                    campaign: self.campaign.to_owned(),
                    input_identity: self.input.into(),
                    backend_identity: self.backend.into(),
                    label: format!("{label}/candidate-{index}"),
                    track: track.into(),
                    plan: plan.into(),
                    stage,
                    seed_base,
                    brain: campaign_work::brain_value(brain),
                    archive: archive.to_vec(),
                };
                job.seal()?;
                let job_file = self.out.join("work/jobs").join(format!("{}.json", job.id));
                let result_file = self
                    .out
                    .join("work/results")
                    .join(format!("{}.json", job.id));
                self.used
                    .lock()
                    .map_err(|_| "work ledger poisoned")?
                    .push((job_file.clone(), result_file.clone()));
                if job_file.exists() {
                    let old: EvaluationJob = campaign_io::read_json(&job_file)?;
                    old.validate()?;
                    if old.id != job.id {
                        return Err("stale job file".into());
                    }
                } else {
                    campaign_io::atomic_json(&job_file, &job)?;
                }
                if result_file.exists() {
                    let result: EvaluationResult = campaign_io::read_json(&result_file)?;
                    campaign_work::validate_result(&job, prepared.plan(), &result)?;
                    self.progress(|p| {
                        p.jobs_completed += 1;
                        p.jobs_cached += 1;
                    })?;
                    return Ok(Some(result.report));
                }
                if self.external {
                    self.pending.fetch_add(1, Ordering::Relaxed);
                    self.progress(|p| p.jobs_waiting += 1)?;
                    return Ok(None);
                }
                // Preparing a large work payload and its files can consume the
                // remaining budget; check again immediately before simulation.
                if self.should_stop()? {
                    return Ok(None);
                }
                let result = campaign_work::execute_prepared(&job, prepared)?;
                campaign_io::atomic_json(&result_file, &result)?;
                self.progress(|p| p.jobs_completed += 1)?;
                Ok(Some(result.report))
            })
            .collect();
        let mut complete = Vec::new();
        let mut waiting = false;
        for result in results {
            match result? {
                Some(report) => complete.push(report),
                None => waiting = true,
            }
        }
        if waiting {
            Ok(None)
        } else {
            Ok(Some(complete))
        }
    }
    fn plan(
        &self,
        prepared: &PreparedPlan,
        track: &str,
        purpose: &str,
        seed: u64,
        label: &str,
        brain: &Brain,
    ) -> Result<Option<PlanReport>, String> {
        let mut reports = Vec::new();
        for stage in 0..prepared.plan().stages.len() {
            let Some(mut batch) = self.stage(
                prepared,
                track,
                purpose,
                stage,
                campaign_eval::stage_seed(seed, stage)?,
                &format!("{label}/stage-{stage}"),
                std::slice::from_ref(brain),
                &[],
            )?
            else {
                return Ok(None);
            };
            reports.push(batch.remove(0));
        }
        Ok(Some(campaign_eval::aggregate(reports)?))
    }
    fn cleanup(&self) -> Result<(), String> {
        for (job, result) in self
            .used
            .lock()
            .map_err(|_| "work ledger poisoned")?
            .drain(..)
        {
            for file in [job, result] {
                if file.exists() {
                    fs::remove_file(&file).map_err(|e| format!("clean {}: {e}", file.display()))?;
                }
            }
        }
        Ok(())
    }
}

fn initial(
    config: &CampaignConfig,
    runtime: &[TrackRuntime],
    input: String,
    backend: String,
) -> Result<CampaignState, String> {
    let tracks = config
        .tracks
        .iter()
        .zip(runtime)
        .map(|(track, rt)| {
            let base = campaign_work::brain_value(&rt.base);
            let islands = track
                .islands
                .iter()
                .map(|island| {
                    Ok(IslandState {
                        name: island.name.clone(),
                        es: EvolutionStrategy::new(rt.base.flatten(&rt.mask), island.es)?.state(),
                        last_progress: 0,
                        restarts: 0,
                        base: None,
                        structural_attempts: 0,
                        last_structural_generation: 0,
                        champion_follows: vec![],
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(TrackState {
                structure_snapshots: {
                    let mut snapshots = Default::default();
                    snapshot(&mut snapshots, base.clone());
                    snapshots
                },
                structural_history: vec![],
                promotion_history: vec![],
                reference: rt.reference.as_ref().map(campaign_work::brain_value),
                reference_report: None,
                frontier: config.frontier_limit.map(|_| Default::default()),
                name: track.name.clone(),
                base: base.clone(),
                best: base.clone(),
                best_report: None,
                initial_report: None,
                best_generation: 0,
                archive: if config.archive_size > 0 {
                    vec![base]
                } else {
                    vec![]
                },
                islands,
                holdout: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CampaignState {
        format: "minibar-campaign-state".into(),
        version: 1,
        input_identity: input,
        backend_identity: backend,
        completed_generations: 0,
        tracks,
        history: vec![],
    })
}
fn validate_state(
    state: &CampaignState,
    config: &CampaignConfig,
    runtime: &[TrackRuntime],
    input: &str,
    backend: &str,
) -> Result<(), String> {
    if state.format != "minibar-campaign-state"
        || state.version != 1
        || state.input_identity != input
        || state.backend_identity != backend
    {
        return Err(
            "checkpoint format, campaign inputs or executable changed; resume refused".into(),
        );
    }
    if state.tracks.len() != config.tracks.len()
        || state.history.len() as u64 != state.completed_generations
    {
        return Err("checkpoint track/generation dimensions do not match".into());
    }
    if state
        .history
        .iter()
        .enumerate()
        .any(|(i, row)| row.generation != i as u64 + 1 || row.tracks.len() != config.tracks.len())
    {
        return Err("checkpoint history sequence is invalid".into());
    }
    for ((saved, track), rt) in state.tracks.iter().zip(&config.tracks).zip(runtime) {
        if saved.reference != rt.reference.as_ref().map(campaign_work::brain_value)
            || saved.frontier.is_some() != config.frontier_limit.is_some()
        {
            return Err("checkpoint reference/frontier differs".into());
        }
        if let Some(frontier) = &saved.frontier {
            frontier.validate()?;
            if frontier.entries.len() > config.frontier_limit.unwrap_or(0) {
                return Err("frontier exceeds configured limit".into());
            }
            for e in &frontier.entries {
                campaign_work::parse_brain(&e.brain)?
                    .check_against(
                        &rt.base.kind,
                        &rt.base
                            .modules
                            .iter()
                            .map(|m| m.spec.clone())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|e| e.to_string())?;
                validate_report(&e.report, rt.validation.plan())?;
            }
        }
        if let Some(report) = &saved.reference_report {
            validate_report(report, rt.validation.plan())?;
        }

        if saved.name != track.name
            || saved.islands.len() != track.islands.len()
            || campaign_work::parse_brain(&saved.base)? != rt.base
            || saved.best_generation > state.completed_generations
            || saved.archive.len() > config.archive_size
        {
            return Err("checkpoint track identity/shape is invalid".into());
        }
        if saved.best_report.is_some() != saved.initial_report.is_some()
            || (state.completed_generations > 0 && saved.best_report.is_none())
        {
            return Err("checkpoint validation state is incomplete".into());
        }
        for report in saved.best_report.iter().chain(&saved.initial_report) {
            validate_report(report, rt.validation.plan())?;
        }
        if let Some(report) = &saved.holdout {
            validate_report(
                report,
                rt.holdout
                    .as_ref()
                    .ok_or("unexpected saved holdout")?
                    .plan(),
            )?;
        }
        let specs: Vec<_> = rt.base.modules.iter().map(|m| m.spec.clone()).collect();
        for brain in std::iter::once(&saved.best).chain(&saved.archive) {
            let brain = campaign_work::parse_brain(brain)?;
            brain
                .check_against(&rt.base.kind, &specs)
                .map_err(|e| e.to_string())?;
            if brain.kind != rt.base.kind || brain.param_count() != rt.base.param_count() {
                return Err("checkpoint brain shape mismatch".into());
            }
        }
        for (hash, value) in &saved.structure_snapshots {
            if campaign_io::digest(&serde_json::to_vec(value).map_err(|e| e.to_string())?) != *hash
            {
                return Err("checkpoint structural snapshot hash mismatch".into());
            }
            campaign_work::parse_brain(value)?
                .check_against(&rt.base.kind, &specs)
                .map_err(|e| e.to_string())?;
        }
        for event in &saved.structural_history {
            if event.generation == 0
                || event.generation > state.completed_generations
                || !saved.structure_snapshots.contains_key(&event.parent_sha256)
                || !saved
                    .structure_snapshots
                    .contains_key(&event.proposal_sha256)
                || !matches!(
                    event.outcome.as_str(),
                    "training"
                        | "evaluated_without_promotion"
                        | "accepted"
                        | "superseded_without_evaluation"
                )
            {
                return Err("checkpoint structural history invalid".into());
            }
        }
        if !saved.structure_snapshots.is_empty() {
            let recorded: Vec<_> = state
                .history
                .iter()
                .flat_map(|g| {
                    g.tracks
                        .iter()
                        .filter(|r| r.name == track.name)
                        .flat_map(move |r| {
                            r.islands.iter().flat_map(move |i| {
                                i.promoted.iter().map(move |candidate| {
                                    (g.generation, i.name.as_str(), candidate.as_str())
                                })
                            })
                        })
                })
                .collect();
            if recorded.len() != saved.promotion_history.len() {
                return Err(
                    "checkpoint promotion history count differs from committed records".into(),
                );
            }
            let mut previous = &saved.base;
            for (promotion, (generation, island, candidate)) in
                saved.promotion_history.iter().zip(recorded)
            {
                let parent = saved
                    .structure_snapshots
                    .get(&promotion.parent_sha256)
                    .ok_or("promotion parent snapshot missing")?;
                let brain = saved
                    .structure_snapshots
                    .get(&promotion.brain_sha256)
                    .ok_or("promotion brain snapshot missing")?;
                if promotion.generation != generation
                    || promotion.island != island
                    || promotion.candidate != candidate
                    || parent != previous
                {
                    return Err(
                        "checkpoint promotion lineage differs from committed results".into(),
                    );
                }
                previous = brain;
            }
            if previous != &saved.best {
                return Err("checkpoint champion differs from promotion lineage".into());
            }
        } else if !saved.promotion_history.is_empty() {
            return Err("promotion history requires brain snapshots".into());
        }
        for (saved, island) in saved.islands.iter().zip(&track.islands) {
            if saved.name != island.name
                || saved.last_progress > state.completed_generations
                || saved.es.theta.len() != rt.base.trainable_len(&rt.mask)
            {
                return Err("checkpoint island shape mismatch".into());
            }
            let island_base = saved
                .base
                .as_ref()
                .map(campaign_work::parse_brain)
                .transpose()?
                .unwrap_or_else(|| rt.base.clone());
            island_base
                .check_against(&rt.base.kind, &specs)
                .map_err(|e| e.to_string())?;
            if island_base
                .modules
                .iter()
                .zip(&rt.base.modules)
                .any(|(a, b)| a.net.sizes() != b.net.sizes())
            {
                return Err("checkpoint structural layer sizes differ".into());
            }
            let inherited = island_base.flatten(&rt.mask);
            for (i, active) in island_base.parameter_activity(&rt.mask).iter().enumerate() {
                if !active
                    && (saved.es.theta[i].to_bits() != inherited[i].to_bits()
                        || saved.es.first_moment.get(i) != Some(&0.0)
                        || saved.es.second_moment.get(i) != Some(&0.0))
                {
                    return Err(
                        "checkpoint dormant parameters or moments differ from inherited structure"
                            .into(),
                    );
                }
            }
            EvolutionStrategy::from_state(saved.es.clone())?;
            let mut actual = saved.es.config;
            let mut expected = island.es;
            actual.seed = 0;
            expected.seed = 0;
            if let Some(search) = &island.structural_search {
                let attempts = if state.completed_generations == 0 {
                    0
                } else {
                    (state.completed_generations - 1) / search.every + 1
                };
                let start = if attempts == 0 {
                    0
                } else {
                    1 + (attempts - 1) * search.every
                };
                let expected_seed = if attempts == 0 {
                    island.es.seed
                } else {
                    structural_seed(
                        config.seed,
                        config
                            .tracks
                            .iter()
                            .position(|t| t.name == track.name)
                            .unwrap(),
                        track
                            .islands
                            .iter()
                            .position(|i| i.name == island.name)
                            .unwrap(),
                        start,
                    ) ^ 0x535452554354
                };
                if actual != expected
                    || saved.structural_attempts != attempts
                    || saved.last_structural_generation != start
                    || saved.es.generation
                        != state
                            .completed_generations
                            .saturating_sub(start)
                            .saturating_add(u64::from(attempts > 0))
                    || saved.es.config.seed != expected_seed
                    || saved.restarts != 0
                    || saved.base.is_some() != (attempts > 0)
                {
                    return Err("checkpoint structural optimizer counters/settings differ".into());
                }
                let events: Vec<_> = state
                    .tracks
                    .iter()
                    .find(|t| t.name == track.name)
                    .unwrap()
                    .structural_history
                    .iter()
                    .filter(|e| e.island == island.name)
                    .collect();
                if events.len() as u64 != attempts {
                    return Err("checkpoint structural history count differs".into());
                }
                for (attempt_index, event) in events.iter().enumerate() {
                    let generation = 1 + attempt_index as u64 * search.every;
                    if event.generation != generation || event.attempt != attempt_index as u64 + 1 {
                        return Err("checkpoint structural attempt sequence differs".into());
                    }
                    let track_state = state.tracks.iter().find(|t| t.name == track.name).unwrap();
                    let parent = campaign_work::parse_brain(
                        &track_state.structure_snapshots[&event.parent_sha256],
                    )?;
                    let seed = structural_seed(
                        config.seed,
                        config
                            .tracks
                            .iter()
                            .position(|t| t.name == track.name)
                            .unwrap(),
                        track
                            .islands
                            .iter()
                            .position(|i| i.name == island.name)
                            .unwrap(),
                        generation,
                    );
                    let (proposal, edit) =
                        super::structural_search::mutate(&parent, &rt.mask, search, seed)?;
                    if edit != event.edit
                        || campaign_work::brain_value(&proposal)
                            != track_state.structure_snapshots[&event.proposal_sha256]
                        || (event.attempt == attempts
                            && campaign_work::brain_value(&island_base)
                                != track_state.structure_snapshots[&event.proposal_sha256])
                    {
                        return Err(
                            "checkpoint structural proposal differs from recorded inheritance"
                                .into(),
                        );
                    }
                }
                continue;
            }
            let restarts: Vec<_> = state
                .history
                .iter()
                .filter(|g| {
                    g.tracks.iter().any(|t| {
                        t.name == track.name
                            && t.islands
                                .iter()
                                .any(|i| i.name == island.name && i.restarted_module.is_some())
                    })
                })
                .map(|g| g.generation)
                .collect();
            if restarts.len() as u64 != saved.restarts {
                return Err("checkpoint restart history differs".into());
            }
            let mut expected_base = &rt.base;
            let track_state = state.tracks.iter().find(|t| t.name == track.name).unwrap();
            let restart_base;
            if island.follow_champion {
                let mut follows = Vec::new();
                let mut previous = None;
                for generation in 1..=state.completed_generations {
                    let value = track_state
                        .promotion_history
                        .iter()
                        .rfind(|p| p.generation < generation)
                        .map(|p| &track_state.structure_snapshots[&p.brain_sha256])
                        .unwrap_or(&track_state.base);
                    let hash = campaign_io::digest(&serde_json::to_vec(value).unwrap());
                    let last_progress = follows
                        .last()
                        .map_or(0, |f: &ChampionFollow| f.generation)
                        .max(
                            track_state
                                .promotion_history
                                .iter()
                                .filter(|p| p.generation < generation)
                                .map(|p| p.generation)
                                .max()
                                .unwrap_or(0),
                        );
                    if follower_due(
                        previous.as_deref(),
                        &hash,
                        config.reanchor_after,
                        generation,
                        last_progress,
                    ) {
                        follows.push(ChampionFollow {
                            generation,
                            brain_hash: hash.clone(),
                        });
                        previous = Some(hash);
                    }
                }
                if saved.champion_follows != follows || saved.restarts != 0 {
                    return Err("checkpoint champion follower lineage differs".into());
                }
                if let Some(last) = follows.last() {
                    restart_base = campaign_work::parse_brain(
                        &track_state.structure_snapshots[&last.brain_hash],
                    )?;
                    expected_base = &restart_base;
                }
            } else if !saved.champion_follows.is_empty() {
                return Err("unexpected champion follower state".into());
            } else if saved.base.is_some() && saved.restarts > 0 {
                let before = *restarts.last().unwrap();
                let value = track_state
                    .promotion_history
                    .iter()
                    .rfind(|p| p.generation < before)
                    .map(|p| &track_state.structure_snapshots[&p.brain_sha256])
                    .unwrap_or(&track_state.base);
                restart_base = campaign_work::parse_brain(value)?;
                expected_base = &restart_base;
            }
            if island_base != *expected_base {
                return Err(
                    "checkpoint weight-only island structure differs from inherited champion"
                        .into(),
                );
            }
            if saved.structural_attempts != 0 || saved.last_structural_generation != 0 {
                return Err("unexpected structural search state".into());
            }
            if actual != expected
                || saved.restarts > state.completed_generations
                || (!island.restart && saved.restarts > 0)
                || saved.es.generation > state.completed_generations
                || (saved.restarts == 0
                    && (saved.es.generation != state.completed_generations
                        || saved.es.config.seed != island.es.seed))
                || (saved.restarts > 0
                    && (saved.es.generation == 0
                        || saved.last_progress
                            < state.completed_generations - saved.es.generation + 1))
            {
                return Err("checkpoint optimizer settings/counters differ from campaign".into());
            }
        }
    }
    Ok(())
}
fn validate_report(
    report: &PlanReport,
    plan: &campaign_eval::EvaluationPlan,
) -> Result<(), String> {
    if report.stages.len() != plan.stages.len()
        || !report.fitness.is_finite()
        || report.stages.iter().zip(&plan.stages).any(|(a, b)| {
            a.name != b.name
                || a.difficulty != b.difficulty
                || a.weight != b.weight
                || !a.fitness.is_finite()
        })
    {
        return Err("checkpoint stage report does not match its curriculum".into());
    }
    let aggregate = campaign_eval::aggregate(report.stages.clone())?;
    if (aggregate.fitness - report.fitness).abs() > 1e-10 * (1.0 + report.fitness.abs()) {
        return Err("checkpoint aggregate fitness is inconsistent".into());
    }
    Ok(())
}
fn frontier_exports(out: &Path, state: &CampaignState) -> Result<(), String> {
    for track in &state.tracks {
        if let Some(frontier) = &track.frontier {
            frontier.validate()?;
            for entry in &frontier.entries {
                let root = out.join("frontier").join(&track.name);
                let path = root.join(format!("{}.brain.json", entry.sha256));
                if path.exists() {
                    let old: Value = campaign_io::read_json(&path)?;
                    if old != entry.brain {
                        return Err("immutable frontier object differs".into());
                    }
                } else {
                    campaign_io::atomic_json(&path, &entry.brain)?;
                }
                let report_path = root.join(format!("{}.report.json", entry.sha256));
                let report = serde_json::to_value(&entry.report).map_err(|e| e.to_string())?;
                if report_path.exists() {
                    let old: Value = campaign_io::read_json(&report_path)?;
                    if old != report {
                        return Err("immutable frontier report differs".into());
                    }
                } else {
                    campaign_io::atomic_json(&report_path, &report)?;
                }
            }
        }
    }
    Ok(())
}
fn checkpoint(out: &Path, state: &CampaignState, runtime: &[TrackRuntime]) -> Result<(), String> {
    // Objects precede the authoritative state commit; exports are repaired on resume.
    frontier_exports(out, state)?;
    campaign_io::atomic_json(&out.join("state.json"), state)?;
    exports(out, state, runtime)
}
fn exports(out: &Path, state: &CampaignState, runtime: &[TrackRuntime]) -> Result<(), String> {
    frontier_exports(out, state)?;
    for (track, rt) in state.tracks.iter().zip(runtime) {
        let root = out.join("tracks").join(&track.name);
        if let Some(reference) = &track.reference {
            campaign_io::atomic_json(&root.join("brain.reference.json"), reference)?;
        }
        if let Some(frontier) = &track.frontier {
            campaign_io::atomic_json(&root.join("frontier.json"), frontier)?;
        }

        campaign_io::atomic_json(&root.join("brain.initial.json"), &track.base)?;
        campaign_io::atomic_json(&root.join("brain.best.json"), &track.best)?;
        if let Some(report) = &track.best_report {
            campaign_io::atomic_json(&root.join("validation.best.json"), report)?;
        }
        if let Some(report) = &track.holdout {
            campaign_io::atomic_json(&root.join("holdout.json"), report)?;
        } else if root.join("holdout.json").exists() {
            fs::remove_file(root.join("holdout.json")).map_err(|e| e.to_string())?;
        }
        for island in &track.islands {
            let base = island
                .base
                .as_ref()
                .map(campaign_work::parse_brain)
                .transpose()?
                .unwrap_or_else(|| rt.base.clone());
            let brain = base
                .with_flat(&rt.mask, &island.es.theta)
                .map_err(|e| e.to_string())?;
            campaign_io::atomic_json(
                &root.join(&island.name).join("brain.latest.json"),
                &campaign_work::brain_value(&brain),
            )?;
        }
    }
    let mut lines = String::new();
    for row in &state.history {
        lines.push_str(&serde_json::to_string(row).map_err(|e| e.to_string())?);
        lines.push('\n');
    }
    campaign_io::atomic_bytes(&out.join("history.jsonl"), lines.as_bytes())
}
fn finish(
    out: &Path,
    state: &CampaignState,
    target: u64,
    status: &str,
    waiting: usize,
) -> Result<CampaignSummary, String> {
    let result = summary(state, target, status, waiting);
    campaign_io::atomic_json(&out.join("summary.json"), &result)?;
    Ok(result)
}
fn summary(state: &CampaignState, target: u64, status: &str, waiting: usize) -> CampaignSummary {
    CampaignSummary {status:status.into(),completed_generations:state.completed_generations,target_generations:target,waiting_jobs:waiting,
        tracks:state.tracks.iter().map(|track| serde_json::json!({"name":track.name,"best_generation":track.best_generation,
            "diagnostic_challengers":track.frontier.as_ref().map_or(0,|f|f.entries.len()),"initial_fitness":track.initial_report.as_ref().map(|r|r.fitness),"best_fitness":track.best_report.as_ref().map(|r|r.fitness),
            "holdout_fitness":track.holdout.as_ref().map(|r|r.fitness),"restarts":track.islands.iter().map(|i|serde_json::json!({"name":i.name,"count":i.restarts})).collect::<Vec<_>>() })).collect() }
}

/// The island's optimizer restarted at `theta`: same configuration, seed and
/// generation count (so noise streams stay fresh), but zeroed Adam moments/clock,
/// which described a different centre.
fn recentred(state: &EsState, theta: Vec<f32>) -> Result<EsState, String> {
    if theta.len() != state.theta.len() {
        return Err("recentred parameters do not match the island's dimension".into());
    }
    let len = theta.len();
    let moved = EsState {
        theta,
        first_moment: vec![0.0; len],
        second_moment: vec![0.0; len],
        adam_updates: Some(0),
        ..state.clone()
    };
    // Reject a nonfinite centre before it is stored.
    EvolutionStrategy::from_state(moved.clone())?;
    Ok(moved)
}

fn follower_due(
    previous: Option<&str>,
    champion: &str,
    after: u64,
    generation: u64,
    last_progress: u64,
) -> bool {
    previous != Some(champion)
        || (after > 0 && generation.saturating_sub(1).saturating_sub(last_progress) >= after)
}

fn restart_island(
    rt: &TrackRuntime,
    island: &mut IslandState,
    champion: &Brain,
    scale: f32,
    seed: u64,
    generation: u64,
) -> Result<String, String> {
    let selected: Vec<_> = rt
        .base
        .modules
        .iter()
        .enumerate()
        .filter(|(i, _)| rt.mask[*i])
        .collect();
    let mut rng = Rng::for_stream(seed ^ generation, island.restarts);
    let pick = rng.next_u64() as usize % selected.len();
    let module = selected[pick].0;
    let mut theta = champion.flatten(&rt.mask);
    let active = champion.parameter_activity(&rt.mask);
    island.base = Some(campaign_work::brain_value(champion));
    let mut offset = 0;
    for (index, m) in rt.base.modules.iter().enumerate() {
        if !rt.mask[index] {
            continue;
        }
        let len = m.net.params().len();
        if index == module {
            for (i, p) in theta[offset..offset + len].iter_mut().enumerate() {
                let noise = rng.gaussian() * scale;
                if active[offset + i] {
                    *p = noise;
                }
            }
        }
        offset += len;
    }
    let mut es_config = island.es.config;
    es_config.seed = rng.next_u64();
    island.es = EvolutionStrategy::new(theta, es_config)?.state();
    island.restarts += 1;
    island.last_progress = generation;
    Ok(rt.base.modules[module].spec.name.clone())
}

/// Preserve score summaries and bounded examples in committed generation history.
fn compact_history_details(details: &mut Value) {
    fn collect(
        value: &Value,
        seen: &mut std::collections::BTreeSet<String>,
        sample_keys: &mut std::collections::BTreeSet<String>,
        counts: &mut std::collections::BTreeMap<String, u64>,
        examples: &mut Vec<Value>,
    ) {
        if let Some(records) = value.get("matchup_records").and_then(Value::as_array) {
            for record in records {
                let hash = super::content_hash::sha256(record.to_string().as_bytes());
                if !seen.insert(hash) {
                    continue;
                }
                let family = record
                    .get("family")
                    .and_then(Value::as_str)
                    .unwrap_or("unlabelled");
                *counts.entry(family.into()).or_default() += 1;
                let key = format!("{}:{}", record["seed"], record["index"]);
                if examples.len() < 2 && sample_keys.insert(key) {
                    examples.push(record.clone());
                }
            }
        }
        for key in ["baseline", "historical", "scripted", "learned", "report"] {
            if let Some(child) = value.get(key) {
                collect(child, seen, sample_keys, counts, examples);
            }
        }
        if let Some(layouts) = value.get("layouts").and_then(Value::as_array) {
            for layout in layouts {
                collect(layout, seen, sample_keys, counts, examples);
            }
        }
    }
    fn prune(value: &mut Value) {
        let Some(object) = value.as_object_mut() else {
            return;
        };
        object.retain(|key, _| {
            matches!(
                key.as_str(),
                "fitness"
                    | "robust_fitness"
                    | "fitness_score"
                    | "families"
                    | "facets"
                    | "facets_complete"
                    | "objectives"
                    | "layout_seed"
                    | "mean_layout_fitness"
                    | "total"
                    | "rows"
                    | "trials"
                    | "baseline"
                    | "historical"
                    | "scripted"
                    | "learned"
                    | "report"
                    | "baseline_fitness"
                    | "historical_fitness"
                    | "archive_index"
                    | "brain_kind"
                    | "companion_identity"
            )
        });
        for key in ["baseline", "historical", "scripted", "learned", "report"] {
            if let Some(child) = object.get_mut(key) {
                prune(child);
            }
        }
        if let Some(rows) = object.get_mut("rows").and_then(Value::as_array_mut) {
            for row in rows {
                if let Some(row) = row.as_object_mut() {
                    row.retain(|key, _| {
                        matches!(key.as_str(), "scenario" | "opponent" | "family" | "tally")
                    });
                }
            }
        }
    }
    let (counts, examples) = if let Some(counts) = details.get("coverage_counts") {
        (
            counts.clone(),
            details
                .get("matchup_records")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
    } else {
        let mut counts = std::collections::BTreeMap::new();
        let mut examples = Vec::new();
        collect(
            details,
            &mut Default::default(),
            &mut Default::default(),
            &mut counts,
            &mut examples,
        );
        (serde_json::json!(counts), serde_json::json!(examples))
    };
    prune(details);
    if counts.as_object().is_some_and(|counts| !counts.is_empty()) {
        details["coverage_counts"] = counts;
        details["matchup_records"] = examples;
        details["matchup_records_sampled"] = serde_json::json!(true);
    }
}

fn compact_history_report(report: &mut PlanReport) {
    for stage in &mut report.stages {
        compact_history_details(&mut stage.details);
    }
}

/// Receipts and queue results are already sealed before these history-only copies shrink.
fn compact_generation_reports(record: &mut TrackRecord) {
    for island in &mut record.islands {
        for report in [&mut island.center_validation, &mut island.sample_validation]
            .into_iter()
            .flatten()
        {
            compact_history_report(report);
        }
        for module in &mut island.module_validation {
            if let Some(stages) = module["report"]["stages"].as_array_mut() {
                for stage in stages {
                    compact_history_details(&mut stage["details"]);
                }
            }
        }
        for decision in &mut island.decisions {
            for report in [
                &mut decision.screen_candidate,
                &mut decision.screen_incumbent,
            ]
            .into_iter()
            .flatten()
            {
                compact_history_report(report);
            }
        }
        for challenge in &mut island.promotion_challenges {
            for report in challenge
                .report
                .candidate
                .iter_mut()
                .chain(&mut challenge.report.incumbent)
            {
                compact_history_report(report);
            }
        }
    }
}

/// Assemble a whole brain before evaluation; never promote an isolated module.
fn module_combination(
    champion: &Brain,
    donor: &Brain,
    trainable: &[bool],
    generation: u64,
    cadence: u64,
) -> Result<Option<(String, Brain)>, String> {
    if cadence == 0 || generation == 0 || !generation.is_multiple_of(cadence) {
        return Ok(None);
    }
    if champion.kind != donor.kind
        || champion.modules.len() != donor.modules.len()
        || trainable.len() != champion.modules.len()
    {
        return Err("module recombination needs matching brain kinds and module dimensions".into());
    }
    let selected: Vec<_> = trainable
        .iter()
        .enumerate()
        .filter_map(|(index, enabled)| enabled.then_some(index))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let index = selected[((generation / cadence - 1) % selected.len() as u64) as usize];
    if champion.modules[index].spec != donor.modules[index].spec {
        return Err(
            "module recombination donor does not match the champion module contract".into(),
        );
    }
    let mut assembled = champion.clone();
    assembled.modules[index] = donor.modules[index].clone();
    Ok(Some((
        format!("module_{}", donor.modules[index].spec.name),
        assembled,
    )))
}

fn train_track(
    config: &CampaignConfig,
    index: usize,
    generation: u64,
    saved: &mut TrackState,
    rt: &TrackRuntime,
    queue: &WorkQueue,
) -> Result<Option<TrackRecord>, String> {
    let track = &config.tracks[index];
    let archive = saved.archive.clone();
    let champion_start = campaign_work::parse_brain(&saved.best)?;
    // Synchronize at the generation boundary. All followers see the same
    // champion, independent of island ordering within this generation.
    let champion_hash = snapshot(&mut saved.structure_snapshots, saved.best.clone());
    for (island, settings) in saved.islands.iter_mut().zip(&track.islands) {
        if settings.follow_champion
            && follower_due(
                island
                    .champion_follows
                    .last()
                    .map(|f| f.brain_hash.as_str()),
                &champion_hash,
                config.reanchor_after,
                generation,
                island.last_progress,
            )
        {
            island.es = recentred(&island.es, champion_start.flatten(&rt.mask))?;
            island.base = Some(saved.best.clone());
            island.last_progress = generation;
            island.champion_follows.push(ChampionFollow {
                generation,
                brain_hash: champion_hash.clone(),
            });
        }
    }
    let mut records = Vec::new();
    for island_index in 0..saved.islands.len() {
        let island_config = &track.islands[island_index];
        if let Some(search) = &island_config.structural_search
            && (generation - 1).is_multiple_of(search.every)
        {
            if let Some(previous) = saved
                .structural_history
                .iter_mut()
                .rev()
                .find(|e| e.island == island_config.name)
                && previous.outcome == "training"
            {
                previous.outcome = "superseded_without_evaluation".into();
            }
            let parent = campaign_work::parse_brain(&saved.best)?;
            let seed = structural_seed(config.seed, index, island_index, generation);
            let (child, edit) = super::structural_search::mutate(&parent, &rt.mask, search, seed)?;
            let parent_sha256 = snapshot(
                &mut saved.structure_snapshots,
                campaign_work::brain_value(&parent),
            );
            let proposal_sha256 = snapshot(
                &mut saved.structure_snapshots,
                campaign_work::brain_value(&child),
            );
            let island = &mut saved.islands[island_index];
            let mut es_config = island_config.es;
            es_config.seed = seed ^ 0x535452554354;
            island.es = EvolutionStrategy::new(child.flatten(&rt.mask), es_config)?.state();
            island.base = Some(campaign_work::brain_value(&child));
            island.structural_attempts += 1;
            island.last_structural_generation = generation;
            saved.structural_history.push(StructuralAttempt {
                generation,
                island: island_config.name.clone(),
                attempt: island.structural_attempts,
                parent_sha256,
                proposal_sha256,
                edit,
                outcome: "training".into(),
                accepted_generation: None,
            });
        }
        let island = &mut saved.islands[island_index];
        let restarted_module = if island_config.restart
            && config.restart_after > 0
            && generation
                .saturating_sub(1)
                .saturating_sub(island.last_progress)
                >= config.restart_after
        {
            Some(restart_island(
                rt,
                island,
                &champion_start,
                config.restart_scale,
                config.seed ^ (index as u64) << 24 ^ island_index as u64,
                generation,
            )?)
        } else {
            None
        };
        let island_base = island
            .base
            .as_ref()
            .map(campaign_work::parse_brain)
            .transpose()?
            .unwrap_or_else(|| rt.base.clone());
        let active = island_base.parameter_activity(&rt.mask);
        let mut es = EvolutionStrategy::from_state(island.es.clone())?;
        let candidates = es
            .ask_masked(&active)?
            .iter()
            .map(|theta| {
                island_base
                    .with_flat(&rt.mask, theta)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut reports: Vec<Vec<StageReport>> = vec![vec![]; candidates.len()];
        let label = format!(
            "generation-{generation}/{}/{}/train",
            track.name, island.name
        );
        // Disjoint training epochs, with identical seed ranges for all islands
        // and candidates in a track. Validation/holdout live in separate bands.
        let seed = 10_000_000_000u64
            .checked_add(
                generation
                    .checked_mul(1_000_000_000)
                    .ok_or("generation seed overflow")?,
            )
            .and_then(|s| s.checked_add((config.seed % 1_000_000) * 1000))
            .ok_or("generation seed overflow")?;
        for stage in 0..rt.training.plan().stages.len() {
            eprintln!(
                "generation {generation} {} / {} / {}",
                track.name,
                island.name,
                rt.training.plan().stages[stage].name
            );
            let Some(batch) = queue.stage(
                &rt.training,
                &track.name,
                "training",
                stage,
                campaign_eval::stage_seed(seed, stage)?,
                &format!("{label}/stage-{stage}"),
                &candidates,
                &archive,
            )?
            else {
                return Ok(None);
            };
            for (list, report) in reports.iter_mut().zip(batch) {
                list.push(report);
            }
        }
        let reports = reports
            .into_iter()
            .map(campaign_eval::aggregate)
            .collect::<Result<Vec<_>, _>>()?;
        let scores: Vec<_> = reports.iter().map(|r| r.fitness).collect();
        let stage_diagnostics = stage_diagnostics(&reports);
        let candidate_stage_fitness = reports
            .iter()
            .map(|r| r.stages.iter().map(|s| s.fitness).collect())
            .collect();
        let optimizer = es.tell_masked(&scores, &active)?;
        let mut record = IslandRecord {
            name: island.name.clone(),
            restarted_module,
            optimizer,
            candidate_fitness: scores.clone(),
            candidate_stage_fitness,
            stage_diagnostics,
            center_validation: None,
            sample_validation: None,
            module_validation: vec![],
            promotion_challenges: vec![],
            decisions: vec![],
            promoted: vec![],
        };
        island.es = es.state();
        if validation_due(generation, config.validation_every) {
            let best_sample = scores
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(i, _)| i)
                .unwrap();
            let center = island_base
                .with_flat(&rt.mask, es.theta())
                .map_err(|e| e.to_string())?;
            let graft = module_combination(
                &champion_start,
                &center,
                &rt.mask,
                generation,
                config.module_recombination_every,
            )?;
            let donor_sha256 = super::content_hash::sha256(center.to_json().as_bytes());
            let mut validation_candidates = vec![
                ("center".to_string(), center),
                ("sample".to_string(), candidates[best_sample].clone()),
            ];
            if let Some(graft) = graft {
                validation_candidates.push(graft);
            }
            for (kind, brain) in &validation_candidates {
                let kind = kind.as_str();
                let label = format!(
                    "generation-{generation}/{}/{}/{kind}",
                    track.name, island_config.name
                );
                let Some(report) = queue.plan(
                    &rt.validation,
                    &track.name,
                    "validation",
                    1_000_000_000,
                    &label,
                    brain,
                )?
                else {
                    return Ok(None);
                };
                let reference_report = saved
                    .reference_report
                    .as_ref()
                    .unwrap_or(saved.initial_report.as_ref().unwrap())
                    .clone();
                if let (Some(frontier), Some(limit)) = (&mut saved.frontier, config.frontier_limit)
                {
                    let value = campaign_work::brain_value(brain);
                    if value != saved.best
                        && value != *saved.reference.as_ref().unwrap_or(&saved.base)
                    {
                        frontier.consider(value,report.clone(),&reference_report,
                        serde_json::json!({"generation":generation,"island":island_config.name,"candidate":kind}),limit,config.max_faction_regression.unwrap_or(0.05))?;
                    }
                }
                let mut accepted = promotes(
                    &report,
                    saved.best_report.as_ref().unwrap(),
                    saved.initial_report.as_ref().unwrap(),
                    config.min_gain,
                    config.max_stage_regression,
                );
                let mut decision = CandidateDecision {
                    candidate: kind.into(),
                    reason: "fixed_audit_rejected".into(),
                    screen_gain: report.fitness - saved.best_report.as_ref().unwrap().fitness,
                    screen_min_gain: if config.fresh_screening {
                        config.screen_min_gain
                    } else {
                        config.min_gain
                    },
                    promotion_min_gain: config.min_gain,
                    screen_candidate: None,
                    screen_incumbent: None,
                };
                if config.fresh_screening {
                    let incumbent = campaign_work::parse_brain(&saved.best)?;
                    // Screening and acceptance use disjoint seed bands. The
                    // fixed audit above remains comparable over the whole run.
                    let seed = (1u64 << 59)
                        + generation * 1_000_000_000
                        + (config.seed % 1_000_000) * 1000;
                    let Some(current) = queue.plan(
                        &rt.validation,
                        &track.name,
                        "validation",
                        seed,
                        &format!("{label}/screen/candidate"),
                        brain,
                    )?
                    else {
                        return Ok(None);
                    };
                    let Some(previous) = queue.plan(
                        &rt.validation,
                        &track.name,
                        "validation",
                        seed,
                        &format!("generation-{generation}/{}/screen/incumbent", track.name),
                        &incumbent,
                    )?
                    else {
                        return Ok(None);
                    };
                    decision.screen_gain = current.fitness - previous.fitness;
                    // Preserve fixed-audit stage floors without imposing its
                    // historical aggregate score as a second promotion bar.
                    let audit_safe = stages_safe(
                        &report,
                        saved.best_report.as_ref().unwrap(),
                        config.max_stage_regression,
                    ) && stages_safe(
                        &report,
                        saved.initial_report.as_ref().unwrap(),
                        config.max_stage_regression,
                    );
                    accepted = decision.screen_gain > config.screen_min_gain
                        && stages_safe(&current, &previous, config.max_stage_regression)
                        && audit_safe;
                    decision.reason = if !audit_safe
                        || !stages_safe(&current, &previous, config.max_stage_regression)
                    {
                        "stage_regression"
                    } else if decision.screen_gain <= 0.0 {
                        "screen_below_champion"
                    } else if decision.screen_gain <= config.screen_min_gain {
                        "screen_gain_too_small"
                    } else {
                        "screen_passed"
                    }
                    .into();
                    decision.screen_candidate = Some(current);
                    decision.screen_incumbent = Some(previous);
                } else if !accepted {
                    decision.reason = if !stages_safe(
                        &report,
                        saved.best_report.as_ref().unwrap(),
                        config.max_stage_regression,
                    ) || !stages_safe(
                        &report,
                        saved.initial_report.as_ref().unwrap(),
                        config.max_stage_regression,
                    ) {
                        "stage_regression"
                    } else if decision.screen_gain <= 0.0 {
                        "screen_below_champion"
                    } else {
                        "screen_gain_too_small"
                    }
                    .into();
                }
                if let Some(slack) = config.max_faction_regression {
                    if !super::campaign_frontier::protected(&report, &reference_report, slack)? {
                        accepted = false;
                        decision.reason = "faction_regression".into();
                    }
                }
                if accepted && config.promotion_seeds > 0 {
                    let incumbent = campaign_work::parse_brain(&saved.best)?;
                    let mut candidate_reports = Vec::new();
                    let mut incumbent_reports = Vec::new();
                    let mut reference_reports = Vec::new();
                    for sample in 0..config.promotion_seeds {
                        let seed = campaign_promotion::seed_base(generation, sample)?;
                        if config.max_faction_regression.is_some() {
                            let reference = campaign_work::parse_brain(
                                saved.reference.as_ref().unwrap_or(&saved.base),
                            )?;
                            let label = format!(
                                "generation-{generation}/{}/promotion-{sample}/reference",
                                track.name
                            );
                            let Some(r) = queue.plan(
                                &rt.promotion,
                                &track.name,
                                "promotion",
                                seed,
                                &label,
                                &reference,
                            )?
                            else {
                                return Ok(None);
                            };
                            reference_reports.push(r);
                        }

                        for (side, contender, reports) in [
                            ("candidate", brain, &mut candidate_reports),
                            ("incumbent", &incumbent, &mut incumbent_reports),
                        ] {
                            // Share unchanged-incumbent work across challengers.
                            // The sealed brain payload changes the job identity
                            // immediately after a champion is replaced.
                            let pair_label = if side == "incumbent" {
                                format!(
                                    "generation-{generation}/{}/promotion-{sample}/incumbent",
                                    track.name
                                )
                            } else {
                                format!("{label}/promotion-{sample}/candidate")
                            };
                            let Some(report) = queue.plan(
                                &rt.promotion,
                                &track.name,
                                "promotion",
                                seed,
                                &pair_label,
                                contender,
                            )?
                            else {
                                return Ok(None);
                            };
                            reports.push(report);
                        }
                    }
                    let mut challenge = campaign_promotion::assess_with_t(
                        candidate_reports,
                        incumbent_reports,
                        config.min_gain,
                        config.max_stage_regression,
                        config.promotion_min_t,
                    )?;
                    decision.reason = if challenge.mean_gain <= config.min_gain {
                        "fresh_gain_too_small"
                    } else if challenge.wins <= challenge.seeds / 2 {
                        "fresh_majority_failed"
                    } else if config.promotion_min_t > 0.0
                        && challenge.t_statistic < config.promotion_min_t
                    {
                        "fresh_evidence_too_weak"
                    } else if !challenge.accepted {
                        "fresh_stage_regression"
                    } else {
                        "promoted"
                    }
                    .into();
                    if let Some(slack) = config.max_faction_regression {
                        if !super::campaign_frontier::protected(
                            &super::campaign_frontier::mean_report(&challenge.candidate)?,
                            &super::campaign_frontier::mean_report(&reference_reports)?,
                            slack,
                        )? {
                            challenge.accepted = false;
                            decision.reason = "fresh_faction_regression".into();
                        }
                    }
                    accepted = challenge.accepted;
                    record.promotion_challenges.push(PromotionChallenge {
                        candidate: kind.into(),
                        report: challenge,
                    });
                }
                if let Some(event) = saved
                    .structural_history
                    .iter_mut()
                    .rev()
                    .find(|e| e.island == island_config.name)
                {
                    if accepted {
                        event.outcome = "accepted".into();
                        event.accepted_generation = Some(generation);
                    } else if event.outcome == "training" {
                        event.outcome = "evaluated_without_promotion".into();
                    }
                }
                if accepted {
                    if config.archive_size > 0 {
                        if saved.archive.last() != Some(&saved.best) {
                            saved.archive.push(saved.best.clone());
                        }
                        if saved.archive.len() > config.archive_size {
                            saved.archive.remove(0);
                        }
                    }
                    let parent_sha256 =
                        snapshot(&mut saved.structure_snapshots, saved.best.clone());
                    saved.best = campaign_work::brain_value(brain);
                    let brain_sha256 = snapshot(&mut saved.structure_snapshots, saved.best.clone());
                    saved.promotion_history.push(BrainPromotion {
                        generation,
                        island: island_config.name.clone(),
                        candidate: kind.into(),
                        parent_sha256,
                        brain_sha256,
                    });
                    if let Some(frontier) = &mut saved.frontier {
                        frontier.entries.retain(|e| {
                            e.brain != saved.best
                                && e.brain != *saved.reference.as_ref().unwrap_or(&saved.base)
                        });
                    }
                    saved.best_report = Some(report.clone());
                    saved.best_generation = generation;
                    saved.islands[island_index].last_progress = generation;
                    if kind == "sample" && config.recentre_on_promotion {
                        let moved =
                            recentred(&saved.islands[island_index].es, brain.flatten(&rt.mask))?;
                        saved.islands[island_index].es = moved;
                    }
                    record.promoted.push(kind.into());
                    decision.reason = "promoted".into();
                }
                // Retain evaluated candidates, including useful rejected modules,
                // outside the bounded diagnostic frontier. This is not a promotion.
                let value = campaign_work::brain_value(brain);
                let mut receipt = serde_json::json!({
                    "format":"minibar.evaluated_candidate", "version":1,
                    "track":track.name, "generation":generation,
                    "island":island_config.name, "candidate":kind,
                    "brain_sha256":super::content_hash::sha256(brain.to_json().as_bytes()),
                    "parent":campaign_work::brain_value(if kind.starts_with("module_") {&champion_start} else {&island_base}), "brain":value, "report":report,
                    "decision":decision, "input_identity":queue.input,
                    "backend_identity":queue.backend
                });
                if kind.starts_with("module_") {
                    receipt["module_donor_sha256"] = serde_json::json!(donor_sha256);
                }
                campaign_io::atomic_json(
                    &queue.out.join("candidates").join(&track.name).join(format!(
                        "generation-{generation}-{}-{kind}.json",
                        island_config.name
                    )),
                    &receipt,
                )?;
                record.decisions.push(decision);
                if kind == "center" {
                    record.center_validation = Some(report);
                } else if kind == "sample" {
                    record.sample_validation = Some(report);
                } else {
                    record
                        .module_validation
                        .push(serde_json::json!({"candidate":kind,"report":report}));
                }
            }
        }
        records.push(record);
    }
    let mut record = TrackRecord {
        name: track.name.clone(),
        islands: records,
        champion_fitness: saved.best_report.as_ref().unwrap().fitness,
        diagnostic_challengers: saved.frontier.as_ref().map(|f| f.entries.len()),
        best_generation: saved.best_generation,
    };
    compact_generation_reports(&mut record);
    Ok(Some(record))
}

pub fn run(
    config_path: &Path,
    out: &Path,
    options: CampaignOptions,
) -> Result<CampaignSummary, String> {
    let started = Instant::now();
    let console =
        console::Reporter::start(started, std::time::Duration::from_secs(30), |message| {
            eprintln!("{message}");
        });
    let config_path = config_path.canonicalize().map_err(|e| e.to_string())?;
    let config = CampaignConfig::load(&config_path)?;
    let target = options.generations.unwrap_or(config.generations);
    if target == 0
        || target > 1_000_000
        || options
            .max_seconds
            .is_some_and(|s| !s.is_finite() || s <= 0.0)
    {
        return Err("invalid campaign limits".into());
    }
    let input = campaign_io::input_identity(&config_path)?;
    let backend = campaign_io::binary_identity()?;
    let mut runtime = Vec::new();
    for track in &config.tracks {
        let base = Brain::load(&track.brain).map_err(|e| e.to_string())?;
        let mask = base
            .trainable_mask(&track.train_modules)
            .map_err(|e| e.to_string())?;
        if base.trainable_len(&mask) == 0 {
            return Err("track has no trainable parameters".into());
        }
        for island in &track.islands {
            if let Some(search) = &island.structural_search {
                super::structural_search::mutate(&base, &mask, search, 0)?;
            }
        }
        let training = PreparedPlan::new(&track.training)?;
        let validation = PreparedPlan::new(&track.validation)?;
        let mut promotion_plan = track.validation.clone();
        promotion_plan.seeds = 1;
        let promotion = PreparedPlan::new(&promotion_plan)?;
        let holdout = track.holdout.as_ref().map(PreparedPlan::new).transpose()?;
        if training.brain_kind() != base.kind
            || validation.brain_kind() != base.kind
            || holdout
                .as_ref()
                .is_some_and(|p| p.brain_kind() != base.kind)
        {
            return Err("track brain and curriculum kinds differ".into());
        }
        if config.frontier_limit.is_some() || config.max_faction_regression.is_some() {
            validation.validate_faction_metadata()?;
            promotion.validate_faction_metadata()?;
        }
        let reference = track
            .reference_brain
            .as_ref()
            .map(|p| Brain::load(p).map_err(|e| e.to_string()))
            .transpose()?;
        if reference.as_ref().is_some_and(|b| b.kind != base.kind) {
            return Err("reference kind differs".into());
        }
        runtime.push(TrackRuntime {
            reference,
            base,
            mask,
            training,
            validation,
            promotion,
            holdout,
        });
    }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(out.join(".campaign.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("campaign directory is already in use: {e}"))?;
    let path = out.join("state.json");
    let mut state = if options.resume {
        let state = campaign_io::read_json(&path)?;
        validate_state(&state, &config, &runtime, &input, &backend)?;
        state
    } else {
        if fs::read_dir(out)
            .map_err(|e| e.to_string())?
            .any(|item| item.is_ok_and(|i| i.file_name() != ".campaign.lock"))
        {
            return Err("training output is not empty; use --resume or a fresh directory".into());
        }
        let state = initial(&config, &runtime, input.clone(), backend.clone())?;
        checkpoint(out, &state, &runtime)?;
        state
    };
    if target < state.completed_generations {
        return Err("generation target is before the saved checkpoint".into());
    }
    exports(out, &state, &runtime)?;
    let queue = WorkQueue {
        campaign: &config_path,
        out,
        input: &input,
        backend: &backend,
        external: options.external,
        pending: AtomicUsize::new(0),
        used: Mutex::new(vec![]),
        started,
        max_seconds: options.max_seconds,
        stop_reason: Mutex::new(None),
        progress: Mutex::new(Progress::default()),
        console: &console,
    };
    queue.finish(&state, target, "running")?;
    for index in 0..state.tracks.len() {
        if state.tracks[index].best_report.is_none() {
            let brain = campaign_work::parse_brain(&state.tracks[index].best)?;
            let Some(report) = queue.plan(
                &runtime[index].validation,
                &config.tracks[index].name,
                "validation",
                1_000_000_000,
                "initial",
                &brain,
            )?
            else {
                return queue.incomplete(&state, target);
            };
            if campaign_io::input_identity(&config_path)? != input {
                return Err(
                    "campaign assets changed during initialization; checkpoint not advanced".into(),
                );
            }
            state.tracks[index].best_report = Some(report.clone());
            state.tracks[index].initial_report = Some(report);
            checkpoint(out, &state, &runtime)?;
            queue.cleanup()?;
            queue.finish(&state, target, "running")?;
        }
    }
    for index in 0..state.tracks.len() {
        if state.tracks[index].reference_report.is_none() {
            if let Some(value) = &state.tracks[index].reference {
                let brain = campaign_work::parse_brain(value)?;
                let Some(report) = queue.plan(
                    &runtime[index].validation,
                    &config.tracks[index].name,
                    "validation",
                    1_000_000_000,
                    "reference",
                    &brain,
                )?
                else {
                    return queue.incomplete(&state, target);
                };
                if campaign_io::input_identity(&config_path)? != input {
                    return Err("campaign assets changed during reference evaluation".into());
                }
                state.tracks[index].reference_report = Some(report);
                checkpoint(out, &state, &runtime)?;
                queue.cleanup()?;
            }
        }
        if let Some(frontier) = &state.tracks[index].frontier {
            frontier.validate()?;
        }
    }
    while state.completed_generations < target {
        if queue.should_stop()? {
            return queue.finish(&state, target, "stopped");
        }
        let generation = state.completed_generations + 1;
        let generation_started = Instant::now();
        let mut next = state.clone();
        let mut records = Vec::new();
        for index in 0..next.tracks.len() {
            let Some(record) = train_track(
                &config,
                index,
                generation,
                &mut next.tracks[index],
                &runtime[index],
                &queue,
            )?
            else {
                return queue.incomplete(&state, target);
            };
            next.tracks[index].holdout = None;
            records.push(record);
        }
        if campaign_io::input_identity(&config_path)? != input {
            return Err(
                "campaign assets changed during generation; checkpoint not advanced".into(),
            );
        }
        next.completed_generations = generation;
        next.history.push(GenerationRecord {
            generation,
            tracks: records,
        });
        checkpoint(out, &next, &runtime)?;
        state = next;
        queue.cleanup()?;
        queue.finish(&state, target, "running")?;
        eprintln!(
            "{}",
            console::generation_report(
                state.history.last().expect("committed generation"),
                target,
                generation_started.elapsed().as_secs_f64(),
            )
        );
    }
    for index in 0..state.tracks.len() {
        if let Some(prepared) = &runtime[index].holdout {
            if state.tracks[index].holdout.is_none() {
                let brain = campaign_work::parse_brain(&state.tracks[index].best)?;
                let Some(report) = queue.plan(
                    prepared,
                    &config.tracks[index].name,
                    "holdout",
                    3_000_000_000,
                    "holdout",
                    &brain,
                )?
                else {
                    return queue.incomplete(&state, target);
                };
                if campaign_io::input_identity(&config_path)? != input {
                    return Err(
                        "campaign assets changed during holdout; checkpoint not advanced".into(),
                    );
                }
                state.tracks[index].holdout = Some(report);
                checkpoint(out, &state, &runtime)?;
                queue.cleanup()?;
                queue.finish(&state, target, "running")?;
            }
        }
    }
    queue.finish(&state, target, "completed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_history_compacts_examples_without_changing_scores_or_source_evidence() {
        let mut evidence = report(0.5, 0.0);
        let records: Vec<_> = (0..8).map(|index| serde_json::json!({
            "seed":7,"index":index,"family":"heavy_swarm","learner_side":0,
            "scenario":{"name":format!("fight-{index}"),"learner":[{"unit":"armpw","count":40}]}
        })).collect();
        evidence.stages[0].details = serde_json::json!({
            "fitness_score":0.5,"families":{"heavy_swarm":0.2},
            "brain_kind":"tactical_v4","companion_identity":"frozen-air-sha256",
            "facets":{"armada":0.1,"cortex":0.3},"facets_complete":true,
            "objectives":{"skirmish":0.2},"layout_seed":7,"mean_layout_fitness":0.4,
            "total":{"wins":4,"battles":8},"matchup_records":records,
            "layouts":[{"report":{"matchup_records":records}}]
        });
        let original = serde_json::to_value(&evidence).unwrap();
        let mut history = evidence.clone();
        compact_history_report(&mut history);
        let details = &history.stages[0].details;
        assert!(details.get("layouts").is_none());
        assert_eq!(details["matchup_records"].as_array().unwrap().len(), 2);
        assert_eq!(details["coverage_counts"]["heavy_swarm"], 8);
        assert_eq!(details["matchup_records_sampled"], true);
        assert_eq!(history.fitness, evidence.fitness);
        assert_eq!(history.stages[0].fitness, evidence.stages[0].fitness);
        for key in [
            "fitness_score",
            "families",
            "facets",
            "facets_complete",
            "objectives",
            "layout_seed",
            "mean_layout_fitness",
            "total",
            "brain_kind",
            "companion_identity",
        ] {
            assert_eq!(details[key], evidence.stages[0].details[key]);
        }
        assert_eq!(serde_json::to_value(&evidence).unwrap(), original);
        assert!(evidence.stages[0].details.get("layouts").is_some());
        // Reprocessing saved history must not shrink the recorded coverage.
        let once = serde_json::to_value(&history).unwrap();
        compact_history_report(&mut history);
        assert_eq!(serde_json::to_value(&history).unwrap(), once);
    }

    #[test]
    fn generation_compaction_visits_all_validation_and_promotion_report_copies() {
        let mut evidence = report(0.5, 0.0);
        let records: Vec<_> = (0..4)
            .map(|index| {
                serde_json::json!({
                    "seed":7,"index":index,"family":"air_air"
                })
            })
            .collect();
        evidence.stages[0].details = serde_json::json!({"matchup_records":records,"layouts":[{"large":"detail"}],"families":{"air_air":0.5}});
        let challenge =
            campaign_promotion::assess(vec![evidence.clone()], vec![evidence.clone()], 0.0, 0.1)
                .unwrap();
        let mut generation = TrackRecord {
            name: "air".into(),
            champion_fitness: 0.5,
            diagnostic_challengers: None,
            best_generation: 1,
            islands: vec![IslandRecord {
                name: "local".into(),
                restarted_module: None,
                optimizer: GenerationStats {
                    generation: 1,
                    fitness_mean: 0.5,
                    fitness_best: 0.5,
                    fitness_worst: 0.5,
                    update_norm: 0.0,
                },
                candidate_fitness: vec![0.5],
                candidate_stage_fitness: vec![vec![0.5]],
                stage_diagnostics: vec![],
                center_validation: Some(evidence.clone()),
                sample_validation: Some(evidence.clone()),
                module_validation: vec![
                    serde_json::json!({"candidate":"module_air_target","report":evidence}),
                ],
                decisions: vec![CandidateDecision {
                    candidate: "center".into(),
                    reason: "fixed_audit_rejected".into(),
                    screen_gain: 0.0,
                    screen_min_gain: 0.0,
                    promotion_min_gain: 0.0,
                    screen_candidate: Some(evidence.clone()),
                    screen_incumbent: Some(evidence.clone()),
                }],
                promotion_challenges: vec![PromotionChallenge {
                    candidate: "center".into(),
                    report: challenge,
                }],
                promoted: vec![],
            }],
        };
        compact_generation_reports(&mut generation);
        let island = &generation.islands[0];
        for report in [
            island.center_validation.as_ref().unwrap(),
            island.sample_validation.as_ref().unwrap(),
            island.decisions[0].screen_candidate.as_ref().unwrap(),
            island.decisions[0].screen_incumbent.as_ref().unwrap(),
            &island.promotion_challenges[0].report.candidate[0],
            &island.promotion_challenges[0].report.incumbent[0],
        ] {
            assert!(report.stages[0].details.get("layouts").is_none());
            assert_eq!(report.stages[0].details["coverage_counts"]["air_air"], 4);
            assert_eq!(
                report.stages[0].details["matchup_records"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
        }
        assert_eq!(
            island.module_validation[0]["report"]["stages"][0]["details"]["coverage_counts"]["air_air"],
            4
        );
        assert_eq!(island.promotion_challenges[0].report.mean_gain, 0.0);
        assert_eq!(
            evidence.stages[0].details["matchup_records"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn module_combination_transfers_only_the_selected_complete_module() {
        let champion = super::super::tactical::template(4).unwrap();
        let original = champion.clone();
        let mut donor = champion.clone();
        // Distinct weights in every donor module make accidental whole-brain
        // replacement visible, while a structural edit checks complete transfer.
        let all = vec![true; donor.modules.len()];
        let theta: Vec<_> = donor.flatten(&all).iter().map(|v| v + 0.125).collect();
        donor = donor.with_flat(&all, &theta).unwrap();
        let mut structure = donor.modules[1].net.structure().clone();
        structure.inputs[0] = false;
        donor.modules[1].net.set_structure(structure).unwrap();
        let mut mask = vec![false; champion.modules.len()];
        mask[1] = true;
        mask[3] = true;
        let (kind, assembled) = module_combination(&champion, &donor, &mask, 5, 5)
            .unwrap()
            .unwrap();
        assert_eq!(kind, format!("module_{}", donor.modules[1].spec.name));
        for index in 0..champion.modules.len() {
            assert_eq!(
                assembled.modules[index],
                if index == 1 {
                    donor.modules[index].clone()
                } else {
                    champion.modules[index].clone()
                }
            );
        }
        assert_eq!(champion, original);
        let (_, next) = module_combination(&champion, &donor, &mask, 10, 5)
            .unwrap()
            .unwrap();
        assert_eq!(next.modules[3], donor.modules[3]);
        assert_eq!(next.modules[1], champion.modules[1]);
        assert!(
            module_combination(&champion, &donor, &mask, 6, 5)
                .unwrap()
                .is_none()
        );
        assert!(
            module_combination(&champion, &donor, &mask, 5, 0)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn stagnation_reanchor_is_bounded_and_disabled_by_default() {
        assert!(!follower_due(Some("a"), "a", 0, 100, 1));
        assert!(!follower_due(Some("a"), "a", 30, 31, 1));
        assert!(follower_due(Some("a"), "a", 30, 32, 1));
        assert!(follower_due(Some("a"), "b", 0, 2, 1));
    }
    fn report(easy: f64, hard: f64) -> PlanReport {
        campaign_eval::aggregate(vec![
            StageReport {
                name: "easy".into(),
                difficulty: 1,
                weight: 1.0,
                fitness: easy,
                details: Value::Null,
            },
            StageReport {
                name: "hard".into(),
                difficulty: 2,
                weight: 1.0,
                fitness: hard,
                details: Value::Null,
            },
        ])
        .unwrap()
    }
    #[test]
    fn promotions_protect_simple_skills_and_require_a_gain() {
        let base = report(0.5, 0.0);
        let improved = report(0.55, 0.2);
        assert!(promotes(&improved, &base, &base, 0.01, 0.1));
        assert!(!promotes(&report(0.0, 1.0), &base, &base, 0.01, 0.1));
        assert!(!promotes(&base, &base, &base, 0.0, 0.1));
        assert!(validation_due(1, 5));
        assert!(!validation_due(2, 5));
        assert!(validation_due(5, 5));
    }
    #[test]
    fn stage_spread_exposes_flat_and_differentiating_rewards() {
        let stats = stage_diagnostics(&[report(0.0, 0.5), report(1.0, 0.5)]);
        assert_eq!(stats[0].mean, 0.5);
        assert_eq!(stats[0].std_dev, 0.5);
        assert_eq!(stats[0].unique_fitness, 2);
        assert_eq!(stats[1].std_dev, 0.0);
        assert_eq!(stats[1].unique_fitness, 1);
    }
    #[test]
    fn recentring_moves_the_centre_and_clears_moments_but_keeps_stream_and_config() {
        let config = crate::learn::es::EsConfig {
            population: 4,
            ..Default::default()
        };
        let mut es = EvolutionStrategy::new(vec![0.0; 3], config).unwrap();
        es.tell(&[1.0, 0.0, 2.0, 3.0]).unwrap();
        let old = es.state();
        assert!(old.first_moment.iter().any(|m| *m != 0.0));
        let moved = recentred(&old, vec![0.5, -0.5, 0.25]).unwrap();
        assert_eq!(moved.theta, vec![0.5, -0.5, 0.25]);
        assert!(moved.first_moment.iter().all(|m| *m == 0.0));
        assert!(moved.second_moment.iter().all(|m| *m == 0.0));
        assert_eq!(moved.generation, old.generation);
        assert_eq!(moved.adam_updates, Some(0));
        assert_eq!(moved.config, old.config);
        // The next population is centred on the new point and uses fresh noise.
        let next = EvolutionStrategy::from_state(moved).unwrap().ask();
        let mean: Vec<f32> = (0..3)
            .map(|i| next.iter().map(|c| c[i]).sum::<f32>() / next.len() as f32)
            .collect();
        assert!((mean[0] - 0.5).abs() < 1e-5 && (mean[1] + 0.5).abs() < 1e-5);
        assert!(recentred(&old, vec![0.0; 2]).is_err());
        assert!(recentred(&old, vec![f32::NAN, 0.0, 0.0]).is_err());
    }
}
