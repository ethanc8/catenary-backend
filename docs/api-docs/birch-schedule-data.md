# birch: static schedule data

Server: **birch**, `http://127.0.0.1:17419` (production: `birch.catenarymaps.org`, unconfirmed — see [README.md](README.md)). All endpoints below are `GET` unless noted, and are registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs).

These endpoints serve GTFS static schedule data straight from Postgres. None of them require authentication or set a `Cache-Control` header unless stated. See [types-reference.md](types-reference.md) for the shared `Route`, `Agency`, `Chateau`, etc. types.

## `GET /getchateaus`

- Source: [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs) (`chateaus`, ~line 521)
- Purpose: **The canonical way to discover valid chateau IDs.** Returns every chateau as a GeoJSON `FeatureCollection`, one polygon feature per chateau describing its geographic coverage hull.
- Params: none.
- Response: `application/json`, a GeoJSON `FeatureCollection`. Each feature's `geometry` is the chateau's coverage `MultiPolygon` (coordinates truncated to 7 decimal places), and `properties` is `{ "chateau": string, "realtime_feeds": string[], "schedule_feeds": string[], "languages_avaliable": string[] }`. The feature's `id` is also the chateau string.
- Caching: `Cache-Control: max-age=60, public`. Also cached in-process (in an `RwLock`) for up to 1 hour before recomputing from Postgres, so very fresh chateau changes may take up to an hour to appear even past the HTTP cache.
- Footguns: chateau IDs are arbitrary `~`-separated slugs, often derived from mangled/de-accented agency names in non-English languages (e.g. `île~de~france~mobilités`, `pražskáintegrovanádoprava`) — don't assume ASCII or a fixed format when parsing/storing them.

## `GET /getchateausnogeom`

- Source: `src/birch/server.rs` (`chateaus_no_geom`, ~line 673)
- Purpose: Same data as `/getchateaus` but without the (often large) geometry — a plain JSON array instead of GeoJSON, for clients that only need the chateau ID/feed lists.
- Response: `application/json` array of `{ "chateau": string, "realtime_feeds": string[], "schedule_feeds": string[], "languages_avaliable": string[] }`, sorted by chateau ID. Not cached in-process (recomputed from Postgres every call), but sets `Cache-Control: max-age=60, public`.

## `GET /route_info` and `GET /route_info_v2`

- Source: [`src/birch/route_info.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/route_info.rs) (`route_info` line 64, `route_info_v2` line 576)
- Purpose: Full metadata for one route within one chateau — agency info, every direction pattern with its ordered stop sequence, every stop used (including parent stations), route-level alerts, a bounding box, and "connecting" routes/transfers at nearby stops.
- Query params: `chateau: String` (required), `route_id: String` (required — URL-decoded server-side).
- Response (`application/json`):
  - `agency_name`, `agency_id`, `short_name`, `long_name`, `url`, `color`, `text_color`: `Option<String>`
  - `route_type: i16`
  - `pdf_url: Option<String>` — **always `null`**; unimplemented stub, don't treat this as "no PDF exists," it's simply never populated.
  - `stops: HashMap<String, SerializableStop>` (see [types-reference.md](types-reference.md)) keyed by stop_id — includes parent stations of visited stops even if the route never stops there directly.
  - `direction_patterns: BTreeMap<String, DirectionsSummary>` keyed by `direction_pattern_id`, where `DirectionsSummary = { direction_pattern: DirectionPatternMeta, rows: Vec<DirectionPatternRow> }` (rows sorted by `stop_sequence`).
  - `shapes_polyline: BTreeMap<String, String>` (**v1 only**) — shape_id → encoded polyline (precision 5). **v2 replaces this with `shape_ids: Vec<String>`** (just the IDs, no geometry — fetch geometry separately via `/get_shape`/`/get_shapes`, see [birch-maps-and-tiles.md](birch-maps-and-tiles.md)). v2 still queries the full shapes internally to compute the bounding box, so it isn't meaningfully cheaper on the database side — only the response payload is smaller.
  - `alert_ids_for_this_route: Vec<String>`, `alert_id_to_alert: BTreeMap<String, AspenisedAlert>` — **route-level alerts only**. `stop_id_to_alert_ids` is defined but **effectively always empty** — the underlying realtime RPC (`get_alert_from_stop_ids`) is a server-side stub that unconditionally returns `None`. Do not rely on this endpoint for per-stop alerts (e.g. "elevator out at this station").
  - `onestop_feed_id: String`
  - `bounding_box: Option<geo::Rect<f64>>` — computed from all shape points and stop coordinates; `x` = longitude, `y` = latitude. The exact JSON key names (`min`/`max`?) weren't independently confirmed from the vendored source for this doc — check a live response before hard-coding a parser against it.
  - `connecting_routes: Option<BTreeMap<String, BTreeMap<String, Route>>>` (chateau → route_id → `Route`) — **in practice always `Some(...)`**, even when empty, so `null` vs `{}` don't distinguish "not computed" from "computed but empty" here.
  - `connections_per_stop: Option<BTreeMap<String, BTreeMap<String, Vec<String>>>>` (stop_id → chateau → route_ids) — unlike the field above, this one genuinely uses `None` when empty. The two "twin" optional fields have **inconsistent `Option` semantics** — don't assume they behave the same way.
- Status codes: `200` on success. **Route not found returns `500`, not `404`** (body `"Error finding route"`) — treat any `500` from this endpoint as potentially "the route_id/chateau combination doesn't exist," not necessarily a server fault. Numerous internal DB queries use bare `.unwrap()`, so a transient Postgres hiccup can also panic the request into a generic `500`.
- Caching: no `Cache-Control` header.
- Notable behavior/footguns:
  - `route_id` is URL-decoded with `.unwrap()` — malformed percent-encoding panics the request.
  - If the chateau's realtime backend is down or unassigned, alert fields silently degrade to empty — no error, no `null`, just empty collections. You cannot tell "no alerts" from "realtime unavailable" from this endpoint alone.
  - "Connecting routes" is a **geographic-proximity heuristic**, not authoritative GTFS `transfers.txt` data: candidate stops within ~667m (0.006°) are found via PostGIS, then filtered by true Haversine distance with mode-specific thresholds (bus 150–200m, tram/subway 200–300m, rail 300–400m) based on both the queried route's mode and each candidate stop's own primary mode. Same-`stop_id` transfers (routes literally sharing a stop) are folded in separately and are more reliable than the geographic heuristic.
  - This algorithm exists as **two independently-maintained copies** — [`src/birch/connections_lookup.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/connections_lookup.rs) (used by this endpoint) and [`src/connections_lookup.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/connections_lookup.rs) (used elsewhere, e.g. by `/ws/trip`'s trip-detail logic). They implement the same algorithm today but aren't guaranteed to stay in sync.

## `GET /getroutesofchateau/{chateau}`

- Source: `src/birch/server.rs` (`routesofchateau`, ~line 245)
- Purpose: All routes belonging to a chateau, no filtering.
- Path param: `chateau: String`.
- Response: `application/json` array of `Route` (see [types-reference.md](types-reference.md)).
- Status codes: effectively always `200` (unknown chateau → `200` with `[]`) — this handler has **no error handling at all** on the DB connection step (bare `.unwrap()`), so a pool exhaustion/DB outage panics the request rather than returning `500`.
- Caching: `Cache-Control: max-age=3600` — the only one of the three `getroutesofchateau*` variants that sets this.

## `POST /getroutesofchateauwithagency/{chateau}` and `POST /getroutesofchateauwithagencyv2`

- Source: `src/birch/server.rs` (`routesofchateauwithagency` ~line 347, `routesofchateauwithagencyv2` ~line 278)
- Purpose: Same as `/getroutesofchateau/{chateau}` but lets the caller filter to specific agencies within a multi-agency chateau.
- Request body: v1 — `{ "agency_filter": string[] | null }` with `chateau` as a path segment; v2 — `{ "chateau": string, "agency_filter": string[] | null }` entirely in the body (no path segment).
- Behavior: if the chateau's routes only ever reference **one** distinct `agency_id` (or none), `agency_filter` is effectively ignored and all routes are returned — the filter only has an effect for genuinely multi-agency chateaux. Routes with no `agency_id` at all are excluded whenever a filter is active (since they can't match any listed agency).
- Response: `application/json` array of `Route`. `Cache-Control: max-age=3600`.

## `GET /get_agencies`

- Source: [`src/birch/get_agencies.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/get_agencies.rs) (`get_agencies_raw`, line 20)
- Purpose: **Every** GTFS agency row, system-wide, sorted by name.
- Response: `application/json` array of `Agency` (see [types-reference.md](types-reference.md) — note `bbox` is never present, `#[serde(skip)]`).
- Footguns: no pagination, no filter — potentially a large payload. If you only need one chateau's agencies, use the endpoint below instead.

## `GET /get_agencies_for_chateau`

- Source: `src/birch/get_agencies.rs` (`get_agencies_for_chateau`, line 57)
- Query params: `chateau: String` (required).
- Response: `application/json` array of `Agency`.
- Status codes: `200` with `[]` for an unknown/nonexistent chateau (not `404`); `400` if `chateau` is missing/malformed.

## `GET /feed_metadata`

- Source: [`src/birch/feed_metadata.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/feed_metadata.rs) (line 14)
- Purpose: Ops/debug-style endpoint showing static-GTFS ingestion history for one feed.
- Query params: `feed_id: String` (required) — **this is the Onestop feed ID (`onestop_feed_id`), not a chateau ID** — a different ID namespace than most other endpoints in this doc.
- Response: `application/json`, `{ "ingested_static": IngestedStatic[], "static_download_attempts": StaticDownloadAttempt[] }` (see field lists in the source; both are ingestion-pipeline audit records — timestamps are Unix **milliseconds**, dates are `"YYYY-MM-DD"` strings).
- Status codes: `200` even for a nonexistent `feed_id` (both arrays empty) — and, notably, `200` with empty arrays **even if the underlying database query itself errors**, since query errors are swallowed via `.unwrap_or_else(|_| vec![])`. The only real `500` is for a Postgres *connection* failure (`{"error": "Database connection error"}`). This means a real DB problem on the query itself is indistinguishable from "this feed simply has no data" — unlike most other endpoints in this API.

## `GET /get_block`

- Source: [`src/birch/block_api.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/block_api.rs) (line 55)
- Purpose: Given a GTFS `block_id` and a service date, return the ordered list of scheduled trips making up that vehicle's full run for the day (useful for a "full day of this vehicle" view), with computed absolute start/end times and the full `Route` for every route touched.
- Query params: `chateau: String` (required), `block_id: String` (required), `service_date: String` (required) — **flexible format**: all non-numeric characters are stripped before parsing as `%Y%m%d`, so `"2026-08-30"`, `"2026/08/30"`, and `"20260830"` all work identically.
- Response (`application/json`):
  - `trips: Vec<TripInBlock>`, sorted ascending by `start_time`. Each: `start_time`/`end_time: u64` (**absolute Unix seconds**, not GTFS seconds-since-midnight — the conversion to real wall-clock time is already done for you), `first_stop_name`/`last_stop_name: String` (`""` if unnamed), `timezone_start`/`timezone_end: String`, `trip_id`, `route_id`, `trip_short_name: Option<CompactString>`, `trip_headsign: Option<String>`, `stop_count: usize`.
  - `routes: HashMap<String, Route>` keyed by `route_id`, containing only routes actually used by trips in the block that are active on `service_date`.
- Status codes: `200` on success; `404` (body `"No trips found for block"`) if the `chateau`+`block_id` combination doesn't exist at all — this is the one endpoint in this group that correctly distinguishes "not found" from a server error; `400` for an unparseable `service_date`; `500` for a Postgres connection failure.
- Footguns:
  - Passing a `service_date` on which none of the block's trips are actually running (per GTFS calendar/calendar_dates) still returns `200` with `trips: []`, **not** `404` — the 404 check only covers "does this block_id exist at all," not "is it running on this date."
  - Can panic (index-out-of-bounds or timezone-parse failure) if the block's itinerary-pattern metadata is missing or has an invalid IANA timezone string — an internal data-consistency issue that surfaces as a crashed request rather than a clean error.

## `GET /chateau_gtfs_rt_counts/{chateau}` and `GET /chateau_authoritative_counts/{chateau}`

- Source: `src/birch/server.rs` (~lines 1158, 1185)
- Purpose: Debug/ops counters — the number of GTFS-RT entities currently held for a chateau, and separately, counts from the "authoritative" backing stores.
- Path param: `chateau: String`.
- Response: `application/json`, shape defined by the internal aspen RPC (`get_gtfs_rt_entity_counts` / `get_authoritative_store_counts`) — not independently modeled here; treat as an opaque debug object.
- Status codes: `404 Not Found` (body `"Chateau or data not found"`) if the chateau has no assigned realtime node, or the RPC call fails/returns nothing. This is one of the few endpoints that uses `404` consistently for "chateau unavailable."
