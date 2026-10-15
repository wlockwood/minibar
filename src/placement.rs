//! Deterministic building placement, 16-elmo build-grid snapping,
//! footprint and yardmap rotation, and structured placement validation.

use crate::map::CompiledMap;
use crate::model::{UnitDef, Vec2};
use crate::occupancy::{CellLayers, OccupancyGrid, OccupancyLayerKind, OccupantId};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const BUILD_GRID_SIZE: f32 = 16.0;
pub const HALF_GRID_SIZE: f32 = 8.0;
const BOUNDS_EPSILON: f32 = 1e-4;

/// Compass build orientation matching Recoil engine's `FacingMap`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum BuildFacing {
    #[default]
    South = 0,
    East = 1,
    North = 2,
    West = 3,
}

impl BuildFacing {
    pub fn from_u8(value: u8) -> Self {
        match value % 4 {
            0 => Self::South,
            1 => Self::East,
            2 => Self::North,
            3 => Self::West,
            _ => unreachable!(),
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// World-space heading used by the tactical kernel: zero points east and
    /// angles increase toward south/+Z.
    pub const fn heading(self) -> f32 {
        match self {
            Self::South => std::f32::consts::FRAC_PI_2,
            Self::East => 0.0,
            Self::North => -std::f32::consts::FRAC_PI_2,
            Self::West => std::f32::consts::PI,
        }
    }

    /// Rotates a rectangular footprint `[width, depth]` in 16-elmo build blocks.
    #[inline]
    pub fn rotated_footprint(self, footprint: [u16; 2]) -> [u16; 2] {
        match self {
            Self::South | Self::North => footprint,
            Self::East | Self::West => [footprint[1], footprint[0]],
        }
    }

    /// Rotates a 16-elmo building yardmap to match this facing using Recoil's transformation matrix.
    pub fn rotate_yardmap(self, yardmap: &Yardmap) -> Yardmap {
        let xsize = yardmap.cols as usize;
        let zsize = yardmap.rows as usize;

        let (out_cols, out_rows) = match self {
            Self::South | Self::North => (yardmap.cols, yardmap.rows),
            Self::East | Self::West => (yardmap.rows, yardmap.cols),
        };

        let mut rotated_cells = vec![YardmapStatus::Blocked; out_cols as usize * out_rows as usize];

        for z in 0..zsize {
            for x in 0..xsize {
                let src_status = yardmap.cells[z * xsize + x];
                let (dest_x, dest_z) = match self {
                    Self::South => (x, z),
                    Self::North => (xsize - 1 - x, zsize - 1 - z),
                    Self::East => (z, xsize - 1 - x),
                    Self::West => (zsize - 1 - z, x),
                };
                let dest_idx = dest_z * out_cols as usize + dest_x;
                rotated_cells[dest_idx] = src_status;
            }
        }

        Yardmap {
            cols: out_cols,
            rows: out_rows,
            cells: rotated_cells,
        }
    }
}

/// Yardmap statuses corresponding to Recoil's building yardmap characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum YardmapStatus {
    /// Fully impassable building body ('o', 'x', 'w', 'f').
    Blocked,
    /// Open passable cell ('y').
    Open,
    /// Factory yard/pad where mobile units are constructed and exit ('c').
    Yard,
    /// Inverted yard ('i').
    YardInv,
    /// Geothermal vent location ('g').
    Geo,
    /// Stackable geothermal location ('j').
    GeoStackable,
    /// Stackable passable structure ('s').
    Stackable,
    /// Build only ('b').
    BuildOnly,
    /// Unbuildable ('u').
    Unbuildable,
    /// Factory exit corridor ('e').
    ExitOnly,
}

impl YardmapStatus {
    pub fn from_char(c: char) -> Self {
        match c.to_ascii_lowercase() {
            'c' => Self::Yard,
            'i' => Self::YardInv,
            'y' => Self::Open,
            'g' => Self::Geo,
            'j' => Self::GeoStackable,
            's' => Self::Stackable,
            'b' => Self::BuildOnly,
            'u' => Self::Unbuildable,
            'e' => Self::ExitOnly,
            _ => Self::Blocked,
        }
    }
}

/// A 2D grid of yardmap statuses for a structure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Yardmap {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<YardmapStatus>,
}

impl Yardmap {
    pub fn new(cols: u16, rows: u16, fill: YardmapStatus) -> Self {
        let total = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            cells: vec![fill; total],
        }
    }

    pub fn parse(s: &str, cols: u16, rows: u16) -> Result<Self, String> {
        let clean: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
        let expected = cols as usize * rows as usize;
        if clean.len() != expected {
            return Err(format!(
                "yardmap string has {} characters; expected {} ({cols}x{rows})",
                clean.len(),
                expected
            ));
        }
        let cells = clean.into_iter().map(YardmapStatus::from_char).collect();
        Ok(Self { cols, rows, cells })
    }

    #[inline]
    pub fn get(&self, col: u16, row: u16) -> Option<YardmapStatus> {
        if col < self.cols && row < self.rows {
            Some(self.cells[row as usize * self.cols as usize + col as usize])
        } else {
            None
        }
    }
}

/// Snaps a 1D world coordinate to the 16-elmo build grid based on footprint parity.
#[inline]
pub fn snap_build_coord(coord: f32, footprint_size: u16) -> f32 {
    if footprint_size % 2 == 1 {
        // Odd size: center is on half-cell (k * 16 + 8)
        (coord / BUILD_GRID_SIZE).floor() * BUILD_GRID_SIZE + HALF_GRID_SIZE
    } else {
        // Even size: center is on 16-elmo boundary (k * 16)
        ((coord + HALF_GRID_SIZE) / BUILD_GRID_SIZE).floor() * BUILD_GRID_SIZE
    }
}

/// Snaps a 2D world position to the deterministic 16-elmo build grid for the given facing and footprint.
pub fn snap_position(pos: Vec2, footprint: [u16; 2], facing: BuildFacing) -> Vec2 {
    let [rot_w, rot_d] = facing.rotated_footprint(footprint);
    Vec2::new(
        snap_build_coord(pos.x, rot_w),
        snap_build_coord(pos.z, rot_d),
    )
}

/// Builder position and reach context for range validation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuilderContext {
    pub position: Vec2,
    pub build_distance: f32,
}

/// Resource requirements for geothermal and metal extractors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub need_geo: bool,
    pub extracts_metal: bool,
    pub extract_range: f32,
    pub min_metal_yield: f32,
}

/// Terrain slope and water depth constraints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlacementConstraints {
    pub max_slope: Option<f32>,
    pub max_height_diff: Option<f32>,
    pub min_water_depth: Option<f32>,
    pub max_water_depth: Option<f32>,
}

/// High-level building placement request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacementRequest {
    pub occupant_id: OccupantId,
    pub owner: Option<u8>,
    pub unit_name: Option<String>,
    pub position: Vec2,
    pub facing: BuildFacing,
    pub footprint: [u16; 2],
    pub yardmap: Option<Yardmap>,
    pub movement_class: Option<String>,
    pub builder: Option<BuilderContext>,
    pub resources: ResourceRequirements,
    pub constraints: PlacementConstraints,
}

impl PlacementRequest {
    /// Builds a PlacementRequest from a catalog UnitDef with default constraints.
    pub fn from_unit_def(
        occupant_id: OccupantId,
        unit_def: &UnitDef,
        position: Vec2,
        facing: BuildFacing,
    ) -> Self {
        let build_constraints = &unit_def.strategic.build_constraints;
        Self {
            occupant_id,
            owner: None,
            unit_name: Some(unit_def.name.clone()),
            position,
            facing,
            footprint: unit_def.strategic.footprint,
            yardmap: build_constraints.yardmap.as_ref().and_then(|yardmap| {
                Yardmap::parse(
                    yardmap,
                    unit_def.strategic.footprint[0],
                    unit_def.strategic.footprint[1],
                )
                .ok()
            }),
            movement_class: unit_def.strategic.movement_class.clone(),
            builder: None,
            resources: ResourceRequirements {
                need_geo: unit_def.strategic.need_geo,
                extracts_metal: unit_def.strategic.extracts_metal > 0.0,
                extract_range: unit_def.strategic.extract_range,
                min_metal_yield: 0.001,
            },
            constraints: PlacementConstraints {
                max_slope: build_constraints.max_slope,
                max_height_diff: build_constraints.level_ground.then_some(0.0),
                min_water_depth: build_constraints.min_water_depth,
                max_water_depth: build_constraints.max_water_depth,
            },
        }
    }
}

/// Structured reasons why building placement was rejected.
#[derive(Clone, Debug, PartialEq)]
pub enum PlacementRejection {
    OutOfBounds {
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        map_width: f32,
        map_depth: f32,
    },
    OutOfBuilderRange {
        distance: f32,
        max_range: f32,
    },
    ImpassableTerrain {
        movement_class: String,
        cell: (u32, u32),
    },
    ExcessiveSlope {
        height_diff: f32,
        max_allowed: f32,
    },
    WaterConstraintViolated {
        current_depth: f32,
        min_water: Option<f32>,
        max_water: Option<f32>,
    },
    OccupancyConflict {
        layer: OccupancyLayerKind,
        existing_occupant: OccupantId,
        cell: (u32, u32),
    },
    MissingGeothermalVent,
    GeothermalVentAlreadyReserved {
        vent_index: usize,
        reserved_by: OccupantId,
    },
    InsufficientMetal {
        yield_value: f32,
        min_required: f32,
    },
    MetalSpotAlreadyReserved {
        spot_index: usize,
        reserved_by: OccupantId,
    },
    ExtractorRangeOverlap {
        existing_occupant: OccupantId,
    },
}

impl fmt::Display for PlacementRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds {
                min_x,
                min_z,
                max_x,
                max_z,
                map_width,
                map_depth,
            } => write!(
                f,
                "placement out of bounds: [{min_x:.1}, {min_z:.1} .. {max_x:.1}, {max_z:.1}] on map size {map_width:.1}x{map_depth:.1}"
            ),
            Self::OutOfBuilderRange {
                distance,
                max_range,
            } => write!(
                f,
                "out of builder range: distance {distance:.1} > max {max_range:.1}"
            ),
            Self::ImpassableTerrain {
                movement_class,
                cell,
            } => write!(
                f,
                "impassable terrain for {movement_class} at cell ({}, {})",
                cell.0, cell.1
            ),
            Self::ExcessiveSlope {
                height_diff,
                max_allowed,
            } => write!(
                f,
                "excessive slope: height diff {height_diff:.1} > max allowed {max_allowed:.1}"
            ),
            Self::WaterConstraintViolated {
                current_depth,
                min_water,
                max_water,
            } => write!(
                f,
                "water constraint violated: depth {current_depth:.1} (min: {min_water:?}, max: {max_water:?})"
            ),
            Self::OccupancyConflict {
                layer,
                existing_occupant,
                cell,
            } => write!(
                f,
                "occupancy conflict on {layer:?} by occupant {:?} at cell ({}, {})",
                existing_occupant, cell.0, cell.1
            ),
            Self::MissingGeothermalVent => write!(f, "no geothermal vent within tolerance"),
            Self::GeothermalVentAlreadyReserved {
                vent_index,
                reserved_by,
            } => write!(
                f,
                "geothermal vent #{vent_index} already reserved by occupant {reserved_by:?}"
            ),
            Self::InsufficientMetal {
                yield_value,
                min_required,
            } => write!(
                f,
                "insufficient metal: yield {yield_value:.3} < required {min_required:.3}"
            ),
            Self::MetalSpotAlreadyReserved {
                spot_index,
                reserved_by,
            } => write!(
                f,
                "metal spot #{spot_index} already reserved by occupant {reserved_by:?}"
            ),
            Self::ExtractorRangeOverlap { existing_occupant } => write!(
                f,
                "continuous metal extraction radius overlaps existing extractor {existing_occupant:?}"
            ),
        }
    }
}

impl std::error::Error for PlacementRejection {}

/// Validated placement resolution ready to be committed to the occupancy grid.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementResolution {
    pub occupant_id: OccupantId,
    pub owner: Option<u8>,
    pub snapped_position: Vec2,
    pub facing: BuildFacing,
    pub bounds: (f32, f32, f32, f32),
    pub cell_bounds: (u32, u32, u32, u32),
    pub cells: Vec<((u32, u32), CellLayers)>,
    pub geo_vent: Option<usize>,
    pub metal_spot: Option<usize>,
    pub continuous_extractor: Option<(Vec2, f32)>,
}

/// Validates a building placement request against map geometry and dynamic occupancy.
pub fn validate_placement(
    map: &CompiledMap,
    grid: &OccupancyGrid,
    request: &PlacementRequest,
    default_layers: CellLayers,
) -> Result<PlacementResolution, PlacementRejection> {
    // 1. Snapping to 16-elmo build grid
    let [rot_w, rot_d] = request.facing.rotated_footprint(request.footprint);
    let snapped = snap_position(request.position, request.footprint, request.facing);

    let half_w = rot_w as f32 * HALF_GRID_SIZE;
    let half_d = rot_d as f32 * HALF_GRID_SIZE;

    let min_x = snapped.x - half_w;
    let max_x = snapped.x + half_w;
    let min_z = snapped.z - half_d;
    let max_z = snapped.z + half_d;

    // 2. Map bounds check
    if min_x < -BOUNDS_EPSILON
        || min_z < -BOUNDS_EPSILON
        || max_x > map.width + BOUNDS_EPSILON
        || max_z > map.depth + BOUNDS_EPSILON
    {
        return Err(PlacementRejection::OutOfBounds {
            min_x,
            min_z,
            max_x,
            max_z,
            map_width: map.width,
            map_depth: map.depth,
        });
    }

    // 3. Builder range check (if builder context supplied)
    if let Some(builder) = request.builder {
        let dist = builder.position.distance(snapped);
        let footprint_reach = half_w.max(half_d);
        let max_reach = builder.build_distance + footprint_reach;
        if dist > max_reach + BOUNDS_EPSILON {
            return Err(PlacementRejection::OutOfBuilderRange {
                distance: dist,
                max_range: max_reach,
            });
        }
    }

    // 4. Compute exact cell boundaries
    let left = (min_x / grid.cell_size()).round().max(0.0) as u32;
    let top = (min_z / grid.cell_size()).round().max(0.0) as u32;
    let right = left + rot_w as u32 - 1;
    let bottom = top + rot_d as u32 - 1;

    // 5. Terrain passability (MoveDef check)
    if let Some(ref class) = request.movement_class {
        for r in top..=bottom {
            let sample_z = (r as f32 + 0.5) * grid.cell_size();
            for c in left..=right {
                let sample_x = (c as f32 + 0.5) * grid.cell_size();
                if !map.is_passable(class, sample_x, sample_z) {
                    return Err(PlacementRejection::ImpassableTerrain {
                        movement_class: class.clone(),
                        cell: (c, r),
                    });
                }
            }
        }
    }

    // 6. Ground height variation / slope check
    let mut min_height = f32::INFINITY;
    let mut max_height = f32::NEG_INFINITY;
    for r in top..=bottom {
        let sample_z = (r as f32 + 0.5) * grid.cell_size();
        for c in left..=right {
            let sample_x = (c as f32 + 0.5) * grid.cell_size();
            let h = map.ground_height(sample_x, sample_z);
            min_height = min_height.min(h);
            max_height = max_height.max(h);
        }
    }

    let height_diff = max_height - min_height;
    if let Some(max_diff) = request.constraints.max_height_diff
        && height_diff > max_diff + BOUNDS_EPSILON
    {
        return Err(PlacementRejection::ExcessiveSlope {
            height_diff,
            max_allowed: max_diff,
        });
    }
    if let Some(max_slope) = request.constraints.max_slope {
        let diag = ((rot_w as f32 * grid.cell_size()).powi(2)
            + (rot_d as f32 * grid.cell_size()).powi(2))
        .sqrt();
        let slope = if diag > 0.0 { height_diff / diag } else { 0.0 };
        if slope > max_slope + BOUNDS_EPSILON {
            return Err(PlacementRejection::ExcessiveSlope {
                height_diff,
                max_allowed: max_slope * diag,
            });
        }
    }

    // 7. Water depth constraints
    let center_water_depth = map.water_depth(snapped.x, snapped.z);
    if let Some(min_water) = request.constraints.min_water_depth
        && center_water_depth < min_water - BOUNDS_EPSILON
    {
        return Err(PlacementRejection::WaterConstraintViolated {
            current_depth: center_water_depth,
            min_water: Some(min_water),
            max_water: request.constraints.max_water_depth,
        });
    }
    if let Some(max_water) = request.constraints.max_water_depth
        && center_water_depth > max_water + BOUNDS_EPSILON
    {
        return Err(PlacementRejection::WaterConstraintViolated {
            current_depth: center_water_depth,
            min_water: request.constraints.min_water_depth,
            max_water: Some(max_water),
        });
    }

    // 8. Rotate yardmap if present
    let rotated_yardmap = request
        .yardmap
        .as_ref()
        .map(|ym| request.facing.rotate_yardmap(ym));

    // 9. Existing occupancy and reservations check
    let mut cells = Vec::with_capacity((rot_w * rot_d) as usize);
    for r in top..=bottom {
        let ym_r = (r - top) as u16;
        for c in left..=right {
            let ym_c = (c - left) as u16;
            let cell_layers = if let Some(ref ym) = rotated_yardmap {
                match ym.get(ym_c, ym_r).unwrap_or(YardmapStatus::Blocked) {
                    YardmapStatus::Yard | YardmapStatus::YardInv => CellLayers {
                        blocks_movement: false,
                        fire_height: None,
                        construction_reservation: false,
                        factory_yard: true,
                    },
                    YardmapStatus::Open => CellLayers::EMPTY,
                    YardmapStatus::ExitOnly => CellLayers {
                        blocks_movement: false,
                        fire_height: None,
                        construction_reservation: false,
                        factory_yard: true,
                    },
                    _ => default_layers,
                }
            } else {
                default_layers
            };

            // If the cell wants to block movement or reserve, verify it doesn't conflict
            if (cell_layers.blocks_movement || cell_layers.construction_reservation)
                && let Some(occ_cell) = grid.cell(c, r)
            {
                for contrib in &occ_cell.contributions {
                    if contrib.occupant_id != request.occupant_id {
                        if contrib.layers.blocks_movement {
                            return Err(PlacementRejection::OccupancyConflict {
                                layer: OccupancyLayerKind::Movement,
                                existing_occupant: contrib.occupant_id,
                                cell: (c, r),
                            });
                        }
                        if contrib.layers.construction_reservation {
                            return Err(PlacementRejection::OccupancyConflict {
                                layer: OccupancyLayerKind::ConstructionReservation,
                                existing_occupant: contrib.occupant_id,
                                cell: (c, r),
                            });
                        }
                    }
                }
            }

            // If it is a factory yard, reject if overlapping another factory yard or blocker
            if cell_layers.factory_yard
                && let Some(occ_cell) = grid.cell(c, r)
            {
                for contrib in &occ_cell.contributions {
                    if contrib.occupant_id != request.occupant_id
                        && (contrib.layers.factory_yard || contrib.layers.blocks_movement)
                    {
                        return Err(PlacementRejection::OccupancyConflict {
                            layer: OccupancyLayerKind::FactoryYard,
                            existing_occupant: contrib.occupant_id,
                            cell: (c, r),
                        });
                    }
                }
            }

            cells.push(((c, r), cell_layers));
        }
    }

    // 10. Geothermal validation & reservation
    let mut geo_vent = None;
    if request.resources.need_geo {
        let vent_idx = map.geothermal_at(snapped.x, snapped.z, HALF_GRID_SIZE);
        let Some(idx) = vent_idx else {
            return Err(PlacementRejection::MissingGeothermalVent);
        };
        if let Some(reserved_by) = grid.is_geothermal_reserved(idx)
            && reserved_by != request.occupant_id
        {
            return Err(PlacementRejection::GeothermalVentAlreadyReserved {
                vent_index: idx,
                reserved_by,
            });
        }
        geo_vent = Some(idx);
    }

    // 11. Metal validation & reservation
    let mut metal_spot = None;
    let mut continuous_extractor = None;
    if request.resources.extracts_metal {
        let site_yield = map.metal_yield_at(snapped.x, snapped.z, request.resources.extract_range);
        if site_yield < request.resources.min_metal_yield - BOUNDS_EPSILON {
            return Err(PlacementRejection::InsufficientMetal {
                yield_value: site_yield,
                min_required: request.resources.min_metal_yield,
            });
        }

        // Check if there is a discrete spot in range
        let discrete_spot = map.metal_nodes.iter().enumerate().find(|(_, node)| {
            let d_sq = (node.x - snapped.x).powi(2) + (node.z - snapped.z).powi(2);
            d_sq <= request.resources.extract_range * request.resources.extract_range
        });

        if let Some((spot_idx, _)) = discrete_spot {
            if let Some(reserved_by) = grid.is_metal_spot_reserved(spot_idx)
                && reserved_by != request.occupant_id
            {
                return Err(PlacementRejection::MetalSpotAlreadyReserved {
                    spot_index: spot_idx,
                    reserved_by,
                });
            }
            metal_spot = Some(spot_idx);
        } else {
            // Continuous metal extraction zone: check for overlap with other extractors
            if let Some(existing) = grid.is_extractor_range_overlapping(
                snapped,
                request.resources.extract_range,
                Some(request.occupant_id),
            ) {
                return Err(PlacementRejection::ExtractorRangeOverlap {
                    existing_occupant: existing,
                });
            }
            continuous_extractor = Some((snapped, request.resources.extract_range));
        }
    }

    Ok(PlacementResolution {
        occupant_id: request.occupant_id,
        owner: request.owner,
        snapped_position: snapped,
        facing: request.facing,
        bounds: (min_x, min_z, max_x, max_z),
        cell_bounds: (left, top, right, bottom),
        cells,
        geo_vent,
        metal_spot,
        continuous_extractor,
    })
}

/// Applies a validated placement resolution directly to the occupancy grid.
pub fn execute_placement(grid: &mut OccupancyGrid, resolution: &PlacementResolution) -> bool {
    let added = grid.add_occupant(
        resolution.occupant_id,
        resolution.owner,
        resolution.cells.clone(),
    );
    if !added {
        return false;
    }

    if let Some(vent_idx) = resolution.geo_vent {
        grid.reserve_geothermal(resolution.occupant_id, vent_idx);
    }
    if let Some(spot_idx) = resolution.metal_spot {
        grid.reserve_metal_spot(resolution.occupant_id, spot_idx);
    }
    if let Some((center, radius)) = resolution.continuous_extractor {
        grid.reserve_extractor_site(resolution.occupant_id, center, radius);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{HelloMapOptions, MapSource};

    fn test_map_source(width: f32, depth: f32) -> MapSource {
        let json = format!(
            r#"{{
            "map_name": "test_map",
            "map_size": [{width}, {depth}],
            "water_level": 0.0,
            "terrain": {{
                "heights": [[10.0, 10.0], [10.0, 10.0]],
                "movement_classes": {{
                    "default": [[true, true], [true, true]]
                }}
            }},
            "metal_spots": [
                {{"x": 32.0, "y": 10.0, "z": 32.0, "worth": 2.0}}
            ],
            "geothermal_spots": [
                {{"x": 48.0, "y": 10.0, "z": 48.0, "radius": 16.0}}
            ]
        }}"#
        );
        MapSource::from_hello_json(&json, HelloMapOptions::default()).unwrap()
    }

    #[test]
    fn rectangular_footprint_rotation_for_all_four_facings() {
        let footprint = [2, 4]; // width 2, depth 4
        assert_eq!(BuildFacing::South.rotated_footprint(footprint), [2, 4]);
        assert_eq!(BuildFacing::North.rotated_footprint(footprint), [2, 4]);
        assert_eq!(BuildFacing::East.rotated_footprint(footprint), [4, 2]);
        assert_eq!(BuildFacing::West.rotated_footprint(footprint), [4, 2]);

        // Yardmap rotation
        let ym_str = "cy\
                      oo"; // 2 cols, 2 rows: row 0: 'c', 'y'; row 1: 'o', 'o'
        let ym = Yardmap::parse(ym_str, 2, 2).unwrap();

        let ym_south = BuildFacing::South.rotate_yardmap(&ym);
        assert_eq!(ym_south.get(0, 0), Some(YardmapStatus::Yard));
        assert_eq!(ym_south.get(1, 0), Some(YardmapStatus::Open));
        assert_eq!(ym_south.get(0, 1), Some(YardmapStatus::Blocked));
        assert_eq!(ym_south.get(1, 1), Some(YardmapStatus::Blocked));

        let ym_north = BuildFacing::North.rotate_yardmap(&ym);
        assert_eq!(ym_north.get(1, 1), Some(YardmapStatus::Yard));
        assert_eq!(ym_north.get(0, 1), Some(YardmapStatus::Open));

        let ym_east = BuildFacing::East.rotate_yardmap(&ym);
        // (x=0, z=0) in south goes to (dest_x = z = 0, dest_z = 2 - 1 - 0 = 1)
        assert_eq!(ym_east.get(0, 1), Some(YardmapStatus::Yard));

        let ym_west = BuildFacing::West.rotate_yardmap(&ym);
        // (x=0, z=0) in south goes to (dest_x = 2 - 1 - 0 = 1, dest_z = x = 0)
        assert_eq!(ym_west.get(1, 0), Some(YardmapStatus::Yard));
    }

    #[test]
    fn build_grid_snapping_and_map_edge_rejection() {
        // Even footprint (2x2): snaps to multiples of 16
        assert_eq!(snap_build_coord(10.0, 2), 16.0);
        assert_eq!(snap_build_coord(24.0, 2), 32.0);
        assert_eq!(snap_build_coord(31.0, 2), 32.0);

        // Odd footprint (1x1): snaps to multiples of 16 + 8
        assert_eq!(snap_build_coord(10.0, 1), 8.0);
        assert_eq!(snap_build_coord(16.0, 1), 24.0);
        assert_eq!(snap_build_coord(24.0, 1), 24.0);

        let map = CompiledMap::compile(test_map_source(128.0, 128.0)).unwrap();
        let grid = OccupancyGrid::new(128.0, 128.0, 16.0).unwrap();

        // Valid placement at (16, 16) for 2x2
        let req_valid = PlacementRequest {
            occupant_id: OccupantId(1),
            owner: Some(0),
            unit_name: None,
            position: Vec2::new(16.0, 16.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements::default(),
            constraints: PlacementConstraints::default(),
        };
        assert!(validate_placement(&map, &grid, &req_valid, CellLayers::MOVEMENT_ONLY).is_ok());

        // Invalid placement hanging over left edge (min_x = 0 - 16 = -16)
        let mut req_edge = req_valid.clone();
        req_edge.position = Vec2::new(0.0, 16.0);
        // Snapped center is 0.0, half_w is 16.0 -> min_x is -16.0 -> rejected
        let res = validate_placement(&map, &grid, &req_edge, CellLayers::MOVEMENT_ONLY);
        assert!(matches!(res, Err(PlacementRejection::OutOfBounds { .. })));
    }

    #[test]
    fn impassable_terrain_excessive_slope_and_invalid_water_placement() {
        let json = r#"{
            "map_name": "slope_map",
            "map_size": [128.0, 128.0],
            "water_level": 5.0,
            "terrain": {
                "heights": [[0.0, 100.0], [0.0, 100.0]],
                "movement_classes": {
                    "land": [[false, true], [false, true]]
                }
            }
        }"#;
        let source = MapSource::from_hello_json(json, HelloMapOptions::default()).unwrap();
        let map = CompiledMap::compile(source).unwrap();
        let grid = OccupancyGrid::new(128.0, 128.0, 16.0).unwrap();

        // Impassable terrain check on "land"
        let req_impassable = PlacementRequest {
            occupant_id: OccupantId(1),
            owner: Some(0),
            unit_name: None,
            position: Vec2::new(16.0, 16.0), // left side is false
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: Some("land".into()),
            builder: None,
            resources: ResourceRequirements::default(),
            constraints: PlacementConstraints::default(),
        };
        let res1 = validate_placement(&map, &grid, &req_impassable, CellLayers::MOVEMENT_ONLY);
        assert!(matches!(
            res1,
            Err(PlacementRejection::ImpassableTerrain { .. })
        ));

        // Excessive slope check
        let req_slope = PlacementRequest {
            occupant_id: OccupantId(2),
            owner: Some(0),
            unit_name: None,
            position: Vec2::new(64.0, 64.0),
            facing: BuildFacing::South,
            footprint: [4, 4],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements::default(),
            constraints: PlacementConstraints {
                max_height_diff: Some(5.0),
                ..Default::default()
            },
        };
        let res2 = validate_placement(&map, &grid, &req_slope, CellLayers::MOVEMENT_ONLY);
        assert!(matches!(
            res2,
            Err(PlacementRejection::ExcessiveSlope { .. })
        ));

        // Water placement check (requires water depth == 0.0, but water_level is 5.0, height is 0.0 -> depth 5.0)
        let req_water = PlacementRequest {
            occupant_id: OccupantId(3),
            owner: Some(0),
            unit_name: None,
            position: Vec2::new(16.0, 16.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements::default(),
            constraints: PlacementConstraints {
                max_water_depth: Some(0.0),
                ..Default::default()
            },
        };
        let res3 = validate_placement(&map, &grid, &req_water, CellLayers::MOVEMENT_ONLY);
        assert!(matches!(
            res3,
            Err(PlacementRejection::WaterConstraintViolated { .. })
        ));
    }

    #[test]
    fn overlap_rejection() {
        let map = CompiledMap::compile(test_map_source(128.0, 128.0)).unwrap();
        let mut grid = OccupancyGrid::new(128.0, 128.0, 16.0).unwrap();

        let req1 = PlacementRequest {
            occupant_id: OccupantId(10),
            owner: Some(0),
            unit_name: None,
            position: Vec2::new(32.0, 32.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements::default(),
            constraints: PlacementConstraints::default(),
        };
        let res1 = validate_placement(&map, &grid, &req1, CellLayers::MOVEMENT_ONLY).unwrap();
        assert!(execute_placement(&mut grid, &res1));

        // Second building overlapping the same cells
        let req2 = PlacementRequest {
            occupant_id: OccupantId(11),
            owner: Some(0),
            unit_name: None,
            position: Vec2::new(32.0, 32.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements::default(),
            constraints: PlacementConstraints::default(),
        };
        let res2 = validate_placement(&map, &grid, &req2, CellLayers::MOVEMENT_ONLY);
        assert!(matches!(
            res2,
            Err(PlacementRejection::OccupancyConflict { .. })
        ));
    }

    #[test]
    fn geo_reservation_accepts_correct_vent_and_rejects_duplicate() {
        let map = CompiledMap::compile(test_map_source(128.0, 128.0)).unwrap();
        let mut grid = OccupancyGrid::new(128.0, 128.0, 16.0).unwrap();

        // Vent is at (48, 48) with radius 16.0
        let req1 = PlacementRequest {
            occupant_id: OccupantId(1),
            owner: Some(0),
            unit_name: Some("armgeo".into()),
            position: Vec2::new(48.0, 48.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements {
                need_geo: true,
                ..Default::default()
            },
            constraints: PlacementConstraints::default(),
        };

        let res1 = validate_placement(&map, &grid, &req1, CellLayers::building(20.0)).unwrap();
        assert_eq!(res1.geo_vent, Some(0));
        assert!(execute_placement(&mut grid, &res1));
        assert_eq!(grid.is_geothermal_reserved(0), Some(OccupantId(1)));

        // Duplicate placement at the same vent must be rejected
        let req2 = PlacementRequest {
            occupant_id: OccupantId(2),
            ..req1.clone()
        };
        let res2 = validate_placement(&map, &grid, &req2, CellLayers::building(20.0));
        assert!(matches!(
            res2,
            Err(PlacementRejection::OccupancyConflict { .. }
                | PlacementRejection::GeothermalVentAlreadyReserved { .. })
        ));

        // Release first geo plant
        grid.remove_occupant(OccupantId(1));
        assert_eq!(grid.is_geothermal_reserved(0), None);

        // Now second geo plant can place
        let res3 = validate_placement(&map, &grid, &req2, CellLayers::building(20.0)).unwrap();
        assert!(execute_placement(&mut grid, &res3));
        assert_eq!(grid.is_geothermal_reserved(0), Some(OccupantId(2)));
    }

    #[test]
    fn metal_placement_discrete_spots_and_continuous_fields() {
        // Map with discrete spot at (32, 32)
        let map = CompiledMap::compile(test_map_source(128.0, 128.0)).unwrap();
        let mut grid = OccupancyGrid::new(128.0, 128.0, 16.0).unwrap();

        // Extractor 1 placed at discrete spot (32, 32)
        let req_spot1 = PlacementRequest {
            occupant_id: OccupantId(1),
            owner: Some(0),
            unit_name: Some("armmex".into()),
            position: Vec2::new(32.0, 32.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements {
                extracts_metal: true,
                extract_range: 50.0,
                min_metal_yield: 0.1,
                ..Default::default()
            },
            constraints: PlacementConstraints::default(),
        };
        let res1 = validate_placement(&map, &grid, &req_spot1, CellLayers::building(10.0)).unwrap();
        assert_eq!(res1.metal_spot, Some(0));
        assert!(execute_placement(&mut grid, &res1));
        assert_eq!(grid.is_metal_spot_reserved(0), Some(OccupantId(1)));

        // Extractor placed at (96, 96) where there is no metal
        let mut req_no_metal = req_spot1.clone();
        req_no_metal.occupant_id = OccupantId(2);
        req_no_metal.position = Vec2::new(96.0, 96.0);
        let res_no_metal =
            validate_placement(&map, &grid, &req_no_metal, CellLayers::building(10.0));
        assert!(matches!(
            res_no_metal,
            Err(PlacementRejection::InsufficientMetal { .. })
        ));

        // Now test continuous metal field
        let json_continuous = r#"{
            "map_name": "continuous_metal_map",
            "map_size": [128.0, 128.0],
            "water_level": 0.0,
            "terrain": {
                "heights": [[0.0, 0.0], [0.0, 0.0]],
                "movement_classes": { "default": [[true, true], [true, true]] }
            },
            "metal_grid": [[1.0, 1.0], [1.0, 1.0]]
        }"#;
        let cont_source = MapSource::from_hello_json(
            json_continuous,
            HelloMapOptions {
                extractor_radius: 30.0,
                extraction_scale: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        let cont_map = CompiledMap::compile(cont_source).unwrap();
        let mut cont_grid = OccupancyGrid::new(128.0, 128.0, 16.0).unwrap();

        // Extractor on continuous metal field
        let req_cont1 = PlacementRequest {
            occupant_id: OccupantId(10),
            owner: Some(0),
            unit_name: Some("cormex".into()),
            position: Vec2::new(32.0, 32.0),
            facing: BuildFacing::South,
            footprint: [2, 2],
            yardmap: None,
            movement_class: None,
            builder: None,
            resources: ResourceRequirements {
                extracts_metal: true,
                extract_range: 20.0,
                min_metal_yield: 0.1,
                ..Default::default()
            },
            constraints: PlacementConstraints::default(),
        };
        let res_cont1 = validate_placement(
            &cont_map,
            &cont_grid,
            &req_cont1,
            CellLayers::building(10.0),
        )
        .unwrap();
        assert!(res_cont1.continuous_extractor.is_some());
        assert!(execute_placement(&mut cont_grid, &res_cont1));

        // Second extractor within overlapping radius (separation 32 < 40) but non-overlapping footprint
        let mut req_cont2 = req_cont1.clone();
        req_cont2.occupant_id = OccupantId(11);
        req_cont2.position = Vec2::new(64.0, 32.0); // separation is 32 < 40
        let res_cont2 = validate_placement(
            &cont_map,
            &cont_grid,
            &req_cont2,
            CellLayers::building(10.0),
        );
        assert!(matches!(
            res_cont2,
            Err(PlacementRejection::ExtractorRangeOverlap { .. })
        ));
    }
}
