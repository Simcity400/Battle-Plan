use super::*;
use crate::products::sinkings::work::{WORK, Work};
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    observer_tests,
};
use replay_data::{rows::SunkShipEntry, rows::SunkShipsRow};

fn input(first: i32, ships: usize, results: usize) -> DecodedFrame {
    let mut frame = source(1, first, Some(1));
    for table in [
        TableId::NavalCombats,
        TableId::NavalCombatParticipants,
        TableId::NavalCombatDetails,
        TableId::NavalResults,
        TableId::NavalResultMembers,
        TableId::NavalResultCountries,
        TableId::NavalCombatResultDetails,
        TableId::NavalResultSources,
        TableId::TaskForces,
        TableId::Fleets,
        TableId::SunkConvoys,
    ] {
        frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
    }
    frame.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(
        (1..=results)
            .map(|id| NavalResultRow {
                id: id as u64,
                province: 45,
                result_hour: first - 1,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: (0..ships).map(|index| sinking(first, index)).collect(),
        }]))
        .into();
    frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
    frame.metrics.table_ns[TableId::SunkConvoys.index()] = 1;
    frame
}
fn sinking(hour: i32, index: usize) -> SunkShipEntry {
    SunkShipEntry {
        ship_name: format!("synthetic ship {index}").into_bytes(),
        victim_country: 9001,
        attacker_country: 9001,
        sink_hour: hour,
        class_token: 77,
        ..Default::default()
    }
}
fn configure(scan: &mut Scan, source: &Source, context: &LoadContext) {
    let content = crate::products::ships_sunk::prepared_content(source, context);
    scan.configure(source, 1, &content).unwrap();
}
fn observe(
    scan: &mut Scan,
    source: &Source,
    context: &LoadContext,
    frame: &DecodedFrame,
    tracker: &mut FrameChangeTracker,
) {
    let changes = tracker.observe(frame, source.replay_ref().format);
    observe_changes(scan, source, context, frame, &changes);
}
fn observe_changes(
    scan: &mut Scan,
    source: &Source,
    context: &LoadContext,
    frame: &DecodedFrame,
    changes: &FrameChanges,
) {
    scan.observe_prepared(frame, changes);
    scan.observe_details(source, context, frame, &AtomicBool::new(false), changes)
        .unwrap();
}
fn reset_work() {
    WORK.with(|work| *work.borrow_mut() = Work::default());
}
fn work() -> Work {
    WORK.with(|work| *work.borrow())
}
fn balance_read(frame: &mut DecodedFrame) {
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(i64::from(frame.hour));
    }
}

fn historical_workload(ships: usize, results: usize, hours: usize, pending: bool) -> (Work, Work) {
    let first = crate::products::sinkings::month_start(1940 * 12).unwrap() + 1;
    let mut frame = input(first, ships, results);
    if pending
        && let TableView::Complete(TableRows::SunkShips(rows)) =
            &mut *frame.tables[TableId::SunkShips.index()]
    {
        for (index, ship) in rows[0].entries.iter_mut().enumerate() {
            ship.victim_country = 200000 + index as u32;
        }
    }
    let mut end = frame.clone();
    end.hour = first + 700;
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "naval-work".into(),
            revision: "1".into(),
        },
        "naval-work.battleplan".into(),
        Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone(), end])]).unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    configure(&mut scan, &source, &context);
    observe(&mut scan, &source, &context, &frame, &mut tracker);
    let original = frame
        .complete::<replay_data::rows::tables::SunkShips>()
        .unwrap()[0]
        .entries
        .clone();
    for hour in 1..=hours {
        frame.hour = first + hour as i32;
        balance_read(&mut frame);
        let mut entries = original.clone();
        let mut next = sinking(frame.hour, ships + hour);
        if pending {
            next.victim_country = 200000 + (ships + hour) as u32;
        }
        entries.push(next);
        frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }])).into();
        observe(&mut scan, &source, &context, &frame, &mut tracker);
    }
    reset_work();
    for _ in 0..8 {
        frame.hour += 1;
        balance_read(&mut frame);
        observe(&mut scan, &source, &context, &frame, &mut tracker);
    }
    let unchanged = work();
    // Readonly builders must see deferred supplying-read endpoints.
    if results == 1 {
        assert_eq!(reply(&scan, frame.hour).rows.len(), 1);
    }
    let episode = scan.episodes.last().unwrap();
    assert_eq!(
        scan.run_last(episode, episode.runs.last().unwrap()),
        frame.hour
    );
    reset_work();
    frame.hour += 1;
    balance_read(&mut frame);
    let mut entries = frame
        .complete::<replay_data::rows::tables::SunkShips>()
        .unwrap()[0]
        .entries
        .clone();
    entries.push(sinking(frame.hour, ships + hours + 1000));
    frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }])).into();
    observe(&mut scan, &source, &context, &frame, &mut tracker);
    (unchanged, work())
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "5,000 retained rows and 10x history work regression"
)]
fn naval_observation_work_is_flat_after_ten_times_history_and_5000_ships() {
    let small_short = historical_workload(1, 1, 50, false);
    let small_long = historical_workload(1, 1, 500, false);
    let large_short = historical_workload(5000, 5000, 50, false);
    let large_long = historical_workload(5000, 5000, 500, false);
    let pending_short = historical_workload(5000, 5000, 50, true);
    let pending_long = historical_workload(5000, 5000, 500, true);
    assert_eq!(pending_short.0, Work::default());
    assert_eq!(pending_long.0, Work::default());
    assert_eq!(
        pending_short.1, pending_long.1,
        "pruned pending attribution cannot introduce history-length work"
    );
    assert_eq!(
        pending_long.1, large_long.1,
        "an unrelated sinking never revisits the unresolved history"
    );
    for (unchanged, _) in [small_short, small_long, large_short, large_long] {
        assert_eq!(
            unchanged,
            Work::default(),
            "unchanged event/name/result work must be zero at every history size"
        );
    }
    assert_eq!(
        small_short.1, small_long.1,
        "one sinking cannot revisit 10x more historical work"
    );
    assert_eq!(
        large_short.1, large_long.1,
        "one sinking remains bounded after 10x history with 5,000 retained rows"
    );
    assert_eq!(
        small_short.1, large_long.1,
        "only the new occurrence contributes to event and naming work"
    );
    assert_eq!(
        large_long.1.names, 2,
        "the log and naval details name only the new sinking"
    );
    assert_eq!(
        large_long.1.results, 0,
        "a sinking cannot rebuild unchanged retained battle results"
    );
    assert!(
        large_long.1.events < 64,
        "one sinking visits a bounded number of collector contributions"
    );
}

#[test]
fn naval_content_operands_are_read_once_per_branch_configuration_including_failures_and_resume() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for has_defines in [false, true] {
        let root =
            std::env::temp_dir().join(format!("naval-content-work-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        if has_defines {
            std::fs::write(root.join("common/defines/naval.lua"),
                "NDefines.NNavy.NAVAL_COMBAT_RESULT_TIMEOUT_YEARS = 2\nNDefines.NNavy.CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS = 24\n").unwrap();
        }
        let first = crate::products::sinkings::month_start(1940 * 12).unwrap();
        let mut frame = input(first, 1, 1);
        let (source, context) = dashboard_base::test_fixtures::source_with_content(
            (0..=20)
                .map(|offset| {
                    let mut read = frame.clone();
                    read.hour += offset;
                    read
                })
                .collect(),
            &root,
        );
        let mut scan = Scan::default();
        let mut tracker = FrameChangeTracker::new();
        reset_work();
        configure(&mut scan, &source, &context);
        observe(&mut scan, &source, &context, &frame, &mut tracker);
        for _ in 0..20 {
            frame.hour += 1;
            balance_read(&mut frame);
            observe(&mut scan, &source, &context, &frame, &mut tracker);
        }
        assert_eq!(
            (work().defines, work().units, work().regions),
            (1, 1, 1),
            "successes and refusals are configured once"
        );
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut writer = fixture.writer();
        scan.publish_block(
            &source,
            &context,
            &mut writer,
            1,
            0,
            &AtomicBool::new(false),
        )
        .unwrap();
        dashboard_base::prepared::series::write_reply(&mut writer, "resume", 0, &scan).unwrap();
        writer.commit().unwrap();
        scan =
            dashboard_base::prepared::series::read_reply(&fixture.store, &fixture.key, "resume", 0)
                .unwrap()
                .unwrap();
        scan.resume(&frame);
        tracker.invalidate();
        reset_work();
        configure(&mut scan, &source, &context);
        for _ in 0..20 {
            frame.hour += 1;
            balance_read(&mut frame);
            observe(&mut scan, &source, &context, &frame, &mut tracker);
        }
        assert_eq!(
            (work().defines, work().units, work().regions),
            (0, 0, 0),
            "the binding retains success and failure operands across branch resume"
        );
        drop(scan);
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn shared_work() -> observer_tests::Work {
    let work = work();
    observer_tests::Work {
        content_loads: work.defines + work.units + work.regions,
        row_visits: work.names + work.results + work.convoys + work.fleets + work.combats,
        history_visits: work.events,
        emitted: work.values,
        interpreter_calls: 0,
        dictionary_interpretations: 0,
    }
}

#[test]
fn naval_shared_driver_unchanged_reads_keep_native_work_flat_and_read_proof_exact() {
    use replay_data::{rows::NavalCombatDetailsRow, rows::SunkConvoyRow};
    let scaled = observer_tests::scaled_work(8, |hours| {
        let first = crate::products::sinkings::month_start(1940 * 12).unwrap() + 100;
        let mut frame = input(first, 2, 2);
        frame.tables[TableId::NavalCombats.index()] =
            TableView::Complete(TableRows::NavalCombats(vec![NavalCombatRow {
                id: 1001,
                province: 45,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
                combat: 1001,
                province: 45,
                custom_sea: Some(vec![]),
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::SunkConvoys.index()] = TableView::Complete(TableRows::SunkConvoys(
            (0..128)
                .map(|ordinal| SunkConvoyRow {
                    month: 1940 * 12,
                    owner_country: 9001 + ordinal,
                    killer_country: 9001,
                    ordinal,
                    convoys: 0,
                })
                .collect(),
        ))
        .into();
        let recording: Vec<_> = (0..=hours)
            .map(|offset| {
                let mut read = frame.clone();
                read.hour += offset as i32;
                read
            })
            .collect();
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "naval-shared-driver".into(),
                revision: "1".into(),
            },
            "naval-shared-driver.battleplan".into(),
            Arc::new(Replay::from_records(
                CAMPAIGN.into(),
                &[records(1, recording)],
            )?),
            &AtomicBool::new(false),
        )?;
        let context = LoadContext { content: None };
        let mut scan = Scan::default();
        reset_work();
        configure(&mut scan, &source, &context);
        observe_changes(
            &mut scan,
            &source,
            &context,
            &frame,
            &FrameChangeTracker::new().observe(&frame, source.replay_ref().format),
        );
        assert_eq!(work().convoys, 128);
        assert_eq!(work().fleets, 1);
        assert_eq!(work().combats, 1);
        let baseline = shared_work();
        let mut last = frame.hour;
        let delta = observer_tests::unchanged_frames(
            baseline,
            &frame,
            hours as i32,
            source.replay_ref().format,
            |read, changes| {
                observe_changes(&mut scan, &source, &context, read, changes);
                last = read.hour;
                Ok(shared_work())
            },
        )?;
        assert_eq!(
            delta,
            observer_tests::Work::default(),
            "fresh equal reads are clocks, not another native join"
        );
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut writer = fixture.writer();
        scan.publish_block(
            &source,
            &context,
            &mut writer,
            1,
            0,
            &AtomicBool::new(false),
        )?;
        writer.commit()?;
        let flag = AtomicBool::new(false);
        let mut reads = dashboard_base::prepared::series::Reader::new(
            &fixture.store,
            &fixture.key,
            "naval-loss-reads-1".into(),
            &flag,
        )?;
        let proof = reads.events(&[1], first, last, 0, u32::MAX)?;
        assert_eq!(proof.rows.last().unwrap().hour, last);
        assert_eq!(proof.rows.len(), hours + 1);
        let naval = scan.reply_window(
            0,
            &BattleWindow {
                first_hour: first,
                last_hour: last,
            },
        )?;
        let oracle =
            crate::products::naval_battles::scan(source.replay_ref(), 1, &AtomicBool::new(false))?;
        observer_tests::assert_reply_parity(
            &naval,
            &oracle.reply_window(
                0,
                &BattleWindow {
                    first_hour: first,
                    last_hour: last,
                },
            )?,
        )?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "shared 5,000 row and 10x history work driver"
)]
fn naval_shared_driver_one_sinking_work_is_flat_after_ten_times_history() {
    for ships in [1, 5000] {
        let work = observer_tests::scaled_work(50, |history| {
            historical_workload(ships, ships, history, false);
            Ok(shared_work())
        })
        .unwrap();
        observer_tests::assert_flat(work);
        assert_eq!(work[0].row_visits, 2);
        assert_eq!(work[0].history_visits, 13);
        assert!(work[0].emitted <= 12);
    }
}

#[test]
fn naval_shared_driver_census_parity_across_held_rows_changes_gaps_and_country_refresh() {
    let initial = super::frame(100);
    let mut held = initial.clone();
    held.hour = 101;
    let mut changed = held.clone();
    changed.hour = 102;
    if let TableView::Complete(TableRows::NavalCombatParticipants(rows)) =
        &mut *changed.tables[TableId::NavalCombatParticipants.index()]
    {
        rows[0].state ^= 0x20;
    }
    let mut gap = changed.clone();
    gap.hour = 104;
    let mut renamed = gap.clone();
    renamed.hour = 105;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"synthetic renamed actor".to_vec();
    }
    let mut unavailable = renamed.clone();
    unavailable.hour = 106;
    unavailable.tables[TableId::NavalCombatParticipants.index()] = TableView::Unavailable {
        reason: "synthetic missing participant census".into(),
    }
    .into();
    let mut carried = unavailable.clone();
    carried.hour = 107;
    let mut recovered = renamed.clone();
    recovered.hour = 108;
    let mut prepared = Scan::default();
    let mut oracle = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for (index, frame) in [
        initial,
        held,
        changed,
        gap,
        renamed,
        unavailable,
        carried,
        recovered,
    ]
    .iter()
    .enumerate()
    {
        let before = work().combats;
        prepared.observe_prepared(
            frame,
            &tracker.observe(frame, replay_data::format::FormatVersion::default()),
        );
        if index == 1 || index == 6 {
            assert_eq!(
                work().combats - before,
                0,
                "equal active samples need only accepted coverage"
            );
        }
        oracle.observe(frame);
        let window = BattleWindow {
            first_hour: 100,
            last_hour: frame.hour,
        };
        observer_tests::assert_reply_parity(
            &prepared.reply_window(0, &window).unwrap(),
            &oracle.reply_window(0, &window).unwrap(),
        )
        .unwrap();
    }
}
