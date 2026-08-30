# birch: stop and station search

**Public endpoint:** `https://birch.catenarymaps.org/` (there are additional domain names pointing to the same server and port, in order to allow concurrent requests in catenary-web)

**Localhost endpoint:** `https://localhost:17419/`

**Source:** Registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs); implementations in [`src/birch/stop_preview.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/stop_preview.rs), [`src/birch/osm_station_lookup.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/osm_station_lookup.rs), [`src/birch/osm_station_search.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/osm_station_search.rs), [`src/birch/osm_station_preview.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/osm_station_preview.rs), and [`src/birch/text_search/mod.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/text_search/mod.rs).

None of these set a `Cache-Control` header. Two of them (`osm_station_search`, `text_search_v1`) query **Elasticsearch** in addition to Postgres — text relevance ranking, language handling, and result caps described below only apply to those two.

**Important — read before integrating:** several conceptually-identical objects have multiple, non-identical JSON shapes depending which endpoint returns them:
- **"Stop" objects**: `stop_preview` and `osm_station_preview` share one `StopDeserialised` shape; `text_search_v1` uses a *different* `StopDeserialised` type (same name, different fields — it adds `parent_station` and `agency_names`). Don't assume a "stop" from one endpoint has the same fields as a "stop" from another.
- **"OSM station" objects**: `osm_station_lookup`, `osm_station_preview`, and `osm_station_search` each define their own, different `OsmStationInfo`/`OsmStationSearchResult` shape (different field subsets — e.g. only `osm_station_lookup`'s version includes `uic_ref`/`wikidata`/`operator`/`network`/`level`/`local_ref`).
- **`osm_station_id` type**: a raw `i64` in `osm_station_lookup`'s top-level fields and in `osm_station_preview`'s query param, but a **stringified** `Option<String>` inside every `StopDeserialised.osm_station_id`.
- **Coordinate representation**: stop objects serialize `point` as `geo::Point<f64>` → `{"x": lon, "y": lat}` (longitude first); OSM station objects instead use named `lat`/`lon` scalar fields. Both conventions appear within the very same response in some cases.
- **`name_translations` values are JSON-stringified with stray quotes.** The shared helper `serde_value_to_translated_hashmap` (used by `stop_preview`, `osm_station_preview` for stop objects, and `text_search_v1`) calls `serde_json::Value::to_string()` instead of extracting the string content, so a translation like `"Gare Centrale"` comes back as the literal 16-character string `"\"Gare Centrale\""` (with embedded escaped quotes), not the clean text. This affects every endpoint that returns `name_translations` as a `HashMap<String,String>` (it does **not** affect the raw-JSON `name_translations` fields returned by `osm_station_lookup`/`osm_station_preview`'s top-level OSM-station object, which pass the JSON through unconverted).

See [types-reference.md](types-reference.md) for `Route`/`Agency`/`OsmStation`.

## `POST /stop_preview`

- Source: [`src/birch/stop_preview.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/stop_preview.rs) (line 47)
- Purpose: Batch-fetch full stop details for a set of chateau→stop-id groups, plus every route those stops (and their child/parent stops) serve.
- Request body: `{ "chateaus": { "<chateau>": ["<stop_id>", ...], ... } }`.
- Response: `{ "stops": { "<chateau>": { "<stop_id>": StopDeserialised } }, "routes": { "<chateau>": { "<route_id>": Route } } }`. Child stops of any requested stop are fetched automatically (even if not explicitly requested) so their routes can be inherited onto the parent.
- `StopDeserialised` (this file's version, also used by `osm_station_preview`): `gtfs_id, name: Option<String>, url: Option<String>, timezone: Option<String>, point: Option<geo::Point<f64>>, level_id: Option<String>, primary_route_type: Option<i16>, platform_code: Option<String>, routes: Vec<String>, route_types: Vec<i16>, children_ids: Vec<String>, children_route_types: Vec<i16>, station_feature: bool, wheelchair_boarding: i16, name_translations: Option<HashMap<String,String>>, osm_station_id: Option<String>`.
- Status codes: always `200`, even for unknown chateaus/stop-ids (empty inner maps). **Per-chateau DB errors are logged server-side and that chateau's group is silently omitted from the response** — a client cannot distinguish "no stops matched" from "that chateau's query failed."
- Footguns: null array elements in Postgres array columns are silently dropped, not represented as `null` in the output arrays. Uses a fresh connection per concurrent (max 4 at a time) chateau fetch, with unguarded `.unwrap()`s — a DB pool outage panics the request instead of returning a clean error.

## `GET /osm_station_lookup`

- Source: [`src/birch/osm_station_lookup.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/osm_station_lookup.rs) (line 57)
- Purpose: Reverse lookup — given a GTFS stop, tell me whether/which OSM station it's linked to. Exact match only, no fuzzy search, Postgres only (no Elasticsearch).
- Query params: `chateau_id: String` (required), `gtfs_stop_id: String` (required).
- Response: `{ "found": bool, "gtfs_stop_id": string, "chateau_id": string, "osm_station_id": Option<i64>, "osm_platform_id": Option<i64>, "osm_station_info": Option<OsmStationInfo> }`, where `OsmStationInfo` here = `{ osm_id, osm_type, name, name_translations (raw JSON object, NOT translated-hashmap-converted), station_type, railway_tag, mode_type, uic_ref, ref_, wikidata, operator, network, level, local_ref, lat: f64, lon: f64 }`.
- Status codes: `200` with `found: false` for an unmatched stop (not `404`); `500` (`{"error": "Database connection failed"}` / `{"error": "Failed to query stop"}`) for connection/query failure on the *stop* lookup. If the stop is linked (`osm_station_id: Some(id)`) but the *follow-up* OSM-station-table query errors, the error is masked as `osm_station_info: None` with a normal `200` — it does not surface as `500`.

## `GET /osm_station_search`

- Source: [`src/birch/osm_station_search.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/osm_station_search.rs) (line 45)
- Purpose: Elasticsearch-backed typeahead search over OSM railway/subway/tram/etc. station records (index `osm_stations`), enriched with linked GTFS routes.
- Query params: `text: String` (required). `lang: Option<String>` (tries `name_translations[lang]` then `name_translations["name:"+lang]` for a translated display name). `focus_lat`, `focus_lon`, `focus_weight: Option<f64>` — **accepted but completely unused/dead** (the geo-boost code path they were presumably meant to control is an empty conditional body). Do not document or rely on these as working geo-ranking controls.
- Response: `{ "results": OsmStationSearchResult[] }`, where each result = `{ osm_id, name: Option<string>, point: Option<geo::Point<f64>> ("x"=lon,"y"=lat), mode_type, operator, network, admin_hierarchy: Option<Value>, routes: Route[], confidence: f64 }`. `confidence` is the **raw, un-normalized Elasticsearch `_score`** — don't treat it as a 0–1 probability.
- Behavior worth knowing:
  - Query text is lowercased and has a hardcoded multi-language stop-word list stripped (`station, tube, metro, subway, train, railway, transit, center, centre, bus, stop, hbf, hauptbahnhof, bahnhof, gare, estación, estacion, estação, stazione, terminal, interchange`, plus CJK "station" characters 駅/站/역) before matching — plain substring removal, not word-boundary aware, and falls back to the original text if stripping empties the query. Matching also boosts by mode (`rail`×4.0, `subway`×3.0, `tram`×2.0, else ×1.5).
  - **Result count is hard-capped at 30** (Elasticsearch `size: 30`), with **no pagination parameters at all** — you cannot page past the top 30 hits.
  - **Stations with zero linked GTFS routes are silently excluded from results entirely**, even if they're the best text match — this endpoint effectively only returns OSM stations that are *also* GTFS-linked, despite being named/framed as an OSM search.
- Status codes: `200` (including `{"results": []}`); `500` **plain-text** body (`"ES Error: ..."` / `"ES JSON Error: ..."`) on Elasticsearch failure — inconsistent content-type vs. the JSON success path. Postgres pool failure is an unguarded `.unwrap()` (panics).

## `GET /osm_station_preview`

- Source: [`src/birch/osm_station_preview.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/osm_station_preview.rs) (line 49)
- Purpose: "Detail page" for one OSM station — its own metadata plus every linked GTFS stop (across all chateaus) and every route those stops serve.
- Query params: `osm_station_id: i64` (required).
- Response: `{ "osm_station": Option<OsmStationInfo>, "stops": { "<chateau>": { "<stop_id>": StopDeserialised } }, "routes": { "<chateau>": { "<route_id>": Route } } }`. `OsmStationInfo` here is a **third, different** shape from the other two OSM-info structs: `{ osm_id, osm_type, name, name_translations (raw JSON, not translated-hashmap-converted here), station_type, railway_tag, mode_type, lat: f64, lon: f64 }` (no `uic_ref`/`wikidata`/`operator`/`network`/`level`/`local_ref`). `StopDeserialised` is the *same type* as `/stop_preview`'s (imported directly), so stop-level `name_translations` here *is* run through the lossy `serde_value_to_translated_hashmap` conversion, while the top-level OSM station's own `name_translations` is not — an inconsistency within a single response.
- Status codes: **always `200`**, including `osm_station: null, stops: {}, routes: {}` for a completely nonexistent `osm_station_id` — there is no `404` case at all in this endpoint. `500` only for a connection failure or a query error on the *stops* query specifically; a query error on the *osm_stations* lookup itself is masked as `osm_station: null` with `200`.
- Purely Postgres, no Elasticsearch, exact-match only.

## `GET /text_search_v1`

- Source: [`src/birch/text_search/mod.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/text_search/mod.rs) (line 120)
- Purpose: **The general-purpose "search everything" endpoint** — searches stops and routes (and their agencies) simultaneously via Elasticsearch (indices `stops` and `routes`, queried concurrently), with optional map-viewport-aware re-ranking of stop results. This is distinct from `/osm_station_search`, which is specifically for OSM points-of-interest.
- Query params: `text: String` (required). `user_lat`, `user_lon: Option<f32>` — **accepted but unused/dead**, same footgun pattern as `osm_station_search`'s focus params; do not treat as working. `map_lat`, `map_lon`, `map_z: Option<f32>` — these three **do** work, but only when **all three** are present together (see geo-boost behavior below).
- Response: `{ "stops_section": { "stops": {...StopDeserialised...}, "routes": {...RouteDeserialised...}, "agencies": {...Agency...}, "ranking": StopRankingInfo[] }, "routes_section": { "routes": {...RouteDeserialised...}, "agencies": {...Agency...}, "ranking": RouteRankingInfo[] } }`, all outer maps keyed by chateau then by id. `RouteDeserialised = { ...Route fields flattened..., agency_name: String }` (`""` if the agency couldn't be resolved, not `null`). `StopRankingInfo`/`RouteRankingInfo = { gtfs_id: string, score: f64, chateau: string }`. This file's `StopDeserialised` has **extra fields** vs. `stop_preview`'s: `parent_station: Option<String>`, `agency_names: Vec<String>` (alphabetically sorted).
- Behavior worth knowing:
  - The stops sub-query and the routes sub-query search **different, independently-cleaned text** — stops search the raw query text; routes search a version with "train"/"rail" or "subway"/"metro" substrings stripped (whole-string replace, not word-boundary aware) — so a search that surfaces a stop may not surface the "equivalent" route result, and vice versa.
  - Result size is **hard-capped at 100 per section**, with no pagination and no total-hit-count exposed.
  - Geographic re-ranking (only active when `map_lat`+`map_lon`+`map_z` are **all** present) applies a distance-decay boost to stop scores only — routes are never geo-boosted. The pivot radius/weight is tiered by `map_z`: `>12` → 20km pivot, weight 0.05; `10<z≤12` → 50km, weight 0.02; `z≤10` → 500km, weight 0.01. Note the *finest* zoom tier gets the *largest* weight and the *coarsest* zoom tier gets the *smallest* — double-check this is intentional tuning before relying on it, it isn't obviously documented as such in the code.
  - A **route_type scoring quirk**: when the query text contains "subway"/"metro", the relevance boost for GTFS route_type 0 (**tram**) is set higher (×4.0) than for route_type 1 (**subway**, ×3.0) — searching "subway station" can rank tram routes above subway routes. This looks like an unintentional off-by-one in the scoring script rather than deliberate tuning.
- Status codes: **always `200`** on the happy path; this handler has essentially **no error handling** — DB pool failures, Elasticsearch transport errors, and malformed Elasticsearch JSON responses are all unguarded `.unwrap()`s that panic the request into a generic `500` with no informative body. This is the most panic-prone endpoint in this group.

## Cross-cutting notes for this whole group

- **No endpoint here ever returns `404` for an unrecognized chateau, stop, or OSM-station ID** — every one of them returns `200` with empty/null data instead. Only Postgres/Elasticsearch connection or query failures (inconsistently) produce `500`.
- **No pagination anywhere** in this group — `osm_station_search` caps at 30 results, `text_search_v1` caps at 100 per section, with no way to request more and no indication that more results exist.
