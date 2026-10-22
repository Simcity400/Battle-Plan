use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::NukeRow, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn frame(hour: i32, rows: &[(u32, u32, i64, i32)], count: u32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("C{id:05}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
        rows.iter()
            .map(|&(country, nuke_type, amount_raw, ready)| NukeRow {
                country,
                nuke_type,
                amount_raw,
                ready,
            })
            .collect(),
    ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: NUCLEAR_BOMB_TOKEN,
                literal: b"mod_fission_stockpile".to_vec(),
            },
            LexicalTokenRow {
                token: THERMONUCLEAR_BOMB_TOKEN,
                literal: b"mod_fusion_stockpile".to_vec(),
            },
        ]));
    frame
}

fn scanned(frames: Vec<DecodedFrame>) -> ScannedNukes {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    scan(&replay, 1, &AtomicBool::new(false)).unwrap()
}

#[test]
fn country_nukes_native_amount_rounding_and_sentinels_do_not_use_launch_ready() {
    assert_eq!(displayed_amount(199_999_999), 1);
    assert_eq!(displayed_amount(200_000_000), 2);
    assert_eq!(displayed_amount(-199_999_999), -1);
    assert_eq!(displayed_amount(i64::MIN), 0xa7c5);
    assert_eq!(displayed_amount(i64::MIN + 1), 0xa7c5);
    assert_eq!(displayed_amount(i64::MAX), 0xa7c5);
    // Finite raw still uses machine-width multiplication, not raw / 100_000_000.
    assert_eq!(displayed_amount(i64::MAX - 1), 0);
    let result = scanned(vec![frame(
        10,
        &[(1, 0, 200_000_000, 97), (1, 1, 300_000_000, 99)],
        1,
    )]);
    assert_eq!(result.read_hours, [10]);
    assert_eq!(result.series["C00001"][0].total, Some(5));
    assert_eq!(
        result.series["C00001"][0].counts,
        [
            NukeCount {
                ordinal: 0,
                value: 2
            },
            NukeCount {
                ordinal: 1,
                value: 3
            }
        ]
    );
}

#[test]
fn country_nukes_missing_rows_and_gaps_remain_missing_and_identities_break() {
    let mut changed_identity = frame(15, &[(1, 0, 200_000_000, 0), (1, 1, 0, 0)], 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed_identity.tables[TableId::Countries.index()]
    {
        rows[0].id = 90;
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed_identity.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 90,
            canonical_slot: 1,
        });
    }
    let result = scanned(vec![
        frame(10, &[(1, 0, 200_000_000, 0), (1, 1, 0, 0)], 2),
        frame(11, &[], 2),
        frame(12, &[(1, 0, 200_000_000, 0)], 2),
        frame(13, &[(1, 1, 300_000_000, 0)], 2),
        frame(14, &[(1, 0, 200_000_000, 0), (1, 1, 0, 0)], 2),
        changed_identity,
        // Hour 16 is missing; neither native type is filled across it.
        frame(17, &[(1, 0, 200_000_000, 0), (1, 1, 0, 0)], 2),
    ]);
    assert!(!result.series.contains_key("C00002"));
    let runs = &result.series["C00001"];
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.total, run.starts_segment))
            .collect::<Vec<_>>(),
        [
            (10, 10, Some(2), true),
            (12, 12, Some(2), true),
            (13, 13, None, false),
            (14, 14, Some(2), false),
            (15, 15, Some(2), true),
            (17, 17, Some(2), true)
        ]
    );
    // The shorter complete array at hour 12 contributes its own exact total;
    // the other type is absent, never an invented zero.
    assert_eq!(
        runs[1].counts,
        [NukeCount {
            ordinal: 0,
            value: 2
        }]
    );
}

#[test]
fn country_nukes_complete_independent_native_arrays_keep_their_own_totals() {
    let result = scanned(vec![frame(
        10,
        &[
            (1, 0, 200_000_000, 0),
            (2, 0, 300_000_000, 0),
            (2, 1, 500_000_000, 0),
        ],
        3,
    )]);
    assert_eq!(result.series["C00001"][0].total, Some(2));
    assert_eq!(result.series["C00002"][0].total, Some(8));
    assert_eq!(result.series["C00001"][0].counts.len(), 1);
    assert!(!result.series.contains_key("C00003"));
}

#[test]
fn country_nukes_unsupported_native_type_keeps_its_amount_and_complete_total() {
    let result = scanned(vec![frame(
        10,
        &[(1, 0, 200_000_000, 0), (1, 1, 0, 0), (1, 2, 900_000_000, 0)],
        1,
    )]);
    assert_eq!(result.refused.get(&2), Some(&1));
    assert_eq!(result.series["C00001"][0].total, Some(11));
    assert_eq!(
        result.series["C00001"][0].counts.last(),
        Some(&NukeCount {
            ordinal: 2,
            value: 9
        })
    );
    let types = type_info(&result, |key| Some(format!("Name of {key}")));
    let unknown = types.iter().find(|kind| kind.ordinal == 2).unwrap();
    assert!(!unknown.supported);
    assert_eq!(unknown.token, None);
    assert_eq!(unknown.name, None);
}

#[test]
fn country_nukes_a_native_array_hole_stays_unknown_even_with_unnamed_counts() {
    let result = scanned(vec![frame(
        10,
        &[(1, 0, 200_000_000, 0), (1, 1, 0, 0), (1, 9, 900_000_000, 0)],
        1,
    )]);
    assert_eq!(result.refused.get(&9), Some(&1));
    assert_eq!(result.series["C00001"][0].total, None);
    assert_eq!(result.series["C00001"][0].counts.len(), 3);
    let types = type_info(&result, |key| Some(format!("Name of {key}")));
    assert!(
        !types
            .iter()
            .find(|kind| kind.ordinal == 9)
            .unwrap()
            .supported
    );
}

#[test]
fn country_nukes_top_omits_zero_missing_and_absent_but_chosen_zero_is_exact() {
    let mut absent = frame(12, &[(1, 0, 0, 0), (2, 0, 700_000_000, 0)], 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *absent.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.id != 2);
    }
    let result = scanned(vec![
        frame(10, &[(1, 0, 0, 0), (2, 0, 200_000_000, 0)], 3),
        frame(11, &[(1, 0, 0, 0), (2, 0, 0, 0)], 3),
        absent,
    ]);
    let top = requested_series(
        &result,
        &NukesParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: Some(10),
            origin_hour: None,
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].tag, "C00002");
    assert_eq!(
        top[0].membership,
        Some(vec![HourSpan {
            first_hour: 10,
            last_hour: 10
        }])
    );
    let chosen = requested_series(
        &result,
        &NukesParams {
            branch_id: "1".into(),
            tags: vec!["C00001".into(), "C00003".into()],
            top: None,
            origin_hour: None,
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].runs[0].total, Some(0));
    assert_eq!(chosen[0].runs[0].last_hour, 12);
    assert_eq!(result.series["C00002"].last().unwrap().last_hour, 11);
}

#[test]
fn country_nukes_all_zero_top_is_empty_and_signed_nonzero_totals_still_rank() {
    let zeros = scanned(vec![frame(10, &[(1, 0, 0, 0), (2, 0, 0, 0)], 2)]);
    let params = NukesParams {
        branch_id: "1".into(),
        tags: Vec::new(),
        top: Some(10),
        origin_hour: None,
    };
    assert!(
        requested_series(&zeros, &params, None, &AtomicBool::new(false))
            .unwrap()
            .is_empty()
    );
    let signed = scanned(vec![frame(10, &[(1, 0, 0, 0), (2, 0, -100_000_000, 0)], 2)]);
    let top = requested_series(&signed, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].tag, "C00002");
    assert_eq!(top[0].runs[0].total, Some(-1));
    assert_eq!(zeros.empty_read_hours, [10]);
    assert!(signed.empty_read_hours.is_empty());
}

#[test]
fn country_nukes_empty_rank_reads_keep_missing_unknown_and_identity_failures_distinct() {
    let mut countries_unknown = frame(13, &[(1, 0, 0, 0)], 1);
    *countries_unknown.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic unreadable country identity".into(),
    };
    let mut links_unknown = frame(14, &[(1, 0, 0, 0)], 1);
    *links_unknown.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic unreadable country links".into(),
    };
    let mut partial = frame(15, &[(1, 0, 0, 0)], 1);
    *partial.tables[TableId::Nukes.index()] = TableView::Truncated {
        rows: TableRows::Nukes(vec![NukeRow {
            country: 1,
            nuke_type: 0,
            amount_raw: 0,
            ready: 0,
        }]),
        missing: 1,
    };
    let result = scanned(vec![
        // A complete empty table is an empty race, without synthetic series.
        frame(10, &[], 2),
        // Country 2 has no recorded amount and is outside the race.
        frame(11, &[(1, 0, 0, 0)], 2),
        // An unknown array is different from both zero and missing.
        frame(12, &[(1, 1, 0, 0)], 2),
        countries_unknown,
        links_unknown,
        partial,
        frame(16, &[(90, 0, 0, 0)], 1),
    ]);
    assert_eq!(result.empty_read_hours, [10, 11]);
    assert_eq!(result.read_hours, [10, 11, 12, 13, 14, 16]);
    assert!(!result.series.contains_key("C00002"));
    assert_eq!(result.series["C00001"][0].first_hour, 11);
    assert_eq!(result.series["C00001"][1].total, None);
}

#[test]
fn country_nukes_static_tokens_resolve_independently_from_sorted_recorded_rows() {
    let mut partial = frame(10, &[(1, 0, 0, 0)], 1);
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *partial.tables[TableId::LexicalTokens.index()]
    {
        rows.retain(|row| row.token == NUCLEAR_BOMB_TOKEN);
    }
    let result = scanned(vec![partial, frame(11, &[(1, 0, 0, 0)], 1)]);
    assert_eq!(
        result.tokens.get(&0).map(String::as_str),
        Some("mod_fission_stockpile")
    );
    assert_eq!(
        result.tokens.get(&1).map(String::as_str),
        Some("mod_fusion_stockpile")
    );
}

#[test]
fn country_nukes_are_ranked_without_player_status_filtering() {
    let rows: Vec<_> = (1..=24)
        .flat_map(|country| {
            [
                (country, 0, i64::from(country) * 100_000_000, 0),
                (country, 1, 0, 0),
            ]
        })
        .collect();
    let result = scanned(vec![frame(10, &rows, 24)]);
    assert_eq!(result.series.len(), 24);
    let series = requested_series(
        &result,
        &NukesParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: Some(10),
            origin_hour: None,
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(series.len(), 10);
    assert!(series.iter().any(|series| series.tag == "C00024"));
    assert!(
        series
            .iter()
            .all(|series| series.runs[0].total.unwrap() >= 15)
    );
    assert!(dashboard_core::transport::json_len(&series, MAX_PRODUCT_REPLY_BYTES).is_some());
    for params in [
        NukesParams {
            branch_id: "1".into(),
            tags: result.series.keys().cloned().collect(),
            top: None,
            origin_hour: None,
        },
        NukesParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: Some(24),
            origin_hour: None,
        },
    ] {
        CountryNukes::validate(&params).unwrap();
        let series = requested_series(&result, &params, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(series.len(), 24);
    }
}

#[test]
fn country_nukes_only_serialized_reply_bytes_bound_the_selection() {
    let counts = (0..MAX_PRODUCT_REPLY_BYTES / 64 + 1)
        .map(|ordinal| NukeCount {
            ordinal: ordinal as u32,
            value: 0,
        })
        .collect();
    let mut result = ScannedNukes::default();
    result.series.insert(
        "opaque".into(),
        vec![NukeRun {
            first_hour: 10,
            last_hour: 10,
            starts_segment: true,
            counts,
            total: Some(0),
        }],
    );
    let params = NukesParams {
        branch_id: "1".into(),
        tags: vec!["opaque".into()],
        top: None,
        origin_hour: None,
    };
    // The old per-count estimate refused this reply despite its actual fit.
    let selected = requested_series(&result, &params, None, &AtomicBool::new(false)).unwrap();
    assert!(dashboard_core::transport::json_len(&selected, MAX_PRODUCT_REPLY_BYTES).is_some());
    let counts = &mut result.series.get_mut("opaque").unwrap()[0].counts;
    counts.extend(
        (counts.len()..MAX_PRODUCT_REPLY_BYTES / 20).map(|ordinal| NukeCount {
            ordinal: ordinal as u32,
            value: 0,
        }),
    );
    let error = requested_series(&result, &params, None, &AtomicBool::new(false)).unwrap_err();
    assert!(error.to_string().starts_with("NuclearReplyTooLarge:"));
}

#[test]
fn country_nukes_modded_type_names_come_from_the_content_stack() {
    let result = scanned(vec![frame(10, &[(1, 0, 200_000_000, 0), (1, 1, 0, 0)], 1)]);
    let scratch = std::env::var_os("BATTLE_PLAN_WIDGET_TEST_SCRATCH").or_else(|| std::env::var_os("HOI4_WIDGET_TEST_SCRATCH"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = scratch.join(format!("hoi4_nuclear_content_{}", std::process::id()));
    let vanilla = root.join("base");
    let modded = root.join("mod");
    for (folder, path, text) in [
        (
            &vanilla,
            "localisation/english/base_l_english.yml",
            "\u{feff}l_english:\n mod_fission_stockpile:0 \"Base name\"\n mod_fusion_stockpile:0 \"Base fusion\"\n",
        ),
        (
            &modded,
            "localisation/replace/english/override_l_english.yml",
            "\u{feff}l_english:\n mod_fission_stockpile:0 \"Mod singularity bombs\"\n mod_fusion_stockpile:0 \"Mod stellar bombs\"\n",
        ),
    ] {
        let path = folder.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers =
        installed_content_core::ResolvedLayers::from_folders(&[(&vanilla, &[][..]), (&modded, &[][..])])
            .unwrap();
    let localisation = installed_content_localisation::localisation::read(&layers, "english").unwrap();
    let types = type_info(&result, |token| {
        localisation.get(token)?;
        dashboard_text::catalogue::localised(&localisation, token, catalogue::View::Plain).ok()
    });
    assert_eq!(types[0].name.as_deref(), Some("Mod singularity bombs"));
    assert_eq!(types[1].name.as_deref(), Some("Mod stellar bombs"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn country_nukes_selections_follow_native_holders_through_rename_and_tag_reuse() {
    let mut first = frame(10, &[(1, 0, 200_000_000, 0)], 1);
    let mut renamed = frame(11, &[(1, 0, 300_000_000, 0)], 1);
    let mut reused = frame(12, &[(1, 0, 400_000_000, 0), (2, 0, 9_900_000_000, 0)], 2);
    let mut replaced = frame(13, &[(1, 0, 700_000_000, 0), (2, 0, 10_000_000_000, 0)], 2);
    for (frame, tags) in [
        (&mut first, vec!["OLD"]),
        (&mut renamed, vec!["NEW"]),
        (&mut reused, vec!["NEW", "OLD"]),
        (&mut replaced, vec!["NEW", "OLD"]),
    ] {
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            panic!("countries")
        };
        for (row, tag) in rows.iter_mut().zip(tags) {
            row.tag = tag.as_bytes().to_vec();
        }
    }
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replaced.tables[TableId::Countries.index()]
    else {
        panic!("countries")
    };
    rows[0].id = 90;
    rows.sort_by_key(|row| row.id);
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first, renamed, reused, replaced])],
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let history =
        dashboard_base::products::country_history::scan_histories(&replay, 1, false, &[], &cancel)
            .unwrap();
    let scanned = scan(&replay, 1, &cancel).unwrap();
    let params = NukesParams {
        branch_id: "1".into(),
        tags: vec!["OLD".into()],
        top: None,
        origin_hour: Some(10),
    };
    CountryNukes::validate(&params).unwrap();
    let selected = requested_series(&scanned, &params, Some(&history), &cancel).unwrap();
    let old = selected.iter().find(|series| series.tag == "OLD").unwrap();
    let new = selected.iter().find(|series| series.tag == "NEW").unwrap();
    assert_eq!(
        old.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.total))
            .collect::<Vec<_>>(),
        [(10, 10, Some(2))]
    );
    assert_eq!(
        new.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.total))
            .collect::<Vec<_>>(),
        [(11, 11, Some(3)), (12, 12, Some(4))]
    );
    assert_eq!(
        old.segments,
        [HourSpan {
            first_hour: 10,
            last_hour: 10
        }]
    );
    assert_eq!(
        new.segments,
        [HourSpan {
            first_hour: 11,
            last_hour: 12
        }]
    );
    let manual = NukesParams {
        origin_hour: None,
        ..params.clone()
    };
    let selected = requested_series(&scanned, &manual, Some(&history), &cancel).unwrap();
    let old = selected.iter().find(|series| series.tag == "OLD").unwrap();
    let new = selected.iter().find(|series| series.tag == "NEW").unwrap();
    // A manual root includes every holder of OLD; NEW is included only while
    // it belongs to one of those identities, never its unrelated replacement.
    assert_eq!(
        old.runs.iter().map(|run| run.total).collect::<Vec<_>>(),
        [Some(2), Some(99), Some(100)]
    );
    assert_eq!(
        new.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.total))
            .collect::<Vec<_>>(),
        [(11, 11, Some(3)), (12, 12, Some(4))]
    );
    assert_eq!(new.membership, Some(new.segments.clone()));
    let missing = NukesParams {
        origin_hour: Some(9),
        ..params.clone()
    };
    assert!(
        requested_series(&scanned, &missing, Some(&history), &cancel)
            .unwrap()
            .is_empty()
    );
    assert!(requested_series(&scanned, &params, None, &cancel).is_err());
    let new_holder = NukesParams {
        origin_hour: Some(12),
        ..params
    };
    let selected = requested_series(&scanned, &new_holder, Some(&history), &cancel).unwrap();
    assert_eq!(selected.len(), 1);
    assert!(selected[0].runs.iter().all(|run| run.total.unwrap() >= 99));
}
