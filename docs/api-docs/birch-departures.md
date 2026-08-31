# birch: departures (schedule + realtime)

**Public endpoint:** `https://birch.catenarymaps.org/` (there are additional domain names pointing to the same server and port, in order to allow concurrent requests in catenary-web)

**Localhost endpoint:** `https://localhost:17419/`

**Source:** Registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs); implementations in [`src/birch/departures_at_stop.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/departures_at_stop.rs), [`src/birch/departures_at_osm_station.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/departures_at_osm_station.rs), and (for the geographic version) [`src/birch/nearby_departuresv3.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/nearby_departuresv3.rs). `src/birch/departures_shared.rs` is a shared internal helper module with no HTTP routes of its own.

All three endpoints below merge GTFS static schedule data with live GTFS-Realtime data fetched from the internal `aspen` service per chateau, opening a **new tarpc connection per chateau per request** (no connection pooling for this endpoint family — a latency consideration under load). None set a `Cache-Control` header.

**The single most important thing to know about this whole group:** when a chateau's realtime backend is unreachable — not registered in etcd, connection refused, RPC error, or timeout — **all three endpoints silently fall back to schedule-only data.** There is no field anywhere in any of these responses indicating "realtime was unavailable for this chateau/trip." A `realtime_departure: null` therefore means either "this trip genuinely has no live update yet" or "the whole realtime backend was down when we asked" — the API gives you no way to tell which.

**All returned timestamps are absolute Unix seconds (UTC)** — the server has already resolved GTFS's "seconds since midnight" (which can exceed 86400 for post-midnight trips) plus the correct service-day reference into a real timestamp. You do not need to do midnight-rollover math yourself.

## `GET /departures_at_stop`

- Source: [`src/birch/departures_at_stop.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/departures_at_stop.rs) (line 125)
- Purpose: Upcoming departure/arrival events for one GTFS stop, its parent station, and same-code stops in other chateaus.
- Status codes: essentially always `200` on the happy path. **No graceful "stop not found" handling** — an unknown `stop_id`/`chateau_id` indexes into an empty result and **panics** the request (surfaces as a generic `500`), rather than returning a clean `404`.

### Request

Specified as query string parameters.

```rs
struct NearbyFromStops {
    stop_id: String,
    chateau_id: String,
    // In Unix seconds. Default: now - 3600
    greater_than_time: Option<u64>,
    // In Unix seconds. Default: now + 86400
    less_than_time: Option<u64>,
    // Default: true
    include_shapes: Option<bool>,
}
```

### Response

Returned as JSON.

```rs
struct NearbyFromStopsResponse {
    primary: StopInfoResponse,
    parent: Option<StopInfoResponse>,
    // children_and_related is always empty in /departures_at_stop responses.
    children_and_related: Vec<StopInfoResponse>,
    events: Vec<StopEvent>,
    routes: BTreeMap<String, BTreeMap<String, Route>>,
    pub shapes: BTreeMap<EcoString, BTreeMap<EcoString, String>>,
    pub alerts: BTreeMap<String, BTreeMap<String, catenary::aspen_dataset::AspenisedAlert>>,
    pub agencies: BTreeMap<String, BTreeMap<String, Agency>>,
}
```

Subordinate types:

```rs
struct StopEvent {
    scheduled_arrival: Option<u64>,
    scheduled_departure: Option<u64>,
    realtime_arrival: Option<u64>,
    realtime_departure: Option<u64>,
    trip_modified: bool,
    stop_cancelled: bool,
    trip_cancelled: bool,
    trip_deleted: bool,
    trip_id: String,
    headsign: Option<String>,
    route_id: String,
    chateau: String,
    stop_id: String,
    uses_primary_stop: bool,
    unscheduled_trip: bool,
    moved_info: Option<MovedStopData>,
    platform_string_realtime: Option<String>,
    level_id: Option<String>,
    platform_code: Option<String>,
    vehicle_number: Option<String>,
    trip_short_name: Option<CompactString>,
    service_date: Option<NaiveDate>,
    last_stop: bool,
    scheduled_trip_shape_id: Option<CompactString>,
}

struct StopInfoResponse {
    chateau: String,
    stop_id: String,
    stop_name: String,
    stop_lat: f64,
    stop_lon: f64,
    stop_code: Option<String>,
    level_id: Option<String>,
    platform_code: Option<String>,
    parent_station: Option<String>,
    children_ids: Vec<String>,
    timezone: String,
    stop_name_translations: Option<HashMap<String, String>>,
}
```

## `GET /departures_at_osm_station`

- Source: [`src/birch/departures_at_osm_station.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/departures_at_osm_station.rs) (line 143)
- Purpose: Same concept, keyed by an OSM station ID instead of a GTFS stop_id — aggregates departures across every GTFS stop linked to that OSM station.
- **Special response shapes for the "no direct link" case:**
  - If no GTFS stops are directly linked to `osm_station_id`, the server searches for a *nearby* (within 500m), similarly-named (Jaro-Winkler similarity > 0.8) OSM station that *does* have linked stops, and if found, returns a **completely different JSON shape**: `{"redirect_to_osm_station_id": <i64>}`, still `200`. Callers must check for this key and re-query with the new ID.
  - If no redirect candidate is found either, returns the normal response shape with everything empty.
- Status codes: `200` for all of the above; `500` (`{"error": "Failed to query stops"}`) only if the stops query itself errors. An unknown `osm_station_id` is never `404`.
- Behavioral differences from `/departures_at_stop` worth knowing if you use both:
  - **Unscheduled/"ADDED" trips (GTFS-RT trips with no static-schedule counterpart) are fetched from aspen but never turned into events here** — `/departures_at_stop` does surface these; this endpoint appears to have a gap where the fetch happens but the result is discarded. If you need ADDED-trip visibility for an OSM-station query, you may need to also call `/departures_at_stop` per linked GTFS stop.
  - Sort order differs: this endpoint sorts scheduled-time-first (falling back to realtime), while `/departures_at_stop` sorts realtime-time-first (falling back to scheduled) — a delayed trip can appear in a different relative position between the two endpoints for the same physical stop.
  - This endpoint has an extra **dedup pass** that collapses events sharing the same `(scheduled_departure, route_id, headsign)`, preferring whichever has realtime data — this can incorrectly merge two genuinely different trips (e.g. two same-route, same-time, same-headsign trips from different origins on an interlined/branching service) into one event.
  - Realtime stop-matching is more lenient here (matches by `stop_sequence` OR by an underscore-platform-suffix heuristic, e.g. RT `"8833001_7"` matching scheduled `"8833001"`) than `/departures_at_stop` (exact `stop_id` string match only) — the same underlying feed can produce different match results between the two endpoints.

### Request

Specified as query string parameters.

```rs
pub struct DeparturesAtOsmStationQuery {
    pub osm_station_id: i64,
    // In Unix seconds. Default: now - 3600
    greater_than_time: Option<u64>,
    // In Unix seconds. Default: now + 86400
    less_than_time: Option<u64>,
    // Default: true. Note: even when `false`, the shape fetch/encode work still happens server-side and is only stripped from the JSON at the very end, so setting this to `false` does not actually save any work, contrary to what you'd expect
    pub include_shapes: Option<bool>,
}
```

### Response

- This file's `StopInfoResponse` is a separately-defined (structurally identical) type — but unlike the `/departures_at_stop` version, **`children_ids` actually is populated here**.
- `StopEvent` here has one extra field vs. the other endpoint: `final_station_name: Option<string>` — populated **only** for chateau `"île~de~france~mobilités"` (Paris IDFM), `null` for every other chateau. A hardcoded, undocumented special case.
- `debug: DeparturesAtOsmStationDebug = { total_time_ms, etcd_connection_time_ms (always 0 — dead/misleading field), db_connection_time_ms, initial_osm_query_ms, stop_data_fetch_ms, aspen_data_fetch_ms, event_generation_ms }` — always present in every response (not gated behind a debug flag); this exposes internal timing/implementation detail to every caller.

Returned as JSON.

```rs
struct DeparturesAtOsmStationResponse {
    osm_station: Option<OsmStationInfoForResponse>,
    stops: Vec<StopInfoResponse>,
    events: Vec<StopEvent>,
    routes: BTreeMap<String, BTreeMap<String, catenary::models::Route>>,
    shapes: BTreeMap<EcoString, BTreeMap<EcoString, String>>,
    alerts: BTreeMap<String, BTreeMap<String, catenary::aspen_dataset::AspenisedAlert>>,
    agencies: BTreeMap<String, BTreeMap<String, catenary::models::Agency>>,
    debug: DeparturesAtOsmStationDebug,
}
```

Subordinate types:

```rs
struct StopEvent {
    scheduled_arrival: Option<u64>,
    scheduled_departure: Option<u64>,
    realtime_arrival: Option<u64>,
    realtime_departure: Option<u64>,
    trip_modified: bool,
    stop_cancelled: bool,
    trip_cancelled: bool,
    trip_deleted: bool,
    trip_id: String,
    headsign: Option<String>,
    route_id: String,
    chateau: String,
    stop_id: String,
    uses_primary_stop: bool,
    unscheduled_trip: bool,
    moved_info: Option<MovedStopData>,
    platform_string_realtime: Option<String>,
    level_id: Option<String>,
    platform_code: Option<String>,
    vehicle_number: Option<String>,
    trip_short_name: Option<CompactString>,
    service_date: Option<NaiveDate>,
    last_stop: bool,
    scheduled_trip_shape_id: Option<CompactString>,
    pub final_station_name: Option<String>,
}

struct StopInfoResponse {
    chateau: String,
    stop_id: String,
    stop_name: String,
    stop_lat: f64,
    stop_lon: f64,
    stop_code: Option<String>,
    level_id: Option<String>,
    platform_code: Option<String>,
    parent_station: Option<String>,
    children_ids: Vec<String>,
    timezone: String,
    stop_name_translations: Option<HashMap<String, String>>,
}

pub struct OsmStationInfoForResponse {
    pub osm_id: i64,
    pub osm_type: String,
    pub name: Option<String>,
    pub name_translations: Option<serde_json::Value>,
    pub station_type: Option<String>,
    pub railway_tag: Option<String>,
    pub mode_type: String,
    pub lat: f64,
    pub lon: f64,
}

pub struct DeparturesAtOsmStationDebug {
    pub total_time_ms: u64,
    pub etcd_connection_time_ms: u64,
    pub db_connection_time_ms: u64,
    pub initial_osm_query_ms: u64,
    pub stop_data_fetch_ms: u64,
    pub aspen_data_fetch_ms: u64,
    pub event_generation_ms: u64,
}
```

## Shared behavior (both endpoints above)

- **Time window vs. internal lookahead:** your `greater_than_time`/`less_than_time` window is honored for filtering, but the server always internally searches at least 12 hours and at most 5 days ahead regardless of how narrow your requested window is (clamped: `requested_window + 300s grace`, bounded to `[12h, 5 days]`). A 5-minute window still costs roughly 12h of internal schedule computation.
- **Inclusion filter is an OR across all four timestamps** (`scheduled_arrival`, `scheduled_departure`, `realtime_arrival`, `realtime_departure`) — an event whose *scheduled* times are outside your window but whose *realtime* time (e.g. a large delay) falls inside it will still be included, and vice versa. Don't filter purely on one field client-side and assume it matches what the server included.
- **Chateau-specific calendar lookback** differs: 2 days for `sncb`, `schweiz`, `sncf`, `deutschland`, `nederlandse~spoorwegen`, `nationalrailuk`, `île~de~france~mobilités`; 8 days for `bus~dft~gov~uk` (OSM-station endpoint only); 14 days for everyone else.
- **`StopEvent` fields that are defined but never actually populated by either endpoint** — don't rely on these: `moved_info` (always `null`), `level_id` (always `null`), `platform_code` (always `null` — use `platform_string_realtime` instead, which *is* populated), `trip_modified` (always `false`).
- **`trip_cancelled`/`trip_deleted`/`stop_cancelled`**: `trip_cancelled` comes from GTFS-RT `schedule_relationship == CANCELLED`; `trip_deleted` from `== DELETED`; `stop_cancelled` from either a stop-level RT `Skipped` status or a `NO_SERVICE` alert whose `informed_entity` explicitly names this stop (a route/trip/agency-wide alert with no stop_id never sets `stop_cancelled`, per GTFS semantics). **Handling of `trip_deleted` trips is inconsistent** across the two endpoints and even within one endpoint's own frequency-vs-pinned-time code paths — sometimes deleted trips are excluded from `events` entirely, sometimes included with the flag set. Always check `trip_deleted` yourself rather than assuming the server already filtered deleted trips out.
- **Frequency-based (headway) trips**: for these, one `StopEvent` is synthesized per theoretical departure slot (`start_time..end_time` stepped by `headway_secs`). Realtime matching for a frequency-based trip requires the RT feed to explicitly carry a `start_time` matching one of those theoretical slots — if a feed doesn't populate RT `start_time` for frequency trips, no RT match is possible for them at all.
- **`chateau` IDs come from `GET /getchateaus`** (see [birch-schedule-data.md](birch-schedule-data.md)); a chateau ID typo or one that never resolves to an assigned aspen worker degrades to schedule-only data with no error, as noted at the top of this doc.

## `GET /nearbydeparturesfromcoordsv3`

- Source: [`src/birch/nearby_departuresv3.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/nearby_departuresv3.rs) (line 162)
- Purpose: Given a lat/lon, return upcoming scheduled+realtime departures grouped by nearby stops, split into "long distance" (intercity rail-style groupings) and "local" (bus/metro/tram route groupings), with OSM-clustered stations merged together.
- **Important:** this exact path, `GET /nearbydeparturesfromcoordsv3`, is **also** served independently by **spruce** ([`src/spruce/nearby_departures.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/spruce/nearby_departures.rs), port 52771) — this is not a proxy/alias, it's a **second, independently-maintained implementation** of the same feature on a different server (spruce's version additionally powers the WebSocket `nearby_departures` client message documented in [spruce-websocket-api.md](spruce-websocket-api.md)). The two share the same request struct and most of the algorithm, but diverge in implementation detail (e.g. how they connect to etcd, an extra stop-count/distance cutoff present in one but not the other). **Don't assume calling birch's copy and spruce's copy for the same coordinates always returns byte-identical results.**
- Query params (`NearbyFromCoordsV3`): `lat: f64` (required), `lon: f64` (required), `departure_time: Option<u64>` (Unix seconds, default = server's current time), `radius: Option<f64>` (meters, default `5000.0`), `limit_per_station: Option<usize>` (default `10`), `limit_per_headsign: Option<usize>` (default `20`), `skip_realtime: Option<bool>` (default `false` — if `true`, skips all realtime RPC calls, pure-schedule mode), `rt_timeout_ms: Option<u64>` (default `2000` — timeout for the combined realtime fetch per chateau).
- Response (`application/json`), `NearbyDeparturesV3Response`:
  - `long_distance: Vec<StationDepartureGroupExport>` — each `{ station_name, osm_station_id: Option<i64>, distance_m: f64, departures: DepartureItem[], lat, lon: f64, timezone: string }`.
  - `DepartureItem = { scheduled_departure, realtime_departure, scheduled_arrival, realtime_arrival: Option<u64> (unix seconds), service_date: NaiveDate, headsign: string, platform: Option<string>, trip_id, trip_short_name: Option<string>, route_id, stop_id, agency_id, cancelled: bool, delayed: bool (true if realtime departure is >60s later than scheduled), chateau_id, last_stop: bool, final_station_name: Option<string> (only populated for chateau "île~de~france~mobilités", otherwise always null) }`.
  - `local: Vec<DepartureRouteGroupExportV3>` — each `{ chateau_id, route_id: CompactString, color, text_color, short_name: Option<CompactString>, long_name: Option<string>, route_type: i16, agency_name: Option<string>, headsigns: HashMap<string, LocalDepartureItem[]>, closest_distance: f64 (meters) }`.
  - `LocalDepartureItem = { trip_id: CompactString, departure_schedule, departure_realtime, arrival_schedule, arrival_realtime: Option<u64> (unix seconds), stop_id: CompactString, stop_name: Option<string>, cancelled: bool, platform: Option<string>, service_date: NaiveDate, last_stop: bool }`.
  - `routes: HashMap<chateau, HashMap<route_id, RouteInfoExport>>` where `RouteInfoExport = { short_name, long_name, agency_name, color, text_color: Option<string>, route_type: i32 }`.
  - `stops: HashMap<chateau, HashMap<gtfs_stop_id, StopOutputV3>>` where `StopOutputV3 = { gtfs_id: CompactString, name: string, lat, lon: f64, osm_station_id: Option<i64>, timezone: string }`.
  - `debug: NearbyDeparturesDebug = { total_time_ms, db_connection_time_ms, stops_fetch_time_ms, etcd_connection_time_ms (always 0 in birch's copy — dead field), pipeline_processing_time_ms }` — always included in every response, exposing internal timing to every caller.
- Status codes: `500` only on a DB pool/connection failure or stop-fetch SQL error; otherwise always `200`. **No distinct "chateau not found" error** — chateaus with no assigned realtime node just silently get empty/schedule-only results for their stops (logged server-side, not surfaced to the client). Realtime timeouts (per `rt_timeout_ms`) are likewise silently swallowed, falling back to schedule-only data per chateau.
- Caching: no `Cache-Control` header.
- Notable behavior/footguns:
  - **The "long distance" vs "local" split is driven by a hardcoded chateau-ID whitelist** baked into this handler (`sncf`, `nationalrailuk`, `sncb`, `nederlandse~spoorwegen`, `rejseplanen~dk~gtfs`, `norge`, `sverige`, `lv`, `ztp~krakow`, `deutschland`, `schweiz`, `trenitalia`, `kordis`, `pražskáintegrovanádoprava`, `koleje~dolnoslaskie`, `pkp~intercity~pl`, `renfeoperadora`), with extra per-chateau route_type exceptions for `nationalrailuk` (certain agency codes excluded) and `upexpress` (never long-distance). This list is invisible to API consumers and entirely determines which section a given stop's departures land in.
  - `radius` is **not authoritative** for bus stops: as candidate stop counts grow (past 100/300/500/800), the effective distance cutoff for bus-mode stops (`primary_route_type==3`) is progressively tightened (4000m → 2500m → 1500m → 1000m) regardless of the `radius` you requested. Increasing `radius` in a dense area does not linearly increase bus coverage.
  - Only stops with `allowed_spatial_query = true` are ever considered, regardless of geographic proximity.
  - Realtime service-date matching falls back to a heuristic (inferring the service day from a stop-time's absolute RT timestamp when the feed doesn't supply `start_date`) that can occasionally attach the wrong day's update to a trip near midnight/DST transitions.
  - Up to 3 chateaus are queried concurrently (`buffer_unordered(3)`); a query spanning many chateau boundaries can take noticeably longer as it serializes beyond that.
