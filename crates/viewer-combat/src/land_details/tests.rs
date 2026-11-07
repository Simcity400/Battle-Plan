use super::*;
use replay_data::{rows::LandDivisionModifier, rows::LandDivisionWidthTerm, rows::Recorded};

#[test]
fn border_width_uses_the_recorded_base_and_native_signed_wrapping() {
    let mut detail = LandCombatDetailsRow {
        border_war: Recorded::Value(Some(replay_data::rows::LandBorderWarInputs {
            combat_width: Recorded::Value(8_000_000),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(border_width(&detail), Ok(8_000_000));
    detail.width_factor = 25_000;
    assert_eq!(border_width(&detail), Ok(10_000_000));
    detail
        .border_war
        .require_mut("LandCombatDetails.border_war")
        .unwrap()
        .as_mut()
        .unwrap()
        .combat_width = Recorded::Value(-1);
    assert_eq!(border_width(&detail), Ok(-1));
    detail.width_factor = i64::MAX;
    detail
        .border_war
        .require_mut("LandCombatDetails.border_war")
        .unwrap()
        .as_mut()
        .unwrap()
        .combat_width = Recorded::Value(i64::MIN);
    assert_eq!(border_width(&detail), Ok(-92_233_720_368_547));
    detail
        .border_war
        .require_mut("LandCombatDetails.border_war")
        .unwrap()
        .as_mut()
        .unwrap()
        .combat_width = Recorded::Value(0);
    assert_eq!(border_width(&detail), Ok(0));
    detail
        .border_war
        .require_mut("LandCombatDetails.border_war")
        .unwrap()
        .as_mut()
        .unwrap()
        .combat_width = Recorded::NotRecorded;
    assert_eq!(
        border_width(&detail),
        Err("LandBorderWarInputs.combat_width not recorded".into())
    );
    detail.border_war = Recorded::NotRecorded;
    assert_eq!(
        border_width(&detail),
        Err("LandCombatDetails.border_war not recorded".into())
    );
    detail.border_war = Recorded::Value(None);
    assert!(
        border_width(&detail)
            .unwrap_err()
            .contains("absent for the border combat")
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
fn width_replays_direction_grouping_and_totals() {
    const COMBAT: u32 = 362;
    fn row(division: u64, province: u32, group: i32) -> LandCombatDivisionRow {
        LandCombatDivisionRow {
            combat: 9,
            side: 0,
            division,
            province,
            direction_group: group,
            own: Some(replay_data::rows::LandCombatMemberValues {
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    let still = BTreeMap::new();
    fn count(
        rows: &[LandCombatDivisionRow],
        committed: &[u64],
        movement: &BTreeMap<u64, u32>,
    ) -> Option<usize> {
        let divisions: BTreeMap<(u8, u64), _> =
            rows.iter().map(|row| ((0, row.division), row)).collect();
        direction_count(committed, &divisions, COMBAT, movement).ok()
    }
    // Distinct provinces without latched grouping count twice.
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
    // Base width 80 plus one support term of 40 gives total width 120.
    let detail = LandCombatDetailsRow::default();
    let plains = [8_000_000, 4_000_000];
    assert_eq!(total_width(&detail, plains, 1), 8_000_000);
    assert_eq!(total_width(&detail, plains, 2), 12_000_000);
    assert_eq!(total_width(&detail, plains, 0), 8_000_000);
}

#[test]
fn border_width_badge_refuses_missing_configured_width_instead_of_terrain_totals() {
    let mut detail = LandCombatDetailsRow {
        border_war: Recorded::Value(Some(replay_data::rows::LandBorderWarInputs {
            combat_width: Recorded::NotRecorded,
            ..Default::default()
        })),
        ..Default::default()
    };
    let terrain = [7_000_000, 3_500_000];
    for directions in [1, 2] {
        let expected = if directions == 1 { "70" } else { "105" };
        assert_eq!(
            width_badge(&detail, Some(false), || Ok(total_width(
                &detail, terrain, directions
            ))),
            Ok(expected.into())
        );
        assert_eq!(
            width_badge(&detail, Some(true), || unreachable!()),
            Err("LandBorderWarInputs.combat_width not recorded".into())
        );
    }
    assert!(
        width_badge(&detail, None, || unreachable!())
            .unwrap_err()
            .contains("combat class")
    );
    detail.border_war = Recorded::NotRecorded;
    assert_eq!(
        width_badge(&detail, Some(false), || Ok(7_000_000)),
        Ok("70".into())
    );
    assert_eq!(
        width_badge(&detail, Some(true), || unreachable!()),
        Err("LandCombatDetails.border_war not recorded".into())
    );
    detail.border_war = Recorded::Value(Some(replay_data::rows::LandBorderWarInputs {
        combat_width: Recorded::Value(0),
        ..Default::default()
    }));
    assert_eq!(
        width_badge(&detail, Some(true), || unreachable!()),
        Ok("0".into())
    );
}
