use engine_native_memory::{Memory, read};
use engine_allocation as allocation;
use engine_native_parts::cequipment_type as equipment;

#[derive(Default)]
pub struct VolunteerAir {
    certificate: Option<(u64, u64, u64)>,
    categories: equipment::CategoryCache,
    totals: Vec<(u32, Option<i32>)>,
}
impl VolunteerAir {
    pub fn prepare(&mut self, certificate: Option<(u64, u64, u64)>) {
        self.categories.prepare(certificate);
        self.certificate = certificate;
        self.totals.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.categories.allocated_bytes() + self.totals.capacity() * size_of::<(u32, Option<i32>)>()
    }
    pub(super) fn begin(&mut self, memory: &impl Memory, slot: u32, country: usize) {
        let demanded = self.certificate.is_some()
            && country
                .checked_add(engine_native::ccountry::VOLUNTEER_HOSTS_COUNT)
                .and_then(|at| read::<i32>(memory, at))
                .is_some_and(|count| count > 0);
        if demanded && allocation::reserve(&mut self.totals, 1).is_ok() {
            self.totals.push((slot, Some(0)));
        }
    }
    pub(super) fn add(&mut self, memory: &impl Memory, base: usize, slot: u32, bytes: &[u8]) {
        let Some(index) = self
            .totals
            .len()
            .checked_sub(1)
            .filter(|&index| self.totals[index].0 == slot && self.totals[index].1.is_some())
        else {
            return;
        };
        for row in bytes.chunks_exact(super::ROW) {
            let pointer = u64::from_le_bytes(row[..8].try_into().unwrap()) as usize;
            let class = self
                .categories
                .classify(memory, base, pointer, equipment::PLANE_MASK);
            match class {
                Ok(true) => {
                    let amount = i64::from_le_bytes(
                        row[engine_native_parts::allocation_equipment_row::AMOUNT..super::ROW]
                            .try_into()
                            .unwrap(),
                    );
                    self.totals[index].1 = self.totals[index]
                        .1
                        .map(|sum| sum.wrapping_add((amount / 100_000) as i32));
                }
                Ok(false) => {}
                Err(_) => {
                    self.totals[index].1 = None;
                    break;
                }
            }
        }
    }
    pub(super) fn total(&self, slot: u32) -> Option<i32> {
        self.totals
            .binary_search_by_key(&slot, |entry| entry.0)
            .ok()
            .and_then(|index| self.totals[index].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeMap};
    #[derive(Default)]
    struct Image {
        bytes: BTreeMap<usize, u8>,
        reads: Cell<usize>,
    }
    impl Memory for Image {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            for (index, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(at + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl Image {
        fn word(&mut self, at: usize, word: u64) {
            self.bytes.extend(
                word.to_le_bytes()
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (at + i, b)),
            );
        }
        fn definition(&mut self, at: usize, archetype: usize, flags: u64) {
            self.word(at, equipment::VTABLE_RVA.0 as u64);
            self.word(at + equipment::ARCHETYPE, archetype as u64);
            self.word(at + equipment::PRIMARY_CATEGORY_FLAGS, flags);
            self.word(at + equipment::SECONDARY_CATEGORY_FLAGS, 0);
        }
    }
    fn allocation(pointer: usize, amount: i64) -> [u8; super::super::ROW] {
        let mut row = [0; super::super::ROW];
        row[..8].copy_from_slice(&(pointer as u64).to_le_bytes());
        row[engine_native_parts::allocation_equipment_row::AMOUNT..].copy_from_slice(&amount.to_le_bytes());
        row
    }
    #[test]
    fn totals_are_uncapped_and_truncate_each_archetype_before_wrapping_sum() {
        let mut image = Image::default();
        image.definition(0x1000, 0x1000, equipment::PLANE_MASK);
        image.definition(0x2000, 0x2000, 1);
        let mut air = VolunteerAir::default();
        air.prepare(Some((1, 2, 3)));
        air.totals.push((1, Some(0)));
        for (pointer, amount) in [
            (0x1000, 15_999_999),
            (0x2000, 999_999_999),
            (0x1000, -199_999),
        ] {
            air.add(&image, 0, 1, &allocation(pointer, amount));
        }
        assert_eq!(air.total(1), Some(158));
        air.add(
            &image,
            0,
            1,
            &allocation(0x1000, i64::from(i32::MAX) * 100_000),
        );
        assert_eq!(air.total(1), Some(158_i32.wrapping_add(i32::MAX)));
    }
    #[test]
    fn inherited_positive_category_is_lazy_and_quiet_certificate_avoids_type_reads() {
        let mut image = Image::default();
        image.word(0x1000, equipment::VTABLE_RVA.0 as u64);
        image.word(0x1000 + equipment::ARCHETYPE, 0x2000);
        image.definition(0x2000, 0x2000, equipment::PLANE_MASK);
        let mut air = VolunteerAir::default();
        air.prepare(Some((1, 2, 3)));
        air.totals.push((1, Some(0)));
        air.add(&image, 0, 1, &allocation(0x1000, 100_000));
        assert_eq!(air.total(1), Some(1));
        let reads = image.reads.get();
        image.bytes.clear();
        air.prepare(Some((1, 2, 3)));
        air.totals.push((1, Some(0)));
        air.add(&image, 0, 1, &allocation(0x1000, 200_000));
        assert_eq!((air.total(1), image.reads.get()), (Some(2), reads));
        air.prepare(Some((1, 2, 4)));
        air.totals.push((1, Some(0)));
        air.add(&image, 0, 1, &allocation(0x1000, 100_000));
        assert_eq!(air.total(1), None);
    }
    #[test]
    fn unobserved_equipment_writers_never_fall_back_to_category_polling() {
        let image = Image::default();
        let mut air = VolunteerAir::default();
        air.prepare(None);
        air.begin(&image, 1, 0x1000);
        air.add(&image, 0, 1, &allocation(0x2000, 100_000));
        assert_eq!((air.total(1), image.reads.get()), (None, 0));
    }
}
