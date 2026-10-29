use crate::{
    products::{
        minefields::{self, MinefieldsObserver},
        naval_supremacy::{self, NavalSupremacyObserver},
    },
    shared::Labels,
};
use dashboard_base::{
    catalogue::Refusals,
    prepared::{coverage::Coverage, prefix::Prefix},
    test_fixtures::{self, world_frame},
};
use dashboard_keyed::{
    Observer, Scan, Values,
    query::{self, Query},
};
use replay_data::{
    format::FormatVersion,
    rows::{self, TableId, TableRows, tables},
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
};

fn put(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
}
fn mine(country: u32, kind: u8, region: u32, amount: i64) -> rows::NavalRegionalMineRow {
    rows::NavalRegionalMineRow {
        country,
        kind,
        region,
        amount,
    }
}
fn mines(hour: i32, rows: Vec<rows::NavalRegionalMineRow>) -> DecodedFrame {
    let mut f = world_frame(hour, false);
    put(
        &mut f,
        TableId::NavalRegionalMines,
        TableRows::NavalRegionalMines(rows),
    );
    f
}
fn observe<O: Observer>(frame: &DecodedFrame) -> (O, Values) {
    let mut o = O::default();
    let mut v = Values::default();
    assert!(o.observe(FormatVersion::CURRENT, frame, &mut v).unwrap());
    (o, v)
}
fn values(v: &Values) -> BTreeMap<&str, i128> {
    v.iter().collect()
}
fn labels(minimum: i64, ratio: i64) -> Result<Arc<Labels>, String> {
    Ok(Arc::new(Labels {
        regions: Ok(Arc::new(dashboard_keyed::regions::RegionNames::new(BTreeMap::from([(73, Ok("Mod sea".into()))]), Vec::new()))),
        sea_regions: Ok(BTreeSet::from([73])),
        minima: BTreeMap::from([(73, Ok(minimum))]),
        ratio: Ok(ratio),
        warnings: Vec::new(),
    }))
}
fn supremacy(hour: i32, sample: Option<i32>, raws: &[(u32, i64)]) -> DecodedFrame {
    let mut f = world_frame(hour, false);
    for table in [
        TableId::FactionMembers,
        TableId::Subjects,
        TableId::WarRelations,
        TableId::AirCountryRelations,
        TableId::RelationTuples,
        TableId::CountryCounterInputs,
    ] {
        let rows = match table {
            TableId::FactionMembers => TableRows::FactionMembers(vec![]),
            TableId::Subjects => TableRows::Subjects(vec![]),
            TableId::WarRelations => TableRows::WarRelations(vec![]),
            TableId::AirCountryRelations => TableRows::AirCountryRelations(vec![]),
            TableId::RelationTuples => TableRows::RelationTuples(vec![]),
            TableId::CountryCounterInputs => TableRows::CountryCounterInputs(vec![]),
            _ => TableRows::ReservedCountryNaming(vec![]),
        };
        put(&mut f, table, rows);
    }
    f.tables[TableId::NavalSupremacy.index()] =
        replay_frame::frame::SharedView::from(TableView::Complete(TableRows::NavalSupremacy(
            raws.iter()
                .map(|&(country, current)| rows::NavalSupremacyRow {
                    region: 73,
                    region_slot: 4,
                    country,
                    current,
                    ..Default::default()
                })
                .collect(),
        )))
        .with_sampled_at(sample);
    f
}
fn publish<O: Observer>(
    frames: &[DecodedFrame],
) -> (dashboard_base::prepared::series::tests::Fixture, Scan<O>) {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    let mut writer = fixture.writer();
    for f in frames {
        scan.prepare(
            FormatVersion::CURRENT,
            f,
            O::READ != TableId::NavalSupremacy
                || f.tables[O::READ.index()].sampled_at() == Some(f.hour),
        )
        .unwrap();
    }
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    (fixture, scan)
}

#[test]
fn mine_witness_zero_unknown_negative_and_aliases() {
    let f = mines(10, vec![mine(11, 0, 100, 0), mine(11, 1, 73, -1)]);
    let (_, v) = observe::<MinefieldsObserver>(&f);
    let v = values(&v);
    assert_eq!(v.get("GER@7|extent"), Some(&100));
    assert_eq!(v.get("GER@7|73"), Some(&-1));
    assert!(!v.contains_key("GER@7|74"));
    assert!(!v.contains_key("ITA@8|extent"));
    let mut o = MinefieldsObserver::default();
    let mut v = Values::default();
    for rows in [
        vec![mine(11, 1, 73, 1)],
        vec![mine(11, 0, 2, 9)],
        vec![mine(11, 0, 2, 0), mine(11, 0, 2, 0)],
        vec![mine(11, 0, 2, 0), mine(11, 1, 2, 1)],
        vec![mine(11, 0, 2, 0), mine(9001, 0, 2, 0)],
    ] {
        v.clear();
        assert!(
            o.observe(FormatVersion::CURRENT, &mines(10, rows), &mut v)
                .unwrap()
        );
        // A refused vector is dated evidence of an incomplete inventory, never
        // an owner without mines.
        assert_eq!(values(&v), BTreeMap::from([("GER@7|refused", 1)]));
    }
    assert_eq!(o.refusals.values().sum::<u64>(), 5);
    // A vector whose owner cannot be named still counts against completeness.
    v.clear();
    assert!(
        o.observe(FormatVersion::CURRENT, &mines(10, vec![mine(4242, 0, 2, 0)]), &mut v)
            .unwrap()
    );
    assert_eq!(values(&v), BTreeMap::from([("?|refused", 1)]));
}
#[test]
fn invalid_mine_owner_does_not_erase_another_owner() {
    let (_, v) =
        observe::<MinefieldsObserver>(&mines(10, vec![mine(11, 0, 3, 0), mine(21, 1, 1, 2)]));
    let v = values(&v);
    assert_eq!(v.get("GER@7|extent"), Some(&3));
    assert!(!v.contains_key("ITA@8|extent"));
}
#[test]
fn mine_prepared_runs_keep_gaps_absence_and_resume() {
    let frames = [
        mines(10, vec![mine(11, 0, 100, 0), mine(11, 1, 73, -7)]),
        mines(11, vec![mine(11, 0, 100, 0)]),
        mines(13, vec![mine(9001, 0, 80, 0)]),
    ];
    let (fixture, _) = publish::<MinefieldsObserver>(&frames[..2]);
    let flag = AtomicBool::new(false);
    let mut scan = dashboard_base::prepared::series::read_catalogue::<Scan<MinefieldsObserver>>(
        &fixture.store,
        &fixture.key,
        MinefieldsObserver::NAME,
        1,
    )
    .unwrap()
    .unwrap()
    .1;
    let mut writer = fixture.writer();
    scan.prepare(FormatVersion::CURRENT, &frames[2], true)
        .unwrap();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut opened = query::open::<MinefieldsObserver>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let w = opened.window(10, 13, None, |_| true).unwrap();
    assert_eq!(
        w.known
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(10, 11), (13, 13)]
    );
    assert_eq!(
        w.series.iter().find(|s| s.name == "GER@7|73").unwrap().runs,
        [(10, 10, "-7".into(), true)]
    );
    let prefix = opened
        .window(10, 13, Some(10), |n| n.ends_with("extent"))
        .unwrap();
    assert_eq!(prefix.series[0].runs, [(10, 10, "100".into(), true)]);
}
#[test]
fn reports_keep_kinds_occurrences_order_and_do_not_use_expiry() {
    let rows = [
        rows::NavalAccidentReportRow {
            kind: 0,
            region: 73,
            ..Default::default()
        },
        rows::NavalAccidentReportRow {
            kind: 1,
            region: 73,
            ordinal: 0,
            damage: -9,
            expire_hour: 999,
            ..Default::default()
        },
        rows::NavalAccidentReportRow {
            kind: 1,
            region: 73,
            ordinal: 1,
            damage: -9,
            expire_hour: 10,
            ..Default::default()
        },
    ];
    let out: Vec<_> = minefields::standing(&rows, 73)
        .map(|r| (r.ordinal, r.damage))
        .collect();
    assert_eq!(out, [(0, -9), (1, -9)]);
}

#[test]
fn supremacy_prepared_inputs_roundtrip_and_match_native_control_and_share() {
    let mut f = supremacy(
        25,
        Some(24),
        &[(11, 400_000), (21, 100_000), (9001, 20_000)],
    );
    put(
        &mut f,
        TableId::WarRelations,
        TableRows::WarRelations(vec![rows::WarRelationRow {
            first_country: 21,
            second_country: 11,
            ..Default::default()
        }]),
    );
    put(
        &mut f,
        TableId::AirCountryRelations,
        TableRows::AirCountryRelations(vec![
            rows::AirCountryRelationRow {
                country: 7,
                kind: 2,
                ordinal: 0,
                other: 21,
            },
            rows::AirCountryRelationRow {
                country: 7,
                kind: 2,
                ordinal: 1,
                other: 21,
            },
        ]),
    );
    let (_, v) = observe::<NavalSupremacyObserver>(&f);
    let values = values(&v);
    let reconstructed = naval_supremacy::restore(1, 25, &values, 24).unwrap();
    let links = f
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let native =
        replay_derive::naval_region_control::Snapshot::read(&f, 7, 73, 10_000, &links).unwrap();
    let mut notes = Refusals::default();
    let out = naval_supremacy::project(
        &reconstructed,
        "GER",
        &BTreeSet::from([73]),
        &labels(10_000, 60_000),
        &mut notes,
    );
    assert_eq!(out["73|friendly"], i128::from(native.friendly));
    assert_eq!(out["73|enemy"], i128::from(native.enemy));
    assert_eq!(out["73|shareNum"], i128::from(native.ratio_friendly));
    assert_eq!(
        out["73|shareDen"],
        i128::from(native.ratio_friendly.wrapping_add(native.ratio_enemy))
    );
    assert_eq!(out["73|own"], 400_000);
    assert_eq!(out["73|control"], 1);
    assert_eq!(out["73|shareDen"], 620_000);
    // The displayed share is the native quotient, from the same operands.
    assert_eq!(out["73|share"], i128::from(native.share().unwrap()));
    assert_eq!(notes.counts("fields"), Vec::<String>::new());
}
#[test]
fn share_failure_keeps_control_and_raw_and_content_failure_keeps_raw_sums() {
    let mut f = supremacy(25, Some(24), &[(11, 100_000)]);
    f.tables[TableId::AirCountryRelations.index()] = TableView::Unavailable {
        reason: "Missing roster".into(),
    }
    .into();
    let mut n = Refusals::default();
    let out =
        naval_supremacy::project(&f, "GER", &BTreeSet::from([73]), &labels(0, 60_000), &mut n);
    assert_eq!(out["73|own"], 100_000);
    assert_eq!(out["73|control"], 1);
    assert!(!out.contains_key("73|shareNum"));
    let out = naval_supremacy::project(
        &f,
        "GER",
        &BTreeSet::from([73]),
        &Err("Missing terrain content".into()),
        &mut n,
    );
    assert_eq!(out["73|own"], 100_000);
    assert_eq!(out["73|friendly"], 100_000);
    assert!(!out.contains_key("73|control"));
}
#[test]
fn family_precedes_war_and_direct_subject_is_not_sibling() {
    let mut f = supremacy(25, Some(24), &[(11, 100_000), (21, 400_000)]);
    put(
        &mut f,
        TableId::FactionMembers,
        TableRows::FactionMembers(vec![
            rows::FactionMemberRow {
                country: 11,
                faction: 9,
                ordinal: 0,
            },
            rows::FactionMemberRow {
                country: 21,
                faction: 9,
                ordinal: 1,
            },
        ]),
    );
    put(
        &mut f,
        TableId::WarRelations,
        TableRows::WarRelations(vec![rows::WarRelationRow {
            first_country: 11,
            second_country: 21,
            ..Default::default()
        }]),
    );
    let links = f
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    assert_eq!(
        replay_derive::naval_region_control::control_sums(&f, 7, 73, &links).unwrap(),
        (500_000, 0)
    );
}
#[test]
fn epoch_gate_no_provenance_midnight_and_midday_refresh() {
    for (hour, sample, known) in [
        (25, Some(24), true),
        (48, Some(24), false),
        (50, Some(50), true),
        (25, None, false),
        (25, Some(26), false),
    ] {
        let f = supremacy(hour, sample, &[(11, 0)]);
        let mut n = Refusals::default();
        let out =
            naval_supremacy::project(&f, "GER", &BTreeSet::from([73]), &labels(0, 60_000), &mut n);
        assert_eq!(out.contains_key("73|own"), known);
        if known {
            assert_eq!(out["73|own"], 0);
        }
    }
}
#[test]
fn daily_chooses_last_recorded_endpoint_even_when_its_inputs_are_unknown() {
    let recorded = vec![
        dashboard_base::products::country_groups::HourSpan {
            first_hour: 10,
            last_hour: 20,
        },
        dashboard_base::products::country_groups::HourSpan {
            first_hour: 22,
            last_hour: 25,
        },
        dashboard_base::products::country_groups::HourSpan {
            first_hour: 50,
            last_hour: 52,
        },
    ];
    let days = dashboard_keyed::daily::Days::new(&recorded, 10, 52);
    assert_eq!(
        days.iter()
            .map(|d| (d.first, d.last, d.hour))
            .collect::<Vec<_>>(),
        [(10, 23, 23), (24, 47, 25), (48, 52, 52)]
    );
    let days = dashboard_keyed::daily::Days::new(&recorded, 10, 22);
    assert_eq!(days.iter().next().unwrap().hour, 22);
}
#[test]
fn country_tag_reuse_is_not_merged() {
    let mut f = supremacy(25, Some(24), &[(11, 1), (21, 2)]);
    let TableView::Complete(TableRows::Countries(c)) = &mut *f.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    for c in c {
        c.tag = b"NEW".to_vec();
    }
    let mut n = Refusals::default();
    let out =
        naval_supremacy::project(&f, "NEW", &BTreeSet::from([73]), &labels(0, 60_000), &mut n);
    assert!(out.is_empty());
    let out = naval_supremacy::project(
        &f,
        "NEW@7",
        &BTreeSet::from([73]),
        &labels(0, 60_000),
        &mut n,
    );
    assert_eq!(out["73|own"], 1);
}

#[test]
fn own_current_is_unknown_while_an_unlinked_current_could_be_the_viewers() {
    let project = |raws: &[(u32, i64)]| {
        let mut notes = Refusals::default();
        let out = naval_supremacy::project(
            &supremacy(25, Some(24), raws),
            "GER",
            &BTreeSet::from([73]),
            &labels(0, 60_000),
            &mut notes,
        );
        (out, notes.counts("fields"))
    };
    // Reference 4242 has no country link, so its current could be GER's.
    let (out, warnings) = project(&[(4242, 100_000)]);
    assert!(!out.contains_key("73|own"), "{out:?}");
    assert!(
        warnings.iter().any(|why| why.contains("own current is unknown")),
        "{warnings:?}"
    );
    // A zero current is no power whoever holds it.
    let (out, warnings) = project(&[(4242, 0)]);
    assert_eq!(out["73|own"], 0);
    assert!(
        !warnings.iter().any(|why| why.contains("own current is unknown")),
        "{warnings:?}"
    );
    // The viewer's own current is read whatever else cannot be named.
    assert_eq!(project(&[(11, 5), (4242, 100_000)]).0["73|own"], 5);
    // Every nonzero current resolved to another country: GER has none.
    assert_eq!(project(&[(21, 100_000)]).0["73|own"], 0);
}

#[test]
fn query_daily_rule_prefix_and_zero_vs_unknown() {
    let frames = vec![
        supremacy(23, Some(0), &[(11, 0)]),
        supremacy(24, Some(0), &[(11, 0)]),
        supremacy(25, Some(0), &[(11, 0)]),
    ];
    let (fixture, _) = publish::<NavalSupremacyObserver>(&frames);
    let replay = replay::container::Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, frames.clone())],
    )
    .unwrap();
    let flag = AtomicBool::new(false);
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "naval-test".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let p = json!({"branchId":"1","tag":"GER","daily":true});
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 23,
            last_hour: 23,
            complete: false,
        },
        None,
    );
    let q = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let reply = naval_supremacy::query(&q).unwrap();
    assert_eq!(reply["dayHours"], 24);
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":23,"lastHour":23}])
    );
    assert_eq!(reply["window"]["series"][0]["name"], "73|enemy");
    let empty = dashboard_base::prepared::series::tests::Fixture::new();
    let q = Query {
        store: &empty.store,
        key: &empty.key,
        ..q
    };
    let cold = naval_supremacy::query(&q).unwrap();
    assert_eq!(cold["window"]["series"], json!([]));
}

#[test]
fn diplomacy_alias_enemy_multiplicity_blockade_and_minimum_are_distinct() {
    let mut f = supremacy(
        25,
        Some(24),
        &[(11, 100_000), (21, 200_000), (9001, 30_000)],
    );
    put(
        &mut f,
        TableId::AirCountryRelations,
        TableRows::AirCountryRelations(vec![
            rows::AirCountryRelationRow {
                country: 7,
                kind: 1,
                ordinal: 0,
                other: 21,
            },
            rows::AirCountryRelationRow {
                country: 7,
                kind: 1,
                ordinal: 1,
                other: 21,
            },
            rows::AirCountryRelationRow {
                country: 7,
                kind: 2,
                ordinal: 0,
                other: 11,
            },
            rows::AirCountryRelationRow {
                country: 7,
                kind: 2,
                ordinal: 1,
                other: 9001,
            },
        ]),
    );
    put(
        &mut f,
        TableId::RelationTuples,
        TableRows::RelationTuples(vec![rows::RelationTupleRow {
            owner_country: 21,
            target_country: 11,
            type_id: 0x27f8,
            first_country: 21,
            second_country: 11,
            ..Default::default()
        }]),
    );
    let (_, v) = observe::<NavalSupremacyObserver>(&f);
    let f = naval_supremacy::restore(1, 25, &values(&v), 24).unwrap();
    let mut notes = Refusals::default();
    let out = naval_supremacy::project(
        &f,
        "GER",
        &BTreeSet::from([73]),
        &labels(500_000, 60_000),
        &mut notes,
    );
    assert_eq!(out["73|friendly"], 130_000);
    assert_eq!(out["73|enemy"], 0);
    assert_eq!(out["73|shareNum"], 330_000);
    assert_eq!(out["73|shareDen"], 830_000);
    assert_eq!(out["73|share"], 330_000 * 100_000 / 830_000);
}

#[test]
fn the_explanatory_share_follows_the_native_quotient_rules() {
    use replay_derive::naval_region_control::share;
    // A zero sum is zero, not a missing value.
    assert_eq!(share(0, 0).unwrap(), 0);
    // The quotient truncates toward zero on the fixed5 scale.
    assert_eq!(share(1, 2).unwrap(), 33_333);
    // A sentinel numerator keeps the native sentinel rather than dividing.
    assert_eq!(share(i64::MAX, 1).unwrap(), 0xffff_ffff);
}
#[test]
fn subject_matching_is_direct_not_siblings_or_transitive() {
    let mut f = supremacy(25, Some(24), &[(11, 100_000), (21, 400_000), (31, 300_000)]);
    let mut countries = f.complete::<tables::Countries>().unwrap().to_vec();
    countries.push(rows::CountryRow {
        id: 31,
        canonical_slot: 9,
        tag: b"MOD".to_vec(),
        ..Default::default()
    });
    countries.sort_by_key(|r| r.id);
    put(&mut f, TableId::Countries, TableRows::Countries(countries));
    let mut links = f.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(rows::CountryLinkRow {
        id: 31,
        canonical_slot: 9,
    });
    links.sort_by_key(|r| r.id);
    put(
        &mut f,
        TableId::CountryLinks,
        TableRows::CountryLinks(links),
    );
    put(
        &mut f,
        TableId::Subjects,
        TableRows::Subjects(vec![
            rows::SubjectRow {
                overlord: 11,
                subject: 21,
                ..Default::default()
            },
            rows::SubjectRow {
                overlord: 11,
                subject: 31,
                ..Default::default()
            },
        ]),
    );
    let links = f
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    assert_eq!(
        replay_derive::naval_region_control::control_sums(&f, 8, 73, &links).unwrap(),
        (500_000, 0)
    );
    assert_eq!(
        replay_derive::naval_region_control::control_sums(&f, 7, 73, &links).unwrap(),
        (800_000, 0)
    );
}
#[test]
fn reduced_reserved_packets_roundtrip_without_strengthening_their_witnesses() {
    let mut f = supremacy(25, Some(24), &[(0, 100_000), (11, 200_000)]);
    put(
        &mut f,
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![rows::ReservedCountryNamingRow {
            tag: b"ZER".to_vec(),
            faction: rows::Recorded::Value(Some(0)),
            reverse_visibility: rows::Recorded::Value(Some(
                rows::ReservedReverseVisibilityInputs {
                    sample_hour: rows::PacketHour::Host,
                    cells: (0..8)
                        .map(|slot| {
                            (slot == 7).then_some(rows::ReverseZeroVisibilityInputs {
                                target: None,
                                war: Some([1, 0]),
                                subject: Some(rows::ZeroSubjectInputs {
                                    relation_type: None,
                                    parties: None,
                                }),
                                ..Default::default()
                            })
                        })
                        .collect(),
                },
            )),
            ..Default::default()
        }]),
    );
    let (_, v) = observe::<NavalSupremacyObserver>(&f);
    let restored = naval_supremacy::restore(1, 25, &values(&v), 24).unwrap();
    let packets = restored
        .complete::<tables::ReservedCountryNaming>()
        .unwrap();
    let original = f.complete::<tables::ReservedCountryNaming>().unwrap();
    assert_eq!(
        packets[0].reverse_visibility,
        original[0].reverse_visibility
    );
    assert_eq!(packets[0].faction, original[0].faction);
    assert_eq!(
        replay_derive::directed_wars::Owner::new(&restored, 0)
            .unwrap()
            .pointer_active_slot(7),
        Ok(true)
    );
    let links = f
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    assert_eq!(
        replay_derive::naval_region_control::control_sums(&restored, 7, 73, &links).unwrap(),
        (200_000, 100_000)
    );
    let mut stale = restored;
    stale.hour = 26;
    stale.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "Reserved host witness unavailable".into(),
    }
    .into();
    assert!(
        replay_derive::directed_wars::Owner::new(&stale, 0)
            .unwrap()
            .pointer_active_slot(7)
            .is_err()
    );
}
#[test]
fn supplying_read_changes_and_unavailable_endpoints_do_not_reuse_earlier_values() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<NavalSupremacyObserver>::default();
    let mut writer = fixture.writer();
    scan.prepare(
        FormatVersion::CURRENT,
        &supremacy(23, Some(0), &[(11, 0)]),
        false,
    )
    .unwrap();
    scan.advance_quiet(24, true).unwrap();
    scan.advance_quiet(25, false).unwrap();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut opened = query::open::<NavalSupremacyObserver>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let w = opened.window(23, 25, None, |_| true).unwrap();
    assert_eq!(w.reads, [(24, 25, 24)]);
    let mut unavailable = supremacy(26, Some(24), &[(11, 0)]);
    unavailable.tables[TableId::NavalSupremacy.index()] = TableView::Unavailable {
        reason: "Unknown daily endpoint".into(),
    }
    .into();
    let mut writer = fixture.writer();
    scan.prepare(FormatVersion::CURRENT, &unavailable, false)
        .unwrap();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut opened = query::open::<NavalSupremacyObserver>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let values = opened.values_at(&[dashboard_keyed::KNOWN], 26).unwrap();
    assert_eq!(values, [None]);
}

#[test]
fn mod_replace_path_seas_names_terrain_and_defines_keep_raw_values() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stack = installed_content_core::test_support::Stack::new("naval_region_mod_content");
    let base = stack.base();
    let addon = stack.addon();
    stack.write(
        &base,
        "map/strategicregions/base.txt",
        "strategic_region={id=73 name=base_sea provinces={1} naval_terrain=base_terrain}",
    );
    stack.write(
        &addon,
        "map/strategicregions/mod.txt",
        "strategic_region={id=73 name=mod_sea provinces={2} naval_terrain=mod_terrain}",
    );
    stack.write(&addon, "map/default.map", r#"definitions="definition.csv" provinces="unused.bmp" terrain="unused.bmp" rivers="unused.bmp" heightmap="unused.bmp" tree_definition="unused.bmp""#);
    stack.write(
        &addon,
        "map/definition.csv",
        "0;0;0;0;land\n1;1;2;3;land\n2;2;3;4;sea\n",
    );
    stack.write(
        &addon,
        "common/terrain/mod.txt",
        "categories={mod_terrain={minimum_seazone_dominance=7}} ",
    );
    stack.write(
        &base,
        "common/defines/00-base.lua",
        "NDefines = { NNavy = { DOMINANCE_CONTROLLED_THRESHOLD_RATIO = 0.6 } }",
    );
    stack.write(
        &addon,
        "common/defines/mod.lua",
        "NDefines.NNavy.DOMINANCE_CONTROLLED_THRESHOLD_RATIO = 0.7",
    );
    stack.write(
        &addon,
        "localisation/english/replace/mod_l_english.yml",
        "\u{feff}l_english:\n mod_sea: \"Authored mod sea\"\n",
    );
    let frames = vec![supremacy(24, Some(24), &[(11, 100_000)])];
    let _ = test_fixtures::source_with_content(frames.clone(), &base);
    stack.write(
        &base,
        "user/mod/naval.mod",
        &format!(
            "name=\"Naval fixture\" path=\"{}\" replace_path=\"map/strategicregions\"",
            addon.to_string_lossy().replace('\\', "/")
        ),
    );
    stack.write(
        &base,
        "user/dlc_load.json",
        r#"{"enabled_mods":["mod/naval.mod"],"disabled_dlcs":[]}"#,
    );
    let (options, identity) = test_fixtures::content_identity(&base);
    let mut records = test_fixtures::records(1, frames.clone());
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    let replay =
        replay::container::Replay::from_records(test_fixtures::CAMPAIGN.into(), &[records])
            .unwrap();
    let flag = AtomicBool::new(false);
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "naval-mod".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let (fixture, _) = publish::<NavalSupremacyObserver>(&frames);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 24,
            last_hour: 24,
            complete: true,
        },
        None,
    );
    let params = json!({"branchId":"1","tag":"GER","daily":true});
    let q = Query {
        source: &source,
        params: &params,
        content: Some(&options),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let labels = crate::shared::labels(&q).unwrap();
    assert_eq!(labels.regions.as_ref().unwrap().name(73).as_deref(), Ok("Authored mod sea"));
    assert_eq!(labels.sea_regions, Ok(BTreeSet::from([73])));
    assert_eq!(labels.minima[&73], Ok(700_000));
    assert_eq!(labels.ratio, Ok(70_000));
    let reply = naval_supremacy::query(&q).unwrap();
    assert_eq!(reply["regions"][0]["name"], "Authored mod sea");
    let own = reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "73|own")
        .unwrap();
    assert_eq!(own["runs"], json!([[24, 24, "100000", true]]));
}

fn source_for_queries(frames: Vec<DecodedFrame>) -> dashboard_base::Source {
    let replay = replay::container::Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, frames)],
    )
    .unwrap();
    dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "naval-query-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn standing_report_query_is_independent_of_preparation_and_reuses_full_handle_frame_cache() {
    use dashboard_base::prepared::query::{frames, progressive};
    let mut frame = world_frame(25, false);
    frame.metrics.table_ns[TableId::NavalAccidentReports.index()] = 1;
    put(
        &mut frame,
        TableId::NavalAccidentReports,
        TableRows::NavalAccidentReports(vec![
            rows::NavalAccidentReportRow {
                country: 11,
                kind: 0,
                region: 73,
                ..Default::default()
            },
            rows::NavalAccidentReportRow {
                country: 11,
                kind: 1,
                ordinal: 0,
                region: 73,
                owner: 9001,
                mine_owner: 21,
                damage: -7,
                ship: (9_u64 << 32) | 5,
                equipment: 777,
                expire_hour: 999,
                ..Default::default()
            },
            rows::NavalAccidentReportRow {
                country: 11,
                kind: 1,
                ordinal: 1,
                region: 73,
                owner: 9001,
                mine_owner: 21,
                damage: -7,
                sunk: 1,
                expire_hour: 10,
                ..Default::default()
            },
        ]),
    );
    // A name for a recycled partial handle must not name the retained ship.
    put(
        &mut frame,
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(vec![rows::CountryCounterInputRow {
            country: 9001,
            pride_of_the_fleet: (8_u64 << 32) | 5,
            pride_name: rows::Recorded::Value(Some(b"Other ship".to_vec())),
            ..Default::default()
        }]),
    );
    let source = source_for_queries(vec![frame]);
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 25,
            last_hour: 24,
            complete: false,
        },
        None,
    );
    let p = json!({"branchId":"1","view":"reports","hour":25,"region":73});
    let flag = AtomicBool::new(false);
    let q = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let reply = minefields::query(&q).unwrap();
    assert_eq!(reply["hour"], 25);
    assert_eq!(reply["region"], 73);
    assert_eq!(reply["readHour"], 25);
    assert_eq!(
        reply["reports"],
        json!([
            {"kind":1,"ordinal":0,"victim":"GER@7","mineOwner":"ITA@8","ship":null,
             "equipment":null,"damage":"-7","sunk":false,"firstSeen":null},
            {"kind":1,"ordinal":1,"victim":"GER@7","mineOwner":"ITA@8","ship":null,
             "equipment":null,"damage":"-7","sunk":true,"firstSeen":null}
        ])
    );
    assert!(!reply.as_object().unwrap().contains_key("window"));
    assert!(!reply["warnings"].as_array().unwrap().is_empty());
    let reads = frames::reads(&source);
    let (again, decoded) = progressive::without_block_decodes(|| minefields::query(&q));
    assert!(!decoded);
    assert_eq!(again.unwrap(), reply);
    assert_eq!(frames::reads(&source), reads);
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        minefields::query(&q).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn whole_daily_query_refuses_unknown_endpoint_and_midnight_until_midday_refresh() {
    let mut frames: Vec<_> = (22..=50)
        .map(|h| supremacy(h, Some(if h == 50 { 50 } else { 0 }), &[(11, 123)]))
        .collect();
    frames[1].tables[TableId::NavalSupremacy.index()] = TableView::Unavailable {
        reason: "The day's endpoint was not captured".into(),
    }
    .into();
    let (fixture, _) = publish::<NavalSupremacyObserver>(&frames);
    let source = source_for_queries(frames);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 22,
            last_hour: 50,
            complete: true,
        },
        None,
    );
    let p = json!({"branchId":"1","tag":"GER","daily":true});
    let flag = AtomicBool::new(false);
    let q = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let reply = naval_supremacy::query(&q).unwrap();
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":48,"lastHour":50}])
    );
    assert_eq!(reply["window"]["reads"], json!([[48, 50, 50]]));
    let own = reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "73|own")
        .unwrap();
    assert_eq!(own["runs"], json!([[48, 50, "123", true]]));
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&source),
        (0, 0)
    );
    let bad = json!({"branchId":"1","tag":"GER","daily":false});
    assert!(naval_supremacy::query(&Query { params: &bad, ..q }).is_err());
}

#[test]
fn reserved_viewer_and_refused_reverse_packet_keep_independent_raw_currents() {
    let mut frame = supremacy(25, Some(24), &[(0, 100_000), (11, 200_000)]);
    put(
        &mut frame,
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![rows::ReservedCountryNamingRow {
            tag: b"ZER".to_vec(),
            faction: rows::Recorded::Value(Some(0)),
            reverse_visibility: rows::Recorded::Value(None),
            ..Default::default()
        }]),
    );
    let (_, prepared) = observe::<NavalSupremacyObserver>(&frame);
    assert_eq!(values(&prepared).get("ZER@0|73|raw:0"), Some(&100_000));
    let restored = naval_supremacy::restore(1, 25, &values(&prepared), 24).unwrap();
    let mut notes = Refusals::default();
    let out = naval_supremacy::project(
        &restored,
        "ZER@0",
        &BTreeSet::from([73]),
        &labels(0, 60_000),
        &mut notes,
    );
    assert_eq!(out["73|own"], 100_000);
    assert!(!out.contains_key("73|control"));
    let out = naval_supremacy::project(
        &restored,
        "GER",
        &BTreeSet::from([73]),
        &labels(0, 60_000),
        &mut notes,
    );
    assert_eq!(out["73|own"], 200_000);
    assert!(!out.contains_key("73|control"));
    assert!(!notes.counts("fields").is_empty());
}

#[test]
fn negative_wrapped_control_and_friendly_ties_match_shared_native_arithmetic() {
    for amounts in [
        vec![(11, 300_000), (21, 300_000)],
        vec![(11, -7), (21, -19)],
        vec![(11, i64::MAX), (21, 10), (9001, 15)],
    ] {
        let mut frame = supremacy(25, Some(24), &amounts);
        put(
            &mut frame,
            TableId::WarRelations,
            TableRows::WarRelations(vec![rows::WarRelationRow {
                first_country: 21,
                second_country: 11,
                ..Default::default()
            }]),
        );
        let links = frame
            .complete::<tables::CountryLinks>()
            .unwrap()
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let native =
            replay_derive::naval_region_control::Snapshot::read(&frame, 7, 73, 0, &links).unwrap();
        let (_, prepared) = observe::<NavalSupremacyObserver>(&frame);
        let restored = naval_supremacy::restore(1, 25, &values(&prepared), 24).unwrap();
        let mut notes = Refusals::default();
        let out = naval_supremacy::project(
            &restored,
            "GER",
            &BTreeSet::from([73]),
            &labels(0, 60_000),
            &mut notes,
        );
        assert_eq!(out["73|friendly"], i128::from(native.friendly));
        assert_eq!(out["73|enemy"], i128::from(native.enemy));
        assert_eq!(
            out["73|control"],
            match native.control(0, 60_000).unwrap() {
                1 => 1,
                2 => -1,
                _ => 0,
            }
        );
        if amounts[0].1 == 300_000 {
            assert_eq!(out["73|control"], 1);
        }
    }
}

#[test]
fn reply_budget_refuses_instead_of_truncating_and_cancel_precedes_history_reads() {
    let source = source_for_queries(vec![supremacy(24, Some(24), &[(11, 0)])]);
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 24,
            last_hour: 24,
            complete: true,
        },
        None,
    );
    let p = json!({"branchId":"1","tag":"GER","daily":true});
    let flag = AtomicBool::new(false);
    let q = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let large = naval_supremacy::Reply {
        window: crate::shared::empty(24, 24),
        day_hours: 24,
        regions: vec![],
        scale: 100_000,
        warnings: vec!["x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES)],
    };
    assert!(
        crate::shared::finish(&q, &large)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        naval_supremacy::query(&q).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn equal_fresh_read_at_midnight_renews_daily_query_without_changing_current() {
    let frames: Vec<_> = (23..=25)
        .map(|h| supremacy(h, Some(0), &[(11, 0)]))
        .collect();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<NavalSupremacyObserver>::default();
    scan.prepare(FormatVersion::CURRENT, &frames[0], false)
        .unwrap();
    scan.advance_quiet(24, true).unwrap();
    scan.advance_quiet(25, false).unwrap();
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let source = source_for_queries(frames);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 23,
            last_hour: 25,
            complete: true,
        },
        None,
    );
    let p = json!({"branchId":"1","tag":"GER","daily":true});
    let q = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let reply = naval_supremacy::query(&q).unwrap();
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":23,"lastHour":25}])
    );
    assert_eq!(reply["window"]["reads"], json!([[23, 23, 0], [24, 25, 24]]));
    let own = reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "73|own")
        .unwrap();
    assert_eq!(own["runs"], json!([[23, 25, "0", true]]));
}

#[test]
fn a_selected_country_query_keeps_owners_that_could_not_be_named() {
    // GER's vector is admitted; reference 4242 has no country link, so its owner
    // might be GER and the inventory is incomplete, not empty.
    let frames = vec![mines(
        10,
        vec![mine(11, 0, 100, 0), mine(4242, 0, 2, 0), mine(4242, 1, 1, 5)],
    )];
    let (fixture, _) = publish::<MinefieldsObserver>(&frames);
    let source = source_for_queries(frames);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 10,
            last_hour: 10,
            complete: true,
        },
        None,
    );
    let flag = AtomicBool::new(false);
    let p = json!({"branchId":"1","view":"history","tags":["GER"]});
    let q = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let reply = minefields::query(&q).unwrap();
    let names: Vec<_> = reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|series| series["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"?|refused".to_owned()), "{names:?}");
    assert!(names.contains(&"GER@7|extent".to_owned()), "{names:?}");
    // Another selected country is filtered as before.
    let p = json!({"branchId":"1","view":"history","tags":["ITA"]});
    let q = Query { params: &p, ..q };
    let reply = minefields::query(&q).unwrap();
    let names: Vec<_> = reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|series| series["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["?|refused"]);
}

/// `frames` published in batches ending at each of `cuts`, as preparation commits.
fn publish_in<O: Observer>(
    frames: &[DecodedFrame],
    cuts: &[usize],
) -> dashboard_base::prepared::series::tests::Fixture {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    let mut start = 0;
    for &end in cuts.iter().chain([frames.len()].iter()) {
        let mut writer = fixture.writer();
        for f in &frames[start..end] {
            scan.prepare(
                FormatVersion::CURRENT,
                f,
                f.tables[O::READ.index()].sampled_at() == Some(f.hour),
            )
            .unwrap();
        }
        scan.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        start = end;
    }
    fixture
}

#[test]
fn swept_days_reply_exactly_as_each_day_restored_whole() {
    // Five days of hourly frames, each day read at midnight but the fourth, which keeps
    // the third's read: currents change, a country leaves and returns, a war starts
    // on the third day and an air relation joins on the fifth.
    let frames: Vec<DecodedFrame> = (0..5 * 24)
        .map(|hour| {
            let day = hour / 24;
            let raws: &[(u32, i64)] = match day {
                0 => &[(11, 100_000)],
                1 => &[(11, 200_000), (21, 50_000)],
                2 | 3 => &[(21, 50_000)],
                _ => &[(11, 300_000), (21, 70_000)],
            };
            let read = 24 * if day == 3 { 2 } else { day };
            let mut f = supremacy(hour, Some(read), raws);
            if day >= 2 {
                put(
                    &mut f,
                    TableId::WarRelations,
                    TableRows::WarRelations(vec![rows::WarRelationRow {
                        first_country: 21,
                        second_country: 11,
                        ..Default::default()
                    }]),
                );
            }
            if day == 4 {
                put(
                    &mut f,
                    TableId::AirCountryRelations,
                    TableRows::AirCountryRelations(vec![rows::AirCountryRelationRow {
                        country: 7,
                        kind: 1,
                        ordinal: 0,
                        other: 21,
                    }]),
                );
            }
            if hour == 24 + 5 {
                f.tables[TableId::NavalSupremacy.index()] = TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into();
            }
            f
        })
        .collect();
    // Batches end within days, so a day's values are read across them.
    let fixture = publish_in::<NavalSupremacyObserver>(&frames, &[30, 75]);
    let source = source_for_queries(frames);
    let flag = AtomicBool::new(false);
    for (through, params) in [
        (119, json!({"branchId":"1","tag":"GER","daily":true})),
        (
            119,
            json!({"branchId":"1","tag":"GER","daily":true,"firstHour":40,"lastHour":100}),
        ),
        (90, json!({"branchId":"1","tag":"GER","daily":true})),
    ] {
        let prefix = Prefix::new(
            &source,
            1,
            Coverage {
                schema: 1,
                first_hour: 0,
                last_hour: through,
                complete: through == 119,
            },
            None,
        );
        let q = Query {
            source: &source,
            params: &params,
            content: None,
            store: &fixture.store,
            key: &fixture.key,
            branch: 1,
            prefix: &prefix,
            cancelled: &flag,
        };
        let swept = naval_supremacy::query(&q).unwrap();
        let whole = naval_supremacy::reply(&q, naval_supremacy::lookup).unwrap();
        assert_eq!(swept, whole);
        let own = swept["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "73|own")
            .unwrap();
        assert!(own["runs"].as_array().unwrap().len() > 1, "{own}");
    }
}

mod changes;
mod reference;
