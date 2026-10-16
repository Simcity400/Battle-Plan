//! A branch's newest land battle window answers before forward preparation
//! has accepted any block, as the recording product answers it, and forward
//! preparation later answers it the same.
use super::*;
use crate::products::{
    LoadContext, Product,
    land_battles::{
        LandBattles, LandBattlesManifest, LandBattlesManifestParams, LandBattlesParams,
    },
};
use replay::rows::{LandCombatRow, LandCombatSideRow};
use serde_json::{Value, json};

/// An hour of `branch`: combat 1 is open from hour 11 in branch 1, combat 2
/// from hour 21 in branch 2.
fn land_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    let open: Vec<(u64, i32)> = match branch {
        1 if hour >= 11 => vec![(1, 11)],
        2 if hour >= 21 => vec![(2, 21)],
        _ => Vec::new(),
    };
    *observed.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(
        open.iter()
            .map(|&(id, zero)| LandCombatRow {
                id,
                province: id as u32,
                day: 0,
                duration: hour - zero,
                attackers: vec![id << 32 | 1],
                defenders: vec![id << 32 | 2],
                ..Default::default()
            })
            .collect(),
    ));
    *observed.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(
            open.iter()
                .map(|&(id, _)| LandCombatSideRow {
                    combat: id,
                    ..Default::default()
                })
                .collect(),
        ));
    observed
}

fn query(source: &Source, flag: &AtomicBool, product: &str, params: Value) -> Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    serde_json::from_str::<Value>(
        &crate::prepared::query::query(source, &request, None, flag).unwrap(),
    )
    .unwrap()["data"]
        .clone()
}

#[test]
fn newest_land_window_answers_while_forward_preparation_has_accepted_nothing() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![land_frame(1, 10), land_frame(1, 11)])],
    )
    .unwrap();
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![land_frame(1, 12), land_frame(1, 13)],
        )])
        .unwrap();
    let third = second
        .append(&[continuation_records(
            &second,
            1,
            vec![land_frame(1, 14), land_frame(1, 15)],
        )])
        .unwrap();
    let replay = third
        .append(&[records(2, vec![land_frame(2, 20), land_frame(2, 21)])])
        .unwrap();
    assert_eq!(replay.index.len(), 4);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-land");
    let flag = AtomicBool::new(false);
    // Forward preparation stalls: it is cancelled at its first commit.
    let error = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |_| {
            flag.store(true, Ordering::Release);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    flag.store(false, Ordering::Release);
    assert_eq!(
        unprepared_blocks(&source, None).unwrap(),
        Some(replay.index.len()),
        "forward preparation accepted no block"
    );
    let context = LoadContext { content: None };
    let mut answered = Vec::new();
    for (branch, last) in [("1", 2), ("2", 0)] {
        let manifest = query(
            &source,
            &flag,
            "land-battles-manifest",
            json!({ "branchId": branch }),
        );
        assert_eq!(
            manifest,
            serde_json::to_value(
                LandBattlesManifest::load(
                    &source,
                    &LandBattlesManifestParams {
                        branch_id: branch.into()
                    },
                    &context,
                    &flag
                )
                .unwrap()
            )
            .unwrap(),
            "branch {branch}: the manifest answers from the newest window"
        );
        let params = LandBattlesParams {
            branch_id: branch.into(),
            window: last,
        };
        let window = query(
            &source,
            &flag,
            "land-battles",
            serde_json::to_value(&params).unwrap(),
        );
        assert_eq!(
            window,
            serde_json::to_value(LandBattles::load(&source, &params, &context, &flag).unwrap())
                .unwrap(),
            "branch {branch}: the newest window answers as the recording product"
        );
        answered.push((branch, params, manifest, window));
    }
    assert!(
        query(
            &source,
            &flag,
            "land-battles",
            json!({ "branchId": "1", "window": 0 })
        )
        .is_null(),
        "an earlier window waits for forward preparation"
    );
    run(&source, None, &flag, |_| {}).unwrap();
    for (branch, params, manifest, window) in answered {
        assert_eq!(
            query(
                &source,
                &flag,
                "land-battles-manifest",
                json!({ "branchId": branch })
            ),
            manifest
        );
        assert_eq!(
            query(
                &source,
                &flag,
                "land-battles",
                serde_json::to_value(&params).unwrap()
            ),
            window,
            "branch {branch}: forward preparation answers the newest window the same"
        );
    }
}
