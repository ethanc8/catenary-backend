# Catenary Backend API Documentation

This directory documents every HTTP and WebSocket API that `catenary-backend` exposes. It is meant to be useful both to people building against these APIs from outside the project ("external users": app/frontend authors, third-party integrators) and to people working on Catenary itself ("internal users": other services, ops, future maintainers).

These docs describe **behavior as implemented**, including bugs, inconsistencies, and footguns discovered while reading the source. Where something looks like a bug rather than intended behavior, it's called out explicitly rather than silently normalized. Source links point at `main` on GitHub; if the code has moved since these docs were written, use the link as a starting point and search the file for the function/route name.

## The four servers

`catenary-backend` is a Cargo workspace that builds several binaries. Four of them expose network APIs over HTTP or WebSockets; the rest (`maple`, `aspen`, `alpenrose`, `avens`, etc.) are data-ingestion workers, or expose only an internal [tarpc](https://github.com/google/tarpc) RPC protocol over raw TCP (not HTTP/WebSockets, so out of scope for this doc set — see "Internal RPC" below).

| Server | Binary / entry point | Listens on (source) | Public host (if known) | What it's for |
|---|---|---|---|---|
| **birch** | [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs) | `127.0.0.1:17419` | `birch.catenarymaps.org` (plus sharded subdomains for some tile layers, see [birch-maps-and-tiles.md](birch-maps-and-tiles.md)) | The main REST/JSON/GeoJSON/MVT HTTP API — static GTFS schedule data, map tiles, search, departures, realtime vehicle/trip/alert data, vehicle history, admin key management, misc proxies. |
| **spruce** | [`src/spruce/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/spruce/main.rs) | `127.0.0.1:52771` | not yet documented — ask the Catenary team | WebSocket API for live map data: per-trip realtime subscriptions, viewport-based live vehicle locations, trajectories, and nearby-departures. Also serves one plain HTTP endpoint. |
| **ramonda** | [`src/ramonda/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/ramonda/main.rs) | `127.0.0.1:52772` (configurable) | not yet documented | A second, independent WebSocket service for per-trip realtime subscriptions only (no map/trajectory support). Actively used in production, but is **not** simply a "lite" version of spruce — see [ramonda-websocket-api.md](ramonda-websocket-api.md) for exactly how its protocol differs. |
| **harebell** (+ **globeflower**) | [`src/harebell/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/harebell/main.rs) | `127.0.0.1:8080` (default; CLI flag) | not yet documented | A minimal static file server for pre-generated Mapbox Vector Tiles (route-line rendering). `globeflower` is the offline tile-generation tool; `harebell` just serves the files it produces. |

**Note on production hostnames:** all four binaries bind to loopback only in source — there's no domain/TLS/reverse-proxy configuration in this repo. `birch.catenarymaps.org` is inferred from tile-cache URLs embedded in birch's own responses (see the tile docs); the public hostnames for spruce/ramonda/harebell aren't derivable from source and should be confirmed with whoever operates the production deployment.

### Related services in other repositories

Two more services are documented here for convenience even though they live in **separate repositories**, not in `catenary-backend`. They're unrelated codebases (different languages/frameworks, no shared code with birch/spruce/ramonda/harebell) — mentioned here because they're part of the same production system and a consumer of "the Catenary API" may reasonably need to know about them too.

| Service | Repository | Production URL | What it's for |
|---|---|---|---|
| **tulip** | [`catenarytransit/tulip`](https://github.com/catenarytransit/tulip) | `https://tulip.catenarymaps.org` | A Leptos admin/debug web portal. Its API surface is a small number of Leptos server functions that mostly proxy birch's admin/debug endpoints — see [tulip-api.md](tulip-api.md). Two of its endpoints relay birch admin credentials; read the security note there before treating it as a public API. |
| **cypress** | [`catenarytransit/cypress`](https://github.com/catenarytransit/cypress) | not yet confirmed | A standalone geocoding service (forward/reverse search, autocomplete, place details) over OpenStreetMap data, with its own from-scratch bigram/FST search index and ScyllaDB storage — no Elasticsearch, no shared code with birch's search endpoints. See [cypress-geocoding-api.md](cypress-geocoding-api.md). |

### Internal RPC (out of scope, but useful context)

Behind all four servers sits **aspen** ([`src/aspen/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/aspen/main.rs)), the process that ingests and holds live GTFS-Realtime data in memory. birch/spruce/ramonda talk to aspen using [tarpc](https://docs.rs/tarpc) over a raw TCP + Bincode transport — not HTTP, not WebSockets — so it isn't documented here. It matters for these docs only because:

- Realtime data is partitioned by **"chateau"** — Catenary's internal ID for a region/agency-data-cluster (not a GTFS-standard term). A chateau is not a single GTFS static feed and not a single realtime feed; it's the unit that one `aspen` worker node owns. You discover valid chateau IDs via `GET /getchateaus` (see [birch-schedule-data.md](birch-schedule-data.md)).
- Every "realtime" HTTP/WebSocket endpoint below internally does: look up which aspen node currently owns a chateau (via an etcd-backed cache), open/reuse a tarpc connection to it, make an RPC call, and translate the result to JSON. **When aspen is down or a chateau has no assigned node, most endpoints degrade silently** — see the "Cross-cutting footguns" section below. This is the single most important thing to understand before integrating with any realtime endpoint in this API.

## Cross-cutting conventions and footguns

These apply across most or all of the endpoints documented in this folder. Individual endpoint docs repeat only what's endpoint-specific.

- **No authentication on nearly everything.** CORS is wide open (`Access-Control-Allow-Origin: *` plus `allow_any_origin()`), and there is no API-key or auth-token requirement anywhere except the admin key-management endpoints (see [birch-admin-api.md](birch-admin-api.md)), which use a much weaker scheme (email+password against an internal admin table) than the sensitivity of what they protect would suggest. There is no rate limiting anywhere in the code (any that exists is at a reverse-proxy/CDN layer outside this repo).
- **"Chateau not found" / "realtime backend unreachable" is handled inconsistently across endpoints** — sometimes `200 OK` with an empty/placeholder body, sometimes `404`, sometimes `500`, and there is no universal rule. Each endpoint doc states its specific behavior. As a general rule: **do not assume a 200 response means the data is fresh or complete** — many endpoints silently fall back to schedule-only data, or to an empty result, when the realtime backend for a chateau is down, and give the client no signal that this happened.
- **Timestamps are usually Unix seconds, except where they're milliseconds.** Fields explicitly named `..._ms` (e.g. `last_updated_time_ms`) are milliseconds; almost everything else (arrival/departure times, alert active periods, vehicle timestamps) is Unix seconds. GTFS's own "seconds since midnight" convention (which can exceed 86400 for a post-midnight trip) is used only in a few low-level internal fields; most response fields have already been converted to absolute Unix timestamps for you.
- **Coordinate order is inconsistent across the codebase.** GeoJSON-shaped output (features, `geo::Point`/`geo::Rect` serialization) is `[longitude, latitude]` / `{"x": lon, "y": lat}` (standard GIS order). Many hand-written DTOs instead use named `lat`/`lon` or `latitude`/`longitude` fields — check field names, don't assume positional order.
- **Units:** speed is meters/second (GTFS-RT convention, not km/h or mph), bearing is degrees clockwise from true north, distances in "meters" query params are approximate (see the tile-simplification and connection-distance footguns in [birch-maps-and-tiles.md](birch-maps-and-tiles.md) and [birch-schedule-data.md](birch-schedule-data.md)).
- **Route type** fields use the standard [GTFS extended `route_type`](https://gtfs.org/schedule/reference/#routestxt) integer values (0=tram, 1=subway/metro, 2=rail, 3=bus, 4=ferry, 5=cable tram, 6=aerial lift, 7=funicular, 11=trolleybus, 12=monorail, plus some feed-specific extended codes like 200 for coach) unless otherwise noted.
- **Panics on malformed/edge-case input are common.** Many handlers use bare `.unwrap()` on database results, header parsing, or RPC responses. Actix turns a panicked handler into a generic `500` with no useful body, which is indistinguishable from other server errors. Where a specific input is known to trigger this, it's called out per-endpoint.
- **Duplicate/divergent implementations exist for conceptually-identical concepts.** For example, three or four independently-defined `OsmStationInfo`-shaped structs exist across different search endpoints with different field subsets, and `connections_lookup` (the "nearby connecting routes" algorithm) has two independently-maintained copies (one in the `birch` binary, one in the `catenary` library crate). Don't assume the same-looking data from two different endpoints has the exact same shape — each endpoint doc calls out its own response shape explicitly.

## Where to find what

- [birch-schedule-data.md](birch-schedule-data.md) — static GTFS schedule data: chateaus, routes, agencies, feed ingestion metadata, blocks/vehicle-blocks.
- [birch-maps-and-tiles.md](birch-maps-and-tiles.md) — shapes (as GeoJSON/polyline or Mapbox Vector Tiles), stop/station tiles, route geometry export.
- [birch-search.md](birch-search.md) — stop lookup/preview, OSM station lookup/search/preview, general text search.
- [birch-departures.md](birch-departures.md) — scheduled + realtime departure boards for a stop, an OSM station, or a geographic point.
- [birch-realtime.md](birch-realtime.md) — live vehicle positions, trip detail/refresh, raw GTFS-RT feed passthrough, alerts.
- [birch-vehicle-history.md](birch-vehicle-history.md) — historical vehicle-to-trip assignment records.
- [birch-admin-api.md](birch-admin-api.md) — realtime feed credential management. **Internal/admin only — read the security notes before exposing this publicly.**
- [birch-proxies-and-misc.md](birch-proxies-and-misc.md) — third-party proxies (Amtrak, CTA, CAL FIRE, Metrolink, OpenRailwayMap, terrain tiles, Watch Duty) and small utility endpoints.
- [spruce-websocket-api.md](spruce-websocket-api.md) — the live-map WebSocket protocol (`/ws/trip`, `/ws/live`).
- [ramonda-websocket-api.md](ramonda-websocket-api.md) — the standalone trip-subscription WebSocket protocol.
- [harebell-tile-server.md](harebell-tile-server.md) — static vector-tile file server.
- [types-reference.md](types-reference.md) — shared data types (`Route`, `Stop`, `Agency`, `AspenisedVehiclePosition`, etc.) referenced by multiple endpoints, documented once instead of repeated everywhere.
- [tulip-api.md](tulip-api.md) — the tulip admin/debug portal's API (separate repository).
- [cypress-geocoding-api.md](cypress-geocoding-api.md) — the cypress geocoding service's API (separate repository).

## A note on how this was written

These docs were produced by reading the `catenary-backend` source directly (not by testing a live deployment), current as of commit `89cf84e6` (2026-08-30). Response shapes and status codes are as implemented; a few details flagged as "not fully verifiable from source alone" (e.g. the exact JSON key names `geo::Rect<f64>` serializes to) should be double-checked against a live response before being treated as a hard contract. If you find a discrepancy, it's more likely the docs are stale than the code is wrong — please update both.
