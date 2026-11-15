//! Per-country character state: appointed and available advisors and active
//! and retired character records. The scientist roster reads the same owner.
//!
//! Appointment authenticates the owner, preserves sorted advisor occurrences,
//! and scopes on-add effects to that country. Country modifiers traverse the
//! appointed array in order. Serialization retains advisor slot text and full
//! character references when appointments exist. Sort order uses idea hash then
//! idea text; capture preserves the observed order rather than deriving it.
//! A persistent advisor role alone does not establish appointment. Available
//! advisors use a separate array, whose exact-name lookup needs character and
//! idea operands but no slot bytes.
//!
//! One country pass copies active and retired records with independent failures.
//! Synthetic fixtures cover these walks; live capture and fresh-save agreement
//! remain unverified.

use crate::{
    Memory, cadvisor,
    ccharacter::{self, Literal, NameWalk},
    ccountry,
    cpdx_array::Array,
    cpdx_lexer, read, read_slice,
    walk::Outcome,
};
use engine_sites::Rva;

/// Country-character vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a0_6b48);
/// Owning country pointer used for appointment scope.
pub const OWNER: usize = 0x08;
/// Appointed-advisor pointer array with signed capacity and count.
pub const APPOINTED_ADVISORS: usize = 0x58;
/// Available-advisor pointer array maintained by role changes, retirement
/// and transfers. Exact-name queries walk this roster.
pub const AVAILABLE_ADVISORS: usize = 0x40;

/// One appointed advisor, in native array order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Raw id of the CCountry that owns this CCountryCharacters.
    pub country: u32,
    /// Position in the appointed array; the engine's own order.
    pub ordinal: u32,
    /// CCharacter numeric id behind `cadvisor::CHARACTER`.
    pub character: u32,
    pub idea_start: usize,
    pub idea_len: usize,
    pub slot_start: usize,
    pub slot_len: usize,
}

/// Pointers copied per guarded read.
const POINTERS_PER_READ: usize = 32;

/// Active-character records sorted by character name token, with null
/// characters ordered as zero. Token lookup selects the first occurrence;
/// serialization retains every record.
pub const ACTIVE_CHARACTERS: usize = 0x10;
/// Record stride; each holds a character pointer and four boolean bytes.
pub const ACTIVE_RECORD_STRIDE: usize = 0x10;

/// One active-character record, in native array order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ActiveRow {
    /// Raw id of the CCountry that owns this CCountryCharacters.
    pub country: u32,
    /// Position in the active array; the engine's own order.
    pub ordinal: u32,
    /// CCharacter numeric id, `None` for a null record.
    pub character: Option<u32>,
}

/// The authenticated CCountryCharacters of `country`, `None` for the engine's
/// null object.
pub(crate) fn characters_object(
    memory: &impl Memory,
    country: &ccountry::Country,
    vtable: u64,
) -> Result<Option<usize>, &'static str> {
    let object = country
        .pointer
        .checked_add(ccountry::CHARACTERS)
        .and_then(|address| read::<u64>(memory, address))
        .ok_or("country characters pointer unreadable")?;
    let object = usize::try_from(object).map_err(|_| "country characters pointer overflow")?;
    if object == 0 {
        return Ok(None);
    }
    let head = read::<[u8; 16]>(memory, object).ok_or("country characters unreadable")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != vtable {
        return Err("country characters vtable mismatch");
    }
    if u64::from_le_bytes(head[OWNER..OWNER + 8].try_into().unwrap()) != country.pointer as u64 {
        return Err("country characters owner is another country");
    }
    Ok(Some(object))
}

/// Retired records sharing the active layout and name-token order.
/// Retirement inserts at the lower bound and removes the active record;
/// retired lookup searches that same token order.
pub const RETIRED_CHARACTERS: usize = 0x28;

/// One table's rows of the record walk and its own first failure.
struct Records<'a> {
    output: crate::walk::Buffer<'a, ActiveRow>,
    len: usize,
    unread_rows: u32,
    failure: Option<&'static str>,
}

impl<'a> Records<'a> {
    fn new(output: crate::walk::Buffer<'a, ActiveRow>) -> Self {
        Self {
            output,
            len: 0,
            unread_rows: 0,
            failure: None,
        }
    }

    /// Appends one country's records at `address`, unless this table already
    /// failed; a failure here fails this table alone.
    fn read(&mut self, memory: &impl Memory, base: usize, country: u32, address: usize) {
        if self.failure.is_none()
            && let Err(reason) = self.read_array(memory, base, country, address)
        {
            self.failure = Some(reason);
        }
    }

    fn read_array(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: u32,
        address: usize,
    ) -> Result<(), &'static str> {
        let records = Array::read(memory, address, ACTIVE_RECORD_STRIDE)
            .ok_or("character record array unreadable or invalid")?;
        self.output.ensure(
            self.len
                .checked_add(records.count as usize)
                .ok_or("character record count overflow")?,
        )?;
        let mut batch = [0_u8; POINTERS_PER_READ * ACTIVE_RECORD_STRIDE];
        let mut headers = [[0_u8; 16]; POINTERS_PER_READ];
        let mut first = 0;
        while first < records.count {
            if self.len == self.output.len() {
                self.unread_rows = self
                    .unread_rows
                    .checked_add(records.count - first)
                    .ok_or("character record count overflow")?;
                break;
            }
            let count = ((records.count - first) as usize)
                .min(POINTERS_PER_READ)
                .min(self.output.len() - self.len);
            let address = records
                .element(first, ACTIVE_RECORD_STRIDE)
                .ok_or("character record address overflow")?;
            let batch = &mut batch[..count * ACTIVE_RECORD_STRIDE];
            if !read_slice(memory, address, batch) {
                return Err("character records unreadable");
            }
            // The characters are scattered objects: one guarded, prefetched
            // batch copies every nonnull record's identity header so their
            // misses overlap. Records are then checked in order, so the first
            // fault is that of a read per record.
            let pointers: [usize; POINTERS_PER_READ] = std::array::from_fn(|index| {
                batch
                    .get(index * ACTIVE_RECORD_STRIDE..index * ACTIVE_RECORD_STRIDE + 8)
                    .map_or(0, |pointer| {
                        u64::from_le_bytes(pointer.try_into().unwrap()) as usize
                    })
            });
            let mut slots = headers.iter_mut().enumerate();
            let identified = memory
                .copy_scattered_prefetched(
                    pointers,
                    &mut std::array::from_fn(|_| {
                        let (index, header) = slots.next().unwrap();
                        &mut header[..if index < count && pointers[index] != 0 {
                            16
                        } else {
                            0
                        }]
                    }),
                )
                .min(count);
            for (offset, &pointer) in pointers[..count].iter().enumerate() {
                let character = if pointer == 0 {
                    None
                } else if offset >= identified {
                    return Err("character identity unreadable");
                } else {
                    Some(ccharacter::identity_from_header(base, headers[offset])?)
                };
                self.output[self.len] = ActiveRow {
                    country,
                    ordinal: first + offset as u32,
                    character,
                };
                self.len += 1;
            }
            first += count as u32;
        }
        Ok(())
    }

    /// This table's outcome; `shared` is a failure of the country pass both
    /// tables share, which a table that already failed on its own does not
    /// take.
    fn outcome(&self, shared: Option<&'static str>) -> Outcome {
        match self.failure.or(shared) {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.unread_rows == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread_rows,
            },
        }
    }
}

pub fn count_records(memory: &impl Memory, base: usize, game_state: usize) -> [Outcome; 2] {
    walk_records_buffer(memory, base, game_state, crate::walk::Buffer::Count, crate::walk::Buffer::Count)
}

/// Worker storage grows from the original native headers before publication.
pub fn walk_records(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    active: &mut Vec<ActiveRow>,
    retired: &mut Vec<ActiveRow>,
) -> [Outcome; 2] {
    walk_records_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(active),
        crate::walk::Buffer::Growing(retired),
    )
}

pub fn walk_records_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    active: crate::walk::Buffer<'_, ActiveRow>,
    retired: crate::walk::Buffer<'_, ActiveRow>,
) -> [Outcome; 2] {
    let mut tables = [Records::new(active), Records::new(retired)];
    let result = (|| {
        let characters_vtable =
            engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let Some(object) = characters_object(memory, &country, characters_vtable)? else {
                continue;
            };
            for (table, offset) in tables
                .iter_mut()
                .zip([ACTIVE_CHARACTERS, RETIRED_CHARACTERS])
            {
                let address = object
                    .checked_add(offset)
                    .ok_or("character record array address overflow")?;
                table.read(memory, base, country.id, address);
            }
        }
        Ok(())
    })();
    let shared = result.err();
    tables.each_ref().map(|table| table.outcome(shared))
}

/// Worker storage grows from the original native headers before publication.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    bytes: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    match walk_inner(
        memory,
        base,
        game_state,
        output,
        bytes,
        APPOINTED_ADVISORS,
        true,
        &mut Fresh,
        &|_| true,
    ) {
        Ok(walk) => walk,
        Err(reason) => NameWalk {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        },
    }
}

/// One authenticated country's available-advisor array, before any of its
/// advisors is read: the country, its `CCountryCharacters` and the array's
/// data pointer and count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pool {
    pub country: u32,
    pub object: usize,
    pub data: usize,
    pub count: u32,
}

/// A caller's held available-advisor rows per country
/// ([`walk_available_carried`]).
pub trait AvailableCarry {
    /// Registers an authenticated pool before any advisor is read: a plan
    /// token and, when its held rows may be carried, their row and byte
    /// counts.
    fn plan(&mut self, pool: &Pool) -> (usize, Option<(usize, usize)>);
    /// Writes a carryable plan's rows, their idea offsets starting at
    /// `bytes_start`, and their bytes.
    fn carry(&mut self, token: usize, rows: &mut [Row], bytes: &mut [u8], bytes_start: usize);
    /// One advisor of a pool read afresh: its character, before that
    /// character's identity and the advisor's idea bytes are read.
    fn advisor(&mut self, token: usize, character: usize);
    /// A pool read afresh and whole: its rows and their bytes, whose idea
    /// offsets start at `bytes_start`.
    fn read(&mut self, token: usize, rows: &[Row], bytes: &[u8], bytes_start: usize);
}

/// No held rows: every pool is read.
pub struct Fresh;
impl AvailableCarry for Fresh {
    fn plan(&mut self, _: &Pool) -> (usize, Option<(usize, usize)>) {
        (0, None)
    }
    fn carry(&mut self, _: usize, _: &mut [Row], _: &mut [u8], _: usize) {}
    fn advisor(&mut self, _: usize, _: usize) {}
    fn read(&mut self, _: usize, _: &[Row], _: &[u8], _: usize) {}
}

/// [`walk_available`] for the countries `select` admits by native
/// country id, with `carry` supplying the rows of pools whose held read it
/// proves current: every selected country, `CCountryCharacters` and array
/// header is still read and authenticated, and a carried pool's advisors,
/// characters and idea bytes are not. An unselected country is not read and
/// has no rows; a selected country's rows equal the whole walk's.
pub fn walk_available_carried(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
    carry: &mut impl AvailableCarry,
    select: &dyn Fn(u32) -> bool,
) -> NameWalk {
    walk_inner(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
        AVAILABLE_ADVISORS,
        false,
        carry,
        select,
    )
    .unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

/// Worker storage grows from the original native headers before publication.
pub fn walk_available(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    walk_available_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
    )
}

pub fn walk_available_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    bytes: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    walk_inner(
        memory,
        base,
        game_state,
        output,
        bytes,
        AVAILABLE_ADVISORS,
        false,
        &mut Fresh,
        &|_| true,
    )
    .unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

/// Live header totals and exact idea lengths; no row storage or allocations.
pub fn demand_available(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<(usize, usize), &'static str> {
    demand_advisors(memory, base, game_state, AVAILABLE_ADVISORS, false)
}
pub fn demand_appointed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<(usize, usize), &'static str> {
    demand_advisors(memory, base, game_state, APPOINTED_ADVISORS, true)
}

fn demand_advisors(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roster: usize,
    slots: bool,
) -> Result<(usize, usize), &'static str> {
    let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut rows = 0_usize;
    let mut bytes = 0_usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let Some(object) = characters_object(memory, &country, vtable)? else {
            continue;
        };
        let array = Array::read(memory, ccharacter::at(object, roster)?, 8)
            .ok_or("available advisor array unreadable or invalid")?;
        rows = rows
            .checked_add(array.count as usize)
            .ok_or("available advisor count overflow")?;
        for ordinal in 0..array.count {
            let pointer = array
                .element(ordinal, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or("available advisor pointer unreadable or null")?;
            let address = ccharacter::at(pointer, cadvisor::IDEA_TOKEN)?;
            let header =
                read::<[u8; 32]>(memory, address).ok_or("available advisor idea unreadable")?;
            let idea = Literal::from_header(address, header)?;
            Literal::validate(memory, [&idea])?;
            let slot = if slots {
                Literal::read(memory, ccharacter::at(pointer, cadvisor::SLOT)?)?.len
            } else {
                0
            };
            bytes = bytes
                .checked_add(
                    idea.len
                        .checked_add(slot)
                        .ok_or("advisor string size overflow")?,
                )
                .ok_or("available advisor string size overflow")?;
        }
    }
    Ok((rows, bytes))
}

#[allow(clippy::too_many_arguments)]
fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut bytes: crate::walk::Buffer<'_, u8>,
    roster: usize,
    slots: bool,
    carry: &mut impl AvailableCarry,
    select: &dyn Fn(u32) -> bool,
) -> Result<NameWalk, &'static str> {
    let reason = |appointed, available| if slots { appointed } else { available };
    let module = |rva: Rva| {
        engine_sites::at(base, rva)
            .map(|address| address as u64)
            .ok_or("module address overflow")
    };
    let characters_vtable = module(VTABLE_RVA)?;
    let advisor_vtable = module(cadvisor::VTABLE_RVA)?;
    let character_vtable = module(ccharacter::VTABLE_RVA)?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut len = 0_usize;
    let mut bytes_len = 0_usize;
    let mut required_bytes = 0;
    let mut unread_rows = 0_u32;
    let mut pointers = [0_u8; POINTERS_PER_READ * 8];
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        if !select(country.id) {
            continue;
        }
        let object = country
            .pointer
            .checked_add(ccountry::CHARACTERS)
            .and_then(|address| read::<u64>(memory, address))
            .ok_or("country characters pointer unreadable")?;
        let object = usize::try_from(object).map_err(|_| "country characters pointer overflow")?;
        if object == 0 {
            continue;
        }
        let head = read::<[u8; 16]>(memory, object).ok_or("country characters unreadable")?;
        if u64::from_le_bytes(head[..8].try_into().unwrap()) != characters_vtable {
            return Err("country characters vtable mismatch");
        }
        if u64::from_le_bytes(head[OWNER..OWNER + 8].try_into().unwrap()) != country.pointer as u64
        {
            return Err("country characters owner is another country");
        }
        let advisors = object
            .checked_add(roster)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or(reason(
                "appointed advisor array unreadable or invalid",
                "available advisor array unreadable or invalid",
            ))?;
        output.ensure(
            len.checked_add(advisors.count as usize)
                .ok_or("advisor row count overflow")?,
        )?;
        if unread_rows != 0 {
            unread_rows = unread_rows.checked_add(advisors.count).ok_or(reason(
                "appointed advisor count overflow",
                "available advisor count overflow",
            ))?;
            continue;
        }
        let (token, carried) = carry.plan(&Pool {
            country: country.id,
            object,
            data: advisors.data,
            count: advisors.count,
        });
        if let Some((rows, needed)) = carried {
            let end = len.checked_add(rows).ok_or("advisor row count overflow")?;
            let bytes_end = bytes_len
                .checked_add(needed)
                .ok_or("available advisor string size overflow")?;
            if rows == advisors.count as usize && output.ensure(end)? && bytes.ensure(bytes_end)? {
                carry.carry(
                    token,
                    &mut output[len..end],
                    &mut bytes[bytes_len..bytes_end],
                    bytes_len,
                );
                len = end;
                bytes_len = bytes_end;
                continue;
            }
        }
        let (pool_start, pool_bytes) = (len, bytes_len);
        let mut first = 0;
        while first < advisors.count && unread_rows == 0 {
            let count = ((advisors.count - first) as usize).min(POINTERS_PER_READ);
            let address = advisors.element(first, 8).ok_or(reason(
                "appointed advisor address overflow",
                "available advisor address overflow",
            ))?;
            let batch = &mut pointers[..count * 8];
            if !read_slice(memory, address, batch) {
                return Err(reason(
                    "appointed advisor pointers unreadable",
                    "available advisor pointers unreadable",
                ));
            }
            for (offset, pointer) in batch.chunks_exact(8).enumerate() {
                let ordinal = first + offset as u32;
                if len == output.len() {
                    unread_rows = advisors.count - ordinal;
                    required_bytes = required_bytes.max(bytes_len);
                    break;
                }
                let role = usize::try_from(u64::from_le_bytes(pointer.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or(reason(
                        "appointed advisor pointer null",
                        "available advisor pointer null",
                    ))?;
                let mut vtable = [0; 8];
                let mut parent = [0; 8];
                let mut idea_header = [0; 32];
                let mut slot_header = [0; 32];
                if memory.copy_scattered(
                    [
                        role,
                        ccharacter::at(role, cadvisor::CHARACTER)?,
                        ccharacter::at(role, cadvisor::IDEA_TOKEN)?,
                        if slots {
                            ccharacter::at(role, cadvisor::SLOT)?
                        } else {
                            role
                        },
                    ],
                    &mut [
                        &mut vtable,
                        &mut parent,
                        &mut idea_header,
                        if slots { &mut slot_header } else { &mut [] },
                    ],
                ) != 4
                {
                    return Err(reason(
                        "appointed advisor fields unreadable",
                        "available advisor fields unreadable",
                    ));
                }
                if u64::from_le_bytes(vtable) != advisor_vtable {
                    return Err(reason(
                        "appointed advisor vtable mismatch",
                        "available advisor vtable mismatch",
                    ));
                }
                let character = usize::try_from(u64::from_le_bytes(parent))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or(reason(
                        "appointed advisor has a null character",
                        "available advisor has a null character",
                    ))?;
                carry.advisor(token, character);
                let identity = read::<[u8; 16]>(memory, character).ok_or(reason(
                    "appointed advisor character unreadable",
                    "available advisor character unreadable",
                ))?;
                if u64::from_le_bytes(identity[..8].try_into().unwrap()) != character_vtable
                    || u32::from_le_bytes(
                        identity[ccharacter::TYPE..ccharacter::ID]
                            .try_into()
                            .unwrap(),
                    ) != ccharacter::REFERENCE_TYPE
                {
                    return Err(reason(
                        "appointed advisor character identity mismatch",
                        "available advisor character identity mismatch",
                    ));
                }
                let id = u32::from_le_bytes(identity[ccharacter::ID..16].try_into().unwrap());
                if id == 0 {
                    return Err(reason(
                        "appointed advisor character has a zero id",
                        "available advisor character has a zero id",
                    ));
                }
                let idea =
                    Literal::from_header(ccharacter::at(role, cadvisor::IDEA_TOKEN)?, idea_header)?;
                let slot_text = if slots {
                    Literal::from_header(ccharacter::at(role, cadvisor::SLOT)?, slot_header)?
                } else {
                    slot_header[24..].copy_from_slice(&15_u64.to_le_bytes());
                    Literal::from_header(role, slot_header)?
                };
                Literal::validate(memory, [&idea, &slot_text])?;
                let needed = idea.len.checked_add(slot_text.len).ok_or(reason(
                    "appointed advisor string size overflow",
                    "available advisor string size overflow",
                ))?;
                let end = bytes_len.checked_add(needed).ok_or(reason(
                    "appointed advisor string size overflow",
                    "available advisor string size overflow",
                ))?;
                bytes.ensure(end)?;
                if end > bytes.len() {
                    // A declared length is not allocation evidence: growth is
                    // certified from read bytes only, one bounded step per
                    // string that does not fit (as the modifier definitions
                    // do), continuing while each string is certified whole.
                    let mut used = bytes_len;
                    for literal in [&idea, &slot_text] {
                        let end = used + literal.len;
                        bytes.ensure(end)?;
                        if end > bytes.len() {
                            let certified = cpdx_lexer::probe_growth_prefix(
                                memory,
                                literal.source(),
                                literal.len,
                                used,
                                bytes.len(),
                            )
                            .map_err(|_| {
                                reason(
                                    "appointed advisor string bytes unreadable",
                                    "available advisor string bytes unreadable",
                                )
                            })?;
                            if certified < end {
                                used = certified;
                                break;
                            }
                        }
                        used = end;
                    }
                    let required = used;
                    unread_rows = advisors.count - ordinal;
                    required_bytes = required_bytes.max(required);
                    break;
                }
                Literal::copy_all(memory, [&idea, &slot_text], &mut bytes[bytes_len..end])?;
                output[len] = Row {
                    country: country.id,
                    ordinal,
                    character: id,
                    idea_start: bytes_len,
                    idea_len: idea.len,
                    slot_start: bytes_len + idea.len,
                    slot_len: slot_text.len,
                };
                len += 1;
                bytes_len = end;
            }
            first += count as u32;
        }
        if unread_rows == 0 {
            carry.read(
                token,
                &output[pool_start..len],
                &bytes[pool_bytes..bytes_len],
                pool_bytes,
            );
        }
    }
    Ok(NameWalk {
        outcome: if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        },
        bytes_len,
        required_bytes: required_bytes.max(bytes_len),
    })
}

#[cfg(test)]
mod tests;
