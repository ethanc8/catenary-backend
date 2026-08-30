# birch: live realtime data

**Public endpoint:** `https://birch.catenarymaps.org/` (there are additional domain names pointing to the same server and port, in order to allow concurrent requests in catenary-web)

**Localhost endpoint:** `https://localhost:17419/`

**Source:** Registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs); implementations in [`src/birch/aspenised_data_over_https.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/aspenised_data_over_https.rs), [`src/birch/get_vehicle_trip_information.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/get_vehicle_trip_information.rs), [`src/birch/vehicle_api.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/vehicle_api.rs), [`src/birch/gtfs_rt_api.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/gtfs_rt_api.rs), and [`src/birch/alerts.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/alerts.rs).

All of these read from the in-memory realtime state held by the internal `aspen` service, reached over tarpc keyed by **chateau ID** (discoverable via `GET /getchateaus`, see [birch-schedule-data.md](birch-schedule-data.md)) — except `/gtfs_rt`, which is keyed by a **separate `feed_id` namespace** (an individual realtime feed URL's identity, not a chateau). See [types-reference.md](types-reference.md) for the `Aspenised*` type definitions referenced throughout.

## Cross-cutting notes for this whole group

- **"Chateau not found"/"realtime backend unreachable" status codes are inconsistent across every endpoint below** — some return `200` with a plain-text body, some `404`, some `500`. There is no universal rule; each endpoint's status codes are listed explicitly. **Do not write a single generic error handler that assumes one status code means "chateau unavailable" across this whole group.**
- **Timestamp units**: most fields are Unix **seconds**; anything named `..._ms` (e.g. `last_updated_time_ms`) is Unix **milliseconds**. Mixing these up is easy since both appear side-by-side in some responses.
- **`schedule_relationship` wire format varies by endpoint**: sometimes the Rust enum's variant name as a JSON string (e.g. `"Cancelled"`) — this is what you get from raw `AspenisedTripUpdate`/`AspenisedVehiclePosition` objects — and sometimes a small integer code (`Option<u8>`) from locally-converted `_Output`-suffixed types. Check which type each endpoint below actually returns.
- **Connection reuse**: only `get_trip_init`/`get_trip_rt_update` reuse tarpc connections via `AspenClientManager`; every other endpoint in this file opens a brand-new TCP+tarpc connection to the relevant aspen node on every single request. This is a real latency/scalability consideration for high-volume callers.
- **Speed/bearing/odometer units**: meters/second, degrees clockwise from true north, and meters respectively (GTFS-RT convention) — see [types-reference.md](types-reference.md#catenaryrtvehicleposition).

## Vehicle positions

### `GET /get_realtime_locations/{chateau_id}/{category}/{last_updated_time_ms}/{existing_fasthash_of_routes}`

- Source: [`src/birch/aspenised_data_over_https.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/aspenised_data_over_https.rs) (line 597)
- Purpose: All live vehicle positions for one chateau, filtered to one mode category, with long-poll-style short-circuiting.
- Path params: `chateau_id: string`; `category: string` — one of `"metro"` (route_types 0,1,5,7,12), `"bus"` (3,11), `"rail"` (2), `"other"` (4,6); `last_updated_time_ms: u64` — pass `0` on first call, else your last-seen value; `existing_fasthash_of_routes: u64` — accepted but does not appear to actually gate anything in the response logic (looks like a currently-inert parameter).
- Response: `GetVehicleLocationsResponse = { vehicle_route_cache: Option<HashMap<string, AspenisedVehicleRouteCache>>, vehicle_positions: HashMap<string, AspenisedVehiclePosition> (raw enum form — not the "_Output" numeric-code variant), hash_of_routes: u64, last_updated_time_ms: u64 }` (milliseconds).
- Status codes: `200` (JSON) on data; `200` with a **plain-text** body `"No assigned node found for this chateau according to etcd database"` if the chateau has no realtime worker assigned — **note this is 200, not 404**, so status-code-only error handling will treat this as success and then fail to parse it as JSON; `200` plain-text `"No realtime data found for this chateau, aspen server returned None"` similarly; `404` (`"Invalid category"`) for an unrecognized category string; `204 No Content` if your `last_updated_time_ms` already matches the server's (nothing new — an efficient "no change" signal); `500` (`"Error connecting to assigned node. Failed to connect to tarpc"`) if the aspen connection fails. An RPC-level error (not simply "no data") from aspen is unguarded and can **panic** the request.
- Caching: `Cache-Control: no-cache` on every branch.

### `POST /bulk_realtime_fetch_v3`

- Source: `src/birch/aspenised_data_over_https.rs` (line 260)
- Purpose: Vehicle positions for **multiple chateaus at once**, split by mode category and bucketed into slippy-map tiles at a fixed zoom per category (metro=8, rail=7, bus=12, other=5), supporting incremental/delta updates so a live map doesn't have to re-fetch everything every poll. This is the bulk/map-oriented counterpart to `/get_realtime_locations`.
- Request body (`BulkFetchParamsV3`): `{ "chateaus": { "<chateau_id>": { "category_params": { "bus"|"metro"|"rail"|"other": { "last_updated_time_ms": u64, "prev_user_min_x"/"max_x"/"min_y"/"max_y": Option<u32> } } } }, "categories": ["metro","bus","rail","other", ...unrecognized strings silently dropped], "bounds_input": { "level5": BoundsInputPerLevel, "level7": ..., "level8": ..., "level12": ... } }`, where `BoundsInputPerLevel = { min_x, max_x, min_y, max_y: u32 }` — these are **slippy-map XYZ tile coordinates at the fixed zoom for each level (5/7/8/12), not lat/lon degrees.**
- Response (`BulkFetchResponseV2`): `{ "chateaus": { "<chateau_id>": { "categories": { "metro"|"bus"|"rail"|"other": EachCategoryPayloadV2 } } } }`, where `EachCategoryPayloadV2 = { vehicle_positions: Option<BTreeMap<tile_x, BTreeMap<tile_y, BTreeMap<vehicle_id, AspenisedVehiclePositionOutput>>>>, last_updated_time_ms: u64, replaces_all: bool, z_level: u8, list_of_agency_ids: Option<string[]> }`. `AspenisedVehiclePositionOutput` is the JSON-friendly converted form (numeric `schedule_relationship`, no `consist` field) — different wire shape than `/get_realtime_locations`'s raw `AspenisedVehiclePosition`.
- Status codes: **always `200`** — chateaus whose realtime backend is unreachable, or that return no data, are simply **omitted from the response map entirely** (there is no per-chateau error field). You cannot distinguish "chateau doesn't exist" from "chateau has zero vehicles right now" except by checking whether the key is present at all vs. present with empty tile maps.
- Caching: `Cache-Control: no-cache`.
- Notable behavior/footguns:
  - **`replaces_all` delta semantics matter and are easy to get wrong client-side.** `replaces_all: true` means "discard your entire local cache for this chateau+category and replace it with what's here." `replaces_all: false` means "here are only the tiles that are *newly* in view since your last-reported bounds" — if a vehicle moves from one already-in-view tile to another tile that was *also* already in view, **no update is sent for it at all**. The server never tells you to remove a vehicle/tile that scrolled out of view — your client must locally evict anything outside its current requested bounds itself.
  - Vehicles reporting exactly `(0.0, 0.0)` are treated as "no GPS fix" and silently excluded from every tile bucket (a legitimate vehicle at true null-island would vanish, but this is vanishingly unlikely in practice).
  - Up to 32 chateaus are fetched concurrently, each opening a fresh tarpc connection — no explicit limit on how many chateaus one request's `chateaus` map can contain.

### `GET /get_rt_of_single_route`

- Source: `src/birch/aspenised_data_over_https.rs` (line 706)
- Purpose: Realtime vehicle positions + trip updates for one route within one chateau, plus supporting schedule metadata, for a route-detail live view.
- Query params (`SingleRouteRtInfo`): `chateau: string` (required), `route_id: string` (required), `last_updated_time_ms: Option<u64>` (matches server's current value → `204`).
- Response (`PerRouteRtInfo`): `{ vehicle_positions: Option<HashMap<string, AspenisedVehiclePositionOutput>>, last_updated_time_ms: u64 (ms), trips_to_trips_compressed: Option<HashMap<trip_id, CompressedTrip>>, itinerary_to_direction_id: Option<HashMap<string,string>>, trip_updates: AspenisedTripUpdate[] (raw enum form — inconsistent with vehicle_positions in the same response, which IS converted), trip_id_to_direction_id: HashMap<trip_id, Option<string>>, trip_id_to_direction_pattern_parent_id: HashMap<trip_id, Option<string>> }`.
- Status codes: `500` on DB connection failure; `200` plain-text `"No assigned node found..."` if chateau has no realtime worker (again 200, not 404); `500` on tarpc connect failure; `204` if `last_updated_time_ms` matches; `200` JSON otherwise.
- **Reliability footgun:** this handler has essentially no error handling beyond the initial connection checks — several raw `.unwrap()`s on DB queries and RPC results, including one that will **panic if any itinerary pattern in the DB has a `NULL` `direction_pattern_id`** — a data-consistency issue in the DB surfaces here as a crashed request, not a clean error.
- Each returned trip update's `stop_time_update` list is truncated to at most **8 upcoming entries** (past-due entries are dropped first, but only when there's more than one entry to begin with — a lone single entry is kept even if it's in the past).
- Caching: `no-cache` on error branches; **no `Cache-Control` header on the success path**.

## Debug/dump endpoints (internal — see warnings)

> These two return the server's **entire in-memory realtime cache** for a chateau, unbounded in size, in **RON (Rusty Object Notation), not JSON** — with no `Content-Type` header set to warn you. They read as debugging/ops tools rather than stable public API surface. Documented here per project decision to cover everything, but treat their response *shape* as unstable and treat calling them from a production client as unusual.

### `GET /fetch_full_trip_updates_dataset`

- Source: `src/birch/aspenised_data_over_https.rs` (line 949)
- Query params: `chateau: string` (required).
- Response: `ron::ser::to_string_pretty` of the entire `AspenisedData` struct for that chateau (see [types-reference.md](types-reference.md) for its rough shape) — every vehicle position, every trip update, every alert, internal lookup caches, an R-tree spatial index, all of it. **This is RON text, not JSON** — parse it with a RON library (e.g. Rust's `ron` crate), not `JSON.parse`.
- Status codes: `500` (not 404) if the chateau has no assigned node — inconsistent with this file's sibling endpoints, which mostly use `200` for that condition; `500` on connect/RPC failure.
- Caching: `no-cache` on error paths, none on success.

### `GET /get_all_trajectories`

- Source: `src/birch/aspenised_data_over_https.rs` (line 995)
- Query params: `chateau: string` (required).
- Behavior: internally requests the **entire world** (`bbox: [-180,-90,180,90]`), zoom 20, all 11 known modes, with a hardcoded `client_reference: "ron_dump"` — ignoring any notion of a bounded viewport. This is clearly a debug/export tool, not meant for interactive use; for interactive trajectory data use the WebSocket protocol in [spruce-websocket-api.md](spruce-websocket-api.md) instead.
- Response: RON-serialized array of `AspenisedTrajectory` (see [types-reference.md](types-reference.md)) — again RON, not JSON, no distinguishing `Content-Type`.
- Status codes: `404` (the one endpoint in this file that correctly uses 404) if no assigned node; `500` on connect/RPC failure.

## Trip detail and refresh

### `GET /get_trip_information/{chateau}/`

- Source: [`src/birch/get_vehicle_trip_information.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/get_vehicle_trip_information.rs) (line 195); core logic in [`src/trip_logic.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/trip_logic.rs) `fetch_trip_information` (line 455).
- Purpose: The full "trip detail" view — every stop-time (with realtime overlays), route/agency metadata, shape, alerts, live vehicle, and connections. Falls back to a **realtime-only** reconstruction for GTFS-RT ADDED/UNSCHEDULED trips that have no static-schedule counterpart at all.
- Path param: `chateau: string` — **note the trailing slash in the route pattern is required**; omitting it 404s.
- Query params: `QueryTripInformationParams` (see [types-reference.md](types-reference.md)) — `trip_id: string` (required), `start_time`, `start_date`, `route_id` (all optional disambiguators).
- Response: `TripIntroductionInformation` — full field list in [types-reference.md](types-reference.md).
- Status codes: `500` on DB connection/query failure; `500` ("Could not connect to realtime data server") if the trip isn't in the static DB and the chateau has no realtime node, or the realtime connection fails; **`404`** if the underlying error string happens to contain the substring `"not found"` (e.g. `"Trip not found in rt database"`) — **the 404-vs-500 decision is a fragile substring check on an error message**, not a structured error code, so future wording changes to internal error strings could silently change which status code a given failure produces.
- Caching: `Cache-Control: no-cache`, plus a `Server-Timing` header exposing internal pipeline stage names (`open_pg_connection`, `query_compressed_trip`, `connect_to_etcd`, etc.) — useful for perf debugging, but exposes implementation detail to any caller.
- Footguns: fields available differ significantly for the "RT-only" fallback path (`trip_id_found_in_db: false`, `schedule_trip_exists: false`, many fields absent) vs. a normal static-schedule trip — don't assume a uniformly-populated response. Opens up to three separate Postgres connections per request.

### `GET /get_trip_information_rt_update/{chateau}/`

- Source: `src/birch/get_vehicle_trip_information.rs` (line 169); core logic `src/trip_logic.rs` `fetch_trip_rt_update` (line 281).
- Purpose: A lightweight "just refresh the realtime part" companion to the endpoint above, meant to be polled repeatedly by a client that already has the static trip detail.
- Query params: same `QueryTripInformationParams` (trailing slash required in the path here too).
- Response: `ResponseForGtfsRtRefresh { found_data: bool, data: Option<GtfsRtRefreshData> }` (see [types-reference.md](types-reference.md)).
- Status codes: **no 404 distinction at all here** (unlike the endpoint above) — any failure (etcd/aspen unreachable, RPC transport error) is `500` with the raw error string as the body. Critically: **a trip simply having no current realtime update is *not* an error** — that's a normal `200` with `found_data: false, data: null`. So `500` here specifically means "couldn't reach the realtime backend," while "no live data for this trip right now" is `200`.
- Notable: when multiple RT trip-updates match the same `trip_id` (duplicated/looped trips), disambiguates by `start_time`+`start_date` if you supplied them, else falls back to an arbitrary first match — pass `start_time`/`start_date` if precision matters for a repeating trip_id.

## Vehicle lookup

### `GET /get_vehicle_information/{chateau}/{gtfs_rt_id}`

- Source: `src/birch/get_vehicle_trip_information.rs` (line 110)
- Purpose: Look up one live vehicle by its GTFS-RT `VehicleDescriptor.id` within a chateau.
- Response: `ResponseForGtfsVehicle { found_data: bool, data: Option<Vec<AspenisedVehiclePosition>>> }` (0 or 1 element; **raw enum form**, not the `_Output` variant — different wire shape from `/bulk_realtime_fetch_v3`/`/get_rt_of_single_route`).
- Status codes: `200` with `found_data: false` for a vehicle that genuinely doesn't exist right now (aspen returned `Ok(None)`); `500` (`"Could not connect to assigned node"`) for **every** connection-level failure (chateau not assigned, connect failure, RPC error) — these are not distinguished from each other in the response body.
- Caching: none.

### `GET /get_vehicle_information_from_label/{chateau}/{vehicle_label}`

- Source: `src/birch/get_vehicle_trip_information.rs` (line 56)
- Purpose: Same as above, but by human-readable vehicle label/fleet number instead of the GTFS-RT internal ID.
- Same response shape, status codes, and caching as `/get_vehicle_information`.
- Footgun: for some feeds, `AspenisedVehicleDescriptor.label` is simply a copy of `.id` (a server-side normalization for feeds that don't supply a real label) — so "look up by label" and "look up by id" can be identical or different depending on the specific feed, with no way to tell from this endpoint alone which case you're in.

### `GET /get_vehicle`

- Source: [`src/birch/vehicle_api.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/vehicle_api.rs) (line 56)
- **This is not a realtime/live-position endpoint, despite sitting alongside the others and being named "get_vehicle."** It looks up **static fleet-catalog metadata** (manufacturer, model, year, etc.) from Postgres by matching a vehicle's fleet number against per-agency numeric ranges. It never talks to aspen/tarpc and returns no position or trip data. This is probably the single biggest naming footgun in this group — confirm you actually want this endpoint before integrating against it.
- Query params: `label: string` (required — a fleet number, parsing rules vary by agency, see below), `chateau: string` (required — a **hardcoded string switch specific to this endpoint**, using its own ad-hoc spellings like `"metro~losangeles"`, `"rseaudetransportdelacapitalertc"` (Réseau de transport de la Capitale, de-accented) — **these do not necessarily match the canonical chateau IDs from `GET /getchateaus`**), `route_id: Option<string>` (only consulted for `chateau == "san-francisco-bay-area"`, to disambiguate that region's compound `AGENCY:route` route IDs).
- Response: `{ found_data: bool, vehicle: Option<VehicleEntry> }` (see [types-reference.md](types-reference.md)).
- Status codes: mostly `200` with `found_data` indicating success (including on many internal DB errors, which are wrapped into `found_data: false` rather than a real error status); `500` for some DB errors inside the lookup helper; `400` if `label` fails to parse as an integer for chateaus that require purely-numeric labels.
- Footguns: for `"metro~losangeles"`, a non-numeric `label` without a dash **panics** (`label.split("-").next().unwrap().parse::<i32>().unwrap()`); `"foothilltransit"` and `"nyct"` silently strip all non-digit characters from `label` (e.g. `"Bus #452"` → `"452"`) rather than rejecting malformed input; an unrecognized `chateau` string returns `200` with `found_data: false`, indistinguishable from "chateau recognized but vehicle not found."

## Alerts

### `GET /fetchalertsofchateau/`

- Source: [`src/birch/alerts.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/alerts.rs) (line 24)
- Purpose: All currently-active GTFS-RT service alerts for a chateau, "hydrated" with the actual `Route`/`Stop` records each alert's `informed_entity` references (so you don't need a second round-trip to show route/stop names in an alert banner).
- Query params: `chateau: string` (required). Note the route itself has a trailing slash.
- Response: `AlertsResponse = { alerts: HashMap<alert_id, AspenisedAlert> (raw form — cause/effect are raw GTFS-RT integer codes, not translated), routes: HashMap<route_id, Route> (only routes actually referenced by an alert), stops: HashMap<stop_id, SerializableStop> (only stops actually referenced by an alert) }`.
- Status codes: `500` on etcd connection/query failure; `200` plain-text `"No assigned node found for this chateau"` if unassigned (again 200, not 404); `200` with all-empty maps if aspen returns no alerts (indistinguishable from certain other edge cases, but the request itself succeeded either way); `500` (`"Error fetching alerts from aspen"`) on an RPC-level error.
- Caching: `no-cache` on error/not-found branches; the success branches set `Content-Type: application/json` but **no `Cache-Control`** — inconsistent with the error paths.
- Footgun: **this is the only endpoint in the whole realtime group that talks to etcd directly on every request** instead of using the pre-warmed in-memory chateau-assignment cache every other endpoint uses — meaning it has strictly higher per-request latency, and could theoretically (briefly) disagree with other endpoints about which node currently owns a chateau if etcd and the cache's watch stream are momentarily out of sync.

## Raw GTFS-Realtime feed passthrough

### `GET /gtfs_rt`

- Source: [`src/birch/gtfs_rt_api.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/gtfs_rt_api.rs) (line 23)
- Purpose: The closest thing to a standard GTFS-Realtime feed in this API — re-serves one raw feed exactly as aspen currently holds it, with optional conversion to JSON/RON for debugging.
- Query params: `feed_id: string` (required — **a realtime-feed ID, looked up via a separate etcd keyspace (`/aspen_assigned_realtime_feed_ids/{feed_id}`) — this is NOT a chateau_id**, since one chateau can bundle multiple realtime feeds; this is the only endpoint in this whole API keyed this way), `feed_type: string` (required — `"vehicle"`, `"trip"`, or `"alert"`; anything else → `404`), `format: Option<string>` (`"pb"` default, `"json"`, or `"ron"`; unrecognized values silently fall back to `"pb"`).
- Response: `format=pb` → raw `gtfs_realtime::FeedMessage` protobuf bytes (**no `Content-Type` header is set** — parse with any standard GTFS-RT protobuf library against `google.transit.realtime.FeedMessage`); `format=json` → pretty-printed JSON of the same `FeedMessage`; `format=ron` → RON text, `Content-Type: text/plain`.
- Status codes: `500` ("Could not connect to etcd") on etcd failure; `500` ("Could not find Assigned Node") if the feed_id has no assigned node — **arguably should be 404, but is coded as 500**; `404` ("Bad Feed Type...") for an invalid `feed_type`; `500` ("Node crashed during request") on RPC error; `500` ("Data doesn't exist on node. try again in a few minutes?") if aspen has no data yet — **this is the one place in the whole API that explicitly suggests the client retry later**, worth treating as a transient/expected condition rather than a hard failure; `500` ("Failed to decode protobuf data") on data corruption.
- Caching: `Cache-Control: no-cache` on every branch.
- Footgun: `spawn_aspen_client_from_ip(...).await.unwrap()` — a stale/unreachable node socket (e.g. crashed but not yet expired from etcd) **panics** the request rather than returning a graceful error, unlike most sibling endpoints in this file.
