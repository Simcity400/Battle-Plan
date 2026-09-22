//! Hourly exact native leader portrait strings.
use crate::{
    projection::{Projection, project_table},
    sampling::qpc,
    table_capture::CaptureRows,
};
use engine::{Memory, cunit_leader, walk::Outcome};
use replay::{
    frame::TableInput,
    rows::{LeaderPortraitRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LeaderPortraits];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    sources: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 8192,
            sources: 16384,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<cunit_leader::PortraitRow>,
    // Reused only by the adjacent path walk; storage grows off-thread.
    sources: CaptureRows<(u64, usize)>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows.fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.sources.fail(self.rows.outcome);
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            sources: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            sources: CaptureRows::new(capacity.sources),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.sources.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.sources.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.sources.grow(&mut capacity.sources);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, _state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.sources.reset();
        let acquired =
            cunit_leader::walk_portraits(memory, base, &mut self.sources.rows, &mut self.rows.rows);
        self.rows.outcome = acquired.outcome;
        self.sources.outcome = acquired.sources;
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub(crate) fn sources(&self) -> Option<&[(u64, usize)]> {
        match (self.rows.outcome, self.sources.outcome) {
            (Outcome::Rows { .. }, Outcome::Rows { len }) => self.sources.rows.get(..len),
            _ => None,
        }
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub(crate) fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        [project_table::<tables::LeaderPortraits, _>(
            projection,
            self.outcomes()[0],
            &self.rows.rows,
            |row| u128::from(row.leader),
            |row, previous| {
                row.leader == previous.leader
                    && row.character == previous.character
                    && row.strings[0].as_bytes() == previous.portrait_c0
                    && row.strings[1].as_bytes() == previous.portrait_e0
                    && row.strings[2].as_bytes() == previous.portrait_100
            },
            |r| LeaderPortraitRow {
                leader: r.leader,
                character: r.character,
                portrait_c0: r.strings[0].as_bytes().to_vec(),
                portrait_e0: r.strings[1].as_bytes().to_vec(),
                portrait_100: r.strings[2].as_bytes().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::{DecodedFrame, FrameEncoder, Metrics, TableView};

    #[test]
    fn census_reuse_requires_both_complete_outcomes_and_resets_between_captures() {
        let mut capacity = Capacities {
            rows: 1,
            sources: 1,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.sources.rows[0] = (u64::MAX, 0x6000);
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.sources.outcome = Outcome::Rows { len: 1 };
        assert_eq!(capture.sources(), Some([(u64::MAX, 0x6000)].as_slice()));
        for incomplete in [
            Outcome::Truncated {
                len: 1,
                unread_rows: 2,
            },
            Outcome::Unavailable { reason: "fault" },
        ] {
            capture.rows.outcome = incomplete;
            assert_eq!(capture.sources(), None);
            capture.rows.outcome = Outcome::Rows { len: 1 };
            capture.sources.outcome = incomplete;
            assert_eq!(capture.sources(), None);
            capture.sources.outcome = Outcome::Rows { len: 1 };
        }
        capture.sources.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        };
        capture.grow(&mut capacity);
        assert_eq!(capacity.sources, 4);
        assert_eq!(capture.sources.rows.len(), 4);
        assert_eq!(
            capture.allocated_bytes(),
            capture.rows.allocated_bytes() + 4 * size_of::<(u64, usize)>()
        );
        capture.reset();
        assert_eq!(capture.sources(), None);
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.sources.outcome = Outcome::Rows { len: 1 };
        capture.capture(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
        );
        assert_eq!(capture.sources(), None);
        let vacant = Captured::vacant();
        assert_eq!(vacant.allocated_bytes(), 0);
        assert_eq!(vacant.sources(), None);
    }

    #[test]
    fn projection_compares_character_and_all_three_exact_strings() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            sources: 0,
        });
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0].leader = u64::MAX;
        capture.rows.rows[0].character = 99;
        for (index, string) in capture.rows.rows[0].strings.iter_mut().enumerate() {
            string.bytes[..3].copy_from_slice(&[0xff, 0, index as u8]);
            string.len = 3;
        }
        let [TableInput::Sampled(sample)] = capture.table_inputs(None) else {
            panic!("fresh portrait sample")
        };
        let mut tables =
            std::array::from_fn(|_| crate::spooling::unavailable("outside portrait fixture"));
        tables[TABLES[0].index()] = sample;
        let mut encoder = FrameEncoder::new();
        encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables,
                    metrics: Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedEqual(_)
        ));
        let original = capture.rows.rows[0];
        for field in 0..4 {
            capture.rows.rows[0] = original;
            if field == 3 {
                capture.rows.rows[0].character ^= 1;
            } else {
                capture.rows.rows[0].strings[field].bytes[0] ^= 1;
            }
            let [TableInput::ComparedChanges { indices, rows, .. }] =
                capture.table_inputs(Some(&mut projection))
            else {
                panic!("changed portrait field {field}")
            };
            assert_eq!(indices, [0]);
            let [TableInput::Sampled(expected)] = capture.table_inputs(None) else {
                panic!("fresh portrait sample")
            };
            assert_eq!(TableView::Complete(rows), expected);
        }
        capture.rows.rows[0] = original;
        capture.rows.rows[0].strings[1].len = 0;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedChanges { .. }
        ));
        capture.rows.rows[0] = original;
        capture.rows.rows[0].leader -= 1;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::Sampled(TableView::Complete(_))
        ));
        capture.rows.outcome = Outcome::Unavailable {
            reason: "portrait capture failed",
        };
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }
}
