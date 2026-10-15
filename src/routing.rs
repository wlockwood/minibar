//! Movement-class grid routing, unit radius clearance, path smoothing,
//! bounded search budgets, and obstacle-epoch cache invalidation.

use crate::map::CompiledMap;
use crate::model::Vec2;
use crate::occupancy::OccupancyGrid;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::f32::consts::SQRT_2;

pub const DEFAULT_MAX_SEARCH_NODES: usize = 2048;

/// Status of a calculated route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteStatus {
    /// Reached the exact requested destination.
    Complete,
    /// Could not reach the exact destination; returned the closest reachable point within budget.
    Partial,
    /// Destination is completely unreachable and no progress could be made.
    Unreachable,
    /// Start and destination are the same cell.
    AlreadyAtGoal,
}

/// A calculated route consisting of smoothed world-coordinate waypoints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub waypoints: Vec<Vec2>,
    pub path_length: f32,
    pub status: RouteStatus,
    pub nodes_expanded: usize,
    pub cached_epoch: u64,
}

impl Route {
    pub fn empty(status: RouteStatus, cached_epoch: u64) -> Self {
        Self {
            waypoints: Vec::new(),
            path_length: 0.0,
            status,
            nodes_expanded: 0,
            cached_epoch,
        }
    }
}

/// Configuration for a route calculation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteRequest {
    pub start: Vec2,
    pub goal: Vec2,
    pub movement_class: String,
    pub unit_radius: f32,
    pub max_search_nodes: usize,
    pub allow_partial: bool,
}

impl RouteRequest {
    pub fn new(start: Vec2, goal: Vec2, movement_class: impl Into<String>) -> Self {
        Self {
            start,
            goal,
            movement_class: movement_class.into(),
            unit_radius: 8.0,
            max_search_nodes: DEFAULT_MAX_SEARCH_NODES,
            allow_partial: true,
        }
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.unit_radius = radius.max(0.0);
        self
    }

    pub fn with_budget(mut self, budget: usize) -> Self {
        self.max_search_nodes = budget.max(1);
        self
    }

    pub fn with_allow_partial(mut self, allow: bool) -> Self {
        self.allow_partial = allow;
        self
    }
}

#[derive(Copy, Clone, PartialEq)]
struct AStarNode {
    cell: u32,
    f_score: f32,
    g_score: f32,
}

impl Eq for AStarNode {}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap
        other
            .f_score
            .partial_cmp(&self.f_score)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Cache key for storing routes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RouteCacheKey {
    movement_class: String,
    start_cell: (u32, u32),
    goal_cell: (u32, u32),
    quantized_radius: u16,
}

/// Fixed-capacity route cache invalidated by dynamic obstacle topology changes.
#[derive(Clone, Debug, Default)]
pub struct RouteCache {
    entries: HashMap<RouteCacheKey, (Route, u64)>,
    max_entries: usize,
}

impl RouteCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(max_entries),
            max_entries: max_entries.max(16),
        }
    }

    pub fn get(
        &self,
        request: &RouteRequest,
        start_cell: (u32, u32),
        goal_cell: (u32, u32),
        current_movement_epoch: u64,
    ) -> Option<Route> {
        let key = RouteCacheKey {
            movement_class: request.movement_class.clone(),
            start_cell,
            goal_cell,
            quantized_radius: (request.unit_radius.round() as u16).max(1),
        };

        if let Some((route, epoch)) = self.entries.get(&key)
            && *epoch == current_movement_epoch
        {
            return Some(route.clone());
        }
        None
    }

    pub fn insert(
        &mut self,
        request: &RouteRequest,
        start_cell: (u32, u32),
        goal_cell: (u32, u32),
        route: Route,
        current_movement_epoch: u64,
    ) {
        if self.entries.len() >= self.max_entries {
            self.entries.clear();
        }
        let key = RouteCacheKey {
            movement_class: request.movement_class.clone(),
            start_cell,
            goal_cell,
            quantized_radius: (request.unit_radius.round() as u16).max(1),
        };
        self.entries.insert(key, (route, current_movement_epoch));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Reusable grid router supporting A*, radius clearance, smoothing, and epoch-cached routing.
#[derive(Clone, Debug, Default)]
pub struct GridRouter {
    cache: RouteCache,
    // Reusable buffers to avoid per-search heap allocations
    g_scores: Vec<f32>,
    came_from: Vec<u32>,
    visited_epochs: Vec<u32>,
    current_search_epoch: u32,
}

impl GridRouter {
    pub fn new() -> Self {
        Self {
            cache: RouteCache::new(512),
            g_scores: Vec::new(),
            came_from: Vec::new(),
            visited_epochs: Vec::new(),
            current_search_epoch: 0,
        }
    }

    pub fn with_cache_capacity(capacity: usize) -> Self {
        Self {
            cache: RouteCache::new(capacity),
            g_scores: Vec::new(),
            came_from: Vec::new(),
            visited_epochs: Vec::new(),
            current_search_epoch: 0,
        }
    }

    /// Finds a route satisfying the request across `CompiledMap` passability and `OccupancyGrid`.
    pub fn find_route(
        &mut self,
        map: &CompiledMap,
        grid: &OccupancyGrid,
        request: &RouteRequest,
    ) -> Route {
        let Some(start_cell) = grid.coord_to_cell(request.start.x, request.start.z) else {
            return Route::empty(RouteStatus::Unreachable, grid.movement_epoch());
        };
        let Some(goal_cell) = grid.coord_to_cell(request.goal.x, request.goal.z) else {
            return Route::empty(RouteStatus::Unreachable, grid.movement_epoch());
        };

        if start_cell == goal_cell {
            return Route {
                waypoints: vec![request.goal],
                path_length: request.start.distance(request.goal),
                status: RouteStatus::AlreadyAtGoal,
                nodes_expanded: 0,
                cached_epoch: grid.movement_epoch(),
            };
        }

        // 1. Check RouteCache
        if let Some(cached) = self
            .cache
            .get(request, start_cell, goal_cell, grid.movement_epoch())
        {
            return cached;
        }

        // 2. Perform A* search
        let route = self.search_astar(map, grid, request, start_cell, goal_cell);

        // 3. Store in cache if complete or valid partial
        if route.status == RouteStatus::Complete
            || (route.status == RouteStatus::Partial && request.allow_partial)
        {
            self.cache.insert(
                request,
                start_cell,
                goal_cell,
                route.clone(),
                grid.movement_epoch(),
            );
        }

        route
    }

    fn ensure_buffers(&mut self, size: usize) {
        if self.g_scores.len() < size {
            self.g_scores.resize(size, f32::INFINITY);
            self.came_from.resize(size, u32::MAX);
            self.visited_epochs.resize(size, 0);
        }
        self.current_search_epoch = self.current_search_epoch.wrapping_add(1);
        if self.current_search_epoch == 0 {
            self.visited_epochs.fill(0);
            self.current_search_epoch = 1;
        }
    }

    fn is_cell_passable(
        map: &CompiledMap,
        grid: &OccupancyGrid,
        movement_class: &str,
        col: u32,
        row: u32,
        radius: f32,
    ) -> bool {
        if col >= grid.cols() || row >= grid.rows() {
            return false;
        }
        let cell_size = grid.cell_size();

        // Check center
        let center_x = (col as f32 + 0.5) * cell_size;
        let center_z = (row as f32 + 0.5) * cell_size;

        if !map.is_passable(movement_class, center_x, center_z) {
            return false;
        }
        if grid.is_movement_blocked_cell(col, row) {
            return false;
        }

        // Radius clearance check: if radius exceeds half-cell, test perimeter cells
        if radius > cell_size * 0.5 {
            let offset_cells = (radius / cell_size).ceil() as i32;
            let radius_sq = radius * radius;
            for dr in -offset_cells..=offset_cells {
                for dc in -offset_cells..=offset_cells {
                    if dc == 0 && dr == 0 {
                        continue;
                    }
                    let nc = col as i32 + dc;
                    let nr = row as i32 + dr;
                    if nc < 0 || nr < 0 || nc >= grid.cols() as i32 || nr >= grid.rows() as i32 {
                        return false;
                    }
                    let test_x = (nc as f32 + 0.5) * cell_size;
                    let test_z = (nr as f32 + 0.5) * cell_size;
                    let dist_sq = (test_x - center_x).powi(2) + (test_z - center_z).powi(2);
                    if dist_sq <= radius_sq {
                        if !map.is_passable(movement_class, test_x, test_z) {
                            return false;
                        }
                        if grid.is_movement_blocked_cell(nc as u32, nr as u32) {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    fn octile_distance(c1: u32, r1: u32, c2: u32, r2: u32, cell_size: f32) -> f32 {
        let dx = (c1 as f32 - c2 as f32).abs();
        let dz = (r1 as f32 - r2 as f32).abs();
        let min_d = dx.min(dz);
        let max_d = dx.max(dz);
        let h = (max_d - min_d) * cell_size + min_d * cell_size * SQRT_2;
        // Tie-breaker: slight bias towards straight lines
        h * 1.0001
    }

    fn search_astar(
        &mut self,
        map: &CompiledMap,
        grid: &OccupancyGrid,
        request: &RouteRequest,
        start_cell: (u32, u32),
        goal_cell: (u32, u32),
    ) -> Route {
        let cols = grid.cols();
        let rows = grid.rows();
        let total_cells = (cols * rows) as usize;
        let cell_size = grid.cell_size();

        self.ensure_buffers(total_cells);
        let search_epoch = self.current_search_epoch;

        let start_idx = (start_cell.1 * cols + start_cell.0) as usize;
        let goal_idx = (goal_cell.1 * cols + goal_cell.0) as usize;

        // Verify start is passable
        if !Self::is_cell_passable(
            map,
            grid,
            &request.movement_class,
            start_cell.0,
            start_cell.1,
            request.unit_radius,
        ) {
            return Route::empty(RouteStatus::Unreachable, grid.movement_epoch());
        }

        let mut open_set = BinaryHeap::new();

        self.g_scores[start_idx] = 0.0;
        self.visited_epochs[start_idx] = search_epoch;
        self.came_from[start_idx] = u32::MAX;

        let initial_h = Self::octile_distance(
            start_cell.0,
            start_cell.1,
            goal_cell.0,
            goal_cell.1,
            cell_size,
        );
        open_set.push(AStarNode {
            cell: start_idx as u32,
            f_score: initial_h,
            g_score: 0.0,
        });

        let mut nodes_expanded = 0;
        let mut best_cell = start_idx;
        let mut min_h_to_goal = initial_h;
        let mut reached_goal = false;

        // 8-directional deltas: (dc, dr, is_diagonal)
        let neighbors: [(i32, i32, bool); 8] = [
            (0, -1, false), // North
            (1, 0, false),  // East
            (0, 1, false),  // South
            (-1, 0, false), // West
            (1, -1, true),  // North-East
            (1, 1, true),   // South-East
            (-1, 1, true),  // South-West
            (-1, -1, true), // North-West
        ];

        while let Some(current) = open_set.pop() {
            let curr_idx = current.cell as usize;
            if curr_idx == goal_idx {
                best_cell = curr_idx;
                reached_goal = true;
                break;
            }

            if nodes_expanded >= request.max_search_nodes {
                break;
            }
            nodes_expanded += 1;

            let curr_c = (curr_idx % cols as usize) as u32;
            let curr_r = (curr_idx / cols as usize) as u32;

            for (dc, dr, is_diag) in neighbors {
                let next_c_i = curr_c as i32 + dc;
                let next_r_i = curr_r as i32 + dr;

                if next_c_i < 0
                    || next_r_i < 0
                    || next_c_i >= cols as i32
                    || next_r_i >= rows as i32
                {
                    continue;
                }

                let next_c = next_c_i as u32;
                let next_r = next_r_i as u32;

                // Diagonal move: prevent corner-cutting by verifying both orthogonal neighbors
                if is_diag
                    && (!Self::is_cell_passable(
                        map,
                        grid,
                        &request.movement_class,
                        curr_c,
                        next_r,
                        request.unit_radius,
                    ) || !Self::is_cell_passable(
                        map,
                        grid,
                        &request.movement_class,
                        next_c,
                        curr_r,
                        request.unit_radius,
                    ))
                {
                    continue;
                }

                if !Self::is_cell_passable(
                    map,
                    grid,
                    &request.movement_class,
                    next_c,
                    next_r,
                    request.unit_radius,
                ) {
                    continue;
                }

                let next_idx = (next_r * cols + next_c) as usize;
                let step_cost = if is_diag {
                    cell_size * SQRT_2
                } else {
                    cell_size
                };
                let tentative_g = current.g_score + step_cost;

                let is_visited = self.visited_epochs[next_idx] == search_epoch;
                if !is_visited || tentative_g < self.g_scores[next_idx] {
                    self.visited_epochs[next_idx] = search_epoch;
                    self.g_scores[next_idx] = tentative_g;
                    self.came_from[next_idx] = curr_idx as u32;

                    let h =
                        Self::octile_distance(next_c, next_r, goal_cell.0, goal_cell.1, cell_size);
                    if h < min_h_to_goal {
                        min_h_to_goal = h;
                        best_cell = next_idx;
                    }

                    open_set.push(AStarNode {
                        cell: next_idx as u32,
                        f_score: tentative_g + h,
                        g_score: tentative_g,
                    });
                }
            }
        }

        let status = if reached_goal {
            RouteStatus::Complete
        } else if request.allow_partial && best_cell != start_idx {
            RouteStatus::Partial
        } else {
            RouteStatus::Unreachable
        };

        if status == RouteStatus::Unreachable {
            return Route {
                waypoints: Vec::new(),
                path_length: 0.0,
                status,
                nodes_expanded,
                cached_epoch: grid.movement_epoch(),
            };
        }

        // Reconstruct raw grid path
        let mut raw_path = Vec::new();
        let mut trace_idx = best_cell;
        while trace_idx != usize::MAX && trace_idx != start_idx {
            let c = (trace_idx % cols as usize) as f32;
            let r = (trace_idx / cols as usize) as f32;
            raw_path.push(Vec2::new((c + 0.5) * cell_size, (r + 0.5) * cell_size));
            trace_idx = self.came_from[trace_idx] as usize;
        }

        if reached_goal {
            raw_path.insert(0, request.goal);
        }
        raw_path.push(request.start);
        raw_path.reverse();

        // 4. Smooth waypoints via line-of-sight shortcutting
        let smoothed = Self::smooth_path(
            map,
            grid,
            &request.movement_class,
            request.unit_radius,
            &raw_path,
        );

        // Compute total path length
        let mut path_length = 0.0;
        for i in 1..smoothed.len() {
            path_length += smoothed[i - 1].distance(smoothed[i]);
        }

        Route {
            waypoints: smoothed,
            path_length,
            status,
            nodes_expanded,
            cached_epoch: grid.movement_epoch(),
        }
    }

    /// Smooths a path by skipping intermediate waypoints if direct line-of-sight is unobstructed.
    fn smooth_path(
        map: &CompiledMap,
        grid: &OccupancyGrid,
        movement_class: &str,
        radius: f32,
        path: &[Vec2],
    ) -> Vec<Vec2> {
        if path.len() <= 2 {
            return path.to_vec();
        }

        let mut smoothed = Vec::with_capacity(path.len());
        smoothed.push(path[0]);

        let mut current_idx = 0;
        while current_idx < path.len() - 1 {
            let mut farthest = current_idx + 1;
            // Greedily find the farthest visible waypoint
            for next_idx in (current_idx + 2..path.len()).rev() {
                if Self::is_line_clear(
                    map,
                    grid,
                    movement_class,
                    radius,
                    path[current_idx],
                    path[next_idx],
                ) {
                    farthest = next_idx;
                    break;
                }
            }
            smoothed.push(path[farthest]);
            current_idx = farthest;
        }

        smoothed
    }

    /// Checks whether a straight line between two world positions is clear for a unit of `radius`.
    fn is_line_clear(
        map: &CompiledMap,
        grid: &OccupancyGrid,
        movement_class: &str,
        radius: f32,
        from: Vec2,
        to: Vec2,
    ) -> bool {
        let dist = from.distance(to);
        let step = (grid.cell_size() * 0.5).max(1.0);
        let samples = (dist / step).ceil() as usize;
        if samples <= 1 {
            return true;
        }

        for s in 1..samples {
            let t = s as f32 / samples as f32;
            let point = from + (to - from) * t;
            let Some((col, row)) = grid.coord_to_cell(point.x, point.z) else {
                return false;
            };
            if !Self::is_cell_passable(map, grid, movement_class, col, row, radius) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{HelloMapOptions, MapSource};
    use crate::occupancy::{CellLayers, OccupantId};

    fn test_map(width: f32, depth: f32) -> CompiledMap {
        let json = format!(
            r#"{{
            "map_name": "route_map",
            "map_size": [{width}, {depth}],
            "water_level": 0.0,
            "terrain": {{
                "heights": [[0.0, 0.0], [0.0, 0.0]],
                "movement_classes": {{
                    "land": [[true, true], [true, true]]
                }}
            }}
        }}"#
        );
        let source = MapSource::from_hello_json(&json, HelloMapOptions::default()).unwrap();
        CompiledMap::compile(source).unwrap()
    }

    #[test]
    fn clear_corridor_reaches_goal_and_smooths() {
        let map = test_map(160.0, 160.0);
        let grid = OccupancyGrid::new(160.0, 160.0, 16.0).unwrap();
        let mut router = GridRouter::new();

        let req = RouteRequest::new(Vec2::new(16.0, 16.0), Vec2::new(144.0, 144.0), "land");
        let route = router.find_route(&map, &grid, &req);

        assert_eq!(route.status, RouteStatus::Complete);
        assert!(route.path_length > 0.0);
        // Direct line of sight: smoothing turns it into a straight 2-point segment
        assert_eq!(route.waypoints.len(), 2);
        assert_eq!(route.waypoints[0], Vec2::new(16.0, 16.0));
        assert_eq!(route.waypoints[1], Vec2::new(144.0, 144.0));
    }

    #[test]
    fn wall_closes_route_and_gap_enables_it() {
        let map = test_map(160.0, 160.0);
        let mut grid = OccupancyGrid::new(160.0, 160.0, 16.0).unwrap();
        let mut router = GridRouter::new();

        // Continuous wall along column 4 (x from 64 to 80 across all rows 0..10)
        for r in 0..10 {
            grid.add_occupant(
                OccupantId(100 + r),
                Some(0),
                [((4, r as u32), CellLayers::MOVEMENT_ONLY)],
            );
        }

        let req = RouteRequest::new(Vec2::new(16.0, 80.0), Vec2::new(144.0, 80.0), "land")
            .with_allow_partial(false);

        let blocked_route = router.find_route(&map, &grid, &req);
        assert_eq!(blocked_route.status, RouteStatus::Unreachable);

        // Remove segment at row 5 (y around 80.0)
        grid.remove_occupant(OccupantId(105));

        let open_route = router.find_route(&map, &grid, &req);
        assert_eq!(open_route.status, RouteStatus::Complete);
        assert!(open_route.waypoints.len() >= 2);
    }

    #[test]
    fn radius_clearance_blocks_large_unit() {
        let map = test_map(160.0, 160.0);
        let mut grid = OccupancyGrid::new(160.0, 160.0, 16.0).unwrap();
        let mut router = GridRouter::new();

        // Wall with a single 16-elmo gap at (4, 5)
        for r in 0..10 {
            if r == 5 {
                continue;
            }
            grid.add_occupant(
                OccupantId(200 + r),
                Some(0),
                [((4, r as u32), CellLayers::MOVEMENT_ONLY)],
            );
        }

        // Small unit (radius 6.0) passes through 16-elmo gap
        let req_small = RouteRequest::new(Vec2::new(16.0, 88.0), Vec2::new(144.0, 88.0), "land")
            .with_radius(6.0)
            .with_allow_partial(false);
        let route_small = router.find_route(&map, &grid, &req_small);
        assert_eq!(route_small.status, RouteStatus::Complete);

        // Large unit (radius 24.0) cannot fit through 16-elmo gap
        let req_large = RouteRequest::new(Vec2::new(16.0, 88.0), Vec2::new(144.0, 88.0), "land")
            .with_radius(24.0)
            .with_allow_partial(false);
        let route_large = router.find_route(&map, &grid, &req_large);
        assert_eq!(route_large.status, RouteStatus::Unreachable);
    }

    #[test]
    fn route_cache_hits_and_obstacle_epoch_invalidation() {
        let map = test_map(160.0, 160.0);
        let mut grid = OccupancyGrid::new(160.0, 160.0, 16.0).unwrap();
        let mut router = GridRouter::new();

        let req = RouteRequest::new(Vec2::new(16.0, 16.0), Vec2::new(144.0, 16.0), "land");

        // First calculation expands nodes
        let route1 = router.find_route(&map, &grid, &req);
        assert_eq!(route1.status, RouteStatus::Complete);
        assert!(route1.nodes_expanded > 0);

        // Second calculation with unchanged epoch is served from cache
        let route2 = router.find_route(&map, &grid, &req);
        assert_eq!(route2.status, RouteStatus::Complete);
        assert_eq!(route2.nodes_expanded, route1.nodes_expanded);

        // Place a dynamic wall: updates movement_epoch
        grid.add_occupant(
            OccupantId(300),
            Some(0),
            [((4, 1), CellLayers::MOVEMENT_ONLY)],
        );

        // Route must now navigate around the new obstacle
        let route3 = router.find_route(&map, &grid, &req);
        assert_eq!(route3.status, RouteStatus::Complete);
        assert_eq!(route3.cached_epoch, grid.movement_epoch());
    }

    #[test]
    fn bounded_search_prevents_runaway_expansion() {
        let map = test_map(160.0, 160.0);
        let mut grid = OccupancyGrid::new(160.0, 160.0, 16.0).unwrap();
        let mut router = GridRouter::new();

        // Enclose goal in walls
        for c in 6..10 {
            for r in 6..10 {
                if c == 6 || c == 9 || r == 6 || r == 9 {
                    grid.add_occupant(
                        OccupantId(400 + c * 10 + r),
                        Some(0),
                        [((c as u32, r as u32), CellLayers::MOVEMENT_ONLY)],
                    );
                }
            }
        }

        // Bounded budget of 20 nodes
        let req = RouteRequest::new(Vec2::new(16.0, 16.0), Vec2::new(120.0, 120.0), "land")
            .with_budget(20)
            .with_allow_partial(true);

        let route = router.find_route(&map, &grid, &req);
        assert_eq!(route.status, RouteStatus::Partial);
        assert!(route.nodes_expanded <= 20);
    }
}
