//! Diagnostic alternatives, independent from official champion selection.
use super::{campaign_eval::PlanReport, content_hash::sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
pub fn dimensions(report: &PlanReport) -> Result<BTreeMap<String, f64>, String> {
    let mut result = BTreeMap::new();
    for stage in &report.stages {
        if stage
            .details
            .get("facets_complete")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err("faction protection requires complete faction metadata".into());
        }
        let facets = stage
            .details
            .get("facets")
            .and_then(Value::as_object)
            .ok_or("faction protection requires explicit faction metadata")?;
        if facets.is_empty() {
            return Err("faction protection requires nonempty faction metadata".into());
        }
        for (f, v) in facets {
            let value = v
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("invalid faction score")?;
            result.insert(format!("{}/{}", stage.name, f), value);
        }
    }
    if result.is_empty() {
        return Err("empty frontier dimensions".into());
    }
    Ok(result)
}
pub fn protected(
    candidate: &PlanReport,
    reference: &PlanReport,
    slack: f64,
) -> Result<bool, String> {
    if !slack.is_finite() || slack < 0.0 {
        return Err("invalid faction slack".into());
    }
    let a = dimensions(candidate)?;
    let b = dimensions(reference)?;
    if a.keys().ne(b.keys()) {
        return Err("faction dimensions differ".into());
    }
    Ok(a.iter().all(|(k, v)| *v >= b[k] - slack))
}
pub fn mean_report(reports: &[PlanReport]) -> Result<PlanReport, String> {
    let mut result = reports.first().ok_or("empty paired reference")?.clone();
    let all: Vec<_> = reports.iter().map(dimensions).collect::<Result<_, _>>()?;
    let keys = all[0].keys().collect::<Vec<_>>();
    if all.iter().any(|r| r.keys().collect::<Vec<_>>() != keys) {
        return Err("paired facet dimensions differ".into());
    }
    for stage in &mut result.stages {
        for (f, v) in stage.details["facets"].as_object_mut().unwrap() {
            let k = format!("{}/{}", stage.name, f);
            *v = serde_json::json!(all.iter().map(|r| r[&k]).sum::<f64>() / all.len() as f64);
        }
    }
    Ok(result)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrontierEntry {
    pub sha256: String,
    pub brain: Value,
    pub report: PlanReport,
    pub dimensions: BTreeMap<String, f64>,
    pub provenance: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrontierState {
    pub version: u32,
    pub entries: Vec<FrontierEntry>,
    pub pruning: Vec<Value>,
}
impl Default for FrontierState {
    fn default() -> Self {
        Self {
            version: 1,
            entries: vec![],
            pruning: vec![],
        }
    }
}
fn dominates(a: &BTreeMap<String, f64>, b: &BTreeMap<String, f64>) -> bool {
    a.keys().eq(b.keys()) && a.iter().all(|(k, v)| *v >= b[k]) && a.iter().any(|(k, v)| *v > b[k])
}
impl FrontierState {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.entries.len() > 64 {
            return Err("invalid frontier version/size".into());
        }
        for e in &self.entries {
            if sha256(&serde_json::to_vec(&e.brain).map_err(|e| e.to_string())?) != e.sha256
                || dimensions(&e.report)? != e.dimensions
            {
                return Err("frontier content identity differs".into());
            }
        }
        Ok(())
    }
    pub fn consider(
        &mut self,
        brain: Value,
        report: PlanReport,
        reference: &PlanReport,
        provenance: Value,
        limit: usize,
        slack: f64,
    ) -> Result<bool, String> {
        if limit == 0 || limit > 64 {
            return Err("frontier limit must be 1..64".into());
        }
        self.validate()?;
        if !protected(&report, reference, slack)? {
            return Ok(false);
        }
        let hash = sha256(&serde_json::to_vec(&brain).map_err(|e| e.to_string())?);
        if self.entries.iter().any(|e| e.sha256 == hash) {
            return Ok(false);
        }
        let dims = dimensions(&report)?;
        if self.entries.iter().any(|e| dominates(&e.dimensions, &dims)) {
            return Ok(false);
        }
        self.entries.retain(|e| !dominates(&dims, &e.dimensions));
        self.entries.push(FrontierEntry {
            sha256: hash.clone(),
            brain,
            report,
            dimensions: dims,
            provenance,
        });
        self.entries.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        if self.entries.len() > limit {
            self.prune(limit);
        }
        Ok(self.entries.iter().any(|e| e.sha256 == hash))
    }
    fn prune(&mut self, limit: usize) {
        let keys: Vec<_> = self.entries[0].dimensions.keys().cloned().collect();
        let bounds: BTreeMap<_, _> = keys
            .iter()
            .map(|k| {
                (
                    k.clone(),
                    [
                        self.entries
                            .iter()
                            .map(|e| e.dimensions[k])
                            .fold(f64::INFINITY, f64::min),
                        self.entries
                            .iter()
                            .map(|e| e.dimensions[k])
                            .fold(f64::NEG_INFINITY, f64::max),
                    ],
                )
            })
            .collect();
        let mut selected = Vec::new();
        let preference = |a: usize, b: usize| {
            self.entries[a]
                .report
                .fitness
                .total_cmp(&self.entries[b].report.fitness)
                .then_with(|| self.entries[b].sha256.cmp(&self.entries[a].sha256))
        };
        for key in &keys {
            if selected.len() == limit {
                break;
            }
            let best = (0..self.entries.len())
                .max_by(|a, b| {
                    self.entries[*a].dimensions[key]
                        .total_cmp(&self.entries[*b].dimensions[key])
                        .then_with(|| preference(*a, *b))
                })
                .unwrap();
            if !selected.contains(&best) {
                selected.push(best);
            }
        }
        let distance = |a: usize, b: usize| {
            keys.iter()
                .map(|k| {
                    let [lo, hi] = bounds[k];
                    if hi == lo {
                        0.0
                    } else {
                        ((self.entries[a].dimensions[k] - self.entries[b].dimensions[k])
                            / (hi - lo))
                            .powi(2)
                    }
                })
                .sum::<f64>()
        };
        while selected.len() < limit {
            let best = (0..self.entries.len())
                .filter(|i| !selected.contains(i))
                .max_by(|a, b| {
                    let da = selected
                        .iter()
                        .map(|i| distance(*a, *i))
                        .fold(f64::INFINITY, f64::min);
                    let db = selected
                        .iter()
                        .map(|i| distance(*b, *i))
                        .fold(f64::INFINITY, f64::min);
                    da.total_cmp(&db).then_with(|| preference(*a, *b))
                })
                .unwrap();
            selected.push(best);
        }
        let retained: Vec<_> = selected
            .iter()
            .map(|i| self.entries[*i].sha256.clone())
            .collect();
        self.pruning
            .push(serde_json::json!({"bounds":bounds,"retained":retained}));
        self.entries = self
            .entries
            .iter()
            .enumerate()
            .filter(|(i, _)| selected.contains(i))
            .map(|(_, e)| e.clone())
            .collect();
    }
}
