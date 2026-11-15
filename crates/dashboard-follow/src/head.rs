//! Followed values made from something other than the playhead's tables (the recording's command
//! stream, which no table holds): the dashboard builds a [`HeadValue`] for a follow query naming
//! `{"head": <product>, "params": ..}`; the hook asks it for its answer at every step's hour, on the
//! stepper's thread, and sends the answer only when it differs, as every followed answer. It is
//! asked in a gap too: its source is not the tables.

use crate::answer::{self, State, Table};
use replay_data::Hour;
use serde_json::Value;
use std::sync::{Arc, Mutex, PoisonError};

/// A value at the head, made by its owner from its own source. Asked in hour order while playing;
/// `reset` after a seek, when the hour may be anywhere.
pub trait HeadValue: Send {
    /// The cells of each row.
    fn columns(&self) -> usize;
    /// Its rows at `hour` into `rows`, or why it has none there.
    fn answer(&mut self, hour: Hour, reset: bool, rows: &mut HeadRows<'_>) -> Result<(), String>;
}

/// The rows a [`HeadValue`] writes: a row, then its cells.
pub struct HeadRows<'a>(&'a mut Table);

impl HeadRows<'_> {
    /// A row of `key`, named `name` (what the UI knows it by).
    pub fn row(&mut self, key: u128, name: &str) {
        self.0.row(key);
        self.0.name(name);
    }

    pub fn number(&mut self, value: Option<i64>) {
        match value {
            Some(value) => self.0.cell(value, State::Value),
            None => self.0.cell(0, State::Missing),
        }
    }

    /// A float as its bits, which the UI reads back exactly.
    pub fn float(&mut self, value: Option<f64>) {
        self.number(value.map(|value| value.to_bits() as i64));
    }

    pub fn flag(&mut self, value: bool) {
        self.number(Some(i64::from(value)));
    }
}

/// A followed head value: shared by the query's clones, asked only on the hook's thread.
#[derive(Clone)]
pub struct Head {
    value: Arc<Mutex<Box<dyn HeadValue>>>,
}

impl Head {
    pub fn new(value: impl HeadValue + 'static) -> Self {
        Self { value: Arc::new(Mutex::new(Box::new(value))) }
    }
}

/// A head value's follow query built by its owner: none when the query names no head; why not when
/// it names one the owner cannot make.
pub type HeadOf<'a> = dyn FnOnce(&Value) -> Option<Result<Head, String>> + 'a;

/// The table a head reads: none of the playhead's, so it joins no index and declares no column.
pub(crate) const TABLE: usize = usize::MAX;

/// `head`'s answer at `hour` as a body, in `out`; `rows` is scratch.
pub(crate) fn write(head: &Head, hour: Hour, reset: bool, rows: &mut Table, out: &mut Vec<u8>) {
    let mut value = head.value.lock().unwrap_or_else(PoisonError::into_inner);
    rows.clear(value.columns(), false);
    match value.answer(hour, reset, &mut HeadRows(rows)) {
        Ok(()) => rows.encode(out),
        Err(reason) => answer::not_recorded(&reason, out),
    }
}
