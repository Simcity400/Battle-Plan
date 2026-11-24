
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{Raw, RecordField, RecordLayout, RecordSources, packed_fields},
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::{ccountry, cgame_state};
use crate::{
    cstrategic_operative as strategic,
    cstrategic_operative_manager as manager,
    operation_token_map,
};

pub const ENTRY_HASH: usize = 0x00;
pub const ENTRY_PROBE: usize = 0x04;
pub const ENTRY_COUNTRY: usize = 0x08;
pub const TRADE_STRIDE: usize = 0x18;
pub const TRADE_VALUE: usize = 0x10;
pub const PRESSURE_STRIDE: usize = 0x28;
pub const PRESSURE_ACCEPTANCE: usize = 0x10;
pub const PRESSURE_TENSION: usize = 0x18;
pub const MISSION_ROSTERS: [(u32, usize); 5] =
    [(3, 0x28), (5, 0x58), (8, 0x70), (6, 0x88), (7, 0xa0)];
const _: () = assert!(TRADE_VALUE == PRESSURE_ACCEPTANCE);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub mission_counts: [usize; 5],
    pub native_pointer: usize,
    pub country: u32,
    pub slot: u32,
    pub mission_activity: i64,
    pub operation_activity: i64,
    pub entries_start: usize,
    pub trade_len: usize,
    pub pressure_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub operative: u64,
    pub owner: u32,
    pub value: i64,
    pub tension: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub objects: Option<Array>,
    pub entries_len: usize,
    pub required_entries: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    entries: &mut Vec<Entry>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(entries),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut entries: engine_native_memory::walk::Buffer<'_, Entry>,
) -> Walk {
    const BATCH: usize = 64;
    const OBJECT_FIELDS: [RecordField; 10] = packed_fields([
        (0, 12),
        (MISSION_ROSTERS[0].1, 16),
        (MISSION_ROSTERS[1].1, 16),
        (MISSION_ROSTERS[2].1, 16),
        (MISSION_ROSTERS[3].1, 16),
        (MISSION_ROSTERS[4].1, 16),
        (strategic::TRADE_INFLUENCE + operation_token_map::TABLE, 17),
        (
            strategic::DIPLOMATIC_PRESSURE + operation_token_map::TABLE,
            17,
        ),
        (strategic::MISSION_ACTIVITY, 8),
        (strategic::OPERATION_ACTIVITY, 8),
    ]);
    const OBJECT_BYTES: usize = 142;
    const OBJECT_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&OBJECT_FIELDS, OBJECT_BYTES).unwrap();
    const ROSTER_FIELDS: [RecordField; 5] = packed_fields([
        (MISSION_ROSTERS[0].1, 16),
        (MISSION_ROSTERS[1].1, 16),
        (MISSION_ROSTERS[2].1, 16),
        (MISSION_ROSTERS[3].1, 16),
        (MISSION_ROSTERS[4].1, 16),
    ]);
    const ROSTER_BYTES: usize = 80;
    const ROSTER_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&ROSTER_FIELDS, ROSTER_BYTES).unwrap();
    const SENTINEL_FIELDS: [RecordField; 1] = packed_fields([(0, 1)]);
    const SENTINEL_LAYOUT: RecordLayout<'static> = RecordLayout::new(&SENTINEL_FIELDS, 1).unwrap();

    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let manager_pointer =
            pointer(memory, game_state, cgame_state::STRATEGIC_OPERATIVE_MANAGER)?;
        if manager_pointer == 0 {
            return Ok(Walk {
                outcome: Outcome::Rows { len: 0 },
                objects: None,
                entries_len: 0,
                required_entries: 0,
            });
        }
        let objects = at(manager_pointer, manager::OPERATIVES_DATA).and_then(|address| {
            Array::read(memory, address, manager::OPERATIVE_STRIDE)
                .ok_or("strategic operative vector unreadable or invalid")
        })?;
        let expected = engine_sites::at(base, strategic::VTABLE_RVA)
            .ok_or("strategic operative vtable address overflow")?;
        let (mut len, mut unread_rows) = (0_usize, 0_u32);
        let (mut entries_len, mut required_entries) = (0_usize, 0_usize);
        let mut country_slots = [(
            0_u32,
            engine_native_memory::country_identity::Country {
                pointer: 0,
                canonical_slot: 0,
                id: 0,
            },
        ); BATCH];
        let mut slot_bytes = [0_u8; BATCH * 8];
        let mut obj_ptrs = [0_usize; BATCH];
        let mut obj_map = [u8::MAX; BATCH];
        let mut obj_records = [0_u8; BATCH * OBJECT_BYTES];
        let mut sent_ptrs = [0_usize; BATCH * 2];
        let mut sent_map = [[u8::MAX; 2]; BATCH];
        let mut sent_bytes = [0_u8; BATCH * 2];
        let mut quiet_ptrs = [0_usize; BATCH];
        let mut quiet_map = [u8::MAX; BATCH];
        let mut quiet_rosters = [0_u8; BATCH * ROSTER_BYTES];
        let mut single_rosters = [0_u8; ROSTER_BYTES];
        let mut resolver = engine_native_parts::creference_registry::ScopedResolver::new(base);
        let mut next = 1_u32;
        while next < countries.count() {
            let chunk_start = next;
            let end = countries.count().min(next + BATCH as u32);
            let mut present = 0_usize;
            for slot in chunk_start..end {
                if let Some(country) = countries.at(slot)? {
                    country_slots[present] = (slot, country);
                    present += 1;
                }
            }
            next = end;
            if present == 0 {
                continue;
            }
            let span_len = (end - chunk_start) as usize;
            let slice_ok = end <= objects.count
                && objects
                    .element(chunk_start, manager::OPERATIVE_STRIDE)
                    .is_some_and(|addr| read_slice(memory, addr, &mut slot_bytes[..span_len * 8]));
            let mut live = 0_usize;
            obj_map[..present].fill(u8::MAX);
            if slice_ok {
                for index in 0..present {
                    let off = (country_slots[index].0 - chunk_start) as usize * 8;
                    let raw = u64::from_le_bytes(slot_bytes[off..off + 8].try_into().unwrap());
                    if let Ok(object) = usize::try_from(raw)
                        && object != 0
                    {
                        obj_map[index] = live as u8;
                        obj_ptrs[live] = object;
                        live += 1;
                    }
                }
            }
            let obj_copied = if live == 0 {
                0
            } else {
                memory.copy_records(
                    RecordSources::Pointers(&obj_ptrs[..live]),
                    &OBJECT_LAYOUT,
                    &mut obj_records[..live * OBJECT_BYTES],
                )
            };
            let mut sent_count = 0_usize;
            let mut quiet_count = 0_usize;
            for index in 0..present {
                sent_map[index] = [u8::MAX; 2];
                quiet_map[index] = u8::MAX;
                let mapped = obj_map[index];
                if mapped == u8::MAX || (mapped as usize + 1) * OBJECT_FIELDS.len() > obj_copied {
                    continue;
                }
                let rec = &obj_records[mapped as usize * OBJECT_BYTES..][..OBJECT_BYTES];
                let trade_res = Map::parse_header(rec[92..109].try_into().unwrap(), TRADE_STRIDE);
                if let Ok((_, Some(sentinel))) = trade_res {
                    sent_map[index][0] = sent_count as u8;
                    sent_ptrs[sent_count] = sentinel;
                    sent_count += 1;
                }
                let pressure_res =
                    Map::parse_header(rec[109..126].try_into().unwrap(), PRESSURE_STRIDE);
                if let Ok((_, Some(sentinel))) = pressure_res {
                    sent_map[index][1] = sent_count as u8;
                    sent_ptrs[sent_count] = sentinel;
                    sent_count += 1;
                }
                let trade_empty = matches!(trade_res, Ok((m, _)) if m.count == 0);
                let pressure_empty = matches!(pressure_res, Ok((m, _)) if m.count == 0);
                if trade_empty && pressure_empty {
                    let all_zero = (0..5).all(|r| {
                        let off = 12 + r * 16;
                        u32::from_le_bytes(rec[off + 12..off + 16].try_into().unwrap()) == 0
                    });
                    if all_zero {
                        quiet_map[index] = quiet_count as u8;
                        quiet_ptrs[quiet_count] = obj_ptrs[mapped as usize];
                        quiet_count += 1;
                    }
                }
            }
            let sent_copied = if sent_count == 0 {
                0
            } else {
                memory.copy_records(
                    RecordSources::Pointers(&sent_ptrs[..sent_count]),
                    &SENTINEL_LAYOUT,
                    &mut sent_bytes[..sent_count],
                )
            };
            let quiet_copied = if quiet_count == 0 {
                0
            } else {
                memory.copy_records(
                    RecordSources::Pointers(&quiet_ptrs[..quiet_count]),
                    &ROSTER_LAYOUT,
                    &mut quiet_rosters[..quiet_count * ROSTER_BYTES],
                )
            };
            for index in 0..present {
                let (slot, country) = country_slots[index];
                let object = if slice_ok {
                    let off = (slot - chunk_start) as usize * 8;
                    usize::try_from(u64::from_le_bytes(
                        slot_bytes[off..off + 8].try_into().unwrap(),
                    ))
                    .map_err(|_| "strategic operative slot unreadable or outside the vector")?
                } else {
                    objects
                        .element(slot, manager::OPERATIVE_STRIDE)
                        .and_then(|address| read::<u64>(memory, address))
                        .and_then(|object| usize::try_from(object).ok())
                        .ok_or("strategic operative slot unreadable or outside the vector")?
                };
                if object == 0 {
                    continue;
                }
                let mapped = obj_map[index];
                let batched =
                    mapped != u8::MAX && (mapped as usize + 1) * OBJECT_FIELDS.len() <= obj_copied;
                let (owner, mission_activity, operation_activity, trade, pressure, rosters) =
                    if batched {
                        let rec = &obj_records[mapped as usize * OBJECT_BYTES..][..OBJECT_BYTES];
                        if u64::from_le_bytes(rec[0..8].try_into().unwrap()) != expected as u64 {
                            return Err("strategic operative vtable mismatch");
                        }
                        let owner = u32::from_le_bytes(rec[8..12].try_into().unwrap());
                        if countries.resolve(owner)?.pointer != country.pointer {
                            return Err("strategic operative owner does not match its country slot");
                        }
                        let mission_activity =
                            i64::from_le_bytes(rec[126..134].try_into().unwrap());
                        let operation_activity =
                            i64::from_le_bytes(rec[134..142].try_into().unwrap());
                        let (trade, trade_sentinel) =
                            Map::parse_header(rec[92..109].try_into().unwrap(), TRADE_STRIDE)?;
                        if let Some(sentinel) = trade_sentinel {
                            let s_idx = sent_map[index][0];
                            let ok = if s_idx != u8::MAX && (s_idx as usize) < sent_copied {
                                sent_bytes[s_idx as usize] == u8::MAX
                            } else {
                                read::<u8>(memory, sentinel) == Some(u8::MAX)
                            };
                            if !ok {
                                return Err(
                                    "strategic operative map end sentinel invalid or unreadable",
                                );
                            }
                        }
                        let (pressure, pressure_sentinel) =
                            Map::parse_header(rec[109..126].try_into().unwrap(), PRESSURE_STRIDE)?;
                        if let Some(sentinel) = pressure_sentinel {
                            let s_idx = sent_map[index][1];
                            let ok = if s_idx != u8::MAX && (s_idx as usize) < sent_copied {
                                sent_bytes[s_idx as usize] == u8::MAX
                            } else {
                                read::<u8>(memory, sentinel) == Some(u8::MAX)
                            };
                            if !ok {
                                return Err(
                                    "strategic operative map end sentinel invalid or unreadable",
                                );
                            }
                        }
                        let mut rosters = [Array::default(); 5];
                        for r in 0..5 {
                            let off = 12 + r * 16;
                            rosters[r] =
                                Array::from_header(rec[off..off + 16].try_into().unwrap(), 8)
                                    .ok_or("strategic operative mission roster invalid")?;
                        }
                        (
                            owner,
                            mission_activity,
                            operation_activity,
                            trade,
                            pressure,
                            rosters,
                        )
                    } else {
                        if read::<u64>(memory, object) != Some(expected as u64) {
                            return Err("strategic operative vtable mismatch");
                        }
                        let owner = field::<u32>(memory, object, strategic::OWNER)?;
                        if countries.resolve(owner)?.pointer != country.pointer {
                            return Err("strategic operative owner does not match its country slot");
                        }
                        let mission_activity = field(memory, object, strategic::MISSION_ACTIVITY)?;
                        let operation_activity =
                            field(memory, object, strategic::OPERATION_ACTIVITY)?;
                        let trade = Map::read(
                            memory,
                            at(object, strategic::TRADE_INFLUENCE)?,
                            TRADE_STRIDE,
                        )?;
                        let pressure = Map::read(
                            memory,
                            at(object, strategic::DIPLOMATIC_PRESSURE)?,
                            PRESSURE_STRIDE,
                        )?;
                        let mut rosters = [Array::default(); 5];
                        for (r, (_, offset)) in MISSION_ROSTERS.into_iter().enumerate() {
                            rosters[r] = Array::read(memory, at(object, offset)?, 8)
                                .ok_or("strategic operative mission roster invalid")?;
                        }
                        (
                            owner,
                            mission_activity,
                            operation_activity,
                            trade,
                            pressure,
                            rosters,
                        )
                    };
                let mut mission_counts = [0; 5];
                let mut mission_entries = 0usize;
                for r in 0..5 {
                    mission_counts[r] = rosters[r].count as usize;
                    mission_entries = mission_entries
                        .checked_add(mission_counts[r])
                        .ok_or("strategic operative mission extent overflow")?;
                }
                let count = (trade.count as usize)
                    .checked_add(pressure.count as usize)
                    .and_then(|count| count.checked_add(mission_entries))
                    .ok_or("strategic operative entry count overflow")?;
                required_entries = required_entries
                    .checked_add(count)
                    .ok_or("strategic operative entry demand overflow")?;
                entries.ensure(required_entries)?;
                output.ensure(
                    len.checked_add(1)
                        .ok_or("strategic operative row demand overflow")?,
                )?;
                let fits = len < output.len() && count <= entries.len() - entries_len;
                let mut written = 0;
                for (map, tension) in [(trade, None), (pressure, Some(PRESSURE_TENSION))] {
                    map.visit(memory, &countries, |owner, slot| {
                        if fits {
                            entries[entries_len + written] = Entry {
                                operative: 0,
                                owner,
                                value: word(slot, TRADE_VALUE),
                                tension: tension.map_or(0, |at| word(slot, at)),
                            };
                            written += 1;
                        }
                    })?;
                }
                if mission_entries == 0 {
                    let q = quiet_map[index];
                    let verified = if q != u8::MAX && (q as usize + 1) * 5 <= quiet_copied {
                        let rec = &quiet_rosters[q as usize * ROSTER_BYTES..][..ROSTER_BYTES];
                        (0..5).all(|r| {
                            Array::from_header(
                                rec[r * 16..(r + 1) * 16].try_into().unwrap(),
                                8,
                            ) == Some(rosters[r])
                        })
                    } else if memory.copy_records(
                        RecordSources::Pointers(&[object]),
                        &ROSTER_LAYOUT,
                        &mut single_rosters,
                    ) == 5
                    {
                        (0..5).all(|r| {
                            Array::from_header(
                                single_rosters[r * 16..(r + 1) * 16].try_into().unwrap(),
                                8,
                            ) == Some(rosters[r])
                        })
                    } else {
                        let mut ok = true;
                        for (roster, (_, offset)) in rosters.into_iter().zip(MISSION_ROSTERS) {
                            if Array::read(memory, at(object, offset)?, 8) != Some(roster) {
                                ok = false;
                                break;
                            }
                        }
                        ok
                    };
                    if !verified {
                        return Err("strategic operative mission roster changed during capture");
                    }
                } else {
                    for (roster, (_, offset)) in rosters.into_iter().zip(MISSION_ROSTERS) {
                        for index in 0..roster.count {
                            let handle = roster
                                .element(index, 8)
                                .and_then(|address| read::<u64>(memory, address))
                                .ok_or("strategic operative mission reference unreadable")?;
                            let operative = match resolver.resolve(memory, handle)? {
                                None => 0,
                                Some(pointer) => {
                                    if field::<u64>(memory, pointer, 0)?
                                        != engine_sites::at(
                                            base,
                                            engine_native_parts::coperative_leader_layout::VTABLE_RVA,
                                        )
                                        .ok_or("strategic operative mission vtable overflow")?
                                            as u64
                                        || field::<u64>(
                                            memory,
                                            pointer,
                                            engine_native_intelligence::coperative_leader::IDENTITY,
                                        )? != handle
                                    {
                                        return Err(
                                            "strategic operative mission identity mismatch",
                                        );
                                    }
                                    handle
                                }
                            };
                            if fits {
                                entries[entries_len + written] = Entry {
                                    operative,
                                    ..Entry::default()
                                };
                                written += 1;
                            }
                        }
                        if Array::read(memory, at(object, offset)?, 8) != Some(roster) {
                            return Err(
                                "strategic operative mission roster changed during capture",
                            );
                        }
                    }
                }
                if !fits {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("strategic operative unread row count overflow")?;
                    continue;
                }
                output[len] = Row {
                    mission_counts,
                    native_pointer: object,
                    country: owner,
                    slot,
                    mission_activity,
                    operation_activity,
                    entries_start: entries_len,
                    trade_len: trade.count as usize,
                    pressure_len: pressure.count as usize,
                };
                len += 1;
                entries_len += count;
            }
        }
        Ok(Walk {
            objects: Some(objects),
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            entries_len,
            required_entries,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        objects: None,
        outcome: Outcome::Unavailable { reason },
        entries_len: 0,
        required_entries: 0,
    })
}

#[derive(Debug, Clone, Copy)]
struct Map {
    data: usize,
    count: u32,
    slots: usize,
    mask: u32,
    overflow: u8,
    stride: usize,
}

fn word(slot: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(slot[at..at + 8].try_into().unwrap())
}

impl Map {
    fn parse_header(header: [u8; 17], stride: usize) -> Result<(Self, Option<usize>), &'static str> {
        let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "strategic operative map pointer overflow")?;
        let count = i32::from_le_bytes(header[8..12].try_into().unwrap());
        let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let overflow = header[16];
        if count < 0 {
            return Err("strategic operative map count negative");
        }
        if mask == 0 && overflow == 0 {
            return if count == 0 {
                Ok((
                    Self {
                        data,
                        count: 0,
                        slots: 0,
                        mask,
                        overflow,
                        stride,
                    },
                    None,
                ))
            } else {
                Err("strategic operative static map has live entries")
            };
        }
        let buckets = mask
            .checked_add(1)
            .ok_or("strategic operative map mask overflow")?;
        if !buckets.is_power_of_two()
            || !(2..=1 << 30).contains(&buckets)
            || u32::from(overflow) != buckets.trailing_zeros() + 2
            || count as u32 > buckets
        {
            return Err("strategic operative map geometry invalid");
        }
        let slots = buckets as usize + usize::from(overflow);
        if data == 0 {
            return Err("strategic operative map storage null");
        }
        let sentinel = slots
            .checked_mul(stride)
            .and_then(|extent| data.checked_add(extent))
            .and_then(|end| end.checked_add(ENTRY_PROBE))
            .ok_or("strategic operative map extent overflow")?;
        Ok((
            Self {
                data,
                count: count as u32,
                slots,
                mask,
                overflow,
                stride,
            },
            Some(sentinel),
        ))
    }

    fn read(memory: &impl Memory, map: usize, stride: usize) -> Result<Self, &'static str> {
        let header = at(map, operation_token_map::TABLE).and_then(|address| {
            read::<[u8; 17]>(memory, address).ok_or("strategic operative map header unreadable")
        })?;
        let (parsed, sentinel) = Self::parse_header(header, stride)?;
        if let Some(sentinel) = sentinel
            && read::<u8>(memory, sentinel) != Some(u8::MAX)
        {
            return Err("strategic operative map end sentinel invalid or unreadable");
        }
        Ok(parsed)
    }

    fn visit<M: Memory>(
        &self,
        memory: &M,
        countries: &ccountry::Reader<'_, M>,
        mut each: impl FnMut(u32, &[u8]),
    ) -> Result<(), &'static str> {
        const BATCH: usize = 64;
        let mut bytes = [0_u8; BATCH * PRESSURE_STRIDE];
        let mut observed = 0_u32;
        let mut first = 0;
        while first < self.slots {
            let batch = (self.slots - first).min(BATCH);
            let address = first
                .checked_mul(self.stride)
                .and_then(|offset| self.data.checked_add(offset))
                .ok_or("strategic operative map address overflow")?;
            let chunk = &mut bytes[..batch * self.stride];
            if !read_slice(memory, address, chunk) {
                return Err("strategic operative map slot unreadable");
            }
            for (offset, slot) in chunk.chunks_exact(self.stride).enumerate() {
                let index = first + offset;
                let distance = slot[ENTRY_PROBE];
                match distance {
                    0 => continue,
                    u8::MAX => return Err("strategic operative map sentinel inside census"),
                    _ => {}
                }
                let hash = u32::from_le_bytes(slot[ENTRY_HASH..ENTRY_HASH + 4].try_into().unwrap());
                let home = (hash & self.mask) as usize;
                if distance > self.overflow || home + usize::from(distance) - 1 != index {
                    return Err("strategic operative map probe distance invalid");
                }
                observed = observed
                    .checked_add(1)
                    .filter(|&observed| observed <= self.count)
                    .ok_or("strategic operative map census differs from count")?;
                let country =
                    u32::from_le_bytes(slot[ENTRY_COUNTRY..ENTRY_COUNTRY + 4].try_into().unwrap());
                if country != 0 {
                    countries.resolve(country)?;
                }
                each(country, slot);
            }
            first += batch;
        }
        if observed != self.count {
            return Err("strategic operative map census differs from count");
        }
        Ok(())
    }
}

pub fn trade_influence<M: Memory>(
    memory: &M,
    base: usize,
    game: usize,
    countries: &ccountry::Reader<'_, M>,
    target: u32,
    owner: u32,
) -> Result<i64, &'static str> {
    let target = countries.resolve(target)?;
    let owner = countries.resolve(owner)?;
    let manager = pointer(memory, game, cgame_state::STRATEGIC_OPERATIVE_MANAGER)?;
    if manager == 0 {
        return Ok(0);
    }
    let objects = Array::read(memory, at(manager, manager::OPERATIVES_DATA)?, 8)
        .ok_or("strategic operative membership invalid")?;
    let object = objects
        .element(target.canonical_slot, 8)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("strategic operative target unreadable")? as usize;
    if object == 0 {
        return Ok(0);
    }
    let expected = engine_sites::at(base, strategic::VTABLE_RVA)
        .ok_or("strategic operative address overflow")?;
    if field::<u64>(memory, object, 0)? != expected as u64
        || countries
            .resolve(field(memory, object, strategic::OWNER)?)?
            .canonical_slot
            != target.canonical_slot
    {
        return Err("strategic operative target identity mismatch");
    }
    let at = at(object, strategic::TRADE_INFLUENCE)?;
    let header =
        read::<[u8; 25]>(memory, at).ok_or("strategic operative trade header unreadable")?;
    let map = Map::read(memory, at, TRADE_STRIDE)?;
    let mut value = 0;
    let mut invalid = false;
    let mut found = false;
    map.visit(memory, countries, |raw, slot| {
        if raw == owner.id
            || (raw != 0
                && countries
                    .resolve(raw)
                    .is_ok_and(|c| c.canonical_slot == owner.canonical_slot))
        {
            invalid |= found;
            found = true;
            value = word(slot, TRADE_VALUE);
        }
    })?;
    if invalid || read::<[u8; 25]>(memory, at) != Some(header) {
        return Err("strategic operative trade map changed");
    }
    Ok(value)
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("strategic operative address overflow")
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("strategic operative field unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "strategic operative pointer overflow")
}

#[cfg(test)]
mod tests;
