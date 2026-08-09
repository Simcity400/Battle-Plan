//! d95 `pov_spotted_units_live` — the current POV's map-spotted land units and
//! task forces.
//!
//! Derived from `world_fold.rs:12676`-`:12744`
//! (`fold_pov_spotted_units`, dispatched at `:4752`), its payload codec
// @world-fold-authority domain=95 functions=fold_pov_spotted_units lines=12676-12744 sha256=22f810187fac25dbae53375a864cad075f8e9428c503ed00f198bb73bbd1b81e
//! `extractor/native/compact_contract/src/spotted_units.rs` (authority #2 for
//! offsets, per the recipe), the production dispatcher's d95 error arm
//! (`world_fold.rs::apply_domain_data`), the hour-boundary reset (`world_fold.rs::reset_hour_state`) and
//! the collector read that decides when a surface is captured at all
//! (`extractor/native/lanes/spotted_units_live/src/a729.rs:175`-`:202`,
//! `:234`-`:266`). `extractor/documents/domains/` carries no d95 dossier, which
//! `domains/UNDOCUMENTED_DOMAINS.md` says is expected and not a blocker.
//!
//! **Payload grammar** (`spotted_units.rs:102`-`:149`): a 44-byte header, then
//! `land_count` 12-byte rows, then `taskforce_count` 12-byte rows, then the
//! producer-hour trailer. One row form; there is no legacy shorter row.
//!
//! | offset | width | header word | gate |
//! |---|---|---|---|
//! | `+0`  | 4  | magic `SPU1`              | — |
//! | `+4`  | u16 | `version` (must be 1)    | — |
//! | `+6`  | u16 | `flags`                  | — |
//! | `+8`  | u64 | `capture_sequence`       | none; zero rejected (`:74`-`:76`) |
//! | `+16` | u32 | `pov_country_id`         | none; `1..=2048` (`:77`-`:79`) |
//! | `+20` | u32 | `land_count`             | — |
//! | `+24` | u32 | `taskforce_count`        | — |
//! | `+28` | u64 | `land_frame_sequence`    | `FLAG_LAND_FRAME` (`1<<0`) |
//! | `+36` | u64 | `taskforce_frame_sequence` | `FLAG_TASKFORCE_FRAME` (`1<<1`) |
//!
//! **The producer-hour trailer** (`spotted_units.rs:138`-`:147`, read back at
//! `:169`-`:213`). After the last row, one 8-byte little-endian `i64` per SET
//! hour flag, in `(land, taskforce)` order — so the payload is exactly
//! `44 + 12·(land_count + taskforce_count) + 8·hour_words` bytes and nothing a
//! pre-existing reader walks moved.
//!
//! | trailer slot | width | word | gate |
//! |---|---|---|---|
//! | `+0` | i64 | `land_frame_hour_raw`      | `FLAG_LAND_FRAME_HOUR` (`1<<2`); must fit `i32` (`:202`-`:204`) |
//! | next | i64 | `taskforce_frame_hour_raw` | `FLAG_TASKFORCE_FRAME_HOUR` (`1<<3`); must fit `i32` |
//!
//! Each hour word is a **statement by** its producer frame: the codec refuses a
//! payload whose hour flag is set while the matching frame flag is clear
//! (`spotted_units.rs:90`-`:98`), because an hour with no frame would let a
//! consumer age an observation the payload does not carry.
//!
//! The hour is carried as a **presence bit** here and its value is checked and
//! then dropped, which is exactly what `world_fold.rs` does with it: the
//! shipping fold decodes `land_frame_hour_raw`/`taskforce_frame_hour_raw` and
//! reads neither, so nothing this port would encode depends on them and the
//! schema does not move. Carrying them into the stored `capture` record would
//! make the port publish a field the shipping arm does not, which is the one
//! thing a byte-for-byte comparison against the shipping stream may not do.
//! d94's restatement of the same trailer holds the same line (`d94.rs:258`-`:266`).
//!
//! | offset | width | row field | scale |
//! |---|---|---|---|
//! | `+0` | u32 | `type_id`    | raw; zero rejected (`:163`) |
//! | `+4` | u32 | `stable_id`  | raw; zero rejected (`:163`) |
//! | `+8` | u8  | `raw_intel`  | raw; `>= 250` land, `>= 150` task force (`:42`, `:47`, `:163`) |
//! | `+9` | 3 B | reserved     | must be zero (`:118`-`:119`) |
//!
//! **No field on this domain is scaled.** Every header word and every row field
//! is emitted raw: the codec neither multiplies nor divides, and the fold copies
//! the decoded snapshot verbatim (`world_fold.rs::fold_pov_spotted_units`). `raw_intel` in
//! particular is the engine intel byte as the getter returns it
//! (`a729.rs:274`: `original(unit) & 0xff`), not a fixed-point word — the
//! `FIXED_SCALE`/`32768` class of error has no surface here, which the offset
//! test pins anyway because a `u8` wedged between two `u32`s is exactly the
//! transposition shape the recipe warns about.
//!
//! # The four states, and the centrepiece
//!
//! `land_frame_sequence` and `taskforce_frame_sequence` are `Option<u64>` gated
//! by their flag bits (`spotted_units.rs:11`-`:12`, `:128`-`:130`). A clear flag
//! is **not** "this surface is empty": the collector's lane retains the last
//! *complete* producer frame per surface and reports `None` when it has none —
//! because the producer has not run yet, because the retained frame belonged to a
//! different POV country and was dropped by the tie-break
//! (`a729.rs:183`-`:187`), or, for task forces, because the navy debug-override
//! byte was non-zero and no frame was built at all (`a729.rs:256`-`:260`). Every
//! one of those is §3's **unknown** — "applicable, not captured" — so both fields
//! are `.unknown_capable()` and nothing else. A surface with a *set* flag and
//! zero rows is a measured empty surface, and that is a different fact.
//!
//! The fold's own JSON says the same thing in the only vocabulary it has: it
//! publishes `"land": Null` for an unflagged surface and `"land_frame_complete":
//! false` beside it (`world_fold.rs::build_pov_visibility_value`). Per the
//! recipe's step 2 that decides value-vs-nothing only; the three-way choice is
//! made here, from the collector read, and it is not a tie-break.
//!
//! **What that means for the two row collections, stated as a reader contract.**
//! An empty `land`/`taskforces` collection is ambiguous *on its own*: it is a
//! measured-empty surface under a set flag and an unmeasured surface under a
//! clear one. This is the d17 war-graph shape — a keyed set has no way to say
//! "this collection's membership was not measured this hour" (`d17.rs:71`-`:77`)
//! — with one decisive difference, and it is the difference between a stop and a
//! port:
//!
//! - The state has a carrier. d95's payload is a per-POV frame record, so the
//!   `capture` collection exists anyway and holds the per-surface answer, keyed by
//!   the same `pov_country_id` the row collections are keyed by. d17 had no such
//!   record to hang it on.
//! - The carrier cannot disagree with the rows. The codec **refuses** a payload
//!   whose unflagged surface carries any row (`spotted_units.rs:153`-`:156`), so
//!   "unmeasured" and "has rows" is not a representable input, and the annotation
//!   can never contradict the membership it annotates.
//!
//! So the stored pair `(rows empty, capture.<surface>_frame_sequence = Unknown)`
//! carries exactly the information the recorder's own stored pair
//! `(land: vec![], land_frame_sequence: None)` carries — no more and no less.
//! **A reader must read the two together**: an empty surface whose frame sequence
//! is `Unknown` was not measured, and an empty surface whose frame sequence is a
//! value was measured and is empty. Nothing here claims a bare keyed set can
//! express the state; the claim is that this domain always ships the record that
//! disambiguates it. A domain whose fold admitted rows under a clear flag, or
//! that had no frame-level record, would still stop.
//!
//! Measured on `runs/batch3/deepwar_b2`: the task-force surface is unflagged on
//! the first 53 of 1,033 hours (the producer had not yet run) and flagged on the
//! remaining 980; the land surface is flagged on all 1,033. The Unknown state is
//! therefore encoded, delta'd and replayed on real data, not only in tests.
//!
//! # d18 dependency
//!
//! The fold refuses the payload outright unless a `country_identity` (d18) record
//! landed **this hour** and the POV country is in that roster
//! (`world_fold.rs::fold_pov_spotted_units`), clearing the domain-hour in both cases. That
//! is an input the payload does not carry, so it is a typed mapper input
//! ([`CountryRoster`]) rather than something a caller re-derives — the recipe's
//! step-7 rule, and a new member of its measured family.
//!
//! # No derived field (§6)
//!
//! The three sequence words come from one process-wide counter the producer bumps
//! per published frame and per drain (`a729.rs:191`, `:321`, `:348`-`:351`), so
//! their per-hour increment is a function of how often the renderer ran, not of
//! the clock: `stored = v + c * (frame_tick >> u)` cannot hold. The mapper still
//! takes `frame_tick` so every domain's boundary has one shape.

use std::collections::BTreeSet;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 95;

/// The per-POV frame record: one row per hour, carrying the capture sequence and
/// the two per-surface capture states.
pub const CAPTURE: u32 = 1;
/// Spotted land units, `spotted_units.rs:30`.
pub const LAND: u32 = 2;
/// Spotted task forces, `spotted_units.rs:31`.
pub const TASKFORCES: u32 = 3;

pub const CAPTURE_INDEX: usize = 0;
pub const LAND_INDEX: usize = 1;
pub const TASKFORCES_INDEX: usize = 2;
pub const COLLECTION_COUNT: usize = 3;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
/// Ids are unique across the whole domain even though §11 scopes them per
/// collection, so a field id names one concept here.
pub mod field {
    pub const POV_COUNTRY_ID: u32 = 1;
    pub const CAPTURE_SEQUENCE: u32 = 2;
    pub const LAND_FRAME_SEQUENCE: u32 = 3;
    pub const TASKFORCE_FRAME_SEQUENCE: u32 = 4;
    pub const TYPE_ID: u32 = 5;
    pub const STABLE_ID: u32 = 6;
    pub const RAW_INTEL: u32 = 7;
}

// --- Payload grammar, from `compact_contract/src/spotted_units.rs`. ----------

/// `CARRIER_MAGIC`, `spotted_units.rs:7`.
pub const MAGIC: [u8; 4] = *b"SPU1";
/// `CARRIER_VERSION`, `spotted_units.rs:8`.
pub const CARRIER_VERSION: u16 = 1;
/// `HEADER_LEN`, `spotted_units.rs:45`.
pub const HEADER_LEN: usize = 44;
/// `ROW_LEN`, `spotted_units.rs:46`.
pub const ROW_LEN: usize = 12;
/// `HOUR_LEN`, `spotted_units.rs:47`: the width of ONE producer-hour trailer
/// word. The codec writes an `i64` (`:146`) and narrows it back to `i32` on the
/// way in (`:202`-`:204`), so the wire width and the admitted range differ and
/// both are restated.
pub const HOUR_LEN: usize = 8;
/// `FLAG_LAND_FRAME`, `spotted_units.rs:11`.
pub const FLAG_LAND_FRAME: u16 = 1 << 0;
/// `FLAG_TASKFORCE_FRAME`, `spotted_units.rs:12`.
pub const FLAG_TASKFORCE_FRAME: u16 = 1 << 1;
/// `FLAG_LAND_FRAME_HOUR`, `spotted_units.rs:40`. The land frame carries the
/// hour its PRODUCER observed at, in a trailer word after the rows — additive,
/// so the carrier version does not move and a payload written before the field
/// existed decodes byte-identically.
pub const FLAG_LAND_FRAME_HOUR: u16 = 1 << 2;
/// `FLAG_TASKFORCE_FRAME_HOUR`, `spotted_units.rs:42`. The task-force frame's
/// own hour; the two producers publish on different engine calls, so a drain
/// routinely serves one fresh frame beside one retained one and each surface's
/// hour is an independent fact.
pub const FLAG_TASKFORCE_FRAME_HOUR: u16 = 1 << 3;
/// `KNOWN_FLAGS`, `spotted_units.rs:43`-`:44`. Any other bit means the payload
/// is not this grammar (`:157`-`:159`). The two hour bits are DEFINED here, so
/// a stamped payload is admitted for its bits and judged on its length and its
/// words — every bit from `1<<4` up is still refused, which is what keeps
/// widening this mask from being a relaxation.
pub const KNOWN_FLAGS: u16 =
    FLAG_LAND_FRAME | FLAG_TASKFORCE_FRAME | FLAG_LAND_FRAME_HOUR | FLAG_TASKFORCE_FRAME_HOUR;
/// `MAX_POV_COUNTRY_ID`, `spotted_units.rs:9`; the admitted POV range is
/// `1..=2048` (`:39`-`:40`).
pub const MAX_POV_COUNTRY_ID: u32 = 2_048;
/// `MAX_ROWS_PER_SURFACE`, `spotted_units.rs:10`; enforced per surface at
/// `:150`-`:151`.
pub const MAX_ROWS_PER_SURFACE: usize = 131_072;
/// The land surface's map-visibility threshold (`spotted_units.rs:42`,
/// `a729.rs:278`): a land row below it is never published.
pub const LAND_MIN_INTEL: u8 = 250;
/// The task-force surface's threshold (`spotted_units.rs:43`-`:48`,
/// `a729.rs:281`).
pub const TASKFORCE_MIN_INTEL: u8 = 150;

/// Which of the two row tables a row belongs to. The surface is *not* a key
/// component: it is the collection's own identity, which is what keeps the
/// per-surface "not captured" state a per-collection fact (see the module docs).
/// The survey's `(pov_country_id, surface, type_id, stable_id)` identity is
/// therefore carried as `collection_id` plus the three-component key, which is
/// the same 4-tuple with the surface factored out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Land,
    Taskforce,
}

impl Surface {
    pub fn collection_id(self) -> u32 {
        match self {
            Surface::Land => LAND,
            Surface::Taskforce => TASKFORCES,
        }
    }

    pub fn collection_index(self) -> usize {
        match self {
            Surface::Land => LAND_INDEX,
            Surface::Taskforce => TASKFORCES_INDEX,
        }
    }

    /// `validate_surface`'s per-surface `minimum_intel` (`spotted_units.rs:42`
    /// and `:43`-`:48`, applied at `:163`).
    pub fn min_intel(self) -> u8 {
        match self {
            Surface::Land => LAND_MIN_INTEL,
            Surface::Taskforce => TASKFORCE_MIN_INTEL,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Surface::Land => "land",
            Surface::Taskforce => "taskforce",
        }
    }
}

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour
        // (the fold admits several and keeps the last, see [`HourAccumulator`],
        // but the cadence the container samples on is the hour).
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: CAPTURE,
                // The POV country persists across hours, which is what a keyed
                // set needs, and only a keyed set has a delta grammar (§7.3).
                // Full replace per hour: the fold assigns one whole snapshot
                // (`world_fold.rs::fold_pov_spotted_units`).
                shape: Shape::KeyedSet,
                fields: vec![
                    // `pov_country_id` (`spotted_units.rs:98`, header +16). A
                    // country identity is content-defined (§5) — a manifest must
                    // resolve it to keep its meaning under another mod set — so
                    // `ContentId`, exactly as d63 classes `tag_db_id` and d20
                    // classes `owner`. It is also the id the d18 roster is keyed
                    // by (`world_fold.rs::fold_pov_spotted_units`).
                    Field::new(f::POV_COUNTRY_ID, Kind::ContentId).key(0),
                    // The producer's capture counter, monotone by construction
                    // (`a729.rs:348`-`:351`) and required non-zero
                    // (`spotted_units.rs:36`-`:37`), so it is always a concrete value:
                    // no capability is declared. `Monotone` is an encoding hint,
                    // never a validity constraint (§4).
                    Field::new(f::CAPTURE_SEQUENCE, Kind::Monotone),
                    // The two per-surface capture states. Unknown — and ONLY
                    // Unknown — when the flag is clear; see the module docs.
                    Field::new(f::LAND_FRAME_SEQUENCE, Kind::Monotone).unknown_capable(),
                    Field::new(f::TASKFORCE_FRAME_SEQUENCE, Kind::Monotone).unknown_capable(),
                ],
            },
            spotted_collection(LAND),
            spotted_collection(TASKFORCES),
        ],
        event_part: None,
    }
}

/// The two row collections are one grammar under two identities — the pair
/// pattern in miniature. Building both from one function is what makes the
/// `schema_hash` equality test below a real assertion rather than a copy check.
fn spotted_collection(collection_id: u32) -> Collection {
    use field as f;
    Collection {
        collection_id,
        shape: Shape::KeyedSet,
        fields: vec![
            // The POV rides in the key because the whole table is relative to one
            // observing country: it is a frame property, and letting it drift
            // into a per-row column would let two POVs' rows share a key.
            Field::new(f::POV_COUNTRY_ID, Kind::ContentId).key(0),
            // `type_id` is the engine's identity-type discriminator and
            // `stable_id` the runtime instance id, both halves of the packed
            // `CUnit` reference at `unit+0x18` (`a729.rs:333`-`:338`; the fold
            // names it "CUnit_reference_type_id_stable_id",
            // `world_fold.rs::build_pov_visibility_value`). Neither is content: no manifest resolves
            // them, and a `stable_id` is a different unit next campaign. §5's
            // deciding question answers "no", so `Unsigned`, exactly as d20
            // classes `div_type` and `div_id`.
            //
            // **Checked against d52's ruling, which does not reach here.** d52
            // reversed its `occupation_law_token_id` to `ContentId` because that
            // id is a hash of a content-defined law token — the deciding question
            // is whether a manifest MUST resolve it, not whether one can today.
            // d95 carries no hashed token at all: the two halves are read
            // verbatim out of one packed engine word (`a729.rs:333`-`:338`), and
            // neither the lane, the codec nor the fold hashes any string on this
            // path. There is therefore no d52-shaped field to reverse, and no
            // name-hash pre-image gap to record. What WOULD change the answer for
            // `type_id` is evidence that the engine's type index is defined by
            // mod content rather than by the executable; this capture carries
            // zero rows, so it offers none either way (see the open questions).
            Field::new(f::TYPE_ID, Kind::Unsigned).key(1),
            Field::new(f::STABLE_ID, Kind::Unsigned).key(2),
            // The raw intel byte the getter returned (`a729.rs:274`), kept raw.
            // Always a measured value: the codec admits only `>= min_intel`
            // (`spotted_units.rs:163`), so it has no empty state and declares no
            // capability.
            Field::new(f::RAW_INTEL, Kind::Unsigned),
        ],
    }
}

// --- The d18 dependency, as a typed input -----------------------------------

/// The `country_identity` (d18) state the d95 fold reads and the payload does not
/// carry: whether a d18 record measured the roster THIS hour
/// (`world_fold.rs:1944`, reset at `world_fold.rs::reset_hour_state`) and which `tag_db_id`s it carries
/// (`:1933`, full-replaced at `world_fold.rs::fold_country_identity`).
///
/// Two separate facts, deliberately not collapsed: `tag_by_db_id` persists across
/// hours while the known flag resets, so "the roster contains 368" and "a d18
/// record landed this hour" are independent, and the fold requires both
/// (`world_fold.rs::fold_pov_spotted_units`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryRoster {
    known_this_hour: bool,
    tag_db_ids: BTreeSet<u32>,
}

impl CountryRoster {
    /// No d18 record has landed this hour. Every d95 payload is refused against
    /// it, which is what the fold does (`world_fold.rs::fold_pov_spotted_units`).
    pub fn unmeasured() -> Self {
        Self::default()
    }

    /// A roster a d18 record measured this hour.
    pub fn measured<I: IntoIterator<Item = u32>>(tag_db_ids: I) -> Self {
        Self {
            known_this_hour: true,
            tag_db_ids: tag_db_ids.into_iter().collect(),
        }
    }

    /// The hour boundary: `country_identity_known_this_hour` resets, the id map
    /// deliberately persists (`world_fold.rs::reset_hour_state`).
    pub fn begin_hour(&mut self) {
        self.known_this_hour = false;
    }

    /// A d18 record folded successfully this hour, full-replacing the map
    /// (`world_fold.rs::fold_country_identity`).
    pub fn measure<I: IntoIterator<Item = u32>>(&mut self, tag_db_ids: I) {
        self.tag_db_ids = tag_db_ids.into_iter().collect();
        self.known_this_hour = true;
    }

    pub fn known_this_hour(&self) -> bool {
        self.known_this_hour
    }

    pub fn contains(&self, tag_db_id: u32) -> bool {
        self.tag_db_ids.contains(&tag_db_id)
    }

    pub fn len(&self) -> usize {
        self.tag_db_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tag_db_ids.is_empty()
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 44-byte payload header (`spotted_units.rs:88`-`:102`).
///
/// The two frame sequences are `Option` here for the same reason they are
/// `Option` in the codec: the flag and the word are one fact, and keeping them
/// apart invites a caller to read a sequence the flag says was never captured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    pub land_count: u32,
    pub taskforce_count: u32,
    pub land_frame_sequence: Option<u64>,
    pub taskforce_frame_sequence: Option<u64>,
    /// `FLAG_LAND_FRAME_HOUR` (`spotted_units.rs:40`, `:111`-`:113`): this
    /// payload carries a land producer-hour word in its trailer.
    ///
    /// The BIT is a header fact and lives here; the WORD is not, because it
    /// rides after the rows at an offset the two row counts decide, so
    /// [`split_payload`] — which is the only function that sees the whole
    /// payload — is what reads and range-checks it. Keeping the bit here is
    /// what makes [`flags`](Self::flags) a faithful round trip instead of a
    /// lossy one that would re-encode a stamped payload as an unstamped one.
    pub land_frame_hour: bool,
    /// `FLAG_TASKFORCE_FRAME_HOUR` (`spotted_units.rs:42`, `:114`-`:116`); see
    /// [`land_frame_hour`](Self::land_frame_hour).
    pub taskforce_frame_hour: bool,
}

impl PayloadHeader {
    /// The flags word this header encodes back to
    /// (`spotted_units.rs:104`-`:116`, `:125`).
    ///
    /// All four defined bits, not just the two frame bits: a `flags()` that
    /// dropped the hour bits would re-encode a stamped payload into a shorter
    /// one whose trailer the codec then reads as missing.
    pub fn flags(&self) -> u16 {
        let mut flags = 0u16;
        if self.land_frame_sequence.is_some() {
            flags |= FLAG_LAND_FRAME;
        }
        if self.taskforce_frame_sequence.is_some() {
            flags |= FLAG_TASKFORCE_FRAME;
        }
        if self.land_frame_hour {
            flags |= FLAG_LAND_FRAME_HOUR;
        }
        if self.taskforce_frame_hour {
            flags |= FLAG_TASKFORCE_FRAME_HOUR;
        }
        flags
    }

    /// How many 8-byte trailer words this payload carries
    /// (`spotted_units.rs:169`-`:170`).
    pub fn hour_words(&self) -> usize {
        usize::from(self.land_frame_hour) + usize::from(self.taskforce_frame_hour)
    }

    pub fn frame_sequence(&self, surface: Surface) -> Option<u64> {
        match surface {
            Surface::Land => self.land_frame_sequence,
            Surface::Taskforce => self.taskforce_frame_sequence,
        }
    }

    pub fn row_count(&self, surface: Surface) -> u32 {
        match surface {
            Surface::Land => self.land_count,
            Surface::Taskforce => self.taskforce_count,
        }
    }
}

/// Parse and check the 44-byte header alone, without touching the body.
///
/// Every check here is one `SpottedUnitsSnapshot::decode`/`validate` applies to a
/// header word (`spotted_units.rs:88`-`:102`, `:128`-`:138`, `:35`-`:53`). d95's
/// fold has **no** early exit that skips the body — it has no status word at all,
/// and `decode` is atomic — so this split exists for the mapper and the tests,
/// not to reproduce a non-folding path. The accepted set of
/// `parse_header` + [`split_payload`] + the row mapper is exactly `decode`'s;
/// only the order in which two rejections are reported can differ.
pub fn parse_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN || payload[..4] != MAGIC {
        return Err(format!(
            "pov_spotted_units carrier magic/length mismatch ({} bytes)",
            payload.len()
        ));
    }
    let version = u16_at(payload, 4);
    let flags = u16_at(payload, 6);
    if version != CARRIER_VERSION || flags & !KNOWN_FLAGS != 0 {
        return Err(format!(
            "pov_spotted_units carrier version {version}/flags {flags:#x} mismatch"
        ));
    }
    let capture_sequence = u64_at(payload, 8);
    let pov_country_id = u32_at(payload, 16);
    let land_count = u32_at(payload, 20);
    let taskforce_count = u32_at(payload, 24);
    let land_sequence = u64_at(payload, 28);
    let taskforce_sequence = u64_at(payload, 36);

    // `spotted_units.rs:134`-`:138`: an absent frame must pack a zero sequence.
    // The pair (flag clear, sequence non-zero) is a contradiction, not data.
    if flags & FLAG_LAND_FRAME == 0 && land_sequence != 0 {
        return Err("pov_spotted_units absent land frame has a nonzero sequence".into());
    }
    if flags & FLAG_TASKFORCE_FRAME == 0 && taskforce_sequence != 0 {
        return Err("pov_spotted_units absent taskforce frame has a nonzero sequence".into());
    }
    let header = PayloadHeader {
        capture_sequence,
        pov_country_id,
        land_count,
        taskforce_count,
        land_frame_sequence: (flags & FLAG_LAND_FRAME != 0).then_some(land_sequence),
        taskforce_frame_sequence: (flags & FLAG_TASKFORCE_FRAME != 0).then_some(taskforce_sequence),
        land_frame_hour: flags & FLAG_LAND_FRAME_HOUR != 0,
        taskforce_frame_hour: flags & FLAG_TASKFORCE_FRAME_HOUR != 0,
    };
    // `validate`, `spotted_units.rs:35`-`:53`.
    if header.capture_sequence == 0 {
        return Err("pov_spotted_units capture sequence is zero".into());
    }
    if header.pov_country_id == 0 || header.pov_country_id > MAX_POV_COUNTRY_ID {
        return Err(format!(
            "pov_spotted_units POV country {} is outside the admitted range",
            header.pov_country_id
        ));
    }
    for surface in [Surface::Land, Surface::Taskforce] {
        let count = header.row_count(surface);
        if count as usize > MAX_ROWS_PER_SURFACE {
            return Err(format!(
                "pov_spotted_units {} row count {count} exceeds the admitted ceiling",
                surface.name()
            ));
        }
        match header.frame_sequence(surface) {
            // An unflagged surface must be rowless. This is the invariant the
            // module docs' reader contract rests on: "not captured" and "carries
            // rows" is not a representable input, so the `capture` record's
            // Unknown can never contradict a non-empty collection.
            None if count != 0 => {
                return Err(format!(
                    "pov_spotted_units {} rows lack a complete frame",
                    surface.name()
                ))
            }
            Some(0) => {
                return Err(format!(
                    "pov_spotted_units {} frame sequence is zero",
                    surface.name()
                ))
            }
            _ => {}
        }
    }
    // `spotted_units.rs:87`-`:89`: a snapshot with neither frame is refused
    // outright — the producer published nothing at all.
    if header.land_frame_sequence.is_none() && header.taskforce_frame_sequence.is_none() {
        return Err("pov_spotted_units snapshot has no complete producer frame".into());
    }
    // `spotted_units.rs:90`-`:98`, and the last two rules `validate` applies. A
    // producer hour is a statement BY a producer frame: without the frame there
    // is nothing for it to date, and admitting the pair would let a consumer age
    // an observation that is not in the payload. This is a FLAG-only rule — both
    // halves are bits at `+6` — so it belongs in the header parse even though
    // the word it guards lives in the trailer, and it is the refusal that stops
    // widening `KNOWN_FLAGS` from admitting `1<<2` as a free-floating bit.
    if header.land_frame_hour && header.land_frame_sequence.is_none() {
        return Err("pov_spotted_units land producer hour has no land frame".into());
    }
    if header.taskforce_frame_hour && header.taskforce_frame_sequence.is_none() {
        return Err("pov_spotted_units taskforce producer hour has no taskforce frame".into());
    }
    Ok(header)
}

/// Split a d95 DATA payload into its header and its two row slices, applying the
/// codec's structural checks (`spotted_units.rs:166`-`:213`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
///
/// The producer-hour trailer is sized and range-checked HERE and nowhere else,
/// because it is the only boundary that sees the whole payload: its offset is
/// `44 + 12·(land_count + taskforce_count)`, which the header alone does not fix
/// a position for. The two row slices this returns therefore stop at the last
/// row — a trailer folded into the task-force slice would be read as rows.
pub fn split_payload(
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    let header = parse_header(payload)?;
    // Checked arithmetic: the counts are attacker-shaped input from a stream on
    // disk (`spotted_units.rs:166`-`:178` does the same).
    let row_count = (header.land_count as usize)
        .checked_add(header.taskforce_count as usize)
        .ok_or("pov_spotted_units row count overflow")?;
    let rows_end = ROW_LEN
        .checked_mul(row_count)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or("pov_spotted_units length overflow")?;
    // `spotted_units.rs:169`-`:178`: one 8-byte word per SET hour flag, and the
    // total is exact in both directions — a trailing byte is as much a refusal
    // as a truncation.
    let expected = rows_end
        .checked_add(HOUR_LEN.saturating_mul(header.hour_words()))
        .ok_or("pov_spotted_units length overflow")?;
    if payload.len() != expected {
        return Err(format!(
            "pov_spotted_units carrier is truncated or has trailing bytes: {} != {expected}",
            payload.len()
        ));
    }
    // `spotted_units.rs:199`-`:213`: the codec writes an `i64` and narrows it
    // back to `i32`, refusing a word that does not fit. Checking the trailer's
    // PRESENCE but not its VALUE would leave a payload the recorder's own
    // authority refuses and this restatement accepts — precisely the drift the
    // restatement exists to make impossible. Land first, then task force, which
    // is both the order the words are written in and the order they are read.
    let mut at = rows_end;
    for (present, surface) in [
        (header.land_frame_hour, Surface::Land),
        (header.taskforce_frame_hour, Surface::Taskforce),
    ] {
        if !present {
            continue;
        }
        let word = i64::from_le_bytes(payload[at..at + HOUR_LEN].try_into().expect("8 bytes"));
        if i32::try_from(word).is_err() {
            return Err(format!(
                "pov_spotted_units {} producer hour {word} is outside the engine's date range",
                surface.name()
            ));
        }
        at += HOUR_LEN;
    }
    let land_end = HEADER_LEN + ROW_LEN * header.land_count as usize;
    Ok((
        header,
        &payload[HEADER_LEN..land_end],
        &payload[land_end..rows_end],
    ))
}

/// The key of the `capture` row: the POV country (`world_fold.rs::fold_pov_spotted_units`).
pub fn key_from_capture_row(header_bytes: &[u8]) -> core::result::Result<Key, String> {
    Ok(key_from_capture_header(&parse_header(header_bytes)?))
}

/// [`key_from_capture_row`] over an already-parsed header, so a caller that has
/// walked the payload once does not parse it twice.
pub fn key_from_capture_header(header: &PayloadHeader) -> Key {
    key(&[header.pov_country_id as u64])
}

/// The 44-byte payload header, as the collector emits it today, onto the stored
/// `capture` Record. `header_bytes` is the raw header slice — for this collection
/// the frame header *is* the row, which is why the step-8 offset test covers
/// every header word through this mapper.
///
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d95 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed: a header the codec would reject is an `Err` here, never a coerced
/// value. Nothing in this function can panic on any byte slice.
pub fn record_from_capture_row(
    collection: &Collection,
    header_bytes: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let header = parse_header(header_bytes)?;
    record_from_capture_header(collection, &header, frame_tick)
}

/// [`record_from_capture_row`] over an already-parsed header.
pub fn record_from_capture_header(
    collection: &Collection,
    header: &PayloadHeader,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d95 has no derived field; §6's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    // A record built for the wrong collection would otherwise come out looking
    // plausible under another collection's field ids.
    if collection.collection_id != CAPTURE {
        return Err(format!(
            "pov_spotted_units capture mapper called with collection {}",
            collection.collection_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::CAPTURE_SEQUENCE => Value::Unsigned(header.capture_sequence),
            // A clear flag means the producer had no complete frame for this
            // surface — captured nothing, rather than measured nothing. §3's
            // Unknown; see the module docs for why it is not Null or Absent.
            field::LAND_FRAME_SEQUENCE => gated(header.land_frame_sequence),
            field::TASKFORCE_FRAME_SEQUENCE => gated(header.taskforce_frame_sequence),
            other => {
                return Err(format!(
                    "pov_spotted_units capture has no mapping for field id {other}"
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// A frame sequence the producer did not capture is `unknown`, never a
/// fabricated zero and never null — see the module docs.
fn gated(sequence: Option<u64>) -> Value {
    match sequence {
        Some(value) => Value::Unsigned(value),
        None => Value::Unknown,
    }
}

/// The key of one spotted row: the POV country it was spotted by, then the
/// engine handle's two halves (`spotted_units.rs:115`-`:117`, `:163`).
/// The parent's id comes first, as d23 and d83 do it, so a caller that has
/// already parsed the frame header does not parse it twice.
pub fn key_from_spotted_row(
    pov_country_id: u32,
    row: &[u8],
) -> core::result::Result<Key, String> {
    check_row_len(row)?;
    let type_id = u32_at(row, 0);
    let stable_id = u32_at(row, 4);
    if type_id == 0 || stable_id == 0 {
        return Err("pov_spotted_units row is outside the admitted range".into());
    }
    Ok(key(&[pov_country_id as u64, type_id as u64, stable_id as u64]))
}

/// One compact row of the `land`/`taskforces` table, as the collector emits it
/// today, onto its stored Record. `row` is the raw 12-byte row slice (the caller
/// has already consumed the frame header via [`split_payload`]); d95 has exactly
/// one row form, so length selects nothing and a wrong length is rejected.
/// `surface` is the typed input that says which of the two tables this row came
/// from — the codec's admitted intel band differs between them
/// (`spotted_units.rs:42`, `:47`), so mapping a task-force row as a land row would
/// admit rows the land surface refuses.
///
/// `frame_tick` is §6's derived-field input; d95 declares no derived field, so it
/// is taken and ignored.
///
/// Fail-closed throughout, mirroring the codec: a row it would reject is an `Err`
/// here, never a coerced value. Nothing here can panic on any byte slice.
pub fn record_from_spotted_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
    surface: Surface,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    // The two surfaces have identical field lists, so the id check is the only
    // thing standing between a task-force row and a record filed under `land`.
    if collection.collection_id != surface.collection_id() {
        return Err(format!(
            "pov_spotted_units {} mapper called with collection {}",
            surface.name(),
            collection.collection_id
        ));
    }
    check_row_len(row)?;
    // `spotted_units.rs:163`: zero halves and a below-threshold intel byte are
    // outside the admitted range for the whole payload, not just this row.
    let _ = key_from_spotted_row(1, row)?;
    let raw_intel = row[8];
    if raw_intel < surface.min_intel() {
        return Err(format!(
            "pov_spotted_units {} row intel {raw_intel} is below the admitted minimum {}",
            surface.name(),
            surface.min_intel()
        ));
    }
    // `spotted_units.rs:118`-`:119`: the three pad bytes must be zero. A row that
    // used them would be a different grammar carrying data this mapper drops.
    if row[9..12] != [0u8; 3] {
        return Err("pov_spotted_units row reserved bytes are nonzero".into());
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::RAW_INTEL => Value::Unsigned(raw_intel as u64),
            other => {
                return Err(format!(
                    "pov_spotted_units {} has no mapping for field id {other}",
                    surface.name()
                ))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The frame-level fold ----------------------------------------------------

/// One hour's stored d95 state: the frame record and the two row tables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpottedUnitsState {
    pub capture: KeyedState,
    pub land: KeyedState,
    pub taskforces: KeyedState,
}

impl SpottedUnitsState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The three collections in schema order, for `encode_baseline`/`encode_delta`.
    pub fn as_slice(&self) -> [KeyedState; COLLECTION_COUNT] {
        [
            self.capture.clone(),
            self.land.clone(),
            self.taskforces.clone(),
        ]
    }

    pub fn get(&self, index: usize) -> &KeyedState {
        match index {
            CAPTURE_INDEX => &self.capture,
            LAND_INDEX => &self.land,
            _ => &self.taskforces,
        }
    }

    pub fn rows(&self) -> usize {
        self.capture.len() + self.land.len() + self.taskforces.len()
    }
}

/// The stored snapshot the recorder keeps for the hour: exactly
/// `pov_spotted_units_this_hour` (`world_fold.rs:1840`), retained as the raw
/// payload so the state is rebuilt through the public mappers rather than through
/// a second, parallel decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSnapshot {
    pub header: PayloadHeader,
    land_rows: Vec<u8>,
    taskforce_rows: Vec<u8>,
}

impl StoredSnapshot {
    pub fn rows(&self, surface: Surface) -> &[u8] {
        match surface {
            Surface::Land => &self.land_rows,
            Surface::Taskforce => &self.taskforce_rows,
        }
    }
}

/// The recorder's per-hour d95 accumulator: `pov_spotted_units_this_hour` and
/// `pov_spotted_units_seen_this_hour` (`world_fold.rs:1840`-`:1841`).
///
/// **Error contract, mirrored rather than chosen.** `fold_pov_spotted_units`
/// leaves the stored snapshot untouched when the payload fails to decode
/// (`world_fold.rs::fold_pov_spotted_units`: the `?` runs before any mutation) and *clears* it on the
/// three semantic refusals (`world_fold.rs::fold_pov_spotted_units` — the `= None` lines) —
/// but production reaches
/// the fold through a dispatcher whose d95 arm clears the snapshot on **every**
/// `Err` (`world_fold.rs::apply_domain_data`). The composed production effect is what a harness must
/// reproduce, so [`HourAccumulator::fold_payload`] clears on every error path and
/// both anchors are cited here rather than one being silently preferred.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HourAccumulator {
    seen: bool,
    stored: Option<StoredSnapshot>,
}

impl HourAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// The hour boundary (`world_fold.rs::reset_hour_state`): both the snapshot and
    /// the seen flag reset. d95 keeps nothing across hours.
    pub fn begin_hour(&mut self) {
        self.seen = false;
        self.stored = None;
    }

    /// `pov_spotted_units_seen_this_hour` — set by every d95 record, valid or not
    /// (`world_fold.rs:10539`, `:4927`).
    pub fn seen(&self) -> bool {
        self.seen
    }

    /// The hour's stored snapshot, or `None` when no record folded.
    pub fn stored(&self) -> Option<&StoredSnapshot> {
        self.stored.as_ref()
    }

    /// Fold one d95 DATA payload, exactly as `fold_pov_spotted_units` does
    /// (`world_fold.rs::fold_pov_spotted_units`) under the production dispatcher.
    ///
    /// `roster` is the d18 state the fold reads and the payload does not carry.
    /// Several records in one hour are legal and are folded in stream order: a
    /// payload whose `capture_sequence` strictly increases over the stored one
    /// replaces it (`world_fold.rs::fold_pov_spotted_units`, the assignment at `world_fold.rs::fold_pov_spotted_units`), and one that
    /// does not CLEARS the stored snapshot (`world_fold.rs::fold_pov_spotted_units`) rather than ending the
    /// hour — a cleared accumulator has nothing to compare against, so the next
    /// record stores whatever it carries. That is why this is an accumulator and
    /// not a one-shot decode.
    pub fn fold_payload(
        &mut self,
        payload: &[u8],
        roster: &CountryRoster,
    ) -> core::result::Result<(), String> {
        self.seen = true;
        let stored = match self.decode_stored(payload, roster) {
            Ok(stored) => stored,
            Err(error) => {
                // Every non-folding path clears the domain-hour; see the type's
                // docs for the two anchors this composes.
                self.stored = None;
                return Err(error);
            }
        };
        self.stored = Some(stored);
        Ok(())
    }

    fn decode_stored(
        &self,
        payload: &[u8],
        roster: &CountryRoster,
    ) -> core::result::Result<StoredSnapshot, String> {
        let (header, land, taskforces) = split_payload(payload)
            .map_err(|error| format!("pov_spotted_units SPU1 invalid: {error}"))?;
        // The rows must also be admissible, which `decode` decides before the
        // fold ever sees the snapshot (`spotted_units.rs:114`-`:123`, `:160`-`:171`).
        check_rows(land, Surface::Land)?;
        check_rows(taskforces, Surface::Taskforce)?;
        // `world_fold.rs::fold_pov_spotted_units`.
        if !roster.known_this_hour() {
            return Err("pov_spotted_units requires a current-hour D18 identity roster".into());
        }
        // `world_fold.rs::fold_pov_spotted_units`.
        if !roster.contains(header.pov_country_id) {
            return Err(format!(
                "pov_spotted_units POV country {} is absent from current D18 roster",
                header.pov_country_id
            ));
        }
        // `world_fold.rs::fold_pov_spotted_units`: within one hour the capture sequence
        // must strictly increase, and a violation clears what was already stored.
        if self
            .stored
            .as_ref()
            .is_some_and(|current| header.capture_sequence <= current.header.capture_sequence)
        {
            return Err("pov_spotted_units sequence is not strictly increasing".into());
        }
        Ok(StoredSnapshot {
            header,
            land_rows: land.to_vec(),
            taskforce_rows: taskforces.to_vec(),
        })
    }

    /// Build the hour's wire state from the stored snapshot, or `None` when the
    /// hour stored nothing — which is an hour the recorder publishes no d95 state
    /// for, and therefore an hour a measurement excludes rather than encodes.
    pub fn state(
        &self,
        schema: &Schema,
    ) -> core::result::Result<Option<SpottedUnitsState>, String> {
        let Some(stored) = self.stored.as_ref() else {
            return Ok(None);
        };
        state_from_stored(schema, stored, 0).map(Some)
    }
}

/// Fold one stored snapshot into its three keyed collections.
///
/// Separate from [`HourAccumulator`] so the tests and the harness share one walk,
/// and so a caller holding a snapshot can build its state without an accumulator.
pub fn state_from_stored(
    schema: &Schema,
    stored: &StoredSnapshot,
    frame_tick: u64,
) -> core::result::Result<SpottedUnitsState, String> {
    if schema.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "pov_spotted_units schema has {} collections, expected {COLLECTION_COUNT}",
            schema.collections.len()
        ));
    }
    let mut state = SpottedUnitsState::new();
    let capture_collection = &schema.collections[CAPTURE_INDEX];
    state.capture.insert(
        key_from_capture_header(&stored.header),
        record_from_capture_header(capture_collection, &stored.header, frame_tick)?,
    );
    for surface in [Surface::Land, Surface::Taskforce] {
        let collection = &schema.collections[surface.collection_index()];
        let target = match surface {
            Surface::Land => &mut state.land,
            Surface::Taskforce => &mut state.taskforces,
        };
        let mut previous: Option<(u32, u32)> = None;
        for row in stored.rows(surface).chunks_exact(ROW_LEN) {
            let identity = (u32_at(row, 0), u32_at(row, 4));
            // `spotted_units.rs:166`-`:170`: rows are unique and strictly
            // ascending by identity. Mirrored rather than assumed — this is the
            // insert where a silent overwrite would happen.
            if previous.is_some_and(|prior| prior >= identity) {
                return Err(format!(
                    "pov_spotted_units {} rows are not unique and sorted",
                    surface.name()
                ));
            }
            previous = Some(identity);
            let row_key = key_from_spotted_row(stored.header.pov_country_id, row)?;
            let record = record_from_spotted_row(collection, row, frame_tick, surface)?;
            if target.insert(row_key, record).is_some() {
                return Err(format!(
                    "pov_spotted_units {} repeats a unit identity",
                    surface.name()
                ));
            }
        }
        // `split_payload` has already made the length exact, so this can only
        // differ if the two disagree — which is precisely the drift worth failing
        // on.
        if target.len() != stored.header.row_count(surface) as usize {
            return Err(format!(
                "pov_spotted_units {} body carries {} rows, header says {}",
                surface.name(),
                target.len(),
                stored.header.row_count(surface)
            ));
        }
    }
    Ok(state)
}

/// The per-row admissibility the codec applies before the fold sees a snapshot.
/// Runs over the raw slice so a payload is refused whole, exactly as `decode` is
/// all-or-nothing (`spotted_units.rs:114`-`:123`, `:160`-`:171`).
fn check_rows(rows: &[u8], surface: Surface) -> core::result::Result<(), String> {
    if rows.len() % ROW_LEN != 0 {
        return Err(format!(
            "pov_spotted_units {} row region {} is not a multiple of {ROW_LEN}",
            surface.name(),
            rows.len()
        ));
    }
    let mut previous: Option<(u32, u32)> = None;
    for row in rows.chunks_exact(ROW_LEN) {
        let identity = (u32_at(row, 0), u32_at(row, 4));
        if previous.is_some_and(|prior| prior >= identity) {
            return Err(format!(
                "pov_spotted_units {} rows are not unique and sorted",
                surface.name()
            ));
        }
        previous = Some(identity);
        if identity.0 == 0 || identity.1 == 0 || row[8] < surface.min_intel() {
            return Err(format!(
                "pov_spotted_units {} row is outside the admitted range",
                surface.name()
            ));
        }
        if row[9..12] != [0u8; 3] {
            return Err("pov_spotted_units row reserved bytes are nonzero".into());
        }
    }
    Ok(())
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "pov_spotted_units row length {} != {ROW_LEN}",
            row.len()
        ));
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

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn capture_collection() -> Collection {
        schema().collections[CAPTURE_INDEX].clone()
    }

    fn surface_collection(surface: Surface) -> Collection {
        schema().collections[surface.collection_index()].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 12-byte row, built the way the codec packs one
    /// (`spotted_units.rs:79`-`:84`).
    fn raw_row(type_id: u32, stable_id: u32, raw_intel: u8) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&type_id.to_le_bytes());
        row.extend_from_slice(&stable_id.to_le_bytes());
        row.push(raw_intel);
        row.extend_from_slice(&[0u8; 3]);
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A raw 44-byte header, built the way the codec packs one
    /// (`spotted_units.rs:123`-`:131`).
    fn raw_header(
        capture_sequence: u64,
        pov_country_id: u32,
        land: (Option<u64>, u32),
        taskforce: (Option<u64>, u32),
    ) -> Vec<u8> {
        raw_header_with_hours(capture_sequence, pov_country_id, land, taskforce, (false, false))
    }

    /// The same, with the two producer-hour FLAG bits set independently of the
    /// trailer — so a test can build a header whose bits and whose length
    /// disagree, which is the malformed shape the length check must catch.
    fn raw_header_with_hours(
        capture_sequence: u64,
        pov_country_id: u32,
        land: (Option<u64>, u32),
        taskforce: (Option<u64>, u32),
        hours: (bool, bool),
    ) -> Vec<u8> {
        let mut flags = 0u16;
        if land.0.is_some() {
            flags |= FLAG_LAND_FRAME;
        }
        if taskforce.0.is_some() {
            flags |= FLAG_TASKFORCE_FRAME;
        }
        if hours.0 {
            flags |= FLAG_LAND_FRAME_HOUR;
        }
        if hours.1 {
            flags |= FLAG_TASKFORCE_FRAME_HOUR;
        }
        let mut out = Vec::with_capacity(HEADER_LEN);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&CARRIER_VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&capture_sequence.to_le_bytes());
        out.extend_from_slice(&pov_country_id.to_le_bytes());
        out.extend_from_slice(&land.1.to_le_bytes());
        out.extend_from_slice(&taskforce.1.to_le_bytes());
        out.extend_from_slice(&land.0.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&taskforce.0.unwrap_or(0).to_le_bytes());
        assert_eq!(out.len(), HEADER_LEN);
        out
    }

    fn payload(
        capture_sequence: u64,
        pov_country_id: u32,
        land: (Option<u64>, &[Vec<u8>]),
        taskforce: (Option<u64>, &[Vec<u8>]),
    ) -> Vec<u8> {
        payload_with_hours(
            capture_sequence,
            pov_country_id,
            land,
            taskforce,
            (None, None),
        )
    }

    /// A whole payload with the producer-hour trailer appended AFTER the rows,
    /// one 8-byte little-endian `i64` per carried hour, land first
    /// (`spotted_units.rs:138`-`:147`).
    fn payload_with_hours(
        capture_sequence: u64,
        pov_country_id: u32,
        land: (Option<u64>, &[Vec<u8>]),
        taskforce: (Option<u64>, &[Vec<u8>]),
        hours: (Option<i32>, Option<i32>),
    ) -> Vec<u8> {
        let mut out = raw_header_with_hours(
            capture_sequence,
            pov_country_id,
            (land.0, land.1.len() as u32),
            (taskforce.0, taskforce.1.len() as u32),
            (hours.0.is_some(), hours.1.is_some()),
        );
        for row in land.1.iter().chain(taskforce.1.iter()) {
            out.extend_from_slice(row);
        }
        for hour in [hours.0, hours.1].into_iter().flatten() {
            out.extend_from_slice(&i64::from(hour).to_le_bytes());
        }
        out
    }

    fn both_surfaces_payload() -> Vec<u8> {
        payload(
            9,
            368,
            (Some(7), &[raw_row(3, 11, 250), raw_row(4, 2, 255)]),
            (Some(8), &[raw_row(5, 9, 150)]),
        )
    }

    fn roster() -> CountryRoster {
        CountryRoster::measured([1, 368, 900])
    }

    fn folded(payload: &[u8]) -> HourAccumulator {
        let mut hour = HourAccumulator::new();
        hour.fold_payload(payload, &roster()).expect("folds");
        hour
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d95 schema is well-formed");
    }

    #[test]
    fn the_shape_and_keys_are_what_the_fold_identifies_rows_by() {
        let s = schema();
        assert_eq!(s.collections.len(), COLLECTION_COUNT);
        for c in &s.collections {
            assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        }

        let capture = &s.collections[CAPTURE_INDEX];
        assert_eq!(capture.collection_id, CAPTURE);
        let keys = capture.key_fields();
        assert_eq!(keys.len(), 1, "the frame record is keyed by its POV country");
        assert_eq!(keys[0].field_id, field::POV_COUNTRY_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country identity is content-defined (§5)"
        );
        assert!(keys[0].kind.legal_as_key());

        for surface in [Surface::Land, Surface::Taskforce] {
            let c = &s.collections[surface.collection_index()];
            assert_eq!(c.collection_id, surface.collection_id());
            let keys = c.key_fields();
            assert_eq!(
                keys.iter().map(|f| (f.field_id, f.kind)).collect::<Vec<_>>(),
                vec![
                    (field::POV_COUNTRY_ID, Kind::ContentId),
                    (field::TYPE_ID, Kind::Unsigned),
                    (field::STABLE_ID, Kind::Unsigned),
                ],
                "{}: the survey's (pov, surface, type_id, stable_id) identity, with the \
                 surface factored into the collection",
                surface.name()
            );
            assert!(keys.iter().all(|f| f.kind.legal_as_key()));
            assert_eq!(c.fields.len(), 4, "3 key components + raw_intel");
        }
    }

    #[test]
    fn the_two_surfaces_are_one_grammar_under_two_identities() {
        // The pair pattern's machine check: the row collections must stay
        // byte-identical grammars, so a later edit cannot give one surface a
        // field the other lacks.
        let s = schema();
        let land = &s.collections[LAND_INDEX];
        let taskforces = &s.collections[TASKFORCES_INDEX];
        assert_eq!(land.fields, taskforces.fields);
        assert_eq!(land.shape, taskforces.shape);
        assert_ne!(land.collection_id, taskforces.collection_id);
    }

    #[test]
    fn exactly_the_two_frame_sequences_are_unknown_capable() {
        // The schema declares exactly the states the mapper emits. Absent would
        // claim a surface is inapplicable to a POV country; Null would claim we
        // know the producer's frame is nothing. A clear flag means the lane held
        // no complete frame (`a729.rs:175`-`:199`), which is §3's "applicable,
        // not captured".
        let s = schema();
        let capture = &s.collections[CAPTURE_INDEX];
        assert_eq!(capture.optional_count(), 0, "no field is ever inapplicable");
        assert!(capture.value_fields().all(|f| !f.nullable));
        assert_eq!(
            capture
                .value_fields()
                .filter(|f| f.unknown_capable)
                .map(|f| f.field_id)
                .collect::<Vec<_>>(),
            vec![field::LAND_FRAME_SEQUENCE, field::TASKFORCE_FRAME_SEQUENCE],
        );
        // The row collections declare NO capability at all: `raw_intel` is
        // admitted only in a band that excludes every empty state, and the three
        // key components are concrete by §11. There is therefore no
        // absent/null/unknown case to build for them, which this records rather
        // than leaving as a silently missing test.
        for surface in [Surface::Land, Surface::Taskforce] {
            let c = &s.collections[surface.collection_index()];
            assert_eq!(c.optional_count(), 0, "{}", surface.name());
            assert!(c.value_fields().all(|f| !f.nullable && !f.unknown_capable));
        }
    }

    #[test]
    fn no_field_is_continuous_so_no_quantization_is_declared() {
        // Step 4's "quantization saves bytes" test does not apply: d95 has no
        // continuous field at all. Every value it carries is an integer the codec
        // emits raw — two producer counters, an engine handle and an intel byte —
        // so there is nothing to quantize and nothing that could be rounded.
        for c in schema().collections {
            for f in c.value_fields() {
                assert_ne!(f.kind, Kind::Continuous, "field {}", f.field_id);
                assert_eq!(f.quant, Quant::None, "field {}", f.field_id);
            }
        }
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; the producer sequences
        // move with renderer frames, not with the clock.
        for c in schema().collections {
            assert!(c.fields.iter().all(|f| f.derived.is_none()));
        }
    }

    #[test]
    fn the_content_id_sweep_is_a_result_not_a_silence() {
        // Every ContentId in this domain, so the §13 manifest-assignment retrofit
        // can find them: `pov_country_id`, and it is a KEY in all three
        // collections — the heavier retrofit class, because key components sort
        // and delta-encode by raw value.
        let mut content_keys = Vec::new();
        let mut content_values = Vec::new();
        for c in schema().collections {
            for f in &c.fields {
                if f.kind == Kind::ContentId {
                    if f.is_key() {
                        content_keys.push((c.collection_id, f.field_id));
                    } else {
                        content_values.push((c.collection_id, f.field_id));
                    }
                }
            }
        }
        assert_eq!(
            content_keys,
            vec![
                (CAPTURE, field::POV_COUNTRY_ID),
                (LAND, field::POV_COUNTRY_ID),
                (TASKFORCES, field::POV_COUNTRY_ID),
            ]
        );
        assert!(content_values.is_empty(), "no ContentId value field");
        // And the other half of the sweep, recorded as a result rather than a
        // silence: no field here is a hashed content token of d52's shape. Every
        // non-key value is a producer counter or the raw intel byte, and the two
        // Unsigned key components are halves of one packed engine word that no
        // string ever passed through (`a729.rs:333`-`:338`). The assertion is on
        // the field list itself, so a later field that DID carry a token would
        // have to be classified here rather than slipping in unclassified.
        let value_kinds: Vec<(u32, u32, Kind)> = schema()
            .collections
            .iter()
            .flat_map(|c| {
                c.value_fields()
                    .map(move |f| (c.collection_id, f.field_id, f.kind))
            })
            .collect();
        assert_eq!(
            value_kinds,
            vec![
                (CAPTURE, field::CAPTURE_SEQUENCE, Kind::Monotone),
                (CAPTURE, field::LAND_FRAME_SEQUENCE, Kind::Monotone),
                (CAPTURE, field::TASKFORCE_FRAME_SEQUENCE, Kind::Monotone),
                (LAND, field::RAW_INTEL, Kind::Unsigned),
                (TASKFORCES, field::RAW_INTEL, Kind::Unsigned),
            ],
            "every value field is a counter or the raw intel byte; none is a content token"
        );
    }

    /// MUTATION RUN: make `flags()` blind to the two hour bits, so the header
    /// re-encodes a stamped payload into an unstamped one. OBSERVED: FAILED --
    /// ``assertion `left == right` failed: the header must encode back to the
    /// flags word it was read from / left: 3 / right: 7``.
    #[test]
    fn every_header_word_reads_at_its_own_offset() {
        // The header IS the `capture` row, so this is both the header test the
        // recipe requires and the offset test for that collection. Distinct
        // values everywhere: a transposed pair fails.
        let c = capture_collection();
        let header_bytes = raw_header(0x1122_3344_5566_7788, 1_234, (Some(0x2a), 0), (Some(0x2b), 0));
        let header = parse_header(&header_bytes).expect("parses");
        assert_eq!(header.capture_sequence, 0x1122_3344_5566_7788);
        assert_eq!(header.pov_country_id, 1_234);
        assert_eq!(header.land_count, 0);
        assert_eq!(header.taskforce_count, 0);
        assert_eq!(header.land_frame_sequence, Some(0x2a));
        assert_eq!(header.taskforce_frame_sequence, Some(0x2b));
        assert!(!header.land_frame_hour && !header.taskforce_frame_hour);
        assert_eq!(header.flags(), FLAG_LAND_FRAME | FLAG_TASKFORCE_FRAME);
        assert_eq!(header.hour_words(), 0);

        // The flags word round-trips through the header for all four defined
        // bits, one at a time as well as together — a `flags()` blind to the
        // hour bits would re-encode a stamped payload into a shorter one whose
        // trailer the codec then reads as missing. `1<<2` and `1<<3` only ever
        // read back as themselves if they are read at their own bit positions.
        for (land_hour, taskforce_hour) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let bytes = raw_header_with_hours(
                9,
                368,
                (Some(0x2a), 0),
                (Some(0x2b), 0),
                (land_hour, taskforce_hour),
            );
            let mut stamped = bytes.clone();
            for hour in [land_hour, taskforce_hour].iter().filter(|set| **set) {
                let _ = hour;
                stamped.extend_from_slice(&0i64.to_le_bytes());
            }
            let header = parse_header(&stamped).expect("parses");
            assert_eq!(header.land_frame_hour, land_hour);
            assert_eq!(header.taskforce_frame_hour, taskforce_hour);
            assert_eq!(
                header.flags(),
                u16_at(&stamped, 6),
                "the header must encode back to the flags word it was read from"
            );
            assert_eq!(
                header.hour_words(),
                usize::from(land_hour) + usize::from(taskforce_hour)
            );
            assert!(split_payload(&stamped).is_ok());
        }
        assert_eq!(
            parse_header(&raw_header_with_hours(
                9,
                368,
                (Some(0x2a), 0),
                (Some(0x2b), 0),
                (true, true)
            ))
            .expect("parses")
            .flags(),
            KNOWN_FLAGS,
            "all four defined bits at once IS `KNOWN_FLAGS`"
        );

        let r = record_from_capture_row(&c, &header_bytes, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CAPTURE_SEQUENCE)],
            Value::Unsigned(0x1122_3344_5566_7788)
        );
        assert_eq!(
            r.values[index_of(&c, field::LAND_FRAME_SEQUENCE)],
            Value::Unsigned(0x2a)
        );
        assert_eq!(
            r.values[index_of(&c, field::TASKFORCE_FRAME_SEQUENCE)],
            Value::Unsigned(0x2b)
        );
        assert_eq!(key_from_capture_row(&header_bytes).expect("keys"), key(&[1_234]));

        // The two row counts are read at their own offsets too, which only a
        // payload with different counts per surface can show.
        let p = payload(
            5,
            7,
            (Some(1), &[raw_row(1, 1, 250), raw_row(2, 1, 250)]),
            (Some(2), &[raw_row(9, 9, 200)]),
        );
        let (header, land, taskforces) = split_payload(&p).expect("splits");
        assert_eq!((header.land_count, header.taskforce_count), (2, 1));
        assert_eq!(land.len(), 2 * ROW_LEN);
        assert_eq!(taskforces.len(), ROW_LEN);
    }

    #[test]
    fn every_row_field_reads_at_its_own_offset() {
        // The offset test that matters here: `raw_intel` is one byte wedged
        // between two `u32`s and three pad bytes, so a slice off by one reads a
        // plausible value. Distinct values everywhere.
        for surface in [Surface::Land, Surface::Taskforce] {
            let c = surface_collection(surface);
            let row = raw_row(0x0102_0304, 0x0a0b_0c0d, 251);
            assert_eq!(
                key_from_spotted_row(368, &row).expect("keys"),
                key(&[368, 0x0102_0304, 0x0a0b_0c0d]),
                "{}",
                surface.name()
            );
            let r = record_from_spotted_row(&c, &row, 0, surface).expect("maps");
            assert_eq!(r.values.len(), 1, "raw_intel is the only value field");
            assert_eq!(
                r.values[index_of(&c, field::RAW_INTEL)],
                Value::Unsigned(251),
                "{}",
                surface.name()
            );
            // The intel byte is raw: no scale, no fixed-point division. 255 and
            // the surface's own minimum both read back as themselves.
            for intel in [surface.min_intel(), 251, 255] {
                let r =
                    record_from_spotted_row(&c, &raw_row(1, 1, intel), 0, surface).expect("maps");
                assert_eq!(
                    r.values[index_of(&c, field::RAW_INTEL)],
                    Value::Unsigned(intel as u64)
                );
            }
        }
    }

    #[test]
    fn a_full_record_roundtrips_byte_for_byte() {
        let schema = schema();
        let hour = folded(&both_surfaces_payload());
        let state = hour
            .state(&schema)
            .expect("state builds")
            .expect("the hour stored a snapshot");
        for (index, collection) in schema.collections.iter().enumerate() {
            for record in state.get(index).values() {
                let mut bytes = Vec::new();
                record.encode(collection, &mut bytes).expect("encodes");
                let (back, pos) = Record::decode(collection, &bytes, 0).expect("decodes");
                assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
                assert_eq!(&back, record, "decode reproduces the record");
                let mut reencoded = Vec::new();
                back.encode(collection, &mut reencoded).expect("re-encodes");
                assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
            }
        }
    }

    #[test]
    fn an_unmeasured_surface_is_unknown_and_survives_a_roundtrip() {
        // The centrepiece, through the public mapper on raw bytes: with a flag
        // clear the surface's sequence is Unknown — not zero, not null, not
        // absent — and the state survives encode/decode.
        let c = capture_collection();
        let cases = [
            (Some(4u64), None, "task force unmeasured"),
            (None, Some(4u64), "land unmeasured"),
        ];
        for (land, taskforce, label) in cases {
            let bytes = raw_header(9, 368, (land, 0), (taskforce, 0));
            let r = record_from_capture_row(&c, &bytes, 0).unwrap_or_else(|e| panic!("{label}: {e}"));
            let expect = |seq: Option<u64>| match seq {
                Some(v) => Value::Unsigned(v),
                None => Value::Unknown,
            };
            assert_eq!(
                r.values[index_of(&c, field::LAND_FRAME_SEQUENCE)],
                expect(land),
                "{label}"
            );
            assert_eq!(
                r.values[index_of(&c, field::TASKFORCE_FRAME_SEQUENCE)],
                expect(taskforce),
                "{label}"
            );
            // Unknown is never the zero it is packed as on the wire.
            assert_ne!(
                r.values[index_of(&c, field::LAND_FRAME_SEQUENCE)],
                Value::Unsigned(0),
                "{label}"
            );
            let mut encoded = Vec::new();
            r.encode(&c, &mut encoded).expect("encodes");
            let (back, pos) = Record::decode(&c, &encoded, 0).expect("decodes");
            assert_eq!(pos, encoded.len());
            assert_eq!(back, r, "{label}: the unknown state survives");
        }
    }

    #[test]
    fn a_measured_empty_surface_is_not_an_unmeasured_one() {
        // The distinction the whole design exists to keep: both hours store zero
        // land rows, and the two states are different records.
        let c = capture_collection();
        let measured = record_from_capture_row(&c, &raw_header(9, 368, (Some(4), 0), (Some(5), 0)), 0)
            .expect("maps");
        let unmeasured =
            record_from_capture_row(&c, &raw_header(9, 368, (None, 0), (Some(5), 0)), 0)
                .expect("maps");
        assert_ne!(measured, unmeasured);
        assert_eq!(
            unmeasured.values[index_of(&c, field::LAND_FRAME_SEQUENCE)],
            Value::Unknown
        );
        // And the row collections are equally empty in both, which is exactly why
        // the reader must consult the frame record.
        let schema = schema();
        for header in [
            raw_header(9, 368, (Some(4), 0), (Some(5), 0)),
            raw_header(9, 368, (None, 0), (Some(5), 0)),
        ] {
            let hour = folded(&header);
            let state = hour.state(&schema).expect("builds").expect("stored");
            assert!(state.land.is_empty() && state.taskforces.is_empty());
            assert_eq!(state.capture.len(), 1);
        }
    }

    #[test]
    fn a_real_value_is_never_collapsed_into_an_empty_state() {
        // Sequence 1 is the smallest value the codec admits and must read as a
        // value; the intel minimum likewise. §3 keeps real values out of the
        // three empty states.
        let c = capture_collection();
        let r = record_from_capture_row(&c, &raw_header(1, 1, (Some(1), 0), (Some(1), 0)), 0)
            .expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::CAPTURE_SEQUENCE)],
            Value::Unsigned(1)
        );
        assert_eq!(
            r.values[index_of(&c, field::LAND_FRAME_SEQUENCE)],
            Value::Unsigned(1)
        );
        for surface in [Surface::Land, Surface::Taskforce] {
            let sc = surface_collection(surface);
            let r = record_from_spotted_row(&sc, &raw_row(1, 1, surface.min_intel()), 0, surface)
                .expect("maps");
            assert_eq!(
                r.values[index_of(&sc, field::RAW_INTEL)],
                Value::Unsigned(surface.min_intel() as u64)
            );
        }
    }

    /// MUTATION RUN: delete the two orphan-hour refusals from `parse_header`,
    /// so an hour flag may outlive the frame flag it dates. OBSERVED: FAILED --
    /// `assertion failed: parse_header(&raw_header_with_hours(9, 368, (None, 0),
    /// (Some(1), 0), (true, false))).is_err()`.
    ///
    /// This is the ONE mutation in this lane the differential test in
    /// `bin/bench_real_d95.rs` does not also kill, and the reason is worth
    /// stating rather than leaving as a coincidence: the codec's `validate`
    /// refuses an orphan hour, so `encode` can never build one, and the
    /// differential corpus's byte patches (`0x00`, `0x01`, `0xff`) cannot reach
    /// a flags word with an hour bit set and its frame bit clear. An orphan
    /// payload only exists if something hand-builds it, which is what this test
    /// does.
    ///
    /// MUTATION RUN: widen `KNOWN_FLAGS` to `u16::MAX`. OBSERVED: FAILED --
    /// `flag bit 4`.
    #[test]
    fn parse_header_rejects_every_header_the_codec_rejects() {
        // Short of the header, at every length.
        for len in 0..HEADER_LEN {
            let bytes = raw_header(9, 368, (Some(1), 0), (Some(1), 0));
            assert!(parse_header(&bytes[..len]).is_err(), "length {len}");
        }
        // Magic, at every byte of it.
        for i in 0..4 {
            let mut bytes = raw_header(9, 368, (Some(1), 0), (Some(1), 0));
            bytes[i] ^= 0xff;
            assert!(parse_header(&bytes).is_err(), "magic byte {i}");
        }
        // Version.
        for version in [0u16, 2, u16::MAX] {
            let mut bytes = raw_header(9, 368, (Some(1), 0), (Some(1), 0));
            bytes[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(parse_header(&bytes).is_err(), "version {version}");
        }
        // Every unknown flag bit, not one sample. The carrier now DEFINES four
        // bits, so the sweep starts at `1<<4`: widening `KNOWN_FLAGS` moved the
        // boundary and did not remove it.
        for bit in 4..16u16 {
            let mut bytes = raw_header(9, 368, (Some(1), 0), (Some(1), 0));
            bytes[6..8].copy_from_slice(&(KNOWN_FLAGS | (1 << bit)).to_le_bytes());
            assert!(parse_header(&bytes).is_err(), "flag bit {bit}");
        }
        // And the mask really is exactly those four: a payload carrying only the
        // undefined bit — with no other bit set — is refused for the bit and not
        // as a side effect of some other rule.
        for bit in 4..16u16 {
            let mut bytes = raw_header(9, 368, (Some(1), 0), (Some(1), 0));
            bytes[6..8]
                .copy_from_slice(&(FLAG_LAND_FRAME | FLAG_TASKFORCE_FRAME | (1 << bit)).to_le_bytes());
            let reason = parse_header(&bytes).expect_err("flag bit {bit}");
            assert!(reason.contains("flags"), "flag bit {bit}: {reason}");
        }
        // A producer hour with no producer frame (`spotted_units.rs:90`-`:98`):
        // the hour bit is DEFINED, so this must be refused by its own rule
        // rather than by the unknown-bit mask. Both surfaces, and the surviving
        // frame keeps the payload otherwise well-formed.
        assert!(parse_header(&raw_header_with_hours(
            9,
            368,
            (None, 0),
            (Some(1), 0),
            (true, false)
        ))
        .is_err());
        assert!(parse_header(&raw_header_with_hours(
            9,
            368,
            (Some(1), 0),
            (None, 0),
            (false, true)
        ))
        .is_err());
        // ...and the matching frame makes each of them legal, so the rule is a
        // pairing rule and not a ban on the bit.
        for hours in [(true, false), (false, true), (true, true)] {
            assert!(parse_header(&raw_header_with_hours(
                9,
                368,
                (Some(1), 0),
                (Some(1), 0),
                hours
            ))
            .is_ok());
        }
        // Zero capture sequence, zero and out-of-range POV.
        assert!(parse_header(&raw_header(0, 368, (Some(1), 0), (Some(1), 0))).is_err());
        assert!(parse_header(&raw_header(9, 0, (Some(1), 0), (Some(1), 0))).is_err());
        assert!(parse_header(&raw_header(
            9,
            MAX_POV_COUNTRY_ID + 1,
            (Some(1), 0),
            (Some(1), 0)
        ))
        .is_err());
        // ...and the boundary itself is accepted, so the bound is not off by one.
        assert!(parse_header(&raw_header(9, MAX_POV_COUNTRY_ID, (Some(1), 0), (Some(1), 0))).is_ok());
        // A flagged frame with a zero sequence word.
        for bytes in [
            raw_header(9, 368, (Some(0), 0), (Some(1), 0)),
            raw_header(9, 368, (Some(1), 0), (Some(0), 0)),
        ] {
            assert!(parse_header(&bytes).is_err());
        }
        // An unflagged frame with a non-zero sequence word: the flag and the word
        // are one fact, and a contradiction is not data.
        for offset in [28usize, 36] {
            let mut bytes = raw_header(9, 368, (None, 0), (Some(1), 0));
            if offset == 36 {
                bytes = raw_header(9, 368, (Some(1), 0), (None, 0));
            }
            bytes[offset..offset + 8].copy_from_slice(&7u64.to_le_bytes());
            assert!(parse_header(&bytes).is_err(), "offset {offset}");
        }
        // Neither frame present (`spotted_units.rs:49`-`:51`).
        assert!(parse_header(&raw_header(9, 368, (None, 0), (None, 0))).is_err());
        // A row count above the per-surface ceiling.
        for surface in [Surface::Land, Surface::Taskforce] {
            let counts = match surface {
                Surface::Land => ((MAX_ROWS_PER_SURFACE + 1) as u32, 0),
                Surface::Taskforce => (0, (MAX_ROWS_PER_SURFACE + 1) as u32),
            };
            let bytes = raw_header(9, 368, (Some(1), counts.0), (Some(1), counts.1));
            assert!(parse_header(&bytes).is_err(), "{}", surface.name());
        }
        // An unflagged surface claiming rows: the invariant the reader contract
        // rests on.
        assert!(parse_header(&raw_header(9, 368, (None, 1), (Some(1), 0))).is_err());
        assert!(parse_header(&raw_header(9, 368, (Some(1), 0), (None, 1))).is_err());
    }

    #[test]
    fn split_payload_enforces_the_codecs_structural_rules() {
        let p = both_surfaces_payload();
        let (header, land, taskforces) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.capture_sequence, 9);
        assert_eq!(land.len(), 2 * ROW_LEN);
        assert_eq!(taskforces.len(), ROW_LEN);

        // Truncated and trailing bytes, both directions.
        let mut short = p.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = p.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());

        // A count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length. The ceiling catches this one
        // first, which is itself the property worth pinning.
        let mut huge = p.clone();
        huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        huge[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
    }

    /// Law: the producer-hour trailer is sized, range-checked and kept OUT of
    /// the row slices — the restatement accepts exactly the stamped payloads the
    /// SPU1 codec emits and no others.
    ///
    /// This is the defect this test exists for: the codec grew the trailer and
    /// `KNOWN_FLAGS` here still read `bit0|bit1`, so `parse_header` hard-refused
    /// every payload the codec now stamps. Widening the mask alone would have
    /// traded that refusal for a worse one — a trailer read as task-force rows.
    ///
    /// MUTATION RUN: drop `FLAG_LAND_FRAME_HOUR|FLAG_TASKFORCE_FRAME_HOUR` from
    /// `KNOWN_FLAGS`, i.e. the state this lane found. OBSERVED: FAILED --
    /// `a land-stamped payload splits: "pov_spotted_units carrier version
    /// 1/flags 0x7 mismatch"`.
    ///
    /// MUTATION RUN: widen `KNOWN_FLAGS` to `u16::MAX`, the lazy fix. OBSERVED:
    /// FAILED in `every_header_word_reads_at_its_own_offset` -- ``assertion
    /// `left == right` failed: all four defined bits at once IS `KNOWN_FLAGS` /
    /// left: 15 / right: 65535``, and in
    /// `parse_header_rejects_every_header_the_codec_rejects` -- `flag bit 4`.
    ///
    /// MUTATION RUN: drop the `hour_words` term from `expected` in
    /// `split_payload`, i.e. mis-size the trailer. OBSERVED: FAILED --
    /// `a land-stamped payload splits: "pov_spotted_units carrier is truncated
    /// or has trailing bytes: 88 != 80"`.
    ///
    /// MUTATION RUN: slice the task-force rows as `&payload[land_end..]` again.
    /// OBSERVED: FAILED -- ``assertion `left == right` failed: the trailer is
    /// not a row / left: 20 / right: 12``.
    ///
    /// MUTATION RUN: check the trailer's presence but not its value, i.e. drop
    /// the `i32::try_from` refusal. OBSERVED: FAILED --
    /// `hour word 4294967296 at slot 0`.
    ///
    /// MUTATION RUN: read both trailer words at `rows_end` instead of advancing
    /// `at` by `HOUR_LEN`. OBSERVED: FAILED --
    /// `hour word 4294967296 at slot 1`, the task-force slot: slot 0 still
    /// reads its own legal word, so only the SECOND word exposes the cursor.
    #[test]
    fn split_payload_carries_the_producer_hour_trailer_the_codec_emits() {
        let rows = [raw_row(3, 11, 250), raw_row(4, 2, 255)];
        let taskforce_rows = [raw_row(5, 9, 150)];
        for (land_hour, taskforce_hour, words) in [
            (None, None, 0usize),
            (Some(4_100), None, 1),
            (None, Some(4_099), 1),
            (Some(4_100), Some(4_099), 2),
            (Some(-7), Some(0), 2),
            (Some(i32::MIN), Some(i32::MAX), 2),
        ] {
            let p = payload_with_hours(
                9,
                368,
                (Some(7), &rows),
                (Some(8), &taskforce_rows),
                (land_hour, taskforce_hour),
            );
            assert_eq!(p.len(), HEADER_LEN + 3 * ROW_LEN + HOUR_LEN * words);
            let (header, land, taskforces) =
                split_payload(&p).expect("a land-stamped payload splits");
            assert_eq!(header.land_frame_hour, land_hour.is_some());
            assert_eq!(header.taskforce_frame_hour, taskforce_hour.is_some());
            assert_eq!(header.hour_words(), words);
            assert_eq!(land.len(), 2 * ROW_LEN);
            assert_eq!(taskforces.len(), ROW_LEN, "the trailer is not a row");
            // And it folds end to end: the trailer must not reach `check_rows`.
            let hour = folded(&p);
            let stored = hour.stored().expect("a stamped payload stores");
            assert_eq!(stored.rows(Surface::Taskforce).len(), ROW_LEN);
            let state = hour.state(&schema()).expect("builds").expect("stored");
            assert_eq!(state.land.len(), 2);
            assert_eq!(state.taskforces.len(), 1);
        }

        // The trailer is exact in both directions, exactly as the rows are.
        let stamped = payload_with_hours(
            9,
            368,
            (Some(7), &rows),
            (Some(8), &taskforce_rows),
            (Some(4_100), Some(4_099)),
        );
        for cut in 1..=(2 * HOUR_LEN) {
            assert!(
                split_payload(&stamped[..stamped.len() - cut]).is_err(),
                "a trailer short by {cut} bytes must not split"
            );
        }
        let mut padded = stamped.clone();
        padded.push(0);
        assert!(split_payload(&padded).is_err());

        // A flag with no trailer at all, and a trailer with no flag: the bit and
        // the word are one fact.
        let mut flag_without_trailer = both_surfaces_payload();
        flag_without_trailer[6..8].copy_from_slice(&KNOWN_FLAGS.to_le_bytes());
        assert!(split_payload(&flag_without_trailer).is_err());
        let mut trailer_without_flag = both_surfaces_payload();
        trailer_without_flag.extend_from_slice(&0i64.to_le_bytes());
        assert!(split_payload(&trailer_without_flag).is_err());

        // The trailer's VALUE, not only its presence: the codec narrows the
        // `i64` to `i32` and refuses a word that does not fit, so a restatement
        // that accepted one would drift from the recorder's own authority.
        for word in [1i64 << 32, i64::MIN, i64::MAX, i64::from(i32::MAX) + 1] {
            // ...at BOTH trailer slots, which only advancing `at` can get right.
            for slot in 0..2usize {
                let mut out_of_range = stamped.clone();
                let at = out_of_range.len() - (2 - slot) * HOUR_LEN;
                out_of_range[at..at + HOUR_LEN].copy_from_slice(&word.to_le_bytes());
                assert!(
                    split_payload(&out_of_range).is_err(),
                    "hour word {word} at slot {slot}"
                );
            }
        }
        // ...and the two edges that DO fit are still accepted, so the bound is
        // not off by one.
        for word in [i64::from(i32::MIN), i64::from(i32::MAX)] {
            let mut edge = stamped.clone();
            let at = edge.len() - 2 * HOUR_LEN;
            edge[at..at + HOUR_LEN].copy_from_slice(&word.to_le_bytes());
            edge[at + HOUR_LEN..].copy_from_slice(&word.to_le_bytes());
            assert!(split_payload(&edge).is_ok(), "hour word {word}");
        }
    }

    #[test]
    fn the_mapper_rejects_every_row_the_codec_rejects() {
        for surface in [Surface::Land, Surface::Taskforce] {
            let c = surface_collection(surface);
            // Wrong length, both directions, including the empty slice.
            for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1] {
                let row = vec![0u8; len];
                assert!(record_from_spotted_row(&c, &row, 0, surface).is_err());
                assert!(key_from_spotted_row(368, &row).is_err());
            }
            // Zero halves of the handle.
            assert!(record_from_spotted_row(&c, &raw_row(0, 5, 255), 0, surface).is_err());
            assert!(record_from_spotted_row(&c, &raw_row(5, 0, 255), 0, surface).is_err());
            // Every intel byte below the surface's own minimum, and the minimum
            // itself accepted — the bound is not off by one, and the two surfaces
            // genuinely differ.
            for intel in 0..surface.min_intel() {
                assert!(
                    record_from_spotted_row(&c, &raw_row(1, 1, intel), 0, surface).is_err(),
                    "{} intel {intel}",
                    surface.name()
                );
            }
            assert!(
                record_from_spotted_row(&c, &raw_row(1, 1, surface.min_intel()), 0, surface).is_ok()
            );
            // Non-zero reserved bytes, each of the three.
            for i in 9..12 {
                let mut row = raw_row(1, 1, 255);
                row[i] = 1;
                assert!(record_from_spotted_row(&c, &row, 0, surface).is_err(), "byte {i}");
            }
        }
        // A land row mapped as a task-force row would admit intel the land
        // surface refuses, which is why the surface is a typed input.
        let land = surface_collection(Surface::Land);
        assert!(record_from_spotted_row(&land, &raw_row(1, 1, 200), 0, Surface::Land).is_err());
        assert!(
            record_from_spotted_row(&surface_collection(Surface::Taskforce), &raw_row(1, 1, 200), 0, Surface::Taskforce)
                .is_ok()
        );
    }

    #[test]
    fn a_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another collection must not get a record full of
        // placeholders under this collection's field ids. The two row collections
        // have identical field lists, so only the id check separates them.
        let capture = capture_collection();
        let land = surface_collection(Surface::Land);
        let taskforces = surface_collection(Surface::Taskforce);
        let header = raw_header(9, 368, (Some(1), 0), (Some(1), 0));
        assert!(record_from_capture_row(&land, &header, 0).is_err());
        assert!(record_from_spotted_row(&capture, &raw_row(1, 1, 255), 0, Surface::Land).is_err());
        assert!(record_from_spotted_row(&taskforces, &raw_row(1, 1, 255), 0, Surface::Land).is_err());
        assert!(record_from_spotted_row(&land, &raw_row(1, 1, 255), 0, Surface::Taskforce).is_err());
        // And a field id this mapper does not know is refused rather than filled.
        let mut extended = capture.clone();
        extended.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_capture_row(&extended, &header, 0).is_err());
        let mut extended_rows = land.clone();
        extended_rows.fields.push(Field::new(999, Kind::Unsigned));
        assert!(
            record_from_spotted_row(&extended_rows, &raw_row(1, 1, 255), 0, Surface::Land).is_err()
        );
    }

    #[test]
    fn the_fold_requires_a_current_hour_d18_roster() {
        // `world_fold.rs::fold_pov_spotted_units`: both halves, and both clear the hour.
        let p = both_surfaces_payload();
        let mut hour = HourAccumulator::new();
        assert!(hour.fold_payload(&p, &CountryRoster::unmeasured()).is_err());
        assert!(hour.seen(), "a refused record was still seen");
        assert!(hour.stored().is_none());

        // A roster that carries the POV but was not measured this hour is still a
        // refusal: the two facts are independent.
        let mut stale = CountryRoster::measured([368]);
        stale.begin_hour();
        assert!(!stale.known_this_hour() && stale.contains(368));
        assert!(hour.fold_payload(&p, &stale).is_err());

        // Measured this hour, but the POV is not in it.
        assert!(hour
            .fold_payload(&p, &CountryRoster::measured([1, 2, 3]))
            .is_err());

        // And the same payload folds against a roster that carries it.
        assert!(hour.fold_payload(&p, &roster()).is_ok());
        assert!(hour.stored().is_some());
    }

    #[test]
    fn a_later_record_in_one_hour_replaces_the_earlier_one() {
        // `world_fold.rs::fold_pov_spotted_units`: several records in one hour are legal;
        // the last strictly-increasing one wins.
        let schema = schema();
        let mut hour = HourAccumulator::new();
        hour.fold_payload(&payload(5, 368, (Some(1), &[]), (None, &[])), &roster())
            .expect("first folds");
        hour.fold_payload(
            &payload(6, 368, (Some(2), &[raw_row(1, 1, 250)]), (Some(3), &[])),
            &roster(),
        )
        .expect("second folds");
        let state = hour.state(&schema).expect("builds").expect("stored");
        assert_eq!(state.land.len(), 1, "the later record's rows are the state");
        let capture = &schema.collections[CAPTURE_INDEX];
        let record = state.capture.values().next().expect("one frame row");
        assert_eq!(
            record.values[index_of(capture, field::CAPTURE_SEQUENCE)],
            Value::Unsigned(6)
        );
        assert_eq!(
            record.values[index_of(capture, field::TASKFORCE_FRAME_SEQUENCE)],
            Value::Unsigned(3),
            "the earlier record's Unknown is replaced, not merged"
        );

        // A non-increasing sequence CLEARS the stored snapshot rather than being
        // ignored — and clearing is not ending the hour: the accumulator then has
        // nothing to compare against, so the next record stores whatever it
        // carries, however small (`world_fold.rs::fold_pov_spotted_units`).
        for sequence in [1u64, 6] {
            let mut hour = HourAccumulator::new();
            hour.fold_payload(&payload(6, 368, (Some(1), &[]), (None, &[])), &roster())
                .expect("first folds");
            assert!(hour
                .fold_payload(&payload(sequence, 368, (Some(2), &[]), (None, &[])), &roster())
                .is_err());
            assert!(
                hour.stored().is_none(),
                "sequence {sequence} must clear the hour, not leave the earlier state"
            );
            // The third record, below every sequence seen so far, is stored.
            hour.fold_payload(&payload(1, 368, (Some(3), &[raw_row(2, 2, 250)]), (None, &[])), &roster())
                .expect("a cleared accumulator compares against nothing");
            let stored = hour.stored().expect("stored");
            assert_eq!(stored.header.capture_sequence, 1);
        }

        // The hour boundary drops everything (`world_fold.rs::reset_hour_state`), so
        // the next hour's first record may carry any sequence above zero.
        let mut hour = HourAccumulator::new();
        hour.fold_payload(&payload(9, 368, (Some(1), &[]), (None, &[])), &roster())
            .expect("folds");
        hour.begin_hour();
        assert!(!hour.seen() && hour.stored().is_none());
        assert!(hour
            .fold_payload(&payload(2, 368, (Some(1), &[]), (None, &[])), &roster())
            .is_ok());
    }

    #[test]
    fn a_malformed_payload_clears_the_hour_and_is_refused() {
        // The production dispatcher's d95 arm clears the snapshot on every Err
        // (`world_fold.rs::apply_domain_data`), including the decode failures the fold
        // function itself would leave alone.
        let mut hour = HourAccumulator::new();
        hour.fold_payload(&payload(5, 368, (Some(1), &[]), (None, &[])), &roster())
            .expect("folds");
        assert!(hour.fold_payload(&[], &roster()).is_err());
        assert!(hour.stored().is_none());

        // Rows the codec refuses refuse the whole payload, not just the row.
        let cases: [(&str, Vec<u8>); 4] = [
            (
                "unsorted",
                payload(
                    5,
                    368,
                    (Some(1), &[raw_row(4, 1, 250), raw_row(3, 1, 250)]),
                    (None, &[]),
                ),
            ),
            (
                "duplicate",
                payload(
                    5,
                    368,
                    (Some(1), &[raw_row(3, 1, 250), raw_row(3, 1, 251)]),
                    (None, &[]),
                ),
            ),
            (
                "below land minimum",
                payload(5, 368, (Some(1), &[raw_row(3, 1, 249)]), (None, &[])),
            ),
            (
                "below taskforce minimum",
                payload(5, 368, (Some(1), &[]), (Some(2), &[raw_row(3, 1, 149)])),
            ),
        ];
        for (label, p) in cases {
            let mut hour = HourAccumulator::new();
            assert!(hour.fold_payload(&p, &roster()).is_err(), "{label}");
            assert!(hour.stored().is_none(), "{label}");
        }
    }

    #[test]
    fn the_state_is_every_row_of_both_surfaces() {
        let schema = schema();
        let hour = folded(&both_surfaces_payload());
        let state = hour.state(&schema).expect("builds").expect("stored");
        assert_eq!(state.capture.len(), 1);
        assert_eq!(state.land.len(), 2);
        assert_eq!(state.taskforces.len(), 1);
        assert_eq!(state.rows(), 4);
        // The same identity on both surfaces is two different rows: the surface
        // is the collection, so nothing collides.
        let p = payload(
            5,
            368,
            (Some(1), &[raw_row(3, 11, 250)]),
            (Some(2), &[raw_row(3, 11, 150)]),
        );
        let state = folded(&p).state(&schema).expect("builds").expect("stored");
        assert_eq!(state.land.len(), 1);
        assert_eq!(state.taskforces.len(), 1);
        let land_intel = state.land.values().next().expect("row").values[0];
        let taskforce_intel = state.taskforces.values().next().expect("row").values[0];
        assert_eq!(land_intel, Value::Unsigned(250));
        assert_eq!(taskforce_intel, Value::Unsigned(150));
    }

    #[test]
    fn an_hour_that_stored_nothing_has_no_state() {
        let schema = schema();
        let mut hour = HourAccumulator::new();
        assert_eq!(hour.state(&schema).expect("builds"), None);
        assert!(hour.fold_payload(&[], &roster()).is_err());
        assert_eq!(
            hour.state(&schema).expect("builds"),
            None,
            "a refused record leaves no state to encode"
        );
    }

    #[test]
    fn state_from_stored_refuses_a_schema_that_is_not_this_domains() {
        let hour = folded(&both_surfaces_payload());
        let stored = hour.stored().expect("stored");
        let mut wrong = schema();
        wrong.collections.pop();
        assert!(state_from_stored(&wrong, stored, 0).is_err());
    }

    #[test]
    fn the_unmeasured_surface_survives_the_delta_path() {
        // End to end, the way the harness encodes it: a measured hour, then an
        // hour whose task-force surface was not captured, then a measured one.
        // The Unknown must arrive at the reader through a DELTA, not only in a
        // baseline.
        let schema = schema();
        let hours = [
            payload(5, 368, (Some(1), &[raw_row(3, 11, 250)]), (Some(2), &[])),
            payload(6, 368, (Some(3), &[raw_row(3, 11, 250)]), (None, &[])),
            payload(7, 368, (Some(4), &[raw_row(3, 11, 251)]), (Some(5), &[])),
        ];
        let states: Vec<SpottedUnitsState> = hours
            .iter()
            .map(|p| folded(p).state(&schema).expect("builds").expect("stored"))
            .collect();

        let mut bytes = Vec::new();
        encode_baseline(&schema, &states[0].as_slice(), &mut bytes).expect("baseline");
        let (mut reader, consumed) = decode_baseline(&schema, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());

        for (index, state) in states.iter().enumerate().skip(1) {
            let prior = &states[index - 1];
            let deltas: Vec<Option<crate::body::KeyedDelta>> = (0..COLLECTION_COUNT)
                .map(|i| {
                    let d = diff(&schema.collections[i], prior.get(i), state.get(i));
                    (!d.is_empty()).then_some(d)
                })
                .collect();
            let mut delta_bytes = Vec::new();
            encode_delta(&schema, &prior.as_slice(), &deltas, &mut delta_bytes).expect("delta");
            let (decoded, consumed) =
                decode_delta(&schema, &reader, &delta_bytes, 0).expect("decodes");
            assert_eq!(consumed, delta_bytes.len());
            for (i, delta) in decoded.iter().enumerate() {
                if let Some(delta) = delta {
                    apply(&mut reader[i], delta);
                }
            }
            for i in 0..COLLECTION_COUNT {
                assert!(
                    diff(&schema.collections[i], &reader[i], state.get(i)).is_empty(),
                    "hour {index} collection {i} is not stored-equal after replay"
                );
            }
        }

        // And the middle hour really did carry the Unknown across the wire.
        let capture = &schema.collections[CAPTURE_INDEX];
        let unmeasured = states[1].capture.values().next().expect("frame row");
        assert_eq!(
            unmeasured.values[index_of(capture, field::TASKFORCE_FRAME_SEQUENCE)],
            Value::Unknown
        );
    }

    #[test]
    fn nothing_panics_on_arbitrary_payload_bytes() {
        // The mappers are public and read bytes from a stream on disk: every
        // rejection must be a diagnosis, never an unwind.
        let schema = schema();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..8_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 200;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(&MAGIC);
            }
            let _ = split_payload(&bytes);
            let _ = parse_header(&bytes);
            let _ = key_from_capture_row(&bytes);
            let _ = record_from_capture_row(&capture_collection(), &bytes, 0);
            let _ = key_from_spotted_row(1, &bytes);
            for surface in [Surface::Land, Surface::Taskforce] {
                let _ = record_from_spotted_row(&surface_collection(surface), &bytes, 0, surface);
            }
            let mut hour = HourAccumulator::new();
            let _ = hour.fold_payload(&bytes, &roster());
            let _ = hour.state(&schema);
        }
    }
}
