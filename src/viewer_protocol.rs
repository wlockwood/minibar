//! Versioned, transport-neutral data contract shared by simulator and recorded viewers.
//!
//! This module deliberately has no dependency on [`crate::world::World`]. Live
//! simulation, simulator traces, and historical BAR adapters translate their
//! own state into these types. In particular, a historical adapter must never
//! initialize or advance a simulator world to fill a missing observation.

use serde::{Deserialize, Serialize};

pub const VIEWER_PROTOCOL_VERSION: u16 = 1;
pub const VIEWER_FRAME_SCHEMA: &str = "visigoth.viewer_frame";
pub const VIEWER_CONFIG_SCHEMA: &str = "visigoth.viewer_config";

/// The dependency-free frontend used by Rust's live server and by the Python
/// historical-review server. Python should serve the source file verbatim from
/// `simulator/viewer/viewer.html` so there is only one maintained viewer.
pub const SHARED_VIEWER_HTML: &str = include_str!("../viewer/viewer.html");

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewerMode {
    #[default]
    LiveSimulation,
    SimulatorTrace,
    HistoricalReview,
    Aggregate,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewerCapabilities {
    #[serde(default)]
    pub time_controls: bool,
    #[serde(default)]
    pub unit_commands: bool,
    #[serde(default)]
    pub playback: bool,
    #[serde(default)]
    pub review: bool,
    #[serde(default)]
    pub future_confirmation: bool,
}

/// Same-origin bootstrap document read by the shared asset. A historical
/// server supplies this from `/viewer-config.json`; the Rust live server may
/// omit it, in which case the frontend uses live `/ws` defaults.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewerConfig {
    pub schema: String,
    pub schema_version: u16,
    pub mode: ViewerMode,
    pub capabilities: ViewerCapabilities,
    pub frame_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction_endpoint: Option<String>,
    pub initial_frame_index: usize,
    pub frame_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_frame_index: Option<usize>,
}

impl ViewerConfig {
    pub fn historical(frame_endpoint: impl Into<String>, frame_count: usize) -> Self {
        Self {
            schema: VIEWER_CONFIG_SCHEMA.into(),
            schema_version: VIEWER_PROTOCOL_VERSION,
            mode: ViewerMode::HistoricalReview,
            capabilities: ViewerCapabilities {
                playback: true,
                review: true,
                future_confirmation: true,
                ..ViewerCapabilities::default()
            },
            frame_endpoint: frame_endpoint.into(),
            review_endpoint: None,
            prediction_endpoint: None,
            initial_frame_index: 0,
            frame_count,
            decision_frame_index: None,
        }
    }

    pub fn validate(&self) -> Result<(), ViewerProtocolError> {
        validate_schema(&self.schema, VIEWER_CONFIG_SCHEMA, self.schema_version)?;
        if !self.frame_endpoint.starts_with('/') || self.frame_endpoint.starts_with("//") {
            return Err(ViewerProtocolError::UnsafeFrameEndpoint);
        }
        if self
            .review_endpoint
            .as_ref()
            .is_some_and(|endpoint| !endpoint.starts_with('/') || endpoint.starts_with("//"))
        {
            return Err(ViewerProtocolError::UnsafeFrameEndpoint);
        }
        if self
            .prediction_endpoint
            .as_ref()
            .is_some_and(|endpoint| !endpoint.starts_with('/') || endpoint.starts_with("//"))
        {
            return Err(ViewerProtocolError::UnsafeFrameEndpoint);
        }
        if self.frame_count > 0 && self.initial_frame_index >= self.frame_count {
            return Err(ViewerProtocolError::FrameIndexOutOfRange);
        }
        if self
            .decision_frame_index
            .is_some_and(|index| index >= self.frame_count)
        {
            return Err(ViewerProtocolError::FrameIndexOutOfRange);
        }
        if matches!(
            self.mode,
            ViewerMode::HistoricalReview | ViewerMode::SimulatorTrace
        ) && (self.capabilities.time_controls || self.capabilities.unit_commands)
        {
            return Err(ViewerProtocolError::HistoricalMutationCapability);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewerProvenance {
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewerEvidence {
    Simulation,
    Recorded,
    Derived,
    #[default]
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewerAnnotation {
    pub label: String,
    pub evidence: ViewerEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerPoint {
    pub x: f32,
    pub z: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerSectorGrid {
    pub columns: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerTerrain {
    pub columns: u32,
    pub rows: u32,
    pub heights: Vec<f32>,
    #[serde(default)]
    pub water_depth: Vec<f32>,
    #[serde(default)]
    pub passable: Vec<bool>,
    pub evidence: ViewerEvidence,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerMap {
    pub name: String,
    pub width: f32,
    pub depth: f32,
    /// BAR/Recoil horizontal coordinates: X increases right, Z increases down.
    pub coordinate_system: String,
    pub sector_grid: ViewerSectorGrid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<ViewerTerrain>,
    pub evidence: ViewerEvidence,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewerRelation {
    Friendly,
    Enemy,
    Neutral,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewerVisibility {
    Own,
    LineOfSight,
    Radar,
    Remembered,
    #[default]
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewerRole {
    Commander,
    Builder,
    Raider,
    Skirmisher,
    Assault,
    Riot,
    Artillery,
    AntiAir,
    Factory,
    Extractor,
    Energy,
    Defense,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ViewerUnitMetadata {
    /// BAR/Recoil `iconType` captured with the historical catalog. This is
    /// presentation metadata only; it never changes simulation semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_type: Option<String>,
    /// A same-origin or data URI for the exact iconType bitmap. Historical
    /// adapters leave this absent when the version-matched package is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub role: ViewerRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tech_level: Option<u8>,
    #[serde(default)]
    pub builder: bool,
    #[serde(default)]
    pub factory: bool,
    #[serde(default)]
    pub building: bool,
    #[serde(default)]
    pub evidence: ViewerEvidence,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerTrail {
    pub points: Vec<ViewerPoint>,
    pub evidence: ViewerEvidence,
    #[serde(default)]
    pub ambiguous: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerCommand {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<ViewerPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_position: Option<ViewerPoint>,
    pub evidence: ViewerEvidence,
    #[serde(default)]
    pub issuer_ambiguous: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerEntity {
    /// Opaque and stable only within the source replay/run.
    pub id: String,
    pub relation: ViewerRelation,
    pub visibility: ViewerVisibility,
    pub position: ViewerPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default)]
    pub metadata: ViewerUnitMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_health: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_progress: Option<f32>,
    #[serde(default)]
    pub commands: Vec<ViewerCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail: Option<ViewerTrail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_coverage: Option<ViewerSensorCoverage>,
    #[serde(default)]
    pub annotations: Vec<ViewerAnnotation>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ViewerSensorCoverage {
    #[serde(default)]
    pub los: f32,
    #[serde(default)]
    pub air_los: f32,
    #[serde(default)]
    pub radar: f32,
    #[serde(default)]
    pub sonar: f32,
    #[serde(default)]
    pub jammer: f32,
    #[serde(default)]
    pub evidence: ViewerEvidence,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ViewerSensorSummary {
    pub evidence: ViewerEvidence,
    pub source_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub los_sources: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_sources: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jammer_sources: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_los: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_radar: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_jammer: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerResourceState {
    pub metal_stored: f64,
    pub metal_capacity: f64,
    pub metal_income: f64,
    pub metal_expense: f64,
    pub energy_stored: f64,
    pub energy_capacity: f64,
    pub energy_income: f64,
    pub energy_expense: f64,
    #[serde(default)]
    pub stalled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerTimedResourceState {
    pub game_time_seconds: f64,
    pub resources: ViewerResourceState,
}

/// The review decision being inspected, distinct from commands recorded in a
/// frame.  Historical adapters may only populate this from the prediction
/// under review and its detector evidence; it is not a reconstructed player
/// intention or a simulator result.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ViewerDecision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub evidence: Vec<ViewerAnnotation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerSite {
    pub kind: String,
    pub position: ViewerPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yield_per_second: Option<f32>,
    pub evidence: ViewerEvidence,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerEvent {
    pub kind: String,
    pub game_time_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<ViewerPoint>,
    pub evidence: ViewerEvidence,
}

/// Neutral frame sent by historical/trace adapters. Optional unavailable data
/// stays absent and is accompanied by an explicit annotation; adapters must
/// never fill it from a current catalog, future frame, or simulated world.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerFrame {
    #[serde(flatten)]
    pub header: ViewerFrameHeader,
    pub map: ViewerMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction_id: Option<String>,
    pub frame_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
    pub game_time_seconds: f64,
    #[serde(default)]
    pub decision_frame: bool,
    #[serde(default)]
    pub future_confirmation: bool,
    #[serde(default)]
    pub entities: Vec<ViewerEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ViewerResourceState>,
    #[serde(default)]
    pub resource_history: Vec<ViewerTimedResourceState>,
    /// The inferred goal and its explicitly-labelled detector evidence, when
    /// this frame is part of a review.  It remains absent for ordinary live
    /// simulation and tactical traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ViewerDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sector: Option<[u16; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected_target: Option<ViewerPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_position: Option<ViewerPoint>,
    #[serde(default)]
    pub resource_sites: Vec<ViewerSite>,
    #[serde(default)]
    pub detector_evidence: Vec<ViewerAnnotation>,
    #[serde(default)]
    pub events: Vec<ViewerEvent>,
    #[serde(default)]
    pub annotations: Vec<ViewerAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_coverage: Option<ViewerSensorSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerFrameHeader {
    pub schema: String,
    pub schema_version: u16,
    pub source_mode: ViewerMode,
    pub provenance: ViewerProvenance,
    pub capabilities: ViewerCapabilities,
}

impl ViewerFrameHeader {
    pub fn new(source_mode: ViewerMode, provenance: ViewerProvenance) -> Self {
        let capabilities = match source_mode {
            ViewerMode::LiveSimulation => ViewerCapabilities {
                time_controls: true,
                unit_commands: true,
                ..ViewerCapabilities::default()
            },
            ViewerMode::SimulatorTrace => ViewerCapabilities {
                playback: true,
                ..ViewerCapabilities::default()
            },
            ViewerMode::HistoricalReview => ViewerCapabilities {
                playback: true,
                review: true,
                future_confirmation: true,
                ..ViewerCapabilities::default()
            },
            ViewerMode::Aggregate => ViewerCapabilities::default(),
        };
        Self {
            schema: VIEWER_FRAME_SCHEMA.into(),
            schema_version: VIEWER_PROTOCOL_VERSION,
            source_mode,
            provenance,
            capabilities,
        }
    }
}

impl ViewerFrame {
    /// Converts one existing simulator trace sample without advancing any
    /// simulation state. The resulting frames are suitable for the same
    /// read-only endpoint used by historical playback.
    pub fn from_trace(
        trace: &crate::trace::RunTrace,
        frame_index: usize,
    ) -> Result<Self, ViewerProtocolError> {
        if trace.format_version != crate::trace::TRACE_FORMAT_VERSION {
            return Err(ViewerProtocolError::UnsupportedTraceVersion(
                trace.format_version,
            ));
        }
        let frame = trace
            .frames
            .get(frame_index)
            .ok_or(ViewerProtocolError::FrameIndexOutOfRange)?;
        let entities = frame
            .units
            .iter()
            .map(|unit| {
                let trail_points = trace.frames[..=frame_index]
                    .iter()
                    .filter_map(|candidate| {
                        candidate
                            .units
                            .iter()
                            .find(|other| other.id == unit.id)
                            .map(|other| ViewerPoint {
                                x: other.position[0],
                                z: other.position[1],
                            })
                    })
                    .collect();
                ViewerEntity {
                    id: unit.id.to_string(),
                    relation: if unit.side == 0 {
                        ViewerRelation::Friendly
                    } else {
                        ViewerRelation::Enemy
                    },
                    visibility: if unit.side == 0 {
                        ViewerVisibility::Own
                    } else {
                        ViewerVisibility::LineOfSight
                    },
                    position: ViewerPoint {
                        x: unit.position[0],
                        z: unit.position[1],
                    },
                    definition: Some(unit.definition.clone()),
                    metadata: ViewerUnitMetadata {
                        display_name: Some(unit.definition.clone()),
                        evidence: ViewerEvidence::Simulation,
                        ..ViewerUnitMetadata::default()
                    },
                    health: Some(unit.hp),
                    max_health: Some(unit.max_hp),
                    build_progress: None,
                    commands: Vec::new(),
                    trail: Some(ViewerTrail {
                        points: trail_points,
                        evidence: ViewerEvidence::Simulation,
                        ambiguous: false,
                    }),
                    sensor_coverage: None,
                    annotations: Vec::new(),
                }
            })
            .collect();
        let resource_sites = trace
            .map
            .resources
            .iter()
            .map(|site| ViewerSite {
                kind: match site.kind {
                    crate::trace::ResourceKind::MetalSpot => "metal_spot",
                    crate::trace::ResourceKind::MetalField => "metal_field",
                    crate::trace::ResourceKind::Geothermal => "geothermal",
                }
                .into(),
                position: ViewerPoint {
                    x: site.position[0],
                    z: site.position[1],
                },
                yield_per_second: Some(site.yield_per_second),
                evidence: ViewerEvidence::Simulation,
            })
            .collect();
        let map_columns =
            u32::try_from(trace.map.cols).map_err(|_| ViewerProtocolError::InvalidMap)?;
        let map_rows =
            u32::try_from(trace.map.rows).map_err(|_| ViewerProtocolError::InvalidMap)?;
        let result = Self {
            header: ViewerFrameHeader::new(
                ViewerMode::SimulatorTrace,
                ViewerProvenance {
                    source_id: format!("simulator-trace-seed-{}", trace.seed),
                    ..ViewerProvenance::default()
                },
            ),
            map: ViewerMap {
                name: "simulator trace".into(),
                width: trace.map.width,
                depth: trace.map.depth,
                coordinate_system: "x_z".into(),
                sector_grid: ViewerSectorGrid {
                    columns: 8,
                    rows: 8,
                },
                terrain: Some(ViewerTerrain {
                    columns: map_columns,
                    rows: map_rows,
                    heights: trace.map.heights.clone(),
                    water_depth: trace.map.water_depth.clone(),
                    passable: trace.map.passable.clone(),
                    evidence: ViewerEvidence::Simulation,
                }),
                evidence: ViewerEvidence::Simulation,
            },
            prediction_id: None,
            frame_index,
            frame_count: Some(trace.frames.len()),
            tick: Some(frame.tick),
            game_time_seconds: f64::from(frame.time),
            decision_frame: false,
            future_confirmation: false,
            entities,
            resources: None,
            resource_history: vec![],
            decision: None,
            target_sector: None,
            corrected_target: None,
            home_position: None,
            resource_sites,
            detector_evidence: vec![],
            events: vec![],
            annotations: vec![ViewerAnnotation {
                label: "economy unavailable in tactical trace".into(),
                evidence: ViewerEvidence::Unavailable,
                detail: None,
            }],
            sensor_coverage: None,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), ViewerProtocolError> {
        validate_schema(
            &self.header.schema,
            VIEWER_FRAME_SCHEMA,
            self.header.schema_version,
        )?;
        if !self.map.width.is_finite()
            || !self.map.depth.is_finite()
            || self.map.width <= 0.0
            || self.map.depth <= 0.0
            || self.map.sector_grid.columns == 0
            || self.map.sector_grid.rows == 0
        {
            return Err(ViewerProtocolError::InvalidMap);
        }
        if matches!(
            self.header.source_mode,
            ViewerMode::HistoricalReview | ViewerMode::SimulatorTrace
        ) && (self.header.capabilities.time_controls || self.header.capabilities.unit_commands)
        {
            return Err(ViewerProtocolError::HistoricalMutationCapability);
        }
        if self.entities.len() > 100_000 || self.resource_history.len() > 10_000 {
            return Err(ViewerProtocolError::PayloadTooLarge);
        }
        for entity in &self.entities {
            if entity.visibility == ViewerVisibility::Radar
                && (entity.definition.is_some()
                    || entity.metadata.display_name.is_some()
                    || entity.health.is_some()
                    || entity.max_health.is_some()
                    || entity.build_progress.is_some()
                    || entity.sensor_coverage.is_some())
            {
                return Err(ViewerProtocolError::RadarIdentityLeak(entity.id.clone()));
            }
        }
        Ok(())
    }
}

pub fn parse_viewer_frame(json: &str) -> Result<ViewerFrame, ViewerProtocolError> {
    let frame: ViewerFrame = serde_json::from_str(json).map_err(ViewerProtocolError::Json)?;
    frame.validate()?;
    Ok(frame)
}

fn validate_schema(
    actual: &str,
    expected: &'static str,
    version: u16,
) -> Result<(), ViewerProtocolError> {
    if actual != expected {
        return Err(ViewerProtocolError::WrongSchema {
            expected,
            actual: actual.into(),
        });
    }
    if version != VIEWER_PROTOCOL_VERSION {
        return Err(ViewerProtocolError::UnsupportedVersion(version));
    }
    Ok(())
}

#[derive(Debug)]
pub enum ViewerProtocolError {
    Json(serde_json::Error),
    WrongSchema {
        expected: &'static str,
        actual: String,
    },
    UnsupportedVersion(u16),
    UnsupportedTraceVersion(u16),
    UnsafeFrameEndpoint,
    FrameIndexOutOfRange,
    HistoricalMutationCapability,
    InvalidMap,
    PayloadTooLarge,
    RadarIdentityLeak(String),
}

impl std::fmt::Display for ViewerProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => error.fmt(formatter),
            Self::WrongSchema { expected, actual } => {
                write!(formatter, "expected schema {expected}, got {actual}")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported viewer protocol version {version}")
            }
            Self::UnsupportedTraceVersion(version) => {
                write!(formatter, "unsupported simulator trace version {version}")
            }
            Self::UnsafeFrameEndpoint => {
                formatter.write_str("frame endpoint must be a same-origin absolute path")
            }
            Self::FrameIndexOutOfRange => {
                formatter.write_str("initial frame index is out of range")
            }
            Self::HistoricalMutationCapability => {
                formatter.write_str("historical review cannot expose simulation mutation controls")
            }
            Self::InvalidMap => {
                formatter.write_str("viewer map dimensions or sector grid are invalid")
            }
            Self::PayloadTooLarge => {
                formatter.write_str("viewer frame exceeds bounded collection limits")
            }
            Self::RadarIdentityLeak(entity) => write!(
                formatter,
                "radar-only entity {entity} contains identity or health metadata"
            ),
        }
    }
}

impl std::error::Error for ViewerProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> ViewerFrame {
        ViewerFrame {
            header: ViewerFrameHeader::new(
                ViewerMode::HistoricalReview,
                ViewerProvenance {
                    source_id: "shard.parquet:player-0".into(),
                    ..ViewerProvenance::default()
                },
            ),
            map: ViewerMap {
                name: "historical map unavailable".into(),
                width: 1024.0,
                depth: 1024.0,
                coordinate_system: "x_z".into(),
                sector_grid: ViewerSectorGrid {
                    columns: 8,
                    rows: 8,
                },
                terrain: None,
                evidence: ViewerEvidence::Unavailable,
            },
            prediction_id: Some("prediction-1".into()),
            frame_index: 0,
            frame_count: Some(1),
            tick: None,
            game_time_seconds: 30.0,
            decision_frame: true,
            future_confirmation: false,
            entities: vec![],
            resources: None,
            resource_history: vec![],
            decision: None,
            target_sector: Some([3, 4]),
            corrected_target: None,
            home_position: None,
            resource_sites: vec![],
            detector_evidence: vec![],
            events: vec![],
            annotations: vec![],
            sensor_coverage: None,
        }
    }

    #[test]
    fn frame_schema_round_trips_and_validates() {
        let mut expected = frame();
        expected.sensor_coverage = Some(ViewerSensorSummary {
            evidence: ViewerEvidence::Derived,
            source_count: 2,
            detail: None,
            los_sources: Some(2),
            radar_sources: Some(1),
            jammer_sources: Some(1),
            max_los: Some(380.0),
            max_radar: Some(2100.0),
            max_jammer: Some(450.0),
        });
        expected.entities.push(ViewerEntity {
            id: "unit-1".into(),
            relation: ViewerRelation::Friendly,
            visibility: ViewerVisibility::Own,
            position: ViewerPoint { x: 10.0, z: 20.0 },
            definition: Some("armjam".into()),
            metadata: ViewerUnitMetadata::default(),
            health: Some(400.0),
            max_health: Some(400.0),
            build_progress: Some(1.0),
            commands: vec![],
            trail: None,
            sensor_coverage: Some(ViewerSensorCoverage {
                los: 280.0,
                air_los: 0.0,
                radar: 0.0,
                sonar: 0.0,
                jammer: 450.0,
                evidence: ViewerEvidence::Derived,
            }),
            annotations: vec![],
        });
        let json = serde_json::to_string(&expected).unwrap();
        assert_eq!(parse_viewer_frame(&json).unwrap(), expected);
    }

    #[test]
    fn simulator_trace_conversion_preserves_x_z_health_and_identity() {
        use crate::trace::{RunTrace, TraceFrame, TraceMap, TraceUnit};

        let trace = RunTrace {
            format_version: crate::trace::TRACE_FORMAT_VERSION,
            seed: 4,
            tick_hz: 30.0,
            map: TraceMap {
                width: 100.0,
                depth: 200.0,
                cols: 1,
                rows: 1,
                cell_size: 100.0,
                heights: vec![0.0],
                water_depth: vec![0.0],
                passable: vec![true],
                resources: vec![],
            },
            frames: vec![TraceFrame {
                tick: 30,
                time: 1.0,
                units: vec![TraceUnit {
                    id: 7,
                    side: 0,
                    definition: "armpw".into(),
                    icon_key: "armbot".into(),
                    position: [12.0, 34.0],
                    heading: 0.0,
                    hp: 50.0,
                    max_hp: 100.0,
                    radius: 8.0,
                    alive: true,
                    selected: false,
                }],
                aim_attempts: vec![],
            }],
            outcome: None,
        };
        let frame = ViewerFrame::from_trace(&trace, 0).unwrap();
        assert_eq!(frame.entities[0].position, ViewerPoint { x: 12.0, z: 34.0 });
        assert_eq!(frame.entities[0].health, Some(50.0));
        assert_eq!(frame.entities[0].id, "7");
        assert_eq!(frame.header.source_mode, ViewerMode::SimulatorTrace);
    }

    #[test]
    fn radar_contacts_cannot_carry_identity_or_health() {
        let mut frame = frame();
        frame.entities.push(ViewerEntity {
            id: "contact-1".into(),
            relation: ViewerRelation::Enemy,
            visibility: ViewerVisibility::Radar,
            position: ViewerPoint { x: 1.0, z: 2.0 },
            definition: Some("future_identity_leak".into()),
            metadata: ViewerUnitMetadata::default(),
            health: None,
            max_health: None,
            build_progress: None,
            commands: vec![],
            trail: None,
            sensor_coverage: None,
            annotations: vec![],
        });
        assert!(matches!(
            frame.validate(),
            Err(ViewerProtocolError::RadarIdentityLeak(_))
        ));
    }

    #[test]
    fn radar_contacts_cannot_carry_sensor_coverage() {
        let mut frame = frame();
        frame.entities.push(ViewerEntity {
            id: "contact-2".into(),
            relation: ViewerRelation::Enemy,
            visibility: ViewerVisibility::Radar,
            position: ViewerPoint { x: 1.0, z: 2.0 },
            definition: None,
            metadata: ViewerUnitMetadata::default(),
            health: None,
            max_health: None,
            build_progress: None,
            commands: vec![],
            trail: None,
            sensor_coverage: Some(ViewerSensorCoverage {
                los: 380.0,
                air_los: 0.0,
                radar: 0.0,
                sonar: 0.0,
                jammer: 0.0,
                evidence: ViewerEvidence::Derived,
            }),
            annotations: vec![],
        });
        assert!(matches!(
            frame.validate(),
            Err(ViewerProtocolError::RadarIdentityLeak(_))
        ));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut value = serde_json::to_value(frame()).unwrap();
        value["schema_version"] = serde_json::json!(99);
        let error = parse_viewer_frame(&value.to_string()).unwrap_err();
        assert!(matches!(error, ViewerProtocolError::UnsupportedVersion(99)));
    }

    #[test]
    fn historical_config_cannot_enable_simulation_actions() {
        let mut config = ViewerConfig::historical("/api/viewer/frame", 3);
        config.capabilities.unit_commands = true;
        assert!(matches!(
            config.validate(),
            Err(ViewerProtocolError::HistoricalMutationCapability)
        ));
    }

    #[test]
    fn radar_only_entity_cannot_carry_future_identity_or_health() {
        let mut value = frame();
        value.entities.push(ViewerEntity {
            id: "inferred-contact-1".into(),
            relation: ViewerRelation::Enemy,
            visibility: ViewerVisibility::Radar,
            position: ViewerPoint { x: 20.0, z: 40.0 },
            definition: Some("armcom".into()),
            metadata: ViewerUnitMetadata::default(),
            health: None,
            max_health: None,
            build_progress: None,
            commands: vec![],
            trail: None,
            sensor_coverage: None,
            annotations: vec![],
        });
        assert!(matches!(
            value.validate(),
            Err(ViewerProtocolError::RadarIdentityLeak(_))
        ));
    }

    #[test]
    fn shared_asset_has_live_and_historical_bootstrap_hooks() {
        assert!(SHARED_VIEWER_HTML.contains("VISIGOTH_SHARED_VIEWER"));
        assert!(SHARED_VIEWER_HTML.contains("/ws"));
    }
}
