use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{CountryLinkRow, FactionMemberRow, SubjectRow, TableRows, WarRelationRow},
};

fn built(
    links: &BTreeMap<u32, u32>,
    ids: &BTreeMap<u32, u32>,
    relations: &Relations,
    ours: u32,
    sent: &[SentVolunteerRow],
    divisions: &[DivisionRow],
) -> Result<Index, String> {
    let key = Key::read(sent, divisions, links, ids, ours)?;
    Ok(Index::new(links, relations, &key))
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

fn world(
    wars: &[(u32, u32)],
    factions: &[(u32, u64)],
    subjects: &[(u32, u32)],
) -> (BTreeMap<u32, u32>, BTreeMap<u32, u32>, Relations) {
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
    let relations = Relations::read(&frame, &links).unwrap();
    (
        links,
        (0..8).map(|slot| (slot, 100 + slot)).collect(),
        relations,
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
    // A common enemy alone (wartime ally) is neither faction nor subject.
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
    // Host 2 fights the representative's logical host 3: stop before host 3.
    assert_eq!(build(&[10, 20]).of(103, 103), Some(Enemy));
    assert_eq!(build(&[20, 10]).of(103, 103), Some(Friend));
    assert_eq!(build(&[0, 999, 20, 10, 20]).of(103, 103), Some(Friend));
    assert_eq!(build(&[10, 20]).of(104, 103), Some(Friend));
    assert_eq!(build(&[10]).of(102, 107), Some(Friend));
    assert_eq!(build(&[10]).of(107, 103), Some(Enemy));
    // Membership belongs to the vector owner, not the division roster owner.
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
    // The viewer (1) fights no one itself; its volunteers serve host 2, which
    // fights 3 and 4. 5 shares the viewer's faction; 6 is unrelated.
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
    // Without the volunteer the owner-only reading leaves 3 and 4 other.
    let alone = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
    assert_eq!(plate(false, alone.of(103, 103)), Allegiance::Other);
    // Ours needs no relation input; an unavailable relation is drawn as other.
    assert_eq!(plate(true, None), Allegiance::Ours);
    assert_eq!(plate(false, None), Allegiance::Other);
}

/// Port items and combat tiles (`14197B4F0`) test enemy before friend over the first task
/// force's roster and logical owners, with no sent-volunteer loop and no bypass for a
/// signed-nonpositive roster owner; the at-sea counter's relation (`1418C4B50`) differs on
/// each of these.
#[test]
fn mission_items_test_enemy_first_without_the_volunteer_loop() {
    use Allegiance::*;
    // The viewer (1) shares a faction with 2 and fights 4.
    let (links, ids, relations) = world(&[(1, 4)], &[(1, 7), (2, 7)], &[]);
    let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
    for (own, roster, logical, expected) in [
        (true, 104, 104, Ours),
        (false, 102, 102, Allied),
        (false, 104, 104, Enemy),
        (false, 106, 106, Other),
        // A friend and an enemy: the enemy byte wins, where the counter's friend does.
        (false, 102, 104, Enemy),
        (false, 104, 102, Enemy),
        (false, 106, 102, Allied),
        // The null roster reference is no bypass here.
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
    // Volunteers the viewer sends to 2, which fights 3, colour only the counter.
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
    // Unavailable relations or an unlinked owner name no side.
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
    ids.insert(3, u32::MAX);
    assert_eq!(
        built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions)
            .unwrap()
            .of(103, 103),
        Some(Side::Neutral)
    );
}
#[test]
fn attache_and_air_access_do_not_tint_an_unrelated_country() {
    use replay::rows::RelationLinkRow;
    let (links, ids, relations) = world(&[], &[], &[]);
    let mut frame = empty_frame();
    *frame.tables[TableId::RelationLinks.index()] =
        TableView::Complete(TableRows::RelationLinks(vec![RelationLinkRow {
            country: 102,
            target_index: 1,
            links: RelationLinkRow::ATTACHE | RelationLinkRow::AIR_BASE_ACCESS,
        }]));
    // Native type 2934 = military access (140D0A6D0 factory). The
    // counter's flags 0,0 exclude the +2A0 access arm of 1406F3B00.
    *frame.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(vec![
            replay::rows::RelationTupleRow {
                owner_country: 101,
                target_country: 102,
                type_id: 0x2934,
                first_country: 101,
                second_country: 102,
                ..Default::default()
            },
        ]));
    let relations = relations.with_attaches(&frame, &links).unwrap();
    assert!(relations.allied_for_fog(2, 1));
    assert_eq!(
        built(&links, &ids, &relations, 1, &[], &[])
            .unwrap()
            .of(102, 102),
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
    for shift in 0..9 {
        let mut order: Vec<_> = (0..9).collect(); // null and one unresolved handle
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
                    let mut friend = relations.shared(1, a) || relations.shared(1, b);
                    let mut enemy = relations.at_war(1, a) || relations.at_war(1, b);
                    for reference in &order {
                        let Some(division) = divisions.iter().find(|d| d.id == *reference) else {
                            continue;
                        };
                        let host = links[&division.logical_owner];
                        if host == a {
                            friend = true;
                            break;
                        }
                        if relations.at_war(b, host) {
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
    let (links, ids, _) = world(&[], &[], &[]);
    let mut frame = empty_frame();
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        links
            .iter()
            .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
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
    let memo = Memo::new(TABLES, 4);
    let first = Index::read(&memo, &frame, &links, &ids, 1);
    assert_eq!(first.of(102, 102), Some(Side::Friend));
    // Divisions is a new allocation every hour; an equal host keeps the index.
    frame.tables[TableId::Divisions.index()] = divisions(102);
    assert!(Arc::ptr_eq(
        &first,
        &Index::read(&memo, &frame, &links, &ids, 1)
    ));
    // A changed host or a changed relation table builds a new one.
    frame.tables[TableId::Divisions.index()] = divisions(103);
    let moved = Index::read(&memo, &frame, &links, &ids, 1);
    assert_eq!(moved.of(103, 103), Some(Side::Friend));
    frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 101,
            second_country: 104,
            ..Default::default()
        }]))
        .into();
    let at_war = Index::read(&memo, &frame, &links, &ids, 1);
    assert!(!Arc::ptr_eq(&moved, &at_war));
    assert_eq!(at_war.of(104, 104), Some(Side::Enemy));
    // A missing input withholds rather than reusing an older index.
    frame.tables[TableId::SentVolunteers.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    }
    .into();
    assert!(
        Index::read(&memo, &frame, &links, &ids, 1)
            .problem()
            .contains("SentVolunteers")
    );
}

#[test]
#[ignore = "unit-scale tint cost; run release, one test thread"]
fn type_tint_unit_scale() {
    use std::{hint::black_box, time::Instant};
    let stress = std::env::var("BATTLE_PLAN_UNIT_SHAPE").or_else(|_| std::env::var("HOI4_UNIT_SHAPE")).as_deref() == Ok("stress");
    let (n, d) = if stress {
        (5_000_u32, 100_000_usize)
    } else {
        (1_500, 50_000)
    };
    let links: BTreeMap<_, _> = (1..=n).map(|s| (s + 10_000, s)).collect();
    let ids: BTreeMap<_, _> = links.iter().map(|(&id, &slot)| (slot, id)).collect();
    let mut frame = empty_frame();
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (1..=n)
            .flat_map(|a| {
                (1..=4).map(move |offset| WarRelationRow {
                    first_country: 10_000 + a,
                    second_country: 10_000 + (a + offset) % n + 1,
                    ..Default::default()
                })
            })
            .collect(),
    ));
    let relations = Relations::read(&frame, &links).unwrap();
    let divisions: Vec<_> = (1..=d)
        .map(|i| DivisionRow {
            id: i as u64,
            roster_owner: 10_001,
            logical_owner: 10_000 + (i as u32 % n) + 1,
            ..Default::default()
        })
        .collect();
    // Deliberately all divisions in this viewer's ordered sent list. The
    // ordinary case references only a small part of the division census.
    let sent: Vec<_> = (0..d)
        .map(|i| SentVolunteerRow {
            country: 10_001,
            ordinal: i as u32,
            division: (i + 1) as u64,
        })
        .collect();
    let (mut keys, mut builds, mut lookups) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..25 {
        // Every hour reads the key; only a changed key or table builds.
        let start = Instant::now();
        let key = black_box(Key::read(&sent, &divisions, &links, &ids, 1).unwrap());
        keys.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        let index = black_box(Index::new(&links, &relations, &key));
        builds.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        for division in &divisions {
            black_box(index.of(
                black_box(division.roster_owner),
                black_box(division.logical_owner),
            ));
        }
        lookups.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(index.of(10_001, 10_001), Some(Side::Friend));
    }
    for times in [&mut keys, &mut builds, &mut lookups] {
        times.sort_by(f64::total_cmp);
    }
    println!(
        "TYPE_TINT countries={n} divisions={d} sent={} hourly_key_median_ms={:.6} build_median_ms={:.6} all_lookup_median_ms={:.6}",
        sent.len(),
        keys[12],
        builds[12],
        lookups[12]
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
    let layers = installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
    let defines = installed_content::defines::read(&layers).unwrap();
    let colors = Colors::read(&defines).unwrap();
    assert_eq!(colors, Colors::TEST);
    for side in [Side::Friend, Side::Enemy, Side::Neutral] {
        assert_eq!(colors.of(side)[3], 1.0);
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
