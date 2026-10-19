# Sensors, movement and strategic weapons

MiniBAR models the mechanics below, using the effective BAR unit database for
source catalog exports. This is a fast combat model, with limits described here;
matching unit values does not establish identical behavior to a running BAR game.

| Mechanic | Behavior |
| --- | --- |
| LOS | Ground sight respects terrain; aircraft use air sight. Contacts control acquisition and aiming. |
| Radar and sonar | Separate coverage, stealth and jamming; radar contacts disclose estimated positions rather than perfect hidden state. |
| Jamming | Enemy electronic detection can be denied without removing visual sight. Activation and EMP affect electronics; energy shortage alone does not switch radar off. |
| Cloak | Energy costs for stationary/moving units, proximity decloak and firing disclosure. Radar stealth and cloak are separate. |
| Ground movement | Speed cap, acceleration, braking, heading turn rate, reverse speed and turning in place. |
| Fixed-wing aircraft | Effective turn radius limits heading change according to the speed of that movement step. Acceleration and braking affect speed. |
| Hovering aircraft | Heading turn rate applies instead of radius; aircraft can stop and turn at rest. Builders using hover flight are classified separately from fixed wings. |
| Turrets | Independent yaw and pitch tracking using resolved script rates; weapons wait for aim readiness. |
| Nukes | Explicit manual ground launch, ammunition manufacturing, resource costs, reload, travel time and area damage. Static silos can launch in any direction. |
| Anti-nukes | Interceptor/targetable masks, coverage around the intended impact point, inventory, resources and reload gates. |

Air units retain their separate learned policy. These changes do not merge the
ground and air brains or teach the current policies to launch nuclear weapons.

## Directional engagements on land and in the air

Scenarios can set an `approach` for both armies. Heading offsets are degrees from
facing the other army: `0` faces it, `90` turns right, `-90` turns left and `180`
faces away. Speed fractions are fractions of each unit's own top speed; static
units remain stopped. For example, this starts the learner pointing at an opponent
that is crossing its path, with both armies already moving:

```json
"approach": {
  "name": "learner_crosses_opponent",
  "learner_heading_offset_deg": 0,
  "enemy_heading_offset_deg": 90,
  "learner_speed_fraction": 1,
  "enemy_speed_fraction": 1
}
```

The army's formation rotates with its heading. Its front-line separation is
preserved and flat arenas grow to fit it. Settings follow the learner and opponent
when their physical map sides are swapped. Speeds are restored before every trial.
Captured-map starts that cannot fit are rejected. Omitted settings retain the
historical stationary head-on start.

Generated matchups accept an `approaches` array of these objects. Each fight samples
one reproducibly, independently of its composition/resource/range draws; the chosen
setup appears in its scenario record and generated battle label. New catalog exports
include stopped/moving head-on, crossing from either side, rear and oblique presets
for both land and air. Existing configurations need that array added to opt in.

The engine withholds fire until fixed weapons face the target. Turrets additionally
respect their source mount's body-relative 3D cone (`maindir`/`maxangledif`), as well
as their aiming rates. No universal 30-degree arc or directional damage bonus is
added. Fixed weapons use source `tolerance` as the forward half-angle;
`firetolerance` is a separate reaim threshold. Fixed missiles launch along the hull
heading, then use their guidance turn rate to pursue the target. Fixed guns on
hovering aircraft keep their horizontal constraint but treat body pitch toward
the target as instantaneous, so close ground attacks remain possible. Other fixed
weapons and turret mount cones include elevation. Body pitch/roll
and inherited aircraft velocity remain simplified. Missile turning limits apply
to horizontal steering; vertical homing remains immediate. Benefits depend on the actual weapons,
movement and policies, so crossing does not guarantee victory.

Controlled examples hold rosters and distance fixed while changing approach:
[ground](../examples/approach-ground.json) and [air](../examples/approach-air.json).
The air example starts already moving near contact, rather than simulating takeoff.
To make a watchable air comparison using a fresh authored baseline, without training:

```powershell
python scripts/export_training_catalog.py --out data/approach-check armpw armfig
cargo build --release --target-dir target/directional-verification
$sim = 'target/directional-verification/release/visigoth-simulator.exe'
& $sim --seed-brain tactical-air-v1 --brain-out data/approach-check/air-baseline.json
& $sim --catalog data/approach-check/catalog.json --watch data/approach-check/air-baseline.json --train-config examples/approach-air.json --watch-out data/approach-check/air.html
```

Use `tactical_v4` and the ground example for the equivalent land comparison. These
examples test support and expose behavior; they do not establish BAR combat parity
or strength of a trained policy. Re-export old catalogs to include mount cone data.

## Correct unit values

Use the [`bar-units` MCP](UNIT_DB.md) for unit facts. The exporter uses its shared
database code for movement defaults and conversions and BAR's standardized wreck
values. For real source units, it also reads the saved database and checks the
selected effective records against current source before importing weapon timing,
splash and script turret rates. A mismatch requires rebuilding the database.
Weapon mounts are matched by source index, since several mounts can share a name.
`source_mount` in generated weapon data records that correspondence.
The database currently omits firing/reaim tolerances; the exporter reads these
from the hashed source weapon definitions using Recoil's documented defaults.

Ground heading rates use the engine conversion, rather than the former rounded
`0.16` multiplier. Fixed-wing aircraft use effective `turnradius`, including engine
defaults; their ground-style `turnrate` is ignored. Hovering aircraft use heading
rates. Effective acceleration/braking defaults prevent missing Lua fields from
becoming zero acceleration. Wreck health and metal include BAR's default
postprocessing rather than just the raw feature table.

Old catalogs still load with their stored values. Rebuild them to obtain these
corrections. Do not use `make_catalog.py --base` to refresh entries: it preserves
existing entries verbatim. Existing campaign results are not comparable across a
changed catalog or executable; use a fresh output directory when training again.

## Reproduce the source-backed weapon probe

From the repository directory:

```powershell
python scripts/unit_db.py check
python scripts/export_training_catalog.py --out data/mechanics-check armpw corsumo armfig armpnix armbrawl armrad armjamt armspy armsilo armamd
cargo run --release --target-dir target/mechanics-verification --example mechanics_probe -- data/mechanics-check/catalog.json
```

The probe queues a launch before manufacture finishes, with no disclosed enemy
entity. It checks that an unopposed warhead damages its target after manufacturing,
and that a stocked anti-nuke protects the target in a second trial. It uses finite
resource pools and the generated units' actual ammunition times. This is a workflow
and functional check, not a replay comparison against BAR.

The launch API is `CombatStepper::set_manual_ground_target(key, mount, position)`.
It queues one salvo, waits for ammunition and aim, then completes. A new launch
requires a new order. Ordinary targeting never launches manual weapons. The
strategic `World` attack actions and existing learned policies do not issue this
new explicit command yet. Stockpile manufacturing automatically queues inventory
up to exported `stockpile_limit`; legacy catalogs default to one ready round.
Manufacturing charges each round's resource cost at the start, rather than
streaming its cost over construction time.

## Fidelity limits

- Sensor rasterization, radar position error and scripted shutdown/cloak gadgets
  retain the limits described in [BAR_SENSORS.md](BAR_SENSORS.md).
- Flight uses kinematics, not the engine's aerodynamic banking and steering.
  Fixed wings retain a modeled minimum speed of 55% of top speed. Hovering aircraft
  move along their heading; sideways gunship motion, takeoff, landing, transports
  and terrain-following climb dynamics are absent.
- Unresolved turret scripts use instantaneous aim. A canonical null rate removes
  guesses from the older exporter; it is not a measured rate. Arbitrary script
  animation, aim acceleration and additional script constraints are not executed.
- Nuclear Starburst travel uses timed straight-line projectile flight, without
  BAR's vertical launch, cruise/verticalize gadget, acceleration profile or crater
  deformation. Interception resolves against ready defenders at launch rather
  than flying a separate interceptor that can miss. Defensive changes while the
  warhead is airborne therefore are not reproduced. The ordinary weapon kernel's
  simplified collision, splash and friendly-fire rules still apply.
- Ground movement uses MiniBAR's terrain/passability and collision model. Exact
  Recoil slope-dependent dynamics and pathfinding are not reproduced.

Tests cover the modeled rules and the source-to-catalog boundary. Full trajectory,
timing and combat parity still require captures from a running BAR/Recoil game.
