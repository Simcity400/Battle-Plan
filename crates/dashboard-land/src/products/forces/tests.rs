use super::*;

fn part(first: i32, last: i32, starts: bool) -> Part {
    Part { run: 0, key: 7, first, last, slot: 7, id: 70, segment_first: first, starts }
}

#[test]
fn parts_split_a_total_only_where_their_rows_add_up_to_it() {
    let held = [(0, Some(30)), (0, Some(20)), (1, Some(10))];
    assert_eq!(split(Some(60), &held, 2), vec![Some(50), Some(10)]);
    assert_eq!(split(Some(50), &[(0, Some(50)), (1, None)], 2), vec![Some(50), Some(0)]);
    assert_eq!(split(Some(60), &[(0, Some(50)), (1, None)], 2), vec![None, None]);
    assert_eq!(split(Some(40), &held, 2), vec![None, None]);
    assert_eq!(split(None, &held, 2), vec![None, None]);
    assert_eq!(split(Some(0), &[], 3), vec![Some(0), Some(0), Some(0)]);
}

#[test]
fn a_holders_runs_cut_where_its_total_reference_or_a_part_moves_and_join_where_nothing_does() {
    let totals = vec![(0, 9, Some(10)), (10, 19, None), (20, 29, Some(12))];
    let reference = vec![(0, 14, Some(100)), (15, 29, Some(120))];
    let first = vec![(0, 4, Some(10)), (5, 29, Some(4))];
    let second = vec![(0, 4, None), (5, 29, Some(6)), ];
    let owned: Owned<'_> = vec![(0, first.as_slice()), (1, second.as_slice())];
    let mut out = Vec::new();
    part_runs(&part(0, 29, true), 0, 29, &totals, Some(&reference), &owned, 2, &mut out);
    let shape: Vec<_> = out.iter().map(|run| (run.first_hour, run.last_hour, run.starts_segment, run.total, run.values.clone(), run.reference_total)).collect();
    assert_eq!(
        shape,
        vec![
            (0, 4, true, Some(10), vec![Some(10), Some(0)], Some(Some(100))),
            (5, 9, false, Some(10), vec![Some(4), Some(6)], Some(Some(100))),
            (10, 14, false, None, vec![None, None], Some(Some(100))),
            (15, 19, false, None, vec![None, None], Some(Some(120))),
            (20, 29, false, Some(12), vec![None, None], Some(Some(120))),
        ]
    );
    let mut cut = Vec::new();
    part_runs(&part(0, 29, true), 6, 8, &totals, None, &owned, 2, &mut cut);
    assert_eq!(cut, vec![ForceRun { first_hour: 6, last_hour: 8, starts_segment: false, total: Some(10), values: vec![Some(4), Some(6)], reference_total: None }]);
}

#[test]
fn recorded_parts_group_under_their_reply_part_in_key_order() {
    let resolved = BTreeMap::from([
        (3u128, ForcePart { key: "fighter".into(), name: Some("Fighter".into()), unresolved: false }),
        (1u128, ForcePart { key: "cas".into(), name: None, unresolved: false }),
        (2u128, ForcePart { key: "fighter".into(), name: Some("Fighter".into()), unresolved: false }),
    ]);
    let named = Named::group(resolved);
    assert_eq!(named.parts.iter().map(|part| part.key.as_str()).collect::<Vec<_>>(), vec!["cas", "fighter"]);
    assert_eq!((named.of[&1], named.of[&2], named.of[&3]), (0, 1, 1));
}

#[test]
fn force_at_hour_ranks_top_and_withholds_unreconciled_parts() {
    use dashboard_base::products::at_hour::{At, AtHour};
    use dashboard_follow::live::Cells;
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    use replay_data::rows::{AirForcePartRow, AirForceTotalRow, Recorded, TableRows};
    use replay_frame::frame::{DecodedFrame, Metrics, TableView};
    use std::sync::atomic::AtomicBool;

    let mut holders = Holders::default();
    holders.tags.insert("GER".into(), vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 1, id: 10 }, starts_segment: true }]);
    holders.tags.insert("ENG".into(), vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 2, id: 20 }, starts_segment: true }]);
    holders.tags.insert("FRA".into(), vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 3, id: 30 }, starts_segment: true }]);

    let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside test".into() }.into());
    *tables[TableId::AirForceTotals.index()] = TableView::Complete(TableRows::AirForceTotals(vec![
        AirForceTotalRow { slot: 1, roster_planes: Recorded::Value(50), ..AirForceTotalRow::default() },
        AirForceTotalRow { slot: 2, roster_planes: Recorded::Value(80), ..AirForceTotalRow::default() },
        AirForceTotalRow { slot: 3, roster_planes: Recorded::Value(80), ..AirForceTotalRow::default() },
    ]));
    *tables[TableId::AirForceParts.index()] = TableView::Complete(TableRows::AirForceParts(vec![
        AirForcePartRow { slot: 2, ownership: 0, variant: Some(100), planes: Recorded::Value(80) },
        AirForcePartRow { slot: 3, ownership: 0, variant: Some(100), planes: Recorded::Value(60) },
    ]));
    let frame = DecodedFrame { branch: 1, hour: 10, tables, metrics: Metrics::default() };
    let live = Cells::default();
    let cancel = AtomicBool::new(false);
    let ctx = (None, Vec::new());
    let at = At { owner: None, holders: &holders, hour: 10, context: &ctx, cancelled: &cancel, live: &live };
    let params = AirForceAtParams {
        branch_id: "1".into(),
        countries: Vec::new(),
        top: Some(2),
        measure: AirForceMeasure::Planes,
        ownership: AirForceOwnership::Roster,
    };
    let reply = AirForceAt::reply(&frame, &params, &at).unwrap();
    assert_eq!(reply.missing, None);
    assert_eq!(reply.rows.iter().map(|row| (row.tag.as_str(), row.total)).collect::<Vec<_>>(), vec![("ENG", Some(80)), ("FRA", Some(80))]);
    assert_eq!(reply.rows[0].parts_missing, None);
    assert_eq!(reply.rows[0].values, vec![Some(80)]);
    assert!(reply.rows[1].parts_missing.is_some(), "parts that do not add up to the total stay withheld");
    assert_eq!(reply.rows[1].values, vec![None]);

    let truncated_params = AirForceAtParams { countries: vec![Country::from("ENG"), Country::from("GER")], top: None, ..params };
    let mut trunc_tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside test".into() }.into());
    *trunc_tables[TableId::AirForceTotals.index()] = TableView::Truncated {
        rows: TableRows::AirForceTotals(vec![AirForceTotalRow { slot: 2, roster_planes: Recorded::Value(80), ..AirForceTotalRow::default() }]),
        missing: 1,
    };
    *trunc_tables[TableId::AirForceParts.index()] = TableView::NotRecorded { mode: replay_frame::scope::CaptureMode::Map };
    let trunc_frame = DecodedFrame { branch: 1, hour: 10, tables: trunc_tables, metrics: Metrics::default() };
    let trunc_reply = AirForceAt::reply(&trunc_frame, &truncated_params, &at).unwrap();
    assert_eq!(trunc_reply.rows[0].tag, "ENG");
    assert_eq!(trunc_reply.rows[0].total, Some(80), "witnessed country keeps its total from a truncated table");
    assert!(trunc_reply.rows[0].parts_missing.is_some());
    assert_eq!(trunc_reply.rows[1].tag, "GER");
    assert_eq!(trunc_reply.rows[1].total, None);
    assert!(trunc_reply.rows[1].missing.is_some());
}

