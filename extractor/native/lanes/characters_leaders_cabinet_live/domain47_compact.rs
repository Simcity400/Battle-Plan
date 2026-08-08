use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

#[cfg(not(test))]
use compact_contract::{parse_compact_record_header, RecordFramingError};

pub const DOMAIN_ID: u16 = 47;
pub const DOMAIN_NAME: &str = "characters_leaders_cabinet_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const HEADER_LEN: usize = 64;
pub const STRING_NONE: u32 = u32::MAX;
pub const SKILL_NONE: u32 = u32::MAX;
pub const BYTE_NONE: u8 = u8::MAX;
pub const EXPERIENCE_RAW_NONE: i32 = i32::MIN;
pub const MAX_STRING_BYTES: usize = 160;
pub const MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_CHARACTER_ID: u32 = 2_000_000;
pub const MAX_UNIT_LEADER_ID: u32 = i32::MAX as u32;
pub const MAX_TRAITS_PER_LEADER: u16 = 64;
pub const MAX_SKILL: u32 = 15;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const SCHEMA_ROOTS_VALIDATED: u32 = 1;
pub const SCHEMA_STATIC_NAME_CLOSURE: u32 = 1 << 1;
pub const SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS: u32 = 1 << 2;
/// The header word at offset 60 carries a [`FAMILY_ALL`]-domain presence mask
/// naming which row families this payload OBSERVED.
///
/// Without this flag the word is reserved and must be zero, and the payload
/// means what every d47 payload has always meant: a full snapshot of all eight
/// families. That is why the flag is additive rather than a version bump —
/// `schema_flags` is already this domain's capability-declaration axis, and the
/// fold already conditions on two of its bits.
pub const SCHEMA_FAMILY_PRESENCE_DECLARED: u32 = 1 << 3;

/// Every assigned `schema_flags` bit. An unassigned bit has no defined meaning,
/// so a payload carrying one is refused rather than interpreted — the same rule
/// the shared status vocabulary applies to unassigned status bits.
pub const SCHEMA_ALL_ASSIGNED: u32 = SCHEMA_ROOTS_VALIDATED
    | SCHEMA_STATIC_NAME_CLOSURE
    | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS
    | SCHEMA_FAMILY_PRESENCE_DECLARED;

pub const ROW_CHARACTER: u8 = 1;
pub const ROW_UNIT_LEADER: u8 = 2;
pub const ROW_UNIT_LEADER_TRAIT: u8 = 3;
pub const ROW_COUNTRY_LEADER_ROLE: u8 = 4;
pub const ROW_ACTIVE_RULER: u8 = 5;
pub const ROW_ADVISOR_ROSTER: u8 = 6;
pub const ROW_APPOINTED_ADVISOR: u8 = 7;
pub const ROW_CHARACTER_PORTRAIT: u8 = 8;

/// One bit per row family, positionally derived from the row kind so the two
/// vocabularies cannot drift: family bit `n` is row kind `n + 1`.
///
/// # What a family bit means, and what it does not
///
/// A SET bit says the producer observed that family this frame. The family may
/// still be legitimately EMPTY — a country with no advisors carries no advisor
/// rows — and that emptiness is an observation.
///
/// A CLEAR bit says the producer did NOT observe that family: its rows are
/// unknown, not absent. This is the whole distinction the mask exists to carry.
/// A consumer must retain whatever it already knew about a clear family and
/// must never read the missing rows as deletions.
///
/// The mask is a WHOLE-FAMILY vocabulary. A family declared present whose rows
/// were partially lost is a defect, not a gap, and must carry an untolerated
/// status bit; clearing an unrelated family's bit does not launder it.
pub const FAMILY_CHARACTER: u32 = 1 << (ROW_CHARACTER - 1);
pub const FAMILY_UNIT_LEADER: u32 = 1 << (ROW_UNIT_LEADER - 1);
pub const FAMILY_UNIT_LEADER_TRAIT: u32 = 1 << (ROW_UNIT_LEADER_TRAIT - 1);
pub const FAMILY_COUNTRY_LEADER_ROLE: u32 = 1 << (ROW_COUNTRY_LEADER_ROLE - 1);
pub const FAMILY_ACTIVE_RULER: u32 = 1 << (ROW_ACTIVE_RULER - 1);
pub const FAMILY_ADVISOR_ROSTER: u32 = 1 << (ROW_ADVISOR_ROSTER - 1);
pub const FAMILY_APPOINTED_ADVISOR: u32 = 1 << (ROW_APPOINTED_ADVISOR - 1);
pub const FAMILY_CHARACTER_PORTRAIT: u32 = 1 << (ROW_CHARACTER_PORTRAIT - 1);

/// All eight families observed. This is what an undeclared payload decodes to,
/// so legacy streams keep their exact meaning.
pub const FAMILY_ALL: u32 = FAMILY_CHARACTER
    | FAMILY_UNIT_LEADER
    | FAMILY_UNIT_LEADER_TRAIT
    | FAMILY_COUNTRY_LEADER_ROLE
    | FAMILY_ACTIVE_RULER
    | FAMILY_ADVISOR_ROSTER
    | FAMILY_APPOINTED_ADVISOR
    | FAMILY_CHARACTER_PORTRAIT;

/// The families whose roots come from the shared background heap sweep rather
/// than from the exact engine-owned character-manager census.
///
/// The sweep needs seconds to complete its first pass over process memory, and
/// until it has, these four families are unknown while the census families are
/// exactly known. That asymmetry — not any property of the data — is why the
/// mask exists.
///
/// Appointed advisors are in the set because they are an intersection of the
/// advisor roster with each country's active ideas: no roster, no appointment
/// claim. Active rulers are NOT, because they are read from the same-frame
/// country politics roots, which the sweep has nothing to do with.
pub const FAMILY_HEAP_SWEEP_DERIVED: u32 = FAMILY_UNIT_LEADER_TRAIT
    | FAMILY_COUNTRY_LEADER_ROLE
    | FAMILY_ADVISOR_ROSTER
    | FAMILY_APPOINTED_ADVISOR;

/// What a prewarm that censused the character manager but found the heap sweep
/// cold can honestly claim to have observed.
///
/// HISTORICAL SINCE LANE FU (2026-08-08): the collector no longer produces this
/// mask. Three of the four families above are discovered by the character-down
/// container walk now and are observed whenever the census is, so a cold sweep
/// costs [`FAMILY_UNIT_LEADER_TRAIT`] alone — see
/// [`FAMILY_WALK_WITHOUT_TRAITS`]. The constant stays because it names a shape
/// the fold and replay still have to decode: every capture up to t43 emitted it.
pub const FAMILY_CENSUS_ONLY: u32 = FAMILY_ALL & !FAMILY_HEAP_SWEEP_DERIVED;

/// The families that still depend on the shared heap sweep (lane FU).
///
/// One, and it is not a role: `CUnitLeaderTrait` is a definition DATABASE
/// loaded from installed content. It hangs off no character — `CUnitLeader+0x2A8`
/// holds a vector of pointers INTO it — so a character-down walk cannot reach
/// the definitions, only the subset some leader happens to reference, and the
/// adapter needs the whole table to resolve any of them. The owning container
/// has not been found; until it is, this is what a cold sweep costs.
pub const FAMILY_TRAIT_SWEEP_DERIVED: u32 = FAMILY_UNIT_LEADER_TRAIT;

/// What a prewarm that censused the character manager and walked its role
/// containers observed while the heap sweep was still cold.
///
/// This is the mask the collector emits in the cold window since lane FU, and
/// the ~12.2 s of every capture that used to publish [`FAMILY_CENSUS_ONLY`]
/// now publish this instead: seven families of eight, missing only the trait
/// definitions.
pub const FAMILY_WALK_WITHOUT_TRAITS: u32 = FAMILY_ALL & !FAMILY_TRAIT_SWEEP_DERIVED;

/// The wire name of each family bit, ascending. Used by the fold and by replay
/// to spell a gap in the snapshot rather than as a number nobody can read.
pub const FAMILY_NAMES: &[(u32, &str)] = &[
    (FAMILY_CHARACTER, "characters"),
    (FAMILY_UNIT_LEADER, "unit_leaders"),
    (FAMILY_UNIT_LEADER_TRAIT, "unit_leader_traits"),
    (FAMILY_COUNTRY_LEADER_ROLE, "country_leader_roles"),
    (FAMILY_ACTIVE_RULER, "active_rulers"),
    (FAMILY_ADVISOR_ROSTER, "advisor_roster"),
    (FAMILY_APPOINTED_ADVISOR, "appointed_advisors"),
    (FAMILY_CHARACTER_PORTRAIT, "character_portraits"),
];

/// The families this payload did NOT observe, by wire name, ascending.
pub fn absent_family_names(family_presence_mask: u32) -> Vec<&'static str> {
    FAMILY_NAMES
        .iter()
        .filter(|(bit, _)| (family_presence_mask & bit) == 0)
        .map(|(_, name)| *name)
        .collect()
}

pub const UNIT_LEADER_ARMY: u8 = 1;
pub const UNIT_LEADER_NAVY: u8 = 2;

pub const ROW_FLAG_AMBIGUOUS: u16 = 1;
pub const ROW_FLAG_SKILL_PRESENT: u16 = 1 << 1;
pub const ROW_FLAG_EXPERIENCE_PRESENT: u16 = 1 << 2;
/// SET on a [`ROW_CHARACTER_PORTRAIT`] row whose `gfx` identity was NOT valid
/// UTF-8 in the engine's memory and was decoded through the producer's
/// Windows-1252 fallback.
///
/// # Why the provenance rides the row
///
/// The string in the table is the DECODED text either way, and the two
/// decodings are indistinguishable from it: `caf\u{e9}` is `caf\u{e9}` whether
/// the engine held `caf\xc3\xa9` or `caf\xe9`. A consumer that has to know
/// which it was — anyone matching an identity against a mod's own files, or
/// auditing how much of a world needed the fallback — cannot recover it from
/// the payload, so the producer states it.
///
/// CLEAR means the identity's bytes WERE valid UTF-8, and that reading is
/// sound for every stream ever produced: before the fallback existed the
/// producer's strict reader admitted only bytes in `32..127`, so no payload has
/// ever carried a non-UTF-8 identity without this flag.
pub const ROW_FLAG_PORTRAIT_GFX_WINDOWS1252: u16 = 1 << 3;

/// Every flag bit a [`ROW_CHARACTER_PORTRAIT`] row may carry.
///
/// A bit outside this mask has no defined meaning, so a portrait row carrying
/// one is REFUSED rather than interpreted — the same rule
/// [`SCHEMA_ALL_ASSIGNED`] applies to the header's schema word. Without it a
/// future producer's flag would be silently dropped by today's decoder and the
/// row would read as ordinary, which is exactly the silent guess the provenance
/// flag exists to end.
///
/// SCOPED TO THE PORTRAIT ROW on purpose: it is the row family this rule was
/// written for, and extending the same admission to the other seven kinds is a
/// change to their contracts, not a side effect of this one.
pub const ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT: u16 = ROW_FLAG_PORTRAIT_GFX_WINDOWS1252;

/// WHICH DECODING produced a portrait `gfx` identity, as carried by
/// [`ROW_FLAG_PORTRAIT_GFX_WINDOWS1252`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortraitIdentityEncoding {
    /// The engine's bytes were valid UTF-8 (which includes every pure-ASCII
    /// identity). The default, and what every identity published before the
    /// fallback existed was.
    #[default]
    Utf8,
    /// The engine's bytes were NOT valid UTF-8 and were decoded as
    /// Windows-1252 by the producer.
    Windows1252,
}

impl PortraitIdentityEncoding {
    /// The wire spelling, for the snapshot sidecar and any other text surface.
    /// Stable: a consumer may match on it.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Utf8 => "utf8",
            Self::Windows1252 => "windows1252",
        }
    }

    /// The inverse of [`Self::wire_name`]. `None` for a spelling nobody
    /// assigned, so a consumer refuses it rather than defaulting it to `Utf8`
    /// and publishing a provenance claim the producer never made.
    pub fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "utf8" => Some(Self::Utf8),
            "windows1252" => Some(Self::Windows1252),
            _ => None,
        }
    }
}

const CHARACTER_ROW_LEN: u8 = 20;
const UNIT_LEADER_ROW_LEN: u8 = 42;
const TRAIT_ROW_LEN: u8 = 16;
const COUNTRY_LEADER_ROLE_ROW_LEN: u8 = 20;
const ACTIVE_RULER_ROW_LEN: u8 = 28;
const ADVISOR_ROW_LEN: u8 = 24;
const CHARACTER_PORTRAIT_ROW_LEN: u8 = 24;
/// The widest row `encode_payload` can emit, used ONLY to size the row buffer
/// up front. Asserted against every row-length constant by
/// `max_row_len_covers_every_row_kind`, so adding a wider row kind without
/// updating this is a test failure rather than a silent re-introduction of the
/// reallocation it exists to avoid.
const MAX_ROW_LEN: u8 = UNIT_LEADER_ROW_LEN;
const ADVISOR_CABINET_BUCKETS: &[&[u8]] = &[
    b"political_advisor",
    b"army_chief",
    b"navy_chief",
    b"air_chief",
    b"high_command",
    b"theorist",
    b"unknown",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub name: Option<&'a str>,
    pub token: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderRow<'a> {
    pub country_tag_db_id: u32,
    pub unit_leader_id: u32,
    pub char_id: u32,
    pub leader_kind: u8,
    pub skill: Option<u32>,
    pub experience_raw: Option<i32>,
    pub attack: Option<u8>,
    pub defense: Option<u8>,
    pub planning: Option<u8>,
    pub logistics: Option<u8>,
    pub maneuvering: Option<u8>,
    pub coordination: Option<u8>,
    pub token: Option<&'a str>,
    pub name: Option<&'a str>,
    pub trait_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderTraitRow<'a> {
    pub unit_leader_id: u32,
    pub ordinal: u16,
    pub trait_token: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeaderRoleRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub name: Option<&'a str>,
    pub token: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRulerRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub ruling_party: Option<&'a str>,
    pub name: Option<&'a str>,
    pub token: Option<&'a str>,
    pub leader_vec_size: u16,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvisorRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub slot: Option<&'a str>,
    pub ledger: Option<&'a str>,
    pub idea_token: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPortraitRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub role: &'a str,
    pub size: &'a str,
    pub gfx: &'a str,
    /// Which decoding produced `gfx`. Encoded as
    /// [`ROW_FLAG_PORTRAIT_GFX_WINDOWS1252`] in the row header's flag word, so
    /// it costs no row bytes.
    pub gfx_encoding: PortraitIdentityEncoding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharacterLaneRow<'a> {
    Character(CharacterRow<'a>),
    UnitLeader(UnitLeaderRow<'a>),
    UnitLeaderTrait(UnitLeaderTraitRow<'a>),
    CountryLeaderRole(CountryLeaderRoleRow<'a>),
    ActiveRuler(ActiveRulerRow<'a>),
    AdvisorRoster(AdvisorRow<'a>),
    AppointedAdvisor(AdvisorRow<'a>),
    CharacterPortrait(CharacterPortraitRow<'a>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain44GateSummary {
    pub status_flags_or: u32,
    pub schema_flags_or: u32,
    /// Which row families this payload observed. Zero only when the header was
    /// too short to decode at all; every decoded payload — declared or not —
    /// leaves a real mask here, and an undeclared one leaves [`FAMILY_ALL`].
    pub family_presence_mask: u32,
    pub row_count_total: u64,
    pub character_rows: u64,
    pub unit_leader_rows: u64,
    pub trait_rows: u64,
    pub country_leader_role_rows: u64,
    pub active_ruler_rows: u64,
    pub advisor_roster_rows: u64,
    pub appointed_advisor_rows: u64,
    pub character_portrait_rows: u64,
    /// The subset of `character_portrait_rows` whose `gfx` identity was decoded
    /// through the producer's Windows-1252 fallback
    /// ([`ROW_FLAG_PORTRAIT_GFX_WINDOWS1252`]). Zero on every payload written
    /// before the fallback existed, and on every world whose portrait
    /// identities are UTF-8.
    pub character_portrait_windows1252_rows: u64,
    pub string_count: u64,
    pub string_bytes: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDomain47Payload {
    pub status_flags: u32,
    pub schema_flags: u32,
    /// Which row families this payload observed; see [`FAMILY_CHARACTER`].
    /// [`FAMILY_ALL`] for every payload that carries no declaration, so a fold
    /// written against this field needs no legacy branch.
    pub family_presence_mask: u32,
    pub rows: Vec<DecodedDomain47Row>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodedDomain47Row {
    Character {
        country_tag_db_id: u32,
        char_id: u32,
        name: Option<String>,
        token: Option<String>,
    },
    UnitLeader {
        country_tag_db_id: u32,
        unit_leader_id: u32,
        char_id: u32,
        leader_kind: u8,
        skill: Option<u32>,
        experience_raw: Option<i32>,
        attack: Option<u8>,
        defense: Option<u8>,
        planning: Option<u8>,
        logistics: Option<u8>,
        maneuvering: Option<u8>,
        coordination: Option<u8>,
        token: Option<String>,
        name: Option<String>,
    },
    UnitLeaderTrait {
        unit_leader_id: u32,
        ordinal: u16,
        trait_token: String,
    },
    CountryLeaderRole {
        country_tag_db_id: u32,
        char_id: u32,
        name: Option<String>,
        token: Option<String>,
    },
    ActiveRuler {
        country_tag_db_id: u32,
        char_id: u32,
        ruling_party: Option<String>,
        name: Option<String>,
        token: Option<String>,
        leader_vec_size: u16,
        ambiguous: bool,
    },
    AdvisorRoster {
        country_tag_db_id: u32,
        char_id: u32,
        slot: Option<String>,
        ledger: Option<String>,
        idea_token: String,
    },
    AppointedAdvisor {
        country_tag_db_id: u32,
        char_id: u32,
        slot: Option<String>,
        ledger: Option<String>,
        idea_token: String,
    },
    CharacterPortrait {
        country_tag_db_id: u32,
        char_id: u32,
        role: String,
        size: String,
        gfx: String,
        /// Which decoding produced `gfx`, from
        /// [`ROW_FLAG_PORTRAIT_GFX_WINDOWS1252`]. A payload written before the
        /// flag existed decodes to [`PortraitIdentityEncoding::Utf8`], which is
        /// what its producer's strict reader guaranteed.
        gfx_encoding: PortraitIdentityEncoding,
    },
}

/// Interning side table for the payload's string block.
///
/// `values` IS the encoded block: `encode_into` walks it in order and the ids
/// handed back are indices into it, so its contents and its order are wire
/// state. `by_value` is a pure lookup accelerator over the same strings and
/// is never iterated — nothing about the emitted bytes can depend on how it
/// is keyed or ordered.
///
/// WHY IT IS SHAPED LIKE THIS (lane DB, 2026-08-06). Domain 47 emits ~28k
/// rows and ~25k distinct strings per frame, and `intern` is called on nearly
/// every row field, so this is the encoder's whole hot loop. The offline cost
/// bench (`d47_cost_read_bracket_observation_vs_compute`) measured
/// `encode_frame_payload` at 6.87 ms against 1.85 ms for the engine walk it
/// serialises — the encoder cost 3.7x as much as reading the game. Three
/// things in the previous shape caused that, and each is fixed here WITHOUT
/// changing a single emitted byte:
///
///  * The map was a `BTreeMap<String, _>`, so every lookup was O(log n) FULL
///    STRING COMPARISONS — ~15 memcmps against a 25k-entry table, ~28k times
///    a frame. A hash map answers the same question in one.
///  * Every distinct string was allocated TWICE, once for `values` and once
///    for the map key. `Rc<str>` stores it once and the map holds a second
///    handle to the same bytes.
///  * `is_domain47_string_value` — which DECODES the whole string as UTF-8
///    chars — ran ahead of the lookup, so it re-validated every repeat
///    occurrence of an already-interned string.
///
/// The validation move is the only one that changes WHEN work happens rather
/// than how fast it is, so it is worth stating why it is not a relaxation: the
/// checks are pure functions of the string's bytes, and an id is only ever
/// returned for a value that passed them the first time it was seen. A value
/// that would fail still fails, on its first occurrence, with the same panic —
/// there is no input for which the old shape rejects and this one accepts.
#[derive(Default)]
struct StringTable {
    values: Vec<Rc<str>>,
    by_value: HashMap<Rc<str>, u32>,
}

impl StringTable {
    fn intern(&mut self, value: Option<&str>) -> u32 {
        let Some(value) = value else {
            return STRING_NONE;
        };
        // Already interned => already validated, by construction: the only
        // path that inserts an id runs both asserts below first.
        if let Some(id) = self.by_value.get(value) {
            return *id;
        }
        assert!(
            value.len() <= MAX_STRING_BYTES,
            "character lane string exceeds max length"
        );
        assert!(
            is_domain47_string_value(value),
            "character lane string must be valid non-control UTF-8"
        );
        let id = self.values.len().min(u32::MAX as usize) as u32;
        // ONE allocation, two handles: `values` owns the encode order and the
        // map owns the lookup, over the same bytes.
        let shared: Rc<str> = Rc::from(value);
        self.values.push(Rc::clone(&shared));
        self.by_value.insert(shared, id);
        id
    }

    fn encoded_len(&self) -> usize {
        self.values.iter().map(|value| 2 + value.len()).sum()
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        for value in &self.values {
            out.extend_from_slice(&(value.len() as u16).to_le_bytes());
            out.extend_from_slice(value.as_bytes());
        }
    }
}

fn is_domain47_string_value(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

#[derive(Default)]
struct RowCounts {
    total: u32,
    characters: u32,
    unit_leaders: u32,
    traits: u32,
    country_leader_roles: u32,
    active_rulers: u32,
    advisor_roster: u32,
    appointed_advisors: u32,
    character_portraits: u32,
}

/// Encode a payload that observed every family.
///
/// This is the shape d47 has always emitted, byte for byte: the declaration
/// flag stays clear and offset 60 stays zero. Every fixture, gate and
/// verification helper that predates per-family admission still calls this.
pub fn encode_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    rows: &[CharacterLaneRow<'_>],
) -> Vec<u8> {
    encode_payload_with_families(elapsed_wall_ms, status_flags, schema_flags, FAMILY_ALL, rows)
}

/// The one place a d47 payload is built, and therefore the one place the status
/// word and the family declaration are derived from the same fact.
///
/// # Why the status is decided here and not by the caller
///
/// The tolerance table admits a d47 record whose only failure-class bit is
/// `PARTIAL_DATA`. If a caller could set that bit without declaring a family
/// gap — or declare a gap without setting it — a genuine defect would be
/// admitted as a reviewed partial, which is the one failure mode the shared
/// degraded vocabulary says is worse than refusing outright. So the bit is a
/// FUNCTION of `family_presence_mask` here, and [`decode_payload`] checks the
/// implication in both directions. A new emit site cannot get this wrong,
/// because there is nothing for it to get wrong.
///
/// Callers keep their own failure bits: a declared partial that ALSO lost rows
/// inside a family it claims to have observed still carries whatever bit says
/// so, which puts it back outside the tolerance where it belongs.
pub fn encode_payload_with_families(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    family_presence_mask: u32,
    rows: &[CharacterLaneRow<'_>],
) -> Vec<u8> {
    let declared = family_presence_mask != FAMILY_ALL;
    let schema_flags = if declared {
        schema_flags | SCHEMA_FAMILY_PRESENCE_DECLARED
    } else {
        schema_flags & !SCHEMA_FAMILY_PRESENCE_DECLARED
    };
    let status_flags = if declared {
        status_flags | STATUS_PARTIAL_DATA
    } else {
        status_flags
    };
    let reserved_or_families = if declared { family_presence_mask } else { 0 };
    encode_payload_inner(
        elapsed_wall_ms,
        status_flags,
        schema_flags,
        reserved_or_families,
        rows,
    )
}

fn encode_payload_inner(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    reserved_or_families: u32,
    rows: &[CharacterLaneRow<'_>],
) -> Vec<u8> {
    let mut strings = StringTable::default();
    // ONE allocation for the row block instead of ~20 doubling reallocations,
    // each of which memcpy'd everything written so far. `MAX_ROW_LEN` is the
    // widest row this encoder can emit, so the reservation is an upper bound
    // and the buffer is never grown again; it is transient and dropped at the
    // end of this call. Purely an allocation decision — a capacity hint cannot
    // change a byte of what is written into it.
    let mut row_bytes = Vec::with_capacity(rows.len().saturating_mul(MAX_ROW_LEN as usize));
    let mut counts = RowCounts::default();

    for row in rows {
        counts.total = counts.total.saturating_add(1);
        match row {
            CharacterLaneRow::Character(row) => {
                counts.characters = counts.characters.saturating_add(1);
                push_header(&mut row_bytes, ROW_CHARACTER, CHARACTER_ROW_LEN, 0);
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(row.name));
                push_u32(&mut row_bytes, strings.intern(row.token));
            }
            CharacterLaneRow::UnitLeader(row) => {
                counts.unit_leaders = counts.unit_leaders.saturating_add(1);
                let mut flags = 0u16;
                if row.skill.is_some() {
                    flags |= ROW_FLAG_SKILL_PRESENT;
                }
                if row.experience_raw.is_some() {
                    flags |= ROW_FLAG_EXPERIENCE_PRESENT;
                }
                push_header(&mut row_bytes, ROW_UNIT_LEADER, UNIT_LEADER_ROW_LEN, flags);
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.unit_leader_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, row.skill.unwrap_or(SKILL_NONE));
                push_i32(
                    &mut row_bytes,
                    row.experience_raw.unwrap_or(EXPERIENCE_RAW_NONE),
                );
                push_u32(&mut row_bytes, strings.intern(row.token));
                push_u32(&mut row_bytes, strings.intern(row.name));
                row_bytes.extend_from_slice(&row.trait_count.to_le_bytes());
                row_bytes.push(row.attack.unwrap_or(BYTE_NONE));
                row_bytes.push(row.defense.unwrap_or(BYTE_NONE));
                row_bytes.push(row.planning.unwrap_or(BYTE_NONE));
                row_bytes.push(row.logistics.unwrap_or(BYTE_NONE));
                row_bytes.push(row.maneuvering.unwrap_or(BYTE_NONE));
                row_bytes.push(row.coordination.unwrap_or(BYTE_NONE));
                row_bytes.push(row.leader_kind);
                row_bytes.push(0);
            }
            CharacterLaneRow::UnitLeaderTrait(row) => {
                counts.traits = counts.traits.saturating_add(1);
                push_header(&mut row_bytes, ROW_UNIT_LEADER_TRAIT, TRAIT_ROW_LEN, 0);
                push_u32(&mut row_bytes, row.unit_leader_id);
                row_bytes.extend_from_slice(&row.ordinal.to_le_bytes());
                row_bytes.extend_from_slice(&0u16.to_le_bytes());
                push_u32(&mut row_bytes, strings.intern(Some(row.trait_token)));
            }
            CharacterLaneRow::CountryLeaderRole(row) => {
                counts.country_leader_roles = counts.country_leader_roles.saturating_add(1);
                push_header(
                    &mut row_bytes,
                    ROW_COUNTRY_LEADER_ROLE,
                    COUNTRY_LEADER_ROLE_ROW_LEN,
                    0,
                );
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(row.name));
                push_u32(&mut row_bytes, strings.intern(row.token));
            }
            CharacterLaneRow::ActiveRuler(row) => {
                counts.active_rulers = counts.active_rulers.saturating_add(1);
                let flags = if row.ambiguous { ROW_FLAG_AMBIGUOUS } else { 0 };
                push_header(
                    &mut row_bytes,
                    ROW_ACTIVE_RULER,
                    ACTIVE_RULER_ROW_LEN,
                    flags,
                );
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(row.ruling_party));
                push_u32(&mut row_bytes, strings.intern(row.name));
                push_u32(&mut row_bytes, strings.intern(row.token));
                row_bytes.extend_from_slice(&row.leader_vec_size.to_le_bytes());
                row_bytes.extend_from_slice(&0u16.to_le_bytes());
            }
            CharacterLaneRow::AdvisorRoster(row) => {
                counts.advisor_roster = counts.advisor_roster.saturating_add(1);
                encode_advisor_row(&mut row_bytes, &mut strings, ROW_ADVISOR_ROSTER, row);
            }
            CharacterLaneRow::AppointedAdvisor(row) => {
                counts.appointed_advisors = counts.appointed_advisors.saturating_add(1);
                encode_advisor_row(&mut row_bytes, &mut strings, ROW_APPOINTED_ADVISOR, row);
            }
            CharacterLaneRow::CharacterPortrait(row) => {
                counts.character_portraits = counts.character_portraits.saturating_add(1);
                let flags = match row.gfx_encoding {
                    PortraitIdentityEncoding::Utf8 => 0,
                    PortraitIdentityEncoding::Windows1252 => ROW_FLAG_PORTRAIT_GFX_WINDOWS1252,
                };
                push_header(
                    &mut row_bytes,
                    ROW_CHARACTER_PORTRAIT,
                    CHARACTER_PORTRAIT_ROW_LEN,
                    flags,
                );
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(Some(row.role)));
                push_u32(&mut row_bytes, strings.intern(Some(row.size)));
                push_u32(&mut row_bytes, strings.intern(Some(row.gfx)));
            }
        }
    }

    let mut out = Vec::with_capacity(HEADER_LEN + strings.encoded_len() + row_bytes.len());
    push_u64(&mut out, elapsed_wall_ms);
    push_u32(&mut out, status_flags);
    push_u32(&mut out, counts.total);
    push_u32(&mut out, strings.values.len().min(u32::MAX as usize) as u32);
    push_u32(
        &mut out,
        strings.encoded_len().min(u32::MAX as usize) as u32,
    );
    push_u32(&mut out, counts.characters);
    push_u32(&mut out, counts.unit_leaders);
    push_u32(&mut out, counts.country_leader_roles);
    push_u32(&mut out, counts.active_rulers);
    push_u32(&mut out, counts.advisor_roster);
    push_u32(&mut out, counts.appointed_advisors);
    push_u32(&mut out, counts.traits);
    push_u32(&mut out, row_bytes.len().min(u32::MAX as usize) as u32);
    push_u32(&mut out, schema_flags);
    push_u32(&mut out, reserved_or_families);
    strings.encode_into(&mut out);
    out.extend_from_slice(&row_bytes);
    out
}

pub fn encode_compact_data_record(sequence: u64, game_hour_raw: i32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&DOMAIN_ID.to_le_bytes());
    out.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Read the header's family declaration and refuse every spelling of it that
/// would let a consumer read a gap as a fact.
///
/// Returns the presence mask the rest of the decode should believe. On any
/// refusal it returns [`FAMILY_ALL`] — the conservative value, because it makes
/// the row-side checks below demand a complete payload rather than excusing
/// missing rows on the strength of a declaration that was just rejected.
///
/// # The status coupling
///
/// Both directions are enforced, and they are what binds the shared tolerance
/// table to this payload's own words:
///
///  * a declared payload MUST carry `PARTIAL_DATA`, so it can never certify as
///    a complete hour while admitting it did not observe a family;
///  * a payload whose only failure-class bit is `PARTIAL_DATA` MUST be
///    declared, so the tolerance entry can never absorb an undeclared defect.
///
/// The second is the load-bearing one. `COMPACT_EXPECTED_PARTIAL_TOLERANCES`
/// classifies from the status word alone; without this rule any d47 reader that
/// set `PARTIAL_DATA` for its own reasons would silently inherit a tolerance
/// written for a completely different situation.
fn decode_family_presence_mask(
    offset: usize,
    schema_flags: u32,
    status_flags: u32,
    reserved_or_families: u32,
    parse_errors: &mut Vec<String>,
) -> u32 {
    let unassigned_schema = schema_flags & !SCHEMA_ALL_ASSIGNED;
    if unassigned_schema != 0 {
        parse_errors.push(format!(
            "offset {offset}: character lane schema_flags carry unassigned bits {unassigned_schema:#010x}"
        ));
        return FAMILY_ALL;
    }
    let declared = (schema_flags & SCHEMA_FAMILY_PRESENCE_DECLARED) != 0;
    // The failure-class bits this payload spells for itself. Kept local rather
    // than imported so this crate's wire grammar does not take a dependency on
    // the shared contract crate purely to name three constants it already has.
    let failure_flags = status_flags
        & (STATUS_WORKER_PANIC
            | STATUS_BRIDGE_UNAVAILABLE
            | STATUS_SOURCE_UNAVAILABLE
            | STATUS_PARTIAL_DATA);
    if !declared {
        if reserved_or_families != 0 {
            parse_errors.push(format!(
                "offset {offset}: character lane reserved header field is nonzero"
            ));
        }
        if failure_flags == STATUS_PARTIAL_DATA {
            parse_errors.push(format!(
                "offset {offset}: character lane declares partial data with no family-presence \
                 declaration, so nothing names what it failed to observe"
            ));
        }
        return FAMILY_ALL;
    }
    if (status_flags & STATUS_PARTIAL_DATA) == 0 {
        parse_errors.push(format!(
            "offset {offset}: character lane declares a family-presence mask without partial data \
             status, so an incomplete hour would certify complete"
        ));
    }
    let unassigned_families = reserved_or_families & !FAMILY_ALL;
    if unassigned_families != 0 {
        parse_errors.push(format!(
            "offset {offset}: character lane family-presence mask carries unassigned bits \
             {unassigned_families:#010x}"
        ));
        return FAMILY_ALL;
    }
    if reserved_or_families == FAMILY_ALL {
        // Two spellings of "complete" would make the byte-identity of an
        // ordinary payload unprovable: the same snapshot could arrive declared
        // or undeclared and nothing would say which was intended.
        parse_errors.push(format!(
            "offset {offset}: character lane declares every family present; a complete snapshot is \
             spelled by omitting the declaration"
        ));
        return FAMILY_ALL;
    }
    if reserved_or_families == 0 {
        parse_errors.push(format!(
            "offset {offset}: character lane family-presence mask observes no family at all"
        ));
        return FAMILY_ALL;
    }
    if (reserved_or_families & FAMILY_CHARACTER) == 0 {
        // Every other family's rows carry a (country, char_id) that must join
        // this payload's own character census; without it nothing in the
        // payload can be validated at all.
        parse_errors.push(format!(
            "offset {offset}: character lane family-presence mask omits the character census that \
             every other family joins against"
        ));
        return FAMILY_ALL;
    }
    if (reserved_or_families & FAMILY_UNIT_LEADER_TRAIT) != 0
        && (reserved_or_families & FAMILY_UNIT_LEADER) == 0
    {
        parse_errors.push(format!(
            "offset {offset}: character lane observes unit-leader traits without the unit leaders \
             they attach to"
        ));
        return FAMILY_ALL;
    }
    if (reserved_or_families & FAMILY_APPOINTED_ADVISOR) != 0
        && (reserved_or_families & FAMILY_ADVISOR_ROSTER) == 0
    {
        // An appointed advisor is a member of the roster it was appointed from;
        // consumers validate exactly that subset relation.
        parse_errors.push(format!(
            "offset {offset}: character lane observes appointed advisors without the advisor roster \
             they are a subset of"
        ));
        return FAMILY_ALL;
    }
    if (reserved_or_families & FAMILY_APPOINTED_ADVISOR) != 0
        && (schema_flags & SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS) == 0
    {
        // Checked on the FAMILY and not on the rows: an appointed family
        // observed to be empty is still an appointment claim, and it needs the
        // same identity-intersection authority a non-empty one does.
        parse_errors.push(format!(
            "offset {offset}: character lane observes appointed advisors without the active-idea \
             identity-intersection authority"
        ));
        return FAMILY_ALL;
    }
    if (reserved_or_families & FAMILY_CHARACTER_PORTRAIT) != 0
        && (schema_flags & SCHEMA_STATIC_NAME_CLOSURE) == 0
    {
        parse_errors.push(format!(
            "offset {offset}: character lane observes character portraits without static \
             name-closure authority"
        ));
        return FAMILY_ALL;
    }
    reserved_or_families
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain44GateSummary {
    let mut summary = Domain44GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: character lane payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }
    let status_flags = read_u32(payload, 8);
    let expected_total = read_u32(payload, 12) as u64;
    let string_count = read_u32(payload, 16) as usize;
    let string_bytes = read_u32(payload, 20) as usize;
    let expected_characters = read_u32(payload, 24) as u64;
    let expected_unit_leaders = read_u32(payload, 28) as u64;
    let expected_country_leader_roles = read_u32(payload, 32) as u64;
    let expected_active_rulers = read_u32(payload, 36) as u64;
    let expected_advisor_roster = read_u32(payload, 40) as u64;
    let expected_appointed_advisors = read_u32(payload, 44) as u64;
    let expected_traits = read_u32(payload, 48) as u64;
    let row_bytes = read_u32(payload, 52) as usize;
    let schema_flags = read_u32(payload, 56);
    let reserved_or_families = read_u32(payload, 60);
    summary.status_flags_or |= status_flags;
    summary.schema_flags_or |= schema_flags;

    let family_presence_mask = decode_family_presence_mask(
        offset,
        schema_flags,
        status_flags,
        reserved_or_families,
        &mut summary.parse_errors,
    );
    summary.family_presence_mask = family_presence_mask;

    let mut cursor = HEADER_LEN;
    let string_start = cursor;
    let mut strings = Vec::with_capacity(string_count);
    for index in 0..string_count {
        if cursor + 2 > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} has truncated length"
            ));
            return summary;
        }
        let len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;
        if len > MAX_STRING_BYTES {
            summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} length {len} exceeds {MAX_STRING_BYTES}"
            ));
        }
        if cursor + len > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} is truncated"
            ));
            return summary;
        }
        let raw = &payload[cursor..cursor + len];
        match std::str::from_utf8(raw) {
            Ok(value) if is_domain47_string_value(value) => {}
            Ok(_) => summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} contains control characters"
            )),
            Err(_) => summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} is invalid UTF-8"
            )),
        }
        strings.push(raw);
        cursor += len;
    }
    let measured_string_bytes = cursor.saturating_sub(string_start);
    summary.string_count = string_count as u64;
    summary.string_bytes = measured_string_bytes as u64;
    if measured_string_bytes != string_bytes {
        summary.parse_errors.push(format!(
            "offset {offset}: string_bytes header {string_bytes} != decoded {measured_string_bytes}"
        ));
    }
    if cursor + row_bytes != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: row_bytes header {row_bytes} does not match remaining {}",
            payload.len().saturating_sub(cursor)
        ));
        if cursor + row_bytes > payload.len() {
            return summary;
        }
    }

    let rows_end = cursor.saturating_add(row_bytes).min(payload.len());
    let mut row_index = 0usize;
    let mut seen_characters = BTreeSet::new();
    let mut seen_unit_leaders = BTreeSet::new();
    let mut seen_country_roles = BTreeSet::new();
    let mut seen_active_rulers = BTreeSet::new();
    let mut seen_advisor_rows = BTreeSet::new();
    let mut seen_portrait_rows = BTreeSet::new();
    let mut declared_trait_counts = BTreeMap::<u32, u16>::new();
    let mut observed_trait_counts = BTreeMap::<u32, u16>::new();

    while cursor < rows_end {
        if cursor + 4 > rows_end {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has truncated row header"
            ));
            break;
        }
        let row_offset = cursor;
        let kind = payload[cursor];
        let row_len = payload[cursor + 1] as usize;
        let flags = read_u16(payload, cursor + 2);
        if row_len < 4 || cursor + row_len > rows_end {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} kind {kind} has invalid length {row_len}"
            ));
            break;
        }
        summary.row_count_total = summary.row_count_total.saturating_add(1);
        match kind {
            ROW_CHARACTER => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    CHARACTER_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let name_sid = read_u32(payload, cursor + 12);
                let token_sid = read_u32(payload, cursor + 16);
                summary.character_rows = summary.character_rows.saturating_add(1);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                if !seen_characters.insert(char_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates character id {char_id}"
                    ));
                }
            }
            ROW_UNIT_LEADER => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    UNIT_LEADER_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let unit_id = read_u32(payload, cursor + 8);
                let char_id = read_u32(payload, cursor + 12);
                let skill = read_u32(payload, cursor + 16);
                let experience_raw = read_i32(payload, cursor + 20);
                let token_sid = read_u32(payload, cursor + 24);
                let name_sid = read_u32(payload, cursor + 28);
                let trait_count = read_u16(payload, cursor + 32);
                let leader_kind = payload[cursor + 40];
                summary.unit_leader_rows = summary.unit_leader_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_unit_leader(&mut summary, offset, row_index, unit_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                if skill != SKILL_NONE && skill > MAX_SKILL {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} has invalid unit-leader skill {skill}"
                    ));
                }
                if (flags & ROW_FLAG_SKILL_PRESENT != 0) != (skill != SKILL_NONE) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} skill-present flag disagrees with value"
                    ));
                }
                if (flags & ROW_FLAG_EXPERIENCE_PRESENT != 0)
                    != (experience_raw != EXPERIENCE_RAW_NONE)
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} experience-present flag disagrees with value"
                    ));
                }
                if trait_count > MAX_TRAITS_PER_LEADER {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} declares {trait_count} traits"
                    ));
                }
                if leader_kind != UNIT_LEADER_ARMY && leader_kind != UNIT_LEADER_NAVY {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} has invalid leader kind {leader_kind}"
                    ));
                }
                if !seen_unit_leaders.insert(unit_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates unit leader id {unit_id}"
                    ));
                }
                declared_trait_counts.insert(unit_id, trait_count);
            }
            ROW_UNIT_LEADER_TRAIT => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    TRAIT_ROW_LEN,
                );
                let unit_id = read_u32(payload, cursor + 4);
                let ordinal = read_u16(payload, cursor + 8);
                let trait_sid = read_u32(payload, cursor + 12);
                summary.trait_rows = summary.trait_rows.saturating_add(1);
                validate_unit_leader(&mut summary, offset, row_index, unit_id);
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "trait_token",
                    trait_sid,
                    strings.len(),
                );
                if ordinal >= MAX_TRAITS_PER_LEADER {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} has invalid trait ordinal {ordinal}"
                    ));
                }
                let entry = observed_trait_counts.entry(unit_id).or_default();
                *entry = entry.saturating_add(1);
            }
            ROW_COUNTRY_LEADER_ROLE => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    COUNTRY_LEADER_ROLE_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let name_sid = read_u32(payload, cursor + 12);
                let token_sid = read_u32(payload, cursor + 16);
                summary.country_leader_role_rows =
                    summary.country_leader_role_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                if !seen_country_roles.insert(char_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates country-leader role char id {char_id}"
                    ));
                }
            }
            ROW_ACTIVE_RULER => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    ACTIVE_RULER_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let party_sid = read_u32(payload, cursor + 12);
                let name_sid = read_u32(payload, cursor + 16);
                let token_sid = read_u32(payload, cursor + 20);
                let vec_size = read_u16(payload, cursor + 24);
                summary.active_ruler_rows = summary.active_ruler_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, true);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "ruling_party",
                    party_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                if vec_size == 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} active-ruler leader_vec_size is zero"
                    ));
                }
                if (flags & ROW_FLAG_AMBIGUOUS != 0) != (vec_size != 1) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} ambiguous flag disagrees with leader_vec_size {vec_size}"
                    ));
                }
                if !seen_active_rulers.insert(country) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates active ruler for country tag id {country}"
                    ));
                }
            }
            ROW_ADVISOR_ROSTER | ROW_APPOINTED_ADVISOR => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    ADVISOR_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let slot_sid = read_u32(payload, cursor + 12);
                let ledger_sid = read_u32(payload, cursor + 16);
                let idea_sid = read_u32(payload, cursor + 20);
                validate_country(&mut summary, offset, row_index, country, true);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "slot",
                    slot_sid,
                    strings.len(),
                );
                validate_required_string_value(
                    &mut summary,
                    offset,
                    row_index,
                    "cabinet_bucket",
                    ledger_sid,
                    &strings,
                    ADVISOR_CABINET_BUCKETS,
                );
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "idea_token",
                    idea_sid,
                    strings.len(),
                );
                if kind == ROW_ADVISOR_ROSTER {
                    summary.advisor_roster_rows = summary.advisor_roster_rows.saturating_add(1);
                } else {
                    summary.appointed_advisor_rows =
                        summary.appointed_advisor_rows.saturating_add(1);
                }
                if !seen_advisor_rows.insert((kind, country, char_id, idea_sid)) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates advisor row"
                    ));
                }
            }
            ROW_CHARACTER_PORTRAIT => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    CHARACTER_PORTRAIT_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let role_sid = read_u32(payload, cursor + 12);
                let size_sid = read_u32(payload, cursor + 16);
                let gfx_sid = read_u32(payload, cursor + 20);
                summary.character_portrait_rows = summary.character_portrait_rows.saturating_add(1);
                // Same admission rule the row decoder applies, reported as a
                // parse error rather than a hard stop so the gate still counts
                // the rest of the payload.
                let unassigned = flags & !ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT;
                if unassigned != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} character portrait carries unassigned flag bits {unassigned:#06x}"
                    ));
                }
                if (flags & ROW_FLAG_PORTRAIT_GFX_WINDOWS1252) != 0 {
                    summary.character_portrait_windows1252_rows =
                        summary.character_portrait_windows1252_rows.saturating_add(1);
                }
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "portrait_role",
                    role_sid,
                    strings.len(),
                );
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "portrait_size",
                    size_sid,
                    strings.len(),
                );
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "portrait_gfx",
                    gfx_sid,
                    strings.len(),
                );
                if !seen_portrait_rows.insert((country, char_id, role_sid, size_sid, gfx_sid)) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates character portrait row"
                    ));
                }
            }
            _ => summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has unknown row kind {kind}"
            )),
        }
        if row_len == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has zero length at stream offset {row_offset}"
            ));
            break;
        }
        cursor += row_len;
        row_index += 1;
    }

    let observed_total = summary.row_count_total;
    let observed_characters = summary.character_rows;
    let observed_unit_leaders = summary.unit_leader_rows;
    let observed_country_leader_roles = summary.country_leader_role_rows;
    let observed_active_rulers = summary.active_ruler_rows;
    let observed_advisor_roster = summary.advisor_roster_rows;
    let observed_appointed_advisors = summary.appointed_advisor_rows;
    let observed_traits = summary.trait_rows;

    compare_count(
        &mut summary,
        offset,
        "row_count_total",
        expected_total,
        observed_total,
    );
    compare_count(
        &mut summary,
        offset,
        "character_rows",
        expected_characters,
        observed_characters,
    );
    compare_count(
        &mut summary,
        offset,
        "unit_leader_rows",
        expected_unit_leaders,
        observed_unit_leaders,
    );
    compare_count(
        &mut summary,
        offset,
        "country_leader_role_rows",
        expected_country_leader_roles,
        observed_country_leader_roles,
    );
    compare_count(
        &mut summary,
        offset,
        "active_ruler_rows",
        expected_active_rulers,
        observed_active_rulers,
    );
    compare_count(
        &mut summary,
        offset,
        "advisor_roster_rows",
        expected_advisor_roster,
        observed_advisor_roster,
    );
    compare_count(
        &mut summary,
        offset,
        "appointed_advisor_rows",
        expected_appointed_advisors,
        observed_appointed_advisors,
    );
    compare_count(
        &mut summary,
        offset,
        "trait_rows",
        expected_traits,
        observed_traits,
    );

    // A row of a family the header says was NOT observed is a payload
    // contradicting itself: the mask would tell a consumer to retain its prior
    // state for that family while the rows told it to replace it.
    for (family, name, observed) in [
        (FAMILY_CHARACTER, "character", observed_characters),
        (FAMILY_UNIT_LEADER, "unit leader", observed_unit_leaders),
        (FAMILY_UNIT_LEADER_TRAIT, "unit leader trait", observed_traits),
        (
            FAMILY_COUNTRY_LEADER_ROLE,
            "country leader role",
            observed_country_leader_roles,
        ),
        (FAMILY_ACTIVE_RULER, "active ruler", observed_active_rulers),
        (
            FAMILY_ADVISOR_ROSTER,
            "advisor roster",
            observed_advisor_roster,
        ),
        (
            FAMILY_APPOINTED_ADVISOR,
            "appointed advisor",
            observed_appointed_advisors,
        ),
        (
            FAMILY_CHARACTER_PORTRAIT,
            "character portrait",
            summary.character_portrait_rows,
        ),
    ] {
        if (family_presence_mask & family) == 0 && observed != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: character lane carries {observed} {name} row(s) for a family its \
                 header declares it did not observe"
            ));
        }
    }

    for (unit_id, expected) in declared_trait_counts {
        // A unit-leader row's `trait_count` is a statement about the trait
        // family. When that family was not observed the producer never walked
        // the trait vector, so the only honest value is zero — and zero here
        // means "not observed", never "this leader has no traits". Without this
        // the UnitLeader family would be declared present while quietly
        // asserting a trait count nobody read.
        if (family_presence_mask & FAMILY_UNIT_LEADER_TRAIT) == 0 && expected != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: unit leader {unit_id} declares {expected} traits although the \
                 trait family was not observed"
            ));
            continue;
        }
        let observed = observed_trait_counts.get(&unit_id).copied().unwrap_or(0);
        if observed != expected {
            summary.parse_errors.push(format!(
                "offset {offset}: unit leader {unit_id} declared {expected} traits but decoded {observed}"
            ));
        }
    }
    for unit_id in observed_trait_counts.keys() {
        if !seen_unit_leaders.contains(unit_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: trait rows reference unit leader {unit_id} without a unit-leader row"
            ));
        }
    }

    add_unit(&mut summary, "character_rows", observed_characters);
    add_unit(&mut summary, "unit_leader_rows", observed_unit_leaders);
    add_unit(&mut summary, "unit_leader_trait_rows", observed_traits);
    add_unit(
        &mut summary,
        "country_leader_role_rows",
        observed_country_leader_roles,
    );
    add_unit(&mut summary, "active_ruler_rows", observed_active_rulers);
    add_unit(&mut summary, "advisor_roster_rows", observed_advisor_roster);
    add_unit(
        &mut summary,
        "appointed_advisor_rows",
        observed_appointed_advisors,
    );
    let character_portrait_rows = summary.character_portrait_rows;
    add_unit(
        &mut summary,
        "character_portrait_rows",
        character_portrait_rows,
    );
    summary
}

pub fn decode_payload_rows(
    offset: usize,
    payload: &[u8],
) -> Result<DecodedDomain47Payload, Vec<String>> {
    let summary = decode_payload(offset, payload);
    if !summary.parse_errors.is_empty() {
        return Err(summary.parse_errors);
    }
    decode_payload_rows_checked(payload).map_err(|err| vec![format!("offset {offset}: {err}")])
}

fn decode_payload_rows_checked(payload: &[u8]) -> Result<DecodedDomain47Payload, String> {
    need_payload(payload, HEADER_LEN, "domain47 header")?;
    let status_flags = read_u32(payload, 8);
    let expected_total = read_u32(payload, 12) as usize;
    let string_count = read_u32(payload, 16) as usize;
    let string_bytes = read_u32(payload, 20) as usize;
    let row_bytes = read_u32(payload, 52) as usize;
    let schema_flags = read_u32(payload, 56);
    // Not re-validated here: `decode_payload_rows` runs the full
    // `decode_payload` gate first and returns its errors, so by this point the
    // declaration is either valid or the caller never got here.
    let family_presence_mask = if (schema_flags & SCHEMA_FAMILY_PRESENCE_DECLARED) != 0 {
        read_u32(payload, 60)
    } else {
        FAMILY_ALL
    };

    let mut cursor = HEADER_LEN;
    let string_end = cursor
        .checked_add(string_bytes)
        .ok_or("domain47 string table length overflow")?;
    need_payload(payload, string_end, "domain47 string table")?;
    let mut strings = Vec::with_capacity(string_count);
    for index in 0..string_count {
        need_payload(payload, cursor + 2, "domain47 string length")?;
        let len = read_u16(payload, cursor) as usize;
        cursor += 2;
        if len > MAX_STRING_BYTES {
            return Err(format!(
                "domain47 string {index} exceeds {MAX_STRING_BYTES} bytes"
            ));
        }
        need_payload(payload, cursor + len, "domain47 string bytes")?;
        let raw = &payload[cursor..cursor + len];
        let value = std::str::from_utf8(raw)
            .map_err(|err| format!("domain47 string {index} invalid UTF-8: {err}"))?;
        if !is_domain47_string_value(value) {
            return Err(format!(
                "domain47 string {index} contains control characters"
            ));
        }
        strings.push(value.to_string());
        cursor += len;
    }
    if cursor != string_end {
        return Err("domain47 decoded string table length mismatch".into());
    }

    let rows_end = cursor
        .checked_add(row_bytes)
        .ok_or("domain47 row bytes overflow")?;
    if rows_end != payload.len() {
        return Err(format!(
            "domain47 row bytes {row_bytes} do not match payload remainder {}",
            payload.len().saturating_sub(cursor)
        ));
    }

    let mut rows = Vec::with_capacity(expected_total);
    while cursor < rows_end {
        need_payload(payload, cursor + 4, "domain47 row header")?;
        let kind = payload[cursor];
        let row_len = payload[cursor + 1] as usize;
        let flags = read_u16(payload, cursor + 2);
        if row_len < 4 {
            return Err(format!(
                "domain47 row kind {kind} has invalid length {row_len}"
            ));
        }
        need_payload(payload, cursor + row_len, "domain47 row body")?;
        match kind {
            ROW_CHARACTER => {
                require_decode_len(kind, row_len, CHARACTER_ROW_LEN)?;
                rows.push(DecodedDomain47Row::Character {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 12))?,
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                });
            }
            ROW_UNIT_LEADER => {
                require_decode_len(kind, row_len, UNIT_LEADER_ROW_LEN)?;
                let skill_raw = read_u32(payload, cursor + 16);
                let experience_raw = read_i32(payload, cursor + 20);
                rows.push(DecodedDomain47Row::UnitLeader {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    unit_leader_id: read_u32(payload, cursor + 8),
                    char_id: read_u32(payload, cursor + 12),
                    leader_kind: payload[cursor + 40],
                    skill: if (flags & ROW_FLAG_SKILL_PRESENT) != 0 && skill_raw != SKILL_NONE {
                        Some(skill_raw)
                    } else {
                        None
                    },
                    experience_raw: if (flags & ROW_FLAG_EXPERIENCE_PRESENT) != 0
                        && experience_raw != EXPERIENCE_RAW_NONE
                    {
                        Some(experience_raw)
                    } else {
                        None
                    },
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 24))?,
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 28))?,
                    attack: decoded_optional_byte(payload[cursor + 34]),
                    defense: decoded_optional_byte(payload[cursor + 35]),
                    planning: decoded_optional_byte(payload[cursor + 36]),
                    logistics: decoded_optional_byte(payload[cursor + 37]),
                    maneuvering: decoded_optional_byte(payload[cursor + 38]),
                    coordination: decoded_optional_byte(payload[cursor + 39]),
                });
            }
            ROW_UNIT_LEADER_TRAIT => {
                require_decode_len(kind, row_len, TRAIT_ROW_LEN)?;
                rows.push(DecodedDomain47Row::UnitLeaderTrait {
                    unit_leader_id: read_u32(payload, cursor + 4),
                    ordinal: read_u16(payload, cursor + 8),
                    trait_token: decoded_required_string(&strings, read_u32(payload, cursor + 12))?,
                });
            }
            ROW_COUNTRY_LEADER_ROLE => {
                require_decode_len(kind, row_len, COUNTRY_LEADER_ROLE_ROW_LEN)?;
                rows.push(DecodedDomain47Row::CountryLeaderRole {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 12))?,
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                });
            }
            ROW_ACTIVE_RULER => {
                require_decode_len(kind, row_len, ACTIVE_RULER_ROW_LEN)?;
                rows.push(DecodedDomain47Row::ActiveRuler {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    ruling_party: decoded_optional_string(
                        &strings,
                        read_u32(payload, cursor + 12),
                    )?,
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 20))?,
                    leader_vec_size: read_u16(payload, cursor + 24),
                    ambiguous: (flags & ROW_FLAG_AMBIGUOUS) != 0,
                });
            }
            ROW_ADVISOR_ROSTER | ROW_APPOINTED_ADVISOR => {
                require_decode_len(kind, row_len, ADVISOR_ROW_LEN)?;
                let row = (
                    read_u32(payload, cursor + 4),
                    read_u32(payload, cursor + 8),
                    decoded_optional_string(&strings, read_u32(payload, cursor + 12))?,
                    decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                    decoded_required_string(&strings, read_u32(payload, cursor + 20))?,
                );
                if kind == ROW_ADVISOR_ROSTER {
                    rows.push(DecodedDomain47Row::AdvisorRoster {
                        country_tag_db_id: row.0,
                        char_id: row.1,
                        slot: row.2,
                        ledger: row.3,
                        idea_token: row.4,
                    });
                } else {
                    rows.push(DecodedDomain47Row::AppointedAdvisor {
                        country_tag_db_id: row.0,
                        char_id: row.1,
                        slot: row.2,
                        ledger: row.3,
                        idea_token: row.4,
                    });
                }
            }
            ROW_CHARACTER_PORTRAIT => {
                require_decode_len(kind, row_len, CHARACTER_PORTRAIT_ROW_LEN)?;
                // An unassigned flag bit is a claim by a producer this decoder
                // does not understand. Refused, not masked away: masking would
                // silently publish the row as if the producer had said nothing.
                let unassigned = flags & !ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT;
                if unassigned != 0 {
                    return Err(format!(
                        "domain47 character portrait row carries unassigned flag bits {unassigned:#06x}"
                    ));
                }
                rows.push(DecodedDomain47Row::CharacterPortrait {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    role: decoded_required_string(&strings, read_u32(payload, cursor + 12))?,
                    size: decoded_required_string(&strings, read_u32(payload, cursor + 16))?,
                    gfx: decoded_required_string(&strings, read_u32(payload, cursor + 20))?,
                    gfx_encoding: if (flags & ROW_FLAG_PORTRAIT_GFX_WINDOWS1252) != 0 {
                        PortraitIdentityEncoding::Windows1252
                    } else {
                        PortraitIdentityEncoding::Utf8
                    },
                });
            }
            _ => return Err(format!("domain47 unknown row kind {kind}")),
        }
        cursor += row_len;
    }

    Ok(DecodedDomain47Payload {
        status_flags,
        schema_flags,
        family_presence_mask,
        rows,
    })
}

fn encode_advisor_row(
    row_bytes: &mut Vec<u8>,
    strings: &mut StringTable,
    kind: u8,
    row: &AdvisorRow<'_>,
) {
    push_header(row_bytes, kind, ADVISOR_ROW_LEN, 0);
    push_u32(row_bytes, row.country_tag_db_id);
    push_u32(row_bytes, row.char_id);
    push_u32(row_bytes, strings.intern(row.slot));
    push_u32(row_bytes, strings.intern(row.ledger));
    push_u32(row_bytes, strings.intern(Some(row.idea_token)));
}

fn push_header(out: &mut Vec<u8>, kind: u8, row_len: u8, flags: u16) {
    out.push(kind);
    out.push(row_len);
    out.extend_from_slice(&flags.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(payload[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 range"))
}

fn require_len(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    kind: u8,
    got: usize,
    want: u8,
) {
    if got != want as usize {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} kind {kind} length {got} != {}",
            want
        ));
    }
}

fn validate_country(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    country: u32,
    nonzero: bool,
) {
    if (nonzero && country == 0) || country > MAX_TAG_DB_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} has invalid country_tag_db_id {country}"
        ));
    }
}

fn validate_character(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    char_id: u32,
) {
    if char_id == 0 || char_id > MAX_CHARACTER_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} has invalid char_id {char_id}"
        ));
    }
}

fn validate_unit_leader(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    unit_id: u32,
) {
    if unit_id == 0 || unit_id > MAX_UNIT_LEADER_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} has invalid unit_leader_id {unit_id}"
        ));
    }
}

fn validate_optional_string(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    field: &str,
    string_id: u32,
    string_count: usize,
) {
    if string_id != STRING_NONE && string_id as usize >= string_count {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid string id {string_id}"
        ));
    }
}

fn validate_required_string(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    field: &str,
    string_id: u32,
    string_count: usize,
) {
    if string_id == STRING_NONE || string_id as usize >= string_count {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid required string id {string_id}"
        ));
    }
}

fn validate_required_string_value(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    field: &str,
    string_id: u32,
    strings: &[&[u8]],
    allowed: &[&[u8]],
) {
    if string_id == STRING_NONE || string_id as usize >= strings.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid required string id {string_id}"
        ));
        return;
    }
    let value = strings[string_id as usize];
    if !allowed.iter().any(|allowed_value| *allowed_value == value) {
        let value = String::from_utf8_lossy(value);
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid value {value}"
        ));
    }
}

fn compare_count(
    summary: &mut Domain44GateSummary,
    offset: usize,
    label: &'static str,
    expected: u64,
    observed: u64,
) {
    if expected != observed {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} header {expected} != decoded {observed}"
        ));
    }
}

fn add_unit(summary: &mut Domain44GateSummary, key: &'static str, value: u64) {
    if value == 0 {
        return;
    }
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value);
}

fn need_payload(payload: &[u8], at_least: usize, label: &str) -> Result<(), String> {
    if payload.len() < at_least {
        Err(format!(
            "{label}: need {at_least} bytes, have {}",
            payload.len()
        ))
    } else {
        Ok(())
    }
}

fn require_decode_len(kind: u8, got: usize, want: u8) -> Result<(), String> {
    if got != want as usize {
        Err(format!("domain47 row kind {kind} length {got} != {}", want))
    } else {
        Ok(())
    }
}

fn decoded_optional_byte(raw: u8) -> Option<u8> {
    (raw != BYTE_NONE).then_some(raw)
}

fn decoded_optional_string(strings: &[String], string_id: u32) -> Result<Option<String>, String> {
    if string_id == STRING_NONE {
        return Ok(None);
    }
    decoded_required_string(strings, string_id).map(Some)
}

fn decoded_required_string(strings: &[String], string_id: u32) -> Result<String, String> {
    strings
        .get(string_id as usize)
        .cloned()
        .ok_or_else(|| format!("domain47 invalid string id {string_id}"))
}

#[cfg(not(test))]
fn main() {
    if let Err(err) = run_cli() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

#[cfg(not(test))]
fn run_cli() -> Result<(), String> {
    let mut args = std::env::args();
    let program = args
        .next()
        .unwrap_or_else(|| "domain47_compact".to_string());
    let Some(path) = args.next() else {
        return Err(format!("usage: {program} <domain47_payload_or_record.bin>"));
    };
    if args.next().is_some() {
        return Err(format!("usage: {program} <domain47_payload_or_record.bin>"));
    }
    let bytes = std::fs::read(&path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let (payload, wrapped_record) = payload_from_input(&bytes)?;
    let summary = decode_payload(0, payload);
    println!("{}", summary_json(&summary, wrapped_record, payload.len()));
    if !summary.parse_errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(not(test))]
fn payload_from_input(bytes: &[u8]) -> Result<(&[u8], bool), String> {
    if bytes.starts_with(COMPACT_RECORD_MAGIC) {
        let header = parse_compact_record_header(bytes).map_err(|err| match err {
            RecordFramingError::TruncatedHeader(_) | RecordFramingError::BadMagic => {
                format!("invalid compact record header: {err}")
            }
            RecordFramingError::UnsupportedVersion(version) => {
                format!("unsupported compact record version {version}")
            }
            RecordFramingError::UnsupportedHeaderLen(header_len) => {
                format!("header_len {header_len} != {COMPACT_RECORD_HEADER_LEN}")
            }
            RecordFramingError::PayloadTooLarge(_) => {
                format!("invalid compact record header: {err}")
            }
        })?;
        if header.domain_id != DOMAIN_ID {
            return Err(format!("domain_id {} != {DOMAIN_ID}", header.domain_id));
        }
        if header.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
            return Err(format!(
                "record_type {} != {COMPACT_RECORD_TYPE_DOMAIN_DATA}",
                header.record_type
            ));
        }
        if bytes.len() != header.record_len() {
            return Err(format!(
                "record payload length {} plus header {COMPACT_RECORD_HEADER_LEN} != file bytes {}",
                header.payload_len,
                bytes.len()
            ));
        }
        compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            &bytes[COMPACT_RECORD_HEADER_LEN..],
        )
        .map_err(|error| format!("invalid compact record semantics: {error}"))?;
        return Ok((&bytes[COMPACT_RECORD_HEADER_LEN..], true));
    }
    Ok((bytes, false))
}

#[cfg(not(test))]
fn summary_json(
    summary: &Domain44GateSummary,
    wrapped_record: bool,
    payload_bytes: usize,
) -> String {
    let mut units = String::new();
    units.push('{');
    for (idx, (key, value)) in summary.data_units_total.iter().enumerate() {
        if idx > 0 {
            units.push(',');
        }
        units.push('"');
        units.push_str(key);
        units.push_str("\":");
        units.push_str(&value.to_string());
    }
    units.push('}');
    format!(
        "{{\"domain\":\"{}\",\"domain_id\":{},\"wrapped_record\":{},\"payload_bytes\":{},\"status_flags_or\":{},\"schema_flags_or\":{},\"row_count_total\":{},\"character_rows\":{},\"unit_leader_rows\":{},\"unit_leader_trait_rows\":{},\"country_leader_role_rows\":{},\"active_ruler_rows\":{},\"advisor_roster_rows\":{},\"appointed_advisor_rows\":{},\"character_portrait_rows\":{},\"character_portrait_windows1252_rows\":{},\"string_count\":{},\"string_bytes\":{},\"data_units_total\":{},\"parse_errors\":{}}}",
        json_escape(DOMAIN_NAME),
        DOMAIN_ID,
        wrapped_record,
        payload_bytes,
        summary.status_flags_or,
        summary.schema_flags_or,
        summary.row_count_total,
        summary.character_rows,
        summary.unit_leader_rows,
        summary.trait_rows,
        summary.country_leader_role_rows,
        summary.active_ruler_rows,
        summary.advisor_roster_rows,
        summary.appointed_advisor_rows,
        summary.character_portrait_rows,
        summary.character_portrait_windows1252_rows,
        summary.string_count,
        summary.string_bytes,
        units,
        json_string_array(&summary.parse_errors),
    )
}

#[cfg(not(test))]
fn json_string_array(values: &[String]) -> String {
    let mut out = String::from("[");
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(value));
        out.push('"');
    }
    out.push(']');
    out
}

#[cfg(not(test))]
fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MAX_ROW_LEN` sizes the row buffer up front. If a row kind is ever
    /// wider than it, the reservation stops being an upper bound and the
    /// buffer silently starts reallocating again — the exact cost it was
    /// introduced to remove, with nothing observable to say so.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: widening any of the seven
    /// EXISTING row-length constants past `MAX_ROW_LEN` without updating it.
    ///
    /// WHAT IT DOES NOT CATCH, stated because the list below is maintained by
    /// hand (Codex review 019fd6bf-a0af-7ca2-a974-880b6832bba6, OPEN 2): a
    /// NEW row kind whose constant is never added here, and a row arm that
    /// writes more bytes than its own constant declares. Neither is a
    /// byte-correctness defect — a capacity that is too small only makes
    /// `Vec` reallocate, which is the cost this reservation exists to avoid,
    /// not a wrong payload — so this is a coverage gap in a performance
    /// guard, and it is named rather than papered over.
    #[test]
    fn max_row_len_covers_every_row_kind() {
        for (name, len) in [
            ("character", CHARACTER_ROW_LEN),
            ("unit_leader", UNIT_LEADER_ROW_LEN),
            ("trait", TRAIT_ROW_LEN),
            ("country_leader_role", COUNTRY_LEADER_ROLE_ROW_LEN),
            ("active_ruler", ACTIVE_RULER_ROW_LEN),
            ("advisor", ADVISOR_ROW_LEN),
            ("character_portrait", CHARACTER_PORTRAIT_ROW_LEN),
        ] {
            assert!(
                len <= MAX_ROW_LEN,
                "{name} row is {len} bytes, wider than MAX_ROW_LEN {MAX_ROW_LEN}"
            );
        }
    }

    /// The string table's IDs are indices into the encode-order block, so
    /// they must follow FIRST-USE order and repeats must return the id the
    /// first occurrence got. The lookup side is a hash map, which has no
    /// order at all — this pins the property the emitted bytes actually
    /// depend on, rather than the container that happens to provide it.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: handing out ids from the
    /// lookup map's iteration order (or sorting `values`) instead of
    /// appending to `values` — which is exactly the mistake available now
    /// that the map is unordered, and which the previous `BTreeMap` shape
    /// would have masked by being sorted anyway.
    #[test]
    fn string_ids_follow_first_use_order_and_repeats_reuse_them() {
        let mut table = StringTable::default();
        // Deliberately NOT in sorted order: under the old BTreeMap a test
        // that used sorted inputs could not tell insertion order from key
        // order.
        assert_eq!(table.intern(Some("zulu")), 0);
        assert_eq!(table.intern(Some("alpha")), 1);
        assert_eq!(table.intern(Some("mike")), 2);
        assert_eq!(table.intern(Some("alpha")), 1, "repeat must reuse its id");
        assert_eq!(table.intern(Some("zulu")), 0, "repeat must reuse its id");
        assert_eq!(table.intern(None), STRING_NONE);
        assert_eq!(
            table.values.iter().map(|v| &**v).collect::<Vec<_>>(),
            vec!["zulu", "alpha", "mike"],
            "the encoded block must be in first-use order"
        );

        // The block the decoder will read back, byte for byte.
        let mut out = Vec::new();
        table.encode_into(&mut out);
        assert_eq!(out.len(), table.encoded_len());
        assert_eq!(
            out,
            b"\x04\x00zulu\x05\x00alpha\x04\x00mike".to_vec(),
            "the encoded string block changed"
        );
    }

    /// The validity checks moved BEHIND the interned-value lookup, so this
    /// pins that a bad string is still refused on its FIRST occurrence —
    /// i.e. that the move traded no rejection away.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping either assert, or
    /// interning the value before checking it.
    #[test]
    #[should_panic(expected = "character lane string must be valid non-control UTF-8")]
    fn a_control_character_is_still_refused_on_first_sight() {
        let mut table = StringTable::default();
        table.intern(Some("has a \u{7} bell"));
    }

    /// The same, for the length bound.
    #[test]
    #[should_panic(expected = "character lane string exceeds max length")]
    fn an_overlong_string_is_still_refused_on_first_sight() {
        let mut table = StringTable::default();
        let overlong = "x".repeat(MAX_STRING_BYTES + 1);
        table.intern(Some(&overlong));
    }

    #[test]
    fn row_len_literals_are_stable() {
        // Pin the per-kind row-length literals (stale-const guard).
        assert_eq!(CHARACTER_ROW_LEN, 20);
        assert_eq!(UNIT_LEADER_ROW_LEN, 42);
        assert_eq!(TRAIT_ROW_LEN, 16);
        assert_eq!(COUNTRY_LEADER_ROLE_ROW_LEN, 20);
        assert_eq!(ACTIVE_RULER_ROW_LEN, 28);
        assert_eq!(ADVISOR_ROW_LEN, 24);
        assert_eq!(CHARACTER_PORTRAIT_ROW_LEN, 24);
    }

    /// The end of the chain that run 20260731-deflist-16 broke at the start
    /// of. The collector census now ADMITS country_tag_db_id == 0 (the
    /// engine's constructor default, meaning "no country"), so the wire
    /// format has to keep accepting it -- and the collector-side tests
    /// cannot see this file. Flipping the character row's
    /// `validate_country(.., nonzero)` to true re-breaks zero-tag output
    /// while every collector test stays green.
    #[test]
    fn a_character_with_no_country_survives_encode_and_decode() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 0,
                char_id: 4711,
                name: Some("Unaffiliated Character"),
                token: Some("generic_unaffiliated"),
            }),
            // An unassigned leader is the same shape one level down, and is
            // permitted for the same reason.
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 0,
                unit_leader_id: 991,
                char_id: 4711,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(3),
                experience_raw: Some(0),
                attack: Some(1),
                defense: Some(1),
                planning: Some(1),
                logistics: Some(1),
                maneuvering: None,
                coordination: None,
                token: Some("generic_unaffiliated"),
                name: Some("Unaffiliated Character"),
                trait_count: 0,
            }),
        ];
        let payload = encode_payload(1, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "a character with no country must encode and decode cleanly: {:?}",
            summary.parse_errors
        );
        assert_eq!(summary.row_count_total, 2);
        assert_eq!(summary.unit_leader_rows, 1);

        // And the ceiling above it is still enforced, so the two ends of the
        // range cannot drift apart: this is where an over-ceiling tag is
        // caught, which is why the census reports kind 32 rather than
        // silently handing the encoder a tag the profile forbids.
        let over = vec![CharacterLaneRow::Character(CharacterRow {
            country_tag_db_id: MAX_TAG_DB_ID + 1,
            char_id: 4712,
            name: None,
            token: None,
        })];
        let over_summary = decode_payload(0, &encode_payload(2, 0, SCHEMA_ROOTS_VALIDATED, &over));
        assert!(
            !over_summary.parse_errors.is_empty(),
            "a tag over the profile ceiling must not validate"
        );
    }

    #[test]
    fn encodes_full_character_leader_cabinet_slice() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 3,
                char_id: 2458,
                name: Some("Georgy Zhukov"),
                token: Some("SOV_georgy_zhukov"),
            }),
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 3,
                unit_leader_id: 676,
                char_id: 2458,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(8),
                experience_raw: Some(42_000_000),
                attack: Some(6),
                defense: Some(7),
                planning: Some(5),
                logistics: Some(10),
                maneuvering: None,
                coordination: None,
                token: Some("SOV_georgy_zhukov"),
                name: Some("Georgy Zhukov"),
                trait_count: 2,
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 676,
                ordinal: 0,
                trait_token: "brilliant_strategist",
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 676,
                ordinal: 1,
                trait_token: "panzer_leader",
            }),
            CharacterLaneRow::CountryLeaderRole(CountryLeaderRoleRow {
                country_tag_db_id: 3,
                char_id: 2450,
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
            }),
            CharacterLaneRow::ActiveRuler(ActiveRulerRow {
                country_tag_db_id: 3,
                char_id: 2450,
                ruling_party: Some("communism"),
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
                leader_vec_size: 1,
                ambiguous: false,
            }),
            CharacterLaneRow::AdvisorRoster(AdvisorRow {
                country_tag_db_id: 2,
                char_id: 1001,
                slot: Some("political_advisor"),
                ledger: Some("political_advisor"),
                idea_token: "ENG_maxwell_aitken",
            }),
            CharacterLaneRow::AppointedAdvisor(AdvisorRow {
                country_tag_db_id: 2,
                char_id: 1001,
                slot: Some("political_advisor"),
                ledger: Some("political_advisor"),
                idea_token: "ENG_maxwell_aitken",
            }),
            CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                country_tag_db_id: 3,
                char_id: 2458,
                role: "army",
                size: "large",
                gfx: "GFX_portrait_SOV_georgy_zhukov",
                gfx_encoding: PortraitIdentityEncoding::Utf8,
            }),
        ];
        let payload = encode_payload(
            123,
            0,
            SCHEMA_ROOTS_VALIDATED
                | SCHEMA_STATIC_NAME_CLOSURE
                | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            &rows,
        );
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.schema_flags_or & SCHEMA_ROOTS_VALIDATED,
            SCHEMA_ROOTS_VALIDATED
        );
        assert_eq!(summary.status_flags_or, 0);
        assert_eq!(summary.row_count_total, 9);
        assert_eq!(summary.unit_leader_rows, 1);
        assert_eq!(summary.trait_rows, 2);
        assert_eq!(summary.active_ruler_rows, 1);
        assert_eq!(summary.appointed_advisor_rows, 1);
        assert_eq!(summary.character_portrait_rows, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("unit_leader_trait_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("character_portrait_rows")
                .copied(),
            Some(1)
        );

        let compact = encode_compact_data_record(7, 42, &payload);
        assert_eq!(&compact[0..4], COMPACT_RECORD_MAGIC);
        assert_eq!(
            u16::from_le_bytes(compact[20..22].try_into().unwrap()),
            DOMAIN_ID
        );
        assert_eq!(
            u16::from_le_bytes(compact[22..24].try_into().unwrap()),
            COMPACT_RECORD_TYPE_DOMAIN_DATA
        );
    }

    /// THE PORTRAIT IDENTITY'S PROVENANCE SURVIVES THE WIRE, PER ROW.
    ///
    /// The producer decodes a non-UTF-8 engine identity as Windows-1252 and
    /// says that it did; the string in the table is the decoded text either
    /// way, so a consumer that has to know which decoding produced it can only
    /// learn that from this flag. Three rows go in — UTF-8, Windows-1252, UTF-8
    /// — and all three must come back with the encoding they went in with, in
    /// order.
    ///
    /// The encoded ROW LENGTH is unchanged: the flag lives in the row header's
    /// existing flag word, which every portrait row has always carried as zero.
    ///
    /// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: dropping the flag in the
    /// encoder (every row decodes as `Utf8`), setting it unconditionally,
    /// reading it from the wrong bit, or widening the portrait row to carry the
    /// provenance as a payload field without updating both sides.
    #[test]
    fn a_portrait_identity_carries_its_decoding_across_the_wire() {
        let rows = vec![
            CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                country_tag_db_id: 3,
                char_id: 2458,
                role: "army",
                size: "large",
                gfx: "GFX_portrait_SOV_georgy_zhukov",
                gfx_encoding: PortraitIdentityEncoding::Utf8,
            }),
            CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                country_tag_db_id: 3,
                char_id: 2459,
                role: "navy",
                size: "small",
                gfx: "GFX_portrait_caf\u{e9}",
                gfx_encoding: PortraitIdentityEncoding::Windows1252,
            }),
            CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                country_tag_db_id: 3,
                char_id: 2460,
                role: "civilian",
                size: "large",
                gfx: "GFX_portrait_plain",
                gfx_encoding: PortraitIdentityEncoding::Utf8,
            }),
        ];
        let payload = encode_payload(11, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let decoded = decode_payload_rows(0, &payload).expect("payload must decode");
        let encodings: Vec<PortraitIdentityEncoding> = decoded
            .rows
            .iter()
            .filter_map(|row| match row {
                DecodedDomain47Row::CharacterPortrait { gfx_encoding, .. } => Some(*gfx_encoding),
                _ => None,
            })
            .collect();
        assert_eq!(
            encodings,
            vec![
                PortraitIdentityEncoding::Utf8,
                PortraitIdentityEncoding::Windows1252,
                PortraitIdentityEncoding::Utf8,
            ],
            "each row must come back with the decoding it was encoded with"
        );
        assert_eq!(
            decoded.rows[1],
            DecodedDomain47Row::CharacterPortrait {
                country_tag_db_id: 3,
                char_id: 2459,
                role: "navy".to_string(),
                size: "small".to_string(),
                gfx: "GFX_portrait_caf\u{e9}".to_string(),
                gfx_encoding: PortraitIdentityEncoding::Windows1252,
            }
        );

        // The gate summary sees the same census.
        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.character_portrait_rows, 3);
        assert_eq!(summary.character_portrait_windows1252_rows, 1);

        // AND THE FLAG IS THE ONLY DIFFERENCE. The same rows all-UTF-8 encode
        // to a payload of the same length, so the provenance costs no row bytes
        // and cannot have been smuggled into the payload.
        let utf8_rows: Vec<CharacterLaneRow> = rows
            .iter()
            .map(|row| match row {
                CharacterLaneRow::CharacterPortrait(portrait) => {
                    CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                        gfx_encoding: PortraitIdentityEncoding::Utf8,
                        ..portrait.clone()
                    })
                }
                other => other.clone(),
            })
            .collect();
        let utf8_payload = encode_payload(11, 0, SCHEMA_ROOTS_VALIDATED, &utf8_rows);
        assert_eq!(utf8_payload.len(), payload.len());
        assert_ne!(
            utf8_payload, payload,
            "the two payloads must differ in the flag word, or the flag is not \
             being written at all"
        );
        assert_eq!(
            decode_payload(0, &utf8_payload).character_portrait_windows1252_rows,
            0
        );
    }

    /// AN UNASSIGNED PORTRAIT ROW FLAG IS REFUSED, not masked away.
    ///
    /// A bit this decoder does not understand is a claim by a producer it does
    /// not understand. Dropping it would publish the row as if the producer had
    /// said nothing — which is exactly how the provenance flag itself would
    /// have been lost, silently, by any decoder written before it existed.
    ///
    /// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: masking the flag word to
    /// the assigned bits instead of refusing, or forgetting to widen
    /// `ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT` when a new bit is assigned
    /// (the new bit's own round-trip test would then fail here first).
    #[test]
    fn a_portrait_row_with_an_unassigned_flag_bit_is_refused() {
        let rows = vec![CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
            country_tag_db_id: 3,
            char_id: 2458,
            role: "army",
            size: "large",
            gfx: "GFX_portrait_SOV_georgy_zhukov",
            gfx_encoding: PortraitIdentityEncoding::Utf8,
        })];
        let payload = encode_payload(11, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        // The row block starts after the header and the string table; the
        // string-table byte count is the header word at offset 20.
        let string_bytes = u32::from_le_bytes(payload[20..24].try_into().unwrap()) as usize;
        let row_start = HEADER_LEN + string_bytes;
        assert_eq!(payload[row_start], ROW_CHARACTER_PORTRAIT);
        // The clean payload decodes, so the flag word is the only thing the
        // surgery below changes.
        assert!(decode_payload_rows(0, &payload).is_ok());

        // Bit 0 means something on ANOTHER row kind (`ROW_FLAG_AMBIGUOUS` on an
        // active-ruler row) and bit 15 means nothing anywhere. Neither is
        // assigned on a portrait row, and the rule is about THIS row's
        // vocabulary, so both must refuse.
        for bit in [1u16, 1u16 << 15] {
            assert_eq!(
                bit & ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT,
                0,
                "the fixture bit must be unassigned for a portrait row"
            );
            let mut tampered = payload.clone();
            tampered[row_start + 2..row_start + 4].copy_from_slice(&bit.to_le_bytes());
            let error = decode_payload_rows(0, &tampered)
                .expect_err("an unassigned portrait flag bit must be refused");
            assert!(
                error.iter().any(|line| line.contains("unassigned flag bits")),
                "{error:?}"
            );
            assert!(
                decode_payload(0, &tampered)
                    .parse_errors
                    .iter()
                    .any(|line| line.contains("unassigned flag bits")),
                "the offline gate must report it too"
            );
        }
    }

    #[test]
    fn accepts_measured_empty_appointee_slice() {
        let rows = vec![CharacterLaneRow::ActiveRuler(ActiveRulerRow {
            country_tag_db_id: 20,
            char_id: 300,
            ruling_party: Some("neutrality"),
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_antonio_de_oliveira_salazar"),
            leader_vec_size: 1,
            ambiguous: false,
        })];
        let payload = encode_payload(
            25,
            0,
            SCHEMA_ROOTS_VALIDATED | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            &rows,
        );
        let summary = decode_payload(12, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.appointed_advisor_rows, 0);
        assert_eq!(summary.active_ruler_rows, 1);
    }

    #[test]
    fn preserves_utf8_display_names() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 1,
                char_id: 701,
                name: Some("Günther Lütjens"),
                token: Some("GER_gunther_lutjens"),
            }),
            CharacterLaneRow::ActiveRuler(ActiveRulerRow {
                country_tag_db_id: 1,
                char_id: 701,
                ruling_party: Some("neutrality"),
                name: Some("Günther Lütjens"),
                token: Some("GER_gunther_lutjens"),
                leader_vec_size: 1,
                ambiguous: false,
            }),
        ];
        let payload = encode_payload(25, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let decoded = decode_payload_rows(0, &payload).unwrap();
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            DecodedDomain47Row::Character { name: Some(name), .. }
                if name == "Günther Lütjens"
        )));
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            DecodedDomain47Row::ActiveRuler { name: Some(name), .. }
                if name == "Günther Lütjens"
        )));
    }

    #[test]
    fn rejects_unknown_advisor_bucket_values_outside_enum() {
        let rows = vec![CharacterLaneRow::AdvisorRoster(AdvisorRow {
            country_tag_db_id: 2,
            char_id: 1001,
            slot: Some("political_advisor"),
            ledger: Some("political"),
            idea_token: "ENG_maxwell_aitken",
        })];
        let payload = encode_payload(25, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.iter().any(
                |err| err.contains("cabinet_bucket") && err.contains("invalid value political")
            ),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn rejects_trait_count_mismatch_and_duplicate_unit_leader() {
        let rows = vec![
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 1,
                unit_leader_id: 10,
                char_id: 100,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(4),
                experience_raw: Some(1),
                attack: Some(1),
                defense: Some(1),
                planning: Some(1),
                logistics: Some(1),
                maneuvering: None,
                coordination: None,
                token: Some("GER_test"),
                name: Some("Test"),
                trait_count: 2,
            }),
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 1,
                unit_leader_id: 10,
                char_id: 101,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(4),
                experience_raw: Some(1),
                attack: Some(1),
                defense: Some(1),
                planning: Some(1),
                logistics: Some(1),
                maneuvering: None,
                coordination: None,
                token: Some("GER_test_2"),
                name: Some("Test 2"),
                trait_count: 0,
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 10,
                ordinal: 0,
                trait_token: "old_guard",
            }),
        ];
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(4, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicates unit leader id 10")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("declared 0 traits but decoded 1")));
    }

    #[test]
    fn rejects_bad_active_ruler_ambiguity_flag() {
        let rows = vec![CharacterLaneRow::ActiveRuler(ActiveRulerRow {
            country_tag_db_id: 3,
            char_id: 2450,
            ruling_party: Some("communism"),
            name: Some("Iosif Stalin"),
            token: Some("SOV_iosif_stalin"),
            leader_vec_size: 2,
            ambiguous: false,
        })];
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("ambiguous flag disagrees")));
    }

    /// The family bits and the row kinds are one vocabulary, not two lists that
    /// have to be kept in step by hand.
    #[test]
    fn every_row_kind_has_exactly_one_family_bit() {
        let kinds = [
            ROW_CHARACTER,
            ROW_UNIT_LEADER,
            ROW_UNIT_LEADER_TRAIT,
            ROW_COUNTRY_LEADER_ROLE,
            ROW_ACTIVE_RULER,
            ROW_ADVISOR_ROSTER,
            ROW_APPOINTED_ADVISOR,
            ROW_CHARACTER_PORTRAIT,
        ];
        assert_eq!(kinds.len(), FAMILY_NAMES.len());
        let mut union = 0u32;
        for (index, kind) in kinds.into_iter().enumerate() {
            let bit = 1u32 << (kind - 1);
            assert_eq!(
                FAMILY_NAMES[index].0, bit,
                "row kind {kind} and family name table disagree"
            );
            assert_eq!(union & bit, 0, "family bit for row kind {kind} is duplicated");
            union |= bit;
        }
        assert_eq!(union, FAMILY_ALL);
        assert_eq!(
            FAMILY_CENSUS_ONLY | FAMILY_HEAP_SWEEP_DERIVED,
            FAMILY_ALL,
            "the census and sweep sets must partition the families, or a family belongs to neither"
        );
        assert_eq!(FAMILY_CENSUS_ONLY & FAMILY_HEAP_SWEEP_DERIVED, 0);
    }

    fn declared(mask: u32, rows: &[CharacterLaneRow<'_>]) -> Vec<String> {
        let payload = encode_payload_with_families(
            0,
            0,
            SCHEMA_ROOTS_VALIDATED
                | SCHEMA_STATIC_NAME_CLOSURE
                | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            mask,
            rows,
        );
        decode_payload(0, &payload).parse_errors
    }

    /// A declared census-only payload is the ordinary new shape and must decode
    /// cleanly, carrying its mask through to the consumer.
    #[test]
    fn a_declared_census_only_payload_decodes_and_carries_its_mask() {
        let rows = vec![CharacterLaneRow::Character(CharacterRow {
            country_tag_db_id: 3,
            char_id: 2450,
            name: Some("Iosif Stalin"),
            token: Some("SOV_iosif_stalin"),
        })];
        let payload = encode_payload_with_families(
            0,
            0,
            SCHEMA_ROOTS_VALIDATED
                | SCHEMA_STATIC_NAME_CLOSURE
                | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            FAMILY_CENSUS_ONLY,
            &rows,
        );
        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.family_presence_mask, FAMILY_CENSUS_ONLY);
        // The encoder derives the status from the mask, so the two can never
        // disagree about whether this hour is complete.
        assert_eq!(summary.status_flags_or & STATUS_PARTIAL_DATA, STATUS_PARTIAL_DATA);
        assert_eq!(summary.status_flags_or & STATUS_SOURCE_UNAVAILABLE, 0);
        assert_eq!(
            summary.schema_flags_or & SCHEMA_FAMILY_PRESENCE_DECLARED,
            SCHEMA_FAMILY_PRESENCE_DECLARED
        );
        assert_eq!(
            absent_family_names(summary.family_presence_mask),
            vec![
                "unit_leader_traits",
                "country_leader_roles",
                "advisor_roster",
                "appointed_advisors"
            ]
        );
    }

    /// Undeclared payloads keep their old meaning exactly: every family
    /// observed, and the reserved word still refused when non-zero.
    #[test]
    fn an_undeclared_payload_still_means_every_family() {
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &[]);
        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.family_presence_mask, FAMILY_ALL);
        assert_eq!(summary.schema_flags_or & SCHEMA_FAMILY_PRESENCE_DECLARED, 0);

        let mut tampered = payload.clone();
        tampered[60..64].copy_from_slice(&FAMILY_CENSUS_ONLY.to_le_bytes());
        assert!(decode_payload(0, &tampered)
            .parse_errors
            .iter()
            .any(|error| error.contains("reserved header field is nonzero")));
    }

    /// Every spelling the declaration must refuse, and why each one matters.
    #[test]
    fn the_family_declaration_refuses_every_incoherent_spelling() {
        let character = CharacterLaneRow::Character(CharacterRow {
            country_tag_db_id: 3,
            char_id: 2450,
            name: None,
            token: None,
        });

        // Omitting the census every other family joins against.
        assert!(declared(FAMILY_UNIT_LEADER | FAMILY_ACTIVE_RULER, &[])
            .iter()
            .any(|error| error.contains("omits the character census")));

        // Traits without the unit leaders they attach to.
        assert!(
            declared(FAMILY_CHARACTER | FAMILY_UNIT_LEADER_TRAIT, &[])
                .iter()
                .any(|error| error.contains("without the unit leaders")),
        );

        // Appointed advisors without the roster they are a subset of.
        assert!(
            declared(FAMILY_CHARACTER | FAMILY_APPOINTED_ADVISOR, &[])
                .iter()
                .any(|error| error.contains("without the advisor roster")),
        );

        // A row belonging to a family the header says went unobserved.
        let trait_row = CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
            unit_leader_id: 77,
            ordinal: 0,
            trait_token: "old_guard",
        });
        assert!(
            declared(FAMILY_CENSUS_ONLY, &[character.clone(), trait_row])
                .iter()
                .any(|error| error.contains("did not observe")),
        );

        // A unit leader asserting a trait count while the trait family was
        // never walked: zero is the only honest value there.
        let leader_claiming_traits = CharacterLaneRow::UnitLeader(UnitLeaderRow {
            country_tag_db_id: 3,
            unit_leader_id: 77,
            char_id: 2450,
            leader_kind: UNIT_LEADER_ARMY,
            skill: None,
            experience_raw: None,
            attack: None,
            defense: None,
            planning: None,
            logistics: None,
            maneuvering: None,
            coordination: None,
            token: None,
            name: None,
            trait_count: 2,
        });
        assert!(
            declared(FAMILY_CENSUS_ONLY, &[character, leader_claiming_traits])
                .iter()
                .any(|error| error.contains("although the trait family was not observed")),
        );
    }

    /// A complete snapshot has exactly one spelling, and a declared one has
    /// exactly one status word. Both directions are what bind d47's
    /// expected-partial tolerance to the payload's own declaration.
    #[test]
    fn the_declaration_and_the_status_word_cannot_disagree() {
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &[]);

        // Declared, but every family present: two encodings for "complete".
        let mut both = payload.clone();
        both[56..60].copy_from_slice(
            &(SCHEMA_ROOTS_VALIDATED | SCHEMA_FAMILY_PRESENCE_DECLARED).to_le_bytes(),
        );
        both[60..64].copy_from_slice(&FAMILY_ALL.to_le_bytes());
        both[8..12].copy_from_slice(&STATUS_PARTIAL_DATA.to_le_bytes());
        assert!(decode_payload(0, &both)
            .parse_errors
            .iter()
            .any(|error| error.contains("declares every family present")));

        // Declared a gap, but the status says the hour is complete.
        let mut silent = payload.clone();
        silent[56..60].copy_from_slice(
            &(SCHEMA_ROOTS_VALIDATED | SCHEMA_FAMILY_PRESENCE_DECLARED).to_le_bytes(),
        );
        silent[60..64].copy_from_slice(&FAMILY_CENSUS_ONLY.to_le_bytes());
        assert!(decode_payload(0, &silent)
            .parse_errors
            .iter()
            .any(|error| error.contains("without partial data status")));

        // Partial with no declaration: the shape the tolerance would otherwise
        // absorb without anything naming what was lost.
        let mut bare = payload.clone();
        bare[8..12].copy_from_slice(&STATUS_PARTIAL_DATA.to_le_bytes());
        assert!(decode_payload(0, &bare)
            .parse_errors
            .iter()
            .any(|error| error.contains("no family-presence declaration")));

        // Partial alongside another failure bit needs no declaration: it is not
        // a family gap and the tolerance does not reach it.
        let mut source_failed = payload.clone();
        source_failed[8..12]
            .copy_from_slice(&(STATUS_PARTIAL_DATA | STATUS_SOURCE_UNAVAILABLE).to_le_bytes());
        assert!(decode_payload(0, &source_failed).parse_errors.is_empty());

        // An unassigned schema bit is refused rather than interpreted.
        let mut future = payload;
        future[56..60].copy_from_slice(&(SCHEMA_ROOTS_VALIDATED | 1 << 20).to_le_bytes());
        assert!(decode_payload(0, &future)
            .parse_errors
            .iter()
            .any(|error| error.contains("unassigned bits")));
    }

    /// One row of every family, with every optional field on both sides of its
    /// present/absent switch, encoded through the ordinary four-argument
    /// [`encode_payload`].
    ///
    /// This exists to be hashed, not read: it is the fixture behind
    /// `a_fully_present_payload_is_byte_identical`, and it must stay
    /// deterministic — no map iteration order, no time, no addresses.
    fn byte_identity_fixture_rows() -> Vec<CharacterLaneRow<'static>> {
        vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 3,
                char_id: 2450,
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
            }),
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 3,
                char_id: 2451,
                name: None,
                token: None,
            }),
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 3,
                unit_leader_id: 77,
                char_id: 2450,
                leader_kind: UNIT_LEADER_NAVY,
                skill: Some(4),
                experience_raw: Some(123_456),
                attack: Some(2),
                defense: Some(3),
                planning: None,
                logistics: Some(1),
                maneuvering: Some(5),
                coordination: None,
                token: Some("SOV_iosif_stalin"),
                name: Some("Iosif Stalin"),
                trait_count: 1,
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 77,
                ordinal: 0,
                trait_token: "old_guard",
            }),
            CharacterLaneRow::CountryLeaderRole(CountryLeaderRoleRow {
                country_tag_db_id: 3,
                char_id: 2450,
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
            }),
            CharacterLaneRow::ActiveRuler(ActiveRulerRow {
                country_tag_db_id: 3,
                char_id: 2450,
                ruling_party: Some("communism"),
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
                leader_vec_size: 1,
                ambiguous: false,
            }),
            CharacterLaneRow::AdvisorRoster(AdvisorRow {
                country_tag_db_id: 3,
                char_id: 2451,
                slot: None,
                ledger: Some("political_advisor"),
                idea_token: "SOV_lavrentiy_beria",
            }),
            CharacterLaneRow::AppointedAdvisor(AdvisorRow {
                country_tag_db_id: 3,
                char_id: 2451,
                slot: Some("political_advisor"),
                ledger: Some("high_command"),
                idea_token: "SOV_lavrentiy_beria",
            }),
            CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                country_tag_db_id: 3,
                char_id: 2450,
                role: "navy",
                size: "large",
                gfx: "gfx/leaders/SOV/portrait_sov_iosif_stalin.dds",
                gfx_encoding: PortraitIdentityEncoding::Utf8,
            }),
        ]
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// A d47 payload that observed every family is byte-for-byte what it was
    /// before per-family partial admission existed.
    ///
    /// # Why a literal and not a round-trip
    ///
    /// A round-trip test re-derives the expectation from the same encoder it is
    /// testing, so it passes through any change the encoder makes to itself.
    /// This literal was produced by the encoder as it stood BEFORE the
    /// family-presence declaration was added, and it is the only artifact in
    /// this crate that can still tell you so. If a change to `encode_payload`
    /// moves one byte of a fully-present payload, this is where it is caught —
    /// and a fully-present payload moving is a stream-format break for every
    /// capture already on disk, never a cosmetic diff.
    ///
    /// A DECLARED partial payload is deliberately not pinned here: it is a new
    /// shape, so it has no prior bytes to be identical to.
    const FULLY_PRESENT_PAYLOAD_HEX: &str = "070000000000000000000000090000000a000000a800000002000000010000000100000001000000010000000100000001000000da00000007000000000000000c00496f736966205374616c696e1000534f565f696f7369665f7374616c696e09006f6c645f67756172640900636f6d6d756e69736d1100706f6c69746963616c5f61647669736f721300534f565f6c617672656e7469795f62657269610c00686967685f636f6d6d616e6404006e61767905006c617267652d006766782f6c6561646572732f534f562f706f7274726169745f736f765f696f7369665f7374616c696e2e6464730114000003000000920900000000000001000000011400000300000093090000ffffffffffffffff022a0600030000004d000000920900000400000040e20100010000000000000001000203ff0105ff0200031000004d00000000000000020000000414000003000000920900000000000001000000051c0000030000009209000003000000000000000100000001000000061800000300000093090000ffffffff0400000005000000071800000300000093090000040000000600000005000000081800000300000092090000070000000800000009000000";

    #[test]
    fn a_fully_present_payload_is_byte_identical() {
        let payload = encode_payload(
            7,
            0,
            SCHEMA_ROOTS_VALIDATED
                | SCHEMA_STATIC_NAME_CLOSURE
                | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            &byte_identity_fixture_rows(),
        );
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "the byte-identity fixture must itself be a valid payload: {:?}",
            summary.parse_errors
        );
        assert_eq!(
            to_hex(&payload),
            FULLY_PRESENT_PAYLOAD_HEX,
            "a fully-present d47 payload changed shape; every capture already on \
             disk decodes under the old bytes"
        );
    }
}
