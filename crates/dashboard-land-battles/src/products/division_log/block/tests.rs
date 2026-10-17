//! Block folds of the division log against hourly observation: the same
//! pages, totals and saved scan, through a division seen again in a later
//! block, presentations dated before a block's first template read, a block
//! whose census is unavailable at its first hour, a block whose country
//! tables are not read at its first hour, a gap and a removal witness; with
//! only each block's first hour observed on the carried scan where it
//! continues as its extract did.
use super::*;
use dashboard_base::{
    prepared::{prepass::extract_alone, series::tests::Fixture},
    test_fixtures::{CAMPAIGN, continuation_records, records},
};
use replay::{
    frame::TableView,
    rows::{
        CountryLinkRow, CountryRow, DestroyedDivisionsRow, DivisionEquipmentDeletionRow,
        DivisionRow, DivisionStatusRow, DivisionTemplateRow, LandEventCoverageRow, NameOperands,
        Recorded, TableRows,
    },
};
use std::io::Read;

fn handle(id: u32) -> u64 {
    u64::from(id) << 32 | 4713
}

fn template(id: u32) -> u64 {
    u64::from(id % 2) << 32 | 4714
}

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

/// Whether `hour` reads the template table, by its recorded cadence.
fn template_read(hour: i32) -> bool {
    replay::cadence::cadence_in(
        TableId::DivisionTemplates,
        replay::format::FormatVersion::CURRENT,
    )
    .is_refresh(hour)
}

/// The scenario's hours, from `base` (the hour after a template read): its
/// blocks, a gap of two hours before the third, and each hour's live
/// divisions and events.
struct Scenario {
    base: i32,
}

impl Scenario {
    fn new() -> Self {
        let read = (100..148)
            .find(|&hour| template_read(hour))
            .expect("a template read every day");
        Self { base: read + 1 }
    }

    /// The blocks' hours: a day ending at the next template read, four
    /// hours without one, then after the gap a day starting with an
    /// unavailable census, and a day starting without country links.
    fn blocks(&self) -> Vec<Vec<i32>> {
        let base = self.base;
        vec![
            (base..base + 24).collect(),
            (base + 24..base + 28).collect(),
            (base + 30..base + 54).collect(),
            (base + 54..base + 78).collect(),
        ]
    }

    /// Divisions live at `hour`: 1 throughout; 2 until +10 and again from
    /// +25 (a later block); 3 from +5; 4 from +12 until its destruction at
    /// +18; 5 from +26; 6 from +40 until its transfer at +65.
    fn live(&self, hour: i32) -> Vec<u32> {
        let at = hour - self.base;
        let mut live = vec![1];
        if !(10..25).contains(&at) {
            live.push(2);
        }
        if at >= 5 {
            live.push(3);
        }
        if (12..=18).contains(&at) {
            live.push(4);
        }
        if at >= 26 {
            live.push(5);
        }
        if (40..=65).contains(&at) {
            live.push(6);
        }
        live
    }

    fn frame(&self, hour: i32) -> DecodedFrame {
        let at = hour - self.base;
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(0));
        set(
            &mut frame,
            TableRows::Countries(
                (1..=3)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("C{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            &mut frame,
            TableRows::CountryLinks(
                (1..=3)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ),
        );
        let live = self.live(hour);
        // Every division moves, and its status changes, every hour.
        set(
            &mut frame,
            TableRows::Divisions(
                live.iter()
                    .map(|&id| DivisionRow {
                        id: handle(id),
                        roster_owner: id % 3 + 1,
                        logical_owner: 1,
                        expeditionary_owner: u32::MAX,
                        province: id * 1000 + at as u32,
                        template: template(id),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            &mut frame,
            TableRows::DivisionStatus(
                live.iter()
                    .map(|&id| DivisionStatusRow {
                        division: handle(id),
                        supply_ratio: i64::from(at),
                        naming: Recorded::Value(Some(NameOperands {
                            custom: format!("Unit {id}").into_bytes(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        // The second template is renamed at +45.
        set(
            &mut frame,
            TableRows::DivisionTemplates(
                (0..2)
                    .map(|id| DivisionTemplateRow {
                        template: template(id),
                        owner: 1,
                        name: if id == 1 && at >= 45 {
                            b"Renamed".to_vec()
                        } else {
                            format!("Template {id}").into_bytes()
                        },
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(&mut frame, TableRows::DivisionTemplateIcons(Vec::new()));
        let (deletions, destroyed) = match at {
            18 => (
                vec![DivisionEquipmentDeletionRow {
                    division_handle: handle(4),
                    country: 2,
                    expeditionary_owner: u32::MAX,
                    disposition: Disposition::Loss,
                    cause: Some(Cause::DivisionDestroyed),
                }],
                vec![DestroyedDivisionsRow {
                    id: 1,
                    division: handle(4),
                    owner: 2,
                    killer_country: 3,
                    killer_division: Some(handle(9)),
                    province: 17,
                    hour,
                    ..Default::default()
                }],
            ),
            65 => (
                vec![DivisionEquipmentDeletionRow {
                    division_handle: handle(6),
                    country: 1,
                    expeditionary_owner: u32::MAX,
                    disposition: Disposition::VolunteerTransferred,
                    cause: None,
                }],
                Vec::new(),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        set(&mut frame, TableRows::DivisionEquipmentDeletions(deletions));
        set(&mut frame, TableRows::DestroyedDivisions(destroyed));
        set(
            &mut frame,
            TableRows::LandEventCoverage(vec![LandEventCoverageRow {
                from_hour: hour - 1,
                to_hour: hour,
                complete: true,
                ..Default::default()
            }]),
        );
        // The third block's first hour has no census; the fourth's, and +60,
        // no country links.
        if at == 30 {
            *frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
                reason: "synthetic census failure".into(),
            };
        }
        if at == 54 || at == 60 {
            *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                reason: "synthetic missing links".into(),
            };
        }
        frame
    }

    fn replay(&self) -> Replay {
        let blocks = self.blocks();
        let frames = |hours: &[i32]| hours.iter().map(|&hour| self.frame(hour)).collect();
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames(&blocks[0]))]).unwrap();
        for hours in &blocks[1..] {
            replay = replay
                .append(&[continuation_records(&replay, 1, frames(hours))])
                .unwrap();
        }
        replay
    }
}

fn source(replay: Replay) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "division-blocks".into(),
            revision: "1".into(),
        },
        "division-blocks.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Every range of the division log's sections of branch 1.
fn pages(fixture: &Fixture, blocks: usize) -> BTreeMap<(String, u64), Vec<u8>> {
    let mut pages = BTreeMap::new();
    for section in [
        "division-log-1",
        "division-log-manifest-1",
        "division-log-counts-1",
        "division-log-count-keys-1-catalogue",
    ] {
        let count = fixture
            .store
            .range_count(&fixture.key, section)
            .unwrap()
            .unwrap_or_else(|| panic!("{section} was published"));
        for range in 0..count {
            let mut bytes = Vec::new();
            fixture
                .store
                .read(&fixture.key, section, range)
                .unwrap()
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            pages.insert((section.to_owned(), range), bytes);
        }
    }
    assert_eq!(
        pages
            .keys()
            .filter(|(name, _)| name == "division-log-1")
            .count(),
        blocks,
        "a window per block"
    );
    pages
}

/// Hourly preparation's division scan: every hour observed with the change
/// facts of its block alone, each block published at its end.
fn hourly(source: &Source, fixture: &Fixture) -> Scan {
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut scan = Scan::new(replay.format);
    for (ordinal, index) in replay.index.iter().enumerate() {
        let mut tracker = FrameChangeTracker::new();
        replay
            .read_block_alone(
                index,
                &TABLES,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                    scan.configure(replay.format);
                    scan.prepare_changed(frame, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let mut writer = fixture.writer();
        scan.publish_block(source, &context, &mut writer, 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
    }
    scan
}

/// The family's fold of every block, committing after every `commit` blocks
/// and reading its saved scan back there; how each block folded (None for a
/// block observed hour by hour because its extract stopped).
fn folded(source: &Source, fixture: &Fixture, commit: usize) -> (Vec<u8>, Vec<Option<Folded>>) {
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut own = OwnBranch::load(None::<&[u8]>, replay.format).unwrap();
    let mut outcomes = Vec::new();
    for (at, index) in replay.index.iter().enumerate() {
        let mut observed = |_: i32, result: io::Result<()>| -> io::Result<bool> {
            result?;
            Ok(true)
        };
        let mut check = || -> io::Result<()> { Ok(()) };
        outcomes.push(match extract_alone(replay, index, &FAMILY) {
            Ok(facts) => Some(
                own.fold_facts(
                    facts,
                    source,
                    &context,
                    replay,
                    index,
                    &mut observed,
                    &mut check,
                )
                .unwrap()
                .0,
            ),
            Err(_) => {
                own.observe_hours(source, &context, replay, index, &mut observed, &mut check)
                    .unwrap();
                None
            }
        });
        if (at + 1) % commit == 0 || at + 1 == replay.index.len() {
            let mut writer = fixture.writer();
            own.publish_blocks(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let saved = own.state_json().unwrap();
            own = OwnBranch::load(Some(saved.as_slice()), replay.format).unwrap();
        }
    }
    (own.state_json().unwrap(), outcomes)
}

#[test]
fn division_block_folds_publish_and_save_what_hourly_observation_does() {
    let scenario = Scenario::new();
    let blocks = scenario.blocks();
    // The scenario's template reads: the first block's first hour follows
    // one and its last hour is one; the second block has none.
    assert!(!template_read(blocks[0][0]) && template_read(*blocks[0].last().unwrap()));
    assert!(blocks[1].iter().all(|&hour| !template_read(hour)));
    assert!(blocks[2][1..].iter().any(|&hour| template_read(hour)));
    let source = source(scenario.replay());
    let replay = source.replay_ref();
    assert_eq!(replay.index.len(), blocks.len());
    assert_eq!(
        replay.format.version(),
        replay::format::FormatVersion::CURRENT.version(),
        "template reads were found for the recording's format"
    );
    let reference = Fixture::new();
    let hourly = hourly(&source, &reference);
    let expected = pages(&reference, blocks.len());
    // Division 2 is seen again in the second block: no creation is logged.
    let creations = |division: u32| {
        hourly
            .events
            .iter()
            .filter(|event| {
                event.row.kind == DivisionLogKind::Created
                    && event.row.division == handle(division).to_string()
            })
            .count()
    };
    assert_eq!(creations(2), 0, "division 2 was seen at the baseline");
    assert_eq!(creations(5), 1);
    assert_eq!(creations(6), 1);
    let state = serde_json::to_value(&hourly).unwrap();
    for commit in [1, 2, 4] {
        let fixture = Fixture::new();
        let observed = dashboard_base::prepared::prepass::noted(OBSERVED);
        let (saved, outcomes) = folded(&source, &fixture, commit);
        let label = format!("committing every {commit} blocks");
        assert_eq!(pages(&fixture, blocks.len()), expected, "{label}");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&saved).unwrap(),
            state,
            "{label}: the saved scan"
        );
        // The first two blocks fold from their first hour; the third's
        // census is unavailable there, so its extract stops and it is
        // observed hour by hour; the fourth reads no country links at its
        // first hour, so its carried scan does not continue the extract's.
        assert_eq!(
            outcomes,
            [
                Some(Folded::Fast),
                Some(Folded::Fast),
                None,
                Some(Folded::Fallback)
            ],
            "{label}"
        );
        assert_eq!(
            dashboard_base::prepared::prepass::noted(OBSERVED) - observed,
            2 + blocks[2].len() + blocks[3].len(),
            "{label}: only the folded blocks' first hours are observed on the carried scan"
        );
    }
}

/// A block whose first hour reads the template table and continues: no
/// presentation carries the placeholder of an inherited template read hour,
/// and every one is dated as hourly observation dates it.
#[test]
fn division_block_folds_date_presentations_by_the_carried_template_read() {
    let scenario = Scenario::new();
    let source = source(scenario.replay());
    let reference = Fixture::new();
    let hourly = hourly(&source, &reference);
    let fixture = Fixture::new();
    let (saved, _) = folded(&source, &fixture, 4);
    let own: Scan = serde_json::from_slice(&saved).unwrap();
    let dates = |scan: &Scan| -> Vec<Option<i32>> {
        scan.events
            .iter()
            .flat_map(|event| {
                [
                    event.row.template_hour,
                    event.presentation.as_ref().and_then(|p| p.template_hour),
                ]
            })
            .chain(scan.presentation.iter().flat_map(|census| {
                std::iter::once(census.template_hour)
                    .chain(census.rows.iter().map(|row| row.template_hour))
            }))
            .collect()
    };
    assert!(!dates(&own).contains(&Some(INHERITED)));
    assert_eq!(dates(&own), dates(&hourly));
    // Events of the second block, which reads no template, are dated by the
    // first block's last template read.
    let read = *scenario.blocks()[0].last().unwrap();
    assert!(
        own.events
            .iter()
            .filter(|event| scenario.blocks()[1].contains(&event.row.read_hour))
            .all(|event| event.row.template_hour == Some(read))
    );
}

#[test]
fn newest_division_window_matches_hourly_rows_with_reappearing_handles_and_reuses_its_proof() {
    let scenario = Scenario::new();
    let (source, root) = dashboard_base::test_fixtures::file_source(scenario.replay());
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let mut scan = Scan::new(source.replay_ref().format);
    for block in &source.replay_ref().index {
        source
            .replay_ref()
            .read_block_alone(
                block,
                &TABLES,
                |_, frame| {
                    scan.observe_with_changes(frame, None);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    let params = DivisionLogParams {
        branch_id: "1".into(),
        window: scenario.blocks().len() - 1,
    };
    let expected = scan.reply(&source, &params, &context, &cancelled).unwrap();
    let (actual, _, _) = super::super::newest::query(&source, &params, &context, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let neighbour = DivisionLogParams {
        branch_id: "1".into(),
        window: params.window - 1,
    };
    let _ = super::super::newest::query(&source, &neighbour, &context, &cancelled).unwrap();
    let reads = dashboard_base::prepared::query::frames::reads(&source);
    let (again, refused) =
        dashboard_base::prepared::query::progressive::without_block_decodes(|| {
            super::super::newest::query(&source, &params, &context, &cancelled)
        });
    assert!(!refused);
    assert!(again.unwrap().is_some());
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&source),
        reads
    );
    for (product, params) in [
        ("division-log-manifest", serde_json::json!({"branchId":"1"})),
        (
            "division-log",
            serde_json::json!({"branchId":"1","window":params.window}),
        ),
    ] {
        let reply =
            super::super::prepared::first_view(&source, product, params, &context, &cancelled)
                .unwrap();
        assert!(reply.is_object());
        if product == "division-log-manifest" {
            assert!(!reply["pending"].as_array().unwrap().is_empty());
        } else {
            assert!(reply["pending"].as_array().unwrap().is_empty());
            assert!(reply["exact"].is_number());
        }
    }
    let _ = std::fs::remove_dir_all(root);
}
