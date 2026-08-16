//! d52 `occupation_resistance_compliance_live` — per-state occupation, resistance
//! and compliance, plus the `GAR1` garrison-alert and `COL1` collaboration
//! trailers.
//!
//! Derived from `world_fold.rs:22699`-`:22705`
//! (`fold_occupation_resistance_compliance`, dispatched `:4714`), its payload
// @world-fold-authority domain=52 functions=fold_occupation_resistance_compliance lines=22699-22705 sha256=3264eb66c9def2ec9621972f8ec726bae49e6c181c3bdea79c7f28437185d962 closure=6676db9ead4b5e7ba55bb5dbc1e9560b5c8b195cc48a709571b37a5f5c51b1ca
// @lane-payload-authority domain=52 path=extractor/native/lanes/occupation_resistance_compliance_live/domain44_compact.rs function=decode_payload lines=265-662 sha256=974013e723937ae796d4f566b1b702b266e152ddf2e990ba2f3409d31994eff5
//! constants at `:388`-`:399` and `:435`-`:466`, its two trailer decoders
//! (`occupation_garrison_alert_len` `:30271`, `decode_occupation_collaboration`
//! `:30291`, `decode_occupation_garrison_alert` `:30355`), the lane's own payload
//! contract
//! (`extractor/native/lanes/occupation_resistance_compliance_live/domain44_compact.rs`,
//! included verbatim as `domain52_compact.rs`) and the collector read that
//! produces the presence flags (`collector_module.rs:1079`-`:1117`), plus
//! `extractor/documents/domains/d52_occupation_resistance_compliance_live.md`.
//! This note used to say there was no domain dossier, which was true when the
//! module was written; lane IM added one's census section and the prose contract
//! for the coverage verdict lives there now.
//!
//! # Payload grammar
//!
//! A 56-byte header, `row_count` fixed 64-byte state rows, then **two optional
//! trailers** in a fixed order: `GAR1` (40-byte header + 16-byte equipment rows)
//! and `COL1` (32-byte header + 16-byte relation rows).
//!
//! **The fold reads four header words** — `row_count` at `+8`, `status_flags`
//! at `+16` and, since lane IM (GP-U6), `observed_state_rows` at `+20` and
//! `invalid_state_rows` at `+52`
//! (`world_fold.rs::fold_occupation_resistance_compliance`). The header grew from
//! 52 to 56 bytes in that lane, because the extractor had always counted the
//! state roots it could not read and never encoded the count: on a FULL frame the
//! loss was still derivable as `observed - row_count`, but on a DELTA it was not
//! derivable at all, and a delta that silently dropped a root left the fold
//! retaining the previous hour's value for that state under this hour's stamp.
//! The census pair is not a [`split_payload`] invariant: the fold PUBLISHES the
//! pair, as the census witness `_r2_occupation_census_live`
//! (`world_fold.rs::record_occupation_census`). The pair is a COVERAGE VERDICT
//! rather than a measurement of the engine's cardinality -- zero means the hour
//! is trustworthy, nonzero means it is not, and the magnitude is accounting
//! rather than a count of lost states -- and the thing that refuses on it
//! is the replay admission for `mapmode.occupation_resistance_compliance`
//! (`replay_interface/native/map_viewer/src/map_authority.rs::occupation_resistance_compliance_status`),
//! which reconciles the recorder's published count and membership digest against
//! the frame's own `states` object. Refusing in `split_payload` instead would
//! throw away the HOUR over an incomplete census, which is the opposite of what
//! the census is for.
//!
//! [`check_authority_payload`] is the other half of that split and DOES constrain
//! the pair, mirroring the lane decoder, because these are arithmetic no frame
//! the producer built can break:
//!
//! * on EVERY frame, `row_count + invalid_state_rows <= observed_state_rows` —
//!   a delta's rows are a subset of the same valid extraction the verdict
//!   describes, so rows and losses together never exceed the roots walked;
//! * on a NON-DELTA frame, that sum is an equality, because a full extraction
//!   accounts for every root exactly once.
//!
//! The first subsumes the separate `row_count <= observed_state_rows` and
//! `invalid_state_rows <= observed_state_rows` bounds that used to stand beside
//! it; both were deleted rather than kept as guards no test could isolate. Every other word is
//! parsed and reported by [`PayloadHeader`] and **never enforced here**, per the
//! recipe's rule that the test is whether YOUR fold reads it. In particular
//! `changed_rows` at `+12` is *not* an invariant of this fold: it is the lane
//! contract's (`domain44_compact.rs:276`) and therefore lives in
//! [`check_authority_payload`], not in [`split_payload`].
//!
//! | offset | width | header word | read by the fold |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms`         | no |
//! | `+8`  | u32 | `row_count`               | **yes** (`:16802`) |
//! | `+12` | u32 | `changed_rows`            | no |
//! | `+16` | u32 | `status_flags`            | **yes** (`:16803`) |
//! | `+20` | u32 | `observed_state_rows`     | **yes** (`world_fold.rs::fold_occupation_resistance_compliance`) |
//! | `+24` | u32 | `occupied_state_rows`     | no |
//! | `+28` | u32 | `wall_badge_rows`         | no |
//! | `+32` | u32 | `scale_raw` (== 100000)   | no |
//! | `+36` | u32 | `law_token_rows`          | no |
//! | `+40` | u32 | `garrison_template_rows`  | no |
//! | `+44` | u32 | `garrison_scalar_rows`    | no |
//! | `+48` | u32 | `garrison_anomaly_rows`   | no |
//! | `+52` | u32 | `invalid_state_rows` (coverage verdict; zero = trustworthy) | **yes** (`world_fold.rs::fold_occupation_resistance_compliance`) |
//!
//! State row, 64 bytes (`world_fold.rs::fold_occupation_resistance_compliance`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `state_id`                   | raw    | key; zero rejected (`world_fold.rs::fold_occupation_resistance_compliance`) |
//! | `+4`  | u32 | `owner_tag_db_id`            | raw    | `OWNER` (`1<<0`) |
//! | `+8`  | u32 | `controller_tag_db_id`       | raw    | `CONTROLLER` (`1<<1`) |
//! | `+12` | u32 | `occupied_country_tag_db_id` | raw    | `OCCUPIED_TAG_DERIVED` (`1<<5`) |
//! | `+16` | i32 | `resistance_raw`             | /1e5   | `RESISTANCE` (`1<<2`) |
//! | `+20` | i32 | `compliance_raw`             | /1e5   | `COMPLIANCE` (`1<<3`) |
//! | `+24` | i32 | `resistance_speed_raw`       | /1e5   | `RESISTANCE_SPEED` (`1<<11`) |
//! | `+28` | i32 | `compliance_speed_raw`       | /1e5   | `COMPLIANCE_SPEED` (`1<<12`) |
//! | `+32` | i32 | `garrison_manpower_need`     | **raw**| `GARRISON_SCALARS` (`1<<15`) |
//! | `+36` | i32 | `garrison_strength_ratio_raw`| /1e5   | `GARRISON_SCALARS` |
//! | `+40` | i32 | `garrison_required_raw`      | /1e5   | `GARRISON_SCALARS` |
//! | `+44` | u32 | `occupation_law_token_id`    | raw    | `OCCUPATION_LAW_TOKEN` (`1<<6`) |
//! | `+48` | u32 | `garrison_template_id`       | raw    | `GARRISON_TEMPLATE` (`1<<7`) |
//! | `+52` | u32 | `flags`                      | —      | — |
//! | `+56` | u32 | `reserved1` (must be 0)      | —      | — |
//! | `+60` | u32 | `reserved2` (must be 0)      | —      | — |
//!
//! **Mixed scales inside one row.** `garrison_manpower_need` is a plain manpower
//! count with **no divisor** (`world_fold.rs::fold_occupation_resistance_compliance`, and the lane says so
//! in the type itself: "Plain i32 manpower count (CStateGarrisonData+0x88), NOT
//! fixed-scaled", `domain44_compact.rs:82`), while the two scalars beside it at
//! `+36`/`+40` are `/FIXED_SCALE` (`:16964`-`:16973`). Reading the count through
//! the divisor would be wrong by 100,000x and every round-trip test would still
//! pass. [`the_scales_are_per_field_not_per_row`] is the test that catches it.
//!
//! # Grammar class: baseline + delta
//!
//! `COMPACT_STATUS_DELTA_DATA` against `delta_baselines_seen` (`:16836`-`:16852`).
//! A **non-delta** frame removes `occupation` and `occupation_law` from **every**
//! state before folding its rows (`:16853`-`:16858`) — a full replace of this
//! domain's slice — and a delta frame leaves untouched states alone, so an
//! accumulator is required ([`State`]).
//!
//! A delta arriving before any baseline folds nothing into the base table and
//! retains the accumulator (`:16836`-`:16845`); so does a failure status
//! (`:16825`-`:16835`). Every `Err` from this module is a stream rejection: the
//! recorder's fold returns `Err` for them and `WorldFold::ingest_record` aborts
//! the whole fold.
//!
//! **The two non-folding paths are not equivalent, and the difference is in the
//! trailers.** The failure path *overwrites* the decoded alert with `Null` and
//! clears the collaboration rows (`:16827`-`:16829`), so nothing of that hour
//! survives and skipping it loses nothing. The delta-before-baseline path returns
//! **without touching them** (`:16838`-`:16844`), so the trailers decoded at
//! `:16813`-`:16820` stay live and are published at `:5218`-`:5268` and
//! `:6014`-`:6019` — real folded data on an hour whose base table is unknown.
//! A keyed set cannot say "this collection's membership was not measured this
//! hour" (`WIRE_FORMAT.md` §13, the gap that stopped d17), so that hour cannot be
//! encoded faithfully and cannot be silently skipped either. [`FoldOutcome`]
//! reports it and the measurement harness **refuses the run**; see
//! `bin/bench_real_d52.rs`.
//!
//! **The fold's call order puts the trailer decode BEFORE the status check**
//! (`:16813`-`:16823` decode, `:16825` status). A malformed `GAR1`/`COL1` trailer
//! therefore refuses the stream even on a failure-status frame, and
//! [`State::fold_payload`] reproduces that order exactly.
//!
//! # Row deletion is real
//!
//! The fold builds an `occupation` object and then
//! `if occupation.is_empty() { state.remove("occupation") }` (`:17018`-`:17022`).
//! A row whose flags contribute nothing therefore **deletes** the state from this
//! domain's slice. [`row_folds_to_deletion`] is that predicate, and
//! [`State::fold_payload`] removes the key — which is a keyed-set tombstone.
//! The collector cannot emit such a row (`collector_module.rs:1079`-`:1082` sets
//! `RESISTANCE | COMPLIANCE | RESISTANCE_SPEED | COMPLIANCE_SPEED`
//! unconditionally on every row it builds), but the fold accepts it and so does
//! this port — the recipe's rule is to mirror the fold's permissiveness.
//!
//! # `occupation_law` is a constant, not a field
//!
//! The fold writes `state["occupation_law"] = Value::Null` for **every** row it
//! folds (`:17017`) — a constant with no measured value, so it is not re-encoded
//! (the recipe's derived-constant rule, d62's witness-flags word). One consequence
//! is recorded rather than papered over: the marker is written even by a deletion
//! row, so the fold's marker set is a superset of its `occupation` set. The two
//! coincide on every payload the collector can produce (a deletion row is
//! unreachable, above) and on every non-delta frame (both keys are removed
//! together, `:16848`-`:16849`). [`the_occupation_law_marker_is_a_constant`] pins
//! the reasoning.
//!
//! # Four-state semantics
//!
//! `WIRE_FORMAT.md` §3's four states are decided per field from the **collector**,
//! which is the only authority on *why* a gate is clear, and corroborated against
//! the 7,498 real rows of `runs/batch3/deepwar_a`.
//!
//! * **`resistance`, `compliance` — Unknown on a clear gate, no Absent.** The
//!   collector sets both flags **unconditionally** on every row it emits
//!   (`collector_module.rs:1079`-`:1080`) and drops the whole row when the read
//!   fails or a bound trips (`:1047`, `:1056`-`:1076`), so a clear gate is a
//!   payload it cannot produce. The concept applies to every state (so not
//!   Absent) and is not a relation that can point at nothing (so not Null); §3's
//!   remaining reading is "applicable, not captured". This is the recipe's
//!   tie-break, and it is in the open questions.
//! * **`resistance_speed`, `compliance_speed` — Unknown on a clear gate, for the
//!   same reason and from the same read.** `collector_module.rs:1052`-`:1055`
//!   reads all four channel words together and `:1079`-`:1082` sets all four
//!   flags together, so a clear speed gate is no more an inapplicable concept
//!   than a clear resistance gate is. `RESISTANCE_SPEED_UNAVAILABLE` (`1<<8`) is
//!   a badge the fold stores as `resistance_speed_status: "unavailable"`
//!   (`:16949`-`:16950`) and which the lane makes exclusive with **both** speed
//!   value flags, not just the one it is named after
//!   (`domain44_compact.rs:534`-`:540`); it says *why* the value is missing, so
//!   it rides beside them as its own `Boolean` rather than collapsing one of the
//!   two empty states into an Absent the collector cannot justify. The badge is
//!   never set in the measured capture — a coverage caveat, see the deliverable.
//! * **The garrison block is a three-state gate.** `GARRISON_SCALARS` (`1<<15`)
//!   and `GARRISON_SIGNALS_UNAVAILABLE` (`1<<9`) are mutually exclusive and the
//!   fold errors when both are set (`:16951`-`:16957`); values under neither badge
//!   are a hard refusal (`:16974`-`:16983`). The lane states the third state in as
//!   many words: "rows carrying neither flag have no garrison measurement at all
//!   (absent)" (`domain44_compact.rs:46`). So value / **Unknown** / **Absent** for
//!   all three scalars, driven by two bits.
//! * **`occupier_tag`, `occupied_country_tag` — Absent when the state is not
//!   occupied, Null when the relation is empty.** Both live under
//!   `OCCUPIED_STATE` (`1<<4`, `:16932`): a state that is not occupied has no
//!   occupier and no occupied country, which is §3's "not applicable" — d20's
//!   `move_target_province` reasoning. Inside that gate the fold takes the
//!   controller for `occupier_tag` (`:16933`-`:16938`) and the derived tag or,
//!   failing that, the owner for `occupied_country_tag` (`:16938`-`:16947`); the
//!   collector clears `OWNER`/`CONTROLLER` exactly when the tag word is `0`
//!   (`collector_module.rs:1083`-`:1088`), i.e. the relation genuinely points at
//!   nothing — §3's **null**.
//! * **`occupation_law_token_id` — Unknown on a clear flag, never Null, and the
//!   badge that looks like the gate is not one.** `OCCUPATION_LAW_UNAVAILABLE`
//!   (`1<<10`) is the obvious neighbour and it is the **wrong** one: the collector
//!   sets it for every occupied state unconditionally and never clears it when the
//!   token *is* found (`collector_module.rs:1097`-`:1099`), and the capture bears
//!   that out — the badge is set on 6,800 rows while the token is present on all
//!   7,498. That is d28's `leader_trait_count` lesson: look for the sibling, then
//!   check it means what its name says. `OCCUPATION_STATUS_ROOT` (`1<<13`) is the
//!   nearer sibling — it is set iff the controller's occupation-status record was
//!   found (`:1104`-`:1110`) — but it does **not** rescue a Null either, because
//!   inside a valid status root the token still reaches zero along three separate
//!   read-failure paths the collector collapses: an unreadable law pointer
//!   (`unwrap_or(0)`, `:536`-`:541`), a pointer that fails its heap/vtable test,
//!   and a name string that will not read (`:546`-`:556`). "The country has no
//!   default occupation law" and "we could not read it" are therefore
//!   indistinguishable — d16's and d24's value-derived-gate ambiguity — and the
//!   recipe's tie-break makes it **Unknown**. Recorded upstream as a collector-side
//!   loss.
//! * **`garrison_template_id` — Absent, Unknown or a value.** Absent when the
//!   state is not occupied, because the collector only attempts the read under
//!   `OCCUPIED_STATE` (`collector_module.rs:1111`-`:1115`) — the concept does not
//!   apply to a row it never looks at. Unknown inside that gate whenever the flag
//!   is clear: `template_id` comes through the same `unwrap_or(0)` collapse and
//!   the same vtable/type rejection as the law token (`:559`-`:585`), so a zero
//!   cannot be read as "no template defined".
//! * **The four badge bits that gate no value are Booleans, always present.**
//!   `OCCUPATION_STATUS_ROOT`, `OCCUPATION_LAW_UNAVAILABLE`,
//!   `COUNTRY_DEFAULT_LAW_TOKEN` and `RESISTANCE_SPEED_UNAVAILABLE` are each
//!   stored by the fold as their own key (`:16948`-`:17010`) and each carries
//!   information no other field does, so each is a `Boolean` with **no** declared
//!   capability — one bit-packed bit rather than a presence bit plus a byte.
//! * **The two trailers' unavailable arm is Unknown, not a zero.** `GAR1`'s
//!   `controller_tag_db_id`, `predicate_returned` and `missing_manpower`, and
//!   `COL1`'s `observed_country_rows`, are hard-zeroed by their header invariants
//!   exactly when the source is unavailable (`world_fold.rs:30395`-`:30399`,
//!   `:30322`-`:30326`). On that arm the words are grammar rather than
//!   measurement — nothing was read — so emitting `0`/`false` would publish "no
//!   controller, the predicate did not fire, nothing is missing, no countries"
//!   as measured facts. `COL1`'s `anomaly_rows` is the exception that proves the
//!   rule: it is constrained on the *complete* arm and unconstrained on the
//!   unavailable one, so it is measured on both and declares no empty state.
//!
//! # No derived field (§6)
//!
//! Nothing in the state rows changes deterministically with the clock. The `GAR1`
//! trailer's `game_hour_raw` **is** clock-linear on this capture — it equals the
//! record's own game hour on 514 of 514 hours — but no domain in this crate
//! declares a §6 derived field yet and the fold enforces no such identity, so it
//! ships as a plain `Signed` field and the derived-field candidacy is an open
//! question. The mappers still take `frame_tick` so every domain's boundary has
//! one shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 52;

/// Per-state occupation, resistance and compliance — the base table.
pub const STATES: u32 = 1;
/// The `GAR1` garrison-alert header, as a singleton row (see [`SINGLETON_KEY_COMPONENT`]).
pub const GARRISON_ALERT: u32 = 2;
/// The `GAR1` missing-equipment rows, keyed by equipment archetype.
pub const GARRISON_ALERT_EQUIPMENT: u32 = 3;
/// The `COL1` header, as a singleton row (see [`SINGLETON_KEY_COMPONENT`]).
pub const COLLABORATION_STATUS: u32 = 4;
/// The `COL1` relation rows, keyed by `(owner, target)`.
pub const COLLABORATION_RELATIONS: u32 = 5;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
/// They are unique across the whole domain, not merely within a collection, so a
/// misrouted record cannot silently land on a same-numbered field elsewhere.
pub mod field {
    // --- STATES ---
    pub const STATE_ID: u32 = 1;
    pub const RESISTANCE: u32 = 2;
    pub const COMPLIANCE: u32 = 3;
    pub const RESISTANCE_SPEED: u32 = 4;
    pub const COMPLIANCE_SPEED: u32 = 5;
    pub const OCCUPIER_TAG: u32 = 6;
    pub const OCCUPIED_COUNTRY_TAG: u32 = 7;
    pub const GARRISON_ARMY_MANPOWER_NEED: u32 = 8;
    pub const GARRISON_STRENGTH_RATIO: u32 = 9;
    pub const GARRISON_REQUIRED: u32 = 10;
    pub const OCCUPATION_LAW_TOKEN_ID: u32 = 11;
    pub const OCCUPATION_LAW_TOKEN_IS_COUNTRY_DEFAULT: u32 = 12;
    pub const OCCUPATION_LAW_UNAVAILABLE: u32 = 13;
    pub const GARRISON_TEMPLATE_ID: u32 = 14;
    pub const OCCUPATION_STATUS_ROOT: u32 = 15;
    pub const RESISTANCE_SPEED_UNAVAILABLE: u32 = 16;

    // --- GARRISON_ALERT ---
    pub const ALERT_SINGLETON: u32 = 20;
    pub const ALERT_SOURCE_UNAVAILABLE: u32 = 21;
    pub const ALERT_GAME_HOUR_RAW: u32 = 22;
    pub const ALERT_CONTROLLER_TAG_DB_ID: u32 = 23;
    pub const ALERT_PREDICATE_RETURNED: u32 = 24;
    pub const ALERT_MISSING_MANPOWER: u32 = 25;

    // --- GARRISON_ALERT_EQUIPMENT ---
    pub const EQUIPMENT_ARCHETYPE_ID: u32 = 30;
    pub const EQUIPMENT_WHOLE_UNITS: u32 = 31;

    // --- COLLABORATION_STATUS ---
    pub const COLLABORATION_SINGLETON: u32 = 40;
    pub const COLLABORATION_SOURCE_UNAVAILABLE: u32 = 41;
    pub const COLLABORATION_OBSERVED_COUNTRY_ROWS: u32 = 42;
    pub const COLLABORATION_ANOMALY_ROWS: u32 = 43;

    // --- COLLABORATION_RELATIONS ---
    pub const RELATION_OWNER_TAG_DB_ID: u32 = 50;
    pub const RELATION_TARGET_TAG_DB_ID: u32 = 51;
    pub const RELATION_PROGRESS: u32 = 52;
}

// --- Payload grammar constants, from `world_fold.rs:388`-`:399`, `:435`-`:466`.

/// `OCCUPATION_HEADER_LEN`, `world_fold.rs::OCCUPATION_HEADER_LEN`.
pub const HEADER_LEN: usize = 56;
/// `OCCUPATION_ROW_LEN`, `world_fold.rs::OCCUPATION_ROW_LEN`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 64;
/// `OCCUPATION_GARRISON_ALERT_HEADER_LEN`, `world_fold.rs::OCCUPATION_GARRISON_ALERT_HEADER_LEN`.
pub const GARRISON_ALERT_HEADER_LEN: usize = 40;
/// `OCCUPATION_GARRISON_ALERT_ROW_LEN`, `world_fold.rs::OCCUPATION_GARRISON_ALERT_ROW_LEN`.
pub const GARRISON_ALERT_ROW_LEN: usize = 16;
/// `OCCUPATION_COLLABORATION_HEADER_LEN`, `world_fold.rs::OCCUPATION_COLLABORATION_HEADER_LEN`.
pub const COLLABORATION_HEADER_LEN: usize = 32;
/// `OCCUPATION_COLLABORATION_ROW_LEN`, `world_fold.rs::OCCUPATION_COLLABORATION_ROW_LEN`.
pub const COLLABORATION_ROW_LEN: usize = 16;

/// `FIXED_SCALE`, the divisor the fold applies to five of this row's seven
/// signed words (`world_fold.rs:16903`-`:16929`, `:16964`-`:16973`). **Not** the
/// divisor for `garrison_manpower_need`, which has none.
pub const FIXED_SCALE: f64 = 100_000.0;
/// The header's `scale_raw` word (`+32`) and both trailers' `divisor` words carry
/// this value. The fold asserts it in the trailers (`:30319`, `:30389`) and never
/// reads the header's, so it is an invariant here, never a field.
pub const FIXED_SCALE_RAW: u32 = 100_000;

/// The `Quant::Fixed(5)` decimal grid is the source's own: an `i32` stored over a
/// `100_000` denominator (the recipe's d24 rule). It is exact, and it recovers the
/// raw integer downstream consumers need, which `Exact` would spend eight bytes a
/// value to do. Every `i32` reproduces exactly — the whole `i32` range is more
/// than three decimal orders inside `2^51`, and
/// [`fixed_five_reproduces_every_i32_it_is_given`] samples it densely.
pub const FIXED_QUANT: Quant = Quant::Fixed(5);

pub const ROW_FLAG_OWNER: u32 = 1;
pub const ROW_FLAG_CONTROLLER: u32 = 1 << 1;
pub const ROW_FLAG_RESISTANCE: u32 = 1 << 2;
pub const ROW_FLAG_COMPLIANCE: u32 = 1 << 3;
pub const ROW_FLAG_OCCUPIED_STATE: u32 = 1 << 4;
pub const ROW_FLAG_OCCUPIED_TAG_DERIVED: u32 = 1 << 5;
pub const ROW_FLAG_OCCUPATION_LAW_TOKEN: u32 = 1 << 6;
pub const ROW_FLAG_GARRISON_TEMPLATE: u32 = 1 << 7;
pub const ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE: u32 = 1 << 8;
pub const ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE: u32 = 1 << 9;
pub const ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE: u32 = 1 << 10;
pub const ROW_FLAG_RESISTANCE_SPEED: u32 = 1 << 11;
pub const ROW_FLAG_COMPLIANCE_SPEED: u32 = 1 << 12;
pub const ROW_FLAG_OCCUPATION_STATUS_ROOT: u32 = 1 << 13;
pub const ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN: u32 = 1 << 14;
pub const ROW_FLAG_GARRISON_SCALARS: u32 = 1 << 15;

/// `OCC_ROW_KNOWN_FLAGS`, `world_fold.rs::OCC_ROW_KNOWN_FLAGS`. Any other bit is a hard
/// error (`:16876`-`:16880`) — an unrecognised flag word means the row is not
/// this grammar.
pub const KNOWN_FLAGS: u32 = ROW_FLAG_OWNER
    | ROW_FLAG_CONTROLLER
    | ROW_FLAG_RESISTANCE
    | ROW_FLAG_COMPLIANCE
    | ROW_FLAG_OCCUPIED_STATE
    | ROW_FLAG_OCCUPIED_TAG_DERIVED
    | ROW_FLAG_OCCUPATION_LAW_TOKEN
    | ROW_FLAG_GARRISON_TEMPLATE
    | ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
    | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
    | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
    | ROW_FLAG_RESISTANCE_SPEED
    | ROW_FLAG_COMPLIANCE_SPEED
    | ROW_FLAG_OCCUPATION_STATUS_ROOT
    | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
    | ROW_FLAG_GARRISON_SCALARS;

/// Every flag that on its own puts a key into the fold's `occupation` object —
/// the four value channels (`world_fold.rs::fold_occupation_resistance_compliance`) and the eight badge
/// or extension keys (`world_fold.rs::fold_occupation_resistance_compliance`). `OCCUPIED_STATE` is deliberately NOT
/// here: on its own it contributes nothing, and only pairs with `OWNER`,
/// `CONTROLLER` or `OCCUPIED_TAG_DERIVED` (`world_fold.rs::fold_occupation_resistance_compliance`).
pub const OCCUPATION_CONTENT_FLAGS: u32 = ROW_FLAG_RESISTANCE
    | ROW_FLAG_COMPLIANCE
    | ROW_FLAG_RESISTANCE_SPEED
    | ROW_FLAG_COMPLIANCE_SPEED
    | ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
    | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
    | ROW_FLAG_GARRISON_SCALARS
    | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
    | ROW_FLAG_OCCUPATION_LAW_TOKEN
    | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
    | ROW_FLAG_GARRISON_TEMPLATE
    | ROW_FLAG_OCCUPATION_STATUS_ROOT;

pub const GARRISON_ALERT_MAGIC: &[u8; 4] = b"GAR1";
pub const GARRISON_ALERT_SCHEMA_VERSION: u16 = 1;
pub const GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE: u32 = 1;
pub const GARRISON_ALERT_FLAG_PREDICATE_RETURNED: u32 = 1 << 1;
pub const GARRISON_ALERT_FLAG_MISSING_MANPOWER: u32 = 1 << 2;
pub const GARRISON_ALERT_FLAG_COMPLETE: u32 = 1 << 3;
pub const GARRISON_ALERT_KNOWN_FLAGS: u32 = GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE
    | GARRISON_ALERT_FLAG_PREDICATE_RETURNED
    | GARRISON_ALERT_FLAG_MISSING_MANPOWER
    | GARRISON_ALERT_FLAG_COMPLETE;

pub const COLLABORATION_MAGIC: &[u8; 4] = b"COL1";
pub const COLLABORATION_SCHEMA_VERSION: u16 = 1;
pub const COLLABORATION_FLAG_SOURCE_UNAVAILABLE: u32 = 1;
pub const COLLABORATION_FLAG_COMPLETE: u32 = 1 << 1;
pub const COLLABORATION_KNOWN_FLAGS: u32 =
    COLLABORATION_FLAG_SOURCE_UNAVAILABLE | COLLABORATION_FLAG_COMPLETE;

/// `COMPACT_STATUS_DELTA_DATA` (`compact_contract/src/status_flags_base.rs:16`).
/// Named here rather than depended on: the library half of this crate is the
/// reference implementation and must not pull in the shipping contract crate.
/// [`the_delta_bit_matches_the_shipping_contract`] pins the value.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

/// The constant singleton key both header collections use. Zero, so a reader that
/// ever sees a second key in one of them has seen a malformed frame rather than a
/// second alert (d68's `SINGLETON_KEY_COMPONENT` convention).
pub const SINGLETON_KEY_COMPONENT: u64 = 0;

pub fn alert_singleton_key() -> Key {
    key(&[SINGLETON_KEY_COMPONENT])
}

pub fn collaboration_singleton_key() -> Key {
    key(&[SINGLETON_KEY_COMPONENT])
}

pub fn schema() -> Schema {
    use field as f;

    let states = Collection {
        collection_id: STATES,
        // A `state_id` is a stable identity that persists across hours, which is
        // what a keyed set needs, and only a keyed set has a delta grammar
        // (§7.3) — which this baseline-plus-delta domain requires.
        shape: Shape::KeyedSet,
        fields: vec![
            // `ContentId`, not `Unsigned`: §5 names state ids as content-defined
            // in as many words ("Country tags, equipment names, province and
            // state ids are content-defined"), so a manifest must resolve this
            // for it to keep its meaning under another mod set. It is a ContentId
            // **key**, the heavier §13 retrofit class — flagged separately in the
            // deliverable, because a manifest remap reorders keys and changes the
            // byte layout of every baseline and key list.
            Field::new(f::STATE_ID, Kind::ContentId).key(0),
            // Unknown-only: the collector sets these two gates unconditionally
            // (`collector_module.rs:1079`-`:1080`), so a clear gate is uncaptured
            // data, never an inapplicable concept and never an empty relation.
            Field::new(f::RESISTANCE, Kind::Continuous)
                .unknown_capable()
                .quant(FIXED_QUANT),
            Field::new(f::COMPLIANCE, Kind::Continuous)
                .unknown_capable()
                .quant(FIXED_QUANT),
            // Unknown-only, for the same reason as the two channels above and
            // from the same collector read: `collector_module.rs:1052`-`:1055`
            // reads all four words together and `:1079`-`:1082` sets all four
            // flags together, so a clear speed gate is uncaptured data rather
            // than an inapplicable concept. The `RESISTANCE_SPEED_UNAVAILABLE`
            // badge says WHY it was not captured and rides beside them as its
            // own Boolean, so nothing the fold stores is lost.
            Field::new(f::RESISTANCE_SPEED, Kind::Continuous)
                .unknown_capable()
                .quant(FIXED_QUANT),
            Field::new(f::COMPLIANCE_SPEED, Kind::Continuous)
                .unknown_capable()
                .quant(FIXED_QUANT),
            // Absent when the state is not occupied, Null when the relation
            // points at nothing. Country tags are content-defined (§5).
            Field::new(f::OCCUPIER_TAG, Kind::ContentId)
                .optional()
                .nullable(),
            Field::new(f::OCCUPIED_COUNTRY_TAG, Kind::ContentId)
                .optional()
                .nullable(),
            // `Signed`, not `Unsigned`: the FOLD applies no sign check
            // (`world_fold.rs::fold_occupation_resistance_compliance`), and mapping a negative word
            // through an unsigned kind would publish a different number. The lane
            // contract rejects a negative one (`domain44_compact.rs:478`), which
            // is a stream refusal in `check_authority_payload`, not a mapper gate.
            Field::new(f::GARRISON_ARMY_MANPOWER_NEED, Kind::Signed)
                .optional()
                .unknown_capable(),
            Field::new(f::GARRISON_STRENGTH_RATIO, Kind::Continuous)
                .optional()
                .unknown_capable()
                .quant(FIXED_QUANT),
            Field::new(f::GARRISON_REQUIRED, Kind::Continuous)
                .optional()
                .unknown_capable()
                .quant(FIXED_QUANT),
            // `ContentId`. The fold labels this an FNV-1a-32 hash of the law's
            // NAME (`world_fold.rs::fold_occupation_resistance_compliance`), and a law name is
            // content-defined — §5 lists law tokens among the content-defined
            // vocabularies, and the recipe's deciding question ("must a content
            // manifest resolve this id for it to keep its meaning across mod
            // sets?") answers yes: which law the hash denotes is defined by the
            // mod set, and under another one it may denote nothing. That no
            // manifest can invert a hash TODAY is a gap in the manifest, not a
            // statement about the kind, and it is in the open questions.
            //
            // Unknown-only, never Null. A zero token under a valid status root
            // does NOT mean "this country has no default occupation law": the
            // collector collapses a failed pointer read (`unwrap_or(0)`,
            // `collector_module.rs:536`-`:541`) and a failed name-string read
            // (`:546`-`:556`) into the same zero, so the two are
            // indistinguishable — d16's and d24's value-derived-gate ambiguity,
            // and the recipe's tie-break says Unknown.
            Field::new(f::OCCUPATION_LAW_TOKEN_ID, Kind::ContentId).unknown_capable(),
            // Badges that gate no value: one bit-packed bit each, no capability.
            Field::new(f::OCCUPATION_LAW_TOKEN_IS_COUNTRY_DEFAULT, Kind::Boolean),
            Field::new(f::OCCUPATION_LAW_UNAVAILABLE, Kind::Boolean),
            // Absent off the occupied gate (the collector only attempts the
            // read there, `collector_module.rs:1111`-`:1115`), Unknown inside it
            // whenever the flag is clear — `template_id` comes through the same
            // `unwrap_or(0)` collapse as the law token (`:559`-`:585`), so a zero
            // cannot be read as "no template defined".
            Field::new(f::GARRISON_TEMPLATE_ID, Kind::Unsigned)
                .optional()
                .unknown_capable(),
            Field::new(f::OCCUPATION_STATUS_ROOT, Kind::Boolean),
            // The speed badge, carried so the fold's
            // `resistance_speed_status: "unavailable"` key (`:16949`-`:16950`)
            // survives now that the speeds themselves are Unknown-only.
            Field::new(f::RESISTANCE_SPEED_UNAVAILABLE, Kind::Boolean),
        ],
    };

    let garrison_alert = Collection {
        collection_id: GARRISON_ALERT,
        shape: Shape::KeyedSet,
        fields: vec![
            // The constant singleton key. `Unsigned`: nothing content-defined
            // names it, and no manifest resolves it.
            Field::new(f::ALERT_SINGLETON, Kind::Unsigned).key(0),
            // The trailer's own two-state discriminator
            // (`world_fold.rs::decode_occupation_garrison_alert`: `unavailable == complete` is a
            // rejected header, so exactly one of them holds).
            Field::new(f::ALERT_SOURCE_UNAVAILABLE, Kind::Boolean),
            // Carried by the header in BOTH arms and constrained by neither, so
            // always present — a deliberate fidelity gain over the fold, which
            // drops the whole object on the unavailable path (`:30407`).
            Field::new(f::ALERT_GAME_HOUR_RAW, Kind::Signed),
            // These three are hard-zeroed by the header invariants exactly when
            // the source is unavailable (`world_fold.rs::decode_occupation_garrison_alert`), so on
            // that arm their words are grammar rather than measurement: the
            // alert was not read at all. Emitting `0`/`false` there would
            // publish "no controller, the predicate did not fire, nothing is
            // missing" as measured facts. §3's reading is **unknown** —
            // "applicable, not captured".
            Field::new(f::ALERT_CONTROLLER_TAG_DB_ID, Kind::ContentId).unknown_capable(),
            Field::new(f::ALERT_PREDICATE_RETURNED, Kind::Boolean).unknown_capable(),
            // `missing_manpower < 0` is a rejected header (`:30393`), so the
            // value is always non-negative and `Unsigned` is exact.
            Field::new(f::ALERT_MISSING_MANPOWER, Kind::Unsigned).unknown_capable(),
        ],
    };

    let garrison_alert_equipment = Collection {
        collection_id: GARRISON_ALERT_EQUIPMENT,
        shape: Shape::KeyedSet,
        fields: vec![
            // `CEquipmentArchetype+0x418` (`domain44_compact.rs:95`). §5 names
            // equipment as content-defined, so this is a ContentId key.
            Field::new(f::EQUIPMENT_ARCHETYPE_ID, Kind::ContentId).key(0),
            // `whole_units <= 0` is a rejected row (`world_fold.rs::decode_occupation_garrison_alert`).
            Field::new(f::EQUIPMENT_WHOLE_UNITS, Kind::Unsigned),
        ],
    };

    let collaboration_status = Collection {
        collection_id: COLLABORATION_STATUS,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::COLLABORATION_SINGLETON, Kind::Unsigned).key(0),
            Field::new(f::COLLABORATION_SOURCE_UNAVAILABLE, Kind::Boolean),
            // Hard-zeroed exactly when the source is unavailable
            // (`world_fold.rs::decode_occupation_collaboration`), i.e. the country population was
            // never counted — **unknown**, not a zero that would read as a
            // measured population of none.
            Field::new(f::COLLABORATION_OBSERVED_COUNTRY_ROWS, Kind::Unsigned).unknown_capable(),
            // Constrained to zero on the complete arm and UNconstrained on the
            // unavailable one, so it is a measured value on both and carries no
            // empty state.
            Field::new(f::COLLABORATION_ANOMALY_ROWS, Kind::Unsigned),
        ],
    };

    let collaboration_relations = Collection {
        collection_id: COLLABORATION_RELATIONS,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::RELATION_OWNER_TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::RELATION_TARGET_TAG_DB_ID, Kind::ContentId).key(1),
            // `progress_raw / FIXED_SCALE` (`world_fold.rs::build_snapshot`), an i64 the
            // decoder bounds to `0..=100_000` (`world_fold.rs::decode_occupation_collaboration`). `Fixed(5)` is the
            // source's own decimal grid, exactly as for the row scalars.
            Field::new(f::RELATION_PROGRESS, Kind::Continuous).quant(FIXED_QUANT),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            states,
            garrison_alert,
            garrison_alert_equipment,
            collaboration_status,
            collaboration_relations,
        ],
        event_part: None,
    }
}

// --- Header and payload split -----------------------------------------------

/// The 56-byte payload header. Every word is parsed; only `row_count` and
/// `status_flags` are enforced by [`split_payload`].
///
/// `fold_occupation_resistance_compliance` reads two further words — the census
/// pair `observed_state_rows` at `+20` and `invalid_state_rows` at `+52` — and
/// [`split_payload`] deliberately does not enforce either. Enforcement there
/// would mean refusing the RECORD, and a census that does not add up is a
/// refused census, not a refused hour: the rows the record carries are still the
/// engine's rows. The fold publishes the pair with its own verdict as
/// `_r2_occupation_census_live` (`world_fold.rs::record_occupation_census`) and
/// the replay admission for `mapmode.occupation_resistance_compliance` is what
/// refuses on it
/// (`replay_interface/native/map_viewer/src/map_authority.rs::occupation_resistance_compliance_status`).
///
/// [`check_authority_payload`] is a different question and DOES constrain the
/// pair, mirroring the lane decoder: `row_count + invalid_state_rows <=
/// observed_state_rows` on EVERY frame, and that same sum as an EQUALITY on a
/// non-delta one. Both hold of every frame the producer builds, so a payload
/// that breaks one is malformed rather than merely incomplete — which is why
/// these live here and not in [`split_payload`], whose refusals throw away the
/// hour. The inequality subsumes the two separate bounds that used to stand
/// beside it, which were deleted rather than kept as unisolable guards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    /// Parsed and reported, never enforced here: this fold never reads it.
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_state_rows: u32,
    pub occupied_state_rows: u32,
    pub wall_badge_rows: u32,
    pub scale_raw: u32,
    pub law_token_rows: u32,
    pub garrison_template_rows: u32,
    pub garrison_scalar_rows: u32,
    pub garrison_anomaly_rows: u32,
    /// The frame's COVERAGE VERDICT, at `+52`. Zero means the walk accounted for
    /// every root it could see and the province array did not move under it;
    /// nonzero means the hour's coverage is not to be trusted. The magnitude is
    /// an accounting quantity, not a defensible count of lost states — on a
    /// replaced array it is simply the roots the old walk found, which bears no
    /// relation to the new array's size.
    pub invalid_state_rows: u32,
}

impl PayloadHeader {
    pub fn is_delta(&self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
}

/// Read the header alone, without paying for the body.
///
/// A caller that needs to *classify* a record — delta or not, which status — must
/// be able to do it before the length arithmetic, because the fold's own
/// delta-before-baseline exit sits between the two.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "occupation payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_state_rows: u32_at(payload, 20),
        occupied_state_rows: u32_at(payload, 24),
        wall_badge_rows: u32_at(payload, 28),
        scale_raw: u32_at(payload, 32),
        law_token_rows: u32_at(payload, 36),
        garrison_template_rows: u32_at(payload, 40),
        garrison_scalar_rows: u32_at(payload, 44),
        garrison_anomaly_rows: u32_at(payload, 48),
        invalid_state_rows: u32_at(payload, 52),
    })
}

/// Split a d52 DATA payload into its header, its state rows and the trailing
/// bytes, applying exactly the fold's structural checks
/// (`world_fold.rs::fold_occupation_resistance_compliance`).
///
/// Note what is **not** here. The fold requires `payload.len() >= base_len`, not
/// equality: everything past the rows is the trailer region, and its own grammar
/// is [`parse_trailers`]'s. And `changed_rows` is not compared to anything,
/// because this fold never reads it — enforcing the lane contract's
/// `changed_rows <= row_count` here would refuse records the recorder folds.
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = peek_header(payload)?;
    // Checked arithmetic: `row_count` is attacker-shaped input from a stream on
    // disk, and `row_count * ROW_LEN` overflows `usize` for a large enough word.
    let base_len = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("occupation row_count {} overflows", header.row_count))?;
    if payload.len() < base_len {
        return Err(format!("occupation len {} < base {base_len}", payload.len()));
    }
    Ok((header, &payload[HEADER_LEN..base_len], &payload[base_len..]))
}

// --- Trailers ----------------------------------------------------------------

/// One `GAR1` missing-equipment row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlertEquipmentRow {
    pub archetype_id: u32,
    pub whole_units: i64,
}

/// The decoded `GAR1` trailer (`world_fold.rs::decode_occupation_garrison_alert`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GarrisonAlert {
    pub game_hour_raw: i32,
    pub controller_tag_db_id: u32,
    pub flags: u32,
    pub missing_manpower: i64,
    pub equipment: Vec<AlertEquipmentRow>,
}

impl GarrisonAlert {
    pub fn source_unavailable(&self) -> bool {
        self.flags & GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE != 0
    }
    pub fn predicate_returned(&self) -> bool {
        self.flags & GARRISON_ALERT_FLAG_PREDICATE_RETURNED != 0
    }
}

/// One `COL1` relation row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CollaborationRelationRow {
    pub owner_tag_db_id: u32,
    pub target_tag_db_id: u32,
    pub progress_raw: i64,
}

/// The decoded `COL1` trailer (`world_fold.rs::decode_occupation_collaboration`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Collaboration {
    pub flags: u32,
    pub observed_country_rows: u32,
    pub anomaly_rows: u32,
    pub rows: Vec<CollaborationRelationRow>,
}

impl Collaboration {
    pub fn source_unavailable(&self) -> bool {
        self.flags & COLLABORATION_FLAG_SOURCE_UNAVAILABLE != 0
    }
}

/// Both trailers, as the fold reads them: `GAR1` first and mandatory once the
/// trailer region is non-empty, `COL1` second and optional
/// (`world_fold.rs::fold_occupation_resistance_compliance`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Trailers {
    pub garrison_alert: Option<GarrisonAlert>,
    pub collaboration: Option<Collaboration>,
}

/// Decode the trailer region. An empty region is the legal "no trailers" case
/// (`world_fold.rs::fold_occupation_resistance_compliance`); anything else must be a well-formed `GAR1`, optionally
/// followed by a well-formed `COL1` that consumes the rest exactly.
pub fn parse_trailers(trailers: &[u8]) -> core::result::Result<Trailers, String> {
    if trailers.is_empty() {
        return Ok(Trailers::default());
    }
    let alert_len = garrison_alert_len(trailers)?;
    let alert = parse_garrison_alert(&trailers[..alert_len])?;
    let rest = &trailers[alert_len..];
    let collaboration = if rest.is_empty() {
        None
    } else {
        Some(parse_collaboration(rest)?)
    };
    Ok(Trailers {
        garrison_alert: Some(alert),
        collaboration,
    })
}

/// `occupation_garrison_alert_len`, `world_fold.rs::occupation_garrison_alert_len`.
pub fn garrison_alert_len(trailers: &[u8]) -> core::result::Result<usize, String> {
    need(trailers, GARRISON_ALERT_HEADER_LEN, "occupation GAR1 header")?;
    if &trailers[0..4] != GARRISON_ALERT_MAGIC
        || u16_at(trailers, 4) != GARRISON_ALERT_SCHEMA_VERSION
        || u16_at(trailers, 6) as usize != GARRISON_ALERT_HEADER_LEN
    {
        return Err("occupation GAR1 magic/schema/header is invalid".into());
    }
    let equipment_count = u32_at(trailers, 28) as usize;
    let len = equipment_count
        .checked_mul(GARRISON_ALERT_ROW_LEN)
        .and_then(|rows| rows.checked_add(GARRISON_ALERT_HEADER_LEN))
        .ok_or_else(|| format!("occupation GAR1 equipment_count {equipment_count} overflows"))?;
    need(trailers, len, "occupation GAR1 payload")?;
    Ok(len)
}

/// `decode_occupation_garrison_alert`, `world_fold.rs::decode_occupation_garrison_alert`. The slice
/// must be exactly the trailer, as the fold slices it before calling.
pub fn parse_garrison_alert(trailer: &[u8]) -> core::result::Result<GarrisonAlert, String> {
    need(trailer, GARRISON_ALERT_HEADER_LEN, "occupation GAR1 header")?;
    if &trailer[0..4] != GARRISON_ALERT_MAGIC
        || u16_at(trailer, 4) != GARRISON_ALERT_SCHEMA_VERSION
        || u16_at(trailer, 6) as usize != GARRISON_ALERT_HEADER_LEN
    {
        return Err("occupation GAR1 magic/schema/header is invalid".into());
    }
    let game_hour_raw = i32_at(trailer, 8);
    let controller_tag_db_id = u32_at(trailer, 12);
    let flags = u32_at(trailer, 16);
    let missing_manpower = i64_at(trailer, 20);
    let equipment_count = u32_at(trailer, 28) as usize;
    let divisor = u32_at(trailer, 32);
    let reserved = u32_at(trailer, 36);
    let expected = equipment_count
        .checked_mul(GARRISON_ALERT_ROW_LEN)
        .and_then(|rows| rows.checked_add(GARRISON_ALERT_HEADER_LEN))
        .ok_or_else(|| format!("occupation GAR1 equipment_count {equipment_count} overflows"))?;
    if trailer.len() != expected {
        return Err(format!(
            "occupation GAR1 len {} != {expected}",
            trailer.len()
        ));
    }
    let unavailable = flags & GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE != 0;
    let complete = flags & GARRISON_ALERT_FLAG_COMPLETE != 0;
    let predicate_returned = flags & GARRISON_ALERT_FLAG_PREDICATE_RETURNED != 0;
    if flags & !GARRISON_ALERT_KNOWN_FLAGS != 0
        || divisor != FIXED_SCALE_RAW
        || reserved != 0
        || unavailable == complete
        || (flags & GARRISON_ALERT_FLAG_MISSING_MANPOWER != 0) != (missing_manpower > 0)
        || missing_manpower < 0
        || (unavailable
            && (controller_tag_db_id != 0
                || predicate_returned
                || missing_manpower != 0
                || equipment_count != 0))
        || (complete && controller_tag_db_id == 0)
        || (complete && predicate_returned == (missing_manpower > 0 || equipment_count != 0))
    {
        return Err("occupation GAR1 header invariants are invalid".into());
    }
    let mut equipment = Vec::with_capacity(equipment_count);
    let mut seen: Vec<u32> = Vec::with_capacity(equipment_count);
    for index in 0..equipment_count {
        let cursor = GARRISON_ALERT_HEADER_LEN + index * GARRISON_ALERT_ROW_LEN;
        let archetype_id = u32_at(trailer, cursor);
        let whole_units = i64_at(trailer, cursor + 4);
        let row_reserved = u32_at(trailer, cursor + 12);
        if archetype_id == 0 || whole_units <= 0 || row_reserved != 0 || seen.contains(&archetype_id)
        {
            return Err(format!("occupation GAR1 equipment row {index} is invalid"));
        }
        seen.push(archetype_id);
        equipment.push(AlertEquipmentRow {
            archetype_id,
            whole_units,
        });
    }
    Ok(GarrisonAlert {
        game_hour_raw,
        controller_tag_db_id,
        flags,
        missing_manpower,
        equipment,
    })
}

/// `decode_occupation_collaboration`, `world_fold.rs::decode_occupation_collaboration`.
pub fn parse_collaboration(trailer: &[u8]) -> core::result::Result<Collaboration, String> {
    need(trailer, COLLABORATION_HEADER_LEN, "occupation COL1 header")?;
    if &trailer[0..4] != COLLABORATION_MAGIC
        || u16_at(trailer, 4) != COLLABORATION_SCHEMA_VERSION
        || u16_at(trailer, 6) as usize != COLLABORATION_HEADER_LEN
    {
        return Err("occupation COL1 magic/schema/header is invalid".into());
    }
    let flags = u32_at(trailer, 8);
    let row_count = u32_at(trailer, 12) as usize;
    let observed_country_rows = u32_at(trailer, 16);
    let anomaly_rows = u32_at(trailer, 20);
    let divisor = u32_at(trailer, 24);
    let reserved = u32_at(trailer, 28);
    let expected = row_count
        .checked_mul(COLLABORATION_ROW_LEN)
        .and_then(|rows| rows.checked_add(COLLABORATION_HEADER_LEN))
        .ok_or_else(|| format!("occupation COL1 row_count {row_count} overflows"))?;
    let unavailable = flags & COLLABORATION_FLAG_SOURCE_UNAVAILABLE != 0;
    let complete = flags & COLLABORATION_FLAG_COMPLETE != 0;
    if trailer.len() != expected
        || flags & !COLLABORATION_KNOWN_FLAGS != 0
        || unavailable == complete
        || divisor != FIXED_SCALE_RAW
        || reserved != 0
        || (unavailable && (row_count != 0 || observed_country_rows != 0))
        || (complete && observed_country_rows == 0)
        || (complete && anomaly_rows != 0)
    {
        return Err("occupation COL1 header invariants are invalid".into());
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut seen: Vec<(u32, u32)> = Vec::with_capacity(row_count);
    for index in 0..row_count {
        let cursor = COLLABORATION_HEADER_LEN + index * COLLABORATION_ROW_LEN;
        let owner = u32_at(trailer, cursor);
        let target = u32_at(trailer, cursor + 4);
        let progress_raw = i64_at(trailer, cursor + 8);
        if owner == 0
            || owner > MAX_TAG_DB_ID
            || target == 0
            || target > MAX_TAG_DB_ID
            || owner == target
            || !(0..=FIXED_SCALE_RAW as i64).contains(&progress_raw)
            || seen.contains(&(owner, target))
        {
            return Err(format!("occupation COL1 row {index} is invalid"));
        }
        seen.push((owner, target));
        rows.push(CollaborationRelationRow {
            owner_tag_db_id: owner,
            target_tag_db_id: target,
            progress_raw,
        });
    }
    Ok(Collaboration {
        flags,
        observed_country_rows,
        anomaly_rows,
        rows,
    })
}

/// `MAX_TAG_DB_ID`, `world_fold.rs::decode_occupation_collaboration` and `domain44_compact.rs:50`.
///
/// Despite the name this is the `_CountryLinkTags` index space, NOT the country
/// database's — see the provenance in
/// `lanes/occupation_resistance_compliance_live/collector_module.rs`, and note
/// that a refusal here is attributed to
/// `bound_trips::BOUND_TRIP_COUNTRY_LINK_INDEX` rather than to the database's
/// id. Applying one space's bound to the other would be a category error.
///
/// Was `1_024` — the tightest spelling in the family and 2.3x the vanilla
/// country count, and an admitted content guess in a space whose engine bound
/// is a live field (`u32[CGameState + 0x364]`). ENGINE-STRUCTURAL now, for THIS
/// space: the engine's PDX vectors carry a signed 32-bit size and subscript
/// with a signed index, so the widest index any loaded link vector can hold is
/// `i32::MAX - 1` — the identical derivation the country database's bound uses,
/// applied to its own vector rather than borrowed from it.
///
/// Bounding against the live size instead of the structural width still needs
/// `u32[CGameState + 0x364]` carried in the d52 header; that is lane GV's
/// residual R-GV-3 and is a grammar change, not a bounds edit. What it would
/// buy is tightness, not coverage: no row is discarded at this width.
pub const MAX_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
/// `MAX_STATE_ID`, `domain44_compact.rs:49` — a lane-contract bound, not a fold
/// one, so it is enforced in [`check_authority_payload`] and nowhere else.
/// Was `65_535`, a guess; now the state id space's STRUCTURAL width
/// (`claim_39c989ce13504504353c`, `runs/ia_province_caps/SWEEP.md`).
pub const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
/// `MAX_ABS_FIXED_RAW`, `domain44_compact.rs:51` — likewise lane-contract only.
pub const MAX_ABS_FIXED_RAW: i32 = 100_000_000;

// --- Keys --------------------------------------------------------------------

/// The key of one state row: its `state_id`, with the fold's zero check
/// (`world_fold.rs::fold_occupation_resistance_compliance`).
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[state_id_of_row(row)? as u64]))
}

pub fn state_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let state_id = u32_at(row, 0);
    if state_id == 0 {
        return Err("occupation row has zero state_id".into());
    }
    Ok(state_id)
}

pub fn key_from_equipment_row(entry: &AlertEquipmentRow) -> Key {
    key(&[entry.archetype_id as u64])
}

pub fn key_from_collaboration_row(entry: &CollaborationRelationRow) -> Key {
    key(&[entry.owner_tag_db_id as u64, entry.target_tag_db_id as u64])
}

// --- The boundary mapping ----------------------------------------------------

/// One compact row of the `states` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 64-byte row slice (the caller has already
/// consumed the table header via [`split_payload`]); d52 has exactly one row form,
/// so length selects nothing and a wrong length is rejected. `frame_tick` is §6's
/// derived-field input; d52 declares no derived field, so it is taken and ignored
/// — every domain's mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_occupation_resistance_compliance`'s row
/// loop (`world_fold.rs::fold_occupation_resistance_compliance`): a row the fold would
/// reject is an `Err` here, never a coerced value, and every check the fold does
/// **not** make is left to [`check_authority_payload`]. Nothing in this function
/// can panic on any 64-byte input.
///
/// A row that folds to a **deletion** ([`row_folds_to_deletion`]) is still mapped
/// rather than refused — the caller decides removal — because the mapper's job is
/// the row's values and the accumulator's job is the fold's insert/remove rule.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d52 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    if collection.collection_id != STATES {
        return Err(format!(
            "record_from_row called with collection {} rather than STATES ({STATES})",
            collection.collection_id
        ));
    }
    let flags = row_flags(row)?;

    let owner = u32_at(row, 4);
    let controller = u32_at(row, 8);
    let occupied = u32_at(row, 12);
    let occupied_state = flags & ROW_FLAG_OCCUPIED_STATE != 0;
    let status_root = flags & ROW_FLAG_OCCUPATION_STATUS_ROOT != 0;
    let garrison_measured = flags & ROW_FLAG_GARRISON_SCALARS != 0;
    let garrison_unavailable = flags & ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0;
    let speed_unavailable = flags & ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE != 0;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::RESISTANCE => gated_fixed(flags & ROW_FLAG_RESISTANCE != 0, row, 16),
            field::COMPLIANCE => gated_fixed(flags & ROW_FLAG_COMPLIANCE != 0, row, 20),
            field::RESISTANCE_SPEED => gated_fixed(flags & ROW_FLAG_RESISTANCE_SPEED != 0, row, 24),
            field::COMPLIANCE_SPEED => gated_fixed(flags & ROW_FLAG_COMPLIANCE_SPEED != 0, row, 28),
            field::RESISTANCE_SPEED_UNAVAILABLE => Value::Bool(speed_unavailable),
            field::OCCUPIER_TAG => {
                if !occupied_state {
                    Value::Absent
                } else if flags & ROW_FLAG_CONTROLLER != 0 {
                    Value::ContentId(controller as u64)
                } else {
                    Value::Null
                }
            }
            field::OCCUPIED_COUNTRY_TAG => {
                if !occupied_state {
                    Value::Absent
                } else if flags & ROW_FLAG_OCCUPIED_TAG_DERIVED != 0 {
                    Value::ContentId(occupied as u64)
                } else if flags & ROW_FLAG_OWNER != 0 {
                    // The fold's own fallback (`world_fold.rs::fold_occupation_resistance_compliance`):
                    // a derived value, not a carried one.
                    Value::ContentId(owner as u64)
                } else {
                    Value::Null
                }
            }
            // The one field in this row with NO divisor.
            field::GARRISON_ARMY_MANPOWER_NEED => {
                if garrison_measured {
                    Value::Signed(i32_at(row, 32) as i64)
                } else if garrison_unavailable {
                    Value::Unknown
                } else {
                    Value::Absent
                }
            }
            field::GARRISON_STRENGTH_RATIO => {
                garrison_fixed(garrison_measured, garrison_unavailable, row, 36)
            }
            field::GARRISON_REQUIRED => {
                garrison_fixed(garrison_measured, garrison_unavailable, row, 40)
            }
            field::OCCUPATION_LAW_TOKEN_ID => {
                if flags & ROW_FLAG_OCCUPATION_LAW_TOKEN != 0 {
                    Value::ContentId(u32_at(row, 44) as u64)
                } else {
                    // Never Null: a clear flag is a zero token, and a zero token
                    // is produced both by "no default law" and by three distinct
                    // read failures the collector collapses into it.
                    Value::Unknown
                }
            }
            field::OCCUPATION_LAW_TOKEN_IS_COUNTRY_DEFAULT => {
                Value::Bool(flags & ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN != 0)
            }
            field::OCCUPATION_LAW_UNAVAILABLE => {
                Value::Bool(flags & ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE != 0)
            }
            field::GARRISON_TEMPLATE_ID => {
                if flags & ROW_FLAG_GARRISON_TEMPLATE != 0 {
                    Value::Unsigned(u32_at(row, 48) as u64)
                } else if !occupied_state {
                    // The collector only attempts the read under the occupied
                    // gate (`collector_module.rs:1111`-`:1115`).
                    Value::Absent
                } else {
                    Value::Unknown
                }
            }
            field::OCCUPATION_STATUS_ROOT => Value::Bool(status_root),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!("occupation has no mapping for field id {other}"));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The row's flag word, with every check the FOLD makes before it reads a field
/// (`world_fold.rs::fold_occupation_resistance_compliance`).
pub fn row_flags(row: &[u8]) -> core::result::Result<u32, String> {
    // The key's validity is part of the row's validity even though the key is not
    // carried in the record body (§3).
    let _ = state_id_of_row(row)?;
    let flags = u32_at(row, 52);
    if flags & !KNOWN_FLAGS != 0 {
        return Err(format!("occupation row has unknown flags {flags:#x}"));
    }
    if u32_at(row, 56) != 0 || u32_at(row, 60) != 0 {
        return Err("occupation row reserved field is nonzero".into());
    }
    if flags & ROW_FLAG_OWNER != 0 && u32_at(row, 4) == 0 {
        return Err("occupation row has owner flag without tag".into());
    }
    if flags & ROW_FLAG_CONTROLLER != 0 && u32_at(row, 8) == 0 {
        return Err("occupation row has controller flag without tag".into());
    }
    if flags & ROW_FLAG_OCCUPIED_TAG_DERIVED != 0 && u32_at(row, 12) == 0 {
        return Err("occupation row has occupied-country flag without tag".into());
    }
    if flags & ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0
        && flags & ROW_FLAG_GARRISON_SCALARS != 0
    {
        return Err("occupation row marks garrison scalars both measured and unavailable".into());
    }
    if flags & (ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE | ROW_FLAG_GARRISON_SCALARS) == 0
        && (i32_at(row, 32) != 0 || i32_at(row, 36) != 0 || i32_at(row, 40) != 0)
    {
        // `world_fold.rs::fold_occupation_resistance_compliance`: "Values without a measured badge are
        // torn/foreign data: fail the fold closed rather than emit them."
        return Err("occupation row has garrison scalar values without measured flag".into());
    }
    Ok(flags)
}

/// Whether this row makes the fold's `occupation` object empty, i.e. whether
/// folding it **removes** the state from this domain's slice
/// (`world_fold.rs::fold_occupation_resistance_compliance`). See the module docs.
pub fn row_folds_to_deletion(row: &[u8]) -> core::result::Result<bool, String> {
    let flags = row_flags(row)?;
    let has_content = flags & OCCUPATION_CONTENT_FLAGS != 0;
    let has_tag = flags & ROW_FLAG_OCCUPIED_STATE != 0
        && flags
            & (ROW_FLAG_OWNER | ROW_FLAG_CONTROLLER | ROW_FLAG_OCCUPIED_TAG_DERIVED)
            != 0;
    Ok(!has_content && !has_tag)
}

/// The `GAR1` header, onto the [`GARRISON_ALERT`] singleton's Record.
pub fn record_from_garrison_alert(
    collection: &Collection,
    alert: &GarrisonAlert,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != GARRISON_ALERT {
        return Err(format!(
            "record_from_garrison_alert called with collection {}",
            collection.collection_id
        ));
    }
    let unavailable = alert.source_unavailable();
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::ALERT_SOURCE_UNAVAILABLE => Value::Bool(unavailable),
            field::ALERT_GAME_HOUR_RAW => Value::Signed(alert.game_hour_raw as i64),
            // On the unavailable arm these three words are grammar, not
            // measurement: the header invariants hard-zero them
            // (`world_fold.rs::decode_occupation_garrison_alert`) because nothing was read.
            field::ALERT_CONTROLLER_TAG_DB_ID => {
                if unavailable {
                    Value::Unknown
                } else {
                    Value::ContentId(alert.controller_tag_db_id as u64)
                }
            }
            field::ALERT_PREDICATE_RETURNED => {
                if unavailable {
                    Value::Unknown
                } else {
                    Value::Bool(alert.predicate_returned())
                }
            }
            field::ALERT_MISSING_MANPOWER => {
                if unavailable {
                    Value::Unknown
                } else {
                    // `parse_garrison_alert` refuses a negative word, so this
                    // cast is total. It is written as a guarded conversion rather
                    // than an `as` so a future loosening cannot silently wrap.
                    let raw = u64::try_from(alert.missing_manpower)
                        .map_err(|_| "occupation GAR1 missing_manpower is negative".to_string())?;
                    Value::Unsigned(raw)
                }
            }
            other => return Err(format!("occupation GAR1 has no mapping for field id {other}")),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `GAR1` equipment row.
pub fn record_from_equipment_row(
    collection: &Collection,
    entry: &AlertEquipmentRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != GARRISON_ALERT_EQUIPMENT {
        return Err(format!(
            "record_from_equipment_row called with collection {}",
            collection.collection_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::EQUIPMENT_WHOLE_UNITS => {
                let raw = u64::try_from(entry.whole_units)
                    .map_err(|_| "occupation GAR1 whole_units is negative".to_string())?;
                Value::Unsigned(raw)
            }
            other => {
                return Err(format!(
                    "occupation GAR1 equipment has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The `COL1` header, onto the [`COLLABORATION_STATUS`] singleton's Record.
pub fn record_from_collaboration_status(
    collection: &Collection,
    collaboration: &Collaboration,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != COLLABORATION_STATUS {
        return Err(format!(
            "record_from_collaboration_status called with collection {}",
            collection.collection_id
        ));
    }
    let unavailable = collaboration.source_unavailable();
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::COLLABORATION_SOURCE_UNAVAILABLE => Value::Bool(unavailable),
            field::COLLABORATION_OBSERVED_COUNTRY_ROWS => {
                if unavailable {
                    // Hard-zeroed by the header invariants because the country
                    // population was never counted (`world_fold.rs::decode_occupation_collaboration`).
                    Value::Unknown
                } else {
                    Value::Unsigned(collaboration.observed_country_rows as u64)
                }
            }
            field::COLLABORATION_ANOMALY_ROWS => {
                Value::Unsigned(collaboration.anomaly_rows as u64)
            }
            other => return Err(format!("occupation COL1 has no mapping for field id {other}")),
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One `COL1` relation row.
pub fn record_from_collaboration_row(
    collection: &Collection,
    entry: &CollaborationRelationRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != COLLABORATION_RELATIONS {
        return Err(format!(
            "record_from_collaboration_row called with collection {}",
            collection.collection_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::RELATION_PROGRESS => {
                Value::Continuous(entry.progress_raw as f64 / FIXED_SCALE)
            }
            other => {
                return Err(format!(
                    "occupation COL1 relations have no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame-level fold ----------------------------------------------------

/// What one payload did to the accumulator — the vocabulary a harness needs to
/// tell an excluded hour from a measured one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// A non-delta frame: the whole slice was cleared and rebuilt.
    Baseline,
    /// A delta frame applied to the retained accumulator.
    Delta,
    /// `DELTA_DATA` before any baseline (`world_fold.rs::fold_occupation_resistance_compliance`). The
    /// base table is left untouched — not cleared, not upserted, and its rows are
    /// never parsed, because the fold returns before its row loop — and the
    /// domain-hour is marked unknown. **The trailers are still this hour's and
    /// are still installed**, because the fold assigns them before taking this
    /// exit and the recorder publishes them from there; that asymmetry with the
    /// failure path (which overwrites them, `world_fold.rs::fold_occupation_resistance_compliance`) is the whole
    /// reason this outcome has its own name. An hour in this state carries
    /// measured trailer data beside an unknown base table, which no keyed set can
    /// express, so a measurement harness must refuse the run rather than skip the
    /// hour — see `bin/bench_real_d52.rs`.
    DeltaBeforeBaseline,
}

/// The five per-hour keyed states d52 folds into, plus the one bit of fold state
/// the payload grammar depends on.
///
/// One record is not one state for this domain: the base table is
/// baseline-plus-delta, so an hour's `states` is the whole history of the stream
/// up to that hour rather than that hour's rows. The two trailers are the
/// opposite — they are reset at the start of every hour
/// (`world_fold.rs::reset_hour_state`) and are therefore whole-cloth per hour. This is
/// the recipe's fourth public boundary, and it exists so the measurement bin, the
/// tests and any future caller share ONE replay of the shipping layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Collection [`STATES`], keyed by [`key_from_row`].
    pub states: KeyedState,
    /// Collection [`GARRISON_ALERT`] — at most the one singleton row.
    pub garrison_alert: KeyedState,
    /// Collection [`GARRISON_ALERT_EQUIPMENT`], keyed by archetype.
    pub garrison_alert_equipment: KeyedState,
    /// Collection [`COLLABORATION_STATUS`] — at most the one singleton row.
    pub collaboration_status: KeyedState,
    /// Collection [`COLLABORATION_RELATIONS`], keyed by `(owner, target)`.
    pub collaboration_relations: KeyedState,
    /// `delta_baselines_seen.contains(&52)` (`world_fold.rs::fold_occupation_resistance_compliance`).
    baseline_seen: bool,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a baseline has landed. Public because the delta-before-baseline
    /// exclusion is decided from it.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// The five keyed states in schema order, for `encode_baseline` / `diff`.
    pub fn collections(&self) -> [&KeyedState; 5] {
        [
            &self.states,
            &self.garrison_alert,
            &self.garrison_alert_equipment,
            &self.collaboration_status,
            &self.collaboration_relations,
        ]
    }

    /// Fold one d52 DATA payload exactly as `fold_occupation_resistance_compliance`
    /// folds it (`world_fold.rs::fold_occupation_resistance_compliance`), in the fold's own order.
    ///
    /// The caller is responsible for the **failure-status** exclusion, because the
    /// predicate is `compact_contract`'s
    /// (`compact_status_failure_flags_after_expected_partial_tolerance`) and this
    /// crate's library half must not depend on that crate nor copy its mask. The
    /// fold consults it at `world_fold.rs::fold_occupation_resistance_compliance` — **after** the header checks and after the
    /// trailer decode, and before everything else — so a caller that gets the
    /// order right calls [`split_payload`] and [`parse_trailers`], excludes on
    /// status, and only then calls this.
    ///
    /// The **delta-before-baseline** exclusion is handled here, because it needs
    /// no vocabulary this crate lacks: it is `DELTA_DATA` against
    /// [`State::baseline_seen`], and the fold makes the same decision before
    /// touching anything (`world_fold.rs::fold_occupation_resistance_compliance`).
    ///
    /// **Error contract: all-or-nothing.** Every trailer record and every row
    /// record is built before the first mutation, mirroring the fold, whose
    /// row-loop errors
    /// all precede the `states.entry(..)` write for that row — with one
    /// difference that matters and is therefore reproduced rather than glossed:
    /// the fold writes each row as it goes, so a fold that errors on row `k` has
    /// already written rows `0..k` **and** has already run the non-delta clear.
    /// A recorder in that state aborts the whole stream fold
    /// (`WorldFold::ingest_record` propagates the `Err`), so no consumer observes
    /// the half-written accumulator; this port therefore leaves the accumulator
    /// untouched on `Err`, which is the same observable behaviour and a safer one.
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
    ) -> core::result::Result<FoldOutcome, String> {
        let (header, rows, trailer_bytes) = split_payload(payload)?;
        // The fold decodes the trailers BEFORE it reads the status word
        // (`:16812`-`:16823` then `:16825`), so a malformed trailer refuses the
        // stream even on a frame that folds nothing.
        let trailers = parse_trailers(trailer_bytes)?;

        let delta = header.is_delta();

        let states_collection = collection_of(schema, STATES)?;
        let alert_collection = collection_of(schema, GARRISON_ALERT)?;
        let equipment_collection = collection_of(schema, GARRISON_ALERT_EQUIPMENT)?;
        let status_collection = collection_of(schema, COLLABORATION_STATUS)?;
        let relations_collection = collection_of(schema, COLLABORATION_RELATIONS)?;

        // The trailers are built FIRST, and installed even on the
        // delta-before-baseline exit, because that is what the fold does: it
        // assigns them at `world_fold.rs::fold_occupation_resistance_compliance` and only then takes the
        // exit at `world_fold.rs::fold_occupation_resistance_compliance`, and the recorder publishes them from there
        // (`world_fold.rs::build_snapshot`). Every one of these mappers is
        // infallible on a payload `parse_trailers` accepted — the only errors
        // they can return are a wrong collection id and a negative word the
        // parser already refused — so building them here cannot make this
        // function fail on an input the fold folds.
        let mut alert_state = KeyedState::new();
        let mut equipment_state = KeyedState::new();
        if let Some(alert) = &trailers.garrison_alert {
            alert_state.insert(
                alert_singleton_key(),
                record_from_garrison_alert(alert_collection, alert, 0)?,
            );
            for entry in &alert.equipment {
                equipment_state.insert(
                    key_from_equipment_row(entry),
                    record_from_equipment_row(equipment_collection, entry, 0)?,
                );
            }
        }
        let mut status_state = KeyedState::new();
        let mut relations_state = KeyedState::new();
        if let Some(collaboration) = &trailers.collaboration {
            status_state.insert(
                collaboration_singleton_key(),
                record_from_collaboration_status(status_collection, collaboration, 0)?,
            );
            for entry in &collaboration.rows {
                relations_state.insert(
                    key_from_collaboration_row(entry),
                    record_from_collaboration_row(relations_collection, entry, 0)?,
                );
            }
        }

        // `world_fold.rs::fold_occupation_resistance_compliance`. The base table is left exactly as it
        // was — no clear, no upsert, no `baseline_seen` — and the rows are never
        // even parsed, because the fold returns before its row loop. The
        // trailers, though, are this hour's and stay.
        if delta && !self.baseline_seen {
            self.garrison_alert = alert_state;
            self.garrison_alert_equipment = equipment_state;
            self.collaboration_status = status_state;
            self.collaboration_relations = relations_state;
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        // Build every row before mutating anything.
        let mut pending: Vec<(Key, Option<Record>)> = Vec::with_capacity(rows.len() / ROW_LEN);
        for row in rows.chunks_exact(ROW_LEN) {
            let row_key = key_from_row(row)?;
            if row_folds_to_deletion(row)? {
                pending.push((row_key, None));
            } else {
                pending.push((row_key, Some(record_from_row(states_collection, row, 0)?)));
            }
        }

        // `world_fold.rs::fold_occupation_resistance_compliance`: a non-delta frame removes this domain's
        // two keys from EVERY state before folding its own rows. Over this
        // domain's slice that is exactly a clear.
        if !delta {
            self.states.clear();
        }
        for (row_key, record) in pending {
            match record {
                // `world_fold.rs::fold_occupation_resistance_compliance` — the empty-occupation removal.
                None => {
                    self.states.remove(&row_key);
                }
                // `:17021` — a plain insert, so a key repeated inside one frame is
                // last-writer-wins exactly as the fold's `entry`/`insert` is. The
                // lane contract rejects the duplicate (`domain44_compact.rs:353`),
                // which is `check_authority_payload`'s job, not this one's.
                Some(record) => {
                    self.states.insert(row_key, record);
                }
            }
        }
        // The trailers are per-hour, cleared at the top of every hour
        // (`world_fold.rs::reset_hour_state`), so they are replaced wholesale and are
        // empty on an hour whose payload carries no trailer.
        self.garrison_alert = alert_state;
        self.garrison_alert_equipment = equipment_state;
        self.collaboration_status = status_state;
        self.collaboration_relations = relations_state;

        if !delta {
            // `world_fold.rs::fold_occupation_resistance_compliance`.
            self.baseline_seen = true;
            Ok(FoldOutcome::Baseline)
        } else {
            Ok(FoldOutcome::Delta)
        }
    }
}

fn collection_of(schema: &Schema, id: u32) -> core::result::Result<&Collection, String> {
    schema
        .collections
        .iter()
        .find(|c| c.collection_id == id)
        .ok_or_else(|| format!("schema carries no collection {id}"))
}

// --- The lane contract's own decode (production's SECOND authority) ----------

/// Every check `occupation_resistance_compliance_live`'s payload decoder makes
/// (`lanes/occupation_resistance_compliance_live/domain44_compact.rs:235`-`:610`),
/// which `recorder_host::domain_data_authority` runs on **every** DOMAIN_DATA
/// record whatever its status; any complaint makes the record
/// `malformed_payload_records`, which `strict_stream_certification_verdict`
/// refuses.
///
/// It is deliberately separate from [`record_from_row`]. The mapper mirrors the
/// **fold**, which is far more permissive: it never checks `changed_rows`, the
/// header's cross-counters, `scale_raw`, the `MAX_*` bounds, tag-without-flag,
/// value-without-flag, or `state_id` uniqueness. Folding those into the mapper
/// would refuse rows the recorder folds, which is the data loss this recipe
/// exists to prevent; leaving them out of the harness would measure a stream the
/// recorder refuses. So both exist, in their own places.
///
/// Total: no payload of any length panics, and the first complaint wins (the lane
/// accumulates them; a harness only needs to know the stream is refused).
pub fn check_authority_payload(payload: &[u8]) -> core::result::Result<(), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "occupation payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
    }
    let header = peek_header(payload)?;
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "occupation fixed scale {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    let row_count = header.row_count as usize;
    let base_len = row_count
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("occupation row_count {row_count} overflows"))?;
    if payload.len() < base_len {
        return Err(format!(
            "occupation payload is {} byte(s), expected at least {base_len}",
            payload.len()
        ));
    }
    // The census invariants, mirrored from the lane decoder
    // (`occupation_resistance_compliance_live/domain44_compact.rs::decode_payload`).
    // This function is the SECONDARY authority `bench_real_d52` treats as
    // decoder-equivalent, so an invariant enforced only over there would let a
    // stream certify here and be refused in production.
    // Rows and drops together never exceed the roots walked, on ANY frame. A
    // delta's rows are a subset of the same valid extraction the verdict
    // describes (`collector_module.rs::delta_rows` selects from it), so the
    // inequality holds for every frame the producer can build even though only a
    // FULL frame satisfies it with equality.
    //
    // Two standalone checks stood here and were removed for the same reason:
    // `invalid_state_rows > observed_state_rows` and `row_count >
    // observed_state_rows`. Neither count is ever negative, so this one
    // inequality already implies both, and no payload can fail either without
    // failing this.
    //
    // Both operands are widened to `usize` before the addition, exactly as
    // `occupation_resistance_compliance_live/domain44_compact.rs::decode_payload`
    // widens both of its own at read time (the d53 lane has a file of the same
    // name and its own `decode_payload`).
    // The two authorities have to agree on the arithmetic as well as the rule.
    let census_accounted = row_count.saturating_add(header.invalid_state_rows as usize);
    let observed = header.observed_state_rows as usize;
    if census_accounted > observed {
        return Err(format!(
            "occupation row_count {row_count} + invalid_state_rows {} > observed_state_rows {}",
            header.invalid_state_rows, header.observed_state_rows
        ));
    }
    // A FULL extraction visits every state root exactly once and either emits a
    // row or counts a drop, so equality is an identity rather than a policy. A
    // delta carries only the changed rows and cannot be checked that way, which
    // is the whole reason the verdict is on the wire at all.
    if !header.is_delta() && census_accounted != observed {
        return Err(format!(
            "occupation full frame row_count {row_count} + invalid_state_rows {} != observed_state_rows {}",
            header.invalid_state_rows, header.observed_state_rows
        ));
    }
    if header.changed_rows > header.row_count {
        return Err(format!(
            "occupation changed_rows {} > row_count {row_count}",
            header.changed_rows
        ));
    }
    for (name, value) in [
        ("occupied_state_rows", header.occupied_state_rows),
        ("wall_badge_rows", header.wall_badge_rows),
        ("law_token_rows", header.law_token_rows),
        ("garrison_template_rows", header.garrison_template_rows),
        ("garrison_scalar_rows", header.garrison_scalar_rows),
    ] {
        if value as usize > row_count {
            return Err(format!(
                "occupation counter {name} {value} exceeds row_count {row_count}"
            ));
        }
    }

    let mut seen_states: Vec<u32> = Vec::with_capacity(row_count);
    let mut occupied = 0u32;
    let mut wall_badges = 0u32;
    let mut law_tokens = 0u32;
    let mut templates = 0u32;
    let mut scalars = 0u32;
    for (index, row) in payload[HEADER_LEN..base_len]
        .chunks_exact(ROW_LEN)
        .enumerate()
    {
        let state_id = u32_at(row, 0);
        let owner = u32_at(row, 4);
        let controller = u32_at(row, 8);
        let occupied_tag = u32_at(row, 12);
        let flags = u32_at(row, 52);
        if state_id == 0 || state_id > MAX_STATE_ID {
            return Err(format!("occupation row {index} has invalid state_id {state_id}"));
        }
        if seen_states.contains(&state_id) {
            return Err(format!("occupation row {index} duplicates state_id {state_id}"));
        }
        seen_states.push(state_id);
        for (name, value, present) in [
            ("owner", owner, flags & ROW_FLAG_OWNER != 0),
            ("controller", controller, flags & ROW_FLAG_CONTROLLER != 0),
            (
                "occupied country",
                occupied_tag,
                flags & ROW_FLAG_OCCUPIED_TAG_DERIVED != 0,
            ),
        ] {
            if value > MAX_TAG_DB_ID {
                return Err(format!(
                    "occupation row {index} has invalid {name} tag_db_id {value}"
                ));
            }
            if !present && value != 0 {
                return Err(format!("occupation row {index} has {name} tag without flag"));
            }
            if present && value == 0 {
                return Err(format!("occupation row {index} has {name} flag without tag"));
            }
        }
        for (name, offset) in [
            ("resistance", 16usize),
            ("compliance", 20),
            ("resistance_speed", 24),
            ("compliance_speed", 28),
            ("garrison_manpower_need", 32),
            ("garrison_strength_ratio", 36),
            ("garrison_required", 40),
        ] {
            let value = i32_at(row, offset);
            // `i32::MIN.abs()` panics under this workspace's overflow checks, so
            // the comparison is written on the widened value.
            if (value as i64).abs() > MAX_ABS_FIXED_RAW as i64 {
                return Err(format!(
                    "occupation row {index} has invalid {name} raw value {value}"
                ));
            }
        }
        if flags & ROW_FLAG_OCCUPIED_STATE != 0 {
            occupied += 1;
        }
        if flags
            & (ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
                | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE)
            != 0
        {
            wall_badges += 1;
        }
        let law_token = u32_at(row, 44);
        if flags & ROW_FLAG_OCCUPATION_LAW_TOKEN != 0 {
            law_tokens += 1;
            if law_token == 0 {
                return Err(format!(
                    "occupation row {index} has law-token flag without token"
                ));
            }
        } else if law_token != 0 {
            return Err(format!("occupation row {index} has law token without flag"));
        }
        let template = u32_at(row, 48);
        if flags & ROW_FLAG_GARRISON_TEMPLATE != 0 {
            templates += 1;
            if template == 0 {
                return Err(format!(
                    "occupation row {index} has garrison-template flag without id"
                ));
            }
        } else if template != 0 {
            return Err(format!(
                "occupation row {index} has garrison template id without flag"
            ));
        }
        let manpower = i32_at(row, 32);
        let ratio = i32_at(row, 36);
        let required = i32_at(row, 40);
        if flags & ROW_FLAG_GARRISON_SCALARS != 0 {
            scalars += 1;
            if flags & ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0 {
                return Err(format!(
                    "occupation row {index} marks garrison scalars both measured and unavailable"
                ));
            }
            if flags & ROW_FLAG_CONTROLLER == 0 {
                return Err(format!(
                    "occupation row {index} has garrison scalars without controller"
                ));
            }
            if manpower < 0 {
                return Err(format!(
                    "occupation row {index} has negative garrison manpower need {manpower}"
                ));
            }
        } else if manpower != 0 || ratio != 0 || required != 0 {
            return Err(format!(
                "occupation row {index} has garrison scalar values without measured flag"
            ));
        }
        if flags & !KNOWN_FLAGS != 0 {
            return Err(format!("occupation row {index} has unknown flags {flags}"));
        }
        for (name, offset, gate) in [
            ("resistance", 16usize, ROW_FLAG_RESISTANCE),
            ("compliance", 20, ROW_FLAG_COMPLIANCE),
            ("resistance speed", 24, ROW_FLAG_RESISTANCE_SPEED),
            ("compliance speed", 28, ROW_FLAG_COMPLIANCE_SPEED),
        ] {
            if flags & gate == 0 && i32_at(row, offset) != 0 {
                return Err(format!("occupation row {index} has {name} without flag"));
            }
        }
        if flags & ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE != 0
            && flags & (ROW_FLAG_RESISTANCE_SPEED | ROW_FLAG_COMPLIANCE_SPEED) != 0
        {
            return Err(format!(
                "occupation row {index} marks speed both measured and unavailable"
            ));
        }
        if flags & ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0
            && (manpower != 0 || ratio != 0 || required != 0)
        {
            return Err(format!(
                "occupation row {index} marks garrison signals unavailable with nonzero garrison values"
            ));
        }
        if flags & ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN != 0
            && flags & ROW_FLAG_OCCUPATION_LAW_TOKEN == 0
        {
            return Err(format!(
                "occupation row {index} marks default law token without law-token flag"
            ));
        }
        if flags & ROW_FLAG_OCCUPIED_TAG_DERIVED != 0
            && !(flags & ROW_FLAG_OWNER != 0 && flags & ROW_FLAG_CONTROLLER != 0)
        {
            return Err(format!(
                "occupation row {index} derives occupied tag without owner/controller"
            ));
        }
        if u32_at(row, 56) != 0 || u32_at(row, 60) != 0 {
            return Err(format!("occupation row {index} reserved field is nonzero"));
        }
    }

    // The lane decodes the trailers here, between the row loop and the counter
    // cross-checks (`domain44_compact.rs:570`-`:583`).
    check_authority_trailers(&payload[base_len..])?;

    for (name, claimed, decoded) in [
        ("occupied_state_rows", header.occupied_state_rows, occupied),
        ("wall_badge_rows", header.wall_badge_rows, wall_badges),
        ("law_token_rows", header.law_token_rows, law_tokens),
        (
            "garrison_template_rows",
            header.garrison_template_rows,
            templates,
        ),
        ("garrison_scalar_rows", header.garrison_scalar_rows, scalars),
    ] {
        if claimed != decoded {
            return Err(format!("occupation {name} {claimed} != decoded {decoded}"));
        }
    }
    Ok(())
}

/// [`parse_trailers`] plus the **one** rule the lane decoder applies to a trailer
/// that the fold does not.
///
/// The two trailer grammars were compared invariant by invariant and they agree
/// everywhere except here: on a `complete` `GAR1`, the lane bounds the controller
/// at [`MAX_TAG_DB_ID`] (`domain44_compact.rs:666`-`:667`) while the fold only
/// rejects a zero (`world_fold.rs:30400`). `COL1`'s two decoders are identical,
/// down to the order of their clauses. So this function is the whole divergence,
/// and it lives beside [`check_authority_payload`] rather than inside
/// [`parse_garrison_alert`] for the same reason the row rules do: the mapper and
/// the fold-facing parser mirror the FOLD, and folding a lane-only bound into
/// them would refuse a trailer the recorder folds.
pub fn check_authority_trailers(trailers: &[u8]) -> core::result::Result<(), String> {
    let parsed = parse_trailers(trailers)?;
    if let Some(alert) = &parsed.garrison_alert {
        if !alert.source_unavailable() && alert.controller_tag_db_id > MAX_TAG_DB_ID {
            return Err(format!(
                "occupation GAR1 controller tag_db_id {} exceeds {MAX_TAG_DB_ID}",
                alert.controller_tag_db_id
            ));
        }
    }
    Ok(())
}

// --- Small helpers -----------------------------------------------------------

/// A gate that is clear on a channel the collector always measures means the
/// value was not captured — see the module docs. §3 calls that **unknown**.
fn gated_fixed(present: bool, row: &[u8], offset: usize) -> Value {
    if present {
        Value::Continuous(i32_at(row, offset) as f64 / FIXED_SCALE)
    } else {
        Value::Unknown
    }
}

/// The garrison block's three-state gate: measured badge = value, unavailable
/// badge = Unknown, neither = Absent. Unlike the speed channels, the third state
/// here is a documented one — the lane says rows carrying neither flag "have no
/// garrison measurement at all (absent)" (`domain44_compact.rs:46`).
fn garrison_fixed(measured: bool, unavailable: bool, row: &[u8], offset: usize) -> Value {
    if measured {
        Value::Continuous(i32_at(row, offset) as f64 / FIXED_SCALE)
    } else if unavailable {
        Value::Unknown
    } else {
        Value::Absent
    }
}

fn need(bytes: &[u8], len: usize, label: &str) -> core::result::Result<(), String> {
    if bytes.len() < len {
        return Err(format!("{label}: {} byte(s) < {len}", bytes.len()));
    }
    Ok(())
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!("occupation row length {} != {ROW_LEN}", row.len()));
    }
    Ok(())
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
    use crate::TickGrid;
    use std::collections::BTreeSet;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection(id: u32) -> Collection {
        schema()
            .collections
            .into_iter()
            .find(|c| c.collection_id == id)
            .expect("collection is in the schema")
    }

    fn states_collection() -> Collection {
        collection(STATES)
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    fn value_of(c: &Collection, r: &Record, id: u32) -> Value {
        r.values[index_of(c, id)]
    }

    /// Every word of a 64-byte state row, in wire order — so a test can move one
    /// word at a time and a transposition shows up as a failure.
    #[derive(Clone, Copy, Debug, Default)]
    struct RawRow {
        state_id: u32,
        owner: u32,
        controller: u32,
        occupied: u32,
        resistance: i32,
        compliance: i32,
        resistance_speed: i32,
        compliance_speed: i32,
        garrison_manpower_need: i32,
        garrison_strength_ratio: i32,
        garrison_required: i32,
        law_token: u32,
        template: u32,
        flags: u32,
        reserved1: u32,
        reserved2: u32,
    }

    impl RawRow {
        fn bytes(self) -> Vec<u8> {
            let mut row = Vec::with_capacity(ROW_LEN);
            row.extend_from_slice(&self.state_id.to_le_bytes());
            row.extend_from_slice(&self.owner.to_le_bytes());
            row.extend_from_slice(&self.controller.to_le_bytes());
            row.extend_from_slice(&self.occupied.to_le_bytes());
            row.extend_from_slice(&self.resistance.to_le_bytes());
            row.extend_from_slice(&self.compliance.to_le_bytes());
            row.extend_from_slice(&self.resistance_speed.to_le_bytes());
            row.extend_from_slice(&self.compliance_speed.to_le_bytes());
            row.extend_from_slice(&self.garrison_manpower_need.to_le_bytes());
            row.extend_from_slice(&self.garrison_strength_ratio.to_le_bytes());
            row.extend_from_slice(&self.garrison_required.to_le_bytes());
            row.extend_from_slice(&self.law_token.to_le_bytes());
            row.extend_from_slice(&self.template.to_le_bytes());
            row.extend_from_slice(&self.flags.to_le_bytes());
            row.extend_from_slice(&self.reserved1.to_le_bytes());
            row.extend_from_slice(&self.reserved2.to_le_bytes());
            assert_eq!(row.len(), ROW_LEN);
            row
        }
    }

    /// The flag word of the most common real row in `runs/batch3/deepwar_a`
    /// (`0xFCFF`, 4,055 of 7,498 rows): every value channel measured, the state
    /// occupied with a derived tag, a law token from the country default, a
    /// garrison template, and measured garrison scalars.
    const REAL_FULL_FLAGS: u32 = ROW_FLAG_OWNER
        | ROW_FLAG_CONTROLLER
        | ROW_FLAG_RESISTANCE
        | ROW_FLAG_COMPLIANCE
        | ROW_FLAG_OCCUPIED_STATE
        | ROW_FLAG_OCCUPIED_TAG_DERIVED
        | ROW_FLAG_OCCUPATION_LAW_TOKEN
        | ROW_FLAG_GARRISON_TEMPLATE
        | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
        | ROW_FLAG_RESISTANCE_SPEED
        | ROW_FLAG_COMPLIANCE_SPEED
        | ROW_FLAG_OCCUPATION_STATUS_ROOT
        | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
        | ROW_FLAG_GARRISON_SCALARS;

    fn full_row() -> RawRow {
        RawRow {
            state_id: 64,
            owner: 3,
            controller: 5,
            occupied: 3,
            resistance: 123_456,
            compliance: 5_000_000,
            resistance_speed: -450,
            compliance_speed: 571,
            garrison_manpower_need: 14_000,
            garrison_strength_ratio: 87_500,
            garrison_required: 250_000,
            law_token: 1_626_184_329,
            template: 2_421,
            flags: REAL_FULL_FLAGS,
            reserved1: 0,
            reserved2: 0,
        }
    }

    fn payload_of(header_tail: [u32; 11], elapsed: u64, rows: &[RawRow]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&elapsed.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        for word in header_tail {
            p.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(p.len(), HEADER_LEN);
        for row in rows {
            p.extend_from_slice(&row.bytes());
        }
        p
    }

    /// A well-formed payload whose header counters agree with its rows, so it
    /// passes [`check_authority_payload`] as well as the fold.
    fn payload(status_flags: u32, rows: &[RawRow]) -> Vec<u8> {
        let count = |mask: u32| rows.iter().filter(|r| r.flags & mask != 0).count() as u32;
        payload_of(
            [
                rows.len() as u32, // changed_rows
                status_flags,
                rows.len().max(1) as u32, // observed_state_rows
                count(ROW_FLAG_OCCUPIED_STATE),
                count(
                    ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
                        | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                        | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE,
                ),
                FIXED_SCALE_RAW,
                count(ROW_FLAG_OCCUPATION_LAW_TOKEN),
                count(ROW_FLAG_GARRISON_TEMPLATE),
                count(ROW_FLAG_GARRISON_SCALARS),
                0, // garrison_anomaly_rows
                // invalid_state_rows: the helper's `observed_state_rows` is
                // `rows.len().max(1)`, so a ROWLESS payload declares one root and
                // must account for it as a drop or the full-frame identity
                // `row_count + invalid == observed` fails.
                if rows.is_empty() { 1 } else { 0 },
            ],
            7,
            rows,
        )
    }

    /// The `GAR1` trailer, built the way the lane encodes one
    /// (`domain44_compact.rs:181`-`:197`).
    fn gar1(
        game_hour: i32,
        controller: u32,
        flags: u32,
        missing_manpower: i64,
        equipment: &[(u32, i64)],
    ) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(GARRISON_ALERT_MAGIC);
        t.extend_from_slice(&GARRISON_ALERT_SCHEMA_VERSION.to_le_bytes());
        t.extend_from_slice(&(GARRISON_ALERT_HEADER_LEN as u16).to_le_bytes());
        t.extend_from_slice(&game_hour.to_le_bytes());
        t.extend_from_slice(&controller.to_le_bytes());
        t.extend_from_slice(&flags.to_le_bytes());
        t.extend_from_slice(&missing_manpower.to_le_bytes());
        t.extend_from_slice(&(equipment.len() as u32).to_le_bytes());
        t.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(t.len(), GARRISON_ALERT_HEADER_LEN);
        for (archetype, units) in equipment {
            t.extend_from_slice(&archetype.to_le_bytes());
            t.extend_from_slice(&units.to_le_bytes());
            t.extend_from_slice(&0u32.to_le_bytes());
        }
        t
    }

    /// The `GAR1` trailer every hour of the measured capture carries: source
    /// unavailable, no controller, no equipment.
    fn gar1_unavailable(game_hour: i32) -> Vec<u8> {
        gar1(game_hour, 0, GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE, 0, &[])
    }

    /// A complete `GAR1`: the path the measured capture never exercises.
    fn gar1_complete(game_hour: i32, equipment: &[(u32, i64)], missing_manpower: i64) -> Vec<u8> {
        let mut flags = GARRISON_ALERT_FLAG_COMPLETE;
        if missing_manpower > 0 {
            flags |= GARRISON_ALERT_FLAG_MISSING_MANPOWER;
        }
        // `complete && predicate_returned == (missing_manpower > 0 || equipment)`
        // is a rejected header, so the bit is the negation of that disjunction.
        if !(missing_manpower > 0 || !equipment.is_empty()) {
            flags |= GARRISON_ALERT_FLAG_PREDICATE_RETURNED;
        }
        gar1(game_hour, 7, flags, missing_manpower, equipment)
    }

    fn col1(
        flags: u32,
        observed_country_rows: u32,
        anomaly_rows: u32,
        rows: &[(u32, u32, i64)],
    ) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(COLLABORATION_MAGIC);
        t.extend_from_slice(&COLLABORATION_SCHEMA_VERSION.to_le_bytes());
        t.extend_from_slice(&(COLLABORATION_HEADER_LEN as u16).to_le_bytes());
        t.extend_from_slice(&flags.to_le_bytes());
        t.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        t.extend_from_slice(&observed_country_rows.to_le_bytes());
        t.extend_from_slice(&anomaly_rows.to_le_bytes());
        t.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(t.len(), COLLABORATION_HEADER_LEN);
        for (owner, target, progress) in rows {
            t.extend_from_slice(&owner.to_le_bytes());
            t.extend_from_slice(&target.to_le_bytes());
            t.extend_from_slice(&progress.to_le_bytes());
        }
        t
    }

    // --- Step 8.1: the schema is well-formed ---------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d52 schema is well-formed");
        // And on the hourly grid this domain actually runs on.
        schema().validate(TickGrid::HOURLY.log2()).expect("hourly");
    }

    #[test]
    fn the_delta_bit_matches_the_shipping_contract() {
        // d52 is not admitted to the production-codec dependency surface, so
        // the bit is named here and pinned against the contract's own value,
        // which the measurement bin asserts too.
        assert_eq!(STATUS_DELTA_DATA, 1 << 10);
    }

    // --- Step 8.5: shape and key ---------------------------------------------

    #[test]
    fn every_collection_is_a_keyed_set_with_the_folds_own_key() {
        let s = schema();
        assert_eq!(s.domain, DOMAIN);
        assert_eq!(s.cadence_log2, 0, "one DATA record per game hour");
        assert_eq!(s.collections.len(), 5);
        assert!(s.event_part.is_none(), "d52 carries no happened-at-tick data");
        for c in &s.collections {
            assert_eq!(
                c.shape,
                Shape::KeyedSet,
                "collection {} must have a delta grammar",
                c.collection_id
            );
            assert!(!c.key_fields().is_empty());
            for k in c.key_fields() {
                assert!(k.kind.legal_as_key(), "field {}", k.field_id);
                assert!(
                    !k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none(),
                    "key field {} must be concrete",
                    k.field_id
                );
            }
        }

        let states = states_collection();
        let keys = states.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys states by state_id alone");
        assert_eq!(keys[0].field_id, field::STATE_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "§5 names state ids as content-defined"
        );
        assert_eq!(states.fields.len(), 16, "1 key + 15 value fields");

        let relations = collection(COLLABORATION_RELATIONS);
        let relation_keys = relations.key_fields();
        assert_eq!(relation_keys.len(), 2, "a relation is (owner, target)");
        assert_eq!(relation_keys[0].field_id, field::RELATION_OWNER_TAG_DB_ID);
        assert_eq!(relation_keys[1].field_id, field::RELATION_TARGET_TAG_DB_ID);

        for id in [GARRISON_ALERT, COLLABORATION_STATUS] {
            let c = collection(id);
            assert_eq!(c.key_fields().len(), 1);
            assert_eq!(
                c.key_fields()[0].kind,
                Kind::Unsigned,
                "the singleton key names nothing content-defined"
            );
        }
        assert_eq!(alert_singleton_key(), key(&[SINGLETON_KEY_COMPONENT]));
        assert_eq!(
            collaboration_singleton_key(),
            key(&[SINGLETON_KEY_COMPONENT])
        );
    }

    #[test]
    fn field_ids_are_unique_across_the_whole_domain() {
        // §11 needs uniqueness per collection; this domain goes further so a
        // record routed to the wrong collection cannot land on a same-numbered
        // field and be silently accepted.
        let mut seen: Vec<u32> = Vec::new();
        for c in &schema().collections {
            for f in &c.fields {
                assert!(!seen.contains(&f.field_id), "field id {} reused", f.field_id);
                seen.push(f.field_id);
            }
        }
        assert_eq!(seen.len(), 16 + 6 + 2 + 4 + 3);
    }

    #[test]
    fn every_content_id_field_is_listed_for_the_manifest_retrofit() {
        // §13's manifest-id assignment rule will change every mapper that emits a
        // ContentId, so the retrofit sweep needs this to be an assertion rather
        // than a claim — and it needs the KEY components called out separately,
        // because a remap there reorders keys and rewrites every baseline's bytes.
        let mut value_ids: BTreeSet<u32> = BTreeSet::new();
        let mut key_ids: BTreeSet<u32> = BTreeSet::new();
        for c in &schema().collections {
            for f in &c.fields {
                if f.kind == Kind::ContentId {
                    if f.is_key() {
                        key_ids.insert(f.field_id);
                    } else {
                        value_ids.insert(f.field_id);
                    }
                }
            }
        }
        assert_eq!(
            key_ids,
            BTreeSet::from([
                field::STATE_ID,
                field::EQUIPMENT_ARCHETYPE_ID,
                field::RELATION_OWNER_TAG_DB_ID,
                field::RELATION_TARGET_TAG_DB_ID,
            ]),
            "ContentId KEY components — the heavier retrofit class"
        );
        assert_eq!(
            value_ids,
            BTreeSet::from([
                field::OCCUPIER_TAG,
                field::OCCUPIED_COUNTRY_TAG,
                field::OCCUPATION_LAW_TOKEN_ID,
                field::ALERT_CONTROLLER_TAG_DB_ID,
            ]),
            "ContentId value fields"
        );
        // The law token is in that list on purpose: it is an FNV-1a-32 hash of a
        // content-defined law NAME, and §5 lists law tokens among the
        // content-defined vocabularies. No manifest can invert a hash today,
        // which is a gap in the manifest rather than a statement about the kind —
        // and putting it in the sweep is what makes that gap findable.
        // The two id-like fields that are deliberately NOT ContentId:
        for id in [field::GARRISON_TEMPLATE_ID, field::ALERT_SINGLETON] {
            let f = schema()
                .collections
                .iter()
                .flat_map(|c| c.fields.iter())
                .find(|f| f.field_id == id)
                .expect("present")
                .clone();
            assert_ne!(f.kind, Kind::ContentId, "field {id}");
        }
    }

    // --- Step 8.6: every field's offset, with scale as the special case ------

    #[test]
    fn the_scales_are_per_field_not_per_row() {
        // The test that catches a wrong divisor: known fixed-point words at known
        // offsets against their exact readings. `garrison_manpower_need` sits
        // BETWEEN two /100000 words and has no divisor at all, so a row-wide scale
        // shows up here as a 100,000x error rather than as a round-trip failure.
        let c = states_collection();
        let row = RawRow {
            resistance: 100_000,       // 1.0
            compliance: 50_000,        // 0.5
            resistance_speed: -25_000, // -0.25
            compliance_speed: 1,       // 1e-5
            garrison_manpower_need: 24_665,
            garrison_strength_ratio: 87_500, // 0.875
            garrison_required: -100_000,     // -1.0
            ..full_row()
        };
        let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
        assert_eq!(value_of(&c, &r, field::RESISTANCE), Value::Continuous(1.0));
        assert_eq!(value_of(&c, &r, field::COMPLIANCE), Value::Continuous(0.5));
        assert_eq!(
            value_of(&c, &r, field::RESISTANCE_SPEED),
            Value::Continuous(-0.25)
        );
        assert_eq!(
            value_of(&c, &r, field::COMPLIANCE_SPEED),
            Value::Continuous(1.0 / 100_000.0)
        );
        assert_eq!(
            value_of(&c, &r, field::GARRISON_ARMY_MANPOWER_NEED),
            Value::Signed(24_665),
            "a plain manpower count, emitted raw by the fold (world_fold.rs:16960)"
        );
        assert_eq!(
            value_of(&c, &r, field::GARRISON_STRENGTH_RATIO),
            Value::Continuous(0.875)
        );
        assert_eq!(
            value_of(&c, &r, field::GARRISON_REQUIRED),
            Value::Continuous(-1.0)
        );
        // The unscaled id-like words, each at its own offset.
        assert_eq!(
            value_of(&c, &r, field::OCCUPATION_LAW_TOKEN_ID),
            Value::ContentId(1_626_184_329)
        );
        assert_eq!(
            value_of(&c, &r, field::GARRISON_TEMPLATE_ID),
            Value::Unsigned(2_421)
        );
        // The three tag words at +4/+8/+12: `occupier_tag` is the CONTROLLER and
        // `occupied_country_tag` the derived word, not the other way round.
        assert_eq!(value_of(&c, &r, field::OCCUPIER_TAG), Value::ContentId(5));
        assert_eq!(
            value_of(&c, &r, field::OCCUPIED_COUNTRY_TAG),
            Value::ContentId(3)
        );
        // And the key is the word at +0, not any of them.
        assert_eq!(key_from_row(&row.bytes()).expect("keys"), key(&[64]));
    }

    #[test]
    fn every_header_word_reads_at_its_own_offset() {
        // A transposed header word mis-slices every row behind it and no
        // round-trip or replay assertion can see it — both arms agree on the
        // wrong slice. So every word `peek_header` reads gets a known value.
        let rows = [full_row()];
        let p = payload_of([11, 22, 33, 44, 55, FIXED_SCALE_RAW, 77, 88, 99, 111, 122], 9, &rows);
        let h = peek_header(&p).expect("header");
        assert_eq!(h.elapsed_wall_ms, 9);
        assert_eq!(h.row_count, 1);
        assert_eq!(h.changed_rows, 11);
        assert_eq!(h.status_flags, 22);
        assert_eq!(h.observed_state_rows, 33);
        assert_eq!(h.occupied_state_rows, 44);
        assert_eq!(h.wall_badge_rows, 55);
        assert_eq!(h.scale_raw, FIXED_SCALE_RAW);
        assert_eq!(h.law_token_rows, 77);
        assert_eq!(h.garrison_template_rows, 88);
        assert_eq!(h.garrison_scalar_rows, 99);
        assert_eq!(h.garrison_anomaly_rows, 111);
        assert_eq!(h.invalid_state_rows, 122);
        // The delta bit is read from +16 and nowhere else.
        assert!(!h.is_delta());
        let delta = payload_of(
            [0, STATUS_DELTA_DATA, 1, 0, 0, FIXED_SCALE_RAW, 0, 0, 0, 0, 0],
            0,
            &rows,
        );
        assert!(peek_header(&delta).expect("header").is_delta());
    }

    #[test]
    fn every_gar1_and_col1_header_word_reads_at_its_own_offset() {
        let alert = parse_garrison_alert(&gar1_complete(-7, &[(1_234, 9)], 250))
            .expect("well-formed GAR1");
        assert_eq!(alert.game_hour_raw, -7);
        assert_eq!(alert.controller_tag_db_id, 7);
        assert_eq!(alert.missing_manpower, 250);
        assert!(!alert.source_unavailable());
        assert!(!alert.predicate_returned());
        assert_eq!(
            alert.equipment,
            vec![AlertEquipmentRow {
                archetype_id: 1_234,
                whole_units: 9
            }]
        );

        let collaboration = parse_collaboration(&col1(
            COLLABORATION_FLAG_COMPLETE,
            436,
            0,
            &[(24, 1, 30_000)],
        ))
        .expect("well-formed COL1");
        assert_eq!(collaboration.observed_country_rows, 436);
        assert_eq!(collaboration.anomaly_rows, 0);
        assert!(!collaboration.source_unavailable());
        assert_eq!(
            collaboration.rows,
            vec![CollaborationRelationRow {
                owner_tag_db_id: 24,
                target_tag_db_id: 1,
                progress_raw: 30_000
            }]
        );
    }
    // --- Step 8.2: byte-identical round trips --------------------------------

    fn roundtrips(c: &Collection, record: &Record) {
        let mut bytes = Vec::new();
        record.encode(c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(&back, record, "decode reproduces the record");
        let mut reencoded = Vec::new();
        back.encode(c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn a_full_row_of_every_collection_roundtrips_byte_for_byte() {
        let states = states_collection();
        roundtrips(
            &states,
            &record_from_row(&states, &full_row().bytes(), 0).expect("maps"),
        );

        let alert_c = collection(GARRISON_ALERT);
        let alert = parse_garrison_alert(&gar1_complete(5, &[(11, 2)], 300)).expect("GAR1");
        roundtrips(
            &alert_c,
            &record_from_garrison_alert(&alert_c, &alert, 0).expect("maps"),
        );
        let equipment_c = collection(GARRISON_ALERT_EQUIPMENT);
        roundtrips(
            &equipment_c,
            &record_from_equipment_row(&equipment_c, &alert.equipment[0], 0).expect("maps"),
        );

        let status_c = collection(COLLABORATION_STATUS);
        let collaboration =
            parse_collaboration(&col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, 30_000)]))
                .expect("COL1");
        roundtrips(
            &status_c,
            &record_from_collaboration_status(&status_c, &collaboration, 0).expect("maps"),
        );
        let relations_c = collection(COLLABORATION_RELATIONS);
        roundtrips(
            &relations_c,
            &record_from_collaboration_row(&relations_c, &collaboration.rows[0], 0).expect("maps"),
        );
    }

    #[test]
    fn every_empty_state_survives_a_roundtrip() {
        // Absent, Null and Unknown all in one record, so the presence and null
        // bitmaps are exercised together rather than one at a time.
        let states = states_collection();
        let row = RawRow {
            state_id: 9,
            owner: 0,
            controller: 0,
            occupied: 0,
            resistance: 0,
            compliance: 0,
            resistance_speed: 0,
            compliance_speed: 0,
            garrison_manpower_need: 0,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            law_token: 0,
            template: 0,
            // Occupied with no tags at all (both tag fields Null), no garrison
            // badge (Absent), no speed gate and no law token (Unknown).
            flags: ROW_FLAG_OCCUPIED_STATE,
            reserved1: 0,
            reserved2: 0,
        };
        let record = record_from_row(&states, &row.bytes(), 0).expect("maps");
        assert_eq!(value_of(&states, &record, field::RESISTANCE), Value::Unknown);
        assert_eq!(
            value_of(&states, &record, field::RESISTANCE_SPEED),
            Value::Unknown
        );
        assert_eq!(
            value_of(&states, &record, field::GARRISON_STRENGTH_RATIO),
            Value::Absent,
            "the garrison block is the one that still reaches Absent"
        );
        assert_eq!(value_of(&states, &record, field::OCCUPIER_TAG), Value::Null);
        assert_eq!(
            value_of(&states, &record, field::OCCUPATION_LAW_TOKEN_ID),
            Value::Unknown
        );
        roundtrips(&states, &record);
    }

    // --- Step 8.3: the four states, through the mapper, from raw bytes -------

    #[test]
    fn resistance_and_compliance_are_value_or_unknown_and_never_anything_else() {
        // The collector sets both gates unconditionally
        // (`collector_module.rs:1079`-`:1080`) and drops the whole row when the
        // read fails, so a clear gate is uncaptured data. The schema therefore
        // declares `unknown_capable` and nothing else for these two, and the
        // mapper must not be able to produce Absent or Null for them.
        let c = states_collection();
        for f in c.value_fields() {
            if f.field_id == field::RESISTANCE || f.field_id == field::COMPLIANCE {
                assert!(f.unknown_capable, "field {}", f.field_id);
                assert!(!f.optional && !f.nullable, "field {}", f.field_id);
            }
        }
        let base = RawRow {
            flags: REAL_FULL_FLAGS,
            ..full_row()
        };
        for (cleared, id, other_id) in [
            (ROW_FLAG_RESISTANCE, field::RESISTANCE, field::COMPLIANCE),
            (ROW_FLAG_COMPLIANCE, field::COMPLIANCE, field::RESISTANCE),
        ] {
            let mut row = base;
            row.flags &= !cleared;
            // A gated-off channel packs a zero in every payload the lane accepts.
            if cleared == ROW_FLAG_RESISTANCE {
                row.resistance = 0;
            } else {
                row.compliance = 0;
            }
            let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
            assert_eq!(value_of(&c, &r, id), Value::Unknown, "field {id}");
            assert!(
                value_of(&c, &r, other_id).carries_value(),
                "only the cleared gate goes Unknown"
            );
        }
    }

    #[test]
    fn the_two_speeds_are_value_or_unknown_and_the_badge_rides_beside_them() {
        let c = states_collection();
        // Value.
        let r = record_from_row(&c, &full_row().bytes(), 0).expect("maps");
        assert_eq!(
            value_of(&c, &r, field::RESISTANCE_SPEED),
            Value::Continuous(-0.0045)
        );
        assert_eq!(
            value_of(&c, &r, field::COMPLIANCE_SPEED),
            Value::Continuous(0.00571)
        );
        assert_eq!(
            value_of(&c, &r, field::RESISTANCE_SPEED_UNAVAILABLE),
            Value::Bool(false)
        );
        // A clear gate is Unknown WHATEVER the badge says, because the collector
        // reads both speeds for every row it emits — the same argument as
        // resistance/compliance, from the same lines of the same read. The badge
        // is carried beside them so the fold's `resistance_speed_status` key is
        // not lost, and it is the only thing that changes between these two rows.
        let bare = RawRow {
            resistance_speed: 0,
            compliance_speed: 0,
            flags: REAL_FULL_FLAGS & !(ROW_FLAG_RESISTANCE_SPEED | ROW_FLAG_COMPLIANCE_SPEED),
            ..full_row()
        };
        let badged = RawRow {
            flags: bare.flags | ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE,
            ..bare
        };
        for (row, badge) in [(bare, false), (badged, true)] {
            let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
            assert_eq!(value_of(&c, &r, field::RESISTANCE_SPEED), Value::Unknown);
            assert_eq!(value_of(&c, &r, field::COMPLIANCE_SPEED), Value::Unknown);
            assert_eq!(
                value_of(&c, &r, field::RESISTANCE_SPEED_UNAVAILABLE),
                Value::Bool(badge),
                "the badge is carried, not collapsed into the value's empty state"
            );
        }
        // Neither speed field can ever be Absent, so the schema must not declare
        // it — a declared capability nothing emits costs a presence bit per row.
        for id in [field::RESISTANCE_SPEED, field::COMPLIANCE_SPEED] {
            let f = c.fields.iter().find(|f| f.field_id == id).expect("present");
            assert!(!f.optional && !f.nullable && f.unknown_capable, "field {id}");
        }
        // And the two are independent when only one gate is clear.
        let one = RawRow {
            resistance_speed: 0,
            flags: REAL_FULL_FLAGS & !ROW_FLAG_RESISTANCE_SPEED,
            ..full_row()
        };
        let r = record_from_row(&c, &one.bytes(), 0).expect("maps");
        assert_eq!(value_of(&c, &r, field::RESISTANCE_SPEED), Value::Unknown);
        assert_eq!(
            value_of(&c, &r, field::COMPLIANCE_SPEED),
            Value::Continuous(0.00571)
        );
    }

    #[test]
    fn the_garrison_block_is_a_three_state_gate_over_all_three_scalars() {
        let c = states_collection();
        let ids = [
            field::GARRISON_ARMY_MANPOWER_NEED,
            field::GARRISON_STRENGTH_RATIO,
            field::GARRISON_REQUIRED,
        ];
        // Badge `GARRISON_SCALARS`: real values, including a real zero.
        let measured = RawRow {
            garrison_manpower_need: 0,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            ..full_row()
        };
        let r = record_from_row(&c, &measured.bytes(), 0).expect("maps");
        assert_eq!(
            value_of(&c, &r, field::GARRISON_ARMY_MANPOWER_NEED),
            Value::Signed(0),
            "a measured zero is a value, never an empty state"
        );
        assert_eq!(
            value_of(&c, &r, field::GARRISON_STRENGTH_RATIO),
            Value::Continuous(0.0)
        );
        assert_eq!(
            value_of(&c, &r, field::GARRISON_REQUIRED),
            Value::Continuous(0.0)
        );
        // Badge `GARRISON_SIGNALS_UNAVAILABLE`: Unknown.
        let unavailable = RawRow {
            garrison_manpower_need: 0,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            flags: (REAL_FULL_FLAGS & !ROW_FLAG_GARRISON_SCALARS)
                | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE,
            ..full_row()
        };
        let r = record_from_row(&c, &unavailable.bytes(), 0).expect("maps");
        for id in ids {
            assert_eq!(value_of(&c, &r, id), Value::Unknown, "field {id}");
        }
        // Neither badge and all three words zero: Absent — the lane's own words,
        // "rows carrying neither flag have no garrison measurement at all".
        let absent = RawRow {
            garrison_manpower_need: 0,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            flags: REAL_FULL_FLAGS & !ROW_FLAG_GARRISON_SCALARS,
            ..full_row()
        };
        let r = record_from_row(&c, &absent.bytes(), 0).expect("maps");
        for id in ids {
            assert_eq!(value_of(&c, &r, id), Value::Absent, "field {id}");
        }
        // Both badges: the fold errors (`world_fold.rs::fold_occupation_resistance_compliance`).
        let both = RawRow {
            flags: REAL_FULL_FLAGS | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE,
            ..full_row()
        };
        assert!(record_from_row(&c, &both.bytes(), 0).is_err());
        // Values under neither badge: the fold errors (`:16974`-`:16983`), each
        // of the three words independently.
        for word in 0..3usize {
            let mut row = absent;
            match word {
                0 => row.garrison_manpower_need = 1,
                1 => row.garrison_strength_ratio = 1,
                _ => row.garrison_required = 1,
            }
            assert!(
                record_from_row(&c, &row.bytes(), 0).is_err(),
                "garrison word {word} without a measured badge"
            );
        }
        // The scalars are `Signed` for the manpower count because the FOLD makes
        // no sign check: a negative word must map to the number it is.
        let negative = RawRow {
            garrison_manpower_need: -5,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            ..full_row()
        };
        let r = record_from_row(&c, &negative.bytes(), 0).expect("the fold accepts it");
        assert_eq!(
            value_of(&c, &r, field::GARRISON_ARMY_MANPOWER_NEED),
            Value::Signed(-5)
        );
    }

    #[test]
    fn the_two_tags_are_absent_off_the_occupied_gate_and_null_inside_it() {
        let c = states_collection();
        // Not occupied: both concepts are inapplicable.
        let unoccupied = RawRow {
            occupied: 0,
            garrison_manpower_need: 0,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            template: 0,
            flags: REAL_FULL_FLAGS
                & !(ROW_FLAG_OCCUPIED_STATE
                    | ROW_FLAG_OCCUPIED_TAG_DERIVED
                    | ROW_FLAG_GARRISON_SCALARS
                    | ROW_FLAG_GARRISON_TEMPLATE),
            ..full_row()
        };
        let r = record_from_row(&c, &unoccupied.bytes(), 0).expect("maps");
        assert_eq!(value_of(&c, &r, field::OCCUPIER_TAG), Value::Absent);
        assert_eq!(value_of(&c, &r, field::OCCUPIED_COUNTRY_TAG), Value::Absent);
        assert_eq!(
            value_of(&c, &r, field::GARRISON_TEMPLATE_ID),
            Value::Absent,
            "a template is only read under the occupied gate"
        );

        // Occupied, derived tag clear, owner set: the fold's fallback to the
        // owner word (`world_fold.rs::fold_occupation_resistance_compliance`) — a derived value.
        let fallback = RawRow {
            occupied: 0,
            flags: REAL_FULL_FLAGS & !ROW_FLAG_OCCUPIED_TAG_DERIVED,
            ..full_row()
        };
        let r = record_from_row(&c, &fallback.bytes(), 0).expect("maps");
        assert_eq!(
            value_of(&c, &r, field::OCCUPIED_COUNTRY_TAG),
            Value::ContentId(3),
            "falls back to the OWNER word at +4, not the zeroed word at +12"
        );

        // Occupied with neither owner nor derived tag: the relation points at
        // nothing, which is §3's null.
        let null_tags = RawRow {
            owner: 0,
            controller: 0,
            occupied: 0,
            garrison_manpower_need: 0,
            garrison_strength_ratio: 0,
            garrison_required: 0,
            flags: REAL_FULL_FLAGS
                & !(ROW_FLAG_OWNER
                    | ROW_FLAG_CONTROLLER
                    | ROW_FLAG_OCCUPIED_TAG_DERIVED
                    | ROW_FLAG_GARRISON_SCALARS),
            ..full_row()
        };
        let r = record_from_row(&c, &null_tags.bytes(), 0).expect("maps");
        assert_eq!(value_of(&c, &r, field::OCCUPIER_TAG), Value::Null);
        assert_eq!(value_of(&c, &r, field::OCCUPIED_COUNTRY_TAG), Value::Null);
    }

    #[test]
    fn the_law_token_is_unknown_on_a_clear_flag_and_the_badge_is_not_its_gate() {
        // The trap this test exists for: `OCCUPATION_LAW_UNAVAILABLE` is the
        // obvious neighbour and it is the WRONG one — the collector sets it for
        // every occupied state and never clears it when the token IS found
        // (`collector_module.rs:1097`-`:1099`). The capture agrees: 6,800 rows
        // carry the badge while all 7,498 carry a token.
        let c = states_collection();
        let with_badge_and_token = full_row();
        assert!(with_badge_and_token.flags & ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE != 0);
        assert!(with_badge_and_token.flags & ROW_FLAG_OCCUPATION_LAW_TOKEN != 0);
        let r = record_from_row(&c, &with_badge_and_token.bytes(), 0).expect("maps");
        assert_eq!(
            value_of(&c, &r, field::OCCUPATION_LAW_TOKEN_ID),
            Value::ContentId(1_626_184_329),
            "the badge must not turn a carried token into an empty state"
        );
        assert_eq!(
            value_of(&c, &r, field::OCCUPATION_LAW_UNAVAILABLE),
            Value::Bool(true),
            "and the badge itself is still carried, as its own field"
        );

        // A clear token flag is Unknown WITH the status root and WITHOUT it. The
        // status root is the nearer sibling, but it does not rescue a Null:
        // inside a valid root the token still reaches zero along three collapsed
        // read-failure paths (`collector_module.rs:536`-`:556`), so "no default
        // law" and "could not read it" are indistinguishable.
        let no_default = RawRow {
            law_token: 0,
            flags: REAL_FULL_FLAGS
                & !(ROW_FLAG_OCCUPATION_LAW_TOKEN | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN),
            ..full_row()
        };
        let no_root = RawRow {
            template: 0,
            flags: no_default.flags
                & !(ROW_FLAG_GARRISON_TEMPLATE | ROW_FLAG_OCCUPATION_STATUS_ROOT),
            ..no_default
        };
        for (label, row) in [("status root set", no_default), ("status root clear", no_root)] {
            let r = record_from_row(&c, &row.bytes(), 0).expect("maps");
            assert_eq!(
                value_of(&c, &r, field::OCCUPATION_LAW_TOKEN_ID),
                Value::Unknown,
                "{label}"
            );
            assert_eq!(
                value_of(&c, &r, field::OCCUPATION_LAW_TOKEN_IS_COUNTRY_DEFAULT),
                Value::Bool(false),
                "{label}"
            );
        }
        // The token can never be Null, so the schema must not declare it.
        let f = c
            .fields
            .iter()
            .find(|f| f.field_id == field::OCCUPATION_LAW_TOKEN_ID)
            .expect("present");
        assert!(!f.optional && !f.nullable && f.unknown_capable);
        // The template shares the Unknown inside the occupied gate.
        let r = record_from_row(&c, &no_root.bytes(), 0).expect("maps");
        assert_eq!(
            value_of(&c, &r, field::GARRISON_TEMPLATE_ID),
            Value::Unknown
        );
        assert_eq!(
            value_of(&c, &r, field::OCCUPATION_STATUS_ROOT),
            Value::Bool(false)
        );
    }

    #[test]
    fn the_garrison_template_is_a_value_unknown_or_absent() {
        let c = states_collection();
        // Value.
        let r = record_from_row(&c, &full_row().bytes(), 0).expect("maps");
        assert_eq!(
            value_of(&c, &r, field::GARRISON_TEMPLATE_ID),
            Value::Unsigned(2_421)
        );
        // Unknown inside the occupied gate, whatever the status root says: the
        // zero comes through the same `unwrap_or(0)` collapse as the law token.
        let cleared = RawRow {
            template: 0,
            flags: REAL_FULL_FLAGS & !ROW_FLAG_GARRISON_TEMPLATE,
            ..full_row()
        };
        assert_eq!(
            value_of(
                &c,
                &record_from_row(&c, &cleared.bytes(), 0).expect("maps"),
                field::GARRISON_TEMPLATE_ID
            ),
            Value::Unknown
        );
        // Absent off the occupied gate is covered by
        // `the_two_tags_are_absent_off_the_occupied_gate_and_null_inside_it`.
        // The schema declares exactly the two capabilities those three states
        // need, and never `nullable`.
        let f = c
            .fields
            .iter()
            .find(|f| f.field_id == field::GARRISON_TEMPLATE_ID)
            .expect("present");
        assert!(f.optional && !f.nullable && f.unknown_capable);
    }

    #[test]
    fn the_schema_declares_exactly_the_states_the_mappers_can_emit() {
        // Every declared capability costs real bits, and a state the mapper
        // cannot reach must not be declared. This is the table, field by field.
        let states = states_collection();
        let expect = |id: u32, optional: bool, nullable: bool, unknown: bool| {
            let f = states
                .fields
                .iter()
                .find(|f| f.field_id == id)
                .unwrap_or_else(|| panic!("field {id}"));
            assert_eq!((f.optional, f.nullable, f.unknown_capable), (optional, nullable, unknown), "field {id}");
        };
        expect(field::RESISTANCE, false, false, true);
        expect(field::COMPLIANCE, false, false, true);
        expect(field::RESISTANCE_SPEED, false, false, true);
        expect(field::COMPLIANCE_SPEED, false, false, true);
        expect(field::OCCUPIER_TAG, true, true, false);
        expect(field::OCCUPIED_COUNTRY_TAG, true, true, false);
        expect(field::GARRISON_ARMY_MANPOWER_NEED, true, false, true);
        expect(field::GARRISON_STRENGTH_RATIO, true, false, true);
        expect(field::GARRISON_REQUIRED, true, false, true);
        expect(field::OCCUPATION_LAW_TOKEN_ID, false, false, true);
        expect(field::OCCUPATION_LAW_TOKEN_IS_COUNTRY_DEFAULT, false, false, false);
        expect(field::OCCUPATION_LAW_UNAVAILABLE, false, false, false);
        expect(field::GARRISON_TEMPLATE_ID, true, false, true);
        expect(field::OCCUPATION_STATUS_ROOT, false, false, false);
        expect(field::RESISTANCE_SPEED_UNAVAILABLE, false, false, false);
        assert_eq!(states.optional_count(), 6);
        assert!(
            states.value_fields().filter(|f| f.nullable).count() == 2,
            "only the two occupation tags can point at nothing"
        );

        // The trailer headers: the words their invariants hard-zero on the
        // unavailable arm are unknown-capable, and nothing anywhere is nullable
        // or optional — a trailer field is measured or it was not captured.
        let alert = collection(GARRISON_ALERT);
        assert_eq!(alert.optional_count(), 0);
        assert!(alert.value_fields().all(|f| !f.nullable));
        let alert_unknown: Vec<u32> = alert
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            alert_unknown,
            vec![
                field::ALERT_CONTROLLER_TAG_DB_ID,
                field::ALERT_PREDICATE_RETURNED,
                field::ALERT_MISSING_MANPOWER,
            ]
        );
        let status = collection(COLLABORATION_STATUS);
        assert_eq!(status.optional_count(), 0);
        assert!(status.value_fields().all(|f| !f.nullable));
        let status_unknown: Vec<u32> = status
            .value_fields()
            .filter(|f| f.unknown_capable)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(
            status_unknown,
            vec![field::COLLABORATION_OBSERVED_COUNTRY_ROWS],
            "anomaly_rows is unconstrained on the unavailable arm, so it is measured on both"
        );
        // GARRISON_ALERT_EQUIPMENT and COLLABORATION_RELATIONS declare NO
        // capability at all: every row of both is a complete measurement, and a
        // row that is not is a rejected trailer rather than an emptied field
        // (`world_fold.rs:30419`, `:30345`). That is why neither has a raw-bytes
        // case for Absent, Null or Unknown — there is none to build.
        for id in [GARRISON_ALERT_EQUIPMENT, COLLABORATION_RELATIONS] {
            let c = collection(id);
            assert_eq!(c.optional_count(), 0, "collection {id}");
            assert!(
                c.value_fields().all(|f| !f.nullable && !f.unknown_capable),
                "collection {id}"
            );
        }
    }

    #[test]
    fn the_two_trailer_singletons_are_absent_present_or_unavailable() {
        // The three states of each trailer, which the FOLD conflates: it renders
        // "no trailer" and "trailer says unavailable" both as JSON null
        // (`world_fold.rs:6014`-`:6019`, `:30407`). The singleton row's presence
        // separates them, which is a fidelity gain §3 asks for.
        let s = schema();
        let alert_c = collection(GARRISON_ALERT);
        let unavailable = parse_garrison_alert(&gar1_unavailable(1_234)).expect("GAR1");
        let r = record_from_garrison_alert(&alert_c, &unavailable, 0).expect("maps");
        assert_eq!(
            value_of(&alert_c, &r, field::ALERT_SOURCE_UNAVAILABLE),
            Value::Bool(true)
        );
        for id in [
            field::ALERT_CONTROLLER_TAG_DB_ID,
            field::ALERT_PREDICATE_RETURNED,
            field::ALERT_MISSING_MANPOWER,
        ] {
            assert_eq!(
                value_of(&alert_c, &r, id),
                Value::Unknown,
                "field {id}: the header invariants hard-zero it because nothing \
                 was read, so a 0/false here would be a fabricated measurement"
            );
        }
        assert_eq!(
            value_of(&alert_c, &r, field::ALERT_GAME_HOUR_RAW),
            Value::Signed(1_234),
            "the observation hour is carried even on the unavailable arm, which \
             the fold drops entirely"
        );
        roundtrips(&alert_c, &r);

        // Absent trailer: the singleton row simply is not there.
        let mut state = State::new();
        state
            .fold_payload(&s, &payload(0, &[full_row()]))
            .expect("folds");
        assert!(state.garrison_alert.is_empty(), "no GAR1 in this payload");
        assert!(state.collaboration_status.is_empty(), "no COL1 either");

        let mut with_trailer = payload(0, &[full_row()]);
        with_trailer.extend_from_slice(&gar1_unavailable(1_234));
        let mut state = State::new();
        state.fold_payload(&s, &with_trailer).expect("folds");
        assert_eq!(state.garrison_alert.len(), 1);
        assert!(state.collaboration_status.is_empty());
    }

    // --- Step 8.4: quantization actually saves bytes -------------------------

    #[test]
    fn fixed_five_saves_bytes_against_an_all_exact_variant() {
        let quantized = states_collection();
        let mut exact = quantized.clone();
        for f in &mut exact.fields {
            if f.kind == Kind::Continuous {
                f.quant = Quant::Exact;
            }
        }
        let row = full_row().bytes();
        let mut a = Vec::new();
        record_from_row(&quantized, &row, 0)
            .expect("maps")
            .encode(&quantized, &mut a)
            .expect("encodes");
        let mut b = Vec::new();
        record_from_row(&exact, &row, 0)
            .expect("maps")
            .encode(&exact, &mut b)
            .expect("encodes");
        assert!(
            a.len() < b.len(),
            "fixed(5) {} bytes must beat exact {} bytes",
            a.len(),
            b.len()
        );

        // Same for the COL1 relation row, the other quantized collection.
        let relations = collection(COLLABORATION_RELATIONS);
        let mut relations_exact = relations.clone();
        relations_exact.fields[2].quant = Quant::Exact;
        let entry = CollaborationRelationRow {
            owner_tag_db_id: 24,
            target_tag_db_id: 1,
            progress_raw: 30_000,
        };
        let mut a = Vec::new();
        record_from_collaboration_row(&relations, &entry, 0)
            .expect("maps")
            .encode(&relations, &mut a)
            .expect("encodes");
        let mut b = Vec::new();
        record_from_collaboration_row(&relations_exact, &entry, 0)
            .expect("maps")
            .encode(&relations_exact, &mut b)
            .expect("encodes");
        assert!(a.len() < b.len());
    }

    #[test]
    fn fixed_five_reproduces_every_i32_it_is_given() {
        // `Fixed(5)` is claimed as the SOURCE's own decimal grid, not a precision
        // budget, so the claim is that `quantize(raw / 100000)` returns `raw` for
        // every `i32` the fold can hand it. This is a SAMPLE, not an enumeration:
        // a dense band around zero, every decade and its neighbours, the i32
        // edges, and 20,000 deterministic pseudorandom draws across the range.
        use crate::value::{escapes, quantize};
        let check = |raw: i32| {
            let v = raw as f64 / FIXED_SCALE;
            assert!(!escapes(v, FIXED_QUANT), "raw {raw} escapes");
            assert_eq!(
                quantize(v, FIXED_QUANT),
                Some(raw as i64),
                "raw {raw} does not reproduce"
            );
        };
        for raw in -200_000i32..=200_000 {
            check(raw);
        }
        let mut decade = 1i64;
        while decade < i32::MAX as i64 {
            for delta in -2i64..=2 {
                for sign in [1i64, -1] {
                    let candidate = sign * (decade + delta);
                    if (i32::MIN as i64..=i32::MAX as i64).contains(&candidate) {
                        check(candidate as i32);
                    }
                }
            }
            decade *= 10;
        }
        for raw in [i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX] {
            check(raw);
        }
        // A deterministic LCG, so a failure is reproducible.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..20_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            check((seed >> 32) as u32 as i32);
        }
        // And the whole COL1 progress band, which is small enough to enumerate.
        for raw in 0..=FIXED_SCALE_RAW as i64 {
            let v = raw as f64 / FIXED_SCALE;
            assert_eq!(quantize(v, FIXED_QUANT), Some(raw), "COL1 progress {raw}");
        }
    }

    // --- The mapper mirrors the fold, and only the fold ----------------------

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = states_collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 120] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_row(&row).is_err(), "length {len}");
        }
        // Zero state_id (`world_fold.rs::fold_occupation_resistance_compliance`).
        let zero = RawRow {
            state_id: 0,
            ..full_row()
        };
        assert!(record_from_row(&c, &zero.bytes(), 0).is_err());
        assert!(key_from_row(&zero.bytes()).is_err());
        // Every unknown flag bit, not just one sample (`:16879`).
        for bit in 16..32u32 {
            let row = RawRow {
                flags: REAL_FULL_FLAGS | (1 << bit),
                ..full_row()
            };
            assert!(
                record_from_row(&c, &row.bytes(), 0).is_err(),
                "flag bit {bit} is not in KNOWN_FLAGS and must be rejected"
            );
        }
        // Either reserved word non-zero (`:16884`).
        for which in 0..2 {
            let mut row = full_row();
            if which == 0 {
                row.reserved1 = 1;
            } else {
                row.reserved2 = 1;
            }
            assert!(record_from_row(&c, &row.bytes(), 0).is_err(), "reserved {which}");
        }
        // A tag flag with a zero tag, each of the three (`:16886`-`:16901`).
        for (flag, zero_field) in [
            (ROW_FLAG_OWNER, 0usize),
            (ROW_FLAG_CONTROLLER, 1),
            (ROW_FLAG_OCCUPIED_TAG_DERIVED, 2),
        ] {
            let mut row = full_row();
            assert!(row.flags & flag != 0, "the base row carries this flag");
            match zero_field {
                0 => row.owner = 0,
                1 => row.controller = 0,
                _ => row.occupied = 0,
            }
            assert!(
                record_from_row(&c, &row.bytes(), 0).is_err(),
                "flag {flag:#x} without its tag"
            );
        }
    }

    #[test]
    fn the_mapper_mirrors_the_folds_permissiveness_where_the_lane_is_stricter() {
        // The recipe's default: a port may not refuse what the fold accepts
        // merely because the emitter cannot produce it. Every input below is
        // rejected by `check_authority_payload` (so a real stream carrying it is
        // refused) and folded by the recorder (so the mapper must map it).
        let c = states_collection();
        // The `state_id: 70_000` case was retired by lane IA: 70 000 is an
        // ORDINARY state id now, so the lane contract no longer refuses it and
        // this list would be asserting the defect rather than the rule. The
        // state id space's structural width is `i32::MAX - 1`
        // (`claim_39c989ce13504504353c`); a state id past THAT is covered by
        // the boundary walk in
        // `tests/producer_consumer_admission_agreement.rs`, which probes the
        // structural edge rather than a content-scale number.
        let cases: [(&str, RawRow); 3] = [
            (
                "a tag past the lane's id-space bound",
                RawRow {
                    // One past the structural id-space width. Was `5_000`,
                    // which was only out of band while the lane's ceiling was
                    // the `1_024` content guess; it is an ordinary country id
                    // now, so the case had to move to the real boundary or it
                    // would stop testing the refusal at all.
                    owner: MAX_TAG_DB_ID + 1,
                    occupied: MAX_TAG_DB_ID + 1,
                    ..full_row()
                },
            ),
            (
                "a fixed word past the lane's 1e8 bound",
                RawRow {
                    resistance: 200_000_000,
                    ..full_row()
                },
            ),
            (
                "a negative garrison manpower need",
                RawRow {
                    garrison_manpower_need: -5,
                    ..full_row()
                },
            ),
        ];
        for (label, row) in cases {
            record_from_row(&c, &row.bytes(), 0)
                .unwrap_or_else(|e| panic!("the fold folds {label}, so the mapper must: {e}"));
            assert!(
                check_authority_payload(&payload(0, &[row])).is_err(),
                "the lane contract refuses {label}"
            );
        }
    }

    #[test]
    fn every_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another collection's descriptor must not get a
        // record full of placeholders under this collection's field ids. The
        // check is explicit because two of these collections would otherwise
        // iterate nothing and come out silently empty.
        let states = states_collection();
        assert!(record_from_row(&collection(GARRISON_ALERT), &full_row().bytes(), 0).is_err());
        let alert = parse_garrison_alert(&gar1_unavailable(1)).expect("GAR1");
        assert!(record_from_garrison_alert(&states, &alert, 0).is_err());
        let entry = AlertEquipmentRow {
            archetype_id: 1,
            whole_units: 1,
        };
        assert!(record_from_equipment_row(&states, &entry, 0).is_err());
        let collaboration =
            parse_collaboration(&col1(COLLABORATION_FLAG_SOURCE_UNAVAILABLE, 0, 0, &[]))
                .expect("COL1");
        assert!(record_from_collaboration_status(&states, &collaboration, 0).is_err());
        let relation = CollaborationRelationRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 2,
            progress_raw: 0,
        };
        assert!(record_from_collaboration_row(&states, &relation, 0).is_err());

        // And an unknown field id inside the right collection is refused too.
        let mut extended = states.clone();
        extended.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&extended, &full_row().bytes(), 0).is_err());
    }

    // --- `split_payload` carries the fold's structural checks, and no others --

    #[test]
    fn split_payload_enforces_the_folds_structural_rules_and_nothing_more() {
        let rows = [full_row(), RawRow { state_id: 65, ..full_row() }];
        let p = payload(0, &rows);
        let (header, body, trailers) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 2);
        assert_eq!(body.len(), 2 * ROW_LEN);
        assert!(trailers.is_empty());

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
            assert!(peek_header(&p[..len]).is_err(), "len {len}");
        }
        // Short of the rows the header claims.
        let mut truncated = p.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let mut huge = p.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());

        // `changed_rows` is NOT an invariant of this fold: the fold never reads
        // the word (`world_fold.rs::fold_occupation_resistance_compliance`), so refusing a record for it
        // here would refuse a record the recorder folds. It is the lane
        // contract's, and it lives in `check_authority_payload`.
        let mut mismatched = p.clone();
        mismatched[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(
            split_payload(&mismatched).is_ok(),
            "changed_rows is parsed and reported, never enforced"
        );
        assert_eq!(peek_header(&mismatched).expect("header").changed_rows, 0);
        let mut over = p.clone();
        over[12..16].copy_from_slice(&9u32.to_le_bytes());
        assert!(split_payload(&over).is_ok());
        assert!(
            check_authority_payload(&over).is_err(),
            "changed_rows > row_count is the LANE's rule, and it refuses the stream"
        );
        // Likewise `scale_raw`: never read by the fold, always by the lane.
        let mut wrong_scale = p.clone();
        wrong_scale[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert!(split_payload(&wrong_scale).is_ok());
        assert!(check_authority_payload(&wrong_scale).is_err());

        // Bytes past the rows are the trailer region, not an error: the fold
        // requires `>=`, never `==` (`world_fold.rs::fold_occupation_resistance_compliance`).
        let mut with_trailer = p.clone();
        with_trailer.extend_from_slice(&gar1_unavailable(3));
        let (_, body, trailers) = split_payload(&with_trailer).expect("trailers are legal");
        assert_eq!(body.len(), 2 * ROW_LEN);
        assert_eq!(trailers.len(), GARRISON_ALERT_HEADER_LEN);
    }

    // --- The trailers --------------------------------------------------------

    #[test]
    fn a_malformed_trailer_refuses_the_payload() {
        let good = gar1_unavailable(3);
        assert!(parse_trailers(&good).is_ok());
        // Truncated below the header.
        for len in 1..GARRISON_ALERT_HEADER_LEN {
            assert!(parse_trailers(&good[..len]).is_err(), "GAR1 cut to {len}");
        }
        // Magic, schema version and header length are each load-bearing.
        for (offset, patch) in [
            (0usize, vec![b'X']),
            (4, 2u16.to_le_bytes().to_vec()),
            (6, 41u16.to_le_bytes().to_vec()),
        ] {
            let mut t = good.clone();
            t[offset..offset + patch.len()].copy_from_slice(&patch);
            assert!(parse_trailers(&t).is_err(), "GAR1 patched at +{offset}");
        }
        // The header invariants, one at a time.
        for (label, t) in [
            (
                "both unavailable and complete",
                gar1(3, 0, GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE | GARRISON_ALERT_FLAG_COMPLETE, 0, &[]),
            ),
            ("neither unavailable nor complete", gar1(3, 0, 0, 0, &[])),
            (
                "unavailable with a controller",
                gar1(3, 7, GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE, 0, &[]),
            ),
            (
                "complete with no controller",
                gar1(3, 0, GARRISON_ALERT_FLAG_COMPLETE | GARRISON_ALERT_FLAG_PREDICATE_RETURNED, 0, &[]),
            ),
            (
                "a missing-manpower badge that disagrees with the word",
                gar1(3, 7, GARRISON_ALERT_FLAG_COMPLETE | GARRISON_ALERT_FLAG_MISSING_MANPOWER, 0, &[]),
            ),
            (
                "a negative missing-manpower word",
                gar1(3, 7, GARRISON_ALERT_FLAG_COMPLETE, -1, &[]),
            ),
            (
                "an unknown alert flag bit",
                gar1(3, 0, GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE | (1 << 8), 0, &[]),
            ),
        ] {
            assert!(parse_trailers(&t).is_err(), "GAR1 {label} must be refused");
        }
        // A zero divisor, a non-zero reserved word and a bad equipment row.
        let mut bad_divisor = gar1_complete(3, &[(11, 2)], 0);
        bad_divisor[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_trailers(&bad_divisor).is_err());
        let mut bad_reserved = gar1_complete(3, &[(11, 2)], 0);
        bad_reserved[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_trailers(&bad_reserved).is_err());
        for equipment in [
            vec![(0u32, 1i64)],
            vec![(11, 0)],
            vec![(11, -1)],
            vec![(11, 1), (11, 2)],
        ] {
            assert!(
                parse_trailers(&gar1_complete(3, &equipment, 0)).is_err(),
                "equipment {equipment:?}"
            );
        }
        // Trailing bytes after GAR1 that are not a whole COL1.
        let mut ragged = gar1_unavailable(3);
        ragged.push(0);
        assert!(parse_trailers(&ragged).is_err());

        // COL1's own invariants.
        let mut both = gar1_unavailable(3);
        both.extend_from_slice(&col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, 30_000)]));
        assert!(parse_trailers(&both).is_ok());
        for (label, tail) in [
            ("unproven empty set", col1(COLLABORATION_FLAG_COMPLETE, 0, 0, &[])),
            (
                "unavailable carrying rows",
                col1(COLLABORATION_FLAG_SOURCE_UNAVAILABLE, 0, 0, &[(24, 1, 1)]),
            ),
            (
                "complete with anomalies",
                col1(COLLABORATION_FLAG_COMPLETE, 436, 1, &[]),
            ),
            ("both flags", col1(COLLABORATION_KNOWN_FLAGS, 436, 0, &[])),
            ("neither flag", col1(0, 436, 0, &[])),
            (
                "a self relation",
                col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 24, 1)]),
            ),
            (
                "a zero owner",
                col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(0, 1, 1)]),
            ),
            (
                "an out-of-band tag",
                // One past the structural id-space width; `5_000` is an
                // ordinary country id now that the ceiling is not a content
                // guess.
                col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(MAX_TAG_DB_ID + 1, 1, 1)]),
            ),
            (
                "progress past 1.0",
                col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, 100_001)]),
            ),
            (
                "negative progress",
                col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, -1)]),
            ),
            (
                "a duplicated relation",
                col1(
                    COLLABORATION_FLAG_COMPLETE,
                    436,
                    0,
                    &[(24, 1, 1), (24, 1, 2)],
                ),
            ),
        ] {
            let mut t = gar1_unavailable(3);
            t.extend_from_slice(&tail);
            assert!(parse_trailers(&t).is_err(), "COL1 {label} must be refused");
        }
    }

    // --- `fold_payload`: baseline, delta, deletion, exclusion ----------------

    #[test]
    fn a_non_delta_frame_replaces_and_a_delta_frame_upserts() {
        let s = schema();
        let mut state = State::new();
        assert!(!state.baseline_seen());

        // The baseline: two states.
        let baseline = payload(
            0,
            &[full_row(), RawRow { state_id: 65, ..full_row() }],
        );
        assert_eq!(
            state.fold_payload(&s, &baseline).expect("folds"),
            FoldOutcome::Baseline
        );
        assert!(state.baseline_seen());
        assert_eq!(state.states.len(), 2);

        // A delta carrying one row leaves the other alone.
        let delta = payload(
            STATUS_DELTA_DATA,
            &[RawRow {
                state_id: 65,
                resistance: 999,
                ..full_row()
            }],
        );
        assert_eq!(
            state.fold_payload(&s, &delta).expect("folds"),
            FoldOutcome::Delta
        );
        assert_eq!(state.states.len(), 2, "the untouched state is retained");
        let c = states_collection();
        assert_eq!(
            value_of(&c, &state.states[&key(&[65])], field::RESISTANCE),
            Value::Continuous(0.00999)
        );
        assert_eq!(
            value_of(&c, &state.states[&key(&[64])], field::RESISTANCE),
            Value::Continuous(1.23456)
        );

        // A second non-delta frame CLEARS everything first
        // (`world_fold.rs::fold_occupation_resistance_compliance`), so the state it does not carry is
        // gone rather than retained.
        let rebaseline = payload(0, &[RawRow { state_id: 65, ..full_row() }]);
        assert_eq!(
            state.fold_payload(&s, &rebaseline).expect("folds"),
            FoldOutcome::Baseline
        );
        assert_eq!(state.states.len(), 1);
        assert!(state.states.contains_key(&key(&[65])));
    }

    #[test]
    fn a_delta_before_any_baseline_keeps_the_base_table_but_installs_the_trailers() {
        // The asymmetry that gives this outcome its own name. The fold assigns
        // the trailers (`world_fold.rs::fold_occupation_resistance_compliance`) and only THEN takes the
        // delta-before-baseline exit (`world_fold.rs::fold_occupation_resistance_compliance`) without touching them,
        // so they are this hour's data and the recorder publishes them. The base
        // table is left exactly as it was.
        let s = schema();
        let mut state = State::new();
        let delta = payload(STATUS_DELTA_DATA, &[full_row()]);
        assert_eq!(
            state.fold_payload(&s, &delta).expect("folds"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(state.states.is_empty(), "the base table is untouched");
        assert!(!state.baseline_seen(), "and no baseline was seen");
        assert!(state.garrison_alert.is_empty(), "this payload has no trailer");

        // With trailers, they land — all four collections, not just the alert.
        let mut with_trailers = delta.clone();
        with_trailers.extend_from_slice(&gar1_complete(9, &[(11, 2), (12, 3)], 0));
        with_trailers
            .extend_from_slice(&col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, 30_000)]));
        assert_eq!(
            state.fold_payload(&s, &with_trailers).expect("folds"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert_eq!(state.garrison_alert.len(), 1);
        assert_eq!(state.garrison_alert_equipment.len(), 2);
        assert_eq!(state.collaboration_status.len(), 1);
        assert_eq!(state.collaboration_relations.len(), 1);
        assert!(state.states.is_empty(), "still no base-table data");
        assert!(!state.baseline_seen());
        let alert_c = collection(GARRISON_ALERT);
        assert_eq!(
            value_of(
                &alert_c,
                &state.garrison_alert[&alert_singleton_key()],
                field::ALERT_GAME_HOUR_RAW
            ),
            Value::Signed(9),
            "the installed alert is THIS hour's, not a stale one"
        );

        // And they are replaced wholesale on the next such hour, like any other.
        let bare = payload(STATUS_DELTA_DATA, &[full_row()]);
        assert_eq!(
            state.fold_payload(&s, &bare).expect("folds"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(state.garrison_alert.is_empty());
        assert!(state.collaboration_relations.is_empty());

        // A row the fold would reject does NOT refuse this frame, because the
        // fold returns before its row loop and never parses the rows at all.
        let mut bad_row = payload(
            STATUS_DELTA_DATA,
            &[RawRow { state_id: 0, ..full_row() }],
        );
        bad_row.extend_from_slice(&gar1_unavailable(3));
        assert_eq!(
            state.fold_payload(&s, &bad_row).expect("the fold never reads the rows"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert_eq!(state.garrison_alert.len(), 1);

        // But a malformed TRAILER still refuses the stream, because the fold
        // decodes the trailers BEFORE it makes that decision (`:16813`, `:16836`).
        let mut broken = delta;
        broken.extend_from_slice(&gar1(3, 0, 0, 0, &[]));
        assert!(state.fold_payload(&s, &broken).is_err());
    }

    #[test]
    fn a_row_that_folds_to_an_empty_occupation_deletes_the_state() {
        // `world_fold.rs::fold_occupation_resistance_compliance`. This is a keyed-set tombstone, and it
        // is the reason `State::fold_payload` cannot simply upsert every row.
        let s = schema();
        let empty = RawRow {
            state_id: 64,
            ..Default::default()
        };
        assert!(row_folds_to_deletion(&empty.bytes()).expect("flags"));
        assert!(!row_folds_to_deletion(&full_row().bytes()).expect("flags"));

        let mut state = State::new();
        state
            .fold_payload(&s, &payload(0, &[full_row()]))
            .expect("folds");
        assert_eq!(state.states.len(), 1);
        state
            .fold_payload(&s, &payload(STATUS_DELTA_DATA, &[empty]))
            .expect("folds");
        assert!(state.states.is_empty(), "the row removed the state");

        // Every single content flag, on its own, keeps the state alive.
        for bit in 0..16u32 {
            let flag = 1u32 << bit;
            if flag & KNOWN_FLAGS == 0 {
                continue;
            }
            let mut row = RawRow {
                state_id: 64,
                flags: flag,
                ..Default::default()
            };
            // The three tag flags need their tag word, and `OCCUPIED_TAG_DERIVED`
            // additionally contributes nothing without `OCCUPIED_STATE`.
            match flag {
                ROW_FLAG_OWNER => row.owner = 1,
                ROW_FLAG_CONTROLLER => row.controller = 1,
                ROW_FLAG_OCCUPIED_TAG_DERIVED => row.occupied = 1,
                _ => {}
            }
            let expected_deletion = flag & OCCUPATION_CONTENT_FLAGS == 0;
            assert_eq!(
                row_folds_to_deletion(&row.bytes()).expect("flags"),
                expected_deletion,
                "flag {flag:#x} alone"
            );
        }
        // `OCCUPIED_STATE` alone contributes nothing; paired with any tag flag it
        // does (`world_fold.rs::fold_occupation_resistance_compliance`).
        let bare = RawRow {
            state_id: 64,
            flags: ROW_FLAG_OCCUPIED_STATE,
            ..Default::default()
        };
        assert!(row_folds_to_deletion(&bare.bytes()).expect("flags"));
        for (flag, word) in [
            (ROW_FLAG_OWNER, 0usize),
            (ROW_FLAG_CONTROLLER, 1),
            (ROW_FLAG_OCCUPIED_TAG_DERIVED, 2),
        ] {
            let mut row = bare;
            row.flags |= flag;
            match word {
                0 => row.owner = 1,
                1 => row.controller = 1,
                _ => row.occupied = 1,
            }
            assert!(
                !row_folds_to_deletion(&row.bytes()).expect("flags"),
                "OCCUPIED_STATE | {flag:#x} carries a tag"
            );
        }
    }

    #[test]
    fn the_occupation_law_marker_is_a_constant_and_is_not_re_encoded() {
        // `world_fold.rs::fold_occupation_resistance_compliance` writes `state["occupation_law"] = Null` for every
        // folded row — a constant with no measured value, so it is not a field
        // (the derived-constant rule). The one consequence: the marker is written
        // even by a deletion row, so the fold's marker set is a SUPERSET of its
        // `occupation` set. The two coincide on every payload the collector can
        // produce, because the collector never emits a deletion row, and this
        // test pins that reasoning to the flag word it depends on.
        let states = states_collection();
        assert!(
            states.fields.iter().all(|f| f.field_id != 0),
            "no field id is reserved for the marker"
        );
        // The collector's unconditional flags (`collector_module.rs:1079`-`:1082`).
        let collector_floor = ROW_FLAG_RESISTANCE
            | ROW_FLAG_COMPLIANCE
            | ROW_FLAG_RESISTANCE_SPEED
            | ROW_FLAG_COMPLIANCE_SPEED;
        assert_eq!(
            collector_floor & OCCUPATION_CONTENT_FLAGS,
            collector_floor,
            "every flag the collector always sets is an occupation-content flag"
        );
        let minimal = RawRow {
            state_id: 1,
            flags: collector_floor,
            ..Default::default()
        };
        assert!(
            !row_folds_to_deletion(&minimal.bytes()).expect("flags"),
            "so the leanest row the collector can emit is never a deletion"
        );
    }

    #[test]
    fn the_trailers_are_replaced_wholesale_every_hour() {
        // `world_fold.rs::reset_hour_state` clears both at the top of every hour, so
        // they are per-hour data even though the base table is an accumulator.
        let s = schema();
        let mut state = State::new();
        let mut first = payload(0, &[full_row()]);
        first.extend_from_slice(&gar1_complete(1, &[(11, 2), (12, 3)], 0));
        first.extend_from_slice(&col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, 30_000)]));
        state.fold_payload(&s, &first).expect("folds");
        assert_eq!(state.garrison_alert.len(), 1);
        assert_eq!(state.garrison_alert_equipment.len(), 2);
        assert_eq!(state.collaboration_status.len(), 1);
        assert_eq!(state.collaboration_relations.len(), 1);

        // An hour with no trailer at all empties every one of them, and leaves
        // the base accumulator alone.
        state
            .fold_payload(&s, &payload(STATUS_DELTA_DATA, &[]))
            .expect("folds");
        assert_eq!(state.states.len(), 1, "the base table is retained");
        assert!(state.garrison_alert.is_empty());
        assert!(state.garrison_alert_equipment.is_empty());
        assert!(state.collaboration_status.is_empty());
        assert!(state.collaboration_relations.is_empty());
    }

    #[test]
    fn fold_payload_is_all_or_nothing_on_a_bad_row() {
        // The documented error contract: nothing is mutated when any row fails.
        let s = schema();
        let mut state = State::new();
        state
            .fold_payload(&s, &payload(0, &[full_row()]))
            .expect("folds");
        let before = state.clone();
        let bad = payload(
            STATUS_DELTA_DATA,
            &[
                RawRow { state_id: 65, ..full_row() },
                RawRow { state_id: 0, ..full_row() },
            ],
        );
        assert!(state.fold_payload(&s, &bad).is_err());
        assert_eq!(state, before, "a failed fold leaves the accumulator untouched");
    }

    // --- The lane contract's decode ------------------------------------------

    #[test]
    fn check_authority_payload_enforces_the_lane_contracts_own_rules() {
        let rows = [full_row(), RawRow { state_id: 65, ..full_row() }];
        let good = payload(0, &rows);
        check_authority_payload(&good).expect("a well-formed payload");

        // Header cross-counters must EQUAL the decoded counts, not merely bound
        // them (`domain44_compact.rs:584`-`:608`).
        for offset in [24usize, 28, 36, 40, 44] {
            let mut p = good.clone();
            let claimed = u32_at(&p, offset);
            p[offset..offset + 4].copy_from_slice(&claimed.wrapping_sub(1).to_le_bytes());
            assert!(
                check_authority_payload(&p).is_err(),
                "counter at +{offset} must be cross-checked"
            );
        }
        // Rows beyond the roots walked. This used to have its own check and its
        // own message; the universal census inequality below subsumes it, so
        // this now asserts the REFUSAL rather than a particular wording.
        let mut p = good.clone();
        p[20..24].copy_from_slice(&1u32.to_le_bytes());
        assert!(check_authority_payload(&p).is_err());

        // The census pair, mirrored from the lane decoder. Deleting either check
        // from `check_authority_payload` left the whole suite green while this
        // authority certified streams the lane decoder refuses -- the exact
        // two-authority drift the checks exist to prevent. Each assertion below
        // is arranged so that exactly ONE production check can decide it.
        //
        // (a) The UNIVERSAL inequality, isolated on a DELTA so the full-frame
        // identity cannot be what refuses. `good` has two rows and two observed
        // roots; as a delta claiming two drops, rows + drops = 4 > 2.
        let mut p = good.clone();
        p[16..20].copy_from_slice(&STATUS_DELTA_DATA.to_le_bytes());
        p[52..56].copy_from_slice(&2u32.to_le_bytes());
        assert!(
            check_authority_payload(&p).is_err(),
            "rows and drops together may not exceed the roots walked, delta or not"
        );
        // (b) The FULL-frame identity, isolated by staying INSIDE the
        // inequality: three observed roots, two rows, zero drops. Nothing is
        // over-accounted, so only the equality can refuse it.
        let mut p = good.clone();
        p[20..24].copy_from_slice(&3u32.to_le_bytes());
        assert!(
            check_authority_payload(&p).is_err(),
            "a full frame's rows and drops must account for every observed root"
        );
        // (c) The SCOPING: the very same payload is legal as a delta, because a
        // delta carries only the changed rows and its count says nothing about
        // the walk. Without this, an identity applied to every frame would
        // refuse every healthy delta, and no other assertion here would notice.
        p[16..20].copy_from_slice(&STATUS_DELTA_DATA.to_le_bytes());
        check_authority_payload(&p)
            .expect("a delta's row count need not account for every observed root");
        // A duplicated state_id inside one frame — a rejected frame, never a
        // last-writer-wins overwrite.
        let dup = payload(0, &[full_row(), full_row()]);
        assert!(check_authority_payload(&dup).is_err());
        // Value-without-flag, one channel at a time.
        for (offset, flag) in [
            (16usize, ROW_FLAG_RESISTANCE),
            (20, ROW_FLAG_COMPLIANCE),
            (24, ROW_FLAG_RESISTANCE_SPEED),
            (28, ROW_FLAG_COMPLIANCE_SPEED),
        ] {
            let row = RawRow {
                flags: REAL_FULL_FLAGS & !flag,
                ..full_row()
            };
            let mut p = payload(0, &[row]);
            let at = HEADER_LEN + offset;
            p[at..at + 4].copy_from_slice(&7i32.to_le_bytes());
            assert!(check_authority_payload(&p).is_err(), "channel at +{offset}");
        }
        // Tag-without-flag: the derived flag is clear but the word at +12 is
        // still populated, which the FOLD ignores entirely and the lane refuses.
        let mut p = payload(
            0,
            &[RawRow {
                flags: REAL_FULL_FLAGS & !ROW_FLAG_OCCUPIED_TAG_DERIVED,
                ..full_row()
            }],
        );
        assert!(check_authority_payload(&p).is_err());
        // Zeroing the word makes the same payload well-formed, so the rejection
        // above is about the tag/flag pairing and nothing else.
        p[HEADER_LEN + 12..HEADER_LEN + 16].copy_from_slice(&0u32.to_le_bytes());
        check_authority_payload(&p).expect("a cleared flag with a zeroed word is legal");
        // The derived flag requires BOTH owner and controller
        // (`domain44_compact.rs:557`-`:563`).
        let no_owner = RawRow {
            owner: 0,
            flags: REAL_FULL_FLAGS & !ROW_FLAG_OWNER,
            ..full_row()
        };
        assert!(check_authority_payload(&payload(0, &[no_owner])).is_err());
        // A default-law badge without a token.
        let orphan_badge = RawRow {
            law_token: 0,
            flags: REAL_FULL_FLAGS & !ROW_FLAG_OCCUPATION_LAW_TOKEN,
            ..full_row()
        };
        assert!(check_authority_payload(&payload(0, &[orphan_badge])).is_err());
        // Garrison scalars without a controller.
        let no_controller = RawRow {
            controller: 0,
            occupied: 0,
            flags: REAL_FULL_FLAGS
                & !(ROW_FLAG_CONTROLLER | ROW_FLAG_OCCUPIED_TAG_DERIVED),
            ..full_row()
        };
        assert!(check_authority_payload(&payload(0, &[no_controller])).is_err());
        // A speed badge beside a measured speed.
        let both_speed = RawRow {
            flags: REAL_FULL_FLAGS | ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE,
            ..full_row()
        };
        assert!(check_authority_payload(&payload(0, &[both_speed])).is_err());
        // And a malformed trailer, which the lane decodes between the row loop
        // and the counter cross-checks.
        let mut bad_trailer = good.clone();
        bad_trailer.extend_from_slice(&gar1(3, 0, 0, 0, &[]));
        assert!(check_authority_payload(&bad_trailer).is_err());
        // `i32::MIN` must be rejected by the magnitude bound without panicking on
        // an overflowing `abs()`.
        let extreme = RawRow {
            resistance: i32::MIN,
            ..full_row()
        };
        assert!(check_authority_payload(&payload(0, &[extreme])).is_err());
        // Payloads shorter than the header, and one whose rows are short.
        for len in 0..HEADER_LEN {
            assert!(check_authority_payload(&good[..len]).is_err(), "len {len}");
        }
        assert!(check_authority_payload(&good[..good.len() - 1]).is_err());
    }

    #[test]
    fn the_lane_bounds_a_complete_alerts_controller_and_the_fold_does_not() {
        // The one place the two trailer grammars disagree
        // (`domain44_compact.rs:666`-`:667` against `world_fold.rs::decode_occupation_garrison_alert`). The
        // fold-facing parser must stay permissive — refusing a trailer the
        // recorder folds is the data loss this recipe exists to prevent — and the
        // authority path must refuse it, or the harness measures a stream the
        // recorder rejects. Swept over the bound rather than asserted at one
        // point, so an off-by-one fails.
        let alert_at = |controller: u32| {
            let mut t = gar1(3, controller, GARRISON_ALERT_FLAG_COMPLETE | GARRISON_ALERT_FLAG_PREDICATE_RETURNED, 0, &[]);
            assert_eq!(t.len(), GARRISON_ALERT_HEADER_LEN);
            t.truncate(GARRISON_ALERT_HEADER_LEN);
            t
        };
        for controller in [1u32, 2, MAX_TAG_DB_ID - 1, MAX_TAG_DB_ID] {
            let t = alert_at(controller);
            parse_trailers(&t).unwrap_or_else(|e| panic!("controller {controller}: {e}"));
            check_authority_trailers(&t)
                .unwrap_or_else(|e| panic!("controller {controller} is inside the bound: {e}"));
        }
        // `5_000` used to sit in this list; it is an ordinary country id now
        // that the ceiling is the engine's id-space width rather than a guess
        // at the vanilla country count, so the out-of-band probes are the
        // boundary itself and the words a signed index cannot express.
        for controller in [MAX_TAG_DB_ID + 1, MAX_TAG_DB_ID + 2, 0x8000_0000, u32::MAX] {
            let t = alert_at(controller);
            parse_trailers(&t).unwrap_or_else(|e| {
                panic!("the FOLD folds controller {controller}, so the parser must: {e}")
            });
            assert!(
                check_authority_trailers(&t).is_err(),
                "the lane refuses controller {controller}"
            );
        }
        // The unavailable arm is untouched by the bound: its controller is
        // already forced to zero by an invariant both decoders share.
        check_authority_trailers(&gar1_unavailable(3)).expect("unavailable is unaffected");

        // And the whole payload path agrees, since that is what the harness calls.
        let mut p = payload(0, &[full_row()]);
        p.extend_from_slice(&alert_at(MAX_TAG_DB_ID));
        check_authority_payload(&p).expect("inside the bound");
        let mut p = payload(0, &[full_row()]);
        p.extend_from_slice(&alert_at(MAX_TAG_DB_ID + 1));
        assert!(check_authority_payload(&p).is_err(), "past the bound");
        // The mapper still maps the out-of-bound alert, because the fold does.
        let alert = parse_garrison_alert(&alert_at(MAX_TAG_DB_ID + 1)).expect("the fold folds it");
        let alert_c = collection(GARRISON_ALERT);
        let r = record_from_garrison_alert(&alert_c, &alert, 0).expect("maps");
        assert_eq!(
            value_of(&alert_c, &r, field::ALERT_CONTROLLER_TAG_DB_ID),
            Value::ContentId((MAX_TAG_DB_ID + 1) as u64)
        );
    }

    // --- Step 9's replay, in miniature ---------------------------------------

    #[test]
    fn a_baseline_and_a_delta_replay_to_the_folds_own_state() {
        // The measurement bin asserts this every hour against a real capture;
        // this is the same assertion over hand-built payloads, so the boundary is
        // covered by the suite as well as by the harness.
        let s = schema();
        let mut fold = State::new();

        let mut hour0 = payload(0, &[full_row(), RawRow { state_id: 65, ..full_row() }]);
        hour0.extend_from_slice(&gar1_complete(1, &[(11, 2)], 0));
        hour0.extend_from_slice(&col1(COLLABORATION_FLAG_COMPLETE, 436, 0, &[(24, 1, 30_000)]));
        fold.fold_payload(&s, &hour0).expect("folds");
        let prior: Vec<KeyedState> = fold.collections().iter().map(|c| (*c).clone()).collect();

        let mut wire = Vec::new();
        encode_baseline(&s, &prior, &mut wire).expect("baseline encodes");
        let (decoded, consumed) = decode_baseline(&s, &wire, 0).expect("baseline decodes");
        assert_eq!(consumed, wire.len());
        assert_eq!(decoded, prior);

        // Hour 1: one state changes, one is deleted, the trailers go away.
        let hour1 = payload(
            STATUS_DELTA_DATA,
            &[
                RawRow { state_id: 65, resistance: 1, ..full_row() },
                RawRow { state_id: 64, ..Default::default() },
            ],
        );
        fold.fold_payload(&s, &hour1).expect("folds");
        let next: Vec<KeyedState> = fold.collections().iter().map(|c| (*c).clone()).collect();
        assert_eq!(next[0].len(), 1, "state 64 was deleted by its empty row");
        assert!(next[1].is_empty(), "the GAR1 singleton went away");

        let deltas: Vec<Option<_>> = s
            .collections
            .iter()
            .zip(prior.iter())
            .zip(next.iter())
            .map(|((c, p), n)| {
                let d = diff(c, p, n);
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect();
        let mut delta_bytes = Vec::new();
        encode_delta(&s, &prior, &deltas, &mut delta_bytes).expect("delta encodes");
        let (read, consumed) =
            decode_delta(&s, &decoded, &delta_bytes, 0).expect("delta decodes");
        assert_eq!(consumed, delta_bytes.len());
        let mut replayed = decoded;
        for (state, delta) in replayed.iter_mut().zip(read.iter()) {
            if let Some(delta) = delta {
                apply(state, delta);
            }
        }
        for (i, (c, expected)) in s.collections.iter().zip(next.iter()).enumerate() {
            assert!(
                diff(c, &replayed[i], expected).is_empty(),
                "collection {} is not stored-equal to the fold",
                c.collection_id
            );
            assert_eq!(replayed[i].len(), expected.len(), "collection {i} row count");
        }
    }

}
