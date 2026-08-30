# harebell: static route-line tile server

This is an experimental service, which will in future provide (using `globeflower`) a prebuilt map of railway (metro, tramway, mainline railway) routes that has been ["LOOMed"](https://github.com/ad-freiburg/loom), i.e. overlapping routes are drawn as parallel lines with optimized line ordering.

**Public endpoint:** Not deployed.

**Localhost endpoint:** `https://localhost:8080/`

**Source:** [`src/harebell/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/harebell/main.rs) (CLI entry point) and [`src/harebell/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/harebell/server.rs) (the actual HTTP handler). 

The companion `globeflower` binary ([`src/globeflower/main.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/globeflower/main.rs)) is an **offline CLI tool**, not a network service — it builds a routing/rendering graph from OSM + GTFS data and pre-generates the `.pbf` tile files that harebell then serves. It's mentioned here only for context; it has no HTTP API of its own.

## Starting the server

`harebell serve --address <addr> --port <port>` (defaults `localhost:8080`). It serves whatever static tiles already exist under `./tiles_output/` relative to its working directory — it does **not** generate tiles itself; that's `globeflower export`'s job, run offline ahead of time.

## `GET /tiles/{z}/{x}/{y}.pbf`

- Source: [`src/harebell/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/harebell/server.rs) (`get_tile`)
- Purpose: Serve a pre-generated Mapbox Vector Tile file from disk.
- Path params: `z: u8, x: u32, y: u32` — standard XYZ slippy-map tile coordinates.
- Behavior: reads the file at `./tiles_output/{z}/{x}/{y}.pbf` directly off disk — no database, no cache invalidation logic, no dynamic generation. If `globeflower` hasn't been re-run since the underlying GTFS/OSM data changed, this server will keep serving stale tiles indefinitely with no way to detect staleness from the HTTP response.
- Response: `application/x-protobuf` with the raw file bytes on success.
- Status codes: `404 Not Found` if the file doesn't exist at that path; `500 Internal Server Error` if the file exists but can't be read (permissions, I/O error); `200` otherwise.
- Caching: **no `Cache-Control` header set at all.**
- CORS: permissive (`Cors::permissive()` applied to the whole server) — no restriction on which origins can fetch tiles.
- Footguns: this is the entire API surface of harebell — there is no metadata/TileJSON endpoint, no listing of available zoom levels, and no indication of which regions have been exported. A client needs out-of-band knowledge (from whoever runs `globeflower export`) of what tile coverage currently exists on disk.
