
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

pub trait StatWitness {
    fn quiet(&self) -> Result<Option<(u64, u64)>, &'static str> {
        Ok(None)
    }
    fn record_withheld(&mut self, _lane: usize, _reason: &'static str) {}
}

impl StatWitness for () {}

pub trait OrganisationCarry {
    fn fresh(&mut self, index: usize, source: &RegistryRow) -> Result<bool, &'static str>;
    fn read(&mut self, _index: usize, _source: &RegistryRow) {}
}
impl OrganisationCarry for () {
    fn fresh(&mut self, _: usize, _: &RegistryRow) -> Result<bool, &'static str> {
        Ok(true)
    }
}
fn read_organisation(
    memory: &impl Memory,
    pointer: usize,
    row: &mut Organisation,
) -> Result<(), &'static str> {
    let mut author = [0_u8; 8];
    let mut marker = [0_u8; 4];
    let mut header = [0_u8; 16];
    if memory.copy_scattered(
        [
            at(pointer, INDUSTRIAL_ORGANISATION)?,
            at(pointer, ORGANISATION_TRAIT_COUNT)?,
            at(pointer, ORGANISATION_BONUSES)?,
        ],
        &mut [&mut author, &mut marker, &mut header],
    ) != 3
    {
        return Err("design MIO fields unreadable");
    }
    row.author = u64::from_le_bytes(author);
    row.trait_count = i32::from_le_bytes(marker);
    let array = Array::from_header(header, 16).ok_or("design MIO bonus array invalid")?;
    crate::allocation::resize_default(&mut row.bonuses, array.count as usize)
        .map_err(|_| "design MIO bonus allocation failed")?;
    for (index, bonus) in row.bonuses.iter_mut().enumerate() {
        let address = array
            .element(index as u32, 16)
            .ok_or("design MIO bonus address overflow")?;
        bonus.key = read(memory, address).ok_or("design MIO bonus key unreadable")?;
        bonus.value_raw =
            read(memory, at(address, 8)?).ok_or("design MIO bonus value unreadable")?;
    }
    Ok(())
}

#[cfg(test)]
pub fn walk_stats(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<StatsRow>,
) -> Outcome {
    walk_stats_carried(memory, base, variants, output, &mut (), &mut ())
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
        let rebuild_located = engine_sites::at(base, mutations::REBUILD_STATS_RVA).is_some();
        let def_vtable = engine_sites::at(base, cequipment_type::VTABLE_RVA);
        let null_def_vtable = engine_sites::at(base, cequipment_type::NULL_VTABLE_RVA);
        let mut thrust_factor: Option<Option<i64>> = None;
        let mut len = 0;
        const BATCH: usize = 64;
        const FIELDS: [RecordField; 13] = [
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
        ];
        const STRIDE: usize = 124;
        const DEF_FIELDS: [RecordField; 7] = [
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
        ];
        const DEF_STRIDE: usize = 92;
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, STRIDE).expect("valid record layout");
        const DEF_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DEF_FIELDS, DEF_STRIDE).expect("valid definition stat layout");
        let mut pointers = [0; BATCH];
        let mut fields = [0; BATCH * STRIDE];
        let mut definitions = [0; BATCH];
        let mut def_bytes = [0; BATCH * DEF_STRIDE];
        for sources in variants.chunks(BATCH) {
            for (pointer, source) in pointers.iter_mut().zip(sources) {
                *pointer = source.pointer();
            }
            let before = witness.quiet();
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..sources.len()]),
                &LAYOUT,
                &mut fields,
            ) != sources.len() * FIELDS.len()
            {
                return Err("variant stat cache unreadable");
            }
            for (pointer, field) in definitions
                .iter_mut()
                .zip(fields.chunks_exact(STRIDE))
                .take(sources.len())
            {
                *pointer = usize::try_from(u64::from_le_bytes(field[108..116].try_into().unwrap()))
                    .unwrap_or(0);
            }
            let mut def_valid = [false; BATCH];
            if rebuild_located && def_vtable.is_some() && null_def_vtable.is_some() {
                let batch_copied = memory.copy_records(
                    RecordSources::Pointers(&definitions[..sources.len()]),
                    &DEF_LAYOUT,
                    &mut def_bytes,
                );
                let ready = batch_copied / DEF_FIELDS.len();
                for index in 0..ready {
                    def_valid[index] = definitions[index] != 0;
                }
                for index in ready..sources.len() {
                    if definitions[index] == 0 {
                        continue;
                    }
                    let slice = &mut def_bytes[index * DEF_STRIDE..(index + 1) * DEF_STRIDE];
                    def_valid[index] = memory.copy_records(
                        RecordSources::Pointers(&definitions[index..index + 1]),
                        &DEF_LAYOUT,
                        slice,
                    ) == DEF_FIELDS.len();
                }
            }
            let after = witness.quiet();
            let batch_refusal: Option<&'static str> = if !rebuild_located {
                Some("variant stat rebuild site unlocated")
            } else {
                match (before, after) {
                    (Err(reason), _) | (_, Err(reason)) => Some(reason),
                    (Ok(b), Ok(a)) if b != a => Some("variant stat writer crossed read"),
                    (Ok(_), Ok(_)) => None,
                }
            };
            for (slot, (source, field)) in
                sources.iter().zip(fields.chunks_exact(STRIDE)).enumerate()
            {
                let mut raw = [0_i64; STAT_LANE_COUNT];
                for (value, bytes) in raw.iter_mut().zip(field[..80].chunks_exact(8)) {
                    *value = i64::from_le_bytes(bytes.try_into().unwrap());
                }
                let mut values = [None; STAT_LANE_COUNT];
                if let Some(reason) = batch_refusal {
                    for lane in 0..STAT_LANE_COUNT {
                        witness.record_withheld(lane, reason);
                    }
                } else {
                    let def_slice = &def_bytes[slot * DEF_STRIDE..(slot + 1) * DEF_STRIDE];
                    let def_vtable_ok = def_valid[slot]
                        && def_vtable.zip(null_def_vtable).is_some_and(|(expected, null)| {
                            let vtable = u64::from_le_bytes(def_slice[..8].try_into().unwrap());
                            vtable == expected as u64 || vtable == null as u64
                        });
                    if !def_vtable_ok {
                        for lane in 0..STAT_LANE_COUNT {
                            witness.record_withheld(lane, "variant definition unverified for stat cache");
                        }
                    } else {
                        let upgrades = u32::from_le_bytes(field[80..84].try_into().unwrap());
                        let modules = u32::from_le_bytes(field[84..88].try_into().unwrap());
                        let bonus_sources = u32::from_le_bytes(field[88..92].try_into().unwrap());
                        let named_bonuses = u32::from_le_bytes(field[92..96].try_into().unwrap());
                        let org_bonuses = u32::from_le_bytes(field[96..100].try_into().unwrap());
                        let build_cost_3d0 =
                            i64::from_le_bytes(field[100..108].try_into().unwrap());
                        let classifier_flags =
                            u64::from_le_bytes(field[116..124].try_into().unwrap());
                        let is_plane = (classifier_flags & cequipment_type::PLANE_MASK) != 0;
                        let type_rel_bonuses =
                            i32::from_le_bytes(def_slice[88..92].try_into().unwrap());
                        let mut base_stats = [0_i64; STAT_LANE_COUNT];
                        for (dst, bytes) in base_stats.iter_mut().zip(def_slice[8..88].chunks_exact(8))
                        {
                            *dst = i64::from_le_bytes(bytes.try_into().unwrap());
                        }
                        let unmodified = upgrades == 0
                            && modules == 0
                            && bonus_sources == 0
                            && named_bonuses == 0
                            && org_bonuses == 0
                            && type_rel_bonuses == 0;
                        let no_design_parts =
                            upgrades == 0 && modules == 0 && type_rel_bonuses == 0;
                        for lane in 0..STAT_LANE_COUNT {
                            let val = raw[lane];
                            let refusal: Option<&'static str> = match lane {
                                1 if is_plane && val < STAT_FIXED_ONE => {
                                    Some("variant stat air_defence below plane minimum")
                                }
                                3 if is_plane && val < STAT_FIXED_ONE => {
                                    Some("variant stat air_agility below plane minimum")
                                }
                                4 if val < 0 => Some("variant stat maximum_speed is negative"),
                                5 if val < 0 => Some("variant stat reliability is negative"),
                                _ => None,
                            };
                            if let Some(reason) = refusal {
                                witness.record_withheld(lane, reason);
                                continue;
                            }
                            if lane == 9 && no_design_parts && val != build_cost_3d0 {
                                witness.record_withheld(
                                    lane,
                                    "variant stat build_cost_ic disagrees with resource build cost",
                                );
                                continue;
                            }
                            if unmodified {
                                let base_val = base_stats[lane];
                                let matches = match lane {
                                    1 => val == if is_plane { base_val.max(STAT_FIXED_ONE) } else { base_val },
                                    3 => {
                                        let no_thrust = if is_plane {
                                            base_val.max(STAT_FIXED_ONE)
                                        } else {
                                            base_val
                                        };
                                        if val == no_thrust {
                                            true
                                        } else if is_plane && raw[6] < raw[7] {
                                            let factor = *thrust_factor.get_or_insert_with(|| {
                                                engine_sites::at(base, THRUST_WEIGHT_AGILITY_FACTOR_RVA)
                                                    .and_then(|at| read::<i64>(memory, at))
                                            });
                                            factor.and_then(|factor| {
                                                raw[7]
                                                    .checked_sub(raw[6])?
                                                    .checked_mul(factor)?
                                                    .checked_div(STAT_FIXED_ONE)?
                                                    .checked_add(base_val)
                                                    .map(|v| v.max(STAT_FIXED_ONE))
                                            }) == Some(val)
                                        } else {
                                            false
                                        }
                                    }
                                    4 => val == base_val.max(0),
                                    5 => {
                                        val == base_val.clamp(0, STAT_FIXED_ONE)
                                            || val == base_val.max(0)
                                    }
                                    _ => val == base_val,
                                };
                                if !matches {
                                    witness.record_withheld(lane, STAT_LANES[lane].mismatch_reason);
                                    continue;
                                }
                            }
                            values[lane] = Some(val);
                        }
                    }
                }
                if carry.fresh(len, source)? {
                    read_organisation(memory, source.pointer(), &mut output[len].organisation)?;
                }
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
