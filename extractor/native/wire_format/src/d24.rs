//! d24 `weather_live` — per-strategic-region and per-province realized weather.
//!
//! Derived from `world_fold.rs:18392`-`:18719`
//! (`fold_weather`), its payload constants at `:761`-`:808`, the state it folds
// @world-fold-authority domain=24 functions=fold_weather lines=18392-18719 sha256=a40946a82ef36080e9dbea9252098cd68770424c9592db32455e6389cc9d3135
//! into at `:24179`-`:24301`, the collector read that produces the flags
//! (`extractor/native/collector/src/attach_log.rs:59018`-`:59192`), and the
//! production consumer of the folded JSON
//! (`replay_interface/native/replay_core/src/world.rs:7438`-`:7591`,
//! `:750`-`:790`).
//!
//! **Payload grammar** (`world_fold.rs::fold_weather`): a 48-byte header, then
//! `region_count` fixed 32-byte region rows, then `province_count` fixed 32-byte
//! province rows. Two collections, one payload. There is no legacy short row:
//! the fold requires `payload.len() == 48 + region_count*32 + province_count*32`
//! exactly (`world_fold.rs::fold_weather`), so length never selects a row form.
//!
//! | header | offset | field |
//! |---|---|---|
//! | u64 | `+0`  | `elapsed_wall_ms` |
//! | u32 | `+8`  | `region_count` (rows emitted by THIS record) |
//! | u32 | `+12` | `changed_rows` |
//! | u32 | `+16` | `status_flags` |
//! | u32 | `+20` | `province_count` (rows emitted by THIS record) |
//! | u32 | `+24` | `observed_region_rows` (must be 305, `world_fold.rs::fold_weather`) |
//! | u32 | `+28` | `observed_province_rows` (0, or 10_000..=65_536) |
//! | u32 | `+32` | `scale_raw` (asserted `== FIXED_SCALE`, `world_fold.rs::fold_weather`) |
//! | u32 | `+36` | `active_region_count` (delta-local witness, `world_fold.rs::fold_weather`) |
//! | u32 | `+40` | `reserved0` (must be 0) |
//! | u32 | `+44` | `reserved1` (must be 0) |
//!
//! **This domain is baseline + delta** (`world_fold.rs::fold_weather`), unlike d63's
//! full-replace. A record without `COMPACT_STATUS_DELTA_DATA` resets
//! `weather_live` and must carry all 305 regions and all `observed_province_rows`
//! provinces; a delta record upserts its rows and never deletes. `WeatherState`
//! below is that accumulator, so the measurement harness and the tests fold the
//! same way the recorder does instead of each rebuilding it.
//!
//! | region row | offset | field | scale | gate |
//! |---|---|---|---|---|
//! | u32 | `+0`  | `region_id`             | raw    | none (key; `<= 304`) |
//! | u32 | `+4`  | `flags`                 | —      | — |
//! | i32 | `+8`  | `temperature_raw`       | /1e5   | none |
//! | u32 | `+12` | `phenomenon`            | raw    | none (`<= 16`) |
//! | u32 | `+16` | `next_weather_change_raw` | raw  | none (`<= i32::MAX`) |
//! | u32 | `+20` | `active_modifier_count` | raw    | none (`<= 4096`) |
//! | u32 | `+24` | `intensity`             | raw    | none (`<= 255`) |
//! | u32 | `+28` | reserved (must be 0)    | —      | — |
//!
//! | province row | offset | field | scale | gate |
//! |---|---|---|---|---|
//! | u32 | `+0`  | `province_id`            | raw  | none (key; **0 is a real row**, `world_fold.rs::fold_weather`) |
//! | u32 | `+4`  | `flags`                  | —    | — |
//! | i32 | `+8`  | `temperature_raw`        | /1e5 | `TEMP_PRESENT` |
//! | i32 | `+12` | `temperature_change_raw` | /1e5 | `TEMP_CHANGE_PRESENT` |
//! | i32 | `+16` | `water_raw`              | /1e5 | `WATER_PRESENT` |
//! | i32 | `+20` | `snow_raw`               | /1e5 | `SNOW_PRESENT` |
//! | u32 | `+24` | `custom_modifier_count`  | raw  | `CUSTOM_MODIFIER_COUNT_PRESENT` |
//! | u32 | `+28` | reserved (must be 0)     | —    | — |
//!
//! # Which fields are carried, and which are exactly derivable
//!
//! The fold enforces five cross-field consistency rules on every row it accepts.
//! Four of them make a flag an exact function of other carried state, so the
//! schema carries the function's inputs and not its outputs — that is a rewrite
//! of the same information, not a loss, and every one is proved by a test here:
//!
//! - `active_signal == phenomenon_active == (phenomenon != 0)` (`world_fold.rs::fold_weather`). Both
//!   bits are what the fold itself calls "activity witnesses"; `phenomenon` is
//!   carried and both bits read off it.
//! - `map_recolor == (temp_present || water_present || snow_present)` (`world_fold.rs::fold_weather`).
//!   Those three presences are the record's own unknown bits.
//! - `dynamic_lut == (snow_present || mud_present)` (`world_fold.rs::fold_weather`). Likewise.
//!   The collector names both of these `..._DRIVER`
//!   (`lanes/weather_live/domain24_weather_sandstorm_v3.rs:49`-`:50`,
//!   `:133`-`:138`) and computes them from the presences exactly this way.
//! - `mud_active` implies `mud_present` (`:17253`). `MUD_PRESENT` is a
//!   *measurement* gate like the other five `..._PRESENT` bits — the collector
//!   sets it unconditionally next to the other presences
//!   (`attach_log.rs:59164`-`:59166`) — so it is the gate on `mud_active`, and
//!   the implication becomes unrepresentable rather than merely checked.
//!
//! The fifth rule, "named weather flags are mutually exclusive" (`:17176`), is
//! why the five named bits become **one `Enum`**: an enum cannot encode the
//! overlap the fold rejects, and both the fold (`:24228`-`:24240`) and the
//! production replay consumer (`world.rs:7533`-`:7545`) already collapse those
//! bits to a single `weather_kind` string by the same if-chain. `arctic_water`
//! is not part of that chain (`world.rs:7576` keeps it a separate boolean) and
//! stays its own field.
//!
//! `active_region_count` is **not** a field: the fold recomputes it host-side
//! over the reconstructed 305-region state after merging (`:17349`-`:17355`).
//! It is checked here as a header witness over the emitted rows and then
//! discarded.
//!
//! # Four-state semantics
//!
//! **The region collection reaches none of the three empty states.** No region
//! field is gated: every one of the seven values is read unconditionally and is
//! present in every accepted row (`:17123`-`:17208`). Its schema therefore
//! declares no `optional`, `nullable` or `unknown_capable` field at all.
//!
//! **Every gated province field is `unknown`, never `null` or `absent`.** The
//! fold emits JSON `Null` uniformly on a clear gate (`:24277`-`:24295`), which
//! per `WIRE_FORMAT.md` §3 decides value-vs-nothing and nothing else. Reading
//! the collector for *why* a gate is clear:
//!
//! - `TEMP_PRESENT` is `temperature_raw != 0` and `TEMP_CHANGE_PRESENT` is the
//!   *same* predicate (`attach_log.rs:59155`-`:59156`), after which both words
//!   are zeroed (`:59178`-`:59187`). A province whose temperature reads zero
//!   therefore loses a `temperature_change` the collector had already read
//!   successfully: the value existed and was not carried. That is §3's
//!   **unknown** — "applicable, not captured" — word for word, and it is not
//!   `null` (temperature is not a relation that can point at nothing) nor
//!   `absent` (every province has a temperature).
//! - `WATER_PRESENT`, `SNOW_PRESENT`, `MUD_PRESENT` and
//!   `CUSTOM_MODIFIER_COUNT_PRESENT` are set unconditionally by today's
//!   collector (`:59164`-`:59170`), so a clear gate is unreachable from it and
//!   carries no evidence either way. §3's tie-break applies: choose **unknown**,
//!   because `null` asserts "definitively empty" about something merely
//!   uncaptured and that assertion is irreversible. All four are open questions.
//!
//! A province that genuinely has no water reads `0` with its flag **set** — the
//! deep-war capture has 153,964 such rows — and that is a real `Zero`, not one
//! of the three empty states.
//!
//! # Scales
//!
//! `scale_raw` is a header word the fold asserts equal to `FIXED_SCALE`
//! (`ensure_fixed_scale`, `world_fold.rs:17058`/`:30245`) and then stores on
//! every row (`:17207`, `:17324`) without dividing. The division happens in the
//! fold's JSON: `temperature`, `temperature_change`, `water` and `snow` are all
//! `raw / scale_raw` (`:24243`, `:24277`-`:24284`, `optional_scaled_i32` at
//! `world_fold.rs::optional_scaled_i32`). This mapper applies that division. The five raw-only region words
//! (`phenomenon`, `next_weather_change_raw`, `active_modifier_count`,
//! `intensity`) are emitted undivided by the fold and are undivided here.
//!
//! # No derived field (§6)
//!
//! `next_weather_change_raw` is the one clock-shaped candidate, and it is an
//! **absolute** game hour, not a countdown: the consumer names it
//! `transition_expiry_game_hour`, "Absolute game-hour expiry written at
//! `SWeatherPerRegion+0x150`" (`world.rs:762`-`:763`), and across the 903-hour
//! deep-war capture it never decreases for any region. An absolute expiry is
//! already constant between weather changes, so §6's `c` is zero and no
//! transform applies. The mapper still takes `frame_tick` so every domain's
//! boundary has one shape.

use std::collections::BTreeMap;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 24;
pub const REGIONS: u32 = 1;
pub const PROVINCES: u32 = 2;

/// Region field ids. Stable and append-only; §11 forbids reuse for a different
/// field. Field ids are scoped per collection (`SchemaError::DuplicateFieldId`
/// carries a `collection_id`), so the two collections number independently.
pub mod region_field {
    pub const REGION_ID: u32 = 1;
    pub const WEATHER_KIND: u32 = 2;
    pub const ARCTIC_WATER: u32 = 3;
    pub const TEMPERATURE: u32 = 4;
    pub const PHENOMENON: u32 = 5;
    pub const NEXT_WEATHER_CHANGE: u32 = 6;
    pub const ACTIVE_MODIFIER_COUNT: u32 = 7;
    pub const INTENSITY: u32 = 8;
}

/// Province field ids. Stable and append-only.
pub mod province_field {
    pub const PROVINCE_ID: u32 = 1;
    pub const TEMPERATURE: u32 = 2;
    pub const TEMPERATURE_CHANGE: u32 = 3;
    pub const WATER: u32 = 4;
    pub const SNOW: u32 = 5;
    pub const CUSTOM_MODIFIER_COUNT: u32 = 6;
    pub const MUD_ACTIVE: u32 = 7;
    pub const STRUCT_VALIDATED: u32 = 8;
}

/// `weather_kind` codes. The set and the collapsing order are the fold's
/// (`world_fold.rs:24228`-`:24240`) and the replay consumer's
/// (`replay_core/src/world.rs:7533`-`:7545`); the *numbering* is this schema's
/// and is append-only. `NO_PHENOMENON` is code 0 because it is the else-branch
/// of both if-chains and the commonest row.
pub mod weather_kind {
    pub const NO_PHENOMENON: u32 = 0;
    pub const RAIN_LIGHT: u32 = 1;
    pub const RAIN_HEAVY: u32 = 2;
    pub const SNOW: u32 = 3;
    pub const BLIZZARD: u32 = 4;
    pub const SANDSTORM: u32 = 5;
    /// §4 rejects a code at or above the cardinality.
    pub const CARDINALITY: u32 = 6;
}

// --- Payload grammar, from `world_fold.rs:761`-`:808` and `:17032`-`:17326`. --

/// `WEATHER_HEADER_LEN`, `world_fold.rs::WEATHER_HEADER_LEN`.
pub const HEADER_LEN: usize = 48;
/// `WEATHER_REGION_ROW_LEN`, `world_fold.rs::WEATHER_REGION_ROW_LEN`.
pub const REGION_ROW_LEN: usize = 32;
/// `WEATHER_PROVINCE_ROW_LEN`, `world_fold.rs::WEATHER_PROVINCE_ROW_LEN`.
pub const PROVINCE_ROW_LEN: usize = 32;
/// `WEATHER_REGION_ROWS`, `world_fold.rs::WEATHER_REGION_ROWS`. A full snapshot carries exactly
/// this many regions (`:17072`).
pub const REGION_ROWS: usize = 305;
/// `WEATHER_MIN_PROVINCE_ROWS` / `WEATHER_MAX_PROVINCE_ROWS`, `:765`-`:766`.
pub const MIN_PROVINCE_ROWS: usize = 10_000;
pub const MAX_PROVINCE_ROWS: usize = 65_536;
/// `WEATHER_MAX_REGION_ID`, `world_fold.rs::WEATHER_MAX_REGION_ID`.
pub const MAX_REGION_ID: u32 = 304;
/// `WEATHER_MAX_RAW_ACTIVITY_CANDIDATE`, `world_fold.rs::WEATHER_MAX_RAW_ACTIVITY_CANDIDATE`.
pub const MAX_RAW_ACTIVITY_CANDIDATE: u32 = 16;
/// `WEATHER_MAX_TEMPERATURE_RAW`, `world_fold.rs::WEATHER_MAX_TEMPERATURE_RAW`.
pub const MAX_TEMPERATURE_RAW: i32 = 10_000_000;
/// `WEATHER_MAX_TEMP_CHANGE_RAW`, `world_fold.rs::WEATHER_MAX_TEMP_CHANGE_RAW`.
pub const MAX_TEMP_CHANGE_RAW: i32 = 10_000_000;
/// `WEATHER_MAX_PRECIP_RAW`, `world_fold.rs::WEATHER_MAX_PRECIP_RAW`.
pub const MAX_PRECIP_RAW: i32 = 100_000_000;
/// `WEATHER_MAX_ACTIVE_MODIFIERS`, `world_fold.rs::WEATHER_MAX_ACTIVE_MODIFIERS`.
pub const MAX_ACTIVE_MODIFIERS: u32 = 4_096;
/// `world_fold.rs::fold_weather` bounds `intensity` at one byte.
pub const MAX_INTENSITY: u32 = u8::MAX as u32;

/// `COMPACT_STATUS_DELTA_DATA`. Named here because d24 is not admitted to the
/// production-codec dependency surface; the measurement bin asserts the local
/// value and the shipping contract agree.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`, and the header word the fold asserts
/// equal to it (`:17058`).
pub const FIXED_SCALE: f64 = 100_000.0;
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// `FIXED_SCALE` is `10^5`, so `Quant::Fixed(5)` is exactly the source's own
/// precision — see `schema()` and `fixed_five_is_lossless_over_every_declared_raw`.
pub const SCALE_DIGITS: u8 = 5;

pub const REGION_RAIN_LIGHT: u32 = 1;
pub const REGION_RAIN_HEAVY: u32 = 1 << 1;
pub const REGION_SNOW: u32 = 1 << 2;
pub const REGION_BLIZZARD: u32 = 1 << 3;
pub const REGION_ARCTIC_WATER: u32 = 1 << 4;
pub const REGION_ACTIVE_SIGNAL: u32 = 1 << 5;
pub const REGION_PHENOMENON_ACTIVE: u32 = 1 << 6;
pub const REGION_SANDSTORM: u32 = 1 << 7;
/// `WEATHER_REGION_KNOWN_FLAGS`, `world_fold.rs::WEATHER_REGION_KNOWN_FLAGS`. Any other bit is a hard
/// error (`:17137`) — an unrecognised flag word means the row is not this
/// grammar.
pub const REGION_KNOWN_FLAGS: u32 = REGION_RAIN_LIGHT
    | REGION_RAIN_HEAVY
    | REGION_SNOW
    | REGION_BLIZZARD
    | REGION_ARCTIC_WATER
    | REGION_ACTIVE_SIGNAL
    | REGION_PHENOMENON_ACTIVE
    | REGION_SANDSTORM;
/// The mutually-exclusive named weather bits (`world_fold.rs::fold_weather`),
/// paired with the `weather_kind` code each maps to, in the fold's own
/// if-chain order.
pub const NAMED_WEATHER: [(u32, u32); 5] = [
    (REGION_RAIN_LIGHT, weather_kind::RAIN_LIGHT),
    (REGION_RAIN_HEAVY, weather_kind::RAIN_HEAVY),
    (REGION_SNOW, weather_kind::SNOW),
    (REGION_BLIZZARD, weather_kind::BLIZZARD),
    (REGION_SANDSTORM, weather_kind::SANDSTORM),
];

pub const PROVINCE_TEMP_PRESENT: u32 = 1;
pub const PROVINCE_TEMP_CHANGE_PRESENT: u32 = 1 << 1;
pub const PROVINCE_WATER_PRESENT: u32 = 1 << 2;
pub const PROVINCE_SNOW_PRESENT: u32 = 1 << 3;
pub const PROVINCE_MUD_PRESENT: u32 = 1 << 4;
pub const PROVINCE_MUD_ACTIVE: u32 = 1 << 5;
pub const PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT: u32 = 1 << 6;
pub const PROVINCE_STRUCT_VALIDATED: u32 = 1 << 7;
pub const PROVINCE_MAP_RECOLOR: u32 = 1 << 8;
pub const PROVINCE_DYNAMIC_LUT: u32 = 1 << 9;
/// `WEATHER_PROVINCE_KNOWN_FLAGS`, `world_fold.rs::WEATHER_PROVINCE_KNOWN_FLAGS`; enforced at `:17239`.
pub const PROVINCE_KNOWN_FLAGS: u32 = PROVINCE_TEMP_PRESENT
    | PROVINCE_TEMP_CHANGE_PRESENT
    | PROVINCE_WATER_PRESENT
    | PROVINCE_SNOW_PRESENT
    | PROVINCE_MUD_PRESENT
    | PROVINCE_MUD_ACTIVE
    | PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT
    | PROVINCE_STRUCT_VALIDATED
    | PROVINCE_MAP_RECOLOR
    | PROVINCE_DYNAMIC_LUT;

pub fn schema() -> Schema {
    // Every scaled channel is `Quant::Fixed(5)`, and that is not a precision
    // *choice*: the source is a signed int32 CFixedPoint whose denominator the
    // fold asserts to be exactly `10^5` (`world_fold.rs::fold_weather`), so
    // `fixed(5)` reproduces the source integer bit for bit over the whole
    // declared raw range (proved in the tests below). §6's "exact unless
    // justified" bar is met by a stronger fact than a precision budget: nothing
    // is rounded at all. It also matters that the raw integer survives, because
    // the production consumer requires it — `WeatherProvinceSnowView`
    // "deliberately retains raw fixed-point integers" and range-checks
    // `snow_raw` against `0..=100_000_000`
    // (`replay_core/src/world.rs:778`-`:790`, `:7681`-`:7690`); `fixed(5)` is
    // precisely the quantization under which `round(value * 1e5)` returns that
    // integer unchanged, and `exact` would cost 8 raw bytes per value to carry
    // the same information.
    let quant = Quant::Fixed(SCALE_DIGITS);

    let regions = Collection {
        collection_id: REGIONS,
        // A strategic region keeps its identity for the whole campaign, which is
        // what a keyed set needs — and only a keyed set has a delta grammar
        // (§7.3). d24 is itself baseline + delta (`world_fold.rs::fold_weather`).
        shape: Shape::KeyedSet,
        fields: vec![
            // ContentId because a strategic region is content-defined (§5): a
            // manifest must resolve id 42 for it to keep its meaning under
            // another mod set, which is the deciding question for
            // ContentId-vs-Unsigned. Not an instance id — it is not created
            // during play — and not an engine constant either.
            Field::new(region_field::REGION_ID, Kind::ContentId).key(0),
            // The five mutually-exclusive named weather bits (`:17176`), plus
            // the else-branch, as one code. See the module docs.
            Field::new(region_field::WEATHER_KIND, Kind::Enum)
                .enum_card(weather_kind::CARDINALITY),
            // Not part of the weather_kind chain in either consumer.
            Field::new(region_field::ARCTIC_WATER, Kind::Boolean),
            // `temperature_raw / scale_raw` (`world_fold.rs:24243`).
            Field::new(region_field::TEMPERATURE, Kind::Continuous).quant(quant),
            // The raw `+0x78` activity candidate. `Unsigned`, NOT `Enum`: both
            // the fold (`:24224`-`:24226`) and the collector
            // (`attach_log.rs:59034`-`:59035`) state it "is not a stable
            // weather-kind enum", and its only bound is `<= 16` (`:17149`).
            Field::new(region_field::PHENOMENON, Kind::Unsigned),
            // An absolute game hour that rises as each weather spell expires;
            // `Monotone` is §4's encoding hint, never a validity constraint.
            Field::new(region_field::NEXT_WEATHER_CHANGE, Kind::Monotone),
            // A live count of attached modifiers: it falls as well as rises, so
            // `Unsigned` rather than `Monotone`.
            Field::new(region_field::ACTIVE_MODIFIER_COUNT, Kind::Unsigned),
            // One byte (`:17186`). A count-like magnitude with no declared
            // vocabulary, so `Unsigned`, not `Enum`.
            Field::new(region_field::INTENSITY, Kind::Unsigned),
        ],
    };

    let provinces = Collection {
        collection_id: PROVINCES,
        shape: Shape::KeyedSet,
        fields: vec![
            // ContentId for the same reason as d20's `location_province`.
            // Province 0 is a REAL row here (`world_fold.rs::fold_weather`),
            // unlike gameplay-facing province references.
            Field::new(province_field::PROVINCE_ID, Kind::ContentId).key(0),
            Field::new(province_field::TEMPERATURE, Kind::Continuous)
                .unknown_capable()
                .quant(quant),
            Field::new(province_field::TEMPERATURE_CHANGE, Kind::Continuous)
                .unknown_capable()
                .quant(quant),
            Field::new(province_field::WATER, Kind::Continuous)
                .unknown_capable()
                .quant(quant),
            Field::new(province_field::SNOW, Kind::Continuous)
                .unknown_capable()
                .quant(quant),
            Field::new(province_field::CUSTOM_MODIFIER_COUNT, Kind::Unsigned).unknown_capable(),
            // Gated by `MUD_PRESENT`, which is a measurement gate like the other
            // five `..._PRESENT` bits — see the module docs. Carrying the gate
            // as the field's unknown state is what makes the fold's
            // "mud_active implies mud_present" (`:17253`) unrepresentable.
            Field::new(province_field::MUD_ACTIVE, Kind::Boolean).unknown_capable(),
            // Ungated (`:24296`): the collector sets it on every row it emits.
            Field::new(province_field::STRUCT_VALIDATED, Kind::Boolean),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![regions, provinces],
        event_part: None,
    }
}

// --- The payload header ------------------------------------------------------

/// The 48-byte payload header (`world_fold.rs::fold_weather`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub region_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub province_count: u32,
    pub observed_region_rows: u32,
    pub observed_province_rows: u32,
    pub scale_raw: u32,
    pub active_region_count: u32,
    pub reserved0: u32,
    pub reserved1: u32,
}

impl PayloadHeader {
    /// `world_fold.rs::fold_weather`.
    pub fn is_delta(self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
}

/// Read the header words alone.
///
/// Split out from [`split_payload`] because the fold reads the header first and
/// only then decides whether to fold at all: a failure-status or
/// delta-before-baseline record returns early (`:17043`-`:17048`) and never
/// reaches the length and consistency checks. A harness that validated the whole
/// payload of a record the recorder discards unread would reject streams the
/// recorder accepts.
pub fn parse_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "weather payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        region_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        province_count: u32_at(payload, 20),
        observed_region_rows: u32_at(payload, 24),
        observed_province_rows: u32_at(payload, 28),
        scale_raw: u32_at(payload, 32),
        active_region_count: u32_at(payload, 36),
        reserved0: u32_at(payload, 40),
        reserved1: u32_at(payload, 44),
    })
}

/// Split a d24 DATA payload into its header, its region rows and its province
/// rows, applying every structural check `fold_weather` applies
/// (`world_fold.rs::fold_weather`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness
/// that can drift from the recorder without anything failing.
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = parse_header(payload)?;
    let region_count = header.region_count as usize;
    let province_count = header.province_count as usize;
    let observed_region_rows = header.observed_region_rows as usize;
    let observed_province_rows = header.observed_province_rows as usize;

    // Checked arithmetic throughout: the counts are attacker-shaped input from a
    // stream on disk and `count * ROW_LEN` overflows `usize` for a large enough
    // word. The fold saturates instead (`:17050`-`:17053`), which is safe there
    // only because the equality below then fails; checked is safe by
    // construction and rejects rather than mis-reporting the expected length.
    let region_bytes = region_count
        .checked_mul(REGION_ROW_LEN)
        .ok_or_else(|| format!("weather region_count {region_count} overflows"))?;
    let province_bytes = province_count
        .checked_mul(PROVINCE_ROW_LEN)
        .ok_or_else(|| format!("weather province_count {province_count} overflows"))?;
    let expected = HEADER_LEN
        .checked_add(region_bytes)
        .and_then(|len| len.checked_add(province_bytes))
        .ok_or_else(|| "weather payload length overflows".to_string())?;
    if payload.len() != expected {
        return Err(format!("weather len {} != {expected}", payload.len()));
    }

    // `:17058` — the scale is a contract, not data: the mapper's `/1e5` is only
    // correct while this holds.
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "weather scale_raw {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    // `:17059`
    if observed_region_rows != REGION_ROWS {
        return Err(format!(
            "weather observed_region_rows {observed_region_rows} != {REGION_ROWS}"
        ));
    }
    // `:17064`
    if observed_province_rows != 0
        && !(MIN_PROVINCE_ROWS..=MAX_PROVINCE_ROWS).contains(&observed_province_rows)
    {
        return Err(format!(
            "weather observed_province_rows {observed_province_rows} outside \
             {MIN_PROVINCE_ROWS}..={MAX_PROVINCE_ROWS}"
        ));
    }
    let delta = header.is_delta();
    // `:17072` — a full snapshot is the complete 305-region set.
    if !delta && region_count != REGION_ROWS {
        return Err(format!(
            "weather full snapshot region_count {region_count} != {REGION_ROWS}"
        ));
    }
    // `:17077`
    if !delta && observed_province_rows != 0 && province_count != observed_province_rows {
        return Err(format!(
            "weather full snapshot province_count {province_count} != \
             observed_province_rows {observed_province_rows}"
        ));
    }
    // `:17082`
    if region_count > observed_region_rows {
        return Err(format!(
            "weather region_count {region_count} > observed_region_rows {observed_region_rows}"
        ));
    }
    // `:17087`
    if observed_province_rows == 0 && province_count != 0 {
        return Err("weather province_count emitted with zero observed_province_rows".to_string());
    }
    // `:17092`
    if province_count > observed_province_rows {
        return Err(format!(
            "weather province_count {province_count} > observed_province_rows \
             {observed_province_rows}"
        ));
    }
    let emitted_rows = region_count.saturating_add(province_count);
    // `:17097`
    if header.changed_rows as usize > emitted_rows {
        return Err(format!(
            "weather changed_rows {} > emitted_rows {emitted_rows}",
            header.changed_rows
        ));
    }
    // `:17102` — a delta emits exactly the rows it changed.
    if delta && header.changed_rows as usize != emitted_rows {
        return Err(format!(
            "weather delta changed_rows {} != emitted_rows {emitted_rows}",
            header.changed_rows
        ));
    }
    // `:17107`
    if header.active_region_count as usize > region_count {
        return Err(format!(
            "weather active_region_count {} > region_count {region_count}",
            header.active_region_count
        ));
    }
    // `:17112`
    if header.reserved0 != 0 || header.reserved1 != 0 {
        return Err(format!(
            "weather reserved fields are nonzero ({}, {})",
            header.reserved0, header.reserved1
        ));
    }

    let region_end = HEADER_LEN + region_bytes;
    Ok((
        header,
        &payload[HEADER_LEN..region_end],
        &payload[region_end..],
    ))
}

// --- The boundary mapping: regions -------------------------------------------

/// The key of one compact region row: its `region_id`
/// (`world_fold.rs::fold_weather`).
pub fn key_from_region_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[region_id_of_row(row)? as u64]))
}

/// `region_id` with the fold's row-local bound (`world_fold.rs::fold_weather`). The
/// duplicate half of that same check is frame-level and lives in
/// [`WeatherState::fold_payload`].
pub fn region_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_len(row, REGION_ROW_LEN, "region")?;
    let region_id = u32_at(row, 0);
    if region_id > MAX_REGION_ID {
        return Err(format!("weather region row has invalid region_id {region_id}"));
    }
    Ok(region_id)
}

/// One compact row of the `regions` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 32-byte row slice (the caller has
/// already consumed the payload header via [`split_payload`]); d24 has exactly
/// one region row form, so length selects nothing and a wrong length is
/// rejected. `frame_tick` is §6's derived-field input
/// (stored = v + c * (frame_tick >> u)); d24 declares no derived field, so it is
/// taken and ignored — every domain's mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_weather`: a row the fold would reject
/// is an `Err` here, never a coerced value. Nothing in this function can panic
/// on any 32-byte input.
pub fn record_from_region_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    // The key is not carried in the record body (§3), but its validity is still
    // part of this row's validity.
    let _ = region_id_of_row(row)?;

    let flags = u32_at(row, 4);
    // `:17137` — an unknown flag bit means the row is not this grammar, so
    // nothing in it can be trusted.
    if flags & !REGION_KNOWN_FLAGS != 0 {
        return Err(format!("weather region row has unknown flags {flags:#x}"));
    }
    let temperature_raw = i32_at(row, 8);
    // `:17142`
    if !(-MAX_TEMPERATURE_RAW..=MAX_TEMPERATURE_RAW).contains(&temperature_raw) {
        return Err(format!(
            "weather region row has invalid temperature_raw {temperature_raw}"
        ));
    }
    let phenomenon = u32_at(row, 12);
    // `:17149`
    if phenomenon > MAX_RAW_ACTIVITY_CANDIDATE {
        return Err(format!(
            "weather region row has invalid raw +0x78 activity candidate {phenomenon}"
        ));
    }
    let next_weather_change_raw = u32_at(row, 16);
    // `:17154` — the source word is a signed i32 the collector refused to store
    // negative (`attach_log.rs:59072`-`:59075`).
    if next_weather_change_raw > i32::MAX as u32 {
        return Err(format!(
            "weather region row has invalid signed transition expiry \
             {next_weather_change_raw}"
        ));
    }
    // `:17159`-`:17165` — the three activity witnesses must agree. This is the
    // rule that makes `active_signal` and `phenomenon_active` derivable from
    // `phenomenon`, so it is enforced here rather than assumed.
    let active_signal = flags & REGION_ACTIVE_SIGNAL != 0;
    let phenomenon_active = flags & REGION_PHENOMENON_ACTIVE != 0;
    if active_signal != phenomenon_active || active_signal != (phenomenon != 0) {
        return Err("weather region row has inconsistent raw activity witnesses".to_string());
    }
    // `:17166`-`:17180` — at most one named weather bit. This is the rule that
    // makes the `Enum` a faithful encoding rather than a lossy one.
    let named: Vec<u32> = NAMED_WEATHER
        .iter()
        .filter(|(bit, _)| flags & *bit != 0)
        .map(|(_, code)| *code)
        .collect();
    if named.len() > 1 {
        return Err("weather region row overlaps mutually-exclusive named weather flags".to_string());
    }
    let kind = named.first().copied().unwrap_or(weather_kind::NO_PHENOMENON);
    let active_modifier_count = u32_at(row, 20);
    // `:17181`
    if active_modifier_count > MAX_ACTIVE_MODIFIERS {
        return Err(format!(
            "weather region row has invalid active_modifier_count {active_modifier_count}"
        ));
    }
    let intensity = u32_at(row, 24);
    // `:17186`
    if intensity > MAX_INTENSITY {
        return Err(format!("weather region row has invalid intensity {intensity}"));
    }
    // `:17194`
    if u32_at(row, 28) != 0 {
        return Err(format!(
            "weather region row reserved field is {}",
            u32_at(row, 28)
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            region_field::WEATHER_KIND => Value::Enum(kind),
            region_field::ARCTIC_WATER => Value::Bool(flags & REGION_ARCTIC_WATER != 0),
            region_field::TEMPERATURE => Value::Continuous(scaled(temperature_raw)),
            region_field::PHENOMENON => Value::Unsigned(phenomenon as u64),
            region_field::NEXT_WEATHER_CHANGE => Value::Unsigned(next_weather_change_raw as u64),
            region_field::ACTIVE_MODIFIER_COUNT => Value::Unsigned(active_modifier_count as u64),
            region_field::INTENSITY => Value::Unsigned(intensity as u64),
            // A collection this mapper does not know how to fill. Filling it with
            // a placeholder would publish invented data under a field id that
            // means something else.
            other => {
                return Err(format!(
                    "weather regions has no mapping for field id {other}"
                ))
            }
        });
    }
    Ok(Record::new(values))
}

// --- The boundary mapping: provinces -----------------------------------------

/// The key of one compact province row: its `province_id`
/// (`world_fold.rs::fold_weather`). **Province 0 is a real row** (`world_fold.rs::fold_weather`), so
/// there is no zero rejection here — the fold's only id check is the
/// `< observed_province_rows` bound, which is header-relative and therefore
/// frame-level ([`WeatherState::fold_payload`]).
pub fn key_from_province_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[province_id_of_row(row)? as u64]))
}

pub fn province_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_len(row, PROVINCE_ROW_LEN, "province")?;
    Ok(u32_at(row, 0))
}

/// The fold's header-relative province id bound (`world_fold.rs::fold_weather`). Public
/// because it cannot live in [`record_from_province_row`] — it needs the
/// payload header — and a caller that re-derives it can drift.
pub fn check_province_id(
    province_id: u32,
    observed_province_rows: u32,
) -> core::result::Result<(), String> {
    if province_id >= observed_province_rows {
        return Err(format!(
            "weather province row has invalid province_id {province_id} \
             (observed_province_rows {observed_province_rows})"
        ));
    }
    Ok(())
}

/// One compact row of the `provinces` table onto its stored Record. Same
/// contract as [`record_from_region_row`]; see [`check_province_id`] for the one
/// fold check this cannot make.
pub fn record_from_province_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let _ = province_id_of_row(row)?;

    let flags = u32_at(row, 4);
    // `:17239`
    if flags & !PROVINCE_KNOWN_FLAGS != 0 {
        return Err(format!("weather province row has unknown flags {flags:#x}"));
    }
    let temp_present = flags & PROVINCE_TEMP_PRESENT != 0;
    let temp_change_present = flags & PROVINCE_TEMP_CHANGE_PRESENT != 0;
    let water_present = flags & PROVINCE_WATER_PRESENT != 0;
    let snow_present = flags & PROVINCE_SNOW_PRESENT != 0;
    let mud_present = flags & PROVINCE_MUD_PRESENT != 0;
    let mud_active = flags & PROVINCE_MUD_ACTIVE != 0;
    let custom_present = flags & PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT != 0;
    let map_recolor = flags & PROVINCE_MAP_RECOLOR != 0;
    let dynamic_lut = flags & PROVINCE_DYNAMIC_LUT != 0;

    // `:17253` — the implication this schema turns into "a value needs its gate".
    if mud_active && !mud_present {
        return Err("weather province row has mud_active without mud_present".to_string());
    }
    // `:17258` — `map_recolor` is a function of the three presences, so a row
    // that disagrees is malformed and its flag word cannot be trusted.
    if map_recolor != (temp_present || water_present || snow_present) {
        return Err("weather province row has inconsistent map recolor flag".to_string());
    }
    // `:17263`
    if dynamic_lut != (snow_present || mud_present) {
        return Err("weather province row has inconsistent dynamic LUT flag".to_string());
    }

    // `validate_weather_i32_optional`, `world_fold.rs::validate_weather_i32_optional`: a present value
    // must be in range, and a gated-off field must pack a zero. The second half
    // is what makes an ungated non-zero word a malformed row rather than data.
    let temperature_raw = gated_i32(
        row,
        8,
        temp_present,
        -MAX_TEMPERATURE_RAW,
        MAX_TEMPERATURE_RAW,
        "temperature_raw",
    )?;
    let temperature_change_raw = gated_i32(
        row,
        12,
        temp_change_present,
        -MAX_TEMP_CHANGE_RAW,
        MAX_TEMP_CHANGE_RAW,
        "temperature_change_raw",
    )?;
    let water_raw = gated_i32(row, 16, water_present, 0, MAX_PRECIP_RAW, "water_raw")?;
    let snow_raw = gated_i32(row, 20, snow_present, 0, MAX_PRECIP_RAW, "snow_raw")?;

    let custom_modifier_count = u32_at(row, 24);
    // `:17300`-`:17310`
    if custom_present {
        if custom_modifier_count > MAX_ACTIVE_MODIFIERS {
            return Err(format!(
                "weather province row has invalid custom_modifier_count {custom_modifier_count}"
            ));
        }
    } else if custom_modifier_count != 0 {
        return Err(
            "weather province row has custom_modifier_count without presence flag".to_string(),
        );
    }
    // `:17311`
    if u32_at(row, 28) != 0 {
        return Err(format!(
            "weather province row reserved field is {}",
            u32_at(row, 28)
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            province_field::TEMPERATURE => gated_continuous(temp_present, temperature_raw),
            province_field::TEMPERATURE_CHANGE => {
                gated_continuous(temp_change_present, temperature_change_raw)
            }
            province_field::WATER => gated_continuous(water_present, water_raw),
            province_field::SNOW => gated_continuous(snow_present, snow_raw),
            province_field::CUSTOM_MODIFIER_COUNT => {
                if custom_present {
                    Value::Unsigned(custom_modifier_count as u64)
                } else {
                    Value::Unknown
                }
            }
            province_field::MUD_ACTIVE => {
                if mud_present {
                    Value::Bool(mud_active)
                } else {
                    Value::Unknown
                }
            }
            province_field::STRUCT_VALIDATED => {
                Value::Bool(flags & PROVINCE_STRUCT_VALIDATED != 0)
            }
            other => {
                return Err(format!(
                    "weather provinces has no mapping for field id {other}"
                ))
            }
        });
    }
    Ok(Record::new(values))
}

// --- The accumulator ---------------------------------------------------------

/// What one DATA record did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// `handle_compact_failure` (`world_fold.rs:17043`): the domain-hour is
    /// unknown and the accumulator is retained. Nothing was folded.
    FailureRetained,
    /// `reject_delta_before_baseline` (`:17046`): a delta with no baseline yet
    /// has no identity set to patch. Nothing was folded.
    DeltaBeforeBaseline,
    /// The record was applied. `reset` is true for a full snapshot, which
    /// replaces the whole state (`:17328`).
    Folded { reset: bool },
}

/// The reconstructed `weather_live` state, folded exactly as `fold_weather`
/// folds it (`world_fold.rs::fold_weather`).
///
/// d24 is baseline + delta, so an hour's state is *not* an hour's rows: it is
/// this accumulator after the hour's record. Every caller — the measurement
/// harness and the tests — folds through this one implementation, because a
/// second one is a second answer to "what did hour N hold".
#[derive(Clone, Debug, Default)]
pub struct WeatherState {
    pub regions: KeyedState,
    pub provinces: KeyedState,
    pub observed_region_rows: u32,
    pub observed_province_rows: u32,
    pub scale_raw: u32,
    /// `delta_baselines_seen` for this domain (`:17330`).
    pub baseline_seen: bool,
}

impl WeatherState {
    /// The active-region witness the fold recomputes over the *reconstructed*
    /// state after merging (`world_fold.rs::fold_weather`) — a derived value,
    /// which is why no field carries it.
    ///
    /// It is derived from `phenomenon`, not from a stored `active_signal` bit,
    /// because the fold proves the two identical for every accepted row
    /// (`world_fold.rs::fold_weather`).
    pub fn active_region_count(&self, collection: &Collection) -> usize {
        let Some(index) = collection
            .value_fields()
            .position(|f| f.field_id == region_field::PHENOMENON)
        else {
            return 0;
        };
        self.regions
            .values()
            .filter(|record| {
                matches!(record.values.get(index), Some(Value::Unsigned(p)) if *p != 0)
            })
            .count()
    }

    /// Fold one d24 DATA payload. `schema` must be [`schema()`]'s.
    ///
    /// Every rejection path is an `Err` — a payload the recorder would refuse is
    /// refused here, never partially applied. On `Err` the accumulator is left
    /// untouched: rows are built into scratch maps first, exactly as the fold
    /// builds `regions`/`provinces` vectors before merging (`:17118`, `:17216`).
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        frame_tick: u64,
    ) -> core::result::Result<FoldOutcome, String> {
        let header = parse_header(payload)?;
        // The fold's own early exits, in the fold's order. Both happen before
        // any structural validation of the payload body.
        if header.status_flags & FAILURE_STATUS_MASK != 0 {
            return Ok(FoldOutcome::FailureRetained);
        }
        if header.is_delta() && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        let (header, region_bytes, province_bytes) = split_payload(payload)?;
        let region_collection = schema
            .collections
            .iter()
            .find(|c| c.collection_id == REGIONS)
            .ok_or_else(|| "weather schema has no regions collection".to_string())?;
        let province_collection = schema
            .collections
            .iter()
            .find(|c| c.collection_id == PROVINCES)
            .ok_or_else(|| "weather schema has no provinces collection".to_string())?;

        let mut regions: KeyedState = BTreeMap::new();
        let mut computed_active = 0usize;
        for row in region_bytes.chunks_exact(REGION_ROW_LEN) {
            // `:17191` counts the ACTIVE_SIGNAL bit; the row mapper has already
            // proved it equals `phenomenon != 0`.
            if u32_at(row, 4) & REGION_ACTIVE_SIGNAL != 0 {
                computed_active += 1;
            }
            let k = key_from_region_row(row)?;
            let record = record_from_region_row(region_collection, row, frame_tick)?;
            // `:17132` — a duplicate id in one frame is a rejected frame, never a
            // last-writer-wins overwrite.
            if regions.insert(k, record).is_some() {
                return Err("weather frame repeats a region_id".to_string());
            }
        }
        // `:17210` — the header's witness is over the rows this record emitted.
        if computed_active != header.active_region_count as usize {
            return Err(format!(
                "weather active_region_count {} != computed {computed_active}",
                header.active_region_count
            ));
        }

        let mut provinces: KeyedState = BTreeMap::new();
        for row in province_bytes.chunks_exact(PROVINCE_ROW_LEN) {
            let province_id = province_id_of_row(row)?;
            check_province_id(province_id, header.observed_province_rows)?;
            let k = key_from_province_row(row)?;
            let record = record_from_province_row(province_collection, row, frame_tick)?;
            // `:17233`
            if provinces.insert(k, record).is_some() {
                return Err("weather frame repeats a province_id".to_string());
            }
        }

        // `:17328`-`:17343`: a full snapshot replaces, a delta upserts, and
        // nothing is ever deleted.
        let reset = !header.is_delta();
        if reset {
            self.regions.clear();
            self.provinces.clear();
            self.baseline_seen = true;
        }
        self.observed_region_rows = header.observed_region_rows;
        self.observed_province_rows = header.observed_province_rows;
        self.scale_raw = header.scale_raw;
        self.regions.extend(regions);
        self.provinces.extend(provinces);
        Ok(FoldOutcome::Folded { reset })
    }
}

/// `COMPACT_STATUS_FAILURE_MASK`, `compact_contract/src/lib.rs:110`. d24 has no
/// lane-local status bits and no expected-partial tolerance, so the mask is the
/// whole predicate for this domain; the measurement bin calls
/// `compact_contract`'s own function and asserts the two agree, so this constant
/// cannot drift unnoticed.
pub const FAILURE_STATUS_MASK: u32 = (1 << 0)   // STATE_UNAVAILABLE
    | (1 << 1)  // PROFILE_MISMATCH
    | (1 << 2)  // NO_CLOCK
    | (1 << 3)  // NO_SCALAR_SOURCE
    | (1 << 6)  // WORKER_PANIC
    | (1 << 7)  // BRIDGE_UNAVAILABLE
    | (1 << 8)  // SOURCE_UNAVAILABLE
    | (1 << 9)  // PARTIAL_DATA
    | (1 << 12); // CONTROL_INVALID

// --- Helpers -----------------------------------------------------------------

/// The one place the fixed-point division happens. `scale_raw` is asserted equal
/// to `FIXED_SCALE` by [`split_payload`], so the divisor is a constant.
fn scaled(raw: i32) -> f64 {
    raw as f64 / FIXED_SCALE
}

/// A gate that is clear means the value was not carried, not that it is nothing
/// — see the module docs. §3 calls that **unknown**.
fn gated_continuous(present: bool, raw: i32) -> Value {
    if present {
        Value::Continuous(scaled(raw))
    } else {
        Value::Unknown
    }
}

fn gated_i32(
    row: &[u8],
    offset: usize,
    present: bool,
    min: i32,
    max: i32,
    label: &str,
) -> core::result::Result<i32, String> {
    let raw = i32_at(row, offset);
    if present {
        if !(min..=max).contains(&raw) {
            return Err(format!("weather province row has invalid {label} {raw}"));
        }
    } else if raw != 0 {
        return Err(format!(
            "weather province row has {label} without presence flag"
        ));
    }
    Ok(raw)
}

fn check_len(row: &[u8], expected: usize, what: &str) -> core::result::Result<(), String> {
    if row.len() != expected {
        return Err(format!(
            "weather {what} row length {} != {expected}",
            row.len()
        ));
    }
    Ok(())
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{escapes, quantize};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn region_collection() -> Collection {
        schema().collections[0].clone()
    }

    fn province_collection() -> Collection {
        schema().collections[1].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 32-byte region row, packed the way the collector packs one
    /// (`attach_log.rs:59080`-`:59113`).
    fn region_row(
        region_id: u32,
        flags: u32,
        temperature_raw: i32,
        phenomenon: u32,
        next_change: u32,
        active_modifier_count: u32,
        intensity: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(REGION_ROW_LEN);
        row.extend_from_slice(&region_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&temperature_raw.to_le_bytes());
        row.extend_from_slice(&phenomenon.to_le_bytes());
        row.extend_from_slice(&next_change.to_le_bytes());
        row.extend_from_slice(&active_modifier_count.to_le_bytes());
        row.extend_from_slice(&intensity.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(row.len(), REGION_ROW_LEN);
        row
    }

    /// The `flags | phenomenon` pair for an active region: the three activity
    /// witnesses have to agree or the fold rejects the row (`:17161`).
    fn active(named: u32) -> (u32, u32) {
        (named | REGION_ACTIVE_SIGNAL | REGION_PHENOMENON_ACTIVE, 1)
    }

    /// A raw 32-byte province row (`attach_log.rs:59157`-`:59191`).
    fn province_row(
        province_id: u32,
        flags: u32,
        temperature_raw: i32,
        temperature_change_raw: i32,
        water_raw: i32,
        snow_raw: i32,
        custom_modifier_count: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(PROVINCE_ROW_LEN);
        row.extend_from_slice(&province_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&temperature_raw.to_le_bytes());
        row.extend_from_slice(&temperature_change_raw.to_le_bytes());
        row.extend_from_slice(&water_raw.to_le_bytes());
        row.extend_from_slice(&snow_raw.to_le_bytes());
        row.extend_from_slice(&custom_modifier_count.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(row.len(), PROVINCE_ROW_LEN);
        row
    }

    /// The flag word the collector emits for a fully-read province
    /// (`attach_log.rs:59157`-`:59174`): every presence set, `MUD_ACTIVE` and
    /// the two driver bits per their rules. This is flag word 991/1023 — the
    /// only two words in the 542,108 province rows of the deep-war capture.
    fn full_province_flags(mud_active: bool) -> u32 {
        let mut flags = PROVINCE_TEMP_PRESENT
            | PROVINCE_TEMP_CHANGE_PRESENT
            | PROVINCE_WATER_PRESENT
            | PROVINCE_SNOW_PRESENT
            | PROVINCE_MUD_PRESENT
            | PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT
            | PROVINCE_STRUCT_VALIDATED
            | PROVINCE_MAP_RECOLOR
            | PROVINCE_DYNAMIC_LUT;
        if mud_active {
            flags |= PROVINCE_MUD_ACTIVE;
        }
        flags
    }

    /// A well-formed payload from explicit rows.
    fn payload(
        status_flags: u32,
        observed_province_rows: u32,
        regions: &[Vec<u8>],
        provinces: &[Vec<u8>],
        active_region_count: u32,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(regions.len() as u32).to_le_bytes());
        p.extend_from_slice(&((regions.len() + provinces.len()) as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(provinces.len() as u32).to_le_bytes());
        p.extend_from_slice(&(REGION_ROWS as u32).to_le_bytes());
        p.extend_from_slice(&observed_province_rows.to_le_bytes());
        p.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        p.extend_from_slice(&active_region_count.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        assert_eq!(p.len(), HEADER_LEN);
        for r in regions {
            p.extend_from_slice(r);
        }
        for r in provinces {
            p.extend_from_slice(r);
        }
        p
    }

    /// A full 305-region / N-province snapshot payload the fold accepts.
    fn baseline_payload(province_rows: u32) -> Vec<u8> {
        let regions: Vec<Vec<u8>> = (0..REGION_ROWS as u32)
            .map(|id| {
                if id % 3 == 0 {
                    let (flags, phenomenon) = active(REGION_RAIN_LIGHT);
                    region_row(id, flags, 1_234_567, phenomenon, 60_832_505, 1, 2)
                } else {
                    region_row(id, 0, -50_000, 0, 60_832_505, 0, 0)
                }
            })
            .collect();
        let active_regions = (0..REGION_ROWS as u32).filter(|id| id % 3 == 0).count() as u32;
        let provinces: Vec<Vec<u8>> = (0..province_rows)
            .map(|id| {
                province_row(
                    id,
                    full_province_flags(id % 5 == 0),
                    100_000 + id as i32,
                    -2_500,
                    3_000,
                    0,
                    if id % 7 == 0 { 1 } else { 0 },
                )
            })
            .collect();
        payload(0, province_rows, &regions, &provinces, active_regions)
    }

    /// A full snapshot with no province rows at all. `observed_province_rows`
    /// may legally be zero (`world_fold.rs::fold_weather`), and then `province_count`
    /// must be zero too (`world_fold.rs::fold_weather`) — the cheap fixture for whole-payload sweeps.
    fn region_only_baseline() -> Vec<u8> {
        let regions: Vec<Vec<u8>> = (0..REGION_ROWS as u32)
            .map(|id| region_row(id, 0, -50_000, 0, 60_832_505, 0, 0))
            .collect();
        payload(0, 0, &regions, &[], 0)
    }

    // --- Step 8.1: the schema is well-formed ---------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d24 schema is well-formed");
        // And on the hourly grid the container actually declares.
        schema().validate(TickGrid::HOURLY.log2()).expect("hourly");
    }

    // --- Step 8.5: shape and key ---------------------------------------------

    #[test]
    fn both_collections_are_keyed_sets_keyed_by_a_content_id() {
        let s = schema();
        assert_eq!(s.domain, DOMAIN);
        assert_eq!(s.collections.len(), 2, "regions and provinces");
        assert_eq!(s.cadence_log2, 0, "one DATA record per game hour");
        assert!(s.event_part.is_none(), "d24 is state, not an event stream");

        let regions = region_collection();
        assert_eq!(regions.collection_id, REGIONS);
        assert_eq!(regions.shape, Shape::KeyedSet, "baseline resets, deltas upsert");
        let keys = regions.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys regions by region_id alone");
        assert_eq!(keys[0].field_id, region_field::REGION_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a strategic region is content-defined (§5)"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(regions.fields.len(), 8, "1 key + 7 values");

        let provinces = province_collection();
        assert_eq!(provinces.collection_id, PROVINCES);
        assert_eq!(provinces.shape, Shape::KeyedSet);
        let keys = provinces.key_fields();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].field_id, province_field::PROVINCE_ID);
        assert_eq!(keys[0].kind, Kind::ContentId);
        assert_eq!(provinces.fields.len(), 8, "1 key + 7 values");
    }

    #[test]
    fn province_zero_is_a_real_key() {
        // `world_fold.rs::fold_weather`: the engine array is indexed
        // `0..observed-1`, so id 0 is a completeness row, not a null reference.
        // The deep-war capture carries it in every baseline.
        let row = province_row(0, full_province_flags(false), 0, 0, 0, 0, 0);
        assert_eq!(key_from_province_row(&row).expect("keys"), key(&[0]));
        assert!(check_province_id(0, 13_414).is_ok());
    }

    // --- Step 8.3: the states, through the mapper ----------------------------

    #[test]
    fn the_region_collection_declares_no_empty_state_at_all() {
        // No region field is gated: `fold_weather` reads all seven values
        // unconditionally (`world_fold.rs::fold_weather`), so the region
        // collection can never emit Absent, Null or Unknown and must declare
        // none of the three. Step 8.3's per-capability raw-bytes cases therefore
        // do not apply to this collection; the assertion below is what keeps the
        // declaration honest, and `every_region_field_is_a_value_in_every_row`
        // is the mapper-side half.
        let c = region_collection();
        assert_eq!(c.optional_count(), 0);
        assert!(c.value_fields().all(|f| !f.nullable));
        assert!(c.value_fields().all(|f| !f.unknown_capable));
    }

    #[test]
    fn every_region_field_is_a_value_in_every_row() {
        let c = region_collection();
        // The whole legal flag space, not one sample: every combination of the
        // independent bits crossed with every named-weather choice.
        for (named, _) in NAMED_WEATHER.iter().copied().chain([(0, 0)]) {
            for arctic in [0, REGION_ARCTIC_WATER] {
                for (activity, phenomenon) in
                    [(0u32, 0u32), (REGION_ACTIVE_SIGNAL | REGION_PHENOMENON_ACTIVE, 1)]
                {
                    if named != 0 && activity == 0 {
                        // The capture never shows a named phenomenon on an
                        // inactive region, but the fold does not forbid it, so it
                        // is exercised below rather than skipped.
                    }
                    let flags = named | arctic | activity;
                    let row = region_row(7, flags, 0, phenomenon, 0, 0, 0);
                    let r = record_from_region_row(&c, &row, 0)
                        .unwrap_or_else(|e| panic!("flags {flags:#x}: {e}"));
                    assert!(
                        r.values.iter().all(|v| v.carries_value()),
                        "flags {flags:#x} produced an empty state"
                    );
                }
            }
        }
    }

    #[test]
    fn the_province_collection_declares_unknown_and_nothing_else() {
        // The schema declares exactly one capability, so the mapper may emit
        // exactly one empty state. Absent would claim the concept does not apply
        // to a province; Null would claim we know there is nothing. A clear gate
        // means the value was not carried — see the module docs.
        let c = province_collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(
            c.value_fields().all(|f| !f.nullable),
            "no field is a relation that can point at nothing"
        );
        let unknown_capable: Vec<u32> = c
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            unknown_capable,
            vec![
                province_field::TEMPERATURE,
                province_field::TEMPERATURE_CHANGE,
                province_field::WATER,
                province_field::SNOW,
                province_field::CUSTOM_MODIFIER_COUNT,
                province_field::MUD_ACTIVE,
            ],
            "the six gated fields, and only those"
        );
        assert!(
            !c.value_fields()
                .find(|f| f.field_id == province_field::STRUCT_VALIDATED)
                .expect("field")
                .unknown_capable,
            "struct_validated is ungated (`world_fold.rs:24222`)"
        );
    }

    #[test]
    fn a_clear_province_gate_is_unknown_field_by_field() {
        let c = province_collection();
        // One gate clear at a time, over the whole gate space. Each case builds
        // the flag word the fold would accept: a gated-off scalar must pack a
        // zero, and the two driver bits follow their rules.
        let cases: [(u32, u32); 6] = [
            (PROVINCE_TEMP_PRESENT, province_field::TEMPERATURE),
            (PROVINCE_TEMP_CHANGE_PRESENT, province_field::TEMPERATURE_CHANGE),
            (PROVINCE_WATER_PRESENT, province_field::WATER),
            (PROVINCE_SNOW_PRESENT, province_field::SNOW),
            (
                PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT,
                province_field::CUSTOM_MODIFIER_COUNT,
            ),
            (PROVINCE_MUD_PRESENT, province_field::MUD_ACTIVE),
        ];
        for (cleared, gated_id) in cases {
            let flags = full_province_flags(false) & !cleared;
            if cleared == PROVINCE_MUD_PRESENT {
                // `:17263`: dynamic_lut == (snow_present || mud_present), and
                // snow is still present here, so the driver bit stays set.
                assert!(flags & PROVINCE_DYNAMIC_LUT != 0);
            }
            if cleared == PROVINCE_SNOW_PRESENT {
                assert!(flags & PROVINCE_MAP_RECOLOR != 0, "temp and water remain");
            }
            let row = province_row(
                1,
                flags,
                if flags & PROVINCE_TEMP_PRESENT != 0 { 100_000 } else { 0 },
                if flags & PROVINCE_TEMP_CHANGE_PRESENT != 0 { -100_000 } else { 0 },
                if flags & PROVINCE_WATER_PRESENT != 0 { 200_000 } else { 0 },
                if flags & PROVINCE_SNOW_PRESENT != 0 { 300_000 } else { 0 },
                if flags & PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT != 0 { 3 } else { 0 },
            );
            let r = record_from_province_row(&c, &row, 0)
                .unwrap_or_else(|e| panic!("flags {flags:#x}: {e}"));
            for f in c.value_fields() {
                assert_eq!(
                    r.values[index_of(&c, f.field_id)] == Value::Unknown,
                    f.field_id == gated_id,
                    "field {} under flags {flags:#x}",
                    f.field_id
                );
            }
        }
    }

    #[test]
    fn the_one_gate_the_real_capture_clears_maps_to_unknown() {
        // The reviewer-checkable case: across 542,108 province rows in
        // `runs/batch1/deepwar_t1`, exactly one row has a clear gate — province
        // 0 in the baseline, flag word 988 (`0b11_1101_1100`), every value zero.
        // Its temperature and temperature_change are Unknown, and every other
        // field is a real value including a real Zero.
        let c = province_collection();
        let flags = 0b11_1101_1100u32;
        assert_eq!(flags & PROVINCE_TEMP_PRESENT, 0);
        assert_eq!(flags & PROVINCE_TEMP_CHANGE_PRESENT, 0);
        let r = record_from_province_row(&c, &province_row(0, flags, 0, 0, 0, 0, 0), 0)
            .expect("the capture's own row maps");
        assert_eq!(r.values[index_of(&c, province_field::TEMPERATURE)], Value::Unknown);
        assert_eq!(
            r.values[index_of(&c, province_field::TEMPERATURE_CHANGE)],
            Value::Unknown
        );
        assert_eq!(
            r.values[index_of(&c, province_field::WATER)],
            Value::Continuous(0.0),
            "a measured zero is a value, never an empty state"
        );
        assert_eq!(r.values[index_of(&c, province_field::SNOW)], Value::Continuous(0.0));
        assert_eq!(
            r.values[index_of(&c, province_field::CUSTOM_MODIFIER_COUNT)],
            Value::Unsigned(0)
        );
        assert_eq!(r.values[index_of(&c, province_field::MUD_ACTIVE)], Value::Bool(false));
        assert_eq!(
            r.values[index_of(&c, province_field::STRUCT_VALIDATED)],
            Value::Bool(true)
        );
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        let rc = region_collection();
        let r = record_from_region_row(&rc, &region_row(0, 0, 0, 0, 0, 0, 0), 0).expect("maps");
        assert_eq!(r.values[index_of(&rc, region_field::TEMPERATURE)], Value::Continuous(0.0));
        assert_eq!(r.values[index_of(&rc, region_field::PHENOMENON)], Value::Unsigned(0));
        assert_eq!(
            r.values[index_of(&rc, region_field::NEXT_WEATHER_CHANGE)],
            Value::Unsigned(0)
        );
        assert_eq!(
            r.values[index_of(&rc, region_field::WEATHER_KIND)],
            Value::Enum(weather_kind::NO_PHENOMENON)
        );

        let pc = province_collection();
        let r = record_from_province_row(&pc, &province_row(9, full_province_flags(false), 0, 0, 0, 0, 0), 0)
            .expect("maps");
        for id in [
            province_field::TEMPERATURE,
            province_field::TEMPERATURE_CHANGE,
            province_field::WATER,
            province_field::SNOW,
        ] {
            assert_eq!(r.values[index_of(&pc, id)], Value::Continuous(0.0), "field {id}");
        }
        assert_eq!(
            r.values[index_of(&pc, province_field::CUSTOM_MODIFIER_COUNT)],
            Value::Unsigned(0)
        );
    }

    // --- Step 8.2: byte-identical round trips --------------------------------

    #[test]
    fn a_full_region_row_roundtrips_byte_for_byte() {
        let c = region_collection();
        let (flags, phenomenon) = active(REGION_SANDSTORM);
        let row = region_row(
            304,
            flags | REGION_ARCTIC_WATER,
            3_642_623,
            phenomenon,
            60_832_505,
            4_096,
            255,
        );
        let record = record_from_region_row(&c, &row, 0).expect("maps");
        assert_roundtrips(&c, &record);
    }

    #[test]
    fn a_full_province_row_roundtrips_byte_for_byte() {
        let c = province_collection();
        let row = province_row(13_413, full_province_flags(true), -896_588, 12_345, 20_000_000, 8_000_000, 4_096);
        let record = record_from_province_row(&c, &row, 0).expect("maps");
        assert_roundtrips(&c, &record);
    }

    #[test]
    fn a_province_row_with_unknowns_roundtrips_byte_for_byte() {
        let c = province_collection();
        // Every gate clear at once: the flag word is `STRUCT_VALIDATED` alone,
        // which the fold accepts because both driver bits are then correctly 0.
        let row = province_row(1, PROVINCE_STRUCT_VALIDATED, 0, 0, 0, 0, 0);
        let record = record_from_province_row(&c, &row, 0).expect("maps");
        for id in [
            province_field::TEMPERATURE,
            province_field::TEMPERATURE_CHANGE,
            province_field::WATER,
            province_field::SNOW,
            province_field::CUSTOM_MODIFIER_COUNT,
            province_field::MUD_ACTIVE,
        ] {
            assert_eq!(record.values[index_of(&c, id)], Value::Unknown, "field {id}");
        }
        assert_roundtrips(&c, &record);
    }

    fn assert_roundtrips(c: &Collection, record: &Record) {
        let mut bytes = Vec::new();
        record.encode(c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(&back, record, "decode reproduces the record");
        let mut reencoded = Vec::new();
        back.encode(c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    // --- Step 8.6: every scale factor ----------------------------------------

    #[test]
    fn every_scaled_field_divides_by_exactly_one_hundred_thousand() {
        // The test that catches a 1e5-vs-32768-vs-raw mapping error: known
        // fixed-point words against their exact f64 readings, each at its own
        // offset so a transposed pair fails too.
        let rc = region_collection();
        let r = record_from_region_row(&rc, &region_row(3, 0, -896_588, 0, 100_000, 4_096, 100), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&rc, region_field::TEMPERATURE)],
            Value::Continuous(-8.96588)
        );
        // And the raw-emitted words are NOT divided (`world_fold.rs:24245`-`:24249`).
        assert_eq!(
            r.values[index_of(&rc, region_field::NEXT_WEATHER_CHANGE)],
            Value::Unsigned(100_000)
        );
        assert_eq!(
            r.values[index_of(&rc, region_field::ACTIVE_MODIFIER_COUNT)],
            Value::Unsigned(4_096)
        );
        assert_eq!(r.values[index_of(&rc, region_field::INTENSITY)], Value::Unsigned(100));

        let pc = province_collection();
        let r = record_from_province_row(
            &pc,
            &province_row(4, full_province_flags(false), 2_500_000, -125_000, 20_000_000, 750, 4_096),
            0,
        )
        .expect("maps");
        assert_eq!(r.values[index_of(&pc, province_field::TEMPERATURE)], Value::Continuous(25.0));
        assert_eq!(
            r.values[index_of(&pc, province_field::TEMPERATURE_CHANGE)],
            Value::Continuous(-1.25)
        );
        assert_eq!(r.values[index_of(&pc, province_field::WATER)], Value::Continuous(200.0));
        assert_eq!(r.values[index_of(&pc, province_field::SNOW)], Value::Continuous(0.0075));
        assert_eq!(
            r.values[index_of(&pc, province_field::CUSTOM_MODIFIER_COUNT)],
            Value::Unsigned(4_096),
            "a count is emitted raw, never divided"
        );
    }

    #[test]
    fn fixed_five_is_lossless_over_every_declared_raw() {
        // `Quant::Fixed(5)` is chosen because it is the source's own precision,
        // so the claim to prove is `quantize(raw / 1e5) == raw` with no escape,
        // over the whole range the fold admits — not "the error is small".
        let q = Quant::Fixed(SCALE_DIGITS);
        let check = |raw: i32| {
            let v = scaled(raw);
            assert!(!escapes(v, q), "raw {raw} escaped fixed(5)");
            assert_eq!(quantize(v, q), Some(raw as i64), "raw {raw} did not survive");
        };
        // Both signed bounds, both zeroes, the precipitation ceiling, and the
        // extremes the deep-war capture actually reached.
        for raw in [
            0,
            1,
            -1,
            MAX_TEMPERATURE_RAW,
            -MAX_TEMPERATURE_RAW,
            MAX_PRECIP_RAW,
            -MAX_PRECIP_RAW,
            -896_588,
            3_642_623,
            20_000_000,
            8_000_000,
        ] {
            check(raw);
        }
        // Exhaustive across the unit interval, where rounding ties live.
        for raw in -100_000..=100_000 {
            check(raw);
        }
        // A stride over the whole precipitation range, coprime with 10^5 so the
        // samples do not all land on the same residue.
        let mut raw: i64 = -(MAX_PRECIP_RAW as i64);
        while raw <= MAX_PRECIP_RAW as i64 {
            check(raw as i32);
            raw += 10_007;
        }
    }

    // --- Step 8.4: quantization saves bytes ----------------------------------

    #[test]
    fn quantization_saves_bytes_against_an_all_exact_variant() {
        let quantized = province_collection();
        let mut exact = quantized.clone();
        for f in &mut exact.fields {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        let mut quantized_len = 0usize;
        let mut exact_len = 0usize;
        for id in 0..64u32 {
            let row = province_row(
                id,
                full_province_flags(id % 2 == 0),
                1_234_567 + id as i32,
                -2_500,
                3_000 * id as i32,
                id as i32,
                id % 5,
            );
            for (c, total) in [(&quantized, &mut quantized_len), (&exact, &mut exact_len)] {
                let mut bytes = Vec::new();
                record_from_province_row(c, &row, 0)
                    .expect("maps")
                    .encode(c, &mut bytes)
                    .expect("encodes");
                *total += bytes.len();
            }
        }
        assert!(
            quantized_len < exact_len,
            "fixed(5) must beat exact: {quantized_len} vs {exact_len}"
        );
        // The region collection's single continuous field, likewise.
        let region_quantized = region_collection();
        let mut region_exact = region_quantized.clone();
        for f in &mut region_exact.fields {
            if matches!(f.kind, Kind::Continuous) {
                f.quant = Quant::Exact;
            }
        }
        let row = region_row(1, 0, -896_588, 0, 60_832_505, 0, 0);
        let mut a = Vec::new();
        let mut b = Vec::new();
        record_from_region_row(&region_quantized, &row, 0)
            .expect("maps")
            .encode(&region_quantized, &mut a)
            .expect("encodes");
        record_from_region_row(&region_exact, &row, 0)
            .expect("maps")
            .encode(&region_exact, &mut b)
            .expect("encodes");
        assert!(a.len() < b.len(), "fixed(5) must beat exact: {} vs {}", a.len(), b.len());
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers. `next_weather_change` is
        // the one candidate and it is an ABSOLUTE game hour
        // (`replay_core/src/world.rs:762`), already constant between weather
        // changes, so `frame_tick` never enters a value. The mapper takes it and
        // must ignore it — proved by mapping the same row at two ticks.
        for c in [region_collection(), province_collection()] {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
        let rc = region_collection();
        let row = region_row(1, 0, 1, 0, 60_832_505, 0, 0);
        assert_eq!(
            record_from_region_row(&rc, &row, 0).expect("maps"),
            record_from_region_row(&rc, &row, u64::MAX).expect("maps")
        );
        let pc = province_collection();
        let prow = province_row(1, full_province_flags(true), 1, 1, 1, 1, 1);
        assert_eq!(
            record_from_province_row(&pc, &prow, 0).expect("maps"),
            record_from_province_row(&pc, &prow, u64::MAX).expect("maps")
        );
    }

    // --- The four derived flags this schema does not carry -------------------

    #[test]
    fn the_activity_witnesses_are_recoverable_from_phenomenon() {
        // `world_fold.rs::fold_weather` makes active_signal == phenomenon_active ==
        // (phenomenon != 0) on every accepted row, so the two flag bits are a
        // function of a carried field. Over the whole legal phenomenon space.
        let c = region_collection();
        let phen_index = index_of(&c, region_field::PHENOMENON);
        for phenomenon in 0..=MAX_RAW_ACTIVITY_CANDIDATE {
            let flags = if phenomenon == 0 {
                0
            } else {
                REGION_ACTIVE_SIGNAL | REGION_PHENOMENON_ACTIVE
            };
            let row = region_row(2, flags, 0, phenomenon, 0, 0, 0);
            let r = record_from_region_row(&c, &row, 0).expect("maps");
            let recovered = matches!(r.values[phen_index], Value::Unsigned(p) if p != 0);
            assert_eq!(recovered, flags & REGION_ACTIVE_SIGNAL != 0);
            assert_eq!(recovered, flags & REGION_PHENOMENON_ACTIVE != 0);
        }
    }

    #[test]
    fn the_two_driver_flags_are_recoverable_from_the_unknown_bits() {
        // `:17258` and `:17263`. Over every legal province flag word, the two
        // driver bits are exactly the record's own presence pattern — which is
        // why they are not fields.
        let c = province_collection();
        let temp_i = index_of(&c, province_field::TEMPERATURE);
        let water_i = index_of(&c, province_field::WATER);
        let snow_i = index_of(&c, province_field::SNOW);
        let mud_i = index_of(&c, province_field::MUD_ACTIVE);
        let gates = [
            PROVINCE_TEMP_PRESENT,
            PROVINCE_TEMP_CHANGE_PRESENT,
            PROVINCE_WATER_PRESENT,
            PROVINCE_SNOW_PRESENT,
            PROVINCE_MUD_PRESENT,
        ];
        let mut accepted = 0usize;
        for mask in 0..(1u32 << gates.len()) {
            for mud_active in [false, true] {
                for custom in [false, true] {
                    for validated in [false, true] {
                        let mut flags = 0u32;
                        for (i, gate) in gates.iter().enumerate() {
                            if mask & (1 << i) != 0 {
                                flags |= gate;
                            }
                        }
                        if mud_active {
                            flags |= PROVINCE_MUD_ACTIVE;
                        }
                        if custom {
                            flags |= PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT;
                        }
                        if validated {
                            flags |= PROVINCE_STRUCT_VALIDATED;
                        }
                        let temp = flags & PROVINCE_TEMP_PRESENT != 0;
                        let water = flags & PROVINCE_WATER_PRESENT != 0;
                        let snow = flags & PROVINCE_SNOW_PRESENT != 0;
                        let mud = flags & PROVINCE_MUD_PRESENT != 0;
                        if temp || water || snow {
                            flags |= PROVINCE_MAP_RECOLOR;
                        }
                        if snow || mud {
                            flags |= PROVINCE_DYNAMIC_LUT;
                        }
                        let row = province_row(1, flags, 0, 0, 0, 0, 0);
                        let Ok(r) = record_from_province_row(&c, &row, 0) else {
                            assert!(mud_active && !mud, "only mud_active without mud is illegal");
                            continue;
                        };
                        accepted += 1;
                        let map_recolor = r.values[temp_i] != Value::Unknown
                            || r.values[water_i] != Value::Unknown
                            || r.values[snow_i] != Value::Unknown;
                        let dynamic_lut = r.values[snow_i] != Value::Unknown
                            || r.values[mud_i] != Value::Unknown;
                        assert_eq!(map_recolor, flags & PROVINCE_MAP_RECOLOR != 0, "{flags:#x}");
                        assert_eq!(dynamic_lut, flags & PROVINCE_DYNAMIC_LUT != 0, "{flags:#x}");
                    }
                }
            }
        }
        assert!(accepted > 100, "the sweep must actually accept rows");
    }

    #[test]
    fn the_weather_kind_enum_is_the_folds_own_collapse() {
        // `world_fold.rs:24228`-`:24240` and `replay_core/src/world.rs:7533`.
        // Because the five bits are mutually exclusive (`:17176`), the if-chain
        // order cannot change the answer — but the mapping bit -> code must.
        let c = region_collection();
        let kind_i = index_of(&c, region_field::WEATHER_KIND);
        for (bit, code) in NAMED_WEATHER {
            let (flags, phenomenon) = active(bit);
            let row = region_row(1, flags, 0, phenomenon, 0, 0, 0);
            let r = record_from_region_row(&c, &row, 0).expect("maps");
            assert_eq!(r.values[kind_i], Value::Enum(code), "bit {bit:#x}");
            assert!(code < weather_kind::CARDINALITY);
        }
        // Active with no named bit is a real state: 920 of the 1,671 region rows
        // in the deep-war capture are exactly this.
        let (flags, phenomenon) = active(0);
        let r = record_from_region_row(&c, &region_row(1, flags, 0, phenomenon, 0, 0, 0), 0)
            .expect("maps");
        assert_eq!(r.values[kind_i], Value::Enum(weather_kind::NO_PHENOMENON));
        // And arctic_water is NOT part of the chain (`world.rs:7576`).
        let r = record_from_region_row(
            &c,
            &region_row(1, REGION_ARCTIC_WATER, 0, 0, 0, 0, 0),
            0,
        )
        .expect("maps");
        assert_eq!(r.values[kind_i], Value::Enum(weather_kind::NO_PHENOMENON));
        assert_eq!(r.values[index_of(&c, region_field::ARCTIC_WATER)], Value::Bool(true));
    }

    // --- Rejection: every row the fold rejects -------------------------------

    #[test]
    fn the_region_mapper_rejects_every_row_the_fold_rejects() {
        let c = region_collection();
        for len in [0usize, 1, REGION_ROW_LEN - 1, REGION_ROW_LEN + 1, 48] {
            assert!(
                record_from_region_row(&c, &vec![0u8; len], 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_region_row(&vec![0u8; len]).is_err());
        }
        // region_id above the bound (`:17132`).
        assert!(record_from_region_row(&c, &region_row(305, 0, 0, 0, 0, 0, 0), 0).is_err());
        assert!(record_from_region_row(&c, &region_row(u32::MAX, 0, 0, 0, 0, 0, 0), 0).is_err());
        assert!(record_from_region_row(&c, &region_row(304, 0, 0, 0, 0, 0, 0), 0).is_ok());
        // Unknown flag bits (`:17137`) — every one of them.
        for bit in 8..32u32 {
            assert!(
                record_from_region_row(&c, &region_row(1, 1 << bit, 0, 0, 0, 0, 0), 0).is_err(),
                "flag bit {bit} is not in REGION_KNOWN_FLAGS"
            );
        }
        // Temperature out of range, both directions, and the bounds themselves.
        for raw in [MAX_TEMPERATURE_RAW + 1, -MAX_TEMPERATURE_RAW - 1, i32::MIN, i32::MAX] {
            assert!(record_from_region_row(&c, &region_row(1, 0, raw, 0, 0, 0, 0), 0).is_err());
        }
        for raw in [MAX_TEMPERATURE_RAW, -MAX_TEMPERATURE_RAW] {
            assert!(record_from_region_row(&c, &region_row(1, 0, raw, 0, 0, 0, 0), 0).is_ok());
        }
        // phenomenon above 16 (`:17149`) — but a phenomenon in 1..=16 needs its
        // witnesses, so the in-range side is checked with them set.
        for phenomenon in [MAX_RAW_ACTIVITY_CANDIDATE + 1, u32::MAX] {
            let flags = REGION_ACTIVE_SIGNAL | REGION_PHENOMENON_ACTIVE;
            assert!(record_from_region_row(&c, &region_row(1, flags, 0, phenomenon, 0, 0, 0), 0).is_err());
        }
        // Signed transition expiry (`:17154`).
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, i32::MAX as u32 + 1, 0, 0), 0).is_err());
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, i32::MAX as u32, 0, 0), 0).is_ok());
        // Disagreeing activity witnesses (`:17161`), all eight combinations.
        for signal in [0, REGION_ACTIVE_SIGNAL] {
            for active_bit in [0, REGION_PHENOMENON_ACTIVE] {
                for phenomenon in [0u32, 1] {
                    let flags = signal | active_bit;
                    let agree = (signal != 0) == (active_bit != 0)
                        && (signal != 0) == (phenomenon != 0);
                    let row = region_row(1, flags, 0, phenomenon, 0, 0, 0);
                    assert_eq!(
                        record_from_region_row(&c, &row, 0).is_ok(),
                        agree,
                        "signal {signal:#x} active {active_bit:#x} phenomenon {phenomenon}"
                    );
                }
            }
        }
        // Overlapping named weather (`:17176`), every unordered pair.
        for i in 0..NAMED_WEATHER.len() {
            for j in (i + 1)..NAMED_WEATHER.len() {
                let (flags, phenomenon) = active(NAMED_WEATHER[i].0 | NAMED_WEATHER[j].0);
                assert!(
                    record_from_region_row(&c, &region_row(1, flags, 0, phenomenon, 0, 0, 0), 0)
                        .is_err(),
                    "{i} and {j} overlap"
                );
            }
        }
        // active_modifier_count (`:17181`) and intensity (`:17186`) bounds.
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, 0, MAX_ACTIVE_MODIFIERS + 1, 0), 0).is_err());
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, 0, MAX_ACTIVE_MODIFIERS, 0), 0).is_ok());
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, 0, 0, MAX_INTENSITY + 1), 0).is_err());
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, 0, 0, MAX_INTENSITY), 0).is_ok());
        // A non-zero reserved word (`:17194`).
        let mut row = region_row(1, 0, 0, 0, 0, 0, 0);
        row[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(record_from_region_row(&c, &row, 0).is_err());
    }

    #[test]
    fn the_province_mapper_rejects_every_row_the_fold_rejects() {
        let c = province_collection();
        for len in [0usize, 1, PROVINCE_ROW_LEN - 1, PROVINCE_ROW_LEN + 1] {
            assert!(record_from_province_row(&c, &vec![0u8; len], 0).is_err(), "length {len}");
            assert!(key_from_province_row(&vec![0u8; len]).is_err());
        }
        // Unknown flag bits (`:17239`).
        for bit in 10..32u32 {
            assert!(
                record_from_province_row(&c, &province_row(1, 1 << bit, 0, 0, 0, 0, 0), 0).is_err(),
                "flag bit {bit} is not in PROVINCE_KNOWN_FLAGS"
            );
        }
        // mud_active without mud_present (`:17253`).
        assert!(record_from_province_row(
            &c,
            &province_row(1, PROVINCE_MUD_ACTIVE | PROVINCE_STRUCT_VALIDATED, 0, 0, 0, 0, 0),
            0
        )
        .is_err());
        // Either driver bit disagreeing with its rule (`:17258`, `:17263`),
        // in both directions.
        for wrong in [PROVINCE_MAP_RECOLOR, PROVINCE_DYNAMIC_LUT] {
            let flags = full_province_flags(false) & !wrong;
            assert!(
                record_from_province_row(&c, &province_row(1, flags, 100_000, 0, 0, 0, 0), 0).is_err(),
                "clearing {wrong:#x} on a row that needs it"
            );
            let flags = PROVINCE_STRUCT_VALIDATED | wrong;
            assert!(
                record_from_province_row(&c, &province_row(1, flags, 0, 0, 0, 0, 0), 0).is_err(),
                "setting {wrong:#x} on a row with no presences"
            );
        }
        // Out-of-range present values, each at its own bound (`:26794`).
        let full = full_province_flags(false);
        let cases: [(usize, i32, i32); 4] = [
            (0, MAX_TEMPERATURE_RAW, -MAX_TEMPERATURE_RAW),
            (1, MAX_TEMP_CHANGE_RAW, -MAX_TEMP_CHANGE_RAW),
            (2, MAX_PRECIP_RAW, 0),
            (3, MAX_PRECIP_RAW, 0),
        ];
        for (slot, max, min) in cases {
            for (value, ok) in [(max, true), (min, true), (max + 1, false), (min - 1, false)] {
                let mut vals = [0i32; 4];
                vals[slot] = value;
                let row = province_row(1, full, vals[0], vals[1], vals[2], vals[3], 0);
                assert_eq!(
                    record_from_province_row(&c, &row, 0).is_ok(),
                    ok,
                    "slot {slot} value {value}"
                );
            }
        }
        // A gated-off scalar carrying a non-zero word, each of the four.
        for slot in 0..4usize {
            let gate = [
                PROVINCE_TEMP_PRESENT,
                PROVINCE_TEMP_CHANGE_PRESENT,
                PROVINCE_WATER_PRESENT,
                PROVINCE_SNOW_PRESENT,
            ][slot];
            let mut flags = full & !gate;
            // Keep the driver bits consistent with the remaining presences.
            let temp = flags & PROVINCE_TEMP_PRESENT != 0;
            let water = flags & PROVINCE_WATER_PRESENT != 0;
            let snow = flags & PROVINCE_SNOW_PRESENT != 0;
            let mud = flags & PROVINCE_MUD_PRESENT != 0;
            flags &= !(PROVINCE_MAP_RECOLOR | PROVINCE_DYNAMIC_LUT);
            if temp || water || snow {
                flags |= PROVINCE_MAP_RECOLOR;
            }
            if snow || mud {
                flags |= PROVINCE_DYNAMIC_LUT;
            }
            let mut vals = [0i32; 4];
            vals[slot] = 1;
            let row = province_row(1, flags, vals[0], vals[1], vals[2], vals[3], 0);
            assert!(
                record_from_province_row(&c, &row, 0).is_err(),
                "slot {slot} is non-zero without its flag"
            );
        }
        // custom_modifier_count: bound when present, zero when absent.
        assert!(record_from_province_row(&c, &province_row(1, full, 0, 0, 0, 0, MAX_ACTIVE_MODIFIERS), 0).is_ok());
        assert!(record_from_province_row(&c, &province_row(1, full, 0, 0, 0, 0, MAX_ACTIVE_MODIFIERS + 1), 0).is_err());
        let no_custom = full & !PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT;
        assert!(record_from_province_row(&c, &province_row(1, no_custom, 0, 0, 0, 0, 1), 0).is_err());
        assert!(record_from_province_row(&c, &province_row(1, no_custom, 0, 0, 0, 0, 0), 0).is_ok());
        // A non-zero reserved word (`:17311`).
        let mut row = province_row(1, full, 0, 0, 0, 0, 0);
        row[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(record_from_province_row(&c, &row, 0).is_err());
        // The header-relative id bound (`:17232`).
        assert!(check_province_id(13_414, 13_414).is_err());
        assert!(check_province_id(13_413, 13_414).is_ok());
        assert!(check_province_id(0, 0).is_err(), "no province is legal at zero rows");
    }

    #[test]
    fn a_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = region_collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_region_row(&c, &region_row(1, 0, 0, 0, 0, 0, 0), 0).is_err());
        let mut c = province_collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_province_row(&c, &province_row(1, full_province_flags(false), 0, 0, 0, 0, 0), 0).is_err());
    }

    // --- The payload header ---------------------------------------------------

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let good = baseline_payload(10_000);
        let (header, regions, provinces) = split_payload(&good).expect("well-formed");
        assert_eq!(header.region_count, REGION_ROWS as u32);
        assert_eq!(header.province_count, 10_000);
        assert_eq!(regions.len(), REGION_ROWS * REGION_ROW_LEN);
        assert_eq!(provinces.len(), 10_000 * PROVINCE_ROW_LEN);
        assert!(!header.is_delta());

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&good[..len]).is_err(), "len {len}");
            assert!(parse_header(&good[..len]).is_err(), "len {len}");
        }
        // A length that does not match the counts.
        let mut short = good.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());

        let patch = |offset: usize, value: u32| {
            let mut p = good.clone();
            p[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            p
        };
        // scale_raw (`:17058`), in both directions.
        for wrong in [0u32, 1, 32_768, 99_999, 100_001] {
            assert!(split_payload(&patch(32, wrong)).is_err(), "scale {wrong}");
        }
        // observed_region_rows (`:17059`).
        for wrong in [0u32, 304, 306] {
            assert!(split_payload(&patch(24, wrong)).is_err(), "observed regions {wrong}");
        }
        // observed_province_rows outside its window (`:17064`).
        for wrong in [1u32, 9_999, 65_537, u32::MAX] {
            assert!(split_payload(&patch(28, wrong)).is_err(), "observed provinces {wrong}");
        }
        // A full snapshot short of 305 regions (`:17072`) or of its own
        // observed province count (`:17077`).
        let mut fewer = baseline_payload(10_000);
        fewer[8..12].copy_from_slice(&(REGION_ROWS as u32 - 1).to_le_bytes());
        assert!(split_payload(&fewer).is_err());
        assert!(split_payload(&patch(20, 9_999)).is_err());
        // active_region_count above region_count (`:17107`).
        assert!(split_payload(&patch(36, REGION_ROWS as u32 + 1)).is_err());
        // Non-zero reserved words (`:17112`).
        assert!(split_payload(&patch(40, 1)).is_err());
        assert!(split_payload(&patch(44, 1)).is_err());
        // changed_rows above the emitted rows (`:17097`).
        assert!(split_payload(&patch(12, REGION_ROWS as u32 + 10_001)).is_err());
        // A count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
    }

    #[test]
    fn a_delta_payload_must_emit_exactly_the_rows_it_changed() {
        // `:17102`. Build a two-row delta and then lie about `changed_rows`.
        let (flags, phenomenon) = active(REGION_SNOW);
        let regions = vec![region_row(4, flags, 0, phenomenon, 0, 0, 0)];
        let provinces = vec![province_row(9, full_province_flags(false), 1, 1, 1, 1, 0)];
        let mut p = payload(STATUS_DELTA_DATA, 13_414, &regions, &provinces, 1);
        assert!(split_payload(&p).is_ok());
        p[12..16].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&p).is_err(), "changed_rows 1 != emitted 2");
        // And a delta may carry no rows at all: 222 of the 903 d24 records in
        // the deep-war capture are exactly that.
        let empty = payload(STATUS_DELTA_DATA, 13_414, &[], &[], 0);
        let (header, r, pr) = split_payload(&empty).expect("an empty delta is well-formed");
        assert!(header.is_delta());
        assert!(r.is_empty() && pr.is_empty());
    }

    // --- The accumulator ------------------------------------------------------

    #[test]
    fn a_baseline_replaces_and_a_delta_upserts() {
        let s = schema();
        let mut state = WeatherState::default();
        assert_eq!(
            state
                .fold_payload(&s, &baseline_payload(10_000), 0)
                .expect("folds"),
            FoldOutcome::Folded { reset: true }
        );
        assert_eq!(state.regions.len(), REGION_ROWS);
        assert_eq!(state.provinces.len(), 10_000);
        assert_eq!(state.observed_province_rows, 10_000);
        assert_eq!(state.scale_raw, FIXED_SCALE_RAW);
        assert!(state.baseline_seen);
        let before = state.provinces[&key(&[7])].clone();

        // A delta touching one province leaves every other row standing.
        let touched = province_row(7, full_province_flags(true), 999_999, 1, 1, 1, 2);
        let delta = payload(STATUS_DELTA_DATA, 10_000, &[], &[touched], 0);
        assert_eq!(
            state.fold_payload(&s, &delta, 0).expect("folds"),
            FoldOutcome::Folded { reset: false }
        );
        assert_eq!(state.regions.len(), REGION_ROWS, "a delta deletes nothing");
        assert_eq!(state.provinces.len(), 10_000);
        assert_ne!(state.provinces[&key(&[7])], before, "the touched row moved");

        // A second baseline replaces the whole state, including its size.
        assert_eq!(
            state
                .fold_payload(&s, &baseline_payload(11_000), 0)
                .expect("folds"),
            FoldOutcome::Folded { reset: true }
        );
        assert_eq!(state.provinces.len(), 11_000);
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing() {
        // `world_fold.rs:17046` / `:2904`: there is no identity set to patch.
        let s = schema();
        let mut state = WeatherState::default();
        let delta = payload(
            STATUS_DELTA_DATA,
            13_414,
            &[],
            &[province_row(1, full_province_flags(false), 1, 1, 1, 1, 0)],
            0,
        );
        assert_eq!(
            state.fold_payload(&s, &delta, 0).expect("skips"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(state.provinces.is_empty());
        assert!(!state.baseline_seen);
    }

    #[test]
    fn a_failure_status_record_folds_nothing_and_retains_the_accumulator() {
        // `world_fold.rs:17043` / `:2863`. Every failure bit, one at a time —
        // and the payload is deliberately malformed to prove the fold's order:
        // a discarded record is never structurally validated.
        let s = schema();
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut state = WeatherState::default();
            state
                .fold_payload(&s, &baseline_payload(10_000), 0)
                .expect("folds");
            let before = state.provinces.len();
            let mut broken = payload(1 << bit, 13_414, &[], &[], 7);
            broken[24..28].copy_from_slice(&9u32.to_le_bytes()); // bad observed_region_rows
            assert_eq!(
                state.fold_payload(&s, &broken, 0).expect("skips"),
                FoldOutcome::FailureRetained,
                "bit {bit}"
            );
            assert_eq!(state.provinces.len(), before, "accumulator retained");
        }
        // The delta bit is NOT a failure bit.
        let mut state = WeatherState::default();
        state.fold_payload(&s, &baseline_payload(10_000), 0).expect("folds");
        assert_eq!(
            state
                .fold_payload(&s, &payload(STATUS_DELTA_DATA, 10_000, &[], &[], 0), 0)
                .expect("folds"),
            FoldOutcome::Folded { reset: false }
        );
    }

    #[test]
    fn the_frame_level_rules_reject_what_the_row_mapper_cannot_see() {
        let s = schema();
        let mut state = WeatherState::default();
        state.fold_payload(&s, &baseline_payload(10_000), 0).expect("folds");

        // A duplicated region id in one frame (`:17132`).
        let (flags, phenomenon) = active(0);
        let row = region_row(4, flags, 0, phenomenon, 0, 0, 0);
        let dup = payload(STATUS_DELTA_DATA, 10_000, &[row.clone(), row], &[], 2);
        assert!(state.clone().fold_payload(&s, &dup, 0).is_err());

        // A duplicated province id in one frame (`:17233`).
        let prow = province_row(5, full_province_flags(false), 0, 0, 0, 0, 0);
        let dup = payload(STATUS_DELTA_DATA, 10_000, &[], &[prow.clone(), prow], 0);
        assert!(state.clone().fold_payload(&s, &dup, 0).is_err());

        // A province id at or past `observed_province_rows` (`:17232`).
        let out = province_row(10_000, full_province_flags(false), 0, 0, 0, 0, 0);
        let bad = payload(STATUS_DELTA_DATA, 10_000, &[], &[out], 0);
        assert!(state.clone().fold_payload(&s, &bad, 0).is_err());

        // A header witness that disagrees with the rows (`:17210`), both ways.
        let active_row = region_row(6, flags, 0, phenomenon, 0, 0, 0);
        for claimed in [0u32, 2] {
            let bad = payload(STATUS_DELTA_DATA, 10_000, &[active_row.clone()], &[], claimed);
            assert!(
                state.clone().fold_payload(&s, &bad, 0).is_err(),
                "claimed {claimed} active regions for one active row"
            );
        }
        let ok = payload(STATUS_DELTA_DATA, 10_000, &[active_row], &[], 1);
        assert!(state.clone().fold_payload(&s, &ok, 0).is_ok());
    }

    #[test]
    fn a_rejected_payload_leaves_the_accumulator_untouched() {
        let s = schema();
        let mut state = WeatherState::default();
        state.fold_payload(&s, &baseline_payload(10_000), 0).expect("folds");
        let snapshot = state.provinces.clone();
        // A frame whose SECOND province row is invalid: a mapper that merged as
        // it went would leave the first row applied.
        let good = province_row(1, full_province_flags(false), 1, 1, 1, 1, 0);
        let bad = province_row(2, 1 << 20, 0, 0, 0, 0, 0);
        let frame = payload(STATUS_DELTA_DATA, 10_000, &[], &[good, bad], 0);
        assert!(state.fold_payload(&s, &frame, 0).is_err());
        assert_eq!(state.provinces, snapshot, "nothing was half-applied");
    }

    #[test]
    fn the_active_region_witness_is_recomputed_over_reconstructed_state() {
        // `world_fold.rs::fold_weather`: after a delta the count is over the
        // whole 305-region state, not over the delta's own rows. A field
        // carrying the header's number would be wrong on every delta.
        let s = schema();
        let c = region_collection();
        let mut state = WeatherState::default();
        state.fold_payload(&s, &baseline_payload(10_000), 0).expect("folds");
        let baseline_active = (0..REGION_ROWS).filter(|id| id % 3 == 0).count();
        assert_eq!(state.active_region_count(&c), baseline_active);

        // Deactivate one region with a one-row delta whose own header witness
        // is 0. The reconstructed count must fall by exactly one.
        let calmed = region_row(0, 0, 0, 0, 0, 0, 0);
        let delta = payload(STATUS_DELTA_DATA, 10_000, &[calmed], &[], 0);
        state.fold_payload(&s, &delta, 0).expect("folds");
        assert_eq!(state.active_region_count(&c), baseline_active - 1);
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // The accumulator reads bytes off disk and must return a diagnosis.
        let s = schema();
        let mut rng = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..3_000 {
            let len = (next() as usize) % 400;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                *byte = next() as u8;
            }
            let _ = WeatherState::default().fold_payload(&s, &bytes, 0);
        }
        // And corruption of genuinely well-formed payloads: a whole snapshot
        // (every header word and every region row form) and a delta carrying
        // province rows (every province row form).
        let provinces: Vec<Vec<u8>> = (0..50u32)
            .map(|id| province_row(id, full_province_flags(id % 2 == 0), 1, 1, 1, 1, 1))
            .collect();
        let delta = payload(STATUS_DELTA_DATA, 13_414, &[], &provinces, 0);
        // The stride on the snapshot keeps the sweep cheap while still hitting
        // every header word and every byte offset within a region row.
        for (good, stride) in [(region_only_baseline(), 13usize), (delta, 1)] {
            for i in (0..good.len()).step_by(stride) {
                let mut p = good.clone();
                p[i] ^= 0xFF;
                let mut state = WeatherState::default();
                state.baseline_seen = true;
                let _ = state.fold_payload(&s, &p, 0);
            }
        }
    }

    #[test]
    fn a_whole_frame_survives_a_baseline_and_delta_round_trip() {
        // End to end at the collection level: fold, encode a baseline, decode
        // it, fold a delta, encode the delta against the reader's own prior,
        // and require stored-equality both times. This is the property the
        // measurement harness asserts every hour, at small scale.
        use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};

        let s = schema();
        let mut state = WeatherState::default();
        state
            .fold_payload(&s, &baseline_payload(MIN_PROVINCE_ROWS as u32), 0)
            .expect("folds");

        let mut bytes = Vec::new();
        encode_baseline(&s, &[state.regions.clone(), state.provinces.clone()], &mut bytes)
            .expect("baseline encodes");
        let (mut reader, consumed) = decode_baseline(&s, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(reader[0], state.regions);
        assert_eq!(reader[1], state.provinces);

        let prior_regions = state.regions.clone();
        let prior_provinces = state.provinces.clone();
        let touched = province_row(7, full_province_flags(true), 111_111, 2, 3, 4, 1);
        let (flags, phenomenon) = active(REGION_BLIZZARD);
        let region = region_row(11, flags, -12_345, phenomenon, 60_900_000, 2, 2);
        let delta_payload = payload(STATUS_DELTA_DATA, MIN_PROVINCE_ROWS as u32, &[region], &[touched], 1);
        state.fold_payload(&s, &delta_payload, 0).expect("folds");

        let deltas = vec![
            Some(diff(&s.collections[0], &prior_regions, &state.regions)),
            Some(diff(&s.collections[1], &prior_provinces, &state.provinces)),
        ];
        let mut dbytes = Vec::new();
        encode_delta(&s, &[prior_regions, prior_provinces], &deltas, &mut dbytes)
            .expect("delta encodes");
        let (decoded, consumed) =
            decode_delta(&s, &reader, &dbytes, 0).expect("delta decodes");
        assert_eq!(consumed, dbytes.len());
        for (i, d) in decoded.iter().enumerate() {
            apply(&mut reader[i], d.as_ref().expect("both collections changed"));
        }
        assert_eq!(reader[0], state.regions, "regions replay stored-equal");
        assert_eq!(reader[1], state.provinces, "provinces replay stored-equal");
    }
}
