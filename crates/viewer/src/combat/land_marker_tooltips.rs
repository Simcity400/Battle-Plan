//! CLandCombatMapIcon::BuildTooltip@1418AC1D0, normal (non-debug) hover.
use super::{
    binding::{TextIcons, builtin},
    engaged::Engaged,
    land_details,
};
use crate::{
    document::Document,
    engaged_index::{self, EngagedIndex},
    map::{raids::tooltips::Tooltip, visibility::Relations},
    table_access::rows,
};
use installed_content::{
    country_names::CountryNames,
    localisation::{Argument, Localisation},
};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatRow, LandCombatSideRow, tables},
};
use std::{collections::BTreeMap, sync::Arc};

// Executable literals at 1418AC1D0 and 1413CCBC0.
super::text::tooltip_keys! {
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

/// 1413CCBC0: strict wrapped fixed5 comparisons. In an overflow where both
/// tests hold, attacker membership wins; otherwise either membership makes
/// the text relative to the viewer. Neutral viewers get the absolute side.
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
    // Deliberately select the defender only when attacker-winning, rather
    // than min(): signed/overflow branches preserve the native selection.
    let selected = if aw { defender } else { attacker };
    let fixed = if (selected as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        selected.wrapping_mul(100_000) / 2_400_000
    } else {
        i64::from(u32::MAX)
    };
    (key, super::fixed::round(fixed)) // 1424D8760, then low int32.
}

/// 141A1D590 consumes the primary list in order, doing no sorting or
/// deduplication here: the recorded list is the native country's set.
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
    // Empty UI selection makes 140C72F60 empty; 1418AC1D0 still adds LF.
    // The viewer has no selected native units or native debug-tooltip flag.
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
        text.push('\n'); // 1418AC1D0 appends a literal LF after each side.
    }
    text.push_str(&outlook(icons, loc, scores, member)?);
    Ok(text)
}

/// The common outcome/duration tail of 1413CCBC0.
pub(super) fn outlook(
    icons: &dyn TextIcons,
    loc: &Localisation,
    scores: [i64; 2],
    member: [bool; 2],
) -> Result<String, String> {
    let (key, days) = outcome(scores, member);
    let mut text = builtin(icons, loc, key, &[])?;
    super::text::ensure_newlines(&mut text, 1);
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

pub(crate) fn hit(hits: &[super::Hit], point: [f32; 2]) -> Option<(u64, [f32; 4])> {
    let hit = hits.iter().rev().find(|hit| hit.contains(point))?;
    match hit.selection {
        super::Selection::Land(id) => Some((id, hit.rect)),
        _ => None,
    }
}

// CountryTagListContainsEquivalent@1406D7F90 -> 140BA60A0: primary or
// secondary membership, direct references resolving to the same country.
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

// 1413CD880: either cached side engaged; unknown is not no-intel.
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

/// The branch, hour, viewing country and combat a tooltip was built for, and
/// whether the engaged index covered that hour, and the viewer's NATO preference.
type CacheKey = (replay::BranchId, i32, u32, u64, bool, bool);

#[derive(Default)]
pub(crate) struct Cache {
    held: Option<(CacheKey, Arc<Tooltip>)>,
}
impl Cache {
    pub(crate) fn request(
        &mut self,
        document: &Document,
        frame: &DecodedFrame,
        viewer: u32,
        id: u64,
        index: Option<&EngagedIndex>,
        nato: bool,
    ) -> Arc<Tooltip> {
        let key = (
            frame.branch,
            frame.hour,
            viewer,
            id,
            index.is_some_and(|index| index.covers(frame.branch, frame.hour)),
            nato,
        );
        if let Some((held, text)) = &self.held
            && *held == key
        {
            return Arc::clone(text);
        }
        let mut text = Tooltip::default();
        let result: Result<String, String> = (|| {
            let links: BTreeMap<_, _> = crate::map::tables::core::<tables::CountryLinks>(frame)
                .map_err(|e| e.to_string())?
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect();
            let relations = Relations::read(frame, &links);
            let (flags, provenance) = engaged_index::combats(
                frame,
                engaged_index::LAND,
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
                &document.icons.combat,
                &document.names.localisation,
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
            let conditions = land_details::conditions(frame).map_err(|e| e.to_string())?;
            let scores = land_details::side_scores(
                detail,
                divisions.rows(),
                &conditions,
                [&combat.attackers, &combat.defenders],
                document.counters.defines.land_combat.scores,
            )
            .map_err(|e| format!("survival estimate: {e:?}"))?;
            let names = CountryNames::read(frame, &document.names.localisation);
            let result = seen(
                &document.icons.combat,
                &document.names.localisation,
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
                        document, frame, viewer, &links, border, nato, &mut text,
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
            // BuildTooltip@1418AC1D0 writes only its third (delayed) output.
            Ok(body) => text.body = body,
            Err(why) => {
                text.refused.insert(format!("land marker {id}: {why}"), 1);
            }
        }
        let text = Arc::new(text);
        self.held = Some((key, Arc::clone(&text)));
        text
    }
}

#[cfg(test)]
mod tests;
