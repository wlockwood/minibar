//! Local tactical routes around neutral blocking features. Strategic worlds use
//! their occupancy-grid router; this bounded visibility graph preserves a
//! tactical policy's destination while detouring around nearby wreck cylinders.
use crate::model::{Command, Terrain, UnitState, Vec2};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

#[derive(Default)]
pub(crate) struct WreckNavigator {
    topology: Vec<(u32, Vec2, f32)>,
    routes: HashMap<u32, CachedRoute>,
}

struct CachedRoute {
    goal: Vec2,
    radius: f32,
    points: Vec<Vec2>,
    next: usize,
}

impl WreckNavigator {
    pub fn clear(&mut self) {
        self.topology.clear();
        self.routes.clear();
    }

    pub fn steer(&mut self, units: &[UnitState], commands: &mut [Command], terrain: &Terrain) {
        let topology: Vec<_> = units
            .iter()
            .filter(|u| u.alive && u.side >= 2 && u.definition.movement.blocks_movement)
            .map(|u| (u.id, u.position, u.definition.movement.radius))
            .collect();
        if topology != self.topology {
            self.routes.clear();
            self.topology = topology;
        }
        if self.topology.is_empty() {
            return;
        }
        for (unit, command) in units.iter().zip(commands.iter_mut()) {
            if !unit.alive
                || unit.side >= 2
                || unit.definition.flight.is_some()
                || unit.definition.movement.max_speed <= 0.0
            {
                continue;
            }
            let Some(goal) = command.destination else {
                self.routes.remove(&unit.id);
                continue;
            };
            let radius = unit.definition.movement.radius;
            let refresh = self
                .routes
                .get(&unit.id)
                .is_none_or(|r| r.radius != radius || r.goal.distance_squared(goal) > 48.0 * 48.0);
            if refresh {
                let obstacles: Vec<_> = self
                    .topology
                    .iter()
                    .map(|(_, position, r)| (*position, *r + radius + 3.0))
                    .collect();
                if clear(unit.position, goal, &obstacles) {
                    self.routes.remove(&unit.id);
                    continue;
                }
                self.routes.insert(
                    unit.id,
                    CachedRoute {
                        goal,
                        radius,
                        points: route(unit.position, goal, radius, &obstacles, terrain),
                        next: 0,
                    },
                );
            }
            if let Some(route) = self.routes.get_mut(&unit.id) {
                while route.next < route.points.len()
                    && unit.position.distance(route.points[route.next])
                        < (unit.definition.movement.max_speed * crate::model::DT + 4.0).max(8.0)
                {
                    route.next += 1;
                }
                if let Some(next) = route.points.get(route.next) {
                    command.destination = Some(*next);
                } else {
                    self.routes.remove(&unit.id);
                }
            }
        }
    }
}

fn clear(from: Vec2, to: Vec2, obstacles: &[(Vec2, f32)]) -> bool {
    let delta = to - from;
    let length = delta.length_squared();
    obstacles.iter().all(|(center, radius)| {
        let fraction = if length > 1e-6 {
            let p = *center - from;
            ((p.x * delta.x + p.z * delta.z) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (from + delta * fraction).distance_squared(*center) >= radius * radius - 0.001
    })
}

// A new corpse can appear beside an existing mover. Permit an initial edge
// out of its padded clearance only when distance from every containing circle
// increases monotonically; ordinary edges still require full clearance.
fn escape_clear(from: Vec2, to: Vec2, obstacles: &[(Vec2, f32)]) -> bool {
    obstacles.iter().all(|(center, radius)| {
        let offset = from - *center;
        if offset.length_squared() < radius * radius {
            let delta = to - from;
            offset.x * delta.x + offset.z * delta.z >= -0.001
                && to.distance_squared(*center) >= radius * radius
        } else {
            clear(from, to, &[(*center, *radius)])
        }
    })
}

fn route(
    start: Vec2,
    goal: Vec2,
    radius: f32,
    obstacles: &[(Vec2, f32)],
    terrain: &Terrain,
) -> Vec<Vec2> {
    // Only nearby obstacles supply graph vertices. Every edge is still checked
    // against ALL obstacles, so the bound cannot make a route cross a far wreck.
    let mut nearby: Vec<_> = obstacles.iter().copied().collect();
    nearby.sort_by(|a, b| {
        a.0.distance_squared(start)
            .total_cmp(&b.0.distance_squared(start))
    });
    nearby.truncate(16);
    let mut nodes = vec![start, goal];
    for (center, clearance) in nearby {
        // A circumscribed octagon keeps its edges outside each clearance circle.
        let ring = (clearance + 5.0) / (PI / 8.0).cos();
        for i in 0..8 {
            let point = center + Vec2::from_heading(i as f32 * TAU / 8.0) * ring;
            if terrain.is_passable_with_radius(point, radius) && clear(point, point, obstacles) {
                nodes.push(point);
            }
        }
    }
    let mut cost = vec![f32::INFINITY; nodes.len()];
    let mut previous = vec![usize::MAX; nodes.len()];
    let mut visited = vec![false; nodes.len()];
    cost[0] = 0.0;
    for _ in 0..nodes.len() {
        let Some(current) = (0..nodes.len())
            .filter(|i| !visited[*i] && cost[*i].is_finite())
            .min_by(|a, b| cost[*a].total_cmp(&cost[*b]))
        else {
            break;
        };
        visited[current] = true;
        if current == 1 {
            break;
        }
        for next in 1..nodes.len() {
            if visited[next] {
                continue;
            }
            let distance = nodes[current].distance(nodes[next]);
            if cost[current] + distance >= cost[next]
                || !(if current == 0 {
                    escape_clear(nodes[current], nodes[next], obstacles)
                } else {
                    clear(nodes[current], nodes[next], obstacles)
                })
            {
                continue;
            }
            let samples = (distance / (terrain.cell_size * 0.5).max(1.0)).ceil() as usize;
            if !(0..=samples).all(|i| {
                terrain.is_passable_with_radius(
                    nodes[current]
                        + (nodes[next] - nodes[current]) * (i as f32 / samples.max(1) as f32),
                    radius,
                )
            }) {
                continue;
            }
            cost[next] = cost[current] + distance;
            previous[next] = current;
        }
    }
    let finish = if cost[1].is_finite() {
        1
    } else {
        (0..nodes.len())
            .filter(|i| cost[*i].is_finite())
            .min_by(|a, b| {
                nodes[*a]
                    .distance_squared(goal)
                    .total_cmp(&nodes[*b].distance_squared(goal))
            })
            .unwrap_or(0)
    };
    let mut points = Vec::new();
    let mut current = finish;
    while current != 0 && current != usize::MAX {
        points.push(nodes[current]);
        current = previous[current];
    }
    points.reverse();
    if points.is_empty() {
        points.push(start);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_unit_touching_a_wreck_can_escape_clearance_and_detour() {
        let start = Vec2::new(70.0, 100.0);
        let goal = Vec2::new(150.0, 100.0);
        let path = route(
            start,
            goal,
            10.0,
            &[(Vec2::new(100.0, 100.0), 33.0)],
            &Terrain::flat(300.0, 300.0, 16.0),
        );
        assert_eq!(path.last(), Some(&goal));
        assert!(path.iter().any(|p| (p.z - 100.0).abs() > 20.0));
    }
}
