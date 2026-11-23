
use super::RawLog;
use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::ccountry;

pub struct CountrySlots {
    countries: Array,
    links: Array,
    vtable: u64,
    reserved: Option<usize>,
}

impl CountrySlots {
    pub fn new(countries: Array, links: Array, base: usize) -> Option<Self> {
        if countries.count < 2 || links.count == 0 {
            return None;
        }
        Some(Self {
            countries,
            links,
            vtable: engine_sites::at(base, ccountry::VTABLE_RVA)? as u64,
            reserved: None,
        })
    }

    pub fn count(&self) -> u32 {
        self.countries.count
    }

    pub fn links(&self) -> Array {
        self.links
    }

    pub fn resolve<const N: usize>(
        &self,
        log: &mut RawLog<N>,
        memory: &impl Memory,
        raw: u32,
        mask: u32,
    ) -> Option<()> {
        let slot = self.canonical(log, memory, raw, mask)?;
        let pointer = self.pointer(log, memory, slot, mask)?;
        if pointer == 0 || u64_at(log, memory, Some(pointer), mask)? != self.vtable {
            return None;
        }
        let id = u32_at(log, memory, pointer.checked_add(ccountry::ID), mask)?;
        let canonical = self.canonical(log, memory, id, mask)?;
        if self.canonical(log, memory, slot, mask)? != canonical
            || self.canonical(log, memory, canonical, mask)? != canonical
        {
            return None;
        }
        self.pointer(log, memory, canonical, mask)?;
        Some(())
    }

    pub fn take<const N: usize>(
        &mut self,
        log: &mut RawLog<N>,
        memory: &impl Memory,
        slot: u32,
        mask: u32,
    ) -> Option<Option<usize>> {
        let pointer = self.pointer(log, memory, slot, mask)?;
        if pointer == 0 || self.reserved == Some(pointer) {
            return Some(None);
        }
        let vtable = u64_at(log, memory, Some(pointer), mask)?;
        let id = u32_at(log, memory, pointer.checked_add(ccountry::ID), mask)?;
        if vtable != self.vtable {
            return None;
        }
        if id == 0 {
            let root = self.pointer(log, memory, 0, mask)?;
            if root == 0
                || u64_at(log, memory, Some(root), mask)? != self.vtable
                || u32_at(log, memory, root.checked_add(ccountry::ID), mask)? != 0
                || root != pointer
            {
                return None;
            }
            self.reserved = Some(root);
            return Some(None);
        }
        let canonical = self.canonical(log, memory, id, mask)?;
        if self.canonical(log, memory, slot, mask)? != canonical
            || self.canonical(log, memory, canonical, mask)? != canonical
            || self.pointer(log, memory, canonical, mask)? != pointer
        {
            return None;
        }
        Some((slot == canonical).then_some(pointer))
    }

    fn pointer<const N: usize>(
        &self,
        log: &mut RawLog<N>,
        memory: &impl Memory,
        slot: u32,
        mask: u32,
    ) -> Option<usize> {
        usize::try_from(u64_at(log, memory, self.countries.element(slot, 8), mask)?).ok()
    }

    fn canonical<const N: usize>(
        &self,
        log: &mut RawLog<N>,
        memory: &impl Memory,
        raw: u32,
        mask: u32,
    ) -> Option<u32> {
        if raw == 0 {
            return None;
        }
        let slot = u32_at(log, memory, self.links.element(raw, 4), mask)?;
        (slot != 0 && slot < self.countries.count).then_some(slot)
    }
}

fn u64_at<const N: usize>(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    address: Option<usize>,
    mask: u32,
) -> Option<u64> {
    let start = log.take(memory, address?, 8, mask)?;
    Some(u64::from_le_bytes(
        log.bytes[start..start + 8].try_into().unwrap(),
    ))
}

fn u32_at<const N: usize>(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    address: Option<usize>,
    mask: u32,
) -> Option<u32> {
    let start = log.take(memory, address?, 4, mask)?;
    Some(u32::from_le_bytes(
        log.bytes[start..start + 4].try_into().unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::Recorded;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const BASE: usize = 0x1_4000_0000;
    const STATE: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x3000;
    const OBJECTS: usize = 0x4000;
    const SIZE: usize = 0x20;

    fn world() -> Vec<u8> {
        let mut bytes = vec![0; object(4)];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        for (header, data) in [
            (cgame_state::COUNTRIES, COUNTRIES),
            (cgame_state::COUNTRY_LINKS, LINKS),
        ] {
            put(STATE + header, &(data as u64).to_le_bytes());
            put(STATE + header + 8, &6_u32.to_le_bytes());
            put(STATE + header + 12, &6_u32.to_le_bytes());
        }
        for (slot, held) in [(0, 0), (1, 1), (3, 0), (4, 2), (5, 2)] {
            put(COUNTRIES + slot * 8, &(object(held) as u64).to_le_bytes());
        }
        for (index, id) in [(0, 0_u32), (1, 1), (2, 4), (3, 0)] {
            put(
                object(index),
                &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(object(index) + ccountry::ID, &id.to_le_bytes());
        }
        for (raw, slot) in [(1, 1_u32), (4, 4), (5, 4)] {
            put(LINKS + raw * 4, &slot.to_le_bytes());
        }
        bytes
    }
    fn object(index: usize) -> usize {
        OBJECTS + index * SIZE
    }

    fn census(memory: &impl Memory) -> (RawLog<1>, Option<Vec<usize>>) {
        let mut log = RawLog::<1>::new(0, 0);
        let countries = Array::read(memory, STATE + cgame_state::COUNTRIES, 8).unwrap();
        let links = Array::read(memory, STATE + cgame_state::COUNTRY_LINKS, 4).unwrap();
        for (address, len) in [
            (STATE + cgame_state::COUNTRIES, 16),
            (STATE + cgame_state::COUNTRY_LINKS, 16),
        ] {
            log.take(memory, address, len, 1).unwrap();
        }
        let mut slots = CountrySlots::new(countries, links, BASE).unwrap();
        let mut admitted = Vec::new();
        for slot in 1..slots.count() {
            match slots.take(&mut log, memory, slot, 1) {
                Some(Some(pointer)) => admitted.push(pointer),
                Some(None) => {}
                None => return (log, None),
            }
        }
        (log, Some(admitted))
    }

    fn reader(memory: &impl Memory) -> Result<Vec<usize>, &'static str> {
        let reader = ccountry::Reader::new(memory, BASE, STATE)?;
        let mut admitted = Vec::new();
        for slot in 1..reader.count() {
            if let Some(country) = reader.at(slot)? {
                admitted.push(country.pointer);
            }
        }
        Ok(admitted)
    }

    #[test]
    fn reserved_and_positive_aliases_are_skipped_as_the_reader_skips_them() {
        let bytes = world();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let (log, admitted) = census(&memory);
        let expected = vec![object(1), object(2)];
        assert_eq!(admitted, Some(expected.clone()));
        assert_eq!(reader(&memory), Ok(expected.clone()));
        assert_eq!(reader(&Recorded::build(&log).unwrap()), Ok(expected));
    }

    #[test]
    fn a_raw_alias_without_a_physical_slot_resolves_once_its_reads_are_recorded() {
        let mut bytes = world();
        bytes[LINKS + 2 * 4..][..4].copy_from_slice(&4_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let live = ccountry::Reader::new(&memory, BASE, STATE).unwrap();
        assert_eq!(live.resolve(2).unwrap().pointer, object(2));
        let replayed = |log: &RawLog<1>| {
            ccountry::Reader::new(&Recorded::build(log).unwrap(), BASE, STATE)
                .unwrap()
                .resolve(2)
        };
        let (mut log, admitted) = census(&memory);
        assert!(admitted.is_some());
        assert!(replayed(&log).is_err(), "the census never reads its link");
        let countries = Array::read(&memory, STATE + cgame_state::COUNTRIES, 8).unwrap();
        let links = Array::read(&memory, STATE + cgame_state::COUNTRY_LINKS, 4).unwrap();
        let slots = CountrySlots::new(countries, links, BASE).unwrap();
        assert_eq!(slots.resolve(&mut log, &memory, 2, 1), Some(()));
        assert_eq!(replayed(&log), live.resolve(2));
        assert_eq!(slots.resolve(&mut log, &memory, 3, 1), None);
        assert!(live.resolve(3).is_err());
    }

    #[test]
    fn an_id_zero_object_other_than_the_reserved_one_ends_the_census_as_the_reader_fails() {
        for slot in [3, 0] {
            let mut bytes = world();
            let pointer = if slot == 3 { object(3) as u64 } else { 0 };
            bytes[COUNTRIES + slot * 8..][..8].copy_from_slice(&pointer.to_le_bytes());
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert_eq!(census(&memory).1, None, "slot {slot}");
            assert!(reader(&memory).is_err(), "slot {slot}");
        }
    }
}
