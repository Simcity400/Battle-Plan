//! CRelationStatus, one eagerly allocated relation record per target country.

use crate::{
    Memory, cair_base_access_relation, cattache_relation, ccountry, cdiplomacy_status,
    cdocking_rights_relation, cmilitary_access_relation, coffer_air_base_access_relation,
    coffer_docking_rights_relation, coffer_military_access_relation,
    cpdx_array::Array,
    crelation, csubject_relation, cwar_relation,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
    walk::{Buffer, Outcome},
};

mod baseline;
use baseline::Identity;
pub use baseline::{Baseline, Counts as BaselineCounts, Mark, Observation};

/// CRelationStatus primary vtable.
pub const VTABLE_RVA: usize = 0x0294_a020;
/// Relation-status constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00d0_3410;
/// Relation-status allocation size.
pub const OBJECT_SIZE: usize = 0x9c0;
/// Target country reference, separate from the owner.
pub const TARGET_COUNTRY: usize = 0x50;
/// CRelationStatus five embedded CGameDate starts.
pub const DATE_BLOCKS: [usize; 5] = [0x58, 0x70, 0x88, 0xa0, 0xb8];
/// Subrelation pointer-vector data.
pub const SUBRELATIONS: usize = 0xd0;
/// Subrelation pointer-vector capacity read as signed i32.
pub const SUBRELATION_CAPACITY: usize = 0xd8;
/// Subrelation pointer-vector count read as signed i32.
pub const SUBRELATION_COUNT: usize = 0xdc;
/// CRelationStatus other pointer-vector data offsets.
pub const OTHER_RELATION_VECTORS: [usize; 3] = [0xe8, 0x100, 0x118];
/// Serializable relation vector containing relations only for their first endpoint.
pub const SERIALIZABLE_RELATIONS: usize = 0xe8;
/// Serializable relation count, signed i32, shrunk on removal and used to bound serialization.
pub const SERIALIZABLE_RELATION_COUNT: usize = 0xf4;
/// CRelationStatus corresponding pointer-vector count offsets.
pub const OTHER_RELATION_COUNTS: [usize; 3] = [0xf4, 0x10c, 0x124];
/// CRelationStatus embedded CModifier.
pub const MODIFIER: usize = 0x130;
/// Owning diplomacy pointer, assigned only during construction and equal to
/// the owner country's diplomacy. Opinion-write observers use it to file a
/// mark under that owner.
pub const DIPLOMACY_STATUS: usize = 0x1f0;
/// Subject-relation pointer, authenticated before following its fields.
pub const SUBJECT_RELATION: usize = 0x288;
/// CRelationStatus foreign-supply access pointer-sized word.
pub const SUPPLY_ACCESS: usize = 0x2b8;
/// War-relation pointer, separate from inline visibility fields.
pub const WAR_RELATION: usize = 0x2e8;
/// Directed stored co-belligerent byte. Rebuilding assigns zero or one;
/// construction and reset clear it. Counter grouping and path cost test
/// nonzero without an active-war veto.
pub const CO_BELLIGERENT: usize = 0x310;
/// Cached opinion, signed i32, serialized as `cached_sum` only when nonzero.
pub const OPINION: usize = 0x300;

/// Serializer writing opinion modifiers, conditional cached opinion and dates,
/// all serializable relations, attitude and friction.
pub const SERIALIZER_RVA: usize = 0x00d1_9300;
/// Cached-opinion key token, emitted when the signed dword is nonzero.
pub const OPINION_TOKEN: u32 = 0x2d4e;
/// Directional attache link written in the second country's status toward
/// the first, cleared by removal and tested by fog alliance.
pub const ATTACHE_LINK: usize = 0x210;
/// Reciprocal attache link written in the first country's status toward
/// the second; the attache predicate tests only the opposite field.
pub const RECIPROCAL_ATTACHE_LINK: usize = 0x208;
/// Subject first-party predicate validating subject type and cosmetic endpoint equality.
pub const SUBJECT_FIRST_PARTY_MATCH_RVA: usize = 0x00d1_1030;
/// Subject second-party predicate validating subject type and cosmetic endpoint equality.
pub const SUBJECT_SECOND_PARTY_MATCH_RVA: usize = 0x00d1_1080;
/// Attache predicate testing the directional link for nonzero.
pub const HAS_ATTACHE_LINK_RVA: usize = 0x00d1_00b0;

/// Air-base access pointer admitting foreign air-base visibility for the viewer.
pub const AIR_BASE_ACCESS: usize = 0x2c0;
/// [`LinkRow::links`] bit: [`ATTACHE_LINK`] is set.
pub const LINK_ATTACHE: u8 = 1;
/// [`LinkRow::links`] bit: [`AIR_BASE_ACCESS`] is set.
pub const LINK_AIR_BASE_ACCESS: u8 = 2;
/// Military-access pointer used by province movement and full-country
/// visibility. Direct access links the first country toward the second;
/// offered access links the second toward the first. Removal clears the slot.
pub const MILITARY_ACCESS: usize = 0x2a0;
/// [`LinkRow::links`] bit: [`MILITARY_ACCESS`] is set.
pub const LINK_MILITARY_ACCESS: u8 = 8;
/// Docking-rights pointer used by province access, full-country visibility
/// and naval-base icon access. Direct rights link the first toward the second;
/// offered rights link the second toward the first.
pub const DOCKING_RIGHTS: usize = 0x2b0;
/// [`LinkRow::links`] bit: [`DOCKING_RIGHTS`] is set.
pub const LINK_DOCKING_RIGHTS: u8 = 16;

/// A directed status with at least one political pointer slot set.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkRow {
    /// Raw id of the country whose matrix holds the status.
    pub country: u32,
    /// Matrix index: the target's canonical slot.
    pub target: u32,
    pub links: u8,
    /// Supplied only by links_observed_transport; no native pointer survives.
    pub transport_subject: Option<Result<TransportSubject, &'static str>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportSubject {
    pub sample_hour: i32,
    /// Self-keyed row certifies this owner's complete sparse candidate walk.
    pub complete_owner: bool,
    pub target_raw: Option<u32>,
    pub subject: Option<ZeroSubjectInputs>,
}

mod transport_access;

/// Country occupation-rule/enemy-set operands. Active owned war relations
/// supply the same endpoint set as the enemy vector, regardless of order.
/// Read the vector where derivation can fail: civil-war revolters before
/// their own declaration and owners of retired relations lingering in lists.
/// The selected-route suffix independently retains every enemy occurrence,
/// including zero and repeats. This exceptional-set prefix cannot supply it.
/// Wartime allies and co-belligerent bytes are not needed for the raid source
/// search because their mode-2 edge cost cannot alter source reachability.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WarRow {
    /// Raw id of the country.
    pub country: u32,
    /// The native [`ccountry::CAN_OCCUPY_NON_WAR`] byte, not normalized.
    pub can_occupy_non_war: u8,
    /// Whether the enemy vector was read; otherwise the span is empty.
    pub enemies_read: bool,
    /// Span of the raw links, in native order, in [`WarOutput::countries`].
    pub enemies_start: usize,
    pub enemies_len: usize,
    pub ordered: Option<(usize, usize)>,
    pub ordered_hour: Option<i32>,
    pub ordered_failure: Option<&'static str>,
}

/// Caller-owned bounded storage for the war rows the relation-link walk produces.
pub struct WarOutput<'a> {
    pub rows: &'a mut [WarRow],
    /// Every row's enemy links.
    pub countries: &'a mut [u32],
}

/// Independent outcomes of the one diplomacy pass behind RelationLinks and CountryWars.
pub struct WarBuffers<'a> {
    pub rows: Buffer<'a, WarRow>,
    pub countries: Buffer<'a, u32>,
}
impl<'a> From<WarOutput<'a>> for WarBuffers<'a> {
    fn from(wars: WarOutput<'a>) -> Self {
        Self {
            rows: Buffer::Fixed(wars.rows),
            countries: Buffer::Fixed(wars.countries),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkWalk {
    pub links: Outcome,
    pub wars: Outcome,
    /// Links written to [`WarOutput::countries`].
    pub countries_len: usize,
    /// Links every nonempty vector needed, including rows that did not fit.
    pub required_countries: usize,
    /// Current occurrence refusals, including missing reserved roots and short rows.
    pub ordered_refusals: u32,
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}
/// Type word, both endpoints and the retired byte of one owned relation.
const OWNED_FIELDS: [RecordField; 3] = [
    field(crelation::TYPE_ID, 0, 4),
    field(crelation::FIRST_COUNTRY, 4, 8),
    field(crelation::RETIRED, 12, 1),
];
const OWNED_BYTES: usize = 13;
const OWNED_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&OWNED_FIELDS, OWNED_BYTES).expect("valid owned relation layout");
/// A country's diplomacy status pointer, occupation rule byte and civil-war
/// link, read for a batch of countries in one guarded copy.
const COUNTRY_FIELDS: [RecordField; 3] = [
    field(ccountry::DIPLOMACY_STATUS, 0, 8),
    field(ccountry::CAN_OCCUPY_NON_WAR, 8, 1),
    field(ccountry::CIVIL_WAR_LINK, 9, 4),
];
const COUNTRY_BYTES: usize = 13;
const COUNTRY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COUNTRY_FIELDS, COUNTRY_BYTES).expect("valid country layout");
/// A diplomacy status's vtable, owning country and owned-relation header
/// (what `diplomacy_root` checks one country at a time).
const DIPLOMACY_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(cdiplomacy_status::COUNTRY, 8, 8),
    field(cdiplomacy_status::OWNED_RELATIONS, 16, 16),
];
const DIPLOMACY_BYTES: usize = 32;
const DIPLOMACY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DIPLOMACY_FIELDS, DIPLOMACY_BYTES).expect("valid diplomacy layout");
/// Identity and the four political slots of one status.
const LINK_FIELDS: [RecordField; 7] = [
    field(0, 0, 8),
    field(DIPLOMACY_STATUS, 8, 8),
    field(TARGET_COUNTRY, 16, 4),
    field(ATTACHE_LINK, 20, 8),
    field(AIR_BASE_ACCESS, 28, 8),
    field(MILITARY_ACCESS, 36, 8),
    field(DOCKING_RIGHTS, 44, 8),
];
const LINK_BYTES: usize = 52;
const LINK_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&LINK_FIELDS, LINK_BYTES).expect("valid link layout");

/// The owned relation types whose matrix status supplies a link bit.
fn original_candidate(kind: u32) -> bool {
    matches!(
        kind,
        cattache_relation::TYPE
            | cair_base_access_relation::TYPE
            | coffer_air_base_access_relation::TYPE
            | cmilitary_access_relation::TYPE
            | coffer_military_access_relation::TYPE
            | cdocking_rights_relation::TYPE
            | coffer_docking_rights_relation::TYPE
    )
}

/// A matrix cell pointer.
const CELL_FIELDS: [RecordField; 1] = [field(0, 0, 8)];
const CELL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CELL_FIELDS, 8).expect("valid matrix cell layout");

/// A candidate status's link fields ([`LINK_FIELDS`]) and its transport
/// subject pointer, which the walk reads next.
const CANDIDATE_FIELDS: [RecordField; 8] = [
    LINK_FIELDS[0],
    LINK_FIELDS[1],
    LINK_FIELDS[2],
    LINK_FIELDS[3],
    LINK_FIELDS[4],
    LINK_FIELDS[5],
    LINK_FIELDS[6],
    field(SUBJECT_RELATION, LINK_BYTES, 8),
];
const CANDIDATE_BYTES: usize = LINK_BYTES + 8;
const CANDIDATE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CANDIDATE_FIELDS, CANDIDATE_BYTES).expect("valid candidate layout");

/// One batch of owned relations' distinct political candidates: each
/// target slot, its matrix status, that status's link fields and its
/// transport subject pointer.
struct Candidates {
    len: usize,
    others: [u32; BATCH],
    statuses: [usize; BATCH],
    records: [u8; BATCH * CANDIDATE_BYTES],
}

impl Candidates {
    fn new() -> Self {
        Self {
            len: 0,
            others: [0; BATCH],
            statuses: [0; BATCH],
            records: [0; BATCH * CANDIDATE_BYTES],
        }
    }

    /// The gathered status, link fields and transport subject pointer of
    /// target slot `other`.
    fn find(&self, other: u32) -> Option<(usize, [u8; LINK_BYTES], u64)> {
        let at = self.others[..self.len]
            .iter()
            .position(|&slot| slot == other)?;
        let record = &self.records[at * CANDIDATE_BYTES..][..CANDIDATE_BYTES];
        Some((
            self.statuses[at],
            record[..LINK_BYTES].try_into().unwrap(),
            u64::from_le_bytes(record[LINK_BYTES..].try_into().unwrap()),
        ))
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the serial walk, which gathers no candidate ahead.
    static SERIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: `walk` as the serial walk makes it, with no candidate gathered or
/// resolved ahead ([`gather_candidates`]).
#[cfg(test)]
pub(crate) fn serially<T>(walk: impl FnOnce() -> T) -> T {
    SERIAL.set(true);
    let result = walk();
    SERIAL.set(false);
    result
}

/// The relaxed read order (owner decision 2026-10-12): the distinct
/// political candidates of one batch of owned `records` (the targets the
/// walk resolves them to, as it does), then every candidate's matrix cell in
/// one guarded gather, then every cell's status link fields and transport
/// subject pointer in another,
/// read before the walk visits them, a duplicate or skipped candidate
/// included. The walk then takes each candidate's status and fields from
/// `candidates` and applies its own checks in its own order, so healthy
/// memory gives its rows and refusals. Anything that does not complete (a
/// resolution, the matrix header, a cell, a null cell, a status) leaves
/// `candidates` empty and returns false: the walk then reads each
/// candidate itself, once, as the serial walk does, which gives its result
/// for a fault that persists and refuses no more for one that does not.
#[allow(clippy::too_many_arguments)]
fn gather_candidates(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    resolver: &mut Resolver,
    country: ccountry::Country,
    diplomacy: usize,
    matrix: &mut Option<Result<Array, &'static str>>,
    records: &[u8],
    candidates: &mut Candidates,
) -> bool {
    candidates.len = 0;
    #[cfg(test)]
    if SERIAL.get() {
        return true;
    }
    let mut gathered = || {
        for record in records.chunks_exact(OWNED_BYTES) {
            let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
            if !original_candidate(word(0)) {
                continue;
            }
            let a = resolver.resolve(countries, word(4)).ok()?.canonical_slot;
            let b = resolver.resolve(countries, word(8)).ok()?.canonical_slot;
            let other = if a == country.canonical_slot {
                b
            } else if b == country.canonical_slot {
                a
            } else {
                return None;
            };
            if other != country.canonical_slot
                && !candidates.others[..candidates.len].contains(&other)
            {
                candidates.others[candidates.len] = other;
                candidates.len += 1;
            }
        }
        let count = candidates.len;
        if count == 0 {
            return Some(());
        }
        let array = match *matrix {
            Some(result) => result.ok()?,
            None => {
                let array = diplomacy
                    .checked_add(cdiplomacy_status::RELATIONS)
                    .and_then(|address| Array::read(memory, address, 8))?;
                *matrix = Some(Ok(array));
                array
            }
        };
        let mut cells = [0_usize; BATCH];
        for (cell, &other) in cells.iter_mut().zip(&candidates.others[..count]) {
            *cell = array.element(other, 8)?;
        }
        let mut words = [0_u8; BATCH * 8];
        if memory.copy_records(
            RecordSources::Pointers(&cells[..count]),
            &CELL_LAYOUT,
            &mut words[..count * 8],
        ) != count
        {
            return None;
        }
        for (status, word) in candidates.statuses[..count]
            .iter_mut()
            .zip(words[..count * 8].chunks_exact(8))
        {
            *status = usize::try_from(u64::from_le_bytes(word.try_into().unwrap()))
                .ok()
                .filter(|&pointer| pointer != 0)?;
        }
        (memory.copy_records(
            RecordSources::Pointers(&candidates.statuses[..count]),
            &CANDIDATE_LAYOUT,
            &mut candidates.records[..count * CANDIDATE_BYTES],
        ) == count * CANDIDATE_FIELDS.len())
        .then_some(())
    };
    if gathered().is_some() {
        return true;
    }
    candidates.len = 0;
    false
}

/// Visits every raw u32 link of one vector in native order, in guarded batches.
fn for_each_link(
    memory: &impl Memory,
    vector: Array,
    mut visit: impl FnMut(u32, u32) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0_u8; BATCH * 4];
    let mut first = 0;
    while first < vector.count {
        let count = (vector.count - first).min(BATCH as u32) as usize;
        let address = vector
            .element(first, 4)
            .ok_or("diplomacy country vector address overflow")?;
        if !read_slice(memory, address, &mut bytes[..count * 4]) {
            return Err("diplomacy country vector unreadable");
        }
        for (index, word) in bytes[..count * 4].chunks_exact(4).enumerate() {
            visit(
                first + index as u32,
                u32::from_le_bytes(word.try_into().unwrap()),
            )?;
        }
        first += count as u32;
    }
    Ok(())
}

/// Presence of attache, air-base access, military access and docking-rights
/// slots supplies fog, province and naval-base visibility predicates. The same
/// pass records occupation rules and enemy sets where derivation is unavailable.
/// Only the corresponding relation links set these slots, and both endpoints
/// own each relation, so sparse owned vectors name every potentially set status.
/// Emit only set slots without walking the dense matrix.
/// Batches use stack storage and the caller resolver. Worker target bitmaps
/// follow acquired country extent, set on emitted rows and clear after each
/// country. A short output counts each relation naming the missing pair, so
/// unread demand can overstate required growth but cannot understate it.
pub fn links(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LinkRow],
    wars: WarOutput<'_>,
    resolver: &mut Resolver,
) -> LinkWalk {
    links_observed_inner(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        wars.into(),
        resolver,
        None,
        None,
    )
}

/// Same authenticated diplomacy pass, with complete ordered +98 occurrences
/// for every authenticated positive receiver and actual reserved receiver.
/// Old set-only callers stay unchanged.
pub fn links_observed_current(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LinkRow],
    wars: WarOutput<'_>,
    resolver: &mut Resolver,
    hour: i32,
) -> LinkWalk {
    links_observed_inner(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        wars.into(),
        resolver,
        Some(hour),
        None,
    )
}

/// Same original pass, with subject shared-vision inputs and one owner
/// certificate. Subject application and clearing install/remove +288 through
/// the same owned relation lifetime as relation replacement. The installing
/// setter, used by application helpers and clearing, removes the old
/// +D0/+20/+E8 memberships before first-owner destruction, and registers a
/// replacement in +D0 and owner +20. Reset calls each subject's vf22 before
/// directly clearing +288; destruction calls that reset. Loading constructs
/// through the relation factory, reads the relation, then initialization unless
/// duplicated dispatches vf21 (subject application), so loading uses the same
/// installer. No status/subject pointer or seen bitmap crosses this callback;
/// Resolver::begin retires prior country/diplomacy identities. Dates and the
/// retired byte never enter the shared-vision predicates. These source paths do
/// not independently prove the collector boundary's thread quiescence.
pub fn links_observed_transport(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LinkRow],
    wars: WarOutput<'_>,
    resolver: &mut Resolver,
    sample_hour: i32,
) -> LinkWalk {
    links_observed_inner(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        wars.into(),
        resolver,
        Some(sample_hour),
        Some(sample_hour),
    )
}

/// Same original transport pass with worker-owned storage; native
/// observations are never restarted.
pub fn links_observed_transport_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<LinkRow>,
    wars: WarBuffers<'_>,
    resolver: &mut Resolver,
    sample_hour: i32,
) -> LinkWalk {
    links_observed_inner(
        memory,
        base,
        game_state,
        Buffer::Growing(output),
        wars,
        resolver,
        Some(sample_hour),
        Some(sample_hour),
    )
}

#[allow(clippy::too_many_arguments)]
fn links_observed_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, LinkRow>,
    mut wars: WarBuffers<'_>,
    resolver: &mut Resolver,
    ordered_hour: Option<i32>,
    transport_hour: Option<i32>,
) -> LinkWalk {
    let mut countries_len = 0_usize;
    let mut required_countries = 0_usize;
    let mut ordered_refusals = 0_u32;
    // Read each receiver's original enemy vector. Its first required-read
    // failure refuses wars without stopping relation links.
    let mut war_failure = None;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let status_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        resolver.begin(&countries);
        if output.is_growing() {
            resolver
                .grow()
                .map_err(|_| "relation resolver allocation failed")?;
        }
        wars.rows.ensure(countries.count() as usize)?;
        let mut slots = [0_u8; BATCH * 8];
        let mut pointers = [0_usize; BATCH];
        let mut records = [0_u8; BATCH * OWNED_BYTES];
        let mut candidates = Candidates::new();
        // The targets of this country's rows in `output[start..len]`.
        let mut emitted = target_words(countries.count())?;
        // Independent suffix observations must neither suppress original
        // link-field attempts nor retry their own failed subject attempt.
        let mut subject_seen = target_words(countries.count())?;
        let (mut len, mut unread) = (0_usize, 0_u32);
        let (mut war_len, mut war_unread) = (0, 0_u32);
        let diplomacy_vtable = base
            .checked_add(cdiplomacy_status::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut members = [None::<ccountry::Country>; BATCH];
        let mut country_pointers = [0_usize; BATCH];
        let mut country_records = [0_u8; BATCH * COUNTRY_BYTES];
        let mut diplomacy_pointers = [0_usize; BATCH];
        let mut diplomacy_records = [0_u8; BATCH * DIPLOMACY_BYTES];
        // Actual slot zero uses the same reader's original/cached reserved
        // observation. Positive aliases never substitute for this receiver.
        // Its optional suffix cannot turn a reserved failure into a failure
        // of the established positive-country relation-link pass.
        if let Some(hour) = ordered_hour {
            let source = countries.reserved().and_then(|country| {
                let diplomacy = diplomacy_root(memory, base, country)?;
                let rule = country
                    .pointer
                    .checked_add(ccountry::CAN_OCCUPY_NON_WAR)
                    .and_then(|address| read::<u8>(memory, address))
                    .ok_or("selected reserved occupation rule unreadable")?;
                Ok((country, diplomacy, rule))
            });
            match source {
                Err(_) => {
                    ordered_refusals += 1;
                }
                Ok((country, diplomacy, rule)) => {
                    let array = diplomacy
                        .checked_add(cdiplomacy_status::ENEMY_COUNTRIES)
                        .and_then(|address| Array::read(memory, address, 4))
                        .ok_or("ordered reserved enemy vector unreadable or invalid");
                    let count = array.as_ref().map_or(0, |array| array.count as usize);
                    required_countries = required_countries
                        .checked_add(count)
                        .ok_or("war link count overflow")?;
                    wars.countries.ensure(required_countries)?;
                    if war_len == wars.rows.len() {
                        ordered_refusals += 1;
                        war_unread += 1;
                    } else {
                        let span = array.and_then(|array| {
                            if count > wars.countries.len() {
                                return Err("ordered enemy arena exhausted");
                            }
                            for_each_link(memory, array, |index, raw| {
                                wars.countries[index as usize] = raw;
                                Ok(())
                            })?;
                            Ok((0, count))
                        });
                        if span.is_err() {
                            ordered_refusals += 1;
                        }
                        if span.is_ok() {
                            countries_len = count;
                        }
                        wars.rows[war_len] = WarRow {
                            country: country.id,
                            can_occupy_non_war: rule,
                            ordered: span.ok(),
                            ordered_hour: Some(hour),
                            ordered_failure: span.err(),
                            ..Default::default()
                        };
                        war_len += 1;
                    }
                }
            }
        }
        let mut next = 1;
        while next < countries.count() {
            // Each batch of countries: one guarded copy of their fields, one
            // of their diplomacy statuses' identity and owned-relation header.
            let end = countries.count().min(next + BATCH as u32);
            let mut present = 0;
            for slot in next..end {
                if let Some(country) = countries.at(slot)? {
                    members[present] = Some(country);
                    country_pointers[present] = country.pointer;
                    present += 1;
                }
            }
            next = end;
            if memory.copy_records(
                RecordSources::Pointers(&country_pointers[..present]),
                &COUNTRY_LAYOUT,
                &mut country_records[..present * COUNTRY_BYTES],
            ) < present * COUNTRY_FIELDS.len()
            {
                return Err("country diplomacy, occupation rule or civil-war link unreadable");
            }
            for (pointer, record) in diplomacy_pointers
                .iter_mut()
                .zip(country_records[..present * COUNTRY_BYTES].chunks_exact(COUNTRY_BYTES))
            {
                *pointer = usize::try_from(u64::from_le_bytes(record[..8].try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("country diplomacy is null")?;
            }
            if memory.copy_records(
                RecordSources::Pointers(&diplomacy_pointers[..present]),
                &DIPLOMACY_LAYOUT,
                &mut diplomacy_records[..present * DIPLOMACY_BYTES],
            ) < present * DIPLOMACY_FIELDS.len()
            {
                return Err("country diplomacy unreadable");
            }
            // Each owner's first owned-relation pointers and its enemy
            // vector header, which its pass reads one owner at a time: their
            // misses overlap the batch. Hints read nothing, so rows and the
            // first fault are unchanged.
            for (record, &diplomacy) in diplomacy_records[..present * DIPLOMACY_BYTES]
                .chunks_exact(DIPLOMACY_BYTES)
                .zip(&diplomacy_pointers[..present])
            {
                if let Some(owned) = Array::from_header(record[16..32].try_into().unwrap(), 8) {
                    memory.prefetch(owned.data, owned.count.min(BATCH as u32) as usize * 8);
                }
                memory.prefetch(
                    diplomacy.wrapping_add(cdiplomacy_status::ENEMY_COUNTRIES),
                    16,
                );
            }
            for index in 0..present {
                let country = members[index].expect("filled for every present country");
                let fields = &country_records[index * COUNTRY_BYTES..][..COUNTRY_BYTES];
                let record = &diplomacy_records[index * DIPLOMACY_BYTES..][..DIPLOMACY_BYTES];
                let diplomacy = diplomacy_pointers[index];
                if u64::from_le_bytes(record[..8].try_into().unwrap()) != diplomacy_vtable {
                    return Err("diplomacy vtable mismatch");
                }
                if u64::from_le_bytes(record[8..16].try_into().unwrap()) != country.pointer as u64 {
                    return Err("diplomacy country mismatch");
                }
                let owned = Array::from_header(record[16..32].try_into().unwrap(), 8)
                    .ok_or("owned relation vector unreadable or invalid")?;
                // The matrix header is read lazily, once per owner.
                let mut matrix: Option<Result<Array, &'static str>> = None;
                // Owned relations and the owner suffix are the complete row
                // demand.
                output.ensure(
                    len.checked_add(owned.count as usize)
                        .and_then(|n| n.checked_add(usize::from(transport_hour.is_some())))
                        .ok_or("relation link demand overflow")?,
                )?;
                let start = len;
                if let Some(sample_hour) = transport_hour {
                    if let Some(row) = output.get_mut(len) {
                        *row = LinkRow {
                            country: country.id,
                            target: country.canonical_slot,
                            links: 0,
                            transport_subject: Some(Ok(TransportSubject {
                                sample_hour,
                                complete_owner: true,
                                ..Default::default()
                            })),
                        };
                        len += 1;
                    } else {
                        unread = unread
                            .checked_add(1)
                            .ok_or("transport subject owner count overflow")?;
                    }
                }
                let mut retired_war = false;
                let mut subject_domain_failed = false;
                let mut first = 0;
                while first < owned.count {
                    let count = (owned.count - first).min(BATCH as u32) as usize;
                    let address = owned
                        .element(first, 8)
                        .ok_or("owned relation address overflow")?;
                    if !read_slice(memory, address, &mut slots[..count * 8]) {
                        return Err("owned relation pointers unreadable");
                    }
                    for (pointer, chunk) in
                        pointers.iter_mut().zip(slots[..count * 8].chunks_exact(8))
                    {
                        *pointer = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap()))
                            .ok()
                            .filter(|&pointer| pointer != 0)
                            .ok_or("owned relation pointer null or invalid")?;
                    }
                    if memory.copy_records(
                        RecordSources::Pointers(&pointers[..count]),
                        &OWNED_LAYOUT,
                        &mut records[..count * OWNED_BYTES],
                    ) < count * OWNED_FIELDS.len()
                    {
                        return Err("owned relation fields unreadable");
                    }
                    // The batch's political candidates' matrix cells, then
                    // their statuses, each in one guarded gather; the walk
                    // below takes them instead of reading each candidate's.
                    if !gather_candidates(
                        memory,
                        &countries,
                        resolver,
                        country,
                        diplomacy,
                        &mut matrix,
                        &records[..count * OWNED_BYTES],
                        &mut candidates,
                    ) {
                        crate::batch_fallback::record(crate::batch_fallback::Batch::RelationLinks);
                    }
                    for record in records[..count * OWNED_BYTES].chunks_exact(OWNED_BYTES) {
                        let word =
                            |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
                        if word(0) == cwar_relation::TYPE && record[12] != 0 {
                            retired_war = true;
                        }
                        let original_candidate = original_candidate(word(0));
                        if !original_candidate {
                            if let Some(sample_hour) = transport_hour
                                && !subject_domain_failed
                                && word(0) == csubject_relation::TYPE_VALUE
                            {
                                let observed = transport_access::candidate(
                                    memory,
                                    base,
                                    &countries,
                                    resolver,
                                    transport_access::Candidate {
                                        country,
                                        diplomacy,
                                        matrix: &mut matrix,
                                        parties: [word(4), word(8)],
                                        seen: &mut subject_seen,
                                    },
                                );
                                match observed {
                                    Ok(Some((other, status, target))) => {
                                        let transport_subject =
                                            Some(
                                                transport_access::capture(
                                                    memory,
                                                    base,
                                                    status,
                                                    target,
                                                    |raw| {
                                                        resolver
                                                            .resolve(&countries, raw)
                                                            .map(|value| value.canonical_slot)
                                                    },
                                                )
                                                .map(|subject| TransportSubject {
                                                    sample_hour,
                                                    complete_owner: false,
                                                    target_raw: Some(target),
                                                    subject: Some(subject),
                                                }),
                                            );
                                        if len == output.len() {
                                            unread = unread
                                                .checked_add(1)
                                                .ok_or("relation link count overflow")?;
                                        } else {
                                            output[len] = LinkRow {
                                                country: country.id,
                                                target: other,
                                                links: 0,
                                                transport_subject,
                                            };
                                            len += 1;
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(reason) => {
                                        subject_domain_failed = true;
                                        if let Some(row) = output.get_mut(start)
                                            && row.country == country.id
                                            && row.target == country.canonical_slot
                                        {
                                            row.transport_subject = Some(Err(reason));
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        let a = resolver.resolve(&countries, word(4))?.canonical_slot;
                        let b = resolver.resolve(&countries, word(8))?.canonical_slot;
                        let other = if a == country.canonical_slot {
                            b
                        } else if b == country.canonical_slot {
                            a
                        } else {
                            return Err("owned relation does not include its owner");
                        };
                        let bit = 1_u64 << (other % 64);
                        let word = emitted
                            .get(other as usize / 64)
                            .ok_or("relation endpoint beyond the engine's country slots")?;
                        if other == country.canonical_slot || word & bit != 0 {
                            continue;
                        }
                        let (status, record, subject) = match candidates.find(other) {
                            Some((status, record, subject)) => (status, record, Some(subject)),
                            None => {
                                let matrix = match matrix {
                                    Some(result) => result?,
                                    None => {
                                        (*matrix.insert(
                                            diplomacy
                                                .checked_add(cdiplomacy_status::RELATIONS)
                                                .and_then(|address| Array::read(memory, address, 8))
                                                .ok_or("relation matrix unreadable or invalid"),
                                        ))?
                                    }
                                };
                                let status = matrix
                                    .element(other, 8)
                                    .and_then(|address| read::<u64>(memory, address))
                                    .and_then(|pointer| usize::try_from(pointer).ok())
                                    .filter(|&pointer| pointer != 0)
                                    .ok_or("relation status missing from matrix")?;
                                let mut record = [0_u8; LINK_BYTES];
                                if memory.copy_records(
                                    RecordSources::Pointers(&[status]),
                                    &LINK_LAYOUT,
                                    &mut record,
                                ) < LINK_FIELDS.len()
                                {
                                    return Err("relation status link fields unreadable");
                                }
                                (status, record, None)
                            }
                        };
                        let word =
                            |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
                        if word(0) != status_vtable {
                            return Err("relation status vtable mismatch");
                        }
                        if word(8) != diplomacy as u64 {
                            return Err("relation status diplomacy owner mismatch");
                        }
                        let target = u32::from_le_bytes(record[16..20].try_into().unwrap());
                        if resolver.resolve(&countries, target)?.canonical_slot != other {
                            return Err("relation status target differs from matrix slot");
                        }
                        let links = (u8::from(word(20) != 0) * LINK_ATTACHE)
                            | (u8::from(word(28) != 0) * LINK_AIR_BASE_ACCESS)
                            | (u8::from(word(36) != 0) * LINK_MILITARY_ACCESS)
                            | (u8::from(word(44) != 0) * LINK_DOCKING_RIGHTS);
                        if links == 0 && transport_hour.is_none() {
                            continue;
                        }
                        let existing = if transport_hour.is_some()
                            && subject_seen[other as usize / 64] & bit != 0
                        {
                            output[start..len]
                                .iter()
                                .position(|row| row.target == other)
                                .map(|at| start + at)
                        } else {
                            None
                        };
                        let transport_subject = if let Some(at) = existing {
                            output[at].transport_subject
                        } else {
                            transport_hour.map(|sample_hour| {
                                subject_seen[other as usize / 64] |= bit;
                                transport_access::capture_with(
                                    memory,
                                    base,
                                    status,
                                    subject,
                                    target,
                                    |raw| {
                                        resolver
                                            .resolve(&countries, raw)
                                            .map(|value| value.canonical_slot)
                                    },
                                )
                                .map(|subject| TransportSubject {
                                    sample_hour,
                                    complete_owner: false,
                                    target_raw: Some(target),
                                    subject: Some(subject),
                                })
                            })
                        };
                        if let Some(at) = existing {
                            output[at].links = links;
                            if links != 0 {
                                emitted[other as usize / 64] |= bit;
                            }
                            continue;
                        }
                        if len == output.len() {
                            unread = unread
                                .checked_add(1)
                                .ok_or("relation link count overflow")?;
                            continue;
                        }
                        output[len] = LinkRow {
                            country: country.id,
                            target: other,
                            links,
                            transport_subject,
                        };
                        len += 1;
                        if links != 0 {
                            emitted[other as usize / 64] |= bit;
                        }
                    }
                    first += count as u32;
                }
                for row in &output[start..len] {
                    emitted[row.target as usize / 64] = 0;
                }
                subject_seen.fill(0);
                let rule = [fields[8]];
                let civil_war: [u8; 4] = fields[9..13].try_into().unwrap();
                let read_enemies = retired_war || u32::from_le_bytes(civil_war) != 0;
                if rule[0] != 0 || read_enemies || ordered_hour.is_some() {
                    // Read this header/payload once. Both legacy exceptional
                    // set consumers and current occurrence consumers borrow it.
                    let mut failure = None;
                    let enemies = if read_enemies || ordered_hour.is_some() {
                        let result = diplomacy
                            .checked_add(cdiplomacy_status::ENEMY_COUNTRIES)
                            .and_then(|address| Array::read(memory, address, 4))
                            .ok_or(if ordered_hour.is_some() {
                                "ordered enemy vector unreadable or invalid"
                            } else {
                                "enemy vector unreadable or invalid"
                            });
                        match result {
                            Ok(value) => Some(value),
                            Err(reason) => {
                                if read_enemies {
                                    war_failure.get_or_insert(reason);
                                }
                                failure = Some(reason);
                                None
                            }
                        }
                    } else {
                        None
                    };
                    let count = enemies.map_or(0, |array| array.count as usize);
                    required_countries = required_countries
                        .checked_add(count)
                        .ok_or("war link count overflow")?;
                    wars.countries.ensure(required_countries)?;
                    let start = countries_len;
                    let fits = start
                        .checked_add(count)
                        .is_some_and(|end| end <= wars.countries.len());
                    if enemies.is_some() && !fits {
                        failure = Some("ordered enemy arena exhausted");
                    }
                    if failure.is_none()
                        && war_len < wars.rows.len()
                        && let Some(array) = enemies
                    {
                        let result = for_each_link(memory, array, |index, raw| {
                            wars.countries[start + index as usize] = raw;
                            Ok(())
                        });
                        if let Err(reason) = result {
                            if read_enemies {
                                war_failure.get_or_insert(reason);
                            }
                            failure = Some(reason);
                        }
                    }
                    if ordered_hour.is_some() && (failure.is_some() || war_len == wars.rows.len()) {
                        ordered_refusals = ordered_refusals
                            .checked_add(1)
                            .ok_or("ordered enemy refusal count overflow")?;
                    }
                    if war_len == wars.rows.len() || (read_enemies && !fits) {
                        war_unread = war_unread
                            .checked_add(1)
                            .ok_or("country war unread row count overflow")?;
                        continue;
                    }
                    if read_enemies && failure.is_some() {
                        continue;
                    }
                    if failure.is_none() {
                        countries_len = start + count;
                    }
                    wars.rows[war_len] = WarRow {
                        country: country.id,
                        can_occupy_non_war: rule[0],
                        enemies_read: read_enemies,
                        enemies_start: if read_enemies { start } else { 0 },
                        enemies_len: if read_enemies { count } else { 0 },
                        ordered: ordered_hour
                            .filter(|_| failure.is_none())
                            .map(|_| (start, count)),
                        ordered_hour,
                        ordered_failure: failure,
                    };
                    war_len += 1;
                }
            }
        }
        Ok((
            outcome(len, unread),
            war_failure.map_or_else(
                || outcome(war_len, war_unread),
                |reason| Outcome::Unavailable { reason },
            ),
        ))
    })();
    match result {
        Ok((links, wars)) => LinkWalk {
            links,
            wars,
            countries_len,
            required_countries,
            ordered_refusals,
        },
        Err(reason) => LinkWalk {
            links: Outcome::Unavailable { reason },
            wars: Outcome::Unavailable {
                reason: war_failure.unwrap_or(reason),
            },
            countries_len: 0,
            required_countries,
            ordered_refusals,
        },
    }
}

/// One directed country pair whose cached opinion dword is nonzero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OpinionRow {
    /// Raw reference of the country whose relation matrix holds this record.
    pub owner_country: u32,
    /// Raw reference stored at CRelationStatus +0x50, not the matrix index.
    pub target_country: u32,
    /// Native signed dword at +0x300, retained without scaling.
    pub opinion: i32,
}

/// One typed relation object serialized under a directed country pair.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TupleRow {
    pub owner_country: u32,
    pub target_country: u32,
    /// Native CRelation type word at +0x08, which is also the serialized key token.
    pub type_id: u32,
    /// Native index in the serializable relation vector at +0xe8, counting the
    /// CWarRelation and CSubjectRelation elements that this table skips.
    pub ordinal: u32,
    /// Raw CRelation +0x10 reference, retained even when it is an alias of the owner.
    pub first_country: u32,
    /// Raw CRelation +0x14 reference, retained even when it is an alias of the target.
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    /// Raw engine byte, retained without boolean normalization.
    pub cancel: u8,
    /// Raw engine byte, retained without boolean normalization.
    pub retired: u8,
}

/// Independent outcomes for the two tables one matrix pass produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub opinions: Outcome,
    pub tuples: Outcome,
}

pub const BATCH: usize = 64;
/// Relation identity-authentication spans for vtable and type.
const TUPLE_AUTH_BYTES: usize = 12;
/// Exactly the vtable and type word the tuple skip decision needs from one relation.
const TUPLE_AUTH_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: crelation::TYPE_ID,
        destination_offset: 8,
        len: 4,
    },
];
const TUPLE_AUTH_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TUPLE_AUTH_FIELDS, TUPLE_AUTH_BYTES).expect("valid record layout");
/// Relation payload spans for endpoints, dates and cancellation.
const TUPLE_REST_BYTES: usize = 18;
/// Exactly the payload spans the tuple rows need from one non-skipped relation.
const TUPLE_REST_FIELDS: [RecordField; 6] = [
    RecordField {
        source_offset: crelation::FIRST_COUNTRY,
        destination_offset: 0,
        len: 4,
    },
    RecordField {
        source_offset: crelation::SECOND_COUNTRY,
        destination_offset: 4,
        len: 4,
    },
    RecordField {
        source_offset: crelation::START_HOUR,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: crelation::END_HOUR,
        destination_offset: 12,
        len: 4,
    },
    RecordField {
        source_offset: crelation::CANCEL,
        destination_offset: 16,
        len: 1,
    },
    RecordField {
        source_offset: crelation::RETIRED,
        destination_offset: 17,
        len: 1,
    },
];
const TUPLE_REST_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TUPLE_REST_FIELDS, TUPLE_REST_BYTES).expect("valid record layout");

/// A marked status: identity, owning diplomacy, target and opinion.
const MARK_FIELDS: [RecordField; 4] = [
    field(0, 0, 8),
    field(DIPLOMACY_STATUS, 8, 8),
    field(TARGET_COUNTRY, 16, 4),
    field(OPINION, 20, 4),
];
const MARK_BYTES: usize = 24;
const MARK_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&MARK_FIELDS, MARK_BYTES).expect("valid mark layout");
/// One matrix entry, read at its own address.
const POINTER_FIELDS: [RecordField; 1] = [field(0, 0, 8)];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELDS, 8).expect("valid pointer layout");
/// Identity, target and opinion of a status not yet authenticated.
const AUTH_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(TARGET_COUNTRY, 8, 4),
    field(OPINION, 12, 4),
];
const AUTH_BYTES: usize = 16;
const AUTH_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&AUTH_FIELDS, AUTH_BYTES).expect("valid authentication layout");
/// A tuple-only pass authenticates without the opinion.
const TARGET_FIELDS: [RecordField; 2] = [AUTH_FIELDS[0], AUTH_FIELDS[1]];
const TARGET_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TARGET_FIELDS, AUTH_BYTES).expect("valid target layout");
const SERIALIZABLE_FIELDS: [RecordField; 1] = [field(SERIALIZABLE_RELATIONS, 0, 16)];
const SERIALIZABLE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SERIALIZABLE_FIELDS, 16).expect("valid serializable layout");

/// Country-link and direct-war queries use signed dword slots. Province
/// controller storage does not bound the domain. Preparation uses original
/// acquired extent, including holes.
fn target_words(count: u32) -> Result<Vec<u64>, &'static str> {
    crate::allocation::filled((count as usize).div_ceil(u64::BITS as usize), 0)
        .map_err(|_| "relation target set allocation failed")
}

/// Emits nonzero cached opinions and serializable typed relations in stored
/// vector order. Matrix serialization starts at one, leaving reserved slot
/// zero without a row. War and subject elements authenticate and consume their
/// ordinal before being skipped for their own recorded tables.
///
/// Matrix/status identities are written during construction at world reset.
/// When every opinion write since [`Baseline`] was observed, unchanged country,
/// diplomacy and matrix controls allow replaying prior rows with marked statuses
/// reread. Other owners are read in full.
/// Relation installers append owned relations and serialize them only for the
/// first endpoint; removals drop both memberships and reset empties serializable
/// lists. First-endpoint owned relations therefore name all serializing statuses.
/// Refuse RelationTuples if their counts disagree, independently of Opinions.
/// A refused walk or an unauthenticated mark forgets the baseline so the next
/// walk reads all statuses. Worker target storage follows original extent
/// without calling the game.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    opinions: &mut [OpinionRow],
    tuples: &mut [TupleRow],
    active: [bool; 2],
) -> Walk {
    walk_with(
        memory,
        base,
        game_state,
        opinions,
        tuples,
        active,
        &mut Resolver::default(),
        &mut Baseline::default(),
        None,
        |_| true,
    )
}

/// `walk` with caller-owned resolution storage and opinion baseline. Rows,
/// order and outcomes are identical for every storage size and observation;
/// they only remove repeated reads.
///
/// `select` names the owners (native country IDs) this walk reads: the
/// relations are read once a day, one owner slice per hour
/// (`replay::cadence::Cadence::Sliced`). An unselected owner is not read and
/// has no rows; every country still counts toward the matrix bound check, and
/// the baseline keeps an unselected owner's rows and marks for its own slice.
#[allow(clippy::too_many_arguments)]
pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    opinions: &mut [OpinionRow],
    tuples: &mut [TupleRow],
    active: [bool; 2],
    resolved: &mut Resolver,
    baseline: &mut Baseline,
    observed: Option<Observation<'_>>,
    select: impl Fn(u32) -> bool,
) -> Walk {
    walk_buffers(
        memory,
        base,
        game_state,
        Buffer::Fixed(opinions),
        Buffer::Fixed(tuples),
        active,
        resolved,
        baseline,
        observed,
        select,
    )
}

/// Worker output grows at each original nonzero opinion or serializable
/// occurrence without another read.
#[allow(clippy::too_many_arguments)]
pub fn walk_growing_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    opinions: &mut Vec<OpinionRow>,
    tuples: &mut Vec<TupleRow>,
    active: [bool; 2],
    resolved: &mut Resolver,
    baseline: &mut Baseline,
    observed: Option<Observation<'_>>,
    select: impl Fn(u32) -> bool,
) -> Walk {
    walk_buffers(
        memory,
        base,
        game_state,
        Buffer::Growing(opinions),
        Buffer::Growing(tuples),
        active,
        resolved,
        baseline,
        observed,
        select,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_buffers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut opinions: Buffer<'_, OpinionRow>,
    mut tuples: Buffer<'_, TupleRow>,
    active: [bool; 2],
    resolved: &mut Resolver,
    baseline: &mut Baseline,
    observed: Option<Observation<'_>>,
    select: impl Fn(u32) -> bool,
) -> Walk {
    if !active.into_iter().any(|active| active) {
        return Walk {
            opinions: Outcome::Rows { len: 0 },
            tuples: Outcome::Rows { len: 0 },
        };
    }
    let mut targets = Vec::new();
    // Opinions never read the owned vectors: a failure there refuses
    // RelationTuples alone, and the rest of the walk reads opinions only.
    let mut owned_failure: Option<&'static str> = None;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let status_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut highest_slot = 0;
        for slot in 1..countries.count() {
            if countries.at(slot)?.is_some() {
                highest_slot = slot;
            }
        }
        resolved.begin(&countries);
        let scope = (base, game_state);
        let replaying = active[0]
            && baseline.prepare(
                memory,
                scope,
                &countries,
                status_vtable,
                resolved,
                observed,
                &select,
            );
        baseline.begin(scope, active[0] && observed.is_some(), countries.count());
        let mut counters = Counters::default();
        let mut found_country = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                if active[0] {
                    baseline.empty(slot);
                }
                continue;
            };
            found_country = true;
            if !select(country.id) {
                if active[0] {
                    baseline.carry(slot, country.pointer);
                }
                continue;
            }
            let owner = read_owner(memory, base, country, active[1] && owned_failure.is_none())?;
            if active[1] && owned_failure.is_none() {
                let words = (owner.matrix.count as usize).div_ceil(u64::BITS as usize);
                crate::allocation::resize(&mut targets, words, 0)
                    .map_err(|_| "relation target set allocation failed")?;
            }
            if owner.matrix.count <= highest_slot {
                return Err("relation matrix does not cover country ordinals");
            }
            let expected = match owner.owned {
                None => None,
                Some(owned) => match owned.and_then(|owned| {
                    owned_targets(
                        memory,
                        &countries,
                        country,
                        owned,
                        owner.matrix,
                        &mut targets,
                    )
                }) {
                    Ok(expected) => Some(expected),
                    Err(reason) if active[0] => {
                        owned_failure = Some(reason);
                        targets.fill(0);
                        None
                    }
                    Err(reason) => return Err(reason),
                },
            };
            let identity = Identity {
                country: country.pointer,
                id: country.id,
                diplomacy: owner.diplomacy,
                matrix: owner.matrix,
            };
            let replay = replaying && baseline.unchanged(slot, identity);
            if active[0] {
                baseline.visit(slot, identity);
            }
            let pass = Pass {
                memory,
                base,
                countries: &countries,
                country,
                owner,
                status_vtable,
                active: [active[0] && !replay, expected.is_some()],
            };
            let found = pass.statuses(
                &mut targets,
                baseline,
                resolved,
                &mut counters,
                &mut opinions,
                &mut tuples,
            )?;
            if replay {
                baseline.replay(slot, country.id, &mut counters, &mut opinions)?;
            }
            if active[0] {
                baseline.end_owner(!replay);
            }
            if expected.is_some_and(|expected| found != expected) {
                const DISAGREE: &str = "owned and serializable relations disagree";
                if !active[0] {
                    return Err(DISAGREE);
                }
                owned_failure = Some(DISAGREE);
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(counters.walk())
    })();
    let mut walk = result.unwrap_or_else(|reason| Walk {
        opinions: if active[0] {
            Outcome::Unavailable { reason }
        } else {
            Outcome::Rows { len: 0 }
        },
        tuples: if active[1] {
            Outcome::Unavailable { reason }
        } else {
            Outcome::Rows { len: 0 }
        },
    });
    if let Some(reason) = owned_failure {
        walk.tuples = Outcome::Unavailable { reason };
    }
    if active[0] {
        baseline.finish(
            !matches!(walk.opinions, Outcome::Unavailable { .. }),
            observed.map(|observed| observed.token),
        );
    }
    walk
}

/// One owner's authenticated diplomacy and the two vectors the walk reads.
/// `owned` is read only for tuples, and its failure refuses tuples alone.
#[derive(Clone, Copy)]
struct Owner {
    diplomacy: usize,
    matrix: Array,
    owned: Option<Result<Array, &'static str>>,
}

/// The diplomacy identity, matrix header and (for tuples) owned-vector header
/// in one guarded gather: the three headers share the object's first line.
fn read_owner(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    owned: bool,
) -> Result<Owner, &'static str> {
    let diplomacy = country
        .pointer
        .checked_add(ccountry::DIPLOMACY_STATUS)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("country diplomacy unreadable")?;
    if diplomacy == 0 {
        return Err("country diplomacy is null");
    }
    let at = |offset| {
        diplomacy
            .checked_add(offset)
            .ok_or("country diplomacy unreadable")
    };
    let (mut vtable, mut backlink) = ([0_u8; 8], [0_u8; 8]);
    let (mut matrix, mut vector) = ([0_u8; 16], [0_u8; 16]);
    let copied = memory.copy_scattered(
        [
            diplomacy,
            at(cdiplomacy_status::COUNTRY)?,
            at(cdiplomacy_status::RELATIONS)?,
            at(cdiplomacy_status::OWNED_RELATIONS)?,
        ],
        &mut [
            &mut vtable,
            &mut backlink,
            &mut matrix,
            &mut vector[..if owned { 16 } else { 0 }],
        ],
    );
    if copied < 2 {
        return Err("country diplomacy unreadable");
    }
    let expected = base
        .checked_add(cdiplomacy_status::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(vtable) != expected as u64 {
        return Err("diplomacy vtable mismatch");
    }
    if u64::from_le_bytes(backlink) != country.pointer as u64 {
        return Err("diplomacy country mismatch");
    }
    let matrix = (copied > 2)
        .then(|| Array::from_header(matrix, 8))
        .flatten()
        .ok_or("relation matrix unreadable or invalid")?;
    let owned = owned.then(|| {
        (copied > 3)
            .then(|| Array::from_header(vector, 8))
            .flatten()
            .ok_or("owned relation vector unreadable or invalid")
    });
    Ok(Owner {
        diplomacy,
        matrix,
        owned,
    })
}

/// Marks the matrix slot of every owned relation whose first endpoint is this
/// owner and returns how many there are. A relation whose second endpoint
/// links to no slot would sit in the reserved slot zero, which no pass reads.
fn owned_targets(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: ccountry::Country,
    owned: Array,
    matrix: Array,
    targets: &mut [u64],
) -> Result<u32, &'static str> {
    let mut slots = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut records = [0_u8; BATCH * OWNED_BYTES];
    let mut expected = 0_u32;
    let mut first = 0;
    while first < owned.count {
        let count = (owned.count - first).min(BATCH as u32) as usize;
        let address = owned
            .element(first, 8)
            .ok_or("owned relation address overflow")?;
        if !read_slice(memory, address, &mut slots[..count * 8]) {
            return Err("owned relation pointers unreadable");
        }
        for (pointer, chunk) in pointers.iter_mut().zip(slots[..count * 8].chunks_exact(8)) {
            *pointer = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap()))
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("owned relation pointer null or invalid")?;
        }
        if memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &OWNED_LAYOUT,
            &mut records[..count * OWNED_BYTES],
        ) < count * OWNED_FIELDS.len()
        {
            return Err("owned relation fields unreadable");
        }
        for record in records[..count * OWNED_BYTES].chunks_exact(OWNED_BYTES) {
            let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
            // Relation replacement compares the first endpoint with the owner's ID
            // and then by country; an alias links to the same slot. An unreadable
            // link refuses the walk rather than shrinking the expected count.
            let owned_first =
                word(4) != 0 && countries.link(word(4))? == Some(country.canonical_slot);
            if !owned_first {
                continue;
            }
            // `canonical_slot` maps a null or unlinked second endpoint to
            // the reserved slot zero, which is never serialized.
            if word(8) == 0 {
                continue;
            }
            let Some(target) = countries.link(word(8))? else {
                continue;
            };
            if target >= matrix.count {
                return Err("relation endpoint outside the relation matrix");
            }
            let word = targets
                .get_mut(target as usize / 64)
                .ok_or("relation endpoint beyond the engine's country slots")?;
            *word |= 1 << (target % 64);
            expected = expected
                .checked_add(1)
                .ok_or("owned relation count overflow")?;
        }
        first += count as u32;
    }
    Ok(expected)
}

/// One owner's pass over its matrix.
struct Pass<'p, 'm, M: Memory> {
    memory: &'m M,
    base: usize,
    countries: &'p ccountry::Reader<'m, M>,
    country: ccountry::Country,
    owner: Owner,
    status_vtable: u64,
    active: [bool; 2],
}

impl<M: Memory> Pass<'_, '_, M> {
    /// Reads the matrix page by page and returns the serializable relations
    /// found in the statuses `targets` names, clearing each target it visits.
    /// Opinions read and authenticate every status and record each row in
    /// `baseline`; a tuple-only pass reads the target pages only.
    fn statuses(
        &self,
        targets: &mut [u64],
        baseline: &mut Baseline,
        resolved: &mut Resolver,
        counters: &mut Counters,
        opinions: &mut Buffer<'_, OpinionRow>,
        tuples: &mut Buffer<'_, TupleRow>,
    ) -> Result<u32, &'static str> {
        let matrix = self.owner.matrix;
        let mut pointers = [0_usize; BATCH];
        let mut subset = [0_usize; BATCH];
        let mut records = [0_u8; BATCH * AUTH_BYTES];
        let mut headers = [0_u8; BATCH * 16];
        let mut found = 0_u32;
        let mut index = 1;
        while index < matrix.count {
            let len = (matrix.count - index).min(BATCH as u32) as usize;
            let mut wanted = 0_u64;
            for offset in 0..len {
                let slot = index as usize + offset;
                if let Some(word) = targets.get_mut(slot / 64)
                    && *word & (1 << (slot % 64)) != 0
                {
                    *word &= !(1 << (slot % 64));
                    wanted |= 1 << offset;
                }
            }
            if !self.active[0] && wanted == 0 {
                index += len as u32;
                continue;
            }
            read_pointers(self.memory, matrix, index, &mut pointers[..len])?;
            // Opinions need every status of the page; tuples only their own.
            let needed = if self.active[0] {
                u64::MAX >> (64 - len)
            } else {
                wanted
            };
            let count = gather(&pointers[..len], needed, &mut subset);
            let (layout, fields) = if self.active[0] {
                (&AUTH_LAYOUT, AUTH_FIELDS.len())
            } else {
                (&TARGET_LAYOUT, TARGET_FIELDS.len())
            };
            let copied = self.memory.copy_records(
                RecordSources::Pointers(&subset[..count]),
                layout,
                &mut records[..count * AUTH_BYTES],
            );
            let wanted_count = gather(&pointers[..len], wanted, &mut subset);
            let headers_copied = if wanted_count == 0 {
                0
            } else {
                self.memory.copy_records(
                    RecordSources::Pointers(&subset[..wanted_count]),
                    &SERIALIZABLE_LAYOUT,
                    &mut headers[..wanted_count * 16],
                )
            };
            let (mut row, mut header) = (0, 0);
            for offset in (0..len).filter(|offset| needed & (1 << offset) != 0) {
                let slot = index + offset as u32;
                // `copy_records` counts completed fields in row-major order.
                if copied < (row + 1) * fields {
                    return Err("relation matrix slot fields unreadable");
                }
                let record = &records[row * AUTH_BYTES..(row + 1) * AUTH_BYTES];
                let tuple = wanted & (1 << offset) != 0;
                if tuple && headers_copied <= header {
                    return Err("relation matrix slot fields unreadable");
                }
                if u64::from_le_bytes(record[..8].try_into().unwrap()) != self.status_vtable {
                    return Err("relation status vtable mismatch");
                }
                let target_country = u32::from_le_bytes(record[8..12].try_into().unwrap());
                if resolved
                    .resolve(self.countries, target_country)?
                    .canonical_slot
                    != slot
                {
                    return Err("relation status target differs from matrix slot");
                }
                let opinion = i32::from_le_bytes(record[12..16].try_into().unwrap());
                if self.active[0] && opinion != 0 {
                    baseline.record(slot, target_country, opinion);
                    counters.push_opinion(
                        opinions,
                        OpinionRow {
                            owner_country: self.country.id,
                            target_country,
                            opinion,
                        },
                    )?;
                }
                if tuple {
                    let serializable = Array::from_header(
                        headers[header * 16..(header + 1) * 16].try_into().unwrap(),
                        8,
                    )
                    .ok_or("serializable relation vector invalid")?;
                    header += 1;
                    found = found
                        .checked_add(serializable.count)
                        .ok_or("serializable relation count overflow")?;
                    read_tuples(
                        self.memory,
                        self.base,
                        self.countries,
                        self.country,
                        target_country,
                        serializable,
                        tuples,
                        counters,
                        resolved,
                    )?;
                }
                row += 1;
            }
            index += len as u32;
        }
        Ok(found)
    }
}

/// Copies the pointers `mask` selects, in order, and returns how many.
fn gather(pointers: &[usize], mask: u64, output: &mut [usize; BATCH]) -> usize {
    let mut count = 0;
    for (offset, &pointer) in pointers.iter().enumerate() {
        if mask & (1 << offset) != 0 {
            output[count] = pointer;
            count += 1;
        }
    }
    count
}

/// Caller-owned dense successful-authentication storage for one relation walk: raw
/// link IDs select reference cells and canonical country slots select diplomacy
/// cells. Store successful results by full raw ID or slot plus country pointer.
/// Reuse is valid only within the stable hourly world and cannot detect later
/// mutation or unreadability within that walk. Live census/link extents bound
/// storage; uncovered IDs use uncached validation and grow off-thread from
/// `required_*`. Generation stamps make `begin` constant-time.
#[derive(Default)]
pub struct Resolver {
    references: Vec<(u32, ccountry::Country)>,
    diplomacies: Vec<(u32, usize, usize)>,
    generation: u32,
    required_links: usize,
    required_slots: usize,
}

const VACANT_COUNTRY: ccountry::Country = ccountry::Country {
    pointer: 0,
    canonical_slot: 0,
    id: 0,
};

impl Resolver {
    /// Storage for `links` raw ids and as many canonical slots.
    pub fn try_with_capacity(links: usize) -> std::io::Result<Self> {
        let mut resolver = Self {
            required_links: links,
            required_slots: links,
            ..Self::default()
        };
        resolver.grow()?;
        Ok(resolver)
    }

    #[cfg(test)]
    pub fn with_capacity(links: usize) -> Self {
        Self::try_with_capacity(links).unwrap()
    }

    /// Link and census header counts observed by the most recent walk.
    pub fn required(&self) -> [usize; 2] {
        [self.required_links, self.required_slots]
    }

    /// Current cell counts, `[links, slots]`.
    pub fn capacity(&self) -> [usize; 2] {
        [self.references.len(), self.diplomacies.len()]
    }

    /// Grow to the observed headers. Called only by the storage owner, off the
    /// game thread; allocation failure is reported before use of new storage.
    pub fn grow(&mut self) -> std::io::Result<()> {
        // New cells carry generation zero, which `begin` never makes current.
        let links = self.required_links.max(self.references.len());
        let slots = self.required_slots.max(self.diplomacies.len());
        let additional = links - self.references.len();
        crate::allocation::reserve_exact(&mut self.references, additional)?;
        self.references.resize(links, (0, VACANT_COUNTRY));
        let additional = slots - self.diplomacies.len();
        crate::allocation::reserve_exact(&mut self.diplomacies, additional)?;
        self.diplomacies.resize(slots, (0, 0, 0));
        Ok(())
    }

    /// Heap payload only; the owner accounts the inline object.
    pub fn allocated_bytes(&self) -> usize {
        self.references.capacity() * size_of::<(u32, ccountry::Country)>()
            + self.diplomacies.capacity() * size_of::<(u32, usize, usize)>()
    }

    /// Start one walk: invalidate every cell and record the live headers.
    pub(crate) fn begin(&mut self, countries: &ccountry::Reader<'_, impl Memory>) {
        let [census, links] = countries.arrays();
        self.required_links = links.count as usize;
        self.required_slots = census.count as usize;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            // Once per 2^32 walks: stale stamps could otherwise become current.
            self.references.fill((0, VACANT_COUNTRY));
            self.diplomacies.fill((0, 0, 0));
            self.generation = 1;
        }
    }

    pub(crate) fn resolve(
        &mut self,
        countries: &ccountry::Reader<'_, impl Memory>,
        raw: u32,
    ) -> Result<ccountry::Country, &'static str> {
        let generation = self.generation;
        let Some(cell) = self
            .references
            .get_mut(raw as usize)
            .filter(|_| generation != 0)
        else {
            return countries.resolve(raw);
        };
        if cell.0 == generation {
            return Ok(cell.1);
        }
        let country = countries.resolve(raw)?;
        *cell = (generation, country);
        Ok(country)
    }

    /// Reuses a successful diplomacy authentication of this exact country
    /// object; `validate` is the original uncached reader.
    pub(crate) fn diplomacy(
        &mut self,
        country: ccountry::Country,
        validate: impl FnOnce() -> Result<usize, &'static str>,
    ) -> Result<usize, &'static str> {
        let generation = self.generation;
        let Some(cell) = self
            .diplomacies
            .get_mut(country.canonical_slot as usize)
            .filter(|_| generation != 0 && country.pointer != 0)
        else {
            return validate();
        };
        if cell.0 == generation && cell.1 == country.pointer {
            return Ok(cell.2);
        }
        let diplomacy = validate()?;
        *cell = (generation, country.pointer, diplomacy);
        Ok(diplomacy)
    }
}

#[derive(Default)]
struct Counters {
    opinions: usize,
    tuples: usize,
    unread_opinions: u32,
    unread_tuples: u32,
}

impl Counters {
    fn push_opinion(
        &mut self,
        output: &mut Buffer<'_, OpinionRow>,
        row: OpinionRow,
    ) -> Result<(), &'static str> {
        if !output.ensure(
            self.opinions
                .checked_add(1)
                .ok_or("relation opinion row count overflow")?,
        )? {
            self.unread_opinions = self
                .unread_opinions
                .checked_add(1)
                .ok_or("relation opinion unread row count overflow")?;
            return Ok(());
        }
        output[self.opinions] = row;
        self.opinions += 1;
        Ok(())
    }

    fn push_tuple(
        &mut self,
        output: &mut Buffer<'_, TupleRow>,
        row: TupleRow,
    ) -> Result<(), &'static str> {
        if !output.ensure(
            self.tuples
                .checked_add(1)
                .ok_or("relation tuple row count overflow")?,
        )? {
            self.unread_tuples = self
                .unread_tuples
                .checked_add(1)
                .ok_or("relation tuple unread row count overflow")?;
            return Ok(());
        }
        output[self.tuples] = row;
        self.tuples += 1;
        Ok(())
    }

    fn walk(&self) -> Walk {
        Walk {
            opinions: outcome(self.opinions, self.unread_opinions),
            tuples: outcome(self.tuples, self.unread_tuples),
        }
    }
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

/// Reserved-slot-zero operands for stateless-province movement. War tests
/// current pointer and retirement, while co-belligerence reads its byte even
/// without war. No serializer writes this status.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ZeroRelation {
    pub war_present: u8,
    /// The pointed relation's raw +49 byte; zero when there is no war.
    pub war_retired: u8,
    pub co_belligerent: u8,
}

pub fn zero_relation(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<ZeroRelation, &'static str> {
    let diplomacy = diplomacy_root(memory, base, country)?;
    let status = status_at(memory, base, diplomacy, 0)?;
    zero_target(memory, status)?;
    let [war_present, war_retired] = zero_war_at(memory, status)?;
    Ok(ZeroRelation {
        war_present,
        war_retired,
        co_belligerent: read::<u8>(memory, status + CO_BELLIGERENT)
            .ok_or("slot-0 co-belligerent byte unreadable")?,
    })
}

fn zero_target(memory: &impl Memory, status: usize) -> Result<(), &'static str> {
    (read::<u32>(memory, status + TARGET_COUNTRY) == Some(0))
        .then_some(())
        .ok_or("slot-0 relation target mismatch")
}

/// Slot zero's independently readable war operands, after status authentication.
fn zero_war_at(memory: &impl Memory, status: usize) -> Result<[u8; 2], &'static str> {
    zero_war_observed(memory, status, &mut ForwardZeroObservation::default())
}

/// Stack-only attempts from one positive-to-zero owner observation. None
/// means unattempted; the independent packet may fill only those None slots.
/// Original attempts, including Err, stay immutable. Pointed values can only
/// be shared after the reverse cell observes the same nonnull pointer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ForwardZeroObservation {
    // Controls and the raw +50 attempt precede the original literal-zero
    // guard. Both outcomes are retained; the new packet never retries them.
    status: Option<Result<usize, &'static str>>,
    target: Option<Result<u32, &'static str>>,
    war_pointer: Option<Result<u64, &'static str>>,
    war_retired: Option<Result<u8, &'static str>>,
    subject_pointer: Option<Result<u64, &'static str>>,
    subject_type: Option<Result<u32, &'static str>>,
    subject_identity: Option<Result<(), &'static str>>,
    first_party: Option<Result<u32, &'static str>>,
    second_party: Option<Result<u32, &'static str>>,
}

fn zero_war_observed(
    memory: &impl Memory,
    status: usize,
    observed: &mut ForwardZeroObservation,
) -> Result<[u8; 2], &'static str> {
    let war = read::<u64>(memory, status + WAR_RELATION).ok_or("slot-0 war pointer unreadable");
    observed.war_pointer = Some(war);
    let war = war?;
    let war_retired = if war == 0 {
        0
    } else {
        let retired = usize::try_from(war)
            .ok()
            .and_then(|p| p.checked_add(crelation::RETIRED))
            .and_then(|at| read::<u8>(memory, at))
            .ok_or("slot-0 war retired byte unreadable");
        observed.war_retired = Some(retired);
        retired?
    };
    Ok([u8::from(war != 0), war_retired])
}

/// Exact slot-zero subject inputs. Null and non-subject type supply false.
/// First-party zero short-circuits before second demand; a failed first read
/// can still retain a known second zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ZeroSubjectInputs {
    pub relation_type: Option<u32>,
    pub parties: Option<[Option<u32>; 2]>,
}

impl ZeroSubjectInputs {
    /// The successful partial reader result that still contains a guard
    /// refusal. First zero/second None is a normal native short-circuit.
    pub fn read_refusal(&self) -> Option<&'static str> {
        (self.relation_type == Some(csubject_relation::TYPE_VALUE)
            && self.parties == Some([None, Some(0)]))
        .then_some("slot-0 subject first party unreadable")
    }
}

fn zero_subject_at(
    memory: &impl Memory,
    base: usize,
    status: usize,
    observed: &mut ForwardZeroObservation,
) -> Result<ZeroSubjectInputs, &'static str> {
    let pointer = status
        .checked_add(SUBJECT_RELATION)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("slot-0 subject pointer unreadable");
    observed.subject_pointer = Some(pointer);
    let pointer = pointer?;
    if pointer == 0 {
        return Ok(ZeroSubjectInputs::default());
    }
    let pointer = usize::try_from(pointer).map_err(|_| "slot-0 subject pointer overflow");
    let relation_type = pointer.and_then(|pointer| {
        pointer
            .checked_add(crelation::TYPE_ID)
            .and_then(|at| read::<u32>(memory, at))
            .ok_or("slot-0 subject type unreadable")
    });
    observed.subject_type = Some(relation_type);
    let relation_type = relation_type?;
    let pointer = pointer?;
    let parties = if relation_type == csubject_relation::TYPE_VALUE {
        let identity = base
            .checked_add(csubject_relation::VTABLE_RVA)
            .ok_or("slot-0 subject module address overflow")
            .and_then(|expected| {
                (read::<u64>(memory, pointer) == Some(expected as u64))
                    .then_some(())
                    .ok_or("slot-0 subject vtable mismatch")
            });
        observed.subject_identity = Some(identity);
        identity?;
        let first = pointer
            .checked_add(crelation::FIRST_COUNTRY)
            .and_then(|at| read::<u32>(memory, at))
            .ok_or("slot-0 subject first party unreadable");
        observed.first_party = Some(first);
        let first = first.ok();
        if first == Some(0) {
            Some([first, None])
        } else {
            let second = pointer
                .checked_add(crelation::SECOND_COUNTRY)
                .and_then(|at| read::<u32>(memory, at))
                .ok_or("slot-0 subject second party unreadable");
            observed.second_party = Some(second);
            let second = second.ok();
            if first.is_none() && second != Some(0) {
                return Err("slot-0 subject first party unreadable");
            }
            if second.is_none() {
                return Err("slot-0 subject second party unreadable");
            }
            Some([first, second])
        }
    } else {
        None
    };
    Ok(ZeroSubjectInputs {
        relation_type: Some(relation_type),
        parties,
    })
}

/// The authenticated status in `diplomacy`'s matrix at `slot`.
fn status_at(
    memory: &impl Memory,
    base: usize,
    diplomacy: usize,
    slot: u32,
) -> Result<usize, &'static str> {
    status_in_matrix(
        memory,
        base,
        diplomacy,
        relation_matrix(memory, diplomacy)?,
        slot,
    )
}

fn relation_matrix(memory: &impl Memory, diplomacy: usize) -> Result<Array, &'static str> {
    diplomacy
        .checked_add(cdiplomacy_status::RELATIONS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("relation matrix unreadable or invalid")
}

fn status_in_matrix(
    memory: &impl Memory,
    base: usize,
    diplomacy: usize,
    matrix: Array,
    slot: u32,
) -> Result<usize, &'static str> {
    let status = matrix
        .element(slot, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("relation status missing")?;
    status
        .checked_add(CO_BELLIGERENT)
        .ok_or("relation status address overflow")?;
    if read::<u64>(memory, status) != base.checked_add(VTABLE_RVA).map(|p| p as u64)
        || read::<u64>(memory, status + DIPLOMACY_STATUS) != Some(diplomacy as u64)
    {
        return Err("relation status identity mismatch");
    }
    Ok(status)
}

/// One co-belligerent candidate's directed byte: [`status_in_matrix`], then
/// [`country_target_with`] over the country links, then the +310 byte. The status's identity, target and
/// byte sit on four separate lines of a scattered object, so after the matrix
/// element they are copied in one guarded, prefetched batch whose misses
/// overlap, and checked in the order of the separate reads: the first fault
/// is theirs.
fn co_candidate_byte(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    diplomacy: usize,
    matrix: Array,
    slot: u32,
) -> Result<u8, &'static str> {
    let status = matrix
        .element(slot, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("relation status missing")?;
    status
        .checked_add(CO_BELLIGERENT)
        .ok_or("relation status address overflow")?;
    let mut vtable = [0_u8; 8];
    let mut owner = [0_u8; 8];
    let mut target = [0_u8; 4];
    let mut byte = [0_u8; 1];
    // CO_BELLIGERENT is the furthest field, so none of these overflow.
    let copied = memory.copy_scattered_prefetched(
        [
            status,
            status + DIPLOMACY_STATUS,
            status + TARGET_COUNTRY,
            status + CO_BELLIGERENT,
        ],
        &mut [&mut vtable, &mut owner, &mut target, &mut byte],
    );
    if copied < 2
        || Some(u64::from_le_bytes(vtable)) != base.checked_add(VTABLE_RVA).map(|p| p as u64)
        || u64::from_le_bytes(owner) != diplomacy as u64
    {
        return Err("relation status identity mismatch");
    }
    if copied < 3 {
        return Err("relation status target unreadable");
    }
    let raw = u32::from_le_bytes(target);
    if raw == 0 || countries.link(raw)? != Some(slot) {
        return Err("relation status target differs from matrix slot");
    }
    if copied < 4 {
        return Err("co-belligerent byte unreadable");
    }
    Ok(byte[0])
}

fn country_target_with(
    memory: &impl Memory,
    status: usize,
    slot: u32,
    mut canonical: impl FnMut(u32) -> Result<Option<u32>, &'static str>,
) -> Result<(), &'static str> {
    // Construction stores each resolved matrix index's raw link. Live links can
    // be aliases, so raw equality alone does not establish identity.
    let raw =
        read::<u32>(memory, status + TARGET_COUNTRY).ok_or("relation status target unreadable")?;
    if raw == 0 || canonical(raw)? != Some(slot) {
        return Err("relation status target differs from matrix slot");
    }
    Ok(())
}

const CO_CANDIDATE_VECTORS: [usize; 2] = [
    cdiplomacy_status::WARTIME_ALLIES,
    cdiplomacy_status::ENEMY_COUNTRIES,
];

fn co_candidate_vector(
    memory: &impl Memory,
    diplomacy: usize,
    offset: usize,
) -> Result<Array, &'static str> {
    diplomacy
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, 4))
        .ok_or("co-belligerent candidate vector unreadable or invalid")
}

fn for_each_co_candidate(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    diplomacy: usize,
    mut visit: impl FnMut(u32) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for_each_co_candidate_with(
        memory,
        diplomacy,
        |raw| {
            countries
                .link(raw)?
                .ok_or("co-belligerent candidate has no country link")
        },
        &mut visit,
    )
}

fn for_each_co_candidate_with(
    memory: &impl Memory,
    diplomacy: usize,
    mut canonical: impl FnMut(u32) -> Result<u32, &'static str>,
    mut visit: impl FnMut(u32) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for offset in CO_CANDIDATE_VECTORS {
        let vector = co_candidate_vector(memory, diplomacy, offset)?;
        for_each_link(memory, vector, |_, raw| {
            // Resolve canonical slot as the setter does for matrix indexing.
            let slot = if raw == 0 { 0 } else { canonical(raw)? };
            visit(slot)
        })?;
    }
    Ok(())
}

/// Independent native operands for one already-authenticated positive country.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoBelligerentInputs {
    /// Complete sorted unique canonical target slots whose stored +310 byte
    /// is nonzero, including slot zero. An error exposes no usable prefix.
    pub targets: Result<usize, &'static str>,
    /// Exact unique target demand after a complete walk, including an arena
    /// shortage; on a read refusal, only the known demand before that fault.
    pub required_targets: usize,
    /// Slot-zero war pointer presence (0/1) and the pointed relation's exact
    /// retired byte (zero for a null pointer), independent of `targets`.
    pub zero_war: Result<[u8; 2], &'static str>,
    /// Independent pointer/type/parties from the same authenticated slot-zero
    /// status. A failure does not erase the war or directed-cache operands.
    pub zero_subject: Result<ZeroSubjectInputs, &'static str>,
}

/// Directed cache for counter grouping and movement cost. Rebuilding clears
/// matrix bytes then sets targets in wartime allies. Later enemy classification
/// can move targets without clearing bytes, so both lists supply a candidate
/// superset rather than derived values. Status reset only clears bytes; full
/// reset clears lists or rebuilds. Read slot zero explicitly. Any nonzero byte
/// is stored truth, including values above one; add no symmetry or war veto.
/// Controls are acquired once and never retried. Slot-zero identity failure
/// refuses all results; after authentication, independent war, subject and cache
/// failures preserve other results. Refusal clears the written prefix; success
/// retains exact demand despite short capacity and exposes only returned length.
/// No allocation or game call.
pub fn co_belligerent_inputs(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: ccountry::Country,
    output: &mut [u32],
) -> CoBelligerentInputs {
    co_belligerent_inputs_observed(
        memory,
        base,
        countries,
        country,
        output,
        &mut ForwardZeroObservation::default(),
    )
}

/// The original reader with its exact attempted pointed operands retained
/// for one matching reverse cell. Outputs, source order and faults are
/// identical to [`co_belligerent_inputs`]; no new native reads or allocation.
pub fn co_belligerent_inputs_observed(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: ccountry::Country,
    output: &mut [u32],
    observed: &mut ForwardZeroObservation,
) -> CoBelligerentInputs {
    co_belligerent_inputs_observed_buffer(
        memory,
        base,
        countries,
        country,
        Buffer::Fixed(output),
        observed,
    )
}

pub fn co_belligerent_inputs_observed_buffer(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: ccountry::Country,
    mut output: Buffer<'_, u32>,
    observed: &mut ForwardZeroObservation,
) -> CoBelligerentInputs {
    *observed = ForwardZeroObservation::default();
    let controls = (|| {
        let diplomacy = diplomacy_root(memory, base, country)?;
        let matrix = relation_matrix(memory, diplomacy)?;
        let zero = status_in_matrix(memory, base, diplomacy, matrix, 0)?;
        Ok::<_, &'static str>((diplomacy, matrix, zero))
    })();
    observed.status = Some(controls.map(|(_, _, zero)| zero));
    let controls = controls.and_then(|controls| {
        let raw = read::<u32>(memory, controls.2 + TARGET_COUNTRY);
        observed.target = Some(raw.ok_or("forward-zero relation target unreadable"));
        // Preserve zero_target's original read width, mismatch error and
        // short-circuit; the new packet alone accepts a positive alias0.
        (raw == Some(0))
            .then_some(controls)
            .ok_or("slot-0 relation target mismatch")
    });
    let (diplomacy, matrix, zero) = match controls {
        Ok(controls) => controls,
        Err(reason) => {
            return CoBelligerentInputs {
                targets: Err(reason),
                required_targets: 0,
                zero_war: Err(reason),
                zero_subject: Err(reason),
            };
        }
    };
    let zero_war = zero_war_observed(memory, zero, observed);
    let zero_subject = zero_subject_at(memory, base, zero, observed);
    let mut required_targets = 0;
    let targets = (|| {
        // Wartime-ally rebuilding indexes the acquired signed matrix extent.
        // Deduplicate before payload reads even beyond output capacity.
        let mut seen = target_words(matrix.count)?;
        seen[0] = 1;
        if read::<u8>(memory, zero + CO_BELLIGERENT)
            .ok_or("slot-0 co-belligerent byte unreadable")?
            != 0
        {
            required_targets = 1;
            output.ensure(required_targets)?;
            if let Some(cell) = output.first_mut() {
                *cell = 0;
            }
        }
        for_each_co_candidate(memory, countries, diplomacy, |slot| {
            let bit = 1_u64 << (slot % 64);
            let word = seen
                .get_mut(slot as usize / 64)
                .ok_or("co-belligerent target outside relation matrix")?;
            if *word & bit != 0 {
                return Ok(());
            }
            *word |= bit;
            if co_candidate_byte(memory, base, countries, diplomacy, matrix, slot)? != 0 {
                // Wartime ally rebuilding supplies these native candidate
                // headers.
                output.ensure(
                    required_targets
                        .checked_add(1)
                        .ok_or("co-belligerent target demand overflow")?,
                )?;
                if let Some(cell) = output.get_mut(required_targets) {
                    *cell = slot;
                }
                required_targets += 1;
            }
            Ok(())
        })?;
        if required_targets > output.len() {
            return Err("co-belligerent targets do not fit arena");
        }
        output[..required_targets].sort_unstable();
        Ok(required_targets)
    })();
    if targets.is_err() {
        let written = required_targets.min(output.len());
        output[..written].fill(0);
    }
    CoBelligerentInputs {
        targets,
        required_targets,
        zero_war,
        zero_subject,
    }
}

/// Full-country-visibility operands. Original receiver-zero cells and positive
/// receivers' status0 use the same shape. Missing remains distinct from null
/// and false. Authenticate raw target aliases against the cell matrix ordinal.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReverseZeroVisibilityInputs {
    pub target: Option<u32>,
    pub war: Option<[u8; 2]>,
    pub subject: Option<ZeroSubjectInputs>,
    /// Native flag1 arm: exact +2A0 qword presence, not a pointed getter.
    /// Some(0/1) is null/nonnull; None is skipped or refused.
    pub military_access: Option<u8>,
}

/// Nonserialized diagnostics for target, war, subject and military access respectively.
/// Unreached None values have no failure; partial OR truth retains the first
/// actually refused operand. A status fault refuses the whole cell value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReverseZeroVisibilityCapture {
    pub value: Result<ReverseZeroVisibilityInputs, &'static str>,
    pub failures: [Option<&'static str>; 4],
}

/// Independent status0 packet of a positive receiver. The original guard
/// and fields are unchanged. This consumes that same callback's attempts,
/// including failures, and fills only operands the old guard never reached.
/// Its value shape is the existing four-value visibility payload; no pointers
/// escape. War precedes military, and flag0 subject inputs remain independent.
pub fn forward_zero_visibility_inputs<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    observed: &mut ForwardZeroObservation,
) -> ReverseZeroVisibilityCapture {
    let mut failures = [None; 4];
    let value = (|| {
        let status = observed
            .status
            .ok_or("forward-zero status observation unattempted")??;
        let mut value = ReverseZeroVisibilityInputs::default();
        let war = (|| {
            let pointer = *observed.war_pointer.get_or_insert_with(|| {
                read::<u64>(memory, status + WAR_RELATION)
                    .ok_or("forward-zero war pointer unreadable")
            });
            let pointer = pointer?;
            if pointer == 0 {
                return Ok([0, 0]);
            }
            let retired = *observed.war_retired.get_or_insert_with(|| {
                usize::try_from(pointer)
                    .ok()
                    .and_then(|pointer| pointer.checked_add(crelation::RETIRED))
                    .and_then(|at| read::<u8>(memory, at))
                    .ok_or("forward-zero war retired byte unreadable")
            });
            Ok([1, retired?])
        })();
        value.war = war.ok();
        failures[1] = war.err();
        if !matches!(war, Ok([0, _]) | Ok([1, 1..=u8::MAX])) {
            return Ok(value);
        }
        let military = read::<u64>(memory, status + MILITARY_ACCESS)
            .map(|pointer| u8::from(pointer != 0))
            .ok_or("forward-zero military access pointer unreadable");
        value.military_access = military.ok();
        failures[3] = military.err();
        // These exact raw target/party links belong to one native status0
        // predicate. Three fixed keys retain failed as well as healthy links.
        let mut links = [None; 3];
        let mut canonical = |raw: u32| {
            if raw == 0 {
                return Ok(0);
            }
            if let Some((_, result)) = links.iter().flatten().find(|(key, _)| *key == raw) {
                return *result;
            }
            let result = countries.link(raw).map(|slot| slot.unwrap_or(0));
            let cell = links
                .iter_mut()
                .find(|cell| cell.is_none())
                .ok_or("forward-zero local link demand exceeds operands")?;
            *cell = Some((raw, result));
            result
        };
        let target = observed
            .target
            .unwrap_or(Err("forward-zero relation target observation unattempted"))
            .and_then(|raw| {
                if canonical(raw)? != 0 {
                    return Err("forward-zero relation target differs from matrix slot");
                }
                Ok(raw)
            });
        value.target = target.ok();
        failures[0] = target.err();
        let pointer = *observed.subject_pointer.get_or_insert_with(|| {
            read::<u64>(memory, status + SUBJECT_RELATION)
                .ok_or("forward-zero subject pointer unreadable")
        });
        let subject = pointer.and_then(|pointer| {
            visibility_subject_at_pointer(
                memory,
                base,
                pointer,
                target,
                observed,
                &mut canonical,
                &mut failures[2],
            )
        });
        value.subject = subject.ok();
        if let Err(reason) = subject {
            failures[2].get_or_insert(reason);
        }
        Ok(value)
    })();
    ReverseZeroVisibilityCapture { value, failures }
}

/// One callback's authenticated receiver-zero root and matrix header. The
/// shared naming preparation owns F88/identity/backlink/faction attempts;
/// this source adds only the matrix header. No native pointer arena or index.
#[derive(Debug)]
pub struct ReservedVisibilitySource {
    controls: Option<(usize, Array)>,
    required: usize,
    faction: Result<u64, &'static str>,
}

impl ReservedVisibilitySource {
    pub fn new(
        memory: &impl Memory,
        observed: &ccountry::ReservedVisibilityObservation,
    ) -> Result<Self, &'static str> {
        let root = observed.root()?;
        if root.id != 0 || root.canonical_slot != 0 || root.pointer == 0 {
            return Err("reserved visibility owner is not actual zero");
        }
        let diplomacy = observed.diplomacy()?;
        let matrix = relation_matrix(memory, diplomacy)?;
        Ok(Self {
            controls: Some((diplomacy, matrix)),
            required: matrix.count as usize,
            faction: observed.faction(),
        })
    }

    /// Actual native matrix count, including0, retained after control clear.
    pub fn required_slots(&self) -> usize {
        self.required
    }

    /// Invoke each admitted ordinal once, including0 independently of the
    /// positive census. The caller admits/commits one whole value arena.
    /// Exact same-nonnull pointed observations reuse failures as failures.
    pub fn cell<M: Memory>(
        &self,
        memory: &M,
        base: usize,
        countries: Option<&ccountry::Reader<'_, M>>,
        slot: u32,
        forward: Option<&ForwardZeroObservation>,
    ) -> ReverseZeroVisibilityCapture {
        let mut failures = [None; 4];
        let value = (|| {
            let (diplomacy, matrix) = self
                .controls
                .ok_or("reserved visibility controls cleared")?;
            let status = status_in_matrix(memory, base, diplomacy, matrix, slot)?;
            let mut value = ReverseZeroVisibilityInputs::default();
            let war = (|| {
                let pointer = read::<u64>(memory, status + WAR_RELATION)
                    .ok_or("reverse-zero war pointer unreadable")?;
                if pointer == 0 {
                    return Ok([0, 0]);
                }
                let shared = forward.filter(|observed| observed.war_pointer == Some(Ok(pointer)));
                let retired = shared
                    .and_then(|observed| observed.war_retired)
                    .unwrap_or_else(|| {
                        usize::try_from(pointer)
                            .ok()
                            .and_then(|pointer| pointer.checked_add(crelation::RETIRED))
                            .and_then(|at| read::<u8>(memory, at))
                            .ok_or("reverse-zero war retired byte unreadable")
                    })?;
                Ok([1, retired])
            })();
            value.war = war.ok();
            failures[1] = war.err();
            // War veto precedes the military-access word. A failed faction observation
            // cannot hide this independent arm.
            if !matches!(war, Ok([0, _]) | Ok([1, 1..=u8::MAX])) {
                return Ok(value);
            }
            let military = read::<u64>(memory, status + MILITARY_ACCESS)
                .map(|pointer| u8::from(pointer != 0))
                .ok_or("reverse-zero military access pointer unreadable");
            value.military_access = military.ok();
            failures[3] = military.err();
            // The same packet serves flag0 and flag1. Retain the flag0
            // faction/subject union even when military access is present.
            if self.faction.is_err() || (slot == 0 && matches!(self.faction, Ok(1..=u64::MAX))) {
                return Ok(value);
            }
            // Three fixed stack keys avoid repeated target/party link reads
            // in this cell. Literal zero bypasses the positive Reader.
            let mut links = [None; 3];
            let mut canonical = |raw: u32| {
                if raw == 0 {
                    return Ok(0);
                }
                if let Some((_, result)) = links.iter().flatten().find(|(key, _)| *key == raw) {
                    return *result;
                }
                let result = countries
                    .ok_or("reverse-zero positive country links unavailable")
                    .and_then(|countries| countries.link(raw))
                    .map(|slot| slot.unwrap_or(0));
                let cell = links
                    .iter_mut()
                    .find(|cell| cell.is_none())
                    .ok_or("reverse-zero local link demand exceeds operands")?;
                *cell = Some((raw, result));
                result
            };
            let target = read::<u32>(memory, status + TARGET_COUNTRY)
                .ok_or("reverse-zero relation target unreadable")
                .and_then(|raw| {
                    if (raw == 0 && slot != 0) || canonical(raw)? != slot {
                        return Err("reverse-zero relation target differs from matrix slot");
                    }
                    Ok(raw)
                });
            value.target = target.ok();
            failures[0] = target.err();
            let subject = reverse_subject_at(
                memory,
                base,
                status,
                target,
                forward,
                &mut canonical,
                &mut failures[2],
            );
            value.subject = subject.ok();
            if let Err(reason) = subject {
                failures[2].get_or_insert(reason);
            }
            Ok(value)
        })();
        ReverseZeroVisibilityCapture { value, failures }
    }

    pub fn clear(&mut self) {
        self.controls = None;
        self.faction = Err("reserved visibility controls cleared");
    }
}

impl Drop for ReservedVisibilitySource {
    fn drop(&mut self) {
        self.clear();
    }
}

fn reverse_subject_at(
    memory: &impl Memory,
    base: usize,
    status: usize,
    target: Result<u32, &'static str>,
    forward: Option<&ForwardZeroObservation>,
    canonical: &mut impl FnMut(u32) -> Result<u32, &'static str>,
    failure: &mut Option<&'static str>,
) -> Result<ZeroSubjectInputs, &'static str> {
    let raw_pointer = read::<u64>(memory, status + SUBJECT_RELATION)
        .ok_or("reverse-zero subject pointer unreadable")?;
    // A distinct reverse status always reads its own pointer. Only exact
    // equality to a known nonnull pointer admits sharing pointed attempts.
    let mut attempts = forward
        .filter(|observed| raw_pointer != 0 && observed.subject_pointer == Some(Ok(raw_pointer)))
        .copied()
        .unwrap_or_default();
    visibility_subject_at_pointer(
        memory,
        base,
        raw_pointer,
        target,
        &mut attempts,
        canonical,
        failure,
    )
}

// One narrow pointed body serves both roles. Existing diagnostic strings
// remain exact. get_or_insert_with retains every original attempt and stores
// newly reached Err as Err for the paired reverse consumer; no pointer cache.
fn visibility_subject_at_pointer(
    memory: &impl Memory,
    base: usize,
    raw_pointer: u64,
    target: Result<u32, &'static str>,
    attempts: &mut ForwardZeroObservation,
    canonical: &mut impl FnMut(u32) -> Result<u32, &'static str>,
    failure: &mut Option<&'static str>,
) -> Result<ZeroSubjectInputs, &'static str> {
    if raw_pointer == 0 {
        return Ok(ZeroSubjectInputs::default());
    }
    let pointer = usize::try_from(raw_pointer).map_err(|_| "reverse-zero subject pointer overflow");
    let relation_type = (*attempts.subject_type.get_or_insert_with(|| {
        pointer.and_then(|pointer| {
            pointer
                .checked_add(crelation::TYPE_ID)
                .and_then(|at| read::<u32>(memory, at))
                .ok_or("reverse-zero subject type unreadable")
        })
    }))?;
    if relation_type != csubject_relation::TYPE_VALUE {
        return Ok(ZeroSubjectInputs {
            relation_type: Some(relation_type),
            parties: None,
        });
    }
    let pointer = pointer?;
    (*attempts.subject_identity.get_or_insert_with(|| {
        let expected = base
            .checked_add(csubject_relation::VTABLE_RVA)
            .ok_or("reverse-zero subject module address overflow")?;
        (read::<u64>(memory, pointer) == Some(expected as u64))
            .then_some(())
            .ok_or("reverse-zero subject vtable mismatch")
    }))?;
    let first = *attempts.first_party.get_or_insert_with(|| {
        pointer
            .checked_add(crelation::FIRST_COUNTRY)
            .and_then(|at| read::<u32>(memory, at))
            .ok_or("reverse-zero subject first party unreadable")
    });
    if let Err(reason) = first {
        failure.get_or_insert(reason);
    }
    let first_matches = first.and_then(|first| {
        target.and_then(|target| {
            if first == target {
                return Ok(true);
            }
            if first == 0 || target == 0 {
                return Ok(false);
            }
            Ok(canonical(first)? == canonical(target)?)
        })
    });
    // Target refusal has its own domain; a refused party link is a subject
    // fault even when its raw word was readable.
    if target.is_ok()
        && let Err(reason) = first_matches
    {
        failure.get_or_insert(reason);
    }
    if first_matches == Ok(true) {
        return Ok(ZeroSubjectInputs {
            relation_type: Some(relation_type),
            parties: Some([first.ok(), None]),
        });
    }
    let second = *attempts.second_party.get_or_insert_with(|| {
        pointer
            .checked_add(crelation::SECOND_COUNTRY)
            .and_then(|at| read::<u32>(memory, at))
            .ok_or("reverse-zero subject second party unreadable")
    });
    if let Err(reason) = second {
        failure.get_or_insert(reason);
    }
    // Raw parties remain useful after a refused first read. The consumer
    // applies raw OR; a known second match can still prove true.
    Ok(ZeroSubjectInputs {
        relation_type: Some(relation_type),
        parties: Some([first.ok(), second.ok()]),
    })
}

/// Standalone attach/poll hint only. No names, count, faction or strings are
/// acquired. Never use this as a fallback after a failed capture source.
pub fn reserved_visibility_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<usize, &'static str> {
    let root = ccountry::read_reserved_root(memory, base, state)?;
    let diplomacy = diplomacy_root(memory, base, root)?;
    Ok(relation_matrix(memory, diplomacy)?.count as usize)
}

/// Header-only conservative target demand for initial live arena sizing.
/// The native +B0/+98 counts plus one explicit slot-zero status bound the
/// complete set. This hint neither reads payload nor certifies captured data;
/// a caller sums each readable country's bound without dropping earlier
/// countries when another country refuses its own headers.
pub fn co_belligerent_demand(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    let diplomacy = diplomacy_root(memory, base, country)?;
    let mut required = 1_usize;
    for offset in CO_CANDIDATE_VECTORS {
        required = required
            .checked_add(co_candidate_vector(memory, diplomacy, offset)?.count as usize)
            .ok_or("co-belligerent target demand overflow")?;
    }
    Ok(required)
}

/// Nonzero co-belligerent target slots in ascending order, with written and
/// required counts. Reserved slot zero is separate. Rebuilding clears bytes
/// then sets wartime allies; moving an ally to enemies can leave its byte.
/// Status reset clears bytes and full reset clears vectors or rebuilds. Their
/// union therefore supplies candidates; read each byte rather than inferring it.
pub fn co_belligerents(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: ccountry::Country,
    output: &mut [u32],
) -> Result<(usize, usize), &'static str> {
    co_belligerents_with(
        memory,
        base,
        country,
        output,
        |raw| {
            countries
                .link(raw)?
                .ok_or("co-belligerent candidate has no country link")
        },
        |raw| countries.link(raw),
    )
}

/// Native raw-zero / alias-to-zero consuming law used by invasion route
/// operands. Slot-zero candidates are skipped before status reads because
/// `zero_relation` captures that independent slot. Positive status targets
/// still require a nonzero raw reference and its exact canonical slot.
/// Like [`co_belligerents`], required is exact when the output fits; shortage
/// retains the established conservative upper bound because duplicate targets
/// outside the stored prefix are not tracked. Such a prefix is never complete.
pub fn co_belligerents_native(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    country: ccountry::Country,
    output: &mut [u32],
) -> Result<(usize, usize), &'static str> {
    co_belligerents_native_buffer(
        memory,
        base,
        countries,
        country,
        crate::walk::Buffer::Fixed(output),
    )
}

/// Invasion worker retains original candidate traversal, growing slots as
/// each distinct status authenticates.
pub fn co_belligerents_native_buffer(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    country: ccountry::Country,
    output: crate::walk::Buffer<'_, u32>,
) -> Result<(usize, usize), &'static str> {
    // Check the borrowed pass without demanding a country/header observation,
    // including when every candidate is zero or both vectors are empty.
    countries.canonical_slot(0)?;
    co_belligerents_buffer(
        memory,
        base,
        country,
        output,
        |raw| countries.canonical_slot(raw),
        |raw| countries.canonical_slot(raw).map(Some),
    )
}

fn co_belligerents_with(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    output: &mut [u32],
    canonical_candidate: impl FnMut(u32) -> Result<u32, &'static str>,
    canonical_target: impl FnMut(u32) -> Result<Option<u32>, &'static str>,
) -> Result<(usize, usize), &'static str> {
    co_belligerents_buffer(
        memory,
        base,
        country,
        crate::walk::Buffer::Fixed(output),
        canonical_candidate,
        canonical_target,
    )
}

fn co_belligerents_buffer(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    mut output: crate::walk::Buffer<'_, u32>,
    canonical_candidate: impl FnMut(u32) -> Result<u32, &'static str>,
    mut canonical_target: impl FnMut(u32) -> Result<Option<u32>, &'static str>,
) -> Result<(usize, usize), &'static str> {
    let diplomacy = diplomacy_root(memory, base, country)?;
    let (mut len, mut required) = (0, 0);
    for_each_co_candidate_with(memory, diplomacy, canonical_candidate, |slot| {
        if slot == 0 {
            return Ok(());
        }
        let status = status_at(memory, base, diplomacy, slot)?;
        country_target_with(memory, status, slot, &mut canonical_target)?;
        if read::<u8>(memory, status + CO_BELLIGERENT).ok_or("co-belligerent byte unreadable")? == 0
            || output[..len.min(output.len())].contains(&slot)
        {
            return Ok(());
        }
        required += 1;
        output.ensure(required)?;
        if let Some(cell) = output.get_mut(len) {
            *cell = slot;
            len += 1;
        }
        Ok(())
    })?;
    output[..len].sort_unstable();
    Ok((len, required))
}

fn diplomacy_root(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::DIPLOMACY_STATUS)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("country diplomacy unreadable")?;
    if pointer == 0 {
        return Err("country diplomacy is null");
    }
    let expected = base
        .checked_add(cdiplomacy_status::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("diplomacy vtable mismatch");
    }
    if pointer
        .checked_add(cdiplomacy_status::COUNTRY)
        .and_then(|address| read::<u64>(memory, address))
        != Some(country.pointer as u64)
    {
        return Err("diplomacy country mismatch");
    }
    Ok(pointer)
}

/// One guarded copy of the contiguous pointer span; a null slot is a fault
/// because diplomacy construction constructs every matrix cell eagerly.
fn read_pointers(
    memory: &impl Memory,
    matrix: Array,
    first: u32,
    output: &mut [usize],
) -> Result<(), &'static str> {
    let address = matrix
        .element(first, 8)
        .ok_or("relation matrix pointer address overflow")?;
    let mut bytes = [0_u8; BATCH * 8];
    let span = &mut bytes[..output.len() * 8];
    if !read_slice(memory, address, span) {
        return Err("relation matrix pointer batch unreadable");
    }
    for (slot, chunk) in output.iter_mut().zip(span.chunks_exact(8)) {
        let pointer = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap()))
            .map_err(|_| "relation matrix pointer overflow")?;
        if pointer == 0 {
            return Err("null relation matrix entry");
        }
        *slot = pointer;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_tuples(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    owner: ccountry::Country,
    target_country: u32,
    serializable: Array,
    output: &mut Buffer<'_, TupleRow>,
    counters: &mut Counters,
    resolved: &mut Resolver,
) -> Result<(), &'static str> {
    // Relation pointers with their authentication and payload spans are
    // gathered in 64-row batches instead of eight guarded reads per tuple;
    // every tuple is still authenticated, skipped and emitted in native vector
    // order with its own copied prefix, so ordinals and single-fault reasons
    // keep their positions.
    let mut first = 0;
    let mut pointers = [0_usize; BATCH];
    let mut pointer_bytes = [0_u8; BATCH * 8];
    let mut auth_records = [0_u8; BATCH * TUPLE_AUTH_BYTES];
    let mut rest_records = [0_u8; BATCH * TUPLE_REST_BYTES];
    let mut live_pointers = [0_usize; BATCH];
    let mut live_ordinals = [0_u32; BATCH];
    let mut live_types = [0_u32; BATCH];
    while first < serializable.count {
        let chunk = ((serializable.count - first) as usize).min(BATCH);
        let first_address = serializable
            .element(first, 8)
            .ok_or("serializable relation entry unreadable")?;
        if !read_slice(memory, first_address, &mut pointer_bytes[..chunk * 8]) {
            return Err("serializable relation entry unreadable");
        }
        for index in 0..chunk {
            let bytes: [u8; 8] = pointer_bytes[index * 8..index * 8 + 8].try_into().unwrap();
            let pointer = usize::try_from(u64::from_le_bytes(bytes))
                .map_err(|_| "serializable relation entry unreadable")?;
            if pointer == 0 {
                return Err("null serializable relation entry");
            }
            pointers[index] = pointer;
        }
        let auth_copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..chunk]),
            &TUPLE_AUTH_LAYOUT,
            &mut auth_records[..chunk * TUPLE_AUTH_BYTES],
        );
        let mut live_len = 0;
        for index in 0..chunk {
            let ordinal = first + index as u32;
            if auth_copied < (index + 1) * TUPLE_AUTH_FIELDS.len() {
                let failed = auth_copied % TUPLE_AUTH_FIELDS.len();
                let pointer = pointers[auth_copied / TUPLE_AUTH_FIELDS.len()];
                let field = &TUPLE_AUTH_FIELDS[failed];
                if pointer
                    .checked_add(field.source_offset)
                    .and_then(|address| address.checked_add(field.len))
                    .is_none()
                {
                    return Err("relation field address overflow");
                }
                if failed == 0 {
                    return Err("relation vtable unreadable");
                }
                return Err("relation type word unreadable");
            }
            let record = &auth_records[index * TUPLE_AUTH_BYTES..(index + 1) * TUPLE_AUTH_BYTES];
            let vtable = u64::from_le_bytes(record[..8].try_into().unwrap());
            let rva = usize::try_from(vtable)
                .ok()
                .and_then(|value| value.checked_sub(base))
                .ok_or("relation vtable outside the module")?;
            let expected_type = crelation::class_type(rva)
                .ok_or("serializable relation has an unclassified vtable")?;
            let type_id = u32::from_le_bytes(record[8..12].try_into().unwrap());
            if type_id != expected_type {
                return Err("relation type word differs from its class");
            }
            // CWarRelation and CSubjectRelation rows are recorded by WarRelations and
            // Subjects. They are authenticated here like every other class, so an
            // unclassified or contradictory object still fails, and then skipped.
            // Their native ordinal is still consumed, because the ordinal is the
            // serializable vector's own index and the save writes that vector whole.
            if rva == cwar_relation::VTABLE_RVA || rva == csubject_relation::VTABLE_RVA {
                continue;
            }
            live_pointers[live_len] = pointers[index];
            live_ordinals[live_len] = ordinal;
            live_types[live_len] = type_id;
            live_len += 1;
        }
        if live_len > 0 {
            let rest_copied = memory.copy_records(
                RecordSources::Pointers(&live_pointers[..live_len]),
                &TUPLE_REST_LAYOUT,
                &mut rest_records[..live_len * TUPLE_REST_BYTES],
            );
            for live in 0..live_len {
                if rest_copied < (live + 1) * TUPLE_REST_FIELDS.len() {
                    let failed = rest_copied % TUPLE_REST_FIELDS.len();
                    let pointer = live_pointers[rest_copied / TUPLE_REST_FIELDS.len()];
                    let field = &TUPLE_REST_FIELDS[failed];
                    if pointer
                        .checked_add(field.source_offset)
                        .and_then(|address| address.checked_add(field.len))
                        .is_none()
                    {
                        return Err("relation field address overflow");
                    }
                    match failed {
                        0 => return Err("relation first country unreadable"),
                        1 => return Err("relation second country unreadable"),
                        2 => return Err("relation start hour unreadable"),
                        3 => return Err("relation end hour unreadable"),
                        4 => return Err("relation cancel byte unreadable"),
                        _ => return Err("relation retired byte unreadable"),
                    }
                }
                let record = &rest_records[live * TUPLE_REST_BYTES..(live + 1) * TUPLE_REST_BYTES];
                let first_country = u32::from_le_bytes(record[..4].try_into().unwrap());
                let second_country = u32::from_le_bytes(record[4..8].try_into().unwrap());
                if resolved.resolve(countries, first_country)?.canonical_slot
                    != owner.canonical_slot
                {
                    return Err("serializable relation is not owned by its first endpoint");
                }
                resolved.resolve(countries, second_country)?;
                counters.push_tuple(
                    output,
                    TupleRow {
                        owner_country: owner.id,
                        target_country,
                        type_id: live_types[live],
                        ordinal: live_ordinals[live],
                        first_country,
                        second_country,
                        start_hour: i32::from_le_bytes(record[8..12].try_into().unwrap()),
                        end_hour: i32::from_le_bytes(record[12..16].try_into().unwrap()),
                        cancel: record[16],
                        retired: record[17],
                    },
                )?;
            }
        }
        first += chunk as u32;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "crelation_status/zero_subject_tests.rs"]
mod zero_subject_tests;

/// Synthetic capture benches for the army-reach walkers: an arena whose address
/// is real, read through the SEH-guarded `LiveMemory` with call and byte counts.
/// The first [`bench::IMAGE`] bytes stand in for the module image, so every
/// `base + RVA` global lands inside the arena.
#[cfg(all(
    any(test, feature = "test-support"),
    windows,
    target_env = "msvc",
    target_arch = "x86_64"
))]
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
#[doc(hidden)]
pub mod bench;
