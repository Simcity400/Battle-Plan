use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cunit_leader;
use replay_data::{rows::LeaderPortraitPathRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LeaderPortraitPaths];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });
impl Capacities {
    pub fn presize(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let count = collector_extended_support::demand::rows(
            engine_native::corders_group::count_roots(memory, base),
        )?;
        crate::table_capture::grow_capacity(&mut self.rows, count);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<cunit_leader::PortraitPathRow<engine_native_memory::capture_text::Owned>>,
    tag_memo: cunit_leader::PortraitTagMemo<engine_native_memory::capture_text::Owned>,
}
impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }

    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            tag_memo: Default::default(),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.tag_memo.allocated_bytes()
            + self
                .rows
                .rows
                .iter()
                .map(|row| row.tag.allocated_bytes())
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        let _ = self.tag_memo.grow();
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
        self.rows.outcome = match sources {
            Some(sources) => cunit_leader::walk_portrait_paths_memo(
                memory,
                base,
                state,
                sources,
                &mut self.rows.rows,
                &mut self.tag_memo,
            ),
            None => Outcome::Unavailable {
                reason: "HQ portrait source census unavailable",
            },
        };
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
                tag: owned_vec(copy_vec(r.tag.as_bytes())),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    #[test]
    fn missing_hq_census_is_unavailable_and_tag_bytes_convert_exactly() {
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
        capture.rows.rows[0].tag.bytes.resize(3, 0);
        capture.rows.rows[0].tag.bytes[..3].copy_from_slice(b"G\0R");
        capture.rows.rows[0].tag.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay_data::rows::TableRows::LeaderPortraitPaths(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].tag, b"G\0R");
        assert_eq!(rows[0].tag_reference, 3);
    }
}
