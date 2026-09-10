//! The global dictionary is acquired once per callback with passive witnesses.
//! Only spool-thread growth and conversion allocate.

use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cpdx_lexer, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LexicalTokens];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[("global lexical dictionary", 0, &[0])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    text: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 32768,
            text: 1_048_576,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<cpdx_lexer::Row>,
    text: Arena<u8>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
            text: Arena::new(capacity.text, successful[0]),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.text.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.text.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.rows.grow(&mut capacity.rows, successful[0]);
        self.text.grow(&mut capacity.text, successful[0]);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let start = qpc();
        let first = cpdx_lexer::walk(memory, base, &mut self.rows.rows, &mut self.text.first);
        self.rows.outcome = first.outcome;
        self.text.len = first.text_len;
        self.text.required = first.text_required;
        let mut matched = false;
        if needed[0] {
            self.rows.checked = true;
            let second =
                cpdx_lexer::walk(memory, base, &mut self.rows.second, &mut self.text.second);
            matched = self.rows.compare(second.outcome);
            if !self.text.compare(second.text_len, second.text_required) {
                self.rows.mismatches = self.rows.mismatches.saturating_add(1);
                matched = false;
            }
        }
        ([matched], [qpc().saturating_sub(start).max(0) as u64])
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "lexical dictionary changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::LexicalTokens, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |row| rows::LexicalTokenRow {
                token: row.token,
                literal: self.text.first[row.text_start..row.text_start + row.text_len].to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cpdx_lexer::*, memory::SliceMemory};
    use std::cell::Cell;

    struct MemoryFixture {
        header: [u8; 64],
        names: [u8; 64],
        copies: Cell<usize>,
        change_on_second: bool,
    }

    impl Memory for MemoryFixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == WRITER_FLAG_RVA {
                destination.fill(0);
                return true;
            }
            if address == INITIALIZED_RVA {
                destination.copy_from_slice(&self.header);
                return true;
            }
            let count = self.copies.get();
            if address == 0x1000 && destination.len() == 32 {
                self.copies.set(count + 1);
            }
            let success = SliceMemory {
                base: 0x1000,
                bytes: &self.names,
            }
            .copy(address, destination);
            if success
                && address == 0x1000
                && destination.len() == 1
                && self.change_on_second
                && self.copies.get() > 1
            {
                destination[0] = b'z';
            }
            success
        }
    }

    fn fixture(change_on_second: bool) -> MemoryFixture {
        let mut header = [0; 64];
        header[0] = 1;
        header[48..56].copy_from_slice(&0x1000_u64.to_le_bytes());
        header[56..60].copy_from_slice(&1_i32.to_le_bytes());
        header[60..64].copy_from_slice(&1_i32.to_le_bytes());
        let mut names = [0; 64];
        names[0] = 255;
        names[16..24].copy_from_slice(&1_u64.to_le_bytes());
        names[24..32].copy_from_slice(&15_u64.to_le_bytes());
        MemoryFixture {
            header,
            names,
            copies: Cell::new(0),
            change_on_second,
        }
    }

    #[test]
    fn lexical_pack_preserves_exact_bytes_and_compares_the_arena() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, text: 32 }, &[0]);
        let (matched, _) = captured.capture(&fixture(false), 0, 0, &[true]);
        assert_eq!(matched, [true]);
        let views = captured.tables();
        let TableView::Complete(rows) = &views[0] else {
            panic!("complete dictionary expected")
        };
        let row = &rows.rows::<tables::LexicalTokens>().unwrap()[0];
        assert_eq!(row.literal, [255]);
        assert_eq!(row.token, 0);
        let (matched, _) = captured.capture(&fixture(true), 0, 0, &[true]);
        assert_eq!(matched, [false]);
        assert_eq!(captured.checked_mismatches(), [1]);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn lexical_pool_grows_after_stable_truncation_and_releases_comparison_storage() {
        let mut capacity = Capacities { rows: 1, text: 0 };
        let mut captured = Captured::with_capacities(&capacity, &[0]);
        captured.capture(&fixture(false), 0, 0, &[false]);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(capacity.text >= 1);
        assert!(captured.rows.second.is_empty() && captured.text.second.is_empty());
        captured.capture(&fixture(false), 0, 0, &[false]);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
    }
}
