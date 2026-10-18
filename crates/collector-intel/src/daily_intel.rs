//! CountryIntel capture storage shared by the frame pool.
//!
//! CountryIntel is walked once a day (and on forced refreshes), so the pool
//! needs one buffer rather than one per frame. A frame borrows it for the walk
//! and holds it until the spool has converted and grown it; dropping the loan
//! also returns it. A walk that finds it on loan is skipped rather than waited
//! for, the hour carries the last sample, and the next hour walks instead.

use crate::table_capture::{CaptureRows, grow_capacity};
use engine_allocation::{filled, oom};
use engine_native::country_intel_row::Row;
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicUsize, Ordering},
};

pub struct Buffer {
    pub rows: CaptureRows<Row>,
    /// Fixed copy window for the bulk record reads; it never grows.
    pub scratch: Vec<u8>,
    /// Optional reserved suffix pressure cannot change the positive outcome.
    pub reserved_required: usize,
}

impl Buffer {
    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.scratch.capacity()
    }
}

struct Slot {
    buffer: Option<Box<Buffer>>,
    /// A due walk found the buffer on loan; the next walk runs off-cadence.
    missed: bool,
}

struct Inner {
    slot: Mutex<Slot>,
    bytes: AtomicUsize,
}

/// The pool's one buffer. Clones share it.
#[derive(Clone)]
pub struct Home(Arc<Inner>);

impl Home {
    pub fn try_new(rows: usize, scratch: usize) -> std::io::Result<Self> {
        let rows = CaptureRows::new(rows);
        if crate::allocation::failed() {
            return Err(oom());
        }
        let buffer = Box::new(Buffer {
            rows,
            scratch: filled(scratch, 0)?,
            reserved_required: 0,
        });
        Ok(Self(Arc::new(Inner {
            bytes: AtomicUsize::new(buffer.allocated_bytes()),
            slot: Mutex::new(Slot {
                buffer: Some(buffer),
                missed: false,
            }),
        })))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new(rows: usize, scratch: usize) -> Self {
        Self::try_new(rows, scratch).unwrap()
    }

    /// The shared buffer's bytes, to be counted once for the whole pool.
    pub fn allocated_bytes(&self) -> usize {
        self.0.bytes.load(Ordering::Relaxed)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn shares(&self, other: &Home) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Held only for a pointer swap, never across a walk or an allocation.
    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.0.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub enum Walk<'a> {
    /// Not due this hour.
    Skip,
    /// Due, but another frame still holds the buffer.
    Busy,
    Ready(&'a mut Buffer),
}

/// One frame's access to the shared buffer.
#[derive(Default)]
pub struct Loan {
    home: Option<Home>,
    buffer: Option<Box<Buffer>>,
}

impl Loan {
    pub fn new(home: Home) -> Self {
        Self {
            home: Some(home),
            buffer: None,
        }
    }

    pub fn home(&self) -> Option<&Home> {
        self.home.as_ref()
    }

    /// Borrow the buffer for a walk that cadence makes `due`, or that an
    /// earlier hour missed. Never waits for another frame.
    pub fn borrow(&mut self, due: bool) -> Walk<'_> {
        if self.buffer.is_none() {
            let Some(home) = &self.home else {
                return if due { Walk::Busy } else { Walk::Skip };
            };
            let mut slot = home.slot();
            if !due && !slot.missed {
                return Walk::Skip;
            }
            let Some(buffer) = slot.buffer.take() else {
                slot.missed = true;
                return Walk::Busy;
            };
            slot.missed = false;
            drop(slot);
            self.buffer = Some(buffer);
        }
        Walk::Ready(self.buffer.as_deref_mut().expect("borrowed above"))
    }

    /// The borrowed rows, for conversion.
    pub fn rows(&self) -> Option<&[Row]> {
        self.buffer
            .as_deref()
            .map(|buffer| buffer.rows.rows.as_slice())
    }

    /// Spool thread, after conversion: grow the buffer from this frame's walk,
    /// or an idle one to a raised capacity, and return it.
    pub fn grow(&mut self, capacity: &mut usize) {
        let Some(home) = &self.home else {
            return;
        };
        let mut buffer = match self.buffer.take() {
            Some(buffer) => buffer,
            None => {
                let mut slot = home.slot();
                match slot.buffer.take() {
                    Some(buffer)
                        if buffer.rows.rows.len() < *capacity
                            || buffer.reserved_required > *capacity =>
                    {
                        buffer
                    }
                    idle => {
                        slot.buffer = idle;
                        return;
                    }
                }
            }
        };
        grow_capacity(capacity, buffer.reserved_required);
        buffer.rows.grow(capacity);
        // Pressure is consumed; an idle buffer must not grow from it again.
        buffer.rows.reset();
        buffer.reserved_required = 0;
        home.0
            .bytes
            .store(buffer.allocated_bytes(), Ordering::Relaxed);
        home.slot().buffer = Some(buffer);
    }

    /// Return the buffer without growing it.
    pub fn release(&mut self) {
        if let (Some(home), Some(buffer)) = (&self.home, self.buffer.take()) {
            home.slot().buffer = Some(buffer);
        }
    }
}

impl Drop for Loan {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::walk::Outcome;

    #[test]
    fn a_due_walk_never_waits_for_a_held_buffer_and_the_next_hour_walks_instead() {
        let home = Home::new(4, 32);
        let (mut first, mut second) = (Loan::new(home.clone()), Loan::new(home.clone()));
        assert!(matches!(first.borrow(true), Walk::Ready(_)));
        // The spool has not converted the first frame yet: the second frame's
        // due walk returns at once instead of blocking the capture.
        assert!(matches!(second.borrow(true), Walk::Busy));
        // Off-cadence hours skip until the buffer is back, then walk once.
        assert!(matches!(second.borrow(false), Walk::Busy));
        first.grow(&mut 4);
        assert!(matches!(second.borrow(false), Walk::Ready(_)));
        second.release();
        assert!(matches!(second.borrow(false), Walk::Skip));
    }

    #[test]
    fn growth_follows_the_walk_and_raised_capacity_and_a_dropped_loan_returns_the_buffer() {
        let home = Home::new(4, 32);
        let mut loan = Loan::new(home.clone());
        let Walk::Ready(buffer) = loan.borrow(true) else {
            panic!("buffer is free")
        };
        buffer.rows.outcome = Outcome::Truncated {
            len: 4,
            unread_rows: 6,
        };
        let mut capacity = 4;
        loan.grow(&mut capacity);
        assert!(capacity >= 10);
        assert_eq!(home.allocated_bytes(), capacity * size_of::<Row>() + 32);
        // An idle buffer grows only to a raised capacity, never from stale pressure.
        loan.grow(&mut capacity);
        assert_eq!(home.allocated_bytes(), capacity * size_of::<Row>() + 32);
        let mut raised = capacity * 2;
        loan.grow(&mut raised);
        assert_eq!(home.allocated_bytes(), raised * size_of::<Row>() + 32);
        {
            let mut held = Loan::new(home.clone());
            assert!(matches!(held.borrow(true), Walk::Ready(_)));
        }
        assert!(matches!(loan.borrow(true), Walk::Ready(_)));
    }

    #[test]
    fn optional_suffix_pressure_grows_without_poisoning_complete_positive_rows() {
        let home = Home::new(4, 32);
        let mut loan = Loan::new(home.clone());
        let Walk::Ready(buffer) = loan.borrow(true) else {
            panic!("free buffer")
        };
        buffer.rows.outcome = Outcome::Rows { len: 2 };
        buffer.reserved_required = 13;
        let mut capacity = 4;
        loan.grow(&mut capacity);
        assert!(capacity >= 13);
        let grown = capacity;
        loan.grow(&mut capacity);
        assert_eq!(capacity, grown);
        let Walk::Ready(buffer) = loan.borrow(true) else {
            panic!("returned buffer")
        };
        assert_eq!(buffer.reserved_required, 0);
        assert_eq!(buffer.rows.rows.len(), capacity);
        assert_eq!(buffer.scratch.len(), 32);
    }

    #[test]
    fn released_optional_suffix_pressure_is_consumed_by_idle_growth_once() {
        let home = Home::new(4, 32);
        let mut loan = Loan::new(home);
        let Walk::Ready(buffer) = loan.borrow(true) else {
            panic!("free buffer")
        };
        buffer.rows.outcome = Outcome::Rows { len: 2 };
        buffer.reserved_required = 11;
        loan.release();
        let mut capacity = 4;
        loan.grow(&mut capacity);
        assert!(capacity >= 11);
        let grown = capacity;
        loan.grow(&mut capacity);
        assert_eq!(capacity, grown);
    }
}
