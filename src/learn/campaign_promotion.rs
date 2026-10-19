//! Fresh paired promotion checks. These are conservative acceptance rules,
//! not a statistical confidence interval or a significance test.

use super::campaign_eval::{PlanReport, aggregate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromotionReport {
    pub candidate_fitness: f64,
    pub incumbent_fitness: f64,
    pub mean_gain: f64,
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
    pub seeds: usize,
    /// Mean gain over its standard error across the pairs; zero when there is
    /// one pair or no spread to measure it against and the gain is not positive.
    #[serde(default)]
    pub t_statistic: f64,
    pub accepted: bool,
    pub candidate: Vec<PlanReport>,
    pub incumbent: Vec<PlanReport>,
}

fn mean(values: impl IntoIterator<Item = f64>) -> Result<f64, String> {
    let values: Vec<_> = values.into_iter().collect();
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err("paired promotion needs nonempty finite scores".into());
    }
    let scale = values.iter().map(|value| value.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return Ok(0.0);
    }
    let count = values.len() as f64;
    Ok(values
        .iter()
        .map(|value| (value / scale) / count)
        .sum::<f64>()
        .clamp(-1.0, 1.0)
        * scale)
}

/// Both lists must contain corresponding fresh seeds in the same order.
/// Acceptance requires an aggregate gain, a strict majority of positive pairs,
/// and no stage whose mean falls below its incumbent mean by more than slack.
pub fn assess(
    candidate: Vec<PlanReport>,
    incumbent: Vec<PlanReport>,
    min_gain: f64,
    max_stage_regression: f64,
) -> Result<PromotionReport, String> {
    assess_with_t(candidate, incumbent, min_gain, max_stage_regression, 0.0)
}

/// [`assess`] that also requires the paired t statistic of the per-seed gains
/// to reach `min_t`. Zero disables the requirement. With few pairs the majority
/// rule can be passed by luck; this ties acceptance to the observed spread.
pub fn assess_with_t(
    candidate: Vec<PlanReport>,
    incumbent: Vec<PlanReport>,
    min_gain: f64,
    max_stage_regression: f64,
    min_t: f64,
) -> Result<PromotionReport, String> {
    if !min_t.is_finite() || min_t < 0.0 {
        return Err("promotion t threshold must be finite and nonnegative".into());
    }
    if candidate.is_empty() || candidate.len() != incumbent.len() || candidate.len() > 64 {
        return Err("paired promotion requires matching dimensions and 1..64 seeds".into());
    }
    if !min_gain.is_finite()
        || min_gain < 0.0
        || !max_stage_regression.is_finite()
        || max_stage_regression < 0.0
    {
        return Err("promotion gain and regression slack must be finite and nonnegative".into());
    }
    let reference = &candidate[0].stages;
    if reference.is_empty() || reference.len() > 64 {
        return Err("paired promotion requires 1..64 stages".into());
    }
    for plan in candidate.iter().chain(&incumbent) {
        if !plan.fitness.is_finite() || plan.stages.len() != reference.len() {
            return Err("paired promotion scores or stage dimensions are invalid".into());
        }
        for (stage, expected) in plan.stages.iter().zip(reference) {
            if stage.name != expected.name
                || stage.difficulty != expected.difficulty
                || stage.weight != expected.weight
            {
                return Err("paired promotion stage metadata differ between samples".into());
            }
        }
        let recomputed = aggregate(plan.stages.clone())?.fitness;
        if (recomputed - plan.fitness).abs()
            > 1e-10 * recomputed.abs().max(plan.fitness.abs()).max(1.0)
        {
            return Err(
                "paired promotion plan score differs from its weighted stage scores".into(),
            );
        }
    }
    let gains: Vec<_> = candidate
        .iter()
        .zip(&incumbent)
        .map(|(a, b)| a.fitness - b.fitness)
        .collect();
    let mean_gain = mean(gains.iter().copied())?;
    let wins = gains.iter().filter(|gain| **gain > 0.0).count();
    let losses = gains.iter().filter(|gain| **gain < 0.0).count();
    let ties = gains.len() - wins - losses;
    let t_statistic = paired_t(&gains, mean_gain);
    let mut stages_protected = true;
    let candidate_families = candidate
        .iter()
        .map(super::campaign_eval::family_scores)
        .collect::<Result<Vec<_>, _>>()?;
    let incumbent_families = incumbent
        .iter()
        .map(super::campaign_eval::family_scores)
        .collect::<Result<Vec<_>, _>>()?;
    let family_keys = candidate_families[0].keys().collect::<Vec<_>>();
    if candidate_families
        .iter()
        .chain(&incumbent_families)
        .any(|f| f.keys().collect::<Vec<_>>() != family_keys)
    {
        return Err("paired promotion family coverage differs".into());
    }
    for key in family_keys {
        let current = mean(candidate_families.iter().map(|f| f[key]))?;
        let previous = mean(incumbent_families.iter().map(|f| f[key]))?;
        if current < previous - max_stage_regression {
            stages_protected = false;
        }
    }
    for index in 0..reference.len() {
        let current = mean(candidate.iter().map(|plan| plan.stages[index].fitness))?;
        let previous = mean(incumbent.iter().map(|plan| plan.stages[index].fitness))?;
        if current < previous - max_stage_regression {
            stages_protected = false;
        }
    }
    Ok(PromotionReport {
        candidate_fitness: mean(candidate.iter().map(|plan| plan.fitness))?,
        incumbent_fitness: mean(incumbent.iter().map(|plan| plan.fitness))?,
        mean_gain,
        wins,
        losses,
        ties,
        seeds: candidate.len(),
        t_statistic,
        accepted: mean_gain > min_gain
            && wins > candidate.len() / 2
            && stages_protected
            && (min_t == 0.0 || t_statistic >= min_t),
        candidate,
        incumbent,
    })
}

/// Mean over standard error. A single pair has no spread, so it reports zero
/// (and fails any positive threshold); identical positive gains are infinitely
/// certain, capped so the report stays finite JSON.
fn paired_t(gains: &[f64], mean_gain: f64) -> f64 {
    let n = gains.len();
    if n < 2 {
        return 0.0;
    }
    let variance = gains.iter().map(|g| (g - mean_gain).powi(2)).sum::<f64>() / (n - 1) as f64;
    let error = (variance / n as f64).sqrt();
    if error <= 1e-12 {
        return if mean_gain > 0.0 { 1e6 } else { 0.0 };
    }
    (mean_gain / error).clamp(-1e6, 1e6)
}

/// Fresh promotion seeds occupy a separate high band. Each sample reserves
/// 100 million seeds (64 stages use at most 64 million), and each generation
/// reserves 10 billion, keeping all 64 samples disjoint.
pub fn seed_base(generation: u64, sample_index: usize) -> Result<u64, String> {
    if sample_index >= 64 {
        return Err("promotion sample index must be below 64".into());
    }
    generation
        .checked_mul(10_000_000_000)
        .and_then(|offset| (1u64 << 61).checked_add(offset))
        .and_then(|base| base.checked_add(sample_index as u64 * 100_000_000))
        .filter(|base| base.checked_add(64_000_000).is_some())
        .ok_or_else(|| "promotion seed band overflows u64".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::campaign_eval::{StageReport, aggregate};

    fn plan(easy: f64, hard: f64) -> PlanReport {
        aggregate(vec![
            StageReport {
                name: "easy".into(),
                difficulty: 1,
                weight: 1.0,
                fitness: easy,
                details: serde_json::Value::Null,
            },
            StageReport {
                name: "hard".into(),
                difficulty: 2,
                weight: 1.0,
                fitness: hard,
                details: serde_json::Value::Null,
            },
        ])
        .unwrap()
    }

    #[test]
    fn paired_positive_majority_and_stage_protection_are_required() {
        let accepted = assess(
            vec![plan(3.0, 3.0), plan(2.2, 2.2), plan(1.9, 1.9)],
            vec![plan(2.0, 2.0); 3],
            0.1,
            0.0,
        )
        .unwrap();
        assert!(accepted.accepted);
        assert_eq!(
            (
                accepted.wins,
                accepted.losses,
                accepted.ties,
                accepted.seeds
            ),
            (2, 1, 0, 3)
        );
        let noise = assess(
            vec![
                plan(3.0, 3.0),
                plan(1.9, 1.9),
                plan(1.9, 1.9),
                plan(1.9, 1.9),
            ],
            vec![plan(2.0, 2.0); 4],
            0.1,
            0.0,
        )
        .unwrap();
        assert!(noise.mean_gain > 0.1);
        assert!(!noise.accepted);
        let regression =
            assess(vec![plan(1.9, 4.0); 3], vec![plan(2.0, 2.0); 3], 0.1, 0.05).unwrap();
        assert!(!regression.accepted);
        assert!(
            !assess(vec![plan(2.5, 2.5)], vec![plan(2.0, 2.0)], 0.5, 0.0)
                .unwrap()
                .accepted
        );
        assert!(
            !assess(
                vec![plan(3.0, 3.0), plan(2.0, 2.0)],
                vec![plan(2.0, 2.0); 2],
                0.0,
                0.0
            )
            .unwrap()
            .accepted
        );
    }

    #[test]
    fn t_threshold_rejects_a_lucky_majority_but_accepts_a_consistent_gain() {
        // 3 of 5 pairs win, but one huge loss leaves the mean barely positive
        // and the spread huge: passes the old rule, fails t >= 2.
        let lucky = vec![
            plan(2.5, 2.5),
            plan(2.5, 2.5),
            plan(2.5, 2.5),
            plan(1.2, 1.2),
            plan(1.5, 1.5),
        ];
        let base = vec![plan(2.0, 2.0); 5];
        let old = assess(lucky.clone(), base.clone(), 0.0, 10.0).unwrap();
        assert!(old.accepted);
        let strict = assess_with_t(lucky, base.clone(), 0.0, 10.0, 2.0).unwrap();
        assert!(strict.t_statistic.is_finite() && strict.t_statistic < 2.0);
        assert!(!strict.accepted);
        let steady: Vec<_> = [2.3, 2.4, 2.3, 2.5, 2.4]
            .iter()
            .map(|v| plan(*v, *v))
            .collect();
        let accepted = assess_with_t(steady, base, 0.0, 10.0, 2.0).unwrap();
        assert!(accepted.t_statistic > 2.0 && accepted.accepted);
        // One pair has no spread: a positive threshold cannot be met.
        let single =
            assess_with_t(vec![plan(3.0, 3.0)], vec![plan(2.0, 2.0)], 0.0, 10.0, 0.1).unwrap();
        assert_eq!(single.t_statistic, 0.0);
        assert!(!single.accepted);
        assert!(assess_with_t(vec![plan(1.0, 1.0)], vec![plan(1.0, 1.0)], 0.0, 0.0, -1.0).is_err());
    }

    #[test]
    fn inconsistent_or_nonfinite_pairs_are_errors() {
        assert!(assess(vec![], vec![], 0.0, 0.0).is_err());
        assert!(assess(vec![plan(1.0, 1.0)], vec![], 0.0, 0.0).is_err());
        let mut invalid = plan(1.0, 1.0);
        invalid.fitness = f64::NAN;
        assert!(assess(vec![invalid], vec![plan(1.0, 1.0)], 0.0, 0.0).is_err());
        let mut invalid = plan(1.0, 1.0);
        invalid.stages[1].name = "other".into();
        assert!(assess(vec![invalid], vec![plan(1.0, 1.0)], 0.0, 0.0).is_err());
        let mut invalid = plan(1.0, 1.0);
        invalid.stages[0].fitness = f64::INFINITY;
        assert!(assess(vec![invalid], vec![plan(1.0, 1.0)], 0.0, 0.0).is_err());
        let mut invalid = plan(1.0, 1.0);
        invalid.fitness = 2.0;
        assert!(assess(vec![invalid], vec![plan(1.0, 1.0)], 0.0, 0.0).is_err());
        let mut invalid = plan(1.0, 1.0);
        invalid.stages.pop();
        assert!(assess(vec![invalid], vec![plan(1.0, 1.0)], 0.0, 0.0).is_err());
        for (gain, slack) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (-1.0, 0.0),
            (0.0, -1.0),
        ] {
            assert!(assess(vec![plan(1.0, 1.0)], vec![plan(1.0, 1.0)], gain, slack).is_err());
        }
    }

    #[test]
    fn seed_bands_are_fresh_disjoint_and_checked() {
        let first = seed_base(0, 0).unwrap();
        assert!(first >= 1 << 61);
        assert_eq!(first, seed_base(0, 0).unwrap());
        assert!(seed_base(0, 1).unwrap() > first + 64 * 1_000_000);
        assert!(seed_base(1, 0).unwrap() > seed_base(0, 63).unwrap() + 64 * 1_000_000);
        assert!(seed_base(0, 64).is_err());
        assert!(seed_base(u64::MAX, 0).is_err());
    }
}
