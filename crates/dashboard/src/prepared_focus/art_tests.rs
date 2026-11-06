//! Focus art and static text, evaluated when a reply asks for them.
use super::*;
use crate::products::Product;
use crate::test_fixtures::{CAMPAIGN, records};
use replay::container::Replay;
use replay_data::rows::{CompletedFocusRow, FocusRow, TableRows};
use std::sync::Arc;

/// Kept art stays within its byte budget: the least recently used goes
/// first, art larger than the budget is not kept, and a replaced key counts
/// once.
#[test]
fn kept_art_stays_within_its_budget_evicting_the_least_recently_used() {
    let art = |token: &str, icon: usize| -> crate::catalogue::Drawn<artwork::ResolvedArt> {
        crate::catalogue::Drawn::new(|| {
            (
                Artwork {
                    token: token.into(),
                    name: Some("Name".into()),
                    description: None,
                    base_duration_days: None,
                    icon: Some("i".repeat(icon)),
                },
                None,
            )
        })
    };
    let key = |token: &str| {
        (
            crate::FocusDatabase::National,
            Presentation::Display,
            token.to_owned(),
            String::new(),
        )
    };
    let mut kept = art::Kept::with_budget(100_000);
    kept.insert(key("a"), art("a", 40_000));
    kept.insert(key("b"), art("b", 40_000));
    // Using a makes b the least recently used.
    assert!(kept.get(&key("a")).is_some());
    kept.insert(key("c"), art("c", 40_000));
    assert!(kept.bytes() <= 100_000);
    assert!(
        kept.get(&key("b")).is_none(),
        "the least recently used is evicted"
    );
    assert!(kept.get(&key("a")).is_some() && kept.get(&key("c")).is_some());
    kept.insert(key("c"), art("c", 40_000));
    assert_eq!(kept.count(), 2);
    assert!(kept.bytes() <= 100_000);
    kept.insert(key("huge"), art("huge", 200_000));
    assert!(kept.get(&key("huge")).is_none(), "larger than the budget");
    assert_eq!(kept.count(), 2);
}

/// `focus-art` evaluates any recorded hour as hourly preparation presented
/// it there, row for row and with its warnings: titles reading the country,
/// Clock-seeded random text and effect sessions, conditional icons, a
/// refused sprite and static titles, for a token subset or the whole order.
/// Static art is kept per content binding: another country asking first
/// leaves the next country's rows unchanged. A request of static tokens
/// reads no recording; one with a dynamic token reads its hour once.
#[test]
fn focus_art_matches_hourly_presentation_and_reads_frames_only_for_dynamic_tokens() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use super::art::art_at;
    use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
    use installed_content_image::Rgba8Image;
    use replay_data::rows::{CountryLinkRow, CountryRow};
    let definitions = format!(
        "focus_tree={{{} focus={{id=a_static text=static_title icon=GFX_mod_true}} focus={{id=b_refused text=static_title icon=GFX_missing}}}}",
        (0..12)
            .map(|index| format!(
                "focus={{id=t{index:02} text=title{} icon={{GFX_mod_true={{tag=GER}} GFX_mod_other=yes}}}}",
                index % 3
            ))
            .collect::<Vec<_>>()
            .join(" ")
    );
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
    let tokens: Vec<String> = (0..12)
        .map(|index| format!("t{index:02}"))
        .chain(["a_static".into(), "b_refused".into()])
        .collect();
    let frames: Vec<DecodedFrame> = [100, 101, 102]
        .into_iter()
        .map(|hour| {
            let mut decoded = crate::test_fixtures::source(1, hour, Some(1));
            for table in TableId::ALL {
                // Unused singleton inputs keep their named unavailable state.
                if table.key_bits() == 0 {
                    continue;
                }
                if matches!(
                    &*decoded.tables[table.index()],
                    TableView::Unavailable { .. }
                ) {
                    *decoded.tables[table.index()] = TableView::Complete(TableRows::empty(table));
                }
            }
            // ENG (9002, slot 8) completes the static focus too.
            *decoded.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![
                    CountryRow {
                        id: 9001,
                        canonical_slot: 7,
                        tag: b"GER".to_vec(),
                        ..Default::default()
                    },
                    CountryRow {
                        id: 9002,
                        canonical_slot: 8,
                        tag: b"ENG".to_vec(),
                        ..Default::default()
                    },
                ]));
            *decoded.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow {
                        id: 9001,
                        canonical_slot: 7,
                    },
                    CountryLinkRow {
                        id: 9002,
                        canonical_slot: 8,
                    },
                ]));
            *decoded.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(vec![
                FocusRow {
                    country: 9001,
                    completed_count: tokens.len() as i32,
                    current: Some(b"t05".to_vec()),
                    ..Default::default()
                },
                FocusRow {
                    country: 9002,
                    completed_count: 1,
                    ..Default::default()
                },
            ]));
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
                        .chain(std::iter::once(CompletedFocusRow {
                            country: 9002,
                            ordinal: 0,
                            token: b"a_static".to_vec(),
                        }))
                        .collect(),
                ));
            *decoded.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![replay_data::rows::ClockRow {
                    hour,
                    seed: 900 + hour,
                    ..Default::default()
                }]));
            *decoded.tables[TableId::PowerBalances.index()] =
                TableView::Complete(TableRows::PowerBalances(vec![
                    replay_data::rows::PowerBalanceRow {
                        balance: b"balance".to_vec(),
                        active_range: b"active".to_vec(),
                        ..Default::default()
                    },
                ]));
            *decoded.tables[TableId::PowerBalanceMembers.index()] =
                TableView::Complete(TableRows::PowerBalanceMembers(vec![
                    replay_data::rows::PowerBalanceMemberRow {
                        country: 9001,
                        ..Default::default()
                    },
                ]));
            decoded
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: uuid::Uuid::new_v4().to_string(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancelled,
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folders[0].0.clone(),
            user_dir: folders[0].0.join("user"),
            cache_dir: folders[0].0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let catalogue = Arc::new(catalogue);
    crate::catalogue::set_test_catalogue(&source, &content, Arc::clone(&catalogue));
    let diagnostic_source = Source::from_replay(
        crate::SourceKey {
            source_id: "diagnostic-art".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        source.replay(),
        &cancelled,
    )
    .unwrap();
    crate::catalogue::set_test_catalogue(&diagnostic_source, &content, Arc::clone(&catalogue));
    let diagnostic_art = || {
        installed_content_core::content::diagnostics::observe(|| {
            art_at(
                &diagnostic_source,
                &content,
                1,
                "GER",
                100,
                crate::FocusDatabase::National,
                &["a_static".into()],
                Presentation::Full,
                &cancelled,
            )
            .unwrap()
        })
    };
    let (cold_art, cold_notes) = diagnostic_art();
    assert!(cold_notes.has_uncovered());
    catalogue
        .content
        .layers
        .note_shader_warning("unrelated prior drawing".into());
    let (warm_art, warm_notes) = diagnostic_art();
    assert_eq!(
        serde_json::to_value(cold_art).unwrap(),
        serde_json::to_value(warm_art).unwrap()
    );
    assert_eq!(cold_notes, warm_notes);
    let reads = || art::FRAME_READS.with(std::cell::Cell::get);
    let national = artwork::Kind::Focus(crate::FocusDatabase::National);
    // Hourly preparation's rows: every token of the country's order
    // evaluated on the hour's whole frame by a context of its own.
    let oracle = |frame: &DecodedFrame, tag: &str, tokens: &[String], mode: Presentation| {
        let mut context = artwork::Context::load(Arc::clone(&catalogue), national).unwrap();
        let inputs = artwork::FocusFrame::new(frame, &catalogue);
        let requests: Vec<_> = tokens.iter().map(|token| (token.as_str(), tag)).collect();
        let rows: Vec<artwork::ResolvedArt> =
            context.focuses_in(&requests, &inputs, true, mode, |items| items.collect());
        let mut warnings = context.standing_warnings();
        warnings.sort();
        warnings.dedup();
        let rows: BTreeMap<String, serde_json::Value> = rows
            .into_iter()
            .map(|(artwork, reason)| {
                (
                    artwork.token.clone(),
                    serde_json::to_value(PresentedFocus { artwork, reason }).unwrap(),
                )
            })
            .collect();
        (rows, warnings)
    };
    let mut sorted = tokens.clone();
    sorted.sort();
    for frame in &frames {
        for mode in [Presentation::Display, Presentation::Text] {
            // ENG asks for the shared static focus first.
            let before = reads();
            let eng = art_at(
                &source,
                &content,
                1,
                "ENG",
                frame.hour,
                crate::FocusDatabase::National,
                &["a_static".into()],
                mode,
                &cancelled,
            )
            .unwrap();
            assert_eq!(reads(), before, "a static request reads no recording");
            let (expected, warnings) = oracle(frame, "ENG", &["a_static".into()], mode);
            assert_eq!(
                serde_json::to_value(&eng.art[0]).unwrap(),
                expected["a_static"]
            );
            assert_eq!(eng.warnings, warnings);
            let (expected, warnings) = oracle(frame, "GER", &sorted, mode);
            for subset in [
                sorted.clone(),
                vec!["a_static".into(), "b_refused".into()],
                sorted[..5].to_vec(),
            ] {
                let before = reads();
                let reply = art_at(
                    &source,
                    &content,
                    1,
                    "GER",
                    frame.hour,
                    crate::FocusDatabase::National,
                    &subset,
                    mode,
                    &cancelled,
                )
                .unwrap();
                let dynamic = subset.iter().any(|token| token.starts_with('t'));
                assert_eq!(
                    reads() - before,
                    usize::from(dynamic),
                    "{subset:?}: a frame only for a dynamic token"
                );
                assert!(reply.failures.is_empty());
                assert_eq!(reply.warnings, warnings);
                assert_eq!(reply.art.len(), subset.len());
                for (token, row) in subset.iter().zip(&reply.art) {
                    assert_eq!(
                        serde_json::to_value(row).unwrap(),
                        expected[token],
                        "{token} at {} in {mode:?}",
                        frame.hour
                    );
                }
            }
        }
    }
    // The refused sprite is named on its row.
    let refused = art_at(
        &source,
        &content,
        1,
        "GER",
        100,
        crate::FocusDatabase::National,
        &["b_refused".into()],
        Presentation::Display,
        &cancelled,
    )
    .unwrap();
    assert!(refused.art[0].reason.is_some());
    // Static text is the content's, in request order with repeats.
    let wanted = vec!["t00".to_owned(), "a_static".into(), "t00".into()];
    let text = art::static_text(
        &source,
        Some(&content),
        crate::FocusDatabase::National,
        &wanted,
        &cancelled,
    )
    .unwrap();
    let context = artwork::Context::load(Arc::clone(&catalogue), national).unwrap();
    for (token, item) in wanted.iter().zip(&text) {
        let (mut expected, _) = context
            .text_catalogue(BTreeSet::from([token.as_str()]), &cancelled)
            .unwrap();
        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::to_value(expected.pop().unwrap()).unwrap()
        );
    }
    assert!(
        art::static_text(
            &source,
            None,
            crate::FocusDatabase::National,
            &wanted,
            &cancelled
        )
        .is_err()
    );
    // Preparation chooses each completion's art at its hour, which only a
    // dynamic completion reads the frame for, and draws no icon; the reply
    // draws it, giving the event presentation of that hour's frame.
    let mut presenter = art::Presenter::default();
    let mut events = artwork::Context::load(Arc::clone(&catalogue), national).unwrap();
    for (hour, tag, token, dynamic) in [
        (101, "GER", "t00", true),
        (102, "GER", "a_static", false),
        (101, "ENG", "a_static", false),
    ] {
        let before = reads();
        let shown = presenter
            .present_sought(&source, &catalogue, (1, hour), &[(token, tag)])
            .unwrap();
        assert_eq!(reads() > before, dynamic, "{token} at {hour}");
        let expected = events
            .focuses_at(&[(token, tag)], &frames[(hour - 100) as usize])
            .pop()
            .unwrap();
        let [art::CompletionArt::Shown { art: chosen, .. }] = &shown[..] else {
            panic!("{token} at {hour}: {shown:?}");
        };
        assert_eq!(chosen.art.icon, None, "{token} at {hour}: nothing drawn");
        let (replied, _) = art::event_art(&source, &content, &[(hour, shown.into_iter().next())])
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&replied[0]).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    // Completions sharing an hour's text context each name what building it
    // noted, so a page showing either names it.
    let shared = art::Presenter::default()
        .present_sought(&source, &catalogue, (1, 101), &[("t00", "GER"), ("t00", "GER")])
        .unwrap();
    for shown in &shared {
        let art::CompletionArt::Shown { noted, .. } = shown else {
            panic!("{shown:?}");
        };
        assert!(
            noted
                .readers()
                .contains(&dashboard_text::catalogue::Reader::Scripted),
            "{shown:?}"
        );
    }
}

/// `focus-art` asks for distinct tokens in order at each hour, hours in
/// order with each database once, at most a page and the active focus in all.
#[test]
fn focus_art_params_are_ordered_groups_of_distinct_tokens_within_a_page() {
    use crate::FocusDatabase::{Continuous, National};
    let group = |hour: i32, database, tokens: Vec<&str>| FocusArtGroup {
        hour,
        database,
        tokens: tokens.into_iter().map(str::to_owned).collect(),
    };
    let params = |groups: Vec<FocusArtGroup>| FocusArtParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        groups,
        mode: FocusArtMode::Display,
    };
    assert!(FocusArt::validate(&params(vec![group(100, National, vec!["a", "b"])])).is_ok());
    assert!(
        FocusArt::validate(&params(vec![
            group(100, National, vec!["a"]),
            group(100, Continuous, vec!["a"]),
            group(101, National, vec!["a"]),
        ]))
        .is_ok()
    );
    for tokens in [
        vec![],
        vec!["b", "a"],
        vec!["a", "a"],
        vec![""],
        vec!["a\0"],
    ] {
        assert!(FocusArt::validate(&params(vec![group(100, National, tokens)])).is_err());
    }
    assert!(FocusArt::validate(&params(Vec::new())).is_err());
    for groups in [
        vec![group(101, National, vec!["a"]), group(100, National, vec!["a"])],
        vec![group(100, National, vec!["a"]), group(100, National, vec!["b"])],
        vec![group(100, Continuous, vec!["a"]), group(100, National, vec!["b"])],
    ] {
        assert!(FocusArt::validate(&params(groups)).is_err());
    }
    let tokens = |count: usize| -> Vec<String> {
        (0..count).map(|index| format!("t{index:04}")).collect()
    };
    let mut full = params(vec![group(100, National, Vec::new()), group(101, National, vec!["a"])]);
    full.groups[0].tokens = tokens(PAGE_ROWS);
    assert!(FocusArt::validate(&full).is_ok());
    full.groups[1].tokens.push("b".into());
    assert!(FocusArt::validate(&full).is_err());
}

/// `focus-art` and `focus-static-text` look every token up in the database
/// the request names. One id names a different focus in each: the art kept
/// for the national focus never answers for the continuous one, and a
/// refused continuous file leaves the national focus answered.
#[test]
fn focus_art_and_static_text_keep_each_database_apart() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::FocusDatabase::{Continuous, National};
    for refused in [false, true] {
        let mut files = vec![
            (
                "common/national_focus/databases.txt",
                "focus_tree={focus={id=shared text=national_title}}",
            ),
            (
                "common/continuous_focus/databases.txt",
                "continuous_focus_palette={focus={id=shared text=continuous_title}}",
            ),
            (
                "localisation/databases_l_english.yml",
                "\u{feff}l_english:\n national_title:0 \"National title\"\n national_title_desc:0 \"National details\"\n continuous_title:0 \"Continuous title\"\n continuous_title_desc:0 \"Continuous details\"\n",
            ),
        ];
        if refused {
            // A later continuous file the reader refuses could redefine any continuous focus.
            files.push((
                "common/continuous_focus/zz_refused.txt",
                "continuous_focus_palette={focus={text=continuous_title}}",
            ));
        }
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&files);
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![crate::test_fixtures::source(1, 100, Some(1))])],
        )
        .unwrap();
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: uuid::Uuid::new_v4().to_string(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &cancelled,
        )
        .unwrap();
        let content = ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.join("user"),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        crate::catalogue::set_test_catalogue(&source, &content, Arc::new(catalogue));
        let context = LoadContext {
            content: Some(content.clone()),
        };
        let art = |database| {
            let reply = FocusArt::load(
                &source,
                &FocusArtParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    groups: vec![FocusArtGroup {
                        hour: 100,
                        database,
                        tokens: vec!["shared".into()],
                    }],
                    mode: FocusArtMode::Text,
                },
                &context,
                &cancelled,
            )
            .unwrap();
            let [group] = &reply.groups[..] else {
                panic!("{:?}", reply.groups);
            };
            assert!(group.error.is_none(), "{:?}", group.error);
            assert!(group.failures.is_empty(), "{:?}", group.failures);
            group.art[0].clone()
        };
        // The national art is kept first, where a key without its database would answer again.
        for _ in 0..2 {
            let national = art(National);
            assert_eq!(national.artwork.name.as_deref(), Some("National title"));
            assert_eq!(
                national.artwork.description.as_deref(),
                Some("National details")
            );
            assert_eq!(national.reason, None);
            let continuous = art(Continuous);
            if refused {
                assert_eq!(continuous.artwork.name, None);
                assert_eq!(continuous.artwork.description, None);
                let reason = continuous.reason.unwrap();
                assert!(
                    reason.contains("common/continuous_focus/zz_refused.txt"),
                    "{reason}"
                );
            } else {
                assert_eq!(continuous.artwork.name.as_deref(), Some("Continuous title"));
                assert_eq!(
                    continuous.artwork.description.as_deref(),
                    Some("Continuous details")
                );
                assert_eq!(continuous.reason, None);
            }
        }
        let text = |database| {
            art::static_text(
                &source,
                Some(&content),
                database,
                &["shared".to_owned()],
                &cancelled,
            )
            .unwrap()
            .remove(0)
        };
        let national = text(National);
        assert_eq!(national.name.as_deref(), Some("National title"));
        assert_eq!(national.description.as_deref(), Some("National details"));
        let continuous = text(Continuous);
        let expected = |text: &str| (!refused).then(|| text.to_owned());
        assert_eq!(continuous.name, expected("Continuous title"));
        assert_eq!(continuous.description, expected("Continuous details"));
        // The event feed's completions are national focuses.
        let catalogue = crate::catalogue::catalogue(&source, &content).unwrap();
        let events = art::Presenter::default()
            .present_sought(&source, &catalogue, (1, 100), &[("shared", "GER")])
            .unwrap();
        let [art::CompletionArt::Shown { art: event, .. }] = &events[..] else {
            panic!("{events:?}");
        };
        assert_eq!(event.art.name.as_deref(), Some("National title"));
        assert_eq!(event.art.description.as_deref(), Some("National details"));
    }
}
