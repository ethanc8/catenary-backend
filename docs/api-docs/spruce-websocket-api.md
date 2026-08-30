# spruce: live map WebSocket API

**Public endpoint:** `wss://spruce.catenarymaps.org/`

**Localhost endpoint:** `ws://127.0.0.1:52771/`

**Source:** [`src/spruce/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/spruce/main.rs)

This is the WebSocket API a live transit map frontend uses — per-trip live updates, viewport-based live vehicle locations, trajectories, and nearby-departures — built on `actix` + `actix-web-actors`.

## Routes

| Route | Actor | Purpose |
|---|---|---|
| `GET /ws/trip`, `/ws/trip/` | `TripWebSocket` ([`src/spruce/trip_websocket.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/spruce/trip_websocket.rs)) | (Deprecated) Subscribe to live updates for specific trips. |
| `GET /ws/`, `/ws/live`, `/ws/live/` | `LiveLocationsWebSocket` ([`src/spruce/live_websocket.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/spruce/live_websocket.rs)) | Viewport-based live vehicle locations + trajectories, for the map view itself. |
| `GET /nearbydeparturesfromcoordsv3` | plain HTTP (not a WebSocket) | Same feature as birch's identically-named endpoint — see [birch-departures.md](birch-departures.md#get-nearbydeparturesfromcoordsv3), independently implemented here. |
| `GET /` | plain HTTP | `"Hello World from Catenary Spruce! <rfc3339 timestamp>"` — a liveness check, not part of the API. |

There is no session/reconnect support anywhere in this API: **all subscription state lives only in the WebSocket actor's memory.** A dropped connection loses everything server-side (on `/ws/live`, disconnecting explicitly unsubscribes from every chateau it was watching) — after reconnecting, a client must resend every `subscribe_*` message it had active before.

## Message envelope

Every client→server message is JSON with a `"type"` discriminator field (Rust `#[serde(tag = "type")]`), and — because the inner params use `#[serde(flatten)]` — **the params are merged directly into the top-level object, not nested under a `"params"` key**. For example, `subscribe_trip` looks like:
```json
{"type": "subscribe_trip", "chateau": "sncf", "trip_id": "12345", "start_time": "08:15:00", "start_date": "20260830", "route_id": "TER"}
```
not `{"type": "subscribe_trip", "params": {...}}`.

### `ClientMessage` variants (send these)

| `type` | Extra fields | Valid on |
|---|---|---|
| `subscribe_trip` | `chateau: string`, plus flattened `QueryTripInformationParams` (see [types-reference.md](types-reference.md)) | `/ws/trip` only |
| `unsubscribe_trip` | same as above | `/ws/trip` only |
| `unsubscribe_all_trips` | (none) | `/ws/trip` only |
| `update_map` | flattened `MapViewportUpdate = { chateaus: string[], categories: string[], bounds_input: BoundsInputV3 }` — the **v1** map protocol | `/ws/live` only |
| `subscribe_map_v2` | flattened `SubscribeMapV2Params = { categories: string[], bounds_input: BoundsInputV3 }` — the **v2** map protocol (server derives chateaus geographically, you don't list them) | `/ws/live` only |
| `unsubscribe_map_v2` | (none) | `/ws/live` only — **there is no `unsubscribe_map` (v1) message at all**; see footguns below |
| `subscribe_trajectories` | flattened `ClientTrajectorySubscriptionParams` (see below) | `/ws/live` only |
| `unsubscribe_trajectories` | (none) | `/ws/live` only |
| `nearby_departures` | flattened `NearbyFromCoordsV3` (same params as [birch-departures.md](birch-departures.md#get-nearbydeparturesfromcoordsv3)), plus `request_id: string` | `/ws/trip` only |
| `ping` | (none) — an application-level ping, distinct from WebSocket protocol ping/pong (see Heartbeat below) | both |

Sending a message type not valid for the endpoint you're connected to — or sending malformed JSON that fails to parse as `ClientMessage` at all — produces the **same generic `error` message** on both endpoints ("This endpoint only supports trip information and routing. Please connect to /ws/live/ for live locations." on `/ws/trip`; "This endpoint only supports live locations (map/trajectory) updates." on `/ws/live`). **The connection is not closed** in either case, and there's no way to distinguish "you sent the wrong message type" from "your JSON didn't parse" from the error text alone.

### `ServerMessage` variants (you receive these)

| `type` | Shape | Sent by |
|---|---|---|
| `initial_trip` | `{"type":"initial_trip","data": TripIntroductionInformation}` | `/ws/trip`, in response to `subscribe_trip` |
| `update_trip` | `{"type":"update_trip","data": GtfsRtRefreshData}` | `/ws/trip`, pushed periodically when a subscribed trip's realtime data changes |
| `error` | `{"type":"error","message": string}` | both, see error conditions above/below |
| `map_update` | **`{"type":"map_update", ...BulkFetchResponseV2 fields flattened directly, no "data" wrapper...}`** | `/ws/live`, pushed on map updates |
| `nearby_departures_chunk` | `{"type":"nearby_departures_chunk","request_id": string,"chunk_index": usize,"total_chunks": usize,"is_hydration": bool,"data": NearbyDeparturesV3Response}` | `/ws/trip`, in response to `nearby_departures` |
| `buffer` | `{"type":"buffer","timestamp": u64,"client_reference": string,"chateau": string,"content": TrajectoryWrapper[],"chunk_index": usize,"total_chunks": usize}` | `/ws/live`, pushed for trajectory subscriptions |
| `pong` | `{"type":"pong"}` | both, in response to a `ping` |

**Footgun:** `map_update` is a "newtype" enum variant, so its JSON does **not** nest under a `data` key the way `initial_trip`/`update_trip` do — `BulkFetchResponseV2`'s own fields (`chateaus: {...}`) are merged straight into the top-level message object. Client code that generically looks for `msg.data` on every message type will break specifically for `map_update`.

## `/ws/trip` — per-trip subscriptions

**Deprecated**. Use [ramonda](./ramonda-websocket-api.md) instead.

- Multiple concurrent trip subscriptions are supported (keyed internally by `(chateau, QueryTripInformationParams)`); there's no limit enforced.
- On `subscribe_trip`, the server immediately fetches the full trip detail (schedule + realtime, via `fetch_trip_information`) and replies with `initial_trip` or `error`. Re-subscribing to an already-subscribed key just re-fetches and resends `initial_trip` — it does not error or dedupe, so sending `subscribe_trip` repeatedly for the same trip is wasteful (each call is a full DB+RPC round trip) but not rejected.
- After the initial fetch, the server polls the realtime backend for that trip **every 300ms** and sends `update_trip` **only when the stop-time data actually changed** (compared via a hash of the serialized `stoptimes` — not the whole payload). If an upstream feed refresh changes only its `timestamp` field with byte-identical `stoptimes`, **no `update_trip` is sent** — don't rely on `update_trip` frequency as a feed-freshness signal.
- **Realtime-poll failures are completely silent** — if the trip's realtime backend is unreachable, no `error` message is ever sent for it; you simply stop receiving `update_trip` for that trip with no indication why.
- `unsubscribe_trip` removes exactly one subscription (silent no-op if it wasn't subscribed); `unsubscribe_all_trips` clears everything.

## `/ws/live` — viewport-based map data + trajectories

### Two independent, coexisting map protocols

There are two ways to subscribe to map vehicle data, and they can conflict:

- **v1 (`update_map`)**: you explicitly list `chateaus: string[]` yourself. Sending `update_map` again **replaces** the whole viewport state wholesale (not additive). **There is no v1 unsubscribe message** — once you've used `update_map`, the only way to clear that state is to disconnect.
- **v2 (`subscribe_map_v2`)**: you send `categories` + `bounds_input` only; the server derives which chateaus to subscribe to by intersecting your tile bounds against a spatial index of chateau coverage areas (`ChateauRTree`, loaded from Postgres at startup). `unsubscribe_map_v2` clears it and unsubscribes from every chateau.
- **If both are active simultaneously, v1 silently wins every tick** — the code checks v1 first and only falls back to v2 if v1 is unset. There's no error or warning if you have both set. **Practical advice: pick one protocol and stick with it for the life of a connection; don't mix `update_map` and `subscribe_map_v2` on the same socket.**

### `BoundsInputV3` — units are tile coordinates, not lat/lon

```
BoundsInputV3 { level5: BoundsInputPerLevel, level7: ..., level8: ..., level12: ... }
BoundsInputPerLevel { min_x, max_x, min_y, max_y: u32 }
```
These are **slippy-map XYZ tile indices**, one bounds rectangle per **fixed** zoom level tied to a vehicle category: `level5`↔"other" (ferries/gondolas), `level7`↔"rail", `level8`↔"metro", `level12`↔"bus". The zoom-to-category mapping is fixed server-side, not client-selectable. All four levels must be present in every `bounds_input` payload (the struct isn't `Option`-wrapped per level) even if you only care about one category — unused levels are simply ignored. **There is no server-side maximum-area/max-zoom-out check** — a client can request the whole world at zoom 12 for buses with nothing rejecting or clamping it.

### Categories

| category string | GTFS `route_type`s included | tile zoom |
|---|---|---|
| `"metro"` | 0, 1, 5, 7, 12 (tram, subway, cable tram, funicular, monorail) | 8 |
| `"bus"` | 3, 11 (bus, trolleybus) | 12 |
| `"rail"` | 2 (rail) | 7 |
| `"other"` | 4, 6 (ferry, gondola) | 5 |

Any other string is silently ignored. A vehicle whose `route_type` isn't in any of these four lists never appears in any map category at all.

### Map update delivery and delta semantics

Under the hood, a pool of `BulkFetchCoordinator` actors (sharded by chateau) polls the realtime backend roughly once per second per subscribed chateau; every 50ms, a coalescer drains whatever's changed and sends you a `map_update`. From your side, updates arrive as irregular pushes (up to ~20/sec) driven by data changes, not a fixed client-driven request/response.

**You must maintain client-side state carefully.** Each category's payload carries a `replaces_all: bool`:
- `true` → discard everything you have cached for this chateau+category and treat this payload as the complete set.
- `false` → this payload contains only vehicles in tiles that are **newly** in view since your last-reported bounds. **The server never explicitly tells you to remove a vehicle that scrolled out of view or moved between two tiles you already had in view** — you must locally evict anything outside your current requested bounds yourself.

See [birch-realtime.md](birch-realtime.md#post-bulk_realtime_fetch_v3) for the full `EachCategoryPayloadV2`/`AspenisedVehiclePositionOutput` shape, which is shared between spruce's `map_update` and birch's `/bulk_realtime_fetch_v3`.

**If a chateau's realtime backend is unreachable, you simply stop receiving `map_update` for it — no `error` message is ever sent for this condition.** A client should implement its own staleness timeout if it needs to detect this.

### Trajectories

- `subscribe_trajectories` takes `ClientTrajectorySubscriptionParams = { bbox: [min_lon, min_lat, max_lon, max_lat] (f64[4], no length validation — a malformed/short array silently defaults missing entries to 0.0, which can produce a nonsensical (0,0)-anchored box with no error), zoom: u8, modes: string[] (matched against AspenisedTrajectory.mode; "bus" is always dropped when zoom<9, regardless of what you requested), precision: Option<u8> (accepted but NOT actually used — server derives coordinate rounding purely from zoom, so this field is currently dead input), client_reference: string (an opaque ID you choose, echoed back on every buffer message) }`. Replaces any prior trajectory subscription wholesale.
- **Trajectories are coupled to your active map viewport.** The set of chateaus queried for trajectories is the same `subscribed_chateaus` set derived from your map subscription (v1 or v2) — **if you subscribe to trajectories without an active map subscription covering the chateaus you care about, you will silently receive no trajectory data at all.** Always establish a map subscription (v1 or v2) before/alongside `subscribe_trajectories`.
- Trajectory support is currently limited to a **hardcoded allowlist of 11 chateaus**: `deutschland, sncf, nationalrailuk, schweiz, île~de~france~mobilités, sncb, tisséo, vbb, busÉireann, nederland, danmark`. Any other chateau returns no trajectories at all, regardless of whether its realtime backend could technically supply them.
- Refreshes every 30 seconds automatically, plus immediately on `subscribe_trajectories` or whenever your subscribed-chateau set changes. `unsubscribe_trajectories` sends one empty `buffer` (`content: [], chunk_index: 0, total_chunks: 0`) per previously-subscribed chateau as an explicit "clear" signal, then stops.
- **A trajectory fetch failure (e.g. backend unreachable, retried 3× internally then given up) results in no `buffer` message at all for that refresh cycle** — again, silent, not an `error` message.
- **Race condition to be aware of:** if a new viewport/subscription change starts before an in-flight trajectory fetch's chunks have all been sent, the server drops the entire stale in-flight response (not just the not-yet-sent chunks) — you can observe a chunk sequence begin (`chunk_index: 0`) and never receive its remaining chunks. Client reassembly should use a short timeout to discard incomplete sequences rather than assuming every started sequence completes.
- `AspenisedTrajectory`/`AspenisedTrajectorySegment`/`AspenisedTrajectoryStop` shapes are in [types-reference.md](types-reference.md).

### Chunking

Two different message types are chunked, with **different reliability characteristics**:

- **`buffer` (trajectories)**: split into chunks of up to 200 items; `total_chunks` is computed correctly from the actual chunk count. Reassemble by grouping on `(client_reference, chateau, timestamp)` and collecting `chunk_index in 0..total_chunks`. An empty sentinel (`content: [], chunk_index: 0, total_chunks: 0`) means "this chateau currently has no trajectories" / "clear what you had," not a malformed sequence.
- **`nearby_departures_chunk`**: **`total_chunks` is hardcoded to the literal `2`, regardless of how many chunks the underlying stream actually produces** (which can range from 0 to 4, depending on how many spatial groups and hydration phases the query yields). **Do not trust `total_chunks` for this message type.** Instead, correlate by `request_id` and treat the stream as "keep applying chunks for this request_id until you stop receiving them" (e.g. with an idle-gap timeout), using `is_hydration` to know whether a given chunk supersedes an earlier non-hydrated chunk for the same request.

## Heartbeat and idle timeout

Identical on both endpoints: the server sends a **WebSocket-protocol** ping frame every 5 seconds; if no ping/pong has been seen from the client for more than 10 seconds, the server force-closes the connection. **The JSON-level `{"type":"ping"}`/`{"type":"pong"}` messages do not reset this timer at all** — only real WS-protocol ping/pong frames do. A client that only ever sends JSON `ping` messages (and doesn't respond to/initiate WS-level pings, which most WebSocket client libraries do automatically) will still be disconnected after ~10-15 seconds of WS-level silence. Treat the JSON ping/pong as a latency-measurement nicety, not a substitute for transport-level keepalive.

## What produces an `error` message

Only these conditions ever produce `ServerMessage::Error` — everything else (realtime backend down during periodic polling, map data unavailable, trajectory fetch failure) is silent, as called out above:

1. `/ws/trip`: `subscribe_trip`'s initial fetch failed (bad trip_id, DB error, realtime backend unreachable, invalid start_time/date, etc.) — message text is a human-readable string from the underlying fetch logic.
2. Either endpoint: sending a message type invalid for that endpoint, or malformed/unparseable JSON — generic endpoint-mismatch text (see Message envelope section above). **The connection stays open** in this case.

## Other footguns

- Binary WebSocket frames are simply echoed back verbatim on both endpoints — not part of the JSON protocol, presumably vestigial, don't rely on it.
- No rate limiting of client messages anywhere — nothing stops a client from spamming `subscribe_trip`/`update_map`/`subscribe_map_v2`, each of which triggers real DB/RPC work server-side.
- Vehicle-tile bucketing uses a **fixed** zoom per category (8/7/12/5) regardless of your actual map zoom — a client must maintain up to four independently-zoomed tile-bound calculations (one per requested category), not a single "current viewport."
- Coordinate order is inconsistent within this very API: trajectory segment coordinates and the `bbox` param are `[lon, lat]` array pairs (GeoJSON convention), while `AspenisedTrajectoryStop`/vehicle positions use named `lat`/`lon` (or `latitude`/`longitude`) fields instead.
