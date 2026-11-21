use super::*;
use replay_data::rows::{
    CountryMarketRow, LendLeaseAgreementRow, MarketContractRuntime, MarketContractTermsRow,
    MarketEquipment, MarketPool, SharedMarketPool, TableId,
};
use replay_frame::{
    KEYFRAME_INTERVAL,
    frame::{FrameDecoder, FrameInput, Metrics},
};
use std::sync::Arc;

const TABLES: [TableId; 3] = [
    TableId::CountryMarket,
    TableId::MarketContractTerms,
    TableId::LendLeaseAgreements,
];

fn state(table: TableId, stage: u8, pool: &SharedMarketPool) -> TableRows {
    match table {
        TableId::CountryMarket => TableRows::CountryMarket(
            (if stage == 0 {
                vec![1, 5000, 5001]
            } else {
                vec![5000, 5001]
            })
            .into_iter()
            .map(|country| CountryMarketRow {
                country,
                inventory: pool.clone(),
                auto_send_market_access: country == 5000 && stage == 1,
                ..Default::default()
            })
            .collect(),
        ),
        TableId::MarketContractTerms => TableRows::MarketContractTerms(
            (if stage == 0 {
                vec![(5000, 17), (5000, 18), (5001, 99)]
            } else {
                vec![(5000, 18), (5000, 19), (5001, 99)]
            })
            .into_iter()
            .map(|(country, id)| MarketContractTermsRow {
                country,
                id,
                kind: 1,
                equipment: pool.clone(),
                prices: pool.clone(),
                runtime: MarketContractRuntime {
                    route_equipment: pool.clone(),
                    period_days: if id == 19 { i32::from(stage) } else { 0 },
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
        ),
        TableId::LendLeaseAgreements => TableRows::LendLeaseAgreements(
            (if stage == 0 {
                vec![(5000, 17), (5000, 18), (5001, 99)]
            } else {
                vec![(5000, 18), (5000, 19), (5001, 99)]
            })
            .into_iter()
            .map(|(country, id)| LendLeaseAgreementRow {
                country,
                id,
                monthly: pool.clone(),
                delivered: pool.clone(),
                fuel_sent_raw: if id == 19 { i64::from(stage) } else { 0 },
                ..Default::default()
            })
            .collect(),
        ),
        _ => unreachable!(),
    }
}

fn selected(
    projection: Projection<'_>,
    table: TableId,
    stage: u8,
    pool: &SharedMarketPool,
) -> TableInput {
    if stage == 0 {
        return if table == TableId::CountryMarket {
            projection.certified_owners(TableRows::empty(table), Vec::new(), false)
        } else {
            projection.certified_keys(TableRows::empty(table), Vec::new(), false)
        };
    }
    match state(table, stage, pool) {
        TableRows::CountryMarket(mut rows) => {
            rows.retain(|row| row.country == 5000);
            projection.certified_owners(
                TableRows::CountryMarket(rows),
                if stage == 1 {
                    vec![1, 5000]
                } else {
                    vec![5000]
                },
                false,
            )
        }
        TableRows::MarketContractTerms(mut rows) => {
            rows.retain(|row| row.country == 5000 && row.id == 19);
            let keys = (if stage == 1 {
                vec![17_u128, 19]
            } else {
                vec![19]
            })
            .into_iter()
            .map(|id| (5000_u128 << 72) | (id << 8) | 1)
            .collect();
            projection.certified_keys(TableRows::MarketContractTerms(rows), keys, false)
        }
        TableRows::LendLeaseAgreements(mut rows) => {
            rows.retain(|row| row.country == 5000 && row.id == 19);
            let keys = (if stage == 1 {
                vec![17_u128, 19]
            } else {
                vec![19]
            })
            .into_iter()
            .map(|id| (5000_u128 << 64) | id)
            .collect();
            projection.certified_keys(TableRows::LendLeaseAgreements(rows), keys, false)
        }
        _ => unreachable!(),
    }
}

fn input(hour: Hour, table: TableId, value: TableInput) -> FrameInput {
    let mut frame = FrameInput {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| unavailable("fixture table").into()),
    };
    frame.tables[table.index()] = value;
    frame
}

fn assert_shared_payloads(rows: &TableRows, allocation: *const MarketEquipment) {
    match rows {
        TableRows::CountryMarket(rows) => assert!(
            rows.iter()
                .all(|row| row.inventory.items.as_ptr() == allocation)
        ),
        TableRows::MarketContractTerms(rows) => {
            assert!(
                rows.iter()
                    .all(|row| row.equipment.items.as_ptr() == allocation
                        && row.prices.items.as_ptr() == allocation
                        && row.runtime.route_equipment.items.as_ptr() == allocation)
            )
        }
        TableRows::LendLeaseAgreements(rows) => assert!(
            rows.iter()
                .all(|row| row.monthly.items.as_ptr() == allocation
                    && row.delivered.items.as_ptr() == allocation)
        ),
        _ => unreachable!(),
    }
}

#[test]
fn all_market_state_tables_cross_periodic_keyframes_and_keep_the_next_sparse_update() {
    let pool: SharedMarketPool = MarketPool {
        items: vec![
            MarketEquipment {
                variant: 31,
                amount_raw: 99
            };
            5000
        ],
        ..Default::default()
    }
    .into();
    for table in TABLES {
        for changed in [false, true] {
            let mut encoder = FrameEncoder::new();
            let mut reference = FrameEncoder::new();
            let mut decoder = FrameDecoder::for_tables(&[table]);
            let scratch = ProjectionScratch::default();
            let keyframe_hour = KEYFRAME_INTERVAL as i32;
            for hour in [0, keyframe_hour - 1] {
                let rows = state(table, 0, &pool);
                let frame = encoder
                    .encode_bytes(
                        input(
                            hour,
                            table,
                            TableInput::SharedComplete(Arc::new(rows.clone())),
                        ),
                        false,
                    )
                    .unwrap()
                    .decode()
                    .unwrap();
                reference
                    .encode_bytes(
                        input(hour, table, TableInput::SharedComplete(Arc::new(rows))),
                        false,
                    )
                    .unwrap();
                decoder.apply(&frame).unwrap();
            }
            assert!(
                encoder
                    .slice_baseline(table, 1, keyframe_hour, false)
                    .is_none()
            );
            for (hour, stage) in [
                (keyframe_hour, if changed { 1 } else { 0 }),
                (keyframe_hour + 1, if changed { 2 } else { 0 }),
            ] {
                let value = selected(
                    Projection::new(&encoder, 1, hour, false, &scratch),
                    table,
                    stage,
                    &pool,
                );
                if hour == keyframe_hour {
                    let TableInput::SharedComplete(rows) = &value else {
                        panic!("encoding keyframe lost certified state");
                    };
                    assert_shared_payloads(rows, pool.items.as_ptr());
                    if !changed {
                        assert!(Arc::ptr_eq(rows, encoder.shared_baseline(table).unwrap()));
                    }
                } else {
                    assert!(
                        matches!(value, TableInput::SliceUpdate { .. }),
                        "keyframe dropped the following hour's baseline"
                    );
                }
                let expected = state(table, stage, &pool);
                let frame = encoder
                    .encode_bytes(input(hour, table, value), false)
                    .unwrap()
                    .decode()
                    .unwrap();
                let ordinary = reference
                    .encode_bytes(
                        input(
                            hour,
                            table,
                            TableInput::SharedComplete(Arc::new(expected.clone())),
                        ),
                        false,
                    )
                    .unwrap()
                    .decode()
                    .unwrap();
                assert_eq!(frame, ordinary);
                assert_eq!(frame.keyframe, hour == keyframe_hour);
                let decoded = decoder.apply(&frame).unwrap();
                assert_eq!(
                    &*decoded.tables[table.index()],
                    &TableView::Complete(expected.clone())
                );
                if frame.keyframe {
                    let sought = FrameDecoder::for_tables(&[table]).apply(&frame).unwrap();
                    assert_eq!(
                        &*sought.tables[table.index()],
                        &TableView::Complete(expected)
                    );
                }
            }
        }
    }
}

#[test]
fn certified_keyframe_fallback_rejects_wrong_branch_gaps_and_missing_or_partial_baselines() {
    for table in TABLES {
        let pool = SharedMarketPool::default();
        let mut encoder = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        assert!(matches!(
            selected(
                Projection::new(&encoder, 1, 0, true, &scratch),
                table,
                0,
                &pool
            ),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        encoder
            .encode_bytes(
                input(
                    0,
                    table,
                    TableInput::SharedComplete(Arc::new(state(table, 0, &pool))),
                ),
                false,
            )
            .unwrap();
        assert!(
            encoder
                .certified_keyframe_baseline(table, 1, 1, true)
                .is_some()
        );
        for (branch, hour, force) in [(2, 1, true), (1, 0, true), (1, 2, true), (1, 1, false)] {
            assert!(
                encoder
                    .certified_keyframe_baseline(table, branch, hour, force)
                    .is_none()
            );
        }
        for (branch, hour) in [(2, 1), (1, 0), (1, 2)] {
            assert!(matches!(
                selected(
                    Projection::new(&encoder, branch, hour, true, &scratch),
                    table,
                    0,
                    &pool
                ),
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
        for partial in [false, true] {
            let value = if partial {
                TableView::Truncated {
                    rows: state(table, 0, &pool),
                    missing: 1,
                }
            } else {
                unavailable("synthetic baseline loss")
            };
            encoder
                .encode_bytes(
                    input(if partial { 2 } else { 1 }, table, value.into()),
                    false,
                )
                .unwrap();
            assert!(matches!(
                selected(
                    Projection::new(&encoder, 1, if partial { 3 } else { 2 }, true, &scratch),
                    table,
                    0,
                    &pool
                ),
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
    }
}
