use super::*;
use std::{cell::Cell, collections::BTreeMap};

#[derive(Default)]
struct MemoryMap {
    spans: BTreeMap<usize, Vec<u8>>,
    reads: Cell<usize>,
    log: std::cell::RefCell<Vec<(usize, usize)>>,
}
impl Memory for MemoryMap {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.set(self.reads.get() + 1);
        self.log.borrow_mut().push((address, destination.len()));
        self.span_copy(address, destination)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.reads.set(self.reads.get() + 1);
        self.log
            .borrow_mut()
            .push((addresses[0], destinations.iter().map(|d| d.len()).sum()));
        for (copied, (address, destination)) in addresses.into_iter().zip(destinations).enumerate()
        {
            if !destination.is_empty() && !self.span_copy(address, destination) {
                return copied;
            }
        }
        N
    }
}
impl MemoryMap {
    fn span_copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some((&start, bytes)) = self.spans.range(..=address).next_back() else {
            return false;
        };
        let Some(end) = (address - start).checked_add(destination.len()) else {
            return false;
        };
        let Some(source) = bytes.get(address - start..end) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
    fn span(&mut self, address: usize, len: usize) {
        self.spans.insert(address, vec![0; len]);
    }
    fn put(&mut self, address: usize, value: &[u8]) {
        let (&start, bytes) = self.spans.range_mut(..=address).next_back().unwrap();
        bytes[address - start..address - start + value.len()].copy_from_slice(value);
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }
}
const STATE: usize = 0x100000;
const COUNTRIES: usize = 0x200000;
const LINKS: usize = 0x300000;
const COUNTRY: usize = 0x10000000;
const STATUS: usize = 0x20000000;
const CONSUMERS: usize = 0x30000000;
fn fixture(count: u32) -> MemoryMap {
    let mut memory = MemoryMap::default();
    memory.span(cgame_state::SINGLETON_RVA.0, 8);
    memory.put(cgame_state::SINGLETON_RVA.0, &(STATE as u64).to_le_bytes());
    memory.span(STATE, 0x800);
    memory.span(COUNTRIES, (count as usize + 1) * 8);
    memory.span(LINKS, (count as usize + 2) * 4);
    memory.array(STATE + cgame_state::COUNTRIES, COUNTRIES, count + 1);
    memory.array(STATE + cgame_state::COUNTRY_LINKS, LINKS, count + 1);
    for slot in 1..=count {
        let country = COUNTRY + slot as usize * 0x2000;
        let status = STATUS + slot as usize * 0x1000;
        let consumers = CONSUMERS + slot as usize * 0x100;
        memory.span(country, 0x1590);
        memory.span(status, 0xe8);
        memory.span(consumers, 3 * crate::fuel_consumer_data::STRIDE);
        memory.put(
            COUNTRIES + slot as usize * 8,
            &(country as u64).to_le_bytes(),
        );
        memory.put(LINKS + slot as usize * 4, &slot.to_le_bytes());
        memory.put(country, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(country + ccountry::ID, &slot.to_le_bytes());
        memory.put(
            country + ccountry::SURRENDER_TARGET,
            &(slot + 9000).to_le_bytes(),
        );
        memory.put(
            country + ccountry::FUEL_STATUS,
            &(status as u64).to_le_bytes(),
        );
        memory.put(status, &(fuel::VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(status + fuel::COUNTRY, &(country as u64).to_le_bytes());
        memory.put(status + fuel::GAIN, &(-17_i64).to_le_bytes());
        memory.put(status + fuel::GAIN_FROM_STATES, &23_i64.to_le_bytes());
        memory.put(status + fuel::COST, &(-5_i64).to_le_bytes());
        memory.put(status + 0x68, &i64::MAX.to_le_bytes());
        memory.put(status + fuel::REMAINING_HOURS, &7_i32.to_le_bytes());
        memory.array(status + fuel::CONSUMERS, consumers, 3);
    }
    memory
}

struct Holes<'a> {
    inner: &'a MemoryMap,
    holes: [std::ops::Range<usize>; 2],
}
impl Memory for Holes<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let end = address + destination.len();
        !self
            .holes
            .iter()
            .any(|hole| address < hole.end && hole.start < end)
            && self.inner.copy(address, destination)
    }
}

#[test]
fn start_and_settle_read_only_their_operand_fields() {
    let memory = fixture(1);
    let status = STATUS + 0x1000;
    let consumers = CONSUMERS + 0x100;
    let gaps = Holes {
        inner: &memory,
        holes: [
            status + 0x58..status + 0x60,
            consumers + 0x18..consumers + 0x28,
        ],
    };
    let started = start(&gaps, 0, status).unwrap();
    assert_eq!(start(&memory, 0, status), Ok(started));
    assert_eq!(
        settle(&gaps, 0, started, status),
        settle(&memory, 0, started, status)
    );
    assert!(settle(&gaps, 0, started, status).is_ok());
    let operand = Holes {
        inner: &memory,
        holes: [status + 0x70..status + 0x71, 0..0],
    };
    assert_eq!(
        start(&operand, 0, status),
        Err("fuel award operand unreadable or overflowed")
    );
}

#[test]
fn applied_acquisition_preserves_wrap_zero_and_loaded_pending() {
    let mut memory = fixture(2);
    let received = [11_i64, -23, 37];
    for (index, value) in received.iter().enumerate() {
        memory.put(
            CONSUMERS
                + 0x100
                + index * crate::fuel_consumer_data::STRIDE
                + crate::fuel_consumer_data::RECEIVED,
            &value.to_le_bytes(),
        );
    }
    for slot in 1..=2 {
        let status = STATUS + slot * 0x1000;
        let begun = start(&memory, 0, status).unwrap();
        let amount = if slot == 1 { 0 } else { 31_i64 };
        memory.put(
            status + 0x68,
            &begun.pending_gain.wrapping_add(amount).to_le_bytes(),
        );
        let result = settle(&memory, 0, begun, status).unwrap();
        assert_eq!(result.own_gain, amount);
        assert_eq!(result.gains, [-17, 23, amount - 11, 5]);
        assert_eq!(result.received, if slot == 1 { received } else { [0; 3] });
        assert_eq!(begun.remaining_hours, 7);
    }
    assert!(memory.reads.get() < 2 * 120);
}
#[test]
fn country_replacement_and_changed_applied_fields_refuse_only_the_root() {
    let mut memory = fixture(2);
    let status = STATUS + 0x1000;
    let begun = start(&memory, 0, status).unwrap();
    memory.array(STATE + cgame_state::COUNTRY_LINKS, LINKS, 4);
    memory.put(LINKS + 3 * 4, &1_u32.to_le_bytes());
    memory.put(COUNTRY + 0x2000 + ccountry::ID, &3_u32.to_le_bytes());
    assert_eq!(
        settle(&memory, 0, begun, status),
        Err("fuel award root or applied operands changed")
    );
    let other = start(&memory, 0, STATUS + 0x2000).unwrap();
    assert!(settle(&memory, 0, other, STATUS + 0x2000).is_ok());
    let changed = start(&memory, 0, status).unwrap();
    assert_eq!((changed.country, changed.status), (3, begun.status));
    assert!(settle(&memory, 0, changed, status).is_ok());
    memory.put(status + fuel::GAIN, &100_i64.to_le_bytes());
    assert_eq!(
        settle(&memory, 0, changed, status),
        Err("fuel award root or applied operands changed")
    );
    assert!(settle(&memory, 0, other, STATUS + 0x2000).is_ok());
}
#[test]
fn native_retained_name_supports_heap_length_without_a_chosen_limit() {
    let mut memory = MemoryMap::default();
    let army = 0x1000;
    let member = 0x2000;
    let payload = 0x4000;
    memory.span(army, 0x348);
    memory.span(member, 0x80);
    memory.span(payload, 8193);
    memory.put(
        army + engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER,
        &(member as u64).to_le_bytes(),
    );
    memory.put(member + 0x60, &(payload as u64).to_le_bytes());
    memory.put(member + 0x70, &8192_u64.to_le_bytes());
    memory.put(member + 0x78, &8192_u64.to_le_bytes());
    let name = stored_name(&memory, army, 0).unwrap();
    let mut output = vec![1; 8192];
    name.copy(&memory, &mut output).unwrap();
    assert_eq!(output, vec![0; 8192]);
    memory.put(payload + 8192, &[1]);
    assert!(stored_name(&memory, army, 0).is_err());
}

#[test]
fn live_census_includes_null_air_countries_and_all_name_bytes() {
    let mut memory = fixture(3);
    let manager = 0x600000;
    let airs = 0x700000;
    memory.span(manager, 0x50);
    memory.span(airs, 4 * 8);
    memory.put(
        STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        &(manager as u64).to_le_bytes(),
    );
    memory.put(
        manager,
        &(cstrategic_air_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    memory.array(manager + cstrategic_air_manager::COUNTRY_AIR, airs, 4);
    for slot in 1..=3 {
        let country = COUNTRY + slot * 0x2000;
        let roster = 0x60000000 + slot * 0x100;
        let division = 0x70000000 + slot * 0x1000;
        let member = 0x80000000 + slot * 0x1000;
        memory.span(roster, 8);
        memory.span(division, 0x348);
        memory.span(member, 0x80);
        memory.array(country + ccountry::DIVISIONS, roster, 1);
        memory.put(roster, &(division as u64).to_le_bytes());
        memory.put(
            division + engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER,
            &(member as u64).to_le_bytes(),
        );
        memory.put(member + 0x60, b"recorded");
        memory.put(member + 0x70, &8_u64.to_le_bytes());
        memory.put(member + 0x78, &15_u64.to_le_bytes());
    }
    assert_eq!(
        demand(&memory, 0, STATE).unwrap(),
        Demand {
            entries: 6,
            names: 24,
            name_stride: 8
        }
    );
    assert_eq!(demand(&memory, 0, 0).unwrap(), Demand::default());
}

#[test]
fn a_held_start_keeps_the_country_table_controls() {
    let memory = fixture(1);
    let status = STATUS + 0x1000;
    let (_, located, _) = start_held(&memory, 0, status, None).unwrap();
    let header = STATE + cgame_state::COUNTRIES;
    let faulted = Holes {
        inner: &memory,
        holes: [header..header + 16, 0..0],
    };
    let fresh = start_held(&faulted, 0, status, None).map(|(started, _, _)| started);
    assert!(fresh.is_err());
    assert_eq!(
        start_held(&faulted, 0, status, Some(&located)).map(|(started, _, _)| started),
        fresh
    );
}
