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
pub const CAPABILITY_FLIGHT: &str = "flight";

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
    CAPABILITY_FLIGHT,
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
            if unit.flight.is_some() {
                insert_requirement(&mut requirements, CAPABILITY_FLIGHT, Some(name), None);
            }
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
            if strategic.jammer_radius.is_some_and(|r| r > 0.0)
                || strategic.sonar_jammer_radius.is_some_and(|r| r > 0.0)
            {
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
    if let Some(flight) = &unit.flight {
        validate_nonnegative_f32(
            issues,
            &format!("{prefix}.flight.cruise_altitude"),
            flight.cruise_altitude,
        );
        validate_nonnegative_f32(
            issues,
            &format!("{prefix}.flight.minimum_speed"),
            flight.minimum_speed,
        );
        if flight.cruise_altitude <= 0.0
            || (!flight.hover && flight.minimum_speed <= 0.0)
            || flight.minimum_speed > unit.movement.max_speed
        {
            push_issue(
                issues,
                format!("{prefix}.flight"),
                "flight requires positive altitude and minimum_speed <= max_speed; fixed wings require positive minimum_speed",
            );
        }
        if let Some(radius) = flight.turn_radius {
            if !radius.is_finite() || radius <= 0.0 || flight.hover {
                push_issue(
                    issues,
                    format!("{prefix}.flight.turn_radius"),
                    "must be finite and positive, and only applies to fixed-wing aircraft",
                );
            }
        }
    }
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
    let flanking = &unit.strategic.flanking;
    if flanking.mode > 3 {
        push_issue(
            issues,
            format!("{prefix}.flanking.mode"),
            "must be between 0 and 3",
        );
    }
    for (field, value) in [
        ("min_damage", flanking.min_damage),
        ("max_damage", flanking.max_damage),
        ("mobility_add", flanking.mobility_add),
    ] {
        validate_nonnegative_f32(issues, &format!("{prefix}.flanking.{field}"), value);
    }
    if flanking.min_damage > flanking.max_damage {
        push_issue(
            issues,
            format!("{prefix}.flanking"),
            "minimum damage exceeds maximum",
        );
    }
    for (axis, value) in [("x", flanking.direction.x), ("z", flanking.direction.z)] {
        validate_optional_finite(
            issues,
            &format!("{prefix}.flanking.direction.{axis}"),
            Some(value),
        );
    }
    if flanking.direction.length_squared() > 1.0 + 1e-5 {
        push_issue(
            issues,
            format!("{prefix}.flanking.direction"),
            "length must not exceed one",
        );
    }
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
        validate_corpse(issues, &format!("{prefix}.corpse"), corpse);
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
    for (field, value) in [
        ("sonar_radius", unit.strategic.sonar_radius),
        ("jammer_radius", unit.strategic.jammer_radius),
        ("sonar_jammer_radius", unit.strategic.sonar_jammer_radius),
        ("air_sight_distance", unit.strategic.air_sight_distance),
        ("cloak_cost_moving", unit.strategic.cloak_cost_moving),
    ] {
        validate_optional_nonnegative(issues, &format!("{prefix}.{field}"), value);
    }
    for (field, value) in [
        ("cloak_cost", unit.strategic.cloak_cost),
        ("min_cloak_distance", unit.strategic.min_cloak_distance),
    ] {
        validate_nonnegative_f32(issues, &format!("{prefix}.{field}"), value);
    }
    validate_optional_finite(
        issues,
        &format!("{prefix}.sight_emit_height"),
        Some(unit.strategic.sight_emit_height),
    );
    validate_optional_finite(
        issues,
        &format!("{prefix}.radar_emit_height"),
        unit.strategic.radar_emit_height,
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
        if weapon
            .max_angle_difference
            .is_some_and(|angle| !angle.is_finite() || !(0.0..=360.0).contains(&angle))
        {
            push_issue(
                issues,
                format!("{weapon_prefix}.max_angle_difference"),
                "must be finite and between zero and 360 degrees",
            );
        }
        if weapon.main_direction.is_some_and(|axis| {
            axis.iter().any(|value| !value.is_finite())
                || axis.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON
        }) {
            push_issue(
                issues,
                format!("{weapon_prefix}.main_direction"),
                "must be a finite nonzero direction",
            );
        }
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
        if weapon.stockpile && weapon.stockpile_limit == 0 {
            push_issue(
                issues,
                format!("{weapon_prefix}.stockpile_limit"),
                "must be positive for a stockpiled weapon",
            );
        }
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

fn validate_corpse(issues: &mut Vec<CatalogDataIssue>, prefix: &str, corpse: &CorpseDef) {
    if corpse.name.trim().is_empty() {
        push_issue(issues, format!("{prefix}.name"), "must not be empty");
    }
    for (field, value) in [
        ("health", Some(corpse.health)),
        ("radius", corpse.radius),
        ("height", corpse.height),
    ] {
        if value.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            push_issue(
                issues,
                format!("{prefix}.{field}"),
                "must be finite and positive",
            );
        }
    }
    validate_optional_finite(
        issues,
        &format!("{prefix}.collision_mid_y"),
        corpse.collision_mid_y,
    );
    validate_reclaim(issues, &format!("{prefix}.reclaim"), &corpse.reclaim);
    if let Some(heap) = &corpse.heap_definition {
        if corpse.heap.as_deref().is_some_and(|name| name != heap.name) {
            push_issue(
                issues,
                format!("{prefix}.heap"),
                "must match heap_definition.name",
            );
        }
        validate_corpse(issues, &format!("{prefix}.heap_definition"), heap);
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
    flight: Option<crate::model::FlightDef>,
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
    flanking: crate::model::FlankingDef,
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
    #[serde(default = "default_one")]
    corpse_health: f32,
    #[serde(default)]
    corpse_radius: Option<f32>,
    #[serde(default)]
    corpse_height: Option<f32>,
    #[serde(default)]
    corpse_collision_mid_y: Option<f32>,
    #[serde(default = "default_true")]
    corpse_blocks_movement: bool,
    #[serde(default = "default_true")]
    corpse_blocks_fire: bool,
    #[serde(default)]
    corpse_heap_definition: Option<Box<CorpseDef>>,
    #[serde(
        default,
        alias = "sight_radius",
        alias = "sight_range",
        alias = "sightdistance",
        alias = "sightDistance"
    )]
    sight_distance: Option<f32>,
    #[serde(
        default,
        alias = "radar_distance",
        alias = "radar_range",
        alias = "radardistance",
        alias = "radarDistance"
    )]
    radar_radius: Option<f32>,
    #[serde(
        default,
        alias = "sonar_distance",
        alias = "sonar_range",
        alias = "sonardistance",
        alias = "sonarDistance"
    )]
    sonar_radius: Option<f32>,
    #[serde(
        default,
        alias = "jammer_distance",
        alias = "jammer_range",
        alias = "radar_jammer_radius",
        alias = "radardistancejam",
        alias = "radarDistanceJam"
    )]
    jammer_radius: Option<f32>,
    #[serde(default, alias = "cloak", alias = "cancloak", alias = "canCloak")]
    can_cloak: Option<bool>,
    #[serde(default)]
    stealth: bool,
    #[serde(default, alias = "sonarstealth", alias = "sonarStealth")]
    sonar_stealth: bool,
    #[serde(default, alias = "sonardistancejam", alias = "sonarDistanceJam")]
    sonar_jammer_radius: Option<f32>,
    #[serde(
        default = "default_sight_emit_height",
        alias = "sightemitheight",
        alias = "sightEmitHeight",
        alias = "losemitheight",
        alias = "losEmitHeight"
    )]
    sight_emit_height: f32,
    #[serde(default, alias = "radaremitheight", alias = "radarEmitHeight")]
    radar_emit_height: Option<f32>,
    #[serde(default, alias = "airsightdistance", alias = "airSightDistance")]
    air_sight_distance: Option<f32>,
    #[serde(default, alias = "useairlos", alias = "useAirLos")]
    use_air_los: bool,
    #[serde(default, alias = "leavesghost", alias = "leavesGhost")]
    leaves_ghost: Option<bool>,
    #[serde(default, alias = "initcloaked", alias = "initCloaked")]
    start_cloaked: bool,
    #[serde(default, alias = "cloakcost", alias = "cloakCost")]
    cloak_cost: f32,
    #[serde(default, alias = "cloakcostmoving", alias = "cloakCostMoving")]
    cloak_cost_moving: Option<f32>,
    #[serde(default, alias = "mincloakdistance", alias = "minCloakDistance")]
    min_cloak_distance: f32,
    #[serde(
        default = "default_true",
        alias = "decloakspherical",
        alias = "decloakSpherical"
    )]
    decloak_spherical: bool,
    #[serde(
        default = "default_true",
        alias = "decloakonfire",
        alias = "decloakOnFire"
    )]
    decloak_on_fire: bool,
    #[serde(default, alias = "istargetingupgrade", alias = "isTargetingUpgrade")]
    targeting_facility: bool,
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

fn default_sight_emit_height() -> f32 {
    20.0
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
                // Remove accepted source aliases before writing canonical names,
                // otherwise reloading an enriched catalog sees duplicate fields.
                if let Some(object) = unit_val.as_object_mut() {
                    for alias in [
                        "sightdistance",
                        "sightDistance",
                        "radardistance",
                        "radarDistance",
                        "sonardistance",
                        "sonarDistance",
                        "radardistancejam",
                        "radarDistanceJam",
                        "sight_radius",
                        "sight_range",
                        "radar_distance",
                        "radar_range",
                        "sonar_distance",
                        "sonar_range",
                        "jammer_distance",
                        "jammer_range",
                        "radar_jammer_radius",
                        "cloak",
                        "cancloak",
                        "canCloak",
                        "sonarstealth",
                        "sonarStealth",
                        "sonardistancejam",
                        "sonarDistanceJam",
                        "sightemitheight",
                        "sightEmitHeight",
                        "losemitheight",
                        "losEmitHeight",
                        "radaremitheight",
                        "radarEmitHeight",
                        "airsightdistance",
                        "airSightDistance",
                        "useairlos",
                        "useAirLos",
                        "leavesghost",
                        "leavesGhost",
                        "initcloaked",
                        "initCloaked",
                        "cloakcost",
                        "cloakCost",
                        "cloakcostmoving",
                        "cloakCostMoving",
                        "mincloakdistance",
                        "minCloakDistance",
                        "decloakspherical",
                        "decloakSpherical",
                        "decloakonfire",
                        "decloakOnFire",
                        "istargetingupgrade",
                        "isTargetingUpgrade",
                    ] {
                        object.remove(alias);
                    }
                }
                let sensor_values = serde_json::to_value(&unit.strategic)
                    .map_err(|error| format!("serializing sensors for {name}: {error}"))?;
                // Persist the complete feature chain using the flat source schema.
                let corpse_values =
                    serde_json::to_value(unit.strategic.corpse.clone().unwrap_or_default())
                        .map_err(|error| format!("serializing corpse for {name}: {error}"))?;
                unit_val["corpse"] = unit
                    .strategic
                    .corpse
                    .as_ref()
                    .map(|corpse| serde_json::json!(corpse.name))
                    .unwrap_or(serde_json::Value::Null);
                for field in [
                    "health",
                    "radius",
                    "height",
                    "collision_mid_y",
                    "blocks_movement",
                    "blocks_fire",
                    "heap",
                    "heap_definition",
                ] {
                    unit_val[format!("corpse_{field}")] = corpse_values[field].clone();
                }
                for field in ["reclaimable", "reclaim_time", "metal", "energy"] {
                    unit_val[format!("corpse_{field}")] = corpse_values["reclaim"][field].clone();
                }
                for field in [
                    "sight_distance",
                    "radar_radius",
                    "sonar_radius",
                    "jammer_radius",
                    "can_cloak",
                    "stealth",
                    "sonar_stealth",
                    "sonar_jammer_radius",
                    "sight_emit_height",
                    "radar_emit_height",
                    "air_sight_distance",
                    "use_air_los",
                    "leaves_ghost",
                    "start_cloaked",
                    "cloak_cost",
                    "cloak_cost_moving",
                    "min_cloak_distance",
                    "decloak_spherical",
                    "decloak_on_fire",
                    "targeting_facility",
                ] {
                    unit_val[field] = sensor_values[field].clone();
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
        flight: entry.flight.clone(),
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
            flanking: entry.flanking,
            death_explosion: (entry.death_explosion_damage > 0.0
                && entry.death_explosion_radius > 0.0)
                .then_some(DeathExplosionDef {
                    damage: entry.death_explosion_damage,
                    radius: entry.death_explosion_radius,
                    edge_effectiveness: 0.0,
                }),
            // Set by experiments (see `TrainConfig::key_units`), never by the catalog.
            key_unit: false,
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
                health: entry.corpse_health,
                radius: entry.corpse_radius,
                height: entry.corpse_height,
                collision_mid_y: entry.corpse_collision_mid_y,
                blocks_movement: entry.corpse_blocks_movement,
                blocks_fire: entry.corpse_blocks_fire,
                heap_definition: entry.corpse_heap_definition.clone(),
            }),
            sight_distance: entry.sight_distance,
            radar_radius: entry.radar_radius,
            sonar_radius: entry.sonar_radius,
            jammer_radius: entry.jammer_radius,
            can_cloak: entry.can_cloak.unwrap_or(entry.cloak_cost != 0.0),
            stealth: entry.stealth,
            sonar_stealth: entry.sonar_stealth,
            sonar_jammer_radius: entry.sonar_jammer_radius,
            sight_emit_height: entry.sight_emit_height,
            radar_emit_height: entry.radar_emit_height,
            air_sight_distance: entry.air_sight_distance,
            use_air_los: entry.use_air_los,
            leaves_ghost: entry.leaves_ghost,
            start_cloaked: entry.start_cloaked,
            cloak_cost: entry.cloak_cost,
            cloak_cost_moving: entry.cloak_cost_moving,
            min_cloak_distance: entry.min_cloak_distance,
            decloak_spherical: entry.decloak_spherical,
            decloak_on_fire: entry.decloak_on_fire,
            targeting_facility: entry.targeting_facility,
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

    #[test]
    fn corpse_geometry_chain_and_inert_definition_round_trip() {
        let (_, mut unit) = parse(
            r#"{"unit_catalog":{"test":{"health":100,"metal_cost":70,
            "simulation":{"radius":20,"height":40},"corpse":"test_dead","corpse_health":90,
            "corpse_radius":10,"corpse_height":6,"corpse_collision_mid_y":4,
            "corpse_blocks_movement":true,"corpse_blocks_fire":false,"corpse_metal":25,
            "corpse_heap":"test_heap","corpse_heap_definition":{"name":"test_heap",
            "health":30,"radius":6,"height":2,"blocks_movement":false,"blocks_fire":false,
            "reclaim":{"metal":8}}}}}"#,
        );
        unit.weapons.push(WeaponDef::default());
        unit.strategic.key_unit = true;
        unit.strategic.energy_make = 500.0;
        let corpse = unit.strategic.corpse.as_ref().unwrap();
        let feature = corpse.unit_definition(&unit);
        assert_eq!(feature.health, 90.0);
        assert_eq!(feature.movement.radius, 10.0);
        assert_eq!(feature.movement.collision_vertical_bounds(0.0), (1.0, 7.0));
        assert!(!feature.movement.blocks_fire);
        assert!(feature.weapons.is_empty());
        assert_eq!(feature.resource_value(), 0.0);
        assert_eq!(feature.strategic.energy_make, 0.0);
        assert!(!feature.strategic.key_unit);
        assert_eq!(feature.strategic.reclaim.metal, 25.0);
        assert_eq!(feature.strategic.corpse.as_ref().unwrap().health, 30.0);
        let native = serde_json::to_string(corpse).unwrap();
        assert_eq!(serde_json::from_str::<CorpseDef>(&native).unwrap(), *corpse);
    }

    #[test]
    fn corpse_legacy_defaults_and_invalid_nested_geometry() {
        let corpse: CorpseDef = serde_json::from_str(r#"{"name":"old_dead"}"#).unwrap();
        assert_eq!(corpse.health, 1.0);
        assert!(corpse.blocks_movement && corpse.blocks_fire);
        let source = UnitDef {
            movement: MovementDef {
                radius: 12.0,
                height: 9.0,
                collision_mid_y: Some(7.0),
                ..MovementDef::default()
            },
            ..UnitDef::default()
        };
        let legacy = corpse.unit_definition(&source);
        assert_eq!(legacy.movement.radius, 12.0);
        assert_eq!(legacy.movement.collision_mid_y, Some(7.0));
        let mut invalid = corpse;
        invalid.heap_definition = Some(Box::new(CorpseDef {
            name: "bad_heap".into(),
            health: f32::NAN,
            radius: Some(-2.0),
            height: Some(0.0),
            collision_mid_y: Some(f32::INFINITY),
            ..CorpseDef::default()
        }));
        let mut issues = vec![];
        validate_corpse(&mut issues, "corpse", &invalid);
        assert_eq!(issues.len(), 4);
        assert!(
            issues
                .iter()
                .all(|issue| issue.path.contains("heap_definition"))
        );
    }

    #[test]
    fn corpse_catalog_export_persists_mutated_chain() {
        let directory =
            std::env::temp_dir().join(format!("minibar-corpse-catalog-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.json");
        let out = directory.join("out.json");
        fs::write(
            &source,
            r#"{"unit_catalog":{"test":{"health":100,"corpse":"test_dead","simulation":{}}}}"#,
        )
        .unwrap();
        let mut catalog = load_catalog(&source).unwrap();
        let corpse = catalog
            .units
            .get_mut("test")
            .unwrap()
            .strategic
            .corpse
            .as_mut()
            .unwrap();
        corpse.health = 42.0;
        corpse.radius = Some(13.0);
        corpse.blocks_fire = false;
        corpse.heap = Some("test_heap".into());
        corpse.heap_definition = Some(Box::new(CorpseDef {
            name: "test_heap".into(),
            health: 10.0,
            ..CorpseDef::default()
        }));
        export_catalog_json(&catalog, &out, &source).unwrap();
        let restored = load_catalog(&out).unwrap();
        assert_eq!(
            restored.units["test"].strategic.corpse,
            catalog.units["test"].strategic.corpse
        );
        fs::remove_file(source).unwrap();
        fs::remove_file(out).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn sensor_catalog_fields_load_flattened_and_round_trip_native() {
        let (_, unit) = parse(
            r#"{"unit_catalog":{"test":{"health":100,"simulation":{},
            "stealth":true,"sonar_stealth":true,"sonar_jammer_radius":250,
            "sight_emit_height":32,"radar_emit_height":80,"air_sight_distance":700,
            "use_air_los":true,"leaves_ghost":false,"start_cloaked":true,
            "cloak_cost":15,"cloak_cost_moving":40,"min_cloak_distance":75,
            "decloak_spherical":false,"decloak_on_fire":false,"targeting_facility":true}}}"#,
        );
        let s = &unit.strategic;
        assert!(
            s.stealth
                && s.sonar_stealth
                && s.use_air_los
                && s.start_cloaked
                && s.targeting_facility
        );
        assert_eq!(s.sonar_jammer_radius, Some(250.0));
        assert_eq!(s.sight_emit_height, 32.0);
        assert_eq!(s.radar_emit_height, Some(80.0));
        assert_eq!(s.air_sight_distance, Some(700.0));
        assert_eq!(s.leaves_ghost, Some(false));
        assert_eq!(s.cloak_cost, 15.0);
        assert_eq!(s.cloak_cost_moving, Some(40.0));
        assert_eq!(s.min_cloak_distance, 75.0);
        assert!(!s.decloak_spherical && !s.decloak_on_fire);
        let native = serde_json::to_value(&unit).unwrap();
        let restored: UnitDef = serde_json::from_value(native.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), native);
    }

    #[test]
    fn sensor_catalog_defaults_and_source_aliases_match_recoil() {
        let (_, unit) = parse(r#"{"unit_catalog":{"test":{"health":100,"simulation":{}}}}"#);
        let s = &unit.strategic;
        assert_eq!(s.sight_emit_height, 20.0);
        assert!(
            s.radar_emit_height.is_none()
                && s.air_sight_distance.is_none()
                && s.leaves_ghost.is_none()
        );
        assert!(s.decloak_spherical && s.decloak_on_fire);
        let (_, unit) = parse(
            r#"{"unit_catalog":{"test":{"health":100,"simulation":{},
            "cloakcost":15,"cloakcostmoving":40,"initcloaked":true,
            "sightemitheight":30,"sonarstealth":true,"istargetingupgrade":true}}}"#,
        );
        assert!(
            unit.strategic.can_cloak
                && unit.strategic.start_cloaked
                && unit.strategic.sonar_stealth
        );
        assert!(unit.strategic.targeting_facility);
        assert_eq!(unit.strategic.sight_emit_height, 30.0);
        assert_eq!(unit.strategic.cloak_cost_moving, Some(40.0));
    }

    #[test]
    fn sensor_catalog_rejects_negative_ranges_costs_and_nonfinite_heights() {
        for (field, value) in [
            ("air_sight_distance", -1.0),
            ("sonar_jammer_radius", -2.0),
            ("cloak_cost", -1.0),
            ("cloak_cost_moving", -2.0),
            ("min_cloak_distance", -1.0),
        ] {
            let text = format!(
                r#"{{"unit_catalog":{{"test":{{"health":100,"simulation":{{}},"{field}":{value}}}}}}}"#
            );
            let document: Document = serde_json::from_str(&text).unwrap();
            assert!(
                unit_from_entry(&document.unit_catalog["test"], "test").is_err(),
                "{field}"
            );
        }
        let mut unit = UnitDef::default();
        unit.strategic.sight_emit_height = f32::NAN;
        unit.strategic.radar_emit_height = Some(f32::INFINITY);
        let mut issues = vec![];
        validate_unit_data(&mut issues, "test", &unit);
        assert!(issues.iter().any(|i| i.path.ends_with("sight_emit_height")));
        assert!(issues.iter().any(|i| i.path.ends_with("radar_emit_height")));
    }

    #[test]
    fn sensor_catalog_export_persists_mutations_and_removes_source_aliases() {
        let directory =
            std::env::temp_dir().join(format!("minibar-sensor-catalog-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.json");
        let out = directory.join("out.json");
        fs::write(&source, r#"{"unit_catalog":{"test":{"health":100,"cloakcost":15,"sonarstealth":true,"simulation":{}}}}"#).unwrap();
        let mut catalog = load_catalog(&source).unwrap();
        let unit = catalog.units.get_mut("test").unwrap();
        unit.strategic.sonar_stealth = false;
        unit.strategic.sonar_jammer_radius = Some(275.0);
        unit.strategic.targeting_facility = true;
        unit.strategic.decloak_on_fire = false;
        export_catalog_json(&catalog, &out, &source).unwrap();
        let restored = load_catalog(&out).unwrap();
        let s = &restored.units["test"].strategic;
        assert!(!s.sonar_stealth && !s.decloak_on_fire && s.targeting_facility);
        assert_eq!(s.sonar_jammer_radius, Some(275.0));
        assert_eq!(s.cloak_cost, 15.0);
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
        assert!(value["unit_catalog"]["test"].get("cloakcost").is_none());
        assert!(value["unit_catalog"]["test"].get("sonarstealth").is_none());
        fs::remove_file(source).unwrap();
        fs::remove_file(out).unwrap();
        fs::remove_dir(directory).unwrap();
    }

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
    fn flanking_catalog_defaults_and_explicit_overrides_are_loaded() {
        let (_, unit) = parse(r#"{"unit_catalog":{"test":{"health":100,"simulation":{}}}}"#);
        assert_eq!(
            unit.strategic.flanking,
            crate::model::FlankingDef::default()
        );
        let (_, unit) = parse(
            r#"{"unit_catalog":{"test":{"health":100,"flanking":{"mode":0,"min_damage":0.5,"max_damage":1.5,"mobility_add":0.02,"direction":{"x":1,"z":0}},"simulation":{}}}}"#,
        );
        assert_eq!(unit.strategic.flanking.mode, 0);
        assert_eq!(unit.strategic.flanking.min_damage, 0.5);
        assert_eq!(unit.strategic.flanking.max_damage, 1.5);
        assert_eq!(unit.strategic.flanking.mobility_add, 0.02);
        assert_eq!(
            unit.strategic.flanking.direction,
            crate::model::Vec2::new(1.0, 0.0)
        );
    }

    #[test]
    fn flanking_catalog_rejects_invalid_rules() {
        for flanking in [
            r#"{"mode":4}"#,
            r#"{"mobility_add":-1}"#,
            r#"{"min_damage":2,"max_damage":1}"#,
            r#"{"max_damage":-1}"#,
        ] {
            let text = format!(
                r#"{{"unit_catalog":{{"test":{{"health":100,"flanking":{flanking},"simulation":{{}}}}}}}}"#
            );
            let document: Document = serde_json::from_str(&text).unwrap();
            let result = unit_from_entry(&document.unit_catalog["test"], "test");
            assert!(result.is_err(), "accepted invalid flanking: {flanking}");
        }
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
            flight: None,
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
    fn directional_mount_catalog_rejects_invalid_cones() {
        for (direction, angle, path) in [
            (Some([0.0, 0.0, 0.0]), Some(90.0), "main_direction"),
            (Some([f32::NAN, 0.0, 1.0]), None, "main_direction"),
            (None, Some(-1.0), "max_angle_difference"),
            (None, Some(361.0), "max_angle_difference"),
            (None, Some(f32::INFINITY), "max_angle_difference"),
        ] {
            let unit = UnitDef {
                weapons: vec![WeaponDef {
                    main_direction: direction,
                    max_angle_difference: angle,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let mut issues = Vec::new();
            validate_unit_data(&mut issues, "test", &unit);
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.path == format!("units.test.weapons[0].{path}")),
                "{issues:?}"
            );
        }
        let weapon: WeaponDef = serde_json::from_str("{}").unwrap();
        assert_eq!(weapon.main_direction, None);
        assert_eq!(weapon.max_angle_difference, None);
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
            flight: None,
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
    #[test]
    fn flight_catalog_contract_is_preserved() {
        let entry: CatalogUnit = serde_json::from_value(serde_json::json!({
            "health":100,"metal_cost":10,
            "flight":{"cruise_altitude":120,"minimum_speed":90,"attack_run":true},
            "simulation":{"max_speed":120,"weapons":[]}
        }))
        .unwrap();
        let unit = unit_from_entry(&entry, "bomber").unwrap();
        assert!(serde_json::to_value(unit).unwrap()["flight"].is_object());
    }
}
