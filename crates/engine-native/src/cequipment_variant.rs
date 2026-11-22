
use engine_sites::Rva;
mod design;
pub mod mutations;
pub mod source_mutations;
pub use design::*;

use crate::{
    Memory, cequipment_type, cgame_state,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_string, read, read_slice,
    walk::Outcome,
};

pub const VTABLE_RVA: Rva = Rva(0x029c_e250);
pub use engine_executable::native_values::equipment_variant::REFERENCE_TYPE;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const EQUIPMENT_TOKEN: usize = 0x18;
pub const CREATOR_LINK: usize = 0x1c;
pub const ORIGIN_LINK: usize = 0x20;
#[cfg(any(test, feature = "test-support"))]
pub const APPEND_VARIANT_NAME: usize = 0x24;
pub const DISPLAY_NAME: usize = 0x28;
pub const VARIANT_NAME: usize = 0x48;
pub const UPGRADES: usize = 0x68;
pub const UPGRADES_COUNT: usize = 0x7c;
pub const MODULES: usize = 0xb0;
pub const MODULES_COUNT: usize = 0xc4;
pub const AIR_RANGE_STAT: usize = 0x270;
pub const AIR_DEFENCE_STAT: usize = 0x278;
pub const AIR_ATTACK_STAT: usize = 0x280;
pub const AIR_AGILITY_STAT: usize = 0x288;
pub const MAXIMUM_SPEED_STAT: usize = 0x2f0;
pub const RELIABILITY_STAT: usize = 0x308;
pub const WEIGHT_STAT: usize = 0x318;
pub const THRUST_STAT: usize = 0x320;
pub const FUEL_CONSUMPTION_STAT: usize = 0x328;
pub const BUILD_COST_IC_STAT: usize = 0x368;
pub const THRUST_WEIGHT_AGILITY_FACTOR_RVA: Rva = Rva(0x033e_59f8);
pub const STAT_LANE_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatLane {
    pub stat_index: usize,
    pub offset: usize,
    pub name: &'static str,
    pub mismatch_reason: &'static str,
}

pub const STAT_LANES: [StatLane; STAT_LANE_COUNT] = [
    StatLane {
        stat_index: 46,
        offset: AIR_RANGE_STAT,
        name: "air_range",
        mismatch_reason: "variant stat air_range disagrees with base definition",
    },
    StatLane {
        stat_index: 47,
        offset: AIR_DEFENCE_STAT,
        name: "air_defence",
        mismatch_reason: "variant stat air_defence disagrees with base definition",
    },
    StatLane {
        stat_index: 48,
        offset: AIR_ATTACK_STAT,
        name: "air_attack",
        mismatch_reason: "variant stat air_attack disagrees with base definition",
    },
    StatLane {
        stat_index: 49,
        offset: AIR_AGILITY_STAT,
        name: "air_agility",
        mismatch_reason: "variant stat air_agility disagrees with base definition",
    },
    StatLane {
        stat_index: 62,
        offset: MAXIMUM_SPEED_STAT,
        name: "maximum_speed",
        mismatch_reason: "variant stat maximum_speed disagrees with base definition",
    },
    StatLane {
        stat_index: 65,
        offset: RELIABILITY_STAT,
        name: "reliability",
        mismatch_reason: "variant stat reliability disagrees with base definition",
    },
    StatLane {
        stat_index: 67,
        offset: WEIGHT_STAT,
        name: "weight",
        mismatch_reason: "variant stat weight disagrees with base definition",
    },
    StatLane {
        stat_index: 68,
        offset: THRUST_STAT,
        name: "thrust",
        mismatch_reason: "variant stat thrust disagrees with base definition",
    },
    StatLane {
        stat_index: 69,
        offset: FUEL_CONSUMPTION_STAT,
        name: "fuel_consumption",
        mismatch_reason: "variant stat fuel_consumption disagrees with base definition",
    },
    StatLane {
        stat_index: 77,
        offset: BUILD_COST_IC_STAT,
        name: "build_cost_ic",
        mismatch_reason: "variant stat build_cost_ic disagrees with base definition",
    },
];

const _: () = {
    let mut i = 0;
    while i < STAT_LANE_COUNT {
        assert!(STAT_LANES[i].offset == STATS + STAT_LANES[i].stat_index * 8);
        i += 1;
    }
};
pub const MISSION_GROUP_DATA: usize = 0x370;
pub const DEFINITION: usize = 0x3f0;
pub const CLASSIFIER_FLAGS: usize = 0x408;
pub const CONVOY_CLASSIFIER_MASK: u64 = 1;
pub const CAPITAL_SHIP_CLASSIFIER_MASK: u64 = 1 << 6;
pub const FLOATING_HARBOR_CLASSIFIER_MASK: u64 = 1 << 9;
pub const MISSION_MASK: usize = 0x410;
pub const VERSION: usize = 0x41c;
pub const OBSOLETE: usize = 0x424;
pub const EXPLICIT_ENTITY: usize = 0x430;
pub const ICON_OVERRIDE_ACTIVE: usize = 0x450;
pub const ICON_OVERRIDE_KIND: usize = 0x454;
pub const ICON_OVERRIDE_TOKEN: usize = 0x458;
pub const ICON_ENTRY_RECORDED: usize = 0x28;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00c2_6060);

pub fn identity(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("variant module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("variant identity unreadable")?;
    identity_header(expected, bytes)
}

pub fn identity_from_header(expected: usize, bytes: [u8; 16]) -> Result<u64, &'static str> {
    identity_header(expected, bytes)
}

fn identity_header(expected: usize, bytes: [u8; 16]) -> Result<u64, &'static str> {
    let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let handle = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if vtable != expected as u64 || handle as u32 != REFERENCE_TYPE {
        return Err("variant vtable or reference type mismatch");
    }
    Ok(handle)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegistryRow {
    pub id: u64,
    address: usize,
}

impl RegistryRow {
    pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Self, &'static str> {
        if pointer == 0 {
            return Err("variant pointer is null");
        }
        Ok(Self {
            id: identity(memory, base, pointer)?,
            address: pointer,
        })
    }
    pub fn pointer(self) -> usize {
        self.address
    }
}

pub fn walk_registry(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<RegistryRow>,
) -> Outcome {
    let result = (|| {
        let singleton = engine_sites::at(base, cgame_state::SINGLETON_RVA)
            .ok_or("variant registry module address overflow")?;
        if game_state == 0 || read::<u64>(memory, singleton) != Some(game_state as u64) {
            return Err("variant registry game state unreadable or changed");
        }
        let array = Array::read(memory, at(game_state, cgame_state::EQUIPMENT_VARIANTS)?, 8)
            .ok_or("variant registry array unreadable or invalid")?;
        let expected = engine_sites::at(base, VTABLE_RVA)
            .ok_or("variant module address overflow")?;
        const BATCH: usize = 64;
        let mut pointer_bytes = [0_u8; BATCH * 8];
        let mut headers = [[0_u8; 16]; BATCH];
        let mut first = 0_u32;
        let mut len = 0;
        while first < array.count {
            let count = ((array.count - first) as usize).min(BATCH);
            let source = array
                .element(first, 8)
                .ok_or("variant registry array address overflow")?;
            if !read_slice(memory, source, &mut pointer_bytes[..count * 8]) {
                return Err("variant registry pointers unreadable");
            }
            let mut addresses = [0_usize; BATCH];
            for (address, bytes) in addresses
                .iter_mut()
                .zip(pointer_bytes.chunks_exact(8))
                .take(count)
            {
                *address = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("variant registry pointer null or invalid")?;
            }
            let retained = count;
            if retained != 0 {
                let mut fields = headers.iter_mut().enumerate();
                let copied = memory.copy_scattered(
                    addresses,
                    &mut std::array::from_fn(|_| {
                        let (slot, bytes) = fields.next().unwrap();
                        &mut bytes[..if slot < retained { 16 } else { 0 }]
                    }),
                );
                let required = len + copied.min(retained);
                if required > output.len() {
                    crate::allocation::reserve(output, required - output.len())
                        .map_err(|_| "variant registry allocation failed")?;
                    output.resize(required, RegistryRow::default());
                }
                for slot in 0..copied.min(retained) {
                    output[len] = RegistryRow {
                        id: identity_header(expected, headers[slot])?,
                        address: addresses[slot],
                    };
                    len += 1;
                }
                if copied < retained {
                    return Err("variant identity unreadable");
                }
            }
            first += count as u32;
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub equipment_token: u32,
    pub archetype_token: u32,
    pub creator_link: u32,
    pub origin_link: u32,
    pub append_variant_name: u8,
    pub obsolete: u8,
    pub classifier_flags: u64,
    pub mission_mask: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextSpan {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DefinitionRow<D = VariantDesignState> {
    pub identity: Row,
    pub archetype_pointer: usize,
    pub organisation_pointer: Option<usize>,
    pub source_links: Option<DesignSourceLinks>,
    pub display_name: TextSpan,
    pub variant_name: TextSpan,
    pub explicit_entity: TextSpan,
    pub icon_active: u8,
    pub icon_kind: u32,
    pub icon: TextSpan,
    pub version: u8,
    pub design: D,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOutcome {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn walk_definitions(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<DefinitionRow>,
    bytes: &mut Vec<u8>,
) -> TextOutcome {
    walk_definitions_with(memory, base, variants, output, bytes, Ok)
}

pub fn walk_definitions_with<D: Default>(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<DefinitionRow<D>>,
    bytes: &mut Vec<u8>,
    map: impl FnMut(VariantDesignState) -> Result<D, &'static str>,
) -> TextOutcome {
    walk_definitions_carried(memory, base, variants, output, bytes, |_| Ok(None), map)
}

pub fn walk_definitions_carried<D: Default>(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<DefinitionRow<D>>,
    bytes: &mut Vec<u8>,
    mut carry: impl FnMut(&RegistryRow) -> Result<Option<D>, &'static str>,
    mut map: impl FnMut(VariantDesignState) -> Result<D, &'static str>,
) -> TextOutcome {
    let result = (|| {
        let mut len = 0;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        if variants.len() > output.len() {
            crate::allocation::reserve(output, variants.len() - output.len())
                .map_err(|_| "variant definition allocation failed")?;
            output.resize_with(variants.len(), DefinitionRow::default);
        }
        const BATCH: usize = 64;
        const PREFIX_WIDTH: usize = 131 + ICON_ENTRY_RECORDED;
        const STRIDE: usize = PREFIX_WIDTH + design::DESIGN_WIDTH;
        const PREFIX_FIELDS: [RecordField; 8] = [
            RecordField {
                source_offset: EQUIPMENT_TOKEN,
                destination_offset: 0,
                len: 13,
            },
            RecordField {
                source_offset: DEFINITION,
                destination_offset: 13,
                len: 8,
            },
            RecordField {
                source_offset: OBSOLETE,
                destination_offset: 21,
                len: 1,
            },
            RecordField {
                source_offset: CLASSIFIER_FLAGS,
                destination_offset: 22,
                len: 12,
            },
            RecordField {
                source_offset: DISPLAY_NAME,
                destination_offset: 34,
                len: 64,
            },
            RecordField {
                source_offset: EXPLICIT_ENTITY,
                destination_offset: 98,
                len: 32,
            },
            RecordField {
                source_offset: ICON_OVERRIDE_ACTIVE,
                destination_offset: 130,
                len: ICON_ENTRY_RECORDED,
            },
            RecordField {
                source_offset: VERSION,
                destination_offset: 130 + ICON_ENTRY_RECORDED,
                len: 1,
            },
        ];
        const FIELDS: [RecordField; PREFIX_FIELDS.len() + design::DESIGN_FIELDS.len()] = {
            let mut result = [RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 0,
            }; PREFIX_FIELDS.len() + design::DESIGN_FIELDS.len()];
            let mut i = 0;
            while i < PREFIX_FIELDS.len() {
                result[i] = PREFIX_FIELDS[i];
                i += 1;
            }
            let mut j = 0;
            while j < design::DESIGN_FIELDS.len() {
                let field = design::DESIGN_FIELDS[j];
                result[i] = RecordField {
                    source_offset: field.source_offset,
                    destination_offset: PREFIX_WIDTH + field.destination_offset,
                    len: field.len,
                };
                i += 1;
                j += 1;
            }
            result
        };
        const EQUIPMENT_FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 12,
            },
            RecordField {
                source_offset: cequipment_type::ARCHETYPE,
                destination_offset: 12,
                len: 8,
            },
        ];
        const ARCHETYPE_FIELDS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 12,
        }];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, STRIDE).expect("valid record layout");
        const PREFIX_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&PREFIX_FIELDS, STRIDE).expect("valid prefix layout");
        const DESIGN_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&design::DESIGN_FIELDS, design::DESIGN_WIDTH)
                .expect("valid design layout");
        const EQUIPMENT_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&EQUIPMENT_FIELDS, 20).expect("valid record layout");
        const ARCHETYPE_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&ARCHETYPE_FIELDS, 12).expect("valid record layout");
        let expected = engine_sites::at(base, cequipment_type::VTABLE_RVA)
            .ok_or("equipment module address overflow")?;
        let null_expected = engine_sites::at(base, cequipment_type::NULL_VTABLE_RVA)
            .ok_or("equipment module address overflow")?;
        let equipment_class = |header: &[u8]| {
            let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
            vtable == expected as u64 || vtable == null_expected as u64
        };
        let mut pointers = [0; BATCH];
        let mut definitions = [0; BATCH];
        let mut fields = [0; BATCH * STRIDE];
        let mut equipment = [0; BATCH * 20];
        let mut archetypes = [0; BATCH];
        let mut archetype_bytes = [0; BATCH * 12];
        let mut references = crate::creference_registry::ScopedResolver::new(base);
        for sources in variants.chunks(BATCH) {
            let mut carried: [Option<D>; BATCH] = std::array::from_fn(|_| None);
            let mut misses = [0; BATCH];
            let mut miss_count = 0;
            for (index, source) in sources.iter().enumerate() {
                carried[index] = carry(source)?;
                if carried[index].is_none() {
                    misses[miss_count] = index;
                    miss_count += 1;
                }
            }
            for (pointer, source) in pointers.iter_mut().zip(sources) {
                *pointer = source.pointer();
            }
            let layout = if miss_count == sources.len() {
                &LAYOUT
            } else {
                &PREFIX_LAYOUT
            };
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..sources.len()]),
                layout,
                &mut fields,
            );
            let field_count = if miss_count == sources.len() {
                FIELDS.len()
            } else {
                PREFIX_FIELDS.len()
            };
            let ready = copied / field_count;
            let mut deep_ready = ready;
            if miss_count != 0 && miss_count != sources.len() {
                let mut demanded = [0; BATCH];
                let count = misses[..miss_count].partition_point(|index| *index < ready);
                for (pointer, index) in demanded.iter_mut().zip(&misses[..count]) {
                    *pointer = pointers[*index];
                }
                let mut deep = [0; BATCH * design::DESIGN_WIDTH];
                let copied = memory.copy_records(
                    RecordSources::Pointers(&demanded[..count]),
                    &DESIGN_LAYOUT,
                    &mut deep,
                );
                let complete = copied / design::DESIGN_FIELDS.len();
                for (ordinal, index) in misses[..complete].iter().enumerate() {
                    fields[*index * STRIDE + PREFIX_WIDTH..(*index + 1) * STRIDE].copy_from_slice(
                        &deep[ordinal * design::DESIGN_WIDTH..(ordinal + 1) * design::DESIGN_WIDTH],
                    );
                }
                if complete < count {
                    deep_ready = misses[complete];
                }
            }
            for (pointer, field) in definitions
                .iter_mut()
                .zip(fields.chunks_exact(STRIDE))
                .take(ready)
            {
                *pointer = usize::try_from(u64::from_le_bytes(field[13..21].try_into().unwrap()))
                    .unwrap_or(0);
            }
            let equipment_copied = memory.copy_records(
                RecordSources::Pointers(&definitions[..ready]),
                &EQUIPMENT_LAYOUT,
                &mut equipment,
            );
            let equipment_ready = equipment_copied / EQUIPMENT_FIELDS.len();
            for (pointer, row) in archetypes
                .iter_mut()
                .zip(equipment.chunks_exact(20))
                .take(equipment_ready)
            {
                *pointer = usize::try_from(u64::from_le_bytes(row[12..20].try_into().unwrap()))
                    .unwrap_or(0);
            }
            let archetype_copied = memory.copy_records(
                RecordSources::Pointers(&archetypes[..equipment_ready]),
                &ARCHETYPE_LAYOUT,
                &mut archetype_bytes,
            );
            for (index, variant) in sources.iter().enumerate() {
                if index >= ready {
                    return Err(match copied % field_count {
                        0 => "variant identity fields unreadable",
                        1 => "variant equipment definition unreadable or null",
                        2 => "variant obsolete byte unreadable",
                        3 => "variant classifier flags or mission mask unreadable",
                        4 => "variant string headers unreadable",
                        5 => "variant explicit entity header unreadable",
                        6 => "variant icon entry unreadable",
                        7 => "variant version byte unreadable",
                        _ => "variant design fields unreadable",
                    });
                }
                if index >= deep_ready {
                    return Err("variant design fields unreadable");
                }
                if definitions[index] == 0 {
                    return Err("variant equipment definition unreadable or null");
                }
                if equipment_copied < (index + 1) * EQUIPMENT_FIELDS.len() {
                    return Err("equipment definition identity unreadable");
                }
                if archetypes[index] == 0 || archetype_copied <= index {
                    return Err("equipment archetype unreadable or null");
                }
                let field = &fields[index * STRIDE..(index + 1) * STRIDE];
                let definition = &equipment[index * 20..(index + 1) * 20];
                let archetype = &archetype_bytes[index * 12..(index + 1) * 12];
                let equipment_token = u32::from_le_bytes(field[..4].try_into().unwrap());
                if !equipment_class(definition)
                    || u32::from_le_bytes(definition[8..12].try_into().unwrap()) != equipment_token
                {
                    return Err("variant equipment definition identity mismatch");
                }
                if !equipment_class(archetype) {
                    return Err("variant equipment archetype identity mismatch");
                }
                let row = Row {
                    id: variant.id,
                    equipment_token,
                    archetype_token: u32::from_le_bytes(archetype[8..].try_into().unwrap()),
                    creator_link: u32::from_le_bytes(field[4..8].try_into().unwrap()),
                    origin_link: u32::from_le_bytes(field[8..12].try_into().unwrap()),
                    append_variant_name: field[12],
                    obsolete: field[21],
                    classifier_flags: u64::from_le_bytes(field[22..30].try_into().unwrap()),
                    mission_mask: u32::from_le_bytes(field[30..34].try_into().unwrap()),
                };
                let strings = [
                    Literal::from_header(field[34..66].try_into().unwrap())?,
                    Literal::from_header(field[66..98].try_into().unwrap())?,
                    Literal::from_header(field[98..130].try_into().unwrap())?,
                    Literal::from_header(field[138..170].try_into().unwrap())?,
                ];
                let wanted = strings
                    .iter()
                    .try_fold(0_usize, |n, s| n.checked_add(s.len))
                    .ok_or("variant string size overflow")?;
                required_bytes = required_bytes
                    .checked_add(wanted)
                    .ok_or("variant string size overflow")?;
                let mut spans = [TextSpan::default(); 4];
                for (span, string) in spans.iter_mut().zip(&strings) {
                    let size = string.len;
                    string.copy_growing(
                        memory,
                        bytes,
                        bytes_len,
                        "variant name allocation failed",
                    )?;
                    *span = TextSpan {
                        start: bytes_len,
                        len: size,
                    };
                    bytes_len += size;
                }
                let (design, organisation_pointer, source_links) =
                    if let Some(carried) = carried[index].take() {
                        (carried, None, None)
                    } else {
                        let (state, pointer, links) = design::read_design_prefetched(
                            memory,
                            base,
                            variant.pointer(),
                            &field[PREFIX_WIDTH..],
                            &mut references,
                        )?;
                        (map(state)?, pointer, Some(links))
                    };
                output[len] = DefinitionRow {
                    identity: row,
                    archetype_pointer: archetypes[index],
                    organisation_pointer,
                    source_links,
                    display_name: spans[0],
                    variant_name: spans[1],
                    explicit_entity: spans[2],
                    icon_active: field[130],
                    icon_kind: u32::from_le_bytes(field[134..138].try_into().unwrap()),
                    icon: spans[3],
                    version: field[130 + ICON_ENTRY_RECORDED],
                    design,
                };
                len += 1;
            }
        }
        Ok(TextOutcome {
            outcome: Outcome::Rows { len },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(text_failure)
}

pub use engine_executable::native_values::equipment_variant::STAT_COUNT;
pub const STATS: usize = 0x100;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatsRow {
    pub variant: u64,
    pub values: [Option<i64>; STAT_LANE_COUNT],
    pub organisation: Organisation,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Organisation {
    pub author: u64,
    pub trait_count: i32,
    pub bonuses: Vec<OrganisationBonus>,
}
pub use game_model::equipment::OrganisationBonus;

pub const INDUSTRIAL_ORGANISATION: usize = 0x488;
pub const ORGANISATION_BONUSES: usize = 0x498;
pub const ORGANISATION_BONUSES_COUNT: usize = 0x4a4;
pub const ORGANISATION_TRAIT_COUNT: usize = 0x4b0;
pub const STAT_FIXED_ONE: i64 = 100_000;
pub const AIRCRAFT_DESIGNER_PLANE_MASK: u64 = 0x1d00_26fc_00;
pub const BY_BLOOD_ALONE_MASK: u32 = 0x0000_0100;
const DLC_FEATURE_MASK_RVA: Rva = crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuietToken {
    pub epoch: u64,
    pub revision: u64,
    pub world_epoch: u64,
    pub world_revision: u64,
}

impl QuietToken {
    pub const fn new(epoch: u64, revision: u64) -> Self {
        Self {
            epoch,
            revision,
            world_epoch: 0,
            world_revision: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedStat {
    pub variant_id: u64,
    pub classifier_flags: u64,
    pub value: i64,
}

pub trait StatWitness {
    fn quiet(&self) -> Result<QuietToken, &'static str> {
        Err("variant stat writer witness unavailable")
    }
    fn record_withheld(&mut self, _lane: usize, _reason: &'static str) {}
}

impl StatWitness for () {}

#[cfg(test)]
struct TestQuietWitness;

#[cfg(test)]
impl StatWitness for TestQuietWitness {
    fn quiet(&self) -> Result<QuietToken, &'static str> {
        Ok(QuietToken::new(0, 0))
    }
}

#[derive(Clone, Copy, Default)]
struct StatAccumulator {
    add: i64,
    avg_sum: i64,
    avg_count: i32,
    mult: i64,
}

impl StatAccumulator {
    fn finalize(self) -> Option<i64> {
        let mut value = self.add;
        if self.avg_count > 0 {
            value = value.checked_add(self.avg_sum / i64::from(self.avg_count))?;
        }
        if self.mult == 0 {
            return Some(value);
        }
        let factor = i128::from(self.mult).checked_add(i128::from(STAT_FIXED_ONE))?;
        let scaled = i128::from(value).checked_mul(factor)? / i128::from(STAT_FIXED_ONE);
        i64::try_from(scaled).ok()
    }
}

fn lane_for_stat_index(stat_index: i32) -> Option<usize> {
    match stat_index {
        46 => Some(0),
        47 => Some(1),
        48 => Some(2),
        49 => Some(3),
        62 => Some(4),
        65 => Some(5),
        67 => Some(6),
        68 => Some(7),
        69 => Some(8),
        77 => Some(9),
        _ => None,
    }
}

struct WitnessSites {
    rebuild_located: bool,
    variant_vtable: Option<usize>,
    def_vtable: Option<usize>,
    null_def_vtable: Option<usize>,
    module_vtable: Option<usize>,
    upgrade_vtable: Option<usize>,
    specific_bonus_vtable: Option<usize>,
    trait_bonus_vtable: Option<usize>,
    dlc_mask: Option<Option<u32>>,
    thrust_factor: Option<Option<i64>>,
}

impl WitnessSites {
    fn resolve(base: usize) -> Self {
        Self {
            rebuild_located: engine_sites::resolve(base, mutations::REBUILD_STATS_RVA).is_ok(),
            variant_vtable: engine_sites::resolve(base, VTABLE_RVA).ok(),
            def_vtable: engine_sites::resolve(base, cequipment_type::VTABLE_RVA).ok(),
            null_def_vtable: engine_sites::resolve(base, cequipment_type::NULL_VTABLE_RVA).ok(),
            module_vtable: engine_sites::resolve(base, crate::cequipment_module::VTABLE_RVA).ok(),
            upgrade_vtable: engine_sites::resolve(base, crate::cequipment_upgrade::VTABLE_RVA).ok(),
            specific_bonus_vtable: engine_sites::resolve(base, SPECIFIC_BONUS_VTABLE_RVA).ok(),
            trait_bonus_vtable: engine_sites::resolve(base, TRAIT_BONUS_VTABLE_RVA).ok(),
            dlc_mask: None,
            thrust_factor: None,
        }
    }

    fn clear_live_operands(&mut self) {
        self.dlc_mask = None;
        self.thrust_factor = None;
    }

    fn dlc_mask(&mut self, memory: &impl Memory, base: usize) -> Option<u32> {
        *self.dlc_mask.get_or_insert_with(|| {
            engine_sites::resolve(base, DLC_FEATURE_MASK_RVA)
                .ok()
                .and_then(|at| read::<u32>(memory, at))
        })
    }

    fn thrust_factor(&mut self, memory: &impl Memory, base: usize) -> Option<i64> {
        *self.thrust_factor.get_or_insert_with(|| {
            engine_sites::resolve(base, THRUST_WEIGHT_AGILITY_FACTOR_RVA)
                .ok()
                .and_then(|at| read::<i64>(memory, at))
        })
    }
}

fn stat_pair_array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    reason: &'static str,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, 16).ok_or(reason)
}

fn read_stat_pair(
    memory: &impl Memory,
    array: Array,
    index: u32,
    reason: &'static str,
) -> Result<(i32, i64), &'static str> {
    let entry = array.element(index, 16).ok_or(reason)?;
    let bytes = read::<[u8; 16]>(memory, entry).ok_or(reason)?;
    Ok((
        i32::from_le_bytes(bytes[..4].try_into().unwrap()),
        i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
    ))
}

fn module_stat_factor(
    memory: &impl Memory,
    multipliers: Array,
    stat_index: i32,
) -> Result<i64, &'static str> {
    for index in 0..multipliers.count {
        let (candidate, factor) = read_stat_pair(
            memory,
            multipliers,
            index,
            "variant stat module multiplier unreadable",
        )?;
        if candidate == stat_index {
            return Ok(factor);
        }
    }
    Ok(0)
}

fn scale_fixed(value: i64, factor: i64) -> Result<i64, &'static str> {
    let scale = i128::from(factor)
        .checked_add(i128::from(STAT_FIXED_ONE))
        .ok_or("variant stat arithmetic overflow")?;
    let scaled = i128::from(value)
        .checked_mul(scale)
        .ok_or("variant stat arithmetic overflow")?
        / i128::from(STAT_FIXED_ONE);
    i64::try_from(scaled).map_err(|_| "variant stat arithmetic overflow")
}

#[derive(Clone, Copy)]
struct VariantStatMeta {
    upgrades: u32,
    modules: u32,
    bonus_sources: u32,
    named_bonuses: u32,
    org_bonuses: u32,
    resource_build_cost: Option<i64>,
    definition: usize,
    classifier_flags: u64,
    equipment_token: u32,
}

impl VariantStatMeta {
    fn from_record(field: &[u8]) -> Self {
        Self {
            upgrades: u32::from_le_bytes(field[80..84].try_into().unwrap()),
            modules: u32::from_le_bytes(field[84..88].try_into().unwrap()),
            bonus_sources: u32::from_le_bytes(field[88..92].try_into().unwrap()),
            named_bonuses: u32::from_le_bytes(field[92..96].try_into().unwrap()),
            org_bonuses: u32::from_le_bytes(field[96..100].try_into().unwrap()),
            resource_build_cost: Some(i64::from_le_bytes(field[100..108].try_into().unwrap())),
            definition: usize::try_from(u64::from_le_bytes(field[108..116].try_into().unwrap()))
                .unwrap_or(0),
            classifier_flags: u64::from_le_bytes(field[116..124].try_into().unwrap()),
            equipment_token: u32::from_le_bytes(field[124..128].try_into().unwrap()),
        }
    }

    fn read_individually(memory: &impl Memory, pointer: usize) -> Option<Self> {
        Some(Self {
            upgrades: read::<u32>(memory, pointer.checked_add(UPGRADES_COUNT)?)?,
            modules: read::<u32>(memory, pointer.checked_add(MODULES_COUNT)?)?,
            bonus_sources: read::<u32>(memory, pointer.checked_add(BONUS_SOURCES + 0x0c)?)?,
            named_bonuses: read::<u32>(memory, pointer.checked_add(NAMED_BONUSES + 0x0c)?)?,
            org_bonuses: read::<u32>(memory, pointer.checked_add(ORGANISATION_BONUSES_COUNT)?)?,
            resource_build_cost: pointer
                .checked_add(BUILD_COST)
                .and_then(|addr| read::<i64>(memory, addr)),
            definition: usize::try_from(read::<u64>(memory, pointer.checked_add(DEFINITION)?)?)
                .unwrap_or(0),
            classifier_flags: read::<u64>(memory, pointer.checked_add(CLASSIFIER_FLAGS)?)?,
            equipment_token: read::<u32>(memory, pointer.checked_add(EQUIPMENT_TOKEN)?)?,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct DefinitionStats {
    auth_ok: bool,
    type_rel_bonuses: Option<i32>,
    base_stats: [Option<i64>; STAT_LANE_COUNT],
}

impl DefinitionStats {
    fn from_record(def_slice: &[u8], expected_token: u32, sites: &WitnessSites) -> Self {
        let def_token = u32::from_le_bytes(def_slice[92..96].try_into().unwrap());
        let auth_ok = def_token == expected_token
            && sites
                .def_vtable
                .zip(sites.null_def_vtable)
                .is_some_and(|(expected, null)| {
                    let vtable = u64::from_le_bytes(def_slice[..8].try_into().unwrap());
                    vtable == expected as u64 || vtable == null as u64
                });
        let type_rel_bonuses = Some(i32::from_le_bytes(def_slice[88..92].try_into().unwrap()));
        let mut base_stats = [None; STAT_LANE_COUNT];
        for (dst, bytes) in base_stats.iter_mut().zip(def_slice[8..88].chunks_exact(8)) {
            *dst = Some(i64::from_le_bytes(bytes.try_into().unwrap()));
        }
        Self {
            auth_ok,
            type_rel_bonuses,
            base_stats,
        }
    }

    fn read_individually(
        memory: &impl Memory,
        definition: usize,
        expected_token: u32,
        sites: &WitnessSites,
    ) -> Self {
        if definition == 0 {
            return Self::default();
        }
        let vtable_ok = read::<u64>(memory, definition).is_some_and(|vtable| {
            sites
                .def_vtable
                .zip(sites.null_def_vtable)
                .is_some_and(|(expected, null)| vtable == expected as u64 || vtable == null as u64)
        });
        let token_ok = definition
            .checked_add(cequipment_type::TOKEN)
            .and_then(|addr| read::<u32>(memory, addr))
            == Some(expected_token);
        if !vtable_ok || !token_ok {
            return Self::default();
        }
        let type_rel_bonuses = definition
            .checked_add(cequipment_type::RELIABILITY_BONUS_COUNT)
            .and_then(|addr| read::<i32>(memory, addr));
        let mut base_stats = [None; STAT_LANE_COUNT];
        for (dst, stat_lane) in base_stats.iter_mut().zip(STAT_LANES) {
            *dst = definition
                .checked_add(cequipment_type::STATS_BASE + stat_lane.stat_index * 8)
                .and_then(|addr| read::<i64>(memory, addr));
        }
        Self {
            auth_ok: true,
            type_rel_bonuses,
            base_stats,
        }
    }
}

fn compute_modified_lanes(
    memory: &impl Memory,
    variant: usize,
    meta: VariantStatMeta,
    base_stats: &[Option<i64>; STAT_LANE_COUNT],
    type_rel_bonuses: i32,
    sites: &WitnessSites,
) -> Result<[Result<i64, &'static str>; STAT_LANE_COUNT], &'static str> {
    let mut acc = [StatAccumulator::default(); STAT_LANE_COUNT];
    if meta.named_bonuses > 0 {
        let handles = Array::read(memory, at(variant, NAMED_BONUSES)?, 4)
            .ok_or("variant stat named equipment bonus unreadable")?;
        for index in 0..handles.count {
            let entry = handles
                .element(index, 4)
                .ok_or("variant stat named equipment bonus unreadable")?;
            let handle = read::<i32>(memory, entry)
                .ok_or("variant stat named equipment bonus unreadable")?;
            if handle != -1 {
                return Err("variant stat named equipment bonus unverified");
            }
        }
    }
    if meta.org_bonuses > 0 {
        let bonuses = stat_pair_array(
            memory,
            variant,
            ORGANISATION_BONUSES,
            "variant stat organisation bonus unreadable",
        )?;
        for index in 0..bonuses.count {
            let (_, value) = read_stat_pair(
                memory,
                bonuses,
                index,
                "variant stat organisation bonus unreadable",
            )?;
            if value != 0 {
                return Err("variant stat organisation bonus unverified");
            }
        }
    }
    if meta.bonus_sources > 0 {
        let (Some(specific_vtable), Some(trait_vtable)) =
            (sites.specific_bonus_vtable, sites.trait_bonus_vtable)
        else {
            return Err("variant stat bonus source site unlocated");
        };
        let sources = Array::read(memory, at(variant, BONUS_SOURCES)?, 8)
            .ok_or("variant stat bonus source array unreadable")?;
        for index in 0..sources.count {
            let entry = sources
                .element(index, 8)
                .ok_or("variant stat bonus source pointer unreadable")?;
            let bonus = usize::try_from(
                read::<u64>(memory, entry).ok_or("variant stat bonus source pointer unreadable")?,
            )
            .ok()
            .filter(|&ptr| ptr != 0)
            .ok_or("variant stat bonus source pointer null")?;
            let vtable =
                read::<u64>(memory, bonus).ok_or("variant stat bonus source vtable unreadable")?;
            if vtable != specific_vtable as u64 && vtable != trait_vtable as u64 {
                return Err("variant stat bonus source identity mismatch");
            }
            let stats = stat_pair_array(
                memory,
                bonus,
                BONUS_STATS,
                "variant stat bonus source stats unreadable",
            )?;
            for stat_i in 0..stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    stats,
                    stat_i,
                    "variant stat bonus source stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    acc[lane].mult = acc[lane]
                        .mult
                        .checked_add(value)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
        }
    }
    if meta.upgrades > 0 {
        let upgrade_vtable = sites
            .upgrade_vtable
            .ok_or("variant stat upgrade site unlocated")?;
        let entries = Array::read(memory, at(variant, UPGRADES + 8)?, 16)
            .ok_or("variant stat upgrade array unreadable")?;
        for index in 0..entries.count {
            let entry = entries
                .element(index, 16)
                .ok_or("variant stat upgrade entry unreadable")?;
            let raw = read::<[u8; 16]>(memory, entry)
                .ok_or("variant stat upgrade entry unreadable")?;
            let upgrade = usize::try_from(u64::from_le_bytes(raw[..8].try_into().unwrap()))
                .ok()
                .filter(|&ptr| ptr != 0)
                .ok_or("variant stat upgrade pointer null")?;
            let level = i64::from(raw[8]);
            let vtable =
                read::<u64>(memory, upgrade).ok_or("variant stat upgrade vtable unreadable")?;
            if vtable != upgrade_vtable as u64 {
                return Err("variant stat upgrade vtable mismatch");
            }
            let add_stats = stat_pair_array(
                memory,
                upgrade,
                crate::cequipment_upgrade::ADD_STATS,
                "variant stat upgrade stats unreadable",
            )?;
            for stat_i in 0..add_stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    add_stats,
                    stat_i,
                    "variant stat upgrade stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    let delta = level
                        .checked_mul(value)
                        .ok_or("variant stat arithmetic overflow")?;
                    acc[lane].add = acc[lane]
                        .add
                        .checked_add(delta)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
            let mult_stats = stat_pair_array(
                memory,
                upgrade,
                crate::cequipment_upgrade::MULTIPLY_STATS,
                "variant stat upgrade stats unreadable",
            )?;
            for stat_i in 0..mult_stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    mult_stats,
                    stat_i,
                    "variant stat upgrade stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    let delta = level
                        .checked_mul(value)
                        .ok_or("variant stat arithmetic overflow")?;
                    acc[lane].mult = acc[lane]
                        .mult
                        .checked_add(delta)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
            let avg_stats = stat_pair_array(
                memory,
                upgrade,
                crate::cequipment_upgrade::AVERAGE_STATS,
                "variant stat upgrade stats unreadable",
            )?;
            for stat_i in 0..avg_stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    avg_stats,
                    stat_i,
                    "variant stat upgrade stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    let delta = level
                        .checked_mul(value)
                        .ok_or("variant stat arithmetic overflow")?;
                    acc[lane].avg_sum = acc[lane]
                        .avg_sum
                        .checked_add(delta)
                        .ok_or("variant stat arithmetic overflow")?;
                    acc[lane].avg_count = acc[lane]
                        .avg_count
                        .checked_add(1)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
        }
    }
    if meta.modules > 0 {
        let module_vtable = sites
            .module_vtable
            .ok_or("variant stat module site unlocated")?;
        let add_mults = stat_pair_array(
            memory,
            meta.definition,
            cequipment_type::MODULE_ADD_MULTIPLIERS,
            "variant stat module multiplier array unreadable",
        )?;
        let mult_mults = stat_pair_array(
            memory,
            meta.definition,
            cequipment_type::MODULE_MULT_MULTIPLIERS,
            "variant stat module multiplier array unreadable",
        )?;
        let entries = Array::read(memory, at(variant, MODULES + 8)?, 16)
            .ok_or("variant stat module array unreadable")?;
        for index in 0..entries.count {
            let entry = entries
                .element(index, 16)
                .ok_or("variant stat module entry unreadable")?;
            let raw =
                read::<[u8; 16]>(memory, entry).ok_or("variant stat module entry unreadable")?;
            let module = usize::try_from(u64::from_le_bytes(raw[8..16].try_into().unwrap()))
                .ok()
                .filter(|&ptr| ptr != 0)
                .ok_or("variant stat module pointer null")?;
            let vtable =
                read::<u64>(memory, module).ok_or("variant stat module vtable unreadable")?;
            if vtable != module_vtable as u64 {
                return Err("variant stat module vtable mismatch");
            }
            let add_stats = stat_pair_array(
                memory,
                module,
                crate::cequipment_module::ADD_STATS,
                "variant stat module stats unreadable",
            )?;
            for stat_i in 0..add_stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    add_stats,
                    stat_i,
                    "variant stat module stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    let factor = module_stat_factor(memory, add_mults, stat_idx)?;
                    let delta = scale_fixed(value, factor)?;
                    acc[lane].add = acc[lane]
                        .add
                        .checked_add(delta)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
            let mult_stats = stat_pair_array(
                memory,
                module,
                crate::cequipment_module::MULTIPLY_STATS,
                "variant stat module stats unreadable",
            )?;
            for stat_i in 0..mult_stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    mult_stats,
                    stat_i,
                    "variant stat module stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    let factor = module_stat_factor(memory, mult_mults, stat_idx)?;
                    let delta = scale_fixed(value, factor)?;
                    acc[lane].mult = acc[lane]
                        .mult
                        .checked_add(delta)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
            let avg_stats = stat_pair_array(
                memory,
                module,
                crate::cequipment_module::AVERAGE_STATS,
                "variant stat module stats unreadable",
            )?;
            for stat_i in 0..avg_stats.count {
                let (stat_idx, value) = read_stat_pair(
                    memory,
                    avg_stats,
                    stat_i,
                    "variant stat module stats unreadable",
                )?;
                if let Some(lane) = lane_for_stat_index(stat_idx) {
                    let factor = module_stat_factor(memory, add_mults, stat_idx)?;
                    let delta = scale_fixed(value, factor)?;
                    acc[lane].avg_sum = acc[lane]
                        .avg_sum
                        .checked_add(delta)
                        .ok_or("variant stat arithmetic overflow")?;
                    acc[lane].avg_count = acc[lane]
                        .avg_count
                        .checked_add(1)
                        .ok_or("variant stat arithmetic overflow")?;
                }
            }
        }
    }
    let mut lane_errors: [Option<&'static str>; STAT_LANE_COUNT] = [None; STAT_LANE_COUNT];
    if type_rel_bonuses != 0 {
        let rel = match base_stats[5] {
            Some(base_rel) => {
                let mut rel_acc = acc[5];
                rel_acc
                    .add
                    .checked_add(base_rel)
                    .and_then(|add| {
                        rel_acc.add = add;
                        rel_acc.finalize()
                    })
                    .map(|v| v.clamp(0, STAT_FIXED_ONE))
                    .ok_or("variant stat arithmetic overflow")
            }
            None => Err("variant definition base stat unreadable"),
        };
        let bonuses = stat_pair_array(
            memory,
            meta.definition,
            cequipment_type::RELIABILITY_BONUSES,
            "variant stat reliability bonus array unreadable",
        )?;
        for index in 0..bonuses.count {
            let (stat_idx, value) = read_stat_pair(
                memory,
                bonuses,
                index,
                "variant stat reliability bonus array unreadable",
            )?;
            if let Some(lane) = lane_for_stat_index(stat_idx) {
                if value == 0 {
                    continue;
                }
                match rel {
                    Ok(rel) => {
                        let updated = i128::from(value)
                            .checked_mul(i128::from(rel))
                            .map(|prod| prod / i128::from(STAT_FIXED_ONE))
                            .and_then(|scaled| i64::try_from(scaled).ok())
                            .and_then(|delta| acc[lane].add.checked_add(delta));
                        match updated {
                            Some(add) => acc[lane].add = add,
                            None => {
                                lane_errors[lane]
                                    .get_or_insert("variant stat arithmetic overflow");
                            }
                        }
                    }
                    Err(reason) => {
                        lane_errors[lane].get_or_insert(reason);
                    }
                }
            }
        }
    }
    let mut expected = [Err("variant definition base stat unreadable"); STAT_LANE_COUNT];
    for (((dst, mut entry), base_val), lane_error) in expected
        .iter_mut()
        .zip(acc)
        .zip(*base_stats)
        .zip(lane_errors)
    {
        *dst = if let Some(reason) = lane_error {
            Err(reason)
        } else {
            match base_val {
                Some(base_val) => entry
                    .add
                    .checked_add(base_val)
                    .and_then(|add| {
                        entry.add = add;
                        entry.finalize()
                    })
                    .ok_or("variant stat arithmetic overflow"),
                None => Err("variant definition base stat unreadable"),
            }
        };
    }
    Ok(expected)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Lane verification passes the shared memory, site cache, expected stat block and variant metadata explicitly."
)]
fn verify_lane(
    memory: &impl Memory,
    base: usize,
    sites: &mut WitnessSites,
    lane: usize,
    val: i64,
    expected: &[Result<i64, &'static str>; STAT_LANE_COUNT],
    classifier_flags: u64,
    no_design_parts: bool,
    resource_build_cost: Option<i64>,
) -> Result<(), &'static str> {
    let is_plane = (classifier_flags & cequipment_type::PLANE_MASK) != 0;
    match lane {
        1 if is_plane && val < STAT_FIXED_ONE => {
            return Err("variant stat air_defence below plane minimum");
        }
        3 if is_plane && val < STAT_FIXED_ONE => {
            return Err("variant stat air_agility below plane minimum");
        }
        4 if val < 0 => return Err("variant stat maximum_speed is negative"),
        5 if val < 0 => return Err("variant stat reliability is negative"),
        _ => {}
    }
    if lane == 9 && no_design_parts {
        let cost = resource_build_cost.ok_or("variant stat resource build cost unreadable")?;
        if val != cost {
            return Err("variant stat build_cost_ic disagrees with resource build cost");
        }
    }
    let base_val = expected[lane]?;
    let matches = match lane {
        1 => {
            val == if is_plane {
                base_val.max(STAT_FIXED_ONE)
            } else {
                base_val
            }
        }
        3 => {
            let no_thrust = if is_plane {
                base_val.max(STAT_FIXED_ONE)
            } else {
                base_val
            };
            let is_aircraft_designer_plane =
                (classifier_flags & AIRCRAFT_DESIGNER_PLANE_MASK) != 0;
            if is_aircraft_designer_plane {
                let dlc_mask = sites.dlc_mask(memory, base);
                if dlc_mask.is_some_and(|mask| (mask & BY_BLOOD_ALONE_MASK) == 0) {
                    val == no_thrust
                } else {
                    let weight = expected[6]?;
                    let thrust = expected[7]?;
                    if weight < thrust {
                        if dlc_mask.is_none() {
                            return Err("variant stat dlc feature mask unreadable");
                        }
                        let Some(factor) = sites.thrust_factor(memory, base) else {
                            return Err("variant stat thrust agility factor unreadable");
                        };
                        let Some(with_thrust) = thrust
                            .checked_sub(weight)
                            .and_then(|diff| i128::from(diff).checked_mul(i128::from(factor)))
                            .map(|prod| prod / i128::from(STAT_FIXED_ONE))
                            .and_then(|bonus| i64::try_from(bonus).ok())
                            .and_then(|bonus| bonus.checked_add(base_val))
                            .map(|v| v.max(STAT_FIXED_ONE))
                        else {
                            return Err("variant stat arithmetic overflow");
                        };
                        val == with_thrust
                    } else {
                        val == no_thrust
                    }
                }
            } else {
                val == no_thrust
            }
        }
        4 => val == base_val.max(0),
        5 => {
            if base_val > STAT_FIXED_ONE {
                return Err("variant stat reliability rebuild cap mode ambiguous");
            }
            val == base_val.max(0)
        }
        _ => val == base_val,
    };
    if matches {
        Ok(())
    } else {
        Err(STAT_LANES[lane].mismatch_reason)
    }
}

pub fn verified_cached_stat(
    memory: &impl Memory,
    base: usize,
    variant: usize,
    lane: usize,
    witness: &impl StatWitness,
) -> Result<VerifiedStat, &'static str> {
    if lane >= STAT_LANE_COUNT {
        return Err("variant stat lane index out of bounds");
    }
    let before = witness.quiet()?;
    let expected_vtable = engine_sites::resolve(base, VTABLE_RVA)
        .ok()
        .ok_or("variant vtable site unlocated")?;
    let header = read::<[u8; 16]>(memory, variant).ok_or("variant identity unreadable")?;
    let variant_id = identity_header(expected_vtable, header)?;
    let mut sites = WitnessSites::resolve(base);
    if !sites.rebuild_located {
        return Err("variant stat rebuild site unlocated");
    }
    let meta = VariantStatMeta::read_individually(memory, variant)
        .ok_or("variant stat metadata unreadable")?;
    let def = DefinitionStats::read_individually(memory, meta.definition, meta.equipment_token, &sites);
    if !def.auth_ok {
        return Err("variant definition unverified for stat cache");
    }
    let type_rel_bonuses = def
        .type_rel_bonuses
        .ok_or("variant definition unverified for stat cache")?;
    let unmodified = meta.upgrades == 0
        && meta.modules == 0
        && meta.bonus_sources == 0
        && meta.named_bonuses == 0
        && meta.org_bonuses == 0
        && type_rel_bonuses == 0;
    let expected = if unmodified {
        def.base_stats
            .map(|value| value.ok_or("variant definition base stat unreadable"))
    } else {
        compute_modified_lanes(memory, variant, meta, &def.base_stats, type_rel_bonuses, &sites)?
    };
    let val = read::<i64>(memory, at(variant, STAT_LANES[lane].offset)?)
        .ok_or("variant stat lane unreadable")?;
    let no_design_parts = meta.upgrades == 0 && meta.modules == 0 && type_rel_bonuses == 0;
    verify_lane(
        memory,
        base,
        &mut sites,
        lane,
        val,
        &expected,
        meta.classifier_flags,
        no_design_parts,
        meta.resource_build_cost,
    )?;
    let after = witness.quiet()?;
    if before != after {
        return Err("variant stat writer crossed read");
    }
    Ok(VerifiedStat {
        variant_id,
        classifier_flags: meta.classifier_flags,
        value: val,
    })
}

pub trait OrganisationCarry {
    fn fresh(&mut self, index: usize, source: &RegistryRow) -> Result<bool, &'static str>;
    fn read(&mut self, _index: usize, _source: &RegistryRow) {}
}
impl OrganisationCarry for () {
    fn fresh(&mut self, _: usize, _: &RegistryRow) -> Result<bool, &'static str> {
        Ok(true)
    }
}
#[derive(Clone, Copy)]
enum OrganisationReadError {
    Identity(&'static str),
    Fault(&'static str),
}

fn verify_live_header(
    expected_vtable: Option<usize>,
    header: Option<[u8; 16]>,
    expected_id: u64,
    changed_reason: &'static str,
) -> Result<(), &'static str> {
    let expected_vtable = expected_vtable.ok_or("variant vtable site unlocated")?;
    let header = header.ok_or("variant identity unreadable")?;
    match identity_header(expected_vtable, header) {
        Ok(live_id) if live_id == expected_id => Ok(()),
        Ok(_) => Err(changed_reason),
        Err(reason) => Err(reason),
    }
}

fn read_organisation_verified(
    memory: &impl Memory,
    pointer: usize,
    expected_vtable: Option<usize>,
    expected_id: u64,
    row: &mut Organisation,
) -> Result<(), OrganisationReadError> {
    let mut author = [0_u8; 8];
    let mut marker = [0_u8; 4];
    let mut header = [0_u8; 16];
    let mut live_header = [0_u8; 16];
    let copied = memory.copy_scattered(
        [
            at(pointer, INDUSTRIAL_ORGANISATION).map_err(OrganisationReadError::Fault)?,
            at(pointer, ORGANISATION_TRAIT_COUNT).map_err(OrganisationReadError::Fault)?,
            at(pointer, ORGANISATION_BONUSES).map_err(OrganisationReadError::Fault)?,
            pointer,
        ],
        &mut [&mut author, &mut marker, &mut header, &mut live_header],
    );
    if copied != 4 {
        verify_live_header(
            expected_vtable,
            read::<[u8; 16]>(memory, pointer),
            expected_id,
            "variant identity changed during organisation read",
        )
        .map_err(OrganisationReadError::Identity)?;
        return Err(OrganisationReadError::Fault("design MIO fields unreadable"));
    }
    verify_live_header(
        expected_vtable,
        Some(live_header),
        expected_id,
        "variant identity changed during organisation read",
    )
    .map_err(OrganisationReadError::Identity)?;
    let Some(array) = Array::from_header(header, 16) else {
        verify_live_header(
            expected_vtable,
            read::<[u8; 16]>(memory, pointer),
            expected_id,
            "variant identity changed during organisation read",
        )
        .map_err(OrganisationReadError::Identity)?;
        return Err(OrganisationReadError::Fault(
            "design MIO bonus array invalid",
        ));
    };
    crate::allocation::resize_default(&mut row.bonuses, array.count as usize)
        .map_err(|_| OrganisationReadError::Fault("design MIO bonus allocation failed"))?;
    if array.count > 0 {
        for (index, bonus) in row.bonuses.iter_mut().enumerate() {
            let address = array
                .element(index as u32, 16)
                .ok_or(OrganisationReadError::Fault(
                    "design MIO bonus address overflow",
                ))?;
            let Some(key) = read(memory, address) else {
                verify_live_header(
                    expected_vtable,
                    read::<[u8; 16]>(memory, pointer),
                    expected_id,
                    "variant identity changed during organisation read",
                )
                .map_err(OrganisationReadError::Identity)?;
                return Err(OrganisationReadError::Fault(
                    "design MIO bonus key unreadable",
                ));
            };
            let Some(value_raw) = at(address, 8).ok().and_then(|at| read(memory, at)) else {
                verify_live_header(
                    expected_vtable,
                    read::<[u8; 16]>(memory, pointer),
                    expected_id,
                    "variant identity changed during organisation read",
                )
                .map_err(OrganisationReadError::Identity)?;
                return Err(OrganisationReadError::Fault(
                    "design MIO bonus value unreadable",
                ));
            };
            bonus.key = key;
            bonus.value_raw = value_raw;
        }
        verify_live_header(
            expected_vtable,
            read::<[u8; 16]>(memory, pointer),
            expected_id,
            "variant identity changed during organisation read",
        )
        .map_err(OrganisationReadError::Identity)?;
    }
    row.author = u64::from_le_bytes(author);
    row.trait_count = i32::from_le_bytes(marker);
    Ok(())
}

#[cfg(test)]
pub fn walk_stats(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<StatsRow>,
) -> Outcome {
    walk_stats_carried(
        memory,
        base,
        variants,
        output,
        &mut (),
        &mut TestQuietWitness,
    )
}

#[allow(clippy::too_many_arguments, reason = "Variant stat capture keeps preallocated output, organisation carry and live stat witness explicit.")]
pub fn walk_stats_carried(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<StatsRow>,
    carry: &mut impl OrganisationCarry,
    witness: &mut impl StatWitness,
) -> Outcome {
    let result = (|| {
        if variants.len() > output.len() {
            crate::allocation::reserve(output, variants.len() - output.len())
                .map_err(|_| "variant stat allocation failed")?;
            output.resize(variants.len(), StatsRow::default());
        }
        let mut sites = WitnessSites::resolve(base);
        let mut len = 0;
        const BATCH: usize = 64;
        const FIELDS: [RecordField; 15] = [
            RecordField {
                source_offset: AIR_RANGE_STAT,
                destination_offset: 0,
                len: 32,
            },
            RecordField {
                source_offset: MAXIMUM_SPEED_STAT,
                destination_offset: 32,
                len: 8,
            },
            RecordField {
                source_offset: RELIABILITY_STAT,
                destination_offset: 40,
                len: 8,
            },
            RecordField {
                source_offset: WEIGHT_STAT,
                destination_offset: 48,
                len: 24,
            },
            RecordField {
                source_offset: BUILD_COST_IC_STAT,
                destination_offset: 72,
                len: 8,
            },
            RecordField {
                source_offset: UPGRADES_COUNT,
                destination_offset: 80,
                len: 4,
            },
            RecordField {
                source_offset: MODULES_COUNT,
                destination_offset: 84,
                len: 4,
            },
            RecordField {
                source_offset: BONUS_SOURCES + 0x0c,
                destination_offset: 88,
                len: 4,
            },
            RecordField {
                source_offset: NAMED_BONUSES + 0x0c,
                destination_offset: 92,
                len: 4,
            },
            RecordField {
                source_offset: ORGANISATION_BONUSES_COUNT,
                destination_offset: 96,
                len: 4,
            },
            RecordField {
                source_offset: BUILD_COST,
                destination_offset: 100,
                len: 8,
            },
            RecordField {
                source_offset: DEFINITION,
                destination_offset: 108,
                len: 8,
            },
            RecordField {
                source_offset: CLASSIFIER_FLAGS,
                destination_offset: 116,
                len: 8,
            },
            RecordField {
                source_offset: EQUIPMENT_TOKEN,
                destination_offset: 124,
                len: 4,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 128,
                len: 16,
            },
        ];
        const STRIDE: usize = 144;
        const DEF_FIELDS: [RecordField; 8] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: cequipment_type::STATS_BASE + 46 * 8,
                destination_offset: 8,
                len: 32,
            },
            RecordField {
                source_offset: cequipment_type::STATS_BASE + 62 * 8,
                destination_offset: 40,
                len: 8,
            },
            RecordField {
                source_offset: cequipment_type::STATS_BASE + 65 * 8,
                destination_offset: 48,
                len: 8,
            },
            RecordField {
                source_offset: cequipment_type::STATS_BASE + 67 * 8,
                destination_offset: 56,
                len: 24,
            },
            RecordField {
                source_offset: cequipment_type::STATS_BASE + 77 * 8,
                destination_offset: 80,
                len: 8,
            },
            RecordField {
                source_offset: cequipment_type::RELIABILITY_BONUS_COUNT,
                destination_offset: 88,
                len: 4,
            },
            RecordField {
                source_offset: cequipment_type::TOKEN,
                destination_offset: 92,
                len: 4,
            },
        ];
        const DEF_STRIDE: usize = 96;
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, STRIDE).expect("valid record layout");
        const DEF_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DEF_FIELDS, DEF_STRIDE).expect("valid definition stat layout");
        let mut pointers = [0; BATCH];
        let mut fields = [0; BATCH * STRIDE];
        let mut definitions = [0; BATCH];
        let mut def_bytes = [0; BATCH * DEF_STRIDE];
        let mut raw_batch = [[0_i64; STAT_LANE_COUNT]; BATCH];
        let mut lane_readable = [[false; STAT_LANE_COUNT]; BATCH];
        let mut metas: [Option<VariantStatMeta>; BATCH] = [None; BATCH];
        let mut headers: [Option<[u8; 16]>; BATCH] = [None; BATCH];
        for sources in variants.chunks(BATCH) {
            sites.clear_live_operands();
            for (pointer, source) in pointers.iter_mut().zip(sources) {
                *pointer = source.pointer();
            }
            let before = witness.quiet();
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..sources.len()]),
                &LAYOUT,
                &mut fields,
            );
            if copied == sources.len() * FIELDS.len() {
                for (slot, field) in fields.chunks_exact(STRIDE).take(sources.len()).enumerate() {
                    for (lane, bytes) in field[..80].chunks_exact(8).enumerate() {
                        raw_batch[slot][lane] = i64::from_le_bytes(bytes.try_into().unwrap());
                        lane_readable[slot][lane] = true;
                    }
                    let meta = VariantStatMeta::from_record(field);
                    definitions[slot] = meta.definition;
                    metas[slot] = Some(meta);
                    headers[slot] = Some(field[128..144].try_into().unwrap());
                }
            } else {
                for (slot, source) in sources.iter().enumerate() {
                    let pointer = source.pointer();
                    for (lane, stat_lane) in STAT_LANES.into_iter().enumerate() {
                        match pointer
                            .checked_add(stat_lane.offset)
                            .and_then(|addr| read::<i64>(memory, addr))
                        {
                            Some(value) => {
                                raw_batch[slot][lane] = value;
                                lane_readable[slot][lane] = true;
                            }
                            None => {
                                lane_readable[slot][lane] = false;
                            }
                        }
                    }
                    headers[slot] = read::<[u8; 16]>(memory, pointer);
                    metas[slot] = VariantStatMeta::read_individually(memory, pointer);
                    definitions[slot] = metas[slot].map_or(0, |meta| meta.definition);
                }
            }
            let mut def_stats = [DefinitionStats::default(); BATCH];
            if sites.rebuild_located && sites.def_vtable.is_some() && sites.null_def_vtable.is_some()
            {
                let batch_copied = memory.copy_records(
                    RecordSources::Pointers(&definitions[..sources.len()]),
                    &DEF_LAYOUT,
                    &mut def_bytes,
                );
                let ready = batch_copied / DEF_FIELDS.len();
                for index in 0..ready {
                    if let Some(meta) = metas[index]
                        && definitions[index] != 0
                    {
                        let slice = &def_bytes[index * DEF_STRIDE..(index + 1) * DEF_STRIDE];
                        def_stats[index] =
                            DefinitionStats::from_record(slice, meta.equipment_token, &sites);
                    }
                }
                for index in ready..sources.len() {
                    let Some(meta) = metas[index] else {
                        continue;
                    };
                    if definitions[index] == 0 {
                        continue;
                    }
                    let slice = &mut def_bytes[index * DEF_STRIDE..(index + 1) * DEF_STRIDE];
                    if memory.copy_records(
                        RecordSources::Pointers(&definitions[index..index + 1]),
                        &DEF_LAYOUT,
                        slice,
                    ) == DEF_FIELDS.len()
                    {
                        def_stats[index] =
                            DefinitionStats::from_record(slice, meta.equipment_token, &sites);
                    } else {
                        def_stats[index] = DefinitionStats::read_individually(
                            memory,
                            definitions[index],
                            meta.equipment_token,
                            &sites,
                        );
                    }
                }
            }
            let mut identity_error: [Option<&'static str>; BATCH] = [None; BATCH];
            for (slot, source) in sources.iter().enumerate() {
                identity_error[slot] = verify_live_header(
                    sites.variant_vtable,
                    headers[slot],
                    source.id,
                    "variant identity changed before stat read",
                )
                .err();
            }
            let pre_refusal: Option<&'static str> = if !sites.rebuild_located {
                Some("variant stat rebuild site unlocated")
            } else {
                before.err()
            };
            let mut candidate_values = [[None; STAT_LANE_COUNT]; BATCH];
            let mut candidate_refusals = [[None; STAT_LANE_COUNT]; BATCH];
            if pre_refusal.is_none() {
                for (slot, source) in sources.iter().enumerate() {
                    if identity_error[slot].is_some() {
                        continue;
                    }
                    let Some(meta) = metas[slot] else {
                        for lane in 0..STAT_LANE_COUNT {
                            candidate_refusals[slot][lane] = Some(if lane_readable[slot][lane] {
                                "variant stat metadata unreadable"
                            } else {
                                "variant stat lane unreadable"
                            });
                        }
                        continue;
                    };
                    let def = def_stats[slot];
                    if !def.auth_ok {
                        candidate_refusals[slot] =
                            [Some("variant definition unverified for stat cache"); STAT_LANE_COUNT];
                        continue;
                    }
                    let Some(type_rel_bonuses) = def.type_rel_bonuses else {
                        candidate_refusals[slot] =
                            [Some("variant definition unverified for stat cache"); STAT_LANE_COUNT];
                        continue;
                    };
                    let unmodified = meta.upgrades == 0
                        && meta.modules == 0
                        && meta.bonus_sources == 0
                        && meta.named_bonuses == 0
                        && meta.org_bonuses == 0
                        && type_rel_bonuses == 0;
                    let expected = if unmodified {
                        Ok(def
                            .base_stats
                            .map(|value| value.ok_or("variant definition base stat unreadable")))
                    } else {
                        compute_modified_lanes(
                            memory,
                            source.pointer(),
                            meta,
                            &def.base_stats,
                            type_rel_bonuses,
                            &sites,
                        )
                    };
                    match expected {
                        Err(reason) => {
                            candidate_refusals[slot] = [Some(reason); STAT_LANE_COUNT];
                        }
                        Ok(expected) => {
                            let no_design_parts = meta.upgrades == 0
                                && meta.modules == 0
                                && type_rel_bonuses == 0;
                            for lane in 0..STAT_LANE_COUNT {
                                if !lane_readable[slot][lane] {
                                    candidate_refusals[slot][lane] =
                                        Some("variant stat lane unreadable");
                                    continue;
                                }
                                let val = raw_batch[slot][lane];
                                match verify_lane(
                                    memory,
                                    base,
                                    &mut sites,
                                    lane,
                                    val,
                                    &expected,
                                    meta.classifier_flags,
                                    no_design_parts,
                                    meta.resource_build_cost,
                                ) {
                                    Ok(()) => candidate_values[slot][lane] = Some(val),
                                    Err(reason) => candidate_refusals[slot][lane] = Some(reason),
                                }
                            }
                        }
                    }
                }
            }
            let after = witness.quiet();
            let batch_refusal: Option<&'static str> = if let Some(reason) = pre_refusal {
                Some(reason)
            } else {
                match (before, after) {
                    (Err(reason), _) | (_, Err(reason)) => Some(reason),
                    (Ok(b), Ok(a)) if b != a => Some("variant stat writer crossed read"),
                    (Ok(_), Ok(_)) => None,
                }
            };
            for (slot, source) in sources.iter().enumerate() {
                if let Some(reason) = identity_error[slot] {
                    for lane in 0..STAT_LANE_COUNT {
                        witness.record_withheld(lane, reason);
                    }
                    continue;
                }
                let fresh = carry.fresh(len, source)? || output[len].variant != source.id;
                let mut row_refusal: Option<&'static str> = None;
                if fresh {
                    output[len].variant = 0;
                    let org_before = witness.quiet();
                    match read_organisation_verified(
                        memory,
                        source.pointer(),
                        sites.variant_vtable,
                        source.id,
                        &mut output[len].organisation,
                    ) {
                        Ok(()) => {
                            if batch_refusal.is_none() {
                                let org_after = witness.quiet();
                                match (org_before, org_after) {
                                    (Ok(b), Ok(a)) if b == a => {}
                                    (Err(reason), _) | (_, Err(reason)) => {
                                        row_refusal = Some(reason);
                                    }
                                    (Ok(_), Ok(_)) => {
                                        row_refusal = Some("variant stat writer crossed read");
                                    }
                                }
                            }
                        }
                        Err(OrganisationReadError::Identity(reason)) => {
                            row_refusal = Some(reason);
                        }
                        Err(OrganisationReadError::Fault(reason)) => {
                            return Err(reason);
                        }
                    }
                }
                if let Some(reason) = row_refusal {
                    output[len].variant = 0;
                    output[len].organisation.author = 0;
                    output[len].organisation.trait_count = 0;
                    output[len].organisation.bonuses.clear();
                    for lane in 0..STAT_LANE_COUNT {
                        witness.record_withheld(lane, reason);
                    }
                    continue;
                }
                let values = if let Some(reason) = batch_refusal {
                    for lane in 0..STAT_LANE_COUNT {
                        witness.record_withheld(lane, reason);
                    }
                    [None; STAT_LANE_COUNT]
                } else {
                    for (lane, refusal) in candidate_refusals[slot].into_iter().enumerate() {
                        if let Some(reason) = refusal {
                            witness.record_withheld(lane, reason);
                        }
                    }
                    candidate_values[slot]
                };
                output[len].variant = source.id;
                output[len].values = values;
                carry.read(len, source);
                len += 1;
            }
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub(crate) fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("variant address overflow")
}

pub(crate) fn text_failure(reason: &'static str) -> TextOutcome {
    TextOutcome {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    }
}

pub(crate) struct Literal {
    inline: [u8; 16],
    heap: Option<usize>,
    pub len: usize,
}

impl Literal {
    pub fn from_header(header: [u8; 32]) -> Result<Self, &'static str> {
        let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(header[24..].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("equipment string header invalid");
        }
        let len = usize::try_from(len).map_err(|_| "equipment string length overflow")?;
        let inline = header[..16].try_into().unwrap();
        let heap = if capacity == msvc_string::INLINE_CAPACITY {
            if header[len] != 0 {
                return Err("equipment string terminator unreadable or invalid");
            }
            None
        } else {
            let pointer = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("equipment string pointer null or invalid")?;
            at(pointer, len)?;
            Some(pointer)
        };
        Ok(Self { inline, heap, len })
    }

    pub fn copy_growing(
        &self,
        memory: &impl Memory,
        output: &mut Vec<u8>,
        start: usize,
        allocation: &'static str,
    ) -> Result<(), &'static str> {
        let end = start.checked_add(self.len).ok_or(allocation)?;
        if self.len <= 256 {
            let mut bytes = [0; 256];
            self.copy(memory, &mut bytes[..self.len])?;
            if end > output.len() {
                crate::allocation::reserve(output, end - output.len()).map_err(|_| allocation)?;
                output.resize(end, 0);
            }
            output[start..end].copy_from_slice(&bytes[..self.len]);
            return Ok(());
        }
        let pointer = self.heap.expect("large equipment string is heap allocated");
        msvc_string::copy_growing(
            memory,
            pointer,
            self.len,
            output,
            start,
            "equipment string bytes or terminator unreadable",
            allocation,
        )?;
        match read::<u8>(memory, at(pointer, self.len)?) {
            Some(0) => Ok(()),
            Some(_) => Err("equipment string terminator unreadable or invalid"),
            None => Err("equipment string bytes or terminator unreadable"),
        }
    }

    pub fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        if output.len() != self.len {
            return Err("equipment string destination length mismatch");
        }
        if let Some(pointer) = self.heap {
            let mut terminator = [0; 1];
            if memory.copy_scattered(
                [pointer, at(pointer, self.len)?],
                &mut [output, &mut terminator],
            ) != 2
            {
                return Err("equipment string bytes or terminator unreadable");
            }
            if terminator[0] != 0 {
                return Err("equipment string terminator unreadable or invalid");
            }
        } else {
            output.copy_from_slice(&self.inline[..self.len]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
