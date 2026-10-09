use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// A frame of Germany (slot 7, native id `germany`, named by its other link 11 in
/// the factory table) and Italy (slot 8). `read` times the factory walk.
fn factory_frame(hour: i32, read: bool, germany: u32, civilian_total_raw: i64) -> DecodedFrame {
    use replay::rows::CountryFactoriesRow;
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 8),
            link(germany, 7),
        ]));
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(vec![
            CountryFactoriesRow {
                country: 11,
                military_total_raw: 1_299_999,
                military_controlled_raw: 1_200_000,
                military_owned_raw: 1_000_000,
                military_damaged_raw: 199_999,
                military_unavailable: 3,
                military_to_overlord: 1,
                military_to_others: 2,
                civilian_total_raw,
                civilian_controlled_raw: civilian_total_raw,
                civilian_owned_raw: civilian_total_raw,
                civilian_damaged_raw: -150_000,
                civilian_unavailable: 15,
                consumer_goods: 5,
                contract_payments: 1,
                special_projects: 2,
                license_imports: 3,
                trade_imports: 4,
                civilian_trade_exports: 6,
                civilian_license_exports: 7,
                ..Default::default()
            },
            CountryFactoriesRow {
                country: 21,
                naval_total_raw: 500_000,
                // A withheld factory its parts do not explain.
                naval_unavailable: 1,
                ..Default::default()
            },
        ]));
    if read {
        frame.metrics.table_ns[TableId::CountryFactories.index()] = 1;
    }
    frame
}

#[test]
fn factories_are_truncated_as_the_game_shows_them_and_runs_never_cross_gaps_or_identities() {
    use crate::products::country_factories::{
        FactoryField, FactoryGroupKey, FactoryPart, fields, scan,
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                factory_frame(100, true, 9001, 2_000_000),
                factory_frame(101, true, 9001, 2_099_999),
                // The same whole count on an hourly read with no timing charged.
                factory_frame(102, false, 9001, 2_099_999),
                factory_frame(103, true, 9001, 2_100_000),
                // Hour 104 is missing: the next run starts a new line.
                factory_frame(105, true, 9001, 2_100_000),
                // Another country takes the tag.
                factory_frame(106, true, 9002, 2_100_000),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let fields = fields();
    assert_eq!(fields.len(), 31);
    let at = |group, part| {
        fields
            .iter()
            .position(|field| *field == FactoryField { group, part })
            .unwrap()
    };
    let spans = |tag: &str| -> Vec<(i32, i32, bool)> {
        scanned.series[tag]
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect()
    };
    // 2_000_000 and 2_099_999 both truncate to 20 factories: one run.
    assert_eq!(
        spans("GER"),
        [
            (100, 102, true),
            (103, 103, false),
            (105, 105, true),
            (106, 106, true)
        ]
    );
    let germany = &scanned.series["GER"];
    let identity_spans: Vec<_> = scanned.segments["GER"]
        .iter()
        .map(|span| (span.first_hour, span.last_hour))
        .collect();
    assert_eq!(identity_spans, [(100, 103), (105, 105), (106, 106)]);
    let values = &germany[0].values;
    use FactoryGroupKey::{Civilian, Military};
    assert_eq!(values[at(Military, FactoryPart::Total)], 12);
    assert_eq!(values[at(Military, FactoryPart::Controlled)], 12);
    assert_eq!(values[at(Military, FactoryPart::Owned)], 10);
    assert_eq!(values[at(Military, FactoryPart::Damaged)], 1);
    assert_eq!(values[at(Military, FactoryPart::ToOverlord)], 1);
    assert_eq!(values[at(Military, FactoryPart::ToOthers)], 2);
    assert_eq!(values[at(Civilian, FactoryPart::Total)], 20);
    // Toward zero, as (int)(x / 100000) is.
    assert_eq!(values[at(Civilian, FactoryPart::Damaged)], -1);
    assert_eq!(values[at(Civilian, FactoryPart::ConsumerGoods)], 5);
    assert_eq!(values[at(Civilian, FactoryPart::ContractPayments)], 1);
    assert_eq!(values[at(Civilian, FactoryPart::SpecialProjects)], 2);
    assert_eq!(values[at(Civilian, FactoryPart::LicenseImports)], 3);
    assert_eq!(values[at(Civilian, FactoryPart::TradeImports)], 4);
    assert_eq!(values[at(Civilian, FactoryPart::TradeExports)], 6);
    assert_eq!(values[at(Civilian, FactoryPart::LicenseExports)], 7);
    assert_eq!(germany[1].values[at(Civilian, FactoryPart::Total)], 21);
    // Italy: one unbroken run, except across the missing hour 104.
    assert_eq!(spans("ITA"), [(100, 103, true), (105, 106, true)]);
    // Italy's withheld dockyard is unexplained at each of its six read hours.
    assert_eq!(
        scanned.unexplained,
        std::collections::BTreeMap::from([(FactoryGroupKey::Naval, 6)])
    );
    assert_eq!(scanned.unreadable, 0);
}

#[test]
fn factories_leave_out_rows_no_single_country_owns_and_hours_the_tables_cannot_tell() {
    use crate::products::country_factories::scan;
    let mut duplicated = factory_frame(101, true, 9001, 2_000_000);
    if let TableView::Complete(TableRows::CountryFactories(rows)) =
        &mut *duplicated.tables[TableId::CountryFactories.index()]
    {
        // Germany named twice (by both its links), and a row no link resolves.
        let mut again = rows[0].clone();
        again.country = 9001;
        rows.push(again);
        let mut stray = rows[1].clone();
        stray.country = 77;
        rows.push(stray);
        rows.sort_by_key(|row| row.country);
    }
    let mut incomplete = factory_frame(102, true, 9001, 2_000_000);
    *incomplete.tables[TableId::CountryFactories.index()] = TableView::Unavailable {
        reason: "not walked".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                factory_frame(100, true, 9001, 2_000_000),
                duplicated,
                incomplete,
                factory_frame(103, true, 9001, 2_000_000),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let spans = |tag: &str| -> Vec<(i32, i32, bool)> {
        scanned.series[tag]
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect()
    };
    assert_eq!(spans("GER"), [(100, 100, true), (103, 103, true)]);
    assert_eq!(spans("ITA"), [(100, 101, true), (103, 103, true)]);
    // Missing factory values do not split identities known from Countries/Links.
    for tag in ["GER", "ITA"] {
        assert_eq!(
            scanned.segments[tag],
            [crate::products::country_groups::HourSpan {
                first_hour: 100,
                last_hour: 103
            }]
        );
    }
    assert_eq!(scanned.unreadable, 2);
}

#[test]
fn factory_groups_are_named_by_the_content_buildings_their_flags_count() {
    use crate::products::country_factories::{FactoryGroupKey, group_buildings};
    use installed_content::buildings::{Buildings, FactoryFlags};
    use std::collections::{BTreeMap, BTreeSet};
    let flags = |military_production, naval_production, general_production| FactoryFlags {
        military_production,
        naval_production,
        general_production,
        ..Default::default()
    };
    let buildings = Buildings {
        factory_flags: BTreeMap::from([
            ("arms".to_owned(), flags(1, 0, 0)),
            ("civ".to_owned(), flags(0, 0, 1)),
            ("yard".to_owned(), flags(0, 2, 0)),
            ("both".to_owned(), flags(1, 0, 1)),
            ("odd".to_owned(), flags(0, 0, 1)),
            ("depot".to_owned(), flags(0, -1, 0)),
        ]),
        declaration_order: ["civ", "arms", "yard", "both", "odd", "depot"]
            .map(str::to_owned)
            .to_vec(),
        ambiguous_factory_types: BTreeSet::from(["odd".to_owned()]),
        ..Default::default()
    };
    let groups = group_buildings(&buildings, |token| {
        (token != "both").then(|| format!("Name of {token}"))
    });
    let listed = |key| -> Vec<(String, Option<String>)> {
        groups
            .iter()
            .find(|group| group.key == key)
            .unwrap()
            .buildings
            .iter()
            .map(|building| (building.token.clone(), building.name.clone()))
            .collect()
    };
    let named = |token: &str| (token.to_owned(), Some(format!("Name of {token}")));
    assert_eq!(
        listed(FactoryGroupKey::Military),
        [named("arms"), ("both".to_owned(), None)]
    );
    assert_eq!(
        listed(FactoryGroupKey::Civilian),
        [named("civ"), ("both".to_owned(), None)]
    );
    assert_eq!(listed(FactoryGroupKey::Naval), [named("yard")]);
    assert_eq!(groups[0].flag, "military_production");
    let long_name = "Factory name ".repeat(30);
    let long = group_buildings(&buildings, |_| Some(long_name.clone()));
    assert_eq!(
        long[0].buildings[0].name.as_deref(),
        Some(long_name.as_str())
    );
}

#[test]
fn factory_types_are_answered_without_a_scan_and_concurrent_requests_share_one_scan() {
    use crate::products::{
        LoadContext, Product,
        country_factories::{
            CountryFactories, FactoriesParams, FactoryCount, FactoryGroupKey, FactoryTop, scans_run,
        },
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                (100..=160)
                    .map(|hour| {
                        factory_frame(hour, true, 9001, 2_000_000 + i64::from(hour) * 100_000)
                    })
                    .collect(),
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "factories-flight".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    // The types alone: from content, no scan, even when the request is cancelled.
    let types = CountryFactories::load(
        &source,
        &FactoriesParams {
            branch_id: "1".into(),
            tags: vec![],
            top: None,
        },
        &context,
        &AtomicBool::new(true),
    )
    .unwrap();
    assert_eq!(types.groups.len(), 3);
    assert!(types.series.is_empty());
    assert_eq!(scans_run(&source), 0);
    // Three widgets with different top N on the uncached branch: one scan between them.
    let top = |count| FactoriesParams {
        branch_id: "1".into(),
        tags: vec![],
        top: Some(FactoryTop {
            count,
            mode: FactoryCount::Built,
            groups: vec![FactoryGroupKey::Civilian],
        }),
    };
    let replies: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = [1, 2, 3]
            .map(|count| {
                let (source, context) = (&source, &context);
                scope.spawn(move || {
                    CountryFactories::load(source, &top(count), context, &AtomicBool::new(false))
                        .unwrap()
                })
            })
            .into_iter()
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });
    assert_eq!(scans_run(&source), 1);
    assert_eq!(replies[0].series.len(), 1);
    assert_eq!(replies[1].series.len(), 2);
}

#[test]
fn factories_reply_the_countries_asked_for_without_content_and_answer_again_from_the_cache() {
    use crate::products::{
        LoadContext, Product,
        country_factories::{
            CountryFactories, FactoriesParams, FactoryCount, FactoryGroupKey, FactoryTop,
        },
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                (100..=103)
                    .map(|hour| factory_frame(hour, true, 9001, 2_000_000))
                    .collect(),
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "factories".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = |tags: &[&str], top: Option<FactoryTop>| FactoriesParams {
        branch_id: "1".into(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        top,
    };
    let top = |count, groups: Vec<FactoryGroupKey>| FactoryTop {
        count,
        mode: FactoryCount::Free,
        groups,
    };
    let civilian = || vec![FactoryGroupKey::Civilian];
    assert!(CountryFactories::validate(&params(&["ITA", "GER"], None)).is_ok());
    assert!(CountryFactories::validate(&params(&[], Some(top(10, civilian())))).is_ok());
    assert!(CountryFactories::validate(&params(&[], None)).is_ok());
    // No groups named sums all three.
    assert!(CountryFactories::validate(&params(&[], Some(top(3, vec![])))).is_ok());
    for invalid in [
        FactoriesParams {
            branch_id: "x".into(),
            ..params(&[], None)
        },
        params(&[""], None),
        params(&["GER"], Some(top(10, civilian()))),
        params(&[], Some(top(0, civilian()))),
        params(
            &[],
            Some(top(3, vec![FactoryGroupKey::Naval, FactoryGroupKey::Naval])),
        ),
    ] {
        assert!(CountryFactories::validate(&invalid).is_err(), "{invalid:?}");
    }
    let load = |params: &FactoriesParams| {
        CountryFactories::load(
            &source,
            params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let chosen = params(&["ITA", "XXX", "ITA"], None);
    let reply = load(&chosen);
    let tags: Vec<&str> = reply
        .series
        .iter()
        .map(|series| series.tag.as_str())
        .collect();
    // Only what was asked, once each, in the order asked; a tag not recorded is said.
    assert_eq!(tags, ["ITA"]);
    assert!(reply.series[0].membership.is_none());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX: no recorded factories"))
    );
    // The groups alone, for a settings list.
    assert!(load(&params(&[], None)).series.is_empty());
    // Top 1 by free civilian factories: Germany (20 − (−1) − 15 = 6) over Italy (0).
    let leader = load(&params(&[], Some(top(1, civilian()))));
    assert_eq!(leader.series.len(), 1);
    assert_eq!(leader.series[0].tag, "GER");
    assert_eq!(
        leader.series[0].membership.as_ref().unwrap()[0].first_hour,
        100
    );
    assert_eq!(reply.fields.len(), 31);
    assert!(reply.groups.iter().all(|group| group.buildings.is_empty()));
    let flags: Vec<&str> = reply
        .groups
        .iter()
        .map(|group| group.flag.as_str())
        .collect();
    assert_eq!(
        flags,
        [
            "military_production",
            "naval_production",
            "general_production"
        ]
    );
    let warned = |start: &str| {
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with(start))
    };
    assert!(
        warned("Factory type names are unavailable"),
        "{:?}",
        reply.warnings
    );
    assert!(
        warned("4 country-hours record naval_production"),
        "{:?}",
        reply.warnings
    );
    assert_eq!(load(&chosen).series, reply.series);
}

/// Values with a group's total and damaged set; every other part zero.
fn factory_values(group: usize, total: i32, damaged: i32) -> Vec<i32> {
    let mut values = vec![0; 31];
    values[group * 8] = total;
    values[group * 8 + 3] = damaged;
    values
}

fn factory_run(
    first_hour: i32,
    last_hour: i32,
    values: Vec<i32>,
) -> crate::products::country_factories::FactoryRun {
    crate::products::country_factories::FactoryRun {
        first_hour,
        last_hour,
        starts_segment: false,
        values,
    }
}

#[test]
fn factory_counts_follow_the_decided_formulas() {
    use crate::products::country_factories::{FactoryCount, FactoryGroupKey, group_count};
    let mut values = factory_values(2, 40, 3);
    // Civilian given to the overlord and others, and the five consumers.
    values[2 * 8 + 5] = 1;
    values[2 * 8 + 7] = 2;
    for (offset, taken) in [(26, 5), (27, 1), (28, 2), (29, 3), (30, 4)] {
        values[offset] = taken;
    }
    // Trade and licence exports are in the total already and withhold nothing.
    values[24] = 6;
    values[25] = 7;
    values[5] = 9;
    let civilian = |mode| group_count(&values, FactoryGroupKey::Civilian, mode);
    assert_eq!(civilian(FactoryCount::Built), 40);
    assert_eq!(civilian(FactoryCount::TopBar), 40 - 3 - 3);
    assert_eq!(civilian(FactoryCount::Free), 40 - 3 - 3 - 15);
    // Military: built − damaged − given away, with nothing else withheld.
    assert_eq!(
        group_count(&values, FactoryGroupKey::Military, FactoryCount::Free),
        -9
    );
}

#[test]
fn top_n_is_ranked_at_every_change_and_cut_to_the_hours_each_country_was_in_it() {
    use crate::products::{
        country_factories::{
            FactoryCount, FactoryGroupKey, FactoryTop, ScannedFactories, clip_runs, top_membership,
        },
        country_groups::HourSpan,
    };
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    let civilian = |total| factory_values(2, total, 0);
    let mut scanned = ScannedFactories::default();
    scanned.series.insert(
        "AAA".into(),
        vec![
            factory_run(100, 109, civilian(5)),
            factory_run(110, 119, civilian(20)),
        ],
    );
    // Recorded only from 105; ties with AAA at 5 until 110, and AAA wins by tag.
    scanned
        .series
        .insert("BBB".into(), vec![factory_run(105, 119, civilian(5))]);
    // Leads until it stops being recorded after 107, then returns at 112.
    scanned.series.insert(
        "CCC".into(),
        vec![
            factory_run(100, 107, civilian(50)),
            factory_run(112, 119, civilian(1)),
        ],
    );
    let top = |count| FactoryTop {
        count,
        mode: FactoryCount::Built,
        groups: vec![FactoryGroupKey::Civilian],
    };
    let members = top_membership(&scanned.series, &top(2), &AtomicBool::new(false)).unwrap();
    assert_eq!(members["CCC"], [span(100, 107)]);
    assert_eq!(members["AAA"], [span(100, 119)]);
    // BBB is third while CCC is recorded, and second from 108 (CCC is 1 from 112).
    assert_eq!(members["BBB"], [span(108, 119)]);
    // No groups named sums them all: the same here, only the civilian group is set.
    let all = FactoryTop {
        groups: vec![],
        ..top(2)
    };
    assert_eq!(
        top_membership(&scanned.series, &all, &AtomicBool::new(false)).unwrap(),
        members
    );
    let one = top_membership(&scanned.series, &top(1), &AtomicBool::new(false)).unwrap();
    assert_eq!(one["CCC"], [span(100, 107)]);
    assert_eq!(one["AAA"], [span(108, 119)]);
    assert!(!one.contains_key("BBB"));
    // Runs are cut to the hours; a piece keeps a line break only where its run starts.
    let mut broken = factory_run(100, 109, civilian(5));
    broken.starts_segment = true;
    let clipped = clip_runs(
        &[broken, factory_run(110, 119, civilian(20))],
        &[span(105, 112)],
        &AtomicBool::new(false),
    )
    .unwrap();
    let pieces: Vec<_> = clipped
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
        .collect();
    assert_eq!(pieces, [(105, 109, false), (110, 112, false)]);
    // Several spans over several runs, some runs outside every span.
    let mut starts = factory_run(100, 102, civilian(1));
    starts.starts_segment = true;
    let clipped = clip_runs(
        &[
            starts,
            factory_run(103, 109, civilian(2)),
            factory_run(115, 120, civilian(3)),
        ],
        &[span(100, 101), span(104, 106), span(108, 116)],
        &AtomicBool::new(false),
    )
    .unwrap();
    let pieces: Vec<_> = clipped
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
        .collect();
    assert_eq!(
        pieces,
        [
            (100, 101, true),
            (104, 106, false),
            (108, 109, false),
            (115, 116, false)
        ]
    );
}

/// Two countries alternating first place every hour for `hours` hours: the leader's
/// runs and its top-1 spans, one every other hour.
fn alternating(
    hours: i32,
) -> (
    Vec<crate::products::country_factories::FactoryRun>,
    Vec<crate::products::country_groups::HourSpan>,
) {
    let runs = (0..hours / 2)
        .map(|index| factory_run(index * 2, index * 2, factory_values(2, index, 0)))
        .collect();
    let spans = (0..hours / 2)
        .map(|index| crate::products::country_groups::HourSpan {
            first_hour: index * 2,
            last_hour: index * 2,
        })
        .collect();
    (runs, spans)
}

#[test]
fn top_n_clipping_follows_hourly_lead_changes_and_can_be_cancelled() {
    use crate::products::country_factories::clip_runs;
    // The case from review: 60,000 hours, first place changing every hour.
    let (runs, spans) = alternating(60_000);
    let clipped = clip_runs(&runs, &spans, &AtomicBool::new(false)).unwrap();
    assert_eq!(clipped.len(), 30_000);
    assert!(
        clipped
            .iter()
            .zip(&runs)
            .all(|(piece, run)| piece.first_hour == run.first_hour)
    );
    // A cancelled request stops.
    let error = clip_runs(&runs, &spans, &AtomicBool::new(true)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

/// Writes `files` (relative path, text) under a fresh folder named for `name`.
fn content_folder(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

#[test]
fn factory_types_follow_a_modded_content_stack() {
    use crate::{
        catalogue::Catalogue,
        content_check::Kind,
        products::country_factories::{
            FactoryGroupKey, content_groups, describe_groups_with_findings,
        },
    };
    let vanilla = content_folder(
        "factories_vanilla",
        &[
            (
                "common/buildings/00_buildings.txt",
                "buildings = { arms_factory = { military_production = 1 } industrial_complex = { general_production = 1 } dockyard = { naval_production = 1 } infrastructure = { } }",
            ),
            (
                "localisation/english/buildings_l_english.yml",
                "\u{feff}l_english:\n arms_factory:0 \"Military Factory\"\n industrial_complex:0 \"Civilian Factory\"\n dockyard:0 \"Naval Dockyard\"\n",
            ),
        ],
    );
    // The mod replaces the building folder: no dockyards, a factory type of its own
    // counted as military, one it forgot to name, and the military factory renamed.
    let modded = content_folder(
        "factories_mod",
        &[
            (
                "common/buildings/mod_buildings.txt",
                "buildings = { arms_factory = { military_production = 1 } tank_works = { military_production = 1 rocket_production = 2 } industrial_complex = { general_production = 1 } workshop = { general_production = 1 rocket_production = -1 } launch_site = { rocket_launch_capacity = 3 rocket_production = 0 } }",
            ),
            (
                "localisation/english/mod_l_english.yml",
                "\u{feff}l_english:\n tank_works:0 \"Tank Works\"\n",
            ),
            (
                "localisation/replace/english/mod_replace_l_english.yml",
                "\u{feff}l_english:\n arms_factory:0 \"Arms Plant\"\n",
            ),
        ],
    );
    let layers = installed_content::ResolvedLayers::from_folders(&[
        (&vanilla, &[][..]),
        (&modded, &["common/buildings"][..]),
    ])
    .unwrap();
    let catalogue = Catalogue::of_layers_for_check(layers, "english").unwrap();
    let (groups, warnings) = content_groups(&catalogue);
    let findings = catalogue.take_findings();
    assert_eq!(findings.len(), 3);
    assert!(findings.iter().any(|finding| {
        finding.kind == Kind::Refused
            && finding.subject == "building tank_works"
            && finding.count == Some(1)
            && finding.message.contains("positive rocket_production")
    }));
    assert!(findings.iter().any(|finding| {
        finding.kind == Kind::Content
            && finding.subject == "building workshop"
            && finding.message.contains("142230940")
    }));
    assert!(findings.iter().any(|finding| {
        finding.kind == Kind::Fallback && finding.subject == "factory group naval_production"
    }));
    let _ = std::fs::remove_dir_all(&vanilla);
    let _ = std::fs::remove_dir_all(&modded);
    let named = |key| -> Vec<(String, Option<String>)> {
        groups
            .iter()
            .find(|group| group.key == key)
            .unwrap()
            .buildings
            .iter()
            .map(|building| (building.token.clone(), building.name.clone()))
            .collect()
    };
    let some = |text: &str| Some(text.to_owned());
    assert_eq!(
        named(FactoryGroupKey::Military),
        [
            ("arms_factory".to_owned(), some("Arms Plant")),
            ("tank_works".to_owned(), some("Tank Works"))
        ]
    );
    assert_eq!(
        named(FactoryGroupKey::Civilian),
        [
            ("industrial_complex".to_owned(), some("Civilian Factory")),
            ("workshop".to_owned(), None)
        ]
    );
    // The removed vanilla dockyard is gone; the group is named by its flag.
    assert!(named(FactoryGroupKey::Naval).is_empty());
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("naval_production")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .all(|warning| !warning.starts_with("workshop:")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("tank_works"))
    );
    assert!(
        warnings
            .iter()
            .all(|warning| !warning.contains("launch_site"))
    );
    // The native integer scan leaves an unresolved token at its constructor's zero.
    let scripted = content_folder(
        "factories_scripted",
        &[(
            "common/buildings/00_buildings.txt",
            "buildings = { arms_factory = { military_production = @factories } }",
        )],
    );
    let layers = installed_content::ResolvedLayers::from_folders(&[(&scripted, &[][..])]).unwrap();
    let (groups, warnings, findings) = describe_groups_with_findings(&layers, |_| None);
    let _ = std::fs::remove_dir_all(&scripted);
    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group.buildings.is_empty()));
    assert!(
        warnings
            .iter()
            .all(|warning| warning.starts_with("No building"))
    );
    assert_eq!(findings.len(), 3);
    assert!(
        findings
            .iter()
            .all(|finding| finding.kind == Kind::Fallback)
    );
    // A modifier block the reader cannot settle is refused, never treated as vanilla.
    let unsupported = content_folder(
        "factories_unsupported",
        &[(
            "common/buildings/00_buildings.txt",
            "buildings = { works = { military_production = 1 state_modifiers = { custom_modifier = { value = 1 } } } }",
        )],
    );
    let layers = installed_content::ResolvedLayers::from_folders(&[(&unsupported, &[][..])]).unwrap();
    let (groups, warnings, findings) = describe_groups_with_findings(&layers, |_| None);
    let _ = std::fs::remove_dir_all(&unsupported);
    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group.buildings.is_empty()));
    assert!(
        warnings[0].starts_with("Factory types are unavailable"),
        "{warnings:?}"
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, Kind::Refused);
    assert_eq!(findings[0].subject, "factory types");
}

#[test]
fn factory_identity_segments_are_not_cut_to_top_membership() {
    use crate::products::{
        country_factories::{
            FactoriesParams, FactoryCount, FactoryGroupKey, FactoryTop, requested_series, scan,
        },
        country_groups::HourSpan,
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                factory_frame(100, true, 9001, 2_000_000),
                // Same identity leaves the top, then another identity takes the tag.
                factory_frame(101, true, 9001, -100_000),
                factory_frame(102, true, 9002, -100_000),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let series = requested_series(
        &scanned,
        &FactoriesParams {
            branch_id: "1".into(),
            tags: vec![],
            top: Some(FactoryTop {
                count: 1,
                mode: FactoryCount::Built,
                groups: vec![FactoryGroupKey::Civilian],
            }),
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let country = series.iter().find(|item| item.tag == "GER").unwrap();
    assert_eq!(country.runs.last().unwrap().last_hour, 100);
    assert_eq!(
        country.membership.as_deref().unwrap(),
        [HourSpan {
            first_hour: 100,
            last_hour: 100
        }]
    );
    assert_eq!(
        country.segments,
        [
            HourSpan {
                first_hour: 100,
                last_hour: 101
            },
            HourSpan {
                first_hour: 102,
                last_hour: 102
            }
        ]
    );
}

#[test]
fn factory_tags_are_opaque_strings_ranked_in_byte_order() {
    use crate::products::{
        Product,
        country_factories::{
            CountryFactories, FactoriesParams, FactoryCount, FactoryGroupKey, FactoryTop,
            ScannedFactories, requested_series,
        },
    };
    // Dynamic, spaced, lower-case and non-ASCII tags, all tied.
    let tags = ["D01", "MY TAG", "aaa", "AAA", "Ü1"];
    let mut scanned = ScannedFactories::default();
    for tag in tags {
        scanned.series.insert(
            tag.to_owned(),
            vec![factory_run(100, 101, factory_values(2, 7, 0))],
        );
    }
    let chosen = FactoriesParams {
        branch_id: "1".into(),
        tags: vec!["MY TAG".into(), "D01".into()],
        top: None,
    };
    assert!(CountryFactories::validate(&chosen).is_ok());
    let series = requested_series(&scanned, &chosen, &AtomicBool::new(false)).unwrap();
    let names: Vec<&str> = series.iter().map(|item| item.tag.as_str()).collect();
    assert_eq!(names, ["MY TAG", "D01"]);
    // Tied at 7, the top 3 are the first three in byte order: AAA, D01, MY TAG.
    let top = FactoriesParams {
        tags: vec![],
        top: Some(FactoryTop {
            count: 3,
            mode: FactoryCount::Built,
            groups: vec![FactoryGroupKey::Civilian],
        }),
        ..chosen
    };
    let series = requested_series(&scanned, &top, &AtomicBool::new(false)).unwrap();
    let names: Vec<&str> = series.iter().map(|item| item.tag.as_str()).collect();
    assert_eq!(names, ["AAA", "D01", "MY TAG"]);
}

#[test]
fn factory_selections_follow_the_recorded_country_count() {
    use crate::products::{
        Product,
        country_factories::{
            CountryFactories, FactoriesParams, FactoryCount, FactoryTop, ScannedFactories,
            requested_series,
        },
    };
    let scanned = ScannedFactories {
        series: (0..5_000)
            .map(|country| {
                (
                    format!("T{country}"),
                    vec![factory_run(10, 10, vec![0; 31])],
                )
            })
            .collect(),
        ..Default::default()
    };
    let mut params = FactoriesParams {
        branch_id: "1".into(),
        tags: scanned.series.keys().cloned().collect(),
        top: None,
    };
    assert!(CountryFactories::validate(&params).is_ok());
    assert_eq!(
        requested_series(&scanned, &params, &AtomicBool::new(false))
            .unwrap()
            .len(),
        scanned.series.len()
    );
    params.tags.clear();
    params.top = Some(FactoryTop {
        count: scanned.series.len() as u32,
        mode: FactoryCount::Built,
        groups: vec![],
    });
    assert!(CountryFactories::validate(&params).is_ok());
    assert_eq!(
        requested_series(&scanned, &params, &AtomicBool::new(false))
            .unwrap()
            .len(),
        scanned.series.len()
    );
}

#[test]
fn a_long_campaign_s_reply_is_bounded_by_the_countries_asked_for() {
    use crate::products::country_factories::{
        FactoriesParams, FactoryCount, FactoryGroupKey, FactoryTop, MAX_REPLY_BYTES,
        ScannedFactories, requested_bytes, requested_series, series_bytes,
    };
    // 100 countries, each with 2,500 changes (a long campaign's daily runs) of
    // five-digit values in every field.
    let mut scanned = ScannedFactories::default();
    for country in 0..100 {
        let runs = (0..2_500)
            .map(|day| {
                let first = 60_000_000 + day * 24;
                let values = (0..31)
                    .map(|field| 10_000 + (country + day + field) % 89_999)
                    .collect();
                factory_run(first, first + 23, values)
            })
            .collect();
        scanned.series.insert(format!("T{country:02}"), runs);
    }
    let every: usize = scanned.series.values().map(|runs| runs.len()).sum();
    assert_eq!(every, 250_000);
    let serialized = |series| serde_json::to_string(series).unwrap().len();
    let selected = 30;
    let chosen = FactoriesParams {
        branch_id: "1".into(),
        tags: scanned.series.keys().take(selected).cloned().collect(),
        top: None,
    };
    let series = requested_series(&scanned, &chosen, &AtomicBool::new(false)).unwrap();
    assert_eq!(series.len(), selected);
    // The size is counted from the scan, before anything is copied, as exactly what
    // is then built, and never undercounted.
    let counted = |params: &FactoriesParams| {
        requested_bytes(&scanned, params, &AtomicBool::new(false)).unwrap()
    };
    assert_eq!(counted(&chosen), series_bytes(&series));
    assert!(series_bytes(&series) >= serialized(&series));
    assert!(serialized(&series) < MAX_REPLY_BYTES);
    let top = FactoriesParams {
        top: Some(FactoryTop {
            count: 10,
            mode: FactoryCount::Free,
            groups: vec![FactoryGroupKey::Civilian, FactoryGroupKey::Military],
        }),
        tags: vec![],
        ..chosen
    };
    let series = requested_series(&scanned, &top, &AtomicBool::new(false)).unwrap();
    // At most ten countries' runs hold any hour.
    for hour in [60_000_000, 60_030_011, 60_059_999] {
        let holding = series
            .iter()
            .filter(|item| {
                item.runs
                    .iter()
                    .any(|run| run.first_hour <= hour && hour <= run.last_hour)
            })
            .count();
        assert_eq!(holding, 10, "{hour}");
    }
    assert_eq!(counted(&top), series_bytes(&series));
    assert!(series_bytes(&series) >= serialized(&series));
    assert!(serialized(&series) < MAX_REPLY_BYTES);
    // A history longer than a reply holds is refused before it is built.
    let mut huge = ScannedFactories::default();
    huge.series.insert(
        "BIG".into(),
        (0..200_000)
            .map(|hour| factory_run(hour, hour, vec![-1_000_000_000; 31]))
            .collect(),
    );
    let asked = FactoriesParams {
        branch_id: "1".into(),
        tags: vec!["BIG".into()],
        top: None,
    };
    assert!(requested_bytes(&huge, &asked, &AtomicBool::new(false)).unwrap() > MAX_REPLY_BYTES);
    assert!(requested_series(&huge, &asked, &AtomicBool::new(false)).is_err());
}

#[test]
fn factory_slot_hour_reads_are_dated_at_their_hour_without_table_timing() {
    use crate::products::country_factories::{
        FactoryField, FactoryGroupKey, FactoryPart, fields, scan,
    };
    let frames = vec![
        factory_frame(100, false, 9001, 2_000_000),
        factory_frame(101, false, 9001, 2_100_000),
        factory_frame(102, false, 9001, 2_100_000),
    ];
    assert!(
        frames
            .iter()
            .all(|frame| frame.metrics.table_ns.iter().all(|time| *time == 0))
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let civilian = fields()
        .iter()
        .position(|field| {
            *field
                == FactoryField {
                    group: FactoryGroupKey::Civilian,
                    part: FactoryPart::Total,
                }
        })
        .unwrap();
    let reads: Vec<_> = scanned.series["GER"]
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.values[civilian]))
        .collect();
    assert_eq!(reads, [(100, 100, 20), (101, 102, 21)]);
    assert_eq!(
        scanned.reads["GER"],
        [super::FactoryReadRun {
            first_hour: 100,
            last_hour: 102,
            read_hour: 100,
            hourly: true,
        }]
    );
}

#[test]
fn factory_read_confirmations_do_not_use_value_compression_as_provenance() {
    use super::{FactoryReadRun, append_read};
    let mut reads = Vec::new();
    append_read(&mut reads, 100, 100, true, false);
    append_read(&mut reads, 101, 100, false, true);
    // A fresh confirmation of the same value must not retain the carried read.
    append_read(&mut reads, 102, 102, true, true);
    append_read(&mut reads, 103, 103, true, true);
    // A different holder and a missing read each start a new provenance span.
    append_read(&mut reads, 104, 104, true, false);
    append_read(&mut reads, 106, 106, true, false);
    assert_eq!(
        reads,
        [
            FactoryReadRun {
                first_hour: 100,
                last_hour: 100,
                read_hour: 100,
                hourly: true
            },
            FactoryReadRun {
                first_hour: 101,
                last_hour: 101,
                read_hour: 100,
                hourly: false
            },
            FactoryReadRun {
                first_hour: 102,
                last_hour: 103,
                read_hour: 102,
                hourly: true
            },
            FactoryReadRun {
                first_hour: 104,
                last_hour: 104,
                read_hour: 104,
                hourly: true
            },
            FactoryReadRun {
                first_hour: 106,
                last_hour: 106,
                read_hour: 106,
                hourly: true
            },
        ]
    );
}

#[test]
#[ignore = "large synthetic recording"]
fn factories_scale_1500_and_5000_countries_preserves_every_country() {
    use crate::products::country_factories::scan;
    use crate::test_fixtures::scale_frame;
    use replay::rows::CountryFactoriesRow;
    const HOURS: i32 = 24;
    for (count, changing) in [1_500, 5_000]
        .into_iter()
        .flat_map(|count| [(count, false), (count, true)])
    {
        let frames = (100..100 + HOURS)
            .map(|hour| {
                let mut frame = scale_frame(1, hour, count);
                *frame.tables[TableId::CountryFactories.index()] =
                    TableView::Complete(TableRows::CountryFactories(
                        (0..count)
                            .map(|index| CountryFactoriesRow {
                                country: 100_000 + index,
                                civilian_total_raw: (i64::from(index)
                                    + if changing && index == 0 {
                                        i64::from(hour - 100)
                                    } else {
                                        0
                                    })
                                    * 100_000,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(scanned.series.len(), count as usize);
        assert!(scanned.series.iter().all(|(tag, runs)| {
            if changing && tag == &String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap() {
                runs.len() == HOURS as usize && runs.last().unwrap().last_hour == 100 + HOURS - 1
            } else {
                runs.len() == 1 && runs[0].last_hour == 100 + HOURS - 1
            }
        }));
    }
}

#[test]
fn faction_factory_adapter_clips_recorded_runs_around_proven_zero_spans() {
    use super::{FactoryCount, FactoryGroupKey, FactoryRun, faction_measure_runs};
    use crate::products::country_groups::HourSpan;
    let recorded = [FactoryRun {
        first_hour: 0,
        last_hour: 10,
        starts_segment: true,
        values: vec![7; super::fields().len()],
    }];
    let absent = [
        HourSpan {
            first_hour: 3,
            last_hour: 5,
        },
        HourSpan {
            first_hour: 9,
            last_hour: 11,
        },
    ];
    let runs = faction_measure_runs(
        &recorded,
        &absent,
        FactoryCount::Built,
        &[FactoryGroupKey::Military],
        &mut 0,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.value))
            .collect::<Vec<_>>(),
        [
            (0, 2, Some(7)),
            (3, 5, Some(0)),
            (6, 8, Some(7)),
            (9, 11, Some(0))
        ]
    );
    assert!(
        runs.windows(2)
            .all(|pair| pair[0].last_hour < pair[1].first_hour)
    );
    assert!(
        runs[2].starts_segment,
        "absence breaks the factory country's identity"
    );
}
