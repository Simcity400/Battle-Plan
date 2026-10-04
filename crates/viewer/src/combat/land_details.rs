//! Derived land-window operands from the pinned native binders.
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
/// `DivisionStatus` row leaves only the maxima unknown: 1412A2AA0's
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

/// The `NMilitary` defines 1412A26B0's early survival estimate reads. The
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

    /// The shipped values, for tests only.
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

// 1412A2AA0 and 140C781E0 range test (`x + 0x7FFFFFFFFFFFFFFE <
// 0xFFFFFFFFFFFFFFFD`): excludes i64::MAX, MIN and MIN + 1.
fn ranged(value: i64) -> bool {
    (value as u64).wrapping_add(0x7FFF_FFFF_FFFF_FFFE) < 0xFFFF_FFFF_FFFF_FFFD
}

// 140C781E0: current HP (+0xF8) times one over maximum HP (+0x108), clamped
// to [0, one]; a zero maximum or an out-of-range current HP gives one.
// 140BECAD0 then truncates to tenths and keeps at least one tenth.
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
    /// organisation (vtable +0x120, 140C78810) or a firing committed
    /// division's current HP (+0xF8).
    Division(u64),
    /// A firing committed division without its `DivisionStatus` row, whose
    /// maximum HP (+0x108) 140C781E0 reads on the early path.
    MaximumStrength(u64),
    /// A committed division without its `LandCombatDivisions` row.
    Member { side: u8, division: u64 },
}

impl ScoreGap {
    /// What the estimate lacks, naming the division and the table.
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Division(division) => format!(
                "division {division} has no Divisions row, whose current organisation and hit points it reads"
            ),
            Self::MaximumStrength(division) => format!(
                "committed division {division} has no DivisionStatus row, whose maximum hit points the early damage estimate reads (140C781E0)"
            ),
            Self::Member { side, division } => format!(
                "committed division {division} on side {side} has no LandCombatDivisions row"
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
    let targets = &detail.sides[1 - firing_side].committed;
    let average = |stat: usize| -> Result<i64, ScoreGap> {
        if targets.is_empty() {
            return Ok(0);
        }
        let sum = targets.iter().try_fold(0_i64, |sum, id| {
            Ok(sum.wrapping_add(member(1 - firing_side, *id)?.stat_values[stat]))
        })?;
        Ok(sum.wrapping_mul(100_000) / (targets.len() as i64 * 100_000))
    };
    let hardness = average(2)?;
    // An attacker meets defence (+A8); a defender meets breakthrough (+B0).
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
            let row = member(firing_side, *id)?;
            let soft = fixed_mul(
                fixed_mul(row.stat_values[3], 10_000),
                100_000_i64.wrapping_sub(hardness),
            );
            let hard = fixed_mul(fixed_mul(row.stat_values[4], 10_000), hardness);
            let attack = fixed_mul(
                soft.wrapping_add(hard),
                fixed_mul(row.scales[2], row.scales[0]),
            );
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

// 140551820: absent/feature-ineligible modifiers return zero; metadata bit 4
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
    // 140C6E270 multiplies signed count by fixed one before width, including
    // native qword overflow, then applies one + country10F + leader10F once.
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

/// A unit's direction key (1413CD7E0, 141298E00): its location, or its
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

/// 141298E00's direction count over the attacker committed divisions in
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

/// 141298D50: the width-factor term over the terrain's `combat_width`, plus
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

/// Native 1412A2AA0 survival estimates, in fixed5, for the two complete sides.
/// `participants` is the ordered full LandCombats census (not committed only).
/// Early battles use 1412A26B0 and the verified content definition context.
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
        // Plain qword sums and products, as 1412A2AA0 computes them. It
        // reads current organisation only, never a maximum.
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
        let loss = state
            .organisation_loss
            .iter()
            .fold(0_i64, |sum, value| sum.wrapping_add(*value));
        let average = comparison_ratio(loss, 2_400_000);
        let average = if average < 1 { 100 } else { average };
        scores[side] = comparison_ratio(organisation, average);
    }
    Ok(scores)
}

/// 1412A2AA0: ADD/IMUL keep the low qword, then signed IDIV truncates.
/// Only the three special fixed-point values bypass division; MOV ECX,
/// 0xffffffff (1412A2B9A / 1412A2C35) zero-extends their positive sentinel.
/// Both callers supply a positive denominator, so the division cannot trap.
fn comparison_ratio(value: i64, denominator: i64) -> i64 {
    if (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        i64::from(u32::MAX)
    } else {
        value.wrapping_mul(100_000) / denominator
    }
}

// 1415C8950 selects one-based frames 1,2,3,4,5 at strict 2x/1.2x
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
        // 1415C8950 deliberately truncates, unlike the stat formatter.
        let width = army_width(&detail.sides[side].committed, side as u8, &divisions)
            .map(|width| (width / 100_000).to_string())
            .map_err(|reason| format!("Land combat {id} {prefix} army width is hidden: {reason}."));
        out.bind(&format!("{prefix}_army_width"), TEXT, width);
        let air_key = format!("{prefix}_air_text");
        if detail.sides.iter().all(|state| state.air_counts.is_empty()) {
            hide(&mut out.values, &air_key);
            continue;
        }
        // 1415C94A0 -> 1415B8200: LAND_COMBAT_CAS_INFO with LOST (type 5,
        // the opposing combatant's +0x140 dword), STR and ORG (type 9, its
        // +0x148 and +0x150).
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
    let movement: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)?
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
    let total = match widths {
        Some(&widths) => match &directions {
            Ok(count) => Ok((total_width(detail, widths, *count) / 100_000).to_string()),
            // Without a support width the direction count cannot matter.
            Err(_) if widths[1] == 0 => Ok((total_width(detail, widths, 0) / 100_000).to_string()),
            Err(reason) => Err(format!(
                "Land combat {id} total width is hidden: the 141298E00 direction count is unknown ({reason})."
            )),
        },
        None => Err(format!(
            "Land combat {id} total width is hidden: province {} has no terrain category in the content.",
            detail.province
        )),
    };
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
                    "Land combat {id} heading is hidden: the 1412A2AA0 survival estimate is unknown because {}.",
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
            format!("Land combat heading is hidden: the content lacks sprite {name} (1415C8950)."),
        ),
    }
    out.warnings.push("Derived land values: committed widths apply recorded country/contextual-leader modifiers; the total width takes the province terrain's content widths; CAS damage and kills belong to the opposing side; heading uses the native early damage estimate or 24-loss mean and all participants.".into());
    Ok(())
}

/// The committed divisions' summed width (140C6E270 each), or why it is
/// unknown.
fn army_width(committed: &[u64], side: u8, divisions: &Divisions<'_>) -> Result<i64, String> {
    committed.iter().try_fold(0_i64, |sum, unit| {
        let row = divisions
            .get(&(side, *unit))
            .ok_or_else(|| format!("committed division {unit} has no LandCombatDivisions row"))?;
        let width = division_width(row).ok_or_else(|| {
            format!(
                "division {unit}'s width modifier names eligibility callback kind {}, which 140551820 is not reproduced for",
                row.context.eligibility_callback
            )
        })?;
        Ok(sum.wrapping_add(width))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{LandDivisionModifier, LandDivisionWidthTerm};

    #[test]
    fn early_score_uses_opposing_committed_damage_and_all_current_organisation() {
        let mut detail = LandCombatDetailsRow {
            combat: 9,
            ..Default::default()
        };
        detail.sides[0].committed = vec![1];
        detail.sides[1].committed = vec![2];
        let divisions = [
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division: 1,
                stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0],
                scales: [100_000; 4],
                ..Default::default()
            },
            LandCombatDivisionRow {
                combat: 9,
                side: 1,
                division: 2,
                stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0],
                scales: [100_000; 4],
                ..Default::default()
            },
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division: 3,
                ..Default::default()
            },
        ];
        let condition = |organisation| Condition {
            strength: 100_000,
            max_strength: Some(100_000),
            organisation,
            max_organisation: Some(organisation),
        };
        let mut conditions = Conditions::from([
            (1, condition(3_000_000)),
            (2, condition(6_000_000)),
            (3, condition(3_000_000)),
        ]);
        let scores = |detail: &LandCombatDetailsRow, conditions: &Conditions, first: &[u64]| {
            side_scores(
                detail,
                &divisions,
                conditions,
                [first, &[2]],
                ScoreDefines::VANILLA,
            )
        };
        assert_eq!(scores(&detail, &conditions, &[1]), Ok([200_000, 333_333]));
        assert_eq!(
            scores(&detail, &conditions, &[1, 3]),
            Ok([400_000, 333_333])
        );
        conditions.get_mut(&2).unwrap().strength = 49_999; // Native tenths truncate to 0.4.
        detail.defender_bonus = 10_000; // Added to coarse strength, not a multiplier.
        assert_eq!(scores(&detail, &conditions, &[1]), Ok([400_000, 333_333]));
        detail.sides[1].committed.clear();
        assert_eq!(
            scores(&detail, &conditions, &[1]).map(|v| v[0]),
            Ok(99_999_900_000)
        );
        // A participant without its Divisions and DivisionStatus rows is unknown.
        conditions.remove(&3);
        assert_eq!(
            scores(&detail, &conditions, &[1, 3]),
            Err(ScoreGap::Division(3))
        );
    }

    /// Conditions read from the recorded tables: `(id, hit points,
    /// organisation)` Divisions rows, and `(id, maximum hit points)`
    /// DivisionStatus rows.
    fn recorded(divisions: &[(u64, i64, i64)], status: &[(u64, i64)]) -> Conditions {
        use replay::{
            frame::{Metrics, TableView},
            rows::{DivisionRow, DivisionStatusRow, TableId, TableRows},
        };
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 1,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not recorded".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            divisions
                .iter()
                .map(|&(id, hit_points, organisation)| DivisionRow {
                    id,
                    hit_points,
                    organisation,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(
                status
                    .iter()
                    .map(|&(division, maximum_hit_points)| DivisionStatusRow {
                        division,
                        maximum_hit_points,
                        maximum_organisation: 6_000_000,
                        ..Default::default()
                    })
                    .collect(),
            ));
        conditions(&frame).unwrap()
    }

    /// 1412A2AA0 sums current organisation only, so a division without its
    /// DivisionStatus row still scores; only the early path's 140C781E0 on
    /// a firing committed division reads its maximum hit points.
    #[test]
    fn a_missing_status_row_blocks_only_early_firing_damage() {
        let mut detail = LandCombatDetailsRow {
            combat: 9,
            ..Default::default()
        };
        detail.sides[0].committed = vec![1];
        detail.sides[1].committed = vec![2];
        let member = |side, division| LandCombatDivisionRow {
            combat: 9,
            side,
            division,
            stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0],
            scales: [100_000; 4],
            ..Default::default()
        };
        let divisions = [member(0, 1), member(1, 2), member(0, 3)];
        let scores = |detail: &LandCombatDetailsRow, conditions: &Conditions| {
            side_scores(
                detail,
                &divisions,
                conditions,
                [&[1, 3], &[2]],
                ScoreDefines::VANILLA,
            )
        };
        let full = [(1, 100_000), (2, 100_000)];
        let conditions = recorded(
            &[
                (1, 100_000, 3_000_000),
                (2, 100_000, 6_000_000),
                (3, 50_000, 3_000_000),
            ],
            &full,
        );
        assert_eq!(
            conditions[&3],
            Condition {
                strength: 50_000,
                max_strength: None,
                organisation: 3_000_000,
                max_organisation: None,
            }
        );
        // Division 3 is a participant in reserve: its organisation counts.
        assert_eq!(scores(&detail, &conditions), Ok([400_000, 333_333]));
        // Committed, it fires on the defender's early estimate.
        detail.sides[0].committed = vec![1, 3];
        assert_eq!(
            scores(&detail, &conditions),
            Err(ScoreGap::MaximumStrength(3))
        );
        // The late path reads the loss ring and no maximum.
        let mut late = detail.clone();
        for side in &mut late.sides {
            side.organisation_loss_count = 24;
            side.organisation_loss = [100_000; 24];
        }
        assert_eq!(scores(&late, &conditions), Ok([6_000_000, 6_000_000]));
        // Without its Divisions row a participant has no organisation.
        let without = recorded(&[(1, 100_000, 3_000_000), (2, 100_000, 6_000_000)], &full);
        assert_eq!(scores(&late, &without), Err(ScoreGap::Division(3)));

        // 140C781E0: a zero maximum or an out-of-range current HP reads as
        // full strength; a negative ratio clamps to zero, which 140BECAD0
        // raises to one tenth like any ratio under a tenth.
        detail.sides[0].committed = vec![1];
        let defender = |hit_points, maximum| {
            scores(
                &detail,
                &recorded(
                    &[
                        (1, hit_points, 3_000_000),
                        (2, 100_000, 6_000_000),
                        (3, 50_000, 3_000_000),
                    ],
                    &[(1, maximum), (2, 100_000)],
                ),
            )
            .map(|scores| scores[1])
        };
        assert_eq!(defender(100_000, 100_000), Ok(333_333));
        assert_eq!(defender(50_000, 0), Ok(333_333));
        assert_eq!(defender(i64::MAX, 100_000), Ok(333_333));
        assert_eq!(defender(250_000, 100_000), Ok(333_333));
        assert_eq!(defender(-5, 100_000), defender(1, 100_000));
        assert_ne!(defender(1, 100_000), Ok(333_333));
    }

    #[test]
    fn width_adds_context_modifiers_before_one_multiplication() {
        let mut row = LandCombatDivisionRow::default();
        row.context.template = 1;
        row.context.leader_modifier_kind = 1;
        row.width_terms.push(LandDivisionWidthTerm {
            count: 9,
            width: 200_000,
            ..Default::default()
        });
        row.context.country_modifiers[1] = LandDivisionModifier {
            raw: -10_000,
            present: 1,
        };
        row.context.leader_modifiers[1] = LandDivisionModifier {
            raw: -5_000,
            present: 1,
        };
        assert_eq!(division_width(&row), Some(1_530_000));
        row.context.modifier_metadata[1][0] = 0x10;
        assert_eq!(division_width(&row), Some(1_800_000));
        row.context.country_modifiers[1].raw = 50_000;
        row.context.modifier_metadata[1][1] = 2;
        assert_eq!(division_width(&row), Some(1_800_000));
        row.context.eligibility_callback = 1;
        row.context.feature_flags = 2;
        assert_eq!(division_width(&row), Some(2_700_000));
    }

    #[test]
    fn heading_uses_strict_thresholds_and_progress_edge() {
        assert_eq!(heading([200_000, 100_000], 50_000, 190.0, 16.0), (1, 79.0));
        assert_eq!(heading([120_000, 100_000], 50_000, 190.0, 16.0), (2, 87.0));
        assert_eq!(heading([100_000, 200_001], 50_000, 190.0, 16.0), (4, 95.0));
    }

    #[test]
    fn survival_uses_full_roster_and_rejects_early_or_missing_rows() {
        let mut detail = LandCombatDetailsRow {
            combat: 9,
            ..Default::default()
        };
        let divisions: [LandCombatDivisionRow; 0] = [];
        let conditions = Conditions::from([
            (
                1,
                Condition {
                    organisation: 3_000_000,
                    ..Default::default()
                },
            ),
            (
                2,
                Condition {
                    organisation: 6_000_000,
                    ..Default::default()
                },
            ),
        ]);
        let scores = |detail: &LandCombatDetailsRow, first: &[u64]| {
            side_scores(
                detail,
                &divisions,
                &conditions,
                [first, &[2]],
                ScoreDefines::VANILLA,
            )
        };
        // An early battle whose committed division has no row is unknown.
        detail.sides[1].committed = vec![99];
        assert_eq!(
            scores(&detail, &[1]),
            Err(ScoreGap::Member {
                side: 1,
                division: 99
            })
        );
        for side in &mut detail.sides {
            side.organisation_loss_count = 24;
            side.organisation_loss = [100_000; 24];
        }
        assert_eq!(scores(&detail, &[1]), Ok([3_000_000, 6_000_000]));
        assert_eq!(scores(&detail, &[1, 3]), Err(ScoreGap::Division(3)));
        detail.sides[0].organisation_loss = [0; 24];
        assert_eq!(scores(&detail, &[1]), Ok([3_000_000_000, 6_000_000]));
        // Sums and products wrap as 1412A2AA0's qwords do, and a sum at the
        // fixed-point range edge scores 0xFFFFFFFF instead of failing.
        detail.sides[0].organisation_loss = [0; 24];
        detail.sides[0].organisation_loss[0] = i64::MAX;
        assert_eq!(
            scores(&detail, &[1]).map(|v| v[0]),
            Ok(300_000_000_000 / 0xFFFF_FFFF)
        );
        detail.sides[0].organisation_loss[0] = i64::MAX / 50_000;
        let wrapped = (i64::MAX / 50_000).wrapping_mul(100_000) / 2_400_000;
        assert_eq!(
            scores(&detail, &[1]).map(|v| v[0]),
            Ok(300_000_000_000 / if wrapped < 1 { 100 } else { wrapped })
        );
    }

    #[test]
    fn survival_keeps_native_wrapping_sums_products_and_special_values() {
        let mut detail = LandCombatDetailsRow {
            combat: 9,
            ..Default::default()
        };
        for side in &mut detail.sides {
            side.organisation_loss_count = 24;
            side.organisation_loss = [100_000; 24];
        }
        let divisions = [1, 2].map(|division| LandCombatDivisionRow {
            combat: 9,
            side: 0,
            division,
            ..Default::default()
        });
        // Current organisation is the Divisions row's (Conditions).
        let mut organisation = [0_i64, 1];
        let score = |detail: &LandCombatDetailsRow, organisation: [i64; 2], units| {
            let conditions: Conditions = [1_u64, 2]
                .into_iter()
                .zip(organisation)
                .map(|(id, organisation)| {
                    (
                        id,
                        Condition {
                            organisation,
                            ..Condition::default()
                        },
                    )
                })
                .collect();
            side_scores(
                detail,
                &divisions,
                &conditions,
                [units, &[]],
                ScoreDefines::VANILLA,
            )
            .ok()
            .map(|scores| scores[0])
        };
        // IMUL overflows without failing; the final signed quotient is -1.
        organisation[0] = i64::MAX / 2;
        assert_eq!(score(&detail, organisation, &[1]), Some(-1));
        for value in [i64::MAX, i64::MIN, i64::MIN + 1] {
            organisation[0] = value;
            assert_eq!(score(&detail, organisation, &[1]), Some(4_294_967_295));
        }
        // The participant sum itself wraps to a guarded value.
        organisation[0] = i64::MAX;
        assert_eq!(score(&detail, organisation, &[1, 2]), Some(4_294_967_295));
        organisation[0] = 100_000;
        detail.sides[0].organisation_loss = [0; 24];
        detail.sides[0].organisation_loss[0] = i64::MAX;
        assert_eq!(score(&detail, organisation, &[1]), Some(2));
        detail.sides[0].organisation_loss[1] = 1;
        assert_eq!(score(&detail, organisation, &[1]), Some(2));
        // A non-special loss sum can overflow its product: the resulting
        // nonpositive average gets native's minimum denominator of 100.
        detail.sides[0].organisation_loss[0] = i64::MAX - 1;
        detail.sides[0].organisation_loss[1] = 0;
        assert_eq!(score(&detail, organisation, &[1]), Some(100_000_000));
        assert_eq!(comparison_ratio(i64::MAX / 2, 100), -1_000);
        assert_eq!(comparison_ratio(i64::MAX, 100), 4_294_967_295);
    }

    #[test]
    fn width_replays_direction_grouping_and_totals() {
        const COMBAT: u32 = 362;
        fn row(division: u64, province: u32, group: i32) -> LandCombatDivisionRow {
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division,
                province,
                direction_group: group,
                ..Default::default()
            }
        }
        let still = BTreeMap::new();
        fn count(
            rows: &[LandCombatDivisionRow],
            committed: &[u64],
            movement: &BTreeMap<u64, u32>,
        ) -> Option<usize> {
            let divisions: Divisions<'_> =
                rows.iter().map(|row| ((0, row.division), row)).collect();
            direction_count(committed, &divisions, COMBAT, movement).ok()
        }
        // Danzig proves the latch reading: the game shows 120, and the base
        // term alone is 80, so the count is 2 with one additional-width term
        // of 40. Distinct provinces without latched grouping count twice.
        assert_eq!(
            count(&[row(1, 9263, 0), row(2, 389, 0)], &[1, 2], &still),
            Some(2)
        );
        // Two positive latch words group into one direction: the first arms
        // the latch, the second folds into its direction.
        assert_eq!(
            count(&[row(1, 9263, 5), row(2, 389, 7)], &[1, 2], &still),
            Some(1)
        );
        // A latched unit on an already-seen key does not arm the latch: the
        // next fresh latched key still opens its own direction.
        let rows = [row(1, 9263, 0), row(2, 9263, 5), row(3, 389, 7)];
        assert_eq!(count(&rows, &[1, 2, 3], &still), Some(2));
        // Missing rows leave the count unknown; empty lists count zero.
        assert_eq!(count(&rows, &[1, 9], &still), None);
        assert_eq!(count(&rows, &[], &still), Some(0));
        // A unit inside the combat province keys on its movement source, and
        // one standing there without a source keys on the province itself.
        let entered = [row(1, COMBAT, 0), row(2, 9263, 0)];
        assert_eq!(count(&entered, &[1, 2], &still), None);
        assert_eq!(
            count(&entered, &[1, 2], &BTreeMap::from([(1, 9263)])),
            Some(1)
        );
        assert_eq!(count(&entered, &[1, 2], &BTreeMap::from([(1, 0)])), Some(2));
        // Danzig totals: (0 + 100000) * 8000000 / 100000 = 80 base, plus one
        // 40 support term for the second direction = 120.
        let detail = LandCombatDetailsRow::default();
        let plains = [8_000_000, 4_000_000];
        assert_eq!(total_width(&detail, plains, 1), 8_000_000);
        assert_eq!(total_width(&detail, plains, 2), 12_000_000);
        assert_eq!(total_width(&detail, plains, 0), 8_000_000);
    }
}
