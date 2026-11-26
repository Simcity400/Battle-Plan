use crate::{ccountry_collaboration_data as relation, ccountry_collaboration_status as status};
use engine_native::ccountry;
use engine_native_memory::{Memory, country_identity::Country, cpdx_array::Array, read};
use engine_native_parts::{modifier_accumulator, modifier_definition};
use engine_occupation::pdx_robin_hood_table;
use engine_sites::Rva;

pub const SURRENDER_PROGRESS_RVA: Rva = Rva(0x0072_1c50);
const HELD_CORE_WEIGHT: usize = 0x1338;
const HELD_TERRITORY_WEIGHT: usize = 0x133c;
const TOTAL_CORE_WEIGHT: usize = 0x1340;
const TOTAL_TERRITORY_WEIGHT: usize = 0x1344;
const RELATIONS_BY_TARGET: usize = 0x10;
const MODIFIERS: [&[u8]; 3] = [b"surrender_limit", b"forced_surrender_limit", b"max_surrender_limit_offset"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub held_core: Option<i32>,
    pub total_core: Option<i32>,
    pub held_territory: Option<i32>,
    pub total_territory: Option<i32>,
    pub modifiers: [Option<i64>; 3],
    pub collaboration: Option<i64>,
}

#[derive(Debug, Default)]
pub struct ModifierKeys {
    held: Option<(Array, [Option<u32>; 3])>,
}

impl ModifierKeys {
    pub fn resolve(&mut self, memory: &impl Memory, base: usize) -> [Option<u32>; 3] {
        let Ok(header) = modifier_definition::table(memory, base) else {
            self.held = None;
            return [None; 3];
        };
        if let Some((before, keys)) = self.held && before == header {
            return keys;
        }
        self.held = modifier_definition::keys_by_names_at(memory, base, &MODIFIERS)
            .ok().map(|(keys, table)| (table, keys));
        self.held.map_or([None; 3], |(_, keys)| keys)
    }
}

pub struct Hour {
    layout: bool,
    keys: [Option<u32>; 3],
    definitions: modifier_definition::DefinitionMemo,
}

impl Hour {
    pub fn new(memory: &impl Memory, base: usize, keys: &mut ModifierKeys) -> Self {
        Self {
            layout: engine_sites::at(base, SURRENDER_PROGRESS_RVA).is_some(),
            keys: keys.resolve(memory, base),
            definitions: Default::default(),
        }
    }

    pub fn country<M: Memory>(
        &self, memory: &M, base: usize, countries: Option<&ccountry::Reader<'_, M>>, owner: Country,
    ) -> Inputs {
        if !self.layout { return Inputs::default(); }
        let cached = |field| read::<i32>(memory, owner.pointer.checked_add(field)?);
        let accumulator = owner.pointer.checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR);
        let modifiers = self.keys.map(|key| {
            let key = key?;
            let value = modifier_accumulator::lookup_requested_at(memory, accumulator?, key).ok()?;
            if value.present {
                modifier_definition::effective_memo(memory, base, key, value.raw, &self.definitions).ok()
            } else {
                Some(0)
            }
        });
        let collaboration = cached(ccountry::SURRENDER_TARGET).and_then(|target| {
            if target <= 0 { return Some(0); }
            let countries = countries?;
            let target = countries.resolve(target as u32).ok()?;
            let pointer = usize::try_from(read::<u64>(memory, owner.pointer.checked_add(ccountry::COLLABORATION_STATUS)?)?).ok()?;
            if pointer == 0 || read::<u64>(memory, pointer)? != engine_sites::at(base, status::VTABLE_RVA)? as u64
                || read::<u64>(memory, pointer.checked_add(status::OWNER)?)? != owner.pointer as u64 {
                return None;
            }
            let found = pdx_robin_hood_table::find(memory, pointer.checked_add(RELATIONS_BY_TARGET)?, target.canonical_slot).ok()?;
            let Some(entry) = found else { return Some(0); };
            let pair = relation::read_row(memory, base, entry.value, owner.id).ok()?;
            (countries.resolve(pair.target).ok()?.canonical_slot == target.canonical_slot).then_some(pair.progress)
        });
        Inputs {
            held_core: cached(HELD_CORE_WEIGHT),
            total_core: cached(TOTAL_CORE_WEIGHT),
            held_territory: cached(HELD_TERRITORY_WEIGHT),
            total_territory: cached(TOTAL_TERRITORY_WEIGHT),
            modifiers,
            collaboration,
        }
    }
}
