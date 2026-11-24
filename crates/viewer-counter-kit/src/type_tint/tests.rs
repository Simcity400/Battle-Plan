use super::{
    Allegiance, Arc, BTreeMap, BTreeSet, Colors, DecodedFrame, DivisionRow, Index, Key, Memo, SentVolunteerRow, Side, TABLES, TableId, plate, tables,
};
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow, rows::DirectedWars,
    rows::FactionMemberRow, rows::Recorded, rows::ReservedCountryNamingRow,
    rows::ReservedReverseVisibilityInputs, rows::ReverseZeroVisibilityInputs, rows::SubjectRow,
    rows::TableRows, rows::WarCell, rows::WarRelationRow,
};
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_foundation::visibility::Relations;

fn built(
    links: &BTreeMap<u32, u32>,
    ids: &BTreeMap<u32, u32>,
    frame: &DecodedFrame,
    ours: u32,
    sent: &[SentVolunteerRow],
    divisions: &[DivisionRow],
) -> Result<Index, String> {
    let key = Key::read(sent, divisions, links, ours, frame.hour)?;
    let mut frame = frame.clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.id = ids[&row.canonical_slot];
        }
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *frame.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows {
            if let Recorded::Value(Some(packet)) = &mut row.directed_wars {
                row.country = ids[&packet.receiver_slot];
                packet.receiver_raw_id = row.country as i32;
            }
        }
    }
    Ok(Index::new(&frame, links, &key))
}

fn empty_frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 1,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
    }
}

#[test]
fn foreign_division_icons_complete_war_census_survives_the_tint_memo_projection() {
    let (links, _, mut frame) = world(&[(1, 4)], &[(1, 9), (2, 9)], &[]);
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *frame.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows {
            row.directed_wars = Recorded::NotRecorded;
        }
    }
    let memo = Memo::new(TABLES);
    let index = Index::read(&memo, &frame, &links, 1);
    assert_eq!(index.of(101, 101), Some(Side::Friend));
    assert_eq!(index.of(102, 102), Some(Side::Friend));
    assert_eq!(index.of(103, 103), Some(Side::Neutral));
    assert_eq!(index.of(104, 104), Some(Side::Enemy));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(Vec::new()));
    assert_eq!(
        Index::read(&memo, &frame, &links, 1).of(104, 104),
        Some(Side::Neutral)
    );
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "capture refused".into(),
    };
    let unavailable = Index::read(&memo, &frame, &links, 1);
    assert_eq!(unavailable.of(101, 101), Some(Side::Friend));
    assert_eq!(unavailable.of(104, 104), None);
    assert!(unavailable.problem().contains("WarRelations"));
}

fn world(
    wars: &[(u32, u32)],
    factions: &[(u32, u64)],
    subjects: &[(u32, u32)],
) -> (BTreeMap<u32, u32>, BTreeMap<u32, u32>, DecodedFrame) {
    let links: BTreeMap<_, _> = (0..8)
        .map(|s| (100 + s, s))
        .chain([(0, 0), (501, 1)])
        .collect();
    let mut frame = empty_frame();
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        links
            .iter()
            .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        wars.iter()
            .map(|&(a, b)| WarRelationRow {
                first_country: 100 + a,
                second_country: 100 + b,
                start_hour: 1,
                end_hour: 2,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            factions
                .iter()
                .map(|&(c, f)| FactionMemberRow {
                    country: 100 + c,
                    faction: f,
                    ordinal: 0,
                })
                .collect(),
        ));
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
        subjects
            .iter()
            .map(|&(a, b)| SubjectRow {
                overlord: 100 + a,
                subject: 100 + b,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..8)
            .map(|slot| CountryRow {
                id: 100 + slot,
                canonical_slot: slot,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (1..8)
                .map(|slot| CountryCounterInputRow {
                    country: 100 + slot,
                    directed_wars: Recorded::Value(Some(DirectedWars {
                        sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                        receiver_raw_id: (100 + slot) as i32,
                        receiver_slot: slot,
                        matrix_count: 8,
                        cells: (0..8)
                            .map(|target_slot| WarCell {
                                target_slot,
                                active: wars.contains(&(slot, target_slot))
                                    || wars.contains(&(target_slot, slot)),
                            })
                            .collect(),
                        ..Default::default()
                    })),
                    forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                        target: Some(0),
                        war: Some([0, 0]),
                        subject: Some(Default::default()),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                faction: Recorded::Value(Some(0)),
                reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                    sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                    cells: (0..8)
                        .map(|slot| {
                            Some(ReverseZeroVisibilityInputs {
                                target: Some(100 + slot),
                                war: Some([0, 0]),
                                subject: Some(Default::default()),
                                ..Default::default()
                            })
                        })
                        .collect(),
                })),
                ..Default::default()
            },
        ]));
    (
        links,
        (0..8).map(|slot| (slot, 100 + slot)).collect(),
        frame,
    )
}
#[test]
fn country_identity_faction_subject_and_active_war_predicates() {
    use Side::*;
    let (links, ids, relations) = world(
        &[(1, 4), (1, 5)],
        &[(1, 7), (2, 7), (4, 7)],
        &[(1, 3), (1, 5), (3, 6)],
    );
    let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
    for (owner, logical, expected) in [
        (101, 101, Friend),
        (501, 501, Friend),
        (102, 102, Friend),
        (103, 103, Friend),
        (104, 104, Enemy),
        (105, 105, Enemy),
        (106, 106, Neutral),
        (107, 107, Neutral),
        (107, 102, Friend),
        (107, 104, Enemy),
        (102, 104, Friend),
        (104, 102, Friend),
        (0, 104, Friend),
        (u32::MAX, 104, Friend),
    ] {
        assert_eq!(
            index.of(owner, logical),
            Some(expected),
            "{owner} {logical}"
        );
    }
    let reverse = built(&links, &ids, &relations, 3, &[], &[]).unwrap();
    assert_eq!(reverse.of(101, 101), Some(Friend));
    let (links, ids, relations) = world(&[(1, 4), (2, 4)], &[], &[]);
    assert_eq!(
        built(&links, &ids, &relations, 1, &[], &[])
            .unwrap()
            .of(102, 102),
        Some(Neutral)
    );
}
fn volunteers(order: &[u64]) -> Vec<SentVolunteerRow> {
    order
        .iter()
        .enumerate()
        .map(|(ordinal, &division)| SentVolunteerRow {
            country: 101,
            ordinal: ordinal as u32,
            division,
        })
        .collect()
}

fn edit_wars(frame: &mut DecodedFrame, slot: u32, edit: impl FnOnce(&mut DirectedWars)) {
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    let packet = match &mut rows[(slot - 1) as usize].directed_wars {
        Recorded::Value(Some(packet)) => packet,
        _ => panic!("fixture directed packet missing"),
    };
    edit(packet);
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(rows));
}

#[test]
fn directed_owner_veto_and_volunteer_tests_never_inherit_reverse_truth() {
    let (links, ids, mut frame) = world(&[(1, 4), (2, 3)], &[(1, 9), (4, 9)], &[]);
    edit_wars(&mut frame, 1, |packet| packet.cells[4].active = false);
    let index = built(&links, &ids, &frame, 1, &[], &[]).unwrap();
    assert_eq!(index.of(104, 104), Some(Side::Friend));
    assert_eq!(
        index.mission_item(false, 104, 104),
        Some(Allegiance::Allied)
    );
    edit_wars(&mut frame, 3, |packet| packet.cells[2].active = false);
    let divisions = [DivisionRow {
        id: 10,
        logical_owner: 102,
        ..Default::default()
    }];
    let index = built(&links, &ids, &frame, 1, &volunteers(&[10]), &divisions).unwrap();
    assert_eq!(index.of(103, 103), Some(Side::Neutral));
}

#[test]
fn unknown_directed_cells_refuse_only_unproved_sides_and_ordered_hosts() {
    let (links, ids, mut frame) = world(&[], &[], &[]);
    edit_wars(&mut frame, 1, |packet| {
        packet.cells.retain(|cell| cell.target_slot != 4)
    });
    let index = built(&links, &ids, &frame, 1, &[], &[]).unwrap();
    assert_eq!(index.of(104, 104), None);
    assert_eq!(index.of(501, 104), Some(Side::Friend));
    assert_eq!(index.mission_item(false, 501, 104), None);
    assert_eq!(index.of(u32::MAX, 104), Some(Side::Friend));
    edit_wars(&mut frame, 3, |packet| {
        packet.cells.retain(|cell| cell.target_slot != 2)
    });
    let divisions = [
        DivisionRow {
            id: 10,
            logical_owner: 102,
            ..Default::default()
        },
        DivisionRow {
            id: 20,
            logical_owner: 103,
            ..Default::default()
        },
    ];
    assert_eq!(
        built(&links, &ids, &frame, 1, &volunteers(&[10, 20]), &divisions)
            .unwrap()
            .of(103, 103),
        None
    );
    assert_eq!(
        built(&links, &ids, &frame, 1, &volunteers(&[20, 10]), &divisions)
            .unwrap()
            .of(103, 103),
        Some(Side::Friend)
    );
}

#[test]
fn actual_zero_and_positive_zero_aliases_keep_distinct_native_self_gates() {
    let (links, ids, frame) = world(&[], &[], &[]);
    let index = built(&links, &ids, &frame, 0, &[], &[]).unwrap();
    assert_eq!(index.of(100, 100), Some(Side::Neutral));
    assert_eq!(index.mission_item(false, 0, 0), Some(Allegiance::Allied));
    assert_eq!(index.mission_item(false, 100, 100), Some(Allegiance::Other));
    let divisions = [DivisionRow {
        id: 10,
        logical_owner: 0,
        ..Default::default()
    }];
    let sent = [SentVolunteerRow {
        country: 0,
        ordinal: 0,
        division: 10,
    }];
    let index = built(&links, &ids, &frame, 0, &sent, &divisions).unwrap();
    assert_eq!(index.of(100, 100), Some(Side::Neutral));
}

#[test]
fn missing_volunteer_inputs_keep_proven_friend_and_independent_port_enemy() {
    let (links, _ids, mut frame) = world(&[(1, 4)], &[], &[]);
    frame.tables[TableId::SentVolunteers.index()] = TableView::Unavailable {
        reason: "sent list capture failed".into(),
    }
    .into();
    let index = Index::read(&Memo::new(TABLES), &frame, &links, 1);
    assert_eq!(index.of(501, 501), Some(Side::Friend));
    assert_eq!(index.of(104, 104), None);
    assert_eq!(index.mission_item(false, 104, 104), Some(Allegiance::Enemy));
    assert!(index.problem().contains("SentVolunteers"));
}

#[test]
fn carried_directed_packets_do_not_survive_the_memo_hour_key() {
    let (links, _, frame) = world(&[(1, 4)], &[], &[]);
    let memo = Memo::new(TABLES);
    let first = Index::read(&memo, &frame, &links, 1);
    assert_eq!(first.of(104, 104), Some(Side::Enemy));
    let mut carried = frame.clone();
    carried.hour += 1;
    let next = Index::read(&memo, &carried, &links, 1);
    assert_eq!(next.of(104, 104), None);
    assert!(!Arc::ptr_eq(&first, &next));
    assert_eq!(next.of(501, 501), Some(Side::Friend));
    assert!(Arc::ptr_eq(&first, &Index::read(&memo, &frame, &links, 1)));
}
#[test]
fn first_volunteer_match_and_friend_precedence_survive_aliases_and_stale_refs() {
    use Side::*;
    let (links, ids, relations) = world(&[(2, 3)], &[(1, 1), (4, 1)], &[]);
    let divisions = vec![
        DivisionRow {
            id: 10,
            logical_owner: 102,
            ..Default::default()
        },
        DivisionRow {
            id: 20,
            logical_owner: 103,
            ..Default::default()
        },
    ];
    let build =
        |order: &[u64]| built(&links, &ids, &relations, 1, &volunteers(order), &divisions).unwrap();
    assert_eq!(build(&[10, 20]).of(103, 103), Some(Enemy));
    assert_eq!(build(&[20, 10]).of(103, 103), Some(Friend));
    assert_eq!(build(&[0, 999, 20, 10, 20]).of(103, 103), Some(Friend));
    assert_eq!(build(&[10, 20]).of(104, 103), Some(Friend));
    assert_eq!(build(&[10]).of(102, 107), Some(Friend));
    assert_eq!(build(&[10]).of(107, 103), Some(Enemy));
    let mut sent = volunteers(&[10]);
    sent[0].country = 102;
    assert_eq!(
        built(&links, &ids, &relations, 1, &sent, &divisions)
            .unwrap()
            .of(102, 102),
        Some(Neutral)
    );
}
#[test]
fn plates_take_the_type_icons_relation_through_sent_volunteers() {
    let (links, ids, relations) = world(&[(2, 3), (2, 4)], &[(1, 9), (5, 9)], &[]);
    let divisions = [DivisionRow {
        id: 10,
        logical_owner: 102,
        ..Default::default()
    }];
    let index = built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions).unwrap();
    let plate_of = |slot: u32| plate(slot == 1, index.of(100 + slot, 100 + slot));
    assert_eq!(plate_of(1), Allegiance::Ours);
    assert_eq!(plate_of(2), Allegiance::Allied);
    assert_eq!(plate_of(3), Allegiance::Enemy);
    assert_eq!(plate_of(4), Allegiance::Enemy);
    assert_eq!(plate_of(5), Allegiance::Allied);
    assert_eq!(plate_of(6), Allegiance::Other);
    let alone = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
    assert_eq!(plate(false, alone.of(103, 103)), Allegiance::Other);
    assert_eq!(plate(true, None), Allegiance::Ours);
    assert_eq!(plate(false, None), Allegiance::Other);
}

#[test]
fn mission_items_test_enemy_first_without_the_volunteer_loop() {
    use Allegiance::*;
    let (links, ids, relations) = world(&[(1, 4)], &[(1, 7), (2, 7)], &[]);
    let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
    for (own, roster, logical, expected) in [
        (true, 104, 104, Ours),
        (false, 102, 102, Allied),
        (false, 104, 104, Enemy),
        (false, 106, 106, Other),
        (false, 102, 104, Enemy),
        (false, 104, 102, Enemy),
        (false, 106, 102, Allied),
        (false, 0, 104, Enemy),
        (false, 0, 106, Other),
    ] {
        assert_eq!(
            index.mission_item(own, roster, logical),
            Some(expected),
            "{own} {roster} {logical}"
        );
    }
    assert_eq!(index.of(102, 104), Some(Side::Friend));
    assert_eq!(index.of(0, 104), Some(Side::Friend));
    let (links, ids, relations) = world(&[(2, 3)], &[], &[]);
    let divisions = [DivisionRow {
        id: 10,
        logical_owner: 102,
        ..Default::default()
    }];
    let index = built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions).unwrap();
    assert_eq!(index.of(102, 102), Some(Side::Friend));
    assert_eq!(index.of(103, 103), Some(Side::Enemy));
    assert_eq!(index.mission_item(false, 102, 102), Some(Other));
    assert_eq!(index.mission_item(false, 103, 103), Some(Other));
    assert_eq!(
        Index::unavailable("x".into()).mission_item(false, 102, 102),
        None
    );
    assert_eq!(
        Index::unavailable("x".into()).mission_item(true, 102, 102),
        Some(Ours)
    );
    assert_eq!(index.mission_item(false, 999, 999), None);
}

#[test]
fn nonpositive_receiver_id_cannot_be_at_war() {
    let (links, mut ids, relations) = world(&[(1, 2), (2, 3)], &[], &[]);
    ids.insert(1, u32::MAX);
    assert_eq!(
        built(&links, &ids, &relations, 1, &[], &[])
            .unwrap()
            .of(102, 102),
        Some(Side::Neutral)
    );
    let divisions = [DivisionRow {
        id: 10,
        logical_owner: 102,
        ..Default::default()
    }];
    ids.insert(3, u32::MAX - 1);
    assert_eq!(
        built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions)
            .unwrap()
            .of(103, 103),
        Some(Side::Neutral)
    );
}
#[test]
fn indexed_first_match_agrees_with_the_ordered_native_loop() {
    let (links, ids, relations) = world(
        &[(1, 4), (2, 3), (3, 5), (2, 6)],
        &[(1, 8), (7, 8)],
        &[(1, 2)],
    );
    let divisions: Vec<_> = (1..8)
        .map(|id| DivisionRow {
            id,
            logical_owner: 100 + id as u32,
            ..Default::default()
        })
        .collect();
    let ordinary = Relations::read(&relations, &links).unwrap();
    for shift in 0..9 {
        let mut order: Vec<_> = (0..9).collect();
        order.rotate_left(shift);
        for reverse in [false, true] {
            if reverse {
                order.reverse();
            }
            let index =
                built(&links, &ids, &relations, 1, &volunteers(&order), &divisions).unwrap();
            for roster in 100..108 {
                for logical in 100..108 {
                    let a = links[&roster];
                    let b = links[&logical];
                    let mut friend = ordinary.shared(1, a) || ordinary.shared(1, b);
                    let mut enemy = ordinary.at_war(1, a) || ordinary.at_war(1, b);
                    for reference in &order {
                        let Some(division) = divisions.iter().find(|d| d.id == *reference) else {
                            continue;
                        };
                        let host = links[&division.logical_owner];
                        if host == a {
                            friend = true;
                            break;
                        }
                        if ordinary.at_war(b, host) {
                            enemy = true;
                            break;
                        }
                    }
                    let expected = if friend {
                        Side::Friend
                    } else if enemy {
                        Side::Enemy
                    } else {
                        Side::Neutral
                    };
                    assert_eq!(
                        index.of(roster, logical),
                        Some(expected),
                        "order={order:?} {roster} {logical}"
                    );
                }
            }
        }
    }
}
#[test]
fn the_memo_reuses_an_index_until_its_tables_or_key_change() {
    let (links, _ids, mut frame) = world(&[], &[], &[]);
    let divisions = |host: u32| {
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: 10,
            logical_owner: host,
            ..Default::default()
        }]))
        .into()
    };
    frame.tables[TableId::Divisions.index()] = divisions(102);
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(volunteers(&[10])));
    let memo = Memo::new(TABLES);
    let first = Index::read(&memo, &frame, &links, 1);
    assert_eq!(first.of(102, 102), Some(Side::Friend));
    frame.tables[TableId::Divisions.index()] = divisions(102);
    assert!(Arc::ptr_eq(&first, &Index::read(&memo, &frame, &links, 1)));
    frame.tables[TableId::Divisions.index()] = divisions(103);
    let moved = Index::read(&memo, &frame, &links, 1);
    assert_eq!(moved.of(103, 103), Some(Side::Friend));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs({
            let mut rows = frame
                .complete::<tables::CountryCounterInputs>()
                .unwrap()
                .to_vec();
            if let Recorded::Value(Some(packet)) = &mut rows[0].directed_wars {
                packet.cells[4].active = true;
            }
            rows
        }));
    let at_war = Index::read(&memo, &frame, &links, 1);
    assert!(!Arc::ptr_eq(&moved, &at_war));
    assert_eq!(at_war.of(104, 104), Some(Side::Enemy));
    frame.tables[TableId::SentVolunteers.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    }
    .into();
    assert!(
        Index::read(&memo, &frame, &links, 1)
            .problem()
            .contains("SentVolunteers")
    );
}

#[test]
fn incomplete_inputs_withhold_instead_of_inventing_neutral() {
    let (links, ids, relations) = world(&[], &[], &[]);
    let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
    assert_eq!(index.of(101, 777), None);
    let bad = Index {
        problem: Some("SentVolunteers unavailable".into()),
        ..Default::default()
    };
    assert_eq!(bad.of(101, 101), None);
    assert!(bad.problem().contains("SentVolunteers"));
}
#[test]
fn mod_defines_override_all_three_type_colors_and_keep_texture_alpha() {
    let root = std::env::temp_dir().join(format!("hoi4_type_tint_{}", std::process::id()));
    let base = root.join("base");
    let addon = root.join("mod");
    std::fs::create_dir_all(base.join("common/defines")).unwrap();
    std::fs::create_dir_all(addon.join("common/defines")).unwrap();
    std::fs::write(base.join("common/defines/00_base.lua"),b"NDefines={NGraphics={FRIEND_COLOR={0.1,0.2,0.3},ENEMY_COLOR={0.4,0.5,0.6},NEUTRAL_COLOR={0.7,0.8,0.9}}}").unwrap();
    std::fs::write(addon.join("common/defines/zz_tint.lua"),b"NDefines.NGraphics.FRIEND_COLOR={0.2,0.8,0.3};NDefines.NGraphics.ENEMY_COLOR={0.9,0.1,0.4};NDefines.NGraphics.NEUTRAL_COLOR={0.8,0.6,0.4}").unwrap();
    let layers =
        content_reader_layers::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
    let defines = content_reader_defines::defines::read(&layers).unwrap();
    let colors = Colors::read(&defines).unwrap();
    assert_eq!(colors, Colors::TEST);
    for side in [Side::Friend, Side::Enemy, Side::Neutral] {
        assert_eq!(colors.of(side)[3], 1.0);
    }
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    for file in [
        base.join("common/defines/00_base.lua"),
        addon.join("common/defines/zz_tint.lua"),
    ] {
        defines
            .load(file.to_str().unwrap(), &std::fs::read(&file).unwrap())
            .unwrap();
    }
    defines
        .load("bad", b"NDefines.NGraphics.ENEMY_COLOR={1,2}")
        .unwrap();
    assert!(
        Colors::read(&defines)
            .unwrap_err()
            .to_string()
            .contains("ENEMY_COLOR")
    );
    std::fs::remove_dir_all(root).unwrap();
}
