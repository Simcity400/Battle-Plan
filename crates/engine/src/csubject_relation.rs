//! CSubjectRelation, a directed overlord-to-subject relation.

use crate::{
    Memory, autonomy_state_definition, ccountry, cdiplomacy_status,
    cpdx_array::Array,
    crelation, crelation_status,
    cwar_relation::{MembershipReasons, OwnedSnapshotScratch, scan_occurrence},
    memory::Raw,
    msvc_string, read, read_slice,
    walk::Outcome,
};

/// CSubjectRelation primary vtable; build_profile.rs:349, confirmed by pinned-image RTTI.
pub const VTABLE_RVA: usize = 0x0294_9f18;
/// CSubjectRelation inherited type word; build_profile.rs:629 and the D53 intel-visibility subject check.
pub const TYPE_ID: usize = 0x08;
/// CSubjectRelation expected type word; build_profile.rs:632, D53 intel-visibility reader compares this exact value.
pub const TYPE_VALUE: u32 = 0x30d1;
/// CSubjectRelation overlord country link u32; build_profile.rs:361, D53 read_subject_relation_from_ptr.
pub const OVERLORD: usize = 0x10;
/// CSubjectRelation subject country link u32; build_profile.rs:362, D53 authenticates direction from this pair.
pub const SUBJECT: usize = 0x14;
/// CSubjectRelation start hour i32; build_profile.rs:378 and D53 subject reader.
pub const START_HOUR: usize = 0x20;
/// CSubjectRelation autonomy-state definition pointer; build_profile.rs:363, D53 reads its name at +8.
pub const AUTONOMY_STATE: usize = 0x58;
/// CSubjectRelation raw autonomy ratio i64/100000; build_profile.rs:365, old D53 narrowed it to i32.
pub const AUTONOMY_RATIO: usize = 0x60;

pub const CONSTRUCTOR_RVA: usize = 0x00d0_3810;

/// A complete byte string in the caller-owned arena. Empty names remain empty.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Name {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub overlord: u32,
    pub subject: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
    pub autonomy_ratio: i64,
    pub state: Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    /// All native name lengths, including rows omitted for either capacity.
    pub required_bytes: usize,
}

/// Enumerates native owned relations, authenticating both endpoint matrices and
/// ownership vectors. Emits each active relation once, from its overlord.
/// No allocation, retries, name hashing or precision conversion occurs.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
) -> NameWalk {
    let mut pass = Pass {
        resolver: &mut crelation_status::Resolver::default(),
        ownership: None,
    };
    walk_from(memory, base, game_state, output, names, false, &mut pass)
}

/// `walk` with caller-owned storage, grown off the game thread from what the
/// previous pass observed. An overlord with S subjects and K owned relations
/// is visited from every subject; its owned vector is scanned once and then
/// answered from the counted snapshot, instead of S complete K-entry scans.
/// Country and diplomacy authentication is reused through dense cells. Rows,
/// order and outcomes are identical for every storage size, including empty.
pub fn walk_owned(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
    scratch: &mut OwnedSnapshotScratch,
) -> NameWalk {
    scratch.begin();
    let mut resolver = std::mem::take(&mut scratch.resolver);
    let mut pass = Pass {
        resolver: &mut resolver,
        ownership: Some(&mut *scratch),
    };
    let result = walk_from(memory, base, game_state, output, names, false, &mut pass);
    scratch.resolver = resolver;
    result
}

/// Independent matrix census for bounded consistency validation. This root does
/// not require an owned-vector entry; compare ordered country keys and name bytes
/// off the game thread, because order and arena offsets can differ.
pub fn walk_matrix(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
) -> NameWalk {
    let mut pass = Pass {
        resolver: &mut crelation_status::Resolver::default(),
        ownership: None,
    };
    walk_from(memory, base, game_state, output, names, true, &mut pass)
}

/// Original messages of the subject ownership scans, shared with the war
/// walker's owned-membership snapshot.
const SUBJECT_MEMBERSHIP: MembershipReasons = MembershipReasons {
    vector: "subject relation vector unreadable or invalid",
    pointer: "subject relation vector entry unreadable or outside array",
    null: "null subject ownership entry",
    mismatch: "subject ownership cross-link count mismatch",
    changed: "subject ownership snapshot changed or became unreadable",
    budget: "subject ownership rescans exceed the census budget; snapshot storage grows for the next pass",
};

/// Per-walk reusable storage; both parts may be empty.
struct Pass<'s> {
    resolver: &'s mut crelation_status::Resolver,
    ownership: Option<&'s mut OwnedSnapshotScratch>,
}

fn walk_from(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
    matrix: bool,
    pass: &mut Pass<'_>,
) -> NameWalk {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        pass.resolver.begin(&reader.countries);
        if let Some(scratch) = pass.ownership.as_deref_mut() {
            scratch.set_census(reader.countries.count());
        }
        let mut found_country = false;
        let (mut len, mut bytes_len, mut required_bytes) = (0, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        let mut highest_slot = 0;
        if matrix {
            for slot in 1..reader.countries.count() {
                if reader.countries.at(slot)?.is_some() {
                    highest_slot = slot;
                }
            }
        }
        for slot in 1..reader.countries.count() {
            let Some(country) = reader.countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let diplomacy = pass
                .resolver
                .diplomacy(country, || reader.diplomacy(country))?;
            let source = reader.array(
                diplomacy,
                if matrix {
                    cdiplomacy_status::RELATIONS
                } else {
                    cdiplomacy_status::OWNED_RELATIONS
                },
            )?;
            if matrix && source.count <= highest_slot {
                return Err("subject matrix does not cover country ordinals");
            }
            if !matrix && let Some(scratch) = pass.ownership.as_deref_mut() {
                scratch.observe_owned(source.count);
            }
            let mut matrix_pointers = [0; 64];
            for index in 0..source.count {
                let pointer = if matrix {
                    if index % 64 == 0 {
                        reader.matrix_pointers(source, index, diplomacy, &mut matrix_pointers)?;
                    }
                    let pointer = matrix_pointers[index as usize % 64];
                    if pointer == 0 {
                        continue;
                    }
                    pointer
                } else {
                    let entry = reader.element(source, index)?;
                    if entry == 0 {
                        return Err("null subject relation root entry");
                    }
                    if reader.field::<u32>(entry, TYPE_ID)? != TYPE_VALUE {
                        continue;
                    }
                    entry
                };
                let mut row = match reader.relation(pointer, country, matrix.then_some(index), pass)
                {
                    Ok(Some(row)) => row,
                    Ok(None) => continue,
                    Err(reason) => {
                        if reason == SUBJECT_MEMBERSHIP.budget {
                            reader.observe_remaining(slot + 1, pass);
                        }
                        return Err(reason);
                    }
                };
                let state = reader.pointer(pointer, AUTONOMY_STATE)?;
                let (source, name_len) = definition_name(memory, state)?;
                required_bytes = required_bytes
                    .checked_add(name_len)
                    .ok_or("subject name byte count overflow")?;
                if len == output.len() || name_len > names.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("subject row count overflow")?;
                    continue;
                }
                if !read_slice(memory, source, &mut names[bytes_len..bytes_len + name_len]) {
                    return Err("subject name bytes unreadable");
                }
                row.state = Name {
                    start: bytes_len,
                    len: name_len,
                };
                output[len] = row;
                len += 1;
                bytes_len += name_len;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(unavailable)
}

pub(crate) fn unavailable(reason: &'static str) -> NameWalk {
    NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    }
}

/// Header validation keeps the native byte sequence and the complete length.
/// Definition objects are authenticated by the native relation/status pointers;
/// no unproved vtable or arbitrary identifier-length whitelist is imposed.
pub(crate) fn definition_name(
    memory: &impl Memory,
    definition: usize,
) -> Result<(usize, usize), &'static str> {
    if definition == 0 {
        return Err("autonomy state definition is null");
    }
    let pointer = definition
        .checked_add(autonomy_state_definition::NAME)
        .ok_or("autonomy name address overflow")?;
    let header = read::<[u8; 32]>(memory, pointer).ok_or("autonomy name header unreadable")?;
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(
        header[msvc_string::CAPACITY..msvc_string::CAPACITY + 8]
            .try_into()
            .unwrap(),
    );
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("autonomy name header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "autonomy name length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        pointer
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("autonomy name pointer unreadable or null")?
    };
    let terminator = source
        .checked_add(len)
        .ok_or("autonomy name address overflow")?;
    if read::<u8>(memory, terminator) != Some(0) {
        return Err("autonomy name terminator unreadable or invalid");
    }
    Ok((source, len))
}

pub(crate) struct Reader<'a, M: Memory> {
    pub(crate) memory: &'a M,
    pub(crate) base: usize,
    pub(crate) countries: ccountry::Reader<'a, M>,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub(crate) fn new(memory: &'a M, base: usize, game_state: usize) -> Result<Self, &'static str> {
        Ok(Self {
            memory,
            base,
            countries: ccountry::Reader::new(memory, base, game_state)?,
        })
    }

    pub(crate) fn field<T: Raw>(&self, pointer: usize, offset: usize) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read::<T>(self.memory, address))
            .ok_or("subject field unreadable or address overflow")
    }

    pub(crate) fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?).map_err(|_| "subject pointer overflow")
    }

    pub(crate) fn vtable(
        &self,
        pointer: usize,
        rva: usize,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    fn array(&self, pointer: usize, offset: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| Array::read(self.memory, address, 8))
            .ok_or("subject relation vector unreadable or invalid")
    }

    fn element(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|address| read::<u64>(self.memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("subject relation vector entry unreadable or outside array")
    }

    pub(crate) fn diplomacy(&self, country: ccountry::Country) -> Result<usize, &'static str> {
        let pointer = self.pointer(country.pointer, ccountry::DIPLOMACY_STATUS)?;
        if pointer == 0 {
            return Err("country diplomacy is null");
        }
        self.vtable(
            pointer,
            cdiplomacy_status::VTABLE_RVA,
            "diplomacy vtable mismatch",
        )?;
        if self.field::<u64>(pointer, cdiplomacy_status::COUNTRY)? != country.pointer as u64 {
            return Err("diplomacy country mismatch");
        }
        Ok(pointer)
    }

    fn status(&self, pointer: usize, diplomacy: usize) -> Result<(), &'static str> {
        self.vtable(
            pointer,
            crelation_status::VTABLE_RVA,
            "relation status vtable mismatch",
        )?;
        if self.field::<u64>(pointer, crelation_status::DIPLOMACY_STATUS)? != diplomacy as u64 {
            return Err("relation status diplomacy mismatch");
        }
        Ok(())
    }

    /// Every matrix cell is still inspected, including inactive/sentinel slots.
    /// The same three exact fields are copied together for 64 cells; no relation
    /// pointer is cached across walks, and empty cells remain authenticated.
    fn matrix_pointers(
        &self,
        source: Array,
        first: u32,
        diplomacy: usize,
        output: &mut [usize; 64],
    ) -> Result<(), &'static str> {
        let count = ((source.count - first) as usize).min(output.len());
        let address = source
            .element(first, 8)
            .ok_or("subject matrix pointer address overflow")?;
        let mut pointers = [0_u8; 64 * 8];
        if !read_slice(self.memory, address, &mut pointers[..count * 8]) {
            return Err("subject matrix pointer batch unreadable");
        }
        let mut addresses = [0; 64 * 3];
        for (index, bytes) in pointers[..count * 8].chunks_exact(8).enumerate() {
            let entry = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                .map_err(|_| "subject pointer overflow")?;
            if entry == 0 {
                return Err("null subject relation root entry");
            }
            for (field, offset) in [
                0,
                crelation_status::DIPLOMACY_STATUS,
                crelation_status::SUBJECT_RELATION,
            ]
            .into_iter()
            .enumerate()
            {
                addresses[index * 3 + field] = entry
                    .checked_add(offset)
                    .ok_or("subject field address overflow")?;
            }
        }
        let mut fields = [[0_u8; 8]; 64 * 3];
        let mut buffers = fields.iter_mut().enumerate();
        let mut destinations = std::array::from_fn(|_| {
            let (index, bytes) = buffers.next().unwrap();
            &mut bytes[..if index < count * 3 { 8 } else { 0 }]
        });
        if self.memory.copy_scattered(addresses, &mut destinations) != 64 * 3 {
            return Err("subject matrix field batch unreadable");
        }
        let expected = self
            .base
            .checked_add(crelation_status::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        for index in 0..count {
            if u64::from_le_bytes(fields[index * 3]) != expected {
                return Err("relation status vtable mismatch");
            }
            if u64::from_le_bytes(fields[index * 3 + 1]) != diplomacy as u64 {
                return Err("relation status diplomacy mismatch");
            }
            output[index] = usize::try_from(u64::from_le_bytes(fields[index * 3 + 2]))
                .map_err(|_| "subject pointer overflow")?;
        }
        Ok(())
    }

    /// Whether a relation sits at the overlord's `+0x288` slot toward its
    /// subject. Any unreadable or unresolvable step answers no.
    fn installed(&self, pointer: usize, overlord: u32, subject: u32, pass: &mut Pass<'_>) -> bool {
        (|| {
            let first = pass.resolver.resolve(&self.countries, overlord)?;
            let second = pass.resolver.resolve(&self.countries, subject)?;
            let diplomacy = pass.resolver.diplomacy(first, || self.diplomacy(first))?;
            let matrix = self.array(diplomacy, cdiplomacy_status::RELATIONS)?;
            let status = self.element(matrix, second.canonical_slot)?;
            if status == 0 {
                return Ok(false);
            }
            Ok::<_, &'static str>(
                self.pointer(status, crelation_status::SUBJECT_RELATION)? == pointer,
            )
        })()
        .unwrap_or(false)
    }

    fn pair_status(
        &self,
        diplomacy: usize,
        target: ccountry::Country,
        relation: usize,
        pass: &mut Pass<'_>,
    ) -> Result<usize, &'static str> {
        let matrix = self.array(diplomacy, cdiplomacy_status::RELATIONS)?;
        let status = self.element(matrix, target.canonical_slot)?;
        if status == 0 {
            return Err("null subject relation matrix entry");
        }
        self.status(status, diplomacy)?;
        let target_ref = self.field::<u32>(status, crelation_status::TARGET_COUNTRY)?;
        if pass
            .resolver
            .resolve(&self.countries, target_ref)?
            .canonical_slot
            != target.canonical_slot
        {
            return Err("subject relation target differs from matrix slot");
        }
        if self.pointer(status, crelation_status::SUBJECT_RELATION)? != relation {
            return Err("subject endpoint matrix pointers disagree");
        }
        Ok(status)
    }

    /// After a rescan-budget failure, records the owned-vector headers of the
    /// countries the pass did not reach so one off-thread growth covers the
    /// whole world. The pass has already failed; a fault only ends this early.
    fn observe_remaining(&self, from: u32, pass: &mut Pass<'_>) {
        for slot in from..self.countries.count() {
            let country = match self.countries.at(slot) {
                Ok(Some(country)) => country,
                Ok(None) => continue,
                Err(_) => break,
            };
            let Ok(diplomacy) = pass.resolver.diplomacy(country, || self.diplomacy(country)) else {
                break;
            };
            let Ok(owned) = self.array(diplomacy, cdiplomacy_status::OWNED_RELATIONS) else {
                break;
            };
            if let Some(scratch) = pass.ownership.as_deref_mut() {
                scratch.observe_owned(owned.count);
            }
        }
    }

    /// Counts `relation` in the vector at `root + offset`. Diplomacy-owned
    /// vectors, which grow with a country's relation count, use the pass's
    /// snapshot when one is supplied; all other vectors hold one country pair.
    fn occurrence(
        &self,
        root: usize,
        offset: usize,
        relation: usize,
        expected: u32,
        pass: &mut Pass<'_>,
    ) -> Result<(), &'static str> {
        if offset == cdiplomacy_status::OWNED_RELATIONS
            && expected == 1
            && let Some(scratch) = pass.ownership.as_deref_mut()
        {
            return scratch.membership(self.memory, root, relation, SUBJECT_MEMBERSHIP);
        }
        let array = self.array(root, offset)?;
        scan_occurrence(
            self.memory,
            array,
            relation,
            expected,
            SUBJECT_MEMBERSHIP,
            |_| {},
        )
    }

    fn relation(
        &self,
        pointer: usize,
        owner: ccountry::Country,
        matrix_index: Option<u32>,
        pass: &mut Pass<'_>,
    ) -> Result<Option<Row>, &'static str> {
        self.vtable(pointer, VTABLE_RVA, "subject relation vtable mismatch")?;
        if self.field::<u32>(pointer, TYPE_ID)? != TYPE_VALUE {
            return Err("subject matrix contains another relation type");
        }
        let retired = self.field::<u8>(pointer, crelation::RETIRED)? != 0;
        let overlord = self.field::<u32>(pointer, OVERLORD)?;
        let subject = self.field::<u32>(pointer, SUBJECT)?;
        // The shared-vision subject terms (140D11030, 140D11080) read only the
        // status +0x288 pointer and its type, never the retired byte, and
        // clear_link (vf22@140D12990) is what uninstalls a subject. So a
        // retired relation counts exactly while it is still installed; one
        // left only in an owned vector is skipped.
        if retired && matrix_index.is_none() && !self.installed(pointer, overlord, subject, pass) {
            return Ok(None);
        }
        let first = pass.resolver.resolve(&self.countries, overlord)?;
        let second = pass.resolver.resolve(&self.countries, subject)?;
        if first.canonical_slot == second.canonical_slot {
            return Err("subject endpoints resolve to same country");
        }
        let target = if owner.canonical_slot == first.canonical_slot {
            second
        } else if owner.canonical_slot == second.canonical_slot {
            first
        } else {
            return Err("subject relation does not include owning country");
        };
        if matrix_index.is_some_and(|index| index != target.canonical_slot) {
            return Err("subject relation stored in wrong matrix slot");
        }
        let first_diplomacy = pass.resolver.diplomacy(first, || self.diplomacy(first))?;
        let second_diplomacy = pass.resolver.diplomacy(second, || self.diplomacy(second))?;
        let first_status = self.pair_status(first_diplomacy, second, pointer, pass)?;
        let second_status = self.pair_status(second_diplomacy, first, pointer, pass)?;
        if matrix_index.is_none() {
            for diplomacy in [first_diplomacy, second_diplomacy] {
                self.occurrence(
                    diplomacy,
                    cdiplomacy_status::OWNED_RELATIONS,
                    pointer,
                    1,
                    pass,
                )?;
            }
            for status in [first_status, second_status] {
                self.occurrence(status, crelation_status::SUBRELATIONS, pointer, 1, pass)?;
            }
            self.occurrence(
                first_status,
                crelation_status::SERIALIZABLE_RELATIONS,
                pointer,
                1,
                pass,
            )?;
            self.occurrence(
                second_status,
                crelation_status::SERIALIZABLE_RELATIONS,
                pointer,
                0,
                pass,
            )?;
        }
        if owner.canonical_slot != first.canonical_slot {
            return Ok(None);
        }
        Ok(Some(Row {
            overlord,
            subject,
            start_hour: self.field(pointer, START_HOUR)?,
            end_hour: self.field(pointer, crelation::END_HOUR)?,
            cancel: self.field(pointer, crelation::CANCEL)?,
            autonomy_ratio: self.field(pointer, AUTONOMY_RATIO)?,
            state: Name::default(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    const GS: usize = 0x1000;
    const COUNTRIES: [usize; 2] = [0x4000, 0x6000];
    const DIPLOMACY: [usize; 2] = [0x8000, 0x9000];
    const RELATION: usize = 0x18000;
    const RETIRED_RELATION: usize = 0x19000;
    const DEFINITION: usize = 0x1b000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, values.len() as u32);
        p32(bytes, at + 12, values.len() as u32);
        for (index, value) in values.iter().enumerate() {
            p64(bytes, data + index * 8, *value);
        }
    }
    fn status(owner: usize, target: usize) -> usize {
        0xb000 + owner * 0x5000 + target * 0xa00
    }
    fn owned(owner: usize) -> usize {
        0xa100 + owner * 0x100
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x20000];
        array(
            &mut bytes,
            GS + cgame_state::COUNTRIES,
            0x3000,
            &[
                u64::MAX,
                COUNTRIES[0] as u64,
                COUNTRIES[1] as u64,
                COUNTRIES[0] as u64,
            ],
        );
        p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 4);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 4);
        for (index, value) in [0, 1, 2, 1].into_iter().enumerate() {
            p32(&mut bytes, 0x3100 + index * 4, value);
        }
        for (owner, diplomacy) in DIPLOMACY.into_iter().enumerate() {
            p64(&mut bytes, COUNTRIES[owner], ccountry::VTABLE_RVA as u64);
            p32(&mut bytes, COUNTRIES[owner] + ccountry::ID, [3, 2][owner]);
            p64(
                &mut bytes,
                COUNTRIES[owner] + ccountry::DIPLOMACY_STATUS,
                DIPLOMACY[owner] as u64,
            );
            p64(
                &mut bytes,
                DIPLOMACY[owner],
                cdiplomacy_status::VTABLE_RVA as u64,
            );
            p64(
                &mut bytes,
                DIPLOMACY[owner] + cdiplomacy_status::COUNTRY,
                COUNTRIES[owner] as u64,
            );
            let entries: Vec<_> = (0..4).map(|target| status(owner, target) as u64).collect();
            array(
                &mut bytes,
                diplomacy + cdiplomacy_status::RELATIONS,
                0xa000 + owner * 0x40,
                &entries,
            );
            array(
                &mut bytes,
                DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
                owned(owner),
                &[RELATION as u64, RETIRED_RELATION as u64, 0x1a000],
            );
            for target in 0..4 {
                let entry = status(owner, target);
                p64(&mut bytes, entry, crelation_status::VTABLE_RVA as u64);
                p64(
                    &mut bytes,
                    entry + crelation_status::DIPLOMACY_STATUS,
                    DIPLOMACY[owner] as u64,
                );
                p32(
                    &mut bytes,
                    entry + crelation_status::TARGET_COUNTRY,
                    [0, 3, 2, 0][target],
                );
            }
            let entry = status(owner, [2, 1][owner]);
            p64(
                &mut bytes,
                entry + crelation_status::SUBJECT_RELATION,
                RELATION as u64,
            );
            array(
                &mut bytes,
                entry + crelation_status::SUBRELATIONS,
                entry + 0x400,
                &[RELATION as u64],
            );
            if owner == 0 {
                array(
                    &mut bytes,
                    entry + crelation_status::SERIALIZABLE_RELATIONS,
                    entry + 0x420,
                    &[RELATION as u64],
                );
            }
        }
        p64(&mut bytes, RELATION, VTABLE_RVA as u64);
        p32(&mut bytes, RELATION + TYPE_ID, TYPE_VALUE);
        p32(&mut bytes, RELATION + OVERLORD, 3);
        p32(&mut bytes, RELATION + SUBJECT, 2);
        p32(&mut bytes, RELATION + START_HOUR, i32::MIN as u32);
        p32(&mut bytes, RELATION + crelation::END_HOUR, i32::MAX as u32);
        bytes[RELATION + crelation::CANCEL] = 255;
        p64(&mut bytes, RELATION + AUTONOMY_RATIO, i64::MIN as u64);
        p64(&mut bytes, RELATION + AUTONOMY_STATE, DEFINITION as u64);
        let name = DEFINITION + autonomy_state_definition::NAME;
        bytes[name..name + 4].copy_from_slice(&[b'a', 255, 0, b'z']);
        p64(&mut bytes, name + msvc_string::LENGTH, 4);
        p64(&mut bytes, name + msvc_string::CAPACITY, 15);
        p64(&mut bytes, RETIRED_RELATION, VTABLE_RVA as u64);
        p32(&mut bytes, RETIRED_RELATION + TYPE_ID, TYPE_VALUE);
        bytes[RETIRED_RELATION + crelation::RETIRED] = 7;
        p32(&mut bytes, 0x1a000 + TYPE_ID, 0x1234);
        bytes
    }

    fn capture(bytes: &[u8], matrix: bool, output: &mut [Row], names: &mut [u8]) -> NameWalk {
        let memory = SliceMemory { base: 0, bytes };
        if matrix {
            walk_matrix(&memory, 0, GS, output, names)
        } else {
            walk(&memory, 0, GS, output, names)
        }
    }

    #[test]
    fn censuses_preserve_native_direction_full_width_and_literal_bytes() {
        let bytes = fixture();
        for matrix in [false, true] {
            let mut rows = [Row::default(); 2];
            let mut names = [0; 20];
            assert_eq!(
                capture(&bytes, matrix, &mut rows, &mut names),
                NameWalk {
                    outcome: Outcome::Rows { len: 1 },
                    bytes_len: 4,
                    required_bytes: 4
                }
            );
            assert_eq!(
                rows[0],
                Row {
                    overlord: 3,
                    subject: 2,
                    start_hour: i32::MIN,
                    end_hour: i32::MAX,
                    cancel: 255,
                    autonomy_ratio: i64::MIN,
                    state: Name { start: 0, len: 4 }
                }
            );
            assert_eq!(&names[..4], &[b'a', 255, 0, b'z']);
        }
    }

    /// A retired relation still installed at `+0x288` is what the native
    /// subject terms read, so it stays a row; the retired relation left only in
    /// the owned vector (the fixture's second entry) is skipped.
    #[test]
    fn a_retired_subject_counts_while_it_is_installed() {
        let mut bytes = fixture();
        // The owned-only retired relation names the same pair but is not the
        // one installed, so it resolves and is still skipped.
        p32(&mut bytes, RETIRED_RELATION + OVERLORD, 3);
        p32(&mut bytes, RETIRED_RELATION + SUBJECT, 2);
        bytes[RELATION + crelation::RETIRED] = 1;
        for matrix in [false, true] {
            let mut rows = [Row::default(); 2];
            let mut names = [0; 20];
            assert_eq!(
                capture(&bytes, matrix, &mut rows, &mut names).outcome,
                Outcome::Rows { len: 1 }
            );
            assert_eq!((rows[0].overlord, rows[0].subject), (3, 2));
        }
    }

    #[test]
    fn capacity_omits_whole_rows_and_reports_exact_required_bytes() {
        let bytes = fixture();
        for matrix in [false, true] {
            for (row_capacity, name_capacity) in [(0, 4), (1, 3)] {
                let mut rows = [Row::default(); 1];
                let mut names = [77; 4];
                assert_eq!(
                    capture(
                        &bytes,
                        matrix,
                        &mut rows[..row_capacity],
                        &mut names[..name_capacity]
                    ),
                    NameWalk {
                        outcome: Outcome::Truncated {
                            len: 0,
                            unread_rows: 1
                        },
                        bytes_len: 0,
                        required_bytes: 4
                    }
                );
                assert_eq!(names, [77; 4]);
            }
        }
    }

    #[test]
    fn matrix_census_can_detect_a_relation_missing_from_both_owned_roots() {
        let mut bytes = fixture();
        for diplomacy in DIPLOMACY {
            array(
                &mut bytes,
                diplomacy + cdiplomacy_status::OWNED_RELATIONS,
                0,
                &[],
            );
        }
        let mut rows = [Row::default(); 1];
        let mut names = [0; 4];
        assert_eq!(
            capture(&bytes, false, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 0 }
        );
        assert_eq!(
            capture(&bytes, true, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
    }

    #[test]
    fn matrix_batches_authenticate_empty_tail_cells_and_do_not_read_past_count() {
        let mut bytes = fixture();
        bytes.resize(0x24000, 0);
        for (owner, diplomacy) in DIPLOMACY.into_iter().enumerate() {
            let mut entries = vec![status(owner, 0) as u64; 65];
            for (index, entry) in entries[..4].iter_mut().enumerate() {
                *entry = status(owner, index) as u64;
            }
            array(
                &mut bytes,
                diplomacy + cdiplomacy_status::RELATIONS,
                0x20000 + owner * 0x400,
                &entries,
            );
        }
        let mut rows = [Row::default(); 1];
        let mut names = [0; 4];
        assert_eq!(
            capture(&bytes, true, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!((rows[0].overlord, rows[0].subject), (3, 2));
        // The 65th cell has no subject, but an invalid owner must still fail.
        p64(&mut bytes, 0x20000 + 64 * 8, 0x22000);
        p64(&mut bytes, 0x22000, crelation_status::VTABLE_RVA as u64);
        assert!(matches!(
            capture(&bytes, true, &mut rows, &mut names).outcome,
            Outcome::Unavailable {
                reason: "relation status diplomacy mismatch"
            }
        ));
        p64(
            &mut bytes,
            0x22000 + crelation_status::DIPLOMACY_STATUS,
            DIPLOMACY[0] as u64,
        );
        assert_eq!(
            capture(&bytes, true, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
        // Slot 65 is outside the 65-entry array and may contain garbage.
        p64(&mut bytes, 0x20000 + 65 * 8, u64::MAX);
        assert_eq!(
            capture(&bytes, true, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
    }

    #[test]
    fn mismatched_ownership_target_and_invalid_names_are_unavailable() {
        for corruption in 0..5 {
            let mut bytes = fixture();
            match corruption {
                0 => p64(
                    &mut bytes,
                    status(1, 1) + crelation_status::SUBJECT_RELATION,
                    0,
                ),
                1 => p32(
                    &mut bytes,
                    status(0, 2) + crelation_status::TARGET_COUNTRY,
                    3,
                ),
                2 => array(
                    &mut bytes,
                    DIPLOMACY[1] + cdiplomacy_status::OWNED_RELATIONS,
                    owned(1),
                    &[RELATION as u64, RELATION as u64],
                ),
                3 => p64(&mut bytes, RELATION + AUTONOMY_STATE, 0),
                _ => p64(
                    &mut bytes,
                    DEFINITION + autonomy_state_definition::NAME + msvc_string::LENGTH,
                    16,
                ),
            }
            assert!(
                matches!(
                    capture(&bytes, false, &mut [Row::default(); 1], &mut [0; 40]).outcome,
                    Outcome::Unavailable { .. }
                ),
                "corruption {corruption}"
            );
        }
    }

    #[test]
    fn heap_names_have_no_fixed_length_limit_or_lossy_conversion() {
        let mut bytes = fixture();
        let name = DEFINITION + autonomy_state_definition::NAME;
        p64(&mut bytes, name, 0x1c000);
        p64(&mut bytes, name + msvc_string::LENGTH, 128);
        p64(&mut bytes, name + msvc_string::CAPACITY, 128);
        bytes[0x1c000..0x1c080].fill(255);
        let mut names = [0; 128];
        assert_eq!(
            capture(&bytes, false, &mut [Row::default(); 1], &mut names),
            NameWalk {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 128,
                required_bytes: 128
            }
        );
        assert_eq!(names, [255; 128]);
        bytes[0x1c080] = 1;
        assert!(matches!(
            capture(&bytes, false, &mut [Row::default(); 1], &mut names).outcome,
            Outcome::Unavailable { .. }
        ));
    }

    /// Bump-allocated synthetic address space for census-scale fixtures.
    struct Space {
        bytes: Vec<u8>,
    }

    impl Space {
        fn alloc(&mut self, size: usize) -> usize {
            let at = (self.bytes.len() + 15) & !15;
            self.bytes.resize(at + size, 0);
            at
        }

        fn vector(&mut self, header: usize, values: &[u64]) {
            let data = if values.is_empty() {
                0
            } else {
                self.alloc(values.len() * 8)
            };
            array(&mut self.bytes, header, data, values);
        }
    }

    /// `countries` canonical countries; country 1 is the overlord of every
    /// other one, and its owned vector holds all those subjects.
    fn overlord_census(countries: usize) -> (Vec<u8>, Vec<Row>) {
        let mut space = Space {
            bytes: vec![0; 0x2000],
        };
        let census = space.alloc((countries + 1) * 8);
        let links = space.alloc((countries + 1) * 4);
        // 24-byte cells: vtable and id, then a later country's diplomacy
        // pointer (0xf88 = 165 * 24 + 16), so objects overlap without clashing.
        let objects = space.alloc(24 * (countries + 2 + ccountry::DIPLOMACY_STATUS / 24));
        let country = |slot: usize| objects + 24 * slot;
        let mut pointers = vec![u64::MAX];
        pointers.extend((1..=countries).map(|slot| country(slot) as u64));
        array(
            &mut space.bytes,
            GS + cgame_state::COUNTRIES,
            census,
            &pointers,
        );
        p64(
            &mut space.bytes,
            GS + cgame_state::COUNTRY_LINKS,
            links as u64,
        );
        for offset in [8, 12] {
            p32(
                &mut space.bytes,
                GS + cgame_state::COUNTRY_LINKS + offset,
                countries as u32 + 1,
            );
        }
        for slot in 0..=countries {
            p32(&mut space.bytes, links + slot * 4, slot as u32);
        }
        let mut diplomacy = vec![0; countries + 1];
        for (slot, root) in diplomacy.iter_mut().enumerate().skip(1) {
            let at = country(slot);
            p64(&mut space.bytes, at, ccountry::VTABLE_RVA as u64);
            p32(&mut space.bytes, at + ccountry::ID, slot as u32);
            *root = space.alloc(0x290);
            p64(
                &mut space.bytes,
                at + ccountry::DIPLOMACY_STATUS,
                *root as u64,
            );
            p64(
                &mut space.bytes,
                *root,
                cdiplomacy_status::VTABLE_RVA as u64,
            );
            p64(
                &mut space.bytes,
                *root + cdiplomacy_status::COUNTRY,
                at as u64,
            );
        }
        let definition = space.alloc(0x40);
        let name = definition + autonomy_state_definition::NAME;
        space.bytes[name..name + 6].copy_from_slice(b"puppet");
        p64(&mut space.bytes, name + msvc_string::LENGTH, 6);
        p64(&mut space.bytes, name + msvc_string::CAPACITY, 15);
        let status = |space: &mut Space, owner: usize, target: usize, relation: usize, first| {
            let at = space.alloc(0x2a0);
            p64(&mut space.bytes, at, crelation_status::VTABLE_RVA as u64);
            p64(
                &mut space.bytes,
                at + crelation_status::DIPLOMACY_STATUS,
                diplomacy[owner] as u64,
            );
            p32(
                &mut space.bytes,
                at + crelation_status::TARGET_COUNTRY,
                target as u32,
            );
            p64(
                &mut space.bytes,
                at + crelation_status::SUBJECT_RELATION,
                relation as u64,
            );
            space.vector(at + crelation_status::SUBRELATIONS, &[relation as u64]);
            if first {
                space.vector(
                    at + crelation_status::SERIALIZABLE_RELATIONS,
                    &[relation as u64],
                );
            }
            at
        };
        let mut hub_matrix = vec![0_u64; countries + 1];
        let mut hub_owned = Vec::new();
        let mut rows = Vec::new();
        for subject in 2..=countries {
            let relation = space.alloc(0x68);
            p64(&mut space.bytes, relation, VTABLE_RVA as u64);
            p32(&mut space.bytes, relation + TYPE_ID, TYPE_VALUE);
            p32(&mut space.bytes, relation + OVERLORD, 1);
            p32(&mut space.bytes, relation + SUBJECT, subject as u32);
            p32(&mut space.bytes, relation + START_HOUR, subject as u32);
            p32(&mut space.bytes, relation + crelation::END_HOUR, u32::MAX);
            p64(&mut space.bytes, relation + AUTONOMY_RATIO, subject as u64);
            p64(
                &mut space.bytes,
                relation + AUTONOMY_STATE,
                definition as u64,
            );
            hub_matrix[subject] = status(&mut space, 1, subject, relation, true) as u64;
            let back = status(&mut space, subject, 1, relation, false);
            let matrix = space.alloc(16);
            p64(&mut space.bytes, matrix + 8, back as u64);
            p64(
                &mut space.bytes,
                diplomacy[subject] + cdiplomacy_status::RELATIONS,
                matrix as u64,
            );
            for offset in [8, 12] {
                p32(
                    &mut space.bytes,
                    diplomacy[subject] + cdiplomacy_status::RELATIONS + offset,
                    2,
                );
            }
            space.vector(
                diplomacy[subject] + cdiplomacy_status::OWNED_RELATIONS,
                &[relation as u64],
            );
            hub_owned.push(relation as u64);
            rows.push(Row {
                overlord: 1,
                subject: subject as u32,
                start_hour: subject as i32,
                end_hour: -1,
                autonomy_ratio: subject as i64,
                state: Name {
                    start: rows.len() * 6,
                    len: 6,
                },
                ..Row::default()
            });
        }
        space.vector(diplomacy[1] + cdiplomacy_status::RELATIONS, &hub_matrix);
        space.vector(
            diplomacy[1] + cdiplomacy_status::OWNED_RELATIONS,
            &hub_owned,
        );
        (space.bytes, rows)
    }

    /// Counts guarded reads of one address range.
    struct CountingMemory<'a> {
        bytes: &'a [u8],
        watched: std::ops::Range<usize>,
        hits: std::cell::Cell<usize>,
    }

    impl Memory for CountingMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if self.watched.contains(&address) {
                self.hits.set(self.hits.get() + 1);
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    #[test]
    fn owned_subject_walk_matches_the_plain_walk_and_scans_each_owner_once() {
        let (bytes, expected) = overlord_census(600);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut plain = vec![Row::default(); 600];
        let mut plain_names = vec![0; 600 * 6];
        let plain_walk = walk(&memory, 0, GS, &mut plain, &mut plain_names);
        assert_eq!(plain_walk.outcome, Outcome::Rows { len: 599 });
        assert_eq!(plain[..599], expected[..]);
        for grown in [false, true] {
            let mut scratch = OwnedSnapshotScratch::default();
            if grown {
                let mut rows = vec![Row::default(); 600];
                let mut names = vec![0; 600 * 6];
                walk_owned(&memory, 0, GS, &mut rows, &mut names, &mut scratch);
                scratch.grow(0);
            }
            let mut rows = vec![Row::default(); 600];
            let mut names = vec![0; 600 * 6];
            let owned = walk_owned(&memory, 0, GS, &mut rows, &mut names, &mut scratch);
            assert_eq!(owned, plain_walk);
            assert_eq!(rows, plain);
            assert_eq!(names, plain_names);
            // Empty storage is the plain walk; grown storage installs the
            // overlord's vector once and answers every later visit from it.
            assert_eq!(scratch.stats().installed > 0, grown);
        }
    }

    #[test]
    fn owned_subject_walk_at_census_scale_is_linear_in_relations() {
        // 15,000 countries, one overlord of 14,999 subjects: the plain walk
        // scans the overlord's 14,999-entry vector for every subject from
        // both endpoints (about 4.5e8 element reads).
        const COUNTRIES: usize = 15_000;
        let (bytes, expected) = overlord_census(COUNTRIES);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        // A cold pass (no storage) fails cleanly once its complete rescans
        // pass the census budget, instead of stalling on 4.5e8 reads; it
        // still records the whole world's demand. A small test budget keeps
        // this fixture fast.
        let mut scratch = OwnedSnapshotScratch::default();
        scratch.rescan_override = Some(1_000_000);
        let mut rows = vec![Row::default(); COUNTRIES];
        let mut names = vec![0; COUNTRIES * 6];
        let hub = {
            let reader = Reader::new(&memory, 0, GS).unwrap();
            let country = reader.countries.at(1).unwrap().unwrap();
            let diplomacy = reader.diplomacy(country).unwrap();
            reader
                .array(diplomacy, cdiplomacy_status::OWNED_RELATIONS)
                .unwrap()
        };
        let counting = CountingMemory {
            bytes: &bytes,
            watched: hub.data..hub.data + hub.count as usize * 8,
            hits: std::cell::Cell::new(0),
        };
        let cold = walk_owned(&counting, 0, GS, &mut rows, &mut names, &mut scratch);
        assert_eq!(cold, unavailable(SUBJECT_MEMBERSHIP.budget));
        assert!(counting.hits.get() <= 1_000_000 + 3 * hub.count as usize);
        scratch.grow(0);
        let allocated = scratch.allocated_bytes();
        counting.hits.set(0);
        let result = walk_owned(&counting, 0, GS, &mut rows, &mut names, &mut scratch);
        assert_eq!(
            result,
            NameWalk {
                outcome: Outcome::Rows { len: COUNTRIES - 1 },
                bytes_len: (COUNTRIES - 1) * 6,
                required_bytes: (COUNTRIES - 1) * 6,
            }
        );
        assert_eq!(rows[..COUNTRIES - 1], expected[..]);
        assert!(
            names[..(COUNTRIES - 1) * 6]
                .chunks(6)
                .all(|name| name == b"puppet")
        );
        assert_eq!(scratch.allocated_bytes(), allocated);
        // The outer census reads each hub entry once; the snapshot adds one
        // more complete scan, instead of 2 x 14,999 scans.
        assert!(
            counting.hits.get() <= 2 * hub.count as usize,
            "{}",
            counting.hits.get()
        );
        assert_eq!(scratch.stats().installed, 1);
        // Output capacity still truncates honestly, never by country count.
        let short = walk_owned(&memory, 0, GS, &mut rows[..10], &mut names, &mut scratch);
        assert_eq!(
            short.outcome,
            Outcome::Truncated {
                len: 10,
                unread_rows: (COUNTRIES - 11) as u32
            }
        );
    }
}
