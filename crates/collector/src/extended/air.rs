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
    Memory, cace, cair_base, cair_region_combat_data, cair_wing, cequipment_variant,
    cequipment_variant_pool, cloop_history, cpdx_array::Array, equipment_entry,
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
    /// Raw-copy split gate for the hourly air-statistics table. Set by the
    /// game thread on hours with a valid raw payload; `capture` then skips
    /// the typed statistics walk and the decode worker fills these rows
    /// before the ready handoff. Preserved across `reset` (set per hour,
    /// cleared by the worker after decode).
    pub(crate) stats_raw_mode: bool,
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

#[derive(Debug)]
pub(crate) struct AirWingJob {
    pub log: RawLog<WING_PACK_N>,
    pub game_state: usize,
}

impl AirWingJob {
    pub fn new() -> Self {
        Self {
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
    let start = wspan(log, memory, links.data.checked_add(offset), 4, WING_CENSUS_MASK)?;
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
    let countries = wspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRIES), 16, WING_CENSUS_MASK)?;
    let links = wspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRY_LINKS), 16, WING_CENSUS_MASK)?;
    let manager_ptr = wspan(log, memory, game_state.checked_add(engine::cgame_state::STRATEGIC_AIR_MANAGER), 8, WING_CENSUS_MASK)
        .and_then(|s| usize::try_from(wing_pool_u64(log, s)).ok().filter(|&p| p != 0))?;
    let _ = wspan(log, memory, Some(manager_ptr), 8, WING_CENSUS_MASK)?;
    let owners_header = wspan(log, memory, manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR), 16, WING_CENSUS_MASK)?;
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
        let pointer_addr = (slot as usize).checked_mul(8).and_then(|o| countries_array.data.checked_add(o));
        let pointer = wspan(log, memory, pointer_addr, 8, WING_CENSUS_MASK)
            .and_then(|s| usize::try_from(wing_pool_u64(log, s)).ok().filter(|&p| p != 0))?;
        let vtable = wspan(log, memory, Some(pointer), 8, WING_CENSUS_MASK)?;
        if wing_pool_u64(log, vtable) != country_vtable {
            return None;
        }
        let id = wspan(log, memory, pointer.checked_add(ccountry::ID), 4, WING_CENSUS_MASK)?;
        let id = wing_pool_u32(log, id);
        let canonical = wing_link(log, memory, &links_array, countries_array.count, id)?;
        if wing_link(log, memory, &links_array, countries_array.count, slot) != Some(canonical) {
            return None;
        }
        if wing_link(log, memory, &links_array, countries_array.count, canonical) != Some(canonical) {
            return None;
        }
        let canonical_addr = (canonical as usize).checked_mul(8).and_then(|o| countries_array.data.checked_add(o));
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
        let owner_field = wspan(log, memory, owner_pointer.checked_add(cstrategic_air::COUNTRY), 4, WING_CENSUS_MASK)?;
        let owner = wing_pool_u32(log, owner_field);
        if owner != 0 && owner < links_array.count {
            let offset = (owner as usize).checked_mul(4);
            let _ = wspan(log, memory, offset.and_then(|o| links_array.data.checked_add(o)), 4, WING_CENSUS_MASK)?;
        }
        let pools_header = wspan(log, memory, owner_pointer.checked_add(cstrategic_air::POOLS), 16, WING_CENSUS_MASK)?;
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[pools_header..pools_header + 16]);
        let pools = match Array::from_header(header, 8) {
            Some(array) => array,
            None => continue,
        };
        for pool_index in 0..pools.count {
            let pool_addr = pools.data.checked_add((pool_index as usize).saturating_mul(8));
            let pool = wspan(log, memory, pool_addr, 8, WING_CENSUS_MASK)
                .and_then(|s| usize::try_from(wing_pool_u64(log, s)).ok().filter(|&p| p != 0))?;
            let _ = wspan(log, memory, Some(pool), 8, WING_CENSUS_MASK)?;
            let _ = wspan(log, memory, pool.checked_add(cair_wing_pool::OWNER), 8, WING_CENSUS_MASK)?;
            let wings_header = wspan(log, memory, pool.checked_add(cair_wing_pool::WINGS), 16, WING_CENSUS_MASK)?;
            let _ = wspan(log, memory, pool.checked_add(cair_wing_pool::AIR_BASE_REFERENCE), 8, WING_CENSUS_MASK)?;
            let mut wings_raw = [0u8; 16];
            wings_raw.copy_from_slice(&log.bytes[wings_header..wings_header + 16]);
            let wings = match Array::from_header(wings_raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for wing_index in 0..wings.count {
                let wing_addr = wings.data.checked_add((wing_index as usize).saturating_mul(8));
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
    use engine::{air_mission, cair_wing};
    for (offset, len, mask) in [
        (0usize, 8, WING_CENSUS_MASK),
        (cair_wing::SECONDARY, 16, WING_CENSUS_MASK),
        (cair_wing::POOL, 8, WING_CENSUS_MASK),
        (cair_wing::BASE_OVERRIDE, 8, WING_CENSUS_MASK),
        (cair_wing::PLANE_COUNT, 4, WING_CENSUS_MASK),
        (cair_wing::EFFECTIVE_PLANES, 4, WING_CENSUS_MASK),
        (cair_wing::MISSION + air_mission::ENABLED_MASK, 4, WING_CENSUS_MASK),
        (cair_wing::MISSION + air_mission::REGION, 8, WING_CENSUS_MASK),
        (cair_wing::COUNTRY_OVERRIDE, 4, WING_CENSUS_MASK),
        (cair_wing::AIR_ACCIDENTS, 4, CUMULATIVE_MASK),
        (cair_wing::EXPERIENCE, 8, CUMULATIVE_MASK),
    ] {
        let _ = wspan(log, memory, wing.checked_add(offset), len, mask);
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
        let entry_addr = array.data.checked_add((index as usize).saturating_mul(equipment_entry::STRIDE));
        let entry = match wspan(log, memory, entry_addr, equipment_entry::STRIDE, EQUIPMENT_MASK) {
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
    second: Option<&mut AirWingJob>,
) -> AirWingCopyReport {
    let mut ticks = [0u64; WING_PACK_N];
    copy_air_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; WING_PACK_N];
    if let Some(second_job) = second {
        let mut second_ticks = [0u64; WING_PACK_N];
        copy_air_pass(second_job, memory, ctx.base, ctx.game_state, &mut second_ticks);
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        // Per-table partitions: shared wing census gates both slots,
        // equipment/cumulative private spans only their own.
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal =
                raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed;
    AirWingCopyReport { raw_ok, matched, copy_ticks: ticks, raw_bytes: first.log.raw_bytes }
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
    let needed = [true, false, true, false, false, false, false];
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
                    let source = WingSource { pointer: *pointer, row, second: false };
                    captured.visit_wing(&recorded, base, source, &needed);
                }
                captured.finish_wings(outcome, None, &needed);
                break;
            }
        }
    }
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

pub(crate) const AIR_STATS_N: usize = 1;
const STATS_MASK: u32 = 0x01;

/// One refresh of raw air-statistics payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_air_stats_job`).
#[derive(Debug)]
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
    use engine::{cloop_history, cloop_history::air_stats, loop_history_record_fields, loop_history_window};
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
    let field_count = match sspan(log, memory, history.checked_add(cloop_history::FIELD_COUNT), 4, STATS_MASK) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    if field_count != air_stats::FIELD_COUNT {
        return false;
    }
    let self_ref = match sspan(log, memory, history.checked_add(cloop_history::SELF_REFERENCE), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, self_ref) != history as u64 {
        return false;
    }
    let window = match sspan(log, memory, history.checked_add(cloop_history::WINDOW), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let window = match usize::try_from(stats_pool_u64(log, window)).ok().filter(|&p| p != 0) {
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
    let records_header = match sspan(log, memory, window.checked_add(loop_history_window::RECORDS), 16, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[records_header..records_header + 16]);
    let records = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return false,
    };
    let capacity = match sspan(log, memory, window.checked_add(loop_history_window::CAPACITY), 4, STATS_MASK) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    let year_slot = match sspan(log, memory, window.checked_add(loop_history_window::CURRENT_SLOT), 4, STATS_MASK) {
        Some(start) => stats_pool_u32(log, start),
        None => return false,
    };
    let wrapped = match sspan(log, memory, window.checked_add(loop_history_window::WRAPPED), 1, STATS_MASK) {
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
    let owner_back = match sspan(log, memory, window.checked_add(loop_history_window::HISTORY), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, owner_back) != history as u64 {
        return false;
    }
    let record_ptr = match sspan(log, memory, window.checked_add(loop_history_window::RECORD_FIELDS), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    let record = match usize::try_from(stats_pool_u64(log, record_ptr)).ok().filter(|&p| p != 0) {
        Some(record) => record,
        None => return false,
    };
    let slot_addr = match (year_slot as usize).checked_mul(8).and_then(|o| records.data.checked_add(o)) {
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
    let record_owner = match sspan(log, memory, record.checked_add(loop_history_record_fields::HISTORY), 8, STATS_MASK) {
        Some(start) => start,
        None => return false,
    };
    if stats_pool_u64(log, record_owner) != history as u64 {
        return false;
    }
    let values_header = match sspan(log, memory, record.checked_add(loop_history_record_fields::VALUES), 16, STATS_MASK) {
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
    // Nine channel spans at the pinned channel indices (mirrors the
    // copy_scattered gather; the replay decomposes it into these exact
    // copies served from the recorded spans).
    for (len, index) in [(32, 0), (8, 10), (8, 17), (8, 24), (72, 36), (8, 51), (8, 58), (8, 65), (40, 77)] {
        let addr = match (index as usize)
            .checked_mul(loop_history_record_fields::VALUE_STRIDE)
            .and_then(|o| values.data.checked_add(o))
        {
            Some(addr) => addr,
            None => return false,
        };
        if sspan(log, memory, Some(addr), len, STATS_MASK).is_none() {
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
    let countries = match sspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRIES), 16, STATS_MASK) {
        Some(start) => start,
        None => return,
    };
    let links = match sspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRY_LINKS), 16, STATS_MASK) {
        Some(start) => start,
        None => return,
    };
    let manager_ptr = match sspan(log, memory, game_state.checked_add(engine::cgame_state::STRATEGIC_AIR_MANAGER), 8, STATS_MASK)
        .and_then(|s| usize::try_from(stats_pool_u64(log, s)).ok().filter(|&p| p != 0))
    {
        Some(manager) => manager,
        None => return,
    };
    if sspan(log, memory, Some(manager_ptr), 8, STATS_MASK).is_none() {
        return;
    }
    let owners_header = match sspan(log, memory, manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR), 16, STATS_MASK) {
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
        let addr = match (link as usize).checked_mul(4).and_then(|o| links_array.data.checked_add(o)) {
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
        let pointer_addr = (slot as usize).checked_mul(8).and_then(|o| countries_array.data.checked_add(o));
        let pointer = match sspan(log, memory, pointer_addr, 8, STATS_MASK)
            .and_then(|s| usize::try_from(stats_pool_u64(log, s)).ok().filter(|&p| p != 0))
        {
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
        let id = match sspan(log, memory, pointer.checked_add(ccountry::ID), 4, STATS_MASK) {
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
        let owner_field = match sspan(log, memory, owner.checked_add(cstrategic_air::COUNTRY), 4, STATS_MASK) {
            Some(start) => start,
            None => return,
        };
        let _ = stats_pool_u32(log, owner_field);
        let table_header = match sspan(log, memory, owner.checked_add(cstrategic_air::HISTORY_TABLE), 16, STATS_MASK) {
            Some(start) => start,
            None => return,
        };
        let mut table_raw = [0u8; 16];
        table_raw.copy_from_slice(&log.bytes[table_header..table_header + 16]);
        let table = match Array::from_header(table_raw, 8) {
            Some(array) => array,
            None => continue,
        };
        for region in 0..table.count {
            let slot_addr = match (region as usize).checked_mul(8).and_then(|o| table.data.checked_add(o)) {
                Some(addr) => addr,
                None => return,
            };
            let slot_start = match sspan(log, memory, Some(slot_addr), 8, STATS_MASK) {
                Some(start) => start,
                None => return,
            };
            let history = match usize::try_from(stats_pool_u64(log, slot_start)).ok() {
                Some(history) => history,
                None => return,
            };
            if history == 0 {
                continue;
            }
            if !copy_stats_history(log, memory, base, history) {
                return;
            }
            if log.overflowed {
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
    second: Option<&mut AirStatsJob>,
) -> AirStatsCopyReport {
    let mut ticks = [0u64; AIR_STATS_N];
    copy_stats_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; AIR_STATS_N];
    if let Some(second_job) = second {
        let mut second_ticks = [0u64; AIR_STATS_N];
        copy_stats_pass(second_job, memory, ctx.base, ctx.game_state, &mut second_ticks);
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed;
    AirStatsCopyReport { raw_ok, matched, copy_ticks: ticks, raw_bytes: first.log.raw_bytes }
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
    let walked = cloop_history::walk(&recorded, base, job.game_state, &mut captured.statistics.rows);
    captured.statistics.outcome = bounded(walked, captured.statistics.rows.len());
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    captured.stats_raw_mode = false;
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

    #[test]
    fn wing_pack_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        for table in [TableId::WingEquipment, TableId::WingCumulative] {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(replay::cadence::is_refresh(table, hour), "{table:?} at {hour}");
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
        use super::wing_test_kit as kit;
        use super::Capacities;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for (equipment, cumulative) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 3), (8192, 4096)] {
            let capacity = Capacities { rows: [equipment, 2048, cumulative, 8192, 16384, 1024, 2048], ace_names: 65536 };
            let direct = kit::direct_with(&memory, &capacity);
            let (split, report, _) = kit::split_with(&memory, &capacity);
            assert!(report.raw_ok, "equipment={equipment} cumulative={cumulative}");
            assert_eq!(report.matched, [true; 2], "equipment={equipment} cumulative={cumulative}");
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows with all rows present.
        let capacity = Capacities { rows: [3, 2048, 2, 8192, 16384, 1024, 2048], ace_names: 65536 };
        let (split, _, _) = kit::split_with(&memory, &capacity);
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
            (kit::GAME_STATE + engine::cgame_state::COUNTRIES, [false, false]),
            (kit::WING, [false, false]),
            (kit::WING + engine::cair_wing::EXPERIENCE, [true, false]),
            (kit::WING + engine::cair_wing::EQUIPMENT_POOL, [false, true]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory { base: 0, bytes: &bytes },
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
        let ctx = AirWingCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
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
            let capacities = Capacities { rows: [8192, 2048, 4096, 8192, 16384, 1024, rows], ace_names: 65536 };
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "rows={rows}");
            assert_eq!(report.matched, [true], "rows={rows}");
            kit::assert_matches_direct(&split, &direct);
        }
        let capacities = Capacities { rows: [8192, 2048, 4096, 8192, 16384, 1024, 2], ace_names: 65536 };
        let (split, _, _) = kit::split(&memory, &capacities);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn stats_double_copy_compare_blocks_checks_on_tearing() {
        use super::stats_test_kit as kit;
        use super::{AirStatsCopyContext, AirStatsJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirStatsCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
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
        let ctx = AirStatsCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
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
        let memory = Recorder { inner: kit::memory(&bytes), sizes: std::cell::RefCell::new(BTreeSet::new()) };
        let ctx = super::AirStatsCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
        let mut job = super::AirStatsJob::new();
        let report = super::copy_air_stats_refresh(&memory, ctx, &mut job, None);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 16, 32, 40, 72] {
            assert!(sizes.contains(&required), "missing size {required} in {sizes:?}");
        }
    }

    #[test]
    fn wing_staging_overflow_falls_back_to_typed_walk() {
        use super::wing_test_kit as kit;
        use super::{AirWingCopyContext, AirWingJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirWingCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
        let mut tiny = AirWingJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
        let report = super::copy_air_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }
}

/// Shared synthetic address space for the wing-pack copy/decode harness:
/// one country, one pool, two wings with equipment entries and cumulative
/// counters. Mirrors the engine wing/equipment fixture shapes.
#[cfg(test)]
pub(crate) mod wing_test_kit {
    use super::{AirWingCopyContext, AirWingCopyReport, AirWingJob, Capacities, Captured};
    use engine::{memory::SliceMemory, walk::Outcome, Memory};

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
        use engine::{cair_wing, ccountry, cequipment_variant, cequipment_variant_pool, equipment_entry};
        let mut bytes = vec![0; 0x40000];
        header(&mut bytes, GAME_STATE + engine::cgame_state::COUNTRIES, CTAB, 3);
        p64(&mut bytes, CTAB, 0);
        p64(&mut bytes, CTAB + 8, C0 as u64);
        p64(&mut bytes, CTAB + 16, C0 as u64);
        header(&mut bytes, GAME_STATE + engine::cgame_state::COUNTRY_LINKS, LTAB, 3);
        p32(&mut bytes, LTAB, 0);
        p32(&mut bytes, LTAB + 4, 1);
        p32(&mut bytes, LTAB + 8, 1);
        p64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, C0 + ccountry::ID, 1);
        p64(&mut bytes, GAME_STATE + engine::cgame_state::STRATEGIC_AIR_MANAGER, MANAGER as u64);
        p64(&mut bytes, MANAGER, engine::cstrategic_air_manager::VTABLE_RVA as u64);
        header(&mut bytes, MANAGER + engine::cstrategic_air_manager::COUNTRY_AIR, 0x38000, 2);
        p64(&mut bytes, 0x38000, 0);
        p64(&mut bytes, 0x38000 + 8, OWNER_AIR as u64);
        p64(&mut bytes, OWNER_AIR, engine::cstrategic_air::VTABLE_RVA as u64);
        p32(&mut bytes, OWNER_AIR + engine::cstrategic_air::COUNTRY, 1);
        header(&mut bytes, OWNER_AIR + engine::cstrategic_air::POOLS, 0x38100, 1);
        p64(&mut bytes, 0x38100, POOL as u64);
        p64(&mut bytes, POOL, engine::cair_wing_pool::VTABLE_RVA as u64);
        p64(&mut bytes, POOL + engine::cair_wing_pool::OWNER, OWNER_AIR as u64);
        p64(&mut bytes, POOL + engine::cair_wing_pool::AIR_BASE_REFERENCE, 0x1234u64);
        header(&mut bytes, POOL + engine::cair_wing_pool::WINGS, 0x38200, 2);
        p64(&mut bytes, 0x38200, WING as u64);
        p64(&mut bytes, 0x38200 + 8, WING2 as u64);
        for (wing, id, accidents, exp) in [(WING, 0x4500_0001u64, 3i32, 1000i64), (WING2, 0x4500_0002u64, -5i32, i64::MAX)] {
            p64(&mut bytes, wing, cair_wing::VTABLE_RVA as u64);
            p64(&mut bytes, wing + cair_wing::SECONDARY, cair_wing::SECONDARY_VTABLE_RVA as u64);
            p64(&mut bytes, wing + cair_wing::SECONDARY + 8, id);
            p64(&mut bytes, wing + cair_wing::POOL, POOL as u64);
            p64(&mut bytes, wing + cair_wing::BASE_OVERRIDE, 0);
            p32(&mut bytes, wing + cair_wing::PLANE_COUNT, 10);
            p32(&mut bytes, wing + cair_wing::EFFECTIVE_PLANES, 8);
            p32(&mut bytes, wing + cair_wing::MISSION + engine::air_mission::ENABLED_MASK, 0xff);
            p64(&mut bytes, wing + cair_wing::MISSION + engine::air_mission::REGION, 0);
            p32(&mut bytes, wing + cair_wing::COUNTRY_OVERRIDE, 0);
            p32(&mut bytes, wing + cair_wing::AIR_ACCIDENTS, accidents as u32);
            p64(&mut bytes, wing + cair_wing::EXPERIENCE, exp as u64);
            // Equipment pool: two entries on WING, one on WING2.
            let (items, entries) = if wing == WING {
                (0x39000usize, vec![(VARIANT, 100i64), (VARIANT2, -50i64)])
            } else {
                (0x39100usize, vec![(VARIANT, 7i64)])
            };
            let pool = wing + cair_wing::EQUIPMENT_POOL;
            p64(&mut bytes, pool, cequipment_variant_pool::VTABLE_RVA as u64);
            header(&mut bytes, pool + cequipment_variant_pool::ITEMS, items, entries.len() as u32);
            for (index, (variant, amount)) in entries.into_iter().enumerate() {
                p64(&mut bytes, items + index * equipment_entry::STRIDE, variant as u64);
                p64(&mut bytes, items + index * equipment_entry::STRIDE + 8, amount as u64);
            }
        }
        for (variant, id) in [(VARIANT, 11u32), (VARIANT2, 12u32)] {
            p64(&mut bytes, variant, cequipment_variant::VTABLE_RVA as u64);
            p64(&mut bytes, variant + 8, ((id as u64) << 32) | cequipment_variant::REFERENCE_TYPE as u64);
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
        let outcome = cair_wing::walk_with(memory, BASE, GAME_STATE, &mut wing_rows, |pointer, _, row| {
            collected.push((pointer, *row));
        });
        for (pointer, row) in &collected {
            let source = super::WingSource { pointer: *pointer, row, second: false };
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
        let ctx = AirWingCopyContext { base: BASE, game_state: GAME_STATE };
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

    pub(crate) fn split_with(memory: &impl Memory, capacity: &Capacities) -> (Split, AirWingCopyReport, AirWingJob) {
        let ctx = AirWingCopyContext { base: BASE, game_state: GAME_STATE };
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
        let capacity = Capacities { rows: [1, 2048, 1, 8192, 16384, 1024, 2048], ace_names: 65536 };
        collect_typed(memory, &capacity)
    }

    pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, AirWingCopyReport, AirWingJob) {
        let ctx = AirWingCopyContext { base: BASE, game_state: GAME_STATE };
        let mut first = AirWingJob::new();
        let mut second = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let capacity = Capacities { rows: [1, 2048, 1, 8192, 16384, 1024, 2048], ace_names: 65536 };
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
        assert_eq!(split.equipment_outcome, direct.equipment_outcome, "equipment outcome");
        assert_eq!(split.equipment, direct.equipment, "equipment rows");
        assert_eq!(split.cumulative_outcome, direct.cumulative_outcome, "cumulative outcome");
        assert_eq!(split.cumulative, direct.cumulative, "cumulative rows");
    }

    /// Decode-worker assertion against pooled buffers (same module, so
    /// private rows are visible). Compares outcomes and rows with the typed
    /// reference without cloning through public views.
    pub(crate) fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
        assert_eq!(captured.equipment.outcome, direct.equipment_outcome, "equipment outcome");
        assert_eq!(&captured.equipment.rows, &direct.equipment, "equipment rows");
        assert_eq!(captured.cumulative.outcome, direct.cumulative_outcome, "cumulative outcome");
        assert_eq!(&captured.cumulative.rows, &direct.cumulative, "cumulative rows");
    }
}

/// Shared synthetic address space for the air-statistics copy/decode
/// harness: one owner with a two-region history table, each region holding
/// one history with a valid yearly window, record and channel values.
/// Mirrors the engine statistics fixture shapes contiguously.
#[cfg(test)]
pub(crate) mod stats_test_kit {
    use super::{AirStatsCopyContext, AirStatsCopyReport, AirStatsJob, Capacities, Captured};
    use engine::{memory::SliceMemory, walk::Outcome, Memory};

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

    fn history(bytes: &mut [u8], history: usize, window: usize, slots: usize, record: usize, values: usize, seed: i64) {
    use engine::{cloop_history::air_stats, cloop_history, loop_history_record_fields, loop_history_window};
        p64(bytes, history, (BASE + cloop_history::VTABLE_RVA) as u64);
        p32(bytes, history + cloop_history::FIELD_COUNT, air_stats::FIELD_COUNT);
        p64(bytes, history + cloop_history::SELF_REFERENCE, history as u64);
        p64(bytes, history + cloop_history::WINDOW, window as u64);
        p64(bytes, window, (BASE + loop_history_window::VTABLE_RVA) as u64);
        header(bytes, window + loop_history_window::RECORDS, slots, 5);
        p32(bytes, window + loop_history_window::CAPACITY, 5);
        p32(bytes, window + loop_history_window::CURRENT_SLOT, 2);
        bytes[window + loop_history_window::WRAPPED] = 1;
        p64(bytes, window + loop_history_window::HISTORY, history as u64);
        p64(bytes, window + loop_history_window::RECORD_FIELDS, record as u64);
        for slot in 0..5 {
            p64(bytes, slots + slot * 8, 0);
        }
        p64(bytes, slots + 2 * 8, record as u64);
        p64(bytes, record + loop_history_record_fields::HISTORY, history as u64);
        header(bytes, record + loop_history_record_fields::VALUES, values, air_stats::FIELD_COUNT);
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
        p64(&mut bytes, GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER as u64);
        p64(&mut bytes, MANAGER, (BASE + cstrategic_air_manager::VTABLE_RVA) as u64);
        header(&mut bytes, MANAGER + cstrategic_air_manager::COUNTRY_AIR, OWNER_TAB, 3);
        p64(&mut bytes, OWNER_TAB, 0);
        p64(&mut bytes, OWNER_TAB + 8, AIR as u64);
        p64(&mut bytes, OWNER_TAB + 16, 0);
        p64(&mut bytes, AIR, (BASE + cstrategic_air::VTABLE_RVA) as u64);
        p32(&mut bytes, AIR + cstrategic_air::COUNTRY, 2);
        header(&mut bytes, AIR + cstrategic_air::HISTORY_TABLE, HIST_TAB, 2);
        p64(&mut bytes, HIST_TAB, HISTORY1 as u64);
        p64(&mut bytes, HIST_TAB + 8, HISTORY2 as u64);
        history(&mut bytes, HISTORY1, WINDOW1, SLOTS1, RECORD1, VALUES1, 1000);
        history(&mut bytes, HISTORY2, WINDOW2, SLOTS2, RECORD2, VALUES2, 2000);
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
        Split { outcome: captured.statistics.outcome, rows: captured.statistics.rows.clone() }
    }

    pub(crate) fn direct(memory: &impl Memory, capacity: &Capacities) -> Split {
        collect_typed(memory, capacity)
    }

    pub(crate) fn split(memory: &impl Memory, capacity: &Capacities) -> (Split, AirStatsCopyReport, AirStatsJob) {
        let ctx = AirStatsCopyContext { base: BASE, game_state: GAME_STATE };
        let mut first = AirStatsJob::new();
        let mut second = AirStatsJob::new();
        let report = super::copy_air_stats_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity, &[0; 7]);
        let mut ticks = [0u64; 1];
        super::decode_air_stats_payload(&mut captured, BASE, &first, &mut ticks);
        (
            Split { outcome: captured.statistics.outcome, rows: captured.statistics.rows.clone() },
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
        assert_eq!(captured.statistics.outcome, direct.outcome, "statistics outcome");
        assert_eq!(&captured.statistics.rows, &direct.rows, "statistics rows");
    }
}
