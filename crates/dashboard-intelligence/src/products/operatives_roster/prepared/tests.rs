use super::super::tests::{base, frame, row};
use super::*;
use dashboard_base::{prepared::series::tests::Fixture, test_fixtures};
use replay_data::rows::TableRows;
use replay_frame::frame::TableView;

fn source(frames: Vec<replay_frame::frame::DecodedFrame>) -> Source {
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
    engine_executable::executable_tables::initialize_for_tests()
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
        options: installed_content_core::ResolveOptions {
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

#[test]
fn first_view_keeps_selected_string_provenance_and_follows_native_aliases() {
    let mut other = row(2);
    other.roster_country = 9;
    other.owner_country = 9;
    other.name = b"Unselected operative".to_vec().into();
    // This holder's missing assignment reason must not enter the selected dictionary.
    other.action_variant = Some(u32::MAX);
    let frames = (0..=1)
        .map(|hour| {
            let mut input = frame(hour, true, vec![row(1), other.clone()]);
            if hour == 1 {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                else {
                    panic!("countries");
                };
                rows[0].tag = b"renamed holder".to_vec();
            }
            input
        })
        .collect();
    let memory = source(frames);
    let (source, root) =
        test_fixtures::file_source(Replay::open(memory.replay_ref().bytes().to_vec()).unwrap());
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut params = params(Some(base()), Some(base() + 1));
    params.include_departed = false;
    let reply = first_view(&source, &params, &context, &flag).unwrap();
    let countries = reply["countries"].as_array().unwrap();
    assert_eq!(
        countries.len(),
        2,
        "both aliases of the selected native holder"
    );
    for country in countries {
        assert_ne!(country["tag"], "target old");
        let identities = country["identities"].as_array().unwrap();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0]["slot"], 1);
        assert_eq!(identities[0]["countryId"], 7);
        for read in country["observed"].as_array().unwrap() {
            assert!(
                read["firstHour"].as_i64().unwrap() >= identities[0]["firstHour"].as_i64().unwrap()
            );
            assert!(
                read["lastHour"].as_i64().unwrap() <= identities[0]["lastHour"].as_i64().unwrap()
            );
        }
        assert_eq!(country["operatives"].as_array().unwrap().len(), 1);
        let entry = &country["operatives"][0];
        assert_eq!(entry["id"], "1");
        assert_eq!(
            entry["preparing"],
            serde_json::json!(["joinedHour", "departedHour"])
        );
        let value = &entry["runs"][0]["value"];
        for field in ["portraitMissing", "experienceMissing", "assignmentMissing"] {
            let reason = value[field].as_u64().unwrap() as usize;
            assert!(
                reply["reasons"][reason].is_string(),
                "{field} retains its reason"
            );
        }
    }
    assert!(reply["reasons"].as_array().unwrap().iter().all(|reason| {
        !reason
            .as_str()
            .unwrap()
            .contains("Unsupported operative mission variant")
    }));
    let reads = dashboard_base::prepared::query::frames::reads(&source);
    assert_eq!(
        first_view(&source, &params, &context, &flag).unwrap(),
        reply
    );
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&source),
        reads
    );
    let _ = std::fs::remove_dir_all(root);
}

#[derive(Default)]
struct IdentityProgress(Option<dashboard_base::prepared::coverage::Coverage>);

/// Identity preparation advances independently of the roster's accepted pages.
struct IdentityGates;
impl dashboard_base::prepared::coverage::Gates for IdentityGates {
    fn product_coverage(
        &self,
        source: &Source,
        content: Option<&dashboard_base::products::ContentOptions>,
        branch: u64,
        product: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<dashboard_base::prepared::coverage::Coverage>> {
        if product == dashboard_base::products::country_history::IDENTITIES_PRODUCT
            && let Some(coverage) = source.state::<IdentityProgress>().lock().unwrap().0.clone()
        {
            return Ok(Some(coverage));
        }
        dashboard_base::prepared::coverage::coverage(source, content, branch, cancelled)
    }
    fn refusal(
        &self,
        _source: &Source,
        _content: Option<&dashboard_base::products::ContentOptions>,
        _branch: u64,
        _query: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        dashboard_base::metadata::check_cancel(cancelled)
    }
}

/// H1 used A then keeps B and an operative. H2 either keeps A from hour two,
/// or has an operative under C before becoming A at hour 96.
fn historical_roster(
    future_alias: bool,
) -> (
    Source,
    std::path::PathBuf,
    Vec<replay_frame::frame::DecodedFrame>,
) {
    static GATES: IdentityGates = IdentityGates;
    dashboard_base::prepared::coverage::install_gates(&GATES);
    let frames: Vec<_> = (0..120)
        .map(|hour| {
            let mut operatives = vec![row(1)];
            if future_alias {
                let mut operative = row(2);
                operative.roster_country = 9;
                operative.owner_country = 9;
                operatives.push(operative);
            }
            let mut input = frame(hour, true, operatives);
            input.metrics.table_ns[TableId::Operations.index()] = 1;
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
            else {
                panic!("countries");
            };
            rows[0].tag = match hour {
                0..=1 => b"A".to_vec(),
                _ => b"B".to_vec(),
            };
            rows[1].tag = match hour {
                96.. if future_alias => b"A".to_vec(),
                _ if future_alias => b"C".to_vec(),
                0..=1 => b"other holder".to_vec(),
                _ => b"A".to_vec(),
            };
            input
        })
        .collect();
    let mut recorded = test_fixtures::replay(frames[..24].to_vec());
    for hours in frames[24..].chunks(24) {
        recorded = recorded
            .append(&[test_fixtures::continuation_records(
                &recorded,
                1,
                hours.to_vec(),
            )])
            .unwrap();
    }
    assert_eq!(recorded.index.len(), 5);
    let (source, root) = test_fixtures::file_source(recorded);
    (source, root, frames)
}

fn publish_roster_prefix(
    source: &Source,
    frames: &[replay_frame::frame::DecodedFrame],
    last: usize,
) {
    publish_prefixes(source, frames, last, last.min(48));
}

fn publish_prefixes(
    source: &Source,
    frames: &[replay_frame::frame::DecodedFrame],
    identity_last: usize,
    roster_last: usize,
) {
    use dashboard_base::{
        prepared::coverage::publish_coverage, products::country_history::HistoryScan,
    };
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut identities = HistoryScan::new(false, &[]);
    let mut roster = Prepared::default();
    for (index, input) in frames[..=identity_last.max(roster_last)].iter().enumerate() {
        if index <= identity_last {
            identities.observe(input);
        }
        if index <= roster_last {
            roster.observe(source, &context, input).unwrap();
        }
    }
    let (store, key) = dashboard_base::prepared::files(source, None).unwrap();
    let mut writer = store.replace(key).unwrap();
    roster.flush(&mut writer, 1, &flag).unwrap();
    writer
        .write_at(
            "country-identities-1",
            0,
            serde_json::to_vec(&identities).unwrap().as_slice(),
        )
        .unwrap();
    publish_coverage(&mut writer, 1, base(), frames[roster_last].hour, false).unwrap();
    writer.commit().unwrap();
    source.state::<IdentityProgress>().lock().unwrap().0 =
        Some(dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: base(),
            last_hour: frames[identity_last].hour,
            complete: identity_last + 1 == frames.len(),
        });
}

fn historical_day(source: &Source) -> serde_json::Value {
    crate::products::intelligence_prepared::query(
        source,
        "operatives-roster",
        serde_json::json!({"branchId":"1", "tags":["A"],
            "firstHour":base()+48, "lastHour":base()+71, "includeDeparted":false}),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn assert_selection_pending(reply: &serde_json::Value) {
    assert_eq!(reply["preparing"], serde_json::json!(["countries"]));
    let first =
        i64::from(base() + 48).max(reply["preparedCoverage"]["lastHour"].as_i64().unwrap() + 1);
    assert_eq!(
        reply["pending"],
        serde_json::json!([{"firstHour":first, "lastHour":base()+71}])
    );
}

#[test]
fn a_local_holder_does_not_complete_a_branch_wide_tag_selection() {
    let (source, root, frames) = historical_roster(false);
    let cold = historical_day(&source);
    assert_selection_pending(&cold);
    assert_eq!(cold["countries"][0]["tag"], "A");
    assert!(
        cold["countries"][0]["operatives"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // The accepted early alias adds B's known operative before reaching the day.
    publish_roster_prefix(&source, &frames, 23);
    let early = historical_day(&source);
    assert_selection_pending(&early);
    assert_eq!(early["countries"][1]["tag"], "B");
    assert_eq!(early["countries"][1]["operatives"][0]["id"], "1");
    publish_roster_prefix(&source, &frames, 48);
    assert_selection_pending(&historical_day(&source));
    publish_roster_prefix(&source, &frames, 119);
    let reply = historical_day(&source);
    assert_eq!(reply["pending"], serde_json::json!([]));
    assert!(reply.get("preparing").is_none());
    let countries = reply["countries"].as_array().unwrap();
    assert_eq!(countries.len(), 2, "A selects both native holders");
    assert_eq!(countries[0]["tag"], "A");
    assert!(countries[0]["operatives"].as_array().unwrap().is_empty());
    assert_eq!(countries[1]["tag"], "B");
    assert_eq!(countries[1]["operatives"][0]["id"], "1");
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    let expected = query(
        &source,
        &LoadContext { content: None },
        &store,
        &key,
        1,
        &Params {
            branch_id: "1".into(),
            tags: vec!["A".into()],
            first_hour: Some(base() + 48),
            last_hour: Some(base() + 48),
            include_departed: false,
        },
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        expected
            .countries
            .iter()
            .filter(|country| !country.operatives.is_empty())
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
    let exact_country = expected
        .countries
        .iter()
        .find(|country| country.tag == "B")
        .unwrap();
    assert_eq!(
        reply["countries"][1]["operatives"][0]["runs"][0]["value"],
        serde_json::to_value(&exact_country.operatives[0].runs[0].value).unwrap(),
        "the historical selection keeps the prepared hour's recorded values"
    );
    store.clear().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_future_alias_adds_its_holder_to_the_cached_day_before_selection_completes() {
    use dashboard_base::prepared::query::{frames as cached_frames, observations};
    let (source, root, frames) = historical_roster(true);
    let cold = historical_day(&source);
    assert_selection_pending(&cold);
    assert!(cold.as_object().unwrap().contains_key("selectionCoverage"));
    assert!(cold["selectionCoverage"].is_null());
    assert_eq!(historical_day(&source), cold);
    assert_eq!(observations::usage(&source, "operatives-roster:day").2, 1);
    let reads = cached_frames::reads(&source);
    publish_roster_prefix(&source, &frames, 48);
    let before = historical_day(&source);
    assert_selection_pending(&before);
    assert_eq!(before["selectionCoverage"]["lastHour"], base() + 48);
    assert_eq!(before["countries"].as_array().unwrap().len(), 1);
    assert_eq!(before["countries"][0]["operatives"][0]["id"], "1");
    assert_eq!(observations::usage(&source, "operatives-roster:day").2, 2);
    publish_roster_prefix(&source, &frames, 96);
    let later = historical_day(&source);
    assert_selection_pending(&later);
    assert_eq!(later["selectionCoverage"]["lastHour"], base() + 96);
    assert_eq!(later["selectionCoverage"]["complete"], false);
    for field in ["preparedCoverage", "pending", "preparing"] {
        assert_eq!(later[field], before[field], "only identity progress moves");
    }
    assert_eq!(later["countries"][0]["tag"], "B");
    assert_eq!(later["countries"][1]["tag"], "C");
    assert_eq!(later["countries"][1]["operatives"][0]["id"], "2");
    assert_eq!(historical_day(&source), later);
    assert_eq!(observations::usage(&source, "operatives-roster:day").2, 3);
    publish_roster_prefix(&source, &frames, 119);
    let resolved = historical_day(&source);
    assert_eq!(resolved["selectionCoverage"]["complete"], true);
    assert_eq!(resolved["pending"], serde_json::json!([]));
    assert!(resolved.get("preparing").is_none());
    assert_eq!(resolved["countries"][1]["operatives"][0]["id"], "2");
    assert_eq!(historical_day(&source), resolved);
    assert_eq!(observations::usage(&source, "operatives-roster:day").2, 4);
    assert_eq!(cached_frames::reads(&source), reads);
    let (store, _) = dashboard_base::prepared::files(&source, None).unwrap();
    store.clear().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn prepared_day_values_keep_the_branch_wide_selection_preparing() {
    let (source, root, frames) = historical_roster(true);
    for (identity_last, tags) in [
        (48, &["A", "B"][..]),
        (96, &["A", "B", "C"][..]),
        (119, &["A", "B", "C"][..]),
    ] {
        publish_prefixes(&source, &frames, identity_last, 71);
        let reply = historical_day(&source);
        assert_eq!(
            reply["selectionCoverage"]["lastHour"],
            frames[identity_last].hour
        );
        assert_eq!(reply["selectionCoverage"]["complete"], identity_last == 119);
        assert_eq!(reply["pending"], serde_json::json!([]));
        let countries = reply["countries"].as_array().unwrap();
        assert_eq!(
            countries
                .iter()
                .map(|country| country["tag"].as_str().unwrap())
                .collect::<Vec<_>>(),
            tags
        );
        assert!(countries[0]["operatives"].as_array().unwrap().is_empty());
        assert_eq!(countries[1]["operatives"][0]["id"], "1");
        if identity_last >= 96 {
            assert_eq!(countries[2]["operatives"][0]["id"], "2");
        }
        if identity_last < 119 {
            assert_eq!(reply["preparing"], serde_json::json!(["countries"]));
        } else {
            assert!(reply.get("preparing").is_none());
        }
    }
    let (store, _) = dashboard_base::prepared::files(&source, None).unwrap();
    store.clear().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
