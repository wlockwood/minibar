//! Shared BAR sensor rules. Geometry is supplied by the caller so tactical
//! terrain and strategic maps use the same detection semantics.

use crate::model::{UnitDef, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detection {
    Los,
    Radar,
    Sonar,
}

#[derive(Clone, Copy, Debug)]
pub struct SensorUnit {
    pub key: u64,
    pub side: u8,
    pub position: Vec2,
    pub ground_height: f32,
    pub target_height: f32,
    pub active: bool,
    pub enabled: bool,
    pub being_built: bool,
    pub cloaked: bool,
    pub stealth: bool,
    pub sonar_stealth: bool,
    pub in_water: bool,
    pub under_water: bool,
    pub use_air_los: bool,
    pub sight: f32,
    pub air_sight: f32,
    pub radar: f32,
    pub sonar: f32,
    pub jammer: f32,
    pub sonar_jammer: f32,
    pub sight_height: f32,
    pub radar_height: f32,
    pub leaves_ghost: bool,
    pub targeting_facility: bool,
}

impl SensorUnit {
    pub fn from_definition(key: u64, side: u8, position: Vec2, ground: f32, def: &UnitDef) -> Self {
        let d = &def.strategic;
        let sight = d.sight_distance.unwrap_or(0.0);
        // The kernel has ground positions only. Floating definitions sit at
        // water level; air LOS is explicit until actual flight is simulated.
        let base = if d.build_constraints.floater {
            ground.max(0.0)
        } else {
            ground
        };
        Self {
            key,
            side,
            position,
            ground_height: base,
            target_height: base + def.movement.collision_mid_y(),
            active: true,
            enabled: true,
            being_built: false,
            cloaked: false,
            stealth: d.stealth,
            sonar_stealth: d.sonar_stealth,
            in_water: base <= 0.0 && !d.use_air_los,
            // Recoil uses midpoint plus model bounding radius, not collision
            // volume top. The simulator's radius is the horizontal proxy.
            under_water: base + def.movement.collision_mid_y() + def.movement.radius < 0.0
                && !d.use_air_los,
            use_air_los: d.use_air_los,
            sight,
            air_sight: d.air_sight_distance.unwrap_or(sight * 1.5),
            radar: d.radar_radius.unwrap_or(0.0),
            sonar: d.sonar_radius.unwrap_or(0.0),
            jammer: d.jammer_radius.unwrap_or(0.0),
            sonar_jammer: d.sonar_jammer_radius.unwrap_or(0.0),
            sight_height: d.sight_emit_height,
            radar_height: d.radar_emit_height.unwrap_or(d.sight_emit_height),
            leaves_ghost: d.leaves_ghost.unwrap_or(d.is_building),
            targeting_facility: d.targeting_facility,
        }
    }
}

fn covers(range: f32, from: Vec2, to: Vec2) -> bool {
    range.is_finite() && range > 0.0 && from.distance_squared(to) <= range * range
}

/// BAR's separate jammer maps suppress contacts belonging to the jammer's
/// side. A hostile jammer never blinds the observer's own radar globally.
pub fn jammed(target: &SensorUnit, units: &[SensorUnit], sonar: bool) -> bool {
    units.iter().any(|u| {
        u.side == target.side
            && u.active
            && u.enabled
            && !u.being_built
            && covers(
                if sonar { u.sonar_jammer } else { u.jammer },
                u.position,
                target.position,
            )
    })
}

pub fn visual_coverage(
    observer: &SensorUnit,
    target: &SensorUnit,
    terrain_los: &impl Fn(Vec2, f32, Vec2, f32) -> bool,
) -> bool {
    observer.active
        && !observer.being_built
        && if target.use_air_los {
            covers(observer.air_sight, observer.position, target.position)
        } else {
            covers(observer.sight, observer.position, target.position)
                && terrain_los(
                    observer.position,
                    (observer.target_height + observer.sight_height).max(0.0),
                    target.position,
                    target.ground_height + 0.5,
                )
        }
}

/// Recoil `InRadar(unit)` first tries sonar for water contacts, then radar
/// for non-submerged targets. Cloak only changes LOS; stealth is independent.
pub fn detect(
    side: u8,
    target: &SensorUnit,
    units: &[SensorUnit],
    terrain_los: impl Fn(Vec2, f32, Vec2, f32) -> bool,
) -> Option<Detection> {
    let sonar_jammed = target.in_water && jammed(target, units, true);
    let radar_jammed = !target.under_water && jammed(target, units, false);
    let sonar = target.in_water
        && (!target.sonar_stealth || target.being_built)
        && !sonar_jammed
        && units.iter().any(|u| {
            u.side == side
                && u.active
                && u.enabled
                && !u.being_built
                && covers(u.sonar, u.position, target.position)
        });
    let radar = !target.under_water
        && (!target.stealth || target.being_built)
        && !radar_jammed
        && units.iter().any(|u| {
            u.side == side
                && u.active
                && u.enabled
                && !u.being_built
                && covers(u.radar, u.position, target.position)
                && terrain_los(
                    u.position,
                    (u.target_height + u.radar_height).max(0.0),
                    target.position,
                    target.ground_height + 0.5,
                )
        });
    if !target.cloaked
        && (!target.under_water || sonar)
        && units
            .iter()
            .any(|u| u.side == side && visual_coverage(u, target, &terrain_los))
    {
        Some(Detection::Los)
    } else if sonar {
        Some(Detection::Sonar)
    } else if radar {
        Some(Detection::Radar)
    } else {
        None
    }
}

/// Smooth horizontal radar bias, shared by all observers on one side. Unlike
/// grid rounding this continues to drift for stationary targets. Recoil uses
/// a synced random vector/delta over 256 frames; hashing gives repeatable,
/// query-order-independent vectors without consuming combat RNG.
pub fn radar_position(key: u64, frame: u64, position: Vec2, error_size: f32) -> Vec2 {
    if !error_size.is_finite() || error_size <= 0.0 {
        return position;
    }
    fn hash(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9e3779b97f4a7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    let base_angle = (hash(key) >> 40) as f32 * (std::f32::consts::TAU / 16777216.0);
    let vector = |epoch: u64| {
        let noise = (hash(key ^ epoch.wrapping_mul(0xd1b54a32d192ed03)) >> 40) as f32 / 16777216.0;
        let angle = base_angle + (noise - 0.5) * std::f32::consts::PI;
        let radius = 0.25
            + 0.75
                * ((hash(key.wrapping_add(epoch) ^ 0xa0761d6478bd642f) >> 40) as f32 / 16777216.0)
                    .sqrt();
        Vec2::from_heading(angle) * radius
    };
    let epoch = frame / 256;
    let blend = (frame % 256) as f32 / 256.0;
    position + (vector(epoch) * (1.0 - blend) + vector(epoch + 1) * blend) * error_size
}

pub fn radar_error_size(side: u8, units: &[SensorUnit], base: f32) -> f32 {
    let facilities = units
        .iter()
        .filter(|u| {
            u.side == side && u.active && u.enabled && !u.being_built && u.targeting_facility
        })
        .count()
        .min(126);
    base * 0.5_f32.powi(facilities as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(key: u64, side: u8, x: f32) -> SensorUnit {
        let definition: UnitDef = serde_json::from_value(serde_json::json!({
            "name":"sensor","health":100,"metal_cost":0,"movement":{"height":8},
            "strategic":{"sight_distance":100,"radar_radius":500,"sonar_radius":500}
        }))
        .unwrap();
        SensorUnit::from_definition(key, side, Vec2::new(x, 0.0), 20.0, &definition)
    }

    #[test]
    fn cloak_and_stealth_are_independent_and_construction_disables_stealth() {
        let observer = unit(1, 0, 0.0);
        let mut target = unit(2, 1, 50.0);
        target.cloaked = true;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| true),
            Some(Detection::Radar)
        );
        target.stealth = true;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| true), None);
        target.cloaked = false;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| true),
            Some(Detection::Los)
        );
        target.position.x = 200.0;
        target.being_built = true;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| true),
            Some(Detection::Radar)
        );
    }

    #[test]
    fn radar_always_uses_terrain_but_air_visual_coverage_is_circular() {
        let observer = unit(1, 0, 0.0);
        let mut target = unit(2, 1, 200.0);
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| false), None);
        target.use_air_los = true;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| false), None);
        target.position.x = 120.0;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| false),
            Some(Detection::Los)
        );
    }

    #[test]
    fn sonar_is_required_underwater_and_has_its_own_stealth_and_jamming() {
        let mut observer = unit(1, 0, 0.0);
        let mut target = unit(2, 1, 50.0);
        target.in_water = true;
        target.under_water = true;
        observer.sonar = 0.0;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| true), None);
        observer.sonar = 500.0;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| true),
            Some(Detection::Los)
        );
        target.sonar_stealth = true;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| true), None);
        target.sonar_stealth = false;
        target.cloaked = true;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| false),
            Some(Detection::Sonar)
        );
        let mut jammer = unit(3, 1, 50.0);
        jammer.sonar_jammer = 80.0;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            None
        );
    }

    #[test]
    fn jammer_only_protects_its_own_side_and_never_hides_visual_units() {
        let observer = unit(1, 0, 0.0);
        let target = unit(2, 1, 200.0);
        let mut jammer = unit(3, 0, 200.0);
        jammer.jammer = 80.0;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            Some(Detection::Los)
        );
        jammer.sight = 0.0;
        jammer.air_sight = 0.0;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            Some(Detection::Radar)
        );
        jammer.side = 1;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            None
        );
        jammer.enabled = false;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            Some(Detection::Radar)
        );
    }

    #[test]
    fn radar_error_is_continuous_bounded_deterministic_and_not_grid_snapping() {
        let pos = Vec2::new(200.0, 200.0);
        let first = radar_position(7, 0, pos, 96.0);
        assert_ne!(first, pos);
        assert_eq!(first, radar_position(7, 0, pos, 96.0));
        assert_ne!(first, radar_position(8, 0, pos, 96.0));
        for frame in 1..1025 {
            let current = radar_position(7, frame, pos, 96.0);
            assert!(current.distance(pos) <= 96.001);
            assert!(current.distance(radar_position(7, frame - 1, pos, 96.0)) < 1.0);
        }
        assert_eq!(radar_position(7, 5, pos, 0.0), pos);
    }

    #[test]
    fn underwater_uses_midpoint_plus_radius_rather_than_collision_volume_top() {
        let def: UnitDef = serde_json::from_value(serde_json::json!({
            "name":"wide","health":100,"metal_cost":0,"movement":{"height":4,"radius":30}
        }))
        .unwrap();
        let shallow = SensorUnit::from_definition(1, 1, Vec2::default(), -20.0, &def);
        assert!(shallow.in_water);
        assert!(!shallow.under_water);
        assert!(SensorUnit::from_definition(1, 1, Vec2::default(), -40.0, &def).under_water);
    }

    #[test]
    fn surface_contacts_can_fall_back_between_independent_radar_and_sonar() {
        let observer = unit(1, 0, 0.0);
        let mut target = unit(2, 1, 200.0);
        target.in_water = true;
        target.stealth = true;
        let mut jammer = unit(3, 1, 200.0);
        jammer.jammer = 80.0;
        // Radar stealth/jamming do not suppress sonar, even behind terrain.
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| false),
            Some(Detection::Sonar)
        );
        target.stealth = false;
        target.sonar_stealth = true;
        jammer.jammer = 0.0;
        jammer.sonar_jammer = 80.0;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            Some(Detection::Radar)
        );
        target.under_water = true;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            None
        );
        // Nanoframes lose sonar stealth, but never bypass sonar jamming.
        target.being_built = true;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            None
        );
        jammer.enabled = false;
        assert_eq!(
            detect(0, &target, &[observer, jammer], |_, _, _, _| true),
            Some(Detection::Sonar)
        );
    }

    #[test]
    fn electronic_shutdown_preserves_visual_sight_and_construction_emits_nothing() {
        let mut observer = unit(1, 0, 0.0);
        let mut target = unit(2, 1, 50.0);
        // `enabled` combines activation and stun in both runtime adapters.
        observer.enabled = false;
        assert_eq!(
            detect(0, &target, &[observer], |_, _, _, _| true),
            Some(Detection::Los)
        );
        target.cloaked = true;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| true), None);
        target.in_water = true;
        target.under_water = true;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| true), None);
        observer.enabled = true;
        observer.being_built = true;
        target.cloaked = false;
        assert_eq!(detect(0, &target, &[observer], |_, _, _, _| true), None);
    }

    #[test]
    fn targeting_facilities_stack_only_while_completed_and_operational() {
        let mut facility = unit(1, 0, 0.0);
        facility.targeting_facility = true;
        let mut second = facility;
        second.key = 2;
        assert_eq!(radar_error_size(0, &[facility, second], 96.0), 24.0);
        for ineligible in [
            SensorUnit { side: 1, ..second },
            SensorUnit {
                active: false,
                ..second
            },
            SensorUnit {
                enabled: false,
                ..second
            },
            SensorUnit {
                being_built: true,
                ..second
            },
        ] {
            assert_eq!(radar_error_size(0, &[facility, ineligible], 96.0), 48.0);
        }
        assert_eq!(radar_error_size(0, &[], 96.0), 96.0);
    }
}
