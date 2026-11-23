use super::defines::LandDefines;
use replay_data::{
    rows::LandCombatDetailsRow, rows::LandCombatMember, rows::LandCombatMemberValues,
};
use std::collections::BTreeMap;

const SCALE: i64 = 100_000;

const RECORDED_KINDS: [u32; 8] = [2, 7, 10, 13, 27, 8, 17, 16];

fn guarded(numerator: i64, denominator: i64) -> Option<i64> {
    if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        numerator.wrapping_mul(SCALE).checked_div(denominator)
    } else {
        Some(0xffff_ffff)
    }
}

pub(super) fn pierce_bucket(thresholds: &[i64], armour: i64, piercing: i64) -> Option<usize> {
    let ratio = if armour == 0 {
        10_000_000
    } else {
        guarded(piercing, armour)?
    };
    Some(
        thresholds
            .iter()
            .position(|threshold| *threshold <= ratio)
            .unwrap_or(0),
    )
}

fn fixed_limit(define: f32) -> i64 {
    ((if define >= 0.0 { 1.0_f32 } else { 0.0 } + define * 100_000.0_f32) - 0.5_f32) as i64
}

fn armour_frames(
    content: &LandDefines,
    unit: &LandCombatMemberValues,
    opponents: &[&LandCombatMemberValues],
) -> Option<[Option<u32>; 2]> {
    let thresholds = &content.piercing_thresholds;
    let defines = content.armour_icons;
    let mut ours = vec![0_i32; thresholds.len()];
    let mut theirs = vec![0_i32; thresholds.len()];
    let mut armoured = 0_i32;
    for opponent in opponents {
        if opponent.armour > 0 {
            armoured += 1;
        }
        if !thresholds.is_empty() {
            ours[pierce_bucket(thresholds, opponent.armour, unit.piercing)?] += 1;
            theirs[pierce_bucket(thresholds, unit.armour, opponent.piercing)?] += 1;
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
        guarded(rest, full)?
    };
    let armour = if ratio >= fixed_limit(defines[2]) {
        Some(9)
    } else if ratio >= fixed_limit(defines[3]) {
        Some(8)
    } else {
        None
    };
    Some([piercing, armour])
}

pub(super) fn fuel_ratio([fuel, capacity, define]: [i64; 3]) -> i64 {
    let capacity = capacity.wrapping_mul(define) / SCALE;
    if capacity <= 0 {
        return SCALE;
    }
    guarded(fuel, capacity).map_or(SCALE, |ratio| ratio.clamp(0, SCALE))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Omitted {
    Details,
    Opponent,
    Values,
    ArmourTrap,
    Defines,
    AirDefense(&'static str),
    SideFlags,
    Exile(u32),
}

impl Omitted {
    pub(super) fn reason(self) -> String {
        const FROM_ARMOUR: &str =
            "armour 8 to 11, leader 12, fuel 16, anti-air 13 and the exile icons 14 and 15";
        const FROM_AIR: &str = "the anti-air icon 13 and the exile icons 14 and 15 after it";
        match self {
            Self::Details => format!(
                "the combat has no LandCombatDetails row, so every derived icon after the recorded kinds is omitted: {FROM_ARMOUR}"
            ),
            Self::Opponent => format!(
                "an opposing committed LandCombatDivisions row is missing, so every icon from armour on is omitted: {FROM_ARMOUR}"
            ),
            Self::Values => format!(
                "a committed division's values are in neither its LandCombatDivisions row nor its division's rows, so every icon from armour on is omitted: {FROM_ARMOUR}"
            ),
            Self::ArmourTrap => format!(
                "the game's armour division traps on the recorded armour and piercing, so every icon from armour on is omitted: {FROM_ARMOUR}"
            ),
            Self::Defines => format!(
                "the content's land combat defines are unavailable, so every derived icon after the recorded kinds is omitted: {FROM_ARMOUR}"
            ),
            Self::AirDefense(cause) => format!(
                "the anti-air value cannot be computed ({cause}), so {FROM_AIR} are omitted"
            ),
            Self::SideFlags => format!(
                "the combat has no LandCombatSides row, so the side flag 0x4000 that decides the anti-air icon is unknown and {FROM_AIR} are omitted"
            ),
            Self::Exile(14) => {
                "the exile icon 14 is undecided, so it and the exile icon 15 after it are omitted"
                    .into()
            }
            Self::Exile(frame) => format!("the exile icon {frame} is undecided and omitted"),
        }
    }
}

pub(super) fn frames(
    recorded: impl Iterator<Item = u32>,
    detail: Option<(&LandCombatDetailsRow, usize)>,
    own_flags: Option<u32>,
    unit: LandCombatMember<'_>,
    opponents: Option<&[LandCombatMember<'_>]>,
    content: Option<&LandDefines>,
    exile: [Option<bool>; 2],
) -> (Vec<u32>, Option<Omitted>) {
    let mut out: Vec<u32> = recorded
        .filter_map(|kind| RECORDED_KINDS.iter().position(|k| *k == kind))
        .map(|frame| frame as u32)
        .collect();
    let Some((detail, side)) = detail else {
        return (out, Some(Omitted::Details));
    };
    let Some(opponents) = opponents else {
        return (out, Some(Omitted::Opponent));
    };
    let Some(content) = content else {
        return (out, Some(Omitted::Defines));
    };
    let against: Option<Vec<_>> = opponents.iter().map(|opponent| opponent.values).collect();
    let (Some(values), Some(against)) = (unit.values, against) else {
        return (out, Some(Omitted::Values));
    };
    let Some(armour) = armour_frames(content, values, &against) else {
        return (out, Some(Omitted::ArmourTrap));
    };
    out.extend(armour.into_iter().flatten());
    let own = &detail.sides[side];
    if unit.commander != 0 && unit.commander != own.commander {
        out.push(12);
    }
    let [fuel, capacity] = values.fuel_inputs;
    if fuel_ratio([fuel, capacity, content.fuel_penalty_start_ratio]) < SCALE {
        out.push(16);
    }
    match engine_native_parts::csub_unit_definition::movement_air_defense_value(
        values.air_attack,
        content.air_superiority[0],
        content.air_superiority[1],
    ) {
        Ok(value) if value > 1000 => match own_flags {
            Some(flags) if flags & 0x4000 != 0 => out.push(13),
            Some(_) => (),
            None => return (out, Some(Omitted::SideFlags)),
        },
        Ok(_) => (),
        Err(cause) => return (out, Some(Omitted::AirDefense(cause))),
    }
    for (frame, shown) in [14, 15].into_iter().zip(exile) {
        match shown {
            Some(true) => out.push(frame),
            Some(false) => (),
            None => return (out, Some(Omitted::Exile(frame))),
        }
    }
    (out, None)
}

pub(super) fn omission_warnings(
    combat: u64,
    omitted: &BTreeMap<Omitted, usize>,
) -> impl Iterator<Item = String> + '_ {
    omitted.iter().map(move |(cause, entries)| {
        format!(
            "Land division icons: {entries} committed entries of combat {combat} end early: {}.",
            cause.reason()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLDS: [i64; 4] = [100_000, 75_000, 50_000, 0];

    fn content() -> LandDefines {
        LandDefines {
            armour_icons: [0.6, 0.25, 0.6, 0.25],
            piercing_thresholds: THRESHOLDS.to_vec(),
            air_superiority: [62_500_000, 75_000],
            fuel_penalty_start_ratio: 25_000,
            ..LandDefines::default()
        }
    }

    fn detail() -> LandCombatDetailsRow {
        let mut detail = LandCombatDetailsRow::default();
        detail.sides[1].commander = 1_254_130_455_145;
        detail
    }

    struct Unit {
        row: replay_data::rows::LandCombatDivisionRow,
        values: LandCombatMemberValues,
    }
    impl Unit {
        fn member(&self) -> LandCombatMember<'_> {
            LandCombatMember {
                row: &self.row,
                values: Some(&self.values),
            }
        }
    }

    fn unit(armour: i64, piercing: i64) -> Unit {
        Unit {
            row: replay_data::rows::LandCombatDivisionRow {
                side: 1,
                ..Default::default()
            },
            values: LandCombatMemberValues {
                armour,
                piercing,
                ..Default::default()
            },
        }
    }

    fn icons(
        recorded: &[u32],
        detail: Option<&LandCombatDetailsRow>,
        flags: u32,
        unit: &Unit,
        opponents: Option<&[&Unit]>,
        exile: [Option<bool>; 2],
    ) -> (Vec<u32>, Option<Omitted>) {
        let opponents: Option<Vec<_>> =
            opponents.map(|opponents| opponents.iter().map(|unit| unit.member()).collect());
        frames(
            recorded.iter().copied(),
            detail.map(|detail| (detail, 1)),
            Some(flags),
            unit.member(),
            opponents.as_deref(),
            Some(&content()),
            exile,
        )
    }

    #[test]
    fn piercing_buckets_take_the_first_threshold_not_above_the_ratio() {
        assert_eq!(pierce_bucket(&THRESHOLDS, 0, 0), Some(0));
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 1_000_000), Some(0));
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 800_000), Some(1));
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 500_000), Some(2));
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, 499_999), Some(3));
        assert_eq!(pierce_bucket(&THRESHOLDS, 1_000_000, -20_000_000), Some(0));
        assert_eq!(pierce_bucket(&[], 1, 1), Some(0));
        assert_eq!(fixed_limit(0.6), 60_000);
        assert_eq!(fixed_limit(0.25), 25_000);
    }

    #[test]
    fn a_trapping_armour_division_ends_the_list_by_name() {
        assert_eq!(pierce_bucket(&THRESHOLDS, -1, 1 << 58), None);
        let detail = detail();
        let armoured = unit(-1, 0);
        let piercer = unit(0, 1 << 58);
        assert_eq!(
            icons(
                &[2],
                Some(&detail),
                0,
                &armoured,
                Some(&[&piercer]),
                [Some(false); 2]
            ),
            (vec![0], Some(Omitted::ArmourTrap))
        );
    }

    #[test]
    fn armour_frames_follow_the_opposing_committed_array() {
        let content = content();
        let infantry = unit(0, 4_861_568);
        let enemy = unit(0, 481_327);
        assert_eq!(
            armour_frames(&content, &infantry.values, &[&enemy.values, &enemy.values]),
            Some([None, None])
        );
        let tank = unit(2_000_000, 400_000);
        let piercer = unit(0, 400_000);
        assert_eq!(
            armour_frames(&content, &piercer.values, &[&tank.values, &tank.values]),
            Some([None, None])
        );
        let piercer = unit(0, 1_200_000);
        assert_eq!(
            armour_frames(&content, &piercer.values, &[&tank.values, &tank.values, &enemy.values]).unwrap()[0],
            Some(11)
        );
        let partial = unit(0, 1_000_000);
        let heavy = unit(10_000_000, 0);
        assert_eq!(
            armour_frames(&content, &partial.values, &[&tank.values, &heavy.values]).unwrap()[0],
            Some(10)
        );
        let armoured = unit(5_000_000, 0);
        assert_eq!(armour_frames(&content, &armoured.values, &[&enemy.values]).unwrap()[1], Some(9));
        let medium = unit(1_000_000, 0);
        let pierce_three_quarters = unit(0, 800_000);
        assert_eq!(
            armour_frames(&content, &medium.values, &[&pierce_three_quarters.values]).unwrap()[1],
            Some(8)
        );
        assert_eq!(
            armour_frames(&content, &infantry.values, &[]),
            Some([None, Some(9)])
        );
    }

    #[test]
    fn fuel_air_and_leader_frames_use_the_native_gates() {
        assert_eq!(fuel_ratio([0, 0, 25_000]), SCALE);
        assert_eq!(fuel_ratio([100_000, 1_000_000, 25_000]), 40_000);
        assert_eq!(fuel_ratio([1_000_000, 1_000_000, 25_000]), SCALE);
        assert_eq!(fuel_ratio([-5, 1_000_000, 25_000]), 0);
        let detail = detail();
        let enemy = unit(0, 0);
        let decided = [Some(false); 2];
        let mut madison = unit(0, 4_861_568);
        madison.row.commander = 4_140_348_478_057;
        madison.values.air_attack = 2_127_473;
        assert_eq!(
            icons(
                &[],
                Some(&detail),
                0x80200C9,
                &madison,
                Some(&[&enemy]),
                decided
            ),
            (vec![12], None)
        );
        madison.values.fuel_inputs = [100_000, 1_000_000];
        assert_eq!(
            icons(
                &[27, 3],
                Some(&detail),
                0x4000,
                &madison,
                Some(&[&enemy]),
                decided
            ),
            (vec![4, 12, 16, 13], None)
        );
        assert_eq!(
            icons(&[2], None, 0x4000, &madison, Some(&[&enemy]), decided),
            (vec![0], Some(Omitted::Details))
        );
        assert_eq!(
            icons(&[2], Some(&detail), 0x4000, &madison, None, decided),
            (vec![0], Some(Omitted::Opponent))
        );
        assert_eq!(
            frames(
                [2].into_iter(),
                Some((&detail, 1)),
                Some(0x4000),
                madison.member(),
                Some(&[enemy.member()]),
                None,
                decided
            ),
            (vec![0], Some(Omitted::Defines))
        );
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                None,
                madison.member(),
                Some(&[enemy.member()]),
                Some(&content()),
                [Some(true); 2]
            ),
            (vec![12, 16], Some(Omitted::SideFlags))
        );
        madison.values.air_attack = 158_327;
        madison.row.commander = detail.sides[1].commander;
        assert_eq!(
            icons(
                &[],
                Some(&detail),
                0x4000,
                &madison,
                Some(&[&enemy]),
                decided
            ),
            (vec![16], None)
        );
        assert_eq!(
            frames(
                [].into_iter(),
                Some((&detail, 1)),
                None,
                madison.member(),
                Some(&[enemy.member()]),
                Some(&content()),
                [Some(true); 2]
            ),
            (vec![16, 14, 15], None)
        );
        for (exile, expected) in [
            ([Some(true), Some(false)], (vec![16, 14], None)),
            ([Some(false), Some(true)], (vec![16, 15], None)),
            ([Some(true), Some(true)], (vec![16, 14, 15], None)),
            ([None, Some(true)], (vec![16], Some(Omitted::Exile(14)))),
            ([Some(true), None], (vec![16, 14], Some(Omitted::Exile(15)))),
        ] {
            assert_eq!(
                icons(&[], Some(&detail), 0x4000, &madison, Some(&[&enemy]), exile),
                expected
            );
        }
    }

    #[test]
    fn omissions_are_counted_per_cause_and_name_the_omitted_suffix() {
        let omitted = BTreeMap::from([
            (Omitted::SideFlags, 3),
            (Omitted::Opponent, 1),
            (Omitted::Exile(14), 2),
        ]);
        let lines: Vec<_> = omission_warnings(62, &omitted).collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with(
            "Land division icons: 1 committed entries of combat 62 end early: an opposing committed LandCombatDivisions row is missing"
        ));
        assert!(lines[1].starts_with(
            "Land division icons: 3 committed entries of combat 62 end early: the combat has no LandCombatSides row"
        ));
        assert!(lines[2].starts_with(
            "Land division icons: 2 committed entries of combat 62 end early: the exile icon 14 is undecided"
        ));
        assert!(lines.iter().all(|line| line.contains("exile icon")));
    }
}
