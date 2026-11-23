
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        Gathered,
        RecordField,
        RecordLayout,
        gather_records,
        packed_fields,
        packed_map,
        packed_stride,
    },
    read,
    read_slice,
    walk::Outcome,
};
use crate::{carmy, corders_group_member, ctheatre, cunit, cunit_leader};
use engine_native_parts::{
    carmy_group,
    creference_registry as registry,
    ctheater_group,
    reference_bucket as bucket,
};

pub const VTABLE_RVA: Rva = Rva(0x029c_edc0);
pub use engine_executable::native_values::orders_group::REFERENCE_TYPE;
pub const HANDLE: usize = 0x08;
pub const IS_ARMY_GROUP: usize = 0x39;
pub const MOTORIZATION_LEVEL: usize = 0x38;
pub const COUNTRY: usize = 0x3c;
pub const MEMBER_DATA: usize = 0x50;
pub const MEMBER_CAPACITY: usize = 0x58;
pub const MEMBER_COUNT: usize = 0x5c;
pub const HQ_PROVIDER: usize = 0x68;
pub const LEADER: usize = 0x88;
pub const PRIMARY_ORDER_DATA: usize = 0x98;
pub const SECONDARY_ORDER_DATA: usize = 0xb0;
pub const TERTIARY_ORDER_DATA: usize = 0xc8;
pub const COLOUR: usize = 0x110;
pub const NAME: usize = 0x160;
pub const THEATER_GROUP: usize = 0x180;
pub const TRAINING: usize = 0x19d;
pub const STOP_AT_MAXIMUM: usize = 0x1a0;
pub const BORDER_WAR: usize = 0x1b0;
pub const WITHDRAWING: usize = 0x19b;
pub const HQ_DISTANCE: usize = 0x1c4;
pub const HQ_NEAREST_FRONT: usize = 0x1c8;
pub const INVASION_SOURCE_DISTANCE: usize = 0x1cc;
pub const PARENT: usize = 0x1b8;
pub const THEATRE: usize = 0x40;
pub const SIZE: usize = 0x230;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00c3_f480);

mod counter_tooltips;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Root {
    pub pointer: usize,
    pub handle: u64,
    pub country: u32,
}

impl Root {
    pub fn pointer(self) -> usize {
        self.pointer
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanRow {
    pub group: u64,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    pub primary_start: usize,
    pub primary_len: usize,
    pub fallback_start: usize,
    pub fallback_len: usize,
    pub name_start: usize,
    pub name_len: usize,
}

pub fn plan_pointer_handle(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u64, &'static str> {
    gathered_pointer_handle(memory, base, pointer, Gathered::NONE)
}

fn gathered_pointer_handle(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    record: Gathered<'_>,
) -> Result<u64, &'static str> {
    let vtable = record
        .get::<u64>(0)
        .or_else(|| read::<u64>(memory, pointer))
        .ok_or("plan group identity unreadable")?;
    if ![VTABLE_RVA, carmy_group::VTABLE_RVA]
        .into_iter()
        .any(|key| engine_sites::at(base, key).map(|v| v as u64) == Some(vtable))
    {
        return Err("plan group vtable mismatch");
    }
    let handle = crate::corder_instance::gathered_field::<u64>(memory, pointer, HANDLE, record)?;
    if handle as u32 != REFERENCE_TYPE {
        return Err("plan group reference type mismatch");
    }
    Ok(handle)
}

#[cfg(test)]
pub(crate) fn plan_identity(
    memory: &impl Memory,
    base: usize,
    root: Root,
) -> Result<(), &'static str> {
    plan_identity_gathered(memory, base, root, Gathered::NONE)
}

pub(crate) fn plan_identity_gathered(
    memory: &impl Memory,
    base: usize,
    root: Root,
    record: Gathered<'_>,
) -> Result<(), &'static str> {
    if gathered_pointer_handle(memory, base, root.pointer, record)? != root.handle
        || crate::corder_instance::gathered_field::<u32>(memory, root.pointer, COUNTRY, record)?
            != root.country
    {
        return Err("plan group census identity changed");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct PlanWalk {
    pub outcome: Outcome,
    pub orders: usize,
    pub names: usize,
}

const PLAN_SPANS: [(usize, usize); 5] = [
    (0, COUNTRY + 4),
    (PRIMARY_ORDER_DATA, 16),
    (SECONDARY_ORDER_DATA, 16),
    (COLOUR, 16),
    (NAME, engine_native_memory::msvc_string::STRIDE),
];
const PLAN_FIELDS: [RecordField; 5] = packed_fields(PLAN_SPANS);
const PLAN_STRIDE: usize = packed_stride(PLAN_SPANS);
const PLAN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PLAN_FIELDS, PLAN_STRIDE).expect("valid group plan layout");
const PLAN_MAP: [u16; NAME + engine_native_memory::msvc_string::STRIDE] = packed_map(PLAN_SPANS);

fn prefetch_plan(memory: &impl Memory, root: &Root) {
    for (offset, len) in PLAN_SPANS {
        if let Some(address) = root.pointer.checked_add(offset) {
            memory.prefetch(address, len);
        }
    }
}

fn prefetch_dependents(memory: &impl Memory, record: Gathered<'_>) {
    if let Some(header) = record.get::<[u8; engine_native_memory::msvc_string::STRIDE]>(NAME)
        && let Some((address, len)) = engine_native_memory::msvc_string::heap_span(&header)
    {
        memory.prefetch(address, len);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn walk_plans(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    source: Outcome,
    visits: &[crate::corder_instance::Visit],
    order_source: Outcome,
    output: &mut [PlanRow],
    orders: &mut Vec<u32>,
    names: &mut Vec<u8>,
) -> PlanWalk {
    const BATCH: usize = crate::corder_instance::PLAN_BATCH;
    let mut used = 0;
    let mut names_used = 0;
    let result = (|| {
        let Outcome::Rows { len: count } = source else {
            return Err("group plans require complete group census");
        };
        if count != roots.len() {
            return Err("group plan census length mismatch");
        }
        let Outcome::Rows { len: expected } = order_source else {
            return Err("group plans require a complete army order census");
        };
        if visits.iter().filter(|v| v.pointer != 0).count() != expected {
            return Err("group plan order census count mismatch");
        }
        let len = count.min(output.len());
        let walked = &roots[..len];
        for root in walked.iter().take(2 * BATCH) {
            prefetch_plan(memory, root);
        }
        let mut records = [[0_u8; BATCH * PLAN_STRIDE]; 2];
        let mut complete = [[false; BATCH]; 2];
        let mut slots = [0_u8; 64 * 8];
        let stage = |batch_start: usize, records: &mut [u8], complete: &mut [bool]| {
            let batch = &walked[batch_start.min(len)..(batch_start + BATCH).min(len)];
            let mut pointers = [0_usize; BATCH];
            for (pointer, root) in pointers.iter_mut().zip(batch) {
                *pointer = root.pointer;
            }
            gather_records(
                memory,
                &pointers[..batch.len()],
                &PLAN_LAYOUT,
                records,
                complete,
            );
            for index in 0..batch.len() {
                prefetch_dependents(
                    memory,
                    Gathered::row(&PLAN_LAYOUT, &PLAN_MAP, records, complete, index),
                );
            }
        };
        stage(0, &mut records[0], &mut complete[0]);
        for (batch_index, batch_start) in (0..len).step_by(BATCH).enumerate() {
            let [first, second] = &mut records;
            let [first_complete, second_complete] = &mut complete;
            let ((records, complete), (next_records, next_complete)) = if batch_index % 2 == 0 {
                ((&*first, &*first_complete), (second, second_complete))
            } else {
                ((&*second, &*second_complete), (first, first_complete))
            };
            stage(batch_start + BATCH, next_records, next_complete);
            let batch = &walked[batch_start..(batch_start + BATCH).min(len)];
            for (index, root) in batch.iter().enumerate() {
                let owner = batch_start + index;
                if let Some(next) = walked.get(owner + 2 * BATCH) {
                    prefetch_plan(memory, next);
                }
                let record = Gathered::row(&PLAN_LAYOUT, &PLAN_MAP, records, complete, index);
                let row = &mut output[owner];
                plan_identity_gathered(memory, base, *root, record)?;
                let raw = crate::corder_instance::gathered_field::<[u8; 16]>(
                    memory,
                    root.pointer,
                    COLOUR,
                    record,
                )?;
                *row = PlanRow {
                    group: root.handle,
                    red: u32::from_le_bytes(raw[0..4].try_into().unwrap()),
                    green: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
                    blue: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
                    alpha: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
                    ..Default::default()
                };
                let name = root
                    .pointer
                    .checked_add(NAME)
                    .ok_or("group plan name address overflow")?;
                (row.name_start, row.name_len) = match record
                    .get::<[u8; engine_native_memory::msvc_string::STRIDE]>(NAME)
                {
                    Some(header) => engine_native_memory::msvc_string::append_with_header(
                        memory,
                        &header,
                        names,
                        &mut names_used,
                    )?,
                    None => {
                        engine_native_memory::msvc_string::append(memory, name, names, &mut names_used)?
                    }
                };
                for (offset, start, list_len) in [
                    (
                        PRIMARY_ORDER_DATA,
                        &mut row.primary_start,
                        &mut row.primary_len,
                    ),
                    (
                        SECONDARY_ORDER_DATA,
                        &mut row.fallback_start,
                        &mut row.fallback_len,
                    ),
                ] {
                    let array = root
                        .pointer
                        .checked_add(offset)
                        .and_then(|address| {
                            record
                                .get::<[u8; 16]>(offset)
                                .or_else(|| read::<[u8; 16]>(memory, address))
                        })
                        .and_then(|header| Array::from_header(header, 8))
                        .ok_or("group plan root array invalid")?;
                    *start = used;
                    for first in (0..array.count).step_by(64) {
                        let batched = (array.count - first).min(64);
                        let copied = array.element(first, 8).is_some_and(|address| {
                            read_slice(memory, address, &mut slots[..batched as usize * 8])
                        });
                        for index in first..first + batched {
                            let pointer = (if copied {
                                let at = (index - first) as usize * 8;
                                u64::from_le_bytes(slots[at..at + 8].try_into().unwrap())
                            } else {
                                array
                                    .element(index, 8)
                                    .and_then(|slot| read::<u64>(memory, slot))
                                    .ok_or("group plan root pointer unreadable")?
                            }) as usize;
                            let instance =
                                crate::corder_instance::census_instance(visits, pointer, owner)?;
                            crate::corder_instance::grow_payload(
                                orders,
                                used + 1,
                                "group plan root allocation failed",
                            )?;
                            orders[used] = instance;
                            used += 1;
                        }
                    }
                    *list_len = used - *start;
                }
            }
        }
        output[..len].sort_unstable_by_key(|r| r.group);
        Ok(if len == count {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: (count - len) as u32,
            }
        })
    })();
    PlanWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        orders: used,
        names: names_used,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CounterInputsRow {
    pub group: u64,
    pub withdrawing: u8,
    pub hq_distance: Option<i32>,
    pub leader_enroute_days: Option<i32>,
    pub border_war_presence: Option<u8>,
    pub invasion_source_distance: Option<i32>,
    pub hq_nearest_front: Option<i32>,
}

pub fn walk_counter_inputs(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    source: Outcome,
    output: &mut [CounterInputsRow],
) -> Outcome {
    walk_counter_inputs_in_state(memory, base, 0, roots, source, output)
}

pub fn walk_counter_inputs_in_state(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roots: &[Root],
    source: Outcome,
    output: &mut [CounterInputsRow],
) -> Outcome {
    counter_inputs(memory, base, game_state, roots, source, output)
}

const COUNTER_PREFIX_SPANS: [(usize, usize); 2] = [(0, COUNTRY + 4), (WITHDRAWING, 1)];
const COUNTER_PREFIX_FIELDS: [RecordField; 2] = packed_fields(COUNTER_PREFIX_SPANS);
const COUNTER_PREFIX_STRIDE: usize = packed_stride(COUNTER_PREFIX_SPANS);
const COUNTER_PREFIX_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COUNTER_PREFIX_FIELDS, COUNTER_PREFIX_STRIDE)
        .expect("valid group counter layout");
const COUNTER_PREFIX_MAP: [u16; WITHDRAWING + 1] = packed_map(COUNTER_PREFIX_SPANS);
const COUNTER_SUFFIX_SPANS: [(usize, usize); 2] = [
    (LEADER, 8),
    (BORDER_WAR, INVASION_SOURCE_DISTANCE + 4 - BORDER_WAR),
];
const COUNTER_SUFFIX_FIELDS: [RecordField; 2] = packed_fields(COUNTER_SUFFIX_SPANS);
const COUNTER_SUFFIX_STRIDE: usize = packed_stride(COUNTER_SUFFIX_SPANS);
const COUNTER_SUFFIX_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COUNTER_SUFFIX_FIELDS, COUNTER_SUFFIX_STRIDE)
        .expect("valid group counter suffix layout");
const COUNTER_SUFFIX_MAP: [u16; INVASION_SOURCE_DISTANCE + 4] = packed_map(COUNTER_SUFFIX_SPANS);
const COUNTER_BATCH: usize = crate::corder_instance::PLAN_BATCH;
const ROOT_SPANS: [(usize, usize); 2] = [(0, 16), (COUNTRY, 4)];
const ROOT_FIELDS: [RecordField; 2] = packed_fields(ROOT_SPANS);
const ROOT_STRIDE: usize = packed_stride(ROOT_SPANS);
const ROOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ROOT_FIELDS, ROOT_STRIDE).expect("valid group root layout");

fn gather_counter_records(
    memory: &impl Memory,
    roots: &[Root],
    layout: &RecordLayout<'_>,
    records: &mut [u8],
    complete: &mut [bool],
) {
    let mut pointers = [0_usize; COUNTER_BATCH];
    for (pointer, root) in pointers.iter_mut().zip(roots) {
        *pointer = root.pointer;
    }
    gather_records(
        memory,
        &pointers[..roots.len()],
        layout,
        &mut records[..roots.len() * layout.stride()],
        &mut complete[..roots.len()],
    );
}

fn counter_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roots: &[Root],
    source: Outcome,
    output: &mut [CounterInputsRow],
) -> Outcome {
    let mut prefix = [0_u8; COUNTER_BATCH * COUNTER_PREFIX_STRIDE];
    let mut suffix = [0_u8; COUNTER_BATCH * COUNTER_SUFFIX_STRIDE];
    let mut complete = [false; COUNTER_BATCH];
    let result = (|| {
        let (count, missing) = match source {
            Outcome::Rows { len } => (len, 0),
            Outcome::Truncated { len, unread_rows } => (len, unread_rows),
            Outcome::Unavailable { reason } => return Err(reason),
        };
        let roots = roots
            .get(..count)
            .ok_or("orders group counter census exceeds its buffer")?;
        let len = count.min(output.len());
        for first in (0..len).step_by(COUNTER_BATCH) {
            let batch = &roots[first..(first + COUNTER_BATCH).min(len)];
            gather_counter_records(
                memory,
                batch,
                &COUNTER_PREFIX_LAYOUT,
                &mut prefix,
                &mut complete,
            );
            for (index, (root, row)) in batch.iter().zip(&mut output[first..]).enumerate() {
                let record = Gathered::row(
                    &COUNTER_PREFIX_LAYOUT,
                    &COUNTER_PREFIX_MAP,
                    &prefix,
                    &complete,
                    index,
                );
                plan_identity_gathered(memory, base, *root, record)?;
                let withdrawing = crate::corder_instance::gathered_field::<u8>(
                    memory,
                    root.pointer,
                    WITHDRAWING,
                    record,
                )?;
                *row = CounterInputsRow {
                    group: root.handle,
                    withdrawing,
                    hq_distance: None,
                    leader_enroute_days: None,
                    border_war_presence: None,
                    invasion_source_distance: None,
                    hq_nearest_front: None,
                };
            }
        }
        let unread = (count - len) as u32 + missing;
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    let outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    let established = outcome.len();
    let mut resolver = registry::HeaderResolver::default();
    for first in (0..established).step_by(COUNTER_BATCH) {
        let end = (first + COUNTER_BATCH).min(established);
        let batch = &roots[first..end];
        gather_counter_records(
            memory,
            batch,
            &COUNTER_SUFFIX_LAYOUT,
            &mut suffix,
            &mut complete,
        );
        for (index, (root, row)) in batch.iter().zip(&mut output[first..end]).enumerate() {
            let record = Gathered::row(
                &COUNTER_SUFFIX_LAYOUT,
                &COUNTER_SUFFIX_MAP,
                &suffix,
                &complete,
                index,
            );
            row.hq_distance = record.get::<i32>(HQ_DISTANCE).or_else(|| {
                root.pointer
                    .checked_add(HQ_DISTANCE)
                    .and_then(|address| read(memory, address))
            });
            row.hq_nearest_front = record.get::<i32>(HQ_NEAREST_FRONT).or_else(|| {
                root.pointer
                    .checked_add(HQ_NEAREST_FRONT)
                    .and_then(|address| read(memory, address))
            });
            row.border_war_presence = record
                .get::<u64>(BORDER_WAR)
                .or_else(|| {
                    root.pointer
                        .checked_add(BORDER_WAR)
                        .and_then(|address| read::<u64>(memory, address))
                })
                .map(|pointer| u8::from(pointer != 0));
            let leader = match record.get::<u64>(LEADER) {
                Some(pointer) => {
                    usize::try_from(pointer).map_err(|_| "HQ cooldown leader pointer overflow")
                }
                None => counter_tooltips::direct_leader(memory, root.pointer),
            };
            let observed = leader.and_then(|pointer| {
                counter_tooltips::authenticate_leader(memory, base, pointer, &mut resolver)
            });
            row.leader_enroute_days = counter_tooltips::leader_enroute_observed(
                memory,
                base,
                game_state,
                leader,
                observed,
                &mut resolver,
            )
            .ok();
            row.invasion_source_distance =
                record.get::<i32>(INVASION_SOURCE_DISTANCE).or_else(|| {
                    root.pointer
                        .checked_add(INVASION_SOURCE_DISTANCE)
                        .and_then(|address| read(memory, address))
                });
        }
    }
    outcome
}

pub fn count_roots(memory: &impl Memory, base: usize) -> Outcome {
    walk_roots_buffer(memory, base, engine_native_memory::walk::Buffer::Count)
}

pub fn walk_roots(memory: &impl Memory, base: usize, output: &mut Vec<Root>) -> Outcome {
    walk_roots_buffer(memory, base, engine_native_memory::walk::Buffer::Growing(output))
}

#[cfg(test)]
mod root_growth_tests;

pub fn walk_roots_buffer(
    memory: &impl Memory,
    base: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Root>,
) -> Outcome {
    let result = (|| {
        let table = engine_sites::at(base, registry::LOW_ROOTS_RVA)
            .and_then(|p| p.checked_add(REFERENCE_TYPE as usize * 8))
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("orders registry pointer unreadable or null")?;
        let header = table
            .checked_add(registry::ENTRIES)
            .and_then(|p| read::<[u8; 17]>(memory, p))
            .ok_or("orders registry header unreadable")?;
        let entries = usize::try_from(u64::from_le_bytes(header[0..8].try_into().unwrap()))
            .map_err(|_| "orders registry address overflow")?;
        let count = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let distance = header[16];
        let capacity = mask.checked_add(1).ok_or("orders registry mask overflow")?;
        if !capacity.is_power_of_two()
            || capacity > (1 << registry::MAXIMUM_EXPONENT)
            || count > capacity
            || entries == 0
            || distance == bucket::TERMINAL_DISTANCE
        {
            return Err("orders registry header invalid");
        }
        let static_empty = engine_sites::at(base, registry::STATIC_EMPTY_RVA)
            .ok_or("module address overflow")?;
        let exponent = capacity.trailing_zeros();
        if capacity == 1 {
            if entries != static_empty || count != 0 || distance != 0 {
                return Err("orders registry header invalid");
            }
        } else if entries == static_empty
            || !(registry::MINIMUM_EXPONENT..=registry::MAXIMUM_EXPONENT).contains(&exponent)
            || u32::from(distance) != exponent + 2
        {
            return Err("orders registry header invalid");
        }
        let terminal = capacity
            .checked_add(u32::from(distance))
            .ok_or("orders registry terminal overflow")?;
        let terminal_address = entries
            .checked_add(terminal as usize * bucket::STRIDE)
            .and_then(|p| p.checked_add(bucket::DISTANCE))
            .ok_or("orders registry address overflow")?;
        if read::<u8>(memory, terminal_address) != Some(bucket::TERMINAL_DISTANCE) {
            return Err("orders registry terminal sentinel invalid");
        }
        output.ensure(count as usize)?;
        let normal_vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let group_vtable = engine_sites::at(base, engine_native_parts::carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut copied = 0_usize;
        let mut occupied = 0_u32;
        let mut first = 0_u32;
        let mut bytes = [0_u8; 64 * bucket::STRIDE];
        while first < terminal {
            let slots = (terminal - first).min(64) as usize;
            let address = entries
                .checked_add(first as usize * bucket::STRIDE)
                .ok_or("orders registry address overflow")?;
            if !read_slice(memory, address, &mut bytes[..slots * bucket::STRIDE]) {
                return Err("orders registry buckets unreadable");
            }
            let mut pointers = [0_usize; 64];
            let mut handles = [0_u64; 64];
            let mut gathered = 0;
            let mut invalid = None;
            for entry in bytes[..slots * bucket::STRIDE].chunks_exact(bucket::STRIDE) {
                let probe = entry[bucket::DISTANCE];
                if probe == 0 {
                    continue;
                }
                if probe == bucket::TERMINAL_DISTANCE || u16::from(probe) > u16::from(distance) + 1
                {
                    invalid = Some("orders registry bucket distance invalid");
                    break;
                }
                occupied = occupied
                    .checked_add(1)
                    .ok_or("orders registry count overflow")?;
                let handle =
                    u64::from_le_bytes(entry[bucket::TYPE..bucket::OBJECT].try_into().unwrap());
                if handle as u32 != REFERENCE_TYPE {
                    invalid = Some("orders registry handle type mismatch");
                    break;
                }
                if copied + gathered == output.len() {
                    continue;
                }
                let Ok(pointer) = usize::try_from(u64::from_le_bytes(
                    entry[bucket::OBJECT..bucket::STRIDE].try_into().unwrap(),
                )) else {
                    invalid = Some("orders group address overflow");
                    break;
                };
                pointers[gathered] = pointer;
                handles[gathered] = handle;
                gathered += 1;
            }
            let mut records = [0_u8; 64 * ROOT_STRIDE];
            let completed = if gathered == 0 {
                0
            } else {
                memory.copy_records(
                    engine_native_memory::memory::RecordSources::Pointers(&pointers[..gathered]),
                    &ROOT_LAYOUT,
                    &mut records[..gathered * ROOT_STRIDE],
                )
            };
            if completed < gathered * ROOT_FIELDS.len() {
                engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::OrderRoots);
            }
            for index in 0..gathered {
                let (pointer, handle) = (pointers[index], handles[index]);
                let mut record = [0_u8; ROOT_STRIDE];
                let fields = if completed >= (index + 1) * ROOT_FIELDS.len() {
                    record.copy_from_slice(&records[index * ROOT_STRIDE..][..ROOT_STRIDE]);
                    ROOT_FIELDS.len()
                } else {
                    memory.copy_records(
                        engine_native_memory::memory::RecordSources::Pointers(&[pointer]),
                        &ROOT_LAYOUT,
                        &mut record,
                    )
                };
                if fields == 0 {
                    return Err("orders group identity unreadable");
                }
                let vtable = u64::from_le_bytes(record[..8].try_into().unwrap());
                if ![normal_vtable, group_vtable].contains(&vtable) {
                    return Err("orders group vtable mismatch");
                }
                if u64::from_le_bytes(record[8..16].try_into().unwrap()) != handle {
                    return Err("orders group registry handle mismatch");
                }
                if fields < ROOT_FIELDS.len() {
                    return Err("orders group country unreadable");
                }
                let country = u32::from_le_bytes(record[16..].try_into().unwrap());
                output[copied] = Root {
                    pointer,
                    handle,
                    country,
                };
                copied += 1;
            }
            if let Some(reason) = invalid {
                return Err(reason);
            }
            first += slots as u32;
        }
        if occupied != count {
            return Err("orders registry occupancy mismatch");
        }
        output[..copied].sort_unstable_by_key(|row| row.handle);
        if output[..copied]
            .windows(2)
            .any(|rows| rows[0].handle == rows[1].handle)
        {
            return Err("duplicate orders group handle");
        }
        Ok(if occupied as usize == copied {
            Outcome::Rows { len: copied }
        } else {
            Outcome::Truncated {
                len: copied,
                unread_rows: occupied - copied as u32,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GroupRow {
    pub handle: u64,
    pub country: u32,
    pub is_army_group: u8,
    pub motorization_level: u8,
    pub theatre: u64,
    pub theatre_ordinal: Option<u32>,
    pub theater_group: u64,
    pub parent: u64,
    pub leader: u64,
    pub hq_provider: u64,
    pub pointer: usize,
    pub name_start: usize,
    pub name_len: usize,
    pub name_known: bool,
}

pub fn copy_name_buffer(
    memory: &impl Memory,
    pointer: usize,
    mut arena: engine_native_memory::walk::Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), &'static str> {
    let address = pointer
        .checked_add(NAME)
        .ok_or("orders group name address overflow")?;
    let name = engine_native_memory::msvc_string::Descriptor::read(memory, address)?;
    *required = required
        .checked_add(name.len)
        .ok_or("orders group name demand overflow")?;
    let start = *used;
    let end = start
        .checked_add(name.len)
        .ok_or("orders group name span overflow")?;
    arena.ensure(end)?;
    name.copy(
        memory,
        arena
            .get_mut(start..end)
            .ok_or("orders group name arena too short")?,
    )?;
    *used = end;
    Ok((start, name.len))
}

#[cfg(test)]
mod command_name_tests {
    use super::*;
    #[test]
    fn owned_command_name_grows_to_the_live_heap_demand_without_truncating() {
        let mut bytes = vec![0; NAME + 64];
        let payload = vec![b'x'; 5000];
        let heap = bytes.len();
        bytes.extend_from_slice(&payload);
        bytes.push(0);
        bytes[NAME..NAME + 8].copy_from_slice(&(heap as u64).to_le_bytes());
        bytes[NAME + 16..NAME + 24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes[NAME + 24..NAME + 32].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let (mut used, mut required) = (0, 0);
        let mut arena = vec![0; 1];
        assert_eq!(
            copy_name_buffer(&memory, 0, engine_native_memory::walk::Buffer::Growing(&mut arena), &mut used, &mut required),
            Ok((0, 5000))
        );
        assert_eq!(&arena[..5000], payload);
        assert_eq!((used, required), (5000, 5000));
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub group: u64,
    pub ordinal: u32,
    pub unit: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChildRow {
    pub group: u64,
    pub ordinal: u32,
    pub child: u64,
}

pub struct OrderOfBattle<'a> {
    pub groups: &'a mut Vec<GroupRow>,
    pub members: &'a mut Vec<MemberRow>,
    pub children: &'a mut Vec<ChildRow>,
    pub roster_scratch: &'a mut Vec<RosterEntry>,
}

pub struct OrderOfBattleStorage<'a> {
    pub groups: engine_native_memory::walk::Buffer<'a, GroupRow>,
    pub members: engine_native_memory::walk::Buffer<'a, MemberRow>,
    pub children: engine_native_memory::walk::Buffer<'a, ChildRow>,
    pub roster_scratch: engine_native_memory::walk::Buffer<'a, RosterEntry>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RosterEntry {
    pointer: usize,
    theatre: usize,
    root: usize,
    is_army_group: bool,
    ordinal: Option<u32>,
    walked: bool,
    head: u32,
    next: u32,
    walked_before: bool,
}

pub fn captured_theatre_rosters(
    groups: &[GroupRow],
    scratch: &[RosterEntry],
    theatre: usize,
    handle: u64,
) -> Result<bool, &'static str> {
    if theatre == 0 || handle as u32 != ctheatre::REFERENCE_TYPE || handle >> 32 == 0 {
        return Err("reserved land theatre proof identity invalid");
    }
    if groups.len() != scratch.len()
        || groups.iter().any(|row| row.handle as u32 != REFERENCE_TYPE)
        || groups
            .windows(2)
            .any(|rows| rows[0].handle >= rows[1].handle)
    {
        return Err("reserved land OOB proof census invalid");
    }
    let mut found = false;
    for entry in scratch {
        let row = groups
            .get(entry.root)
            .ok_or("reserved land OOB proof root invalid")?;
        let same_pointer = entry.theatre == theatre;
        let same_handle = row.theatre == handle;
        if same_pointer != same_handle {
            return Err("reserved land OOB theatre pointer or handle mismatch");
        }
        if same_pointer {
            if entry.is_army_group != (row.is_army_group != 0)
                || entry.ordinal != row.theatre_ordinal
            {
                return Err("reserved land OOB subtype or ordinal mismatch");
            }
            found = true;
        }
    }
    Ok(found)
}

const END: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderOfBattleWalk {
    pub groups: Outcome,
    pub members: Outcome,
    pub children: Outcome,
}

#[derive(Default, Clone, Copy)]
struct Count {
    len: usize,
    unread: u32,
}

impl Count {
    fn push<T>(&mut self, output: &mut [T], row: T) -> Result<(), &'static str> {
        if let Some(destination) = output.get_mut(self.len) {
            *destination = row;
            self.len += 1;
        } else {
            self.unread = self
                .unread
                .checked_add(1)
                .ok_or("orders group row count overflow")?;
        }
        Ok(())
    }

    fn outcome(self, failure: Option<&'static str>) -> Outcome {
        match failure {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.unread == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            },
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn walk_order_of_battle(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    output: &mut OrderOfBattle<'_>,
) -> OrderOfBattleWalk {
    walk_order_of_battle_buffers(
        memory,
        base,
        roots,
        &mut OrderOfBattleStorage {
            groups: engine_native_memory::walk::Buffer::Growing(&mut *output.groups),
            members: engine_native_memory::walk::Buffer::Growing(&mut *output.members),
            children: engine_native_memory::walk::Buffer::Growing(&mut *output.children),
            roster_scratch: engine_native_memory::walk::Buffer::Growing(&mut *output.roster_scratch),
        },
    )
}

pub fn walk_order_of_battle_buffers(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    output: &mut OrderOfBattleStorage<'_>,
) -> OrderOfBattleWalk {
    let mut counts = [Count::default(); 3];
    let reader = Reader { memory, base };
    let failure = (|| {
        output.groups.ensure(roots.len())?;
        output.roster_scratch.ensure(roots.len())?;
        let scratch = output
            .roster_scratch
            .get_mut(..roots.len())
            .ok_or("orders group roster scratch capacity exhausted")?;
        for (index, root) in roots.iter().enumerate() {
            let (identity, theatre) = reader.identity(root)?;
            scratch[index] = RosterEntry {
                pointer: root.pointer,
                theatre,
                root: index,
                is_army_group: identity.is_army_group != 0,
                ordinal: None,
                walked: false,
                head: END,
                next: END,
                walked_before: false,
            };
            counts[0].push(&mut output.groups, identity)?;
            let members = reader
                .array(root.pointer, MEMBER_DATA)
                .ok_or("orders group member vector unreadable or invalid")?;
            output.members.ensure(
                counts[1]
                    .len
                    .checked_add(members.count as usize)
                    .ok_or("orders group member demand overflow")?,
            )?;
            for ordinal in 0..members.count {
                let member = reader.pointer(members, ordinal)?;
                let unit = reader.member_unit(member, root.pointer)?;
                counts[1].push(
                    &mut output.members,
                    MemberRow {
                        group: root.handle,
                        ordinal,
                        unit,
                    },
                )?;
            }
            if identity.is_army_group == 0 {
                continue;
            }
            let children = reader
                .array(root.pointer, carmy_group::MEMBER_DATA)
                .ok_or("army group child vector unreadable or invalid")?;
            output.children.ensure(
                counts[2]
                    .len
                    .checked_add(children.count as usize)
                    .ok_or("army group child demand overflow")?,
            )?;
            for ordinal in 0..children.count {
                let child = reader.pointer(children, ordinal)?;
                let handle = reader.group_handle(child)?;
                counts[2].push(
                    &mut output.children,
                    ChildRow {
                        group: root.handle,
                        ordinal,
                        child: handle,
                    },
                )?;
            }
        }
        theatre_ordinals(&reader, scratch, &mut output.groups)
    })()
    .err();
    OrderOfBattleWalk {
        groups: counts[0].outcome(failure),
        members: counts[1].outcome(failure),
        children: counts[2].outcome(failure),
    }
}

fn theatre_ordinals<M: Memory>(
    reader: &Reader<'_, M>,
    scratch: &mut [RosterEntry],
    groups: &mut [GroupRow],
) -> Result<(), &'static str> {
    if !chain_by_pointer(scratch) {
        return theatre_ordinals_sorted(reader, scratch, groups);
    }
    for index in 0..scratch.len() {
        let theatre = scratch[index].theatre;
        if theatre == 0 || scratch[index].ordinal.is_some() || scratch[index].walked {
            continue;
        }
        scratch[index].walked = true;
        if let Err(reason) = walk_theatre(reader, scratch, groups, theatre, find_chained) {
            return first_sorted_failure(reader, scratch, groups, index, theatre, reason);
        }
    }
    Ok(())
}

fn theatre_ordinals_sorted<M: Memory>(
    reader: &Reader<'_, M>,
    scratch: &mut [RosterEntry],
    groups: &mut [GroupRow],
) -> Result<(), &'static str> {
    scratch.sort_unstable_by_key(|entry| entry.pointer);
    for index in 0..scratch.len() {
        let theatre = scratch[index].theatre;
        if theatre == 0 || scratch[index].ordinal.is_some() || scratch[index].walked {
            continue;
        }
        scratch[index].walked = true;
        walk_theatre(reader, scratch, groups, theatre, find_sorted)?;
    }
    Ok(())
}

fn first_sorted_failure<M: Memory>(
    reader: &Reader<'_, M>,
    scratch: &mut [RosterEntry],
    groups: &mut [GroupRow],
    failed_at: usize,
    failed: usize,
    reason: &'static str,
) -> Result<(), &'static str> {
    scratch.sort_unstable_by_key(|entry| (entry.theatre, entry.root));
    let mut run = 0;
    while run < scratch.len() {
        let theatre = scratch[run].theatre;
        let end = run
            + scratch[run..]
                .iter()
                .take_while(|entry| entry.theatre == theatre)
                .count();
        let walked_before = theatre != 0 && scratch[run].root < failed_at;
        for entry in &mut scratch[run..end] {
            entry.walked_before = walked_before;
            entry.walked = false;
            if theatre == failed {
                entry.ordinal = None;
            }
        }
        run = end;
    }
    scratch.sort_unstable_by_key(|entry| entry.pointer);
    for index in 0..scratch.len() {
        let theatre = scratch[index].theatre;
        if theatre == 0 || scratch[index].ordinal.is_some() || scratch[index].walked {
            continue;
        }
        scratch[index].walked = true;
        if theatre == failed {
            return Err(reason);
        }
        if !scratch[index].walked_before {
            walk_theatre(reader, scratch, groups, theatre, find_sorted)?;
        }
    }
    Err(reason)
}

fn walk_theatre<M: Memory>(
    reader: &Reader<'_, M>,
    scratch: &mut [RosterEntry],
    groups: &mut [GroupRow],
    theatre: usize,
    find: fn(&[RosterEntry], usize) -> Result<usize, &'static str>,
) -> Result<(), &'static str> {
    for (offset, army_group) in [
        (ctheatre::ORDERS_GROUP_DATA, false),
        (ctheatre::ARMY_GROUP_DATA, true),
    ] {
        let members = reader
            .array(theatre, offset)
            .ok_or("theatre orders group roster unreadable or invalid")?;
        for ordinal in 0..members.count {
            let entry = &mut scratch[find(scratch, reader.pointer(members, ordinal)?)?];
            if entry.theatre != theatre {
                return Err("theatre roster group backlink mismatch");
            }
            if entry.is_army_group != army_group {
                return Err("theatre roster group subtype mismatch");
            }
            match entry.ordinal {
                Some(seen) if seen == ordinal => continue,
                Some(_) => return Err("duplicate group in theatre roster"),
                None => entry.ordinal = Some(ordinal),
            }
            if let Some(row) = groups.get_mut(entry.root) {
                row.theatre_ordinal = Some(ordinal);
            }
        }
    }
    Ok(())
}

fn find_sorted(scratch: &[RosterEntry], pointer: usize) -> Result<usize, &'static str> {
    scratch
        .binary_search_by_key(&pointer, |entry| entry.pointer)
        .map_err(|_| "theatre roster group absent from census")
}

fn chain_slot(pointer: usize, buckets: usize) -> usize {
    let mut hash = pointer as u64;
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash as usize & (buckets - 1)
}

fn chain_buckets(len: usize) -> usize {
    1 << (usize::BITS - 1 - len.leading_zeros())
}

fn chain_by_pointer(scratch: &mut [RosterEntry]) -> bool {
    if scratch.is_empty() {
        return true;
    }
    if scratch.len() >= END as usize {
        return false;
    }
    let buckets = chain_buckets(scratch.len());
    for entry in scratch.iter_mut() {
        entry.head = END;
    }
    for index in 0..scratch.len() {
        let pointer = scratch[index].pointer;
        let slot = chain_slot(pointer, buckets);
        let mut at = scratch[slot].head;
        while at != END {
            if scratch[at as usize].pointer == pointer {
                return false;
            }
            at = scratch[at as usize].next;
        }
        scratch[index].next = scratch[slot].head;
        scratch[slot].head = index as u32;
    }
    true
}

fn find_chained(scratch: &[RosterEntry], pointer: usize) -> Result<usize, &'static str> {
    let mut at = scratch[chain_slot(pointer, chain_buckets(scratch.len()))].head;
    while at != END {
        let entry = &scratch[at as usize];
        if entry.pointer == pointer {
            return Ok(at as usize);
        }
        at = entry.next;
    }
    Err("theatre roster group absent from census")
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
}

impl<M: Memory> Reader<'_, M> {
    fn at(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        pointer
            .checked_add(offset)
            .ok_or("orders group address overflow")
    }

    fn field<T: engine_native_memory::memory::Raw>(
        &self,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        read(self.memory, self.at(pointer, offset)?).ok_or("orders group field unreadable")
    }

    fn array(&self, pointer: usize, offset: usize) -> Option<Array> {
        Array::read(self.memory, pointer.checked_add(offset)?, 8)
    }

    fn pointer(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|address| read::<u64>(self.memory, address))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("orders group vector entry unreadable or null")
    }

    fn optional(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "orders group pointer overflow")
    }

    fn identity(&self, root: &Root) -> Result<(GroupRow, usize), &'static str> {
        let is_army_group = self.field::<u8>(root.pointer, IS_ARMY_GROUP)?;
        let motorization_level = self.field::<u8>(root.pointer, MOTORIZATION_LEVEL)?;
        let theatre_pointer = self.optional(root.pointer, THEATRE)?;
        let theatre = self.reference(
            theatre_pointer,
            ctheatre::VTABLE_RVA,
            ctheatre::REFERENCE_TYPE,
        )?;
        let theater_group = self.reference(
            self.optional(root.pointer, THEATER_GROUP)?,
            ctheater_group::VTABLE_RVA,
            ctheater_group::REFERENCE_TYPE,
        )?;
        let parent_pointer = self.optional(root.pointer, PARENT)?;
        let parent = if parent_pointer == 0 {
            0
        } else {
            if self.field::<u8>(parent_pointer, IS_ARMY_GROUP)? == 0 {
                return Err("orders group parent is not an army group");
            }
            self.reference(parent_pointer, carmy_group::VTABLE_RVA, REFERENCE_TYPE)?
        };
        let leader_pointer = self.optional(root.pointer, LEADER)?;
        let leader = if leader_pointer == 0 {
            0
        } else {
            cunit_leader::handle(self.memory, self.base, leader_pointer)?
        };
        let provider = self.optional(root.pointer, HQ_PROVIDER)?;
        let hq_provider = if provider == 0 {
            0
        } else {
            self.member_unit(provider, root.pointer)?
        };
        Ok((
            GroupRow {
                handle: root.handle,
                country: root.country,
                is_army_group,
                motorization_level,
                theatre,
                theatre_ordinal: None,
                theater_group,
                parent,
                leader,
                hq_provider,
                pointer: root.pointer,
                ..Default::default()
            },
            theatre_pointer,
        ))
    }

    fn group_handle(&self, pointer: usize) -> Result<u64, &'static str> {
        let identity =
            read::<[u8; 16]>(self.memory, pointer).ok_or("orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = engine_sites::at(self.base, VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let group = engine_sites::at(self.base, carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if vtable != normal && vtable != group {
            return Err("orders group vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != REFERENCE_TYPE {
            return Err("orders group reference type mismatch");
        }
        Ok(handle)
    }

    fn reference(
        &self,
        pointer: usize,
        vtable: Rva,
        reference_type: u32,
    ) -> Result<u64, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let identity = read::<[u8; 16]>(self.memory, pointer)
            .ok_or("orders group referenced identity unreadable")?;
        let expected =
            engine_sites::at(self.base, vtable).ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected {
            return Err("orders group referenced object vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != reference_type {
            return Err("orders group referenced object type mismatch");
        }
        Ok(handle)
    }

    fn member_unit(&self, member: usize, group: usize) -> Result<u64, &'static str> {
        let unit = member
            .checked_sub(corders_group_member::HOST_OFFSET)
            .ok_or("orders group member is below its embedded base offset")?;
        let header = read::<[u8; 32]>(self.memory, unit)
            .ok_or("orders group member host identity unreadable")?;
        let primary = engine_sites::at(self.base, carmy::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let secondary = engine_sites::at(self.base, carmy::SECONDARY_VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != primary {
            return Err("orders group member host primary vtable mismatch");
        }
        if u32::from_le_bytes(header[8..12].try_into().unwrap()) != 0 {
            return Err("orders group member host is not a class-zero army");
        }
        if u64::from_le_bytes(
            header[carmy::SECONDARY_BASE..carmy::SECONDARY_BASE + 8]
                .try_into()
                .unwrap(),
        ) != secondary
        {
            return Err("orders group member host secondary vtable mismatch");
        }
        let handle =
            u64::from_le_bytes(header[cunit::HANDLE..cunit::HANDLE + 8].try_into().unwrap());
        if handle == 0 {
            return Err("orders group member host has a null handle");
        }
        if self.field::<u64>(member, corders_group_member::GROUP)? != group as u64 {
            return Err("orders group member does not point back at its group");
        }
        Ok(handle)
    }
}

#[cfg(test)]
mod order_of_battle_tests;
