
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native::{cequipment_variant::pool, cgame_state, corders_group};
use engine_native_parts::equipment_entry;
use crate::{ncombat_log_cequipment_loss as loss, ncombat_log_corders_group_logs as logs};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_a260);
pub const GROUPS: usize = 0x08;
pub const COUNTRY: usize = 0x20;
pub const SIZE: usize = 0x28;

pub const PRUNE_RVA: Rva = Rva(0x00d3_a9f0);

pub const RECOVERED_KIND: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Months {
    All,
    Window,
}

pub const WINDOW_ENTRIES: u32 = 2;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub group: u32,
    pub kind: u8,
    pub month: u16,
    pub variant: u32,
    pub date: i32,
    pub amount_raw: i64,
}

impl Row {
    pub fn key(self) -> (u32, u32, u8, u16, u32) {
        (
            self.country,
            self.group,
            self.kind,
            self.month,
            self.variant,
        )
    }
}

const BATCH: usize = 64;

const MANAGER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: SIZE,
}];
const MANAGER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&MANAGER_FIELDS, SIZE).expect("valid record layout");

const GROUP_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: logs::SIZE,
}];
const GROUP_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&GROUP_FIELDS, logs::SIZE).expect("valid record layout");
const GROUP_IDENTITY_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: logs::GROUP,
        destination_offset: 8,
        len: 8,
    },
];
const GROUP_IDENTITY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&GROUP_IDENTITY_FIELDS, 16).expect("valid record layout");
const KINDS: usize = logs::REASON_COUNT + 1;

const ENTRY_BYTES: usize = loss::POOL + pool::ITEMS + 16;
const _: () = assert!(ENTRY_BYTES <= loss::SIZE);
const ENTRY_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: ENTRY_BYTES,
}];
const ENTRY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ENTRY_FIELDS, ENTRY_BYTES).expect("valid record layout");

const VARIANT_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 16,
}];
const VARIANT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&VARIANT_FIELDS, 16).expect("valid record layout");

struct Vtables {
    manager: u64,
    group: u64,
    entry: u64,
    pool: u64,
    variant: usize,
}

const SMALL_MEMO_SLOTS: usize = 256;

struct SmallVariantMemo {
    pointers: [usize; SMALL_MEMO_SLOTS],
    ids: [u32; SMALL_MEMO_SLOTS],
    len: usize,
}

impl Default for SmallVariantMemo {
    fn default() -> Self {
        Self {
            pointers: [0; SMALL_MEMO_SLOTS],
            ids: [0; SMALL_MEMO_SLOTS],
            len: 0,
        }
    }
}

impl SmallVariantMemo {
    #[inline]
    fn slot(pointer: usize) -> usize {
        (pointer >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15) & (SMALL_MEMO_SLOTS - 1)
    }

    fn get(&self, pointer: usize) -> Option<u32> {
        let mut slot = Self::slot(pointer);
        loop {
            match self.pointers[slot] {
                0 => return None,
                found if found == pointer => return Some(self.ids[slot]),
                _ => slot = (slot + 1) & (SMALL_MEMO_SLOTS - 1),
            }
        }
    }

    fn insert(&mut self, pointer: usize, id: u32) {
        if self.len >= SMALL_MEMO_SLOTS / 2 {
            return;
        }
        let mut slot = Self::slot(pointer);
        while self.pointers[slot] != 0 {
            if self.pointers[slot] == pointer {
                return;
            }
            slot = (slot + 1) & (SMALL_MEMO_SLOTS - 1);
        }
        self.pointers[slot] = pointer;
        self.ids[slot] = id;
        self.len += 1;
    }
}

enum Memo<'a> {
    None,
    Heap(&'a mut VariantMemo),
    Stack(&'a mut SmallVariantMemo),
}

impl Memo<'_> {
    fn reserve(&mut self, additional: usize) -> std::io::Result<()> {
        match self {
            Self::Heap(memo) => memo.reserve(additional),
            Self::None | Self::Stack(_) => Ok(()),
        }
    }

    fn get(&self, pointer: usize) -> Option<u32> {
        match self {
            Self::None => None,
            Self::Heap(memo) => memo.get(pointer),
            Self::Stack(memo) => memo.get(pointer),
        }
    }

    fn insert(&mut self, pointer: usize, id: u32) {
        match self {
            Self::None => {}
            Self::Heap(memo) => memo.insert(pointer, id),
            Self::Stack(memo) => memo.insert(pointer, id),
        }
    }
}

struct Output<'a> {
    rows: Buffer<'a, Row>,
    len: usize,
    unread_rows: u32,
    memo: Memo<'a>,
}

#[derive(Default)]
pub struct VariantMemo {
    pointers: Vec<usize>,
    ids: Vec<u32>,
    len: usize,
}

impl VariantMemo {
    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * size_of::<usize>() + self.ids.capacity() * size_of::<u32>()
    }

    fn reserve(&mut self, additional: usize) -> std::io::Result<()> {
        let required = crate::allocation::add(crate::allocation::add(self.len, additional)?, 1)?;
        let slots = required
            .checked_next_power_of_two()
            .ok_or_else(crate::allocation::oom)?;
        if slots <= self.pointers.len() {
            return Ok(());
        }
        let mut pointers = crate::allocation::filled(slots, 0)?;
        let mut ids = crate::allocation::filled(slots, 0)?;
        for (&pointer, &id) in self.pointers.iter().zip(&self.ids) {
            if pointer == 0 {
                continue;
            }
            let mut slot = (pointer >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15) & (slots - 1);
            while pointers[slot] != 0 {
                slot = (slot + 1) & (slots - 1);
            }
            pointers[slot] = pointer;
            ids[slot] = id;
        }
        self.pointers = pointers;
        self.ids = ids;
        Ok(())
    }

    fn clear(&mut self) {
        if self.len != 0 {
            self.pointers.fill(0);
            self.len = 0;
        }
    }

    fn slot(&self, pointer: usize) -> Option<usize> {
        let mask = self.pointers.len().checked_sub(1)?;
        Some((pointer >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15) & mask)
    }

    fn get(&self, pointer: usize) -> Option<u32> {
        let mut slot = self.slot(pointer)?;
        loop {
            match self.pointers[slot] {
                0 => return None,
                found if found == pointer => return Some(self.ids[slot]),
                _ => slot = (slot + 1) & (self.pointers.len() - 1),
            }
        }
    }

    fn insert(&mut self, pointer: usize, id: u32) {
        let Some(mut slot) = self.slot(pointer) else {
            return;
        };
        if self.len + 1 >= self.pointers.len() {
            return;
        }
        while self.pointers[slot] != 0 {
            if self.pointers[slot] == pointer {
                return;
            }
            slot = (slot + 1) & (self.pointers.len() - 1);
        }
        self.pointers[slot] = pointer;
        self.ids[slot] = id;
        self.len += 1;
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut select: impl FnMut(u32) -> Option<Months>,
    rows: &mut Vec<Row>,
    memo: &mut VariantMemo,
) -> Outcome {
    walk_prepared(
        memory,
        base,
        game_state,
        &mut (),
        |_, _| Ok(()),
        |_, country| Ok(select(country)),
        rows,
        memo,
    )
}

pub fn row_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    match walk_prepared_buffer(
        memory,
        base,
        game_state,
        &mut (),
        |_, _| Ok(()),
        |_, _| Ok(Some(Months::All)),
        Buffer::Count,
        None,
        &mut |_, _, _, _| Ok(()),
    ) {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("combat log row demand overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the caller's prepare and select callbacks share its state, and the rows and memo are caller-owned"
)]
pub fn walk_prepared<T>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: &mut T,
    prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    select: impl FnMut(&mut T, u32) -> Result<Option<Months>, &'static str>,
    rows: &mut Vec<Row>,
    memo: &mut VariantMemo,
) -> Outcome {
    walk_prepared_buffer(
        memory,
        base,
        game_state,
        state,
        prepare,
        select,
        Buffer::Growing(rows),
        Some(memo),
        &mut |_, _, _, _| Ok(()),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the caller's prepare, select and group callbacks share its state, and the rows and memo are caller-owned"
)]
pub fn walk_prepared_observed<T>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: &mut T,
    prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    select: impl FnMut(&mut T, u32) -> Result<Option<Months>, &'static str>,
    rows: &mut Vec<Row>,
    memo: &mut VariantMemo,
    visit_group: &mut impl FnMut(u32, usize, u32, &[u8]) -> Result<(), &'static str>,
) -> Outcome {
    walk_prepared_buffer(
        memory,
        base,
        game_state,
        state,
        prepare,
        select,
        Buffer::Growing(rows),
        Some(memo),
        visit_group,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the caller's prepare, select and group callbacks share its state, and the rows buffer and memo are caller-owned"
)]
fn walk_prepared_buffer<T>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: &mut T,
    prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    mut select: impl FnMut(&mut T, u32) -> Result<Option<Months>, &'static str>,
    rows: Buffer<'_, Row>,
    mut memo: Option<&mut VariantMemo>,
    visit_group: &mut impl FnMut(u32, usize, u32, &[u8]) -> Result<(), &'static str>,
) -> Outcome {
    if let Some(memo) = memo.as_deref_mut() {
        memo.clear();
    }
    let mut output = Output {
        rows,
        len: 0,
        unread_rows: 0,
        memo: match memo {
            Some(memo) => Memo::Heap(memo),
            None => Memo::None,
        },
    };
    let read = |state: &mut T, memory: &_, vtables: &Vtables, country, groups| {
        let Some(months) = select(state, country)? else {
            return Ok(());
        };
        let (start, unread) = (output.len, output.unread_rows);
        read_groups_observed(
            memory,
            vtables,
            country,
            groups,
            months,
            &mut output,
            visit_group,
        )?;
        if output.unread_rows != unread {
            let partial = u32::try_from(output.len - start)
                .map_err(|_| "combat log unread row count exceeds u32")?;
            output.unread_rows = output
                .unread_rows
                .checked_add(partial)
                .ok_or("combat log unread row count exceeds u32")?;
            output.len = start;
        }
        Ok(())
    };
    let result = for_each_manager(memory, base, game_state, state, prepare, read);
    match result {
        Ok(()) if output.unread_rows == 0 => Outcome::Rows { len: output.len },
        Ok(()) => Outcome::Truncated {
            len: output.len,
            unread_rows: output.unread_rows,
        },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

pub fn visit_retained_groups(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visit: impl FnMut(u32, usize, u32, &[u8]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for_each_manager(
        memory,
        base,
        game_state,
        &mut (),
        |_, _| Ok(()),
        |_, memory, vtables, country, groups| {
            for_each_group(memory, vtables, groups, |pointer, ordinal, bytes, _| {
                visit(country, pointer, ordinal, bytes)
            })
        },
    )
}

fn for_each_manager<M: Memory, T>(
    memory: &M,
    base: usize,
    game_state: usize,
    state: &mut T,
    mut prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    mut visit: impl FnMut(&mut T, &M, &Vtables, u32, Array) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let relocate =
        |key: Rva| engine_sites::at(base, key).ok_or("combat log module address overflow");
    let vtables = Vtables {
        manager: relocate(VTABLE_RVA)? as u64,
        group: relocate(logs::VTABLE_RVA)? as u64,
        entry: relocate(loss::VTABLE_RVA)? as u64,
        pool: relocate(pool::VTABLE_RVA)? as u64,
        variant: relocate(engine_native_parts::equipment_variant_identity::VTABLE_RVA)?,
    };
    let managers = game_state
        .checked_add(cgame_state::COMBAT_LOGS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("combat log manager array unreadable or invalid")?;
    if managers.count == 0 {
        return Err("combat log manager array has no sentinel slot");
    }
    prepare(state, managers.count as usize)?;
    let mut pointer_bytes = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut headers = [0_u8; BATCH * SIZE];
    let mut first = 1;
    while first < managers.count {
        let count = ((managers.count - first) as usize).min(BATCH);
        let span = managers
            .slice(first, count as u32, 8)
            .map_err(|_| "combat log manager window invalid")?;
        if !read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
            return Err("combat log manager pointers unreadable");
        }
        let mut live = 0;
        for bytes in pointer_bytes[..count * 8].chunks_exact(8) {
            let pointer = u64_at(bytes, 0) as usize;
            if pointer != 0 {
                pointers[live] = pointer;
                live += 1;
            }
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..live]),
            &MANAGER_LAYOUT,
            &mut headers[..live * SIZE],
        );
        for index in 0..live {
            if copied < index + 1 {
                return Err("combat log manager unreadable");
            }
            let bytes = &headers[index * SIZE..(index + 1) * SIZE];
            if u64_at(bytes, 0) != vtables.manager {
                return Err("combat log manager vtable mismatch");
            }
            let groups = Array::from_header(bytes[GROUPS..GROUPS + 16].try_into().unwrap(), 8)
                .ok_or("combat log group array invalid")?;
            let country = u32_at(bytes, COUNTRY);
            if groups.count != 0 {
                visit(state, memory, &vtables, country, groups)?;
            }
        }
        first += count as u32;
    }
    Ok(())
}

fn read_groups_observed(
    memory: &impl Memory,
    vtables: &Vtables,
    country: u32,
    groups: Array,
    months: Months,
    output: &mut Output<'_>,
    visit: &mut impl FnMut(u32, usize, u32, &[u8]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for_each_group(
        memory,
        vtables,
        groups,
        |pointer, ordinal, bytes, reference| {
            visit(country, pointer, ordinal, bytes)?;
            read_group(memory, vtables, country, bytes, reference, months, output)
        },
    )
}

fn read_group(
    memory: &impl Memory,
    vtables: &Vtables,
    country: u32,
    bytes: &[u8],
    reference: u64,
    months: Months,
    output: &mut Output<'_>,
) -> Result<(), &'static str> {
    {
        let group = (reference >> 32) as u32;
        for kind in 0..KINDS {
            let at = if kind < logs::REASON_COUNT {
                logs::REASONS + kind * logs::REASON_STRIDE
            } else {
                logs::RECOVERED
            };
            let entries = Array::from_header(bytes[at..at + 16].try_into().unwrap(), 8)
                .ok_or("combat log month array invalid")?;
            let entries = match months {
                Months::All => entries,
                Months::Window => {
                    let first = entries.count.saturating_sub(WINDOW_ENTRIES);
                    entries
                        .slice(first, entries.count - first, 8)
                        .map_err(|_| "combat log month window invalid")?
                }
            };
            let key = Key {
                country,
                group,
                kind: kind as u8,
            };
            read_entries(memory, vtables, key, entries, output)?;
        }
        Ok(())
    }
}

fn for_each_group(
    memory: &impl Memory,
    vtables: &Vtables,
    groups: Array,
    mut visit: impl FnMut(usize, u32, &[u8], u64) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut pointer_bytes = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut headers = [0_u8; BATCH * logs::SIZE];
    let mut first = 0;
    while first < groups.count {
        let count = ((groups.count - first) as usize).min(BATCH);
        let span = groups
            .slice(first, count as u32, 8)
            .map_err(|_| "combat log group window invalid")?;
        if !read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
            return Err("combat log group pointers unreadable");
        }
        for (pointer, bytes) in pointers.iter_mut().zip(pointer_bytes.chunks_exact(8)) {
            *pointer = u64_at(bytes, 0) as usize;
        }
        if pointers[..count].contains(&0) {
            return Err("combat log group pointer null");
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &GROUP_LAYOUT,
            &mut headers[..count * logs::SIZE],
        );
        for index in 0..count {
            if copied < index + 1 {
                return Err("combat log group unreadable");
            }
            let bytes = &headers[index * logs::SIZE..(index + 1) * logs::SIZE];
            if u64_at(bytes, 0) != vtables.group {
                return Err("combat log group vtable mismatch");
            }
            let reference = u64_at(bytes, logs::GROUP);
            if reference as u32 != corders_group::REFERENCE_TYPE {
                return Err("combat log group reference is not an orders group");
            }
            visit(pointers[index], first + index as u32, bytes, reference)?;
        }
        first += count as u32;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Key {
    country: u32,
    group: u32,
    kind: u8,
}

fn read_entries(
    memory: &impl Memory,
    vtables: &Vtables,
    key: Key,
    entries: Array,
    output: &mut Output<'_>,
) -> Result<(), &'static str> {
    let mut pointer_bytes = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut headers = [0_u8; BATCH * ENTRY_BYTES];
    let mut previous_month = None;
    let mut first = 0;
    while first < entries.count {
        let count = ((entries.count - first) as usize).min(BATCH);
        let span = entries
            .slice(first, count as u32, 8)
            .map_err(|_| "combat log month window invalid")?;
        if !read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
            return Err("combat log month pointers unreadable");
        }
        for (pointer, bytes) in pointers.iter_mut().zip(pointer_bytes.chunks_exact(8)) {
            *pointer = u64_at(bytes, 0) as usize;
        }
        if pointers[..count].contains(&0) {
            return Err("combat log month pointer null");
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &ENTRY_LAYOUT,
            &mut headers[..count * ENTRY_BYTES],
        );
        for index in 0..count {
            if copied < index + 1 {
                return Err("combat log month unreadable");
            }
            let bytes = &headers[index * ENTRY_BYTES..(index + 1) * ENTRY_BYTES];
            if u64_at(bytes, 0) != vtables.entry {
                return Err("combat log month vtable mismatch");
            }
            if key.kind != RECOVERED_KIND && i32_at(bytes, loss::REASON) != i32::from(key.kind) {
                return Err("combat log month reason differs from its array");
            }
            if u64_at(bytes, loss::POOL) != vtables.pool {
                return Err("combat log month pool vtable mismatch");
            }
            let date = i32_at(bytes, loss::DATE);
            let month = u16::try_from(loss::month_bucket(date))
                .map_err(|_| "combat log month outside the recorded range")?;
            if previous_month.is_some_and(|previous| previous >= month) {
                return Err("combat log months repeat or are out of order");
            }
            previous_month = Some(month);
            let header = loss::POOL + pool::ITEMS;
            let items = Array::from_header(
                bytes[header..header + 16].try_into().unwrap(),
                equipment_entry::STRIDE,
            )
            .ok_or("combat log pool entry array invalid")?;
            read_pool(memory, vtables, key, month, date, items, output)?;
        }
        first += count as u32;
    }
    Ok(())
}

fn read_pool(
    memory: &impl Memory,
    vtables: &Vtables,
    key: Key,
    month: u16,
    date: i32,
    items: Array,
    output: &mut Output<'_>,
) -> Result<(), &'static str> {
    let required = output
        .len
        .checked_add(items.count as usize)
        .ok_or("combat log row demand overflow")?;
    output.rows.ensure(required)?;
    if output.unread_rows != 0 || items.count as usize > output.rows.len() - output.len {
        output.unread_rows = output
            .unread_rows
            .checked_add(items.count)
            .ok_or("combat log unread row count exceeds u32")?;
        return Ok(());
    }
    output
        .memo
        .reserve(items.count as usize)
        .map_err(|_| "combat log variant memo allocation failed")?;
    let start = output.len;
    let mut entries = [0_u8; BATCH * equipment_entry::STRIDE];
    let mut ids = [0_u32; BATCH];
    let mut misses = [0_usize; BATCH];
    let mut unknown = [0_usize; BATCH];
    let mut identities = [0_u8; BATCH * 16];
    let mut first = 0;
    while first < items.count {
        let count = ((items.count - first) as usize).min(BATCH);
        let span = items
            .slice(first, count as u32, equipment_entry::STRIDE)
            .map_err(|_| "combat log pool window invalid")?;
        let bytes = &mut entries[..count * equipment_entry::STRIDE];
        if !read_slice(memory, span.data, bytes) {
            return Err("combat log pool entries unreadable");
        }
        let mut missed = 0;
        for (index, entry) in bytes.chunks_exact(equipment_entry::STRIDE).enumerate() {
            let variant = u64_at(entry, equipment_entry::VARIANT) as usize;
            if variant == 0 {
                return Err("combat log pool variant pointer null");
            }
            match output.memo.get(variant) {
                Some(id) => ids[index] = id,
                None => {
                    misses[missed] = index;
                    unknown[missed] = variant;
                    missed += 1;
                }
            }
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&unknown[..missed]),
            &VARIANT_LAYOUT,
            &mut identities[..missed * 16],
        );
        for miss in 0..missed {
            if copied < miss + 1 {
                return Err("combat log pool variant unreadable");
            }
            let handle = engine_native_parts::equipment_variant_identity::identity_from_header(
                vtables.variant,
                identities[miss * 16..miss * 16 + 16].try_into().unwrap(),
            )?;
            let id = (handle >> 32) as u32;
            ids[misses[miss]] = id;
            output.memo.insert(unknown[miss], id);
        }
        for (index, entry) in bytes.chunks_exact(equipment_entry::STRIDE).enumerate() {
            output.rows[output.len] = Row {
                country: key.country,
                group: key.group,
                kind: key.kind,
                month,
                variant: ids[index],
                date,
                amount_raw: i64_at(entry, equipment_entry::AMOUNT),
            };
            output.len += 1;
        }
        first += count as u32;
    }
    let written = &mut output.rows[start..output.len];
    written.sort_unstable_by_key(|row| row.variant);
    if written
        .windows(2)
        .any(|pair| pair[0].variant == pair[1].variant)
    {
        return Err("combat log pool names a variant twice");
    }
    Ok(())
}

const BUCKET_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 24,
}];
const BUCKET_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BUCKET_FIELDS, 24).expect("valid record layout");

pub fn countries_with_deleted_groups(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visit: impl FnMut(u32),
) -> Result<(), &'static str> {
    let mut resolver = engine_native_parts::creference_registry::ScopedResolver::new(base);
    for_each_manager(
        memory,
        base,
        game_state,
        &mut (),
        |_, _| Ok(()),
        |_, memory, vtables, country, groups| {
            let mut deleted = false;
            for_each_group(memory, vtables, groups, |_, _, _, reference| {
                if !deleted && resolver.resolve(memory, reference)?.is_none() {
                    deleted = true;
                }
                Ok(())
            })?;
            if deleted {
                visit(country);
            }
            Ok(())
        },
    )
}

pub fn deleted_logs(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    rows: &mut [Row],
) -> Result<(u32, Outcome), &'static str> {
    let relocate =
        |key: Rva| engine_sites::at(base, key).ok_or("combat log module address overflow");
    let vtables = Vtables {
        manager: relocate(VTABLE_RVA)? as u64,
        group: relocate(logs::VTABLE_RVA)? as u64,
        entry: relocate(loss::VTABLE_RVA)? as u64,
        pool: relocate(pool::VTABLE_RVA)? as u64,
        variant: relocate(engine_native_parts::equipment_variant_identity::VTABLE_RVA)?,
    };
    let mut header = [0_u8; SIZE];
    if !read_slice(memory, manager, &mut header) {
        return Err("combat log manager unreadable");
    }
    if u64_at(&header, 0) != vtables.manager {
        return Err("combat log manager vtable mismatch");
    }
    let groups = Array::from_header(header[GROUPS..GROUPS + 16].try_into().unwrap(), 8)
        .ok_or("combat log group array invalid")?;
    let country = u32_at(&header, COUNTRY);
    let mut small_memo = SmallVariantMemo::default();
    let mut output = Output {
        rows: Buffer::Exact(rows),
        len: 0,
        unread_rows: 0,
        memo: Memo::Stack(&mut small_memo),
    };
    let mut resolver = engine_native_parts::creference_registry::ScopedResolver::new(base);
    let mut pointer_bytes = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut identities = [0_u8; BATCH * 16];
    let mut references = [0_u64; BATCH];
    let mut cell_headers = [None; BATCH];
    let mut cell_pointers = [0_usize; BATCH];
    let mut cell_map = [0_usize; BATCH];
    let mut cell_bytes = [0_u8; BATCH * 24];
    let mut group_bytes = [0_u8; logs::SIZE];
    let mut first = 0;
    while first < groups.count {
        let count = ((groups.count - first) as usize).min(BATCH);
        let span = groups
            .slice(first, count as u32, 8)
            .map_err(|_| "combat log group window invalid")?;
        if !read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
            return Err("combat log group pointers unreadable");
        }
        for (pointer, bytes) in pointers.iter_mut().zip(pointer_bytes.chunks_exact(8)) {
            *pointer = u64_at(bytes, 0) as usize;
        }
        if pointers[..count].contains(&0) {
            return Err("combat log group pointer null");
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &GROUP_IDENTITY_LAYOUT,
            &mut identities[..count * 16],
        );
        let mut cell_count = 0_usize;
        for index in 0..count {
            if copied < index + 1 {
                return Err("combat log group unreadable");
            }
            let identity = &identities[index * 16..(index + 1) * 16];
            if u64_at(identity, 0) != vtables.group {
                return Err("combat log group vtable mismatch");
            }
            let reference = u64_at(identity, 8);
            if reference as u32 != corders_group::REFERENCE_TYPE {
                return Err("combat log group reference is not an orders group");
            }
            references[index] = reference;
            if let Some((header, cell)) = resolver.first_cell(memory, reference)? {
                cell_headers[index] = Some(header);
                cell_pointers[cell_count] = cell;
                cell_map[index] = cell_count;
                cell_count += 1;
            } else {
                cell_headers[index] = None;
            }
        }
        let copied_cells = if cell_count != 0 {
            memory.copy_records(
                RecordSources::Pointers(&cell_pointers[..cell_count]),
                &BUCKET_LAYOUT,
                &mut cell_bytes[..cell_count * 24],
            )
        } else {
            0
        };
        for index in 0..count {
            let resolved = match cell_headers[index] {
                Some(header) => {
                    let slot = cell_map[index];
                    if slot < copied_cells {
                        let first_bucket: [u8; 24] = cell_bytes[slot * 24..(slot + 1) * 24]
                            .try_into()
                            .unwrap();
                        resolver.resolve_gathered(memory, header, references[index], first_bucket)?
                    } else {
                        resolver.resolve(memory, references[index])?
                    }
                }
                None => None,
            };
            if resolved.is_some() {
                continue;
            }
            if !read_slice(memory, pointers[index], &mut group_bytes) {
                return Err("combat log group unreadable");
            }
            read_group(
                memory,
                &vtables,
                country,
                &group_bytes,
                references[index],
                Months::All,
                &mut output,
            )?;
        }
        first += count as u32;
    }
    let outcome = if output.unread_rows == 0 {
        Outcome::Rows { len: output.len }
    } else {
        Outcome::Truncated {
            len: output.len,
            unread_rows: output.unread_rows,
        }
    };
    Ok((country, outcome))
}

pub fn viewing_country(memory: &impl Memory, game_state: usize) -> Option<i32> {
    game_state
        .checked_add(cgame_state::LOCAL_PLAYER_COUNTRY)
        .and_then(|address| read::<i32>(memory, address))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests;
