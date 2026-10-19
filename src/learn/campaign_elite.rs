//! Bounded, reproducible reuse of fresh validation evidence. Holdouts never enter here.
use super::super::campaign_io::TrackConfig;
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Origin {
    generation: u64,
    island: String,
    candidate: String,
    import_index: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub brain: Value,
    pub parent: Value,
    pub brain_sha256: String,
    pub origin: Origin,
    observations: u64,
    mean_gain: f64,
    stage_gains: Vec<f64>,
    last_retested: Option<u64>,
    last_screen_gain: Option<f64>,
    last_paired_gain: Option<f64>,
    last_reason: String,
}
fn hash(value: &Value) -> String {
    super::super::content_hash::sha256(
        campaign_work::parse_brain(value)
            .expect("validated elite")
            .to_json()
            .as_bytes(),
    )
}
impl Entry {
    pub fn import(brain: Value, index: usize) -> Self {
        Self {
            brain_sha256: hash(&brain),
            parent: brain.clone(),
            brain,
            origin: Origin {
                generation: 0,
                island: "import".into(),
                candidate: "import".into(),
                import_index: Some(index),
            },
            observations: 0,
            mean_gain: 0.0,
            stage_gains: vec![],
            last_retested: None,
            last_screen_gain: None,
            last_paired_gain: None,
            last_reason: "awaiting_fresh_evaluation".into(),
        }
    }
}
pub(super) fn imports(track: &TrackConfig, base: &Brain) -> Result<Vec<Value>, String> {
    let specs = base
        .modules
        .iter()
        .map(|m| m.spec.clone())
        .collect::<Vec<_>>();
    let mut result = vec![];
    for path in &track.elite_brains {
        let brain = Brain::load(path).map_err(|e| format!("elite {}: {e}", path.display()))?;
        brain
            .check_against(&base.kind, &specs)
            .map_err(|e| e.to_string())?;
        if brain.param_count() != base.param_count() {
            return Err("elite parameter shape differs".into());
        }
        let value = campaign_work::brain_value(&brain);
        if !result.contains(&value) {
            result.push(value);
        }
    }
    Ok(result)
}
pub(super) fn next<'a>(pool: &'a [Entry], champion: &Value) -> Option<&'a Entry> {
    pool.iter().filter(|e| &e.brain != champion).min_by(|a, b| {
        a.last_retested
            .unwrap_or(0)
            .cmp(&b.last_retested.unwrap_or(0))
            .then(a.origin.generation.cmp(&b.origin.generation))
            .then(a.brain_sha256.cmp(&b.brain_sha256))
    })
}
/// Keep an aggregate leader and round-robin stage specialists. Scores are running
/// means of fresh paired differences, so negative absolute fitness is meaningful.
fn trim(pool: &mut Vec<Entry>, size: usize) {
    if pool.len() <= size {
        return;
    }
    let mut selected = Vec::new();
    // Imported brains get a first fresh probe before competing with measured elites.
    for (index, e) in pool.iter().enumerate() {
        if e.observations == 0 && selected.len() < size {
            selected.push(index);
        }
    }
    let stages = pool.iter().map(|e| e.stage_gains.len()).max().unwrap_or(0);
    for dimension in 0..=stages {
        if selected.len() == size {
            break;
        }
        if let Some(index) = (0..pool.len())
            .filter(|i| !selected.contains(i))
            .max_by(|a, b| {
                let score = |i: usize| {
                    if dimension == 0 {
                        pool[i].mean_gain
                    } else {
                        pool[i]
                            .stage_gains
                            .get(dimension - 1)
                            .copied()
                            .unwrap_or(f64::NEG_INFINITY)
                    }
                };
                score(*a)
                    .total_cmp(&score(*b))
                    .then(pool[*a].observations.cmp(&pool[*b].observations))
                    .then(pool[*b].brain_sha256.cmp(&pool[*a].brain_sha256))
            })
        {
            selected.push(index);
        }
    }
    while selected.len() < size {
        let index = (0..pool.len())
            .filter(|i| !selected.contains(i))
            .max_by(|a, b| {
                pool[*a]
                    .mean_gain
                    .total_cmp(&pool[*b].mean_gain)
                    .then(pool[*b].brain_sha256.cmp(&pool[*a].brain_sha256))
            })
            .unwrap();
        selected.push(index);
    }
    let mut index = 0;
    pool.retain(|_| {
        let keep = selected.contains(&index);
        index += 1;
        keep
    });
}
pub(super) fn consider(
    pool: &mut Vec<Entry>,
    brain: &Brain,
    parent: Value,
    generation: u64,
    island: &str,
    kind: &str,
    decision: &CandidateDecision,
    paired: Option<&PromotionReport>,
    size: usize,
    retest: bool,
) {
    let (Some(current), Some(previous)) = (&decision.screen_candidate, &decision.screen_incumbent)
    else {
        return;
    };
    let gains: Vec<f64> = current
        .stages
        .iter()
        .zip(&previous.stages)
        .map(|(a, b)| a.fitness - b.fitness)
        .collect();
    let gain = paired.map_or(decision.screen_gain, |p| p.mean_gain);
    let value = campaign_work::brain_value(brain);
    let identity = hash(&value);
    let existing = pool.iter().position(|e| e.brain_sha256 == identity);
    if existing.is_none() && gain <= 0.0 && !gains.iter().any(|g| *g > 0.0) {
        return;
    }
    let index = existing.unwrap_or_else(|| {
        pool.push(Entry {
            brain: value,
            parent,
            brain_sha256: identity,
            origin: Origin {
                generation,
                island: island.into(),
                candidate: kind.into(),
                import_index: None,
            },
            observations: 0,
            mean_gain: 0.0,
            stage_gains: vec![0.0; gains.len()],
            last_retested: None,
            last_screen_gain: None,
            last_paired_gain: None,
            last_reason: String::new(),
        });
        pool.len() - 1
    });
    let entry = &mut pool[index];
    entry.observations += 1;
    let n = entry.observations as f64;
    entry.mean_gain += (gain - entry.mean_gain) / n;
    if entry.stage_gains.is_empty() {
        entry.stage_gains = vec![0.0; gains.len()];
    }
    for (mean, gain) in entry.stage_gains.iter_mut().zip(gains) {
        *mean += (gain - *mean) / n;
    }
    if retest {
        entry.last_retested = Some(generation);
    }
    entry.last_screen_gain = Some(decision.screen_gain);
    entry.last_paired_gain = paired.map(|p| p.mean_gain);
    entry.last_reason = decision.reason.clone();
    trim(pool, size);
}
pub(super) fn metadata(pool: &[Entry]) -> Value {
    Value::Array(
        pool.iter()
            .map(|e| {
                serde_json::json!({"brain_sha256":e.brain_sha256,"origin":e.origin,
        "observations":e.observations,"mean_gain":e.mean_gain,"stage_gains":e.stage_gains,
        "last_retested":e.last_retested,"last_screen_gain":e.last_screen_gain,
        "last_paired_gain":e.last_paired_gain,"last_reason":e.last_reason})
            })
            .collect(),
    )
}
pub(super) fn validate(
    saved: &TrackState,
    track: &TrackConfig,
    config: &CampaignConfig,
    base: &Brain,
    generation: u64,
) -> Result<(), String> {
    if saved.elite_imports != imports(track, base)?
        || saved.elite_pool.len() > config.elite_size
        || (config.elite_retest_every == 0 && !saved.elite_pool.is_empty())
    {
        return Err("checkpoint elite imports/pool differ".into());
    }
    let specs = base
        .modules
        .iter()
        .map(|m| m.spec.clone())
        .collect::<Vec<_>>();
    let mut identities = std::collections::BTreeSet::new();
    for e in &saved.elite_pool {
        for value in [&e.brain, &e.parent] {
            campaign_work::parse_brain(value)?
                .check_against(&base.kind, &specs)
                .map_err(|e| e.to_string())?;
        }
        if hash(&e.brain) != e.brain_sha256
            || !identities.insert(&e.brain_sha256)
            || e.origin.generation > generation
            || e.last_retested.is_some_and(|g| {
                g == 0 || g > generation || g % config.elite_retest_every.max(1) != 0
            })
            || !e.mean_gain.is_finite()
            || e.stage_gains.iter().any(|v| !v.is_finite())
            || e.last_screen_gain.is_some_and(|g| !g.is_finite())
            || e.last_paired_gain.is_some_and(|g| !g.is_finite())
            || (e.observations > 0 && e.stage_gains.len() != track.validation.stages.len())
            || e.origin
                .import_index
                .is_some_and(|i| saved.elite_imports.get(i) != Some(&e.brain))
        {
            return Err("checkpoint elite identity/evidence invalid".into());
        }
    }
    Ok(())
}

/// Check scheduling and the capped dashboard copy against committed decisions.
pub(super) fn validate_history(
    saved: &TrackState,
    state: &CampaignState,
    config: &CampaignConfig,
) -> Result<(), String> {
    let mut last = None;
    let mut tested = std::collections::BTreeMap::new();
    for generation in &state.history {
        let row = generation
            .tracks
            .iter()
            .find(|t| t.name == saved.name)
            .ok_or("elite history track missing")?;
        let decisions: Vec<_> = row
            .islands
            .iter()
            .enumerate()
            .flat_map(|(index, island)| {
                island
                    .decisions
                    .iter()
                    .filter(|d| d.candidate.starts_with("elite_"))
                    .map(move |d| (index, d))
            })
            .collect();
        if decisions.len() != row.elite_retests
            || decisions.len() > 1
            || (!decisions.is_empty()
                && (config.elite_retest_every == 0
                    || generation.generation % config.elite_retest_every != 0
                    || decisions[0].0 != 0))
        {
            return Err("checkpoint elite retest schedule/count differs".into());
        }
        if let Some((_, decision)) = decisions.first() {
            let identity = decision.candidate.strip_prefix("elite_").unwrap();
            tested.insert(identity, generation.generation);
            last = Some((generation.generation, identity, *decision));
        }
    }
    match (&saved.last_elite_retest, last) {
        (None, None) => {}
        (Some(summary), Some((generation, identity, decision))) => {
            if summary["generation"].as_u64() != Some(generation)
                || summary["brain_sha256"].as_str() != Some(identity)
                || summary["screen_gain"].as_f64() != Some(decision.screen_gain)
                || summary["reason"].as_str() != Some(decision.reason.as_str())
            {
                return Err("checkpoint last elite retest differs from decisions".into());
            }
        }
        _ => return Err("checkpoint last elite retest missing/unexpected".into()),
    }
    for entry in &saved.elite_pool {
        let historical = tested.get(entry.brain_sha256.as_str()).copied();
        if entry.last_retested.is_some() && entry.last_retested != historical
            || entry.last_retested.is_none()
                && historical.is_some_and(|g| g >= entry.origin.generation)
        {
            return Err("checkpoint elite retest timestamp differs from decisions".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn brain(offset: f32) -> Brain {
        let brain = Brain::load(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/brains/from_replays/tactical/baseline.json"),
        )
        .unwrap();
        let mask = vec![true; brain.modules.len()];
        let params = brain
            .flatten(&mask)
            .into_iter()
            .map(|x| x + offset)
            .collect::<Vec<_>>();
        brain.with_flat(&mask, &params).unwrap()
    }
    fn report(values: &[f64]) -> PlanReport {
        campaign_eval::aggregate(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| StageReport {
                    name: format!("stage-{i}"),
                    difficulty: i as u32 + 1,
                    weight: 1.0,
                    fitness: *v,
                    details: Value::Null,
                })
                .collect(),
        )
        .unwrap()
    }
    fn decision(current: &[f64], previous: &[f64]) -> CandidateDecision {
        let current = report(current);
        let previous = report(previous);
        CandidateDecision {
            candidate: "center".into(),
            reason: "stage_regression".into(),
            screen_gain: current.fitness - previous.fitness,
            screen_min_gain: 0.0,
            promotion_min_gain: 0.0,
            screen_candidate: Some(current),
            screen_incumbent: Some(previous),
        }
    }
    #[test]
    fn negative_fitness_specialists_survive_stage_veto_and_retests_update_running_evidence() {
        let base = brain(0.0);
        let candidate = brain(0.1);
        let mut pool = vec![];
        consider(
            &mut pool,
            &candidate,
            campaign_work::brain_value(&base),
            1,
            "local",
            "center",
            &decision(&[-4.0, -20.0], &[-10.0, -10.0]),
            None,
            2,
            false,
        );
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].mean_gain, -2.0);
        assert_eq!(pool[0].stage_gains, vec![6.0, -10.0]);
        consider(
            &mut pool,
            &candidate,
            campaign_work::brain_value(&base),
            4,
            "local",
            "elite",
            &decision(&[-2.0, -2.0], &[-10.0, -10.0]),
            None,
            2,
            true,
        );
        assert_eq!(pool[0].origin.generation, 1);
        assert_eq!(pool[0].observations, 2);
        assert_eq!(pool[0].mean_gain, 3.0);
        assert_eq!(pool[0].last_retested, Some(4));
    }
    #[test]
    fn bounded_pool_deduplicates_and_rotates_untested_and_least_recent_entries() {
        let a = campaign_work::brain_value(&brain(0.1));
        let b = campaign_work::brain_value(&brain(0.2));
        let mut pool = vec![Entry::import(a.clone(), 0), Entry::import(b.clone(), 1)];
        let champion = campaign_work::brain_value(&brain(0.0));
        let first = next(&pool, &champion).unwrap().brain_sha256.clone();
        pool.iter_mut()
            .find(|e| e.brain_sha256 == first)
            .unwrap()
            .last_retested = Some(3);
        assert_ne!(next(&pool, &champion).unwrap().brain_sha256, first);
        assert_eq!(next(&pool, &b).unwrap().brain, a);
        let candidate = brain(0.1);
        for generation in 1..=3 {
            consider(
                &mut pool,
                &candidate,
                champion.clone(),
                generation,
                "local",
                "center",
                &decision(&[2.0, 3.0], &[0.0, 0.0]),
                None,
                2,
                false,
            );
        }
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.iter().find(|e| e.brain == a).unwrap().observations, 3);
        trim(&mut pool, 1);
        assert_eq!(pool.len(), 1);
    }
}
