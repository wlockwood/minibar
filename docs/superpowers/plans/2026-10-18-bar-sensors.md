# BAR sensors implementation plan

> Execute inline with focused parallel implementation of catalog and tactical consumers. The user authorized implementation; preserve shared checkout changes and existing training artifacts.

**Goal:** Sensor-limited training and world combat must obey BAR's distinctions between visual LOS, radar/sonar, stealth, cloak, building ghosts and jamming.

**Architecture:** A shared `sensors` module owns detection and deterministic smooth radar error. Catalogs carry source sensor properties. World owns cloak resource/state transitions and retained knowledge; the tactical engine consumes disclosed target positions and exposes filtered policy views. Authoritative physical collision and damage retain real world geometry.

**Tech stack:** Rust, Python catalog conversion, PowerShell verification; existing dependencies only.

**Spec:** User request in this conversation plus BAR `gamedata/modrules.lua`, `LuaRules/gadgets/unit_cloak.lua` and Recoil `LosHandler.cpp`, `Unit.cpp`, `UnitDef.cpp`.

## Constraints and review focus

- Never rebuild/overwrite an active training executable or modify reference checkouts; use `target/sensors-verification`.
- Ground LOS and radar use terrain coverage (also for radar contacts of aircraft); units/buildings do not occlude sensor maps. Air LOS, sonar and jammer coverage is circular. BAR separate jammers protect their own side only.
- Cloak suppresses visual detection, stealth suppresses radar; neither substitutes for the other. Underwater units require sonar. Under-construction stealth is inactive.
- Previously seen immobiles retain exact ghosts until visible terrain disproves them; radar coverage alone cannot disprove a stealth/cloaked ghost. Mobile memory stays explicitly stale. Continuous radar can retain identity; contact loss breaks that continuity.
- Radar aiming and guided projectiles must use disclosed positions. Hidden HP, velocity and unit definitions must not reach tactical brains. Ground fire can target a ghost without resolving a live unit ID.
- Radar error follows BAR's smooth bias behavior with default magnitude96 and halving per active targeting facility; deterministic horizontal implementation is not bit-identical Recoil RNG or sensor mipmap rasterization.
- Aircraft flight, transports, area-cloak gadgets and per-unit BOS script execution remain outside the simulator's existing movement/script model; document any resulting limits precisely.

## Tasks

- [x] Add regressions reproducing missing stealth, cloak, ghost persistence and targeting isolation; observe failures.
- [x] Add validated, round-trippable catalog fields and source exporter coverage (unit_sensors agent).
- [x] Implement shared detection and radar-position error helpers, with rule/terrain/wobble regressions.
- [x] Update World knowledge, cloak commands/proximity/resource costs/recloak/fire behavior, ghost lifecycle and sensor-limited combat commands.
- [x] Update tactical aim/guidance/ground fire and standalone policy observations, with focused integration tests (tactics_sensors agent).
- [x] Run isolated release suite and focused Python checks; review source and document actual approximation boundaries and training backend implications.

No commits requested. No optimizer/checkpoint conversions or package rebuilds are part of this sensor change.

Final evidence: 440 release Rust tests and 43 focused Python tests pass. Native
campaign smoke trained one generation and resumed to two, with both champions
retained; no strength gain claimed. Source checks, source-only catalog diffs,
33 local doc links, viewer JavaScript syntax and final review passed. Evidence:
`data/brains/sensor-verification-20261018/verification.json`. Existing portable
ZIP hash is unchanged. Default executable changed concurrently; this task
qualified only the isolated build. Contact-age additions were preserved.
