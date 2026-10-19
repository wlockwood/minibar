//! The training objective, stated in one place.
//!
//! For one battle, from the learner's side:
//!
//! ```text
//! fitness = win_weight      · outcome · stake
//!         + exchange_weight · exchange
//! ```
//!
//! * `outcome` is +1 for a win and −1 for a loss. A *timeout* (both sides
//!   still alive when time runs out) is a partial result: `+timeout_underdog`
//!   when the learner was the underdog (surviving a stronger force is a partial
//!   win), otherwise `−timeout_penalty` (failing to beat an equal or weaker
//!   force is a partial loss). A mutual wipe-out scores 0; the exchange term
//!   judges it.
//! * `stake = clamp(ratio, lower, cap)^exponent`, where `ratio` is the
//!   *opponent's* starting value over the learner's for a win, and the
//!   learner's over the opponent's for a loss. Beating a stronger force pays
//!   more than beating an equal one, and losing to a weaker force costs more
//!   than losing to an equal one. Losing as the underdog is cheap; it is
//!   never rewarded. `lower` is `win_stake_floor` for a win, so a favourite's
//!   win is never discounted (default 1: beating a weaker force pays as much
//!   as an even fight), and `1/cap` otherwise.
//! * `key_term` rewards destroying the opponent's key units (a commander) and
//!   charges for losing our own: `key_unit_bonus * enemy_key_fraction_lost -
//!   key_unit_penalty * own_key_fraction_lost`. A tactical-only score can
//!   teach a policy to flee a commander whose death explosion kills its
//!   attackers; this term makes that trade visibly worth taking when it is.
//! * `exchange` is the fraction of the combined starting value that the
//!   opponent lost (HP-weighted) minus what the learner lost. HP-weighting
//!   keeps a signal alive before the first kill, which evolution needs.

use crate::score::{BattleResult, wilson_interval};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ObjectiveWeights {
    pub win: f64,
    pub exchange: f64,
    /// 0 treats every win alike; 1 makes a 2× stronger opponent worth 2×.
    pub underdog_exponent: f64,
    /// Largest value ratio that still increases the stake.
    pub underdog_cap: f64,
    /// Opponent/learner value ratio at or above which a battle is reported as
    /// an "underdog" battle.
    pub underdog_threshold: f64,
    /// Fraction of a win awarded for surviving to the time limit as the
    /// underdog.
    pub timeout_underdog: f64,
    /// Fraction of a loss charged for a timeout when not the underdog.
    /// Stalling against a force you outnumber is as bad as losing a fair
    /// fight at the default.
    pub timeout_penalty: f64,
    /// Smallest stake a win can have. 1.0 means beating a weaker force pays
    /// the same as an even fight; 0 restores the old discount for favourites.
    pub win_stake_floor: f64,
    /// Weight of the worst quarter of (scenario, opponent) rows in the
    /// training score: `(1 - w) * mean + w * mean_of_worst_quarter`. Stops the
    /// optimiser trading one scenario (say a favourite fight) for the rest.
    pub worst_case_weight: f64,
    /// Reward for destroying every enemy key unit (scaled by the fraction
    /// destroyed). Zero effect when no unit is flagged as a key unit.
    pub key_unit_bonus: f64,
    /// Charge for losing every one of our own key units (scaled likewise).
    pub key_unit_penalty: f64,
}

impl Default for ObjectiveWeights {
    fn default() -> Self {
        Self {
            win: 1.0,
            exchange: 0.5,
            underdog_exponent: 1.0,
            underdog_cap: 4.0,
            underdog_threshold: 1.2,
            timeout_underdog: 0.3,
            timeout_penalty: 0.6,
            win_stake_floor: 1.0,
            worst_case_weight: 0.3,
            key_unit_bonus: 0.5,
            key_unit_penalty: 0.5,
        }
    }
}

impl ObjectiveWeights {
    pub fn validate(&self) -> Result<(), String> {
        let finite = [
            self.win,
            self.exchange,
            self.underdog_exponent,
            self.underdog_cap,
            self.underdog_threshold,
            self.timeout_underdog,
            self.timeout_penalty,
            self.win_stake_floor,
            self.worst_case_weight,
            self.key_unit_bonus,
            self.key_unit_penalty,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !finite
            || self.win < 0.0
            || self.exchange < 0.0
            || self.underdog_exponent < 0.0
            || self.underdog_cap < 1.0
            || self.underdog_threshold < 1.0
            || self.timeout_underdog > 1.0
            || self.timeout_penalty > 1.0
            || self.worst_case_weight > 1.0
            || self.win_stake_floor > self.underdog_cap
        {
            return Err(
                "objective weights must be finite, non-negative, with cap and \
                        threshold at least 1"
                    .into(),
            );
        }
        if self.win == 0.0 && self.exchange == 0.0 {
            return Err("objective needs a non-zero win or exchange weight".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Win,
    /// Both sides eliminated each other on the same tick.
    Draw,
    /// Time ran out with both sides alive.
    Timeout,
    Loss,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BattleAssessment {
    /// Learner then opponent, regardless of arena side.
    pub firing: [crate::score::FiringStats; 2],
    pub outcome: Outcome,
    /// Opponent starting value over the learner's.
    pub value_ratio: f64,
    pub stake: f64,
    pub exchange: f64,
    /// Contribution of key-unit kills and losses, already weighted.
    pub key_term: f64,
    pub fitness: f64,
    pub underdog: bool,
    /// Simulated length of the battle in seconds.
    pub duration: f64,
}

/// What a fight is for, when it is not simply to destroy the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveKind {
    /// The learner must destroy the enemy's target.
    Attack,
    /// The learner must keep its own target alive.
    Defend,
}

pub fn assess(
    result: &BattleResult,
    learner_side: usize,
    w: &ObjectiveWeights,
) -> BattleAssessment {
    assess_objective(result, learner_side, w, None)
}

/// [`assess`] for a fight that may carry an objective. The target is the
/// owner's key unit: for an attack, destroying it wins however many units
/// remain; for a defence, losing it loses, and holding it until time runs out
/// wins. Elimination still decides the fight otherwise.
pub fn assess_objective(
    result: &BattleResult,
    learner_side: usize,
    w: &ObjectiveWeights,
    objective: Option<ObjectiveKind>,
) -> BattleAssessment {
    let own = &result.sides[learner_side];
    let enemy = &result.sides[1 - learner_side];
    let (own_start, enemy_start) = (own.starting_value as f64, enemy.starting_value as f64);

    let destroyed = |score: &crate::score::SideScore| {
        score.key_units_started > 0 && score.key_units_surviving == 0
    };
    let by_elimination = match result.winner {
        Some(side) if side as usize == learner_side => Outcome::Win,
        Some(_) => Outcome::Loss,
        None if result.timed_out => Outcome::Timeout,
        None => Outcome::Draw,
    };
    let outcome = match objective {
        Some(ObjectiveKind::Attack) if destroyed(enemy) => Outcome::Win,
        Some(ObjectiveKind::Defend) if destroyed(own) => Outcome::Loss,
        Some(ObjectiveKind::Defend) if by_elimination == Outcome::Timeout => Outcome::Win,
        _ => by_elimination,
    };
    let value_ratio = if own_start > 0.0 {
        enemy_start / own_start
    } else {
        1.0
    };
    let underdog = value_ratio >= w.underdog_threshold;
    let stake_ratio = match outcome {
        Outcome::Win => value_ratio,
        Outcome::Loss => 1.0 / value_ratio.max(1e-9),
        Outcome::Timeout if underdog => value_ratio,
        Outcome::Timeout => 1.0 / value_ratio.max(1e-9),
        Outcome::Draw => 1.0,
    };
    let lower = match outcome {
        Outcome::Win => w.win_stake_floor.max(1.0 / w.underdog_cap),
        _ => 1.0 / w.underdog_cap,
    };
    let stake = stake_ratio
        .clamp(lower, w.underdog_cap)
        .powf(w.underdog_exponent);
    let signed_outcome = match outcome {
        Outcome::Win => 1.0,
        Outcome::Draw => 0.0,
        Outcome::Timeout if underdog => w.timeout_underdog,
        Outcome::Timeout => -w.timeout_penalty,
        Outcome::Loss => -1.0,
    };

    let total = own_start + enemy_start;
    let exchange = if total > 0.0 {
        let enemy_lost = enemy_start - enemy.remaining_hp_value as f64;
        let own_lost = own_start - own.remaining_hp_value as f64;
        (enemy_lost - own_lost) / total
    } else {
        0.0
    };

    let lost_fraction = |score: &crate::score::SideScore| {
        if score.key_units_started > 0 {
            let surviving = score.key_units_surviving.min(score.key_units_started);
            f64::from(score.key_units_started - surviving) / f64::from(score.key_units_started)
        } else {
            0.0
        }
    };
    let key_term =
        w.key_unit_bonus * lost_fraction(enemy) - w.key_unit_penalty * lost_fraction(own);

    BattleAssessment {
        firing: [own.firing, enemy.firing],
        outcome,
        value_ratio,
        stake,
        exchange,
        key_term,
        duration: f64::from(result.duration),
        fitness: w.win * signed_outcome * stake + w.exchange * exchange + key_term,
        underdog,
    }
}

/// The score training maximises: a blend of the mean over every
/// (scenario, opponent) row and the mean of the worst quarter of them.
pub fn robust_fitness<'a>(rows: impl Iterator<Item = &'a Tally>, w: &ObjectiveWeights) -> f64 {
    let mut means: Vec<f64> = rows
        .filter(|tally| tally.battles > 0)
        .map(Tally::mean_fitness)
        .collect();
    if means.is_empty() {
        return 0.0;
    }
    let n = means.len();
    let mean = means.iter().sum::<f64>() / n as f64;
    means.sort_by(f64::total_cmp);
    let worst_count = n.div_ceil(4).max(1);
    let worst = means[..worst_count].iter().sum::<f64>() / worst_count as f64;
    (1.0 - w.worst_case_weight) * mean + w.worst_case_weight * worst
}

/// Running totals over assessed battles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    /// None for legacy evidence without firing counters; sums of unit-battle
    /// participation and ready weapon checks otherwise. Learner then opponent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firing: Option<[crate::score::FiringStats; 2]>,
    pub battles: usize,
    pub wins: usize,
    /// Mutual wipe-outs.
    pub draws: usize,
    /// Time ran out with both sides alive.
    pub timeouts: usize,
    pub losses: usize,
    pub fitness_sum: f64,
    pub exchange_sum: f64,
    pub key_term_sum: f64,
    /// Total simulated battle time, in seconds.
    #[serde(default)]
    pub duration_sum: f64,
    pub underdog_battles: usize,
    pub underdog_wins: usize,
}

impl Tally {
    pub fn add(&mut self, a: &BattleAssessment) {
        let firing = self.firing.get_or_insert_with(Default::default);
        for (total, incoming) in firing.iter_mut().zip(a.firing.iter()) {
            total.merge(incoming);
        }
        self.battles += 1;
        self.fitness_sum += a.fitness;
        self.exchange_sum += a.exchange;
        self.key_term_sum += a.key_term;
        self.duration_sum += a.duration;
        match a.outcome {
            Outcome::Win => self.wins += 1,
            Outcome::Draw => self.draws += 1,
            Outcome::Timeout => self.timeouts += 1,
            Outcome::Loss => self.losses += 1,
        }
        if a.underdog {
            self.underdog_battles += 1;
            if a.outcome == Outcome::Win {
                self.underdog_wins += 1;
            }
        }
    }

    pub fn merge(&mut self, other: &Tally) {
        if let Some(incoming) = other.firing {
            let firing = self.firing.get_or_insert_with(Default::default);
            for (total, source) in firing.iter_mut().zip(incoming.iter()) {
                total.merge(source);
            }
        }
        self.battles += other.battles;
        self.wins += other.wins;
        self.draws += other.draws;
        self.timeouts += other.timeouts;
        self.losses += other.losses;
        self.fitness_sum += other.fitness_sum;
        self.exchange_sum += other.exchange_sum;
        self.key_term_sum += other.key_term_sum;
        self.duration_sum += other.duration_sum;
        self.underdog_battles += other.underdog_battles;
        self.underdog_wins += other.underdog_wins;
    }

    pub fn mean_fitness(&self) -> f64 {
        self.fitness_sum / self.battles.max(1) as f64
    }

    pub fn mean_exchange(&self) -> f64 {
        self.exchange_sum / self.battles.max(1) as f64
    }

    pub fn win_rate(&self) -> f64 {
        self.wins as f64 / self.battles.max(1) as f64
    }

    pub fn win_rate_95ci(&self) -> [f64; 2] {
        wilson_interval(self.wins, self.battles)
    }

    pub fn underdog_win_rate(&self) -> Option<f64> {
        (self.underdog_battles > 0)
            .then(|| self.underdog_wins as f64 / self.underdog_battles as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::SideScore;

    #[test]
    fn firing_diagnostics_follow_learner_side_and_pool_counts() {
        let mut battle = result(None, (100.0, 50.0), (100.0, 50.0));
        battle.sides[0].firing.armed_units = 40;
        battle.sides[0].firing.units_fired = 12;
        battle.sides[0].firing.ready_checks = 100;
        battle.sides[0].firing.blocked_friendly = 80;
        battle.sides[1].firing.armed_units = 1;
        battle.sides[1].firing.units_fired = 1;
        let assessment = assess_objective(&battle, 1, &ObjectiveWeights::default(), None);
        let mut a = Tally::default();
        a.add(&assessment);
        let b = a;
        a.merge(&b);
        let stats = a.firing.unwrap();
        assert_eq!(stats[0].armed_units, 2);
        assert_eq!(stats[1].units_fired, 24);
        assert_eq!(stats[1].blocked_friendly, 160);
        let mut old = serde_json::to_value(a).unwrap();
        old.as_object_mut().unwrap().remove("firing");
        assert!(
            serde_json::from_value::<Tally>(old)
                .unwrap()
                .firing
                .is_none()
        );
    }

    fn result(winner: Option<u8>, own: (f32, f32), enemy: (f32, f32)) -> BattleResult {
        // (starting value, remaining hp value)
        BattleResult {
            winner,
            timed_out: winner.is_none(),
            duration: 10.0,
            sides: [
                SideScore {
                    side: 0,
                    starting_value: own.0,
                    remaining_hp_value: own.1,
                    ..Default::default()
                },
                SideScore {
                    side: 1,
                    starting_value: enemy.0,
                    remaining_hp_value: enemy.1,
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn an_attack_is_won_by_destroying_the_target_even_if_most_units_died() {
        let mut r = result(None, (100.0, 10.0), (100.0, 90.0));
        r.sides[1].key_units_started = 1;
        r.sides[1].key_units_surviving = 0;
        let w = ObjectiveWeights::default();
        assert_eq!(assess(&r, 0, &w).outcome, Outcome::Timeout);
        assert_eq!(
            assess_objective(&r, 0, &w, Some(ObjectiveKind::Attack)).outcome,
            Outcome::Win
        );
        // The same fight seen by the defender is a lost defence.
        assert_eq!(
            assess_objective(&r, 1, &w, Some(ObjectiveKind::Defend)).outcome,
            Outcome::Loss
        );
    }

    #[test]
    fn holding_the_target_until_time_runs_out_wins_a_defence() {
        let mut r = result(None, (100.0, 40.0), (100.0, 80.0));
        r.sides[0].key_units_started = 1;
        r.sides[0].key_units_surviving = 1;
        let w = ObjectiveWeights::default();
        assert_eq!(
            assess_objective(&r, 0, &w, Some(ObjectiveKind::Defend)).outcome,
            Outcome::Win
        );
        // An attacker that ran out of time failed: a timeout, not a win.
        assert_eq!(
            assess_objective(&r, 1, &w, Some(ObjectiveKind::Attack)).outcome,
            Outcome::Timeout
        );
    }

    #[test]
    fn no_objective_changes_nothing() {
        let r = result(Some(0), (100.0, 60.0), (100.0, 0.0));
        let w = ObjectiveWeights::default();
        assert_eq!(assess(&r, 0, &w), assess_objective(&r, 0, &w, None));
    }

    #[test]
    fn beating_a_stronger_force_beats_beating_an_equal_one() {
        // The old, discounted-favourite behaviour, to check ordering by odds.
        let w = ObjectiveWeights {
            win_stake_floor: 0.0,
            ..ObjectiveWeights::default()
        };
        let even = assess(&result(Some(0), (100.0, 50.0), (100.0, 0.0)), 0, &w);
        let underdog = assess(&result(Some(0), (100.0, 50.0), (200.0, 0.0)), 0, &w);
        let favourite = assess(&result(Some(0), (200.0, 100.0), (100.0, 0.0)), 0, &w);
        assert!(underdog.underdog && !even.underdog && !favourite.underdog);
        assert!(underdog.fitness > even.fitness);
        assert!(even.fitness > favourite.fitness);
        assert_eq!(underdog.stake, 2.0);
        assert_eq!(favourite.stake, 0.5);
    }

    #[test]
    fn losing_to_a_weaker_force_costs_more_than_losing_as_the_underdog() {
        let w = ObjectiveWeights::default();
        let as_favourite = assess(&result(Some(1), (200.0, 0.0), (100.0, 50.0)), 0, &w);
        let as_underdog = assess(&result(Some(1), (100.0, 0.0), (200.0, 100.0)), 0, &w);
        assert!(as_favourite.fitness < as_underdog.fitness);
        assert!(as_underdog.fitness < 0.0, "losing is never rewarded");
    }

    #[test]
    fn exchange_gives_signal_before_anyone_wins() {
        let w = ObjectiveWeights::default();
        let w = ObjectiveWeights {
            timeout_penalty: 0.0,
            ..w
        };
        let good = assess(&result(None, (100.0, 90.0), (100.0, 40.0)), 0, &w);
        let bad = assess(&result(None, (100.0, 40.0), (100.0, 90.0)), 0, &w);
        assert_eq!(good.outcome, Outcome::Timeout);
        assert!((good.exchange - 0.25).abs() < 1e-9);
        assert!((bad.exchange + 0.25).abs() < 1e-9);
        assert!(good.fitness > 0.0 && bad.fitness < 0.0);
    }

    #[test]
    fn surviving_as_the_underdog_is_a_partial_win_and_an_even_stall_a_partial_loss() {
        let w = ObjectiveWeights {
            exchange: 0.0,
            timeout_penalty: 0.3,
            ..Default::default()
        };
        let underdog = assess(&result(None, (100.0, 100.0), (200.0, 200.0)), 0, &w);
        let even = assess(&result(None, (100.0, 100.0), (100.0, 100.0)), 0, &w);
        let favourite = assess(&result(None, (200.0, 200.0), (100.0, 100.0)), 0, &w);
        assert_eq!(underdog.outcome, Outcome::Timeout);
        // 0.3 of a win, at the 2x stake of beating a force twice its value.
        assert!((underdog.fitness - 0.3 * 2.0).abs() < 1e-9);
        assert!((even.fitness + 0.3).abs() < 1e-9);
        // Failing to finish a weaker force is worse than stalling evenly.
        assert!(favourite.fitness < even.fitness);
        // And a partial win never outranks an actual win.
        let win = assess(&result(Some(0), (100.0, 50.0), (200.0, 0.0)), 0, &w);
        assert!(win.fitness > underdog.fitness);
        // Mutual wipe-out is neutral on the outcome term.
        let mut wipe = result(None, (100.0, 0.0), (100.0, 0.0));
        wipe.timed_out = false;
        assert_eq!(assess(&wipe, 0, &w).outcome, Outcome::Draw);
        assert_eq!(assess(&wipe, 0, &w).fitness, 0.0);
    }

    #[test]
    fn learner_on_side_one_is_scored_from_its_own_view() {
        let w = ObjectiveWeights::default();
        let a = assess(&result(Some(1), (200.0, 0.0), (100.0, 80.0)), 1, &w);
        assert_eq!(a.outcome, Outcome::Win);
        assert_eq!(a.value_ratio, 2.0);
    }

    #[test]
    fn stake_is_capped_and_exponent_zero_ignores_odds() {
        let capped = ObjectiveWeights::default();
        let huge = assess(&result(Some(0), (10.0, 5.0), (1_000.0, 0.0)), 0, &capped);
        assert_eq!(huge.stake, 4.0);
        let flat = ObjectiveWeights {
            underdog_exponent: 0.0,
            ..capped
        };
        assert_eq!(
            assess(&result(Some(0), (10.0, 5.0), (1_000.0, 0.0)), 0, &flat).stake,
            1.0
        );
    }

    #[test]
    fn killing_a_commander_pays_even_without_winning_and_losing_ours_costs() {
        let w = ObjectiveWeights {
            win: 0.0,
            exchange: 0.0,
            key_unit_bonus: 0.6,
            key_unit_penalty: 0.4,
            ..Default::default()
        };
        let mut r = result(None, (100.0, 90.0), (100.0, 90.0));
        r.sides[0].key_units_started = 1;
        r.sides[0].key_units_surviving = 1;
        r.sides[1].key_units_started = 2;
        r.sides[1].key_units_surviving = 1; // half the enemy key units died
        let a = assess(&r, 0, &w);
        assert!((a.key_term - 0.3).abs() < 1e-12 && (a.fitness - 0.3).abs() < 1e-12);

        // The same battle seen from the other side: we lost half our key units.
        let b = assess(&r, 1, &w);
        assert!((b.key_term + 0.2).abs() < 1e-12);

        // No key units anywhere means no key term at all.
        let plain = assess(&result(None, (100.0, 90.0), (100.0, 90.0)), 0, &w);
        assert_eq!(plain.key_term, 0.0);
    }

    #[test]
    fn a_favourite_must_win_and_stalling_is_nearly_as_bad_as_losing() {
        let w = ObjectiveWeights {
            exchange: 0.0,
            ..Default::default()
        };
        // 5 of value 50 against 3 of value 50: the ratio is 0.6.
        let win = assess(&result(Some(0), (250.0, 200.0), (150.0, 0.0)), 0, &w);
        let stall = assess(&result(None, (250.0, 250.0), (150.0, 150.0)), 0, &w);
        let loss = assess(&result(Some(1), (250.0, 0.0), (150.0, 100.0)), 0, &w);
        // Wins are never discounted for being the favourite...
        assert_eq!(win.stake, 1.0);
        assert!((win.fitness - 1.0).abs() < 1e-12);
        // ...and failing to win costs more than the win pays.
        assert!(stall.fitness < -0.9, "{}", stall.fitness);
        assert!(loss.fitness < stall.fitness);
    }

    #[test]
    fn robust_fitness_punishes_one_terrible_row_more_than_the_mean_does() {
        let row = |fitness: f64| Tally {
            battles: 10,
            fitness_sum: fitness * 10.0,
            ..Default::default()
        };
        let rows = [row(1.0), row(1.0), row(1.0), row(-1.0)];
        let none = ObjectiveWeights {
            worst_case_weight: 0.0,
            ..Default::default()
        };
        let some = ObjectiveWeights {
            worst_case_weight: 0.5,
            ..Default::default()
        };
        assert!((robust_fitness(rows.iter(), &none) - 0.5).abs() < 1e-12);
        // mean 0.5, worst quarter (one row) -1: 0.5*0.5 + 0.5*(-1) = -0.25
        assert!((robust_fitness(rows.iter(), &some) + 0.25).abs() < 1e-12);
        // With uniform rows the blend equals the mean.
        let uniform = [row(0.3), row(0.3)];
        assert!((robust_fitness(uniform.iter(), &some) - 0.3).abs() < 1e-12);
        assert_eq!(robust_fitness([].iter(), &some), 0.0);
    }

    #[test]
    fn invalid_weights_are_rejected() {
        assert!(ObjectiveWeights::default().validate().is_ok());
        assert!(
            ObjectiveWeights {
                underdog_cap: 0.5,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            ObjectiveWeights {
                win: 0.0,
                exchange: 0.0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
