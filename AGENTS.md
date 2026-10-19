# Agent notes for Minibar

## BAR unit facts: use the `bar-units` MCP server

For any Beyond All Reason unit fact (cost, health, wreck value, build power,
builder abilities, stealth, cloak, sensor and jammer ranges, speed and turning
or turn radius, weapon damage and range against land, air and subs, splash,
turret arcs and turn speeds, size),
query the `bar-units` MCP server. Don't guess, and don't take numbers from
memory or from other models' answers.

- `get_units` takes codenames (`cordoom`) or English names (`Bulwark`), one
  or many. Use `fields` to fetch only what you need.
- `search_units` filters and sorts, e.g. every Cortex unit with radar jamming.
- `validate_unit_data` checks claimed values (for example from a design doc
  or another model) and returns the real value for each mismatch.
- `check_db_freshness` confirms the database still matches the BAR checkout.

If the server is not connected, the same data is in
`data/units/bar_units.json`. `python scripts/unit_db.py show <codename>`
prints one record. Field meanings, units and known gaps are in
[docs/UNIT_DB.md](docs/UNIT_DB.md).

Run `python scripts/test_unit_db.py` after changing `scripts/unit_db.py`,
`scripts/unit_db_mcp.py` or `scripts/bar_lua.py`. Run
`python scripts/unit_db.py build` after the BAR checkout is updated.
