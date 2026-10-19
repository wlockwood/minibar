# BAR unit database and MCP server

`data/units/bar_units.json` holds one record per Armada, Cortex and Legion unit
(679 units at BAR commit `138018b6b2`, 2026-10-18). The records are built from
the Beyond All Reason checkout at `D:\code\BAR\Beyond-All-Reason`. The
`bar-units` MCP server lets Claude Code, Codex and other agents look units up,
search them and check claimed values without parsing Lua themselves.

| File | Purpose |
| --- | --- |
| `scripts/unit_db.py` | Builds and checks the JSON (`build`, `check`, `show <codename>`). |
| `scripts/unit_db_mcp.py` | The MCP server (stdio, Python standard library only). |
| `scripts/test_unit_db.py` | Golden values, drift checks against the checkout, and MCP tests. |
| `.mcp.json` | Registers the server with Claude Code. |
| `.codex/config.toml` | Registers the server with Codex. |

## MCP tools

| Tool | Use it to |
| --- | --- |
| `get_units` | Fetch one or many units by codename (`cordoom`) or English name (`Bulwark`). `fields` returns only the listed paths. |
| `search_units` | Filter by faction, tech level, mobility, text, or `{field, op, value}` filters, with sorting and field selection. |
| `resolve_unit_name` | Turn a name into a codename, or list close candidates. |
| `validate_unit_data` | Check claims like `{unit: "Pulsar", field: "weapons[ata].range", value: 1400}`. Each claim comes back as `match`, `mismatch` (with the real value), `unknown_unit`, `ambiguous_unit` or `unknown_field`. `check_source: true` also re-reads those units from the BAR checkout. |
| `check_db_freshness` | Rebuild records from the checkout and list every difference from the saved JSON. |
| `describe_fields` | Field meanings, units, the source BAR commit, and the path syntax. |

Names that match more than one unit, such as "Commander", are never guessed.
The tools return the candidates instead.

**Paths** are dotted: `cost.metal`, `sensors.radar_jam`, `weapons[0].range`
(0-based), `weapons[atadr].vs.air.dps` (selected by weapon def), or
`weapons.range` (one value per weapon).

## What each record contains

| Key | Contents |
| --- | --- |
| `name`, `description`, `faction`, `tech_level` | English name and tooltip from `language/en/units.json`. `tech_level` is `customparams.techlevel`, which defaults to 1. |
| `cost` | `metal`, `energy`, and `buildtime` (the build power cost). |
| `builder` | `buildpower` (workertime), `build_distance`, `build_options`, and the separate `can_build`, `can_assist`, `can_repair`, `can_reclaim`, `can_resurrect` flags, after the engine's defaults are applied. |
| `stealth`, `sonar_stealth` | Radar and sonar stealth. |
| `cloak` | `can_cloak`, `cost_static` and `cost_moving` (energy per second), and `decloak_range` (mincloakdistance). |
| `sensors` | `los`, `air_los`, `radar`, `sonar`, `radar_jam`, `sonar_jam`, `seismic` (elmos). |
| `movement` | `null` for units that cannot move. `move_type` is `ground`, `hover_air` or `strafe_air` (see below). Also `speed` (elmos/s) and `acceleration` and `brake` (engine elmos/frame², with elmos/s² copies). Ground units and hovering aircraft have `turn_rate` (heading units/frame) and `turn_rate_deg_s`. Fixed-wing aircraft have `turn_radius` (elmos) instead. |
| `wreck` | `null` if the unit leaves no corpse. Otherwise the wreck's `metal`, `energy` and `health`, plus `chain`: the wreck, then what it decays into (usually the heap), each with `metal`, `energy`, `health`, `reclaimable` and `resurrectable`. |
| `size` | `radius`: the circle with the same area as the unit's outline seen from above. It comes from the collision volume when the unit file sets one, otherwise from the footprint. |
| `weapons[]` | One entry per weapon mount. See the next section. |
| `combat` | For land, air and sub targets: `dps` (total from weapons that fire on their own), `max_range`, and `dps_by_range`. `dps_by_range` lists one band per distinct weapon range, longest first. Each band has its mounts, their DPS, and `cumulative_dps` (the DPS of every counted weapon that reaches that distance). |
| `summary` | One plain-text line for search and RAG indexing. |

Each weapon mount has:

- `vs.land`, `vs.air`, `vs.sub`: `can_target`, `range` (null if it cannot
  target that kind), damage per hit, salvo damage and DPS for that kind of
  target. Land targets use the `default` damage
  column, aircraft use `vtol`, and submarines use `subs`. `can_target` comes
  from the mount's `onlytargetcategory`. Hitting submarines also needs a
  water weapon.
- `aoe`: the splash radius and its edge effectiveness, plus two radii that
  account for falloff. `half_damage_radius` is where splash damage drops to
  half. `effective_radius` is the radius of a full-damage circle that would
  deal the same total damage.
- `arc`: the mount's aim limits from `maindir` and `maxangledif`.
- `turret_turn` (only when `turret = true`): yaw and pitch speed in degrees
  per second, read from the unit script.

**Aircraft move types** follow the engine (RecoilEngine `UnitDef.h`,
`IsHoveringAirUnit`). Aircraft that can build, carry transports or have
`hoverattack` (gunships) are `hover_air`. They hover all the time and turn at
`turn_rate`. All other aircraft (fighters, bombers, scouts) are `strafe_air`.
The engine ignores their `turnrate` and steers them by `turn_radius`, which
defaults to 500 (1000 for bombers) when the unit file doesn't set it. Their
acceleration defaults to 0.065 for armed planes.

**Wrecks** use BAR's rules from `alldefs_post.lua`. For every unit except
commanders, the wreck and heap health is set to the unit's health, and their
metal to 60% and 25% of the unit's metal cost (rounded down). So the Pawn's
wreck is 32 metal, not the 29 written in its file. Commanders keep their file
values. The engine's default `resurrectable` (-1) means only the unit's own
corpse can be resurrected, not the heap.

`describe_fields` and the JSON's `field_notes` give the full definitions.

## How faithful the values are

The values are the unit-file values after the parts of BAR's default
post-processing that change these fields (`gamedata/alldefs_post.lua` with no
mod options set):

- reload and burst times are rounded to whole frames;
- `impactonly` beam lasers get areaofeffect 11 and edge effectiveness 1;
- tech level defaults to 1;
- `Spring.GetModOptions().x` takes the default value from `modoptions.lua`.

Engine defaults come from RecoilEngine `UnitDef.cpp`. For example, `air_los`
defaults to 1.5 × LOS, `can_repair` and `can_reclaim` default to whether the
unit is a builder, and `can_resurrect` defaults to false. Mod options such as
unit multipliers, techsplit or the air rework are **not** applied.

Known gaps:

- `armcomnew`, `legcombabyunits` and `legevodecom` use Lua the reader cannot
  parse (string concatenation and `VFS.Include`). They are listed under
  `errors` and have no records.
- Turret speeds are read from the unit scripts. 502 of 530 turrets resolve.
  The rest have `method: "unresolved"` and null speeds, because their script
  works the speed out at run time. `instant` means the aim function turns
  nothing, or turns with `now`. `no-aim-function` means the script has no aim
  function for that mount, so the engine aims without it. The Lua commander
  scripts share one `AimWeapon` across mounts; those entries carry a `note`.
- Bombers can show air DPS. BAR lets their bombs target any non-submarine,
  and the engine only checks that the target is below the plane, so in
  practice that number rarely means anything.
- Scripts can limit aim further than `arc` shows.

## Keeping it in sync with BAR

After the BAR checkout is updated:

```powershell
python scripts/unit_db.py check   # lists differences; exits 1 if the JSON is stale
python scripts/unit_db.py build   # rewrites data/units/bar_units.json
python scripts/test_unit_db.py
```

`test_unit_db.py` fails when the saved JSON no longer matches a fresh build
from the checkout. It also re-reads every unit's plain top-level fields (costs,
health, build power, sensor ranges, stealth, cloak, speed, acceleration, turn
rate) from the raw `.lua` text with a regex. That check is separate from the
parser that built the database, so a parser bug cannot hide a wrong value. The
drift tests are skipped when the BAR checkout is missing.

## Registration

- **Claude Code** reads `.mcp.json`. The first session in this folder asks
  you to approve `bar-units`. `.claude/settings.local.json` lists it under
  `enabledMcpjsonServers`.
- **Codex** reads `.codex/config.toml` because `d:\code\bar\minibar` is a
  trusted project. `codex mcp list` should show `bar-units` as enabled.
- Both run `python scripts/unit_db_mcp.py` from the repository root. The
  server reloads the JSON when the file changes, so a rebuild does not need a
  restart.
