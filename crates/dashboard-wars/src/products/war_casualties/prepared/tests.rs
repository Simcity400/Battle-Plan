use super::*;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_base::test_fixtures::{CAMPAIGN, records, scale_frame};
use replay_data::{rows::TableRows, rows::WarRelationRow};

impl Prepared {
    fn work(&self) -> dashboard_base::prepared::observer_tests::Work {
        use dashboard_base::prepared::observer_tests::Work;
        let row = self.builder.work;
        let pair = self.pairs.work;
        Work {
            row_visits: row.row_visits,
            history_visits: self.work.history_visits + pair.history_visits,
            emitted: self.work.emitted,
            ..Default::default()
        }
    }
}

#[test]
fn unchanged_casualty_frames_advance_without_holder_pair_or_value_sweeps() {
    use dashboard_base::prepared::observer_tests::{assert_flat, scaled_work, unchanged_frames};
    let measured = scaled_work(10, |hours| {
        let first = frame(10, 100);
        let source = source(std::slice::from_ref(&first));
        let replay = source.replay_ref();
        let flag = AtomicBool::new(false);
        let mut prepared = Prepared::new(replay, 1);
        prepared.observe(
            &first,
            replay,
            &source,
            &LoadContext { content: None },
            &flag,
        )?;
        unchanged_frames(
            prepared.work(),
            &first,
            hours as i32,
            replay.format,
            |frame, changes| {
                prepared.observe_changes(frame, replay, &flag, changes)?;
                Ok(prepared.work())
            },
        )
    })
    .unwrap();
    assert_flat(measured);
    assert_eq!(measured[1].row_visits, 0);
    assert_eq!(measured[1].history_visits, 0);
    assert_eq!(measured[1].emitted, 0);
}

#[test]
fn casualty_endpoint_changes_visit_only_its_retained_lifetimes_and_resume_rejoins_once() {
    use dashboard_base::prepared::observer_tests::{Work, assert_flat, scaled_work};
    let measured = scaled_work(10, |history| {
        let mut frames = Vec::new();
        for offset in 0..history as i32 {
            let mut next = frame(10 + offset, 4);
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *next.tables[TableId::WarRelations.index()]
            {
                rows[1].start_hour = next.hour;
            }
            frames.push(next);
        }
        let mut last = frames.last().unwrap().clone();
        last.hour += 1;
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *last.tables[TableId::WarRelations.index()]
        {
            rows.truncate(1);
        }
        frames.push(last.clone());
        let source = source(&frames);
        let replay = source.replay_ref();
        let context = LoadContext { content: None };
        let flag = AtomicBool::new(false);
        let mut prepared = Prepared::new(replay, 1);
        for frame in &frames {
            prepared.observe(frame, replay, &source, &context, &flag)?;
        }
        // Retain H unrelated ended lifetimes, then rebuild the runtime join once.
        prepared.builder.scan.seal();
        prepared.pairs.seal();
        let mut resumed: Prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
        let mut same = last.clone();
        same.hour += 1;
        prepared.observe(&same, replay, &source, &context, &flag)?;
        resumed.observe(&same, replay, &source, &context, &flag)?;
        prepared.pairs.seal();
        resumed.pairs.seal();
        let records = |prepared: &Prepared| {
            prepared
                .pairs
                .runs()
                .map(|(key, runs)| (key.clone(), runs.to_vec()))
                .collect::<Vec<_>>()
        };
        dashboard_base::prepared::observer_tests::assert_reply_parity(
            &records(&prepared),
            &records(&resumed),
        )?;
        let before = resumed.work();
        let mut rename = same.clone();
        rename.hour += 1;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *rename.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod renamed endpoint".to_vec();
        }
        resumed.observe(&rename, replay, &source, &context, &flag)?;
        let after = resumed.work();
        let work = Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            work.history_visits <= 5,
            "unrelated lifetime history was visited: {work:?}"
        );
        Ok(work)
    })
    .unwrap();
    assert_flat(measured);
}

#[test]
fn deferred_casualty_windows_and_country_replies_match_oracle_across_commits_resume_and_append()
-> io::Result<()> {
    use dashboard_base::prepared::observer_tests::assert_reply_parity;
    let first = frame(10, 4);
    let mut frames = vec![first.clone()];
    for hour in 11..=31 {
        let mut next = first.clone();
        next.hour = hour;
        if hour >= 19
            && let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *next.tables[TableId::WarRelations.index()]
        {
            rows.truncate(1);
        }
        if hour >= 25
            && let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *next.tables[TableId::WarRelations.index()]
        {
            rows[0].first_casualties += 7;
        }
        if hour >= 29
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod renamed endpoint".to_vec();
        }
        frames.push(next);
    }
    let source = source(&frames);
    let replay = source.replay_ref();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut prepared = Prepared::new(replay, 1);
    let mut oracle = ScanFrames::new(replay, 1);
    let mut oracle_pairs = pairs::PairHistory::default();
    let mut fixture = Fixture::new();
    let mut reference = Fixture::new();
    for (ordinal, chunk) in frames.chunks(8).enumerate() {
        for frame in chunk {
            prepared.observe(frame, replay, &source, &context, &flag)?;
            oracle.frame(frame, replay);
            oracle_pairs.observe(&oracle.scan);
        }
        let mut writer = if ordinal > 1 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        let mut oracle_writer = if ordinal > 1 {
            reference.append_writer()
        } else {
            reference.writer()
        };
        let span = HourSpan {
            first_hour: chunk[0].hour,
            last_hour: chunk.last().unwrap().hour,
        };
        prepared.finish_block(&mut writer, 1, span.first_hour, span.last_hour)?;
        oracle_pairs.publish_window(
            &mut oracle_writer,
            1,
            ordinal as u64,
            span,
            &oracle.scan.observed,
        )?;
        prepared.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        oracle_writer.commit()?;
        let read = |fixture: &Fixture| {
            series::read_reply::<Value>(
                &fixture.store,
                &fixture.key,
                "casualty-pairs-1",
                ordinal as u64,
            )
        };
        assert_reply_parity(&read(&fixture)?.unwrap(), &read(&reference)?.unwrap())?;
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
    }
    let oracle = oracle.scan.finish(&flag)?;
    for hour in 10..=31 {
        let actual = point(&fixture, hour, None, Vec::new());
        let rows = actual["series"].as_array().unwrap();
        let actual_tags: BTreeSet<_> = rows
            .iter()
            .map(|row| row["tag"].as_str().unwrap())
            .collect();
        let expected_tags: BTreeSet<_> = oracle
            .tracks
            .iter()
            .filter_map(|(tag, _)| oracle.country_at(tag, hour).map(|_| tag.as_str()))
            .collect();
        assert_eq!(actual_tags, expected_tags, "active tags at hour {hour}");
        for row in rows {
            let expected = oracle
                .country_at(row["tag"].as_str().unwrap(), hour)
                .unwrap();
            assert_eq!(row["dealt"][0], expected.dealt.to_string());
            assert_eq!(row["taken"][0], expected.taken.to_string());
            assert_eq!(
                row["unknown"][0],
                expected
                    .unknown
                    .map(|value| value.to_string())
                    .map_or(Value::Null, Value::String)
            );
        }
    }
    Ok(())
}

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (0..count / 2)
            .map(|pair| WarRelationRow {
                first_country: 100_000 + pair * 2,
                second_country: 100_001 + pair * 2,
                start_hour: 0,
                first_casualties: i64::from(hour) * 3 + i64::from(pair),
                second_casualties: -i64::from(hour) + i64::from(pair),
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

fn source(frames: &[DecodedFrame]) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "casualty-prepared-test".into(),
            revision: "1".into(),
        },
        "casualty-prepared-test.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn static_casualty_label_is_bound_once_for_success_empty_text_and_refusal() {
    use dashboard_base::prepared::observer_tests::{Work, unchanged_frames};
    for (authored, expected) in [
        (Some("mod casualty label"), "mod casualty label"),
        (Some(""), ""),
        (Some(UNKNOWN_LABEL), UNKNOWN_LABEL),
        (Some("[ROOT.GetName]"), "[ROOT.GetName]"),
        (None, "Unknown attacker"),
    ] {
        let first = frame(10, 2);
        let source = source(std::slice::from_ref(&first));
        let text = authored.map_or_else(
            || "\u{feff}l_english:\n".to_owned(),
            |label| format!("\u{feff}l_english:\n {UNKNOWN_LABEL}:0 \"{label}\"\n"),
        );
        let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
            "localisation/english/casualty_l_english.yml",
            &text,
        )]);
        let catalogue = Arc::new(catalogue);
        let options = dashboard_core::config::ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.clone(),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        dashboard_base::catalogue::set_test_catalogue(&source, &options, Arc::clone(&catalogue));
        let context = LoadContext {
            content: Some(options.clone()),
        };
        let content = prepared_content(&source, &context).unwrap();
        let labels = catalogue
            .content
            .layers
            .derived::<dashboard_text::catalogue::KeyedResults<(String, String), String>>();
        let label_key = (UNKNOWN_LABEL.to_owned(), "plain".to_owned());
        assert_eq!(labels.loads(&label_key), 1);
        let _ = prepared_content(&source, &context).unwrap();
        assert_eq!(
            labels.loads(&label_key),
            1,
            "refusals share the binding too"
        );
        let flag = AtomicBool::new(false);
        let replay = source.replay_ref();
        let mut prepared = Prepared::new(replay, 1);
        prepared.configure(&content);
        prepared
            .observe(&first, replay, &source, &context, &flag)
            .unwrap();
        assert_eq!(prepared.catalogue.unknown_label, expected);
        let loads = || Work {
            content_loads: labels.loads(&label_key)
                + catalogue.content_loads()
                + dashboard_base::catalogue::catalogue_loads(&source, &options),
            ..Default::default()
        };
        unchanged_frames(loads(), &first, 100, replay.format, |frame, changes| {
            prepared.observe_changes(frame, replay, &flag, changes)?;
            Ok(loads())
        })
        .unwrap();
        assert_eq!(prepared.catalogue.unknown_label, expected);
    }
}

#[test]
fn restored_casualties_publish_what_continuous_observation_publishes() {
    // A war ends at the first read after the first commit, and an endpoint is
    // renamed at the first read after the second. Country 000's naming
    // operands are refused with the hour they were read, so a restored scan
    // that named again at an unchanged hour would publish a refusal the
    // continued one never makes.
    let frames: Vec<_> = (10..22)
        .map(|hour| {
            let mut next = frame(hour, 6);
            if hour >= 14
                && let TableView::Complete(TableRows::WarRelations(rows)) =
                    &mut *next.tables[TableId::WarRelations.index()]
            {
                rows.truncate(2);
            }
            if hour >= 18
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *next.tables[TableId::Countries.index()]
            {
                rows[0].tag = b"mod renamed endpoint".to_vec();
            }
            next
        })
        .collect();
    let source = source(&frames);
    let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/english/casualty_l_english.yml",
        "\u{feff}l_english:\n",
    )]);
    let catalogue = Arc::new(catalogue);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    dashboard_base::catalogue::set_test_catalogue(&source, &options, Arc::clone(&catalogue));
    let content = prepared_content(
        &source,
        &LoadContext {
            content: Some(options),
        },
    )
    .unwrap();
    assert!(matches!(content, Ok(Some(_))));
    let replay = source.replay_ref();
    let flag = AtomicBool::new(false);
    let blocks = [&frames[..4], &frames[4..8], &frames[8..]];
    let mut commits = 0;
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        replay.format,
        &blocks,
        || Prepared::new(replay, 1),
        |observer, frame, changes, _| {
            observer.configure(&content);
            observer
                .observe_changes(frame, replay, &flag, changes)
                .unwrap();
        },
        |observer, writer, ordinal| {
            let block = blocks[ordinal];
            observer
                .finish_block(writer, 1, block[0].hour, block.last().unwrap().hour)
                .unwrap();
            observer.flush(writer, 1, &flag).unwrap();
            let saved = serde_json::to_vec(observer).unwrap();
            writer
                .write_at("casualty-continuation-1", 0, saved.as_slice())
                .unwrap();
        },
        |fixture| {
            // Before the next block another family refuses a text in the
            // shared content; no casualty naming input changes.
            catalogue.refuse(
                "Synthetic text refused by another family",
                &format!("another family's text {commits}"),
            );
            commits += 1;
            serde_json::from_reader(std::io::BufReader::new(
                fixture
                    .store
                    .read(&fixture.key, "casualty-continuation-1", 0)
                    .unwrap()
                    .unwrap(),
            ))
            .unwrap()
        },
    );
}

#[test]
fn catalogue_refusal_is_bound_before_casualty_observation() {
    use dashboard_base::prepared::observer_tests::{Work, unchanged_frames};
    let first = frame(10, 2);
    let source = source(std::slice::from_ref(&first));
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let context = LoadContext {
        content: Some(options.clone()),
    };
    let content = prepared_content(&source, &context).unwrap();
    assert!(content.is_err());
    let count = || Work {
        content_loads: dashboard_base::catalogue::catalogue_loads(&source, &options),
        ..Default::default()
    };
    assert_eq!(count().content_loads, 1);
    let flag = AtomicBool::new(false);
    let replay = source.replay_ref();
    let mut prepared = Prepared::new(replay, 1);
    prepared.configure(&content);
    prepared
        .observe(&first, replay, &source, &context, &flag)
        .unwrap();
    let warnings = prepared.catalogue.warnings.clone();
    assert!(warnings[0].starts_with("Casualty titles are unavailable:"));
    unchanged_frames(count(), &first, 100, replay.format, |frame, changes| {
        prepared.observe_changes(frame, replay, &flag, changes)?;
        Ok(count())
    })
    .unwrap();
    assert_eq!(prepared.catalogue.warnings, warnings);
}

fn point(fixture: &Fixture, hour: i32, top: Option<u32>, tags: Vec<String>) -> Value {
    query(
        "series-state",
        json!({"branchId":"1", "tags": tags,
        "family":{"kind":"war-casualties", "top":top.map(|count| json!({"count":count}))},
        "reads":[{"tag":"", "hour":hour}]}),
        &fixture.store,
        &fixture.key,
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn prepared_points_rankings_and_resume_match_the_casualty_scan() {
    let mut frames: Vec<_> = [10, 11, 12, 14, 15, 16]
        .into_iter()
        .map(|hour| frame(hour, 6))
        .collect();
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frames[3].tables[TableId::Countries.index()]
    {
        countries[0].tag = b"mod renamed country".to_vec();
    }
    let source = source(&frames);
    let replay = source.replay();
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let oracle = scan_frames(&replay, 1, &flag)
        .unwrap()
        .0
        .finish(&flag)
        .unwrap();
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1);
    for (batch, frames) in frames.chunks(2).enumerate() {
        for frame in frames {
            prepared
                .observe(frame, &replay, &source, &context, &flag)
                .unwrap();
        }
        let mut writer = if batch == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared
            .finish_block(&mut writer, 1, frames[0].hour, frames.last().unwrap().hour)
            .unwrap();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        assert!(prepared.builder.census.is_none());
        assert!(
            prepared
                .builder
                .scan
                .tracks
                .values()
                .all(|track| track.packed.is_empty())
        );
    }
    for hour in [10, 11, 12, 14, 15, 16] {
        let reply = point(&fixture, hour, None, Vec::new());
        for series in reply["series"].as_array().unwrap() {
            let tag = series["tag"].as_str().unwrap();
            let expected = oracle.country_at(tag, hour);
            if let Some(expected) = expected {
                assert_eq!(
                    series["dealt"][0],
                    expected.dealt.to_string(),
                    "{tag} at {hour}"
                );
                assert_eq!(
                    series["taken"][0],
                    expected.taken.to_string(),
                    "{tag} at {hour}"
                );
                assert_eq!(series["atLeastFrom"].is_null(), !expected.at_least);
            }
        }
        let top = point(&fixture, hour, Some(2), Vec::new());
        let expected = oracle.memberships(&CasualtyParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: Some(CasualtyTop { count: 2 }),
        });
        let selected: BTreeSet<_> = expected
            .iter()
            .filter(|(_, spans)| {
                spans
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|span| span.first_hour <= hour && hour <= span.last_hour)
            })
            .map(|(tag, _)| tag.as_str())
            .collect();
        let actual: BTreeSet<_> = top["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|series| series["tag"].as_str().unwrap())
            .collect();
        assert_eq!(actual, selected);
    }
    let gap = point(&fixture, 13, Some(2), Vec::new());
    assert!(gap["series"].as_array().unwrap().is_empty());
    let alias = point(
        &fixture,
        14,
        None,
        vec![
            dashboard_base::test_fixtures::scale_tag(0)
                .into_iter()
                .map(char::from)
                .collect(),
        ],
    );
    assert!(
        alias["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|series| series["tag"] == "mod renamed country")
    );
}

#[test]
fn manual_selection_before_its_first_war_keeps_branch_sample_warnings_exact() {
    let mut first = frame(10, 2);
    *first.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(Vec::new()));
    let frames = vec![first, frame(11, 2)];
    let source = source(&frames);
    let replay = source.replay();
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1);
    for frame in &frames {
        prepared
            .observe(
                frame,
                &replay,
                &source,
                &LoadContext { content: None },
                &flag,
            )
            .unwrap();
    }
    let mut writer = fixture.writer();
    prepared.finish_block(&mut writer, 1, 10, 11).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let tag = String::from_utf8(dashboard_base::test_fixtures::scale_tag(0)).unwrap();
    let before = point(&fixture, 10, None, vec![tag]);
    assert!(before["series"].as_array().unwrap().is_empty());
    assert!(
        before["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|warning| { !warning.as_str().unwrap().contains("no sampled casualties") })
    );
    let absent = point(&fixture, 10, None, vec!["never recorded".into()]);
    assert!(
        absent["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| { warning == "never recorded: no sampled casualties in this branch" })
    );
}

#[test]
fn observed_spans_survive_actor_order_that_differs_from_observation_order() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut observations = Accumulator::default();
    let mut coverage = Accumulator::default();
    let mut values: [Accumulator; 3] = Default::default();
    let mut catalogue = Catalogue::default();
    let mut keys = BTreeMap::new();
    for (hour, actor, tag) in [(10, 1, "earlier"), (14, 0, "later")] {
        let identity = (actor as u32, actor as u32 + 100);
        catalogue.actors.insert(actor, (tag.into(), identity));
        keys.insert(tag.into(), actor);
        let observation = Observation {
            first: hour,
            sample_first: hour,
            identity,
            read_hour: None,
            at_least: false,
        };
        observations.observe(actor, hour, Some(1), true);
        observations.event(actor, hour, &observation).unwrap();
        coverage.observe(0, hour, Some(1), true);
        for values in &mut values {
            values.observe(actor, hour, Some(0), true);
        }
    }
    let mut writer = fixture.writer();
    observations
        .flush(&mut writer, "casualty-observations-1", 0, &flag)
        .unwrap();
    coverage
        .flush(&mut writer, "casualty-coverage-1", 0, &flag)
        .unwrap();
    for (index, values) in values.iter_mut().enumerate() {
        values
            .flush(&mut writer, &format!("casualty-values-1-{index}"), 0, &flag)
            .unwrap();
    }
    series::write_catalogue(&mut writer, "casualty", 1, &keys, &catalogue).unwrap();
    writer.commit().unwrap();
    let reply = point(&fixture, 14, None, Vec::new());
    assert_eq!(
        reply["observed"],
        json!([{"firstHour":10,"lastHour":10}, {"firstHour":14,"lastHour":14}])
    );
    let tags: Vec<_> = reply["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|series| series["tag"].as_str().unwrap())
        .collect();
    assert_eq!(tags, ["later"], "closed actors keep coverage, not values");
}

#[test]
fn prepared_five_thousand_countries_have_bounded_continuation_and_exact_keys() {
    let frames = vec![
        frame(10, 5000),
        frame(11, 5000),
        frame(12, 5000),
        frame(13, 5000),
    ];
    let source = source(&frames);
    let replay = source.replay();
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1);
    let mut sizes = Vec::new();
    for chunk in frames.chunks(2) {
        for frame in chunk {
            prepared
                .observe(
                    frame,
                    &replay,
                    &source,
                    &LoadContext { content: None },
                    &flag,
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared
            .finish_block(&mut writer, 1, chunk[0].hour, chunk.last().unwrap().hour)
            .unwrap();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        sizes.push(serde_json::to_vec(&prepared).unwrap().len());
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    assert_eq!(prepared.tags.len(), 5000);
    assert_eq!(prepared.builder.scan.countries.len(), 5000);
    assert_eq!(prepared.builder.scan.pairs.len(), 2500);
    assert!(
        prepared
            .builder
            .scan
            .tracks
            .values()
            .all(|track| track.packed.is_empty())
    );
    eprintln!(
        "casualties: 5000 tag keys, 15000 numeric keys, 5000 observation keys; checkpoint bytes after commits: {sizes:?}"
    );
    assert_eq!(
        point(&fixture, 13, Some(10), Vec::new())["series"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}
