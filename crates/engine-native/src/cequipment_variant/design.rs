use engine_sites::Rva;
use super::*;

pub const PARENT: usize = 0x3f8;
pub const MAX_VERSION: usize = 0x41d;
pub const IS_FRAME: usize = 0x41e;
pub const MANPOWER: usize = 0x420;
pub const AUTO_UPGRADED: usize = 0x425;
pub const HIGHLIGHT: usize = 0x426;
pub const CAN_UPGRADE_TYPE: usize = 0x427;
pub const CAN_UPGRADE_VARIANT: usize = 0x428;
pub const CAN_UPGRADE_MODULES: usize = 0x429;
pub const ROLE_ICON_INDEX: usize = 0x42c;
pub const DIVISION_NAMES_GROUP: usize = 0x480;
pub const DESIGN_TEAM: usize = 0x488;
pub const DESIGN_TEAM_BONUS: usize = 0x498;
pub const PENDING_PARENT_VERSION: usize = 0x42a;
pub const MODULE_PRIORITY_SUM: usize = 0x418;
pub const MODULE_MISSION_EXCLUSIONS: usize = 0x388;
pub const TERRAIN_ATTRITION: usize = 0x3a0;
pub const NICHES: usize = 0x3b8;
pub const BUILD_COST: usize = 0x3d0;
pub const BUILD_RESOURCES: usize = 0x3d8;
pub const DESIGN_TEAM_TRAITS: usize = 0x4b0;
pub const NAMED_BONUSES: usize = 0xe8;
pub const BONUS_SOURCES: usize = 0xd0;
pub const SPECIFIC_BONUS_VTABLE_RVA: Rva = Rva(0x0279_69f0);
pub const TRAIT_BONUS_VTABLE_RVA: Rva = Rva(0x0286_7878);
pub const BONUS_STATS: usize = 0x40;
pub const BONUS_TERRAIN: usize = 0x58;
pub const BONUS_IDEA: usize = 0x78;
pub const NAMED_BONUS_VTABLE_RVA: Rva = Rva(0x029e_cff8);
pub const COUNTRY_NAMED_BONUSES: usize = 0x178;
pub const NAMED_BONUS_STRIDE: usize = 200;
pub const NAMED_BONUS_HANDLE: usize = 0xc0;
pub const NAMED_BONUS_SPECIFIC: usize = 8;
pub const NAMED_BONUS_NAME: usize = 0x80;
pub const NAMED_BONUS_DESCRIPTION: usize = 0xa0;

pub fn bonus_source_size(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<usize, &'static str> {
    let table = word::<u64>(memory, pointer, 0)?;
    if table == site(base, SPECIFIC_BONUS_VTABLE_RVA)? as u64 {
        Ok(BONUS_IDEA)
    } else if table == site(base, TRAIT_BONUS_VTABLE_RVA)? as u64 {
        Ok(BONUS_IDEA + 8)
    } else {
        Err("variant bonus source identity mismatch")
    }
}

pub use game_model::equipment::{VariantStatEntry, VariantModuleMissionExclusion, VariantMissionStats, VariantNamedBonus, VariantBonusSource, VariantDesignState};

fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("variant address overflow")
}

fn word<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("variant design field unreadable")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, stride)
        .ok_or("variant design array unreadable or invalid")
}

fn entries(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<Vec<VariantStatEntry>, &'static str> {
    entries_with_layout(memory, object, offset, false)
}

fn entries_with_layout(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    resource_pool: bool,
) -> Result<Vec<VariantStatEntry>, &'static str> {
    let source = array(memory, object, offset, 16)?;
    let mut result = Vec::new();
    for ordinal in 0..source.count {
        let bytes = read::<[u8; 16]>(
            memory,
            source
                .element(ordinal, 16)
                .ok_or("variant design address overflow")?,
        )
        .ok_or("variant stat entry unreadable")?;
        engine_allocation::reserve(&mut result, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.push(VariantStatEntry {
            index: u32::from_le_bytes(
                if resource_pool {
                    &bytes[8..12]
                } else {
                    &bytes[..4]
                }
                .try_into()
                .unwrap(),
            ),
            value: i64::from_le_bytes(
                if resource_pool {
                    &bytes[..8]
                } else {
                    &bytes[8..]
                }
                .try_into()
                .unwrap(),
            ),
        });
    }
    Ok(result)
}

#[cfg(test)]
pub(super) fn read_design(
    memory: &impl Memory,
    base: usize,
    object: usize,
) -> Result<VariantDesignState, &'static str> {
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&DESIGN_FIELDS, DESIGN_WIDTH).expect("valid design layout");
    let mut bytes = [0; DESIGN_WIDTH];
    if memory.copy_records(RecordSources::Pointers(&[object]), &LAYOUT, &mut bytes)
        != DESIGN_FIELDS.len()
    {
        return Err("variant design fields unreadable");
    }
    read_design_prefetched(
        memory,
        base,
        object,
        &bytes,
        &mut engine_native_parts::creference_registry::ScopedResolver::new(base),
    )
    .map(|(state, _, _)| state)
}

pub(super) const DESIGN_FIELDS: [RecordField; 7] = [
    RecordField {
        source_offset: BONUS_SOURCES,
        destination_offset: 0,
        len: 40,
    },
    RecordField {
        source_offset: MISSION_GROUP_DATA,
        destination_offset: 40,
        len: 128,
    },
    RecordField {
        source_offset: MODULE_PRIORITY_SUM,
        destination_offset: 246,
        len: 4,
    },
    RecordField {
        source_offset: PARENT,
        destination_offset: 168,
        len: 8,
    },
    RecordField {
        source_offset: MAX_VERSION,
        destination_offset: 176,
        len: 7,
    },
    RecordField {
        source_offset: AUTO_UPGRADED,
        destination_offset: 183,
        len: 11,
    },
    RecordField {
        source_offset: DIVISION_NAMES_GROUP,
        destination_offset: 194,
        len: 52,
    },
];
pub(super) const DESIGN_WIDTH: usize = 250;

pub(super) fn read_design_prefetched(
    memory: &impl Memory,
    base: usize,
    object: usize,
    bytes: &[u8],
    references: &mut engine_native_parts::creference_registry::ScopedResolver,
) -> Result<(VariantDesignState, Option<usize>, DesignSourceLinks), &'static str> {
    struct FieldsMemory<'a, M> {
        source: &'a M,
        object: usize,
        bytes: &'a [u8],
    }
    impl<M: Memory> Memory for FieldsMemory<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            for field in DESIGN_FIELDS {
                let Some(offset) = address
                    .checked_sub(self.object)
                    .and_then(|v| v.checked_sub(field.source_offset))
                else {
                    continue;
                };
                if offset <= field.len && output.len() <= field.len - offset {
                    let start = field.destination_offset + offset;
                    output.copy_from_slice(&self.bytes[start..start + output.len()]);
                    return true;
                }
            }
            self.source.copy(address, output)
        }
    }
    let held = FieldsMemory {
        source: memory,
        object,
        bytes,
    };
    read_design_fields(&held, base, object, references)
}

fn read_design_fields(
    memory: &impl Memory,
    base: usize,
    object: usize,
    references: &mut engine_native_parts::creference_registry::ScopedResolver,
) -> Result<(VariantDesignState, Option<usize>, DesignSourceLinks), &'static str> {
    let parent = word::<u64>(memory, object, PARENT)? as usize;
    let group = word::<u64>(memory, object, DIVISION_NAMES_GROUP)? as usize;
    let design_team = word(memory, object, DESIGN_TEAM)?;
    let organisation_pointer;
    let mut result = VariantDesignState {
        parent: if parent == 0 {
            None
        } else {
            Some(identity(memory, base, parent)?)
        },
        max_version: word(memory, object, MAX_VERSION)?,
        is_frame: word(memory, object, IS_FRAME)?,
        manpower: word(memory, object, MANPOWER)?,
        auto_upgraded: word(memory, object, AUTO_UPGRADED)?,
        highlight: word(memory, object, HIGHLIGHT)?,
        can_upgrade_type: word(memory, object, CAN_UPGRADE_TYPE)?,
        can_upgrade_variant: word(memory, object, CAN_UPGRADE_VARIANT)?,
        can_upgrade_modules: word(memory, object, CAN_UPGRADE_MODULES)?,
        role_icon_index: word(memory, object, ROLE_ICON_INDEX)?,
        division_names_group: if group == 0 {
            None
        } else {
            if word::<u64>(memory, group, 0)?
                != site(base, crate::cname_group_member::GROUP_VTABLE_RVA)? as u64
            {
                return Err("variant division naming group identity mismatch");
            }
            let literal =
                Literal::from_header(word(memory, group, crate::cname_group_member::GROUP_KEY)?)?;
            let mut bytes = Vec::new();
            literal.copy_growing(memory, &mut bytes, 0, "variant design allocation failed")?;
            Some(bytes)
        },
        pending_parent_version: word(memory, object, PENDING_PARENT_VERSION)?,
        module_priority_sum: word(memory, object, MODULE_PRIORITY_SUM)?,
        module_mission_exclusions: module_exclusions(memory, base, object)?,
        terrain_attrition: scalar_array(memory, object, TERRAIN_ATTRITION)?,
        niches: scalar_array(memory, object, NICHES)?,
        build_cost: word(memory, object, BUILD_COST)?,
        build_resources: entries_with_layout(memory, object, BUILD_RESOURCES, true)?,
        design_team,
        design_team_serialized: {
            organisation_pointer = references.resolve(memory, design_team)?;
            organisation_pointer.is_some()
        },
        design_team_traits: word(memory, object, DESIGN_TEAM_TRAITS)?,
        design_team_bonus: entries(memory, object, DESIGN_TEAM_BONUS)?,
        ..Default::default()
    };
    let named = array(memory, object, NAMED_BONUSES, 4)?;
    for ordinal in 0..named.count {
        let value = read(
            memory,
            named
                .element(ordinal, 4)
                .ok_or("variant design address overflow")?,
        )
        .ok_or("variant named bonus unreadable")?;
        engine_allocation::reserve(&mut result.named_bonuses, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.named_bonuses.push(value);
    }
    if !result.named_bonuses.is_empty() {
        let cache = word::<[u8; STAT_COUNT * 8]>(memory, object, STATS)?;
        for index in 0..STAT_COUNT {
            if STAT_LANES.iter().any(|lane| lane.stat_index == index) {
                continue;
            }
            let value = i64::from_le_bytes(cache[index * 8..index * 8 + 8].try_into().unwrap());
            engine_allocation::reserve(&mut result.unresolved_stats, 1)
                .map_err(|_| "variant design allocation failed")?;
            result.unresolved_stats.push(VariantStatEntry {
                index: index as u32,
                value,
            });
        }
    }
    let mut source_links = Vec::new();
    let sources = array(memory, object, BONUS_SOURCES, 8)?;
    for ordinal in 0..sources.count {
        let pointer = read::<u64>(
            memory,
            sources
                .element(ordinal, 8)
                .ok_or("variant design address overflow")?,
        )
        .ok_or("variant bonus source unreadable")? as usize;
        let source = read_bonus_source(memory, base, pointer)?;
        engine_allocation::reserve(&mut source_links, 1)
            .map_err(|_| "variant source binding allocation failed")?;
        source_links.push(pointer);
        engine_allocation::reserve(&mut result.bonus_sources, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.bonus_sources.push(source);
    }
    let provider = visit_named_sources(memory, base, object, |pointer, owner, handle| {
        let source = read_named_bonus_source(memory, base, pointer, owner, handle)?;
        engine_allocation::reserve(&mut result.named_sources, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.named_sources.push(source);
        Ok(())
    })?;
    let missions = array(memory, object, MISSION_GROUP_DATA, 40)?;
    for ordinal in 0..missions.count {
        let pointer = missions
            .element(ordinal, 40)
            .ok_or("variant design address overflow")?;
        let mission = VariantMissionStats {
            mask: word(memory, pointer, 0)?,
            stats: entries(memory, pointer, 16)?,
        };
        engine_allocation::reserve(&mut result.mission_stats, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.mission_stats.push(mission);
    }
    Ok((
        result,
        organisation_pointer,
        DesignSourceLinks {
            direct: source_links,
            provider,
        },
    ))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DesignSourceLinks {
    pub direct: Vec<usize>,
    pub provider: Option<(u32, usize)>,
}

pub fn read_bonus_source(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<VariantBonusSource, &'static str> {
    let size = bonus_source_size(memory, base, pointer)?;
    let idea = if size > BONUS_IDEA {
        word::<u64>(memory, pointer, BONUS_IDEA)? as usize
    } else {
        0
    };
    let idea_token = if idea == 0 {
        None
    } else {
        Some(word(memory, idea, 8)?)
    };
    let idea = if idea == 0 {
        None
    } else {
        if word::<u64>(memory, idea, 0)? != site(base, engine_native_parts::cidea::VTABLE_RVA)? as u64 {
            return Err("variant bonus idea identity mismatch");
        }
        let header = word(memory, idea, engine_native_parts::cidea::TOKEN)?;
        let literal = Literal::from_header(header)?;
        let mut bytes = Vec::new();
        literal.copy_growing(memory, &mut bytes, 0, "variant design allocation failed")?;
        Some(bytes)
    };
    let mut source = VariantBonusSource {
        idea,
        idea_token,
        stats: entries(memory, pointer, BONUS_STATS)?,
        ..Default::default()
    };
    let terrain = array(memory, pointer, BONUS_TERRAIN, 8)?;
    for ordinal in 0..terrain.count {
        let value = read(
            memory,
            terrain
                .element(ordinal, 8)
                .ok_or("variant design address overflow")?,
        )
        .ok_or("variant terrain bonus unreadable")?;
        engine_allocation::reserve(&mut source.terrain_attrition, 1)
            .map_err(|_| "variant design allocation failed")?;
        source.terrain_attrition.push(value);
    }
    Ok(source)
}

pub fn read_named_bonus_source(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    owner: u32,
    handle: u32,
) -> Result<VariantBonusSource, &'static str> {
    if word::<u64>(memory, pointer, 0)? != site(base, NAMED_BONUS_VTABLE_RVA)? as u64
        || word::<u64>(memory, pointer, NAMED_BONUS_SPECIFIC)? != site(base, SPECIFIC_BONUS_VTABLE_RVA)? as u64
    {
        return Err("variant named source identity mismatch");
    }
    let text = |offset| {
        let literal = Literal::from_header(word(memory, pointer, offset)?)?;
        let mut bytes = Vec::new();
        literal.copy_growing(memory, &mut bytes, 0, "variant design allocation failed")?;
        Ok::<_, &'static str>(bytes)
    };
    let source = VariantBonusSource {
        named: Some(VariantNamedBonus {
            owner,
            handle,
            name: text(NAMED_BONUS_NAME)?,
            description: text(NAMED_BONUS_DESCRIPTION)?,
        }),
        stats: entries(memory, pointer, NAMED_BONUS_SPECIFIC + BONUS_STATS)?,
        terrain_attrition: scalar_array(memory, pointer, 8 + BONUS_TERRAIN)?,
        ..Default::default()
    };
    Ok(source)
}

pub fn visit_named_sources(
    memory: &impl Memory,
    base: usize,
    object: usize,
    mut visit: impl FnMut(usize, u32, u32) -> Result<(), &'static str>,
) -> Result<Option<(u32, usize)>, &'static str> {
    let handles = array(memory, object, NAMED_BONUSES, 4)?;
    if handles.count == 0 {
        return Ok(None);
    }
    let origin = word::<i32>(memory, object, ORIGIN_LINK)?;
    let owner = if origin > 0 {
        origin as u32
    } else {
        word(memory, object, CREATOR_LINK)?
    };
    let state = word::<u64>(memory, site(base, cgame_state::SINGLETON_RVA)?, 0)? as usize;
    let countries = crate::ccountry::Reader::new(memory, base, state)?;
    let country = countries.resolve(owner)?;
    let status = crate::cproduction_status::manager(memory, base, country)?;
    let sources = array(memory, status, COUNTRY_NAMED_BONUSES, NAMED_BONUS_STRIDE)?;
    for ordinal in 0..handles.count {
        let handle = read::<u32>(
            memory,
            handles
                .element(ordinal, 4)
                .ok_or("variant named handle address overflow")?,
        )
        .ok_or("variant named handle unreadable")?;
        if handle == u32::MAX {
            continue;
        }
        for index in 0..sources.count {
            let pointer = sources
                .element(index, NAMED_BONUS_STRIDE)
                .ok_or("variant named source address overflow")?;
            if word::<u32>(memory, pointer, NAMED_BONUS_HANDLE)? == handle {
                if word::<u64>(memory, pointer, 0)? != site(base, NAMED_BONUS_VTABLE_RVA)? as u64
                    || word::<u64>(memory, pointer, NAMED_BONUS_SPECIFIC)?
                        != site(base, SPECIFIC_BONUS_VTABLE_RVA)? as u64
                {
                    return Err("variant named source identity mismatch");
                }
                visit(pointer, owner, handle)?;
                break;
            }
        }
    }
    Ok(Some((owner, status)))
}

fn scalar_array<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<Vec<T>, &'static str> {
    let source = array(memory, object, offset, std::mem::size_of::<T>())?;
    let mut result = Vec::new();
    for ordinal in 0..source.count {
        let value = read(
            memory,
            source
                .element(ordinal, std::mem::size_of::<T>())
                .ok_or("variant design address overflow")?,
        )
        .ok_or("variant design array entry unreadable")?;
        engine_allocation::reserve(&mut result, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.push(value);
    }
    Ok(result)
}

fn module_exclusions(
    memory: &impl Memory,
    base: usize,
    object: usize,
) -> Result<Vec<VariantModuleMissionExclusion>, &'static str> {
    let source = array(memory, object, MODULE_MISSION_EXCLUSIONS, 16)?;
    let mut result = Vec::new();
    for ordinal in 0..source.count {
        let entry = source
            .element(ordinal, 16)
            .ok_or("variant design address overflow")?;
        let module = word::<u64>(memory, entry, 0)? as usize;
        let header = word::<[u8; 12]>(memory, module, 0)?;
        if u64::from_le_bytes(header[..8].try_into().unwrap())
            != site(base, engine_native_parts::cequipment_module::VTABLE_RVA)? as u64
        {
            return Err("variant excluded module identity mismatch");
        }
        let token = u32::from_le_bytes(header[8..].try_into().unwrap());
        let mask = word(memory, entry, 8)?;
        engine_allocation::reserve(&mut result, 1)
            .map_err(|_| "variant design allocation failed")?;
        result.push(VariantModuleMissionExclusion {
            module_token: token,
            mask,
        });
    }
    Ok(result)
}
