use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cunit_leader, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{LeaderPortraitPathRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LeaderPortraitPaths];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 8192 }
    }
}

pub struct Captured {
    rows: CaptureRows<cunit_leader::PortraitPathRow>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows.fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }

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
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Option<&[(u64, usize)]>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome =
            cunit_leader::walk_portrait_paths(memory, base, state, sources, &mut self.rows.rows);
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::LeaderPortraitPaths, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| LeaderPortraitPathRow {
                leader: r.leader,
                country: r.country,
                tag_reference: r.tag_reference,
                tag: r.tag.as_bytes().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn missing_registry_is_unavailable_and_tag_bytes_convert_exactly() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 });
        capture.capture(&memory, 0, 0, None);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0].leader = (9 << 32) | 4713;
        capture.rows.rows[0].country = 12;
        capture.rows.rows[0].tag_reference = 3;
        capture.rows.rows[0].tag.bytes[..3].copy_from_slice(b"G\0R");
        capture.rows.rows[0].tag.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::LeaderPortraitPaths(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].tag, b"G\0R");
        assert_eq!(rows[0].tag_reference, 3);
    }
}
