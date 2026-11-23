use content_reader_localisation::localisation::Argument;
use replay_data::{
    rows::LandCombatDetailsRow, rows::LandCombatDivisionRow, rows::land_combat_members,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_engagements::engagement_progress::Progresses;
use viewer_engagements::land_combat_math::{
    Divisions,
    combat_rows,
    conditions,
    member_values,
    modifier,
    side_scores,
};
use {
    super::binding::{Bindings, TEXT, hide},
    viewer_foundation::tooltip_text::builtin,
};
use super::content::Content as StaticContent;
use viewer_foundation::tables::rows;

pub(super) fn division_width(row: &LandCombatDivisionRow) -> Option<i64> {
    if row.context.template == 0 {
        return Some(0);
    }
    let base = row.width_terms.iter().fold(0_i64, |sum, term| {
        sum.wrapping_add(
            i64::from(term.count)
                .wrapping_mul(100_000)
                .wrapping_mul(term.width)
                / 100_000,
        )
    });
    let factor = 100_000_i64
        .wrapping_add(modifier(&row.context, 1, false)?)
        .wrapping_add(modifier(&row.context, 1, true)?);
    Some(base.wrapping_mul(factor) / 100_000)
}

pub(super) fn direction_key(
    row: &LandCombatDivisionRow,
    province: u32,
    movement: &BTreeMap<u64, u32>,
) -> Option<u32> {
    if row.province != province {
        return Some(row.province);
    }
    Some(match *movement.get(&row.division)? {
        0 => row.province,
        source => source,
    })
}

pub(super) fn direction_count<D: std::ops::Deref<Target = LandCombatDivisionRow>>(
    committed: &[u64],
    divisions: &BTreeMap<(u8, u64), D>,
    province: u32,
    movement: &BTreeMap<u64, u32>,
) -> Result<usize, String> {
    let mut seen: Vec<u32> = Vec::new();
    let mut grouped = false;
    for unit in committed {
        let row = divisions.get(&(0, *unit)).ok_or_else(|| {
            format!("attacker committed division {unit} has no LandCombatDivisions row")
        })?;
        if row.direction_group > 0 && grouped {
            continue;
        }
        let key = direction_key(row, province, movement).ok_or_else(|| {
            format!(
                "attacker committed division {unit} stands in the combat province and has no DivisionMovement row for the source its direction takes"
            )
        })?;
        if seen.contains(&key) {
            continue;
        }
        if row.direction_group > 0 {
            grouped = true;
        }
        seen.push(key);
    }
    Ok(seen.len())
}

pub(super) fn total_width(
    detail: &LandCombatDetailsRow,
    [width, support]: [i64; 2],
    directions: usize,
) -> i64 {
    let extra = directions.saturating_sub(1) as i64;
    let width = detail
        .width_factor
        .wrapping_add(100_000)
        .wrapping_mul(width)
        / 100_000;
    width.wrapping_add(extra.wrapping_mul(100_000).wrapping_mul(support) / 100_000)
}

fn border_width(detail: &LandCombatDetailsRow) -> Result<i64, String> {
    let base = detail
        .border_war
        .require("LandCombatDetails.border_war")?
        .as_ref()
        .ok_or_else(|| "LandCombatDetails.border_war is absent for the border combat".to_owned())?
        .combat_width
        .require("LandBorderWarInputs.combat_width")?;
    Ok(detail
        .width_factor
        .wrapping_add(100_000)
        .wrapping_mul(*base)
        / 100_000)
}

fn width_badge(
    detail: &LandCombatDetailsRow,
    border_war: Option<bool>,
    ordinary: impl FnOnce() -> Result<i64, String>,
) -> Result<String, String> {
    match border_war {
        Some(true) => border_width(detail).map(|width| (width / 100_000).to_string()),
        Some(false) => ordinary().map(|width| (width / 100_000).to_string()),
        None => Err(
            "LandCombatModifiers lacks the combat class, so its native width getter is unknown"
                .into(),
        ),
    }
}

fn heading(scores: [i64; 2], progress: i64, bar_width: f32, icon_width: f32) -> (u32, f32) {
    let mul = |value: i64, factor: i64| value.wrapping_mul(factor) / 100_000;
    let (frame, shift) = if scores[0] > mul(scores[1], 200_000) {
        (0, icon_width)
    } else if scores[0] > mul(scores[1], 120_000) {
        (1, icon_width)
    } else if scores[1] > mul(scores[0], 200_000) {
        (4, 0.0)
    } else if scores[1] > mul(scores[0], 120_000) {
        (3, 0.0)
    } else {
        (2, (icon_width / 2.0).trunc())
    };
    let x = ((bar_width as i64)
        .wrapping_mul(progress)
        .wrapping_mul(100_000)
        / 10_000_000_000) as f32;
    (frame, x - shift)
}

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
) -> io::Result<()> {
    let details = rows::<tables::LandCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let division_rows = rows::<tables::LandCombatDivisions>(frame)?;
    let members = combat_rows(division_rows.rows(), id);
    let values = member_values(frame, members)?;
    let divisions: Divisions<'_> = land_combat_members(members, &values)
        .map(|row| ((row.side, row.division), row))
        .collect();
    for (side, prefix) in ["attacker", "defender"].iter().enumerate() {
        let width = army_width(&detail.sides[side].committed, side as u8, &divisions)
            .map(|width| (width / 100_000).to_string())
            .map_err(|reason| format!("Land combat {id} {prefix} army width is hidden: {reason}."));
        out.bind(&format!("{prefix}_army_width"), TEXT, width);
        let air_key = format!("{prefix}_air_text");
        if detail.sides.iter().all(|state| state.air_counts.is_empty()) {
            hide(&mut out.values, &air_key);
            continue;
        }
        let opponent = &detail.sides[1 - side];
        let line = builtin(
            static_content.assets.as_ref(),
            &static_content.localisation,
            "LAND_COMBAT_CAS_INFO",
            &[
                ("LOST", Argument::Int32(opponent.air_kills as i32)),
                ("STR", Argument::Fixed5(opponent.damage[0])),
                ("ORG", Argument::Fixed5(opponent.damage[1])),
            ],
        )
        .map_err(|reason| format!("Land combat {id} {prefix} CAS line is hidden: {reason}."));
        out.bind(&air_key, TEXT, line);
    }
    let combats = rows::<tables::LandCombats>(frame)?;
    let Some(combat) = combats.rows().iter().find(|row| row.id == id) else {
        out.withhold(
            &["combat_width", "combat_heading"],
            format!(
                "Land combat {id}: its LandCombats roster is missing, so the total width and the heading are hidden."
            ),
        );
        return Ok(());
    };
    let modifiers = rows::<tables::LandCombatModifiers>(frame)?;
    let total = width_badge(
        detail,
        super::details::border_war(modifiers.rows(), id),
        || {
            let movement: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)
                .map_err(|error| error.to_string())?
                .rows()
                .iter()
                .map(|row| (row.division, row.source))
                .collect();
            let widths = static_content
                .terrain
                .combat_terrain
                .get(&detail.province)
                .and_then(|terrain| static_content.terrain.combat_widths(terrain));
            let directions = direction_count(
                &detail.sides[0].committed,
                &divisions,
                detail.province,
                &movement,
            );
            match widths {
                Some(&widths) => match directions {
                    Ok(count) => Ok(total_width(detail, widths, count)),
                    Err(_) if widths[1] == 0 => Ok(total_width(detail, widths, 0)),
                    Err(reason) => Err(format!("attack direction count is unknown ({reason})")),
                },
                None => Err(format!(
                    "province {} has no terrain category in the content",
                    detail.province
                )),
            }
        },
    )
    .map_err(|why| format!("Land combat {id} total width is hidden: {why}."));
    out.bind("combat_width", TEXT, total);
    let conditions = conditions(frame)?;
    let participants = [&combat.attackers[..], &combat.defenders[..]];
    let scores = match side_scores(
        detail,
        land_combat_members(members, &values),
        &conditions,
        participants,
        static_content.scores,
    ) {
        Ok(scores) => scores,
        Err(gap) => {
            out.withhold(
                &["combat_heading"],
                format!(
                    "Land combat {id} heading is hidden: the survival estimate is unknown because {}.",
                    gap.reason()
                ),
            );
            return Ok(());
        }
    };
    let Some(progress) = Progresses::read(frame)?.attacker(id) else {
        out.withhold(
            &["combat_heading"],
            format!("Land combat {id} heading is hidden: no recorded progress for the combat."),
        );
        return Ok(());
    };
    let [bar, icon] = [
        "GFX_landcombat_progress",
        "GFX_land_battle_heading_indicator_strip",
    ]
    .map(|name| static_content.assets.sprite_size(name).ok_or(name));
    match (bar, icon) {
        (Ok(bar), Ok(icon)) => {
            let (frame, x) = heading(scores, progress, bar[0], icon[0]);
            let widget = out.values.entry("combat_heading".into()).or_default();
            widget.hidden = false;
            widget.frame = Some(frame);
            widget.offset = [x, 0.0];
        }
        (Err(name), _) | (_, Err(name)) => out.withhold(
            &["combat_heading"],
            format!("Land combat heading is hidden: the content lacks sprite {name}."),
        ),
    }
    out.warnings.push("Derived land values: committed widths apply recorded country/contextual-leader modifiers; ordinary combat total width takes terrain and attack directions, while border total width applies the recorded configured base; CAS damage and kills belong to the opposing side; heading uses the native early damage estimate or 24-loss mean and all participants.".into());
    Ok(())
}

fn army_width(committed: &[u64], side: u8, divisions: &Divisions<'_>) -> Result<i64, String> {
    committed.iter().try_fold(0_i64, |sum, unit| {
        let row = divisions
            .get(&(side, *unit))
            .ok_or_else(|| format!("committed division {unit} has no LandCombatDivisions row"))?;
        let width = division_width(row.row).ok_or_else(|| {
            format!(
                "division {unit}'s width modifier names eligibility callback kind {}, which the viewer does not support",
                row.context.eligibility_callback
            )
        })?;
        Ok(sum.wrapping_add(width))
    })
}

#[cfg(test)]
mod tests;
