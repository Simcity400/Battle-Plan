//! Air region statistics: the received-damage grids the strategic air view
//! draws in its default state for the
//! hovered province's strategic region and the viewing country, over the
//! last day. The rows come from AirRegionCombat at the frame's hour after the
//! daily prune ([`replay::air_region_derive`]); each side lists its air grid
//! and its ground grid for our missions and the enemy's. Each line uses the
//! item's tooltip key and its value as the received-damage formatter
//! writes it, green for ours and red for the enemy's.
use super::{Arg, Context, line};
use crate::map::raids::tooltips::Tooltip;
use replay::{air_region_derive as derive, rows::tables};

/// Each cause's tooltip keys, ours and the enemy's.
const KEYS: [(u32, &str, &str); 9] = [
    (
        0,
        "AIR_VIEW_DAMAGE_TO_BUILDINGS_BY_US",
        "AIR_VIEW_DAMAGE_TO_BUILDINGS_BY_ENEMY",
    ),
    (
        1,
        "AIR_VIEW_DAMAGE_TO_SHIPS_BY_US",
        "AIR_VIEW_DAMAGE_TO_SHIPS_BY_ENEMY",
    ),
    (
        2,
        "AIR_VIEW_DAMAGE_TO_DIVISIONS_BY_US",
        "AIR_VIEW_DAMAGE_TO_DIVISIONS_BY_ENEMY",
    ),
    (
        3,
        "AIR_VIEW_DAMAGE_DISRUPTED_PLANES_BY_US",
        "AIR_VIEW_DAMAGE_DISRUPTED_PLANES_BY_ENEMY",
    ),
    (
        4,
        "AIR_VIEW_DAMAGE_FROM_PLANES_BY_US",
        "AIR_VIEW_DAMAGE_FROM_PLANES_BY_ENEMY",
    ),
    (
        5,
        "AIR_VIEW_DAMAGE_FROM_AA_BY_US",
        "AIR_VIEW_DAMAGE_FROM_AA_BY_ENEMY",
    ),
    (
        6,
        "AIR_VIEW_DAMAGE_TO_LOGISTICS_BY_US",
        "AIR_VIEW_DAMAGE_TO_LOGISTICS_BY_ENEMY",
    ),
    (
        7,
        "AIR_VIEW_DAMAGE_TO_RAILWAY_GUNS_BY_US",
        "AIR_VIEW_DAMAGE_TO_RAILWAY_GUNS_BY_ENEMY",
    ),
    (
        10,
        "AIR_VIEW_DAMAGE_FROM_TRAINS_BY_US",
        "AIR_VIEW_DAMAGE_FROM_TRAINS_BY_ENEMY",
    ),
];

/// Fixed-point rounding: half a unit away from zero, then
/// the integer part, as an `i32`.
fn fixed_round(value: i64) -> i32 {
    let biased = if value < 0 {
        value.wrapping_sub(50_000)
    } else {
        value.wrapping_add(50_000)
    };
    (biased / 100_000) as i32
}

/// Received-damage value text: causes 0, 1, 2, 6 and 7 through
/// fixed5 formatting with precision 2, cause 3 through `AIRWING_DISRUPTED_VAL`'s
/// `VAL`, every other cause rounded to an integer.
fn value(context: &Context<'_>, item: derive::Item) -> Result<String, String> {
    let text = match item.cause {
        0 | 1 | 2 | 6 | 7 => installed_content::fixed5::fixed5_to_string(item.amount, 2),
        3 => context.loc(
            "AIRWING_DISRUPTED_VAL",
            &[("VAL", Arg::Fixed5(item.amount))],
        )?,
        _ => fixed_round(item.amount).to_string(),
    };
    let colour = if item.ours() { 'G' } else { 'R' };
    Ok(format!("\u{11}{colour}{text}\u{11}!"))
}

/// One grid line: the tooltip's first line, then the value.
fn item_line(context: &Context<'_>, item: derive::Item) -> Result<String, String> {
    let (_, ours, theirs) = KEYS
        .iter()
        .find(|(cause, _, _)| *cause == item.cause)
        .ok_or_else(|| {
            format!(
                "no strategic air view tooltip for damage cause {}",
                item.cause
            )
        })?;
    let label = context.loc(if item.ours() { ours } else { theirs }, &[])?;
    let label = label.split('\n').next().unwrap_or_default().to_owned();
    Ok(format!("{label}: {}", value(context, item)?))
}

/// The viewing country's holder of the province's strategic region at the
/// frame's hour, or `None` when the region or the holder has no records.
fn retained<'a>(
    context: &'a Context<'_>,
    rows: &'a [replay::rows::AirRegionCombatRow],
) -> Result<Option<derive::Retained<'a>>, String> {
    let Some(region) = context.content.counters.region(context.province.id) else {
        return Ok(None);
    };
    let ours = context.vision.ours();
    let Some(row) = rows
        .iter()
        .find(|row| row.region == region && context.slot(row.country) == Ok(ours))
    else {
        return Ok(None);
    };
    // The daily pass prunes only countries owning a state.
    let states = context
        .frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?;
    if !states
        .iter()
        .any(|state| context.slot(state.owner) == Ok(ours))
    {
        return Err(derive::UNPRUNED_CONTAINER.into());
    }
    let timeout = *context
        .content
        .province_hover
        .air_log_timeout
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(Some(derive::retained(row, context.frame.hour, timeout)))
}

pub(super) fn append(context: &Context<'_>, out: &mut Tooltip) {
    let rows = match context.frame.complete::<tables::AirRegionCombat>() {
        Ok(rows) => rows,
        Err(e) => return line(out, "air region statistics", 0, Err(e.to_string())),
    };
    let state = match retained(context, rows) {
        Ok(Some(state)) => state,
        Ok(None) => return,
        Err(reason) => return line(out, "air region statistics", 0, Err(reason)),
    };
    for (enemy, header) in [
        (false, "AIR_VIEW_SHOW_OUR_MISSIONS"),
        (true, "AIR_VIEW_SHOW_ENEMY_MISSIONS"),
    ] {
        let grids = match derive::grids(state, context.frame.hour, enemy) {
            Ok(grids) => grids,
            Err(reason) => {
                line(out, "air region statistics", 0, Err(reason.into()));
                continue;
            }
        };
        if grids.air.is_empty() && grids.ground.is_empty() {
            continue;
        }
        line(
            out,
            "air region statistics header",
            0,
            context
                .loc(header, &[])
                .map(|text| text.split('\n').next().unwrap_or_default().to_owned()),
        );
        for item in grids.air.into_iter().chain(grids.ground) {
            line(
                out,
                "air region statistics line",
                1,
                item_line(context, item),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_as_the_native_helpers_do() {
        assert_eq!(fixed_round(149_999), 1);
        assert_eq!(fixed_round(150_000), 2);
        assert_eq!(fixed_round(-150_000), -2);
        assert_eq!(fixed_round(-149_999), -1);
        assert_eq!(fixed_round(0), 0);
        // Every cause the grids list has a tooltip pair; 8 and 9 never show.
        for cause in [0, 1, 2, 3, 4, 5, 6, 7, 10] {
            assert!(KEYS.iter().any(|(listed, _, _)| *listed == cause));
        }
        assert!(
            !KEYS
                .iter()
                .any(|(listed, _, _)| *listed == 9 || *listed == 8)
        );
    }
}
