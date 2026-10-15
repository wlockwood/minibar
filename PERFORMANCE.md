# Performance review and baseline

## September 2026 audit

The apparent near-50% regression was real, but trace support was not the cause.
`run_internal::<false>` const-specializes the headless path, so aim-trace pushes
and recorder work compile away. An empty `Vec` for aim attempts also performs no
heap allocation.

The dominant regression was a `WeaponDef::clone()` inside the unit/weapon/tick
loop. That cloned the weapon name and type strings on every aiming tick. A
controlled experiment that changed only those strings to empty improved the old
normal Pawn workload by 29%, and a 120-second disabled-weapon workload by 67%.
Direct-fire leading itself costs roughly 7–9% per simulated tick; it also made
the reference fight longer, which exaggerated the drop in simulations/second.

Implemented corrections:

- immutable definitions are prepared once and addressed by runtime definition
  index rather than deep-cloned per trial or weapon tick;
- unit forward vectors are cached, removing repeated `sin`/`cos` reconstruction
  for target velocity and firing origins;
- command, pending-impact, and damage-event storage is reused within a trial;
- range, destination, collision rejection, target selection, and impact hit
  tests use squared distances, with square roots only when required;
- collision separation calculates one square root only for an actual overlap;
- headless terrain LOS and unit-blocker scans occur only when a shot is due;
- terrain stores an exact height ceiling, making high-clearance and flat-map LOS
  acceptance constant-time;
- a regression test confirms traced and untraced runs produce the same outcome.
- scheduled impacts use compact due-tick intrusive queues, so long-flight
  projectiles are not revisited on every intervening tick;
- blocker and impact-victim queries use an epoch-stamped uniform grid in dense
  fights, with an allocation-free linear fallback below the measured crossover.
- Rayon workers retain and reset complete trial workspaces with `map_init`, so
  unit/weapon runtime state, commands, impacts, and spatial scratch keep their
  capacity across seeds;
- per-side live counts are maintained as damage kills entities, avoiding full
  army scans both at tick termination and after each impact batch.
- zero-splash hits first resolve their dense target slot directly; only a miss
  falls back to an incidental-collision spatial query.

## Current baseline (September 2026 post-integration)

Command:

```powershell
cargo build --release --manifest-path simulator/Cargo.toml
simulator/target/release/visigoth-simulator.exe --catalog simulator/examples/pawn-hello.json --unit armpw --benchmark-suite --benchmark-runs 10000 --seed 1
```

Measured on 32 Rayon threads (Windows x86_64, source revision `49a7a0c`, catalog hash `fnv1a:344ed508110e08e3`):

| Workload | Runs | Spawned Units | Simulations/s | Simulated Ticks/s | Unit-Ticks/s | Aggregate Realtime | Allocations / Sim-Tick |
|---|---|---|---|---|---|---|---|
| `duel_1v1` | 10,000 | 2 | 94,025 | 52,797,474 | 105,594,949 | **1,759,916×** | 0.02 calls, 16.6 B |
| `formation_16v16` | 100 | 32 | 2,064 | 3,598,308 | 115,145,851 | **119,944×** | 0.04 calls, 32.8 B |
| `burst_aoe_4v4` | 1,000 | 8 | 132,294 | 17,642,522 | 141,140,175 | **588,084×** | 0.17 calls, 52.7 B |
| `long_flight_1v1` | 10,000 | 2 | 89,286 | 75,337,535 | 150,675,070 | **2,511,251×** | 0.01 calls, 24.0 B |
| `terrain_los_blocked_1v1` | 10,000 | 2 | 634,236 | 95,135,409 | 190,270,819 | **3,171,180×** | 0.02 calls, 2.4 B |
| `traced_1v1` | 10 | 2 | 16,992 | 9,344,128 | 18,688,255 | **311,471×** | 1.45 calls, 160.7 B |
| `strategic_only_16v16` | 10,000 | 32 | N/A | 3,572,449 | 114,318,377 | **119,082×** | **0.00 calls, 0.0 B** |

Every tactical and strategic workload exceeds the 1,000× aggregate realtime requirement by at least two orders of magnitude. The headless strategic world operates with zero heap allocations per tick.

## Active performance invariants

1. **Zero-allocation untraced tick**: The untraced tactical tick and strategic tick loop must perform zero steady-state heap allocations. Rayon workers retain preallocated capacity across trials.
2. **5% regression threshold**: Any change that degrades tactical or strategic unit-ticks/second by more than 5% must be profiled and justified before merging.
3. **Due-tick scheduling**: Long-flight ballistic and projectile updates must maintain intrusive due-tick queues rather than scanning active flights on every tick.
4. **LOS caching & ceilings**: Height ceilings and grid-DDA fast paths must keep flat-map and clear-LOS queries constant-time.
5. **Spatial partitioning**: Uniform grid partitioning activates at $\ge 16$ combatants and indexed collision candidates at $\ge 64$ units, keeping duels free of grid bookkeeping overhead.
