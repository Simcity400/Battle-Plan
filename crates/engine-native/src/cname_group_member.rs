
use engine_sites::Rva;
use engine_native_memory::{Memory, msvc_string, read, read_slice};

pub const VTABLE_RVA: Rva = Rva(0x029b_3c88);
pub const TYPE: usize = 0x08;
pub const FIRST_NAME_TEXT: usize = 0x10;
pub const TEMPLATE_REFERENCE: usize = 0x58;
pub const NAMING_EQUIPMENT: usize = 0x50;
pub const TEMPLATE_NAME_GROUP: usize = 0x1d0;
pub const GROUP_VTABLE_RVA: Rva = Rva(0x029b_3cd8);
pub const GROUP_KEY: usize = 8;
pub const ORDER: usize = 0x80;
pub const CACHED_NAME: usize = 0x60;
pub const OVERRIDE_NAME: usize = 0x88;
pub const OVERRIDE_NAME_LENGTH: usize = 0x98;
pub const FLAG_A: usize = 0xa8;
pub const FLAG_B: usize = 0xa9;
pub const CONSTRUCT_RVA: Rva = Rva(0x00a0_ac50);
const NAME_DATABASE_RVA: Rva = Rva(0x033d_d2d0);

pub fn order(memory: &impl Memory, base: usize, pointer: usize) -> Result<i32, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division name module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("division name vtable mismatch");
    }
    pointer
        .checked_add(ORDER)
        .and_then(|address| read::<i32>(memory, address))
        .ok_or("division name order unreadable")
}

pub fn custom(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(usize, usize, u8, u8), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division name module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("division name vtable mismatch");
    }
    let string = pointer
        .checked_add(OVERRIDE_NAME)
        .ok_or("division name address overflow")?;
    let len = read::<u64>(
        memory,
        string
            .checked_add(msvc_string::LENGTH)
            .ok_or("division name address overflow")?,
    )
    .ok_or("division name override length unreadable")?;
    let capacity = read::<u64>(
        memory,
        string
            .checked_add(msvc_string::CAPACITY)
            .ok_or("division name address overflow")?,
    )
    .ok_or("division name override capacity unreadable")?;
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("division name override header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "division name override length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        read::<u64>(memory, string)
            .and_then(|address| usize::try_from(address).ok())
            .filter(|address| *address != 0)
            .ok_or("division name override pointer unreadable or null")?
    };
    if len != 0
        && read::<u8>(
            memory,
            source
                .checked_add(len)
                .ok_or("division name address overflow")?,
        ) != Some(0)
    {
        return Err("division name override terminator unreadable or invalid");
    }
    let flag_a = read::<u8>(
        memory,
        pointer
            .checked_add(FLAG_A)
            .ok_or("division name address overflow")?,
    )
    .ok_or("division name flag unreadable")?;
    let flag_b = read::<u8>(
        memory,
        pointer
            .checked_add(FLAG_B)
            .ok_or("division name address overflow")?,
    )
    .ok_or("division name flag unreadable")?;
    Ok((source, len, flag_a, flag_b))
}

pub fn copy_custom(
    memory: &impl Memory,
    source: usize,
    len: usize,
    arena: &mut [u8],
    used: usize,
) -> Result<(usize, usize), &'static str> {
    if len > arena.len().saturating_sub(used) {
        return Err("division name override does not fit the caller arena");
    }
    if len != 0 && !read_slice(memory, source, &mut arena[used..used + len]) {
        return Err("division name override bytes unreadable");
    }
    Ok((used, len))
}

pub type Span = (usize, usize);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub programmatic: u8,
    pub custom: Span,
    pub group: Span,
    pub template: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Naming {
    Row(Inputs),
    DoesNotFit,
}

pub struct Names<'a> {
    pub bytes: engine_native_memory::walk::Buffer<'a, u8>,
    pub len: usize,
    pub required: usize,
}

pub const INPUT_SPAN: usize = FLAG_B + 1;


pub fn cached_name_growing(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Option<Span>, &'static str> {
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Growing(bytes),
        len: *len,
        required: *required,
    };
    let result = cached_name(memory, base, pointer, &mut names);
    *len = names.len;
    *required = names.required;
    result
}

pub fn cached_name(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    names: &mut Names<'_>,
) -> Result<Option<Span>, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division cached name module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("division cached name member identity unreadable or invalid");
    }
    let address = pointer
        .checked_add(CACHED_NAME)
        .ok_or("division cached name address overflow")?;
    let header = read::<[u8; msvc_string::STRIDE]>(memory, address)
        .ok_or("division cached name header unreadable")?;
    copy_string(memory, &header, names, &CACHED)
}

pub fn cached_name_demand(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<usize, &'static str> {
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Count,
        len: 0,
        required: 0,
    };
    cached_name(memory, base, pointer, &mut names)?;
    Ok(names.required)
}

#[cfg(test)]
#[path = "cname_group_member/cached_name_tests.rs"]
mod cached_name_tests;

#[derive(Default)]
pub struct GroupKeys {
    templates: Vec<Entry<u64>>,
    keys: Vec<Entry<usize>>,
    generation: u32,
    registry: engine_native_parts::creference_registry::HeaderResolver,
}

#[derive(Clone, Copy, Default)]
struct Entry<K> {
    key: K,
    generation: u32,
    span: Span,
}

const UNFIT: Span = (usize::MAX, 0);
const NULL_GROUP: Span = (usize::MAX - 1, 0);

const PROBES: usize = 16;

fn slot(hash: u64, len: usize) -> usize {
    let hash = hash.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    ((hash ^ (hash >> 32)) as usize) & (len - 1)
}

impl GroupKeys {
    pub fn try_with_capacity(templates: usize, groups: usize) -> std::io::Result<Self> {
        let size = |n: usize| {
            if n == 0 {
                Ok(0)
            } else {
                n.checked_next_power_of_two()
                    .ok_or_else(engine_allocation::oom)
            }
        };
        Ok(Self {
            templates: engine_allocation::filled(size(templates)?, Entry::default())?,
            keys: engine_allocation::filled(size(groups)?, Entry::default())?,
            generation: 1,
            registry: Default::default(),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_capacity(templates: usize, groups: usize) -> Self {
        Self::try_with_capacity(templates, groups).unwrap()
    }

    pub fn capacities(&self) -> [usize; 2] {
        [self.templates.len(), self.keys.len()]
    }

    pub fn allocated_bytes(&self) -> usize {
        self.templates.capacity() * size_of::<Entry<u64>>()
            + self.keys.capacity() * size_of::<Entry<usize>>()
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.templates.fill(Entry::default());
            self.keys.fill(Entry::default());
            self.generation = 1;
        }
        self.registry = Default::default();
    }

    fn find<K: Copy + PartialEq>(
        table: &[Entry<K>],
        generation: u32,
        key: K,
        hash: u64,
    ) -> Result<Span, Option<usize>> {
        if table.is_empty() {
            return Err(None);
        }
        let first = slot(hash, table.len());
        for probe in 0..PROBES.min(table.len()) {
            let index = (first + probe) & (table.len() - 1);
            let entry = table[index];
            if entry.generation != generation {
                return Err(Some(index));
            }
            if entry.key == key {
                return Ok(entry.span);
            }
        }
        Err(None)
    }

    fn key(
        &mut self,
        memory: &impl Memory,
        base: usize,
        group: usize,
        names: &mut Names<'_>,
    ) -> Result<Span, &'static str> {
        if group == 0 {
            return Ok(NULL_GROUP);
        }
        let free = match Self::find(&self.keys, self.generation, group, group as u64) {
            Ok(span) => return Ok(span),
            Err(free) => free,
        };
        let header = read::<[u8; 40]>(memory, group).ok_or("division naming group unreadable")?;
        if Some(u64::from_le_bytes(header[..8].try_into().unwrap()))
            != engine_sites::at(base, GROUP_VTABLE_RVA).map(|p| p as u64)
        {
            return Err("division naming group identity mismatch");
        }
        let span =
            match copy_string(memory, &header[GROUP_KEY..GROUP_KEY + 32], names, &KEY)? {
                Some(span) => {
                    names.len += span.1;
                    span
                }
                None => UNFIT,
            };
        if let Some(index) = free {
            self.keys[index] = Entry {
                key: group,
                generation: self.generation,
                span,
            };
        }
        Ok(span)
    }
}

pub type StringErrors = [&'static str; 4];
const CUSTOM: StringErrors = [
    "division name override header invalid",
    "division name override pointer unreadable or null",
    "division name override bytes unreadable",
    "division name override terminator unreadable or invalid",
];
const KEY: StringErrors = [
    "division naming group key header invalid",
    "division naming group key pointer unreadable or null",
    "division naming group key unreadable",
    "division naming group key terminator unreadable or invalid",
];
const CACHED: StringErrors = [
    "division cached name header invalid",
    "division cached name pointer unreadable or null",
    "division cached name bytes unreadable",
    "division cached name terminator unreadable or invalid",
];


pub fn copy_string(
    memory: &impl Memory,
    header: &[u8],
    names: &mut Names<'_>,
    errors: &StringErrors,
) -> Result<Option<Span>, &'static str> {
    let start = names.len;
    let free = names.bytes.len().saturating_sub(start);
    let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let (raw_len, raw_cap) = (word(msvc_string::LENGTH), word(msvc_string::CAPACITY));
    let can_copy_inline = match &names.bytes {
        crate::walk::Buffer::Count => false,
        crate::walk::Buffer::Exact(_) => raw_len <= free as u64,
        crate::walk::Buffer::Growing(_) | crate::walk::Buffer::GrowingFrom(_, _) => true,
    };
    if can_copy_inline
        && raw_cap > msvc_string::INLINE_CAPACITY
        && raw_len != 0
        && raw_len <= raw_cap
        && raw_len <= 255
    {
        if let Ok(len) = usize::try_from(raw_len)
            && let Some(source) = usize::try_from(word(0)).ok().filter(|&a| a != 0)
            && source.checked_add(len).is_some()
        {
            let mut buf = [0_u8; 256];
            if read_slice(memory, source, &mut buf[..len + 1]) && buf[len] == 0 {
                names.required = demand(names.required, len)?;
                if !names.bytes.ensure(
                    start
                        .checked_add(len)
                        .ok_or("division name arena overflow")?,
                )? {
                    return Ok(None);
                }
                names.bytes[start..start + len].copy_from_slice(&buf[..len]);
                return Ok(Some((start, len)));
            }
        }
    }
    let (source, len) = match string_source(memory, header, errors)? {
        StringSource::Empty => return Ok(Some((start, 0))),
        StringSource::Inline(text) => {
            let len = text.len();
            names.required = demand(names.required, len)?;
            if !names.bytes.ensure(
                start
                    .checked_add(len)
                    .ok_or("division name arena overflow")?,
            )? {
                return Ok(None);
            }
            names.bytes[start..start + len].copy_from_slice(text);
            return Ok(Some((start, len)));
        }
        StringSource::Heap { source, len } => (source, len),
    };
    names.required = demand(names.required, len)?;
    if len > free {
        match &mut names.bytes {
            engine_native_memory::walk::Buffer::Count | engine_native_memory::walk::Buffer::Exact(_) => return Ok(None),
            engine_native_memory::walk::Buffer::GrowingFrom(bytes, offset) => {
                msvc_string::copy_growing(
                    memory,
                    source,
                    len,
                    bytes,
                    *offset + start,
                    errors[2],
                    "division name arena allocation failed",
                )?;
                return Ok(Some((start, len)));
            }
            engine_native_memory::walk::Buffer::Growing(bytes) => {
                msvc_string::copy_growing(
                    memory,
                    source,
                    len,
                    bytes,
                    start,
                    errors[2],
                    "division name arena allocation failed",
                )?;
                return Ok(Some((start, len)));
            }
        }
    }
    if !read_slice(memory, source, &mut names.bytes[start..start + len]) {
        return Err(errors[2]);
    }
    Ok(Some((start, len)))
}

enum StringSource<'h> {
    Empty,
    Inline(&'h [u8]),
    Heap {
        source: usize,
        len: usize,
    },
}

fn string_source<'h>(
    memory: &impl Memory,
    header: &'h [u8],
    errors: &StringErrors,
) -> Result<StringSource<'h>, &'static str> {
    let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let (len, capacity) = (word(msvc_string::LENGTH), word(msvc_string::CAPACITY));
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err(errors[0]);
    }
    let len = usize::try_from(len).map_err(|_| errors[0])?;
    if capacity == msvc_string::INLINE_CAPACITY {
        if len != 0 && header[len] != 0 {
            return Err(errors[3]);
        }
        return Ok(StringSource::Inline(&header[..len]));
    }
    let source = usize::try_from(word(0))
        .ok()
        .filter(|address| *address != 0)
        .ok_or(errors[1])?;
    if len == 0 {
        return Ok(StringSource::Empty);
    }
    let last = source.checked_add(len - 1).ok_or(errors[3])?;
    source.checked_add(len).ok_or(errors[3])?;
    let (mut first, mut tail) = ([0_u8; 1], [0_u8; 2]);
    match memory.copy_scattered_prefetched([source, last], &mut [&mut first, &mut tail]) {
        0 => return Err(errors[2]),
        1 => return Err(errors[3]),
        _ => {}
    }
    if tail[1] != 0 {
        return Err(errors[3]);
    }
    Ok(StringSource::Heap { source, len })
}

fn demand(required: usize, len: usize) -> Result<usize, &'static str> {
    required
        .checked_add(len)
        .ok_or("division name demand overflow")
}

fn copy_string_into(
    memory: &impl Memory,
    header: &[u8],
    bytes: &mut Vec<u8>,
    at: usize,
    required: &mut usize,
    errors: &StringErrors,
) -> Result<usize, &'static str> {
    let word = |offset: usize| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
    let (raw_len, raw_cap) = (word(msvc_string::LENGTH), word(msvc_string::CAPACITY));
    if raw_cap > msvc_string::INLINE_CAPACITY && raw_len != 0 && raw_len <= raw_cap && raw_len <= 255
    {
        if let Ok(len) = usize::try_from(raw_len)
            && let Some(source) = usize::try_from(word(0)).ok().filter(|&a| a != 0)
            && source.checked_add(len).is_some()
        {
            let mut buf = [0_u8; 256];
            if read_slice(memory, source, &mut buf[..len + 1]) && buf[len] == 0 {
                *required = demand(*required, len)?;
                let end = at.checked_add(len).ok_or("division name arena overflow")?;
                crate::walk::grow(bytes, end)?;
                bytes[at..end].copy_from_slice(&buf[..len]);
                return Ok(len);
            }
        }
    }
    let (source, len) = match string_source(memory, header, errors)? {
        StringSource::Empty => return Ok(0),
        StringSource::Inline(text) => {
            *required = demand(*required, text.len())?;
            let end = at
                .checked_add(text.len())
                .ok_or("division name arena overflow")?;
            engine_native_memory::walk::grow(bytes, end)?;
            bytes[at..end].copy_from_slice(text);
            return Ok(text.len());
        }
        StringSource::Heap { source, len } => (source, len),
    };
    *required = demand(*required, len)?;
    if len > bytes.len().saturating_sub(at) {
        msvc_string::copy_growing(
            memory,
            source,
            len,
            bytes,
            at,
            errors[2],
            "division name arena allocation failed",
        )?;
    } else if !read_slice(memory, source, &mut bytes[at..at + len]) {
        return Err(errors[2]);
    }
    Ok(len)
}

pub fn inputs(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    groups: &mut GroupKeys,
    names: &mut Names<'_>,
) -> Result<Naming, &'static str> {
    read_inputs(memory, base, pointer, groups, names, false)
}

pub fn member(memory: &impl Memory, pointer: usize) -> Result<[u8; INPUT_SPAN], &'static str> {
    read::<[u8; INPUT_SPAN]>(memory, pointer).ok_or("division name unreadable")
}

pub struct MemberBatch {
    bytes: [u8; 64 * INPUT_SPAN],
    rows: [usize; 64],
    complete: [bool; 64],
}

impl Default for MemberBatch {
    fn default() -> Self {
        Self {
            bytes: [0; 64 * INPUT_SPAN],
            rows: [usize::MAX; 64],
            complete: [false; 64],
        }
    }
}

impl MemberBatch {
    pub fn capture(&mut self, memory: &impl Memory, selected: &[usize]) {
        const FIELDS: [engine_native_memory::memory::RecordField; 1] = [engine_native_memory::memory::RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: INPUT_SPAN,
        }];
        const LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
            engine_native_memory::memory::RecordLayout::new(&FIELDS, INPUT_SPAN).expect("valid member layout");
        self.rows.fill(usize::MAX);
        let mut pointers = [0_usize; 64];
        let mut len = 0;
        for (index, &pointer) in selected.iter().enumerate() {
            if pointer != 0 {
                pointers[len] = pointer;
                self.rows[index] = len;
                len += 1;
            }
        }
        engine_native_memory::memory::gather_records_once(
            memory,
            &pointers[..len],
            &LAYOUT,
            &mut self.bytes[..len * INPUT_SPAN],
            &mut self.complete,
            engine_native_memory::batch_fallback::Batch::DivisionNaming,
        );
    }

    pub fn member(
        &self,
        memory: &impl Memory,
        index: usize,
        pointer: usize,
    ) -> Result<[u8; INPUT_SPAN], &'static str> {
        let row = self.rows[index];
        if row != usize::MAX && self.complete[row] {
            return Ok(self.bytes[row * INPUT_SPAN..][..INPUT_SPAN]
                .try_into()
                .unwrap());
        }
        member(memory, pointer)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn inputs_growing(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    member: &[u8; INPUT_SPAN],
    groups: &mut GroupKeys,
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Naming, &'static str> {
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Growing(bytes),
        len: *len,
        required: *required,
    };
    let result = member_inputs(memory, base, pointer, member, groups, &mut names, false);
    *len = names.len;
    *required = names.required;
    result
}

pub fn held_cached_name(
    text: &[u8],
    bytes: &mut Vec<u8>,
    len: usize,
    required: &mut usize,
) -> Result<Option<Span>, &'static str> {
    *required = required
        .checked_add(text.len())
        .ok_or("division name demand overflow")?;
    let start = len;
    let end = start
        .checked_add(text.len())
        .ok_or("division name arena overflow")?;
    let mut arena = engine_native_memory::walk::Buffer::Growing(bytes);
    arena.ensure(end)?;
    arena[start..end].copy_from_slice(text);
    Ok(Some((start, text.len())))
}

pub fn cached_name_from_member(
    memory: &impl Memory,
    base: usize,
    member: &[u8; INPUT_SPAN],
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Option<Span>, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division cached name module address overflow")?;
    if u64::from_le_bytes(member[..8].try_into().unwrap()) != expected as u64 {
        return Err("division cached name member identity unreadable or invalid");
    }
    let header = &member[CACHED_NAME..CACHED_NAME + msvc_string::STRIDE];
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Growing(bytes),
        len: *len,
        required: *required,
    };
    let result = copy_fitting_string(memory, header, &mut names, &CACHED);
    *len = names.len;
    *required = names.required;
    result
}

pub fn raw_override_from_member(
    memory: &impl Memory,
    member: &[u8; INPUT_SPAN],
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Option<Span>, &'static str> {
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Growing(bytes),
        len: *len,
        required: *required,
    };
    let result = copy_fitting_string(
        memory,
        &member[OVERRIDE_NAME..OVERRIDE_NAME + msvc_string::STRIDE],
        &mut names,
        &CUSTOM,
    );
    *len = names.len;
    *required = names.required;
    result
}

fn copy_fitting_string(
    memory: &impl Memory,
    header: &[u8],
    names: &mut Names<'_>,
    errors: &StringErrors,
) -> Result<Option<Span>, &'static str> {
    let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let (len, capacity) = (word(msvc_string::LENGTH), word(msvc_string::CAPACITY));
    let start = names.len;
    if let (Ok(len), Ok(source)) = (usize::try_from(len), usize::try_from(word(0)))
        && capacity > msvc_string::INLINE_CAPACITY
        && len as u64 <= capacity
        && len != 0
        && len < names.bytes.len().saturating_sub(start)
        && read_slice(memory, source, &mut names.bytes[start..=start + len])
    {
        if names.bytes[start + len] != 0 {
            return Err(errors[3]);
        }
        names.required = names
            .required
            .checked_add(len)
            .ok_or("division name demand overflow")?;
        return Ok(Some((start, len)));
    }
    copy_string(memory, header, names, errors)
}

pub(crate) fn codename_inputs_into(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    bytes: &mut Vec<u8>,
    from: usize,
    len: &mut usize,
    required: &mut usize,
) -> Result<Inputs, &'static str> {
    engine_sites::at(base, VTABLE_RVA).ok_or("division name module address overflow")?;
    let member = member(memory, pointer)?;
    let mut row = member_row(base, &member)?;
    if row.kind != 2 {
        return Err("operative codename member is not type two");
    }
    let mut copy =
        |header: &[u8], len: &mut usize, errors: &StringErrors| -> Result<Span, &'static str> {
            let at = from
                .checked_add(*len)
                .ok_or("division name arena overflow")?;
            let copied = copy_string_into(memory, header, bytes, at, required, errors)?;
            let span = (*len, copied);
            *len += copied;
            Ok(span)
        };
    row.group = copy(&member[FIRST_NAME_TEXT..FIRST_NAME_TEXT + 32], len, &KEY)?;
    if shows_custom(memory, base, pointer, &member, &row)? {
        row.custom = copy(&member[OVERRIDE_NAME..OVERRIDE_NAME + 32], len, &CUSTOM)?;
    }
    Ok(row)
}

pub(crate) fn codename_inputs_buffer(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    buffer: engine_native_memory::walk::Buffer<'_, u8>,
    len: &mut usize,
    required: &mut usize,
    heap: &mut [bool; 2],
) -> Result<Naming, &'static str> {
    engine_sites::at(base, VTABLE_RVA)
        .ok_or("division name module address overflow")?;
    let member = member(memory, pointer)?;
    *heap = [FIRST_NAME_TEXT, OVERRIDE_NAME].map(|at| {
        let at = at + msvc_string::CAPACITY;
        u64::from_le_bytes(member[at..at + 8].try_into().unwrap()) != msvc_string::INLINE_CAPACITY
    });
    let mut arena = Names {
        bytes: buffer,
        len: *len,
        required: *required,
    };
    let result = member_inputs(
        memory,
        base,
        pointer,
        &member,
        &mut GroupKeys::default(),
        &mut arena,
        true,
    );
    *len = arena.len;
    *required = arena.required;
    result
}

pub fn custom_copied(row: &Inputs) -> bool {
    row.programmatic == 0 || row.custom.1 != 0
}

pub fn carry_codename(
    names: &mut Names<'_>,
    held: Inputs,
    group: &[u8],
    custom: Option<&[u8]>,
    heap: [bool; 2],
) -> Result<Naming, &'static str> {
    let mut row = Inputs {
        custom: Span::default(),
        group: Span::default(),
        ..held
    };
    let mut fits = true;
    match place_string(names, group, heap[0], &KEY)? {
        Some(span) => {
            row.group = span;
            names.len += span.1;
        }
        None => fits = false,
    }
    if let Some(custom) = custom {
        match place_string(names, custom, heap[1], &CUSTOM)? {
            Some(span) => row.custom = span,
            None => return Ok(Naming::DoesNotFit),
        }
    }
    Ok(if fits {
        Naming::Row(row)
    } else {
        Naming::DoesNotFit
    })
}

fn place_string(
    names: &mut Names<'_>,
    bytes: &[u8],
    heap: bool,
    errors: &StringErrors,
) -> Result<Option<Span>, &'static str> {
    let len = bytes.len();
    let start = names.len;
    let free = names.bytes.len().saturating_sub(start);
    if !heap {
        names.required = names
            .required
            .checked_add(len)
            .ok_or("division name demand overflow")?;
        if !names.bytes.ensure(
            start
                .checked_add(len)
                .ok_or("division name arena overflow")?,
        )? {
            return Ok(None);
        }
        names.bytes[start..start + len].copy_from_slice(bytes);
        return Ok(Some((start, len)));
    }
    if len == 0 {
        return Ok(Some((start, 0)));
    }
    names.required = names
        .required
        .checked_add(len)
        .ok_or("division name demand overflow")?;
    if len > free {
        let (output, at) = match &mut names.bytes {
            engine_native_memory::walk::Buffer::Count | engine_native_memory::walk::Buffer::Exact(_) => return Ok(None),
            engine_native_memory::walk::Buffer::GrowingFrom(output, offset) => (output, *offset + start),
            engine_native_memory::walk::Buffer::Growing(output) => (output, start),
        };
        msvc_string::place(
            bytes,
            output,
            at,
            errors[2],
            "division name arena allocation failed",
        )?;
        return Ok(Some((start, len)));
    }
    names.bytes[start..start + len].copy_from_slice(bytes);
    Ok(Some((start, len)))
}

pub fn read_inputs(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    groups: &mut GroupKeys,
    names: &mut Names<'_>,
    codename: bool,
) -> Result<Naming, &'static str> {
    engine_sites::at(base, VTABLE_RVA)
        .ok_or("division name module address overflow")?;
    let member = member(memory, pointer)?;
    member_inputs(memory, base, pointer, &member, groups, names, codename)
}

fn member_inputs(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    member: &[u8; INPUT_SPAN],
    groups: &mut GroupKeys,
    names: &mut Names<'_>,
    codename: bool,
) -> Result<Naming, &'static str> {
    let word = |at: usize| u64::from_le_bytes(member[at..at + 8].try_into().unwrap());
    let mut row = member_row(base, member)?;
    if codename && row.kind != 2 {
        return Err("operative codename member is not type two");
    }
    let mut fits = true;
    if row.kind == 2 {
        match copy_string(
            memory,
            &member[FIRST_NAME_TEXT..FIRST_NAME_TEXT + 32],
            names,
            &KEY,
        )? {
            Some(span) => {
                row.group = span;
                names.len += span.1;
            }
            None => fits = false,
        }
    }
    let header = &member[OVERRIDE_NAME..OVERRIDE_NAME + 32];
    if shows_custom(memory, base, pointer, member, &row)? {
        match copy_string(memory, header, names, &CUSTOM)? {
            Some(span) => row.custom = span,
            None => return Ok(Naming::DoesNotFit),
        }
    }
    if row.kind != 0 || row.custom.1 != 0 {
        return Ok(if fits {
            Naming::Row(row)
        } else {
            Naming::DoesNotFit
        });
    }
    match template_group(memory, base, word(TEMPLATE_REFERENCE), groups, names)? {
        UNFIT => Ok(Naming::DoesNotFit),
        NULL_GROUP if word(OVERRIDE_NAME + msvc_string::LENGTH) != 0 => {
            match copy_string(memory, header, names, &CUSTOM)? {
                Some(span) => row.custom = span,
                None => return Ok(Naming::DoesNotFit),
            }
            Ok(Naming::Row(row))
        }
        NULL_GROUP => Ok(Naming::Row(row)),
        span => {
            row.group = span;
            Ok(Naming::Row(row))
        }
    }
}

fn member_row(base: usize, member: &[u8; INPUT_SPAN]) -> Result<Inputs, &'static str> {
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("division name module address overflow")?;
    let word = |at: usize| u64::from_le_bytes(member[at..at + 8].try_into().unwrap());
    if word(0) != expected as u64 {
        return Err("division name vtable mismatch");
    }
    let signed = |at: usize| i32::from_le_bytes(member[at..at + 4].try_into().unwrap());
    Ok(Inputs {
        kind: signed(TYPE),
        order: signed(ORDER),
        ordered: member[FLAG_A],
        programmatic: member[FLAG_B],
        template: word(TEMPLATE_REFERENCE),
        ..Inputs::default()
    })
}

fn shows_custom(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    member: &[u8; INPUT_SPAN],
    row: &Inputs,
) -> Result<bool, &'static str> {
    let length = u64::from_le_bytes(
        member[OVERRIDE_NAME_LENGTH..OVERRIDE_NAME_LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    Ok(row.programmatic == 0
        || (row.kind == 2
            && length != 0
            && operative_group_is_null(memory, base, pointer, member)?))
}

pub fn operative_group_is_null(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    member: &[u8; INPUT_SPAN],
) -> Result<bool, &'static str> {
    let at = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("operative name group address overflow")
    };
    let pointer_at = |address| {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("operative name group pointer unreadable")
    };
    let database = pointer_at(
        engine_sites::at(base, NAME_DATABASE_RVA).ok_or("operative name group address overflow")?,
    )?;
    if database == 0 {
        return Err("operative name database null");
    }
    let buckets = read::<u32>(memory, at(database, 0x4c)?)
        .filter(|&count| count != 0)
        .ok_or("operative name group bucket count unreadable or zero")?;
    let table = pointer_at(at(database, 0x50)?)?;
    if table == 0 {
        return Err("operative name group bucket array null");
    }
    let hash = u32::from_le_bytes(member[0x30..0x34].try_into().unwrap());
    let mut node = pointer_at(at(table, (hash % buckets) as usize * 8)?)?;
    while node != 0 {
        let group = pointer_at(node)?;
        let header =
            read::<[u8; 44]>(memory, group).ok_or("operative name group identity unreadable")?;
        if Some(u64::from_le_bytes(header[..8].try_into().unwrap()))
            != engine_sites::at(base, GROUP_VTABLE_RVA).map(|value| value as u64)
        {
            return Err("operative name group identity mismatch");
        }
        if u32::from_le_bytes(header[0x28..0x2c].try_into().unwrap()) == hash
            && group_keys_equal(
                memory,
                at(group, GROUP_KEY)?,
                &header[GROUP_KEY..GROUP_KEY + 32],
                at(pointer, FIRST_NAME_TEXT)?,
                &member[FIRST_NAME_TEXT..FIRST_NAME_TEXT + 32],
            )?
        {
            return Ok(false);
        }
        node = pointer_at(at(node, 8)?)?;
    }
    Ok(true)
}

fn group_keys_equal(
    memory: &impl Memory,
    left: usize,
    left_header: &[u8],
    right: usize,
    right_header: &[u8],
) -> Result<bool, &'static str> {
    let source = |pointer, header: &[u8]| {
        let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
        let (len, capacity) = (word(16), word(24));
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("operative name group key header invalid");
        }
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            pointer
        } else {
            usize::try_from(word(0)).map_err(|_| "operative name group key pointer invalid")?
        };
        if source == 0 {
            return Err("operative name group key pointer null");
        }
        Ok(source)
    };
    let (left, right) = (source(left, left_header)?, source(right, right_header)?);
    let mut index = 0_usize;
    loop {
        let byte = |source: usize| {
            source
                .checked_add(index)
                .and_then(|at| read::<u8>(memory, at))
                .ok_or("operative name group key bytes unreadable")
        };
        let (left, right) = (byte(left)?, byte(right)?);
        if left != right {
            if !left.is_ascii() || !right.is_ascii() {
                return Err("operative name group comparison locale unavailable");
            }
            if !left.eq_ignore_ascii_case(&right) {
                return Ok(false);
            }
        }
        if left == 0 {
            return Ok(true);
        }
        index = index
            .checked_add(1)
            .ok_or("operative name group key extent overflow")?;
    }
}

fn template_group(
    memory: &impl Memory,
    base: usize,
    template: u64,
    groups: &mut GroupKeys,
    names: &mut Names<'_>,
) -> Result<Span, &'static str> {
    if template == 0 {
        return Ok(NULL_GROUP);
    }
    let free = match GroupKeys::find(&groups.templates, groups.generation, template, template) {
        Ok(span) => return Ok(span),
        Err(free) => free,
    };
    let Some(object) = groups.registry.resolve(memory, base, template)? else {
        return Ok(NULL_GROUP);
    };
    use crate::creferenced_division_template as reference;
    let (mut identity, mut group) = ([0_u8; 16], [0_u8; 8]);
    let group_address = object
        .checked_add(TEMPLATE_NAME_GROUP)
        .ok_or("division naming group pointer unreadable")?;
    let copied =
        memory.copy_scattered_prefetched([object, group_address], &mut [&mut identity, &mut group]);
    if copied == 0 {
        return Err("division template identity unreadable");
    }
    let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
        != engine_sites::at(base, reference::VTABLE_RVA).map(|p| p as u64)
        || handle as u32 != reference::REFERENCE_TYPE
    {
        return Err("division template vtable or reference type mismatch");
    }
    if handle != template {
        return Err("division naming template identity mismatch");
    }
    if copied != 2 {
        return Err("division naming group pointer unreadable");
    }
    let group = usize::try_from(u64::from_le_bytes(group))
        .map_err(|_| "division naming group pointer unreadable")?;
    let span = groups.key(memory, base, group, names)?;
    if let Some(index) = free {
        groups.templates[index] = Entry {
            key: template,
            generation: groups.generation,
            span,
        };
    }
    Ok(span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    struct Guard {
        bytes: Vec<u8>,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
        database: Option<usize>,
    }
    impl Memory for Guard {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if address == NAME_DATABASE_RVA.0 && output.len() == 8 {
                if let Some(database) = self.database {
                    output.copy_from_slice(&(database as u64).to_le_bytes());
                    return true;
                }
                return false;
            }
            SliceMemory {
                base: 0x1000,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn guard(bytes: &[u8]) -> Guard {
        Guard {
            bytes: bytes.to_vec(),
            reads: Default::default(),
            database: None,
        }
    }
    fn run(memory: &impl Memory, arena: usize) -> (Result<Naming, &'static str>, Vec<u8>, usize) {
        let mut bytes = vec![0; arena];
        let mut names = Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut bytes),
            len: 0,
            required: 0,
        };
        let result = inputs(
            memory,
            0,
            0x1000,
            &mut GroupKeys::with_capacity(4, 4),
            &mut names,
        );
        let required = names.required;
        (result, bytes, required)
    }
    fn count(memory: &impl Memory) -> (Result<Naming, &'static str>, usize) {
        let mut names = Names {
            bytes: engine_native_memory::walk::Buffer::Count,
            len: 0,
            required: 0,
        };
        let result = inputs(
            memory,
            0,
            0x1000,
            &mut GroupKeys::with_capacity(4, 4),
            &mut names,
        );
        (result, names.required)
    }
    fn row(result: Result<Naming, &'static str>) -> Inputs {
        match result {
            Ok(Naming::Row(row)) => row,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn operative_group_and_override_are_independent_operands() {
        let mut bytes = [0_u8; INPUT_SPAN];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[TYPE..TYPE + 4].copy_from_slice(&2_i32.to_le_bytes());
        bytes[FIRST_NAME_TEXT..FIRST_NAME_TEXT + 5].copy_from_slice(b"GROUP");
        bytes[0x20..0x28].copy_from_slice(&5_u64.to_le_bytes());
        bytes[0x28..0x30].copy_from_slice(&15_u64.to_le_bytes());
        bytes[OVERRIDE_NAME..OVERRIDE_NAME + 4].copy_from_slice(b"user");
        bytes[0x98..0xa0].copy_from_slice(&4_u64.to_le_bytes());
        bytes[0xa0..0xa8].copy_from_slice(&15_u64.to_le_bytes());
        let (result, arena, required) = run(&guard(&bytes), 9);
        let named = row(result);
        assert_eq!(
            &arena[named.group.0..named.group.0 + named.group.1],
            b"GROUP"
        );
        assert_eq!(
            &arena[named.custom.0..named.custom.0 + named.custom.1],
            b"user"
        );
        assert_eq!(required, 9);
        assert_eq!(run(&guard(&bytes), 0).2, 9);
        assert_eq!(count(&guard(&bytes)), (Ok(Naming::DoesNotFit), 9));
        bytes[FLAG_B] = 1;
        bytes[OVERRIDE_NAME_LENGTH..OVERRIDE_NAME_LENGTH + 8].fill(0);
        let (result, _, required) = run(&guard(&bytes), 5);
        assert_eq!(row(result).custom.1, 0);
        assert_eq!(required, 5);
        bytes[FIRST_NAME_TEXT + 5] = b'x';
        assert_eq!(run(&guard(&bytes), 9).0, Err(KEY[3]));
    }

    #[test]
    fn operative_programmatic_override_follows_the_native_group_lookup() {
        let mut memory = guard(&[0; 0x500]);
        memory.database = Some(0x1100);
        let bytes = &mut memory.bytes;
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        put(bytes, 0, VTABLE_RVA.0 as u64);
        bytes[TYPE..TYPE + 4].copy_from_slice(&2_i32.to_le_bytes());
        bytes[FIRST_NAME_TEXT..FIRST_NAME_TEXT + 5].copy_from_slice(b"GROUP");
        put(bytes, 0x20, 5);
        put(bytes, 0x28, 15);
        bytes[0x30..0x34].copy_from_slice(&123_u32.to_le_bytes());
        bytes[OVERRIDE_NAME..OVERRIDE_NAME + 6].copy_from_slice(b"legacy");
        put(bytes, OVERRIDE_NAME_LENGTH, 6);
        put(bytes, OVERRIDE_NAME + 24, 15);
        bytes[FLAG_B] = 1;
        bytes[0x14c..0x150].copy_from_slice(&1_u32.to_le_bytes());
        put(bytes, 0x150, 0x1200);
        let (result, arena, required) = run(&memory, 11);
        let named = row(result);
        assert_eq!(
            &arena[named.custom.0..named.custom.0 + named.custom.1],
            b"legacy"
        );
        assert_eq!(required, 11);
        assert_eq!(count(&memory), (Ok(Naming::DoesNotFit), 11));
        put(&mut memory.bytes, 0x200, 0x1300);
        put(&mut memory.bytes, 0x300, 0x1400);
        put(&mut memory.bytes, 0x400, GROUP_VTABLE_RVA.0 as u64);
        memory.bytes[0x408..0x40d].copy_from_slice(b"group");
        put(&mut memory.bytes, 0x418, 5);
        put(&mut memory.bytes, 0x420, 15);
        memory.bytes[0x428..0x42c].copy_from_slice(&123_u32.to_le_bytes());
        let (result, _, required) = run(&memory, 5);
        assert_eq!(row(result).custom.1, 0);
        assert_eq!(required, 5);
        memory.bytes[0x428..0x42c].copy_from_slice(&124_u32.to_le_bytes());
        assert_eq!(row(run(&memory, 11).0).custom.1, 6);
        memory.bytes[0x428..0x42c].copy_from_slice(&123_u32.to_le_bytes());
        memory.bytes[0x408] = 0xff;
        assert_eq!(
            run(&memory, 11).0,
            Err("operative name group comparison locale unavailable")
        );
    }

    #[test]
    fn a_programmatic_name_is_published_for_a_null_group() {
        let mut bytes = [0_u8; 0xb0];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[FLAG_A] = 1;
        bytes[FLAG_B] = 1;
        bytes[OVERRIDE_NAME..OVERRIDE_NAME + 4].copy_from_slice(b"1st\0");
        bytes[OVERRIDE_NAME + 16..OVERRIDE_NAME + 24].copy_from_slice(&3_u64.to_le_bytes());
        bytes[OVERRIDE_NAME + 24..OVERRIDE_NAME + 32].copy_from_slice(&15_u64.to_le_bytes());
        let (result, arena, required) = run(&guard(&bytes), 64);
        let named = row(result);
        assert_eq!(named.programmatic, 1);
        assert_eq!(
            &arena[named.custom.0..named.custom.0 + named.custom.1],
            b"1st"
        );
        assert_eq!((named.group.1, required), (0, 3));
        assert_eq!(count(&guard(&bytes)), (Ok(Naming::DoesNotFit), 3));
        let mut unnamed = bytes;
        unnamed[OVERRIDE_NAME + 16..OVERRIDE_NAME + 24].fill(0);
        let named = row(run(&guard(&unnamed), 64).0);
        assert_eq!((named.custom.1, named.group.1), (0, 0));
        bytes[TYPE] = 3;
        assert_eq!(row(run(&guard(&bytes), 64).0).custom.1, 0);
    }

    #[test]
    fn one_copy_covers_the_member_and_a_programmatic_override_of_another_type_is_not_published() {
        let mut bytes = [0xff; 0xb0];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[TYPE..TYPE + 4].copy_from_slice(&1_i32.to_le_bytes());
        bytes[ORDER..ORDER + 4].copy_from_slice(&(-7_i32).to_le_bytes());
        bytes[FLAG_A] = 0;
        bytes[FLAG_B] = 1;
        let memory = guard(&bytes);
        let first = row(run(&memory, 64).0);
        assert_eq!(
            (first.kind, first.order, first.ordered, first.programmatic),
            (1, -7, 0, 1)
        );
        assert_eq!(first.custom.1, 0);
        assert_eq!(*memory.reads.borrow(), [(0x1000, INPUT_SPAN)]);
        bytes[FLAG_B] = 0;
        bytes[OVERRIDE_NAME..OVERRIDE_NAME + 6].copy_from_slice(b"Guard\0");
        bytes[OVERRIDE_NAME + 16..OVERRIDE_NAME + 24].copy_from_slice(&5_u64.to_le_bytes());
        bytes[OVERRIDE_NAME + 24..OVERRIDE_NAME + 32].copy_from_slice(&15_u64.to_le_bytes());
        let memory = guard(&bytes);
        let (result, arena, required) = run(&memory, 64);
        let named = row(result);
        assert_eq!(
            &arena[named.custom.0..named.custom.0 + named.custom.1],
            b"Guard"
        );
        assert_eq!((named.group.1, required), (0, 5));
        assert_eq!(memory.reads.borrow().len(), 1);
        let mut changed = bytes;
        changed[0x60..0x80].fill(0x5a);
        assert_eq!(run(&guard(&changed), 64).0, Ok(Naming::Row(named)));
        assert_eq!(count(&guard(&bytes)), (Ok(Naming::DoesNotFit), 5));
        bytes[OVERRIDE_NAME + 5] = b'!';
        assert_eq!(
            run(&guard(&bytes), 64).0,
            Err("division name override terminator unreadable or invalid")
        );
    }

    #[test]
    fn names_of_any_length_go_to_the_arena_after_their_terminator_is_proved() {
        let mut bytes = vec![0; 0x800];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[TYPE..TYPE + 4].copy_from_slice(&3_i32.to_le_bytes());
        bytes[FLAG_A] = 1;
        let name: Vec<u8> = (0..300).map(|i| b'a' + (i % 26) as u8).collect();
        bytes[0x100..0x100 + name.len()].copy_from_slice(&name);
        bytes[OVERRIDE_NAME..OVERRIDE_NAME + 8].copy_from_slice(&0x1100_u64.to_le_bytes());
        bytes[OVERRIDE_NAME + 16..OVERRIDE_NAME + 24]
            .copy_from_slice(&(name.len() as u64).to_le_bytes());
        bytes[OVERRIDE_NAME + 24..OVERRIDE_NAME + 32].copy_from_slice(&511_u64.to_le_bytes());
        let memory = guard(&bytes);
        let (result, arena, required) = run(&memory, 512);
        let named = row(result);
        assert_eq!(&arena[..named.custom.1], name.as_slice());
        assert_eq!(required, name.len());
        assert_eq!(
            *memory.reads.borrow(),
            [
                (0x1000, INPUT_SPAN),
                (0x1100, 1),
                (0x1100 + name.len() - 1, 2),
                (0x1100, name.len())
            ]
        );
        let memory = guard(&bytes);
        assert_eq!(count(&memory), (Ok(Naming::DoesNotFit), 300));
        assert_eq!(memory.reads.borrow().len(), 3);
        for (at, value, reason) in [
            (
                0x100 + name.len(),
                b'x',
                "division name override terminator unreadable or invalid",
            ),
            (
                OVERRIDE_NAME,
                0,
                "division name override pointer unreadable or null",
            ),
            (
                OVERRIDE_NAME + 24,
                3,
                "division name override header invalid",
            ),
            (0, 0, "division name vtable mismatch"),
        ] {
            let mut bad = bytes.clone();
            bad[at] = value;
            if at == OVERRIDE_NAME {
                bad[at..at + 8].fill(0);
            }
            if at == OVERRIDE_NAME + 24 {
                bad[at..at + 8].copy_from_slice(&3_u64.to_le_bytes());
            }
            let (result, _, required) = run(&guard(&bad), 512);
            assert_eq!((result, required), (Err(reason), 0));
        }
        assert_eq!(
            run(&guard(&bytes[..0x40]), 512).0,
            Err("division name unreadable")
        );
    }

    #[test]
    fn negative_and_zero_orders_survive_and_a_wrong_vtable_fails() {
        let mut bytes = vec![0; 0x100];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[ORDER..ORDER + 4].copy_from_slice(&i32::MIN.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(order(&memory, 0, 0x1000), Ok(i32::MIN));
        bytes[ORDER..ORDER + 4].fill(0);
        assert_eq!(
            order(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Ok(0)
        );
        bytes[0] ^= 1;
        assert_eq!(
            order(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Err("division name vtable mismatch")
        );
        assert_eq!(
            order(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &[]
                },
                0,
                0x1000
            ),
            Err("division name vtable mismatch")
        );
    }
}
