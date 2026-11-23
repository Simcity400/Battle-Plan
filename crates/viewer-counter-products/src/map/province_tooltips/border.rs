use super::localise;
use content_reader_localisation::localisation::{Argument as Arg, Localisation};
#[cfg(test)]
use replay_data::rows::DivisionRow;
use replay_data::rows::{StateFlagRow, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;

pub(super) use viewer_foundation::border_power::Defines;
use viewer_foundation::border_power::{self, enemy_ratio, same};
fn power(
    content: &super::Content,
    frame: &DecodedFrame,
    country: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
    states: &BTreeMap<u32, &replay_data::rows::StateRow>,
    countries: &BTreeMap<u32, &replay_data::rows::CountryRow>,
) -> Result<[i64; 2], String> {
    border_power::power(
        &border_power::Inputs {
            members: &content.members,
            neighbors: &content.neighbors,
            border: &content.border,
        },
        frame,
        country,
        links,
        state,
        states,
        countries,
    )
}
pub(super) fn enemy_share(
    content: &super::Content,
    frame: &DecodedFrame,
    country: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
) -> Result<i64, String> {
    border_power::enemy_share(
        &border_power::Inputs {
            members: &content.members,
            neighbors: &content.neighbors,
            border: &content.border,
        },
        frame,
        country,
        links,
        state,
    )
}
pub(super) fn build(
    content: &super::Content,
    loc: &Localisation,
    frame: &DecodedFrame,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
    flag: &StateFlagRow,
) -> Result<String, String> {
    if *flag
        .border_conflict_active
        .require("StateFlags.border_conflict_active")?
        == 0
    {
        return Ok(String::new());
    }
    let states = frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?;
    let states: BTreeMap<_, _> = states.iter().map(|row| (row.id, row)).collect();
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let countries: BTreeMap<_, _> = countries
        .iter()
        .map(|row| (row.canonical_slot, row))
        .collect();
    let viewer_raw = countries
        .get(&viewer)
        .ok_or("border viewing country absent")?
        .id;
    let hovered = states.get(&state).ok_or("border state absent")?;
    let mut involved = same(links, hovered.controller, viewer_raw)?;
    if !involved {
        'pairs: for pair in flag
            .border_conflict_countries
            .require("StateFlags.border_conflict_countries")?
        {
            for &raw in pair {
                if same(links, raw as u32, viewer_raw)? {
                    involved = true;
                    break 'pairs;
                }
            }
        }
    }
    if !involved {
        return localise(loc, "BORDER_WAR_NOT_OURS", &[]).map(|text| format!("    {text}"));
    }
    let defines = content.border.as_ref().map_err(Clone::clone)?;
    let power = power(
        content, frame, viewer_raw, links, state, &states, &countries,
    )?;
    let share = 100_000_i64.wrapping_sub(enemy_ratio(power));
    let mut text = format!("    {}", localise(loc, "BORDER_WAR_HEADER", &[])?);
    let mut block = String::new();
    for (key, value, blank) in [
        ("BORDER_WAR_POWER", power[0], false),
        ("BORDER_WAR_POWER_ENEMY", power[1], true),
        ("BORDER_WAR_POWER_RATIO", share, false),
        ("BORDER_WAR_WIN", defines.victory, false),
        (
            "BORDER_WAR_LOSS",
            100_000_i64.wrapping_sub(defines.victory),
            true,
        ),
    ] {
        block.push_str(&localise(loc, key, &[("VAL", Arg::Fixed5(value))])?);
        block.push('\n');
        if blank {
            block.push('\n');
        }
    }
    block.push_str(&localise(loc, "BORDER_WAR_HINT", &[])?);
    block.push('\n');
    for (index, part) in block.split('\n').enumerate() {
        if index != 0 {
            text.push('\n');
        }
        if !part.is_empty() {
            text.push_str("        ");
            text.push_str(part);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
