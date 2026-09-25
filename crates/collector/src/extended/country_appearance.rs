//! Current country colour bits, cosmetic tag and name operands, sampled hourly.
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
impl Capacities {
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::AppearanceRow>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome = match ccountry::dynamic_boundary(memory, base) {
            Some(boundary) => {
                ccountry::walk_appearance(memory, base, state, boundary, &mut self.rows.rows)
            }
            None => Outcome::Unavailable {
                reason: "country tag database unreadable",
            },
        };
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
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
                name_tag: r.name_tag.as_bytes().to_vec(),
                leader_ideology: r.leader_ideology.as_bytes().to_vec(),
                overlord: r.overlord,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_census_is_unavailable_and_raw_colors_preserve_primary_values() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 });
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0].country = 17;
        capture.rows.rows[0].rgba = [0x7fc01234, 0x80000000, 0xffffffff, 0x3f800000];
        capture.rows.rows[0].cosmetic_tag.bytes[..3].copy_from_slice(&[65, 0, 255]);
        capture.rows.rows[0].cosmetic_tag.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::CountryAppearance(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].red, 0x7fc01234);
        assert_eq!(rows[0].cosmetic_tag, [65, 0, 255]);
        assert_eq!(rows[0].green, 0x80000000);
        assert_eq!(rows[0].blue, 0xffffffff);
        assert_eq!(rows[0].alpha, 0x3f800000);
    }
}
