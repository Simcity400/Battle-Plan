//! Ordered complete CUnit simulation combat-reference observations.
//! Class projection is independent of the runtime-minted low reference word.
//! CUnit owns its CPdxArray at +1A8 (count +1B4); the existing first-combat
//! paths establish pointer stride eight. map_combat_identity authenticates
//! native primary/secondary vtables and the full registry reference; this
//! helper does not broaden that class or lifetime contract.
//! Fixed callbacks use prepared storage; workers grow from original headers.
//! Capture is a field-local optional suffix, never a replacement for existing
//! country-role, full combat side/member, naval country or province data.

use super::*;

pub const COMBAT_LAND: u8 = 0;
pub const COMBAT_BORDER_WAR: u8 = 1;
pub const COMBAT_NAVAL: u8 = 2;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapCombatReference {
    pub combat: u64,
    pub kind: u8,
}

pub(crate) struct CombatReferencesCapture<'a> {
    pub entries: crate::walk::Buffer<'a, MapCombatReference>,
    pub required: usize,
    pub written: usize,
    pub failure: Option<&'static str>,
}
impl<'a> CombatReferencesCapture<'a> {
    pub fn new(entries: &'a mut [MapCombatReference]) -> Self {
        Self::new_buffer(crate::walk::Buffer::Fixed(entries))
    }
    pub fn new_buffer(entries: crate::walk::Buffer<'a, MapCombatReference>) -> Self {
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
        // Use the original combat-vector count for the action query.
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
        registry: &mut crate::creference_registry::HeaderResolver,
    ) {
        if self.failure.is_some() {
            return;
        }
        match super::map_combat_identity(memory, base, pointer, registry) {
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

/// Division's established movement reader already acquired the first pointer.
/// Reuse it and the same copied header; read each unvisited tail pointer once.
/// Some(empty) requires an authenticated zero count and final header agreement.
pub(crate) fn read_unit_combat_references(
    memory: &impl Memory,
    base: usize,
    unit: usize,
    header: [u8; 16],
    first: usize,
    output: &mut [MapCombatReference],
    registry: &mut crate::creference_registry::HeaderResolver,
) -> Result<usize, &'static str> {
    let vector = Array::from_header(header, 8).ok_or("unit combat-reference vector invalid")?;
    let mut capture = CombatReferencesCapture::new(output);
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
            capture.observe(memory, base, pointer, registry);
            if capture.failure.is_some() {
                break;
            }
        }
    }
    capture.finish(memory, unit, header);
    capture.result()
}

#[cfg(test)]
mod tests;
