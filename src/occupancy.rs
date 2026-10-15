//! Layered, counted occupancy grid and topology epochs.
//!
//! Replaces boolean occupancy with counted layers for movement, fire blocking
//! (with height), construction reservations, factory yards, and resource nodes.
//! Topology epochs allow pathfinding and LOS caches to invalidate only when
//! their effective geometry changes.

use crate::model::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

pub const DEFAULT_BUILD_GRID_CELL_SIZE: f32 = 16.0;

/// Stable identifier for units, nanoframes, walls, construction projects,
/// factory yards, or reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OccupantId(pub u64);

impl From<u64> for OccupantId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

/// The discrete layers tracked by the occupancy grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OccupancyLayerKind {
    Movement,
    Fire,
    ConstructionReservation,
    FactoryYard,
    Resource,
}

/// The set of layers contributed by an occupant to a specific cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellLayers {
    pub blocks_movement: bool,
    pub fire_height: Option<f32>,
    pub construction_reservation: bool,
    pub factory_yard: bool,
}

impl CellLayers {
    pub const EMPTY: Self = Self {
        blocks_movement: false,
        fire_height: None,
        construction_reservation: false,
        factory_yard: false,
    };

    pub const MOVEMENT_ONLY: Self = Self {
        blocks_movement: true,
        fire_height: None,
        construction_reservation: false,
        factory_yard: false,
    };

    pub const RESERVATION_ONLY: Self = Self {
        blocks_movement: false,
        fire_height: None,
        construction_reservation: true,
        factory_yard: false,
    };

    pub const FACTORY_YARD_ONLY: Self = Self {
        blocks_movement: false,
        fire_height: None,
        construction_reservation: false,
        factory_yard: true,
    };

    pub fn wall(height: f32) -> Self {
        Self {
            blocks_movement: true,
            fire_height: Some(height.max(0.0)),
            construction_reservation: false,
            factory_yard: false,
        }
    }

    pub fn building(height: f32) -> Self {
        Self {
            blocks_movement: true,
            fire_height: Some(height.max(0.0)),
            construction_reservation: true,
            factory_yard: false,
        }
    }
}

/// A contribution from one occupant to a cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellContribution {
    pub occupant_id: OccupantId,
    pub layers: CellLayers,
}

/// A single cell in the build grid.
#[derive(Clone, Debug, Default)]
pub struct OccupancyCell {
    pub movement_count: u16,
    pub reservation_count: u16,
    pub factory_yard_count: u16,
    pub max_fire_height: f32,
    pub contributions: Vec<CellContribution>,
}

impl OccupancyCell {
    #[inline]
    pub fn is_movement_blocked(&self) -> bool {
        self.movement_count > 0
    }

    #[inline]
    pub fn is_reserved(&self) -> bool {
        self.reservation_count > 0
    }

    #[inline]
    pub fn is_factory_yard(&self) -> bool {
        self.factory_yard_count > 0
    }

    #[inline]
    pub fn has_fire_blocker(&self) -> bool {
        self.max_fire_height > 0.0
    }

    fn recompute_max_fire_height(&mut self) -> f32 {
        let mut max_h = 0.0_f32;
        for contrib in &self.contributions {
            if let Some(h) = contrib.layers.fire_height
                && h > max_h
            {
                max_h = h;
            }
        }
        self.max_fire_height = max_h;
        max_h
    }
}

/// Internal record of an occupant's registered presence in the grid.
#[derive(Clone, Debug)]
struct OccupantRecord {
    owner: Option<u8>,
    cells: Vec<(u32, u32)>,
    geo_vent: Option<usize>,
    metal_spot: Option<usize>,
    continuous_extractor: Option<(Vec2, f32)>,
}

/// Continuous metal extractor exclusion zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtractorSite {
    pub occupant_id: OccupantId,
    pub center: Vec2,
    pub radius: f32,
}

/// Multi-layer, reference-counted spatial occupancy grid.
#[derive(Clone, Debug)]
pub struct OccupancyGrid {
    cols: u32,
    rows: u32,
    cell_size: f32,
    cells: Vec<OccupancyCell>,
    occupants: BTreeMap<OccupantId, OccupantRecord>,
    geothermal_reservations: BTreeMap<usize, OccupantId>,
    metal_spot_reservations: BTreeMap<usize, OccupantId>,
    continuous_extractors: Vec<ExtractorSite>,
    movement_epoch: u64,
    fire_epoch: u64,
    reservation_epoch: u64,
    yard_epoch: u64,
    resource_epoch: u64,
    global_epoch: u64,
}

impl OccupancyGrid {
    pub fn new(width: f32, depth: f32, cell_size: f32) -> Result<Self, String> {
        if !width.is_finite() || width <= 0.0 || !depth.is_finite() || depth <= 0.0 {
            return Err("dimensions must be finite and positive".into());
        }
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err("cell size must be finite and positive".into());
        }
        let cols = (width / cell_size).ceil() as u32;
        let rows = (depth / cell_size).ceil() as u32;
        let total = cols as usize * rows as usize;
        let mut cells = Vec::with_capacity(total);
        cells.resize_with(total, OccupancyCell::default);
        Ok(Self {
            cols,
            rows,
            cell_size,
            cells,
            occupants: BTreeMap::new(),
            geothermal_reservations: BTreeMap::new(),
            metal_spot_reservations: BTreeMap::new(),
            continuous_extractors: Vec::new(),
            movement_epoch: 0,
            fire_epoch: 0,
            reservation_epoch: 0,
            yard_epoch: 0,
            resource_epoch: 0,
            global_epoch: 0,
        })
    }

    #[inline]
    pub fn cols(&self) -> u32 {
        self.cols
    }

    #[inline]
    pub fn rows(&self) -> u32 {
        self.rows
    }

    #[inline]
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    #[inline]
    pub fn width(&self) -> f32 {
        self.cols as f32 * self.cell_size
    }

    #[inline]
    pub fn depth(&self) -> f32 {
        self.rows as f32 * self.cell_size
    }

    #[inline]
    pub fn movement_epoch(&self) -> u64 {
        self.movement_epoch
    }

    #[inline]
    pub fn fire_epoch(&self) -> u64 {
        self.fire_epoch
    }

    #[inline]
    pub fn reservation_epoch(&self) -> u64 {
        self.reservation_epoch
    }

    #[inline]
    pub fn yard_epoch(&self) -> u64 {
        self.yard_epoch
    }

    #[inline]
    pub fn resource_epoch(&self) -> u64 {
        self.resource_epoch
    }

    /// Global epoch incremented on any state change.
    #[inline]
    pub fn epoch(&self) -> u64 {
        self.global_epoch
    }

    #[inline]
    pub fn occupant_owner(&self, occupant_id: OccupantId) -> Option<u8> {
        self.occupants.get(&occupant_id).and_then(|rec| rec.owner)
    }

    #[inline]
    pub fn index(&self, col: u32, row: u32) -> Option<usize> {
        if col < self.cols && row < self.rows {
            Some((row * self.cols + col) as usize)
        } else {
            None
        }
    }

    #[inline]
    pub fn coord_to_cell(&self, x: f32, z: f32) -> Option<(u32, u32)> {
        if !x.is_finite() || !z.is_finite() || x < 0.0 || z < 0.0 {
            return None;
        }
        let col = (x / self.cell_size) as u32;
        let row = (z / self.cell_size) as u32;
        if col < self.cols && row < self.rows {
            Some((col, row))
        } else {
            None
        }
    }

    #[inline]
    pub fn cell(&self, col: u32, row: u32) -> Option<&OccupancyCell> {
        self.index(col, row).map(|idx| &self.cells[idx])
    }

    /// Adds an occupant with specified contributions to a set of cells.
    /// Overlapping contributions are reference-safe and counted.
    pub fn add_occupant(
        &mut self,
        occupant_id: OccupantId,
        owner: Option<u8>,
        contributions: impl IntoIterator<Item = ((u32, u32), CellLayers)>,
    ) -> bool {
        if self.occupants.contains_key(&occupant_id) {
            return false;
        }

        let mut cells_list = Vec::new();
        let mut movement_topology_changed = false;
        let mut fire_topology_changed = false;
        let mut reservation_changed = false;
        let mut yard_changed = false;

        for ((col, row), layers) in contributions {
            let Some(idx) = self.index(col, row) else {
                continue;
            };
            cells_list.push((col, row));
            let cell = &mut self.cells[idx];

            if layers.blocks_movement {
                let was_blocked = cell.movement_count > 0;
                cell.movement_count = cell.movement_count.saturating_add(1);
                if !was_blocked {
                    movement_topology_changed = true;
                }
            }

            if layers.construction_reservation {
                cell.reservation_count = cell.reservation_count.saturating_add(1);
                reservation_changed = true;
            }

            if layers.factory_yard {
                cell.factory_yard_count = cell.factory_yard_count.saturating_add(1);
                yard_changed = true;
            }

            let old_max_fire = cell.max_fire_height;
            cell.contributions.push(CellContribution {
                occupant_id,
                layers,
            });
            let new_max_fire = cell.recompute_max_fire_height();
            if (old_max_fire - new_max_fire).abs() > 1e-4 {
                fire_topology_changed = true;
            }
        }

        self.occupants.insert(
            occupant_id,
            OccupantRecord {
                owner,
                cells: cells_list,
                geo_vent: None,
                metal_spot: None,
                continuous_extractor: None,
            },
        );

        if movement_topology_changed {
            self.movement_epoch = self.movement_epoch.wrapping_add(1);
        }
        if fire_topology_changed {
            self.fire_epoch = self.fire_epoch.wrapping_add(1);
        }
        if reservation_changed {
            self.reservation_epoch = self.reservation_epoch.wrapping_add(1);
        }
        if yard_changed {
            self.yard_epoch = self.yard_epoch.wrapping_add(1);
        }
        self.global_epoch = self.global_epoch.wrapping_add(1);

        true
    }

    /// Removes an occupant, cleanly releasing its contributions while preserving
    /// any overlapping contributions from other occupants.
    pub fn remove_occupant(&mut self, occupant_id: OccupantId) -> bool {
        let Some(record) = self.occupants.remove(&occupant_id) else {
            return false;
        };

        let mut movement_topology_changed = false;
        let mut fire_topology_changed = false;
        let mut reservation_changed = false;
        let mut yard_changed = false;

        for (col, row) in record.cells {
            let Some(idx) = self.index(col, row) else {
                continue;
            };
            let cell = &mut self.cells[idx];

            if let Some(pos) = cell
                .contributions
                .iter()
                .position(|c| c.occupant_id == occupant_id)
            {
                let removed = cell.contributions.swap_remove(pos);
                if removed.layers.blocks_movement {
                    let was_blocked = cell.movement_count > 0;
                    cell.movement_count = cell.movement_count.saturating_sub(1);
                    if was_blocked && cell.movement_count == 0 {
                        movement_topology_changed = true;
                    }
                }
                if removed.layers.construction_reservation {
                    cell.reservation_count = cell.reservation_count.saturating_sub(1);
                    reservation_changed = true;
                }
                if removed.layers.factory_yard {
                    cell.factory_yard_count = cell.factory_yard_count.saturating_sub(1);
                    yard_changed = true;
                }
                let old_max_fire = cell.max_fire_height;
                let new_max_fire = cell.recompute_max_fire_height();
                if (old_max_fire - new_max_fire).abs() > 1e-4 {
                    fire_topology_changed = true;
                }
            }
        }

        // Release geothermal reservation if any
        if let Some(vent) = record.geo_vent
            && self.geothermal_reservations.get(&vent) == Some(&occupant_id)
        {
            self.geothermal_reservations.remove(&vent);
            self.resource_epoch = self.resource_epoch.wrapping_add(1);
        }

        // Release metal spot reservation if any
        if let Some(spot) = record.metal_spot
            && self.metal_spot_reservations.get(&spot) == Some(&occupant_id)
        {
            self.metal_spot_reservations.remove(&spot);
            self.resource_epoch = self.resource_epoch.wrapping_add(1);
        }

        // Release continuous extractor site if any
        if record.continuous_extractor.is_some() {
            self.continuous_extractors
                .retain(|site| site.occupant_id != occupant_id);
            self.resource_epoch = self.resource_epoch.wrapping_add(1);
        }

        if movement_topology_changed {
            self.movement_epoch = self.movement_epoch.wrapping_add(1);
        }
        if fire_topology_changed {
            self.fire_epoch = self.fire_epoch.wrapping_add(1);
        }
        if reservation_changed {
            self.reservation_epoch = self.reservation_epoch.wrapping_add(1);
        }
        if yard_changed {
            self.yard_epoch = self.yard_epoch.wrapping_add(1);
        }
        self.global_epoch = self.global_epoch.wrapping_add(1);

        true
    }

    /// Updates the fire-blocking height of an occupant (e.g. wall or nanoframe)
    /// in-place without removing or reinserting unrelated layers.
    pub fn update_fire_height(&mut self, occupant_id: OccupantId, new_height: f32) -> bool {
        let Some(record) = self.occupants.get(&occupant_id) else {
            return false;
        };

        let height = new_height.max(0.0);
        let mut fire_topology_changed = false;

        for &(col, row) in &record.cells {
            let Some(idx) = self.index(col, row) else {
                continue;
            };
            let cell = &mut self.cells[idx];
            let mut found = false;
            for contrib in &mut cell.contributions {
                if contrib.occupant_id == occupant_id {
                    contrib.layers.fire_height = Some(height);
                    found = true;
                    break;
                }
            }
            if found {
                let old_max = cell.max_fire_height;
                let new_max = cell.recompute_max_fire_height();
                if (old_max - new_max).abs() > 1e-4 {
                    fire_topology_changed = true;
                }
            }
        }

        if fire_topology_changed {
            self.fire_epoch = self.fire_epoch.wrapping_add(1);
            self.global_epoch = self.global_epoch.wrapping_add(1);
        }

        true
    }

    pub fn reserve_geothermal(&mut self, occupant_id: OccupantId, vent_index: usize) -> bool {
        if self.geothermal_reservations.contains_key(&vent_index) {
            return false;
        }
        if let Some(record) = self.occupants.get_mut(&occupant_id) {
            record.geo_vent = Some(vent_index);
            self.geothermal_reservations.insert(vent_index, occupant_id);
            self.resource_epoch = self.resource_epoch.wrapping_add(1);
            self.global_epoch = self.global_epoch.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn is_geothermal_reserved(&self, vent_index: usize) -> Option<OccupantId> {
        self.geothermal_reservations.get(&vent_index).copied()
    }

    pub fn reserve_metal_spot(&mut self, occupant_id: OccupantId, spot_index: usize) -> bool {
        if self.metal_spot_reservations.contains_key(&spot_index) {
            return false;
        }
        if let Some(record) = self.occupants.get_mut(&occupant_id) {
            record.metal_spot = Some(spot_index);
            self.metal_spot_reservations.insert(spot_index, occupant_id);
            self.resource_epoch = self.resource_epoch.wrapping_add(1);
            self.global_epoch = self.global_epoch.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn is_metal_spot_reserved(&self, spot_index: usize) -> Option<OccupantId> {
        self.metal_spot_reservations.get(&spot_index).copied()
    }

    pub fn reserve_extractor_site(
        &mut self,
        occupant_id: OccupantId,
        center: Vec2,
        radius: f32,
    ) -> bool {
        if self
            .is_extractor_range_overlapping(center, radius, Some(occupant_id))
            .is_some()
        {
            return false;
        }
        if let Some(record) = self.occupants.get_mut(&occupant_id) {
            record.continuous_extractor = Some((center, radius));
            self.continuous_extractors.push(ExtractorSite {
                occupant_id,
                center,
                radius,
            });
            self.resource_epoch = self.resource_epoch.wrapping_add(1);
            self.global_epoch = self.global_epoch.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn is_extractor_range_overlapping(
        &self,
        center: Vec2,
        radius: f32,
        ignore_occupant: Option<OccupantId>,
    ) -> Option<OccupantId> {
        for site in &self.continuous_extractors {
            if Some(site.occupant_id) == ignore_occupant {
                continue;
            }
            let max_sep = radius + site.radius;
            if center.distance_squared(site.center) < max_sep * max_sep {
                return Some(site.occupant_id);
            }
        }
        None
    }

    #[inline]
    pub fn is_movement_blocked(&self, x: f32, z: f32) -> bool {
        self.coord_to_cell(x, z)
            .is_none_or(|(col, row)| self.is_movement_blocked_cell(col, row))
    }

    #[inline]
    pub fn is_movement_blocked_cell(&self, col: u32, row: u32) -> bool {
        self.index(col, row)
            .is_none_or(|idx| self.cells[idx].is_movement_blocked())
    }

    #[inline]
    pub fn movement_count_at(&self, x: f32, z: f32) -> u16 {
        self.coord_to_cell(x, z)
            .map_or(1, |(col, row)| self.movement_count_cell(col, row))
    }

    #[inline]
    pub fn movement_count_cell(&self, col: u32, row: u32) -> u16 {
        self.index(col, row)
            .map_or(1, |idx| self.cells[idx].movement_count)
    }

    #[inline]
    pub fn effective_fire_height_at(&self, x: f32, z: f32) -> f32 {
        self.coord_to_cell(x, z)
            .map_or(0.0, |(col, row)| self.effective_fire_height_cell(col, row))
    }

    #[inline]
    pub fn effective_fire_height_cell(&self, col: u32, row: u32) -> f32 {
        self.index(col, row)
            .map_or(0.0, |idx| self.cells[idx].max_fire_height)
    }

    #[inline]
    pub fn has_fire_blocker_at(&self, x: f32, z: f32) -> bool {
        self.effective_fire_height_at(x, z) > 0.0
    }

    #[inline]
    pub fn is_reserved_at(&self, x: f32, z: f32) -> bool {
        self.coord_to_cell(x, z)
            .is_some_and(|(col, row)| self.is_reserved_cell(col, row))
    }

    #[inline]
    pub fn is_reserved_cell(&self, col: u32, row: u32) -> bool {
        self.index(col, row)
            .is_some_and(|idx| self.cells[idx].is_reserved())
    }

    #[inline]
    pub fn reservation_count_cell(&self, col: u32, row: u32) -> u16 {
        self.index(col, row)
            .map_or(0, |idx| self.cells[idx].reservation_count)
    }

    #[inline]
    pub fn is_factory_yard_at(&self, x: f32, z: f32) -> bool {
        self.coord_to_cell(x, z)
            .is_some_and(|(col, row)| self.is_factory_yard_cell(col, row))
    }

    #[inline]
    pub fn is_factory_yard_cell(&self, col: u32, row: u32) -> bool {
        self.index(col, row)
            .is_some_and(|idx| self.cells[idx].is_factory_yard())
    }

    /// Converts an axis-aligned world bounding box to inclusive `(left, top, right, bottom)` cell bounds.
    pub fn cell_rect(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Option<(u32, u32, u32, u32)> {
        if ![min_x, min_z, max_x, max_z].iter().all(|v| v.is_finite())
            || min_x < 0.0
            || min_z < 0.0
            || max_x <= min_x
            || max_z <= min_z
            || max_x > self.cols as f32 * self.cell_size
            || max_z > self.rows as f32 * self.cell_size
        {
            return None;
        }
        let left = (min_x / self.cell_size).floor() as u32;
        let top = (min_z / self.cell_size).floor() as u32;
        let right = ((max_x / self.cell_size).ceil() as u32).saturating_sub(1);
        let bottom = ((max_z / self.cell_size).ceil() as u32).saturating_sub(1);
        Some((left, top, right, bottom))
    }

    /// Returns true if all cells in the specified bounding box have zero movement count.
    pub fn rect_movement_clear(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        let Some((left, top, right, bottom)) = self.cell_rect(min_x, min_z, max_x, max_z) else {
            return false;
        };
        for row in top..=bottom {
            for col in left..=right {
                if self.is_movement_blocked_cell(col, row) {
                    return false;
                }
            }
        }
        true
    }

    /// Tests whether a footprint of dimensions `(footprint_w, footprint_d)` in cells
    /// can be placed with its top-left at `(col, row)` without overlapping any movement blocker.
    pub fn footprint_cells_clear(
        &self,
        col: u32,
        row: u32,
        footprint_w: u16,
        footprint_d: u16,
    ) -> bool {
        let w = footprint_w as u32;
        let d = footprint_d as u32;
        if col + w > self.cols || row + d > self.rows {
            return false;
        }
        for r in row..(row + d) {
            for c in col..(col + w) {
                if self.is_movement_blocked_cell(c, r) {
                    return false;
                }
            }
        }
        true
    }

    /// Checks whether an axis-aligned box of `(footprint_w, footprint_d)` cells can navigate
    /// between `start` and `end` cell coordinates through non-movement-blocked cells.
    /// Uses 4-connected BFS for corridor passage evaluation.
    pub fn has_corridor_passage(
        &self,
        start: (u32, u32),
        end: (u32, u32),
        footprint_w: u16,
        footprint_d: u16,
    ) -> bool {
        if !self.footprint_cells_clear(start.0, start.1, footprint_w, footprint_d) {
            return false;
        }
        if !self.footprint_cells_clear(end.0, end.1, footprint_w, footprint_d) {
            return false;
        }
        if start == end {
            return true;
        }

        let mut visited = vec![false; self.cols as usize * self.rows as usize];
        let mut queue = VecDeque::new();

        visited[(start.1 * self.cols + start.0) as usize] = true;
        queue.push_back(start);

        let deltas = [(0_i32, -1_i32), (0, 1), (-1, 0), (1, 0)];

        while let Some((c, r)) = queue.pop_front() {
            if (c, r) == end {
                return true;
            }
            for (dc, dr) in deltas {
                let next_c = c as i32 + dc;
                let next_r = r as i32 + dr;
                if next_c < 0 || next_r < 0 {
                    continue;
                }
                let nc = next_c as u32;
                let nr = next_r as u32;
                if nc + footprint_w as u32 > self.cols || nr + footprint_d as u32 > self.rows {
                    continue;
                }
                let idx = (nr * self.cols + nc) as usize;
                if visited[idx] {
                    continue;
                }
                visited[idx] = true;
                if self.footprint_cells_clear(nc, nr, footprint_w, footprint_d) {
                    queue.push_back((nc, nr));
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_one_of_two_counted_contributors_preserves_the_other() {
        let mut grid = OccupancyGrid::new(64.0, 64.0, 16.0).unwrap();
        let occ1 = OccupantId(1);
        let occ2 = OccupantId(2);

        // Both place movement blocking on cell (1, 1)
        grid.add_occupant(occ1, Some(0), [((1, 1), CellLayers::MOVEMENT_ONLY)]);
        assert_eq!(grid.movement_count_cell(1, 1), 1);
        assert!(grid.is_movement_blocked_cell(1, 1));
        assert_eq!(grid.movement_epoch(), 1);

        grid.add_occupant(occ2, Some(0), [((1, 1), CellLayers::MOVEMENT_ONLY)]);
        assert_eq!(grid.movement_count_cell(1, 1), 2);
        assert!(grid.is_movement_blocked_cell(1, 1));
        // Effective topology did not change (already blocked)
        assert_eq!(grid.movement_epoch(), 1);

        // Remove occ1: cell must still be blocked by occ2
        assert!(grid.remove_occupant(occ1));
        assert_eq!(grid.movement_count_cell(1, 1), 1);
        assert!(grid.is_movement_blocked_cell(1, 1));
        assert_eq!(grid.movement_epoch(), 1);

        // Remove occ2: cell becomes clear, topology epoch increments
        assert!(grid.remove_occupant(occ2));
        assert_eq!(grid.movement_count_cell(1, 1), 0);
        assert!(!grid.is_movement_blocked_cell(1, 1));
        assert_eq!(grid.movement_epoch(), 2);
    }

    #[test]
    fn movement_fire_reservation_and_factory_yard_layers_remain_independent() {
        let mut grid = OccupancyGrid::new(64.0, 64.0, 16.0).unwrap();
        let res_occ = OccupantId(10);
        let yard_occ = OccupantId(20);

        // Reservation on (0, 0)
        grid.add_occupant(res_occ, Some(0), [((0, 0), CellLayers::RESERVATION_ONLY)]);
        assert!(grid.is_reserved_cell(0, 0));
        assert!(!grid.is_movement_blocked_cell(0, 0));
        assert_eq!(grid.effective_fire_height_cell(0, 0), 0.0);
        assert!(!grid.is_factory_yard_cell(0, 0));
        assert_eq!(grid.movement_epoch(), 0);
        assert_eq!(grid.fire_epoch(), 0);
        assert_eq!(grid.reservation_epoch(), 1);

        // Factory yard on (1, 1)
        grid.add_occupant(yard_occ, Some(0), [((1, 1), CellLayers::FACTORY_YARD_ONLY)]);
        assert!(grid.is_factory_yard_cell(1, 1));
        assert!(!grid.is_movement_blocked_cell(1, 1));
        assert!(!grid.is_reserved_cell(1, 1));
        assert_eq!(grid.movement_epoch(), 0);
        assert_eq!(grid.yard_epoch(), 1);
    }

    #[test]
    fn wall_height_updates_affect_fire_queries_not_movement_counts() {
        let mut grid = OccupancyGrid::new(64.0, 64.0, 16.0).unwrap();
        let wall_id = OccupantId(55);

        grid.add_occupant(wall_id, Some(0), [((1, 1), CellLayers::wall(10.0))]);
        assert_eq!(grid.effective_fire_height_cell(1, 1), 10.0);
        assert_eq!(grid.movement_count_cell(1, 1), 1);
        assert_eq!(grid.movement_epoch(), 1);
        assert_eq!(grid.fire_epoch(), 1);

        // Update wall height
        grid.update_fire_height(wall_id, 25.0);
        assert_eq!(grid.effective_fire_height_cell(1, 1), 25.0);
        assert_eq!(grid.movement_count_cell(1, 1), 1);
        assert_eq!(grid.movement_epoch(), 1); // Unchanged!
        assert_eq!(grid.fire_epoch(), 2); // Incremented!
    }

    #[test]
    fn continuous_wall_closes_corridor_and_gap_opens_it() {
        // 5x5 grid (80x80 world units)
        let mut grid = OccupancyGrid::new(80.0, 80.0, 16.0).unwrap();

        // Place a vertical wall down column 2 (cells (2, 0), (2, 1), (2, 2), (2, 3), (2, 4))
        for row in 0..5 {
            grid.add_occupant(
                OccupantId(100 + row),
                Some(0),
                [((2, row as u32), CellLayers::MOVEMENT_ONLY)],
            );
        }

        // Passage from left (0, 2) to right (4, 2) should be completely closed
        assert!(!grid.has_corridor_passage((0, 2), (4, 2), 1, 1));

        // Remove the wall segment at (2, 2) to create a 1-cell gap
        grid.remove_occupant(OccupantId(102));

        // Now a 1x1 footprint can pass through
        assert!(grid.has_corridor_passage((0, 2), (4, 2), 1, 1));
    }

    #[test]
    fn small_and_large_footprints_receive_different_corridor_results() {
        // 5x5 grid
        let mut grid = OccupancyGrid::new(80.0, 80.0, 16.0).unwrap();

        // Place wall down column 2 with a 1-cell gap at (2, 2)
        for row in [0, 1, 3, 4] {
            grid.add_occupant(
                OccupantId(200 + row),
                Some(0),
                [((2, row as u32), CellLayers::MOVEMENT_ONLY)],
            );
        }

        // Small 1x1 footprint fits through 1-cell gap
        assert!(grid.has_corridor_passage((0, 2), (4, 2), 1, 1));

        // Large 2x2 footprint CANNOT fit through 1-cell gap
        assert!(!grid.has_corridor_passage((0, 2), (3, 2), 2, 2));
    }

    #[test]
    fn epochs_change_exactly_when_documented_topology_changes() {
        let mut grid = OccupancyGrid::new(64.0, 64.0, 16.0).unwrap();
        let occ1 = OccupantId(1);
        let occ2 = OccupantId(2);

        assert_eq!(grid.movement_epoch(), 0);
        assert_eq!(grid.fire_epoch(), 0);
        assert_eq!(grid.reservation_epoch(), 0);

        // Reservation change does NOT touch movement or fire epochs
        grid.add_occupant(occ1, Some(0), [((1, 1), CellLayers::RESERVATION_ONLY)]);
        assert_eq!(grid.movement_epoch(), 0);
        assert_eq!(grid.fire_epoch(), 0);
        assert_eq!(grid.reservation_epoch(), 1);

        // Movement addition on clear cell increments movement_epoch
        grid.add_occupant(occ2, Some(0), [((1, 1), CellLayers::MOVEMENT_ONLY)]);
        assert_eq!(grid.movement_epoch(), 1);
        assert_eq!(grid.fire_epoch(), 0);
        assert_eq!(grid.reservation_epoch(), 1);

        // Second movement contributor on already-blocked cell does NOT increment movement_epoch
        let occ3 = OccupantId(3);
        grid.add_occupant(occ3, Some(0), [((1, 1), CellLayers::MOVEMENT_ONLY)]);
        assert_eq!(grid.movement_epoch(), 1);

        // Removing one of two movement contributors does NOT increment movement_epoch
        grid.remove_occupant(occ3);
        assert_eq!(grid.movement_epoch(), 1);

        // Removing the final movement contributor DOES increment movement_epoch
        grid.remove_occupant(occ2);
        assert_eq!(grid.movement_epoch(), 2);
    }
}
