//! Test-only verbatim copy of origin/main's land combat window reader:
//! `read_window_operands`, `template_frames` and the commander selection
//! (`Reader::commander_source`, here a free function over the reader). The
//! cost cuts' tests compare the shared window with it read for read.

use super::*;

pub(super) fn read_window_operands(
    memory: &impl Memory,
    base: usize,
    state: usize,
    source: Source,
    out: &mut [WindowDivision],
) -> Result<WindowOperands, &'static str> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let border = reader
        .class(source.pointer)?
        .ok_or("land window operands source is naval")?;
    let header = reader.header(source.pointer, border)?;
    if header != (source.id, source.attackers, source.defenders) {
        return Err("land window operands source changed");
    }
    let combat_province = reader.province_id(reader.pointer(source.pointer, PROVINCE)?)?;
    let dlc_mask = base
        .checked_add(crate::dlc_feature_state::MASK_RVA)
        .and_then(|address| read::<u32>(memory, address))
        .ok_or("land window DLC mask unreadable")?;
    let thunder = dlc_mask & crate::dlc_feature_state::THUNDER_AT_OUR_GATES != 0;
    let countries = crate::ccountry::Reader::new(memory, base, state)?;
    let group_base = base
        .checked_add(COUNTRY_GROUP_BASE_RVA)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|pointer| *pointer != 0);
    // Committed pointers and handles for both sides; the opposing side's feed
    // the frame-14 scan and the own side's feed HQ membership.
    let mut committed: [[usize; DETAIL_UNIT_LIMIT as usize]; 2] =
        [[0; DETAIL_UNIT_LIMIT as usize]; 2];
    let mut committed_handles: [[u64; DETAIL_UNIT_LIMIT as usize]; 2] =
        [[0; DETAIL_UNIT_LIMIT as usize]; 2];
    let mut committed_len = [0_usize; 2];
    for (side, units) in [source.attackers, source.defenders].into_iter().enumerate() {
        if units.count > DETAIL_UNIT_LIMIT {
            return Err("land window units exceed safety bound");
        }
        let side_pointer = reader.pointer(source.pointer, [ATTACKER, DEFENDER][side])?;
        let vector = reader.array(side_pointer, cland_combatant::COMMITTED, 8)?;
        if vector.count > DETAIL_UNIT_LIMIT {
            return Err("land window committed exceed safety bound");
        }
        for index in 0..vector.count {
            let unit = reader.element(vector, index)?;
            committed[side][index as usize] = unit;
            committed_handles[side][index as usize] = reader.unit(unit)?;
        }
        committed_len[side] = vector.count as usize;
        let reserves = reader.array(side_pointer, cland_combatant::RESERVES, 8)?;
        if reserves.count > DETAIL_UNIT_LIMIT {
            return Err("land window reserves exceed safety bound");
        }
    }
    let mut result = WindowOperands::default();
    for (side, units) in [source.attackers, source.defenders].into_iter().enumerate() {
        let opposing = 1 - side;
        let commander = commander_source(&reader, units)?;
        result.hq_unit[side] = hq_unit(
            &reader,
            thunder,
            &commander,
            &committed_handles[side][..committed_len[side]],
        )?;
        // Every full participant resolves direction operands in native order,
        // including reserves and unassigned units; only committed rows
        // evaluate template frames.
        for index in 0..units.count {
            let unit = reader.element(units, index)?;
            let handle = reader.unit(unit)?;
            let committed_member = committed[side][..committed_len[side]].contains(&unit);
            let mut row = WindowDivision {
                handle,
                ..WindowDivision::default()
            };
            direction(&reader, unit, combat_province, &mut row)?;
            if committed_member {
                row.template_frames = template_frames(
                    &reader,
                    &countries,
                    group_base,
                    unit,
                    &committed[opposing][..committed_len[opposing]],
                )?;
            }
            let next = result.divisions_len;
            if next >= WINDOW_DIVISION_LIMIT {
                return Err("land window operands exceed safety bound");
            }
            *out.get_mut(next)
                .ok_or("land window operands exceed division capacity")? = row;
            result.divisions_len = next + 1;
        }
    }
    Ok(result)
}

/// 1415CA660 frames 14 and 15 for one committed division. Nulls along the
/// template, country, state and list chains report zero frames exactly as the
/// native's own null checks do; only faulting reads fail the walk.
fn template_frames(
    reader: &Reader<'_, impl Memory>,
    countries: &crate::ccountry::Reader<'_, impl Memory>,
    group_base: Option<usize>,
    unit: usize,
    opposing: &[usize],
) -> Result<u8, &'static str> {
    let template = reader.pointer(unit, crate::carmy::STANDARD_TEMPLATE)?;
    if template == 0 {
        return Ok(0);
    }
    let inner = template
        .checked_add(TEMPLATE_INNER)
        .ok_or("land window template address overflow")?;
    if reader.field::<i32>(inner, TEMPLATE_STATE)? != 2 {
        return Ok(0);
    }
    let country: u32 = if reader.field::<i32>(inner, TEMPLATE_COUNTRY)? >= 1 {
        reader.field(inner, TEMPLATE_COUNTRY)?
    } else {
        reader.field(inner, TEMPLATE_COUNTRY_ALT)?
    };
    let mut frames = 0;
    // Frame 14: any opposing committed owner in our template country's set.
    if let Ok(object) = country_object(countries, country) {
        let set = country_tag_set(reader, object)?;
        for opposing_unit in opposing {
            let key = opposing_owner(reader, *opposing_unit)?;
            if tag_set_contains(reader, group_base, set, key)? {
                frames |= TEMPLATE_FRAME_14;
                break;
            }
        }
    }
    // Frame 15: our template country cores the location state.
    let province = reader.pointer(unit, crate::cunit::LOCATION)?;
    if province != 0 {
        let state = reader.pointer(province, crate::cprovince::STATE)?;
        if state != 0 {
            let cores = state
                .checked_add(crate::cstate::CORES)
                .ok_or("land window core list address overflow")?;
            if let Some(cores) = country_list(reader, cores, CORE_LIST_ERRORS)? {
                let mut index = 0;
                while index < cores.count {
                    let entry = cores
                        .element(index, 4)
                        .and_then(|address| read::<u32>(reader.memory, address))
                        .ok_or("land window core entry unreadable")?;
                    if entry == country
                        || (entry != 0
                            && country != 0
                            && group_equal(group_base, reader, entry, country)?)
                    {
                        frames |= TEMPLATE_FRAME_15;
                        break;
                    }
                    index += 1;
                }
            }
        }
    }
    Ok(frames)
}

fn commander_source<M: Memory>(
    reader: &Reader<'_, M>,
    units: Array,
) -> Result<CommanderSource, &'static str> {
    use crate::{carmy_leader, corders_group, cunit_leader, cunit_leader_skill};
    if units.count > DETAIL_UNIT_LIMIT {
        return Err("land commander units exceed safety bound");
    }
    let mut selected = CommanderSource::default();
    for index in 0..units.count {
        let unit = reader.element(units, index)?;
        reader.unit(unit)?;
        let group = reader.pointer(unit, carmy::ORDERS_GROUP)?;
        if group == 0 {
            continue;
        }
        reader.vtable(group, 0, corders_group::VTABLE_RVA)?;
        let leader = reader.pointer(group, corders_group::LEADER)?;
        if leader == 0 {
            continue;
        }
        reader.vtable(leader, 0, carmy_leader::VTABLE_RVA)?;
        let handle = reader.field::<u64>(leader, cunit_leader::TYPE)?;
        if handle as u32 != cunit_leader::REFERENCE_TYPE || handle >> 32 == 0 {
            return Err("land commander reference invalid");
        }
        let skill = reader.pointer(leader, cunit_leader::SKILL)?;
        reader.vtable(skill, 0, cunit_leader_skill::VTABLE_RVA)?;
        let skill = reader.field(skill, cunit_leader_skill::LEVEL)?;
        let category = reader.field(leader, cunit_leader::CATEGORY)?;
        if selected.pointer == 0
            || skill > selected.skill
            || (skill == selected.skill && selected.category == 0 && category == 1)
        {
            selected = CommanderSource {
                pointer: leader,
                handle,
                skill,
                category,
                ..CommanderSource::default()
            };
        }
    }
    if selected.pointer != 0 && (selected.category as u32) < 2 {
        selected.formation = reader.leader_formation(selected.pointer)?;
        if selected.formation != 0 {
            let parent = reader.pointer(selected.formation, corders_group::PARENT)?;
            if parent != 0 {
                reader.vtable(parent, 0, crate::carmy_group::VTABLE_RVA)?;
                selected.parent = reader.pointer(parent, corders_group::LEADER)?;
                if selected.parent != 0 {
                    reader.vtable(selected.parent, 0, carmy_leader::VTABLE_RVA)?;
                }
            }
        }
    }
    Ok(selected)
}
