use super::SelectedModelVariant;
use engine_native_memory::{Memory, read};
use engine_allocation as allocation;
use engine_native_parts::cequipment_type;
use crate::cequipment_variant;

pub(super) const HEAVY_ATTACK: usize = 0x1d8;
pub(super) const SPEED_RANGE: usize = 0x208;
pub(super) const CARRIER_SIZE: usize = 0x340;
const DOMINANCE_WEIGHT: usize = 0x450;
pub(super) const CARRIER_SIZE_SELECTOR: u64 = 1 << 7;

#[derive(Default)]
pub struct NavalPowerCache {
    certificate: Option<(u64, u64, u64)>,
    categories: cequipment_type::CategoryCache,
    definitions: Vec<(usize, Option<i64>)>,
}
impl NavalPowerCache {
    pub fn prepare(&mut self, certificate: Option<(u64, u64, u64)>) -> bool {
        let changed = self.certificate != certificate;
        if changed || certificate.is_none() {
            self.definitions.clear();
        }
        self.categories.prepare(certificate);
        self.certificate = certificate;
        changed
    }
    pub fn allocated_bytes(&self) -> usize {
        self.categories.allocated_bytes()
            + self.definitions.capacity() * size_of::<(usize, Option<i64>)>()
    }
    fn weight(&mut self, memory: &impl Memory, base: usize, definition: usize) -> Option<i64> {
        self.certificate?;
        match self
            .definitions
            .binary_search_by_key(&definition, |entry| entry.0)
        {
            Ok(index) => self.definitions[index].1,
            Err(index) => {
                let value = self
                    .categories
                    .classify(memory, base, definition, cequipment_type::SHIP_MASK)
                    .ok()
                    .and_then(|ship| {
                        if ship {
                            definition
                                .checked_add(DOMINANCE_WEIGHT)
                                .and_then(|at| read(memory, at))
                        } else {
                            Some(0)
                        }
                    });
                allocation::reserve(&mut self.definitions, 1).ok()?;
                self.definitions.insert(index, (definition, value));
                value
            }
        }
    }
}

pub fn read_naval_power(
    memory: &impl Memory,
    base: usize,
    source: &SelectedModelVariant,
    cache: &mut NavalPowerCache,
) -> Option<[i64; 5]> {
    let weight = cache.weight(memory, base, source.definition)?;
    if weight == 0 {
        return Some([0; 5]);
    }
    let at = |offset| source.pointer.checked_add(offset);
    let pair: [u8; 16] = read(memory, at(SPEED_RANGE)?)?;
    let speed = i64::from_le_bytes(pair[..8].try_into().unwrap());
    let range = i64::from_le_bytes(pair[8..].try_into().unwrap());
    let attack: i64 = read(memory, at(HEAVY_ATTACK)?)?;
    let flags: u64 = read(memory, at(cequipment_variant::CLASSIFIER_FLAGS)?)?;
    let carrier: i64 = read(
        memory,
        at(CARRIER_SIZE + usize::from(flags & CARRIER_SIZE_SELECTOR != 0) * 8)?,
    )?;
    Some([weight, range, speed, carrier, attack])
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    #[test]
    fn unavailable_certificate_does_not_invalidate_an_already_unknown_packet() {
        let mut cache = NavalPowerCache::default();
        assert!(!cache.prepare(None));
        assert!(!cache.prepare(None));
        assert!(cache.prepare(Some((1, 2, 3))));
        assert!(!cache.prepare(Some((1, 2, 3))));
        assert!(cache.prepare(None));
        assert!(!cache.prepare(None));
    }
    #[test]
    fn raw_selected_design_preserves_speed_range_carrier_selection_and_carried_definition() {
        let mut bytes = vec![0; 0x3000];
        let definition = 0x1000usize;
        let variant = 0x2000usize;
        let word = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        word(&mut bytes, definition, cequipment_type::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            definition + cequipment_type::ARCHETYPE,
            definition as u64,
        );
        word(
            &mut bytes,
            definition + cequipment_type::PRIMARY_CATEGORY_FLAGS,
            1,
        );
        word(&mut bytes, definition + DOMINANCE_WEIGHT, 321);
        for (offset, value) in [
            (SPEED_RANGE, 11),
            (SPEED_RANGE + 8, 22),
            (HEAVY_ATTACK, 33),
            (CARRIER_SIZE, 44),
            (CARRIER_SIZE + 8, 55),
        ] {
            word(&mut bytes, variant + offset, value);
        }
        word(
            &mut bytes,
            variant + cequipment_variant::CLASSIFIER_FLAGS,
            0x80,
        );
        let source = SelectedModelVariant {
            pointer: variant,
            handle: 70,
            definition,
            token: [0; 4],
        };
        let mut cache = NavalPowerCache::default();
        cache.prepare(Some((1, 2, 3)));
        assert_eq!(
            read_naval_power(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                &source,
                &mut cache
            ),
            Some([321, 22, 11, 55, 33])
        );
        bytes[definition..definition + 0x600].fill(0);
        assert_eq!(
            read_naval_power(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                &source,
                &mut cache
            ),
            Some([321, 22, 11, 55, 33])
        );
        cache.prepare(None);
        assert_eq!(
            read_naval_power(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                &source,
                &mut cache
            ),
            None
        );
    }
}
