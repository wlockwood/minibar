//! Versioned work units shared by the local coordinator and standalone workers.
use super::{
    brain::Brain,
    campaign_eval::{EvaluationPlan, PreparedPlan, StageReport},
    campaign_io::{self, CampaignConfig},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationJob {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub campaign: PathBuf,
    pub input_identity: String,
    pub backend_identity: String,
    pub label: String,
    pub track: String,
    pub plan: String,
    pub stage: usize,
    pub seed_base: u64,
    pub brain: Value,
    pub archive: Vec<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationResult {
    pub format: String,
    pub version: u32,
    pub job_id: String,
    pub input_identity: String,
    pub backend_identity: String,
    pub report: StageReport,
}
impl EvaluationJob {
    pub fn seal(&mut self) -> Result<(), String> {
        self.id = self.expected_id()?;
        Ok(())
    }
    pub fn expected_id(&self) -> Result<String, String> {
        let mut value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let object = value.as_object_mut().ok_or("invalid work payload")?;
        object.remove("id");
        object.remove("campaign");
        Ok(campaign_io::digest(
            &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        ))
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "minibar-evaluation-job"
            || self.version != 1
            || self.id != self.expected_id()?
        {
            return Err("invalid/stale evaluation job identity or protocol version".into());
        }
        if !matches!(
            self.plan.as_str(),
            "training" | "validation" | "holdout" | "promotion"
        ) {
            return Err("unknown evaluation plan".into());
        }
        if self.plan != "training" && !self.archive.is_empty() {
            return Err(
                "validation/holdout/promotion cannot use changing archive opponents".into(),
            );
        }
        Ok(())
    }
}
pub fn brain_value(brain: &Brain) -> Value {
    serde_json::from_str(&brain.to_json()).expect("validated brain serializes")
}
pub fn parse_brain(value: &Value) -> Result<Brain, String> {
    Brain::from_json(&value.to_string()).map_err(|e| e.to_string())
}
pub fn plan_for<'a>(
    config: &'a CampaignConfig,
    track: &str,
    plan: &str,
) -> Result<Cow<'a, EvaluationPlan>, String> {
    let track = config
        .tracks
        .iter()
        .find(|t| t.name == track)
        .ok_or("job references unknown track")?;
    match plan {
        "training" => Ok(Cow::Borrowed(&track.training)),
        "validation" => Ok(Cow::Borrowed(&track.validation)),
        "promotion" => {
            let mut plan = track.validation.clone();
            plan.seeds = 1;
            Ok(Cow::Owned(plan))
        }
        "holdout" => track
            .holdout
            .as_ref()
            .map(Cow::Borrowed)
            .ok_or_else(|| "track has no holdout plan".into()),
        _ => Err("unknown evaluation plan".into()),
    }
}
pub fn execute_prepared(
    job: &EvaluationJob,
    prepared: &PreparedPlan,
) -> Result<EvaluationResult, String> {
    job.validate()?;
    if job.plan == "promotion" && prepared.plan().seeds != 1 {
        return Err("promotion work must evaluate exactly one paired seed".into());
    }
    let brain = parse_brain(&job.brain)?;
    let archive = job
        .archive
        .iter()
        .map(parse_brain)
        .collect::<Result<Vec<_>, _>>()?;
    let report =
        prepared.evaluate_stage_with_archive(job.stage, &brain, job.seed_base, &archive)?;
    let result = EvaluationResult {
        format: "minibar-evaluation-result".into(),
        version: 1,
        job_id: job.id.clone(),
        input_identity: job.input_identity.clone(),
        backend_identity: job.backend_identity.clone(),
        report,
    };
    validate_result(job, prepared.plan(), &result)?;
    Ok(result)
}
pub fn validate_result(
    job: &EvaluationJob,
    plan: &EvaluationPlan,
    result: &EvaluationResult,
) -> Result<(), String> {
    job.validate()?;
    if job.plan == "promotion" && plan.seeds != 1 {
        return Err("promotion results require a single-seed plan".into());
    }
    let expected = plan
        .stages
        .get(job.stage)
        .ok_or("result stage out of bounds")?;
    if result.format != "minibar-evaluation-result"
        || result.version != 1
        || result.job_id != job.id
        || result.input_identity != job.input_identity
        || result.backend_identity != job.backend_identity
        || result.report.name != expected.name
        || result.report.difficulty != expected.difficulty
        || result.report.weight != expected.weight
        || !result.report.fitness.is_finite()
    {
        return Err("worker result does not match job, stage, inputs or backend".into());
    }
    Ok(())
}

fn verify_inputs_unchanged(campaign: &Path, expected: &str) -> Result<(), String> {
    if campaign_io::input_identity(campaign)? != expected {
        return Err(
            "worker campaign assets changed; refusing to publish or reuse a stale result".into(),
        );
    }
    Ok(())
}
/// A moved worker bundle can override the coordinator's campaign location.
/// Content identity stays the same even when paths have been rewritten.
pub fn worker(
    job_path: &Path,
    output: &Path,
    config_override: Option<&Path>,
) -> Result<EvaluationResult, String> {
    let mut job: EvaluationJob = campaign_io::read_json(job_path)?;
    job.validate()?;
    let campaign = config_override.map(Path::to_path_buf).unwrap_or_else(|| {
        if job.campaign.is_relative() {
            job_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&job.campaign)
        } else {
            job.campaign.clone()
        }
    });
    if campaign_io::input_identity(&campaign)? != job.input_identity
        || campaign_io::binary_identity()? != job.backend_identity
    {
        return Err("worker campaign assets or executable differ from the job".into());
    }
    let config = CampaignConfig::load(&campaign)?;
    let plan = plan_for(&config, &job.track, &job.plan)?;
    // campaign is an addressing hint excluded from job identity.
    job.campaign = campaign;
    let prepared = PreparedPlan::new(plan.as_ref())?;
    if output.exists() {
        let existing: EvaluationResult = campaign_io::read_json(output)?;
        validate_result(&job, plan.as_ref(), &existing)?;
        verify_inputs_unchanged(&job.campaign, &job.input_identity)?;
        return Ok(existing);
    }
    let result = execute_prepared(&job, &prepared)?;
    verify_inputs_unchanged(&job.campaign, &job.input_identity)?;
    campaign_io::atomic_json(output, &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::campaign_eval::{StageSpec, StageTask};
    use super::*;
    fn job() -> EvaluationJob {
        let mut j = EvaluationJob {
            format: "minibar-evaluation-job".into(),
            version: 1,
            id: String::new(),
            campaign: "campaign.json".into(),
            input_identity: "assets".into(),
            backend_identity: "backend".into(),
            label: "generation1-candidate0".into(),
            track: "tactical".into(),
            plan: "training".into(),
            stage: 0,
            seed_base: 1000,
            brain: serde_json::json!({}),
            archive: vec![],
        };
        j.seal().unwrap();
        j
    }
    #[test]
    fn job_identity_detects_changes_but_allows_relocated_campaign() {
        let mut j = job();
        j.validate().unwrap();
        j.campaign = "different/campaign.json".into();
        j.validate().unwrap();
        j.seed_base += 1;
        assert!(j.validate().is_err());
    }
    #[test]
    fn publication_rechecks_bundle_content_identity() {
        let root =
            std::env::temp_dir().join(format!("campaign-worker-recheck-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let campaign = root.join("campaign.json");
        let asset = root.join("asset.json");
        std::fs::write(&campaign, r#"{"config":"asset.json"}"#).unwrap();
        std::fs::write(&asset, r#"{"value":1}"#).unwrap();
        let identity = campaign_io::input_identity(&campaign).unwrap();
        verify_inputs_unchanged(&campaign, &identity).unwrap();
        std::fs::write(&asset, r#"{"value":2}"#).unwrap();
        assert!(verify_inputs_unchanged(&campaign, &identity).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn promotion_protocol_uses_an_owned_single_seed_validation_plan() {
        use super::super::campaign_io::{IslandConfig, TrackConfig};
        let plan = EvaluationPlan {
            layouts: 1,
            catalog: "catalog.json".into(),
            seeds: 7,
            stages: vec![StageSpec {
                name: "easy".into(),
                difficulty: 1,
                weight: 1.0,
                task: StageTask::Tactical {
                    config: "tactical.json".into(),
                },
            }],
        };
        let config = CampaignConfig {
            tracks: vec![TrackConfig {
                reference_brain: None,
                name: "tactical".into(),
                brain: "brain.json".into(),
                train_modules: vec![],
                training: plan.clone(),
                validation: plan.clone(),
                holdout: None,
                islands: vec![IslandConfig {
                    follow_champion: false,
                    name: "local".into(),
                    es: Default::default(),
                    restart: false,
                    structural_search: None,
                }],
            }],
            ..Default::default()
        };
        let promotion = plan_for(&config, "tactical", "promotion").unwrap();
        assert!(matches!(promotion, std::borrow::Cow::Owned(_)));
        assert_eq!(promotion.seeds, 1);
        assert_eq!(config.tracks[0].validation.seeds, 7);
        assert!(matches!(
            plan_for(&config, "tactical", "validation").unwrap(),
            std::borrow::Cow::Borrowed(_)
        ));
        let mut j = job();
        j.plan = "promotion".into();
        j.seal().unwrap();
        j.validate().unwrap();
        j.archive = vec![Value::Null];
        j.seal().unwrap();
        assert!(j.validate().is_err());
    }
    #[test]
    fn stale_wrong_stage_and_nonfinite_worker_results_are_rejected() {
        let j = job();
        let plan = EvaluationPlan {
            layouts: 1,
            catalog: "c.json".into(),
            seeds: 1,
            stages: vec![StageSpec {
                name: "easy".into(),
                difficulty: 1,
                weight: 1.0,
                task: StageTask::Tactical {
                    config: "t.json".into(),
                },
            }],
        };
        let mut result = EvaluationResult {
            format: "minibar-evaluation-result".into(),
            version: 1,
            job_id: j.id.clone(),
            input_identity: j.input_identity.clone(),
            backend_identity: j.backend_identity.clone(),
            report: StageReport {
                name: "easy".into(),
                difficulty: 1,
                weight: 1.0,
                fitness: 0.5,
                details: Value::Null,
            },
        };
        validate_result(&j, &plan, &result).unwrap();
        result.job_id = "stale".into();
        assert!(validate_result(&j, &plan, &result).is_err());
        result.job_id = j.id.clone();
        result.report.name = "hard".into();
        assert!(validate_result(&j, &plan, &result).is_err());
        result.report.name = "easy".into();
        result.report.fitness = f64::INFINITY;
        assert!(validate_result(&j, &plan, &result).is_err());
    }
}
