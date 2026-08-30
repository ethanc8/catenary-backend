# tulip: admin/debug portal API

**Public endpoint:** `https://tulip.catenarymaps.org/`

**Repository:** [`catenarytransit/tulip`](https://github.com/catenarytransit/tulip)

Tulip ("Transport Unification Live Infrastructure Portal") is a [Leptos](https://leptos.dev) server-rendered Rust/WASM web app — both a human-browsable admin/debug UI and, incidentally, the source of a small number of JSON API endpoints. **It is not a general-purpose public API** — every endpoint here exists to back one specific admin page, and two of them (`load_realtime_keys`, and the unnamed `submit_data` function) directly relay Catenary admin credentials to birch's admin key-management module (see [birch-admin-api.md](birch-admin-api.md)) — read the security note below before treating this as safe to call from untrusted contexts.

## How the API surface works

Tulip's endpoints are [Leptos server functions](https://book.leptos.dev/server/25_server_functions.html) (`#[server(...)]`-annotated async functions), not hand-written REST handlers. The server registers a single catch-all route, `/api/{tail:.*}` → `leptos_actix::handle_server_fns()` ([`src/main.rs`](https://github.com/catenarytransit/tulip/blob/main/src/main.rs) line 83), which dispatches to whichever server function matches. Each function that specifies `#[server(endpoint = "some_name")]` becomes reachable at `POST /api/some_name` — which is exactly why the three endpoints you already knew about have clean, predictable names.

**Wire format caveat:** because these are Leptos server functions rather than a bespoke JSON API, the exact request/response encoding follows `server_fn` crate conventions (this repo pins `server_fn` 0.7.5/0.8.11 in its lockfile, but the crate source isn't vendored here to inspect directly). Based on the observed `POST /api/<name>` pattern and default `server_fn` behavior, arguments are most likely sent as a form-encoded POST body and successful responses as a JSON-encoded return value, with errors surfaced via a non-200 status and a serialized `ServerFnError`. **This is not independently confirmed from source** — check a live request in your browser's devtools before hard-coding a client against it. The Rust function signatures documented below are accurate regardless of wire encoding and fully describe the parameter names, types, and response shapes.

Tulip also serves a handful of plain HTML page routes (`/`, `/realtimekeys`, `/chateaux`, `/debug/schedule/:feed_id`, `/help`, `/test1`, `/404.html`, plus `robots.txt`) — these are Leptos UI routes, not API endpoints, and aren't documented further here.

## ⚠️ Security note

`load_realtime_keys` and `submit_data` both accept a plaintext admin `master_email`/`master_password` as ordinary server-function arguments and forward them to birch's admin endpoints (`POST /getrealtimekeys` and `POST /setrealtimekey/{feed_id}/` respectively — see [birch-admin-api.md](birch-admin-api.md)). This means:
- Calling `POST /api/load_realtime_keys` or the credential-taking `submit_data` function from anywhere reachable on the internet is **exactly as sensitive as calling birch's admin endpoints directly** — tulip adds no additional protection, and in fact adds its own weakness: **it logs the submitted email and password to tulip's own server-side stdout in cleartext** (`println!("Sending to Birch, {}, {}", master_email, master_password)`, [`src/app.rs`](https://github.com/catenarytransit/tulip/blob/main/src/app.rs) line 193).
- There's no rate limiting, CSRF protection, or session mechanism on tulip's side either — the `/realtimekeys` page's own login state lives only in browser-side reactive signals, not a real session.

## `POST /api/get_chateaus_nogeom`

- Source: [`src/chateaux.rs`](https://github.com/catenarytransit/tulip/blob/main/src/chateaux.rs) line 13, `pub async fn get_chateaus_nogeom() -> Result<Vec<ChateauToSendNoGeom>, ServerFnError>`
- Purpose: backs the `/chateaux` admin page — proxies birch's chateau list with no request parameters.
- Params: none.
- Behavior: calls `GET https://birch.catenarymaps.org/getchateausnogeom` (see [birch-schedule-data.md](birch-schedule-data.md)) and re-sorts the result alphabetically by `chateau` (birch's own endpoint doesn't guarantee sort order to arbitrary callers, though its live implementation happens to already sort — tulip re-sorts defensively regardless).
- Response shape: `ChateauToSendNoGeom { chateau: String, realtime_feeds: Vec<String>, schedule_feeds: Vec<String>, languages_avaliable: Vec<String> }` — a locally-defined struct in tulip, independent of (but field-for-field matching) birch's own `ChateauToSendNoGeom`.
- Error behavior: any network failure or non-2xx from birch, or a response body that doesn't deserialize into this exact shape, becomes a generic `ServerFnError` (the underlying reqwest/serde error text, not a structured error code).

## `POST /api/get_feed_metadata`

- Source: [`src/feed_metadata.rs`](https://github.com/catenarytransit/tulip/blob/main/src/feed_metadata.rs) line 67, `pub async fn get_feed_metadata(feed_id: String) -> Result<FeedMetadataResponse, ServerFnError>`
- Purpose: backs the `/debug/schedule/:feed_id` admin page — shows GTFS static ingestion history for one feed.
- Params: `feed_id: String` — this is birch's `onestop_feed_id`, not a chateau ID (same distinction as birch's own endpoint — see [birch-schedule-data.md](birch-schedule-data.md#get-feed_metadata)).
- Behavior: calls `GET https://birch.catenarymaps.org/feed_metadata?feed_id={feed_id}` and passes the JSON straight through, deserialized into a locally-defined (structurally identical, independently maintained) copy of birch's `FeedMetadataResponse { ingested_static: Vec<IngestedStatic>, static_download_attempts: Vec<StaticDownloadAttempt> }` — see [types-reference.md](types-reference.md) for field-level detail on those two row types (same fields here).
- Footgun: birch's `/feed_metadata` always returns `200` (even for a nonexistent `feed_id`, or when its own underlying DB query silently errored — see the birch doc's footgun note), so tulip has no way to distinguish "no data for this feed" from "birch's query errored" either; it will just show empty tables. If birch ever does return a non-2xx (e.g. its one real error path, a Postgres *connection* failure), tulip surfaces that as a generic `ServerFnError`.

## `POST /api/load_realtime_keys`

- Source: [`src/app.rs`](https://github.com/catenarytransit/tulip/blob/main/src/app.rs) line 179, `pub async fn load_realtime_keys(master_email: String, master_password: String) -> Result<Option<KeyResponse>, ServerFnError>`
- Purpose: backs the `/realtimekeys` admin page's "Load" button — fetches every realtime feed's stored credentials from birch, gated by admin login.
- Params: `master_email: String`, `master_password: String` — see security note above.
- Behavior: builds a form-urlencoded body (`email=...&password=...`, values percent-encoded) and calls `POST https://birch.catenarymaps.org/getrealtimekeys` (see [birch-admin-api.md](birch-admin-api.md#post-getrealtimekeys)).
- Response semantics — **the three-way result is meaningful, don't collapse it to a boolean**:
  - Birch responds `200` → `Ok(Some(KeyResponse))`, where `KeyResponse = { passwords: BTreeMap<String /* onestop_feed_id */, EachPasswordRow> }` and `EachPasswordRow = { passwords: Option<PasswordFormat>, fetch_interval_ms: Option<i32> }` (see `PasswordFormat`/`KeyFormat`/`PasswordInfo` field lists in [birch-admin-api.md](birch-admin-api.md#post-setrealtimekeyfeed_id)).
  - Birch responds `401` → `Ok(None)` — i.e. **"wrong credentials" and "no error occurred" are the same `Result::Ok` variant here**, just with an inner `None`; callers must check for `Ok(None)` specifically to detect a bad login, not just match on `Err`.
  - Any other birch status → `Err(ServerFnError)` whose message **includes birch's raw response body text** — if birch ever returns a verbose error page, that text is embedded in the error surfaced to whatever's calling this server function.

## `submit_data` (unnamed endpoint — exact path unconfirmed)

- Source: [`src/app.rs`](https://github.com/catenarytransit/tulip/blob/main/src/app.rs) line 237-238, `#[server] async fn submit_data(master_email: String, master_password: String, feed_id: String, password: String, interval: String) -> Result<bool, ServerFnError>`
- **This function has no explicit `endpoint = "..."` name** (unlike the three above), so — unlike them — its exact `/api/...` path is not confirmed from source alone. This is likely why it wasn't in your original list of three known endpoints. Confirm the real path via browser devtools on the `/realtimekeys` page's "Submit" button before relying on a guessed path.
- Purpose: backs the `/realtimekeys` page's "Submit" button — writes (adds or overwrites) one feed's realtime credentials.
- Params: `master_email`, `master_password` (admin login, see security note), `feed_id: String`, `password: String` (a **RON-encoded** `Option<PasswordFormat>` — same RON, not JSON, format birch's own `/setrealtimekey/{feed_id}/` expects, since this value is round-tripped through `ron::from_str`/`ron::ser::to_string` unchanged), `interval: String` (a RON-encoded `Option<i32>`, milliseconds).
- Behavior: parses `password`/`interval` from RON into `EachPasswordRow { passwords, fetch_interval_ms }`, re-serializes that struct to RON, and `POST`s it as the raw body to `https://birch.catenarymaps.org/setrealtimekey/{feed_id}/` with `email`/`password` headers set to the *admin* credentials (not per-feed credentials) — see [birch-admin-api.md](birch-admin-api.md#post-setrealtimekeyfeed_id).
- Response: `Ok(true)` on birch `200`; `Ok(false)` on birch `401` (bad admin login — again collapsed into the `Ok` variant, not `Err`); `Err(ServerFnError::new("Data did not submit correctly"))` for any other status (birch's actual response text/status is only logged server-side on tulip, not included in the error returned to the caller — the opposite tradeoff from `load_realtime_keys` above).
- Footgun: if `password`/`interval` fail to parse as RON, this returns `Err` (propagated from the `?` on `ron::from_str`) **before any network call happens** — a malformed RON payload never reaches birch, so a caller getting an error here should check their RON syntax first rather than assuming a birch-side problem.
