//! Current raw tie-breaking operands of the ship's already selected variant.
//!
//! Equal-strength distinct variants use mutable simulation terms. Commands
//! can rebuild the same variant and refresh its retained organisation marker,
//! so daily variant tables cannot replace this current observation.
//! No game pointer is returned in the packet and no getter is called.

use super::{ModelMemo, SelectedModelVariant};
use crate::{
    Memory, cequipment_database, cequipment_type, cequipment_variant, cpdx_array::Array,
    memory::Raw, read,
};

/// Module comparison reads the live count. Rebuilding derives summary fields
/// from upgrade levels and signed module priorities.
const MODULE_COUNT: usize = 0xc4;
const UPGRADE_LEVEL_SUM: usize = 0x414;
const MODULE_PRIORITY_SUM: usize = 0x418;
/// Organisation-bonus refresh stores the linked organisation's marker marker here; serialization
/// writes its nonzero signed value.
const ORGANISATION_MARKER: usize = 0x4b0;
/// General definition ordinal. Construction stores the current database
/// count, null construction stores -1, and copying preserves the field.
const DEFINITION_INDEX: usize = 0x418;
/// The database begins with the null definition at slot zero. Loading and
/// copying append at their supplied index.
const DEFINITIONS: usize = 0x68;
/// The database installs the null-definition vtable, clears ACTIVE and
/// publishes its sole pointer. Null type construction supplies the null
/// definition token and ordinal -1. This is not a null variant reference.
pub(super) const NULL_DEFINITION_VTABLE_RVA: usize = 0x0292_19f0;
pub(super) const NULL_DEFINITION_SINGLETON_RVA: usize = 0x0332_0c78;
pub(super) const NULL_DEFINITION_TOKEN: u32 = 0x165;

/// Only the actual native singleton may widen the ordinary CType guard.
/// An equal-header clone or unreadable global/active byte is not that source.
/// Ordinary definitions never execute these extra null-source reads.
pub(super) fn native_null_definition(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    header: [u8; 12],
) -> bool {
    let Some(expected) = base.checked_add(NULL_DEFINITION_VTABLE_RVA) else {
        return false;
    };
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64
        || u32::from_le_bytes(header[8..].try_into().unwrap()) != NULL_DEFINITION_TOKEN
    {
        return false;
    }
    let Some(global) = base.checked_add(NULL_DEFINITION_SINGLETON_RVA) else {
        return false;
    };
    read::<u64>(memory, global) == Some(pointer as u64)
        && scalar::<u8>(memory, pointer, cequipment_type::ACTIVE) == Some(0)
}

/// Independent guarded fields; arithmetic and content classification belong
/// to the reached consumer. None is unreadable/refused, never a numeric zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HullVariantInputs {
    pub definition_index: Option<i32>,
    pub module_count: Option<i32>,
    pub upgrade_level_sum: Option<i32>,
    pub module_priority_sum: Option<i32>,
    pub version: Option<u8>,
    pub organisation_marker: Option<i32>,
}

/// One consecutive selected-source observation within this stopped capture.
/// Both healthy and failed fields remain frozen while this exact key repeats;
/// a different key begins the next observation. A worker also keeps every
/// packet whose six fields all succeeded, keyed by the same exact source, so
/// a variant seen earlier in the capture is not acquired again; refused or
/// partial packets never enter that map. No global cache.
#[derive(Default)]
pub(super) struct Memo {
    last: Option<(Key, Option<HullVariantInputs>)>,
    complete: crate::combat_sources::Map<Key, HullVariantInputs>,
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

    /// A successful different/null model source breaks adjacency even when
    /// no optional tuple is requested for that row. This performs no reads.
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

/// Acquire only after the original model and maximum-organisation reads have
/// succeeded, during their same stopped/caller-owned census observation.
/// The source is minted by read_model_with_selection, never by a daily join.
/// A changed selected identity/token/definition refuses the whole packet.
/// Definition/DB faults withhold only the ordinal; every raw term is independent.
///
/// At most14 fixed guarded spans/101 bytes per nonnull selection, without
/// retries or a definition walk. Consecutive same-key observations retain
/// their completed/failed packet without reading again, and a worker reuses
/// any earlier complete packet of the same key. Reset ModelMemo at every
/// capture; a different key replaces the last observation. The caller
/// admits this separately from the old budget. Empty selection has no reads.
/// A registered variant can reference the native null CType: its exact
/// singleton yields ordinal-1 (12 spans/70 bytes), never recorded Equipment[0].
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
                && memo.growing
                // An allocation failure is already terminal for the capture;
                // it only forgoes this reuse.
                && crate::allocation::reserve_map(&mut state.complete, 1).is_ok()
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
    let expected = base.checked_add(cequipment_variant::VTABLE_RVA)?;
    let identity = scalar::<[u8; 16]>(memory, source.pointer, 0)?;
    if cequipment_variant::identity_from_header(expected, identity).ok()? != source.handle {
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
    let expected = base.checked_add(cequipment_type::VTABLE_RVA)? as u64;
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
    // A real positive N maps to recorded Equipment[N-1]. An ordinary-vptr
    // nonpositive index cannot claim the separately authenticated null source.
    let slot = u32::try_from(index).ok().filter(|index| *index != 0)?;
    let global = base.checked_add(cequipment_database::SINGLETON_RVA)?;
    let database = usize::try_from(read::<u64>(memory, global)?).ok()?;
    let expected = base.checked_add(cequipment_database::VTABLE_RVA)? as u64;
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
    // read also checks the complete span, not just its first address.
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| read(memory, at))
}

#[cfg(test)]
mod tests;
