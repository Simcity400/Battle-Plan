use super::{Content, DecodedFrame, Vision, bind_hover, loc};
use crate::binding::Bindings;
use content_reader_parser::fixed5::{fixed5_percent, fixed5_to_string};
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{AirPowerMenu, tables};
use std::{cmp::Ordering, collections::BTreeMap};

fn fraction(friendly: i64, total: i64) -> i64 {
    if viewer_unit_reads::air_range::finite(friendly) {
        (friendly.wrapping_mul(100_000) / total).clamp(0, 100_000)
    } else {
        100_000
    }
}

fn colour(ours: i64, other: i64) -> char {
    match ours.cmp(&other) {
        Ordering::Less => 'R',
        Ordering::Equal => 'Y',
        Ordering::Greater => 'G',
    }
}

fn breakdown(
    content: &Content,
    menu: &AirPowerMenu,
    side: usize,
    links: &BTreeMap<u32, u32>,
    marked: &dyn Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    let mut lines = Vec::new();
    for contribution in &menu.contributions[side] {
        if contribution.amount == 0 {
            continue;
        }
        let country = links
            .get(&contribution.country)
            .copied()
            .ok_or("air power country contribution is unresolved")?;
        lines.push(format!(
            "  {} = \u{11}{}{}\u{11}!",
            marked(country)?,
            if side == 0 { 'G' } else { 'R' },
            fixed5_to_string(contribution.amount, 1)
        ));
    }
    for (kind, offset) in [("AA", side), ("SAM_MISSION", side + 2)] {
        let key = format!(
            "AIR_SUPERIORITY_{kind}_REDUCTION_{}",
            if side == 0 { "ENEMY" } else { "OUR" }
        );
        lines.push(format!(
            "  {}",
            loc(
                content,
                &key,
                &[(
                    "RED",
                    Argument::Fixed5(menu.reductions[offset].wrapping_neg())
                )]
            )?
        ));
    }
    Ok(lines.join("\n"))
}

fn disabled(
    content: &Content,
    frame: &DecodedFrame,
    ours: u32,
    region: u32,
    links: &BTreeMap<u32, u32>,
) -> Result<String, String> {
    let wings: BTreeMap<_, _> = frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut present = false;
    for registration in frame
        .complete::<tables::AirRegionalMissions>()
        .map_err(|e| e.to_string())?
    {
        let wing = wings
            .get(&registration.wing)
            .ok_or("air superiority registered wing is missing")?;
        if wing.mission_region == Some(region) && links.get(&wing.owner) == Some(&ours) {
            present = true;
            break;
        }
    }
    loc(
        content,
        if present {
            "AIR_SUPERIORITY_BAR_DISABLED"
        } else {
            "AIR_SUPERIORITY_BAR_DISABLED_NO_WINGS"
        },
        &[],
    )
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    out: &mut Bindings,
    marked: &dyn Fn(u32) -> Result<String, String>,
) {
    let scores = (|| {
        let links: BTreeMap<_, _> = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let owner = links
            .iter()
            .find_map(|(&raw, &slot)| (slot == vision.ours()).then_some(raw))
            .ok_or("air superiority viewing country is unresolved")?;
        let scores = frame
            .complete::<tables::RaidAirScores>()
            .map_err(|e| e.to_string())?;
        let score = scores
            .iter()
            .find(|row| row.region == region && row.country == owner);
        Ok::<_, String>((links, score))
    })();
    let (links, score) = match scores {
        Ok(scores) => scores,
        Err(why) => {
            out.withhold(&["progress", "progress_disabled"], why);
            return;
        }
    };
    let (friendly, enemy) = score.map_or((0, 0), |score| (score.friendly, score.enemy));
    let total = friendly.wrapping_add(enemy);
    out.values
        .entry("progress_disabled".into())
        .or_default()
        .hidden = total > 0;
    let bar = out.values.entry("progress".into()).or_default();
    bar.hidden = total < 1;
    if total < 1 {
        bind_hover(
            out,
            "progress_disabled",
            disabled(content, frame, vision.ours(), region, &links),
            Ok(String::new()),
        );
        return;
    }
    let share = fraction(friendly, total);
    bar.progress = Some((share.wrapping_mul(100).wrapping_add(50_000) / 100_000) as i32);
    let hover = (|| {
        let menu = match score.map(|score| &score.menu) {
            Some(replay_data::rows::Recorded::Value(Some(menu))) => menu,
            Some(replay_data::rows::Recorded::NotRecorded) => {
                return Err(String::from("air superiority breakdown was not recorded"));
            }
            _ => return Err(String::from("air superiority breakdown was unreadable")),
        };
        let percent = format!(
            "\u{11}{}{}\u{11}!",
            if share < 33_000 {
                'R'
            } else if share < 66_000 {
                'Y'
            } else {
                'G'
            },
            fixed5_percent(share, 1, true)
        );
        let our = format!(
            "\u{11}{}{}\u{11}!",
            colour(friendly, enemy),
            fixed5_to_string(friendly, 1)
        );
        let their = format!(
            "\u{11}{}{}\u{11}!",
            colour(enemy, friendly),
            fixed5_to_string(enemy, 1)
        );
        let mut text = loc(
            content,
            "AIR_SUPERIORITY_BAR_TOOLTIP",
            &[("PERC", Argument::Text(&percent))],
        )?;
        text.push_str(&loc(
            content,
            "AIR_SUPERIORITY_BAR_OUR",
            &[("OUR", Argument::Text(&our))],
        )?);
        text.push('\n');
        text.push_str(&breakdown(content, menu, 0, &links, marked)?);
        text.push_str(&loc(
            content,
            "AIR_SUPERIORITY_BAR_ENEMY",
            &[("ENEMY", Argument::Text(&their))],
        )?);
        text.push('\n');
        text.push_str(&breakdown(content, menu, 1, &links, marked)?);
        let delayed = format!(
            "{}\n{}",
            loc(content, "AIR_SUPERIORITY_BAR_EXPLANATION", &[])?,
            loc(
                content,
                "AIR_SUPERIORITY_BAR",
                &[("WE_COVERAGE", Argument::Fixed5(menu.coverage))]
            )?
        );
        Ok::<_, String>((text, delayed))
    })();
    match hover {
        Ok((text, delayed)) => bind_hover(out, "progress", Ok(text), Ok(delayed)),
        Err(why) => bind_hover(out, "progress", Err(why), Ok(String::new())),
    }
}
