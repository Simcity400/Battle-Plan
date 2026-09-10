//! Shared typed acquisition buffers. Allocation and growth happen on the spool thread.

use crate::sampling::{CONSISTENCY_TARGET, full_match, mismatches};
use engine::walk::Outcome;
use std::mem::size_of;

const UNAVAILABLE: Outcome = Outcome::Unavailable {
    reason: "clock unavailable",
};

pub struct CaptureRows<R> {
    pub rows: Vec<R>,
    pub second: Vec<R>,
    pub outcome: Outcome,
    pub mismatches: usize,
    pub checked: bool,
}

impl<R: Default + Clone + PartialEq> CaptureRows<R> {
    pub fn new(capacity: usize, successful: u16) -> Self {
        Self {
            rows: vec![R::default(); capacity],
            second: comparison_rows(capacity, successful),
            outcome: UNAVAILABLE,
            mismatches: 0,
            checked: false,
        }
    }

    pub fn reset(&mut self) {
        self.outcome = UNAVAILABLE;
        self.mismatches = 0;
        self.checked = false;
    }

    pub fn compare(&mut self, second: Outcome) -> bool {
        self.mismatches = mismatches(self.outcome, &self.rows, second, &self.second);
        full_match(self.outcome, second, self.mismatches)
    }

    pub fn allocated_bytes(&self) -> usize {
        (self.rows.capacity() + self.second.capacity()) * size_of::<R>()
    }

    pub fn grow(&mut self, capacity: &mut usize, successful: u16) {
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
        comparison(&mut self.second, *capacity, successful);
    }
}

/// Pressure adds headroom off-thread, including when one observation jumps well
/// beyond the previous capacity. Idle buffers receive the same logical length.
pub(crate) fn grow_capacity(capacity: &mut usize, required: usize) {
    if required > *capacity {
        *capacity = capacity
            .saturating_mul(2)
            .max(required.checked_next_power_of_two().unwrap_or(required));
    }
}

pub(crate) fn comparison_rows<T: Default + Clone>(capacity: usize, successful: u16) -> Vec<T> {
    if successful >= CONSISTENCY_TARGET {
        Vec::new()
    } else {
        vec![T::default(); capacity]
    }
}

pub(crate) fn grow_vec<T: Default + Clone>(rows: &mut Vec<T>, capacity: usize) {
    if rows.len() < capacity {
        rows.resize(capacity, T::default());
    }
}

pub(crate) fn comparison<T: Default + Clone>(rows: &mut Vec<T>, capacity: usize, successful: u16) {
    if successful >= CONSISTENCY_TARGET {
        *rows = Vec::new();
    } else {
        grow_vec(rows, capacity);
    }
}

pub struct Arena<T> {
    pub first: Vec<T>,
    pub second: Vec<T>,
    pub len: usize,
    pub required: usize,
}

impl<T: Default + Clone + PartialEq> Arena<T> {
    pub fn new(capacity: usize, successful: u16) -> Self {
        Self {
            first: vec![T::default(); capacity],
            second: comparison_rows(capacity, successful),
            len: 0,
            required: 0,
        }
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.required = 0;
    }

    pub fn compare(&mut self, second_len: usize, second_required: usize) -> bool {
        let matched = self.len == second_len
            && self.required == second_required
            && self
                .first
                .get(..self.len)
                .zip(self.second.get(..second_len))
                .is_some_and(|(first, second)| first == second);
        self.required = self.required.max(second_required);
        matched
    }

    pub fn grow(&mut self, capacity: &mut usize, successful: u16) {
        grow_capacity(capacity, self.required);
        if self.len > 0 && self.len >= capacity.saturating_sub(*capacity / 4) {
            grow_capacity(capacity, capacity.saturating_add(1));
        }
        grow_vec(&mut self.first, *capacity);
        comparison(&mut self.second, *capacity, successful);
    }

    pub fn allocated_bytes(&self) -> usize {
        (self.first.capacity() + self.second.capacity()) * size_of::<T>()
    }
}
