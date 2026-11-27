use replay_columnar::manifest::Status;
use super::*;
use dashboard_query::timeline::{Join, Stretch, Timeline};

#[test]
fn each_day_is_read_at_its_last_held_hour_and_a_day_split_by_runs_takes_the_later() {
    let timeline = Timeline {
        id: 1,
        first: 30,
        last: 100,
        stretches: vec![Stretch { run: 0, first: 30, last: 50, join: Join::Start }, Stretch { run: 1, first: 60, last: 100, join: Join::Break }],
        unheld: Vec::new(),
    };
    let hours = day_hours(&timeline, 30, 100);
    assert_eq!(hours.get(&0), Some(&(0, 47)));
    assert_eq!(hours.get(&1), Some(&(1, 71)), "hours 48-50 and 60-71: the later run's last hour");
    assert_eq!(hours.get(&2), Some(&(1, 95)));
    assert_eq!(hours.get(&3), Some(&(1, 100)), "the window's end cuts the last day");
}

#[test]
fn equal_days_make_one_run_and_an_unknown_day_ends_it() {
    let contested = Values { control: Some(0), friendly: Some(5), enemy: Some(5), share: Some(50_000), own: Some(5), missing: None };
    let friendly = Values { control: Some(1), ..contested.clone() };
    let refused = Values { control: None, ..contested.clone() };
    let piece = |day: u32, values: &Values| (day, day, values.clone());
    let runs = runs(BTreeMap::from([(9, vec![piece(3, &contested), piece(4, &friendly), piece(5, &refused), (0, 1, contested.clone())])]));
    let summary: Vec<(u32, u32, Option<i8>)> = runs[&9].iter().map(|run| (run.first_day, run.last_day, run.control)).collect();
    assert_eq!(summary, [(0, 1, Some(0)), (3, 3, Some(0)), (4, 4, Some(1)), (5, 5, None)], "day 2 is not known: no run spans it");
    assert_eq!(runs[&9][0].share.as_deref(), Some("50000"));
}

#[test]
fn a_viewers_row_comes_first_then_the_every_viewer_row_then_zeros_and_a_refusal_is_no_value() {
    let viewer = 7_u32;
    let key = |holder: u32, region: u32| u128::from(holder) << 32 | u128::from(region);
    let n = Cell::Number;
    let row = |friendly, enemy, own, refusal: u32| Some(Box::from([n(friendly), n(enemy), n(friendly), n(enemy), n(own), Cell::Text(refusal), Cell::Text(0)]));
    let step = |hour, cells| lanes::Step { hour, cells };
    let history = lanes::History {
        columns: COLUMNS.to_vec(),
        to: 200,
        texts: vec![Box::from(&b""[..]), Box::from(&b"Naval current owner identity is unresolved"[..])],
        keys: vec![
            (key(viewer, 1), vec![step(100, row(300, 100, 50, 0))]),
            (key(viewer, 4), vec![step(100, row(1, 1, 1, 1))]),
            (key(NavalRegionControlRow::EVERY_VIEWER, 1), vec![step(100, row(9, 9, 9, 0))]),
            (key(NavalRegionControlRow::EVERY_VIEWER, 2), vec![step(100, row(0, 200, 0, 0)), step(150, None)]),
        ],
        statuses: vec![lanes::StatusRun { first: 0, last: 200, status: Status::Complete }],
        ..Default::default()
    };
    let sea = SeaContent {
        sea: Ok(BTreeSet::from([1, 2, 3, 4])),
        land: Ok(BTreeSet::new()),
        all: BTreeSet::from([1, 2, 3, 4]),
        minima: (1..=4).map(|region| (region, Ok(1_000))).collect(),
        ratio: Ok(200_000),
        notes: Vec::new(),
    };
    let mut notes = Refusals::default();
    let mut pieces = BTreeMap::new();
    region_runs(&history, viewer, &[(0, 120), (1, 160), (2, 170)], &BTreeSet::from([3]), &sea, &mut notes, &mut HashMap::new(), &mut pieces);
    let at = |region: u32, day: u32| pieces[&region].iter().find(|run: &&(u32, u32, Values)| run.0 <= day && day <= run.1).unwrap().2.clone();
    let first = BTreeMap::from([1, 2, 3, 4].map(|region| (region, at(region, 0))));
    assert_eq!((first[&1].friendly, first[&1].enemy, first[&1].own), (Some(300), Some(100), Some(50)), "the viewer's own row, not the every-viewer row");
    assert_eq!((first[&2].enemy, first[&2].own), (Some(200), Some(0)), "the every-viewer row");
    assert_eq!((first[&3].friendly, first[&3].own, first[&3].share), (Some(0), Some(0), Some(0)), "no row: zeros");
    assert_eq!(first[&4].control, None, "a recorded refusal is no control");
    assert_eq!(at(2, 1).enemy, Some(0), "the every-viewer row gone: zeros");
    assert_eq!(pieces[&1], vec![(0, 2, at(1, 0))], "a region whose rows do not change is one run");
    assert_eq!(pieces[&2].len(), 2, "the every-viewer row's end starts a run");
}

#[test]
fn mission_registered_ships_match_panel_count_and_refuse_missing_tables() {
    use replay_data::rows::{CountryLinkRow, CountryRow, NavalMissionLedgerRow, ShipRow, TableRows};
    use replay_frame::frame::{Metrics, TableView, not_selected};

    let mut frame = DecodedFrame { branch: 1, hour: 500, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![
        ShipRow { id: 1, task_force: 100, ..Default::default() },
        ShipRow { id: 2, task_force: 100, ..Default::default() },
        ShipRow { id: 3, task_force: 200, ..Default::default() },
        ShipRow { id: 4, task_force: 300, ..Default::default() },
    ]));
    *frame.tables[TableId::NavalMissionLedgers.index()] = TableView::Complete(TableRows::NavalMissionLedgers(vec![
        NavalMissionLedgerRow { owner: 40, region: 7, task_forces: vec![100, 200, 100, 0], ..Default::default() },
        NavalMissionLedgerRow { owner: 50, region: 7, task_forces: vec![300], ..Default::default() },
    ]));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow { id: 40, canonical_slot: 4, ..Default::default() },
        CountryRow { id: 50, canonical_slot: 5, ..Default::default() },
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
        CountryLinkRow { id: 40, canonical_slot: 4 },
        CountryLinkRow { id: 50, canonical_slot: 5 },
    ]));
    *frame.tables[TableId::AirCountryRelations.index()] = TableView::Complete(TableRows::AirCountryRelations(vec![
        AirCountryRelationRow { country: 4, kind: AirCountryRelationRow::ENEMY, ordinal: 0, other: 50 },
    ]));
    let counts = ship_counts(&frame, 4, &mut ShipInputs { regions: BTreeSet::from([7]), ..ShipInputs::default() });
    assert_eq!(counts.own.as_ref().unwrap().get(&7), Some(&5), "duplicate task-force registrations count each time as in the panel");
    assert_eq!(counts.enemy.as_ref().unwrap().get(&7), Some(&1));

    let relations = frame.tables[TableId::AirCountryRelations.index()].clone();
    *frame.tables[TableId::AirCountryRelations.index()] = TableView::Unavailable { reason: "not recorded".into() };
    let counts = ship_counts(&frame, 4, &mut ShipInputs { regions: BTreeSet::from([7]), ..ShipInputs::default() });
    assert_eq!(counts.own.as_ref().unwrap().get(&7), Some(&5), "an unknown enemy roster withholds only the enemy count");
    assert!(counts.enemy.is_err());
    frame.tables[TableId::AirCountryRelations.index()] = relations;

    *frame.tables[TableId::NavalMissionLedgers.index()] = TableView::Unavailable { reason: "not recorded".into() };
    let counts = ship_counts(&frame, 4, &mut ShipInputs { regions: BTreeSet::from([7]), ..ShipInputs::default() });
    assert!(counts.own.is_err() && counts.enemy.is_err(), "missing ledger table refuses both sides instead of returning zero");
}

#[test]
fn naval_supremacy_legend_includes_only_states_present_in_displayed_regions() {
    let region = SupremacyRegion {
        id: "7".into(),
        name: "North Sea".into(),
        pinned: false,
        own_ships: MissionShips::Counted { ships: 5 },
        enemy_ships: MissionShips::Counted { ships: 1 },
        readout: None,
        runs: vec![SupremacyRun { first_day: 0, last_day: 2, control: Some(1), ..Default::default() }],
    };
    let legend = supremacy_legend(&[region], "German Reich");
    assert_eq!(legend.len(), 1);
    assert_eq!(legend[0].id, "friendly");
    assert_eq!(legend[0].label, "German Reich and friends");
}

fn gapped_navy(case: &str) -> (std::sync::Arc<Source>, LoadContext, std::path::PathBuf) {
    use dashboard_base::{
        Source,
        test_fixtures::{content_identity, records, source, with_facts},
    };
    use replay_capture_spool::spool::Record;
    use replay_data::rows::{CountryLinkRow, CountryRow, NavalMissionLedgerRow, NavalSupremacyRow, ShipRow, TableSchema};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let root = std::env::temp_dir().join(format!("naval-supremacy-{case}-{}", std::process::id()));
    std::fs::create_dir_all(root.join("user")).unwrap();
    for (path, text) in [("hoi4.exe", "test"), ("user/settings.txt", "language=l_english force_pow2_textures=no"), ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#)] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let (options, identity) = content_identity(&root);

    let make_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1));
        let set_complete = |f: &mut DecodedFrame, idx: usize, rows| {
            *f.tables[idx] = TableView::Complete(rows);
            f.metrics.table_ns[idx] = 1;
        };
        set_complete(
            &mut frame,
            TableId::Countries.index(),
            tables::Countries::wrap(vec![
                CountryRow { id: 40, canonical_slot: 4, tag: b"GER".to_vec(), ..Default::default() },
                CountryRow { id: 50, canonical_slot: 5, tag: b"ENG".to_vec(), ..Default::default() },
            ]),
        );
        set_complete(
            &mut frame,
            TableId::CountryLinks.index(),
            tables::CountryLinks::wrap(vec![CountryLinkRow { id: 40, canonical_slot: 4 }, CountryLinkRow { id: 50, canonical_slot: 5 }]),
        );
        set_complete(
            &mut frame,
            TableId::AirCountryRelations.index(),
            tables::AirCountryRelations::wrap(vec![AirCountryRelationRow { country: 4, kind: AirCountryRelationRow::ENEMY, ordinal: 0, other: 50 }]),
        );
        set_complete(
            &mut frame,
            TableId::Ships.index(),
            tables::Ships::wrap(vec![
                ShipRow { id: 1, task_force: 100, ..Default::default() },
                ShipRow { id: 2, task_force: 100, ..Default::default() },
                ShipRow { id: 3, task_force: if hour < 30 { 300 } else { 100 }, ..Default::default() },
            ]),
        );
        set_complete(
            &mut frame,
            TableId::NavalMissionLedgers.index(),
            tables::NavalMissionLedgers::wrap(vec![
                NavalMissionLedgerRow { owner: 40, region: 7, task_forces: vec![100], ..Default::default() },
                NavalMissionLedgerRow { owner: 50, region: 7, task_forces: vec![300], ..Default::default() },
            ]),
        );
        set_complete(&mut frame, TableId::NavalSupremacy.index(), tables::NavalSupremacy::wrap(vec![NavalSupremacyRow {
            country: 40, region: 7, current: i64::from(hour + 1), ..Default::default()
        }]));
        let supremacy = &mut frame.tables[TableId::NavalSupremacy.index()];
        *supremacy = supremacy.clone().with_sampled_at(Some(hour));
        set_complete(&mut frame, TableId::FactionMembers.index(), tables::FactionMembers::wrap(vec![]));
        set_complete(&mut frame, TableId::Subjects.index(), tables::Subjects::wrap(vec![]));
        set_complete(&mut frame, TableId::WarRelations.index(), tables::WarRelations::wrap(vec![]));
        set_complete(&mut frame, TableId::RelationTuples.index(), tables::RelationTuples::wrap(vec![]));
        frame
    };

    let mut frames = Vec::new();
    for h in 0..=10 {
        frames.push(make_frame(h));
    }
    for h in 30..=32 {
        frames.push(make_frame(h));
    }
    let mut input = records(1, frames);
    let Record::SessionStart(session) = &mut input[0] else { unreachable!() };
    session.identity = identity.clone();
    let replay = with_facts(&options, &identity, input).into_timelines(&Default::default()).unwrap();
    let src = Arc::new(Source::from_replay(dashboard_base::SourceKey { source_id: case.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap());
    let load = LoadContext { content: Some(options) };
    (src, load, root)
}

fn gapped_panel(source: &std::sync::Arc<Source>, load: &LoadContext) -> Box<dyn dashboard_follow::panel::Panel> {
    playback::panel(source, load, &serde_json::json!({ "branchId": "1", "country": "GER", "regions": [7] })).unwrap()
}

#[test]
fn ships_at_end_uses_last_recorded_day_hour_when_window_ends_in_a_gap() {
    let (src, load, root) = gapped_navy("gap");
    let reply = NavalSupremacy::load(
        &src,
        &NavalSupremacyParams {
            branch_id: "1".into(),
            country: Country::Tag("GER".into()),
            window: None,
            first_hour: Some(0),
            last_hour: Some(20),
            regions: Some(vec![7]),
            pinned_regions: None,
            hidden_regions: None,
            top: None,
        },
        &load,
        &AtomicBool::new(false),
    )
    .unwrap();
    let region = reply.regions.iter().find(|r| r.id == "7").unwrap();
    assert_eq!(region.own_ships, MissionShips::Counted { ships: 2 });
    assert_eq!(region.enemy_ships, MissionShips::Counted { ships: 1 });
    assert_ne!(reply.viewer_name_missing.as_deref(), Some("The recording holds no hour here"), "the viewer is named at the readout's hour, not inside the gap");

    let panel = gapped_panel(&src, &load);
    let exact = panel.exact().unwrap();
    let cancelled = AtomicBool::new(false);
    let steps = dashboard_base::test_fixtures::played(src.replay_ref(), exact, 0, 12);
    let gap = steps.iter().find(|step| step.hour == 12).unwrap();
    assert_eq!(*gap.value, (exact.cold)(12, &cancelled));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_new_run_projects_its_own_naval_rows() {
    let (src, load, root) = gapped_navy("run");
    let panel = gapped_panel(&src, &load);
    let exact = panel.exact().unwrap();
    let cancelled = AtomicBool::new(false);
    let seed = (exact.advance)(0, true, &cancelled, None);
    assert!(seed.lanes.iter().any(|lane| lane.table == TableId::Ships.index()), "playback declares the ships it reads");
    let peer = gapped_panel(&src, &load);
    let same = (peer.exact().unwrap().advance)(0, false, &cancelled, None);
    assert!(std::sync::Arc::ptr_eq(&seed.value, &same.value), "identical subscriptions share the projected reply");
    let first_run = dashboard_base::queries::query(&src, 1).unwrap().timeline.at(0).unwrap().run;
    let later_run = dashboard_base::queries::query(&src, 1).unwrap().timeline.at(30).unwrap().run;
    assert_ne!(first_run, later_run);
    let steps = dashboard_base::test_fixtures::played(src.replay_ref(), exact, 0, 32);
    for step in steps.iter().filter(|step| step.hour >= 30) {
        assert!(!step.pending);
        assert_eq!(*step.value, (exact.cold)(step.hour, &cancelled));
        assert_eq!(step.value.as_ref().as_ref().unwrap()["regions"][0]["ownShips"]["ships"], 3);
        assert_eq!(step.value.as_ref().as_ref().unwrap()["regions"][0]["runs"][0]["own"], (step.hour + 1).to_string());
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn seeking_back_rebuilds_the_naval_projection() {
    let (src, load, root) = gapped_navy("seek");
    let panel = gapped_panel(&src, &load);
    let exact = panel.exact().unwrap();
    let cancelled = AtomicBool::new(false);
    assert!((exact.advance)(30, true, &cancelled, None).value.is_ok());
    let sought = (exact.advance)(0, true, &cancelled, None);
    assert_eq!(*sought.value, (exact.cold)(0, &cancelled));
    assert_eq!(sought.value.as_ref().as_ref().unwrap()["regions"][0]["ownShips"]["ships"], 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failed_naval_read_can_be_rebuilt() {
    let (src, load, root) = gapped_navy("error");
    let panel = gapped_panel(&src, &load);
    let exact = panel.exact().unwrap();
    let cancelled = AtomicBool::new(false);
    assert!((exact.advance)(0, true, &cancelled, None).value.is_ok());
    let failed = (exact.advance)(5, true, &AtomicBool::new(true), None);
    assert!(failed.value.is_err() && failed.lanes.is_empty());
    assert_eq!(*(exact.advance)(5, false, &cancelled, None).value, (exact.cold)(5, &cancelled));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn another_panels_ship_lane_does_not_make_an_empty_selection_read_ships() {
    use std::sync::Arc;

    let (src, load, root) = gapped_navy("unneeded-ships");
    let pinned = gapped_panel(&src, &load);
    let ships = (pinned.exact().unwrap().advance)(0, true, &AtomicBool::new(false), None)
        .lanes.into_iter().find(|lane| lane.table == TableId::Ships.index()).unwrap();
    let panel = playback::panel(&src, &load, &serde_json::json!({ "branchId": "1", "country": "GER", "regions": [] })).unwrap();
    let exact = panel.exact().unwrap();
    let advance = exact.advance.clone();
    let checked = dashboard_follow::panel::AtHour {
        advance: Arc::new(move |hour, seed, cancelled, step| {
            let mut read = advance(hour, seed, cancelled, step);
            if let Some(step) = step {
                assert!(step.lanes.get(ships.table).is_some());
                assert!(!read.lanes.iter().any(|lane| lane.table == ships.table));
            } else {
                read.lanes.push(ships.clone());
            }
            read
        }),
        cold: exact.cold.clone(),
        read: exact.read.clone(),
    };
    dashboard_base::test_fixtures::proven_by_steps(src.replay_ref(), &checked, 0, 1, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_automatic_regions_are_ranked_by_the_score_of_the_day_read_at_the_hour() {
    let run = |first_day, last_day, friendly: &str, enemy: &str| SupremacyRun { first_day, last_day, control: Some(0), friendly: Some(friendly.into()), enemy: Some(enemy.into()), share: None, own: None, missing: None };
    let busy_before = vec![run(0, 4, "900", "900"), run(5, 5, "1", "0")];
    let busy_now = vec![run(0, 4, "0", "0"), run(5, 5, "40", "2")];
    let idle = vec![run(0, 5, "0", "0")];
    let regions = [(7, busy_before), (8, busy_now), (9, idle)];
    assert_eq!(ranked(regions.iter().map(|(id, runs)| (*id, activity_on(runs, 5)))), Ok(vec![8, 7]), "the day read at the hour decides, not the days before it; an idle region is not ranked");
    assert_eq!(ranked(regions.iter().map(|(id, runs)| (*id, activity_on(runs, 2)))).map(|order| order[..1].to_vec()), Ok(vec![7]));
    let unread = vec![SupremacyRun { first_day: 0, last_day: 5, control: None, friendly: None, enemy: Some("3".into()), share: None, own: None, missing: None }];
    assert!(ranked([(7, activity_on(&regions[0].1, 5)), (10, activity_on(&unread, 5))].into_iter()).is_err(), "a candidate whose score is not recorded leaves the rank unproven");
}

#[test]
fn a_region_a_truncated_hour_did_not_reach_is_not_recorded_with_why_never_zero() {
    let viewer = 7_u32;
    let key = |holder: u32, region: u32| u128::from(holder) << 32 | u128::from(region);
    let n = Cell::Number;
    let row = |friendly| Some(Box::from([n(friendly), n(1), n(friendly), n(1), n(0), Cell::Text(0), Cell::Text(0)]));
    let step = |hour, cells| lanes::Step { hour, cells };
    let history = lanes::History {
        columns: COLUMNS.to_vec(),
        to: 71,
        texts: vec![Box::from(&b""[..])],
        keys: vec![(key(viewer, 1), vec![step(0, row(40)), step(24, None), step(48, row(50))]), (key(viewer, 2), vec![step(0, row(30)), step(24, row(60))])],
        statuses: vec![
            lanes::StatusRun { first: 0, last: 23, status: Status::Complete },
            lanes::StatusRun { first: 24, last: 47, status: Status::Truncated { missing: 3 } },
            lanes::StatusRun { first: 48, last: 71, status: Status::Complete },
        ],
        ..Default::default()
    };
    let sea = SeaContent { sea: Ok(BTreeSet::from([1, 2])), land: Ok(BTreeSet::new()), all: BTreeSet::from([1, 2]), minima: (1..=2).map(|region| (region, Ok(1_000))).collect(), ratio: Ok(200_000), notes: Vec::new() };
    let mut pieces = BTreeMap::new();
    region_runs(&history, viewer, &[(0, 23), (1, 47), (2, 71)], &BTreeSet::new(), &sea, &mut Refusals::default(), &mut HashMap::new(), &mut pieces);
    let at = |region: u32, day: u32| pieces[&region].iter().find(|run: &&(u32, u32, Values)| run.0 <= day && day <= run.1).unwrap().2.clone();
    assert_eq!(at(2, 1).friendly, Some(60), "the truncated read reached region 2: its value stands");
    let unread = at(1, 1);
    assert_eq!((unread.friendly, unread.control), (None, None), "region 1 was not reached: no zero, no earlier value");
    assert!(unread.missing.is_some_and(|reason| !reason.is_empty()));
    assert_eq!(at(1, 2).friendly, Some(50));
    assert_eq!(activity_on(&runs(pieces.clone())[&1], 1), None, "an unread region leaves the rank unproven");
}

#[test]
fn a_viewers_read_continued_by_a_later_day_is_the_read_made_whole() {
    let viewer = 7_u32;
    let key = |holder: u32, region: u32| u128::from(holder) << 32 | u128::from(region);
    let n = Cell::Number;
    let row = |friendly, refusal: u32| Some(Box::from([n(friendly), n(1), n(friendly), n(1), n(0), Cell::Text(refusal), Cell::Text(0)]));
    let step = |hour, cells| lanes::Step { hour, cells };
    let history = lanes::History {
        columns: COLUMNS.to_vec(),
        to: 95,
        texts: vec![Box::from(&b""[..]), Box::from(&b"Naval current owner identity is unresolved"[..])],
        keys: vec![(key(viewer, 1), vec![step(0, row(40, 1)), step(60, row(50, 1))]), (key(viewer, 2), vec![step(0, row(30, 0))])],
        statuses: vec![lanes::StatusRun { first: 0, last: 95, status: Status::Complete }],
        ..Default::default()
    };
    let sea = SeaContent { sea: Ok(BTreeSet::from([1, 2])), land: Ok(BTreeSet::new()), all: BTreeSet::from([1, 2]), minima: (1..=2).map(|region| (region, Ok(1_000))).collect(), ratio: Ok(200_000), notes: Vec::new() };
    let days = [(0, 23), (1, 47), (2, 71), (3, 95)];
    let read = |parts: &[&[(u32, i32)]]| {
        let (mut notes, mut made, mut pieces) = (Refusals::default(), HashMap::new(), BTreeMap::new());
        for days in parts {
            region_runs(&history, viewer, days, &BTreeSet::from([1, 2]), &sea, &mut notes, &mut made, &mut pieces);
        }
        (runs(pieces), notes.lines("naval supremacy fields"))
    };
    assert_eq!(read(&[&days[..2], &days[2..]]), read(&[&days]), "a refusal carried across the junction is noted once, as in one read");
}

#[test]
fn a_history_continued_by_a_later_day_reads_the_hours_between_and_is_the_history_read_whole() {
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    let viewer = 7_u32;
    let key = |holder: u32, region: u32| u128::from(holder) << 32 | u128::from(region);
    let n = Cell::Number;
    let row = |friendly| Some(Box::from([n(friendly), n(1), n(friendly), n(1), n(0), Cell::Text(0), Cell::Text(0)]));
    let step = |hour, cells| lanes::Step { hour, cells };
    let recorded = [
        (key(viewer, 1), vec![step(0, row(40)), step(60, row(50))]),
        (key(viewer, 9), vec![step(30, row(70)), step(41, None)]),
        (key(NavalRegionControlRow::EVERY_VIEWER, 2), vec![step(0, row(30))]),
    ];
    let mut reads = Vec::new();
    let mut reader = |_run: u32, _viewer: u32, only: Option<&BTreeSet<u32>>, from: i32, to: i32| -> io::Result<lanes::History> {
        reads.push((from, to));
        let keys = recorded
            .iter()
            .filter(|(key, _)| only.is_none_or(|only| only.contains(&(*key as u32))))
            .filter_map(|(key, steps)| {
                let carried = steps.iter().rev().find(|step| step.hour < from).and_then(|step| step.cells.clone()).filter(|_| !steps.iter().any(|step| step.hour == from));
                let held: Vec<lanes::Step> = carried.map(|cells| step(from, Some(cells))).into_iter().chain(steps.iter().filter(|step| from <= step.hour && step.hour <= to).cloned()).collect();
                (!held.is_empty()).then_some((*key, held))
            })
            .collect();
        Ok(lanes::History { columns: COLUMNS.to_vec(), to, texts: vec![Box::from(&b""[..])], keys, statuses: vec![lanes::StatusRun { first: 0, last: 95, status: Status::Complete }], ..Default::default() })
    };
    let timeline = Timeline { id: 1, first: 0, last: 95, stretches: vec![Stretch { run: 0, first: 0, last: 95, join: Join::Start }], unheld: Vec::new() };
    let holders = Holders::new([("GER".to_owned(), vec![HolderRun { first: 0, last: 95, holder: Holder::Found { slot: viewer, id: viewer }, starts_segment: true }])].into(), Vec::new());
    let sea = SeaContent { sea: Ok(BTreeSet::from([1, 2])), land: Ok(BTreeSet::new()), all: BTreeSet::from([1, 2]), minima: (1..=2).map(|region| (region, Ok(1_000))).collect(), ratio: Ok(200_000), notes: Vec::new() };
    let country = Country::Tag("GER".into());
    let cancelled = AtomicBool::new(false);
    let shown = |history: &NavalHistory| (runs(history.pieces.clone()), history.known.clone(), history.refused.clone(), history.notes.lines("naval supremacy fields"));
    let mut whole = NavalHistory::default();
    whole.read(&timeline, &holders, &sea, &country, 0, (0, 71), &mut reader, &cancelled).unwrap();
    let mut held = NavalHistory::default();
    for span in [(0, 23), (24, 47), (48, 71)] {
        held.read(&timeline, &holders, &sea, &country, 0, span, &mut reader, &cancelled).unwrap();
    }
    assert!(shown(&held).0.contains_key(&9), "a region whose row came and went between two days' readings is still found");
    assert_eq!(shown(&held), shown(&whole));
    assert_eq!(reads[1..], [(23, 23), (24, 47), (23, 23), (48, 71)], "each continuation reads the hours since the last read, and only the new region's earlier days are read again");
}
