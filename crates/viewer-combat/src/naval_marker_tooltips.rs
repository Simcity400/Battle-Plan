use super::content::Content as StaticContent;
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    rows::NavalCombatDetailsRow, rows::NavalCombatShipRow, rows::ShipRow, rows::TableSchema,
    rows::TaskForceCounterInputRow, rows::TaskForceRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};
use viewer_engagements::engaged::Engaged;
use viewer_engagements::engagement_history::{self, History};
use viewer_foundation::tables::core;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::{visibility::Relations, tooltip_text::Tooltip};
use dashboard_text::table_access::{Available, rows};
use {
    super::land_marker_tooltips,
    viewer_foundation::tooltip_text::{TextIcons, builtin},
};

tooltip_keys! {
    NAVAL_COMBAT_TOOLTIP_ATTACKER = "NAVAL_COMBAT_TOOLTIP_ATTACKER",
    NAVAL_COMBAT_TOOLTIP_ATTACKER_WITH_CONVOYS = "NAVAL_COMBAT_TOOLTIP_ATTACKER_WITH_CONVOYS",
    NAVAL_COMBAT_TOOLTIP_DEFENDER = "NAVAL_COMBAT_TOOLTIP_DEFENDER",
    NAVAL_COMBAT_TOOLTIP_DEFENDER_WITH_CONVOYS = "NAVAL_COMBAT_TOOLTIP_DEFENDER_WITH_CONVOYS",
    NAVAL_COMBAT_CLICK_DESC = "NAVAL_COMBAT_CLICK_DESC",
}

#[cfg(test)]
pub(crate) fn hit(hits: &[super::Hit], point: [f32; 2]) -> Option<(u64, [f32; 4])> {
    let hit = hits.iter().rev().find(|hit| hit.contains(point))?;
    match hit.selection {
        super::Selection::Naval(id) => Some((id, hit.rect)),
        _ => None,
    }
}

fn whole<'a, T: TableSchema>(table: &Available<'a, T>, name: &str) -> Result<&'a [T::Row], String> {
    match table {
        Available::Complete(rows) => Ok(rows),
        Available::Truncated { missing, .. } => {
            Err(format!("{name} is truncated ({missing} rows unread)"))
        }
        Available::Unavailable { reason } => Err(format!("{name} is unavailable ({reason})")),
        Available::NotRecorded { mode } => {
            Err(format!("{name} is not recorded in a {mode} recording"))
        }
    }
}

fn contains(ids: &[u32], viewer: u32, links: &BTreeMap<u32, u32>) -> Result<bool, String> {
    if ids.iter().any(|raw| links.get(raw) == Some(&viewer)) {
        return Ok(true);
    }
    if let Some(raw) = ids.iter().find(|raw| !links.contains_key(raw)) {
        return Err(format!("CountryLinks lacks participant {raw}"));
    }
    Ok(false)
}

fn hq_count(
    detail: &NavalCombatDetailsRow,
    members: &[&NavalCombatShipRow],
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    ships: &BTreeMap<u64, &ShipRow>,
    forces: &BTreeMap<u64, &TaskForceRow>,
    inputs: &BTreeMap<u64, &TaskForceCounterInputRow>,
) -> Result<i32, String> {
    let mut side = None;
    for (index, value) in detail.sides.iter().enumerate() {
        if contains(&value.summary.countries, viewer, links)? {
            side = Some(index as u8);
            break;
        }
    }
    let Some(side) = side else {
        return Ok(0);
    };
    let mut seen = BTreeSet::new();
    for member in members
        .iter()
        .filter(|row| row.side == side && row.live_ship != 0)
    {
        let ship = ships
            .get(&member.ship)
            .ok_or_else(|| format!("Ships lacks ship {}", member.ship))?;
        if !seen.insert(ship.task_force) {
            continue;
        }
        let force = forces
            .get(&ship.task_force)
            .ok_or_else(|| format!("TaskForces lacks task force {}", ship.task_force))?;
        let owner = links
            .get(&force.roster_owner)
            .ok_or_else(|| format!("CountryLinks lacks roster owner {}", force.roster_owner))?;
        if *owner != viewer {
            continue;
        }
        let input = inputs
            .get(&force.id)
            .ok_or_else(|| format!("TaskForceCounterInputs lacks task force {}", force.id))?;
        if input.attached_headquarters != 0 {
            return Ok(input.attached_headquarters);
        }
    }
    Ok(0)
}

fn seen(
    icons: &dyn TextIcons,
    loc: &Localisation,
    detail: &NavalCombatDetailsRow,
    members: &[&NavalCombatShipRow],
    hq: i32,
    membership: [bool; 2],
    name: &impl Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = String::new();
    for (side, value) in detail.sides.iter().enumerate() {
        let convoys = members
            .iter()
            .filter(|row| row.side as usize == side && row.live_convoy != 0 && row.state != 0x10)
            .fold(0_i32, |count, _| count.wrapping_add(1));
        let countries = land_marker_tooltips::countries(&value.summary.countries, name)?;
        let key = match (side, convoys != 0) {
            (0, false) => NAVAL_COMBAT_TOOLTIP_ATTACKER,
            (0, true) => NAVAL_COMBAT_TOOLTIP_ATTACKER_WITH_CONVOYS,
            (_, false) => NAVAL_COMBAT_TOOLTIP_DEFENDER,
            (_, true) => NAVAL_COMBAT_TOOLTIP_DEFENDER_WITH_CONVOYS,
        };
        text.push_str(&builtin(
            icons,
            loc,
            key,
            &[
                ("TCOUNT", Argument::Int32(value.units.len() as i32)),
                ("CONVOY_COUNT", Argument::Int32(convoys)),
                ("COUNTRIES", Argument::Text(&countries)),
                ("HQCOUNT", Argument::Int32(hq)),
            ],
        )?);
        text.push('\n');
    }
    text.push_str(&land_marker_tooltips::outlook(
        icons,
        loc,
        [0, 0],
        membership,
    )?);
    ensure_newlines(&mut text, 1);
    text.push_str(&builtin(icons, loc, NAVAL_COMBAT_CLICK_DESC, &[])?);
    Ok(text)
}

pub fn build(
    static_content: &StaticContent,
    frame: &DecodedFrame,
    viewer: u32,
    id: u64,
    index: Option<&History>,
) -> Tooltip {
    let mut text = Tooltip::default();
    let result: Result<String, String> = (|| {
        let links = core::<tables::CountryLinks>(frame).map_err(|e| e.to_string())?;
        let links: BTreeMap<_, _> = links
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let relations = Relations::read(frame, &links);
        let (flags, provenance) = engagement_history::combats(
            frame,
            engagement_history::NAVAL,
            viewer,
            &links,
            relations.as_ref().map_err(|e| *e),
            index,
        )
        .map_err(|e| e.to_string())?;
        if let Some(why) = provenance.limitation() {
            text.refused.insert(format!("naval marker {id}: {why}"), 1);
        }
        if let Some(title) = land_marker_tooltips::intel(
            static_content.assets.as_ref(),
            &static_content.localisation,
            flags.get(&id).copied().unwrap_or(Engaged::Unknown),
        )? {
            return Ok(title);
        }
        let details = rows::<tables::NavalCombatDetails>(frame).map_err(|e| e.to_string())?;
        let detail = details
            .rows()
            .iter()
            .find(|row| row.combat == id)
            .ok_or_else(|| {
                whole(&details, "NavalCombatDetails")
                    .err()
                    .unwrap_or_else(|| format!("NavalCombatDetails lacks combat {id}"))
            })?;
        let member_rows = rows::<tables::NavalCombatShips>(frame).map_err(|e| e.to_string())?;
        let mut members: Vec<_> = whole(&member_rows, "NavalCombatShips")?
            .iter()
            .filter(|row| {
                row.combat == id && row.member_source == 0 && row.group_index != u32::MAX
            })
            .collect();
        members.sort_by_key(|row| (row.side, row.group_index, row.member_index));
        let mut primary_side = None;
        for (side, value) in detail.sides.iter().enumerate() {
            if contains(&value.summary.countries, viewer, &links)? {
                primary_side = Some(side as u8);
                break;
            }
        }
        let hq = if primary_side.is_some_and(|side| {
            members
                .iter()
                .any(|row| row.side == side && row.live_ship != 0)
        }) {
            let ships = rows::<tables::Ships>(frame).map_err(|e| e.to_string())?;
            let forces = rows::<tables::TaskForces>(frame).map_err(|e| e.to_string())?;
            let inputs =
                rows::<tables::TaskForceCounterInputs>(frame).map_err(|e| e.to_string())?;
            let ships = whole(&ships, "Ships")?
                .iter()
                .map(|row| (row.id, row))
                .collect();
            let forces = whole(&forces, "TaskForces")?
                .iter()
                .map(|row| (row.id, row))
                .collect();
            let inputs = whole(&inputs, "TaskForceCounterInputs")?
                .iter()
                .map(|row| (row.task_force, row))
                .collect();
            hq_count(detail, &members, viewer, &links, &ships, &forces, &inputs)?
        } else {
            0
        };
        let membership = detail.sides.each_ref().map(|side| {
            Ok(contains(&side.summary.countries, viewer, &links)?
                || contains(&side.summary.secondary_countries, viewer, &links)?)
        });
        let [a, b]: [Result<bool, String>; 2] = membership;
        let names = CountryNames::read(frame, &static_content.localisation);
        seen(
            static_content.assets.as_ref(),
            &static_content.localisation,
            detail,
            &members,
            hq,
            [a?, b?],
            &|raw| {
                names.name(
                    *links
                        .get(&raw)
                        .ok_or_else(|| format!("CountryLinks lacks participant {raw}"))?,
                )
            },
        )
    })();
    match result {
        Ok(body) => text.body = body,
        Err(why) => {
            text.refused.insert(format!("naval marker {id}: {why}"), 1);
        }
    }
    text
}

#[cfg(test)]
mod tests;
