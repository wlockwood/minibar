# BAR sensors in MiniBAR

Sensor-limited worlds and standalone battles share detection rules. Strategies
receive disclosed contacts; aiming and missile guidance use their side's contact
positions. Physical collision and damage use authoritative geometry, so a round
can hit a hidden unit by chance.

| Contact | Disclosed information |
| --- | --- |
| Visual LOS | Exact position, type and current health. Terrain shadows apply; units and walls do not block the sensor map. |
| Radar / sonar | Slowly drifting position error; no current health. Unknown contacts have no concrete type. Visually identified mobiles retain identity during continuous tracking; full contact loss breaks identification. |
| Building ghost | Last visual position and known type, with no live health updates. Live ghosts survive cloaking. An entirely unseen death leaves a ghost until its location is visually checked; a radar-tracked death is known and clears it. |
| Remembered mobile | Optional stale last disclosure, distinct from a building ghost. It is not a current entity target. |

Ground LOS and radar are terrain limited. Air LOS is circular; radar coverage
still uses terrain. Radar error defaults to **96 elmos**, halved by each completed,
activated, unstunned targeting facility. Stationary blips drift too. Previously
seen immobiles retain exact known positions even when tracked only by radar.

`radar_position_resolution` retains its legacy name but now means error magnitude,
replacing grid snapping. JSON also accepts `radar_error_size`; explicit zero
disables error for controlled tests. The CLI switch remains `--radar-resolution`
(default 96).

## Detection rules

- **Stealth** blocks radar, independently of visual sight. Sonar stealth is
  separate. Under-construction units do not benefit from stealth.
- **Cloak** blocks visual detection without granting radar stealth. Actual cloak
  requires completion, energy, no EMP stun, no enemy within decloak distance, and
  no firing cooldown. The desired cloak state persists when admission fails.
- Cloak charges half a second of energy at each half-second check. Moving cost
  applies above approximately 13.416 elmos/second. Spherical proximity includes
  enemy radius; cylindrical proximity uses horizontal center distance. Nearby
  enemies need no LOS. Firing normally blocks recloak for 128 frames, extended by
  later shots. Damage alone does not universally decloak in BAR.
- **Jamming** protects contacts belonging to the jammer's side inside its circular
  field. It does not suppress visual sight, the other side's contacts, or building
  ghosts. Sonar jamming is separate. Incomplete, deactivated or stunned electronic
  sensors do not operate. Visual sight survives deactivation/EMP; energy shortage
  alone does not toggle radar/jamming off.
- Fully submerged units require sonar, including for visual identification.
  Surface units can be detected by sonar or radar. Rocks and wrecks are globally
  disclosed, matching BAR's `featureVisibility=3`.

World commands accept `{"type":"set_cloak","entities":[{"slot":3,"generation":0}],"cloaked":true}` (using an owned entity ID),
with ownership/capability validation. `World::entity_is_cloaked` reports actual
admission. The strategic controller and live viewer attack ghosts using explicit
ground targets, including after an unseen death.

## Catalogs and training

Both source catalog generators now export stealth, cloak costs/distance,
emission heights, sonar/jammer radii, targeting facilities and ghost classification.
Catalog loading, validation and re-export preserve these properties.

This workflow writes fresh catalog copies and uses an isolated build. Previously
exported packages retain their bundled inputs and executable. New copies:

- [22-unit training catalog](../data/brains/from_replays/catalog.sensors.json)
- [354-unit simulable catalog](../data/replay/catalog-simulable.sensors.json)
- [Campaign with the refreshed training catalog](../examples/campaign/replay-informed-sensors.json)

The new campaign starts from authored baselines. Historical champion scores do
not establish strength under these changed rules. Start a new run directory;
old optimizer checkpoints have a different backend/input identity.

```powershell
cargo build --release --target-dir target/sensors-verification
target/sensors-verification/release/visigoth-simulator.exe --train-campaign examples/campaign/replay-informed-sensors.json --train-out data/brains/campaign-sensors --threads 4
```

Refresh another flat catalog without changing it in place:

```powershell
python scripts/refresh_sensor_catalog.py INPUT.json --out NEW.json --bar-root ../Beyond-All-Reason
```

Refresh rejects missing/ambiguous sources and existing output files, preserves
movement/weapons/costs, and records source hashes.

## Precision and remaining limits

These are source-backed detection/knowledge rules, not bit-identical Recoil
simulation. Terrain coverage uses captured heightfields instead of Recoil's
64/128/32-elmo LOS/air/radar raster maps, emission-height buckets and staggered
updates. Radar error uses deterministic horizontal 256-frame interpolation instead
of Recoil's synced 3D random walk. Water and spherical proximity use MiniBAR's
radius proxy. Aircraft now have bounded fixed-wing and hover flight, including
simulated cruise altitude; landing and transports remain unsupported. Source
aircraft exports set `use_air_los`. See [MECHANICS.md](MECHANICS.md) for movement
and strategic-weapon support and its remaining limits.

Arbitrary scripts and area-cloak gadgets are not executed. Some BAR jammer and
pinpointer BOS scripts disable their unit for eight seconds after damage. Those
per-unit shutdowns remain unsupported; a jammer radius does not imply that rule.

Standalone policy snapshots censor unknown contacts using an `unknown-contact`
definition, zero as an unknown-health placeholder, no live velocity, and no
private weapon state. Continuously identified contacts retain their known
definition. Existing brain inputs lack an explicit unknown-health mask and need
retraining for these observations. Weapon prediction conservatively withholds
unknown radar velocity; Recoil's internal weapon code can use true velocity.
Legacy tactical fixtures with no sensor declarations retain their oracle path;
strategic `ObservationPolicy::Omniscient` remains available for benchmarks.

Tests exercise helpers, actual World combat and standalone battles: modalities,
terrain, jamming/EMP, error drift, cloak costs/admission/firing, identity loss,
ghosts/removal, censored views, ground fire and lost-contact missile/beam behavior.
They do not substitute for a running-game parity capture.

[Verification receipt](../data/brains/sensor-verification-20261018/verification.json):
440 release Rust tests, 43 focused Python tests, and a two-generation native
training/resume smoke on the 354-unit catalog passed. The smoke retained both
initial champions; it establishes workflow operation, not playing strength.
The source checkout contains concurrent contact-age/uncertainty additions, which
were preserved and tested together with the sensor changes.

Source snapshots: BAR 905b6912, Recoil 70a612b8:

- [BAR modrules](https://github.com/beyond-all-reason/Beyond-All-Reason/blob/905b691212bcdc6057c3c0f66beacd68d223eb5f/gamedata/modrules.lua), [cloak gadget](https://github.com/beyond-all-reason/Beyond-All-Reason/blob/905b691212bcdc6057c3c0f66beacd68d223eb5f/luarules/gadgets/unit_cloak.lua)
- [Sensor coverage](https://github.com/beyond-all-reason/RecoilEngine/blob/70a612b80392d17f10e60fdf0544fa934d74bc9f/rts/Sim/Misc/LosHandler.cpp), [unit detection/error](https://github.com/beyond-all-reason/RecoilEngine/blob/70a612b80392d17f10e60fdf0544fa934d74bc9f/rts/Sim/Units/Unit.cpp)
- [Definition defaults](https://github.com/beyond-all-reason/RecoilEngine/blob/70a612b80392d17f10e60fdf0544fa934d74bc9f/rts/Sim/Units/UnitDef.cpp), [ghost lifecycle](https://github.com/beyond-all-reason/RecoilEngine/blob/70a612b80392d17f10e60fdf0544fa934d74bc9f/rts/Rendering/Units/UnitDrawerData.cpp), [proximity](https://github.com/beyond-all-reason/RecoilEngine/blob/70a612b80392d17f10e60fdf0544fa934d74bc9f/rts/Game/GameHelper.cpp)
