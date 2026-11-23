
use super::*;

pub const COMBAT_LAND: u8 = 0;
pub const COMBAT_BORDER_WAR: u8 = 1;
pub const COMBAT_NAVAL: u8 = 2;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapCombatReference {
    pub combat: u64,
    pub kind: u8,
}

#[derive(Default)]
pub(crate) struct MapCombatIdentityMemo {
    entries: std::collections::HashMap<(usize, usize), MapCombatReference>,
    registry: engine_native_parts::creference_registry::HeaderResolver,
}

impl MapCombatIdentityMemo {
    pub(crate) fn reserve(&mut self, capacity: usize) -> std::io::Result<()> {
        let additional = capacity.saturating_sub(self.entries.len());
        engine_allocation::reserve_map(&mut self.entries, additional)
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.registry = Default::default();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * (size_of::<((usize, usize), MapCombatReference)>() + 1)
    }

    fn identity(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<(u64, u8), &'static str> {
        let key = (base, pointer);
        if let Some(value) = self.entries.get(&key) {
            return Ok((value.combat, value.kind));
        }
        let (combat, kind) = super::map_combat_identity(memory, base, pointer, &mut self.registry)?;
        if self.entries.len() < self.entries.capacity() {
            self.entries
                .insert(key, MapCombatReference { combat, kind });
        }
        Ok((combat, kind))
    }
}

pub struct CombatReferencesCapture<'a> {
    pub entries: engine_native_memory::walk::Buffer<'a, MapCombatReference>,
    pub required: usize,
    pub written: usize,
    pub failure: Option<&'static str>,
}
impl<'a> CombatReferencesCapture<'a> {
    pub fn new_buffer(entries: engine_native_memory::walk::Buffer<'a, MapCombatReference>) -> Self {
        Self {
            entries,
            required: 0,
            written: 0,
            failure: None,
        }
    }
    pub fn begin(&mut self, count: u32) {
        self.required = count as usize;
        self.written = 0;
        self.failure = None;
        if let Err(reason) = self.entries.ensure(self.required) {
            self.refuse(reason);
        }
        if self.required > self.entries.len() {
            self.refuse("unit combat-reference arena capacity exceeded");
        }
    }
    pub fn refuse(&mut self, reason: &'static str) {
        self.failure.get_or_insert(reason);
    }
    pub fn observe(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        registry: &mut engine_native_parts::creference_registry::HeaderResolver,
    ) {
        if self.failure.is_some() {
            return;
        }
        self.observe_identity(super::map_combat_identity(memory, base, pointer, registry));
    }
    fn observe_identity(&mut self, identity: Result<(u64, u8), &'static str>) {
        match identity {
            Ok((combat, kind)) => {
                let Some(slot) = self.entries.get_mut(self.written) else {
                    self.refuse("unit combat-reference arena capacity exceeded");
                    return;
                };
                *slot = MapCombatReference { combat, kind };
                self.written += 1;
            }
            Err(reason) => self.refuse(reason),
        }
    }
    pub fn finish(&mut self, memory: &impl Memory, unit: usize, header: [u8; 16]) {
        if self.failure.is_some() {
            return;
        }
        if self.written != self.required {
            self.refuse("unit combat-reference acquisition length invalid");
            return;
        }
        if unit
            .checked_add(COMBAT_REFERENCE_DATA)
            .and_then(|at| read::<[u8; 16]>(memory, at))
            != Some(header)
        {
            self.refuse("unit combat-reference vector changed or unreadable");
        }
    }
    pub fn result(&self) -> Result<usize, &'static str> {
        if let Some(reason) = self.failure {
            return Err(reason);
        }
        if self.written != self.required {
            return Err("unit combat-reference acquisition length invalid");
        }
        Ok(self.written)
    }
}

pub(crate) fn read_unit_combat_references(
    memory: &impl Memory,
    base: usize,
    unit: usize,
    header: [u8; 16],
    first: usize,
    output: &mut Vec<MapCombatReference>,
    start: usize,
    memo: &mut MapCombatIdentityMemo,
) -> Result<usize, &'static str> {
    read_unit_combat_references_with(memory, unit, header, first, output, start, |pointer| {
        memo.identity(memory, base, pointer)
    })
}

fn read_unit_combat_references_with(
    memory: &impl Memory,
    unit: usize,
    header: [u8; 16],
    first: usize,
    output: &mut Vec<MapCombatReference>,
    start: usize,
    mut identity: impl FnMut(usize) -> Result<(u64, u8), &'static str>,
) -> Result<usize, &'static str> {
    let vector = Array::from_header(header, 8).ok_or("unit combat-reference vector invalid")?;
    let mut capture =
        CombatReferencesCapture::new_buffer(engine_native_memory::walk::Buffer::GrowingFrom(output, start));
    capture.begin(vector.count);
    if capture.failure.is_none() {
        for index in 0..vector.count {
            let pointer = if index == 0 {
                Some(first).filter(|&pointer| pointer != 0)
            } else {
                vector
                    .element(index, 8)
                    .and_then(|at| read::<u64>(memory, at))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .filter(|&pointer| pointer != 0)
            };
            let Some(pointer) = pointer else {
                capture.refuse("unit combat-reference pointer unreadable or null");
                break;
            };
            capture.observe_identity(identity(pointer));
            if capture.failure.is_some() {
                break;
            }
        }
    }
    if vector.count != 0 {
        capture.finish(memory, unit, header);
    }
    capture.result()
}

#[cfg(test)]
mod tests;
