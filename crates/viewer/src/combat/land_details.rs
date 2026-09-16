//! Derived land-window operands from the pinned native binders.
use super::binding::{Bindings, hide, text};
use crate::{document::Document, map::tables::rows};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDetailsRow, LandCombatDivisionRow, LandDivisionContext, tables},
};
use std::{collections::BTreeMap, io};

type Divisions<'a> = BTreeMap<(u8, u64), &'a LandCombatDivisionRow>;

/// Hash-covered NMilitary definitions consumed by 1412A26B0. The native
/// estimate uses the named avoid chances directly, not their complements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScoreDefines {
    pub avoid_with_defence: i64,
    pub avoid_without_defence: i64,
    pub organisation_die: i32,
}

impl ScoreDefines {
    /// Read once when combat assets load. This bounded reader accepts the
    /// shipped NMilitary block and direct literal overrides. Scripted Lua is
    /// deliberately unavailable rather than evaluated with guessed semantics.
    pub(crate) fn load(layers: &installed_content::ResolvedLayers) -> io::Result<Option<Self>> {
        let mut values = [None; 3];
        let mut paths: Vec<_> = layers
            .paths()
            .filter(|path| path.starts_with("common/defines/") && path.ends_with(".lua"))
            .collect();
        paths.sort_unstable();
        for path in paths {
            let source = layers.read(path)?;
            let Ok(source) = std::str::from_utf8(&source) else {
                return Ok(None);
            };
            if !score_assignments(source, path.ends_with("/00_defines.lua"), &mut values) {
                return Ok(None);
            }
        }
        Ok(match values {
            [Some(a), Some(b), Some(die)] if die % 100_000 == 0 => i32::try_from(die / 100_000)
                .ok()
                .map(|organisation_die| Self {
                    avoid_with_defence: a,
                    avoid_without_defence: b,
                    organisation_die,
                }),
            _ => None,
        })
    }
}

fn fixed_literal(value: &str) -> Option<i64> {
    let value = value.trim().trim_end_matches([',', ';']).trim();
    let negative = value.starts_with('-');
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|v| v.is_ascii_digit())
        || fraction.len() > 5
        || !fraction.bytes().all(|v| v.is_ascii_digit())
    {
        return None;
    }
    let magnitude = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100_000)?
        .checked_add(if fraction.is_empty() {
            0
        } else {
            fraction
                .parse::<i64>()
                .ok()?
                .checked_mul(10_i64.pow(5 - fraction.len() as u32))?
        })?;
    if negative {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}

fn score_assignments(source: &str, base: bool, values: &mut [Option<i64>; 3]) -> bool {
    const KEYS: [&str; 3] = [
        "BASE_CHANCE_TO_AVOID_HIT",
        "CHANCE_TO_AVOID_HIT_AT_NO_DEF",
        "LAND_COMBAT_ORG_DICE_SIZE",
    ];
    let mut military_depth = 0_i32;
    let touches_military = source.contains("NMilitary");
    for line in source.lines() {
        let line = line.split_once("--").map_or(line, |(code, _)| code).trim();
        if touches_military
            && [
                "if ",
                "for ",
                "while ",
                "function ",
                "local ",
                "require(",
                "dofile(",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            return false;
        }
        if base && line == "NMilitary = {" {
            military_depth = 1;
            continue;
        }
        for (index, key) in KEYS.iter().enumerate() {
            if !line.contains(key) {
                continue;
            }
            let qualified = format!("NDefines.NMilitary.{key}");
            let lhs = if base && military_depth == 1 {
                *key
            } else {
                &qualified
            };
            let Some(rhs) = line
                .strip_prefix(lhs)
                .and_then(|rest| rest.trim_start().strip_prefix('='))
            else {
                return false;
            };
            let Some(value) = fixed_literal(rhs) else {
                return false;
            };
            values[index] = Some(value);
        }
        // Replacing or aliasing the whole namespace cannot be represented by
        // this three-value reader. Unrelated graphics/career defines are fine.
        if !base && line.contains("NMilitary") {
            let Some((key, rhs)) = line
                .strip_prefix("NDefines.NMilitary.")
                .and_then(|assignment| assignment.split_once('='))
            else {
                return false;
            };
            let key = key.trim();
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
                || (fixed_literal(rhs).is_none() && !matches!(rhs.trim(), "true" | "false"))
            {
                return false;
            }
        }
        if military_depth > 0 {
            // Definitions before the scalar keys include nested numeric arrays.
            // Quoted text cannot affect the table depth.
            let mut quote = None;
            let mut escape = false;
            for ch in line.chars() {
                if escape {
                    escape = false;
                    continue;
                }
                if quote.is_some() && ch == '\\' {
                    escape = true;
                    continue;
                }
                if Some(ch) == quote {
                    quote = None;
                    continue;
                }
                if quote.is_none() {
                    match ch {
                        '\'' | '"' => quote = Some(ch),
                        '{' => military_depth += 1,
                        '}' => military_depth -= 1,
                        _ => (),
                    }
                }
            }
        }
    }
    true
}

fn fixed_mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}

// 140C781E0 -> 140BECAD0: clamp the HP ratio, truncate to tenths and
// retain a minimum one tenth even for an exhausted division.
fn coarse_strength(row: &LandCombatDivisionRow) -> i64 {
    let fraction =
        (row.strength.wrapping_mul(100_000) / row.max_strength.max(100)).clamp(0, 100_000);
    (fraction.wrapping_mul(1_000_000) / 10_000_000_000).max(1) * 10_000
}

fn early_damage(
    detail: &LandCombatDetailsRow,
    divisions: &Divisions<'_>,
    firing_side: usize,
    defines: ScoreDefines,
) -> Option<i64> {
    let targets = &detail.sides[1 - firing_side].committed;
    let average = |stat: usize| -> Option<i64> {
        if targets.is_empty() {
            return Some(0);
        }
        let sum = targets.iter().try_fold(0_i64, |sum, id| {
            Some(
                sum.wrapping_add(divisions.get(&((1 - firing_side) as u8, *id))?.stat_values[stat]),
            )
        })?;
        Some(sum.wrapping_mul(100_000) / (targets.len() as i64 * 100_000))
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
            let row = divisions.get(&(firing_side as u8, *id))?;
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
            let damage = coarse_strength(row)
                .wrapping_add(bonus)
                .wrapping_mul(i64::from(defines.organisation_die))
                .wrapping_mul(50_000)
                / 100_000;
            Some(total.wrapping_add(fixed_mul(damage, covered.wrapping_add(uncovered))))
        })
}

// 140551820: absent/feature-ineligible modifiers return zero; metadata bit 4
// clamps the getter to [0,1]. Country and contextual leader use the same rule.
fn modifier(context: &LandDivisionContext, leader: bool) -> Option<i64> {
    let value = if leader {
        &context.leader_modifiers[1]
    } else {
        &context.country_modifiers[1]
    };
    let [flags, selector] = context.modifier_metadata[1];
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

fn division_width(row: &LandCombatDivisionRow) -> Option<i64> {
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
        .wrapping_add(modifier(&row.context, false)?)
        .wrapping_add(modifier(&row.context, true)?);
    Some(base.wrapping_mul(factor) / 100_000)
}

/// 141298E00's direction count over the attacker committed divisions in
/// recorded order: one direction per fresh province key, except positive
/// direction-group units after the first, which fold into its direction.
/// A missing division row leaves the count unknown rather than guessed.
fn direction_count(committed: &[u64], divisions: &Divisions<'_>) -> Option<usize> {
    let mut seen: Vec<u32> = Vec::new();
    let mut grouped = false;
    for unit in committed {
        let row = divisions.get(&(0, *unit))?;
        if row.direction_group > 0 && grouped {
            continue;
        }
        if seen.contains(&row.direction) {
            continue;
        }
        if row.direction_group > 0 {
            grouped = true;
        }
        seen.push(row.direction);
    }
    Some(seen.len())
}

/// 141298D50's total from the recorded width operands: the width-factor term
/// plus one additional-width term per direction beyond the first, all fixed5
/// with native truncation.
fn total_width(detail: &LandCombatDetailsRow, directions: usize) -> i64 {
    let extra = directions.saturating_sub(1) as i64;
    let width = detail
        .width_factor
        .wrapping_add(100_000)
        .wrapping_mul(detail.terrain_width)
        / 100_000;
    width.wrapping_add(
        extra
            .wrapping_mul(100_000)
            .wrapping_mul(detail.terrain_additional_width)
            / 100_000,
    )
}

/// Native 1412A2AA0 survival estimates, in fixed5, for the two complete sides.
/// `participants` is the ordered full LandCombats census (not committed only).
/// Early battles use 1412A26B0 and the verified content definition context.
pub(crate) fn side_scores<'a>(
    detail: &LandCombatDetailsRow,
    divisions: impl IntoIterator<Item = &'a LandCombatDivisionRow>,
    participants: [&[u64]; 2],
    defines: Option<ScoreDefines>,
) -> Option<[i64; 2]> {
    let divisions: Divisions<'_> = divisions
        .into_iter()
        .filter(|row| row.combat == detail.combat)
        .map(|row| ((row.side, row.division), row))
        .collect();
    let mut scores = [0; 2];
    for side in 0..2 {
        let state = &detail.sides[side];
        let organisation = participants[side].iter().try_fold(0_i64, |sum, id| {
            sum.checked_add(divisions.get(&(side as u8, *id))?.organisation)
        })?;
        if state.organisation_loss_count < 24 {
            let damage = early_damage(detail, &divisions, 1 - side, defines?)?;
            scores[side] = if damage > 99 {
                organisation.checked_mul(100_000)? / damage
            } else {
                0x0017_4875_6160 // Native no-effective-loss sentinel, not zero.
            };
            continue;
        }
        let loss = state
            .organisation_loss
            .iter()
            .try_fold(0_i64, |sum, value| sum.checked_add(*value))?;
        let average = loss.checked_mul(100_000)? / 2_400_000;
        let average = if average < 1 { 100 } else { average };
        scores[side] = organisation.checked_mul(100_000)? / average;
    }
    Some(scores)
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
        let width = detail.sides[side]
            .committed
            .iter()
            .try_fold(0_i64, |sum, unit| {
                Some(sum.wrapping_add(division_width(divisions.get(&(side as u8, *unit))?)?))
            });
        if let Some(width) = width {
            // 1415C8950 deliberately truncates, unlike the stat formatter.
            text(
                &mut out.values,
                &format!("{prefix}_army_width"),
                (width / 100_000).to_string(),
            );
        }
        let air_key = format!("{prefix}_air_text");
        if detail.sides.iter().all(|state| state.air_counts.is_empty()) {
            hide(&mut out.values, &air_key);
        } else {
            let opponent = &detail.sides[1 - side];
            let template = document.names.localisation.get("LAND_COMBAT_CAS_INFO")
                .unwrap_or("Total shot down: $LOST|R$\n£organization_texticon Damage $ORG|R0$\n£strenght_texticon Damage: $STR|R0$");
            let value = template
                .replace("$LOST|R$", &format!("§R{}§!", opponent.air_kills))
                .replace(
                    "$ORG|R0$",
                    &format!("§R{:.0}§!", opponent.damage[1] as f64 / 100_000.0),
                )
                .replace(
                    "$STR|R0$",
                    &format!("§R{:.0}§!", opponent.damage[0] as f64 / 100_000.0),
                )
                .replace("\\n", "\n");
            text(&mut out.values, &air_key, value);
            out.values.entry(air_key).or_default().hidden = false;
        }
    }
    let combats = rows::<tables::LandCombats>(frame)?;
    if let Some(combat) = combats.rows().iter().find(|row| row.id == id) {
        let directions = direction_count(&detail.sides[0].committed, &divisions);
        // Direction 0 is unrecordable live (province ids start at 1), so an
        // all-zero committed vector is a pre-25 file, not a one-direction
        // battle; replaying it would print a confident wrong base total where
        // the old viewer left an honest dash. An empty committed list counts
        // zero natively and needs no operands.
        let live = detail.sides[0].committed.is_empty()
            || detail.sides[0].committed.iter().any(|unit| {
                divisions
                    .get(&(0, *unit))
                    .is_some_and(|row| row.direction != 0)
            });
        if detail.terrain_additional_width == 0 || directions.is_some_and(|_| live) {
            // 141298D50: the width-factor term plus one additional-width term
            // per direction beyond the first, all fixed5 with native truncation.
            text(
                &mut out.values,
                "combat_width",
                (total_width(detail, directions.unwrap_or(0)) / 100_000).to_string(),
            );
        } else {
            out.warnings.push("Land combat total width: an attacker committed LandCombatDivisions row is missing, so the 141298E00 direction count is unknown and the total stays dash rather than guessing.".into());
        }
        if let Some(scores) = side_scores(
            detail,
            division_rows.rows(),
            [&combat.attackers, &combat.defenders],
            document.icons.combat.score_defines,
        ) {
            let progress = rows::<tables::LandCombatProgress>(frame)?;
            if let (Some(progress), Some(bar), Some(icon)) = (
                progress.rows().iter().find(|row| row.combat == id),
                document.icons.combat.sprites.get("GFX_landcombat_progress"),
                document
                    .icons
                    .combat
                    .sprites
                    .get("GFX_land_battle_heading_indicator_strip"),
            ) {
                let (frame, x) = heading(
                    scores,
                    progress.attacker_progress,
                    bar.size[0],
                    icon.size[0],
                );
                let widget = out.values.entry("combat_heading".into()).or_default();
                widget.hidden = false;
                widget.frame = Some(frame);
                widget.offset = [x, 0.0];
            }
        } else {
            out.warnings.push("Land combat heading: incomplete participant rows or unavailable/unsupported scripted NMilitary score definitions leave the survival estimate unknown.".into());
        }
    }
    out.warnings.push("Format-22 derived land: committed widths apply recorded country/contextual-leader modifiers; CAS damage and kills belong to the opposing side; heading uses the native early damage estimate or 24-loss mean and all participants. UNVERIFIED: derived side-value render parity requires a capture with LandCombatDetails available; reference07 lacks that parent table.".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{LandDivisionModifier, LandDivisionWidthTerm};

    const SCORE_DEFINES: ScoreDefines = ScoreDefines {
        avoid_with_defence: 9_000_000,
        avoid_without_defence: 6_000_000,
        organisation_die: 4,
    };

    #[test]
    fn score_definitions_accept_literals_and_reject_scripted_values() {
        let mut values = [None; 3];
        assert!(score_assignments(
            "NMilitary = {\nNESTED = {\n1, 2\n},\nBASE_CHANCE_TO_AVOID_HIT = 90, -- chance\nCHANCE_TO_AVOID_HIT_AT_NO_DEF = 60,\nLAND_COMBAT_ORG_DICE_SIZE = 4,\n},",
            true,
            &mut values
        ));
        assert_eq!(values, [Some(9_000_000), Some(6_000_000), Some(400_000)]);
        assert!(score_assignments(
            "NDefines.NMilitary.BASE_CHANCE_TO_AVOID_HIT = 85.25",
            false,
            &mut values
        ));
        assert_eq!(values[0], Some(8_525_000));
        assert!(score_assignments(
            "NDefines.NMilitary.UNRELATED_DEFINE = 1.5",
            false,
            &mut values
        ));
        assert_eq!(values[0], Some(8_525_000));
        assert!(!score_assignments(
            "NDefines.NMilitary.BASE_CHANCE_TO_AVOID_HIT = 90 / 2",
            false,
            &mut values
        ));
        assert!(!score_assignments(
            "if condition then\nNDefines.NMilitary.BASE_CHANCE_TO_AVOID_HIT = 90\nend",
            false,
            &mut values
        ));
    }

    #[test]
    fn early_score_uses_opposing_committed_damage_and_all_current_organisation() {
        let mut detail = LandCombatDetailsRow {
            combat: 9,
            ..Default::default()
        };
        detail.sides[0].committed = vec![1];
        detail.sides[1].committed = vec![2];
        let mut divisions = [
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division: 1,
                organisation: 3_000_000,
                stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0],
                scales: [100_000; 4],
                strength: 100_000,
                max_strength: 100_000,
                ..Default::default()
            },
            LandCombatDivisionRow {
                combat: 9,
                side: 1,
                division: 2,
                organisation: 6_000_000,
                stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0],
                scales: [100_000; 4],
                strength: 100_000,
                max_strength: 100_000,
                ..Default::default()
            },
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division: 3,
                organisation: 3_000_000,
                ..Default::default()
            },
        ];
        assert_eq!(
            side_scores(&detail, &divisions, [&[1], &[2]], Some(SCORE_DEFINES)),
            Some([200_000, 333_333])
        );
        assert_eq!(
            side_scores(&detail, &divisions, [&[1, 3], &[2]], Some(SCORE_DEFINES)),
            Some([400_000, 333_333])
        );
        divisions[1].strength = 49_999; // Native tenths truncate to 0.4.
        detail.defender_bonus = 10_000; // Added to coarse strength, not a multiplier.
        assert_eq!(
            side_scores(&detail, &divisions, [&[1], &[2]], Some(SCORE_DEFINES)),
            Some([400_000, 333_333])
        );
        detail.sides[1].committed.clear();
        assert_eq!(
            side_scores(&detail, &divisions, [&[1], &[2]], Some(SCORE_DEFINES)).map(|v| v[0]),
            Some(99_999_900_000)
        );
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
        let divisions = [
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division: 1,
                organisation: 3_000_000,
                ..Default::default()
            },
            LandCombatDivisionRow {
                combat: 9,
                side: 1,
                division: 2,
                organisation: 6_000_000,
                ..Default::default()
            },
        ];
        assert_eq!(side_scores(&detail, &divisions, [&[1], &[2]], None), None);
        for side in &mut detail.sides {
            side.organisation_loss_count = 24;
            side.organisation_loss = [100_000; 24];
        }
        assert_eq!(
            side_scores(&detail, &divisions, [&[1], &[2]], None),
            Some([3_000_000, 6_000_000])
        );
        assert_eq!(
            side_scores(&detail, &divisions, [&[1, 3], &[2]], None),
            None
        );
        detail.sides[0].organisation_loss = [0; 24];
        assert_eq!(
            side_scores(&detail, &divisions, [&[1], &[2]], None),
            Some([3_000_000_000, 6_000_000])
        );
    }

    #[test]
    fn width_replays_direction_grouping_and_totals() {
        fn row(division: u64, direction: u32, group: i32) -> LandCombatDivisionRow {
            LandCombatDivisionRow {
                combat: 9,
                side: 0,
                division,
                direction,
                direction_group: group,
                ..Default::default()
            }
        }
        // Danzig proves the latch reading: the game shows 120, and the base
        // term alone is 80, so the count is 2 with one additional-width term
        // of 40. Distinct provinces without latched grouping count twice.
        let rows = [row(1, 9263, 0), row(2, 389, 0)];
        let divisions: Divisions<'_> = rows.iter().map(|row| ((0, row.division), row)).collect();
        assert_eq!(direction_count(&[1, 2], &divisions), Some(2));
        // Two positive latch words group into one direction: the first arms
        // the latch, the second folds into its direction.
        let rows = [row(1, 9263, 5), row(2, 389, 7)];
        let divisions: Divisions<'_> = rows.iter().map(|row| ((0, row.division), row)).collect();
        assert_eq!(direction_count(&[1, 2], &divisions), Some(1));
        // A latched unit on an already-seen key does not arm the latch: the
        // next fresh latched key still opens its own direction.
        let rows = [row(1, 9263, 0), row(2, 9263, 5), row(3, 389, 7)];
        let divisions: Divisions<'_> = rows.iter().map(|row| ((0, row.division), row)).collect();
        assert_eq!(direction_count(&[1, 2, 3], &divisions), Some(2));
        // Missing rows leave the count unknown; empty lists count zero.
        assert_eq!(direction_count(&[1, 9], &divisions), None);
        assert_eq!(direction_count(&[], &divisions), Some(0));
        // Legacy zeros are not a direction: pre-25 files decode every operand
        // as zero, which the binder must gate back to dash (see bind()).
        let legacy = [row(1, 0, 0), row(2, 0, 0)];
        let divisions: Divisions<'_> = legacy.iter().map(|row| ((0, row.division), row)).collect();
        assert_eq!(direction_count(&[1, 2], &divisions), Some(1));
        // Danzig totals: (0 + 100000) * 8000000 / 100000 = 80 base, plus one
        // 40 additional-width term for the second direction = 120.
        let detail = LandCombatDetailsRow {
            width_factor: 0,
            terrain_width: 8_000_000,
            terrain_additional_width: 4_000_000,
            ..Default::default()
        };
        assert_eq!(total_width(&detail, 1), 8_000_000);
        assert_eq!(total_width(&detail, 2), 12_000_000);
        assert_eq!(total_width(&detail, 0), 8_000_000);
    }
}
