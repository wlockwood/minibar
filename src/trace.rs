//! Optional observations for replay, visualization, and aggregate analysis.
//!
//! The simulator's fast path should hold this behind an `Option<TraceRecorder>`.
//! A disabled run therefore allocates and serializes nothing.

use crate::model::{UnitId, WorldView};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const TRACE_FORMAT_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceMap {
    pub width: f32,
    pub depth: f32,
    pub cols: usize,
    pub rows: usize,
    pub cell_size: f32,
    /// Absolute terrain height, in row-major order.
    pub heights: Vec<f32>,
    /// Optional water depth. Zero is dry; positive values are submerged.
    #[serde(default)]
    pub water_depth: Vec<f32>,
    /// General-purpose visualization mask. Movement-class-specific masks can
    /// be supplied as additional layers without changing trace frames.
    pub passable: Vec<bool>,
    #[serde(default)]
    pub resources: Vec<ResourceMarker>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResourceMarker {
    pub kind: ResourceKind,
    pub position: [f32; 2],
    /// Metal income for a discrete spot, or density for a metal-map sample.
    pub yield_per_second: f32,
    #[serde(default)]
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    MetalSpot,
    MetalField,
    Geothermal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceUnit {
    pub id: UnitId,
    pub side: u8,
    pub definition: String,
    /// Recoil/BAR icon type. Viewers fall back to a side-colored symbol.
    pub icon_key: String,
    pub position: [f32; 2],
    pub heading: f32,
    /// World-space yaw indexed like `AimAttempt::weapon`. Fixed guns follow
    /// the hull; null slots have no modeled barrel (shield/manual/interceptor).
    /// Absent in older traces; never infer an unknown turret yaw from the hull.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weapon_headings: Vec<Option<f32>>,
    pub hp: f32,
    pub max_hp: f32,
    pub radius: f32,
    /// Remaining reclaimable metal; zero for non-reclaimable objects and older traces.
    #[serde(default)]
    pub reclaim_metal: f32,
    pub alive: bool,
    #[serde(default)]
    pub selected: bool,
}

/// What a weapon was trying to do during this sampled interval. This is an
/// intent line, not a projectile, so predicted lead points remain visible even
/// when a weapon is waiting to turn, reload, or clear a friendly unit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AimAttempt {
    pub shooter: UnitId,
    pub weapon: u16,
    pub origin: [f32; 2],
    pub aim_point: [f32; 2],
    pub target: Option<UnitId>,
    pub state: AimState,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AimState {
    Tracking,
    Reloading,
    BlockedTerrain,
    BlockedFriendly,
    Fired,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceFrame {
    pub tick: u64,
    pub time: f32,
    pub units: Vec<TraceUnit>,
    #[serde(default)]
    pub aim_attempts: Vec<AimAttempt>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceOutcome {
    #[serde(default)]
    pub wreckage: crate::score::WreckageReport,
    pub winner: Option<u8>,
    pub duration: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RunTrace {
    pub format_version: u16,
    pub seed: u64,
    pub tick_hz: f32,
    pub map: TraceMap,
    pub frames: Vec<TraceFrame>,
    pub outcome: Option<TraceOutcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceConfig {
    /// One keeps every simulation tick; six records a 30 Hz run at 5 fps.
    pub sample_every_ticks: u64,
    /// Zero means unlimited. A cap retains the earliest qualifying frames.
    pub max_frames: usize,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            sample_every_ticks: 3,
            max_frames: 0,
        }
    }
}

#[derive(Debug)]
pub struct TraceRecorder {
    config: TraceConfig,
    trace: RunTrace,
}

impl TraceRecorder {
    pub fn new(config: TraceConfig, seed: u64, tick_hz: f32, map: TraceMap) -> Self {
        Self {
            config,
            trace: RunTrace {
                format_version: TRACE_FORMAT_VERSION,
                seed,
                tick_hz,
                map,
                frames: Vec::new(),
                outcome: None,
            },
        }
    }

    pub fn should_sample(&self, tick: u64) -> bool {
        let interval = self.config.sample_every_ticks.max(1);
        tick.is_multiple_of(interval)
            && (self.config.max_frames == 0 || self.trace.frames.len() < self.config.max_frames)
    }

    /// Captures public world state. Aim attempts are supplied by the weapon
    /// phase because they include decisions that cannot be inferred afterward.
    pub fn record_world(
        &mut self,
        tick: u64,
        world: &WorldView<'_>,
        aim_attempts: Vec<AimAttempt>,
    ) -> bool {
        if !self.should_sample(tick) {
            return false;
        }
        let units = world
            .units
            .iter()
            .map(|unit| TraceUnit {
                id: unit.id,
                side: unit.side,
                definition: unit.definition.name.clone(),
                icon_key: unit
                    .definition
                    .icon_type
                    .clone()
                    .unwrap_or_else(|| unit.definition.name.clone()),
                position: [unit.position.x, unit.position.z],
                heading: unit.heading,
                weapon_headings: unit
                    .definition
                    .weapons
                    .iter()
                    .zip(&unit.weapons)
                    .map(|(mount, state)| {
                        if mount.is_shield || mount.manual_fire || mount.interceptor != 0 {
                            None
                        } else if mount.turret {
                            Some(state.aim_heading)
                        } else {
                            Some(unit.heading)
                        }
                    })
                    .collect(),
                hp: unit.hp.max(0.0),
                max_hp: unit.definition.health,
                radius: unit.definition.movement.radius,
                reclaim_metal: if unit.alive && unit.definition.strategic.reclaim.reclaimable {
                    unit.definition.strategic.reclaim.metal.max(0.0)
                } else {
                    0.0
                },
                alive: unit.alive,
                selected: false,
            })
            .collect();
        self.trace.frames.push(TraceFrame {
            tick,
            time: world.time,
            units,
            aim_attempts,
        });
        true
    }

    pub fn record_frame(&mut self, frame: TraceFrame) -> bool {
        if !self.should_sample(frame.tick) {
            return false;
        }
        self.trace.frames.push(frame);
        true
    }

    pub fn finish(mut self, outcome: TraceOutcome) -> RunTrace {
        self.trace.outcome = Some(outcome);
        self.trace
    }

    pub fn into_unfinished(self) -> RunTrace {
        self.trace
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HeatLayer {
    pub side: Option<u8>,
    pub values: Vec<f32>,
    pub maximum: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AggregateTrace {
    pub format_version: u16,
    pub run_count: usize,
    pub map_width: f32,
    pub map_depth: f32,
    pub cols: usize,
    pub rows: usize,
    pub occupancy: Vec<HeatLayer>,
    pub aim_origins: HeatLayer,
    pub aim_points: HeatLayer,
    pub deaths: HeatLayer,
    pub wins_by_side: BTreeMap<u8, usize>,
    pub draws: usize,
}

impl AggregateTrace {
    /// Reduces many sampled runs into fixed-size heatmaps. Memory depends only
    /// on output grid size and side count, not on the number of runs.
    pub fn from_runs(runs: &[RunTrace], cols: usize, rows: usize) -> Result<Self, &'static str> {
        let first = runs.first().ok_or("at least one run is required")?;
        if cols == 0 || rows == 0 {
            return Err("aggregate dimensions must be non-zero");
        }
        let width = first.map.width;
        let depth = first.map.depth;
        let cells = cols
            .checked_mul(rows)
            .ok_or("aggregate dimensions overflow")?;
        let mut occupancy: BTreeMap<u8, Vec<f32>> = BTreeMap::new();
        let mut aim_origins = vec![0.0; cells];
        let mut aim_points = vec![0.0; cells];
        let mut deaths = vec![0.0; cells];
        let mut wins_by_side = BTreeMap::new();
        let mut draws = 0;

        for run in runs {
            if (run.map.width - width).abs() > f32::EPSILON
                || (run.map.depth - depth).abs() > f32::EPSILON
            {
                return Err("all runs must use the same map dimensions");
            }
            let mut previously_alive = BTreeMap::new();
            for frame in &run.frames {
                for unit in &frame.units {
                    if unit.alive {
                        let layer = occupancy
                            .entry(unit.side)
                            .or_insert_with(|| vec![0.0; cells]);
                        if let Some(index) = heat_index(unit.position, width, depth, cols, rows) {
                            layer[index] += 1.0;
                        }
                    }
                    if previously_alive.insert(unit.id, unit.alive) == Some(true)
                        && !unit.alive
                        && let Some(index) = heat_index(unit.position, width, depth, cols, rows)
                    {
                        deaths[index] += 1.0;
                    }
                }
                for aim in &frame.aim_attempts {
                    if let Some(index) = heat_index(aim.origin, width, depth, cols, rows) {
                        aim_origins[index] += 1.0;
                    }
                    if let Some(index) = heat_index(aim.aim_point, width, depth, cols, rows) {
                        aim_points[index] += 1.0;
                    }
                }
            }
            match run.outcome.as_ref().and_then(|outcome| outcome.winner) {
                Some(side) => *wins_by_side.entry(side).or_insert(0) += 1,
                None => draws += 1,
            }
        }

        Ok(Self {
            format_version: TRACE_FORMAT_VERSION,
            run_count: runs.len(),
            map_width: width,
            map_depth: depth,
            cols,
            rows,
            occupancy: occupancy
                .into_iter()
                .map(|(side, values)| heat_layer(Some(side), values))
                .collect(),
            aim_origins: heat_layer(None, aim_origins),
            aim_points: heat_layer(None, aim_points),
            deaths: heat_layer(None, deaths),
            wins_by_side,
            draws,
        })
    }
}

fn heat_index(
    position: [f32; 2],
    width: f32,
    depth: f32,
    cols: usize,
    rows: usize,
) -> Option<usize> {
    if width <= 0.0
        || depth <= 0.0
        || position[0] < 0.0
        || position[1] < 0.0
        || position[0] >= width
        || position[1] >= depth
    {
        return None;
    }
    let col = (position[0] / width * cols as f32) as usize;
    let row = (position[1] / depth * rows as f32) as usize;
    Some(row * cols + col)
}

fn heat_layer(side: Option<u8>, values: Vec<f32>) -> HeatLayer {
    let maximum = values.iter().copied().fold(0.0_f32, f32::max);
    HeatLayer {
        side,
        values,
        maximum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> TraceMap {
        TraceMap {
            width: 100.0,
            depth: 100.0,
            cols: 2,
            rows: 2,
            cell_size: 50.0,
            heights: vec![0.0; 4],
            water_depth: vec![0.0; 4],
            passable: vec![true; 4],
            resources: vec![],
        }
    }

    fn unit(id: UnitId, side: u8, position: [f32; 2], alive: bool) -> TraceUnit {
        TraceUnit {
            id,
            side,
            definition: "pawn".into(),
            icon_key: "bot".into(),
            position,
            heading: 0.0,
            weapon_headings: vec![],
            hp: if alive { 100.0 } else { 0.0 },
            max_hp: 100.0,
            radius: 5.0,
            reclaim_metal: 0.0,
            alive,
            selected: false,
        }
    }

    #[test]
    fn legacy_unit_and_outcome_default_wreck_data() {
        let mut json = serde_json::to_value(unit(1, 0, [1.0, 2.0], true)).unwrap();
        json.as_object_mut().unwrap().remove("reclaim_metal");
        let restored: TraceUnit = serde_json::from_value(json).unwrap();
        assert_eq!(restored.reclaim_metal, 0.0);
        let outcome: TraceOutcome = serde_json::from_str(r#"{"winner":0,"duration":1.0}"#).unwrap();
        assert_eq!(outcome.wreckage.total_metal, 0.0);
    }

    #[test]
    fn recorder_samples_without_allocating_extra_frames() {
        let mut recorder = TraceRecorder::new(
            TraceConfig {
                sample_every_ticks: 3,
                max_frames: 2,
            },
            7,
            30.0,
            map(),
        );
        for tick in 0..10 {
            recorder.record_frame(TraceFrame {
                tick,
                time: tick as f32 / 30.0,
                units: vec![],
                aim_attempts: vec![],
            });
        }
        let trace = recorder.into_unfinished();
        assert_eq!(trace.frames.len(), 2);
        assert_eq!(trace.frames[0].tick, 0);
        assert_eq!(trace.frames[1].tick, 3);
    }

    #[test]
    fn aggregate_tracks_occupancy_aim_deaths_and_wins() {
        let frames = vec![
            TraceFrame {
                tick: 0,
                time: 0.0,
                units: vec![unit(1, 0, [10.0, 10.0], true)],
                aim_attempts: vec![AimAttempt {
                    shooter: 1,
                    weapon: 0,
                    origin: [10.0, 10.0],
                    aim_point: [90.0, 90.0],
                    target: Some(2),
                    state: AimState::Tracking,
                }],
            },
            TraceFrame {
                tick: 1,
                time: 1.0 / 30.0,
                units: vec![unit(1, 0, [10.0, 10.0], false)],
                aim_attempts: vec![],
            },
        ];
        let run = RunTrace {
            format_version: TRACE_FORMAT_VERSION,
            seed: 1,
            tick_hz: 30.0,
            map: map(),
            frames,
            outcome: Some(TraceOutcome {
                wreckage: Default::default(),
                winner: Some(1),
                duration: 1.0,
            }),
        };
        let aggregate = AggregateTrace::from_runs(&[run], 2, 2).unwrap();
        assert_eq!(aggregate.occupancy[0].values[0], 1.0);
        assert_eq!(aggregate.aim_origins.values[0], 1.0);
        assert_eq!(aggregate.aim_points.values[3], 1.0);
        assert_eq!(aggregate.deaths.values[0], 1.0);
        assert_eq!(aggregate.wins_by_side[&1], 1);
    }
}
