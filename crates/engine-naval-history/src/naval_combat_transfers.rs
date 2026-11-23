
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::{cgame_state, cland_combat, cnaval_combat};
use engine_native_parts::{cland_border_war_combat, creference_registry::HeaderResolver};
use crate::cnaval_unit_transfer;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CombatTransfers {
    pub combat: u64,
    pub transfers: [usize; 2],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransferUnits {
    pub transfer: u64,
    pub units: [usize; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub combats: Outcome,
    pub transfers: Outcome,
    pub transfer_handles_required: usize,
    pub unit_handles_required: usize,
}

const UNIT_REFERENCE_BASE: usize = 0x10;

fn handles(memory: &impl Memory, pointer: usize, offset: usize) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("naval transfer handle vector invalid")
}

fn for_each_handle(
    memory: &impl Memory,
    array: Array,
    mut visit: impl FnMut(u32, u64) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    const CHUNK: u32 = 64;
    let mut bytes = [0u8; CHUNK as usize * 8];
    let mut first = 0;
    while first < array.count {
        let n = (array.count - first).min(CHUNK);
        let address = array
            .element(first, 8)
            .ok_or("naval transfer slot unreadable")?;
        if !engine_native_memory::read_slice(memory, address, &mut bytes[..n as usize * 8]) {
            return Err("naval transfer slot unreadable");
        }
        for (index, word) in bytes[..n as usize * 8].chunks_exact(8).enumerate() {
            visit(
                first + index as u32,
                u64::from_le_bytes(word.try_into().expect("eight bytes")),
            )?;
        }
        first += n;
    }
    Ok(())
}

fn outcome(len: usize, missing: usize) -> Outcome {
    if missing == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: u32::try_from(missing).unwrap_or(u32::MAX),
        }
    }
}

pub struct Buffers<'a> {
    pub combats: &'a mut Vec<CombatTransfers>,
    pub transfer_handles: &'a mut Vec<u64>,
    pub transfers: &'a mut Vec<TransferUnits>,
    pub unit_handles: &'a mut Vec<u64>,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    buffers: Buffers<'_>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Growing(buffers.combats),
        engine_native_memory::walk::Buffer::Growing(buffers.transfer_handles),
        engine_native_memory::walk::Buffer::Growing(buffers.transfers),
        engine_native_memory::walk::Buffer::Growing(buffers.unit_handles),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut combats: engine_native_memory::walk::Buffer<'_, CombatTransfers>,
    mut transfer_handles: engine_native_memory::walk::Buffer<'_, u64>,
    mut transfers: engine_native_memory::walk::Buffer<'_, TransferUnits>,
    mut unit_handles: engine_native_memory::walk::Buffer<'_, u64>,
) -> Walk {
    let (mut combat_len, mut combat_missing, mut transfer_used) = (0usize, 0usize, 0usize);
    let (mut transfer_len, mut transfer_missing, mut unit_used) = (0usize, 0usize, 0usize);
    let (mut transfer_required, mut unit_required) = (0usize, 0usize);
    let result = (|| {
        let registry = handles(memory, state, cgame_state::COMBATS + 8)?;
        let expected = |key: Rva| engine_sites::at(base, key).map(|address| address as u64);
        let naval = expected(cnaval_combat::VTABLE_RVA).ok_or("module address overflow")?;
        let land = [
            expected(cland_combat::VTABLE_RVA),
            expected(cland_border_war_combat::VTABLE_RVA),
        ];
        let transfer_vtable =
            expected(cnaval_unit_transfer::VTABLE_RVA).ok_or("module address overflow")?;
        for_each_handle(memory, registry, |_, combat| {
            let combat = usize::try_from(combat)
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("naval transfer combat pointer null")?;
            let header =
                read::<[u8; 0x20]>(memory, combat).ok_or("naval transfer combat unreadable")?;
            let class = u64::from_le_bytes(header[..8].try_into().expect("eight bytes"));
            if class != naval {
                if land.contains(&Some(class)) {
                    return Ok(());
                }
                return Err("naval transfer combat class unknown");
            }
            let handle = u64::from_le_bytes(
                header[cnaval_combat::HANDLE..][..8]
                    .try_into()
                    .expect("eight bytes"),
            );
            if handle == 0 {
                return Err("naval transfer combat handle absent");
            }
            let vector = handles(memory, combat, cnaval_combat::PARTICIPANTS)?;
            if vector.count == 0 {
                return Ok(());
            }
            let count = vector.count as usize;
            transfer_required = transfer_required
                .checked_add(count)
                .ok_or("naval transfer demand overflow")?;
            let start = transfer_used;
            combats.ensure(
                combat_len
                    .checked_add(1)
                    .ok_or("naval transfer combat demand overflow")?,
            )?;
            transfer_handles.ensure(
                start
                    .checked_add(count)
                    .ok_or("naval transfer demand overflow")?,
            )?;
            if combat_len < combats.len() && start + count <= transfer_handles.len() {
                for_each_handle(memory, vector, |entry, transfer| {
                    transfer_handles[start + entry as usize] = transfer;
                    Ok(())
                })?;
                transfer_used += count;
                combats[combat_len] = CombatTransfers {
                    combat: handle,
                    transfers: [start, count],
                };
                combat_len += 1;
            } else {
                combat_missing += 1;
            }
            Ok(())
        })?;
        if combat_missing > 0 {
            transfer_missing = transfer_required.max(1);
            return Ok(());
        }
        let named = transfer_handles[..transfer_used]
            .iter()
            .filter(|&&handle| handle != 0)
            .count();
        transfers.ensure(named)?;
        if named > transfers.len() {
            transfer_missing = named;
            return Ok(());
        }
        let mut distinct = 0;
        for &handle in transfer_handles[..transfer_used]
            .iter()
            .filter(|&&handle| handle != 0)
        {
            transfers[distinct] = TransferUnits {
                transfer: handle,
                units: [0; 2],
            };
            distinct += 1;
        }
        transfers[..distinct].sort_unstable_by_key(|row| row.transfer);
        let mut kept = 0;
        for index in 0..distinct {
            if kept == 0 || transfers[kept - 1].transfer != transfers[index].transfer {
                transfers[kept] = transfers[index];
                kept += 1;
            }
        }
        let mut resolver = HeaderResolver::default();
        for index in 0..kept {
            let transfer = transfers[index].transfer;
            let Some(pointer) = resolver.resolve(memory, base, transfer)? else {
                continue;
            };
            let identity =
                read::<[u8; 0x10]>(memory, pointer).ok_or("naval transfer identity unreadable")?;
            if u64::from_le_bytes(identity[..8].try_into().expect("eight bytes")) != transfer_vtable
                || u64::from_le_bytes(
                    identity[cnaval_unit_transfer::HANDLE..][..8]
                        .try_into()
                        .expect("eight bytes"),
                ) != transfer
            {
                return Err("naval transfer identity mismatch");
            }
            let units = handles(memory, pointer, cnaval_unit_transfer::UNITS)?;
            let start = unit_used;
            unit_handles.ensure(
                start
                    .checked_add(units.count as usize)
                    .ok_or("naval transfer unit demand overflow")?,
            )?;
            let mut live = 0usize;
            for_each_handle(memory, units, |_, unit| {
                let resolved = resolver.resolve(memory, base, unit)?;
                if unit == 0 || resolved.is_none_or(|object| object == UNIT_REFERENCE_BASE) {
                    return Ok(());
                }
                if start + live < unit_handles.len() {
                    unit_handles[start + live] = unit;
                }
                live += 1;
                Ok(())
            })?;
            unit_required = unit_required
                .checked_add(live)
                .ok_or("naval transfer unit demand overflow")?;
            if start + live <= unit_handles.len() {
                unit_used += live;
                transfers[transfer_len] = TransferUnits {
                    transfer,
                    units: [start, live],
                };
                transfer_len += 1;
            } else {
                transfer_missing += 1;
            }
        }
        Ok(())
    })();
    let (combats, transfers) = match result {
        Ok(()) => (
            outcome(combat_len, combat_missing),
            outcome(transfer_len, transfer_missing),
        ),
        Err(reason) => (
            Outcome::Unavailable { reason },
            Outcome::Unavailable { reason },
        ),
    };
    Walk {
        combats,
        transfers,
        transfer_handles_required: transfer_required,
        unit_handles_required: unit_required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    use engine_native_memory::memory::SliceMemory;

    const STATE: usize = 0x1000;
    const LAND: usize = 0x2400;
    const TABLE: usize = 0x3000;
    const BUCKETS: usize = 0x3100;
    const KIND: u64 = 4713;

    fn put(bytes: &mut Vec<u8>, at: usize, value: &[u8]) {
        if bytes.len() < at + value.len() {
            bytes.resize(at + value.len(), 0);
        }
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut Vec<u8>, at: usize, data: usize, count: u32) {
        put(bytes, at, &(data as u64).to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }

    fn handle(id: u64) -> u64 {
        (id << 32) | KIND
    }

    struct Image<'a>(SliceMemory<'a>);

    impl Memory for Image<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == registry::HIGH_ROOT_RVA.0 {
                destination.copy_from_slice(&(TABLE as u64).to_le_bytes()[..destination.len()]);
                return true;
            }
            if address >= registry::HIGH_ROOT_RVA.0 {
                destination.fill(0);
                return true;
            }
            self.0.copy(address, destination)
        }
    }

    fn install(bytes: &mut Vec<u8>, objects: &[(u64, u64)]) -> usize {
        let capacity = (objects.len() * 3).max(8).next_power_of_two();
        let exponent = capacity.trailing_zeros();
        let overflow = exponent + 2;
        let cells = capacity + overflow as usize + 1;
        let mut table = vec![(0u32, 0u64, 0u64); cells];
        for &(key, object) in objects {
            let hash = key.rotate_left(32).wrapping_mul(registry::HASH_MULTIPLIER);
            let mut index = (((hash >> 32) as u32 ^ hash as u32) as usize) & (capacity - 1);
            let mut entry = (1u32, key, object);
            loop {
                assert!(
                    entry.0 <= overflow + 1 && index < cells - 1,
                    "table too dense"
                );
                if table[index].0 == 0 {
                    table[index] = entry;
                    break;
                }
                if table[index].0 < entry.0 {
                    std::mem::swap(&mut table[index], &mut entry);
                }
                index += 1;
                entry.0 += 1;
            }
        }
        put(
            bytes,
            TABLE + registry::ENTRIES,
            &(BUCKETS as u64).to_le_bytes(),
        );
        put(
            bytes,
            TABLE + registry::COUNT,
            &(objects.len() as u32).to_le_bytes(),
        );
        put(
            bytes,
            TABLE + registry::MASK,
            &(capacity as u32 - 1).to_le_bytes(),
        );
        put(
            bytes,
            TABLE + registry::OVERFLOW_DISTANCE,
            &[overflow as u8],
        );
        for (index, &(distance, key, object)) in table.iter().enumerate() {
            let at = BUCKETS + index * bucket::STRIDE;
            put(bytes, at, &[0; bucket::STRIDE]);
            put(bytes, at + bucket::DISTANCE, &[distance as u8]);
            put(bytes, at + bucket::TYPE, &key.to_le_bytes());
            put(bytes, at + bucket::OBJECT, &object.to_le_bytes());
        }
        put(
            bytes,
            BUCKETS + (cells - 1) * bucket::STRIDE + bucket::DISTANCE,
            &[bucket::TERMINAL_DISTANCE],
        );
        BUCKETS + cells * bucket::STRIDE
    }

    struct Fixture {
        bytes: Vec<u8>,
    }

    impl Fixture {
        fn new(
            combats: &[Vec<u64>],
            units: &[(u64, Vec<u64>)],
            live_units: &[u64],
            null_units: &[u64],
        ) -> Self {
            let mut bytes = vec![0; 0x3000];
            let mut objects = Vec::new();
            let mut heap = 0x10_0000usize;
            let mut next = |bytes: &mut Vec<u8>, size: usize| {
                let at = heap;
                heap += size.next_multiple_of(0x10);
                put(bytes, at + size.max(1) - 1, &[0]);
                at
            };
            let mut transfer_objects = Vec::new();
            for (transfer, unit_handles) in units {
                let object = next(&mut bytes, 0x50);
                let slots = next(&mut bytes, unit_handles.len() * 8);
                put(
                    &mut bytes,
                    object,
                    &(cnaval_unit_transfer::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    object + cnaval_unit_transfer::HANDLE,
                    &transfer.to_le_bytes(),
                );
                array(
                    &mut bytes,
                    object + cnaval_unit_transfer::UNITS,
                    slots,
                    unit_handles.len() as u32,
                );
                for (index, unit) in unit_handles.iter().enumerate() {
                    put(&mut bytes, slots + index * 8, &unit.to_le_bytes());
                }
                transfer_objects.push((*transfer, object as u64));
            }
            objects.extend(transfer_objects);
            objects.extend(live_units.iter().map(|&unit| (unit, 0x20_0010)));
            objects.extend(null_units.iter().map(|&unit| (unit, 0x10)));
            let registry_slots = next(&mut bytes, (combats.len() + 1) * 8);
            array(
                &mut bytes,
                STATE + cgame_state::COMBATS + 8,
                registry_slots,
                combats.len() as u32 + 1,
            );
            put(&mut bytes, registry_slots, &(LAND as u64).to_le_bytes());
            put(
                &mut bytes,
                LAND,
                &(cland_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            for (index, transfers) in combats.iter().enumerate() {
                let combat = next(&mut bytes, 0x100);
                let slots = next(&mut bytes, transfers.len() * 8);
                put(
                    &mut bytes,
                    registry_slots + (index + 1) * 8,
                    &(combat as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    combat,
                    &(cnaval_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    combat + cnaval_combat::HANDLE,
                    &handle(900_000 + index as u64).to_le_bytes(),
                );
                array(
                    &mut bytes,
                    combat + cnaval_combat::PARTICIPANTS,
                    slots,
                    transfers.len() as u32,
                );
                for (entry, transfer) in transfers.iter().enumerate() {
                    put(&mut bytes, slots + entry * 8, &transfer.to_le_bytes());
                }
            }
            install(&mut bytes, &objects);
            Self { bytes }
        }

        fn walk(
            &self,
            capacity: [usize; 4],
        ) -> (
            Walk,
            Vec<CombatTransfers>,
            Vec<u64>,
            Vec<TransferUnits>,
            Vec<u64>,
        ) {
            let memory = Image(SliceMemory {
                base: 0,
                bytes: &self.bytes,
            });
            self.walk_on(&memory, capacity)
        }

        fn walk_on(
            &self,
            memory: &impl Memory,
            capacity: [usize; 4],
        ) -> (
            Walk,
            Vec<CombatTransfers>,
            Vec<u64>,
            Vec<TransferUnits>,
            Vec<u64>,
        ) {
            let mut combats = vec![CombatTransfers::default(); capacity[0]];
            let mut transfer_handles = vec![0; capacity[1]];
            let mut transfers = vec![TransferUnits::default(); capacity[2]];
            let mut unit_handles = vec![0; capacity[3]];
            let walked = walk(
                memory,
                0,
                STATE,
                Buffers {
                    combats: &mut combats,
                    transfer_handles: &mut transfer_handles,
                    transfers: &mut transfers,
                    unit_handles: &mut unit_handles,
                },
            );
            (walked, combats, transfer_handles, transfers, unit_handles)
        }
    }

    #[test]
    fn vectors_keep_native_order_and_units_keep_only_live_handles() {
        let (live, stale) = (handle(1), handle(2));
        let (unit, gone, null) = (handle(10), handle(11), handle(12));
        let fixture = Fixture::new(
            &[vec![live, 0, stale, live], vec![live]],
            &[(live, vec![gone, unit, null, 0])],
            &[unit],
            &[null],
        );
        let (walked, combats, handles, transfers, units) = fixture.walk([2, 5, 4, 4]);
        assert_eq!(walked.combats, Outcome::Rows { len: 2 });
        assert_eq!(walked.transfer_handles_required, 5);
        assert_eq!(combats[0].transfers, [0, 4]);
        assert_eq!(handles, [live, 0, stale, live, live]);
        assert_eq!(walked.transfers, Outcome::Rows { len: 1 });
        assert_eq!(
            transfers[0],
            TransferUnits {
                transfer: live,
                units: [0, 1]
            }
        );
        assert_eq!(units[0], unit);
        assert_eq!(walked.unit_handles_required, 1);
        let (short, ..) = fixture.walk([2, 4, 3, 4]);
        assert_eq!(short.combats, Outcome::Rows { len: 2 });
        assert_eq!(short.transfers, Outcome::Rows { len: 1 });
    }

    #[test]
    fn growing_transfers_keep_full_original_vectors_and_distinct_scratch_before_publication() {
        let transfer = handle(1);
        let unit = handle(2);
        let mut combats = Vec::new();
        let mut named = Vec::new();
        let mut transfers = Vec::new();
        let mut units = Vec::new();
        for count in [5_000_usize, 12_000] {
            let fixture = Fixture::new(
                &vec![vec![transfer, 0, transfer]; count],
                &[(transfer, vec![unit; count])],
                &[unit],
                &[],
            );
            let memory = Image(SliceMemory {
                base: 0,
                bytes: &fixture.bytes,
            });
            let capture = walk(
                &memory,
                0,
                STATE,
                Buffers {
                    combats: &mut combats,
                    transfer_handles: &mut named,
                    transfers: &mut transfers,
                    unit_handles: &mut units,
                },
            );
            assert_eq!(capture.combats, Outcome::Rows { len: count });
            assert_eq!(capture.transfers, Outcome::Rows { len: 1 });
            assert_eq!(capture.transfer_handles_required, count * 3);
            assert_eq!(capture.unit_handles_required, count);
            assert!(transfers.len() >= count * 2);
            assert_eq!(combats[count - 1].transfers, [(count - 1) * 3, 3]);
            assert_eq!(transfers[0].units, [0, count]);
            assert!(units[..count].iter().all(|&value| value == unit));
        }
    }

    #[test]
    fn many_transfers_cost_reads_linear_in_their_count() {
        let mut per_transfer = None;
        for count in [1_500u64, 5_000] {
            let transfers: Vec<u64> = (0..count).map(|id| handle(100_000 + id)).collect();
            let units: Vec<(u64, Vec<u64>)> = transfers
                .iter()
                .enumerate()
                .map(|(index, &transfer)| (transfer, vec![handle(200_000 + index as u64)]))
                .collect();
            let live: Vec<u64> = units.iter().map(|(_, units)| units[0]).collect();
            let fixture = Fixture::new(&[transfers.clone(), transfers.clone()], &units, &live, &[]);
            let memory = Image(SliceMemory {
                base: 0,
                bytes: &fixture.bytes,
            });
            let census = engine_native_memory::read_census::ReadCensus::new(&memory);
            let n = count as usize;
            let started = std::time::Instant::now();
            let (walked, _, _, rows, _) = fixture.walk_on(&census, [2, 2 * n, 2 * n, n]);
            println!(
                "{count} transfers named twice: {} reads, {:.3} ms",
                census.calls(),
                started.elapsed().as_secs_f64() * 1e3
            );
            assert_eq!(walked.transfers, Outcome::Rows { len: n });
            assert!(
                rows[..n]
                    .windows(2)
                    .all(|pair| pair[0].transfer < pair[1].transfer)
            );
            let reads = census.calls() as f64 / count as f64;
            assert!(reads < 12.0, "{reads} reads per transfer at {count}");
            if let Some(previous) = per_transfer {
                assert!(reads <= previous * 1.1, "{previous} -> {reads}");
            }
            per_transfer = Some(reads);
        }
    }
}
