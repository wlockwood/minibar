use std::sync::Arc;

use visigoth_simulator::map::{CompiledMap, HelloMapOptions, MapSource};
use visigoth_simulator::model::{UnitDef, Vec2};
use visigoth_simulator::placement::BuildFacing;
use visigoth_simulator::strategy::{ObservationPolicy, ObservationVisibility};
use visigoth_simulator::world::{World, WorldCatalog, WorldConfig};

fn world(target: serde_json::Value) -> World {
    let source = MapSource::from_hello_json(
        r#"{"map_name":"sensors","map_size":[1024,256],"water_level":0,
        "terrain":{"heights":[[0,0],[0,0]],"movement_classes":{"BOT":[[true,true],[true,true]]}},
        "metal_spots":[],"geothermal_spots":[]}"#,
        HelloMapOptions::default(),
    )
    .unwrap();
    let scout: UnitDef = serde_json::from_value(serde_json::json!({
        "name":"scout","health":100,"metal_cost":0,"movement":{"radius":4,"height":8},
        "strategic":{"is_mobile":true,"sight_distance":100,"radar_radius":500}
    }))
    .unwrap();
    let target: UnitDef = serde_json::from_value(target).unwrap();
    World::new(
        Arc::new(CompiledMap::compile(source).unwrap()),
        Arc::new(WorldCatalog::new(vec![scout, target]).unwrap()),
        WorldConfig {
            observation_policy: ObservationPolicy::Filtered {
                line_of_sight_range: 0.0,
                radar_range: 0.0,
                radar_position_resolution: 96.0,
                remember_last_seen: true,
            },
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn stealth_never_produces_radar_but_remains_visually_visible() {
    let mut world = world(
        serde_json::json!({"name":"stealth","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_mobile":true,"stealth":true}}),
    );
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("stealth").unwrap(),
            Vec2::new(240.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    assert!(
        world.observation(0).unwrap().entity(target).is_none(),
        "stealth leaked as a radar blip"
    );
    world.entities.get_mut(scout).unwrap().position = Vec2::new(180.0, 40.0);
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::LineOfSight
    );
}

#[test]
fn initially_cloaked_unit_is_radar_only_even_inside_visual_range() {
    let mut world = world(
        serde_json::json!({"name":"cloaker","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_mobile":true,"can_cloak":true,
        "start_cloaked":true,"min_cloak_distance":10}}),
    );
    world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("cloaker").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.step();
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::Radar
    );
}

#[test]
fn destroyed_building_ghost_survives_radar_coverage_until_visual_recheck() {
    let mut world = world(
        serde_json::json!({"name":"building","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_building":true,"stealth":true}}),
    );
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("building").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.observation(0).unwrap();
    world.entities.get_mut(scout).unwrap().position = Vec2::new(400.0, 40.0);
    world.destroy_entity(target);
    let observation = world.observation(0).unwrap();
    let ghost = observation
        .entity(target)
        .expect("radar cannot disprove a building ghost");
    assert_eq!(ghost.position, Vec2::new(100.0, 40.0));
    assert_eq!(serde_json::to_value(ghost.visibility).unwrap(), "ghost");
    world.entities.get_mut(scout).unwrap().position = Vec2::new(40.0, 40.0);
    assert!(world.observation(0).unwrap().entity(target).is_none());
}

#[test]
fn mobile_identity_survives_continuous_radar_but_is_lost_after_a_gap() {
    let mut world = world(
        serde_json::json!({"name":"mobile","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_mobile":true}}),
    );
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("mobile").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.observation(0).unwrap();
    world.entities.get_mut(target).unwrap().position.x = 240.0;
    let radar = world.observation(0).unwrap();
    assert_eq!(
        radar.entity(target).unwrap().definition.as_deref(),
        Some("mobile")
    );
    assert_eq!(radar.entity(target).unwrap().health_fraction, None);
    world.entities.get_mut(scout).unwrap().position.x = 900.0;
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::Remembered
    );
    world.entities.get_mut(scout).unwrap().position.x = 40.0;
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .definition,
        None
    );
}

#[test]
fn a_live_cloaked_stealth_building_keeps_its_exact_ghost_even_in_visible_terrain() {
    let mut world = world(
        serde_json::json!({"name":"building","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_building":true,"can_cloak":true,"stealth":true}}),
    );
    world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("building").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.observation(0).unwrap();
    world.set_cloak(target, true).unwrap();
    let seen = world.observation(0).unwrap();
    let ghost = seen.entity(target).unwrap();
    assert_eq!(ghost.visibility, ObservationVisibility::Ghost);
    assert_eq!(ghost.position, Vec2::new(100.0, 40.0));
    assert_eq!(ghost.health_fraction, None);
}

#[test]
fn cloak_charges_half_second_checks_and_reveals_on_energy_failure_or_proximity() {
    let mut world = world(
        serde_json::json!({"name":"cloaker","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_mobile":true,"can_cloak":true,
        "cloak_cost":100,"min_cloak_distance":30}}),
    );
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("cloaker").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.teams[1].economy.energy.stored = 100.0;
    world.set_cloak(target, true).unwrap();
    assert!(world.entity_is_cloaked(target));
    assert_eq!(world.teams[1].economy.energy.stored, 50.0);
    world.step_seconds(0.5);
    assert!(world.entity_is_cloaked(target));
    assert_eq!(world.teams[1].economy.energy.stored, 0.0);
    world.step_seconds(0.5);
    assert!(!world.entity_is_cloaked(target));
    world.teams[1].economy.energy.stored = 1000.0;
    world.entities.get_mut(scout).unwrap().position.x = 80.0;
    world.step_seconds(0.5);
    assert!(!world.entity_is_cloaked(target));
    world.entities.get_mut(scout).unwrap().position.x = 40.0;
    world.step_seconds(0.5);
    assert!(
        world.entity_is_cloaked(target),
        "proximity itself adds no firing cooldown"
    );
}

#[test]
fn world_automatic_targeting_cannot_fire_on_unseen_stealth_but_can_fire_when_spotted() {
    let mut world = world(
        serde_json::json!({"name":"stealth","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_mobile":true,"stealth":true}}),
    );
    let mut definitions = world.catalog.definitions().to_vec();
    definitions[0].weapons.push(
        serde_json::from_value(serde_json::json!({
            "damage":10,"range":500,"reload":0.1,"hitscan":true,"turret":true,
            "projectile_speed":10000,"fire_tolerance":3.14
        }))
        .unwrap(),
    );
    world.catalog = Arc::new(WorldCatalog::new(definitions).unwrap());
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("stealth").unwrap(),
            Vec2::new(240.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.enable_combat(1, "BOT", 16.0).unwrap();
    for _ in 0..30 {
        world.step();
    }
    assert_eq!(world.entities.get(target).unwrap().health, 100.0);
    world.entities.get_mut(scout).unwrap().position.x = 180.0;
    for _ in 0..6 {
        world.step();
    }
    assert!(world.entities.get(target).is_none_or(|e| e.health < 100.0));
}

#[test]
fn radar_tracked_building_destruction_is_known_and_does_not_leave_a_dead_ghost() {
    let mut world = world(
        serde_json::json!({"name":"building","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_building":true}}),
    );
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("building").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.observation(0).unwrap();
    world.entities.get_mut(scout).unwrap().position.x = 400.0;
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::Radar
    );
    world.destroy_entity(target);
    assert!(world.observation(0).unwrap().entity(target).is_none());
}

#[test]
fn initial_cloak_request_does_not_bypass_energy_admission() {
    let mut world = world(
        serde_json::json!({"name":"cloaker","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"can_cloak":true,"start_cloaked":true,
        "cloak_cost":100}}),
    );
    world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("cloaker").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.teams[1].economy.energy.stored = 0.0;
    assert!(!world.entity_is_cloaked(target));
    world.step();
    assert!(!world.entity_is_cloaked(target));
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::LineOfSight
    );
}

#[test]
fn features_are_globally_disclosed_by_bar_modrules_even_outside_all_sensors() {
    let mut world = world(
        serde_json::json!({"name":"rock","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8}}),
    );
    let rock = world
        .spawn_active(
            1,
            world.catalog.id("rock").unwrap(),
            Vec2::new(900.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let entity = world.entities.get_mut(rock).unwrap();
    entity.owner = u8::MAX;
    entity.role = visigoth_simulator::world::EntityRole::Feature;
    let seen = world.observation(0).unwrap();
    assert_eq!(
        seen.entity(rock).unwrap().definition.as_deref(),
        Some("rock")
    );
    assert_eq!(seen.entity(rock).unwrap().position, Vec2::new(900.0, 40.0));
}

#[test]
fn repeated_cloak_requests_are_idempotent_and_do_not_charge_extra_energy() {
    let mut world = world(
        serde_json::json!({"name":"cloaker","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"can_cloak":true,"cloak_cost":100}}),
    );
    let target = world
        .spawn_active(
            1,
            world.catalog.id("cloaker").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.set_cloak(target, true).unwrap();
    let stored = world.teams[1].economy.energy.stored;
    for _ in 0..30 {
        world.set_cloak(target, true).unwrap();
    }
    assert_eq!(world.teams[1].economy.energy.stored, stored);
    assert!(world.entity_is_cloaked(target));
}

#[test]
fn building_ghosts_are_retained_and_pruned_independently_of_mobile_memory_switch() {
    let mut world = world(
        serde_json::json!({"name":"building","health":100,"metal_cost":0,
        "movement":{"radius":4,"height":8},"strategic":{"is_building":true,"stealth":true,"can_cloak":true}}),
    );
    world.set_observation_policy(ObservationPolicy::Filtered {
        line_of_sight_range: 0.0,
        radar_range: 0.0,
        radar_position_resolution: 96.0,
        remember_last_seen: false,
    });
    let scout = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("building").unwrap(),
            Vec2::new(100.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.observation(0).unwrap();
    world.set_cloak(target, true).unwrap();
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::Ghost
    );
    world.entities.get_mut(scout).unwrap().position.x = 400.0;
    world.destroy_entity(target);
    assert_eq!(
        world
            .observation(0)
            .unwrap()
            .entity(target)
            .unwrap()
            .visibility,
        ObservationVisibility::Ghost
    );
    world.entities.get_mut(scout).unwrap().position.x = 40.0;
    assert!(world.observation(0).unwrap().entity(target).is_none());
}

#[test]
fn world_can_fire_at_radar_and_sustained_fire_keeps_extending_recloak_delay() {
    let mut world = world(
        serde_json::json!({"name":"target","health":10000,"metal_cost":0,
        "movement":{"radius":4,"height":8}}),
    );
    world.set_observation_policy(ObservationPolicy::Filtered {
        line_of_sight_range: 0.0,
        radar_range: 0.0,
        radar_position_resolution: 0.0,
        remember_last_seen: true,
    });
    let mut definitions = world.catalog.definitions().to_vec();
    definitions[0].strategic.can_cloak = true;
    definitions[0].weapons.push(
        serde_json::from_value(serde_json::json!({
            "damage":1,"range":500,"reload":0.1,"hitscan":true,"turret":true,
            "projectile_speed":10000,"fire_tolerance":3.14
        }))
        .unwrap(),
    );
    world.catalog = Arc::new(WorldCatalog::new(definitions).unwrap());
    let shooter = world
        .spawn_active(
            0,
            world.catalog.id("scout").unwrap(),
            Vec2::new(40.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    let target = world
        .spawn_active(
            1,
            world.catalog.id("target").unwrap(),
            Vec2::new(240.0, 40.0),
            BuildFacing::South,
        )
        .unwrap();
    world.enable_combat(1, "BOT", 16.0).unwrap();
    world.set_cloak(shooter, true).unwrap();
    for _ in 0..3 {
        world.step();
    }
    assert!(
        world.entities.get(target).unwrap().health < 10000.0,
        "radar aim must not be rejected as terrain"
    );
    for _ in 0..180 {
        world.step();
        assert!(
            !world.entity_is_cloaked(shooter),
            "ongoing fire must keep delaying cloak"
        );
    }
    world.entities.get_mut(shooter).unwrap().enabled = false;
    for _ in 0..150 {
        world.step();
    }
    assert!(world.entity_is_cloaked(shooter));
}
