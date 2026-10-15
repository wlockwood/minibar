//! Reusable trial-local broad phase for unit-centre queries.
//!
//! Each live unit occupies one grid cell. Callers expand query bounds by the
//! maximum unit radius, then perform their exact cylinder/segment test on the
//! returned candidates. The intrusive head/next representation allocates only
//! when unit or map capacity grows and does not allocate per grid cell.

use crate::model::{Terrain, UnitState, Vec2};

#[derive(Debug)]
pub(crate) struct SpatialGrid {
    width: f32,
    depth: f32,
    cell_size: f32,
    cols: usize,
    rows: usize,
    heads: Vec<i32>,
    head_epochs: Vec<u32>,
    next: Vec<i32>,
    maximum_radius: f32,
    generation: u32,
    indexed: bool,
}

impl SpatialGrid {
    pub(crate) fn new(terrain: &Terrain, cell_size: f32) -> Self {
        let width = terrain.cols as f32 * terrain.cell_size;
        let depth = terrain.rows as f32 * terrain.cell_size;
        let cell_size = cell_size.max(terrain.cell_size).max(1.0);
        let cols = (width / cell_size).ceil().max(1.0) as usize;
        let rows = (depth / cell_size).ceil().max(1.0) as usize;
        Self {
            width,
            depth,
            cell_size,
            cols,
            rows,
            heads: Vec::new(),
            head_epochs: Vec::new(),
            next: Vec::new(),
            maximum_radius: 0.0,
            generation: 0,
            indexed: false,
        }
    }

    pub(crate) fn rebuild(&mut self, units: &[UnitState]) {
        // Below this crossover, a linear scan is faster and requires no grid
        // allocation. Dense scenarios amortize the index construction.
        self.indexed = units.len() >= 16;
        self.maximum_radius = 0.0;
        if !self.indexed {
            return;
        }
        if self.heads.is_empty() {
            self.heads.resize(self.cols * self.rows, -1);
            self.head_epochs.resize(self.cols * self.rows, 0);
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.head_epochs.fill(0);
            self.generation = 1;
        }
        if self.next.len() < units.len() {
            self.next.resize(units.len(), -1);
        }
        // Reverse insertion leaves indices ascending within each cell. Cell
        // traversal order is deterministic across trials and worker counts.
        for (index, unit) in units.iter().enumerate().rev() {
            if !unit.alive {
                continue;
            }
            self.maximum_radius = self
                .maximum_radius
                .max(unit.definition.movement.radius.max(0.0));
            let Some(cell) = self.cell_index(unit.position) else {
                continue;
            };
            self.next[index] = if self.head_epochs[cell] == self.generation {
                self.heads[cell]
            } else {
                -1
            };
            self.heads[cell] = index as i32;
            self.head_epochs[cell] = self.generation;
        }
    }

    pub(crate) fn maximum_radius(&self) -> f32 {
        self.maximum_radius
    }

    pub(crate) fn is_indexed(&self) -> bool {
        self.indexed
    }

    pub(crate) fn query_aabb(&self, minimum: Vec2, maximum: Vec2, output: &mut Vec<usize>) {
        output.clear();
        debug_assert!(self.indexed);
        let Some((min_col, max_col)) =
            cell_span(minimum.x, maximum.x, self.width, self.cell_size, self.cols)
        else {
            return;
        };
        let Some((min_row, max_row)) =
            cell_span(minimum.z, maximum.z, self.depth, self.cell_size, self.rows)
        else {
            return;
        };
        for row in min_row..=max_row {
            for col in min_col..=max_col {
                let cell = row * self.cols + col;
                if self.head_epochs[cell] != self.generation {
                    continue;
                }
                let mut current = self.heads[cell];
                while current >= 0 {
                    let index = current as usize;
                    output.push(index);
                    current = self.next[index];
                }
            }
        }
    }

    fn cell_index(&self, position: Vec2) -> Option<usize> {
        if !position.x.is_finite()
            || !position.z.is_finite()
            || position.x < 0.0
            || position.z < 0.0
            || position.x >= self.width
            || position.z >= self.depth
        {
            return None;
        }
        let col = (position.x / self.cell_size) as usize;
        let row = (position.z / self.cell_size) as usize;
        Some(row * self.cols + col)
    }
}

fn cell_span(
    minimum: f32,
    maximum: f32,
    extent: f32,
    cell_size: f32,
    cells: usize,
) -> Option<(usize, usize)> {
    if !minimum.is_finite()
        || !maximum.is_finite()
        || maximum < 0.0
        || minimum >= extent
        || minimum > maximum
    {
        return None;
    }
    let first = (minimum.max(0.0) / cell_size) as usize;
    let last = (maximum.min(extent - f32::EPSILON) / cell_size) as usize;
    Some((first.min(cells - 1), last.min(cells - 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MovementDef, StrategicDef, UnitDef, WeaponState};
    use std::sync::Arc;

    fn unit(id: u32, position: Vec2, radius: f32, alive: bool) -> UnitState {
        UnitState {
            id,
            side: 0,
            definition: Arc::new(UnitDef {
                name: format!("unit-{id}"),
                icon_type: None,
                health: 100.0,
                metal_cost: 1.0,
                energy_cost: 0.0,
                strategic: StrategicDef::default(),
                movement: MovementDef {
                    radius,
                    ..Default::default()
                },
                weapons: vec![],
            }),
            definition_index: id as usize - 1,
            position,
            heading: 0.0,
            forward: Vec2::new(1.0, 0.0),
            speed: 0.0,
            hp: 100.0,
            collision_height_scale: 1.0,
            paralyze_damage: 0.0,
            stunned: false,
            alive,
            weapons: Vec::<WeaponState>::new(),
        }
    }

    #[test]
    fn query_returns_only_live_centres_in_intersected_cells() {
        let terrain = Terrain::flat(512.0, 512.0, 16.0);
        let units = vec![
            unit(1, Vec2::new(20.0, 20.0), 5.0, true),
            unit(2, Vec2::new(140.0, 20.0), 12.0, true),
            unit(3, Vec2::new(145.0, 25.0), 50.0, false),
            unit(4, Vec2::new(400.0, 400.0), 8.0, true),
            unit(5, Vec2::new(300.0, 40.0), 4.0, true),
            unit(6, Vec2::new(300.0, 80.0), 4.0, true),
            unit(7, Vec2::new(300.0, 120.0), 4.0, true),
            unit(8, Vec2::new(300.0, 160.0), 4.0, true),
            unit(9, Vec2::new(310.0, 40.0), 4.0, true),
            unit(10, Vec2::new(310.0, 80.0), 4.0, true),
            unit(11, Vec2::new(310.0, 120.0), 4.0, true),
            unit(12, Vec2::new(310.0, 160.0), 4.0, true),
            unit(13, Vec2::new(320.0, 40.0), 4.0, true),
            unit(14, Vec2::new(320.0, 80.0), 4.0, true),
            unit(15, Vec2::new(320.0, 120.0), 4.0, true),
            unit(16, Vec2::new(320.0, 160.0), 4.0, true),
        ];
        let mut grid = SpatialGrid::new(&terrain, 128.0);
        grid.rebuild(&units);
        assert_eq!(grid.maximum_radius(), 12.0);

        let mut found = Vec::new();
        grid.query_aabb(Vec2::new(0.0, 0.0), Vec2::new(255.0, 127.0), &mut found);
        found.sort_unstable();
        assert_eq!(found, [0, 1]);

        grid.query_aabb(Vec2::new(350.0, 350.0), Vec2::new(450.0, 450.0), &mut found);
        assert!(found.contains(&3));
    }
}
