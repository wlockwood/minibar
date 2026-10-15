//! Offline map compilation and allocation-light runtime queries.
//!
//! The acquisition side is deliberately independent of SMF parsing. A Recoil
//! widget, headless extraction match, or future archive reader can all produce
//! [`MapSource`]. Trials consume the compact [`CompiledMap`] and never call the
//! engine or parse map archives.

use crate::model::Terrain;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const MAP_SOURCE_VERSION: u16 = 1;
pub const MAP_CACHE_VERSION: u16 = 3;
const CACHE_MAGIC: [u8; 8] = *b"VSGMAP\0\0";

fn sibling_acquisition_path(input: &Path) -> Option<PathBuf> {
    let file_name = input.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".map_source.json")?;
    Some(input.with_file_name(format!("{stem}.acquisition.json")))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScalarGridSource {
    pub cols: u32,
    pub rows: u32,
    /// World X of the first sample. Height grids normally use 0.
    pub origin_x: f32,
    /// World Z of the first sample. Height grids normally use 0.
    pub origin_z: f32,
    /// Distance between samples, not the overall map size.
    pub step_x: f32,
    pub step_z: f32,
    /// Row-major, with X changing fastest: `values[z * cols + x]`.
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassabilitySource {
    /// Stable movement-class name, preferably the Recoil MoveDef name.
    pub movement_class: String,
    pub cols: u32,
    pub rows: u32,
    /// Cell extent; unlike height samples these values describe areas.
    pub cell_size_x: f32,
    pub cell_size_z: f32,
    /// Row-major. Acquisition should account for slope, depth and terrain type.
    pub passable: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceNode {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Metal income or other normalized value supplied by the extractor.
    pub yield_value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeothermalNode {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Placement tolerance around the vent center.
    pub radius: f32,
}

/// One captured map feature whose footprint affects dynamic occupancy.
///
/// Features are kept out of terrain masks because they may be crushed,
/// reclaimed, or destroyed independently in each trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticBlocker {
    /// Engine feature ID at capture time; provenance, not a runtime entity ID.
    pub feature_id: u32,
    pub definition: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Absolute world-space footprint bounds.
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
    pub blocks_movement: bool,
    pub blocks_fire: bool,
    /// Collision height above the feature base. Zero means unknown/no fire
    /// obstruction rather than an invented geometry estimate.
    pub fire_height: f32,
    pub crushable: bool,
    pub reclaimable: bool,
    pub reclaim_time: f32,
    pub metal: f32,
    pub energy: f32,
    pub geothermal: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContinuousMetalSource {
    /// Metal amount for each metal-map square as returned by Recoil.
    pub density: ScalarGridSource,
    /// Radius used by the game's extractor rule, in world units.
    pub extractor_radius: f32,
    /// Converts the sum of covered metal squares to simulator income.
    pub extraction_scale: f32,
}

/// Recoil/BAR identity needed to decide whether a compiled cache is reusable.
/// This is populated from a native capture's acquisition sidecar. Legacy hello
/// inputs do not carry these fields and therefore leave it absent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MapProvenance {
    pub format: String,
    pub format_version: u16,
    pub assembler_revision: u32,
    pub identity: Option<NativeMapIdentity>,
    #[serde(default)]
    pub map_options: BTreeMap<String, String>,
    #[serde(default)]
    pub mod_options: BTreeMap<String, String>,
    pub extractor_radius: Option<f32>,
    pub extraction_scale: Option<f32>,
    /// Full MoveDef records, including path type, slope/depth rules, footprint,
    /// crush strength, and the representative unit used by TestMoveOrder.
    #[serde(default)]
    pub movement_schema: Vec<BTreeMap<String, serde_json::Value>>,
}

impl MapProvenance {
    pub fn is_complete(&self) -> bool {
        self.format == "visigoth.native_map"
            && self.format_version > 0
            && self.identity.is_some()
            && self.extractor_radius.is_some()
            && self.extraction_scale.is_some()
            && !self.movement_schema.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeMapIdentity {
    pub map_name: String,
    pub map_checksum: String,
    pub game_name: String,
    pub game_version: String,
    pub mod_checksum: String,
    pub engine_version: String,
    pub exporter_revision: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapInputFormat {
    NativeSource,
    Hello,
}

/// The on-disk representation accepted by [`CompiledMap::load_path`].
///
/// A source JSON is compiled in memory. This keeps the friendly authoring
/// path available to live MiniBar sessions, while long-running batches can
/// still use a precompiled cache and pay no parse/compile cost per trial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapLoadFormat {
    CompiledCache,
    NativeSource,
    Hello,
}

/// Options for loading a map file supplied to MiniBar.
///
/// Native map sources can have an adjacent acquisition sidecar. Loading that
/// sidecar is enabled by default so a scenario started from JSON retains map,
/// engine, and movement-class provenance just like an explicitly compiled
/// cache.
#[derive(Clone, Debug, PartialEq)]
pub struct MapLoadOptions {
    pub hello: HelloMapOptions,
    pub load_sibling_acquisition: bool,
}

impl Default for MapLoadOptions {
    fn default() -> Self {
        Self {
            hello: HelloMapOptions::default(),
            load_sibling_acquisition: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSource {
    pub source_version: u16,
    pub map_name: String,
    /// Content identity (archive checksum/hash plus relevant mod options).
    pub content_hash: String,
    pub width: f32,
    pub depth: f32,
    /// Retained explicitly: zero is common but not guaranteed forever.
    pub water_level: f32,
    pub heights: ScalarGridSource,
    /// One exact mask per movement class needed by the simulation catalog.
    pub passability: Vec<PassabilitySource>,
    /// Normal maps generally use these. They may coexist with a metal field.
    #[serde(default)]
    pub metal_nodes: Vec<ResourceNode>,
    /// Metal maps stay continuous rather than becoming arbitrary fake spots.
    pub continuous_metal: Option<ContinuousMetalSource>,
    #[serde(default)]
    pub geothermal_nodes: Vec<GeothermalNode>,
    /// Initial dynamic feature occupancy, separate from immutable terrain.
    #[serde(default)]
    pub static_blockers: Vec<StaticBlocker>,
    /// Minimum wind speed captured for the map.
    #[serde(default)]
    pub wind_min: Option<f32>,
    /// Maximum wind speed captured for the map.
    #[serde(default)]
    pub wind_max: Option<f32>,
    /// Tidal energy output captured for the map.
    #[serde(default)]
    pub tidal: Option<f32>,
    /// Optional acquisition identity. The standard assembler currently writes
    /// this to the sibling `.acquisition.json`; the CLI merges it before
    /// compilation. It may also be embedded directly by future producers.
    #[serde(default)]
    pub provenance: MapProvenance,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapError {
    UnsupportedSourceVersion(u16),
    UnsupportedCacheVersion(u16),
    Invalid(&'static str),
    Json(String),
    Io(String),
    MissingMovementClasses,
    UnknownMovementClass(String),
    DuplicateMovementClass(String),
    UnsupportedJsonFormat(String),
    Acquisition(String),
    CorruptCache(&'static str),
    TrailingCacheData,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSourceVersion(v) => write!(f, "unsupported map source version {v}"),
            Self::UnsupportedCacheVersion(v) => write!(f, "unsupported map cache version {v}"),
            Self::Invalid(message) => write!(f, "invalid map source: {message}"),
            Self::Json(message) => write!(f, "invalid map JSON: {message}"),
            Self::Io(message) => write!(f, "unable to read map: {message}"),
            Self::MissingMovementClasses => write!(
                f,
                "hello terrain has only the legacy land mask; supply named movement_classes or explicitly opt into a legacy class"
            ),
            Self::UnknownMovementClass(name) => write!(f, "unknown movement class {name:?}"),
            Self::DuplicateMovementClass(name) => {
                write!(f, "duplicate movement class {name:?}")
            }
            Self::UnsupportedJsonFormat(message) => {
                write!(f, "unsupported map JSON format: {message}")
            }
            Self::Acquisition(message) => write!(f, "invalid map acquisition metadata: {message}"),
            Self::CorruptCache(message) => write!(f, "corrupt map cache: {message}"),
            Self::TrailingCacheData => write!(f, "map cache has trailing data"),
        }
    }
}

impl std::error::Error for MapError {}

/// Controls conversion of the bridge's existing `hello` JSON into `MapSource`.
/// New acquisition should export named movement classes and the water level.
#[derive(Clone, Debug, PartialEq)]
pub struct HelloMapOptions {
    pub content_hash: String,
    pub fallback_water_level: f32,
    pub extractor_radius: f32,
    pub extraction_scale: f32,
    /// Explicit compatibility escape hatch for old 32x32 `height >= 0` masks.
    /// It is intentionally `None` by default because that mask is not valid for
    /// ships, hovers, amphibious units, or slope-constrained MoveDefs.
    pub accept_legacy_mask_as: Option<String>,
    /// Used only if a geothermal entry has no radius.
    pub default_geothermal_radius: f32,
}

impl Default for HelloMapOptions {
    fn default() -> Self {
        Self {
            content_hash: String::new(),
            fallback_water_level: 0.0,
            extractor_radius: 500.0,
            extraction_scale: 1.0,
            accept_legacy_mask_as: None,
            // One normal 16-elmo building square. Prefer the feature collision
            // radius or an acquisition-time TestBuildOrder-derived value.
            default_geothermal_radius: 16.0,
        }
    }
}

#[derive(Deserialize)]
struct HelloMap {
    map_name: String,
    map_size: [f32; 2],
    water_level: Option<f32>,
    #[serde(default)]
    wind_min: Option<f32>,
    #[serde(default)]
    wind_max: Option<f32>,
    #[serde(default)]
    tidal: Option<f32>,
    terrain: HelloTerrain,
    #[serde(default)]
    metal_spots: Vec<HelloNode>,
    metal_grid: Option<Vec<Vec<f32>>>,
    #[serde(default, alias = "geothermals")]
    geothermal_spots: Vec<HelloGeo>,
}

#[derive(Deserialize)]
struct HelloTerrain {
    heights: Vec<Vec<f32>>,
    #[serde(default)]
    passability: Vec<Vec<MaskValue>>,
    #[serde(default)]
    movement_classes: BTreeMap<String, Vec<Vec<MaskValue>>>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
enum MaskValue {
    Boolean(bool),
    Number(u8),
}

impl MaskValue {
    fn get(self) -> bool {
        match self {
            Self::Boolean(value) => value,
            Self::Number(value) => value != 0,
        }
    }
}

#[derive(Deserialize)]
struct HelloNode {
    x: f32,
    #[serde(default)]
    y: f32,
    z: f32,
    #[serde(default, alias = "yield_value")]
    worth: f32,
}

#[derive(Deserialize)]
struct HelloGeo {
    x: f32,
    #[serde(default)]
    y: f32,
    z: f32,
    radius: Option<f32>,
}

#[derive(Deserialize)]
struct AcquisitionSidecar {
    format: String,
    format_version: u16,
    #[serde(default)]
    assembler_revision: u32,
    content_hash: String,
    identity: NativeMapIdentity,
    map: AcquisitionMap,
    #[serde(default)]
    options: AcquisitionOptions,
    #[serde(default)]
    movedefs: Vec<BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    static_blockers: Vec<StaticBlocker>,
}

#[derive(Deserialize)]
struct AcquisitionMap {
    extractor_radius: f32,
    #[serde(default)]
    wind_min: Option<f32>,
    #[serde(default)]
    wind_max: Option<f32>,
    #[serde(default)]
    tidal: Option<f32>,
}

#[derive(Default, Deserialize)]
struct AcquisitionOptions {
    #[serde(default)]
    map: BTreeMap<String, String>,
    #[serde(default)]
    mod_options: BTreeMap<String, String>,
    #[serde(default, rename = "mod")]
    mod_alias: BTreeMap<String, String>,
}

impl MapSource {
    /// Detects and imports either canonical native `MapSource` JSON or a
    /// bridge hello. Raw capture records and acquisition sidecars are rejected
    /// with an actionable format error rather than being misparsed as hello.
    pub fn from_json(
        json: &str,
        hello_options: HelloMapOptions,
    ) -> Result<(Self, MapInputFormat), MapError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| MapError::Json(error.to_string()))?;
        let object = value
            .as_object()
            .ok_or_else(|| MapError::UnsupportedJsonFormat("top level must be an object".into()))?;
        if object.contains_key("source_version") {
            let source: Self = serde_json::from_value(value)
                .map_err(|error| MapError::Json(format!("invalid canonical MapSource: {error}")))?;
            validate_source(&source)?;
            return Ok((source, MapInputFormat::NativeSource));
        }
        if object.contains_key("map_size") && object.contains_key("terrain") {
            return Self::from_hello_json(json, hello_options)
                .map(|source| (source, MapInputFormat::Hello));
        }
        if object.get("format").and_then(serde_json::Value::as_str) == Some("visigoth.native_map") {
            return Err(MapError::UnsupportedJsonFormat(
                "this is a native capture record or acquisition sidecar; pass the sibling .map_source.json"
                    .into(),
            ));
        }
        Err(MapError::UnsupportedJsonFormat(
            "expected canonical MapSource fields (`source_version`, `heights`) or bridge hello fields (`map_size`, `terrain`)"
                .into(),
        ))
    }

    /// Adds full capture provenance from the assembler's sibling acquisition
    /// artifact. Identity, content hash, extractor parameters, and MoveDef
    /// names must agree with the canonical map source.
    pub fn apply_acquisition_json(&mut self, json: &str) -> Result<(), MapError> {
        let acquisition: AcquisitionSidecar =
            serde_json::from_str(json).map_err(|error| MapError::Acquisition(error.to_string()))?;
        if acquisition.format != "visigoth.native_map" || acquisition.format_version == 0 {
            return Err(MapError::Acquisition(
                "unsupported format or format_version".into(),
            ));
        }
        if acquisition.identity.map_name != self.map_name {
            return Err(MapError::Acquisition(format!(
                "map name mismatch: source {:?}, acquisition {:?}",
                self.map_name, acquisition.identity.map_name
            )));
        }
        if acquisition.content_hash != self.content_hash {
            return Err(MapError::Acquisition(format!(
                "content hash mismatch: source {:?}, acquisition {:?}",
                self.content_hash, acquisition.content_hash
            )));
        }
        if !finite_positive(acquisition.map.extractor_radius) {
            return Err(MapError::Acquisition(
                "extractor_radius must be finite and positive".into(),
            ));
        }
        let extraction_scale = self
            .continuous_metal
            .as_ref()
            .map(|metal| metal.extraction_scale);
        if let Some(metal) = &self.continuous_metal
            && (metal.extractor_radius - acquisition.map.extractor_radius).abs() > 1e-4
        {
            return Err(MapError::Acquisition(format!(
                "extractor radius mismatch: source {}, acquisition {}",
                metal.extractor_radius, acquisition.map.extractor_radius
            )));
        }
        if self.wind_min.is_none() {
            self.wind_min = acquisition.map.wind_min;
        }
        if self.wind_max.is_none() {
            self.wind_max = acquisition.map.wind_max;
        }
        if self.tidal.is_none() {
            self.tidal = acquisition.map.tidal;
        }
        let mut movement_names = acquisition
            .movedefs
            .iter()
            .map(|record| {
                record
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| MapError::Acquisition("MoveDef is missing a name".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        movement_names.sort();
        movement_names.dedup();
        let mut passability_names = self
            .passability
            .iter()
            .map(|layer| layer.movement_class.clone())
            .collect::<Vec<_>>();
        passability_names.sort();
        passability_names.dedup();
        if movement_names != passability_names {
            return Err(MapError::Acquisition(format!(
                "MoveDef names {movement_names:?} do not match passability layers {passability_names:?}"
            )));
        }
        let mut mod_options = acquisition.options.mod_options;
        mod_options.extend(acquisition.options.mod_alias);
        self.provenance = MapProvenance {
            format: acquisition.format,
            format_version: acquisition.format_version,
            assembler_revision: acquisition.assembler_revision,
            identity: Some(acquisition.identity),
            map_options: acquisition.options.map,
            mod_options,
            extractor_radius: Some(acquisition.map.extractor_radius),
            extraction_scale,
            movement_schema: acquisition.movedefs,
        };
        if !acquisition.static_blockers.is_empty() {
            validate_static_blockers(&acquisition.static_blockers, self.width, self.depth)
                .map_err(|_| MapError::Acquisition("invalid static blocker metadata".into()))?;
            if !self.static_blockers.is_empty()
                && self.static_blockers != acquisition.static_blockers
            {
                return Err(MapError::Acquisition(
                    "static blockers do not match canonical map source".into(),
                ));
            }
            self.static_blockers = acquisition.static_blockers;
        }
        Ok(())
    }

    /// Adapts the bridge's `hello` payload. Nested bridge grids are X-major;
    /// this transposes them into the cache's conventional row-major layout.
    pub fn from_hello_json(json: &str, options: HelloMapOptions) -> Result<Self, MapError> {
        let hello: HelloMap =
            serde_json::from_str(json).map_err(|error| MapError::Json(error.to_string()))?;
        if !finite_positive(hello.map_size[0]) || !finite_positive(hello.map_size[1]) {
            return Err(MapError::Invalid("invalid hello map_size"));
        }
        let (height_cols, height_rows, height_values) = transpose_scalar(&hello.terrain.heights)?;
        let height_step_x = hello.map_size[0] / height_cols as f32;
        let height_step_z = hello.map_size[1] / height_rows as f32;
        let mut passability = Vec::new();
        for (name, grid) in hello.terrain.movement_classes {
            let (cols, rows, values) = transpose_mask(&grid)?;
            passability.push(PassabilitySource {
                movement_class: name,
                cols,
                rows,
                cell_size_x: hello.map_size[0] / cols as f32,
                cell_size_z: hello.map_size[1] / rows as f32,
                passable: values,
            });
        }
        if passability.is_empty() {
            let Some(name) = options.accept_legacy_mask_as else {
                return Err(MapError::MissingMovementClasses);
            };
            let (cols, rows, values) = transpose_mask(&hello.terrain.passability)?;
            passability.push(PassabilitySource {
                movement_class: name,
                cols,
                rows,
                cell_size_x: hello.map_size[0] / cols as f32,
                cell_size_z: hello.map_size[1] / rows as f32,
                passable: values,
            });
        }
        let continuous_metal = hello
            .metal_grid
            .as_ref()
            .map(|grid| {
                let (cols, rows, values) = transpose_scalar(grid)?;
                let step_x = hello.map_size[0] / cols as f32;
                let step_z = hello.map_size[1] / rows as f32;
                Ok(ContinuousMetalSource {
                    density: ScalarGridSource {
                        cols,
                        rows,
                        origin_x: step_x * 0.5,
                        origin_z: step_z * 0.5,
                        step_x,
                        step_z,
                        values,
                    },
                    extractor_radius: options.extractor_radius,
                    extraction_scale: options.extraction_scale,
                })
            })
            .transpose()?;
        let source = Self {
            source_version: MAP_SOURCE_VERSION,
            map_name: hello.map_name,
            content_hash: options.content_hash,
            width: hello.map_size[0],
            depth: hello.map_size[1],
            water_level: hello.water_level.unwrap_or(options.fallback_water_level),
            heights: ScalarGridSource {
                cols: height_cols,
                rows: height_rows,
                origin_x: height_step_x * 0.5,
                origin_z: height_step_z * 0.5,
                step_x: height_step_x,
                step_z: height_step_z,
                values: height_values,
            },
            passability,
            metal_nodes: hello
                .metal_spots
                .into_iter()
                .map(|spot| ResourceNode {
                    x: spot.x,
                    y: spot.y,
                    z: spot.z,
                    yield_value: spot.worth,
                })
                .collect(),
            continuous_metal,
            geothermal_nodes: hello
                .geothermal_spots
                .into_iter()
                .map(|vent| GeothermalNode {
                    x: vent.x,
                    y: vent.y,
                    z: vent.z,
                    radius: vent.radius.unwrap_or(options.default_geothermal_radius),
                })
                .collect(),
            static_blockers: Vec::new(),
            wind_min: hello.wind_min,
            wind_max: hello.wind_max,
            tidal: hello.tidal,
            provenance: MapProvenance::default(),
        };
        validate_source(&source)?;
        Ok(source)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuantizedGrid {
    pub cols: u32,
    pub rows: u32,
    pub origin_x: f32,
    pub origin_z: f32,
    pub step_x: f32,
    pub step_z: f32,
    pub offset: f32,
    pub scale: f32,
    pub samples: Vec<u16>,
}

impl QuantizedGrid {
    fn compile(source: &ScalarGridSource) -> Result<Self, MapError> {
        validate_scalar_grid(source)?;
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        for &value in &source.values {
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
        let range = maximum - minimum;
        let scale = if range > 0.0 {
            range / u16::MAX as f32
        } else {
            1.0
        };
        let samples = source
            .values
            .iter()
            .map(|&value| {
                ((value - minimum) / scale)
                    .round()
                    .clamp(0.0, u16::MAX as f32) as u16
            })
            .collect();
        Ok(Self {
            cols: source.cols,
            rows: source.rows,
            origin_x: source.origin_x,
            origin_z: source.origin_z,
            step_x: source.step_x,
            step_z: source.step_z,
            offset: minimum,
            scale,
            samples,
        })
    }

    #[inline]
    fn decode(&self, index: usize) -> f32 {
        self.offset + self.samples[index] as f32 * self.scale
    }

    /// Bilinear sample, clamped to the map's sampled extent.
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let fx = ((x - self.origin_x) / self.step_x).clamp(0.0, self.cols.saturating_sub(1) as f32);
        let fz = ((z - self.origin_z) / self.step_z).clamp(0.0, self.rows.saturating_sub(1) as f32);
        let x0 = fx.floor() as usize;
        let z0 = fz.floor() as usize;
        let x1 = (x0 + 1).min(self.cols as usize - 1);
        let z1 = (z0 + 1).min(self.rows as usize - 1);
        let tx = fx - x0 as f32;
        let tz = fz - z0 as f32;
        let cols = self.cols as usize;
        let top = lerp(self.decode(z0 * cols + x0), self.decode(z0 * cols + x1), tx);
        let bottom = lerp(self.decode(z1 * cols + x0), self.decode(z1 * cols + x1), tx);
        lerp(top, bottom, tz)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassabilityLayer {
    pub movement_class: String,
    pub cols: u32,
    pub rows: u32,
    pub cell_size_x: f32,
    pub cell_size_z: f32,
    bits: Vec<u64>,
}

impl PassabilityLayer {
    #[inline]
    pub fn is_passable(&self, x: f32, z: f32) -> bool {
        self.index(x, z)
            .is_some_and(|index| self.bits[index / 64] & (1_u64 << (index % 64)) != 0)
    }

    #[inline]
    fn index(&self, x: f32, z: f32) -> Option<usize> {
        if !x.is_finite() || !z.is_finite() || x < 0.0 || z < 0.0 {
            return None;
        }
        let col = (x / self.cell_size_x) as u32;
        let row = (z / self.cell_size_z) as u32;
        (col < self.cols && row < self.rows).then_some((row * self.cols + col) as usize)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetalField {
    pub density: QuantizedGrid,
    pub extractor_radius: f32,
    pub extraction_scale: f32,
    /// Preconvolved yield at each legal density-grid center for O(1) queries.
    extraction_yield: QuantizedGrid,
}

impl MetalField {
    #[inline]
    pub fn density_at(&self, x: f32, z: f32) -> f32 {
        self.density.sample(x, z)
    }

    #[inline]
    pub fn extractor_yield_at(&self, x: f32, z: f32) -> f32 {
        self.extraction_yield.sample(x, z)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledMap {
    pub cache_version: u16,
    pub map_name: String,
    pub content_hash: String,
    pub provenance: MapProvenance,
    pub width: f32,
    pub depth: f32,
    pub water_level: f32,
    pub heights: QuantizedGrid,
    pub passability: Vec<PassabilityLayer>,
    pub metal_nodes: Vec<ResourceNode>,
    pub metal_field: Option<MetalField>,
    pub geothermal_nodes: Vec<GeothermalNode>,
    pub static_blockers: Vec<StaticBlocker>,
    pub wind_min: f32,
    pub wind_max: f32,
    pub tidal: f32,
}

impl CompiledMap {
    /// Loads either a compact `.vsgmap` cache or an external JSON map
    /// description. JSON may be canonical [`MapSource`] data or the bridge's
    /// `hello` shape. Source JSON is compiled once for this process; callers
    /// running many trials should use `--compile-map` to persist the result.
    ///
    /// This is intentionally a map-only operation. It does not create a
    /// world, add units, or mutate any economy/policy state. Resource nodes,
    /// continuous metal, geothermal vents, water, movement masks and static
    /// blockers are retained by the resulting [`CompiledMap`].
    pub fn load_path(
        path: &Path,
        options: MapLoadOptions,
    ) -> Result<(Self, MapLoadFormat), MapError> {
        let bytes = fs::read(path).map_err(|error| MapError::Io(error.to_string()))?;
        if bytes.starts_with(&CACHE_MAGIC) {
            return Self::from_cache_bytes(&bytes).map(|map| (map, MapLoadFormat::CompiledCache));
        }

        let json = std::str::from_utf8(&bytes).map_err(|_| {
            MapError::Json("map source must be UTF-8 JSON or a VSGMAP cache".into())
        })?;
        let (mut source, source_format) = MapSource::from_json(json, options.hello)?;
        if source_format == MapInputFormat::NativeSource
            && options.load_sibling_acquisition
            && let Some(acquisition_path) =
                sibling_acquisition_path(path).filter(|sidecar| sidecar.is_file())
        {
            let acquisition = fs::read_to_string(&acquisition_path).map_err(|error| {
                MapError::Io(format!("{}: {error}", acquisition_path.display()))
            })?;
            source.apply_acquisition_json(&acquisition)?;
        }
        let map = Self::compile(source)?;
        let format = match source_format {
            MapInputFormat::NativeSource => MapLoadFormat::NativeSource,
            MapInputFormat::Hello => MapLoadFormat::Hello,
        };
        Ok((map, format))
    }

    #[inline]
    pub fn average_wind(&self) -> f32 {
        (self.wind_min + self.wind_max) * 0.5
    }
    pub fn compile(source: MapSource) -> Result<Self, MapError> {
        validate_source(&source)?;
        let heights = QuantizedGrid::compile(&source.heights)?;
        let mut passability = Vec::with_capacity(source.passability.len());
        for layer in source.passability {
            if passability
                .iter()
                .any(|existing: &PassabilityLayer| existing.movement_class == layer.movement_class)
            {
                return Err(MapError::DuplicateMovementClass(layer.movement_class));
            }
            let mut bits = vec![0_u64; layer.passable.len().div_ceil(64)];
            for (index, &open) in layer.passable.iter().enumerate() {
                if open {
                    bits[index / 64] |= 1_u64 << (index % 64);
                }
            }
            passability.push(PassabilityLayer {
                movement_class: layer.movement_class,
                cols: layer.cols,
                rows: layer.rows,
                cell_size_x: layer.cell_size_x,
                cell_size_z: layer.cell_size_z,
                bits,
            });
        }
        let metal_field = source
            .continuous_metal
            .as_ref()
            .map(compile_metal_field)
            .transpose()?;
        let mut static_blockers = source.static_blockers;
        static_blockers.sort_by(|left, right| {
            left.feature_id
                .cmp(&right.feature_id)
                .then_with(|| left.definition.cmp(&right.definition))
                .then_with(|| left.z.total_cmp(&right.z))
                .then_with(|| left.x.total_cmp(&right.x))
        });
        Ok(Self {
            cache_version: MAP_CACHE_VERSION,
            map_name: source.map_name,
            content_hash: source.content_hash,
            provenance: source.provenance,
            width: source.width,
            depth: source.depth,
            water_level: source.water_level,
            heights,
            passability,
            metal_nodes: source.metal_nodes,
            metal_field,
            geothermal_nodes: source.geothermal_nodes,
            static_blockers,
            wind_min: source.wind_min.unwrap_or(0.0),
            wind_max: source.wind_max.unwrap_or(0.0),
            tidal: source.tidal.unwrap_or(0.0),
        })
    }

    #[inline]
    pub fn contains(&self, x: f32, z: f32) -> bool {
        x.is_finite() && z.is_finite() && x >= 0.0 && z >= 0.0 && x < self.width && z < self.depth
    }

    #[inline]
    pub fn ground_height(&self, x: f32, z: f32) -> f32 {
        self.heights.sample(x, z)
    }

    #[inline]
    pub fn water_depth(&self, x: f32, z: f32) -> f32 {
        (self.water_level - self.ground_height(x, z)).max(0.0)
    }

    #[inline]
    pub fn is_underwater(&self, x: f32, z: f32) -> bool {
        self.ground_height(x, z) < self.water_level
    }

    pub fn movement_layer(&self, movement_class: &str) -> Option<&PassabilityLayer> {
        self.passability
            .iter()
            .find(|layer| layer.movement_class == movement_class)
    }

    #[inline]
    pub fn is_passable(&self, movement_class: &str, x: f32, z: f32) -> bool {
        self.contains(x, z)
            && self
                .movement_layer(movement_class)
                .is_some_and(|layer| layer.is_passable(x, z))
    }

    /// Materializes a flat passability mask for a specific movement class
    /// conforming to the tactical terrain dimensions with the given cell size.
    pub fn tactical_passability_mask(
        &self,
        movement_class: &str,
        cell_size: f32,
    ) -> Result<Vec<bool>, MapError> {
        if !finite_positive(cell_size) {
            return Err(MapError::Invalid("invalid tactical terrain cell size"));
        }
        if self.movement_layer(movement_class).is_none() {
            return Err(MapError::UnknownMovementClass(movement_class.into()));
        }
        let exact_cols = self.width / cell_size;
        let exact_rows = self.depth / cell_size;
        if (exact_cols - exact_cols.round()).abs() > 1e-4
            || (exact_rows - exact_rows.round()).abs() > 1e-4
        {
            return Err(MapError::Invalid(
                "tactical cell size must evenly divide both map dimensions",
            ));
        }
        let cols = exact_cols.round() as usize;
        let rows = exact_rows.round() as usize;
        let mut passable = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            let z = (row as f32 + 0.5) * cell_size;
            for col in 0..cols {
                let x = (col as f32 + 0.5) * cell_size;
                passable.push(self.is_passable(movement_class, x, z));
            }
        }
        Ok(passable)
    }

    /// Materializes the simulator's current square-grid tactical terrain from
    /// this richer cache. Do this once per scenario, never once per trial.
    pub fn to_tactical_terrain(
        &self,
        movement_class: &str,
        cell_size: f32,
    ) -> Result<Terrain, MapError> {
        let passable = self.tactical_passability_mask(movement_class, cell_size)?;
        let cols = (self.width / cell_size).round() as usize;
        let rows = (self.depth / cell_size).round() as usize;
        let mut heights = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            let z = (row as f32 + 0.5) * cell_size;
            for col in 0..cols {
                let x = (col as f32 + 0.5) * cell_size;
                heights.push(self.ground_height(x, z));
            }
        }
        Ok(Terrain {
            cols,
            rows,
            cell_size,
            water_level: self.water_level,
            maximum_height: heights.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            heights,
            passable,
        })
    }

    /// Best estimate from the raw field or BAR's discrete spot finder. These
    /// normally describe the same metal, so they are not added together.
    pub fn metal_yield_at(&self, x: f32, z: f32, node_radius: f32) -> f32 {
        let field = self
            .metal_field
            .as_ref()
            .map_or(0.0, |metal| metal.extractor_yield_at(x, z));
        let radius_sq = node_radius.max(0.0).powi(2);
        let node = self
            .metal_nodes
            .iter()
            .filter(|spot| (spot.x - x).powi(2) + (spot.z - z).powi(2) <= radius_sq)
            .map(|spot| spot.yield_value)
            .fold(0.0, f32::max);
        field.max(node)
    }

    pub fn geothermal_at(&self, x: f32, z: f32, extra_tolerance: f32) -> Option<usize> {
        self.geothermal_nodes.iter().position(|vent| {
            let radius = (vent.radius + extra_tolerance).max(0.0);
            (vent.x - x).powi(2) + (vent.z - z).powi(2) <= radius * radius
        })
    }

    /// Portable little-endian cache. This is much smaller and faster to load than
    /// acquisition JSON while remaining independent of platform layout.
    pub fn to_cache_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&CACHE_MAGIC);
        put_u16(&mut out, self.cache_version);
        put_string(&mut out, &self.map_name);
        put_string(&mut out, &self.content_hash);
        put_string(
            &mut out,
            &serde_json::to_string(&self.provenance).expect("map provenance serializes"),
        );
        put_f32(&mut out, self.width);
        put_f32(&mut out, self.depth);
        put_f32(&mut out, self.water_level);
        put_f32(&mut out, self.wind_min);
        put_f32(&mut out, self.wind_max);
        put_f32(&mut out, self.tidal);
        put_grid(&mut out, &self.heights);
        put_u32(&mut out, self.passability.len() as u32);
        for layer in &self.passability {
            put_string(&mut out, &layer.movement_class);
            put_u32(&mut out, layer.cols);
            put_u32(&mut out, layer.rows);
            put_f32(&mut out, layer.cell_size_x);
            put_f32(&mut out, layer.cell_size_z);
            put_u64_vec(&mut out, &layer.bits);
        }
        put_nodes(&mut out, &self.metal_nodes);
        out.push(u8::from(self.metal_field.is_some()));
        if let Some(field) = &self.metal_field {
            put_grid(&mut out, &field.density);
            put_f32(&mut out, field.extractor_radius);
            put_f32(&mut out, field.extraction_scale);
            put_grid(&mut out, &field.extraction_yield);
        }
        put_u32(&mut out, self.geothermal_nodes.len() as u32);
        for vent in &self.geothermal_nodes {
            put_f32(&mut out, vent.x);
            put_f32(&mut out, vent.y);
            put_f32(&mut out, vent.z);
            put_f32(&mut out, vent.radius);
        }
        put_u32(&mut out, self.static_blockers.len() as u32);
        for blocker in &self.static_blockers {
            put_u32(&mut out, blocker.feature_id);
            put_string(&mut out, &blocker.definition);
            for value in [
                blocker.x,
                blocker.y,
                blocker.z,
                blocker.min_x,
                blocker.min_z,
                blocker.max_x,
                blocker.max_z,
                blocker.fire_height,
                blocker.reclaim_time,
                blocker.metal,
                blocker.energy,
            ] {
                put_f32(&mut out, value);
            }
            let flags = u8::from(blocker.blocks_movement)
                | (u8::from(blocker.blocks_fire) << 1)
                | (u8::from(blocker.crushable) << 2)
                | (u8::from(blocker.reclaimable) << 3)
                | (u8::from(blocker.geothermal) << 4);
            out.push(flags);
        }
        out
    }

    pub fn from_cache_bytes(bytes: &[u8]) -> Result<Self, MapError> {
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != CACHE_MAGIC {
            return Err(MapError::CorruptCache("bad magic"));
        }
        let cache_version = reader.u16()?;
        if cache_version != MAP_CACHE_VERSION {
            return Err(MapError::UnsupportedCacheVersion(cache_version));
        }
        let map_name = reader.string()?;
        let content_hash = reader.string()?;
        let provenance = serde_json::from_str(&reader.string()?)
            .map_err(|_| MapError::CorruptCache("invalid provenance JSON"))?;
        let width = reader.f32()?;
        let depth = reader.f32()?;
        let water_level = reader.f32()?;
        let wind_min = reader.f32()?;
        let wind_max = reader.f32()?;
        let tidal = reader.f32()?;
        let heights = reader.grid()?;
        let layer_count = reader.len()?;
        let mut passability = Vec::with_capacity(layer_count);
        for _ in 0..layer_count {
            let movement_class = reader.string()?;
            let cols = reader.u32()?;
            let rows = reader.u32()?;
            let cell_size_x = reader.f32()?;
            let cell_size_z = reader.f32()?;
            let bits = reader.u64_vec()?;
            let expected = checked_cell_count(cols, rows)?.div_ceil(64);
            if bits.len() != expected {
                return Err(MapError::CorruptCache("passability bit count mismatch"));
            }
            passability.push(PassabilityLayer {
                movement_class,
                cols,
                rows,
                cell_size_x,
                cell_size_z,
                bits,
            });
        }
        let metal_nodes = reader.nodes()?;
        let metal_field = match reader.u8()? {
            0 => None,
            1 => Some(MetalField {
                density: reader.grid()?,
                extractor_radius: reader.f32()?,
                extraction_scale: reader.f32()?,
                extraction_yield: reader.grid()?,
            }),
            _ => return Err(MapError::CorruptCache("invalid optional-field marker")),
        };
        let vent_count = reader.len()?;
        let mut geothermal_nodes = Vec::with_capacity(vent_count);
        for _ in 0..vent_count {
            geothermal_nodes.push(GeothermalNode {
                x: reader.f32()?,
                y: reader.f32()?,
                z: reader.f32()?,
                radius: reader.f32()?,
            });
        }
        let blocker_count = reader.len()?;
        let mut static_blockers = Vec::with_capacity(blocker_count);
        for _ in 0..blocker_count {
            let feature_id = reader.u32()?;
            let definition = reader.string()?;
            let x = reader.f32()?;
            let y = reader.f32()?;
            let z = reader.f32()?;
            let min_x = reader.f32()?;
            let min_z = reader.f32()?;
            let max_x = reader.f32()?;
            let max_z = reader.f32()?;
            let fire_height = reader.f32()?;
            let reclaim_time = reader.f32()?;
            let metal = reader.f32()?;
            let energy = reader.f32()?;
            let flags = reader.u8()?;
            if flags & !0x1f != 0 {
                return Err(MapError::CorruptCache("invalid static blocker flags"));
            }
            static_blockers.push(StaticBlocker {
                feature_id,
                definition,
                x,
                y,
                z,
                min_x,
                min_z,
                max_x,
                max_z,
                blocks_movement: flags & 1 != 0,
                blocks_fire: flags & 2 != 0,
                fire_height,
                crushable: flags & 4 != 0,
                reclaimable: flags & 8 != 0,
                reclaim_time,
                metal,
                energy,
                geothermal: flags & 16 != 0,
            });
        }
        if !reader.is_empty() {
            return Err(MapError::TrailingCacheData);
        }
        let map = Self {
            cache_version,
            map_name,
            content_hash,
            provenance,
            width,
            depth,
            water_level,
            heights,
            passability,
            metal_nodes,
            metal_field,
            geothermal_nodes,
            static_blockers,
            wind_min,
            wind_max,
            tidal,
        };
        validate_compiled(&map)?;
        Ok(map)
    }
}

/// Per-trial static occupancy. It is intentionally separate from `CompiledMap`
/// so the latter can be shared by all Rayon workers. Mobile-unit collision stays
/// in the simulator's spatial index; this tracks structures, reservations and walls.
#[derive(Clone, Debug)]
pub struct DynamicOccupancy {
    cols: u32,
    rows: u32,
    cell_size_x: f32,
    cell_size_z: f32,
    bits: Vec<u64>,
    epoch: u64,
}

impl DynamicOccupancy {
    pub fn new(width: f32, depth: f32, cell_size: f32) -> Result<Self, MapError> {
        if !finite_positive(width) || !finite_positive(depth) || !finite_positive(cell_size) {
            return Err(MapError::Invalid("invalid occupancy dimensions"));
        }
        let cols = (width / cell_size).ceil() as u32;
        let rows = (depth / cell_size).ceil() as u32;
        let count = checked_cell_count(cols, rows)?;
        Ok(Self {
            cols,
            rows,
            cell_size_x: width / cols as f32,
            cell_size_z: depth / rows as f32,
            bits: vec![0; count.div_ceil(64)],
            epoch: 0,
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn is_blocked(&self, x: f32, z: f32) -> bool {
        self.index(x, z)
            .is_none_or(|index| self.bits[index / 64] & (1_u64 << (index % 64)) != 0)
    }

    /// Marks an axis-aligned footprint. Building facings should rotate the
    /// footprint dimensions before this call. Returns false if it leaves the map.
    pub fn set_rect(
        &mut self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        blocked: bool,
    ) -> bool {
        let Some((left, top, right, bottom)) = self.cell_rect(min_x, min_z, max_x, max_z) else {
            return false;
        };
        let mut changed = false;
        for row in top..=bottom {
            for col in left..=right {
                let index = (row * self.cols + col) as usize;
                let word = &mut self.bits[index / 64];
                let mask = 1_u64 << (index % 64);
                let old = *word & mask != 0;
                if blocked {
                    *word |= mask;
                } else {
                    *word &= !mask;
                }
                changed |= old != blocked;
            }
        }
        if changed {
            self.epoch = self.epoch.wrapping_add(1);
        }
        true
    }

    pub fn rect_is_clear(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        let Some((left, top, right, bottom)) = self.cell_rect(min_x, min_z, max_x, max_z) else {
            return false;
        };
        (top..=bottom).all(|row| {
            (left..=right).all(|col| {
                let index = (row * self.cols + col) as usize;
                self.bits[index / 64] & (1_u64 << (index % 64)) == 0
            })
        })
    }

    fn index(&self, x: f32, z: f32) -> Option<usize> {
        if !x.is_finite() || !z.is_finite() || x < 0.0 || z < 0.0 {
            return None;
        }
        let col = (x / self.cell_size_x) as u32;
        let row = (z / self.cell_size_z) as u32;
        (col < self.cols && row < self.rows).then_some((row * self.cols + col) as usize)
    }

    fn cell_rect(
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
            || max_x > self.cols as f32 * self.cell_size_x
            || max_z > self.rows as f32 * self.cell_size_z
        {
            return None;
        }
        let left = (min_x / self.cell_size_x).floor() as u32;
        let top = (min_z / self.cell_size_z).floor() as u32;
        // A footprint ending exactly on a cell edge must not occupy the next cell.
        let right = ((max_x / self.cell_size_x).ceil() as u32).saturating_sub(1);
        let bottom = ((max_z / self.cell_size_z).ceil() as u32).saturating_sub(1);
        Some((left, top, right, bottom))
    }
}

fn compile_metal_field(source: &ContinuousMetalSource) -> Result<MetalField, MapError> {
    if !finite_positive(source.extractor_radius)
        || !source.extraction_scale.is_finite()
        || source.extraction_scale < 0.0
    {
        return Err(MapError::Invalid("invalid continuous-metal parameters"));
    }
    validate_scalar_grid(&source.density)?;
    if source.density.values.iter().any(|value| *value < 0.0) {
        return Err(MapError::Invalid("negative metal density"));
    }
    let grid = &source.density;
    let mut yields = vec![0.0; grid.values.len()];
    let rz = (source.extractor_radius / grid.step_z).ceil() as i32;
    let radius_sq = source.extractor_radius * source.extractor_radius;
    // Prefix each row once. A circle intersects each row in one contiguous
    // span, reducing preprocessing from O(cells * rx * rz) to O(cells * rz).
    let prefix_cols = grid.cols as usize + 1;
    let mut row_prefix = vec![0.0_f32; prefix_cols * grid.rows as usize];
    for row in 0..grid.rows as usize {
        for col in 0..grid.cols as usize {
            row_prefix[row * prefix_cols + col + 1] =
                row_prefix[row * prefix_cols + col] + grid.values[row * grid.cols as usize + col];
        }
    }
    for center_z in 0..grid.rows as i32 {
        for center_x in 0..grid.cols as i32 {
            let mut sum = 0.0;
            for dz in -rz..=rz {
                let sample_z = center_z + dz;
                if !(0..grid.rows as i32).contains(&sample_z) {
                    continue;
                }
                let world_dz = dz as f32 * grid.step_z;
                let world_dz_sq = world_dz * world_dz;
                if world_dz_sq > radius_sq {
                    continue;
                }
                let half_width = ((radius_sq - world_dz_sq).sqrt() / grid.step_x).floor() as i32;
                let left = (center_x - half_width).max(0) as usize;
                let right = (center_x + half_width).min(grid.cols as i32 - 1) as usize;
                let prefix_row = sample_z as usize * prefix_cols;
                sum += row_prefix[prefix_row + right + 1] - row_prefix[prefix_row + left];
            }
            yields[(center_z as u32 * grid.cols + center_x as u32) as usize] =
                sum * source.extraction_scale;
        }
    }
    let yield_source = ScalarGridSource {
        cols: grid.cols,
        rows: grid.rows,
        origin_x: grid.origin_x,
        origin_z: grid.origin_z,
        step_x: grid.step_x,
        step_z: grid.step_z,
        values: yields,
    };
    Ok(MetalField {
        density: QuantizedGrid::compile(grid)?,
        extractor_radius: source.extractor_radius,
        extraction_scale: source.extraction_scale,
        extraction_yield: QuantizedGrid::compile(&yield_source)?,
    })
}

fn transpose_scalar(columns: &[Vec<f32>]) -> Result<(u32, u32, Vec<f32>), MapError> {
    if columns.is_empty()
        || columns[0].is_empty()
        || columns
            .iter()
            .any(|column| column.len() != columns[0].len())
    {
        return Err(MapError::Invalid("ragged or empty X-major scalar grid"));
    }
    let cols = u32::try_from(columns.len()).map_err(|_| MapError::Invalid("grid is too large"))?;
    let rows =
        u32::try_from(columns[0].len()).map_err(|_| MapError::Invalid("grid is too large"))?;
    let mut values = Vec::with_capacity(checked_cell_count(cols, rows)?);
    for row in 0..rows as usize {
        for column in columns {
            values.push(column[row]);
        }
    }
    Ok((cols, rows, values))
}

fn transpose_mask(columns: &[Vec<MaskValue>]) -> Result<(u32, u32, Vec<bool>), MapError> {
    if columns.is_empty()
        || columns[0].is_empty()
        || columns
            .iter()
            .any(|column| column.len() != columns[0].len())
    {
        return Err(MapError::Invalid("ragged or empty X-major mask grid"));
    }
    let cols = u32::try_from(columns.len()).map_err(|_| MapError::Invalid("grid is too large"))?;
    let rows =
        u32::try_from(columns[0].len()).map_err(|_| MapError::Invalid("grid is too large"))?;
    let mut values = Vec::with_capacity(checked_cell_count(cols, rows)?);
    for row in 0..rows as usize {
        for column in columns {
            values.push(column[row].get());
        }
    }
    Ok((cols, rows, values))
}

fn validate_source(source: &MapSource) -> Result<(), MapError> {
    if source.source_version != MAP_SOURCE_VERSION {
        return Err(MapError::UnsupportedSourceVersion(source.source_version));
    }
    if source.map_name.is_empty()
        || !finite_positive(source.width)
        || !finite_positive(source.depth)
    {
        return Err(MapError::Invalid("missing name or invalid map dimensions"));
    }
    if !source.water_level.is_finite() {
        return Err(MapError::Invalid("invalid water level"));
    }
    if let Some(wmin) = source.wind_min
        && (!wmin.is_finite() || wmin < 0.0)
    {
        return Err(MapError::Invalid("invalid wind_min"));
    }
    if let Some(wmax) = source.wind_max
        && (!wmax.is_finite() || wmax < 0.0)
    {
        return Err(MapError::Invalid("invalid wind_max"));
    }
    if let (Some(wmin), Some(wmax)) = (source.wind_min, source.wind_max)
        && wmin > wmax
    {
        return Err(MapError::Invalid("wind_min exceeds wind_max"));
    }
    if let Some(tidal) = source.tidal
        && (!tidal.is_finite() || tidal < 0.0)
    {
        return Err(MapError::Invalid("invalid tidal"));
    }
    validate_scalar_grid(&source.heights)?;
    for layer in &source.passability {
        if layer.movement_class.is_empty()
            || !finite_positive(layer.cell_size_x)
            || !finite_positive(layer.cell_size_z)
            || checked_cell_count(layer.cols, layer.rows)? != layer.passable.len()
            || (layer.cols as f32 * layer.cell_size_x - source.width).abs() > 0.01
            || (layer.rows as f32 * layer.cell_size_z - source.depth).abs() > 0.01
        {
            return Err(MapError::Invalid("invalid passability layer"));
        }
    }
    if source.metal_nodes.iter().any(|node| {
        ![node.x, node.y, node.z, node.yield_value]
            .iter()
            .all(|value| value.is_finite())
            || node.yield_value < 0.0
    }) {
        return Err(MapError::Invalid("invalid metal node"));
    }
    if source.geothermal_nodes.iter().any(|vent| {
        ![vent.x, vent.y, vent.z, vent.radius]
            .iter()
            .all(|value| value.is_finite())
            || vent.radius < 0.0
    }) {
        return Err(MapError::Invalid("invalid geothermal node"));
    }
    validate_static_blockers(&source.static_blockers, source.width, source.depth)?;
    validate_provenance(
        &source.provenance,
        &source.map_name,
        &source.content_hash,
        source
            .passability
            .iter()
            .map(|layer| layer.movement_class.as_str()),
    )?;
    Ok(())
}

fn validate_static_blockers(
    blockers: &[StaticBlocker],
    width: f32,
    depth: f32,
) -> Result<(), MapError> {
    let mut feature_ids = BTreeSet::new();
    for blocker in blockers {
        if !feature_ids.insert(blocker.feature_id)
            || blocker.definition.is_empty()
            || ![
                blocker.x,
                blocker.y,
                blocker.z,
                blocker.min_x,
                blocker.min_z,
                blocker.max_x,
                blocker.max_z,
                blocker.fire_height,
                blocker.reclaim_time,
                blocker.metal,
                blocker.energy,
            ]
            .iter()
            .all(|value| value.is_finite())
            || blocker.x < 0.0
            || blocker.x >= width
            || blocker.z < 0.0
            || blocker.z >= depth
            || blocker.min_x < 0.0
            || blocker.min_z < 0.0
            || blocker.max_x <= blocker.min_x
            || blocker.max_z <= blocker.min_z
            || blocker.max_x > width
            || blocker.max_z > depth
            || blocker.fire_height < 0.0
            || blocker.reclaim_time < 0.0
            || blocker.metal < 0.0
            || blocker.energy < 0.0
            || (blocker.blocks_fire && blocker.fire_height <= 0.0)
            || (blocker.reclaimable && blocker.reclaim_time <= 0.0)
        {
            return Err(MapError::Invalid("invalid static blocker"));
        }
    }
    Ok(())
}

fn validate_provenance<'a>(
    provenance: &MapProvenance,
    map_name: &str,
    content_hash: &str,
    movement_classes: impl Iterator<Item = &'a str>,
) -> Result<(), MapError> {
    if provenance == &MapProvenance::default() {
        return Ok(());
    }
    if !provenance.is_complete() || content_hash.is_empty() {
        return Err(MapError::Invalid("incomplete native map provenance"));
    }
    let identity = provenance.identity.as_ref().expect("completeness checked");
    if identity.map_name != map_name
        || [
            identity.map_checksum.as_str(),
            identity.game_name.as_str(),
            identity.game_version.as_str(),
            identity.mod_checksum.as_str(),
            identity.engine_version.as_str(),
        ]
        .iter()
        .any(|field| field.is_empty())
        || identity.exporter_revision == 0
        || !provenance.extractor_radius.is_some_and(finite_positive)
        || !provenance
            .extraction_scale
            .is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        return Err(MapError::Invalid("invalid native map provenance"));
    }
    let mut expected = movement_classes.map(str::to_owned).collect::<Vec<_>>();
    expected.sort();
    expected.dedup();
    let mut actual = provenance
        .movement_schema
        .iter()
        .filter_map(|record| record.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    actual.sort();
    actual.dedup();
    if actual != expected || actual.len() != provenance.movement_schema.len() {
        return Err(MapError::Invalid(
            "native movement schema does not match passability layers",
        ));
    }
    Ok(())
}

fn validate_scalar_grid(grid: &ScalarGridSource) -> Result<(), MapError> {
    if checked_cell_count(grid.cols, grid.rows)? != grid.values.len()
        || !finite_positive(grid.step_x)
        || !finite_positive(grid.step_z)
        || !grid.origin_x.is_finite()
        || !grid.origin_z.is_finite()
        || grid.values.iter().any(|value| !value.is_finite())
    {
        return Err(MapError::Invalid("invalid scalar grid"));
    }
    Ok(())
}

fn validate_compiled(map: &CompiledMap) -> Result<(), MapError> {
    if !finite_positive(map.width)
        || !finite_positive(map.depth)
        || !map.water_level.is_finite()
        || !valid_quantized_grid(&map.heights)?
        || !map.wind_min.is_finite()
        || map.wind_min < 0.0
        || !map.wind_max.is_finite()
        || map.wind_max < 0.0
        || map.wind_min > map.wind_max
        || !map.tidal.is_finite()
        || map.tidal < 0.0
    {
        return Err(MapError::CorruptCache("invalid compiled map metadata"));
    }
    for layer in &map.passability {
        let cells = checked_cell_count(layer.cols, layer.rows)?;
        if layer.movement_class.is_empty()
            || !finite_positive(layer.cell_size_x)
            || !finite_positive(layer.cell_size_z)
            || layer.bits.len() != cells.div_ceil(64)
        {
            return Err(MapError::CorruptCache("invalid passability layer"));
        }
    }
    if let Some(field) = &map.metal_field
        && (!valid_quantized_grid(&field.density)?
            || !valid_quantized_grid(&field.extraction_yield)?
            || !finite_positive(field.extractor_radius)
            || !field.extraction_scale.is_finite()
            || field.extraction_scale < 0.0)
    {
        return Err(MapError::CorruptCache("invalid continuous-metal field"));
    }
    validate_static_blockers(&map.static_blockers, map.width, map.depth)
        .map_err(|_| MapError::CorruptCache("invalid static blocker"))?;
    validate_provenance(
        &map.provenance,
        &map.map_name,
        &map.content_hash,
        map.passability
            .iter()
            .map(|layer| layer.movement_class.as_str()),
    )
    .map_err(|_| MapError::CorruptCache("invalid native map provenance"))?;
    Ok(())
}

fn valid_quantized_grid(grid: &QuantizedGrid) -> Result<bool, MapError> {
    Ok(
        grid.samples.len() == checked_cell_count(grid.cols, grid.rows)?
            && finite_positive(grid.step_x)
            && finite_positive(grid.step_z)
            && grid.origin_x.is_finite()
            && grid.origin_z.is_finite()
            && grid.offset.is_finite()
            && finite_positive(grid.scale),
    )
}

fn checked_cell_count(cols: u32, rows: u32) -> Result<usize, MapError> {
    if cols == 0 || rows == 0 {
        return Err(MapError::Invalid("empty grid"));
    }
    (cols as usize)
        .checked_mul(rows as usize)
        .ok_or(MapError::Invalid("grid is too large"))
}

#[inline]
fn finite_positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_string(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}
fn put_u64_vec(out: &mut Vec<u8>, values: &[u64]) {
    put_u32(out, values.len() as u32);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}
fn put_grid(out: &mut Vec<u8>, grid: &QuantizedGrid) {
    put_u32(out, grid.cols);
    put_u32(out, grid.rows);
    for value in [
        grid.origin_x,
        grid.origin_z,
        grid.step_x,
        grid.step_z,
        grid.offset,
        grid.scale,
    ] {
        put_f32(out, value);
    }
    put_u32(out, grid.samples.len() as u32);
    for sample in &grid.samples {
        put_u16(out, *sample);
    }
}
fn put_nodes(out: &mut Vec<u8>, nodes: &[ResourceNode]) {
    put_u32(out, nodes.len() as u32);
    for node in nodes {
        for value in [node.x, node.y, node.z, node.yield_value] {
            put_f32(out, value);
        }
    }
}

struct Reader<'a> {
    remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }
    fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], MapError> {
        if count > self.remaining.len() {
            return Err(MapError::CorruptCache("unexpected end of file"));
        }
        let (head, tail) = self.remaining.split_at(count);
        self.remaining = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, MapError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, MapError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, MapError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, MapError> {
        let value = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        value
            .is_finite()
            .then_some(value)
            .ok_or(MapError::CorruptCache("non-finite float"))
    }
    fn len(&mut self) -> Result<usize, MapError> {
        let value = self.u32()? as usize;
        (value <= self.remaining.len())
            .then_some(value)
            .ok_or(MapError::CorruptCache("unreasonable collection length"))
    }
    fn string(&mut self) -> Result<String, MapError> {
        let length = self.len()?;
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| MapError::CorruptCache("invalid UTF-8"))
    }
    fn u64_vec(&mut self) -> Result<Vec<u64>, MapError> {
        let length = self.len()?;
        let mut out = Vec::with_capacity(length);
        for _ in 0..length {
            out.push(u64::from_le_bytes(self.take(8)?.try_into().unwrap()));
        }
        Ok(out)
    }
    fn grid(&mut self) -> Result<QuantizedGrid, MapError> {
        let cols = self.u32()?;
        let rows = self.u32()?;
        let origin_x = self.f32()?;
        let origin_z = self.f32()?;
        let step_x = self.f32()?;
        let step_z = self.f32()?;
        let offset = self.f32()?;
        let scale = self.f32()?;
        let length = self.len()?;
        if length != checked_cell_count(cols, rows)? {
            return Err(MapError::CorruptCache("grid sample count mismatch"));
        }
        let mut samples = Vec::with_capacity(length);
        for _ in 0..length {
            samples.push(self.u16()?);
        }
        if !finite_positive(step_x) || !finite_positive(step_z) || !finite_positive(scale) {
            return Err(MapError::CorruptCache("invalid quantized grid metadata"));
        }
        Ok(QuantizedGrid {
            cols,
            rows,
            origin_x,
            origin_z,
            step_x,
            step_z,
            offset,
            scale,
            samples,
        })
    }
    fn nodes(&mut self) -> Result<Vec<ResourceNode>, MapError> {
        let length = self.len()?;
        let mut nodes = Vec::with_capacity(length);
        for _ in 0..length {
            nodes.push(ResourceNode {
                x: self.f32()?,
                y: self.f32()?,
                z: self.f32()?,
                yield_value: self.f32()?,
            });
        }
        Ok(nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> MapSource {
        MapSource {
            source_version: MAP_SOURCE_VERSION,
            map_name: "Tiny Strait".into(),
            content_hash: "sha256:test".into(),
            width: 24.0,
            depth: 24.0,
            water_level: 2.0,
            heights: ScalarGridSource {
                cols: 3,
                rows: 3,
                origin_x: 0.0,
                origin_z: 0.0,
                step_x: 12.0,
                step_z: 12.0,
                values: vec![4.0, 4.0, 4.0, 4.0, -6.0, 4.0, 4.0, 4.0, 4.0],
            },
            passability: vec![
                PassabilitySource {
                    movement_class: "BOT3".into(),
                    cols: 3,
                    rows: 3,
                    cell_size_x: 8.0,
                    cell_size_z: 8.0,
                    passable: vec![true, true, true, true, false, true, true, true, true],
                },
                PassabilitySource {
                    movement_class: "BOAT3".into(),
                    cols: 3,
                    rows: 3,
                    cell_size_x: 8.0,
                    cell_size_z: 8.0,
                    passable: vec![false, false, false, false, true, false, false, false, false],
                },
            ],
            metal_nodes: vec![ResourceNode {
                x: 4.0,
                y: 4.0,
                z: 4.0,
                yield_value: 2.0,
            }],
            continuous_metal: Some(ContinuousMetalSource {
                density: ScalarGridSource {
                    cols: 3,
                    rows: 3,
                    origin_x: 4.0,
                    origin_z: 4.0,
                    step_x: 8.0,
                    step_z: 8.0,
                    values: vec![1.0; 9],
                },
                extractor_radius: 8.1,
                extraction_scale: 0.5,
            }),
            geothermal_nodes: vec![GeothermalNode {
                x: 20.0,
                y: 4.0,
                z: 20.0,
                radius: 8.0,
            }],
            static_blockers: vec![StaticBlocker {
                feature_id: 7,
                definition: "rock".into(),
                x: 12.0,
                y: 4.0,
                z: 4.0,
                min_x: 8.0,
                min_z: 0.0,
                max_x: 16.0,
                max_z: 8.0,
                blocks_movement: true,
                blocks_fire: true,
                fire_height: 6.0,
                crushable: false,
                reclaimable: true,
                reclaim_time: 10.0,
                metal: 3.0,
                energy: 1.0,
                geothermal: false,
            }],
            wind_min: Some(5.0),
            wind_max: Some(20.0),
            tidal: Some(15.0),
            provenance: MapProvenance::default(),
        }
    }

    #[test]
    fn terrain_water_and_movement_classes_remain_distinct() {
        let map = CompiledMap::compile(source()).unwrap();
        assert!(map.ground_height(12.0, 12.0) < -5.99);
        assert!((map.water_depth(12.0, 12.0) - 8.0).abs() < 0.01);
        assert!(!map.is_passable("BOT3", 12.0, 12.0));
        assert!(map.is_passable("BOAT3", 12.0, 12.0));
        assert!(!map.is_passable("unknown", 4.0, 4.0));
        assert!(!map.is_passable("BOT3", 24.0, 4.0));

        let tactical = map.to_tactical_terrain("BOT3", 8.0).unwrap();
        assert_eq!((tactical.cols, tactical.rows), (3, 3));
        assert_eq!(tactical.heights.len(), 9);
        assert!(!tactical.passable[4]);
        assert!(matches!(
            map.to_tactical_terrain("missing", 8.0),
            Err(MapError::UnknownMovementClass(name)) if name == "missing"
        ));
        assert!(map.to_tactical_terrain("BOT3", 10.0).is_err());
    }

    #[test]
    fn continuous_metal_is_preconvolved_and_can_coexist_with_spots() {
        let map = CompiledMap::compile(source()).unwrap();
        // Center plus its four axial neighbors: 5 * 1 * 0.5.
        let field_yield = map
            .metal_field
            .as_ref()
            .unwrap()
            .extractor_yield_at(12.0, 12.0);
        assert!((field_yield - 2.5).abs() < 0.001);
        // Both representations are retained, but they describe the same map
        // metal and are therefore not double-counted.
        assert!((map.metal_yield_at(4.0, 4.0, 1.0) - 2.0).abs() < 0.001);
    }

    #[test]
    fn geothermal_queries_use_placement_tolerance() {
        let map = CompiledMap::compile(source()).unwrap();
        assert_eq!(map.geothermal_at(12.1, 20.0, 0.0), Some(0));
        assert_eq!(map.geothermal_at(11.9, 20.0, 0.0), None);
        assert_eq!(map.geothermal_at(11.9, 20.0, 0.2), Some(0));
    }

    #[test]
    fn binary_cache_round_trips_and_rejects_damage() {
        let map = CompiledMap::compile(source()).unwrap();
        let bytes = map.to_cache_bytes();
        let loaded = CompiledMap::from_cache_bytes(&bytes).unwrap();
        assert_eq!(loaded, map);
        assert!(CompiledMap::from_cache_bytes(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(
            CompiledMap::from_cache_bytes(&trailing),
            Err(MapError::TrailingCacheData)
        );
    }

    #[test]
    fn map_loader_accepts_external_source_or_compiled_cache_with_resources() {
        let nonce = format!("{}-{:?}", std::process::id(), std::thread::current().id());
        let safe_nonce = nonce.replace(['(', ')', ' ', '"'], "_");
        let source_path =
            std::env::temp_dir().join(format!("visigoth-map-loader-{safe_nonce}.map_source.json"));
        let cache_path =
            std::env::temp_dir().join(format!("visigoth-map-loader-{safe_nonce}.vsgmap"));
        let input = source();
        fs::write(&source_path, serde_json::to_vec(&input).unwrap()).unwrap();
        let acquisition_path = sibling_acquisition_path(&source_path).unwrap();
        fs::write(
            &acquisition_path,
            r#"{
                "format":"visigoth.native_map", "format_version":1,
                "content_hash":"sha256:test",
                "identity":{
                    "map_name":"Tiny Strait", "map_checksum":"map-42",
                    "game_name":"Beyond All Reason", "game_version":"2026.09",
                    "mod_checksum":"mod-17", "engine_version":"Recoil 2026.1",
                    "exporter_revision":2
                },
                "map":{"extractor_radius":8.1},
                "movedefs":[{"name":"BOT3"},{"name":"BOAT3"}]
            }"#,
        )
        .unwrap();

        let (loaded_source, source_format) =
            CompiledMap::load_path(&source_path, MapLoadOptions::default()).unwrap();
        assert_eq!(source_format, MapLoadFormat::NativeSource);
        assert_eq!(loaded_source.map_name, "Tiny Strait");
        assert!(loaded_source.provenance.is_complete());
        assert_eq!(
            loaded_source
                .provenance
                .identity
                .as_ref()
                .unwrap()
                .map_checksum,
            "map-42"
        );
        assert_eq!(loaded_source.metal_nodes, input.metal_nodes);
        assert_eq!(loaded_source.geothermal_nodes, input.geothermal_nodes);
        assert!(loaded_source.metal_field.is_some());
        assert_eq!(loaded_source.static_blockers, input.static_blockers);
        assert!((loaded_source.water_depth(12.0, 12.0) - 8.0).abs() < 0.01);

        fs::write(&cache_path, loaded_source.to_cache_bytes()).unwrap();
        let (loaded_cache, cache_format) =
            CompiledMap::load_path(&cache_path, MapLoadOptions::default()).unwrap();
        assert_eq!(cache_format, MapLoadFormat::CompiledCache);
        assert_eq!(loaded_cache, loaded_source);

        fs::remove_file(source_path).unwrap();
        fs::remove_file(acquisition_path).unwrap();
        fs::remove_file(cache_path).unwrap();
    }

    #[test]
    fn wall_occupancy_changes_epoch_and_obstructs_rectangles() {
        let mut occupancy = DynamicOccupancy::new(64.0, 64.0, 8.0).unwrap();
        assert!(occupancy.rect_is_clear(16.0, 16.0, 24.0, 40.0));
        assert!(occupancy.set_rect(16.0, 16.0, 24.0, 40.0, true));
        assert_eq!(occupancy.epoch(), 1);
        assert!(occupancy.is_blocked(20.0, 36.0));
        assert!(!occupancy.rect_is_clear(8.0, 24.0, 32.0, 32.0));
        assert!(occupancy.set_rect(16.0, 16.0, 24.0, 40.0, false));
        assert_eq!(occupancy.epoch(), 2);
        assert!(occupancy.rect_is_clear(8.0, 24.0, 32.0, 32.0));
    }

    #[test]
    fn malformed_or_duplicate_input_fails_early() {
        let mut bad = source();
        bad.heights.values.pop();
        assert!(CompiledMap::compile(bad).is_err());
        let mut duplicate = source();
        duplicate.passability.push(duplicate.passability[0].clone());
        assert_eq!(
            CompiledMap::compile(duplicate),
            Err(MapError::DuplicateMovementClass("BOT3".into()))
        );
        let mut outside = source();
        outside.static_blockers[0].max_x = outside.width + 1.0;
        assert_eq!(
            CompiledMap::compile(outside),
            Err(MapError::Invalid("invalid static blocker"))
        );
        let mut duplicate_feature = source();
        duplicate_feature
            .static_blockers
            .push(duplicate_feature.static_blockers[0].clone());
        assert_eq!(
            CompiledMap::compile(duplicate_feature),
            Err(MapError::Invalid("invalid static blocker"))
        );
    }

    #[test]
    fn hello_adapter_transposes_grids_and_rejects_implicit_land_only_mask() {
        let legacy = r#"{
            "map_name":"Adapter Test", "map_size":[16,16],
            "terrain":{"heights":[[1,3],[2,4]],"passability":[[1,0],[1,1]]},
            "metal_spots":[], "metal_grid":null
        }"#;
        assert_eq!(
            MapSource::from_hello_json(legacy, HelloMapOptions::default()),
            Err(MapError::MissingMovementClasses)
        );
        let options = HelloMapOptions {
            accept_legacy_mask_as: Some("legacy-land".into()),
            ..HelloMapOptions::default()
        };
        let map =
            CompiledMap::compile(MapSource::from_hello_json(legacy, options).unwrap()).unwrap();
        // X-major [[1,3],[2,4]] becomes conventional rows [[1,2],[3,4]].
        assert!((map.ground_height(4.0, 12.0) - 3.0).abs() < 0.001);
        assert!(!map.is_passable("legacy-land", 4.0, 12.0));

        let modern = r#"{
            "map_name":"Modern", "map_size":[16,16], "water_level":3,
            "terrain":{"heights":[[1,1],[1,1]],"movement_classes":{"ship":[[0,1],[0,1]]}},
            "metal_spots":[{"x":4,"z":4,"worth":2}],
            "metal_grid":[[1,1],[1,1]],
            "geothermal_spots":[{"x":8,"y":1,"z":8,"radius":12}]
        }"#;
        let source = MapSource::from_hello_json(modern, HelloMapOptions::default()).unwrap();
        assert_eq!(source.water_level, 3.0);
        assert_eq!(source.passability[0].movement_class, "ship");
        assert_eq!(source.geothermal_nodes.len(), 1);
        assert_eq!(source.geothermal_nodes[0].radius, 12.0);
        assert!(source.continuous_metal.is_some());
    }

    #[test]
    fn auto_loader_distinguishes_native_source_hello_and_wrong_artifacts() {
        let canonical = serde_json::to_string(&source()).unwrap();
        let (native, format) =
            MapSource::from_json(&canonical, HelloMapOptions::default()).unwrap();
        assert_eq!(format, MapInputFormat::NativeSource);
        assert_eq!(native.heights.origin_x, 0.0);

        let hello = r#"{
            "map_name":"Hello", "map_size":[16,16],
            "terrain":{"heights":[[0,0],[0,0]],
            "movement_classes":{"bot":[[true,true],[true,true]]}}
        }"#;
        let (_, format) = MapSource::from_json(hello, HelloMapOptions::default()).unwrap();
        assert_eq!(format, MapInputFormat::Hello);

        let sidecar = r#"{"format":"visigoth.native_map","format_version":1}"#;
        assert!(matches!(
            MapSource::from_json(sidecar, HelloMapOptions::default()),
            Err(MapError::UnsupportedJsonFormat(message)) if message.contains(".map_source.json")
        ));
        assert!(matches!(
            MapSource::from_json("{}", HelloMapOptions::default()),
            Err(MapError::UnsupportedJsonFormat(message)) if message.contains("source_version")
        ));
    }

    #[test]
    fn native_acquisition_identity_and_movement_schema_survive_cache() {
        let mut source = source();
        let acquisition = r#"{
            "format":"visigoth.native_map", "format_version":1,
            "assembler_revision":3, "content_hash":"sha256:test",
            "identity":{
                "map_name":"Tiny Strait", "map_checksum":"map-42",
                "game_name":"Beyond All Reason", "game_version":"2026.09",
                "mod_checksum":"mod-17", "engine_version":"Recoil 2026.1",
                "exporter_revision":2
            },
            "map":{"extractor_radius":8.1},
            "options":{"map":{"weather":"clear"},"mod":{"startmetal":"1000"}},
            "movedefs":[
                {"name":"BOAT3","path_type":7,"depth":12,"is_submarine":false},
                {"name":"BOT3","path_type":2,"max_slope":0.55,"representative_unit":"armpw"}
            ]
        }"#;
        source.apply_acquisition_json(acquisition).unwrap();
        assert!(source.provenance.is_complete());
        assert_eq!(source.provenance.map_options["weather"], "clear");
        assert_eq!(source.provenance.mod_options["startmetal"], "1000");
        assert_eq!(source.provenance.movement_schema.len(), 2);

        // The current assembler embeds this same provenance object directly in
        // canonical map_source.json, in addition to writing the sidecar.
        let embedded = serde_json::to_string(&source).unwrap();
        let (source, format) = MapSource::from_json(&embedded, HelloMapOptions::default()).unwrap();
        assert_eq!(format, MapInputFormat::NativeSource);
        assert!(source.provenance.is_complete());

        let compiled = CompiledMap::compile(source).unwrap();
        let loaded = CompiledMap::from_cache_bytes(&compiled.to_cache_bytes()).unwrap();
        assert_eq!(loaded.provenance, compiled.provenance);
        let identity = loaded.provenance.identity.unwrap();
        assert_eq!(identity.map_checksum, "map-42");
        assert_eq!(identity.game_version, "2026.09");
        assert_eq!(identity.mod_checksum, "mod-17");
    }

    #[test]
    fn acquisition_mismatch_is_rejected_before_cache_creation() {
        let mut source = source();
        let wrong = r#"{
            "format":"visigoth.native_map", "format_version":1,
            "content_hash":"sha256:other",
            "identity":{
                "map_name":"Tiny Strait", "map_checksum":"m", "game_name":"g",
                "game_version":"v", "mod_checksum":"c", "engine_version":"e",
                "exporter_revision":1
            },
            "map":{"extractor_radius":8.1}, "movedefs":[]
        }"#;
        assert!(matches!(
            source.apply_acquisition_json(wrong),
            Err(MapError::Acquisition(message)) if message.contains("content hash mismatch")
        ));
    }

    #[test]
    fn test_randomized_cache_queries_match_recoil_oracle() {
        let compiled = CompiledMap::compile(source()).unwrap();
        assert_eq!(compiled.wind_min, 5.0);
        assert_eq!(compiled.wind_max, 20.0);
        assert_eq!(compiled.average_wind(), 12.5);
        assert_eq!(compiled.tidal, 15.0);

        let mut lcg = 123456789_u64;
        let mut next_f32 = |min: f32, max: f32| -> f32 {
            lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1);
            let unit = ((lcg >> 33) as f32) / (u32::MAX as f32);
            min + unit * (max - min)
        };

        for _ in 0..200 {
            let x = next_f32(-4.0, 28.0);
            let z = next_f32(-4.0, 28.0);
            let in_bounds = compiled.contains(x, z);

            let height = compiled.ground_height(x, z);
            assert!(height.is_finite());
            assert!((-6.01..=4.01).contains(&height));

            let expected_depth = (compiled.water_level - height).max(0.0);
            assert!((compiled.water_depth(x, z) - expected_depth).abs() < 1e-4);
            assert_eq!(compiled.is_underwater(x, z), height < compiled.water_level);

            for class in ["BOT3", "BOAT3", "NONEXISTENT"] {
                let passable = compiled.is_passable(class, x, z);
                if !in_bounds || class == "NONEXISTENT" {
                    assert!(!passable);
                } else {
                    let layer = compiled.movement_layer(class).unwrap();
                    let col = (x / layer.cell_size_x) as u32;
                    let row = (z / layer.cell_size_z) as u32;
                    let idx = (row * layer.cols + col) as usize;
                    let bit = layer.bits[idx / 64] & (1_u64 << (idx % 64)) != 0;
                    assert_eq!(passable, bit);
                }
            }

            // Test geothermal query
            let geo_idx = compiled.geothermal_at(x, z, 0.0);
            let expected_geo = compiled.geothermal_nodes.iter().position(|vent| {
                (vent.x - x).powi(2) + (vent.z - z).powi(2) <= vent.radius * vent.radius
            });
            assert_eq!(geo_idx, expected_geo);
        }
    }

    #[test]
    fn test_extractor_income_matches_recoil_formula() {
        let compiled = CompiledMap::compile(source()).unwrap();
        let metal_field = compiled.metal_field.as_ref().unwrap();

        // Exact Recoil oracle: sum covered metal squares within extractor_radius, scaled
        let step_x = metal_field.density.step_x;
        let step_z = metal_field.density.step_z;
        let radius = metal_field.extractor_radius;
        let scale = metal_field.extraction_scale;
        let radius_sq = radius * radius;

        for test_x in [4.0, 12.0, 20.0] {
            for test_z in [4.0, 12.0, 20.0] {
                let mut oracle_sum = 0.0_f32;
                for row in 0..metal_field.density.rows {
                    let cz = metal_field.density.origin_z + row as f32 * step_z;
                    for col in 0..metal_field.density.cols {
                        let cx = metal_field.density.origin_x + col as f32 * step_x;
                        let dist_sq = (cx - test_x).powi(2) + (cz - test_z).powi(2);
                        if dist_sq <= radius_sq {
                            let idx = (row * metal_field.density.cols + col) as usize;
                            oracle_sum += metal_field.density.decode(idx);
                        }
                    }
                }
                let oracle_yield = oracle_sum * scale;
                let sampled_yield = metal_field.extractor_yield_at(test_x, test_z);
                assert!(
                    (sampled_yield - oracle_yield).abs() < 1e-4,
                    "yield at ({test_x}, {test_z}): expected {oracle_yield}, got {sampled_yield}"
                );
            }
        }
    }
}
