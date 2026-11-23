
use engine_sites::Rva;
use super::{ModelMemo, SelectedModelVariant};
use engine_native_memory::{Memory, cpdx_array::Array, memory::Raw, read};
use engine_native_parts::{cequipment_database, cequipment_type};
use crate::cequipment_variant;

const MODULE_COUNT: usize = 0xc4;
const UPGRADE_LEVEL_SUM: usize = 0x414;
const MODULE_PRIORITY_SUM: usize = 0x418;
const ORGANISATION_MARKER: usize = 0x4b0;
const DEFINITION_INDEX: usize = 0x478;
const DEFINITIONS: usize = 0x68;
pub(super) const NULL_DEFINITION_VTABLE_RVA: Rva = Rva(0x029b_5f70);
pub(super) const NULL_DEFINITION_SINGLETON_RVA: Rva = Rva(0x033e_75a8);
pub(super) const NULL_DEFINITION_TOKEN: u32 = 0x165;

pub(super) fn native_null_definition(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    header: [u8; 12],
) -> bool {
    let Some(expected) = engine_sites::at(base, NULL_DEFINITION_VTABLE_RVA) else {
        return false;
    };
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64
        || u32::from_le_bytes(header[8..].try_into().unwrap()) != NULL_DEFINITION_TOKEN
    {
        return false;
    }
    let Some(global) = engine_sites::at(base, NULL_DEFINITION_SINGLETON_RVA) else {
        return false;
    };
    read::<u64>(memory, global) == Some(pointer as u64)
        && scalar::<u8>(memory, pointer, cequipment_type::ACTIVE) == Some(0)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HullVariantInputs {
    pub definition_index: Option<i32>,
    pub module_count: Option<i32>,
    pub upgrade_level_sum: Option<i32>,
    pub module_priority_sum: Option<i32>,
    pub version: Option<u8>,
    pub organisation_marker: Option<i32>,
}

#[derive(Default)]
pub(super) struct Memo {
    last: Option<(Key, Option<HullVariantInputs>)>,
    complete: engine_native_parts::keyed_map::Map<Key, HullVariantInputs>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    base: usize,
    pointer: usize,
    handle: u64,
    definition: usize,
    token: [u8; 4],
}

impl Key {
    fn new(base: usize, source: &SelectedModelVariant) -> Self {
        Self {
            base,
            pointer: source.pointer,
            handle: source.handle,
            definition: source.definition,
            token: source.token,
        }
    }
}

impl Memo {
    pub(super) fn reset(&mut self) {
        self.last = None;
        self.complete.clear();
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.complete.capacity() * std::mem::size_of::<(Key, HullVariantInputs)>()
    }

    pub(super) fn retain_source(&mut self, base: usize, source: Option<&SelectedModelVariant>) {
        let current = source.map(|source| Key::new(base, source));
        if self
            .last
            .as_ref()
            .is_some_and(|(saved, _)| Some(*saved) != current)
        {
            self.last = None;
        }
    }
}

pub fn read_hull_variant(
    memory: &impl Memory,
    base: usize,
    source: &SelectedModelVariant,
    memo: &mut ModelMemo,
) -> Option<HullVariantInputs> {
    let key = Key::new(base, source);
    let state = &mut memo.hull_variant;
    if let Some((_, packet)) = state.last.as_ref().filter(|(saved, _)| *saved == key) {
        return *packet;
    }
    let packet = match state.complete.get(&key) {
        Some(packet) => Some(*packet),
        None => {
            let packet = acquire(memory, base, source);
            if let Some(complete) = packet.filter(complete)
                && engine_allocation::reserve_map(&mut state.complete, 1).is_ok()
            {
                state.complete.insert(key, complete);
            }
            packet
        }
    };
    state.last = Some((key, packet));
    packet
}

fn complete(packet: &HullVariantInputs) -> bool {
    packet.definition_index.is_some()
        && packet.module_count.is_some()
        && packet.upgrade_level_sum.is_some()
        && packet.module_priority_sum.is_some()
        && packet.version.is_some()
        && packet.organisation_marker.is_some()
}

fn acquire(
    memory: &impl Memory,
    base: usize,
    source: &SelectedModelVariant,
) -> Option<HullVariantInputs> {
    let expected = engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)?;
    let identity = scalar::<[u8; 16]>(memory, source.pointer, 0)?;
    if engine_native_parts::equipment_variant_identity::identity_from_header(expected, identity).ok()? != source.handle {
        return None;
    }
    let token = scalar::<[u8; 4]>(memory, source.pointer, cequipment_variant::EQUIPMENT_TOKEN)?;
    let definition = scalar::<u64>(memory, source.pointer, cequipment_variant::DEFINITION)?;
    if token != source.token || usize::try_from(definition).ok()? != source.definition {
        return None;
    }
    Some(HullVariantInputs {
        definition_index: definition_index(memory, base, source),
        module_count: scalar(memory, source.pointer, MODULE_COUNT),
        upgrade_level_sum: scalar(memory, source.pointer, UPGRADE_LEVEL_SUM),
        module_priority_sum: scalar(memory, source.pointer, MODULE_PRIORITY_SUM),
        version: scalar(memory, source.pointer, cequipment_variant::VERSION),
        organisation_marker: scalar(memory, source.pointer, ORGANISATION_MARKER),
    })
}

fn definition_index(
    memory: &impl Memory,
    base: usize,
    source: &SelectedModelVariant,
) -> Option<i32> {
    let expected = engine_sites::at(base, cequipment_type::VTABLE_RVA)? as u64;
    let header = scalar::<[u8; 12]>(memory, source.definition, 0)?;
    if header[8..12] != source.token {
        return None;
    }
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected {
        if source.handle >> 32 == 0
            || !native_null_definition(memory, base, source.definition, header)
        {
            return None;
        }
        return scalar::<i32>(memory, source.definition, DEFINITION_INDEX)
            .filter(|index| *index == -1);
    }
    let index: i32 = scalar(memory, source.definition, DEFINITION_INDEX)?;
    let slot = u32::try_from(index).ok().filter(|index| *index != 0)?;
    let global = engine_sites::at(base, cequipment_database::SINGLETON_RVA)?;
    let database = usize::try_from(read::<u64>(memory, global)?).ok()?;
    let expected = engine_sites::at(base, cequipment_database::VTABLE_RVA)? as u64;
    if scalar::<u64>(memory, database, 0)? != expected {
        return None;
    }
    let list = scalar::<[u8; 16]>(memory, database, DEFINITIONS)
        .and_then(|header| Array::from_header(header, 8))?;
    let entry = list.element(slot, 8)?;
    if read::<u64>(memory, entry)? != source.definition as u64 {
        return None;
    }
    Some(index)
}

#[inline]
fn scalar<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Option<T> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| read(memory, at))
}

#[cfg(test)]
mod tests;
