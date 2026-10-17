use super::*;
use std::cell::Cell;

#[test]
fn carried_reductions_visit_no_rows_and_keep_each_supplying_date() {
    let source = crate::test_fixtures::scale_frame(1, 100, 32);
    let visits = Cell::new(0);
    let project = |frame: &DecodedFrame| {
        frame.complete::<tables::Countries>().ok().map(|rows| {
            rows.iter()
                .map(|row| {
                    visits.set(visits.get() + 1);
                    CountryRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                        tag: row.tag.clone(),
                        ..Default::default()
                    }
                })
                .collect()
        })
    };
    let mut reduced = Reduced::new(TableId::Countries);
    let mut first = source.clone();
    reduced.reduce(&mut first, project, TableRows::Countries);
    assert_eq!(visits.get(), 32);
    for hour in 101..111 {
        let mut carried = source.clone();
        carried.hour = hour;
        carried.tables[TableId::Countries.index()] = carried.tables[TableId::Countries.index()]
            .clone()
            .with_sampled_at(Some(hour));
        reduced.reduce(&mut carried, project, TableRows::Countries);
        assert_eq!(visits.get(), 32, "carried rows were reduced again");
        assert!(SharedView::ptr_eq(
            &first.tables[TableId::Countries.index()],
            &carried.tables[TableId::Countries.index()]
        ));
        assert_eq!(
            carried.tables[TableId::Countries.index()].sampled_at(),
            Some(hour)
        );
    }
    let mut changed = source.clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"changed holder".to_vec();
    }
    reduced.reduce(&mut changed, project, TableRows::Countries);
    assert_eq!(visits.get(), 64);
    assert_eq!(
        changed.complete::<tables::Countries>().unwrap()[0].tag,
        b"changed holder"
    );
    assert!(!SharedView::ptr_eq(
        &first.tables[TableId::Countries.index()],
        &changed.tables[TableId::Countries.index()]
    ));

    for view in [
        TableView::Unavailable {
            reason: "synthetic missing countries".into(),
        },
        TableView::Truncated {
            rows: TableRows::Countries(vec![CountryRow::default()]),
            missing: 7,
        },
    ] {
        let mut missing = source.clone();
        missing.tables[TableId::Countries.index()] = view.clone().into();
        reduced.reduce(&mut missing, project, TableRows::Countries);
        assert_eq!(*missing.tables[TableId::Countries.index()], view);
        assert_eq!(visits.get(), 64);
    }
}
