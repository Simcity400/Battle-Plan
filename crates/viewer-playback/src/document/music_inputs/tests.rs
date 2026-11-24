use super::*;
use dashboard_text::triggers::ContentFacts;
use content_reader_key_names::tag_aliases::TagAliases;
use content_reader_script::trigger_keys::TriggerNames;
use content_reader_sound::music::{Music, Song, Track};
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableView};
use std::collections::BTreeSet;

fn enter_music_tables() {
    use engine_executable::executable_tables::{self, Tables, TriggerReader};

    static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        content_reader_script::test_game::executable()
            .trigger("check_variable", 0, TriggerReader::Body, 0)
            .build()
    });
    executable_tables::enter(Some(tables));
}

fn fixture_content() -> Arc<viewer_audio::Content> {
    let mut music = Music::default();
    for (index, (station, body)) in [
        ("on", "base=1"),
        ("on", "base=2 modifier={has_country_flag=x factor=3}"),
        (
            "on",
            "base=1 modifier={always=yes factor=random} modifier={always=yes factor=random}",
        ),
        ("on", "base=3 modifier={check_variable={x=5} factor=2}"),
        ("on", "base=7 modifier={has_war_with=BBB factor=2}"),
        (
            "on",
            "base=4 modifier={check_variable={event_target:missing.x=0} factor=2}",
        ),
        (
            "on",
            "base=1 modifier={always=yes factor=0} modifier={unanswered_fixture=yes factor=2}",
        ),
        ("on", "base=0.12345"),
        ("off", "base=99 modifier={unanswered_fixture=yes factor=3}"),
        (
            "on",
            "base=1 modifier={always=yes factor=random} modifier={always=yes factor=random}",
        ),
        ("on", "base=5 modifier={date>1936.1.2 factor=2}"),
    ]
    .into_iter()
    .enumerate()
    {
        music.tracks.push(Track {
            name: format!("t{index}"),
            path: format!("music/t{index}.ogg"),
            volume: 1.0,
            source: String::new(),
        });
        music.songs.push(Song {
            track: index,
            station: station.into(),
            chance: content_reader_parser::parser::parse(body.as_bytes())
                .unwrap()
                .items,
            source: String::new(),
        });
    }
    Arc::new(
        viewer_audio::Content::new(Default::default(), None).with_music(
            music,
            Some(vec!["on".into()]),
            0,
        ),
    )
}

fn frame(country_count: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 16,
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
        metrics: Metrics::default(),
    };
    let mut put = |rows: TableRows| {
        let index = rows.table().index();
        frame.tables[index] = TableView::Complete(rows).into();
    };
    put(TableRows::Countries(
        (1..=country_count)
            .map(|slot| CountryRow {
                id: slot * 100,
                canonical_slot: slot,
                tag: if slot == 1 {
                    b"AAA".to_vec()
                } else if slot == 2 {
                    b"BBB".to_vec()
                } else {
                    format!("C{slot}").into_bytes()
                },
                ..Default::default()
            })
            .collect(),
    ));
    put(TableRows::CountryLinks(
        (1..=country_count)
            .map(|slot| CountryLinkRow {
                id: slot * 100,
                canonical_slot: slot,
            })
            .collect(),
    ));
    put(TableRows::CountryCounterInputs(
        (1..=country_count)
            .map(|slot| CountryCounterInputRow {
                country: slot * 100,
                owned_state_count: 1,
                ..Default::default()
            })
            .collect(),
    ));
    put(TableRows::LexicalTokens(vec![LexicalTokenRow {
        token: 900,
        literal: b"x".to_vec(),
    }]));
    put(TableRows::ScriptFlags(vec![ScriptFlagRow {
        scope_kind: 1,
        scope_id: 100,
        token: 900,
        value: 1,
        ..Default::default()
    }]));
    put(TableRows::ScriptVariables(
        (1..=country_count)
            .map(|slot| ScriptVariablesRow {
                scope_kind: 1,
                scope_id: slot * 100,
                seed: Recorded::Value(Some(37)),
                entries: vec![ScriptVariableEntry {
                    name: b"x".to_vec(),
                    value_raw: 500_000,
                }],
            })
            .collect(),
    ));
    put(TableRows::WarRelations(vec![WarRelationRow {
        first_country: 100,
        second_country: 200,
        start_hour: 10,
        end_hour: 20,
        ..Default::default()
    }]));
    put(TableRows::Clock(vec![ClockRow {
        hour: 16,
        seed: 37,
        ..Default::default()
    }]));
    frame.tables[TableId::GlobalSavedEventTargets.index()] = TableView::Unavailable {
        reason: "fixture saved targets".into(),
    }
    .into();
    frame
}

fn with_world<T>(frame: &DecodedFrame, run: impl FnOnce(&World<'_>) -> T) -> T {
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let dlcs = BTreeSet::new();
    let world = World::read(frame, ContentFacts::content_only(&dlcs, &names, &aliases));
    run(&world)
}

fn original(
    frame: &DecodedFrame,
    content: &viewer_audio::Content,
    country: u32,
) -> viewer_audio::Chances {
    with_world(frame, |world| {
        let evaluator = Evaluator::new(world, None);
        super::super::song_chances(content, |body| {
            evaluator
                .mean_time_to_happen("music chance", body, Scopes::of(Scope::Country(country)))
                .map_err(|why| why.to_string())
        })
    })
}

fn evaluations(memo: &Memo) -> usize {
    memo.evaluations.load(std::sync::atomic::Ordering::Relaxed)
}

fn run(
    memo: &Memo,
    frame: &DecodedFrame,
    content: &Arc<viewer_audio::Content>,
    country: u32,
) -> (viewer_audio::Chances, usize) {
    let expected = original(frame, content, country);
    let before = evaluations(memo);
    let actual = with_world(frame, |world| {
        inputs::reusing(|| memo.chances(frame, content, country, world))
    });
    assert_eq!(
        bits(&actual),
        bits(&expected),
        "original weights: branch {} hour {} country {country}",
        frame.branch,
        frame.hour
    );
    assert_eq!(
        actual.refusals, expected.refusals,
        "original refusals: branch {} hour {} country {country}",
        frame.branch, frame.hour
    );
    (actual, evaluations(memo) - before)
}

fn bits(chances: &viewer_audio::Chances) -> Vec<Option<u64>> {
    chances
        .weights
        .iter()
        .map(|weight| weight.map(f64::to_bits))
        .collect()
}

#[test]
fn music_inputs_keep_fixed_weights_rng_refusal_order_and_multi_hour_changes() {
    enter_music_tables();
    let content = fixture_content();
    let memo = Memo::new(8);
    let base = frame(3);
    let reference = original(&base, &content, 1);
    assert_eq!(
        bits(&reference),
        [
            Some(1.0f64.to_bits()),
            Some(6.0f64.to_bits()),
            Some(0.38783f64.to_bits()),
            Some(6.0f64.to_bits()),
            Some(14.0f64.to_bits()),
            None,
            Some(0.0f64.to_bits()),
            Some(0.12345f64.to_bits()),
            None,
            Some(0.38783f64.to_bits()),
            None,
        ]
    );
    assert!(
        reference
            .refusals
            .iter()
            .any(|why| why.starts_with("song t5:") && why.contains("saved event target")),
        "{:?}",
        reference.refusals
    );
    let (first, evaluated) = run(&memo, &base, &content, 1);
    assert_eq!(bits(&first), bits(&reference));
    assert_eq!(first.refusals, reference.refusals);
    assert_eq!(evaluated, 10);
    assert_eq!(
        first.weights.iter().filter(|value| value.is_none()).count(),
        3
    );
    let (warm, evaluated) = run(&memo, &base, &content, 1);
    assert_eq!(evaluated, 0, "confirmed refused outcomes are reused too");
    let mut player = viewer_audio::music::Player::new(37);
    let mut reference_player = viewer_audio::music::Player::new(37);
    for draw in 1..=128 {
        assert_eq!(
            player.update(&content.music, Some(&warm), 3),
            reference_player.update(&content.music, Some(&reference), 3)
        );
        assert_eq!(player.take_wanted(), reference_player.take_wanted());
        assert_eq!(player.playing(), reference_player.playing());
        assert_eq!(player.draws(), draw);
        assert_eq!(player.draws(), reference_player.draws());
        player.stop();
        reference_player.stop();
    }
    let before = evaluations(&memo);
    with_world(&base, |world| {
        let checked = memo.chances(&base, &content, 1, world);
        assert_eq!(checked.refusals, reference.refusals);
        assert_eq!(bits(&checked), bits(&reference));
    });
    assert_eq!(evaluations(&memo) - before, 10);
    let mut reused = 0;
    let mut evaluated = 0;
    for step in 0..192u32 {
        let mut next = base.clone();
        next.hour = [16, 17, 21, 10, 9, 60827943, 60827992][step as usize % 7];
        next.branch = if step.is_multiple_of(17) { 2 } else { 1 };
        let country = if step.is_multiple_of(13) { 2 } else { 1 };
        if step.is_multiple_of(3) {
            next.tables[TableId::ScriptFlags.index()] =
                TableView::Complete(TableRows::ScriptFlags(Vec::new())).into();
        }
        if step.is_multiple_of(5) {
            let mut rows = next.complete::<tables::ScriptVariables>().unwrap().to_vec();
            rows[0].seed = Recorded::Value(Some(38));
            rows[0].entries[0].value_raw = 100_000;
            next.tables[TableId::ScriptVariables.index()] =
                TableView::Complete(TableRows::ScriptVariables(rows)).into();
        }
        match step % 19 {
            0 => {
                next.tables[TableId::ScriptFlags.index()] = TableView::Unavailable {
                    reason: "fixture flags".into(),
                }
                .into()
            }
            1 => {
                next.tables[TableId::ScriptFlags.index()] = TableView::Truncated {
                    rows: TableRows::ScriptFlags(Vec::new()),
                    missing: 2,
                }
                .into()
            }
            2 => {
                next.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                        id: 100,
                        canonical_slot: 1,
                    }]))
                    .into()
            }
            3 => {
                next.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
                    reason: "fixture variables".into(),
                }
                .into()
            }
            _ => {}
        }
        next.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: next.hour,
                seed: step as i32,
                ..Default::default()
            }]))
            .into();
        let expected = original(&next, &content, country);
        let (actual, calls) = run(&memo, &next, &content, country);
        assert_eq!(bits(&actual), bits(&expected), "step {step}");
        assert_eq!(actual.refusals, expected.refusals, "step {step}");
        reused += 10 - calls;
        evaluated += calls;
    }
    assert!(reused > 0 && evaluated > 0);
    let mut changed = fixture_content();
    Arc::get_mut(&mut changed).unwrap().stations = Some(vec!["off".into()]);
    let (actual, evaluated) = run(&memo, &base, &changed, 1);
    assert_eq!(evaluated, 1);
    let mut expected = vec![None; 11];
    expected[8] = Some(297.0f64.to_bits());
    assert_eq!(bits(&actual), expected);
}

#[test]
fn music_inputs_missing_country_counts_and_known_null_targets_match_original() {
    enter_music_tables();
    let mut frame = frame(3);
    frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(Vec::new())).into();
    frame.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(Vec::new())).into();
    let content = fixture_content();
    let reference = original(&frame, &content, 1);
    assert_eq!(reference.weights[4], None);
    assert!(reference.refusals.iter().any(|why| {
        why.starts_with("song t4:") && why.contains("the owned state count of country BBB is not recorded")
    }), "{:?}", reference.refusals);
    assert_eq!(
        reference.weights[5].map(f64::to_bits),
        Some(8.0f64.to_bits())
    );
    assert!(
        reference
            .refusals
            .iter()
            .all(|why| !why.starts_with("song t5:"))
    );
    let memo = Memo::new(8);
    let (_, cold) = run(&memo, &frame, &content, 1);
    assert_eq!(cold, 10);
    let (_, warm) = run(&memo, &frame, &content, 1);
    assert_eq!(warm, 0);
}

#[test]
fn music_inputs_unknown_keys_eviction_and_contended_memo_recompute_without_waiting() {
    enter_music_tables();
    let frame = frame(2);
    let content = fixture_content();
    let reference = original(&frame, &content, 1);
    assert_eq!(
        reference.weights[0].map(f64::to_bits),
        Some(1.0f64.to_bits())
    );
    let memo = Memo::new(1);
    run(&memo, &frame, &content, 1);
    let (_, certificate) = inputs::capture(&frame, || {
        inputs::query(|| inputs::Key::FocusTag("AAA".into()), || 7u64)
    });
    {
        let mut entries = memo.entries.lock().unwrap();
        let mut songs = entries[0].songs.clone();
        songs[0] = Some(Arc::new(Evaluation {
            value: Outcome {
                value: Some(-1),
                refusal: None,
            },
            inputs: certificate,
        }));
        entries[0] = Arc::new(Entry {
            content: Arc::clone(&content),
            branch: frame.branch,
            country: 1,
            songs,
        });
    }
    let (actual, evaluated) = run(&memo, &frame, &content, 1);
    assert_eq!(actual.weights[0].map(f64::to_bits), Some(1.0f64.to_bits()));
    assert_eq!(
        evaluated, 1,
        "an unfamiliar certificate recomputes just that song"
    );
    run(&memo, &frame, &content, 2);
    let (_, evaluated) = run(&memo, &frame, &content, 1);
    assert_eq!(evaluated, 10, "evicted country recomputes");
    let lock = memo.entries.lock().unwrap();
    let (actual, evaluated) = run(&memo, &frame, &content, 1);
    assert_eq!(evaluated, 10);
    assert_eq!(bits(&actual), bits(&original(&frame, &content, 1)));
    drop(lock);
}

#[test]
fn music_inputs_size_from_five_thousand_countries_and_do_not_cap_authored_songs() {
    enter_music_tables();
    let frame = frame(5000);
    let mut content = fixture_content();
    let music = &mut Arc::get_mut(&mut content).unwrap().music;
    let base = music.songs[0].clone();
    music.songs.extend(std::iter::repeat_n(base, 5000));
    let memo = Memo::new(8);
    let (actual, evaluated) = run(&memo, &frame, &content, 5000);
    assert_eq!(actual.weights.len(), 5011);
    assert_eq!(evaluated, 5010);
    let (_, evaluated) = run(&memo, &frame, &content, 5000);
    assert_eq!(evaluated, 0);
}

#[test]
fn music_inputs_recheck_has_war_end_hours_and_unresolved_unrelated_endpoints() {
    enter_music_tables();
    let mut music = Music::default();
    for (index, body) in [
        "base=1 modifier={has_war=yes factor=2}",
        "base=1 modifier={has_war=no factor=2}",
        "base=3",
    ]
    .into_iter()
    .enumerate()
    {
        music.tracks.push(Track {
            name: format!("t{index}"),
            path: format!("music/t{index}.ogg"),
            volume: 1.0,
            source: String::new(),
        });
        music.songs.push(Song {
            track: index,
            station: "on".into(),
            chance: content_reader_parser::parser::parse(body.as_bytes())
                .unwrap()
                .items,
            source: String::new(),
        });
    }
    let content = Arc::new(
        viewer_audio::Content::new(Default::default(), None).with_music(
            music,
            Some(vec!["on".into()]),
            0,
        ),
    );
    for unresolved in [false, true] {
        let mut base = frame(3);
        base.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(vec![CountryWarRow {
                country: 100,
                enemies_read: 1,
                enemies: Vec::new(),
                ..Default::default()
            }]))
            .into();
        base.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: if unresolved { 200 } else { 100 },
                second_country: if unresolved { 300 } else { 200 },
                start_hour: 10,
                end_hour: 20,
                ..Default::default()
            }]))
            .into();
        if unresolved {
            let links = base
                .complete::<tables::CountryLinks>()
                .unwrap()
                .iter()
                .filter(|row| row.id != 300)
                .cloned()
                .collect();
            base.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(links)).into();
        }
        for first in [16, 21] {
            let memo = Memo::new(8);
            let other = if first == 16 { 21 } else { 16 };
            for (step, hour) in [first, first, other, other, first, first]
                .into_iter()
                .enumerate()
            {
                let mut next = base.clone();
                next.hour = hour;
                for (before, after) in base.tables.iter().zip(&next.tables) {
                    assert!(replay_frame::frame::SharedView::ptr_eq(before, after));
                }
                let (actual, evaluated) = run(&memo, &next, &content, 1);
                let active = hour == 16;
                let weights = match (active, unresolved) {
                    (true, true) => [None, None, Some(3.0f64)],
                    (true, false) => [Some(2.0f64), Some(1.0f64), Some(3.0f64)],
                    (false, _) => [Some(1.0f64), Some(2.0f64), Some(3.0f64)],
                };
                assert_eq!(
                    bits(&actual),
                    weights.map(|weight| weight.map(f64::to_bits)),
                    "unresolved {unresolved}, first {first}, hour {hour}"
                );
                let refusals = if active && unresolved {
                    vec![
                        "song t0: trigger has_war = yes: war relations second country is unresolved"
                            .to_owned(),
                        "song t1: trigger has_war = no: war relations second country is unresolved"
                            .to_owned(),
                    ]
                } else {
                    Vec::new()
                };
                assert_eq!(actual.refusals, refusals);
                assert_eq!(
                    evaluated,
                    if step == 0 {
                        3
                    } else if step % 2 == 0 {
                        2
                    } else {
                        0
                    },
                    "only hour-dependent songs reevaluate at crossings; warm refusals reuse"
                );
            }
        }
    }
}

#[test]
fn music_inputs_recheck_hour_boundaries_and_refused_table_availability() {
    enter_music_tables();
    let content = fixture_content();
    let memo = Memo::new(8);
    let base = frame(3);
    run(&memo, &base, &content, 1);
    let mut next = base.clone();
    next.hour = 21;
    let (ended, evaluated) = run(&memo, &next, &content, 1);
    assert_eq!(ended.weights[4].map(f64::to_bits), Some(7.0f64.to_bits()));
    assert!(evaluated > 0 && evaluated < 10);
    next.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(Vec::new())).into();
    let (known, evaluated) = run(&memo, &next, &content, 1);
    assert_eq!(known.weights[5].map(f64::to_bits), Some(8.0f64.to_bits()));
    assert!(
        known
            .refusals
            .iter()
            .all(|why| !why.starts_with("song t5:"))
    );
    assert!(evaluated > 0);
    next.tables[TableId::GlobalSavedEventTargets.index()] = TableView::Unavailable {
        reason: "changed target refusal".into(),
    }
    .into();
    let (unknown, evaluated) = run(&memo, &next, &content, 1);
    assert_eq!(unknown.weights[5], None);
    assert!(
        unknown
            .refusals
            .iter()
            .any(|why| why.starts_with("song t5:"))
    );
    assert!(evaluated > 0);
    let (_, evaluated) = run(&memo, &next, &content, 1);
    assert_eq!(
        evaluated, 0,
        "the changed refusal is reusable once confirmed"
    );
    next.branch = 2;
    let (_, evaluated) = run(&memo, &next, &content, 1);
    assert_eq!(evaluated, 10, "another branch cannot borrow this context");
}

#[test]
fn music_outcomes_keep_f64_scaling_and_leave_refused_songs_unknown_in_order() {
    let mut music = Music::default();
    for (index, (station, body)) in [
        ("on", "base=2 modifier={always=yes factor=3}"),
        ("off", "base=99"),
        ("on", "base=0.12345"),
        ("on", "base=3 modifier={always=yes factor=2}"),
        ("on", "base=-0.12345"),
    ]
    .into_iter()
    .enumerate()
    {
        music.tracks.push(Track {
            name: format!("t{index}"),
            path: format!("music/t{index}.ogg"),
            volume: 1.0,
            source: String::new(),
        });
        music.songs.push(Song {
            track: index,
            station: station.into(),
            chance: content_reader_parser::parser::parse(body.as_bytes())
                .unwrap()
                .items,
            source: String::new(),
        });
    }
    let content = viewer_audio::Content::new(Default::default(), None).with_music(
        music,
        Some(vec!["on".into()]),
        0,
    );
    let answers = [
        Err("first condition".to_owned()),
        Ok(12_345),
        Err("later condition".to_owned()),
        Ok(-12_345),
    ];
    let mut reference_answers = answers.clone().into_iter();
    let mut reference_calls = Vec::new();
    let reference = super::super::song_chances(&content, |body| {
        reference_calls.push(body.to_vec());
        reference_answers.next().unwrap()
    });
    assert!(reference_answers.next().is_none());
    let expected_bodies = [
        "base=2 modifier={always=yes factor=3}",
        "base=0.12345",
        "base=3 modifier={always=yes factor=2}",
        "base=-0.12345",
    ]
    .map(|body| {
        content_reader_parser::parser::parse(body.as_bytes())
            .unwrap()
            .items
    });
    assert_eq!(reference_calls, expected_bodies);
    assert_eq!(
        reference.refusals,
        ["song t0: first condition", "song t3: later condition"]
    );
    assert_eq!(
        bits(&reference),
        [
            None,
            None,
            Some(0.12345_f64.to_bits()),
            None,
            Some((-0.12345_f64).to_bits()),
        ]
    );
    let mut answers = answers.into_iter();
    let mut calls = Vec::new();
    let mut actual = viewer_audio::Chances::default();
    for (index, song) in content.music.songs.iter().enumerate() {
        if !content.enabled(index) {
            actual.weights.push(None);
            continue;
        }
        Outcome::evaluate(
            &content.music.tracks[song.track].name,
            &song.chance,
            &mut |body| {
                calls.push(body.to_vec());
                answers.next().unwrap()
            },
        )
        .append(&mut actual);
    }
    assert!(answers.next().is_none());
    assert_eq!(calls, expected_bodies);
    assert_eq!(bits(&actual), bits(&reference));
    assert_eq!(actual.refusals, reference.refusals);
}

#[test]
fn map_products_music_by_identity_reads_again_only_what_a_replaced_table_holds() {
    enter_music_tables();
    let content = fixture_content();
    let memo = map_products::music_inputs::Memo::by_identity();
    let mut kept = None;
    let mut check = |frame: &DecodedFrame, why: &str| {
        let expected = original(frame, &content, 1);
        let (actual, next) = with_world(frame, |world| memo.chances_kept(frame, &content, 1, world));
        kept = Some(next);
        assert_eq!(bits(&actual), bits(&expected), "{why}");
        assert_eq!(actual.refusals, expected.refusals, "{why}");
    };
    let mut frame = frame(2);
    check(&frame, "first");
    frame.hour = 17;
    check(&frame, "an hour on, every table the same allocation");
    frame.tables[TableId::ScriptFlags.index()] = TableView::Complete(TableRows::ScriptFlags(Vec::new())).into();
    check(&frame, "the flag gone with a new ScriptFlags");
    frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(Vec::new())).into();
    frame.hour = 18;
    check(&frame, "no war with a new WarRelations");
    drop(kept);
}
