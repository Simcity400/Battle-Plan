//! Separate technology definition objects with indexed identities.
//! Every declaration has its own template; case-insensitive lexer handles may repeat.

/// Technology template MSVC id string.
pub const ID: usize = 0x10;
/// Technology template id-string capacity, u64.
pub const ID_CAPACITY: usize = 0x28;
/// One-based positive i32 database index; the instance slot is this index minus one.
pub const DATABASE_INDEX: usize = 0x38;
/// Technology template u32 handle, copied into its instances.
pub const TECHNOLOGY_HANDLE: usize = 0x3c;
/// Authored technology year, signed i32.
pub const YEAR: usize = 0x3d8;
/// Signed i32 maximum researched level.
pub const MAX_LEVEL: usize = 0x3dc;
/// Technology template research cost, i64 fixed5.
pub const RESEARCH_COST: usize = 0x3e0;

/// Template-first ahead-of-time UI helper.
pub const UI_YEAR_HELPER_RVA: usize = 0x00ab_eaa0;
/// Ahead-of-time UI helper entry bytes.
pub const UI_YEAR_HELPER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x41, 0x56, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x3e,
];
/// Template-first effective ahead-of-time value.
pub const EFFECTIVE_VALUE_PRODUCER_RVA: usize = 0x00ab_e4c0;
/// Effective ahead-of-time producer entry bytes.
pub const EFFECTIVE_VALUE_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];

use crate::{Memory, msvc_string, read, walk::Outcome};

/// Raw header spans copied on the game thread with no interpretation.
/// 32-byte MSVC string object plus 8-byte identity plus 16-byte operands.
pub const HEADER_BYTES: usize = 56;
/// Game-thread raw copy of the three fixed template spans in walker order.
/// Returns address-overflow errors unchanged; short copies mean unreadable.
pub fn copy_header(
    memory: &impl Memory,
    pointer: usize,
    out: &mut [u8; HEADER_BYTES],
) -> Result<usize, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("technology template address overflow")
    };
    let (string, rest) = out.split_at_mut(32);
    let (identity, operands) = rest.split_at_mut(8);
    let mut string_ref: &mut [u8] = string;
    let mut identity_ref: &mut [u8] = identity;
    let mut operands_ref: &mut [u8] = operands;
    Ok(memory.copy_scattered_prefetched(
        [at(ID)?, at(DATABASE_INDEX)?, at(YEAR)?],
        &mut [&mut string_ref, &mut identity_ref, &mut operands_ref],
    ))
}

/// Decode-side interpretation of an already-copied header with identical faults.
pub struct DecodedHeader {
    pub database_index: i32,
    pub year: i32,
    pub max_level: i32,
    pub research_cost_raw: i64,
    pub id_len: usize,
    pub source: usize,
}

pub fn decode_header(
    pointer: usize,
    header: &[u8; HEADER_BYTES],
    handle: u32,
) -> Result<DecodedHeader, &'static str> {
    let string = &header[..32];
    let identity = &header[32..40];
    let operands = &header[40..56];
    if u32::from_le_bytes(identity[4..].try_into().unwrap()) != handle {
        return Err("technology template handle mismatch");
    }
    let database_index = i32::from_le_bytes(identity[..4].try_into().unwrap());
    if database_index <= 0 {
        return Err("technology template database index is not positive");
    }
    let id_len = usize::try_from(u64::from_le_bytes(string[16..24].try_into().unwrap()))
        .map_err(|_| "technology id length overflow")?;
    let capacity = u64::from_le_bytes(string[24..].try_into().unwrap());
    if id_len == 0 || id_len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("technology id string header invalid");
    }
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("technology template address overflow")
    };
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        at(ID)?
    } else {
        usize::try_from(u64::from_le_bytes(string[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("technology id pointer invalid")?
    };
    Ok(DecodedHeader {
        database_index,
        year: i32::from_le_bytes(operands[..4].try_into().unwrap()),
        max_level: i32::from_le_bytes(operands[4..8].try_into().unwrap()),
        research_cost_raw: i64::from_le_bytes(operands[8..].try_into().unwrap()),
        id_len,
        source,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub technology_handle: u32,
    pub database_index: i32,
    pub year: i32,
    pub max_level: i32,
    pub research_cost_raw: i64,
    pub id_start: usize,
    pub id_len: usize,
}

/// Caller-owned dictionary scratch. Native pointers are acquisition data only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub row: Row,
    pub template_pointer: usize,
    pub copied: bool,
    occupied: bool,
    // Local acquisition index only; these never enter a replay or survive a
    // Dictionary::new. A generation separates each country's roster.
    instance_generation: u32,
    // Independent pointer-index slots share the allocation with the full-key
    // definition index; growing rehashes both from these owned fields.
    source_pointer: usize,
    source_row: u32,
    source_generation: u32,
}

/// Every declaration has a unique database index and a case-insensitively
/// interned handle that may repeat.
/// One dictionary per callback, shared by every country's walk. Both indexes
/// compare complete native keys; storage grows on the acquisition worker.
/// No pointer or membership survives Dictionary::new.
pub struct Dictionary<'a> {
    pub entries: &'a mut Vec<Entry>,
    pub names: &'a mut Vec<u8>,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub required_capacity: usize,
    used: usize,
    copied: usize,
    error: Option<&'static str>,
    instance_generation: u32,
}

impl<'a> Dictionary<'a> {
    pub fn new(entries: &'a mut Vec<Entry>, names: &'a mut Vec<u8>) -> Self {
        entries.fill(Entry::default());
        Self {
            entries,
            names,
            bytes_len: 0,
            required_bytes: 0,
            required_capacity: 0,
            used: 0,
            copied: 0,
            error: None,
            instance_generation: 0,
        }
    }

    pub fn outcome(&self) -> Outcome {
        if let Some(reason) = self.error {
            Outcome::Unavailable { reason }
        } else {
            Outcome::Rows { len: self.copied }
        }
    }

    fn index_slot(entries: &[Entry], database_index: i32) -> Option<usize> {
        if entries.is_empty() {
            return None;
        }
        let first = (database_index as usize)
            .wrapping_mul(0x9e37_79b9)
            .rotate_right(16)
            % entries.len();
        (0..entries.len())
            .map(|step| (first + step) % entries.len())
            .find(|&slot| {
                !entries[slot].occupied || entries[slot].row.database_index == database_index
            })
    }

    fn pointer_slot(entries: &[Entry], pointer: usize, generation: u32) -> Option<usize> {
        if entries.is_empty() {
            return None;
        }
        let first = pointer.wrapping_mul(0x9e37_79b9).rotate_right(16) % entries.len();
        (0..entries.len())
            .map(|step| (first + step) % entries.len())
            .find(|&slot| {
                entries[slot].source_generation != generation
                    || entries[slot].source_pointer == pointer
            })
    }

    pub fn entry(&self, database_index: i32) -> Option<&Entry> {
        let entry = &self.entries[Self::index_slot(self.entries, database_index)?];
        entry.occupied.then_some(entry)
    }

    fn ensure_entry(&mut self, database_index: i32) -> Result<usize, &'static str> {
        if let Some(slot) = Self::index_slot(self.entries, database_index)
            && self.entries[slot].occupied
        {
            return Ok(slot);
        }
        if self.used >= self.entries.len() / 2 {
            let size = self
                .entries
                .len()
                .checked_mul(2)
                .ok_or("technology dictionary capacity overflow")?
                .max(2);
            let mut grown = Vec::new();
            crate::allocation::reserve_exact(&mut grown, size)
                .map_err(|_| "technology dictionary allocation failed")?;
            grown.resize(size, Entry::default());
            // Growing changes both hash moduli. Rebuild only from owned values,
            // including current-country pointer membership installed so far.
            for old in self.entries.iter() {
                if old.occupied {
                    let slot = Self::index_slot(&grown, old.row.database_index).unwrap();
                    let entry = &mut grown[slot];
                    entry.row = old.row;
                    entry.template_pointer = old.template_pointer;
                    entry.copied = old.copied;
                    entry.occupied = true;
                    entry.instance_generation = old.instance_generation;
                }
                if self.instance_generation != 0
                    && old.source_generation == self.instance_generation
                {
                    let slot =
                        Self::pointer_slot(&grown, old.source_pointer, self.instance_generation)
                            .unwrap();
                    grown[slot].source_pointer = old.source_pointer;
                    grown[slot].source_row = old.source_row;
                    grown[slot].source_generation = old.source_generation;
                }
            }
            *self.entries = grown;
        }
        self.required_capacity = self.entries.len();
        let slot = Self::index_slot(self.entries, database_index).unwrap();
        self.entries[slot].occupied = true;
        self.entries[slot].row.database_index = database_index;
        self.used += 1;
        Ok(slot)
    }

    pub(crate) fn admit(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        handle: u32,
        database_index: i32,
    ) {
        if let Err(reason) = self.read_definition(memory, pointer, handle, database_index) {
            self.error.get_or_insert(reason);
        }
    }

    pub fn begin_roster(&mut self) {
        self.instance_generation = self.instance_generation.wrapping_add(1);
        if self.instance_generation == 0 {
            for entry in self.entries.iter_mut() {
                entry.instance_generation = 0;
                entry.source_generation = 0;
            }
            self.instance_generation = 1;
        }
    }

    #[inline]
    pub(crate) fn admit_instance(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        template: usize,
        handle: u32,
        database_index: i32,
        previous: &[crate::ctechnology::Row],
    ) -> Result<(), &'static str> {
        let slot = match self.ensure_entry(database_index) {
            Ok(slot) => slot,
            Err(reason) => {
                self.error.get_or_insert(reason);
                if previous
                    .iter()
                    .any(|row| row.database_index == database_index)
                {
                    return Err("duplicate technology occurrence in country roster");
                }
                return Ok(());
            }
        };
        let entry = &mut self.entries[slot];
        if entry.instance_generation == self.instance_generation {
            return Err("duplicate technology occurrence in country roster");
        }
        entry.instance_generation = self.instance_generation;
        let read_definition = entry.template_pointer == 0;
        if entry.copied && entry.row.technology_handle != handle {
            self.error
                .get_or_insert("technology template handle mismatch");
        }
        if !read_definition && entry.template_pointer != template {
            self.error
                .get_or_insert("technology occurrence has inconsistent template pointers");
        }
        if self.instance_pointer_row(pointer).is_some() {
            return Err("duplicate technology pointer in country roster");
        }
        let slot = Self::pointer_slot(self.entries, pointer, self.instance_generation)
            .expect("technology pointer index has space for admitted definitions");
        self.entries[slot].source_pointer = pointer;
        self.entries[slot].source_row = previous.len() as u32;
        self.entries[slot].source_generation = self.instance_generation;
        if read_definition {
            self.admit(memory, template, handle, database_index);
        }
        Ok(())
    }

    /// Exact native pointer membership in the current country's owned rows.
    pub(crate) fn instance_pointer_row(&self, pointer: usize) -> Option<usize> {
        if self.instance_generation == 0 {
            return None;
        }
        let entry =
            &self.entries[Self::pointer_slot(self.entries, pointer, self.instance_generation)?];
        (entry.source_generation == self.instance_generation).then_some(entry.source_row as usize)
    }

    fn read_definition(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        handle: u32,
        database_index: i32,
    ) -> Result<(), &'static str> {
        let slot = self.ensure_entry(database_index)?;
        let entry = &mut self.entries[slot];
        if entry.template_pointer != 0 {
            if entry.template_pointer != pointer {
                return Err("technology occurrence has inconsistent template pointers");
            }
            return Ok(());
        }
        // Failed definitions are attempted only once in this pass. Keep their
        // template identity installed so another country cannot retry them.
        entry.template_pointer = pointer;
        let mut header = [0; HEADER_BYTES];
        match copy_header(memory, pointer, &mut header) {
            Err(reason) => return Err(reason),
            Ok(3) => {}
            Ok(_) => return Err("technology template fields unreadable"),
        }
        let decoded = decode_header(pointer, &header, handle)?;
        // The instance slot is database_index - 1.
        if decoded.database_index != database_index {
            return Err("technology template index disagrees with country roster");
        }
        let source = decoded.source;
        let id_len = decoded.id_len;
        if read::<u8>(
            memory,
            source
                .checked_add(id_len)
                .ok_or("technology id address overflow")?,
        ) != Some(0)
        {
            return Err("technology id terminator invalid");
        }
        let end = self
            .bytes_len
            .checked_add(id_len)
            .ok_or("technology id byte count overflow")?;
        entry.row = Row {
            technology_handle: handle,
            database_index: decoded.database_index,
            year: decoded.year,
            max_level: decoded.max_level,
            research_cost_raw: decoded.research_cost_raw,
            id_start: self.bytes_len,
            id_len,
        };
        msvc_string::copy_growing(
            memory,
            source,
            id_len,
            self.names,
            self.bytes_len,
            "technology id bytes unreadable",
            "technology id allocation failed",
        )?;
        self.bytes_len = end;
        // An unreadable native length must not become deferred arena demand
        // when the collector later recycles this capture.
        self.required_bytes = end;
        self.copied += 1;
        entry.copied = true;
        Ok(())
    }
}
