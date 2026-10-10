//! Type-53 army/order container.

use crate::{
    Memory, carmy, carmy_group, corders_group_member,
    cpdx_array::Array,
    creference_registry as registry, ctheater_group, ctheatre, cunit, cunit_leader,
    memory::{
        Gathered, RecordField, RecordLayout, gather_records, packed_fields, packed_map,
        packed_stride,
    },
    read, read_slice, reference_bucket as bucket,
    walk::Outcome,
};

/// COrdersGroup vtable.
pub const VTABLE_RVA: usize = 0x0293_bc60;
/// COrdersGroup reference type.
pub const REFERENCE_TYPE: u32 = 53;
/// COrdersGroup packed type/id u64 handle.
pub const HANDLE: usize = 0x08;
/// Army-group discriminator byte, initialized to zero or one and always serialized.
pub const IS_ARMY_GROUP: usize = 0x39;
/// Motorization-level byte, always serialized.
pub const MOTORIZATION_LEVEL: usize = 0x38;
/// Raw country reference resolved before allocating order IDs.
pub const COUNTRY: usize = 0x3c;
/// Member-vector data containing 8-byte embedded membership pointers.
pub const MEMBER_DATA: usize = 0x50;
/// Member u32 capacity between data pointer and signed live count in the
/// array header; no individual writer identifies it.
pub const MEMBER_CAPACITY: usize = 0x58;
/// Signed i32 member count; only positive counts enter the loop.
pub const MEMBER_COUNT: usize = 0x5c;
/// HQ-provider embedded membership pointer, serialized as the leader unit.
pub const HQ_PROVIDER: usize = 0x68;
/// Army-leader pointer; serialization uses its identity subobject when present.
pub const LEADER: usize = 0x88;
/// COrdersGroup primary order pointer-vector data.
pub const PRIMARY_ORDER_DATA: usize = 0x98;
/// COrdersGroup primary order u32 count.
pub const PRIMARY_ORDER_COUNT: usize = 0xa4;
/// COrdersGroup secondary order pointer-vector data.
pub const SECONDARY_ORDER_DATA: usize = 0xb0;
/// COrdersGroup secondary order u32 count.
pub const SECONDARY_ORDER_COUNT: usize = 0xbc;
/// COrdersGroup tertiary order pointer-vector data.
pub const TERTIARY_ORDER_DATA: usize = 0xc8;
/// COrdersGroup tertiary order u32 count.
pub const TERTIARY_ORDER_COUNT: usize = 0xd4;
/// Four raw colour components copied for arrow presentation.
pub const COLOUR: usize = 0x110;
/// The group's stored name is free text when renamed by a player and already
/// localised when generated at creation. Generation substitutes the smallest
/// unused sequence number into a loaded name format and retains only the result.
/// No key or sequence number survives here; serialization writes the same text.
pub const NAME: usize = 0x160;
/// COrdersGroup CTheaterGroup pointer.
pub const THEATER_GROUP: usize = 0x180;
/// Training byte; hourly writer coverage unresolved.
pub const TRAINING: usize = 0x19d;
/// COrdersGroup stop-at-max training byte.
pub const STOP_AT_MAXIMUM: usize = 0x1a0;
/// Execution enum, u32: zero careful, one balanced, two aggressive.
pub const EXECUTION_TYPE: usize = 0x1a4;
/// Cohesion enum, u32: zero flexible, one balanced, two rigid, three immobile.
pub const COHESION_TYPE: usize = 0x1a8;
/// Leader-proximity raw value, u32; meaning beyond the enum is unresolved.
pub const LEADER_PROXIMITY: usize = 0x1ac;
/// COrdersGroup border-war 8-byte presence operand.
pub const BORDER_WAR: usize = 0x1b0;
/// Withdrawing byte, serialized and used when withdrawing the general
/// and selecting the HQ leader-en-route flag.
pub const WITHDRAWING: usize = 0x19b;
/// Stored signed HQ distance read directly by the counter tooltip.
/// Construction initializes it to -1, and serialization emits it only when
/// nonnegative. Preserve other negative values; omission proves negativity
/// rather than a particular sentinel. Invasion-source distance is separate.
pub const HQ_DISTANCE: usize = 0x1c4;
/// Signed distance selected for a kind-3 primary HQ root. Serialization
/// emits it only when nonnegative; omission proves negativity rather than
/// a particular sentinel.
pub const INVASION_SOURCE_DISTANCE: usize = 0x1cc;
/// Parent army-group pointer, separate from a vector or child-owned orders.
pub const PARENT: usize = 0x1b8;
/// COrdersGroup parent CTheatre pointer.
pub const THEATRE: usize = 0x40;
/// COrdersGroup allocation size.
pub const SIZE: usize = 0x230;
/// Group constructor initializing the non-group discriminator.
pub const CONSTRUCTOR_RVA: usize = 0x00bd_a700;
/// Order-execution setter applying a boolean to roots and children.
pub const SET_ORDER_EXECUTION_RVA: usize = 0x0184_1140;
/// Theatre-group setter, distinct from the theatre pointer.
pub const SET_THEATER_GROUP_RVA: usize = 0x00be_6db0;
/// Single-HQ query using the provider accessor when an army leader exists.
pub const SINGLE_HQ_UNIT_RVA: usize = 0x00bd_fd90;
/// Group serializer emitting members, leader unit, leader and group properties.
pub const SERIALIZE_RVA: usize = 0x00be_8100;
/// HQ-provider clear guarded by the embedded membership pointer.
pub const CLEAR_HQ_PROVIDER_RVA: usize = 0x00be_4970;

mod counter_tooltips;

/// An authenticated type-53 registry root. Fields are copied, never engine pointers in the replay.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Root {
    pub(crate) pointer: usize,
    pub handle: u64,
    pub country: u32,
}

impl Root {
    /// The authenticated group's address, for a raw copy of the same
    /// acquisition pass that authenticated it; never a published field.
    pub fn pointer(self) -> usize {
        self.pointer
    }
}

/// One group's colour and ordered root-order lists; the lists are ranges of
/// the caller's instance arena.
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
    /// Byte span of the group's name in the caller's name arena.
    pub name_start: usize,
    pub name_len: usize,
}

pub(crate) fn plan_pointer_handle(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u64, &'static str> {
    gathered_pointer_handle(memory, base, pointer, Gathered::NONE)
}

/// [`plan_pointer_handle`] taking what `record` gathered from `pointer`.
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
        .any(|rva| base.checked_add(rva).map(|v| v as u64) == Some(vtable))
    {
        return Err("plan group vtable mismatch");
    }
    let handle = crate::corder_instance::gathered_field::<u64>(memory, pointer, HANDLE, record)?;
    if handle as u32 != REFERENCE_TYPE {
        return Err("plan group reference type mismatch");
    }
    Ok(handle)
}

pub(crate) fn plan_identity(
    memory: &impl Memory,
    base: usize,
    root: Root,
) -> Result<(), &'static str> {
    plan_identity_gathered(memory, base, root, Gathered::NONE)
}

/// [`plan_identity`] taking what `record` gathered from the root.
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
    /// Instance ids written to the arena.
    pub orders: usize,
    /// Name bytes written to the name arena.
    pub names: usize,
}

/// The COrdersGroup spans [`walk_plans`] reads, as `(offset, len)`: identity
/// through country, the primary and fallback root headers, colour and name.
/// They are hinted a batch ahead and gathered in one record per group. Each
/// is a width the guarded record copy moves inline (64, 16 or 32 bytes); the
/// eight bytes between the two root headers are not copied.
const PLAN_SPANS: [(usize, usize); 5] = [
    (0, COUNTRY + 4),
    (PRIMARY_ORDER_DATA, 16),
    (SECONDARY_ORDER_DATA, 16),
    (COLOUR, 16),
    (NAME, crate::msvc_string::STRIDE),
];
const PLAN_FIELDS: [RecordField; 5] = packed_fields(PLAN_SPANS);
const PLAN_STRIDE: usize = packed_stride(PLAN_SPANS);
const PLAN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PLAN_FIELDS, PLAN_STRIDE).expect("valid group plan layout");
const PLAN_MAP: [u16; NAME + crate::msvc_string::STRIDE] = packed_map(PLAN_SPANS);

/// Hints one census group's spans; advisory only.
fn prefetch_plan(memory: &impl Memory, root: &Root) {
    for (offset, len) in PLAN_SPANS {
        if let Some(address) = root.pointer.checked_add(offset) {
            memory.prefetch(address, len);
        }
    }
}

/// Hints what a gathered group record points at: its heap name. Hints read
/// nothing. The root arrays are not hinted: in the paired capture-cost bench
/// that cost more than it saved.
fn prefetch_dependents(memory: &impl Memory, record: Gathered<'_>) {
    if let Some(header) = record.get::<[u8; crate::msvc_string::STRIDE]>(NAME)
        && let Some((address, len)) = crate::msvc_string::heap_span(&header)
    {
        memory.prefetch(address, len);
    }
}

/// Four colour components retain their raw bit patterns, including NaNs and
/// signed zero. Member orders resolve from primary and fallback root arrays
/// in array order, so both retain ordered authenticated ArmyOrders instance IDs.
///
/// Groups are read in batches: one guarded record per group gathers every fixed
/// span before decoding. An incomplete record falls back to its original
/// per-field reads, preserving rows and first failure.
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
        // Hints only, as in the order walk: they overlap group misses and
        // leave every value and its failure precedence unchanged. A batch's
        // own spans are hinted two batches ahead; it is gathered, and its
        // heap names hinted, one batch before it is decoded.
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
                    .get::<[u8; crate::msvc_string::STRIDE]>(NAME)
                {
                    Some(header) => crate::msvc_string::append_growing_with_header(
                        memory,
                        &header,
                        names,
                        &mut names_used,
                    )?,
                    None => {
                        crate::msvc_string::append_growing(memory, name, names, &mut names_used)?
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
                    // Up to 64 slots per guarded copy. If a copy fails, its
                    // slots are read one by one as before, so the first
                    // unreadable slot still fails in array order.
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

/// Callback-local observation from the original counter loop. Pointer and
/// authenticated handle failures are retained; no consumer retries group+88.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectCommander {
    pub group: u64,
    pub pointer: Result<usize, &'static str>,
    pub leader: Result<Option<u64>, &'static str>,
    pub enroute_days: Result<i32, &'static str>,
}
impl Default for DirectCommander {
    fn default() -> Self {
        Self {
            group: 0,
            pointer: Err("group commander not observed"),
            leader: Err("group commander not observed"),
            enroute_days: Err("group commander not observed"),
        }
    }
}

/// Orders-group presentation byte, keyed by the group's full handle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CounterInputsRow {
    pub group: u64,
    pub withdrawing: u8,
    /// [`HQ_DISTANCE`]; None is this field's isolated guarded-read failure.
    pub hq_distance: Option<i32>,
    /// Passive modifier-cooldown-days operands, independent of distance.
    pub leader_enroute_days: Option<i32>,
    /// Presence of the raw border-war pointer; the pointee is never followed.
    pub border_war_presence: Option<u8>,
    /// Distinct from +1C4; unknown negative values remain exact signed words.
    pub invasion_source_distance: Option<i32>,
}

/// Reads each root of a complete registry census after re-authenticating its identity.
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
    counter_inputs(memory, base, game_state, roots, source, output, None)
}

/// Shares each original +88 selecting occurrence and army-leader identity
/// with independent commander inputs. Controls must cover the admitted roots.
pub fn walk_counter_inputs_with_commanders(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roots: &[Root],
    source: Outcome,
    output: &mut [CounterInputsRow],
    commanders: &mut [DirectCommander],
) -> Outcome {
    counter_inputs(
        memory,
        base,
        game_state,
        roots,
        source,
        output,
        Some(commanders),
    )
}

fn counter_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roots: &[Root],
    source: Outcome,
    output: &mut [CounterInputsRow],
    mut commanders: Option<&mut [DirectCommander]>,
) -> Outcome {
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
        for (root, row) in roots.iter().zip(output.iter_mut()) {
            plan_identity(memory, base, *root)?;
            let withdrawing =
                crate::corder_instance::plan_field::<u8>(memory, root.pointer, WITHDRAWING)?;
            *row = CounterInputsRow {
                group: root.handle,
                withdrawing,
                hq_distance: None,
                leader_enroute_days: None,
                border_war_presence: None,
                invasion_source_distance: None,
            };
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
    // Every established root identity/withdrawing byte is copied before the
    // optional tooltip suffix. A failure here does not truncate those roots.
    // Independent group controls use the authenticated registry census even
    // if a primary withdrawing read failed; their selecting read is attempted
    // once per current root and is never retried by a later consumer.
    if let Some(controls) = commanders.as_deref_mut() {
        for (index, root) in roots.iter().take(source.len()).enumerate() {
            if index >= controls.len() {
                break;
            }
            let pointer = counter_tooltips::direct_leader(memory, root.pointer);
            let prior = controls[..index]
                .iter()
                .find(|row| row.pointer == pointer && pointer.is_ok());
            let (leader, enroute_days) = if let Some(prior) = prior {
                (prior.leader, prior.enroute_days)
            } else {
                let leader = pointer.and_then(|pointer| {
                    counter_tooltips::authenticate_leader(memory, base, pointer)
                });
                let enroute_days = counter_tooltips::leader_enroute_observed(
                    memory, base, game_state, pointer, leader,
                );
                (leader, enroute_days)
            };
            controls[index] = DirectCommander {
                group: root.handle,
                pointer,
                leader,
                enroute_days,
            };
        }
    }
    for (index, (root, row)) in roots
        .iter()
        .zip(output.iter_mut())
        .take(outcome.len())
        .enumerate()
    {
        row.hq_distance = root
            .pointer
            .checked_add(HQ_DISTANCE)
            .and_then(|address| read(memory, address));
        row.border_war_presence = root
            .pointer
            .checked_add(BORDER_WAR)
            .and_then(|address| read::<u64>(memory, address))
            .map(|pointer| u8::from(pointer != 0));
        row.leader_enroute_days = match commanders.as_deref() {
            Some(controls) => controls
                .get(index)
                .and_then(|control| control.enroute_days.ok()),
            None => {
                counter_tooltips::leader_enroute_days(memory, base, game_state, root.pointer).ok()
            }
        };
        row.invasion_source_distance = root
            .pointer
            .checked_add(INVASION_SOURCE_DISTANCE)
            .and_then(|address| read(memory, address));
    }
    outcome
}

/// Reads the complete native registry, including army-group roots exactly once.
/// Registry growth is permitted only outside the accepted hourly callback.
/// The caller owns storage; a capacity miss counts every omitted registry entry.
pub fn walk_roots(memory: &impl Memory, base: usize, output: &mut [Root]) -> Outcome {
    walk_roots_buffer(memory, base, crate::walk::Buffer::Fixed(output))
}

/// Worker storage follows the original registry count before any occupied
/// bucket is decoded. Fixed callback storage keeps its verdict.
pub fn walk_roots_growing(memory: &impl Memory, base: usize, output: &mut Vec<Root>) -> Outcome {
    walk_roots_buffer(memory, base, crate::walk::Buffer::Growing(output))
}

#[cfg(test)]
mod root_growth_tests;

pub fn walk_roots_buffer(
    memory: &impl Memory,
    base: usize,
    mut output: crate::walk::Buffer<'_, Root>,
) -> Outcome {
    let result = (|| {
        let table = base
            .checked_add(registry::LOW_ROOTS_RVA)
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
        let static_empty = base
            .checked_add(registry::STATIC_EMPTY_RVA)
            .ok_or("module address overflow")?;
        // Construction uses the static-empty header. Growth uses 2^exponent buckets
        // and exponent+2 overflow cells.
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
        let normal_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let group_vtable = base
            .checked_add(crate::carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut copied = 0_usize;
        let mut occupied = 0_u32;
        let mut first = 0_u32;
        // Batching contiguous buckets amortizes SEH without reading beyond the
        // native terminal or retaining a second registry-sized allocation.
        let mut bytes = [0_u8; 64 * bucket::STRIDE];
        while first < terminal {
            let slots = (terminal - first).min(64) as usize;
            let address = entries
                .checked_add(first as usize * bucket::STRIDE)
                .ok_or("orders registry address overflow")?;
            if !read_slice(memory, address, &mut bytes[..slots * bucket::STRIDE]) {
                return Err("orders registry buckets unreadable");
            }
            for entry in bytes[..slots * bucket::STRIDE].chunks_exact(bucket::STRIDE) {
                let probe = entry[bucket::DISTANCE];
                if probe == 0 {
                    continue;
                }
                if probe == bucket::TERMINAL_DISTANCE || u16::from(probe) > u16::from(distance) + 1
                {
                    return Err("orders registry bucket distance invalid");
                }
                occupied = occupied
                    .checked_add(1)
                    .ok_or("orders registry count overflow")?;
                let handle =
                    u64::from_le_bytes(entry[bucket::TYPE..bucket::OBJECT].try_into().unwrap());
                if handle as u32 != REFERENCE_TYPE {
                    return Err("orders registry handle type mismatch");
                }
                if copied == output.len() {
                    continue;
                }
                let pointer = usize::try_from(u64::from_le_bytes(
                    entry[bucket::OBJECT..bucket::STRIDE].try_into().unwrap(),
                ))
                .map_err(|_| "orders group address overflow")?;
                let identity =
                    read::<[u8; 16]>(memory, pointer).ok_or("orders group identity unreadable")?;
                let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
                if ![normal_vtable, group_vtable].contains(&vtable) {
                    return Err("orders group vtable mismatch");
                }
                if u64::from_le_bytes(identity[8..].try_into().unwrap()) != handle {
                    return Err("orders group registry handle mismatch");
                }
                let country = pointer
                    .checked_add(COUNTRY)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("orders group country unreadable")?;
                output[copied] = Root {
                    pointer,
                    handle,
                    country,
                };
                copied += 1;
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

/// One authenticated type-53 group's own identity. Engine pointers never leave
/// this crate: each reference becomes the pointee's native handle, and a null
/// native pointer stays zero rather than becoming a missing row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GroupRow {
    pub handle: u64,
    /// Raw country reference at +0x3c; aliases are preserved, never canonicalized.
    pub country: u32,
    /// Raw discriminator byte at +0x39, the save's field_marshal_group source.
    pub is_army_group: u8,
    /// Raw motorization byte, always serialized.
    pub motorization_level: u8,
    /// Full CTheatre reference, or zero for the native null parent pointer.
    pub theatre: u64,
    /// Native index in that theatre's +0x80 regular or +0x98 army-group
    /// roster, chosen by `is_army_group`. None is absence, not position zero.
    pub theatre_ordinal: Option<u32>,
    /// Full CTheaterGroup reference, or zero for the native null pointer.
    pub theater_group: u64,
    /// Full parent CArmyGroup reference, or zero when this group has no parent.
    pub parent: u64,
    /// Full CUnitLeader reference, or zero when no leader is assigned.
    pub leader: u64,
    /// Full CUnit reference of the HQ-providing unit, or zero when absent.
    pub hq_provider: u64,
    /// Callback/worker-only source pointer, never serialized.
    pub pointer: usize,
    pub name_start: usize,
    pub name_len: usize,
    pub name_known: bool,
}

/// Stored command name populated by generation or player rename and serialized
/// as text. Demand includes every readable descriptor even when the arena is short.
pub fn copy_name(
    memory: &impl Memory,
    pointer: usize,
    arena: &mut [u8],
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), &'static str> {
    copy_name_buffer(
        memory,
        pointer,
        crate::walk::Buffer::Fixed(arena),
        used,
        required,
    )
}

pub fn copy_name_buffer(
    memory: &impl Memory,
    pointer: usize,
    mut arena: crate::walk::Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), &'static str> {
    let address = pointer
        .checked_add(NAME)
        .ok_or("orders group name address overflow")?;
    let name = crate::msvc_string::Descriptor::read(memory, address)?;
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
    fn owned_command_name_counts_live_heap_demand_without_truncating() {
        let mut bytes = vec![0; NAME + 64];
        let payload = vec![b'x'; 5000];
        let heap = bytes.len();
        bytes.extend_from_slice(&payload);
        bytes.push(0);
        bytes[NAME..NAME + 8].copy_from_slice(&(heap as u64).to_le_bytes());
        bytes[NAME + 16..NAME + 24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes[NAME + 24..NAME + 32].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        let memory = crate::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let (mut used, mut required) = (0, 0);
        assert_eq!(
            copy_name(&memory, 0, &mut [0; 1], &mut used, &mut required),
            Err("orders group name arena too short")
        );
        assert_eq!((used, required), (0, 5000));
        let mut arena = vec![0; required];
        required = 0;
        assert_eq!(
            copy_name(&memory, 0, &mut arena, &mut used, &mut required),
            Ok((0, 5000))
        );
        assert_eq!(arena, payload);
        assert_eq!((used, required), (5000, 5000));
    }
}

/// One entry of a group's own ordered member vector at +0x50. Native order is
/// state: the ordinal is the vector index, not a generated key.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub group: u64,
    pub ordinal: u32,
    /// Full CUnit reference recovered from the embedded COrdersGroupMember.
    pub unit: u64,
}

/// One entry of a CArmyGroup's ordered child vector at +0x230.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChildRow {
    pub group: u64,
    pub ordinal: u32,
    /// Full child COrdersGroup reference.
    pub child: u64,
}

/// Caller-owned row storage; the walker never allocates.
pub struct OrderOfBattle<'a> {
    pub groups: &'a mut [GroupRow],
    pub members: &'a mut [MemberRow],
    pub children: &'a mut [ChildRow],
    /// One entry per root; sized by the caller from the live root census.
    pub roster_scratch: &'a mut [RosterEntry],
}

/// Worker-owned OOB buffers grow from the complete census and original member headers.
pub struct OrderOfBattleStorage<'a> {
    pub groups: crate::walk::Buffer<'a, GroupRow>,
    pub members: crate::walk::Buffer<'a, MemberRow>,
    pub children: crate::walk::Buffer<'a, ChildRow>,
    pub roster_scratch: crate::walk::Buffer<'a, RosterEntry>,
}

/// Per-census scratch only; native pointers are never serialized. Kept in
/// root order, with each group reachable by pointer through chains threaded
/// through the entries themselves; see `theatre_ordinals`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RosterEntry {
    pointer: usize,
    theatre: usize,
    root: usize,
    is_army_group: bool,
    ordinal: Option<u32>,
    walked: bool,
    /// First entry of the chain of groups whose pointer hashes to this slot.
    head: u32,
    /// Next entry in this group's chain.
    next: u32,
    /// Only while resolving a failure: this entry's theatre was walked,
    /// successfully, before the failed one.
    walked_before: bool,
}

/// Reuse a successful complete OOB walk for one freshly authenticated theatre.
/// Both slices must be that walk's exact complete root prefix, with Groups Rows.
/// The walk has validated both native rosters, all their pointer occurrences,
/// backlinks, subtypes and ordinals. This helper reads no native memory.
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

/// Ends a [`RosterEntry`] chain.
const END: u32 = u32::MAX;

/// Independent outcomes: a full member buffer must not silently shorten the
/// group census, and an army group with unreadable children must not turn its
/// identity row into an invented empty membership.
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

/// Reads the order of battle for an already authenticated registry census.
///
/// `roots` must be the exact prefix the caller's registry walk filled; the
/// caller keeps that census's own outcome, because a truncated or unavailable
/// registry cannot establish a complete order of battle. Every reference is
/// authenticated by its pointee's vtable and reference type before it becomes a
/// row field. Members are resolved by subtracting the embedded
/// COrdersGroupMember offset and re-authenticating the CUnit host, including
/// its reciprocal +0xc0 back-pointer, so no unresolved pointer becomes a row.
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
            groups: crate::walk::Buffer::Fixed(output.groups),
            members: crate::walk::Buffer::Fixed(output.members),
            children: crate::walk::Buffer::Fixed(output.children),
            roster_scratch: crate::walk::Buffer::Fixed(output.roster_scratch),
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

/// Theatre simulation rosters visit army groups first, then each group's
/// roots. Serialization keeps both rosters separately in source order. Read
/// each roster once and validate against this root census.
/// Theatres follow first-seen root order, matching recorded staged spans;
/// resolve entries through scratch pointer chains without sorting. Success
/// and ordinals are independent of theatre order, but first failure is not.
/// `first_sorted_failure` restores pointer-ordered failure without rereading.
/// Repeated pointers or an extent too long to chain use the direct walk.
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
        // A root already placed was reached through its theatre's walk. Only a
        // root its theatre's rosters omit can see that theatre walked twice,
        // and the second walk finds every position already equal.
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

/// The original walk: the scratch sorted by group pointer, each theatre
/// walked when its first unplaced group in that order is reached, and each
/// roster entry found by binary search.
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

/// The failure the original pointer-ordered walk reports, after the root-order
/// walk failed at the theatre of root `failed_at`. Theatres whose first root
/// precedes it were walked without failing, and the original walk passes them
/// the same way; the failed theatre fails with the same reason, since a
/// theatre's walk depends only on its own rosters. So the original order is
/// replayed with those outcomes known, and only a theatre neither walk has
/// reached yet is read, for the first time.
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
                // The failed walk may have placed some of its groups; the
                // original walk has not walked this theatre yet.
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

/// Reads one theatre's two rosters in native order and places each listed
/// group, after checking it is a census root that names this theatre and has
/// the roster's subtype.
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

/// The chain slot of `pointer` among `buckets`, a power of two.
fn chain_slot(pointer: usize, buckets: usize) -> usize {
    let mut hash = pointer as u64;
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash as usize & (buckets - 1)
}

/// The largest power of two not above the census length: the chain count.
fn chain_buckets(len: usize) -> usize {
    1 << (usize::BITS - 1 - len.leading_zeros())
}

/// Threads a chain per pointer hash through the scratch, in place: entry
/// `h`'s `head` starts the chain of slot `h`, and each entry's `next`
/// continues its own. False, with nothing to rely on, when two roots share a
/// pointer or the census is too long for 32-bit links.
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

    fn field<T: crate::memory::Raw>(
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

    /// Optional engine pointer: zero preserves the native absence exactly.
    fn optional(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "orders group pointer overflow")
    }

    fn identity(&self, root: &Root) -> Result<(GroupRow, usize), &'static str> {
        // The census already authenticated the root's vtable, handle and
        // country; this reads only the fields it does not carry.
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

    /// Authenticates a group pointer that is not already a census root.
    fn group_handle(&self, pointer: usize) -> Result<u64, &'static str> {
        let identity =
            read::<[u8; 16]>(self.memory, pointer).ok_or("orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = self
            .base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let group = self
            .base
            .checked_add(carmy_group::VTABLE_RVA)
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
        vtable: usize,
        reference_type: u32,
    ) -> Result<u64, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let identity = read::<[u8; 16]>(self.memory, pointer)
            .ok_or("orders group referenced identity unreadable")?;
        let expected = self
            .base
            .checked_add(vtable)
            .ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected {
            return Err("orders group referenced object vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != reference_type {
            return Err("orders group referenced object type mismatch");
        }
        Ok(handle)
    }

    /// Recovers the CUnit that owns an embedded COrdersGroupMember. The engine
    /// reaches the unit through a virtual accessor the callback may not call,
    /// so the host is authenticated instead: both CArmy vtables, a nonzero
    /// handle and the reciprocal +0xc0 back-pointer must all agree.
    fn member_unit(&self, member: usize, group: usize) -> Result<u64, &'static str> {
        let unit = member
            .checked_sub(corders_group_member::HOST_OFFSET)
            .ok_or("orders group member is below its embedded base offset")?;
        // Vtable, class word, secondary vtable and handle are one exact span.
        let header = read::<[u8; 32]>(self.memory, unit)
            .ok_or("orders group member host identity unreadable")?;
        let primary = self
            .base
            .checked_add(carmy::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let secondary = self
            .base
            .checked_add(carmy::SECONDARY_VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != primary {
            return Err("orders group member host primary vtable mismatch");
        }
        // Army construction supplies class zero; formation-division counts select
        // only this class.
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
