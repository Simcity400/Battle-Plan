//! Pooled air state. Acquisition never allocates; conversion and growth are off-thread.
//!
//! A single independently bounded wing census acquires map, equipment,
//! cumulative and naval-membership value rows. The worker installs owned
//! extension rows without replaying native memory reads. The same census,
//! joined by each wing's mission address, orders the registered missions and
//! names the bases and mines the flying-aircraft map needs.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cace, cair_base, cair_region_combat_data,
    cair_regional_mission::{self, WingIdentity},
    cair_wing, cair_wing_pool, ccountry, ccountry_air_container, cgame_state, cloop_history,
    cpdx_array::Array,
    cprovince, cstate, cstrategic_air_manager, cstrategic_navy_manager, equipment_entry,
    map_air_base_reference,
    memory::{RecordLayout, RecordSources},
    msvc_string, sair_wing_combat_data, sdamage_entry,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

use super::air_reference_witness::{self, BaseWitness};

pub const N: usize = 10;
pub const TABLES: [TableId; N] = [
    TableId::WingEquipment,
    TableId::AirBases,
    TableId::WingCumulative,
    TableId::AirRegionHistory,
    TableId::AirRegionDamage,
    TableId::AirAces,
    TableId::AirStatistics,
    TableId::AirRegionalMissions,
    TableId::AirReferenceWitnesses,
    TableId::NavalMineLevels,
];

/// The regional source walk is charged to history. Owned wing acquisition is charged to
/// WingEquipment, excluding separately charged naval-membership reads. Typed
/// fallback extension times exclude the enclosing map census.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("regional air history and damage", 3, &[3, 4])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    /// History/damage capacities count native entries, before off-thread grouping.
    /// Witnesses count probed base handles.
    pub rows: [usize; N],
    pub ace_names: usize,
    /// The wing census joined to missions, and its base-handle scratch.
    pub wings: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [8192, 2048, 4096, 8192, 16_384, 1024, 2048, 4096, 1024, 64],
            ace_names: 65_536,
            wings: 8192,
        }
    }
}

/// The exact invocation context from the shared authenticated AirWings visitor.
/// It is never retained. The pool pointer is unnecessary for these two fields.
pub struct WingSource<'a> {
    pub pointer: usize,
    pub row: &'a cair_wing::Row,
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
    /// This hour's complete wing census by mission address. Acquisition
    /// context only: pointers never reach the replay.
    identities: Arena<WingIdentity>,
    identity_outcome: Outcome,
    /// The owned wing job installed the census; the typed visitor did not run.
    identities_owned: bool,
    registrations: CaptureRows<cair_regional_mission::Row>,
    /// Base handles to probe: registered pool bases, unregistered training
    /// wings' pool bases and every transfer base.
    handles: Vec<u64>,
    references: CaptureRows<BaseWitness>,
    mines: CaptureRows<rows::NavalMineLevelRow>,
    wing_ticks: [u64; 2],
    /// Game-thread time installing the owned census, charged to
    /// AirRegionalMissions, which joins it.
    identity_ticks: u64,
    active: [bool; N],
    /// Raw-copy split gate for the scheduled air-statistics table. Set by the
    /// callback on hours with a valid raw payload; `capture` then skips
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
    /// (copy/decode split). Set by the callback on scheduled hours with a valid
    /// raw payload; `capture` then skips the typed regional walk and the
    /// decode worker fills both row sets before the ready handoff. Preserved
    /// across `reset`, cleared by the worker.
    pub(crate) history_raw_mode: bool,
}

const NO_WING_SOURCE: Outcome = Outcome::Unavailable {
    reason: "shared air wing census not supplied",
};

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay::cadence::SKIPPED_REASON,
};

fn failed(active: bool, reason: &'static str) -> Outcome {
    if active {
        Outcome::Unavailable { reason }
    } else {
        SKIPPED
    }
}

impl Captured {
    /// An accepted Late job never retries. Independent owned payloads still
    /// install their tables on decode; their raw gates must survive this path.
    pub(crate) fn fail_late_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        self.equipment.fail(failed(self.active[0], reason));
        self.cumulative.fail(failed(self.active[2], reason));
        self.registrations.fail(failed(self.active[7], reason));
        self.references.fail(failed(self.active[8], reason));
        self.mines.fail(failed(self.active[9], reason));
        if !self.bases_raw_mode {
            self.bases.fail(failed(self.active[1], reason));
            self.aces.fail(failed(self.active[5], reason));
        }
        if !self.history_raw_mode {
            self.history.fail(failed(self.active[3], reason));
            self.damage.fail(failed(self.active[4], reason));
        }
        if !self.stats_raw_mode {
            self.statistics.fail(failed(self.active[6], reason));
        }
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            ace_names: 0,
            wings: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            equipment: CaptureRows::new(capacity.rows[0]),
            bases: CaptureRows::new(capacity.rows[1]),
            cumulative: CaptureRows::new(capacity.rows[2]),
            history: CaptureRows::new(capacity.rows[3]),
            damage: CaptureRows::new(capacity.rows[4]),
            aces: CaptureRows::new(capacity.rows[5]),
            statistics: CaptureRows::new(capacity.rows[6]),
            names: Arena::new(capacity.ace_names),
            identities: Arena::new(capacity.wings),
            identity_outcome: NO_WING_SOURCE,
            identities_owned: false,
            registrations: CaptureRows::new(capacity.rows[7]),
            handles: vec![0; capacity.wings * 2],
            references: CaptureRows::new(capacity.rows[8]),
            mines: CaptureRows::new(capacity.rows[9]),
            wing_ticks: [0; 2],
            identity_ticks: 0,
            active: [true; N],
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
        self.identities.reset();
        self.identity_outcome = NO_WING_SOURCE;
        self.identities_owned = false;
        self.reset_missions();
        self.wing_ticks = [0; 2];
        self.identity_ticks = 0;
    }

    fn reset_missions(&mut self) {
        self.registrations.reset();
        self.references.reset();
        self.mines.reset();
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
            + self.identities.allocated_bytes()
            + self.registrations.allocated_bytes()
            + self.handles.capacity() * size_of::<u64>()
            + self.references.allocated_bytes()
            + self.mines.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.equipment.grow(&mut capacity.rows[0]);
        self.bases.grow(&mut capacity.rows[1]);
        self.cumulative.grow(&mut capacity.rows[2]);
        self.history.grow(&mut capacity.rows[3]);
        self.damage.grow(&mut capacity.rows[4]);
        self.aces.grow(&mut capacity.rows[5]);
        self.statistics.grow(&mut capacity.rows[6]);
        self.names.grow(&mut capacity.ace_names);
        self.identities.grow(&mut capacity.wings);
        // One pool base per registration plus one transfer base per wing.
        crate::table_capture::grow_vec(
            &mut self.handles,
            (capacity.wings * 2).max(capacity.rows[8]),
        );
        self.registrations.grow(&mut capacity.rows[7]);
        self.references.grow(&mut capacity.rows[8]);
        self.mines.grow(&mut capacity.rows[9]);
    }

    /// Freeze the requested tables before contexts and shared map callbacks.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
    }

    /// Initialize the extension rows immediately before the shared map walk.
    pub fn begin_wings(&mut self) {
        self.equipment.reset();
        self.cumulative.reset();
        self.equipment.outcome = Outcome::Rows { len: 0 };
        self.cumulative.outcome = if self.active[2] {
            Outcome::Rows { len: 0 }
        } else {
            SKIPPED
        };
        self.identities.reset();
        self.identity_outcome = Outcome::Rows { len: 0 };
        self.identities_owned = false;
        self.wing_ticks = [0; 2];
        self.identity_ticks = 0;
    }

    fn push_identity(&mut self, pointer: usize, row: &cair_wing::Row) {
        if matches!(self.identity_outcome, Outcome::Unavailable { .. }) {
            return;
        }
        let identity = match WingIdentity::new(pointer, row) {
            Ok(identity) => identity,
            Err(reason) => {
                self.identity_outcome = Outcome::Unavailable { reason };
                return;
            }
        };
        self.identities.required += 1;
        if let Some(destination) = self.identities.first.get_mut(self.identities.len) {
            *destination = identity;
            self.identities.len += 1;
        }
        let len = self.identities.len;
        let unread_rows = (self.identities.required - len).min(u32::MAX as usize) as u32;
        self.identity_outcome = if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        };
    }

    /// The owned wing job's complete source census, installed on the game
    /// thread after the job joins; the typed visitor does not run that hour.
    pub(crate) fn install_owned_mission_identities(&mut self, job: &AirWingJob) {
        let begin = qpc();
        self.install_identities(job);
        self.identity_ticks = self.identity_ticks.saturating_add(elapsed(begin));
    }

    fn install_identities(&mut self, job: &AirWingJob) {
        self.identities.reset();
        self.identities_owned = true;
        self.identity_outcome = Outcome::Rows { len: 0 };
        match job.source_outcome {
            Outcome::Rows { len } if len <= job.sources.len() => {
                for source in &job.sources[..len] {
                    self.push_identity(source.pointer, &source.row);
                }
            }
            _ => {
                self.identity_outcome = Outcome::Unavailable {
                    reason: "registered missions lack the complete wing census",
                }
            }
        }
    }

    /// Extension failure never changes the map row. Equipment retains full
    /// native references and amounts, with no transient pointers in pooled rows.
    pub fn visit_wing(&mut self, memory: &impl Memory, base: usize, source: WingSource<'_>) {
        self.push_identity(source.pointer, source.row);
        let mut reads = read_accounting::table(TableId::WingEquipment);
        let begin = qpc();
        let outcome = &mut self.equipment.outcome;
        if !matches!(outcome, Outcome::Unavailable { .. }) {
            let next = read_equipment(
                memory,
                base,
                &source,
                &mut self.equipment.rows[outcome.len()..],
            );
            *outcome = append(*outcome, next);
        }
        let end = qpc();
        self.wing_ticks[0] =
            self.wing_ticks[0].saturating_add(end.saturating_sub(begin).max(0) as u64);
        if !self.active[2] {
            self.cumulative.outcome = SKIPPED;
            return;
        }
        reads.select(TableId::WingCumulative);
        let outcome = &mut self.cumulative.outcome;
        if !matches!(outcome, Outcome::Unavailable { .. }) {
            let next = if let Some(destination) = self.cumulative.rows.get_mut(outcome.len()) {
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
        self.wing_ticks[1] = self.wing_ticks[1].saturating_add(elapsed(end));
    }

    /// Both extensions require a complete parent census. Subtract these ticks
    /// from the enclosing map walk's charged time.
    pub fn finish_wings(&mut self, source: Outcome) -> [u64; 2] {
        if !self.identities_owned {
            self.identity_outcome = require_parent(source, self.identity_outcome);
        }
        self.equipment.outcome = require_parent(source, self.equipment.outcome);
        self.cumulative.outcome = if self.active[2] {
            require_parent(source, self.cumulative.outcome)
        } else {
            SKIPPED
        };
        self.wing_ticks
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
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.bases.reset();
        self.history.reset();
        self.damage.reset();
        self.aces.reset();
        self.statistics.reset();
        self.names.reset();
        self.reset_missions();
        let mut ticks = [0; N];
        ticks[0] = self.wing_ticks[0];
        ticks[2] = self.wing_ticks[1];
        if !self.bases_raw_mode {
            let begin = qpc();
            capture_rows(&mut self.bases, |rows| {
                let _reads = read_accounting::table(TableId::AirBases);
                cair_base::walk(memory, base, game_state, rows)
            });
            ticks[1] = elapsed(begin);
        }
        if !self.active[3] {
            self.history.outcome = SKIPPED;
            self.damage.outcome = SKIPPED;
        } else if !self.history_raw_mode {
            let _reads = read_accounting::table(TableId::AirRegionHistory);
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
            ticks[3] = elapsed(begin);
        }
        if !self.active[5] {
            self.aces.outcome = SKIPPED;
        } else if !self.bases_raw_mode {
            let _reads = read_accounting::table(TableId::AirAces);
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
            ticks[5] = elapsed(begin);
        }
        if !self.active[6] {
            self.statistics.outcome = SKIPPED;
        } else if !self.stats_raw_mode {
            let begin = qpc();
            capture_rows(&mut self.statistics, |rows| {
                let _reads = read_accounting::table(TableId::AirStatistics);
                cloop_history::walk(memory, base, game_state, rows)
            });
            ticks[6] = elapsed(begin);
        }
        let begin = qpc();
        self.registrations.outcome = if !self.active[7] {
            SKIPPED
        } else if let Outcome::Rows { len } = self.identity_outcome {
            let _reads = read_accounting::table(TableId::AirRegionalMissions);
            let identities = &mut self.identities.first[..len];
            identities.sort_unstable_by_key(|wing| wing.mission_pointer);
            bounded(
                cair_regional_mission::walk(
                    memory,
                    base,
                    game_state,
                    identities,
                    &mut self.registrations.rows,
                ),
                self.registrations.rows.len(),
            )
        } else {
            Outcome::Unavailable {
                reason: "registered missions lack the complete wing census",
            }
        };
        ticks[7] = elapsed(begin).saturating_add(self.identity_ticks);
        // The remaining joins look wings up by handle.
        let (wings, registered) = match (self.identity_outcome, self.registrations.outcome) {
            (Outcome::Rows { len: wings }, Outcome::Rows { len }) => {
                let identities = &mut self.identities.first[..wings];
                identities.sort_unstable_by_key(|wing| wing.wing);
                (&*identities, Some(&self.registrations.rows[..len]))
            }
            _ => (&[][..], None),
        };
        let begin = qpc();
        self.references.outcome = if !self.active[8] {
            SKIPPED
        } else {
            let _reads = read_accounting::table(TableId::AirReferenceWitnesses);
            capture_references(
                memory,
                base,
                wings,
                registered,
                &mut self.handles,
                &mut self.references.rows,
            )
            .unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[8] = elapsed(begin);
        let begin = qpc();
        self.mines.outcome = if !self.active[9] {
            SKIPPED
        } else {
            let _reads = read_accounting::table(TableId::NavalMineLevels);
            capture_mines(
                memory,
                base,
                game_state,
                wings,
                registered,
                &mut self.handles,
                &mut self.mines.rows,
            )
            .unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[9] = elapsed(begin);
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.equipment.outcome,
            self.bases.outcome,
            self.cumulative.outcome,
            self.history.outcome,
            self.damage.outcome,
            self.aces.outcome,
            self.statistics.outcome,
            self.registrations.outcome,
            self.references.outcome,
            self.mines.outcome,
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
            table::<tables::AirRegionalMissions, _>(outcomes[7], &self.registrations.rows, |r| {
                rows::AirRegionalMissionRow {
                    wing: r.wing,
                    ordinal: r.ordinal,
                }
            }),
            air_reference_witness::table_view(
                outcomes[8],
                &self.references.rows[..outcomes[8].len().min(self.references.rows.len())],
                match outcomes[1] {
                    Outcome::Rows { len } => self.bases.rows.get(..len),
                    _ => None,
                },
            ),
            table::<tables::NavalMineLevels, _>(outcomes[9], &self.mines.rows, Clone::clone),
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

fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut [R]) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
}

/// Each registration's wing; `wings` is sorted by handle.
fn registered_wings<'a>(
    wings: &'a [WingIdentity],
    registered: Option<&'a [cair_regional_mission::Row]>,
) -> Result<impl Iterator<Item = Result<&'a WingIdentity, &'static str>>, &'static str> {
    let registered =
        registered.ok_or("air mission joins lack the complete registration census")?;
    Ok(registered.iter().map(move |registration| {
        wings
            .binary_search_by_key(&registration.wing, |wing| wing.wing)
            .map(|index| &wings[index])
            .map_err(|_| "registered mission missing from the wing census")
    }))
}

/// Probe each registered wing's pool base, each unregistered training wing's
/// pool base (`14122EFD0`'s training pass) and every transfer base once. A
/// wing is registered only with a region, so the scratch's two entries per
/// wing cover every probe.
fn capture_references(
    memory: &impl Memory,
    base: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    handles: &mut [u64],
    output: &mut [BaseWitness],
) -> Result<Outcome, &'static str> {
    let mut required = 0;
    let mut want = |handle: u64| {
        if let Some(slot) = handles.get_mut(required) {
            *slot = handle;
        }
        required += 1;
    };
    for wing in registered_wings(wings, registered)? {
        let wing = wing?;
        if wing.air_base == 0 {
            return Err("registered wing has a null pool base");
        }
        want(wing.air_base);
    }
    for wing in wings.iter().filter(|wing| {
        wing.region.is_none()
            && wing.mission_mask & rows::AIR_MISSION_TRAINING != 0
            && wing.air_base != 0
    }) {
        want(wing.air_base);
    }
    for wing in wings.iter().filter(|wing| wing.base_override != 0) {
        want(wing.base_override);
    }
    if required > handles.len() {
        return Ok(Outcome::Truncated {
            len: 0,
            unread_rows: required.min(u32::MAX as usize) as u32,
        });
    }
    let probes = &mut handles[..required];
    probes.sort_unstable();
    let mut len = 0;
    let mut unread_rows = 0_u32;
    let mut previous = 0;
    for &handle in probes.iter() {
        if handle == previous {
            continue;
        }
        previous = handle;
        let witness = BaseWitness {
            handle,
            base: map_air_base_reference::read_base(memory, base, handle)?,
        };
        match output.get_mut(len) {
            Some(slot) => {
                *slot = witness;
                len += 1;
            }
            None => unread_rows = unread_rows.saturating_add(1),
        }
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

/// `CanLayMinesInRegion@140F65E40`: each registered mine-laying wing's
/// effective-country mines in its mission region, once per pair. `pairs` is
/// scratch with one entry per registration.
fn capture_mines(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    pairs: &mut [u64],
    output: &mut [rows::NavalMineLevelRow],
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut count = 0;
    for wing in registered_wings(wings, registered)? {
        let wing = wing?;
        let Some(region) = wing
            .region
            .filter(|_| wing.mission_mask & rows::AIR_MISSION_LAY_MINES != 0)
        else {
            continue;
        };
        let country = countries.resolve(wing.effective_country())?;
        *pairs
            .get_mut(count)
            .ok_or("mine pair scratch smaller than the registrations")? =
            u64::from(country.canonical_slot) << 32 | u64::from(region);
        count += 1;
    }
    let pairs = &mut pairs[..count];
    pairs.sort_unstable();
    let mut len = 0;
    let mut unread_rows = 0_u32;
    let mut previous = None;
    for &pair in pairs.iter() {
        if previous.replace(pair) == Some(pair) {
            continue;
        }
        let Some(slot) = output.get_mut(len) else {
            unread_rows = unread_rows.saturating_add(1);
            continue;
        };
        let (country, region) = ((pair >> 32) as u32, pair as u32);
        let country = countries
            .at(country)?
            .ok_or("mine-laying country slot is empty")?;
        *slot = rows::NavalMineLevelRow {
            country: country.canonical_slot,
            region,
            raw_mines: cstrategic_navy_manager::regional_operand(
                memory, base, game_state, country, region,
            )?,
        };
        len += 1;
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
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

/// One complete source census, independent of the bounded map projection.
/// All transient addresses stay owned opaque scalars and are consumed only
/// during acquisition; the worker installs value rows without memory reads.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WingMembershipRead {
    pub value: engine::ctheatre::WingMembership,
    pub error: Option<&'static str>,
}

#[derive(Debug)]
pub(crate) struct AirWingJob {
    pub(crate) map_rows: Vec<cair_wing::Row>,
    sources: Vec<cair_wing::Source>,
    equipment: Vec<rows::WingEquipmentRow>,
    cumulative: Vec<cair_wing::CumulativeRow>,
    memberships: Vec<WingMembershipRead>,
    source_len: usize,
    pub(crate) map_outcome: Outcome,
    source_outcome: Outcome,
    naval_outcome: Outcome,
    equipment_outcome: Outcome,
    cumulative_outcome: Outcome,
    cumulative_active: bool,
    pub(crate) theatres_active: bool,
    limits: [usize; 3],
    required: [usize; 3],
    required_memberships: usize,
    acquired: bool,
    pub(crate) membership_ticks: u64,
    raw_bytes: [u64; WING_PACK_N],
    capacity_charge: crate::raw_memory::Charge,
}

impl Default for AirWingJob {
    fn default() -> Self {
        Self {
            map_rows: Vec::new(),
            sources: Vec::new(),
            equipment: Vec::new(),
            cumulative: Vec::new(),
            memberships: Vec::new(),
            source_len: 0,
            map_outcome: NO_WING_SOURCE,
            source_outcome: NO_WING_SOURCE,
            naval_outcome: NO_WING_SOURCE,
            equipment_outcome: NO_WING_SOURCE,
            cumulative_outcome: NO_WING_SOURCE,
            cumulative_active: false,
            theatres_active: false,
            limits: [0; 3],
            required: [0; 3],
            required_memberships: 0,
            acquired: false,
            membership_ticks: 0,
            raw_bytes: [0; 2],
            capacity_charge: Default::default(),
        }
    }
}

impl AirWingJob {
    pub fn new() -> Self {
        let mut job = Self {
            map_rows: vec![Default::default(); 8192],
            sources: vec![Default::default(); 8192],
            equipment: vec![Default::default(); 8192],
            cumulative: vec![Default::default(); 4096],
            memberships: vec![Default::default(); 8192],
            ..Default::default()
        };
        job.update_capacity_charge();
        job
    }
    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.map_rows)
                + vector_bytes(&self.equipment)
                + vector_bytes(&self.cumulative)
                + vector_bytes(&self.memberships),
            auxiliary_vectors: vector_bytes(&self.sources),
            ..Default::default()
        });
    }
    pub fn has_storage(&self) -> bool {
        !self.sources.is_empty()
    }
    pub fn clear(&mut self) {
        self.source_len = 0;
        self.map_outcome = NO_WING_SOURCE;
        self.source_outcome = NO_WING_SOURCE;
        self.naval_outcome = NO_WING_SOURCE;
        self.equipment_outcome = NO_WING_SOURCE;
        self.cumulative_outcome = NO_WING_SOURCE;
        self.cumulative_active = false;
        self.theatres_active = false;
        self.limits = [0; 3];
        self.required = [0; 3];
        self.required_memberships = 0;
        self.acquired = false;
        self.membership_ticks = 0;
        self.raw_bytes = [0; 2];
    }
    pub fn grow_for_next(&self) -> bool {
        self.required
            .iter()
            .zip([
                self.map_rows.len(),
                self.equipment.len(),
                self.cumulative.len(),
            ])
            .any(|(&required, present)| required > present)
            || matches!(self.source_outcome, Outcome::Truncated { .. })
            || self.required_memberships > self.memberships.len()
    }
    pub fn grow(&mut self) {
        self.map_rows.resize(
            self.required[0].max(self.map_rows.len()),
            Default::default(),
        );
        self.equipment.resize(
            self.required[1].max(self.equipment.len()),
            Default::default(),
        );
        self.cumulative.resize(
            self.required[2].max(self.cumulative.len()),
            Default::default(),
        );
        if self.sources.is_empty() {
            self.sources.resize(8192, Default::default());
        }
        if self.required_memberships > self.memberships.len() {
            self.memberships
                .resize(self.required_memberships, Default::default());
        }
        if let Outcome::Truncated { len, unread_rows } = self.source_outcome {
            let required = len
                .saturating_add(unread_rows as usize)
                .max(self.sources.len().saturating_mul(2));
            self.sources.resize(required, Default::default());
        }
        self.update_capacity_charge();
    }
    pub(crate) fn memberships(&self) -> (Outcome, &[WingMembershipRead]) {
        (self.naval_outcome, &self.memberships[..self.source_len])
    }
    pub(crate) fn map_memberships(&self) -> &[WingMembershipRead] {
        if self.theatres_active {
            &self.memberships[..self.map_outcome.len()]
        } else {
            &[]
        }
    }
}

/// Successful native byte counts for the shared wing census and its extensions.
struct WingMemory<'a, M> {
    inner: &'a M,
    bytes: std::cell::Cell<[u64; 2]>,
    mask: std::cell::Cell<u8>,
}
impl<M: Memory> WingMemory<'_, M> {
    fn count(&self, len: usize) {
        let mask = self.mask.get();
        if mask == 0 {
            return;
        }
        let mut total = self.bytes.get();
        let slot = usize::from(mask == 2);
        total[slot] = total[slot].saturating_add(len as u64);
        self.bytes.set(total);
    }
}
impl<M: Memory> Memory for WingMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.inner.read_scalar::<T>(address);
        if value.is_some() {
            self.count(size_of::<T>());
        }
        value
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ok = self.inner.copy(address, destination);
        if ok {
            self.count(destination.len());
        }
        ok
    }
    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        let completed = self.inner.copy_scattered(addresses, destinations);
        for destination in destinations.iter().take(completed) {
            self.count(destination.len());
        }
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirWingCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; WING_PACK_N],
    pub raw_bytes: [u64; WING_PACK_N],
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct AirWingCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    /// Independent map, equipment and cumulative destination limits.
    pub limits: [usize; 3],
    /// Independently pooled naval source capacity.
    pub naval_limit: usize,
}

impl Captured {
    pub(crate) fn wing_limits(&self) -> [usize; 2] {
        [self.equipment.rows.len(), self.cumulative.rows.len()]
    }

    /// Complete a failed owned wing job after the typed visitors were gated.
    /// Other air tables retain their independently acquired results and costs.
    pub(crate) fn fail_air_wing_capture(&mut self, reason: &'static str) {
        self.equipment.outcome = Outcome::Unavailable { reason };
        self.cumulative.outcome = failed(self.active[2], reason);
    }

    /// Complete a failed statistics job after its typed walk was gated.
    pub(crate) fn fail_air_stats_capture(&mut self, reason: &'static str) {
        self.statistics.outcome = failed(self.active[6], reason);
        self.stats_raw_mode = false;
    }
}

fn copy_air_pass(job: &mut AirWingJob, memory: &impl Memory, ctx: AirWingCopyContext) {
    let mut reads = read_accounting::table(TableId::WingEquipment);
    job.clear();
    job.cumulative_active =
        replay::cadence::should_walk(TableId::WingCumulative, ctx.hour, ctx.force_full);
    job.theatres_active =
        replay::cadence::should_walk(TableId::AirTheatreMembers, ctx.hour, ctx.force_full);
    job.limits = ctx.limits;
    if !job.cumulative_active {
        job.limits[2] = 0;
    }
    job.required = job.limits;
    let naval_limit = if job.theatres_active {
        ctx.naval_limit
    } else {
        0
    };
    job.required_memberships = if job.theatres_active {
        ctx.limits[0].max(naval_limit)
    } else {
        0
    };
    let source = WingMemory {
        inner: memory,
        bytes: std::cell::Cell::new([0; 2]),
        mask: std::cell::Cell::new(3),
    };
    let mut equipment = Outcome::Rows { len: 0 };
    let mut cumulative = if job.cumulative_active {
        Outcome::Rows { len: 0 }
    } else {
        SKIPPED
    };
    let mut source_len = 0;
    let mut membership_len = 0;
    let mut membership_ticks = 0;
    let walked = cair_wing::walk_shared(
        &source,
        ctx.base,
        ctx.game_state,
        &mut job.map_rows[..ctx.limits[0]],
        &mut job.sources,
        naval_limit,
        |pointer, _, row, admitted| {
            let wing = WingSource { pointer, row };
            if admitted.sources && !matches!(equipment, Outcome::Unavailable { .. }) {
                source.mask.set(1);
                let next = read_equipment(
                    &source,
                    ctx.base,
                    &wing,
                    &mut job.equipment[equipment.len()..ctx.limits[1]],
                );
                equipment = append(equipment, next);
            }
            if admitted.sources && !matches!(cumulative, Outcome::Unavailable { .. }) {
                reads.select(TableId::WingCumulative);
                source.mask.set(2);
                let next = if let Some(destination) =
                    job.cumulative[..ctx.limits[2]].get_mut(cumulative.len())
                {
                    match cair_wing::read_cumulative(&source, pointer, row) {
                        Ok(value) => {
                            *destination = value;
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
                cumulative = append(cumulative, next);
            }
            if job.theatres_active && (admitted.map || admitted.naval) {
                let _reads = read_accounting::table(TableId::AirTheatreMembers);
                source.mask.set(0);
                let started = qpc();
                job.memberships[membership_len] =
                    match engine::ctheatre::read_wing_membership(&source, pointer, row) {
                        Ok(value) => WingMembershipRead { value, error: None },
                        Err(reason) => WingMembershipRead {
                            error: Some(reason),
                            ..Default::default()
                        },
                    };
                membership_ticks += elapsed(started);
                membership_len += 1;
                source_len += usize::from(admitted.naval);
            }
            source.mask.set(3);
            reads.select(TableId::WingEquipment);
        },
    );
    job.map_outcome = walked.map;
    job.source_outcome = walked.sources;
    job.naval_outcome = if job.theatres_active {
        walked.naval
    } else {
        SKIPPED
    };
    job.equipment_outcome = require_parent(walked.sources, equipment);
    job.cumulative_outcome = if job.cumulative_active {
        require_parent(walked.sources, cumulative)
    } else {
        SKIPPED
    };
    job.source_len = source_len;
    job.membership_ticks = membership_ticks;
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}

pub(crate) fn copy_air_refresh(
    memory: &impl Memory,
    ctx: AirWingCopyContext,
    job: &mut AirWingJob,
) -> AirWingCopyReport {
    let started = qpc();
    let cumulative_active =
        replay::cadence::should_walk(TableId::WingCumulative, ctx.hour, ctx.force_full);
    let theatres_active =
        replay::cadence::should_walk(TableId::AirTheatreMembers, ctx.hour, ctx.force_full);
    let mut limits = ctx.limits;
    if !cumulative_active {
        limits[2] = 0;
    }
    let memberships = if theatres_active {
        ctx.limits[0].max(ctx.naval_limit)
    } else {
        0
    };
    let fits = job.has_storage()
        && job.memberships.len() >= memberships
        && limits
            .iter()
            .zip([
                job.map_rows.len(),
                job.equipment.len(),
                job.cumulative.len(),
            ])
            .all(|(&required, capacity)| required <= capacity);
    if !fits {
        job.clear();
        job.cumulative_active = cumulative_active;
        job.theatres_active = theatres_active;
        job.required = limits;
        job.required_memberships = memberships;
        return AirWingCopyReport::default();
    }
    copy_air_pass(job, memory, ctx);
    AirWingCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started).saturating_sub(job.membership_ticks), 0],
        raw_bytes: job.raw_bytes,
    }
}

pub(crate) fn decode_air_payload(
    captured: &mut Captured,
    job: &AirWingJob,
    decode_ticks: &mut [u64; WING_PACK_N],
) {
    let started = qpc();
    captured.active[2] = job.cumulative_active;
    if !job.acquired
        || captured.equipment.rows.len() != job.limits[1]
        || (job.cumulative_active && captured.cumulative.rows.len() != job.limits[2])
    {
        captured.equipment.outcome = NO_WING_SOURCE;
        captured.cumulative.outcome = if job.cumulative_active {
            NO_WING_SOURCE
        } else {
            SKIPPED
        };
    } else {
        let equipment = job.equipment_outcome.len();
        let cumulative = job.cumulative_outcome.len();
        // WingEquipmentRow owns only integer scalars; cloning does not allocate.
        captured.equipment.rows[..equipment].clone_from_slice(&job.equipment[..equipment]);
        if job.cumulative_active {
            captured.cumulative.rows[..cumulative].copy_from_slice(&job.cumulative[..cumulative]);
        }
        captured.equipment.outcome = job.equipment_outcome;
        captured.cumulative.outcome = job.cumulative_outcome;
    }
    decode_ticks[0] += elapsed(started);
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
    active: bool,
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
            active: false,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
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
    pub copy_ticks: [u64; AIR_STATS_N],
    pub raw_bytes: [u64; AIR_STATS_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirStatsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
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
    let _reads = read_accounting::table(TableId::AirStatistics);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    let started = qpc();
    // Single per-owner loop (no duplicate takes): census takes and history
    // takes interleave in owner order. The replay serves by exact
    // (address, length) with in-order multiplicity, so take order never
    // affects serving. Only this acquisition supplies the owned decode.
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
    memory: &impl Memory,
    ctx: AirStatsCopyContext,
    job: &mut AirStatsJob,
) -> AirStatsCopyReport {
    if !replay::cadence::should_walk(TableId::AirStatistics, ctx.hour, ctx.force_full) {
        job.clear();
        return AirStatsCopyReport::default();
    }
    let mut ticks = [0u64; AIR_STATS_N];
    copy_stats_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    AirStatsCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

pub(crate) fn decode_air_stats_payload(
    captured: &mut Captured,
    base: usize,
    job: &AirStatsJob,
    decode_ticks: &mut [u64; AIR_STATS_N],
) {
    captured.active[6] = job.active;
    if !job.active {
        captured.fail_air_stats_capture(replay::cadence::SKIPPED_REASON);
        return;
    }
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
// Acquisition workers copy raw bytes: one authenticated country census,
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
    /// Complete registration-row and wing-handle demand from the original
    /// subtree acquisition. Shared census faults can coexist with these counts;
    /// the worker rejects faulted hints and the owned walk validates all fields.
    pub(crate) registration_demand: Option<[usize; 2]>,
    aces_active: bool,
    /// Canonical (pointer, slot) countries parsed from the census takes.
    census: Vec<(usize, u32)>,
    /// Link raws already taken this pass with their first successfully
    /// staged slot values. Failed resolutions are not cached.
    seen_links: Vec<(u32, u32)>,
    /// Direct-map hints into `seen_links`, reset each pass. Collisions use
    /// the authoritative list, never a second observation of live memory.
    /// Once grown to the live link count every resolvable raw has its own cell.
    seen_link_index: Vec<usize>,
    /// Live census demand observed this pass: canonical country slots (the
    /// country count less the sentinel) and country links. Growth sizes
    /// `census` and `seen_links` to these in one step, even when the pass also
    /// recorded a source fault, so a large census cannot keep overflowing.
    required_census: usize,
    required_links: usize,
    capacity_charge: crate::raw_memory::Charge,
}

impl AirBasesJob {
    pub fn new() -> Self {
        let seen_links = Vec::with_capacity(crate::country_scale::census_rows(1024));
        let seen_link_index = vec![usize::MAX; seen_links.capacity()];
        let mut job = Self {
            // Late-campaign worst case is 396 bases with 5,148 registration
            // rows plus wing vectors and 57 aces with names (appendix
            // census); both pools start above that so steady state never
            // overflows (see the worst-case caps test).
            log: RawLog::new(1 << 17, 8 << 20),
            game_state: 0,
            registration_demand: None,
            aces_active: false,
            census: Vec::with_capacity(crate::country_scale::census_rows(1024)),
            seen_links,
            seen_link_index,
            required_census: 0,
            required_links: 0,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.registration_demand = None;
        self.aces_active = false;
        self.census.clear();
        self.seen_links.clear();
        self.seen_link_index.fill(usize::MAX);
        self.required_census = 0;
        self.required_links = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
            || self.required_census > self.census.capacity()
            || self.required_links > self.seen_links.capacity()
    }

    /// Off-thread only. Census storage jumps to the observed live demand;
    /// otherwise it keeps the existing doubling after a staging overflow.
    pub fn grow(&mut self) {
        fn grow_to<T>(rows: &mut Vec<T>, required: usize) {
            if required > rows.capacity() {
                rows.reserve_exact(required - rows.len());
            } else {
                crate::raw::double_capacity(rows);
            }
        }
        self.log.grow();
        grow_to(&mut self.census, self.required_census);
        grow_to(&mut self.seen_links, self.required_links);
        self.seen_link_index
            .resize(self.seen_links.capacity(), usize::MAX);
        self.seen_link_index.fill(usize::MAX);
        self.update_capacity_charge();
    }

    /// Off the game thread: one census row and one link hint per observed
    /// country slot, so a large save's first hour does not overflow.
    pub fn presize_countries(&mut self) {
        let census = crate::country_scale::reserve_census(&mut self.census);
        if crate::country_scale::reserve_census(&mut self.seen_links) {
            self.seen_link_index
                .resize(self.seen_links.capacity(), usize::MAX);
            self.seen_link_index.fill(usize::MAX);
        } else if !census {
            return;
        }
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
/// refresh on a campaign/load signal, clock discontinuity or refresh request; the
/// worker additionally requires adjacent sequence/hour and the same Shared.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AirBasesMemoContext {
    pub owner: usize,
    pub base: usize,
    pub seq: u64,
    pub hour: i32,
    pub force_full: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AirBasesMemoKey {
    owner: usize,
    base: usize,
    game_state: usize,
    // Logical slice lengths control admission and therefore observations.
    // Exact equality is deliberately more conservative than "large enough".
    capacities: [usize; 5],
    aces_active: bool,
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
/// pooled job/frame. Every capture still acquires current bytes before the game resumes.
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
        let aces_active = self.key.is_none_or(|key| key.aces_active);
        !log.overflowed
            && !log
                .fault
                .iter()
                .enumerate()
                .any(|(slot, fault)| *fault && (slot < 2 || aces_active))
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
        air.active[5] = job.aces_active;
        let compared_started = qpc();
        let adjacent = self.previous.is_some_and(|(seq, hour)| {
            seq.checked_add(1) == Some(ctx.seq) && hour.checked_add(1) == Some(ctx.hour)
        });
        if ctx.force_full || !adjacent {
            self.key = None;
        }
        // Storage must be final before comparing or installing the capacity
        // fingerprint: another pooled frame may still have the old limits.
        fog.prepare_registrations_payload(job);
        let capacity = air
            .bases_memo_context()
            .zip(fog.registrations_memo_context());
        let key = capacity.map(|(air, fog)| AirBasesMemoKey {
            owner: ctx.owner,
            base: ctx.base,
            game_state: job.game_state,
            capacities: [air[0], air[1], air[2], fog[0], fog[1]],
            aces_active: job.aces_active,
        });
        report.eligible = key.is_some()
            && !job.log.overflowed
            && !job
                .log
                .fault
                .iter()
                .enumerate()
                .any(|(slot, fault)| *fault && (slot < 2 || job.aces_active))
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
            if job.aces_active {
                let started = qpc();
                air.aces.rows[..self.aces.len()].copy_from_slice(&self.aces);
                air.aces.outcome = Outcome::Rows {
                    len: self.aces.len(),
                };
                air.names.first[..self.names.len()].copy_from_slice(&self.names);
                air.names.len = self.names.len();
                air.names.required = self.required_names;
                decode_ticks[2] += elapsed(started);
            } else {
                air.aces.outcome = SKIPPED;
                air.names.reset();
            }
            air.bases_raw_mode = false;
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
            self.names.clear();
            self.required_names = 0;
            if job.aces_active {
                self.aces
                    .extend_from_slice(&air.aces.rows[..air.aces.outcome.len()]);
                self.names
                    .extend_from_slice(&air.names.first[..air.names.len]);
                self.required_names = air.names.required;
            }
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
        self.bases_raw_mode.then_some([
            self.bases.rows.len(),
            if self.active[5] {
                self.aces.rows.len()
            } else {
                0
            },
            if self.active[5] {
                self.names.first.len()
            } else {
                0
            },
        ])
    }

    fn bases_memo_complete(&self) -> bool {
        matches!(self.bases.outcome, Outcome::Rows { len } if len <= self.bases.rows.len())
            && (!self.active[5]
                || (matches!(self.aces.outcome, Outcome::Rows { .. })
                    && self.names.required >= self.names.len
                    && valid_names(
                        self.aces.outcome,
                        &self.aces.rows,
                        &self.names.first,
                        self.names.len,
                    )))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirBasesCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walks (identical rows through the existing paths) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; AIR_BASES_N],
    pub raw_bytes: [u64; AIR_BASES_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirBasesCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
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
    mask: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(log, memory, links.data.checked_add(offset), 4, mask)?;
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
/// `required` receives the live [canonical slot, link] demand as soon as both
/// headers parse, so an overflowing census still sizes its growth exactly.
fn copy_air_bases_census(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<(usize, u32)>,
    required: [&mut usize; 2],
    mask: u32,
) -> Option<(Array, u32)> {
    let countries_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        mask,
    )?;
    let links_start = bspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        mask,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let [required_census, required_links] = required;
    *required_census = countries.count as usize - 1;
    *required_links = links.count as usize;
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let Some(pointer_start) = bspan(log, memory, pointer_addr, 8, mask) else {
            continue;
        };
        let pointer = match usize::try_from(bases_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = bspan(log, memory, Some(pointer), 8, mask);
        let id_start = bspan(log, memory, pointer.checked_add(ccountry::ID), 4, mask);
        let (Some(vtable_start), Some(id_start)) = (vtable_start, id_start) else {
            continue;
        };
        if bases_pool_u64(log, vtable_start) != vtable {
            continue;
        }
        let id = bases_pool_u32(log, id_start);
        let canonical = bases_link(log, memory, &links, countries.count, id, mask);
        let slot_link = bases_link(log, memory, &links, countries.count, slot, mask);
        let (Some(canonical), Some(slot_link)) = (canonical, slot_link) else {
            continue;
        };
        if slot_link != canonical {
            continue;
        }
        let check = bases_link(log, memory, &links, countries.count, canonical, mask);
        if check != Some(canonical) {
            continue;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = bspan(log, memory, canonical_addr, 8, mask);
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
    // Each insertion fills its bucket until the next pass clears both stores.
    // An empty bucket proves absence; collisions and invalid hints still need
    // the authoritative list, as does the unindexed path.
    if bucket.is_none_or(|bucket| index[bucket] != usize::MAX)
        && let Some(position) = seen.iter().position(|(key, _)| *key == raw)
    {
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
    demand: &mut Option<[usize; 2]>,
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
            // Retain the completed pointer prefix even when a later element
            // faults: earlier containers still need validation in source order.
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
        *demand = demand.and_then(|[rows, wings]| Some([rows.checked_add(1)?, wings]));
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
                *demand = demand.and_then(|[rows, wings]| {
                    Some([rows, wings.checked_add(members.count as usize)?])
                });
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
    aces_active: bool,
    ticks: &mut [u64; AIR_BASES_N],
) {
    let started = qpc();
    job.clear();
    job.game_state = game_state;
    job.aces_active = aces_active;
    let census_mask = if aces_active {
        AIR_BASES_CENSUS_MASK
    } else {
        BASE_VECTORS_MASK
    };
    let census = read_accounting::with_table(TableId::AirBases, || {
        copy_air_bases_census(
            &mut job.log,
            memory,
            base,
            game_state,
            &mut job.census,
            [&mut job.required_census, &mut job.required_links],
            census_mask,
        )
    });
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
    if job.log.overflowed || !aces_active {
        return;
    }
    // Aces phase: independent of the base phases, served by the shared
    // census takes plus roster/object/name takes below. Index iteration
    // avoids cloning the census on the game thread.
    let _reads = read_accounting::table(TableId::AirAces);
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
    let _reads = read_accounting::table(TableId::AirBases);
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
    let mut registration_demand = Some([0usize; 2]);
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
                    let _reads = read_accounting::table(TableId::AirBaseRegistrations);
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
                        &mut registration_demand,
                    ) {
                        reg_alive = false;
                    }
                    reg_ticks += elapsed(begin);
                }
            } else if reg_alive {
                // The bases walk already stopped: the registrations walk
                // still needs the shared identity take plus its own takes.
                let _reads = read_accounting::table(TableId::AirBaseRegistrations);
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
                    &mut registration_demand,
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
    if reg_alive && !job.log.overflowed {
        job.registration_demand = registration_demand;
    }
    ticks[0] += elapsed(vectors_started)
        .saturating_sub(bases_ticks)
        .saturating_sub(reg_ticks);
    ticks[0] += bases_ticks;
    ticks[1] += reg_ticks;
}

/// Acquire the shared bases, registrations and aces into one owned payload.
pub(crate) fn copy_air_bases_refresh(
    memory: &impl Memory,
    ctx: AirBasesCopyContext,
    job: &mut AirBasesJob,
) -> AirBasesCopyReport {
    let mut ticks = [0u64; AIR_BASES_N];
    copy_air_bases_pass(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        replay::cadence::should_walk(TableId::AirAces, ctx.hour, ctx.force_full),
        &mut ticks,
    );
    AirBasesCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Install an accepted worker failure after the ordinary capture was gated.
    /// Registrations are failed separately by their fog owner; no live retry runs.
    pub(crate) fn fail_air_bases_capture(&mut self, reason: &'static str) {
        self.bases.outcome = Outcome::Unavailable { reason };
        self.aces.outcome = failed(self.active[5], reason);
        self.names.reset();
        self.bases_raw_mode = false;
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
        self.active[5] = job.aces_active;
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let walked = cair_base::walk(&recorded, base, job.game_state, &mut self.bases.rows);
        self.bases.outcome = bounded(walked, self.bases.rows.len());
        decode_ticks[0] += elapsed(started);
        if !job.aces_active {
            self.aces.outcome = SKIPPED;
            self.names.reset();
            self.bases_raw_mode = false;
            return;
        }
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

/// Shared regional-history payload slots in report order: [history, damage].
pub(crate) const AIR_HISTORY_N: usize = 2;
const AIR_DAMAGE_MASK: u8 = 2;
const AIR_HISTORY_CENSUS_MASK: u8 = 3;
/// One full acquisition owns native values, not a virtual copy of game memory.
/// The worker copies these values before returning the job to its pool.
#[derive(Debug)]
pub(crate) struct AirHistoryJob {
    history: Vec<sair_wing_combat_data::Row>,
    damage: Vec<sdamage_entry::Row>,
    history_result: Outcome,
    damage_result: Outcome,
    history_limit: usize,
    damage_limit: usize,
    required_history: usize,
    required_damage: usize,
    acquired: bool,
    active: bool,
    raw_bytes: [u64; AIR_HISTORY_N],
    capacity_charge: crate::raw_memory::Charge,
}
impl Default for AirHistoryJob {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            damage: Vec::new(),
            history_result: Outcome::Unavailable {
                reason: "air history not acquired",
            },
            damage_result: Outcome::Unavailable {
                reason: "air history not acquired",
            },
            history_limit: 0,
            damage_limit: 0,
            required_history: 0,
            required_damage: 0,
            acquired: false,
            active: false,
            raw_bytes: [0; AIR_HISTORY_N],
            capacity_charge: Default::default(),
        }
    }
}
impl AirHistoryJob {
    pub fn new() -> Self {
        let mut job = Self {
            history: vec![sair_wing_combat_data::Row::default(); 8192],
            damage: vec![sdamage_entry::Row::default(); 16_384],
            ..Self::default()
        };
        job.update_capacity_charge();
        job
    }
    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.history) + vector_bytes(&self.damage),
            ..Capacity::default()
        });
    }
    pub fn has_storage(&self) -> bool {
        !self.history.is_empty() && !self.damage.is_empty()
    }
    pub fn clear(&mut self) {
        self.history_result = Outcome::Unavailable {
            reason: "air history not acquired",
        };
        self.damage_result = self.history_result;
        self.history_limit = 0;
        self.damage_limit = 0;
        self.required_history = 0;
        self.required_damage = 0;
        self.acquired = false;
        self.active = false;
        self.raw_bytes = [0; AIR_HISTORY_N];
    }
    pub fn grow_for_next(&self) -> bool {
        self.required_history > self.history.len() || self.required_damage > self.damage.len()
    }
    pub fn grow(&mut self) {
        fn target(current: usize, required: usize) -> usize {
            if required > current {
                required.max(current.saturating_mul(2))
            } else {
                current
            }
        }
        self.history.resize(
            target(self.history.len(), self.required_history),
            Default::default(),
        );
        self.damage.resize(
            target(self.damage.len(), self.required_damage),
            Default::default(),
        );
        self.update_capacity_charge();
    }
}

/// Meter the existing native walker without retaining a second observation.
struct AirHistoryMemory<'a, M> {
    inner: &'a M,
    raw_bytes: std::cell::Cell<[u64; AIR_HISTORY_N]>,
    record: std::cell::Cell<Option<usize>>,
}
impl<M: Memory> AirHistoryMemory<'_, M> {
    fn structural_mask(&self, address: usize, len: usize) -> u8 {
        if len == 16
            && self.record.get().is_some_and(|record| {
                record.checked_add(sair_wing_combat_data::FIRST_DAMAGE) == Some(address)
                    || record.checked_add(sair_wing_combat_data::SECOND_DAMAGE) == Some(address)
            })
        {
            AIR_DAMAGE_MASK
        } else {
            AIR_HISTORY_CENSUS_MASK
        }
    }
    fn count(&self, mask: u8, bytes: usize) {
        let mut values = self.raw_bytes.get();
        let slot = usize::from(mask == AIR_DAMAGE_MASK);
        values[slot] = values[slot].saturating_add(bytes as u64);
        self.raw_bytes.set(values);
    }
}
impl<M: Memory> Memory for AirHistoryMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let bytes = size_of::<T>();
        let value = self.inner.read_scalar::<T>(address);
        if value.is_some() {
            self.count(self.structural_mask(address, bytes), bytes);
        }
        value
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let succeeded = self.inner.copy(address, destination);
        if succeeded {
            self.count(
                self.structural_mask(address, destination.len()),
                destination.len(),
            );
        }
        succeeded
    }
    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        let record_leaf = K == 3
            && destinations[0].len() == 26
            && destinations[1].len() == 4
            && destinations[2].len() == 1
            && addresses[0].checked_add(sair_wing_combat_data::ATTRIBUTED_COUNTRY)
                == Some(addresses[1])
            && addresses[0].checked_add(sair_wing_combat_data::COPIED_FLAG) == Some(addresses[2]);
        let damage_leaf = K == 2
            && destinations[0].len() == 12
            && destinations[1].len() == 8
            && addresses[0].checked_add(sdamage_entry::AMOUNT) == Some(addresses[1]);
        if record_leaf {
            self.record.set(Some(addresses[0]));
        }
        let completed = self.inner.copy_scattered(addresses, destinations);
        for (index, destination) in destinations.iter().take(completed).enumerate() {
            let mask = if damage_leaf {
                AIR_DAMAGE_MASK
            } else if record_leaf {
                AIR_HISTORY_CENSUS_MASK
            } else {
                self.structural_mask(addresses[index], destination.len())
            };
            self.count(mask, destination.len());
        }
        completed
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.record.set(None);
        let completed = self.inner.copy_records(sources, layout, destination);
        let count = completed.min(sources.len().saturating_mul(layout.field_count()));
        self.count(AIR_HISTORY_CENSUS_MASK, layout.completed_bytes(count));
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AirHistoryCopyReport {
    /// Owned staging ready. Missing row storage uses the existing typed fallback;
    /// a native fault never retries an accepted acquisition.
    pub raw_ok: bool,
    /// Now includes the one direct native walk; worker cost is only installation.
    pub copy_ticks: [u64; AIR_HISTORY_N],
    /// Actual successfully acquired bytes, anchor-charged like the old mirror.
    pub raw_bytes: [u64; AIR_HISTORY_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AirHistoryCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    /// Exact native row limits of this frame's destination buffers.
    pub history_limit: usize,
    pub damage_limit: usize,
}

fn acquire_air_history_pass(
    job: &mut AirHistoryJob,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    history_limit: usize,
    damage_limit: usize,
) {
    let _reads = read_accounting::table(TableId::AirRegionHistory);
    job.clear();
    job.active = true;
    job.required_history = history_limit;
    job.required_damage = damage_limit;
    job.history_limit = history_limit;
    job.damage_limit = damage_limit;
    if history_limit > job.history.len() || damage_limit > job.damage.len() {
        return;
    }
    let source = AirHistoryMemory {
        inner: memory,
        raw_bytes: std::cell::Cell::new([0; AIR_HISTORY_N]),
        record: std::cell::Cell::new(None),
    };
    let captured = cair_region_combat_data::walk_world(
        &source,
        base,
        game_state,
        &mut job.history[..history_limit],
        &mut job.damage[..damage_limit],
    );
    job.history_result = captured.records;
    job.damage_result = captured.damage;
    job.acquired = true;
    job.raw_bytes = source.raw_bytes.get();
}

/// Acquire regional history and damage after checking both destination limits.
pub(crate) fn copy_air_history_refresh(
    memory: &impl Memory,
    ctx: AirHistoryCopyContext,
    job: &mut AirHistoryJob,
) -> AirHistoryCopyReport {
    if !replay::cadence::should_walk(TableId::AirRegionHistory, ctx.hour, ctx.force_full) {
        job.clear();
        return AirHistoryCopyReport::default();
    }
    let started = qpc();
    acquire_air_history_pass(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        ctx.history_limit,
        ctx.damage_limit,
    );
    AirHistoryCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started), 0],
        raw_bytes: job.raw_bytes,
    }
}

impl Captured {
    /// Native row limits carried into an independently submitted acquisition.
    pub(crate) fn air_history_limits(&self) -> [usize; AIR_HISTORY_N] {
        [self.history.rows.len(), self.damage.rows.len()]
    }

    /// Install an accepted worker failure after the ordinary capture was gated.
    pub(crate) fn fail_air_history_capture(&mut self, reason: &'static str) {
        self.history.outcome = failed(self.active[3], reason);
        self.damage.outcome = failed(self.active[4], reason);
        self.history_raw_mode = false;
    }

    /// Installation copies only admitted owned row prefixes. Unavailable outcomes
    /// publish no rows, irrespective of any initialized/stale staging tail.
    pub(crate) fn decode_air_history_payload(
        &mut self,
        job: &AirHistoryJob,
        decode_ticks: &mut [u64; AIR_HISTORY_N],
    ) {
        self.active[3] = job.active;
        self.active[4] = job.active;
        if !job.active {
            self.fail_air_history_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        if !job.acquired
            || job.history_limit != self.history.rows.len()
            || job.damage_limit != self.damage.rows.len()
            || job.history_limit > job.history.len()
            || job.damage_limit > job.damage.len()
        {
            self.history.outcome = Outcome::Unavailable {
                reason: "air history owned payload capacity mismatch",
            };
            self.damage.outcome = self.history.outcome;
        } else {
            let history_len = job.history_result.len();
            let damage_len = job.damage_result.len();
            self.history.rows[..history_len].copy_from_slice(&job.history[..history_len]);
            self.damage.rows[..damage_len].copy_from_slice(&job.damage[..damage_len]);
            self.history.outcome = job.history_result;
            self.damage.outcome = job.damage_result;
        }
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

    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacity = Capacities {
            rows: [3; N],
            ace_names: 8,
            wings: 8192,
        };
        let captured = Captured::with_capacities(&capacity);
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
        captured.equipment.outcome = Outcome::Unavailable {
            reason: "equipment source fault",
        };
        captured.cumulative.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 0 });
    }

    #[test]
    fn growth_counts_native_entries_and_retains_primary_capacity() {
        let (mut capacity, mut captured) = small();
        captured.history.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 60,
        };
        captured.names.required = 100;
        captured.grow(&mut capacity);
        assert!(capacity.rows[3] >= 63);
        assert!(capacity.ace_names >= 100);
        assert_eq!(captured.history.rows.len(), capacity.rows[3]);
        let before = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacity);
        assert_eq!(captured.allocated_bytes(), before);
    }

    #[test]
    fn wing_daily_private_fields_and_capacities_do_not_gate_hourly_equipment() {
        use super::wing_test_kit as kit;
        struct Guard<'a> {
            inner: engine::memory::SliceMemory<'a>,
            cumulative: bool,
            theatres: bool,
        }
        impl Memory for Guard<'_> {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                for (field, active) in [
                    (cair_wing::AIR_ACCIDENTS, self.cumulative),
                    (cair_wing::EXPERIENCE, self.cumulative),
                    (cair_wing::GROUP_HANDLE, self.theatres),
                ] {
                    let target = kit::WING + field;
                    assert!(
                        active || !(address <= target && target < address + bytes.len()),
                        "inactive wing field {field:#x}"
                    );
                }
                self.inner.copy(address, bytes)
            }
        }
        let bytes = kit::fixture();
        let mut job = AirWingJob::new();
        for hour in 0..24 {
            let cumulative = hour == 16;
            let theatres = hour == 13;
            job.cumulative
                .resize(if cumulative { 2 } else { 0 }, Default::default());
            job.memberships
                .resize(if theatres { 2 } else { 0 }, Default::default());
            let memory = Guard {
                inner: kit::memory(&bytes),
                cumulative,
                theatres,
            };
            let report = copy_air_refresh(
                &memory,
                AirWingCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                    limits: [2, 3, if cumulative { 2 } else { usize::MAX }],
                    naval_limit: if theatres { 2 } else { usize::MAX },
                },
                &mut job,
            );
            assert!(report.raw_ok && !job.grow_for_next());
            assert_eq!(job.equipment_outcome, Outcome::Rows { len: 3 });
            assert_eq!(
                job.cumulative_outcome,
                if cumulative {
                    Outcome::Rows { len: 2 }
                } else {
                    SKIPPED
                }
            );
            if !cumulative {
                assert_eq!(report.raw_bytes[1], 0);
            }
            if !theatres {
                assert_eq!((job.required_memberships, job.membership_ticks), (0, 0));
            }
            let mut captured = Captured::with_capacities(&Capacities {
                rows: [3, 0, if cumulative { 2 } else { 0 }, 0, 0, 0, 0, 4096, 1024, 64],
                ace_names: 0,
                wings: 8192,
            });
            decode_air_payload(&mut captured, &job, &mut [0; WING_PACK_N]);
            assert_eq!(captured.equipment.outcome, job.equipment_outcome);
            assert_eq!(captured.cumulative.outcome, job.cumulative_outcome);
            captured.fail_air_wing_capture("worker failed");
            assert_eq!(
                captured.cumulative.outcome,
                failed(cumulative, "worker failed")
            );
        }
    }

    #[test]
    fn daily_statistics_and_history_skip_reads_and_capacity_preflight() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                panic!("inactive air read at {address:#x}");
            }
        }
        let mut stats = AirStatsJob::default();
        let mut history = AirHistoryJob::default();
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.prepare_hour(hour, false);
            if hour != 5 {
                let report = copy_air_stats_refresh(
                    &NoReads,
                    AirStatsCopyContext {
                        base: 0,
                        game_state: 1,
                        hour,
                        force_full: false,
                    },
                    &mut stats,
                );
                assert!(!report.raw_ok && !stats.grow_for_next());
                assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
                captured.fail_air_stats_capture("worker failed");
                let mut ticks = [0];
                decode_air_stats_payload(&mut captured, 0, &stats, &mut ticks);
                assert_eq!((captured.statistics.outcome, ticks), (SKIPPED, [0]));
            }
            if hour != 7 {
                let report = copy_air_history_refresh(
                    &NoReads,
                    AirHistoryCopyContext {
                        base: 0,
                        game_state: 1,
                        hour,
                        force_full: false,
                        history_limit: usize::MAX,
                        damage_limit: usize::MAX,
                    },
                    &mut history,
                );
                assert!(!report.raw_ok && !history.grow_for_next());
                assert_eq!((report.copy_ticks, report.raw_bytes), ([0; 2], [0; 2]));
                captured.fail_air_history_capture("worker failed");
                let mut ticks = [0; 2];
                captured.decode_air_history_payload(&history, &mut ticks);
                assert_eq!(
                    (captured.history.outcome, captured.damage.outcome, ticks),
                    (SKIPPED, SKIPPED, [0; 2])
                );
            }
        }
    }

    #[test]
    fn wing_equipment_is_hourly_and_cumulative_uses_slot_16() {
        use replay::rows::TableId;
        for table in [TableId::WingEquipment, TableId::WingCumulative] {
            assert_eq!(
                replay::cadence::is_staggered(table),
                table == TableId::WingCumulative
            );
            for hour in -24_i32..48 {
                assert_eq!(
                    replay::cadence::is_refresh(table, hour),
                    table == TableId::WingEquipment || hour.rem_euclid(24) == 16,
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
                rows: [equipment, 2048, cumulative, 8192, 16384, 1024, 2048, 4096, 1024, 64],
                ace_names: 65536,
                wings: 8192,
            };
            let direct = kit::direct_with(&memory, &capacity);
            let (split, report, _) = kit::split_with(&memory, &capacity);
            assert!(
                report.raw_ok,
                "equipment={equipment} cumulative={cumulative}"
            );
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows with all rows present.
        let capacity = Capacities {
            rows: [3, 2048, 2, 8192, 16384, 1024, 2048, 4096, 1024, 64],
            ace_names: 65536,
            wings: 8192,
        };
        let (split, _, _) = kit::split_with(&memory, &capacity);
        assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
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
        assert!(report.raw_ok);
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
        for fault in [
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::WING,
            kit::WING + engine::cair_wing::EXPERIENCE,
            kit::WING + engine::cair_wing::EQUIPMENT_POOL,
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
        }
    }

    #[test]
    fn shared_wing_map_caps_do_not_bound_extension_or_membership_censuses() {
        use super::wing_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for map_limit in [0, 1, 2] {
            let mut first = super::AirWingJob::new();
            let context = super::AirWingCopyContext {
                hour: 0,
                force_full: true,
                base: kit::BASE,
                game_state: kit::GAME_STATE,

                limits: [map_limit, 3, 2],
                naval_limit: 4096,
            };
            let report = super::copy_air_refresh(&memory, context, &mut first);
            assert!(report.raw_ok);
            assert_eq!(first.source_outcome, Outcome::Rows { len: 2 });
            assert_eq!(first.equipment_outcome, Outcome::Rows { len: 3 });
            assert_eq!(first.cumulative_outcome, Outcome::Rows { len: 2 });
            assert_eq!(first.memberships().1.len(), 2);
            assert_eq!(
                first.map_outcome,
                if map_limit == 2 {
                    Outcome::Rows { len: 2 }
                } else {
                    Outcome::Truncated {
                        len: map_limit,
                        unread_rows: (2 - map_limit) as u32,
                    }
                }
            );
        }
    }

    #[test]
    fn shared_wing_naval_cap_keeps_tail_leaf_failure_local() {
        use super::wing_test_kit as kit;
        let mut bytes = kit::fixture();
        // The later wing is beyond the naval prefix and remains inside the
        // independently bounded map and extension census.
        bytes[0x34000..0x34008].fill(0);
        let memory = kit::memory(&bytes);
        let mut first = super::AirWingJob::new();
        let context = super::AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            limits: [2, 3, 2],
            naval_limit: 1,
        };
        let report = super::copy_air_refresh(&memory, context, &mut first);
        assert!(report.raw_ok);
        assert!(matches!(first.source_outcome, Outcome::Unavailable { .. }));
        assert!(matches!(first.map_outcome, Outcome::Unavailable { .. }));
        assert_eq!(
            first.naval_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(first.memberships().1.len(), 1);
    }

    #[test]
    fn stats_split_copy_matches_direct_walk() {
        use super::stats_test_kit as kit;
        use engine::memory::{CountedMemory, ReadCounts};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = Capacities::default();
        let direct = kit::direct(&memory, &capacities);
        let memory = CountedMemory::new(memory);
        let capture = read_accounting::Capture::new();
        let (split, report, mut job) = kit::split(&memory, &capacities);
        let counts = capture.finish();
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
        assert_eq!((split.rows[0].country, split.rows[0].region), (2, 0));
        assert_eq!((split.rows[1].country, split.rows[1].region), (2, 1));
        assert_eq!(split.rows[0].channels[0], 1000);
        assert_eq!(split.rows[1].channels[0], 2000);
        let anchor = TableId::AirStatistics;
        let mut expected = read_accounting::ZERO;
        // 21 census scalars, 14 header scalars per history, two nine-span
        // channel gathers and one two-pointer record batch: 790 requested bytes.
        expected[anchor.index()] = ReadCounts {
            copy_calls: 49,
            scattered_calls: 2,
            record_calls: 1,
            plan_calls: 0,
            spans: 69,
            bytes: 790,
        };
        assert_eq!(counts, expected);
        let mut refreshes = 0;
        for hour in 0..24 {
            if replay::cadence::is_refresh(anchor, hour) {
                refreshes += 1;
                continue;
            }
            let capture = read_accounting::Capture::new();
            let skipped = copy_air_stats_refresh(
                &memory,
                AirStatsCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                },
                &mut job,
            );
            assert_eq!(capture.finish(), read_accounting::ZERO);
            assert!(!skipped.raw_ok && !job.grow_for_next());
            assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
        }
        assert_eq!(refreshes, 1);
        let measured = counts[anchor.index()];
        let per_refresh = [
            measured.copy_calls,
            measured.scattered_calls,
            measured.record_calls,
            measured.plan_calls,
            measured.spans,
            measured.bytes,
        ];
        println!(
            "daily_fixture cohort=stats_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
            per_refresh.map(|value| value * 24)
        );
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
                rows: [8192, 2048, 4096, 8192, 16384, 1024, rows, 4096, 1024, 64],
                ace_names: 65536,
                wings: 8192,
            };
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "rows={rows}");
            kit::assert_matches_direct(&split, &direct);
        }
        let capacities = Capacities {
            rows: [8192, 2048, 4096, 8192, 16384, 1024, 2, 4096, 1024, 64],
            ace_names: 65536,
            wings: 8192,
        };
        let (split, _, _) = kit::split(&memory, &capacities);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn stats_staging_overflow_falls_back_to_typed_walk() {
        use super::stats_test_kit as kit;
        use super::{AirStatsCopyContext, AirStatsJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirStatsCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = AirStatsJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            ..Default::default()
        };
        let report = super::copy_air_stats_refresh(&memory, ctx, &mut tiny);
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
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut job = super::AirStatsJob::new();
        let report = super::copy_air_stats_refresh(&memory, ctx, &mut job);
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
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            limits: [1024, 8192, 4096],
            naval_limit: 4096,
        };
        let mut tiny = AirWingJob::default();
        let report = super::copy_air_refresh(&memory, ctx, &mut tiny);
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
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.bases_outcome, Outcome::Rows { len: 4 });
        assert_eq!(split.regs.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.aces_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn bases_and_registrations_are_hourly_and_aces_use_slot_21() {
        use replay::rows::TableId;
        for table in [
            TableId::AirBases,
            TableId::AirBaseRegistrations,
            TableId::AirAces,
        ] {
            assert_eq!(
                replay::cadence::is_staggered(table),
                table == TableId::AirAces
            );
            for hour in -24_i32..48 {
                assert_eq!(
                    replay::cadence::is_refresh(table, hour),
                    table != TableId::AirAces || hour.rem_euclid(24) == 21,
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
    fn air_bases_truncation_matrix_preserves_other_tables_and_grows_registrations() {
        // Bases and aces retain their bounded typed result. Registrations can
        // fill complete current rows from the already-owned payload even when
        // their initial wing arena is too small.
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
            let mut direct = kit::direct_with(&memory, bases, wings, aces, names);
            direct.regs = crate::extended::fog::Captured::collect_registrations_direct(
                2,
                2,
                &memory,
                kit::BASE,
                kit::GAME_STATE,
            );
            let (split, report, _) = kit::split_with(&memory, bases, wings, aces, names);
            assert!(
                report.raw_ok,
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
        for fault in [
            // Shared country census: every slot blocks.
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::C0,
            kit::LTAB + 4,
            // Shared manager and vectors: bases and registrations block.
            kit::MANAGER,
            kit::SVEC,
            kit::B0,
            // Bases-private fields: only bases blocks.
            kit::B0 + engine::cair_base::HOST_REFERENCE,
            kit::S0,
            // Registrations-private spans: only registrations blocks.
            kit::CONT1,
            kit::CONT1 + engine::ccountry_air_container::COUNTRY,
            kit::POOL0,
            kit::W0,
            // Aces-private spans: only aces blocks.
            kit::C0 + engine::ccountry::ACES,
            kit::A0,
            kit::A0 + engine::cace::ALIVE,
            kit::NSTR,
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
            let (split, report, job) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            // The census keeps copying discoverable subtrees after a country
            // fault, so counts can be complete while the table is unavailable.
            // The worker's fault gate must still reject that storage hint.
            assert_eq!(
                job.registration_demand.filter(|_| !job.log.fault[1]),
                matches!(direct.regs.outcome, Outcome::Rows { .. }).then_some([2, 2]),
                "fault={fault:x}"
            );
            // Empty registration storage must preserve every unavailable
            // result, including census faults with complete subtree counts.
            // Keep unrelated bases/aces admitted: their zero-capacity walkers
            // skip validation reads and have a different acquisition contract.
            if matches!(direct.regs.outcome, Outcome::Unavailable { .. }) {
                let direct = kit::collect_typed(&memory, [8; N], 256, 0, 0);
                let (split, _, job) = kit::collect_split(&memory, [8; N], 256, 0, 0);
                kit::assert_matches_direct(&split, &direct);
                assert_eq!(job.registration_demand.filter(|_| !job.log.fault[1]), None);
            }
        }
    }

    #[test]
    fn air_bases_registration_demand_counts_duplicate_occurrences_and_empty_rows() {
        use super::air_bases_test_kit as kit;
        let mut bytes = kit::fixture();
        // Revisit the same bases in another vector, the same pool twice, and
        // the first wing twice per pool. Every occurrence remains a value.
        let vector = kit::MANAGER + cstrategic_air_manager::STATE_GUN_BASES;
        kit::put64(&mut bytes, vector, kit::SVEC as u64);
        kit::put32(&mut bytes, vector + 8, 3);
        kit::put32(&mut bytes, vector + 12, 3);
        for (header, pointer, count) in [
            (kit::CONT1 + ccountry_air_container::POOLS, kit::POOL0, 2),
            (kit::POOL0 + cair_wing_pool::WINGS, kit::W0, 3),
        ] {
            let array = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
            kit::put64(&mut bytes, array + (count - 1) * 8, pointer as u64);
            kit::put32(&mut bytes, header + 8, count as u32);
            kit::put32(&mut bytes, header + 12, count as u32);
        }
        let memory = kit::memory(&bytes);
        let expected = kit::collect_typed(&memory, [8; N], 256, 4, 12);
        for capacity in [0, 1] {
            let (split, report, mut job) =
                kit::collect_split(&memory, [8; N], 256, capacity, capacity);
            assert!(report.raw_ok);
            assert_eq!(job.registration_demand, Some([4, 12]));
            kit::assert_matches_direct(&split, &expected);
            assert_eq!(split.regs.outcome, Outcome::Rows { len: 4 });
            assert_eq!(
                split
                    .regs
                    .rows
                    .iter()
                    .filter(|row| row.wings_len == 0)
                    .count(),
                2
            );
            assert_eq!(split.regs.wings.len(), 12);
            job.clear();
            assert_eq!(job.registration_demand, None);
        }
        // An invalid repeated wing still refuses the table even though the
        // original acquisition already saw the container and pool counts.
        kit::put64(&mut bytes, kit::W0 + cair_wing::REFERENCE_TYPE, 0);
        let memory = kit::memory(&bytes);
        let direct = kit::collect_typed(&memory, [8; N], 256, 0, 0);
        let (split, _, job) = kit::collect_split(&memory, [8; N], 256, 0, 0);
        assert!(matches!(split.regs.outcome, Outcome::Unavailable { .. }));
        assert_eq!(job.registration_demand, None);
        kit::assert_matches_direct(&split, &direct);
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
            if bad_slot.is_some_and(|slot| slot > 1) {
                let mut registrations = [engine::cair_base::RegistrationRow::default(); 4];
                let mut wings = [0; 8];
                let prefix = engine::cair_base::walk_registrations(
                    &memory,
                    kit::BASE,
                    kit::GAME_STATE,
                    engine::cair_base::RegistrationBuffers {
                        registrations: &mut registrations,
                        wings: &mut wings,
                        resolved: &mut [],
                    },
                );
                assert_eq!((prefix.wings_len, prefix.required_wings), (2, 2));
                assert_eq!(&wings[..2], &[kit::WING_HANDLE_0, kit::WING_HANDLE_1]);
            }
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
        // A later pointer fault must not replace the first container's own
        // independent failure, in either the native reader or owned replay.
        kit::put64(&mut bytes, kit::CONT1, 0);
        let memory = Fault {
            source: kit::memory(&bytes),
            address: Some(array + 63 * 8),
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(
            direct.regs.outcome,
            Outcome::Unavailable {
                reason: "air base or linked object vtable mismatch",
            }
        );
        kit::assert_matches_direct(&split, &direct);
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
        // The other bucket is cold despite the populated seen list. Faults
        // and invalid slots leave it empty; a successful observation fills it.
        assert_eq!(job.seen_link_index[0], usize::MAX);
        for (value, expected, calls) in [(None, None, 3), (Some(8), None, 4), (Some(3), Some(3), 5)]
        {
            memory.value.set(value);
            assert_eq!(
                resolve_link(
                    &mut job.log,
                    &memory,
                    &links,
                    &mut job.seen_links,
                    &mut job.seen_link_index,
                    4,
                    2,
                ),
                expected
            );
            assert_eq!(job.seen_link_index[0] == usize::MAX, expected.is_none());
            assert_eq!(memory.calls.borrow().len(), calls);
        }
        assert!(job.seen_link_index.iter().all(|&cell| cell != usize::MAX));
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
        assert_eq!(memory.calls.borrow().len(), 6);
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
                true,
                &mut [0; 3],
            );
            copy_air_bases_pass(
                &mut linear,
                &reference,
                kit::BASE,
                kit::GAME_STATE,
                true,
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
        let mut fog = crate::extended::fog::Captured::with_capacities(&fog_cap);
        fog.registrations_raw_mode = true;
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
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirBasesCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = AirBasesJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            registration_demand: None,
            aces_active: false,
            census: Vec::new(),
            seen_links: Vec::new(),
            seen_link_index: Vec::new(),
            required_census: 0,
            required_links: 0,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        let report = super::copy_air_bases_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert_eq!(tiny.registration_demand, None);
    }

    /// 5,000 countries, far above the initial 1,024-entry census and link
    /// stores. One host base files a container at every country slot except
    /// slot five, whose country object is unreadable: that census fault must
    /// not stop growth, and it touches no published row. After one off-thread
    /// growth the owned payload decodes exactly like the typed walks.
    #[test]
    fn air_bases_census_grows_once_to_five_thousand_countries() {
        use crate::extended::fog;
        use engine::{
            cair_base, ccountry, ccountry_air_container, cgame_state, cstrategic_air_manager,
        };
        const COUNTRIES: u32 = 5_001;
        const GAME: usize = 0x10000;
        const MANAGER: usize = 0x20000;
        const HOSTS: usize = 0x21000;
        const HOST: usize = 0x22000;
        const TABLE: usize = 0x30000;
        const LINKS: usize = 0x40000;
        const OBJECTS: usize = 0x50000;
        const SLOTS: usize = 0x70000;
        const CONTAINERS: usize = 0x100000;
        const UNREADABLE: u32 = 5;
        let put32 = |bytes: &mut Vec<u8>, at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        let put64 = |bytes: &mut Vec<u8>, at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let header = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
            put64(bytes, at, data as u64);
            put32(bytes, at + 8, count);
            put32(bytes, at + 12, count);
        };
        let mut bytes = vec![0; CONTAINERS + COUNTRIES as usize * 0x100];
        header(&mut bytes, GAME + cgame_state::COUNTRIES, TABLE, COUNTRIES);
        header(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS, COUNTRIES);
        put64(
            &mut bytes,
            GAME + cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        put64(
            &mut bytes,
            MANAGER,
            cstrategic_air_manager::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
            HOSTS,
            1,
        );
        put64(&mut bytes, HOSTS, HOST as u64);
        put64(&mut bytes, HOST, cair_base::VTABLE_RVA as u64);
        put64(
            &mut bytes,
            HOST + cair_base::HANDLE,
            (0x43_u64 << 32) | u64::from(cair_base::TYPE),
        );
        put64(&mut bytes, HOST + cair_base::HOST_REFERENCE, (3 << 32) | 51);
        header(
            &mut bytes,
            HOST + cair_base::COUNTRY_CONTAINERS,
            SLOTS,
            COUNTRIES,
        );
        for slot in 1..COUNTRIES {
            let object = if slot == UNREADABLE {
                0x7fff_0000_0000
            } else {
                OBJECTS + slot as usize * 16
            };
            put64(&mut bytes, TABLE + slot as usize * 8, object as u64);
            put32(&mut bytes, LINKS + slot as usize * 4, slot);
            if slot == UNREADABLE {
                continue;
            }
            put64(&mut bytes, object, ccountry::VTABLE_RVA as u64);
            put32(&mut bytes, object + ccountry::ID, slot);
            let container = CONTAINERS + slot as usize * 0x100;
            put64(
                &mut bytes,
                container,
                ccountry_air_container::VTABLE_RVA as u64,
            );
            put32(
                &mut bytes,
                container + ccountry_air_container::COUNTRY,
                slot,
            );
            put64(&mut bytes, SLOTS + slot as usize * 8, container as u64);
        }
        let memory = kit_memory(&bytes);
        let ctx = AirBasesCopyContext {
            hour: 0,
            force_full: false,
            base: 0,
            game_state: GAME,
        };
        let mut job = AirBasesJob::new();
        assert!(job.census.capacity() < COUNTRIES as usize - 1);
        let report = copy_air_bases_refresh(&memory, ctx, &mut job);
        assert!(!report.raw_ok, "the initial census store overflows");
        assert!(job.log.fault.iter().any(|fault| *fault));
        assert!(!job.log.grow_for_next(), "staging alone would never grow");
        assert!(job.grow_for_next());
        job.grow();
        job.clear();
        assert!(job.census.capacity() >= COUNTRIES as usize - 1);
        assert!(job.seen_links.capacity() >= COUNTRIES as usize);
        assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
        let report = copy_air_bases_refresh(&memory, ctx, &mut job);
        assert!(report.raw_ok);
        assert!(!job.grow_for_next());

        let rows = COUNTRIES as usize;
        let mut air = Captured::with_capacities(&Capacities::default());
        let mut ticks = [0; AIR_BASES_N];
        air.decode_air_bases_payload(0, &job, &mut ticks);
        let mut bases = vec![cair_base::Row::default(); 4];
        let direct_bases = cair_base::walk(&memory, 0, GAME, &mut bases);
        assert_eq!(direct_bases, Outcome::Rows { len: 1 });
        assert_eq!(air.bases.outcome, direct_bases);
        assert_eq!(air.bases.rows[0], bases[0]);
        let direct = fog::Captured::collect_registrations_direct(rows, 1, &memory, 0, GAME);
        assert_eq!(direct.outcome, Outcome::Rows { len: rows - 2 });
        assert!(
            direct
                .rows
                .iter()
                .zip((1..COUNTRIES).filter(|slot| *slot != UNREADABLE))
                .all(|(row, slot)| (row.slot, row.country) == (slot, slot))
        );
        let fog_capacities = fog::Capacities {
            rows: [0, 0, 0, 0, 0, rows, 0],
            radar_provinces: 0,
            area_provinces: 0,
            wings: 1,
        };
        let mut owned = fog::Captured::with_capacities(&fog_capacities);
        // The first decode has no resolver yet; it records the live demand,
        // and growth after it serves every later decode from the dense cache.
        for expected_len in [0, rows] {
            owned.registrations_raw_mode = true;
            owned.decode_registrations_payload(0, &job, &mut ticks);
            assert_eq!(owned.resolver_sizes(), (expected_len, rows));
            let decoded = owned.registrations_snapshot();
            assert_eq!(decoded.outcome, direct.outcome);
            assert_eq!(decoded.rows, direct.rows);
            assert_eq!(decoded.wings, direct.wings);
            owned.grow(&mut fog::Capacities {
                rows: [0, 0, 0, 0, 0, rows, 0],
                radar_provinces: 0,
                area_provinces: 0,
                wings: 1,
            });
        }
    }

    fn kit_memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
        engine::memory::SliceMemory { base: 0, bytes }
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
    fn air_history_and_damage_share_slot_7() {
        use replay::rows::TableId;
        for table in [TableId::AirRegionHistory, TableId::AirRegionDamage] {
            assert!(replay::cadence::is_staggered(table), "{table:?}");
            for hour in -24_i32..48 {
                assert_eq!(
                    replay::cadence::is_refresh(table, hour),
                    hour.rem_euclid(24) == 7,
                    "{table:?} at {hour}"
                );
            }
        }
    }

    #[test]
    fn air_history_split_copy_matches_direct_walk() {
        use super::air_history_test_kit as kit;
        use engine::memory::{CountedMemory, ReadCounts};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let memory = CountedMemory::new(memory);
        let capture = read_accounting::Capture::new();
        let (split, report, mut job) = kit::split(&memory);
        let counts = capture.finish();
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        // Three records across two sides plus three damage entries.
        assert_eq!(split.history_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.damage_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.history[0].key.wing_id, 0x1111);
        assert_eq!(split.history[0].region_owner, 7);
        assert_eq!(split.damage[0].cause, 5);
        assert_eq!(split.damage[0].amount, 1000);
        assert_eq!(split.damage[2].channel, 1);
        let anchor = TableId::AirRegionHistory;
        let mut expected = read_accounting::ZERO;
        // 220 scalar bytes, 153 bytes in six leaf gathers and 88 bytes in
        // four root batches. AirRegionDamage shares this actual native anchor.
        expected[anchor.index()] = ReadCounts {
            copy_calls: 20,
            scattered_calls: 6,
            record_calls: 4,
            plan_calls: 0,
            spans: 45,
            bytes: 461,
        };
        assert_eq!(counts, expected);
        let mut refreshes = 0;
        for hour in 0..24 {
            if replay::cadence::is_refresh(anchor, hour) {
                refreshes += 1;
                continue;
            }
            let capture = read_accounting::Capture::new();
            let skipped = copy_air_history_refresh(
                &memory,
                AirHistoryCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                    history_limit: usize::MAX,
                    damage_limit: usize::MAX,
                },
                &mut job,
            );
            assert_eq!(capture.finish(), read_accounting::ZERO);
            assert!(!skipped.raw_ok && !job.grow_for_next());
            assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0; 2], [0; 2]));
        }
        assert_eq!(refreshes, 1);
        let measured = counts[anchor.index()];
        let per_refresh = [
            measured.copy_calls,
            measured.scattered_calls,
            measured.record_calls,
            measured.plan_calls,
            measured.spans,
            measured.bytes,
        ];
        println!(
            "daily_fixture cohort=air_history_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
            per_refresh.map(|value| value * 24)
        );
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
            kit::assert_matches_direct(&split, &direct);
        }
        let direct = kit::direct_with(&memory, 3, 3);
        assert_eq!(direct.history_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.damage_outcome, Outcome::Rows { len: 3 });
    }

    #[test]
    fn air_history_split_copy_matches_direct_walk_on_fault_matrix() {
        // Every fault arm asserts identical admitted rows/outcomes plus per-slot
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
        for fault in [
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::MANAGER,
            kit::R0,
            kit::R0 + engine::cair_region_combat_data::OWNER,
            kit::REC0,
            // Damage-scoped faults: history still decodes identical Rows.
            kit::REC0 + engine::sair_wing_combat_data::FIRST_DAMAGE,
            kit::E0,
            kit::E0 + engine::sdamage_entry::AMOUNT,
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
        }
    }

    #[test]
    fn air_history_staging_overflow_falls_back_to_typed_walk() {
        use super::air_history_test_kit as kit;
        use super::{AirHistoryCopyContext, AirHistoryJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = AirHistoryCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            history_limit: 8192,
            damage_limit: 16_384,
        };
        let mut tiny = AirHistoryJob::default();
        let report = super::copy_air_history_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
    }

    #[test]
    fn air_history_pools_cover_appendix_worst_case() {
        let job = AirHistoryJob::new();
        assert!(job.history.len() >= 1_341);
        assert!(job.damage.len() >= 500);
    }

    #[test]
    fn air_history_seed_covers_full_country_by_region_null_pointer_census() {
        use super::air_history_test_kit as kit;
        use engine::cstrategic_air;
        fn put32(bytes: &mut [u8], at: usize, value: u32) {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn put64(bytes: &mut [u8], at: usize, value: usize) {
            bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
            put64(bytes, at, data);
            put32(bytes, at + 8, count);
            put32(bytes, at + 12, count);
        }
        let mut bytes = kit::fixture();
        bytes.resize(0xa0000, 0);
        header(
            &mut bytes,
            kit::GAME_STATE + cgame_state::COUNTRIES,
            0x40000,
            440,
        );
        header(
            &mut bytes,
            kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
            0x41000,
            440,
        );
        header(
            &mut bytes,
            kit::MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            0x42000,
            440,
        );
        for slot in 1..440_usize {
            let country = 0x50000 + slot * 16;
            let air = 0x60000 + slot * 512;
            put64(&mut bytes, 0x40000 + slot * 8, country);
            put32(&mut bytes, 0x41000 + slot * 4, slot as u32);
            put64(&mut bytes, country, ccountry::VTABLE_RVA);
            put32(&mut bytes, country + ccountry::ID, slot as u32);
            put64(&mut bytes, 0x42000 + slot * 8, air);
            put64(&mut bytes, air, cstrategic_air::VTABLE_RVA);
            put32(&mut bytes, air + cstrategic_air::COUNTRY, slot as u32);
            // Shared zero bytes deliberately still require every observation.
            header(
                &mut bytes,
                air + cstrategic_air::REGION_COMBAT,
                0x43000,
                512,
            );
        }
        let mut job = AirHistoryJob::new();
        acquire_air_history_pass(
            &mut job,
            &kit::memory(&bytes),
            kit::BASE,
            kit::GAME_STATE,
            0,
            0,
        );
        assert_eq!(job.history_result, Outcome::Rows { len: 0 });
        assert_eq!(job.damage_result, Outcome::Rows { len: 0 });
        assert!(job.raw_bytes[0] >= (439 * 512 * 8) as u64);
        assert!(job.acquired);
        assert!(!job.grow_for_next());
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

    #[test]
    fn air_history_preflights_both_buffers_before_source_reads() {
        struct Unread;
        impl Memory for Unread {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("preflight must not read");
            }
        }
        for short_history in [false, true] {
            let mut job = AirHistoryJob::new();
            if short_history {
                job.history.clear();
            } else {
                job.damage.clear();
            }
            let report = copy_air_history_refresh(
                &Unread,
                AirHistoryCopyContext {
                    hour: 0,
                    force_full: true,
                    base: 0,
                    game_state: 0x1000,
                    history_limit: 8192,
                    damage_limit: 16_384,
                },
                &mut job,
            );
            assert!(!report.raw_ok && !job.acquired);
            assert!(job.grow_for_next());
            job.grow();
            assert!(job.has_storage());
            assert!(!job.grow_for_next());
        }
    }

    #[test]
    fn air_history_owned_walk_preserves_exact_read_order_and_fault_prefix_without_retry() {
        use super::air_history_test_kit as kit;
        struct Observed<'a> {
            inner: engine::memory::SliceMemory<'a>,
            fault: usize,
            calls: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Observed<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                if address <= self.fault && self.fault < address.saturating_add(output.len()) {
                    return false;
                }
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        for fault in [
            usize::MAX,
            kit::R0 + cair_region_combat_data::OWNER,
            kit::REC0 + sair_wing_combat_data::ATTRIBUTED_COUNTRY,
            kit::E0 + sdamage_entry::AMOUNT,
        ] {
            let memory = Observed {
                inner: kit::memory(&bytes),
                fault,
                calls: Default::default(),
            };
            let expected = kit::direct(&memory);
            let calls = std::mem::take(&mut *memory.calls.borrow_mut());
            let mut job = AirHistoryJob::new();
            acquire_air_history_pass(&mut job, &memory, kit::BASE, kit::GAME_STATE, 8192, 16_384);
            assert!(job.acquired);
            assert_eq!(job.history_result, expected.history_outcome);
            assert_eq!(job.damage_result, expected.damage_outcome);
            assert_eq!(*memory.calls.borrow(), calls, "fault {fault:x}");
        }
    }

    #[test]
    fn air_history_worker_installs_only_admitted_owned_prefix_before_recycle() {
        use super::air_history_test_kit as kit;
        let bytes = kit::fixture();
        let (_, _, mut job) = kit::split(&kit::memory(&bytes));
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.history.rows[4].planes = 123;
        job.history[4].planes = 456;
        captured.decode_air_history_payload(&job, &mut [0; 2]);
        assert_eq!(
            captured.history.rows[4].planes, 123,
            "unadmitted tail is not copied"
        );
        let first = captured.history.rows[0];
        job.history[0].planes ^= 7;
        job.clear();
        assert_eq!(
            captured.history.rows[0], first,
            "recycled storage does not alias frame"
        );
    }

    #[test]
    fn air_history_capacity_charge_follows_owned_payload_growth() {
        use crate::raw_memory::{Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = AirHistoryJob {
            capacity_charge: Charge::empty_on(&LOCAL),
            required_history: 3,
            required_damage: 5,
            ..Default::default()
        };
        job.grow();
        let capacity = LOCAL.snapshot().current;
        assert_eq!(
            capacity.payload_vectors,
            vector_bytes(&job.history) + vector_bytes(&job.damage)
        );
        assert_eq!(capacity.auxiliary_vectors, 0);
        assert_eq!(capacity.index_vectors, 0);
        job.clear();
        assert_eq!(LOCAL.snapshot().current, capacity);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Default::default());
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
    fn history_owned_pass_preserves_every_record_fault_and_damage_continuation() {
        use super::air_history_test_kit as kit;
        let record_fields = [
            (kit::REC0, 26),
            (kit::REC0 + sair_wing_combat_data::ATTRIBUTED_COUNTRY, 4),
            (kit::REC0 + sair_wing_combat_data::COPIED_FLAG, 1),
        ];
        let faults: Vec<_> = std::iter::once(0..0)
            .chain(record_fields.into_iter().flat_map(|(address, len)| {
                (0..len).map(move |byte| address + byte..address + byte + 1)
            }))
            .chain([
                kit::REC0 + 0x20..kit::REC0 + 0x21,
                kit::REC0 + 0x38..kit::REC0 + 0x39,
                kit::E0..kit::E0 + 1,
                kit::E0 + 0x10..kit::E0 + 0x11,
            ])
            .collect();
        for fault in faults {
            for invalid_damage_header in [false, true] {
                let mut bytes = kit::fixture();
                if invalid_damage_header {
                    let at = kit::REC0 + sair_wing_combat_data::FIRST_DAMAGE_CAPACITY;
                    bytes[at..at + 4].fill(0);
                }
                let mut actual_memory = TraceMemory::new(&bytes, fault.clone());
                let mut direct_memory = TraceMemory::new(&bytes, fault.clone());
                actual_memory.changing_address = Some(kit::REC0 + 0x50);
                direct_memory.changing_address = Some(kit::REC0 + 0x50);
                let mut actual = AirHistoryJob::new();
                let mut previous_attribution = None;
                for _ in 0..2 {
                    let direct = kit::direct(&direct_memory);
                    acquire_air_history_pass(
                        &mut actual,
                        &actual_memory,
                        kit::BASE,
                        kit::GAME_STATE,
                        8192,
                        16_384,
                    );
                    assert!(actual.acquired);
                    assert_eq!(actual.history_result, direct.history_outcome);
                    assert_eq!(actual.damage_result, direct.damage_outcome);
                    assert_eq!(
                        actual.history[..actual.history_result.len()],
                        direct.history[..direct.history_outcome.len()]
                    );
                    assert_eq!(
                        actual.damage[..actual.damage_result.len()],
                        direct.damage[..direct.damage_outcome.len()]
                    );
                    assert_eq!(*actual_memory.reads.borrow(), *direct_memory.reads.borrow());
                    assert_eq!(actual_memory.gathers.get(), direct_memory.gathers.get());
                    if fault.is_empty() {
                        let attribution = actual.history[0].attributed_country;
                        if let Some(previous) = previous_attribution {
                            assert_ne!(
                                attribution, previous,
                                "each pass must acquire fresh leaf values"
                            );
                        }
                        previous_attribution = Some(attribution);
                    }
                }
                if fault.start == kit::E0 && !invalid_damage_header {
                    assert!(
                        !actual_memory
                            .reads
                            .borrow()
                            .iter()
                            .any(|&(address, _)| address == kit::E0 + sdamage_entry::AMOUNT),
                        "failed damage head must not retry or acquire its uncompleted amount"
                    );
                }
            }
        }
    }

    #[test]
    fn history_owned_pass_preserves_native_capacity_prefixes() {
        use super::air_history_test_kit as kit;
        let bytes = kit::fixture();
        for history_cap in 0..=4 {
            for damage_cap in 0..=4 {
                let actual_memory = TraceMemory::new(&bytes, 0..0);
                let direct_memory = TraceMemory::new(&bytes, 0..0);
                let direct = kit::direct_with(&direct_memory, history_cap, damage_cap);
                let mut actual = AirHistoryJob::new();
                let capacity = (actual.history.capacity(), actual.damage.capacity());
                acquire_air_history_pass(
                    &mut actual,
                    &actual_memory,
                    kit::BASE,
                    kit::GAME_STATE,
                    history_cap,
                    damage_cap,
                );
                assert!(actual.acquired);
                assert_eq!(actual.history_result, direct.history_outcome);
                assert_eq!(actual.damage_result, direct.damage_outcome);
                assert_eq!(
                    actual.history[..actual.history_result.len()],
                    direct.history[..direct.history_outcome.len()]
                );
                assert_eq!(
                    actual.damage[..actual.damage_result.len()],
                    direct.damage[..direct.damage_outcome.len()]
                );
                assert_eq!(*actual_memory.reads.borrow(), *direct_memory.reads.borrow());
                assert_eq!(
                    (actual.history.capacity(), actual.damage.capacity()),
                    capacity,
                    "no callback growth"
                );
            }
        }
    }

    #[test]
    fn stats_mirror_gathers_preserve_invalid_addresses() {
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
    }

    #[test]
    fn history_owned_pass_preserves_invalid_record_vector_addresses() {
        use super::air_history_test_kit as kit;
        for address in [0_u64, u64::MAX - 32, u64::MAX, 0x0000_8000_0000_0000] {
            let mut bytes = kit::fixture();
            let at = kit::R0 + cair_region_combat_data::FIRST_RECORDS;
            bytes[at..at + 8].copy_from_slice(&address.to_le_bytes());
            let actual_memory = TraceMemory::new(&bytes, 0..0);
            let direct_memory = TraceMemory::new(&bytes, 0..0);
            let direct = kit::direct(&direct_memory);
            let mut actual = AirHistoryJob::new();
            acquire_air_history_pass(
                &mut actual,
                &actual_memory,
                kit::BASE,
                kit::GAME_STATE,
                8192,
                16_384,
            );
            assert!(actual.acquired);
            assert_eq!(actual.history_result, direct.history_outcome);
            assert_eq!(actual.damage_result, direct.damage_outcome);
            assert!(matches!(actual.history_result, Outcome::Unavailable { .. }));
            assert_eq!(*actual_memory.reads.borrow(), *direct_memory.reads.borrow());
        }
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
        let mut captured = Captured::with_capacities(capacity);
        // Typed reference: wing walk with visitor into pooled buffers,
        // exactly like the game thread's Sources::air sequence.
        captured.begin_wings();
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
            };
            captured.visit_wing(memory, BASE, source);
        }
        captured.finish_wings(outcome);
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
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,

            limits: [1024, 8192, 4096],
            naval_limit: 4096,
        };
        let mut first = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut ticks = [0u64; 2];
        super::decode_air_payload(&mut captured, &first, &mut ticks);
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
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,

            limits: [1024, capacity.rows[0], capacity.rows[2]],
            naval_limit: 4096,
        };
        let mut first = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity);
        let mut ticks = [0u64; 2];
        super::decode_air_payload(&mut captured, &first, &mut ticks);
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
            rows: [1, 2048, 1, 8192, 16384, 1024, 2048, 4096, 1024, 64],
            ace_names: 65536,
            wings: 8192,
        };
        collect_typed(memory, &capacity)
    }

    pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, AirWingCopyReport, AirWingJob) {
        let ctx = AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,

            limits: [1024, 1, 1],
            naval_limit: 4096,
        };
        let mut first = AirWingJob::new();
        let report = super::copy_air_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let capacity = Capacities {
            rows: [1, 2048, 1, 8192, 16384, 1024, 2048, 4096, 1024, 64],
            ace_names: 65536,
            wings: 8192,
        };
        let mut captured = Captured::with_capacities(&capacity);
        let mut ticks = [0u64; 2];
        super::decode_air_payload(&mut captured, &first, &mut ticks);
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
        assert_eq!(
            split.equipment[..split.equipment_outcome.len()],
            direct.equipment[..direct.equipment_outcome.len()],
            "equipment rows"
        );
        assert_eq!(
            split.cumulative_outcome, direct.cumulative_outcome,
            "cumulative outcome"
        );
        assert_eq!(
            split.cumulative[..split.cumulative_outcome.len()],
            direct.cumulative[..direct.cumulative_outcome.len()],
            "cumulative rows"
        );
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
            &captured.equipment.rows[..captured.equipment.outcome.len()],
            &direct.equipment[..direct.equipment_outcome.len()],
            "equipment rows"
        );
        assert_eq!(
            captured.cumulative.outcome, direct.cumulative_outcome,
            "cumulative outcome"
        );
        assert_eq!(
            &captured.cumulative.rows[..captured.cumulative.outcome.len()],
            &direct.cumulative[..direct.cumulative_outcome.len()],
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

    pub(crate) fn set_first_value(bytes: &mut [u8], value: i64) {
        p64(bytes, VALUES1, value as u64);
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
        let mut captured = Captured::with_capacities(capacity);
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
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = AirStatsJob::new();
        let report = super::copy_air_stats_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity);
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
mod air_bases_memo_tests {
    use super::air_bases_test_kit as kit;
    use super::*;

    const CAPS: [usize; 5] = [8, 8, 256, 16, 64];

    #[test]
    fn daily_aces_are_excluded_from_hourly_memo_inputs_and_outputs() {
        struct Guard<'a> {
            inner: engine::memory::SliceMemory<'a>,
            aces_active: bool,
            census_reads: std::cell::Cell<usize>,
        }
        impl Memory for Guard<'_> {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                if address == kit::GAME_STATE + cgame_state::COUNTRIES {
                    self.census_reads.set(self.census_reads.get() + 1);
                }
                assert!(self.aces_active || address != kit::C0 + ccountry::ACES);
                self.inner.copy(address, bytes)
            }
        }
        let bytes = kit::fixture();
        let mut memo = AirBasesMemo::default();
        let mut job = AirBasesJob::new();
        for (index, hour) in (20..24).enumerate() {
            let active = hour == 21;
            let memory = Guard {
                inner: kit::memory(&bytes),
                aces_active: active,
                census_reads: std::cell::Cell::new(0),
            };
            let copy = copy_air_bases_refresh(
                &memory,
                AirBasesCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                },
                &mut job,
            );
            assert!(copy.raw_ok);
            assert_eq!(memory.census_reads.get(), 1);
            if hour == 23 {
                // An inactive slot cannot make the active memo ineligible.
                job.log.fault[2] = true;
            }
            let private = if active {
                [8, 256]
            } else if hour == 23 {
                [64, 1024]
            } else {
                [0, 0]
            };
            let (mut air, mut fog) = parts([8, private[0], private[1], 16, 64]);
            air.prepare_hour(hour, false);
            let mut ticks = [0; AIR_BASES_N];
            let report = memo.decode(
                &mut air,
                &mut fog,
                &job,
                AirBasesMemoContext {
                    hour,
                    ..context(index as u64 + 1)
                },
                &mut ticks,
            );
            assert!(report.eligible);
            assert_eq!(report.hit, hour == 23);
            assert!(matches!(air.bases.outcome, Outcome::Rows { .. }));
            assert!(fog.registrations_memo_complete());
            assert_eq!(
                air.aces.outcome,
                if active {
                    Outcome::Rows { len: 2 }
                } else {
                    SKIPPED
                }
            );
            if !active {
                assert_eq!((copy.copy_ticks[2], copy.raw_bytes[2], ticks[2]), (0, 0, 0));
                assert_eq!(job.log.fault[2], hour == 23);
                assert_eq!((air.names.len, air.names.required), (0, 0));
                air.fail_air_bases_capture("worker failed");
                assert_eq!(air.aces.outcome, SKIPPED);
            }
        }
    }

    fn context(seq: u64) -> AirBasesMemoContext {
        AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq,
            hour: 100 + seq as i32,
            force_full: false,
        }
    }

    fn parts(cap: [usize; 5]) -> (Captured, super::super::fog::Captured) {
        let mut air = Captured::with_capacities(&Capacities {
            rows: [0, cap[0], 0, 0, 0, cap[1], 0, 4096, 1024, 64],
            ace_names: cap[2],
            wings: 8192,
        });
        let mut fog =
            super::super::fog::Captured::with_capacities(&super::super::fog::Capacities {
                rows: [0, 0, 0, 0, 0, cap[3], 0],
                radar_provinces: 0,
                area_provinces: 0,
                wings: cap[4],
            });
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
            true,
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
        let report = memo.decode(&mut air, &mut fog, &job, context(2), &mut [0; 3]);
        assert!(report.hit && report.compared && !report.installed);
        assert_rows(&air, &fog, &expected);
        assert!(!air.bases_raw_mode && !fog.registrations_raw_mode);
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
    fn memo_declines_current_fault_and_admission_states() {
        let bytes = kit::fixture();
        let mut job = AirBasesJob::new();
        for case in [0, 1, 2, 9] {
            stage(&mut job, &bytes);
            let mut memo = AirBasesMemo::default();
            assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
            let (mut air, mut fog) = parts(CAPS);
            match case {
                0 => job.log.overflowed = true,
                1 => job.log.fault[0] = true,
                2 => job.log.spans[0].ok = false,
                9 => air.bases_raw_mode = false,
                _ => unreachable!(),
            }
            let report = memo.decode(&mut air, &mut fog, &job, context(2), &mut [0; 3]);
            assert!(
                !report.hit && !report.installed && !report.eligible,
                "case {case}"
            );
            assert!(memo.key.is_none() && memo.previous.is_none());
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
        // Only bases/aces/names retain fixed typed capacities. Complete raw
        // registrations supply their own storage demand before decode.
        for slot in 0..3 {
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
    fn memo_grows_registrations_before_keying_and_restoring_another_small_frame() {
        let bytes = kit::fixture();
        let expected = kit::direct(&kit::memory(&bytes));
        let mut job = AirBasesJob::new();
        stage(&mut job, &bytes);
        for initial in [0, 1] {
            let mut cap = CAPS;
            cap[3] = initial;
            cap[4] = initial;
            let mut memo = AirBasesMemo::default();
            let (air, fog, report) = run(&mut memo, &job, context(1), cap);
            assert!(report.installed && !report.hit);
            assert_eq!(memo.key.unwrap().capacities[3..], [2, 2]);
            assert_rows(&air, &fog, &expected);
            stage(&mut job, &bytes);
            let (air, fog, report) = run(&mut memo, &job, context(2), cap);
            assert!(report.hit && report.compared && !report.installed);
            assert_rows(&air, &fog, &expected);
            assert!(!air.bases_raw_mode && !fog.registrations_raw_mode);
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

    pub(super) fn collect_typed(
        memory: &impl Memory,
        air_rows: [usize; super::N],
        ace_names: usize,
        fog_rows: usize,
        fog_wings: usize,
    ) -> Split {
        use engine::{cace, cair_base};
        let capacities = Capacities {
            rows: air_rows,
            ace_names,
            wings: 8192,
        };
        let mut captured = Captured::with_capacities(&capacities);
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

    pub(super) fn collect_split(
        memory: &impl Memory,
        air_rows: [usize; super::N],
        ace_names: usize,
        fog_rows: usize,
        fog_wings: usize,
    ) -> (Split, AirBasesCopyReport, AirBasesJob) {
        let ctx = AirBasesCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let capacities = Capacities {
            rows: air_rows,
            ace_names,
            wings: 8192,
        };
        let mut air = Captured::with_capacities(&capacities);
        let mut fog =
            crate::extended::fog::Captured::with_capacities(&crate::extended::fog::Capacities {
                rows: [4, 4, 4, 4, 4, fog_rows, 4],
                radar_provinces: 8,
                area_provinces: 8,
                wings: fog_wings,
            });
        let mut first = AirBasesJob::new();
        let report = super::copy_air_bases_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        fog.registrations_raw_mode = true;
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
            [8192, bases, 4096, 8192, 16384, aces, 2048, 4096, 1024, 64],
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
            [8192, bases, 4096, 8192, 16384, aces, 2048, 4096, 1024, 64],
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
            rows: [8192, 2048, 4096, history_cap, damage_cap, 1024, 2048, 4096, 1024, 64],
            ace_names: 65_536,
            wings: 8192,
        };
        let mut captured = Captured::with_capacities(&capacities);
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
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,

            history_limit: history_cap,
            damage_limit: damage_cap,
        };
        let capacities = Capacities {
            rows: [8192, 2048, 4096, history_cap, damage_cap, 1024, 2048, 4096, 1024, 64],
            ace_names: 65_536,
            wings: 8192,
        };
        let mut captured = Captured::with_capacities(&capacities);
        let mut first = AirHistoryJob::new();
        let report = super::copy_air_history_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 2];
        captured.decode_air_history_payload(&first, &mut ticks);
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
        assert_eq!(
            split.history[..split.history_outcome.len()],
            direct.history[..direct.history_outcome.len()],
            "history rows"
        );
        assert_eq!(
            split.damage_outcome, direct.damage_outcome,
            "damage outcome"
        );
        assert_eq!(
            split.damage[..split.damage_outcome.len()],
            direct.damage[..direct.damage_outcome.len()],
            "damage rows"
        );
    }
}

#[cfg(test)]
#[path = "air_mission_scale_tests.rs"]
mod mission_scale_tests;
