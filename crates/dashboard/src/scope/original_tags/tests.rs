use crate::test_fixtures::{CAMPAIGN, session};
use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    container::Replay,
    frame::{DecodedFrame, FrameEncoder},
    spool::Record,
};
use std::sync::atomic::AtomicBool;

/// OriginalTagLists::derive reads the branch from its first frame: a load
/// gives slot order (1401DA010), later name_tag changes append by hour.
#[test]
fn original_tag_lists_derive_from_a_loaded_branch() {
    use crate::scope::{OriginalTagLists, RecordedScopes, tests::dynamic_frame};
    let frames = vec![
        dynamic_frame(10, ["GER", "GER", ""], [0, 1, 1, 1]),
        dynamic_frame(11, ["", "GER", ""], [0, 1, 1, 1]),
        dynamic_frame(12, ["", "GER", "GER"], [0, 0, 0, 1]),
    ];
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(10),
            first_hour: 10,
            parent: Parentage::Unknown,
            reason: BranchReason::LoadSignal,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for mut frame in frames.clone() {
        frame.branch = 1;
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let derived = OriginalTagLists::derive(
        &replay,
        1,
        &[10, 12].into_iter().collect(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let resolve = |hour: i32, frame: &DecodedFrame| {
        RecordedScopes::read_with(frame, derived[&hour].clone(), Default::default())
            .resolve_tag("GER")
    };
    // At the load, D01 (slot 1) comes first.
    assert_eq!(resolve(10, &frames[0]), Ok(1));
    // At hour 12 only D03 (appended at hour 12) owns a state.
    assert_eq!(resolve(12, &frames[2]), Ok(3));
    let members: Vec<Vec<u32>> = derived[&12]
        .list("GER")
        .iter()
        .map(|group| group.members.clone())
        .collect();
    assert_eq!(members, [vec![2], vec![3]]);
}
