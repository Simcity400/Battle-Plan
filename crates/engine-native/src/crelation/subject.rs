
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{Raw, RecordField, RecordLayout, RecordSources, packed_fields},
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native_parts::autonomy_state_definition;
use crate::{
    ccountry,
    crelation::{self, status as crelation_status},
    cwar_relation::{MembershipReasons, OwnedSnapshotScratch, scan_occurrence},
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_d388);
pub const TYPE_ID: usize = 0x08;
pub use engine_executable::native_values::subject_relation::TYPE_VALUE;
pub const OVERLORD: usize = 0x10;
pub const SUBJECT: usize = 0x14;
pub const START_HOUR: usize = 0x20;
pub const AUTONOMY_STATE: usize = 0x58;
pub const AUTONOMY_RATIO: usize = 0x60;

pub const CONSTRUCTOR_RVA: Rva = Rva(0x00d7_9880);

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
    pub required_bytes: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> NameWalk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> NameWalk {
    let mut pass = Pass {
        resolver: &mut crelation_status::Resolver::default(),
        ownership: None,
    };
    walk_from(memory, base, game_state, output, names, false, &mut pass)
}

pub fn walk_owned(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
    scratch: &mut OwnedSnapshotScratch,
) -> NameWalk {
    walk_owned_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
        scratch,
    )
}

pub fn walk_owned_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
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

#[cfg(test)]
fn walk_matrix(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    let mut pass = Pass {
        resolver: &mut crelation_status::Resolver::default(),
        ownership: None,
    };
    walk_from(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
        true,
        &mut pass,
    )
}

const SUBJECT_MEMBERSHIP: MembershipReasons = MembershipReasons {
    vector: "subject relation vector unreadable or invalid",
    pointer: "subject relation vector entry unreadable or outside array",
    null: "null subject ownership entry",
    mismatch: "subject ownership cross-link count mismatch",
    changed: "subject ownership snapshot changed or became unreadable",
};

struct Pass<'s> {
    resolver: &'s mut crelation_status::Resolver,
    ownership: Option<&'s mut OwnedSnapshotScratch>,
}

fn walk_from(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    matrix: bool,
    pass: &mut Pass<'_>,
) -> NameWalk {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        pass.resolver.begin(&reader.countries);
        if output.is_growing() {
            pass.resolver
                .grow()
                .map_err(|_| "subject resolver allocation failed")?;
        }
        let mut found_country = false;
        let (mut len, mut bytes_len, mut required_bytes) = (0_usize, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        let mut highest_slot = 0;
        let mut batch = OwnedBatch::new();
        if matrix {
            for slot in 1..reader.countries.count() {
                if reader.countries.at(slot)?.is_some() {
                    highest_slot = slot;
                }
            }
        }
        const COUNTRY_DIP_FIELDS: [RecordField; 1] =
            packed_fields([(ccountry::DIPLOMACY_STATUS, 8)]);
        const COUNTRY_DIP_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&COUNTRY_DIP_FIELDS, 8).unwrap();
        const DIP_OWNED_FIELDS: [RecordField; 3] = packed_fields([
            (0, 8),
            (cdiplomacy_status::COUNTRY, 8),
            (cdiplomacy_status::OWNED_RELATIONS, 16),
        ]);
        const DIP_OWNED_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DIP_OWNED_FIELDS, 32).unwrap();
        let expected_diplomacy =
            engine_sites::at(base, cdiplomacy_status::VTABLE_RVA).map(|v| v as u64);
        let mut country_slots = [ccountry::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        }; BATCH];
        let mut country_ptrs = [0_usize; BATCH];
        let mut dip_ptr_bytes = [0_u8; BATCH * 8];
        let mut dip_ptrs = [0_usize; BATCH];
        let mut dip_map = [u8::MAX; BATCH];
        let mut dip_records = [0_u8; BATCH * 32];
        let mut next = 1_u32;
        while next < reader.countries.count() {
            let end = reader.countries.count().min(next + BATCH as u32);
            let mut present = 0_usize;
            for slot in next..end {
                if let Some(country) = reader.countries.at(slot)? {
                    country_slots[present] = country;
                    country_ptrs[present] = country.pointer;
                    present += 1;
                }
            }
            next = end;
            if present == 0 {
                continue;
            }
            found_country = true;
            let (ptrs_copied, dips_copied) = if matrix {
                (0, 0)
            } else {
                let ptrs_copied = memory.copy_records(
                    RecordSources::Pointers(&country_ptrs[..present]),
                    &COUNTRY_DIP_LAYOUT,
                    &mut dip_ptr_bytes[..present * 8],
                );
                let mut live = 0_usize;
                dip_map[..present].fill(u8::MAX);
                for index in 0..ptrs_copied.min(present) {
                    let ptr = u64::from_le_bytes(
                        dip_ptr_bytes[index * 8..(index + 1) * 8]
                            .try_into()
                            .unwrap(),
                    );
                    if let Ok(ptr) = usize::try_from(ptr)
                        && ptr != 0
                    {
                        dip_map[index] = live as u8;
                        dip_ptrs[live] = ptr;
                        live += 1;
                    }
                }
                let dips_copied = if live == 0 {
                    0
                } else {
                    memory.copy_records(
                        RecordSources::Pointers(&dip_ptrs[..live]),
                        &DIP_OWNED_LAYOUT,
                        &mut dip_records[..live * 32],
                    )
                };
                (ptrs_copied, dips_copied)
            };
            for index in 0..present {
                let country = country_slots[index];
                let (diplomacy, source) = if !matrix && index < ptrs_copied {
                    let dip = usize::try_from(u64::from_le_bytes(
                        dip_ptr_bytes[index * 8..(index + 1) * 8]
                            .try_into()
                            .unwrap(),
                    ))
                    .map_err(|_| "subject pointer overflow")?;
                    if dip == 0 {
                        return Err("country diplomacy is null");
                    }
                    let slot = dip_map[index] as usize;
                    if dip_map[index] != u8::MAX && (slot + 1) * 3 <= dips_copied {
                        let rec = &dip_records[slot * 32..(slot + 1) * 32];
                        let expected = expected_diplomacy.ok_or("module address overflow")?;
                        if u64::from_le_bytes(rec[0..8].try_into().unwrap()) != expected {
                            return Err("diplomacy vtable mismatch");
                        }
                        if u64::from_le_bytes(rec[8..16].try_into().unwrap())
                            != country.pointer as u64
                        {
                            return Err("diplomacy country mismatch");
                        }
                        pass.resolver.cache_diplomacy(country, dip);
                        let owned = Array::from_header(rec[16..32].try_into().unwrap(), 8)
                            .ok_or("subject relation vector unreadable or invalid")?;
                        (dip, owned)
                    } else {
                        let diplomacy = pass
                            .resolver
                            .diplomacy(country, || reader.diplomacy(country))?;
                        let source = reader.array(diplomacy, engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS)?;
                        (diplomacy, source)
                    }
                } else {
                    let diplomacy = pass
                        .resolver
                        .diplomacy(country, || reader.diplomacy(country))?;
                    let source = reader.array(
                        diplomacy,
                        if matrix {
                            engine_native_parts::cdiplomacy_status_layout::RELATIONS
                        } else {
                            engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS
                        },
                    )?;
                    (diplomacy, source)
                };
            if matrix && source.count <= highest_slot {
                return Err("subject matrix does not cover country ordinals");
            }
            if !matrix && let Some(scratch) = pass.ownership.as_deref_mut() {
                scratch.observe_owned(source.count);
            }
            let mut matrix_pointers = [0; 64];
            for index in 0..source.count {
                let (pointer, head) = if matrix {
                    if index % 64 == 0 {
                        reader.matrix_pointers(source, index, diplomacy, &mut matrix_pointers)?;
                    }
                    let pointer = matrix_pointers[index as usize % 64];
                    if pointer == 0 {
                        continue;
                    }
                    (pointer, None)
                } else {
                    let at = index as usize % BATCH;
                    if at == 0 {
                        batch.read(&reader, source, index);
                    }
                    let entry = match batch.entry(at) {
                        Some(entry) => entry,
                        None => reader.element(source, index)?,
                    };
                    if entry == 0 {
                        return Err("null subject relation root entry");
                    }
                    let subject = match batch.subject(at) {
                        Some(subject) => subject,
                        None => reader.field::<u32>(entry, TYPE_ID)? == TYPE_VALUE,
                    };
                    if !subject {
                        continue;
                    }
                    (entry, batch.head(at))
                };
                let mut row =
                    match reader.relation(pointer, country, matrix.then_some(index), head, pass) {
                        Ok(Some(row)) => row,
                        Ok(None) => continue,
                        Err(reason) => return Err(reason),
                    };
                let state = match head {
                    Some(head) => {
                        usize::try_from(head.state).map_err(|_| "subject pointer overflow")?
                    }
                    None => reader.pointer(pointer, AUTONOMY_STATE)?,
                };
                let (source, name_len) = definition_name(memory, state)?;
                required_bytes = required_bytes
                    .checked_add(name_len)
                    .ok_or("subject name byte count overflow")?;
                output.ensure(len.checked_add(1).ok_or("subject row demand overflow")?)?;
                names.ensure(required_bytes)?;
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

const BATCH: usize = 64;
const HEAD: usize = AUTONOMY_RATIO + 8;

#[derive(Clone, Copy)]
struct Head {
    vtable: u64,
    type_id: u32,
    overlord: u32,
    subject: u32,
    start_hour: i32,
    end_hour: i32,
    cancel: u8,
    retired: u8,
    state: u64,
    autonomy_ratio: i64,
}

impl Head {
    fn new(bytes: &[u8]) -> Self {
        let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let half = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        Self {
            vtable: word(0),
            type_id: half(TYPE_ID),
            overlord: half(OVERLORD),
            subject: half(SUBJECT),
            start_hour: half(START_HOUR) as i32,
            end_hour: half(crelation::END_HOUR) as i32,
            cancel: bytes[crelation::CANCEL],
            retired: bytes[crelation::RETIRED],
            state: word(AUTONOMY_STATE),
            autonomy_ratio: word(AUTONOMY_RATIO) as i64,
        }
    }
}

struct OwnedBatch {
    entries: [usize; BATCH],
    read: usize,
    typed: usize,
    subjects: [bool; BATCH],
    head_of: [Option<u8>; BATCH],
    heads: [u8; BATCH * HEAD],
}

impl OwnedBatch {
    fn new() -> Self {
        Self {
            entries: [0; BATCH],
            read: 0,
            typed: 0,
            subjects: [false; BATCH],
            head_of: [None; BATCH],
            heads: [0; BATCH * HEAD],
        }
    }

    fn read<M: Memory>(&mut self, reader: &Reader<'_, M>, source: Array, first: u32) {
        use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
        const TYPE: [RecordField; 1] = [RecordField {
            source_offset: TYPE_ID,
            destination_offset: 0,
            len: 4,
        }];
        const TYPE_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&TYPE, 4).expect("valid record layout");
        const WHOLE: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: HEAD,
        }];
        const HEAD_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&WHOLE, HEAD).expect("valid record layout");
        let count = ((source.count - first) as usize).min(BATCH);
        (self.read, self.typed) = (0, 0);
        self.head_of = [None; BATCH];
        let mut raw = [0_u8; BATCH * 8];
        let read = source
            .element(first, 8)
            .is_some_and(|address| read_slice(reader.memory, address, &mut raw[..count * 8]));
        if !read {
            engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::Subjects);
            return;
        }
        for (entry, bytes) in self
            .entries
            .iter_mut()
            .zip(raw[..count * 8].chunks_exact(8))
        {
            *entry = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
        }
        self.read = count;
        let live = self.entries[..count]
            .iter()
            .position(|&entry| entry == 0)
            .unwrap_or(count);
        let mut types = [0_u8; BATCH * 4];
        self.typed = reader
            .memory
            .copy_records(
                RecordSources::Pointers(&self.entries[..live]),
                &TYPE_LAYOUT,
                &mut types[..live * 4],
            )
            .min(live);
        let mut subjects = [0; BATCH];
        let mut found = 0;
        for (at, bytes) in types[..self.typed * 4].chunks_exact(4).enumerate() {
            self.subjects[at] = u32::from_le_bytes(bytes.try_into().unwrap()) == TYPE_VALUE;
            if self.subjects[at] {
                subjects[found] = self.entries[at];
                self.head_of[at] = Some(found as u8);
                found += 1;
            }
        }
        let heads = reader
            .memory
            .copy_records(
                RecordSources::Pointers(&subjects[..found]),
                &HEAD_LAYOUT,
                &mut self.heads[..found * HEAD],
            )
            .min(found);
        for slot in &mut self.head_of {
            if slot.is_some_and(|place| place as usize >= heads) {
                *slot = None;
            }
        }
        if self.typed < live || heads < found {
            engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::Subjects);
        }
    }

    fn entry(&self, at: usize) -> Option<usize> {
        (at < self.read).then(|| self.entries[at])
    }

    fn subject(&self, at: usize) -> Option<bool> {
        (at < self.typed).then(|| self.subjects[at])
    }

    fn head(&self, at: usize) -> Option<Head> {
        let place = self.head_of[at]? as usize;
        Some(Head::new(&self.heads[place * HEAD..(place + 1) * HEAD]))
    }
}

pub fn unavailable(reason: &'static str) -> NameWalk {
    NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    }
}

pub fn definition_name(
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

pub struct Reader<'a, M: Memory> {
    pub(crate) memory: &'a M,
    pub(crate) base: usize,
    pub countries: ccountry::Reader<'a, M>,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, game_state: usize) -> Result<Self, &'static str> {
        Ok(Self {
            memory,
            base,
            countries: ccountry::Reader::new(memory, base, game_state)?,
        })
    }

    pub fn field<T: Raw>(&self, pointer: usize, offset: usize) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read::<T>(self.memory, address))
            .ok_or("subject field unreadable or address overflow")
    }

    pub fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?).map_err(|_| "subject pointer overflow")
    }

    pub fn vtable(
        &self,
        pointer: usize,
        rva: Rva,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, rva).ok_or("module address overflow")?;
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

    pub fn diplomacy(&self, country: engine_native_memory::country_identity::Country) -> Result<usize, &'static str> {
        let pointer = self.pointer(country.pointer, ccountry::DIPLOMACY_STATUS)?;
        if pointer == 0 {
            return Err("country diplomacy is null");
        }
        self.vtable(
            pointer,
            engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
            "diplomacy vtable mismatch",
        )?;
        if self.field::<u64>(pointer, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != country.pointer as u64 {
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
        let expected = engine_sites::at(self.base, crelation_status::VTABLE_RVA)
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

    fn installed(
        &self,
        pointer: usize,
        overlord: u32,
        subject: u32,
        pass: &mut Pass<'_>,
    ) -> Result<bool, &'static str> {
        let Some(first) = self.endpoint(overlord, pass)? else {
            return Ok(false);
        };
        let Some(second) = self.endpoint(subject, pass)? else {
            return Ok(false);
        };
        let diplomacy = pass.resolver.diplomacy(first, || self.diplomacy(first))?;
        let matrix = self.array(diplomacy, engine_native_parts::cdiplomacy_status_layout::RELATIONS)?;
        let status = self.element(matrix, second.canonical_slot)?;
        if status == 0 {
            return Ok(false);
        }
        self.status(status, diplomacy)?;
        Ok(self.pointer(status, crelation_status::SUBJECT_RELATION)? == pointer)
    }

    fn endpoint(
        &self,
        raw: u32,
        pass: &mut Pass<'_>,
    ) -> Result<Option<engine_native_memory::country_identity::Country>, &'static str> {
        if raw == 0 {
            return Ok(None);
        }
        let Some(slot) = self.countries.link(raw)? else {
            return Ok(None);
        };
        if self.countries.read_at(slot)?.is_none() {
            return Ok(None);
        }
        pass.resolver.resolve(&self.countries, raw).map(Some)
    }

    fn pair_status(
        &self,
        diplomacy: usize,
        target: engine_native_memory::country_identity::Country,
        relation: usize,
        pass: &mut Pass<'_>,
    ) -> Result<usize, &'static str> {
        let matrix = self.array(diplomacy, engine_native_parts::cdiplomacy_status_layout::RELATIONS)?;
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

    fn occurrence(
        &self,
        root: usize,
        offset: usize,
        relation: usize,
        expected: u32,
        pass: &mut Pass<'_>,
    ) -> Result<(), &'static str> {
        if offset == engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS
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
        owner: engine_native_memory::country_identity::Country,
        matrix_index: Option<u32>,
        head: Option<Head>,
        pass: &mut Pass<'_>,
    ) -> Result<Option<Row>, &'static str> {
        let (retired, overlord, subject) = match head {
            Some(head) => {
                let expected =
                    engine_sites::at(self.base, VTABLE_RVA).ok_or("module address overflow")?;
                if head.vtable != expected as u64 {
                    return Err("subject relation vtable mismatch");
                }
                if head.type_id != TYPE_VALUE {
                    return Err("subject matrix contains another relation type");
                }
                (head.retired != 0, head.overlord, head.subject)
            }
            None => {
                self.vtable(pointer, VTABLE_RVA, "subject relation vtable mismatch")?;
                if self.field::<u32>(pointer, TYPE_ID)? != TYPE_VALUE {
                    return Err("subject matrix contains another relation type");
                }
                (
                    self.field::<u8>(pointer, crelation::RETIRED)? != 0,
                    self.field::<u32>(pointer, OVERLORD)?,
                    self.field::<u32>(pointer, SUBJECT)?,
                )
            }
        };
        if retired && matrix_index.is_none() && !self.installed(pointer, overlord, subject, pass)? {
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
                    engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
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
        Ok(Some(match head {
            Some(head) => Row {
                overlord,
                subject,
                start_hour: head.start_hour,
                end_hour: head.end_hour,
                cancel: head.cancel,
                autonomy_ratio: head.autonomy_ratio,
                state: Name::default(),
            },
            None => Row {
                overlord,
                subject,
                start_hour: self.field(pointer, START_HOUR)?,
                end_hour: self.field(pointer, crelation::END_HOUR)?,
                cancel: self.field(pointer, crelation::CANCEL)?,
                autonomy_ratio: self.field(pointer, AUTONOMY_RATIO)?,
                state: Name::default(),
            },
        }))
    }
}

#[cfg(test)]
#[path = "csubject_relation_cost_baseline.rs"]
mod baseline;

#[cfg(test)]
mod tests;
