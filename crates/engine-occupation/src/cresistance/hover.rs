use super::*;
use crate::{ccountry, msvc_string, read};
use engine_native::{allocation, modifier_accumulator};
use engine_native_politics::cdiplomacy_status;

pub const TARGET: u32 = 0x1de;
pub const OCCUPIED_TARGET: u32 = 0x1df;
pub const DECAY: u32 = 0x1e0;
pub const GROWTH: u32 = 0x1e1;
pub const OCCUPIED_DECAY: u32 = 0x1e2;
pub const OCCUPIED_GROWTH: u32 = 0x1e3;
pub const COMPLIANCE_GROWTH: u32 = 0x1e5;
pub const OCCUPIED_COMPLIANCE_GROWTH: u32 = 0x1e6;
pub const COMPLIANCE_GAIN: u32 = 500;
pub const NO_COMPLIANCE_GAIN: u32 = 501;
pub const KEYS: [(u32, u32); 6] = [
    (TARGET, OCCUPIED_TARGET),
    (GROWTH, OCCUPIED_GROWTH),
    (DECAY, OCCUPIED_DECAY),
    (COMPLIANCE_GROWTH, OCCUPIED_COMPLIANCE_GROWTH),
    (COMPLIANCE_GAIN, 0),
    (NO_COMPLIANCE_GAIN, 0),
];
pub const FLOW_FROM: usize = 0x30;
pub const FLOW: usize = 0x38;
pub const ADDED_TARGETS: usize = 0x258;
pub const ADDED_TARGET_STRIDE: usize = 0x48;
pub const ADDED_VALUE: usize = 0x10;
pub const ADDED_CONTROLLER: usize = 0x1c;
pub const ADDED_OCCUPIED: usize = 0x20;
pub const ADDED_LOCALISATION: usize = 0x28;
pub const GAIN_RATIO: usize = 0x2d0;
pub const EXILE_HOST: usize = 0x1f0;
pub const LEGITIMACY: usize = 0x1f8;
pub const RELATION_MODIFIERS: usize = 0x140;
pub const MODIFIER_ACCUMULATOR: usize = 0x10;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Added {
    pub value: i64,
    pub controller: i32,
    pub occupied: i32,
    pub localisation: Vec<u8>,
}

pub use game_model::occupation_effect::EffectExtra;

const EFFECT_CHILD_COUNT: usize = 0x24;
const EFFECT_CUSTOM_TOOLTIP_COUNT: usize = 0x78;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Inputs {
    pub resistance: Row,
    pub controller: u32,
    pub exile_host: i32,
    pub legitimacy: i64,
    pub flow_from: u32,
    pub flow: i64,
    pub gain_ratio: i64,
    pub added: Vec<Added>,
    pub sources: [usize; 4],
    pub receivers: [usize; 7],
    pub effects: [Vec<(u32, i64)>; 2],
    pub effect_extras: [EffectExtra; 2],
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("occupation hover pointer invalid or overflow")
}
fn field<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("occupation hover field unreadable")
}

pub fn read_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    source: cstate::Source,
) -> Result<Inputs, &'static str> {
    let resistance = super::read_state(memory, base, source.pointer, source.id)?;
    let mut result = Inputs {
        resistance,
        receivers: [source.pointer, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    if resistance.active == 0 && resistance.country as i32 <= 0 {
        return Ok(result);
    }
    let object = at(source.pointer, cstate::RESISTANCE)?;
    let controller_raw = field::<u32>(memory, source.pointer, cstate::CONTROLLER)?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let resolve = |raw| {
        if raw == 0 || countries.link(raw)?.is_none() {
            countries.reserved()
        } else {
            countries.resolve(raw)
        }
    };
    result.controller = controller_raw;
    let controller = resolve(controller_raw)?;
    let occupied = resolve(resistance.country)?;
    let diplomacy = field::<u64>(memory, controller.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
    let occupied_diplomacy =
        field::<u64>(memory, occupied.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
    let diplomacy_vtable = engine_sites::at(base, cdiplomacy_status::VTABLE_RVA)
        .ok_or("occupation hover diplomacy identity mismatch")? as u64;
    for (diplomacy, country) in [
        (diplomacy, controller.pointer),
        (occupied_diplomacy, occupied.pointer),
    ] {
        if read::<u64>(memory, diplomacy) != Some(diplomacy_vtable)
            || field::<u64>(memory, diplomacy, cdiplomacy_status::COUNTRY)? != country as u64
        {
            return Err("occupation hover diplomacy identity mismatch");
        }
    }
    result.exile_host = field(memory, occupied_diplomacy, EXILE_HOST)?;
    result.legitimacy = field(memory, occupied_diplomacy, LEGITIMACY)?;
    let host_diplomacy = if result.exile_host > 0 {
        let host = resolve(result.exile_host as u32)?;
        let diplomacy = field::<u64>(memory, host.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
        if read::<u64>(memory, diplomacy) != Some(diplomacy_vtable)
            || field::<u64>(memory, diplomacy, cdiplomacy_status::COUNTRY)? != host.pointer as u64
        {
            return Err("occupation exile host diplomacy identity mismatch");
        }
        diplomacy
    } else {
        0
    };
    result.flow = field(memory, object, FLOW)?;
    result.gain_ratio = field(memory, object, GAIN_RATIO)?;
    if result.flow > 0 {
        let state = field::<u64>(memory, object, FLOW_FROM)? as usize;
        if state != 0 {
            let expected = engine_sites::at(base, cstate::VTABLE_RVA)
                .ok_or("occupation resistance flow state identity mismatch")?;
            if read::<u64>(memory, state) != Some(expected as u64) {
                return Err("occupation resistance flow state identity mismatch");
            }
            result.flow_from = field(memory, state, cstate::ID)?;
        }
    }
    let targets = Array::read(memory, at(object, ADDED_TARGETS)?, ADDED_TARGET_STRIDE)
        .ok_or("added resistance targets header unreadable")?;
    allocation::reserve(&mut result.added, targets.count as usize)
        .map_err(|_| "occupation hover allocation failed")?;
    for index in 0..targets.count {
        let target = targets
            .element(index, ADDED_TARGET_STRIDE)
            .ok_or("added resistance target address overflow")?;
        let mut value = Added {
            value: field(memory, target, ADDED_VALUE)?,
            controller: field(memory, target, ADDED_CONTROLLER)?,
            occupied: field(memory, target, ADDED_OCCUPIED)?,
            ..Default::default()
        };
        let mut len = 0;
        msvc_string::append(
            memory,
            at(target, ADDED_LOCALISATION)?,
            &mut value.localisation,
            &mut len,
        )?;
        value.localisation.truncate(len);
        result.added.push(value);
    }
    let relations = Array::read(memory, at(diplomacy, cdiplomacy_status::RELATIONS)?, 8)
        .ok_or("occupation hover relation matrix unreadable")?;
    let slot = countries.link(resistance.country)?.unwrap_or(0);
    let relation = relations
        .element(slot, 8)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("occupation hover relation slot unreadable")? as usize;
    if relation != 0 {
        let expected = engine_sites::at(base, engine_native::crelation_status::VTABLE_RVA)
            .ok_or("occupation relation identity mismatch")?;
        if read::<u64>(memory, relation) != Some(expected as u64)
            || field::<u64>(
                memory,
                relation,
                engine_native::crelation_status::DIPLOMACY_STATUS,
            )? != diplomacy as u64
        {
            return Err("occupation relation identity mismatch");
        }
        let target = field::<u32>(
            memory,
            relation,
            engine_native::crelation_status::TARGET_COUNTRY,
        )?;
        if countries.link(target)?.unwrap_or(0) != slot {
            return Err("occupation relation target mismatch");
        }
    }
    result.receivers = [
        source.pointer,
        controller.pointer,
        occupied.pointer,
        occupied_diplomacy,
        relation,
        at(source.pointer, cstate::COUNTRY_MODIFIERS)?,
        host_diplomacy,
    ];
    result.sources = [
        at(controller.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
        cstate::country_modifier_accumulator(memory, game_state, source.pointer, controller_raw)?,
        if relation == 0 {
            0
        } else {
            at(relation, RELATION_MODIFIERS)?
        },
        at(occupied.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
    ];
    for ((entries, extra), offset) in result
        .effects
        .iter_mut()
        .zip(result.effect_extras.iter_mut())
        .zip([RESISTANCE_MODIFIER, COMPLIANCE_MODIFIER])
    {
        let root = at(object, offset + MODIFIER_ACCUMULATOR)?;
        extra.child_sources = u32::try_from(field::<i32>(memory, root, EFFECT_CHILD_COUNT)?)
            .map_err(|_| "occupation effect child count invalid")?;
        extra.custom_tooltips =
            u32::try_from(field::<i32>(memory, root, EFFECT_CUSTOM_TOOLTIP_COUNT)?)
                .map_err(|_| "occupation effect custom tooltip count invalid")?;
        let mut failed = false;
        modifier_accumulator::entries_at(memory, root, &mut |key, value| {
            if allocation::reserve(entries, 1).is_err() {
                failed = true;
            } else {
                entries.push((key, value));
            }
        })?;
        if failed {
            return Err("occupation effect allocation failed");
        }
        let definitions = engine_native::modifier_definition::DefinitionMemo::default();
        let mut used = 0;
        for index in 0..entries.len() {
            let (key, raw) = entries[index];
            let (value, flags) = engine_native::modifier_definition::effective_observed_memo(
                memory,
                base,
                key,
                raw,
                &definitions,
            )?;
            if flags.is_some() {
                entries[used] = (key, value);
                used += 1;
            }
        }
        entries.truncate(used);
    }
    Ok(result)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;
    fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
        bytes[at..at + N].copy_from_slice(&value);
    }
    fn array(bytes: &mut [u8], at: usize, data: u64, count: u32) {
        put(bytes, at, data.to_le_bytes());
        put(bytes, at + 8, count.to_le_bytes());
        put(bytes, at + 12, count.to_le_bytes());
    }
    pub(in crate::cresistance) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x14000];
        array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x11000, 3);
        array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x11100, 4);
        for (slot, country, diplomacy) in [(1_u32, 0x2000, 0x9000), (2, 0x4000, 0xa000)] {
            put(
                &mut bytes,
                0x11000 + slot as usize * 8,
                (country as u64).to_le_bytes(),
            );
            put(&mut bytes, 0x11100 + slot as usize * 4, slot.to_le_bytes());
            put(
                &mut bytes,
                country,
                (ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, country + ccountry::ID, slot.to_le_bytes());
            put(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                (diplomacy as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy,
                (cdiplomacy_status::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy + cdiplomacy_status::COUNTRY,
                (country as u64).to_le_bytes(),
            );
        }
        put(&mut bytes, 0x11100 + 12, 2_u32.to_le_bytes());
        for (pointer, id) in [(0x6000, 7_u32), (0x7000, 9)] {
            put(
                &mut bytes,
                pointer,
                (cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + cstate::ID, id.to_le_bytes());
        }
        put(&mut bytes, 0x6000 + cstate::CONTROLLER, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x6000 + cstate::COUNTRY_MODIFIERS,
            0x12000_u64.to_le_bytes(),
        );
        let object = 0x6000 + cstate::RESISTANCE;
        put(&mut bytes, object, (VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, object + STATE, 0x6000_u64.to_le_bytes());
        put(&mut bytes, object + COUNTRY, 3_u32.to_le_bytes());
        put(&mut bytes, object + ACTIVE, [1]);
        put(&mut bytes, object + LEVEL, 1_234_567_i64.to_le_bytes());
        put(&mut bytes, object + FLOW_FROM, 0x7000_u64.to_le_bytes());
        put(&mut bytes, object + FLOW, 456_789_i64.to_le_bytes());
        put(&mut bytes, object + GAIN_RATIO, 76_543_i64.to_le_bytes());
        put(&mut bytes, 0xa000 + EXILE_HOST, 1_i32.to_le_bytes());
        put(&mut bytes, 0xa000 + LEGITIMACY, 7_654_321_i64.to_le_bytes());
        array(
            &mut bytes,
            0x9000 + cdiplomacy_status::RELATIONS,
            0x11200,
            3,
        );
        put(&mut bytes, 0x11200 + 16, 0xb000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xb000,
            (engine_native::crelation_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb000 + engine_native::crelation_status::DIPLOMACY_STATUS,
            0x9000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb000 + engine_native::crelation_status::TARGET_COUNTRY,
            2_u32.to_le_bytes(),
        );
        array(&mut bytes, object + ADDED_TARGETS, 0x13000, 1);
        put(
            &mut bytes,
            0x13000 + ADDED_VALUE,
            (-12_345_678_i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x13000 + ADDED_CONTROLLER,
            (-1_i32).to_le_bytes(),
        );
        put(&mut bytes, 0x13000 + ADDED_OCCUPIED, 2_i32.to_le_bytes());
        let text = 0x13000 + ADDED_LOCALISATION;
        bytes[text..text + 7].copy_from_slice(b"fixture");
        put(&mut bytes, text + 16, 7_u64.to_le_bytes());
        put(&mut bytes, text + 24, 15_u64.to_le_bytes());
        bytes
    }
    #[test]
    fn occupation_hover_reads_live_caches_aliases_and_ordered_scripted_targets() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let row = read_inputs(
            &memory,
            0,
            0x1000,
            cstate::Source {
                pointer: 0x6000,
                id: 7,
            },
        )
        .unwrap();
        assert_eq!(row.resistance.level, 1_234_567);
        assert_eq!((row.exile_host, row.legitimacy), (1, 7_654_321));
        assert_eq!(row.receivers[3], 0xa000);
        assert_eq!(row.receivers[6], 0x9000);
        assert_eq!(
            (row.flow_from, row.flow, row.gain_ratio),
            (9, 456_789, 76_543)
        );
        assert_eq!(
            row.sources,
            [
                0x2000 + ccountry::MODIFIER_ACCUMULATOR,
                0x6000 + cstate::BASE_MODIFIERS,
                0xb000 + RELATION_MODIFIERS,
                0x4000 + ccountry::MODIFIER_ACCUMULATOR
            ]
        );
        assert_eq!(
            row.added,
            [Added {
                value: -12_345_678,
                controller: -1,
                occupied: 2,
                localisation: b"fixture".to_vec()
            }]
        );
        assert!(row.effects.iter().all(Vec::is_empty));
    }
    #[test]
    fn occupation_marker_inactive_positive_occupied_objects_keep_detailed_hover_operands() {
        let mut bytes = fixture();
        bytes[0x6000 + cstate::RESISTANCE + ACTIVE] = 0;
        let input = read_inputs(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0x1000,
            cstate::Source {
                pointer: 0x6000,
                id: 7,
            },
        )
        .unwrap();
        assert_eq!(input.resistance.active, 0);
        assert_eq!(input.resistance.country, 3);
        assert_eq!(input.flow, 456_789);
        assert_eq!(input.receivers[1], 0x2000);
        assert_eq!(input.added.len(), 1);
    }
    #[test]
    fn occupation_exile_host_dependency_requires_authenticated_diplomacy() {
        let mut bytes = fixture();
        put(&mut bytes, 0xa000 + EXILE_HOST, 99_i32.to_le_bytes());
        assert!(
            read_inputs(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x1000,
                cstate::Source {
                    pointer: 0x6000,
                    id: 7
                }
            )
            .is_err()
        );
        put(&mut bytes, 0xa000 + EXILE_HOST, 0_i32.to_le_bytes());
        let row = read_inputs(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0x1000,
            cstate::Source {
                pointer: 0x6000,
                id: 7,
            },
        )
        .unwrap();
        assert_eq!(row.receivers[6], 0);
    }

    #[test]
    fn occupation_hover_effects_use_the_one_stored_accumulator_read() {
        use std::cell::Cell;
        let mut bytes = fixture();
        let root = 0x6000 + cstate::RESISTANCE + RESISTANCE_MODIFIER + MODIFIER_ACCUMULATOR;
        array(&mut bytes, root, 0x13100, 2);
        put(&mut bytes, 0x13100, 2_u32.to_le_bytes());
        put(&mut bytes, 0x13108, 123_456_i64.to_le_bytes());
        put(&mut bytes, 0x13110, 5_u32.to_le_bytes());
        put(&mut bytes, 0x13118, (-765_432_i64).to_le_bytes());
        struct EffectMemory<'a>(&'a [u8], Cell<usize>, bool);
        impl Memory for EffectMemory<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if address == engine_native::modifier_definition::TABLE_RVA.0 {
                    out.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
                    return true;
                }
                if address == engine_native::modifier_definition::ELIGIBILITY_CALLBACK_RVA.0 {
                    out.copy_from_slice(&0_u64.to_le_bytes());
                    return true;
                }
                if (0x3000_0000..0x3004_0000).contains(&address) {
                    out.fill(0);
                    if self.2
                        && (address - 0x3000_0000) % engine_native::modifier_definition::STRIDE
                            == engine_native::modifier_definition::FEATURE_SELECTOR
                    {
                        out.copy_from_slice(&1_u32.to_le_bytes());
                    }
                    return true;
                }
                if (0x13100..0x13120).contains(&address) {
                    self.1.set(self.1.get() + 1);
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, out)
            }
        }
        put(
            &mut bytes,
            root + EFFECT_CUSTOM_TOOLTIP_COUNT,
            3_i32.to_le_bytes(),
        );
        put(&mut bytes, root + EFFECT_CHILD_COUNT, 1_i32.to_le_bytes());
        let memory = EffectMemory(&bytes, Cell::new(0), false);
        let input = read_inputs(
            &memory,
            0,
            0x1000,
            cstate::Source {
                pointer: 0x6000,
                id: 7,
            },
        )
        .unwrap();
        assert_eq!(input.effects[0], [(2, 123_456), (5, -765_432)]);
        assert_eq!(input.effect_extras[0].custom_tooltips, 3);
        assert_eq!(input.effect_extras[0].child_sources, 1);
        let disabled = read_inputs(
            &EffectMemory(&bytes, Cell::new(0), true),
            0,
            0x1000,
            cstate::Source {
                pointer: 0x6000,
                id: 7,
            },
        )
        .unwrap();
        assert!(disabled.effects[0].is_empty());
        assert_eq!(disabled.effect_extras[0].custom_tooltips, 3);
        assert!(
            memory.1.get() > 0 && memory.1.get() <= 2,
            "effect values must not be looked up again after their census"
        );
    }

    #[test]
    fn occupation_hover_rejects_a_foreign_flow_state_and_a_mismatched_diplomacy_owner() {
        let mut bytes = fixture();
        put(&mut bytes, 0x7000, 0_u64.to_le_bytes());
        let source = cstate::Source {
            pointer: 0x6000,
            id: 7,
        };
        assert_eq!(
            read_inputs(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x1000,
                source
            )
            .unwrap_err(),
            "occupation resistance flow state identity mismatch"
        );
        put(
            &mut bytes,
            0xa000 + cdiplomacy_status::COUNTRY,
            0x2000_u64.to_le_bytes(),
        );
        assert_eq!(
            read_inputs(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x1000,
                source
            )
            .unwrap_err(),
            "occupation hover diplomacy identity mismatch"
        );
    }
}
