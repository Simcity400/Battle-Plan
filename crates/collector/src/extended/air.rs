//! Pooled air state. Acquisition never allocates; conversion and growth are off-thread.
//!
//! Wing-fed split (copy/decode): WingEquipment and WingCumulative move to
//! the single FIFO decode worker as part of the map-pack coupled unit (one
//! task). Each stages its own wing-census copies with no cross-family
//! decoded roots; zero-engine-change replay of the existing wing walk plus
//! equipment/cumulative reads against `Recorded` spans. Choice per table:
//! zero-engine-change (Wars/Divisions precedent for census walks). The other
//! five air tables stay game-side.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cace, cair_base, cair_region_combat_data, cair_wing, cair_wing_pool, ccountry,
    ccountry_air_container, cequipment_variant, cequipment_variant_pool, cgame_state,
    cloop_history, cpdx_array::Array, cprovince, cstate, cstrategic_air, cstrategic_air_manager,
    equipment_entry, msvc_string, sair_wing_combat_data, sdamage_entry, walk::Outcome,
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
    /// Raw-copy split gate for the hourly air-statistics table. Set by the
    /// game thread on hours with a valid raw payload; `capture` then skips
    /// the typed statistics walk and the decode worker fills these rows
    /// before the ready handoff. Preserved across `reset` (set per hour,
    /// cleared by the worker after decode).
    pub(crate) stats_raw_mode: bool,
    /// Raw-copy split gate for the shared bases/registrations/aces payload
    /// (copy/decode split). Set by the game thread every hour with a valid
    /// raw payload; `capture` then skips the typed bases and aces walks and
    /// the decode worker fills those rows (plus the fog registrations) before
    /// the ready handoff. Preserved across `reset`, cleared by the worker.
    pub(crate) bases_raw_mode: bool,
    /// Raw-copy split gate for the shared history/damage payload
    /// (copy/decode split). Set by the game thread every hour with a valid
    /// raw payload; `capture` then skips the typed regional walk and the
    /// decode worker fills both row sets before the ready handoff. Preserved
    /// across `reset`, cleared by the worker.
    pub(crate) history_raw_mode: bool,
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
            stats_raw_mode: false,
            bases_raw_mode: false,
            history_raw_mode: false,
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
        // Hourly bases/aces raw-copy split: on hours with a valid raw payload
        // the game thread stages bytes via `copy_air_bases_refresh` (shared
        // with the fog registrations) and the decode worker fills the typed
        // rows before the ready handoff, so the typed walks are skipped here.
        // Matched flags and costs come from the raw compare, applied by the
        // caller after this returns.
        if self.bases_raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows and outcomes.
            matched[1] = false;
            ticks[1] = 0;
        } else {
            matched[1] = capture_rows(&mut self.bases, needed[1], |rows| {
                cair_base::walk(memory, base, game_state, rows)
            });
            ticks[1] = elapsed(begin);
        }

        let begin = qpc();
        // Hourly history/damage raw-copy split: on hours with a valid raw
        // payload the game thread stages bytes via `copy_air_history_refresh`
        // and the decode worker fills both row sets before the ready handoff,
        // so the typed regional walk is skipped here.
        if self.history_raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows and outcomes.
            matched[3] = false;
            matched[4] = false;
            ticks[3] = 0;
        } else {
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
        }

        let begin = qpc();
        if self.bases_raw_mode {
            // Refresh payload staged (shared bases/aces job): leave the reset
            // outcome for the decode worker.
            matched[5] = false;
            ticks[5] = 0;
        } else {
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
        }

        let begin = qpc();
        // Hourly statistics raw-copy split: on hours with a valid raw payload
        // the game thread stages bytes via `copy_air_stats_refresh` and the
        // decode worker fills the typed rows before the ready handoff, so the
        // typed walk is skipped here. Matched flags and costs come from the
        // raw compare, applied by the caller after this returns.
        if self.stats_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            matched[6] = false;
            ticks[6] = 0;
        } else {
            matched[6] = capture_rows(&mut self.statistics, needed[6], |rows| {
                cloop_history::walk(memory, base, game_state, rows)
            });
            ticks[6] = elapsed(begin);
        }
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

pub(crate) const WING_PACK_N: usize = 2;
const EQUIPMENT_MASK: u32 = 0x01;
const CUMULATIVE_MASK: u32 = 0x02;
const WING_CENSUS_MASK: u32 = 0x03;

#[derive(Debug, Default)]
pub(crate) struct AirWingJob {
    pub log: RawLog<WING_PACK_N>,
    pub game_state: usize,
    checked: [bool; WING_PACK_N],
    matched: [bool; WING_PACK_N],
}

impl AirWingJob {
    pub fn new() -> Self {
        Self {
            log: RawLog::new(1 << 16, 4 << 20),
            game_state: 0,
            checked: [false; WING_PACK_N],
            matched: [false; WING_PACK_N],
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.checked = [false; WING_PACK_N];
        self.matched = [false; WING_PACK_N];
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirWingCopyReport {
    pub raw_ok: bool,
    pub matched: [bool; WING_PACK_N],
    pub copy_ticks: [u64; WING_PACK_N],
    pub raw_bytes: [u64; WING_PACK_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirWingCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; WING_PACK_N],
}

fn wing_pool_u64(log: &RawLog<WING_PACK_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn wing_pool_u32(log: &RawLog<WING_PACK_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn wspan(
    log: &mut RawLog<WING_PACK_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn wing_link(
    log: &mut RawLog<WING_PACK_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = wspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        WING_CENSUS_MASK,
    )?;
    let slot = wing_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

fn copy_air_census(
    log: &mut RawLog<WING_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) -> Option<()> {
    use engine::{cair_wing_pool, ccountry, cstrategic_air, cstrategic_air_manager};
    let countries = wspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        WING_CENSUS_MASK,
    )?;
    let links = wspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
        16,
        WING_CENSUS_MASK,
    )?;
    let manager_ptr = wspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::STRATEGIC_AIR_MANAGER),
        8,
        WING_CENSUS_MASK,
    )
    .and_then(|s| {
        usize::try_from(wing_pool_u64(log, s))
            .ok()
            .filter(|&p| p != 0)
    })?;
    let _ = wspan(log, memory, Some(manager_ptr), 8, WING_CENSUS_MASK)?;
    let owners_header = wspan(
        log,
        memory,
        manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR),
        16,
        WING_CENSUS_MASK,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries..countries + 16]);
    let countries_array = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links..links + 16]);
    let links_array = Array::from_header(raw, 4)?;
    raw.copy_from_slice(&log.bytes[owners_header..owners_header + 16]);
    let owners = Array::from_header(raw, 8)?;
    if owners.count == 0 || countries_array.count < 2 || links_array.count == 0 {
        return None;
    }
    let country_vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let pointer = wspan(log, memory, pointer_addr, 8, WING_CENSUS_MASK)
            .and_then(|s| usize::try_from(wing_pool_u64(log, s)).ok())?;
        if pointer == 0 {
            continue;
        }
        let vtable = wspan(log, memory, Some(pointer), 8, WING_CENSUS_MASK)?;
        if wing_pool_u64(log, vtable) != country_vtable {
            return None;
        }
        let id = wspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            WING_CENSUS_MASK,
        )?;
        let id = wing_pool_u32(log, id);
        let canonical = wing_link(log, memory, &links_array, countries_array.count, id)?;
        if wing_link(log, memory, &links_array, countries_array.count, slot) != Some(canonical) {
            return None;
        }
        if wing_link(log, memory, &links_array, countries_array.count, canonical) != Some(canonical)
        {
            return None;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let canonical_pointer = wspan(log, memory, canonical_addr, 8, WING_CENSUS_MASK)?;
        if wing_pool_u64(log, canonical_pointer) != pointer as u64 {
            return None;
        }
    }
    for slot in 0..owners.count {
        let owner_addr = owners.data.checked_add((slot as usize).saturating_mul(8));
        let owner_pointer = wspan(log, memory, owner_addr, 8, WING_CENSUS_MASK)
            .and_then(|s| usize::try_from(wing_pool_u64(log, s)).ok())?;
        if owner_pointer == 0 {
            continue;
        }
        let _ = wspan(log, memory, Some(owner_pointer), 8, WING_CENSUS_MASK)?;
        let owner_field = wspan(
            log,
            memory,
            owner_pointer.checked_add(cstrategic_air::COUNTRY),
            4,
            WING_CENSUS_MASK,
        )?;
        let owner = wing_pool_u32(log, owner_field);
        if owner != 0 && owner < links_array.count {
            let offset = (owner as usize).checked_mul(4);
            let _ = wspan(
                log,
                memory,
                offset.and_then(|o| links_array.data.checked_add(o)),
                4,
                WING_CENSUS_MASK,
            )?;
        }
        let pools_header = wspan(
            log,
            memory,
            owner_pointer.checked_add(cstrategic_air::POOLS),
            16,
            WING_CENSUS_MASK,
        )?;
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[pools_header..pools_header + 16]);
        let pools = match Array::from_header(header, 8) {
            Some(array) => array,
            None => continue,
        };
        for pool_index in 0..pools.count {
            let pool_addr = pools
                .data
                .checked_add((pool_index as usize).saturating_mul(8));
            let pool = wspan(log, memory, pool_addr, 8, WING_CENSUS_MASK).and_then(|s| {
                usize::try_from(wing_pool_u64(log, s))
                    .ok()
                    .filter(|&p| p != 0)
            })?;
            let _ = wspan(log, memory, Some(pool), 8, WING_CENSUS_MASK)?;
            let _ = wspan(
                log,
                memory,
                pool.checked_add(cair_wing_pool::OWNER),
                8,
                WING_CENSUS_MASK,
            )?;
            let wings_header = wspan(
                log,
                memory,
                pool.checked_add(cair_wing_pool::WINGS),
                16,
                WING_CENSUS_MASK,
            )?;
            let _ = wspan(
                log,
                memory,
                pool.checked_add(cair_wing_pool::AIR_BASE_REFERENCE),
                8,
                WING_CENSUS_MASK,
            )?;
            let mut wings_raw = [0u8; 16];
            wings_raw.copy_from_slice(&log.bytes[wings_header..wings_header + 16]);
            let wings = match Array::from_header(wings_raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for wing_index in 0..wings.count {
                let wing_addr = wings
                    .data
                    .checked_add((wing_index as usize).saturating_mul(8));
                let wing = wspan(log, memory, wing_addr, 8, WING_CENSUS_MASK)
                    .and_then(|s| usize::try_from(wing_pool_u64(log, s)).ok())?;
                if wing == 0 {
                    continue;
                }
                copy_air_wing(log, memory, base, wing);
                if log.overflowed {
                    return None;
                }
            }
        }
    }
    Some(())
}

fn copy_air_wing(
    log: &mut RawLog<WING_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    wing: usize,
) {
    use engine::{air_mission, cair_wing, cstrategic_region};
    let mut region = 0;
    let mut primary_valid = false;
    let mut secondary_valid = false;
    for (offset, len, mask) in [
        (0usize, 8, WING_CENSUS_MASK),
        (cair_wing::SECONDARY, 16, WING_CENSUS_MASK),
        (cair_wing::POOL, 8, WING_CENSUS_MASK),
        (cair_wing::BASE_OVERRIDE, 8, WING_CENSUS_MASK),
        (cair_wing::PLANE_COUNT, 4, WING_CENSUS_MASK),
        (cair_wing::EFFECTIVE_PLANES, 4, WING_CENSUS_MASK),
        (
            cair_wing::MISSION + air_mission::ENABLED_MASK,
            4,
            WING_CENSUS_MASK,
        ),
        (
            cair_wing::MISSION + air_mission::REGION,
            8,
            WING_CENSUS_MASK,
        ),
        (cair_wing::COUNTRY_OVERRIDE, 4, WING_CENSUS_MASK),
        (cair_wing::AIR_ACCIDENTS, 4, CUMULATIVE_MASK),
        (cair_wing::EXPERIENCE, 8, CUMULATIVE_MASK),
    ] {
        let Some(start) = wspan(log, memory, wing.checked_add(offset), len, mask) else {
            if mask == CUMULATIVE_MASK {
                continue;
            }
            return;
        };
        if offset == 0 {
            primary_valid = base.checked_add(cair_wing::VTABLE_RVA)
                == usize::try_from(wing_pool_u64(log, start)).ok();
        } else if offset == cair_wing::SECONDARY {
            secondary_valid = base.checked_add(cair_wing::SECONDARY_VTABLE_RVA)
                == usize::try_from(wing_pool_u64(log, start)).ok()
                && wing_pool_u64(log, start + 8) != 0;
        } else if offset == cair_wing::MISSION + air_mission::REGION {
            let Ok(pointer) = usize::try_from(wing_pool_u64(log, start)) else {
                return;
            };
            region = pointer;
        }
    }
    if !primary_valid || !secondary_valid {
        return;
    }
    // A mission region is a pointer, not the saved id. The typed wing
    // walker authenticates both subobjects before reading that id.
    if region != 0 {
        for (offset, len) in [
            (0, 8),
            (cstrategic_region::SECONDARY, 8),
            (cstrategic_region::ID, 4),
        ] {
            if wspan(
                log,
                memory,
                region.checked_add(offset),
                len,
                WING_CENSUS_MASK,
            )
            .is_none()
            {
                return;
            }
        }
    }
    // Equipment pool with per-entry variant identities (equipment table).
    let pool = match wing.checked_add(cair_wing::EQUIPMENT_POOL) {
        Some(pool) => pool,
        None => return,
    };
    let expected = match base.checked_add(cequipment_variant_pool::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let vtable = match wspan(log, memory, Some(pool), 8, EQUIPMENT_MASK) {
        Some(start) => start,
        None => return,
    };
    if wing_pool_u64(log, vtable) != expected {
        return;
    }
    let header = match wspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ITEMS),
        16,
        EQUIPMENT_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let array = match Array::from_header(raw, equipment_entry::STRIDE) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let entry_addr = array
            .data
            .checked_add((index as usize).saturating_mul(equipment_entry::STRIDE));
        let entry = match wspan(
            log,
            memory,
            entry_addr,
            equipment_entry::STRIDE,
            EQUIPMENT_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let variant = match usize::try_from(u64::from_le_bytes(
            log.bytes[entry..entry + 8].try_into().unwrap(),
        ))
        .ok()
        {
            Some(variant) => variant,
            None => return,
        };
        if wspan(log, memory, Some(variant), 16, EQUIPMENT_MASK).is_none() {
            return;
        }
        if log.overflowed {
            return;
        }
    }
    let _ = cequipment_variant::VTABLE_RVA;
}

fn copy_air_pass(
    job: &mut AirWingJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; WING_PACK_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    let _ = copy_air_census(&mut job.log, memory, base, game_state);
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

pub(crate) fn copy_air_refresh(
    memory: &impl engine::Memory,
    ctx: AirWingCopyContext,
    first: &mut AirWingJob,
    mut second: Option<&mut AirWingJob>,
) -> AirWingCopyReport {
    let mut ticks = [0u64; WING_PACK_N];
    copy_air_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    first.checked = ctx.needed.map(|needed| needed && second.is_some());
    let mut matched = [true; WING_PACK_N];
    if let Some(second_job) = second.as_deref_mut() {
        let mut second_ticks = [0u64; WING_PACK_N];
        copy_air_pass(
            second_job,
            memory,
            ctx.base,
            ctx.game_state,
            &mut second_ticks,
        );
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        // Per-table partitions: shared wing census gates both slots,
        // equipment/cumulative private spans only their own.
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
    first.matched = matched;
    AirWingCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

pub(crate) fn decode_air_payload(
    captured: &mut Captured,
    base: usize,
    job: &AirWingJob,
    decode_ticks: &mut [u64; WING_PACK_N],
) {
    let recorded = crate::raw::Recorded::build(&job.log);
    let started = qpc();
    // Replay through the existing visitor path: begin, per-wing visit
    // (equipment + cumulative into pooled buffers), finish. Mirrors the game
    // thread's `Sources::air` sequence for byte identity. The Wing rows
    // themselves live in the map job; only equipment/cumulative install here.
    // The walk's Wing-row buffer grows worker-side until the census fits, so
    // extension visits never truncate from a short census buffer (the game
    // thread withholds extensions only when its own map AirWings truncates,
    // via require_parent on the map outcome, which this payload does not
    // share). Worker-side growth converges like pooled growth.
    // Consistency was checked on the two live copies. This is one decode
    // pass; comparing it with a nonexistent second census invents failures.
    let needed = [false; N];
    let mut wing_rows: Vec<cair_wing::Row> = Vec::new();
    loop {
        captured.begin_wings();
        let mut collected: Vec<(usize, cair_wing::Row)> = Vec::new();
        let outcome = cair_wing::walk_with(
            &recorded,
            base,
            job.game_state,
            &mut wing_rows,
            |pointer, _, row| {
                collected.push((pointer, *row));
            },
        );
        match outcome {
            Outcome::Truncated { .. } => {
                wing_rows.resize(wing_rows.len() * 2 + 16, cair_wing::Row::default());
                continue;
            }
            _ => {
                for (pointer, row) in &collected {
                    let source = WingSource {
                        pointer: *pointer,
                        row,
                        second: false,
                    };
                    captured.visit_wing(&recorded, base, source, &needed);
                }
                captured.finish_wings(outcome, None, &needed);
                break;
            }
        }
    }
    captured.equipment.checked = job.checked[0];
    captured.equipment.mismatches = usize::from(job.checked[0] && !job.matched[0]);
    captured.cumulative.checked = job.checked[1];
    captured.cumulative.mismatches = usize::from(job.checked[1] && !job.matched[1]);
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

pub(crate) const AIR_STATS_N: usize = 1;
const STATS_MASK: u32 = 0x01;

/// One refresh of raw air-statistics payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_air_stats_job`).
#[derive(Debug, Default)]
pub(crate) struct AirStatsJob {
    pub log: RawLog<AIR_STATS_N>,
    pub game_state: usize,
}

impl AirStatsJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 330 statistics rows (appendix
            // census) with per-history channel spans (~1 KiB each worst
            // case); both pools start above that so steady state never
            // overflows.
            log: RawLog::new(1 << 16, 4 << 20),
            game_state: 0,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirStatsMemoContext {
    pub owner: usize,
    pub base: usize,
    pub seq: u64,
    pub hour: i32,
    pub force_full: bool,
    /// Sampling's authoritative flags, not reset local capture-pack flags.
    pub validation_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AirStatsMemoKey {
    owner: usize,
    base: usize,
    game_state: usize,
    rows: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirStatsMemoReport {
    pub hit: bool,
    pub eligible: bool,
    pub compared: bool,
    pub installed: bool,
    pub compare_ticks: u64,
    pub restore_ticks: u64,
    pub install_ticks: u64,
}

static AIR_STATS_MEMO_MEMORY: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();

pub(crate) fn air_stats_memo_memory() -> serde_json::Value {
    let snapshot = AIR_STATS_MEMO_MEMORY.snapshot();
    serde_json::json!({
        "scope": "one worker-owned AirStatistics memo; actual input and native-output vector capacities, retained across misses",
        "input_capacity_bytes": snapshot.current.payload_vectors,
        "native_output_capacity_bytes": snapshot.current.auxiliary_vectors,
        "current_capacity_bytes": snapshot.current.payload_vectors + snapshot.current.auxiliary_vectors,
        "peak_capacity_bytes": snapshot.peak_total_lower_bound,
        "excluded": ["inline memo metadata", "allocator rounding/metadata and transient reallocation overlap"],
    })
}

/// One complete native result of an exactly equal current owned invocation.
/// Capture remains fresh each hour; no previous game bytes implement Memory.
/// Full span equality, including exact/bulk aliases and repeated observations,
/// is required even when differences would leave emitted statistics unchanged.
#[derive(Debug)]
pub(crate) struct AirStatsMemo {
    key: Option<AirStatsMemoKey>,
    previous: Option<(u64, i32)>,
    spans: Vec<crate::raw::Span>,
    bytes: Vec<u8>,
    has_bulk: bool,
    rows: Vec<cloop_history::Row>,
    capacity_charge: crate::raw_memory::Charge,
}

impl Default for AirStatsMemo {
    fn default() -> Self {
        Self::on_tracker(&AIR_STATS_MEMO_MEMORY)
    }
}

impl AirStatsMemo {
    fn on_tracker(tracker: &'static crate::raw_memory::Tracker) -> Self {
        Self {
            key: None,
            previous: None,
            spans: Vec::new(),
            bytes: Vec::new(),
            has_bulk: false,
            rows: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::empty_on(tracker),
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.key = None;
        self.previous = None;
    }

    fn capacity(&self) -> crate::raw_memory::Capacity {
        use crate::raw_memory::{Capacity, vector_bytes};
        Capacity {
            payload_vectors: vector_bytes(&self.spans) + vector_bytes(&self.bytes),
            auxiliary_vectors: vector_bytes(&self.rows),
            ..Capacity::default()
        }
    }

    fn input_equal(&self, log: &RawLog<AIR_STATS_N>) -> bool {
        // Stored spans are all successes; field equality also rejects any
        // current failed observation. Do not compare Rust struct padding or
        // initialized but unused byte-arena tail.
        !log.overflowed
            && !log.fault[0]
            && self.has_bulk == log.has_bulk
            && self.spans == log.spans
            && self.bytes.as_slice() == log.bytes.as_ref()
    }

    pub(crate) fn decode(
        &mut self,
        captured: &mut Captured,
        job: &AirStatsJob,
        ctx: AirStatsMemoContext,
        decode_ticks: &mut [u64; AIR_STATS_N],
    ) -> AirStatsMemoReport {
        let mut report = AirStatsMemoReport::default();
        let compared_started = qpc();
        let adjacent = self.previous.is_some_and(|(seq, hour)| {
            seq.checked_add(1) == Some(ctx.seq) && hour.checked_add(1) == Some(ctx.hour)
        });
        if ctx.force_full || !adjacent {
            self.key = None;
        }
        let key = AirStatsMemoKey {
            owner: ctx.owner,
            base: ctx.base,
            game_state: job.game_state,
            // The logical slice length controls skipped history reads. Exact
            // equality is conservative even when a larger buffer would fit.
            rows: captured.statistics.rows.len(),
        };
        report.eligible = ctx.validation_allowed
            && captured.stats_raw_mode
            && !captured.statistics.checked
            && captured.statistics.mismatches == 0
            && !job.log.overflowed
            && !job.log.fault[0]
            && job.log.spans.iter().all(|span| span.ok);
        if report.eligible && self.key == Some(key) {
            report.compared = true;
            report.hit = self.input_equal(&job.log);
        }
        report.compare_ticks = elapsed(compared_started);
        decode_ticks[0] += report.compare_ticks;
        if report.hit {
            let started = qpc();
            captured.statistics.rows[..self.rows.len()].copy_from_slice(&self.rows);
            captured.statistics.outcome = Outcome::Rows {
                len: self.rows.len(),
            };
            captured.stats_raw_mode = false;
            // Keep current checks, mismatches and second storage untouched.
            self.previous = Some((ctx.seq, ctx.hour));
            report.restore_ticks = elapsed(started);
            decode_ticks[0] += report.restore_ticks;
            return report;
        }

        self.key = None;
        decode_air_stats_payload(captured, ctx.base, job, decode_ticks);
        let install_started = qpc();
        if report.eligible
            && matches!(captured.statistics.outcome, Outcome::Rows { len } if len <= captured.statistics.rows.len())
        {
            self.spans.clear();
            self.spans.extend_from_slice(&job.log.spans);
            self.bytes.clear();
            self.bytes.extend_from_slice(job.log.bytes.as_ref());
            self.has_bulk = job.log.has_bulk;
            self.rows.clear();
            self.rows
                .extend_from_slice(&captured.statistics.rows[..captured.statistics.outcome.len()]);
            self.capacity_charge.set(self.capacity());
            self.key = Some(key);
            self.previous = Some((ctx.seq, ctx.hour));
            report.installed = true;
        } else {
            self.invalidate();
        }
        // Includes the decision/validation even when installation is declined.
        report.install_ticks = elapsed(install_started);
        decode_ticks[0] += report.install_ticks;
        report
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirStatsCopyReport {
    pub raw_ok: bool,
    pub matched: [bool; AIR_STATS_N],
    pub copy_ticks: [u64; AIR_STATS_N],
    pub raw_bytes: [u64; AIR_STATS_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirStatsCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn sspan(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn stats_pool_u64(log: &RawLog<AIR_STATS_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn stats_pool_u32(log: &RawLog<AIR_STATS_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// Mirror of one history slot's `read_current` takes: vtable, field count,
/// self reference, window chain, yearly-window validation words, record
/// linkage and the nine channel spans. Returns false exactly when the typed
/// read fails for this history.
fn copy_stats_history(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine::Memory,
    base: usize,
    history: usize,
) -> bool {
    use engine::{
        cloop_history, cloop_history::air_stats, loop_history_record_fields, loop_history_window,
    };
    let vtable = match sspan(log, memory, Some(history), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let expected = match base.checked_add(cloop_history::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if stats_pool_u64(log, vtable) != expected {
        return false;
    }
    let field_count = match sspan(
        log,
        memory,
        history.checked_add(cloop_history::FIELD_COUNT),
        4,
        STATS_MASK,
    ) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    if field_count != air_stats::FIELD_COUNT {
        return false;
    }
    let self_ref = match sspan(
        log,
        memory,
        history.checked_add(cloop_history::SELF_REFERENCE),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, self_ref) != history as u64 {
        return false;
    }
    let window = match sspan(
        log,
        memory,
        history.checked_add(cloop_history::WINDOW),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let window = match usize::try_from(stats_pool_u64(log, window))
        .ok()
        .filter(|&p| p != 0)
    {
        Some(window) => window,
        None => return false,
    };
    let window_vtable = match sspan(log, memory, Some(window), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let expected_window = match base.checked_add(loop_history_window::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if stats_pool_u64(log, window_vtable) != expected_window {
        return false;
    }
    let records_header = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::RECORDS),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[records_header..records_header + 16]);
    let records = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    let capacity = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::CAPACITY),
        4,
        STATS_MASK,
    ) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    let year_slot = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::CURRENT_SLOT),
        4,
        STATS_MASK,
    ) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    let wrapped = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::WRAPPED),
        1,
        STATS_MASK,
    ) {
        Some(start) => log.bytes[start],
        None => return false,
    };
    if capacity != loop_history_window::AIR_STATS_YEARLY_CAPACITY
        || records.count != capacity
        || year_slot >= capacity
        || wrapped > 1
    {
        return false;
    }
    let owner_back = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::HISTORY),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, owner_back) != history as u64 {
        return false;
    }
    let record_ptr = match sspan(
        log,
        memory,
        window.checked_add(loop_history_window::RECORD_FIELDS),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let record = match usize::try_from(stats_pool_u64(log, record_ptr))
        .ok()
        .filter(|&p| p != 0)
    {
        Some(record) => record,
        None => return false,
    };
    let slot_addr = match (year_slot as usize)
        .checked_mul(8)
        .and_then(|o| records.data.checked_add(o))
    {
        Some(addr) => addr,
        None => return false,
    };
    let slot = match sspan(log, memory, Some(slot_addr), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, slot) != record as u64 {
        return false;
    }
    let record_owner = match sspan(
        log,
        memory,
        record.checked_add(loop_history_record_fields::HISTORY),
        8,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, record_owner) != history as u64 {
        return false;
    }
    let values_header = match sspan(
        log,
        memory,
        record.checked_add(loop_history_record_fields::VALUES),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    raw.copy_from_slice(&log.bytes[values_header..values_header + 16]);
    let values = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    if values.count != air_stats::FIELD_COUNT {
        return false;
    }
    copy_stats_channels(log, memory, values)
}

/// Retain the typed reader's nine exact channel spans and their observation
/// order. A full-capacity guard is selected before any source read; the cold
/// scalar path preserves partial admission and growth requests at pool limits.
fn copy_stats_channels(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine::Memory,
    values: Array,
) -> bool {
    let lengths = [32, 8, 8, 8, 72, 8, 8, 8, 40];
    let addresses = [0_usize, 10, 17, 24, 36, 51, 58, 65, 77].map(|index| {
        index
            .checked_mul(engine::loop_history_record_fields::VALUE_STRIDE)
            .and_then(|offset| values.data.checked_add(offset))
    });
    if !log.overflowed
        && addresses.iter().all(Option::is_some)
        && log.spans.capacity() - log.spans.len() >= lengths.len()
        && log
            .bytes
            .len()
            .checked_add(192)
            .is_some_and(|end| end <= log.bytes.capacity() && end <= u32::MAX as usize)
    {
        return log
            .take_scattered(
                memory,
                addresses.map(Option::unwrap),
                lengths,
                [STATS_MASK; 9],
            )
            .is_some_and(|(_, completed)| completed == lengths.len());
    }
    for (address, len) in addresses.into_iter().zip(lengths) {
        if sspan(log, memory, address, len, STATS_MASK).is_none() {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

fn copy_stats_pass(
    job: &mut AirStatsJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; AIR_STATS_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    // Single per-owner loop (no duplicate takes): census takes and history
    // takes interleave in owner order. The replay serves by exact
    // (address, length) with in-order multiplicity, so take order never
    // affects serving; both copy passes take deterministically, so the raw
    // compare stays exact.
    copy_stats_owners(&mut job.log, memory, base, game_state);
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

/// Countries/links/manager preamble plus one loop over owner slots: per
/// owner the identity takes, the history-table header take, every slot-word
/// take and every nonnull history object. Stops at the first typed abort
/// point, mirroring the walk's fail-fast.
fn copy_stats_owners(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{ccountry, cstrategic_air, cstrategic_air_manager};
    let countries = match sspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match sspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let manager_ptr = match sspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::STRATEGIC_AIR_MANAGER),
        8,
        STATS_MASK,
    )
    .and_then(|s| {
        usize::try_from(stats_pool_u64(log, s))
            .ok()
            .filter(|&p| p != 0)
    }) {
        Some(manager) => manager,
        None => return,
    };
    if sspan(log, memory, Some(manager_ptr), 8, STATS_MASK).is_none() {
        return;
    }
    let owners_header = match sspan(
        log,
        memory,
        manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR),
        16,
        STATS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries..countries + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links..links + 16]);
    let links_array = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[owners_header..owners_header + 16]);
    let owners = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if owners.count == 0 || countries_array.count < 2 || links_array.count == 0 {
        return;
    }
    // Every link entry the walk can read (canonical(slot)/canonical(id)
    // checks read each visited slot's entry plus each raw id's entry).
    // Taking the whole vector once covers all of them deterministically.
    for link in 0..links_array.count {
        let addr = match (link as usize)
            .checked_mul(4)
            .and_then(|o| links_array.data.checked_add(o))
        {
            Some(addr) => addr,
            None => return,
        };
        if sspan(log, memory, Some(addr), 4, STATS_MASK).is_none() {
            return;
        }
        if log.overflowed {
            return;
        }
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(vtable) => vtable as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let pointer = match sspan(log, memory, pointer_addr, 8, STATS_MASK).and_then(|s| {
            usize::try_from(stats_pool_u64(log, s))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(pointer) => pointer,
            None => return,
        };
        let vtable = match sspan(log, memory, Some(pointer), 8, STATS_MASK) {
            Some(start) => start,
            None => return,
        };
        if stats_pool_u64(log, vtable) != country_vtable {
            return;
        }
        let id = match sspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            STATS_MASK,
        ) {
            Some(start) => stats_pool_u32(log, start),
            None => return,
        };
        // Range validation mirrors the link reader; the entry bytes are
        // already staged above, so no per-id take is needed.
        if id == 0 || id >= links_array.count {
            return;
        }
        if log.overflowed {
            return;
        }
    }
    let air_vtable = match base.checked_add(cstrategic_air::VTABLE_RVA) {
        Some(vtable) => vtable as u64,
        None => return,
    };
    for slot in 0..owners.count {
        let owner_addr = owners.data.checked_add((slot as usize).saturating_mul(8));
        let owner = match sspan(log, memory, owner_addr, 8, STATS_MASK)
            .and_then(|s| usize::try_from(stats_pool_u64(log, s)).ok())
        {
            Some(owner) => owner,
            None => {
                if log.overflowed {
                    return;
                }
                return;
            }
        };
        if owner == 0 {
            continue;
        }
        let vtable = match sspan(log, memory, Some(owner), 8, STATS_MASK) {
            Some(start) => start,
            None => return,
        };
        if stats_pool_u64(log, vtable) != air_vtable {
            return;
        }
        let owner_field = match sspan(
            log,
            memory,
            owner.checked_add(cstrategic_air::COUNTRY),
            4,
            STATS_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let _ = stats_pool_u32(log, owner_field);
        let table_header = match sspan(
            log,
            memory,
            owner.checked_add(cstrategic_air::HISTORY_TABLE),
            16,
            STATS_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let mut table_raw = [0u8; 16];
        table_raw.copy_from_slice(&log.bytes[table_header..table_header + 16]);
        let table = match Array::from_header(table_raw, 8) {
            Some(array) => array,
            None => continue,
        };
        for first in (0..table.count).step_by(64) {
            let count = (table.count - first).min(64) as usize;
            let Some(address) = table.element(first, 8) else {
                return;
            };
            let Some((start, completed)) =
                log.take_array_chunk(memory, address, 8, count, STATS_MASK)
            else {
                return;
            };
            for index in 0..completed {
                let history = match usize::try_from(stats_pool_u64(log, start + index * 8)).ok() {
                    Some(history) => history,
                    None => return,
                };
                if history != 0 && !copy_stats_history(log, memory, base, history) {
                    return;
                }
                if log.overflowed {
                    return;
                }
            }
            if completed != count {
                return;
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// Per-owner history-table walk: header already staged by the census; take
/// each slot word, then each nonnull history via `copy_stats_history`.
/// Stops at the first typed abort point (failed take), mirroring the walk's
/// fail-fast on unreadable slots.
pub(crate) fn copy_air_stats_refresh(
    memory: &impl engine::Memory,
    ctx: AirStatsCopyContext,
    first: &mut AirStatsJob,
    mut second: Option<&mut AirStatsJob>,
) -> AirStatsCopyReport {
    let mut ticks = [0u64; AIR_STATS_N];
    copy_stats_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; AIR_STATS_N];
    if let Some(second_job) = second.as_deref_mut() {
        let mut second_ticks = [0u64; AIR_STATS_N];
        copy_stats_pass(
            second_job,
            memory,
            ctx.base,
            ctx.game_state,
            &mut second_ticks,
        );
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
    AirStatsCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

pub(crate) fn decode_air_stats_payload(
    captured: &mut Captured,
    base: usize,
    job: &AirStatsJob,
    decode_ticks: &mut [u64; AIR_STATS_N],
) {
    let recorded = crate::raw::Recorded::build(&job.log);
    let started = qpc();
    // Replay the existing statistics walk against the recorded spans into
    // the pooled buffers, with the identical bounded/capacity handling the
    // game thread applies.
    let walked = cloop_history::walk(
        &recorded,
        base,
        job.game_state,
        &mut captured.statistics.rows,
    );
    captured.statistics.outcome = bounded(walked, captured.statistics.rows.len());
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    captured.stats_raw_mode = false;
}

// ===========================================================================
// Hourly shared bases/registrations/aces split (AirBases,
// AirBaseRegistrations, AirAces).
//
// The game thread copies raw bytes only: one authenticated country census,
// the strategic-air manager with its four base vectors, per-base identity
// and host/state/province fields, per-base country containers with pool and
// wing vectors, and per-country ace rosters with ace objects and name bytes.
// Shared spans (country census, manager, vectors, base identities) are taken
// ONCE with the combined mask and serve every dependent replay: Recorded
// spans are address-keyed with last-value repeat, so one census walk feeds
// the bases, registrations and aces decodes with no duplicated copy of the
// walk. No row is decoded on the game thread. The single FIFO decode worker
// replays `cair_base::walk`, `cair_base::walk_registrations` and `cace::walk`
// against the recorded spans, so rows, ordinals, skips, truncation counts
// and failure reasons are identical by construction (see `crate::raw` for
// the mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on pool overflow: validation, precedence and error
// scope are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless. Takes follow walk order and
// the pass stops at the first take failure the typed walk would also stop
// at; container resolve link reads are deduplicated per pass through a
// small seen list (the typed walk caches them in its Resolver), which is
// deterministic across passes and byte-identical within a pass.
// ===========================================================================

/// Shared hourly payload slots in calibration/report order: [bases,
/// registrations, aces]. The registrations rows live in the fog pack; the
/// decode worker fills them from this same log (see
/// `crate::extended::fog::Captured::decode_registrations_payload`).
pub(crate) const AIR_BASES_N: usize = 3;
const BASES_MASK: u32 = 0x01;
const AIR_REGISTRATIONS_MASK: u32 = 0x02;
const AIR_ACES_MASK: u32 = 0x04;
/// Country census spans gate every slot: all three typed walks authenticate
/// through the same country reader.
const AIR_BASES_CENSUS_MASK: u32 = 0x07;
/// Manager, vectors and base identities gate bases and registrations: the
/// aces walk never reads them.
const BASE_VECTORS_MASK: u32 = 0x03;

/// Batch chunking for the registration pool/wing pointer blocks. Must match
/// the engine walk's batching exactly: the typed walk issues one
/// `read_slice` per chunk, and the replay serves exact (address, length)
/// spans only.
const REG_MIRROR_BATCH: usize = 64;

/// One hour of raw shared bases payload: the country census, the manager and
/// base vectors, per-base fields, containers with pools and wings, and ace
/// rosters with objects and names. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_air_bases_job`).
#[derive(Debug, Default)]
pub(crate) struct AirBasesJob {
    pub log: RawLog<AIR_BASES_N>,
    pub game_state: usize,
    /// Canonical (pointer, slot) countries parsed from the census takes.
    census: Vec<(usize, u32)>,
    /// Link raws already taken this pass with their first successfully
    /// staged slot values. Failed resolutions are not cached.
    seen_links: Vec<(u32, u32)>,
    /// Direct-map hints into `seen_links`, reset each pass. Collisions use
    /// the authoritative list, never a second observation of live memory.
    seen_link_index: Vec<usize>,
    capacity_charge: crate::raw_memory::Charge,
}

impl AirBasesJob {
    pub fn new() -> Self {
        let seen_links = Vec::with_capacity(1024);
        let seen_link_index = vec![usize::MAX; seen_links.capacity()];
        let mut job = Self {
            // Late-campaign worst case is 396 bases with 5,148 registration
            // rows plus wing vectors and 57 aces with names (appendix
            // census); both pools start above that so steady state never
            // overflows (see the worst-case caps test).
            log: RawLog::new(1 << 17, 8 << 20),
            game_state: 0,
            census: Vec::with_capacity(1024),
            seen_links,
            seen_link_index,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
        self.seen_links.clear();
        self.seen_link_index.fill(usize::MAX);
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
        crate::raw::double_capacity(&mut self.census);
        crate::raw::double_capacity(&mut self.seen_links);
        self.seen_link_index
            .resize(self.seen_links.capacity(), usize::MAX);
        self.seen_link_index.fill(usize::MAX);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census)
                + vector_bytes(&self.seen_links)
                + vector_bytes(&self.seen_link_index),
            ..Default::default()
        });
    }
}

/// Context of one current worker invocation. Sampling already forces a full
/// refresh on a campaign/load signal, clock discontinuity or keyframe; the
/// worker additionally requires adjacent sequence/hour and the same Shared.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AirBasesMemoContext {
    pub owner: usize,
    pub base: usize,
    pub seq: u64,
    pub hour: i32,
    pub force_full: bool,
    /// Authoritative per-frame checks survive the local capture-pack reset.
    pub validation_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AirBasesMemoKey {
    owner: usize,
    base: usize,
    game_state: usize,
    // Logical slice lengths control admission and therefore observations.
    // Exact equality is deliberately more conservative than "large enough".
    capacities: [usize; 5],
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirBasesMemoReport {
    pub hit: bool,
    pub eligible: bool,
    pub compared: bool,
    pub installed: bool,
    pub compare_ticks: u64,
    pub restore_ticks: u64,
    pub install_ticks: u64,
}

// A separate tracker explicitly includes native memo outputs, which are outside
// raw_memory's generic payload/index scope. No per-frame/pool memo allocation.
static AIR_BASES_MEMO_MEMORY: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();

pub(crate) fn air_bases_memo_memory() -> serde_json::Value {
    let snapshot = AIR_BASES_MEMO_MEMORY.snapshot();
    serde_json::json!({
        "scope": "one worker-owned AirBases memo; actual input and native-output vector capacities, retained across misses",
        "input_capacity_bytes": snapshot.current.payload_vectors,
        "native_output_capacity_bytes": snapshot.current.auxiliary_vectors,
        "current_capacity_bytes": snapshot.current.payload_vectors + snapshot.current.auxiliary_vectors,
        "peak_capacity_bytes": snapshot.peak_total_lower_bound,
        "excluded": ["inline memo metadata", "allocator rounding/metadata and transient reallocation overlap"],
    })
}

/// Exactly one previous complete family result, owned independently of every
/// pooled job/frame. Every hour still captures current bytes on the game thread.
/// Equality never serves old observations to a Memory implementation: it only
/// reuses the deterministic native result of an identical complete invocation.
///
/// Full ordered Span equality (not table partitions) retains exact-key cursor,
/// last-repeat and exact-before-bulk semantics even under arbitrary aliases.
/// Rust field equality avoids comparing Span padding; only used bytes matter.
#[derive(Debug)]
pub(crate) struct AirBasesMemo {
    key: Option<AirBasesMemoKey>,
    previous: Option<(u64, i32)>,
    spans: Vec<crate::raw::Span>,
    bytes: Vec<u8>,
    has_bulk: bool,
    bases: Vec<cair_base::Row>,
    aces: Vec<cace::Row>,
    names: Vec<u8>,
    required_names: usize,
    registrations: super::fog::RegistrationsMemo,
    capacity_charge: crate::raw_memory::Charge,
}

impl Default for AirBasesMemo {
    fn default() -> Self {
        Self::on_tracker(&AIR_BASES_MEMO_MEMORY)
    }
}

impl AirBasesMemo {
    fn on_tracker(tracker: &'static crate::raw_memory::Tracker) -> Self {
        Self {
            key: None,
            previous: None,
            spans: Vec::new(),
            bytes: Vec::new(),
            has_bulk: false,
            bases: Vec::new(),
            aces: Vec::new(),
            names: Vec::new(),
            required_names: 0,
            registrations: super::fog::RegistrationsMemo::default(),
            capacity_charge: crate::raw_memory::Charge::empty_on(tracker),
        }
    }

    /// Drop validity, not reusable storage. A missed refresh/typed fallback,
    /// fault or incomplete decode may never carry an older entry forward.
    pub(crate) fn invalidate(&mut self) {
        self.key = None;
        self.previous = None;
    }

    fn capacity(&self) -> crate::raw_memory::Capacity {
        use crate::raw_memory::{Capacity, vector_bytes};
        Capacity {
            payload_vectors: vector_bytes(&self.spans) + vector_bytes(&self.bytes),
            auxiliary_vectors: vector_bytes(&self.bases)
                + vector_bytes(&self.aces)
                + vector_bytes(&self.names)
                + self.registrations.capacity_bytes(),
            ..Capacity::default()
        }
    }

    fn input_equal(&self, log: &RawLog<AIR_BASES_N>) -> bool {
        // A stored witness contains only successes. Equality of every Span's
        // ok field also rejects a failed current observation, without a second
        // explicit span scan after the decode entry's eligibility preflight.
        !log.overflowed
            && !log.fault.iter().any(|fault| *fault)
            && self.has_bulk == log.has_bulk
            && self.spans == log.spans
            && self.bytes.as_slice() == log.bytes.as_ref()
    }

    pub(crate) fn decode(
        &mut self,
        air: &mut Captured,
        fog: &mut super::fog::Captured,
        job: &AirBasesJob,
        ctx: AirBasesMemoContext,
        decode_ticks: &mut [u64; AIR_BASES_N],
    ) -> AirBasesMemoReport {
        let mut report = AirBasesMemoReport::default();
        let compared_started = qpc();
        let adjacent = self.previous.is_some_and(|(seq, hour)| {
            seq.checked_add(1) == Some(ctx.seq) && hour.checked_add(1) == Some(ctx.hour)
        });
        if ctx.force_full || !adjacent {
            self.key = None;
        }
        let capacity = air
            .bases_memo_context()
            .zip(fog.registrations_memo_context());
        let key = capacity.map(|(air, fog)| AirBasesMemoKey {
            owner: ctx.owner,
            base: ctx.base,
            game_state: job.game_state,
            capacities: [air[0], air[1], air[2], fog[0], fog[1]],
        });
        report.eligible = ctx.validation_allowed
            && key.is_some()
            && !job.log.overflowed
            && !job.log.fault.iter().any(|fault| *fault)
            && job.log.spans.iter().all(|span| span.ok);
        if report.eligible && self.key == key {
            report.compared = true;
            report.hit = self.input_equal(&job.log);
        }
        report.compare_ticks = elapsed(compared_started);
        decode_ticks[1] += report.compare_ticks;
        if report.hit {
            let restore_before = decode_ticks.iter().sum::<u64>();
            let started = qpc();
            air.bases.rows[..self.bases.len()].copy_from_slice(&self.bases);
            air.bases.outcome = Outcome::Rows {
                len: self.bases.len(),
            };
            decode_ticks[0] += elapsed(started);
            let started = qpc();
            air.aces.rows[..self.aces.len()].copy_from_slice(&self.aces);
            air.aces.outcome = Outcome::Rows {
                len: self.aces.len(),
            };
            air.names.first[..self.names.len()].copy_from_slice(&self.names);
            air.names.len = self.names.len();
            air.names.required = self.required_names;
            air.bases_raw_mode = false;
            decode_ticks[2] += elapsed(started);
            let started = qpc();
            fog.restore_registrations_memo(&self.registrations);
            decode_ticks[1] += elapsed(started);
            report.restore_ticks = decode_ticks
                .iter()
                .sum::<u64>()
                .saturating_sub(restore_before);
            self.previous = Some((ctx.seq, ctx.hour));
            return report;
        }

        self.key = None;
        // Preserve both original Recorded lifetimes: bases+aces share cursors;
        // registrations starts a separate reader over the same current log.
        air.decode_air_bases_payload(ctx.base, job, decode_ticks);
        fog.decode_registrations_payload(ctx.base, job, decode_ticks);
        // Range validation is memo work too, even when it rejects an install.
        let install_started = qpc();
        if report.eligible && air.bases_memo_complete() && fog.registrations_memo_complete() {
            self.spans.clear();
            self.spans.extend_from_slice(&job.log.spans);
            self.bytes.clear();
            self.bytes.extend_from_slice(job.log.bytes.as_ref());
            self.has_bulk = job.log.has_bulk;
            self.bases.clear();
            self.bases
                .extend_from_slice(&air.bases.rows[..air.bases.outcome.len()]);
            self.aces.clear();
            self.aces
                .extend_from_slice(&air.aces.rows[..air.aces.outcome.len()]);
            self.names.clear();
            self.names
                .extend_from_slice(&air.names.first[..air.names.len]);
            self.required_names = air.names.required;
            fog.save_registrations_memo(&mut self.registrations);
            self.capacity_charge.set(self.capacity());
            self.key = key;
            self.previous = Some((ctx.seq, ctx.hour));
            report.installed = true;
        } else {
            self.invalidate();
        }
        report.install_ticks = elapsed(install_started);
        decode_ticks[1] += report.install_ticks;
        report
    }
}

impl Captured {
    fn bases_memo_context(&self) -> Option<[usize; 3]> {
        (self.bases_raw_mode
            && !self.bases.checked
            && !self.aces.checked
            && self.bases.mismatches == 0
            && self.aces.mismatches == 0)
            .then_some([
                self.bases.rows.len(),
                self.aces.rows.len(),
                self.names.first.len(),
            ])
    }

    fn bases_memo_complete(&self) -> bool {
        matches!(self.bases.outcome, Outcome::Rows { len } if len <= self.bases.rows.len())
            && matches!(self.aces.outcome, Outcome::Rows { .. })
            && self.names.required >= self.names.len
            && valid_names(
                self.aces.outcome,
                &self.aces.rows,
                &self.names.first,
                self.names.len,
            )
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirBasesCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walks (identical rows through the existing paths) and enqueues no
    /// payload.
    pub raw_ok: bool,
    /// Per-partition raw equality gated by live-fault conservatism, in
    /// [bases, registrations, aces] order. Shared census spans gate every
    /// dependent partition; manager/vector spans gate bases and
    /// registrations; private spans gate only their own slot.
    pub matched: [bool; AIR_BASES_N],
    pub copy_ticks: [u64; AIR_BASES_N],
    pub raw_bytes: [u64; AIR_BASES_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirBasesCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; AIR_BASES_N],
}

fn bases_pool_u64(log: &RawLog<AIR_BASES_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn bases_pool_u32(log: &RawLog<AIR_BASES_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn bspan(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one link-table read with local bounds checks. Returns `None`
/// exactly when the typed resolve fails; the replay derives its own reason.
fn bases_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_BASES_CENSUS_MASK,
    )?;
    let slot = bases_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Authenticated country census shared by the bases, registrations and aces
/// walks. Records the exact read set of `ccountry::Reader::new` plus one full
/// `read_at` per slot, taking every discoverable span: validation mismatches
/// never stop the census because each replay derives its own verdict from the
/// same spans (a mismatch fatal to one walk may be unread by another).
/// Returns the parsed link array with its country count for the resolve
/// mirror, or `None` only when the headers themselves are undiscoverable.
fn copy_air_bases_census(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<(usize, u32)>,
) -> Option<(Array, u32)> {
    let countries_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        AIR_BASES_CENSUS_MASK,
    )?;
    let links_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        AIR_BASES_CENSUS_MASK,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let Some(pointer_start) = bspan(log, memory, pointer_addr, 8, AIR_BASES_CENSUS_MASK) else {
            continue;
        };
        let pointer = match usize::try_from(bases_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = bspan(log, memory, Some(pointer), 8, AIR_BASES_CENSUS_MASK);
        let id_start = bspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            AIR_BASES_CENSUS_MASK,
        );
        let (Some(vtable_start), Some(id_start)) = (vtable_start, id_start) else {
            continue;
        };
        if bases_pool_u64(log, vtable_start) != vtable {
            continue;
        }
        let id = bases_pool_u32(log, id_start);
        let canonical = bases_link(log, memory, &links, countries.count, id);
        let slot_link = bases_link(log, memory, &links, countries.count, slot);
        let (Some(canonical), Some(slot_link)) = (canonical, slot_link) else {
            continue;
        };
        if slot_link != canonical {
            continue;
        }
        let check = bases_link(log, memory, &links, countries.count, canonical);
        if check != Some(canonical) {
            continue;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = bspan(log, memory, canonical_addr, 8, AIR_BASES_CENSUS_MASK);
        let Some(canonical_pointer) = canonical_pointer else {
            continue;
        };
        if bases_pool_u64(log, canonical_pointer) != pointer as u64 {
            continue;
        }
        if slot != canonical {
            continue;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push((pointer, slot));
    }
    Some((links, countries.count))
}

/// Deduplicated resolve-link take returning the first successfully staged
/// slot value for a raw in this pass. Index collisions still consult that
/// authoritative observation; failed resolutions remain uncached.
fn resolve_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    links: &Array,
    seen: &mut Vec<(u32, u32)>,
    index: &mut [usize],
    countries: u32,
    raw: u32,
) -> Option<u32> {
    let bucket = (!index.is_empty()).then(|| raw as usize % index.len());
    if let Some(bucket) = bucket
        && let Some(&(key, slot)) = seen.get(index[bucket])
        && key == raw
    {
        return Some(slot);
    }
    if let Some(position) = seen.iter().position(|(key, _)| *key == raw) {
        if let Some(bucket) = bucket {
            index[bucket] = position;
        }
        return Some(seen[position].1);
    }
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_REGISTRATIONS_MASK,
    )?;
    let slot = bases_pool_u32(log, start);
    if slot >= countries {
        return None;
    }
    if seen.len() == seen.capacity() {
        log.overflowed = true;
        return None;
    }
    let position = seen.len();
    seen.push((raw, slot));
    if let Some(bucket) = bucket {
        index[bucket] = position;
    }
    Some(slot)
}

/// Read one native word back out of a recorded pointer-block take (`chunk`
/// words staged at pool offset `start`). The typed walk reads each block
/// with one `read_slice`; the mirror takes the same span and parses the
/// staged bytes locally, never re-reading live memory and never allocating.
fn block_word(log: &RawLog<AIR_BASES_N>, start: usize, index: usize) -> Option<usize> {
    let at = start.checked_add(index.checked_mul(8)?)?;
    let bytes = log.bytes.get(at..at.checked_add(8)?)?;
    usize::try_from(u64::from_le_bytes(bytes.try_into().ok()?)).ok()
}

fn copy_air_base_row(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
    state_slot: Option<u32>,
) -> bool {
    // The 16-byte identity is taken once per base by the shared vector loop
    // (both walks read it); only the bases-private host/state/province
    // fields are taken here.
    let host = match bspan(
        log,
        memory,
        pointer.checked_add(cair_base::HOST_REFERENCE),
        32,
        BASES_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let state_pointer = match usize::try_from(bases_pool_u64(log, host + 8)).ok() {
        Some(pointer) => pointer,
        None => return false,
    };
    let province_pointer = match usize::try_from(bases_pool_u64(log, host + 16)).ok() {
        Some(pointer) => pointer,
        None => return false,
    };
    let state = if state_pointer == 0 {
        None
    } else {
        let vtable = match bspan(log, memory, Some(state_pointer), 8, BASES_MASK) {
            Some(start) => start,
            None => return false,
        };
        let expected_state = match base.checked_add(cstate::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return false,
        };
        if bases_pool_u64(log, vtable) != expected_state {
            return false;
        }
        let id = match bspan(
            log,
            memory,
            state_pointer.checked_add(cstate::ID),
            4,
            BASES_MASK,
        ) {
            Some(start) => bases_pool_u32(log, start),
            None => return false,
        };
        Some(id)
    };
    if state != state_slot {
        return false;
    }
    let host_ref = bases_pool_u64(log, host);
    if state.is_none() && host_ref == 0 {
        return false;
    }
    if province_pointer != 0 {
        let vtable = match bspan(log, memory, Some(province_pointer), 8, BASES_MASK) {
            Some(start) => start,
            None => return false,
        };
        let expected_province = match base.checked_add(cprovince::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return false,
        };
        if bases_pool_u64(log, vtable) != expected_province {
            return false;
        }
        let owner = match bspan(
            log,
            memory,
            province_pointer.checked_add(cprovince::STATE),
            8,
            BASES_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        if state_pointer != 0 && bases_pool_u64(log, owner) != state_pointer as u64 {
            return false;
        }
        if bspan(
            log,
            memory,
            province_pointer.checked_add(cprovince::ID),
            4,
            BASES_MASK,
        )
        .is_none()
        {
            return false;
        }
    }
    true
}

/// Mirror of one country's ace roster and objects. Takes every
/// discoverable span; validation mismatches stop only this country's takes
/// (the typed walk fails there). Returns false exactly when the typed walk
/// fails at or before this country.
fn copy_air_aces_country(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    let header = match bspan(
        log,
        memory,
        pointer.checked_add(ccountry::ACES),
        16,
        AIR_ACES_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let roster = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    for ordinal in 0..roster.count {
        if !copy_air_ace(log, memory, base, pointer, &roster, ordinal) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of one ace object with its three name strings. The seven scattered
/// fields are taken as seven exact spans in walk order; heap names take the
/// terminator byte plus the content bytes, inline names take nothing further.
/// Returns false exactly when the typed row read fails for this ace.
fn copy_air_ace(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    country_pointer: usize,
    roster: &Array,
    ordinal: u32,
) -> bool {
    let addr = roster.element(ordinal, 8);
    let entry = match bspan(log, memory, addr, 8, AIR_ACES_MASK) {
        Some(start) => start,
        None => return false,
    };
    let pointer = match usize::try_from(bases_pool_u64(log, entry))
        .ok()
        .filter(|&pointer| pointer != 0)
    {
        Some(pointer) => pointer,
        None => return false,
    };
    // Seven scattered fields in walk order: identity, mask, owner/wing,
    // the 96-byte three-string header block, alive, kill type, handled.
    let identity = bspan(log, memory, Some(pointer), 16, AIR_ACES_MASK);
    let mask = bspan(
        log,
        memory,
        pointer.checked_add(cace::AIRCRAFT_TYPE_MASK),
        4,
        AIR_ACES_MASK,
    );
    let owner = bspan(
        log,
        memory,
        pointer.checked_add(cace::OWNER_COUNTRY),
        16,
        AIR_ACES_MASK,
    );
    let strings = bspan(
        log,
        memory,
        pointer.checked_add(cace::NAME),
        96,
        AIR_ACES_MASK,
    );
    let alive = bspan(
        log,
        memory,
        pointer.checked_add(cace::ALIVE),
        1,
        AIR_ACES_MASK,
    );
    let kill_type = bspan(
        log,
        memory,
        pointer.checked_add(cace::KILL_TYPE),
        4,
        AIR_ACES_MASK,
    );
    let handled = bspan(
        log,
        memory,
        pointer.checked_add(cace::HANDLED),
        1,
        AIR_ACES_MASK,
    );
    let (Some(identity), Some(_), Some(owner), Some(strings), Some(alive), Some(_), Some(handled)) =
        (identity, mask, owner, strings, alive, kill_type, handled)
    else {
        return false;
    };
    let expected = match base.checked_add(cace::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if bases_pool_u64(log, identity) != expected
        || bases_pool_u64(log, identity + 8) == 0
        || bases_pool_u64(log, owner) != country_pointer as u64
        || log.bytes[alive] > 1
        || log.bytes[handled] > 1
    {
        return false;
    }
    if log.bytes[alive] == 0
        && bspan(
            log,
            memory,
            pointer.checked_add(cace::KILLER_ACE_REFERENCE),
            12,
            AIR_ACES_MASK,
        )
        .is_none()
    {
        return false;
    }
    for index in 0..3 {
        let head = strings + index * 32;
        if !copy_ace_name(log, memory, head) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of one ace name string: header validation is a local parse, heap
/// names take the terminator byte plus the content bytes. Returns false
/// exactly when the typed name read fails.
fn copy_ace_name(log: &mut RawLog<AIR_BASES_N>, memory: &impl engine::Memory, head: usize) -> bool {
    let header = &log.bytes[head..head + 32];
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(
        header[msvc_string::CAPACITY..msvc_string::CAPACITY + 8]
            .try_into()
            .unwrap(),
    );
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return false;
    }
    let len = match usize::try_from(len) {
        Ok(len) => len,
        Err(_) => return false,
    };
    if capacity == msvc_string::INLINE_CAPACITY {
        if header.get(len) != Some(&0) {
            return false;
        }
        return true;
    }
    let source = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::DATA..msvc_string::DATA + 8]
            .try_into()
            .unwrap(),
    ))
    .ok()
    .filter(|&source| source != 0)
    {
        Some(source) => source,
        None => return false,
    };
    if source.checked_add(len).is_none() {
        return false;
    }
    // Terminator first, then content: the walk's read order.
    if bspan(log, memory, source.checked_add(len), 1, AIR_ACES_MASK).is_none() {
        return false;
    }
    if len == 0 {
        return true;
    }
    if bspan(log, memory, Some(source), len, AIR_ACES_MASK).is_none() {
        return false;
    }
    true
}

/// Mirror of one base's `registration` subtree: containers, pools and wings
/// with the engine's 64-row batch chunking. Stops at the first typed abort
/// point, mirroring the walk's fail-fast; null containers are measured
/// absence. Returns false exactly when the typed walk fails for this base.
#[allow(clippy::too_many_arguments)]
fn copy_air_base_registrations(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    base_handle: u64,
    pointer: usize,
    links: &Array,
    countries: u32,
    seen: &mut Vec<(u32, u32)>,
    seen_index: &mut [usize],
) -> bool {
    let _ = base_handle;
    let header = match bspan(
        log,
        memory,
        pointer.checked_add(cair_base::COUNTRY_CONTAINERS),
        16,
        AIR_REGISTRATIONS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let containers = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    let expected_pool = match base.checked_add(cair_wing_pool::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_primary = match base.checked_add(engine::cair_wing::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_secondary = match base.checked_add(engine::cair_wing::SECONDARY_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_container = match base.checked_add(ccountry_air_container::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let mut container_chunk = (0usize, 0usize);
    for slot in 0..containers.count {
        let offset = slot as usize % REG_MIRROR_BATCH;
        if offset == 0 {
            let count = ((containers.count - slot) as usize).min(REG_MIRROR_BATCH);
            let Some(address) = containers.element(slot, 8) else {
                return false;
            };
            let Some(chunk) =
                log.take_array_chunk(memory, address, 8, count, AIR_REGISTRATIONS_MASK)
            else {
                return false;
            };
            container_chunk = chunk;
        }
        if offset >= container_chunk.1 {
            // The first failed pointer is already recorded exactly. Earlier
            // complete slots retain their native validation/row order.
            return false;
        }
        let container_start = container_chunk.0 + offset * 8;
        let container = match usize::try_from(bases_pool_u64(log, container_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(container) => container,
            None => continue,
        };
        let vtable = match bspan(log, memory, Some(container), 8, AIR_REGISTRATIONS_MASK) {
            Some(start) => start,
            None => return false,
        };
        if bases_pool_u64(log, vtable) != expected_container {
            return false;
        }
        let country = match bspan(
            log,
            memory,
            container.checked_add(ccountry_air_container::COUNTRY),
            4,
            AIR_REGISTRATIONS_MASK,
        ) {
            Some(start) => bases_pool_u32(log, start),
            None => return false,
        };
        // Slot/country agreement mirrors the walk's resolver path exactly:
        // the reserved slot accepts only zero or unlinked ids (a readable
        // zero entry is absence; unreadable entries stay faults), any other
        // slot requires its resolved canonical slot. Link reads are
        // deduplicated per pass like the walk's Resolver cache.
        if slot == 0 {
            if country != 0 {
                match resolve_link(log, memory, links, seen, seen_index, countries, country) {
                    Some(0) => {}
                    _ => return false,
                }
                if log.overflowed {
                    return false;
                }
            }
        } else {
            match resolve_link(log, memory, links, seen, seen_index, countries, country) {
                Some(canonical) if canonical == slot => {}
                _ => return false,
            }
            if log.overflowed {
                return false;
            }
        }
        let pools_header = match bspan(
            log,
            memory,
            container.checked_add(ccountry_air_container::POOLS),
            16,
            AIR_REGISTRATIONS_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        raw.copy_from_slice(&log.bytes[pools_header..pools_header + 16]);
        let pools = match Array::from_header(raw, 8) {
            Some(array) => array,
            None => return false,
        };
        let mut pool_first = 0u32;
        while pool_first < pools.count {
            let chunk = ((pools.count - pool_first) as usize).min(REG_MIRROR_BATCH);
            let first_address = pools.element(pool_first, 8);
            let block = match bspan(
                log,
                memory,
                first_address,
                chunk * 8,
                AIR_REGISTRATIONS_MASK,
            ) {
                Some(start) => start,
                None => return false,
            };
            for index in 0..chunk {
                let pointer = match block_word(log, block, index) {
                    Some(pointer) => pointer,
                    None => return false,
                };
                // A null pool pointer inside a counted vector refuses the
                // table, exactly like the typed walk.
                if pointer == 0 {
                    return false;
                }
                // `copy_records` POOL_FIELDS: vtable word plus wing-vector
                // header as two exact spans, in field order.
                let pool_vtable = match bspan(log, memory, Some(pointer), 8, AIR_REGISTRATIONS_MASK)
                {
                    Some(start) => start,
                    None => return false,
                };
                let wings = match bspan(
                    log,
                    memory,
                    pointer.checked_add(engine::cair_wing_pool::WINGS),
                    16,
                    AIR_REGISTRATIONS_MASK,
                ) {
                    Some(start) => start,
                    None => return false,
                };
                if bases_pool_u64(log, pool_vtable) != expected_pool {
                    return false;
                }
                let mut header = [0u8; 16];
                header.copy_from_slice(&log.bytes[wings..wings + 16]);
                let members = match Array::from_header(header, 8) {
                    Some(array) => array,
                    None => return false,
                };
                let mut wing_first = 0u32;
                while wing_first < members.count {
                    let wing_chunk = ((members.count - wing_first) as usize).min(REG_MIRROR_BATCH);
                    let wing_address = members.element(wing_first, 8);
                    let wing_block = match bspan(
                        log,
                        memory,
                        wing_address,
                        wing_chunk * 8,
                        AIR_REGISTRATIONS_MASK,
                    ) {
                        Some(start) => start,
                        None => return false,
                    };
                    for wing_index in 0..wing_chunk {
                        let wing = match block_word(log, wing_block, wing_index) {
                            Some(wing) => wing,
                            None => return false,
                        };
                        if wing == 0 {
                            return false;
                        }
                        // `copy_records` WING_FIELDS: the 32-byte identity
                        // span as one exact span.
                        let identity =
                            match bspan(log, memory, Some(wing), 32, AIR_REGISTRATIONS_MASK) {
                                Some(start) => start,
                                None => return false,
                            };
                        if bases_pool_u64(log, identity) != expected_primary
                            || bases_pool_u64(log, identity + 16) != expected_secondary
                            || bases_pool_u64(log, identity + 24) == 0
                        {
                            return false;
                        }
                    }
                    if log.overflowed {
                        return false;
                    }
                    wing_first += wing_chunk as u32;
                }
            }
            if log.overflowed {
                return false;
            }
            pool_first += chunk as u32;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// One structural copy pass: shared census, manager with base vectors,
/// per-base fields and registrations, then ace rosters. Phases are
/// independent: a validation or take failure stops only the takes belonging
/// to walks that would stop there (tracked with alive flags), so every
/// replay derives its own verdict from the recorded spans. The aces phase
/// runs last on the shared census takes. The job is cleared on entry so
/// pooled refreshes never accumulate spans across hours.
#[allow(clippy::too_many_lines)]
fn copy_air_bases_pass(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let started = qpc();
    job.clear();
    job.game_state = game_state;
    let census = copy_air_bases_census(&mut job.log, memory, base, game_state, &mut job.census);
    ticks[0] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let (links, countries) = match census {
        Some(census) => census,
        None => {
            // Census headers undiscoverable: the manager/base phases are
            // independent and continue (their replays fail at the same
            // headers); resolve and roster takes are skipped (their replays
            // fail at the headers too).
            copy_air_bases_vectors(job, memory, base, None, ticks);
            return;
        }
    };
    copy_air_bases_vectors(job, memory, base, Some((&links, countries)), ticks);
    if job.log.overflowed {
        return;
    }
    // Aces phase: independent of the base phases, served by the shared
    // census takes plus roster/object/name takes below. Index iteration
    // avoids cloning the census on the game thread.
    let aces_started = qpc();
    let mut ace_index = 0;
    while ace_index < job.census.len() {
        let (pointer, _) = job.census[ace_index];
        ace_index += 1;
        if !copy_air_aces_country(&mut job.log, memory, base, pointer) {
            break;
        }
        if job.log.overflowed {
            break;
        }
    }
    ticks[2] += elapsed(aces_started);
}

/// Manager, vectors and per-base takes with per-walk alive flags. Shared
/// spans (manager, headers, element pointers, identities) stop both walks on
/// failure; private spans stop only their own walk while the other continues,
/// mirroring each typed walk's fail-fast scope.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn copy_air_bases_vectors(
    job: &mut AirBasesJob,
    memory: &impl engine::Memory,
    base: usize,
    census: Option<(&Array, u32)>,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let vectors_started = qpc();
    let manager = match bspan(
        &mut job.log,
        memory,
        job.game_state
            .checked_add(cgame_state::STRATEGIC_AIR_MANAGER),
        8,
        BASE_VECTORS_MASK,
    ) {
        Some(start) => match usize::try_from(bases_pool_u64(&job.log, start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(manager) => manager,
            None => {
                ticks[0] += elapsed(vectors_started);
                return;
            }
        },
        None => {
            ticks[0] += elapsed(vectors_started);
            return;
        }
    };
    let vtable = bspan(&mut job.log, memory, Some(manager), 8, BASE_VECTORS_MASK);
    let expected_manager = base.checked_add(cstrategic_air_manager::VTABLE_RVA);
    let manager_ok = match (vtable, expected_manager) {
        (Some(start), Some(expected)) => bases_pool_u64(&job.log, start) == expected as u64,
        _ => false,
    };
    // Four base vectors in walk order, with the walk's indexed flags.
    let mut headers = [(0usize, 0u32, true); 4];
    let mut headers_ok = manager_ok;
    for (slot, (offset, indexed)) in [
        (cstrategic_air_manager::STATE_AIR_BASES, true),
        (cstrategic_air_manager::STATE_ROCKET_BASES, true),
        (cstrategic_air_manager::STATE_GUN_BASES, true),
        (cstrategic_air_manager::STATELESS_AIR_BASES, false),
    ]
    .into_iter()
    .enumerate()
    {
        let header = bspan(
            &mut job.log,
            memory,
            manager.checked_add(offset),
            16,
            BASE_VECTORS_MASK,
        );
        match header {
            Some(start) => {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&job.log.bytes[start..start + 16]);
                match Array::from_header(raw, 8) {
                    Some(array) => {
                        headers[slot] = (start, array.count, indexed);
                        let _ = array;
                    }
                    None => headers_ok = false,
                }
            }
            None => headers_ok = false,
        }
        if job.log.overflowed {
            ticks[0] += elapsed(vectors_started);
            return;
        }
    }
    let mut bases_alive = headers_ok;
    let mut reg_alive = headers_ok && census.is_some();
    let mut bases_ticks = 0u64;
    let mut reg_ticks = 0u64;
    for (header_start, count, indexed) in headers {
        if !bases_alive && !reg_alive {
            break;
        }
        // Re-parse the recorded header (already validated above when
        // headers_ok; re-parse identically when headers failed so the
        // failure scope stays per-walk below).
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&job.log.bytes[header_start..header_start + 16]);
        let Some(array) = Array::from_header(raw, 8) else {
            // Both typed walks fail on this header; later vectors are unread
            // by either replay.
            break;
        };
        let _ = count;
        for index in 0..array.count {
            if !bases_alive && !reg_alive {
                break;
            }
            let addr = (index as usize)
                .checked_mul(8)
                .and_then(|offset| array.data.checked_add(offset));
            let element = bspan(&mut job.log, memory, addr, 8, BASE_VECTORS_MASK);
            let Some(element) = element else {
                bases_alive = false;
                reg_alive = false;
                break;
            };
            let pointer = match usize::try_from(bases_pool_u64(&job.log, element))
                .ok()
                .filter(|&pointer| pointer != 0)
            {
                Some(pointer) => pointer,
                None => {
                    if indexed {
                        continue;
                    }
                    bases_alive = false;
                    reg_alive = false;
                    break;
                }
            };
            if bases_alive {
                let begin = qpc();
                // Shared identity take (both walks read it); a failure or a
                // failed identity stops both walks, mirroring both typed
                // walks. Bases-private fields follow on the bases path only.
                let identity = bspan(&mut job.log, memory, Some(pointer), 16, BASE_VECTORS_MASK);
                let expected = base.checked_add(cair_base::VTABLE_RVA);
                let (identity_ok, handle) = match (identity, expected) {
                    (Some(start), Some(expected)) => (
                        bases_pool_u64(&job.log, start) == expected as u64
                            && bases_pool_u64(&job.log, start + 8) != 0,
                        bases_pool_u64(&job.log, start + 8),
                    ),
                    _ => (false, 0),
                };
                if !identity_ok {
                    bases_alive = false;
                    reg_alive = false;
                    bases_ticks += elapsed(begin);
                    if job.log.overflowed {
                        break;
                    }
                    // Both typed walks fail at this base; later bases are
                    // unread by either replay.
                    break;
                }
                if !copy_air_base_row(
                    &mut job.log,
                    memory,
                    base,
                    pointer,
                    indexed.then_some(index),
                ) {
                    bases_alive = false;
                }
                bases_ticks += elapsed(begin);
                if reg_alive {
                    let begin = qpc();
                    let (links, countries) = census.expect("reg takes need census links");
                    if !copy_air_base_registrations(
                        &mut job.log,
                        memory,
                        base,
                        handle,
                        pointer,
                        links,
                        countries,
                        &mut job.seen_links,
                        &mut job.seen_link_index,
                    ) {
                        reg_alive = false;
                    }
                    reg_ticks += elapsed(begin);
                }
            } else if reg_alive {
                // The bases walk already stopped: the registrations walk
                // still needs the shared identity take plus its own takes.
                let begin = qpc();
                let identity = bspan(&mut job.log, memory, Some(pointer), 16, BASE_VECTORS_MASK);
                let expected = base.checked_add(cair_base::VTABLE_RVA);
                let (identity_ok, handle) = match (identity, expected) {
                    (Some(start), Some(expected)) => (
                        bases_pool_u64(&job.log, start) == expected as u64
                            && bases_pool_u64(&job.log, start + 8) != 0,
                        bases_pool_u64(&job.log, start + 8),
                    ),
                    _ => (false, 0),
                };
                if !identity_ok {
                    reg_alive = false;
                    reg_ticks += elapsed(begin);
                    break;
                }
                let (links, countries) = census.expect("reg takes need census links");
                if !copy_air_base_registrations(
                    &mut job.log,
                    memory,
                    base,
                    handle,
                    pointer,
                    links,
                    countries,
                    &mut job.seen_links,
                    &mut job.seen_link_index,
                ) {
                    reg_alive = false;
                }
                reg_ticks += elapsed(begin);
            }
            if job.log.overflowed {
                break;
            }
        }
        if job.log.overflowed {
            break;
        }
    }
    ticks[0] += elapsed(vectors_started)
        .saturating_sub(bases_ticks)
        .saturating_sub(reg_ticks);
    ticks[0] += bases_ticks;
    ticks[1] += reg_ticks;
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any rest table still needs double-read checks, compares
    /// the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walks and enqueues nothing.
    pub(crate) fn copy_air_bases_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: AirBasesCopyContext,
        fog: &mut super::fog::Captured,
        first: &mut AirBasesJob,
        mut second: Option<&mut AirBasesJob>,
    ) -> AirBasesCopyReport {
        let mut ticks = [0u64; AIR_BASES_N];
        copy_air_bases_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
        let mut matched = [true; AIR_BASES_N];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; AIR_BASES_N];
            copy_air_bases_pass(
                second_job,
                memory,
                ctx.base,
                ctx.game_state,
                &mut second_ticks,
            );
            for (total, extra) in ticks.iter_mut().zip(second_ticks) {
                *total += extra;
            }
            // Per-table partitions: the shared census gates every slot, the
            // manager/vector spans gate bases and registrations, private
            // spans only their own table.
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed paths.
        self.bases.checked = ctx.needed[0];
        self.bases.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.aces.checked = ctx.needed[2];
        self.aces.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        fog.note_registrations_copy(ctx.needed[1], matched[1]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
        AirBasesCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw shared bases payload: replays
    /// the bases and aces walks against the recorded spans into the pooled
    /// buffers (the fog registrations half runs from the same log through
    /// `super::fog::Captured::decode_registrations_payload`). Installs
    /// identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_air_bases_payload(
        &mut self,
        base: usize,
        job: &AirBasesJob,
        decode_ticks: &mut [u64; AIR_BASES_N],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let walked = cair_base::walk(&recorded, base, job.game_state, &mut self.bases.rows);
        self.bases.outcome = bounded(walked, self.bases.rows.len());
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let walked = cace::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.aces.rows,
            &mut self.names.first,
        );
        self.aces.outcome = names_outcome(walked, &self.aces.rows, &self.names.first);
        self.names.len = walked.bytes_len;
        self.names.required = walked.required_bytes;
        decode_ticks[2] += elapsed(started);
        self.bases_raw_mode = false;
    }
}

// ===========================================================================
// Hourly shared history/damage split (AirRegionHistory, AirRegionDamage).
//
// The game thread copies raw bytes only: the country census, the
// strategic-air manager with its country roster, per-owner region-combat
// vectors, per-root identity with both record-vector headers, and every
// record with its two damage vectors. Record spans are shared by both
// tables (a record fault fails both walks, exactly like the typed walk);
// damage-only spans gate the damage slot alone. No row is decoded on the
// game thread. The single FIFO decode worker replays
// `cair_region_combat_data::walk_world` against the recorded spans, so rows,
// ordinals, skips, truncation counts and failure reasons are identical by
// construction (see `crate::raw` for the mirror rules).
//
// Batched gathers decompose into per-element takes in walk order (a
// `copy_records` block becomes one take per field, a `read_slice` block one
// take per chunk), so a faulted batch serves its recorded prefix and the
// replay derives the walk's own error at the same site. Pooled capacities
// are ignored while copying: a superset is recorded and the replay
// truncates identically against the real pooled buffers.
// ===========================================================================

/// Shared hourly payload slots in calibration/report order: [history,
/// damage]. The single regional walk fills both row sets at once.
pub(crate) const AIR_HISTORY_N: usize = 2;
const AIR_DAMAGE_MASK: u32 = 0x02;
/// Census, manager, region and record spans gate both slots: a fault there
/// fails the whole regional walk, exactly like the typed walk.
const AIR_HISTORY_CENSUS_MASK: u32 = 0x03;

/// Batch chunking for the region/root pointer gathers. Must match the engine
/// walk's batching exactly; takes decompose per element in walk order.
const HISTORY_MIRROR_BATCH: usize = 64;

/// One hour of raw shared history payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_air_history_job`).
#[derive(Debug, Default)]
pub(crate) struct AirHistoryJob {
    pub log: RawLog<AIR_HISTORY_N>,
    pub game_state: usize,
}

impl AirHistoryJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 43 history groups with 34 damage
            // groups (appendix census); both pools start above that so
            // steady state never overflows (see the worst-case caps test).
            log: RawLog::new(1 << 15, 2 << 20),
            game_state: 0,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirHistoryCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    /// Per-partition raw equality gated by live-fault conservatism, in
    /// [history, damage] order. Shared spans gate both slots; damage-only
    /// spans gate only the damage slot.
    pub matched: [bool; AIR_HISTORY_N],
    pub copy_ticks: [u64; AIR_HISTORY_N],
    pub raw_bytes: [u64; AIR_HISTORY_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirHistoryCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; AIR_HISTORY_N],
}

fn history_pool_u64(log: &RawLog<AIR_HISTORY_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn hspan(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one link-table read with local bounds checks. Returns `None`
/// exactly when the typed resolve fails; the replay derives its own reason.
fn history_link(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = hspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_HISTORY_CENSUS_MASK,
    )?;
    let slot = u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Mirror of one `ccountry::Reader` resolution for an owner country raw.
/// Takes the link entry plus the full `read_at` span set, returning the
/// canonical slot exactly when the typed resolve succeeds.
fn history_resolve(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    base: usize,
    links: &Array,
    countries_array: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    let offset = if raw == 0 || raw >= links.count {
        return None;
    } else {
        (raw as usize).checked_mul(4)?
    };
    let start = hspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_HISTORY_CENSUS_MASK,
    )?;
    let canonical = u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
    if canonical == 0 || canonical >= countries {
        return None;
    }
    let pointer_addr = (canonical as usize)
        .checked_mul(8)
        .and_then(|offset| countries_array.data.checked_add(offset));
    let pointer_start = hspan(log, memory, pointer_addr, 8, AIR_HISTORY_CENSUS_MASK)?;
    let pointer = usize::try_from(history_pool_u64(log, pointer_start))
        .ok()
        .filter(|&pointer| pointer != 0)?;
    let vtable_start = hspan(log, memory, Some(pointer), 8, AIR_HISTORY_CENSUS_MASK)?;
    let id_start = hspan(
        log,
        memory,
        pointer.checked_add(ccountry::ID),
        4,
        AIR_HISTORY_CENSUS_MASK,
    )?;
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    if history_pool_u64(log, vtable_start) != vtable {
        return None;
    }
    let id = u32::from_le_bytes(log.bytes[id_start..id_start + 4].try_into().unwrap());
    if history_link(log, memory, links, countries, id)? != canonical {
        return None;
    }
    if history_link(log, memory, links, countries, canonical)? != canonical {
        return None;
    }
    let canonical_addr = (canonical as usize)
        .checked_mul(8)
        .and_then(|offset| countries_array.data.checked_add(offset));
    let canonical_pointer = hspan(log, memory, canonical_addr, 8, AIR_HISTORY_CENSUS_MASK)?;
    if history_pool_u64(log, canonical_pointer) != pointer as u64 {
        return None;
    }
    Some(canonical)
}

/// Mirror of one strategic-air owner's region-combat takes: identity,
/// owner/country agreement, region roster, every region root with its
/// records and damage vectors. Damage-scoped faults set `damage_dead`
/// (records continue, mirroring the walk); record-scoped faults stop the
/// whole pass (the typed walk fails both tables). Returns false exactly
/// when the typed walk fails at or before this owner.
#[allow(clippy::too_many_arguments)]
fn copy_history_owner(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    base: usize,
    links: &Array,
    countries_array: &Array,
    countries: u32,
    owner: usize,
    slot: u32,
    country: u32,
    damage_dead: &mut bool,
) -> bool {
    // Identity: vtable word plus country word (IDENTITY_FIELDS).
    if hspan(log, memory, Some(owner), 8, AIR_HISTORY_CENSUS_MASK).is_none() {
        return false;
    }
    let country_start = match hspan(
        log,
        memory,
        owner.checked_add(cstrategic_air::COUNTRY),
        4,
        AIR_HISTORY_CENSUS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let _ = country_start;
    // Owner/country agreement mirrors the visit check; the resolve takes
    // serve the replay identically.
    match history_resolve(
        log,
        memory,
        base,
        links,
        countries_array,
        countries,
        country,
    ) {
        Some(canonical) if canonical == slot => {}
        _ => return false,
    }
    let regions_header = match hspan(
        log,
        memory,
        owner.checked_add(cstrategic_air::REGION_COMBAT),
        16,
        AIR_HISTORY_CENSUS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[regions_header..regions_header + 16]);
    let regions = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    let mut first = 0u32;
    while first < regions.count {
        let chunk = ((regions.count - first) as usize).min(HISTORY_MIRROR_BATCH);
        // One guarded gather, one compact span for the readable prefix.
        let Some(address) = regions.element(first, 8) else {
            return false;
        };
        let Some((start, taken)) =
            log.take_array_chunk(memory, address, 8, chunk, AIR_HISTORY_CENSUS_MASK)
        else {
            return false;
        };
        // Live-compact the recorded prefix exactly like the walk.
        for index in 0..taken {
            let word = history_pool_u64(log, start + index * 8);
            let address = match usize::try_from(word).ok().filter(|&address| address != 0) {
                Some(address) => address,
                None => continue,
            };
            if !copy_history_root(log, memory, base, address, damage_dead) {
                return false;
            }
            if log.overflowed {
                return false;
            }
        }
        if taken != chunk || log.overflowed {
            // The walk processes the recorded prefix, then fails at the
            // faulted element with its own reason; the replay derives it
            // from the same spans.
            return false;
        }
        first += chunk as u32;
    }
    true
}

/// Mirror of one region root: identity, owner and both record vectors with
/// every record and its damage entries. Record faults return false (both
/// tables fail); damage faults set `damage_dead` and skip further damage
/// takes while records continue.
fn copy_history_root(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    base: usize,
    address: usize,
    damage_dead: &mut bool,
) -> bool {
    // ROOT_FIELDS in field order: vtable, owner, first header, second header.
    let vtable = match hspan(log, memory, Some(address), 8, AIR_HISTORY_CENSUS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let expected = match base.checked_add(cair_region_combat_data::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if history_pool_u64(log, vtable) != expected {
        return false;
    }
    let owner = match hspan(
        log,
        memory,
        address.checked_add(cair_region_combat_data::OWNER),
        4,
        AIR_HISTORY_CENSUS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let _region_owner = u32::from_le_bytes(log.bytes[owner..owner + 4].try_into().unwrap());
    let first_header = match hspan(
        log,
        memory,
        address.checked_add(cair_region_combat_data::FIRST_RECORDS),
        16,
        AIR_HISTORY_CENSUS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let second_header = match hspan(
        log,
        memory,
        address.checked_add(cair_region_combat_data::SECOND_RECORDS),
        16,
        AIR_HISTORY_CENSUS_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    for header in [first_header, second_header] {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[header..header + 16]);
        let vector = match Array::from_header(raw, sair_wing_combat_data::STRIDE) {
            Some(array) => array,
            None => return false,
        };
        for ordinal in 0..vector.count {
            let record_addr = vector.element(ordinal, sair_wing_combat_data::STRIDE);
            // Record spans gate both tables: the damage decode needs each
            // record key, exactly like the typed walk.
            if !copy_history_record(log, memory, record_addr, damage_dead) {
                return false;
            }
            if log.overflowed {
                return false;
            }
        }
    }
    true
}

/// Mirror of one combat record with its two damage vectors. Record faults
/// return false; damage faults set `damage_dead` (records continue).
fn copy_history_record(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    record_addr: Option<usize>,
    damage_dead: &mut bool,
) -> bool {
    let address = match record_addr {
        Some(address) => address,
        None => return false,
    };
    if !copy_history_record_fields(log, memory, address) {
        return false;
    }
    if *damage_dead {
        return true;
    }
    for offset in [
        sair_wing_combat_data::FIRST_DAMAGE,
        sair_wing_combat_data::SECOND_DAMAGE,
    ] {
        let header = match hspan(
            log,
            memory,
            address.checked_add(offset),
            16,
            AIR_DAMAGE_MASK,
        ) {
            Some(start) => start,
            None => {
                *damage_dead = true;
                continue;
            }
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[header..header + 16]);
        let entries = match Array::from_header(raw, sdamage_entry::STRIDE) {
            Some(array) => array,
            None => {
                *damage_dead = true;
                continue;
            }
        };
        for index in 0..entries.count {
            let entry = entries.element(index, sdamage_entry::STRIDE);
            // Damage read_row scattered takes: head block plus amount.
            let head = hspan(log, memory, entry, 12, AIR_DAMAGE_MASK);
            let amount = entry.and_then(|entry| entry.checked_add(sdamage_entry::AMOUNT));
            let amount = hspan(log, memory, amount, 8, AIR_DAMAGE_MASK);
            if head.is_none() || amount.is_none() {
                *damage_dead = true;
                break;
            }
            if log.overflowed {
                return false;
            }
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Exactly the existing record head, attribution and copied-flag observations.
/// Damage headers/entries remain outside this group and in their native order.
fn copy_history_record_fields(
    log: &mut RawLog<AIR_HISTORY_N>,
    memory: &impl engine::Memory,
    address: usize,
) -> bool {
    let addresses = [
        Some(address),
        address.checked_add(sair_wing_combat_data::ATTRIBUTED_COUNTRY),
        address.checked_add(sair_wing_combat_data::COPIED_FLAG),
    ];
    let lengths = [0x1a, 4, 1];
    if !log.overflowed
        && addresses.iter().all(Option::is_some)
        && log.spans.capacity() - log.spans.len() >= lengths.len()
        && log
            .bytes
            .len()
            .checked_add(31)
            .is_some_and(|end| end <= log.bytes.capacity() && end <= u32::MAX as usize)
    {
        return log
            .take_scattered(
                memory,
                addresses.map(Option::unwrap),
                lengths,
                [AIR_HISTORY_CENSUS_MASK; 3],
            )
            .is_some_and(|(_, completed)| completed == lengths.len());
    }
    for (address, len) in addresses.into_iter().zip(lengths) {
        if hspan(log, memory, address, len, AIR_HISTORY_CENSUS_MASK).is_none() {
            return false;
        }
    }
    true
}

/// One structural copy pass: shared census, manager roster, per-owner
/// region takes with records and damage vectors. The job is cleared on
/// entry so pooled refreshes never accumulate spans across hours.
fn copy_air_history_pass(
    job: &mut AirHistoryJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; AIR_HISTORY_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    copy_air_history_owners(job, memory, base, game_state);
    // The single regional walk is anchor-charged to the history slot,
    // matching the read-group convention.
    ticks[0] += elapsed(started);
}

fn copy_air_history_owners(
    job: &mut AirHistoryJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    let log = &mut job.log;
    let countries_start = hspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        AIR_HISTORY_CENSUS_MASK,
    );
    let links_start = hspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        AIR_HISTORY_CENSUS_MASK,
    );
    let (Some(countries_start), Some(links_start)) = (countries_start, links_start) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    if countries_array.count < 2 || links.count == 0 {
        return;
    }
    let manager = match hspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STRATEGIC_AIR_MANAGER),
        8,
        AIR_HISTORY_CENSUS_MASK,
    )
    .and_then(|start| {
        usize::try_from(history_pool_u64(log, start))
            .ok()
            .filter(|&pointer| pointer != 0)
    }) {
        Some(manager) => manager,
        None => return,
    };
    let vtable_start = match hspan(log, memory, Some(manager), 8, AIR_HISTORY_CENSUS_MASK) {
        Some(start) => start,
        None => return,
    };
    let expected_manager = match base.checked_add(cstrategic_air_manager::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    // Manager vtable validation is a local parse over the staged word.
    if history_pool_u64(log, vtable_start) != expected_manager {
        return;
    }
    let slots_header = match hspan(
        log,
        memory,
        manager.checked_add(cstrategic_air_manager::COUNTRY_AIR),
        16,
        AIR_HISTORY_CENSUS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[slots_header..slots_header + 16]);
    let slots = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if slots.count == 0 {
        return;
    }
    let expected_owner = match base.checked_add(cstrategic_air::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let mut damage_dead = false;
    let mut first = 0u32;
    while first < slots.count {
        let chunk = ((slots.count - first) as usize).min(HISTORY_MIRROR_BATCH);
        // SLOT_FIELD gather decomposed per slot in walk order with
        // fixed-size stack staging (no allocation on the game thread).
        let mut words = [0u64; HISTORY_MIRROR_BATCH];
        let mut taken = 0usize;
        let mut batch_failed = false;
        for (index, word) in words.iter_mut().enumerate().take(chunk) {
            let addr = slots.element(first + index as u32, 8);
            match addr.and_then(|at| hspan(log, memory, Some(at), 8, AIR_HISTORY_CENSUS_MASK)) {
                Some(start) => {
                    *word = history_pool_u64(log, start);
                    taken += 1;
                }
                None => {
                    batch_failed = true;
                    break;
                }
            }
            if log.overflowed {
                return;
            }
        }
        // Resolve the recorded prefix in slot order, mirroring the visit:
        // identity takes, owner agreement, then region takes.
        for (index, word) in words.iter().enumerate().take(taken) {
            let address = match usize::try_from(*word).ok().filter(|&address| address != 0) {
                Some(address) => address,
                None => continue,
            };
            // IDENTITY_FIELDS: vtable word plus country word.
            let vtable_start = match hspan(log, memory, Some(address), 8, AIR_HISTORY_CENSUS_MASK) {
                Some(start) => start,
                None => return,
            };
            let country_start = match hspan(
                log,
                memory,
                address.checked_add(cstrategic_air::COUNTRY),
                4,
                AIR_HISTORY_CENSUS_MASK,
            ) {
                Some(start) => start,
                None => return,
            };
            let slot = first + index as u32;
            if history_pool_u64(log, vtable_start) != expected_owner {
                return;
            }
            let country = u32::from_le_bytes(
                log.bytes[country_start..country_start + 4]
                    .try_into()
                    .unwrap(),
            );
            if !copy_history_owner(
                log,
                memory,
                base,
                &links,
                &countries_array,
                countries_array.count,
                address,
                slot,
                country,
                &mut damage_dead,
            ) {
                return;
            }
            if log.overflowed {
                return;
            }
        }
        if batch_failed || log.overflowed {
            // The visit processes the recorded prefix, then fails at the
            // faulted slot with its own reason; the replay derives it from
            // the same spans.
            return;
        }
        first += chunk as u32;
    }
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any history table still needs double-read checks,
    /// compares the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_air_history_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: AirHistoryCopyContext,
        first: &mut AirHistoryJob,
        mut second: Option<&mut AirHistoryJob>,
    ) -> AirHistoryCopyReport {
        let mut ticks = [0u64; AIR_HISTORY_N];
        copy_air_history_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
        let mut matched = [true; AIR_HISTORY_N];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; AIR_HISTORY_N];
            copy_air_history_pass(
                second_job,
                memory,
                ctx.base,
                ctx.game_state,
                &mut second_ticks,
            );
            for (total, extra) in ticks.iter_mut().zip(second_ticks) {
                *total += extra;
            }
            // Per-table partitions: shared spans gate both slots, damage
            // spans only the damage slot.
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed paths.
        self.history.checked = ctx.needed[0];
        self.history.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.damage.checked = ctx.needed[1];
        self.damage.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
        AirHistoryCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw shared history payload: replays
    /// the regional walk against the recorded spans into the pooled buffers,
    /// with the identical bounded/capacity handling the game thread applies.
    /// Installs identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_air_history_payload(
        &mut self,
        base: usize,
        job: &AirHistoryJob,
        decode_ticks: &mut [u64; AIR_HISTORY_N],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let walked = cair_region_combat_data::walk_world(
            &recorded,
            base,
            job.game_state,
            &mut self.history.rows,
            &mut self.damage.rows,
        );
        self.history.outcome = bounded(walked.records, self.history.rows.len());
        self.damage.outcome = bounded(walked.damage, self.damage.rows.len());
        decode_ticks[0] += elapsed(started);
        self.history_raw_mode = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_bases_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = AirBasesJob::default();
        job.census.reserve_exact(3);
        job.seen_links.reserve_exact(3);
        job.seen_link_index
            .resize(job.seen_links.capacity(), usize::MAX);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &AirBasesJob| {
            vector_bytes(&job.census)
                + vector_bytes(&job.seen_links)
                + vector_bytes(&job.seen_link_index)
        };
        let initial = LOCAL.snapshot().current.auxiliary_vectors;
        assert_eq!(initial, expected(&job));
        job.grow();
        let grown = LOCAL.snapshot().current.auxiliary_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
        job.seen_link_index[0] = 0;
        job.seen_links.push((1, 1));
        job.clear();
        assert!(job.seen_link_index.iter().all(|&cell| cell == usize::MAX));
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        let mut moved = Some(job);
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        let job = moved.take().unwrap();
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

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

    #[test]
    fn wing_pack_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        for table in [TableId::WingEquipment, TableId::WingCumulative] {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(
                    replay::cadence::is_refresh(table, hour),
                    "{table:?} at {hour}"
                );
            }
        }
    }

    #[test]
    fn wing_split_copy_matches_direct_walk() {
        use super::wing_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn wing_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 2 wings with 3 equipment rows and 2 cumulative rows;
        // equipment cap 2 is the overflow-by-one arm (total 3 == cap+1),
        // cumulative cap 1 the overflow arm (total 2 == cap+1), cap 0 the
        // zero/massive arm. u32 unread saturation is not reachable and is
        // covered by the walk's own saturating arithmetic.
        use super::Capacities;
        use super::wing_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for (equipment, cumulative) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 3), (8192, 4096)] {
            let capacity = Capacities {
                rows: [equipment, 2048, cumulative, 8192, 16384, 1024, 2048],
                ace_names: 65536,
            };
            let direct = kit::direct_with(&memory, &capacity);
            let (split, report, _) = kit::split_with(&memory, &capacity);
            assert!(
                report.raw_ok,
                "equipment={equipment} cumulative={cumulative}"
            );
            assert_eq!(
                report.matched, [true; 2],
                "equipment={equipment} cumulative={cumulative}"
            );
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows with all rows present.
        let capacity = Capacities {
            rows: [3, 2048, 2, 8192, 16384, 1024, 2048],
            ace_names: 65536,
        };
        let (split, _, _) = kit::split_with(&memory, &capacity);
        assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn wing_decode_preserves_live_check_verdict_without_a_second_decode_pass() {
        use super::wing_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let (_, report, mut job) = kit::split(&memory);
        assert_eq!(report.matched, [true; 2]);
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; N]);
        let mut ticks = [0; 2];
        super::decode_air_payload(&mut captured, kit::BASE, &job, &mut ticks);
        assert_eq!(
            (captured.equipment.checked, captured.equipment.mismatches),
            (true, 0)
        );
        assert_eq!(
            (captured.cumulative.checked, captured.cumulative.mismatches),
            (true, 0)
        );
        assert!(matches!(captured.tables()[0], TableView::Complete(_)));
        assert!(matches!(captured.tables()[2], TableView::Complete(_)));
        // A real failed live partition must still withhold that table only.
        job.matched[0] = false;
        super::decode_air_payload(&mut captured, kit::BASE, &job, &mut ticks);
        assert_eq!(captured.equipment.mismatches, 1);
        assert_eq!(captured.cumulative.mismatches, 0);
        assert!(!matches!(captured.tables()[0], TableView::Complete(_)));
        assert!(matches!(captured.tables()[2], TableView::Complete(_)));
    }

    #[test]
    fn wing_copy_includes_assigned_mission_region_and_skips_null_country_slots() {
        use super::wing_test_kit as kit;
        use engine::{air_mission, cstrategic_region};
        let mut bytes = kit::fixture();
        let region = 0x3a000;
        let put = |bytes: &mut [u8], address: usize, value: u64| {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        };
        put(
            &mut bytes,
            kit::WING + cair_wing::MISSION + air_mission::REGION,
            region as u64,
        );
        put(
            &mut bytes,
            region,
            (kit::BASE + cstrategic_region::VTABLE_RVA) as u64,
        );
        put(
            &mut bytes,
            region + cstrategic_region::SECONDARY,
            (kit::BASE + cstrategic_region::SECONDARY_VTABLE_RVA) as u64,
        );
        bytes[region + cstrategic_region::ID..region + cstrategic_region::ID + 4]
            .copy_from_slice(&17u32.to_le_bytes());
        // The fixture's third slot is an unused alias; a null country slot
        // is not an absent wing census and must not stop discovery.
        put(&mut bytes, 0x11000 + 16, 0);
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert_eq!(report.matched, [true; 2]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn wing_split_copy_matches_direct_walk_under_truncation() {
        use super::wing_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct_truncated(&memory);
        let (split, report, _) = kit::split_truncated(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn wing_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::wing_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for (fault, matched) in [
            (
                kit::GAME_STATE + engine::cgame_state::COUNTRIES,
                [false, false],
            ),
            (kit::WING, [false, false]),
            (kit::WING + engine::cair_wing::EXPERIENCE, [true, false]),
            (kit::WING + engine::cair_wing::EQUIPMENT_POOL, [false, true]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "fault={fault:x}");
        }
    }

    #[test]
    fn wing_double_copy_compare_blocks_checks_on_tearing() {
        use super::wing_test_kit as kit;
        use super::{AirWingCopyContext, AirWingJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirWingCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 2],
        };
        let mut first = AirWingJob::new();
        let mut second = AirWingJob::new();
        let report = super::copy_air_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        second.log.bytes[100] ^= 0xff;
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn stats_split_copy_matches_direct_walk() {
        use super::stats_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = Capacities::default();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
        assert_eq!((split.rows[0].country, split.rows[0].region), (2, 0));
        assert_eq!((split.rows[1].country, split.rows[1].region), (2, 1));
        assert_eq!(split.rows[0].channels[0], 1000);
        assert_eq!(split.rows[1].channels[0], 2000);
    }

    #[test]
    fn stats_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Cap 1 is the overflow-by-one arm (total 2 == cap+1), cap 0 the
        // zero/massive arm. u32 unread saturation is not reachable with 2
        // rows and is covered by the walk's own saturating arithmetic.
        use super::stats_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for rows in [0, 1, 2, 64] {
            let capacities = Capacities {
                rows: [8192, 2048, 4096, 8192, 16384, 1024, rows],
                ace_names: 65536,
            };
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "rows={rows}");
            assert_eq!(report.matched, [true], "rows={rows}");
            kit::assert_matches_direct(&split, &direct);
        }
        let capacities = Capacities {
            rows: [8192, 2048, 4096, 8192, 16384, 1024, 2],
            ace_names: 65536,
        };
        let (split, _, _) = kit::split(&memory, &capacities);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn stats_double_copy_compare_blocks_checks_on_tearing() {
        use super::stats_test_kit as kit;
        use super::{AirStatsCopyContext, AirStatsJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirStatsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = AirStatsJob::new();
        let mut second = AirStatsJob::new();
        let report = super::copy_air_stats_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        second.log.bytes[100] ^= 0xff;
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn stats_staging_overflow_falls_back_to_typed_walk() {
        use super::stats_test_kit as kit;
        use super::{AirStatsCopyContext, AirStatsJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirStatsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = AirStatsJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
        let report = super::copy_air_stats_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }

    #[test]
    fn stats_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the statistics copy pass issues,
        // recorded through the real pass. Census words (4/8), array headers
        // (16), window byte (1), channel spans (8/32/40/72); the walk's
        // batched gathers decompose into these exact copies at replay.
        use super::stats_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let ctx = super::AirStatsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut job = super::AirStatsJob::new();
        let report = super::copy_air_stats_refresh(&memory, ctx, &mut job, None);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 16, 32, 40, 72] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn wing_staging_overflow_falls_back_to_typed_walk() {
        use super::wing_test_kit as kit;
        use super::{AirWingCopyContext, AirWingJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirWingCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 2],
        };
        let mut tiny = AirWingJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            checked: [false; 2],
            matched: [false; 2],
        };
        let report = super::copy_air_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }

    #[test]
    fn air_bases_split_copy_matches_direct_walk_with_alias_and_rocket_vector() {
        // Census alias skip plus a non-state base vector: the alias slot
        // publishes no duplicate rows and the rocket base decodes from the
        // same shared spans.
        use super::air_bases_test_kit as kit;
        let mut bytes = kit::fixture();
        kit::add_alias_and_rocket(&mut bytes);
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.bases_outcome, Outcome::Rows { len: 4 });
        assert_eq!(split.regs.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.aces_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn air_bases_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        for table in [
            TableId::AirBases,
            TableId::AirBaseRegistrations,
            TableId::AirAces,
        ] {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(
                    replay::cadence::is_refresh(table, hour),
                    "{table:?} at {hour}"
                );
            }
        }
    }

    #[test]
    fn air_bases_split_copy_matches_direct_walk() {
        use super::air_bases_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        kit::assert_matches_direct(&split, &direct);
        // Two state bases plus one stateless host; two registrations (one
        // with two wings, one empty); two aces (one alive inline, one dead
        // heap-named).
        assert_eq!(split.bases_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.aces_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.regs.outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            split.regs.wings,
            vec![kit::WING_HANDLE_0, kit::WING_HANDLE_1]
        );
        assert_eq!(split.bases[0].state, Some(1));
        assert_eq!(split.bases[0].province, Some(5));
        assert_eq!(split.bases[2].state, None);
        assert_eq!(split.aces[0].alive, 1);
        assert_eq!(split.aces[1].alive, 0);
        assert!(split.aces[1].death.is_some());
    }

    #[test]
    fn air_bases_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 3 bases, 2 registrations (2 wings) and 2 aces;
        // bases cap 2 is the overflow-by-one arm, wings cap 1 the
        // overflow arm, cap 0 the zero/massive arm.
        use super::air_bases_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for (bases, wings, aces, names) in [
            (0, 0, 0, 0),
            (1, 1, 1, 8),
            (2, 1, 1, 16),
            (3, 2, 2, 64),
            (64, 64, 64, 4096),
        ] {
            let direct = kit::direct_with(&memory, bases, wings, aces, names);
            let (split, report, _) = kit::split_with(&memory, bases, wings, aces, names);
            assert!(
                report.raw_ok,
                "bases={bases} wings={wings} aces={aces} names={names}"
            );
            assert_eq!(
                report.matched, [true; 3],
                "bases={bases} wings={wings} aces={aces} names={names}"
            );
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows with all rows present.
        let direct = kit::direct_with(&memory, 3, 2, 2, 64);
        assert_eq!(direct.bases_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.aces_outcome, Outcome::Rows { len: 2 });
        assert_eq!(direct.regs.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn air_bases_split_copy_matches_direct_walk_on_fault_matrix() {
        // Every fault arm asserts byte-identical decode (same outcome and
        // rows as the typed walk on the same torn memory) plus the expected
        // per-slot raw-compare attribution: shared census faults block all
        // three slots, manager/vector faults block bases and registrations,
        // private faults block only their own slot.
        use super::air_bases_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for (fault, matched) in [
            // Shared country census: every slot blocks.
            (
                kit::GAME_STATE + engine::cgame_state::COUNTRIES,
                [false, false, false],
            ),
            (kit::C0, [false, false, false]),
            (kit::LTAB + 4, [false, false, false]),
            // Shared manager and vectors: bases and registrations block.
            (kit::MANAGER, [false, false, true]),
            (kit::SVEC, [false, false, true]),
            (kit::B0, [false, false, true]),
            // Bases-private fields: only bases blocks.
            (
                kit::B0 + engine::cair_base::HOST_REFERENCE,
                [false, true, true],
            ),
            (kit::S0, [false, true, true]),
            // Registrations-private spans: only registrations blocks.
            (kit::CONT1, [true, false, true]),
            (
                kit::CONT1 + engine::ccountry_air_container::COUNTRY,
                [true, false, true],
            ),
            (kit::POOL0, [true, false, true]),
            (kit::W0, [true, false, true]),
            // Aces-private spans: only aces blocks.
            (kit::C0 + engine::ccountry::ACES, [true, true, false]),
            (kit::A0, [true, true, false]),
            (kit::A0 + engine::cace::ALIVE, [true, true, false]),
            (kit::NSTR, [true, true, false]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "fault={fault:x}");
        }
    }

    #[test]
    fn air_base_container_chunks_preserve_null_slots_and_boundary_faults() {
        use super::air_bases_test_kit as kit;
        use engine::Memory;
        let mut bytes = kit::fixture();
        let header = kit::B0 + engine::cair_base::COUNTRY_CONTAINERS;
        let array = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
        // The fixture reserves space before the first container. Keep its one
        // nonnull slot, and extend the zero-filled pointer table across three
        // chunks without changing any registration row.
        assert!(array + 130 * 8 <= kit::CONT1);
        kit::put32(&mut bytes, header + 8, 130);
        kit::put32(&mut bytes, header + 12, 130);
        struct Fault<'a> {
            source: engine::memory::SliceMemory<'a>,
            address: Option<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if self
                    .address
                    .is_some_and(|fault| address <= fault && fault - address < output.len())
                {
                    return false;
                }
                self.source.copy(address, output)
            }
        }
        for bad_slot in [
            None,
            Some(0),
            Some(1),
            Some(63),
            Some(64),
            Some(127),
            Some(129),
        ] {
            let memory = Fault {
                source: kit::memory(&bytes),
                address: bad_slot.map(|slot| array + slot * 8),
            };
            let direct = kit::direct(&memory);
            let (split, report, job) = kit::split(&memory);
            assert!(report.raw_ok, "{bad_slot:?}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(
                report.matched,
                [true, bad_slot.is_none(), true],
                "{bad_slot:?}"
            );
            if bad_slot.is_none() {
                let chunks = job
                    .log
                    .spans
                    .iter()
                    .filter(|span| array <= span.addr && span.addr < array + 130 * 8)
                    .collect::<Vec<_>>();
                assert_eq!(chunks.len(), 3);
                assert!(chunks.iter().all(|span| span.ok && span.bulk));
            }
        }
    }

    #[test]
    fn air_bases_double_copy_compare_blocks_checks_on_tearing() {
        // Targeted tearing: a changed wing handle flips only the
        // registrations partition, a changed census id flips every
        // partition. Both copies run against live memories differing by one
        // field, exactly like a same-hour tear.
        use super::air_bases_test_kit as kit;
        use super::{AirBasesCopyContext, AirBasesJob};
        use crate::extended::fog::Capacities as FogCapacities;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirBasesCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 3],
        };
        let air_cap = super::Capacities::default();
        let fog_cap = FogCapacities::default();
        let mut air = super::Captured::with_capacities(&air_cap, &[0; 7]);
        let mut fog = crate::extended::fog::Captured::with_capacities(&fog_cap, &[0; 6]);
        let mut first = AirBasesJob::new();
        let report = air.copy_air_bases_refresh(&memory, ctx, &mut fog, &mut first, None);
        assert!(report.raw_ok);
        // Tear the wing handle: only the registrations partition flips.
        let mut torn = bytes.clone();
        kit::put64(&mut torn, kit::W0 + 24, 0x1234_5678_9abc_def0);
        let torn_memory = kit::memory(&torn);
        let mut second = AirBasesJob::new();
        let mut air2 = super::Captured::with_capacities(&air_cap, &[0; 7]);
        let mut fog2 = crate::extended::fog::Captured::with_capacities(&fog_cap, &[0; 6]);
        let torn_report =
            air2.copy_air_bases_refresh(&torn_memory, ctx, &mut fog2, &mut second, None);
        assert!(torn_report.raw_ok);
        assert!(first.log.partition_equal(&second.log, 0));
        assert!(!first.log.partition_equal(&second.log, 1));
        assert!(first.log.partition_equal(&second.log, 2));
        // Tear the census id: every partition flips (shared census).
        let mut torn = bytes.clone();
        kit::put32(&mut torn, kit::C0 + engine::ccountry::ID, 9);
        let torn_memory = kit::memory(&torn);
        let mut third = AirBasesJob::new();
        let mut air3 = super::Captured::with_capacities(&air_cap, &[0; 7]);
        let mut fog3 = crate::extended::fog::Captured::with_capacities(&fog_cap, &[0; 6]);
        let torn_report =
            air3.copy_air_bases_refresh(&torn_memory, ctx, &mut fog3, &mut third, None);
        assert!(torn_report.raw_ok);
        assert!(!first.log.partition_equal(&third.log, 0));
        assert!(!first.log.partition_equal(&third.log, 1));
        assert!(!first.log.partition_equal(&third.log, 2));
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn air_bases_shared_resolve_link_taken_once_per_pass() {
        // Shared-copy proof: two containers name the same country, but the
        // resolve link read is taken exactly once per pass (mirroring the
        // walk's Resolver cache) while both registrations decode.
        use super::air_bases_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let (_split, report, first) = kit::split(&memory);
        assert!(report.raw_ok);
        let addr = kit::LTAB + 4;
        let takes = first
            .log
            .spans
            .iter()
            .filter(|span| {
                span.addr == addr && span.len == 4 && span.mask == super::AIR_REGISTRATIONS_MASK
            })
            .count();
        assert_eq!(takes, 1, "resolve link takes: {:?}", first.log.spans);
    }

    /// Frozen pre-index resolver for exact observation/failure comparisons.
    fn linear_air_link(
        log: &mut RawLog<AIR_BASES_N>,
        memory: &impl Memory,
        links: &Array,
        seen: &mut Vec<(u32, u32)>,
        countries: u32,
        raw: u32,
    ) -> Option<u32> {
        if let Some((_, slot)) = seen.iter().find(|(key, _)| *key == raw) {
            return Some(*slot);
        }
        if raw == 0 || raw >= links.count {
            return None;
        }
        let offset = (raw as usize).checked_mul(4)?;
        let start = bspan(
            log,
            memory,
            links.data.checked_add(offset),
            4,
            AIR_REGISTRATIONS_MASK,
        )?;
        let slot = bases_pool_u32(log, start);
        if slot >= countries {
            return None;
        }
        if seen.len() == seen.capacity() {
            log.overflowed = true;
            return None;
        }
        seen.push((raw, slot));
        Some(slot)
    }

    #[derive(Default)]
    struct AirLinkMemory {
        value: std::cell::Cell<Option<u32>>,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }

    impl Memory for AirLinkMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            assert_eq!(output.len(), 4);
            output.fill(0xa5);
            let Some(value) = self.value.get() else {
                return false;
            };
            output.copy_from_slice(&value.to_le_bytes());
            true
        }
    }

    #[test]
    fn air_bases_link_index_matches_linear_collisions_faults_and_full_capacity() {
        // One-cell indexes deliberately collide every raw. Successful
        // observations stay authoritative even if the source changes or
        // faults; failed and invalid resolutions must be observed again.
        let steps = [
            (0, Some(1)),
            (1, Some(2)),
            (3, Some(1)),
            (1, None),
            (3, Some(0)),
            (2, None),
            (2, Some(8)),
            (2, Some(0)),
            (2, None),
            (4, Some(3)),
            (4, Some(1)),
            (8, Some(1)),
            (u32::MAX, Some(1)),
        ];
        for index_len in [0, 1, 2, 8] {
            for capacity in [0, 1, 2, 8] {
                let links = Array {
                    data: 0x1000,
                    count: 8,
                };
                let mut actual = RawLog::new(32, 128);
                let mut expected = RawLog::new(32, 128);
                let mut seen = Vec::with_capacity(capacity);
                let mut linear_seen = Vec::with_capacity(capacity);
                let mut index = vec![usize::MAX; index_len];
                let index_capacity = index.capacity();
                let memory = AirLinkMemory::default();
                let reference = AirLinkMemory::default();
                let mut results = Vec::new();
                for (raw, value) in steps {
                    memory.value.set(value);
                    reference.value.set(value);
                    let result =
                        resolve_link(&mut actual, &memory, &links, &mut seen, &mut index, 4, raw);
                    assert_eq!(
                        result,
                        linear_air_link(
                            &mut expected,
                            &reference,
                            &links,
                            &mut linear_seen,
                            4,
                            raw
                        ),
                        "index {index_len}, seen capacity {capacity}, raw {raw}"
                    );
                    results.push(result);
                    assert!(actual.logs_equal(&expected));
                    assert_eq!(actual.fault, expected.fault);
                    assert_eq!(actual.raw_bytes, expected.raw_bytes);
                    assert_eq!(actual.overflowed, expected.overflowed);
                    assert_eq!(seen, linear_seen);
                    assert_eq!(*memory.calls.borrow(), *reference.calls.borrow());
                    assert_eq!(seen.capacity(), capacity);
                    assert_eq!(index.capacity(), index_capacity);
                }
                if capacity == 8 {
                    assert_eq!(
                        results,
                        [
                            None,
                            Some(2),
                            Some(1),
                            Some(2),
                            Some(1),
                            None,
                            None,
                            Some(0),
                            Some(0),
                            Some(3),
                            Some(3),
                            None,
                            None,
                        ]
                    );
                    assert_eq!(
                        *memory.calls.borrow(),
                        [
                            (0x1004, 4),
                            (0x100c, 4),
                            (0x1008, 4),
                            (0x1008, 4),
                            (0x1008, 4),
                            (0x1010, 4),
                        ]
                    );
                } else {
                    assert!(actual.overflowed);
                    // The uncached successful read occurs before detecting
                    // a full seen list; the index cannot preflight it away.
                    let expected_calls = match capacity {
                        0 => 1,
                        1 => 2,
                        2 => 5,
                        _ => unreachable!(),
                    };
                    assert_eq!(memory.calls.borrow().len(), expected_calls);
                }
            }
        }
    }

    #[test]
    fn air_bases_link_index_preserves_local_and_raw_pool_failures() {
        for data in [0x1000, usize::MAX - 1, 0x8000_0000_0000] {
            for (spans, bytes) in [(0, 8), (2, 0), (2, 8)] {
                let links = Array { data, count: 3 };
                let mut actual = RawLog::new(spans, bytes);
                let mut expected = RawLog::new(spans, bytes);
                let mut seen = Vec::with_capacity(2);
                let mut linear_seen = Vec::with_capacity(2);
                let mut index = [usize::MAX; 1];
                let memory = AirLinkMemory::default();
                let reference = AirLinkMemory::default();
                memory.value.set(Some(1));
                reference.value.set(Some(1));
                for raw in [0, 1, 1, 3, u32::MAX] {
                    assert_eq!(
                        resolve_link(&mut actual, &memory, &links, &mut seen, &mut index, 3, raw),
                        linear_air_link(
                            &mut expected,
                            &reference,
                            &links,
                            &mut linear_seen,
                            3,
                            raw
                        )
                    );
                    assert!(actual.logs_equal(&expected));
                    assert_eq!(actual.fault, expected.fault);
                    assert_eq!(actual.raw_bytes, expected.raw_bytes);
                    assert_eq!(actual.overflowed, expected.overflowed);
                    assert_eq!(actual.grow_for_next(), expected.grow_for_next());
                    assert_eq!(seen, linear_seen);
                    assert_eq!(*memory.calls.borrow(), *reference.calls.borrow());
                }
            }
        }
    }

    #[test]
    fn air_bases_link_index_validates_hints_and_resets_between_passes() {
        let mut job = AirBasesJob {
            log: RawLog::new(8, 32),
            seen_links: Vec::with_capacity(4),
            seen_link_index: vec![usize::MAX; 2],
            ..Default::default()
        };
        let links = Array {
            data: 0x1000,
            count: 4,
        };
        let memory = AirLinkMemory::default();
        for (raw, value, expected) in [(1, 2, 2), (3, 1, 1), (1, 0, 2)] {
            memory.value.set(Some(value));
            assert_eq!(
                resolve_link(
                    &mut job.log,
                    &memory,
                    &links,
                    &mut job.seen_links,
                    &mut job.seen_link_index,
                    4,
                    raw,
                ),
                Some(expected)
            );
        }
        job.seen_link_index[1] = 99;
        memory.value.set(None);
        assert_eq!(
            resolve_link(
                &mut job.log,
                &memory,
                &links,
                &mut job.seen_links,
                &mut job.seen_link_index,
                4,
                1,
            ),
            Some(2)
        );
        assert_eq!(memory.calls.borrow().len(), 2);
        let capacities = (job.seen_links.capacity(), job.seen_link_index.capacity());
        job.clear();
        assert!(job.seen_links.is_empty());
        assert!(job.seen_link_index.iter().all(|&cell| cell == usize::MAX));
        memory.value.set(Some(3));
        assert_eq!(
            resolve_link(
                &mut job.log,
                &memory,
                &links,
                &mut job.seen_links,
                &mut job.seen_link_index,
                4,
                1,
            ),
            Some(3)
        );
        assert_eq!(memory.calls.borrow().len(), 3);
        assert_eq!(
            (job.seen_links.capacity(), job.seen_link_index.capacity()),
            capacities
        );
    }

    #[test]
    fn air_bases_link_index_keeps_the_complete_capture_read_trace() {
        use super::air_bases_test_kit as kit;
        struct Trace<'a> {
            inner: engine::memory::SliceMemory<'a>,
            calls: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Trace<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Trace {
            inner: kit::memory(&bytes),
            calls: Default::default(),
        };
        let reference = Trace {
            inner: kit::memory(&bytes),
            calls: Default::default(),
        };
        let mut indexed = AirBasesJob::new();
        let mut linear = AirBasesJob::new();
        linear.seen_link_index.clear();
        for _ in 0..2 {
            copy_air_bases_pass(
                &mut indexed,
                &memory,
                kit::BASE,
                kit::GAME_STATE,
                &mut [0; 3],
            );
            copy_air_bases_pass(
                &mut linear,
                &reference,
                kit::BASE,
                kit::GAME_STATE,
                &mut [0; 3],
            );
            assert!(!indexed.log.overflowed);
            assert!(indexed.log.logs_equal(&linear.log));
            assert_eq!(indexed.log.fault, linear.log.fault);
            assert_eq!(indexed.log.raw_bytes, linear.log.raw_bytes);
            assert_eq!(indexed.seen_links, linear.seen_links);
            assert_eq!(*memory.calls.borrow(), *reference.calls.borrow());
        }
    }

    #[test]
    fn air_bases_registrations_decode_matches_direct_from_shared_log() {
        // Dual-decode proof: the registrations half decoded alone from the
        // shared log (fresh fog buffers, no bases state) equals the typed
        // walk, proving both families read identical rows from shared spans.
        use super::air_bases_test_kit as kit;
        use crate::extended::fog::Capacities as FogCapacities;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (_split, report, first) = kit::split(&memory);
        assert!(report.raw_ok);
        let fog_cap = FogCapacities::default();
        let mut fog = crate::extended::fog::Captured::with_capacities(&fog_cap, &[0; 6]);
        let mut ticks = [0u64; 3];
        fog.decode_registrations_payload(kit::BASE, &first, &mut ticks);
        let alone = fog.registrations_snapshot();
        assert_eq!(alone.outcome, direct.regs.outcome, "registrations outcome");
        assert_eq!(alone.rows, direct.regs.rows, "registrations rows");
        assert_eq!(alone.wings, direct.regs.wings, "registrations wings");
    }

    #[test]
    fn air_bases_staging_overflow_falls_back_to_typed_walk() {
        use super::air_bases_test_kit as kit;
        use super::{AirBasesCopyContext, AirBasesJob};
        use crate::extended::fog::Capacities as FogCapacities;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirBasesCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 3],
        };
        let mut tiny = AirBasesJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            census: Vec::new(),
            seen_links: Vec::new(),
            seen_link_index: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        let air_cap = super::Capacities::default();
        let fog_cap = FogCapacities::default();
        let mut air = super::Captured::with_capacities(&air_cap, &[0; 7]);
        let mut fog = crate::extended::fog::Captured::with_capacities(&fog_cap, &[0; 6]);
        let report = air.copy_air_bases_refresh(&memory, ctx, &mut fog, &mut tiny, None);
        assert!(!report.raw_ok);
    }

    #[test]
    fn air_bases_pools_cover_appendix_worst_case() {
        // Worst-case caps proof: the steady-state pools exceed a
        // conservative span/byte estimate built from the appendix census
        // (396 bases, 5,148 registrations, 57 aces, 439 countries), so live
        // refreshes never take the overflow fallback in steady state.
        use super::AirBasesJob;
        let job = AirBasesJob::new();
        // Census: 2 headers + 439 slots x (pointer, vtable, id, 3 links,
        // canonical pointer). Vectors: manager + vtable + 4 headers + ~500
        // element pointers + identities. Bases detail: ~500 x 6 field spans.
        // Registrations: 5,148 rows x (container pointer, vtable, country,
        // link, pools header) + pools/blocks/headers + ~10k wing takes.
        // Aces: 439 roster headers + 57 x ~13 object/name spans.
        let spans =
            2 + 439 * 7 + 6 + 500 * 2 + 500 * 6 + 5_148 * 5 + 5_148 * 3 + 10_000 + 439 + 57 * 13;
        assert!(
            job.log.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.log.spans.capacity()
        );
        // Bytes: census words + identities + host blocks + container/pool
        // headers + wing identities + ace objects and names, with headroom.
        let bytes = 439 * 64 + 500 * 48 + 500 * 64 + 5_148 * 64 + 10_000 * 40 + 57 * 512;
        assert!(
            job.log.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.log.bytes.capacity()
        );
        assert!(job.census.capacity() >= 439);
        assert!(job.seen_links.capacity() >= 439);
        assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
    }

    #[test]
    fn air_bases_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the shared copy pass issues, recorded
        // through the real pass. Census words (4/8), array headers (16),
        // base identity (16), host block (32), state/province words (8/4),
        // pool/wing blocks (chunked 8s), wing identities (32), ace scattered
        // fields (16/4/1/12) and name bytes (1 + content).
        use super::air_bases_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let (_split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 12, 16, 32] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn air_history_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        for table in [TableId::AirRegionHistory, TableId::AirRegionDamage] {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(
                    replay::cadence::is_refresh(table, hour),
                    "{table:?} at {hour}"
                );
            }
        }
    }

    #[test]
    fn air_history_split_copy_matches_direct_walk() {
        use super::air_history_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        kit::assert_matches_direct(&split, &direct);
        // Three records across two sides plus three damage entries.
        assert_eq!(split.history_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.damage_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.history[0].key.wing_id, 0x1111);
        assert_eq!(split.history[0].region_owner, 7);
        assert_eq!(split.damage[0].cause, 5);
        assert_eq!(split.damage[0].amount, 1000);
        assert_eq!(split.damage[2].channel, 1);
    }

    #[test]
    fn air_history_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        use super::air_history_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for (history, damage) in [(0, 0), (1, 1), (2, 1), (3, 3), (64, 64)] {
            let direct = kit::direct_with(&memory, history, damage);
            let (split, report, _) = kit::split_with(&memory, history, damage);
            assert!(report.raw_ok, "history={history} damage={damage}");
            assert_eq!(
                report.matched, [true; 2],
                "history={history} damage={damage}"
            );
            kit::assert_matches_direct(&split, &direct);
        }
        let direct = kit::direct_with(&memory, 3, 3);
        assert_eq!(direct.history_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.damage_outcome, Outcome::Rows { len: 3 });
    }

    #[test]
    fn air_history_split_copy_matches_direct_walk_on_fault_matrix() {
        // Every fault arm asserts byte-identical decode plus per-slot
        // attribution: record faults block both slots (the typed walk fails
        // both tables), damage-only faults block damage while history stays
        // Rows (the typed walk keeps records).
        use super::air_history_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for (fault, matched) in [
            (
                kit::GAME_STATE + engine::cgame_state::COUNTRIES,
                [false, false],
            ),
            (kit::MANAGER, [false, false]),
            (kit::R0, [false, false]),
            (
                kit::R0 + engine::cair_region_combat_data::OWNER,
                [false, false],
            ),
            (kit::REC0, [false, false]),
            // Damage-scoped faults: history still decodes identical Rows.
            (
                kit::REC0 + engine::sair_wing_combat_data::FIRST_DAMAGE,
                [true, false],
            ),
            (kit::E0, [true, false]),
            (kit::E0 + engine::sdamage_entry::AMOUNT, [true, false]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "fault={fault:x}");
        }
    }

    #[test]
    fn air_history_double_copy_compare_blocks_checks_on_tearing() {
        // Targeted tearing: a changed damage amount flips only the damage
        // partition, a changed record field flips both.
        use super::air_history_test_kit as kit;
        use super::{AirHistoryCopyContext, AirHistoryJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirHistoryCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 2],
        };
        let capacities = super::Capacities {
            rows: [8192, 2048, 4096, 8192, 16_384, 1024, 2048],
            ace_names: 65_536,
        };
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 7]);
        let mut first = AirHistoryJob::new();
        let report = captured.copy_air_history_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let mut torn = bytes.clone();
        torn[kit::E0 + engine::sdamage_entry::AMOUNT] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut second = AirHistoryJob::new();
        let mut captured2 = super::Captured::with_capacities(&capacities, &[0; 7]);
        let torn_report = captured2.copy_air_history_refresh(&torn_memory, ctx, &mut second, None);
        assert!(torn_report.raw_ok);
        assert!(first.log.partition_equal(&second.log, 0));
        assert!(!first.log.partition_equal(&second.log, 1));
        let mut torn = bytes.clone();
        torn[kit::REC0 + engine::sair_wing_combat_data::PLANE_COUNT] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut third = AirHistoryJob::new();
        let mut captured3 = super::Captured::with_capacities(&capacities, &[0; 7]);
        let torn_report = captured3.copy_air_history_refresh(&torn_memory, ctx, &mut third, None);
        assert!(torn_report.raw_ok);
        assert!(!first.log.partition_equal(&third.log, 0));
        assert!(!first.log.partition_equal(&third.log, 1));
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn air_history_staging_overflow_falls_back_to_typed_walk() {
        use super::air_history_test_kit as kit;
        use super::{AirHistoryCopyContext, AirHistoryJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirHistoryCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 2],
        };
        let mut tiny = AirHistoryJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
        let capacities = super::Capacities {
            rows: [8192, 2048, 4096, 8192, 16_384, 1024, 2048],
            ace_names: 65_536,
        };
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 7]);
        let report = captured.copy_air_history_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }

    #[test]
    fn air_history_pools_cover_appendix_worst_case() {
        // Worst-case caps proof: the steady-state pools exceed a
        // conservative span/byte estimate built from the appendix census
        // (43 history groups with ~1,341 native entries, 34 damage groups),
        // so live refreshes never take the overflow fallback in steady state.
        use super::AirHistoryJob;
        let job = AirHistoryJob::new();
        // Census and manager: headers plus 439 country slots and one owner
        // roster. Regions: ~100 roots x (pointer + identity + headers).
        // Records: 1,341 x (head + attribution + copied + 2 damage headers).
        // Damage: ~500 entries x (head + amount).
        let spans = 2 + 439 * 2 + 6 + 100 * 7 + 1_341 * 6 + 500 * 2;
        assert!(
            job.log.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.log.spans.capacity()
        );
        let bytes = 439 * 32 + 100 * 64 + 1_341 * 64 + 500 * 24;
        assert!(
            job.log.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.log.bytes.capacity()
        );
    }

    #[test]
    fn air_history_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the shared copy pass issues, recorded
        // through the real pass. Census words (4/8), array headers (16),
        // root identity (8/4/16), record head (26) with attribution (4) and
        // copied byte (1), damage headers (16) with entries (12/8).
        use super::air_history_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let (_split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 12, 16, 26] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }
}

#[cfg(test)]
mod fixed_mirror_gather_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct TraceMemory<'a> {
        bytes: &'a [u8],
        blocked: std::ops::Range<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
        gathers: Cell<usize>,
        changing_address: Option<usize>,
    }

    impl<'a> TraceMemory<'a> {
        fn new(bytes: &'a [u8], blocked: std::ops::Range<usize>) -> Self {
            Self {
                bytes,
                blocked,
                reads: RefCell::new(Vec::new()),
                gathers: Cell::new(0),
                changing_address: None,
            }
        }
    }

    struct ScalarFields<'a, 'b>(&'a TraceMemory<'b>);

    impl Memory for ScalarFields<'_, '_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.0.copy(address, destination)
        }
    }

    impl Memory for TraceMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            if address < self.blocked.end && end > self.blocked.start {
                destination.fill(0xa5); // A failed field may have copied bytes.
                return false;
            }
            let copied = engine::memory::SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination);
            if copied && self.changing_address == Some(address) && !destination.is_empty() {
                destination[0] = destination[0].wrapping_add(self.reads.borrow().len() as u8);
            }
            copied
        }

        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.gathers.set(self.gathers.get() + 1);
            ScalarFields(self).copy_scattered(addresses, destinations)
        }
    }

    fn assert_logs_and_growth<const M: usize>(mut actual: RawLog<M>, mut scalar: RawLog<M>) {
        assert_eq!(actual.spans, scalar.spans);
        assert_eq!(actual.bytes, scalar.bytes);
        assert_eq!(actual.fault, scalar.fault);
        assert_eq!(actual.raw_bytes, scalar.raw_bytes);
        assert_eq!(actual.overflowed, scalar.overflowed);
        assert_eq!(actual.has_bulk, scalar.has_bulk);
        assert_eq!(actual.grow_for_next(), scalar.grow_for_next());
        assert_eq!(actual.spans.capacity(), scalar.spans.capacity());
        assert_eq!(actual.bytes.capacity(), scalar.bytes.capacity());
        // Exercise private growth requests through their existing public
        // effect before clear discards them.
        actual.grow();
        scalar.grow();
        assert_eq!(actual.spans.capacity(), scalar.spans.capacity());
        assert_eq!(actual.bytes.capacity(), scalar.bytes.capacity());
        let capacity = (actual.spans.capacity(), actual.bytes.capacity());
        actual.clear();
        scalar.clear();
        assert_eq!(actual.spans, scalar.spans);
        assert_eq!(actual.bytes, scalar.bytes);
        assert_eq!(actual.fault, scalar.fault);
        assert_eq!(actual.raw_bytes, scalar.raw_bytes);
        assert_eq!(actual.overflowed, scalar.overflowed);
        assert_eq!(actual.has_bulk, scalar.has_bulk);
        assert_eq!(actual.grow_for_next(), scalar.grow_for_next());
        assert_eq!((actual.spans.capacity(), actual.bytes.capacity()), capacity);
        assert_eq!((scalar.spans.capacity(), scalar.bytes.capacity()), capacity);
    }

    fn scalar_stats_channels(
        log: &mut RawLog<AIR_STATS_N>,
        memory: &impl Memory,
        values: Array,
    ) -> bool {
        // Independent copy of the original scalar loop, not the gather layout.
        for (len, index) in [
            (32, 0_usize),
            (8, 10),
            (8, 17),
            (8, 24),
            (72, 36),
            (8, 51),
            (8, 58),
            (8, 65),
            (40, 77),
        ] {
            let Some(address) = index
                .checked_mul(engine::loop_history_record_fields::VALUE_STRIDE)
                .and_then(|offset| values.data.checked_add(offset))
            else {
                return false;
            };
            if sspan(log, memory, Some(address), len, STATS_MASK).is_none() {
                return false;
            }
            if log.overflowed {
                return false;
            }
        }
        true
    }

    #[test]
    fn stats_gather_matches_scalar_spans_reads_and_every_field_fault() {
        let bytes: Vec<_> = (0..0x2000).map(|index| (index % 251) as u8).collect();
        let values = Array {
            data: 0x1000,
            count: 89,
        };
        let fields = [
            (0, 32),
            (80, 8),
            (136, 8),
            (192, 8),
            (288, 72),
            (408, 8),
            (464, 8),
            (520, 8),
            (616, 40),
        ];
        let faults = std::iter::once(0..0).chain(fields.into_iter().flat_map(|(offset, len)| {
            (0..len).map(move |byte| values.data + offset + byte..values.data + offset + byte + 1)
        }));
        for fault in faults {
            let mut actual_memory = TraceMemory::new(&bytes, fault.clone());
            let mut scalar_memory = TraceMemory::new(&bytes, fault.clone());
            actual_memory.changing_address = Some(values.data);
            scalar_memory.changing_address = Some(values.data);
            let mut actual = RawLog::new(32, 512);
            let mut scalar = RawLog::new(32, 512);
            // Repeating the same address must retain both observations, even
            // after a source fault. No field or earlier fault is cached.
            for _ in 0..2 {
                assert_eq!(
                    copy_stats_channels(&mut actual, &actual_memory, values),
                    scalar_stats_channels(&mut scalar, &scalar_memory, values),
                    "fault {fault:x?}"
                );
            }
            assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
            assert_eq!(actual_memory.gathers.get(), 2);
            if fault.is_empty() {
                let expected: Vec<_> = fields
                    .into_iter()
                    .cycle()
                    .take(18)
                    .map(|(offset, len)| (values.data + offset, len))
                    .collect();
                assert_eq!(*actual_memory.reads.borrow(), expected);
                assert_eq!(actual.raw_bytes, [384]);
                assert_ne!(actual.bytes[0], actual.bytes[192]);
            }
            assert_logs_and_growth(actual, scalar);
        }
    }

    #[test]
    fn stats_gather_preserves_scalar_capacity_admission_and_growth() {
        let bytes = vec![7; 0x2000];
        let values = Array {
            data: 0x1000,
            count: 89,
        };
        for spans in 0..=10 {
            for capacity in [
                0, 31, 32, 39, 40, 47, 48, 55, 56, 127, 128, 135, 136, 143, 144, 151, 152, 191,
                192, 193,
            ] {
                for prefill in [false, true] {
                    let actual_memory = TraceMemory::new(&bytes, 0..0);
                    let scalar_memory = TraceMemory::new(&bytes, 0..0);
                    let mut actual = RawLog::new(spans, capacity);
                    let mut scalar = RawLog::new(spans, capacity);
                    if prefill {
                        assert_eq!(
                            actual.take(&actual_memory, 0x80, 1, STATS_MASK),
                            scalar.take(&scalar_memory, 0x80, 1, STATS_MASK)
                        );
                    }
                    assert_eq!(
                        copy_stats_channels(&mut actual, &actual_memory, values),
                        scalar_stats_channels(&mut scalar, &scalar_memory, values)
                    );
                    assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
                    assert_logs_and_growth(actual, scalar);
                }
            }
        }
    }

    #[test]
    fn history_gather_preserves_damage_continuation_and_every_record_field_fault() {
        use super::air_history_test_kit as kit;
        let bytes = kit::fixture();
        let record_fields = [
            (kit::REC0, 26),
            (kit::REC0 + 0x50, 4),
            (kit::REC0 + 0x90, 1),
        ];
        let faults = std::iter::once(0..0)
            .chain(record_fields.into_iter().flat_map(|(address, len)| {
                (0..len).map(move |byte| address + byte..address + byte + 1)
            }))
            .chain([
                kit::REC0 + 0x20..kit::REC0 + 0x21,
                kit::REC0 + 0x38..kit::REC0 + 0x39,
                kit::E0..kit::E0 + 1,
                kit::E0 + 0x10..kit::E0 + 0x11,
            ]);
        for fault in faults {
            for initially_dead in [false, true] {
                let mut actual_memory = TraceMemory::new(&bytes, fault.clone());
                let mut scalar_memory = TraceMemory::new(&bytes, fault.clone());
                actual_memory.changing_address = Some(kit::REC0 + 0x50);
                scalar_memory.changing_address = Some(kit::REC0 + 0x50);
                let mut actual = RawLog::new(64, 2048);
                let mut scalar = RawLog::new(64, 2048);
                let mut actual_dead = initially_dead;
                let mut scalar_dead = initially_dead;
                for _ in 0..2 {
                    assert_eq!(
                        copy_history_record(
                            &mut actual,
                            &actual_memory,
                            Some(kit::REC0),
                            &mut actual_dead
                        ),
                        scalar_history_record(
                            &mut scalar,
                            &scalar_memory,
                            Some(kit::REC0),
                            &mut scalar_dead
                        )
                    );
                    assert_eq!(actual_dead, scalar_dead);
                }
                assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
                assert_eq!(actual_memory.gathers.get(), 2);
                if fault.is_empty() {
                    let changes: Vec<_> = actual
                        .spans
                        .iter()
                        .filter(|span| span.addr == kit::REC0 + 0x50)
                        .map(|span| actual.bytes[span.start as usize])
                        .collect();
                    assert_eq!(changes.len(), 2);
                    assert_ne!(changes[0], changes[1]);
                }
                if fault.start == kit::E0 && !initially_dead {
                    let reads = actual_memory.reads.borrow();
                    let failed_head = reads
                        .iter()
                        .position(|&(address, _)| address == kit::E0)
                        .unwrap();
                    assert_eq!(reads[failed_head + 1], (kit::E0 + 0x10, 8));
                }
                assert_logs_and_growth(actual, scalar);
            }
        }
    }

    #[test]
    fn history_gather_preserves_scalar_capacity_prefixes_and_masks() {
        use super::air_history_test_kit as kit;
        let bytes = kit::fixture();
        let memory = TraceMemory::new(&bytes, 0..0);
        let mut probe = RawLog::new(64, 2048);
        let mut probe_dead = false;
        assert!(scalar_history_record(
            &mut probe,
            &memory,
            Some(kit::REC0),
            &mut probe_dead
        ));
        let mut capacities = vec![0, 1];
        let mut end = 1; // The prefix observation below occupies one byte.
        for span in &probe.spans {
            end += span.len as usize;
            capacities.extend([end - 1, end, end + 1]);
        }
        capacities.sort_unstable();
        capacities.dedup();
        for spans in 0..=probe.spans.len() + 1 {
            for &capacity in &capacities {
                for fault in [
                    0..0,
                    kit::REC0 + 0x50..kit::REC0 + 0x51,
                    kit::E0..kit::E0 + 1,
                ] {
                    let actual_memory = TraceMemory::new(&bytes, fault.clone());
                    let scalar_memory = TraceMemory::new(&bytes, fault);
                    let mut actual = RawLog::new(spans, capacity);
                    let mut scalar = RawLog::new(spans, capacity);
                    // Occupy storage with a damage-only observation, so the
                    // fit test must use remaining capacity and preserve masks.
                    assert_eq!(
                        actual.take(&actual_memory, 0x80, 1, AIR_DAMAGE_MASK),
                        scalar.take(&scalar_memory, 0x80, 1, AIR_DAMAGE_MASK)
                    );
                    let mut actual_dead = false;
                    let mut scalar_dead = false;
                    assert_eq!(
                        copy_history_record(
                            &mut actual,
                            &actual_memory,
                            Some(kit::REC0),
                            &mut actual_dead
                        ),
                        scalar_history_record(
                            &mut scalar,
                            &scalar_memory,
                            Some(kit::REC0),
                            &mut scalar_dead
                        )
                    );
                    assert_eq!(actual_dead, scalar_dead);
                    assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
                    assert_logs_and_growth(actual, scalar);
                }
            }
        }
    }

    #[test]
    fn mirror_gathers_preserve_invalid_address_and_preexisting_overflow_behavior() {
        let bytes = vec![0; 1024];
        for address in [0, usize::MAX - 32, usize::MAX, 0x0000_8000_0000_0000] {
            let actual_memory = TraceMemory::new(&bytes, 0..0);
            let scalar_memory = TraceMemory::new(&bytes, 0..0);
            let mut actual = RawLog::new(32, 512);
            let mut scalar = RawLog::new(32, 512);
            let values = Array {
                data: address,
                count: 89,
            };
            assert_eq!(
                copy_stats_channels(&mut actual, &actual_memory, values),
                scalar_stats_channels(&mut scalar, &scalar_memory, values)
            );
            assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
            assert_logs_and_growth(actual, scalar);
        }
        for address in [
            None,
            Some(0),
            Some(usize::MAX - 32),
            Some(usize::MAX),
            Some(0x0000_8000_0000_0000),
        ] {
            for already_overflowed in [false, true] {
                let actual_memory = TraceMemory::new(&bytes, 0..0);
                let scalar_memory = TraceMemory::new(&bytes, 0..0);
                let mut actual = RawLog::new(32, 512);
                let mut scalar = RawLog::new(32, 512);
                actual.overflowed = already_overflowed;
                scalar.overflowed = already_overflowed;
                let mut actual_dead = false;
                let mut scalar_dead = false;
                assert_eq!(
                    copy_history_record(&mut actual, &actual_memory, address, &mut actual_dead),
                    scalar_history_record(&mut scalar, &scalar_memory, address, &mut scalar_dead)
                );
                assert_eq!(actual_dead, scalar_dead);
                assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
                assert_logs_and_growth(actual, scalar);
            }
        }
    }

    fn scalar_history_record(
        log: &mut RawLog<AIR_HISTORY_N>,
        memory: &impl engine::Memory,
        record_addr: Option<usize>,
        damage_dead: &mut bool,
    ) -> bool {
        let address = match record_addr {
            Some(address) => address,
            None => return false,
        };
        // read_row scattered takes: head block, attribution word, copied byte.
        if hspan(log, memory, Some(address), 0x1a, AIR_HISTORY_CENSUS_MASK).is_none() {
            return false;
        }
        if hspan(
            log,
            memory,
            address.checked_add(sair_wing_combat_data::ATTRIBUTED_COUNTRY),
            4,
            AIR_HISTORY_CENSUS_MASK,
        )
        .is_none()
        {
            return false;
        }
        if hspan(
            log,
            memory,
            address.checked_add(sair_wing_combat_data::COPIED_FLAG),
            1,
            AIR_HISTORY_CENSUS_MASK,
        )
        .is_none()
        {
            return false;
        }
        if *damage_dead {
            return true;
        }
        for offset in [
            sair_wing_combat_data::FIRST_DAMAGE,
            sair_wing_combat_data::SECOND_DAMAGE,
        ] {
            let header = match hspan(
                log,
                memory,
                address.checked_add(offset),
                16,
                AIR_DAMAGE_MASK,
            ) {
                Some(start) => start,
                None => {
                    *damage_dead = true;
                    continue;
                }
            };
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&log.bytes[header..header + 16]);
            let entries = match Array::from_header(raw, sdamage_entry::STRIDE) {
                Some(array) => array,
                None => {
                    *damage_dead = true;
                    continue;
                }
            };
            for index in 0..entries.count {
                let entry = entries.element(index, sdamage_entry::STRIDE);
                // Damage read_row scattered takes: head block plus amount.
                let head = hspan(log, memory, entry, 12, AIR_DAMAGE_MASK);
                let amount = entry.and_then(|entry| entry.checked_add(sdamage_entry::AMOUNT));
                let amount = hspan(log, memory, amount, 8, AIR_DAMAGE_MASK);
                if head.is_none() || amount.is_none() {
                    *damage_dead = true;
                    break;
                }
                if log.overflowed {
                    return false;
                }
            }
            if log.overflowed {
                return false;
            }
        }
        true
    }
}

/// Shared synthetic address space for the wing-pack copy/decode harness:
/// one country, one pool, two wings with equipment entries and cumulative
/// counters. Mirrors the engine wing/equipment fixture shapes.
#[cfg(test)]
pub(crate) mod wing_test_kit {
    use super::{AirWingCopyContext, AirWingCopyReport, AirWingJob, Capacities, Captured};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    const C0: usize = 0x20000;
    const MANAGER: usize = 0x30000;
    const OWNER_AIR: usize = 0x31000;
    const POOL: usize = 0x32000;
    pub(crate) const WING: usize = 0x33000;
    const WING2: usize = 0x34000;
    pub(crate) const ENTRIES: usize = 0x35000;
    const VARIANT: usize = 0x36000;
    const VARIANT2: usize = 0x37000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{
            cair_wing, ccountry, cequipment_variant, cequipment_variant_pool, equipment_entry,
        };
        let mut bytes = vec![0; 0x40000];
        header(
            &mut bytes,
            GAME_STATE + engine::cgame_state::COUNTRIES,
            CTAB,
            3,
        );
        p64(&mut bytes, CTAB, 0);
        p64(&mut bytes, CTAB + 8, C0 as u64);
        p64(&mut bytes, CTAB + 16, C0 as u64);
        header(
            &mut bytes,
            GAME_STATE + engine::cgame_state::COUNTRY_LINKS,
            LTAB,
            3,
        );
        p32(&mut bytes, LTAB, 0);
        p32(&mut bytes, LTAB + 4, 1);
        p32(&mut bytes, LTAB + 8, 1);
        p64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, C0 + ccountry::ID, 1);
        p64(
            &mut bytes,
            GAME_STATE + engine::cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        p64(
            &mut bytes,
            MANAGER,
            engine::cstrategic_air_manager::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            MANAGER + engine::cstrategic_air_manager::COUNTRY_AIR,
            0x38000,
            2,
        );
        p64(&mut bytes, 0x38000, 0);
        p64(&mut bytes, 0x38000 + 8, OWNER_AIR as u64);
        p64(
            &mut bytes,
            OWNER_AIR,
            engine::cstrategic_air::VTABLE_RVA as u64,
        );
        p32(&mut bytes, OWNER_AIR + engine::cstrategic_air::COUNTRY, 1);
        header(
            &mut bytes,
            OWNER_AIR + engine::cstrategic_air::POOLS,
            0x38100,
            1,
        );
        p64(&mut bytes, 0x38100, POOL as u64);
        p64(&mut bytes, POOL, engine::cair_wing_pool::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            POOL + engine::cair_wing_pool::OWNER,
            OWNER_AIR as u64,
        );
        p64(
            &mut bytes,
            POOL + engine::cair_wing_pool::AIR_BASE_REFERENCE,
            0x1234u64,
        );
        header(&mut bytes, POOL + engine::cair_wing_pool::WINGS, 0x38200, 2);
        p64(&mut bytes, 0x38200, WING as u64);
        p64(&mut bytes, 0x38200 + 8, WING2 as u64);
        for (wing, id, accidents, exp) in [
            (WING, 0x4500_0001u64, 3i32, 1000i64),
            (WING2, 0x4500_0002u64, -5i32, i64::MAX),
        ] {
            p64(&mut bytes, wing, cair_wing::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                wing + cair_wing::SECONDARY,
                cair_wing::SECONDARY_VTABLE_RVA as u64,
            );
            p64(&mut bytes, wing + cair_wing::SECONDARY + 8, id);
            p64(&mut bytes, wing + cair_wing::POOL, POOL as u64);
            p64(&mut bytes, wing + cair_wing::BASE_OVERRIDE, 0);
            p32(&mut bytes, wing + cair_wing::PLANE_COUNT, 10);
            p32(&mut bytes, wing + cair_wing::EFFECTIVE_PLANES, 8);
            p32(
                &mut bytes,
                wing + cair_wing::MISSION + engine::air_mission::ENABLED_MASK,
                0xff,
            );
            p64(
                &mut bytes,
                wing + cair_wing::MISSION + engine::air_mission::REGION,
                0,
            );
            p32(&mut bytes, wing + cair_wing::COUNTRY_OVERRIDE, 0);
            p32(
                &mut bytes,
                wing + cair_wing::AIR_ACCIDENTS,
                accidents as u32,
            );
            p64(&mut bytes, wing + cair_wing::EXPERIENCE, exp as u64);
            // Equipment pool: two entries on WING, one on WING2.
            let (items, entries) = if wing == WING {
                (0x39000usize, vec![(VARIANT, 100i64), (VARIANT2, -50i64)])
            } else {
                (0x39100usize, vec![(VARIANT, 7i64)])
            };
            let pool = wing + cair_wing::EQUIPMENT_POOL;
            p64(&mut bytes, pool, cequipment_variant_pool::VTABLE_RVA as u64);
            header(
                &mut bytes,
                pool + cequipment_variant_pool::ITEMS,
                items,
                entries.len() as u32,
            );
            for (index, (variant, amount)) in entries.into_iter().enumerate() {
                p64(
                    &mut bytes,
                    items + index * equipment_entry::STRIDE,
                    variant as u64,
                );
                p64(
                    &mut bytes,
                    items + index * equipment_entry::STRIDE + 8,
                    amount as u64,
                );
            }
        }
        for (variant, id) in [(VARIANT, 11u32), (VARIANT2, 12u32)] {
            p64(&mut bytes, variant, cequipment_variant::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                variant + 8,
                ((id as u64) << 32) | cequipment_variant::REFERENCE_TYPE as u64,
            );
        }
        // Alias ENTRIES to the first wing's item vector for the fault test.
        let _ = ENTRIES;
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub equipment_outcome: Outcome,
        pub equipment: Vec<replay::rows::WingEquipmentRow>,
        pub cumulative_outcome: Outcome,
        pub cumulative: Vec<engine::cair_wing::CumulativeRow>,
    }

    fn collect_typed(memory: &impl Memory, capacity: &Capacities) -> Split {
        use engine::cair_wing;
        let mut captured = Captured::with_capacities(capacity, &[0; 7]);
        // Typed reference: wing walk with visitor into pooled buffers,
        // exactly like the game thread's Sources::air sequence.
        captured.begin_wings();
        let needed = [true, false, true, false, false, false, false];
        // Re-walk collecting (pointer,row) for the visitor (typed, no raw).
        let mut collected: Vec<(usize, cair_wing::Row)> = Vec::new();
        let mut wing_rows = vec![cair_wing::Row::default(); 8192];
        let outcome = cair_wing::walk_with(
            memory,
            BASE,
            GAME_STATE,
            &mut wing_rows,
            |pointer, _, row| {
                collected.push((pointer, *row));
            },
        );
        for (pointer, row) in &collected {
            let source = super::WingSource {
                pointer: *pointer,
                row,
                second: false,
            };
            captured.visit_wing(memory, BASE, source, &needed);
        }
        captured.finish_wings(outcome, None, &needed);
        let _ = outcome;
        Split {
            equipment_outcome: captured.equipment.outcome,
            equipment: captured.equipment.rows.clone(),
            cumulative_outcome: captured.cumulative.outcome,
            cumulative: captured.cumulative.rows.clone(),
        }
    }

    fn collect(memory: &impl Memory) -> Split {
        collect_typed(memory, &Capacities::default())
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect(memory)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, AirWingCopyReport, AirWingJob) {
        let ctx = AirWingCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 2],
        };
        let mut first = AirWingJob::new();
        let mut second = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; 7]);
        let mut ticks = [0u64; 2];
        super::decode_air_payload(&mut captured, BASE, &first, &mut ticks);
        (
            Split {
                equipment_outcome: captured.equipment.outcome,
                equipment: captured.equipment.rows.clone(),
                cumulative_outcome: captured.cumulative.outcome,
                cumulative: captured.cumulative.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct_with(memory: &impl Memory, capacity: &Capacities) -> Split {
        collect_typed(memory, capacity)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        capacity: &Capacities,
    ) -> (Split, AirWingCopyReport, AirWingJob) {
        let ctx = AirWingCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 2],
        };
        let mut first = AirWingJob::new();
        let mut second = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity, &[0; 7]);
        let mut ticks = [0u64; 2];
        super::decode_air_payload(&mut captured, BASE, &first, &mut ticks);
        (
            Split {
                equipment_outcome: captured.equipment.outcome,
                equipment: captured.equipment.rows.clone(),
                cumulative_outcome: captured.cumulative.outcome,
                cumulative: captured.cumulative.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct_truncated(memory: &impl Memory) -> Split {
        let capacity = Capacities {
            rows: [1, 2048, 1, 8192, 16384, 1024, 2048],
            ace_names: 65536,
        };
        collect_typed(memory, &capacity)
    }

    pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, AirWingCopyReport, AirWingJob) {
        let ctx = AirWingCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 2],
        };
        let mut first = AirWingJob::new();
        let mut second = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let capacity = Capacities {
            rows: [1, 2048, 1, 8192, 16384, 1024, 2048],
            ace_names: 65536,
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; 7]);
        let mut ticks = [0u64; 2];
        super::decode_air_payload(&mut captured, BASE, &first, &mut ticks);
        (
            Split {
                equipment_outcome: captured.equipment.outcome,
                equipment: captured.equipment.rows.clone(),
                cumulative_outcome: captured.cumulative.outcome,
                cumulative: captured.cumulative.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(
            split.equipment_outcome, direct.equipment_outcome,
            "equipment outcome"
        );
        assert_eq!(split.equipment, direct.equipment, "equipment rows");
        assert_eq!(
            split.cumulative_outcome, direct.cumulative_outcome,
            "cumulative outcome"
        );
        assert_eq!(split.cumulative, direct.cumulative, "cumulative rows");
    }

    /// Decode-worker assertion against pooled buffers (same module, so
    /// private rows are visible). Compares outcomes and rows with the typed
    /// reference without cloning through public views.
    pub(crate) fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
        assert_eq!(
            captured.equipment.outcome, direct.equipment_outcome,
            "equipment outcome"
        );
        assert_eq!(
            &captured.equipment.rows, &direct.equipment,
            "equipment rows"
        );
        assert_eq!(
            captured.cumulative.outcome, direct.cumulative_outcome,
            "cumulative outcome"
        );
        assert_eq!(
            &captured.cumulative.rows, &direct.cumulative,
            "cumulative rows"
        );
    }
}

/// Shared synthetic address space for the air-statistics copy/decode
/// harness: one owner with a two-region history table, each region holding
/// one history with a valid yearly window, record and channel values.
/// Mirrors the engine statistics fixture shapes contiguously.
#[cfg(test)]
pub(crate) mod stats_test_kit {
    use super::{AirStatsCopyContext, AirStatsCopyReport, AirStatsJob, Capacities, Captured};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0x1400_0000;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    const COUNTRY: usize = 0x20000;
    const MANAGER: usize = 0x30000;
    const OWNER_TAB: usize = 0x31000;
    const AIR: usize = 0x40000;
    const HIST_TAB: usize = 0x41000;
    const HISTORY1: usize = 0x50000;
    const HISTORY2: usize = 0x50200;
    const WINDOW1: usize = 0x60000;
    const WINDOW2: usize = 0x60200;
    const SLOTS1: usize = 0x61000;
    const SLOTS2: usize = 0x61200;
    const RECORD1: usize = 0x70000;
    const RECORD2: usize = 0x70200;
    const VALUES1: usize = 0x80000;
    const VALUES2: usize = 0x81000;

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    fn history(
        bytes: &mut [u8],
        history: usize,
        window: usize,
        slots: usize,
        record: usize,
        values: usize,
        seed: i64,
    ) {
        use engine::{
            cloop_history, cloop_history::air_stats, loop_history_record_fields,
            loop_history_window,
        };
        p64(bytes, history, (BASE + cloop_history::VTABLE_RVA) as u64);
        p32(
            bytes,
            history + cloop_history::FIELD_COUNT,
            air_stats::FIELD_COUNT,
        );
        p64(
            bytes,
            history + cloop_history::SELF_REFERENCE,
            history as u64,
        );
        p64(bytes, history + cloop_history::WINDOW, window as u64);
        p64(
            bytes,
            window,
            (BASE + loop_history_window::VTABLE_RVA) as u64,
        );
        header(bytes, window + loop_history_window::RECORDS, slots, 5);
        p32(bytes, window + loop_history_window::CAPACITY, 5);
        p32(bytes, window + loop_history_window::CURRENT_SLOT, 2);
        bytes[window + loop_history_window::WRAPPED] = 1;
        p64(bytes, window + loop_history_window::HISTORY, history as u64);
        p64(
            bytes,
            window + loop_history_window::RECORD_FIELDS,
            record as u64,
        );
        for slot in 0..5 {
            p64(bytes, slots + slot * 8, 0);
        }
        p64(bytes, slots + 2 * 8, record as u64);
        p64(
            bytes,
            record + loop_history_record_fields::HISTORY,
            history as u64,
        );
        header(
            bytes,
            record + loop_history_record_fields::VALUES,
            values,
            air_stats::FIELD_COUNT,
        );
        for channel in 0..air_stats::FIELD_COUNT as usize {
            p64(bytes, values + channel * 8, (seed + channel as i64) as u64);
        }
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{ccountry, cgame_state, cstrategic_air, cstrategic_air_manager};
        let mut bytes = vec![0; 0x82000];
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 3);
        p64(&mut bytes, CTAB, 0);
        p64(&mut bytes, CTAB + 8, COUNTRY as u64);
        p64(&mut bytes, CTAB + 16, COUNTRY as u64);
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
        p32(&mut bytes, LTAB, 0);
        p32(&mut bytes, LTAB + 4, 1);
        p32(&mut bytes, LTAB + 8, 1);
        p32(&mut bytes, LTAB + 12, 1);
        p64(&mut bytes, COUNTRY, (BASE + ccountry::VTABLE_RVA) as u64);
        p32(&mut bytes, COUNTRY + ccountry::ID, 3);
        p64(
            &mut bytes,
            GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        p64(
            &mut bytes,
            MANAGER,
            (BASE + cstrategic_air_manager::VTABLE_RVA) as u64,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            OWNER_TAB,
            3,
        );
        p64(&mut bytes, OWNER_TAB, 0);
        p64(&mut bytes, OWNER_TAB + 8, AIR as u64);
        p64(&mut bytes, OWNER_TAB + 16, 0);
        p64(&mut bytes, AIR, (BASE + cstrategic_air::VTABLE_RVA) as u64);
        p32(&mut bytes, AIR + cstrategic_air::COUNTRY, 2);
        header(&mut bytes, AIR + cstrategic_air::HISTORY_TABLE, HIST_TAB, 2);
        p64(&mut bytes, HIST_TAB, HISTORY1 as u64);
        p64(&mut bytes, HIST_TAB + 8, HISTORY2 as u64);
        history(
            &mut bytes, HISTORY1, WINDOW1, SLOTS1, RECORD1, VALUES1, 1000,
        );
        history(
            &mut bytes, HISTORY2, WINDOW2, SLOTS2, RECORD2, VALUES2, 2000,
        );
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub outcome: Outcome,
        pub rows: Vec<engine::cloop_history::Row>,
    }

    fn collect_typed(memory: &impl Memory, capacity: &Capacities) -> Split {
        use engine::cloop_history;
        let mut captured = Captured::with_capacities(capacity, &[0; 7]);
        let walked = cloop_history::walk(memory, BASE, GAME_STATE, &mut captured.statistics.rows);
        captured.statistics.outcome = super::bounded(walked, captured.statistics.rows.len());
        Split {
            outcome: captured.statistics.outcome,
            rows: captured.statistics.rows.clone(),
        }
    }

    pub(crate) fn direct(memory: &impl Memory, capacity: &Capacities) -> Split {
        collect_typed(memory, capacity)
    }

    pub(crate) fn split(
        memory: &impl Memory,
        capacity: &Capacities,
    ) -> (Split, AirStatsCopyReport, AirStatsJob) {
        let ctx = AirStatsCopyContext {
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = AirStatsJob::new();
        let mut second = AirStatsJob::new();
        let report = super::copy_air_stats_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity, &[0; 7]);
        let mut ticks = [0u64; 1];
        super::decode_air_stats_payload(&mut captured, BASE, &first, &mut ticks);
        (
            Split {
                outcome: captured.statistics.outcome,
                rows: captured.statistics.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.outcome, direct.outcome, "statistics outcome");
        assert_eq!(split.rows, direct.rows, "statistics rows");
    }

    /// Decode-worker assertion against pooled buffers (same module, so
    /// private rows are visible).
    pub(crate) fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
        assert_eq!(
            captured.statistics.outcome, direct.outcome,
            "statistics outcome"
        );
        assert_eq!(&captured.statistics.rows, &direct.rows, "statistics rows");
    }
}

#[cfg(test)]
mod air_stats_memo_tests {
    use super::stats_test_kit as kit;
    use super::*;

    fn context(seq: u64) -> AirStatsMemoContext {
        AirStatsMemoContext {
            owner: 23,
            base: kit::BASE,
            seq,
            hour: 100 + seq as i32,
            force_full: false,
            validation_allowed: true,
        }
    }

    fn captured(rows: usize) -> Captured {
        let mut captured = Captured::with_capacities(
            &Capacities {
                rows: [0, 0, 0, 0, 0, 0, rows],
                ace_names: 0,
            },
            &[200; N],
        );
        captured.stats_raw_mode = true;
        captured
    }

    fn job() -> AirStatsJob {
        AirStatsJob {
            log: RawLog::new(1024, 64 * 1024),
            game_state: 0,
        }
    }

    fn stage(job: &mut AirStatsJob, memory: &impl Memory) {
        copy_stats_pass(job, memory, kit::BASE, kit::GAME_STATE, &mut [0]);
        assert!(!job.log.overflowed);
    }

    fn run(
        memo: &mut AirStatsMemo,
        job: &AirStatsJob,
        ctx: AirStatsMemoContext,
        rows: usize,
    ) -> (Captured, AirStatsMemoReport) {
        let mut captured = captured(rows);
        let report = memo.decode(&mut captured, job, ctx, &mut [0]);
        (captured, report)
    }

    fn assert_direct(captured: &Captured, bytes: &[u8]) {
        let mut direct = vec![cloop_history::Row::default(); captured.statistics.rows.len()];
        let outcome =
            cloop_history::walk(&kit::memory(bytes), kit::BASE, kit::GAME_STATE, &mut direct);
        assert_eq!(captured.statistics.outcome, outcome);
        assert_eq!(
            &captured.statistics.rows[..outcome.len()],
            &direct[..outcome.len()]
        );
    }

    #[test]
    fn stats_memo_witness_includes_every_field_and_earlier_alias_observation() {
        use crate::raw::Span;
        let mut log = RawLog::<1>::new(4, 64);
        log.bytes.extend_from_slice(&[10, 11, 12, 13, 20, 21]);
        log.spans.extend([
            Span {
                addr: 0x1000,
                len: 4,
                ok: true,
                mask: 1,
                start: 0,
                bulk: true,
            },
            Span {
                addr: 0x1001,
                len: 1,
                ok: true,
                mask: 1,
                start: 4,
                bulk: false,
            },
            Span {
                addr: 0x1001,
                len: 1,
                ok: true,
                mask: 1,
                start: 5,
                bulk: false,
            },
        ]);
        log.has_bulk = true;
        let mut memo = AirStatsMemo::default();
        memo.spans.clone_from(&log.spans);
        memo.bytes.extend_from_slice(log.bytes.as_ref());
        memo.has_bulk = true;
        assert!(memo.input_equal(&log));
        let edits: [fn(&mut Span); 6] = [
            |span| span.addr += 1,
            |span| span.len += 1,
            |span| span.ok = false,
            |span| span.mask ^= 2,
            |span| span.start += 1,
            |span| span.bulk = !span.bulk,
        ];
        for edit in edits {
            let saved = log.spans[0];
            edit(&mut log.spans[0]);
            assert!(!memo.input_equal(&log));
            log.spans[0] = saved;
        }
        for index in 0..log.bytes.len() {
            log.bytes[index] ^= 1;
            assert!(!memo.input_equal(&log), "byte {index}");
            log.bytes[index] ^= 1;
        }
        log.spans.swap(1, 2);
        assert!(!memo.input_equal(&log));
        log.spans.swap(1, 2);
        log.has_bulk = false;
        assert!(!memo.input_equal(&log));
        log.has_bulk = true;
        log.fault[0] = true;
        assert!(!memo.input_equal(&log));
        log.fault[0] = false;
        log.overflowed = true;
        assert!(!memo.input_equal(&log));
        log.overflowed = false;
        let spans = log.spans.clone();
        log.clear();
        log.bytes.extend_from_slice(&[99; 32]);
        log.clear();
        log.bytes.extend_from_slice(&memo.bytes);
        log.spans.extend_from_slice(&spans);
        log.has_bulk = true;
        assert!(memo.input_equal(&log), "dirty unused arena is not input");
        log.spans.pop();
        assert!(!memo.input_equal(&log));
    }

    #[test]
    fn stats_memo_fresh_equal_input_restores_independent_native_prefix() {
        let bytes = kit::fixture();
        let mut job = job();
        stage(&mut job, &kit::memory(&bytes));
        let mut memo = AirStatsMemo::default();
        let (mut first, report) = run(&mut memo, &job, context(1), 8);
        assert!(report.installed && !report.hit);
        assert_direct(&first, &bytes);
        first.statistics.rows.fill(cloop_history::Row::default());
        drop(first);
        stage(&mut job, &kit::memory(&bytes));
        let mut target = captured(8);
        target.statistics.second.push(cloop_history::Row {
            country: 999,
            channels: [123; 24],
            ..Default::default()
        });
        let second = target.statistics.second.clone();
        let report = memo.decode(&mut target, &job, context(2), &mut [0]);
        assert!(report.hit && report.compared && !report.installed);
        assert_direct(&target, &bytes);
        assert_eq!(target.statistics.second, second);
        assert!(!target.stats_raw_mode && !target.statistics.checked);
        assert_eq!(target.statistics.mismatches, 0);
    }

    #[test]
    fn stats_memo_fresh_counter_and_structural_bytes_miss_but_unread_channels_do_not() {
        let mut bytes = kit::fixture();
        let mut job = job();
        stage(&mut job, &kit::memory(&bytes));
        let head = job
            .log
            .spans
            .iter()
            .find(|span| span.len == 32 && !span.bulk)
            .unwrap()
            .addr;
        let values_header = job
            .log
            .spans
            .iter()
            .find(|span| {
                let start = span.start as usize;
                span.len == 16
                    && u64::from_le_bytes(job.log.bytes[start..start + 8].try_into().unwrap())
                        == head as u64
            })
            .unwrap()
            .addr;
        let mut memo = AirStatsMemo::default();
        assert!(run(&mut memo, &job, context(1), 8).1.installed);
        bytes[head..head + 8].copy_from_slice(&(-123_i64).to_le_bytes());
        stage(&mut job, &kit::memory(&bytes));
        let (changed, report) = run(&mut memo, &job, context(2), 8);
        assert!(report.compared && !report.hit && report.installed);
        assert_direct(&changed, &bytes);
        assert!(run(&mut memo, &job, context(3), 8).1.hit);

        // Capacity is validated raw input but not emitted. Equal rows alone
        // must not reuse the old invocation's raw witness.
        bytes[values_header + 8..values_header + 12].copy_from_slice(&90_u32.to_le_bytes());
        stage(&mut job, &kit::memory(&bytes));
        let (same_rows, report) = run(&mut memo, &job, context(4), 8);
        assert!(report.compared && !report.hit && report.installed);
        assert_eq!(same_rows.statistics.rows, changed.statistics.rows);
        assert_direct(&same_rows, &bytes);
        // Channel four is outside all nine recorded spans and all 24 outputs.
        bytes[head + 32..head + 40].copy_from_slice(&999_i64.to_le_bytes());
        stage(&mut job, &kit::memory(&bytes));
        let (unchanged, report) = run(&mut memo, &job, context(5), 8);
        assert!(report.hit);
        assert_direct(&unchanged, &bytes);
    }

    #[test]
    fn stats_memo_context_checks_capacity_and_admission_force_real_decode() {
        let bytes = kit::fixture();
        let mut job = job();
        for case in 0..13 {
            stage(&mut job, &kit::memory(&bytes));
            let mut memo = AirStatsMemo::default();
            assert!(run(&mut memo, &job, context(1), 8).1.installed);
            let mut ctx = context(2);
            let mut target = captured(8);
            match case {
                0 => ctx.owner += 1,
                1 => ctx.base += 1,
                2 => ctx.force_full = true,
                3 => ctx.seq += 1,
                4 => ctx.hour += 1,
                5 => ctx.validation_allowed = false,
                6 => target.statistics.rows.push(cloop_history::Row::default()),
                7 => target.statistics.checked = true,
                8 => target.statistics.mismatches = 1,
                9 => target.stats_raw_mode = false,
                10 => job.log.fault[0] = true,
                11 => job.log.overflowed = true,
                12 => job.game_state += 8,
                _ => unreachable!(),
            }
            let report = memo.decode(&mut target, &job, ctx, &mut [0]);
            assert!(!report.hit, "case {case}");
            if matches!(case, 5 | 7..=11) {
                assert!(!report.eligible && !report.installed);
                assert!(memo.key.is_none() && memo.previous.is_none());
            }
        }
    }

    #[test]
    fn stats_memo_faults_and_partial_results_cannot_seed() {
        struct Fault<'a> {
            inner: engine::memory::SliceMemory<'a>,
            byte: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if address <= self.byte && self.byte < address.saturating_add(out.len()) {
                    if let Some(first) = out.first_mut() {
                        *first = 0xab;
                    }
                    false
                } else {
                    self.inner.copy(address, out)
                }
            }
        }
        let bytes = kit::fixture();
        let mut job = job();
        stage(&mut job, &kit::memory(&bytes));
        let head = job
            .log
            .spans
            .iter()
            .find(|span| span.len == 32 && !span.bulk)
            .unwrap()
            .addr;
        for byte in [head, head + 31] {
            let mut memo = AirStatsMemo::default();
            stage(&mut job, &kit::memory(&bytes));
            assert!(run(&mut memo, &job, context(1), 8).1.installed);
            stage(
                &mut job,
                &Fault {
                    inner: kit::memory(&bytes),
                    byte,
                },
            );
            assert!(job.log.fault[0]);
            for seq in [2, 3] {
                let (captured, report) = run(&mut memo, &job, context(seq), 8);
                assert!(matches!(
                    captured.statistics.outcome,
                    Outcome::Unavailable { .. }
                ));
                assert!(!report.hit && !report.eligible && !report.installed);
            }
            stage(&mut job, &kit::memory(&bytes));
            assert!(run(&mut memo, &job, context(4), 8).1.installed);
            assert!(run(&mut memo, &job, context(5), 8).1.hit);
        }
        for rows in [0, 1] {
            let mut memo = AirStatsMemo::default();
            let (captured, report) = run(&mut memo, &job, context(1), rows);
            assert!(matches!(
                captured.statistics.outcome,
                Outcome::Truncated { .. }
            ));
            assert!(!report.hit && !report.installed && memo.key.is_none());
            assert!(run(&mut memo, &job, context(2), 8).1.installed);
            assert!(run(&mut memo, &job, context(3), 8).1.hit);
        }
    }

    #[test]
    fn stats_memo_capacity_is_one_retained_owned_snapshot_until_drop() {
        static LOCAL: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();
        let bytes = kit::fixture();
        let mut job = job();
        stage(&mut job, &kit::memory(&bytes));
        let mut memo = AirStatsMemo::on_tracker(&LOCAL);
        assert!(run(&mut memo, &job, context(1), 8).1.installed);
        assert_eq!(LOCAL.snapshot().current, memo.capacity());
        let before = memo.capacity();
        // An extra nonaliasing successful observation changes the whole witness
        // without changing rows; it forces real decode and snapshot growth.
        let start = job.log.bytes.len() as u32;
        job.log.bytes.extend_from_slice(&[7; 4096]);
        job.log.spans.push(crate::raw::Span {
            addr: 0x900000,
            len: 4096,
            ok: true,
            mask: 1,
            start,
            bulk: false,
        });
        job.log.raw_bytes[0] += 4096;
        assert!(run(&mut memo, &job, context(2), 8).1.installed);
        assert_eq!(LOCAL.snapshot().current, memo.capacity());
        assert!(memo.capacity().payload_vectors > before.payload_vectors);
        assert!(memo.capacity().auxiliary_vectors > 0);
        memo.invalidate();
        assert_eq!(LOCAL.snapshot().current, memo.capacity());
        let moved = Some(memo);
        assert!(LOCAL.snapshot().current.payload_vectors > 0);
        drop(moved);
        assert_eq!(
            LOCAL.snapshot().current,
            crate::raw_memory::Capacity::default()
        );
        assert!(LOCAL.snapshot().peak_total_lower_bound > 0);
    }
}

#[cfg(test)]
mod air_bases_memo_tests {
    use super::air_bases_test_kit as kit;
    use super::*;

    const CAPS: [usize; 5] = [8, 8, 256, 16, 64];

    fn context(seq: u64) -> AirBasesMemoContext {
        AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq,
            hour: 100 + seq as i32,
            force_full: false,
            validation_allowed: true,
        }
    }

    fn parts(cap: [usize; 5]) -> (Captured, super::super::fog::Captured) {
        let mut air = Captured::with_capacities(
            &Capacities {
                rows: [0, cap[0], 0, 0, 0, cap[1], 0],
                ace_names: cap[2],
            },
            &[200; N],
        );
        let mut fog = super::super::fog::Captured::with_capacities(
            &super::super::fog::Capacities {
                rows: [0, 0, 0, 0, 0, cap[3]],
                radar_provinces: 0,
                area_provinces: 0,
                wings: cap[4],
            },
            &[200; super::super::fog::N],
        );
        air.bases_raw_mode = true;
        fog.registrations_raw_mode = true;
        (air, fog)
    }

    fn stage(job: &mut AirBasesJob, bytes: &[u8]) {
        copy_air_bases_pass(
            job,
            &kit::memory(bytes),
            kit::BASE,
            kit::GAME_STATE,
            &mut [0; 3],
        );
        assert!(!job.log.overflowed);
        assert_eq!(job.log.fault, [false; 3]);
    }

    fn run(
        memo: &mut AirBasesMemo,
        job: &AirBasesJob,
        ctx: AirBasesMemoContext,
        capacities: [usize; 5],
    ) -> (Captured, super::super::fog::Captured, AirBasesMemoReport) {
        let (mut air, mut fog) = parts(capacities);
        let report = memo.decode(&mut air, &mut fog, job, ctx, &mut [0; 3]);
        (air, fog, report)
    }

    fn assert_rows(air: &Captured, fog: &super::super::fog::Captured, expected: &kit::Split) {
        assert_eq!(air.bases.outcome, expected.bases_outcome);
        assert_eq!(
            &air.bases.rows[..air.bases.outcome.len()],
            &expected.bases[..expected.bases_outcome.len()]
        );
        assert_eq!(air.aces.outcome, expected.aces_outcome);
        assert_eq!(
            &air.aces.rows[..air.aces.outcome.len()],
            &expected.aces[..expected.aces_outcome.len()]
        );
        assert_eq!(air.names.len, expected.ace_bytes_len);
        assert_eq!(air.names.required, expected.ace_required);
        assert_eq!(
            &air.names.first[..air.names.len],
            &expected.ace_names[..expected.ace_bytes_len]
        );
        let regs = fog.registrations_snapshot();
        assert_eq!(regs.outcome, expected.regs.outcome);
        assert_eq!(regs.rows, expected.regs.rows);
        assert_eq!(regs.wings, expected.regs.wings);
    }

    #[test]
    fn memo_compares_every_span_field_order_alias_and_only_used_bytes() {
        use crate::raw::Span;
        let mut log = RawLog::<3>::new(4, 64);
        // Bulk and exact aliases plus a second exact observation deliberately
        // have different bytes. A partition or last-value-only key is invalid.
        log.bytes.extend_from_slice(&[10, 11, 12, 13, 20, 21]);
        log.spans.extend([
            Span {
                addr: 0x1000,
                len: 4,
                ok: true,
                mask: 1,
                start: 0,
                bulk: true,
            },
            Span {
                addr: 0x1001,
                len: 1,
                ok: true,
                mask: 2,
                start: 4,
                bulk: false,
            },
            Span {
                addr: 0x1001,
                len: 1,
                ok: true,
                mask: 4,
                start: 5,
                bulk: false,
            },
        ]);
        log.has_bulk = true;
        let mut memo = AirBasesMemo::default();
        memo.spans.clone_from(&log.spans);
        memo.bytes.extend_from_slice(log.bytes.as_ref());
        memo.has_bulk = true;
        assert!(memo.input_equal(&log));
        let edits: [fn(&mut Span); 6] = [
            |span| span.addr += 1,
            |span| span.len += 1,
            |span| span.ok = false,
            |span| span.mask ^= 4,
            |span| span.start += 1,
            |span| span.bulk = !span.bulk,
        ];
        for edit in edits {
            let old = log.spans[0];
            edit(&mut log.spans[0]);
            assert!(!memo.input_equal(&log));
            log.spans[0] = old;
        }
        log.spans.swap(1, 2);
        assert!(!memo.input_equal(&log));
        log.spans.swap(1, 2);
        for index in 0..log.bytes.len() {
            log.bytes[index] ^= 1;
            assert!(!memo.input_equal(&log), "byte {index}");
            log.bytes[index] ^= 1;
        }
        log.has_bulk = false;
        assert!(!memo.input_equal(&log));
        log.has_bulk = true;
        for slot in 0..3 {
            log.fault[slot] = true;
            assert!(!memo.input_equal(&log));
            log.fault[slot] = false;
        }
        log.overflowed = true;
        assert!(!memo.input_equal(&log));
        log.overflowed = false;
        let spans = log.spans.clone();
        log.clear();
        log.bytes.extend_from_slice(&[99; 32]);
        log.clear(); // Dirty spare bytes must not invalidate equal used input.
        log.bytes.extend_from_slice(&memo.bytes);
        log.spans.extend_from_slice(&spans);
        log.has_bulk = true;
        assert!(memo.input_equal(&log));
        log.spans.pop();
        assert!(!memo.input_equal(&log));
    }

    #[test]
    fn memo_reuses_complete_current_capture_in_independently_owned_frame() {
        let mut bytes = kit::fixture();
        kit::add_alias_and_rocket(&mut bytes);
        let expected = kit::direct(&kit::memory(&bytes));
        let mut job = AirBasesJob::new();
        stage(&mut job, &bytes);
        let mut memo = AirBasesMemo::default();
        let (mut first, first_fog, report) = run(&mut memo, &job, context(1), CAPS);
        assert!(!report.hit && report.installed && report.eligible);
        assert_rows(&first, &first_fog, &expected);
        // Recycling or mutation of the old native frame cannot change the memo.
        first.bases.rows.fill(cair_base::Row::default());
        first.aces.rows.fill(cace::Row::default());
        first.names.first.fill(0);
        drop(first);
        drop(first_fog);
        stage(&mut job, &bytes); // Actual fresh owned observations, not old values.
        let (mut air, mut fog) = parts(CAPS);
        air.bases.second.push(cair_base::Row {
            id: 999,
            ..Default::default()
        });
        air.aces.second.push(cace::Row {
            id: 998,
            ..Default::default()
        });
        air.names.second.extend_from_slice(b"comparison");
        let report = memo.decode(&mut air, &mut fog, &job, context(2), &mut [0; 3]);
        assert!(report.hit && report.compared && !report.installed);
        assert_rows(&air, &fog, &expected);
        assert_eq!(air.bases.second[0].id, 999);
        assert_eq!(air.aces.second[0].id, 998);
        assert_eq!(air.names.second, b"comparison");
        assert!(!air.bases_raw_mode && !fog.registrations_raw_mode);
        assert!(!air.bases.checked && !air.aces.checked);
        assert_eq!(fog.checked_mismatches()[5], 0);
    }

    #[test]
    fn memo_changed_fresh_leaf_misses_and_installs_new_complete_result() {
        let mut bytes = kit::fixture();
        let mut job = AirBasesJob::new();
        stage(&mut job, &bytes);
        let mut memo = AirBasesMemo::default();
        assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
        kit::put32(&mut bytes, kit::B0 + cair_base::CAPACITY, 123);
        stage(&mut job, &bytes);
        let (air, fog, report) = run(&mut memo, &job, context(2), CAPS);
        assert!(report.compared && !report.hit && report.installed);
        assert_rows(&air, &fog, &kit::direct(&kit::memory(&bytes)));
        assert!(run(&mut memo, &job, context(3), CAPS).2.hit);
    }

    #[test]
    fn memo_declines_current_fault_check_mismatch_and_admission_states() {
        let bytes = kit::fixture();
        let mut job = AirBasesJob::new();
        for case in 0..10 {
            stage(&mut job, &bytes);
            let mut memo = AirBasesMemo::default();
            assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
            let (mut air, mut fog) = parts(CAPS);
            match case {
                0 => job.log.overflowed = true,
                1 => job.log.fault[0] = true,
                2 => job.log.spans[0].ok = false,
                3 => air.bases.checked = true,
                4 => air.aces.checked = true,
                5 => fog.note_registrations_copy(true, true),
                6 => air.bases.mismatches = 1,
                7 => air.aces.mismatches = 1,
                8 => fog.note_registrations_copy(true, false),
                9 => air.bases_raw_mode = false,
                _ => unreachable!(),
            }
            let report = memo.decode(&mut air, &mut fog, &job, context(2), &mut [0; 3]);
            assert!(
                !report.hit && !report.installed && !report.eligible,
                "case {case}"
            );
            assert!(memo.key.is_none() && memo.previous.is_none());
            if case == 5 {
                assert_eq!(fog.checked_mismatches()[5], 0);
            }
        }
    }

    #[test]
    fn memo_context_capacity_branch_and_missing_refresh_never_reuse_old_entry() {
        let bytes = kit::fixture();
        let mut job = AirBasesJob::new();
        stage(&mut job, &bytes);
        for case in 0..11 {
            let mut memo = AirBasesMemo::default();
            assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
            let mut ctx = context(2);
            let mut cap = CAPS;
            match case {
                0 => ctx.owner += 1,
                1 => ctx.base += 1,
                2 => ctx.force_full = true,
                3 => ctx.hour += 1,
                4 => ctx.seq += 1,
                5..=9 => cap[case - 5] += 1,
                10 => memo.invalidate(), // Missing staging/typed fallback adapter.
                _ => unreachable!(),
            }
            assert!(!run(&mut memo, &job, ctx, cap).2.hit, "case {case}");
        }
        let mut memo = AirBasesMemo::default();
        assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
        job.game_state += 8;
        assert!(!run(&mut memo, &job, context(2), CAPS).2.hit);
    }

    #[test]
    fn memo_incomplete_capacity_result_cannot_seed_and_growth_requires_decode() {
        let bytes = kit::fixture();
        let mut job = AirBasesJob::new();
        stage(&mut job, &bytes);
        for slot in 0..5 {
            let mut memo = AirBasesMemo::default();
            let mut cap = CAPS;
            cap[slot] = 0;
            let report = run(&mut memo, &job, context(1), cap).2;
            assert!(!report.hit && !report.installed, "capacity slot {slot}");
            assert!(memo.key.is_none());
            let report = run(&mut memo, &job, context(2), CAPS).2;
            assert!(!report.hit && report.installed);
            assert!(run(&mut memo, &job, context(3), CAPS).2.hit);
        }
    }

    #[test]
    fn memo_capacity_accounting_follows_growth_invalidation_move_and_drop() {
        static LOCAL: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();
        let mut bytes = kit::fixture();
        let mut job = AirBasesJob::new();
        stage(&mut job, &bytes);
        let mut memo = AirBasesMemo::on_tracker(&LOCAL);
        assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
        assert_eq!(LOCAL.snapshot().current, memo.capacity());
        assert!(memo.capacity().payload_vectors > 0 && memo.capacity().auxiliary_vectors > 0);
        let before = memo.capacity();
        kit::add_alias_and_rocket(&mut bytes);
        stage(&mut job, &bytes);
        assert!(run(&mut memo, &job, context(2), CAPS).2.installed);
        assert_eq!(LOCAL.snapshot().current, memo.capacity());
        assert!(memo.capacity().payload_vectors >= before.payload_vectors);
        memo.invalidate();
        assert_eq!(LOCAL.snapshot().current, memo.capacity());
        let moved = Some(memo);
        assert!(LOCAL.snapshot().current.payload_vectors > 0);
        drop(moved);
        assert_eq!(
            LOCAL.snapshot().current,
            crate::raw_memory::Capacity::default()
        );
        assert!(LOCAL.snapshot().peak_total_lower_bound > 0);
    }
}

/// Shared synthetic address space for the bases/registrations/aces
/// copy/decode harness: two countries, a manager with two state bases
/// (one indexed hole) plus one stateless host, per-base containers with
/// pools and wings, and two aces (one alive with inline names, one dead
/// with heap names). Mirrors the engine bases/registrations/aces fixture
/// shapes.
#[cfg(test)]
pub(crate) mod air_bases_test_kit {
    use super::{AirBasesCopyContext, AirBasesCopyReport, AirBasesJob, Capacities, Captured};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    pub(crate) const LTAB: usize = 0x12000;
    pub(crate) const C0: usize = 0x20000;
    const C1: usize = 0x21000;
    pub(crate) const MANAGER: usize = 0x30000;
    pub(crate) const SVEC: usize = 0x31000;
    const HVEC: usize = 0x32000;
    pub(crate) const B0: usize = 0x40000;
    const B1: usize = 0x41000;
    const H0: usize = 0x42000;
    pub(crate) const S0: usize = 0x50000;
    const S1: usize = 0x50100;
    const P0: usize = 0x51000;
    const P1: usize = 0x51100;
    const CVEC0: usize = 0x60000;
    pub(crate) const CONT1: usize = 0x61000;
    const POOLVEC0: usize = 0x61400;
    pub(crate) const POOL0: usize = 0x62000;
    const WVEC: usize = 0x62400;
    pub(crate) const W0: usize = 0x63000;
    const W1: usize = 0x63100;
    const CVEC1: usize = 0x64000;
    const CONT2: usize = 0x65000;
    const POOLVEC1: usize = 0x65400;
    const ACES0: usize = 0x70000;
    pub(crate) const A0: usize = 0x71000;
    const A1: usize = 0x72000;
    pub(crate) const NSTR: usize = 0x73000;
    const RVEC2: usize = 0x32100;
    const B2: usize = 0x43000;
    const S2: usize = 0x50200;
    const P2: usize = 0x51200;
    const CVEC2: usize = 0x66000;

    pub(crate) const WING_HANDLE_0: u64 = (0x77 << 32) | engine::cair_wing::TYPE as u64;
    pub(crate) const WING_HANDLE_1: u64 = (0x78 << 32) | engine::cair_wing::TYPE as u64;

    pub(crate) fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        fixture_inner()
    }

    /// Alias country slot plus a rocket-vector base: exercises the census
    /// alias skip and a non-state base vector with the same shared takes.
    pub(crate) fn add_alias_and_rocket(bytes: &mut [u8]) {
        use engine::{cair_base, cgame_state, cprovince, cstate, cstrategic_air_manager};
        // Alias slot 3 publishing the slot-1 object once: its link entry
        // resolves to slot 1 like its id does.
        header(bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
        put64(bytes, CTAB + 24, C0 as u64);
        put32(bytes, LTAB + 12, 1);
        // Rocket vector with a hole plus one state base at slot 1.
        header(
            bytes,
            MANAGER + cstrategic_air_manager::STATE_ROCKET_BASES,
            RVEC2,
            2,
        );
        put64(bytes, RVEC2, 0);
        put64(bytes, RVEC2 + 8, B2 as u64);
        put64(bytes, B2, cair_base::VTABLE_RVA as u64);
        put64(
            bytes,
            B2 + cair_base::HANDLE,
            (0x44u64 << 32) | u64::from(cair_base::TYPE),
        );
        put64(bytes, B2 + cair_base::HOST_REFERENCE, 0);
        put64(bytes, B2 + cair_base::HOST_REFERENCE + 8, S2 as u64);
        put64(bytes, B2 + cair_base::HOST_REFERENCE + 16, P2 as u64);
        put32(bytes, B2 + cair_base::HOST_REFERENCE + 24, 42);
        put32(bytes, B2 + cair_base::HOST_REFERENCE + 28, 0);
        put64(bytes, S2, cstate::VTABLE_RVA as u64);
        put32(bytes, S2 + cstate::ID, 1);
        put64(bytes, P2, cprovince::VTABLE_RVA as u64);
        put32(bytes, P2 + cprovince::ID, 7);
        put64(bytes, P2 + cprovince::STATE, S2 as u64);
        header(bytes, B2 + cair_base::COUNTRY_CONTAINERS, CVEC2, 0);
    }

    fn fixture_inner() -> Vec<u8> {
        use engine::{
            cair_base, cair_wing, cair_wing_pool, ccountry, ccountry_air_container, cgame_state,
            cprovince, cstate, cstrategic_air_manager, msvc_string,
        };
        let mut bytes = vec![0; 0x80000];
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 3);
        put64(&mut bytes, CTAB, 0);
        put64(&mut bytes, CTAB + 8, C0 as u64);
        put64(&mut bytes, CTAB + 16, C1 as u64);
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
        put32(&mut bytes, LTAB, 0);
        put32(&mut bytes, LTAB + 4, 1);
        put32(&mut bytes, LTAB + 8, 2);
        put32(&mut bytes, LTAB + 12, 0);
        for (country, id) in [(C0, 1u32), (C1, 2u32)] {
            put64(&mut bytes, country, ccountry::VTABLE_RVA as u64);
            put32(&mut bytes, country + ccountry::ID, id);
        }
        header(&mut bytes, C0 + ccountry::ACES, ACES0, 2);
        put64(&mut bytes, ACES0, A0 as u64);
        put64(&mut bytes, ACES0 + 8, A1 as u64);
        header(&mut bytes, C1 + ccountry::ACES, 0, 0);
        // Manager with two state bases (indexed hole at 0) and one host.
        put64(
            &mut bytes,
            GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        put64(
            &mut bytes,
            MANAGER,
            cstrategic_air_manager::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::STATE_AIR_BASES,
            SVEC,
            3,
        );
        put64(&mut bytes, SVEC, 0);
        put64(&mut bytes, SVEC + 8, B0 as u64);
        put64(&mut bytes, SVEC + 16, B1 as u64);
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::STATE_ROCKET_BASES,
            0,
            0,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::STATE_GUN_BASES,
            0,
            0,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
            HVEC,
            1,
        );
        put64(&mut bytes, HVEC, H0 as u64);
        // State bases with authenticated state/province links.
        for (base, id, state, state_id, province, province_id, capacity, kind) in [
            (B0, 0x41u64, S0, 1u32, P0, 5u32, 1200i32, 0i32),
            (B1, 0x42u64, S1, 2u32, P1, 6u32, -3i32, 1i32),
        ] {
            put64(&mut bytes, base, cair_base::VTABLE_RVA as u64);
            put64(
                &mut bytes,
                base + cair_base::HANDLE,
                (id << 32) | u64::from(cair_base::TYPE),
            );
            put64(&mut bytes, base + cair_base::HOST_REFERENCE, 0);
            put64(
                &mut bytes,
                base + cair_base::HOST_REFERENCE + 8,
                state as u64,
            );
            put64(
                &mut bytes,
                base + cair_base::HOST_REFERENCE + 16,
                province as u64,
            );
            put32(
                &mut bytes,
                base + cair_base::HOST_REFERENCE + 24,
                capacity as u32,
            );
            put32(
                &mut bytes,
                base + cair_base::HOST_REFERENCE + 28,
                kind as u32,
            );
            put64(&mut bytes, state, cstate::VTABLE_RVA as u64);
            put32(&mut bytes, state + cstate::ID, state_id);
            put64(&mut bytes, province, cprovince::VTABLE_RVA as u64);
            put32(&mut bytes, province + cprovince::ID, province_id);
            put64(&mut bytes, province + cprovince::STATE, state as u64);
        }
        // Stateless host with a native host reference.
        put64(&mut bytes, H0, cair_base::VTABLE_RVA as u64);
        put64(
            &mut bytes,
            H0 + cair_base::HANDLE,
            (0x43u64 << 32) | u64::from(cair_base::TYPE),
        );
        put64(
            &mut bytes,
            H0 + cair_base::HOST_REFERENCE,
            (3u64 << 32) | 51,
        );
        put64(&mut bytes, H0 + cair_base::HOST_REFERENCE + 8, 0);
        put64(&mut bytes, H0 + cair_base::HOST_REFERENCE + 16, 0);
        put32(
            &mut bytes,
            H0 + cair_base::HOST_REFERENCE + 24,
            (-17i32) as u32,
        );
        put32(&mut bytes, H0 + cair_base::HOST_REFERENCE + 28, 1);
        header(&mut bytes, H0 + cair_base::COUNTRY_CONTAINERS, 0, 0);
        // B0 containers: hole at slot 0 plus one country-1 container with a
        // two-wing pool. B1 containers: hole plus one empty country-2 pool.
        header(&mut bytes, B0 + cair_base::COUNTRY_CONTAINERS, CVEC0, 2);
        put64(&mut bytes, CVEC0, 0);
        put64(&mut bytes, CVEC0 + 8, CONT1 as u64);
        // Country-2's container sits at its canonical slot 2: the engine
        // installs each container at its resolved ordinal.
        header(&mut bytes, B1 + cair_base::COUNTRY_CONTAINERS, CVEC1, 3);
        put64(&mut bytes, CVEC1, 0);
        put64(&mut bytes, CVEC1 + 8, 0);
        put64(&mut bytes, CVEC1 + 16, CONT2 as u64);
        for (container, country, poolvec, pools) in
            [(CONT1, 1u32, POOLVEC0, 1u32), (CONT2, 2u32, POOLVEC1, 0u32)]
        {
            put64(
                &mut bytes,
                container,
                ccountry_air_container::VTABLE_RVA as u64,
            );
            put32(
                &mut bytes,
                container + ccountry_air_container::COUNTRY,
                country,
            );
            header(
                &mut bytes,
                container + ccountry_air_container::POOLS,
                poolvec,
                pools,
            );
        }
        put64(&mut bytes, POOLVEC0, POOL0 as u64);
        put64(&mut bytes, POOL0, cair_wing_pool::VTABLE_RVA as u64);
        header(&mut bytes, POOL0 + cair_wing_pool::WINGS, WVEC, 2);
        put64(&mut bytes, WVEC, W0 as u64);
        put64(&mut bytes, WVEC + 8, W1 as u64);
        for (wing, handle) in [(W0, WING_HANDLE_0), (W1, WING_HANDLE_1)] {
            put64(&mut bytes, wing, cair_wing::VTABLE_RVA as u64);
            put64(&mut bytes, wing + 8, POOL0 as u64);
            put64(
                &mut bytes,
                wing + 16,
                cair_wing::SECONDARY_VTABLE_RVA as u64,
            );
            put64(&mut bytes, wing + 24, handle);
        }
        // Aces: A0 alive with inline names, A1 dead with heap names.
        ace(
            &mut bytes,
            A0,
            C0,
            0xA0,
            1,
            1,
            7,
            None,
            [("Al", true), ("Bo", true), ("Cy", true)],
        );
        ace(
            &mut bytes,
            A1,
            C0,
            0xA1,
            0,
            0,
            3,
            Some((0xB0u64, 9u32)),
            [("DeadAce", false), ("", false), ("Z", false)],
        );
        // Heap name bytes with terminators.
        put_string(&mut bytes, NSTR, b"DeadAce");
        put_string(&mut bytes, NSTR + 8, b"");
        put_string(&mut bytes, NSTR + 9, b"Z");
        let _ = msvc_string::INLINE_CAPACITY;
        bytes
    }

    fn ace(
        bytes: &mut [u8],
        at: usize,
        owner: usize,
        id: u64,
        alive: u8,
        handled: u8,
        kill_type: u32,
        killer: Option<(u64, u32)>,
        names: [(&str, bool); 3],
    ) {
        #![allow(clippy::too_many_arguments)]
        use engine::{cace, msvc_string};
        put64(bytes, at, cace::VTABLE_RVA as u64);
        put64(bytes, at + cace::REFERENCE, (id << 32) | 0xace);
        put32(bytes, at + cace::AIRCRAFT_TYPE_MASK, 0xF);
        put64(bytes, at + cace::OWNER_COUNTRY, owner as u64);
        put64(bytes, at + cace::OWNER_COUNTRY + 8, WING_HANDLE_0);
        for (index, (text, inline)) in names.into_iter().enumerate() {
            let head = at + [cace::NAME, cace::SURNAME, cace::CALLSIGN][index];
            if inline {
                bytes[head..head + text.len()].copy_from_slice(text.as_bytes());
                bytes[head + text.len()] = 0;
                put64(bytes, head + msvc_string::LENGTH, text.len() as u64);
                put64(
                    bytes,
                    head + msvc_string::CAPACITY,
                    msvc_string::INLINE_CAPACITY,
                );
            } else {
                // Heap strings live consecutively from NSTR in field order.
                let source = NSTR + heap_offset(index);
                put64(bytes, head + msvc_string::DATA, source as u64);
                put64(bytes, head + msvc_string::LENGTH, text.len() as u64);
                put64(bytes, head + msvc_string::CAPACITY, 16);
            }
        }
        bytes[at + cace::ALIVE] = alive;
        put32(bytes, at + cace::KILL_TYPE, kill_type);
        if let Some((killer_ace, killer_country)) = killer {
            put64(bytes, at + cace::KILLER_ACE_REFERENCE, killer_ace);
            put32(bytes, at + cace::KILLER_COUNTRY, killer_country);
        }
        bytes[at + cace::HANDLED] = handled;
    }

    fn heap_offset(index: usize) -> usize {
        // NSTR layout: "DeadAce\0" (8) + "" (1) + "Z\0" (2).
        [0, 8, 9][index]
    }

    fn put_string(bytes: &mut [u8], at: usize, text: &[u8]) {
        bytes[at..at + text.len()].copy_from_slice(text);
        bytes[at + text.len()] = 0;
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub bases_outcome: Outcome,
        pub bases: Vec<engine::cair_base::Row>,
        pub aces_outcome: Outcome,
        pub aces: Vec<engine::cace::Row>,
        pub ace_names: Vec<u8>,
        pub ace_bytes_len: usize,
        pub ace_required: usize,
        pub regs: crate::extended::fog::RegistrationsSnapshot,
    }

    fn collect_typed(
        memory: &impl Memory,
        air_rows: [usize; 7],
        ace_names: usize,
        fog_rows: usize,
        fog_wings: usize,
    ) -> Split {
        use engine::{cace, cair_base};
        let capacities = Capacities {
            rows: air_rows,
            ace_names,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; 7]);
        let walked = cair_base::walk(memory, BASE, GAME_STATE, &mut captured.bases.rows);
        captured.bases.outcome = super::bounded(walked, captured.bases.rows.len());
        let walked = cace::walk(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.aces.rows,
            &mut captured.names.first,
        );
        captured.aces.outcome =
            super::names_outcome(walked, &captured.aces.rows, &captured.names.first);
        captured.names.len = walked.bytes_len;
        captured.names.required = walked.required_bytes;
        let regs = crate::extended::fog::Captured::collect_registrations_direct(
            fog_rows, fog_wings, memory, BASE, GAME_STATE,
        );
        Split {
            bases_outcome: captured.bases.outcome,
            bases: captured.bases.rows.clone(),
            aces_outcome: captured.aces.outcome,
            aces: captured.aces.rows.clone(),
            ace_names: captured.names.first.clone(),
            ace_bytes_len: captured.names.len,
            ace_required: captured.names.required,
            regs,
        }
    }

    fn collect_split(
        memory: &impl Memory,
        air_rows: [usize; 7],
        ace_names: usize,
        fog_rows: usize,
        fog_wings: usize,
    ) -> (Split, AirBasesCopyReport, AirBasesJob) {
        let ctx = AirBasesCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 3],
        };
        let capacities = Capacities {
            rows: air_rows,
            ace_names,
        };
        let mut air = Captured::with_capacities(&capacities, &[0; 7]);
        let mut fog = crate::extended::fog::Captured::with_capacities(
            &crate::extended::fog::Capacities {
                rows: [4, 4, 4, 4, 4, fog_rows],
                radar_provinces: 8,
                area_provinces: 8,
                wings: fog_wings,
            },
            &[0; 6],
        );
        let mut first = AirBasesJob::new();
        let mut second = AirBasesJob::new();
        let report =
            air.copy_air_bases_refresh(memory, ctx, &mut fog, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 3];
        air.decode_air_bases_payload(BASE, &first, &mut ticks);
        fog.decode_registrations_payload(BASE, &first, &mut ticks);
        (
            Split {
                bases_outcome: air.bases.outcome,
                bases: air.bases.rows.clone(),
                aces_outcome: air.aces.outcome,
                aces: air.aces.rows.clone(),
                ace_names: air.names.first.clone(),
                ace_bytes_len: air.names.len,
                ace_required: air.names.required,
                regs: fog.registrations_snapshot(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect_typed(memory, Capacities::default().rows, 65_536, 2048, 4096)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, AirBasesCopyReport, AirBasesJob) {
        collect_split(memory, Capacities::default().rows, 65_536, 2048, 4096)
    }

    pub(crate) fn direct_with(
        memory: &impl Memory,
        bases: usize,
        wings: usize,
        aces: usize,
        names: usize,
    ) -> Split {
        collect_typed(
            memory,
            [8192, bases, 4096, 8192, 16384, aces, 2048],
            names,
            bases.max(2),
            wings,
        )
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        bases: usize,
        wings: usize,
        aces: usize,
        names: usize,
    ) -> (Split, AirBasesCopyReport, AirBasesJob) {
        collect_split(
            memory,
            [8192, bases, 4096, 8192, 16384, aces, 2048],
            names,
            bases.max(2),
            wings,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.bases_outcome, direct.bases_outcome, "bases outcome");
        assert_eq!(split.bases, direct.bases, "bases rows");
        assert_eq!(split.aces_outcome, direct.aces_outcome, "aces outcome");
        assert_eq!(split.aces, direct.aces, "aces rows");
        assert_eq!(split.ace_bytes_len, direct.ace_bytes_len, "aces bytes len");
        assert_eq!(split.ace_required, direct.ace_required, "aces required");
        assert_eq!(
            split.ace_names[..split.ace_bytes_len],
            direct.ace_names[..direct.ace_bytes_len],
            "aces names"
        );
        assert_eq!(
            split.regs.outcome, direct.regs.outcome,
            "registrations outcome"
        );
        assert_eq!(split.regs.rows, direct.regs.rows, "registrations rows");
        assert_eq!(split.regs.wings, direct.regs.wings, "registrations wings");
    }
}

/// Shared synthetic address space for the history/damage copy/decode
/// harness: one owner with two region slots (one null), three records with
/// damage vectors on two of them. Mirrors the engine region-combat fixture
/// shapes.
#[cfg(test)]
pub(crate) mod air_history_test_kit {
    use super::{AirHistoryCopyContext, AirHistoryCopyReport, AirHistoryJob, Capacities, Captured};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    const C0: usize = 0x20000;
    const C1: usize = 0x21000;
    pub(crate) const MANAGER: usize = 0x30000;
    const AIR_TAB: usize = 0x30800;
    const AIR0: usize = 0x31000;
    const RVEC: usize = 0x31800;
    pub(crate) const R0: usize = 0x32000;
    const FVEC: usize = 0x32800;
    const SVEC2: usize = 0x32A00;
    // Records live inline in their vectors (stride 0x98): two in FVEC, one
    // in SVEC2. Damage entries live inline in theirs (stride 0x18).
    pub(crate) const REC0: usize = 0x32800;
    const REC1: usize = 0x32898;
    const REC2: usize = 0x32A00;
    const DV0: usize = 0x34000;
    const DV1: usize = 0x34200;
    pub(crate) const E0: usize = 0x34000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{
            cair_region_combat_data, ccountry, cgame_state, cstrategic_air, cstrategic_air_manager,
            sair_wing_combat_data, sdamage_entry,
        };
        let mut bytes = vec![0; 0x40000];
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 3);
        p64(&mut bytes, CTAB, 0);
        p64(&mut bytes, CTAB + 8, C0 as u64);
        p64(&mut bytes, CTAB + 16, C1 as u64);
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
        p32(&mut bytes, LTAB, 0);
        p32(&mut bytes, LTAB + 4, 1);
        p32(&mut bytes, LTAB + 8, 2);
        p32(&mut bytes, LTAB + 12, 0);
        for (country, id) in [(C0, 1u32), (C1, 2u32)] {
            p64(&mut bytes, country, ccountry::VTABLE_RVA as u64);
            p32(&mut bytes, country + ccountry::ID, id);
        }
        p64(
            &mut bytes,
            GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        p64(
            &mut bytes,
            MANAGER,
            cstrategic_air_manager::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            AIR_TAB,
            2,
        );
        p64(&mut bytes, AIR_TAB, 0);
        p64(&mut bytes, AIR_TAB + 8, AIR0 as u64);
        p64(&mut bytes, AIR0, cstrategic_air::VTABLE_RVA as u64);
        p32(&mut bytes, AIR0 + cstrategic_air::COUNTRY, 1);
        header(&mut bytes, AIR0 + cstrategic_air::REGION_COMBAT, RVEC, 2);
        p64(&mut bytes, RVEC, R0 as u64);
        p64(&mut bytes, RVEC + 8, 0);
        p64(&mut bytes, R0, cair_region_combat_data::VTABLE_RVA as u64);
        p32(&mut bytes, R0 + cair_region_combat_data::OWNER, 7);
        header(
            &mut bytes,
            R0 + cair_region_combat_data::FIRST_RECORDS,
            FVEC,
            2,
        );
        p64(&mut bytes, FVEC, REC0 as u64);
        p64(&mut bytes, FVEC + 8, REC1 as u64);
        header(
            &mut bytes,
            R0 + cair_region_combat_data::SECOND_RECORDS,
            SVEC2,
            1,
        );
        p64(&mut bytes, SVEC2, REC2 as u64);
        // Three records inline: REC0/REC1 in the first vector, REC2 in the
        // second. REC0 carries two first-channel damage entries, REC1 one
        // second-channel entry, REC2 none.
        record(
            &mut bytes,
            REC0,
            0x1111,
            100,
            0xF0,
            1,
            1,
            11,
            0,
            Some((DV0, 2)),
            None,
        );
        record(
            &mut bytes,
            REC1,
            0x2222,
            -50,
            0x0F,
            0,
            0,
            12,
            1,
            None,
            Some((DV1, 1)),
        );
        record(&mut bytes, REC2, 0x3333, 0, 0, 1, 0, 13, 0, None, None);
        damage(&mut bytes, E0, 5, 1000);
        damage(&mut bytes, E0 + sdamage_entry::STRIDE, 6, -2000);
        damage(&mut bytes, DV1, 7, 3000);
        let _ = sdamage_entry::STRIDE;
        let _ = sair_wing_combat_data::STRIDE;
        bytes
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        bytes: &mut [u8],
        at: usize,
        wing: u32,
        planes: i32,
        missions: u32,
        selection: u8,
        classification: u8,
        attributed: u32,
        copied: u8,
        first: Option<(usize, u32)>,
        second: Option<(usize, u32)>,
    ) {
        use engine::{sair_wing_combat_data, sdamage_entry};
        p64(bytes, at, sair_wing_combat_data::VTABLE_RVA as u64);
        p32(bytes, at + sair_wing_combat_data::WING_ID, wing);
        p32(bytes, at + sair_wing_combat_data::DATE, 1000);
        p32(
            bytes,
            at + sair_wing_combat_data::PLANE_COUNT,
            planes as u32,
        );
        p32(bytes, at + sair_wing_combat_data::MISSION_MASK, missions);
        bytes[at + sair_wing_combat_data::SELECTION_FLAG] = selection;
        bytes[at + sair_wing_combat_data::CLASSIFICATION_FLAG] = classification;
        p32(
            bytes,
            at + sair_wing_combat_data::ATTRIBUTED_COUNTRY,
            attributed,
        );
        bytes[at + sair_wing_combat_data::COPIED_FLAG] = copied;
        match first {
            Some((data, count)) => {
                header(bytes, at + sair_wing_combat_data::FIRST_DAMAGE, data, count)
            }
            None => header(bytes, at + sair_wing_combat_data::FIRST_DAMAGE, 0, 0),
        }
        match second {
            Some((data, count)) => header(
                bytes,
                at + sair_wing_combat_data::SECOND_DAMAGE,
                data,
                count,
            ),
            None => header(bytes, at + sair_wing_combat_data::SECOND_DAMAGE, 0, 0),
        }
        let _ = sdamage_entry::STRIDE;
    }

    fn damage(bytes: &mut [u8], at: usize, cause: u32, amount: i64) {
        use engine::sdamage_entry;
        p64(bytes, at, sdamage_entry::VTABLE_RVA as u64);
        p32(bytes, at + sdamage_entry::CAUSE_TYPE, cause);
        bytes[at + sdamage_entry::AMOUNT..at + sdamage_entry::AMOUNT + 8]
            .copy_from_slice(&amount.to_le_bytes());
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub history_outcome: Outcome,
        pub history: Vec<engine::sair_wing_combat_data::Row>,
        pub damage_outcome: Outcome,
        pub damage: Vec<engine::sdamage_entry::Row>,
    }

    fn collect_typed(memory: &impl Memory, history_cap: usize, damage_cap: usize) -> Split {
        use engine::cair_region_combat_data;
        let capacities = Capacities {
            rows: [8192, 2048, 4096, history_cap, damage_cap, 1024, 2048],
            ace_names: 65_536,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; 7]);
        let walked = cair_region_combat_data::walk_world(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.history.rows,
            &mut captured.damage.rows,
        );
        captured.history.outcome = super::bounded(walked.records, captured.history.rows.len());
        captured.damage.outcome = super::bounded(walked.damage, captured.damage.rows.len());
        Split {
            history_outcome: captured.history.outcome,
            history: captured.history.rows.clone(),
            damage_outcome: captured.damage.outcome,
            damage: captured.damage.rows.clone(),
        }
    }

    fn collect_split(
        memory: &impl Memory,
        history_cap: usize,
        damage_cap: usize,
    ) -> (Split, AirHistoryCopyReport, AirHistoryJob) {
        let ctx = AirHistoryCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 2],
        };
        let capacities = Capacities {
            rows: [8192, 2048, 4096, history_cap, damage_cap, 1024, 2048],
            ace_names: 65_536,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; 7]);
        let mut first = AirHistoryJob::new();
        let mut second = AirHistoryJob::new();
        let report = captured.copy_air_history_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 2];
        captured.decode_air_history_payload(BASE, &first, &mut ticks);
        (
            Split {
                history_outcome: captured.history.outcome,
                history: captured.history.rows.clone(),
                damage_outcome: captured.damage.outcome,
                damage: captured.damage.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect_typed(memory, 8192, 16_384)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, AirHistoryCopyReport, AirHistoryJob) {
        collect_split(memory, 8192, 16_384)
    }

    pub(crate) fn direct_with(memory: &impl Memory, history: usize, damage: usize) -> Split {
        collect_typed(memory, history, damage)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        history: usize,
        damage: usize,
    ) -> (Split, AirHistoryCopyReport, AirHistoryJob) {
        collect_split(memory, history, damage)
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(
            split.history_outcome, direct.history_outcome,
            "history outcome"
        );
        assert_eq!(split.history, direct.history, "history rows");
        assert_eq!(
            split.damage_outcome, direct.damage_outcome,
            "damage outcome"
        );
        assert_eq!(split.damage, direct.damage, "damage rows");
    }
}
