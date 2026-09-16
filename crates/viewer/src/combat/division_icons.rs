//! A committed land combat entry's modifier icons (1415CA660), frames of
//! `GFX_battle_div_modifier_strip`, in the order the native grid receives them:
//!
//! 1. frames 0-7 for the unit's recorded modifier kinds 2, 7, 10, 13, 27, 8,
//!    17 and 16 (`CUnit+0x170`, stride 24), in recorded order;
//! 2. frame 11 or 10, our piercing against the opposing committed armour;
//! 3. frame 9 or 8, the opposing committed piercing against our armour;
//! 4. frame 12, the division's own leader is not the side's selected commander;
//! 5. frame 16, fuel below full (140C6F960);
//! 6. frame 13, anti-air above 0.01 (141297FD0) on a side with flag 0x4000;
//! 7. frame 14, an opposing committed owner in our template country's set;
//! 8. frame 15, our template country in the location province's `+0xC0` list.
//!
//! Frames 14 and 15 arrive recorded: the collector evaluates 140C79AC0, the
//! 14129FB60/14129FAD0 country-set search and the province `+0xC0` scan live
//! and stores the two bits on the division row. Older formats decode them as
//! zero, so those icons stay absent there exactly as before. The grid takes
//! the frames in this order, so an undecidable step ends the list rather than
//! shifting later icons into its place.
use replay::rows::{LandCombatDetailsRow, LandCombatDivisionRow};

const SCALE: i64 = 100_000;

/// The recorded kinds 1415CA660 turns into frames 0 to 7, in frame order.
const RECORDED_KINDS: [u32; 8] = [2, 7, 10, 13, 27, 8, 17, 16];

/// The native fixed-point division guard: an extreme numerator gives
/// 0xFFFFFFFF instead of dividing.
fn guarded(numerator: i64, denominator: i64) -> i64 {
    if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        numerator.wrapping_mul(SCALE) / denominator
    } else {
        0xffff_ffff
    }
}

/// 141298790: the piercing bucket of `piercing` against `armour`, the first
/// threshold not above the ratio (zero armour is ratio 100), else bucket 0.
fn pierce_bucket(thresholds: &[i64], armour: i64, piercing: i64) -> usize {
    let ratio = if armour == 0 {
        10_000_000
    } else {
        guarded(piercing, armour)
    };
    thresholds
        .iter()
        .position(|threshold| *threshold <= ratio)
        .unwrap_or(0)
}

/// `(i64)(define * 100000 + (define >= 0 ? 0.5 : -0.5))` in single precision,
/// as 1415CA660 turns a float define into a fixed-point limit.
fn fixed_limit(define: f32) -> i64 {
    ((if define >= 0.0 { 1.0_f32 } else { 0.0 } + define * 100_000.0_f32) - 0.5_f32) as i64
}

/// Frames 11/10 and 9/8 from the whole opposing committed array.
fn armour_frames(
    detail: &LandCombatDetailsRow,
    unit: &LandCombatDivisionRow,
    opponents: &[&LandCombatDivisionRow],
) -> [Option<u32>; 2] {
    let thresholds = &detail.piercing_thresholds;
    let defines = detail.armour_icon_defines.map(f32::from_bits);
    let mut ours = vec![0_i32; thresholds.len()];
    let mut theirs = vec![0_i32; thresholds.len()];
    let mut armoured = 0_i32;
    for opponent in opponents {
        if opponent.armour > 0 {
            armoured += 1;
        }
        if !thresholds.is_empty() {
            ours[pierce_bucket(thresholds, opponent.armour, unit.piercing)] += 1;
            theirs[pierce_bucket(thresholds, unit.armour, opponent.piercing)] += 1;
        }
    }
    let piercing = (armoured > 0)
        .then(|| {
            let pierced: i32 = ours[..ours.len().saturating_sub(1)].iter().sum();
            let fraction = pierced as f32 / armoured as f32;
            if fraction >= defines[0] {
                Some(11)
            } else if fraction >= defines[1] {
                Some(10)
            } else {
                None
            }
        })
        .flatten();
    let (full, rest) =
        thresholds
            .iter()
            .zip(&theirs)
            .fold((0_i64, 0_i64), |(full, rest), (threshold, count)| {
                let count = i64::from(*count);
                (
                    full.wrapping_add(threshold.wrapping_mul(count).wrapping_mul(SCALE) / SCALE),
                    rest.wrapping_add(
                        (SCALE.wrapping_sub(*threshold))
                            .wrapping_mul(count)
                            .wrapping_mul(SCALE)
                            / SCALE,
                    ),
                )
            });
    let ratio = if full == 0 {
        SCALE
    } else {
        guarded(rest, full)
    };
    let armour = if ratio >= fixed_limit(defines[2]) {
        Some(9)
    } else if ratio >= fixed_limit(defines[3]) {
        Some(8)
    } else {
        None
    };
    [piercing, armour]
}

/// 140C6F960: fuel over `capacity * define`, clamped to [0, 1]; full when the
/// scaled capacity is not positive or the fuel is extreme.
fn fuel_ratio([fuel, capacity, define]: [i64; 3]) -> i64 {
    let capacity = capacity.wrapping_mul(define) / SCALE;
    if capacity <= 0 {
        return SCALE;
    }
    guarded(fuel, capacity).clamp(0, SCALE)
}

/// Every frame the recording decides for one committed entry, in grid order.
/// `opponents` is `None` when an opposing committed row is missing, which
/// leaves the armour frames and everything after them undecided.
pub(super) fn frames(
    recorded: impl Iterator<Item = u32>,
    detail: Option<(&LandCombatDetailsRow, usize)>,
    unit: &LandCombatDivisionRow,
    opponents: Option<&[&LandCombatDivisionRow]>,
) -> Vec<u32> {
    let mut out: Vec<u32> = recorded
        .filter_map(|kind| RECORDED_KINDS.iter().position(|k| *k == kind))
        .map(|frame| frame as u32)
        .collect();
    let (Some((detail, side)), Some(opponents)) = (detail, opponents) else {
        return out;
    };
    out.extend(armour_frames(detail, unit, opponents).into_iter().flatten());
    let own = &detail.sides[side];
    if unit.commander != 0 && unit.commander != own.commander {
        out.push(12);
    }
    if fuel_ratio(unit.fuel_inputs) < SCALE {
        out.push(16);
    }
    match engine::csub_unit_definition::movement_air_defense_value(
        unit.air_attack,
        detail.air_defines[3],
        detail.air_defines[4],
    ) {
        Ok(value) if value > 1000 && own.flags & 0x4000 != 0 => out.push(13),
        Ok(_) => (),
        Err(_) => return out,
    }
    // Frames 14 and 15 are recorded bits (see the module docs), pushed
    // independently exactly as 1415CA660 pushes them; a zero bit is a
    // decided absence, never an undecidable step.
    if unit.template_frames & 1 != 0 {
        out.push(14);
    }
    if unit.template_frames & 2 != 0 {
        out.push(15);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLDS: [i64; 4] = [100_000, 75_000, 50_000, 0];
    // The pinned defines recorded at pairL: 0.6, 0.25, 0.6, 0.25.
    const DEFINES: [u32; 4] = [1_058_642_330, 1_048_576_000, 1_058_642_330, 1_048_576_000];

    fn detail() -> LandCombatDetailsRow {
        let mut detail = LandCombatDetailsRow {
            piercing_thresholds: THRESHOLDS.to_vec(),
            armour_icon_defines: DEFINES,
            air_defines: [300_000, 250_000, 75_000, 62_500_000, 75_000],
            ..Default::default()
        };
        detail.sides[1].commander = 1_254_130_455_145;
        detail
    }

    fn unit(armour: i64, piercing: i64) -> LandCombatDivisionRow {
        LandCombatDivisionRow {
            armour,
            piercing,
            side: 1,
            fuel_inputs: [0, 0, 25_000],
            ..Default::default()
        }
    }

    #[test]
    fn piercing_buckets_take_the_first_threshold_not_above_the_ratio() {
        assert_eq!(pierce_bucket(&THRESHOLDS, 0, 0), 0);
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 1_000_000), 0);
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 800_000), 1);
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 500_000), 2);
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 499_999), 3);
        // A negative ratio is below every threshold and falls back to bucket 0.
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, -20_000_000), 0);
        assert_eq!(pierce_bucket(&[], 1, 1), 0);
        assert_eq!(fixed_limit(0.6), 60_000);
        assert_eq!(fixed_limit(0.25), 25_000);
    }

    #[test]
    fn armour_frames_follow_the_opposing_committed_array() {
        let detail = detail();
        // pairL: no armour on either side gives no armour icon.
        let infantry = unit(0, 4_861_568);
        let enemy = unit(0, 481_327);
        assert_eq!(
            armour_frames(&detail, &infantry, &[&enemy, &enemy]),
            [None, None]
        );
        // Piercing 0.2 of two tanks' armour: nothing pierced to half.
        let tank = unit(2_000_000, 400_000);
        let piercer = unit(0, 400_000);
        assert_eq!(
            armour_frames(&detail, &piercer, &[&tank, &tank]),
            [None, None]
        );
        // 0.6 of both tanks, and an unarmoured opponent that counts as pierced
        // without counting as armoured: 3 / 2 >= 0.6.
        let piercer = unit(0, 1_200_000);
        assert_eq!(
            armour_frames(&detail, &piercer, &[&tank, &tank, &enemy])[0],
            Some(11)
        );
        let partial = unit(0, 1_000_000);
        let heavy = unit(10_000_000, 0);
        // One of two armoured opponents pierced to at least half: 0.5 >= 0.25.
        assert_eq!(
            armour_frames(&detail, &partial, &[&tank, &heavy])[0],
            Some(10)
        );
        // Our armour against piercing under half: no fully pierced weight, so
        // the ratio defaults to one and the gold shield shows.
        let armoured = unit(5_000_000, 0);
        assert_eq!(armour_frames(&detail, &armoured, &[&enemy])[1], Some(9));
        // The 0.75 bucket: (1 - 0.75) / 0.75 = 0.333, the grey shield.
        let medium = unit(1_000_000, 0);
        let pierce_three_quarters = unit(0, 800_000);
        assert_eq!(
            armour_frames(&detail, &medium, &[&pierce_three_quarters])[1],
            Some(8)
        );
        // No opposing committed units: the ratio defaults to one.
        assert_eq!(armour_frames(&detail, &infantry, &[]), [None, Some(9)]);
    }

    #[test]
    fn fuel_air_and_leader_frames_use_the_native_gates() {
        assert_eq!(fuel_ratio([0, 0, 25_000]), SCALE);
        assert_eq!(fuel_ratio([100_000, 1_000_000, 25_000]), 40_000);
        assert_eq!(fuel_ratio([1_000_000, 1_000_000, 25_000]), SCALE);
        assert_eq!(fuel_ratio([-5, 1_000_000, 25_000]), 0);
        let mut detail = detail();
        let enemy = unit(0, 0);
        // pairL 169th: its own leader, anti-air 21.27 (0.10176 > 0.01), side
        // flags 0x80200C9 without 0x4000: only the general.
        let mut madison = unit(0, 4_861_568);
        madison.commander = 4_140_348_478_057;
        madison.air_attack = 2_127_473;
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                &madison,
                Some(&[&enemy])
            ),
            [12]
        );
        detail.sides[1].flags = 0x4000;
        madison.fuel_inputs = [100_000, 1_000_000, 25_000];
        assert_eq!(
            frames(
                [27, 3].into_iter(),
                Some((&detail, 1)),
                &madison,
                Some(&[&enemy])
            ),
            [4, 12, 16, 13]
        );
        // Without the details row or an opposing row only recorded kinds remain.
        assert_eq!(
            frames([2].into_iter(), None, &madison, Some(&[&enemy])),
            [0]
        );
        assert_eq!(
            frames([2].into_iter(), Some((&detail, 1)), &madison, None),
            [0]
        );
        madison.air_attack = 158_327; // 0.00237 after 141297FD0: below 1000.
        madison.commander = detail.sides[1].commander;
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                &madison,
                Some(&[&enemy])
            ),
            [16]
        );
        // Recorded template bits append frames 14 and 15 independently.
        madison.template_frames = 1;
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                &madison,
                Some(&[&enemy])
            ),
            [16, 14]
        );
        madison.template_frames = 2;
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                &madison,
                Some(&[&enemy])
            ),
            [16, 15]
        );
        madison.template_frames = 3;
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                &madison,
                Some(&[&enemy])
            ),
            [16, 14, 15]
        );
    }
}
