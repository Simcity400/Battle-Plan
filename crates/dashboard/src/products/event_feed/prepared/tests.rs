use super::*;
use crate::prepared::frame_changes::FrameChangeTracker;
use crate::{
    prepared::series::{LateIndex, Reader, tests::Fixture},
    products::country_history::HistoryScan,
};
use replay_data::{
    event_feed_rows::EventFeedCountry, event_feed_rows::EventFeedCounts as NativeCounts,
    event_feed_rows::EventFeedCoverage, event_feed_rows::EventFeedEvent,
    event_feed_rows::EventFeedHistoryRow, event_feed_rows::EventFeedSource, rows::*,
};
use replay_frame::frame::TableView;

#[derive(Serialize, Deserialize)]
struct Sources {
    #[serde(skip)]
    groups: country_groups::Scan,
    group_changes: country_groups::changes::Prepared,
    subjects: subjects::Scan,
    subject_changes: Accumulator,
    subject_late: LateIndex,
    ideas: idea_history::Scan,
    idea_changes: idea_history::PreparedChanges,
    focus: crate::prepared_focus::FocusIndex,
    identities: HistoryScan,
    facts: Observer,
    nukes: country_nukes::prepared::Prepared,
    projects: special_projects::prepared::Prepared,
    #[serde(skip, default = "FrameChangeTracker::new")]
    changes: FrameChangeTracker,
}
impl Sources {
    fn new(source: &Source) -> Self {
        Self {
            groups: country_groups::Scan::new(source.replay_ref(), 1),
            group_changes: Default::default(),
            subjects: subjects::Scan::new(source.replay_ref(), 1),
            subject_changes: Default::default(),
            subject_late: Default::default(),
            ideas: idea_history::Scan::new(source.replay_ref(), 1),
            idea_changes: Default::default(),
            focus: Default::default(),
            identities: HistoryScan::new(false, &[]),
            facts: Default::default(),
            nukes: Default::default(),
            projects: special_projects::prepared::Prepared::new(source.replay_ref(), 1),
            changes: FrameChangeTracker::new(),
        }
    }
    fn observe(
        &mut self,
        frame: &DecodedFrame,
        source: &Source,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.observe_in(frame, source, &LoadContext { content: None }, cancelled)
    }
    fn observe_in(
        &mut self,
        frame: &DecodedFrame,
        source: &Source,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.identities.observe(frame);
        self.group_changes
            .observe(&mut self.groups, frame, source, context)?;
        self.subjects
            .prepare_changes(frame, &mut self.subject_changes, &mut self.subject_late)?;
        self.ideas
            .prepare_changes(frame, &mut self.idea_changes, source, context)?;
        self.focus
            .observe_frame(source.replay_ref(), frame, cancelled)?;
        let changes = self.changes.observe(frame, source.replay_ref().format);
        self.nukes.observe(frame, source.replay_ref(), &changes)?;
        self.projects.observe(source, context, frame, cancelled)?;
        self.facts
            .observe(source.replay_ref(), frame, &self.identities, cancelled)
    }
    fn publish(
        &mut self,
        fixture: &mut Fixture,
        source: &Source,
        append: bool,
        cancelled: &AtomicBool,
    ) {
        self.publish_with_projects(fixture, source, append, cancelled, true);
    }
    // Hand-authored completion-stream fixtures are the sole publisher of that
    // section; an empty observer batch over the same hours would overlap it.
    fn publish_with_projects(
        &mut self,
        fixture: &mut Fixture,
        source: &Source,
        append: bool,
        cancelled: &AtomicBool,
        projects: bool,
    ) {
        let mut writer = if append {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        self.groups.finish(source.replay_ref(), 1);
        self.group_changes
            .flush(&mut writer, 1, &self.groups, cancelled)
            .unwrap();
        self.subject_changes
            .flush(&mut writer, "subject-changes-1", 0, cancelled)
            .unwrap();
        crate::prepared::series::write_catalogue(
            &mut writer,
            "subject-changes",
            1,
            &BTreeMap::new(),
            &(
                self.subjects.observed,
                self.subjects.change_warnings(),
                &self.subject_late,
            ),
        )
        .unwrap();
        self.ideas
            .flush_changes(&mut writer, 1, &mut self.idea_changes, cancelled)
            .unwrap();
        self.focus.flush(&mut writer, cancelled).unwrap();
        self.facts.flush(&mut writer, 1, cancelled).unwrap();
        self.nukes.flush(&mut writer, 1, cancelled).unwrap();
        if projects {
            self.projects.flush(&mut writer, 1, cancelled).unwrap();
        }
        writer
            .write_at("groups", 0, &self.groups.saved_bytes().unwrap()[..])
            .unwrap();
        writer
            .write_at("resume", 0, &serde_json::to_vec(self).unwrap()[..])
            .unwrap();
        writer.commit().unwrap();
        *self = serde_json::from_reader(std::io::BufReader::new(
            fixture
                .store
                .read(&fixture.key, "resume", 0)
                .unwrap()
                .unwrap(),
        ))
        .unwrap();
        self.groups = country_groups::Scan::read(
            fixture
                .store
                .read(&fixture.key, "groups", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    }
}

fn actor(index: u32) -> EventFeedCountry {
    EventFeedCountry {
        reference: 100_000 + index,
        id: 100_000 + index,
        instance: 7 + index,
    }
}
pub(crate) fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = super::super::tests::frame(hour, count);
    for rows in [
        TableRows::CountryExperienceGains(
            (0..count)
                .map(|index| CountryExperienceGainRow {
                    country: actor(index).id,
                    instance: actor(index).instance,
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::Factions(if (101..=102).contains(&hour) {
            vec![FactionRow {
                id: 42,
                name: if hour == 101 {
                    b"Recorded pact".to_vec()
                } else {
                    b"Renamed recorded pact".to_vec()
                },
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
        TableRows::FactionMembers(if (101..=102).contains(&hour) {
            (0..count)
                .map(|index| FactionMemberRow {
                    faction: 42,
                    country: actor(index).reference,
                    ordinal: index,
                })
                .collect()
        } else {
            Vec::new()
        }),
        TableRows::WarRelations(if hour == 102 {
            vec![WarRelationRow {
                first_country: actor(0).reference,
                second_country: actor(1).reference,
                start_hour: 101,
                first_was_instigator: 1,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
        TableRows::Subjects(if hour >= 101 {
            vec![SubjectRow {
                overlord: actor(0).reference,
                subject: actor(1).reference,
                start_hour: 101,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
        TableRows::HiredAdvisors(if hour >= 101 {
            vec![HiredAdvisorRow {
                country: actor(0).reference,
                idea: b"recorded_advisor".to_vec(),
                slot: b"recorded_seat".to_vec(),
                character: 77,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
        TableRows::CountryCounterInputs(
            (0..count)
                .map(|index| CountryCounterInputRow {
                    country: actor(index).id,
                    owned_state_count: if hour < 105 { 1 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::Focus(
            (0..count)
                .map(|index| FocusRow {
                    country: actor(index).id,
                    completed_count: i32::from(hour >= 101) + i32::from(hour >= 105),
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::CompletedFocuses(
            (0..count)
                .flat_map(|index| {
                    (0..(u32::from(hour >= 101) + u32::from(hour >= 105))).map(move |ordinal| {
                        CompletedFocusRow {
                            country: actor(index).id,
                            ordinal,
                            token: format!("recorded_focus_{ordinal}").into_bytes(),
                        }
                    })
                })
                .collect(),
        ),
        TableRows::NuclearStrikes(if hour == 104 {
            vec![NuclearStrikeRow {
                id: 77,
                hour: 101,
                launcher: actor(0),
                owner: actor(1),
                province: 13,
                state: 42,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    ] {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
        frame.metrics.table_ns[table.index()] = 1;
    }
    let events = if hour == 102 {
        vec![EventFeedEvent {
            script: None,
            sequence: 1,
            generation: 9,
            hour: 101,
            source: EventFeedSource::Declaration,
            actor: actor(0),
            target: actor(1),
            // The current writer records the cause packet even when no route
            // or optional operand was observed.
            cause: Recorded::Value(replay_data::event_feed_rows::EventFeedCause::default()),
        }]
    } else {
        Vec::new()
    };
    let acquired = events.len() as u64;
    *frame.tables[TableId::EventFeedHistory.index()] =
        TableView::Complete(TableRows::EventFeedHistory(vec![EventFeedHistoryRow {
            script_refusals: 0,
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
            generation: 9,
            read_hour: hour,
            coverage: EventFeedCoverage::Complete,
            counts: Some(NativeCounts {
                acquired,
                settled: acquired,
                ..Default::default()
            }),
            outstanding: 0,
            failure: None,
            events,
        }]));
    frame.metrics.table_ns[TableId::EventFeedHistory.index()] = 1;
    frame.metrics.table_ns[TableId::Countries.index()] = 1;
    frame.metrics.table_ns[TableId::CountryLinks.index()] = 1;
    frame
}

// Keep every other source at its initial state when a test supplies only
// native facts or strikes. The general frame fixture also creates group,
// subject, advisor, focus and existence changes after hour 100.
fn quiet_frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = frame(100, count);
    frame.hour = hour;
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *frame.tables[TableId::EventFeedHistory.index()]
    {
        rows[0].read_hour = hour;
    }
    frame
}

#[test]
fn prepared_feed_fixture_nested_causes_round_trip_through_current_recording_codec() {
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut decoder = replay_frame::frame::FrameDecoder::new();
    for hour in 100..=107 {
        let expected = frame(hour, 3);
        let wire = encoder.encode(expected.clone(), false).unwrap();
        let actual = decoder.advance(&wire).unwrap();
        assert_eq!(
            actual.complete::<tables::EventFeedHistory>().unwrap(),
            expected.complete::<tables::EventFeedHistory>().unwrap(),
        );
        if hour == 102 {
            let events = &actual.complete::<tables::EventFeedHistory>().unwrap()[0].events;
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].cause,
                Recorded::Value(replay_data::event_feed_rows::EventFeedCause::default())
            );
        }
    }
}

fn rows<T: serde::de::DeserializeOwned>(
    fixture: &Fixture,
    section: &str,
    keys: &[u64],
    cancelled: &AtomicBool,
) -> Vec<T> {
    Reader::new(&fixture.store, &fixture.key, section.into(), cancelled)
        .unwrap()
        .events(keys, i32::MIN, i32::MAX, 0, u32::MAX)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| serde_json::from_value(row.row).unwrap())
        .collect()
}

#[test]
fn prepared_feed_sources_equal_their_change_builders_across_commits_resume_append_and_gaps() {
    for countries in [3, 300] {
        let cancelled = AtomicBool::new(false);
        let mut fixture = Fixture::new();
        let mut sources = None;
        let mut seen = Vec::new();
        for (batch, hours) in [vec![100, 101], vec![102], vec![104, 105]]
            .into_iter()
            .enumerate()
        {
            let new: Vec<_> = hours
                .into_iter()
                .map(|hour| frame(hour, countries))
                .collect();
            seen.extend(new.iter().cloned());
            let source = super::super::tests::opened(seen.clone());
            let state = sources.get_or_insert_with(|| Sources::new(&source));
            state.groups.begin_block();
            for frame in &new {
                state.observe(frame, &source, &cancelled).unwrap();
            }
            state.publish(&mut fixture, &source, batch == 2, &cancelled);
            let context = LoadContext { content: None };
            let feed = build(&source, 1, &context, &cancelled).unwrap();
            let mut expected = semantic(&reply(&feed, "1", feed.first, feed.last, true));
            expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
            assert_eq!(
                pages(
                    &source,
                    &fixture.store,
                    &fixture.key,
                    &context,
                    feed.last,
                    257,
                    expected.len().div_ceil(257) + 1,
                ),
                expected
            );
            let groups = country_groups::cached_scan(&source, 1, &cancelled).unwrap();
            let oracle = diplomacy::scan(source.replay_ref(), 1, &groups, &cancelled).unwrap();
            let mut changes: Vec<country_groups::changes::Change> = rows(
                &fixture,
                "country-group-changes-1",
                &country_groups::changes::KEYS,
                &cancelled,
            );
            for change in &mut changes {
                change.name = None;
            }
            let mut wanted: Vec<_> = oracle
                .moments
                .into_iter()
                .filter(|change| {
                    change.kind != "ceasedToExist" && change.kind != "guaranteeGranted"
                })
                .collect();
            let order = |change: &country_groups::changes::Change| {
                (
                    change.hour,
                    change.read_hour,
                    change.kind.clone(),
                    change.tags.clone(),
                    change.faction,
                )
            };
            changes.sort_by_key(&order);
            wanted.sort_by_key(order);
            assert_eq!(changes, wanted);
            let starts: Vec<subjects::SubjectStart> =
                rows(&fixture, "subject-changes-1", &[0, 1 << 63], &cancelled);
            assert_eq!(
                serde_json::to_value(starts).unwrap(),
                serde_json::to_value(
                    &subjects::scanned_branch(&source, 1, &cancelled)
                        .unwrap()
                        .starts
                )
                .unwrap()
            );
            let additions: Vec<idea_history::Addition> =
                rows(&fixture, "idea-changes-1", &[0, 1, 2, 3], &cancelled);
            let ideas = idea_history::scanned_branch(&source, 1, &cancelled).unwrap();
            let actual: Vec<_> = additions
                .iter()
                .map(|event| (&event.tag, &event.span.token, event.span.first_hour))
                .collect();
            let wanted: Vec<_> = ideas
                .countries
                .iter()
                .flat_map(|(tag, scanned)| {
                    idea_history::named_additions(scanned)
                        .map(move |(span, _)| (tag, &span.token, span.first_hour))
                })
                .collect();
            assert_eq!(actual, wanted);
            // Published across commits and resumes, the completions are those
            // one uninterrupted fold of every hour gives.
            let completions: Vec<crate::products::national_focus::FocusCompletion> =
                rows(&fixture, "focus-completions-1", &[0], &cancelled);
            let mut original = crate::prepared_focus::FocusIndex::default();
            for frame in &seen {
                original
                    .observe_frame(source.replay_ref(), frame, &cancelled)
                    .unwrap();
            }
            original.fold_held(&cancelled).unwrap();
            // The debug trace follows country folds; published calls follow
            // their stable chronological order, including the same-hour ties.
            original.seen.sort_by_key(|completion| completion.hour);
            assert_eq!(completions, original.seen);
            let facts: Vec<Fact> = rows(&fixture, "event-feed-facts-1", &FACT_KEYS, &cancelled);
            assert_eq!(
                facts
                    .iter()
                    .filter(|fact| fact.row.kind == EventFeedKind::WarDeclared)
                    .count(),
                usize::from(batch > 0)
            );
            assert_eq!(
                facts
                    .iter()
                    .filter(|fact| fact.row.kind == EventFeedKind::CeasedToExist)
                    .count(),
                if batch == 2 { countries as usize } else { 0 }
            );
            assert_eq!(
                fixture
                    .store
                    .range_count(&fixture.key, "event-feed-facts-1")
                    .unwrap(),
                Some(batch as u64 + 1)
            );
        }
        let state = sources.unwrap();
        assert!(
            state
                .facts
                .late
                .between(101, 101, 102)
                .any(|row| row.1 == 102)
        );
        assert_eq!(state.facts.late.between(101, 101, 101).count(), 0);
    }
}

fn semantic(reply: &EventFeedReply) -> Vec<serde_json::Value> {
    reply
        .rows
        .iter()
        .map(|row| {
            let mut value = serde_json::to_value(row).unwrap();
            value.as_object_mut().unwrap().remove("id");
            value["item"] = row.item.map_or(serde_json::Value::Null, |item| {
                serde_json::to_value(&reply.items[item as usize]).unwrap()
            });
            value
        })
        .collect()
}

fn pages(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
    live: i32,
    size: u32,
    request_bound: usize,
) -> Vec<serde_json::Value> {
    let mut cursor = None;
    let mut result = Vec::new();
    let mut requested = 0;
    let mut cursors = BTreeSet::new();
    loop {
        requested += 1;
        assert!(
            requested <= request_bound,
            "prepared feed exceeded {request_bound} page requests"
        );
        let page = query(
            source,
            query::Params {
                branch_id: "1".into(),
                first_hour: None,
                last_hour: live,
                cursor,
                page_size: size,
                categories: BTreeSet::from([
                    EventFeedCategory::WarsPeace,
                    EventFeedCategory::Diplomacy,
                    EventFeedCategory::Politics,
                    EventFeedCategory::Milestones,
                ]),
                observations: false,
            },
            context,
            store,
            key,
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        // Display reverses dates, preserving the reply's forward same-date order.
        let mut rows = semantic(&page.reply);
        rows.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
        result.extend(rows);
        let Some(next) = page.next_cursor else {
            break;
        };
        assert_ne!(next, "");
        assert!(
            cursors.insert(next.clone()),
            "prepared feed repeated a continuation"
        );
        cursor = Some(next);
    }
    result
}

pub(crate) fn assert_prepared_content(
    source: &Source,
    frames: &[DecodedFrame],
    context: &LoadContext,
) {
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = Sources::new(source);
    for (index, frame) in frames.iter().enumerate() {
        state.observe_in(frame, source, context, &flag).unwrap();
        state.publish(&mut fixture, source, index > 1, &flag);
    }
    let feed = build(source, 1, context, &flag).unwrap();
    let expected = reply(&feed, "1", feed.first, feed.last, true);
    let mut expected = semantic(&expected);
    expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
    assert_eq!(
        pages(
            source,
            &fixture.store,
            &fixture.key,
            context,
            feed.last,
            1,
            expected.len() + 1
        ),
        expected
    );
}

#[test]
fn prepared_feed_pages_reach_late_facts_past_their_read_and_keep_live_cutoffs() {
    let frames: Vec<_> = [100, 600, 720].map(|hour| quiet_frame(hour, 3)).into();
    let source = super::super::tests::opened(frames.clone());
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = Sources::new(&source);
    for frame in &frames {
        state.observe(frame, &source, &flag).unwrap();
    }
    let mut builder = Builder::new(100, 720);
    for native in [600, 700, 125, 125] {
        builder.row(
            EventFeedKind::WarDeclared,
            RowDate {
                hour: native,
                read_hour: if native == 125 { 720 } else { native },
                name_hour: native,
                exact: true,
            },
            vec!["synthetic actor".into()],
            None,
            None,
        );
    }
    for (index, mut row) in builder.feed.rows.into_iter().enumerate() {
        row.id = format!("native-{index}");
        let key = (row.kind as u64) | (u64::from(row.read_hour > row.hour) << 63);
        let ordinal = state
            .facts
            .events
            .state(key)
            .map_or(0, |state| state.events);
        state
            .facts
            .late
            .insert(row.hour, row.read_hour, key, ordinal);
        state
            .facts
            .events
            .event(
                key,
                row.read_hour,
                &Fact {
                    row,
                    nuke_type: None,
                    script: None,
                    occurrence: None,
                    item: None,
                },
            )
            .unwrap();
    }
    state.publish(&mut fixture, &source, false, &flag);
    let context = LoadContext { content: None };
    let all = pages(&source, &fixture.store, &fixture.key, &context, 720, 1, 4);
    assert_eq!(
        all.iter()
            .map(|row| row["hour"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [700, 600, 125, 125]
    );
    assert_eq!(
        pages(&source, &fixture.store, &fixture.key, &context, 719, 1, 2).len(),
        2
    );
    assert_eq!(
        pages(&source, &fixture.store, &fixture.key, &context, 720, 3, 2),
        all
    );
    flag.store(true, Ordering::Release);
    let error = query(
        &source,
        query::Params {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: 720,
            cursor: None,
            page_size: 1,
            categories: BTreeSet::new(),
            observations: false,
        },
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

pub(crate) fn assert_store_pages(source: &Source, store: &store::Store, key: &store::SourceKey) {
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let feed = build(source, 1, &context, &flag).unwrap();
    let mut expected = semantic(&reply(&feed, "1", feed.first, feed.last, true));
    expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
    assert_eq!(
        pages(
            source,
            store,
            key,
            &context,
            feed.last,
            7,
            expected.len().div_ceil(7) + 1
        ),
        expected
    );
}

#[test]
fn prepared_feed_same_date_completions_page_exactly_across_decimal_boundaries() {
    for count in [101, 1_001] {
        let mut frames: Vec<_> = [100, 101].map(|hour| frame(hour, 3)).into();
        for frame in &mut frames {
            *frame.tables[TableId::Focus.index()] =
                TableView::Complete(TableRows::Focus(vec![FocusRow {
                    country: actor(0).id,
                    completed_count: if frame.hour == 101 { count } else { 0 },
                    ..Default::default()
                }]));
            *frame.tables[TableId::CompletedFocuses.index()] =
                TableView::Complete(TableRows::CompletedFocuses(
                    (0..if frame.hour == 101 { count as u32 } else { 0 })
                        .map(|ordinal| CompletedFocusRow {
                            country: actor(0).id,
                            ordinal,
                            token: format!("mod_completion_{ordinal}").into_bytes(),
                        })
                        .collect(),
                ));
        }
        let source = super::super::tests::opened(frames.clone());
        let flag = AtomicBool::new(false);
        let context = LoadContext { content: None };
        let mut fixture = Fixture::new();
        let mut state = Sources::new(&source);
        for frame in &frames {
            state.observe(frame, &source, &flag).unwrap();
            state.publish(&mut fixture, &source, false, &flag);
        }
        let oracle = build(&source, 1, &context, &flag).unwrap();
        let mut expected = semantic(&reply(&oracle, "1", 100, 101, true));
        expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
        // Deliberately split the 99/100 tie across pages while retaining a strict
        // request bound; this uses the owning observer/reader.
        let size = 100;
        let actual = pages(
            &source,
            &fixture.store,
            &fixture.key,
            &context,
            101,
            size,
            expected.len().div_ceil(size as usize) + 1,
        );
        assert_eq!(actual, expected);
        let tokens: Vec<_> = actual
            .iter()
            .filter(|row| row["kind"] == "focusCompleted")
            .map(|row| row["item"]["token"].as_str().unwrap())
            .collect();
        assert_eq!(tokens.len(), count as usize);
        assert_eq!(
            &tokens[99..101],
            &["mod_completion_99", "mod_completion_100"]
        );
        assert_eq!(
            *tokens.last().unwrap(),
            format!("mod_completion_{}", count - 1)
        );
    }
}

#[test]
fn prepared_feed_nuclear_increases_match_oracle_after_commits_resume_append() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    for count in [3, 300] {
        let mut fixture = Fixture::new();
        let mut state = None;
        let mut seen = Vec::new();
        for (batch, hours) in [[100, 101], [102, 103], [104, 105]].into_iter().enumerate() {
            let frames: Vec<_> = hours
                .into_iter()
                .map(|hour| {
                    let mut frame = super::super::tests::frame(hour, count);
                    let amount = [0, 1, 1, 3, 2, 4][(hour - 100) as usize];
                    *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
                        (0..count)
                            .map(|index| NukeRow {
                                country: actor(index).id,
                                amount_raw: amount * 100_000_000,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                    frame.metrics.table_ns[TableId::Nukes.index()] = 1;
                    frame
                })
                .collect();
            seen.extend(frames.iter().cloned());
            let source = super::super::tests::opened(seen.clone());
            let state = state.get_or_insert_with(|| Sources::new(&source));
            state.groups.begin_block();
            for frame in &frames {
                state.observe(frame, &source, &flag).unwrap();
            }
            state.publish(&mut fixture, &source, batch == 2, &flag);
            let oracle = build(&source, 1, &context, &flag).unwrap();
            let mut expected = semantic(&reply(&oracle, "1", 100, hours[1], true));
            expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
            assert_eq!(expected.len(), count as usize * (batch + 1));
            assert!(
                expected
                    .iter()
                    .all(|row| row["kind"] == "nukesIncreased" && row["exact"] == false)
            );
            assert_eq!(
                pages(
                    &source,
                    &fixture.store,
                    &fixture.key,
                    &context,
                    hours[1],
                    257,
                    expected.len().div_ceil(257) + 1
                ),
                expected
            );
            let live = hours[0];
            let mut earlier = semantic(&reply(&oracle, "1", 100, live, true));
            earlier.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
            assert_eq!(
                pages(
                    &source,
                    &fixture.store,
                    &fixture.key,
                    &context,
                    live,
                    257,
                    earlier.len().div_ceil(257) + 1
                ),
                earlier,
                "the supplying read gates every ordinary increase"
            );
        }
    }
}

fn project_change(
    id: u64,
    native: i32,
    read: i32,
    tag: Option<&str>,
) -> special_projects::prepared::completion_stream::Change {
    let date = special_projects::ProjectDate {
        hour: native,
        exact: true,
    };
    special_projects::prepared::completion_stream::Change {
        id,
        identity: (1, 100_000),
        local_id: id as u32,
        definition_id: 8,
        program_reference: 0,
        date: date.clone(),
        observed_hour: read,
        completion: tag.map(|tag| special_projects::ProjectCompletion {
            tag: tag.into(),
            date,
            observed_hour: read,
            definition_id: 8,
            item_token: Some(format!("mod_project_{id}")),
            name: Some(format!("Recorded project {id}")),
            icon: None,
            icon_missing: None,
            icon_choice: None,
        }),
        warnings: Vec::new(),
        refusals: Default::default(),
    }
}

#[test]
fn prepared_feed_obsolete_project_pages_advance_to_live_rows_after_resume_and_append() {
    use crate::prepared::series::{self, Run};
    use special_projects::prepared::completion_stream as stream;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames: Vec<_> = [100, 110, 119, 120, 130]
        .map(|hour| super::super::tests::frame(hour, 3))
        .into();
    let source = super::super::tests::opened(frames.clone());
    let mut fixture = Fixture::new();
    let mut sources = Sources::new(&source);
    for frame in &frames {
        sources.observe(frame, &source, &flag).unwrap();
    }
    sources.publish_with_projects(&mut fixture, &source, false, &flag, false);

    // Topic operands, not a composed feed. Three newest bounded rows become
    // obsolete; an older live episode and a delayed replacement remain.
    let mut values = Accumulator::default();
    let mut metadata = stream::Metadata {
        available: true,
        ..Default::default()
    };
    values.through(100);
    values
        .event(
            stream::KEY,
            103,
            &project_change(3, 103, 103, Some("mod-old-live")),
        )
        .unwrap();
    values.push(
        stream::episode_key(3).unwrap(),
        Run::new(103, 119, 0, false),
    );
    for (id, tag) in [(0, "mod-A"), (1, "mod-B"), (2, "mod-C")] {
        let ordinal = values.state(stream::KEY).unwrap().events;
        values
            .event(stream::KEY, 110, &project_change(id, 110, 110, Some(tag)))
            .unwrap();
        values.push(
            stream::episode_key(id).unwrap(),
            Run::new(110, 119, i128::from(ordinal), false),
        );
    }
    values.through(119);
    let mut writer = fixture.writer();
    values
        .flush(&mut writer, &stream::section(1), 0, &flag)
        .unwrap();
    series::write_catalogue(&mut writer, stream::FAMILY, 1, &BTreeMap::new(), &metadata).unwrap();
    writer.commit().unwrap();
    values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    metadata = serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap();

    values.through(120);
    for change in [
        project_change(0, 110, 120, None),
        project_change(1, 100, 120, Some("mod-B")),
        project_change(2, 110, 120, None),
    ] {
        let ordinal = values.state(stream::KEY).unwrap().events;
        values.event(stream::KEY, 120, &change).unwrap();
        metadata
            .late
            .insert(change.date.hour, 120, stream::KEY, ordinal);
        if change.completion.is_some() {
            values.push(
                stream::episode_key(change.id).unwrap(),
                Run::new(120, 130, i128::from(ordinal), false),
            );
        }
    }
    values.push(
        stream::episode_key(3).unwrap(),
        Run::new(120, 130, 0, false),
    );
    values.through(130);
    let mut writer = fixture.append_writer();
    values
        .flush(&mut writer, &stream::section(1), 0, &flag)
        .unwrap();
    series::write_catalogue(&mut writer, stream::FAMILY, 1, &BTreeMap::new(), &metadata).unwrap();
    writer.commit().unwrap();

    crate::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let first = query(
        &source,
        query::Params {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: 130,
            cursor: None,
            page_size: 2,
            categories: BTreeSet::from([EventFeedCategory::Milestones]),
            observations: false,
        },
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert!(
        first.reply.rows.is_empty(),
        "Live removes both obsolete raw candidates"
    );
    assert!(
        first.next_cursor.is_some(),
        "the raw cursor still reaches the next live episode"
    );
    let current = pages(&source, &fixture.store, &fixture.key, &context, 130, 2, 4);
    assert_eq!(
        current
            .iter()
            .map(|row| (
                row["hour"].as_i64().unwrap(),
                row["item"]["token"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [(103, "mod_project_3"), (100, "mod_project_1")]
    );
    assert!(
        crate::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty),
        "composition and Live validation only read prepared ranges"
    );
    let earlier = pages(&source, &fixture.store, &fixture.key, &context, 119, 2, 2);
    assert_eq!(
        earlier
            .iter()
            .map(|row| row["item"]["token"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "mod_project_0",
            "mod_project_1",
            "mod_project_2",
            "mod_project_3"
        ]
    );
}

#[test]
fn prepared_feed_projects_seek_tag_order_with_late_rows_and_withdrawals_at_one_read() {
    use crate::prepared::series::{self, Run};
    use special_projects::prepared::completion_stream as stream;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames: Vec<_> = [100, 110]
        .map(|hour| super::super::tests::frame(hour, 3))
        .into();
    let source = super::super::tests::opened(frames.clone());
    let mut fixture = Fixture::new();
    let mut sources = Sources::new(&source);
    for frame in &frames {
        sources.observe(frame, &source, &flag).unwrap();
    }
    sources.publish_with_projects(&mut fixture, &source, false, &flag, false);
    let mut values = Accumulator::default();
    let mut metadata = stream::Metadata {
        available: true,
        ..Default::default()
    };
    values.through(100);
    for (ordinal, change) in [
        project_change(0, 103, 110, Some("mod-A")),
        project_change(1, 110, 110, Some("mod-B")),
        project_change(2, 110, 110, Some("mod-C")),
        project_change(3, 110, 110, None),
    ]
    .into_iter()
    .enumerate()
    {
        values.event(stream::KEY, 110, &change).unwrap();
        metadata
            .late
            .insert(change.date.hour, 110, stream::KEY, ordinal as u64);
        if change.completion.is_some() {
            values.push(
                stream::episode_key(change.id).unwrap(),
                Run::new(110, 110, ordinal as i128, false),
            );
        }
    }
    values.through(110);
    let mut writer = fixture.writer();
    values
        .flush(&mut writer, &stream::section(1), 0, &flag)
        .unwrap();
    series::write_catalogue(&mut writer, stream::FAMILY, 1, &BTreeMap::new(), &metadata).unwrap();
    writer.commit().unwrap();
    let reached = pages(&source, &fixture.store, &fixture.key, &context, 110, 1, 4);
    assert_eq!(
        reached
            .iter()
            .map(|row| row["item"]["token"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["mod_project_1", "mod_project_2", "mod_project_0"]
    );
}

#[test]
fn prepared_feed_project_revisions_match_oracle_after_commits_resume_append() {
    const COMPLETION_HISTORY_MARKER: u32 = 0x4c17;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    for count in [3, 300] {
        let mut fixture = Fixture::new();
        let mut state = None;
        let mut seen = Vec::new();
        for (batch, hours) in [[100, 101], [102, 103], [104, 105]].into_iter().enumerate() {
            let new: Vec<_> = hours
                .into_iter()
                .map(|hour| {
                    let mut frame = super::super::tests::frame(hour, count);
                    *frame.tables[TableId::Projects.index()] =
                        TableView::Complete(TableRows::Projects(
                            (0..count)
                                .map(|index| ProjectRow {
                                    owner: actor(index).id,
                                    local_id: index + 1,
                                    definition_id: 8,
                                    started: 1,
                                    state: ProjectState {
                                        current_offset: if hour >= 102 { 0xe8 } else { 0x18 },
                                        not_completed: u8::from(hour < 102),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })
                                .collect(),
                        ));
                    *frame.tables[TableId::LexicalTokens.index()] =
                        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                            token: 8,
                            literal: b"mod_project".to_vec(),
                        }]));
                    *frame.tables[TableId::ProjectHistory.index()] = if hour >= 104 {
                        TableView::Complete(TableRows::ProjectHistory(
                            (0..count)
                                .map(|index| ProjectHistoryRow {
                                    owner: actor(index).id,
                                    project: index + 1,
                                    ordinal: 0,
                                    prototype_reward_handle: COMPLETION_HISTORY_MARKER,
                                    option_handle: COMPLETION_HISTORY_MARKER,
                                    date: 101,
                                    ..Default::default()
                                })
                                .collect(),
                        ))
                    } else if hour >= 102 {
                        TableView::Unavailable {
                            reason: "native completion date not yet recorded".into(),
                        }
                    } else {
                        TableView::Complete(TableRows::ProjectHistory(Vec::new()))
                    };
                    frame.metrics.table_ns[TableId::Projects.index()] = 1;
                    frame
                })
                .collect();
            seen.extend(new.iter().cloned());
            let source = super::super::tests::opened(seen.clone());
            let state = state.get_or_insert_with(|| Sources::new(&source));
            for frame in &new {
                state.observe(frame, &source, &flag).unwrap();
            }
            state.publish(&mut fixture, &source, batch == 2, &flag);
            let oracle = build(&source, 1, &context, &flag).unwrap();
            let mut expected = semantic(&reply(&oracle, "1", 100, hours[1], true));
            expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
            assert_eq!(expected.len(), if batch == 0 { 0 } else { count as usize });
            assert!(expected.iter().all(|row| row["kind"] == "projectFinished"));
            let raw = if batch == 2 { count * 2 } else { count } as usize;
            crate::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
            assert_eq!(
                pages(
                    &source,
                    &fixture.store,
                    &fixture.key,
                    &context,
                    hours[1],
                    257,
                    raw.div_ceil(257) + 1
                ),
                expected
            );
            assert!(crate::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty));
            if batch == 2 {
                let prefix = super::super::tests::opened(seen[..4].to_vec());
                let oracle = build(&prefix, 1, &context, &flag).unwrap();
                let mut expected = semantic(&reply(&oracle, "1", 100, 103, true));
                expected.sort_by_key(|row| std::cmp::Reverse(row["hour"].as_i64().unwrap()));
                assert_eq!(
                    pages(
                        &source,
                        &fixture.store,
                        &fixture.key,
                        &context,
                        103,
                        257,
                        (count as usize).div_ceil(257) + 1
                    ),
                    expected,
                    "earlier Live keeps the original bounded completion"
                );
            }
        }
    }
}

#[test]
fn prepared_feed_strike_only_campaign_keeps_recorded_types_and_stockpile_refusals() {
    let mut first = quiet_frame(100, 3);
    let mut read = quiet_frame(101, 3);
    *read.tables[TableId::NuclearStrikes.index()] =
        TableView::Complete(TableRows::NuclearStrikes(vec![NuclearStrikeRow {
            id: 7,
            hour: 100,
            launcher: actor(0),
            owner: actor(1),
            nuke_type: 1,
            province: 13_000,
            state: 2,
            ..Default::default()
        }]));
    for frame in [&mut first, &mut read] {
        *frame.tables[TableId::Nukes.index()] = TableView::Unavailable {
            reason: "stockpile not recorded".into(),
        };
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: engine_native::raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN,
                literal: b"mod_thermonuclear_bomb".to_vec(),
            }]));
        frame.metrics.table_ns[TableId::Nukes.index()] = 1;
    }
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames = [first, read];
    let source = super::super::tests::opened(frames.to_vec());
    let mut fixture = Fixture::new();
    let mut state = Sources::new(&source);
    for frame in &frames {
        state.observe(frame, &source, &flag).unwrap();
        state.publish(&mut fixture, &source, false, &flag);
    }
    let oracle = build(&source, 1, &context, &flag).unwrap();
    let expected = semantic(&reply(&oracle, "1", 100, 101, true));
    let actual = pages(&source, &fixture.store, &fixture.key, &context, 101, 2, 4);
    assert_eq!(actual, expected);
    let strike = actual
        .iter()
        .find(|row| row["kind"] == "nuclearStrike")
        .unwrap();
    assert_eq!(strike["item"]["token"], "mod_thermonuclear_bomb");
    assert!(actual.iter().all(|row| row["kind"] != "nukesIncreased"));
    let page = query(
        &source,
        query::Params {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: 101,
            cursor: None,
            page_size: 256,
            categories: BTreeSet::from([EventFeedCategory::WarsPeace]),
            observations: false,
        },
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert!(
        page.reply
            .unavailable
            .iter()
            .any(|row| row.kind == "nukesIncreased"
                && row.reason == "No complete Nukes read was recorded.")
    );
    assert!(
        page.reply
            .unavailable
            .iter()
            .any(|row| row.kind == "nukesIncreased"
                && row.reason.contains("stockpile not recorded")
                && row.count == Some(2))
    );
}

#[test]
fn prepared_feed_ordinary_project_after_many_late_rows_reads_only_its_raw_page() {
    use crate::prepared::series::{self, Run};
    use special_projects::prepared::completion_stream as stream;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames: Vec<_> = [100, 8_000]
        .map(|hour| super::super::tests::frame(hour, 3))
        .into();
    let source = super::super::tests::opened(frames.clone());
    let mut fixture = Fixture::new();
    let mut sources = Sources::new(&source);
    for frame in &frames {
        sources.observe(frame, &source, &flag).unwrap();
    }
    sources.publish_with_projects(&mut fixture, &source, false, &flag, false);
    let mut values = Accumulator::default();
    let mut metadata = stream::Metadata {
        available: true,
        ..Default::default()
    };
    values.through(100);
    for ordinal in 0..50 {
        let change = project_change(ordinal, 1_000, 8_000, Some(&format!("mod-A-{ordinal:05}")));
        values.event(stream::KEY, 8_000, &change).unwrap();
        metadata.late.insert(1_000, 8_000, stream::KEY, ordinal);
    }
    values
        .event(
            stream::KEY,
            8_000,
            &project_change(50, 8_000, 8_000, Some("mod-Z")),
        )
        .unwrap();
    values.push(
        stream::episode_key(50).unwrap(),
        Run::new(8_000, 8_000, 50, false),
    );
    values.through(8_000);
    let mut writer = fixture.writer();
    values
        .flush(&mut writer, &stream::section(1), 0, &flag)
        .unwrap();
    series::write_catalogue(
        &mut writer,
        stream::FAMILY,
        1,
        &BTreeMap::from([("completions".into(), stream::KEY)]),
        &metadata,
    )
    .unwrap();
    writer.commit().unwrap();
    query::EVENT_ROWS_READ.with(|count| count.set(0));
    crate::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let page = query(
        &source,
        query::Params {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: 8_000,
            cursor: None,
            page_size: 1,
            categories: BTreeSet::from([EventFeedCategory::Milestones]),
            observations: false,
        },
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert_eq!(page.reply.rows.len(), 1);
    assert_eq!(page.reply.rows[0].tags, ["mod-Z"]);
    assert_eq!(page.reply.rows[0].hour, 8_000);
    assert!(
        page.next_cursor.is_some(),
        "older raw candidates remain reachable"
    );
    assert_eq!(
        query::EVENT_ROWS_READ.with(|count| count.get()),
        2,
        "one end lookup and one raw page; neither the late prefix nor its older lookahead tie is read"
    );
    assert!(crate::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty));
}

/// A quiet hour whose pack holds `events`, with their tokens, the power
/// mutations and the capture coverage of that hour.
fn script_frame(
    hour: i32,
    events: Vec<EventFeedEvent>,
    tokens: &[u8],
    power: Vec<CountryPowerChangeRow>,
) -> DecodedFrame {
    let mut frame = quiet_frame(hour, 3);
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *frame.tables[TableId::EventFeedHistory.index()]
    {
        let acquired = events.len() as u64;
        rows[0].events = events;
        rows[0].script_tokens = tokens.to_vec();
        rows[0].counts = Some(NativeCounts {
            acquired,
            settled: acquired,
            ..Default::default()
        });
    }
    let coverage = (0..3)
        .map(|index| PoliticsCaptureCoverageRow {
            country: actor(index).id,
            instance: actor(index).instance,
            since_hour: 0,
            through_hour: hour,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
            ..Default::default()
        })
        .collect();
    for rows in [
        TableRows::CountryPowerChanges(power),
        TableRows::PoliticsCaptureCoverage(coverage),
    ] {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

/// A scripted occurrence at `hour` whose token is `tokens[start..start + len]`.
#[allow(clippy::too_many_arguments)]
fn scripted(
    sequence: u64,
    source: EventFeedSource,
    hour: i32,
    (from, to): (u32, u32),
    event: Option<(u32, u32)>,
    instance: Option<u32>,
    token: (u32, u32),
) -> EventFeedEvent {
    EventFeedEvent {
        script: Some(replay_data::event_feed_rows::EventFeedScript {
            token_start: token.0,
            token_len: token.1,
            native_event: event.map(|(kind, id)| {
                replay_data::event_feed_rows::EventFeedEventIdentity { kind, id }
            }),
            instance,
            ..Default::default()
        }),
        sequence,
        generation: 9,
        hour,
        source,
        actor: actor(from),
        target: actor(to),
        cause: Recorded::Value(replay_data::event_feed_rows::EventFeedCause::default()),
    }
}

fn script_page(
    adapter: script::Adapter,
    source: &Source,
    fixture: &Fixture,
    live: i32,
    kinds: BTreeSet<EventFeedKind>,
) -> script::ScriptOccurrencePage {
    script::query(
        adapter,
        source,
        script::ScriptPageParams {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: live,
            cursor: None,
            page_size: 64,
            kinds,
        },
        &LoadContext { content: None },
        &fixture.store,
        &fixture.key,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap()
}

fn kinds_of(occurrence: &script::ScriptOccurrence) -> Vec<EventFeedKind> {
    occurrence.edges.iter().map(|edge| edge.row.kind).collect()
}

#[test]
fn event_choice_and_decision_pages_join_their_own_occurrences_and_prove_charges() {
    // "mod.evt" 0..7, "mod.news" 7..15, "mod_decision" 15..27.
    let tokens = b"mod.evtmod.newsmod_decision";
    let event = (Some((1, 9)), (0, 7));
    let charge = |hour: i32, after: i64| CountryPowerChangeRow {
        country: actor(0).id,
        instance: actor(0).instance,
        sequence: hour as u64,
        hour,
        currency: PowerCurrency::Political,
        balance_before_raw: 10_000_000,
        balance_after_raw: after,
        cause: PowerCause::Decision,
        identity: PowerIdentity {
            action: PowerAction::Execute,
            item_key: b"mod_decision".to_vec(),
            ..Default::default()
        },
        ..Default::default()
    };
    let frames = vec![
        script_frame(100, Vec::new(), b"", Vec::new()),
        script_frame(
            101,
            vec![
                // Sent by the first country to the second.
                scripted(1, EventFeedSource::ScriptEvent, 101, (0, 1), event.0, Some(7), event.1),
                // The same event again: its own occurrence.
                scripted(2, EventFeedSource::ScriptEvent, 101, (0, 1), event.0, Some(8), event.1),
                scripted(3, EventFeedSource::NewsEvent, 101, (0, 2), Some((1, 10)), Some(4), (7, 8)),
            ],
            tokens,
            Vec::new(),
        ),
        script_frame(
            102,
            vec![
                // The receiving country chooses; a choice of an unrecorded
                // dispatch stands alone.
                EventFeedEvent {
                    script: Some(replay_data::event_feed_rows::EventFeedScript {
                        option: Some(1),
                        ..scripted(4, EventFeedSource::EventOption, 102, (1, 1), event.0, Some(7), event.1)
                            .script
                            .unwrap()
                    }),
                    ..scripted(4, EventFeedSource::EventOption, 102, (1, 1), event.0, Some(7), event.1)
                },
                EventFeedEvent {
                    script: Some(replay_data::event_feed_rows::EventFeedScript {
                        option: Some(0),
                        ..scripted(5, EventFeedSource::EventOption, 102, (2, 2), Some((1, 77)), Some(99), event.1)
                            .script
                            .unwrap()
                    }),
                    ..scripted(5, EventFeedSource::EventOption, 102, (2, 2), Some((1, 77)), Some(99), event.1)
                },
            ],
            tokens,
            Vec::new(),
        ),
        script_frame(
            103,
            vec![scripted(6, EventFeedSource::DecisionSelected, 103, (0, 0), None, None, (15, 12))],
            tokens,
            vec![charge(103, 5_000_000)],
        ),
        script_frame(
            104,
            vec![
                scripted(7, EventFeedSource::DecisionCompleted, 104, (0, 0), None, None, (15, 12)),
                // Two selections of one decision in one read share a charge
                // key, so neither is attributed.
                scripted(8, EventFeedSource::DecisionSelected, 104, (0, 0), None, None, (15, 12)),
                scripted(9, EventFeedSource::DecisionSelected, 104, (0, 0), None, None, (15, 12)),
            ],
            tokens,
            vec![charge(104, 9_000_000)],
        ),
    ];
    let source = super::super::tests::opened(frames.clone());
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = Sources::new(&source);
    for frame in &frames {
        state.observe(frame, &source, &flag).unwrap();
    }
    state.publish(&mut fixture, &source, false, &flag);
    // The facts keep their items and the catalogue none; each edge is an entry.
    let (_, (diagnostics, _, _)) =
        crate::prepared::series::read_catalogue::<query::FactsCatalogue>(
            &fixture.store,
            &fixture.key,
            "event-feed-facts",
            1,
        )
        .unwrap()
        .unwrap();
    assert!(diagnostics.feed.items.is_empty() && diagnostics.item_keys.is_empty());
    let section = super::entries_section(1);
    let edges = entries::read(&fixture.store, &fixture.key, &section)
        .unwrap()
        .unwrap()
        .prefix(b"edge")
        .unwrap();
    assert!(!edges.is_empty());

    let page = script_page(script::Adapter::EventChoices, &source, &fixture, 104, BTreeSet::new());
    let cards = &page.reply.occurrences;
    assert_eq!(cards.len(), 4, "{cards:#?}");
    let chosen = cards
        .iter()
        .find(|card| card.edges[0].row.id == "native:9:1")
        .unwrap();
    assert!(chosen.opened);
    assert_eq!(
        kinds_of(chosen),
        [EventFeedKind::ScriptEventFired, EventFeedKind::EventOptionSelected]
    );
    assert_eq!(chosen.edges[1].option, Some(1));
    let repeated = cards
        .iter()
        .find(|card| card.edges[0].row.id == "native:9:2")
        .unwrap();
    assert_eq!(kinds_of(repeated), [EventFeedKind::ScriptEventFired]);
    let alone = cards
        .iter()
        .find(|card| card.edges[0].row.id == "native:9:5")
        .unwrap();
    assert!(!alone.opened);
    // Newest first.
    assert_eq!(cards[0].edges[0].row.hour, 102);

    // Before the choice's read the card holds its dispatch alone.
    let early = script_page(script::Adapter::EventChoices, &source, &fixture, 101, BTreeSet::new());
    assert!(early.reply.occurrences.iter().all(|card| card.edges.len() == 1));
    assert_eq!(early.reply.occurrences.len(), 3);
    // News alone.
    let news = script_page(
        script::Adapter::EventChoices,
        &source,
        &fixture,
        104,
        BTreeSet::from([EventFeedKind::NewsEventFired]),
    );
    assert_eq!(
        news.reply
            .occurrences
            .iter()
            .filter(|card| card.opened)
            .map(|card| card.edges[0].row.id.as_str())
            .collect::<Vec<_>>(),
        ["native:9:3"]
    );

    let page = script_page(script::Adapter::Decisions, &source, &fixture, 104, BTreeSet::new());
    let decisions = &page.reply.occurrences;
    let first = decisions
        .iter()
        .find(|card| card.edges[0].row.id == "native:9:6")
        .unwrap();
    assert_eq!(
        kinds_of(first),
        [EventFeedKind::DecisionSelected, EventFeedKind::DecisionCompleted]
    );
    assert_eq!(
        first.edges[0].costs,
        Some(vec![ScriptCost {
            currency: ScriptCurrency::PoliticalPower,
            amount: "-5000000".into(),
        }])
    );
    for id in ["native:9:8", "native:9:9"] {
        let ambiguous = decisions
            .iter()
            .find(|card| card.edges[0].row.id == id)
            .unwrap();
        assert_eq!(ambiguous.edges[0].costs, None);
    }
    // An opening kind of the other list is refused.
    assert!(
        script::query(
            script::Adapter::Decisions,
            &source,
            script::ScriptPageParams {
                branch_id: "1".into(),
                first_hour: None,
                last_hour: 104,
                cursor: None,
                page_size: 4,
                kinds: BTreeSet::from([EventFeedKind::NewsEventFired]),
            },
            &LoadContext { content: None },
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .is_err()
    );
}

#[test]
fn a_choice_published_while_its_list_is_read_joins_its_card() {
    use std::{cell::Cell, cell::RefCell, rc::Rc};
    /// The feed's facts and entries as a flush publishes them.
    fn publish(state: &mut Sources, fixture: &Fixture, cancelled: &AtomicBool) {
        let mut writer = fixture.writer();
        state.facts.flush(&mut writer, 1, cancelled).unwrap();
        writer.commit().unwrap();
    }
    let tokens = b"mod.evt";
    let event = (Some((1, 9)), (0, 7));
    let choice = scripted(2, EventFeedSource::EventOption, 102, (1, 1), event.0, Some(7), event.1);
    let frames = vec![
        script_frame(100, Vec::new(), b"", Vec::new()),
        script_frame(
            101,
            vec![scripted(1, EventFeedSource::ScriptEvent, 101, (0, 1), event.0, Some(7), event.1)],
            tokens,
            Vec::new(),
        ),
        script_frame(
            102,
            vec![EventFeedEvent {
                script: Some(replay_data::event_feed_rows::EventFeedScript {
                    option: Some(1),
                    ..choice.script.clone().unwrap()
                }),
                ..choice
            }],
            tokens,
            Vec::new(),
        ),
    ];
    let source = Rc::new(super::super::tests::opened(frames.clone()));
    let fixture = Rc::new(Fixture::new());
    let state = Rc::new(RefCell::new(Sources::new(&source)));
    let flag = AtomicBool::new(false);
    for frame in &frames[..2] {
        state.borrow_mut().observe(frame, &source, &flag).unwrap();
    }
    publish(&mut state.borrow_mut(), &fixture, &flag);
    // The choice is published once the list has read its page of the
    // dispatch alone, and the list is read again over that publication.
    let overtook = Rc::new(Cell::new(false));
    {
        let (source, fixture, state, overtook) = (
            Rc::clone(&source),
            Rc::clone(&fixture),
            Rc::clone(&state),
            Rc::clone(&overtook),
        );
        let frame = frames[2].clone();
        entries::overtake_next_composition(move || {
            let flag = AtomicBool::new(false);
            let mut state = state.borrow_mut();
            state.observe(&frame, &source, &flag).unwrap();
            publish(&mut state, &fixture, &flag);
            overtook.set(true);
        });
    }
    let page = script_page(script::Adapter::EventChoices, &source, &fixture, 102, BTreeSet::new());
    assert!(overtook.get());
    let cards = &page.reply.occurrences;
    assert_eq!(cards.len(), 1, "{cards:#?}");
    assert_eq!(
        kinds_of(&cards[0]),
        [EventFeedKind::ScriptEventFired, EventFeedKind::EventOptionSelected]
    );
    assert_eq!(cards[0].edges[1].option, Some(1));
}

#[test]
fn a_decision_edge_after_an_unobserved_interval_does_not_join_the_earlier_selection() {
    let tokens = b"mod_decision";
    let decision =
        |sequence, kind, hour| scripted(sequence, kind, hour, (0, 0), None, None, (0, 12));
    let mut lost = script_frame(101, Vec::new(), b"", Vec::new());
    if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
        &mut *lost.tables[TableId::EventFeedHistory.index()]
    {
        rows[0].coverage = EventFeedCoverage::Partial;
    }
    let frames = vec![
        script_frame(
            100,
            vec![decision(1, EventFeedSource::DecisionSelected, 100)],
            tokens,
            Vec::new(),
        ),
        lost,
        script_frame(
            102,
            vec![decision(2, EventFeedSource::DecisionCompleted, 102)],
            tokens,
            Vec::new(),
        ),
    ];
    let source = super::super::tests::opened(frames.clone());
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = Sources::new(&source);
    for frame in &frames {
        state.observe(frame, &source, &flag).unwrap();
    }
    state.publish(&mut fixture, &source, false, &flag);
    let page = script_page(script::Adapter::Decisions, &source, &fixture, 102, BTreeSet::new());
    let selected = page
        .reply
        .occurrences
        .iter()
        .find(|card| card.edges[0].row.id == "native:9:1")
        .unwrap();
    assert_eq!(kinds_of(selected), [EventFeedKind::DecisionSelected]);
}

#[test]
fn decision_edges_of_one_incomplete_read_do_not_join_each_other() {
    let tokens = b"mod_decision";
    let decision =
        |sequence, kind, hour| scripted(sequence, kind, hour, (0, 0), None, None, (0, 12));
    for (coverage, joined) in [
        (EventFeedCoverage::Complete, true),
        (EventFeedCoverage::Partial, false),
    ] {
        let mut both = script_frame(
            102,
            vec![
                decision(1, EventFeedSource::DecisionSelected, 102),
                decision(2, EventFeedSource::DecisionCompleted, 102),
            ],
            tokens,
            Vec::new(),
        );
        if let TableView::Complete(TableRows::EventFeedHistory(rows)) =
            &mut *both.tables[TableId::EventFeedHistory.index()]
        {
            rows[0].coverage = coverage;
        }
        let frames = vec![
            script_frame(100, Vec::new(), b"", Vec::new()),
            script_frame(101, Vec::new(), b"", Vec::new()),
            both,
        ];
        let source = super::super::tests::opened(frames.clone());
        let flag = AtomicBool::new(false);
        let mut fixture = Fixture::new();
        let mut state = Sources::new(&source);
        for frame in &frames {
            state.observe(frame, &source, &flag).unwrap();
        }
        state.publish(&mut fixture, &source, false, &flag);
        let page = script_page(script::Adapter::Decisions, &source, &fixture, 102, BTreeSet::new());
        let selected = page
            .reply
            .occurrences
            .iter()
            .find(|card| card.edges[0].row.id == "native:9:1")
            .unwrap();
        let expected: &[EventFeedKind] = if joined {
            &[EventFeedKind::DecisionSelected, EventFeedKind::DecisionCompleted]
        } else {
            &[EventFeedKind::DecisionSelected]
        };
        assert_eq!(kinds_of(selected), expected, "{coverage:?}");
    }
}

#[test]
fn immediate_effects_join_their_dispatch_without_taking_its_choice() {
    let tokens = b"mod.evt";
    let event = (Some((1, 9)), (0, 7));
    let option = |sequence: u64, hour: i32, ordinal: i32| {
        let row = scripted(sequence, EventFeedSource::EventOption, hour, (1, 1), event.0, Some(7), event.1);
        EventFeedEvent {
            script: Some(replay_data::event_feed_rows::EventFeedScript {
                option: Some(ordinal),
                ..row.script.unwrap()
            }),
            ..row
        }
    };
    let frames = vec![
        script_frame(100, Vec::new(), b"", Vec::new()),
        script_frame(
            101,
            vec![
                scripted(1, EventFeedSource::ScriptEvent, 101, (0, 1), event.0, Some(7), event.1),
                option(2, 101, -1),
            ],
            tokens,
            Vec::new(),
        ),
        script_frame(102, vec![option(3, 102, 1)], tokens, Vec::new()),
    ];
    let source = super::super::tests::opened(frames.clone());
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = Sources::new(&source);
    for frame in &frames {
        state.observe(frame, &source, &flag).unwrap();
    }
    state.publish(&mut fixture, &source, false, &flag);

    let page = script_page(script::Adapter::EventChoices, &source, &fixture, 102, BTreeSet::new());
    let cards = &page.reply.occurrences;
    assert_eq!(cards.len(), 1, "{cards:#?}");
    assert_eq!(
        kinds_of(&cards[0]),
        [
            EventFeedKind::ScriptEventFired,
            EventFeedKind::EventOptionSelected,
            EventFeedKind::EventOptionSelected
        ]
    );
    assert_eq!(
        cards[0].edges.iter().map(|edge| edge.option).collect::<Vec<_>>(),
        [None, Some(-1), Some(1)]
    );
}

#[test]
fn occurrence_pages_follow_dates_across_pages_with_late_facts_bound_to_the_live_read() {
    let tokens = b"mod.evt";
    let event = (Some((1, 9)), (0, 7));
    let fired = |sequence: u64, native: i32, instance: u32| {
        scripted(sequence, EventFeedSource::ScriptEvent, native, (0, 1), event.0, Some(instance), event.1)
    };
    let frames = vec![
        script_frame(100, Vec::new(), b"", Vec::new()),
        script_frame(101, vec![fired(1, 101, 1)], tokens, Vec::new()),
        // Read at 102: one dated 102, and one dated 100 that arrived late.
        script_frame(102, vec![fired(2, 102, 2), fired(3, 100, 3)], tokens, Vec::new()),
    ];
    let source = super::super::tests::opened(frames.clone());
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = Sources::new(&source);
    for frame in &frames {
        state.observe(frame, &source, &flag).unwrap();
    }
    state.publish(&mut fixture, &source, false, &flag);
    let page = |live: i32, cursor: Option<String>| {
        script::query(
            script::Adapter::EventChoices,
            &source,
            script::ScriptPageParams {
                branch_id: "1".into(),
                first_hour: None,
                last_hour: live,
                cursor,
                page_size: 1,
                kinds: BTreeSet::new(),
            },
            &LoadContext { content: None },
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap()
        .unwrap()
    };
    // One card per page, newest date first, the late one last.
    let mut hours = Vec::new();
    let mut cursor = None;
    loop {
        let next = page(102, cursor);
        hours.extend(next.reply.occurrences.iter().map(|card| card.edges[0].row.hour));
        match next.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(hours, [102, 101, 100]);
    // Before its read, the late fact is not shown at all.
    let early = page(101, None);
    assert_eq!(early.reply.occurrences.len(), 1);
    assert_eq!(early.reply.occurrences[0].edges[0].row.hour, 101);
}

#[test]
fn mutable_surrender_targets_keep_event_and_decision_edges_and_their_exact_charges() {
    let mut links = Links::default();
    let mut opening = Occurrence {
        generation: 1,
        event: Some((1, 9)),
        instance: Some(7),
        target: (24, 0),
        ..Default::default()
    };
    links.link(EventFeedKind::ScriptEventFired, &mut opening, (10, 0));
    let mut choice = Occurrence {
        actor: (24, 1),
        option: Some(0),
        ..opening.clone()
    };
    links.link(EventFeedKind::EventOptionSelected, &mut choice, (10, 1));
    assert_eq!(choice.opener, Some((10, 0)));
    let mut selection = Occurrence {
        actor: (24, 0),
        token: Some("synthetic".into()),
        ..Default::default()
    };
    links.observe_read(20, true);
    links.link(EventFeedKind::DecisionSelected, &mut selection, (11, 0));
    links.observe_read(21, true);
    let mut reused = Occurrence {
        actor: (1501, 0),
        ..selection.clone()
    };
    links.link(EventFeedKind::DecisionCompleted, &mut reused, (11, 1));
    assert_eq!(reused.opener, None);
    let mut completion = Occurrence {
        actor: (24, 1),
        ..selection.clone()
    };
    links.link(EventFeedKind::DecisionCompleted, &mut completion, (11, 2));
    assert_eq!(completion.opener, Some((11, 0)));
    let mut current = script_frame(
        21,
        Vec::new(),
        b"",
        vec![CountryPowerChangeRow {
            country: actor(0).id,
            instance: 99,
            hour: 21,
            currency: PowerCurrency::Political,
            balance_before_raw: 100,
            balance_after_raw: 93,
            cause: PowerCause::Decision,
            identity: PowerIdentity {
                action: PowerAction::Execute,
                item_key: b"synthetic".to_vec(),
                ..Default::default()
            },
            ..Default::default()
        }],
    );
    let key = (21, actor(0).id, "synthetic".to_owned(), false);
    assert_eq!(charges(&current)[&key][0].amount, "-7");
    if let TableView::Complete(TableRows::PoliticsCaptureCoverage(rows)) =
        &mut *current.tables[TableId::PoliticsCaptureCoverage.index()]
    {
        rows[0].country = 1501;
    }
    assert!(!charges(&current).contains_key(&key));
}

#[test]
fn retained_observations_read_back_as_made() {
    use super::super::native::{Projection, tests::retained_packs};
    let flag = AtomicBool::new(false);
    let frames: Vec<DecodedFrame> = retained_packs()
        .into_iter()
        .map(|pack| {
            let mut frame = quiet_frame(pack.read_hour, 3);
            *frame.tables[TableId::EventFeedHistory.index()] =
                TableView::Complete(TableRows::EventFeedHistory(vec![pack]));
            frame
        })
        .collect();
    // Each read's observation made whole, named as the feed names it.
    let mut names = HistoryScan::new(false, &[]);
    let mut whole = Projection::default();
    let mut builder = Builder::new(i32::MIN, i32::MAX);
    for (frame, pack) in frames.iter().zip(retained_packs()) {
        names.observe(frame);
        whole
            .append_named(
                &mut builder,
                &pack,
                |hour, country| names.native_tag(hour, country),
                &flag,
            )
            .unwrap();
    }
    let without_id = |row: &EventFeedRow| {
        let mut value = serde_json::to_value(row).unwrap();
        value.as_object_mut().unwrap().remove("id");
        value
    };
    let expected: Vec<_> = builder.feed.rows.iter().map(without_id).collect();
    assert_eq!(expected.len(), 2);
    let mut fixture = Fixture::new();
    let mut sources = None;
    let mut seen = Vec::new();
    for (batch, range) in [0..2, 2..3, 3..4].into_iter().enumerate() {
        seen.extend(frames[range.clone()].iter().cloned());
        let source = super::super::tests::opened(seen.clone());
        let state = sources.get_or_insert_with(|| Sources::new(&source));
        state.groups.begin_block();
        for frame in &frames[range] {
            state.observe(frame, &source, &flag).unwrap();
        }
        state.publish(&mut fixture, &source, batch == 2, &flag);
    }
    let source = super::super::tests::opened(seen);
    let page = query(
        &source,
        query::Params {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: 123,
            cursor: None,
            page_size: 100,
            categories: BTreeSet::from([
                EventFeedCategory::WarsPeace,
                EventFeedCategory::Diplomacy,
                EventFeedCategory::Politics,
                EventFeedCategory::Milestones,
            ]),
            observations: true,
        },
        &LoadContext { content: None },
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert!(page.next_cursor.is_none());
    let mut served: Vec<_> = page
        .reply
        .rows
        .iter()
        .filter(|row| row.kind == EventFeedKind::ScriptStateObserved)
        .collect();
    served.sort_by_key(|row| row.hour);
    assert_eq!(
        served.into_iter().map(without_id).collect::<Vec<_>>(),
        expected
    );
    let facts: Vec<Fact> = rows(
        &fixture,
        "event-feed-facts-1",
        &[EventFeedKind::ScriptStateObserved as u64],
        &flag,
    );
    assert_eq!(facts.len(), 2);
}
