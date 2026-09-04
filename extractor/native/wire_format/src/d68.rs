//! d68 `session_header_live` — the campaign's top-level save/session header.
//!
//! Derived from `world_fold.rs:16653`-`:16787`
//! (`fold_session_header`), its payload constants at `:224`-`:253`, the carried
// Fold authority: `world_fold.rs::fold_session_header`.
//! accumulator at `:1744` / `:2258`-`:2267`, the value it publishes at
//! `world_fold.rs::build_session_header_value`, and the collector read that produces the presence flags
//! (`extractor/native/collector/src/attach_log.rs:61784`-`:61969`).
//! `domain-68::Overview` in `extractor/documents/technical_evidence.json`
//! carries campaign-identity RE knowledge but no field-level wire knowledge, so
//! the fold and collector read are the whole basis.
//!
//! **This domain is a SPLIT deliverable.** The five scalars — `player_tag_db_id`,
//! `dlcs`, `start_date_raw`, `save_version`, `session` — are ported here. The
//! payload's three length-prefixed UTF-8 strings (`ideology`, `cosmetic_tag`,
//! `version`) are **stop-reported**: they are parsed here, because the fold's
//! trailing-byte rule (`world_fold.rs::fold_session_header`) makes the whole payload invalid if
//! they are malformed and because the deliverable reports their byte share, but
//! they are never mapped onto a `Record`. See "Why the three strings stop".
//!
//! ## This is not a table
//!
//! Every other ported domain folds a payload of rows into a table. d68 folds a
//! payload of *fields* into ONE carried record, merged **last-value-wins per
//! field** through eight presence bits (`world_fold.rs::fold_session_header`). It is
//! the first merge accumulator in the fan-out, and the whole shape of this module
//! follows from that:
//!
//! - the fold's stored state is `Option<SessionHeaderFoldState>` (`:1744`) — one
//!   optional record, not a set of rows;
//! - a set presence bit **assigns**; a clear one **leaves the prior value in
//!   place** (`world_fold.rs::fold_session_header`, one `if has_x { header.x = ... }` per field).
//!   Nothing in the fold ever clears a field, on any input;
//! - a failure status retains the whole prior header and folds nothing
//!   (`world_fold.rs::fold_session_header`), which is the same "unchanged" rule at record
//!   granularity.
//!
//! ## Payload grammar (`world_fold.rs::fold_session_header`, written at `attach_log.rs:61914`-`:61969`)
//!
//! A 16-byte header:
//!
//! | offset | width | field | read at |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms` | written `attach_log.rs:61945`; the fold never reads it |
//! | `+8`  | u32 | `presence`        | `world_fold.rs::fold_session_header` |
//! | `+12` | u32 | `status_flags`    | `world_fold.rs::fold_session_header` |
//!
//! then a fixed 20-byte scalar block, then three length-prefixed strings:
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+16` | u32 | `player_tag_db_id` | raw | `1<<0`, and `1..=MAX_PLAYER_TAG_DB_ID` when set (`world_fold.rs::fold_session_header`) |
//! | `+20` | u32 | `dlcs`             | raw | `1<<1` |
//! | `+24` | u32 | `start_date_raw`   | raw | `1<<2` |
//! | `+28` | u32 | `save_version`     | raw | `1<<3` |
//! | `+32` | u32 | `session`          | raw | `1<<4` |
//! | `+36` | u16+n | `ideology`     | — | `1<<5` — STOPPED |
//! | …     | u16+n | `cosmetic_tag` | — | `1<<6` — STOPPED |
//! | …     | u16+n | `version`      | — | `1<<7` — STOPPED |
//!
//! **No scales anywhere.** Every scalar is a raw `u32` the collector read out of
//! `CGameState` and the fold assigns unmodified (`:10145`-`:10157`). There is no
//! `FIXED_SCALE`, no `/32768`, and no field this domain divides, so the
//! 32768-vs-100000 failure class cannot arise — the offset test in the test module
//! pins that as a claim rather than an assumption.
//!
//! Every gate is enforced **bidirectionally**: a set bit requires a value (and,
//! for `player_tag_db_id`, a value in range), and a clear bit requires a
//! zero-packed scalar or a zero-length string (`world_fold.rs::fold_session_header`).
//! A clear bit therefore carries no data, which is what makes "unchanged" the only
//! reading it can have.
//!
//! ## Four-state semantics: what record-state contract `Absent` means under merge
//!
//! This is the domain's one real judgement, and it deserves the whole argument.
//!
//! **1. record-state contract's four states are properties of a stored record field, and the stored
//! record here is the merged accumulator.** `fold_session_header` stores the
//! merged header (`world_fold.rs::fold_session_header`); `build_session_header_value` publishes that
//! merged header, not the frame (`world_fold.rs::build_session_header_value`); and the measurement's reader
//! replay asserts stored-equality against it. So the question "what state is this
//! field in?" is asked of the accumulator, never of one frame's update.
//!
//! **2. A clear presence bit is a statement about the UPDATE, not about the
//! value.** Its meaning is "do not change the stored field". The wire format
//! already expresses exactly that, one level above record-state contract: a keyed set's delta carries
//! only the rows that changed, and within a changed row the update mask sets a bit
//! only for the fields that changed (delta-minimality contract minimality, `body.rs`). Frame-level
//! presence maps onto **delta mask bits**, not onto a four-state code. That
//! correspondence is the reason this domain fits the existing grammar at all.
//!
//! **3. Reading a clear bit as record-state contract `Absent` would be silent data loss.** `Absent`
//! is "not applicable, or unsupported by this schema" and encodes as presence bit
//! `0` in the record body, which a conforming reader reconstructs as *this field
//! has no value*. Applied to the frame after the collector momentarily failed to
//! read `dlcs`, it would blank a value the recorder still carries — and, because
//! nothing ever restores it except another successful read, would keep it blank.
//! `Absent` is therefore **unreachable** in this schema, and the schema does not
//! declare `.optional()` on anything: `optional_count() == 0`.
//!
//! **4. The one empty state a stored d68 field can hold is `Unknown`** — the
//! accumulator has never had a successful read of that field, so it is
//! "applicable, not captured", record-state contract's unknown word for word. The collector is
//! explicit (`attach_log.rs:61788`-`:61790`): *"Each field is read independently
//! and presence-flagged: a torn/absent read clears its bit (never a fabricated
//! value)."* Every per-field guard behind that bit is a read failure or a validity
//! rejection — an unreadable `CGameState`, a non-heap `CHuman` pointer, a
//! `tag_db_id` that fails its range gate or does not resolve in this frame's
//! validated country roster (`:61815`-`:61839`), a `start_date_raw` of zero
//! (`:61867`), a build banner whose prefix does not match (`:61877`-`:61880`).
//! None of the five scalars is a relation that can point at nothing — a session
//! always has a start date, a save version, a session id and a human player — so
//! `Null` is not reachable either, and no field declares `.nullable()`.
//!
//! **5. `Unknown` is a prefix state.** Because the merge never clears, a field can
//! only be `Unknown` *before* its first successful read; after that it holds a
//! value forever. That is a testable property of [`Header::merge`], not a remark —
//! see `unknown_is_a_prefix_state_the_merge_can_never_re_enter`.
//!
//! The one genuinely undecidable case is `player_tag_db_id` on a session with no
//! human player (an observer or headless host): the collector's roster cross-check
//! would clear the bit for that too, so "no player" and "player unreadable" are
//! indistinguishable on the wire. The recipe's tie-break applies — choose
//! `Unknown`, because `Null` would assert "definitively no player" about a value
//! that may merely have been uncaptured — and it is an open question in the
//! deliverable.
//!
//! ## Key and shape: one row under a constant key
//!
//! `Shape::KeyedSet` with a single **constant** key component
//! ([`SINGLETON_KEY_COMPONENT`]). The payload carries no natural key, and the two
//! alternatives both fail:
//!
//! - **`session` as the key fails the key rules** — it is presence-gated, so it
//!   can be `Unknown`, and schema-identity contract requires a key member to be concrete
//!   (`Schema::validate` returns `KeyMemberNotConcrete`, `schema.rs:302`-`:308`).
//! - **`session` as the key would also lose data**, which is the stronger reason.
//!   The fold merges every frame into the SAME carried header whatever the session
//!   word says (`:10143`); it never resets the accumulator on a session change. So
//!   keying by `session` would split one merged record into several keyed rows,
//!   turn a session change into a tombstone plus an insert, and make "this field
//!   is unchanged from the frame before" inexpressible across that boundary.
//!   `session` is a **value** in this grammar, not an identity.
//!
//! The constant key is concrete, totally ordered, present in every row and carries
//! no capability, so it satisfies schema-identity contract outright; and it persists across hours in the
//! sense step 5 requires, because the singleton it names is the same carried header
//! every hour. The two levels of optionality stay separate and neither collapses:
//!
//! - the fold's OUTER `Option<SessionHeaderFoldState>` — whether any payload has
//!   folded at all — is the keyed set's **membership**: [`keyed_state`] returns an
//!   empty map before the first fold and a one-entry map after it. The flip
//!   happens on the first *folded payload*, not on the first *field*, because
//!   `get_or_insert_with` at `world_fold.rs::fold_session_header` runs before any presence bit is
//!   consulted — so a legal payload carrying nothing still creates a row of all
//!   `Unknown`s ([`Accumulator`]);
//! - each field's own `Option` is the record's **unknown bitmap**.
//!
//! ## Why the three strings stop
//!
//! `ideology`, `cosmetic_tag` and `version` are `read_msvc_string` /
//! `read_module_ascii_cstr` tokens (`attach_log.rs:61850`-`:61880`), carried as
//! `u16` length + UTF-8 bytes (`world_fold.rs::fold_session_header`). schema-kind contract declares no
//! string kind (`schema.rs:12`-`:20`), and the three do not stop for the same
//! reason:
//!
//! - `ideology` (`"fascism"`) and `cosmetic_tag` (`"FRA_VICHY"`) are
//!   **content-defined tokens**, which step 3 of the recipe does NOT stop: content-identity contract's
//!   rule makes them a `ContentId` resolved through a caller-supplied symbol
//!   table, exactly as d29 does. They are stopped **by this assignment's scope**,
//!   not by a technical blocker, and the deliverable says what the resolver would
//!   have to be.
//! - `version` (`"Operation Postern v1.19.2.0.a729"`) is a **build banner** read
//!   from module `.rdata`. No content manifest defines the set of build strings —
//!   it is engine-authored text whose vocabulary is the set of every build ever
//!   shipped or patched — so the symbol rule can never reach it. That is the
//!   Scope stop ("text no content manifest can ever resolve"), whose only recorded
//!   instance so far is player-authored text. It stops on the rule, not on scope.
//!
//! Their bytes stay charged to the shipping arm; the measurement reports the byte
//! split and a second, cut shipping arm (the string block is a payload **suffix**,
//! so it is cuttable — d11's `PDQ1` precedent).
//!
//! ## No other writer of the accumulator
//!
//! `self.session_header` is written in exactly one place in `world_fold.rs`
//! (`world_fold.rs::fold_session_header`, inside this fold), initialised to `None` at `world_fold.rs::new`, and read at
//! `world_fold.rs::build_session_header_value`. There is no prune, no cross-domain deletion path and no second
//! producer, so the recipe's "enumerate every other writer" step resolves by
//! enumeration rather than by a per-capture assertion.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 68;
/// The single collection: the merged session header, one row.
pub const SESSION: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    /// The constant singleton key — see "Key and shape" in the module docs.
    pub const SESSION_SINGLETON: u32 = 1;
    pub const PLAYER_TAG_DB_ID: u32 = 2;
    pub const DLCS: u32 = 3;
    pub const START_DATE_RAW: u32 = 4;
    pub const SAVE_VERSION: u32 = 5;
    pub const SESSION_ID: u32 = 6;
}

// --- Payload grammar, from `world_fold.rs:224`-`:253` and `:10040`-`:10139`. ---

// THE LAYOUT BELOW IS NOT RESTATED HERE ANY MORE.
//
// This module executes `d68_grammar`, the same crate used by the producer, fold,
// and authority validator, so structural constants cannot drift between copied
// decoders. It keeps only its own semantic layer and diagnostics. The constants
// stay `pub` under their original names because this crate's tests and schema are
// written against them.

/// `d68_grammar::HEADER_LEN`.
pub const HEADER_LEN: usize = d68_grammar::HEADER_LEN;
/// `d68_grammar::SCALAR_BLOCK_LEN`.
pub const SCALAR_BLOCK_LEN: usize = d68_grammar::SCALAR_BLOCK_LEN;
/// Where the three length-prefixed strings begin — the stopped slice runs from
/// here to the end of the payload (`d68_grammar::STRING_BLOCK_OFFSET`).
pub const STRING_BLOCK_OFFSET: usize = d68_grammar::STRING_BLOCK_OFFSET;

pub const FLAG_PLAYER: u32 = d68_grammar::Field::Player.flag();
pub const FLAG_DLCS: u32 = d68_grammar::Field::Dlcs.flag();
pub const FLAG_START_DATE: u32 = d68_grammar::Field::StartDate.flag();
pub const FLAG_SAVE_VERSION: u32 = d68_grammar::Field::SaveVersion.flag();
pub const FLAG_SESSION: u32 = d68_grammar::Field::Session.flag();
pub const FLAG_IDEOLOGY: u32 = d68_grammar::Field::Ideology.flag();
pub const FLAG_COSMETIC_TAG: u32 = d68_grammar::Field::CosmeticTag.flag();
pub const FLAG_VERSION: u32 = d68_grammar::Field::Version.flag();
/// `d68_grammar::KNOWN_FLAGS`. Any other bit is a hard error — an unrecognised
/// presence word means the payload is not this grammar, so no bit in it can be
/// trusted. The RULE is still this port's; only the mask is shared.
pub const KNOWN_FLAGS: u32 = d68_grammar::KNOWN_FLAGS;

/// `d68_grammar::MAX_TOKEN_LEN`.
pub const MAX_STRING_LEN: usize = d68_grammar::MAX_TOKEN_LEN;
/// `SESSION_HEADER_MAX_PLAYER_TAG_DB_ID`, `world_fold.rs`.
///
/// Was `2_048`, a guess at the vanilla-plus-DLC country count.
/// ENGINE-STRUCTURAL now: `id < *(int32*)(db + 0x0C)` with a signed-`int` size
/// gives `i32::MAX - 1`. Pinned by
/// `tests/wire_format_suite/producer_consumer_admission_agreement.rs`.
pub const MAX_PLAYER_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;

/// The key component of the one and only row. Its value is arbitrary and carries
/// no information — what it carries is the fact that the collection is a
/// singleton. Zero, so a reader that ever sees a second component or a second key
/// is looking at a different grammar.
pub const SINGLETON_KEY_COMPONENT: u64 = 0;

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // The constant singleton key. `Unsigned`, not `ContentId`: nothing in a
        // content manifest resolves it — it is generated at this boundary and
        // names the collection's single member (step 3's "must a content manifest
        // resolve this id for it to keep its meaning across mod sets?" is No).
        Field::new(f::SESSION_SINGLETON, Kind::Unsigned).key(0),
        // The country the human player is playing. `ContentId`: a country's
        // `tag_db_id` is content-defined (content-identity contract) — under another mod set the same id
        // is a different country — which is what d63's key and d20's `owner`
        // class the same way.
        Field::new(f::PLAYER_TAG_DB_ID, Kind::ContentId).unknown_capable(),
        // A bitmask of enabled DLCs (`attach_log.rs:61861`). `Unsigned`, not
        // `Enum`: the bits are independent, so a code space would have to be the
        // whole 2^32 power set, and schema-kind contract's `Enum` is for mutually exclusive codes.
        // Not `ContentId` either — a DLC is an engine artifact, not mod content,
        // and no manifest resolves a DLC bit; step 3's "engine
        // constants/discriminators are still not content" case.
        Field::new(f::DLCS, Kind::Unsigned).unknown_capable(),
        // The campaign's start date, as the engine's absolute hour counter — the
        // SAME encoding as the H4CR record header's `game_hour_raw`, so
        // `compact_contract::calendar::hour_index_from_raw` /
        // `date_from_raw` apply to it unchanged (`calendar.rs:8`-`:47`). Measured:
        // 60,759,371 == 1936.1.1 hour 12, against a capture whose own hours are
        // 60,831,018.. == 1944.3.7. `Unsigned` and RAW: it is a point in time, not
        // a counter that rises (schema-kind contract's `Monotone` is a hint for accumulating
        // counters, and this field is constant for a whole campaign), and the
        // calendar conversion is the reader's business — a mapper that applied it
        // would bake `DATE_EPOCH_CONST` into stored bytes and lose every raw value
        // outside the calendar's 1850..2200 sanity window, which the fold accepts.
        Field::new(f::START_DATE_RAW, Kind::Unsigned).unknown_capable(),
        // The engine's save-format version word. `Unsigned`: a format
        // discriminator, not a counter accumulating within the stream, so
        // `Monotone`'s delta-encoding hint buys nothing and would assert a rise
        // rule this domain never exercises.
        Field::new(f::SAVE_VERSION, Kind::Unsigned).unknown_capable(),
        // The engine's session id — a runtime instance identity created when the
        // session starts (measured 43,742). `Unsigned` per step 3: a runtime
        // instance id is never a `ContentId`.
        Field::new(f::SESSION_ID, Kind::Unsigned).unknown_capable(),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: SESSION,
            // The only implemented shape, and the right one: the merged header is
            // one row with a stable identity that persists across hours, and only
            // a keyed set has the delta grammar that expresses "unchanged".
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary: header split, update parse, accumulator, mapper ------------

/// The 16-byte payload header (`world_fold.rs::fold_session_header`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub presence: u32,
    pub status_flags: u32,
}

/// Split a d68 DATA payload into its header and the bytes behind it.
///
/// **The split point is this fold's call order, and it is unusually early.**
/// `fold_session_header` runs exactly ONE check before its status exit — the
/// 16-byte minimum (`world_fold.rs::fold_session_header`) — and then returns `Ok` on a failure
/// status (`world_fold.rs::fold_session_header`) having folded nothing. Every other rule, *including*
/// the unknown-presence-flag rejection at `world_fold.rs::fold_session_header`, runs after that exit. So a
/// caller must be able to classify a record from the header alone, and this
/// function must not enforce anything the fold checks later: a failure-status
/// record whose presence word is garbage is a record the recorder accepts and
/// folds nothing from, and refusing it here would refuse a stream the recorder
/// reads happily.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = d68_grammar::read_header(payload).map_err(session_header_violation)?;
    Ok((
        PayloadHeader {
            elapsed_wall_ms: header.elapsed_wall_ms,
            presence: header.presence,
            status_flags: header.status_flags,
        },
        &payload[HEADER_LEN..],
    ))
}

/// This port's spelling of a `d68_grammar` refusal.
///
/// The leaf carries operands and no text, because the four d68 sites spell the
/// same refusal four different ways. Every arm reproduces the message this
/// module produced before the walk moved into the leaf. No wildcard arm: a new
/// structural refusal does not compile until this port has decided how to say
/// it.
fn session_header_violation(violation: d68_grammar::Violation) -> String {
    use d68_grammar::{Section, ViolationKind};
    match violation.kind {
        ViolationKind::Need {
            section: Section::Header,
            needed,
            available,
        } => format!("session_header payload {available} shorter than the {needed}-byte header"),
        ViolationKind::Need {
            section: Section::ScalarBlock,
            needed,
            available,
        } => format!(
            "session_header payload {available} shorter than the {needed}-byte scalar block"
        ),
        ViolationKind::Need {
            section: Section::TokenPrefix(field),
            ..
        } => format!(
            "session_header {} length prefix truncated",
            field.diagnostic_name()
        ),
        ViolationKind::Need {
            section: Section::TokenBytes(field),
            ..
        } => format!("session_header {} bytes truncated", field.diagnostic_name()),
        ViolationKind::TokenTooLong { field, len, .. } => format!(
            "session_header {} length {len} out of range",
            field.diagnostic_name()
        ),
        ViolationKind::TrailingBytes {
            consumed,
            available,
        } => format!("session_header trailing bytes: consumed {consumed} of {available}"),
        ViolationKind::OffsetOverflow { at, len, .. } => {
            format!("session_header offset {at} + {len} overflows")
        }
    }
}

/// One frame's parsed **update** — the fields whose presence bits are set. Not a
/// state: a field that is `None` here was not carried this frame, which under the
/// merge means "leave the stored value alone", never "clear it".
///
/// The three string fields are parsed (the fold's trailing-byte rule makes the
/// whole payload invalid if they are malformed) and are **stop-reported**, never
/// mapped onto a `Record`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Update {
    pub player_tag_db_id: Option<u32>,
    pub dlcs: Option<u32>,
    pub start_date_raw: Option<u32>,
    pub save_version: Option<u32>,
    pub session: Option<u32>,
    /// STOPPED — parsed for validation and byte accounting only.
    pub ideology: Option<String>,
    /// STOPPED — parsed for validation and byte accounting only.
    pub cosmetic_tag: Option<String>,
    /// STOPPED — parsed for validation and byte accounting only.
    pub version: Option<String>,
}

/// Parse everything `fold_session_header` does **after** its status exit
/// (`world_fold.rs::fold_session_header`), in the fold's own order, with the fold's own
/// rejections. Nothing here can panic on any byte string.
pub fn parse_update(payload: &[u8]) -> core::result::Result<Update, String> {
    let (header, _) = split_payload(payload)?;
    let presence = header.presence;
    // `world_fold.rs::fold_session_header`.
    if presence & !KNOWN_FLAGS != 0 {
        return Err(format!(
            "session_header unknown presence flags {presence:#x}"
        ));
    }
    // `:10052` — the scalar block's own length check, AFTER the presence check.
    let [player_tag_db_id, dlcs, start_date_raw, save_version, session] =
        d68_grammar::read_scalar_block(payload)
            .map_err(session_header_violation)?
            .words();

    let has_player = presence & FLAG_PLAYER != 0;
    let has_dlcs = presence & FLAG_DLCS != 0;
    let has_start_date = presence & FLAG_START_DATE != 0;
    let has_save_version = presence & FLAG_SAVE_VERSION != 0;
    let has_session = presence & FLAG_SESSION != 0;

    // `:10073`-`:10095`: an absent scalar must pack a zero, and a present
    // `player_tag_db_id` must be in `1..=MAX_PLAYER_TAG_DB_ID`.
    if !has_player && player_tag_db_id != 0 {
        return Err("session_header player_tag_db_id without flag".into());
    }
    if has_player && (player_tag_db_id == 0 || player_tag_db_id > MAX_PLAYER_TAG_DB_ID) {
        return Err(format!(
            "session_header player_tag_db_id {player_tag_db_id} out of range"
        ));
    }
    if !has_dlcs && dlcs != 0 {
        return Err("session_header dlcs without flag".into());
    }
    if !has_start_date && start_date_raw != 0 {
        return Err("session_header start_date without flag".into());
    }
    if !has_save_version && save_version != 0 {
        return Err("session_header save_version without flag".into());
    }
    if !has_session && session != 0 {
        return Err("session_header session without flag".into());
    }

    // `:10097`-`:10133`: three length-prefixed UTF-8 strings, each present in the
    // blob regardless of its flag (an absent field packs len 0).
    // `d68_grammar` delimits the three tokens; UTF-8 validity and the
    // flag/emptiness pair are this port's semantics and keep the fold's
    // fatality (the port refuses, exactly as the fold does).
    let mut ideology = None;
    let mut cosmetic_tag = None;
    let mut version = None;
    let mut cursor = d68_grammar::TokenCursor::new();
    let end = loop {
        match cursor.next(payload).map_err(session_header_violation)? {
            d68_grammar::TokenStep::Token { field, bytes } => {
                let name = field.diagnostic_name();
                let value = if bytes.is_empty() {
                    None
                } else {
                    Some(
                        core::str::from_utf8(bytes)
                            .map_err(|_| format!("session_header {name} not utf8"))?
                            .to_string(),
                    )
                };
                let flag_present = field.is_present(presence);
                if !flag_present && value.is_some() {
                    return Err(format!("session_header {name} bytes without flag"));
                }
                if flag_present && value.is_none() {
                    return Err(format!("session_header {name} flag set but empty"));
                }
                // No wildcard arm: a ninth token field does not compile until
                // this port has decided what it means.
                match field {
                    d68_grammar::Field::Ideology => ideology = value,
                    d68_grammar::Field::CosmeticTag => cosmetic_tag = value,
                    d68_grammar::Field::Version => version = value,
                    d68_grammar::Field::Player
                    | d68_grammar::Field::Dlcs
                    | d68_grammar::Field::StartDate
                    | d68_grammar::Field::SaveVersion
                    | d68_grammar::Field::Session => {
                        return Err(format!(
                            "session_header scalar field {name} delimited as a token"
                        ))
                    }
                }
            }
            d68_grammar::TokenStep::End(at) => break at,
        }
    };
    // `:10134`: the three strings must account for every remaining byte.
    d68_grammar::exact_consumption(end, payload.len()).map_err(session_header_violation)?;

    Ok(Update {
        player_tag_db_id: has_player.then_some(player_tag_db_id),
        dlcs: has_dlcs.then_some(dlcs),
        start_date_raw: has_start_date.then_some(start_date_raw),
        save_version: has_save_version.then_some(save_version),
        session: has_session.then_some(session),
        ideology,
        cosmetic_tag,
        version,
    })
}

/// The carried header itself — the fold's `SessionHeaderFoldState`
/// (`world_fold.rs:2258`). Every field is independently `Option`, and a `None`
/// means "no successful read of this field has ever folded", which is record-state contract's
/// `Unknown`.
///
/// **This is the INNER half of the fold's state.** The outer half — whether a
/// header exists at all — is [`Accumulator`], and the two must not be conflated;
/// see its docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub player_tag_db_id: Option<u32>,
    pub dlcs: Option<u32>,
    pub start_date_raw: Option<u32>,
    pub save_version: Option<u32>,
    pub session: Option<u32>,
    /// STOPPED — carried so the accumulator mirrors the fold, never encoded.
    pub ideology: Option<String>,
    /// STOPPED — carried so the accumulator mirrors the fold, never encoded.
    pub cosmetic_tag: Option<String>,
    /// STOPPED — carried so the accumulator mirrors the fold, never encoded.
    pub version: Option<String>,
}

impl Header {
    /// Merge one frame's update, last-value-wins per field
    /// (`world_fold.rs::fold_session_header`).
    ///
    /// **A `None` in the update leaves the stored field untouched.** There is no
    /// input, on any code path, that clears a field once it has been set — which
    /// is why `Unknown` is a prefix state and `Absent` is unreachable.
    pub fn merge(&mut self, update: &Update) {
        if let Some(v) = update.player_tag_db_id {
            self.player_tag_db_id = Some(v);
        }
        if let Some(v) = update.dlcs {
            self.dlcs = Some(v);
        }
        if let Some(v) = update.start_date_raw {
            self.start_date_raw = Some(v);
        }
        if let Some(v) = update.save_version {
            self.save_version = Some(v);
        }
        if let Some(v) = update.session {
            self.session = Some(v);
        }
        if let Some(v) = &update.ideology {
            self.ideology = Some(v.clone());
        }
        if let Some(v) = &update.cosmetic_tag {
            self.cosmetic_tag = Some(v.clone());
        }
        if let Some(v) = &update.version {
            self.version = Some(v.clone());
        }
    }
}

/// The fold's accumulator in full: `Option<SessionHeaderFoldState>`
/// (`world_fold.rs:1744`, initialised `None` at `world_fold.rs::new`).
///
/// **The outer `Option` flips on the first payload that FOLDS, not on the first
/// field that arrives.** `fold_session_header` materialises the header with
/// `self.session_header.get_or_insert_with(Default::default)` at `world_fold.rs::fold_session_header`,
/// unconditionally, *before* it consults a single presence bit. So a legal
/// payload carrying no field at all — presence `0`, five zero-packed scalars,
/// three zero-length strings, 42 bytes — still creates a header, and
/// `build_session_header_value` then attaches an object whose every field is
/// empty (`world_fold.rs::build_session_header_value`). Collapsing that into "nothing has folded" would
/// publish an empty collection where the recorder publishes a row, which is why
/// the outer `Option` is a witness of its own and not derived from the fields.
///
/// The two levels map onto two different wire mechanisms and neither may stand in
/// for the other: the outer `Option` is the keyed set's **membership**
/// ([`keyed_state`]), the inner per-field `Option`s are the record's **unknown
/// bitmap**.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accumulator {
    header: Option<Header>,
}

impl Accumulator {
    /// The carried header, or `None` while the fold would still be holding `None`
    /// — no payload has folded yet, so there is nothing to publish.
    pub fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// Whether a header exists at all — the outer `Option`, never a function of
    /// the fields.
    pub fn is_materialized(&self) -> bool {
        self.header.is_some()
    }

    /// `get_or_insert_with(Default::default)` then the per-field merge, in the
    /// fold's own order (`world_fold.rs::fold_session_header`). Materialising first is
    /// the whole point: an update with no field still creates the header.
    pub fn merge(&mut self, update: &Update) {
        self.header.get_or_insert_with(Header::default).merge(update);
    }
}

/// Fold one d68 DATA payload into the carried header — the recipe's fourth public
/// boundary, and the domain's whole grammar.
///
/// **Error contract: all-or-nothing.** `fold_session_header` parses the entire
/// payload before it touches `self.session_header` (the merge block is
/// `world_fold.rs::fold_session_header`, after every rejection), so a payload this function
/// rejects leaves `state` byte-identical. Mirrored here and pinned by
/// `a_rejected_payload_leaves_the_state_untouched`.
///
/// **Status is the CALLER's decision, and it comes first.** The fold's status exit
/// (`world_fold.rs::fold_session_header`) precedes every rule this function implements, so a caller must
/// classify the record's `status_flags` — with
/// `compact_contract`'s own predicate, never a copied mask — and skip this call
/// entirely on a failure status, exactly as the recorder retains the prior header.
/// `wire_format` is the reference implementation of the wire format and does not
/// depend on `compact_contract` outside its measurement bins, which is why the
/// classification is not made here.
pub fn fold_payload(
    accumulator: &mut Accumulator,
    payload: &[u8],
) -> core::result::Result<PayloadHeader, String> {
    let (header, _) = split_payload(payload)?;
    let update = parse_update(payload)?;
    accumulator.merge(&update);
    Ok(header)
}

/// The key of the one row. Takes the state for signature parity with every other
/// domain's `key_from_*`, and ignores it: the key is constant by construction —
/// see "Key and shape" in the module docs for why no carried field can be one.
pub fn key_from_header(_state: &Header) -> Key {
    key(&[SINGLETON_KEY_COMPONENT])
}

/// The merged session header onto its stored Record — the recipe's
/// `record_from_row` with the row replaced by the accumulator, because d68 has no
/// rows (see "This is not a table"). The three fixed arguments keep their meaning:
/// `frame_tick` is derived-field contract's derived-field input, and d68 declares no derived field, so
/// it is taken and ignored.
///
/// Fail-closed: a state carrying a `player_tag_db_id` the fold would have rejected
/// is an `Err`, never a coerced value, because `Header`'s fields are public and a
/// caller can build one without going through [`parse_update`].
pub fn record_from_header(
    collection: &Collection,
    state: &Header,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d68 has no derived field; derived-field contract's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    if collection.collection_id != SESSION {
        // A key-light collection like this one would otherwise fill a foreign
        // collection with plausible values under someone else's field ids.
        return Err(format!(
            "session_header mapper called with collection {}",
            collection.collection_id
        ));
    }
    if let Some(tag) = state.player_tag_db_id {
        if tag == 0 || tag > MAX_PLAYER_TAG_DB_ID {
            return Err(format!(
                "session_header player_tag_db_id {tag} out of range"
            ));
        }
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::PLAYER_TAG_DB_ID => match state.player_tag_db_id {
                Some(v) => Value::ContentId(v as u64),
                None => Value::Unknown,
            },
            field::DLCS => unsigned_or_unknown(state.dlcs),
            field::START_DATE_RAW => unsigned_or_unknown(state.start_date_raw),
            field::SAVE_VERSION => unsigned_or_unknown(state.save_version),
            field::SESSION_ID => unsigned_or_unknown(state.session),
            other => {
                return Err(format!(
                    "session_header has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// The whole collection for one hour: the fold's OUTER `Option` becomes the keyed
/// set's membership. An accumulator no payload has ever folded into yields an
/// EMPTY keyed set — the state in which `build_session_header_value` returns
/// `None` and attaches nothing (`world_fold.rs::build_session_header_value`) — and any materialised
/// accumulator yields exactly one row, however many of its fields are still
/// unread.
pub fn keyed_state(
    collection: &Collection,
    accumulator: &Accumulator,
    frame_tick: u64,
) -> core::result::Result<KeyedState, String> {
    let mut out = KeyedState::new();
    let Some(header) = accumulator.header() else {
        return Ok(out);
    };
    out.insert(
        key_from_header(header),
        record_from_header(collection, header, frame_tick)?,
    );
    Ok(out)
}

/// A field the accumulator has never had a successful read of is `Unknown`
/// ("applicable, not captured"), never `Null` and never `Absent` — see the module
/// docs' four-state argument.
fn unsigned_or_unknown(value: Option<u32>) -> Value {
    match value {
        Some(v) => Value::Unsigned(v as u64),
        None => Value::Unknown,
    }
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

// `u16_at` lost its last reader when the token walk moved into
// `d68_grammar`: this module no longer reads a length prefix itself.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw payload, built the way the collector packs one
    /// (`attach_log.rs:61914`-`:61967`).
    fn raw_payload(
        elapsed_wall_ms: u64,
        presence: u32,
        status_flags: u32,
        scalars: [u32; 5],
        strings: [&str; 3],
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
        p.extend_from_slice(&presence.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        for v in scalars {
            p.extend_from_slice(&v.to_le_bytes());
        }
        for s in strings {
            p.extend_from_slice(&(s.len() as u16).to_le_bytes());
            p.extend_from_slice(s.as_bytes());
        }
        p
    }

    /// The capture's own record, byte for byte
    /// (`runs/batch2/deepwar_t1/collector_stream.bin`, all 146 hours; the 903
    /// hours of `runs/batch1/deepwar_t1` carry the identical values).
    fn measured_payload() -> Vec<u8> {
        raw_payload(
            7,
            KNOWN_FLAGS,
            0,
            [368, 257_535, 60_759_371, 33, 43_742],
            ["fascism", "FRA_VICHY", "Operation Postern v1.19.2.0.a729"],
        )
    }

    fn accumulated(payload: &[u8]) -> Accumulator {
        let mut acc = Accumulator::default();
        fold_payload(&mut acc, payload).expect("the payload folds");
        acc
    }

    fn folded(payload: &[u8]) -> Header {
        accumulated(payload)
            .header()
            .expect("a folded payload materialises the header")
            .clone()
    }

    fn record_of(c: &Collection, acc: &Accumulator) -> Record {
        record_from_header(c, acc.header().expect("materialised"), 0).expect("maps")
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d68 schema is well-formed");
    }

    #[test]
    fn it_is_a_one_row_keyed_set_under_a_constant_key() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet);
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "one constant key component");
        assert_eq!(keys[0].field_id, field::SESSION_SINGLETON);
        assert_eq!(
            keys[0].kind,
            Kind::Unsigned,
            "generated at this boundary; no manifest resolves it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert!(
            !keys[0].optional && !keys[0].nullable && !keys[0].unknown_capable,
            "a key member must be concrete"
        );
        assert_eq!(c.fields.len(), 6, "1 constant key + 5 scalars");

        // The key is the same every hour, whatever the state carries — which is
        // what makes the merged header ONE row across the whole capture rather
        // than a tombstone-and-insert whenever a field moves.
        let mut a = Header::default();
        a.session = Some(1);
        let mut b = Header::default();
        b.session = Some(999_999);
        assert_eq!(key_from_header(&a), key_from_header(&b));
        assert_eq!(key_from_header(&Header::default()), key(&[0]));
    }

    #[test]
    fn session_cannot_be_the_key_and_the_schema_proves_it() {
        // The rule half of the constant-key defense: `session` is presence-gated,
        // so it is `unknown_capable`, and schema-identity contract rejects a key member that is not
        // concrete (`schema.rs:302`-`:308`). This is machine-checked rather than
        // asserted in prose.
        let mut s = schema();
        let fields = &mut s.collections[0].fields;
        fields.retain(|f| f.field_id != field::SESSION_SINGLETON);
        for f in fields.iter_mut() {
            if f.field_id == field::SESSION_ID {
                *f = Field::new(field::SESSION_ID, Kind::Unsigned)
                    .unknown_capable()
                    .key(0);
            }
        }
        assert!(
            matches!(
                s.validate(grid().log2()),
                Err(crate::schema::SchemaError::KeyMemberNotConcrete { .. })
            ),
            "a presence-gated field is not a legal key"
        );
    }

    #[test]
    fn no_field_is_optional_or_nullable_and_every_value_field_is_unknown_capable() {
        // The four-state decision, machine-checked. `Absent` is unreachable
        // because a clear presence bit means "unchanged", which the delta grammar
        // expresses and record-state contract does not; `Null` is unreachable because none of the
        // five is a relation that can point at nothing.
        let c = collection();
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
            c.value_fields().map(|f| f.field_id).collect::<Vec<_>>(),
            "every scalar can be never-yet-captured"
        );
        assert_eq!(unknown_capable.len(), 5);
    }

    #[test]
    fn no_field_is_continuous_so_no_quantization_applies() {
        // Step 4's "quantization saves bytes" test does not apply: d68 has no
        // continuous field at all. Every scalar is an integer word the fold
        // assigns raw (`world_fold.rs::fold_session_header`), so there is nothing to
        // quantize and nothing to justify.
        let c = collection();
        assert!(c.fields.iter().all(|f| !matches!(f.kind, Kind::Continuous)));
        assert!(c.fields.iter().all(|f| f.quant == Quant::None));
        assert!(c.fields.iter().all(|f| f.derived.is_none()));
    }

    #[test]
    fn exactly_one_field_is_a_content_id() {
        // The codec-measurement contract retrofit sweep needs "checked, this one" rather than silence:
        // now that content-identity contract's manifest-id assignment exists, this mapper's raw cast
        // must become a resolver lookup. It is a ContentId VALUE, not a key, so the
        // retrofit does not reorder any key or change a baseline's byte layout.
        let c = collection();
        let content: Vec<u32> = c
            .fields
            .iter()
            .filter(|f| matches!(f.kind, Kind::ContentId))
            .map(|f| f.field_id)
            .collect();
        assert_eq!(content, vec![field::PLAYER_TAG_DB_ID]);
        assert!(
            c.key_fields()
                .iter()
                .all(|f| !matches!(f.kind, Kind::ContentId)),
            "no ContentId key: the heavier retrofit class does not apply here"
        );
    }

    #[test]
    fn the_measured_payload_maps_every_field_at_its_own_offset() {
        // Step 8's offset test. Every scalar carries a DISTINCT value, so a
        // transposed pair fails; d68 divides nothing, so this doubles as the
        // "no scale" claim.
        let c = collection();
        let state = folded(&measured_payload());
        let r = record_from_header(&c, &state, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::PLAYER_TAG_DB_ID)],
            Value::ContentId(368)
        );
        assert_eq!(r.values[index_of(&c, field::DLCS)], Value::Unsigned(257_535));
        assert_eq!(
            r.values[index_of(&c, field::START_DATE_RAW)],
            Value::Unsigned(60_759_371),
            "raw engine hour counter, never converted to a calendar date"
        );
        assert_eq!(
            r.values[index_of(&c, field::SAVE_VERSION)],
            Value::Unsigned(33)
        );
        assert_eq!(
            r.values[index_of(&c, field::SESSION_ID)],
            Value::Unsigned(43_742)
        );
        // The stopped strings are parsed and carried, never encoded.
        assert_eq!(state.ideology.as_deref(), Some("fascism"));
        assert_eq!(state.cosmetic_tag.as_deref(), Some("FRA_VICHY"));
        assert_eq!(
            state.version.as_deref(),
            Some("Operation Postern v1.19.2.0.a729")
        );
        assert_eq!(r.values.len(), 5, "the record carries the five scalars only");
    }

    #[test]
    fn every_header_word_is_read_at_its_own_offset() {
        // A transposed header word mis-reads every gate behind it, and no
        // round-trip or replay assertion can see that — both arms would agree on
        // the wrong slice. Distinct values at all three offsets.
        let payload = raw_payload(
            0x0102_0304_0506_0708,
            FLAG_DLCS,
            0x0000_2000,
            [0, 77, 0, 0, 0],
            ["", "", ""],
        );
        let (header, rest) = split_payload(&payload).expect("splits");
        assert_eq!(header.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(header.presence, FLAG_DLCS);
        assert_eq!(header.status_flags, 0x0000_2000);
        assert_eq!(rest.len(), payload.len() - HEADER_LEN);

        // Short of the header, at every length.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&payload[..len]).is_err(), "len {len}");
        }
        // And the split does NOT enforce a later rule: an unknown presence bit is
        // rejected by `parse_update`, never by `split_payload`, because the fold
        // checks it after the status exit (`world_fold.rs:10044` then `:10047`).
        let garbage = raw_payload(0, 1 << 31, 1, [0; 5], ["", "", ""]);
        assert!(
            split_payload(&garbage).is_ok(),
            "a failure-status record with a garbage presence word must still split"
        );
        assert!(parse_update(&garbage).is_err());
    }

    #[test]
    fn a_full_record_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_header(&c, &folded(&measured_payload()), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(back, record, "decode reproduces the record");
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn an_all_unknown_record_roundtrips_byte_for_byte() {
        // The other extreme: the accumulator after a frame that carried only the
        // stopped strings. Every scalar is `Unknown`, and the unknown bitmap is
        // what has to survive.
        let c = collection();
        let state = folded(&raw_payload(0, FLAG_IDEOLOGY, 0, [0; 5], ["fascism", "", ""]));
        let record = record_from_header(&c, &state, 0).expect("maps");
        for f in c.value_fields() {
            assert_eq!(
                record.values[index_of(&c, f.field_id)],
                Value::Unknown,
                "field {}",
                f.field_id
            );
        }
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back, record);
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes);
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // record-state contract keeps zero out of the three empty states. Every flag set with every
        // scalar zero is not a legal frame — `player_tag_db_id` must be
        // `1..=MAX_PLAYER_TAG_DB_ID`
        // when its bit is set (`world_fold.rs::fold_session_header`) — so this is the largest
        // real-zero frame the fold accepts.
        let c = collection();
        let state = folded(&raw_payload(
            0,
            KNOWN_FLAGS & !FLAG_PLAYER,
            0,
            [0, 0, 0, 0, 0],
            ["a", "b", "c"],
        ));
        let r = record_from_header(&c, &state, 0).expect("maps");
        for id in [
            field::DLCS,
            field::START_DATE_RAW,
            field::SAVE_VERSION,
            field::SESSION_ID,
        ] {
            assert_eq!(r.values[index_of(&c, id)], Value::Unsigned(0), "field {id}");
        }
        assert_eq!(
            r.values[index_of(&c, field::PLAYER_TAG_DB_ID)],
            Value::Unknown,
            "the one field whose bit is clear"
        );
    }

    #[test]
    fn a_never_captured_field_is_unknown_field_by_field() {
        // One presence bit clear at a time, over the whole scalar gate space —
        // not one sampled case. A gated-off scalar must pack a zero, so build the
        // payload that way.
        let c = collection();
        let cases: [(u32, u32); 5] = [
            (FLAG_PLAYER, field::PLAYER_TAG_DB_ID),
            (FLAG_DLCS, field::DLCS),
            (FLAG_START_DATE, field::START_DATE_RAW),
            (FLAG_SAVE_VERSION, field::SAVE_VERSION),
            (FLAG_SESSION, field::SESSION_ID),
        ];
        for (cleared, gated_id) in cases {
            let presence = KNOWN_FLAGS & !cleared;
            let scalars = [
                if presence & FLAG_PLAYER != 0 { 368 } else { 0 },
                if presence & FLAG_DLCS != 0 { 11 } else { 0 },
                if presence & FLAG_START_DATE != 0 { 12 } else { 0 },
                if presence & FLAG_SAVE_VERSION != 0 { 13 } else { 0 },
                if presence & FLAG_SESSION != 0 { 14 } else { 0 },
            ];
            let state = folded(&raw_payload(0, presence, 0, scalars, ["i", "c", "v"]));
            let r = record_from_header(&c, &state, 0).expect("maps");
            for f in c.value_fields() {
                assert_eq!(
                    r.values[index_of(&c, f.field_id)] == Value::Unknown,
                    f.field_id == gated_id,
                    "field {} under presence {presence:#x}",
                    f.field_id
                );
            }
        }
    }

    #[test]
    fn a_clear_bit_carries_the_prior_value_forward_it_never_clears_it() {
        // THE central claim of this domain. Frame 1 carries everything; frame 2
        // carries nothing but a string. The merged state — and therefore the
        // stored record — is unchanged.
        let c = collection();
        let mut acc = Accumulator::default();
        fold_payload(&mut acc, &measured_payload()).expect("frame 1");
        let after_first = record_of(&c, &acc);

        fold_payload(
            &mut acc,
            &raw_payload(9, FLAG_IDEOLOGY, 0, [0; 5], ["fascism", "", ""]),
        )
        .expect("frame 2 carries no scalar");
        assert_eq!(
            record_of(&c, &acc),
            after_first,
            "an unset presence bit means unchanged, never cleared"
        );
        let header = acc.header().expect("materialised");
        assert_eq!(header.player_tag_db_id, Some(368));
        assert_eq!(header.session, Some(43_742));

        // And a frame with NO presence bits at all is still a legal frame that
        // changes nothing (every scalar zero-packed, every string empty).
        fold_payload(&mut acc, &raw_payload(10, 0, 0, [0; 5], ["", "", ""]))
            .expect("frame 3 carries nothing");
        assert_eq!(
            record_of(&c, &acc),
            after_first,
            "an empty presence word is a no-op merge"
        );
    }

    #[test]
    fn a_set_bit_overwrites_last_value_wins_per_field() {
        let c = collection();
        let mut acc = Accumulator::default();
        fold_payload(&mut acc, &measured_payload()).expect("frame 1");
        // Only `session` moves.
        fold_payload(
            &mut acc,
            &raw_payload(0, FLAG_SESSION, 0, [0, 0, 0, 0, 5], ["", "", ""]),
        )
        .expect("frame 2");
        let r = record_of(&c, &acc);
        assert_eq!(r.values[index_of(&c, field::SESSION_ID)], Value::Unsigned(5));
        assert_eq!(
            r.values[index_of(&c, field::PLAYER_TAG_DB_ID)],
            Value::ContentId(368),
            "every other field is untouched"
        );
        assert_eq!(r.values[index_of(&c, field::DLCS)], Value::Unsigned(257_535));
    }

    #[test]
    fn unknown_is_a_prefix_state_the_merge_can_never_re_enter() {
        // The property that makes `Unknown` honest here: once a field has been
        // read successfully it can never go back to "not captured", because no
        // input to `merge` assigns `None`. Swept over every presence word.
        let c = collection();
        let mut acc = Accumulator::default();
        fold_payload(&mut acc, &measured_payload()).expect("seed");
        for presence in 0..=KNOWN_FLAGS {
            if presence & !KNOWN_FLAGS != 0 {
                continue;
            }
            let scalars = [
                if presence & FLAG_PLAYER != 0 { 1 } else { 0 },
                if presence & FLAG_DLCS != 0 { 2 } else { 0 },
                if presence & FLAG_START_DATE != 0 { 3 } else { 0 },
                if presence & FLAG_SAVE_VERSION != 0 { 4 } else { 0 },
                if presence & FLAG_SESSION != 0 { 5 } else { 0 },
            ];
            let strings = [
                if presence & FLAG_IDEOLOGY != 0 { "i" } else { "" },
                if presence & FLAG_COSMETIC_TAG != 0 { "c" } else { "" },
                if presence & FLAG_VERSION != 0 { "v" } else { "" },
            ];
            let mut next = acc.clone();
            fold_payload(&mut next, &raw_payload(0, presence, 0, scalars, strings))
                .unwrap_or_else(|e| panic!("presence {presence:#x}: {e}"));
            let r = record_of(&c, &next);
            for f in c.value_fields() {
                assert_ne!(
                    r.values[index_of(&c, f.field_id)],
                    Value::Unknown,
                    "field {} became Unknown again under presence {presence:#x}",
                    f.field_id
                );
            }
        }
    }

    #[test]
    fn the_folds_outer_option_is_the_keyed_sets_membership() {
        let c = collection();
        let empty = keyed_state(&c, &Accumulator::default(), 0).expect("maps");
        assert!(
            empty.is_empty(),
            "no payload has folded, so the collection has no member"
        );
        let one = keyed_state(&c, &accumulated(&measured_payload()), 0).expect("maps");
        assert_eq!(one.len(), 1);
        assert!(one.contains_key(&key(&[SINGLETON_KEY_COMPONENT])));
    }

    #[test]
    fn a_payload_carrying_no_field_still_materialises_a_row_of_unknowns() {
        // `get_or_insert_with` at `world_fold.rs::fold_session_header` runs BEFORE any presence
        // bit is consulted, so the smallest legal payload — presence 0, five
        // zero-packed scalars, three zero-length strings, 42 bytes — creates the
        // carried header, and `build_session_header_value` (`world_fold.rs::build_session_header_value`) then attaches
        // an object with every field empty. Deriving membership from the FIELDS
        // instead of from the outer `Option` would publish an empty collection
        // where the recorder publishes a row; this is the regression pin.
        let c = collection();
        let payload = raw_payload(0, 0, 0, [0; 5], ["", "", ""]);
        assert_eq!(payload.len(), 42, "the smallest legal d68 payload");

        let acc = accumulated(&payload);
        assert!(acc.is_materialized(), "a folded payload materialises the header");
        assert_eq!(
            acc.header(),
            Some(&Header::default()),
            "and every field of it is still unread"
        );

        let set = keyed_state(&c, &acc, 0).expect("maps");
        assert_eq!(set.len(), 1, "one row, not an empty collection");
        let record = set
            .get(&key(&[SINGLETON_KEY_COMPONENT]))
            .expect("under the singleton key");
        for f in c.value_fields() {
            assert_eq!(
                record.values[index_of(&c, f.field_id)],
                Value::Unknown,
                "field {}",
                f.field_id
            );
        }
        // And it encodes: an all-Unknown row is a legal record, not a hole.
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(&back, record);
    }

    #[test]
    fn membership_is_never_derived_from_the_fields() {
        // The property behind the pin above, swept: EVERY legal presence word
        // yields exactly one row after a fold, including the empty one. Nothing
        // about which fields arrived may change the collection's membership.
        let c = collection();
        for presence in 0..=KNOWN_FLAGS {
            if presence & !KNOWN_FLAGS != 0 {
                continue;
            }
            let scalars = [
                if presence & FLAG_PLAYER != 0 { 368 } else { 0 },
                if presence & FLAG_DLCS != 0 { 1 } else { 0 },
                if presence & FLAG_START_DATE != 0 { 2 } else { 0 },
                if presence & FLAG_SAVE_VERSION != 0 { 3 } else { 0 },
                if presence & FLAG_SESSION != 0 { 4 } else { 0 },
            ];
            let strings = [
                if presence & FLAG_IDEOLOGY != 0 { "i" } else { "" },
                if presence & FLAG_COSMETIC_TAG != 0 { "c" } else { "" },
                if presence & FLAG_VERSION != 0 { "v" } else { "" },
            ];
            let acc = accumulated(&raw_payload(0, presence, 0, scalars, strings));
            let set = keyed_state(&c, &acc, 0).expect("maps");
            assert_eq!(set.len(), 1, "presence {presence:#x} must yield one row");
        }
    }

    #[test]
    fn a_rejected_payload_leaves_the_state_untouched() {
        // The fold's error contract (`world_fold.rs::fold_session_header` runs after every
        // rejection): all-or-nothing. A harness that assumed the other contract
        // would silently disagree with the recorder on exactly the frames that
        // fail.
        let seeded = accumulated(&measured_payload());
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        invalid.push(raw_payload(0, 1 << 8, 0, [0; 5], ["", "", ""])); // unknown flag
        invalid.push(raw_payload(0, 0, 0, [0, 1, 0, 0, 0], ["", "", ""])); // dlcs w/o flag
        invalid.push(raw_payload(0, FLAG_PLAYER, 0, [0; 5], ["", "", ""])); // player 0 w/ flag
        invalid.push(raw_payload(0, FLAG_IDEOLOGY, 0, [0; 5], ["", "", ""])); // flag, empty
        let mut trailing = measured_payload();
        trailing.push(0);
        invalid.push(trailing);
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut acc = seeded.clone();
            assert!(fold_payload(&mut acc, &payload).is_err(), "case {i}");
            assert_eq!(acc, seeded, "case {i} mutated the accumulator");
        }
        // And the same contract from the un-materialised side: a rejected first
        // payload must not create the header.
        let mut fresh = Accumulator::default();
        assert!(fold_payload(&mut fresh, &raw_payload(0, 1 << 8, 0, [0; 5], ["", "", ""])).is_err());
        assert!(!fresh.is_materialized(), "a refused payload materialised nothing");
    }

    #[test]
    fn the_parser_rejects_every_payload_the_fold_rejects() {
        // Unknown presence bits — every one of the 24, not one sample
        // (`world_fold.rs::fold_session_header`).
        for bit in 8..32u32 {
            let p = raw_payload(0, 1 << bit, 0, [0; 5], ["", "", ""]);
            assert!(parse_update(&p).is_err(), "presence bit {bit}");
            let p = raw_payload(0, KNOWN_FLAGS | (1 << bit), 0, [368, 1, 1, 1, 1], ["i", "c", "v"]);
            assert!(parse_update(&p).is_err(), "presence bit {bit} beside the known set");
        }
        // A scalar carrying a non-zero word without its flag — each of the five
        // independently (`:10073`-`:10095`).
        for index in 0..5usize {
            let mut scalars = [0u32; 5];
            scalars[index] = 1;
            let presence = KNOWN_FLAGS & !(1 << index) & !(FLAG_IDEOLOGY | FLAG_COSMETIC_TAG | FLAG_VERSION);
            let p = raw_payload(0, presence, 0, scalars, ["", "", ""]);
            assert!(parse_update(&p).is_err(), "scalar {index} without its flag");
        }
        // `player_tag_db_id` bounds (`:10077`): 0 and one past the ceiling
        // rejected, 1 and the ceiling itself
        // accepted — the bound is not off by one.
        for tag in [0u32, MAX_PLAYER_TAG_DB_ID + 1, u32::MAX] {
            let p = raw_payload(0, FLAG_PLAYER, 0, [tag, 0, 0, 0, 0], ["", "", ""]);
            assert!(parse_update(&p).is_err(), "player {tag}");
        }
        for tag in [1u32, MAX_PLAYER_TAG_DB_ID] {
            let p = raw_payload(0, FLAG_PLAYER, 0, [tag, 0, 0, 0, 0], ["", "", ""]);
            assert_eq!(
                parse_update(&p).expect("in range").player_tag_db_id,
                Some(tag)
            );
        }
        // A short scalar block (`:10052`), at every length between the header and
        // the block's end.
        let full = measured_payload();
        for len in HEADER_LEN..STRING_BLOCK_OFFSET {
            assert!(parse_update(&full[..len]).is_err(), "len {len}");
        }
        // String rules (`:10101`-`:10133`), each independently.
        // - a length prefix over the cap
        let mut over = raw_payload(0, FLAG_IDEOLOGY, 0, [0; 5], ["", "", ""]);
        over[STRING_BLOCK_OFFSET..STRING_BLOCK_OFFSET + 2]
            .copy_from_slice(&((MAX_STRING_LEN + 1) as u16).to_le_bytes());
        assert!(parse_update(&over).is_err(), "length above the cap");
        // - the cap itself is legal
        let at_cap = "x".repeat(MAX_STRING_LEN);
        assert!(parse_update(&raw_payload(0, FLAG_IDEOLOGY, 0, [0; 5], [&at_cap, "", ""])).is_ok());
        // - bytes present without the flag, and a flag set over an empty string,
        //   for each of the three
        for slot in 0..3usize {
            let flags = [FLAG_IDEOLOGY, FLAG_COSMETIC_TAG, FLAG_VERSION];
            let mut strings = ["", "", ""];
            strings[slot] = "x";
            assert!(
                parse_update(&raw_payload(0, 0, 0, [0; 5], strings)).is_err(),
                "slot {slot} bytes without flag"
            );
            assert!(
                parse_update(&raw_payload(0, flags[slot], 0, [0; 5], ["", "", ""])).is_err(),
                "slot {slot} flag set but empty"
            );
        }
        // - a truncated length prefix and truncated bytes
        let base = raw_payload(0, FLAG_IDEOLOGY, 0, [0; 5], ["fascism", "", ""]);
        for len in STRING_BLOCK_OFFSET..base.len() {
            assert!(parse_update(&base[..len]).is_err(), "truncated at {len}");
        }
        // - non-UTF-8 bytes
        let mut bad_utf8 = raw_payload(0, FLAG_IDEOLOGY, 0, [0; 5], ["ab", "", ""]);
        bad_utf8[STRING_BLOCK_OFFSET + 2] = 0xFF;
        assert!(parse_update(&bad_utf8).is_err(), "not utf8");
        // - trailing bytes (`:10134`)
        let mut trailing = measured_payload();
        trailing.push(0);
        assert!(parse_update(&trailing).is_err(), "trailing byte");
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        let c = collection();
        let state = folded(&measured_payload());
        // A foreign collection id — the key-light check the recipe requires.
        let mut foreign = c.clone();
        foreign.collection_id = 99;
        assert!(record_from_header(&foreign, &state, 0).is_err());
        // A field id this mapper has no source for.
        let mut extended = c.clone();
        extended.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_header(&extended, &state, 0).is_err());
        // And a hand-built state carrying a player id the fold would reject.
        let mut bad = Header::default();
        bad.player_tag_db_id = Some(MAX_PLAYER_TAG_DB_ID + 1);
        assert!(record_from_header(&c, &bad, 0).is_err());
    }

    #[test]
    fn parsing_never_panics_on_arbitrary_bytes() {
        // `parse_update` and `fold_payload` read a payload straight off a stream
        // on disk; every length arithmetic in them must be total.
        let mut rng = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..4_000 {
            let len = (next() as usize) % 160;
            let mut bytes = vec![0u8; len];
            for b in &mut bytes {
                *b = next() as u8;
            }
            let mut acc = Accumulator::default();
            let _ = fold_payload(&mut acc, &bytes);
            let _ = split_payload(&bytes);
        }
        // And byte-by-byte corruption of a genuinely well-formed payload.
        let good = measured_payload();
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let mut acc = Accumulator::default();
            let _ = fold_payload(&mut acc, &p);
        }
    }
}
