//! Antithetic evolution strategies with rank-shaped fitness and Adam.
//!
//! Generic over the fitness function so any council (tactical today, strategic
//! later) can be trained by it. Gradient-free: the simulator is a black box.

use super::rng::Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EsConfig {
    /// Candidates per generation; must be even (antithetic pairs).
    pub population: usize,
    /// Standard deviation of the parameter noise.
    pub sigma: f32,
    pub learning_rate: f32,
    /// Decoupled L2 shrinkage per step (scaled by the learning rate).
    pub weight_decay: f32,
    /// L1 shrinkage per step (scaled by the learning rate). Pushes unused
    /// weights to exactly zero, which keeps modules readable.
    pub l1: f32,
    pub seed: u64,
}

impl Default for EsConfig {
    fn default() -> Self {
        Self {
            population: 64,
            sigma: 0.1,
            learning_rate: 0.03,
            weight_decay: 0.0,
            l1: 0.0,
            seed: 1,
        }
    }
}

impl EsConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.population < 2 || self.population % 2 != 0 {
            return Err("population must be an even number of at least 2".into());
        }
        let ok = |v: f32| v.is_finite() && v >= 0.0;
        if !(self.sigma.is_finite() && self.sigma > 0.0)
            || !(self.learning_rate.is_finite() && self.learning_rate > 0.0)
            || !ok(self.weight_decay)
            || !ok(self.l1)
        {
            return Err(
                "sigma and learning_rate must be positive; decay and l1 non-negative".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerationStats {
    pub generation: u64,
    pub fitness_mean: f64,
    pub fitness_best: f64,
    pub fitness_worst: f64,
    /// Euclidean size of the parameter change this generation.
    pub update_norm: f64,
}

/// Checkpoint format for exact continuation, including Adam and sampling state.
pub const ES_STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EsState {
    pub version: u32,
    pub config: EsConfig,
    pub theta: Vec<f32>,
    pub first_moment: Vec<f32>,
    pub second_moment: Vec<f32>,
    pub generation: u64,
    /// Adam updates since moments were reset. Absent in legacy checkpoints,
    /// where the global generation was also the bias-correction clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adam_updates: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct EvolutionStrategy {
    config: EsConfig,
    theta: Vec<f32>,
    first_moment: Vec<f32>,
    second_moment: Vec<f32>,
    generation: u64,
    adam_updates: u64,
}

const BETA1: f32 = 0.9;
const BETA2: f32 = 0.999;
const EPSILON: f32 = 1e-8;

#[cfg(test)]
mod masked_tests {
    use super::*;
    #[test]
    fn dormant_parameters_and_moments_survive_regularization() {
        let config = EsConfig {
            population: 2,
            weight_decay: 1.0,
            l1: 1.0,
            ..Default::default()
        };
        let mut state = EvolutionStrategy::new(vec![2.0, 3.0], config)
            .unwrap()
            .state();
        state.first_moment[1] = 0.5;
        state.second_moment[1] = 0.5;
        let before = state.clone();
        let mut es = EvolutionStrategy::from_state(state).unwrap();
        assert!(es.ask_masked(&[true]).is_err());
        for candidate in es.ask_masked(&[true, false]).unwrap() {
            assert_eq!(candidate[1], 3.0);
        }
        es.tell_masked(&[1.0, 0.0], &[true, false]).unwrap();
        let after = es.state();
        assert_ne!(after.theta[0], before.theta[0]);
        assert_eq!(after.theta[1], before.theta[1]);
        assert_eq!(after.first_moment[1], before.first_moment[1]);
        assert_eq!(after.second_moment[1], before.second_moment[1]);
    }
}

impl EvolutionStrategy {
    pub fn new(theta: Vec<f32>, config: EsConfig) -> Result<Self, String> {
        config.validate()?;
        if theta.is_empty() {
            return Err("nothing to train: zero trainable parameters".into());
        }
        if theta.iter().any(|value| !value.is_finite()) {
            return Err("trainable parameters must all be finite".into());
        }
        let len = theta.len();
        Ok(Self {
            config,
            theta,
            first_moment: vec![0.0; len],
            second_moment: vec![0.0; len],
            generation: 0,
            adam_updates: 0,
        })
    }

    /// Snapshot with enough state to reproduce the next population and update.
    pub fn state(&self) -> EsState {
        EsState {
            version: ES_STATE_VERSION,
            config: self.config,
            theta: self.theta.clone(),
            first_moment: self.first_moment.clone(),
            second_moment: self.second_moment.clone(),
            generation: self.generation,
            adam_updates: Some(self.adam_updates),
        }
    }

    pub fn from_state(state: EsState) -> Result<Self, String> {
        if state.version != ES_STATE_VERSION {
            return Err(format!(
                "unsupported ES state version {}; expected {ES_STATE_VERSION}",
                state.version
            ));
        }
        state.config.validate()?;
        let len = state.theta.len();
        if len == 0 || state.first_moment.len() != len || state.second_moment.len() != len {
            return Err("ES state needs nonempty equal parameter and moment dimensions".into());
        }
        if state
            .theta
            .iter()
            .chain(&state.first_moment)
            .chain(&state.second_moment)
            .any(|value| !value.is_finite())
        {
            return Err("ES state parameters and moments must all be finite".into());
        }
        if state.second_moment.iter().any(|value| *value < 0.0) {
            return Err("ES second moments must be nonnegative".into());
        }
        let adam_updates = state.adam_updates.unwrap_or(state.generation);
        if adam_updates > state.generation {
            return Err("ES Adam update count cannot exceed generation".into());
        }
        Ok(Self {
            config: state.config,
            theta: state.theta,
            first_moment: state.first_moment,
            second_moment: state.second_moment,
            generation: state.generation,
            adam_updates,
        })
    }

    fn noise(&self) -> Vec<Vec<f32>> {
        (0..self.config.population / 2)
            .map(|pair| {
                let mut rng =
                    Rng::for_stream(self.config.seed, (self.generation << 32) | pair as u64);
                (0..self.theta.len()).map(|_| rng.gaussian()).collect()
            })
            .collect()
    }

    /// Deterministic candidate order: positive then negative perturbation per pair.
    /// Calling repeatedly does not advance the optimizer. Keep these indices when
    /// collecting external results; `tell` consumes scores in this same order.
    pub fn ask(&self) -> Vec<Vec<f32>> {
        let noise = self.noise();
        (0..self.config.population)
            .map(|candidate| {
                let sign = if candidate % 2 == 0 { 1.0 } else { -1.0 };
                self.theta
                    .iter()
                    .zip(&noise[candidate / 2])
                    .map(|(t, e)| t + sign * self.config.sigma * e)
                    .collect()
            })
            .collect()
    }

    pub fn ask_masked(&self, active: &[bool]) -> Result<Vec<Vec<f32>>, String> {
        if active.len() != self.theta.len() {
            return Err("parameter activity length mismatch".into());
        }
        let mut candidates = self.ask();
        for candidate in &mut candidates {
            for (i, on) in active.iter().enumerate() {
                if !on {
                    candidate[i] = self.theta[i];
                }
            }
        }
        Ok(candidates)
    }

    pub fn theta(&self) -> &[f32] {
        &self.theta
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Evaluates one antithetic population in parallel and moves `theta`
    /// uphill. `fitness` must be pure with respect to its argument.
    pub fn step<F>(&mut self, fitness: F) -> GenerationStats
    where
        F: Fn(&[f32]) -> f64 + Sync,
    {
        self.step_masked(&vec![true; self.theta.len()], fitness)
            .expect("ES fitness and update must be finite")
    }

    pub fn step_masked<F>(&mut self, active: &[bool], fitness: F) -> Result<GenerationStats, String>
    where
        F: Fn(&[f32]) -> f64 + Sync,
    {
        let candidates = self.ask_masked(active)?;
        let scores: Vec<_> = candidates
            .par_iter()
            .map(|candidate| fitness(candidate))
            .collect();
        self.tell_masked(&scores, active)
    }

    /// Applies one complete externally evaluated population. Invalid results or
    /// numerical updates return an error without changing any optimizer state.
    pub fn tell(&mut self, scores: &[f64]) -> Result<GenerationStats, String> {
        self.tell_masked(scores, &vec![true; self.theta.len()])
    }

    pub fn tell_masked(
        &mut self,
        scores: &[f64],
        active: &[bool],
    ) -> Result<GenerationStats, String> {
        if active.len() != self.theta.len() {
            return Err("parameter activity length mismatch".into());
        }
        if scores.len() != self.config.population {
            return Err(format!(
                "expected {} ES scores, found {}",
                self.config.population,
                scores.len()
            ));
        }
        if scores.iter().any(|score| !score.is_finite()) {
            return Err("ES scores must all be finite".into());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("ES generation overflow")?;
        let adam_updates = self
            .adam_updates
            .checked_add(1)
            .ok_or("ES Adam update count overflow")?;
        let t = i32::try_from(adam_updates)
            .map_err(|_| "ES Adam update count exceeds exponent range")?;
        let noise = self.noise();
        let sigma = self.config.sigma;
        let len = self.theta.len();
        let shaped = centered_ranks(&scores);
        let n = scores.len() as f32;
        let mut gradient = vec![0.0f32; len];
        for (pair, eps) in noise.iter().enumerate() {
            let weight = (shaped[2 * pair] - shaped[2 * pair + 1]) as f32;
            if weight != 0.0 {
                for (g, e) in gradient.iter_mut().zip(eps) {
                    *g += weight * e;
                }
            }
        }
        gradient.iter_mut().for_each(|g| *g /= n * sigma);

        let mut theta = self.theta.clone();
        let mut first_moment = self.first_moment.clone();
        let mut second_moment = self.second_moment.clone();
        let (bias1, bias2) = (1.0 - BETA1.powi(t), 1.0 - BETA2.powi(t));
        let lr = self.config.learning_rate;
        let mut update_sq = 0.0f64;
        for i in 0..len {
            if !active[i] {
                continue;
            }
            let g = gradient[i];
            first_moment[i] = BETA1 * first_moment[i] + (1.0 - BETA1) * g;
            second_moment[i] = BETA2 * second_moment[i] + (1.0 - BETA2) * g * g;
            let step =
                lr * (first_moment[i] / bias1) / ((second_moment[i] / bias2).sqrt() + EPSILON);
            let before = theta[i];
            let mut value = before + step - lr * self.config.weight_decay * before;
            if self.config.l1 > 0.0 {
                let shrink = lr * self.config.l1;
                value = value.signum() * (value.abs() - shrink).max(0.0);
            }
            theta[i] = value;
            update_sq += ((value - before) as f64).powi(2);
        }

        if theta
            .iter()
            .chain(&first_moment)
            .chain(&second_moment)
            .any(|value| !value.is_finite())
        {
            return Err("ES update produced nonfinite parameters or moments".into());
        }
        self.theta = theta;
        self.first_moment = first_moment;
        self.second_moment = second_moment;
        self.generation = generation;
        self.adam_updates = adam_updates;
        Ok(GenerationStats {
            generation,
            fitness_mean: scores.iter().sum::<f64>() / scores.len() as f64,
            fitness_best: scores.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            fitness_worst: scores.iter().copied().fold(f64::INFINITY, f64::min),
            update_norm: update_sq.sqrt(),
        })
    }
}

/// Maps scores to `[-0.5, 0.5]` by rank, giving tied scores their average
/// rank. Many early generations tie (everyone draws); ties must carry no signal.
fn centered_ranks(scores: &[f64]) -> Vec<f64> {
    let n = scores.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| scores[a].total_cmp(&scores[b]));
    let mut ranks = vec![0.0; n];
    let mut start = 0;
    while start < n {
        let mut end = start;
        while end + 1 < n && scores[order[end + 1]] == scores[order[start]] {
            end += 1;
        }
        let average = (start + end) as f64 / 2.0;
        for &index in &order[start..=end] {
            ranks[index] = average;
        }
        start = end + 1;
    }
    let denominator = (n - 1).max(1) as f64;
    ranks.iter().map(|r| r / denominator - 0.5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_share_a_rank_so_a_flat_landscape_gives_no_signal() {
        assert_eq!(centered_ranks(&[1.0, 1.0, 1.0, 1.0]), vec![0.0; 4]);
        let ranks = centered_ranks(&[3.0, 1.0, 2.0, 2.0]);
        assert_eq!(ranks[0], 0.5);
        assert_eq!(ranks[1], -0.5);
        assert_eq!(ranks[2], ranks[3]);
        assert_eq!(ranks[2], 0.0);
    }

    #[test]
    fn climbs_a_quadratic_hill() {
        let target = [0.8f32, -0.5, 0.3, 0.0, 1.2];
        let fitness = |p: &[f32]| {
            -p.iter()
                .zip(&target)
                .map(|(a, b)| ((a - b) as f64).powi(2))
                .sum::<f64>()
        };
        let mut es = EvolutionStrategy::new(
            vec![0.0; 5],
            EsConfig {
                population: 40,
                sigma: 0.1,
                learning_rate: 0.05,
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..300 {
            es.step(fitness);
        }
        for (p, t) in es.theta().iter().zip(&target) {
            assert!((p - t).abs() < 0.1, "{:?}", es.theta());
        }
    }

    #[test]
    fn identical_seeds_give_identical_trajectories() {
        let fitness = |p: &[f32]| -(p[0] as f64 - 1.0).powi(2) - (p[1] as f64).powi(2);
        let run = || {
            let mut es = EvolutionStrategy::new(vec![0.0, 0.5], EsConfig::default()).unwrap();
            for _ in 0..10 {
                es.step(fitness);
            }
            es.theta().to_vec()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn l1_zeroes_parameters_that_do_not_matter() {
        // Only the first parameter affects fitness.
        let fitness = |p: &[f32]| -(p[0] as f64 - 0.7).powi(2);
        let mut es = EvolutionStrategy::new(
            vec![0.0, 0.3, -0.3],
            EsConfig {
                population: 32,
                learning_rate: 0.05,
                l1: 0.3,
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..200 {
            es.step(fitness);
        }
        assert!(es.theta()[0] > 0.4, "{:?}", es.theta());
        assert!(
            es.theta()[1].abs() < 0.06 && es.theta()[2].abs() < 0.06,
            "{:?}",
            es.theta()
        );
    }

    #[test]
    fn bad_configs_are_rejected() {
        assert!(
            EvolutionStrategy::new(
                vec![0.0],
                EsConfig {
                    population: 3,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(EvolutionStrategy::new(vec![], EsConfig::default()).is_err());
        assert!(
            EvolutionStrategy::new(
                vec![0.0],
                EsConfig {
                    sigma: 0.0,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn seeded_step_legacy_trajectory() {
        let mut es = EvolutionStrategy::new(
            vec![0.0, 0.5, -0.25],
            EsConfig {
                population: 8,
                seed: 19,
                weight_decay: 0.02,
                l1: 0.01,
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..7 {
            es.step(|p| {
                -(p[0] as f64 - 0.8).powi(2) - (p[1] as f64 + 0.2).powi(2) - (p[2] as f64).powi(2)
            });
        }
        assert_eq!(
            es.theta.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            vec![1045181946, 1054747823, 3189203073]
        );
        assert_eq!(
            es.first_moment
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            vec![1067986887, 3190203058, 1040699804]
        );
        assert_eq!(
            es.second_moment
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            vec![1029458372, 1002727628, 1007024805]
        );
    }

    fn external_fitness(p: &[f32]) -> f64 {
        -(p[0] as f64 - 0.8).powi(2) - (p[1] as f64 + 0.2).powi(2)
    }

    #[test]
    fn reset_adam_uses_first_update_bias_with_the_existing_noise_stream() {
        let mut state = EvolutionStrategy::new(
            vec![0.0],
            EsConfig {
                population: 2,
                ..Default::default()
            },
        )
        .unwrap()
        .state();
        state.generation = 70;
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded["adam_updates"] = serde_json::json!(0);
        let mut reset =
            EvolutionStrategy::from_state(serde_json::from_value(encoded).unwrap()).unwrap();
        let candidates = reset.ask();
        // With two ranked candidates the gradient is epsilon / (2 * sigma).
        let gradient = candidates[0][0] / (2.0 * state.config.sigma * state.config.sigma);
        let expected = state.config.learning_rate * gradient / (gradient.abs() + EPSILON);
        reset.tell(&[1.0, 0.0]).unwrap();
        assert!((reset.theta()[0] - expected).abs() < 1e-6);
        assert_eq!(reset.generation(), 71);
        assert_eq!(reset.state().adam_updates, Some(1));
        // Reset checkpoints must retain both distinct clocks on exact resume.
        let encoded = serde_json::to_string(&reset.state()).unwrap();
        let mut resumed =
            EvolutionStrategy::from_state(serde_json::from_str(&encoded).unwrap()).unwrap();
        assert_eq!(resumed.ask(), reset.ask());
        for _ in 0..3 {
            assert_eq!(resumed.tell(&[0.0, 1.0]), reset.tell(&[0.0, 1.0]));
            assert_eq!(resumed.state(), reset.state());
        }
    }

    #[test]
    fn legacy_checkpoint_without_adam_clock_preserves_exact_continuation() {
        let mut original = EvolutionStrategy::new(vec![0.0, 0.5], EsConfig::default()).unwrap();
        for _ in 0..4 {
            original.step(external_fitness);
        }
        let mut legacy = serde_json::to_value(original.state()).unwrap();
        legacy.as_object_mut().unwrap().remove("adam_updates");
        let legacy: EsState = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.adam_updates, None);
        let mut resumed = EvolutionStrategy::from_state(legacy).unwrap();
        assert_eq!(resumed.ask(), original.ask());
        for _ in 0..3 {
            assert_eq!(
                resumed.step(external_fitness),
                original.step(external_fitness)
            );
            assert_eq!(resumed.state(), original.state());
        }
    }

    #[test]
    fn invalid_adam_clock_update_does_not_change_optimizer() {
        let mut state = EvolutionStrategy::new(
            vec![0.0],
            EsConfig {
                population: 2,
                ..Default::default()
            },
        )
        .unwrap()
        .state();
        state.generation = i32::MAX as u64;
        state.adam_updates = Some(i32::MAX as u64);
        let mut es = EvolutionStrategy::from_state(state).unwrap();
        let before = es.state();
        assert!(es.tell(&[1.0, 0.0]).is_err());
        assert_eq!(es.state(), before);
        // A reset clock can continue even when the noise stream exceeds that range.
        let mut state = before;
        state.adam_updates = Some(0);
        let mut es = EvolutionStrategy::from_state(state).unwrap();
        assert!(es.tell(&[1.0, 0.0]).is_ok());
        assert_eq!(es.state().adam_updates, Some(1));
    }

    #[test]
    fn json_state_resumes_the_exact_population_and_adam_trajectory() {
        let mut uninterrupted = EvolutionStrategy::new(
            vec![0.0, 0.5],
            EsConfig {
                population: 8,
                seed: 27,
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..4 {
            uninterrupted.step(external_fitness);
        }
        let encoded = serde_json::to_string(&uninterrupted.state()).unwrap();
        let mut resumed =
            EvolutionStrategy::from_state(serde_json::from_str(&encoded).unwrap()).unwrap();
        assert_eq!(uninterrupted.ask(), resumed.ask());
        for _ in 0..6 {
            assert_eq!(
                uninterrupted.step(external_fitness),
                resumed.step(external_fitness)
            );
            assert_eq!(uninterrupted.state(), resumed.state());
        }
    }

    #[test]
    fn caller_can_reorder_external_results_before_tell() {
        let mut direct = EvolutionStrategy::new(
            vec![0.0, 0.5],
            EsConfig {
                population: 8,
                ..Default::default()
            },
        )
        .unwrap();
        let mut external = EvolutionStrategy::from_state(direct.state()).unwrap();
        for _ in 0..5 {
            let before = external.state();
            let candidates = external.ask();
            assert_eq!(candidates, external.ask());
            assert_eq!(before, external.state());
            let mut results: Vec<_> = candidates
                .iter()
                .enumerate()
                .rev()
                .map(|(id, p)| (id, external_fitness(p)))
                .collect();
            results.sort_by_key(|(id, _)| *id);
            let scores: Vec<_> = results.into_iter().map(|(_, score)| score).collect();
            assert_eq!(
                external.tell(&scores).unwrap(),
                direct.step(external_fitness)
            );
            assert_eq!(external.state(), direct.state());
        }
    }

    #[test]
    fn bad_tell_results_do_not_mutate_optimizer() {
        let mut es = EvolutionStrategy::new(
            vec![0.0, 0.5],
            EsConfig {
                population: 4,
                ..Default::default()
            },
        )
        .unwrap();
        es.step(external_fitness);
        let before = es.state();
        for scores in [
            vec![],
            vec![1.0; 3],
            vec![1.0; 5],
            vec![1.0, 2.0, f64::NAN, 3.0],
            vec![f64::INFINITY; 4],
            vec![f64::NEG_INFINITY; 4],
        ] {
            assert!(es.tell(&scores).is_err());
            assert_eq!(es.state(), before);
        }
    }

    #[test]
    fn invalid_state_is_rejected_without_changing_its_source() {
        let es = EvolutionStrategy::new(vec![0.0, 0.5], EsConfig::default()).unwrap();
        let before = es.state();
        let mut invalid = Vec::new();
        let mut state = before.clone();
        state.version += 1;
        invalid.push(state);
        let mut state = before.clone();
        state.config.population = 3;
        invalid.push(state);
        let mut state = before.clone();
        state.theta.clear();
        invalid.push(state);
        let mut state = before.clone();
        state.first_moment.pop();
        invalid.push(state);
        let mut state = before.clone();
        state.second_moment.push(0.0);
        invalid.push(state);
        let mut state = before.clone();
        state.theta[0] = f32::NAN;
        invalid.push(state);
        let mut state = before.clone();
        state.first_moment[0] = f32::INFINITY;
        invalid.push(state);
        let mut state = before.clone();
        state.second_moment[0] = f32::NAN;
        invalid.push(state);
        let mut state = before.clone();
        state.second_moment[0] = -0.1;
        invalid.push(state);
        let mut state = before.clone();
        state.adam_updates = Some(state.generation + 1);
        invalid.push(state);
        for state in invalid {
            assert!(EvolutionStrategy::from_state(state).is_err());
            assert_eq!(es.state(), before);
        }
        assert!(EvolutionStrategy::new(vec![f32::NAN], EsConfig::default()).is_err());
    }
}
