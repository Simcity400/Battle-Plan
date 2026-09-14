//! Province override bytes. Native row/text storage grows only off-thread.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cprovince, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ProvinceNameRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceNames];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("province name overrides and province census", 0, &[0])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 512,
            bytes: 32 * 1024,
        }
    }
}

pub struct Captured {
    names: CaptureRows<cprovince::NameRow>,
    bytes: Arena<u8>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            names: CaptureRows::new(capacity.rows, successful[0]),
            bytes: Arena::new(capacity.bytes, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.names.reset();
        self.bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.names.allocated_bytes() + self.bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.names.grow(&mut capacity.rows, successful[0]);
        self.bytes.grow(&mut capacity.bytes, successful[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        if !replay::cadence::should_walk(TABLES[0], hour, force_full) {
            self.names.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return ([false], [0]);
        }
        let started = qpc();
        let first = cprovince::walk_names(
            memory,
            base,
            game_state,
            &mut self.names.rows,
            &mut self.bytes.first,
        );
        self.names.outcome = first.outcome;
        self.bytes.len = first.bytes_len;
        self.bytes.required = first.required_bytes;
        let mut matched = false;
        if needed[0] {
            self.names.checked = true;
            let second = cprovince::walk_names(
                memory,
                base,
                game_state,
                &mut self.names.second,
                &mut self.bytes.second,
            );
            let rows_match = self.names.compare(second.outcome);
            let bytes_match = self.bytes.compare(second.bytes_len, second.required_bytes);
            if !bytes_match {
                self.names.mismatches += 1;
            }
            matched = rows_match && bytes_match;
        }
        ([matched], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.names.checked && self.names.mismatches != 0 {
            Outcome::Unavailable {
                reason: "province names changed during double read",
            }
        } else {
            self.names.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.names.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.outcomes()[0];
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return [unavailable("province name arena length invalid")];
        };
        let Some(rows) = self.names.rows.get(..outcome.len()) else {
            return [unavailable("province name row count invalid")];
        };
        let text = |r: &cprovince::NameRow| {
            r.start
                .checked_add(r.len)
                .and_then(|end| bytes.get(r.start..end))
        };
        if rows.iter().any(|r| text(r).is_none()) {
            return [unavailable("province name lies outside captured arena")];
        }
        [table::<tables::ProvinceNames, _>(
            outcome,
            &self.names.rows,
            |r| ProvinceNameRow {
                province: r.province,
                name: text(r).unwrap().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;

    #[test]
    fn off_hours_do_not_read_and_forced_refresh_does_not_claim_empty_success() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities::default(), &[0]);
        assert_eq!(
            capture.capture(&memory, 0, 0, &[true], 0, false),
            ([false], [0])
        );
        assert_eq!(
            capture.outcomes(),
            [Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }]
        );
        assert!(!capture.capture(&memory, 0, 0, &[true], 0, true).0[0]);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }

    #[test]
    fn conversion_preserves_bytes_and_rejects_invalid_arena_ranges() {
        let mut capture = Captured::with_capacities(&Capacities::default(), &[0]);
        capture.names.rows[0] = cprovince::NameRow {
            province: 85,
            start: 0,
            len: 3,
        };
        capture.names.outcome = Outcome::Rows { len: 1 };
        capture.bytes.len = 3;
        capture.bytes.first[..3].copy_from_slice(&[b'A', 0, 255]);
        let [TableView::Complete(rows)] = capture.tables() else {
            panic!("complete");
        };
        assert_eq!(
            rows,
            replay::rows::TableRows::ProvinceNames(vec![ProvinceNameRow {
                province: 85,
                name: vec![b'A', 0, 255],
            }])
        );
        capture.names.rows[0].start = usize::MAX;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
