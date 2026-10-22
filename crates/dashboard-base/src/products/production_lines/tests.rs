use super::*;
use crate::products::equipment_icon::Probe;

/// What `content`'s answers make of a variant icon choice, as a reply's key.
fn answered(content: &IconContent, choice: &IconChoice) -> Result<Option<String>, String> {
    Ok(choice
        .walk(|probe| content.answer(probe))?
        .map(|icon| icon_key(&icon)))
}
use crate::test_fixtures::{
    CAMPAIGN, LineFixture, OutputFixture, PRODUCTION_HOUR as FIRST, equipment_types,
    production_frame, records,
};
use replay::container::Replay;
use replay_data::{rows::CountryLinkRow, rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

#[test]
fn production_hour_batches_its_supplying_flags_and_matches_individual_lookups() {
    let format = replay_data::format::FormatVersion::CURRENT;
    for at in 0..48 {
        let mut frame = production_frame(
            FIRST + at,
            at % 24 == 0,
            true,
            9001,
            &[(5, 70, 8, 3276800)],
            None,
        );
        // A peer read dates the entire cadence group, including off-slot reads.
        frame.metrics.table_ns[TableId::ProductionLineModifiers.index()] = u64::from(at == 7);
        if at == 13 {
            *frame.tables[TableId::VariantChanges.index()] =
                TableView::Complete(TableRows::VariantChanges(vec![
                    replay_data::rows::VariantChangeRow {
                        sequence: 1,
                        hour: frame.hour,
                        variant: 70,
                        removed: true,
                        ..Default::default()
                    },
                ]));
        }
        let before = crate::metadata::slot_read_batches();
        let actual = scan_hour(format, frame.hour, &frame, "GER", None);
        assert_eq!(crate::metadata::slot_read_batches() - before, 1);
        let read = |table| crate::metadata::slot_read(format, &frame.metrics, table, frame.hour);
        assert_eq!(actual.lines_read, read(TableId::ProductionLines));
        assert_eq!(
            actual.definitions_read,
            (equipment::design_changed(&frame) || read(TableId::VariantDefinitions))
                && frame.complete::<tables::VariantDefinitions>().is_ok()
        );
        assert_eq!(
            actual.output_due.is_some(),
            read(TableId::ProductionLineOutput)
        );
        assert_eq!(
            actual.modifiers_read,
            read(TableId::CountryProductionModifiers) && read(TableId::ProductionLineModifiers)
        );
        assert_eq!(actual.variants, VariantCatalogue::hour_of(format, &frame));
        if at == 13 {
            assert_eq!(actual.variants, VariantHour::Read);
            assert!(actual.definitions_read);
        }
    }
}

#[test]
fn native_producer_scan_follows_renames_excludes_reused_tags_and_keeps_real_gaps() {
    use replay_data::{rows::CountryRow, rows::ProductionLineRow};
    let frames = (FIRST..=FIRST + 50)
        .filter(|hour| ![FIRST + 30, FIRST + 31].contains(hour))
        .map(|hour| {
            let mut frame = production_frame(
                hour,
                [FIRST, FIRST + 24, FIRST + 48].contains(&hour),
                true,
                9001,
                &[(5, 70, 8, 3276800)],
                Some(&[(5, 70, i64::from(hour - FIRST), true, false)]),
            );
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                panic!("countries")
            };
            countries
                .iter_mut()
                .find(|country| country.id == 9001)
                .unwrap()
                .tag = if hour < FIRST + 24 {
                b"X".to_vec()
            } else {
                b"Y".to_vec()
            };
            if hour >= FIRST + 24 {
                countries.push(CountryRow {
                    id: 9002,
                    canonical_slot: 8,
                    tag: b"X".to_vec(),
                    ..Default::default()
                });
                let TableView::Complete(TableRows::CountryLinks(links)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                else {
                    panic!("links")
                };
                links.push(CountryLinkRow {
                    id: 9002,
                    canonical_slot: 8,
                });
                links.sort_by_key(|link| link.id);
                let TableView::Complete(TableRows::ProductionLines(lines)) =
                    &mut *frame.tables[TableId::ProductionLines.index()]
                else {
                    panic!("lines")
                };
                lines.push(ProductionLineRow {
                    handle: 88,
                    country: 9002,
                    variant: 70,
                    active_factories: 99,
                    ..Default::default()
                });
                lines.sort_by_key(|line| line.handle);
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let selected = scan_holder(
        &replay,
        1,
        "X",
        None,
        Some((7, 9001)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(selected.lines.len(), 1);
    let line = &selected.lines[0];
    assert_eq!(line.key, "5#1");
    assert!(
        line.runs
            .iter()
            .any(|run| run.first_hour <= FIRST + 24 && run.last_hour >= FIRST + 24)
    );
    assert!(
        !line
            .runs
            .iter()
            .any(|run| run.first_hour <= FIRST + 30 && run.last_hour >= FIRST + 30)
    );
    assert!(
        line.runs
            .iter()
            .any(|run| run.first_hour == FIRST + 48 && run.starts_segment)
    );
    assert!(line.runs.iter().all(|run| run.active_factories == 8));
    assert_eq!(line.ended_by, None);
    let literal = scan(&replay, 1, "X", None, &AtomicBool::new(false)).unwrap();
    assert!(literal.lines.iter().any(|line| line.handle == 88));
}

#[test]
fn changed_modifier_layout_refuses_only_the_production_supplement_and_counts_the_supplying_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{
        rows::CountryProductionModifiersRow, rows::LexicalTokenRow, rows::ModifierBreakdownNode,
        rows::ModifierDefinitionRow, rows::ProductionLineModifiersRow,
        rows::ProductionModifierTree,
    };
    let mut frame = production_frame(FIRST, true, true, 9001, &[(5, 70, 1, 3276800)], None);
    let TableView::Complete(TableRows::ProductionLines(rows)) =
        &mut *frame.tables[TableId::ProductionLines.index()]
    else {
        panic!("lines");
    };
    rows.iter_mut().find(|row| row.handle == 5).unwrap().country = 9001;
    let keys = replay_derive::fixed_layout::PRODUCTION_MODIFIERS.keys;
    *frame.tables[TableId::CountryProductionModifiers.index()] =
        TableView::Complete(TableRows::CountryProductionModifiers(vec![
            CountryProductionModifiersRow {
                country: 9001,
                trees: keys
                    .iter()
                    .map(|key| ProductionModifierTree {
                        key: *key,
                        failure: vec![],
                        nodes: vec![ModifierBreakdownNode {
                            children: 0,
                            value: 0,
                            name: b"Synthetic source".to_vec(),
                        }],
                    })
                    .collect(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ProductionLineModifiers.index()] =
        TableView::Complete(TableRows::ProductionLineModifiers(vec![
            ProductionLineModifiersRow {
                line: 5,
                country: 9001,
                variant: 70,
                organisation: 79,
                kind: 0,
                active_factories: 1,
                factory_efficiencies: vec![3276800],
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            keys.iter()
                .map(|key| ModifierDefinitionRow {
                    index: *key,
                    token: key + 1,
                    ..Default::default()
                })
                .collect(),
        ));
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        panic!("tokens");
    };
    tokens.extend(keys.iter().map(|key| LexicalTokenRow {
        token: key + 1,
        literal: b"synthetic_renumbered_modifier".to_vec(),
    }));
    tokens.sort_by_key(|token| token.token);
    frame.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 1;
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.lines[0].runs[0].speed_raw, 310000);
    assert!(scanned.modifiers.is_empty());
    assert_eq!(scanned.modifier_layout_refusals.values().sum::<usize>(), 1);
    assert!(scanned.modifier_layout_refusals.keys().any(|reason| {
        reason.contains("line_change_production_efficiency_factor")
            && reason.contains("different script name")
    }));
}

#[test]
fn production_lines_keep_handles_across_switches_carry_output_and_end_at_deletions_gaps_and_new_identities()
 {
    use crate::products::production_lines::{LineKind, scan};
    // Line 5 (8 factories) runs from the first read; line 6 appears later; line 5
    // switches variant, line 6 is deleted and its final output is published.
    let a = (5, 70, 8, 1_736_704);
    let frame = |hour: i32| {
        let (germany, lines, output): (u32, Vec<LineFixture>, Vec<OutputFixture>) =
            match hour - FIRST {
                // A value seen off a read is not a read.
                2 => (9001, vec![(5, 70, 1, 0)], vec![(5, 70, 999, true, false)]),
                0..=23 => (9001, vec![a], vec![(5, 70, 0, true, false)]),
                24..=47 => (
                    9001,
                    vec![a, (6, 70, 2, 1_638_400)],
                    vec![(5, 70, 6, true, false), (6, 70, 2, true, false)],
                ),
                // Line 5 switches to variant 71; line 6's factories change.
                48..=71 => (
                    9001,
                    vec![(5, 71, 8, 1_769_472), (6, 70, 3, 1_638_400)],
                    vec![
                        (5, 70, 6, true, false),
                        (5, 71, 4, true, false),
                        (6, 70, 5, true, false),
                    ],
                ),
                // Line 6 is deleted: its final rows are published once. Hour 177 is missing.
                72 => (
                    9001,
                    vec![(5, 71, 8, 1_769_472)],
                    vec![
                        (5, 70, 6, true, false),
                        (5, 71, 8, true, false),
                        (6, 70, 7, true, true),
                    ],
                ),
                74 => (
                    9001,
                    vec![(5, 71, 8, 1_769_472)],
                    vec![(5, 70, 6, true, false), (5, 71, 12, true, false)],
                ),
                // Another country takes the tag: its lines are its own.
                _ => (
                    9002,
                    vec![(5, 71, 8, 1_769_472)],
                    vec![(5, 71, 0, true, false)],
                ),
            };
        let read = [
            FIRST,
            FIRST + 24,
            FIRST + 48,
            FIRST + 72,
            FIRST + 74,
            FIRST + 75,
        ]
        .contains(&hour);
        let variants = hour == FIRST || hour == FIRST + 74;
        production_frame(hour, read, variants, germany, &lines, Some(&output))
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            (FIRST..=FIRST + 75)
                .filter(|hour| *hour != FIRST + 73)
                .map(frame)
                .collect(),
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.reads,
        [
            FIRST,
            FIRST + 24,
            FIRST + 48,
            FIRST + 72,
            FIRST + 74,
            FIRST + 75
        ]
    );
    assert_eq!(
        scanned.lines.len(),
        3,
        "Italy's line is not Germany's; a new identity's lines are its own"
    );
    let five = &scanned.lines[0];
    assert_eq!(
        (
            five.key.as_str(),
            five.kind,
            five.before_recording,
            five.ended_by
        ),
        ("5#1", Some(LineKind::Military), true, None)
    );
    let runs: Vec<_> = five
        .runs
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.variant,
                run.efficiency_raw,
                run.max_factories,
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (FIRST, FIRST + 47, true, 70, 1736704, 150),
            (FIRST + 48, FIRST + 72, false, 71, 1769472, 150),
            // After the gap the line is the same, in a new stretch.
            (FIRST + 74, FIRST + 74, true, 71, 1769472, 150),
        ]
    );
    // Output is carried between reads, and ends at the gap.
    let output: Vec<_> = five
        .output
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.parts.iter().map(|part| part.units).collect::<Vec<_>>(),
                run.whole_life,
            )
        })
        .collect();
    assert_eq!(
        output,
        [
            (FIRST, FIRST + 23, vec![0], false),
            (FIRST + 24, FIRST + 47, vec![6], false),
            (FIRST + 48, FIRST + 71, vec![6, 4], false),
            (FIRST + 72, FIRST + 72, vec![6, 8], false),
            (FIRST + 74, FIRST + 74, vec![6, 12], false),
        ]
    );
    assert_eq!(five.output[0].since_hour, FIRST);
    let six = &scanned.lines[1];
    assert_eq!(
        (six.before_recording, six.ended_by),
        (false, Some(FIRST + 72))
    );
    assert_eq!(
        six.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.active_factories))
            .collect::<Vec<_>>(),
        [(FIRST + 24, FIRST + 47, 2), (FIRST + 48, FIRST + 71, 3)]
    );
    // The final output counts every unit, published when it ended; it covers its whole life.
    let last = six.final_output.as_ref().unwrap();
    assert_eq!(
        (last.first_hour, last.parts[0].units, last.whole_life),
        (FIRST + 72, 7, true)
    );
    let other = &scanned.lines[2];
    assert_eq!(
        (other.key.as_str(), other.runs[0].first_hour),
        ("5#2", FIRST + 75)
    );
    assert_eq!(
        scanned
            .output_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST, FIRST + 72), (FIRST + 74, FIRST + 75)]
    );
    assert_eq!(
        scanned.variants.current(71).unwrap().display_name,
        "Gewehr 98"
    );
    assert_eq!(
        scanned.organisations[&79][0].token,
        "GER_mauser_organization"
    );
    assert_eq!(
        scanned.organisations[&79][0].owner_tag.as_deref(),
        Some("GER")
    );
}

/// The case Equipment produced met: a census whose country links are incomplete
/// holds the one publication of a line's final count. The count is lost, so the
/// line's total after it ended is not recorded, never its last carried (too low)
/// count, and the hours are marked.
#[test]
fn a_final_count_in_a_skipped_census_is_not_recorded_never_a_low_total() {
    use crate::products::production_lines::{line_history, scan};
    let frame = |hour: i32| {
        let (lines, output): (&[LineFixture], &[OutputFixture]) = match hour - FIRST {
            // Line 6 runs to the census at 128, which publishes its final 9 units.
            ..24 => (
                &[(5, 70, 8, 0), (6, 70, 2, 0)],
                &[(5, 70, 0, true, false), (6, 70, 4, true, false)],
            ),
            24 => (
                &[(5, 70, 8, 0)],
                &[(5, 70, 6, true, false), (6, 70, 9, true, true)],
            ),
            _ => (&[(5, 70, 8, 0)], &[(5, 70, 12, true, false)]),
        };
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 24, FIRST + 48].contains(&hour),
            hour == FIRST,
            9001,
            lines,
            Some(output),
        );
        if hour == FIRST + 24 {
            *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 50).map(frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    let six = &scanned.lines[1];
    assert_eq!(six.key, "6#1");
    // Ended by the next census read; its final count, published only at 128, is lost.
    assert_eq!(
        (six.ended_by, six.final_output.as_ref()),
        (Some(FIRST + 48), None)
    );
    // Its last recorded count (4, at 104) holds only until the recording broke.
    assert_eq!(
        six.output
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.parts[0].units))
            .collect::<Vec<_>>(),
        [(FIRST, FIRST + 23, 4)]
    );
    // The reply has no final count to show as exact: "Made in all: Not recorded".
    assert!(line_history(six).final_output.is_none());
    assert_eq!(
        scanned
            .unread_output
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST + 1, FIRST + 24)]
    );
    // The line still running counts its own whole total again at the next read.
    assert_eq!(scanned.lines[0].output.last().unwrap().parts[0].units, 12);
}

/// A block of the recording that cannot be read is marked and counted: an output read
/// may lie in it.
#[test]
fn an_unreadable_block_is_marked_and_counted() {
    use crate::products::production_lines::scan;
    let frame = |hour: i32| {
        production_frame(
            hour,
            [FIRST, FIRST + 24, FIRST + 730, FIRST + 754].contains(&hour),
            hour == FIRST,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, 0, true, false)]),
        )
    };
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 800).map(frame).collect())],
    )
    .unwrap();
    // A block holds at most a keyframe interval (720 hours): 104–823, then 824–904.
    assert!(replay.index.len() >= 2, "the fixture spans several blocks");
    // The block holding the read at 834 can no longer be read.
    let broken = replay
        .index
        .iter()
        .position(|block| block.first_hour <= FIRST + 730 && FIRST + 730 <= block.last_hour)
        .unwrap();
    replay.index[broken].length = 1;
    let (first, last) = (
        replay.index[broken].first_hour,
        replay.index[broken].last_hour,
    );
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.unreadable_stretches, 1);
    assert!(
        scanned
            .unread_output
            .iter()
            .any(|span| span.first_hour <= first && last <= span.last_hour),
        "{:?} covers {first}..={last}",
        scanned.unread_output
    );
}

/// Rows whose producer is not a linked country are counted, never dropped silently;
/// one naming this country's current line stays its own, with a count not exact.
#[test]
fn rows_naming_an_unlinked_producer_are_counted_and_never_exact() {
    use crate::products::production_lines::scan;
    let frame = |hour: i32| {
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 24].contains(&hour),
            hour == FIRST,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, if hour >= FIRST + 24 { 6 } else { 0 }, true, false)]),
        );
        if hour == FIRST + 24 {
            // The recorder names a producer the country links do not hold.
            if let TableView::Complete(TableRows::ProductionLines(rows)) =
                &mut *frame.tables[TableId::ProductionLines.index()]
            {
                rows.iter_mut()
                    .filter(|row| row.handle == 5)
                    .for_each(|row| row.country = 77);
            }
            if let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
                &mut *frame.tables[TableId::ProductionLineOutput.index()]
            {
                rows.iter_mut().for_each(|row| row.country = 77);
            }
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 30).map(frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    // The line is not ended by the unlinked row: it is still line 5's one appearance.
    assert_eq!(scanned.lines.len(), 1);
    assert_eq!(scanned.lines[0].ended_by, None);
    assert_eq!(scanned.lines[0].runs.last().unwrap().last_hour, FIRST + 30);
    // Its count at 128 is kept, but not exact; the rows are counted.
    let at_124 = scanned.lines[0].output.last().unwrap();
    assert_eq!(
        (
            at_124.first_hour,
            at_124.parts[0].units,
            at_124.parts[0].exact
        ),
        (FIRST + 24, 6, false)
    );
    assert_eq!(scanned.unlinked_rows, 2);
}

/// A due output read that cannot be read (here the country links are incomplete, so
/// the census and its output are skipped) is never dropped silently: it is counted,
/// and the hours since the read before are marked, since a line made and ended then
/// published its only rows there. A country that does not exist has nothing to miss.
#[test]
fn an_unreadable_output_read_is_counted_and_its_hours_marked() {
    use crate::products::{
        LoadContext, Product,
        production_lines::{ProductionLines, ProductionLinesParams, scan},
    };
    let frame = |hour: i32| {
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 24, FIRST + 48].contains(&hour),
            hour == FIRST,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, 0, true, false)]),
        );
        if hour == FIRST + 24 {
            *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, (FIRST..=FIRST + 50).map(frame).collect())],
        )
        .unwrap(),
    );
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [FIRST, FIRST + 48]);
    assert_eq!(scanned.unread_output_reads, 1);
    assert_eq!(
        scanned
            .unread_output
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST + 1, FIRST + 24)]
    );
    // A tag no country holds is not missing lines where the countries were read (104,
    // 152); only at 128, where whether it existed cannot be told, is the read counted.
    let absent = scan(&replay, 1, "XXX", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(absent.unread_output_reads, 1);
    // The reply warns, counted, and carries the hours for the history.
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "production-unread".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ProductionLines::load(
        &source,
        &ProductionLinesParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            exact: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .unread_output
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST + 1, FIRST + 24)]
    );
    assert!(
        reply.warnings.iter().any(|warning| warning
            .starts_with("1 daily production output reads of GER could not be read")),
        "{:?}",
        reply.warnings
    );
}

#[test]
fn production_output_stops_at_the_first_unavailable_hour_and_lines_between_reads_are_kept() {
    use crate::products::production_lines::scan;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                production_frame(
                    FIRST,
                    true,
                    true,
                    9001,
                    &[(5, 70, 8, 0)],
                    Some(&[(5, 70, 3, false, false), (8, 70, 1, true, false)]),
                ),
                // Readable, then unavailable while the census is still carried.
                production_frame(
                    FIRST + 1,
                    false,
                    false,
                    9001,
                    &[(5, 70, 8, 0)],
                    Some(&[(5, 70, 3, false, false)]),
                ),
                production_frame(FIRST + 2, false, false, 9001, &[(5, 70, 8, 0)], None),
                production_frame(
                    FIRST + 3,
                    false,
                    false,
                    9001,
                    &[(5, 70, 8, 0)],
                    Some(&[(5, 70, 3, false, false)]),
                ),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    let line = &scanned.lines[0];
    // The inexact part stays marked; the output read at 104 holds through 105 and stops
    // at the first unavailable hour, never resuming from a carried value.
    assert_eq!(line.output.len(), 1);
    assert_eq!(
        (line.output[0].last_hour, line.output[0].parts[0].exact),
        (FIRST + 1, false)
    );
    assert_eq!(
        scanned
            .output_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST, FIRST + 1)]
    );
    // The lines themselves stay known while the census is carried.
    assert_eq!(
        scanned
            .line_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST, FIRST + 3)]
    );
    // Line 8 has output that is not a final count and no census: left out, counted.
    assert_eq!(scanned.unlisted_output, 1);
}

/// A variant a line made and deleted before a census produced, whose definition is
/// read only after that census, gets its icon once the definition arrives.
#[test]
fn a_variant_defined_after_its_census_gets_its_icon_when_its_definition_arrives() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{
        equipment_icon::{Icon, IconContent},
        production_lines::scan,
    };
    use installed_content_core::technology::Enablers;
    use installed_content_equipment_graphics::equipment_graphics;
    let frame = |hour: i32| {
        let output: &[OutputFixture] = if hour == FIRST + 24 {
            &[(5, 70, 6, true, false), (9, 71, 4, true, true)]
        } else {
            &[(5, 70, 0, true, false)]
        };
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 24].contains(&hour),
            [FIRST, FIRST + 30].contains(&hour),
            9001,
            &[(5, 70, 8, 0)],
            Some(output),
        );
        // Variant 71 is defined only from 134; every icon entry is inactive, so the
        // rule falls back to the type's sprite.
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        {
            rows.retain(|row| row.id != 71 || hour >= FIRST + 30);
            for row in rows.iter_mut() {
                row.icon_active = 0;
            }
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 40).map(frame).collect())],
    )
    .unwrap();
    let content = IconContent::from_parts(
        equipment_types(&[br#"equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment }
        }"#])
        .unwrap(),
        Enablers::default(),
        equipment_graphics::Content::from_texts(
            &[],
            &equipment_graphics::Names {
                countries: ["GER".to_owned()].into(),
                continents: Default::default(),
            },
        )
        .unwrap(),
        Default::default(),
        ["GFX_infantry_equipment_1_medium".to_owned()].into(),
        Default::default(),
    );
    let scanned = scan(&replay, 1, "GER", Some(&content), &AtomicBool::new(false)).unwrap();
    let icons: Vec<(i32, Result<Option<Icon>, String>)> = scanned.icons[&71]
        .iter()
        .map(|(hour, choice)| (*hour, choice.walk(|probe| content.answer(probe))))
        .collect();
    assert_eq!(
        icons,
        [
            (
                FIRST + 24,
                Err("the variant's definition is not recorded yet".to_owned())
            ),
            (
                FIRST + 30,
                Ok(Some(Icon::Sprite(
                    "GFX_infantry_equipment_1_medium".to_owned()
                )))
            ),
        ]
    );
    // The reply's entries give the art from 134.
    let (entries, _, _) = crate::products::production_lines::variant_entries(
        &scanned,
        None,
        true,
        &mut |choice| answered(&content, choice),
    );
    let art: Vec<_> = entries
        .iter()
        .filter(|entry| entry.id == "71")
        .map(|entry| (entry.first_hour, entry.icon.clone()))
        .collect();
    assert_eq!(
        art,
        [(
            FIRST + 30,
            Some("sprite:GFX_infantry_equipment_1_medium".to_owned())
        )]
    );
}

/// A running line's variant whose definition changes at a variant read (a new icon
/// entry) gets its new icon at that read, not at the next census.
#[test]
fn a_known_variants_icon_follows_a_changed_definition_at_its_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{
        equipment_icon::{Icon, IconContent},
        production_lines::scan,
    };
    use installed_content_core::technology::Enablers;
    use installed_content_equipment_graphics::equipment_graphics;
    let frame = |hour: i32| {
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 48].contains(&hour),
            [FIRST, FIRST + 30].contains(&hour),
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, 0, true, false)]),
        );
        // Variant 70's own sprite entry: A until the read at 134, then B.
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        {
            for row in rows.iter_mut() {
                row.icon_active = 1;
                row.icon_kind = 0;
                row.icon = if hour < FIRST + 30 {
                    b"GFX_A".to_vec()
                } else {
                    b"GFX_B".to_vec()
                };
            }
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 50).map(frame).collect())],
    )
    .unwrap();
    let content = IconContent::from_parts(
        equipment_types(&[br#"equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment }
        }"#])
        .unwrap(),
        Enablers::default(),
        equipment_graphics::Content::from_texts(
            &[],
            &equipment_graphics::Names {
                countries: ["GER".to_owned()].into(),
                continents: Default::default(),
            },
        )
        .unwrap(),
        Default::default(),
        ["GFX_A".to_owned(), "GFX_B".to_owned()].into(),
        Default::default(),
    );
    let scanned = scan(&replay, 1, "GER", Some(&content), &AtomicBool::new(false)).unwrap();
    let sprite = |name: &str| Ok(Some(Icon::Sprite(name.to_owned())));
    let icons: Vec<_> = scanned.icons[&70]
        .iter()
        .map(|(hour, choice)| (*hour, choice.walk(|probe| content.answer(probe))))
        .collect();
    assert_eq!(
        icons,
        [(FIRST, sprite("GFX_A")), (FIRST + 30, sprite("GFX_B"))]
    );
    // A variant no line used is not looked up.
    assert!(!scanned.icons.contains_key(&71));
}

#[test]
fn a_line_made_and_deleted_between_reads_is_shown_from_its_final_output() {
    use crate::products::production_lines::scan;
    let frame = |hour: i32| {
        let (read, output): (bool, &[OutputFixture]) = match hour - FIRST {
            0 => (true, &[(5, 70, 0, true, false)]),
            24 => (
                true,
                &[
                    (5, 70, 6, true, false),
                    (9, 71, 4, true, true),
                    (9, 70, 1, true, true),
                ],
            ),
            _ => (false, &[(5, 70, 0, true, false)]),
        };
        production_frame(
            hour,
            read,
            hour == FIRST,
            9001,
            &[(5, 70, 8, 0)],
            Some(output),
        )
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 24).map(frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.lines.len(), 2);
    let between = &scanned.lines[1];
    // No census listed it: no class, no runs; it was made and deleted within 105–128,
    // producing variants 71 and 70, 5 units in all, counted over its whole life.
    assert_eq!(
        (
            between.key.as_str(),
            between.kind,
            between.runs.len(),
            between.ended_by
        ),
        ("9#1", None, 0, Some(FIRST + 24))
    );
    let span = between.between_reads.as_ref().unwrap();
    assert_eq!((span.first_hour, span.last_hour), (FIRST + 1, FIRST + 24));
    assert_eq!(between.variants, [71, 70]);
    let last = between.final_output.as_ref().unwrap();
    assert_eq!(
        last.parts.iter().map(|part| part.units).collect::<Vec<_>>(),
        [4, 1]
    );
    assert!(last.whole_life);
    assert_eq!(scanned.unlisted_output, 0);
}

#[test]
fn a_variant_is_named_only_from_its_first_definition_read_onwards() {
    use crate::products::{
        equipment::{RawVariant, VariantVersion},
        equipment_icon::Icon,
        production_lines::{LineKind, RawRun, ScannedLine, ScannedLines, variant_entries},
    };
    let raw = |hour: i32, name: &str| RawVariant {
        first_hour: hour,
        token: "infantry_equipment_1".into(),
        display_name: name.into(),
        ..Default::default()
    };
    let mut scanned = ScannedLines::default();
    scanned.lines.push(ScannedLine {
        key: "5#1".into(),
        handle: 5,
        kind: Some(LineKind::Military),
        runs: vec![RawRun {
            first_hour: FIRST,
            last_hour: 200,
            variant: 71,
            ..Default::default()
        }],
        output: vec![],
        final_output: None,
        ended_by: None,
        before_recording: true,
        between_reads: None,
        variants: vec![],
    });
    // Defined (and later renamed) only from hour 128; its icon was looked at from 104.
    scanned.variants.versions.insert(
        71,
        [raw(FIRST + 24, "Gewehr 98"), raw(FIRST + 48, "Gewehr 98k")]
            .map(|raw| VariantVersion {
                first_hour: raw.first_hour,
                raw: Some(raw),
            })
            .to_vec(),
    );
    scanned.icons.insert(
        71,
        vec![
            (
                FIRST,
                IconChoice::unknown("the variant's definition is not recorded yet".into()),
            ),
            (
                FIRST + 24,
                IconChoice {
                    probes: vec![Probe::Sprite("GFX_rifle".into())],
                    otherwise: Ok(()),
                },
            ),
        ],
    );
    let mut texts = installed_content_localisation::localisation::Localisation::default();
    texts.language = "english".into();
    // Every sprite declared.
    let (entries, unnamed, _) = variant_entries(&scanned, Some(&texts), true, &mut |choice| {
        Ok(choice
            .walk(|probe| match probe {
                Probe::Sprite(name) => Ok(Some(Icon::Sprite(name.clone()))),
                _ => Ok(None),
            })?
            .map(|icon| icon_key(&icon)))
    });
    assert_eq!(unnamed, 0);
    // Nothing before hour 128: the hour-104 look is unknown, never the later name.
    assert_eq!(
        entries
            .iter()
            .map(|entry| (
                entry.first_hour,
                entry.name.as_deref(),
                entry.icon.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            (FIRST + 24, Some("Gewehr 98"), Some("sprite:GFX_rifle")),
            (FIRST + 48, Some("Gewehr 98k"), Some("sprite:GFX_rifle"))
        ]
    );
}

#[test]
fn organisations_are_named_and_pictured_as_the_game_does() {
    use crate::products::{
        equipment::Texts,
        production_lines::{RawOrganisation, organisation_icon, organisation_name},
    };
    struct Map(&'static [(&'static str, &'static str)]);
    impl Texts for Map {
        fn has(&self, key: &str) -> bool {
            self.0.iter().any(|(k, _)| *k == key)
        }
        fn text(&self, key: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }
    let texts = Map(&[
        ("mauser", "Mauser"),
        ("GER_mauser", "Mauserwerke"),
        ("renamed_key", "Renamed"),
    ]);
    let raw = RawOrganisation {
        token: "mauser".into(),
        owner_tag: Some("GER".into()),
        ..Default::default()
    };
    // The owner-prefixed key first; an override replaces the script key.
    assert_eq!(
        organisation_name(&raw, Some(&texts)).as_deref(),
        Some("Mauserwerke")
    );
    let renamed = RawOrganisation {
        name_key: "renamed_key".into(),
        ..raw.clone()
    };
    assert_eq!(
        organisation_name(&renamed, Some(&texts)).as_deref(),
        Some("Renamed")
    );
    assert_eq!(organisation_name(&raw, None), None);
    // The stored sprite, else the tag-prefixed and plain script key, else the unknown sprite.
    let stored = RawOrganisation {
        icon: "GFX_idea_mauser".into(),
        ..raw.clone()
    };
    assert_eq!(
        organisation_icon(&stored, |_| true).as_deref(),
        Some("GFX_idea_mauser")
    );
    assert_eq!(
        organisation_icon(&raw, |name| name == "GFX_mauser").as_deref(),
        Some("GFX_mauser")
    );
    assert_eq!(
        organisation_icon(&raw, |name| name.starts_with("GFX_GER")).as_deref(),
        Some("GFX_GER_mauser")
    );
    assert_eq!(
        organisation_icon(&stored, |name| name == "GFX_idea_unknown").as_deref(),
        Some("GFX_idea_unknown")
    );
}

#[test]
fn production_lines_reply_without_content_names_only_designs_and_answers_again_from_the_cache() {
    use crate::products::{
        LoadContext, Product,
        equipment::EQUIPMENT_DECIMALS,
        production_lines::{
            DETAIL_HOURS, ProductionDetailParams, ProductionLineDetail, ProductionLines,
            ProductionLinesParams,
        },
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    production_frame(
                        FIRST,
                        true,
                        true,
                        9001,
                        &[(5, 71, 8, 0), (6, 70, 1, 0)],
                        Some(&[(5, 71, 3, true, false), (6, 70, 1, true, false)]),
                    ),
                    production_frame(
                        FIRST + 1,
                        false,
                        false,
                        9001,
                        &[(5, 71, 8, 0), (6, 70, 1, 0)],
                        Some(&[]),
                    ),
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "production".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = ProductionLinesParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        exact: None,
    };
    let load = |params: &ProductionLinesParams| {
        ProductionLines::load(
            &source,
            params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load(&params);
    assert_eq!(reply.decimals, EQUIPMENT_DECIMALS);
    assert_eq!(reply.lines.len(), 2);
    // The history holds each line's steps; the values are in its detail window.
    assert_eq!(
        (
            reply.lines[0].steps.len(),
            reply.lines[0].steps[0].last_hour,
            reply.lines[0].steps[0].active_factories
        ),
        (1, FIRST + 1, 8)
    );
    assert_eq!(reply.detail_hours, DETAIL_HOURS);
    let detail = |window: i32| {
        ProductionLineDetail::load(
            &source,
            &ProductionDetailParams {
                branch_id: "1".into(),
                tag: "GER".into(),
                window,
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let window = detail(FIRST / DETAIL_HOURS);
    assert_eq!((window.first_hour, window.last_hour), (0, DETAIL_HOURS - 1));
    assert_eq!(window.lines[0].key, reply.lines[0].key);
    assert_eq!(window.lines[0].runs[0].last_hour, FIRST + 1);
    assert_eq!(window.lines[0].runs[0].active_factories, 8);
    assert_eq!(window.lines[0].output[0].parts[0].units, "3");
    // A window without reads holds no lines.
    assert!(detail(5).lines.is_empty());
    assert!(
        ProductionLineDetail::validate(&ProductionDetailParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            window: i32::MAX,
        })
        .is_err()
    );
    let names: Vec<_> = reply
        .variants
        .iter()
        .map(|variant| {
            (
                variant.id.as_str(),
                variant.token.as_str(),
                variant.name.as_deref(),
                variant.icon_unknown.is_some(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("70", "infantry_equipment_1", None, true),
            ("71", "infantry_equipment_1", None, true)
        ]
    );
    assert_eq!(
        (
            reply.organisations.len(),
            reply.organisations[0].name.as_deref(),
            reply.organisations[0].icon.as_deref()
        ),
        (1, None, None)
    );
    assert!(reply.icons.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Equipment names and icons are unavailable"))
    );
    assert_eq!(load(&params).lines, reply.lines);
    let other = load(&ProductionLinesParams {
        tag: "XXX".into(),
        ..params
    });
    assert!(other.reads.is_empty() && other.lines.is_empty());
    assert!(
        other
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX:"))
    );
}

#[test]
fn a_line_ended_before_the_first_complete_census_is_not_given_an_invented_creation() {
    use crate::products::production_lines::scan;
    // Recording starts at 104 with the census unavailable; the first complete census at
    // 112 publishes line 5's ended output, not observed since its creation.
    let frame = |hour: i32| {
        let mut frame = production_frame(
            hour,
            true,
            hour == FIRST,
            9001,
            &[],
            Some(&[(5, 70, 3, true, true)]),
        );
        if hour < FIRST + 8 {
            *frame.tables[TableId::ProductionLines.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 8).map(frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [FIRST + 8]);
    let line = &scanned.lines[0];
    let span = line.between_reads.as_ref().unwrap();
    // It ran from before the observation (since hour 104) and ended by 112.
    assert_eq!(
        (
            line.before_recording,
            span.first_hour,
            span.last_hour,
            line.ended_by
        ),
        (true, FIRST, FIRST + 8, Some(FIRST + 8))
    );
    assert!(!line.final_output.as_ref().unwrap().whole_life);
}

/// A line whose efficiency and output change at every daily read is one step in
/// the history, whatever the campaign's length; its reads come a window at a time,
/// a read holding into a window sent with it.
#[test]
fn production_history_is_one_step_per_setting_and_its_reads_come_in_windows() {
    use crate::products::production_lines::{DETAIL_HOURS, detail_lines, line_history, scan};
    let days = 60;
    let frames: Vec<_> = (0..days * 24)
        .map(|offset| {
            let day = i64::from(offset / 24);
            production_frame(
                FIRST + offset,
                offset % 24 == 0,
                offset == 0,
                9001,
                &[(5, 70, 8, 1_000_000 + day * 1000)],
                Some(&[(5, 70, day, true, false)]),
            )
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    let line = &scanned.lines[0];
    assert_eq!((line.runs.len(), line.output.len()), (60, 60));
    let history = line_history(line);
    assert_eq!(
        history
            .steps
            .iter()
            .map(|step| (step.first_hour, step.last_hour, step.active_factories))
            .collect::<Vec<_>>(),
        [(FIRST, FIRST + days * 24 - 1, 8)]
    );
    // Reads at 104 + 24d: the first window (0 to 719) holds days 0 to 25, the second
    // (720 to 1439) days 25 to 55 and the third days 55 to 59; day 25's read, at 704,
    // holds to 723, and day 55's, at 1424, to 1447.
    let window = |index: i32| {
        detail_lines(
            &scanned,
            index * DETAIL_HOURS,
            (index + 1) * DETAIL_HOURS - 1,
        )
    };
    let (first, second, third) = (window(0), window(1), window(2));
    assert_eq!((first[0].runs.len(), first[0].output.len()), (26, 26));
    assert_eq!((second[0].runs.len(), second[0].output.len()), (31, 31));
    assert_eq!((third[0].runs.len(), third[0].output.len()), (5, 5));
    assert_eq!(
        (
            second[0].runs[0].first_hour,
            second[0].runs[0].efficiency_raw.as_str()
        ),
        (FIRST + 600, "1025000")
    );
    assert!(window(3).is_empty());
}

/// A month window sends each line's recorded values, never its modifier trees: a
/// tree lists every source of every factor at each read, so with many lines it
/// would take a month past one reply. Its factors come from the line's breakdown.
#[test]
fn a_detail_window_does_not_grow_with_modifier_trees() {
    use crate::products::production_lines::{
        DETAIL_HOURS, MAX_REPLY_BYTES, RawRun, ScannedLine, ScannedLines,
        breakdown::RawModifierRun, detail_lines,
    };
    use replay_data::{
        rows::CountryProductionModifiersRow, rows::ModifierBreakdownNode,
        rows::ProductionLineModifiersRow, rows::ProductionModifierTree,
    };
    let runs: Vec<RawRun> = (0..30)
        .map(|day| RawRun {
            first_hour: day * 24,
            last_hour: day * 24 + 23,
            starts_segment: day == 0,
            variant: 70,
            active_factories: 8,
            requested_factories: 8,
            max_factories: 8,
            cost_raw: 500_000,
            speed_raw: 310_000,
            efficiency_raw: 1_000_000 + i64::from(day) * 1000,
            amount: -1,
            ..RawRun::default()
        })
        .collect();
    let mut scanned = ScannedLines {
        lines: (0..300_u64)
            .map(|handle| ScannedLine {
                key: format!("{handle}#1"),
                handle,
                kind: None,
                runs: runs.clone(),
                output: Vec::new(),
                final_output: None,
                ended_by: None,
                before_recording: false,
                between_reads: None,
                variants: Vec::new(),
            })
            .collect(),
        ..ScannedLines::default()
    };
    let without = detail_lines(&scanned, 0, DETAIL_HOURS - 1);
    // Zero-valued sources under each factor key, as a country's trees list them.
    let nodes: Vec<ModifierBreakdownNode> = std::iter::once(ModifierBreakdownNode {
        children: 20,
        value: 0,
        name: b"Synthetic total".to_vec(),
    })
    .chain((0..20).map(|index| ModifierBreakdownNode {
        children: 0,
        value: 0,
        name: format!("Synthetic source {index}").into_bytes(),
    }))
    .collect();
    let country = Arc::new(CountryProductionModifiersRow {
        country: 9001,
        trees: [0xa8, 0xa9, 0xab, 0xdc]
            .into_iter()
            .map(|key| ProductionModifierTree {
                key,
                nodes: nodes.clone(),
                failure: vec![],
            })
            .collect(),
        ..Default::default()
    });
    for line in scanned.lines.iter().take(5) {
        let trees: Vec<RawModifierRun> = line
            .runs
            .iter()
            .map(|run| RawModifierRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                country: Arc::clone(&country),
                line: ProductionLineModifiersRow {
                    line: line.handle,
                    country: 9001,
                    variant: 70,
                    active_factories: 8,
                    factory_efficiencies: vec![run.efficiency_raw; 8],
                    ..Default::default()
                },
                run: *run,
                organisation: None,
                localisation: None,
                recorded_language: Arc::from("english"),
            })
            .collect();
        scanned.modifiers.insert(line.key.clone(), trees);
    }
    let with = detail_lines(&scanned, 0, DETAIL_HOURS - 1);
    assert_eq!(with, without);
    assert_eq!(with.len(), 300);
    assert!(with.iter().all(|line| line.runs.len() == 30));
    assert!(crate::transport::json_len(&with, MAX_REPLY_BYTES).is_some());
    assert!(
        !serde_json::to_string(&with)
            .unwrap()
            .contains("Synthetic source")
    );
}

/// A variant's definitions are known only where their table is read or carried: a
/// census while the variant table is unavailable does not carry the last name or
/// icon over an unobserved change.
#[test]
fn variants_are_known_only_where_their_table_is_read_and_carried() {
    use crate::products::production_lines::scan;
    let frame = |hour: i32| {
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 24].contains(&hour),
            [FIRST, FIRST + 48].contains(&hour),
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, 0, true, false)]),
        );
        if (FIRST + 24..FIRST + 48).contains(&hour) {
            *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 50).map(frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    // A read of the variant slot is timed with the production slot's.
    assert_eq!(scanned.reads, [FIRST, FIRST + 24, FIRST + 48]);
    // The shared catalogue: known from 104, unknown from 128 (the table carried then
    // was not complete), known again from the read at 152.
    let versions: Vec<(i32, bool)> = scanned.variants.versions[&70]
        .iter()
        .map(|version| (version.first_hour, version.raw.is_some()))
        .collect();
    assert_eq!(
        versions,
        [(FIRST, true), (FIRST + 24, false), (FIRST + 48, true)]
    );
    // The reply's entries say so, so a census at 128 does not carry the old name.
    let (entries, _, _) = crate::products::production_lines::variant_entries(
        &scanned,
        None,
        false,
        &mut |_| Err("no choice is answered".into()),
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.id == "70")
            .map(|entry| (entry.first_hour, entry.known))
            .collect::<Vec<_>>(),
        [(FIRST, true), (FIRST + 24, false), (FIRST + 48, true)]
    );
}

#[test]
fn organisations_are_known_only_where_their_table_is_read_and_carried() {
    use crate::products::production_lines::scan;
    let frame = |hour: i32| {
        let mut frame = production_frame(
            hour,
            [FIRST, FIRST + 24].contains(&hour),
            hour == FIRST,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, 0, true, false)]),
        );
        if (FIRST + 2..FIRST + 24).contains(&hour) {
            *frame.tables[TableId::IndustrialOrganisations.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..=FIRST + 24).map(frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    // Read at 104 and carried to 105; unknown from 106 until the next read at 128.
    assert_eq!(
        scanned
            .organisation_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST, FIRST + 1), (FIRST + 24, FIRST + 24)]
    );
}

#[test]
fn a_variant_made_only_between_two_censuses_is_named_in_the_reply() {
    use crate::products::{
        LoadContext, Product,
        production_lines::{ProductionLines, ProductionLinesParams},
    };
    // Line 5 shows variant 70 at both censuses but made some of variant 71 in between.
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    production_frame(
                        FIRST,
                        true,
                        true,
                        9001,
                        &[(5, 70, 8, 0)],
                        Some(&[(5, 70, 0, true, false)]),
                    ),
                    production_frame(
                        FIRST + 1,
                        true,
                        false,
                        9001,
                        &[(5, 70, 8, 0)],
                        Some(&[(5, 70, 2, true, false), (5, 71, 3, true, false)]),
                    ),
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "production-parts".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ProductionLines::load(
        &source,
        &ProductionLinesParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            exact: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply.lines[0]
            .steps
            .iter()
            .map(|step| step.variant.as_str())
            .collect::<Vec<_>>(),
        ["70"]
    );
    let named: Vec<_> = reply
        .variants
        .iter()
        .map(|variant| (variant.id.as_str(), variant.name.as_deref()))
        .collect();
    assert!(named.contains(&("71", None)), "{named:?}");
}

#[test]
fn unresolved_country_identity_marks_lost_between_census_output_and_warns() {
    use crate::products::{
        LoadContext, Product,
        production_lines::{ProductionLines, ProductionLinesParams, scan},
    };
    let frames = (FIRST..=FIRST + 24)
        .map(|hour| {
            let output: &[OutputFixture] = if hour == FIRST + 24 {
                &[(9, 71, 4, true, true)]
            } else {
                &[]
            };
            let mut frame = production_frame(
                hour,
                hour == FIRST || hour == FIRST + 24,
                hour == FIRST,
                9001,
                &[],
                Some(output),
            );
            if hour == FIRST + 24 {
                // Both tables complete: GER exists, but none of its identities resolves.
                *frame.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                        id: 21,
                        canonical_slot: 0,
                    }]));
            }
            frame
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [FIRST]);
    assert!(
        scanned.lines.is_empty(),
        "the between-census line's only publication cannot be attributed"
    );
    assert_eq!(scanned.unread_output_reads, 1);
    assert_eq!(
        scanned
            .unread_output
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(FIRST + 1, FIRST + 24)]
    );
    let absent = scan(&replay, 1, "XXX", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        absent.unread_output_reads, 0,
        "a known absent country has no output to lose"
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "unresolved-production".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ProductionLines::load(
        &source,
        &ProductionLinesParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            exact: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        reply.warnings.iter().any(|warning| warning
            .starts_with("1 daily production output reads of GER could not be read")),
        "{:?}",
        reply.warnings
    );
    assert_eq!(reply.unread_output.len(), 1);
}

#[test]
fn unresolved_organisation_owner_never_selects_generic_name_or_icon() {
    use crate::products::{
        LoadContext, Product,
        equipment::Texts,
        production_lines::{
            ProductionLines, ProductionLinesParams, RawOrganisation, organisation_icon,
            organisation_name, scan,
        },
    };
    struct Names;
    impl Texts for Names {
        fn has(&self, key: &str) -> bool {
            ["AAA_acme", "acme"].contains(&key)
        }
        fn text(&self, key: &str) -> Option<String> {
            match key {
                "AAA_acme" => Some("AAA Acme".into()),
                "acme" => Some("Generic Acme".into()),
                _ => None,
            }
        }
    }
    let mut frame = production_frame(FIRST, true, true, 9001, &[(5, 70, 8, 0)], Some(&[]));
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            replay_data::rows::IndustrialOrganisationRow {
                organisation: 79,
                country: 11,
                owner: 333,
                token: 602,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay_data::rows::LexicalTokenRow {
                token: 602,
                literal: b"acme".to_vec(),
            },
        ]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    let raw = &scanned.organisations[&79][0];
    assert!(raw.owner_unknown);
    assert_eq!(raw.owner_tag, None);
    let exists = |name: &str| ["GFX_AAA_acme", "GFX_acme", "GFX_explicit"].contains(&name);
    assert_eq!(organisation_name(raw, Some(&Names)), None);
    assert_eq!(organisation_icon(raw, exists), None);
    // A stored sprite decides the icon before ownership is needed.
    assert_eq!(
        organisation_icon(
            &RawOrganisation {
                icon: "GFX_explicit".into(),
                ..raw.clone()
            },
            exists
        )
        .as_deref(),
        Some("GFX_explicit")
    );
    // A known absent owner can use the generic candidates.
    let ownerless = RawOrganisation {
        owner_unknown: false,
        ..raw.clone()
    };
    assert_eq!(
        organisation_name(&ownerless, Some(&Names)).as_deref(),
        Some("Generic Acme")
    );
    assert_eq!(
        organisation_icon(&ownerless, exists).as_deref(),
        Some("GFX_acme")
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "unknown-organisation".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ProductionLines::load(
        &source,
        &ProductionLinesParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            exact: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    let organisation = &reply.organisations[0];
    assert_eq!(organisation.name, None);
    assert_eq!(organisation.icon, None);
    assert!(
        organisation
            .name_unknown
            .as_deref()
            .unwrap()
            .contains("owner is unresolved")
    );
    assert!(
        organisation
            .icon_unknown
            .as_deref()
            .unwrap()
            .contains("owner is unresolved")
    );
}

/// A scan is found only by the content it was made with, the `Arc` itself: other
/// content, even equal and allocated after the first is dropped by its caller,
/// never matches, since the key keeps the first alive.
#[test]
fn scans_are_keyed_by_the_content_they_were_made_with() {
    let first = Arc::new(1_u8);
    let key: ScanKey<u8> = (1, "GER".into(), Some(Arc::clone(&first)), None);
    drop(first);
    let other = Arc::new(1_u8);
    assert!(same_scan(&key, &key.clone()));
    assert!(!same_scan(&key, &(1, "GER".into(), Some(other), None)));
    assert!(!same_scan(&key, &(1, "GER".into(), None, None)));
    assert!(!same_scan(&key, &(1, "ITA".into(), key.2.clone(), None)));
    assert!(!same_scan(
        &key,
        &(1, "GER".into(), key.2.clone(), Some((7, 9001)))
    ));
    assert!(same_scan::<u8>(
        &(1, "GER".into(), None, None),
        &(1, "GER".into(), None, None)
    ));
}

/// An organisation picture drawn through one catalogue is never served for another: a reply
/// showing the mods' catalogue draws the mods' picture, and one showing the recording's again
/// draws the recording's, never the other's kept one.
#[test]
fn organisation_pictures_are_served_only_for_the_catalogue_they_were_drawn_through() {
    use crate::test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select};
    let machine = Machine::new("organisation_picture_owner");
    machine.write(
        "interface/organisations.gfx",
        b"spriteTypes = { spriteType = { name = GFX_tok textureFile = gfx/tok.png } }",
    );
    machine.write("gfx/tok.png", &png(RED));
    machine.write_mod("art", "gfx/tok.png", &png(BLUE));
    let raw = RawOrganisation {
        first_hour: 0,
        token: "tok".into(),
        name_key: String::new(),
        icon: String::new(),
        owner_tag: None,
        owner_unknown: false,
        state: None,
    };
    let (source, mut options) = machine.open();
    let state = source.state::<Cache>();
    let shown = |options: &crate::products::ContentOptions| {
        let catalogue = catalogue::catalogue(&source, options).unwrap();
        let (_, image) =
            kept_organisation_picture(&state, &catalogue, &raw, &mut Vec::new()).unwrap();
        first_pixel(&image.unwrap())
    };
    select(&source, &mut options, &[]);
    assert_eq!(shown(&options), RED);
    select(&source, &mut options, &["art"]);
    assert_eq!(shown(&options), BLUE);
    select(&source, &mut options, &[]);
    assert_eq!(shown(&options), RED);
}

#[test]
fn organisation_icon_refuses_unread_token_after_independent_stored_candidate() {
    use super::{RawOrganisation, organisation_icon_with_token};
    let raw = RawOrganisation {
        token: String::new(),
        owner_tag: Some("MOD".into()),
        icon: "GFX_stored".into(),
        ..Default::default()
    };
    assert_eq!(
        organisation_icon_with_token(&raw, false, |name| name == "GFX_stored"),
        Ok(Some("GFX_stored".into()))
    );
    assert_eq!(
        organisation_icon_with_token(&raw, false, |name| name == UNKNOWN_ORGANISATION_ICON),
        Err(
            "LexicalTokens does not spell the recorded organisation token for fallback icon selection"
        )
    );
    // Known empty token remains a native spelling, and may select the generic fallback.
    assert_eq!(
        organisation_icon_with_token(&raw, true, |name| name == UNKNOWN_ORGANISATION_ICON),
        Ok(Some(UNKNOWN_ORGANISATION_ICON.into()))
    );
}

/// The blocks a load reads, as `scan_blocks` counts them on this thread.
fn blocks_read<T>(load: impl FnOnce() -> T) -> (T, Vec<u64>) {
    crate::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let value = load();
    (
        value,
        crate::blocks::SCANNED_BLOCKS.with_borrow(Clone::clone),
    )
}

/// The widget's default request reads a fixed number of the branch's newest blocks
/// however long the recording is, and says which hours it does not answer exactly.
/// A request with `exact` reads every block once and answers what the whole scan
/// answers. Each reply is its request's: held scans answer both again, reading nothing.
#[test]
fn a_first_view_reads_a_bounded_number_of_newest_blocks_whatever_the_recording_length() {
    use crate::{
        products::{
            LoadContext, Product,
            country_groups::HourSpan,
            production_lines::{
                FIRST_VIEW_BLOCKS, ProductionLines, ProductionLinesParams, line_history, scan,
            },
        },
        test_fixtures::continuation_records,
    };
    let bound = FIRST_VIEW_BLOCKS as i32;
    // Every hour is a read of Germany's lines; line 6 ends halfway.
    let frame = |hour: i32, length: i32| {
        let units = i64::from(hour - FIRST);
        let (lines, output): (Vec<LineFixture>, Vec<OutputFixture>) = if hour - FIRST < length / 2 {
            (
                vec![(5, 70, 8, 0), (6, 71, 2, 0)],
                vec![(5, 70, units, true, false), (6, 71, units, true, false)],
            )
        } else {
            (vec![(5, 70, 8, 0)], vec![(5, 70, units, true, false)])
        };
        production_frame(hour, true, true, 9001, &lines, Some(&output))
    };
    for length in [2 * bound, 10 * bound] {
        // Each appended segment starts a keyframe, so each hour is its own block.
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(FIRST, length)])])
                .unwrap();
        for hour in FIRST + 1..FIRST + length {
            replay = replay
                .append(&[continuation_records(&replay, 1, vec![frame(hour, length)])])
                .unwrap();
        }
        assert_eq!(replay.index.len(), length as usize);
        let newest: Vec<u64> = replay.index[replay.index.len() - FIRST_VIEW_BLOCKS..]
            .iter()
            .map(|block| block.offset)
            .collect();
        let whole = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
        let source = crate::Source::from_replay(
            crate::SourceKey {
                source_id: "production".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load = |exact: Option<bool>| {
            ProductionLines::load(
                &source,
                &ProductionLinesParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    exact,
                },
                &LoadContext { content: None },
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        let (first, read) = blocks_read(|| load(None));
        assert_eq!(read, newest, "a {length}-block recording");
        let window = FIRST + length - bound;
        assert_eq!(
            first.approximate,
            [HourSpan {
                first_hour: FIRST,
                last_hour: window
            }]
        );
        assert_eq!(first.reads, (window..FIRST + length).collect::<Vec<_>>());
        assert_eq!(
            first
                .lines
                .iter()
                .map(|line| (line.key.as_str(), line.before_recording, line.ended_by))
                .collect::<Vec<_>>(),
            [("5#1", true, None)]
        );
        assert!(
            first
                .warnings
                .iter()
                .any(|warning| warning.starts_with("This first view reads only GER's"))
        );
        // Another first view answers from the one held.
        let (again, read) = blocks_read(|| load(None));
        assert!(read.is_empty());
        assert_eq!(
            (again.approximate, again.lines),
            (first.approximate.clone(), first.lines.clone())
        );
        let usage =
            || crate::prepared::query::observations::usage(&source, "production-lines-overlap");
        assert_eq!(usage().2, FIRST_VIEW_BLOCKS);
        let (exact, read) = blocks_read(|| load(Some(true)));
        assert_eq!(read.len(), length as usize);
        let (bytes, budget, extracts) = usage();
        assert!(bytes > 0 && bytes <= budget);
        assert_eq!(
            extracts, FIRST_VIEW_BLOCKS,
            "the exact follow-up decoded its newest blocks again"
        );
        assert!(exact.approximate.is_empty());
        assert_eq!(
            exact.lines,
            whole.lines.iter().map(line_history).collect::<Vec<_>>()
        );
        assert_eq!(exact.lines[1].ended_by, Some(FIRST + length / 2));
        assert_eq!(
            (&exact.reads, &exact.line_hours, &exact.output_hours),
            (&whole.reads, &whole.line_hours, &whole.output_hours)
        );
        assert!(
            !exact
                .warnings
                .iter()
                .any(|warning| warning.starts_with("This first view"))
        );
        // A reply is its request's: the default request is still the first view
        // once the whole scan is held, read from the one held; `exact` reads nothing.
        let (again, read) = blocks_read(|| load(None));
        assert!(read.is_empty());
        assert_eq!(
            (again.approximate, again.lines),
            (first.approximate.clone(), first.lines.clone())
        );
        let (held, read) = blocks_read(|| load(Some(true)));
        assert!(read.is_empty());
        assert_eq!(held.lines, exact.lines);
    }
}

/// A first view holds one detail window of the newest blocks, and never reads more
/// blocks than its bound, whatever their size or number.
#[test]
fn a_first_view_holds_a_detail_window_of_newest_blocks_within_its_bound() {
    let spans = |count: i32, hours: i32| {
        (0..count)
            .map(|block| (block * hours, block * hours + hours - 1))
            .collect::<Vec<_>>()
    };
    // Fifty years in blocks of about 660 hours: the newest two hold a window.
    assert_eq!(first_view_blocks(&spans(663, 660)), 2);
    assert_eq!(first_view_blocks(&spans(6_630, 660)), 2);
    assert_eq!(first_view_blocks(&spans(663, DETAIL_HOURS)), 1);
    // Hour-long blocks (appended segments) stop at the bound.
    assert_eq!(first_view_blocks(&spans(5_000, 1)), FIRST_VIEW_BLOCKS);
    // A short branch is read whole.
    assert_eq!(first_view_blocks(&spans(1, 30)), 1);
    assert_eq!(first_view_blocks(&[]), 0);
}

/// A scan builds a variant read's operands again only for definitions whose inputs
/// changed (their row, their modules, or the countries and tokens every name reads),
/// and its catalogue holds exactly the versions a catalogue observing every hour
/// records: across a changed module, a renamed variant, a country renamed, an
/// incomplete read and a gap.
#[test]
fn variant_reads_build_only_changed_definitions_and_name_as_every_read_does() {
    use crate::products::production_lines::scan;
    use replay_data::{rows::VariantModuleRow, rows::VariantUpgradeRow};
    let frame = |hour: i32| {
        let at = hour - FIRST;
        let mut frame = production_frame(
            hour,
            at % 24 == 0,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, i64::from(at), true, false)]),
        );
        // Variant 70 has one module, changed at 30.
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![VariantModuleRow {
                variant: 70,
                slot: 601,
                definition_token: if at >= 30 { 602 } else { 601 },
                definition_active: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(Vec::<VariantUpgradeRow>::new()));
        if at >= 50
            && let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                &mut *frame.tables[TableId::VariantDefinitions.index()]
        {
            rows[1].display_name = b"Mauser".to_vec();
        }
        // The slot-zero country is renamed: its type-tag naming operand changes,
        // even for a variant whose own display name stays the same.
        if at >= 70
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"ITB".to_vec();
        }
        if at == 90 {
            *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
                reason: "fixture".into(),
            };
        }
        frame
    };
    // Hour 100 is missing: a gap.
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            (FIRST..FIRST + 120)
                .filter(|hour| *hour != FIRST + 100)
                .map(frame)
                .collect(),
        )],
    )
    .unwrap();
    // The catalogue as observing every hour records it.
    let mut expected = VariantCatalogue::default();
    let mut tables = vec![TableId::Campaign];
    tables.extend(VariantCatalogue::TABLES);
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&tables);
        for frame in replay.read_block_selected(block, &tables).unwrap() {
            expected.observe(replay.format, decoder.advance(&frame).unwrap());
        }
    }
    VARIANT_BUILDS.with(|builds| builds.set(0));
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.variants.versions, expected.versions);
    // Both at the first read; 70 at its new module; 71 at its new name; both when
    // a country is renamed, after the incomplete read and after the gap.
    assert_eq!(
        VARIANT_BUILDS.with(std::cell::Cell::get),
        2 + 1 + 1 + 2 + 2 + 2
    );
    let names: Vec<_> = expected.versions[&71]
        .iter()
        .map(|version| {
            (
                version.first_hour,
                version.raw.as_ref().map(|raw| raw.display_name.clone()),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            (FIRST, Some("Gewehr 98".to_owned())),
            (FIRST + 50, Some("Mauser".to_owned())),
            (FIRST + 70, Some("Mauser".to_owned())),
            (FIRST + 90, None),
            (FIRST + 91, Some("Mauser".to_owned())),
            (FIRST + 100, None),
            (FIRST + 101, Some("Mauser".to_owned())),
        ]
    );
}

/// Valid synthetic country trees in the native request catalogue's order.
fn saved_modifier_trees() -> Vec<replay_data::rows::ProductionModifierTree> {
    engine_production::production_modifiers::KEYS
        .into_iter()
        .map(|key| replay_data::rows::ProductionModifierTree {
            key,
            nodes: vec![replay_data::rows::ModifierBreakdownNode {
                children: 0,
                value: 0,
                name: b"Synthetic total".to_vec(),
            }],
            failure: vec![],
        })
        .collect()
}

/// A saved scan restores to the scan it saved: every value, its modifier runs'
/// replay rows through the recording's own encoding, a country row its runs share
/// still shared once, and the recording's language and content's localisation
/// given again. A scan saved by another layout is not restored, and one that read
/// a localisation is not restored without it.
#[test]
fn a_saved_scan_restores_the_scan_it_saved() {
    use crate::products::production_lines::{
        LineKind, RawOrganisation, RawRun, SavedLines, ScannedLine, ScannedLines,
        breakdown::RawModifierRun,
    };
    use replay_data::{rows::CountryProductionModifiersRow, rows::ProductionLineModifiersRow};
    let run = |first_hour: i32| RawRun {
        first_hour,
        last_hour: first_hour + 23,
        variant: 70,
        active_factories: 8,
        cost_raw: 500_000,
        efficiency_raw: 1_000_000,
        amount: -1,
        ..RawRun::default()
    };
    let line = |handle: u64| ScannedLine {
        key: format!("{handle}#1"),
        handle,
        kind: Some(LineKind::Military),
        runs: vec![run(0), run(24)],
        output: Vec::new(),
        final_output: None,
        ended_by: Some(48),
        before_recording: true,
        between_reads: None,
        variants: vec![70],
    };
    let mut scanned = ScannedLines {
        lines: vec![line(5), line(6)],
        reads: vec![0, 24],
        unknown_kinds: 1,
        unlinked_rows: 2,
        ..ScannedLines::default()
    };
    scanned.icons.insert(
        70,
        vec![
            (
                0,
                IconChoice {
                    probes: Vec::new(),
                    otherwise: Ok(()),
                },
            ),
            (24, IconChoice::unknown("unknown".into())),
        ],
    );
    scanned.organisations.insert(
        79,
        vec![RawOrganisation {
            first_hour: 0,
            token: "GER_mauser_organization".into(),
            owner_tag: Some("GER".into()),
            ..RawOrganisation::default()
        }],
    );
    scanned
        .modifier_layout_refusals
        .insert("production modifiers refused".into(), 3);
    let shared = Arc::new(CountryProductionModifiersRow {
        country: 9001,
        trees: saved_modifier_trees(),
        ..Default::default()
    });
    let later = Arc::new(CountryProductionModifiersRow {
        country: 9001,
        base_cap: 7,
        trees: shared.trees.clone(),
        ..Default::default()
    });
    for (handle, countries) in [(5, [&shared, &shared]), (6, [&shared, &later])] {
        let runs = [run(0), run(24)]
            .into_iter()
            .zip(countries)
            .map(|(run, country)| RawModifierRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                country: Arc::clone(country),
                line: ProductionLineModifiersRow {
                    line: handle,
                    country: 9001,
                    variant: 70,
                    active_factories: 8,
                    factory_efficiencies: vec![run.efficiency_raw; 8],
                    ..Default::default()
                },
                run,
                organisation: Some("Mauser".into()),
                localisation: None,
                recorded_language: Arc::from("english"),
            })
            .collect();
        scanned.modifiers.insert(format!("{handle}#1"), runs);
    }
    let saved = SavedLines::save(&scanned, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        saved.countries.len(),
        2,
        "a shared country row is kept once"
    );
    let bytes = serde_json::to_vec(&saved).unwrap();
    let parsed: SavedLines = serde_json::from_slice(&bytes).unwrap();
    let restored = parsed
        .clone()
        .restore(None, Arc::from("english"))
        .unwrap()
        .unwrap();
    assert_eq!(restored.lines, scanned.lines);
    assert_eq!(restored.modifiers, scanned.modifiers);
    assert_eq!(restored.icons, scanned.icons);
    assert_eq!(restored.organisations, scanned.organisations);
    assert_eq!(
        restored.modifier_layout_refusals,
        scanned.modifier_layout_refusals
    );
    assert_eq!(restored.variants.versions, scanned.variants.versions);
    assert_eq!(
        (
            restored.reads.clone(),
            restored.unknown_kinds,
            restored.unlinked_rows
        ),
        (scanned.reads.clone(), 1, 2)
    );
    let runs = &restored.modifiers["5#1"];
    assert!(Arc::ptr_eq(&runs[0].country, &runs[1].country));
    assert!(Arc::ptr_eq(
        &runs[0].country,
        &restored.modifiers["6#1"][0].country
    ));
    // Span extension retains the modifier operands' supplying run, and leaves
    // endpoints with a gap untouched just as hourly extension does.
    let open = BTreeMap::from([(5, Open { index: 0 }), (6, Open { index: 1 })]);
    let mut spans = restored.clone();
    let mut hourly = restored.clone();
    carry_lines(&mut spans, &open, (48, 71), true, true);
    for hour in 48..=71 {
        carry_lines(&mut hourly, &open, (hour, hour), true, true);
    }
    assert_eq!(spans.modifiers, hourly.modifiers);
    assert_eq!(spans.lines, hourly.lines);
    assert_eq!(spans.modifiers["5#1"][1].last_hour, 71);
    carry_lines(&mut spans, &open, (74, 80), true, true);
    assert_eq!(spans.modifiers["5#1"][1].last_hour, 71);
    assert_eq!(spans.lines[0].runs[1].last_hour, 71);

    // Another layout's scan is scanned again.
    let mut other = parsed.clone();
    other.schema += 1;
    assert!(other.restore(None, Arc::from("english")).unwrap().is_none());
    // A localised scan needs its content's localisation.
    let mut localised = parsed;
    localised.localised = true;
    assert!(localised.restore(None, Arc::from("english")).is_err());
}

/// A recording of `length` one-hour blocks, each a read of Germany's two lines.
fn hourly_blocks(length: i32) -> Replay {
    use crate::test_fixtures::continuation_records;
    let frame = |hour: i32| {
        let units = i64::from(hour - FIRST);
        production_frame(
            hour,
            true,
            true,
            9001,
            &[(5, 70, 8, 0), (6, 71, 2, 0)],
            Some(&[(5, 70, units, true, false), (6, 71, units, true, false)]),
        )
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(FIRST)])]).unwrap();
    for hour in FIRST + 1..FIRST + length {
        replay = replay
            .append(&[continuation_records(&replay, 1, vec![frame(hour)])])
            .unwrap();
    }
    replay
}

fn memory_source(replay: Replay) -> crate::Source {
    crate::Source::from_replay(
        crate::SourceKey {
            source_id: "production".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn exact_lines(source: &crate::Source) -> ProductionLinesReply {
    use crate::products::{LoadContext, Product, production_lines::ProductionLinesParams};
    ProductionLines::load(
        source,
        &ProductionLinesParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            exact: Some(true),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Requests for one country's whole scan made together read the branch once: a
/// request finding the scan being read waits for it.
#[test]
fn requests_for_a_scan_being_read_wait_for_it() {
    let source = memory_source(hourly_blocks(40));
    let start = std::sync::Barrier::new(3);
    let replies: Vec<ProductionLinesReply> = std::thread::scope(|scope| {
        let readers: Vec<_> = (0..3)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    exact_lines(&source)
                })
            })
            .collect();
        readers
            .into_iter()
            .map(|reader| reader.join().unwrap())
            .collect()
    });
    assert_eq!(
        source
            .state::<WholeScans>()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0,
        1
    );
    assert!(replies.iter().all(|reply| reply.lines == replies[0].lines));
}

/// A country's scan kept between sessions is held again for a source of the same
/// recording, which then answers exactly what its own scan would, reading no
/// block; it is not held once the branch has another last block.
#[test]
fn a_kept_scan_answers_as_the_scan_it_kept_while_it_is_current() {
    use crate::products::{
        LoadContext,
        production_lines::{hold_saved, saved_scan},
    };
    let context = LoadContext { content: None };
    let first = memory_source(hourly_blocks(12));
    let kept = saved_scan(&first, 1, "GER", &context, &AtomicBool::new(false)).unwrap();
    let expected = exact_lines(&first);
    assert_eq!(kept.last_block.map(|block| block.1), Some(FIRST + 11));
    let again = memory_source(hourly_blocks(12));
    assert!(hold_saved(&again, &context, kept.clone()));
    let (reply, read) = blocks_read(|| exact_lines(&again));
    assert!(read.is_empty(), "the kept scan answers without reading");
    assert_eq!(reply.lines, expected.lines);
    assert_eq!(reply.reads, expected.reads);
    assert_eq!(
        again
            .state::<WholeScans>()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0,
        0
    );
    // A recording grown by a block is scanned again.
    let grown = memory_source(hourly_blocks(13));
    assert!(!hold_saved(&grown, &context, kept));
    let (_, read) = blocks_read(|| exact_lines(&grown));
    assert_eq!(read.len(), 13);
}

/// A kept scan's icon choices hold only while what they were made with holds outside the
/// recorded content's hash: its graphic pools (and geography and DLC). Which sprites are
/// declared and which texture files exist do not refuse it: a choice leaves those questions
/// open, the same whatever the answers, and each reply answers them (a deleted texture is
/// then another icon).
#[test]
fn a_kept_scans_choices_hold_only_while_their_inputs_do() {
    use crate::products::{
        equipment_icon::{Icon, IconContent, IconHour, icon_choice},
        production_lines::{SavedLines, SavedScan, pictures_hold},
    };
    use installed_content_equipment_graphics::equipment_graphics;
    use replay_data::rows::VariantDefinitionRow;
    const TEXTURE: &str = "gfx/models/custom.dds";
    let content = |files: &[&str], sprites: &[&str], pools: &[&[u8]]| {
        IconContent::from_parts(
            equipment_types(&[b"equipments = { infantry_equipment_1 = { } }"]).unwrap(),
            Default::default(),
            equipment_graphics::Content::from_texts(
                pools,
                &equipment_graphics::Names {
                    countries: Default::default(),
                    continents: Default::default(),
                },
            )
            .unwrap(),
            Default::default(),
            sprites.iter().map(|name| (*name).to_owned()).collect(),
            files.iter().map(|name| (*name).to_owned()).collect(),
        )
    };
    // An active entry naming a texture file.
    let definition = VariantDefinitionRow {
        id: 70,
        equipment_token: 601,
        creator_link: 11,
        icon_active: 1,
        icon_kind: 1,
        icon: TEXTURE.as_bytes().to_vec(),
        ..Default::default()
    };
    let frame = production_frame(FIRST, true, true, 9001, &[], Some(&[]));
    let with = content(&[TEXTURE], &["GFX_fallback"], &[]);
    let hour = IconHour::read(&frame, &with).unwrap();
    let choice = icon_choice(&with, &hour, &definition, "infantry_equipment_1");
    assert_eq!(
        choice.walk(|probe| with.answer(probe)),
        Ok(Some(Icon::Path(TEXTURE.into())))
    );
    let saved = SavedScan {
        branch: 1,
        tag: "GER".into(),
        pictured: true,
        visual: Some(with.visual_inputs().to_owned()),
        executable: false,
        last_block: None,
        scan: SavedLines::save(&ScannedLines::default(), &AtomicBool::new(false)).unwrap(),
    };
    assert!(pictures_hold(
        Some(&content(&[TEXTURE], &["GFX_fallback"], &[])),
        &saved
    ));
    // The texture deleted, or sprites declared otherwise: the same choice, which the reply
    // answers with another icon; the kept scan holds.
    let deleted = content(&[], &["GFX_other"], &[]);
    assert_eq!(
        icon_choice(&deleted, &hour, &definition, "infantry_equipment_1"),
        choice
    );
    assert_ne!(
        choice.walk(|probe| deleted.answer(probe)),
        Ok(Some(Icon::Path(TEXTURE.into())))
    );
    assert!(pictures_hold(Some(&deleted), &saved));
    // Other graphic pools, or no icon content, refuse it.
    let pooled = content(
        &[TEXTURE],
        &["GFX_fallback"],
        &[br#"default = { infantry_equipment_1 = { pool = { icons = { GFX_fallback } } } }"#],
    );
    assert!(!pictures_hold(Some(&pooled), &saved));
    assert!(!pictures_hold(None, &saved));
    let unpictured = SavedScan {
        pictured: false,
        visual: None,
        ..saved
    };
    assert!(pictures_hold(None, &unpictured));
    assert!(!pictures_hold(Some(&with), &unpictured));
}

/// Saving a scan looks up each modifier run's country row once, by the
/// allocation it shares: its work is one step per run however many rows the
/// runs share, each row is encoded once, and a cancelled save stops.
#[test]
fn saving_a_scan_does_one_lookup_per_modifier_run() {
    use crate::products::production_lines::{
        RawRun, SavedLines, ScannedLines, breakdown::RawModifierRun,
    };
    use replay_data::{rows::CountryProductionModifiersRow, rows::ProductionLineModifiersRow};
    let (lines, days) = (50_u64, 200_i32);
    // The country's row changes at every read; every line shares it.
    let rows: Vec<Arc<CountryProductionModifiersRow>> = (0..days)
        .map(|day| {
            Arc::new(CountryProductionModifiersRow {
                country: 9001,
                base_cap: day,
                trees: saved_modifier_trees(),
                ..Default::default()
            })
        })
        .collect();
    let mut scanned = ScannedLines::default();
    for handle in 0..lines {
        let runs = rows
            .iter()
            .zip(0..)
            .map(|(row, day)| RawModifierRun {
                first_hour: day * 24,
                last_hour: day * 24 + 23,
                country: Arc::clone(row),
                line: ProductionLineModifiersRow {
                    line: handle,
                    country: 9001,
                    ..Default::default()
                },
                run: RawRun::default(),
                organisation: None,
                localisation: None,
                recorded_language: Arc::from("english"),
            })
            .collect();
        scanned.modifiers.insert(format!("{handle}#1"), runs);
    }
    super::saved::SAVE_WORK.with(|work| work.set((0, 0)));
    let saved = SavedLines::save(&scanned, &AtomicBool::new(false)).unwrap();
    let (lookups, encoded) = super::saved::SAVE_WORK.with(std::cell::Cell::get);
    assert_eq!(
        lookups,
        lines as usize * days as usize,
        "one lookup per run"
    );
    assert_eq!(
        (encoded, saved.countries.len()),
        (days as usize, days as usize)
    );
    let error = SavedLines::save(&scanned, &AtomicBool::new(true)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}

#[test]
fn organisation_daily_states_are_sent_only_for_the_demanded_detail_window() {
    let mut scanned = ScannedLines::default();
    scanned.lines.push(ScannedLine {
        key: "5#1".into(),
        handle: 5,
        kind: Some(LineKind::Military),
        runs: vec![RawRun {
            first_hour: 0,
            last_hour: 95,
            mio: 79,
            variant: 70,
            ..Default::default()
        }],
        output: vec![],
        final_output: None,
        ended_by: None,
        before_recording: true,
        between_reads: None,
        variants: vec![],
    });
    let state = |points| {
        let mut row = IndustrialOrganisationDetailsRow::default();
        row.state.points = points;
        organisation_state(&row, Some(24), &[])
    };
    scanned.organisations.insert(
        79,
        vec![
            RawOrganisation {
                first_hour: 0,
                state: Some(state(1)),
                ..Default::default()
            },
            RawOrganisation {
                first_hour: 24,
                state: Some(state(2)),
                ..Default::default()
            },
            RawOrganisation {
                first_hour: 48,
                state: None,
                ..Default::default()
            },
            RawOrganisation {
                first_hour: 72,
                state: Some(state(9)),
                ..Default::default()
            },
        ],
    );
    let details = detail_lines(&scanned, 24, 47);
    assert_eq!(details[0].organisations.len(), 1);
    assert_eq!(details[0].organisations[0].first_hour, 24);
    assert_eq!(
        details[0].organisations[0]
            .state
            .as_ref()
            .unwrap()
            .unspent_points,
        2
    );
    let details = detail_lines(&scanned, 50, 60);
    assert_eq!(details[0].organisations.len(), 1);
    assert!(details[0].organisations[0].state.is_none());
}

#[test]
fn organisation_design_observations_follow_stats_cadence_availability_and_shared_read_dates() {
    use replay_data::rows::{IndustrialOrganisationBonus, VariantStatsRow};
    let design_hour = FIRST + 14; // line slot 08 -> variant slot 22
    let lost_hour = design_hour + 2;
    let recovered_hour = design_hour + 4;
    let frames = (FIRST..=FIRST + 25)
        .map(|hour| {
            let mut frame = production_frame(
                hour,
                (hour - FIRST) % 24 == 0,
                false,
                9001,
                &[(5, 70, 8, 0)],
                Some(&[(5, 70, 0, true, false)]),
            );
            if hour < design_hour || (lost_hour..recovered_hour).contains(&hour) {
                *frame.tables[TableId::VariantStats.index()] = TableView::Unavailable {
                    reason: "not observed".into(),
                };
            } else {
                *frame.tables[TableId::VariantStats.index()] =
                    TableView::Complete(TableRows::VariantStats(vec![VariantStatsRow {
                        variant: 70,
                        organisation: 79,
                        organisation_trait_count: 7,
                        organisation_bonuses: vec![IndustrialOrganisationBonus {
                            key: 601,
                            value_raw: -123,
                        }],
                        ..Default::default()
                    }]));
            }
            // The worker's shared group charges modules, not stats. No new line
            // read accompanies the design observation or its forced recovery.
            *frame.tables[TableId::VariantModules.index()] =
                TableView::Complete(TableRows::VariantModules(vec![]));
            if hour == design_hour || hour == recovered_hour {
                frame.metrics.table_ns[TableId::VariantModules.index()] = 1;
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [FIRST, FIRST + 24]);
    let detail = detail_lines(&scanned, FIRST, FIRST + 25);
    let at = |hour| {
        detail[0]
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && run.last_hour >= hour)
            .unwrap()
    };
    assert_eq!(at(design_hour - 1).design_author, None);
    assert_eq!(at(design_hour).design_author.as_deref(), Some("79"));
    assert_eq!(at(design_hour).design_trait_count, Some(7));
    assert_eq!(at(design_hour).design_bonus_count, Some(1));
    assert_eq!(at(design_hour).design_read_hour, Some(design_hour));
    assert_eq!(at(design_hour + 1).design_read_hour, Some(design_hour));
    assert_eq!(
        (
            at(lost_hour).design_author.as_deref(),
            at(lost_hour).design_read_hour
        ),
        (None, None)
    );
    assert_eq!(at(recovered_hour).design_author.as_deref(), Some("79"));
    assert_eq!(at(recovered_hour).design_read_hour, Some(recovered_hour));
    assert_eq!(
        at(FIRST + 24).design_read_hour,
        Some(recovered_hour),
        "line read must not redate the design"
    );
    let saved = SavedLines::save(&scanned, &AtomicBool::new(false)).unwrap();
    let restored = saved.restore(None, Arc::from("english")).unwrap().unwrap();
    assert_eq!(restored.designs, scanned.designs);
    assert_eq!(detail_lines(&restored, FIRST, FIRST + 25), detail);
}

#[test]
fn design_detail_projects_only_observations_overlapping_the_requested_window() {
    use super::{DesignRead, append_design_runs};
    // One unchanged line can outlive thousands of independent design reads.
    let run = RawRun {
        first_hour: 0,
        last_hour: 240_000,
        starts_segment: true,
        ..Default::default()
    };
    let versions: Vec<_> = (0..10_000)
        .map(|day| DesignRead {
            first_hour: day * 24 + 22,
            read_hour: Some(day * 24 + 22),
            author: Some(day as u64),
            ..Default::default()
        })
        .collect();
    let (first, last) = (120_000, 120_719);
    let mut projected = Vec::new();
    append_design_runs(&mut projected, &run, &versions, first, last);
    assert_eq!(projected.len(), 31);
    assert_eq!(projected[0].first_hour, first - 2);
    assert_eq!(projected.last().unwrap().last_hour, last + 22);
    assert!(projected.iter().all(|value| {
        value.first_hour <= last && value.last_hour >= first && !value.starts_segment
    }));
    assert_eq!(projected[0].design_author.as_deref(), Some("4999"));
    assert_eq!(
        projected.last().unwrap().design_author.as_deref(),
        Some("5029")
    );
    projected.clear();
    append_design_runs(&mut projected, &run, &versions, 0, 21);
    assert_eq!(projected.len(), 1);
    assert_eq!(
        projected[0].design_author, None,
        "no future observation leaks backwards"
    );
    assert!(projected[0].starts_segment);
    projected.clear();
    append_design_runs(&mut projected, &run, &[], first, last);
    assert_eq!(
        (projected[0].first_hour, projected[0].last_hour),
        (0, 240_000)
    );
}

fn country_lookup_reuse(count: u32) {
    use crate::prepared::prepass::noted;
    use crate::test_fixtures::{scale_frame, scale_tag};
    use replay_frame::frame::FrameEncoder;
    let mut encoder = FrameEncoder::default();
    let mut decoder = FrameDecoder::for_tables(&[TableId::Countries, TableId::CountryLinks]);
    let mut selected = CountryRead::default();
    let tag = String::from_utf8(scale_tag(count - 1)).unwrap();
    let mut lookups = 0;
    for at in 0..24 {
        let mut frame = scale_frame(1, at + i32::from(at >= 20), count);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::from(at) * 1_000_000);
            if (4..7).contains(&at) {
                rows.last_mut().unwrap().tag = b"renamed".to_vec();
            }
            if at >= 14 {
                rows.last_mut().unwrap().id = 900_000;
            }
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            if (10..12).contains(&at) {
                rows.pop();
            }
            if at >= 14 {
                rows.last_mut().unwrap().id = 900_000;
            }
        }
        let raw = encoder.encode(frame, at == 0 || at == 18).unwrap();
        let decoded = decoder.advance(&raw).unwrap();
        let before = noted("production-lines.country_lookup");
        let actual = selected.observe(&raw, decoded, &tag, None);
        lookups += noted("production-lines.country_lookup") - before;
        assert_eq!(
            actual,
            select_country(decoded, &tag, None),
            "hour {}",
            decoded.hour
        );
    }
    assert!(
        lookups <= 9,
        "balance-only hours repeated {lookups} whole-country lookups"
    );
}

#[test]
fn production_country_lookup_reuses_keys_and_follows_every_intervening_identity_edit() {
    country_lookup_reuse(32);
}

#[test]
fn production_country_lookup_reuses_keys_at_5000_countries() {
    country_lookup_reuse(5000);
}

fn quiet_production_replay(lines: u64) -> Replay {
    use crate::test_fixtures::continuation_records;
    let frame = |at: i32| {
        let rows: Vec<_> = (0..lines).map(|line| (line + 5, 70, 8, 3276800)).collect();
        let outputs: Vec<_> = (0..lines)
            .map(|line| (line + 5, 70, i64::from(at), true, false))
            .collect();
        let mut frame = production_frame(
            FIRST + at,
            at % 24 == 0 || at == 34,
            at % 24 == 0,
            if at < 80 { 9001 } else { 9002 },
            &rows,
            Some(&outputs),
        );
        let unavailable = match at {
            11 | 12 | 24 => Some(TableId::ProductionLineOutput),
            17 | 18 => Some(TableId::CountryProductionModifiers),
            29 | 30 => Some(TableId::IndustrialOrganisations),
            44 => Some(TableId::CountryLinks),
            52 | 53 => Some(TableId::ProductionLines),
            60 => Some(TableId::VariantDefinitions),
            71 => Some(TableId::LexicalTokens),
            _ => None,
        };
        if let Some(table) = unavailable {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "synthetic quiet-span boundary".into(),
            };
        }
        if (40..43).contains(&at)
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows.last_mut().unwrap().tag = b"renamed".to_vec();
        }
        frame
    };
    let block = |first, last| (first..last).filter(|at| *at != 56).map(frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, block(0, 37))]).unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 1, block(37, 70))])
        .unwrap();
    replay
        .append(&[continuation_records(&replay, 1, block(70, 100))])
        .unwrap()
}

#[test]
fn quiet_production_spans_equal_hourly_scan_bytes_across_reads_gaps_refusals_and_append() {
    use crate::prepared::prepass::noted;
    let replay = quiet_production_replay(32);
    let flag = AtomicBool::new(false);
    for holder in [None, Some((7, 9001))] {
        let blocks = branch_blocks(&replay, 1);
        let before = noted("production-lines.carried_line");
        let hourly =
            scan_blocks_mode(None, &replay, &blocks, "GER", None, holder, &flag, false).unwrap();
        let hourly_work = noted("production-lines.carried_line") - before;
        let before = noted("production-lines.carried_line");
        let actual =
            scan_blocks_mode(None, &replay, &blocks, "GER", None, holder, &flag, true).unwrap();
        let work = noted("production-lines.carried_line") - before;
        assert!(
            work * 3 < hourly_work,
            "quiet spans repeated per-hour line work: {work}/{hourly_work}"
        );
        let saved = |scan: &ScannedLines| {
            serde_json::to_vec(&SavedLines::save(scan, &flag).unwrap()).unwrap()
        };
        assert_eq!(saved(&actual), saved(&hourly), "holder {holder:?}");
        assert!(!actual.unread_output.is_empty());
        for window in 0..5 {
            assert_eq!(
                detail_lines(&actual, FIRST + window * 24, FIRST + window * 24 + 23),
                detail_lines(&hourly, FIRST + window * 24, FIRST + window * 24 + 23)
            );
        }
        // Newest-window folds retain their own origins, not the whole branch's.
        let newest = &blocks[blocks.len() - 1..];
        let first =
            scan_blocks_mode(None, &replay, newest, "GER", None, holder, &flag, true).unwrap();
        let reference =
            scan_blocks_mode(None, &replay, newest, "GER", None, holder, &flag, false).unwrap();
        assert_eq!(saved(&first), saved(&reference));
    }
    assert_eq!(
        scan(&replay, 1, "GER", None, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

/// A variant's icon choice is answered and drawn as the request's selection shows it: the
/// recording's own icon without one; a mod declaring an earlier candidate the recording lacks
/// makes it the icon, as does a mod texture file at an earlier path, found on the selection's
/// own stack. Where the selection's choice cannot be drawn, the recording's own choice is,
/// named on every reply. Each choice renders once per selection, and a second reply only hits
/// the memo.
#[test]
fn variant_icons_follow_the_selection_once_per_choice() {
    use crate::test_fixtures::visual_mods::{
        BLUE, GREEN, Machine, RED, fallback_line, first_pixel, png, select,
    };
    let sprites = |early: Option<&str>| {
        let early = early.map_or(String::new(), |texture| {
            format!("spriteType = {{ name = GFX_early textureFile = {texture} }}")
        });
        format!(
            "spriteTypes = {{ spriteType = {{ name = GFX_late textureFile = gfx/late.png }} {early} }}"
        )
    };
    let machine = Machine::new("production_line_icons");
    machine.write("interface/equipment.gfx", sprites(None).as_bytes());
    machine.write("gfx/late.png", &png(RED));
    machine.write_mod("early", "interface/equipment.gfx", sprites(Some("gfx/early.png")).as_bytes());
    machine.write_mod("early", "gfx/early.png", &png(BLUE));
    machine.write_mod("texture", "gfx/designs/custom.png", &png(GREEN));
    // The mod's declarations replace the recording's file: its early sprite does not load.
    machine.write_mod("broken", "interface/equipment.gfx", sprites(Some("gfx/broken.png")).as_bytes());
    machine.write_mod("broken", "gfx/broken.png", b"not a png");
    let (source, mut options) = machine.open();
    let choice = IconChoice {
        probes: vec![
            Probe::File("gfx/designs/custom.png".into()),
            Probe::Sprite("GFX_early".into()),
            Probe::Sprite("GFX_late".into()),
        ],
        otherwise: Ok(()),
    };
    let shown = |options: &dashboard_core::config::ContentOptions| {
        let admitted = catalogue::catalogue(&source, options).unwrap();
        let recorded = catalogue::recorded_catalogue(&source, options).unwrap();
        let state = source.state::<catalogue::ByCatalogue<VariantArt>>();
        let mut state = state.lock().unwrap();
        let drawn = catalogue::Redrawn::bind(&mut state.of(&admitted).0, &admitted, &recorded);
        let mut images = BTreeMap::new();
        let (key, line) =
            fallback_line(|| draw_variant_icon(drawn, &mut images, &choice).unwrap().unwrap());
        (first_pixel(images[&key].as_deref().unwrap()), line)
    };
    let renders = catalogue::presented;
    select(&source, &mut options, &[]);
    let before = renders();
    assert_eq!(shown(&options), (RED, None));
    assert_eq!(renders() - before, 1, "one render per choice");
    let before = renders();
    assert_eq!(shown(&options), (RED, None));
    assert_eq!(renders() - before, 0, "a second reply only hits the memo");
    select(&source, &mut options, &["early"]);
    assert_eq!(shown(&options), (BLUE, None));
    select(&source, &mut options, &["texture"]);
    assert_eq!(shown(&options), (GREEN, None));
    select(&source, &mut options, &["broken"]);
    for _ in 0..2 {
        let (pixel, line) = shown(&options);
        assert_eq!(pixel, RED, "the recording's own choice");
        assert!(
            line.as_ref().is_some_and(|line| line.contains("production line icon")),
            "{line:?}"
        );
    }
}

/// One reply's pictures are keyed by what they show: a mod recolours the recording's sprite A
/// and adds an earlier candidate E it cannot draw. A variant choosing [E, A] shows the
/// recording's own choice, the recording's red A; a variant choosing [A] and an organisation
/// showing A show the mod's blue A. The two As are two pictures under two keys, and the two
/// blue ones share one.
#[test]
fn reply_pictures_are_keyed_by_what_they_show() {
    use crate::test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select};
    let machine = Machine::new("production_line_picture_keys");
    machine.write(
        "interface/equipment.gfx",
        b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png } }",
    );
    machine.write("gfx/a.png", &png(RED));
    // The mod's declarations replace the recording's file: its E does not load.
    machine.write_mod(
        "selector",
        "interface/equipment.gfx",
        b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png } \
          spriteType = { name = GFX_e textureFile = gfx/broken.png } }",
    );
    machine.write_mod("selector", "gfx/a.png", &png(BLUE));
    machine.write_mod("selector", "gfx/broken.png", b"not a png");
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["selector"]);
    let admitted = catalogue::catalogue(&source, &options).unwrap();
    let recorded = catalogue::recorded_catalogue(&source, &options).unwrap();
    let state = source.state::<catalogue::ByCatalogue<VariantArt>>();
    let mut state = state.lock().unwrap();
    let drawn = catalogue::Redrawn::bind(&mut state.of(&admitted).0, &admitted, &recorded);
    let sprites = |names: &[&str]| IconChoice {
        probes: names.iter().map(|name| Probe::Sprite((*name).into())).collect(),
        otherwise: Ok(()),
    };
    let mut images = BTreeMap::new();
    let fallen = draw_variant_icon(drawn, &mut images, &sprites(&["GFX_e", "GFX_a"]))
        .unwrap()
        .unwrap();
    let plain = draw_variant_icon(drawn, &mut images, &sprites(&["GFX_a"]))
        .unwrap()
        .unwrap();
    let raw = RawOrganisation {
        first_hour: 0,
        token: "tok".into(),
        name_key: String::new(),
        icon: "GFX_a".into(),
        owner_tag: None,
        owner_unknown: false,
        state: None,
    };
    let (organisation, image) = organisation_shown(
        &organisation_picture(&admitted, &raw, ICON_EDGE),
        &admitted,
        &raw,
    )
    .unwrap();
    images.entry(organisation.clone()).or_insert(image.ok());
    let pixel = |key: &str| first_pixel(images[key].as_deref().unwrap());
    assert_eq!(pixel(&fallen), RED, "the recording's own choice");
    assert_eq!(pixel(&plain), BLUE);
    assert_eq!(pixel(&organisation), BLUE);
    assert_ne!(fallen, plain);
    assert_eq!(plain, organisation, "one picture, one key");
    assert_eq!(images.len(), 2);
}

/// The recording declares only the organisation's untagged sprite (A); a mod declares the
/// owner's tagged one (B), which comes first and which the mod cannot draw. The recording's own
/// choice, A, is drawn, named on the reply and again on a memo hit; without a selection A
/// is the shown choice, drawn as it is.
#[test]
fn an_organisation_icon_the_mod_cannot_draw_draws_the_recordings_own_choice() {
    use crate::test_fixtures::visual_mods::{Machine, RED, fallback_line, first_pixel, png, select};
    let machine = Machine::new("organisation_icon");
    machine.write(
        "interface/organisations.gfx",
        b"spriteTypes = { spriteType = { name = GFX_tok textureFile = gfx/tok.png } }",
    );
    machine.write("gfx/tok.png", &png(RED));
    machine.write_mod(
        "tagged",
        "interface/tagged.gfx",
        b"spriteTypes = { spriteType = { name = GFX_TAG_tok textureFile = gfx/tagged.png } }",
    );
    machine.write_mod("tagged", "gfx/tagged.png", b"not a png");
    let raw = RawOrganisation {
        first_hour: 0,
        token: "tok".into(),
        name_key: String::new(),
        icon: String::new(),
        owner_tag: Some("TAG".into()),
        owner_unknown: false,
        state: None,
    };
    let (source, mut options) = machine.open();
    let drawn = |options: &crate::products::ContentOptions| {
        let shown = catalogue::catalogue(&source, options).unwrap();
        let (drawn, line) = fallback_line(|| organisation_picture(&shown, &raw, ICON_EDGE));
        let (key, image) = drawn.get().clone().unwrap().unwrap();
        // A memo hit names what the first draw named.
        let ((), again) = fallback_line(|| {
            drawn.get();
        });
        assert_eq!(line, again);
        (key, first_pixel(&image), line)
    };
    select(&source, &mut options, &[]);
    let (plain, colour, line) = drawn(&options);
    assert!(plain.starts_with("sprite:GFX_tok#"), "{plain}");
    assert_eq!((colour, line), (RED, None));
    select(&source, &mut options, &["tagged"]);
    let (key, colour, line) = drawn(&options);
    assert_eq!(key, plain, "the recording's own picture, the same pixels");
    assert_eq!(colour, RED, "the recording's own choice");
    assert!(
        line.as_ref().is_some_and(|line| line.contains("organisation tok icon")),
        "{line:?}"
    );
}
