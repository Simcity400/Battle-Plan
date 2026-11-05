use super::*;
use crate::prepared::frame_changes::{FrameChangeTracker, FrameChanges};
use crate::products::event_feed::prepared::{KeptFrames, tests::frame as fixture_frame};
use dashboard_base::{
    prepared::series::tests::{Fixture, PublishedPayloads, assert_payloads_equal},
    test_fixtures::{CAMPAIGN, records},
};
use replay::columnar_frames::synthetic::{Input as TableInput, Sample, Synthetic};
use replay_data::{
    event_feed_rows::{
        DECISION_FAMILY_REFUSALS, EventFeedCountry, EventFeedCounts, EventFeedEvent,
        EventFeedEventIdentity, EventFeedScript,
    },
    rows::{
        CountryPowerChangeRow, PoliticsCaptureCoverageRow, Recorded, RelationTupleRow, SliceReadRow,
    },
};

/// One branch's feed folded hour by hour from its rows: the identity history, then the feed
/// naming its countries through it. The parity oracle of the projection.
struct FeedReference {
    identities: HistoryScan,
    feed: Observer,
}

impl Default for FeedReference {
    fn default() -> Self {
        Self::from_parts(HistoryScan::new(false, &[]), Observer::default())
    }
}

impl FeedReference {
    fn from_parts(identities: HistoryScan, feed: Observer) -> Self {
        Self { identities, feed }
    }

    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.identities
            .observe_owned_states(frame, Some(changes), |_, _, _| {});
        self.feed
            .observe(replay, frame, &self.identities, &AtomicBool::new(false))
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        write_identities(writer, branch, &mut self.identities)?;
        self.feed.flush(writer, branch, cancelled)?;
        save_feed(writer, branch, &self.feed)
    }
}

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = fixture_frame(hour, count);
    for rows in [
        TableRows::CountryPowerChanges(Vec::new()),
        TableRows::PoliticsCaptureCoverage(Vec::new()),
    ] {
        let table = rows.table();
        frame.tables[table.index()] = TableView::Complete(rows).into();
    }
    for &table in OWN {
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, unread: Option<TableId>) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0 && unread != Some(T::ID);
    let sample = if read {
        Sample::FromReadBit
    } else {
        Sample::Carried(frame.hour - 1)
    };
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => TableInput::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => TableInput::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => TableInput::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => TableInput::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame, unread: Option<TableId>) {
    write::<tables::Campaign>(walk, frame, unread);
    write::<tables::Countries>(walk, frame, unread);
    write::<tables::CountryLinks>(walk, frame, unread);
    write::<tables::CountryCounterInputs>(walk, frame, unread);
    write::<tables::CountryExperienceGains>(walk, frame, unread);
    write::<tables::RelationTuples>(walk, frame, unread);
    write::<tables::SliceReads>(walk, frame, unread);
    write::<tables::EventFeedHistory>(walk, frame, unread);
    write::<tables::NuclearStrikes>(walk, frame, unread);
    write::<tables::CountryPowerChanges>(walk, frame, unread);
    write::<tables::PoliticsCaptureCoverage>(walk, frame, unread);
}

fn emit(hours: &mut Vec<Hour>, state: &mut Prepared, fixture: &Fixture, flushes: &BTreeSet<i32>) {
    for hour in std::mem::take(hours) {
        let at = hour.hour;
        state.observe_projected(hour, None).unwrap();
        if flushes.contains(&at) {
            let mut writer = fixture.writer();
            state
                .flush(&mut writer, 1, &AtomicBool::new(false))
                .unwrap();
            state.save_state(&mut writer, 1).unwrap();
            writer.commit().unwrap();
        }
    }
}

/// The identity history and feed a fixture's last flush saved.
fn saved(fixture: &Fixture) -> (HistoryScan, Observer) {
    let identities = fixture
        .store
        .read(&fixture.key, "event-feed-identities-1", 0)
        .unwrap()
        .unwrap();
    let feed = fixture
        .store
        .read(&fixture.key, "event_feed-continuation-1", 0)
        .unwrap()
        .unwrap();
    (
        serde_json::from_reader(io::BufReader::new(identities)).unwrap(),
        serde_json::from_reader(io::BufReader::new(
            zstd::stream::Decoder::new(feed).unwrap(),
        ))
        .unwrap(),
    )
}

fn parity(
    inputs: &[DecodedFrame],
    restarts: &[usize],
    unread: Option<(i32, TableId)>,
) -> PublishedPayloads {
    parity_with_resume(inputs, restarts, unread, false)
}

/// Every section the row fold writes, written alike by segment walks starting at `restarts`
/// and at each later run. With `resume`, both continue at each restart after a flush from
/// their saved sections alone, the columnar side with nothing else of its earlier walk.
fn parity_with_resume(
    inputs: &[DecodedFrame],
    restarts: &[usize],
    unread: Option<(i32, TableId)>,
    resume: bool,
) -> PublishedPayloads {
    parity_with(inputs, restarts, unread, resume, 3)
}

/// [`parity_with_resume`], flushing after every `every` inputs and the last.
fn parity_with(
    inputs: &[DecodedFrame],
    restarts: &[usize],
    unread: Option<(i32, TableId)>,
    resume: bool,
    every: usize,
) -> PublishedPayloads {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, inputs.to_vec())]).unwrap();
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut row = FeedReference::default();
    let mut projected = Prepared::new(&replay, 1);
    let mut walk = Synthetic::new(OWN, 1, inputs[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let mut projector = Projector::new(&replay).unwrap();
    let mut hours: Vec<Hour> = Vec::new();
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].hour)
        .unwrap();
    let mut tracker = FrameChangeTracker::new();
    let mut kept = KeptFrames::default();
    let flushes: BTreeSet<_> = inputs
        .iter()
        .enumerate()
        .filter(|(index, _)| index % every == every - 1 || *index == inputs.len() - 1)
        .map(|(_, frame)| frame.hour)
        .collect();
    for (index, input) in inputs.iter().enumerate() {
        let run = index > 0 && inputs[index - 1].hour.checked_add(1) != Some(input.hour);
        let opens = run || restarts.contains(&index);
        if opens {
            emit(&mut hours, &mut projected, &fixtures[1], &flushes);
            if resume && index > 0 && flushes.contains(&inputs[index - 1].hour) {
                let (identities, feed) = saved(&fixtures[0]);
                row = FeedReference::from_parts(identities, feed);
                let (identities, feed) = saved(&fixtures[1]);
                projected = Prepared::restore(&replay, 1, identities, feed);
            }
            projector = Projector::new(&replay).unwrap();
            if run {
                walk = Synthetic::new(OWN, 1, input.hour).unwrap();
                projector
                    .start(Start::Run { continues: true }, 1, input.hour)
                    .unwrap();
            } else {
                projector
                    .start(Start::Restart(walk.state()), 1, input.hour)
                    .unwrap();
            }
            // Production drops the decoder, tracker and kept-frame cache, retaining both histories.
            tracker.invalidate();
            kept = KeptFrames::default();
        }
        write_hour(
            &mut walk,
            input,
            unread
                .filter(|(hour, _)| *hour == input.hour)
                .map(|(_, t)| t),
        );
        walk.end(|state, manifest, decoded, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &super::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            let changes = tracker.observe(decoded, replay.format);
            let frame = kept.kept(decoded);
            let changes = dashboard_base::prepared::kept::kept_changes(&changes);
            row.observe(&replay, &frame, &changes).unwrap();
            if flushes.contains(&frame.hour) {
                let mut writer = fixtures[0].writer();
                row.flush(&mut writer, &replay, 1, &AtomicBool::new(false))
                    .unwrap();
                writer.commit().unwrap();
            }
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: u32::from(run),
                        ordinal: index as u32,
                        starts,
                        opens: starts || opens,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
    }
    emit(&mut hours, &mut projected, &fixtures[1], &flushes);
    let expected = fixtures[0].payloads();
    assert_payloads_equal(&fixtures[1].payloads(), &expected, "event feed columnar");
    for section in [
        "event-feed-identities-1",
        "event-feed-facts-1-catalogue",
        "event_feed-continuation-1",
    ] {
        assert!(
            expected.keys().any(|(name, _)| name == section),
            "{section}"
        );
    }
    expected
}

#[test]
fn every_status_and_recovery_matches_at_several_restarts() {
    for &table in OWN.iter().filter(|&&table| table != TableId::Campaign) {
        for status in 0..4 {
            let mut broken = frame(102, 3);
            let rows = match &*broken.tables[table.index()] {
                TableView::Complete(rows) => rows.clone(),
                _ => continue,
            };
            broken.tables[table.index()] = match status {
                0 => TableView::Truncated { rows, missing: 1 },
                1 => TableView::Truncated { rows, missing: 7 },
                2 => TableView::Unavailable {
                    reason: "synthetic refusal".into(),
                },
                _ => TableView::NotRecorded {
                    mode: CaptureMode::Full,
                },
            }
            .into();
            let inputs = [
                frame(100, 3),
                frame(101, 3),
                broken,
                frame(103, 3),
                frame(104, 3),
            ];
            // The strike read at 104 is dated 101: a walk from 102 on leaves it to the emit.
            for restarts in [vec![], vec![1], vec![2], vec![3], vec![1, 3, 4]] {
                parity(&inputs, &restarts, None);
            }
        }
    }
}

#[test]
fn identity_changes_unread_witnesses_aliases_and_later_runs_match() {
    let mut inputs: Vec<_> = (100..=107).map(|hour| frame(hour, 3)).collect();
    for frame in &mut inputs[2..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod / renamed".to_vec();
        }
    }
    for frame in &mut inputs[4..] {
        if let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
            &mut *frame.tables[TableId::CountryExperienceGains.index()]
        {
            rows[0].instance += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *inputs[3].tables[TableId::CountryCounterInputs.index()]
    {
        rows[0].owned_state_count = -1;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *inputs[5].tables[TableId::CountryLinks.index()]
    {
        rows.push(replay_data::rows::CountryLinkRow {
            id: 900_001,
            canonical_slot: rows[0].canonical_slot,
        });
        rows.sort_unstable_by_key(RowCodec::key);
    }
    inputs.extend([frame(110, 3), frame(111, 3), frame(112, 3)]);
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperienceGains,
    ] {
        for restarts in [vec![], vec![2], vec![3], vec![4, 7], vec![1, 3, 8, 9]] {
            parity(&inputs, &restarts, Some((102, table)));
        }
    }
    parity(&[frame(100, 0), frame(101, 0)], &[1], None);
}

/// A walk names occurrences at its own hours through a history it starts cold, as the carried
/// history names them: across renames, a new alias, a new instance and unread witnesses, for
/// occurrences of the read's hour, of the hour before, and of the alias.
#[test]
fn a_walk_names_its_hours_as_the_carried_history_does() {
    let mut inputs: Vec<_> = (100..=109).map(|hour| quiet(hour, 3)).collect();
    for (index, input) in inputs.iter_mut().enumerate() {
        if index >= 3
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod / renamed".to_vec();
        }
        if index >= 5
            && let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
                &mut *input.tables[TableId::CountryExperienceGains.index()]
        {
            rows[1].instance += 1;
        }
        if index >= 6
            && let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *input.tables[TableId::CountryLinks.index()]
        {
            rows.push(replay_data::rows::CountryLinkRow {
                id: 900_001,
                canonical_slot: rows[0].canonical_slot,
            });
            rows.sort_unstable_by_key(RowCodec::key);
        }
        let hour = input.hour;
        let mut now = script(
            10 * index as u64 + 1,
            EventFeedSource::Declaration,
            hour,
            false,
        );
        now.script = None;
        let mut before = script(
            10 * index as u64 + 2,
            EventFeedSource::Annex,
            hour - 1,
            false,
        );
        before.script = None;
        before.actor.reference = 900_001;
        events(input, vec![now, before]);
    }
    for unread in [
        None,
        Some((104, TableId::Countries)),
        Some((106, TableId::CountryExperienceGains)),
    ] {
        for restarts in [
            vec![],
            vec![1],
            vec![3],
            vec![6],
            vec![2, 4, 6, 8],
            vec![7, 9],
        ] {
            parity(&inputs, &restarts, unread);
        }
    }
    parity_with_resume(&inputs, &[3, 6, 9], None, true);
}

fn quiet(hour: i32, countries: u32) -> DecodedFrame {
    let mut out = frame(100, countries);
    out.hour = hour;
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *out.tables[TableId::EventFeedHistory.index()]
    {
        rows[0].read_hour = hour;
    }
    out
}

#[test]
fn one_changed_slot_stays_one_at_fifteen_hundred_countries() {
    let first = quiet(100, 1_500);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])]).unwrap();
    let mut walk = Synthetic::new(OWN, 1, 100).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let mut projector = Projector::new(&replay).unwrap();
    let mut hours: Vec<Hour> = Vec::new();
    projector
        .start(Start::Run { continues: false }, 1, 100)
        .unwrap();
    write_hour(&mut walk, &first, None);
    walk.end(|state, manifest, _, starts| {
        let world = dashboard_world::World::at(
            &mut world_held,
            starts,
            &super::needs().unwrap(),
            (
                replay_data::format::FormatVersion::CURRENT,
                replay_data::scope::CaptureMode::Full,
            ),
            state,
            manifest,
        )
        .unwrap();
        projector
            .hour(
                &segments::Hour {
                    branch: 1,
                    hour: 100,
                    run: 0,
                    ordinal: 0,
                    starts,
                    opens: starts,
                    state,
                    manifest,
                    world,
                },
                &mut hours,
            )
            .unwrap()
    })
    .unwrap();
    hours.clear();
    let mut changed = quiet(101, 1_500);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.command_power_raw += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
        &mut *changed.tables[TableId::CountryExperienceGains.index()]
    {
        for row in rows {
            row.army_raw += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *changed.tables[TableId::CountryCounterInputs.index()]
    {
        rows[777].owned_state_count += 1;
    }
    write_hour(&mut walk, &changed, None);
    walk.end(|state, manifest, _, starts| {
        let world = dashboard_world::World::at(
            &mut world_held,
            starts,
            &super::needs().unwrap(),
            (
                replay_data::format::FormatVersion::CURRENT,
                replay_data::scope::CaptureMode::Full,
            ),
            state,
            manifest,
        )
        .unwrap();
        projector
            .hour(
                &segments::Hour {
                    branch: 1,
                    hour: 101,
                    run: 0,
                    ordinal: 1,
                    starts,
                    opens: false,
                    state,
                    manifest,
                    world,
                },
                &mut hours,
            )
            .unwrap()
    })
    .unwrap();
    let changed = &hours[0].changes;
    assert!(
        !changed.iter().any(|(table, _)| matches!(
            table,
            TableId::Countries | TableId::CountryExperienceGains
        ))
    );
    let (_, change) = changed
        .iter()
        .find(|(table, _)| *table == TableId::CountryCounterInputs)
        .unwrap();
    let Change::Patch { rows, deleted, .. } = &**change else {
        panic!("sparse counter patch")
    };
    assert_eq!(rows.len(), 1);
    assert!(deleted.is_empty());
}

fn script(sequence: u64, source: EventFeedSource, hour: i32, decision: bool) -> EventFeedEvent {
    let actor = |index: u32| EventFeedCountry {
        reference: 100_000 + index,
        id: 100_000 + index,
        instance: 7 + index,
    };
    EventFeedEvent {
        sequence,
        generation: 9,
        hour,
        source,
        actor: actor(if source == EventFeedSource::EventOption {
            1
        } else {
            0
        }),
        target: actor(if decision { 0 } else { 1 }),
        script: Some(EventFeedScript {
            token_start: 0,
            token_len: 7,
            native_event: (!decision).then_some(EventFeedEventIdentity { kind: 1, id: 9 }),
            instance: (!decision).then_some(7),
            option: (source == EventFeedSource::EventOption).then_some(1),
            ..Default::default()
        }),
        cause: Recorded::Value(Default::default()),
    }
}

fn events(frame: &mut DecodedFrame, events: Vec<EventFeedEvent>) {
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *frame.tables[TableId::EventFeedHistory.index()]
    {
        let count = events.len() as u64;
        rows[0].events = events;
        rows[0].script_tokens = b"mod.evt".to_vec();
        rows[0].counts = Some(EventFeedCounts {
            acquired: count,
            settled: count,
            ..Default::default()
        });
    }
}

#[test]
fn native_late_dates_and_open_occurrences_cross_more_than_seven_hundred_twenty_hours() {
    let mut inputs: Vec<_> = (100..=830).map(|hour| quiet(hour, 3)).collect();
    for frame in &mut inputs[100..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod / later name".to_vec();
        }
    }
    events(
        &mut inputs[1],
        vec![
            script(1, EventFeedSource::ScriptEvent, 101, false),
            script(2, EventFeedSource::DecisionSelected, 101, true),
        ],
    );
    let mut late = script(5, EventFeedSource::Declaration, 101, false);
    late.script = None;
    events(
        &mut inputs[729],
        vec![
            script(3, EventFeedSource::EventOption, 828, false),
            script(4, EventFeedSource::DecisionCompleted, 829, true),
            late,
        ],
    );
    for restarts in [vec![], vec![2, 100, 728], vec![1, 3, 720, 729]] {
        let payloads = parity(&inputs, &restarts, None);
        assert!(
            payloads
                .keys()
                .any(|(name, _)| name == "event-feed-entries-1")
        );
    }
    for boundary in [2, 500, 729] {
        let mut broken = inputs.clone();
        if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
            &mut *broken[boundary].tables[TableId::EventFeedHistory.index()]
        {
            rows[0].coverage = EventFeedCoverage::Partial;
        }
        parity(&broken, &[boundary, 729], None);
    }
}

#[test]
fn charged_decisions_keep_the_same_occurrences_after_publication_and_restore() {
    use replay_data::rows::{PowerAction, PowerCause, PowerCurrency, PowerIdentity};
    let mut inputs: Vec<_> = (100..=109).map(|hour| quiet(hour, 3)).collect();
    events(
        &mut inputs[1],
        vec![script(1, EventFeedSource::DecisionSelected, 101, true)],
    );
    events(
        &mut inputs[7],
        vec![script(2, EventFeedSource::DecisionCompleted, 107, true)],
    );
    inputs[1].tables[TableId::CountryPowerChanges.index()] =
        TableView::Complete(TableRows::CountryPowerChanges(vec![
            CountryPowerChangeRow {
                country: 100_000,
                instance: 7,
                sequence: 1,
                hour: 101,
                currency: PowerCurrency::Political,
                balance_before_raw: 1_000_000,
                balance_after_raw: 750_000,
                cause: PowerCause::Decision,
                identity: PowerIdentity {
                    action: PowerAction::Execute,
                    item_key: b"mod.evt".to_vec(),
                    ..Default::default()
                },
                ..Default::default()
            },
        ]))
        .into();
    inputs[1].tables[TableId::PoliticsCaptureCoverage.index()] =
        TableView::Complete(TableRows::PoliticsCaptureCoverage(vec![
            PoliticsCaptureCoverageRow {
                country: 100_000,
                instance: 7,
                since_hour: 100,
                through_hour: 101,
                streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
                ..Default::default()
            },
        ]))
        .into();
    inputs[1].metrics.table_ns[TableId::CountryPowerChanges.index()] = 1;
    inputs[1].metrics.table_ns[TableId::PoliticsCaptureCoverage.index()] = 1;
    for restarts in [vec![1], vec![2], vec![3, 6, 9]] {
        parity_with_resume(&inputs, &restarts, None, true);
    }
}

#[test]
fn partial_counts_and_unavailable_reasons_change_while_the_table_stays_incomplete() {
    let mut inputs: Vec<_> = (100..=106).map(|hour| frame(hour, 3)).collect();
    for (index, missing) in [(1, 1), (2, 9)] {
        inputs[index].tables[TableId::EventFeedHistory.index()] = TableView::Truncated {
            rows: TableRows::EventFeedHistory(Vec::new()),
            missing,
        }
        .into();
    }
    for (index, reason) in [(3, "first reason"), (4, "second reason")] {
        inputs[index].tables[TableId::EventFeedHistory.index()] = TableView::Unavailable {
            reason: reason.into(),
        }
        .into();
    }
    for restarts in [vec![], vec![2], vec![4], vec![2, 4, 5]] {
        parity(&inputs, &restarts, None);
    }
}

fn retained_inputs() -> Vec<DecodedFrame> {
    use replay_data::event_feed_rows::{
        EventFeedScope, EventFeedScriptChange as Change,
        decisions::{DecisionChange, ORDINAL, Operation, TIMED},
    };
    let mut inputs: Vec<_> = (100..=108).map(|hour| quiet(hour, 3)).collect();
    let decision = |operation, ordinal, flags| {
        Change::Decision(DecisionChange {
            operation,
            container: TIMED,
            ordinal,
            flags,
            political_power: None,
        })
    };
    let operations = [
        vec![
            Change::PendingBaseline,
            Change::FiredBaseline,
            Change::DelayedBaseline,
            decision(Operation::Baseline, 0, 0),
        ],
        vec![
            Change::PendingStored { due_hour: 500 },
            Change::FiredMember,
            Change::DelayedStored { remaining: 10 },
            decision(Operation::Store, 0, 0),
        ],
        vec![
            Change::PendingReady,
            Change::FiredReady,
            Change::DelayedReady,
            decision(Operation::Ready, 0, 0),
        ],
        vec![
            decision(Operation::PassBegin, 1, 0),
            Change::DelayedAdvance {
                count: 1,
                finished: false,
            },
        ],
        vec![
            decision(Operation::PassCut, 0, ORDINAL),
            Change::DelayedAdvance {
                count: 0,
                finished: true,
            },
        ],
        vec![decision(Operation::PassEnd, 0, 0)],
        vec![
            Change::PendingRemoved,
            Change::FiredRemoved,
            Change::DelayedRemoved {
                iteration: false,
                visited: false,
            },
            decision(Operation::Erase, 0, 0),
        ],
        vec![],
        vec![],
    ];
    for (index, changes) in operations.into_iter().enumerate() {
        let hour = inputs[index].hour;
        // Each read's occurrences leave room for others a case adds to it.
        let mut sequence = 100 * index as u64 + 1;
        let mut rows = Vec::new();
        for change in changes {
            let stored = matches!(
                change,
                Change::PendingStored { .. } | Change::DelayedStored { .. }
            );
            let definition = stored || matches!(change, Change::FiredMember | Change::FiredRemoved);
            let member = matches!(change, Change::Decision(change) if matches!(change.operation,
                Operation::Store | Operation::Erase | Operation::PassCut));
            let instance = if member {
                Some(17)
            } else if matches!(
                change,
                Change::PendingStored { .. } | Change::PendingRemoved
            ) {
                Some(11)
            } else if matches!(
                change,
                Change::DelayedStored { .. } | Change::DelayedRemoved { .. }
            ) {
                Some(99)
            } else {
                None
            };
            rows.push(EventFeedEvent {
                sequence,
                generation: 9,
                hour,
                source: EventFeedSource::ScriptState,
                actor: EventFeedCountry {
                    reference: 100_000,
                    id: 100_000,
                    instance: 7,
                },
                target: EventFeedCountry {
                    reference: 100_001,
                    id: 100_001,
                    instance: 8,
                },
                cause: Recorded::Value(Default::default()),
                script: Some(EventFeedScript {
                    change: Some(change),
                    token_start: 0,
                    token_len: if definition || member { 7 } else { 0 },
                    native_event: definition.then_some(EventFeedEventIdentity { kind: 1, id: 9 }),
                    instance,
                    scope_start: 0,
                    scope_len: u32::from(stored),
                    days: member.then_some(if index == 4 { 9 } else { 10 }),
                    lifecycle: member.then_some(0),
                    ..Default::default()
                }),
            });
            sequence += 1;
        }
        events(&mut inputs[index], rows);
        if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
            &mut *inputs[index].tables[TableId::EventFeedHistory.index()]
        {
            rows[0].script_scopes = vec![EventFeedScope::default()];
        }
    }
    inputs
}

#[test]
fn restarts_inside_registry_baselines_decision_passes_and_delayed_visits_keep_every_byte() {
    let inputs = retained_inputs();
    parity(&inputs, &[], None);
    for start in 1..inputs.len() {
        parity(&inputs, &[start], None);
    }
    parity(&inputs, &[1, 2, 3, 4, 5, 6, 7, 8], None);
    parity_with_resume(&inputs, &[3, 6], None, true);
    for missing in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperienceGains,
    ] {
        parity(&inputs, &[2, 4, 6], Some((104, missing)));
    }
    let mut lost = inputs.clone();
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *lost[1].tables[TableId::EventFeedHistory.index()]
    {
        rows[0].coverage = EventFeedCoverage::Partial;
    }
    parity(&lost, &[1, 2, 3, 4, 6], None);
    let mut unavailable = inputs.clone();
    unavailable[3].tables[TableId::EventFeedHistory.index()] = TableView::Unavailable {
        reason: "synthetic retained-state loss".into(),
    }
    .into();
    parity(&unavailable, &[2, 3, 4, 6], None);
}

/// A complete read whose capture refused the decision families retires every decision owner
/// at that read; its retained observation lists every part, as the row path lists them.
#[test]
fn a_complete_read_with_refused_families_matches_at_every_restart() {
    let mut inputs = retained_inputs();
    for index in [3, 6] {
        if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
            &mut *inputs[index].tables[TableId::EventFeedHistory.index()]
        {
            assert_eq!(rows[0].coverage, EventFeedCoverage::Complete);
            rows[0].script_refusals = DECISION_FAMILY_REFUSALS;
        }
    }
    for start in 1..inputs.len() {
        parity(&inputs, &[start], None);
    }
    parity_with_resume(&inputs, &[3, 6], None, true);
}

/// A reduction that fails part way through its read keeps the notices made before it, names
/// no occurrence of the read and leaves the occurrence sequence where it was.
#[test]
fn a_reduction_failing_mid_read_drops_the_read_as_the_row_path_does() {
    use replay_data::event_feed_rows::EventFeedScriptChange as Change;
    let mut inputs = retained_inputs();
    let mut declared = script(701, EventFeedSource::Declaration, 107, false);
    declared.script = None;
    let mut ready = script(702, EventFeedSource::ScriptState, 107, false);
    ready.script = Some(EventFeedScript {
        change: Some(Change::PendingReady),
        ..Default::default()
    });
    let mut option = script(703, EventFeedSource::EventOption, 107, false);
    option.target.reference = 999_999;
    events(&mut inputs[7], vec![declared, ready, option]);
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *inputs[7].tables[TableId::EventFeedHistory.index()]
    {
        rows[0].coverage = EventFeedCoverage::Partial;
    }
    for start in 1..inputs.len() {
        parity(&inputs, &[start], None);
    }
    parity_with_resume(&inputs, &[3, 6], None, true);
}

#[test]
fn roster_rebuilds_take_their_owner_from_the_reduction_at_every_restart() {
    use replay_data::event_feed_rows::{
        EventFeedScriptChange as Change,
        decisions::{DecisionChange, Operation},
    };
    let mut inputs = retained_inputs();
    for (index, sequence) in [(3, 350), (8, 850)] {
        let hour = inputs[index].hour;
        let mut rebuilt = script(sequence, EventFeedSource::ScriptState, hour, true);
        rebuilt.target = EventFeedCountry {
            reference: 100_002,
            id: 100_002,
            instance: 9,
        };
        rebuilt.script = Some(EventFeedScript {
            change: Some(Change::Decision(DecisionChange {
                operation: Operation::Rebuild,
                container: 3,
                flags: 0,
                ordinal: 0,
                political_power: Some(5_000),
            })),
            ..Default::default()
        });
        let mut held = match &*inputs[index].tables[TableId::EventFeedHistory.index()] {
            TableView::Complete(TableRows::EventFeedHistory(rows)) => rows[0].events.clone(),
            _ => Vec::new(),
        };
        held.push(rebuilt);
        events(&mut inputs[index], held);
    }
    for start in 1..inputs.len() {
        parity(&inputs, &[start], None);
    }
    parity(&inputs, &[3, 6, 8], None);
    parity_with_resume(&inputs, &[3, 6], None, true);
}

/// A decision change of the retained fixtures' owner, naming fixture country `target` (none at
/// 0), with a member `instance`.
fn decided(
    sequence: u64,
    hour: i32,
    change: replay_data::event_feed_rows::decisions::DecisionChange,
    target: u32,
    instance: Option<u32>,
) -> EventFeedEvent {
    use replay_data::event_feed_rows::{
        EventFeedScriptChange as Change,
        decisions::{DecisionChange, Operation},
    };
    let roster = change.operation == Operation::Rebuild && change.container == 3;
    EventFeedEvent {
        sequence,
        generation: 9,
        hour,
        source: EventFeedSource::ScriptState,
        actor: EventFeedCountry {
            reference: 100_000,
            id: 100_000,
            instance: 7,
        },
        target: match target {
            0 => EventFeedCountry::default(),
            _ => EventFeedCountry {
                reference: target,
                id: target,
                instance: target - 100_000 + 7,
            },
        },
        cause: Recorded::Value(Default::default()),
        script: Some(EventFeedScript {
            change: Some(Change::Decision(DecisionChange {
                political_power: roster.then_some(5_000),
                ..change
            })),
            token_start: 0,
            token_len: 7,
            instance,
            days: instance.map(|_| 10),
            lifecycle: instance.map(|_| 0),
            ..Default::default()
        }),
    }
}

/// A roster day's changes from `first` on: candidates naming both other countries, the final
/// roster rebuild, a completion with no baseline when `fails`, then the clear of every candidate.
fn refill(first: u64, hour: i32, fails: bool) -> Vec<EventFeedEvent> {
    use replay_data::event_feed_rows::decisions::{
        ALL, CLEAR_ALL_CANDIDATES, DecisionChange, Operation,
    };
    let mut changes: Vec<_> = (0..4_u32)
        .map(|candidate| {
            let change = DecisionChange {
                ordinal: candidate,
                ..DecisionChange::new(Operation::Candidate, (candidate % 2) as u8)
            };
            decided(
                first + u64::from(candidate),
                hour,
                change,
                100_001 + candidate % 2,
                None,
            )
        })
        .collect();
    let rebuild = DecisionChange::new(Operation::Rebuild, 3);
    changes.push(decided(first + 4, hour, rebuild, 0, None));
    if fails {
        let ready = DecisionChange::new(Operation::Ready, 0);
        changes.push(decided(first + 5, hour, ready, 0, None));
    }
    let clear = DecisionChange {
        flags: ALL | CLEAR_ALL_CANDIDATES,
        ..DecisionChange::new(Operation::Candidate, 0)
    };
    changes.push(decided(first + 6, hour, clear, 0, None));
    changes
}

/// `frame`'s pack with `more` changes after its own, of `generation`.
fn extend_pack(frame: &mut DecodedFrame, more: Vec<EventFeedEvent>, generation: u64) {
    let mut held = match &*frame.tables[TableId::EventFeedHistory.index()] {
        TableView::Complete(TableRows::EventFeedHistory(rows)) => rows[0].events.clone(),
        _ => Vec::new(),
    };
    held.extend(more);
    for event in &mut held {
        event.generation = generation;
    }
    events(frame, held);
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *frame.tables[TableId::EventFeedHistory.index()]
    {
        rows[0].generation = generation;
    }
}

/// Reads as daily rosters make them: candidates filled and cleared within a read around its
/// final roster rebuild, their targets written again read after read, and one read failing
/// between its candidates and their clear; a decision part that changes and changes back; a
/// new generation that leaves every part, which a later baseline makes again; a read missing
/// from the recording, and one partly covered.
fn refill_inputs() -> Vec<DecodedFrame> {
    use replay_data::event_feed_rows::decisions::{DecisionChange, IGNORED, Operation, TIMED};
    let mut inputs = retained_inputs();
    inputs.extend([109, 110, 112, 113].map(|hour| quiet(hour, 3)));
    let ignore = |sequence, hour, flags| {
        let change = DecisionChange {
            flags,
            ..DecisionChange::new(Operation::Ignore, 0)
        };
        decided(sequence, hour, change, 0, None)
    };
    let census = vec![
        decided(
            1040,
            110,
            DecisionChange::new(Operation::Baseline, TIMED),
            0,
            None,
        ),
        decided(
            1041,
            110,
            DecisionChange::new(Operation::Store, TIMED),
            100_001,
            Some(17),
        ),
        decided(1042, 110, DecisionChange::new(Operation::Ready, 0), 0, None),
    ];
    let reads = [
        (3, refill(350, 103, false), 9),
        (
            4,
            [refill(450, 104, false), vec![ignore(470, 104, IGNORED)]].concat(),
            9,
        ),
        (5, vec![ignore(570, 105, 0)], 9),
        (7, refill(750, 107, true), 9),
        (8, refill(850, 108, false), 9),
        (9, Vec::new(), 10),
        (10, [census, refill(1050, 110, false)].concat(), 10),
        (11, refill(1150, 112, false), 10),
        (12, refill(1250, 113, false), 10),
    ];
    for (index, more, generation) in reads {
        extend_pack(&mut inputs[index], more, generation);
    }
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *inputs[12].tables[TableId::EventFeedHistory.index()]
    {
        rows[0].coverage = EventFeedCoverage::Partial;
    }
    inputs
}

#[test]
fn roster_days_parts_changing_back_and_returning_keep_every_byte_at_every_restart() {
    let inputs = refill_inputs();
    parity(&inputs, &[], None);
    for start in 1..inputs.len() {
        parity(&inputs, &[start], None);
    }
    let every: Vec<_> = (1..inputs.len()).collect();
    parity(&inputs, &every, None);
    parity_with_resume(&inputs, &[3, 7, 10], None, true);
    parity_with_resume(&inputs, &every, None, true);
    parity(&inputs, &[2, 8], Some((107, TableId::Countries)));
}

/// More roster rebuilds than a writer takes at a time, flushed rarely: their facts are written
/// by several writers and appended in order, across restarts and resumed runs.
#[test]
fn roster_facts_written_by_several_writers_keep_their_order() {
    use replay_data::event_feed_rows::decisions::{DecisionChange, Operation, TIMED};
    let mut inputs: Vec<_> = (100..=699).map(|hour| quiet(hour, 3)).collect();
    let census = vec![
        decided(
            1,
            100,
            DecisionChange::new(Operation::Baseline, TIMED),
            0,
            None,
        ),
        decided(
            2,
            100,
            DecisionChange::new(Operation::Store, TIMED),
            100_001,
            Some(17),
        ),
        decided(3, 100, DecisionChange::new(Operation::Ready, 0), 0, None),
    ];
    extend_pack(&mut inputs[0], census, 9);
    for index in 1..inputs.len() {
        let hour = inputs[index].hour;
        let fails = index % 97 == 0;
        extend_pack(
            &mut inputs[index],
            refill(10 * index as u64, hour, fails),
            9,
        );
    }
    for restarts in [vec![], vec![1, 300, 599]] {
        parity_with(&inputs, &restarts, None, false, 450);
    }
    parity_with(&inputs, &[120, 450], None, true, 450);
}

#[test]
fn existence_facts_keep_json_escaping_and_read_dates_after_quiet_spans() {
    let mut inputs: Vec<_> = (100..=109).map(|hour| quiet(hour, 3)).collect();
    for input in &mut inputs {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].tag = "mod / \"quoted\" \\ café [x]".as_bytes().to_vec();
        }
        if input.hour >= 108
            && let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
                &mut *input.tables[TableId::CountryCounterInputs.index()]
        {
            rows[0].owned_state_count = 0;
        }
    }
    parity(&inputs, &[3, 7, 9], None);
    parity_with_resume(&inputs, &[3, 6, 9], None, true);
}

#[test]
fn private_identity_quiet_steps_ignore_counter_values_but_preserve_key_proofs_and_late_names() {
    let mut inputs: Vec<_> = (100..=111).map(|hour| quiet(hour, 3)).collect();
    for input in &mut inputs {
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *input.tables[TableId::CountryCounterInputs.index()]
        {
            rows[0].owned_state_count = input.hour.rem_euclid(4) - 1;
            if input.hour == 104 {
                rows.push(CountryCounterInputRow {
                    country: 999_999,
                    ..Default::default()
                });
            }
        }
        if input.hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed after the occurrence".to_vec();
        }
        if input.hour >= 107
            && let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
                &mut *input.tables[TableId::CountryExperienceGains.index()]
        {
            rows[0].instance += 1;
        }
    }
    let mut late = script(1, EventFeedSource::Declaration, 102, false);
    late.script = None;
    events(&mut inputs[8], vec![late]);
    for restarts in [vec![], vec![2, 5, 8], vec![4, 6, 7, 9]] {
        parity(&inputs, &restarts, None);
    }
    parity_with_resume(&inputs, &[3, 6, 9], None, true);
}

#[test]
fn sparse_existence_reads_freeze_removed_rows() {
    let mut inputs: Vec<_> = (100..=111).map(|hour| quiet(hour, 3)).collect();
    for input in &mut inputs {
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *input.tables[TableId::CountryCounterInputs.index()]
        {
            if (102..=106).contains(&input.hour) {
                rows.retain(|row| row.country != 100_000);
            } else if input.hour >= 107 {
                rows[0].owned_state_count = 0;
            }
        }
    }
    parity(&inputs, &[2, 4, 7, 9], None);
    parity_with_resume(&inputs, &[3, 6, 9], None, true);
}

/// Guarantees read through their owners' slices: new, seen again, unresolved, exact and late,
/// started before the first hour observed and started later than their read, through
/// renames and an unread hour.
#[test]
fn guarantees_are_granted_once_at_their_first_read_as_the_row_path_grants_them() {
    let guarantee = |first: u32, second: u32, ordinal: u16, start_hour: i32| RelationTupleRow {
        owner_country: 100_000,
        target_country: second,
        type_id: engine_native::cguarantee_relation::TYPE,
        ordinal,
        first_country: first,
        second_country: second,
        start_hour,
        ..Default::default()
    };
    let mut inputs: Vec<_> = (100..=111).map(|hour| quiet(hour, 3)).collect();
    for input in &mut inputs {
        let hour = input.hour;
        let mut relations = vec![
            guarantee(100_000, 100_001, 0, 99),
            guarantee(100_000, 100_001, 1, 102),
            guarantee(100_000, 999_999, 2, 103),
            guarantee(100_000, 100_002, 3, 105),
            guarantee(100_000, 100_002, 4, 109),
        ];
        relations.sort_unstable_by_key(RowCodec::key);
        input.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(relations)).into();
        // The owner is read every other hour.
        let reads = (hour % 2 == 0)
            .then_some(SliceReadRow {
                table: TableId::RelationTuples.index() as u32,
                owner: 100_000,
                sample_hour: hour,
            })
            .into_iter()
            .collect();
        input.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(reads)).into();
        if hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[2].tag = b"mod / renamed guarantor".to_vec();
        }
    }
    for restarts in [
        vec![],
        vec![1],
        vec![2],
        vec![3, 6],
        vec![4, 8, 10],
        vec![9],
    ] {
        parity(&inputs, &restarts, None);
        parity(&inputs, &restarts, Some((104, TableId::Countries)));
    }
    parity_with_resume(&inputs, &[3, 6, 9], None, true);
}
