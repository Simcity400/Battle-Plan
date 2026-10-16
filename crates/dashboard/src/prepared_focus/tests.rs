use super::*;
use crate::prepared::frame_changes::FrameChangeTracker;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{CompletedFocusRow, CountryCounterInputRow, FocusRow, TableRows},
};

// Numeric fixtures omit presentation details and count only rebuilt observations.
impl FocusAccumulator {
    fn frame_changes(&mut self, frame: &DecodedFrame) -> FrameChanges {
        self.test_changes
            .get_or_insert_with(|| Arc::new(std::sync::Mutex::new(FrameChangeTracker::new())))
            .lock()
            .unwrap()
            .observe(frame, replay::format::FormatVersion::CURRENT)
    }
    pub(crate) fn observe(
        &mut self,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
        mut emit: impl FnMut(Observation) -> io::Result<()>,
    ) -> io::Result<()> {
        let changes = self.frame_changes(frame);
        let mut rows = Vec::new();
        self.observe_changes(frame, &changes, cancelled, |tag, observation| {
            rows.push((tag.to_owned(), observation));
            Ok(())
        })?;
        for (tag, observation) in rows {
            emit(observation.unwrap_or_else(|| self.observation(&tag, frame.hour)))?;
        }
        Ok(())
    }
}
impl FocusEvaluator {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_test_frame(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        content: Option<&ContentOptions>,
        frame: &DecodedFrame,
        accumulator: &mut FocusAccumulator,
        index: &mut FocusIndex,
        preferred: Option<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<(u64, Option<String>)> {
        let changes = self
            .test_changes
            .get_or_insert_with(FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        self.write_frame(
            writer,
            source,
            content,
            frame,
            accumulator,
            index,
            preferred,
            cancelled,
            &changes,
        )
    }
}
impl FocusIndex {
    #[allow(clippy::too_many_arguments)]
    fn write_hour(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        observation: Observation,
        presentation: &mut dyn Iterator<Item = artwork::ResolvedArt>,
        warnings: Vec<String>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.write_hour_with_details(
            writer,
            branch,
            observation,
            presentation,
            None,
            warnings,
            cancelled,
        )
    }
    fn hour(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        tag: &str,
        hour: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Hour>> {
        query_hour(store, key, branch, tag, hour, cancelled)
    }
}
fn frame(hour: i32, count: u32, tokens: &[&str]) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        (0..count)
            .map(|index| FocusRow {
                country: 100_000 + index,
                current: Some(b"mod/current".to_vec()),
                progress: i64::MIN,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(
            (0..count)
                .flat_map(|index| {
                    tokens
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, token)| CompletedFocusRow {
                            country: 100_000 + index,
                            ordinal: ordinal as u32,
                            token: token.as_bytes().to_vec(),
                        })
                })
                .collect(),
        ));
    frame
}

fn presentation_source(
    frames: &[DecodedFrame],
    catalogue: Arc<crate::catalogue::Catalogue>,
    root: &std::path::Path,
) -> (Source, ContentOptions) {
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: uuid::Uuid::new_v4().to_string(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: root.to_owned(),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
    };
    crate::catalogue::set_test_catalogue(&source, &content, catalogue);
    (source, content)
}

fn observer_work(
    source: &Source,
    content: &ContentOptions,
    evaluator: &FocusEvaluator,
    index: &FocusIndex,
    accumulator: &FocusAccumulator,
) -> crate::prepared::observer_tests::Work {
    crate::prepared::observer_tests::Work {
        content_loads: crate::catalogue::catalogue_loads(source, content)
            + evaluator
                .catalogue
                .as_ref()
                .map_or(0, |catalogue| artwork::Context::content_loads(catalogue)),
        row_visits: evaluator.counts.observations + accumulator.work.row_visits,
        history_visits: evaluator.counts.completed_rows_copied + accumulator.work.history_visits,
        emitted: index
            .countries
            .values()
            .map(|country| {
                index
                    .series
                    .state(country.key)
                    .map_or(0, |state| state.events) as usize
            })
            .sum::<usize>()
            + index.series.state(0).map_or(0, |state| state.events) as usize,
        interpreter_calls: evaluator.interpreter_calls,
        dictionary_interpretations: 0,
    }
}

#[test]
fn configured_focus_content_and_refusals_do_not_reload_for_scaled_hours_and_history() {
    engine::executable_tables::initialize_for_tests().unwrap();
    use crate::prepared::observer_tests::{assert_flat, scaled_work, unchanged_frames};
    for refused in [false, true] {
        let histories = scaled_work(2, |history| {
            let hours = scaled_work(2, |hours| {
                let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
                    ("common/national_focus/configured.txt", "focus_tree={focus={id=\"mod/a\" text=title_a} focus={id=\"mod/current\" text=scoped_title} focus={id=unused text=unused_scripted}}"),
                    ("common/defines/configured.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}"),
                    ("localisation/configured_l_english.yml", "\u{feff}l_english:\n title_a:0 \"A\"\n scoped_title:0 \"Current\"\n scoped_title_desc:0 \"For current\"\n unused_scripted:0 \"[ROOT.GetTag]\"\n"),
                ]);
                if refused {
                    // The stack has inventoried this synthetic file. Its I/O
                    // refusal reaches the real focus database loader.
                    std::fs::remove_file(folder.0.join("common/national_focus/configured.txt"))?;
                }
                let decoded = frame(100, 3, &vec!["mod/a"; history]);
                let (source, content) = presentation_source(std::slice::from_ref(&decoded), Arc::new(catalogue), &folder.0);
                let cancelled = AtomicBool::new(false);
                let mut evaluator = FocusEvaluator::default();
                let error = evaluator.configure(&source, &content).err().map(|error| error.to_string());
                assert_eq!(error.is_some(), refused);
                let recording = folder.0.join("synthetic.battleplan");
                std::fs::write(&recording, b"synthetic").unwrap();
                let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
                let mut writer = store.replace(store::SourceKey::capture(&recording, &[]).unwrap()).unwrap();
                let mut accumulator = FocusAccumulator::default();
                let mut index = FocusIndex::default();
                let configured = observer_work(&source, &content, &evaluator, &index, &accumulator);
                assert_eq!(configured.content_loads, 1, "an unrecorded scripted definition must not demand its database");
                let mut observe = |frame: &DecodedFrame, changes: &FrameChanges| {
                    let (failures, reason) = evaluator.write_frame(&mut writer, &source, Some(&content), frame, &mut accumulator, &mut index, None, &cancelled, changes)?;
                    assert_eq!(failures, if refused { 3 } else { 0 });
                    assert_eq!(reason, error);
                    for country in index.countries.values() {
                        let last = country.last.as_ref().unwrap();
                        assert_eq!(last.order.completed.len(), history);
                        if let Some(reason) = &error {
                            assert_eq!(last.warnings, std::slice::from_ref(reason));
                        }
                    }
                    Ok(observer_work(&source, &content, &evaluator, &index, &accumulator))
                };
                let first = observe(&decoded, &FrameChangeTracker::new().observe(&decoded, source.replay_ref().format))?;
                assert_eq!(first.content_loads, configured.content_loads, "first observation loaded content after configure");
                let work = unchanged_frames(first, &decoded, hours as i32, source.replay_ref().format, &mut observe)?;
                index.flush(&mut writer, &cancelled)?;
                writer.commit()?;
                store.clear()?;
                Ok(work)
            })?;
            assert_eq!(hours[0].content_loads, 0);
            assert_eq!(hours[1].content_loads, 0);
            assert_flat(hours);
            assert_eq!(hours[0].row_visits, 0);
            assert_eq!(hours[0].history_visits, 0);
            Ok(hours[1])
        }).unwrap();
        assert_eq!(histories[0].content_loads, 0);
        assert_eq!(histories[1].content_loads, 0);
        assert_flat(histories);
    }
}

#[test]
fn configured_focus_catalogue_failure_is_loaded_once_and_keeps_hourly_refusal_counts() {
    use crate::prepared::observer_tests::unchanged_frames;
    let decoded = frame(100, 3, &["mod/a", "mod/a"]);
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded.clone()])]).unwrap();
    replay.identity.verification = replay::identity::Verification::Unchecked;
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "refused-focus-content".into(),
            revision: "1".into(),
        },
        folder.0.join("synthetic.battleplan"),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: false,
    };
    let mut evaluator = FocusEvaluator::default();
    let error = evaluator
        .configure(&source, &content)
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("recording has unverified content:"));
    let recording = folder.0.join("synthetic.battleplan");
    std::fs::write(&recording, b"synthetic").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
    let mut writer = store.replace(key).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut accumulator = FocusAccumulator::default();
    let mut index = FocusIndex::default();
    let configured = observer_work(&source, &content, &evaluator, &index, &accumulator);
    assert_eq!(configured.content_loads, 1);
    evaluator
        .write_test_frame(
            &mut writer,
            &source,
            Some(&content),
            &decoded,
            &mut accumulator,
            &mut index,
            None,
            &cancelled,
        )
        .unwrap();
    let warm = observer_work(&source, &content, &evaluator, &index, &accumulator);
    let work = unchanged_frames(
        warm,
        &decoded,
        20,
        source.replay_ref().format,
        |frame, changes| {
            let (failures, reason) = evaluator.write_frame(
                &mut writer,
                &source,
                Some(&content),
                frame,
                &mut accumulator,
                &mut index,
                None,
                &cancelled,
                changes,
            )?;
            assert_eq!(
                failures, 3,
                "cached failure must still count every affected country/hour"
            );
            assert_eq!(reason.as_deref(), Some(error.as_str()));
            // Exercise a second consumer of the actual shared Source cache.
            assert_eq!(
                crate::catalogue::catalogue(&source, &content)
                    .err()
                    .unwrap()
                    .to_string(),
                error
            );
            Ok(observer_work(
                &source,
                &content,
                &evaluator,
                &index,
                &accumulator,
            ))
        },
    )
    .unwrap();
    assert_eq!(work.content_loads, 0);
    assert_eq!(evaluator.counts.failed_countries, 63);
    assert_eq!(work.row_visits, 0);
    assert_eq!(work.history_visits, 0);
    drop(writer);
    store.clear().unwrap();
}

#[cfg(feature = "slow-tests")]
#[test]
fn shared_facts_keep_5000_country_work_flat_and_progress_updates_history_independent() {
    engine::executable_tables::initialize_for_tests().unwrap();
    use crate::prepared::observer_tests::{scaled_work, unchanged_frames};
    let measured = scaled_work(2, |history| {
        let hours = scaled_work(10, |hours| {
            let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
                ("common/national_focus/work.txt", "focus_tree={focus={id=\"mod/a\" text=title_a} focus={id=\"mod/current\" text=title_c}}"),
                ("common/defines/work.lua", "NDefines={NFocus={FOCUS_POINT_DAYS=7}}"),
                ("localisation/work_l_english.yml", "\u{feff}l_english:\n title_a:0 \"A\"\n title_c:0 \"Current\"\n"),
            ]);
            let decoded = frame(100, 5_000, &vec!["mod/a"; history]);
            let (source, content) = presentation_source(std::slice::from_ref(&decoded), Arc::new(catalogue), &folder.0);
            let mut evaluator = FocusEvaluator::default();
            evaluator.configure(&source, &content)?;
            let recording = folder.0.join("synthetic.battleplan");
            std::fs::write(&recording, b"synthetic")?;
            let key = store::SourceKey::capture(&recording, &[])?;
            let store = store::Store::local(uuid::Uuid::new_v4())?;
            let mut writer = store.replace(key.clone())?;
            let mut accumulator = FocusAccumulator::default();
            let mut index = FocusIndex::default();
            let cancelled = AtomicBool::new(false);
            evaluator.write_test_frame(&mut writer, &source, Some(&content), &decoded, &mut accumulator, &mut index, None, &cancelled)?;
            let before = observer_work(&source, &content, &evaluator, &index, &accumulator);
            let unchanged = unchanged_frames(before, &decoded, hours as i32, source.replay_ref().format, |frame, changes| {
                evaluator.write_frame(&mut writer, &source, Some(&content), frame, &mut accumulator, &mut index, None, &cancelled, changes)?;
                Ok(observer_work(&source, &content, &evaluator, &index, &accumulator))
            })?;
            assert_eq!(unchanged, Default::default(), "old per-country carry loop visits 5000 * hours rows");
            let mut last = decoded.clone();
            last.hour += hours as i32;
            let mut tracker = FrameChangeTracker::new();
            tracker.observe(&last, source.replay_ref().format);
            let mut changed = last.clone();
            changed.hour += 1;
            if let TableView::Complete(TableRows::Focus(rows)) = &mut *changed.tables[TableId::Focus.index()] { rows[0].progress += 10; }
            let changes = tracker.observe(&changed, source.replay_ref().format);
            assert!(!changes.rows_changed[TableId::CompletedFocuses.index()]);
            let before = observer_work(&source, &content, &evaluator, &index, &accumulator);
            let native_before = accumulator.work.row_visits;
            evaluator.write_frame(&mut writer, &source, Some(&content), &changed, &mut accumulator, &mut index, None, &cancelled, &changes)?;
            let after = observer_work(&source, &content, &evaluator, &index, &accumulator);
            assert_eq!(accumulator.work.row_visits - native_before, 5_001, "visit the 5000 changed Focus rows and only the one changed country's cached operands; no unchanged country/link table scan");
            assert_eq!(after.history_visits, before.history_visits, "progress must not copy or walk the completed history");
            assert_eq!(after.interpreter_calls, before.interpreter_calls);
            assert_eq!(after.emitted, before.emitted);
            index.flush(&mut writer, &cancelled)?;
            writer.commit()?;
            let row = query_hour(&store, &key, 1, "000", changed.hour, &cancelled)?.unwrap();
            assert_eq!(row.progress_raw, (i64::MIN + 10).to_string());
            assert_eq!(row.daily_gain_raw.as_deref(), Some("10"));
            assert_eq!(row.completed.rows, history as u64);
            store.clear()?;
            Ok(crate::prepared::observer_tests::Work {
                row_visits: after.row_visits - before.row_visits,
                history_visits: after.history_visits - before.history_visits,
                emitted: after.emitted - before.emitted,
                content_loads: after.content_loads - before.content_loads,
                interpreter_calls: after.interpreter_calls - before.interpreter_calls,
                dictionary_interpretations: after.dictionary_interpretations
                    - before.dictionary_interpretations,
            })
        })?;
        assert_eq!(hours[0], hours[1]);
        Ok(hours[1])
    }).unwrap();
    assert_eq!(measured[0], measured[1]);
    assert_eq!(measured[0].history_visits, 0);
    eprintln!(
        "focus carried-hour row visits: before=50000/500000 after=0/0; progress completed-history visits at H=2/20: before proportional to H, after=0/0"
    );
}

#[test]
fn dynamic_presentation_builds_scoped_inputs_once_per_frame_for_5000_countries() {
    engine::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/national_focus/scoped.txt",
            "focus_tree={focus={id=\"mod/current\" text=scoped_title icon={GFX_first={tag=000} GFX_other=yes}}}",
        ),
        (
            "common/defines/scoped.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
        ),
        (
            "localisation/scoped_l_english.yml",
            "\u{feff}l_english:\n scoped_title:0 \"[ROOT.GetTag] [FROM.GetTag]\"\n scoped_title_desc:0 \"For [ROOT.GetTag]\"\n",
        ),
    ]);
    let mut decoded = frame(100, 5_000, &[]);
    // The tag trigger requires a recorded owned-state count for its operand.
    *decoded.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (0..5_000)
                .map(|index| CountryCounterInputRow {
                    country: 100_000 + index,
                    owned_state_count: 1,
                    controlled_state_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    // Slot zero belongs to the native null country, not a named live row.
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *decoded.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    let (source, content) = presentation_source(
        std::slice::from_ref(&decoded),
        Arc::new(catalogue),
        &folder.0,
    );
    let cancelled = AtomicBool::new(false);
    let recording = folder.0.join("synthetic.battleplan");
    std::fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
    let mut writer = store
        .replace(store::SourceKey::capture(&recording, &[]).unwrap())
        .unwrap();
    let mut evaluator = FocusEvaluator::default();
    evaluator.configure(&source, &content).unwrap();
    let mut accumulator = FocusAccumulator::default();
    let mut index = FocusIndex::default();
    let mut warmed_loads = None;
    for hour in [100, 101] {
        decoded.hour = hour;
        let before = evaluator.scoped_input_builds;
        evaluator
            .write_test_frame(
                &mut writer,
                &source,
                Some(&content),
                &decoded,
                &mut accumulator,
                &mut index,
                None,
                &cancelled,
            )
            .unwrap();
        assert_eq!(evaluator.scoped_input_builds - before, 1);
        let loads =
            observer_work(&source, &content, &evaluator, &index, &accumulator).content_loads;
        assert_eq!(loads, *warmed_loads.get_or_insert(loads));
        assert_eq!(index.countries.len(), 5_000);
        for (tag, state) in &index.countries {
            let state = state.last.as_ref().unwrap();
            let reason = state.presentation[0].reason.as_deref().unwrap();
            assert!(
                reason.contains(if tag == "000" {
                    "selected sprite GFX_first"
                } else {
                    "selected sprite GFX_other"
                }),
                "{tag}: {reason}"
            );
            assert_eq!(
                state.presentation[0].artwork.name.as_deref(),
                Some(format!("{tag} {tag}").as_str())
            );
            assert_eq!(
                state.details.as_ref().unwrap()[0]
                    .artwork
                    .description
                    .as_deref(),
                Some(format!("For {tag}").as_str())
            );
        }
    }
    assert_eq!(evaluator.interpreter_calls, 20_000);
    drop(writer);
    store.clear().unwrap();
}

#[test]
fn demanded_scripted_focus_failure_loads_once_and_matches_the_oracle() {
    engine::executable_tables::initialize_for_tests().unwrap();
    use crate::{
        prepared::observer_tests::unchanged_frames,
        products::{
            LoadContext, Product,
            focus_artwork::{FocusArtwork, FocusArtworkParams},
        },
    };
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/national_focus/demanded.txt",
            "focus_tree={focus={id=\"mod/current\" text=scoped_title}}",
        ),
        // Focus duration is valid; the demanded scripted reader refuses the
        // absent recursion define, using its ordinary named error.
        (
            "common/defines/demanded.lua",
            "NDefines={NFocus={FOCUS_POINT_DAYS=7}}",
        ),
        (
            "localisation/demanded_l_english.yml",
            "\u{feff}l_english:\n scoped_title:0 \"[ROOT.GetTag]\"\n scoped_title_desc:0 \"[ROOT.GetTag]\"\n",
        ),
    ]);
    let mut decoded = frame(100, 1, &[]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *decoded.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 1;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 1;
    }
    // The shared driver observes 100..=120. The recording oracle must receive
    // those real fixture hours too; a Source containing only 100 refuses 101.
    let frames: Vec<_> = (100..=120)
        .map(|hour| {
            let mut frame = decoded.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let (source, content) = presentation_source(&frames, Arc::new(catalogue), &folder.0);
    let mut evaluator = FocusEvaluator::default();
    evaluator.configure(&source, &content).unwrap();
    let mut accumulator = FocusAccumulator::default();
    let mut index = FocusIndex::default();
    let configured = observer_work(&source, &content, &evaluator, &index, &accumulator);
    assert_eq!(
        configured.content_loads, 1,
        "configuration must not demand scripted text"
    );
    let recording = folder.0.join("synthetic.battleplan");
    std::fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
    let mut writer = store
        .replace(store::SourceKey::capture(&recording, &[]).unwrap())
        .unwrap();
    let cancelled = AtomicBool::new(false);
    let context = LoadContext {
        content: Some(content.clone()),
    };
    let mut observe = |frame: &DecodedFrame, changes: &FrameChanges| {
        let (failures, error) = evaluator.write_frame(
            &mut writer,
            &source,
            Some(&content),
            frame,
            &mut accumulator,
            &mut index,
            None,
            &cancelled,
            changes,
        )?;
        assert_eq!(
            (failures, error),
            (0, None),
            "token refusals remain token refusals"
        );
        let oracle = FocusArtwork::load(
            &source,
            &FocusArtworkParams {
                branch_id: "1".into(),
                hour: frame.hour,
                tag: "000".into(),
                tokens: vec!["mod/current".into()],
                text_only: None,
                icons_only: None,
                include_descriptions: Some(false),
            },
            &context,
            &cancelled,
        )?;
        let state = index.countries["000"].last.as_ref().unwrap();
        crate::prepared::observer_tests::assert_reply_parity(&state.presentation, &oracle.items)?;
        assert!(
            state.presentation[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("MAX_SCRIPTED_LOC_RECURSION")
        );
        Ok(observer_work(
            &source,
            &content,
            &evaluator,
            &index,
            &accumulator,
        ))
    };
    let warm = observe(
        &decoded,
        &FrameChangeTracker::new().observe(&decoded, source.replay_ref().format),
    )
    .unwrap();
    assert_eq!(warm.content_loads, configured.content_loads + 1);
    let work =
        unchanged_frames(warm, &decoded, 20, source.replay_ref().format, &mut observe).unwrap();
    assert_eq!(work.content_loads, 0);
    assert_eq!(evaluator.scoped_input_builds, 21);
    drop(writer);
    store.clear().unwrap();
}

#[cfg(feature = "slow-tests")]
#[test]
fn dynamic_focus_projects_only_changed_results_across_5000_countries_and_long_orders() {
    engine::executable_tables::initialize_for_tests().unwrap();
    use crate::prepared::observer_tests::{scaled_work, unchanged_frames};
    let measured = scaled_work(2, |history| {
        let work = scaled_work(2, |hours| {
            let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
                ("common/national_focus/dynamic.txt", "focus_tree={focus={id=\"mod/a\" text=static_title} focus={id=\"mod/current\" text=scoped_title icon={GFX_on={has_global_flag=flip} GFX_off=yes}}}"),
                ("common/defines/dynamic.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}"),
                ("localisation/dynamic_l_english.yml", "\u{feff}l_english:\n static_title:0 \"Static\"\n scoped_title:0 \"[ROOT.GetTag]\"\n scoped_title_desc:0 \"For [ROOT.GetTag]\"\n"),
            ]);
            let mut decoded = frame(100, 5_000, &vec!["mod/a"; history]);
            if let TableView::Complete(TableRows::Countries(rows)) = &mut *decoded.tables[TableId::Countries.index()] {
                for row in rows { row.canonical_slot += 1; }
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *decoded.tables[TableId::CountryLinks.index()] {
                for row in rows { row.canonical_slot += 1; }
            }
            *decoded.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![replay::rows::LexicalTokenRow { token: 900, literal: b"flip".to_vec() }]));
            *decoded.tables[TableId::ScriptFlags.index()] = TableView::Complete(TableRows::ScriptFlags(vec![]));
            let (source, content) = presentation_source(std::slice::from_ref(&decoded), Arc::new(catalogue), &folder.0);
            let cancelled = AtomicBool::new(false);
            let mut evaluator = FocusEvaluator::default();
            evaluator.configure(&source, &content)?;
            let path = folder.0.join("dynamic.battleplan");
            std::fs::write(&path, b"synthetic")?;
            let key = store::SourceKey::capture(&path, &[])?;
            let store = store::Store::local(uuid::Uuid::new_v4())?;
            let mut writer = store.replace(key.clone())?;
            let mut accumulator = FocusAccumulator::default();
            let mut index = FocusIndex::default();
            evaluator.write_test_frame(&mut writer, &source, Some(&content), &decoded, &mut accumulator, &mut index, None, &cancelled)?;
            let before = observer_work(&source, &content, &evaluator, &index, &accumulator);
            let work = unchanged_frames(before, &decoded, hours as i32, source.replay_ref().format, |frame, changes| {
                evaluator.write_frame(&mut writer, &source, Some(&content), frame, &mut accumulator, &mut index, None, &cancelled, changes)?;
                Ok(observer_work(&source, &content, &evaluator, &index, &accumulator))
            })?;
            assert_eq!(work.content_loads, 0);
            assert_eq!(work.history_visits, 0, "unchanged dynamic results must not project/copy completed lists");
            assert_eq!(work.emitted, 0);
            assert_eq!(work.interpreter_calls, 10_000 * hours);
            index.flush(&mut writer, &cancelled)?;
            writer.commit()?;
            accumulator = serde_json::from_slice(&serde_json::to_vec(&accumulator)?)?;
            index = serde_json::from_slice(&serde_json::to_vec(&index)?)?;
            let mut writer = store.extend(&key)?;
            decoded.hour += hours as i32;
            let mut tracker = FrameChangeTracker::new();
            // Production invalidates the tracker on restore: the first accepted
            // frame must rebuild serde-skipped native joins, even for equal rows.
            // Priming it here incorrectly closes every resumed country instead.
            decoded.hour += 1;
            evaluator.write_frame(&mut writer, &source, Some(&content), &decoded, &mut accumulator, &mut index, None, &cancelled, &tracker.observe(&decoded, source.replay_ref().format))?;
            assert_eq!(accumulator.active.len(), 5_000, "restore must rebuild the current native joins");
            for country in index.countries.values() {
                assert!(country.last.as_ref().unwrap().presentation.last().unwrap().reason.as_deref().unwrap().contains("GFX_off"));
            }
            let before_events = observer_work(&source, &content, &evaluator, &index, &accumulator).emitted;
            decoded.hour += 1;
            *decoded.tables[TableId::ScriptFlags.index()] = TableView::Complete(TableRows::ScriptFlags(vec![replay::rows::ScriptFlagRow { scope_kind: 0, scope_id: 0, token: 900, value: 1, ..Default::default() }]));
            evaluator.write_frame(&mut writer, &source, Some(&content), &decoded, &mut accumulator, &mut index, None, &cancelled, &tracker.observe(&decoded, source.replay_ref().format))?;
            assert_eq!(observer_work(&source, &content, &evaluator, &index, &accumulator).emitted - before_events, 5_000);
            index.flush(&mut writer, &cancelled)?; writer.commit()?;
            let mut oracle = artwork::Context::load(crate::catalogue::catalogue(&source, &content)?, artwork::Kind::Focus)?;
            for tag in ["000", "001"] {
                let row = query_hour(&store, &key, 1, tag, decoded.hour, &cancelled)?.unwrap();
                let actual = presentation_page(&store, &key, 1, tag, &row.presentation, 0, &cancelled)?.unwrap();
                let expected = oracle.focus_display_at(&[("mod/a", tag), ("mod/current", tag)], &decoded, |items| items.map(|(artwork, reason)| PresentedFocus { artwork, reason }).collect());
                let expected = align_presentation(&accumulator.previous[tag].order, expected)?;
                crate::prepared::observer_tests::assert_reply_parity(&actual, &expected)?;
                assert!(actual.last().unwrap().reason.as_deref().unwrap().contains("GFX_on"));
            }
            store.clear()?;
            Ok(crate::prepared::observer_tests::Work { history_visits: work.history_visits, content_loads: work.content_loads, emitted: work.emitted, ..Default::default() })
        })?;
        assert_eq!(work[0], work[1]);
        Ok(work[1])
    }).unwrap();
    assert_eq!(measured[0], measured[1]);
}

#[test]
fn tree_overrides_skip_unreached_hourly_icons_and_reuse_exact_presentations() {
    engine::executable_tables::initialize_for_tests().unwrap();
    use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
    let image = png_bytes(&installed_content::image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![19, 0, 0, 255],
    });
    let (catalogue, folders) = texture_catalogue(
        &[
            (
                "common/national_focus/overrides.txt",
                "focus_tree={focus={id=global icon={GFX_unreached={has_country_flag=unrecorded_flag}}} focus={id=country icon={GFX_unreached={has_country_flag=unrecorded_flag}}} focus={id=bad icon={GFX_unreached={has_country_flag=unrecorded_flag}}}}",
            ),
            (
                "common/defines/overrides.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
            ),
            (
                "interface/overrides.gfx",
                "spriteTypes={spriteType={name=GFX_global texturefile=gfx/override.png} spriteType={name=GFX_country texturefile=gfx/override.png} spriteType={name=GFX_000_country texturefile=gfx/override.png} corneredTileSpriteType={name=GFX_bad texturefile=gfx/override.png size={x=y y=1}}}",
            ),
        ],
        &[("gfx/override.png", image)],
    );
    let catalogue = Arc::new(catalogue);
    assert!(catalogue.sprites.find("GFX_bad").is_err());
    let mut decoded = frame(100, 2, &["global", "country", "bad"]);
    if let TableView::Complete(TableRows::Focus(rows)) =
        &mut *decoded.tables[TableId::Focus.index()]
    {
        for row in rows {
            row.current = None;
        }
    }
    *decoded.tables[TableId::ScriptFlags.index()] = TableView::Unavailable {
        reason: "unrequested hourly flags".into(),
    };
    let (source, content) = presentation_source(
        std::slice::from_ref(&decoded),
        Arc::clone(&catalogue),
        &folders[0].0,
    );
    let recording = folders[0].0.join("synthetic.battleplan");
    std::fs::write(&recording, b"synthetic").unwrap();
    let cancelled = AtomicBool::new(false);
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let stores = [
        store::Store::local(uuid::Uuid::new_v4()).unwrap(),
        store::Store::local(uuid::Uuid::new_v4()).unwrap(),
    ];
    let mut writers = [
        stores[0].replace(key.clone()).unwrap(),
        stores[1].replace(key).unwrap(),
    ];
    let mut evaluators = [
        FocusEvaluator {
            legacy: true,
            ..Default::default()
        },
        FocusEvaluator::default(),
    ];
    let mut accumulators = [FocusAccumulator::default(), FocusAccumulator::default()];
    let mut indexes = [FocusIndex::default(), FocusIndex::default()];
    let mut oracle = artwork::Context::load(catalogue, artwork::Kind::Focus).unwrap();
    for hour in [100, 101, 102] {
        decoded.hour = hour;
        if hour == 102
            && let TableView::Complete(TableRows::Focus(rows)) =
                &mut *decoded.tables[TableId::Focus.index()]
        {
            rows[0].progress += 1;
        }
        let before = evaluators[1].interpreter_calls;
        for side in 0..2 {
            evaluators[side]
                .write_test_frame(
                    &mut writers[side],
                    &source,
                    Some(&content),
                    &decoded,
                    &mut accumulators[side],
                    &mut indexes[side],
                    None,
                    &cancelled,
                )
                .unwrap();
            indexes[side].series.seal();
        }
        assert_eq!(
            serde_json::to_value(&indexes[0]).unwrap(),
            serde_json::to_value(&indexes[1]).unwrap()
        );
        if hour > 100 {
            assert_eq!(evaluators[1].interpreter_calls, before);
        }
        for (tag, country) in &indexes[1].countries {
            assert!(evaluators[1].reusable(tag));
            assert!(!evaluators[0].reusable(tag));
            let requests = [
                ("bad", tag.as_str()),
                ("country", tag.as_str()),
                ("global", tag.as_str()),
            ];
            let expected: Vec<_> =
                oracle.focus_display_at(&requests, &decoded, |items| items.collect());
            for (art, reason) in expected {
                let stored = country
                    .last
                    .as_ref()
                    .unwrap()
                    .presentation
                    .iter()
                    .find(|row| row.artwork.token == art.token)
                    .unwrap();
                assert_eq!(stored.artwork, art);
                assert_eq!(stored.reason, reason);
            }
        }
    }
    assert_eq!(
        evaluators[1].scoped_input_builds, 0,
        "unreached triggers construct no scoped inputs"
    );
    assert_eq!(evaluators[1].counts.progress_reused, 1);
    drop(writers);
    for store in stores {
        store.clear().unwrap();
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn rare_changes_do_not_rebuild_hourly_orders_for_5000_countries() {
    let (countries, hours) = (5_000, 3_000);
    let mut decoded = frame(0, countries, &["mod/a", "mod/a", "mod/b"]);
    let mut accumulator = FocusAccumulator::default();
    let cancelled = AtomicBool::new(false);
    let mut observations = 0;
    for hour in 0..hours {
        decoded.hour = hour;
        // Countries balances change hourly, but focus reads only its keys.
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *decoded.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::from(hour));
        }
        if hour > 0
            && hour % 1_000 == 0
            && let TableView::Complete(TableRows::Focus(rows)) =
                &mut *decoded.tables[TableId::Focus.index()]
        {
            rows[0].progress += 10;
        }
        accumulator
            .observe(&decoded, &cancelled, |_| {
                observations += 1;
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(observations, countries + 2);
    assert!(
        accumulator
            .previous
            .keys()
            .all(|tag| accumulator.active.contains(tag) && accumulator.hour == Some(hours - 1))
    );
    eprintln!(
        "focus observations: before={} after={observations}",
        countries * hours as u32
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn batched_focus_files_and_interpretations_at_scale_and_resume() {
    engine::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/national_focus/scale.txt",
            "focus_tree={focus={id=\"mod/a\" text=title_a} focus={id=\"mod/b\" text=title_b} focus={id=\"mod/current\" text=title_c}}",
        ),
        (
            "common/defines/scale.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
        ),
        (
            "localisation/scale_l_english.yml",
            "\u{feff}l_english:\n title_a:0 \"A\"\n title_b:0 \"B\"\n title_c:0 \"C\"\n",
        ),
    ]);
    let cancelled = AtomicBool::new(false);
    let mut decoded = frame(0, 5_000, &["mod/a", "mod/a", "mod/b"]);
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "focus-files-scale".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        std::sync::Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded.clone()])]).unwrap(),
        ),
        &cancelled,
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    crate::catalogue::set_test_catalogue(&source, &content, std::sync::Arc::new(catalogue));
    let id = uuid::Uuid::new_v4();
    let recording = std::env::temp_dir().join(format!("focus-batch-scale-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let directory = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .join("HOI4Replay/prepared")
        .join(id.to_string());
    let files = || {
        std::fs::read_dir(&directory)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|suffix| suffix == "bin")
            })
            .count()
    };
    let mut accumulator = FocusAccumulator::default();
    let mut index = FocusIndex::default();
    let mut evaluator = FocusEvaluator::default();
    let mut observations = 0;
    let mut calls = 0;
    let mut entry_calls = 0;
    let mut writer = store.replace(key.clone()).unwrap();
    for hour in 0..3_008 {
        if hour == 3_000 {
            index.flush(&mut writer, &cancelled).unwrap();
            writer.commit().unwrap();
            assert_eq!(files(), 6); // Five focus payloads plus the completion-feed catalogue.
            assert_eq!(
                store
                    .range_count(&key, "focus-completions-1-catalogue")
                    .unwrap(),
                Some(1)
            );
            assert_eq!(evaluator.interpreted, 6);
            assert_eq!(evaluator.interpreter_calls, 2);
            assert_eq!(observations, 5_002);
            assert_eq!(
                index
                    .countries
                    .values()
                    .map(|country| index
                        .series
                        .state(country.key)
                        .map_or(0, |state| state.events))
                    .sum::<u64>(),
                5_000
            );
            eprintln!(
                "rare focus: files before=20011 after=6 (5 focus payloads + feed catalogue); interpreter calls before=30000000 after=2; tokens before=90000000 after=6"
            );
            let row = query_hour(&store, &key, 1, "000", 1_500, &cancelled)
                .unwrap()
                .unwrap();
            assert_eq!(
                (row.hour, row.last_hour, row.order_first_hour),
                (1_000, 1_999, 0)
            );
            assert_eq!(row.daily_gain_raw.as_deref(), Some("10"));
            assert_eq!(
                query_page(
                    &store,
                    &key,
                    1,
                    "000",
                    1_500,
                    &row.completed,
                    0,
                    "focus-order-page",
                    &cancelled
                )
                .unwrap(),
                serde_json::to_value(
                    order_page(&store, &key, 1, "000", &row.completed, 0, &cancelled).unwrap()
                )
                .unwrap()
            );
            assert_eq!(
                query_page(
                    &store,
                    &key,
                    1,
                    "000",
                    1_500,
                    &row.presentation,
                    0,
                    "focus-presentation-page",
                    &cancelled
                )
                .unwrap(),
                serde_json::to_value(
                    presentation_page(&store, &key, 1, "000", &row.presentation, 0, &cancelled)
                        .unwrap()
                )
                .unwrap()
            );
            for pages in [
                Pages {
                    first: row.completed.first + 1,
                    ..row.completed.clone()
                },
                Pages {
                    rows: row.completed.rows + 1,
                    ..row.completed.clone()
                },
            ] {
                assert!(
                    query_page(
                        &store,
                        &key,
                        1,
                        "000",
                        1_500,
                        &pages,
                        0,
                        "focus-order-page",
                        &cancelled
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("page changed")
                );
            }
            assert_eq!(
                order_page(&store, &key, 1, "000", &row.completed, 0, &cancelled)
                    .unwrap()
                    .unwrap()
                    .len(),
                3
            );
            assert_eq!(
                presentation_page(&store, &key, 1, "000", &row.presentation, 0, &cancelled)
                    .unwrap()
                    .unwrap()
                    .len(),
                4
            );
            // Compare Display and Text with the production boundary at an
            // actually observed hour, including the duplicated completion.
            let mut oracle = artwork::Context::load(
                crate::catalogue::catalogue(&source, &content).unwrap(),
                artwork::Kind::Focus,
            )
            .unwrap();
            let order = &accumulator.previous["000"].order;
            let tokens: Vec<_> = order
                .completed
                .iter()
                .map(|entry| entry.token.clone())
                .chain(order.current.iter().cloned())
                .collect();
            let dated = query_hour(&store, &key, 1, "000", decoded.hour, &cancelled)
                .unwrap()
                .unwrap();
            for mode in [Presentation::Display, Presentation::Text] {
                let expected = crate::products::focus_artwork::present_decoded(
                    &mut oracle,
                    &crate::products::focus_artwork::FocusArtworkParams {
                        branch_id: "1".into(),
                        hour: decoded.hour,
                        tag: "000".into(),
                        tokens: tokens.clone(),
                        text_only: Some(mode == Presentation::Text),
                        icons_only: None,
                        include_descriptions: Some(mode == Presentation::Text),
                    },
                    &decoded,
                    &cancelled,
                )
                .unwrap();
                let expected = align_presentation(
                    order,
                    serde_json::from_value(serde_json::to_value(expected.items).unwrap()).unwrap(),
                )
                .unwrap();
                let actual = if mode == Presentation::Display {
                    presentation_page(&store, &key, 1, "000", &dated.presentation, 0, &cancelled)
                } else {
                    details_page(
                        &store,
                        &key,
                        1,
                        "000",
                        dated.details.as_ref().unwrap(),
                        0,
                        &cancelled,
                    )
                }
                .unwrap()
                .unwrap();
                crate::prepared::observer_tests::assert_reply_parity(&actual, &expected).unwrap();
            }
            accumulator =
                serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
            index = serde_json::from_slice(&serde_json::to_vec(&index).unwrap()).unwrap();
            calls += evaluator.interpreted;
            entry_calls += evaluator.interpreter_calls;
            evaluator = FocusEvaluator::default();
            writer = store.extend(&key).unwrap();
        }
        decoded.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *decoded.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::from(hour));
        }
        if let TableView::Complete(TableRows::Focus(rows)) =
            &mut *decoded.tables[TableId::Focus.index()]
        {
            if hour >= 3_000 {
                for row in rows {
                    row.progress += 10;
                }
            } else if hour > 0 && hour % 1_000 == 0 {
                rows[0].progress += 10;
            }
        }
        let mut changed = Vec::new();
        index
            .write_unavailable(&mut writer, &decoded, &cancelled)
            .unwrap();
        let changes = accumulator.frame_changes(&decoded);
        accumulator
            .observe_changes(&decoded, &changes, &cancelled, |tag, row| {
                observations += 1;
                changed.push((tag.to_owned(), row));
                Ok(())
            })
            .unwrap();
        let before = evaluator.interpreted;
        let before_files = files();
        for (tag, observation) in changed {
            if observation.is_none()
                && evaluator.reusable(&tag)
                && index
                    .write_progress(
                        1,
                        &tag,
                        hour,
                        &accumulator.previous[&tag],
                        Some(evaluator.warnings()),
                        &cancelled,
                    )
                    .unwrap()
            {
                continue;
            }
            let observation = observation.unwrap_or_else(|| accumulator.observation(&tag, hour));
            if evaluator.reusable(&tag)
                && index
                    .write_progress(
                        1,
                        &tag,
                        hour,
                        &accumulator.previous[&tag],
                        Some(evaluator.warnings()),
                        &cancelled,
                    )
                    .unwrap()
            {
                continue;
            }
            let tokens: Vec<_> = observation
                .order
                .completed
                .iter()
                .map(|entry| entry.token.clone())
                .chain(observation.order.current.iter().cloned())
                .collect();
            evaluator
                .prepare_static_text(&mut writer, &source, &content, &tokens, &cancelled)
                .unwrap();
            let mut projections = Vec::new();
            for mode in [Presentation::Display, Presentation::Text] {
                projections.push(
                    evaluator
                        .present(
                            &source,
                            &content,
                            &decoded,
                            &tag,
                            &tokens,
                            mode,
                            &cancelled,
                            |items, _| Ok(items.collect::<Vec<_>>()),
                        )
                        .unwrap(),
                );
            }
            let mut projections = projections.into_iter();
            index
                .write_hour_with_details(
                    &mut writer,
                    1,
                    observation,
                    &mut projections.next().unwrap().into_iter(),
                    Some(&mut projections.next().unwrap().into_iter()),
                    evaluator.warnings().to_vec(),
                    &cancelled,
                )
                .unwrap();
        }
        index.finish_frame(&accumulator, hour, &cancelled).unwrap();
        if (1..3_000).contains(&hour) {
            assert_eq!(evaluator.interpreted, before);
            assert_eq!(
                files(),
                before_files,
                "unchanged hours and two progress changes write no files"
            );
        }
    }
    index.flush(&mut writer, &cancelled).unwrap();
    writer.commit().unwrap();
    calls += evaluator.interpreted;
    entry_calls += evaluator.interpreter_calls;
    assert_eq!((observations, calls), (45_002, 12));
    assert_eq!(entry_calls, 4);
    assert_eq!(
        index
            .countries
            .values()
            .map(|country| index
                .series
                .state(country.key)
                .map_or(0, |state| state.events))
            .sum::<u64>(),
        5_000,
        "progress-only hours never emit focus state events"
    );
    assert_eq!(files(), 9); // Includes both replaced country and completion-feed catalogues.
    let active = store.range_count(&key, "focus-1").unwrap().unwrap()
        + store
            .range_count(&key, "focus-1-catalogue")
            .unwrap()
            .unwrap()
        + store
            .range_count(&key, "focus-completions-1-catalogue")
            .unwrap()
            .unwrap()
        + 3;
    assert_eq!(active, 7);
    eprintln!(
        "focus with dense progress and resume: files before=180011 after=9 (7 active); interpreter calls before=30080000 after={entry_calls}; tokens before=90240000 after={calls}"
    );
    let row = query_hour(&store, &key, 1, "001", 3_007, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.hour, row.last_hour, row.order_first_hour),
        (3_007, 3_007, 0)
    );
    assert_eq!(row.daily_gain_raw.as_deref(), Some("10"));
    assert_eq!(
        query_hour(&store, &key, 1, "000", 999, &cancelled)
            .unwrap()
            .unwrap()
            .last_hour,
        999
    );
    assert_eq!(
        static_text_page(&store, &key, &["mod/a".into()], &cancelled).unwrap()[0]
            .name
            .as_deref(),
        Some("A")
    );
    store.clear().unwrap();
    std::fs::remove_file(recording).unwrap();
}

#[test]
fn sequential_observations_match_history_at_scale_and_resume() {
    let count = 5_000;
    let frames = vec![
        frame(100, count, &["mod/a", "mod/a"]),
        frame(101, count, &["mod/a", "mod/a", "mod/b"]),
        frame(105, count, &["mod/a", "mod/a", "mod/b", "mod/c"]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let cancelled = AtomicBool::new(false);
    let tag = String::from_utf8(scale_tag(count / 2)).unwrap();
    let oracle = crate::focus_history(&replay, 1, &tag, &cancelled).unwrap();
    let mut accumulator = FocusAccumulator::default();
    for frame in &frames {
        let mut observed = 0;
        accumulator
            .observe(frame, &cancelled, |row| {
                observed += 1;
                if row.tag == tag {
                    assert_eq!(row.order, oracle.at(frame.hour).unwrap());
                    assert_eq!(row.progress_raw, i64::MIN.to_string());
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(observed, count);
        // A newly opened process resumes only the latest lists, never historical DTOs.
        accumulator = serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
        assert_eq!(accumulator.previous.len(), count as usize);
    }
}

#[test]
fn missing_reads_and_identity_replacement_preserve_oracle() {
    let first = frame(100, 1, &["a"]);
    let mut missing = frame(101, 1, &["a", "b"]);
    *missing.tables[TableId::Focus.index()] = TableView::Unavailable {
        reason: "synthetic unavailable".into(),
    };
    let after = frame(102, 1, &["a", "b"]);
    let mut replacement = frame(103, 1, &["a", "b", "c"]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replacement.tables[TableId::Countries.index()]
    {
        rows[0].id = 200_000;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *replacement.tables[TableId::CountryLinks.index()]
    {
        rows.push(replay::rows::CountryLinkRow {
            id: 200_000,
            canonical_slot: 0,
        });
    }
    let frames = vec![first, missing, after, replacement];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let cancelled = AtomicBool::new(false);
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let oracle = crate::focus_history(&replay, 1, &tag, &cancelled).unwrap();
    let mut accumulator = FocusAccumulator::default();
    for frame in &frames {
        let mut rows = Vec::new();
        accumulator
            .observe(frame, &cancelled, |row| {
                rows.push(row);
                Ok(())
            })
            .unwrap();
        match oracle.at(frame.hour) {
            Ok(order) => assert_eq!(rows[0].order, order),
            Err(_) => assert!(rows.is_empty()),
        }
    }
}

#[test]
fn cancelled_observation_does_not_advance_resumption() {
    let mut accumulator = FocusAccumulator::default();
    let error = accumulator
        .observe(&frame(1, 1, &[]), &AtomicBool::new(true), |_| {
            panic!("cancelled")
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(accumulator.hour.is_none());
}

#[test]
fn interval_pages_keep_all_progress_and_leave_gaps_missing() {
    let id = uuid::Uuid::new_v4();
    let recording = std::env::temp_dir().join(format!("prepared-focus-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut writer = store.replace(key.clone()).unwrap();
    let mut index = FocusIndex::default();
    let mut accumulator = FocusAccumulator::default();
    let tokens: Vec<_> = (0..300).map(|i| format!("mod/{i}")).collect();
    let borrowed: Vec<_> = tokens.iter().map(String::as_str).collect();
    for hour in [100, 101, 105, 106] {
        let mut current = frame(hour, 1, &borrowed);
        if let TableView::Complete(TableRows::Focus(rows)) =
            &mut *current.tables[TableId::Focus.index()]
        {
            rows[0].progress = if hour < 106 { i64::MAX } else { i64::MIN };
        }
        accumulator
            .observe(&current, &cancelled, |observation| {
                index.write_hour(
                    &mut writer,
                    1,
                    observation,
                    &mut std::iter::empty(),
                    Vec::new(),
                    &cancelled,
                )
            })
            .unwrap();
        assert!(
            index.extend_hour(1, &String::from_utf8(scale_tag(0)).unwrap(), hour, &[])
                || hour != 101
        );
    }
    index.flush(&mut writer, &cancelled).unwrap();
    let state = serde_json::to_vec(&index).unwrap();
    writer.commit().unwrap();
    let index: FocusIndex = serde_json::from_slice(&state).unwrap();
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    assert_eq!(
        crate::prepared::series::Reader::new(&store, &key, "focus-1".into(), &cancelled)
            .unwrap()
            .runs(index.countries[&tag].key, 100, 106)
            .unwrap()
            .len(),
        3
    );
    assert!(
        index
            .hour(&store, &key, 1, &tag, 102, &cancelled)
            .unwrap()
            .is_none()
    );
    let row = index
        .hour(&store, &key, 1, &tag, 101, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(row.progress_raw, i64::MAX.to_string());
    assert_eq!(row.completed.rows, 300);
    let all: Vec<_> = (0..row.completed.count)
        .flat_map(|page| {
            let expected = order_page(&store, &key, 1, &tag, &row.completed, page, &cancelled)
                .unwrap()
                .unwrap();
            assert_eq!(
                query_page(
                    &store,
                    &key,
                    1,
                    &tag,
                    101,
                    &row.completed,
                    page,
                    "focus-order-page",
                    &cancelled
                )
                .unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            expected
        })
        .collect();
    assert_eq!(all.len(), 300);
    assert_eq!(all[299].token, "mod/299");
    assert!(
        query_page(
            &store,
            &key,
            1,
            &tag,
            102,
            &row.completed,
            0,
            "focus-order-page",
            &cancelled
        )
        .unwrap_err()
        .to_string()
        .contains("unavailable at this hour")
    );
    assert!(
        order_page(
            &store,
            &key,
            1,
            &tag,
            &row.completed,
            row.completed.count,
            &cancelled
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        index
            .hour(&store, &key, 1, &tag, 106, &cancelled)
            .unwrap()
            .unwrap()
            .progress_raw,
        i64::MIN.to_string()
    );
    let mut resumed: FocusAccumulator =
        serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
    let mut index = index;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&recording)
        .unwrap()
        .write_all(b"appended tail")
        .unwrap();
    let appended_key = store::SourceKey::capture(&recording, &[]).unwrap();
    let mut writer = store.append(&key, appended_key.clone()).unwrap();
    resumed
        .observe(&frame(107, 1, &borrowed), &cancelled, |row| {
            index.write_hour(
                &mut writer,
                1,
                row,
                &mut std::iter::empty(),
                Vec::new(),
                &cancelled,
            )
        })
        .unwrap();
    index.flush(&mut writer, &cancelled).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        crate::prepared::series::Reader::new(&store, &appended_key, "focus-1".into(), &cancelled)
            .unwrap()
            .runs(index.countries[&tag].key, 100, 107)
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        index
            .hour(&store, &appended_key, 1, &tag, 107, &cancelled)
            .unwrap()
            .unwrap()
            .progress_raw,
        i64::MIN.to_string()
    );
    assert!(
        index
            .hour(&store, &appended_key, 1, &tag, 103, &cancelled)
            .unwrap()
            .is_none()
    );
    store.clear().unwrap();
    std::fs::remove_file(recording).unwrap();
}

#[test]
fn prepared_titles_icons_and_refusals_match_product_modes() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{
        LoadContext, Product,
        focus_artwork::{FocusArtwork, FocusArtworkParams},
    };
    use std::sync::Arc;
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/national_focus/synthetic.txt",
            "focus_tree={focus={id=mod_a text=title_a icon=GFX_missing} focus={id=mod_b text=title_b icon=GFX_missing}}",
        ),
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
        ),
        (
            "interface/synthetic.gfx",
            "spriteTypes={spriteType={name=GFX_000_mod_a}}",
        ),
        (
            "localisation/synthetic_l_english.yml",
            "\u{feff}l_english:\n title_a:0 \"A title\"\n title_b:0 \"B title\"\n",
        ),
    ]);
    let decoded = frame(100, 1, &[]);
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded.clone()])]).unwrap();
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "prepared-focus-oracle".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    crate::catalogue::set_test_catalogue(&source, &content, Arc::new(catalogue));
    let context = LoadContext {
        content: Some(content.clone()),
    };
    let cancelled = AtomicBool::new(false);
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let mut evaluator = FocusEvaluator::default();
    evaluator.configure(&source, &content).unwrap();
    let content_loads = crate::catalogue::catalogue_loads(&source, &content)
        + artwork::Context::content_loads(evaluator.catalogue.as_ref().unwrap());
    for (mode, text_only, icons_only, include_descriptions) in [
        (Presentation::Display, None, None, Some(false)),
        (Presentation::Titles, Some(true), None, Some(false)),
        (Presentation::Icons, None, Some(true), Some(false)),
        (Presentation::Text, Some(true), None, Some(true)),
        (Presentation::Full, None, None, Some(true)),
    ] {
        let mut params = FocusArtworkParams {
            branch_id: "1".into(),
            hour: 100,
            tag: tag.clone(),
            tokens: vec![
                "mod_b".into(),
                "mod_a".into(),
                "mod_b".into(),
                "arbitrary/mod/token".into(),
            ],
            text_only,
            icons_only,
            include_descriptions,
        };
        let oracle = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
        let (rows, warnings) = evaluator
            .present(
                &source,
                &content,
                &decoded,
                &tag,
                &params.tokens,
                mode,
                &cancelled,
                |items, warnings| {
                    Ok((
                        items
                            .map(|(artwork, reason)| PresentedFocus { artwork, reason })
                            .collect::<Vec<_>>(),
                        warnings,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            serde_json::to_value(&oracle.items).unwrap()
        );
        assert_eq!(warnings, oracle.warnings);
        assert_eq!(
            crate::catalogue::catalogue_loads(&source, &content)
                + artwork::Context::content_loads(evaluator.catalogue.as_ref().unwrap()),
            content_loads,
            "prepared and oracle presentation must reuse configured databases"
        );
        let interpreted = evaluator.interpreted;
        for tag in [&tag, "another country"] {
            params.tag = tag.to_owned();
            let oracle = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
            let cached = evaluator
                .present(
                    &source,
                    &content,
                    &decoded,
                    tag,
                    &params.tokens,
                    mode,
                    &cancelled,
                    |items, warnings| {
                        Ok((
                            items
                                .map(|(artwork, reason)| PresentedFocus { artwork, reason })
                                .collect::<Vec<_>>(),
                            warnings,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(
                serde_json::to_value(cached.0).unwrap(),
                serde_json::to_value(&oracle.items).unwrap()
            );
            assert_eq!(cached.1, warnings);
        }
        let scoped = usize::from(matches!(
            mode,
            Presentation::Display | Presentation::Icons | Presentation::Full
        ));
        assert_eq!(
            evaluator.interpreted,
            interpreted + scoped,
            "only country-specific sprite overrides split static results"
        );
    }
    assert_eq!(evaluator.interpreted, 18);
}

#[test]
fn preparation_retains_progress_when_old_retention_budget_refuses_it() {
    let decoded = frame(100, 5_000, &[]);
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded.clone()])]).unwrap();
    let cancelled = AtomicBool::new(false);
    let old = crate::products::national_focus::focus_branch_within(&replay, 1, None, &cancelled, 0)
        .unwrap();
    assert_eq!(old.progress_stopped, Some(100));
    assert!(
        old.countries
            .values()
            .all(|history| history.progress.is_empty())
    );
    let mut prepared = FocusAccumulator::default();
    let mut count = 0;
    prepared
        .observe(&decoded, &cancelled, |row| {
            assert_eq!(row.progress_raw, i64::MIN.to_string());
            count += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 5_000);
}

#[test]
fn long_opaque_tags_and_completed_tokens_are_not_dropped() {
    let tag = "mod/tag/".repeat(200);
    let token = "mod/focus/".repeat(200);
    let mut decoded = frame(100, 1, &[&token]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *decoded.tables[TableId::Countries.index()]
    {
        rows[0].tag = tag.as_bytes().to_vec();
    }
    let cancelled = AtomicBool::new(false);
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded.clone()])]).unwrap();
    assert!(
        crate::products::national_focus::focus_branch(&replay, 1, None, &cancelled)
            .unwrap()
            .countries
            .is_empty()
    );
    let mut accumulator = FocusAccumulator::default();
    let mut observed = Vec::new();
    accumulator
        .observe(&decoded, &cancelled, |row| {
            observed.push(row);
            Ok(())
        })
        .unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].tag, tag);
    assert_eq!(observed[0].order.completed[0].token, token);
}

#[test]
fn only_changed_prepared_focus_schema_is_refused() {
    let accumulator = FocusAccumulator::default();
    let index = FocusIndex::default();
    for mut state in [
        serde_json::to_value(accumulator).unwrap(),
        serde_json::to_value(index).unwrap(),
    ] {
        assert_eq!(state["schema"], STATE_SCHEMA);
        state["schema"] = serde_json::json!(STATE_SCHEMA + 1);
        assert!(serde_json::from_value::<FocusAccumulator>(state.clone()).is_err());
        assert!(serde_json::from_value::<FocusIndex>(state).is_err());
    }
    // Refusing derivative state changes no recording table or recording-only access.
    let decoded = frame(100, 1, &[]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded])]).unwrap();
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    assert!(crate::focus_history(&replay, 1, &tag, &AtomicBool::new(false)).is_ok());
}

#[test]
fn daily_gain_matches_progress_runs_without_crossing_focus_changes_or_gaps() {
    let mut accumulator = FocusAccumulator::default();
    let cancelled = AtomicBool::new(false);
    for (hour, raw, current, expected) in [
        (100, 10, "a", None),
        (101, 20, "a", Some("10")),
        (102, 20, "a", Some("10")),
        (103, 20, "b", None),
        (105, 20, "b", None),
        (106, 30, "b", Some("10")),
    ] {
        let mut decoded = frame(hour, 1, &[]);
        if let TableView::Complete(TableRows::Focus(rows)) =
            &mut *decoded.tables[TableId::Focus.index()]
        {
            rows[0].progress = raw;
            rows[0].current = Some(current.as_bytes().to_vec());
        }
        let mut observed = None;
        accumulator
            .observe(&decoded, &cancelled, |row| {
                observed = Some(row);
                Ok(())
            })
            .unwrap();
        let row = observed.unwrap_or_else(|| {
            accumulator.observation(&String::from_utf8(scale_tag(0)).unwrap(), hour)
        });
        assert_eq!(row.daily_gain_raw.as_deref(), expected);
    }
}

#[test]
fn stored_presentation_follows_repeated_ordinals_and_active_tail() {
    let id = uuid::Uuid::new_v4();
    let recording = std::env::temp_dir().join(format!("prepared-focus-order-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut writer = store.replace(key.clone()).unwrap();
    let mut index = FocusIndex::default();
    FocusAccumulator::default()
        .observe(&frame(100, 1, &["z", "a", "z"]), &cancelled, |row| {
            let mut evaluated = ["a", "mod/current", "z"].into_iter().map(|token| {
                (
                    Artwork {
                        token: token.into(),
                        name: Some(token.into()),
                        description: None,
                        base_duration_days: None,
                        icon: None,
                    },
                    None,
                )
            });
            index.write_hour(&mut writer, 1, row, &mut evaluated, Vec::new(), &cancelled)
        })
        .unwrap();
    index.flush(&mut writer, &cancelled).unwrap();
    writer.commit().unwrap();
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let hour = query_hour(&store, &key, 1, &tag, 100, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(hour.order_first_hour, 100);
    assert_eq!(hour.presentation.rows, 4);
    let items = presentation_page(&store, &key, 1, &tag, &hour.presentation, 0, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.artwork.token.as_str())
            .collect::<Vec<_>>(),
        ["z", "a", "z", "mod/current"]
    );
    store.clear().unwrap();
    std::fs::remove_file(recording).unwrap();
}

#[test]
fn stored_full_order_matches_old_visible_subsets_with_scopes_random_and_effect_sessions() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
    use crate::products::{
        LoadContext, Product,
        focus_artwork::{FocusArtwork, FocusArtworkParams},
    };
    use installed_content::image::Rgba8Image;
    use std::sync::Arc;
    let definitions = format!("focus_tree={{{} focus={{id=a_static text=static_title icon=GFX_mod_true}}}}", (0..12).map(|index| format!(
        "focus={{id=t{index:02} text=title{} icon={{GFX_mod_true={{tag=GER}} GFX_mod_other=yes}}}}", index % 3)).collect::<Vec<_>>().join(" "));
    let image = |red| {
        png_bytes(&Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![red, 0, 0, 255],
        })
    };
    let (catalogue, folders) = texture_catalogue(
        &[
            ("common/national_focus/subsets.txt", &definitions),
            (
                "interface/subsets.gfx",
                "spriteTypes={spriteType={name=GFX_mod_true textureFile=gfx/mod/true.png} spriteType={name=GFX_mod_other textureFile=gfx/mod/other.png}}",
            ),
            (
                "common/defines/subsets.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
            ),
            (
                "common/scripted_localisation/subsets.txt",
                "entry={name=Choice text={random_list={1={localization_key=A} 1={localization_key=B}}}}",
            ),
            (
                "common/bop/subsets.txt",
                "balance={range={id=active on_activate={set_temp_variable={x=7} if={limit={check_variable={temp_var:x=7}} custom_effect_tooltip=SESSION_SEVEN} else={custom_effect_tooltip=SESSION_FRESH}}}}",
            ),
            (
                "localisation/subsets_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " title0:0 \"[ROOT.GetTag] [FROM.GetTag]\"\n title0_desc:0 \"[ROOT.GetTag] description\"\n",
                    " title1:0 \"[Choice] [ROOT.Choice]\"\n title1_desc:0 \"[Choice] description\"\n",
                    " title2:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n title2_desc:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n",
                    " A:0 \"A\"\n B:0 \"B\"\n SESSION_SEVEN:0 \"Session seven\"\n SESSION_FRESH:0 \"Fresh session\"\n",
                    " static_title:0 \"Static title\"\n static_title_desc:0 \"Static details\"\n",
                    " PROGRESS_SECTION_ACTIVATION_EFFECT:0 \"Activate:\\n$DESC$\"\n"
                ),
            ),
        ],
        &[
            ("gfx/mod/true.png", image(17)),
            ("gfx/mod/other.png", image(34)),
        ],
    );
    let mut decoded = crate::test_fixtures::source(1, 100, Some(1));
    for table in TableId::ALL {
        // Unused singleton inputs keep their named unavailable state. Complete
        // singleton tables cannot describe an empty set of rows.
        if matches!(
            table,
            TableId::Clock
                | TableId::Campaign
                | TableId::EventFeedHistory
                | TableId::AirSortieModifierMetadata
                | TableId::CombatLogState
                | TableId::ReservedCountryNaming
                | TableId::LocalPlayer
                | TableId::SimulationFeatures
                | TableId::SunkShips
                | TableId::WorldTension
                | TableId::GameScoreMultipliers
                | TableId::MapEvents
        ) {
            continue;
        }
        if matches!(
            &*decoded.tables[table.index()],
            TableView::Unavailable { .. }
        ) {
            *decoded.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    let tokens: Vec<String> = (0..12)
        .map(|index| format!("t{index:02}"))
        .chain(std::iter::once("t00".into()))
        .chain(std::iter::once("a_static".into()))
        .collect();
    *decoded.tables[TableId::Focus.index()] =
        TableView::Complete(TableRows::Focus(vec![FocusRow {
            country: 9001,
            completed_count: tokens.len() as i32,
            current: Some(b"t05".to_vec()),
            ..Default::default()
        }]));
    *decoded.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(
            tokens
                .iter()
                .enumerate()
                .map(|(ordinal, token)| CompletedFocusRow {
                    country: 9001,
                    ordinal: ordinal as u32,
                    token: token.as_bytes().to_vec(),
                })
                .collect(),
        ));
    *decoded.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![replay::rows::ClockRow {
            hour: decoded.hour,
            seed: 987,
            ..Default::default()
        }]));
    *decoded.tables[TableId::PowerBalances.index()] =
        TableView::Complete(TableRows::PowerBalances(vec![
            replay::rows::PowerBalanceRow {
                balance: b"balance".to_vec(),
                active_range: b"active".to_vec(),
                ..Default::default()
            },
        ]));
    *decoded.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::PowerBalanceMembers(vec![
            replay::rows::PowerBalanceMemberRow {
                country: 9001,
                ..Default::default()
            },
        ]));
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![decoded.clone()])]).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "subset-oracle".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancelled,
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folders[0].0.clone(),
            user_dir: folders[0].0.join("user"),
            cache_dir: folders[0].0.join("cache"),
        },
        allow_unverified: true,
    };
    let catalogue = Arc::new(catalogue);
    crate::catalogue::set_test_catalogue(&source, &content, Arc::clone(&catalogue));
    let context = LoadContext {
        content: Some(content.clone()),
    };
    let id = uuid::Uuid::new_v4();
    let recording = std::env::temp_dir().join(format!("prepared-focus-subsets-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    let mut index = FocusIndex::default();
    let mut evaluator = FocusEvaluator::default();
    let mut observation = None;
    FocusAccumulator::default()
        .observe(&decoded, &cancelled, |row| {
            observation = Some(row);
            Ok(())
        })
        .unwrap();
    let inputs = artwork::FocusFrame::new(&decoded, &catalogue);
    let text = evaluator
        .present_in(
            &source,
            &content,
            &inputs,
            "GER",
            &tokens,
            Presentation::Text,
            &cancelled,
            |items, _| Ok(items.collect::<Vec<_>>()),
        )
        .unwrap();
    evaluator
        .present_in(
            &source,
            &content,
            &inputs,
            "GER",
            &tokens,
            Presentation::Display,
            &cancelled,
            |items, warnings| {
                index.write_hour_with_details(
                    &mut writer,
                    1,
                    observation.take().unwrap(),
                    items,
                    Some(&mut text.into_iter()),
                    warnings,
                    &cancelled,
                )
            },
        )
        .unwrap();
    assert_eq!(inputs.text_builds.get(), 1);
    drop(inputs);
    index.flush(&mut writer, &cancelled).unwrap();
    writer.commit().unwrap();
    let hour = query_hour(&store, &key, 1, "GER", 100, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!((hour.slot, hour.country_id), (7, 9001));
    let display = presentation_page(&store, &key, 1, "GER", &hour.presentation, 0, &cancelled)
        .unwrap()
        .unwrap();
    let details = details_page(
        &store,
        &key,
        1,
        "GER",
        hour.details.as_ref().unwrap(),
        0,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    assert_eq!(display[0].artwork.name.as_deref(), Some("GER GER"));
    assert!(display[1].artwork.name.is_some());
    assert!(
        display[2]
            .artwork
            .name
            .as_deref()
            .unwrap()
            .contains("Session seven")
    );
    let full_display = FocusArtwork::load(
        &source,
        &FocusArtworkParams {
            branch_id: "1".into(),
            hour: 100,
            tag: "GER".into(),
            tokens: tokens.clone(),
            text_only: None,
            icons_only: None,
            include_descriptions: Some(false),
        },
        &context,
        &cancelled,
    )
    .unwrap();
    for item in &display {
        let old = full_display
            .items
            .iter()
            .find(|old| old.artwork.token == item.artwork.token)
            .unwrap();
        assert_eq!(
            item.artwork.icon, old.artwork.icon,
            "display icon for {}",
            item.artwork.token
        );
    }
    for count in [1, 2, 7, 8, 9, 12] {
        let subset: Vec<_> = tokens[..count]
            .iter()
            .rev()
            .cloned()
            .chain(std::iter::once(tokens[0].clone()))
            .collect();
        for (text_only, stored) in [(None, &display), (Some(true), &details)] {
            let params = FocusArtworkParams {
                branch_id: "1".into(),
                hour: 100,
                tag: "GER".into(),
                tokens: subset.clone(),
                text_only,
                include_descriptions: Some(text_only == Some(true)),
                icons_only: None,
            };
            let old = FocusArtwork::load(&source, &params, &context, &cancelled).unwrap();
            for item in old.items {
                let stored = stored
                    .iter()
                    .find(|row| row.artwork.token == item.artwork.token)
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(stored).unwrap(),
                    serde_json::to_value(item).unwrap(),
                    "subset count {count}"
                );
            }
        }
    }
    let mut oracle = artwork::Context::load(
        crate::catalogue::catalogue(&source, &content).unwrap(),
        artwork::Kind::Focus,
    )
    .unwrap();
    let sorted: BTreeSet<_> = tokens.iter().map(String::as_str).collect();
    let requests: Vec<_> = sorted.into_iter().map(|token| (token, "GER")).collect();
    for hour in [101, 102] {
        decoded.hour = hour;
        if let TableView::Complete(TableRows::Clock(rows)) =
            &mut *decoded.tables[TableId::Clock.index()]
        {
            rows[0].hour = hour;
            rows[0].seed = hour;
        }
        let before = evaluator.interpreted;
        let inputs = artwork::FocusFrame::new(&decoded, &catalogue);
        for mode in [Presentation::Display, Presentation::Text] {
            let expected: Vec<_> = match mode {
                Presentation::Display => {
                    oracle.focus_display_at(&requests, &decoded, |items| items.collect())
                }
                _ => oracle.focus_text_at(&requests, &decoded, |items| items.collect()),
            };
            let actual = evaluator
                .present_in(
                    &source,
                    &content,
                    &inputs,
                    "GER",
                    &tokens,
                    mode,
                    &cancelled,
                    |items, _| Ok(items.collect::<Vec<_>>()),
                )
                .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        assert_eq!(inputs.text_builds.get(), 1);
        assert_eq!(
            evaluator.interpreted - before,
            24,
            "only twelve dynamic tokens per mode"
        );
        assert!(!evaluator.reusable("GER"));
    }
    store.clear().unwrap();
    std::fs::remove_file(recording).unwrap();
}

#[test]
fn unavailable_table_intervals_keep_named_reasons_without_refusing_numeric_recovery() {
    let id = uuid::Uuid::new_v4();
    let recording =
        std::env::temp_dir().join(format!("prepared-focus-unavailable-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut writer = store.replace(key.clone()).unwrap();
    let mut index = FocusIndex::default();
    let mut accumulator = FocusAccumulator::default();
    for hour in [100, 101, 102, 105] {
        let mut decoded = frame(hour, 1, &[]);
        if hour != 102 {
            *decoded.tables[TableId::Focus.index()] = TableView::Unavailable {
                reason: "exact synthetic supplying-read reason".into(),
            };
        }
        if hour == 105 {
            *decoded.tables[TableId::CompletedFocuses.index()] = TableView::Truncated {
                rows: TableRows::CompletedFocuses(Vec::new()),
                missing: u32::MAX,
            };
        }
        index
            .write_unavailable(&mut writer, &decoded, &cancelled)
            .unwrap();
        accumulator
            .observe(&decoded, &cancelled, |row| {
                index.write_hour(
                    &mut writer,
                    1,
                    row,
                    &mut std::iter::empty(),
                    Vec::new(),
                    &cancelled,
                )
            })
            .unwrap();
    }
    index.flush(&mut writer, &cancelled).unwrap();
    writer.commit().unwrap();
    let missing = query_unavailable(&store, &key, 1, 101, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!((missing.first_hour, missing.last_hour), (100, 101));
    assert_eq!(
        missing.reasons,
        [TableRefusal {
            table: "Focus".into(),
            reason: Some("exact synthetic supplying-read reason".into()),
            missing_rows: None
        }]
    );
    assert!(
        query_unavailable(&store, &key, 1, 102, &cancelled)
            .unwrap()
            .is_none()
    );
    assert!(
        query_unavailable(&store, &key, 1, 104, &cancelled)
            .unwrap()
            .is_none()
    );
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    assert!(
        query_hour(&store, &key, 1, &tag, 102, &cancelled)
            .unwrap()
            .is_some()
    );
    let later = query_unavailable(&store, &key, 1, 105, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(later.reasons[1].missing_rows.as_deref(), Some("4294967295"));
    store.clear().unwrap();
    std::fs::remove_file(recording).unwrap();
}

#[test]
fn static_text_is_requested_by_token_without_catalogue_or_history_scan() {
    let id = uuid::Uuid::new_v4();
    let recording = std::env::temp_dir().join(format!("prepared-focus-text-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut writer = store.replace(key.clone()).unwrap();
    let tokens = ["opaque/long/".repeat(100), "mod/a".into(), "mod/b".into()];
    for token in &tokens {
        write_static_text(
            &mut writer,
            &Artwork {
                token: token.clone(),
                name: Some(String::new()),
                description: Some(format!("details {token}")),
                base_duration_days: Some(7.5),
                icon: None,
            },
            &cancelled,
        )
        .unwrap();
    }
    writer.commit().unwrap();
    let items = static_text_page(
        &store,
        &key,
        &[tokens[2].clone(), tokens[0].clone(), tokens[2].clone()],
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        items.iter().map(|item| &item.token).collect::<Vec<_>>(),
        [&tokens[2], &tokens[0], &tokens[2]]
    );
    assert_eq!(items[0].name.as_deref(), Some(""));
    assert_eq!(items[0].base_duration_days, Some(7.5));
    assert_eq!(
        items[1].description.as_deref(),
        Some(format!("details {}", tokens[0]).as_str())
    );
    assert!(static_text_page(&store, &key, &["unprepared".into()], &cancelled).is_err());
    store.clear().unwrap();
    std::fs::remove_file(recording).unwrap();
}
