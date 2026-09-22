//! Movement defines for the derived ETA, refreshed hourly because native
//! LoadGameDefines can reload them without changing the play session.
use crate::{sampling::qpc, spooling::table};
use engine::{Memory, carmy, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{MovementDefinesRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::MovementDefines];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

const UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "movement defines not captured",
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

pub struct Captured {
    defines: carmy::MovementDefines,
    outcome: Outcome,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.outcome = super::failed_capture(TABLES[0], hour, force_full, reason);
    }

    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self {
            defines: carmy::MovementDefines::default(),
            outcome: UNCAPTURED,
        }
    }
    pub fn reset(&mut self) {
        self.outcome = UNCAPTURED;
    }
    pub fn allocated_bytes(&self) -> usize {
        0
    }
    pub fn grow(&mut self, _capacity: &mut Capacities) {}
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !replay::cadence::should_walk(TABLES[0], hour, force_full) {
            self.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return [0];
        }
        let started = qpc();
        let first = carmy::read_movement_defines(memory, base);
        self.outcome = match first {
            Ok(defines) => {
                self.defines = defines;
                Outcome::Rows { len: 1 }
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::MovementDefines, _>(
            self.outcomes()[0],
            std::slice::from_ref(&self.defines),
            |defines| MovementDefinesRow {
                transport_scale: defines.transport_scale,
                invasion_speed: defines.invasion_speed,
                outer_scale: defines.outer_scale,
                river_crossing_two: defines.river_crossing_two,
                river_crossing_three: defines.river_crossing_three,
                embark_distance: defines.embark_distance,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::{CountedMemory, ReadCounts, SliceMemory};
    use replay::rows::TableRows;

    #[test]
    fn defines_are_read_each_hour_and_reload_without_a_session_change() {
        let mut bytes = vec![0_u8; carmy::TRANSPORT_SCALE_RVA + 8];
        for (rva, value) in [
            (carmy::TRANSPORT_SCALE_RVA, 33_333_i64),
            (carmy::INVASION_SPEED_RVA, 123_457),
            (carmy::OUTER_SCALE_RVA, 100_003),
            (carmy::RIVER_CROSSING_TWO_RVA, -1),
            (carmy::RIVER_CROSSING_THREE_RVA, i64::MIN),
            (engine::cunit::EMBARK_MOVEMENT_DISTANCE_RVA, i64::MAX),
        ] {
            bytes[rva..rva + 8].copy_from_slice(&value.to_le_bytes());
        }
        let mut capture = Captured::with_capacities(&Capacities);
        for (hour, force) in (-24..24).map(|hour| (hour, false)).chain([(0, true), (7, true)]) {
            let memory = CountedMemory::new(SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            let reads = crate::read_accounting::Capture::new();
            capture.capture(&memory, 0, 0, hour, force);
            let counts = reads.finish();
            let mut expected = crate::read_accounting::ZERO;
            expected[TABLES[0].index()] = ReadCounts {
                copy_calls: 6,
                spans: 6,
                bytes: 48,
                ..ReadCounts::ZERO
            };
            assert_eq!(counts, expected);
            let [TableView::Complete(TableRows::MovementDefines(rows))] = capture.tables() else {
                panic!("complete defines at hour {hour}");
            };
            assert_eq!(
                (
                    rows[0].transport_scale,
                    rows[0].invasion_speed,
                    rows[0].outer_scale
                ),
                (33_333, 123_457, 100_003)
            );
            assert_eq!(
                (
                    rows[0].river_crossing_two,
                    rows[0].river_crossing_three,
                    rows[0].embark_distance
                ),
                (-1, i64::MIN, i64::MAX)
            );
        }

        // The module and session stay the same while reload changes operands.
        bytes[carmy::OUTER_SCALE_RVA..carmy::OUTER_SCALE_RVA + 8]
            .copy_from_slice(&25_000_i64.to_le_bytes());
        bytes[engine::cunit::EMBARK_MOVEMENT_DISTANCE_RVA
            ..engine::cunit::EMBARK_MOVEMENT_DISTANCE_RVA + 8]
            .copy_from_slice(&123_000_i64.to_le_bytes());
        capture.capture(&SliceMemory { base: 0, bytes: &bytes }, 0, 0, 49, false);
        let [TableView::Complete(TableRows::MovementDefines(rows))] = capture.tables() else {
            panic!("reloaded defines must be freshly captured");
        };
        assert_eq!(rows[0].outer_scale, 25_000);
        assert_eq!(rows[0].embark_distance, 123_000);

        // A later unreadable module must not reuse the previous successful values.
        let reads = crate::read_accounting::Capture::new();
        capture.capture(
            &CountedMemory::new(SliceMemory {
                base: 0,
                bytes: &[],
            }),
            0,
            0,
            50,
            false,
        );
        assert_eq!(
            reads.finish()[TABLES[0].index()],
            ReadCounts {
                copy_calls: 1,
                spans: 1,
                bytes: 8,
                ..ReadCounts::ZERO
            }
        );
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
