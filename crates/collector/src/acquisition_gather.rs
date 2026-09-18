//! Owned paired walks for callback-bounded acquisition jobs.
//!
//! The caller moves preallocated buffers into a request, submits it through the
//! draining acquisition Batch, and restores the returned buffers at the original
//! collection point. A runner owns every mutable output and never borrows a
//! frame. Pass two remains an independent walk while its consistency window is
//! open, including when pass one fails. Publication and window credit stay with
//! the callback after all source leases have been verified.

use engine::walk::Outcome;

pub(crate) struct Request<B> {
    pub base: usize,
    pub game_state: usize,
    pub needed: bool,
    pub buffers: Buffers<B>,
}

pub(crate) struct Buffers<B> {
    pub first: B,
    pub second: B,
}

#[derive(Clone, Copy)]
pub(crate) struct Report {
    pub result: Outcome,
    pub second: Outcome,
    pub mismatches: usize,
}

impl Default for Report {
    fn default() -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "gather acquisition did not complete",
        };
        Self {
            result: unavailable,
            second: unavailable,
            mismatches: 0,
        }
    }
}

impl<B: Default> Buffers<B> {
    pub(crate) fn take(first: &mut B, second: &mut B) -> Self {
        Self {
            first: std::mem::take(first),
            second: std::mem::take(second),
        }
    }
}

impl<B> Buffers<B> {
    pub(crate) fn restore(self, first: &mut B, second: &mut B) {
        *first = self.first;
        *second = self.second;
    }

    /// `B` may include row buffers, exact text bytes, and a state visitor. The
    /// pass flag lets a visitor retain the walk's original first/second routing.
    pub(crate) fn run(
        &mut self,
        needed: bool,
        mut walk: impl FnMut(&mut B, bool) -> Outcome,
        compare: impl FnOnce(Outcome, &B, Outcome, &B) -> usize,
    ) -> Report {
        let result = walk(&mut self.first, false);
        let (second, mismatches) = if needed {
            let second = walk(&mut self.second, true);
            let mismatches = compare(result, &self.first, second, &self.second);
            (second, mismatches)
        } else {
            (
                Outcome::Unavailable {
                    reason: "gather consistency window closed",
                },
                0,
            )
        };
        Report {
            result,
            second,
            mismatches,
        }
    }
}

impl<R: PartialEq> Buffers<Vec<R>> {
    pub(crate) fn rows(
        &mut self,
        needed: bool,
        walk: impl FnMut(&mut Vec<R>, bool) -> Outcome,
    ) -> Report {
        self.run(needed, walk, |result, first, second_result, second| {
            crate::sampling::mismatches(result, first, second_result, second)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{acquisition, sampling::full_match};

    #[test]
    fn gather_returns_preallocated_buffers_with_zero_one_and_three_workers() {
        for workers in [0, 1, 3] {
            for needed in [false, true] {
                let pool =
                    acquisition::Pool::start(workers, 1, |request: &mut Request<Vec<u32>>| {
                        request.buffers.rows(request.needed, |rows, _| {
                            rows[0] = 7;
                            Outcome::Rows { len: 1 }
                        })
                    })
                    .unwrap();
                let mut first = vec![0; 4];
                let mut second = vec![0; 4];
                let pointers = (first.as_ptr(), second.as_ptr());
                let mut request = Some(Request {
                    base: 0,
                    game_state: 0,
                    needed,
                    buffers: Buffers::take(&mut first, &mut second),
                });
                assert!(first.is_empty() && second.is_empty());
                // SAFETY: this test owns all inputs and collects its one job.
                let ticket = unsafe { pool.submit(&mut request) }.unwrap();
                let completion = pool.finish(ticket).unwrap();
                let report = completion.report.unwrap();
                assert_eq!(report.result, Outcome::Rows { len: 1 });
                assert_eq!(
                    full_match(report.result, report.second, report.mismatches),
                    needed
                );
                completion.request.buffers.restore(&mut first, &mut second);
                assert_eq!((first.as_ptr(), second.as_ptr()), pointers);
                assert_eq!(first, [7, 0, 0, 0]);
                assert_eq!(second, [if needed { 7 } else { 0 }, 0, 0, 0]);
                assert_eq!(pool.calibration().uncollected, 0);
            }
        }
    }

    #[test]
    fn gather_repeats_failed_walks_and_keeps_partial_outcomes_uncredited() {
        let mut buffers = Buffers {
            first: vec![0; 2],
            second: vec![0; 2],
        };
        let mut passes = Vec::new();
        let report = buffers.rows(true, |rows, second| {
            passes.push(second);
            rows[0] = 9;
            Outcome::Unavailable {
                reason: "read fault",
            }
        });
        assert_eq!(passes, [false, true]);
        assert!(!full_match(report.result, report.second, report.mismatches));
        let report = buffers.rows(true, |rows, second| {
            rows[0] = u32::from(second);
            Outcome::Rows { len: 1 }
        });
        assert_eq!(report.mismatches, 1);
        assert!(!full_match(report.result, report.second, report.mismatches));
    }

    #[test]
    fn gather_panic_returns_both_buffers_without_retry_or_success_credit() {
        let pool = acquisition::Pool::start(0, 1, |request: &mut Request<Vec<u32>>| {
            request.buffers.rows(request.needed, |rows, second| {
                rows[0] = 11;
                assert!(!second, "synthetic second-pass failure");
                Outcome::Rows { len: 1 }
            })
        })
        .unwrap();
        let mut first = vec![0; 4];
        let mut second = vec![0; 4];
        let pointers = (first.as_ptr(), second.as_ptr());
        let mut request = Some(Request {
            base: 0,
            game_state: 0,
            needed: true,
            buffers: Buffers::take(&mut first, &mut second),
        });
        // SAFETY: this test retains its buffers through collection on failure.
        let ticket = unsafe { pool.submit(&mut request) }.unwrap();
        let completion = pool.finish(ticket).unwrap();
        assert!(matches!(
            completion.report,
            Err(acquisition::Failure::Panicked)
        ));
        completion.request.buffers.restore(&mut first, &mut second);
        assert_eq!((first.as_ptr(), second.as_ptr()), pointers);
        assert_eq!(first[0], 11);
        assert_eq!(second[0], 11);
        assert_eq!(pool.calibration().uncollected, 0);
        assert_eq!(pool.calibration().panicked, 1);
    }
}
