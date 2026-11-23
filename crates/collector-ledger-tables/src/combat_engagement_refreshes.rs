use crate::{
    clock::qpc,
    table_capture::{CaptureRows, grow_vec},
    table_conversion::unavailable,
};
use collector_observers::combat_engagement_refreshes::{self as producer, Entry};
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use replay_data::rows::{CombatEngagementRefreshRow, RowCodec, TableId, TableRows};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CombatEngagementRefreshes];
pub type Capacities = usize;

pub struct Captured {
    rows: CaptureRows<Entry>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*c),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        *c = (*c).max(producer::capacity());
        grow_vec(&mut self.rows.rows, *c);
    }

    pub fn fail(&mut self, reason: &'static str) {
        producer::lost_drain();
        self.rows.fail(Outcome::Unavailable { reason });
    }

    pub fn capture(&mut self, _memory: &impl Memory, _base: usize, _state: usize) -> [u64; N] {
        self.reset();
        let started = qpc();
        self.rows.outcome = match producer::drain(&mut self.rows.rows) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let Outcome::Rows { len } = self.rows.outcome else {
            return [unavailable(match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "combat engagement refresh incomplete drain",
            })];
        };
        match project(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::CombatEngagementRefreshes(
                rows,
            ))],
            Err(reason) => {
                producer::lost_output(len);
                [unavailable(reason)]
            }
        }
    }
}

fn project(entries: &[Entry]) -> Result<Vec<CombatEngagementRefreshRow>, &'static str> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows: Vec<CombatEngagementRefreshRow> =
        allocation::vec(entries.len()).map_err(|_| "collector allocation failed")?;
    for entry in entries {
        let valid = match entry.kind {
            CombatEngagementRefreshRow::LAND_SIDE | CombatEngagementRefreshRow::NAVAL_SIDE => {
                entry.target != 0
                    && entry.side <= 1
                    && entry.viewer <= i32::MAX as u32
                    && entry.triggers != 0
                    && (entry.triggers & !3) == 0
            }
            CombatEngagementRefreshRow::CONTROLLER_TARGET => {
                entry.side == 0
                    && entry.viewer == 0
                    && entry.target >= 1
                    && entry.target <= i32::MAX as u64
                    && entry.triggers == CombatEngagementRefreshRow::TRIGGER_RECORDED
            }
            CombatEngagementRefreshRow::LOCAL_ALL_SIDES => {
                entry.side == 0
                    && entry.viewer == 0
                    && entry.target <= i32::MAX as u64
                    && entry.triggers == CombatEngagementRefreshRow::TRIGGER_RECORDED
            }
            _ => false,
        };
        if !valid {
            return Err("combat engagement refresh entry invalid");
        }
        rows.push(CombatEngagementRefreshRow {
            kind: entry.kind,
            target: entry.target,
            side: entry.side,
            viewer: entry.viewer,
            triggers: entry.triggers,
        });
    }
    rows.sort_unstable_by_key(RowCodec::key);
    let mut write = 0;
    for read in 1..rows.len() {
        if (
            rows[write].kind,
            rows[write].target,
            rows[write].side,
            rows[write].viewer,
        ) == (
            rows[read].kind,
            rows[read].target,
            rows[read].side,
            rows[read].viewer,
        ) {
            rows[write].triggers |= rows[read].triggers;
        } else {
            write += 1;
            rows[write] = rows[read].clone();
        }
    }
    rows.truncate(write + 1);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_sorts_deduplicates_and_merges_trigger_bits() {
        let entries = [
            Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 62,
                side: 1,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 62,
                side: 1,
                viewer: 4,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 62,
                side: 1,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 10,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
        ];
        let rows = project(&entries).unwrap();
        assert_eq!(
            rows,
            vec![
                CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::LAND_SIDE,
                    target: 10,
                    side: 0,
                    viewer: 0,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                },
                CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::LAND_SIDE,
                    target: 62,
                    side: 1,
                    viewer: 3,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                        | CombatEngagementRefreshRow::TRIGGER_UNITS,
                },
                CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::LAND_SIDE,
                    target: 62,
                    side: 1,
                    viewer: 4,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                },
                CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                    target: 7,
                    side: 0,
                    viewer: 0,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                },
                CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                    target: 0,
                    side: 0,
                    viewer: 0,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                },
            ]
        );
        assert!(project(&[]).unwrap().is_empty());
        assert!(
            project(&[Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 1,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }])
            .is_err()
        );
        assert!(
            project(&[Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 62,
                side: 0,
                viewer: 0,
                triggers: 0,
            }])
            .is_err()
        );
        for bad in [
            Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::NAVAL_SIDE,
                target: 0,
                side: 1,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 62,
                side: 0,
                viewer: i32::MAX as u32 + 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: i32::MAX as u64 + 1,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
            Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: i32::MAX as u64 + 1,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 0,
                viewer: 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
        ] {
            assert!(project(&[bad]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn skipped_drain_via_fail_does_not_replay_stale_entries_into_next_hour() {
        use engine_native::memory::SliceMemory;
        let clock = |hour: i32| engine_native::cgame_state::Clock {
            hour,
            campaign_id: 1,
            play_session_started_at: 10,
            ..Default::default()
        };
        let memory = SliceMemory { base: 0, bytes: &[] };
        producer::test_reset(true);
        producer::presize(16);
        let mut captured = Captured::with_capacities(&16);

        producer::boundary(Some(clock(100)));
        captured.capture(&memory, 0, 0);
        assert!(matches!(captured.outcomes()[0], Outcome::Unavailable { .. }));

        producer::test_record_entry(Entry {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: 62,
            side: 0,
            viewer: 1,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        });
        producer::boundary(Some(clock(101)));
        captured.fail("rejected");
        assert!(matches!(captured.outcomes()[0], Outcome::Unavailable { .. }));

        producer::boundary(Some(clock(102)));
        captured.capture(&memory, 0, 0);
        assert!(matches!(captured.outcomes()[0], Outcome::Unavailable { .. }));

        producer::boundary(Some(clock(103)));
        captured.capture(&memory, 0, 0);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 0 });
        producer::test_reset(false);
    }
}
