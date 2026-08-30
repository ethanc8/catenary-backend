# birch: realtime feed credential management (internal/admin only)

Server: **birch**, `http://127.0.0.1:17419`. Source: [`src/birch/api_key_management.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/api_key_management.rs). Registered in [`src/birch/server.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/birch/server.rs).

## ⚠️ Security notes — read before exposing this publicly

This module manages the **upstream API keys/passwords Catenary uses to poll third-party GTFS-Realtime feeds** for every agency in the system. It is documented here per project decision to cover the full API surface, but its current protection model has real gaps that anyone operating this service should be aware of:

- **Authentication is a single check**: an `email` + `password` looked up against a local `admin_credentials` Postgres table, hashed with Argon2 and compared. There is no API key, bearer token, session, or CSRF protection, no rate limiting, and no lockout on repeated failed attempts — combined with this server's wide-open CORS policy (see [README.md](README.md)), this is brute-forceable from anywhere.
- **No per-feed ownership/scoping**: any caller who authenticates can read (`GET /getrealtimekeys`) or overwrite (`POST /setrealtimekey/{feed_id}/`) the credentials for **every** feed in the system, and `GET /exportrealtimekeys/` dumps **every** feed's stored credentials as CSV in one call. There is no concept of "your own agency's feed only."
- **Passwords are logged to stdout in cleartext** on every login attempt (both the caller's submitted password and, implicitly, whatever the server compares it against) — anywhere these logs are shipped becomes a credential-leak surface.
- **Missing-header inputs can panic the request** in `set_realtime_key` (`.unwrap()` on missing `email`/`password` headers), though `export_realtime_keys` does check for this more carefully.
- **Conclusion:** treat this whole module as an internal admin tool that must sit behind a firewall/reverse-proxy restriction, not as a publicly-reachable part of the API surface, regardless of what the auth check nominally requires. If you're deciding whether to document/expose this to external integrators: don't.

## `POST /setrealtimekey/{feed_id}/`

- Path param: `feed_id: string` (the target feed's `onestop_feed_id`). Trailing slash in the path is required (actix does not normalize it away here).
- Auth: request headers `email: string`, `password: string` (both required; missing/non-ASCII values panic the request rather than returning `400`).
- Request body: **raw RON (Rusty Object Notation) text, not JSON** — `EachPasswordRow = { passwords: Option<PasswordFormat>, fetch_interval_ms: Option<i32> }`, where `PasswordFormat = { key_formats: Vec<KeyFormat> (KeyFormat = Header(string) | UrlQuery(string)), passwords: Vec<PasswordInfo> (PasswordInfo = { password: Vec<string>, creator_email: string }), override_schedule_url, override_realtime_vehicle_positions, override_realtime_trip_updates, override_alerts: Option<string> }`. This RON-not-JSON body format is not documented anywhere else and is easy to get wrong.
- Response: empty body on success (`200`). No JSON schema on success.
- Status codes: `401` on bad login; `500` (`"Deserialise password failed"`) on a malformed RON body; `500` (`"insert into realtime passwords failed"` / `"insert update interval fail"`) on DB failure; `200` (empty) on success.
- Caching: `Cache-Control: no-cache` on the later branches only — inconsistent across error paths.
- Footgun: no check that `feed_id` actually exists before upserting — a typo'd feed_id silently creates/updates a row for a feed that doesn't correspond to anything, or updates zero rows in `realtime_feeds` with no error.

## `GET /exportrealtimekeys/`

- Auth: same header-based check as above (this one does check for missing headers cleanly, returning `401` rather than panicking).
- Response: **CSV text** (no `Content-Type` set, defaults to plain body), header row `onestop_feed_id,passwords,last_updated_ms`, one row per feed with a stored password — i.e. **every stored realtime-feed credential in the system**, in one response.
- Status codes: `401` on bad/missing auth; `200` CSV on success (a raw DB-query failure here is unguarded and will panic rather than return `500`).
- Caching: `Cache-Control: no-cache` on success.

## `POST /getrealtimekeys`

- Auth: submitted as an **HTML form body** (`application/x-www-form-urlencoded`), not headers — `{ email: string, password: string }`. This is the third different auth-transport convention among these three endpoints (headers+RON body / headers only / form body) — there is no consistency in how credentials or payloads are transmitted across this module.
- Response: `{ "passwords": { "<onestop_feed_id>": EachPasswordRow } }` for **every** row in `realtime_feeds`, `passwords: null` for feeds with no stored credential.
- Status codes: `401` on bad login; `500` on DB query failure; `200` JSON on success.
- Caching: `Cache-Control: no-cache` on all branches.
