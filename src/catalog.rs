use crate::{
    assets::derive_firing_origin,
    model::{
        BuildConstraints, CorpseDef, DeathExplosionDef, MovementDef, ReclaimDef, StartingResources,
        StrategicDef, UnitDef, WeaponDef,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt, fs,
    path::{Path, PathBuf},
};

pub const CAPABILITY_BUILD_CONSTRAINTS: &str = "build_constraints";
pub const CAPABILITY_YARDMAPS: &str = "yardmaps";
pub const CAPABILITY_ARMOR_DAMAGE: &str = "armor_damage";
pub const CAPABILITY_TARGET_CATEGORIES: &str = "target_categories";
pub const CAPABILITY_WEAPON_ENERGY: &str = "weapon_energy";
pub const CAPABILITY_WEAPON_METAL: &str = "weapon_metal";
pub const CAPABILITY_WEAPON_STOCKPILE: &str = "weapon_stockpile";
pub const CAPABILITY_ACTIVATION: &str = "activation";
pub const CAPABILITY_CORPSES: &str = "corpses";
pub const CAPABILITY_RECLAIM: &str = "reclaim";
pub const CAPABILITY_STARTING_RESOURCES: &str = "starting_resources";
pub const CAPABILITY_PARALYSIS: &str = "paralysis";
pub const CAPABILITY_DEATH_EXPLOSIONS: &str = "death_explosions";
pub const CAPABILITY_HOMING_PROJECTILES: &str = "homing_projectiles";
pub const CAPABILITY_SHIELDS: &str = "shields";
pub const CAPABILITY_INTERCEPTORS: &str = "interceptors";
pub const CAPABILITY_SONAR: &str = "sonar";
pub const CAPABILITY_JAMMER: &str = "jammer";
pub const CAPABILITY_CLOAK: &str = "cloak";

/// Mechanics implemented by the current tactical and strategic kernels.
/// Entry points should validate a catalog against this list before running a
/// unit so exporter-declared special behavior is never silently discarded.
pub const IMPLEMENTED_CAPABILITIES: &[&str] = &[
    CAPABILITY_BUILD_CONSTRAINTS,
    CAPABILITY_YARDMAPS,
    CAPABILITY_ARMOR_DAMAGE,
    CAPABILITY_TARGET_CATEGORIES,
    CAPABILITY_WEAPON_ENERGY,
    CAPABILITY_WEAPON_METAL,
    CAPABILITY_WEAPON_STOCKPILE,
    CAPABILITY_ACTIVATION,
    CAPABILITY_CORPSES,
    CAPABILITY_RECLAIM,
    CAPABILITY_STARTING_RESOURCES,
    CAPABILITY_PARALYSIS,
    CAPABILITY_DEATH_EXPLOSIONS,
    CAPABILITY_HOMING_PROJECTILES,
    CAPABILITY_SHIELDS,
    CAPABILITY_INTERCEPTORS,
    CAPABILITY_SONAR,
    CAPABILITY_JAMMER,
    CAPABILITY_CLOAK,
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogMetadata {
    pub catalog_version: u32,
    pub game_version: String,
    pub starting_resources: StartingResources,
    /// Catalog-wide mechanics declared by the exporter. These are combined
    /// with requirements inferred from actual unit and weapon fields.
    pub required_capabilities: Vec<String>,
}

/// One mechanic a caller must support before running this catalog. Unit and
/// weapon context make rejection actionable for large generated catalogs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    pub capability: String,
    pub unit: Option<String>,
    pub weapon: Option<String>,
}

impl fmt::Display for CapabilityRequirement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.unit, &self.weapon) {
            (Some(unit), Some(weapon)) => {
                write!(formatter, "{} required by {unit}.{weapon}", self.capability)
            }
            (Some(unit), None) => write!(formatter, "{} required by {unit}", self.capability),
            (None, _) => write!(formatter, "{} required by catalog", self.capability),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogDataIssue {
    /// Stable dotted path suitable for test output and batch diagnostics.
    pub path: String,
    pub message: String,
}

impl fmt::Display for CatalogDataIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

#[derive(Clone, Debug, Default)]
pub struct UnitCatalog {
    pub units: BTreeMap<String, UnitDef>,
    pub metadata: CatalogMetadata,
}

impl UnitCatalog {
    pub fn get(&self, name: &str) -> Option<&UnitDef> {
        self.units.get(name)
    }

    /// Returns sorted, de-duplicated capability requirements. Requirements
    /// are inferred conservatively from populated fields and augmented by
    /// exporter declarations, so older catalogs remain loadable while newer
    /// catalogs can name mod-specific mechanics explicitly.
    pub fn required_capabilities(&self) -> Vec<CapabilityRequirement> {
        let mut requirements = BTreeSet::new();
        for capability in &self.metadata.required_capabilities {
            insert_requirement(&mut requirements, capability, None, None);
        }
        if self.metadata.starting_resources != StartingResources::default() {
            insert_requirement(&mut requirements, CAPABILITY_STARTING_RESOURCES, None, None);
        }

        for (name, unit) in &self.units {
            let strategic = &unit.strategic;
            for capability in &strategic.required_capabilities {
                insert_requirement(&mut requirements, capability, Some(name), None);
            }
            let constraints = &strategic.build_constraints;
            if constraints.max_slope.is_some()
                || constraints.min_water_depth.is_some()
                || constraints.max_water_depth.is_some()
                || constraints.level_ground
                || constraints.floater
            {
                insert_requirement(
                    &mut requirements,
                    CAPABILITY_BUILD_CONSTRAINTS,
                    Some(name),
                    None,
                );
            }
            if constraints.yardmap.is_some() {
                insert_requirement(&mut requirements, CAPABILITY_YARDMAPS, Some(name), None);
            }
            if !strategic.armor_class.is_empty() {
                insert_requirement(&mut requirements, CAPABILITY_ARMOR_DAMAGE, Some(name), None);
            }
            if strategic.onoffable || !strategic.activate_when_built {
                insert_requirement(&mut requirements, CAPABILITY_ACTIVATION, Some(name), None);
            }
            if strategic.corpse.is_some() {
                insert_requirement(&mut requirements, CAPABILITY_CORPSES, Some(name), None);
            }
            if strategic.death_explosion.is_some() {
                insert_requirement(
                    &mut requirements,
                    CAPABILITY_DEATH_EXPLOSIONS,
                    Some(name),
                    None,
                );
            }
            if (strategic.paralyze_multiplier - 1.0).abs() > f32::EPSILON {
                insert_requirement(&mut requirements, CAPABILITY_PARALYSIS, Some(name), None);
            }
            if strategic.reclaim.reclaimable
                || strategic
                    .corpse
                    .as_ref()
                    .is_some_and(|corpse| corpse.reclaim.reclaimable)
            {
                insert_requirement(&mut requirements, CAPABILITY_RECLAIM, Some(name), None);
            }
            if strategic.sonar_radius.is_some_and(|r| r > 0.0) {
                insert_requirement(&mut requirements, CAPABILITY_SONAR, Some(name), None);
            }
            if strategic.jammer_radius.is_some_and(|r| r > 0.0) {
                insert_requirement(&mut requirements, CAPABILITY_JAMMER, Some(name), None);
            }
            if strategic.can_cloak {
                insert_requirement(&mut requirements, CAPABILITY_CLOAK, Some(name), None);
            }

            for weapon in &unit.weapons {
                let weapon_name = Some(weapon.name.as_str());
                for capability in &weapon.required_capabilities {
                    insert_requirement(&mut requirements, capability, Some(name), weapon_name);
                }
                if !weapon.damage_by_armor.is_empty() {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_ARMOR_DAMAGE,
                        Some(name),
                        weapon_name,
                    );
                }
                if !weapon.only_target_categories.is_empty() {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_TARGET_CATEGORIES,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.energy_cost > 0.0 {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_WEAPON_ENERGY,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.metal_cost > 0.0 {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_WEAPON_METAL,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.stockpile {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_WEAPON_STOCKPILE,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.paralyzer {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_PARALYSIS,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.tracks && !weapon.hitscan {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_HOMING_PROJECTILES,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.is_shield {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_SHIELDS,
                        Some(name),
                        weapon_name,
                    );
                }
                if weapon.interceptor != 0 || weapon.targetable != 0 {
                    insert_requirement(
                        &mut requirements,
                        CAPABILITY_INTERCEPTORS,
                        Some(name),
                        weapon_name,
                    );
                }
            }
        }
        requirements.into_iter().collect()
    }

    /// Lists every required mechanic absent from `supported`, retaining all
    /// affected unit/weapon locations instead of stopping at the first one.
    pub fn unsupported_capabilities<I, S>(&self, supported: I) -> Vec<CapabilityRequirement>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let supported: BTreeSet<String> = supported
            .into_iter()
            .map(|value| value.as_ref().to_owned())
            .collect();
        self.required_capabilities()
            .into_iter()
            .filter(|requirement| !supported.contains(&requirement.capability))
            .collect()
    }

    /// Fails explicitly when loading this catalog into a simulator that does
    /// not implement every required mechanic.
    pub fn validate_capabilities<I, S>(
        &self,
        supported: I,
    ) -> Result<(), Vec<CapabilityRequirement>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let unsupported = self.unsupported_capabilities(supported);
        if unsupported.is_empty() {
            Ok(())
        } else {
            Err(unsupported)
        }
    }

    /// Validates catalog values without choosing which mechanics the caller
    /// supports. Capability support is intentionally a separate decision.
    pub fn validate_data(&self) -> Result<(), Vec<CatalogDataIssue>> {
        let mut issues = Vec::new();
        validate_nonnegative_f64(
            &mut issues,
            "metadata.starting_resources.metal",
            self.metadata.starting_resources.metal,
        );
        validate_nonnegative_f64(
            &mut issues,
            "metadata.starting_resources.energy",
            self.metadata.starting_resources.energy,
        );
        validate_capability_names(
            &mut issues,
            "metadata.required_capabilities",
            &self.metadata.required_capabilities,
        );
        for (name, unit) in &self.units {
            validate_unit_data(&mut issues, name, unit);
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

fn insert_requirement(
    requirements: &mut BTreeSet<CapabilityRequirement>,
    capability: &str,
    unit: Option<&str>,
    weapon: Option<&str>,
) {
    let capability = capability.trim();
    if !capability.is_empty() {
        requirements.insert(CapabilityRequirement {
            capability: capability.to_owned(),
            unit: unit.map(str::to_owned),
            weapon: weapon.filter(|name| !name.is_empty()).map(str::to_owned),
        });
    }
}

fn validate_unit_data(issues: &mut Vec<CatalogDataIssue>, name: &str, unit: &UnitDef) {
    let prefix = format!("units.{name}");
    let constraints = &unit.strategic.build_constraints;
    if constraints
        .yardmap
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        push_issue(issues, format!("{prefix}.yardmap"), "must not be empty");
    }
    validate_optional_nonnegative(
        issues,
        &format!("{prefix}.max_slope"),
        constraints.max_slope,
    );
    validate_optional_finite(
        issues,
        &format!("{prefix}.min_water_depth"),
        constraints.min_water_depth,
    );
    validate_optional_finite(
        issues,
        &format!("{prefix}.max_water_depth"),
        constraints.max_water_depth,
    );
    if let (Some(minimum), Some(maximum)) =
        (constraints.min_water_depth, constraints.max_water_depth)
        && minimum > maximum
    {
        push_issue(
            issues,
            format!("{prefix}.water_depth"),
            "minimum exceeds maximum",
        );
    }
    validate_capability_names(
        issues,
        &format!("{prefix}.required_capabilities"),
        &unit.strategic.required_capabilities,
    );
    validate_nonnegative_f32(
        issues,
        &format!("{prefix}.paralyze_multiplier"),
        unit.strategic.paralyze_multiplier,
    );
    if let Some(explosion) = &unit.strategic.death_explosion {
        validate_nonnegative_f32(
            issues,
            &format!("{prefix}.death_explosion.damage"),
            explosion.damage,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{prefix}.death_explosion.radius"),
            explosion.radius,
        );
        if !explosion.edge_effectiveness.is_finite()
            || !(0.0..=1.0).contains(&explosion.edge_effectiveness)
        {
            push_issue(
                issues,
                format!("{prefix}.death_explosion.edge_effectiveness"),
                "must be finite and between zero and one",
            );
        }
    }
    validate_capability_names(
        issues,
        &format!("{prefix}.target_categories"),
        &unit.strategic.target_categories,
    );
    validate_reclaim(
        issues,
        &format!("{prefix}.reclaim"),
        &unit.strategic.reclaim,
    );
    if let Some(corpse) = &unit.strategic.corpse {
        if corpse.name.trim().is_empty() {
            push_issue(issues, format!("{prefix}.corpse.name"), "must not be empty");
        }
        validate_reclaim(issues, &format!("{prefix}.corpse.reclaim"), &corpse.reclaim);
    }
    validate_optional_nonnegative(
        issues,
        &format!("{prefix}.sight_distance"),
        unit.strategic.sight_distance,
    );
    validate_optional_nonnegative(
        issues,
        &format!("{prefix}.radar_radius"),
        unit.strategic.radar_radius,
    );

    for (index, weapon) in unit.weapons.iter().enumerate() {
        let weapon_prefix = format!("{prefix}.weapons[{index}]");
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.energy_cost"),
            weapon.energy_cost,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.metal_cost"),
            weapon.metal_cost,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.stockpile_time"),
            weapon.stockpile_time,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.paralyze_time"),
            weapon.paralyze_time,
        );
        validate_optional_nonnegative(
            issues,
            &format!("{weapon_prefix}.turret_yaw_speed"),
            weapon.turret_yaw_speed,
        );
        validate_optional_nonnegative(
            issues,
            &format!("{weapon_prefix}.turret_pitch_speed"),
            weapon.turret_pitch_speed,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.projectile_turn_rate"),
            weapon.projectile_turn_rate,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.projectile_lifetime"),
            weapon.projectile_lifetime,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.projectile_gravity"),
            weapon.projectile_gravity,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.shield_radius"),
            weapon.shield_radius,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.shield_power"),
            weapon.shield_power,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.shield_starting_power"),
            weapon.shield_starting_power,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.shield_power_regen"),
            weapon.shield_power_regen,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.shield_power_regen_energy"),
            weapon.shield_power_regen_energy,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.shield_recharge_delay"),
            weapon.shield_recharge_delay,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{weapon_prefix}.coverage_range"),
            weapon.coverage_range,
        );
        if weapon.stockpile && weapon.stockpile_time <= 0.0 {
            push_issue(
                issues,
                format!("{weapon_prefix}.stockpile_time"),
                "must be positive for a stockpiled weapon",
            );
        }
        if weapon.is_shield
            && (weapon.shield_radius <= 0.0
                || weapon.shield_power <= 0.0
                || weapon.shield_intercept_type == 0)
        {
            push_issue(
                issues,
                format!("{weapon_prefix}.shield"),
                "requires positive radius, power, and shield_intercept_type",
            );
        }
        if weapon.shield_starting_power > weapon.shield_power {
            push_issue(
                issues,
                format!("{weapon_prefix}.shield_starting_power"),
                "must not exceed shield_power",
            );
        }
        if weapon.interceptor != 0 && weapon.coverage_range <= 0.0 {
            push_issue(
                issues,
                format!("{weapon_prefix}.coverage_range"),
                "must be positive for an interceptor weapon",
            );
        }
        for (armor, damage) in &weapon.damage_by_armor {
            if armor.trim().is_empty() {
                push_issue(
                    issues,
                    format!("{weapon_prefix}.damage_by_armour"),
                    "contains an empty armour class",
                );
            }
            validate_nonnegative_f32(
                issues,
                &format!("{weapon_prefix}.damage_by_armour.{armor}"),
                *damage,
            );
        }
        validate_capability_names(
            issues,
            &format!("{weapon_prefix}.required_capabilities"),
            &weapon.required_capabilities,
        );
        validate_capability_names(
            issues,
            &format!("{weapon_prefix}.only_target_categories"),
            &weapon.only_target_categories,
        );
    }
}

fn validate_reclaim(issues: &mut Vec<CatalogDataIssue>, prefix: &str, reclaim: &ReclaimDef) {
    validate_nonnegative_f32(issues, &format!("{prefix}.time"), reclaim.reclaim_time);
    validate_nonnegative_f32(issues, &format!("{prefix}.metal"), reclaim.metal);
    validate_nonnegative_f32(issues, &format!("{prefix}.energy"), reclaim.energy);
}

fn validate_capability_names(
    issues: &mut Vec<CatalogDataIssue>,
    prefix: &str,
    capabilities: &[String],
) {
    for (index, capability) in capabilities.iter().enumerate() {
        if capability.trim().is_empty() {
            push_issue(
                issues,
                format!("{prefix}[{index}]"),
                "capability name must not be empty",
            );
        }
    }
}

fn validate_optional_nonnegative(
    issues: &mut Vec<CatalogDataIssue>,
    path: &str,
    value: Option<f32>,
) {
    if let Some(value) = value {
        validate_nonnegative_f32(issues, path, value);
    }
}

fn validate_optional_finite(issues: &mut Vec<CatalogDataIssue>, path: &str, value: Option<f32>) {
    if value.is_some_and(|value| !value.is_finite()) {
        push_issue(issues, path.to_owned(), "must be finite");
    }
}

fn validate_nonnegative_f32(issues: &mut Vec<CatalogDataIssue>, path: &str, value: f32) {
    if !value.is_finite() || value < 0.0 {
        push_issue(issues, path.to_owned(), "must be finite and non-negative");
    }
}

fn validate_nonnegative_f64(issues: &mut Vec<CatalogDataIssue>, path: &str, value: f64) {
    if !value.is_finite() || value < 0.0 {
        push_issue(issues, path.to_owned(), "must be finite and non-negative");
    }
}

fn push_issue(issues: &mut Vec<CatalogDataIssue>, path: String, message: &str) {
    issues.push(CatalogDataIssue {
        path,
        message: message.to_owned(),
    });
}

#[derive(Debug, Deserialize)]
struct Document {
    #[serde(default)]
    catalog_version: u32,
    #[serde(default)]
    game_version: String,
    #[serde(default)]
    starting_resources: StartingResources,
    #[serde(default)]
    starting_metal: Option<f64>,
    #[serde(default)]
    starting_energy: Option<f64>,
    #[serde(default, alias = "capabilities")]
    required_capabilities: Vec<String>,
    unit_catalog: HashMap<String, CatalogUnit>,
}

#[derive(Debug, Deserialize)]
struct CatalogUnit {
    #[serde(default)]
    icon_type: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    script_name: Option<String>,
    #[serde(default)]
    health: f32,
    #[serde(default)]
    metal_cost: f32,
    #[serde(default)]
    energy_cost: f32,
    #[serde(default)]
    build_time: f32,
    #[serde(default)]
    build_distance: f32,
    #[serde(default)]
    build_speed: f32,
    #[serde(default)]
    is_builder: bool,
    #[serde(default)]
    is_factory: bool,
    #[serde(default)]
    is_building: bool,
    #[serde(default)]
    is_mobile: bool,
    #[serde(default)]
    metal_storage: f32,
    #[serde(default)]
    energy_storage: f32,
    #[serde(default)]
    extracts_metal: f32,
    #[serde(default)]
    extract_range: f32,
    #[serde(default)]
    metal_make: f32,
    #[serde(default)]
    energy_make: f32,
    #[serde(default)]
    metal_upkeep: f32,
    #[serde(default)]
    energy_upkeep: f32,
    #[serde(default)]
    wind_generator: f32,
    #[serde(default)]
    tidal_generator: f32,
    #[serde(default)]
    need_geo: bool,
    #[serde(default)]
    footprint: [u16; 2],
    #[serde(default)]
    movement_class: Option<String>,
    #[serde(default)]
    build_options: Vec<String>,
    #[serde(default)]
    can_assist: bool,
    #[serde(default)]
    can_be_assisted: bool,
    #[serde(default)]
    can_repeat: bool,
    #[serde(default = "default_true")]
    activate_when_built: bool,
    #[serde(default)]
    onoffable: bool,
    #[serde(default)]
    yardmap: Option<String>,
    #[serde(default)]
    max_slope: Option<f32>,
    #[serde(default)]
    min_water_depth: Option<f32>,
    #[serde(default)]
    max_water_depth: Option<f32>,
    #[serde(default)]
    level_ground: bool,
    #[serde(default)]
    floater: bool,
    #[serde(default, alias = "armor_class")]
    armour_class: String,
    #[serde(default)]
    target_categories: Vec<String>,
    #[serde(default = "default_one")]
    paralyze_multiplier: f32,
    #[serde(default)]
    death_explosion_radius: f32,
    #[serde(default)]
    death_explosion_damage: f32,
    #[serde(default)]
    reclaimable: bool,
    #[serde(default)]
    reclaim_time: f32,
    #[serde(default)]
    reclaim_metal: f32,
    #[serde(default)]
    reclaim_energy: f32,
    #[serde(default)]
    corpse: Option<String>,
    #[serde(default)]
    corpse_reclaimable: bool,
    #[serde(default)]
    corpse_reclaim_time: f32,
    #[serde(default)]
    corpse_metal: f32,
    #[serde(default)]
    corpse_energy: f32,
    #[serde(default)]
    corpse_heap: Option<String>,
    #[serde(default, alias = "sight_radius", alias = "sight_range")]
    sight_distance: Option<f32>,
    #[serde(default, alias = "radar_distance", alias = "radar_range")]
    radar_radius: Option<f32>,
    #[serde(default, alias = "sonar_distance", alias = "sonar_range")]
    sonar_radius: Option<f32>,
    #[serde(
        default,
        alias = "jammer_distance",
        alias = "jammer_range",
        alias = "radar_jammer_radius"
    )]
    jammer_radius: Option<f32>,
    #[serde(default, alias = "cloak")]
    can_cloak: bool,
    #[serde(default)]
    required_capabilities: Vec<String>,
    simulation: Option<SimulationProfile>,
}

#[derive(Debug, Deserialize)]
struct SimulationProfile {
    #[serde(flatten)]
    movement: MovementDef,
    #[serde(default)]
    weapons: Vec<WeaponDef>,
}

pub fn load_unit(path: &Path, name: &str) -> Result<UnitDef, String> {
    let document = read_document(path)?;
    let entry = find_entry(&document, path, name)?;
    unit_from_entry(entry, name)
}

/// Loads the complete versioned catalog for economy/factory simulations.
/// Runtime entities should retain names or compact caller-assigned IDs and
/// share this immutable catalog between parallel trials.
pub fn load_catalog(path: &Path) -> Result<UnitCatalog, String> {
    let document = read_document(path)?;
    let mut units = BTreeMap::new();
    for (name, entry) in &document.unit_catalog {
        units.insert(name.clone(), unit_from_entry(entry, name)?);
    }
    let mut starting_resources = document.starting_resources;
    if let Some(metal) = document.starting_metal {
        starting_resources.metal = metal;
    }
    if let Some(energy) = document.starting_energy {
        starting_resources.energy = energy;
    }
    let catalog = UnitCatalog {
        units,
        metadata: CatalogMetadata {
            catalog_version: document.catalog_version,
            game_version: document.game_version,
            starting_resources,
            required_capabilities: document.required_capabilities,
        },
    };
    catalog.validate_data().map_err(format_data_issues)?;
    Ok(catalog)
}

/// Loads and immediately checks the catalog against the caller's implemented
/// mechanics. Use this at experiment boundaries to prevent unsupported unit
/// definitions from silently falling through to generic tactical behavior.
pub fn load_catalog_with_capabilities<I, S>(
    path: &Path,
    supported: I,
) -> Result<UnitCatalog, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let catalog = load_catalog(path)?;
    catalog
        .validate_capabilities(supported)
        .map_err(|missing| {
            format!(
                "unsupported catalog capabilities: {}",
                missing
                    .into_iter()
                    .map(|requirement| requirement.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
    Ok(catalog)
}

/// Derives rest-pose firing origins for a unit from model and script assets.
fn apply_assets_to_unit_def(
    unit: &mut UnitDef,
    entry: &CatalogUnit,
    asset_root: &Path,
) -> Result<bool, String> {
    let Some(model_name) = entry.model_name.as_deref() else {
        return Ok(false);
    };
    let Some(script_name) = entry.script_name.as_deref() else {
        return Ok(false);
    };
    let model_path = resolve_vfs_asset(asset_root, "objects3d", model_name)?;
    let mut source_name = script_name.to_owned();
    if source_name.to_ascii_lowercase().ends_with(".cob") {
        source_name.truncate(source_name.len() - 4);
        source_name.push_str(".bos");
    }
    let script_path = resolve_vfs_asset(asset_root, "scripts", &source_name)?;
    let model_data =
        fs::read(&model_path).map_err(|error| format!("{}: {error}", model_path.display()))?;
    let script_source = fs::read_to_string(&script_path)
        .map_err(|error| format!("{}: {error}", script_path.display()))?;
    for (index, weapon) in unit.weapons.iter_mut().enumerate() {
        weapon.firing_origin = Some(
            derive_firing_origin(&model_data, &script_source, index + 1)
                .map_err(|error| format!("{} weapon {}: {error}", unit.name, index + 1))?,
        );
    }
    Ok(true)
}

/// Applies asset-derived firing origins across all units in a loaded catalog.
pub fn apply_assets_to_catalog(
    catalog: &mut UnitCatalog,
    path: &Path,
    asset_root: &Path,
) -> Result<usize, String> {
    let document = read_document(path)?;
    let mut updated = 0;
    for (name, unit) in &mut catalog.units {
        if let Some(entry) = document.unit_catalog.get(name)
            && entry.model_name.is_some()
            && entry.script_name.is_some()
            && !unit.weapons.is_empty()
            && apply_assets_to_unit_def(unit, entry, asset_root)?
        {
            updated += 1;
        }
    }
    Ok(updated)
}

/// Loads a catalog and immediately derives all rest-pose firing origins from
/// the BAR source tree's S3O models and BOS scripts.
pub fn load_catalog_with_assets(path: &Path, asset_root: &Path) -> Result<UnitCatalog, String> {
    let mut catalog = load_catalog(path)?;
    apply_assets_to_catalog(&mut catalog, path, asset_root)?;
    Ok(catalog)
}

/// Serializes the updated catalog back to JSON, persisting derived firing origins
/// and sensor definitions into the unit simulation profiles.
pub fn export_catalog_json(
    catalog: &UnitCatalog,
    out_path: &Path,
    source_catalog_path: &Path,
) -> Result<(), String> {
    let text = fs::read_to_string(source_catalog_path)
        .map_err(|error| format!("{}: {error}", source_catalog_path.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{}: {error}", source_catalog_path.display()))?;
    if let Some(units_obj) = value
        .get_mut("unit_catalog")
        .and_then(|u| u.as_object_mut())
    {
        for (name, unit) in &catalog.units {
            if let Some(unit_val) = units_obj.get_mut(name) {
                if let Some(weapons_arr) = unit_val
                    .get_mut("simulation")
                    .and_then(|sim| sim.get_mut("weapons"))
                    .and_then(|weapons| weapons.as_array_mut())
                {
                    for (i, weapon) in unit.weapons.iter().enumerate() {
                        if i < weapons_arr.len()
                            && let Some(origin) = weapon.firing_origin
                        {
                            weapons_arr[i]["firing_origin"] = serde_json::json!(origin);
                        }
                    }
                }
                if let Some(sight) = unit.strategic.sight_distance {
                    unit_val["sight_distance"] = serde_json::json!(sight);
                }
                if let Some(radar) = unit.strategic.radar_radius {
                    unit_val["radar_radius"] = serde_json::json!(radar);
                }
                if let Some(sonar) = unit.strategic.sonar_radius {
                    unit_val["sonar_radius"] = serde_json::json!(sonar);
                }
                if let Some(jammer) = unit.strategic.jammer_radius {
                    unit_val["jammer_radius"] = serde_json::json!(jammer);
                }
                if unit.strategic.can_cloak {
                    unit_val["can_cloak"] = serde_json::json!(true);
                }
            }
        }
    }
    let formatted = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("failed serializing catalog: {error}"))?;
    fs::write(out_path, formatted).map_err(|error| format!("{}: {error}", out_path.display()))?;
    Ok(())
}

/// Loads a unit and derives each rest-pose firing origin from the BAR source
/// tree's S3O model plus BOS QueryWeapon function. This preprocessing is kept
/// outside the simulation loop and fails explicitly on ambiguous scripts.
pub fn load_unit_with_assets(
    path: &Path,
    name: &str,
    asset_root: &Path,
) -> Result<UnitDef, String> {
    let document = read_document(path)?;
    let entry = find_entry(&document, path, name)?;
    let mut unit = unit_from_entry(entry, name)?;
    apply_assets_to_unit_def(&mut unit, entry, asset_root)?;
    Ok(unit)
}

fn read_document(path: &Path) -> Result<Document, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| {
        format!(
            "{} is not a catalog/hello JSON document: {error}",
            path.display()
        )
    })
}

fn find_entry<'a>(
    document: &'a Document,
    path: &Path,
    name: &str,
) -> Result<&'a CatalogUnit, String> {
    document
        .unit_catalog
        .get(name)
        .ok_or_else(|| format!("unit {name:?} is absent from {}", path.display()))
}

fn unit_from_entry(entry: &CatalogUnit, name: &str) -> Result<UnitDef, String> {
    let profile = entry.simulation.as_ref().ok_or_else(|| format!(
        "unit {name:?} has no detailed simulation profile; regenerate the catalog with the current bridge"
    ))?;
    let unit = UnitDef {
        name: name.to_owned(),
        icon_type: entry.icon_type.clone(),
        health: entry.health,
        metal_cost: entry.metal_cost,
        energy_cost: entry.energy_cost,
        strategic: StrategicDef {
            build_time: entry.build_time,
            build_distance: entry.build_distance,
            build_speed: entry.build_speed,
            is_builder: entry.is_builder,
            is_factory: entry.is_factory,
            is_building: entry.is_building,
            is_wall: entry
                .target_categories
                .iter()
                .any(|c| c.eq_ignore_ascii_case("wall"))
                || name.contains("wall")
                || name.contains("drag"),
            is_mobile: entry.is_mobile,
            metal_storage: entry.metal_storage,
            energy_storage: entry.energy_storage,
            extracts_metal: entry.extracts_metal,
            extract_range: entry.extract_range,
            metal_make: entry.metal_make,
            energy_make: entry.energy_make,
            metal_upkeep: entry.metal_upkeep,
            energy_upkeep: entry.energy_upkeep,
            wind_generator: entry.wind_generator,
            tidal_generator: entry.tidal_generator,
            need_geo: entry.need_geo,
            footprint: entry.footprint,
            movement_class: entry.movement_class.clone(),
            build_options: entry.build_options.clone(),
            can_assist: entry.can_assist,
            can_be_assisted: entry.can_be_assisted,
            can_repeat: entry.can_repeat,
            activate_when_built: entry.activate_when_built,
            onoffable: entry.onoffable,
            build_constraints: BuildConstraints {
                yardmap: entry.yardmap.clone(),
                max_slope: entry.max_slope,
                min_water_depth: entry.min_water_depth,
                max_water_depth: entry.max_water_depth,
                level_ground: entry.level_ground,
                floater: entry.floater,
            },
            armor_class: entry.armour_class.clone(),
            target_categories: entry.target_categories.clone(),
            paralyze_multiplier: entry.paralyze_multiplier,
            death_explosion: (entry.death_explosion_damage > 0.0
                && entry.death_explosion_radius > 0.0)
                .then_some(DeathExplosionDef {
                    damage: entry.death_explosion_damage,
                    radius: entry.death_explosion_radius,
                    edge_effectiveness: 0.0,
                }),
            reclaim: ReclaimDef {
                reclaimable: entry.reclaimable,
                reclaim_time: entry.reclaim_time,
                metal: entry.reclaim_metal,
                energy: entry.reclaim_energy,
            },
            corpse: entry.corpse.as_ref().map(|name| CorpseDef {
                name: name.clone(),
                reclaim: ReclaimDef {
                    reclaimable: entry.corpse_reclaimable,
                    reclaim_time: entry.corpse_reclaim_time,
                    metal: entry.corpse_metal,
                    energy: entry.corpse_energy,
                },
                heap: entry.corpse_heap.clone(),
            }),
            sight_distance: entry.sight_distance,
            radar_radius: entry.radar_radius,
            sonar_radius: entry.sonar_radius,
            jammer_radius: entry.jammer_radius,
            can_cloak: entry.can_cloak,
            required_capabilities: entry.required_capabilities.clone(),
        },
        movement: profile.movement.clone(),
        weapons: profile.weapons.clone(),
    };
    let mut issues = Vec::new();
    validate_unit_data(&mut issues, name, &unit);
    if issues.is_empty() {
        Ok(unit)
    } else {
        Err(format_data_issues(issues))
    }
}

fn format_data_issues(issues: Vec<CatalogDataIssue>) -> String {
    issues
        .into_iter()
        .map(|issue| issue.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn default_true() -> bool {
    true
}

fn default_one() -> f32 {
    1.0
}

fn resolve_vfs_asset(root: &Path, directory: &str, vfs_name: &str) -> Result<PathBuf, String> {
    let normalized = vfs_name.replace('\\', "/");
    let mut components: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if components
        .first()
        .is_some_and(|part| part.eq_ignore_ascii_case(directory))
    {
        components.remove(0);
    }
    let mut current = root.join(directory);
    for component in components {
        let direct = current.join(component);
        if direct.exists() {
            current = direct;
            continue;
        }
        let matching = fs::read_dir(&current)
            .map_err(|error| format!("{}: {error}", current.display()))?
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(component)
            })
            .map(|entry| entry.path())
            .ok_or_else(|| format!("asset {vfs_name:?} not found below {}", root.display()))?;
        current = matching;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Document, UnitDef) {
        let document: Document = serde_json::from_str(text).expect("catalog JSON parses");
        let entry = document.unit_catalog.get("test").expect("test unit exists");
        let unit = unit_from_entry(entry, "test").expect("unit profile converts");
        (document, unit)
    }

    #[test]
    fn legacy_catalogs_receive_safe_defaults() {
        let (_, unit) = parse(
            r#"{
                "unit_catalog": {
                    "test": {
                        "health": 100,
                        "simulation": {"radius": 5, "weapons": [{"damage": 7}]}
                    }
                }
            }"#,
        );

        assert_eq!(unit.weapons[0].damage, 7.0);
        assert!(unit.weapons[0].damage_by_armor.is_empty());
        assert_eq!(unit.weapons[0].energy_cost, 0.0);
        assert!(!unit.weapons[0].stockpile);
        assert!(!unit.weapons[0].paralyzer);
        assert_eq!(unit.weapons[0].paralyze_time, 10.0);
        assert_eq!(
            unit.strategic.build_constraints,
            BuildConstraints::default()
        );
        assert!(unit.strategic.armor_class.is_empty());
        assert_eq!(unit.strategic.reclaim, ReclaimDef::default());
        assert!(unit.strategic.corpse.is_none());
        assert_eq!(unit.strategic.paralyze_multiplier, 1.0);
        assert!(unit.strategic.death_explosion.is_none());
        assert!(!unit.strategic.onoffable);
    }

    #[test]
    fn extended_catalog_fields_survive_conversion() {
        let (document, unit) = parse(
            r#"{
                "catalog_version": 3,
                "game_version": "BAR-test",
                "starting_resources": {"metal": 1000, "energy": 2500},
                "capabilities": ["modded_cloak"],
                "unit_catalog": {
                    "test": {
                        "health": 900,
                        "yardmap": "oooo yyyy",
                        "max_slope": 0.25,
                        "min_water_depth": 4,
                        "max_water_depth": 30,
                        "level_ground": true,
                        "floater": true,
                        "armour_class": "commanders",
                        "target_categories": ["COMMANDER", "SURFACE"],
                        "paralyze_multiplier": 0.25,
                        "death_explosion_radius": 80,
                        "death_explosion_damage": 500,
                        "onoffable": true,
                        "reclaimable": true,
                        "reclaim_time": 12,
                        "reclaim_metal": 75,
                        "reclaim_energy": 20,
                        "corpse": "test_dead",
                        "corpse_reclaimable": true,
                        "corpse_reclaim_time": 18,
                        "corpse_metal": 50,
                        "corpse_energy": 4,
                        "corpse_heap": "test_heap",
                        "required_capabilities": ["custom_unit_rule"],
                        "simulation": {
                            "radius": 12,
                            "weapons": [{
                                "name": "expensive",
                                "damage": 100,
                                "damage_by_armour": {"commanders": 25},
                                "only_target_categories": ["SURFACE"],
                                "energy_cost": 500,
                                "metal_cost": 2,
                                "stockpile": true,
                                "stockpile_time": 45,
                                "paralyzer": true,
                                "paralyze_time": 35,
                                "tracks": true,
                                "projectile_speed": 180,
                                "projectile_gravity": 90,
                                "high_trajectory": true,
                                "projectile_turn_rate": 240,
                                "projectile_lifetime": 6.5,
                                "required_capabilities": ["custom_weapon_rule"]
                            }]
                        }
                    }
                }
            }"#,
        );

        assert_eq!(document.catalog_version, 3);
        assert_eq!(document.game_version, "BAR-test");
        assert_eq!(document.starting_resources.metal, 1000.0);
        assert_eq!(document.required_capabilities, ["modded_cloak"]);
        let strategic = &unit.strategic;
        assert_eq!(
            strategic.build_constraints.yardmap.as_deref(),
            Some("oooo yyyy")
        );
        assert_eq!(strategic.build_constraints.max_slope, Some(0.25));
        assert_eq!(strategic.build_constraints.min_water_depth, Some(4.0));
        assert_eq!(strategic.build_constraints.max_water_depth, Some(30.0));
        assert!(strategic.build_constraints.level_ground);
        assert!(strategic.build_constraints.floater);
        assert_eq!(strategic.armor_class, "commanders");
        assert_eq!(strategic.target_categories, ["COMMANDER", "SURFACE"]);
        assert_eq!(strategic.paralyze_multiplier, 0.25);
        assert_eq!(
            strategic
                .death_explosion
                .as_ref()
                .expect("death explosion retained")
                .damage,
            500.0
        );
        assert!(strategic.onoffable);
        assert_eq!(strategic.reclaim.metal, 75.0);
        let corpse = strategic.corpse.as_ref().expect("corpse retained");
        assert_eq!(corpse.name, "test_dead");
        assert_eq!(corpse.reclaim.reclaim_time, 18.0);
        assert_eq!(corpse.heap.as_deref(), Some("test_heap"));
        let weapon = &unit.weapons[0];
        assert_eq!(weapon.damage_by_armor["commanders"], 25.0);
        assert_eq!(weapon.only_target_categories, ["SURFACE"]);
        assert_eq!(weapon.energy_cost, 500.0);
        assert_eq!(weapon.metal_cost, 2.0);
        assert!(weapon.stockpile);
        assert_eq!(weapon.stockpile_time, 45.0);
        assert!(weapon.paralyzer);
        assert_eq!(weapon.paralyze_time, 35.0);
        assert!(weapon.tracks);
        assert_eq!(weapon.projectile_speed, 180.0);
        assert_eq!(weapon.projectile_gravity, 90.0);
        assert!(weapon.high_trajectory);
        assert_eq!(weapon.projectile_turn_rate, 240.0);
        assert_eq!(weapon.projectile_lifetime, 6.5);
    }

    #[test]
    fn unsupported_capabilities_report_precise_locations() {
        let (document, unit) = parse(
            r#"{
                "starting_metal": 1000,
                "required_capabilities": ["catalog_rule"],
                "unit_catalog": {
                    "test": {
                        "yardmap": "o",
                        "armour_class": "heavy",
                        "onoffable": true,
                        "corpse": "test_dead",
                        "corpse_reclaimable": true,
                        "required_capabilities": ["unit_rule", "unit_rule"],
                        "simulation": {"weapons": [{
                            "name": "gun",
                            "damage_by_armour": {"heavy": 5},
                            "energy_cost": 10,
                            "stockpile": true,
                            "stockpile_time": 1,
                            "required_capabilities": ["weapon_rule"]
                        }]}
                    }
                }
            }"#,
        );
        let catalog = UnitCatalog {
            units: BTreeMap::from([("test".to_owned(), unit)]),
            metadata: CatalogMetadata {
                starting_resources: StartingResources {
                    metal: document.starting_metal.unwrap_or_default(),
                    energy: 0.0,
                },
                required_capabilities: document.required_capabilities,
                ..Default::default()
            },
        };

        let supported = [
            CAPABILITY_YARDMAPS,
            CAPABILITY_ARMOR_DAMAGE,
            CAPABILITY_ACTIVATION,
            CAPABILITY_CORPSES,
            CAPABILITY_RECLAIM,
            CAPABILITY_STARTING_RESOURCES,
            CAPABILITY_WEAPON_ENERGY,
            CAPABILITY_WEAPON_STOCKPILE,
            "catalog_rule",
            "unit_rule",
        ];
        let unsupported = catalog
            .validate_capabilities(supported)
            .expect_err("weapon rule remains unsupported");
        assert_eq!(
            unsupported,
            [CapabilityRequirement {
                capability: "weapon_rule".to_owned(),
                unit: Some("test".to_owned()),
                weapon: Some("gun".to_owned()),
            }]
        );
    }

    #[test]
    fn missing_support_returns_every_affected_context_in_stable_order() {
        let mut first = UnitDef {
            name: "first".into(),
            icon_type: None,
            health: 1.0,
            metal_cost: 0.0,
            energy_cost: 0.0,
            strategic: StrategicDef::default(),
            movement: MovementDef::default(),
            weapons: vec![WeaponDef {
                name: "a".into(),
                energy_cost: 1.0,
                ..Default::default()
            }],
        };
        first.strategic.required_capabilities = vec!["same_rule".into(), "same_rule".into()];
        let mut second = first.clone();
        second.name = "second".into();
        second.weapons[0].name = "b".into();
        let catalog = UnitCatalog {
            units: BTreeMap::from([("first".into(), first), ("second".into(), second)]),
            ..Default::default()
        };

        let missing = catalog.unsupported_capabilities(std::iter::empty::<&str>());
        assert_eq!(missing.len(), 4);
        assert_eq!(missing[0].capability, "same_rule");
        assert_eq!(missing[0].unit.as_deref(), Some("first"));
        assert_eq!(missing[1].unit.as_deref(), Some("second"));
        assert_eq!(missing[2].capability, CAPABILITY_WEAPON_ENERGY);
        assert_eq!(missing[3].weapon.as_deref(), Some("b"));
    }

    #[test]
    fn invalid_extended_values_are_rejected_with_stable_paths() {
        let document: Document = serde_json::from_str(
            r#"{
                "unit_catalog": {
                    "test": {
                        "min_water_depth": 20,
                        "max_water_depth": 10,
                        "yardmap": "   ",
                        "simulation": {"weapons": [{
                            "stockpile": true,
                            "stockpile_time": 0,
                            "projectile_turn_rate": -3,
                            "projectile_lifetime": -4,
                            "damage_by_armour": {"heavy": -1},
                            "required_capabilities": [""]
                        }]}
                    }
                }
            }"#,
        )
        .expect("shape deserializes before semantic validation");
        let error = unit_from_entry(
            document.unit_catalog.get("test").expect("unit exists"),
            "test",
        )
        .expect_err("invalid catalog values fail conversion");

        assert!(error.contains("units.test.yardmap: must not be empty"));
        assert!(error.contains("units.test.water_depth: minimum exceeds maximum"));
        assert!(error.contains("units.test.weapons[0].stockpile_time"));
        assert!(error.contains("units.test.weapons[0].projectile_turn_rate"));
        assert!(error.contains("units.test.weapons[0].projectile_lifetime"));
        assert!(error.contains("units.test.weapons[0].damage_by_armour.heavy"));
        assert!(error.contains("units.test.weapons[0].required_capabilities[0]"));
    }

    #[test]
    fn armor_damage_falls_back_to_default_damage() {
        let weapon = WeaponDef {
            damage: 10.0,
            damage_by_armor: BTreeMap::from([("heavy".into(), 3.0)]),
            ..Default::default()
        };
        assert_eq!(weapon.damage_against("heavy"), 3.0);
        assert_eq!(weapon.damage_against("default"), 10.0);
    }

    #[test]
    fn priority_b_capabilities_are_inferred_with_precise_context() {
        let mut unit = UnitDef {
            name: "emp-platform".into(),
            icon_type: None,
            health: 100.0,
            metal_cost: 10.0,
            energy_cost: 0.0,
            strategic: StrategicDef::default(),
            movement: MovementDef::default(),
            weapons: vec![WeaponDef {
                name: "guided-emp".into(),
                paralyzer: true,
                tracks: true,
                ..Default::default()
            }],
        };
        unit.strategic.paralyze_multiplier = 0.5;
        unit.strategic.death_explosion = Some(DeathExplosionDef {
            damage: 50.0,
            radius: 20.0,
            edge_effectiveness: 0.25,
        });
        let catalog = UnitCatalog {
            units: BTreeMap::from([("emp-platform".into(), unit)]),
            ..Default::default()
        };
        let requirements = catalog.required_capabilities();

        assert!(requirements.iter().any(|requirement| {
            requirement.capability == CAPABILITY_DEATH_EXPLOSIONS
                && requirement.unit.as_deref() == Some("emp-platform")
                && requirement.weapon.is_none()
        }));
        assert!(requirements.iter().any(|requirement| {
            requirement.capability == CAPABILITY_PARALYSIS
                && requirement.weapon.as_deref() == Some("guided-emp")
        }));
        assert!(requirements.iter().any(|requirement| {
            requirement.capability == CAPABILITY_HOMING_PROJECTILES
                && requirement.weapon.as_deref() == Some("guided-emp")
        }));
        let unsupported =
            catalog.unsupported_capabilities(IMPLEMENTED_CAPABILITIES.iter().copied());
        assert!(unsupported.is_empty());
    }
}
