use crate::{
    clock::qpc,
    projection::{Projection, project_table},
    table_capture::CaptureRows,
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{corders_group::Root, cunit_leader};
use replay_data::{rows::LeaderPortraitRow, rows::TableId, rows::tables};
use replay_frame::frame::TableInput;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LeaderPortraits];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    sources: usize,
}

collector_primitives::capacity_max!(Capacities { rows, sources });

impl Capacities {
    pub fn presize(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let count = collector_extended_support::demand::rows(
            engine_native::corders_group::count_roots(memory, base),
        )?;
        crate::table_capture::grow_capacity(&mut self.rows, count);
        crate::table_capture::grow_capacity(&mut self.sources, count);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<cunit_leader::PortraitRow<engine_native_memory::capture_text::Owned>>,
    sources: CaptureRows<(u64, usize)>,
}
impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.sources.fail(self.rows.outcome);
    }

    pub fn vacant() -> Self {
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
        self.rows.allocated_bytes()
            + self.sources.allocated_bytes()
            + self
                .rows
                .rows
                .iter()
                .flat_map(|row| &row.strings)
                .map(engine_native_memory::capture_text::Owned::allocated_bytes)
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.sources.grow(&mut capacity.sources);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[Root],
        outcome: Outcome,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.sources.reset();
        if let Outcome::Rows { len } = outcome
            && let Some(roots) = roots.get(..len)
        {
            let acquired = cunit_leader::walk_hq_portraits(
                memory,
                base,
                roots,
                &mut self.sources.rows,
                &mut self.rows.rows,
            );
            self.rows.outcome = acquired.outcome;
            self.sources.outcome = acquired.sources;
        } else {
            let reason = match outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "HQ portrait orders census incomplete",
            };
            self.rows.outcome = Outcome::Unavailable { reason };
            self.sources.outcome = self.rows.outcome;
        }
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn sources(&self) -> Option<&[(u64, usize)]> {
        match (self.rows.outcome, self.sources.outcome) {
            (Outcome::Rows { .. }, Outcome::Rows { len }) => self.sources.rows.get(..len),
            _ => None,
        }
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
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
                portrait_c0: owned_vec(copy_vec(r.strings[0].as_bytes())),
                portrait_e0: owned_vec(copy_vec(r.strings[1].as_bytes())),
                portrait_100: owned_vec(copy_vec(r.strings[2].as_bytes())),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::{
        frame::DecodedFrame, frame::FrameEncoder, frame::Metrics, frame::TableView,
    };

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
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            &[],
            Outcome::Unavailable {
                reason: "orders census failed",
            },
        );
        assert_eq!(capture.sources(), None);
        let vacant = Captured::vacant();
        assert_eq!(vacant.allocated_bytes(), 0);
        assert_eq!(vacant.sources(), None);
    }

    #[test]
    fn incomplete_order_census_is_not_an_empty_portrait_scope() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::vacant();
        capture.capture(
            &memory,
            0,
            &[],
            Outcome::Truncated {
                len: 0,
                unread_rows: 2,
            },
        );
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        assert_eq!(capture.sources(), None);
        capture.capture(&memory, 0, &[], Outcome::Rows { len: 1 });
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        capture.capture(&memory, 0, &[], Outcome::Rows { len: 0 });
        assert_eq!(capture.outcomes()[0], Outcome::Rows { len: 0 });
        assert_eq!(capture.sources(), Some([].as_slice()));
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
            string.bytes.resize(3, 0);
            string.bytes[..3].copy_from_slice(&[0xff, 0, index as u8]);
            string.len = 3;
        }
        let [TableInput::Sampled(sample)] = capture.table_inputs(None) else {
            panic!("fresh portrait sample")
        };
        let mut tables = std::array::from_fn(|_| {
            crate::table_conversion::unavailable("outside portrait fixture")
        });
        tables[TABLES[0].index()] = sample;
        let mut encoder = FrameEncoder::new();
        encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: tables.map(Into::into),
                    metrics: Metrics::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedEqual(_)
        ));
        let original = capture.rows.rows[0].clone();
        for field in 0..4 {
            capture.rows.rows[0] = original.clone();
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
        capture.rows.rows[0] = original.clone();
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
