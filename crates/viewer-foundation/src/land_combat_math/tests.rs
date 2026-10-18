use super::*;

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
            stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0]
                .map(replay_data::rows::Recorded::Value),
            scales: [100_000; 4].map(replay_data::rows::Recorded::Value),
            ..Default::default()
        },
        LandCombatDivisionRow {
            combat: 9,
            side: 1,
            division: 2,
            stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0]
                .map(replay_data::rows::Recorded::Value),
            scales: [100_000; 4].map(replay_data::rows::Recorded::Value),
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
    use replay_data::{rows::DivisionRow, rows::DivisionStatusRow, rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};
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

/// the game sums current organisation only, so a division without its
/// DivisionStatus row still scores; only the early damage estimate for
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
        stat_values: [10_000_000, 5_000_000, 0, 10_000_000, 0]
            .map(replay_data::rows::Recorded::Value),
        scales: [100_000; 4].map(replay_data::rows::Recorded::Value),
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
        side.organisation_loss_sum = 24 * 100_000;
    }
    assert_eq!(scores(&late, &conditions), Ok([6_000_000, 6_000_000]));
    // Without its Divisions row a participant has no organisation.
    let without = recorded(&[(1, 100_000, 3_000_000), (2, 100_000, 6_000_000)], &full);
    assert_eq!(scores(&late, &without), Err(ScoreGap::Division(3)));

    // a zero maximum or an out-of-range current HP reads as
    // full strength; a negative ratio clamps to zero, which the game
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
        side.organisation_loss_sum = 24 * 100_000;
    }
    assert_eq!(scores(&detail, &[1]), Ok([3_000_000, 6_000_000]));
    assert_eq!(scores(&detail, &[1, 3]), Err(ScoreGap::Division(3)));
    detail.sides[0].organisation_loss_sum = 0;
    assert_eq!(scores(&detail, &[1]), Ok([3_000_000_000, 6_000_000]));
    // Sums and products wrap as qwords do, and a sum at the
    // fixed-point range edge scores 0xFFFFFFFF instead of failing.
    detail.sides[0].organisation_loss_sum = i64::MAX;
    assert_eq!(
        scores(&detail, &[1]).map(|v| v[0]),
        Ok(300_000_000_000 / 0xFFFF_FFFF)
    );
    detail.sides[0].organisation_loss_sum = i64::MAX / 50_000;
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
        side.organisation_loss_sum = 24 * 100_000;
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
    detail.sides[0].organisation_loss_sum = i64::MAX;
    assert_eq!(score(&detail, organisation, &[1]), Some(2));
    // One more sample wraps the sum to i64::MIN, another special value.
    detail.sides[0].organisation_loss_sum = replay_data::rows::LandCombatSideDetails::loss_sum(&{
        let mut samples = [0; 24];
        samples[0] = i64::MAX;
        samples[1] = 1;
        samples
    });
    assert_eq!(detail.sides[0].organisation_loss_sum, i64::MIN);
    assert_eq!(score(&detail, organisation, &[1]), Some(2));
    // A non-special loss sum can overflow its product: the resulting
    // nonpositive average gets native's minimum denominator of 100.
    detail.sides[0].organisation_loss_sum = i64::MAX - 1;
    assert_eq!(score(&detail, organisation, &[1]), Some(100_000_000));
    assert_eq!(comparison_ratio(i64::MAX / 2, 100), -1_000);
    assert_eq!(comparison_ratio(i64::MAX, 100), 4_294_967_295);
}
