//! Pooled air state. Acquisition never allocates; conversion and growth are off-thread.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cace, cair_base, cair_region_combat_data, cair_wing, cloop_history, equipment_entry,
    sair_wing_combat_data, sdamage_entry, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::WingEquipment,
    TableId::AirBases,
    TableId::WingCumulative,
    TableId::AirRegionHistory,
    TableId::AirRegionDamage,
    TableId::AirAces,
    TableId::AirStatistics,
];

/// The one regional source walk is charged to history; damage has only its
/// independent comparison cost. Wing-extension times exclude the enclosing map
/// census, which the caller charges to AirWings after subtracting nested ticks.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("regional air history and damage", 3, &[3, 4])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    /// History/damage capacities count native entries, before off-thread grouping.
    pub rows: [usize; N],
    pub ace_names: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [8192, 2048, 4096, 8192, 16_384, 1024, 2048],
            ace_names: 65_536,
        }
    }
}

/// The exact invocation context from the shared authenticated AirWings visitor.
/// It is never retained. The pool pointer is unnecessary for these two fields.
pub struct WingSource<'a> {
    pub pointer: usize,
    pub row: &'a cair_wing::Row,
    pub second: bool,
}

pub struct Captured {
    equipment: CaptureRows<rows::WingEquipmentRow>,
    bases: CaptureRows<cair_base::Row>,
    cumulative: CaptureRows<cair_wing::CumulativeRow>,
    history: CaptureRows<sair_wing_combat_data::Row>,
    damage: CaptureRows<sdamage_entry::Row>,
    aces: CaptureRows<cace::Row>,
    statistics: CaptureRows<cloop_history::Row>,
    names: Arena<u8>,
    wing_second: [Outcome; 2],
    wing_matched: [bool; 2],
    wing_ticks: [u64; 2],
}

const NO_WING_SOURCE: Outcome = Outcome::Unavailable {
    reason: "shared air wing census not supplied",
};

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            equipment: CaptureRows::new(capacity.rows[0], successful[0]),
            bases: CaptureRows::new(capacity.rows[1], successful[1]),
            cumulative: CaptureRows::new(capacity.rows[2], successful[2]),
            history: CaptureRows::new(capacity.rows[3], successful[3]),
            damage: CaptureRows::new(capacity.rows[4], successful[4]),
            aces: CaptureRows::new(capacity.rows[5], successful[5]),
            statistics: CaptureRows::new(capacity.rows[6], successful[6]),
            names: Arena::new(capacity.ace_names, successful[5]),
            wing_second: [NO_WING_SOURCE; 2],
            wing_matched: [false; 2],
            wing_ticks: [0; 2],
        }
    }

    /// Reset before the shared map walk. `capture` deliberately preserves the
    /// wing extension state prepared by begin/visit/finish_wings.
    pub fn reset(&mut self) {
        self.equipment.reset();
        self.equipment.outcome = NO_WING_SOURCE;
        self.bases.reset();
        self.cumulative.reset();
        self.cumulative.outcome = NO_WING_SOURCE;
        self.history.reset();
        self.damage.reset();
        self.aces.reset();
        self.statistics.reset();
        self.names.reset();
        self.wing_second = [NO_WING_SOURCE; 2];
        self.wing_matched = [false; 2];
        self.wing_ticks = [0; 2];
    }

    pub fn allocated_bytes(&self) -> usize {
        self.equipment.allocated_bytes()
            + self.bases.allocated_bytes()
            + self.cumulative.allocated_bytes()
            + self.history.allocated_bytes()
            + self.damage.allocated_bytes()
            + self.aces.allocated_bytes()
            + self.statistics.allocated_bytes()
            + self.names.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.equipment.grow(&mut capacity.rows[0], successful[0]);
        self.bases.grow(&mut capacity.rows[1], successful[1]);
        self.cumulative.grow(&mut capacity.rows[2], successful[2]);
        self.history.grow(&mut capacity.rows[3], successful[3]);
        self.damage.grow(&mut capacity.rows[4], successful[4]);
        self.aces.grow(&mut capacity.rows[5], successful[5]);
        self.statistics.grow(&mut capacity.rows[6], successful[6]);
        self.names.grow(&mut capacity.ace_names, successful[5]);
    }

    /// Initialize both extension passes immediately before the shared map walk.
    /// The shared map second census remains needed while either extension still
    /// requires its independent consistency check.
    pub fn begin_wings(&mut self) {
        self.equipment.reset();
        self.cumulative.reset();
        self.equipment.outcome = Outcome::Rows { len: 0 };
        self.cumulative.outcome = Outcome::Rows { len: 0 };
        self.wing_second = [Outcome::Rows { len: 0 }; 2];
        self.wing_matched = [false; 2];
        self.wing_ticks = [0; 2];
    }

    /// Extension failure never changes the map row. Equipment retains full
    /// native references and amounts, with no transient pointers in pooled rows.
    pub fn visit_wing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        source: WingSource<'_>,
        needed: &[bool],
    ) {
        if source.second && !needed[0] && !needed[2] {
            return;
        }
        let mut begin = qpc();
        if !source.second || needed[0] {
            let (output, outcome) = if source.second {
                (&mut self.equipment.second, &mut self.wing_second[0])
            } else {
                (&mut self.equipment.rows, &mut self.equipment.outcome)
            };
            if !matches!(outcome, Outcome::Unavailable { .. }) {
                let next = read_equipment(memory, base, &source, &mut output[outcome.len()..]);
                *outcome = append(*outcome, next);
            }
            let end = qpc();
            self.wing_ticks[0] =
                self.wing_ticks[0].saturating_add(end.saturating_sub(begin).max(0) as u64);
            begin = end;
        }
        if !source.second || needed[2] {
            let (output, outcome) = if source.second {
                (&mut self.cumulative.second, &mut self.wing_second[1])
            } else {
                (&mut self.cumulative.rows, &mut self.cumulative.outcome)
            };
            if !matches!(outcome, Outcome::Unavailable { .. }) {
                let next = if let Some(destination) = output.get_mut(outcome.len()) {
                    match cair_wing::read_cumulative(memory, source.pointer, source.row) {
                        Ok(row) => {
                            *destination = row;
                            Outcome::Rows { len: 1 }
                        }
                        Err(reason) => Outcome::Unavailable { reason },
                    }
                } else {
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 1,
                    }
                };
                *outcome = append(*outcome, next);
            }
            self.wing_ticks[1] = self.wing_ticks[1].saturating_add(elapsed(begin));
        }
    }

    /// Parent completeness is essential: a shorter emitted map prefix cannot
    /// certify an equipment census. Returned pairs are equipment/cumulative.
    /// Subtract returned ticks from the enclosing map walk's charged ticks.
    pub fn finish_wings(
        &mut self,
        first: Outcome,
        second: Option<Outcome>,
        needed: &[bool],
    ) -> ([bool; 2], [u64; 2]) {
        self.equipment.outcome = require_parent(first, self.equipment.outcome);
        self.cumulative.outcome = require_parent(first, self.cumulative.outcome);
        for (slot, local_index) in [(0, 0), (1, 2)] {
            self.wing_second[slot] = if needed[local_index] {
                require_parent(second.unwrap_or(NO_WING_SOURCE), self.wing_second[slot])
            } else {
                NO_WING_SOURCE
            };
        }
        if needed[0] {
            let begin = qpc();
            self.equipment.checked = true;
            self.wing_matched[0] = self.equipment.compare(self.wing_second[0]);
            self.wing_ticks[0] = self.wing_ticks[0].saturating_add(elapsed(begin));
        }
        if needed[2] {
            let begin = qpc();
            self.cumulative.checked = true;
            self.wing_matched[1] = self.cumulative.compare(self.wing_second[1]);
            self.wing_ticks[1] = self.wing_ticks[1].saturating_add(elapsed(begin));
        }
        (self.wing_matched, self.wing_ticks)
    }

    /// Monotonic nested equipment/cumulative ticks for the map adapter. Reading
    /// this before/after each visitor or finish call avoids charging work twice.
    pub fn wing_ticks(&self) -> [u64; 2] {
        self.wing_ticks
    }

    /// Five independent sources plus the two extensions supplied by the map
    /// visitor. Reset non-wing buffers here, preserving the prepared extensions.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.bases.reset();
        self.history.reset();
        self.damage.reset();
        self.aces.reset();
        self.statistics.reset();
        self.names.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        matched[0] = self.wing_matched[0];
        matched[2] = self.wing_matched[1];
        ticks[0] = self.wing_ticks[0];
        ticks[2] = self.wing_ticks[1];
        let begin = qpc();
        matched[1] = capture_rows(&mut self.bases, needed[1], |rows| {
            cair_base::walk(memory, base, game_state, rows)
        });
        ticks[1] = elapsed(begin);

        let begin = qpc();
        let first = cair_region_combat_data::walk_world(
            memory,
            base,
            game_state,
            &mut self.history.rows,
            &mut self.damage.rows,
        );
        self.history.outcome = bounded(first.records, self.history.rows.len());
        self.damage.outcome = bounded(first.damage, self.damage.rows.len());
        let second = if needed[3] || needed[4] {
            Some(cair_region_combat_data::walk_world(
                memory,
                base,
                game_state,
                &mut self.history.second,
                &mut self.damage.second,
            ))
        } else {
            None
        };
        ticks[3] = elapsed(begin);
        if let Some(second) = second {
            if needed[3] {
                let begin = qpc();
                self.history.checked = true;
                matched[3] = self
                    .history
                    .compare(bounded(second.records, self.history.second.len()));
                ticks[3] = ticks[3].saturating_add(elapsed(begin));
            }
            if needed[4] {
                let begin = qpc();
                self.damage.checked = true;
                matched[4] = self
                    .damage
                    .compare(bounded(second.damage, self.damage.second.len()));
                ticks[4] = elapsed(begin);
            }
        }

        let begin = qpc();
        let first = cace::walk(
            memory,
            base,
            game_state,
            &mut self.aces.rows,
            &mut self.names.first,
        );
        self.aces.outcome = names_outcome(first, &self.aces.rows, &self.names.first);
        self.names.len = first.bytes_len;
        self.names.required = first.required_bytes;
        if needed[5] {
            self.aces.checked = true;
            let second = cace::walk(
                memory,
                base,
                game_state,
                &mut self.aces.second,
                &mut self.names.second,
            );
            let outcome = names_outcome(second, &self.aces.second, &self.names.second);
            self.aces.compare(outcome);
            self.aces.mismatches +=
                usize::from(!self.names.compare(second.bytes_len, second.required_bytes));
            matched[5] = full_match(self.aces.outcome, outcome, self.aces.mismatches);
        }
        ticks[5] = elapsed(begin);

        let begin = qpc();
        matched[6] = capture_rows(&mut self.statistics, needed[6], |rows| {
            cloop_history::walk(memory, base, game_state, rows)
        });
        ticks[6] = elapsed(begin);
        (matched, ticks)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.equipment.mismatches,
            self.bases.mismatches,
            self.cumulative.mismatches,
            self.history.mismatches,
            self.damage.mismatches,
            self.aces.mismatches,
            self.statistics.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.equipment),
            effective(&self.bases),
            effective(&self.cumulative),
            effective(&self.history),
            effective(&self.damage),
            effective(&self.aces),
            effective(&self.statistics),
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::WingEquipment, _>(outcomes[0], &self.equipment.rows, Clone::clone),
            table::<tables::AirBases, _>(outcomes[1], &self.bases.rows, |r| rows::AirBaseRow {
                id: r.id,
                state: r.state,
                province: r.province,
                host: r.host,
                capacity: r.capacity,
                kind: r.kind,
            }),
            table::<tables::WingCumulative, _>(outcomes[2], &self.cumulative.rows, |r| {
                rows::WingCumulativeRow {
                    id: r.id,
                    accidents: r.accidents,
                    experience: r.experience,
                }
            }),
            history_table(outcomes[3], &self.history.rows),
            damage_table(outcomes[4], &self.damage.rows),
            self.ace_table(outcomes[5]),
            table::<tables::AirStatistics, _>(outcomes[6], &self.statistics.rows, |r| {
                rows::AirStatisticsRow {
                    country: r.country,
                    region: r.region,
                    year_slot: r.year_slot,
                    year_window_wrapped: r.year_window_wrapped,
                    channels: r.channels,
                }
            }),
        ]
    }

    fn ace_table(&self, outcome: Outcome) -> TableView {
        if self.names.required < self.names.len
            || !valid_names(outcome, &self.aces.rows, &self.names.first, self.names.len)
        {
            return unavailable("ace name range exceeds captured arena");
        }
        table::<tables::AirAces, _>(outcome, &self.aces.rows, |r| {
            let text = |range: cace::TextRange| {
                self.names.first[range.start..range.start + range.len].to_vec()
            };
            rows::AirAceRow {
                id: r.id,
                country: r.country,
                ordinal: r.ordinal,
                aircraft_type_mask: r.aircraft_type_mask,
                assigned_wing: r.assigned_wing,
                alive: r.alive,
                handled: r.handled,
                kill_type: r.kill_type,
                death: r.death.map(|death| rows::AceDeath {
                    killer_ace: death.killer_ace,
                    killer_country: death.killer_country,
                }),
                name: text(r.name),
                surname: text(r.surname),
                callsign: text(r.callsign),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "air walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "air walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn capture_rows<R: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> bool {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
    if !needed {
        return false;
    }
    captured.checked = true;
    let second = bounded(walk(&mut captured.second), captured.second.len());
    captured.compare(second)
}

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "air table changed during double read",
        }
    } else {
        captured.outcome
    }
}

fn require_parent(parent: Outcome, child: Outcome) -> Outcome {
    match parent {
        Outcome::Rows { .. } => child,
        Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
        Outcome::Truncated { .. } => Outcome::Unavailable {
            reason: "shared air wing census truncated; dependent census incomplete",
        },
    }
}

fn append(first: Outcome, next: Outcome) -> Outcome {
    let count = |outcome| match outcome {
        Outcome::Rows { len } => Ok((len, 0_u32)),
        Outcome::Truncated { len, unread_rows } => Ok((len, unread_rows)),
        Outcome::Unavailable { reason } => Err(reason),
    };
    let result = (|| {
        let (len, omitted) = count(first)?;
        let (next_len, next_omitted) = count(next)?;
        let len = len.checked_add(next_len).ok_or("air row count overflow")?;
        let unread_rows = omitted
            .checked_add(next_omitted)
            .ok_or("air omitted row count overflow")?;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Read entries individually so only native references and amounts are retained;
/// the pointer used to authenticate each variant remains a local read context.
fn read_equipment(
    memory: &impl Memory,
    base: usize,
    source: &WingSource<'_>,
    output: &mut [rows::WingEquipmentRow],
) -> Outcome {
    let result = (|| {
        let entries = cair_wing::equipment_pool(memory, base, source.pointer)?;
        let len = output.len().min(entries.count as usize);
        for (ordinal, destination) in output[..len].iter_mut().enumerate() {
            let address = entries
                .element(ordinal as u32, equipment_entry::STRIDE)
                .ok_or("air equipment entry address overflow")?;
            let entry = equipment_entry::read(memory, base, address, ordinal as u32)?;
            *destination = rows::WingEquipmentRow {
                wing: source.row.id,
                ordinal: entry.source_ordinal,
                variant: entry.variant_handle,
                amount_raw: entry.amount_raw,
            };
        }
        Ok(if len == entries.count as usize {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: entries.count - len as u32,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn names_outcome(walk: cace::NameWalk, rows: &[cace::Row], bytes: &[u8]) -> Outcome {
    let outcome = bounded(walk.outcome, rows.len());
    if walk.required_bytes < walk.bytes_len || !valid_names(outcome, rows, bytes, walk.bytes_len) {
        Outcome::Unavailable {
            reason: "ace name range exceeds captured arena",
        }
    } else {
        outcome
    }
}

fn valid_names(outcome: Outcome, rows: &[cace::Row], bytes: &[u8], len: usize) -> bool {
    len <= bytes.len()
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                [row.name, row.surname, row.callsign].iter().all(|range| {
                    range
                        .start
                        .checked_add(range.len)
                        .is_some_and(|end| end <= len)
                })
            })
        })
}

/// A partial native entry stream cannot name the exact count of missing grouped
/// parents, and its last parent may be partial. Keep that hour unavailable with
/// a specific capacity reason; raw capture truncation still grows the pool.
fn grouped_source<R>(outcome: Outcome, rows: &[R]) -> Result<&[R], TableView> {
    match bounded(outcome, rows.len()) {
        Outcome::Rows { len } => Ok(&rows[..len]),
        Outcome::Truncated { len, unread_rows } => Err(unavailable(&format!(
            "air regional entry capacity: {len} captured, {unread_rows} omitted; grouped table incomplete"
        ))),
        Outcome::Unavailable { reason } => Err(unavailable(reason)),
    }
}

fn history_table(outcome: Outcome, captured: &[sair_wing_combat_data::Row]) -> TableView {
    let captured = match grouped_source(outcome, captured) {
        Ok(rows) => rows,
        Err(table) => return table,
    };
    let groups: Vec<_> = captured
        .chunk_by(|a, b| (a.key.country, a.key.region) == (b.key.country, b.key.region))
        .collect();
    // The shared table helper rejects a repeated outer parent separated by a
    // different parent. Native duplicate children within a parent remain exact.
    table::<tables::AirRegionHistory, _>(Outcome::Rows { len: groups.len() }, &groups, |group| {
        rows::AirRegionHistoryRow {
            country: group[0].key.country,
            region: group[0].key.region,
            entries: group
                .iter()
                .map(|row| rows::AirHistoryEntry {
                    side: row.key.side,
                    date: row.key.date,
                    wing_id: row.key.wing_id,
                    classification: row.key.classification,
                    region_owner: row.region_owner,
                    planes: row.planes,
                    mission_mask: row.mission_mask,
                    selection: row.selection,
                    copied: row.copied,
                    attributed_country: row.attributed_country,
                })
                .collect(),
        }
    })
}

fn damage_table(outcome: Outcome, captured: &[sdamage_entry::Row]) -> TableView {
    let captured = match grouped_source(outcome, captured) {
        Ok(rows) => rows,
        Err(table) => return table,
    };
    let groups: Vec<_> = captured
        .chunk_by(|a, b| (a.record.country, a.record.region) == (b.record.country, b.record.region))
        .collect();
    table::<tables::AirRegionDamage, _>(Outcome::Rows { len: groups.len() }, &groups, |group| {
        rows::AirRegionDamageRow {
            country: group[0].record.country,
            region: group[0].record.region,
            entries: group
                .iter()
                .map(|row| rows::AirDamageEntry {
                    side: row.record.side,
                    date: row.record.date,
                    wing_id: row.record.wing_id,
                    classification: row.record.classification,
                    channel: row.channel,
                    cause: row.cause,
                    amount: row.amount,
                })
                .collect(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacity = Capacities {
            rows: [3; N],
            ace_names: 8,
        };
        let captured = Captured::with_capacities(&capacity, &[0; N]);
        (capacity, captured)
    }

    fn complete<S: TableSchema>(table: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = table else {
            panic!("expected complete table");
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn equipment_capture_preserves_full_handles_order_and_exact_capacity() {
        use engine::{cequipment_variant, cequipment_variant_pool, memory::SliceMemory};
        const WING: usize = 0x1000;
        const ENTRIES: usize = 0x1400;
        const VARIANT: usize = 0x1480;
        const HANDLE: u64 = 0xfedc_ba98_0000_0046;
        let mut bytes = vec![0; 0x500];
        let pool = cair_wing::EQUIPMENT_POOL;
        bytes[pool..pool + 8]
            .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes());
        bytes[pool + cequipment_variant_pool::ITEMS..pool + cequipment_variant_pool::ITEMS + 8]
            .copy_from_slice(&(ENTRIES as u64).to_le_bytes());
        bytes[pool + cequipment_variant_pool::CAPACITY
            ..pool + cequipment_variant_pool::CAPACITY + 4]
            .copy_from_slice(&2_i32.to_le_bytes());
        bytes[pool + cequipment_variant_pool::COUNT..pool + cequipment_variant_pool::COUNT + 4]
            .copy_from_slice(&2_i32.to_le_bytes());
        bytes[VARIANT - WING..VARIANT - WING + 8]
            .copy_from_slice(&(cequipment_variant::VTABLE_RVA as u64).to_le_bytes());
        bytes[VARIANT - WING + 8..VARIANT - WING + 16].copy_from_slice(&HANDLE.to_le_bytes());
        for (ordinal, amount) in [0_i64, i64::MIN].into_iter().enumerate() {
            let at = ENTRIES - WING + ordinal * equipment_entry::STRIDE;
            bytes[at..at + 8].copy_from_slice(&(VARIANT as u64).to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&amount.to_le_bytes());
        }
        let memory = SliceMemory {
            base: WING,
            bytes: &bytes,
        };
        let row = cair_wing::Row {
            id: u64::MAX,
            ..Default::default()
        };
        let source = WingSource {
            pointer: WING,
            row: &row,
            second: true,
        };
        let mut output = std::array::from_fn::<_, 2, _>(|_| rows::WingEquipmentRow::default());
        assert_eq!(
            read_equipment(&memory, 0, &source, &mut output),
            Outcome::Rows { len: 2 }
        );
        assert!(
            output
                .iter()
                .all(|row| row.wing == u64::MAX && row.variant == HANDLE)
        );
        assert_eq!((output[0].ordinal, output[1].ordinal), (0, 1));
        assert_eq!((output[0].amount_raw, output[1].amount_raw), (0, i64::MIN));
        assert_eq!(
            read_equipment(&memory, 0, &source, &mut output[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn grouped_children_preserve_duplicate_coordinates_order_and_full_damage_amounts() {
        let key = sair_wing_combat_data::Key {
            country: u32::MAX,
            region: 0,
            side: 1,
            date: i32::MIN,
            wing_id: 0,
            classification: u8::MAX,
        };
        let history = [
            sair_wing_combat_data::Row {
                key,
                planes: i32::MAX,
                selection: 1,
                ..Default::default()
            },
            sair_wing_combat_data::Row {
                key,
                planes: i32::MIN,
                selection: 2,
                ..Default::default()
            },
        ];
        let view = history_table(Outcome::Rows { len: 2 }, &history);
        let groups = complete::<tables::AirRegionHistory>(&view);
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].country, groups[0].region), (u32::MAX, 0));
        assert_eq!(
            groups[0]
                .entries
                .iter()
                .map(|entry| (entry.planes, entry.selection))
                .collect::<Vec<_>>(),
            [(i32::MAX, 1), (i32::MIN, 2)]
        );
        let damage = [
            sdamage_entry::Row {
                record: key,
                channel: 1,
                cause: u32::MAX,
                amount: i64::MIN,
            },
            sdamage_entry::Row {
                record: key,
                channel: 1,
                cause: u32::MAX,
                amount: i64::MAX,
            },
        ];
        let view = damage_table(Outcome::Rows { len: 2 }, &damage);
        let groups = complete::<tables::AirRegionDamage>(&view);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].entries[0].amount, i64::MIN);
        assert_eq!(groups[0].entries[1].amount, i64::MAX);
        assert_eq!(groups[0].entries[0].cause, u32::MAX);
    }

    #[test]
    fn incomplete_group_and_repeated_outer_parent_are_never_complete_rows() {
        let first = sair_wing_combat_data::Row::default();
        let mut other = first;
        other.key.region = 3;
        assert!(matches!(
            history_table(Outcome::Rows { len: 3 }, &[first, other, first]),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            history_table(
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 4
                },
                &[first]
            ),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            history_table(Outcome::Rows { len: 2 }, &[first]),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            damage_table(
                Outcome::Unavailable {
                    reason: "late root unreadable"
                },
                &[]
            ),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn ace_bytes_and_native_widths_survive_conversion_but_invalid_ranges_refuse() {
        let (_, mut captured) = small();
        captured.equipment.outcome = Outcome::Rows { len: 1 };
        captured.equipment.rows[0] = rows::WingEquipmentRow {
            wing: u64::MAX,
            ordinal: u32::MAX,
            variant: u64::MAX - 1,
            amount_raw: i64::MIN,
        };
        captured.cumulative.outcome = Outcome::Rows { len: 1 };
        captured.cumulative.rows[0] = cair_wing::CumulativeRow {
            id: u64::MAX,
            accidents: i32::MIN,
            experience: i64::MAX,
        };
        captured.aces.outcome = Outcome::Rows { len: 1 };
        captured.aces.rows[0] = cace::Row {
            id: u64::MAX,
            alive: 0,
            handled: 0,
            death: Some(cace::Death {
                killer_ace: u64::MAX - 2,
                killer_country: u32::MAX,
            }),
            name: cace::TextRange { start: 0, len: 3 },
            ..Default::default()
        };
        captured.names.first[..3].copy_from_slice(&[0xff, 0, b'a']);
        captured.names.len = 3;
        captured.names.required = 3;
        let views = captured.tables();
        let equipment = &complete::<tables::WingEquipment>(&views[0])[0];
        assert_eq!(
            (
                equipment.wing,
                equipment.ordinal,
                equipment.variant,
                equipment.amount_raw
            ),
            (u64::MAX, u32::MAX, u64::MAX - 1, i64::MIN)
        );
        assert_eq!(
            complete::<tables::WingCumulative>(&views[2])[0].experience,
            i64::MAX
        );
        let ace = &complete::<tables::AirAces>(&views[5])[0];
        assert_eq!(ace.name, [0xff, 0, b'a']);
        assert_eq!(ace.death.as_ref().unwrap().killer_ace, u64::MAX - 2);
        for range in [
            cace::TextRange {
                start: usize::MAX,
                len: 1,
            },
            cace::TextRange { start: 2, len: 2 },
        ] {
            captured.aces.rows[0].name = range;
            assert!(matches!(
                captured.ace_table(captured.aces.outcome),
                TableView::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn source_failure_never_certifies_a_prefix_and_capacity_counts_remain_exact() {
        let partial = Outcome::Truncated {
            len: 3,
            unread_rows: 4,
        };
        assert_eq!(
            append(
                partial,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 7
                }
            ),
            Outcome::Truncated {
                len: 3,
                unread_rows: 11
            }
        );
        assert!(matches!(
            append(
                Outcome::Truncated {
                    len: 0,
                    unread_rows: u32::MAX
                },
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                }
            ),
            Outcome::Unavailable { .. }
        ));
        assert_eq!(require_parent(Outcome::Rows { len: 5 }, partial), partial);
        assert!(matches!(
            require_parent(partial, Outcome::Rows { len: 0 }),
            Outcome::Unavailable { .. }
        ));
        assert_eq!(
            require_parent(
                Outcome::Unavailable {
                    reason: "late wing root"
                },
                Outcome::Rows { len: 2 }
            ),
            Outcome::Unavailable {
                reason: "late wing root"
            }
        );
        let (_, mut captured) = small();
        captured.equipment.outcome = Outcome::Rows { len: 0 };
        captured.equipment.checked = true;
        captured.equipment.mismatches = 1;
        captured.cumulative.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 0 });
    }

    #[test]
    fn growth_counts_native_entries_and_releases_completed_comparison_storage() {
        let (mut capacity, mut captured) = small();
        captured.history.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 60,
        };
        captured.names.required = 100;
        captured.grow(&mut capacity, &[0; N]);
        assert!(capacity.rows[3] >= 63);
        assert!(capacity.ace_names >= 100);
        assert_eq!(captured.history.rows.len(), captured.history.second.len());
        let before = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET; N]);
        assert!(captured.equipment.second.is_empty());
        assert!(captured.names.second.is_empty());
        assert!(captured.allocated_bytes() < before);
    }
}
