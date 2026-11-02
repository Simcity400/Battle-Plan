use super::testing::{assert_published_as_walked, operate, publish_hours, write};
use crate::tests::{Changing, Mines, frame};
use replay::columnar_frames::synthetic::Synthetic;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

fn mines(walk: &mut Synthetic, frame: &DecodedFrame) {
    write::<tables::Countries>(walk, frame);
    write::<tables::CountryLinks>(walk, frame);
    write::<tables::NavalRegionalMines>(walk, frame);
}

/// From 100: values moving, a name appearing and ending, the mines unavailable for two hours,
/// an empty read, a gap after 112, and two owners' raw references linking to one slot.
fn scenario() -> Vec<DecodedFrame> {
    let mut frames = vec![
        frame(100, Some(&[(11, 3, 5)])),
        frame(101, Some(&[(11, 3, 5), (9001, 4, 6)])),
        frame(102, Some(&[(11, 3, 7), (21, 4, 2)])),
        frame(103, Some(&[(11, 3, 7), (21, 4, 2)])),
        frame(104, None),
        frame(105, None),
        frame(106, Some(&[(9001, 3, 7)])),
        frame(107, Some(&[(9001, 3, 7), (21, 9, 4)])),
        frame(108, Some(&[(9001, 3, 7), (21, 9, 4)])),
        frame(109, Some(&[])),
        frame(110, Some(&[(21, 4, 3)])),
        frame(111, Some(&[(21, 4, 3)])),
        frame(112, Some(&[(21, 4, 3)])),
        frame(116, Some(&[(21, 4, 3)])),
        frame(117, Some(&[(21, 4, 8), (11, 3, 1)])),
        frame(118, Some(&[(21, 4, 8), (11, 3, 1)])),
    ];
    for frame in &mut frames {
        frame.metrics.table_ns[replay_data::rows::TableId::NavalRegionalMines.index()] = 1;
    }
    frames
}

/// The native publication holds the runs, known hours, reads and names the walk's scan holds
/// of the same hours, whatever segments start, wherever the publisher cuts, after a resume, and
/// whether an observer gives hours whole or as their changes.
#[test]
fn a_native_publication_holds_what_the_walks_scan_holds() {
    let frames = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[118][..], None),
        (&[3, 9][..], &[103, 109, 118][..], Some(109)),
        (&[5, 6, 7, 14][..], &[100, 105, 112, 116, 118][..], Some(105)),
        (&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15][..], &[107, 118][..], Some(107)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (native, walked) =
            assert_published_as_walked::<Mines>(&frames, mines, starts, cuts, resume, &label);
        assert!(native.observer.observed <= walked.observer.observed, "{label}");
        assert_published_as_walked::<Changing<Mines>>(
            &frames,
            mines,
            starts,
            cuts,
            resume,
            &format!("{label}, as changes"),
        );
    }
}

/// Each key's changes over any ascending hours, read from its runs once, are those one lookup of
/// the key at each hour gives: inside runs, across gaps and unavailable hours, before the
/// publication's first hour and after its last.
#[test]
fn changes_read_from_runs_are_those_each_hours_lookup_gives() {
    let frames = scenario();
    let hours = operate::<Mines>(&frames, &[3, 9], mines);
    let published = publish_hours::<Mines>(hours, frames[0].branch, &[103, 109, 118], Some(109));
    for hours in [
        (100..=118).collect::<Vec<i32>>(),
        vec![100],
        vec![118],
        vec![95, 100, 113, 118, 130],
        vec![101, 104, 105, 106, 112, 116],
        (90..=140).step_by(3).collect(),
        Vec::new(),
    ] {
        let [read, looked] = published.changes::<Mines>(&hours);
        assert_eq!(read, looked, "hours {hours:?}");
    }
}
