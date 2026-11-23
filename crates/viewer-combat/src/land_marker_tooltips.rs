use super::content::Content as StaticContent;
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::LandCombatRow, rows::LandCombatSideRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_engagements::engagement_history::{self, History};
use viewer_engagements::land_combat_math::conditions as land_combat_math_conditions;
use viewer_engagements::land_combat_math::side_scores;
use viewer_foundation::tables::core;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_engagements::engaged::Engaged;
use viewer_foundation::tooltip_text::{TextIcons, builtin};
use viewer_foundation::{visibility::Relations, tooltip_text::Tooltip};
use dashboard_text::table_access::rows;

tooltip_keys! {
    LAND_COMBAT_TOOLTIP_NO_INTEL = "LAND_COMBAT_TOOLTIP_NO_INTEL",
    LAND_COMBAT_TOOLTIP_ATTACKER = "LAND_COMBAT_TOOLTIP_ATTACKER",
    LAND_COMBAT_TOOLTIP_DEFENDER = "LAND_COMBAT_TOOLTIP_DEFENDER",
    COMBAT_WE_ARE_WINNING = "COMBAT_WE_ARE_WINNING",
    COMBAT_WE_ARE_LOSING = "COMBAT_WE_ARE_LOSING",
    COMBAT_ATTACKER_WINNING = "COMBAT_ATTACKER_WINNING",
    COMBAT_DEFENDER_WINNING = "COMBAT_DEFENDER_WINNING",
    COMBAT_UNDECIDED = "COMBAT_UNDECIDED",
    COMBAT_LAST_LESS_THAN_DAY = "COMBAT_LAST_LESS_THAN_DAY",
    COMBAT_LAST_MORE_THAN_DAY = "COMBAT_LAST_MORE_THAN_DAY",
}

fn outcome(scores: [i64; 2], member: [bool; 2]) -> (&'static str, i32) {
    let [attacker, defender] = scores;
    let aw = attacker > defender.wrapping_mul(120_000) / 100_000;
    let dw = defender > attacker.wrapping_mul(120_000) / 100_000;
    let key = if member[0] || member[1] {
        if (aw && member[0]) || (dw && !member[0]) {
            COMBAT_WE_ARE_WINNING
        } else if aw || dw {
            COMBAT_WE_ARE_LOSING
        } else {
            COMBAT_UNDECIDED
        }
    } else if aw {
        COMBAT_ATTACKER_WINNING
    } else if dw {
        COMBAT_DEFENDER_WINNING
    } else {
        COMBAT_UNDECIDED
    };
    let selected = if aw { defender } else { attacker };
    let fixed = if (selected as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        selected.wrapping_mul(100_000) / 2_400_000
    } else {
        i64::from(u32::MAX)
    };
    (key, super::fixed::round(fixed))
}

pub(super) fn countries(
    ids: &[u32],
    name: &impl Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    ids.iter()
        .map(|id| Ok(format!("\u{11}Y{}\u{11}!", name(*id)?)))
        .collect::<Result<Vec<_>, String>>()
        .map(|names| names.join(", "))
}

fn seen(
    icons: &dyn TextIcons,
    loc: &Localisation,
    combat: &LandCombatRow,
    sides: &LandCombatSideRow,
    scores: [i64; 2],
    member: [bool; 2],
    name: &impl Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = String::from("\n");
    for (key, roster, ids) in [
        (
            LAND_COMBAT_TOOLTIP_ATTACKER,
            &combat.attackers,
            &sides.attacker_primary,
        ),
        (
            LAND_COMBAT_TOOLTIP_DEFENDER,
            &combat.defenders,
            &sides.defender_primary,
        ),
    ] {
        let names = countries(ids, name)?;
        text.push_str(&builtin(
            icons,
            loc,
            key,
            &[
                ("COUNT", Argument::Int32(roster.len() as i32)),
                ("COUNTRIES", Argument::Text(&names)),
            ],
        )?);
        text.push('\n');
    }
    text.push_str(&outlook(icons, loc, scores, member)?);
    Ok(text)
}

pub(super) fn outlook(
    icons: &dyn TextIcons,
    loc: &Localisation,
    scores: [i64; 2],
    member: [bool; 2],
) -> Result<String, String> {
    let (key, days) = outcome(scores, member);
    let mut text = builtin(icons, loc, key, &[])?;
    ensure_newlines(&mut text, 1);
    text.push_str(&builtin(
        icons,
        loc,
        if days < 2 {
            COMBAT_LAST_LESS_THAN_DAY
        } else {
            COMBAT_LAST_MORE_THAN_DAY
        },
        &[("DAYS", Argument::Int32(days))],
    )?);
    Ok(text)
}

#[cfg(test)]
pub(crate) fn hit(hits: &[super::Hit], point: [f32; 2]) -> Option<(u64, [f32; 4])> {
    let hit = hits.iter().rev().find(|hit| hit.contains(point))?;
    match hit.selection {
        super::Selection::Land(id) => Some((id, hit.rect)),
        _ => None,
    }
}

pub(super) fn membership(
    sides: &LandCombatSideRow,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
) -> Result<[bool; 2], String> {
    let member = |lists: [&[u32]; 2]| -> Result<bool, String> {
        if lists
            .iter()
            .flat_map(|ids| ids.iter())
            .any(|raw| links.get(raw) == Some(&viewer))
        {
            return Ok(true);
        }
        if let Some(raw) = lists
            .iter()
            .flat_map(|ids| ids.iter())
            .find(|raw| !links.contains_key(raw))
        {
            return Err(format!("CountryLinks lacks participant {raw}"));
        }
        Ok(false)
    };
    Ok([
        member([&sides.attacker_primary, &sides.attacker_secondary])?,
        member([&sides.defender_primary, &sides.defender_secondary])?,
    ])
}

pub(super) fn intel(
    icons: &dyn TextIcons,
    loc: &Localisation,
    flag: Engaged,
) -> Result<Option<String>, String> {
    match flag {
        Engaged::Yes => Ok(None),
        Engaged::No => builtin(icons, loc, LAND_COMBAT_TOOLTIP_NO_INTEL, &[]).map(Some),
        flag => Err(format!("engaged flag is {flag:?}")),
    }
}

pub fn build(
    static_content: &StaticContent,
    frame: &DecodedFrame,
    viewer: u32,
    id: u64,
    index: Option<&History>,
    nato: bool,
) -> Tooltip {
    let mut text = Tooltip::default();
    let result: Result<String, String> = (|| {
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let relations = Relations::read(frame, &links);
        let (flags, provenance) = engagement_history::combats(
            frame,
            engagement_history::LAND,
            viewer,
            &links,
            relations.as_ref().map_err(|e| *e),
            index,
        )
        .map_err(|e| e.to_string())?;
        if let Some(why) = provenance.limitation() {
            text.refused.insert(format!("land marker {id}: {why}"), 1);
        }
        if let Some(title) = intel(
            static_content.assets.as_ref(),
            &static_content.localisation,
            flags.get(&id).copied().unwrap_or(Engaged::Unknown),
        )? {
            return Ok(title);
        }
        let combat_rows = rows::<tables::LandCombats>(frame).map_err(|e| e.to_string())?;
        let combat = combat_rows
            .rows()
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("LandCombats lacks combat {id}"))?;
        let side_rows = rows::<tables::LandCombatSides>(frame).map_err(|e| e.to_string())?;
        let sides = side_rows
            .rows()
            .iter()
            .find(|row| row.combat == id)
            .ok_or_else(|| format!("LandCombatSides lacks combat {id}"))?;
        let membership = membership(sides, viewer, &links)?;
        let details = rows::<tables::LandCombatDetails>(frame).map_err(|e| e.to_string())?;
        let detail = details
            .rows()
            .iter()
            .find(|row| row.combat == id)
            .ok_or_else(|| format!("LandCombatDetails lacks combat {id}"))?;
        let divisions =
            rows::<tables::LandCombatDivisions>(frame).map_err(|e| e.to_string())?;
        let members =
            viewer_engagements::land_combat_math::combat_rows(divisions.rows(), id);
        let values = viewer_engagements::land_combat_math::member_values(frame, members)
            .map_err(|e| e.to_string())?;
        let conditions = land_combat_math_conditions(frame).map_err(|e| e.to_string())?;
        let scores = side_scores(
            detail,
            replay_data::rows::land_combat_members(members, &values),
            &conditions,
            [&combat.attackers, &combat.defenders],
            static_content.scores,
        )
        .map_err(|e| format!("survival estimate: {e:?}"))?;
        let names = CountryNames::read(frame, &static_content.localisation);
        let result = seen(
            static_content.assets.as_ref(),
            &static_content.localisation,
            combat,
            sides,
            scores,
            membership,
            &|raw| {
                names.name(
                    *links
                        .get(&raw)
                        .ok_or_else(|| format!("CountryLinks lacks participant {raw}"))?,
                )
            },
        )?;
        text.body = result;
        let border_result = detail
            .border_war
            .require("LandCombatDetails.border_war")
            .and_then(|border| match border {
                Some(border) => super::border_marker::append(
                    static_content,
                    frame,
                    viewer,
                    &links,
                    border,
                    nato,
                    &mut text,
                ),
                None => Ok(()),
            });
        if let Err(why) = border_result {
            *text
                .refused
                .entry(format!("land marker {id} border-conflict tail: {why}"))
                .or_default() += 1;
        }
        Ok(std::mem::take(&mut text.body))
    })();
    match result {
        Ok(body) => text.body = body,
        Err(why) => {
            text.refused.insert(format!("land marker {id}: {why}"), 1);
        }
    }
    text
}

#[cfg(test)]
mod tests;
