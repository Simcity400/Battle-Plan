use super::*;

fn check(
    captured: &[u8],
    saved: &[u8],
    names: &[(u32, &[u8])],
    handle: u32,
    count: u32,
) -> io::Result<Report> {
    let (mut frame, _, prototype) = fixture();
    let mut pack = saved::Pack::default();
    let mut countries = Vec::new();
    let mut research = Vec::new();
    let mut slots = Vec::new();
    for id in 1..=count {
        let tag = format!("C{id}").into_bytes();
        countries.push(CountryRow {
            id,
            canonical_slot: id,
            tag: tag.clone(),
            ..prototype.clone()
        });
        pack.countries.insert(
            tag,
            saved::Country {
                research: Some(saved::Research {
                    technologies: vec![
                        (
                            saved.to_vec(),
                            saved::Technology {
                                level: Some(1),
                                progress: None,
                                ahead_reduction: None,
                            },
                        );
                        2
                    ],
                    slots: vec![saved::ResearchSlot {
                        technology: saved.to_vec(),
                        points: None,
                        used_saved_points: None,
                        points_factor: None,
                    }],
                }),
                ..Default::default()
            },
        );
        for database_index in [5, 6] {
            research.push(ResearchTechnologyRow {
                country: id,
                database_index,
                technology_handle: handle,
                level: 1,
                ..Default::default()
            });
        }
        slots.push(ResearchSlotRow {
            country: id,
            database_index: replay::rows::Recorded::Value(6),
            technology_handle: handle,
            technology_present: 1,
            research_speed_raw: 100_000,
            ..Default::default()
        });
    }
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(
            [5, 6]
                .map(|database_index| TechnologyDefinitionRow {
                    database_index,
                    technology_handle: handle,
                    id: captured.to_vec(),
                    max_level: 2,
                    ..Default::default()
                })
                .to_vec(),
        ));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(research));
    *frame.tables[TableId::ResearchSlots.index()] =
        TableView::Complete(TableRows::ResearchSlots(slots));
    let links = countries.iter().map(|r| (r.id, r)).collect();
    let names = names.iter().copied().collect();
    let mut report = refreshed_report();
    report.read_hours = Some(BTreeMap::new());
    report.forced_full = true;
    compare_research(&frame, &pack, &links, &names, &mut report)?;
    Ok(report)
}

#[test]
fn research_names_use_recorded_first_spelling_for_census_and_slots_at_scale() {
    let report = check(
        b"Mixed_Research\0tail",
        b"mixed_research",
        &[(5, b"mixed_research")],
        5,
        5_000,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ResearchTechnologies"], 10_000);
    assert_eq!(report.compared_tables["ResearchSlots"], 5_000);
    let report = check(
        b"Mixed_Research",
        b"Mixed_Research",
        &[(5, b"mixed_research")],
        5,
        1,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 3);
}

#[test]
fn research_name_joins_refuse_missing_ambiguous_and_non_ascii_aliases() {
    for names in [
        vec![],
        vec![(5, b"other".as_slice())],
        vec![(5, b"Mixed_Research".as_slice()), (6, b"mixed_research")],
    ] {
        assert!(check(b"Mixed_Research", b"mixed_research", &names, 5, 1).is_err());
    }
    assert!(
        check(
            b"\xc3\x89_Research",
            b"\xc3\xa9_research",
            &[(5, b"\xc3\xa9_research")],
            5,
            1
        )
        .is_err()
    );
    let report = check(b"-research", b"zero", &[(0, b"zero")], 0, 1).unwrap();
    assert!(report.differences.is_empty());
}

#[test]
fn research_writer_spelling_authenticates_the_serialized_handle() {
    for (name, saved, names, handle) in [
        (
            b"alpha".as_slice(),
            b"alpha".as_slice(),
            vec![(7, b"alpha".as_slice()), (8, b"beta".as_slice())],
            8,
        ),
        (b"alpha", b"alpha", vec![(7, b"alpha".as_slice())], 8),
        (
            b"-research",
            b"zero",
            vec![(0, b"zero".as_slice()), (5, b"zero".as_slice())],
            5,
        ),
    ] {
        let Err(error) = check(name, saved, &names, handle, 1) else {
            panic!("technology handle {handle} was not authenticated");
        };
        assert!(error.to_string().contains("handle"), "{error}");
    }
    let report = check(b"ZERO", b"zero", &[(0, b"zero")], 0, 1).unwrap();
    assert!(report.differences.is_empty());
}
