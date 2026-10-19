//! Ordered curriculum evaluation shared by campaign coordinators and workers.

use super::{brain::Brain, strategic, train};
use crate::catalog::{self, UnitCatalog};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageTask {
    Tactical { config: PathBuf },
    Strategic { config: PathBuf },
}

impl StageTask {
    pub fn config(&self) -> &Path {
        match self {
            Self::Tactical { config } | Self::Strategic { config } => config,
        }
    }

    fn config_mut(&mut self) -> &mut PathBuf {
        match self {
            Self::Tactical { config } | Self::Strategic { config } => config,
        }
    }

    fn domain(&self) -> &'static str {
        match self {
            Self::Tactical { .. } => "tactical",
            Self::Strategic { .. } => "strategic",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageSpec {
    pub name: String,
    pub difficulty: u32,
    pub weight: f64,
    pub task: StageTask,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationPlan {
    #[serde(default = "super::campaign_sampling::one")]
    pub layouts: usize,
    pub catalog: PathBuf,
    pub stages: Vec<StageSpec>,
    pub seeds: usize,
}

impl EvaluationPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.catalog.as_os_str().is_empty() || self.seeds == 0 || self.stages.is_empty() {
            return Err("evaluation needs a catalog, positive seeds and at least one stage".into());
        }
        validate_stages(
            self.stages
                .iter()
                .map(|s| (s.name.as_str(), s.difficulty, s.weight)),
        )?;
        let domain = self.stages[0].task.domain();
        if self
            .stages
            .iter()
            .any(|s| s.task.domain() != domain || s.task.config().as_os_str().is_empty())
        {
            return Err("all stages must have config paths and use the same brain domain".into());
        }
        if self.seeds as u128 > STAGE_SEED_STRIDE as u128 {
            return Err("stage seed count exceeds the reserved seed interval".into());
        }
        super::campaign_sampling::sample_seed(
            0,
            self.layouts.saturating_sub(1),
            self.seeds.saturating_sub(1),
            self.layouts,
            self.seeds,
        )?;
        stage_seed(0, self.stages.len() - 1)?;
        Ok(())
    }

    pub fn resolve_paths(&mut self, root: &Path) -> Result<(), String> {
        let root =
            std::path::absolute(root).map_err(|e| format!("resolve evaluation root: {e}"))?;
        if self.catalog.is_relative() {
            self.catalog = root.join(&self.catalog);
        }
        for stage in &mut self.stages {
            let config = stage.task.config_mut();
            if config.is_relative() {
                *config = root.join(&*config);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageReport {
    pub name: String,
    pub difficulty: u32,
    pub weight: f64,
    pub fitness: f64,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanReport {
    pub fitness: f64,
    pub stages: Vec<StageReport>,
}

pub(crate) fn family_scores(
    report: &PlanReport,
) -> Result<std::collections::BTreeMap<String, f64>, String> {
    let mut scores = std::collections::BTreeMap::new();
    for stage in &report.stages {
        if let Some(families) = stage
            .details
            .get("families")
            .and_then(serde_json::Value::as_object)
        {
            for (family, score) in families {
                let score = score
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or("invalid family score")?;
                scores.insert(format!("{}/{}", stage.name, family), score);
            }
        }
    }
    Ok(scores)
}

pub(crate) fn families_safe(candidate: &PlanReport, baseline: &PlanReport, slack: f64) -> bool {
    let (Ok(a), Ok(b)) = (family_scores(candidate), family_scores(baseline)) else {
        return false;
    };
    a.keys().eq(b.keys()) && a.iter().all(|(k, v)| *v >= b[k] - slack)
}

fn validate_stages<'a>(
    stages: impl IntoIterator<Item = (&'a str, u32, f64)>,
) -> Result<(), String> {
    let mut previous = None;
    let mut names = BTreeSet::new();
    for (name, difficulty, weight) in stages {
        if name.trim().is_empty() || !names.insert(name) {
            return Err("stage names must be nonempty and unique".into());
        }
        if !weight.is_finite() || weight <= 0.0 {
            return Err(format!(
                "stage {name:?}: weight must be finite and positive"
            ));
        }
        if previous.is_some_and(|last| difficulty < last) {
            return Err("stages must be ordered by nondecreasing difficulty".into());
        }
        previous = Some(difficulty);
    }
    Ok(())
}

/// A stage's explicit weight is independent of its number of simulated battles.
pub fn aggregate(stages: Vec<StageReport>) -> Result<PlanReport, String> {
    if stages.is_empty() {
        return Err("cannot aggregate an empty evaluation".into());
    }
    validate_stages(
        stages
            .iter()
            .map(|s| (s.name.as_str(), s.difficulty, s.weight)),
    )?;
    if stages.iter().any(|s| !s.fitness.is_finite()) {
        return Err("stage fitness must be finite".into());
    }
    // Scale both axes before summing: valid finite scores and weights need not
    // overflow merely because their unnormalised products would be enormous.
    let max_weight = stages.iter().map(|s| s.weight).fold(0.0, f64::max);
    let scale = stages.iter().map(|s| s.fitness.abs()).fold(0.0, f64::max);
    let denominator: f64 = stages.iter().map(|s| s.weight / max_weight).sum();
    let fitness = if scale == 0.0 {
        0.0
    } else {
        let normalized: f64 = stages
            .iter()
            .map(|s| (s.weight / max_weight) / denominator * (s.fitness / scale))
            .sum();
        normalized.clamp(-1.0, 1.0) * scale
    };
    if !fitness.is_finite() {
        return Err("aggregate fitness is not finite".into());
    }
    Ok(PlanReport { fitness, stages })
}

pub const STAGE_SEED_STRIDE: u64 = 1_000_000;

fn historical_index(seed: u64, count: usize) -> usize {
    let mut rng = super::rng::Rng::for_stream(seed, 0x4849_5354_4f52_5941);
    (rng.next_u64() % count as u64) as usize
}

/// Stable, disjoint seed ranges for stages within one evaluation plan.
pub fn stage_seed(seed_base: u64, stage_index: usize) -> Result<u64, String> {
    u64::try_from(stage_index)
        .ok()
        .and_then(|i| i.checked_mul(STAGE_SEED_STRIDE))
        .and_then(|offset| seed_base.checked_add(offset))
        .ok_or_else(|| "stage seed range overflows u64".into())
}

enum PreparedTask {
    Tactical {
        config: train::TrainConfig,
        opponents: Vec<train::Opponent>,
        companion: Option<Brain>,
    },
    Strategic(strategic::Prepared),
}

fn annotate_dimensions(
    v: &mut serde_json::Value,
    config: &train::TrainConfig,
    r: &train::Evaluation,
) {
    if config.matchups.as_ref().is_some_and(|m| {
        m.starting_distance_mode == super::matchups::StartingDistanceMode::UnitRanges
    }) {
        v["combat_budget"] = serde_json::json!(config.max_time);
    }
    let generated = r
        .matchup_records
        .iter()
        .map(|m| &m.scenario)
        .collect::<Vec<_>>();
    let scenarios = config
        .scenarios
        .iter()
        .chain(generated.iter().copied())
        .collect::<Vec<_>>();
    let mut objectives = serde_json::Map::new();
    for kind in [
        None,
        Some(super::objective::ObjectiveKind::Attack),
        Some(super::objective::ObjectiveKind::Defend),
    ] {
        let names = scenarios
            .iter()
            .filter(|s| s.objective.as_ref().map(|o| o.kind) == kind)
            .map(|s| s.name.as_str())
            .collect::<BTreeSet<_>>();
        if !names.is_empty() {
            let label = match kind {
                None => "fight",
                Some(super::objective::ObjectiveKind::Attack) => "attack",
                _ => "defend",
            };
            objectives.insert(
                label.into(),
                serde_json::json!(super::objective::robust_fitness(
                    r.rows
                        .iter()
                        .filter(|r| names.contains(r.scenario.as_str()))
                        .map(|r| &r.tally),
                    &config.objective
                )),
            );
        }
    }
    v["objectives"] = objectives.into();
    let mut facets = serde_json::Map::new();
    for faction in scenarios
        .iter()
        .filter_map(|sc| sc.faction.as_ref())
        .collect::<BTreeSet<_>>()
    {
        let names: BTreeSet<_> = scenarios
            .iter()
            .filter(|sc| sc.faction.as_ref() == Some(faction))
            .map(|sc| sc.name.as_str())
            .collect();
        let f = super::objective::robust_fitness(
            r.rows
                .iter()
                .filter(|row| names.contains(row.scenario.as_str()))
                .map(|r| &r.tally),
            &config.objective,
        );
        facets.insert(faction.clone(), serde_json::json!(f));
    }
    v["facets_complete"] = serde_json::json!(
        scenarios
            .iter()
            .all(|s| s.faction.as_ref().is_some_and(|f| !f.trim().is_empty()))
    );
    v["facets"] = facets.into();
}

fn merge_evaluation_rows(
    rows: &mut Vec<train::EvaluationRow>,
    incoming: Vec<train::EvaluationRow>,
    keep_cases: bool,
) {
    if keep_cases {
        rows.extend(incoming);
        return;
    }
    for row in incoming {
        if let Some(existing) = rows
            .iter_mut()
            .find(|r| r.scenario == row.scenario && r.opponent == row.opponent)
        {
            existing.tally.merge(&row.tally);
        } else {
            rows.push(row);
        }
    }
}

pub struct PreparedPlan {
    plan: EvaluationPlan,
    catalog: UnitCatalog,
    tasks: Vec<PreparedTask>,
    brain_kind: String,
    opponent: Option<Brain>,
}

impl PreparedPlan {
    pub fn new(plan: &EvaluationPlan) -> Result<Self, String> {
        plan.validate()?;
        let mut plan = plan.clone();
        plan.resolve_paths(Path::new("."))?;
        let catalog = catalog::load_catalog(&plan.catalog)?;
        let mut tasks = Vec::new();
        let mut brain_kind: Option<String> = None;
        for stage in &plan.stages {
            let (kind, task) = match &stage.task {
                StageTask::Tactical { config } => {
                    let config = train::TrainConfig::load(config)
                        .map_err(|e| format!("stage {:?}: {e}", stage.name))?;
                    config
                        .validate()
                        .map_err(|e| format!("stage {:?}: {e}", stage.name))?;
                    let opponents = train::opponents_from_config(&config)
                        .map_err(|e| format!("stage {:?}: {e}", stage.name))?;
                    let companion = config
                        .companion_brain
                        .as_ref()
                        .map(|p| Brain::load(p))
                        .transpose()
                        .map_err(|e| e.to_string())?;
                    (
                        config.brain_kind.clone(),
                        PreparedTask::Tactical {
                            config,
                            opponents,
                            companion,
                        },
                    )
                }
                StageTask::Strategic { config } => {
                    let config = strategic::TrainConfig::load(config)
                        .map_err(|e| format!("stage {:?}: {e}", stage.name))?;
                    let prepared = strategic::Prepared::new(&config, &catalog)
                        .map_err(|e| format!("stage {:?}: {e}", stage.name))?;
                    (config.brain_kind.clone(), PreparedTask::Strategic(prepared))
                }
            };
            if brain_kind
                .as_ref()
                .is_some_and(|previous| previous != &kind)
            {
                return Err(format!(
                    "stage {:?}: brain kind {kind:?} differs from earlier stages",
                    stage.name
                ));
            }
            brain_kind = Some(kind);
            tasks.push(task);
        }
        Ok(Self {
            plan,
            catalog,
            tasks,
            opponent: None,
            brain_kind: brain_kind.expect("validated nonempty plan"),
        })
    }

    pub fn brain_kind(&self) -> &str {
        &self.brain_kind
    }
    pub fn plan(&self) -> &EvaluationPlan {
        &self.plan
    }

    /// The supplied seed base is used directly; callers crossing stage barriers
    /// should calculate it with [`stage_seed`], as [`Self::evaluate`] does.
    pub fn evaluate_stage(
        &self,
        stage_index: usize,
        brain: &Brain,
        seed_base: u64,
    ) -> Result<StageReport, String> {
        self.evaluate_stage_with_archive(stage_index, brain, seed_base, &[])
    }

    /// Training may mix one deterministic archived opponent with the unchanged
    /// baseline suite. Validation and holdout callers use an empty archive.
    pub fn evaluate_stage_with_archive(
        &self,
        stage_index: usize,
        brain: &Brain,
        seed_base: u64,
        archive: &[Brain],
    ) -> Result<StageReport, String> {
        if brain.kind != self.brain_kind || archive.iter().any(|b| b.kind != self.brain_kind) {
            return Err("brain kind does not match evaluation".into());
        }
        super::campaign_sampling::sample_seed(
            seed_base,
            self.plan.layouts - 1,
            self.plan.seeds - 1,
            self.plan.layouts,
            self.plan.seeds,
        )?;
        let stage = self
            .plan
            .stages
            .get(stage_index)
            .ok_or("stage out of bounds")?;
        let mut reports = Vec::new();
        for layout in 0..self.plan.layouts {
            let base = super::campaign_sampling::sample_seed(
                seed_base,
                layout,
                0,
                self.plan.layouts,
                self.plan.seeds,
            )?;
            let separated = self.plan.layouts > 1;
            let mut report =
                self.raw_report(stage_index, brain, base, separated, self.opponent.as_ref())?;
            if !archive.is_empty() {
                let historical = self.raw_report(
                    stage_index,
                    brain,
                    base,
                    separated,
                    Some(&archive[historical_index(seed_base, archive.len())]),
                )?;
                let a = report["fitness_score"]
                    .as_f64()
                    .ok_or("missing raw score")?;
                let b = historical["fitness_score"]
                    .as_f64()
                    .ok_or("missing archive score")?;
                report = serde_json::json!({"baseline":report,"historical":historical,"baseline_fitness":a,"historical_fitness":b,"archive_index":historical_index(seed_base,archive.len()),"fitness_score":0.5*a+0.5*b});
            }
            reports.push(serde_json::json!({"layout":layout,"layout_seed":base,"simulator_seed_base":base,"simulator_seeds":self.plan.seeds,"fitness":report["fitness_score"],"report":report}));
        }
        let mut details = if reports.len() == 1 {
            reports[0]["report"].clone()
        } else {
            self.pool_reports(stage_index, &reports, archive.is_empty())?
        };
        let fitness = details["fitness_score"]
            .as_f64()
            .ok_or("missing aggregate fitness")?;
        if !fitness.is_finite() {
            return Err("nonfinite evaluation fitness".into());
        }
        if self.plan.layouts > 1 {
            details["mean_layout_fitness"] = serde_json::json!(
                reports
                    .iter()
                    .map(|r| r["fitness"].as_f64().unwrap())
                    .sum::<f64>()
                    / reports.len() as f64
            );
            details["layouts"] = serde_json::json!(reports);
        }
        Ok(StageReport {
            name: stage.name.clone(),
            difficulty: stage.difficulty,
            weight: stage.weight,
            fitness,
            details,
        })
    }

    pub fn validate_faction_metadata(&self) -> Result<(), String> {
        for task in &self.tasks {
            if let PreparedTask::Tactical { config, .. } = task {
                if config
                    .scenarios
                    .iter()
                    .any(|s| s.faction.as_ref().is_none_or(|f| f.trim().is_empty()))
                {
                    return Err(
                        "faction protection requires explicit faction metadata for every scenario"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn set_opponent(&mut self, brain: Brain) -> Result<(), String> {
        if brain.kind != self.brain_kind {
            return Err("fixed opponent brain kind differs".into());
        }
        self.opponent = Some(brain);
        Ok(())
    }

    fn raw_report(
        &self,
        index: usize,
        brain: &Brain,
        base: u64,
        separated: bool,
        opponent: Option<&Brain>,
    ) -> Result<serde_json::Value, String> {
        let mut result = match &self.tasks[index] {
            PreparedTask::Tactical {
                config,
                opponents,
                companion,
            } => {
                let learner = train::domain_policy(brain.clone(), companion.clone())
                    .map_err(|e| e.to_string())?;
                let replacement = if let Some(b) = opponent {
                    Some(vec![train::Opponent {
                        label: "fixed-brain".into(),
                        policy: train::domain_policy(b.clone(), companion.clone())
                            .map_err(|e| e.to_string())?,
                    }])
                } else {
                    None
                };
                let opponents = replacement.as_ref().unwrap_or(opponents);
                let r = if separated {
                    train::evaluate_on_layout(
                        config,
                        &self.catalog,
                        learner.as_ref(),
                        opponents,
                        self.plan.seeds,
                        base,
                        base,
                    )
                } else {
                    train::evaluate_at(
                        config,
                        &self.catalog,
                        learner.as_ref(),
                        opponents,
                        self.plan.seeds,
                        base,
                    )
                }
                .map_err(|e| e.to_string())?;
                let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
                v["fitness_score"] = serde_json::json!(r.robust_fitness);
                v["brain_kind"] = serde_json::json!(brain.kind);
                v["companion_identity"] = serde_json::json!(
                    companion
                        .as_ref()
                        .map(|b| super::content_hash::sha256(b.to_json().as_bytes()))
                );
                annotate_dimensions(&mut v, config, &r);
                v
            }
            PreparedTask::Strategic(prepared) => {
                let r = if separated {
                    prepared.evaluate_on_layout(brain, opponent, self.plan.seeds, base, base)?
                } else if let Some(opponent) = opponent {
                    prepared.evaluate_against(brain, opponent, self.plan.seeds, base)?
                } else {
                    prepared.evaluate(brain, self.plan.seeds, base)?
                };
                let mut v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
                v["fitness_score"] = serde_json::json!(r.fitness);
                let factions: BTreeSet<_> = r.trials.iter().map(|t| t.faction.clone()).collect();
                let facets: serde_json::Map<String, serde_json::Value> = factions
                    .into_iter()
                    .map(|f| {
                        let rows: Vec<_> = r.trials.iter().filter(|r| r.faction == f).collect();
                        (
                            f,
                            serde_json::json!(
                                rows.iter().map(|r| r.fitness).sum::<f64>() / rows.len() as f64
                            ),
                        )
                    })
                    .collect();
                v["facets_complete"] = serde_json::json!(true);
                v["facets"] = facets.into();
                v
            }
        };
        if !result["fitness_score"].as_f64().is_some_and(f64::is_finite) {
            return Err("nonfinite evaluation".into());
        }
        // Metadata is evaluation-only and does not affect the score.
        result["layout_seed"] = serde_json::json!(base);
        Ok(result)
    }

    fn pool_reports(
        &self,
        index: usize,
        reports: &[serde_json::Value],
        plain: bool,
    ) -> Result<serde_json::Value, String> {
        if !plain {
            let baseline: Vec<_> = reports
                .iter()
                .map(|r| serde_json::json!({"report":r["report"]["baseline"]}))
                .collect();
            let historical: Vec<_> = reports
                .iter()
                .map(|r| serde_json::json!({"report":r["report"]["historical"]}))
                .collect();
            let a = self.pool_reports(index, &baseline, true)?;
            let b = self.pool_reports(index, &historical, true)?;
            return Ok(
                serde_json::json!({"fitness_score":0.5*a["fitness_score"].as_f64().unwrap()+0.5*b["fitness_score"].as_f64().unwrap(),"baseline":a,"historical":b}),
            );
        }
        match &self.tasks[index] {
            PreparedTask::Tactical { config, .. } => {
                let mut aggregate: train::Evaluation =
                    serde_json::from_value(reports[0]["report"].clone())
                        .map_err(|e| e.to_string())?;
                for report in &reports[1..] {
                    let next: train::Evaluation = serde_json::from_value(report["report"].clone())
                        .map_err(|e| e.to_string())?;
                    aggregate.total.merge(&next.total);
                    aggregate.matchup_records.extend(next.matchup_records);
                    merge_evaluation_rows(
                        &mut aggregate.rows,
                        next.rows,
                        config.matchups.is_some(),
                    );
                }
                aggregate.robust_fitness = super::objective::robust_fitness(
                    aggregate.rows.iter().map(|r| &r.tally),
                    &config.objective,
                );
                aggregate.families = train::family_fitness(
                    &aggregate.rows,
                    &aggregate.matchup_records,
                    &config.objective,
                );
                let mut v = serde_json::to_value(&aggregate).map_err(|e| e.to_string())?;
                v["fitness_score"] = serde_json::json!(aggregate.robust_fitness);
                annotate_dimensions(&mut v, config, &aggregate);
                Ok(v)
            }
            PreparedTask::Strategic(_) => {
                let trials: Vec<_> = reports
                    .iter()
                    .flat_map(|r| r["report"]["trials"].as_array().unwrap().iter().cloned())
                    .collect();
                let fitness = trials
                    .iter()
                    .map(|t| t["fitness"].as_f64().unwrap())
                    .sum::<f64>()
                    / trials.len().max(1) as f64;
                let factions: BTreeSet<_> = trials
                    .iter()
                    .map(|t| t["faction"].as_str().unwrap())
                    .collect();
                let facets: serde_json::Map<String, serde_json::Value> = factions
                    .into_iter()
                    .map(|f| {
                        let rows: Vec<_> = trials.iter().filter(|r| r["faction"] == f).collect();
                        (
                            f.into(),
                            serde_json::json!(
                                rows.iter()
                                    .map(|r| r["fitness"].as_f64().unwrap())
                                    .sum::<f64>()
                                    / rows.len() as f64
                            ),
                        )
                    })
                    .collect();
                Ok(
                    serde_json::json!({"fitness":fitness,"fitness_score":fitness,"trials":trials,"facets":facets,"facets_complete":true}),
                )
            }
        }
    }

    /// Evaluates every stage, in declared order, without fitness-based skipping.
    pub fn evaluate(&self, brain: &Brain, seed_base: u64) -> Result<PlanReport, String> {
        let mut reports = Vec::with_capacity(self.tasks.len());
        for index in 0..self.tasks.len() {
            reports.push(self.evaluate_stage(index, brain, stage_seed(seed_base, index)?)?);
        }
        aggregate(reports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooling_keeps_new_compositions_instead_of_zipping_rows() {
        let row = |name: &str| train::EvaluationRow {
            scenario: name.into(),
            opponent: "hold".into(),
            tally: Default::default(),
        };
        let mut rows = vec![row("heavy-swarm-a")];
        merge_evaluation_rows(
            &mut rows,
            vec![row("heavy-swarm-b"), row("heavy-swarm-a")],
            false,
        );
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.scenario == "heavy-swarm-b"));
        // Generated layouts keep anchors as separate cases too, giving them
        // the same weight as the newly sampled compositions in each layout.
        merge_evaluation_rows(&mut rows, vec![row("heavy-swarm-a")], true);
        assert_eq!(rows.len(), 3);
    }

    fn stage(name: &str, difficulty: u32, weight: f64, fitness: f64) -> StageReport {
        StageReport {
            name: name.into(),
            difficulty,
            weight,
            fitness,
            details: serde_json::json!({"battles": if name == "easy" { 100 } else { 1 }}),
        }
    }

    #[test]
    fn stages_keep_order_and_weights_do_not_depend_on_battle_counts() {
        let report =
            aggregate(vec![stage("easy", 1, 1.0, 2.0), stage("hard", 2, 3.0, 6.0)]).unwrap();
        assert_eq!(report.fitness, 5.0);
        assert_eq!(
            report
                .stages
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["easy", "hard"]
        );
    }

    #[test]
    fn aggregate_rejects_empty_nonfinite_and_invalid_weights() {
        assert!(aggregate(vec![]).is_err());
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(aggregate(vec![stage("bad", 1, 1.0, invalid)]).is_err());
            assert!(aggregate(vec![stage("bad", 1, invalid, 1.0)]).is_err());
        }
        assert!(aggregate(vec![stage("bad", 1, 0.0, 1.0)]).is_err());
        assert!(aggregate(vec![stage("bad", 1, -1.0, 1.0)]).is_err());
        assert!(aggregate(vec![stage("hard", 2, 1.0, 1.0), stage("easy", 1, 1.0, 1.0)]).is_err());
    }

    fn plan() -> EvaluationPlan {
        EvaluationPlan {
            layouts: 1,
            catalog: "catalog.json".into(),
            seeds: 1,
            stages: vec![StageSpec {
                name: "easy".into(),
                difficulty: 1,
                weight: 1.0,
                task: StageTask::Tactical {
                    config: "easy.json".into(),
                },
            }],
        }
    }

    #[test]
    fn plans_reject_unsorted_duplicate_mixed_and_invalid_inputs() {
        let mut p = plan();
        assert!(p.validate().is_ok());
        p.seeds = 0;
        assert!(p.validate().is_err());
        p = plan();
        p.stages[0].weight = f64::NAN;
        assert!(p.validate().is_err());
        p = plan();
        p.stages.push(StageSpec {
            name: "macro".into(),
            difficulty: 2,
            weight: 1.0,
            task: StageTask::Strategic {
                config: "macro.json".into(),
            },
        });
        assert!(p.validate().is_err());
        p = plan();
        p.stages.push(p.stages[0].clone());
        assert!(p.validate().is_err());
        p = plan();
        p.stages[0].difficulty = 2;
        let mut next = p.stages[0].clone();
        next.name = "later".into();
        next.difficulty = 1;
        p.stages.push(next);
        assert!(p.validate().is_err());
    }

    #[test]
    fn relative_paths_are_absolute_and_resolution_is_idempotent() {
        let mut p = plan();
        p.resolve_paths(Path::new("examples")).unwrap();
        assert!(p.catalog.is_absolute());
        let first = serde_json::to_value(&p).unwrap();
        p.resolve_paths(Path::new("different")).unwrap();
        assert_eq!(first, serde_json::to_value(&p).unwrap());
        match &p.stages[0].task {
            StageTask::Tactical { config } => assert!(config.is_absolute()),
            _ => panic!(),
        }
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "minibar-campaign-eval-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("catalog.json"),
                include_str!("../../examples/pawn-hello.json"),
            )
            .unwrap();
            let mut config = train::TrainConfig::default_for_unit("armpw");
            config.opponents = vec!["charge".into()];
            config.scenarios.truncate(1);
            config.max_time = 0.1;
            config.layout_jitter = 0.0;
            for (name, kind) in [
                ("easy.json", super::super::tactical::KIND),
                ("hard.json", super::super::tactical::KIND),
                ("v4.json", super::super::tactical::KIND_V4),
            ] {
                config.brain_kind = kind.into();
                std::fs::write(path.join(name), serde_json::to_vec(&config).unwrap()).unwrap();
            }
            Self(path)
        }
        fn plan(&self) -> EvaluationPlan {
            let mut plan = plan();
            plan.stages.push(StageSpec {
                name: "hard".into(),
                difficulty: 2,
                weight: 3.0,
                task: StageTask::Tactical {
                    config: "hard.json".into(),
                },
            });
            plan.resolve_paths(&self.0).unwrap();
            plan
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn prepared_tactical_plan_reproduces_order_scores_and_stage_seed_ranges() {
        let fixture = Fixture::new();
        let prepared = PreparedPlan::new(&fixture.plan()).unwrap();
        let brain = super::super::tactical::template(6).unwrap();
        let first = prepared.evaluate(&brain, 100).unwrap();
        let again = prepared.evaluate(&brain, 100).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(again).unwrap()
        );
        assert_eq!(
            first
                .stages
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["easy", "hard"]
        );
        for (index, stage) in first.stages.iter().enumerate() {
            let separate = prepared
                .evaluate_stage(index, &brain, stage_seed(100, index).unwrap())
                .unwrap();
            assert_eq!(
                serde_json::to_value(stage).unwrap(),
                serde_json::to_value(separate).unwrap()
            );
        }
        assert_eq!(prepared.brain_kind(), super::super::tactical::KIND);
        assert!(prepared.plan().catalog.is_absolute());
        assert!(prepared.evaluate_stage(2, &brain, 100).is_err());
        assert!(
            prepared
                .evaluate_stage(
                    0,
                    &super::super::tactical::template_for_kind(super::super::tactical::KIND_V4, 6)
                        .unwrap(),
                    100
                )
                .is_err()
        );
    }

    #[test]
    fn preparation_rejects_mixed_tactical_versions_and_invalid_config() {
        let fixture = Fixture::new();
        let mut plan = fixture.plan();
        plan.stages[1].task = StageTask::Tactical {
            config: fixture.0.join("v4.json"),
        };
        assert!(
            PreparedPlan::new(&plan)
                .err()
                .unwrap()
                .contains("brain kind")
        );
        let mut invalid = train::TrainConfig::load(&fixture.0.join("easy.json")).unwrap();
        invalid.max_time = -1.0;
        std::fs::write(
            fixture.0.join("easy.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert!(
            PreparedPlan::new(&fixture.plan())
                .err()
                .unwrap()
                .contains("max_time")
        );
    }

    #[test]
    fn finite_extreme_weights_and_seed_overflow_are_handled() {
        let result = aggregate(vec![
            stage("easy", 1, f64::MAX, f64::MAX),
            stage("hard", 2, f64::MAX, -f64::MAX),
        ])
        .unwrap();
        assert_eq!(result.fitness, 0.0);
        assert_eq!(stage_seed(100, 1).unwrap(), 100 + STAGE_SEED_STRIDE);
        assert!(stage_seed(u64::MAX, 1).is_err());
    }

    #[test]
    fn campaign_seed_schedule_visits_historical_opponents() {
        for count in [2, 4, 8] {
            let selected: BTreeSet<_> = (1..=32)
                .map(|generation| {
                    historical_index(10_000_000_000 + generation * 1_000_000_000, count)
                })
                .collect();
            assert!(selected.len() > 1, "archive of {count} never rotates");
        }
    }

    #[test]
    fn archived_opponent_is_deterministic_and_baseline_still_counts() {
        let fixture = Fixture::new();
        let prepared = PreparedPlan::new(&fixture.plan()).unwrap();
        let brain = super::super::tactical::template(6).unwrap();
        let archive = vec![brain.clone(), brain.clone()];
        let baseline = prepared.evaluate_stage(0, &brain, 101).unwrap();
        let mixed = prepared
            .evaluate_stage_with_archive(0, &brain, 101, &archive)
            .unwrap();
        let again = prepared
            .evaluate_stage_with_archive(0, &brain, 101, &archive)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&mixed).unwrap(),
            serde_json::to_value(again).unwrap()
        );
        assert_eq!(
            mixed.details["archive_index"],
            historical_index(101, archive.len())
        );
        assert_eq!(mixed.details["baseline"], baseline.details);
        let historical = mixed.details["historical"]["robust_fitness"]
            .as_f64()
            .unwrap();
        assert_eq!(mixed.fitness, 0.5 * baseline.fitness + 0.5 * historical);
        assert!(
            prepared
                .evaluate_stage_with_archive(0, &brain, 101, &[strategic::template(8).unwrap()])
                .is_err()
        );
    }
}
