//! Each country's Combat Log and ordered equipment-loss month acquisition.
//!
//! Game state owns one manager per country slot, with slot zero null. Creation
//! sets its country link; loading may replace the manager. Writers choose the
//! positive expeditionary owner or roster owner and require an orders group.
//!
//! Land-loss recording also requires the country-gate operand to be below one
//! or the country to match a live player-roster country through canonical links.
//! This predicate tests countries, not player flags or AI control. The equipment
//! loss feature must also be enabled. Country admission alone does not establish
//! coverage when the division has no orders group. `CombatLogState` records
//! gate and prune observations; acquisition reads every country's held logs
//! irrespective of players, AI control or viewpoint.
//!
//! Only land equipment is logged by exact variant, orders group, reason and
//! calendar month. Managers serialize when they hold a group log. Air and naval
//! losses do not reach this history.
//!
//! Writers change only the last month entry, appending when its month differs.
//! An entry stops changing once a later entry exists. After a complete read in
//! the current or previous calendar month, [`Months::Window`] therefore needs
//! the last two entries. [`Months::All`] covers retained history for branch starts
//! or countries whose prior complete read is older or unknown.
//!
//! Daily pruning follows the hour-23 sample. It removes entries aged by the
//! loaded maximum-month define and group logs whose reference no longer resolves
//! or whose arrays are empty. A retired group cannot receive later writes, so
//! its held values remain final until pruning. [`countries_with_deleted_groups`]
//! selects countries holding those logs.

use crate::{
    Memory, cequipment_variant, cequipment_variant_pool as pool, cgame_state, corders_group,
    cpdx_array::Array,
    equipment_entry,
    memory::{RecordField, RecordLayout, RecordSources},
    ncombat_log_cequipment_loss as loss, ncombat_log_corders_group_logs as logs, read, read_slice,
    walk::{Buffer, Outcome},
};

/// Combat-log manager primary vtable.
pub const VTABLE_RVA: usize = 0x0295_d8d8;
/// CPdxArray of group-log pointers, appended by writers and serialized in order.
pub const GROUPS: usize = 0x08;
/// Manager raw country link, serialized as a tag.
pub const COUNTRY: usize = 0x20;
/// Manager allocation size.
pub const SIZE: usize = 0x28;

/// Per-country prune entry; RCX is the manager. It prunes old month entries,
/// then swap-removes and destroys group logs that are empty or no longer resolve.
pub const PRUNE_RVA: usize = 0x00cd_ab70;
/// Prune-entry instruction sentinel.
pub const PRUNE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x20, 0x33, 0xff, 0x48, 0x8b, 0xf1, 0x39,
];

/// Row kind of the recovered array; kinds 0 to 3 are the stored loss reasons.
pub const RECOVERED_KIND: u8 = 4;

/// Which month entries of each array a walk reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Months {
    /// Every retained entry.
    All,
    /// The last two entries: every entry that can have changed since a
    /// complete read in the same or the previous calendar month (see the
    /// module notes). A country last read earlier, or never, needs [`All`].
    ///
    /// [`All`]: Months::All
    Window,
}

/// Entries of each array a [`Months::Window`] read covers.
pub const WINDOW_ENTRIES: u32 = 2;

/// One variant's amount in one month entry of one orders group's log.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The manager's raw country link (held by the manager).
    pub country: u32,
    /// The orders group reference's id (its type is 53).
    pub group: u32,
    /// 0 to 3: the loss reason array; [`RECOVERED_KIND`]: the recovered array.
    pub kind: u8,
    /// Entry month bucket; see [`loss::month_bucket`].
    pub month: u16,
    /// The variant reference's id (its type is 70).
    pub variant: u32,
    /// The entry's stored date: the raw game hour of its first write.
    pub date: i32,
    /// The pool amount, fixed5.
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

/// Pointers, headers and entries read per guarded batch.
const BATCH: usize = 64;

/// Each manager, group log and month entry is copied whole in one span: the
/// fields read lie within the object's own allocation (0x28, 0xe8 and 0x68
/// bytes), so one guarded copy replaces one per field.
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
/// Kinds in read order: the four reasons, then the recovered array.
const KINDS: usize = logs::REASON_COUNT + 1;

/// A month entry through its pool's entry header.
const ENTRY_BYTES: usize = loss::POOL + pool::ITEMS + 16;
const _: () = assert!(ENTRY_BYTES <= loss::SIZE);
const ENTRY_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: ENTRY_BYTES,
}];
const ENTRY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ENTRY_FIELDS, ENTRY_BYTES).expect("valid record layout");

/// A variant's vtable and reference, as `cequipment_variant::identity` reads them.
const VARIANT_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 16,
}];
const VARIANT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&VARIANT_FIELDS, 16).expect("valid record layout");

/// Relocated vtables the walk authenticates against.
struct Vtables {
    manager: u64,
    group: u64,
    entry: u64,
    pool: u64,
    variant: usize,
}

/// Where the next row goes and how many did not fit.
struct Output<'a> {
    rows: Buffer<'a, Row>,
    len: usize,
    unread_rows: u32,
    memo: &'a mut VariantMemo,
}

/// Authenticated variant ids by pointer, for one walk only. Within a walk the
/// game is held at the sampling boundary, so a variant pointer read and
/// authenticated once names the same variant for the rest of that walk; the
/// memo is cleared at the start of every walk and never carried across hours.
pub struct VariantMemo {
    pointers: Vec<usize>,
    ids: Vec<u32>,
    worker_growth: bool,
    len: usize,
}

impl Default for VariantMemo {
    fn default() -> Self {
        Self::vacant()
    }
}

impl VariantMemo {
    /// Allocated once by the owner of the capture buffers.
    pub fn try_new() -> std::io::Result<Self> {
        Ok(Self {
            pointers: Vec::new(),
            ids: Vec::new(),
            worker_growth: true,
            len: 0,
        })
    }

    /// Allocation-free placeholder: remembers nothing.
    pub fn vacant() -> Self {
        Self {
            pointers: Vec::new(),
            ids: Vec::new(),
            worker_growth: false,
            len: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * size_of::<usize>() + self.ids.capacity() * size_of::<u32>()
    }

    fn reserve(&mut self, additional: usize) -> std::io::Result<()> {
        if !self.worker_growth {
            return Ok(());
        }
        // One empty cell terminates probing; rounding is index geometry, not
        // an identity admission limit or per-country storage estimate.
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

/// Reads every country's Combat Log equipment months: for each manager in
/// slot order, each group log in array order, the four reason arrays then the
/// recovered array, each month entry in array order and each pool entry in
/// pool order, one row per variant. Only the country's own losses and
/// recoveries are read; enemy losses, manpower, template statistics and
/// combat references are not.
///
/// Every object is authenticated (manager, group log, entry and pool vtables,
/// the group reference type 53, the variant vtable and reference type 70); a
/// loss entry must carry its array's reason, a month may appear once per
/// array and within the u16 key range, and a pool may name a variant once.
/// Any other form, and any failed read, makes the table unavailable. A
/// country is stored whole or not at all: from the first pool that does not
/// fit, that country's rows and every later pool's entries are counted
/// unread (the rows stay a prefix of whole countries), and no further amounts
/// are read.
/// Only the managers whose country `select` admits (`Some`) are followed
/// past their header, and `select` chooses which entries of that country's
/// arrays are read; it is called once per manager holding a group log, in
/// slot order. `memo` holds the variants this walk has authenticated; it is
/// cleared first.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut select: impl FnMut(u32) -> Option<Months>,
    rows: &mut [Row],
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

/// Stored group/month pool headers only, for attach and cadence sizing.
pub fn row_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    match walk(
        memory,
        base,
        game_state,
        |_| Some(Months::All),
        &mut [],
        &mut VariantMemo::vacant(),
    ) {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("combat log row demand overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

/// Prepares selection state from the live manager census; a failed
/// preparation or selection stops the walk immediately.
#[allow(clippy::too_many_arguments)]
pub fn walk_prepared<T>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: &mut T,
    prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    select: impl FnMut(&mut T, u32) -> Result<Option<Months>, &'static str>,
    rows: &mut [Row],
    memo: &mut VariantMemo,
) -> Outcome {
    walk_prepared_buffer(
        memory,
        base,
        game_state,
        state,
        prepare,
        select,
        Buffer::Fixed(rows),
        memo,
        &mut |_, _, _, _| Ok(()),
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn walk_prepared_growing<T>(
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
        memo,
        &mut |_, _, _, _| Ok(()),
    )
}

/// Shares already authenticated group objects with an initial-history consumer.
/// The callback adds no registry walk and must gate supplemental reads itself.
#[allow(clippy::too_many_arguments)]
pub fn walk_prepared_growing_observed<T>(
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
        memo,
        visit_group,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_prepared_buffer<T>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: &mut T,
    prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    mut select: impl FnMut(&mut T, u32) -> Result<Option<Months>, &'static str>,
    rows: Buffer<'_, Row>,
    memo: &mut VariantMemo,
    visit_group: &mut impl FnMut(u32, usize, u32, &[u8]) -> Result<(), &'static str>,
) -> Outcome {
    memo.clear();
    memo.worker_growth |= rows.is_growing();
    let mut output = Output {
        rows,
        len: 0,
        unread_rows: 0,
        memo,
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
        // A country is stored whole or not at all: rows of a country that
        // did not fit are withdrawn and counted unread, so a merge by
        // country never takes a partial read for a complete one.
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

/// Authenticates every manager in slot order and passes each one holding a
/// group log to `visit` with its country and group array.
fn for_each_manager<M: Memory, T>(
    memory: &M,
    base: usize,
    game_state: usize,
    state: &mut T,
    mut prepare: impl FnMut(&mut T, usize) -> Result<(), &'static str>,
    mut visit: impl FnMut(&mut T, &M, &Vtables, u32, Array) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let relocate = |rva: usize| {
        base.checked_add(rva)
            .ok_or("combat log module address overflow")
    };
    let vtables = Vtables {
        manager: relocate(VTABLE_RVA)? as u64,
        group: relocate(logs::VTABLE_RVA)? as u64,
        entry: relocate(loss::VTABLE_RVA)? as u64,
        pool: relocate(pool::VTABLE_RVA)? as u64,
        variant: relocate(cequipment_variant::VTABLE_RVA)?,
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
    // Slot zero is the null sentinel; serialization and writers start at slot one.
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

#[allow(clippy::too_many_arguments)]
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

/// Reads one authenticated group log's arrays (`bytes` is its whole copy).
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

/// Authenticates each group log of a manager in array order and passes its
/// whole copied object and its orders-group reference to `visit`.
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
            // The loader files an entry by its reason; the recovered array's
            // entries are built with reason 0 and it is not their key.
            if key.kind != RECOVERED_KIND && i32_at(bytes, loss::REASON) != i32::from(key.kind) {
                return Err("combat log month reason differs from its array");
            }
            if u64_at(bytes, loss::POOL) != vtables.pool {
                return Err("combat log month pool vtable mismatch");
            }
            let date = i32_at(bytes, loss::DATE);
            let month = u16::try_from(loss::month_bucket(date))
                .map_err(|_| "combat log month outside the recorded range")?;
            // Only the last entry of a matching month is merged, so months are unique and date ordered.
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
    // Month pools have no content limit. Worker storage grows from the original
    // pool header; callback storage retains its prefix and counting outcome.
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
    // Native pool headers supply the full original attempted variant demand.
    output
        .memo
        .reserve(items.count as usize)
        .map_err(|_| "combat log variant memo allocation failed")?;
    let start = output.len;
    let mut entries = [0_u8; BATCH * equipment_entry::STRIDE];
    let mut ids = [0_u32; BATCH];
    // Positions of this batch whose variant the memo does not hold yet.
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
            let handle = cequipment_variant::identity_from_header(
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
    // Pool addition merges by variant, so each variant occurs once per pool.
    let written = &output.rows[start..output.len];
    for (index, row) in written.iter().enumerate() {
        if written[..index]
            .iter()
            .any(|earlier| earlier.variant == row.variant)
        {
            return Err("combat log pool names a variant twice");
        }
    }
    Ok(())
}

/// Visit each country holding a group log whose orders-group reference no
/// longer resolves. The next daily prune deletes these logs; their values cannot
/// change. Managers and group logs retain the walk's authentication. Failed reads
/// or invalid registry headers remain errors.
pub fn countries_with_deleted_groups(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visit: impl FnMut(u32),
) -> Result<(), &'static str> {
    let mut resolver = crate::creference_registry::HeaderResolver::default();
    for_each_manager(
        memory,
        base,
        game_state,
        &mut (),
        |_, _| Ok(()),
        |_, memory, vtables, country, groups| {
            let mut deleted = false;
            for_each_group(memory, vtables, groups, |_, _, _, reference| {
                if !deleted && resolver.resolve(memory, base, reference)?.is_none() {
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

/// Read every month of retired-group logs from the manager at prune entry
/// before destruction. Their values are final. Return country and rows written;
/// omitted rows remain the error-free `Truncated` count. Resolving is passive.
pub fn deleted_logs(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    rows: &mut [Row],
) -> Result<(u32, Outcome), &'static str> {
    let relocate = |rva: usize| {
        base.checked_add(rva)
            .ok_or("combat log module address overflow")
    };
    let vtables = Vtables {
        manager: relocate(VTABLE_RVA)? as u64,
        group: relocate(logs::VTABLE_RVA)? as u64,
        entry: relocate(loss::VTABLE_RVA)? as u64,
        pool: relocate(pool::VTABLE_RVA)? as u64,
        variant: relocate(cequipment_variant::VTABLE_RVA)?,
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
    // No memo: the detour runs on the game's workers and must not allocate.
    let mut memo = VariantMemo::vacant();
    let mut output = Output {
        rows: Buffer::Fixed(rows),
        len: 0,
        unread_rows: 0,
        memo: &mut memo,
    };
    let mut resolver = crate::creference_registry::HeaderResolver::default();
    for_each_group(memory, &vtables, groups, |_, _, bytes, reference| {
        if resolver.resolve(memory, base, reference)?.is_some() {
            return Ok(());
        }
        read_group(
            memory,
            &vtables,
            country,
            bytes,
            reference,
            Months::All,
            &mut output,
        )
    })?;
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

/// The writer's country-gate operand (`cgame_state::LOCAL_PLAYER_COUNTRY`), which every Combat Log
/// writer tests with `< 1` before its CHuman country-match predicate.
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
