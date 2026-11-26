use crate::{ccountry_collaboration_data as relation, ccountry_collaboration_status as status};
use engine_native::ccountry;
use engine_native_memory::{Memory, country_identity::Country, cpdx_array::Array, read};
use engine_native_parts::{modifier_accumulator, modifier_definition};
use engine_occupation::pdx_robin_hood_table;
use engine_sites::Rva;

const SURRENDER_LIMIT_OPERAND: Rva = Rva(0x0072_1c8d);
const FORCED_LIMIT_OPERAND: Rva = Rva(0x0072_1d68);
const MAX_LIMIT_OFFSET_OPERAND: Rva = Rva(0x0072_1cc3);
const CACHED_WEIGHT_READ: Rva = Rva(0x0072_1da3);
const SELECTED_COLLABORATION_READ: Rva = Rva(0x0149_0d90);
const HELD_CORE_WEIGHT: usize = 0x1338;
const HELD_TERRITORY_WEIGHT: usize = 0x133c;
const TOTAL_CORE_WEIGHT: usize = 0x1340;
const TOTAL_TERRITORY_WEIGHT: usize = 0x1344;
const RELATIONS_BY_TARGET: usize = 0x10;
const MODIFIERS: [&[u8]; 3] = [
    b"surrender_limit",
    b"forced_surrender_limit",
    b"max_surrender_limit_offset",
];

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
        if let Some((before, keys)) = self.held
            && before == header
        {
            return keys;
        }
        self.held = modifier_definition::keys_by_names_at(memory, base, &MODIFIERS)
            .ok()
            .map(|(mut keys, table)| {
                for (key, operand) in keys.iter_mut().zip([
                    SURRENDER_LIMIT_OPERAND,
                    FORCED_LIMIT_OPERAND,
                    MAX_LIMIT_OFFSET_OPERAND,
                ]) {
                    let value = engine_sites::code(base, operand)
                        .and_then(|site| site.original::<6>())
                        .ok()
                        .map(|[_, _, a, b, c, d]| u32::from_le_bytes([a, b, c, d]));
                    if *key != value {
                        *key = None;
                    }
                }
                (table, keys)
            });
        self.held.map_or([None; 3], |(_, keys)| keys)
    }
}

pub struct Hour {
    weights: bool,
    collaboration: bool,
    keys: [Option<u32>; 3],
    definitions: modifier_definition::DefinitionMemo,
}

impl Hour {
    pub fn new(memory: &impl Memory, base: usize, keys: &mut ModifierKeys) -> Self {
        Self {
            weights: engine_sites::require(base, &[CACHED_WEIGHT_READ]).is_ok(),
            collaboration: engine_sites::require(
                base,
                &[MAX_LIMIT_OFFSET_OPERAND, SELECTED_COLLABORATION_READ],
            )
            .is_ok(),
            keys: keys.resolve(memory, base),
            definitions: Default::default(),
        }
    }

    pub fn country<M: Memory>(
        &self,
        memory: &M,
        base: usize,
        countries: Option<&ccountry::Reader<'_, M>>,
        owner: Country,
    ) -> Inputs {
        let cached = |field| read::<i32>(memory, owner.pointer.checked_add(field)?);
        let weight = |field| self.weights.then(|| cached(field)).flatten();
        let accumulator = owner
            .pointer
            .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR);
        let modifiers = self.keys.map(|key| {
            let key = key?;
            let value =
                modifier_accumulator::lookup_requested_at(memory, accumulator?, key).ok()?;
            if value.present {
                modifier_definition::effective_memo(memory, base, key, value.raw, &self.definitions)
                    .ok()
            } else {
                Some(0)
            }
        });
        let collaboration = self
            .collaboration
            .then(|| cached(ccountry::SURRENDER_TARGET))
            .flatten()
            .and_then(|target| {
                if target <= 0 {
                    return Some(0);
                }
                let countries = countries?;
                let target = countries.resolve(target as u32).ok()?;
                let pointer = usize::try_from(read::<u64>(
                    memory,
                    owner.pointer.checked_add(ccountry::COLLABORATION_STATUS)?,
                )?)
                .ok()?;
                if pointer == 0
                    || read::<u64>(memory, pointer)?
                        != engine_sites::at(base, status::VTABLE_RVA)? as u64
                    || read::<u64>(memory, pointer.checked_add(status::OWNER)?)?
                        != owner.pointer as u64
                {
                    return None;
                }
                let found = pdx_robin_hood_table::find(
                    memory,
                    pointer.checked_add(RELATIONS_BY_TARGET)?,
                    target.canonical_slot,
                )
                .ok()?;
                let Some(entry) = found else {
                    return Some(0);
                };
                let pair = relation::read_row(memory, base, entry.value, owner.id).ok()?;
                (countries.resolve(pair.target).ok()?.canonical_slot == target.canonical_slot)
                    .then_some(pair.progress)
            });
        Inputs {
            held_core: weight(HELD_CORE_WEIGHT),
            total_core: weight(TOTAL_CORE_WEIGHT),
            held_territory: weight(HELD_TERRITORY_WEIGHT),
            total_territory: weight(TOTAL_TERRITORY_WEIGHT),
            modifiers,
            collaboration,
        }
    }
}
