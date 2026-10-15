# Shared MiniBar viewer contract

`viewer.html` is the single dependency-free frontend embedded by the Rust live
server and served verbatim by the Python historical-review server. It contains
no source-path access and uses only same-origin HTTP/WebSocket requests.

The protocol schema and validation types live in
`simulator/src/viewer_protocol.rs`. Both configuration and frames use version
1. Unknown versions must be rejected rather than guessed.

## Bootstrap API

The viewer first requests `GET /viewer-config.json`. A missing endpoint means
Rust live mode and falls back to `/ws`. A historical or trace backend returns:

```json
{
  "schema": "visigoth.viewer_config",
  "schema_version": 1,
  "mode": "historical_review",
  "capabilities": {
    "time_controls": false,
    "unit_commands": false,
    "playback": true,
    "review": true,
    "future_confirmation": true
  },
  "frame_endpoint": "/api/viewer/frame",
  "review_endpoint": "/api/review",
  "prediction_endpoint": "/api/viewer/prediction",
  "initial_frame_index": 6,
  "frame_count": 10,
  "decision_frame_index": 6
}
```

Endpoint paths must be same-origin absolute paths. Historical configuration is
invalid if it grants `time_controls` or `unit_commands`.

The viewer requests `GET {frame_endpoint}?index=N`, where the server validates
that `N` is inside its bounded loaded window. The response is one
`visigoth.viewer_frame` document. Previous/next prediction requests use
`GET {prediction_endpoint}?direction=previous|next` and return a replacement
viewer-config document.

Review controls send `POST {review_endpoint}` with:

```json
{
  "prediction_id": "opaque-id",
  "action": "accept",
  "notes": "optional",
  "corrected_goal_kind": "optional",
  "corrected_target_sector": [3, 4]
}
```

The UI reports a durable save only for a successful response containing
`{"committed": true}`. A conflict or save failure leaves the current
prediction displayed.

## ViewerFrame rules

`ViewerFrame` is independent of simulator `World`. Coordinates are BAR/Recoil
X/Z: X increases right and Z increases down. It carries map dimensions and
provenance, time/frame identity, 8x8-by-default sector geometry, entities,
health/build state, metadata, commands, trails, economy history, sites,
detector evidence, nearby events, home/target positions, and annotations.

Every annotation and overlay is marked `simulation`, `recorded`, `derived`, or
`unavailable`. Historical adapters must not fill absent fields from a current
catalog, another observation, spectator state, or MiniBar simulation. Radar
entities may not carry definition, display name, health, or build progress;
Rust validation rejects such a frame. Existing normalized trajectories lack
engine unit IDs, so correspondence trails use replay-local opaque IDs, must be
marked `derived`, and must be suppressed when ambiguous.

The historical server owns bounds, provenance and mutations: it must serve only
loopback, reject `/ws` and simulation actions, bound frame payload/window size,
and commit reviews through the shared provenance-aware review API.
