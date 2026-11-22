use super::*;
use engine_executable::modifier_keys as keys;
use replay_data::rows::DivisionRow;
use replay_derive::modifier_registry::Registry;
use viewer_foundation::division_panel::{
    at_war,
    country_identity,
    equipment,
    held_bonus,
    modifier,
    state,
    state_amount,
};
pub(super) fn fresh(
    content: &StaticContent,
    frame: &DecodedFrame,
    unit: &DivisionRow,
    status: &DivisionStatusRow,
) -> Result<i64, String> {
    use replay_derive::{eta_derive::frame_terms::FrameTerms, eta_derive::frame_terms::Geography};
    use viewer_foundation::border_power;
    let movement = frame
        .complete::<tables::DivisionMovement>()
        .map_err(|why| why.to_string())?;
    let movement = movement
        .binary_search_by_key(&unit.id, |row| row.division)
        .ok()
        .map(|at| &movement[at])
        .ok_or("fresh attrition movement unavailable")?;
    let province = if movement.path_total as i32 > 0 {
        movement
            .path
            .first()
            .copied()
            .filter(|&id| id != 0)
            .or((unit.province != 0).then_some(unit.province))
            .ok_or("fresh attrition first route province unavailable")?
    } else {
        unit.province
    };
    if province == 0 {
        return Ok(0);
    }
    let content_terms = content
        .attrition
        .get_or_init(|| {
            let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
            let sub_units = content
                .movement
                .as_ref()
                .ok_or("attrition movement content unavailable")?
                .land
                .as_ref()
                .map_err(Clone::clone)?
                .sub_units
                .clone();
            viewer_unit_reads::counter_attrition::PanelContent::read(
                &scripted.layers.layers,
                &content.map,
                sub_units,
            )
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let geography = content
        .movement
        .as_ref()
        .ok_or("attrition terrain content unavailable")?;
    if let Some(row) = frame
        .complete::<tables::DivisionPanelSources>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| row.kind == 6 && row.owner == unit.id)
    {
        return Err(String::from_utf8_lossy(&row.reason).into_owned());
    }
    let registry = Registry::from_frame(frame)?;
    let owner = if (unit.expeditionary_owner as i32) > 0 {
        unit.expeditionary_owner
    } else {
        unit.roster_owner
    };
    let owner_identity = country_identity(frame, owner)?;
    let mut total = status.attrition_threshold;
    let moving = movement.path_total as i32 > 0 && movement.strategic_redeployment == 0;
    if moving {
        let terrain = geography
            .terrain_index(province)
            .ok_or("attrition selected terrain unavailable")?;
        if terrain > 0 {
            let raw = content_terms
                .terms
                .terrains
                .get((terrain - 1) as usize)
                .ok_or("attrition selected terrain outside recorded content")?
                .attrition
                .unwrap_or(0);
            let base =
                total.wrapping_add(registry.effective(registry.indexed(keys::ATTRITION)?, raw)?);
            total = equipment(base, [(1, held_bonus(frame, unit.id, terrain as u32)?)]);
        }
    }
    let provinces = frame
        .complete::<tables::Provinces>()
        .map_err(|why| why.to_string())?;
    let province_row = provinces
        .binary_search_by_key(&province, |row| row.id)
        .ok()
        .map(|at| &provinces[at])
        .ok_or("attrition province unavailable")?;
    let state_id = province_row.state;
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    if state_id != 0 {
        let states = frame
            .complete::<tables::States>()
            .map_err(|why| why.to_string())?;
        let controller = states
            .binary_search_by_key(&state_id, |row| row.id)
            .ok()
            .map(|at| states[at].controller)
            .ok_or("attrition state controller unavailable")?;
        total = total.wrapping_add(state_amount(
            frame,
            state_id,
            unit.roster_owner,
            controller,
        )?);
    }
    total = total.wrapping_add(
        FrameTerms::new(frame)
            .attrition_supply(
                unit,
                movement,
                content_terms.terms.supply_threshold,
                &content_terms.terms,
            )?
            .unwrap_or(0),
    );
    if moving && at_war(frame, &links, unit.logical_owner)? {
        if state_id != 0 {
            total =
                total.wrapping_add(state(frame, state_id, unit.logical_owner, keys::ATTRITION)?);
        }
        let weather = frame
            .complete::<tables::WeatherProvinces>()
            .map_err(|why| why.to_string())?;
        if let Ok(at) = weather.binary_search_by_key(&province, |row| row.id) {
            let modifiers = weather[at]
                .modifiers
                .require("WeatherProvinces.modifiers")?
                .as_ref()
                .ok_or("attrition weather modifiers refused")?;
            let effective =
                |key| registry.effective(registry.indexed(key)?, modifiers.get(key).unwrap_or(0));
            total = total.wrapping_add(effective(keys::ATTRITION)?);
            for (key, factor_key) in [
                (keys::HEAT_ATTRITION, keys::HEAT_ATTRITION_FACTOR),
                (keys::WINTER_ATTRITION, keys::WINTER_ATTRITION_FACTOR),
            ] {
                let raw = effective(key)?;
                if raw == 0 {
                    continue;
                }
                let factor = status
                    .weather_factor
                    .require("DivisionStatus.weather_factor")?
                    .ok_or("attrition acclimatization factor refused")?;
                let thermal = raw.wrapping_mul(factor) / 100_000;
                if thermal <= 0 {
                    continue;
                }
                let selected = status
                    .panel_source()
                    .ok_or("attrition selected commander source unavailable")?;
                if selected[1] == u64::MAX {
                    return Err("attrition selected commander identity unavailable".into());
                }
                let leader = if selected[2] == 0 {
                    0
                } else {
                    modifier(frame, &registry, selected[1] as u8, selected[2], factor_key)?
                };
                let scale = 100_000i64
                    .wrapping_add(modifier(frame, &registry, 0, owner_identity, factor_key)?)
                    .wrapping_add(leader);
                total = total.wrapping_add((thermal.wrapping_mul(scale) / 100_000).max(0));
            }
        }
    }
    if status.exercising != 0 && (movement.path_total as i32) < 1 {
        total = total.wrapping_add(content_terms.terms.training);
    }
    if state_id != 0 {
        let flags = frame
            .complete::<tables::StateFlags>()
            .map_err(|why| why.to_string())?;
        let flag = flags
            .binary_search_by_key(&state_id, |row| row.state)
            .ok()
            .map(|at| &flags[at])
            .ok_or("attrition border-war state flag unavailable")?;
        if *flag
            .border_conflict_active
            .require("StateFlags.border_conflict_active")?
            != 0
        {
            let share = border_power::enemy_share(
                &border_power::Inputs {
                    members: &content_terms.members,
                    neighbors: &content_terms.neighbors,
                    border: &content_terms.border,
                },
                frame,
                unit.logical_owner,
                &links,
                state_id,
            )?;
            total =
                total.wrapping_add(share.wrapping_mul(content_terms.terms.border_factor) / 100_000);
        }
    }
    Ok(total.wrapping_mul(
        100_000i64
            .wrapping_add(modifier(
                frame,
                &registry,
                0,
                owner_identity,
                keys::ATTRITION,
            )?)
            .max(0),
    ) / 100_000)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_variants_contribute_once_including_negative_quantities_and_clamp_the_lower_bound() {
        assert_eq!(
            equipment(
                30_000,
                [(1, 2_000), (900_000, 3_000), (0, 80_000), (-2, 4_000)]
            ),
            39_000
        );
        assert_eq!(equipment(30_000, [(1, -80_000)]), 0);
        assert_eq!(equipment(30_000, [(1, 2_000), (1, 2_000)]), 34_000);
    }
}
