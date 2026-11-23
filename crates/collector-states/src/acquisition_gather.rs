use engine_native_memory::walk::Outcome;

pub struct Request<B> {
    pub base: usize,
    pub game_state: usize,
    pub payload: B,
}

#[derive(Clone, Copy)]
pub struct Report {
    pub result: Outcome,
}

impl Default for Report {
    fn default() -> Self {
        Self {
            result: Outcome::Unavailable {
                reason: "gather acquisition did not complete",
            },
        }
    }
}

impl<B: Default> Request<B> {
    pub fn take(base: usize, game_state: usize, payload: &mut B) -> Self {
        Self {
            base,
            game_state,
            payload: std::mem::take(payload),
        }
    }
}

impl<B> Request<B> {
    pub fn restore(self, payload: &mut B) {
        *payload = self.payload;
    }

    pub fn run(&mut self, walk: impl FnOnce(&mut B) -> Outcome) -> Report {
        Report {
            result: walk(&mut self.payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn gather_returns_one_preallocated_payload_with_zero_one_and_three_workers() {
        for workers in [0, 1, 3] {
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = Arc::clone(&calls);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe {
                collector_workers::acquisition::Pool::start(
                    workers,
                    1,
                    move |request: &mut Request<Vec<u32>>| {
                        request.run(|rows| {
                            observed.fetch_add(1, Ordering::Relaxed);
                            rows[0] = 7;
                            Outcome::Rows { len: 1 }
                        })
                    },
                )
            }
            .unwrap();
            let mut rows = vec![0; 4];
            let pointer = rows.as_ptr();
            let mut request = Some(Request::take(0, 0, &mut rows));
            assert!(rows.is_empty());
            let ticket = unsafe { pool.submit(&mut request) }.unwrap();
            let completion = pool.finish(ticket).unwrap();
            assert_eq!(completion.report.unwrap().result, Outcome::Rows { len: 1 });
            completion.request.restore(&mut rows);
            assert_eq!(rows.as_ptr(), pointer);
            assert_eq!(rows, [7, 0, 0, 0]);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }

    #[test]
    fn gather_preserves_a_failed_first_walk_without_retry() {
        let mut request = Request {
            base: 0,
            game_state: 0,
            payload: vec![0; 2],
        };
        let mut calls = 0;
        let report = request.run(|rows| {
            calls += 1;
            rows[0] = 9;
            Outcome::Unavailable {
                reason: "read fault",
            }
        });
        assert_eq!(calls, 1);
        assert_eq!(request.payload, [9, 0]);
        assert_eq!(
            report.result,
            Outcome::Unavailable {
                reason: "read fault"
            }
        );
    }

    #[test]
    fn gather_panic_returns_partial_payload_without_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            collector_workers::acquisition::Pool::start(
                0,
                1,
                move |request: &mut Request<Vec<u32>>| {
                    request.run(|rows| {
                        observed.fetch_add(1, Ordering::Relaxed);
                        rows[0] = 11;
                        panic!("synthetic acquisition failure");
                    })
                },
            )
        }
        .unwrap();
        let mut rows = vec![0; 4];
        let pointer = rows.as_ptr();
        let mut request = Some(Request::take(0, 0, &mut rows));
        let ticket = unsafe { pool.submit(&mut request) }.unwrap();
        let completion = pool.finish(ticket).unwrap();
        assert!(matches!(
            completion.report,
            Err(collector_workers::acquisition::Failure::Panicked)
        ));
        completion.request.restore(&mut rows);
        assert_eq!(rows.as_ptr(), pointer);
        assert_eq!(rows, [11, 0, 0, 0]);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(pool.calibration().uncollected, 0);
        assert_eq!(pool.calibration().panicked, 1);
    }
}
