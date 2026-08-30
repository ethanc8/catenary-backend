# birch: third-party proxies and small utility endpoints

Server: **birch**, `http://127.0.0.1:17419`. All registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs). None require authentication; none set `Cache-Control` unless noted.

## Third-party data proxies

These exist so browser map clients can fetch third-party data through Catenary's own domain (avoiding CORS issues and, for the tile proxies, keeping a paid API key server-side).

### `GET /amtrakproxy`

- Source: `src/birch/server.rs` (`amtrakproxy`, ~line 456)
- Purpose: Proxies Amtrak's live train-position feed (`maps.amtrak.com/services/MapDataService/trains/getTrainsData`), which Amtrak serves in an obfuscated/encrypted form; this endpoint decrypts it server-side (via the `amtk` crate) before returning plain JSON.
- Response: `application/json`, Amtrak's own (undocumented-by-Catenary) train-position JSON schema, passed through as decrypted.
- Status codes: `500` (`"Could not fetch Amtrak data"` or `"Could not decrypt Amtrak data"`) on any upstream/decryption failure; `200` otherwise.

### `GET /metrolinktrackproxy`

- Source: `src/birch/server.rs` (`metrolinktrackproxy`, ~line 412)
- Purpose: Proxies Metrolink's (Southern California commuter rail) `StationScheduleList.json`.
- Response: `application/json`, passed through verbatim.
- Status codes: `500` (`"Could not fetch Metrolink data"`) on upstream failure.

### `GET /calfireproxy`

- Source: `src/birch/server.rs` (`calfireproxy`, ~line 435)
- Purpose: Proxies CAL FIRE's active-incidents GeoJSON list (`incidents.fire.ca.gov`), presumably for a wildfire-overlay map layer.
- Response: `application/json`, passed through verbatim.
- Status codes: `500` (`"could not fetch calfire"`) on upstream failure.

### `GET /watchduty_tiles_proxy/{z}/{x}/{y}`

- Source: `src/birch/server.rs` (`proxy_for_watchduty_tiles`, ~line 1124)
- Purpose: Proxies Watch Duty's evacuation-zone vector tiles (`tiles.watchduty.org/maptiles/evac_zones_ca`).
- Path params: `z: u8, x: u32, y: u32` (standard XYZ).
- Response: `application/x-protobuf`, `Cache-Control: no-cache`, `Access-Control-Allow-Origin: *`.
- Status codes: `500` (`"Could not fetch data"`) on upstream failure.

### `GET /cta_ttarrivals_proxy`

- Source: [`src/birch/chicago_proxy.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/chicago_proxy.rs) (line 9)
- Purpose: Proxies the Chicago Transit Authority's `ttarrivals.aspx` real-time "L" arrivals API.
- Query params: `map_id: string` (required — CTA's station/"mapid" identifier; missing it is a `400` from actix's own query-extraction failure).
- Upstream: uses a hardcoded CTA API key that is CTA's own published public/demo key — low sensitivity, but worth knowing it's shared across all Catenary callers (so CTA-side rate limits on that key apply globally, not per-caller).
- Response: passed through as-is, **no explicit `Content-Type`** set (CTA returns JSON, but this proxy doesn't declare that). This proxy does **not** inspect CTA's own response body for an error condition — even if CTA's JSON reports an internal error code, this endpoint still returns `200` as long as the network request itself succeeded.
- Status codes: `500` (`"Error: {debug-formatted reqwest error}"` — note this can leak internal error/URL details) only on a network-level failure.

### `GET /openrailwaymap_proxy/{path:.*}`

- Source: [`src/birch/openrailwaymap_proxy.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/openrailwaymap_proxy.rs) (line 9)
- Purpose: Proxies OpenRailwayMap's vector-tile server (a Martin tileserver) for a railway-infrastructure map overlay, rewriting the `tiles` URLs inside any TileJSON response so a client keeps hitting Catenary's domain instead of OpenRailwayMap's.
- Path param: `path` — a wildcard capturing everything after the proxy prefix. Query strings on the incoming request are **not** forwarded upstream.
- Response: if upstream's `Content-Type` is exactly `application/json` (i.e. a TileJSON document), returns it with `tiles` URLs rewritten to point at `birch_orm1.catenarymaps.org/openrailwaymap_proxy/...`; otherwise, forwards raw bytes with **no `Content-Type` header at all** — binary tile responses lose their content-type in transit, which can confuse a strict map client.
- Status codes: `200` on success; `500` (empty body) if upstream itself returned a 5xx — the real upstream status/body is discarded, not passed through.
- Footguns: **no `Cache-Control` header at all** — every tile request round-trips to OpenRailwayMap live. Several unguarded `.unwrap()`s (on the outbound HTTP request, on reading the upstream `Content-Type` header, on JSON parsing) mean any network hiccup or unusual upstream response **panics** the request rather than returning a clean error.

### Terrain / contour tile proxies (MapTiler & Mapbox)

Source: [`src/birch/terrain_tiles_proxy.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/terrain_tiles_proxy.rs). Three endpoints, all `GET /{name}/{z}/{x}/{y}.{ext}` with `z: u8, x: u32, y: u32`:

| Path | Upstream | Notes |
|---|---|---|
| `/maptiler_terrain_tiles_proxy/{z}/{x}/{y}.webp` (line 7) | MapTiler `terrain-rgb-v2` | Picks randomly between two hardcoded MapTiler API keys per request (basic load-spreading). |
| `/maptiler_contours_tiles_proxy/{z}/{x}/{y}.pbf` (line 64) | MapTiler `contours-v2` | Same key pool. |
| `/mapbox_terrain_tiles_proxy/{z}/{x}/{y}.vector.pbf` (line 121) | Mapbox `mapbox.mapbox-terrain-v2` | Uses a hardcoded Mapbox **public** access token (`pk.`-prefixed — Mapbox's own "safe to embed" token type). |

For all three: the upstream API key/token is used server-side only and is **not** included in the response body — safe from that angle — but is committed in source, and this proxy spoofs a fixed `Origin`/`Referer` header on the *outbound* request to satisfy the upstream provider's referer-lock, regardless of who actually called this endpoint. Combined with this server's wide-open CORS, **any third party can hotlink through these proxies and consume the shared key's quota**, with no per-caller attribution or throttling on Catenary's side.

- Response on success: `Cache-Control: public, max-age=9999999999` (~317 years — effectively cache-forever) plus `Access-Control-Allow-Origin: *`; `Content-Type` copied from upstream (defaults to `application/octet-stream` if upstream omits it).
- **Any non-success upstream status (rate-limited, auth failure, etc.) is collapsed to a generic `404`** — you cannot distinguish "tile genuinely doesn't exist" from "we got rate-limited/auth-rejected by the upstream provider" from the status code alone.
- `500` (`"Could not fetch data"`) only on a network-level failure.
- Each request builds a brand-new `reqwest::Client` rather than reusing a shared one — a minor inefficiency, not a correctness issue.

## Misc utility endpoints

### `GET /microtime` / `GET /nanotime`

- Source: `src/birch/server.rs` (~lines 219, 232)
- Purpose: Return the server's current time as plain text — `microtime` in microseconds since the Unix epoch, `nanotime` in nanoseconds. Useful for basic clock-skew/latency checks against the server, not a data API.
- Response: `text/plain`, a bare integer.

### `GET /ip_addr_to_geo/`

- Source: `src/birch/server.rs` (`ip_addr_to_geo_api`, ~line 723)
- Purpose: Geolocates the **caller's own** IP address (as seen by the server, e.g. via `X-Forwarded-For`/connection info) against an internal IP-to-geo database — used to pick a sensible default map location for a new visitor.
- Response: `{ data_found: bool, error: bool, geo_resp: Option<IpToGeoAddr>, err_msg: Option<string> }`, where `IpToGeoAddr = { is_ipv6: bool, range_start, range_end: ipnet::IpNet, country_code, geo_state, geo_state2, city, postcode: Option<string>, latitude, longitude: f64, timezone: Option<string> }`.
- Status codes: always `200` — failures are represented in the body (`error: true` / `data_found: false`), not via HTTP status.
- Caching: `Cache-Control: no-cache`.
- Footgun: there is no query parameter to look up an *arbitrary* IP — this endpoint only ever geolocates the requester's own connecting address.

### `GET /size_bbox_zoom`

- Source: `src/birch/server.rs` (`size_bbox_zoom_birch`, ~line 1098)
- Purpose: Given a lat/lon bounding box and a zoom level, compute how many slippy-map tiles that box would cover at that zoom — a helper for clients estimating tile-fetch cost before requesting a large area.
- Query params: `t, b, l, r: f32` (top/bottom/left/right, degrees), `zoom: u8`.
- Response: `application/json`, a bare number (tile count).
- Status codes: `400` (`"Bad BBox"`) if the box is invalid (e.g. degenerate/out-of-range); `200` otherwise.
