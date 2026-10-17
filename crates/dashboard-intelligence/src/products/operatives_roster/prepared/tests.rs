use super::super::tests::{base, frame, row};
use super::*;
use dashboard_base::{prepared::series::tests::Fixture, test_fixtures};
use replay::{frame::TableView, rows::TableRows};

fn source(frames: Vec<replay::frame::DecodedFrame>) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-roster".into(),
            revision: "synthetic".into(),
        },
        "fixtures/roster.battleplan".into(),
        std::sync::Arc::new(
            Replay::from_records(
                test_fixtures::CAMPAIGN.into(),
                &[test_fixtures::records(1, frames)],
            )
            .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn params(first_hour: Option<i32>, last_hour: Option<i32>) -> Params {
    Params {
        branch_id: "1".into(),
        tags: vec!["all!countries".into()],
        first_hour,
        last_hour,
        include_departed: true,
    }
}

#[test]
fn prepared_roster_matches_commits_resume_append_gaps_and_departure() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames: Vec<_> = (0..144)
        .map(|hour| {
            let mut operative = row(1);
            operative.experience_raw += i64::from(hour / 24);
            if (24..48).contains(&hour) {
                operative.assignment = 0;
                operative.captured_hour = base() + 22;
            }
            let mut input = frame(
                hour,
                hour % 24 == 0,
                if hour < 96 {
                    vec![operative]
                } else {
                    Vec::new()
                },
            );
            if hour == 63 {
                *input.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable {
                    reason: "synthetic missed read".into(),
                };
            }
            input
        })
        .collect();
    let source = source(frames.clone());
    let history =
        dashboard_base::products::country_history::branch_histories(&source, 1, false, &[], &flag)
            .unwrap();
    let selected = dashboard_base::products::country_history::selected_identities(
        &history,
        &params(None, None).tags,
    );
    let mut old = Scan::new(&params(None, None).tags, None, None, false).unwrap();
    let mut prepared = Prepared::default();
    for phase in 0..3 {
        for input in &frames[phase * 48..(phase + 1) * 48] {
            old.observe(input, None, Some(&selected)).unwrap();
            prepared.observe(&source, &context, input).unwrap();
        }
        let mut writer = if phase == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        let actual = query_history(
            &fixture.store,
            &fixture.key,
            1,
            &params(None, None),
            &history,
            &flag,
        )
        .unwrap()
        .unwrap();
        let mut expected = old.reply(1, Some(&selected)).unwrap();
        expected.countries[0].identities = history.countries["all!countries"]
            .identities
            .as_ref()
            .clone();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        prepared = series::read_reply(&fixture.store, &fixture.key, "operatives-roster-1", 0)
            .unwrap()
            .unwrap();
        assert!(
            prepared
                .scan
                .as_ref()
                .unwrap()
                .histories
                .iter()
                .all(|h| h.reply.observed.len() <= 1
                    && h.reply.operatives.iter().all(|e| e.runs.len() <= 1))
        );
    }
    let actual = query_history(
        &fixture.store,
        &fixture.key,
        1,
        &params(Some(base() + 120), Some(base() + 143)),
        &history,
        &flag,
    )
    .unwrap()
    .unwrap();
    let entry = &actual.countries[0].operatives[0];
    assert_eq!(entry.departed_hour, Some(base() + 96));
    assert_eq!(entry.runs.last().unwrap().last_hour, base() + 95);
    assert!(
        entry
            .runs
            .iter()
            .any(|run| run.value.captured_hour == Some(base() + 22))
    );
}

#[test]
fn prepared_roster_filters_replaced_alias_holders_and_refusals() {
    let frames: Vec<_> = (0..96)
        .map(|hour| {
            let mut operative = row(1);
            if hour >= 48 {
                operative.name = vec![0xff].into();
            }
            let mut input = frame(hour, hour % 24 == 0, vec![operative]);
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *input.tables[TableId::Countries.index()]
            else {
                panic!("countries");
            };
            if hour >= 24 {
                countries[0].tag = b"alias / opaque".to_vec();
            }
            if hour >= 48 {
                countries[0].id = 70;
                countries.sort_by_key(|country| country.id);
            }
            if hour >= 48 {
                let TableView::Complete(TableRows::CountryLinks(links)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                else {
                    panic!("links");
                };
                links.retain(|row| row.id != 7);
                links.push(CountryLinkRow {
                    id: 70,
                    canonical_slot: 1,
                });
                links.sort_by_key(|row| row.id);
            }
            input
        })
        .collect();
    let source = source(frames.clone());
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let history =
        dashboard_base::products::country_history::branch_histories(&source, 1, false, &[], &flag)
            .unwrap();
    let selected = dashboard_base::products::country_history::selected_identities(
        &history,
        &params(None, None).tags,
    );
    let tags = dashboard_base::products::country_history::selected_aliases(
        &history,
        &params(None, None).tags,
    );
    let mut old = Scan::new(&tags, None, None, false).unwrap();
    let mut state = Prepared::default();
    for chunk in frames.chunks(48) {
        for input in chunk {
            old.observe(input, None, Some(&selected)).unwrap();
            state
                .observe(&source, &LoadContext { content: None }, input)
                .unwrap();
        }
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
    }
    let actual = query_history(
        &fixture.store,
        &fixture.key,
        1,
        &params(None, None),
        &history,
        &flag,
    )
    .unwrap()
    .unwrap();
    let mut expected = old.reply(1, Some(&selected)).unwrap();
    for country in &mut expected.countries {
        country.identities = history.countries[&country.tag].identities.as_ref().clone();
    }
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_roster_handles_five_thousand_countries_without_daily_checkpoint_growth() {
    let mut input = frame(0, true, vec![]);
    *input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5_000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod-country-{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=5_000)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *input.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(
            (1..=5_000)
                .map(|id| OperativeMembershipRow {
                    id: u64::from(id),
                    roster_country: id,
                    owner_country: id,
                    ..row(u64::from(id))
                })
                .collect(),
        ));
    let source = source(vec![input.clone()]);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut state = Prepared::default();
    let mut short = 0;
    for day in 0..100 {
        for offset in 0..24 {
            input.hour = base() + day * 24 + offset;
            input.metrics.table_ns[TableId::OperativeMemberships.index()] = u64::from(offset == 0);
            state
                .observe(&source, &LoadContext { content: None }, &input)
                .unwrap();
        }
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        if day == 1 {
            short = serde_json::to_vec(&state).unwrap().len();
        }
    }
    let long = serde_json::to_vec(&state).unwrap().len();
    eprintln!("roster checkpoint bytes (5,000 countries): 2 days={short}, 100 days={long}");
    assert!(long < short * 2);
    let scan = state.scan.as_ref().unwrap();
    assert_eq!(scan.histories.len(), 5_000);
    assert!(scan.histories.iter().all(|h| h.reply.observed.len() == 1
        && h.reply.operatives.len() == 1
        && h.reply.operatives[0].runs.len() == 1));
}

#[test]
fn intel_shared_driver_roster_equal_reads_are_flat_and_exact() {
    use crate::products::{intelligence_prepared::measured, operative_rows::CountryInputs};
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let scaled = observer_tests::scaled_work(8, |hours| {
        let first = frame(0, true, vec![row(1)]);
        let frames: Vec<_> = (0..=hours)
            .map(|offset| {
                let mut input = first.clone();
                input.hour += offset as i32;
                input
            })
            .collect();
        let source = source(frames.clone());
        let flag = AtomicBool::new(false);
        let history = dashboard_base::products::country_history::branch_histories(
            &source,
            1,
            false,
            &[],
            &flag,
        )?;
        let selected = dashboard_base::products::country_history::selected_identities(
            &history,
            &params(None, None).tags,
        );
        let mut old = Scan::new(&params(None, None).tags, None, None, false)?;
        for input in &frames {
            old.observe(input, None, Some(&selected))?;
        }
        let mut fixture = Fixture::new();
        let mut state = Prepared::default();
        let mut countries = CountryInputs::default();
        let changes = FrameChangeTracker::new().observe(&first, source.replay_ref().format);
        countries.update(&first, Some(&changes));
        state.observe_bound(&first, &changes, &countries, &Ok(None))?;
        let delta = observer_tests::unchanged_frames(
            measured(),
            &first,
            hours as i32,
            source.replay_ref().format,
            |input, changes| {
                let restored = if input.hour == first.hour + (hours / 2) as i32 {
                    let mut writer = fixture.writer();
                    state.flush(&mut writer, 1, &flag)?;
                    writer.commit()?;
                    state =
                        series::read_reply(&fixture.store, &fixture.key, "operatives-roster-1", 0)?
                            .unwrap();
                    countries = CountryInputs::default();
                    Some(FrameChangeTracker::new().observe(input, source.replay_ref().format))
                } else {
                    None
                };
                let changes = restored.as_ref().unwrap_or(changes);
                countries.update(input, Some(changes));
                state.observe_bound(input, changes, &countries, &Ok(None))?;
                Ok(measured())
            },
        )?;
        let mut writer = fixture.append_writer();
        state.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        let actual = query_history(
            &fixture.store,
            &fixture.key,
            1,
            &params(None, None),
            &history,
            &flag,
        )?
        .unwrap();
        let mut expected = old.reply(1, Some(&selected))?;
        expected.countries[0].identities = history.countries["all!countries"]
            .identities
            .as_ref()
            .clone();
        observer_tests::assert_reply_parity(&actual, &expected)?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].history_visits, 1);
}

#[test]
fn restored_intelligence_observer_publishes_what_continuous_observation_publishes() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for metadata tables");
    let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "common/defines/00_fixture.lua",
        "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture\" } } }",
    )]);
    let catalogue = std::sync::Arc::new(catalogue);
    // A held cooldown's progress is a single-precision ratio of whole days.
    let operative = OperativeMembershipRow {
        assignment: 1,
        cooldown_kind: Some(3).into(),
        cooldown_start_hour: Some(base()).into(),
        cooldown_end_hour: Some(base() + 300).into(),
        ..row(1)
    };
    let operative_content = super::super::super::operative_content::test_content_from_catalogue(
        std::sync::Arc::clone(&catalogue),
    );
    let progress = |hour: i32| {
        operative_content
            .cooldown_status(3, base(), base() + 300, base() + hour)
            .progress
            .map(f64::from)
    };
    // The held projection reaches 3/13 at hour 60, between the daily reads; the
    // read at hour 72 supplies that same progress again. Widened, 3/13 needs
    // seventeen significant digits, so the committed continuation stores it
    // as its shortest seventeen-digit decimal.
    assert_ne!(progress(59), progress(60));
    assert_eq!(progress(60), progress(72));
    assert_eq!(progress(72).unwrap().to_string().len(), "0.".len() + 17);
    let frames: Vec<_> = (0..=96)
        .map(|hour| frame(hour, hour % 24 == 0, vec![operative.clone()]))
        .collect();
    // Two recorded blocks, committed separately after hour 65: inside the 3/13
    // day and before its read.
    let original = test_fixtures::records(1, frames[..66].to_vec());
    let first = Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        std::slice::from_ref(&original),
    )
    .unwrap();
    let continued = test_fixtures::continuation_records(&first, 1, frames[66..].to_vec());
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-roster-restore".into(),
            revision: "synthetic".into(),
        },
        "fixtures/roster-restore.battleplan".into(),
        std::sync::Arc::new(
            Replay::from_records(test_fixtures::CAMPAIGN.into(), &[original, continued]).unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(source.replay_ref().index.len(), 2);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    dashboard_base::catalogue::set_test_catalogue(
        &source,
        &options,
        std::sync::Arc::clone(&catalogue),
    );
    let content = crate::products::intelligence_prepared::Content::load(
        &source,
        &LoadContext {
            content: Some(options),
        },
    )
    .unwrap();
    use crate::products::intelligence_prepared::Observer;
    let flag = AtomicBool::new(false);
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        source.replay_ref().format,
        &[&frames[..66], &frames[66..]],
        Observer::default,
        |observer, input, changes, fixture| {
            observer
                .observe(
                    &source,
                    &content,
                    input,
                    changes,
                    &fixture.store,
                    &fixture.key,
                    &flag,
                )
                .unwrap()
        },
        |observer, writer, ordinal| {
            observer
                .finish_block(&source, writer, 1, ordinal, &flag)
                .unwrap();
            observer.flush(writer, 1, &flag).unwrap();
        },
        |fixture| Observer::load(&fixture.store, &fixture.key, Some(&fixture.key), 1).unwrap(),
    );
}

#[test]
fn intel_shared_driver_roster_one_row_ignores_ten_times_departed_handles() {
    use crate::products::intelligence_prepared::{measured, work};
    use dashboard_base::prepared::observer_tests;
    let scaled = observer_tests::scaled_work(8, |history| {
        let mut input = frame(0, true, vec![row(1)]);
        let source = source(vec![input.clone()]);
        let context = LoadContext { content: None };
        let mut state = Prepared::default();
        for index in 0..history {
            input.hour = base() + index as i32;
            let TableView::Complete(TableRows::OperativeMemberships(rows)) =
                &mut *input.tables[TableId::OperativeMemberships.index()]
            else {
                unreachable!()
            };
            rows[0].id = index as u64 + 1;
            state.observe(&source, &context, &input)?;
        }
        assert_eq!(
            state.scan.as_ref().unwrap().histories[0]
                .reply
                .operatives
                .len(),
            history
        );
        input.hour += 1;
        let TableView::Complete(TableRows::OperativeMemberships(rows)) =
            &mut *input.tables[TableId::OperativeMemberships.index()]
        else {
            unreachable!()
        };
        rows[0].experience_raw += 1;
        work(|work| *work = Default::default());
        state.observe(&source, &context, &input)?;
        Ok(measured())
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].emitted, 1);
}

#[test]
fn first_view_selects_requested_holders_before_reply_budgeting() {
    let mut other = row(2);
    other.roster_country = 9;
    other.owner_country = 9;
    let frames = (0..=1)
        .map(|hour| frame(hour, true, vec![row(1), other.clone()]))
        .collect();
    let memory = source(frames);
    let (source, root) = test_fixtures::file_source(
        replay::container::Replay::open(memory.replay_ref().bytes().to_vec()).unwrap(),
    );
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let params = params(Some(base()), Some(base() + 1));
    let reply = first_view(&source, &params, &context, &flag).unwrap();
    let countries = reply["countries"].as_array().unwrap();
    assert_eq!(
        countries.len(),
        1,
        "an unrequested roster must not reach the reply"
    );
    assert_eq!(countries[0]["tag"], "all!countries");
    assert_eq!(countries[0]["operatives"].as_array().unwrap().len(), 1);
    let mut absent = params;
    absent.tags = vec!["unseen holder".into()];
    let empty = first_view(&source, &absent, &context, &flag).unwrap();
    assert!(empty["countries"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}
