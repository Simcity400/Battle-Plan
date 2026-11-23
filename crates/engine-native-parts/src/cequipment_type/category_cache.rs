use engine_native_memory::{Memory, read};
use engine_allocation as allocation;
use crate::cequipment_type as equipment;

#[derive(Default)]
pub struct CategoryCache {
    certificate: Option<(u64, u64, u64)>,
    classes: Vec<(usize, u64, Result<bool, &'static str>)>,
    path: Vec<usize>,
}
impl CategoryCache {
    pub fn prepare(&mut self, certificate: Option<(u64, u64, u64)>) {
        if certificate.is_none() || self.certificate != certificate {
            self.classes.clear();
        }
        self.certificate = certificate;
        self.path.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.classes.capacity() * size_of::<(usize, u64, Result<bool, &'static str>)>()
            + self.path.capacity() * size_of::<usize>()
    }
    pub fn classify(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        mask: u64,
    ) -> Result<bool, &'static str> {
        self.path.clear();
        let result = self.classify_inner(memory, base, pointer, mask);
        if result.is_err()
            && self.certificate.is_some()
            && let Err(index) = self
                .classes
                .binary_search_by_key(&(pointer, mask), |entry| (entry.0, entry.1))
            && allocation::reserve(&mut self.classes, 1).is_ok()
        {
            self.classes.insert(index, (pointer, mask, result));
        }
        result
    }
    fn classify_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        mask: u64,
    ) -> Result<bool, &'static str> {
        if self.certificate.is_none() {
            return Err("equipment category writer certificate unavailable");
        }
        if let Ok(index) = self
            .classes
            .binary_search_by_key(&(pointer, mask), |entry| (entry.0, entry.1))
        {
            return self.classes[index].2;
        }
        if pointer == 0 || self.path.contains(&pointer) {
            return Err("equipment category archetype cycle or null");
        }
        let vtable =
            read::<u64>(memory, pointer).ok_or("equipment category identity unreadable")?;
        if engine_sites::which(
            base,
            vtable as usize,
            &[equipment::VTABLE_RVA, equipment::NULL_VTABLE_RVA],
        )
        .is_none()
        {
            return Err("equipment category identity mismatch");
        }
        let archetype = pointer
            .checked_add(equipment::ARCHETYPE)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("equipment category archetype unreadable")?;
        allocation::reserve(&mut self.path, 1)
            .map_err(|_| "equipment category path allocation failed")?;
        self.path.push(pointer);
        let result =
            if archetype != pointer && self.classify_inner(memory, base, archetype, mask)? {
                true
            } else {
                let flags = pointer
                    .checked_add(equipment::PRIMARY_CATEGORY_FLAGS)
                    .and_then(|at| read::<[u8; 16]>(memory, at))
                    .ok_or("equipment category flags unreadable")?;
                let mut primary = u64::from_le_bytes(flags[..8].try_into().unwrap());
                if primary == 0 {
                    primary = archetype
                        .checked_add(equipment::PRIMARY_CATEGORY_FLAGS)
                        .and_then(|at| read(memory, at))
                        .ok_or("equipment archetype category unreadable")?;
                }
                let mut category = primary | u64::from_le_bytes(flags[8..].try_into().unwrap());
                if category & equipment::NORMALIZE_FIRST_SELECTOR != 0 {
                    category &= !equipment::NORMALIZE_FIRST_CLEAR;
                } else if category & equipment::NORMALIZE_SECOND_SELECTOR != 0 {
                    category &= !equipment::NORMALIZE_SECOND_CLEAR;
                } else if category & equipment::NORMALIZE_THIRD_SELECTOR != 0 {
                    category &= !equipment::SPECIAL_POOL_MASK;
                }
                category & mask != 0
            };
        self.path.pop();
        allocation::reserve(&mut self.classes, 1)
            .map_err(|_| "equipment category cache allocation failed")?;
        let index = self
            .classes
            .binary_search_by_key(&(pointer, mask), |entry| (entry.0, entry.1))
            .unwrap_err();
        self.classes.insert(index, (pointer, mask, Ok(result)));
        Ok(result)
    }
}
