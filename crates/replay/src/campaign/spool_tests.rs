use replay_capture_spool::spool::{RECORD_HEADER_BYTES, Record, SegmentedSpool, encode_record, recover_prefix};
use replay_capture_spool::spool::test_support::{Fault, MemoryBackend, branch, frame, session};
use replay_frame::branches::{BranchReason, BranchStart, Gap, Parentage};

#[test]
fn two_loads_during_disk_outage_preserve_fresh_and_resumed_branch_gaps() {
    use crate::campaign::{Replay, SeekResult};

    for resumed in [false, true] {
        let session = session();
        let start = |id, first_hour, reason| {
            Record::BranchStart(BranchStart {
                id,
                session: 3,
                loaded_hour: Some(first_hour - 1),
                first_hour,
                parent: Parentage::Unknown,
                reason,
            })
        };
        let lost = |branch, hour| {
            Record::Gap(Gap {
                branch,
                first_hour: hour,
                last_hour: hour,
                reason: "spool I/O failure".into(),
            })
        };
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        let mut spool = SegmentedSpool::new(backend);
        let context = [session.clone(), branch()];
        let failed_hour = if resumed {
            spool.start_segment(&context, &frame(10, true)).unwrap();
            let length = shared.borrow().files[&0].len();
            shared.borrow_mut().fault = Some(Fault::WriteAfter(length + RECORD_HEADER_BYTES + 5));
            assert!(spool.append(&Record::Frame(frame(11, false))).is_err());
            11
        } else {
            let length: usize = context
                .iter()
                .map(|record| encode_record(record).unwrap().len())
                .sum();
            shared.borrow_mut().fault = Some(Fault::WriteAfter(length + RECORD_HEADER_BYTES + 5));
            assert!(spool.start_segment(&context, &frame(10, true)).is_err());
            10
        };
        let abandoned = shared.borrow().files[&0].clone();
        let mut context = context.to_vec();
        context.push(lost(7, failed_hour));
        context.push(start(8, 100, BranchReason::LoadSignal));
        let mut second = frame(100, true);
        second.branch = 8;
        shared.borrow_mut().fault = Some(Fault::Create);
        assert!(spool.start_segment(&context, &second).is_err());
        context.push(lost(8, 100));
        context.push(start(9, 200, BranchReason::LoadSignal));
        let mut third = frame(200, true);
        third.branch = 9;
        shared.borrow_mut().fault = None;
        spool.start_segment(&context, &third).unwrap();
        spool.finish_segment().unwrap();

        assert_eq!(spool.next_segment_index(), 3);
        assert_eq!(shared.borrow().files[&0], abandoned);
        assert!(!shared.borrow().files.contains_key(&1));
        let recovered = recover_prefix(&shared.borrow().files[&2]);
        assert!(recovered.damage.is_none());
        context.push(Record::Frame(third.clone()));
        assert_eq!(recovered.records, context);
        let earlier = recover_prefix(&abandoned);
        let replay =
            Replay::from_recovered(recorder_derive::writers(), b"campaign".to_vec(), &[(0, earlier), (2, recovered)]).unwrap();
        assert_eq!(
            replay
                .branches
                .iter()
                .map(|branch| branch.id)
                .collect::<Vec<_>>(),
            [7, 8, 9]
        );
        assert_eq!(replay.gaps.len(), 2);
        for (branch, hour) in [(7, failed_hour), (8, 100)] {
            assert!(
                matches!(replay.seek(branch, hour).unwrap(), SeekResult::Gap { reason, .. }
                    if reason == "spool I/O failure")
            );
        }
        assert!(matches!(replay.seek(9, 200).unwrap(), SeekResult::Frame(_)));
        assert_eq!(replay.index.len(), 1 + usize::from(resumed));
        if resumed {
            assert!(matches!(replay.seek(7, 10).unwrap(), SeekResult::Frame(_)));
        }
    }
}
