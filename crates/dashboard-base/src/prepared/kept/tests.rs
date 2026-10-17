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

#[test]
fn unchanged_country_keys_reuse_the_projection_while_balances_and_samples_move() {
    let mut frame = crate::test_fixtures::scale_frame(1, 100, 5_000);
    let mut reduced = Reduced::new(TableId::Countries);
    let before = crate::prepared::prepass::noted("kept.countries");
    let mut kept = frame.clone();
    reduced.countries(&mut kept);
    let first = kept.tables[TableId::Countries.index()].identity();
    for hour in 101..104 {
        frame.hour = hour;
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        for row in rows {
            row.political_power_raw = Some(i64::from(hour));
        }
        let index = TableId::Countries.index();
        frame.tables[index] = frame.tables[index].clone().with_sampled_at(Some(hour));
        let mut kept = frame.clone();
        reduced.countries(&mut kept);
        assert!(first.is(&kept.tables[index]));
        assert_eq!(kept.tables[index].sampled_at(), Some(hour));
    }
    assert_eq!(
        crate::prepared::prepass::noted("kept.countries") - before,
        1
    );
}

#[test]
fn country_key_changes_rebuild_and_missing_evidence_is_not_reused() {
    let mut source = crate::test_fixtures::scale_frame(1, 100, 2);
    let mut reduced = Reduced::new(TableId::Countries);
    let index = TableId::Countries.index();
    let mut first = source.clone();
    reduced.countries(&mut first);
    let mut previous = first.tables[index].identity();
    for field in 0..3 {
        let TableView::Complete(TableRows::Countries(rows)) = &mut *source.tables[index] else {
            unreachable!()
        };
        match field {
            0 => rows[0].id += 1_000,
            1 => rows[0].canonical_slot += 1_000,
            _ => rows[0].tag = b"changed holder".to_vec(),
        }
        let expected: Vec<_> = rows
            .iter()
            .map(|row| CountryRow {
                id: row.id,
                canonical_slot: row.canonical_slot,
                tag: row.tag.clone(),
                ..Default::default()
            })
            .collect();
        let mut kept = source.clone();
        reduced.countries(&mut kept);
        assert_eq!(kept.complete::<tables::Countries>().unwrap(), expected);
        assert!(!previous.is(&kept.tables[index]));
        previous = kept.tables[index].identity();
    }
    for view in [
        TableView::Unavailable {
            reason: "synthetic".into(),
        },
        TableView::Truncated {
            rows: TableRows::Countries(vec![CountryRow::default()]),
            missing: 7,
        },
    ] {
        let mut missing = source.clone();
        missing.tables[index] = view.clone().into();
        assert!(!reduced.reuse(&mut missing));
        reduced.countries(&mut missing);
        assert_eq!(*missing.tables[index], view);
    }
}
