# ramonda: standalone trip-subscription WebSocket API

**Public endpoint:** `wss://ramonda.catenarymaps.org/`

**Localhost endpoint:** `ws://127.0.0.1:52772/`

**Source:** [`src/ramonda/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/ramonda/main.rs)

This is the WebSocket service for per-trip realtime subscriptions. It superseeds [spruce `/ws/trip`](spruce-websocket-api.md), in order to split the load across multiple services.

## Route

`GET /ws` and `GET /ws/` (both map to the same actor). `GET /` returns a plain-text liveness string (`"Hello World from Catenary Ramonda! <rfc3339 timestamp>"`), not part of the API.

## Protocol

Same JSON envelope convention as spruce (`{"type": "...", ...flattened fields}`).

### Client → server (`ClientMessage`)

| `type` | Extra fields |
|---|---|
| `subscribe_trip` | `chateau: string`, plus flattened `QueryTripInformationParams` (see [types-reference.md](types-reference.md)) |
| `unsubscribe_trip` | same |
| `unsubscribe_all_trips` | (none) |
| `ping` | (none) |

There is **no** `update_map`, `subscribe_map_v2`, `subscribe_trajectories`, or `nearby_departures` message type here at all — this service only knows about trip subscriptions.

### Server → client (`ServerMessage`)

| `type` | Shape |
|---|---|
| `initial_trip` | `{"type":"initial_trip","data": TripIntroductionInformation}` |
| `update_trip` | `{"type":"update_trip","data": GtfsRtRefreshData}` |
| `error` | `{"type":"error","message": string}` |
| `pong` | `{"type":"pong"}` |

Behavior mirrors spruce's `/ws/trip` closely: on `subscribe_trip`, an immediate `fetch_trip_information` call produces `initial_trip` or `error`; thereafter the server polls the realtime backend for each subscribed trip **every 300ms** and sends `update_trip` only when the underlying stop-time data changes (same hash-based de-dup as spruce). Realtime-poll failures for an already-subscribed trip are silent (no `error` sent) — same caveat as spruce.

## Heartbeat

Identical mechanism to spruce: **WebSocket-protocol** ping every 5 seconds, client force-disconnected if no ping/pong response for more than 10 seconds. The JSON `ping`/`pong` messages are cosmetic and do not reset this timer — see the equivalent warning in [spruce-websocket-api.md](spruce-websocket-api.md#heartbeat-and-idle-timeout).

## Differences from spruce's `/ws/trip` worth knowing if you use both

- No map/viewport, trajectory, or nearby-departures support whatsoever — ramonda's `ClientMessage` enum only has the four variants listed above; there's no map-protocol message to accidentally send in the first place.
- No reconnection/session persistence, same as spruce — all subscription state is actor-local memory, lost on disconnect.
- **Malformed/unparseable JSON is handled differently than on spruce.** Ramonda explicitly catches the `serde_json::from_str::<ClientMessage>` failure and replies with `{"type":"error","message":"Invalid message structure: <serde error detail>"}` — a real, JSON-parser-derived error message, not spruce's generic "wrong endpoint" text. This also means the raw serde error string (which may reveal internal field names) is echoed back to the client. The connection is not closed either way.
