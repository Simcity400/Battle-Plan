//! Domain 51 `scripted_state_flags_variables_live` — schema, boundary mappers and
//! the frame-level fold mirror.
//!
//! Authority, in the recipe's precedence order:
//! `world_fold.rs:24364`-`:24581`'s `fold_scripted_state` (dispatched
//! at `:4726`), its row validator `validate_scripted_state_row` (`world_fold.rs::validate_scripted_state_row`), its
//! accumulator writer `apply_scripted_state_row` (`world_fold.rs::apply_scripted_state_row`) and its clear
//! `clear_scripted_state_accumulators` (`world_fold.rs::clear_scripted_state_accumulators`). There is no dossier for this
// @world-fold-authority domain=51 functions=fold_scripted_state lines=24364-24581 sha256=a25fb8448356b23e4b75e48ae1e52ad195bea2388de296f60fa7757be25408fe closure=e2b4ed4991607fe293c71b535d2e2b9c429e418ea5d7945e20a00d44f309683e
// @lane-payload-authority domain=51 path=extractor/native/lanes/scripted_state_flags_variables_live/scripted_state_compact.rs function=decode_payload lines=213-645 sha256=ebb8ac22ae995651cc86c6ff81709f2ccce422c0b4e830714cfb5b48b624bc80
//! domain; the fold is sufficient. The lane's own payload decoder
//! (`lanes/scripted_state_flags_variables_live/scripted_state_compact.rs`,
//! reached from `recorder_host::domain_data_authority:8081`) is production's
//! SECOND, status-independent authority and is transcribed here as
//! [`check_authority_payload`] — deliberately separate from the mappers, which
//! mirror the far more permissive fold.
//!
//! # Grammar class — baseline + delta, with a per-frame sub-clear
//!
//! `DELTA_DATA` (`world_fold.rs:18927`) selects the frame mode and
//! `reject_delta_before_baseline` (`:18937`) refuses a delta that arrives first.
//! That is the recipe's shape 2 — with one wrinkle the recipe has no name for and
//! which is therefore modelled explicitly here:
//!
//! * a **non-delta** frame clears ALL FIVE accumulators (`:19067` →
//!   `clear_scripted_state_accumulators`, `world_fold.rs::clear_scripted_state_accumulators`);
//! * a **delta** frame clears the opaque-badge and delayed-event sub-maps and
//!   ONLY those (`:19069`-`:19072`).
//!
//! So three of this domain's five collections are baseline-plus-delta with
//! tombstones, and two are **full replace every frame** — a delta frame carrying
//! no badge row empties the badge collection. Mixing the two shapes inside one
//! payload is not one of the recipe's five, and it is reported as a recipe gap.
//!
//! # One payload, five accumulators — why five collections
//!
//! `apply_scripted_state_row` writes each `value_kind` into a different place:
//!
//! | kind | fold destination | this module |
//! |---|---|---|
//! | 1 `VARIABLE_FIXED_I64` | `scripted_world_variables` / country `extra._r2_script_variables` / state `_r2_scripted_state.variables` (`world_fold.rs::apply_scripted_state_row`) | [`VARIABLES`] |
//! | 2 `FLAG_NAMED` | `scripted_world_flags` / country `extra.script_flags` / state `.flags` (`world_fold.rs::apply_scripted_state_row`) | [`NAMED_FLAGS`] |
//! | 3 `FLAG_TOKEN_ONLY` | `scripted_world_token_only_flags` / `extra._r2_token_only_flags` / state `.token_only_flags` (`world_fold.rs::apply_scripted_state_row`) | [`TOKEN_FLAGS`] |
//! | 4 `DELAYED_EVENT_OPAQUE`, 5 `SCRIPTED_GUI_OPAQUE` | `scripted_world_opaque_badges: Vec` (`world_fold.rs::apply_scripted_state_row`) | [`OPAQUE_BADGES`] |
//! | 6 `DELAYED_EVENT` | `scripted_world_delayed_events: BTreeMap<(u32,u32,u64),_>` (`world_fold.rs::apply_scripted_state_row`) | [`DELAYED_EVENTS`] |
//!
//! The three scopes (`GLOBAL`/`COUNTRY`/`STATE`) are three storage locations
//! inside each of those families, keyed by `owner_id`, so `scope_kind` and
//! `owner_id` are key components rather than five more collections.
//!
//! # `scope_kind` leads the key, and that is load-bearing for the measurement
//!
//! §7.4 delta-chains only the FIRST key component; later components are absolute
//! varints (`body.rs:88`-`:116`). With `(scope_kind, owner_id, name_symbol)` the
//! §5 symbol id is never the leading component, so a different symbol assignment
//! permutes rows only *within* one `(scope_kind, owner_id)` run — it cannot change
//! the sequence of leading components and therefore cannot change the delta chain.
//! The whole assignment-sensitivity of the measurement collapses to
//! `sum(varint_len(symbol_id))`, which `bin/bench_real_d51.rs` reports as an exact
//! band. A symbol-led key would have made the band a permutation problem instead.
//!
//! # `value_raw` is carried RAW, not as `Fixed(5)` — and this diverges from d52
//!
//! The fold divides `value_raw` by the header's `fixed_scale` for **kind 1 only**
//! (`:19173`-`:19175`), and stores the undivided integer beside it in
//! `scripted_world_variables_raw_fixed5` (`:19157`-`:19160`) — the float is a pure
//! function of the raw and the scale. Kinds 2 and 3 store `value_raw` with no
//! divisor at all (`ScriptFlagState`, `world_fold.rs:25501`), so the SAME row word
//! means two different things depending on `value_kind`: the recipe's d61 hazard,
//! resolved the way d61 resolves it — carry the raw and pin the projection in
//! exactly one place ([`variable_display_value`]).
//!
//! d52 ships the same `/100_000` grid as `Quant::Fixed(5)` on the recipe's d24
//! rule, and this port deliberately does not, for one measured reason: d52's
//! source word is an **i32**, so its whole input domain sits three decimal orders
//! inside the `Fixed(5)` reproduction band, while d51's is an **i64**
//! (`world_fold.rs:19000`, `read_i64`). `Fixed(5)` would silently stop
//! reproducing the raw integer above roughly `2^51`, on input the fold accepts and
//! this port may not refuse (the recipe's "never gate against the fold"). A
//! `Signed` raw is exactly lossless over every `i64` the fold folds and cheaper
//! than the `Exact` f64 that would otherwise be required. The capture's own range
//! is `-1_073_741_821 ..= 100_000_000_000`, well inside `Fixed(5)`'s band — so
//! this is a fidelity choice about the fold's input domain, not about the capture.
//!
//! Consequence, recorded rather than glossed: this domain has **no `Continuous`
//! field at all**, so the recipe's quantization-saves-bytes test does not apply
//! and says so ([`no_field_is_continuous_so_quantization_does_not_apply`]).
//!
//! # Four-state decisions
//!
//! * [`VARIABLES`]`::VALUE_RAW` — required and concrete. `validate_scripted_state_row`
//!   refuses a live variable row without `VALUE_PRESENT` (`world_fold.rs::validate_scripted_state_row`),
//!   so the gate can never be clear on a row the fold folds. No capability.
//! * [`NAMED_FLAGS`] / [`TOKEN_FLAGS`] `VALUE_RAW`, `DATE_HOUR_RAW` — **Unknown**
//!   on a clear gate. The fold stores `Option` and publishes JSON `null`
//!   (`ScriptFlagState::to_json`, `:25507`), which answers null-vs-value only. The
//!   collector's ONLY flag-row construction sets both
//!   (`attach_log.rs:44545`-`:44559`, `value: Some(..)`, `date_hour_raw: Some(..)`),
//!   so a clear gate is a payload the emitter cannot produce and, if one ever
//!   arrived, would mean the producer did not supply a value that exists in the
//!   engine — §3's "applicable, not captured". The recipe's tie-break points the
//!   same way. Zero coverage in this capture: every one of the 1,959 live flag
//!   rows sets both gates.
//! * [`OPAQUE_BADGES`]`::BADGE_KEY` — **Null** on a clear key gate. The fold
//!   stores `key: Option<String>` and emits `"key": null`
//!   (`ScriptedOpaqueBadgeState::to_json`, `:25523`-`:25530`); the string is a
//!   producer-chosen marker, not a read that can fail, so a clear gate is "this
//!   badge names nothing", not "the name was not captured".
//! * [`DELAYED_EVENTS`]`::DUE_REMAINING_HOURS` — **Unknown** on a clear gate. The
//!   collector badges it absent on "an out-of-bounds / torn due read"
//!   (`scripted_state_domain51_collector.rs:99`-`:107`); an unreadable or
//!   range-rejected value is §3's unknown, which is d63's ruling and the rule the
//!   recipe says wins over the fold's own "absent" wording.
//!
//! # Values that are not fields
//!
//! * The delayed-event row's `date_hour_raw` slot carries the **scope tag**, and
//!   the fold requires it to equal `owner_id` (`world_fold.rs::validate_scripted_state_row`).
//!   A value the fold hard-enforces as a function of carried state is not a field:
//!   it is enforced in [`parse_row`] and pinned by
//!   [`the_delayed_event_scope_tag_is_owner_and_is_not_re_encoded`].
//! * The `variables` float. See above — [`variable_display_value`] recomputes it.
//!
//! # Tombstones
//!
//! `deleted = (raw_value_kind & 1<<31) != 0 || (flags & ROW_FLAG_DELETED) != 0`
//! (`world_fold.rs::fold_scripted_state`). Only the value-kind form is refused outside a
//! delta frame (`world_fold.rs::fold_scripted_state`); a `ROW_FLAG_DELETED` tombstone inside a
//! baseline frame is accepted by both the fold and the payload authority, and
//! removes a key from the just-cleared accumulator. Kinds 4, 5 and 6 have no
//! tombstone form at all — `validate_scripted_state_row` refuses one (`world_fold.rs::validate_scripted_state_row`).
//!
//! # The one place this port refuses what the fold accepts
//!
//! The badge accumulator is a `Vec` the fold PUSHes to (`:19280`), so two rows
//! sharing `(scope_kind, owner_id, value_kind)` coexist there; a keyed set cannot
//! express that, and mirroring would silently keep one and drop the other — the
//! data-loss defect itself. An ordered occurrence shape is now available, but a
//! d51 migration to it would need its own reachable-input evidence; a positional
//! key would still be transport re-packed from emission order every frame and
//! churn every badge row whenever the badge set changes shape (d32's ruling).
//! [`fold_input`] therefore still REFUSES two badge rows with the same identity.
//! This is bounded by producer evidence that the input is unreachable: the
//! payload authority's own duplicate-identity check covers
//! `(scope_kind, owner_id, value_kind, key_bytes, token_id, seed)`
//! (`scripted_state_compact.rs:533`-`:546`) **status-independently**, and it also
//! forces every badge row's key to the one constant its kind allows
//! (`validate_opaque_badge_key`, `:684`-`:697`) — so two same-kind badge rows are
//! always a duplicate identity and always refuse the record at certification.
//! The collector emits at most one badge row per kind per frame
//! (`scripted_state_domain51_collector.rs:538`-`:572`). Pinned by
//! [`two_badge_rows_with_one_identity_are_refused`].

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 51;

/// `value_kind` 1 — scripted variables, keyed by `(scope, owner, name)`.
pub const VARIABLES: u32 = 1;
/// `value_kind` 2 — scripted flags whose name resolved, same key shape.
pub const NAMED_FLAGS: u32 = 2;
/// `value_kind` 3 — scripted flags whose name did NOT resolve, keyed by token id.
pub const TOKEN_FLAGS: u32 = 3;
/// `value_kind` 4 and 5 — the two fixture-blocked opaque root badges.
pub const OPAQUE_BADGES: u32 = 4;
/// `value_kind` 6 — measured pending delayed events.
pub const DELAYED_EVENTS: u32 = 5;

pub const COLLECTION_COUNT: usize = 5;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
/// Unique across the whole domain, not merely within a collection, so a misrouted
/// record cannot silently land on a same-numbered field elsewhere.
pub mod field {
    // --- VARIABLES ---
    pub const VARIABLE_SCOPE_KIND: u32 = 10;
    pub const VARIABLE_OWNER_ID: u32 = 11;
    pub const VARIABLE_NAME: u32 = 12;
    pub const VARIABLE_VALUE_RAW: u32 = 13;

    // --- NAMED_FLAGS ---
    pub const NAMED_FLAG_SCOPE_KIND: u32 = 20;
    pub const NAMED_FLAG_OWNER_ID: u32 = 21;
    pub const NAMED_FLAG_NAME: u32 = 22;
    pub const NAMED_FLAG_VALUE_RAW: u32 = 23;
    pub const NAMED_FLAG_DATE_HOUR_RAW: u32 = 24;

    // --- TOKEN_FLAGS ---
    pub const TOKEN_FLAG_SCOPE_KIND: u32 = 30;
    pub const TOKEN_FLAG_OWNER_ID: u32 = 31;
    pub const TOKEN_FLAG_TOKEN_ID: u32 = 32;
    pub const TOKEN_FLAG_VALUE_RAW: u32 = 33;
    pub const TOKEN_FLAG_DATE_HOUR_RAW: u32 = 34;

    // --- OPAQUE_BADGES ---
    pub const BADGE_SCOPE_KIND: u32 = 40;
    pub const BADGE_OWNER_ID: u32 = 41;
    pub const BADGE_KIND: u32 = 42;
    pub const BADGE_KEY: u32 = 43;

    // --- DELAYED_EVENTS ---
    pub const DELAYED_OWNER_TAG_DB_ID: u32 = 50;
    pub const DELAYED_COMBINED_EVENT_ID: u32 = 51;
    pub const DELAYED_SCOPE_RANDOM: u32 = 52;
    pub const DELAYED_DUE_REMAINING_HOURS: u32 = 53;
}

// --- Payload grammar constants ----------------------------------------------
//
// Every one is transcribed from `world_fold.rs:921`-`:965`, with the lane codec
// (`scripted_state_compact.rs:3`-`:74`) as the second copy. The transcription is
// pinned field-by-field by [`the_transcribed_grammar_constants_match_the_fold`].

/// `SCRIPTED_STATE_HEADER_LEN`, `world_fold.rs::SCRIPTED_STATE_HEADER_LEN`.
pub const HEADER_LEN: usize = 64;
/// `SCRIPTED_STATE_ROW_LEN`, `world_fold.rs::SCRIPTED_STATE_ROW_LEN`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 48;
/// `SCRIPTED_STATE_SCHEMA_VERSION`, `world_fold.rs::SCRIPTED_STATE_SCHEMA_VERSION`. The fold refuses any
/// other version BEFORE it classifies the status word, so [`peek_header`] does.
pub const SCHEMA_VERSION: u32 = 2;

/// The header's `fixed_scale` word (`+48`). `ensure_fixed_scale` asserts it
/// (`world_fold.rs:18954`) — an invariant, never a field.
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// The same value as the divisor the fold applies to kind-1 rows only.
pub const FIXED_SCALE: f64 = 100_000.0;

pub const SCOPE_GLOBAL: u32 = 1;
pub const SCOPE_COUNTRY: u32 = 2;
pub const SCOPE_STATE: u32 = 3;

pub const VALUE_VARIABLE_FIXED_I64: u32 = 1;
pub const VALUE_FLAG_NAMED: u32 = 2;
pub const VALUE_FLAG_TOKEN_ONLY: u32 = 3;
pub const VALUE_DELAYED_EVENT_OPAQUE: u32 = 4;
pub const VALUE_SCRIPTED_GUI_OPAQUE: u32 = 5;
pub const VALUE_DELAYED_EVENT: u32 = 6;
/// `SCRIPTED_STATE_VALUE_KIND_DELETED`, `world_fold.rs::SCRIPTED_STATE_VALUE_KIND_DELETED`. The high bit of the
/// value-kind word is one of the two tombstone encodings.
pub const VALUE_KIND_DELETED: u32 = 1 << 31;

pub const ROW_FLAG_KEY_PRESENT: u32 = 1;
pub const ROW_FLAG_TOKEN_PRESENT: u32 = 1 << 1;
pub const ROW_FLAG_VALUE_PRESENT: u32 = 1 << 2;
pub const ROW_FLAG_DATE_PRESENT: u32 = 1 << 3;
pub const ROW_FLAG_DELETED: u32 = 1 << 4;
pub const ROW_FLAG_DELAYED_DUE_PRESENT: u32 = 1 << 5;
/// `SCRIPTED_STATE_ROW_KNOWN_FLAGS`, `world_fold.rs::SCRIPTED_STATE_ROW_KNOWN_FLAGS`. Any other bit is
/// a hard error (`:18966`).
pub const ROW_KNOWN_FLAGS: u32 = ROW_FLAG_KEY_PRESENT
    | ROW_FLAG_TOKEN_PRESENT
    | ROW_FLAG_VALUE_PRESENT
    | ROW_FLAG_DATE_PRESENT
    | ROW_FLAG_DELETED
    | ROW_FLAG_DELAYED_DUE_PRESENT;

/// `COMPACT_STATUS_DELTA_DATA` (`compact_contract/src/status_flags_base.rs:16`).
/// Named here rather than depended on: the library half of this crate is the
/// reference implementation and must not pull in the shipping contract crate.
/// [`the_status_bits_match_the_shipping_contract`] pins every one of these.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
/// `DOMAIN51_STATUS_FLAG_NAMES_PARTIAL` (`status_flags_base.rs:69`) — lane-local
/// bit 11, registered in `lane_local_status_mask` (`compact_contract/src/lib.rs:141`).
pub const STATUS_FLAG_NAMES_PARTIAL: u32 = 1 << 11;
/// `DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED` (`status_flags_base.rs:70`).
pub const STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED: u32 = 1 << 12;
/// `DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED` (`status_flags_base.rs:71`).
pub const STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED: u32 = 1 << 13;

/// `SCRIPTED_STATE_MIN_DELAYED_EVENT_ID` / `_MAX_`, `world_fold.rs:948`-`:949`.
pub const MIN_DELAYED_EVENT_ID: u32 = 100_000;
pub const MAX_DELAYED_EVENT_ID: u32 = 100_000 * 42_949;
/// `DOMAIN51_MAX_DELAYED_EVENT_DUE_HOURS` (`build_profile.rs:325`), read by the
/// fold at `world_fold.rs:958`.
pub const MAX_DELAYED_EVENT_DUE_HOURS: u32 = 500_000;
/// `MAX_COMPACT_OWNER_TAG_DB_ID` (`build_profile.rs`). A **fold** bound only
/// for kind-6 rows (`world_fold.rs::validate_scripted_state_row`); the payload authority applies it to
/// every country-scoped row, which is [`check_authority_payload`]'s business.
///
/// Was `2_048`, a guess at the vanilla-plus-DLC country count. ENGINE-STRUCTURAL
/// now: `id < *(int32*)(db + 0x0C)` with a signed-`int` size gives `i32::MAX - 1`.
/// Pinned by `tests/producer_consumer_admission_agreement.rs`.
pub const MAX_COUNTRY_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
/// `MAX_STATE_ID` (`scripted_state_compact.rs:62`) — authority-only, never a
/// fold bound. Was `65_535`, a guess; now the state id space's STRUCTURAL
/// width, proven from `ai_theatre_graph.cpp:0x86`
/// (`claim_39c989ce13504504353c`, dossier `runs/ia_province_caps/SWEEP.md`).
pub const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
/// `MAX_KEY_BYTES` / `MAX_KEY_TABLE_BYTES` (`scripted_state_compact.rs:60`-`:61`)
/// — authority-only.
pub const MAX_KEY_BYTES: usize = 512;
pub const MAX_KEY_TABLE_BYTES: usize = 8 * 1024 * 1024;

/// The two producer marker strings a badge row's key must carry to pass the
/// payload authority (`scripted_state_compact.rs:44`-`:45`, enforced at `:684`).
/// The FOLD requires nothing of a badge key, so these are not mapper gates.
pub const DELAYED_EVENTS_BADGE_KEY: &str = "root_unresolved:delayed_events";
pub const SCRIPTED_GUI_BADGE_KEY: &str = "root_unresolved:scripted_gui_open_state";

// --- §5 symbols --------------------------------------------------------------

/// The scripted key vocabulary, as a symbol table.
///
/// §5: "Country tags, equipment names, province and state ids are
/// content-defined. The stream carries manifest ids as varints, never text"
/// (`WIRE_FORMAT.md:346`-`:351`). A scripted variable or flag name —
/// `abda_convoy_escort_efficiency`, `sabotaged_coal` — is a mod-set content token
/// declared in script files, so it travels as a `ContentId` resolved through a
/// table the caller supplies, and because `ContentId` is a legal key kind
/// (`schema.rs:29`-`:31`) the text-keyed map becomes a `ContentId`-keyed set with
/// no new shape. **This is d29's rule and d29's implementation** (`d29.rs:205`);
/// see that module for the full argument.
///
/// It is **not** player-authored free text (the Scope stop): a script name comes
/// from content files, never from a text box, and the capture's whole 4,354-symbol
/// vocabulary is `[a-z0-9_^]` — the `^` being the game's array-element syntax.
///
/// The production assignment now lives in
/// `portable_content_manifest::symbol_manifest`; this domain-local mapper still
/// awaits the file-global resolver retrofit. [`Symbols::from_vocabulary`] uses a
/// measurement stand-in that is deterministic,
/// order-independent and derived only from the content set — the 0-based index
/// into the sorted, deduplicated list — and it is labelled a stand-in, not an
/// answer.
///
/// **The invariance claim is narrower than d29's, and the difference is
/// measured.** d29's vocabulary is under 128 symbols, so every assignment costs
/// one varint byte and its measurement is assignment-invariant. This capture's is
/// **4,354** symbols: ids `0..=127` are one-byte varints, `128..=16_383` are two,
/// so a different assignment moves which names pay the second byte. The
/// measurement therefore reports the stand-in's cost **and the exact band** over
/// all assignments of this vocabulary (`bin/bench_real_d51.rs`), which is
/// computable precisely because the symbol is never the leading key component —
/// see the module docs. §5's ids are file-global, so an assignment over the whole
/// content set can widen the varint further; that is claimed nowhere.
///
/// The badge-key marker strings ([`DELAYED_EVENTS_BADGE_KEY`]) ride the same
/// table. They are producer constants rather than mod content, and they are
/// disjoint from the script vocabulary by construction (a `:` cannot occur in a
/// script name); §4 declares no string kind, so a `ContentId` through this table
/// is the only lossless carrier either way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    /// Sorted and deduplicated; the id of a symbol is its index.
    vocabulary: Vec<String>,
}

impl Symbols {
    /// A table with no symbols. A row carrying a key is rejected against it —
    /// fail-closed, so an unresolvable symbol is a diagnosed error and never a
    /// record with the name quietly dropped.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from every key string the content set can produce. The input is
    /// sorted and deduplicated here, so callers may pass it in any order and two
    /// callers over the same content set always mint the same ids.
    pub fn from_vocabulary<I, S>(symbols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut vocabulary: Vec<String> = symbols.into_iter().map(Into::into).collect();
        vocabulary.sort();
        vocabulary.dedup();
        Self { vocabulary }
    }

    pub fn len(&self) -> usize {
        self.vocabulary.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vocabulary.is_empty()
    }

    /// The symbol id of `symbol`, or `None` if the content set does not contain
    /// it. `None` is an error at the boundary, never an `Unknown` value: the
    /// string WAS captured, so claiming it was not would be a false statement
    /// about the capture.
    pub fn id(&self, symbol: &str) -> Option<u64> {
        self.vocabulary
            .binary_search_by(|candidate| candidate.as_str().cmp(symbol))
            .ok()
            .map(|index| index as u64)
    }

    /// The symbol an id names, for a reader replaying the stream.
    pub fn symbol(&self, id: u64) -> Option<&str> {
        usize::try_from(id)
            .ok()
            .and_then(|index| self.vocabulary.get(index))
            .map(String::as_str)
    }
}

// --- Schema ------------------------------------------------------------------

pub fn schema() -> Schema {
    use field as f;

    // `scope_kind` is `Unsigned`, not `Enum`: §7.4 admits only Unsigned, Signed
    // and ContentId as key components (`schema.rs:29`-`:31`), and the scope is
    // part of the identity — the same name in the same country and in the world
    // scope are two different rows in two different fold accumulators. It is an
    // engine discriminator, not content (d20's `div_type` precedent).
    //
    // `owner_id` is `ContentId`. Its manifest domain is selected by
    // `scope_kind`, which is key component 0 — d28's decide-per-(field, kind)
    // rule, applied to a key: COUNTRY carries a `tag_db_id` and STATE a
    // `state_id`, and §5 names both as content-defined in as many words, so a
    // manifest must resolve it for the row to keep its meaning under another mod
    // set (d63's `tag_db_id` key and d52's `state_id` key are the precedents).
    // On GLOBAL the fold hard-enforces `owner_id == 0` (`world_fold.rs::validate_scripted_state_row`),
    // which is the "no owner" sentinel rather than a content reference. This is a
    // ContentId KEY — the heavier §13 retrofit class — and it is flagged as such
    // in the deliverable, twice over: the retrofit needs a scope-selected
    // resolver, not one table.
    let scope_and_owner = |scope_id: u32, owner_id: u32| {
        vec![
            Field::new(scope_id, Kind::Unsigned).key(0),
            Field::new(owner_id, Kind::ContentId).key(1),
        ]
    };

    let mut variable_fields = scope_and_owner(f::VARIABLE_SCOPE_KIND, f::VARIABLE_OWNER_ID);
    variable_fields.extend([
        Field::new(f::VARIABLE_NAME, Kind::ContentId).key(2),
        // Required and concrete: `validate_scripted_state_row` refuses a live
        // variable row whose `VALUE_PRESENT` gate is clear
        // (`world_fold.rs::validate_scripted_state_row`), so no folded row can lack it.
        // `Signed`, carried raw — see the module docs for why not `Fixed(5)`.
        Field::new(f::VARIABLE_VALUE_RAW, Kind::Signed),
    ]);

    let mut named_flag_fields = scope_and_owner(f::NAMED_FLAG_SCOPE_KIND, f::NAMED_FLAG_OWNER_ID);
    named_flag_fields.extend([
        Field::new(f::NAMED_FLAG_NAME, Kind::ContentId).key(2),
        // Unknown-capable, never optional and never nullable: the concept always
        // applies (a flag has a value and a set-date in the engine), and a clear
        // gate means the producer did not supply one — §3's "applicable, not
        // captured". No divisor: the fold stores these verbatim
        // (`ScriptFlagState`, `world_fold.rs:25501`-`:25504`).
        Field::new(f::NAMED_FLAG_VALUE_RAW, Kind::Signed).unknown_capable(),
        Field::new(f::NAMED_FLAG_DATE_HOUR_RAW, Kind::Signed).unknown_capable(),
    ]);

    let mut token_flag_fields = scope_and_owner(f::TOKEN_FLAG_SCOPE_KIND, f::TOKEN_FLAG_OWNER_ID);
    token_flag_fields.extend([
        // `Unsigned`, not `ContentId`. The token id is read verbatim from the
        // engine's own CFlag row word (`CFLAG_ROW_TOKEN_ID`,
        // `attach_log.rs:44505`-`:44508`); nothing on its read path hashes a
        // content-defined string, which is exactly d95's `type_id`/`stable_id`
        // ruling and d20's `div_type` precedent. It is a runtime interning
        // identity, and the fold keys its accumulator by it
        // (`scripted_world_token_only_flags: BTreeMap<u32, _>`,
        // `world_fold.rs:1690`).
        Field::new(f::TOKEN_FLAG_TOKEN_ID, Kind::Unsigned).key(2),
        Field::new(f::TOKEN_FLAG_VALUE_RAW, Kind::Signed).unknown_capable(),
        Field::new(f::TOKEN_FLAG_DATE_HOUR_RAW, Kind::Signed).unknown_capable(),
    ]);

    let mut badge_fields = scope_and_owner(f::BADGE_SCOPE_KIND, f::BADGE_OWNER_ID);
    badge_fields.extend([
        // The badge's `value_kind` (4 or 5) is its identity, so it is a key
        // component; `Unsigned` for the same reason `scope_kind` is.
        Field::new(f::BADGE_KIND, Kind::Unsigned).key(2),
        // Null-capable, not unknown-capable: the fold stores `Option<String>`
        // and publishes `"key": null` (`world_fold.rs:25523`-`:25530`), and the
        // string is a producer-chosen marker rather than a read that can fail,
        // so a clear gate is "this badge names nothing".
        Field::new(f::BADGE_KEY, Kind::ContentId).nullable(),
    ]);

    let delayed_event_fields = vec![
        // The fold keys this accumulator `(owner_id, combined_event_id,
        // scope_random)` (`world_fold.rs::apply_scripted_state_row`), so the port does.
        Field::new(f::DELAYED_OWNER_TAG_DB_ID, Kind::ContentId).key(0),
        // `ContentId`: the combined id is `namespace * 100_000 + index`
        // (`scripted_state_compact.rs:34`-`:36`) and an event namespace is
        // declared by content (`add_namespace`), so id 100_001 names whatever
        // event the loaded mod set defines there.
        Field::new(f::DELAYED_COMBINED_EVENT_ID, Kind::ContentId).key(1),
        // `Unsigned`: a runtime RNG seed pair, an instance identity that no
        // manifest resolves (d20's `div_id` flavour).
        Field::new(f::DELAYED_SCOPE_RANDOM, Kind::Unsigned).key(2),
        // Unknown on a clear gate: the collector badges it absent on an
        // out-of-bounds or torn read (`scripted_state_domain51_collector.rs:104`
        // -`:107`), which is §3's unknown (d63's ruling).
        Field::new(f::DELAYED_DUE_REMAINING_HOURS, Kind::Unsigned).unknown_capable(),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: VARIABLES,
                shape: Shape::KeyedSet,
                fields: variable_fields,
            },
            Collection {
                collection_id: NAMED_FLAGS,
                shape: Shape::KeyedSet,
                fields: named_flag_fields,
            },
            Collection {
                collection_id: TOKEN_FLAGS,
                shape: Shape::KeyedSet,
                fields: token_flag_fields,
            },
            Collection {
                collection_id: OPAQUE_BADGES,
                shape: Shape::KeyedSet,
                fields: badge_fields,
            },
            Collection {
                collection_id: DELAYED_EVENTS,
                shape: Shape::KeyedSet,
                fields: delayed_event_fields,
            },
        ],
        event_part: None,
    }
}

/// The fold's own kind-1 projection, pinned in exactly one place so no consumer
/// re-derives the divisor (`world_fold.rs::apply_scripted_state_row`). Kinds 2 and 3 have no
/// divisor and must never be passed through this.
pub fn variable_display_value(value_raw: i64, fixed_scale: u32) -> f64 {
    value_raw as f64 / fixed_scale as f64
}

// --- Header and payload split ------------------------------------------------

/// The 64-byte payload header (`world_fold.rs::fold_scripted_state`,
/// `scripted_state_compact.rs:207`-`:222`). Every word is parsed; the fold reads
/// only `row_count`, `key_bytes_len`, `status_flags`, `fixed_scale` and
/// `schema_version`, and the rest are reported, never enforced here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub key_bytes_len: u32,
    /// Parsed and reported, never enforced by [`split_payload`]: this fold never
    /// reads it. The payload authority bounds it (`<= row_count`), which is
    /// [`check_authority_payload`]'s business — the d65 rule.
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_scopes: u32,
    pub measured_scopes: u32,
    pub rejected_scopes: u32,
    pub variable_rows: u32,
    pub flag_rows: u32,
    pub token_only_rows: u32,
    pub fixed_scale: u32,
    pub schema_version: u32,
    pub reserved0: u32,
    pub reserved1: u32,
}

impl PayloadHeader {
    pub fn is_delta(&self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }
    pub fn names_partial(&self) -> bool {
        self.status_flags & STATUS_FLAG_NAMES_PARTIAL != 0
    }
}

fn u32_at(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("4 bytes"))
}

fn i32_at(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("4 bytes"))
}

fn i64_at(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(payload[offset..offset + 8].try_into().expect("8 bytes"))
}

/// Read the header alone, and apply the two checks the fold runs BEFORE it
/// classifies the status word: the header length (`world_fold.rs::fold_scripted_state`) and the
/// schema version (`world_fold.rs::fold_scripted_state`).
///
/// The split matters and is this fold's own call order. Everything else the fold
/// checks — the exact payload length and `fixed_scale` — runs AFTER both the
/// failure-status exit (`world_fold.rs::fold_scripted_state`) and the delta-before-baseline exit
/// (`world_fold.rs::fold_scripted_state`), so enforcing them here would refuse records the recorder
/// accepts and folds nothing from.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "scripted_state payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        key_bytes_len: u32_at(payload, 12),
        changed_rows: u32_at(payload, 16),
        status_flags: u32_at(payload, 20),
        observed_scopes: u32_at(payload, 24),
        measured_scopes: u32_at(payload, 28),
        rejected_scopes: u32_at(payload, 32),
        variable_rows: u32_at(payload, 36),
        flag_rows: u32_at(payload, 40),
        token_only_rows: u32_at(payload, 44),
        fixed_scale: u32_at(payload, 48),
        schema_version: u32_at(payload, 52),
        reserved0: u32_at(payload, 56),
        reserved1: u32_at(payload, 60),
    };
    if header.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "scripted_state schema version {} != {SCHEMA_VERSION}",
            header.schema_version
        ));
    }
    Ok(header)
}

/// Split a payload into its header, its fixed-width rows and its key table,
/// applying exactly the structural checks the fold applies after its two early
/// exits: the EXACT length identity `64 + row_count*48 + key_bytes_len`
/// (`world_fold.rs:18944`-`:18952`) and `ensure_fixed_scale` (`:18954`).
///
/// `changed_rows` is NOT compared to anything: this fold never reads the word
/// (the recipe's d65 rule — the test is whether YOUR fold reads it).
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = peek_header(payload)?;
    // Checked arithmetic: `row_count` is bytes off a disk, and `row_count *
    // ROW_LEN` overflows `usize` for a large enough word. The fold uses
    // `saturating_mul`/`saturating_add`, which cannot produce a length that
    // matches a real payload, so a checked error here refuses exactly the
    // payloads the fold refuses.
    let rows_len = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .ok_or_else(|| format!("scripted_state row_count {} overflows", header.row_count))?;
    let key_start = HEADER_LEN
        .checked_add(rows_len)
        .ok_or_else(|| "scripted_state payload length overflows".to_string())?;
    let expected_len = key_start
        .checked_add(header.key_bytes_len as usize)
        .ok_or_else(|| "scripted_state key table length overflows".to_string())?;
    if payload.len() != expected_len {
        return Err(format!(
            "scripted_state len {} != {expected_len}",
            payload.len()
        ));
    }
    if header.fixed_scale != FIXED_SCALE_RAW {
        return Err(format!(
            "scripted_state fixed_scale {} != {FIXED_SCALE_RAW}",
            header.fixed_scale
        ));
    }
    Ok((
        header,
        &payload[HEADER_LEN..key_start],
        &payload[key_start..expected_len],
    ))
}

// --- Rows --------------------------------------------------------------------

/// One decoded row, in the shape `ScriptedStateRow` has inside the fold
/// (`world_fold.rs:19042`-`:19054`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedRow {
    pub scope_kind: u32,
    pub owner_id: u32,
    /// The value kind with the tombstone bit already masked off.
    pub value_kind: u32,
    pub key: Option<String>,
    pub token_id: Option<u32>,
    pub date_hour_raw: Option<i32>,
    pub value_raw: Option<i64>,
    pub deleted: bool,
    pub delayed_due_hours: Option<u32>,
}

impl ParsedRow {
    /// Which collection this row belongs to. Infallible on a row [`parse_row`]
    /// accepted, because the value kind was validated there.
    pub fn collection_id(&self) -> core::result::Result<u32, String> {
        Ok(match self.value_kind {
            VALUE_VARIABLE_FIXED_I64 => VARIABLES,
            VALUE_FLAG_NAMED => NAMED_FLAGS,
            VALUE_FLAG_TOKEN_ONLY => TOKEN_FLAGS,
            VALUE_DELAYED_EVENT_OPAQUE | VALUE_SCRIPTED_GUI_OPAQUE => OPAQUE_BADGES,
            VALUE_DELAYED_EVENT => DELAYED_EVENTS,
            other => return Err(format!("scripted_state unsupported value_kind {other}")),
        })
    }
}

/// Decode and validate one 48-byte row against its payload's key table, exactly
/// as the fold's row loop does (`world_fold.rs:18957`-`:19062`) followed by
/// `validate_scripted_state_row` (`world_fold.rs::validate_scripted_state_row`).
///
/// `is_delta` is a typed input because one of the fold's row rules is
/// frame-relative: a tombstone carried in the VALUE-KIND high bit outside a delta
/// frame is a hard error (`:19055`-`:19060`). Note that the same rule does **not**
/// apply to the `ROW_FLAG_DELETED` form — the fold tests `raw_value_kind` alone —
/// and this port mirrors that asymmetry rather than tidying it.
///
/// Total: no byte string of any length panics.
pub fn parse_row(
    row: &[u8],
    key_table: &[u8],
    is_delta: bool,
) -> core::result::Result<ParsedRow, String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "scripted_state row is {} byte(s), expected {ROW_LEN}",
            row.len()
        ));
    }
    let scope_kind = u32_at(row, 0);
    let owner_id = u32_at(row, 4);
    let raw_value_kind = u32_at(row, 8);
    let value_kind = raw_value_kind & !VALUE_KIND_DELETED;
    let flags = u32_at(row, 12);
    if flags & !ROW_KNOWN_FLAGS != 0 {
        return Err(format!("scripted_state row unknown flags {flags}"));
    }
    let key = if flags & ROW_FLAG_KEY_PRESENT != 0 {
        let key_offset = u32_at(row, 16) as usize;
        let key_len = u32_at(row, 20) as usize;
        let Some(bytes) = key_table.get(key_offset..key_offset.saturating_add(key_len)) else {
            return Err("scripted_state row key range out of bounds".to_string());
        };
        Some(
            std::str::from_utf8(bytes)
                .map_err(|e| format!("scripted_state row key utf8: {e}"))?
                .to_string(),
        )
    } else {
        // The fold checks the LENGTH word only, not the offset word
        // (`world_fold.rs::fold_scripted_state`).
        if u32_at(row, 20) != 0 {
            return Err("scripted_state row has key_len without key flag".to_string());
        }
        None
    };
    let has_delayed_due = flags & ROW_FLAG_DELAYED_DUE_PRESENT != 0;
    let reserved0 = u32_at(row, 40);
    let reserved1 = u32_at(row, 44);
    let token_id_raw = u32_at(row, 24);
    let date_hour_raw = i32_at(row, 28);
    let value_raw = i64_at(row, 32);
    if !has_delayed_due && reserved0 != 0 {
        return Err("scripted_state row has due hours without the due flag".to_string());
    }
    if reserved1 != 0 {
        return Err("scripted_state row reserved field is nonzero".to_string());
    }
    let deleted = raw_value_kind & VALUE_KIND_DELETED != 0 || flags & ROW_FLAG_DELETED != 0;
    if deleted {
        let allowed_flags = match value_kind {
            VALUE_VARIABLE_FIXED_I64 | VALUE_FLAG_NAMED => {
                ROW_FLAG_KEY_PRESENT | ROW_FLAG_DELETED
            }
            VALUE_FLAG_TOKEN_ONLY => ROW_FLAG_TOKEN_PRESENT | ROW_FLAG_DELETED,
            _ => 0,
        };
        if flags & !allowed_flags != 0 {
            return Err("scripted_state tombstone row has live flags".to_string());
        }
        if matches!(
            value_kind,
            VALUE_VARIABLE_FIXED_I64 | VALUE_FLAG_NAMED
        ) && token_id_raw != 0
        {
            return Err("scripted_state tombstone row has live token_id".to_string());
        }
        if date_hour_raw != 0 || value_raw != 0 || reserved0 != 0 || reserved1 != 0 {
            return Err("scripted_state tombstone row has live fields".to_string());
        }
    }
    let parsed = ParsedRow {
        scope_kind,
        owner_id,
        value_kind,
        key,
        token_id: (flags & ROW_FLAG_TOKEN_PRESENT != 0).then_some(token_id_raw),
        date_hour_raw: (flags & ROW_FLAG_DATE_PRESENT != 0).then_some(date_hour_raw),
        value_raw: (flags & ROW_FLAG_VALUE_PRESENT != 0).then_some(value_raw),
        deleted,
        delayed_due_hours: has_delayed_due.then_some(reserved0),
    };
    // `world_fold.rs::fold_scripted_state`: only the VALUE-KIND tombstone form is
    // frame-gated. This sits between the row build and the row validation in the
    // fold, and so does here.
    if raw_value_kind & VALUE_KIND_DELETED != 0 && !is_delta {
        return Err("scripted_state row tombstone appears outside delta frame".to_string());
    }
    validate_parsed_row(&parsed)?;
    Ok(parsed)
}

/// `validate_scripted_state_row` (`world_fold.rs::validate_scripted_state_row`), verbatim in
/// order.
fn validate_parsed_row(row: &ParsedRow) -> core::result::Result<(), String> {
    if row.delayed_due_hours.is_some() && row.value_kind != VALUE_DELAYED_EVENT {
        return Err("scripted_state non-delayed-event row carries due hours".to_string());
    }
    match row.scope_kind {
        SCOPE_GLOBAL => {
            if row.owner_id != 0 {
                return Err("scripted_state global row has nonzero owner_id".to_string());
            }
        }
        SCOPE_COUNTRY | SCOPE_STATE => {
            if row.owner_id == 0 {
                return Err("scripted_state scoped row has zero owner_id".to_string());
            }
        }
        other => return Err(format!("scripted_state unsupported scope {other}")),
    }
    if row.deleted {
        match row.value_kind {
            VALUE_VARIABLE_FIXED_I64 | VALUE_FLAG_NAMED => {
                if row.key.as_ref().map_or(true, String::is_empty) {
                    return Err("scripted_state tombstone row missing key".to_string());
                }
                if row.token_id.is_some()
                    || row.date_hour_raw.is_some()
                    || row.value_raw.is_some()
                    || row.delayed_due_hours.is_some()
                {
                    return Err("scripted_state tombstone row has live fields".to_string());
                }
            }
            VALUE_FLAG_TOKEN_ONLY => {
                if row.key.is_some()
                    || row.token_id.is_none()
                    || row.date_hour_raw.is_some()
                    || row.value_raw.is_some()
                    || row.delayed_due_hours.is_some()
                {
                    return Err("scripted_state token tombstone row has live fields".to_string());
                }
            }
            _ => return Err("scripted_state unsupported tombstone value_kind".to_string()),
        }
        return Ok(());
    }
    match row.value_kind {
        VALUE_VARIABLE_FIXED_I64 => {
            if row.key.as_ref().map_or(true, String::is_empty) {
                return Err("scripted_state variable row missing key".to_string());
            }
            if row.value_raw.is_none() {
                return Err("scripted_state variable row missing value".to_string());
            }
        }
        VALUE_FLAG_NAMED => {
            if row.key.as_ref().map_or(true, String::is_empty) {
                return Err("scripted_state named flag row missing key".to_string());
            }
        }
        VALUE_FLAG_TOKEN_ONLY => {
            if row.token_id.is_none() {
                return Err("scripted_state token flag row missing token_id".to_string());
            }
        }
        VALUE_DELAYED_EVENT_OPAQUE | VALUE_SCRIPTED_GUI_OPAQUE => {}
        VALUE_DELAYED_EVENT => {
            if row.scope_kind != SCOPE_COUNTRY {
                return Err("scripted_state delayed-event row must be country-scoped".to_string());
            }
            if row.owner_id > MAX_COUNTRY_TAG_DB_ID {
                return Err("scripted_state delayed-event row owner tag out of range".to_string());
            }
            match row.token_id {
                Some(id) if (MIN_DELAYED_EVENT_ID..=MAX_DELAYED_EVENT_ID).contains(&id) => {}
                _ => return Err("scripted_state delayed-event row has invalid event id".to_string()),
            }
            // The scope tag rides the date slot and MUST equal the owner
            // (`world_fold.rs::validate_scripted_state_row`) — the identity that makes it a
            // derived value rather than a field.
            match row.date_hour_raw {
                Some(scope_tag)
                    if scope_tag > 0
                        && (scope_tag as u32) <= MAX_COUNTRY_TAG_DB_ID
                        && scope_tag as u32 == row.owner_id => {}
                _ => {
                    return Err(
                        "scripted_state delayed-event row scope tag invalid or != owner".to_string()
                    )
                }
            }
            if row.value_raw.is_none() {
                return Err("scripted_state delayed-event row missing scope random seed".to_string());
            }
            if row.key.is_some() {
                return Err("scripted_state delayed-event row must not carry a key".to_string());
            }
            if let Some(hours) = row.delayed_due_hours {
                if hours > MAX_DELAYED_EVENT_DUE_HOURS {
                    return Err("scripted_state delayed-event row due hours out of range".to_string());
                }
            }
        }
        other => return Err(format!("scripted_state unsupported value_kind {other}")),
    }
    Ok(())
}

// --- Keys --------------------------------------------------------------------

fn symbol_id(name: &str, symbols: &Symbols) -> core::result::Result<u64, String> {
    symbols
        .id(name)
        .ok_or_else(|| format!("scripted_state key {name:?} is not in the symbol table"))
}

/// The key of a `VARIABLES` row: `(scope_kind, owner_id, name_symbol)`.
pub fn key_from_variable_row(
    parsed: &ParsedRow,
    symbols: &Symbols,
) -> core::result::Result<Key, String> {
    let name = parsed
        .key
        .as_deref()
        .ok_or_else(|| "scripted_state variable row has no key".to_string())?;
    Ok(key(&[
        parsed.scope_kind as u64,
        parsed.owner_id as u64,
        symbol_id(name, symbols)?,
    ]))
}

/// The key of a `NAMED_FLAGS` row: `(scope_kind, owner_id, name_symbol)`.
pub fn key_from_named_flag_row(
    parsed: &ParsedRow,
    symbols: &Symbols,
) -> core::result::Result<Key, String> {
    let name = parsed
        .key
        .as_deref()
        .ok_or_else(|| "scripted_state named flag row has no key".to_string())?;
    Ok(key(&[
        parsed.scope_kind as u64,
        parsed.owner_id as u64,
        symbol_id(name, symbols)?,
    ]))
}

/// The key of a `TOKEN_FLAGS` row: `(scope_kind, owner_id, token_id)`.
pub fn key_from_token_flag_row(parsed: &ParsedRow) -> core::result::Result<Key, String> {
    let token = parsed
        .token_id
        .ok_or_else(|| "scripted_state token flag row has no token id".to_string())?;
    Ok(key(&[
        parsed.scope_kind as u64,
        parsed.owner_id as u64,
        token as u64,
    ]))
}

/// The key of an `OPAQUE_BADGES` row: `(scope_kind, owner_id, badge_kind)`.
pub fn key_from_badge_row(parsed: &ParsedRow) -> Key {
    key(&[
        parsed.scope_kind as u64,
        parsed.owner_id as u64,
        parsed.value_kind as u64,
    ])
}

/// The key of a `DELAYED_EVENTS` row: the fold's own accumulator key
/// `(owner_id, combined_event_id, scope_random)` (`world_fold.rs::apply_scripted_state_row`).
pub fn key_from_delayed_event_row(parsed: &ParsedRow) -> core::result::Result<Key, String> {
    let event_id = parsed
        .token_id
        .ok_or_else(|| "scripted_state delayed-event row has no event id".to_string())?;
    let seed = parsed
        .value_raw
        .ok_or_else(|| "scripted_state delayed-event row has no scope random seed".to_string())?;
    Ok(key(&[
        parsed.owner_id as u64,
        event_id as u64,
        seed as u64,
    ]))
}

/// The key of any row, dispatched on its value kind. The single derivation every
/// caller shares — the measurement bin, the tests and any future consumer.
pub fn key_from_row(parsed: &ParsedRow, symbols: &Symbols) -> core::result::Result<Key, String> {
    match parsed.collection_id()? {
        VARIABLES => key_from_variable_row(parsed, symbols),
        NAMED_FLAGS => key_from_named_flag_row(parsed, symbols),
        TOKEN_FLAGS => key_from_token_flag_row(parsed),
        OPAQUE_BADGES => Ok(key_from_badge_row(parsed)),
        DELAYED_EVENTS => key_from_delayed_event_row(parsed),
        other => Err(format!("scripted_state unknown collection {other}")),
    }
}

// --- Records -----------------------------------------------------------------

fn check_collection(collection: &Collection, expected: u32) -> core::result::Result<(), String> {
    if collection.collection_id != expected {
        return Err(format!(
            "scripted_state record built for collection {} but this mapper is collection {expected}",
            collection.collection_id
        ));
    }
    Ok(())
}

/// One `VARIABLES` row onto its stored `Record`.
///
/// `frame_tick` is §6's derived-field input; this domain declares no derived
/// field, so it is named and ignored — every domain's mapper has one shape.
pub fn record_from_variable_row(
    collection: &Collection,
    parsed: &ParsedRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, VARIABLES)?;
    if parsed.deleted {
        return Err("scripted_state variable tombstone is a removal, not a record".to_string());
    }
    let value_raw = parsed
        .value_raw
        .ok_or_else(|| "scripted_state variable row missing value".to_string())?;
    Ok(Record::new(vec![Value::Signed(value_raw)]))
}

/// One `NAMED_FLAGS` row onto its stored `Record`.
pub fn record_from_named_flag_row(
    collection: &Collection,
    parsed: &ParsedRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, NAMED_FLAGS)?;
    if parsed.deleted {
        return Err("scripted_state named flag tombstone is a removal, not a record".to_string());
    }
    Ok(Record::new(vec![
        flag_value(parsed.value_raw),
        flag_date(parsed.date_hour_raw),
    ]))
}

/// One `TOKEN_FLAGS` row onto its stored `Record`.
pub fn record_from_token_flag_row(
    collection: &Collection,
    parsed: &ParsedRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, TOKEN_FLAGS)?;
    if parsed.deleted {
        return Err("scripted_state token flag tombstone is a removal, not a record".to_string());
    }
    Ok(Record::new(vec![
        flag_value(parsed.value_raw),
        flag_date(parsed.date_hour_raw),
    ]))
}

/// A clear `VALUE_PRESENT` gate on a flag row is §3's **unknown** — see the module
/// docs. Carried raw: the fold applies no divisor to a flag value.
fn flag_value(value_raw: Option<i64>) -> Value {
    match value_raw {
        Some(raw) => Value::Signed(raw),
        None => Value::Unknown,
    }
}

fn flag_date(date_hour_raw: Option<i32>) -> Value {
    match date_hour_raw {
        Some(raw) => Value::Signed(raw as i64),
        None => Value::Unknown,
    }
}

/// One `OPAQUE_BADGES` row onto its stored `Record`. `symbols` is the §5 resolver
/// (step 3's typed input): the badge key is text, so there is no raw id to cast.
pub fn record_from_badge_row(
    collection: &Collection,
    parsed: &ParsedRow,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, OPAQUE_BADGES)?;
    // A badge row can never be a tombstone: `validate_scripted_state_row` refuses
    // one (`world_fold.rs::validate_scripted_state_row`). Checked rather than assumed.
    if parsed.deleted {
        return Err("scripted_state badge row cannot be a tombstone".to_string());
    }
    let badge_key = match parsed.key.as_deref() {
        Some(name) => Value::ContentId(symbol_id(name, symbols)?),
        None => Value::Null,
    };
    Ok(Record::new(vec![badge_key]))
}

/// One `DELAYED_EVENTS` row onto its stored `Record`.
pub fn record_from_delayed_event_row(
    collection: &Collection,
    parsed: &ParsedRow,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    check_collection(collection, DELAYED_EVENTS)?;
    if parsed.deleted {
        return Err("scripted_state delayed-event row cannot be a tombstone".to_string());
    }
    Ok(Record::new(vec![match parsed.delayed_due_hours {
        Some(hours) => Value::Unsigned(hours as u64),
        None => Value::Unknown,
    }]))
}

/// The record of any row, dispatched on its value kind. `collection` must be the
/// one [`ParsedRow::collection_id`] names.
pub fn record_from_parsed_row(
    collection: &Collection,
    parsed: &ParsedRow,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    match parsed.collection_id()? {
        VARIABLES => record_from_variable_row(collection, parsed, frame_tick),
        NAMED_FLAGS => record_from_named_flag_row(collection, parsed, frame_tick),
        TOKEN_FLAGS => record_from_token_flag_row(collection, parsed, frame_tick),
        OPAQUE_BADGES => record_from_badge_row(collection, parsed, frame_tick, symbols),
        DELAYED_EVENTS => record_from_delayed_event_row(collection, parsed, frame_tick),
        other => Err(format!("scripted_state unknown collection {other}")),
    }
}

/// One raw 48-byte row onto its stored `Record`.
///
/// The three fixed arguments plus this fold's own typed inputs: `key_table`,
/// because a row's key is a `(offset, len)` slice of the payload's trailing
/// string table and the row alone cannot resolve it (d35's `RowScales` shape);
/// `is_delta`, because one row rule is frame-relative ([`parse_row`]); and
/// `symbols`, §5's resolver (d29's shape).
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    key_table: &[u8],
    is_delta: bool,
    frame_tick: u64,
    symbols: &Symbols,
) -> core::result::Result<Record, String> {
    let parsed = parse_row(row, key_table, is_delta)?;
    record_from_parsed_row(collection, &parsed, frame_tick, symbols)
}

// --- The frame-level fold ----------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// A non-delta frame: all five accumulators were cleared and rebuilt.
    Baseline,
    /// A delta frame: three accumulators were patched, two were replaced.
    Delta,
    /// `DELTA_DATA` before any baseline (`world_fold.rs:18937`-`:18943` →
    /// `reject_delta_before_baseline`, `world_fold.rs::reject_delta_before_baseline`). The fold marks the
    /// domain-hour unknown and returns before the payload-length check, the
    /// `fixed_scale` check and the whole row loop; nothing is written. The
    /// recorder then publishes the WHOLE d51 surface as `Null` for that hour —
    /// world (`:3872`-`:3902`), country (`:5175` → `mask_scripted_country_extra`,
    /// `world_fold.rs::mask_scripted_country_extra`) and state (`:5393`-`:5397`) — so unlike d52 this really is an
    /// excluded hour with nothing of the domain surviving it.
    DeltaBeforeBaseline,
}

/// What [`fold_input`] hands its two callers: either the fold's early exit, or a
/// validated header and row list.
enum FoldInput {
    DeltaBeforeBaseline,
    Rows(PayloadHeader, Vec<ParsedRow>),
}

/// The shared prefix of [`State::fold_payload`] and [`KeyScanner::scan_payload`],
/// in the fold's own order — so the two passes of a two-pass harness cannot
/// disagree about whether a payload is foldable.
///
/// Order is the point. `reject_delta_before_baseline` runs BEFORE the payload
/// length and `fixed_scale` checks (`world_fold.rs:18937` then `:18944`), so a
/// delta-before-baseline record with a malformed body is an excluded hour and not
/// a stream rejection.
fn fold_input(
    payload: &[u8],
    baseline_seen: bool,
) -> core::result::Result<FoldInput, String> {
    let header = peek_header(payload)?;
    if header.is_delta() && !baseline_seen {
        return Ok(FoldInput::DeltaBeforeBaseline);
    }
    let (header, rows, key_table) = split_payload(payload)?;
    let is_delta = header.is_delta();
    let mut parsed = Vec::with_capacity(rows.len() / ROW_LEN);
    for row in rows.chunks_exact(ROW_LEN) {
        parsed.push(parse_row(row, key_table, is_delta)?);
    }
    // The one divergence from the fold, and the only one: the badge accumulator
    // is a `Vec` the fold pushes to, so two badge rows with one identity coexist
    // there and a keyed set would silently keep one. See the module docs for the
    // producer evidence that makes this input unreachable in production.
    let mut badge_identities: BTreeSet<(u32, u32, u32)> = BTreeSet::new();
    for row in &parsed {
        if matches!(
            row.value_kind,
            VALUE_DELAYED_EVENT_OPAQUE | VALUE_SCRIPTED_GUI_OPAQUE
        ) && !badge_identities.insert((row.scope_kind, row.owner_id, row.value_kind))
        {
            return Err(format!(
                "scripted_state carries two opaque badge rows with identity \
                 (scope {}, owner {}, kind {}); the fold's badge accumulator is a Vec \
                 and a keyed set cannot hold both",
                row.scope_kind, row.owner_id, row.value_kind
            ));
        }
    }
    Ok(FoldInput::Rows(header, parsed))
}

/// The five per-hour keyed states d51 folds into, plus the one bit of fold state
/// the payload grammar depends on.
///
/// One record is not one state: three of the five collections are
/// baseline-plus-delta, so an hour's content is the whole history of the stream up
/// to that hour. This is the recipe's fourth public boundary, and it exists so the
/// measurement bin, the tests and any future caller share ONE replay of the
/// shipping layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub variables: KeyedState,
    pub named_flags: KeyedState,
    pub token_flags: KeyedState,
    /// Full replace on EVERY frame (`world_fold.rs::fold_scripted_state`).
    pub opaque_badges: KeyedState,
    /// Full replace on EVERY frame (`world_fold.rs::fold_scripted_state`).
    pub delayed_events: KeyedState,
    /// `delta_baselines_seen.contains(&51)` (`world_fold.rs::fold_scripted_state`).
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
    pub fn collections(&self) -> [&KeyedState; COLLECTION_COUNT] {
        [
            &self.variables,
            &self.named_flags,
            &self.token_flags,
            &self.opaque_badges,
            &self.delayed_events,
        ]
    }

    /// Fold one d51 DATA payload exactly as `fold_scripted_state` folds it
    /// (`world_fold.rs::fold_scripted_state`), in the fold's own order.
    ///
    /// The caller owns the **failure-status** exclusion, because the predicate is
    /// `compact_contract`'s
    /// (`compact_status_failure_flags_after_expected_partial_tolerance`) and this
    /// crate's library half must neither depend on that crate nor copy its mask.
    /// The fold consults it at `world_fold.rs::fold_scripted_state` — after the header length and
    /// schema-version checks and before everything else — so a caller that gets
    /// the order right calls [`peek_header`], excludes on status, and only then
    /// calls this. Note d51's failure predicate is EXACTLY
    /// `COMPACT_STATUS_FAILURE_MASK` over the globally-interpretable status
    /// (`compact_status_for_global_interpretation` strips the three lane-local
    /// badges, of which bit 12 is otherwise `CONTROL_INVALID` and inside the
    /// failure mask), which is what the fold's hand-written expression at
    /// `world_fold.rs::fold_scripted_state` computes; `bin/bench_real_d51.rs` pins the agreement.
    ///
    /// The **delta-before-baseline** exclusion is handled here, because it needs
    /// no vocabulary this crate lacks.
    ///
    /// **Error contract: all-or-nothing.** The fold writes each row as it goes, so
    /// a fold that errors on row `k` has already run the clear and written rows
    /// `0..k`; a recorder in that state aborts the whole stream fold
    /// (`WorldFold::ingest_record` propagates the `Err`), so no consumer ever
    /// observes the half-written accumulator. This port therefore builds every key
    /// and record before the first mutation — the same observable behaviour and a
    /// safer one.
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        symbols: &Symbols,
    ) -> core::result::Result<FoldOutcome, String> {
        let (header, rows) = match fold_input(payload, self.baseline_seen)? {
            FoldInput::DeltaBeforeBaseline => return Ok(FoldOutcome::DeltaBeforeBaseline),
            FoldInput::Rows(header, rows) => (header, rows),
        };
        let delta = header.is_delta();

        // Build everything before mutating anything.
        let mut pending: Vec<(u32, Key, Option<Record>)> = Vec::with_capacity(rows.len());
        for row in &rows {
            let collection_id = row.collection_id()?;
            let collection = collection_of(schema, collection_id)?;
            let row_key = key_from_row(row, symbols)?;
            let record = if row.deleted {
                None
            } else {
                Some(record_from_parsed_row(collection, row, 0, symbols)?)
            };
            pending.push((collection_id, row_key, record));
        }

        if delta {
            // `world_fold.rs::fold_scripted_state` — the partial clear.
            self.opaque_badges.clear();
            self.delayed_events.clear();
        } else {
            // `world_fold.rs:19067` → `clear_scripted_state_accumulators`.
            self.variables.clear();
            self.named_flags.clear();
            self.token_flags.clear();
            self.opaque_badges.clear();
            self.delayed_events.clear();
        }

        for (collection_id, row_key, record) in pending {
            let target = match collection_id {
                VARIABLES => &mut self.variables,
                NAMED_FLAGS => &mut self.named_flags,
                TOKEN_FLAGS => &mut self.token_flags,
                OPAQUE_BADGES => &mut self.opaque_badges,
                DELAYED_EVENTS => &mut self.delayed_events,
                other => return Err(format!("scripted_state unknown collection {other}")),
            };
            match record {
                // The fold's `remove` on the row's own key.
                None => {
                    target.remove(&row_key);
                }
                // A plain insert, so a key repeated inside one frame is
                // last-writer-wins exactly as the fold's `insert` is. The payload
                // authority rejects the duplicate, which is
                // `check_authority_payload`'s job, not this one's.
                Some(record) => {
                    target.insert(row_key, record);
                }
            }
        }

        if delta {
            Ok(FoldOutcome::Delta)
        } else {
            // `world_fold.rs::fold_scripted_state`.
            self.baseline_seen = true;
            Ok(FoldOutcome::Baseline)
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

/// Pass 1 of a two-pass harness: the same fold decisions, on the ONE piece of
/// fold state that decides them, with no symbol resolved.
///
/// §5 forbids in-stream interning — the symbol table "must be fully read before
/// any seek" — so the vocabulary has to be collected over the whole validated
/// stream before any record is mapped, which means pass 1 cannot call a mapper at
/// all (d29's rule; `bin/bench_real_d29.rs:458`-`:465` is the worked example).
/// This type is d29's key-only mirror, and it is thinner than d29's because the
/// only fold state d51's classification depends on is `baseline_seen`: every other
/// decision is a pure function of the payload.
///
/// **The completeness proof pass 2 needs.** [`scan_payload`](Self::scan_payload)
/// appends the key string of EVERY row of every payload it folds, live and
/// tombstone alike, and it obtains those rows from [`fold_input`] — the same
/// function [`State::fold_payload`] obtains them from, over the same payload
/// bytes. The mapper resolves a symbol for exactly one thing: a row's own key
/// string. Therefore pass 2 can never need a symbol pass 1 did not see.
/// [`the_key_scanner_and_the_fold_agree_on_every_payload`] pins the agreement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyScanner {
    baseline_seen: bool,
}

/// What one payload contributed, for a harness's own accounting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyScan {
    pub rows: usize,
    pub tombstones: usize,
    /// Live (non-tombstone) rows per collection, in schema order.
    pub live_rows_by_collection: [usize; COLLECTION_COUNT],
}

impl KeyScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Validate and classify one payload exactly as [`State::fold_payload`] does,
    /// appending every row's key string to `vocabulary`.
    pub fn scan_payload(
        &mut self,
        payload: &[u8],
        vocabulary: &mut BTreeSet<String>,
    ) -> core::result::Result<(FoldOutcome, KeyScan), String> {
        let (header, rows) = match fold_input(payload, self.baseline_seen)? {
            FoldInput::DeltaBeforeBaseline => {
                return Ok((FoldOutcome::DeltaBeforeBaseline, KeyScan::default()))
            }
            FoldInput::Rows(header, rows) => (header, rows),
        };
        let mut scan = KeyScan {
            rows: rows.len(),
            ..KeyScan::default()
        };
        for row in &rows {
            if let Some(name) = row.key.as_deref() {
                if !vocabulary.contains(name) {
                    vocabulary.insert(name.to_string());
                }
            }
            if row.deleted {
                scan.tombstones += 1;
            } else {
                let index = match row.collection_id()? {
                    VARIABLES => 0,
                    NAMED_FLAGS => 1,
                    TOKEN_FLAGS => 2,
                    OPAQUE_BADGES => 3,
                    DELAYED_EVENTS => 4,
                    other => return Err(format!("scripted_state unknown collection {other}")),
                };
                scan.live_rows_by_collection[index] += 1;
            }
        }
        if header.is_delta() {
            Ok((FoldOutcome::Delta, scan))
        } else {
            self.baseline_seen = true;
            Ok((FoldOutcome::Baseline, scan))
        }
    }
}

// --- The lane contract's own decode (production's SECOND authority) -----------

/// Every check `scripted_state_flags_variables_live`'s payload decoder makes
/// (`lanes/scripted_state_flags_variables_live/scripted_state_compact.rs:203`
/// -`:645`, reached through `gate_domain51_payload` at
/// `scripted_state_domain51_gate.rs:13` and
/// `recorder_host::domain_data_authority:8081`), which production runs on **every**
/// DOMAIN_DATA record whatever its status; any complaint makes the record
/// `malformed_payload_records`, which `strict_stream_certification_verdict`
/// refuses.
///
/// It is deliberately separate from [`parse_row`] and the mappers, which mirror
/// the **fold** — far more permissive. The fold never checks `changed_rows`, the
/// three row-family counters, the scope bounds, key printability, `MAX_KEY_BYTES`,
/// a key flag with a zero length, a token or date word without its flag, the badge
/// key constants, the status/badge couplings, duplicate identities or the
/// canonical row order. Folding those into the mapper would refuse rows the
/// recorder folds — the data loss this recipe exists to prevent; leaving them out
/// of the harness would measure a stream the recorder refuses. So both exist, in
/// their own places.
///
/// Total: no payload of any length panics, and the first complaint wins (the lane
/// accumulates them; a harness only needs to know the stream is refused).
pub fn check_authority_payload(payload: &[u8]) -> core::result::Result<(), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "scripted state payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
    }
    let row_count = u32_at(payload, 8) as usize;
    let key_bytes_len = u32_at(payload, 12) as usize;
    let changed_rows = u32_at(payload, 16) as usize;
    let status_flags = u32_at(payload, 20);
    let observed_scopes = u32_at(payload, 24) as usize;
    let measured_scopes = u32_at(payload, 28) as usize;
    let variable_rows = u32_at(payload, 36) as usize;
    let flag_rows = u32_at(payload, 40) as usize;
    let token_only_rows = u32_at(payload, 44) as usize;
    let fixed_scale = u32_at(payload, 48);
    let schema_version = u32_at(payload, 52);
    let reserved0 = u32_at(payload, 56);
    let reserved1 = u32_at(payload, 60);

    if fixed_scale != FIXED_SCALE_RAW {
        return Err(format!(
            "scripted state fixed scale {fixed_scale} != {FIXED_SCALE_RAW}"
        ));
    }
    if schema_version != SCHEMA_VERSION {
        return Err(format!(
            "scripted state schema version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        return Err("scripted state header reserved fields are nonzero".to_string());
    }
    if key_bytes_len > MAX_KEY_TABLE_BYTES {
        return Err(format!(
            "scripted state key table is {key_bytes_len} byte(s), max {MAX_KEY_TABLE_BYTES}"
        ));
    }
    if changed_rows > row_count {
        return Err(format!(
            "scripted state changed_rows {changed_rows} exceeds row_count {row_count}"
        ));
    }
    if measured_scopes > observed_scopes {
        return Err(format!(
            "scripted state measured_scopes {measured_scopes} exceeds observed_scopes \
             {observed_scopes}"
        ));
    }
    let expected_len = row_count
        .checked_mul(ROW_LEN)
        .and_then(|rows| HEADER_LEN.checked_add(rows))
        .and_then(|start| start.checked_add(key_bytes_len))
        .ok_or_else(|| "scripted state payload length overflows".to_string())?;
    if payload.len() != expected_len {
        return Err(format!(
            "scripted state payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
    }

    let key_start = HEADER_LEN + row_count * ROW_LEN;
    let key_table = &payload[key_start..expected_len];
    let is_delta = status_flags & STATUS_DELTA_DATA != 0;
    let mut decoded_variable_rows = 0usize;
    let mut decoded_flag_rows = 0usize;
    let mut decoded_token_only_rows = 0usize;
    let mut decoded_delayed_event_opaque_rows = 0usize;
    let mut decoded_delayed_event_rows = 0usize;
    let mut decoded_scripted_gui_rows = 0usize;
    let mut seen_rows: BTreeSet<(u32, u32, u32, Vec<u8>, u32, i64)> = BTreeSet::new();
    let mut previous_canonical_order: Option<(u32, u32, u32, u32, Vec<u8>, i64, bool)> = None;

    for row_index in 0..row_count {
        let cursor = HEADER_LEN + row_index * ROW_LEN;
        let scope_kind = u32_at(payload, cursor);
        let owner_id = u32_at(payload, cursor + 4);
        let raw_value_kind = u32_at(payload, cursor + 8);
        let deleted_by_value_kind = raw_value_kind & VALUE_KIND_DELETED != 0;
        let value_kind = raw_value_kind & !VALUE_KIND_DELETED;
        let flags = u32_at(payload, cursor + 12);
        let key_offset = u32_at(payload, cursor + 16) as usize;
        let key_len = u32_at(payload, cursor + 20) as usize;
        let token_id = u32_at(payload, cursor + 24);
        let date_hour_raw = i32_at(payload, cursor + 28);
        let value_raw = i64_at(payload, cursor + 32);
        let reserved_row0 = u32_at(payload, cursor + 40);
        let reserved_row1 = u32_at(payload, cursor + 44);

        if flags & !ROW_KNOWN_FLAGS != 0 {
            return Err(format!(
                "scripted state row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = deleted_by_value_kind || flags & ROW_FLAG_DELETED != 0;
        authority_scope(row_index, scope_kind, owner_id)?;
        if !deleted {
            authority_value_kind(row_index, value_kind, flags, token_id)?;
        }
        if deleted_by_value_kind && !is_delta {
            return Err(format!(
                "scripted state row {row_index} tombstone appears outside delta frame"
            ));
        }
        let has_delayed_due = flags & ROW_FLAG_DELAYED_DUE_PRESENT != 0;
        if !has_delayed_due && reserved_row0 != 0 {
            return Err(format!(
                "scripted state row {row_index} has due hours without the due flag"
            ));
        }
        if reserved_row1 != 0 {
            return Err(format!(
                "scripted state row {row_index} reserved field is nonzero"
            ));
        }
        if has_delayed_due {
            if value_kind != VALUE_DELAYED_EVENT {
                return Err(format!(
                    "scripted state row {row_index} sets due-hours flag on a non-delayed-event row"
                ));
            }
            if reserved_row0 > MAX_DELAYED_EVENT_DUE_HOURS {
                return Err(format!(
                    "delayed-event row {row_index} due hours {reserved_row0} out of range"
                ));
            }
        }
        if deleted {
            authority_tombstone(
                row_index,
                value_kind,
                flags,
                token_id,
                date_hour_raw,
                value_raw,
                reserved_row0,
                reserved_row1,
            )?;
        }
        if flags & ROW_FLAG_KEY_PRESENT != 0 {
            if key_len == 0 {
                return Err(format!(
                    "scripted state row {row_index} sets key flag with zero key length"
                ));
            }
        } else if key_len != 0 {
            return Err(format!(
                "scripted state row {row_index} has key bytes without key flag"
            ));
        }
        if key_len > MAX_KEY_BYTES {
            return Err(format!(
                "scripted state row {row_index} key is {key_len} byte(s), max {MAX_KEY_BYTES}"
            ));
        }
        let mut key_bytes_for_row: Option<&[u8]> = None;
        if key_offset
            .checked_add(key_len)
            .map_or(true, |end| end > key_table.len())
        {
            return Err(format!(
                "scripted state row {row_index} key range is outside key table"
            ));
        } else if key_len != 0 {
            let key_bytes = &key_table[key_offset..key_offset + key_len];
            key_bytes_for_row = Some(key_bytes);
            if !is_printable_key(key_bytes) {
                return Err(format!(
                    "scripted state row {row_index} key has non-printable bytes"
                ));
            }
        }
        if flags & ROW_FLAG_TOKEN_PRESENT != 0 {
            if token_id == 0 {
                return Err(format!(
                    "scripted state row {row_index} sets token flag with token_id 0"
                ));
            }
        } else if token_id != 0 {
            return Err(format!(
                "scripted state row {row_index} has token_id without token flag"
            ));
        }
        if flags & ROW_FLAG_DATE_PRESENT == 0 && date_hour_raw != 0 {
            return Err(format!(
                "scripted state row {row_index} has date value without date flag"
            ));
        }

        if value_kind == VALUE_VARIABLE_FIXED_I64 {
            decoded_variable_rows += 1;
        }
        if value_kind == VALUE_FLAG_NAMED || value_kind == VALUE_FLAG_TOKEN_ONLY {
            decoded_flag_rows += 1;
        }
        if key_len == 0 && token_id != 0 && value_kind != VALUE_DELAYED_EVENT {
            decoded_token_only_rows += 1;
        }
        if value_kind == VALUE_DELAYED_EVENT_OPAQUE {
            decoded_delayed_event_opaque_rows += 1;
            if key_bytes_for_row != Some(DELAYED_EVENTS_BADGE_KEY.as_bytes()) {
                return Err(format!(
                    "scripted state delayed-event opaque row {row_index} has wrong badge key"
                ));
            }
        }
        if value_kind == VALUE_DELAYED_EVENT {
            decoded_delayed_event_rows += 1;
            if scope_kind != SCOPE_COUNTRY {
                return Err(format!(
                    "delayed-event row {row_index} must be country-scoped, got scope {scope_kind}"
                ));
            }
            if !(MIN_DELAYED_EVENT_ID..=MAX_DELAYED_EVENT_ID).contains(&token_id) {
                return Err(format!(
                    "delayed-event row {row_index} has out-of-range event id {token_id}"
                ));
            }
            if date_hour_raw <= 0 || date_hour_raw as u32 > MAX_COUNTRY_TAG_DB_ID {
                return Err(format!(
                    "delayed-event row {row_index} has invalid scope tag {date_hour_raw}"
                ));
            } else if date_hour_raw as u32 != owner_id {
                return Err(format!(
                    "delayed-event row {row_index} scope tag {date_hour_raw} != owner {owner_id}"
                ));
            }
        }
        if value_kind == VALUE_SCRIPTED_GUI_OPAQUE {
            decoded_scripted_gui_rows += 1;
            if key_bytes_for_row != Some(SCRIPTED_GUI_BADGE_KEY.as_bytes()) {
                return Err(format!(
                    "scripted state scripted-GUI opaque row {row_index} has wrong badge key"
                ));
            }
        }
        let seed_discriminator: i64 = if value_kind == VALUE_DELAYED_EVENT {
            value_raw
        } else {
            0
        };
        let identity_key = key_bytes_for_row.unwrap_or_default().to_vec();
        if !seen_rows.insert((
            scope_kind,
            owner_id,
            value_kind,
            identity_key.clone(),
            token_id,
            seed_discriminator,
        )) {
            return Err(format!(
                "scripted state row {row_index} duplicates scope/value/key identity"
            ));
        }
        let canonical_order = (
            canonical_value_family(value_kind),
            scope_kind,
            owner_id,
            token_id,
            identity_key,
            seed_discriminator,
            deleted,
        );
        if previous_canonical_order
            .as_ref()
            .is_some_and(|previous| previous >= &canonical_order)
        {
            return Err(format!(
                "scripted state row {row_index} is not in canonical semantic order"
            ));
        }
        previous_canonical_order = Some(canonical_order);
    }

    if decoded_variable_rows != variable_rows {
        return Err(format!(
            "scripted state variable_rows {variable_rows} != decoded {decoded_variable_rows}"
        ));
    }
    if decoded_flag_rows != flag_rows {
        return Err(format!(
            "scripted state flag_rows {flag_rows} != decoded {decoded_flag_rows}"
        ));
    }
    if decoded_token_only_rows != token_only_rows {
        return Err(format!(
            "scripted state token_only_rows {token_only_rows} != decoded {decoded_token_only_rows}"
        ));
    }
    if token_only_rows > 0 && status_flags & STATUS_FLAG_NAMES_PARTIAL == 0 {
        return Err(
            "scripted state token-only flag rows require STATUS_FLAG_NAMES_PARTIAL".to_string(),
        );
    }
    if decoded_delayed_event_opaque_rows > 0
        && status_flags & STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED == 0
    {
        return Err(
            "delayed-event opaque rows require STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED".to_string(),
        );
    }
    if status_flags & STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED != 0
        && decoded_delayed_event_opaque_rows == 0
    {
        return Err(
            "STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED set without delayed-event opaque badge row"
                .to_string(),
        );
    }
    if decoded_delayed_event_rows > 0 && decoded_delayed_event_opaque_rows > 0 {
        return Err(
            "measured delayed-event rows and the opaque badge are mutually exclusive".to_string(),
        );
    }
    if decoded_scripted_gui_rows > 0 && status_flags & STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED == 0 {
        return Err(
            "scripted-GUI opaque rows require STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED".to_string(),
        );
    }
    if status_flags & STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED != 0 && decoded_scripted_gui_rows == 0 {
        return Err(
            "STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED set without scripted-GUI opaque badge row"
                .to_string(),
        );
    }
    Ok(())
}

/// `canonical_value_family` (`scripted_state_compact.rs:640`-`:650`).
fn canonical_value_family(value_kind: u32) -> u32 {
    match value_kind {
        VALUE_VARIABLE_FIXED_I64 => 1,
        VALUE_FLAG_NAMED | VALUE_FLAG_TOKEN_ONLY => 2,
        VALUE_DELAYED_EVENT => 3,
        VALUE_DELAYED_EVENT_OPAQUE => 4,
        VALUE_SCRIPTED_GUI_OPAQUE => 5,
        other => 0x1000_0000u32.saturating_add(other),
    }
}

/// `is_printable_key` (`scripted_state_compact.rs:858`).
fn is_printable_key(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|b| (32..=126).contains(b))
}

/// `validate_scope` (`scripted_state_compact.rs:703`-`:735`).
fn authority_scope(
    row_index: usize,
    scope_kind: u32,
    owner_id: u32,
) -> core::result::Result<(), String> {
    match scope_kind {
        SCOPE_GLOBAL => {
            if owner_id != 0 {
                return Err(format!(
                    "scripted state global row {row_index} has owner_id {owner_id}"
                ));
            }
        }
        SCOPE_COUNTRY => {
            if owner_id == 0 || owner_id > MAX_COUNTRY_TAG_DB_ID {
                return Err(format!(
                    "scripted state country row {row_index} has invalid owner_id {owner_id}"
                ));
            }
        }
        SCOPE_STATE => {
            if owner_id == 0 || owner_id > MAX_STATE_ID {
                return Err(format!(
                    "scripted state state row {row_index} has invalid owner_id {owner_id}"
                ));
            }
        }
        other => {
            return Err(format!(
                "scripted state row {row_index} has unknown scope_kind {other}"
            ))
        }
    }
    Ok(())
}

/// `validate_value_kind` (`scripted_state_compact.rs:738`-`:809`).
fn authority_value_kind(
    row_index: usize,
    value_kind: u32,
    flags: u32,
    token_id: u32,
) -> core::result::Result<(), String> {
    match value_kind {
        VALUE_VARIABLE_FIXED_I64 => {
            if flags & ROW_FLAG_KEY_PRESENT == 0 || flags & ROW_FLAG_VALUE_PRESENT == 0 {
                return Err(format!(
                    "scripted state variable row {row_index} lacks key/value"
                ));
            }
            if token_id != 0 {
                return Err(format!(
                    "scripted state variable row {row_index} has token_id {token_id}"
                ));
            }
        }
        VALUE_FLAG_NAMED => {
            if flags & ROW_FLAG_KEY_PRESENT == 0 {
                return Err(format!(
                    "scripted state named flag row {row_index} lacks key"
                ));
            }
        }
        VALUE_FLAG_TOKEN_ONLY => {
            if flags & ROW_FLAG_TOKEN_PRESENT == 0 {
                return Err(format!("scripted state token flag row {row_index} lacks token"));
            }
        }
        VALUE_DELAYED_EVENT_OPAQUE | VALUE_SCRIPTED_GUI_OPAQUE => {
            if flags & ROW_FLAG_KEY_PRESENT == 0 {
                return Err(format!("scripted state opaque row {row_index} lacks key"));
            }
        }
        VALUE_DELAYED_EVENT => {
            if flags & ROW_FLAG_TOKEN_PRESENT == 0 {
                return Err(format!("delayed-event row {row_index} lacks event id token"));
            }
            if flags & ROW_FLAG_DATE_PRESENT == 0 {
                return Err(format!("delayed-event row {row_index} lacks scope tag"));
            }
            if flags & ROW_FLAG_VALUE_PRESENT == 0 {
                return Err(format!("delayed-event row {row_index} lacks scope random seed"));
            }
            if flags & ROW_FLAG_KEY_PRESENT != 0 {
                return Err(format!(
                    "delayed-event row {row_index} must not carry a string key"
                ));
            }
        }
        other => {
            return Err(format!(
                "scripted state row {row_index} has unknown value_kind {other}"
            ))
        }
    }
    Ok(())
}

/// `validate_tombstone_row` (`scripted_state_compact.rs:812`-`:855`).
#[allow(clippy::too_many_arguments)]
fn authority_tombstone(
    row_index: usize,
    value_kind: u32,
    flags: u32,
    token_id: u32,
    date_hour_raw: i32,
    value_raw: i64,
    reserved_row0: u32,
    reserved_row1: u32,
) -> core::result::Result<(), String> {
    let allowed_flags = match value_kind {
        VALUE_VARIABLE_FIXED_I64 | VALUE_FLAG_NAMED => ROW_FLAG_KEY_PRESENT | ROW_FLAG_DELETED,
        VALUE_FLAG_TOKEN_ONLY => ROW_FLAG_TOKEN_PRESENT | ROW_FLAG_DELETED,
        other => {
            return Err(format!(
                "scripted state tombstone row {row_index} has unsupported value_kind {other}"
            ))
        }
    };
    if flags & !allowed_flags != 0 {
        return Err(format!(
            "scripted state tombstone row {row_index} has live flags {flags}"
        ));
    }
    if matches!(value_kind, VALUE_VARIABLE_FIXED_I64 | VALUE_FLAG_NAMED) && token_id != 0 {
        return Err(format!(
            "scripted state tombstone row {row_index} has live token_id {token_id}"
        ));
    }
    if date_hour_raw != 0 || value_raw != 0 || reserved_row0 != 0 || reserved_row1 != 0 {
        return Err(format!(
            "scripted state tombstone row {row_index} has live fields"
        ));
    }
    Ok(())
}

// --- Test-facing payload builder ---------------------------------------------

/// One row as the collector packs it, for tests and fixtures.
///
/// This is the encode side of [`parse_row`] and is deliberately dumb: it writes
/// exactly what it is given, so a test can build a row the fold or the authority
/// refuses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RowSpec {
    pub scope_kind: u32,
    pub owner_id: u32,
    /// Written verbatim, tombstone bit included.
    pub raw_value_kind: u32,
    pub flags: u32,
    pub key: Option<String>,
    pub token_id: u32,
    pub date_hour_raw: i32,
    pub value_raw: i64,
    pub reserved0: u32,
    pub reserved1: u32,
}

/// Build a payload from a header status word and a row list, interning the keys
/// into a trailing string table exactly as the lane codec does.
///
/// The three row-family counters are filled the way the codec fills them
/// (`scripted_state_compact.rs:120`-`:135`) so a well-formed spec passes
/// [`check_authority_payload`]; a test that wants to break one overwrites the
/// bytes afterwards.
pub fn build_payload(status_flags: u32, rows: &[RowSpec]) -> Vec<u8> {
    let mut key_bytes: Vec<u8> = Vec::new();
    let mut row_keys: Vec<(u32, u32)> = Vec::with_capacity(rows.len());
    let mut variable_rows = 0u32;
    let mut flag_rows = 0u32;
    let mut token_only_rows = 0u32;
    for row in rows {
        let value_kind = row.raw_value_kind & !VALUE_KIND_DELETED;
        if value_kind == VALUE_VARIABLE_FIXED_I64 {
            variable_rows += 1;
        }
        if value_kind == VALUE_FLAG_NAMED || value_kind == VALUE_FLAG_TOKEN_ONLY {
            flag_rows += 1;
        }
        if row.key.is_none() && row.token_id != 0 && value_kind != VALUE_DELAYED_EVENT {
            token_only_rows += 1;
        }
        match row.key.as_deref() {
            Some(name) => {
                let offset = key_bytes.len() as u32;
                key_bytes.extend_from_slice(name.as_bytes());
                row_keys.push((offset, name.len() as u32));
            }
            None => row_keys.push((0, 0)),
        }
    }
    let mut payload = Vec::with_capacity(HEADER_LEN + rows.len() * ROW_LEN + key_bytes.len());
    payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(key_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes()); // observed_scopes
    payload.extend_from_slice(&1u32.to_le_bytes()); // measured_scopes
    payload.extend_from_slice(&0u32.to_le_bytes()); // rejected_scopes
    payload.extend_from_slice(&variable_rows.to_le_bytes());
    payload.extend_from_slice(&flag_rows.to_le_bytes());
    payload.extend_from_slice(&token_only_rows.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(payload.len(), HEADER_LEN);
    for (row, (key_offset, key_len)) in rows.iter().zip(row_keys) {
        payload.extend_from_slice(&row.scope_kind.to_le_bytes());
        payload.extend_from_slice(&row.owner_id.to_le_bytes());
        payload.extend_from_slice(&row.raw_value_kind.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&key_offset.to_le_bytes());
        payload.extend_from_slice(&key_len.to_le_bytes());
        payload.extend_from_slice(&row.token_id.to_le_bytes());
        payload.extend_from_slice(&row.date_hour_raw.to_le_bytes());
        payload.extend_from_slice(&row.value_raw.to_le_bytes());
        payload.extend_from_slice(&row.reserved0.to_le_bytes());
        payload.extend_from_slice(&row.reserved1.to_le_bytes());
    }
    payload.extend_from_slice(&key_bytes);
    payload
}

/// The vocabulary a capture-shaped payload needs, for tests and for a caller that
/// wants the symbol table of one payload rather than a whole stream.
pub fn payload_key_vocabulary(payload: &[u8]) -> core::result::Result<BTreeSet<String>, String> {
    let (header, rows, key_table) = split_payload(payload)?;
    let mut out = BTreeSet::new();
    for row in rows.chunks_exact(ROW_LEN) {
        let parsed = parse_row(row, key_table, header.is_delta())?;
        if let Some(name) = parsed.key {
            out.insert(name);
        }
    }
    Ok(out)
}

/// The number of distinct rows each collection holds, for a harness's reporting.
pub fn collection_lengths(state: &State) -> [usize; COLLECTION_COUNT] {
    let c = state.collections();
    [c[0].len(), c[1].len(), c[2].len(), c[3].len(), c[4].len()]
}

/// A `BTreeMap` view of the schema's collections by id, so a caller does not scan
/// the vector per row.
pub fn collections_by_id(schema: &Schema) -> BTreeMap<u32, &Collection> {
    schema
        .collections
        .iter()
        .map(|c| (c.collection_id, c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{
        apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    };
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection(id: u32) -> Collection {
        schema()
            .collections
            .into_iter()
            .find(|c| c.collection_id == id)
            .expect("collection exists")
    }

    fn symbols() -> Symbols {
        Symbols::from_vocabulary([
            "alpha",
            "beta",
            "gamma",
            "sabotaged_coal",
            DELAYED_EVENTS_BADGE_KEY,
            SCRIPTED_GUI_BADGE_KEY,
        ])
    }

    fn variable_row(scope: u32, owner: u32, name: &str, value: i64) -> RowSpec {
        RowSpec {
            scope_kind: scope,
            owner_id: owner,
            raw_value_kind: VALUE_VARIABLE_FIXED_I64,
            flags: ROW_FLAG_KEY_PRESENT | ROW_FLAG_VALUE_PRESENT,
            key: Some(name.to_string()),
            value_raw: value,
            ..RowSpec::default()
        }
    }

    fn named_flag_row(scope: u32, owner: u32, name: &str, value: i64, date: i32) -> RowSpec {
        RowSpec {
            scope_kind: scope,
            owner_id: owner,
            raw_value_kind: VALUE_FLAG_NAMED,
            flags: ROW_FLAG_KEY_PRESENT | ROW_FLAG_VALUE_PRESENT | ROW_FLAG_DATE_PRESENT,
            key: Some(name.to_string()),
            value_raw: value,
            date_hour_raw: date,
            ..RowSpec::default()
        }
    }

    fn token_flag_row(scope: u32, owner: u32, token: u32, value: i64, date: i32) -> RowSpec {
        RowSpec {
            scope_kind: scope,
            owner_id: owner,
            raw_value_kind: VALUE_FLAG_TOKEN_ONLY,
            flags: ROW_FLAG_TOKEN_PRESENT | ROW_FLAG_VALUE_PRESENT | ROW_FLAG_DATE_PRESENT,
            token_id: token,
            value_raw: value,
            date_hour_raw: date,
            ..RowSpec::default()
        }
    }

    fn badge_row(value_kind: u32, name: Option<&str>) -> RowSpec {
        RowSpec {
            scope_kind: SCOPE_GLOBAL,
            owner_id: 0,
            raw_value_kind: value_kind,
            flags: if name.is_some() { ROW_FLAG_KEY_PRESENT } else { 0 },
            key: name.map(str::to_string),
            ..RowSpec::default()
        }
    }

    fn delayed_event_row(owner: u32, event_id: u32, seed: i64, due: Option<u32>) -> RowSpec {
        RowSpec {
            scope_kind: SCOPE_COUNTRY,
            owner_id: owner,
            raw_value_kind: VALUE_DELAYED_EVENT,
            flags: ROW_FLAG_TOKEN_PRESENT
                | ROW_FLAG_DATE_PRESENT
                | ROW_FLAG_VALUE_PRESENT
                | if due.is_some() { ROW_FLAG_DELAYED_DUE_PRESENT } else { 0 },
            token_id: event_id,
            date_hour_raw: owner as i32,
            value_raw: seed,
            reserved0: due.unwrap_or(0),
            ..RowSpec::default()
        }
    }

    fn parse_one(payload: &[u8], index: usize) -> ParsedRow {
        let (header, rows, key_table) = split_payload(payload).expect("splits");
        parse_row(
            &rows[index * ROW_LEN..(index + 1) * ROW_LEN],
            key_table,
            header.is_delta(),
        )
        .expect("row parses")
    }

    // --- Step 8.1 ---

    #[test]
    fn the_schema_validates() {
        schema().validate(grid().log2()).expect("valid schema");
        schema()
            .validate(TickGrid::HOURLY.log2())
            .expect("valid on the hourly grid too");
    }

    // --- Step 8.5: shape and key ---

    #[test]
    fn every_collection_is_a_keyed_set_with_the_folds_own_key() {
        let expected: Vec<(u32, Vec<(u32, Kind)>)> = vec![
            (
                VARIABLES,
                vec![
                    (field::VARIABLE_SCOPE_KIND, Kind::Unsigned),
                    (field::VARIABLE_OWNER_ID, Kind::ContentId),
                    (field::VARIABLE_NAME, Kind::ContentId),
                ],
            ),
            (
                NAMED_FLAGS,
                vec![
                    (field::NAMED_FLAG_SCOPE_KIND, Kind::Unsigned),
                    (field::NAMED_FLAG_OWNER_ID, Kind::ContentId),
                    (field::NAMED_FLAG_NAME, Kind::ContentId),
                ],
            ),
            (
                TOKEN_FLAGS,
                vec![
                    (field::TOKEN_FLAG_SCOPE_KIND, Kind::Unsigned),
                    (field::TOKEN_FLAG_OWNER_ID, Kind::ContentId),
                    (field::TOKEN_FLAG_TOKEN_ID, Kind::Unsigned),
                ],
            ),
            (
                OPAQUE_BADGES,
                vec![
                    (field::BADGE_SCOPE_KIND, Kind::Unsigned),
                    (field::BADGE_OWNER_ID, Kind::ContentId),
                    (field::BADGE_KIND, Kind::Unsigned),
                ],
            ),
            (
                DELAYED_EVENTS,
                vec![
                    (field::DELAYED_OWNER_TAG_DB_ID, Kind::ContentId),
                    (field::DELAYED_COMBINED_EVENT_ID, Kind::ContentId),
                    (field::DELAYED_SCOPE_RANDOM, Kind::Unsigned),
                ],
            ),
        ];
        assert_eq!(schema().collections.len(), COLLECTION_COUNT);
        for (id, key_fields) in expected {
            let c = collection(id);
            assert_eq!(c.shape, Shape::KeyedSet, "collection {id}");
            let keys = c.key_fields();
            assert_eq!(keys.len(), key_fields.len(), "collection {id} key arity");
            for (position, (field_id, kind)) in key_fields.into_iter().enumerate() {
                assert_eq!(keys[position].field_id, field_id, "collection {id}");
                assert_eq!(keys[position].kind, kind, "collection {id}");
                assert_eq!(keys[position].key_position, Some(position as u32));
                // §11: a key member is concrete.
                assert!(!keys[position].optional);
                assert!(!keys[position].nullable);
                assert!(!keys[position].unknown_capable);
            }
        }
    }

    #[test]
    fn the_symbol_id_is_never_the_leading_key_component() {
        // The measurement's assignment band is computable only because a symbol
        // re-assignment cannot touch the delta-chained first component
        // (`body.rs:88`-`:116`). Two collections carry a symbol in their key and
        // in both it sits at position 2.
        for (id, symbol_field) in [
            (VARIABLES, field::VARIABLE_NAME),
            (NAMED_FLAGS, field::NAMED_FLAG_NAME),
        ] {
            let c = collection(id);
            let keys = c.key_fields();
            assert_eq!(keys[0].kind, Kind::Unsigned, "collection {id} leads on scope");
            assert_eq!(
                keys.iter().position(|f| f.field_id == symbol_field),
                Some(2),
                "collection {id}"
            );
        }
    }

    #[test]
    fn the_content_id_fields_are_exactly_the_swept_set() {
        // "This domain has no ContentId" is a sweep result, not a silence (d35's
        // rule); so is "these six". When §5's assignment rule lands, every one of
        // these changes, and the four that are KEY components are the heavier
        // retrofit class.
        let mut content_ids: Vec<(u32, u32, bool)> = Vec::new();
        for c in schema().collections {
            for f in &c.fields {
                if f.kind == Kind::ContentId {
                    content_ids.push((c.collection_id, f.field_id, f.is_key()));
                }
            }
        }
        assert_eq!(
            content_ids,
            vec![
                (VARIABLES, field::VARIABLE_OWNER_ID, true),
                (VARIABLES, field::VARIABLE_NAME, true),
                (NAMED_FLAGS, field::NAMED_FLAG_OWNER_ID, true),
                (NAMED_FLAGS, field::NAMED_FLAG_NAME, true),
                (TOKEN_FLAGS, field::TOKEN_FLAG_OWNER_ID, true),
                (OPAQUE_BADGES, field::BADGE_OWNER_ID, true),
                (OPAQUE_BADGES, field::BADGE_KEY, false),
                (DELAYED_EVENTS, field::DELAYED_OWNER_TAG_DB_ID, true),
                (DELAYED_EVENTS, field::DELAYED_COMBINED_EVENT_ID, true),
            ]
        );
    }

    // --- Step 8.4: quantization does not apply ---

    #[test]
    fn no_field_is_continuous_so_quantization_does_not_apply() {
        // The recipe's "quantization saves bytes" test is conditional on the
        // schema having a quantized field. This one has no `Continuous` field at
        // all: the only decimal-grid source is the kind-1 variable value, and it
        // is carried as the raw `i64` the fold itself stores (see the module
        // docs), because the fold's input domain is the whole of `i64` and
        // `Fixed(5)` stops reproducing the raw integer above roughly `2^51`.
        for c in schema().collections {
            for f in &c.fields {
                assert_ne!(
                    f.kind,
                    Kind::Continuous,
                    "collection {} field {}",
                    c.collection_id,
                    f.field_id
                );
                assert_eq!(f.quant, Quant::None, "collection {}", c.collection_id);
            }
        }
    }

    #[test]
    fn the_variable_display_value_is_the_folds_own_projection() {
        // The float the fold publishes is a pure function of the raw and the
        // header scale (`world_fold.rs::apply_scripted_state_row`), pinned in one place.
        assert_eq!(variable_display_value(5_250_000, FIXED_SCALE_RAW), 52.5);
        assert_eq!(variable_display_value(-100_000, FIXED_SCALE_RAW), -1.0);
        assert_eq!(variable_display_value(0, FIXED_SCALE_RAW), 0.0);
        // The capture's extremes, which `Signed` carries exactly and a
        // `Fixed(5)` field would too — the reason for the choice is the fold's
        // input domain, not this capture's range.
        assert_eq!(
            variable_display_value(100_000_000_000, FIXED_SCALE_RAW),
            1_000_000.0
        );
        // ...and the input domain that motivates it: a raw beyond 2^53 is not
        // representable as an f64 at all, so no float-valued carrier could hold
        // it. `Signed` does.
        let raw = (1i64 << 53) + 1;
        assert_ne!(raw as f64 as i64, raw);
        let record = record_from_variable_row(
            &collection(VARIABLES),
            &ParsedRow {
                scope_kind: SCOPE_GLOBAL,
                owner_id: 0,
                value_kind: VALUE_VARIABLE_FIXED_I64,
                key: Some("alpha".into()),
                token_id: None,
                date_hour_raw: None,
                value_raw: Some(raw),
                deleted: false,
                delayed_due_hours: None,
            },
            0,
        )
        .expect("maps");
        assert_eq!(record.values[0], Value::Signed(raw));
    }

    // --- Step 8.6: every header word, then every row offset ---

    #[test]
    fn peek_header_reads_every_word_at_its_own_offset() {
        let mut payload = vec![0u8; HEADER_LEN];
        payload[0..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        payload[8..12].copy_from_slice(&11u32.to_le_bytes());
        payload[12..16].copy_from_slice(&12u32.to_le_bytes());
        payload[16..20].copy_from_slice(&13u32.to_le_bytes());
        payload[20..24].copy_from_slice(&14u32.to_le_bytes());
        payload[24..28].copy_from_slice(&15u32.to_le_bytes());
        payload[28..32].copy_from_slice(&16u32.to_le_bytes());
        payload[32..36].copy_from_slice(&17u32.to_le_bytes());
        payload[36..40].copy_from_slice(&18u32.to_le_bytes());
        payload[40..44].copy_from_slice(&19u32.to_le_bytes());
        payload[44..48].copy_from_slice(&20u32.to_le_bytes());
        payload[48..52].copy_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        payload[52..56].copy_from_slice(&SCHEMA_VERSION.to_le_bytes());
        payload[56..60].copy_from_slice(&21u32.to_le_bytes());
        payload[60..64].copy_from_slice(&22u32.to_le_bytes());
        let header = peek_header(&payload).expect("header");
        assert_eq!(header.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(header.row_count, 11);
        assert_eq!(header.key_bytes_len, 12);
        assert_eq!(header.changed_rows, 13);
        assert_eq!(header.status_flags, 14);
        assert_eq!(header.observed_scopes, 15);
        assert_eq!(header.measured_scopes, 16);
        assert_eq!(header.rejected_scopes, 17);
        assert_eq!(header.variable_rows, 18);
        assert_eq!(header.flag_rows, 19);
        assert_eq!(header.token_only_rows, 20);
        assert_eq!(header.fixed_scale, FIXED_SCALE_RAW);
        assert_eq!(header.schema_version, SCHEMA_VERSION);
        assert_eq!(header.reserved0, 21);
        assert_eq!(header.reserved1, 22);
        // A short payload and a wrong version are the only two things `peek_header`
        // refuses, and it refuses them BEFORE any status classification.
        assert!(peek_header(&payload[..HEADER_LEN - 1]).is_err());
        let mut wrong = payload.clone();
        wrong[52..56].copy_from_slice(&1u32.to_le_bytes());
        assert!(peek_header(&wrong).is_err());
    }

    #[test]
    fn parse_row_reads_every_word_at_its_own_offset() {
        // Distinct values per slot, so a transposition cannot pass.
        let spec = RowSpec {
            scope_kind: SCOPE_COUNTRY,
            owner_id: 0x0000_0202,
            raw_value_kind: VALUE_FLAG_NAMED,
            flags: ROW_FLAG_KEY_PRESENT | ROW_FLAG_VALUE_PRESENT | ROW_FLAG_DATE_PRESENT,
            key: Some("gamma".into()),
            token_id: 0,
            date_hour_raw: -0x0304_0506,
            value_raw: -0x0102_0304_0506_0708,
            reserved0: 0,
            reserved1: 0,
        };
        let payload = build_payload(0, &[spec]);
        let parsed = parse_one(&payload, 0);
        assert_eq!(parsed.scope_kind, SCOPE_COUNTRY);
        assert_eq!(parsed.owner_id, 0x0000_0202);
        assert_eq!(parsed.value_kind, VALUE_FLAG_NAMED);
        assert_eq!(parsed.key.as_deref(), Some("gamma"));
        assert_eq!(parsed.token_id, None);
        assert_eq!(parsed.date_hour_raw, Some(-0x0304_0506));
        assert_eq!(parsed.value_raw, Some(-0x0102_0304_0506_0708));
        assert!(!parsed.deleted);
        assert_eq!(parsed.delayed_due_hours, None);

        // The token slot is at +24 and only read under its own flag.
        let payload = build_payload(0, &[token_flag_row(SCOPE_GLOBAL, 0, 0x0708_090a, 7, 9)]);
        let parsed = parse_one(&payload, 0);
        assert_eq!(parsed.token_id, Some(0x0708_090a));
        assert_eq!(parsed.value_raw, Some(7));
        assert_eq!(parsed.date_hour_raw, Some(9));

        // The due-hours slot is at +40 and only readable under bit 5.
        let payload = build_payload(
            0,
            &[delayed_event_row(7, MIN_DELAYED_EVENT_ID + 3, 0x1122_3344_5566_7788, Some(4_242))],
        );
        let parsed = parse_one(&payload, 0);
        assert_eq!(parsed.delayed_due_hours, Some(4_242));
        assert_eq!(parsed.token_id, Some(MIN_DELAYED_EVENT_ID + 3));
        assert_eq!(parsed.value_raw, Some(0x1122_3344_5566_7788));
        assert_eq!(parsed.date_hour_raw, Some(7));
    }

    #[test]
    fn the_reserved_slot_is_dual_purpose_and_reading_it_unconditionally_is_refused() {
        // The hazard: `reserved0` carries the delayed-event due hours under bit 5
        // and MUST be zero otherwise (`world_fold.rs::fold_scripted_state`). A mapper
        // that read it unconditionally would publish garbage on every other row.
        let mut spec = variable_row(SCOPE_GLOBAL, 0, "alpha", 5);
        spec.reserved0 = 9;
        let payload = build_payload(0, &[spec]);
        let (header, rows, key_table) = split_payload(&payload).expect("splits");
        assert!(parse_row(&rows[..ROW_LEN], key_table, header.is_delta()).is_err());
        // And bit 5 on a non-delayed-event row is refused by the row validator.
        let mut spec = variable_row(SCOPE_GLOBAL, 0, "alpha", 5);
        spec.flags |= ROW_FLAG_DELAYED_DUE_PRESENT;
        spec.reserved0 = 9;
        let payload = build_payload(0, &[spec]);
        let (header, rows, key_table) = split_payload(&payload).expect("splits");
        assert!(parse_row(&rows[..ROW_LEN], key_table, header.is_delta()).is_err());
    }

    #[test]
    fn split_payload_enforces_the_exact_length_and_the_fixed_scale() {
        let payload = build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]);
        assert!(split_payload(&payload).is_ok());
        // One trailing byte is not a shorter well-formed payload: the fold
        // requires equality (`world_fold.rs::fold_scripted_state`).
        let mut longer = payload.clone();
        longer.push(0);
        assert!(split_payload(&longer).is_err());
        assert!(split_payload(&payload[..payload.len() - 1]).is_err());
        let mut wrong_scale = payload.clone();
        wrong_scale[48..52].copy_from_slice(&32_768u32.to_le_bytes());
        assert!(split_payload(&wrong_scale).is_err());
        // `row_count` is bytes off a disk: the multiplication must not overflow.
        let mut huge = payload.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
    }

    // --- Step 8.3: every declared state, through the mapper, from raw bytes ---

    #[test]
    fn variable_rows_map_to_a_concrete_signed_value_including_a_real_zero() {
        let c = collection(VARIABLES);
        // The VARIABLES collection declares NO capability: its one value field is
        // required, concrete and never null or unknown, because
        // `validate_scripted_state_row` refuses a live variable row without
        // `VALUE_PRESENT` (`world_fold.rs::validate_scripted_state_row`). So there is no
        // Absent/Null/Unknown case to build here — a real zero and full values are
        // the whole state space.
        assert!(c.value_fields().all(|f| !f.optional && !f.nullable && !f.unknown_capable));
        for (raw, expected) in [
            (0i64, 0i64),
            (5_250_000, 5_250_000),
            (-1_073_741_821, -1_073_741_821),
            (100_000_000_000, 100_000_000_000),
            (i64::MIN, i64::MIN),
            (i64::MAX, i64::MAX),
        ] {
            let payload = build_payload(0, &[variable_row(SCOPE_STATE, 42, "alpha", raw)]);
            let parsed = parse_one(&payload, 0);
            let record = record_from_variable_row(&c, &parsed, 0).expect("maps");
            assert_eq!(record.values, vec![Value::Signed(expected)]);
            assert_eq!(
                key_from_variable_row(&parsed, &symbols()).expect("key"),
                key(&[SCOPE_STATE as u64, 42, symbols().id("alpha").unwrap()])
            );
        }
        // A row missing its value gate cannot reach the mapper — the row
        // validator refuses it first — so the state space really is closed.
        let mut spec = variable_row(SCOPE_GLOBAL, 0, "alpha", 1);
        spec.flags = ROW_FLAG_KEY_PRESENT;
        let payload = build_payload(0, &[spec]);
        let (header, rows, key_table) = split_payload(&payload).expect("splits");
        assert!(parse_row(&rows[..ROW_LEN], key_table, header.is_delta()).is_err());
    }

    #[test]
    fn flag_rows_map_a_clear_gate_to_unknown_and_a_set_one_to_the_raw_word() {
        for (collection_id, spec_full, spec_bare) in [
            (
                NAMED_FLAGS,
                named_flag_row(SCOPE_COUNTRY, 3, "beta", 0, 0),
                RowSpec {
                    scope_kind: SCOPE_COUNTRY,
                    owner_id: 3,
                    raw_value_kind: VALUE_FLAG_NAMED,
                    flags: ROW_FLAG_KEY_PRESENT,
                    key: Some("beta".into()),
                    ..RowSpec::default()
                },
            ),
            (
                TOKEN_FLAGS,
                token_flag_row(SCOPE_COUNTRY, 3, 77, 0, 0),
                RowSpec {
                    scope_kind: SCOPE_COUNTRY,
                    owner_id: 3,
                    raw_value_kind: VALUE_FLAG_TOKEN_ONLY,
                    flags: ROW_FLAG_TOKEN_PRESENT,
                    token_id: 77,
                    ..RowSpec::default()
                },
            ),
        ] {
            let c = collection(collection_id);
            // A REAL ZERO on both channels, never collapsed into unknown.
            let payload = build_payload(0, &[spec_full]);
            let parsed = parse_one(&payload, 0);
            let record = record_from_parsed_row(&c, &parsed, 0, &symbols()).expect("maps");
            assert_eq!(
                record.values,
                vec![Value::Signed(0), Value::Signed(0)],
                "collection {collection_id}"
            );
            // Both gates clear: Unknown, twice.
            let payload = build_payload(0, &[spec_bare]);
            let parsed = parse_one(&payload, 0);
            let record = record_from_parsed_row(&c, &parsed, 0, &symbols()).expect("maps");
            assert_eq!(
                record.values,
                vec![Value::Unknown, Value::Unknown],
                "collection {collection_id}"
            );
            record.validate(&c).expect("unknown is declared");
        }
        // ...and a full row carries both raw words, the date sign included.
        let payload = build_payload(0, &[token_flag_row(SCOPE_GLOBAL, 0, 5, 94_765_057, -3)]);
        let parsed = parse_one(&payload, 0);
        let record =
            record_from_token_flag_row(&collection(TOKEN_FLAGS), &parsed, 0).expect("maps");
        assert_eq!(
            record.values,
            vec![Value::Signed(94_765_057), Value::Signed(-3)]
        );
    }

    #[test]
    fn a_badge_row_without_a_key_maps_to_null_and_with_one_to_a_symbol() {
        let c = collection(OPAQUE_BADGES);
        let payload = build_payload(
            STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            &[badge_row(VALUE_DELAYED_EVENT_OPAQUE, Some(DELAYED_EVENTS_BADGE_KEY))],
        );
        let parsed = parse_one(&payload, 0);
        let record = record_from_badge_row(&c, &parsed, 0, &symbols()).expect("maps");
        assert_eq!(
            record.values,
            vec![Value::ContentId(
                symbols().id(DELAYED_EVENTS_BADGE_KEY).expect("in table")
            )]
        );
        assert_eq!(
            key_from_badge_row(&parsed),
            key(&[SCOPE_GLOBAL as u64, 0, VALUE_DELAYED_EVENT_OPAQUE as u64])
        );
        // The fold requires nothing of a badge key (`world_fold.rs::validate_scripted_state_row`), so a
        // keyless badge is a row it folds and this port maps — Null, because the
        // string is a producer marker and not a read that can fail.
        let payload = build_payload(
            STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[badge_row(VALUE_SCRIPTED_GUI_OPAQUE, None)],
        );
        let parsed = parse_one(&payload, 0);
        let record = record_from_badge_row(&c, &parsed, 0, &Symbols::empty()).expect("maps");
        assert_eq!(record.values, vec![Value::Null]);
        record.validate(&c).expect("null is declared");
    }

    #[test]
    fn a_delayed_event_row_maps_its_due_hours_and_unknown_when_the_gate_is_clear() {
        let c = collection(DELAYED_EVENTS);
        let payload = build_payload(0, &[delayed_event_row(9, MIN_DELAYED_EVENT_ID, 5, Some(0))]);
        let parsed = parse_one(&payload, 0);
        // A real ZERO due-hours, not collapsed into unknown.
        assert_eq!(
            record_from_delayed_event_row(&c, &parsed, 0).expect("maps").values,
            vec![Value::Unsigned(0)]
        );
        let payload = build_payload(
            0,
            &[delayed_event_row(9, MAX_DELAYED_EVENT_ID, -1, Some(MAX_DELAYED_EVENT_DUE_HOURS))],
        );
        let parsed = parse_one(&payload, 0);
        assert_eq!(
            record_from_delayed_event_row(&c, &parsed, 0).expect("maps").values,
            vec![Value::Unsigned(MAX_DELAYED_EVENT_DUE_HOURS as u64)]
        );
        // The seed rides the key as a u64, sign bit included.
        assert_eq!(
            key_from_delayed_event_row(&parsed).expect("key"),
            key(&[9, MAX_DELAYED_EVENT_ID as u64, u64::MAX])
        );
        let payload = build_payload(0, &[delayed_event_row(9, MIN_DELAYED_EVENT_ID, 5, None)]);
        let parsed = parse_one(&payload, 0);
        let record = record_from_delayed_event_row(&c, &parsed, 0).expect("maps");
        assert_eq!(record.values, vec![Value::Unknown]);
        record.validate(&c).expect("unknown is declared");
    }

    #[test]
    fn the_delayed_event_scope_tag_is_owner_and_is_not_re_encoded() {
        // A value the fold hard-enforces as a function of carried state is not a
        // field (`world_fold.rs::validate_scripted_state_row`). Enforced here, and absent from
        // the schema.
        let mut spec = delayed_event_row(9, MIN_DELAYED_EVENT_ID, 1, None);
        spec.date_hour_raw = 10;
        let payload = build_payload(0, &[spec]);
        let (header, rows, key_table) = split_payload(&payload).expect("splits");
        assert!(parse_row(&rows[..ROW_LEN], key_table, header.is_delta()).is_err());
        assert!(collection(DELAYED_EVENTS)
            .fields
            .iter()
            .all(|f| f.field_id != field::DELAYED_OWNER_TAG_DB_ID + 1000));
        assert_eq!(collection(DELAYED_EVENTS).value_fields().count(), 1);
    }

    #[test]
    fn a_mapper_refuses_a_record_built_for_the_wrong_collection() {
        // Every mapper here can produce a record whose value list is short, so a
        // misrouted row would come out silently wrong rather than failing
        // (`variant_upgrades.rs:633`'s rule).
        let payload = build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]);
        let parsed = parse_one(&payload, 0);
        assert!(record_from_variable_row(&collection(NAMED_FLAGS), &parsed, 0).is_err());
        assert!(record_from_named_flag_row(&collection(VARIABLES), &parsed, 0).is_err());
        assert!(record_from_token_flag_row(&collection(VARIABLES), &parsed, 0).is_err());
        assert!(record_from_badge_row(&collection(VARIABLES), &parsed, 0, &symbols()).is_err());
        assert!(record_from_delayed_event_row(&collection(VARIABLES), &parsed, 0).is_err());
    }

    #[test]
    fn an_unresolvable_symbol_is_an_error_and_never_a_dropped_name() {
        let payload = build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]);
        let parsed = parse_one(&payload, 0);
        assert!(key_from_variable_row(&parsed, &Symbols::empty()).is_err());
        assert!(key_from_variable_row(&parsed, &symbols()).is_ok());
    }

    // --- Step 8.2: byte-identical round trip ---

    #[test]
    fn a_full_row_of_every_collection_round_trips_byte_identically() {
        let schema = schema();
        let symbols = symbols();
        let payload = build_payload(
            STATUS_FLAG_NAMES_PARTIAL
                | STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
                | STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", -7),
                variable_row(SCOPE_COUNTRY, 4, "beta", 5_250_000),
                named_flag_row(SCOPE_STATE, 11, "gamma", 3, 60_832_555),
                token_flag_row(SCOPE_COUNTRY, 4, 48_293, 1, 60_832_556),
                badge_row(VALUE_DELAYED_EVENT_OPAQUE, Some(DELAYED_EVENTS_BADGE_KEY)),
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some(SCRIPTED_GUI_BADGE_KEY)),
            ],
        );
        let mut state = State::new();
        assert_eq!(
            state.fold_payload(&schema, &payload, &symbols).expect("folds"),
            FoldOutcome::Baseline
        );
        let states: Vec<KeyedState> = state.collections().iter().map(|c| (*c).clone()).collect();
        let mut first = Vec::new();
        encode_baseline(&schema, &states, &mut first).expect("encodes");
        let (decoded, consumed) = decode_baseline(&schema, &first, 0).expect("decodes");
        assert_eq!(consumed, first.len());
        let mut second = Vec::new();
        encode_baseline(&schema, &decoded, &mut second).expect("re-encodes");
        assert_eq!(first, second, "baseline is byte-identical after a round trip");
        assert_eq!(decoded, states);
    }

    #[test]
    fn a_delta_round_trips_and_the_reader_replays_it() {
        let schema = schema();
        let symbols = symbols();
        let mut state = State::new();
        let baseline = build_payload(
            0,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
                variable_row(SCOPE_COUNTRY, 4, "beta", 2),
            ],
        );
        state.fold_payload(&schema, &baseline, &symbols).expect("folds");
        let prior: Vec<KeyedState> = state.collections().iter().map(|c| (*c).clone()).collect();

        let delta_payload = build_payload(
            STATUS_DELTA_DATA,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 9),
                RowSpec {
                    scope_kind: SCOPE_COUNTRY,
                    owner_id: 4,
                    raw_value_kind: VALUE_VARIABLE_FIXED_I64 | VALUE_KIND_DELETED,
                    flags: ROW_FLAG_KEY_PRESENT,
                    key: Some("beta".into()),
                    ..RowSpec::default()
                },
            ],
        );
        assert_eq!(
            state.fold_payload(&schema, &delta_payload, &symbols).expect("folds"),
            FoldOutcome::Delta
        );
        let next: Vec<KeyedState> = state.collections().iter().map(|c| (*c).clone()).collect();
        assert_eq!(next[0].len(), 1, "the tombstone removed the country row");

        let deltas: Vec<Option<KeyedDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let d = diff(c, &prior[i], &next[i]);
                if d.is_empty() { None } else { Some(d) }
            })
            .collect();
        let mut bytes = Vec::new();
        encode_delta(&schema, &prior, &deltas, &mut bytes).expect("encodes");
        let (decoded, consumed) = decode_delta(&schema, &prior, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        let mut replayed = prior.clone();
        for (state, delta) in replayed.iter_mut().zip(decoded.iter()) {
            if let Some(delta) = delta {
                apply(state, delta);
            }
        }
        assert_eq!(replayed, next, "the reader replay is stored-equal to the fold");
    }

    // --- The fold mirror ---

    #[test]
    fn a_baseline_clears_all_five_accumulators_and_a_delta_clears_only_two() {
        // `world_fold.rs::fold_scripted_state` — the partial clear the recipe has no
        // name for. Three collections survive a delta; two do not.
        let schema = schema();
        let symbols = symbols();
        let mut state = State::new();
        let seed = build_payload(
            STATUS_FLAG_NAMES_PARTIAL | STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
                named_flag_row(SCOPE_GLOBAL, 0, "beta", 2, 3),
                token_flag_row(SCOPE_GLOBAL, 0, 5, 6, 7),
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some(SCRIPTED_GUI_BADGE_KEY)),
            ],
        );
        state.fold_payload(&schema, &seed, &symbols).expect("folds");
        assert_eq!(collection_lengths(&state), [1, 1, 1, 1, 0]);

        // An EMPTY delta: the three keyed families are retained, the two
        // replace-every-frame ones are emptied.
        let empty_delta = build_payload(STATUS_DELTA_DATA, &[]);
        assert_eq!(
            state.fold_payload(&schema, &empty_delta, &symbols).expect("folds"),
            FoldOutcome::Delta
        );
        assert_eq!(collection_lengths(&state), [1, 1, 1, 0, 0]);

        // An empty BASELINE clears everything.
        let empty_baseline = build_payload(0, &[]);
        assert_eq!(
            state.fold_payload(&schema, &empty_baseline, &symbols).expect("folds"),
            FoldOutcome::Baseline
        );
        assert_eq!(collection_lengths(&state), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing_and_does_not_reject_the_stream() {
        let schema = schema();
        let mut state = State::new();
        // The fold takes this exit BEFORE the payload-length and fixed-scale
        // checks (`world_fold.rs:18937` then `:18944`), so a malformed body on
        // such a record is an excluded hour and not a stream rejection. Built by
        // hand because `build_payload` cannot produce a wrong length.
        let mut payload = build_payload(STATUS_DELTA_DATA, &[]);
        payload[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(
            state.fold_payload(&schema, &payload, &symbols()).expect("no rejection"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(!state.baseline_seen());
        assert_eq!(collection_lengths(&state), [0; COLLECTION_COUNT]);
        // ...and once a baseline has landed the same body IS a stream rejection.
        let mut state = State::new();
        state
            .fold_payload(&schema, &build_payload(0, &[]), &symbols())
            .expect("baseline");
        assert!(state.fold_payload(&schema, &payload, &symbols()).is_err());
    }

    #[test]
    fn both_tombstone_encodings_remove_and_only_one_is_frame_gated() {
        // `world_fold.rs:19010`-`:19011` and `:19055`-`:19060`: `deleted` is the
        // OR of the value-kind high bit and row flag bit 4, but only the first is
        // refused outside a delta frame. The survey line claiming "a tombstone
        // outside a delta frame is a hard error" is true of one form only.
        let schema = schema();
        let symbols = symbols();
        let flag_form = RowSpec {
            scope_kind: SCOPE_GLOBAL,
            owner_id: 0,
            raw_value_kind: VALUE_VARIABLE_FIXED_I64,
            flags: ROW_FLAG_KEY_PRESENT | ROW_FLAG_DELETED,
            key: Some("alpha".into()),
            ..RowSpec::default()
        };
        let kind_form = RowSpec {
            raw_value_kind: VALUE_VARIABLE_FIXED_I64 | VALUE_KIND_DELETED,
            flags: ROW_FLAG_KEY_PRESENT,
            ..flag_form.clone()
        };
        // In a BASELINE frame: the flag form is accepted, the value-kind form is
        // not.
        let mut state = State::new();
        assert_eq!(
            state
                .fold_payload(&schema, &build_payload(0, &[flag_form.clone()]), &symbols)
                .expect("accepted in a baseline"),
            FoldOutcome::Baseline
        );
        assert!(State::new()
            .fold_payload(&schema, &build_payload(0, &[kind_form.clone()]), &symbols)
            .is_err());
        // In a DELTA frame both remove the key.
        for tombstone in [flag_form, kind_form] {
            let mut state = State::new();
            state
                .fold_payload(
                    &schema,
                    &build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 3)]),
                    &symbols,
                )
                .expect("baseline");
            assert_eq!(collection_lengths(&state)[0], 1);
            state
                .fold_payload(&schema, &build_payload(STATUS_DELTA_DATA, &[tombstone]), &symbols)
                .expect("delta");
            assert_eq!(collection_lengths(&state)[0], 0);
        }
    }

    #[test]
    fn a_tombstone_for_a_key_that_was_never_present_is_a_no_op() {
        let schema = schema();
        let symbols = symbols();
        let mut state = State::new();
        state.fold_payload(&schema, &build_payload(0, &[]), &symbols).expect("baseline");
        let payload = build_payload(
            STATUS_DELTA_DATA,
            &[RowSpec {
                scope_kind: SCOPE_GLOBAL,
                owner_id: 0,
                raw_value_kind: VALUE_FLAG_TOKEN_ONLY | VALUE_KIND_DELETED,
                flags: ROW_FLAG_TOKEN_PRESENT,
                token_id: 51_485,
                ..RowSpec::default()
            }],
        );
        state.fold_payload(&schema, &payload, &symbols).expect("delta");
        assert_eq!(collection_lengths(&state), [0; COLLECTION_COUNT]);
    }

    #[test]
    fn a_repeated_identity_inside_one_frame_is_last_writer_wins() {
        // The fold's own `insert` is last-writer-wins (`world_fold.rs::apply_scripted_state_row`),
        // so the keyed set reproduces its stored state exactly. The payload
        // authority is what rejects the duplicate, and it is tested separately.
        let schema = schema();
        let symbols = symbols();
        let mut state = State::new();
        let payload = build_payload(
            0,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
                variable_row(SCOPE_GLOBAL, 0, "alpha", 2),
            ],
        );
        state.fold_payload(&schema, &payload, &symbols).expect("folds");
        let stored = state.variables.values().next().expect("one row");
        assert_eq!(stored.values, vec![Value::Signed(2)]);
        assert!(check_authority_payload(&payload).is_err());
    }

    #[test]
    fn two_badge_rows_with_one_identity_are_refused() {
        // The one place this port refuses what the fold accepts. The fold PUSHes
        // both into a `Vec` (`world_fold.rs::apply_scripted_state_row`); a keyed set would silently
        // keep one. See the module docs for the producer evidence.
        let schema = schema();
        let payload = build_payload(
            STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some(SCRIPTED_GUI_BADGE_KEY)),
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some("something_else")),
            ],
        );
        let err = State::new()
            .fold_payload(&schema, &payload, &symbols())
            .expect_err("refused");
        assert!(err.contains("two opaque badge rows"), "{err}");
        // The same refusal reaches the pass-1 scanner, so the two passes agree.
        assert!(KeyScanner::new()
            .scan_payload(&payload, &mut BTreeSet::new())
            .is_err());
        // The producer evidence: the payload authority refuses it too, for its
        // own reason (the badge key constant), and would refuse the equal-key
        // form as a duplicate identity.
        assert!(check_authority_payload(&payload).is_err());
        let equal_keys = build_payload(
            STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some(SCRIPTED_GUI_BADGE_KEY)),
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some(SCRIPTED_GUI_BADGE_KEY)),
            ],
        );
        assert!(check_authority_payload(&equal_keys).is_err());
        // Two badges of DIFFERENT kinds are a legal pair and both are kept.
        let both = build_payload(
            STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED | STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[
                badge_row(VALUE_DELAYED_EVENT_OPAQUE, Some(DELAYED_EVENTS_BADGE_KEY)),
                badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some(SCRIPTED_GUI_BADGE_KEY)),
            ],
        );
        let mut state = State::new();
        state.fold_payload(&schema, &both, &symbols()).expect("folds");
        assert_eq!(collection_lengths(&state)[3], 2);
        check_authority_payload(&both).expect("authority accepts the legal pair");
    }

    #[test]
    fn a_fold_error_leaves_the_accumulator_untouched() {
        let schema = schema();
        let symbols = symbols();
        let mut state = State::new();
        state
            .fold_payload(
                &schema,
                &build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]),
                &symbols,
            )
            .expect("baseline");
        let before = state.clone();
        // A second row that the row validator refuses, after a first that folds.
        let mut bad = variable_row(SCOPE_GLOBAL, 0, "beta", 2);
        bad.reserved1 = 1;
        let payload = build_payload(
            STATUS_DELTA_DATA,
            &[variable_row(SCOPE_GLOBAL, 0, "gamma", 3), bad],
        );
        assert!(state.fold_payload(&schema, &payload, &symbols).is_err());
        assert_eq!(state, before, "all-or-nothing");
    }

    #[test]
    fn the_key_scanner_and_the_fold_agree_on_every_payload() {
        // The two passes of the harness must classify identically, or pass 2
        // measures a stream pass 1 did not validate.
        let schema = schema();
        let symbols = symbols();
        let payloads: Vec<Vec<u8>> = vec![
            build_payload(STATUS_DELTA_DATA, &[]),
            build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]),
            build_payload(
                STATUS_DELTA_DATA | STATUS_FLAG_NAMES_PARTIAL,
                &[token_flag_row(SCOPE_COUNTRY, 2, 9, 1, 2)],
            ),
            build_payload(STATUS_DELTA_DATA, &[variable_row(SCOPE_STATE, 7, "beta", -1)]),
            build_payload(0, &[]),
            // A refused one.
            build_payload(0, &[variable_row(0, 0, "alpha", 1)]),
        ];
        let mut fold = State::new();
        let mut scanner = KeyScanner::new();
        let mut vocabulary = BTreeSet::new();
        for payload in &payloads {
            let scanned = scanner.scan_payload(payload, &mut vocabulary);
            let folded = fold.fold_payload(&schema, payload, &symbols);
            match (&scanned, &folded) {
                (Ok((a, _)), Ok(b)) => assert_eq!(a, b),
                (Err(_), Err(_)) => {}
                other => panic!("scanner and fold disagree: {other:?}"),
            }
            assert_eq!(scanner.baseline_seen(), fold.baseline_seen());
        }
        // Every key the fold resolved is in the vocabulary the scanner built —
        // the completeness property pass 2 depends on.
        assert!(vocabulary.contains("alpha"));
        assert!(vocabulary.contains("beta"));
    }

    #[test]
    fn the_key_scanner_collects_tombstone_keys_too() {
        // A tombstone still needs a symbol: the removal is by key.
        let payload = build_payload(
            STATUS_DELTA_DATA,
            &[RowSpec {
                scope_kind: SCOPE_GLOBAL,
                owner_id: 0,
                raw_value_kind: VALUE_VARIABLE_FIXED_I64 | VALUE_KIND_DELETED,
                flags: ROW_FLAG_KEY_PRESENT,
                key: Some("sabotaged_coal".into()),
                ..RowSpec::default()
            }],
        );
        let mut scanner = KeyScanner::new();
        scanner
            .scan_payload(&build_payload(0, &[]), &mut BTreeSet::new())
            .expect("baseline");
        let mut vocabulary = BTreeSet::new();
        let (outcome, scan) = scanner.scan_payload(&payload, &mut vocabulary).expect("delta");
        assert_eq!(outcome, FoldOutcome::Delta);
        assert_eq!(scan.rows, 1);
        assert_eq!(scan.tombstones, 1);
        assert_eq!(scan.live_rows_by_collection, [0; COLLECTION_COUNT]);
        assert!(vocabulary.contains("sabotaged_coal"));
    }

    #[test]
    fn payload_key_vocabulary_matches_the_scanner() {
        let payload = build_payload(
            STATUS_FLAG_NAMES_PARTIAL,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
                named_flag_row(SCOPE_COUNTRY, 2, "beta", 1, 2),
                token_flag_row(SCOPE_COUNTRY, 2, 3, 1, 2),
            ],
        );
        let mut vocabulary = BTreeSet::new();
        KeyScanner::new()
            .scan_payload(&payload, &mut vocabulary)
            .expect("folds");
        assert_eq!(payload_key_vocabulary(&payload).expect("walks"), vocabulary);
        assert_eq!(vocabulary.len(), 2);
    }

    // --- The permissiveness the port mirrors ---

    #[test]
    fn the_mapper_mirrors_the_folds_permissiveness_where_the_authority_is_stricter() {
        // The fold applies no scope bound, no printability rule and no key-length
        // ceiling; the payload authority applies all three. Refusing them in the
        // mapper would be data loss on rows the recorder folds, so the port maps
        // them and the harness refuses the stream through the authority instead.
        let schema = schema();
        for (label, spec) in [
            (
                "an owner past the authority's country ceiling",
                variable_row(SCOPE_COUNTRY, MAX_COUNTRY_TAG_DB_ID + 1, "alpha", 1),
            ),
            (
                "an owner past the authority's state ceiling",
                variable_row(SCOPE_STATE, MAX_STATE_ID + 1, "beta", 1),
            ),
        ] {
            let payload = build_payload(0, &[spec]);
            let mut state = State::new();
            state
                .fold_payload(&schema, &payload, &symbols())
                .unwrap_or_else(|e| panic!("{label} must fold: {e}"));
            assert_eq!(collection_lengths(&state)[0], 1, "{label}");
            assert!(check_authority_payload(&payload).is_err(), "{label}");
        }
    }

    #[test]
    fn the_fold_refuses_the_rows_it_refuses() {
        let schema = schema();
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "an unknown flag bit",
                build_payload(
                    0,
                    &[RowSpec {
                        flags: ROW_FLAG_KEY_PRESENT | ROW_FLAG_VALUE_PRESENT | (1 << 6),
                        ..variable_row(SCOPE_GLOBAL, 0, "alpha", 1)
                    }],
                ),
            ),
            (
                "a global row with a nonzero owner",
                build_payload(0, &[variable_row(SCOPE_GLOBAL, 3, "alpha", 1)]),
            ),
            (
                "a country row with a zero owner",
                build_payload(0, &[variable_row(SCOPE_COUNTRY, 0, "alpha", 1)]),
            ),
            (
                "an unsupported scope",
                build_payload(0, &[variable_row(4, 1, "alpha", 1)]),
            ),
            (
                "an unsupported value kind",
                build_payload(
                    0,
                    &[RowSpec {
                        raw_value_kind: 7,
                        ..variable_row(SCOPE_GLOBAL, 0, "alpha", 1)
                    }],
                ),
            ),
            (
                "a nonzero reserved1",
                build_payload(
                    0,
                    &[RowSpec {
                        reserved1: 1,
                        ..variable_row(SCOPE_GLOBAL, 0, "alpha", 1)
                    }],
                ),
            ),
            (
                "a badge tombstone",
                build_payload(
                    STATUS_DELTA_DATA | STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
                    &[RowSpec {
                        raw_value_kind: VALUE_SCRIPTED_GUI_OPAQUE | VALUE_KIND_DELETED,
                        flags: 0,
                        key: None,
                        ..badge_row(VALUE_SCRIPTED_GUI_OPAQUE, None)
                    }],
                ),
            ),
            (
                "a delayed-event row outside country scope",
                build_payload(
                    0,
                    &[RowSpec {
                        scope_kind: SCOPE_STATE,
                        ..delayed_event_row(9, MIN_DELAYED_EVENT_ID, 1, None)
                    }],
                ),
            ),
            (
                "a delayed-event id below the floor",
                build_payload(0, &[delayed_event_row(9, MIN_DELAYED_EVENT_ID - 1, 1, None)]),
            ),
            (
                "delayed due hours past the ceiling",
                build_payload(
                    0,
                    &[delayed_event_row(
                        9,
                        MIN_DELAYED_EVENT_ID,
                        1,
                        Some(MAX_DELAYED_EVENT_DUE_HOURS + 1),
                    )],
                ),
            ),
            (
                "a token flag row with no token",
                build_payload(
                    0,
                    &[RowSpec {
                        raw_value_kind: VALUE_FLAG_TOKEN_ONLY,
                        flags: ROW_FLAG_VALUE_PRESENT,
                        key: None,
                        ..RowSpec {
                            scope_kind: SCOPE_GLOBAL,
                            ..RowSpec::default()
                        }
                    }],
                ),
            ),
        ];
        for (label, payload) in cases {
            // Seed a baseline first, so a DELTA case is a real delta rather than
            // the delta-before-baseline exit — which returns `Ok` and would make
            // the refusal look like it fired when it did not.
            let mut state = State::new();
            state
                .fold_payload(&schema, &build_payload(0, &[]), &symbols())
                .expect("seed baseline");
            assert!(
                state.fold_payload(&schema, &payload, &symbols()).is_err(),
                "{label} must be refused"
            );
        }
    }

    #[test]
    fn a_key_range_outside_the_table_is_refused_and_never_panics() {
        let mut payload = build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]);
        // Push the key offset past the end of the table.
        let cursor = HEADER_LEN + 16;
        payload[cursor..cursor + 4].copy_from_slice(&9_999u32.to_le_bytes());
        assert!(State::new().fold_payload(&schema(), &payload, &symbols()).is_err());
        // A non-UTF-8 key is refused by the fold, not coerced.
        let mut payload = build_payload(0, &[variable_row(SCOPE_GLOBAL, 0, "alpha", 1)]);
        let table = payload.len() - 5;
        payload[table] = 0xff;
        assert!(State::new().fold_payload(&schema(), &payload, &symbols()).is_err());
    }

    #[test]
    fn no_payload_of_any_length_panics() {
        for len in 0..(HEADER_LEN + 2 * ROW_LEN + 8) {
            let payload = vec![0xa5u8; len];
            let _ = peek_header(&payload);
            let _ = split_payload(&payload);
            let _ = check_authority_payload(&payload);
            let _ = State::new().fold_payload(&schema(), &payload, &symbols());
            let _ = KeyScanner::new().scan_payload(&payload, &mut BTreeSet::new());
        }
        for len in 0..(2 * ROW_LEN) {
            let _ = parse_row(&vec![0u8; len], &[], true);
            let _ = parse_row(&vec![0xffu8; len], &[1, 2, 3], false);
        }
    }

    // --- The payload authority ---

    #[test]
    fn the_authority_accepts_a_capture_shaped_payload() {
        // Canonical semantic order: family 1 (variables) before family 2 (flags),
        // then scope, owner, token, key.
        let payload = build_payload(
            STATUS_DELTA_DATA | STATUS_FLAG_NAMES_PARTIAL,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
                variable_row(SCOPE_COUNTRY, 4, "beta", 2),
                token_flag_row(SCOPE_COUNTRY, 4, 48_293, 1, 60_832_556),
            ],
        );
        check_authority_payload(&payload).expect("accepted");
    }

    #[test]
    fn the_authority_enforces_what_the_fold_does_not() {
        let good = build_payload(
            STATUS_DELTA_DATA | STATUS_FLAG_NAMES_PARTIAL,
            &[
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
                token_flag_row(SCOPE_COUNTRY, 4, 48_293, 1, 60_832_556),
            ],
        );
        check_authority_payload(&good).expect("accepted");

        // changed_rows > row_count — a word the FOLD never reads (the d65 rule),
        // so it is enforced here and nowhere else.
        let mut broken = good.clone();
        broken[16..20].copy_from_slice(&99u32.to_le_bytes());
        assert!(check_authority_payload(&broken).is_err());
        assert!(split_payload(&broken).is_ok(), "the fold ignores changed_rows");

        // measured_scopes > observed_scopes.
        let mut broken = good.clone();
        broken[28..32].copy_from_slice(&99u32.to_le_bytes());
        assert!(check_authority_payload(&broken).is_err());

        // A miscounted row family.
        let mut broken = good.clone();
        broken[36..40].copy_from_slice(&5u32.to_le_bytes());
        assert!(check_authority_payload(&broken).is_err());

        // Token-only rows without the names-partial badge.
        let mut broken = good.clone();
        broken[20..24].copy_from_slice(&STATUS_DELTA_DATA.to_le_bytes());
        assert!(check_authority_payload(&broken).is_err());

        // A non-canonical row order.
        let swapped = build_payload(
            STATUS_DELTA_DATA | STATUS_FLAG_NAMES_PARTIAL,
            &[
                token_flag_row(SCOPE_COUNTRY, 4, 48_293, 1, 60_832_556),
                variable_row(SCOPE_GLOBAL, 0, "alpha", 1),
            ],
        );
        assert!(check_authority_payload(&swapped).is_err());
        State::new()
            .fold_payload(&schema(), &swapped, &symbols())
            .expect("the fold has no ordering rule");

        // A non-printable key byte.
        let mut broken = good.clone();
        let table = broken.len() - 5;
        broken[table] = 0x01;
        assert!(check_authority_payload(&broken).is_err());
    }

    #[test]
    fn the_authority_couples_each_badge_to_its_status_bit_in_both_directions() {
        let badge = build_payload(
            STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            &[badge_row(VALUE_DELAYED_EVENT_OPAQUE, Some(DELAYED_EVENTS_BADGE_KEY))],
        );
        check_authority_payload(&badge).expect("accepted");
        // The badge without its bit.
        let no_bit = build_payload(
            0,
            &[badge_row(VALUE_DELAYED_EVENT_OPAQUE, Some(DELAYED_EVENTS_BADGE_KEY))],
        );
        assert!(check_authority_payload(&no_bit).is_err());
        // The bit without its badge.
        let no_badge = build_payload(STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED, &[]);
        assert!(check_authority_payload(&no_badge).is_err());
        // A wrong badge key.
        let wrong_key = build_payload(
            STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            &[badge_row(VALUE_SCRIPTED_GUI_OPAQUE, Some("not_the_marker"))],
        );
        assert!(check_authority_payload(&wrong_key).is_err());
        // ...and a measured delayed-event row beside the opaque badge.
        let both = build_payload(
            STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            &[
                delayed_event_row(9, MIN_DELAYED_EVENT_ID, 1, None),
                badge_row(VALUE_DELAYED_EVENT_OPAQUE, Some(DELAYED_EVENTS_BADGE_KEY)),
            ],
        );
        assert!(check_authority_payload(&both).is_err());
    }

    // --- Transcription checks ---

    #[test]
    fn the_transcribed_grammar_constants_match_the_fold() {
        // `world_fold.rs:921`-`:965`, transcribed by hand because `wire_format`'s
        // library half must not depend on `recorder_host`. A drifted mirror fails
        // the run, not the review.
        assert_eq!(HEADER_LEN, 64);
        assert_eq!(ROW_LEN, 48);
        assert_eq!(SCHEMA_VERSION, 2);
        assert_eq!(FIXED_SCALE_RAW, 100_000);
        assert_eq!(FIXED_SCALE, 100_000.0);
        assert_eq!((SCOPE_GLOBAL, SCOPE_COUNTRY, SCOPE_STATE), (1, 2, 3));
        assert_eq!(
            (
                VALUE_VARIABLE_FIXED_I64,
                VALUE_FLAG_NAMED,
                VALUE_FLAG_TOKEN_ONLY,
                VALUE_DELAYED_EVENT_OPAQUE,
                VALUE_SCRIPTED_GUI_OPAQUE,
                VALUE_DELAYED_EVENT,
            ),
            (1, 2, 3, 4, 5, 6)
        );
        assert_eq!(VALUE_KIND_DELETED, 1 << 31);
        assert_eq!(ROW_FLAG_KEY_PRESENT, 1);
        assert_eq!(ROW_FLAG_TOKEN_PRESENT, 1 << 1);
        assert_eq!(ROW_FLAG_VALUE_PRESENT, 1 << 2);
        assert_eq!(ROW_FLAG_DATE_PRESENT, 1 << 3);
        assert_eq!(ROW_FLAG_DELETED, 1 << 4);
        assert_eq!(ROW_FLAG_DELAYED_DUE_PRESENT, 1 << 5);
        assert_eq!(ROW_KNOWN_FLAGS, 0x3f);
        assert_eq!(MIN_DELAYED_EVENT_ID, 100_000);
        assert_eq!(MAX_DELAYED_EVENT_ID, 4_294_900_000);
        assert_eq!(MAX_DELAYED_EVENT_DUE_HOURS, 500_000);
        // Was `2_048`, a guess at the vanilla-plus-DLC country count. Now the
        // engine's structural id-space width: `id < *(int32*)(db + 0x0C)` with
        // a signed-`int` size gives `i32::MAX - 1`.
        assert_eq!(MAX_COUNTRY_TAG_DB_ID, 2_147_483_646);
        // Was `65_535`, a guess with a circular derivation. Now the state id
        // space's structural width (`claim_39c989ce13504504353c`).
        assert_eq!(MAX_STATE_ID, 2_147_483_646);
        assert_eq!(MAX_KEY_BYTES, 512);
        assert_eq!(MAX_KEY_TABLE_BYTES, 8 * 1024 * 1024);
        assert_eq!(DELAYED_EVENTS_BADGE_KEY, "root_unresolved:delayed_events");
        assert_eq!(
            SCRIPTED_GUI_BADGE_KEY,
            "root_unresolved:scripted_gui_open_state"
        );
    }

    #[test]
    fn the_status_bits_match_the_shipping_contract() {
        // `compact_contract/src/status_flags_base.rs:16`, `:69`-`:71`. The three
        // lane-local badges alias globally-assigned bits 11, 12 and 13, which is
        // why the failure classification has to run through
        // `compact_status_for_global_interpretation` — see `bin/bench_real_d51.rs`.
        assert_eq!(STATUS_DELTA_DATA, 1 << 10);
        assert_eq!(STATUS_FLAG_NAMES_PARTIAL, 1 << 11);
        assert_eq!(STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED, 1 << 12);
        assert_eq!(STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED, 1 << 13);
    }

    #[test]
    fn the_symbol_table_is_deterministic_and_order_independent() {
        let a = Symbols::from_vocabulary(["gamma", "alpha", "beta"]);
        let b = Symbols::from_vocabulary(["beta", "gamma", "alpha", "alpha"]);
        assert_eq!(a, b);
        assert_eq!(a.id("alpha"), Some(0));
        assert_eq!(a.id("beta"), Some(1));
        assert_eq!(a.id("gamma"), Some(2));
        assert_eq!(a.symbol(2), Some("gamma"));
        assert_eq!(a.symbol(3), None);
        assert_eq!(a.id("delta"), None);
        assert_eq!(a.len(), 3);
        assert!(Symbols::empty().is_empty());
    }
}
