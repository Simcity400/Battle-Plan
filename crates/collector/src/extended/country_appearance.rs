//! Current country color bits and exact cosmetic tag bytes, sampled hourly.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, ccountry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CountryAppearanceRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CountryAppearance];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 512 }
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::AppearanceRow>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.rows.grow(&mut capacity.rows, successful[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let begin = qpc();
        self.rows.outcome = ccountry::walk_appearance(memory, base, state, &mut self.rows.rows);
        self.rows.checked = needed[0];
        let matched = if needed[0] {
            let second = ccountry::walk_appearance(memory, base, state, &mut self.rows.second);
            self.rows.compare(second)
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "country appearance changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::CountryAppearance, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| CountryAppearanceRow {
                country: r.country,
                red: r.rgba[0],
                green: r.rgba[1],
                blue: r.rgba[2],
                alpha: r.rgba[3],
                cosmetic_tag: r.cosmetic_tag.as_bytes().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_census_is_unavailable_and_raw_color_changes_fail_the_window() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 }, &[0]);
        assert!(!capture.capture(&memory, 0, 0, &[true]).0[0]);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0].country = 17;
        capture.rows.rows[0].rgba = [0x7fc01234, 0x80000000, 0xffffffff, 0x3f800000];
        capture.rows.rows[0].cosmetic_tag.bytes[..3].copy_from_slice(&[65, 0, 255]);
        capture.rows.rows[0].cosmetic_tag.len = 3;
        capture.rows.second[0] = capture.rows.rows[0];
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.checked = true;
        assert!(capture.rows.compare(Outcome::Rows { len: 1 }));
        let [TableView::Complete(replay::rows::TableRows::CountryAppearance(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].red, 0x7fc01234);
        assert_eq!(rows[0].cosmetic_tag, [65, 0, 255]);
        capture.rows.second[0].rgba[0] ^= 1;
        assert!(!capture.rows.compare(Outcome::Rows { len: 1 }));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
