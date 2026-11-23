
pub const ID: usize = 0x10;
pub const DATABASE_INDEX: usize = 0x38;
pub const TECHNOLOGY_HANDLE: usize = 0x3c;
pub const YEAR: usize = 0x3d8;
pub const MAX_LEVEL: usize = 0x3dc;
pub const RESEARCH_COST: usize = 0x3e0;

use engine_native_memory::{Memory, msvc_string, read, walk::Outcome};

pub const HEADER_BYTES: usize = 56;
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub row: Row,
    pub template_pointer: usize,
    pub copied: bool,
    occupied: bool,
    admitted_handle: u32,
    instance_generation: u32,
    source_pointer: usize,
    source_row: u32,
    source_generation: u32,
}

#[derive(Clone, Copy)]
pub struct Admission {
    database_index: i32,
    template: usize,
    handle: u32,
}

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
    admissions: Option<&'a mut Vec<Admission>>,
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
            admissions: None,
        }
    }

    pub fn outcome(&self) -> Outcome {
        if let Some(reason) = self.error {
            Outcome::Unavailable { reason }
        } else {
            Outcome::Rows { len: self.copied }
        }
    }

    pub fn refuse(&mut self, reason: &'static str) {
        self.error.get_or_insert(reason);
    }

    pub fn instances(
        entries: &'a mut Vec<Entry>,
        names: &'a mut Vec<u8>,
        admissions: &'a mut Vec<Admission>,
    ) -> Self {
        admissions.clear();
        Self {
            admissions: Some(admissions),
            ..Self::new(entries, names)
        }
    }

    pub fn merge_attempts(
        &mut self,
        memory: &impl Memory,
        admissions: &[Admission],
    ) -> Result<(), &'static str> {
        for admission in admissions {
            let slot = self.ensure_entry(admission.database_index)?;
            let target = &self.entries[slot];
            if target.template_pointer != 0 {
                if target.copied && target.row.technology_handle != admission.handle {
                    self.error
                        .get_or_insert("technology template handle mismatch");
                }
                if target.template_pointer != admission.template {
                    self.error
                        .get_or_insert("technology occurrence has inconsistent template pointers");
                }
            } else {
                self.admit(
                    memory,
                    admission.template,
                    admission.handle,
                    admission.database_index,
                );
            }
        }
        Ok(())
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
            for old in self.entries.iter() {
                if old.occupied {
                    let slot = Self::index_slot(&grown, old.row.database_index).unwrap();
                    let entry = &mut grown[slot];
                    entry.row = old.row;
                    entry.template_pointer = old.template_pointer;
                    entry.copied = old.copied;
                    entry.occupied = true;
                    entry.instance_generation = old.instance_generation;
                    entry.admitted_handle = old.admitted_handle;
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
        let record = read_definition
            || entry.template_pointer != template
            || entry.admitted_handle != handle;
        if entry.copied && entry.row.technology_handle != handle {
            self.error
                .get_or_insert("technology template handle mismatch");
        }
        if !read_definition && entry.template_pointer != template {
            self.error
                .get_or_insert("technology occurrence has inconsistent template pointers");
        }
        if record && !read_definition {
            self.record_admission(database_index, template, handle)?;
        }
        if self.instance_pointer_row(pointer).is_some() {
            return Err("duplicate technology pointer in country roster");
        }
        if read_definition {
            if record {
                self.record_admission(database_index, template, handle)?;
            }
            self.entries[slot].admitted_handle = handle;
            if self.admissions.is_some() {
                self.entries[slot].template_pointer = template;
            }
        }
        let slot = Self::pointer_slot(self.entries, pointer, self.instance_generation)
            .expect("technology pointer index has space for admitted definitions");
        self.entries[slot].source_pointer = pointer;
        self.entries[slot].source_row = previous.len() as u32;
        self.entries[slot].source_generation = self.instance_generation;
        if read_definition && self.admissions.is_none() {
            self.admit(memory, template, handle, database_index);
        }
        Ok(())
    }

    fn record_admission(
        &mut self,
        database_index: i32,
        template: usize,
        handle: u32,
    ) -> Result<(), &'static str> {
        if let Some(admissions) = self.admissions.as_mut() {
            crate::allocation::reserve(admissions, 1)
                .map_err(|_| "technology admission allocation failed")?;
            admissions.push(Admission {
                database_index,
                template,
                handle,
            });
        }
        Ok(())
    }

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
        entry.template_pointer = pointer;
        let mut header = [0; HEADER_BYTES];
        match copy_header(memory, pointer, &mut header) {
            Err(reason) => return Err(reason),
            Ok(3) => {}
            Ok(_) => return Err("technology template fields unreadable"),
        }
        let decoded = decode_header(pointer, &header, handle)?;
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
        self.required_bytes = end;
        self.copied += 1;
        entry.copied = true;
        Ok(())
    }
}
