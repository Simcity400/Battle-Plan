//! Derived land-window operands from recorded combat state.
use super::binding::{Bindings, TEXT, builtin, hide};
use crate::{document::Document, map::tables::rows};
use installed_content::localisation::Argument;
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDetailsRow, LandCombatDivisionRow, LandDivisionContext, tables},
};
use std::{collections::BTreeMap, io};

pub(super) type Divisions<'a> = BTreeMap<(u8, u64), &'a LandCombatDivisionRow>;

/// A division's current hit points and organisation from `Divisions`, with
/// their maxima from `DivisionStatus` when it has a row this hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Condition {
    pub(crate) strength: i64,
    pub(crate) max_strength: Option<i64>,
    pub(crate) organisation: i64,
    pub(crate) max_organisation: Option<i64>,
}

pub(crate) type Conditions = BTreeMap<u64, Condition>;

/// Every division with a `Divisions` row this hour. A missing
/// `DivisionStatus` row leaves only the maxima unknown:
/// organisation sum reads current organisation alone.
pub(crate) fn conditions(frame: &DecodedFrame) -> io::Result<Conditions> {
    let status: BTreeMap<_, _> = rows::<tables::DivisionStatus>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row))
        .collect();
    Ok(rows::<tables::Divisions>(frame)?
        .rows()
        .iter()
        .map(|row| {
            let status = status.get(&row.id);
            (
                row.id,
                Condition {
                    strength: row.hit_points,
                    max_strength: status.map(|status| status.maximum_hit_points),
                    organisation: row.organisation,
                    max_organisation: status.map(|status| status.maximum_organisation),
                },
            )
        })
        .collect())
}

/// Why [`conditions`] holds no entry for a division: which of its two tables
/// lacks the division's row, and whether that table is complete this hour.
pub(crate) fn condition_gap(frame: &DecodedFrame) -> io::Result<impl Fn(u64) -> String + '_> {
    let divisions = rows::<tables::Divisions>(frame)?;
    let status = rows::<tables::DivisionStatus>(frame)?;
    Ok(move |unit: u64| {
        let lacks = |table: &str, present: bool, complete: bool| {
            (!present).then(|| {
                if complete {
                    format!("the {table} table has no row for it")
                } else {
                    format!("the {table} table is incomplete this hour and holds no row for it")
                }
            })
        };
        let gaps: Vec<String> = [
            lacks(
                "Divisions",
                divisions.rows().iter().any(|row| row.id == unit),
                divisions.is_complete(),
            ),
            lacks(
                "DivisionStatus",
                status.rows().iter().any(|row| row.division == unit),
                status.is_complete(),
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        format!(
            "division {unit}'s condition is unknown: {}",
            gaps.join(", and ")
        )
    })
}

/// The `NMilitary` defines early survival estimate reads. The
/// native estimate uses the named avoid chances directly, not their complements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScoreDefines {
    pub avoid_with_defence: i64,
    pub avoid_without_defence: i64,
    pub organisation_die: i32,
}

impl ScoreDefines {
    pub(crate) fn read(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            avoid_with_defence: defines.fixed5("NMilitary", "BASE_CHANCE_TO_AVOID_HIT")?,
            avoid_without_defence: defines.fixed5("NMilitary", "CHANCE_TO_AVOID_HIT_AT_NO_DEF")?,
            organisation_die: defines.int32("NMilitary", "LAND_COMBAT_ORG_DICE_SIZE")?,
        })
    }

    /// Values exercised by the tests.
    #[cfg(test)]
    pub(crate) const VANILLA: Self = Self {
        avoid_with_defence: 9_000_000,
        avoid_without_defence: 6_000_000,
        organisation_die: 4,
    };
}

fn fixed_mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}

// The exceptional range excludes i64::MAX, MIN and MIN + 1.
fn ranged(value: i64) -> bool {
    (value as u64).wrapping_add(0x7FFF_FFFF_FFFF_FFFE) < 0xFFFF_FFFF_FFFF_FFFD
}

// Current HP as a fraction of maximum HP, clamped
// to [0, one]; a zero maximum or an out-of-range current HP gives one.
// The game then truncates to tenths and keeps at least one tenth.
fn coarse_strength(strength: i64, maximum: i64) -> i64 {
    let fraction = if maximum != 0 && ranged(strength) {
        strength
            .wrapping_mul(100_000)
            .wrapping_div(maximum)
            .clamp(0, 100_000)
    } else {
        100_000
    };
    (fraction.wrapping_mul(1_000_000) / 10_000_000_000).max(1) * 10_000
}

/// Why the survival estimate has no value: an operand it reads is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScoreGap {
    /// A division without its `Divisions` row: a participant's current
    /// organisation or a firing committed division's current HP.
    Division(u64),
    /// A firing committed division without its `DivisionStatus` row, whose
    /// maximum HP is read on the early path.
    MaximumStrength(u64),
    /// A committed division without its `LandCombatDivisions` row.
    Member { side: u8, division: u64 },
    /// A committed division whose `LandCombatDivisions` row holds neither
    /// its stats and scales nor an older recording's packets of them.
    Numbers { side: u8, division: u64 },
}

impl ScoreGap {
    /// What the estimate lacks, naming the division and the table.
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Division(division) => format!(
                "division {division} has no Divisions row, whose current organisation and hit points it reads"
            ),
            Self::MaximumStrength(division) => format!(
                "committed division {division} has no DivisionStatus row, whose maximum hit points the early damage estimate reads"
            ),
            Self::Member { side, division } => format!(
                "committed division {division} on side {side} has no LandCombatDivisions row"
            ),
            Self::Numbers { side, division } => format!(
                "committed division {division} on side {side} has no stats and scales in its LandCombatDivisions row"
            ),
        }
    }
}

fn early_damage(
    detail: &LandCombatDetailsRow,
    divisions: &Divisions<'_>,
    conditions: &Conditions,
    firing_side: usize,
    defines: ScoreDefines,
) -> Result<i64, ScoreGap> {
    let member = |side: usize, id: u64| {
        divisions
            .get(&(side as u8, id))
            .copied()
            .ok_or(ScoreGap::Member {
                side: side as u8,
                division: id,
            })
    };
    // A member's stats and scales, which an older recording holds as packets.
    let numbers = |side: usize, id: u64| -> Result<([i64; 5], [i64; 4]), ScoreGap> {
        let row = member(side, id)?;
        row.stat_values()
            .zip(row.scales())
            .ok_or(ScoreGap::Numbers {
                side: side as u8,
                division: id,
            })
    };
    let targets = &detail.sides[1 - firing_side].committed;
    let average = |stat: usize| -> Result<i64, ScoreGap> {
        if targets.is_empty() {
            return Ok(0);
        }
        let sum = targets.iter().try_fold(0_i64, |sum, id| {
            Ok(sum.wrapping_add(numbers(1 - firing_side, *id)?.0[stat]))
        })?;
        Ok(sum.wrapping_mul(100_000) / (targets.len() as i64 * 100_000))
    };
    let hardness = average(2)?;
    // An attacker meets defence; a defender meets breakthrough.
    let defended = fixed_mul(average(usize::from(firing_side == 1))?, 10_000);
    let bonus = if firing_side == 0 {
        detail.attacker_bonus
    } else {
        detail.defender_bonus
    };
    detail.sides[firing_side]
        .committed
        .iter()
        .rev()
        .try_fold(0_i64, |total, id| {
            let (stats, scales) = numbers(firing_side, *id)?;
            let soft = fixed_mul(
                fixed_mul(stats[3], 10_000),
                100_000_i64.wrapping_sub(hardness),
            );
            let hard = fixed_mul(fixed_mul(stats[4], 10_000), hardness);
            let attack = fixed_mul(soft.wrapping_add(hard), fixed_mul(scales[2], scales[0]));
            let covered = fixed_mul(
                fixed_mul(attack.min(defended), defines.avoid_with_defence),
                1_000,
            );
            let uncovered = fixed_mul(
                fixed_mul(
                    attack.wrapping_sub(defended).max(0),
                    defines.avoid_without_defence,
                ),
                1_000,
            );
            let condition = conditions.get(id).ok_or(ScoreGap::Division(*id))?;
            let maximum = condition
                .max_strength
                .ok_or(ScoreGap::MaximumStrength(*id))?;
            let damage = coarse_strength(condition.strength, maximum)
                .wrapping_add(bonus)
                .wrapping_mul(i64::from(defines.organisation_die))
                .wrapping_mul(50_000)
                / 100_000;
            Ok(total.wrapping_add(fixed_mul(damage, covered.wrapping_add(uncovered))))
        })
}

// absent/feature-ineligible modifiers return zero; metadata bit 4
// clamps the getter to [0,1]. Country and contextual leader use the same rule.
// `key` indexes the recorded modifiers: 0xFB, 0x10F (width), 0x250.
pub(super) fn modifier(context: &LandDivisionContext, key: usize, leader: bool) -> Option<i64> {
    let value = if leader {
        &context.leader_modifiers[key]
    } else {
        &context.country_modifiers[key]
    };
    let [flags, selector] = context.modifier_metadata[key];
    if value.present == 0 || (leader && context.leader_modifier_kind == 0) {
        return Some(0);
    }
    if selector != 0 {
        match context.eligibility_callback {
            0 => return Some(0),
            1 if selector & context.feature_flags == 0 => return Some(0),
            1 => (),
            _ => return None,
        }
    }
    Some(if flags & 0x10 != 0 {
        value.raw.clamp(0, 100_000)
    } else {
        value.raw
    })
}

pub(super) fn division_width(row: &LandCombatDivisionRow) -> Option<i64> {
    if row.context.template == 0 {
        return Some(0);
    }
    // Multiply signed count by fixed one before width, preserving 64-bit
    // overflow. Apply the combined country and leader width multiplier once.
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

/// A unit's direction key: its location, or its
/// movement source when it stands in the combat province and has one.
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

/// direction count over the attacker committed divisions in
/// recorded order: one direction per fresh key, except positive
/// direction-group units after the first, which fold into its direction.
/// A missing row leaves the count unknown rather than guessed; the error
/// names the attacker and the table that lacks its row.
pub(super) fn direction_count(
    committed: &[u64],
    divisions: &Divisions<'_>,
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

/// the width-factor term over the terrain's `combat_width`, plus
/// one `combat_support_width` term per direction beyond the first, all fixed5
/// with native truncation.
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

/// Border combat uses the configured base alone, with wrapped
/// signed multiplication; terrain and attack directions are not operands.
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
    // Border combat uses its recorded base width.
    match border_war {
        Some(true) => border_width(detail).map(|width| (width / 100_000).to_string()),
        Some(false) => ordinary().map(|width| (width / 100_000).to_string()),
        None => Err(
            "LandCombatModifiers lacks the combat class, so its native width getter is unknown"
                .into(),
        ),
    }
}

/// Survival estimates in fixed5 for the two complete sides.
/// `participants` is the ordered full LandCombats census (not committed only).
/// Early battles use damage estimates and the verified content definition context.
pub(crate) fn side_scores<'a>(
    detail: &LandCombatDetailsRow,
    divisions: impl IntoIterator<Item = &'a LandCombatDivisionRow>,
    conditions: &Conditions,
    participants: [&[u64]; 2],
    defines: ScoreDefines,
) -> Result<[i64; 2], ScoreGap> {
    let divisions: Divisions<'_> = divisions
        .into_iter()
        .filter(|row| row.combat == detail.combat)
        .map(|row| ((row.side, row.division), row))
        .collect();
    let mut scores = [0; 2];
    for side in 0..2 {
        let state = &detail.sides[side];
        // Use wrapping 64-bit sums and products over current organisation,
        // never maximum organisation.
        let organisation = participants[side].iter().try_fold(0_i64, |sum, id| {
            Ok(sum.wrapping_add(
                conditions
                    .get(id)
                    .ok_or(ScoreGap::Division(*id))?
                    .organisation,
            ))
        })?;
        if state.organisation_loss_count < 24 {
            let damage = early_damage(detail, &divisions, conditions, 1 - side, defines)?;
            scores[side] = if damage > 99 {
                comparison_ratio(organisation, damage)
            } else {
                0x0017_4875_6160 // Native no-effective-loss sentinel, not zero.
            };
            continue;
        }
        // The 24 samples' signed 64-bit sum, with wrapping addition,
        // recorded as such.
        let loss = state.organisation_loss_sum;
        let average = comparison_ratio(loss, 2_400_000);
        let average = if average < 1 { 100 } else { average };
        scores[side] = comparison_ratio(organisation, average);
    }
    Ok(scores)
}

/// Sums and products wrap at 64 bits before signed division truncates.
/// The three special fixed-point values bypass division and produce the
/// positive 0xffffffff sentinel.
/// Both callers supply a positive denominator, so the division cannot trap.
fn comparison_ratio(value: i64, denominator: i64) -> i64 {
    if (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        i64::from(u32::MAX)
    } else {
        value.wrapping_mul(100_000) / denominator
    }
}

// The game selects one-based frames 1,2,3,4,5 at strict 2x/1.2x
// boundaries. Negative offset makes leftward arrows end at the progress edge.
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

/// Apply after `details::bind`; no additional assets beyond landcombat.gfx.
pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
) -> io::Result<()> {
    let details = rows::<tables::LandCombatDetails>(frame)?;
    // Without a details row `details::land` reports what the window lacks.
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let division_rows = rows::<tables::LandCombatDivisions>(frame)?;
    let divisions: Divisions<'_> = division_rows
        .rows()
        .iter()
        .filter(|row| row.combat == id)
        .map(|row| ((row.side, row.division), row))
        .collect();
    for (side, prefix) in ["attacker", "defender"].iter().enumerate() {
        // The game deliberately truncates, unlike the stat formatter.
        let width = army_width(&detail.sides[side].committed, side as u8, &divisions)
            .map(|width| (width / 100_000).to_string())
            .map_err(|reason| format!("Land combat {id} {prefix} army width is hidden: {reason}."));
        out.bind(&format!("{prefix}_army_width"), TEXT, width);
        let air_key = format!("{prefix}_air_text");
        if detail.sides.iter().all(|state| state.air_counts.is_empty()) {
            hide(&mut out.values, &air_key);
            continue;
        }
        // LAND_COMBAT_CAS_INFO uses the opposing side's losses as an integer
        // argument and strength/organisation damage as fixed-point arguments.
        let opponent = &detail.sides[1 - side];
        let line = builtin(
            &document.icons.combat,
            &document.names.localisation,
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
            let widths = document
                .counters
                .combat_terrain
                .get(&detail.province)
                .and_then(|terrain| document.counters.combat_widths(terrain));
            let directions = direction_count(
                &detail.sides[0].committed,
                &divisions,
                detail.province,
                &movement,
            );
            match widths {
                Some(&widths) => match directions {
                    Ok(count) => Ok(total_width(detail, widths, count)),
                    // Without a support width the direction count cannot matter.
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
        division_rows.rows(),
        &conditions,
        participants,
        document.counters.defines.land_combat.scores,
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
    let Some(progress) = super::progress::Progresses::read(frame)?.attacker(id) else {
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
    .map(|name| document.icons.combat.sprite_size(name).ok_or(name));
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

/// The committed divisions' summed width, or why it is
/// unknown.
fn army_width(committed: &[u64], side: u8, divisions: &Divisions<'_>) -> Result<i64, String> {
    committed.iter().try_fold(0_i64, |sum, unit| {
        let row = divisions
            .get(&(side, *unit))
            .ok_or_else(|| format!("committed division {unit} has no LandCombatDivisions row"))?;
        let width = division_width(row).ok_or_else(|| {
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
