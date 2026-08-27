use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 47;
pub const DOMAIN_NAME: &str = "characters_leaders_cabinet_live";

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
pub const SCHEMA_FAMILY_PRESENCE_DECLARED: u32 = 1 << 3;

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

pub const FAMILY_CHARACTER: u32 = 1 << (ROW_CHARACTER - 1);
pub const FAMILY_UNIT_LEADER: u32 = 1 << (ROW_UNIT_LEADER - 1);
pub const FAMILY_UNIT_LEADER_TRAIT: u32 = 1 << (ROW_UNIT_LEADER_TRAIT - 1);
pub const FAMILY_COUNTRY_LEADER_ROLE: u32 = 1 << (ROW_COUNTRY_LEADER_ROLE - 1);
pub const FAMILY_ACTIVE_RULER: u32 = 1 << (ROW_ACTIVE_RULER - 1);
pub const FAMILY_ADVISOR_ROSTER: u32 = 1 << (ROW_ADVISOR_ROSTER - 1);
pub const FAMILY_APPOINTED_ADVISOR: u32 = 1 << (ROW_APPOINTED_ADVISOR - 1);
pub const FAMILY_CHARACTER_PORTRAIT: u32 = 1 << (ROW_CHARACTER_PORTRAIT - 1);

pub const FAMILY_ALL: u32 = FAMILY_CHARACTER
    | FAMILY_UNIT_LEADER
    | FAMILY_UNIT_LEADER_TRAIT
    | FAMILY_COUNTRY_LEADER_ROLE
    | FAMILY_ACTIVE_RULER
    | FAMILY_ADVISOR_ROSTER
    | FAMILY_APPOINTED_ADVISOR
    | FAMILY_CHARACTER_PORTRAIT;

pub const FAMILY_HEAP_SWEEP_DERIVED: u32 = FAMILY_UNIT_LEADER_TRAIT
    | FAMILY_COUNTRY_LEADER_ROLE
    | FAMILY_ADVISOR_ROSTER
    | FAMILY_APPOINTED_ADVISOR;

pub const FAMILY_CENSUS_ONLY: u32 = FAMILY_ALL & !FAMILY_HEAP_SWEEP_DERIVED;

pub const FAMILY_TRAIT_SWEEP_DERIVED: u32 = FAMILY_UNIT_LEADER_TRAIT;

pub const FAMILY_WALK_WITHOUT_TRAITS: u32 = FAMILY_ALL & !FAMILY_TRAIT_SWEEP_DERIVED;

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
pub const ROW_FLAG_PORTRAIT_GFX_WINDOWS1252: u16 = 1 << 3;

pub const ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT: u16 = ROW_FLAG_PORTRAIT_GFX_WINDOWS1252;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortraitIdentityEncoding {
    #[default]
    Utf8,
    Windows1252,
}

/// The name and its reverse, ONE declaration: they were two matches, so
/// renaming a state in the writer alone compiled and read it back as
/// another (lane TV, Codex round 10).
macro_rules! portraitidentityencoding_names {
    ($( $variant:ident => $name:literal, )+) => {
        impl PortraitIdentityEncoding {
            pub fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            pub fn from_wire_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        const _: () = {
            let names = [$($name,)+];
            let mut i = 0;
            while i < names.len() {
                let mut j = i + 1;
                while j < names.len() {
                    assert!(
                        !compact_contract::diagnostic_channel::const_str_eq(
                            names[i],
                            names[j],
                        ),
                        "two values share a wire name",
                    );
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

portraitidentityencoding_names! {
    Utf8 => "utf8",
    Windows1252 => "windows1252",
}

const CHARACTER_ROW_LEN: u8 = 20;
const UNIT_LEADER_ROW_LEN: u8 = 42;
const TRAIT_ROW_LEN: u8 = 16;
const COUNTRY_LEADER_ROLE_ROW_LEN: u8 = 20;
const ACTIVE_RULER_ROW_LEN: u8 = 28;
const ADVISOR_ROW_LEN: u8 = 24;
const CHARACTER_PORTRAIT_ROW_LEN: u8 = 24;
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
        gfx_encoding: PortraitIdentityEncoding,
    },
}

// =====================================================================
// THE HASH THE TWO INTERNAL INDEXES PROBE WITH (lane SB, 2026-08-24)
//
// It decides a PROBE START and nothing else: every candidate a probe finds
// is confirmed by a full key comparison, so a collision costs one extra
// probe and can never return a wrong id or a wrong duplicate verdict. That
// is why a non-cryptographic word hash is sound here where the standard
// library's SipHash-1-3 was only expensive.
// =====================================================================

const D47_HASH_SEED: u64 = 0x517c_c1b7_2722_0a95;

#[inline]
fn d47_hash_word(hash: u64, word: u64) -> u64 {
    (hash.rotate_left(5) ^ word).wrapping_mul(D47_HASH_SEED)
}

/// Avalanche the accumulator, so the LOW bits a probe mask reads depend on
/// every input bit.
///
/// `d47_hash_word` ends in a multiply, and multiplication only carries
/// upward. It also folds in the rotated prior accumulator, so the low bits
/// are not a function of the last word alone in general (Codex round 18,
/// finding 4). What holds is narrower and is what actually bites here: for
/// inputs sharing a FIXED PREFIX, the accumulator entering the last step is
/// the same, so low-bit locality in that last word survives to the low bits
/// of the result -- and structured ids differing only in a low-order tail are
/// exactly that shape, so without this finalizer they share a probe start. This is the
/// standard 64-bit finalizer (three xor-shift/multiply rounds); it is a
/// bijection, so it introduces no collisions of its own.
///
/// It is defence against ACCIDENTAL clustering. It is not a defence against a
/// chosen-key attack, because the hash is unkeyed and therefore solvable
/// offline; what bounds that case is [`D47_PROBE_LIMIT`] (Codex round 2,
/// finding 1).
#[inline]
fn d47_hash_finish(hash: u64) -> u64 {
    let mut hash = hash;
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    hash ^= hash >> 33;
    hash
}

/// How long ONE probe run may get before a table stops trusting its hash.
///
/// A DEFENSIVE threshold, not a measured one. `decode_payload` is the
/// canonical decoder for payloads this process did not necessarily produce, so
/// it has to assume keys chosen to collide are possible, and an unkeyed hash
/// cannot prevent that. Past this many slots the table stops probing and
/// finishes in an ORDERED structure, whose operations are a deterministic
/// `O(log n)` no key set can push off. For the duplicate detectors that
/// structure is a `BTreeSet`, which is exactly the spelling this lane replaced.
/// For the string table it is a `BTreeMap<Box<[u8]>, usize>`, which is NOT the
/// `HashMap<Rc<str>, u32>` this lane replaced and is not shown equivalent to
/// it -- ordered byte-string compares may cost more than a hash probe, and
/// nothing here measures that (Codex rounds 13 and 14).
///
/// **How often it fires on a real payload is NOT known** (Codex round 13,
/// finding 1; an earlier version of this comment asserted "a couple of dozen
/// slots" and "every real payload", neither of which anything measured). The
/// keys d47 actually decodes ARE in the payloads -- rows carry their keys --
/// but they are not in the manifest aggregates this lane measured from (Codex
/// round 18, finding 2), so nothing here establishes a real-payload probe-run
/// distribution. What the tables do give
/// is the half-full invariant they maintain, and what the offline tests give
/// is that the fallback works when a run does pass this limit.
///
/// Note the loop is `0..=D47_PROBE_LIMIT`, so a run examines up to 129 slots
/// before degrading, not 128 (Codex round 13, finding 2).
///
/// Codex round 2, finding 1.
const D47_PROBE_LIMIT: usize = 128;

/// Word-at-a-time hash of a byte string. The length is folded in first so
/// that the zero padding of the final partial word cannot make `"ab"` and
/// `"ab\0"` share a start (they would still be told apart by the byte
/// comparison; this only keeps the probe short).
#[inline]
fn d47_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = d47_hash_word(D47_HASH_SEED, bytes.len() as u64);
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let word = u64::from_le_bytes(chunk.try_into().expect("chunks_exact(8) yields 8 bytes"));
        hash = d47_hash_word(hash, word);
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut tail = [0u8; 8];
        tail[..rest.len()].copy_from_slice(rest);
        hash = d47_hash_word(hash, u64::from_le_bytes(tail));
    }
    d47_hash_finish(hash)
}

// =====================================================================
// THE STRING TABLE (lane SB, 2026-08-24)
//
// WHAT CHANGED, AND WHY NO PAYLOAD BYTE DID. This was `Vec<Rc<str>>` beside
// a `HashMap<Rc<str>, u32>`: one SipHash-1-3 of the value plus one hash
// probe per interned field, and one `Rc<str>` heap allocation per distinct
// value. `captures/rc_vanilla` runs **110,099 interns per frame**
// (`runs/sb_characters_worker/sb_attribution.txt`), the highest-count
// operation in d47's post-bracket half -- which is a COUNT, not a
// measurement of where the time goes: nothing times that half's parts, and
// `lw_d47_enc_us` exists to. Of those calls, 59,595 are on fields that cannot
// be `None`; the rest are `Option`s. How many DISTINCT values the table ends
// up holding is not on that capture's wire, so the per-allocation half of the
// old cost is unquantified in size and in sign, and no claim is made that
// either half dominates the other.
//
// It is now one byte arena in id order plus an end offset per value, indexed
// by an open-addressed table of `id + 1`. IDS ARE STILL ASSIGNED IN
// FIRST-APPEARANCE ORDER and the values are still emitted in id order, so
// for the identical sequence of `intern` calls the ids, `encoded_len` and
// `encode_into` bytes are identical: the hash chooses only where a value is
// LOOKED UP, never what id it gets or where it lands. Both assertions still
// run on exactly the miss path they ran on before, so a repeat of an
// already-interned value still never re-checks them.
//
// kills: assigning `id` before the miss checks, interning the value into the
// arena on the HIT path, or emitting values in probe order instead of id
// order would each change the payload; `the_string_table_matches_the_map_and_rc_spelling`
// drives the shipped table and a verbatim transcription of the old one over
// one call sequence and compares ids and encoded bytes.
// =====================================================================

// =====================================================================
// THE DECODER'S DUPLICATE DETECTORS (lane SB, 2026-08-24)
//
// One `insert` per decoded row, answering "has this exact key been offered
// before?". `BTreeSet` answered it with an ordered tree; nothing in the
// decoder ever iterates these sets, so the ordering was paid for and never
// used. `SeenSet` is the same question over the same keys with the same
// `true == newly inserted` convention, so the parse errors it drives are
// the same strings emitted at the same rows.
// =====================================================================

/// How many bytes of row region a header's declared populations require.
///
/// Every row kind is a fixed width, so this is exact for a well-formed
/// header. It exists so the decoder can ask ONE question before reserving
/// anything: do the counts this header declares fit in the region it
/// declares them in? Saturating throughout, so a header declaring four
/// billion of everything answers "no" rather than wrapping to "yes" (Codex
/// round 1, finding 3).
fn domain47_declared_row_bytes(
    characters: u64,
    unit_leaders: u64,
    traits: u64,
    country_leader_roles: u64,
    active_rulers: u64,
    advisors: u64,
    portraits: u64,
) -> u64 {
    [
        (characters, CHARACTER_ROW_LEN),
        (unit_leaders, UNIT_LEADER_ROW_LEN),
        (traits, TRAIT_ROW_LEN),
        (country_leader_roles, COUNTRY_LEADER_ROLE_ROW_LEN),
        (active_rulers, ACTIVE_RULER_ROW_LEN),
        (advisors, ADVISOR_ROW_LEN),
        (portraits, CHARACTER_PORTRAIT_ROW_LEN),
    ]
    .into_iter()
    .fold(0u64, |total, (count, len)| {
        total.saturating_add(count.saturating_mul(u64::from(len)))
    })
}

/// Whether a header's declared populations may be reserved for.
///
/// Six duplicate detectors size themselves from six header fields. Applied
/// independently, each is bounded by the whole row region, so a header can
/// declare that region six times over and multiply the reservation by six
/// (Codex round 1, finding 3). This is the one question that stops it: the
/// row widths are fixed, so a header whose declared rows cannot fit in the
/// region it declares them in is not one to size anything from. A well-formed
/// payload always passes -- the counts are exact, so the declared bytes ARE
/// the region -- and a header that fails reserves nothing and lets the sets
/// grow as they fill.
///
/// The bound this buys, stated so it does not have to be re-derived. Under
/// the test each family satisfies `count * row_len <= region`, and the
/// narrowest row is 16 bytes, so the six sets together hold under
/// `region / 16` keys; the widest key is 20 bytes, putting the KEYS under
/// `region * 20 / 16`. The probe tables are `2 * count` slots ROUNDED UP TO A
/// POWER OF TWO, so up to just under FOUR `usize` slots per key (33 rows
/// reserve 128 slots), which is under `region * 32 / 16` bytes -- not under
/// `region`, as an earlier version of this comment said (Codex round 8,
/// finding 4). Both terms are a small constant times the row region the
/// caller already holds; what the gate rules out is the reservation being a
/// MULTIPLE of that region, once per header field.
fn domain47_reservations_are_trustworthy(declared_row_bytes: u64, row_region_bytes: usize) -> bool {
    declared_row_bytes <= row_region_bytes as u64
}

/// A fixed-width key a [`SeenSet`] can index. `seen_hash` picks a probe
/// start only; equality decides membership.
trait SeenKey: Copy + Ord {
    fn seen_hash(&self) -> u64;
}

impl SeenKey for u32 {
    #[inline]
    fn seen_hash(&self) -> u64 {
        d47_hash_finish(d47_hash_word(D47_HASH_SEED, u64::from(*self)))
    }
}

impl<const N: usize> SeenKey for [u32; N] {
    #[inline]
    fn seen_hash(&self) -> u64 {
        let mut hash = d47_hash_word(D47_HASH_SEED, N as u64);
        for word in self {
            hash = d47_hash_word(hash, u64::from(*word));
        }
        d47_hash_finish(hash)
    }
}

/// What one bounded probe found. `T` is whatever the caller wants back with
/// a hit: the arena index for the string table, the key index for a
/// duplicate detector.
enum SeenProbeOf<T> {
    /// A hit, and where it sits.
    Present(T),
    /// A miss; the probe stopped on an empty slot, which `place` may fill.
    Absent,
    /// The run passed [`D47_PROBE_LIMIT`] without resolving either way.
    TooLong,
}

struct SeenSet<K: SeenKey> {
    keys: Vec<K>,
    /// Open-addressed probe table holding `index + 1` into `keys`; 0 is an
    /// empty slot. Length is a power of two, or 0 while `keys` is empty.
    ///
    /// `usize`, not `u32` (Codex round 1, finding 2): a narrower slot would
    /// need a "what happens past four billion keys" arm, and the only honest
    /// arms are a wrong answer or a panic. `Vec::len()` never exceeds
    /// `isize::MAX`, so `index + 1` always fits and no such arm exists.
    slots: Vec<usize>,
    /// Set once a probe passed [`D47_PROBE_LIMIT`]; `keys` and `slots` are
    /// emptied into it and every later question is asked of it instead
    /// (Codex round 2, finding 1). It answers exactly the same questions --
    /// same keys, same `true == newly inserted` convention -- with a
    /// guaranteed `O(log n)` per operation.
    ordered: Option<BTreeSet<K>>,
}

impl<K: SeenKey> SeenSet<K> {
    /// `declared` is the header's own count for this family, which is
    /// attacker-controlled in the general case; `ceiling` is what the bytes
    /// in hand could actually hold. The reservation is the smaller of the
    /// two, so a header claiming four billion rows in a 64-byte payload
    /// reserves the CEILING, not the declaration -- a handful of keys rather
    /// than gigabytes.
    ///
    /// That is the first of TWO protections and only the weaker one (Codex
    /// round 18, finding 1; an earlier version of this comment said the
    /// reservation was "nothing", which it is not). Six detectors each
    /// reserving the whole row region still multiplies. What takes an
    /// infeasible declaration to zero is the JOINT gate
    /// `domain47_reservations_are_trustworthy`, which every production caller
    /// passes its declared count through before it reaches this function.
    fn with_bounded_capacity(declared: u64, ceiling: usize) -> Self {
        let want = usize::try_from(declared).unwrap_or(usize::MAX).min(ceiling);
        let mut set = SeenSet {
            keys: Vec::with_capacity(want),
            slots: Vec::new(),
            ordered: None,
        };
        // An unrepresentable probe size leaves the table empty; `reserve_one`
        // then sizes it on the first insert, so there is no arithmetic that
        // can wrap here.
        if let Some(slots) = want.saturating_mul(2).checked_next_power_of_two() {
            if want != 0 {
                set.slots.resize(slots, 0);
            }
        }
        set
    }

    fn place(&mut self, index: usize, hash: u64) {
        let mask = self.slots.len() - 1;
        let mut probe = (hash as usize) & mask;
        while self.slots[probe] != 0 {
            probe = (probe + 1) & mask;
        }
        self.slots[probe] = index + 1;
    }

    /// Keep the probe table at most half full BEFORE the next insert, so a
    /// linear probe always terminates on an empty slot.
    fn reserve_one(&mut self) {
        if self.slots.len() >= (self.keys.len() + 1) * 2 {
            return;
        }
        let want = (self.keys.len() + 1)
            .saturating_mul(4)
            .next_power_of_two()
            .max(64);
        self.slots.clear();
        self.slots.resize(want, 0);
        for index in 0..self.keys.len() {
            let hash = self.keys[index].seen_hash();
            self.place(index, hash);
        }
    }

    /// A probe bounded by [`D47_PROBE_LIMIT`].
    fn probe(&self, key: K, hash: u64) -> SeenProbeOf<usize> {
        if self.slots.is_empty() {
            return SeenProbeOf::Absent;
        }
        let mask = self.slots.len() - 1;
        let mut probe = (hash as usize) & mask;
        for _ in 0..=D47_PROBE_LIMIT {
            let slot = self.slots[probe];
            if slot == 0 {
                return SeenProbeOf::Absent;
            }
            if self.keys[slot - 1] == key {
                return SeenProbeOf::Present(slot - 1);
            }
            probe = (probe + 1) & mask;
        }
        SeenProbeOf::TooLong
    }

    /// Move every key into an ordered set and stop probing for good.
    fn degrade(&mut self) {
        let mut ordered = BTreeSet::new();
        for key in self.keys.drain(..) {
            ordered.insert(key);
        }
        self.keys = Vec::new();
        self.slots = Vec::new();
        self.ordered = Some(ordered);
    }

    /// `true` when the key was NOT present, matching `BTreeSet::insert`.
    fn insert(&mut self, key: K) -> bool {
        if let Some(ordered) = self.ordered.as_mut() {
            return ordered.insert(key);
        }
        let hash = key.seen_hash();
        match self.probe(key, hash) {
            SeenProbeOf::Present(_) => false,
            SeenProbeOf::Absent => {
                self.reserve_one();
                let index = self.keys.len();
                self.keys.push(key);
                self.place(index, hash);
                true
            }
            SeenProbeOf::TooLong => {
                self.degrade();
                self.ordered
                    .as_mut()
                    .expect("degrade sets the ordered index")
                    .insert(key)
            }
        }
    }

    /// WHY THIS TERMINATES, AND WHY IT CANNOT MISS A KEY. It terminates
    /// because it IS [`SeenSet::probe`], which examines at most
    /// `D47_PROBE_LIMIT + 1` slots and then stops -- nothing here depends on
    /// how long the surrounding cluster is (Codex round 14, finding 1; an
    /// earlier version of this comment claimed no cluster can exceed the
    /// limit, which is false: clusters merge, and keys starting at different
    /// points inside one can carry it past that length).
    ///
    /// It cannot miss a present key for a different reason. While `ordered`
    /// is `None`, every key `insert` placed sits within the first
    /// `D47_PROBE_LIMIT + 1` slots examined from ITS OWN hash start -- an
    /// insert that had to look further returned `TooLong` and degraded the
    /// table instead of placing anything. This walk examines exactly that
    /// same window from the same start, so the window it searches is the
    /// window the key can be in. It is spelled as a plain probe because it
    /// has no absent/too-long distinction to make.
    fn contains(&self, key: K) -> bool {
        if let Some(ordered) = self.ordered.as_ref() {
            return ordered.contains(&key);
        }
        matches!(self.probe(key, key.seen_hash()), SeenProbeOf::Present(_))
    }
}

struct StringTable {
    /// Every interned value, back to back, in id order.
    arena: Vec<u8>,
    /// `ends[id]` is the end offset of value `id` inside `arena`; its start
    /// is `ends[id - 1]`, or 0 for id 0. Monotonic by construction.
    ends: Vec<usize>,
    /// Open-addressed probe table holding `index + 1` into `ends`; 0 is an
    /// empty slot. Length is a power of two, or 0 before the first insert.
    ///
    /// It indexes the ARENA, not the wire id (Codex round 1, finding 1). The
    /// wire id is the arena index clamped to `u32::MAX`, exactly as the old
    /// map spelled it; keeping the two separate is what makes a value past
    /// the clamp still FINDABLE, which is what the old map (keyed by the
    /// string, not by the id) did. `usize` for the same reason `SeenSet`
    /// uses it: `Vec::len()` never exceeds `isize::MAX`, so `index + 1`
    /// always fits and there is no unrepresentable case to arm.
    slots: Vec<usize>,
    /// Set once a probe passed [`D47_PROBE_LIMIT`]; the arena stays (it IS
    /// the payload's string block) and this becomes the index into it. Same
    /// answers, `O(log n)` per lookup, one allocation per distinct value --
    /// which is what the `HashMap<Rc<str>, u32>` this replaced did on EVERY
    /// value (Codex round 2, finding 1).
    ordered: Option<BTreeMap<Box<[u8]>, usize>>,
    /// The largest arena index a wire id can name. Production is
    /// `u32::MAX as usize`, the clamp the old spelling applied; tests lower
    /// it to reach the clamp, which four billion distinct strings otherwise
    /// make untestable (Codex round 2, finding 3).
    wire_id_ceiling: usize,
}

impl Default for StringTable {
    fn default() -> Self {
        StringTable {
            arena: Vec::new(),
            ends: Vec::new(),
            slots: Vec::new(),
            ordered: None,
            wire_id_ceiling: u32::MAX as usize,
        }
    }
}

#[cfg(test)]
impl StringTable {
    /// A table whose wire ids clamp at `ceiling` instead of `u32::MAX`.
    ///
    /// The clamp is unreachable in production -- it needs four billion
    /// DISTINCT strings in one frame -- so without this seam the arm that
    /// keeps a clamped entry FINDABLE (Codex round 1, finding 1) could only
    /// be argued, not tested (Codex round 2, finding 3). Nothing but tests
    /// constructs this.
    fn with_wire_id_ceiling(ceiling: usize) -> Self {
        StringTable {
            wire_id_ceiling: ceiling,
            ..StringTable::default()
        }
    }
}

impl StringTable {
    fn len(&self) -> usize {
        self.ends.len()
    }

    fn value(&self, id: usize) -> &[u8] {
        let start = if id == 0 { 0 } else { self.ends[id - 1] };
        &self.arena[start..self.ends[id]]
    }

    /// The ARENA INDEX of `bytes`, if it has been interned, from a probe
    /// bounded by [`D47_PROBE_LIMIT`].
    fn lookup(&self, bytes: &[u8], hash: u64) -> SeenProbeOf<usize> {
        if self.slots.is_empty() {
            return SeenProbeOf::Absent;
        }
        let mask = self.slots.len() - 1;
        let mut probe = (hash as usize) & mask;
        for _ in 0..=D47_PROBE_LIMIT {
            let slot = self.slots[probe];
            if slot == 0 {
                return SeenProbeOf::Absent;
            }
            let index = slot - 1;
            if self.value(index) == bytes {
                return SeenProbeOf::Present(index);
            }
            probe = (probe + 1) & mask;
        }
        SeenProbeOf::TooLong
    }

    /// Move the index out of the probe table and into an ordered map. The
    /// ARENA is untouched -- it is the payload's string block, in id order --
    /// so ids and bytes are unaffected; only how a repeat is found changes.
    fn degrade(&mut self) {
        let mut ordered = BTreeMap::new();
        for index in 0..self.ends.len() {
            ordered.insert(Box::from(self.value(index)), index);
        }
        self.slots = Vec::new();
        self.ordered = Some(ordered);
    }

    fn place(&mut self, index: usize, hash: u64) {
        let mask = self.slots.len() - 1;
        let mut probe = (hash as usize) & mask;
        while self.slots[probe] != 0 {
            probe = (probe + 1) & mask;
        }
        self.slots[probe] = index + 1;
    }

    /// Keep the probe table at most half full BEFORE the next insert. A
    /// linear probe over a half-empty power-of-two table is expected O(1)
    /// and cannot run off the end, because an empty slot always exists.
    fn reserve_one(&mut self) {
        if self.slots.len() >= (self.ends.len() + 1) * 2 {
            return;
        }
        let want = (self.ends.len() + 1)
            .saturating_mul(4)
            .next_power_of_two()
            .max(64);
        self.slots.clear();
        self.slots.resize(want, 0);
        for index in 0..self.ends.len() {
            let hash = d47_hash_bytes(self.value(index));
            self.place(index, hash);
        }
    }

    /// The wire id of an arena index: the old spelling's
    /// `self.values.len().min(u32::MAX as usize) as u32`, applied at the same
    /// place it was applied before -- to the position in the table.
    fn wire_id(&self, index: usize) -> u32 {
        index.min(self.wire_id_ceiling) as u32
    }

    /// The arena index of an already-interned value, or `None`. Degrades the
    /// table when the probe run passes the limit.
    fn find_existing(&mut self, bytes: &[u8], hash: u64) -> Option<usize> {
        if let Some(ordered) = self.ordered.as_ref() {
            return ordered.get(bytes).copied();
        }
        match self.lookup(bytes, hash) {
            SeenProbeOf::Present(index) => Some(index),
            SeenProbeOf::Absent => None,
            SeenProbeOf::TooLong => {
                self.degrade();
                self.ordered
                    .as_ref()
                    .expect("degrade sets the ordered index")
                    .get(bytes)
                    .copied()
            }
        }
    }

    fn intern(&mut self, value: Option<&str>) -> u32 {
        let Some(value) = value else {
            return STRING_NONE;
        };
        let bytes = value.as_bytes();
        let hash = d47_hash_bytes(bytes);
        if let Some(index) = self.find_existing(bytes, hash) {
            return self.wire_id(index);
        }
        assert!(
            value.len() <= MAX_STRING_BYTES,
            "character lane string exceeds max length"
        );
        assert!(
            is_domain47_string_value(value),
            "character lane string must be valid non-control UTF-8"
        );
        let index = self.ends.len();
        if self.ordered.is_none() {
            self.reserve_one();
        }
        self.arena.extend_from_slice(bytes);
        self.ends.push(self.arena.len());
        // EVERY entry is indexed, including any past the wire-id clamp. The
        // old map was keyed by the STRING, so a repeat of a clamped-id value
        // found the entry it had already made and returned the clamped id
        // again; appending a second copy instead would change `string_count`,
        // `string_bytes` and the encoded block (Codex round 1, finding 1).
        match self.ordered.as_mut() {
            Some(ordered) => {
                ordered.insert(Box::from(bytes), index);
            }
            None => self.place(index, hash),
        }
        self.wire_id(index)
    }

    fn encoded_len(&self) -> usize {
        // Identical to the old `sum(2 + value.len())`: two length bytes per
        // value plus every value byte, and the arena holds exactly those.
        self.ends.len() * 2 + self.arena.len()
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        let mut start = 0usize;
        for &end in &self.ends {
            let value = &self.arena[start..end];
            out.extend_from_slice(&(value.len() as u16).to_le_bytes());
            out.extend_from_slice(value);
            start = end;
        }
    }
}

/// True when `value` contains no Unicode control character.
///
/// EXACTLY `!value.chars().any(char::is_control)`, decided on bytes (lane
/// SB). `&str` is valid UTF-8, and the control scalars are U+0000..=U+001F,
/// U+007F..=U+009F and nothing else. In valid UTF-8:
///
///  * U+0000..=U+007F is one byte with the same value, so `< 0x20` or
///    `== 0x7F` names U+0000..=U+001F and U+007F and cannot name anything
///    else — every continuation byte is 0x80..=0xBF and every lead byte is
///    0xC2..=0xF4, so neither can fall in that range;
///  * U+0080..=U+009F is exactly the two-byte sequences `0xC2` followed by
///    0x80..=0x9F (U+00A0..=U+00BF is `0xC2` followed by 0xA0..=0xBF, and
///    every other lead byte starts at U+00C0 or above);
///  * no scalar above U+009F is a control character.
///
/// kills: dropping the `0xC2` arm would admit U+0085 NEXT LINE, which
/// `char::is_control` refuses; widening it to any 0x80..=0xBF second byte
/// would refuse U+00A0..=U+00BF, which `char::is_control` admits.
/// `the_byte_scan_agrees_with_char_is_control_on_every_scalar` walks every
/// Unicode scalar value against the `chars()` spelling.
fn is_domain47_string_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte < 0x20 || byte == 0x7F {
            return false;
        }
        if byte == 0xC2 {
            if let Some(&next) = bytes.get(index + 1) {
                if (0x80..=0x9F).contains(&next) {
                    return false;
                }
            }
        }
        index += 1;
    }
    true
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

pub fn encode_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    rows: &[CharacterLaneRow<'_>],
) -> Vec<u8> {
    encode_payload_with_families(
        elapsed_wall_ms,
        status_flags,
        schema_flags,
        FAMILY_ALL,
        rows,
    )
}

/// The SLICE entry point, unchanged. THREE callers outside this file hold
/// their rows in a `Vec` or a literal and pass a borrow -- one in
/// `lanes/compact_replay_verification_gates` and two in
/// `recorder_host/src/world_fold/tests.rs` -- plus this file's own tests, and
/// lane SB's iterator form must not cost them a rewrite (Codex round 3,
/// finding 1). An earlier version of this comment counted this file's tests
/// among the outside callers and said four (Codex round 16, finding 3).
pub fn encode_payload_with_families(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    family_presence_mask: u32,
    rows: &[CharacterLaneRow<'_>],
) -> Vec<u8> {
    encode_payload_with_families_iter(
        elapsed_wall_ms,
        status_flags,
        schema_flags,
        family_presence_mask,
        rows.iter().cloned(),
    )
}

/// LANE SB (2026-08-24): the same encode over an ITERATOR of rows.
///
/// The production caller (`LaneFrame::compact_row_iter_over`) used to
/// materialise 44,454 `CharacterLaneRow` values per frame into one ~3.9 MiB
/// `Vec` for this function to walk once and drop
/// (`runs/sb_characters_worker/MEASUREMENT.md`). The rows are consumed in
/// exactly the order the iterator yields them, which for that caller is the
/// order the `Vec` held, so the payload is unchanged; the capacity
/// reservation reads the iterator's lower size bound, which for a chain of
/// slice maps is the same exact count `rows.len()` gave.
///
/// The slice entry point above delegates here, so there is ONE encoder.
pub fn encode_payload_with_families_iter<'a, I>(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    family_presence_mask: u32,
    rows: I,
) -> Vec<u8>
where
    I: IntoIterator<Item = CharacterLaneRow<'a>>,
{
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

fn encode_payload_inner<'a, I>(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    reserved_or_families: u32,
    rows: I,
) -> Vec<u8>
where
    I: IntoIterator<Item = CharacterLaneRow<'a>>,
{
    let rows = rows.into_iter();
    let mut strings = StringTable::default();
    // The iterator's LOWER bound, which for the chain of slice maps the
    // production caller builds and for `slice.iter().cloned()` is the exact
    // row count -- the same number `rows.len()` gave before lane SB. A looser
    // iterator would simply grow the buffer.
    let mut row_bytes = Vec::with_capacity(rows.size_hint().0.saturating_mul(MAX_ROW_LEN as usize));
    let mut counts = RowCounts::default();

    for row in rows {
        counts.total = counts.total.saturating_add(1);
        match &row {
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
    push_u32(&mut out, strings.len().min(u32::MAX as usize) as u32);
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
        parse_errors.push(format!(
            "offset {offset}: character lane observes appointed advisors without the advisor roster \
             they are a subset of"
        ));
        return FAMILY_ALL;
    }
    if (reserved_or_families & FAMILY_APPOINTED_ADVISOR) != 0
        && (schema_flags & SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS) == 0
    {
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
    // THE COUNT IS HEADER-CONTROLLED, SO THE RESERVATION IS DERIVED FROM BYTES
    // IN HAND: every entry costs at least its 2-byte length prefix inside the
    // walked region, so this is a tight ceiling on how many entries could
    // exist. Reserving the declared u32 directly let a 64-byte payload demand
    // ~64 GiB before the first truncation error was recorded (Codex review
    // round 11, finding 1).
    let mut strings =
        Vec::with_capacity(string_count.min(payload.len().saturating_sub(HEADER_LEN) / 2));
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
    // THE SIX DUPLICATE DETECTORS (lane SB, 2026-08-24). These were
    // `BTreeSet`s. The counts, with no claim about their rank inside this
    // function, which nothing times (Codex round 10, finding 5): rc_vanilla
    // decodes 44,454 rows per frame and 41,141 of them perform exactly one
    // ordered-tree insert (every family but the traits, which are reconciled
    // by the two maps below). [`SeenSet`] answers the same question — "was
    // this exact key offered before?" — with the same `true == newly
    // inserted` convention, and the rows are still walked in payload order,
    // so the parse errors are the same strings in the same order.
    //
    // THE ORDERED MAPS BELOW ARE DELIBERATELY LEFT ORDERED: unlike the sets
    // they are ITERATED at the end of this function, and that iteration's
    // order is the order of the trait-reconciliation parse errors. They are
    // also two orders of magnitude smaller (one entry per unit leader, not
    // one per row), so they are not what this function spends.
    //
    // The capacities are derived from BYTES IN HAND, never from the
    // header's declared counts alone — the same guard the string-table
    // reservation above carries (Codex round 11 on lane QL): every row costs
    // at least its four header bytes, so `rows / 4` is a tight ceiling on
    // how many rows the walked region can hold.
    //
    // THAT CEILING ALONE IS NOT ENOUGH, because six sets apply it
    // independently and a header can declare the whole region SIX TIMES OVER
    // (Codex round 1, finding 3). So the reservations are also gated on the
    // declared populations being JOINTLY feasible: each family's rows are a
    // known fixed width, and a header whose declared rows cannot fit in the
    // region it declares them in is not a header any reservation should
    // trust. On every payload this recorder produces the counts are exact,
    // so production reserves exactly once and rehashes nothing; a header that
    // fails the test reserves nothing and the sets grow as they fill.
    let row_region_bytes = rows_end.saturating_sub(cursor);
    let row_ceiling = row_region_bytes / 4;
    // Portraits are the one family the header does not count on its own, so
    // their declared population is the total less every counted family. On a
    // consistent header that is exactly the portrait count; on an
    // inconsistent one the joint test below refuses the whole declaration.
    let expected_portraits = expected_total
        .saturating_sub(expected_characters)
        .saturating_sub(expected_unit_leaders)
        .saturating_sub(expected_country_leader_roles)
        .saturating_sub(expected_active_rulers)
        .saturating_sub(expected_advisor_roster)
        .saturating_sub(expected_appointed_advisors)
        .saturating_sub(expected_traits);
    let advisor_rows_declared = expected_advisor_roster.saturating_add(expected_appointed_advisors);
    let declarations_fit = domain47_reservations_are_trustworthy(
        domain47_declared_row_bytes(
            expected_characters,
            expected_unit_leaders,
            expected_traits,
            expected_country_leader_roles,
            expected_active_rulers,
            advisor_rows_declared,
            expected_portraits,
        ),
        row_region_bytes,
    );
    let trusted = |declared: u64| if declarations_fit { declared } else { 0 };
    let mut seen_characters =
        SeenSet::<u32>::with_bounded_capacity(trusted(expected_characters), row_ceiling);
    let mut seen_unit_leaders =
        SeenSet::<u32>::with_bounded_capacity(trusted(expected_unit_leaders), row_ceiling);
    let mut seen_country_roles =
        SeenSet::<u32>::with_bounded_capacity(trusted(expected_country_leader_roles), row_ceiling);
    let mut seen_active_rulers =
        SeenSet::<u32>::with_bounded_capacity(trusted(expected_active_rulers), row_ceiling);
    let mut seen_advisor_rows =
        SeenSet::<[u32; 4]>::with_bounded_capacity(trusted(advisor_rows_declared), row_ceiling);
    let mut seen_portrait_rows =
        SeenSet::<[u32; 5]>::with_bounded_capacity(trusted(expected_portraits), row_ceiling);
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
        // A DECLARED LENGTH OTHER THAN ITS KIND'S CONSTANT IS AN ERROR, and the
        // row's fixed-offset fields are NOT read: the extent check above only
        // proves `row_len` bytes exist, so reading a 42-byte unit leader out of
        // a 4-byte declaration would slice past the payload and panic instead
        // of producing the parse error every malformed record must produce.
        // The row is skipped (its bytes still consumed) with the error named.
        let declared_want = match kind {
            ROW_CHARACTER => Some(CHARACTER_ROW_LEN),
            ROW_UNIT_LEADER => Some(UNIT_LEADER_ROW_LEN),
            ROW_UNIT_LEADER_TRAIT => Some(TRAIT_ROW_LEN),
            ROW_COUNTRY_LEADER_ROLE => Some(COUNTRY_LEADER_ROLE_ROW_LEN),
            ROW_ACTIVE_RULER => Some(ACTIVE_RULER_ROW_LEN),
            ROW_ADVISOR_ROSTER | ROW_APPOINTED_ADVISOR => Some(ADVISOR_ROW_LEN),
            ROW_CHARACTER_PORTRAIT => Some(CHARACTER_PORTRAIT_ROW_LEN),
            _ => None,
        };
        if let Some(want) = declared_want {
            if row_len != want as usize {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} kind {kind} length {row_len} != {want}"
                ));
                cursor += row_len;
                row_index += 1;
                continue;
            }
        }
        match kind {
            ROW_CHARACTER => {
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
                if !seen_advisor_rows.insert([u32::from(kind), country, char_id, idea_sid]) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates advisor row"
                    ));
                }
            }
            ROW_CHARACTER_PORTRAIT => {
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let role_sid = read_u32(payload, cursor + 12);
                let size_sid = read_u32(payload, cursor + 16);
                let gfx_sid = read_u32(payload, cursor + 20);
                summary.character_portrait_rows = summary.character_portrait_rows.saturating_add(1);
                let unassigned = flags & !ROW_FLAGS_ALL_ASSIGNED_CHARACTER_PORTRAIT;
                if unassigned != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} character portrait carries unassigned flag bits {unassigned:#06x}"
                    ));
                }
                if (flags & ROW_FLAG_PORTRAIT_GFX_WINDOWS1252) != 0 {
                    summary.character_portrait_windows1252_rows = summary
                        .character_portrait_windows1252_rows
                        .saturating_add(1);
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
                if !seen_portrait_rows.insert([country, char_id, role_sid, size_sid, gfx_sid]) {
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

    for (family, name, observed) in [
        (FAMILY_CHARACTER, "character", observed_characters),
        (FAMILY_UNIT_LEADER, "unit leader", observed_unit_leaders),
        (
            FAMILY_UNIT_LEADER_TRAIT,
            "unit leader trait",
            observed_traits,
        ),
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
        if !seen_unit_leaders.contains(*unit_id) {
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
    // Same derived reservation as decode_payload: the declared count is
    // header-controlled and must not size an allocation by itself.
    let mut strings =
        Vec::with_capacity(string_count.min(payload.len().saturating_sub(HEADER_LEN) / 2));
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn string_ids_follow_first_use_order_and_repeats_reuse_them() {
        let mut table = StringTable::default();
        assert_eq!(table.intern(Some("zulu")), 0);
        assert_eq!(table.intern(Some("alpha")), 1);
        assert_eq!(table.intern(Some("mike")), 2);
        assert_eq!(table.intern(Some("alpha")), 1, "repeat must reuse its id");
        assert_eq!(table.intern(Some("zulu")), 0, "repeat must reuse its id");
        assert_eq!(table.intern(None), STRING_NONE);
        assert_eq!(
            (0..table.len())
                .map(|id| std::str::from_utf8(table.value(id)).expect("interned UTF-8"))
                .collect::<Vec<_>>(),
            vec!["zulu", "alpha", "mike"],
            "the encoded block must be in first-use order"
        );

        let mut out = Vec::new();
        table.encode_into(&mut out);
        assert_eq!(out.len(), table.encoded_len());
        assert_eq!(
            out,
            b"\x04\x00zulu\x05\x00alpha\x04\x00mike".to_vec(),
            "the encoded string block changed"
        );
    }

    #[test]
    #[should_panic(expected = "character lane string must be valid non-control UTF-8")]
    fn a_control_character_is_still_refused_on_first_sight() {
        let mut table = StringTable::default();
        table.intern(Some("has a \u{7} bell"));
    }

    #[test]
    #[should_panic(expected = "character lane string exceeds max length")]
    fn an_overlong_string_is_still_refused_on_first_sight() {
        let mut table = StringTable::default();
        let overlong = "x".repeat(MAX_STRING_BYTES + 1);
        table.intern(Some(&overlong));
    }

    // The same two refusals, required of the LEGACY arm the differential test
    // compares against. Without these that arm's assertions are unkilled: the
    // differential fixture is all-valid by construction, so it would stay
    // green with either assertion deleted, and the arm would quietly stop
    // being the verbatim transcription this file calls it (Codex round 17,
    // finding 3). They are spelled against `LegacyStringTable` on purpose --
    // the two tests above already pin production.
    //
    // kills: deleting either assertion from `LegacyStringTable::intern`.

    #[test]
    #[should_panic(expected = "character lane string must be valid non-control UTF-8")]
    fn the_legacy_arm_refuses_a_control_character_the_way_production_does() {
        let mut legacy = LegacyStringTable::default();
        legacy.intern(Some("Iosif\x07Stalin"));
    }

    #[test]
    #[should_panic(expected = "character lane string exceeds max length")]
    fn the_legacy_arm_refuses_an_overlong_string_the_way_production_does() {
        let mut legacy = LegacyStringTable::default();
        let overlong = "x".repeat(MAX_STRING_BYTES + 1);
        legacy.intern(Some(&overlong));
    }

    #[test]
    fn row_len_literals_are_stable() {
        assert_eq!(CHARACTER_ROW_LEN, 20);
        assert_eq!(UNIT_LEADER_ROW_LEN, 42);
        assert_eq!(TRAIT_ROW_LEN, 16);
        assert_eq!(COUNTRY_LEADER_ROLE_ROW_LEN, 20);
        assert_eq!(ACTIVE_RULER_ROW_LEN, 28);
        assert_eq!(ADVISOR_ROW_LEN, 24);
        assert_eq!(CHARACTER_PORTRAIT_ROW_LEN, 24);
    }

    #[test]
    fn a_character_with_no_country_survives_encode_and_decode() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 0,
                char_id: 4711,
                name: Some("Unaffiliated Character"),
                token: Some("generic_unaffiliated"),
            }),
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
    }

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

        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.character_portrait_rows, 3);
        assert_eq!(summary.character_portrait_windows1252_rows, 1);

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
        let string_bytes = u32::from_le_bytes(payload[20..24].try_into().unwrap()) as usize;
        let row_start = HEADER_LEN + string_bytes;
        assert_eq!(payload[row_start], ROW_CHARACTER_PORTRAIT);
        assert!(decode_payload_rows(0, &payload).is_ok());

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
                error
                    .iter()
                    .any(|line| line.contains("unassigned flag bits")),
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
            assert_eq!(
                union & bit,
                0,
                "family bit for row kind {kind} is duplicated"
            );
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
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.family_presence_mask, FAMILY_CENSUS_ONLY);
        assert_eq!(
            summary.status_flags_or & STATUS_PARTIAL_DATA,
            STATUS_PARTIAL_DATA
        );
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

    #[test]
    fn an_undeclared_payload_still_means_every_family() {
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &[]);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.family_presence_mask, FAMILY_ALL);
        assert_eq!(summary.schema_flags_or & SCHEMA_FAMILY_PRESENCE_DECLARED, 0);

        let mut tampered = payload.clone();
        tampered[60..64].copy_from_slice(&FAMILY_CENSUS_ONLY.to_le_bytes());
        assert!(decode_payload(0, &tampered)
            .parse_errors
            .iter()
            .any(|error| error.contains("reserved header field is nonzero")));
    }

    #[test]
    fn the_family_declaration_refuses_every_incoherent_spelling() {
        let character = CharacterLaneRow::Character(CharacterRow {
            country_tag_db_id: 3,
            char_id: 2450,
            name: None,
            token: None,
        });

        assert!(declared(FAMILY_UNIT_LEADER | FAMILY_ACTIVE_RULER, &[])
            .iter()
            .any(|error| error.contains("omits the character census")));

        assert!(declared(FAMILY_CHARACTER | FAMILY_UNIT_LEADER_TRAIT, &[])
            .iter()
            .any(|error| error.contains("without the unit leaders")),);

        assert!(declared(FAMILY_CHARACTER | FAMILY_APPOINTED_ADVISOR, &[])
            .iter()
            .any(|error| error.contains("without the advisor roster")),);

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

    #[test]
    fn the_declaration_and_the_status_word_cannot_disagree() {
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &[]);

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

        let mut silent = payload.clone();
        silent[56..60].copy_from_slice(
            &(SCHEMA_ROOTS_VALIDATED | SCHEMA_FAMILY_PRESENCE_DECLARED).to_le_bytes(),
        );
        silent[60..64].copy_from_slice(&FAMILY_CENSUS_ONLY.to_le_bytes());
        assert!(decode_payload(0, &silent)
            .parse_errors
            .iter()
            .any(|error| error.contains("without partial data status")));

        let mut bare = payload.clone();
        bare[8..12].copy_from_slice(&STATUS_PARTIAL_DATA.to_le_bytes());
        assert!(decode_payload(0, &bare)
            .parse_errors
            .iter()
            .any(|error| error.contains("no family-presence declaration")));

        let mut source_failed = payload.clone();
        source_failed[8..12]
            .copy_from_slice(&(STATUS_PARTIAL_DATA | STATUS_SOURCE_UNAVAILABLE).to_le_bytes());
        assert!(decode_payload(0, &source_failed).parse_errors.is_empty());

        let mut future = payload;
        future[56..60].copy_from_slice(&(SCHEMA_ROOTS_VALIDATED | 1 << 20).to_le_bytes());
        assert!(decode_payload(0, &future)
            .parse_errors
            .iter()
            .any(|error| error.contains("unassigned bits")));
    }

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

    // ==================================================================
    // LANE SB (2026-08-24): the two internal structures this lane
    // re-spelled, each against a VERBATIM transcription of what it
    // replaced. Neither structure reaches the payload, so these are the
    // proofs that the payload cannot have moved;
    // `a_fully_present_payload_is_byte_identical` above is the same claim
    // end to end, over a fixture that interns ten distinct values and
    // repeats five of them across all eight row kinds.
    // ==================================================================

    /// The string table exactly as it stood before lane SB: `Vec<Rc<str>>`
    /// beside `HashMap<Rc<str>, u32>`. Kept in the tree so the differential
    /// test below has something real to compare against (lane RU's pattern).
    #[derive(Default)]
    struct LegacyStringTable {
        values: Vec<std::rc::Rc<str>>,
        by_value: std::collections::HashMap<std::rc::Rc<str>, u32>,
    }

    impl LegacyStringTable {
        fn intern(&mut self, value: Option<&str>) -> u32 {
            let Some(value) = value else {
                return STRING_NONE;
            };
            if let Some(id) = self.by_value.get(value) {
                return *id;
            }
            // The two miss-path assertions the old spelling carried, in the
            // position it carried them: after the lookup, before the push.
            // They were missing here, which made "verbatim" false (Codex
            // round 16, finding 4).
            //
            // The DIFFERENTIAL test does not kill them: its fixture is
            // deliberately all-valid, so deleting either one would leave it
            // green (Codex round 17, finding 3, which found the first version
            // of this comment claiming otherwise). What kills them is the pair
            // of `#[should_panic]` tests below, one per assertion, which
            // require this arm to refuse exactly what production refuses.
            assert!(
                value.len() <= MAX_STRING_BYTES,
                "character lane string exceeds max length"
            );
            assert!(
                is_domain47_string_value(value),
                "character lane string must be valid non-control UTF-8"
            );
            let id = self.values.len().min(u32::MAX as usize) as u32;
            let shared: std::rc::Rc<str> = std::rc::Rc::from(value);
            self.values.push(std::rc::Rc::clone(&shared));
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

    /// An intern sequence built to be hostile to the new table: lengths that
    /// straddle the 8-byte word the hash reads in, shared prefixes that only
    /// differ in the last word, values that differ only by one trailing
    /// byte, non-ASCII, repeats, and `None`s.
    fn adversarial_intern_sequence() -> Vec<Option<String>> {
        let mut sequence: Vec<Option<String>> = Vec::new();
        for len in 1..=24usize {
            sequence.push(Some("a".repeat(len)));
            sequence.push(Some(format!("{}b", "a".repeat(len))));
            sequence.push(Some("a".repeat(len)));
        }
        for index in 0..2_000u32 {
            sequence.push(Some(format!("SOV_character_token_{index:05}")));
            if index % 3 == 0 {
                sequence.push(None);
            }
            if index % 7 == 0 {
                sequence.push(Some(format!("SOV_character_token_{:05}", index / 7)));
            }
        }
        for value in [
            "Iosif Stalin",
            "Iosif  Stalin",
            "\u{cd}s\u{f6}f",
            "\u{a0}nbsp",
            "navy",
            "navy",
        ] {
            sequence.push(Some(value.to_string()));
        }
        sequence
    }

    #[test]
    fn the_string_table_matches_the_map_and_rc_spelling() {
        // kills: assigning ids in probe order, emitting the arena in probe
        // order, returning the probe index instead of the stored id, or
        // interning into the arena on the HIT path (the ids would stay
        // right and the encoded block would grow) - each diverges from the
        // legacy arm on this sequence.
        let sequence = adversarial_intern_sequence();
        let mut shipped = StringTable::default();
        let mut legacy = LegacyStringTable::default();
        for value in &sequence {
            let shipped_id = shipped.intern(value.as_deref());
            let legacy_id = legacy.intern(value.as_deref());
            assert_eq!(
                shipped_id, legacy_id,
                "intern({value:?}) returned a different id than the map/Rc spelling"
            );
        }
        assert_eq!(
            shipped.len(),
            legacy.values.len(),
            "table populations differ"
        );
        assert_eq!(
            shipped.encoded_len(),
            legacy.encoded_len(),
            "encoded_len differs, so the payload header's string_bytes would differ"
        );
        let mut shipped_bytes = Vec::new();
        let mut legacy_bytes = Vec::new();
        shipped.encode_into(&mut shipped_bytes);
        legacy.encode_into(&mut legacy_bytes);
        assert_eq!(
            shipped_bytes, legacy_bytes,
            "the encoded string block is not byte-identical to the map/Rc spelling"
        );
        assert_eq!(
            shipped_bytes.len(),
            shipped.encoded_len(),
            "encoded_len must be what encode_into writes"
        );
    }

    #[test]
    fn the_string_table_looks_every_value_back_up_after_every_growth() {
        // The probe table is rebuilt whenever it would pass half full; a
        // rehash that lost or double-placed an entry shows up as a repeat
        // minting a fresh id.
        let mut table = StringTable::default();
        let mut expected: Vec<String> = Vec::new();
        for index in 0..5_000u32 {
            let value = format!("value_{index}");
            assert_eq!(
                table.intern(Some(&value)),
                index,
                "ids must stay sequential"
            );
            expected.push(value);
        }
        for (index, value) in expected.iter().enumerate() {
            assert_eq!(
                table.intern(Some(value)),
                index as u32,
                "a value interned before a growth stopped being found after it"
            );
        }
        assert_eq!(table.len(), expected.len(), "a repeat minted a new id");
    }

    #[test]
    fn the_byte_scan_agrees_with_char_is_control_on_every_scalar() {
        // kills: dropping the 0xC2 arm (U+0085 would be admitted), widening
        // it to any continuation byte (U+00A0..=U+00BF would be refused), or
        // testing `<= 0x20` instead of `< 0x20` (a plain space would be
        // refused).
        let mut buffer = [0u8; 4];
        for scalar in 0..=0x10_FFFFu32 {
            let Some(character) = char::from_u32(scalar) else {
                continue;
            };
            let value: &str = character.encode_utf8(&mut buffer);
            assert_eq!(
                is_domain47_string_value(value),
                !value.chars().any(char::is_control),
                "byte scan disagrees with char::is_control on U+{scalar:04X}"
            );
        }
        // And in company, so a control scalar is still found when it is not
        // the whole value.
        for scalar in [0x00u32, 0x07, 0x1F, 0x7F, 0x85, 0x9F] {
            let character = char::from_u32(scalar).expect("scalar");
            let value = format!("lead {character} tail");
            assert!(
                !is_domain47_string_value(&value),
                "U+{scalar:04X} was admitted inside a longer value"
            );
        }
        for value in [
            "plain value",
            "Iosif Stalin",
            "\u{cd}s\u{f6}f",
            "\u{a0}",
            "\u{bf}",
            "gfx/leaders/SOV/portrait.dds",
        ] {
            assert!(
                is_domain47_string_value(value),
                "{value:?} must be admitted"
            );
        }
    }

    #[test]
    fn the_duplicate_detectors_match_the_btreeset_spelling() {
        // kills: returning `false` for a fresh key (every row would be
        // reported as a duplicate), returning `true` for a repeat (no
        // duplicate would ever be named), or a rehash that loses a key.
        let mut shipped = SeenSet::<u32>::with_bounded_capacity(4, 4);
        let mut legacy = std::collections::BTreeSet::new();
        for index in 0..4_000u32 {
            let key = index % 2_500;
            assert_eq!(
                shipped.insert(key),
                legacy.insert(key),
                "u32 key {key} disagreed with BTreeSet::insert"
            );
        }
        for key in 0..3_000u32 {
            assert_eq!(
                shipped.contains(key),
                legacy.contains(&key),
                "u32 key {key} disagreed with BTreeSet::contains"
            );
        }

        let mut shipped_quad = SeenSet::<[u32; 4]>::with_bounded_capacity(0, 0);
        let mut legacy_quad = std::collections::BTreeSet::new();
        let mut shipped_quint = SeenSet::<[u32; 5]>::with_bounded_capacity(9, 9);
        let mut legacy_quint = std::collections::BTreeSet::new();
        for index in 0..3_000u32 {
            let quad = [index % 2, index % 997, index % 31, index % 7];
            assert_eq!(
                shipped_quad.insert(quad),
                legacy_quad.insert(quad),
                "4-word key {quad:?} disagreed with BTreeSet::insert"
            );
            let quint = [index % 997, index % 31, index % 7, index % 2, index % 13];
            assert_eq!(
                shipped_quint.insert(quint),
                legacy_quint.insert(quint),
                "5-word key {quint:?} disagreed with BTreeSet::insert"
            );
        }

        // `contains` for the array widths too (Codex round 11, finding 6). The
        // shipped decoder only calls `contains` on the u32 unit-leader set, so
        // this is not covering a live call site -- it is covering the same
        // lookup path the wide keys DO use, whose key comparison is an array
        // compare rather than a scalar one. Half the probes below are for keys
        // that were never offered, so a `contains` that answered from the slot
        // hash without comparing the key would fail here.
        for index in 0..3_000u32 {
            let quad = [index % 2, index % 997, index % 31, index % 7];
            assert_eq!(
                shipped_quad.contains(quad),
                legacy_quad.contains(&quad),
                "4-word key {quad:?} disagreed with BTreeSet::contains"
            );
            let absent_quad = [index % 2, index % 997, index % 31, 8 + index % 7];
            assert_eq!(
                shipped_quad.contains(absent_quad),
                legacy_quad.contains(&absent_quad),
                "absent 4-word key {absent_quad:?} disagreed with BTreeSet::contains"
            );
            let quint = [index % 997, index % 31, index % 7, index % 2, index % 13];
            assert_eq!(
                shipped_quint.contains(quint),
                legacy_quint.contains(&quint),
                "5-word key {quint:?} disagreed with BTreeSet::contains"
            );
            let absent_quint = [
                index % 997,
                index % 31,
                index % 7,
                index % 2,
                14 + index % 13,
            ];
            assert_eq!(
                shipped_quint.contains(absent_quint),
                legacy_quint.contains(&absent_quint),
                "absent 5-word key {absent_quint:?} disagreed with BTreeSet::contains"
            );
        }
    }

    #[test]
    fn a_header_that_over_declares_its_rows_reserves_only_what_the_bytes_allow() {
        // The bounded reservation, in the shape Codex round 11 found on the
        // string table: a declared population of four billion in a payload
        // with room for nine rows must not be reserved.
        let set = SeenSet::<[u32; 5]>::with_bounded_capacity(u64::from(u32::MAX), 9);
        assert!(
            set.keys.capacity() <= 64,
            "the declared count was reserved instead of the ceiling: {}",
            set.keys.capacity()
        );
        assert!(
            set.slots.len() <= 64,
            "the probe table was sized from the declared count: {}",
            set.slots.len()
        );
    }

    /// This file, for the structural pin below. `include_str!` resolves
    /// against this file's own directory, so it is this module's source.
    const COMPACT_SOURCE: &str = include_str!("domain47_compact.rs");

    #[test]
    fn every_production_reservation_goes_through_the_joint_feasibility_gate() {
        // Codex round 6, finding 6. The gate is tested as a FUNCTION by
        // `a_header_that_claims_the_row_region_more_than_once_is_not_reserved_for`,
        // but a function nobody calls closes nothing: changing one of the six
        // reservations from `trusted(expected_x)` back to `expected_x`
        // reopens the multiplicative-reservation defect while every
        // behavioural test stays green, because the amplification only shows
        // on a payload big enough that a test would have to allocate
        // hundreds of megabytes to see it.
        //
        // So the wiring is pinned as SOURCE: every `with_bounded_capacity`
        // call in the decoder passes its declared count through `trusted`,
        // and there are exactly six of them.
        //
        // The needles are assembled at run time so that this test's own text
        // does not match them.
        let source = COMPACT_SOURCE.replace("\r\n", "\n");
        let gated = format!("with_bounded_capacity({}(", "trusted");
        assert_eq!(
            source.matches(gated.as_str()).count(),
            6,
            "the decoder must make exactly six duplicate-detector reservations and every \
             one of them must pass its declared count through the joint feasibility gate"
        );
        let ungated = format!("with_bounded_capacity({}", "expected_");
        assert!(
            !source.contains(ungated.as_str()),
            "a reservation takes a header count directly instead of through `trusted`, \
             which lets six headers each claim the whole row region"
        );
    }

    #[test]
    fn a_payload_with_a_duplicate_in_every_detected_family_still_names_each_one() {
        // The production kill for the SeenSet swap: six families each carry
        // one exact repeat, and the decoder must still name all six through
        // the real encode/decode path.
        let mut rows = byte_identity_fixture_rows();
        let duplicates: Vec<CharacterLaneRow<'static>> = byte_identity_fixture_rows()
            .into_iter()
            .filter(|row| {
                !matches!(
                    row,
                    CharacterLaneRow::UnitLeaderTrait(_) | CharacterLaneRow::AppointedAdvisor(_)
                )
            })
            .filter(|row| {
                !matches!(
                    row,
                    CharacterLaneRow::Character(CharacterRow { char_id: 2451, .. })
                )
            })
            .collect();
        assert_eq!(
            duplicates.len(),
            6,
            "one repeat per duplicate-detected family"
        );
        rows.extend(duplicates);
        let payload = encode_payload(
            7,
            0,
            SCHEMA_ROOTS_VALIDATED
                | SCHEMA_STATIC_NAME_CLOSURE
                | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            &rows,
        );
        let summary = decode_payload(0, &payload);
        let named = |needle: &str| {
            summary
                .parse_errors
                .iter()
                .filter(|error| error.contains(needle))
                .count()
        };
        assert_eq!(
            named("duplicates character id 2450"),
            1,
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            named("duplicates unit leader id 77"),
            1,
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            named("duplicates country-leader role char id 2450"),
            1,
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            named("duplicates active ruler for country tag id 3"),
            1,
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            named("duplicates advisor row"),
            1,
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            named("duplicates character portrait row"),
            1,
            "{:?}",
            summary.parse_errors
        );
        let duplicate_errors: Vec<&String> = summary
            .parse_errors
            .iter()
            .filter(|error| error.contains("duplicates"))
            .collect();
        assert_eq!(duplicate_errors.len(), 6, "{duplicate_errors:?}");
    }

    #[test]
    fn a_clean_payload_reports_no_duplicate_at_all() {
        // The other half of the kill: a detector that answered "seen" for
        // everything would make the shipped fixture unpublishable.
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
            "{:?}",
            summary.parse_errors
        );
    }

    /// A `LaneFrame` whose rows are exactly [`byte_identity_fixture_rows`],
    /// family by family, so the production entry point can be driven over the
    /// same fixture the golden hex above pins (lane SB).
    fn byte_identity_fixture_frame() -> crate::extractor::LaneFrame {
        use crate::extractor::{
            ActiveRulerRecord, AdvisorRecord, CharacterPortraitRecord, CharacterRecord,
            CountryLeaderRoleRecord, LaneFrame, UnitLeaderRecord, UnitLeaderTraitRecord,
        };
        let mut frame = LaneFrame::new(7);
        frame.characters = vec![
            CharacterRecord {
                country_tag_db_id: 3,
                char_id: 2450,
                name: Some("Iosif Stalin".to_string()),
                token: Some("SOV_iosif_stalin".to_string()),
            },
            CharacterRecord {
                country_tag_db_id: 3,
                char_id: 2451,
                name: None,
                token: None,
            },
        ];
        frame.unit_leaders = vec![UnitLeaderRecord {
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
            token: Some("SOV_iosif_stalin".to_string()),
            name: Some("Iosif Stalin".to_string()),
            trait_count: 1,
        }];
        frame.unit_leader_traits = vec![UnitLeaderTraitRecord {
            unit_leader_id: 77,
            ordinal: 0,
            trait_token: "old_guard".to_string(),
        }];
        frame.country_leader_roles = vec![CountryLeaderRoleRecord {
            country_tag_db_id: 3,
            char_id: 2450,
            name: Some("Iosif Stalin".to_string()),
            token: Some("SOV_iosif_stalin".to_string()),
        }];
        frame.active_rulers = vec![ActiveRulerRecord {
            country_tag_db_id: 3,
            char_id: 2450,
            ruling_party: Some("communism".to_string()),
            name: Some("Iosif Stalin".to_string()),
            token: Some("SOV_iosif_stalin".to_string()),
            leader_vec_size: 1,
            ambiguous: false,
        }];
        frame.advisor_roster = vec![AdvisorRecord {
            country_tag_db_id: 3,
            char_id: 2451,
            slot: None,
            ledger: Some("political_advisor".to_string()),
            idea_token: "SOV_lavrentiy_beria".to_string(),
        }];
        frame.appointed_advisors = vec![AdvisorRecord {
            country_tag_db_id: 3,
            char_id: 2451,
            slot: Some("political_advisor".to_string()),
            ledger: Some("high_command".to_string()),
            idea_token: "SOV_lavrentiy_beria".to_string(),
        }];
        frame.character_portraits = vec![CharacterPortraitRecord {
            country_tag_db_id: 3,
            char_id: 2450,
            role: "navy".to_string(),
            size: "large".to_string(),
            gfx: "gfx/leaders/SOV/portrait_sov_iosif_stalin.dds".to_string(),
            gfx_encoding: PortraitIdentityEncoding::Utf8,
        }];
        frame
    }

    #[test]
    fn the_production_encode_path_produces_the_same_golden_payload() {
        // `encode_frame_payload_over` is what the collector calls, and since
        // lane SB it encodes straight off `LaneFrame::compact_row_iter_over`
        // instead of a materialised `Vec`. The golden above drives the SLICE
        // entry point; this drives the production one over the same rows and
        // requires the same bytes.
        //
        // kills: reordering the chain in `compact_row_iter_over` (portraits
        // before characters, traits before their leaders), changing a field
        // mapping inside it, or dropping one of the eight selections - each
        // moves the payload while every slice-path test still passes.
        let frame = byte_identity_fixture_frame();
        let payload = crate::extractor::encode_frame_payload_over(&frame, None);
        assert_eq!(
            to_hex(&payload),
            FULLY_PRESENT_PAYLOAD_HEX,
            "the production encode path no longer produces the payload on disk"
        );
        // And the materialised sequence is still the hand-written one, so a
        // future reader of `compact_rows_over` sees the same order the golden
        // encodes.
        assert_eq!(
            frame.compact_rows_over(None),
            byte_identity_fixture_rows(),
            "the row sequence moved"
        );
    }

    #[test]
    fn values_past_the_wire_id_clamp_are_still_found_by_their_repeats() {
        // Codex round 1, finding 1, tested AT the clamp (Codex round 2,
        // finding 3): the ceiling is lowered to 3, so ids 3, 4 and 5 all
        // clamp to 3 and the entries that carry them are the ones a repeat
        // has to find. The old `HashMap<Rc<str>, u32>` was keyed by the
        // STRING, so it found them; a probe table keyed by the CLAMPED id, or
        // one that skipped `place` past the clamp, would not, and each repeat
        // would append a second copy of a value already in the block.
        //
        // kills: `place`ing by `wire_id(index)` instead of `index`; guarding
        // `place` with `if id != u32::MAX`; looking a repeat up by its id.
        let mut table = StringTable::with_wire_id_ceiling(3);
        let values = ["a", "b", "c", "d", "e", "f"];
        let expected_ids = [0u32, 1, 2, 3, 3, 3];
        for (value, expected) in values.iter().zip(expected_ids) {
            assert_eq!(
                table.intern(Some(value)),
                expected,
                "first sight of {value}"
            );
        }
        assert_eq!(table.len(), values.len(), "a first sight was lost");
        for (value, expected) in values.iter().zip(expected_ids) {
            assert_eq!(table.intern(Some(value)), expected, "repeat of {value}");
        }
        assert_eq!(
            table.len(),
            values.len(),
            "a repeat past the clamp appended a second copy, which would move \
             string_count, string_bytes and the encoded block"
        );
        let mut block = Vec::new();
        table.encode_into(&mut block);
        assert_eq!(
            block,
            b"\x01\x00a\x01\x00b\x01\x00c\x01\x00d\x01\x00e\x01\x00f".to_vec(),
            "the block must still be every value once, in id order"
        );
        // And the production ceiling is the old spelling's.
        assert_eq!(
            StringTable::default().wire_id_ceiling,
            u32::MAX as usize,
            "production must clamp exactly where the map spelling clamped"
        );
    }

    #[test]
    fn both_probe_tables_store_usize_indices() {
        // Codex round 2, finding 3: the behavioural test below cannot tell a
        // `Vec<usize>` from a `Vec<u32>` with a bail-out arm, because the
        // arm is unreachable at any population a test can build. The claim is
        // about the TYPE, so it is asserted about the type: these two lines
        // stop compiling if either table narrows its slots again, and the
        // narrowing is exactly what would reintroduce the arm.
        let seen = SeenSet::<u32>::with_bounded_capacity(0, 0);
        let _: &Vec<usize> = &seen.slots;
        let strings = StringTable::default();
        let _: &Vec<usize> = &strings.slots;
    }

    #[test]
    fn every_key_survives_every_growth_of_the_probe_table() {
        // Codex round 1, finding 2's BEHAVIOURAL half: across ten thousand
        // inserts and the rehashes they force, every key inserted is still
        // found and every repeat is still a repeat.
        //
        // kills: a rehash that loses or double-places a key; an `insert` that
        // returns `true` without storing.
        //
        // It does NOT reach the u32 index ceiling the round-1 defect lived at
        // (Codex round 4, finding 5) -- four billion keys are not something a
        // test can build. That claim is a claim about the TYPE and is
        // asserted as one, by `both_probe_tables_store_usize_indices`.
        let mut set = SeenSet::<u32>::with_bounded_capacity(0, 0);
        for key in 0..10_000u32 {
            assert!(set.insert(key), "{key} is new");
        }
        for key in 0..10_000u32 {
            assert!(!set.insert(key), "{key} is a repeat");
            assert!(set.contains(key), "{key} must still be found");
        }
    }

    #[test]
    fn declared_row_bytes_are_exact_and_saturating() {
        // Codex round 1, finding 3.
        //
        // kills: multiplying without saturating (four billion characters
        // would wrap to a small number and be declared feasible), or leaving
        // a family out of the sum (its rows would be reserved for on top of a
        // region already fully claimed by the others).
        assert_eq!(domain47_declared_row_bytes(0, 0, 0, 0, 0, 0, 0), 0);
        assert_eq!(
            domain47_declared_row_bytes(1, 1, 1, 1, 1, 1, 1),
            u64::from(CHARACTER_ROW_LEN)
                + u64::from(UNIT_LEADER_ROW_LEN)
                + u64::from(TRAIT_ROW_LEN)
                + u64::from(COUNTRY_LEADER_ROLE_ROW_LEN)
                + u64::from(ACTIVE_RULER_ROW_LEN)
                + u64::from(ADVISOR_ROW_LEN)
                + u64::from(CHARACTER_PORTRAIT_ROW_LEN)
        );
        assert_eq!(
            domain47_declared_row_bytes(
                u64::MAX,
                u64::MAX,
                u64::MAX,
                u64::MAX,
                u64::MAX,
                u64::MAX,
                u64::MAX
            ),
            u64::MAX,
            "the sum must saturate, not wrap"
        );
        // Each family, one row, weighed against ITS OWN width -- not merely
        // against zero. The all-ones case above is blind to two constants
        // being swapped, because the sum is the same either way (Codex round
        // 5, finding 7).
        //
        // kills: giving any family another family's row length, or dropping a
        // family from the sum.
        let widths = [
            ("character", CHARACTER_ROW_LEN),
            ("unit_leader", UNIT_LEADER_ROW_LEN),
            ("trait", TRAIT_ROW_LEN),
            ("country_leader_role", COUNTRY_LEADER_ROLE_ROW_LEN),
            ("active_ruler", ACTIVE_RULER_ROW_LEN),
            ("advisor", ADVISOR_ROW_LEN),
            ("character_portrait", CHARACTER_PORTRAIT_ROW_LEN),
        ];
        for (family, (name, width)) in widths.iter().enumerate() {
            let mut counts = [0u64; 7];
            counts[family] = 1;
            assert_eq!(
                domain47_declared_row_bytes(
                    counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
                ),
                u64::from(*width),
                "one {name} row must cost exactly {width} bytes in the sum"
            );
            counts[family] = 3;
            assert_eq!(
                domain47_declared_row_bytes(
                    counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
                ),
                u64::from(*width) * 3,
                "the {name} term must be linear in its count"
            );
        }
    }

    #[test]
    fn a_header_that_claims_the_row_region_more_than_once_is_not_reserved_for() {
        // Codex round 1, finding 3: six detectors sizing themselves from six
        // header fields, each bounded only by the WHOLE region, multiply the
        // reservation by six. A megabyte of rows cannot hold a megabyte of
        // characters AND a megabyte of portraits.
        //
        // kills: dropping the joint feasibility test and keeping only the
        // per-set `row_ceiling` -- the second assertion below then passes
        // where it must fail.
        let region = 1 << 20usize;
        let one_family_full = (region as u64) / u64::from(CHARACTER_ROW_LEN);
        assert!(
            domain47_reservations_are_trustworthy(
                domain47_declared_row_bytes(one_family_full, 0, 0, 0, 0, 0, 0),
                region
            ),
            "a header that fills the region exactly must still be reserved for"
        );
        assert!(
            !domain47_reservations_are_trustworthy(
                domain47_declared_row_bytes(
                    one_family_full,
                    0,
                    0,
                    0,
                    0,
                    0,
                    (region as u64) / u64::from(CHARACTER_PORTRAIT_ROW_LEN)
                ),
                region
            ),
            "two families each claiming the whole region must not be reserved for"
        );
        assert!(
            !domain47_reservations_are_trustworthy(
                domain47_declared_row_bytes(u64::from(u32::MAX), 0, 0, 0, 0, 0, 0),
                region
            ),
            "a four-billion-row declaration in a one-megabyte region must not be reserved for"
        );
        // And a refused declaration reserves nothing at all.
        let refused = SeenSet::<[u32; 5]>::with_bounded_capacity(0, region / 4);
        assert_eq!(refused.keys.capacity(), 0);
        assert_eq!(refused.slots.len(), 0);
    }

    /// Keys chosen to share a probe start: `count` distinct advisor-shaped
    /// keys whose finalized hashes agree in their low `bits`.
    ///
    /// A table holding `count` keys is at most `count * 4` slots wide
    /// (`reserve_one`), so as long as `2^bits >= count * 4` every one of them
    /// masks to the SAME slot whatever size the table settles at, and the
    /// cluster grows by one on every insert. This is the shape Codex round 2
    /// finding 1 described, built rather than argued.
    fn keys_sharing_a_probe_start(count: usize, bits: u32) -> Vec<[u32; 4]> {
        assert!(
            (1usize << bits) >= count * 4,
            "the shared prefix must survive every growth"
        );
        let mask = (1u64 << bits) - 1;
        let target = [7u32, 11, 13, 0].seen_hash() & mask;
        let mut found = vec![[7u32, 11, 13, 0]];
        let mut candidate = 1u32;
        while found.len() < count {
            let key = [7u32, 11, 13, candidate];
            if key.seen_hash() & mask == target {
                found.push(key);
            }
            candidate = candidate.checked_add(1).expect("32 bits is plenty of room");
        }
        found
    }

    #[test]
    fn a_chosen_collision_set_degrades_the_probe_table_instead_of_scanning_forever() {
        // Codex round 2, finding 1. `decode_payload` is the canonical decoder
        // for payloads this process did not necessarily produce, and the hash
        // is unkeyed, so an author who wants every advisor row to hash to one
        // slot can have it. Open addressing then costs O(n^2); the `BTreeSet`
        // this replaced cost O(n log n) and this must not be worse.
        //
        // kills: deleting `D47_PROBE_LIMIT` or the `TooLong` arm -- the
        // assertions below on `ordered` fail, and the same construction at
        // production scale is what turns a sub-megabyte payload into a
        // billion probes.
        let colliding = keys_sharing_a_probe_start(D47_PROBE_LIMIT * 2, 12);
        let mut shipped = SeenSet::<[u32; 4]>::with_bounded_capacity(0, 0);
        let mut legacy = std::collections::BTreeSet::new();
        for key in &colliding {
            assert_eq!(
                shipped.insert(*key),
                legacy.insert(*key),
                "a chosen-collision key must still get the BTreeSet's answer"
            );
        }
        assert!(
            shipped.ordered.is_some(),
            "the table kept probing a cluster of {} keys",
            colliding.len()
        );
        assert!(shipped.slots.is_empty() && shipped.keys.is_empty());
        // And it keeps answering correctly afterwards, for the colliding keys
        // and for ordinary ones mixed in after the switch.
        for key in &colliding {
            assert!(!shipped.insert(*key), "a repeat must still be a repeat");
            assert!(shipped.contains(*key), "a key must survive the switch");
        }
        for index in 0..500u32 {
            let key = [index, index * 3, index * 5, index * 7];
            assert_eq!(shipped.insert(key), legacy.insert(key));
            assert_eq!(shipped.contains(key), legacy.contains(&key));
        }
    }

    /// Strings that share a probe start under the SHIPPED byte hash: `count`
    /// distinct values whose finalized hashes agree in their low `bits`.
    ///
    /// Same construction as `keys_sharing_a_probe_start`, over bytes. A table
    /// holding `count` values is at most `count * 4` slots wide, so as long
    /// as `2^bits >= count * 4` every one of them masks to the same slot at
    /// whatever size the table settles at.
    fn strings_sharing_a_probe_start(count: usize, bits: u32) -> Vec<String> {
        assert!(
            (1usize << bits) >= count * 4,
            "the shared prefix must survive every growth"
        );
        let mask = (1u64 << bits) - 1;
        let first = "collide_00000000".to_string();
        let target = d47_hash_bytes(first.as_bytes()) & mask;
        let mut found = vec![first];
        let mut candidate = 1u64;
        while found.len() < count {
            let value = format!("collide_{candidate:08}");
            if d47_hash_bytes(value.as_bytes()) & mask == target {
                found.push(value);
            }
            candidate += 1;
        }
        found
    }

    #[test]
    fn a_chosen_collision_set_degrades_the_string_table_through_intern() {
        // Codex round 3, finding 3: the test below drives `degrade()` by
        // hand, so deleting the AUTOMATIC fallback would leave it green.
        // This one interns ordinary, valid strings that were merely CHOSEN to
        // share a probe start, and requires `intern` itself to notice.
        //
        // kills: removing the `TooLong` arm from `StringTable::find_existing`
        // -- the assertion on `ordered` fails, and at production scale the
        // same construction is what turns one frame's interning into a
        // quadratic scan.
        let colliding = strings_sharing_a_probe_start(D47_PROBE_LIMIT * 2, 13);
        let mut shipped = StringTable::default();
        let mut legacy = LegacyStringTable::default();
        for value in &colliding {
            assert_eq!(
                shipped.intern(Some(value)),
                legacy.intern(Some(value)),
                "a chosen-collision value must still get the map spelling's id"
            );
        }
        assert!(
            shipped.ordered.is_some(),
            "the table kept probing a cluster of {} values",
            colliding.len()
        );
        // Repeats, and ordinary values mixed in after the switch, still agree.
        for value in colliding.iter().rev() {
            assert_eq!(shipped.intern(Some(value)), legacy.intern(Some(value)));
        }
        for index in 0..200u32 {
            let value = format!("ordinary_{index}");
            assert_eq!(shipped.intern(Some(&value)), legacy.intern(Some(&value)));
        }
        assert_eq!(
            shipped.len(),
            legacy.values.len(),
            "a repeat minted a new id"
        );
        let mut shipped_block = Vec::new();
        let mut legacy_block = Vec::new();
        shipped.encode_into(&mut shipped_block);
        legacy.encode_into(&mut legacy_block);
        assert_eq!(
            shipped_block, legacy_block,
            "the encoded block moved when the table degraded"
        );
    }

    #[test]
    fn the_string_table_degrades_without_moving_one_id_or_one_byte() {
        // The encoder's half of the same bound, driven at a chosen moment.
        // The arena IS the payload's string block, so degrading may not touch
        // it: the ordered index is built FROM the arena and only changes how a
        // repeat is found.
        //
        // kills: rebuilding the arena on degradation, or letting the ordered
        // path mint a fresh id for a value the probe table already holds --
        // either moves `string_count`, `string_bytes` and the block.
        let values: Vec<String> = (0..400).map(|index| format!("token_{index:04}")).collect();
        let mut probed = StringTable::default();
        let mut degraded = StringTable::default();
        degraded.degrade();
        assert!(degraded.ordered.is_some(), "degrade must take effect");
        for value in &values {
            assert_eq!(probed.intern(Some(value)), degraded.intern(Some(value)));
        }
        // Degrade halfway through a third table and require the same result.
        let mut halfway = StringTable::default();
        for (index, value) in values.iter().enumerate() {
            if index == 137 {
                halfway.degrade();
            }
            assert_eq!(halfway.intern(Some(value)), probed.intern(Some(value)));
        }
        assert!(halfway.ordered.is_some());
        for table in [&probed, &degraded, &halfway] {
            assert_eq!(table.len(), values.len(), "a repeat minted a new id");
            assert_eq!(table.encoded_len(), probed.encoded_len());
        }
        let mut probed_block = Vec::new();
        let mut degraded_block = Vec::new();
        let mut halfway_block = Vec::new();
        probed.encode_into(&mut probed_block);
        degraded.encode_into(&mut degraded_block);
        halfway.encode_into(&mut halfway_block);
        assert_eq!(
            probed_block, degraded_block,
            "the block moved when degraded"
        );
        assert_eq!(
            probed_block, halfway_block,
            "the block moved on a mid-frame switch"
        );
    }

    #[test]
    fn the_hash_finalizer_spreads_a_structured_key_set_across_the_low_bits() {
        // Codex round 2, finding 1's accidental half: `d47_hash_word` ends in
        // a multiply, and multiplication carries only upward, so without the
        // finalizer the low bits a probe mask reads are a function of the low
        // bits of the last word alone. Ids that step by a power of two --
        // which string ids and row ids readily do -- then all mask to one
        // slot.
        //
        // kills: deleting `d47_hash_finish` from `SeenKey::seen_hash`.
        // Unfinalized, the low 16 bits of every key below are identical --
        // the last word's low bits are zero and a multiply carries only
        // upward -- so all 512 land in ONE of the 4,096 buckets.
        //
        // The byte hash gets the same finalizer for the same reason. It is
        // not asserted here because a low-bit-constant STRING corpus has to
        // be solved for rather than written down, and an unsolved corpus
        // would pass with or without the finalizer -- a test that cannot
        // fail. What bounds the byte hash's worst case is the probe limit,
        // which `a_chosen_collision_set_degrades_the_string_table_through_intern`
        // drives through `intern` itself.
        let mut buckets = std::collections::BTreeSet::new();
        for step in 0..512u32 {
            buckets.insert([7u32, 11, 13, step * 65_536].seen_hash() as usize & 0xFFF);
        }
        assert!(
            buckets.len() > 256,
            "512 keys stepping by 65,536 landed in only {} of 4,096 buckets",
            buckets.len()
        );
    }
}
