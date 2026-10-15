# Map acquisition and cache pipeline

The fast simulator does not parse SMF archives during a trial. Map handling has
two stages:

1. An acquisition adapter runs once and produces canonical `MapSource` JSON.
   The bridge `hello` payload remains supported through
   `MapSource::from_hello_json` for bounded compatibility captures.
2. `CompiledMap::compile` validates and quantizes it, then
   `to_cache_bytes` writes a portable, versioned `VSGMAP` cache. Every trial
   shares that immutable map and creates only its own `DynamicOccupancy`.

The CLI exposes that seam directly:

```powershell
cargo run --release --manifest-path simulator/Cargo.toml -- --compile-map data/map.map_source.json --map-out data/map.vsgmap
```

For a one-off duel, scenario, or interactive live session, `--map` accepts
either that compiled `.vsgmap` cache **or** the source JSON itself. A source is
validated and compiled exactly once before the world is created; it never puts
parsing or map allocation in the simulation tick. For example:

```powershell
cargo run --release --manifest-path simulator/Cargo.toml -- `
  --catalog simulator/examples/pawn-hello.json `
  --run-scenario simulator/examples/wall-defense.json `
  --map simulator/examples/flat-pawn-map.json --serve 127.0.0.1:0
```

`CompiledMap::load_path` is the library equivalent. It detects the cache magic
or JSON shape and, for a canonical `name.map_source.json`, automatically merges
an adjacent `name.acquisition.json` if present. Thus interactive maps retain
their heights, water plane, per-MoveDef passability, discrete/continuous metal,
geothermal vents, and static feature blockers without a separate importer.
Persist the cache with `--compile-map` before a high-volume benchmark.
[`examples/river-crossing.map_source.json`](examples/river-crossing.map_source.json)
is a small canonical fixture containing a water crossing, distinct bot/hover
passability, metal and geothermal sites, and a reclaimable fire/movement
blocker. It is intentionally useful both as a loader example and a tactical
obstruction test map.

Input format is detected from top-level fields. Canonical sources retain their
row-major native-corner grids; hello grids take the existing X-major adapter.
Raw capture records and acquisition sidecars produce an actionable format
error. Standard assembler output embeds every map/game checksum and version,
map/mod option, extractor parameter, and full MoveDef record in cache version
2. For older canonical files, the CLI auto-loads a sibling
`*.acquisition.json` (or an explicit `--acquisition PATH`). Inputs without
either source still compile but report that provenance is incomplete.

`--legacy-movement-class NAME` is an explicit escape hatch for old hello files
that contain only the land mask. New bridge captures contain named MoveDef
layers and should not use it.

`MapSource` deliberately is not coupled to one archive format. A Recoil widget,
a short headless extraction match, or a future SMF/SD7 reader can all populate
the same input. The content hash remains the compact cache key. Its constituent
identity and rules are also retained as structured provenance so mismatches can
be explained and audited rather than inferred from an opaque digest.

## Preferred Recoil acquisition

- Sample original ground height on the engine's 8-elmo map grid. Preserve the
  water level separately, then depth is `max(water_level - ground_height, 0)`.
  The runtime can therefore distinguish dry land, shallow water, and deep water.
- Export a named passability mask for every distinct MoveDef used by the unit
  catalog. A one-bit land mask is not enough. During an extraction match,
  `Spring.TestMoveOrder` with terrain enabled and objects disabled can produce
  engine-authoritative masks. `Spring.GetGroundInfo` additionally exposes the
  terrain type's tank, kbot, hover, and ship speed multipliers.
- Use `Spring.GetMetalMapSize()` for the metal-grid dimensions and call
  `Spring.GetMetalAmount(mx, mz)` with metal-square coordinates—not world X/Z.
  `Game.metalMapSquareSize` converts between those spaces. Preserve all samples
  in `continuous_metal`, even when BAR's resource spot finder also returns
  discrete spots.

The bridge hello currently samples 32×32 metal-square centers to keep its
single startup message bounded. This detects and represents metal maps without
aliasing world and metal coordinates, but a native-resolution extractor should
populate `MapSource.continuous_metal` directly for balance calibration.
- Enumerate map features and retain positions whose `FeatureDef.geoThermal` is
  true. Prefer an engine-tested placement tolerance; the hello adapter otherwise
  uses one 16-elmo building square.

Recoil terrain, height, water, metal, and feature queries are documented in the
[Lua API](https://recoilengine.org/docs/lua-api/). Recoil uses 8×8-elmo basic map
squares and normally 16×16-elmo building-footprint squares, as described in
[Yardmaps and map squares](https://recoilengine.org/docs/guides/yardmaps/).

The bridge's older 32×32 `height >= 0` mask can only be imported by explicitly
setting `HelloMapOptions.accept_legacy_mask_as`. The default is an error so a
coarse land-only mask cannot silently become the answer for ships, hovers,
amphibious units, slope-limited vehicles, or narrow wall gaps.

## Runtime representation

- Heights and continuous metal use unsigned 16-bit quantization with per-grid
  scale and offset. Height queries are bilinear and allocation-free.
- Named passability masks use one bit per cell. Unknown movement classes and
  out-of-bounds positions are impassable.
- Continuous-metal extraction is convolved with the game's extractor radius at
  compile time. Candidate-site income is then a single bilinear lookup rather
  than a scan of every metal square. Discrete spots and continuous fields can
  coexist.
- Geothermal queries return a vent index, allowing the economy simulation to
  reserve a specific vent instead of treating geothermals as generic terrain.
- `DynamicOccupancy` tracks constructed buildings, construction reservations,
  wall segments, and captured map features independently of terrain. Use
  `World::new_with_static_features` to deterministically materialize the
  cache's blockers as neutral, reclaimable feature entities. Removing a feature
  releases its counted movement, construction, and fire-blocking contributions.
  Occupancy epochs change only with effective geometry. Mobile units remain in
  the combat spatial index rather than this grid.
- `CompiledMap::to_tactical_terrain(class, cell_size)` materializes the current
  combat engine's `Terrain` once per scenario. The cell size must evenly divide
  both map dimensions; 8 or 16 elmos are the intended values.

The cache format starts with an eight-byte magic and an explicit version, uses
little-endian fixed-width values, length-prefixes all collections, rejects
truncation/trailing bytes, and validates dimensions when loaded. Format changes
must increment `MAP_CACHE_VERSION`.

## Intentional approximations

The preconvolved metal yield includes metal-map cells whose centers lie inside
the extractor radius. This is deterministic and close to the engine without
replicating every extraction edge case. Terrain deformation and changing water
or lava levels require a new cache or a future dynamic overlay. A feature whose
collision height is unavailable remains a movement/construction blocker but is
not assigned an invented fire-obstruction height.
