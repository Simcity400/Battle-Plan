//! Every public reply the index answers equals the reply the whole scan of the
//! independent hourly reference answered, field for field, refusals included, however
//! the branch was split into blocks, spilled into chunks or resumed.

use super::tests::{authored, fixture_source, replay_blocks, restore};
use super::*;
use crate::test_fixtures::{
    LineFixture, OutputFixture, PRODUCTION_HOUR as FIRST, production_frame,
};
use replay_data::rows::{
    CountryLinkRow, CountryRow, ProductionLineRow, TableRows, VariantStatsRow,
};
use replay_frame::frame::TableView;

pub(super) type Reply = Result<serde_json::Value, String>;

struct Budget(usize);
impl Drop for Budget {
    fn drop(&mut self) {
        crate::prepared::pages::TEST_BATCH_BYTES.set(self.0);
    }
}

/// What each product answers for one tag: overviews, every window, and each key's factors
/// at each origin.
pub(super) struct Requests {
    pub tag: String,
    pub windows: std::ops::RangeInclusive<i32>,
    pub keys: Vec<String>,
    pub groups: Vec<&'static str>,
    pub origins: Vec<Option<i32>>,
}

fn reply<T: Serialize>(result: io::Result<T>) -> Reply {
    result
        .map(|value| serde_json::to_value(value).unwrap())
        .map_err(|error| format!("{:?}: {error}", error.kind()))
}

pub(super) fn replies(source: &Source, requests: &Requests) -> Vec<(String, Reply)> {
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut out = Vec::new();
    for exact in [None, Some(true)] {
        out.push((
            format!("lines exact={exact:?}"),
            reply(ProductionLines::load(
                source,
                &ProductionLinesParams {
                    branch_id: "1".into(),
                    tag: requests.tag.clone(),
                    exact,
                },
                &context,
                &flag,
            )),
        ));
    }
    for window in requests.windows.clone() {
        out.push((
            format!("detail {window}"),
            reply(ProductionLineDetail::load(
                source,
                &ProductionDetailParams {
                    branch_id: "1".into(),
                    tag: requests.tag.clone(),
                    window,
                },
                &context,
                &flag,
            )),
        ));
    }
    for key in &requests.keys {
        for group in &requests.groups {
            for origin in &requests.origins {
                out.push((
                    format!("breakdown {key} {group} {origin:?}"),
                    reply(ProductionLineBreakdown::load(
                        source,
                        &ProductionBreakdownParams {
                            branch_id: "1".into(),
                            tag: requests.tag.clone(),
                            key: key.clone(),
                            group: (*group).into(),
                            origin_hour: *origin,
                        },
                        &context,
                        &flag,
                    )),
                ));
            }
        }
    }
    out
}

/// How a preparation publishes the branch: chunks spilled at every observed hour, and a
/// resume from the store after every block.
#[derive(Clone, Copy, Debug)]
pub(super) struct Mode {
    pub spill: bool,
    pub resume: bool,
}

const MODES: [Mode; 4] = [
    Mode {
        spill: false,
        resume: false,
    },
    Mode {
        spill: true,
        resume: false,
    },
    Mode {
        spill: false,
        resume: true,
    },
    Mode {
        spill: true,
        resume: true,
    },
];

pub(super) fn prepare(replay: &Replay, tags: &[&str], mode: Mode) -> fixtures::FixturePages {
    let flag = AtomicBool::new(false);
    let _budget = Budget(
        crate::prepared::pages::TEST_BATCH_BYTES.replace(if mode.spill { 1 } else { usize::MAX }),
    );
    let mut prepared = Prepared::new(replay, 1, tags.iter().map(|tag| (*tag).to_owned()), None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, replay, block, &flag).unwrap();
        if mode.resume {
            prepared = restore(&mut pages, replay);
            assert_eq!(prepared.next_block(replay).unwrap(), position + 1);
        }
    }
    assert!(prepared.directory.coverage.complete);
    pages
}

/// Each tag's replies from every preparation mode equal the reference's.
fn assert_parity(
    replay: &Replay,
    tags: &[&str],
    natives: &[(u32, u32)],
    requests: impl Fn(&str, &ScannedLines) -> Requests,
) {
    let flag = AtomicBool::new(false);
    let blocks = branch_blocks(replay, 1);
    let natives: BTreeMap<(u32, u32), ScannedLines> = natives
        .iter()
        .map(|&holder| {
            (
                holder,
                super::super::reference::scan(replay, &blocks, "", None, Some(holder), &flag)
                    .unwrap(),
            )
        })
        .collect();
    let scans: Vec<(&str, ScannedLines)> = tags
        .iter()
        .map(|tag| {
            (
                *tag,
                super::super::reference::scan(replay, &blocks, tag, None, None, &flag).unwrap(),
            )
        })
        .collect();
    for mode in MODES {
        let source = fixture_source(replay, prepare(replay, tags, mode));
        for (tag, scan) in &scans {
            let requests = requests(tag, scan);
            let actual = replies(&source, &requests);
            let expected = super::super::reference::with_native(
                &source,
                scan.clone(),
                natives.clone(),
                || replies(&source, &requests),
            );
            assert_eq!(actual.len(), expected.len());
            for ((label, actual), (_, expected)) in actual.iter().zip(&expected) {
                assert_eq!(actual, expected, "{tag} {label}: {mode:?}");
            }
        }
    }
}

/// The windows a branch's hours touch, with one empty window on each side.
pub(super) fn windows(replay: &Replay) -> std::ops::RangeInclusive<i32> {
    let blocks = branch_blocks(replay, 1);
    let first = blocks.first().unwrap().first_hour.div_euclid(DETAIL_HOURS);
    let last = blocks.last().unwrap().last_hour.div_euclid(DETAIL_HOURS);
    first - 1..=last + 1
}

/// Every line key the reference's scan holds, an appearance it never had and a handle it
/// never listed.
fn keys(scan: &ScannedLines) -> Vec<String> {
    scan.lines
        .iter()
        .map(|line| line.key.clone())
        .chain(["5#99".to_owned(), "123456#1".to_owned()])
        .collect()
}

/// Forty days of hours with a census each day, across the first detail window's end:
/// line 5 runs throughout and stops changing on day 20, so one run spans the window
/// boundary; line 6 switches variant and factories in place, then ends on day 20 with its
/// final output published at the census after it was deleted; handle 7 ends on day 8 and
/// is listed again from day 12, a second appearance; line 8 is made and deleted between
/// two censuses; output goes unread on day 14 and lines are unavailable for hours of day
/// 17, after which line 5 resumes with the same values; design reads are dated every five
/// hours without changing on days 10 to 13; line 5's cost turns negative and its speed
/// overflows on day 22; Germany's holder changes on day 30; a country with a multibyte tag
/// holds a line from day 2.
pub(super) fn branch_frames() -> Vec<DecodedFrame> {
    (0..40 * 24)
        .map(|offset| {
            let day = offset / 24;
            let hour = offset % 24;
            let census = hour == 0;
            let germany = if day < 30 { 9001 } else { 9002 };
            let mut lines: Vec<LineFixture> = vec![(5, 70, 8, i64::from(day.min(20)) * 1_000)];
            if (3..20).contains(&day) {
                lines.push((
                    6,
                    if day < 10 { 70 } else { 71 },
                    if day < 15 { 2 } else { 4 },
                    0,
                ));
            }
            if (5..8).contains(&day) || day >= 12 {
                lines.push((7, 71, 1, 0));
            }
            let mut output: Vec<OutputFixture> = vec![(5, 70, i64::from(offset), true, false)];
            if (3..20).contains(&day) {
                output.push((
                    6,
                    if day < 10 { 70 } else { 71 },
                    i64::from(day),
                    true,
                    false,
                ));
            }
            match day {
                8 => output.push((7, 71, 4, false, true)),
                20 => output.push((6, 71, 99, true, true)),
                25 => output.push((8, 70, 3, true, true)),
                _ => {}
            }
            let unread = day == 14 && (1..6).contains(&hour);
            let mut frame = production_frame(
                FIRST + offset,
                census,
                census,
                germany,
                &lines,
                (!unread).then_some(output.as_slice()),
            );
            if day == 17 && (4..7).contains(&hour) {
                *frame.tables[TableId::ProductionLines.index()] = TableView::Unavailable {
                    reason: "synthetic unavailable census".into(),
                };
            }
            if (10..14).contains(&day) {
                *frame.tables[TableId::VariantStats.index()] =
                    TableView::Complete(TableRows::VariantStats(
                        [70, 71]
                            .map(|variant| VariantStatsRow {
                                variant,
                                organisation: 79,
                                organisation_trait_count: 2,
                                ..Default::default()
                            })
                            .to_vec(),
                    ));
                frame.tables[TableId::VariantStats.index()] = frame.tables
                    [TableId::VariantStats.index()]
                .clone()
                .with_sampled_at(Some(FIRST + offset / 5 * 5));
            }
            if let TableView::Complete(TableRows::ProductionLines(rows)) =
                &mut *frame.tables[TableId::ProductionLines.index()]
            {
                if day == 22
                    && let Some(row) = rows.iter_mut().find(|row| row.handle == 5)
                {
                    row.cost_raw = -5;
                    row.speed_raw = i64::MAX;
                }
                if day >= 2 {
                    rows.push(ProductionLineRow {
                        handle: 41,
                        country: 41,
                        variant: 70,
                        cost_raw: 50_000,
                        speed_raw: 1,
                        active_factories: day % 3,
                        ..Default::default()
                    });
                    rows.sort_by_key(|row| row.handle);
                }
            }
            if day >= 2 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.push(CountryRow {
                        id: 41,
                        canonical_slot: 3,
                        tag: "Ünï".as_bytes().to_vec(),
                        ..Default::default()
                    });
                    rows.sort_by_key(|row| row.id);
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.push(CountryLinkRow {
                        id: 41,
                        canonical_slot: 3,
                    });
                    rows.sort_by_key(|row| row.id);
                }
            }
            frame
        })
        .collect()
}

#[test]
fn every_reply_equals_the_reference_across_windows_blocks_spills_and_resumes() {
    let frames = branch_frames();
    for hours in [24 * 40, 100, 24 * 7 + 5] {
        let replay = replay_blocks(frames.clone(), hours);
        let windows = windows(&replay);
        assert!(windows.clone().count() >= 4, "the branch crosses a window");
        assert_parity(
            &replay,
            &["GER", "ITA", "Ünï", "NONE"],
            &[(7, 9001), (7, 9002)],
            |tag, scan| Requests {
                tag: tag.to_owned(),
                windows: windows.clone(),
                keys: keys(scan),
                groups: vec!["output", "no-such-factor"],
                origins: vec![None, Some(FIRST), Some(FIRST + 24 * 35)],
            },
        );
    }
}

#[test]
fn modifier_factors_equal_the_reference_for_every_factor_read_and_origin() {
    let lines = &[(5, 70, 8, 0), (6, 71, 2, 0), (7, 72, 1, 0)];
    let mut frames = Vec::new();
    for offset in 0..24 * 4 {
        let census = offset % 24 == 0;
        let listed: &[LineFixture] = if (24..48).contains(&offset) {
            &lines[1..]
        } else {
            lines
        };
        let mut frame = authored(
            FIRST + offset,
            listed,
            &[(5, 70, i64::from(offset), true, (24..48).contains(&offset))],
        );
        if !census {
            frame.metrics.table_ns.fill(0);
        }
        frames.push(frame);
    }
    for hours in [24 * 4, 30] {
        let replay = replay_blocks(frames.clone(), hours);
        let windows = windows(&replay);
        assert_parity(&replay, &["GER"], &[(7, 9001)], |_, scan| Requests {
            tag: "GER".to_owned(),
            windows: windows.clone(),
            keys: keys(scan),
            groups: vec![
                "efficiency",
                "start",
                "cap",
                "gain",
                "daily",
                "retention-0",
                "retention-1",
                "retention-2",
                "retention-3",
                "output",
                "power",
                "resources",
                "conversion",
                "refit",
                "no-such-factor",
            ],
            origins: vec![None, Some(FIRST)],
        });
    }
}

#[test]
fn fifteen_hundred_countries_with_unequal_lines_answer_as_the_reference() {
    let mut frames = Vec::new();
    for offset in 0..24 * 3 {
        let mut frame = super::tests::scaled_production(1500, 1);
        frame.hour = FIRST + offset;
        let TableView::Complete(TableRows::ProductionLines(rows)) =
            &mut *frame.tables[TableId::ProductionLines.index()]
        else {
            unreachable!()
        };
        // Country `i` holds `i % 7` lines, one more of them from the second day.
        *rows = (0..1500_u32)
            .flat_map(|country| {
                let count = country % 7 + u32::from(offset >= 24 && country % 5 == 0);
                (0..count).map(move |line| ProductionLineRow {
                    handle: u64::from(country) * 16 + u64::from(line) + 1,
                    country: 100_000 + country,
                    variant: 70,
                    mio: u64::from(1 + country),
                    kind: 0,
                    active_factories: 1 + i32::try_from(line).unwrap(),
                    efficiency_raw: i64::from(offset / 24),
                    ..Default::default()
                })
            })
            .collect();
        rows.sort_by_key(|row| row.handle);
        if offset % 24 != 0 {
            frame.metrics.table_ns.fill(0);
        }
        frames.push(frame);
    }
    let replay = replay_blocks(frames, 30);
    let tags: Vec<String> = [0, 1, 6, 7, 699, 1499]
        .map(|index| String::from_utf8(crate::test_fixtures::scale_tag(index)).unwrap())
        .to_vec();
    let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
    let windows = windows(&replay);
    assert_parity(&replay, &tags, &[], |tag, scan| Requests {
        tag: tag.to_owned(),
        windows: windows.clone(),
        keys: keys(scan).into_iter().take(3).collect(),
        groups: vec!["output"],
        origins: vec![None],
    });
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, [], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
    }
    assert_eq!(prepared.native.len(), 1500);
}
