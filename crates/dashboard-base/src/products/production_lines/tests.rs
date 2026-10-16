use super::*;
use crate::test_fixtures::{
    CAMPAIGN, LineFixture, OutputFixture, PRODUCTION_HOUR as FIRST, equipment_types,
    production_frame, records,
};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{CountryLinkRow, TableId, TableRows},
};

#[test]
fn native_producer_scan_follows_renames_excludes_reused_tags_and_keeps_real_gaps() {
    use replay::rows::{CountryRow, ProductionLineRow};
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay::rows::{
        CountryProductionModifiersRow, LexicalTokenRow, ModifierBreakdownNode,
        ModifierDefinitionRow, ProductionLineModifiersRow, ProductionModifierTree,
    };
    let mut frame = production_frame(FIRST, true, true, 9001, &[(5, 70, 1, 3276800)], None);
    let TableView::Complete(TableRows::ProductionLines(rows)) =
        &mut *frame.tables[TableId::ProductionLines.index()]
    else {
        panic!("lines");
    };
    rows.iter_mut().find(|row| row.handle == 5).unwrap().country = 9001;
    let keys = replay::fixed_layout::PRODUCTION_MODIFIERS.keys;
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
        CountryParams, LoadContext, Product,
        production_lines::{ProductionLines, scan},
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
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{
        equipment_icon::{Icon, IconContent},
        production_lines::scan,
    };
    use installed_content::{equipment_graphics, technology::Enablers};
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
    let icons: Vec<(i32, Result<Option<Icon>, String>)> = scanned.icons[&71].clone();
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
    let mut shown = std::collections::BTreeMap::new();
    let (entries, _, _) =
        crate::products::production_lines::variant_entries(&scanned, None, true, &mut shown);
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{
        equipment_icon::{Icon, IconContent},
        production_lines::scan,
    };
    use installed_content::{equipment_graphics, technology::Enablers};
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
    assert_eq!(
        scanned.icons[&70],
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
                Err("the variant's definition is not recorded yet".into()),
            ),
            (FIRST + 24, Ok(Some(Icon::Sprite("GFX_rifle".into())))),
        ],
    );
    let mut shown = std::collections::BTreeMap::new();
    let mut texts = installed_content::localisation::Localisation::default();
    texts.language = "english".into();
    let (entries, unnamed, _) = variant_entries(&scanned, Some(&texts), true, &mut shown);
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
        CountryParams, LoadContext, Product,
        equipment::EQUIPMENT_DECIMALS,
        production_lines::{
            DETAIL_HOURS, ProductionDetailParams, ProductionLineDetail, ProductionLines,
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
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let load = |params: &CountryParams| {
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
    let other = load(&CountryParams {
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
    use replay::rows::{
        CountryProductionModifiersRow, ModifierBreakdownNode, ProductionLineModifiersRow,
        ProductionModifierTree,
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
    let mut shown = std::collections::BTreeMap::new();
    let (entries, _, _) =
        crate::products::production_lines::variant_entries(&scanned, None, false, &mut shown);
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
    use crate::products::{CountryParams, LoadContext, Product, production_lines::ProductionLines};
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
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
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
        CountryParams, LoadContext, Product,
        production_lines::{ProductionLines, scan},
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
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
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
        CountryParams, LoadContext, Product,
        equipment::Texts,
        production_lines::{
            ProductionLines, RawOrganisation, organisation_icon, organisation_name, scan,
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
            replay::rows::IndustrialOrganisationRow {
                organisation: 79,
                country: 11,
                owner: 333,
                token: 602,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay::rows::LexicalTokenRow {
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
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
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

/// A picture decoded from one content stack is never served for another, even
/// when its decode finishes after the content changed.
#[test]
fn pictures_are_served_only_for_the_catalogue_they_were_decoded_from() {
    let (a, b) = (Arc::new("folder A"), Arc::new("folder B"));
    let mut images: Option<Images<&str>> = None;
    keep_image(&mut images, &b, "path:x.dds".into(), Some("B".into()));
    // A's decode, started before the switch, lands late: B never reads it as its own.
    keep_image(&mut images, &a, "path:x.dds".into(), Some("A".into()));
    assert_eq!(cached_image(&images, &b, "path:x.dds"), None);
    assert_eq!(
        cached_image(&images, &a, "path:x.dds"),
        Some(Some("A".into()))
    );
    keep_image(&mut images, &b, "path:x.dds".into(), Some("B".into()));
    assert_eq!(
        cached_image(&images, &b, "path:x.dds"),
        Some(Some("B".into()))
    );
    assert_eq!(cached_image(&images, &a, "path:x.dds"), None);
    // A cached miss is the owner's own too.
    keep_image(&mut images, &b, "path:gone.dds".into(), None);
    assert_eq!(cached_image(&images, &b, "path:gone.dds"), Some(None));
    assert_eq!(cached_image(&images, &a, "path:gone.dds"), None);
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
