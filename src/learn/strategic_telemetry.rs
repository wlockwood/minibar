//! Evaluation-only event ledger. No information is exposed to a policy.
use crate::world::{EntityRole, World, WorldEvent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrialTelemetry {
    pub terminal_outcome: String,
    pub combat_enabled: bool,
    pub own_surviving_value: f64,
    pub enemy_surviving_value: f64,
    pub production_value: f64,
    pub completed_entity_value_lost: f64,
    pub enemy_completed_entity_value_lost: f64,
    pub reclaimed_value: f64,
}
pub struct TrialLedger {
    known: BTreeMap<u64, (u8, f64, bool)>,
    production_baseline: [f64; 2],
    destroyed: BTreeSet<u64>,
    pub production_value: [f64; 2],
    lost: [f64; 2],
    reclaimed: [f64; 2],
}
impl TrialLedger {
    pub fn new(world: &World) -> Self {
        let mut s = Self {
            known: BTreeMap::new(),
            production_baseline: [
                world.teams[0].metrics.production_value,
                world.teams[1].metrics.production_value,
            ],
            destroyed: BTreeSet::new(),
            production_value: [0.0; 2],
            lost: [0.0; 2],
            reclaimed: [0.0; 2],
        };
        s.remember(world);
        s
    }
    fn remember(&mut self, world: &World) {
        for (id, e) in world
            .entities
            .iter()
            .filter(|(_, e)| e.role != EntityRole::Feature)
        {
            if let Some(d) = world.catalog.get(e.definition) {
                self.known.insert(
                    id.to_raw(),
                    (
                        e.owner,
                        d.resource_value() as f64,
                        e.lifecycle != crate::world::EntityLifecycle::UnderConstruction
                            && world.construction_progress(id).is_none_or(|p| p >= 1.0),
                    ),
                );
            }
        }
    }
    pub fn observe(&mut self, world: &World) {
        self.remember(world);
        // World metrics count actual releases even if a product dies in the same
        // simulation step before it can be seen by this diagnostic observer.
        for side in 0..2 {
            self.production_value[side] =
                world.teams[side].metrics.production_value - self.production_baseline[side];
        }
        for event in &world.events {
            if let WorldEvent::FactoryReleased { entity, .. }
            | WorldEvent::EntityActivated { entity } = event
            {
                if let Some((_, _, complete)) = self.known.get_mut(&entity.to_raw()) {
                    *complete = true;
                }
            }
        }
        let reclaimed: BTreeSet<_> = world
            .events
            .iter()
            .filter_map(|e| {
                if let WorldEvent::Reclaimed { entity, .. } = e {
                    Some(entity.to_raw())
                } else {
                    None
                }
            })
            .collect();
        for event in &world.events {
            match event {
                WorldEvent::EntityDestroyed { entity }
                    if self.destroyed.insert(entity.to_raw()) =>
                {
                    if let Some((side, value, complete)) = self.known.get(&entity.to_raw()) {
                        if *complete {
                            if reclaimed.contains(&entity.to_raw()) {
                                self.reclaimed[*side as usize] += value;
                            } else {
                                self.lost[*side as usize] += value;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    pub fn summary(&self, world: &World, side: u8, combat: bool) -> TrialTelemetry {
        let mut counts = [0usize; 2];
        let mut values = [0.0; 2];
        for (_, e) in world
            .entities
            .iter()
            .filter(|(_, e)| e.role != EntityRole::Feature && e.health > 0.0)
        {
            counts[e.owner as usize] += 1;
            if let Some(d) = world.catalog.get(e.definition) {
                values[e.owner as usize] += d.resource_value() as f64
                    * (e.health / (d.health as f64).max(f64::EPSILON)).clamp(0.0, 1.0);
            }
        }
        let me = side as usize;
        let other = 1 - me;
        let outcome = if !combat {
            "unresolved"
        } else {
            match (counts[me] > 0, counts[other] > 0) {
                (true, false) => "win",
                (false, true) => "loss",
                (false, false) => "mutual_elimination",
                _ => "unresolved",
            }
        };
        TrialTelemetry {
            terminal_outcome: outcome.into(),
            combat_enabled: combat,
            own_surviving_value: values[me],
            enemy_surviving_value: values[other],
            production_value: self.production_value[me],
            completed_entity_value_lost: self.lost[me],
            enemy_completed_entity_value_lost: self.lost[other],
            reclaimed_value: self.reclaimed[me],
        }
    }
}
