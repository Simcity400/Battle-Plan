//! Recorded land combat condition, modifier and survival-score arithmetic.
use crate::map::tables::rows;
use replay_data::{
    rows::LandCombatDetailsRow, rows::LandCombatDivisionRow, rows::LandCombatMember,
    rows::LandCombatMemberValues, rows::LandCombatSideDetails, rows::LandDivisionContext, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub type Divisions<'a> = BTreeMap<(u8, u64), LandCombatMember<'a>>;

/// The rows of `combat`'s members, which sort together.
pub fn combat_rows(rows: &[LandCombatDivisionRow], combat: u64) -> &[LandCombatDivisionRow] {
    let start = rows.partition_point(|row| row.combat < combat);
    let len = rows[start..].partition_point(|row| row.combat == combat);
    &rows[start..start + len]
}

/// Each of `members`' division values, in row order, from the frame's division rows.
pub fn member_values(
    frame: &DecodedFrame,
    members: &[LandCombatDivisionRow],
) -> io::Result<Vec<Option<LandCombatMemberValues>>> {
    Ok(replay_data::rows::land_combat_member_values(
        members,
        rows::<tables::DivisionPanelStats>(frame)?.rows(),
        rows::<tables::DivisionStatus>(frame)?.rows(),
        rows::<tables::DivisionMovement>(frame)?.rows(),
    ))
}

/// A division's current hit points and organisation from `Divisions`, with
/// their maxima from `DivisionStatus` when it has a row this hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Condition {
    pub strength: i64,
    pub max_strength: Option<i64>,
    pub organisation: i64,
    pub max_organisation: Option<i64>,
}

pub type Conditions = BTreeMap<u64, Condition>;

/// Every division with a `Divisions` row this hour. A missing `DivisionPanelStats` row or
/// refused word leaves only the maxima unknown: organisation sum reads current organisation
/// alone.
pub fn conditions(frame: &DecodedFrame) -> io::Result<Conditions> {
    let panels: BTreeMap<_, _> = rows::<tables::DivisionPanelStats>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row))
        .collect();
    Ok(rows::<tables::Divisions>(frame)?
        .rows()
        .iter()
        .map(|row| {
            let panel = panels.get(&row.id);
            (
                row.id,
                Condition {
                    strength: row.hit_points,
                    max_strength: panel.and_then(|panel| panel.maximum_hit_points()),
                    organisation: row.organisation,
                    max_organisation: panel.and_then(|panel| panel.maximum_organisation()),
                },
            )
        })
        .collect())
}

/// Why [`conditions`] holds no entry for a division: which of its two tables
/// lacks the division's row, and whether that table is complete this hour.
pub fn condition_gap(frame: &DecodedFrame) -> io::Result<impl Fn(u64) -> String + '_> {
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
pub struct ScoreDefines {
    pub avoid_with_defence: i64,
    pub avoid_without_defence: i64,
    pub organisation_die: i32,
}

impl ScoreDefines {
    pub fn read(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            avoid_with_defence: defines.fixed5("NMilitary", "BASE_CHANCE_TO_AVOID_HIT")?,
            avoid_without_defence: defines.fixed5("NMilitary", "CHANCE_TO_AVOID_HIT_AT_NO_DEF")?,
            organisation_die: defines.int32("NMilitary", "LAND_COMBAT_ORG_DICE_SIZE")?,
        })
    }

    /// Values exercised by the tests.
    #[cfg(any(test, feature = "test-support"))]
    pub const VANILLA: Self = Self {
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
pub enum ScoreGap {
    /// A division without its `Divisions` row: a participant's current
    /// organisation or a firing committed division's current HP.
    Division(u64),
    /// A firing committed division whose `DivisionPanelStats` row lacks its maximum HP, which
    /// the early path reads.
    MaximumStrength(u64),
    /// A committed division without its `LandCombatDivisions` row.
    Member { side: u8, division: u64 },
    /// A committed division whose stats neither its `LandCombatDivisions` row nor its
    /// division's rows of the hour hold.
    Numbers { side: u8, division: u64 },
}

impl ScoreGap {
    /// What the estimate lacks, naming the division and the table.
    pub fn reason(self) -> String {
        match self {
            Self::Division(division) => format!(
                "division {division} has no Divisions row, whose current organisation and hit points it reads"
            ),
            Self::MaximumStrength(division) => format!(
                "committed division {division} has no maximum hit points in DivisionPanelStats, which the early damage estimate reads"
            ),
            Self::Member { side, division } => format!(
                "committed division {division} on side {side} has no LandCombatDivisions row"
            ),
            Self::Numbers { side, division } => format!(
                "committed division {division} on side {side} has no stats in its LandCombatDivisions row or its division's rows"
            ),
        }
    }
}

/// What the survival estimate reads of a combat's members and divisions.
pub trait ScoreJoins {
    /// A member's stat values (none when neither its row nor its division's rows hold them) and
    /// scales; `None` without its `LandCombatDivisions` row.
    fn member(&self, side: u8, division: u64) -> Option<(Option<[i64; 5]>, [i64; 4])>;
    fn condition(&self, division: u64) -> Option<Condition>;
}

struct RowJoins<'a, 'b> {
    divisions: &'a Divisions<'b>,
    conditions: &'a Conditions,
}

impl ScoreJoins for RowJoins<'_, '_> {
    fn member(&self, side: u8, division: u64) -> Option<(Option<[i64; 5]>, [i64; 4])> {
        self.divisions.get(&(side, division)).map(|member| (member.values.map(|values| values.stat_values), member.scales))
    }

    fn condition(&self, division: u64) -> Option<Condition> {
        self.conditions.get(&division).copied()
    }
}

/// A combat's detail operands the estimate reads.
#[derive(Clone, Copy)]
pub struct ScoreDetail<'a> {
    pub attacker_bonus: i64,
    pub defender_bonus: i64,
    pub sides: &'a [LandCombatSideDetails; 2],
}

fn early_damage(
    detail: ScoreDetail<'_>,
    joins: &impl ScoreJoins,
    firing_side: usize,
    defines: ScoreDefines,
) -> Result<i64, ScoreGap> {
    let member = |side: usize, id: u64| {
        joins.member(side as u8, id).ok_or(ScoreGap::Member {
            side: side as u8,
            division: id,
        })
    };
    let numbers = |side: usize, id: u64| -> Result<([i64; 5], [i64; 4]), ScoreGap> {
        let (values, scales) = member(side, id)?;
        values
            .map(|values| (values, scales))
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
            let condition = joins.condition(*id).ok_or(ScoreGap::Division(*id))?;
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
pub fn modifier(context: &LandDivisionContext, key: usize, leader: bool) -> Option<i64> {
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

/// Survival estimates in fixed5 for the two complete sides.
/// `participants` is the ordered full LandCombats census (not committed only).
/// Early battles use damage estimates and the verified content definition context.
pub fn side_scores<'a>(
    detail: &LandCombatDetailsRow,
    divisions: impl IntoIterator<Item = LandCombatMember<'a>>,
    conditions: &Conditions,
    participants: [&[u64]; 2],
    defines: ScoreDefines,
) -> Result<[i64; 2], ScoreGap> {
    let divisions: Divisions<'_> = divisions
        .into_iter()
        .filter(|row| row.combat == detail.combat)
        .map(|row| ((row.side, row.division), row))
        .collect();
    side_scores_of(
        ScoreDetail { attacker_bonus: detail.attacker_bonus, defender_bonus: detail.defender_bonus, sides: &detail.sides },
        &RowJoins { divisions: &divisions, conditions },
        participants,
        defines,
    )
}

/// [`side_scores`] from values: the detail's operands and the combat's members and divisions.
pub fn side_scores_of(
    detail: ScoreDetail<'_>,
    joins: &impl ScoreJoins,
    participants: [&[u64]; 2],
    defines: ScoreDefines,
) -> Result<[i64; 2], ScoreGap> {
    let mut scores = [0; 2];
    for side in 0..2 {
        let state = &detail.sides[side];
        // Use wrapping 64-bit sums and products over current organisation,
        // never maximum organisation.
        let organisation = participants[side].iter().try_fold(0_i64, |sum, id| {
            Ok(sum.wrapping_add(
                joins
                    .condition(*id)
                    .ok_or(ScoreGap::Division(*id))?
                    .organisation,
            ))
        })?;
        if state.organisation_loss_count < 24 {
            let damage = early_damage(detail, joins, 1 - side, defines)?;
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

#[cfg(test)]
mod tests;
