//! Deterministic resource accounting for a single simulated team.
//!
//! Resource demand is allocated by descending priority. All demands in one
//! priority tier receive the same fulfillment fraction, which makes builders
//! and factories independent of iteration order while preserving stalls.

use std::cmp::Reverse;

pub type DemandId = u64;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Resources {
    pub metal: f64,
    pub energy: f64,
}

impl Resources {
    pub const ZERO: Self = Self {
        metal: 0.0,
        energy: 0.0,
    };

    pub const fn new(metal: f64, energy: f64) -> Self {
        Self { metal, energy }
    }

    pub fn non_negative(self) -> Self {
        Self {
            metal: finite_non_negative(self.metal),
            energy: finite_non_negative(self.energy),
        }
    }

    pub fn scaled(self, scale: f64) -> Self {
        let scale = finite_non_negative(scale);
        Self::new(self.metal * scale, self.energy * scale)
    }

    pub fn is_zero(self) -> bool {
        self.metal <= f64::EPSILON && self.energy <= f64::EPSILON
    }
}

impl std::ops::Add for Resources {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.metal + rhs.metal, self.energy + rhs.energy)
    }
}

impl std::ops::AddAssign for Resources {
    fn add_assign(&mut self, rhs: Self) {
        self.metal += rhs.metal;
        self.energy += rhs.energy;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResourceStats {
    pub generated: f64,
    pub spent: f64,
    pub overflow: f64,
    pub unmet: f64,
    /// Sum of demand-seconds in which this resource was partially or wholly
    /// unavailable. Demands that do not request this resource do not count.
    pub stalled_demand_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResourcePool {
    pub stored: f64,
    pub capacity: f64,
    pub income_per_second: f64,
    pub stats: ResourceStats,
}

impl ResourcePool {
    pub fn new(stored: f64, capacity: f64, income_per_second: f64) -> Self {
        let capacity = finite_non_negative(capacity);
        Self {
            stored: finite_non_negative(stored).min(capacity),
            capacity,
            income_per_second: finite_non_negative(income_per_second),
            stats: ResourceStats::default(),
        }
    }

    fn gather(&mut self, seconds: f64) {
        let generated = self.income_per_second * finite_non_negative(seconds);
        self.stats.generated += generated;
        let accepted = generated.min((self.capacity - self.stored).max(0.0));
        self.stored += accepted;
        self.stats.overflow += generated - accepted;
    }

    pub(crate) fn spend(&mut self, amount: f64) {
        let amount = finite_non_negative(amount).min(self.stored);
        self.stored -= amount;
        if self.stored.abs() < 1e-12 {
            self.stored = 0.0;
        }
        self.stats.spent += amount;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResourceDemand {
    pub id: DemandId,
    /// Higher values are serviced before lower values.
    pub priority: i32,
    /// Absolute resource requested during this tick, not a per-second rate.
    pub requested: Resources,
}

impl ResourceDemand {
    pub fn from_rate(id: DemandId, priority: i32, per_second: Resources, seconds: f64) -> Self {
        Self {
            id,
            priority,
            requested: per_second.scaled(seconds),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResourceAllocation {
    pub id: DemandId,
    pub priority: i32,
    pub requested: Resources,
    pub supplied: Resources,
    pub fraction: f64,
}

impl ResourceAllocation {
    pub fn stalled(self) -> bool {
        !self.requested.is_zero() && self.fraction < 1.0 - 1e-12
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TeamEconomy {
    pub metal: ResourcePool,
    pub energy: ResourcePool,
    /// Sum of the partially or wholly starved demand-seconds. A tick with two
    /// half-starved consumers contributes one demand-second.
    pub stalled_demand_seconds: f64,
}

impl TeamEconomy {
    pub fn new(metal: ResourcePool, energy: ResourcePool) -> Self {
        Self {
            metal,
            energy,
            stalled_demand_seconds: 0.0,
        }
    }

    /// Adds income and records storage overflow. Call exactly once per tick.
    pub fn gather_income(&mut self, seconds: f64) {
        self.metal.gather(seconds);
        self.energy.gather(seconds);
    }

    /// Services upkeep, construction, repair, or weapon demands. Results are
    /// sorted by descending priority and then stable demand ID.
    pub fn allocate(
        &mut self,
        demands: &[ResourceDemand],
        seconds: f64,
    ) -> Vec<ResourceAllocation> {
        let seconds = finite_non_negative(seconds);
        let mut ordered: Vec<_> = demands
            .iter()
            .copied()
            .map(|mut demand| {
                demand.requested = demand.requested.non_negative();
                demand
            })
            .collect();
        ordered.sort_by_key(|demand| (Reverse(demand.priority), demand.id));

        let mut allocations = Vec::with_capacity(ordered.len());
        let mut start = 0;
        while start < ordered.len() {
            let priority = ordered[start].priority;
            let end = ordered[start..]
                .iter()
                .position(|demand| demand.priority != priority)
                .map_or(ordered.len(), |relative| start + relative);
            let tier = &ordered[start..end];
            let total = tier
                .iter()
                .fold(Resources::ZERO, |sum, demand| sum + demand.requested);
            let fraction = limiting_fraction(
                self.metal.stored,
                total.metal,
                self.energy.stored,
                total.energy,
            );
            let supplied_total = total.scaled(fraction);
            self.metal.spend(supplied_total.metal);
            self.energy.spend(supplied_total.energy);

            for demand in tier {
                let supplied = demand.requested.scaled(fraction);
                let unmet = Resources::new(
                    demand.requested.metal - supplied.metal,
                    demand.requested.energy - supplied.energy,
                );
                self.metal.stats.unmet += unmet.metal;
                self.energy.stats.unmet += unmet.energy;
                if !demand.requested.is_zero() {
                    self.stalled_demand_seconds += seconds * (1.0 - fraction);
                }
                if demand.requested.metal > f64::EPSILON {
                    self.metal.stats.stalled_demand_seconds += seconds * (1.0 - fraction);
                }
                if demand.requested.energy > f64::EPSILON {
                    self.energy.stats.stalled_demand_seconds += seconds * (1.0 - fraction);
                }
                allocations.push(ResourceAllocation {
                    id: demand.id,
                    priority: demand.priority,
                    requested: demand.requested,
                    supplied,
                    fraction,
                });
            }
            start = end;
        }
        allocations
    }

    /// Convenience for the common gather-then-spend portion of a tick.
    pub fn run_tick(
        &mut self,
        seconds: f64,
        demands: &[ResourceDemand],
    ) -> Vec<ResourceAllocation> {
        self.gather_income(seconds);
        self.allocate(demands, seconds)
    }
}

fn limiting_fraction(
    metal_available: f64,
    metal_requested: f64,
    energy_available: f64,
    energy_requested: f64,
) -> f64 {
    fn resource_fraction(available: f64, requested: f64) -> f64 {
        if requested <= f64::EPSILON {
            1.0
        } else {
            (available / requested).clamp(0.0, 1.0)
        }
    }

    resource_fraction(metal_available, metal_requested)
        .min(resource_fraction(energy_available, energy_requested))
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn economy(metal: f64, energy: f64) -> TeamEconomy {
        TeamEconomy::new(
            ResourcePool::new(metal, 1_000.0, 0.0),
            ResourcePool::new(energy, 1_000.0, 0.0),
        )
    }

    #[test]
    fn income_is_stored_and_overflow_is_recorded() {
        let mut team = TeamEconomy::new(
            ResourcePool::new(9.0, 10.0, 4.0),
            ResourcePool::new(0.0, 20.0, 5.0),
        );
        team.gather_income(2.0);
        assert_eq!(team.metal.stored, 10.0);
        assert_eq!(team.metal.stats.generated, 8.0);
        assert_eq!(team.metal.stats.overflow, 7.0);
        assert_eq!(team.energy.stored, 10.0);
    }

    #[test]
    fn equal_priority_demands_stall_pro_rata_independent_of_input_order() {
        let demands = [
            ResourceDemand {
                id: 20,
                priority: 0,
                requested: Resources::new(75.0, 15.0),
            },
            ResourceDemand {
                id: 10,
                priority: 0,
                requested: Resources::new(25.0, 5.0),
            },
        ];
        let mut forward = economy(50.0, 100.0);
        let mut reverse = economy(50.0, 100.0);
        let first = forward.allocate(&demands, 1.0);
        let second = reverse.allocate(&[demands[1], demands[0]], 1.0);
        assert_eq!(first, second);
        assert!(first.iter().all(|allocation| allocation.fraction == 0.5));
        assert_eq!(forward.metal.stored, 0.0);
        assert_eq!(forward.energy.stored, 90.0);
        assert_eq!(forward.stalled_demand_seconds, 1.0);
        assert_eq!(forward.metal.stats.stalled_demand_seconds, 1.0);
        assert_eq!(forward.energy.stats.stalled_demand_seconds, 1.0);
    }

    #[test]
    fn high_priority_upkeep_is_paid_before_construction() {
        let demands = [
            ResourceDemand {
                id: 2,
                priority: 0,
                requested: Resources::new(10.0, 0.0),
            },
            ResourceDemand {
                id: 1,
                priority: 100,
                requested: Resources::new(6.0, 0.0),
            },
        ];
        let mut team = economy(10.0, 0.0);
        let allocations = team.allocate(&demands, 1.0);
        assert_eq!(allocations[0].id, 1);
        assert_eq!(allocations[0].fraction, 1.0);
        assert_eq!(allocations[1].fraction, 0.4);
    }

    #[test]
    fn resource_stall_seconds_only_count_resources_requested() {
        let demands = [ResourceDemand {
            id: 1,
            priority: 0,
            requested: Resources::new(10.0, 0.0),
        }];
        let mut team = economy(5.0, 0.0);
        team.allocate(&demands, 0.25);
        assert_eq!(team.metal.stats.stalled_demand_seconds, 0.125);
        assert_eq!(team.energy.stats.stalled_demand_seconds, 0.0);
    }
}
