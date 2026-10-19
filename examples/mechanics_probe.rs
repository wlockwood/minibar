//! End-to-end probe using an independently generated BAR catalog.
//! See docs/MECHANICS.md for the export and run commands.
use std::{path::Path, sync::Arc};
use visigoth_simulator::{
    CombatDefinitionId, CombatSpawn, CombatStepper,
    catalog::{IMPLEMENTED_CAPABILITIES, load_catalog_with_capabilities},
    model::{SIM_HZ, StartingResources, Terrain, Vec2},
};

fn trial(catalog_path: &Path, defended: bool) -> Result<serde_json::Value, String> {
    let catalog =
        load_catalog_with_capabilities(catalog_path, IMPLEMENTED_CAPABILITIES.iter().copied())?;
    let definitions = ["armsilo", "corsumo", "armamd"]
        .iter()
        .map(|name| {
            catalog
                .get(name)
                .cloned()
                .map(Arc::new)
                .ok_or(format!("missing {name}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let launch_delay = definitions[0].weapons[0].stockpile_time;
    let defender_delay = definitions[2].weapons[0].stockpile_time;
    let mut stepper = CombatStepper::new(Terrain::flat(6000.0, 3000.0, 32.0), definitions, 7);
    for (key, side, definition, position) in [
        (1, 0, 0, Vec2::new(500.0, 1500.0)),
        (2, 1, 1, Vec2::new(4500.0, 1500.0)),
        (3, 1, 2, Vec2::new(4500.0, 1900.0)),
    ] {
        if key == 3 && !defended {
            continue;
        }
        stepper
            .insert(CombatSpawn {
                key,
                side,
                definition: CombatDefinitionId(definition),
                position,
                heading: std::f32::consts::PI,
                hp: None,
            })
            .map_err(|e| format!("{e:?}"))?;
    }
    stepper.set_resources(Some(
        [StartingResources {
            metal: 1_000_000.0,
            energy: 1_000_000.0,
        }; 2],
    ));
    let before = stepper.combatant(2).unwrap().hp;
    // Launch is queued before manufacture completes. Ground positions do not
    // require the silo to acquire a hidden entity through fog of war.
    stepper
        .set_target_observations(0, &[])
        .map_err(|e| format!("{e:?}"))?;
    stepper
        .set_manual_ground_target(1, 0, Vec2::new(4500.0, 1500.0))
        .map_err(|e| format!("{e:?}"))?;
    let duration = launch_delay.max(defender_delay) + 15.0;
    let mut first_damage = None;
    for tick in 0..(duration * SIM_HZ).ceil() as usize {
        stepper.step();
        if first_damage.is_none() && stepper.combatant(2).unwrap().hp < before {
            first_damage = Some((tick + 1) as f32 / SIM_HZ);
        }
    }
    let after = stepper.combatant(2).unwrap().hp;
    if defended && after != before {
        return Err("anti-nuke failed to protect target".into());
    }
    if !defended && after >= before {
        return Err("unopposed manual nuke caused no damage".into());
    }
    if first_damage.is_some_and(|time| time < launch_delay) {
        return Err("damage arrived before a warhead was manufactured".into());
    }
    Ok(
        serde_json::json!({"defended": defended, "target_hp_before": before,
        "target_hp_after": after, "first_damage_s": first_damage,
        "nuke_inventory": stepper.stockpiled_rounds(1, 0),
        "anti_nuke_inventory": if defended { stepper.stockpiled_rounds(3, 0) } else { None }}),
    )
}

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("pass the generated catalog.json path")?;
    let results = [
        trial(Path::new(&path), false)?,
        trial(Path::new(&path), true)?,
    ];
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
    Ok(())
}
