# birch: shapes, map tiles, and route geometry export

Server: **birch**, `http://127.0.0.1:17419`. Endpoints registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs); implementations in [`src/birch/shapes.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/shapes.rs), [`src/birch/postgis_download.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/postgis_download.rs), and [`src/birch/export_route_geom.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/export_route_geom.rs).

All Mapbox Vector Tile (MVT) endpoints below are generated with raw PostGIS SQL (`ST_AsMVT`/`ST_AsMVTGeom`), use the standard **XYZ slippy-map tile scheme** (Web Mercator EPSG:3857, origin top-left), and put every feature into a single MVT layer named literally `"data"` regardless of which logical endpoint served it.

> **Known reliability issue, affects most tile endpoints below:** the underlying SQL uses `ST_AsMVT(...)` as an aggregate with no `GROUP BY`. When zero rows match a tile (extremely common — e.g. ocean, or a filtered layer with nothing present in that tile), the aggregate returns a single `NULL` row instead of an empty tile, and the Rust code's `sqlx::Row::get::<Vec<u8>>(...)` **panics** trying to decode that `NULL`. In practice this means **many otherwise-normal "no data in this tile" requests can crash the request into a 500** rather than returning a clean empty tile. This affects `bus_stops`, `station_features`, `rail_stops`, `other_stops`, `unmatched_rail_stops`, `shapes_ferry`, `shapes_local_rail`, `shapes_bus`, `osm_stations`, `osm_stations_ranked`, and the live (non-cached) path of `shapes_intercity_rail`. If you see intermittent 500s from these tile endpoints, this is the most likely cause — treat a 500 from a tile endpoint the same as an empty tile in client code, or retry.

## Shapes as GeoJSON / polyline

### `GET /get_shape`

- Source: [`src/birch/shapes.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/shapes.rs) (line 70)
- Purpose: Fetch one GTFS shape by chateau + shape_id.
- Query params: `chateau: String` (required), `shape_id: String` (required), `min_x`/`min_y`/`max_x`/`max_y: Option<f64>` (all four together or none — lon/lat degrees, WGS84), `simplify: Option<f64>` (tolerance in meters), `format: Option<String>` (`"polyline"` for encoded polyline; anything else, including a typo, **silently falls back to GeoJSON** — there's no validation error for bad `format` values here).
- Response: `application/json`. `format=polyline` → `{"polyline": "<encoded string, precision 5>"}`. Otherwise → a raw GeoJSON `Geometry` (not a Feature): `{"type":"LineString","coordinates":[[lon,lat],...]}`.
- Status codes: `200` (including an **empty** geometry/`""` polyline when the bbox filter excludes the shape — not a `404` in that case); `404` (`"Shape not found"`) only when chateau+shape_id matches zero DB rows; `500` on connection/query failure.
- Footguns:
  - The bbox params are an **intersect/exclude filter, not a clip** — if the shape intersects the box at all, you get the *entire* shape back, not just the portion inside the box, despite the param names suggesting cropping.
  - `simplify` (meters) is converted to a degree tolerance via a flat `/111_111.0` divide with no latitude correction — simplification is measurably more/less aggressive depending on latitude than the "meters" value implies.

### `POST /get_shapes`

- Source: `src/birch/shapes.rs` (line 131)
- Purpose: Batch version of `/get_shape`.
- Query params: same `min_x/min_y/max_x/max_y/simplify/format` as above, applied identically to every shape in the batch (not configurable per item).
- Request body: JSON array of `{ "chateau": string, "shape_ids": string[] }`.
- Response: `format=polyline` → JSON array of `{chateau, shape_id, polyline, color: Option<string>}` (shapes excluded by bbox are just omitted, no per-item marker). Otherwise → a GeoJSON `FeatureCollection`, each feature `properties = {chateau, shape_id, color}`.
- Footguns: **a single bad item aborts the entire batch** with `500` — there's no partial-success response, and results already computed for earlier items in the batch are discarded. No limit on batch size or `shape_ids` per item (other than actix's default body-size limit).

## Vector tiles (MVT)

Every tile family below follows the same pattern: `GET /{name}/{z}/{x}/{y}` returns `application/x-protobuf` MVT bytes, and `GET /{name}` (no coordinates) returns a [TileJSON](https://github.com/mapbox/tilejson-spec) v3 metadata document (`application/json`) describing zoom range and attribute schema. Both z<4 tile requests and the `_meta` endpoints are as noted per family.

### Stop tiles

All four families share one attribute schema per feature (Point geometry): `onestop_feed_id, chateau, attempt_id, gtfs_id, name, displayname, code, gtfs_desc, location_type (smallint), parent_station, zone_id, url, timezone, wheelchair_boarding (smallint), level_id, platform_code, routes (text[]), route_types (smallint[]), children_ids (text[]), children_route_types (smallint[]), osm_station_id (bigint), osm_platform_id (bigint)`. Only stops flagged `allowed_spatial_query = true` are ever included.

| Path | Mode filter (`route_types` or `children_route_types` contains) | Zoom guard | Tile cache | Meta zoom range |
|---|---|---|---|---|
| `/busstops/{z}/{x}/{y}`, meta `/busstops` | 3 (bus), 11 (trolleybus), 200/1700/1500/1702 (extended coach/misc types) | **none** — z=0 queries the whole world | `max-age=1000` | min: null, max: 15 |
| `/station_features/{z}/{x}/{y}`, meta `/station_features` | `location_type` 2, 3, or 4 (entrance/exit, generic node, boarding area) | **none** | `max-age=1000` | min: 7, max: 19 |
| `/railstops/{z}/{x}/{y}`, meta `/railstops` | 0 (tram), 1 (subway), 2 (rail), 5 (cable car), 12 (monorail) | `z<4` → `400` | `max-age=1000` | min: null, max: 15 |
| `/otherstops/{z}/{x}/{y}`, meta `/otherstops` | 4 (ferry), 6 (gondola), 7 (funicular) | `z<4` → `400` | `max-age=1000` | min: null, max: 15 |
| `/unmatched_railstops/{z}/{x}/{y}`, meta `/unmatched_railstops` | rail modes (as above) AND `osm_station_id IS NULL`, excluding SNCF `StopArea:*` stops (a deliberate data-quality carve-out) | `z<4` → `400` | `max-age=1000` | min: null, max: 15 |

Footgun: `bus_stops` and `station_features` are missing the `z<4` low-zoom guard that every sibling tile endpoint has, so a client can request the entire world's bus stops in a single z=0/1/2/3 tile.

### Shape tiles

Shared attribute schema (LineString geometry): `color, text_color, shape_id, onestop_feed_id, routes (text[]), route_type (smallint), route_label, chateau, stop_to_stop_generated (boolean)`.

| Path | route_type filter | Zoom guard | Cache-Control (tile) | Notes |
|---|---|---|---|---|
| `/shapes_intercity_rail/{z}/{x}/{y}`, meta `/shapes_intercity_rail` | `= 2` | `z<4` → `400` | z4:10000s, z5:5000s, z6:2000s, else 1000s (cache-hit path: 600s) | **The only shape/stop layer with a database-backed tile cache** (table `gtfs.tile_storage`, z≤11 only; z≥12 is always computed live). See "Tile cache" below. Meta document's `name` field is mislabeled `"shapes_local_rail"` (copy-paste bug — cosmetic only). |
| `/shapes_ferry/{z}/{x}/{y}`, meta `/shapes_ferry` | `= 4` | `z<4` → `400` | z4:36000s, z5:10000s, z6:2000s, else 1000s | No tile cache. Meta document's `name` field is mislabeled `"shapes_not_bus"` (copy-paste bug). |
| `/shapes_local_rail/{z}/{x}/{y}`, meta `/shapes_local_rail` | in `(0,1,5,7,11,12)` (tram, subway, cable car, funicular, trolleybus, monorail) | `z<4` → `400` | z4-6:1000s, else 500s | No tile cache. Note trolleybus (11) is classified as "local rail" here but also as "bus" in `/busstops` — an inconsistent mode taxonomy across endpoints. |
| `/shapes_bus/{z}/{x}/{y}`, meta `/shapes_bus` | in `(3,11,200)` AND `routes != '{}'`, excluding `chateau IN ('flixbus~europe','flixbus~america','ouibus')` | `z<4` → `400` | **flat `max-age=1000` regardless of zoom** (unlike every sibling) | Long-distance coach operators are deliberately excluded from this layer. |
| `/shapes_not_bus` (meta only) | — | — | `max-age=1000` | **No matching `/shapes_not_bus/{z}/{x}/{y}` tile handler exists anywhere in the server.** This metadata endpoint advertises tile URLs that will 404 if followed. Treat this as dead/aspirational metadata, not a working tile layer. |

Simplification tolerance (in degrees, applied via `ST_Simplify` before projecting) scales with zoom via a shared helper (`tile_width_degrees_from_z`), multiplied by an endpoint-specific constant — e.g. `shapes_bus` uses 0.005 at z6, 0.004 at z7-8, and (perhaps counter-intuitively) the *same, finest* 0.003 coefficient for both z≤5 and z≥9. Like the `simplify` param on `/get_shape`, this is a flat degrees-per-meter approximation with no latitude correction.

### `Tile cache` (only `shapes_intercity_rail`)

For z≤11 requests, the server first checks table `gtfs.tile_storage` (exact `x,y,z,category` match, category `1` = intercity rail) and serves a cached tile immediately (`Cache-Control: max-age=600`) if found. On a cache miss, it computes the tile live and **fires off a background task** (not awaited, errors only logged) to store the result for next time. **There is no visible cache-invalidation call anywhere in this file** — helper functions for it exist elsewhere in the codebase but aren't invoked here, so if the ingestion pipeline doesn't call them elsewhere, a stale cached intercity-rail tile could persist indefinitely past its 600s HTTP cache header. The three other cache "categories" defined in the model (`LocalRailOriginal`, `BusOriginal`, `FerryOriginal`) are currently unused by any live tile handler — only intercity rail is actually cached today.

### OSM station tiles (independent of GTFS)

| Path | Filter | Zoom guard | Attributes (Point geometry) | Meta zoom |
|---|---|---|---|---|
| `/osm_stations/{z}/{x}/{y}`, meta `/osm_stations` | bbox only, no mode filter | `z<4` → `400` | `osm_id (bigint), osm_type, name, station_type, railway_tag, mode_type, uic_ref, ref, wikidata, operator, network, level, local_ref, parent_osm_id (bigint), is_derivative (boolean)` | min 4, max 19 |
| `/osm_stations_ranked/{z}/{x}/{y}`, meta `/osm_stations_ranked` | bbox + `allowed_spatial_query=true` + `number_of_associated_stops != 0` + a **zoom-dependent importance threshold** (`importance_level_station <=` 2 at z0-2, 3 at z3-5, 5 at z6-7, 10 at z≥8 — lower values are more important/shown at lower zoom) | `z<4` → `400` | `osm_id, osm_type, run_id (integer), name, station_type, railway_tag, mode_type, uic_ref, wikidata, operator, network, tram/subway/rail (boolean), number_of_associated_stops (integer), platform_count (integer), terminal_route_count (integer), route_span_log (integer), degree_centrality (integer), importance_level_station (smallint), label_min_zoom (smallint), icon_min_zoom (smallint), overshadowed_by_osm_id (bigint), overshadowed_by_osm_type, allowed_spatial_query (boolean)` | min 4, max 19 |

Both cache `max-age=10000` on tile and meta.

## `GET /countrailinbox`

- Source: `src/birch/postgis_download.rs` (line 35)
- Purpose: Count distinct rail/metro/tram routes whose shapes intersect a bounding box.
- Query params: `min_x, min_y, max_x, max_y: f64` (required, no validation of min<max or valid lon/lat range).
- Response: `{"intercityrail_shapes": usize, "metro_shapes": usize, "tram_shapes": usize}` (route_type 2, 1, 0 respectively).
- Footgun: **always `200`** — if any of the three underlying queries errors, that count is silently reported as `0` rather than surfacing an error. No bbox size limit — a world-spanning box triggers a full-table scan.

## `GET /export_route_geom`

- Source: [`src/birch/export_route_geom.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/export_route_geom.rs) (line 78)
- Purpose: Export a full route's geometry (all shapes referenced by the route, deduplicated) and optionally its stops, as a **downloadable file** (GeoJSON, KML, or GPX) — this is a "download this route" feature, not a tile/map-data endpoint.
- Query params: `chateau: String` (required, non-blank), `route_id: String` (required, non-blank), `format: Option<String>` (`"geojson"`/`"geo-json"`/`"json"`, `"kml"`, or `"gpx"`, case/whitespace-insensitive; anything else → `400`; defaults to GeoJSON), `include_stops: bool` (default `false`, accepted under **three different query-key aliases**: `include_stops`, `stops`, or `with_stops` — only the literal strings `true`/`false` are accepted, not `1`/`0`).
- Response: sets `Content-Disposition: attachment; filename="<chateau>-<route_id>.<ext>"` — **this is a file download**, not a typical fetch-and-parse JSON response, though `fetch()` from JavaScript still works fine.
  - GeoJSON (`application/geo+json`): `FeatureCollection`; route shapes as `LineString` features (`properties: {feature_type:"route_shape", chateau, route_id, shape_id, name, color:"#RRGGBB", stroke:"#RRGGBB", "stroke-width":4, "stroke-opacity":1.0}`), stops (if requested) as `Point` features (`properties: {feature_type:"stop", chateau, route_id, stop_id, name, color, "marker-color", "marker-size":"small"}`). Coordinates `[lon, lat]`, full `f64` precision.
  - KML (`application/vnd.google-earth.kml+xml`): one `Placemark`/`LineString` per shape and one `Placemark`/`Point` per stop, coordinates truncated to 7 decimal places (~1.1cm), colors converted to KML's `AABBGGRR` order with fixed full opacity.
  - GPX (`application/gpx+xml`): all `<wpt>` (stop) elements before any `<trk>` elements (a GPX 1.1 ordering requirement), same 7-decimal coordinate truncation, custom `gpx_style` extension carrying color/opacity/width.
- Status codes: `200`; `400` (JSON body `{"error": "..."}`, note this differs from the success content-type) for blank `chateau`/`route_id` or invalid `format`; `404` if the route doesn't exist, or resolves to zero usable shapes (fewer than 2 finite points); `500` for DB failures.
- Caching: `Cache-Control: public, max-age=3600`.
- Footguns:
  - Route/shape color falls back silently to **black (`#000000`)** if the route's own color and every candidate shape's color are missing or not valid 6-hex-digit strings — there's no signal in the response that the "real" color was unavailable.
  - Multi-branch routes lose their pattern/direction grouping in the output — only `shape_id` distinguishes geometries; a route with several physically distinct alignments returns them all in one file with no linkage back to which direction/pattern each belongs to.
  - Shape IDs are gathered from the union of `direction_pattern_meta.gtfs_shape_id` *and* the route's own `shapes_list` — this can include shapes belonging to different branches/variants of the route, not just the "main" one.
