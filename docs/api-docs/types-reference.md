# Shared type reference

These types are used by more than one endpoint across the docs in this folder. Rather than repeat field lists in every doc, each endpoint doc links back here. Endpoint-specific/local DTOs (used by only one endpoint) are documented inline in that endpoint's own doc instead.

Unless noted otherwise, "chateau" fields are Catenary's internal region/agency-cluster ID (see [README.md](README.md)), and all struct fields shown are exactly what's serialized to JSON (Rust field name = JSON key, since these types don't use `#[serde(rename)]` except where noted).

## Static schedule types (`catenary::models`, [`src/models.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/models.rs))

These are Diesel ORM row types mapped directly from Postgres tables in the `gtfs` schema. Most mirror the GTFS static spec fairly closely, with a `chateau` column added and translations (`*_translations`) stored as a JSON object mapping language code → translated string alongside the untranslated `*` field.

### `Route` (table `gtfs.routes`)
```
onestop_feed_id: String
attempt_id: String
route_id: String
short_name: Option<String>            short_name_translations: Option<Value>
long_name: Option<String>             long_name_translations: Option<Value>
gtfs_desc: Option<String>             gtfs_desc_translations: Option<Value>
route_type: i16                       // GTFS extended route_type
url: Option<String>                   url_translations: Option<Value>
agency_id: Option<String>
gtfs_order: Option<u32>
color: Option<String>                 // hex, no leading '#', may be null/invalid
text_color: Option<String>
continuous_pickup: i16
continuous_drop_off: i16
shapes_list: Option<Vec<Option<String>>>
chateau: String
```

### `Stop` (table `gtfs.stops`)
Note: this Diesel model is **not** `Serialize` — no endpoint returns it verbatim. Every endpoint that exposes "a stop" builds its own DTO from it (and several different, non-identical DTOs exist across the codebase — see each endpoint's footguns). Fields, for reference when reading the source:
```
onestop_feed_id, attempt_id, gtfs_id: String
name: Option<String>                  name_translations: Option<Value>
displayname: Option<String>
code: Option<String>
gtfs_desc: Option<String>             gtfs_desc_translations: Option<Value>
location_type: i16                    // GTFS: 0=stop/platform, 1=station, 2=entrance/exit, 3=generic node, 4=boarding area
parent_station: Option<String>
zone_id: Option<String>
url: Option<String>
point: Option<postgis_diesel::types::Point>   // .x = longitude, .y = latitude
timezone: Option<String>              // IANA tz name, may be absent
wheelchair_boarding: i16
primary_route_type: Option<i16>
level_id: Option<String>
platform_code: Option<String>         platform_code_translations: Option<Value>
routes: Vec<Option<String>>           // route_ids serving this exact stop_id
route_types: Vec<Option<i16>>
children_ids: Vec<Option<String>>     // child stop_ids (for a parent station)
children_route_types: Vec<Option<i16>>
station_feature: bool
hidden: bool
chateau: String
location_alias: Option<Vec<Option<String>>>
tts_name: Option<String>              tts_name_translations: Option<Value>
allowed_spatial_query: bool           // if false, this stop is invisible to nearby/spatial-search endpoints regardless of proximity
osm_station_id: Option<i64>
osm_platform_id: Option<i64>
```

### `Agency` (table `gtfs.agencies`)
```
static_onestop_id, agency_id, attempt_id: String
agency_name: String                   agency_name_translations: Option<Value>
agency_url: String                    agency_url_translations: Option<Value>
agency_timezone: String               // IANA tz name
agency_lang: Option<String>
agency_phone: Option<String>
agency_fare_url: Option<String>       agency_fare_url_translations: Option<Value>
chateau: String
unified_agency_id: Option<String>
level_0s: Option<Vec<Option<String>>> // admin-hierarchy country-level names
level_1s: Option<Vec<Option<String>>> // admin-hierarchy region/state-level names
has_rail, has_tram, has_metro, has_ferry, has_bus: bool
```
`bbox` (a bounding polygon) exists on the Rust struct but is annotated `#[serde(skip)]` — it is **never present** in JSON output, even though it's a real database column.

### `Chateau` (table `gtfs.chateaus`)
Not serialized directly (not `Serialize`); `GET /getchateaus` (see [birch-schedule-data.md](birch-schedule-data.md)) builds its own GeoJSON representation from it. Underlying fields: `chateau: String`, `static_feeds: Vec<Option<String>>`, `realtime_feeds: Vec<Option<String>>`, `languages_avaliable: Vec<Option<String>>`, `hull: Option<MultiPolygon>` (the chateau's geographic coverage area).

### `CompressedTrip` (table `gtfs.trips_compressed`)
Internal-ish; a few fields surface directly in some responses (e.g. vehicle history).
```
onestop_feed_id, trip_id, attempt_id: String
service_id: CompactString
trip_short_name: Option<CompactString>
direction_id: Option<bool>
block_id: Option<String>
wheelchair_accessible: i16
bikes_allowed: i16
chateau: String
frequencies: Option<Vec<u8>>          // protobuf-encoded GTFS Frequency blob, decoded server-side
has_frequencies: bool
itinerary_pattern_id: String
route_id: String
start_time: u32                       // seconds since midnight of the service day; GTFS-style, CAN exceed 86400
```

### `ItineraryPatternRow` / `ItineraryPatternMeta`, `DirectionPatternRow` / `DirectionPatternMeta`
These describe a trip's ordered stop sequence ("itinerary pattern") and a route's overall direction/headsign grouping ("direction pattern"). Mostly internal, but their fields appear (partially) in `/route_info` and `/route_info_v2` (see [birch-schedule-data.md](birch-schedule-data.md)).
```
ItineraryPatternRow {
  onestop_feed_id, attempt_id, itinerary_pattern_id: String
  stop_sequence: i32
  arrival_time_since_start: Option<i32>       // seconds offset from trip start; CAN exceed 86400
  departure_time_since_start: Option<i32>
  interpolated_time_since_start: Option<i32>
  stop_id: CompactString
  chateau: String
  gtfs_stop_sequence: u32
  timepoint: Option<bool>
  stop_headsign_idx: Option<i16>
}
ItineraryPatternMeta {
  onestop_feed_id, attempt_id, itinerary_pattern_id, chateau: String
  trip_ids: Vec<Option<String>>
  trip_headsign: Option<String>               trip_headsign_translations: Option<Value>
  shape_id: Option<String>
  timezone: String
  route_id: CompactString
  direction_pattern_id: Option<String>
  row_count: i32
}
DirectionPatternRow { chateau, direction_pattern_id, onestop_feed_id, attempt_id: String,
  stop_id: CompactString, stop_sequence: u32,
  arrival_time_since_start / departure_time_since_start / interpolated_time_since_start: Option<i32>,
  stop_headsign_idx: Option<i16> }
DirectionPatternMeta { chateau, direction_pattern_id, onestop_feed_id, attempt_id: String,
  headsign_or_destination: String, gtfs_shape_id: Option<String>, fake_shape: bool,
  route_id: Option<CompactString>, route_type: Option<i16>, direction_id: Option<bool>,
  stop_headsigns_unique_list: Option<Vec<Option<String>>>,
  direction_pattern_id_parents: Option<String>, row_count: i32 }
```

### `Shape` (table `gtfs.shapes`)
```
onestop_feed_id, attempt_id, shape_id, chateau: String
linestring: postgis_diesel::types::LineString<Point>   // .points, each .x = lon, .y = lat
color: Option<String>
routes: Option<Vec<Option<String>>>
route_type: i16
route_label: Option<String>           route_label_translations: Option<Value>
text_color: Option<String>
allowed_spatial_query: bool
stop_to_stop_generated: Option<bool>
```

### `Calendar` / `CalendarDate` (tables `gtfs.calendar` / `gtfs.calendar_dates`)
Standard GTFS calendar semantics: `Calendar` has one boolean per weekday plus `gtfs_start_date`/`gtfs_end_date`; `CalendarDate.exception_type` is `1` (service added) or `2` (service removed) for a specific `gtfs_date`.

### `OsmStation` (table `gtfs.osm_stations`)
Raw OpenStreetMap station/stop node/way data, independent of GTFS. Not serialized directly by any endpoint — each OSM-related endpoint builds its own subset DTO (see [birch-search.md](birch-search.md) for why there are several non-identical versions of this).
```
osm_id: i64
osm_type: String
import_id: i32
point: postgis_diesel::types::Point   // .x = lon, .y = lat
name: Option<String>                  name_translations: Option<Value>
station_type: Option<String>
railway_tag: Option<String>
mode_type: String
uic_ref: Option<String>
ref_: Option<String>                  // DB column "ref"
wikidata: Option<String>
operator: Option<String>
network: Option<String>
level: Option<String>
local_ref: Option<String>
parent_osm_id: Option<i64>
is_derivative: bool
admin_hierarchy: Option<Value>
```

### `VehicleEntry` (table `gtfs.vehicles`)
Static fleet-catalog metadata (manufacturer/model/year), keyed by per-agency fleet-number ranges. Returned as-is by `GET /get_vehicle` (see [birch-realtime.md](birch-realtime.md) — note that endpoint is *not* a live-position lookup despite its name).
```
file_path: String                     // internal per-agency catalog key, e.g. "north_america/.../losangelesmetro_bus"
starting_range: Option<i32>           ending_range: Option<i32>
starting_text: Option<String>         ending_text: Option<String>
use_numeric_sorting: Option<bool>
manufacturer: Option<String>
model: Option<String>
years: Option<Vec<Option<String>>>
engine: Option<String>
transmission: Option<String>
notes: Option<String>
key_str: String
```

## Realtime types (`catenary::aspen_dataset`, [`src/aspen_dataset.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/aspen_dataset.rs))

These are populated in-memory by `aspen` from GTFS-Realtime feeds and shipped to birch/spruce/ramonda over the internal tarpc RPC. **Important:** several HTTP endpoints serialize these Rust enums as JSON **strings** (the variant name, e.g. `"Cancelled"`), while other endpoints/locally-defined DTOs convert the same semantic value to a small **integer** code instead. This is inconsistent across the API — each endpoint doc says explicitly which form it uses.

### `AspenisedVehiclePosition`
```
trip: Option<AspenisedVehicleTripInfo>
vehicle: Option<AspenisedVehicleDescriptor>
position: Option<CatenaryRtVehiclePosition>
timestamp: Option<u64>                // unix seconds (GTFS-RT VehiclePosition.timestamp)
route_type: i16
current_stop_sequence: Option<u32>
current_status: Option<i32>           // GTFS-RT VehicleStopStatus: 0=INCOMING_AT, 1=STOPPED_AT, 2=IN_TRANSIT_TO
congestion_level: Option<i32>
occupancy_status: Option<i32>
occupancy_percentage: Option<u32>
consist: Option<UnifiedConsist>       // train-formation/consist data (Swiss/German rail feeds)
```

### `CatenaryRtVehiclePosition`
```
latitude: f32                         // WGS84 degrees
longitude: f32
bearing: Option<f32>                  // degrees clockwise from true north (GTFS-RT convention)
odometer: Option<f64>                 // meters, cumulative (GTFS-RT convention)
speed: Option<f32>                    // meters/second — NOT km/h or mph
```

### `AspenisedVehicleDescriptor`
```
id: Option<String>
label: Option<String>                 // often identical to `id` for feeds that don't supply a real label — see ReplaceVehicleLabelWithVehicleId in aspen_dataset.rs
license_plate: Option<String>
wheelchair_accessible: Option<i32>
```

### `AspenisedVehicleTripInfo`
```
trip_id: Option<String>
trip_headsign: Option<Arc<str>>
route_id: Option<String>
trip_short_name: Option<String>
direction_id: Option<u32>
start_time: Option<String>            // GTFS "HH:MM:SS", can exceed 24h
start_date: Option<chrono::NaiveDate>
schedule_relationship: Option<AspenisedTripScheduleRelationship>
delay: Option<i32>                    // seconds
```

### `AspenisedTripUpdate`
```
trip: AspenRawTripInfo
vehicle: Option<AspenisedVehicleDescriptor>
timestamp: Option<u64>                // unix seconds
delay: Option<i32>                    // seconds
stop_time_update: Vec<AspenisedStopTimeUpdate>
trip_properties: Option<AspenTripProperties>
trip_headsign: Option<Arc<str>>
consist: Option<Box<UnifiedConsist>>
found_schedule_trip_id: bool
last_seen: u64                        // defaults to 0 for old cached data lacking this field
```

### `AspenRawTripInfo`
```
trip_id: Option<String>
route_id: Option<String>
direction_id: Option<u32>
start_time: Option<String>
start_date: Option<chrono::NaiveDate>
schedule_relationship: Option<AspenisedTripScheduleRelationship>
modified_trip: Option<ModifiedTripSelector>
```

### `AspenisedStopTimeUpdate`
```
stop_sequence: Option<u16>
stop_id: Option<Arc<str>>
arrival: Option<AspenStopTimeEvent>
departure: Option<AspenStopTimeEvent>
departure_occupancy_status: Option<AspenisedOccupancyStatus>
schedule_relationship: Option<AspenisedStopTimeScheduleRelationship>
stop_time_properties: Option<AspenisedStopTimeProperties>
platform_string: Option<EcoString>
old_rt_data: bool                     // true = this stop-time-update is stale/carried over from a previous feed refresh
platform_info: Option<AspenisedPlatformInfo>
```

### `AspenStopTimeEvent`
```
delay: Option<i32>                    // seconds
time: Option<i64>                     // unix seconds, absolute (GTFS-RT StopTimeEvent.time)
uncertainty: Option<i16>              // seconds
```

### Enums and their wire values
```
AspenisedTripScheduleRelationship: Scheduled=0, Added=1, Unscheduled=2, Cancelled=3, Replacement=5, Duplicated=6, Deleted=7
AspenisedStopTimeScheduleRelationship: Scheduled=0, Skipped=1, NoData=2, Unscheduled=3
AspenisedOccupancyStatus: Empty=0 .. NotBoardable=8   // standard GTFS-RT occupancy enum
```
As noted above: when these appear inside a raw `AspenisedTripUpdate`/`AspenisedVehiclePosition` they serialize as the **string** variant name; when they've been passed through a local `_Output`-suffixed conversion type (common in [birch-realtime.md](birch-realtime.md)), they serialize as a small **integer**.

### `AspenisedAlert`
```
active_period: Vec<AspenTimeRange>            // { start: Option<u64>, end: Option<u64> }, unix seconds
informed_entity: Vec<AspenEntitySelector>     // { agency_id, route_id, route_type: Option<i32>, trip: Option<AspenRawTripInfo>, stop_id, direction_id }
cause: Option<i32>                            // GTFS-RT Alert.Cause
effect: Option<i32>                           // GTFS-RT Alert.Effect (1 = NO_SERVICE)
url, header_text, description_text, tts_header_text, tts_description_text: Option<AspenTranslatedString>
severity_level: Option<i32>
image: Option<AspenTranslatedImage>
image_alternative_text: Option<AspenTranslatedString>
cause_detail, effect_detail: Option<AspenTranslatedString>
```
`AspenTranslatedString { translation: Vec<{ text: String, language: Option<String> }> }`. `AspenTranslatedImage { localised_image: Vec<{ url, media_type, language: Option<String> }> }`.

An alert whose `informed_entity` list has **no** `route_id`/`trip_id`/`stop_id` at all is treated by most alert-matching code as "applies to everything" and will show up attached to every route/trip/stop queried — this is intentional (agency-wide alerts) but can be surprising.

### `AspenisedStop`
A realtime-injected stop (e.g. from a GTFS-RT `Stop` entity on an unscheduled/added trip), distinct from the static `catenary::models::Stop` above:
```
stop_id: Option<Arc<str>>
stop_code, stop_name, tts_stop_name, stop_desc: Option<AspenTranslatedString>
stop_lat: Option<f32>                 stop_lon: Option<f32>
zone_id, stop_url, parent_station, stop_timezone: Option<String>
wheelchair_boarding: Option<i32>
level_id: Option<String>
platform_code: Option<AspenTranslatedString>
```

### `AspenisedVehicleRouteCache`
A lightweight route summary shipped alongside bulk vehicle-position data so a map client doesn't need a separate route query per vehicle: `route_short_name`, `route_long_name`, `route_colour`, `route_text_colour`, `route_type: i16`, `route_desc`, `agency_id`.

### `AspenisedTrajectory` / `AspenisedTrajectorySegment` / `AspenisedTrajectoryStop`
Used by trajectory endpoints (see [spruce-websocket-api.md](spruce-websocket-api.md) and `GET /get_all_trajectories` in [birch-realtime.md](birch-realtime.md)).
```
AspenisedTrajectory {
  unique_trip_id, chateau_id, trip_id: String
  route_id, start_time, start_date: Option<String>
  display_name: String
  mode: String                        // free-text mode label, e.g. "bus", "rail" — NOT the numeric route_type
  color, text_color: Option<String>
  route_short_name, route_long_name, trip_short_name: Option<String>
  route_type: i32
  distance: f64                       // meters, cumulative trip distance
  segments: Vec<AspenisedTrajectorySegment>
  stops: Vec<AspenisedTrajectoryStop>
  real_time: bool
}
AspenisedTrajectorySegment { from_stop_index: usize, to_stop_index: usize, coordinates: Vec<[f64; 2]> }  // [lon, lat] pairs
AspenisedTrajectoryStop { name: String, stop_id: Option<Arc<str>>, lat: f64, lon: f64, track: Option<String>,
  modes: Vec<String>, arrival: String, departure: String }   // arrival/departure format not fully confirmed from source alone
```

## Internal-RPC discovery types (`catenary::aspen::lib`, [`src/aspen/lib.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/aspen/lib.rs))

Not returned directly to API clients, but referenced throughout the other docs since they explain *why* an endpoint behaves the way it does when a chateau's realtime backend is unavailable:

- `ChateauMetadataEtcd { worker_id: String, socket: SocketAddr }` — the etcd value at key `/aspen_assigned_chateaux/{chateau_id}` telling birch/spruce/ramonda which aspen node currently owns a chateau's realtime data. If a chateau has no assigned node (down, unconfigured, or a typo'd chateau ID), lookups return `None`, and — as covered per-endpoint — this is handled inconsistently (sometimes `200` with empty data, sometimes `404`, sometimes `500`).
- `RealtimeFeedMetadataEtcd { worker_id, socket, chateau_id }` — same idea, but keyed by an individual **realtime feed ID** (`/aspen_assigned_realtime_feed_ids/{feed_id}`) rather than a chateau ID. This is a **separate ID namespace** from chateau IDs, used only by `GET /gtfs_rt` (see [birch-realtime.md](birch-realtime.md)).

## Query-time trip types (`catenary::trip_logic`, [`src/trip_logic.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/trip_logic.rs))

Used by `GET /get_trip_information/{chateau}/`, `GET /get_trip_information_rt_update/{chateau}/` (birch), and the `/ws/trip` and ramonda WebSocket protocols.

### `QueryTripInformationParams`
```
trip_id: String                       // required
start_time: Option<String>            // GTFS "HH:MM:SS" — disambiguates when multiple RT updates match the same trip_id
start_date: Option<String>            // "YYYYMMDD" or "YYYY-MM-DD"
route_id: Option<String>
```

### `TripIntroductionInformation` (full "trip detail" payload)
```
stoptimes: Vec<StopTimeIntroduction>
tz: chrono_tz::Tz                     // the trip's own timezone
block_id: Option<String>
bikes_allowed: i16                    wheelchair_accessible: i16
has_frequencies: bool
route_id, agency_id, agency_name: String
trip_headsign, route_short_name, trip_short_name, route_long_name, color, text_color: Option<String>
vehicle: Option<AspenisedVehicleDescriptor>   // populated only if a live vehicle is currently assigned to this trip
route_type: i16
stop_id_to_alert_ids: BTreeMap<String, Vec<String>>
alert_id_to_alert: BTreeMap<String, AspenisedAlert>
alert_ids_for_this_route: Vec<String>
alert_ids_for_this_trip: Vec<String>
shape_polyline: Option<String>        // encoded polyline (precision 5)
trip_id_found_in_db: bool
service_date: Option<chrono::NaiveDate>
schedule_trip_exists: bool
rt_shape: bool                        // true if this shape came from realtime (e.g. a trip modification) instead of static GTFS
old_shape_polyline: Option<String>
cancelled_stoptimes: Vec<StopTimeIntroduction>
is_cancelled: bool
deleted: bool
connecting_routes: Option<BTreeMap<String, BTreeMap<String, Route>>>          // chateau -> route_id -> Route
connections_per_stop: Option<BTreeMap<String, BTreeMap<String, Vec<String>>>> // stop_id -> chateau -> route_ids
trip_id: Option<String>
chateau: Option<String>
consist: Option<UnifiedConsist>
sbb_formation: Option<SbbFormationData>   // Swiss-rail-specific train formation data
```

### `StopTimeIntroduction`
```
stop_id: CompactString
name: Option<String>                  translations: Option<BTreeMap<String, String>>
platform_code: Option<String>         rt_platform_string: Option<String>
timezone: Option<chrono_tz::Tz>
code: Option<String>
longitude, latitude: Option<f64>
scheduled_arrival_time_unix_seconds: Option<u64>      // already an absolute unix timestamp
scheduled_departure_time_unix_seconds: Option<u64>
rt_arrival, rt_departure: Option<AspenStopTimeEvent>
schedule_relationship: Option<u8>     // numeric code here, not the string enum
gtfs_stop_sequence: Option<u16>
interpolated_stoptime_unix_seconds: Option<u64>
timepoint: Option<bool>
replaced_stop: bool                   // true if substituted in via a GTFS-RT trip modification
osm_station_id: Option<i64>
platform_info: Option<AspenisedPlatformInfo>
```

### `GtfsRtRefreshData` / `StopTimeRefresh` (the lightweight "just the RT part" payload)
```
GtfsRtRefreshData { stoptimes: Vec<StopTimeRefresh>, timestamp: Option<u64>, trip_id: Option<String>, chateau: Option<String> }
StopTimeRefresh {
  stop_id: Option<EcoString>
  rt_arrival, rt_departure: Option<AspenStopTimeEvent>
  schedule_relationship: Option<u8>
  gtfs_stop_sequence: Option<u16>
  rt_platform_string: Option<EcoString>
  departure_occupancy_status: Option<u8>
  platform_info: Option<AspenisedPlatformInfo>
}
```

## `SerializableStop` (a JSON-friendly stop DTO, [`src/lib.rs`](https://github.com/catenarytransit/catenary-backend/blob/main/src/lib.rs))

Used by `/route_info`, `/route_info_v2`, and `/fetchalertsofchateau/` to expose a stop without the raw Postgres/PostGIS types. **Note:** there is an unrelated, differently-shaped struct with the same name in `src/graph_formats.rs` — don't confuse the two.
```
id: String
code: Option<String>
name: Option<String>
description: Option<String>
location_type: i16
parent_station: Option<String>
zone_id: Option<String>
longitude: Option<f64>                latitude: Option<f64>     // derived from Stop.point.x / .y
timezone: Option<String>
platform_code: Option<String>
level_id: Option<String>
routes: Vec<String>
```
