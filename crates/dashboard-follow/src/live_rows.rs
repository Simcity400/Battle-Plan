//! A live query's rows at the head ([`crate::Rows::Live`]): its keys never change (a new structure
//! follows a new query), so its rows are built whole once (at the first step, and after "not
//! recorded"), and from then on a step reads only the rows the step touched: a watched key's slot
//! written in one of its columns, inserted or deleted. A reset, a renumber or a status change
//! touches every row. Its answer is whole until its reader holds rows of these keys, then the rows
//! changed since the answer sent last.

use crate::{
    answer::{State, Table},
    evaluate::{Find, number},
    live::NO_ROW,
};
use replay_columnar::store::TableState;

#[derive(Default)]
pub(crate) struct LiveRows {
    /// Its rows as the last step left them; none before the first and after "not recorded".
    rows: Option<Table>,
    /// The reader holds rows of these keys: the last answer sent was rows, whole or changed.
    held: bool,
    /// Rows changed since the last answer sent: a flag per row, and the rows flagged.
    changed: Vec<bool>,
    list: Vec<u32>,
    /// Rows touched since the last evaluation, and whether every row is.
    touched: Vec<u32>,
    whole: bool,
    cells: Vec<(i64, State)>,
}

/// A live query's answer after a step.
pub(crate) enum Answer {
    /// Nothing changed since the answer sent last.
    Same,
    /// Its body: whole rows, or the rows changed.
    Body,
}

impl LiveRows {
    /// The row of `key` (of the ascending `keys`) may have changed this step.
    pub(crate) fn touch(&mut self, keys: &[u128], key: u128) {
        if let Ok(at) = keys.binary_search(&key) {
            self.touched.push(at as u32);
        }
    }

    /// Every row may have changed this step.
    pub(crate) fn touch_all(&mut self) {
        self.whole = true;
    }

    /// Its table answers "not recorded": its rows are gone, and its next rows go whole.
    pub(crate) fn unrecorded(&mut self) {
        self.rows = None;
        self.touched.clear();
        self.whole = false;
        self.list.clear();
        self.changed.clear();
    }

    /// Its next answer is whole: the reader may not hold the rows the last one changed.
    pub(crate) fn resend(&mut self) {
        self.held = false;
    }

    /// The answer of kind `kind` was sent.
    pub(crate) fn sent(&mut self, kind: u8) {
        self.held = kind == crate::answer::ROWS || kind == crate::answer::CHANGED;
        for &row in &self.list {
            self.changed[row as usize] = false;
        }
        self.list.clear();
    }

    /// Reads the rows the steps since the last touched (every row the first time) from `cells` into
    /// its rows, and its answer into `body`: whole until the reader holds its rows, else the rows
    /// changed since the answer sent last. Returns its answer and the cells read.
    pub(crate) fn evaluate(&mut self, keys: &[u128], columns: &[usize], cells: &TableState, find: &dyn Find, scratch: &mut Table, body: &mut Vec<u8>) -> (Answer, usize) {
        let mut read = 0;
        if self.rows.is_none() {
            let mut rows = Table::default();
            rows.clear(columns.len(), false);
            for &key in keys {
                rows.row(key);
                match find.slot(key) {
                    Some(slot) => {
                        read += row_cells(&mut self.cells, columns, cells, Some(slot));
                        self.cells.iter().for_each(|&(value, state)| rows.cell(value, state));
                    }
                    None => rows.absent(NO_ROW),
                }
            }
            self.rows = Some(rows);
            self.held = false;
            self.changed = vec![false; keys.len()];
            self.list.clear();
        } else {
            if self.whole {
                self.touched.clear();
                self.touched.extend(0..keys.len() as u32);
            } else {
                self.touched.sort_unstable();
                self.touched.dedup();
            }
            let rows = self.rows.as_mut().expect("rows built");
            for &row in &self.touched {
                let row = row as usize;
                let slot = find.slot(keys[row]);
                read += row_cells(&mut self.cells, columns, cells, slot);
                if rows.set_row(row, &self.cells, slot.is_none().then_some(NO_ROW)) && !self.changed[row] {
                    self.changed[row] = true;
                    self.list.push(row as u32);
                }
            }
        }
        let rows = self.rows.as_ref().expect("rows built");
        self.touched.clear();
        self.whole = false;
        body.clear();
        if !self.held {
            rows.encode(body);
            return (Answer::Body, read);
        }
        if self.list.is_empty() {
            return (Answer::Same, read);
        }
        self.list.sort_unstable();
        rows.rows_into(&self.list, scratch);
        scratch.encode_changed(body);
        (Answer::Body, read)
    }

    /// What it holds, in bytes.
    pub(crate) fn bytes(&self) -> usize {
        self.rows.as_ref().map_or(0, Table::bytes) + self.changed.capacity() + (self.list.capacity() + self.touched.capacity()) * 4 + self.cells.capacity() * 16
    }
}

/// The cells of the row at `slot` in `columns` into `out` (none when no live row has the key); the
/// cells read.
fn row_cells(out: &mut Vec<(i64, State)>, columns: &[usize], cells: &TableState, slot: Option<u32>) -> usize {
    out.clear();
    match slot {
        Some(slot) => {
            out.extend(columns.iter().map(|&column| number(cells.value(column, slot))));
            columns.len()
        }
        None => {
            out.extend(columns.iter().map(|_| (0, State::NoRow)));
            0
        }
    }
}
