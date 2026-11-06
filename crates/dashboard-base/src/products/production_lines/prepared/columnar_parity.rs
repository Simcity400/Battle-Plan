//! The fold fed from a native walk of the fold's tables ([`super::super::columnar`]) answers
//! every reply as the fold of the row recording of the same hours does: in one segment, with
//! segments starting inside and between row blocks and its publications cut mid-branch, and
//! taken up again from what a cut published.

use super::parity::{self, Mode, Requests};
use super::tests::{authored, fixture_source, replay_blocks};
use super::*;
use crate::products::production_lines::columnar::{Branch, Projector, needs, tables};
use dashboard_world::World;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{TableSchema, tables as schema},
    scope::CaptureMode,
};
use replay_frame::frame::TableView;

/// The frames of `replay`'s blocks as the row fold reads them.
fn decoded(replay: &Replay) -> Vec<DecodedFrame> {
    let mut frames = Vec::new();
    for block in &replay.index {
        replay
            .read_block_recorded_alone(
                block,
                tables(false),
                |_, frame| {
                    frames.push(frame.clone());
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    frames
}

/// Writes `T`'s view of `frame` as the native recorder writes it: read where the frame read it,
/// and sampled at the hour its view was.
fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let index = T::ID.index();
    let read = frame.metrics.table_ns[index] != 0;
    let view = &frame.tables[index];
    let sample = match view.sampled_at() {
        Some(hour) if read && hour == frame.hour => Sample::FromReadBit,
        Some(hour) => Sample::Carried(hour),
        None => Sample::Reset,
    };
    let input = match &**view {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

/// Writes every table the fold reads without icon content.
fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame) {
    macro_rules! tables { ($($table:ident),* $(,)?) => { $(write::<schema::$table>(walk, frame);)* }; }
    tables!(
        Campaign,
        ProductionLines,
        ProductionLineOutput,
        IndustrialOrganisations,
        IndustrialOrganisationDetails,
        IndustrialOrganisationPolicies,
        VariantStats,
        CountryProductionModifiers,
        ProductionLineModifiers,
        ModifierDefinitions,
        Countries,
        CountryLinks,
        CountryAppearance,
        LexicalTokens,
        VariantDefinitions,
        VariantModules,
        VariantUpgrades,
        VariantChanges,
    );
}

/// How a native run walks and publishes the branch: its segments' first hours past the walk's
/// first, the hours its batches end after, and the cut it is taken up again after.
struct Run {
    label: &'static str,
    starts: Vec<i32>,
    cuts: Vec<i32>,
    resume: Option<i32>,
}

/// The fold's pages of `frames`, walked natively as `run` says.
fn native(replay: &Replay, frames: &[DecodedFrame], tags: &[&str], run: &Run) -> fixtures::FixturePages {
    let flag = AtomicBool::new(false);
    let fresh = || Projector::new(FormatVersion::CURRENT, None).unwrap();
    let needs = needs(false).unwrap();
    let mut world: Option<World> = None;
    let mut pages = fixtures::FixturePages::default();
    let mut walk = Synthetic::new(tables(false), 1, frames[0].hour).unwrap();
    let mut projector = fresh();
    let mut branch = Branch::new(replay, 1, tags.iter().map(|tag| (*tag).to_owned()), None);
    let last = frames.last().unwrap().hour;
    let mut resumed = false;
    // A segment's walk ends before the next begins, and where its batch is published.
    let end = |projector: &mut Projector, branch: &mut Branch, pages: &mut fixtures::FixturePages| {
        let mut ended = Vec::new();
        projector.end(&mut ended).unwrap();
        for hour in ended {
            branch.observe(pages, hour, None).unwrap();
        }
    };
    for (ordinal, frame) in (0..).zip(frames) {
        let cold = std::mem::take(&mut resumed) || run.starts.contains(&frame.hour);
        if cold {
            end(&mut projector, &mut branch, &mut pages);
            projector = fresh();
        }
        write_hour(&mut walk, frame);
        let mut hours = Vec::new();
        walk.end(|state, manifest, _, first| {
            let starts = first || cold;
            let world = World::at(
                &mut world,
                starts,
                &needs,
                (FormatVersion::CURRENT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        for hour in hours {
            branch.observe(&mut pages, hour, None).unwrap();
        }
        if frame.hour == last {
            end(&mut projector, &mut branch, &mut pages);
            branch.publish(&mut pages, replay, Some(last), &flag).unwrap();
        } else if run.cuts.contains(&frame.hour) {
            end(&mut projector, &mut branch, &mut pages);
            projector = fresh();
            resumed = true;
            branch.publish(&mut pages, replay, None, &flag).unwrap();
            if run.resume == Some(frame.hour) {
                branch = Branch::resume(&mut pages, replay, 1, None).unwrap();
                resumed = true;
            }
        }
    }
    pages
}

/// Every line key `source` answers for `tag`, and a key it never listed.
fn line_keys(source: &Source, tag: &str) -> Vec<String> {
    let reply = ProductionLines::load(
        source,
        &ProductionLinesParams {
            branch_id: "1".into(),
            tag: tag.into(),
            exact: Some(true),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    );
    let mut keys: Vec<String> = reply
        .map(|reply| reply.lines.into_iter().map(|line| line.key).collect())
        .unwrap_or_default();
    keys.push("123456#1".to_owned());
    keys
}

/// Every reply of `tags` from each native run of `frames`, in blocks of `hours`, equals the
/// row fold's.
fn assert_native_parity(
    frames: Vec<DecodedFrame>,
    hours: usize,
    tags: &[&str],
    groups: &[&'static str],
    runs: impl Fn(i32) -> Vec<Run>,
) {
    let replay = replay_blocks(frames, hours);
    let decoded = decoded(&replay);
    let first = decoded[0].hour;
    let reference = fixture_source(
        &replay,
        parity::prepare(&replay, tags, Mode {
            spill: false,
            resume: false,
        }),
    );
    let windows = parity::windows(&replay);
    for run in runs(first) {
        let source = fixture_source(&replay, native(&replay, &decoded, tags, &run));
        for &tag in tags {
            let requests = Requests {
                tag: tag.to_owned(),
                windows: windows.clone(),
                keys: line_keys(&reference, tag),
                groups: groups.to_vec(),
                origins: vec![None, Some(first), Some(decoded.last().unwrap().hour)],
            };
            let expected = parity::replies(&reference, &requests);
            let actual = parity::replies(&source, &requests);
            assert_eq!(actual.len(), expected.len());
            for ((label, actual), (_, expected)) in actual.iter().zip(&expected) {
                assert_eq!(actual, expected, "{} {tag} {label}: blocks of {hours}", run.label);
            }
        }
    }
}

#[test]
fn a_native_walk_answers_every_reply_as_the_row_recording() {
    for hours in [24 * 40, 100] {
        assert_native_parity(
            parity::branch_frames(),
            hours,
            &["GER", "ITA", "Ünï", "NONE"],
            &["output", "no-such-factor"],
            |first| {
                vec![
                    Run {
                        label: "one segment",
                        starts: Vec::new(),
                        cuts: Vec::new(),
                        resume: None,
                    },
                    Run {
                        label: "segments and cuts",
                        starts: vec![first + 5, first + 24, first + 24 * 17 + 5, first + 24 * 30],
                        cuts: vec![first + 24 * 10, first + 24 * 17 + 4, first + 24 * 20],
                        resume: None,
                    },
                    Run {
                        label: "resumed",
                        starts: vec![first + 24 * 3],
                        cuts: vec![first + 24 * 8, first + 24 * 15],
                        resume: Some(first + 24 * 15),
                    },
                ]
            },
        );
    }
}

#[test]
fn a_native_walk_answers_every_modifier_factor_as_the_row_recording() {
    let lines = &[(5, 70, 8, 0), (6, 71, 2, 0), (7, 72, 1, 0)];
    let frames: Vec<DecodedFrame> = (0..24 * 4)
        .map(|offset| {
            let census = offset % 24 == 0;
            let listed: &[(u64, u64, i32, i64)] = if (24..48).contains(&offset) {
                &lines[1..]
            } else {
                lines
            };
            let mut frame = authored(
                crate::test_fixtures::PRODUCTION_HOUR + offset,
                listed,
                &[(5, 70, i64::from(offset), true, (24..48).contains(&offset))],
            );
            if !census {
                frame.metrics.table_ns.fill(0);
            }
            frame
        })
        .collect();
    let groups: Vec<&'static str> = super::super::breakdown::FACTORS
        .iter()
        .map(|&(group, _)| group)
        .chain(["no-such-factor"])
        .collect();
    assert_native_parity(frames, 30, &["GER"], &groups, |first| {
        vec![
            Run {
                label: "one segment",
                starts: Vec::new(),
                cuts: Vec::new(),
                resume: None,
            },
            Run {
                label: "a segment a day, resumed",
                starts: vec![first + 24, first + 48, first + 72],
                cuts: vec![first + 47],
                resume: Some(first + 47),
            },
        ]
    });
}

/// A native branch prepared before a later branch named a new tag stays current: its frontier
/// says so, where a row branch's directory must name every tag the recording does.
#[test]
fn a_native_directory_needs_no_tags_a_later_preparation_names() {
    let frames = parity::branch_frames();
    let replay = replay_blocks(frames, 24 * 40);
    let decoded = decoded(&replay);
    let run = Run {
        label: "one segment",
        starts: Vec::new(),
        cuts: Vec::new(),
        resume: None,
    };
    let mut pages = native(&replay, &decoded, &["GER"], &run);
    let bytes = pages.read_at(&directory_section(1), 0).unwrap().unwrap();
    let mut directory = fixtures::read_directory(&bytes).unwrap();
    assert!(directory.native && directory.coverage.complete);
    assert!(!directory.tags.iter().any(|tag| tag == "NEW"));
    assert!(names_every(&directory, ["GER", "NEW"]));
    directory.native = false;
    assert!(!names_every(&directory, ["GER", "NEW"]));
    assert!(names_every(&directory, ["GER"]));
}
