use super::*;
use crate::{
    SourceKey,
    products::find,
    test_fixtures::{CAMPAIGN, records, source},
};
use replay::{
    frame::TableView,
    rows::{
        CountryLinkRow, CountryRow, DivisionManpowerContributor, DivisionRow, DivisionStatusRow,
        Recorded, TableRows,
    },
};
use std::{hint::black_box, path::PathBuf, time::Instant};

fn defines() -> Defines {
    // Synthetic content, deliberately different from shipped thresholds.
    Defines {
        grace: 10,
        low: 60_000,
        very_low_icon: 15_000,
        low_icon: 40_000,
    }
}
fn supply(received: i64, remaining_grace: i64) -> Supply {
    Supply {
        received,
        maximum_grace: 1_000_000,
        remaining_grace,
        naval_transfer: false,
    }
}
fn land(division: u64) -> DivisionLandInputsRow {
    DivisionLandInputsRow {
        division,
        ..Default::default()
    }
}
fn status(division: u64, received: i64, remaining_grace: i64) -> DivisionStatusRow {
    DivisionStatusRow {
        division,
        supply_ratio: received,
        supply_grace_maximum: Recorded::Value(1_000_000),
        supply_grace_remaining: Recorded::Value(remaining_grace),
        ..Default::default()
    }
}
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, None);
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: 1,
            roster_owner: 11,
            ..Default::default()
        },
        DivisionRow {
            id: 2,
            roster_owner: 11,
            ..Default::default()
        },
        DivisionRow {
            id: 3,
            roster_owner: 11,
            ..Default::default()
        },
        DivisionRow {
            id: 4,
            roster_owner: 11,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(vec![
            land(1),
            land(2),
            land(3),
            land(4),
        ]));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![
            status(1, 0, 1_000_000),
            status(2, 14_999, 0),
            status(3, 15_000, 0),
            status(4, 40_000, 0),
        ]));
    frame
}
fn tags() -> Vec<String> {
    vec!["GER".into()]
}
fn counts(run: &SupplyHealthRun) -> [u32; 3] {
    [run.no_supply, run.strained, run.in_supply]
}

#[test]
fn manual_supply_selection_keeps_both_dated_tags_of_the_same_native_country() {
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root =
        Scratch(std::env::temp_dir().join(format!("supply_aliases_{}", uuid::Uuid::new_v4())));
    let folder = root.0.join("common/defines");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("synthetic.lua"), "NDefines={NMilitary={SUPPLY_GRACE=10,LOW_SUPPLY=.6},NGraphics={SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD=.15,SUPPLY_UNIT_COUNTER_LOW_THRESHOLD=.4}}").unwrap();
    std::fs::write(root.0.join("hoi4.exe"), b"test").unwrap();
    std::fs::create_dir_all(root.0.join("user")).unwrap();
    std::fs::write(
        root.0.join("user/dlc_load.json"),
        r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    )
    .unwrap();
    std::fs::write(
        root.0.join("user/settings.txt"),
        "language=l_english force_pow2_textures=no",
    )
    .unwrap();
    let (options, identity) = crate::test_fixtures::content_identity(&root.0);
    let first = frame(100);
    let mut renamed = first.clone();
    renamed.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"native rename".to_vec();
    }
    let mut reused = renamed.clone();
    reused.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"next rename".to_vec();
        rows.push(CountryRow {
            id: 10001,
            canonical_slot: 8,
            tag: b"native rename".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *reused.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 10001,
            canonical_slot: 8,
        });
    }
    let mut wire = records(1, vec![first, renamed, reused]);
    if let replay::spool::Record::SessionStart(session) = &mut wire[0] {
        session.identity = identity;
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[wire]).unwrap();
    let src = Source::from_replay(
        SourceKey {
            source_id: "renamed supply".into(),
            revision: "1".into(),
        },
        PathBuf::from("synthetic.battleplan"),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    for tag in ["GER", "native rename"] {
        let params = SupplyHealthParams {
            branch_id: "1".into(),
            tags: vec![tag.into()],
        };
        let got = SupplyHealth::load(
            &src,
            &params,
            &LoadContext {
                content: Some(options.clone()),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let old = got
            .series
            .iter()
            .find(|series| series.tag == "GER")
            .unwrap();
        let new = got
            .series
            .iter()
            .find(|series| series.tag == "native rename")
            .unwrap();
        assert_eq!((old.runs[0].first_hour, old.runs[0].last_hour), (100, 100));
        assert_eq!((new.runs[0].first_hour, new.runs[0].last_hour), (101, 101));
        assert_eq!(counts(&old.runs[0]), counts(&new.runs[0]));
        let last = if tag == "GER" { 101 } else { 102 };
        assert_eq!(new.runs.last().unwrap().last_hour, last);
        assert_eq!(
            new.identities.as_ref().unwrap().last().unwrap().last_hour,
            last
        );
    }
}

#[test]
fn supply_health_native_icon_gate_frames_and_fixed_arithmetic() {
    let d = defines();
    for (received, grace, expected) in [
        (0, 1_000_000, 0),
        (0, 0, 3),
        (14_999, 0, 3),
        (15_000, 0, 2),
        (39_999, 0, 2),
        (40_000, 0, 1),
        (59_999, 0, 1),
        (60_000, 0, 0),
        (-1, -1, 3),
    ] {
        assert_eq!(d.frame(&supply(received, grace)), expected);
    }
    let mut row = supply(0, 0);
    row.naval_transfer = true;
    assert_eq!(d.frame(&row), 0);
    row.naval_transfer = false;
    row.maximum_grace = 0;
    row.remaining_grace = 1_000_000;
    assert_eq!(d.effective(&row), 0);
    row.received = 100_000;
    assert_eq!(d.frame(&row), 1);
    row.received = 0;
    row.maximum_grace = 1;
    for sentinel in [i64::MIN, i64::MIN + 1, i64::MAX] {
        row.remaining_grace = sentinel;
        assert_eq!(d.effective(&row), 100_000);
    }
    row.remaining_grace = i64::MAX / 100_000 + 1;
    assert_eq!(d.effective(&row), 0); // native multiplication wraps before division
    let mut odd = d;
    odd.grace = 0;
    assert_eq!(odd.effective(&supply(0, 0)), 100_000);
    odd = d;
    odd.very_low_icon = 110_000;
    assert_eq!(odd.frame(&supply(100_000, 0)), 3); // native compares substituted one too
}

#[test]
fn supply_health_reads_mod_defines_and_refuses_missing_forms_by_name() {
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(
        std::env::temp_dir().join(format!("supply_health_defines_{}", uuid::Uuid::new_v4())),
    );
    let root = &scratch.0;
    let base = root.join("base");
    let addon = root.join("mod");
    for layer in [&base, &addon] {
        std::fs::create_dir_all(layer.join("common/defines")).unwrap();
    }
    std::fs::write(base.join("common/defines/00_base.lua"),
        "NDefines={NMilitary={SUPPLY_GRACE=10,LOW_SUPPLY=.6},NGraphics={SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD=.15,SUPPLY_UNIT_COUNTER_LOW_THRESHOLD=.4}}").unwrap();
    std::fs::write(addon.join("common/defines/01_mod.lua"),
        "local x=2; NDefines.NMilitary.SUPPLY_GRACE=x*7; NDefines.NMilitary.LOW_SUPPLY=.73; NDefines.NGraphics.SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD=.123456789; NDefines.NGraphics.SUPPLY_UNIT_COUNTER_LOW_THRESHOLD=.52").unwrap();
    let layers = installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
    let got = Defines::read(&installed_content::defines::read(&layers).unwrap()).unwrap();
    assert_eq!(
        got,
        Defines {
            grace: 14,
            low: 73_000,
            very_low_icon: 12_346,
            low_icon: 52_000
        }
    );
    assert_eq!(got.frame(&supply(50_000, 0)), 2);
    std::fs::write(
        addon.join("common/defines/01_mod.lua"),
        "NDefines.NGraphics.SUPPLY_UNIT_COUNTER_LOW_THRESHOLD={unsupported=1}",
    )
    .unwrap();
    let layers = installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
    assert!(
        Defines::read(&installed_content::defines::read(&layers).unwrap())
            .unwrap_err()
            .to_string()
            .contains("NGraphics.SUPPLY_UNIT_COUNTER_LOW_THRESHOLD")
    );
}

#[test]
fn supply_health_counts_owner_not_host_and_missing_rows_break_only_the_owner() {
    let mut f = frame(100);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"opaque host".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 9002,
            canonical_slot: 8,
        });
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *f.tables[TableId::Divisions.index()]
    {
        rows[0].roster_owner = 9002;
        rows[0].expeditionary_owner = 11;
        rows[1].expeditionary_owner = u32::MAX; // native signed negative: fall back to roster
    }
    let mut scan = Scan::new(&["GER".into(), "opaque host".into()]);
    scan.observe(&f, defines()).unwrap();
    assert_eq!(counts(&scan.series[0].runs[0]), [1, 2, 1]);
    assert_eq!(counts(&scan.series[1].runs[0]), [0; 3]);
    f.hour += 1;
    *f.tables[TableId::DivisionLandInputs.index()] = TableView::Truncated {
        rows: TableRows::DivisionLandInputs(vec![land(2), land(3), land(4)]),
        missing: 1,
    };
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs[0].last_hour, 100);
    assert_eq!(scan.series[1].runs[0].last_hour, 101);
    f.hour += 1;
    f.tables[TableId::DivisionLandInputs.index()] =
        frame(102).tables[TableId::DivisionLandInputs.index()].clone();
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs.len(), 2);
    assert!(scan.series[0].runs[1].starts_segment);
    // Equal fresh reads retain their actual last read hour.
    f.hour += 1;
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs[1].last_hour, 103);
}

#[test]
fn supply_health_classifies_from_division_status_and_needs_its_grace() {
    // DivisionStatus is the record of the received fraction and grace; the
    // land inputs supply only naval transfer.
    let mut f = frame(100);
    if let TableView::Complete(TableRows::DivisionLandInputs(rows)) =
        &mut *f.tables[TableId::DivisionLandInputs.index()]
    {
        rows[1].naval_transfer = true;
    }
    let mut scan = Scan::new(&tags());
    scan.observe(&f, defines()).unwrap();
    assert_eq!(counts(&scan.series[0].runs[0]), [0, 2, 2]);
    // An unavailable status table withholds the hour.
    f.hour += 1;
    let complete = f.tables[TableId::DivisionStatus.index()].clone();
    *f.tables[TableId::DivisionStatus.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs.len(), 1);
    assert_eq!(scan.series[0].runs[0].last_hour, 100);
    // Unrecorded grace cannot classify its division, so the owner breaks.
    f.hour += 1;
    f.tables[TableId::DivisionStatus.index()] = complete.clone();
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *f.tables[TableId::DivisionStatus.index()]
    {
        rows[0].supply_grace_remaining = Recorded::NotRecorded;
    }
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs.len(), 1);
    // A changed status row alone recounts the hour.
    f.hour += 1;
    f.tables[TableId::DivisionStatus.index()] = complete;
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *f.tables[TableId::DivisionStatus.index()]
    {
        rows[3].supply_ratio = 100_000;
    }
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs.len(), 2);
    assert!(scan.series[0].runs[1].starts_segment);
    assert_eq!(counts(&scan.series[0].runs[1]), [0, 1, 3]);
}

#[test]
fn supply_health_replay_scan_preserves_gaps_failures_identity_and_zero_denominators() {
    let a = frame(100);
    let mut failed = a.clone();
    failed.hour = 102;
    *failed.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    let b = frame(103);
    let mut changed = frame(104);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 11;
    }
    let mut empty = changed.clone();
    empty.hour = 105;
    *empty.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(Vec::new()));
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![a, failed, b, changed, empty])],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &tags(), defines(), &AtomicBool::new(false)).unwrap();
    let runs = &scanned.series[0].runs;
    assert_eq!(
        runs.iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103), (104, 104), (105, 105)]
    );
    assert!(runs[..3].iter().all(|r| r.starts_segment));
    assert!(!runs[3].starts_segment);
    assert_eq!(counts(&runs[3]), [0; 3]);
    let mut absent = frame(106);
    *absent.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    let mut scan = Scan::new(&tags());
    scan.observe(&absent, defines()).unwrap();
    scan.observe(&frame(107), defines()).unwrap();
    assert_eq!(scan.series[0].runs[0].first_hour, 107);
}

#[test]
fn supply_health_cached_joins_follow_owner_and_link_changes_not_other_columns() {
    let mut f = frame(100);
    let mut scan = Scan::new(&tags());
    scan.observe(&f, defines()).unwrap();
    f.hour += 1;
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *f.tables[TableId::Divisions.index()]
    {
        rows[0].province = 50;
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(42);
    }
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs.len(), 1);
    assert_eq!(scan.series[0].runs[0].last_hour, 101);
    f.hour += 1;
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *f.tables[TableId::Divisions.index()]
    {
        rows[0].roster_owner = 88;
    }
    scan.observe(&f, defines()).unwrap(); // unresolved owner might be this country
    assert_eq!(scan.series[0].runs[0].last_hour, 101);
    f.hour += 1;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 88,
            canonical_slot: 7,
        });
        rows.sort_by_key(|row| row.id);
    }
    scan.observe(&f, defines()).unwrap();
    assert_eq!(counts(&scan.series[0].runs[1]), [1, 2, 1]);
    assert_eq!(scan.series[0].runs[1].first_hour, 103);
    assert!(scan.series[0].runs[1].starts_segment);
    f.hour += 1;
    *f.tables[TableId::DivisionLandInputs.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    scan.observe(&f, defines()).unwrap();
    assert_eq!(scan.series[0].runs[1].last_hour, 103);
}

#[test]
fn supply_health_registry_no_country_cap_and_old_recording_unavailable_without_content() {
    assert!(find("supply-health").unwrap().content_dependent);
    let params = SupplyHealthParams {
        branch_id: "1".into(),
        tags: (0..5_000).map(|i| format!("opaque {i}")).collect(),
    };
    assert!(SupplyHealth::validate(&params).is_ok());
    for name in [
        "SupplyHealthParams",
        "SupplyHealthRun",
        "SupplyHealthSeries",
        "SupplyHealthReply",
    ] {
        assert!(crate::typescript_contract().contains(&format!("export type {name} =")));
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![source(1, 100, None)])]).unwrap();
    let src = Source::from_replay(
        SourceKey {
            source_id: "fixture".into(),
            revision: "1".into(),
        },
        PathBuf::from("synthetic.battleplan"),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let got = SupplyHealth::load(
        &src,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!got.available);
    assert_eq!(got.missing_inputs, ["DivisionLandInputs"]);
    assert!(got.series.iter().all(|s| s.runs.is_empty()));
    assert_eq!(
        SupplyHealth::load(
            &src,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn supply_health_reply_counts_bytes_and_refuses_instead_of_truncating() {
    let mut scan = Scan::new(&tags());
    scan.observe(&frame(100), defines()).unwrap();
    let got = reply("1".into(), scan, Vec::new()).unwrap();
    let bytes = json_len(&got, MAX_PRODUCT_REPLY_BYTES).unwrap();
    assert_eq!(json_len(&got, bytes), Some(bytes));
    assert_eq!(json_len(&got, bytes - 1), None);
    let mut oversized = Scan::new(&["x".repeat(MAX_PRODUCT_REPLY_BYTES)]);
    oversized.available = true;
    assert!(
        reply("1".into(), oversized, Vec::new())
            .unwrap_err()
            .to_string()
            .contains("nothing was omitted")
    );
}

#[test]
fn supply_health_bad_country_identity_gaps_only_its_own_series() {
    let mut f = frame(100);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"unresolved".to_vec(),
            ..Default::default()
        });
    }
    let mut scan = Scan::new(&["GER".into(), "unresolved".into()]);
    scan.observe(&f, defines()).unwrap();
    assert_eq!(counts(&scan.series[0].runs[0]), [1, 2, 1]);
    assert!(scan.series[1].runs.is_empty());
}

#[test]
fn supply_health_cache_reuses_countries_and_retains_only_current_branch() {
    let state = Mutex::new(Cache::default());
    let cancelled = AtomicBool::new(false);
    let mut scanned = Vec::new();
    for (branch, chosen) in [
        (1, vec!["GER"]),
        (1, vec!["GER", "new"]),
        (1, vec!["new"]),
        (2, vec!["GER"]),
    ] {
        let chosen: Vec<String> = chosen.into_iter().map(str::to_owned).collect();
        let built = cached_scan(&state, branch, &chosen, defines(), &cancelled, |missing| {
            assert!(state.try_lock().is_ok());
            scanned.push((branch, missing.to_vec()));
            let mut scan = Scan::new(missing);
            scan.observe(&frame(100), defines())?;
            Ok(scan)
        })
        .unwrap();
        assert_eq!(built.series.len(), chosen.len());
        assert!(
            state
                .lock()
                .unwrap()
                .entries
                .keys()
                .all(|(from, _)| *from == branch)
        );
    }
    assert_eq!(
        scanned,
        [
            (1, vec!["GER".to_owned()]),
            (1, vec!["new".to_owned()]),
            (2, vec!["GER".to_owned()])
        ]
    );
    let mut changed = defines();
    changed.low = 100_001;
    cached_scan(&state, 2, &tags(), changed, &cancelled, |missing| {
        assert_eq!(missing, tags());
        let mut scan = Scan::new(missing);
        scan.observe(&frame(100), changed)?;
        Ok(scan)
    })
    .unwrap();
}

#[test]
fn supply_health_cached_hit_can_finish_during_another_country_scan() {
    let state = Arc::new(Mutex::new(Cache::default()));
    let cancelled = AtomicBool::new(false);
    cached_scan(&state, 1, &tags(), defines(), &cancelled, |missing| {
        let mut scan = Scan::new(missing);
        scan.observe(&frame(100), defines())?;
        Ok(scan)
    })
    .unwrap();
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, resumed) = std::sync::mpsc::channel();
    let worker_state = Arc::clone(&state);
    let worker = std::thread::spawn(move || {
        cached_scan(
            &worker_state,
            1,
            &["new".into()],
            defines(),
            &AtomicBool::new(false),
            |missing| {
                entered.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(5)).unwrap();
                let mut scan = Scan::new(missing);
                scan.observe(&frame(100), defines())?;
                Ok(scan)
            },
        )
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let hit = cached_scan(&state, 1, &tags(), defines(), &cancelled, |_| {
        panic!("cached country rescanned")
    })
    .unwrap();
    assert_eq!(counts(&hit.series[0].runs[0]), [1, 2, 1]);
    release.send(()).unwrap();
    worker.join().unwrap().unwrap();
}

fn scale_frame(divisions: u64) -> DecodedFrame {
    let mut f = source(1, 1_000_000, None);
    *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5_000)
            .map(|i| CountryRow {
                id: i,
                canonical_slot: i,
                tag: format!("opaque {i}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=5_000)
            .map(|i| CountryLinkRow {
                id: i,
                canonical_slot: i,
            })
            .collect(),
    ));
    *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        (1..=divisions)
            .map(|i| DivisionRow {
                id: i,
                roster_owner: ((i - 1) % 5_000 + 1) as u32,
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(
            (1..=divisions)
                .map(|i| {
                    let mut row = land(i);
                    row.manpower = vec![DivisionManpowerContributor {
                        country: ((i - 1) % 5_000 + 1) as u32,
                        amount: 10_000,
                    }];
                    row
                })
                .collect(),
        ));
    *f.tables[TableId::DivisionStatus.index()] = TableView::Complete(TableRows::DivisionStatus(
        (1..=divisions)
            .map(|i| status(i, 100_000, 1_000_000))
            .collect(),
    ));
    f
}

#[test]
fn supply_health_large_selection_refuses_only_the_measured_reply_budget() {
    let selected: Vec<String> = (1..=5_000).map(|i| format!("opaque {i}")).collect();
    let mut scan = Scan::new(&selected);
    let mut f = scale_frame(5_000);
    // Enough synthetic changes to exceed the transport reply; no production
    // run-count limit. This loop's extent is only the test fixture size.
    for hour in 0..100 {
        f.hour += 1;
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *f.tables[TableId::DivisionStatus.index()]
        {
            for row in rows {
                row.supply_ratio = if hour % 2 == 0 { 0 } else { 100_000 };
                row.supply_grace_remaining = Recorded::Value(0);
            }
        }
        scan.observe(&f, defines()).unwrap();
        assert!(
            scan.series
                .iter()
                .all(|s| s.runs.last().unwrap().last_hour == f.hour)
        );
    }
    assert!(
        reply("1".into(), scan, Vec::new())
            .unwrap_err()
            .to_string()
            .contains("MAX_PRODUCT_REPLY_BYTES")
    );
}

#[test]
#[ignore = "nine-year hourly synthetic scan measurement"]
fn supply_health_scale_nine_years() {
    let selected: Vec<String> = (1..=10).map(|i| format!("opaque {i}")).collect();
    let mut scan = Scan::new(&selected);
    let mut single = Scan::new(&selected[..1]);
    let mut f = scale_frame(50_000);
    let all: Vec<String> = (1..=5_000).map(|i| format!("opaque {i}")).collect();
    let mut all_scan = Scan::new(&all);
    all_scan.observe(&f, defines()).unwrap();
    assert_eq!(all_scan.series.len(), 5_000);
    assert!(
        all_scan
            .series
            .iter()
            .all(|s| counts(&s.runs[0]) == [0, 0, 10])
    );
    let mut times = Vec::new();
    let mut single_times = Vec::new();
    let mut grace_changes = 0usize;
    let mut received_changes = 0usize;
    // Phase buckets update exactly the changing capture rows; fixture sizes
    // are not production limits. One 48-hour disruption per 30 days, staggered.
    let mut phases: Vec<Vec<usize>> = vec![Vec::new(); 720];
    for index in 0..50_000 {
        phases[index * 37 % 720].push(index);
    }
    for hour in 0..9 * 365 * 24 {
        f.hour = 1_000_000 + hour;
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *f.tables[TableId::DivisionStatus.index()]
        {
            let changing: Vec<usize> = if hour == 0 {
                (0..720).collect()
            } else {
                (0..=10).chain(48..=77).collect()
            };
            for phase in changing {
                let bucket = (phase + 720 - hour as usize % 720) % 720;
                let grace = if phase < 10 {
                    (10 - phase) as i64 * 100_000
                } else if phase < 48 {
                    0
                } else if phase < 77 {
                    ((phase - 48) as i64 * 35_000).min(1_000_000)
                } else {
                    1_000_000
                };
                for &index in &phases[bucket] {
                    let row = &mut rows[index];
                    let received = if phase < 48 {
                        [10_000, 25_000, 50_000][index % 3]
                    } else {
                        100_000
                    };
                    grace_changes +=
                        usize::from(row.supply_grace_remaining != Recorded::Value(grace));
                    received_changes += usize::from(row.supply_ratio != received);
                    row.supply_ratio = received;
                    row.supply_grace_remaining = Recorded::Value(grace);
                }
            }
        }
        // Moving divisions and changing country balances must not turn the
        // ownership cache into an unrealistic unchanged-table shortcut.
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *f.tables[TableId::Divisions.index()]
        {
            for row in rows.iter_mut().skip(hour as usize % 20).step_by(20) {
                row.province = hour as u32 + 1;
            }
        }
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *f.tables[TableId::Countries.index()]
        {
            for row in rows.iter_mut().skip(hour as usize % 100).step_by(100) {
                row.political_power_raw = Some(i64::from(hour));
            }
        }
        let started = Instant::now();
        scan.observe(black_box(&f), defines()).unwrap();
        times.push(started.elapsed().as_nanos());
        let started = Instant::now();
        single.observe(&f, defines()).unwrap();
        single_times.push(started.elapsed().as_nanos());
    }
    let hours = times.len();
    times.sort_unstable();
    single_times.sort_unstable();
    let got = reply("1".into(), scan, Vec::new()).unwrap();
    let ten = json_len(&got, MAX_PRODUCT_REPLY_BYTES).unwrap();
    let runs = got.series.iter().map(|s| s.runs.len()).sum::<usize>();
    let one_reply = reply("1".into(), single, Vec::new()).unwrap();
    assert_eq!(one_reply.series[0].runs, got.series[0].runs);
    let one = json_len(&one_reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    println!(
        "supply_health hourly builder: countries=5000 divisions=50000 hours={hours} selected=10; p50={:.3}ms p95={:.3}ms; grace_changes/hour={:.3}% received_changes/hour={:.3}%; one={one}B ten={ten}B runs_ten={runs}; 5% divisions move/hour, 1% country balances change/hour; decoded input scan, fixture mutation/encoding excluded",
        times[hours / 2] as f64 / 1e6,
        times[hours * 95 / 100] as f64 / 1e6,
        grace_changes as f64 / (hours * 50_000) as f64 * 100.0,
        received_changes as f64 / (hours * 50_000) as f64 * 100.0
    );
    println!(
        "supply_health one-country hourly builder: p50={:.3}ms p95={:.3}ms",
        single_times[hours / 2] as f64 / 1e6,
        single_times[hours * 95 / 100] as f64 / 1e6
    );
    let history = Mutex::new(Cache {
        branch: Some(1),
        defines: Some(defines()),
        entries: got
            .series
            .iter()
            .map(|series| ((1, series.tag.clone()), (true, Arc::new(series.clone()))))
            .collect(),
        ..Cache::default()
    });
    for chosen in [&selected[..1], &selected[1..], &selected[..]] {
        let mut elapsed = Vec::new();
        for _ in 0..21 {
            let started = Instant::now();
            let built = cached_scan(
                &history,
                1,
                chosen,
                defines(),
                &AtomicBool::new(false),
                |_| panic!("cached nine-year country rescanned"),
            )
            .unwrap();
            black_box(reply("1".into(), built, Vec::new()).unwrap());
            elapsed.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        elapsed.sort_by(f64::total_cmp);
        println!(
            "supply_health cached nine-year selection: selected={} missing_tags=0 real_wire_hours=0 request p50={:.3}ms p95={:.3}ms; includes complete reply byte counting",
            chosen.len(),
            elapsed[10],
            elapsed[19]
        );
    }
    // Also time the real selected wire decode + build on a 24-hour block.
    let frames: Vec<_> = (0..24)
        .map(|i| {
            let mut f = scale_frame(50_000);
            f.hour += i;
            if let TableView::Complete(TableRows::DivisionStatus(rows)) =
                &mut *f.tables[TableId::DivisionStatus.index()]
            {
                for row in rows.iter_mut().step_by(20) {
                    row.supply_grace_remaining = Recorded::Value(1_000_000 - i64::from(i) * 10_000);
                }
            }
            if let TableView::Complete(TableRows::Divisions(rows)) =
                &mut *f.tables[TableId::Divisions.index()]
            {
                for row in rows.iter_mut().step_by(20) {
                    row.province = i as u32 + 1;
                }
            }
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *f.tables[TableId::Countries.index()]
            {
                for row in rows.iter_mut().step_by(100) {
                    row.political_power_raw = Some(i64::from(i));
                }
            }
            f
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut decode = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        black_box(super::scan(&replay, 1, &selected, defines(), &AtomicBool::new(false)).unwrap());
        decode.push(start.elapsed().as_nanos() as f64 / 24.0 / 1e6);
    }
    decode.sort_by(f64::total_cmp);
    println!(
        "supply_health representative wire block decode + build/hour: p50={:.3}ms p95={:.3}ms; 5% of grace rows change hourly",
        decode[10], decode[19]
    );
    let mut selections: Vec<Vec<f64>> = vec![Vec::new(); 5];
    let choices = [
        selected[..1].to_vec(),
        selected.clone(),
        selected[1..].to_vec(),
        selected
            .iter()
            .cloned()
            .chain(["opaque 11".into()])
            .collect(),
        selected.clone(),
    ];
    let expected_missing = [1, 9, 0, 1, 0];
    for _ in 0..21 {
        let state = Mutex::new(Cache::default());
        for (index, chosen) in choices.iter().enumerate() {
            let mut missing_count = 0;
            let started = Instant::now();
            let built = cached_scan(
                &state,
                1,
                chosen,
                defines(),
                &AtomicBool::new(false),
                |missing| {
                    missing_count = missing.len();
                    super::scan(&replay, 1, missing, defines(), &AtomicBool::new(false))
                },
            )
            .unwrap();
            black_box(reply("1".into(), built, Vec::new()).unwrap());
            selections[index].push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(missing_count, expected_missing[index]);
        }
    }
    for (index, times) in selections.iter_mut().enumerate() {
        times.sort_by(f64::total_cmp);
        println!(
            "supply_health selection={index} selected={} missing_tags={} real_wire_hours={} request p50={:.3}ms p95={:.3}ms (24-hour recording)",
            choices[index].len(),
            expected_missing[index],
            if expected_missing[index] == 0 { 0 } else { 24 },
            times[10],
            times[19]
        );
    }
}
