//! Production features derived exclusively from disclosed, identified units.
use crate::model::UnitDef;
pub const EXTRA_INPUTS: [&str; 11] = [
    "candidate_energy_cost",
    "candidate_build_time",
    "candidate_ground_dps",
    "candidate_ground_range",
    "candidate_speed",
    "enemy_mobile_combat_fraction",
    "enemy_static_combat_fraction",
    "enemy_builder_fraction",
    "enemy_factory_fraction",
    "range_advantage",
    "dps_advantage",
];
pub fn ground_power(unit: &UnitDef) -> (f32, f32) {
    unit.weapons
        .iter()
        .filter(|w| {
            !w.paralyzer
                && !w.is_shield
                && w.interceptor == 0
                && w.required_capabilities.is_empty()
                && (w.only_target_categories.is_empty()
                    || w.only_target_categories.iter().any(|c| {
                        !matches!(
                            c.to_ascii_uppercase().as_str(),
                            "VTOL" | "AIR" | "SUB" | "UNDERWATER"
                        )
                    }))
        })
        .fold((0.0f32, 0.0f32), |(d, r), w| {
            (d + w.paper_dps(), r.max(w.range))
        })
}
fn bounded(x: f32, r: f32) -> f32 {
    let x = if x.is_finite() { x.max(0.0) } else { 0.0 };
    x / (x + r)
}
pub fn extras(candidate: &UnitDef, enemies: &[&UnitDef]) -> [f32; 11] {
    let (dps, range) = ground_power(candidate);
    let n = enemies.len().max(1) as f32;
    let combat: Vec<_> = enemies.iter().filter(|e| ground_power(e).0 > 0.0).collect();
    let mean = |f: fn(&UnitDef) -> f32| {
        combat.iter().map(|e| f(e)).sum::<f32>() / combat.len().max(1) as f32
    };
    let advantage = |a: f32, b: f32| {
        if combat.is_empty() {
            0.0
        } else {
            ((a - b) / (a + b + 1.0)).clamp(-1.0, 1.0)
        }
    };
    [
        bounded(candidate.energy_cost, 1000.0),
        bounded(candidate.strategic.build_time, 1000.0),
        bounded(dps, 100.0),
        bounded(range, 500.0),
        bounded(candidate.movement.max_speed, 100.0),
        enemies
            .iter()
            .filter(|e| e.strategic.is_mobile && ground_power(e).0 > 0.0)
            .count() as f32
            / n,
        enemies
            .iter()
            .filter(|e| !e.strategic.is_mobile && ground_power(e).0 > 0.0)
            .count() as f32
            / n,
        enemies.iter().filter(|e| e.strategic.is_builder).count() as f32 / n,
        enemies.iter().filter(|e| e.strategic.is_factory).count() as f32 / n,
        advantage(range, mean(|e| ground_power(e).1)),
        advantage(dps, mean(|e| ground_power(e).0)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn features_distinguish_cost_range_roles_and_empty_disclosure() {
        let mut candidate = UnitDef::default();
        candidate.energy_cost = 1000.0;
        candidate.strategic.build_time = 1000.0;
        let mut weapon = crate::model::WeaponDef::default();
        weapon.range = 500.0;
        weapon.damage = 100.0;
        candidate.weapons.push(weapon);
        let empty = extras(&candidate, &[]);
        assert_eq!(empty[0], 0.5);
        assert_eq!(empty[1], 0.5);
        assert_eq!(empty[3], 0.5);
        assert_eq!(&empty[5..], &[0.0; 6]);
        let mut enemy = candidate.clone();
        enemy.strategic.is_mobile = true;
        enemy.strategic.is_builder = true;
        enemy.weapons[0].range = 100.0;
        let visible = extras(&candidate, &[&enemy]);
        assert_eq!(visible[5], 1.0);
        assert_eq!(visible[7], 1.0);
        assert!(visible[9] > 0.0);
        candidate.energy_cost = 0.0;
        assert_eq!(extras(&candidate, &[&enemy])[0], 0.0);
        candidate.weapons[0].only_target_categories = vec!["VTOL".into()];
        assert_eq!(ground_power(&candidate), (0.0, 0.0));
        candidate.weapons[0].only_target_categories.clear();
        candidate.weapons[0].paralyzer = true;
        assert_eq!(ground_power(&candidate), (0.0, 0.0));
    }
}
