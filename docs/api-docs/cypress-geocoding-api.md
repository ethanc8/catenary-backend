# cypress: geocoding API

**Repository:** [`catenarytransit/cypress`](https://github.com/catenarytransit/cypress) — a separate repository from `catenary-backend`, checked out as a sibling directory (`../cypress`) in this workspace. Source links below point at `main` on that repo.

Cypress is a standalone Rust geocoding service (forward search, reverse geocoding, autocomplete, place details) built on OpenStreetMap data — a Pelias/Nominatim-style geocoder, but with its own from-scratch search engine: a memory-mapped, zero-copy bigram + FST index (no Elasticsearch dependency for queries) backed by ScyllaDB for full record storage. It is not wired into `catenary-backend` and doesn't share code with birch's `/text_search_v1`/`/osm_station_search` (see [birch-search.md](birch-search.md)), though it appears intended to serve a similar purpose for general place search.

The HTTP server is the `query` binary ([`src/query/main.rs`](https://github.com/catenarytransit/cypress/blob/main/src/query/main.rs)), built on `axum`, default listen address `0.0.0.0:3000` (`--listen` CLI flag). CORS is fully permissive. No authentication on any route.

## How results are assembled (read this before the endpoint reference)

Every search/autocomplete query is scored by an in-process ranking engine (`search_place_ids`, [`src/query/search.rs`](https://github.com/catenarytransit/cypress/blob/main/src/query/search.rs) line 705) that runs entirely against a memory-mapped rkyv index (bigram inverted index + prefix FST + spatial grid + structured address index) — **no database round-trip is needed to rank candidates.** `/v1/search` and `/v2/search` then take the ranked place IDs and "hydrate" full records from ScyllaDB; `/v1/autocomplete` skips that hydration step entirely and reads minimal name/coordinate data straight out of a second flat memory-mapped file for speed.

**`confidence` is an unbounded relative ranking score, not a normalized probability.** It's the negation of an internal match "cost" (lower cost = better match; the sign is flipped so higher `confidence` = better match in the API). It is **not clamped to `[0,1]`**, can be negative, and its absolute magnitude is only meaningful for ordering results *within one response* — never compare `confidence` values across two different queries, and never display it to end users as a percentage.

## `GET /health`

Returns `{"status": "ok"}`. Liveness check only.

## `GET /v1/search` and `GET /v2/search`

- Source: [`src/query/main.rs`](https://github.com/catenarytransit/cypress/blob/main/src/query/main.rs) (`search_handler` line 166, `search_v2_handler` line 198); core logic `src/query/search.rs` (`execute_search` line 1534, `execute_search_v2` line 1626)
- Purpose: forward geocoding — free-text search for places.
- Query params: `text: string` (required), `lang: Option<string>` (preferred language for display names — see below), `bbox: Option<string>` (`"minLon,minLat,maxLon,maxLat"`, four comma-separated floats — a malformed value that doesn't parse to exactly 4 floats is **silently ignored, not rejected**, i.e. treated the same as if `bbox` were absent), `focus.point.lat` / `focus.point.lon`: `Option<f64>` (both must be present to have any effect), `focus.point.weight: Option<f64>` (default `50.0`), `layers: Option<string>` (comma-separated — **accepted but currently has zero effect on results, see footguns below**), `size: Option<usize>` (default `10`, silently clamped to a max of `40`).
- Response: `{"features": SearchResult[], "took_ms": u128}`. Each `SearchResult` (GeoJSON `Feature`-shaped):
  ```
  { "type": "Feature",
    "geometry": { "type": "Point", "coordinates": [lon, lat] },
    "properties": {
      "id": string,                    // source_id, e.g. "node/12345" or "way/67890" — pass to /v1/place/details
      "layer": string,                 // one of the Layer enum's lowercase names (see below)
      "name": string,                  // resolved display name per `lang` (see below)
      "names": { [lang: string]: string },
      "housenumber": string | omitted,
      "street": string | omitted,
      "postcode": string | omitted,
      "country": string | omitted,     // resolved admin-parent NAME (not an ID) — see below
      "region": string | omitted,
      "county": string | omitted,
      "locality": string | omitted,
      "neighbourhood": string | omitted,
      "categories": string[] | omitted (omitted if empty),
      "confidence": number             // see caveat above
    }
  }
  ```
  Note: only 5 of the model's 9 admin-hierarchy levels are surfaced (`country`, `region`, `county`, `locality`, `neighbourhood`) — `macro_region`, `macro_county`, `local_admin`, and `borough` are resolved internally (to compute the ones that *are* shown) but never appear in the response, at either API version.
- **v2 adds one thing**: for each of the five admin-parent fields above, `PropertiesV2` also includes a sibling `<level>_names: { [lang]: string } | omitted` — e.g. `country_names` — giving you every language variant of that admin area's name, not just the one resolved for `lang`. Otherwise v1 and v2 responses are identical.
- **`lang` only affects which name string is *displayed*, not what matches.** Text matching always runs against the language-agnostic indexed bigram/FST data regardless of `lang`. Resolution order for both the place's own name and each admin parent's name: your requested `lang` → the `"default"` key → first available value in that name map → (for the place's own name only, if the whole name map is empty) a synthesized `"{housenumber} {street}"` or `{place}` from the address components.
- Admin-parent names (`country`/`region`/etc.) come from a **separate batched ScyllaDB lookup** (`get_admin_areas`) keyed by the relation IDs stored on each place — meaning these ARE full human-readable names (with full multilingual variants available via v2), unlike `/v1/place/details`'s raw output (see below).
- Status codes: `500` (plain-text body = the underlying error) if the ScyllaDB hydration step itself fails entirely; otherwise always `200`, **including when individual matched places fail to hydrate** — see footguns.

### Footguns specific to search

- **`layers` does nothing.** It's parsed into `SearchParams.layers` and even logged, but is never used to filter or boost results anywhere in the ranking pipeline, on either search endpoint or either reverse endpoint. Don't rely on it. (`Layer`'s valid lowercase values, for when/if this gets wired up, are: `venue`, `address`, `street`, `admin`, `macro_region`, `region`, `macro_county`, `county`, `local_admin`, `locality`, `borough`, `neighbourhood`, `country`.)
- **`bbox` is a hard post-filter over an already-truncated candidate pool, not a targeted spatial query.** The ranking engine first selects its best ~10,000 text-match candidates by score *before* bbox is applied; if all of those happen to fall outside your bbox, you can get zero results even though genuinely matching places exist inside the box further down the (discarded) ranking. There's no fallback/widening behavior.
- **Places that match in the index but fail to hydrate from ScyllaDB are silently dropped**, with no logging and no indication in the response — you can legitimately get fewer than `size` results (or fewer than expected) with no error, simply because the index and the ScyllaDB table have drifted out of sync for those IDs.
- **`focus.point.*` is a step-function ranking bonus, not continuous distance decay**: `+2.5×weight` under 2 km, `+2.0×weight` under 10 km, `+1.0×weight` under 100 km, `+0.5×weight` under 1000 km, `+0` beyond that. `focus.point.weight` of `0` is treated as effectively-zero-but-not-exactly-zero (clamped to a tiny positive floor internally) — it will not perfectly disable the bonus, though the practical effect is negligible.
- Internal candidate limits are compiled-in constants, not configurable: a bigram-similarity cutoff (cosine similarity ≥ 0.17) and hard caps of 6,000 matched name-strings / 10,000 scored place candidates considered per query, before your requested `size` further truncates the final list.

## `GET /v1/reverse` and `GET /v2/reverse`

- Source: `src/query/main.rs` (`reverse_handler` line 230, `reverse_v2_handler` line 258); core logic `src/query/search.rs` (`execute_reverse` line ~1725, `execute_reverse_v2` line ~1796)
- Purpose: reverse geocoding — nearest place(s) to a coordinate.
- Query params: `point.lon`, `point.lat: f64` (both required), `lang: Option<string>` (**v2 only — see footgun below**), `layers: Option<string>` (accepted, **no effect**, same as forward search), `size: Option<usize>` (default 10, clamped to max 40).
- Response: same envelope/shape as forward search (`{"features": SearchResult[] | SearchResultV2[], "took_ms": 0}`) — **`took_ms` is hardcoded to `0` in both reverse endpoints regardless of actual processing time**; don't use it for timing.
- Behavior: searches a fixed **1.0 km radius** around the point using a spatial grid (0.01°-cell, ~1.1 km grid), in a single pass with **no ring expansion** — if there is nothing indexed within 1 km, you get an empty `features` array, full stop, regardless of `size`. There is no way to request a larger search radius via this API. Results are ordered purely by distance ascending; there is no importance/population tie-break.
- **`confidence` is a hardcoded literal `1.0` on every reverse-geocoding result**, regardless of actual distance from the query point — it carries no ranking information here (unlike forward search, where it's a real score). Don't interpret it.
- **Footgun — v1 ignores `lang` entirely.** `/v1/reverse` never forwards the `lang` query parameter internally (its underlying function has no `lang` parameter at all) — passing `lang=fr` to `/v1/reverse` has no effect. Only `/v2/reverse` honors it. This is an undocumented v1/v2 inconsistency; if you need language-aware reverse geocoding, use v2.

## `GET /v1/autocomplete`

- Source: `src/query/main.rs` (`autocomplete_handler` line 114)
- Purpose: fast typeahead suggestions — the "hot path" that skips ScyllaDB entirely.
- Query params: `text: string` (required — must be at least 2 characters after trimming/lowercasing, or you get an empty result set with no error), `size: Option<usize>` (default 10, clamped to max **20**, note this cap is tighter than search/reverse's 40), `focus.point.lat`/`focus.point.lon`/`focus.point.weight: Option<f64>` (same semantics/defaults as forward search).
- **No `bbox` or `layers` parameters exist on this endpoint at all** — and internally, the ranking call this handler makes always passes "no bbox" regardless, so there is no way to geographically scope autocomplete beyond the soft focus-point ranking nudge.
- Response: `{"features": AutocompleteFeature[], "memdb_took_ms": u128}`, where each feature is a minimal GeoJSON Point:
  ```
  { "type": "Feature",
    "geometry": { "type": "Point", "coordinates": [lon, lat] },
    "properties": { "id": string, "name": string } }
  ```
  **No `confidence`, `layer`, admin-hierarchy, or address fields at all** — this endpoint reads only raw name/coordinate bytes from a flat, memory-mapped record file (bypassing both the full ranking-score output and ScyllaDB), so it's the cheapest and least informative of the four query endpoints. If you need any of the omitted fields for a selected autocomplete suggestion, follow up with `/v1/place/details?id=<the id you got back>`.

## `GET /v1/place/details`

- Source: `src/query/main.rs` (`place_details_handler` line 286)
- Purpose: full record lookup by ID (e.g. after a user picks an autocomplete suggestion).
- Query params: `id: string` (required — the `source_id` from any other endpoint's response, formatted `"{osm_type}/{osm_id}"`, e.g. `"node/12345"`, `"way/67890"`, `"relation/111"`).
- Response: the **raw JSON exactly as stored in ScyllaDB** — this is a `NormalizedPlace` document, `Content-Type: application/json`:
  ```
  { "source_id": string, "source_file": string, "import_timestamp": string (RFC3339),
    "osm_type": "node"|"way"|"relation", "osm_id": number,
    "wikidata_id": string | omitted, "importance": number | omitted, "population": number | omitted,
    "layer": string, "categories": string[],
    "name": { [lang: string]: string }, "phrase": string | omitted,
    "address": { "housenumber", "street", "postcode", "city", "place": string | omitted each } | omitted,
    "center_point": { "lat": number, "lon": number },
    "bbox": { "type": "envelope", "coordinates": [[minLon,maxLat],[maxLon,minLat]] } | omitted,
    "parent": { "country", "macro_region", "region", "macro_county", "county", "local_admin", "locality", "borough", "neighbourhood": string | null }
  }
  ```
- **Important difference from the search endpoints: `parent.*` here are bare relation-ID strings (e.g. `"relation/51477"`), not human-readable names.** Unlike `/v1/search`/`/v2/search` (which do a separate batched lookup to resolve these into real names — see above), this endpoint gives you the raw normalized record with IDs only. If you need the admin hierarchy's actual names for a place fetched via this endpoint, you currently have to resolve each relation ID yourself (there's no endpoint exposed for that) — or, more practically, just use the `country`/`region`/etc. fields already present on a `/v1/search` or `/v2/search` result for the same place instead of re-fetching via `place/details`.
- Note `center_point` here uses named `lat`/`lon` fields (not a `[lon, lat]` array like every other endpoint's GeoJSON `geometry.coordinates`) — this endpoint isn't GeoJSON-shaped at all, it's the internal storage record verbatim.
- Status codes: `200` with the JSON body above; `404` (empty body) if `id` doesn't exist; `500` if the ScyllaDB query itself fails.

## Coordinate conventions, summarized

- Every GeoJSON `geometry.coordinates` field across `/v1/search`, `/v2/search`, `/v1/reverse`, `/v2/reverse`, `/v1/autocomplete` is `[longitude, latitude]` (standard GeoJSON order).
- `/v1/place/details`'s `center_point` is the one exception — a named `{"lat": ..., "lon": ...}` object, since that endpoint returns the internal storage record rather than a GeoJSON feature.
- The `bbox` *query parameter* you send (`"minLon,minLat,maxLon,maxLat"`) is a different representation from the `bbox` *field* that can appear inside a `/v1/place/details` response (`{"type":"envelope","coordinates":[[minLon,maxLat],[maxLon,minLat]]}`) — don't confuse the two when round-tripping a bounding box between requests and responses.
