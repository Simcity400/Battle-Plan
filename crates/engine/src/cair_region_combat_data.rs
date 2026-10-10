//! CAirRegionCombatData's retained regional air-combat state.
//! Hourly writers and daily pruning finish before the sampling boundary;
//! live consistency, save coverage and aggregate cost are separate checks.

use crate::{
    Memory,
    cpdx_array::Array,
    cstrategic_air,
    memory::{RecordField, RecordLayout, RecordSources},
    sair_wing_combat_data, sdamage_entry,
    walk::Outcome,
};

/// Regional combat-data vtable.
pub const VTABLE_RVA: usize = 0x0294_2188;
/// First wing-combat-record vector.
pub const FIRST_RECORDS: usize = 0x08;
/// First-vector capacity, i32.
pub const FIRST_RECORDS_CAPACITY: usize = 0x10;
/// First-vector count, i32.
pub const FIRST_RECORDS_COUNT: usize = 0x14;
/// Second wing-combat-record vector, selected by reporting country.
pub const SECOND_RECORDS: usize = 0x20;
/// Second-vector capacity, i32.
pub const SECOND_RECORDS_CAPACITY: usize = 0x28;
/// Second-vector count, i32.
pub const SECOND_RECORDS_COUNT: usize = 0x2c;
/// Raw owner-country link, including zero, copied from strategic air.
pub const OWNER: usize = 0x38;
/// Record lookup and creation using the shared vector layout.
pub const GET_OR_CREATE_RECORD_RVA: usize = 0x0195_0340;
/// Record update accumulating flags, masks, plane high-water count and damage.
pub const UPDATE_RECORD_RVA: usize = 0x0194_f590;
/// Pruning records older than the runtime timeout from both vectors.
pub const PRUNE_RVA: usize = 0x0195_01b0;

/// One non-null slot from a completely enumerated, authenticated CStrategicAir
/// region table. The slot index is the native strategic-region id. Addresses
/// are read context only and never become recorded identity.
#[derive(Debug, Clone, Copy)]
pub struct Root {
    pub country: u32,
    pub region: u32,
    pub address: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub records: Outcome,
    pub damage: Outcome,
}

/// Region slots and roots gathered per guarded round.
const ROOT_BATCH: usize = 64;
const ROOT_RECORD_BYTES: usize = 44;
const POINTER_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELD, 8).expect("valid record layout");
/// The exact identity, owner and both record-vector headers, in that order.
const ROOT_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: OWNER,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: FIRST_RECORDS,
        destination_offset: 12,
        len: 16,
    },
    RecordField {
        source_offset: SECOND_RECORDS,
        destination_offset: 28,
        len: 16,
    },
];
const ROOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ROOT_FIELDS, ROOT_RECORD_BYTES).expect("valid record layout");

/// Enumerates the complete country/region source directly. Every manager slot
/// is authenticated and every region-pointer slot is read, including region
/// zero. Only null native region pointers are absent. A late root failure
/// invalidates both tables rather than certifying a readable prefix.
///
/// Empty allocated holders contribute no rows; this table records retained
/// entries, not the allocation of an otherwise empty holder. Preserve source
/// order when grouping records for playback, including duplicate native lookup
/// coordinates. Damage has no parent-record ordinal, so assigning its entries
/// among duplicate parent coordinates is explicitly ambiguous.
pub fn walk_world(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    records: &mut [sair_wing_combat_data::Row],
    damage: &mut [sdamage_entry::Row],
) -> Capture {
    walk_world_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(records),
        crate::walk::Buffer::Fixed(damage),
    )
}

pub fn walk_world_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    records: &mut Vec<sair_wing_combat_data::Row>,
    damage: &mut Vec<sdamage_entry::Row>,
) -> Capture {
    walk_world_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(records),
        crate::walk::Buffer::Growing(damage),
    )
}

pub fn walk_world_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut records: crate::walk::Buffer<'_, sair_wing_combat_data::Row>,
    mut damage: crate::walk::Buffer<'_, sdamage_entry::Row>,
) -> Capture {
    let mut record_len = 0_usize;
    let mut record_omitted = 0_u32;
    let mut damage_len = 0_usize;
    let mut damage_omitted = 0_u32;
    let mut damage_error = None;
    let result = cstrategic_air::visit(memory, base, game_state, |country| {
        let regions = country
            .address
            .checked_add(cstrategic_air::REGION_COMBAT)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("air combat region roster unreadable or invalid")?;
        // The region table is a contiguous pointer vector. One guarded round
        // per batch replaces one read per region, and the live roots of that
        // batch enter a single walk whose per-root reads are gathered too.
        // Null slots and the first unreadable slot keep their positions.
        let mut slots = [0_usize; ROOT_BATCH];
        let mut slot_bytes = [0_u8; ROOT_BATCH * 8];
        let mut roots = [Root {
            country: 0,
            region: 0,
            address: 0,
        }; ROOT_BATCH];
        let mut first = 0;
        while first < regions.count {
            let count = ((regions.count - first) as usize).min(ROOT_BATCH);
            for (index, slot) in slots[..count].iter_mut().enumerate() {
                *slot = regions.element(first + index as u32, 8).unwrap_or(0);
            }
            let copied = memory.copy_records(
                RecordSources::Pointers(&slots[..count]),
                &POINTER_LAYOUT,
                &mut slot_bytes[..count * 8],
            );
            let mut live = 0;
            let mut unreadable = copied < count;
            for index in 0..copied.min(count) {
                let bytes = slot_bytes[index * 8..index * 8 + 8].try_into().unwrap();
                let Ok(address) = usize::try_from(u64::from_le_bytes(bytes)) else {
                    unreadable = true;
                    break;
                };
                if address != 0 {
                    roots[live] = Root {
                        country: country.country,
                        region: first + index as u32,
                        address,
                    };
                    live += 1;
                }
            }
            let remaining_damage = if damage_error.is_none() {
                damage.suffix(damage_len)
            } else {
                crate::walk::Buffer::Fixed(&mut [])
            };
            let next = walk_buffer(
                memory,
                base,
                &roots[..live],
                records.suffix(record_len),
                remaining_damage,
            );
            match next.records {
                Outcome::Rows { len } => record_len += len,
                Outcome::Truncated { len, unread_rows } => {
                    record_len += len;
                    record_omitted = record_omitted
                        .checked_add(unread_rows)
                        .ok_or("air combat record count overflow")?;
                }
                Outcome::Unavailable { reason } => return Err(reason),
            }
            if damage_error.is_none() {
                match next.damage {
                    Outcome::Rows { len } => damage_len += len,
                    Outcome::Truncated { len, unread_rows } => {
                        damage_len += len;
                        if let Some(total) = damage_omitted.checked_add(unread_rows) {
                            damage_omitted = total;
                        } else {
                            damage_error = Some("air damage row count overflow");
                        }
                    }
                    Outcome::Unavailable { reason } => damage_error = Some(reason),
                }
            }
            if unreadable {
                return Err("air combat region pointer unreadable or address overflow");
            }
            first += count as u32;
        }
        Ok(())
    });
    if let Err(reason) = result {
        return Capture {
            records: Outcome::Unavailable { reason },
            damage: Outcome::Unavailable { reason },
        };
    }
    Capture {
        records: outcome(record_len, record_omitted),
        damage: match damage_error {
            Some(reason) => Outcome::Unavailable { reason },
            None => outcome(damage_len, damage_omitted),
        },
    }
}

/// Reads both physical record vectors and each record's two damage vectors.
/// The caller supplies the complete root census; an unavailable or truncated
/// root census must not be represented by a shorter successful iterator.
/// No allocation, sorting, retry, engine call or generation tracking occurs.
/// Native lookup coordinates need not be unique after load. Preserve source
/// order and duplicates in grouped rows; never overwrite by lookup key.
/// A damage-only fault preserves the independent record table. Capacity is
/// independent too: remaining headers supply exact omitted row counts.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    records: &mut [sair_wing_combat_data::Row],
    damage: &mut [sdamage_entry::Row],
) -> Capture {
    walk_buffer(
        memory,
        base,
        roots,
        crate::walk::Buffer::Fixed(records),
        crate::walk::Buffer::Fixed(damage),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    records: &mut Vec<sair_wing_combat_data::Row>,
    damage: &mut Vec<sdamage_entry::Row>,
) -> Capture {
    walk_buffer(
        memory,
        base,
        roots,
        crate::walk::Buffer::Growing(records),
        crate::walk::Buffer::Growing(damage),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    mut records: crate::walk::Buffer<'_, sair_wing_combat_data::Row>,
    mut damage: crate::walk::Buffer<'_, sdamage_entry::Row>,
) -> Capture {
    let result = (|| {
        let mut record_len = 0_usize;
        let mut record_omitted = 0_u32;
        let mut damage_len = 0_usize;
        let mut damage_omitted = 0_u32;
        let mut damage_error = None;
        // Every root's identity, owner and two record-vector headers come from
        // one guarded gather per batch. Roots are still decoded in order with
        // their own copied prefix and the original overflow-check ordering.
        let mut addresses = [0_usize; ROOT_BATCH];
        let mut copied_fields = [0_usize; ROOT_BATCH];
        let mut identities = [0_u8; ROOT_BATCH * ROOT_RECORD_BYTES];
        let mut batch_first = 0;
        let mut batch_len = 0;
        for (index, root) in roots.iter().enumerate() {
            if index >= batch_first + batch_len {
                batch_first = index;
                batch_len = (roots.len() - index).min(ROOT_BATCH);
                for (slot, staged) in addresses[..batch_len].iter_mut().enumerate() {
                    *staged = roots[index + slot].address;
                }
                let completed = memory.copy_records(
                    RecordSources::Pointers(&addresses[..batch_len]),
                    &ROOT_LAYOUT,
                    &mut identities[..batch_len * ROOT_RECORD_BYTES],
                );
                for (slot, fields) in copied_fields[..batch_len].iter_mut().enumerate() {
                    *fields = completed
                        .saturating_sub(slot * ROOT_FIELDS.len())
                        .min(ROOT_FIELDS.len());
                }
            }
            let slot = index - batch_first;
            let copied = copied_fields[slot];
            let record = &identities[slot * ROOT_RECORD_BYTES..(slot + 1) * ROOT_RECORD_BYTES];
            let expected = base
                .checked_add(VTABLE_RVA)
                .ok_or("module address overflow")?;
            if copied == 0 || u64::from_le_bytes(record[..8].try_into().unwrap()) != expected as u64
            {
                return Err("air combat region vtable mismatch or unreadable");
            }
            root.address
                .checked_add(OWNER)
                .ok_or("air combat region address overflow")?;
            if copied < 2 {
                return Err("air combat region owner unreadable");
            }
            let region_owner = u32::from_le_bytes(record[8..12].try_into().unwrap());
            for (side, offset) in [(0_u8, FIRST_RECORDS), (1, SECOND_RECORDS)] {
                root.address
                    .checked_add(offset)
                    .ok_or("air combat region address overflow")?;
                let header = 12 + usize::from(side) * 16;
                let vector = (copied > 2 + usize::from(side))
                    .then(|| {
                        Array::from_header(
                            record[header..header + 16].try_into().unwrap(),
                            sair_wing_combat_data::STRIDE,
                        )
                    })
                    .flatten()
                    .ok_or("air combat record vector unreadable or invalid")?;
                // Both record vectors grow from these acquired headers.
                records.ensure(
                    record_len
                        .checked_add(vector.count as usize)
                        .ok_or("air combat record count overflow")?,
                )?;
                for ordinal in 0..vector.count {
                    let address = vector
                        .element(ordinal, sair_wing_combat_data::STRIDE)
                        .ok_or("air combat record address overflow")?;
                    let need_record = record_len < records.len();
                    let need_damage = damage_error.is_none()
                        && (damage.is_growing() || damage_len < damage.len());
                    let row = if need_record || need_damage {
                        Some(sair_wing_combat_data::read_row(
                            memory,
                            base,
                            address,
                            root.country,
                            root.region,
                            side,
                            region_owner,
                        )?)
                    } else {
                        None
                    };
                    if need_record {
                        records[record_len] = row.unwrap();
                        record_len += 1;
                    } else {
                        record_omitted = record_omitted
                            .checked_add(1)
                            .ok_or("air combat record count overflow")?;
                    }
                    if damage_error.is_some() {
                        continue;
                    }
                    let damage_result = (|| {
                        for (channel, offset) in [
                            (0, sair_wing_combat_data::FIRST_DAMAGE),
                            (1, sair_wing_combat_data::SECOND_DAMAGE),
                        ] {
                            let header = address
                                .checked_add(offset)
                                .ok_or("air damage vector address overflow")?;
                            let entries = Array::read(memory, header, sdamage_entry::STRIDE)
                                .ok_or("air damage vector unreadable or invalid")?;
                            damage.ensure(
                                damage_len
                                    .checked_add(entries.count as usize)
                                    .ok_or("air damage row count overflow")?,
                            )?;
                            let available = damage.len() - damage_len;
                            let admitted = (entries.count as usize).min(available);
                            damage_omitted = damage_omitted
                                .checked_add(entries.count - admitted as u32)
                                .ok_or("air damage row count overflow")?;
                            for index in 0..admitted as u32 {
                                let entry = entries
                                    .element(index, sdamage_entry::STRIDE)
                                    .ok_or("air damage entry address overflow")?;
                                damage[damage_len] = sdamage_entry::read_row(
                                    memory,
                                    base,
                                    entry,
                                    row.unwrap().key,
                                    channel,
                                )?;
                                damage_len += 1;
                            }
                        }
                        Ok::<_, &'static str>(())
                    })();
                    if let Err(reason) = damage_result {
                        damage_error = Some(reason);
                    }
                }
            }
        }
        Ok(Capture {
            records: outcome(record_len, record_omitted),
            damage: match damage_error {
                Some(reason) => Outcome::Unavailable { reason },
                None => outcome(damage_len, damage_omitted),
            },
        })
    })();
    result.unwrap_or_else(|reason| Capture {
        records: Outcome::Unavailable { reason },
        damage: Outcome::Unavailable { reason },
    })
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use crate::{ccountry, cgame_state, cstrategic_air_manager};

    const MEMORY_BASE: usize = 0x1000;
    const IMAGE_BASE: usize = 0x1_4000_0000;
    const REGION: usize = 0x1000;
    const FIRST: usize = 0x1200;
    const SECOND: usize = 0x1500;
    const DAMAGE: usize = 0x1800;
    const GAME: usize = 0x3000;
    const COUNTRY: usize = 0x5000;
    const MANAGER: usize = 0x6000;
    const AIR: usize = 0x7000;
    const AIR_COUNTRIES: usize = 0x8200;
    const REGION_SLOTS: usize = 0x9000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        let start = address - MEMORY_BASE;
        bytes[start..start + value.len()].copy_from_slice(value);
    }

    fn vector(bytes: &mut [u8], address: usize, data: usize, count: u32) {
        put(bytes, address, &(data as u64).to_le_bytes());
        put(bytes, address + 8, &count.to_le_bytes());
        put(bytes, address + 12, &count.to_le_bytes());
    }

    /// A root batch must report the failing root's own reason at its own
    /// position, even though the batch already gathered the roots after it.
    #[test]
    fn a_mid_batch_root_fault_keeps_its_reason_and_leaves_later_roots_unused() {
        struct Refuse<'a> {
            memory: SliceMemory<'a>,
            address: usize,
        }
        impl Memory for Refuse<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address <= self.address && self.address - address < destination.len() {
                    return false;
                }
                self.memory.copy(address, destination)
            }
        }
        let mut bytes = vec![0; 0x1000];
        let roots: [Root; 3] = std::array::from_fn(|index| {
            let address = REGION + index * 0x100;
            put(
                &mut bytes,
                address,
                &((IMAGE_BASE + VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(&mut bytes, address + OWNER, &(index as u32).to_le_bytes());
            Root {
                country: 3,
                region: index as u32,
                address,
            }
        });
        assert_eq!(
            walk(
                &SliceMemory {
                    base: MEMORY_BASE,
                    bytes: &bytes
                },
                IMAGE_BASE,
                &roots,
                &mut [],
                &mut []
            ),
            Capture {
                records: Outcome::Rows { len: 0 },
                damage: Outcome::Rows { len: 0 },
            }
        );
        for (address, reason) in [
            (
                REGION + 0x100,
                "air combat region vtable mismatch or unreadable",
            ),
            (REGION + 0x100 + OWNER, "air combat region owner unreadable"),
            (
                REGION + 0x100 + SECOND_RECORDS,
                "air combat record vector unreadable or invalid",
            ),
        ] {
            let memory = Refuse {
                memory: SliceMemory {
                    base: MEMORY_BASE,
                    bytes: &bytes,
                },
                address,
            };
            assert_eq!(
                walk(&memory, IMAGE_BASE, &roots, &mut [], &mut []),
                Capture {
                    records: Outcome::Unavailable { reason },
                    damage: Outcome::Unavailable { reason },
                },
                "refused {address:#x}"
            );
        }
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        put(
            &mut bytes,
            REGION,
            &((IMAGE_BASE + VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, REGION + OWNER, &77_u32.to_le_bytes());
        vector(&mut bytes, REGION + FIRST_RECORDS, FIRST, 2);
        vector(&mut bytes, REGION + SECOND_RECORDS, SECOND, 1);
        for (address, wing, classification) in [
            (FIRST, 9_u32, 0),
            (FIRST + sair_wing_combat_data::STRIDE, 9_u32, 1),
            (SECOND, 10_u32, 0),
        ] {
            put(
                &mut bytes,
                address,
                &((IMAGE_BASE + sair_wing_combat_data::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::WING_ID,
                &wing.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::DATE,
                &(-3_i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::PLANE_COUNT,
                &i32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::MISSION_MASK,
                &u32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::CLASSIFICATION_FLAG,
                &[classification],
            );
        }
        vector(
            &mut bytes,
            FIRST + sair_wing_combat_data::FIRST_DAMAGE,
            DAMAGE,
            2,
        );
        for (index, cause, amount) in [(0, u32::MAX, i64::MAX), (1, 1, i64::MIN)] {
            let address = DAMAGE + index * sdamage_entry::STRIDE;
            put(
                &mut bytes,
                address,
                &((IMAGE_BASE + sdamage_entry::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sdamage_entry::CAUSE_TYPE,
                &cause.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sdamage_entry::AMOUNT,
                &amount.to_le_bytes(),
            );
        }
        bytes
    }

    fn world_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        bytes.resize(0x9000, 0);
        vector(&mut bytes, GAME + cgame_state::COUNTRIES, 0x8000, 3);
        vector(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, 0x8100, 4);
        put(&mut bytes, 0x8008, &(COUNTRY as u64).to_le_bytes());
        put(&mut bytes, 0x8010, &(COUNTRY as u64).to_le_bytes());
        for index in 1..4 {
            put(&mut bytes, 0x8100 + index * 4, &1_u32.to_le_bytes());
        }
        put(
            &mut bytes,
            COUNTRY,
            &((IMAGE_BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            GAME + cgame_state::STRATEGIC_AIR_MANAGER,
            &(MANAGER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER,
            &((IMAGE_BASE + cstrategic_air_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        vector(
            &mut bytes,
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            AIR_COUNTRIES,
            3,
        );
        put(&mut bytes, AIR_COUNTRIES + 8, &(AIR as u64).to_le_bytes());
        put(
            &mut bytes,
            AIR,
            &((IMAGE_BASE + cstrategic_air::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AIR + cstrategic_air::COUNTRY,
            &2_u32.to_le_bytes(),
        );
        vector(
            &mut bytes,
            AIR + cstrategic_air::REGION_COMBAT,
            REGION_SLOTS,
            3,
        );
        put(&mut bytes, REGION_SLOTS, &(REGION as u64).to_le_bytes());
        bytes
    }

    #[test]
    fn world_census_preserves_region_zero_owner_alias_and_exact_capacity() {
        let bytes = world_fixture();
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        let census = crate::read_census::ReadCensus::new(&memory);
        let mut records = [sair_wing_combat_data::Row::default(); 3];
        let mut damage = [sdamage_entry::Row::default(); 2];
        assert_eq!(
            walk_world(&census, IMAGE_BASE, GAME, &mut records, &mut damage),
            Capture {
                records: Outcome::Rows { len: 3 },
                damage: Outcome::Rows { len: 2 },
            }
        );
        // Deterministic cost proxy. Every country slot costs one shared slot
        // gather and one identity gather per 64 slots, and every batch of 64
        // region slots costs one pointer gather plus one root gather, instead
        // of three reads per country slot and five per region.
        assert_eq!(census.calls(), 29);
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        assert!(
            records
                .iter()
                .all(|row| row.key.country == 2 && row.key.region == 0)
        );
        assert_eq!(
            walk_world(
                &memory,
                IMAGE_BASE,
                GAME,
                &mut records[..1],
                &mut damage[..1]
            ),
            Capture {
                records: Outcome::Truncated {
                    len: 1,
                    unread_rows: 2
                },
                damage: Outcome::Truncated {
                    len: 1,
                    unread_rows: 1
                },
            }
        );
    }

    #[test]
    fn late_world_root_failure_never_certifies_earlier_rows() {
        let mut records = [sair_wing_combat_data::Row::default(); 3];
        let mut damage = [sdamage_entry::Row::default(); 2];
        // A second live manager slot and a second live region slot both fail
        // after the first one's rows were already read, and each keeps its own
        // reason even though its whole batch was gathered before decoding.
        for (slot, pointer, reason) in [
            (
                AIR_COUNTRIES + 16,
                AIR as u64,
                "strategic air owner differs from canonical country slot",
            ),
            (
                REGION_SLOTS + 16,
                u64::MAX,
                "air combat region vtable mismatch or unreadable",
            ),
        ] {
            let mut bytes = world_fixture();
            put(&mut bytes, slot, &pointer.to_le_bytes());
            let memory = SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            };
            let result = walk_world(&memory, IMAGE_BASE, GAME, &mut records, &mut damage);
            assert_eq!(result.records, Outcome::Unavailable { reason });
            assert_eq!(result.records, result.damage);
        }
    }

    fn capture(
        bytes: &[u8],
        records: &mut [sair_wing_combat_data::Row],
        damage: &mut [sdamage_entry::Row],
    ) -> Capture {
        walk(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes,
            },
            IMAGE_BASE,
            &[Root {
                country: 12,
                region: 35,
                address: REGION,
            }],
            records,
            damage,
        )
    }

    #[test]
    fn retains_native_composite_identity_flags_owner_and_full_damage_width() {
        let mut bytes = fixture();
        let mut records = [sair_wing_combat_data::Row::default(); 3];
        let mut damage = [sdamage_entry::Row::default(); 2];
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Rows { len: 3 },
                damage: Outcome::Rows { len: 2 },
            }
        );
        assert_eq!(
            records[0].key,
            sair_wing_combat_data::Key {
                country: 12,
                region: 35,
                side: 0,
                date: -3,
                wing_id: 9,
                classification: 0,
            }
        );
        assert_ne!(records[0].key, records[1].key);
        assert_eq!(records[2].key.side, 1);
        assert_eq!(records[0].region_owner, 77);
        assert_eq!(records[0].attributed_country, 0);
        assert_eq!(records[0].planes, i32::MAX);
        assert_eq!(records[0].mission_mask, u32::MAX);
        assert_eq!((damage[0].cause, damage[0].amount), (u32::MAX, i64::MAX));
        assert_eq!((damage[1].cause, damage[1].amount), (1, i64::MIN));
        assert_eq!(damage[0].record, records[0].key);
        let key = records[0].key;
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::SELECTION_FLAG,
            &[1],
        );
        put(&mut bytes, FIRST + sair_wing_combat_data::COPIED_FLAG, &[1]);
        capture(&bytes, &mut records, &mut damage);
        assert_eq!(records[0].key, key);
        assert_eq!((records[0].selection, records[0].copied), (1, 1));
        // The record load path accepts a zero/default wing id. It remains a
        // stored historical value, not a live-wing lookup failure.
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::WING_ID,
            &0_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&bytes, &mut records, &mut damage).records,
            Outcome::Rows { len: 3 }
        );
        assert_eq!(records[0].key.wing_id, 0);
    }

    #[test]
    fn capacity_is_independent_and_counts_unread_rows_without_reading_their_fields() {
        let mut bytes = fixture();
        let mut records = [sair_wing_combat_data::Row::default(); 1];
        let mut damage = [sdamage_entry::Row::default(); 1];
        // Once both buffers are full, omitted inline records need only their
        // nested vector extents. Their unrelated fields are not sampled.
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::STRIDE,
            &0_u64.to_le_bytes(),
        );
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Truncated {
                    len: 1,
                    unread_rows: 2
                },
                damage: Outcome::Truncated {
                    len: 1,
                    unread_rows: 1
                },
            }
        );
        let bytes = fixture();
        let mut damage = [sdamage_entry::Row::default(); 2];
        assert_eq!(
            capture(&bytes, &mut [], &mut damage),
            Capture {
                records: Outcome::Truncated {
                    len: 0,
                    unread_rows: 3
                },
                damage: Outcome::Rows { len: 2 },
            }
        );
    }

    #[test]
    fn damage_fault_does_not_erase_good_record_rows_and_bad_keys_fail_both_tables() {
        let mut bytes = fixture();
        let mut records = [sair_wing_combat_data::Row::default(); 3];
        let mut damage = [sdamage_entry::Row::default(); 2];
        put(&mut bytes, DAMAGE, &0_u64.to_le_bytes());
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Rows { len: 3 },
                damage: Outcome::Unavailable {
                    reason: "air damage entry vtable mismatch"
                },
            }
        );
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::CLASSIFICATION_FLAG,
            &[2],
        );
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Unavailable {
                    reason: "air combat record flag is not boolean"
                },
                damage: Outcome::Unavailable {
                    reason: "air combat record flag is not boolean"
                },
            }
        );
    }

    #[test]
    fn empty_root_census_is_empty_and_malformed_vector_is_not() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, IMAGE_BASE, &[], &mut [], &mut []),
            Capture {
                records: Outcome::Rows { len: 0 },
                damage: Outcome::Rows { len: 0 },
            }
        );
        let mut bytes = bytes;
        put(
            &mut bytes,
            REGION + FIRST_RECORDS_COUNT,
            &3_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&bytes, &mut [], &mut []),
            Capture {
                records: Outcome::Unavailable {
                    reason: "air combat record vector unreadable or invalid"
                },
                damage: Outcome::Unavailable {
                    reason: "air combat record vector unreadable or invalid"
                },
            }
        );
    }

    #[test]
    fn growing_regional_history_and_damage_use_original_mod_sized_headers() {
        let mut bytes = fixture();
        let record = bytes
            [FIRST - MEMORY_BASE..FIRST - MEMORY_BASE + sair_wing_combat_data::STRIDE]
            .to_vec();
        let damage =
            bytes[DAMAGE - MEMORY_BASE..DAMAGE - MEMORY_BASE + sdamage_entry::STRIDE].to_vec();
        let record_data = 0x10000;
        let damage_data = 0x300000;
        let mut records = Vec::new();
        let mut damages = Vec::new();
        for count in [5_000_u32, 12_000] {
            bytes.resize(
                damage_data - MEMORY_BASE + count as usize * sdamage_entry::STRIDE,
                0,
            );
            vector(&mut bytes, REGION + FIRST_RECORDS, record_data, count);
            vector(&mut bytes, REGION + SECOND_RECORDS, 0, 0);
            for index in 0..count as usize {
                let at = record_data + index * sair_wing_combat_data::STRIDE;
                bytes[at - MEMORY_BASE..at - MEMORY_BASE + sair_wing_combat_data::STRIDE]
                    .copy_from_slice(&record);
                vector(
                    &mut bytes,
                    at + sair_wing_combat_data::FIRST_DAMAGE,
                    damage_data + index * sdamage_entry::STRIDE,
                    1,
                );
                let damage_at = damage_data + index * sdamage_entry::STRIDE;
                bytes[damage_at - MEMORY_BASE..damage_at - MEMORY_BASE + sdamage_entry::STRIDE]
                    .copy_from_slice(&damage);
            }
            let memory = SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            };
            let roots = [Root {
                country: 12,
                region: 35,
                address: REGION,
            }];
            assert_eq!(
                walk(&memory, IMAGE_BASE, &roots, &mut [], &mut []).records,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: count
                }
            );
            assert_eq!(
                walk_growing(&memory, IMAGE_BASE, &roots, &mut records, &mut damages),
                Capture {
                    records: Outcome::Rows {
                        len: count as usize
                    },
                    damage: Outcome::Rows {
                        len: count as usize
                    }
                }
            );
        }
    }
}
