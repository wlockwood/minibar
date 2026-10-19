//! Generated matchups: random platoons for each side, drawn fresh from a seed.
//!
//! Training on a handful of fixed scenarios lets a council memorise them. This
//! module instead builds a new batch of fights from a declared set of platoon
//! types (the squad roles Planetary Annihilation's AI uses: scouts, raiders,
//! brawlers and so on), so the same seed always gives the same batch and a new
//! seed gives an unseen one.
//!
//! * A platoon contains one movement domain: bots, vehicles, structures or air.
//! * Half of the platoons are **mixed** (several unit types from one platoon
//!   type's pool); the rest are **pure** (one unit type, all of them).
//! * A side has one platoon, or sometimes two that start as separate groups.
//! * Fights come in three investment classes by metal + energy / 60: **underdog** (the learner
//!   has less), **even**, and **overdog** (the learner has more).

use super::objective::ObjectiveKind;
use super::rng::Rng;
use super::train::{ObjectiveSpec, RosterEntry, ScenarioSpec, TrainError};
use crate::catalog::UnitCatalog;
use serde::{Deserialize, Serialize};

/// Which kind of mover a platoon is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Bot,
    /// Tracked, wheeled and hover vehicles.
    Vehicle,
    Static,
    Air,
}

impl Domain {
    fn label(self) -> &'static str {
        match self {
            Domain::Bot => "bot",
            Domain::Vehicle => "veh",
            Domain::Static => "static",
            Domain::Air => "air",
        }
    }

    fn matches(self, movement_class: &str) -> bool {
        let class = movement_class.to_ascii_uppercase();
        match self {
            Domain::Bot => class.contains("BOT"),
            Domain::Vehicle => class.contains("TANK") || class.contains("HOVER"),
            Domain::Static | Domain::Air => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeightedUnit {
    pub unit: String,
    #[serde(default = "one_f32")]
    pub weight: f32,
}

fn one_f32() -> f32 {
    1.0
}

/// A squad role with the units that can fill it, per domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlatoonType {
    pub name: String,
    #[serde(default)]
    pub bots: Vec<WeightedUnit>,
    #[serde(default)]
    pub vehicles: Vec<WeightedUnit>,
    #[serde(default)]
    pub static_units: Vec<WeightedUnit>,
    #[serde(default)]
    pub aircraft: Vec<WeightedUnit>,
}

impl PlatoonType {
    fn pool(&self, domain: Domain) -> &[WeightedUnit] {
        match domain {
            Domain::Bot => &self.bots,
            Domain::Vehicle => &self.vehicles,
            Domain::Static => &self.static_units,
            Domain::Air => &self.aircraft,
        }
    }
}

/// An explicit coverage slot. Slots repeat in order, independent of random draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FamilyQuota {
    pub family: String,
    pub learner_type: String,
    pub enemy_type: String,
}

/// Curated exact armies, admitted only within their declared symmetric gap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchoredMatchup {
    pub name: String,
    pub family: String,
    pub learner: Vec<RosterEntry>,
    pub enemy: Vec<RosterEntry>,
    pub max_gap: f32,
}

/// Native preview and evaluation share these exact seeded scenarios.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchupRecord {
    pub scenario: ScenarioSpec,
    pub family: String,
    pub seed: u64,
    pub index: usize,
    pub requested_budget: [f32; 2],
    pub combat_value: [f32; 2],
    pub objective_value: [f32; 2],
    pub starting_value: [f32; 2],
    /// Enemy / learner - 1; positive means the learner has less investment.
    pub signed_ratio: f32,
    pub absolute_gap: f32,
    pub balance_class: String,
    pub cap_limited: bool,
    pub unit_cap: usize,
    pub support_status: String,
    pub resource_supply: Option<crate::model::StartingResources>,
    pub opponent_identity: String,
    pub learner_side: u8,
}

pub fn symmetric_gap(a: f32, b: f32) -> f32 {
    (a - b).abs() / ((a + b) * 0.5).max(f32::MIN_POSITIVE)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchupConfig {
    /// Fights per batch (a multiple of 3 keeps the three classes balanced).
    pub count: usize,
    pub anchors: Vec<AnchoredMatchup>,
    pub unit_names: std::collections::BTreeMap<String, String>,
    pub families: Vec<FamilyQuota>,
    /// Desired balanced combat investment gap; unmet targets remain explicit.
    pub max_balanced_gap: f32,
    /// Resource value (metal + energy / 60), retaining the historical JSON key.
    pub metal: [f32; 2],
    /// Enemy resource value divided by learner resource value, per class.
    pub underdog_ratio: [f32; 2],
    pub even_ratio: [f32; 2],
    pub overdog_ratio: [f32; 2],
    /// Chance that a platoon is mixed rather than a single unit type.
    pub mixed_share: f32,
    /// Chance that a side fields a second platoon.
    pub second_platoon_chance: f32,
    /// Maximum units per side; cap-limited residuals are reported.
    pub max_units: usize,
    /// Distance between the two front lines at the start.
    pub separation: [f32; 2],
    /// Share of fights where the learner must destroy an enemy target, and
    /// where it must keep its own alive. The rest are plain fights.
    pub attack_share: f32,
    pub defend_share: f32,
    /// Immobile catalog units used as targets (one is drawn per fight).
    pub targets: Vec<String>,
    pub types: Vec<PlatoonType>,
}

impl Default for MatchupConfig {
    fn default() -> Self {
        Self {
            count: 12,
            anchors: Vec::new(),
            unit_names: std::collections::BTreeMap::new(),
            families: Vec::new(),
            max_balanced_gap: 0.05,
            metal: [500.0, 1_400.0],
            underdog_ratio: [1.25, 2.0],
            even_ratio: [0.98, 1.02],
            overdog_ratio: [0.5, 0.8],
            mixed_share: 0.5,
            second_platoon_chance: 0.4,
            max_units: 96,
            separation: [450.0, 800.0],
            attack_share: 0.0,
            defend_share: 0.0,
            targets: Vec::new(),
            types: Vec::new(),
        }
    }
}

impl MatchupConfig {
    pub fn validate(&self, catalog: &UnitCatalog) -> Result<(), TrainError> {
        let err = |m: String| Err(TrainError(format!("matchups: {m}")));
        if self.count == 0 || (self.types.is_empty() && self.anchors.is_empty()) {
            return err("needs a positive count and at least one platoon type".into());
        }
        if self.count < self.anchors.len() + self.families.len() {
            return err("count must cover every anchor and family slot in each batch".into());
        }
        let ranges = [
            ("metal", self.metal),
            ("underdog_ratio", self.underdog_ratio),
            ("even_ratio", self.even_ratio),
            ("overdog_ratio", self.overdog_ratio),
            ("separation", self.separation),
        ];
        for (name, [low, high]) in ranges {
            if !(low.is_finite() && high.is_finite() && low > 0.0 && low <= high) {
                return err(format!("{name} must be a positive range, low <= high"));
            }
        }
        if self.max_units == 0 {
            return err("max_units must be positive".into());
        }
        let shares = [self.attack_share, self.defend_share];
        if shares.iter().any(|s| !(s.is_finite() && *s >= 0.0)) || shares.iter().sum::<f32>() > 1.0
        {
            return err(
                "attack_share and defend_share must be non-negative and sum to at most 1".into(),
            );
        }
        if shares.iter().sum::<f32>() > 0.0 && self.targets.is_empty() {
            return err("objectives need at least one target unit".into());
        }
        for target in &self.targets {
            if catalog.get(target).is_none() {
                return err(format!("target {target:?} is not in the catalog"));
            }
        }
        for platoon in &self.types {
            if [Domain::Bot, Domain::Vehicle, Domain::Static, Domain::Air]
                .iter()
                .all(|d| platoon.pool(*d).is_empty())
            {
                return err(format!("type {:?} has no units", platoon.name));
            }
            for domain in [Domain::Bot, Domain::Vehicle, Domain::Static, Domain::Air] {
                for entry in platoon.pool(domain) {
                    let def = catalog.get(&entry.unit).ok_or_else(|| {
                        TrainError(format!(
                            "matchups: unit {:?} is not in the catalog",
                            entry.unit
                        ))
                    })?;
                    let class = def.strategic.movement_class.as_deref().unwrap_or("");
                    let matches = match domain {
                        Domain::Static => {
                            def.movement.max_speed == 0.0
                                && def.flight.is_none()
                                && !def.weapons.is_empty()
                        }
                        Domain::Air => def.flight.is_some(),
                        _ => def.flight.is_none() && domain.matches(class),
                    };
                    if !matches {
                        return err(format!(
                            "{} listed as a {} but its movement class is {class:?}",
                            entry.unit,
                            domain.label()
                        ));
                    }
                    if !(entry.weight.is_finite() && entry.weight > 0.0) {
                        return err(format!("{}: weight must be positive", entry.unit));
                    }
                    if !def.resource_value().is_finite() || def.resource_value() <= 0.0 {
                        return err(format!(
                            "{}: needs a finite positive metal + energy/60 value",
                            entry.unit
                        ));
                    }
                }
            }
        }
        if !self.mixed_share.is_finite()
            || !(0.0..=1.0).contains(&self.mixed_share)
            || !self.second_platoon_chance.is_finite()
            || !(0.0..=1.0).contains(&self.second_platoon_chance)
            || !self.max_balanced_gap.is_finite()
            || !(0.0..=1.0).contains(&self.max_balanced_gap)
        {
            return err("shares and max_balanced_gap must be finite fractions".into());
        }
        for family in &self.families {
            for name in [&family.learner_type, &family.enemy_type] {
                if !self.types.iter().any(|t| &t.name == name) {
                    return err(format!(
                        "family {} references missing platoon type {name}",
                        family.family
                    ));
                }
            }
        }
        for anchor in &self.anchors {
            if !anchor.max_gap.is_finite() || !(0.0..=0.1).contains(&anchor.max_gap) {
                return err(format!(
                    "anchor {} max_gap must be at most 0.1",
                    anchor.name
                ));
            }
            for army in [&anchor.learner, &anchor.enemy] {
                if army.is_empty() || army.iter().map(|e| e.count).sum::<usize>() > self.max_units {
                    return err(format!(
                        "anchor {} is empty or exceeds unit cap {}",
                        anchor.name, self.max_units
                    ));
                }
                for entry in army {
                    let def = catalog.get(&entry.unit).ok_or_else(|| {
                        TrainError(format!(
                            "anchor {}: missing unit {}",
                            anchor.name, entry.unit
                        ))
                    })?;
                    if entry.count == 0
                        || !def.resource_value().is_finite()
                        || def.resource_value() <= 0.0
                    {
                        return err(format!(
                            "anchor {}: invalid count or resource value for {}",
                            anchor.name, entry.unit
                        ));
                    }
                }
            }
            if symmetric_gap(
                roster_value(&anchor.learner, catalog),
                roster_value(&anchor.enemy, catalog),
            ) > anchor.max_gap
            {
                return err(format!(
                    "anchor {} exceeds declared resource gap",
                    anchor.name
                ));
            }
        }
        let missing = catalog
            .unsupported_capabilities(crate::catalog::IMPLEMENTED_CAPABILITIES.iter().copied());
        let selected = |name: &str| {
            self.types.iter().any(|t| {
                [Domain::Bot, Domain::Vehicle, Domain::Static, Domain::Air]
                    .iter()
                    .any(|d| t.pool(*d).iter().any(|u| u.unit == name))
            }) || self
                .anchors
                .iter()
                .any(|a| a.learner.iter().chain(&a.enemy).any(|e| e.unit == name))
                || self.targets.iter().any(|t| t == name)
        };
        let missing: Vec<_> = missing
            .into_iter()
            .filter(|m| m.unit.as_deref().map_or(true, selected))
            .collect();
        if !missing.is_empty() {
            return err(format!("unsupported mechanics: {missing:?}"));
        }
        Ok(())
    }
}

struct Platoon {
    type_name: String,
    domain: Domain,
    mixed: bool,
    units: Vec<(String, usize)>,
}

fn range(rng: &mut Rng, [low, high]: [f32; 2]) -> f32 {
    low + (high - low) * rng.uniform()
}

fn pick<'a>(rng: &mut Rng, pool: &'a [WeightedUnit]) -> &'a WeightedUnit {
    let total: f32 = pool.iter().map(|u| u.weight).sum();
    let mut roll = rng.uniform() * total;
    for entry in pool {
        if roll < entry.weight {
            return entry;
        }
        roll -= entry.weight;
    }
    pool.last().expect("pool is non-empty")
}

fn add(units: &mut Vec<(String, usize)>, name: &str, count: usize) {
    match units.iter_mut().find(|(unit, _)| unit == name) {
        Some((_, existing)) => *existing += count,
        None => units.push((name.to_owned(), count)),
    }
}

fn platoon(config: &MatchupConfig, catalog: &UnitCatalog, budget: f32, rng: &mut Rng) -> Platoon {
    let platoon_type = &config.types[(rng.next_u64() % config.types.len() as u64) as usize];
    let domains: Vec<_> = [Domain::Bot, Domain::Vehicle, Domain::Static, Domain::Air]
        .into_iter()
        .filter(|d| !platoon_type.pool(*d).is_empty())
        .collect();
    let domain = domains[(rng.next_u64() % domains.len() as u64) as usize];
    let pool = platoon_type.pool(domain);
    let cost = |name: &str| catalog.get(name).map_or(1.0, |d| d.resource_value());
    let mixed = pool.len() > 1 && rng.uniform() < config.mixed_share;
    let mut units: Vec<(String, usize)> = Vec::new();
    let mut count = 0usize;
    let mut left = budget;
    if mixed {
        // Draw units by weight until the budget or the unit cap runs out; the
        // first two draws are forced to differ so "mixed" really is mixed.
        let cheapest = pool.iter().map(|u| cost(&u.unit)).fold(f32::MAX, f32::min);
        while count < config.max_units && left >= cheapest {
            let mut entry = pick(rng, pool);
            if count == 1 && units.len() == 1 {
                for _ in 0..8 {
                    if entry.unit != units[0].0 {
                        break;
                    }
                    entry = pick(rng, pool);
                }
            }
            if cost(&entry.unit) > left && count > 0 {
                // Too expensive for what is left: try the cheapest affordable.
                match pool
                    .iter()
                    .filter(|u| cost(&u.unit) <= left)
                    .min_by(|a, b| cost(&a.unit).total_cmp(&cost(&b.unit)))
                {
                    Some(cheap) => entry = cheap,
                    None => break,
                }
            }
            add(&mut units, &entry.unit, 1);
            count += 1;
            left -= cost(&entry.unit);
        }
    } else {
        let entry = pick(rng, pool);
        let each = cost(&entry.unit);
        let n = ((budget / each).round() as usize).clamp(1, config.max_units);
        add(&mut units, &entry.unit, n);
    }
    if mixed && !units.is_empty() {
        // Compare additions/removals/substitutions on both sides of the budget.
        // Strict improvement terminates and preserves the bounded unit count.
        for _ in 0..config.max_units * 2 {
            let value: f32 = units.iter().map(|(u, n)| cost(u) * *n as f32).sum();
            let mut best_gap = (value - budget).abs();
            let mut best = None;
            for replacement in pool {
                if count < config.max_units {
                    let gap = (value + cost(&replacement.unit) - budget).abs();
                    if gap + 0.001 < best_gap {
                        best_gap = gap;
                        best = Some((None, Some(replacement.unit.clone())));
                    }
                }
                for (old, n) in &units {
                    if *n == 0 {
                        continue;
                    }
                    let gap = (value - cost(old) + cost(&replacement.unit) - budget).abs();
                    if gap + 0.001 < best_gap {
                        best_gap = gap;
                        best = Some((Some(old.clone()), Some(replacement.unit.clone())));
                    }
                    let gap = (value - cost(old) - budget).abs();
                    if count > 1 && gap + 0.001 < best_gap {
                        best_gap = gap;
                        best = Some((Some(old.clone()), None));
                    }
                }
            }
            let Some((remove, insert)) = best else {
                break;
            };
            if let Some(old) = remove {
                let entry = units.iter_mut().find(|(u, _)| u == &old).unwrap();
                entry.1 -= 1;
                count -= 1;
            }
            if let Some(new) = insert {
                add(&mut units, &new, 1);
                count += 1;
            }
            units.retain(|(_, n)| *n > 0);
        }
    }
    if units.is_empty() {
        let entry = pick(rng, pool);
        add(&mut units, &entry.unit, 1);
    }
    let mixed = units.len() > 1;
    Platoon {
        type_name: platoon_type.name.clone(),
        domain,
        mixed,
        units,
    }
}

fn side(config: &MatchupConfig, catalog: &UnitCatalog, budget: f32, rng: &mut Rng) -> Vec<Platoon> {
    if config.max_units >= 2 && rng.uniform() < config.second_platoon_chance {
        let mut first = config.clone();
        first.max_units /= 2;
        let mut second = config.clone();
        second.max_units -= first.max_units;
        let share = 0.35 + 0.3 * rng.uniform();
        vec![
            platoon(&first, catalog, budget * share, rng),
            platoon(&second, catalog, budget * (1.0 - share), rng),
        ]
    } else {
        vec![platoon(config, catalog, budget, rng)]
    }
}

fn value_of(platoons: &[Platoon], catalog: &UnitCatalog) -> f32 {
    platoons
        .iter()
        .flat_map(|p| &p.units)
        .map(|(name, n)| catalog.get(name).map_or(0.0, |d| d.resource_value()) * *n as f32)
        .sum()
}

fn describe(platoons: &[Platoon], config: &MatchupConfig) -> String {
    platoons
        .iter()
        .map(|p| {
            let units = p
                .units
                .iter()
                .map(|(name, n)| match config.unit_names.get(name) {
                    Some(human) => format!("{n}x{human}({name})"),
                    None => format!("{n}x{name}"),
                })
                .collect::<Vec<_>>()
                .join("+");
            format!(
                "{} {} {units}",
                p.domain.label(),
                if p.mixed {
                    format!("{}-mix", p.type_name)
                } else {
                    format!("{}-pure", p.type_name)
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

fn roster(platoons: &[Platoon]) -> Vec<RosterEntry> {
    platoons
        .iter()
        .enumerate()
        .flat_map(|(index, p)| {
            p.units.iter().map(move |(unit, count)| RosterEntry {
                unit: unit.clone(),
                count: *count,
                platoon: index,
            })
        })
        .collect()
}

/// The class a fight falls in, judged on the resource value actually fielded.
pub fn class_of(learner_metal: f32, enemy_metal: f32) -> &'static str {
    let ratio = enemy_metal / learner_metal.max(f32::MIN_POSITIVE);
    if ratio > 1.15 {
        "under"
    } else if ratio < 0.87 {
        "over"
    } else {
        "even"
    }
}

/// A batch of fights for `seed`. The same seed always gives the same batch.

fn roster_value(entries: &[RosterEntry], catalog: &UnitCatalog) -> f32 {
    entries
        .iter()
        .map(|e| catalog.get(&e.unit).unwrap().resource_value() * e.count as f32)
        .sum()
}

/// Retains actual integer packing, objectives and support assumptions for audit/preview.
pub fn generate_records(
    config: &MatchupConfig,
    catalog: &UnitCatalog,
    seed: u64,
) -> Result<Vec<MatchupRecord>, TrainError> {
    config.validate(catalog)?;
    let slots = config.anchors.len() + config.families.len();
    let mut records = Vec::with_capacity(config.count);
    for index in 0..config.count {
        let mut rng = Rng::for_stream(seed ^ 0x6D61_7463_6875_7073, index as u64);
        let slot = if slots == 0 {
            usize::MAX
        } else {
            index % slots
        };
        let (mut scenario, family, requested_budget) = if slot < config.anchors.len() {
            let anchor = &config.anchors[slot];
            let values = [
                roster_value(&anchor.learner, catalog),
                roster_value(&anchor.enemy, catalog),
            ];
            (
                ScenarioSpec {
                    faction: None,
                    name: format!(
                        "m{index:02} [{} {:.2}x] {}",
                        class_of(values[0], values[1]),
                        values[1] / values[0],
                        anchor.name
                    ),
                    learner: anchor.learner.clone(),
                    enemy: anchor.enemy.clone(),
                    objective: None,
                    separation: range(&mut rng, config.separation).round(),
                },
                anchor.family.clone(),
                values,
            )
        } else {
            let mut ours_config = config.clone();
            let mut theirs_config = config.clone();
            let family = if slot != usize::MAX {
                let family = &config.families[slot - config.anchors.len()];
                ours_config.types = vec![
                    config
                        .types
                        .iter()
                        .find(|t| t.name == family.learner_type)
                        .unwrap()
                        .clone(),
                ];
                theirs_config.types = vec![
                    config
                        .types
                        .iter()
                        .find(|t| t.name == family.enemy_type)
                        .unwrap()
                        .clone(),
                ];
                family.family.clone()
            } else {
                "mixed_ground".to_owned()
            };
            let band = if slots == 0 {
                index % 3
            } else {
                (index / slots) % 3
            };
            let ratio = match band {
                0 => config.underdog_ratio,
                1 => config.even_ratio,
                _ => config.overdog_ratio,
            };
            let budget = range(&mut rng, config.metal);
            let enemy_budget = budget * range(&mut rng, ratio);
            let mut learner = side(&ours_config, catalog, budget, &mut rng);
            let mut enemy = side(&theirs_config, catalog, enemy_budget, &mut rng);
            // Try independent compositions near the requested ratio, without a combat-power filter.
            let desired = enemy_budget / budget;
            let error = |a: &[Platoon], b: &[Platoon]| {
                (value_of(b, catalog) / value_of(a, catalog) - desired).abs()
            };
            let mut best = error(&learner, &enemy);
            for _ in 0..24 {
                let a = side(&ours_config, catalog, budget, &mut rng);
                let b = side(&theirs_config, catalog, enemy_budget, &mut rng);
                let candidate = error(&a, &b);
                if candidate < best {
                    best = candidate;
                    learner = a;
                    enemy = b;
                }
                if best <= config.max_balanced_gap * 0.5 {
                    break;
                }
            }
            let roll = rng.uniform();
            let kind = if roll < config.attack_share {
                Some(ObjectiveKind::Attack)
            } else if roll < config.attack_share + config.defend_share {
                Some(ObjectiveKind::Defend)
            } else {
                None
            };
            let objective = kind.map(|kind| ObjectiveSpec {
                kind,
                target: config.targets[(rng.next_u64() % config.targets.len() as u64) as usize]
                    .clone(),
            });
            let tag = objective
                .as_ref()
                .map(|o| {
                    format!(
                        " {}>{}",
                        if o.kind == ObjectiveKind::Attack {
                            "atk"
                        } else {
                            "def"
                        },
                        o.target
                    )
                })
                .unwrap_or_default();
            let (ours, theirs) = (value_of(&learner, catalog), value_of(&enemy, catalog));
            let fight = ScenarioSpec {
                faction: None,
                name: format!(
                    "m{index:02} [{} {:.2}x{tag}] {} {} vs {}",
                    class_of(ours, theirs),
                    theirs / ours,
                    family,
                    describe(&learner, config),
                    describe(&enemy, config)
                ),
                learner: roster(&learner),
                enemy: roster(&enemy),
                objective,
                separation: range(&mut rng, config.separation).round(),
            };
            (fight, family, [budget, enemy_budget])
        };
        let combat_value = [
            roster_value(&scenario.learner, catalog),
            roster_value(&scenario.enemy, catalog),
        ];
        let mut objective_value = [0.0, 0.0];
        if let Some(objective) = &scenario.objective {
            let owner = if objective.kind == ObjectiveKind::Attack {
                1
            } else {
                0
            };
            objective_value[owner] = catalog.get(&objective.target).unwrap().resource_value();
        }
        let starting_value = [
            combat_value[0] + objective_value[0],
            combat_value[1] + objective_value[1],
        ];
        let absolute_gap = symmetric_gap(combat_value[0], combat_value[1]);
        let balance_class = if absolute_gap <= config.max_balanced_gap {
            "balanced"
        } else if combat_value[0] < combat_value[1] {
            "disadvantage"
        } else {
            "advantage"
        }
        .to_owned();
        scenario.name = format!(
            "{} [value {:.2}/{:.2} gap {:.2}% {}]",
            scenario.name,
            combat_value[0],
            combat_value[1],
            absolute_gap * 100.0,
            balance_class
        );
        let factions = scenario
            .learner
            .iter()
            .map(|entry| {
                if entry.unit.starts_with("arm") {
                    "armada"
                } else if entry.unit.starts_with("cor") {
                    "cortex"
                } else if entry.unit.starts_with("leg") {
                    "legion"
                } else {
                    "unspecified"
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        scenario.faction = Some(
            if factions.len() == 1 {
                *factions.first().unwrap()
            } else {
                "mixed"
            }
            .into(),
        );
        let cap_limited =
            [&scenario.learner, &scenario.enemy]
                .iter()
                .enumerate()
                .any(|(side, army)| {
                    army.iter().map(|e| e.count).sum::<usize>() >= config.max_units
                        && combat_value[side] < requested_budget[side]
                });
        records.push(MatchupRecord {
            scenario,
            family,
            seed,
            index,
            requested_budget,
            combat_value,
            objective_value,
            starting_value,
            signed_ratio: combat_value[1] / combat_value[0] - 1.0,
            absolute_gap,
            balance_class,
            cap_limited,
            unit_cap: config.max_units,
            support_status: "supported".into(),
            resource_supply: None,
            opponent_identity: "evaluation roster (assigned by evaluator)".into(),
            learner_side: 0,
        });
    }
    Ok(records)
}

pub fn generate(
    config: &MatchupConfig,
    catalog: &UnitCatalog,
    seed: u64,
) -> Result<Vec<ScenarioSpec>, TrainError> {
    Ok(generate_records(config, catalog, seed)?
        .into_iter()
        .map(|r| r.scenario)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::tactical::tests::skirmisher;

    #[test]
    fn quota_batches_cannot_silently_omit_later_families() {
        let mut cfg = config();
        cfg.count = 1;
        cfg.families = vec![
            FamilyQuota {
                family: "one".into(),
                learner_type: "raid".into(),
                enemy_type: "brawl".into(),
            },
            FamilyQuota {
                family: "two".into(),
                learner_type: "raid".into(),
                enemy_type: "brawl".into(),
            },
        ];
        assert!(cfg.validate(&catalog()).is_err());
    }

    #[test]
    fn odd_side_cap_is_available_to_two_platoons() {
        let mut cfg = config();
        cfg.max_units = 5;
        cfg.second_platoon_chance = 1.0;
        cfg.types.truncate(1);
        cfg.types[0].vehicles.clear();
        cfg.types[0].bots.truncate(1);
        let army = side(&cfg, &catalog(), 100000.0, &mut Rng::new(4));
        assert_eq!(
            army.iter()
                .flat_map(|p| &p.units)
                .map(|(_, n)| *n)
                .sum::<usize>(),
            5
        );
    }

    fn catalog() -> UnitCatalog {
        let mut catalog = UnitCatalog::default();
        for (name, class, cost) in [
            ("b1", "BOT2", 50.0),
            ("b2", "BOT3", 140.0),
            ("v1", "TANK2", 100.0),
            ("v2", "TANK3", 230.0),
        ] {
            let mut def = skirmisher(name);
            def.metal_cost = cost;
            def.strategic.movement_class = Some(class.into());
            catalog.units.insert(name.into(), def);
        }
        catalog
    }

    fn units(names: &[&str]) -> Vec<WeightedUnit> {
        names
            .iter()
            .map(|n| WeightedUnit {
                unit: (*n).into(),
                weight: 1.0,
            })
            .collect()
    }

    fn config() -> MatchupConfig {
        MatchupConfig {
            count: 60,
            types: vec![
                PlatoonType {
                    name: "raid".into(),
                    bots: units(&["b1", "b2"]),
                    vehicles: units(&["v1"]),
                    static_units: vec![],
                    aircraft: vec![],
                },
                PlatoonType {
                    name: "brawl".into(),
                    bots: units(&["b2"]),
                    vehicles: units(&["v1", "v2"]),
                    static_units: vec![],
                    aircraft: vec![],
                },
            ],
            ..MatchupConfig::default()
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_batch_and_a_new_seed_a_new_one() {
        let (config, catalog) = (config(), catalog());
        let a = generate(&config, &catalog, 7).unwrap();
        assert_eq!(a, generate(&config, &catalog, 7).unwrap());
        assert_ne!(a, generate(&config, &catalog, 8).unwrap());
    }

    #[test]
    fn no_platoon_mixes_bots_with_vehicles() {
        let (config, catalog) = (config(), catalog());
        for scenario in generate(&config, &catalog, 3).unwrap() {
            for roster in [&scenario.learner, &scenario.enemy] {
                let platoons = roster.iter().map(|e| e.platoon).max().unwrap() + 1;
                for p in 0..platoons {
                    let classes: Vec<bool> = roster
                        .iter()
                        .filter(|e| e.platoon == p)
                        .map(|e| e.unit.starts_with('b'))
                        .collect();
                    assert!(!classes.is_empty());
                    assert!(
                        classes.iter().all(|c| *c == classes[0]),
                        "{} mixes domains",
                        scenario.name
                    );
                }
            }
        }
    }

    #[test]
    fn all_three_classes_appear_and_roughly_half_the_platoons_are_mixed() {
        let (config, catalog) = (config(), catalog());
        let scenarios = generate(&config, &catalog, 11).unwrap();
        for tag in ["[under", "[even", "[over"] {
            assert!(
                scenarios.iter().filter(|s| s.name.contains(tag)).count() >= 8,
                "{tag}"
            );
        }
        let mixed = scenarios.iter().filter(|s| s.name.contains("-mix")).count();
        let pure = scenarios
            .iter()
            .filter(|s| s.name.contains("-pure"))
            .count();
        assert!(mixed > 0 && pure > 0);
    }

    #[test]
    fn objective_shares_produce_attack_and_defend_fights_with_a_target() {
        let mut catalog = catalog();
        let mut tower = skirmisher("tower");
        tower.movement.max_speed = 0.0;
        catalog.units.insert("tower".into(), tower);
        let config = MatchupConfig {
            attack_share: 0.3,
            defend_share: 0.3,
            targets: vec!["tower".into()],
            ..config()
        };
        let scenarios = generate(&config, &catalog, 5).unwrap();
        let count = |kind| {
            scenarios
                .iter()
                .filter(|s| s.objective.as_ref().is_some_and(|o| o.kind == kind))
                .count()
        };
        assert!(count(ObjectiveKind::Attack) >= 8 && count(ObjectiveKind::Defend) >= 8);
        assert!(scenarios.iter().any(|s| s.objective.is_none()));
        assert!(scenarios.iter().all(|s| {
            s.objective
                .as_ref()
                .map_or(true, |o| s.name.contains(&o.target))
        }));
        let bad = MatchupConfig {
            attack_share: 0.8,
            defend_share: 0.5,
            ..config.clone()
        };
        assert!(bad.validate(&catalog).is_err());
    }

    #[test]
    fn a_domain_mismatch_is_rejected() {
        let mut config = config();
        config.types[0].bots = units(&["v1"]);
        assert!(config.validate(&catalog()).is_err());
    }
    #[test]
    fn packing_spends_energy_equivalent_as_well_as_metal() {
        let mut catalog = catalog();
        catalog.units.get_mut("b1").unwrap().energy_cost = 3000.0;
        let config = MatchupConfig {
            count: 1,
            metal: [1000.0, 1000.0],
            second_platoon_chance: 0.0,
            mixed_share: 0.0,
            types: vec![PlatoonType {
                name: "energy".into(),
                bots: units(&["b1"]),
                vehicles: vec![],
                static_units: vec![],
                aircraft: vec![],
            }],
            ..MatchupConfig::default()
        };
        let scenarios = generate(&config, &catalog, 1).unwrap();
        assert_eq!(scenarios[0].learner[0].count, 10);
    }

    #[test]
    fn curated_heavy_swarm_and_artillery_pairs_keep_realized_values() {
        let mut catalog = catalog();
        for (name, metal, energy) in [
            ("corsumo", 2200.0, 35000.0),
            ("armpw", 54.0, 900.0),
            ("armmanni", 1200.0, 18500.0),
            ("corban", 1000.0, 23000.0),
        ] {
            let mut unit = skirmisher(name);
            unit.metal_cost = metal;
            unit.energy_cost = energy;
            catalog.units.insert(name.into(), unit);
        }
        let army = |unit: &str, count| {
            vec![RosterEntry {
                unit: unit.into(),
                count,
                platoon: 0,
            }]
        };
        let cfg = MatchupConfig {
            count: 2,
            types: vec![],
            anchors: vec![
                AnchoredMatchup {
                    name: "Mammoth vs 40 Pawns".into(),
                    family: "heavy_swarm".into(),
                    learner: army("corsumo", 1),
                    enemy: army("armpw", 40),
                    max_gap: 0.05,
                },
                AnchoredMatchup {
                    name: "Starlight vs Banisher".into(),
                    family: "artillery".into(),
                    learner: army("armmanni", 1),
                    enemy: army("corban", 1),
                    max_gap: 0.1,
                },
            ],
            ..MatchupConfig::default()
        };
        let records = generate_records(&cfg, &catalog, 43).unwrap();
        assert_eq!(records[0].scenario.enemy[0].count, 40);
        assert!((records[0].combat_value[0] - 2783.3333).abs() < 0.01);
        assert!(records[0].absolute_gap < 0.01);
        assert!(records[1].absolute_gap > 0.08 && records[1].absolute_gap < 0.09);
        assert_eq!(records, generate_records(&cfg, &catalog, 43).unwrap());
        let capped = MatchupConfig {
            max_units: 24,
            ..cfg
        };
        assert!(
            generate_records(&capped, &catalog, 43)
                .unwrap_err()
                .0
                .contains("cap")
        );
    }

    #[test]
    fn family_slots_survive_sampling_and_metadata_accounts_for_objectives() {
        let mut catalog = catalog();
        let mut tower = skirmisher("tower");
        tower.movement.max_speed = 0.0;
        catalog.units.insert("tower".into(), tower);
        let mut cfg = config();
        cfg.count = 9;
        cfg.attack_share = 1.0;
        cfg.targets = vec!["tower".into()];
        cfg.families = vec![
            FamilyQuota {
                family: "heavy_swarm".into(),
                learner_type: "raid".into(),
                enemy_type: "brawl".into(),
            },
            FamilyQuota {
                family: "mixed_ground".into(),
                learner_type: "brawl".into(),
                enemy_type: "raid".into(),
            },
        ];
        let records = generate_records(&cfg, &catalog, 19).unwrap();
        assert_eq!(
            records.iter().filter(|r| r.family == "heavy_swarm").count(),
            5
        );
        for record in records {
            assert_eq!(record.objective_value[0], 0.0);
            assert_eq!(
                record.objective_value[1],
                catalog.get("tower").unwrap().resource_value()
            );
            assert_eq!(
                record.starting_value[1],
                record.combat_value[1] + record.objective_value[1]
            );
            assert!(
                record
                    .scenario
                    .learner
                    .iter()
                    .map(|e| e.count)
                    .sum::<usize>()
                    <= cfg.max_units
            );
            assert!(record.scenario.enemy.iter().map(|e| e.count).sum::<usize>() <= cfg.max_units);
        }
    }

    #[test]
    fn invalid_resource_values_and_unsupported_selected_mechanics_are_rejected() {
        let mut catalog = catalog();
        catalog.units.get_mut("b1").unwrap().energy_cost = f32::NAN;
        assert!(config().validate(&catalog).is_err());
        catalog.units.get_mut("b1").unwrap().energy_cost = 0.0;
        catalog
            .units
            .get_mut("b1")
            .unwrap()
            .strategic
            .required_capabilities
            .push("carrier_gadget".into());
        assert!(
            config()
                .validate(&catalog)
                .unwrap_err()
                .0
                .contains("carrier_gadget")
        );
    }
    #[test]
    fn static_and_air_coverage_use_explicit_domains() {
        let mut catalog = catalog();
        let mut tower = skirmisher("tower");
        tower.movement.max_speed = 0.0;
        catalog.units.insert("tower".into(), tower);
        let mut plane = skirmisher("plane");
        plane.flight = Some(crate::model::FlightDef {
            cruise_altitude: 100.0,
            minimum_speed: 40.0,
            attack_run: true,
        });
        plane.strategic.movement_class = None;
        catalog.units.insert("plane".into(), plane);
        let mut cfg = config();
        cfg.count = 6;
        cfg.types.push(PlatoonType {
            name: "static".into(),
            bots: vec![],
            vehicles: vec![],
            static_units: units(&["tower"]),
            aircraft: vec![],
        });
        cfg.types.push(PlatoonType {
            name: "air".into(),
            bots: vec![],
            vehicles: vec![],
            static_units: vec![],
            aircraft: units(&["plane"]),
        });
        cfg.families = vec![FamilyQuota {
            family: "bomber_aa".into(),
            learner_type: "air".into(),
            enemy_type: "static".into(),
        }];
        let records = generate_records(&cfg, &catalog, 17).unwrap();
        assert!(
            records
                .iter()
                .all(|r| r.scenario.learner.iter().all(|u| u.unit == "plane")
                    && r.scenario.enemy.iter().all(|u| u.unit == "tower"))
        );
        assert!(records.iter().all(|r| r.family == "bomber_aa"));
    }

    #[test]
    fn cap_limited_armies_disclose_the_unspent_request() {
        let mut cfg = config();
        cfg.count = 3;
        cfg.max_units = 2;
        cfg.metal = [10_000.0, 10_000.0];
        let catalog = catalog();
        let records = generate_records(&cfg, &catalog, 1).unwrap();
        assert!(records.iter().all(|r| r.cap_limited));
        assert!(
            records
                .iter()
                .all(|r| r.scenario.learner.iter().map(|u| u.count).sum::<usize>() <= 2)
        );
    }
}
