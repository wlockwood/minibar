use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

pub const SIM_HZ: f32 = 30.0;
pub const DT: f32 = 1.0 / SIM_HZ;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub z: f32,
}

impl Vec2 {
    pub const fn new(x: f32, z: f32) -> Self {
        Self { x, z }
    }
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }
    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.z * self.z
    }
    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
    }
    pub fn distance_squared(self, other: Self) -> f32 {
        (self - other).length_squared()
    }
    pub fn normalized(self) -> Self {
        let length = self.length();
        if length > 1e-6 {
            self * (1.0 / length)
        } else {
            Self::default()
        }
    }
    pub fn from_heading(heading: f32) -> Self {
        Self::new(heading.cos(), heading.sin())
    }
    pub fn heading(self) -> f32 {
        self.z.atan2(self.x)
    }
    pub fn rotate(self, angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self::new(self.x * cos - self.z * sin, self.x * sin + self.z * cos)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.z + rhs.z)
    }
}
impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.z - rhs.z)
    }
}
impl std::ops::Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.z * rhs)
    }
}

pub fn angle_delta(from: f32, to: f32) -> f32 {
    (to - from + PI).rem_euclid(TAU) - PI
}

pub fn turn_toward(current: f32, target: f32, maximum: f32) -> f32 {
    current + angle_delta(current, target).clamp(-maximum, maximum)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Terrain {
    pub cols: usize,
    pub rows: usize,
    pub cell_size: f32,
    /// Absolute world-space water plane retained from the map cache.
    pub water_level: f32,
    /// Exact maximum of `heights`, cached for a constant-time clear-sky LOS
    /// rejection before sampling along the ray.
    #[serde(default)]
    pub maximum_height: f32,
    pub heights: Vec<f32>,
    pub passable: Vec<bool>,
}

impl Terrain {
    pub fn flat(width: f32, depth: f32, cell_size: f32) -> Self {
        let cols = (width / cell_size).ceil().max(1.0) as usize;
        let rows = (depth / cell_size).ceil().max(1.0) as usize;
        Self {
            cols,
            rows,
            cell_size,
            water_level: 0.0,
            maximum_height: 0.0,
            heights: vec![0.0; cols * rows],
            passable: vec![true; cols * rows],
        }
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.maximum_height = self
            .heights
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
    }

    fn index(&self, position: Vec2) -> Option<usize> {
        if position.x < 0.0 || position.z < 0.0 {
            return None;
        }
        let col = (position.x / self.cell_size) as usize;
        let row = (position.z / self.cell_size) as usize;
        (col < self.cols && row < self.rows).then_some(row * self.cols + col)
    }

    pub fn is_passable(&self, position: Vec2) -> bool {
        self.index(position)
            .is_some_and(|index| self.passable[index])
    }

    /// Tests the unit's circular ground footprint rather than only its centre.
    /// A footprint extending outside the map or intersecting any impassable
    /// raster cell is rejected. This conservative cell test is deliberately
    /// kept allocation-free because movement calls it every simulation tick.
    pub fn is_passable_with_radius(&self, position: Vec2, radius: f32) -> bool {
        self.is_passable_with_radius_and_mask(position, radius, &self.passable)
    }

    /// Tests circular footprint passability against an arbitrary raster mask
    /// conforming to this terrain's grid dimensions.
    pub fn is_passable_with_radius_and_mask(
        &self,
        position: Vec2,
        radius: f32,
        passable_mask: &[bool],
    ) -> bool {
        let radius = radius.max(0.0);
        if passable_mask.len() < self.cols * self.rows {
            return false;
        }
        if radius <= 0.0 {
            let col = (position.x / self.cell_size).floor() as usize;
            let row = (position.z / self.cell_size).floor() as usize;
            return col < self.cols && row < self.rows && passable_mask[row * self.cols + col];
        }
        let width = self.cols as f32 * self.cell_size;
        let depth = self.rows as f32 * self.cell_size;
        if position.x - radius < 0.0
            || position.z - radius < 0.0
            || position.x + radius > width
            || position.z + radius > depth
        {
            return false;
        }

        let min_col = ((position.x - radius) / self.cell_size).floor() as usize;
        let min_row = ((position.z - radius) / self.cell_size).floor() as usize;
        let max_col = (((position.x + radius) / self.cell_size).ceil() as usize)
            .saturating_sub(1)
            .min(self.cols - 1);
        let max_row = (((position.z + radius) / self.cell_size).ceil() as usize)
            .saturating_sub(1)
            .min(self.rows - 1);
        let radius_squared = radius * radius;
        for row in min_row..=max_row {
            let cell_min_z = row as f32 * self.cell_size;
            let cell_max_z = cell_min_z + self.cell_size;
            let nearest_z = position.z.clamp(cell_min_z, cell_max_z);
            for col in min_col..=max_col {
                if passable_mask[row * self.cols + col] {
                    continue;
                }
                let cell_min_x = col as f32 * self.cell_size;
                let cell_max_x = cell_min_x + self.cell_size;
                let nearest_x = position.x.clamp(cell_min_x, cell_max_x);
                let dx = position.x - nearest_x;
                let dz = position.z - nearest_z;
                if dx * dx + dz * dz <= radius_squared {
                    return false;
                }
            }
        }
        true
    }

    pub fn height_at(&self, position: Vec2) -> f32 {
        self.index(position)
            .map_or(0.0, |index| self.heights[index])
    }

    pub fn has_line_of_sight(&self, from: Vec2, to: Vec2, clearance: f32) -> bool {
        self.has_line_of_sight_3d(
            from,
            self.height_at(from) + clearance,
            to,
            self.height_at(to) + clearance,
            0.0,
        )
    }

    /// Tests a straight projectile segment against the sampled height field.
    /// Heights are absolute world Y coordinates. `radius` is one global,
    /// deliberately tiny projectile clearance rather than weapon-specific data.
    pub fn has_line_of_sight_3d(
        &self,
        from: Vec2,
        from_y: f32,
        to: Vec2,
        to_y: f32,
        radius: f32,
    ) -> bool {
        if from_y.min(to_y) >= self.maximum_height + radius.max(0.0) {
            return true;
        }
        let delta = to - from;
        if delta.length_squared() <= f32::EPSILON {
            return true;
        }
        let Some(start_index) = self.index(from) else {
            return false;
        };
        let Some(end_index) = self.index(to) else {
            return false;
        };
        let mut col = start_index % self.cols;
        let mut row = start_index / self.cols;
        let end_col = end_index % self.cols;
        let end_row = end_index / self.cols;
        let step_col = if delta.x > 0.0 {
            1_i8
        } else if delta.x < 0.0 {
            -1
        } else {
            0
        };
        let step_row = if delta.z > 0.0 {
            1_i8
        } else if delta.z < 0.0 {
            -1
        } else {
            0
        };
        let t_delta_x = if delta.x.abs() > f32::EPSILON {
            self.cell_size / delta.x.abs()
        } else {
            f32::INFINITY
        };
        let t_delta_z = if delta.z.abs() > f32::EPSILON {
            self.cell_size / delta.z.abs()
        } else {
            f32::INFINITY
        };
        let next_x = if step_col > 0 {
            (col + 1) as f32 * self.cell_size
        } else {
            col as f32 * self.cell_size
        };
        let next_z = if step_row > 0 {
            (row + 1) as f32 * self.cell_size
        } else {
            row as f32 * self.cell_size
        };
        let mut t_max_x = if step_col == 0 {
            f32::INFINITY
        } else {
            ((next_x - from.x) / delta.x).max(0.0)
        };
        let mut t_max_z = if step_row == 0 {
            f32::INFINITY
        } else {
            ((next_z - from.z) / delta.z).max(0.0)
        };
        let mut entry_t = 0.0_f32;
        let projectile_radius = radius.max(0.0);
        let vertical_delta = to_y - from_y;

        // Amanatides-Woo traversal visits each crossed height cell once. With
        // piecewise-constant cell heights, comparing against the lower ray end
        // inside the cell is conservative and avoids half-cell sampling gaps.
        loop {
            let exit_t = t_max_x.min(t_max_z).min(1.0);
            let entry_y = from_y + vertical_delta * entry_t;
            let exit_y = from_y + vertical_delta * exit_t;
            if self.heights[row * self.cols + col] + projectile_radius > entry_y.min(exit_y) + 1e-5
            {
                return false;
            }
            if col == end_col && row == end_row {
                return true;
            }

            entry_t = exit_t;
            let cross_x = t_max_x <= t_max_z;
            let cross_z = t_max_z <= t_max_x;
            if cross_x {
                if step_col > 0 {
                    col += 1;
                } else if step_col < 0 {
                    col = col.saturating_sub(1);
                }
                t_max_x += t_delta_x;
            }
            if cross_z {
                if step_row > 0 {
                    row += 1;
                } else if step_row < 0 {
                    row = row.saturating_sub(1);
                }
                t_max_z += t_delta_z;
            }
            if col >= self.cols || row >= self.rows {
                return false;
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MovementDef {
    pub max_speed: f32,
    pub reverse_speed: f32,
    pub acceleration: f32,
    pub braking: f32,
    /// Degrees per second in catalog JSON; converted when used.
    pub turn_rate: f32,
    pub turn_in_place: bool,
    pub turn_in_place_angle_limit: f32,
    pub radius: f32,
    /// Height of the simplified vertical collision volume.
    pub height: f32,
    /// Height of its centre above the unit base. Missing data uses height / 2.
    pub collision_mid_y: Option<f32>,
    /// Static units such as walls still participate in separation and fire blocking.
    pub blocks_movement: bool,
    pub blocks_fire: bool,
}

impl Default for MovementDef {
    fn default() -> Self {
        Self {
            max_speed: 0.0,
            reverse_speed: 0.0,
            acceleration: 0.0,
            braking: 0.0,
            turn_rate: 360.0,
            turn_in_place: true,
            turn_in_place_angle_limit: 90.0,
            radius: 1.0,
            height: 0.0,
            collision_mid_y: None,
            blocks_movement: true,
            blocks_fire: true,
        }
    }
}

impl MovementDef {
    pub fn collision_height(&self) -> f32 {
        if self.height > 0.0 {
            self.height
        } else {
            self.radius * 2.0
        }
    }

    pub fn collision_mid_y(&self) -> f32 {
        self.collision_mid_y
            .unwrap_or_else(|| self.collision_height() * 0.5)
    }

    pub fn collision_vertical_bounds(&self, ground_y: f32) -> (f32, f32) {
        let half = self.collision_height() * 0.5;
        let mid = ground_y + self.collision_mid_y();
        (mid - half, mid + half)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WeaponDef {
    pub name: String,
    /// Damage per projectile. Sustained DPS is derived, never consumed here.
    pub damage: f32,
    /// Per-armour-class overrides. Classes absent from this map use `damage`.
    #[serde(default, rename = "damage_by_armour", alias = "damage_by_armor")]
    pub damage_by_armor: BTreeMap<String, f32>,
    /// Empty means unrestricted. Otherwise the target must share at least one
    /// exported Recoil category with this weapon mount's onlyTargets set.
    pub only_target_categories: Vec<String>,
    pub range: f32,
    pub reload: f32,
    pub burst: u16,
    pub burst_rate: f32,
    pub projectiles: u16,
    pub projectile_speed: f32,
    /// Downward acceleration for unguided ballistic projectiles, in world
    /// units per second squared. A zero value denotes a deliberately
    /// gravity-free weapon, not a fallback cannon approximation.
    pub projectile_gravity: f32,
    /// An enforced high ballistic trajectory. Weapons whose Recoil definition
    /// exposes the player-selectable trajectory state default to low here;
    /// scenario/tactics code can request its high arc explicitly later.
    pub high_trajectory: bool,
    /// Rest-pose projectile emit point [right, up, forward] from the unit base.
    /// This is derived from the S3O QueryWeapon piece, not a visual muzzle size.
    pub firing_origin: Option<[f32; 3]>,
    pub weapon_type: String,
    pub hitscan: bool,
    /// True for guided projectiles. Guided rounds retain their own position
    /// and heading until impact or expiry instead of using the ordinary
    /// due-tick impact calendar.
    pub tracks: bool,
    /// Maximum guided-projectile yaw rate in degrees per second. Zero keeps
    /// the launch heading, which is useful for old catalogs that only carried
    /// the `tracks` bit.
    pub projectile_turn_rate: f32,
    /// Maximum guided-projectile lifetime in seconds. Zero derives a bounded
    /// lifetime from weapon range and projectile speed.
    pub projectile_lifetime: f32,
    /// Projectile class bitmask accepted by shields. Zero means the round is
    /// not shield-interceptable. This is Recoil's `interceptedByShieldType`.
    pub intercepted_by_shield_type: u32,
    /// Projectile class bitmask accepted by interceptor weapons. Zero means
    /// this weapon's projectiles cannot be targeted by an interceptor.
    pub targetable: u32,
    pub turret: bool,
    /// Degrees per second. None means instant aim, an explicit scenario assumption.
    pub turret_yaw_speed: Option<f32>,
    /// Degrees per second for vertical aim. None means instant pitch.
    pub turret_pitch_speed: Option<f32>,
    pub accuracy: f32,
    pub moving_accuracy: f32,
    pub spray_angle: f32,
    pub target_move_error: f32,
    /// Maximum lead displacement in elmos. Negative means unlimited.
    pub lead_limit: f32,
    /// Blends a noisy 0..2x velocity estimate toward perfect prediction.
    pub predict_boost: f32,
    pub area_of_effect: f32,
    pub edge_effectiveness: f32,
    pub fire_tolerance: f32,
    /// Spread total damage evenly across this many seconds (beam/burn models).
    pub damage_duration: f32,
    pub avoid_friendly: bool,
    pub avoid_ground: bool,
    pub collide_enemy: bool,
    pub collide_non_target: bool,
    pub manual_fire: bool,
    pub paralyzer: bool,
    /// Maximum stun duration contributed by this weapon, in seconds. Recoil
    /// stores this on the damage array as `paralyzeDamageTime`.
    pub paralyze_time: f32,
    /// Resources consumed by each emitted projectile salvo.
    pub energy_cost: f32,
    pub metal_cost: f32,
    /// Stockpiled weapons must first manufacture a round over `stockpile_time`.
    pub stockpile: bool,
    pub stockpile_time: f32,
    /// Recoil shield weapon. Shield weapons never enter the ordinary aiming
    /// and firing loop; their runtime state is charge plus a regen deadline.
    pub is_shield: bool,
    pub shield_radius: f32,
    pub shield_power: f32,
    pub shield_starting_power: f32,
    pub shield_power_regen: f32,
    /// Energy per second while regenerating at `shield_power_regen`.
    pub shield_power_regen_energy: f32,
    pub shield_recharge_delay: f32,
    pub shield_intercept_type: u32,
    /// Exterior shields protect allied units in their radius. Non-exterior
    /// shields protect only their owning unit in the simplified kernel.
    pub exterior_shield: bool,
    /// Retained for fidelity diagnostics. The simulator always applies smart
    /// hostile-only filtering, which is the useful combat behavior here.
    pub smart_shield: bool,
    /// Recoil can physically repel shielded projectiles. The simulator records
    /// the flag but deliberately reduces both repulsers and absorbers to a
    /// deterministic absorption/depletion interaction.
    pub shield_repulser: bool,
    /// Bitmask of targetable projectile classes this weapon intercepts.
    pub interceptor: u32,
    /// Radius around the interceptor containing protected impact points.
    pub coverage_range: f32,
    /// Extra capability names supplied by the exporter for mechanics that
    /// cannot be inferred reliably from the common weapon fields.
    pub required_capabilities: Vec<String>,
}

impl Default for WeaponDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            damage: 0.0,
            damage_by_armor: BTreeMap::new(),
            only_target_categories: Vec::new(),
            range: 0.0,
            reload: 1.0,
            burst: 1,
            burst_rate: 0.0,
            projectiles: 1,
            projectile_speed: 0.0,
            projectile_gravity: 0.0,
            high_trajectory: false,
            firing_origin: None,
            weapon_type: "Cannon".into(),
            hitscan: false,
            tracks: false,
            projectile_turn_rate: 0.0,
            projectile_lifetime: 0.0,
            intercepted_by_shield_type: 0,
            targetable: 0,
            turret: true,
            turret_yaw_speed: None,
            turret_pitch_speed: None,
            accuracy: 0.0,
            moving_accuracy: 0.0,
            spray_angle: 0.0,
            target_move_error: 0.0,
            lead_limit: -1.0,
            predict_boost: 0.0,
            area_of_effect: 0.0,
            edge_effectiveness: 0.0,
            fire_tolerance: 0.08726646,
            damage_duration: 0.0,
            avoid_friendly: true,
            avoid_ground: true,
            collide_enemy: true,
            collide_non_target: true,
            manual_fire: false,
            paralyzer: false,
            paralyze_time: 10.0,
            energy_cost: 0.0,
            metal_cost: 0.0,
            stockpile: false,
            stockpile_time: 0.0,
            is_shield: false,
            shield_radius: 0.0,
            shield_power: 0.0,
            shield_starting_power: 0.0,
            shield_power_regen: 0.0,
            shield_power_regen_energy: 0.0,
            shield_recharge_delay: 0.0,
            shield_intercept_type: 0,
            exterior_shield: false,
            smart_shield: false,
            shield_repulser: false,
            interceptor: 0,
            coverage_range: 0.0,
            required_capabilities: Vec::new(),
        }
    }
}

/// Static placement restrictions copied from a Recoil `UnitDef`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConstraints {
    /// Raw yardmap text in south-facing definition order. Parsing and rotation
    /// stay in the placement subsystem rather than on the tactical hot path.
    pub yardmap: Option<String>,
    pub max_slope: Option<f32>,
    pub min_water_depth: Option<f32>,
    pub max_water_depth: Option<f32>,
    pub level_ground: bool,
    pub floater: bool,
}

/// Resources recovered by reclaiming a live unit or feature to completion.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReclaimDef {
    pub reclaimable: bool,
    pub reclaim_time: f32,
    pub metal: f32,
    pub energy: f32,
}

/// Wreck created when a unit dies. A follow-on heap can be named explicitly;
/// its own definition remains catalog data rather than being synthesized.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorpseDef {
    pub name: String,
    pub reclaim: ReclaimDef,
    pub heap: Option<String>,
}

/// Simplified normal-death blast. It is intentionally an instantaneous AOE:
/// projectile meshes, expanding shockwaves, impulse, and terrain deformation
/// remain outside the simulator's fidelity contract.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeathExplosionDef {
    pub damage: f32,
    pub radius: f32,
    pub edge_effectiveness: f32,
}

/// Scenario defaults exported with a catalog. Maps or scenarios may override
/// these values, but absence in an older catalog deliberately means zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartingResources {
    pub metal: f64,
    pub energy: f64,
}

impl WeaponDef {
    pub fn damage_against(&self, armor_class: &str) -> f32 {
        self.damage_by_armor
            .get(armor_class)
            .copied()
            .unwrap_or(self.damage)
    }

    pub fn paper_dps(&self) -> f32 {
        if self.reload <= 0.0 {
            0.0
        } else {
            self.damage * self.burst as f32 * self.projectiles as f32 / self.reload
        }
    }
}

/// BAR/Recoil flanking defaults. Direction is [world X, world Z] for mode 1,
/// [local right, local forward] for modes 2 and 3. Zero disables flanking.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlankingDef {
    pub mode: u8,
    pub min_damage: f32,
    pub max_damage: f32,
    pub mobility_add: f32,
    pub direction: Vec2,
}

impl Default for FlankingDef {
    fn default() -> Self {
        Self {
            mode: 1,
            min_damage: 1.0,
            max_damage: 2.0,
            mobility_add: 0.01,
            direction: Vec2::new(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategicDef {
    pub build_time: f32,
    pub build_distance: f32,
    pub build_speed: f32,
    pub is_builder: bool,
    pub is_factory: bool,
    pub is_building: bool,
    pub is_wall: bool,
    pub is_mobile: bool,
    pub metal_storage: f32,
    pub energy_storage: f32,
    pub extracts_metal: f32,
    pub extract_range: f32,
    pub metal_make: f32,
    pub energy_make: f32,
    pub metal_upkeep: f32,
    pub energy_upkeep: f32,
    pub wind_generator: f32,
    pub tidal_generator: f32,
    pub need_geo: bool,
    pub footprint: [u16; 2],
    pub movement_class: Option<String>,
    pub build_options: Vec<String>,
    pub can_assist: bool,
    pub can_be_assisted: bool,
    pub can_repeat: bool,
    pub activate_when_built: bool,
    pub onoffable: bool,
    pub build_constraints: BuildConstraints,
    /// Armour class presented to weapon damage tables. Empty means Recoil's
    /// default damage class.
    pub armor_class: String,
    /// Recoil unit-category tokens used by weapon target restrictions.
    pub target_categories: Vec<String>,
    /// Multiplier applied to incoming paralyzer damage. Zero is EMP immunity.
    pub paralyze_multiplier: f32,
    pub flanking: FlankingDef,
    pub death_explosion: Option<DeathExplosionDef>,
    /// Marks a unit whose destruction decides or dominates a game (a BAR
    /// commander). Only scoring reads it; training objectives can reward
    /// killing one and punish losing one. Defaults to false.
    pub key_unit: bool,
    pub reclaim: ReclaimDef,
    pub corpse: Option<CorpseDef>,
    pub sight_distance: Option<f32>,
    pub radar_radius: Option<f32>,
    pub sonar_radius: Option<f32>,
    pub jammer_radius: Option<f32>,
    pub can_cloak: bool,
    pub stealth: bool,
    pub sonar_stealth: bool,
    pub sonar_jammer_radius: Option<f32>,
    /// Recoil emits sight from model midpoint plus this height (default 20).
    pub sight_emit_height: f32,
    /// Missing means use sight_emit_height.
    pub radar_emit_height: Option<f32>,
    /// Missing means 1.5 times sight_distance, matching Recoil's UnitDef default.
    pub air_sight_distance: Option<f32>,
    /// Runtime visibility mode; real aircraft switch this when taking off/landing.
    pub use_air_los: bool,
    /// Missing uses the simulator's building classification; source exports derive
    /// Recoil's immobile-with-yardmap rule explicitly.
    pub leaves_ghost: Option<bool>,
    pub start_cloaked: bool,
    pub cloak_cost: f32,
    /// Missing means use cloak_cost.
    pub cloak_cost_moving: Option<f32>,
    pub min_cloak_distance: f32,
    pub decloak_spherical: bool,
    pub decloak_on_fire: bool,
    pub targeting_facility: bool,
    /// Explicit exporter-declared mechanics required to simulate this unit.
    pub required_capabilities: Vec<String>,
}

impl Default for StrategicDef {
    fn default() -> Self {
        Self {
            build_time: 0.0,
            build_distance: 0.0,
            build_speed: 0.0,
            is_builder: false,
            is_factory: false,
            is_building: false,
            is_wall: false,
            is_mobile: false,
            metal_storage: 0.0,
            energy_storage: 0.0,
            extracts_metal: 0.0,
            extract_range: 0.0,
            metal_make: 0.0,
            energy_make: 0.0,
            metal_upkeep: 0.0,
            energy_upkeep: 0.0,
            wind_generator: 0.0,
            tidal_generator: 0.0,
            need_geo: false,
            footprint: [0, 0],
            movement_class: None,
            build_options: Vec::new(),
            can_assist: false,
            can_be_assisted: false,
            can_repeat: false,
            activate_when_built: true,
            onoffable: false,
            build_constraints: BuildConstraints::default(),
            armor_class: String::new(),
            target_categories: Vec::new(),
            paralyze_multiplier: 1.0,
            flanking: FlankingDef::default(),
            death_explosion: None,
            key_unit: false,
            reclaim: ReclaimDef::default(),
            corpse: None,
            sight_distance: None,
            radar_radius: None,
            sonar_radius: None,
            jammer_radius: None,
            can_cloak: false,
            stealth: false,
            sonar_stealth: false,
            sonar_jammer_radius: None,
            sight_emit_height: 20.0,
            radar_emit_height: None,
            air_sight_distance: None,
            use_air_los: false,
            leaves_ghost: None,
            start_cloaked: false,
            cloak_cost: 0.0,
            cloak_cost_moving: None,
            min_cloak_distance: 0.0,
            decloak_spherical: true,
            decloak_on_fire: true,
            targeting_facility: false,
            required_capabilities: Vec::new(),
        }
    }
}

/// Bounded fixed-wing flight contract. Altitude is terrain-relative; takeoff,
/// landing and terrain-following climb dynamics are outside this kernel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlightDef {
    pub cruise_altitude: f32,
    pub minimum_speed: f32,
    pub attack_run: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UnitDef {
    pub name: String,
    /// BAR strategic icon type used only by optional viewers.
    #[serde(default)]
    pub icon_type: Option<String>,
    pub health: f32,
    pub metal_cost: f32,
    #[serde(default)]
    pub energy_cost: f32,
    #[serde(default)]
    pub strategic: StrategicDef,
    #[serde(default)]
    pub flight: Option<FlightDef>,
    pub movement: MovementDef,
    #[serde(default)]
    pub weapons: Vec<WeaponDef>,
}

impl UnitDef {
    /// Comparable resource value used by the default scorer. Raw costs remain
    /// available to experiments that want a different exchange rate.
    pub fn resource_value(&self) -> f32 {
        self.metal_cost + self.energy_cost / 60.0
    }
}

pub type UnitId = u32;

#[derive(Clone, Debug)]
pub struct Spawn {
    pub side: u8,
    pub definition: UnitDef,
    pub position: Vec2,
    pub heading: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Command {
    pub target: Option<UnitId>,
    pub destination: Option<Vec2>,
    /// 0..1 fraction of forward speed.
    pub throttle: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct WeaponState {
    pub aim_heading: f32,
    pub aim_pitch: f32,
    pub reload_until: f32,
    pub next_shot_at: f32,
    pub shots_left: u16,
    pub shot_sequence: u64,
    pub predict_speed_mod: f32,
    pub next_prediction_update: f32,
    /// The tactical engine automatically keeps one stockpiled round queued.
    pub stockpiled_rounds: u16,
    pub stockpile_ready_at: f32,
    pub stockpile_in_progress: bool,
    /// Current charge for a shield weapon; zero for ordinary weapons.
    pub shield_charge: f32,
    /// Absolute time before shield regeneration may resume after a hit.
    pub shield_regen_after: f32,
}

#[derive(Clone, Debug)]
pub struct UnitState {
    pub id: UnitId,
    pub side: u8,
    /// Shared by every trial; catalog strings and weapon vectors are never
    /// deep-cloned in the hot path.
    pub definition: Arc<UnitDef>,
    pub(crate) definition_index: usize,
    pub position: Vec2,
    pub heading: f32,
    pub(crate) forward: Vec2,
    pub speed: f32,
    /// Height above local terrain; zero for ground units.
    pub altitude: f32,
    pub hp: f32,
    /// Fraction of the collision volume's vertical extent currently present.
    /// Nanoframes use build progress; ordinary units remain at one.
    pub collision_height_scale: f32,
    /// EMP damage is non-lethal and declines independently of hit points.
    pub paralyze_damage: f32,
    pub(crate) flanking_direction: Vec2,
    pub(crate) flanking_mobility: f32,
    pub stunned: bool,
    pub alive: bool,
    pub(crate) weapons: Vec<WeaponState>,
}

impl UnitState {
    pub fn velocity(&self) -> Vec2 {
        self.forward * self.speed
    }
    pub fn reload_remaining(&self, time: f32) -> f32 {
        self.weapons
            .first()
            .map_or(0.0, |weapon| (weapon.reload_until - time).max(0.0))
    }
    pub fn primary_range(&self) -> f32 {
        self.definition
            .weapons
            .first()
            .map_or(0.0, |weapon| weapon.range)
    }
}

#[derive(Clone, Debug)]
pub struct WorldView<'a> {
    pub time: f32,
    pub terrain: &'a Terrain,
    pub units: &'a [UnitState],
}

impl WorldView<'_> {
    pub fn unit(&self, id: UnitId) -> Option<&UnitState> {
        self.units.iter().find(|unit| unit.id == id && unit.alive)
    }
    pub fn nearest_enemy(&self, unit: &UnitState) -> Option<&UnitState> {
        self.units
            .iter()
            .filter(|other| other.alive && other.side != unit.side)
            .min_by(|left, right| {
                unit.position
                    .distance_squared(left.position)
                    .total_cmp(&unit.position.distance_squared(right.position))
            })
    }
}

#[cfg(test)]
mod terrain_tests {
    use super::*;

    #[test]
    fn circular_footprint_cannot_clip_an_impassable_cell() {
        let mut terrain = Terrain::flat(96.0, 96.0, 32.0);
        terrain.passable[1] = false;

        assert!(terrain.is_passable(Vec2::new(28.0, 16.0)));
        assert!(terrain.is_passable_with_radius(Vec2::new(28.0, 16.0), 3.0));
        assert!(!terrain.is_passable_with_radius(Vec2::new(28.0, 16.0), 5.0));
    }

    #[test]
    fn circular_footprint_must_remain_inside_map_bounds() {
        let terrain = Terrain::flat(64.0, 64.0, 32.0);
        assert!(terrain.is_passable_with_radius(Vec2::new(8.0, 8.0), 8.0));
        assert!(!terrain.is_passable_with_radius(Vec2::new(7.9, 8.0), 8.0));
    }

    #[test]
    fn grid_dda_detects_a_single_crossed_ridge_cell() {
        let mut terrain = Terrain::flat(64.0, 32.0, 16.0);
        terrain.heights[2] = 11.0;
        terrain.refresh_derived();
        assert!(!terrain.has_line_of_sight_3d(
            Vec2::new(8.0, 8.0),
            10.0,
            Vec2::new(56.0, 8.0),
            10.0,
            0.0,
        ));
        terrain.heights[2] = 9.0;
        terrain.refresh_derived();
        assert!(terrain.has_line_of_sight_3d(
            Vec2::new(8.0, 8.0),
            10.0,
            Vec2::new(56.0, 8.0),
            10.0,
            0.0,
        ));
    }

    #[test]
    fn grid_dda_advances_both_axes_at_exact_corners() {
        let mut terrain = Terrain::flat(64.0, 64.0, 16.0);
        terrain.heights[5] = 20.0;
        terrain.refresh_derived();
        assert!(!terrain.has_line_of_sight_3d(
            Vec2::new(8.0, 8.0),
            10.0,
            Vec2::new(56.0, 56.0),
            10.0,
            0.0,
        ));
    }
}
