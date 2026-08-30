# birch: vehicle history

Server: **birch**, `http://127.0.0.1:17419`. Source: [`src/birch/vehicle_history_lookup.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/vehicle_history_lookup.rs). Registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs).

Both endpoints expose **historical** (already-completed) vehicle-to-trip assignment records — "which physical vehicle ran which trip/route on which date" — backed by the `basic_vehicle_history` Postgres table (see [types-reference.md](types-reference.md) for `BasicVehicleHistory`), enriched with static-GTFS trip/route metadata. Neither talks to the realtime backend. Neither sets a `Cache-Control` header. Neither has pagination — a wide or unspecified date range can return unbounded rows.

Errors from both endpoints share one structured shape: `LookupErrorResponse { error: { code: string, message: string } }`, with `code` one of `"bad_request"` (400), `"not_found"` (404), `"conflict"` (409 — an ambiguous resolution, e.g. a vehicle label existing under multiple unified agencies), `"database_error"` (500, generic message, real error only logged server-side), `"internal_error"` (500, e.g. an invalid stored timezone string).

## `GET /vehicle_history_lookup`

- Purpose: History for a **specific vehicle**, by label, across an optional date range.
- Query params: `vehicle: string` (required — the fleet label/number, not a GTFS vehicle_id). Exactly **one** of these three resolution modes must be provided (else `400`): `chateau` alone; `chateau` + `route_id`; or `unified_agency_id` alone. `start_date`/`end_date: Option<string>` (`YYYY-MM-DD` or `YYYYMMDD`; if both given, `start_date` must be ≤ `end_date`).
- Response: `{ trip_history: RouteHistoryRow[], routes: HashMap<route_id, Route>, agency_timezone: string, agency_name: string }`.
  - `RouteHistoryRow = { operation_date: NaiveDate ("YYYY-MM-DD"), unix_start_time: Option<u64> (Unix seconds — resolved from GTFS start_time via the agency's real timezone, DST-safe; null if no matching static trip metadata was found), trip_id, route_id: string, trip_short_name: Option<string>, direction_headsign: Option<string>, block_id: Option<string> }`, sorted descending by `operation_date`, then ascending by `unix_start_time` (nulls sort last), then by `trip_id`.
  - **`routes` is keyed by bare `route_id`, not `(chateau, route_id)`.** If two different feeds happen to reuse the same simple route_id string (e.g. `"1"`), only the first-inserted route wins in this map and the other is silently dropped. Be careful cross-referencing `trip_history[].route_id` against this map if your query could span multiple chateaus/feeds with colliding route IDs.
- Only **production, non-deleted** static ingests are considered — a trip whose static feed was later deleted/superseded will have `trip_short_name`/`direction_headsign`/`block_id`/`unix_start_time` come back `null` rather than erroring.
- Duplicate raw rows sharing `(operation_date, vehicle_label, trip_id, route_id, block_id)` are collapsed to one.

## `GET /vehicle_history_of_route`

- Purpose: History for a **specific route** (chateau + route_id) — the inverse of the above.
- Query params: `chateau: string` (required), `route_id: string` (required), `start_date`/`end_date: Option<string>` (same format/rules as above).
- Response: `{ trip_history: VehicleHistoryOfRouteRow[], agency_timezone: string, agency_name: string }` — **no `routes` map** (only one route is implied by the request).
  - `VehicleHistoryOfRouteRow = { operation_date: NaiveDate, vehicle_label: string, trip_id, trip_short_name: Option<string>, direction_headsign: Option<string>, block_id: Option<string> }`.
  - **This row shape is asymmetric with `RouteHistoryRow` above** — despite the conceptual similarity, this one has `vehicle_label` per row (since one route → many vehicles) and, notably, **has no `unix_start_time` field at all**, even though the same underlying enrichment logic computes a start time — it's just never exposed here. Don't expect the two endpoints to be drop-in replacements for each other.
  - Not explicitly re-sorted after building — order follows the SQL query's own ordering (operation_date desc, vehicle_label asc, trip_id asc, route_id asc).
