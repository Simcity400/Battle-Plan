
use engine_native_memory::walk::Outcome;
use std::mem::size_of;

const UNAVAILABLE: Outcome = Outcome::Unavailable {
    reason: "clock unavailable",
};

pub fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

pub struct CaptureRows<R> {
    pub rows: Vec<R>,
    pub outcome: Outcome,
    failed_len: usize,
    failed_required: usize,
}

impl<R: Default> CaptureRows<R> {
    pub fn new(capacity: usize) -> Self {
        Self {
            rows: engine_allocation::owned_vec(engine_allocation::default_vec(capacity)),
            outcome: UNAVAILABLE,
            failed_len: 0,
            failed_required: 0,
        }
    }

    pub fn reset(&mut self) {
        self.outcome = UNAVAILABLE;
        self.failed_len = 0;
        self.failed_required = 0;
    }

    pub fn fail(&mut self, outcome: Outcome) {
        debug_assert!(matches!(outcome, Outcome::Unavailable { .. }));
        match self.outcome {
            Outcome::Rows { len } => self.failed_len = self.failed_len.max(len),
            Outcome::Truncated { len, unread_rows } => {
                self.failed_required = self
                    .failed_required
                    .max(len.saturating_add(unread_rows as usize));
            }
            Outcome::Unavailable { .. } => {}
        }
        self.outcome = outcome;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<R>()
    }

    pub fn grow(&mut self, capacity: &mut usize) {
        grow_capacity(capacity, self.failed_len);
        grow_capacity(capacity, self.outcome.len());
        if self.failed_len > 0 && self.failed_len >= capacity.saturating_sub(*capacity / 4) {
            let required = capacity.saturating_add(1);
            grow_capacity(capacity, required);
        }
        grow_capacity(capacity, self.failed_required);
        match self.outcome {
            Outcome::Rows { len } if len > 0 && len >= capacity.saturating_sub(*capacity / 4) => {
                let required = capacity.saturating_add(1);
                grow_capacity(capacity, required);
            }
            Outcome::Truncated { len, unread_rows } => {
                grow_capacity(capacity, len.saturating_add(unread_rows as usize));
            }
            _ => {}
        }
        grow_vec(&mut self.rows, *capacity);
    }
}

pub fn grow_capacity(capacity: &mut usize, required: usize) {
    if required > *capacity {
        *capacity = capacity
            .saturating_mul(2)
            .max(required.checked_next_power_of_two().unwrap_or(required));
    }
}

pub fn grow_vec<T: Default>(rows: &mut Vec<T>, capacity: usize) -> bool {
    if rows.len() < capacity {
        return engine_allocation::resize_default(rows, capacity).is_ok();
    }
    true
}

#[macro_export]
macro_rules! grow_hint {
    ($($rows:expr => $capacity:expr),+ $(,)?) => {{
        (|| {
            $(if $rows.len() < $capacity {
                $rows.try_reserve_exact($capacity - $rows.len())
                    .map_err(|_| "combat sizing hint allocation failed")?;
            })+
            $(if $rows.len() < $capacity {
                $rows.resize_with($capacity, Default::default);
            })+
            Ok::<(), &'static str>(())
        })()
    }};
}
pub use grow_hint;

pub struct Arena<T> {
    pub first: Vec<T>,
    pub len: usize,
    pub required: usize,
}

impl<T: Default> Arena<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            first: engine_allocation::owned_vec(engine_allocation::default_vec(capacity)),
            len: 0,
            required: 0,
        }
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.required = 0;
    }

    pub fn grow(&mut self, capacity: &mut usize) {
        grow_capacity(capacity, self.required.max(self.len));
        if self.len > 0 && self.len >= capacity.saturating_sub(*capacity / 4) {
            grow_capacity(capacity, capacity.saturating_add(1));
        }
        grow_vec(&mut self.first, *capacity);
    }

    pub fn allocated_bytes(&self) -> usize {
        self.first.capacity() * size_of::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_grown_complete_and_failed_rows_raise_all_recycled_storage() {
        for failed in [false, true] {
            let mut rows = CaptureRows::<u32>::new(2);
            grow_vec(&mut rows.rows, 40000);
            rows.outcome = Outcome::Rows { len: 40000 };
            if failed {
                rows.fail(Outcome::Unavailable {
                    reason: "publication failed",
                });
            }
            let mut capacity = 2;
            rows.grow(&mut capacity);
            let mut idle = CaptureRows::<u32>::new(2);
            idle.grow(&mut capacity);
            assert!(capacity >= 40000);
            assert!(idle.rows.len() >= 40000);
        }
    }

    #[test]
    fn sizing_hint_reserve_failure_preserves_lengths_and_retries() {
        let mut rows = vec![0u32; 2];
        let mut text = vec![0u8; 3];
        assert!(grow_hint! { rows => 4, text => usize::MAX }.is_err());
        assert_eq!((rows.len(), text.len()), (2, 3));
        grow_hint! { rows => 4, text => 8 }.unwrap();
        assert_eq!((rows.len(), text.len()), (4, 8));
    }

    #[test]
    fn bounded_preserves_outcomes_and_refuses_invalid_counts_in_order() {
        for outcome in [
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 1 },
            Outcome::Truncated {
                len: 1,
                unread_rows: 3,
            },
            Outcome::Unavailable {
                reason: "guarded copy failed",
            },
        ] {
            assert_eq!(bounded(outcome, 1), outcome);
        }
        assert_eq!(
            bounded(Outcome::Rows { len: 0 }, 0),
            Outcome::Rows { len: 0 }
        );
        for outcome in [
            Outcome::Rows { len: 2 },
            Outcome::Truncated {
                len: 2,
                unread_rows: 3,
            },
            Outcome::Truncated {
                len: 2,
                unread_rows: 0,
            },
        ] {
            assert_eq!(
                bounded(outcome, 1),
                Outcome::Unavailable {
                    reason: "walker row count exceeds allocated buffer",
                }
            );
        }
        assert_eq!(
            bounded(
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 0
                },
                1
            ),
            Outcome::Unavailable {
                reason: "walker marked truncation without missing rows",
            }
        );
    }

    #[test]
    fn failed_publication_retains_growth_demand_until_reset() {
        for observed in [
            Outcome::Rows { len: 6 },
            Outcome::Truncated {
                len: 2,
                unread_rows: 19,
            },
        ] {
            for initial_capacity in [8, 32] {
                let mut actual = CaptureRows::<u8>::new(8);
                let mut expected = CaptureRows::<u8>::new(8);
                actual.outcome = observed;
                expected.outcome = observed;
                let unavailable = Outcome::Unavailable {
                    reason: "accepted job failed",
                };
                actual.fail(unavailable);
                actual.fail(unavailable);
                assert_eq!(actual.outcome, unavailable);
                let mut actual_capacity = initial_capacity;
                let mut expected_capacity = initial_capacity;
                actual.grow(&mut actual_capacity);
                expected.grow(&mut expected_capacity);
                assert_eq!(actual_capacity, expected_capacity);
                assert_eq!(actual.rows.len(), expected.rows.len());
                actual.reset();
                assert_eq!((actual.failed_len, actual.failed_required), (0, 0));
            }
        }
    }
}
