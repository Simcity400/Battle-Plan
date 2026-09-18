//! Owned States census and its callback-scoped StateFlags visitor.
//! Flag bytes are read later, after this request returns at the original census point.

use crate::{acquisition_gather, extended::state_flags, sampling};
use engine::{Memory, cstate, walk::Outcome};

pub(crate) struct Request {
    pub census: acquisition_gather::Request<Vec<cstate::Row>>,
    pub flags_needed: bool,
    pub visitor: state_flags::Captured,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub census: acquisition_gather::Report,
    /// Census, visitor and core comparison cost; StateFlags keeps its later timing.
    /// A failed runner has no report, so its zero ticks mean unmeasured work.
    pub ticks: i64,
}

impl Report {
    pub(crate) fn failed(needed: bool, reason: &'static str) -> Self {
        let unavailable = Outcome::Unavailable { reason };
        Self {
            census: acquisition_gather::Report {
                result: unavailable,
                second: unavailable,
                mismatches: usize::from(needed),
            },
            ticks: 0,
        }
    }
}

impl Request {
    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        let Self {
            census,
            flags_needed,
            visitor,
        } = self;
        let needed = census.needed;
        let begin = sampling::qpc();
        let report = census.buffers.run(
            needed || *flags_needed,
            |rows, second| {
                let outcome =
                    cstate::walk_with(memory, census.base, census.game_state, rows, |source| {
                        visitor.visit(source, second)
                    });
                visitor.finish(outcome, second);
                outcome
            },
            |first, rows, second, other| {
                if needed {
                    sampling::mismatches(first, rows, second, other)
                } else {
                    0
                }
            },
        );
        Report {
            census: report,
            ticks: sampling::qpc() - begin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acquisition::{Pool, SubmitError},
        acquisition_gather::{Buffers, Report as GatherReport},
        acquisition_jobs,
        sampling::{CONSISTENCY_TARGET, full_match},
    };
    use engine::{cgame_state, memory::SliceMemory};
    use std::cell::RefCell;

    const STATE: usize = 0x1000;
    const ROOT: usize = 0x1800;
    const FIRST: usize = 0x2000;
    const LAST: usize = 0x4000;

    fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
        bytes[at..at + N].copy_from_slice(&value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        let root = STATE + cgame_state::STATES;
        put(&mut bytes, root, (ROOT as u64).to_le_bytes());
        put(&mut bytes, root + 8, 5_u32.to_le_bytes());
        put(&mut bytes, root + 12, 4_u32.to_le_bytes());
        // Reserved slot zero, a null hole, and poisoned spare capacity.
        for (slot, pointer) in [
            (0, u64::MAX),
            (1, FIRST as u64),
            (3, LAST as u64),
            (4, u64::MAX),
        ] {
            put(&mut bytes, ROOT + slot * 8, pointer.to_le_bytes());
        }
        for (pointer, row, flag) in [
            (
                FIRST,
                cstate::Row {
                    id: 1,
                    owner: 0,
                    controller: 0xffff_fffe,
                },
                2,
            ),
            (
                LAST,
                cstate::Row {
                    id: 3,
                    owner: 0x8000_0001,
                    controller: 0,
                },
                255,
            ),
        ] {
            put(
                &mut bytes,
                pointer,
                (cstate::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + cstate::ID, row.id.to_le_bytes());
            put(&mut bytes, pointer + cstate::OWNER, row.owner.to_le_bytes());
            put(
                &mut bytes,
                pointer + cstate::CONTROLLER,
                row.controller.to_le_bytes(),
            );
            bytes[pointer + cstate::DEMILITARIZED] = flag;
        }
        bytes
    }

    fn request(needed: [bool; 2], retain_closed: bool) -> Request {
        Request {
            census: acquisition_gather::Request {
                base: 0,
                game_state: STATE,
                needed: needed[0],
                buffers: Buffers {
                    first: vec![cstate::Row::default(); 2],
                    second: if retain_closed || needed[0] || needed[1] {
                        vec![cstate::Row::default(); 2]
                    } else {
                        Vec::new()
                    },
                },
            },
            flags_needed: needed[1],
            visitor: state_flags::Captured::with_capacities(
                &Default::default(),
                &[if retain_closed || needed[1] {
                    0
                } else {
                    CONSISTENCY_TARGET
                }],
            ),
        }
    }

    #[derive(Clone, Copy)]
    enum Action {
        Fail,
        Change,
    }

    struct Traced<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<(usize, usize)>>,
        injection: Option<(usize, Action)>,
    }

    impl<'a> Traced<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                bytes,
                reads: RefCell::new(Vec::new()),
                injection: None,
            }
        }
    }

    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let index = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            let copied = SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output);
            if let Some((at, action)) = self.injection
                && at == index
            {
                match action {
                    Action::Fail => return false,
                    Action::Change => output[0] ^= 1,
                }
            }
            copied
        }
    }

    // Original inline sequence, independently of the paired acquisition helper.
    fn serial(request: &mut Request, memory: &impl Memory) -> GatherReport {
        let census = &mut request.census;
        let result = cstate::walk_with(
            memory,
            census.base,
            census.game_state,
            &mut census.buffers.first,
            |source| request.visitor.visit(source, false),
        );
        request.visitor.finish(result, false);
        let (second, mismatches) = if census.needed || request.flags_needed {
            let second = cstate::walk_with(
                memory,
                census.base,
                census.game_state,
                &mut census.buffers.second,
                |source| request.visitor.visit(source, true),
            );
            request.visitor.finish(second, true);
            let mismatches = if census.needed {
                sampling::mismatches(
                    result,
                    &census.buffers.first,
                    second,
                    &census.buffers.second,
                )
            } else {
                0
            };
            (second, mismatches)
        } else {
            (
                Outcome::Unavailable {
                    reason: "gather consistency window closed",
                },
                0,
            )
        };
        GatherReport {
            result,
            second,
            mismatches,
        }
    }

    fn verdict(report: GatherReport) -> (Outcome, Outcome, usize) {
        (report.result, report.second, report.mismatches)
    }

    #[test]
    fn states_windows_match_serial_reads_and_leave_flags_for_the_later_stage() {
        let bytes = fixture();
        for needed in [[false, false], [true, false], [false, true], [true, true]] {
            for retained in [false, true] {
                let mut expected = request(needed, retained);
                let mut actual = request(needed, retained);
                let serial_memory = Traced::new(&bytes);
                let memory = Traced::new(&bytes);
                let reference = serial(&mut expected, &serial_memory);
                let report = actual.run(&memory);
                assert_eq!(verdict(report.census), verdict(reference));
                assert_eq!(actual.census.buffers.first, expected.census.buffers.first);
                assert_eq!(actual.census.buffers.second, expected.census.buffers.second);
                assert_eq!(*memory.reads.borrow(), *serial_memory.reads.borrow());
                assert_eq!(report.census.result, Outcome::Rows { len: 2 });
                assert_eq!(
                    matches!(report.census.second, Outcome::Rows { len: 2 }),
                    needed[0] || needed[1]
                );
                assert!(memory.reads.borrow().iter().all(|&(at, _)| {
                    at != FIRST + cstate::DEMILITARIZED && at != LAST + cstate::DEMILITARIZED
                }));
                assert!(matches!(
                    actual.visitor.outcomes()[0],
                    Outcome::Unavailable { .. }
                ));
                let flags = actual.visitor.capture(&memory, 0, STATE, &[needed[1]]);
                let expected_flags =
                    expected
                        .visitor
                        .capture(&serial_memory, 0, STATE, &[needed[1]]);
                assert_eq!(flags.0, [needed[1]]);
                assert_eq!(flags.0, expected_flags.0);
                assert_eq!(actual.visitor.tables(), expected.visitor.tables());
                assert_eq!(*memory.reads.borrow(), *serial_memory.reads.borrow());
                let [replay::frame::TableView::Complete(replay::rows::TableRows::StateFlags(rows))] =
                    actual.visitor.tables()
                else {
                    panic!("complete flags from sparse state sources");
                };
                assert_eq!(
                    rows,
                    [
                        replay::rows::StateFlagRow {
                            state: 1,
                            demilitarized: 2
                        },
                        replay::rows::StateFlagRow {
                            state: 3,
                            demilitarized: 255
                        },
                    ]
                );
            }
        }
    }

    #[test]
    fn states_read_faults_keep_the_original_sequence_and_independent_second_census() {
        let bytes = fixture();
        let observed = Traced::new(&bytes);
        request([false, false], false).run(&observed);
        let first_pass_reads = observed.reads.borrow().len();
        for fail_at in 0..first_pass_reads {
            let mut memory = Traced::new(&bytes);
            memory.injection = Some((fail_at, Action::Fail));
            let mut serial_memory = Traced::new(&bytes);
            serial_memory.injection = memory.injection;
            let mut actual = request([true, true], false);
            let mut expected = request([true, true], false);
            let report = actual.run(&memory);
            let reference = serial(&mut expected, &serial_memory);
            assert_eq!(
                verdict(report.census),
                verdict(reference),
                "fault {fail_at}"
            );
            assert!(matches!(report.census.result, Outcome::Unavailable { .. }));
            assert_eq!(report.census.second, Outcome::Rows { len: 2 });
            assert_eq!(report.census.mismatches, 1);
            assert_eq!(*memory.reads.borrow(), *serial_memory.reads.borrow());
            assert_eq!(actual.census.buffers.first, expected.census.buffers.first);
            assert_eq!(actual.census.buffers.second, expected.census.buffers.second);
            assert_eq!(
                actual.visitor.capture(&memory, 0, STATE, &[true]).0,
                [false]
            );
            expected.visitor.capture(&serial_memory, 0, STATE, &[true]);
            assert_eq!(actual.visitor.tables(), expected.visitor.tables());
            assert_eq!(*memory.reads.borrow(), *serial_memory.reads.borrow());
        }
    }

    #[test]
    fn states_core_comparison_is_independent_of_the_flag_window_and_bytes() {
        let bytes = fixture();
        let baseline = Traced::new(&bytes);
        request([true, true], false).run(&baseline);
        let second_owner = baseline
            .reads
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (at, _))| *at == FIRST + cstate::OWNER)
            .nth(1)
            .unwrap()
            .0;
        for needed in [[true, false], [false, true], [true, true]] {
            let mut memory = Traced::new(&bytes);
            memory.injection = Some((second_owner, Action::Change));
            let mut actual = request(needed, false);
            let report = actual.run(&memory);
            assert_eq!(report.census.result, Outcome::Rows { len: 2 });
            assert_eq!(report.census.second, report.census.result);
            assert_ne!(
                actual.census.buffers.first[0].owner,
                actual.census.buffers.second[0].owner
            );
            assert_eq!(report.census.mismatches, usize::from(needed[0]));
            // Owner changes do not poison independently authenticated flag bytes.
            assert_eq!(
                actual.visitor.capture(&memory, 0, STATE, &[needed[1]]).0,
                [needed[1]]
            );
            assert_eq!(actual.visitor.outcomes(), [Outcome::Rows { len: 2 }]);
        }
        let mut actual = request([true, true], false);
        let census = actual
            .run(&SliceMemory {
                base: 0,
                bytes: &bytes,
            })
            .census;
        assert!(full_match(census.result, census.second, census.mismatches));
        let flag_baseline = Traced::new(&bytes);
        let mut reference = request([true, true], false);
        reference.run(&SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        reference.visitor.capture(&flag_baseline, 0, STATE, &[true]);
        let second_flag = flag_baseline
            .reads
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (at, _))| *at == FIRST + cstate::DEMILITARIZED)
            .nth(1)
            .unwrap()
            .0;
        let mut changed = Traced::new(&bytes);
        changed.injection = Some((second_flag, Action::Change));
        assert_eq!(
            actual.visitor.capture(&changed, 0, STATE, &[true]).0,
            [false]
        );
        assert_eq!(actual.visitor.checked_mismatches(), [1]);
        assert!(full_match(census.result, census.second, census.mismatches));
    }

    #[test]
    fn states_root_and_later_source_identity_changes_remain_unavailable() {
        let bytes = fixture();
        let observed = Traced::new(&bytes);
        request([false, false], false).run(&observed);
        let final_root = observed
            .reads
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (at, _))| *at == STATE + cgame_state::STATES)
            .nth(1)
            .unwrap()
            .0;
        let mut memory = Traced::new(&bytes);
        memory.injection = Some((final_root, Action::Change));
        let mut actual = request([true, true], false);
        let report = actual.run(&memory);
        assert_eq!(
            report.census.result,
            Outcome::Unavailable {
                reason: "state array changed during census"
            }
        );
        assert_eq!(report.census.second, Outcome::Rows { len: 2 });
        assert_eq!(
            actual.visitor.capture(&memory, 0, STATE, &[true]).0,
            [false]
        );

        for at in [ROOT + 8, FIRST, FIRST + cstate::ID] {
            let mut actual = request([true, true], false);
            let report = actual.run(&SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            let mut changed = bytes.clone();
            changed[at] ^= 1;
            assert_eq!(
                actual
                    .visitor
                    .capture(
                        &SliceMemory {
                            base: 0,
                            bytes: &changed
                        },
                        0,
                        STATE,
                        &[true]
                    )
                    .0,
                [false]
            );
            assert_eq!(
                actual.visitor.outcomes(),
                [Outcome::Unavailable {
                    reason: "state source identity changed"
                }]
            );
            assert!(full_match(
                report.census.result,
                report.census.second,
                report.census.mismatches
            ));
        }
    }

    #[test]
    fn states_pool_returns_owned_census_and_visitor_with_zero_one_and_three_workers() {
        for workers in [0, 1, 3] {
            for needed in [[false, false], [true, false], [false, true], [true, true]] {
                let bytes = fixture();
                let pool = Pool::start(workers, 1, move |request: &mut Request| {
                    request.run(&SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    })
                })
                .unwrap();
                let owned = request(needed, true);
                let pointers = (
                    owned.census.buffers.first.as_ptr(),
                    owned.census.buffers.second.as_ptr(),
                );
                let visitor_bytes = owned.visitor.allocated_bytes();
                let mut pending = Some(owned);
                // SAFETY: synthetic memory is owned by the runner until collection.
                let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
                assert!(pending.is_none());
                let mut completed = pool.finish(ticket).unwrap();
                let report = completed.report.unwrap();
                assert_eq!(report.census.result, Outcome::Rows { len: 2 });
                assert_eq!(
                    (
                        completed.request.census.buffers.first.as_ptr(),
                        completed.request.census.buffers.second.as_ptr()
                    ),
                    pointers
                );
                assert_eq!(completed.request.visitor.allocated_bytes(), visitor_bytes);
                let bytes = fixture();
                assert_eq!(
                    completed
                        .request
                        .visitor
                        .capture(
                            &SliceMemory {
                                base: 0,
                                bytes: &bytes
                            },
                            0,
                            STATE,
                            &[needed[1]]
                        )
                        .0,
                    [needed[1]]
                );
                assert_eq!(pool.calibration().uncollected, 0);
            }
        }
    }

    #[test]
    fn states_rejected_request_runs_once_inline_through_the_typed_job_route() {
        let pool = acquisition_jobs::Pool::start(0, 1).unwrap();
        let make = || {
            let mut request = request([false, true], true);
            // Overflow before any LiveMemory access, including the second pass.
            request.census.game_state = usize::MAX;
            Some(acquisition_jobs::Job::States(request))
        };
        let mut accepted = make();
        let mut rejected = make();
        let mut batch = acquisition_jobs::Batch::new(&pool);
        // SAFETY: the overflowing roots cannot access foreign memory.
        let ticket = unsafe { batch.submit(&mut accepted) }.unwrap();
        assert_eq!(
            unsafe { batch.submit(&mut rejected) },
            Err(SubmitError::Full)
        );
        assert!(accepted.is_none() && rejected.is_some());
        let accepted = batch.finish(ticket).unwrap();
        let inline = unsafe { batch.run_inline(rejected.take().unwrap()) };
        for completion in [accepted, inline] {
            let acquisition_jobs::Job::States(mut request) = completion.request else {
                panic!("States request route");
            };
            let acquisition_jobs::Report::States(report) = completion.report.unwrap() else {
                panic!("States report route");
            };
            let failure = Outcome::Unavailable {
                reason: "state array unreadable or invalid",
            };
            assert_eq!(report.census.result, failure);
            assert_eq!(report.census.second, failure);
            assert_eq!(report.census.mismatches, 0);
            assert_eq!(
                request
                    .visitor
                    .capture(
                        &SliceMemory {
                            base: 0,
                            bytes: &[]
                        },
                        0,
                        STATE,
                        &[true]
                    )
                    .0,
                [false]
            );
        }
        let calibration = pool.calibration();
        let counters = calibration.roles[acquisition_jobs::Role::States as usize].counters;
        assert_eq!(
            (
                counters.accepted,
                counters.completed,
                counters.rejected,
                counters.inline_completed
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(calibration.executor.uncollected, 0);
    }
}
