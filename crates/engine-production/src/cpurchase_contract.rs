
use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::ccountry;
use crate::{cequipment_market_status, market_contract_manager::Contracts};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29e83d0);
pub const ID_TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const REGISTERED: usize = 0x10;
pub const TYPE: u32 = 81;
pub const SELLER: usize = 0x58;
pub const BUYER: usize = 0x5c;
pub const SPEED: usize = 0xc0;
pub const CONVOYS_ASSIGNED: usize = 0x118;
#[cfg(test)]
pub const CONVOYS_TOTAL: usize = 0x11c;
pub const EFFICIENCY: usize = 0x1c0;
pub const DAYS: usize = 0x1d0;
pub const FACTORY_PROGRESS: usize = 0x1e0;
pub const EQUIPMENT_PROGRESS: usize = 0x1e8;
#[cfg(test)]
pub const COLLECTED: usize = 0x1f0;
pub const COMPLETED: usize = 0x25c;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub seller: u32,
    pub buyer: u32,
    pub speed: i32,
    pub convoys_assigned: i32,
    pub convoys_total: i32,
    pub route_efficiency_sum_raw: i64,
    pub route_days: i32,
    pub factory_cic_progress_raw: i64,
    pub equipment_transfer_progress_raw: i64,
    pub collected_raw: i64,
    pub completed_deliveries: i32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Membership {
    pointer: usize,
    seller_slot: u32,
    buyer_slot: u32,
    matches: u32,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    scratch: &mut Vec<Membership>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(scratch),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    scratch: engine_native_memory::walk::Buffer<'_, Membership>,
) -> Outcome {
    match walk_inner(memory, base, game_state, output, scratch) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut scratch: engine_native_memory::walk::Buffer<'_, Membership>,
) -> Result<Outcome, &'static str> {
    let contracts = Contracts::read(memory, game_state)?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut corroborated = false;
    for slot in 1..countries
        .count()
        .min(contracts.sellers.count)
        .min(contracts.buyers.count)
    {
        if let Some(country) = countries.at(slot)? {
            cequipment_market_status::verify_manager(memory, country.pointer, contracts.pointer)?;
            corroborated = true;
            break;
        }
    }
    if !corroborated {
        return Err("market manager has no authenticated country backlink");
    }
    output.ensure(contracts.owned.count as usize)?;
    scratch.ensure(contracts.owned.count as usize)?;
    let len = (contracts.owned.count as usize)
        .min(output.len())
        .min(scratch.len());
    for (index, row) in output[..len].iter_mut().enumerate() {
        let pointer = contracts
            .owned
            .element(index as u32, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("owned market contract pointer unreadable or null")?;
        *row = read_row(memory, base, pointer)?;
        let seller = countries.resolve(row.seller)?;
        let buyer = countries.resolve(row.buyer)?;
        cequipment_market_status::verify_manager(memory, seller.pointer, contracts.pointer)?;
        cequipment_market_status::verify_manager(memory, buyer.pointer, contracts.pointer)?;
        scratch[index] = Membership {
            pointer,
            seller_slot: seller.canonical_slot,
            buyer_slot: buyer.canonical_slot,
            matches: 0,
        };
    }
    output[..len].sort_unstable_by_key(|row| row.id);
    if output[..len]
        .windows(2)
        .any(|pair| pair[0].id == pair[1].id)
    {
        return Err("duplicate owned market contract id");
    }
    verify_memberships(memory, contracts.sellers, &mut scratch[..len], true)?;
    verify_memberships(memory, contracts.buyers, &mut scratch[..len], false)?;
    let unread_rows = contracts.owned.count - len as u32;
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

fn verify_memberships(
    memory: &impl Memory,
    outer: engine_native_memory::cpdx_array::Array,
    scratch: &mut [Membership],
    sellers: bool,
) -> Result<(), &'static str> {
    let slot = |entry: &Membership| {
        if sellers {
            entry.seller_slot
        } else {
            entry.buyer_slot
        }
    };
    scratch.sort_unstable_by_key(|entry| (slot(entry), entry.pointer));
    let mut start = 0;
    while start < scratch.len() {
        let country = slot(&scratch[start]);
        let count = scratch[start..].partition_point(|entry| slot(entry) == country);
        let group = &mut scratch[start..start + count];
        for entry in group.iter_mut() {
            entry.matches = 0;
        }
        let rows = Contracts::country_rows(memory, outer, country)?;
        for index in 0..rows.count {
            let pointer = rows
                .element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("market country contract pointer unreadable")?;
            if let Ok(index) = group.binary_search_by_key(&pointer, |entry| entry.pointer) {
                group[index].matches += 1;
            }
        }
        if group.iter().any(|entry| entry.matches != 1) {
            return Err("market contract directional membership missing or duplicated");
        }
        start += count;
    }
    Ok(())
}

fn read_row(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    let header = read::<[u8; 16]>(memory, pointer).ok_or("market contract identity unreadable")?;
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
    if engine_sites::at(base, VTABLE_RVA).map(|p| p as u64) != Some(vtable) {
        return Err("market contract vtable mismatch");
    }
    let id = u64::from_le_bytes(header[8..].try_into().unwrap());
    if id as u32 != TYPE || (id >> 32) == 0 {
        return Err("market contract handle invalid");
    }
    let mut countries = [0; 8];
    let mut speed = [0; 4];
    let mut convoys = [0; 8];
    let mut efficiency = [0; 8];
    let mut days = [0; 4];
    let mut progress = [0; 24];
    let mut completed = [0; 4];
    let offsets = [
        SELLER,
        SPEED,
        CONVOYS_ASSIGNED,
        EFFICIENCY,
        DAYS,
        FACTORY_PROGRESS,
        COMPLETED,
    ];
    let addresses = offsets.map(|offset| pointer.checked_add(offset).unwrap_or(0));
    if memory.copy_scattered(
        addresses,
        &mut [
            &mut countries,
            &mut speed,
            &mut convoys,
            &mut efficiency,
            &mut days,
            &mut progress,
            &mut completed,
        ],
    ) != offsets.len()
    {
        return Err("market contract scalar unreadable");
    }
    Ok(Row {
        id,
        seller: u32::from_le_bytes(countries[..4].try_into().unwrap()),
        buyer: u32::from_le_bytes(countries[4..].try_into().unwrap()),
        speed: i32::from_le_bytes(speed),
        convoys_assigned: i32::from_le_bytes(convoys[..4].try_into().unwrap()),
        convoys_total: i32::from_le_bytes(convoys[4..].try_into().unwrap()),
        route_efficiency_sum_raw: i64::from_le_bytes(efficiency),
        route_days: i32::from_le_bytes(days),
        factory_cic_progress_raw: i64::from_le_bytes(progress[..8].try_into().unwrap()),
        equipment_transfer_progress_raw: i64::from_le_bytes(progress[8..16].try_into().unwrap()),
        collected_raw: i64::from_le_bytes(progress[16..].try_into().unwrap()),
        completed_deliveries: i32::from_le_bytes(completed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const GS: usize = 0x1000;
    const COUNTRY1: usize = 0x4000;
    const COUNTRY2: usize = 0x6000;
    const MANAGER: usize = 0x9000;
    const OWNED: usize = 0xa300;
    const SELLERS: usize = 0xa400;
    const BUYERS: usize = 0xa500;
    const CONTRACT1: usize = 0xb000;
    const CONTRACT2: usize = 0xc000;

    fn p32(bytes: &mut [u8], address: usize, value: u32) {
        bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], address: usize, value: u64) {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn header(bytes: &mut [u8], address: usize, data: usize, count: u32) {
        p64(bytes, address, data as u64);
        p32(bytes, address + 8, count);
        p32(bytes, address + 12, count);
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xd000];
        header(&mut bytes, GS + cgame_state::COUNTRIES, 0x3000, 5);
        header(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100, 5);
        for (slot, pointer) in [(1, COUNTRY1), (2, COUNTRY2), (4, COUNTRY1)] {
            p64(&mut bytes, 0x3000 + slot * 8, pointer as u64);
            p32(
                &mut bytes,
                0x3100 + slot * 4,
                if slot == 4 { 1 } else { slot as u32 },
            );
        }
        for (pointer, id, status) in [(COUNTRY1, 4, 0x8000), (COUNTRY2, 2, 0x8100)] {
            p64(&mut bytes, pointer, ccountry::VTABLE_RVA.0 as u64);
            p32(&mut bytes, pointer + ccountry::ID, id);
            p64(
                &mut bytes,
                pointer + ccountry::EQUIPMENT_MARKET_STATUS,
                status,
            );
            p64(
                &mut bytes,
                status as usize + cequipment_market_status::MANAGER,
                MANAGER as u64,
            );
        }
        p64(&mut bytes, GS + cgame_state::MARKET_MANAGER, MANAGER as u64);
        header(&mut bytes, MANAGER, OWNED, 2);
        header(&mut bytes, MANAGER + 0x30, 0xa100, 3);
        header(&mut bytes, MANAGER + 0x48, 0xa200, 3);
        header(&mut bytes, 0xa100 + 0x18, SELLERS, 2);
        header(&mut bytes, 0xa200 + 2 * 0x18, BUYERS, 2);
        for (index, pointer) in [CONTRACT1, CONTRACT2].into_iter().enumerate() {
            for vector in [OWNED, SELLERS, BUYERS] {
                p64(&mut bytes, vector + index * 8, pointer as u64);
            }
            p64(&mut bytes, pointer, VTABLE_RVA.0 as u64);
            p64(
                &mut bytes,
                pointer + ID_TYPE,
                (((2 - index) as u64) << 32) | u64::from(TYPE),
            );
            p32(&mut bytes, pointer + SELLER, 4);
            p32(&mut bytes, pointer + BUYER, 2);
            p32(&mut bytes, pointer + SPEED, 7);
            p32(&mut bytes, pointer + CONVOYS_ASSIGNED, 11);
            p32(&mut bytes, pointer + CONVOYS_TOTAL, 13);
            p64(&mut bytes, pointer + EFFICIENCY, 0x1_1234_5678);
            p32(&mut bytes, pointer + DAYS, 25);
            p64(&mut bytes, pointer + FACTORY_PROGRESS, i64::MAX as u64);
            p64(
                &mut bytes,
                pointer + EQUIPMENT_PROGRESS,
                (-123_456_789_012_i64) as u64,
            );
            p64(&mut bytes, pointer + COLLECTED, 0x2_1234_5678);
            p32(&mut bytes, pointer + COMPLETED, 2);
        }
        bytes
    }
    fn run(bytes: &[u8], output: &mut Vec<Row>) -> Outcome {
        let mut scratch = vec![Membership::default(); 2];
        walk(&SliceMemory { base: 0, bytes }, 0, GS, output, &mut scratch)
    }

    #[test]
    fn native_progress_and_alias_country_references_are_lossless() {
        let bytes = fixture();
        let mut output = vec![Row::default(); 2];
        assert_eq!(run(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(
            output[0],
            Row {
                id: (1_u64 << 32) | u64::from(TYPE),
                seller: 4,
                buyer: 2,
                speed: 7,
                convoys_assigned: 11,
                convoys_total: 13,
                route_efficiency_sum_raw: 0x1_1234_5678,
                route_days: 25,
                factory_cic_progress_raw: i64::MAX,
                equipment_transfer_progress_raw: -123_456_789_012,
                collected_raw: 0x2_1234_5678,
                completed_deliveries: 2,
            }
        );
        assert!(output[0].id < output[1].id);
    }

    #[test]
    fn the_count_probe_reads_no_contract_fields_and_counts_every_row() {
        let mut bytes = fixture();
        p64(&mut bytes, CONTRACT2, 0);
        assert_eq!(
            count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert!(matches!(
            run(&bytes, &mut vec![Row::default(); 2]),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn empty_owner_census_requires_an_authenticated_manager() {
        let mut bytes = fixture();
        header(&mut bytes, MANAGER, 0, 0);
        assert_eq!(run(&bytes, &mut Vec::new()), Outcome::Rows { len: 0 });
        p64(&mut bytes, 0x8000 + cequipment_market_status::MANAGER, 1);
        assert!(matches!(run(&bytes, &mut Vec::new()), Outcome::Unavailable { .. }));
        let mut bytes = fixture();
        p32(&mut bytes, MANAGER + 12, u32::MAX);
        assert!(matches!(run(&bytes, &mut Vec::new()), Outcome::Unavailable { .. }));
    }

    #[test]
    fn corrupt_ownership_identity_and_scalar_faults_never_publish_a_successful_prefix() {
        for (address, value) in [
            (OWNED, 0),
            (CONTRACT2, 0),
            (BUYERS + 8, CONTRACT1 as u64),
            (CONTRACT2 + ID_TYPE, (2_u64 << 32) | u64::from(TYPE)),
            (CONTRACT2 + ID_TYPE, 1_u64 << 32),
            (0x8100 + cequipment_market_status::MANAGER, 1),
        ] {
            let mut bytes = fixture();
            p64(&mut bytes, address, value);
            assert!(
                matches!(
                    run(&bytes, &mut vec![Row::default(); 2]),
                    Outcome::Unavailable { .. }
                ),
                "accepted corruption at {address:x}"
            );
        }
        let bytes = fixture();
        assert!(matches!(
            run(
                &bytes[..CONTRACT2 + COMPLETED + 3],
                &mut vec![Row::default(); 2]
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn high_degree_endpoints_are_scanned_once_and_preserve_membership_failures() {
        use std::cell::Cell;
        const COUNT: usize = 64;
        const OWNERS: usize = 0x20000;
        const SELLER_ROWS: usize = 0x21000;
        const BUYER_ROWS: usize = 0x22000;
        struct Counted<'a> {
            memory: SliceMemory<'a>,
            directional_reads: Cell<usize>,
            fault: Option<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if (SELLER_ROWS..SELLER_ROWS + COUNT * 8).contains(&address)
                    || (BUYER_ROWS..BUYER_ROWS + COUNT * 8).contains(&address)
                {
                    self.directional_reads.set(self.directional_reads.get() + 1);
                }
                if self.fault == Some(address) {
                    return false;
                }
                self.memory.copy(address, destination)
            }
        }
        let mut bytes = fixture();
        let template = bytes[CONTRACT1..CONTRACT1 + 0x300].to_vec();
        bytes.resize(0x40000, 0);
        header(&mut bytes, MANAGER, OWNERS, COUNT as u32);
        header(&mut bytes, 0xa100 + 0x18, SELLER_ROWS, COUNT as u32);
        header(&mut bytes, 0xa200 + 2 * 0x18, BUYER_ROWS, COUNT as u32);
        for index in 0..COUNT {
            let pointer = 0x30000 + index * 0x300;
            bytes[pointer..pointer + template.len()].copy_from_slice(&template);
            p64(
                &mut bytes,
                pointer + ID_TYPE,
                ((index as u64 + 1) << 32) | u64::from(TYPE),
            );
            for vector in [OWNERS, SELLER_ROWS, BUYER_ROWS] {
                p64(&mut bytes, vector + index * 8, pointer as u64);
            }
        }
        let mut output = vec![Row::default(); COUNT];
        let mut scratch = vec![Membership::default(); COUNT];
        let counted = Counted {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            directional_reads: Cell::new(0),
            fault: None,
        };
        assert_eq!(
            walk(&counted, 0, GS, &mut output, &mut scratch),
            Outcome::Rows { len: COUNT }
        );
        assert_eq!(counted.directional_reads.get(), COUNT * 2);
        assert_eq!(output[COUNT - 1].id >> 32, COUNT as u64);
        let fault = Counted {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            directional_reads: Cell::new(0),
            fault: Some(BUYER_ROWS + (COUNT - 1) * 8),
        };
        assert_eq!(
            walk(&fault, 0, GS, &mut output, &mut scratch),
            Outcome::Unavailable {
                reason: "market country contract pointer unreadable"
            }
        );
        p64(&mut bytes, BUYER_ROWS + (COUNT - 1) * 8, 0x30000);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, GS, &mut output, &mut scratch),
            Outcome::Unavailable {
                reason: "market contract directional membership missing or duplicated"
            }
        );
    }

    #[test]
    fn membership_scratch_grows_and_unrelated_index_entries_remain_ignored() {
        let mut bytes = fixture();
        let mut output = vec![Row::default(); 2];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, GS, &mut output, &mut vec![Membership::default(); 1]),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(count(&memory, 0, GS), Outcome::Truncated { len: 0, unread_rows: 2 });
        header(&mut bytes, 0xa100 + 0x18, SELLERS, 3);
        p64(&mut bytes, SELLERS + 16, 0xdead_beef);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut scratch = vec![Membership::default(); 2];
        assert_eq!(
            walk(&memory, 0, GS, &mut output, &mut scratch),
            Outcome::Rows { len: 2 }
        );
        p64(&mut bytes, OWNED + 8, CONTRACT1 as u64);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, GS, &mut output, &mut scratch),
            Outcome::Unavailable {
                reason: "duplicate owned market contract id"
            }
        );
    }
}
