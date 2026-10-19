//! Campaign configuration and portable content identity. No game state is read here.
use super::{campaign_eval::EvaluationPlan, es::EsConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const FORMAT: &str = "minibar-campaign";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CampaignConfig {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub generations: u64,
    pub seed: u64,
    pub validation_every: u64,
    /// Validate one full-module graft from each island centre onto the
    /// generation's starting champion at this cadence; zero disables it.
    pub module_recombination_every: u64,
    /// Fresh paired single-seed samples before promotion; zero explicitly disables it.
    pub promotion_seeds: usize,
    pub min_gain: f64,
    /// Screen on a rotating paired validation batch rather than the historical
    /// champion's fixed audit score. The final promotion pairs stay independent.
    pub fresh_screening: bool,
    pub screen_min_gain: f64,
    /// Minimum paired t statistic of the per-seed gains (mean over standard
    /// error) for a promotion; zero keeps the older gain-and-majority rule.
    pub promotion_min_t: f64,
    pub max_stage_regression: f64,
    /// After an island's sampled candidate becomes champion, restart that
    /// island's search from it (fresh optimizer moments, same generation count).
    pub recentre_on_promotion: bool,
    /// Reanchor champion followers after this many unsuccessful generations;
    /// zero preserves legacy behavior. Sampling streams remain continuous.
    pub reanchor_after: u64,
    pub restart_after: u64,
    pub restart_scale: f32,
    pub archive_size: usize,
    pub tracks: Vec<TrackConfig>,
    pub frontier_limit: Option<usize>,
    pub max_faction_regression: Option<f64>,
}
impl Default for CampaignConfig {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            name: "training campaign".into(),
            generations: 200,
            seed: 20261017,
            validation_every: 5,
            module_recombination_every: 0,
            promotion_seeds: 4,
            min_gain: 0.005,
            fresh_screening: false,
            screen_min_gain: 0.0,
            promotion_min_t: 0.0,
            max_stage_regression: 0.15,
            recentre_on_promotion: false,
            reanchor_after: 0,
            restart_after: 30,
            restart_scale: 0.25,
            archive_size: 8,
            tracks: vec![],
            frontier_limit: None,
            max_faction_regression: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackConfig {
    pub name: String,
    pub brain: PathBuf,
    #[serde(default)]
    pub reference_brain: Option<PathBuf>,
    #[serde(default)]
    pub train_modules: Vec<String>,
    pub training: EvaluationPlan,
    pub validation: EvaluationPlan,
    #[serde(default)]
    pub holdout: Option<EvaluationPlan>,
    pub islands: Vec<IslandConfig>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IslandConfig {
    pub name: String,
    pub es: EsConfig,
    #[serde(default)]
    pub restart: bool,
    /// Refine the global champion (weights and structure) at the next generation.
    #[serde(default)]
    pub follow_champion: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_search: Option<super::structural_search::StructuralSearchConfig>,
}
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
impl CampaignConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut config: Self = read_json(&path)?;
        let root = path.parent().ok_or("campaign has no parent directory")?;
        for track in &mut config.tracks {
            if track.brain.is_relative() {
                track.brain = root.join(&track.brain);
            }
            if let Some(p) = &mut track.reference_brain {
                if p.is_relative() {
                    *p = root.join(&*p);
                }
            }
            track.training.resolve_paths(root)?;
            track.validation.resolve_paths(root)?;
            if let Some(plan) = &mut track.holdout {
                plan.resolve_paths(root)?;
            }
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.version != VERSION {
            return Err("unsupported campaign format/version".into());
        }
        if self.generations == 0
            || self.generations > 1_000_000
            || self.validation_every == 0
            || self.tracks.is_empty()
        {
            return Err(
                "campaign requires positive bounded generations, validation cadence and tracks"
                    .into(),
            );
        }
        if !self.min_gain.is_finite()
            || self.min_gain < 0.0
            || !self.screen_min_gain.is_finite()
            || self.screen_min_gain < 0.0
            || (self.fresh_screening && self.promotion_seeds == 0)
            || !self.promotion_min_t.is_finite()
            || self.promotion_min_t < 0.0
            || !self.max_stage_regression.is_finite()
            || self.max_stage_regression < 0.0
            || !self.restart_scale.is_finite()
            || self.restart_scale <= 0.0
            || self.archive_size > 64
            || self.promotion_seeds > 64
        {
            return Err("invalid promotion, restart or archive settings".into());
        }
        if self.frontier_limit.is_some_and(|n| n == 0 || n > 64)
            || self
                .max_faction_regression
                .is_some_and(|v| !v.is_finite() || v < 0.0)
        {
            return Err("invalid frontier/faction controls".into());
        }
        let mut names = BTreeSet::new();
        for track in &self.tracks {
            if !safe_name(&track.name) || !names.insert(&track.name) || track.islands.is_empty() {
                return Err(
                    "track names must be unique safe directory names and have islands".into(),
                );
            }
            track.training.validate()?;
            track.validation.validate()?;
            if self.fresh_screening && track.validation.layouts < 2 {
                return Err("fresh screening requires at least two validation layouts so positions also vary".into());
            }
            if let Some(plan) = &track.holdout {
                plan.validate()?;
            }
            if track.training.stages.len() > 64
                || track.validation.stages.len() > 64
                || track.holdout.as_ref().is_some_and(|p| p.stages.len() > 64)
            {
                return Err(
                    "campaign plans are limited to 64 stages to keep seed bands disjoint".into(),
                );
            }
            let mut islands = BTreeSet::new();
            if track.islands.iter().all(|i| i.structural_search.is_some()) {
                return Err("structural search requires a weight-only island in each track".into());
            }
            for island in &track.islands {
                if !safe_name(&island.name) || !islands.insert(&island.name) {
                    return Err("island names must be unique safe directory names".into());
                }
                island.es.validate()?;
                if island.follow_champion && (island.restart || island.structural_search.is_some())
                {
                    return Err(
                        "champion follower must be a weight-only island without random restart"
                            .into(),
                    );
                }
                if let Some(search) = &island.structural_search {
                    search.validate()?;
                    if island.restart {
                        return Err("structural search cannot use random-module restart".into());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod promotion_config_tests {
    use super::*;

    #[test]
    fn module_recombination_is_opt_in_and_round_trips() {
        let mut config: CampaignConfig =
            serde_json::from_str(include_str!("../../examples/campaign/smoke.json")).unwrap();
        assert_eq!(config.module_recombination_every, 0);
        config.module_recombination_every = 5;
        config.validate().unwrap();
        let parsed: CampaignConfig =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assert_eq!(parsed.module_recombination_every, 5);
    }

    #[test]
    fn paired_promotion_defaults_and_explicit_disable_are_serializable() {
        let mut config = CampaignConfig::default();
        assert_eq!(config.promotion_seeds, 4);
        config = serde_json::from_str(include_str!("../../examples/campaign/smoke.json")).unwrap();
        config.validate().unwrap();
        config.promotion_seeds = 0;
        config.validate().unwrap();
        let parsed: CampaignConfig =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assert_eq!(parsed.promotion_seeds, 0);
        config.promotion_seeds = 65;
        assert!(config.validate().is_err());
        config.promotion_seeds = 64;
        config.validate().unwrap();
    }
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    atomic_bytes(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
pub fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&temporary, path).map_err(|e| format!("commit {}: {e}", path.display()))
}
/// Stable identity for trusted worker/cache matching, not cryptographic authentication.
pub fn digest(bytes: &[u8]) -> String {
    let mut h = 0xcbf29ce484222325u64;
    for byte in bytes {
        h = (h ^ *byte as u64).wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64-{h:016x}")
}
pub fn binary_identity() -> Result<String, String> {
    Ok(digest(
        &fs::read(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    ))
}

/// Hash configuration semantics plus every runtime dependency. Paths are replaced
/// by content identities, so a correctly rewritten portable bundle has the same ID.
/// The requested total generation count may grow when resuming.
pub fn input_identity(path: &Path) -> Result<String, String> {
    signature(path, &mut BTreeSet::new(), true)
}
fn signature(path: &Path, active: &mut BTreeSet<PathBuf>, root: bool) -> Result<String, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("dependency {}: {e}", path.display()))?;
    if active.len() >= 64 || !active.insert(path.clone()) {
        return Err("cyclic/deep campaign dependency graph".into());
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let identity = if path.extension().is_some_and(|e| e == "json") {
        let mut value: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        if root {
            if let Some(object) = value.as_object_mut() {
                object.remove("generations");
            }
        }
        rewrite_identities(&mut value, path.parent().unwrap(), active)?;
        digest(&serde_json::to_vec(&value).map_err(|e| e.to_string())?)
    } else {
        digest(&bytes)
    };
    active.remove(&path);
    Ok(identity)
}
fn rewrite_identities(
    value: &mut Value,
    root: &Path,
    active: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, item) in object {
                if matches!(
                    key.as_str(),
                    "catalog"
                        | "config"
                        | "brain"
                        | "reference_brain"
                        | "init_from"
                        | "companion_brain"
                        | "map"
                ) {
                    if let Some(path) = item.as_str().filter(|s| !s.is_empty()) {
                        if key == "map" {
                            let bytes = fs::read(root.join(path)).map_err(|e| e.to_string())?;
                            crate::map::CompiledMap::from_cache_bytes(&bytes).map_err(|_| "campaign maps must be compiled VSGMAP caches; compile source maps before training")?;
                        }
                        *item = Value::String(signature(&root.join(path), active, false)?);
                        continue;
                    }
                }
                if key == "opponents" {
                    if let Some(items) = item.as_array_mut() {
                        for opponent in items {
                            if let Some(path) =
                                opponent.as_str().and_then(|s| s.strip_prefix("brain:"))
                            {
                                *opponent = Value::String(format!(
                                    "brain:{}",
                                    signature(&root.join(path), active, false)?
                                ));
                            }
                        }
                    }
                }
                rewrite_identities(item, root, active)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_identities(item, root, active)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_survives_relocation_but_detects_changed_assets() {
        let root = std::env::temp_dir().join(format!("campaign-identity-{}", std::process::id()));
        for name in ["a", "b"] {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join("asset.bin"), b"same terrain").unwrap();
            fs::write(
                root.join(name).join("stage.json"),
                r#"{"catalog":"asset.bin"}"#,
            )
            .unwrap();
            fs::write(
                root.join(name).join("campaign.json"),
                r#"{"generations":2,"config":"stage.json"}"#,
            )
            .unwrap();
        }
        let a = root.join("a/campaign.json");
        let b = root.join("b/campaign.json");
        assert_eq!(input_identity(&a).unwrap(), input_identity(&b).unwrap());
        fs::write(&b, r#"{"generations":300,"config":"stage.json"}"#).unwrap();
        assert_eq!(input_identity(&a).unwrap(), input_identity(&b).unwrap());
        fs::write(root.join("b/asset.bin"), b"changed terrain").unwrap();
        assert_ne!(input_identity(&a).unwrap(), input_identity(&b).unwrap());
    }
    #[test]
    fn dependency_cycles_and_unsafe_names_are_rejected() {
        let path = std::env::temp_dir().join(format!("campaign-cycle-{}.json", std::process::id()));
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"config":path})).unwrap(),
        )
        .unwrap();
        assert!(input_identity(&path).unwrap_err().contains("cyclic"));
        assert!(!safe_name("../escape"));
        assert!(!safe_name("a/b"));
        assert!(safe_name("ground-v1"));
    }
    #[test]
    fn map_sources_cannot_introduce_untracked_acquisition_sidecars() {
        let root = std::env::temp_dir().join(format!("campaign-map-source-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("map.json"), "{}").unwrap();
        fs::write(root.join("config.json"), r#"{"map":"map.json"}"#).unwrap();
        assert!(
            input_identity(&root.join("config.json"))
                .unwrap_err()
                .contains("compiled VSGMAP")
        );
    }
}
