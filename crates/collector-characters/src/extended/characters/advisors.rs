use super::*;
use engine_allocation::{default_vec, owned_vec, reserve, reserve_exact, resize, vec};
#[cfg(any(test, feature = "advisor_profile"))]
use std::cell::Cell;
#[cfg(feature = "advisor_profile")]
const DIAGNOSTIC_SLOTS: usize = 5;
#[cfg(feature = "advisor_profile")]
pub mod profile;

pub trait Changes {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn revision(&self, map: usize) -> Option<(u64, u64)>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub planned: u64,
    pub reused: u64,
    pub read: u64,
    pub roles: u64,
}
#[derive(Clone, Debug)]
struct Entry {
    pointer: usize,
    id: u32,
    country_offset: Option<usize>,
    header: [u8; 16],
    sentinel: [u8; 26],
    revision: Option<(u64, u64)>,
    rows: std::ops::Range<usize>,
    bytes: std::ops::Range<usize>,
}
#[derive(Debug)]
struct Owned {
    rows: Vec<cadvisor::Row>,
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}
impl Owned {
    fn new(rows: usize, bytes: usize, characters: usize) -> Self {
        Self {
            rows: owned_vec(default_vec(rows)),
            bytes: owned_vec(default_vec(bytes)),
            entries: owned_vec(vec(characters)),
        }
    }
    fn allocated(&self) -> u64 {
        collector_primitives::raw_memory::vector_bytes(&self.rows)
            + collector_primitives::raw_memory::vector_bytes(&self.bytes)
            + collector_primitives::raw_memory::vector_bytes(&self.entries)
    }
}
#[derive(Debug)]
pub struct Section {
    plan: collector_snapshots::snapshot_plan::SnapshotPlan,
    geometry: (usize, u32),
    census: bool,
    #[cfg(feature = "advisor_profile")]
    diagnostic: [u64; DIAGNOSTIC_SLOTS],
    current: Owned,
    previous: Owned,
    valid: bool,
    scope: (usize, usize),
    observation: Option<(u64, u64)>,
    pub(super) result: ReadResult,
    pub(super) counts: Counts,
    characters: usize,
    charge: collector_primitives::raw_memory::Charge,
}
impl Section {
    fn carried_whole(&self, scope: (usize, usize)) -> bool {
        self.valid
            && self.census
            && self.scope == scope
            && matches!(self.result.outcome, Outcome::Rows { .. })
            && self.current.entries.len() == self.characters
            && self
                .current
                .entries
                .iter()
                .all(|entry| entry.revision.is_some())
    }
    pub fn with_capacity(rows: usize, bytes: usize, characters: usize) -> Self {
        let mut cache = Self {
            plan: collector_snapshots::snapshot_plan::SnapshotPlan::default(),
            geometry: (0, 0),
            census: false,
            #[cfg(feature = "advisor_profile")]
            diagnostic: [0; DIAGNOSTIC_SLOTS],
            current: Owned::new(rows, bytes, characters),
            previous: Owned::new(rows, bytes, characters),
            valid: false,
            scope: (0, 0),
            observation: None,
            result: ReadResult::rows(Outcome::Rows { len: 0 }),
            counts: Counts::default(),
            characters: 0,
            charge: Default::default(),
        };
        cache.update_charge();
        cache
    }
    fn update_charge(&mut self) {
        self.charge.set(collector_primitives::raw_memory::Capacity {
            payload_vectors: self.current.allocated() + self.previous.allocated(),
            auxiliary_vectors: self.plan.allocated_bytes() as u64,
            ..Default::default()
        });
    }
    pub fn needs_growth(&self) -> bool {
        matches!(self.result.outcome, Outcome::Truncated { .. })
            || self.characters > self.current.entries.capacity()
    }
    pub fn grow(&mut self) {
        let (rows, bytes) = match self.result.outcome {
            Outcome::Truncated { len, unread_rows } => (
                self.current
                    .rows
                    .len()
                    .saturating_mul(2)
                    .max(len.saturating_add(unread_rows as usize)),
                self.current
                    .bytes
                    .len()
                    .saturating_mul(2)
                    .max(self.result.required_bytes),
            ),
            _ => (self.current.rows.len(), self.current.bytes.len()),
        };
        for owned in [&mut self.current, &mut self.previous] {
            let additional = self.characters.saturating_sub(owned.entries.len());
            if resize(&mut owned.rows, rows, cadvisor::Row::default()).is_err()
                || resize(&mut owned.bytes, bytes, 0).is_err()
                || reserve(&mut owned.entries, additional).is_err()
            {
                return;
            }
        }
        self.update_charge();
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn decode(&self, captured: &mut Captured) {
        let rows = self.result.outcome.len();
        if let Err(reason) = captured.reserve_advisors(AdvisorDemand {
            rows,
            bytes: self.result.bytes_len,
            overflowed: false,
        }) {
            captured
                .advisors
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
        } else {
            captured.advisors.rows.rows[..rows].copy_from_slice(&self.current.rows[..rows]);
            captured.advisors.bytes.first[..self.result.bytes_len]
                .copy_from_slice(&self.current.bytes[..self.result.bytes_len]);
            captured.advisors.primary(self.result);
        }
    }
    fn prepare(&mut self) {
        if self.census {
            self.plan.prepare();
            self.update_charge();
            return;
        }
        let data_reads = self.plan.pending_reads().map(|reads| {
            let mut countries = reads.iter().enumerate().filter(|(_, read)| read.len == 4);
            let mut indexes = Vec::new();
            reserve_exact(&mut indexes, self.current.entries.len()).ok()?;
            for entry in &mut self.current.entries {
                let (index, read) = countries.next()?;
                if entry.pointer.checked_add(ccharacter::COUNTRY) != Some(read.address) {
                    return None;
                }
                entry.country_offset = Some(read.destination_offset);
                indexes.push(index);
            }
            countries.next().is_none().then_some(indexes)
        });
        match data_reads {
            Some(Some(indexes)) => self.plan.prepare_with_data(&indexes),
            Some(None) => {
                for entry in &mut self.current.entries {
                    entry.country_offset = None;
                }
                self.plan.prepare();
            }
            None => self.plan.prepare(),
        }
        self.update_charge();
    }
    #[cfg(test)]
    fn capture(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
    ) {
        let roster =
            ccharacter_manager::Roster::read(memory, context.base, context.game_state).unwrap();
        let geometry = (0, roster.count());
        self.capture_section(
            memory,
            context,
            observer,
            Source::Roster { roster, geometry },
            false,
        );
    }
    #[cfg(test)]
    fn capture_section(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
        source: Source<'_>,
        count_only: bool,
    ) {
        self.capture_section_mode(memory, context, observer, source, count_only)
    }

    fn capture_section_mode(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
        source: Source<'_>,
        count_only: bool,
    ) {
        let scope = (context.base, context.game_state);
        let before = observer.quiet();
        let counted = Counted::new(memory);
        let same_source = || match &source {
            Source::Roster { geometry, .. } => !self.census && self.geometry == *geometry,
            Source::Census { roles, range } => {
                self.census
                    && self.current.entries.len() == range.len()
                    && self
                        .current
                        .entries
                        .iter()
                        .zip(range.clone())
                        .all(|(entry, index)| {
                            let occurrence = roles.occurrence(index);
                            entry.pointer == occurrence.pointer
                                && entry.id == occurrence.id()
                                && entry.header == occurrence.advisors()
                        })
            }
        };
        let certified = self.valid
            && self.scope == scope
            && !count_only
            && before.is_some()
            && same_source()
            && (self.observation == before
                || self.current.entries.iter().all(|entry| {
                    entry.revision.is_some()
                        && observer.revision(entry.pointer + ccharacter::ADVISOR_MAP_HEAD)
                            == entry.revision
                }));
        self.counts = Counts::default();
        if self.plan.begin(&counted, certified) {
            #[cfg(feature = "advisor_profile")]
            {
                self.diagnostic = counted.plan.get();
            }
            if before != observer.quiet() {
                self.valid = false;
                self.result = ReadResult::rows(Outcome::Unavailable {
                    reason: "advisor mutated during acquisition",
                });
                self.plan.invalidate();
            } else {
                for (position, entry) in self.current.entries.iter().enumerate() {
                    let country = match &source {
                        Source::Census { roles, range } => {
                            u32::from_le_bytes(roles.occurrence(range.start + position).country())
                        }
                        Source::Roster { .. } => {
                            let Some(offset) = entry.country_offset else {
                                continue;
                            };
                            let Some(bytes) = self.plan.completed_bytes(offset, 4) else {
                                self.valid = false;
                                self.result = ReadResult::rows(Outcome::Unavailable {
                                    reason: "advisor country binding unavailable",
                                });
                                self.plan.invalidate();
                                return;
                            };
                            u32::from_le_bytes(bytes.try_into().unwrap())
                        }
                    };
                    let rows = &mut self.current.rows[entry.rows.clone()];
                    if rows.first().is_some_and(|row| row.country != country) {
                        for row in rows {
                            row.country = country;
                        }
                    }
                }
                self.observation = before;
                self.counts.planned = 1;
                self.counts.reused = self.characters as u64;
                self.counts.roles = self.result.outcome.len() as u64;
            }
            return;
        }
        self.geometry = match &source {
            Source::Roster { geometry, .. } => *geometry,
            Source::Census { .. } => (0, 0),
        };
        self.census = matches!(source, Source::Census { .. });
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.entries.clear();
        self.counts = Counts::default();
        let reuse = self.valid && self.scope == scope && before.is_some();
        self.valid = false;
        self.scope = scope;
        let observed = self.plan.view(&counted);
        let prefetched = Prefetched {
            memory: &observed,
            geometry: self.geometry,
        };
        let memory = &prefetched;
        let characters = match &source {
            Source::Roster { roster, .. } => roster.count() as usize,
            Source::Census { range, .. } => range.len(),
        };
        if reserve(&mut self.current.entries, characters).is_err() {
            self.result = ReadResult::rows(Outcome::Unavailable {
                reason: "collector allocation failed",
            });
            return;
        }
        let mut batch = if count_only {
            Batch::storage(
                engine_native_memory::walk::Buffer::Count,
                engine_native_memory::walk::Buffer::Count,
            )
        } else {
            Batch::storage(
                engine_native_memory::walk::Buffer::Growing(&mut self.current.rows),
                engine_native_memory::walk::Buffer::Growing(&mut self.current.bytes),
            )
        };
        self.characters = match &source {
            Source::Roster { roster, .. } => roster.count() as usize,
            Source::Census { range, .. } => range.len(),
        };
        let result = (|| {
            let mut index = 0;
            macro_rules! observe {
                ($character:expr, $map:expr) => {{
                    let character = $character;
                    let map = $map;
                    let previous = self.previous.entries.get(index);
                    let revision = if reuse
                        && self.observation == before
                        && previous.is_some_and(|entry| {
                            entry.pointer == character.pointer
                                && entry.id == character.id
                                && entry.revision.is_some()
                        }) {
                        previous.unwrap().revision
                    } else {
                        observer.revision(character.pointer + ccharacter::ADVISOR_MAP_HEAD)
                    };
                    if let Some(entry) = previous {
                        batch.rows.ensure(
                            batch
                                .len
                                .checked_add(entry.rows.len())
                                .ok_or("advisor row count overflow")?,
                        )?;
                        batch.bytes.ensure(
                            batch
                                .bytes_len
                                .checked_add(entry.bytes.len())
                                .ok_or("advisor byte count overflow")?,
                        )?;
                    }
                    let hit = reuse
                        && batch.unread == 0
                        && previous.is_some_and(|entry| {
                            entry.pointer == character.pointer
                                && entry.id == character.id
                                && entry.header == map.header
                                && entry.sentinel == map.sentinel
                                && revision.is_some()
                                && entry.revision == revision
                                && entry.rows.len() <= batch.rows.len() - batch.len
                                && entry.bytes.len() <= batch.bytes.len() - batch.bytes_len
                        });
                    let row_start = batch.len;
                    let byte_start = batch.bytes_len;
                    if hit {
                        let entry = previous.unwrap();
                        let rows = entry.rows.len();
                        let bytes = entry.bytes.len();
                        batch.bytes[byte_start..byte_start + bytes]
                            .copy_from_slice(&self.previous.bytes[entry.bytes.clone()]);
                        for (target, source) in batch.rows[row_start..row_start + rows]
                            .iter_mut()
                            .zip(&self.previous.rows[entry.rows.clone()])
                        {
                            *target = *source;
                            target.country = character.country;
                            target.key_start = source.key_start - entry.bytes.start + byte_start;
                            target.idea_start = source.idea_start - entry.bytes.start + byte_start;
                            target.slot_start = source.slot_start - entry.bytes.start + byte_start;
                            target.modifier_start =
                                source.modifier_start - entry.bytes.start + byte_start;
                        }
                        batch.len += rows;
                        batch.bytes_len += bytes;
                        batch.required_bytes = batch
                            .required_bytes
                            .checked_add(bytes)
                            .ok_or("character byte count overflow")?;
                        self.counts.reused += 1;
                        self.counts.roles += rows as u64;
                    } else {
                        batch.append(
                            |rows, bytes| {
                                let _payload = self.plan.untracked_reads();
                                cadvisor::walk_definitions_entry_buffer(
                                    &map,
                                    context.base,
                                    character,
                                    rows,
                                    bytes,
                                )
                                .into()
                            },
                            |row, offset| {
                                relocate(&mut row.key_start, offset)
                                    && relocate(&mut row.idea_start, offset)
                                    && relocate(&mut row.slot_start, offset)
                                    && relocate(&mut row.modifier_start, offset)
                            },
                        );
                        self.counts.read += 1;
                    }
                    if self.current.entries.len() < self.current.entries.capacity() {
                        self.current.entries.push(Entry {
                            pointer: character.pointer,
                            id: character.id,
                            country_offset: None,
                            header: map.header,
                            sentinel: map.sentinel,
                            revision,
                            rows: row_start..batch.len,
                            bytes: byte_start..batch.bytes_len,
                        });
                    }
                    index += 1;
                }};
            }
            match source {
                Source::Roster { roster, .. } => {
                    let mut cursor = roster.cursor();
                    let mut page = ccharacter_manager::ViewPage::default();
                    while batch.active() {
                        let view = loop {
                            if let Some(view) = page.next() {
                                break Some(view?);
                            }
                            if !cursor.refill_page(
                                memory,
                                &mut page,
                                [false, false, false, true],
                            )? {
                                break None;
                            }
                        };
                        let Some(view) = view else {
                            break;
                        };
                        let character = view.entry()?;
                        observe!(
                            character,
                            cadvisor::MapView::read(&view, character.pointer)?
                        );
                    }
                }
                Source::Census { roles, range } => {
                    use census::{SENTINEL, SENTINEL_BATCH};
                    let mut heads = [0; SENTINEL_BATCH];
                    let mut sentinels = [0; SENTINEL_BATCH * SENTINEL];
                    let mut next = range.start;
                    while next < range.end && batch.active() {
                        let mut staged = 0;
                        while staged < SENTINEL_BATCH && next + staged < range.end {
                            let occurrence = roles.occurrence(next + staged);
                            match census::map_head(occurrence.advisors()) {
                                Some((head, _))
                                    if !census::inside_view(occurrence.pointer, head, SENTINEL) =>
                                {
                                    heads[staged] = head;
                                }
                                _ => break,
                            }
                            staged += 1;
                        }
                        let completed =
                            census::gather_sentinels(&observed, &heads[..staged], &mut sentinels);
                        let end = if staged == SENTINEL_BATCH {
                            next + staged
                        } else {
                            range.end.min(next + staged + 1)
                        };
                        for position in next..end {
                            if !batch.active() {
                                break;
                            }
                            let occurrence = roles.occurrence(position);
                            let view = ccharacter::View::acquired(
                                memory,
                                context.base,
                                occurrence.identity(roles.historical),
                                occurrence.fields([false, false, false, true]),
                            )?;
                            let character = view.entry()?;
                            let slot = position - next;
                            let map = if slot < staged {
                                let gathered = (slot < completed).then(|| {
                                    sentinels[slot * SENTINEL..][..SENTINEL].try_into().unwrap()
                                });
                                cadvisor::MapView::with_sentinel(&view, character.pointer, |_| {
                                    gathered
                                })?
                            } else {
                                cadvisor::MapView::read(&view, character.pointer)?
                            };
                            observe!(character, map);
                        }
                        next = end;
                    }
                }
            }
            Ok::<_, &'static str>(())
        })();
        if let Err(reason) = result {
            batch.fail(reason);
        }
        if before != observer.quiet() {
            batch.fail("advisor mutated during acquisition");
        }
        #[cfg(feature = "advisor_profile")]
        {
            self.diagnostic = counted.plan.get();
        }
        self.result = batch.result();
        if !self.plan.finish(
            matches!(self.result.outcome, Outcome::Rows { .. })
                && before.is_some()
                && self.current.entries.len() == self.characters
                && self
                    .current
                    .entries
                    .iter()
                    .all(|entry| entry.revision.is_some()),
        ) {
            self.result = ReadResult::rows(Outcome::Unavailable {
                reason: "advisor plan prefix differs",
            });
        }
        self.valid = matches!(self.result.outcome, Outcome::Rows { .. })
            && before.is_some()
            && self.current.entries.len() == self.characters;
        self.observation = before;
    }
}

enum Source<'a> {
    Roster {
        roster: ccharacter_manager::Roster,
        geometry: (usize, u32),
    },
    Census {
        roles: census::Admitted<'a>,
        range: std::ops::Range<usize>,
    },
}

#[derive(Debug, Default, Clone, Copy)]
struct ModifierSpan {
    start: usize,
    len: usize,
    recorded: bool,
    tooltip_start: usize,
    tooltip_len: usize,
    tooltip_recorded: bool,
    tooltip_failure: Option<&'static str>,
    certificate: Option<RoleCertificate>,
    carried: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoleCertificate {
    role: usize,
    character: usize,
    epoch: u64,
    sequence: u64,
}

#[derive(Clone, Copy)]
pub(super) struct RoleObservation {
    changes: &'static dyn collector_observers::character_changes::Changes,
}
impl RoleObservation {
    pub(super) fn with(
        changes: &'static dyn collector_observers::character_changes::Changes,
    ) -> Self {
        Self { changes }
    }
}
impl Default for RoleObservation {
    fn default() -> Self {
        Self::with(&collector_observers::character_changes::Observer)
    }
}
impl std::fmt::Debug for RoleObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("RoleObservation").finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct HeldModifiers {
    spans: Vec<ModifierSpan>,
    bytes: Vec<u8>,
    rows: usize,
}

static LIVE_MODIFIER_ROWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LIVE_MODIFIER_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LIVE_TOOLTIP_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static TOOLTIP_CAPTURED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TOOLTIP_REFUSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TOOLTIP_FAILURES: std::sync::Mutex<Vec<(&'static str, u64)>> =
    std::sync::Mutex::new(Vec::new());
static MODIFIER_CAPTURED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static MODIFIER_REFUSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// # Safety
/// Only the attaching runtime or its designated session sizing pass (or an isolated test owning
/// advisor globals) may publish process-wide advisor modifier demand.
pub unsafe fn note_modifier_demand(demand: cadvisor::ModifierDemand) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let rows = LIVE_MODIFIER_ROWS.fetch_max(demand.rows, Relaxed);
    let bytes = LIVE_MODIFIER_BYTES.fetch_max(demand.bytes, Relaxed);
    let tooltips = LIVE_TOOLTIP_BYTES.fetch_max(demand.tooltip_bytes, Relaxed);
    demand.rows > rows || demand.bytes > bytes || demand.tooltip_bytes > tooltips
}

pub fn live_modifier_demand() -> cadvisor::ModifierDemand {
    use std::sync::atomic::Ordering::Relaxed;
    cadvisor::ModifierDemand {
        rows: LIVE_MODIFIER_ROWS.load(Relaxed),
        bytes: LIVE_MODIFIER_BYTES.load(Relaxed),
        tooltip_bytes: LIVE_TOOLTIP_BYTES.load(Relaxed),
    }
}

pub(super) fn record_modifiers(rows: &[cadvisor::Row]) {
    use std::sync::atomic::Ordering::Relaxed;
    TOOLTIP_CAPTURED.fetch_add(
        rows.iter().filter(|row| row.tooltip_recorded).count() as u64,
        Relaxed,
    );
    TOOLTIP_REFUSED.fetch_add(
        rows.iter()
            .filter(|row| row.tooltip_failure.is_some())
            .count() as u64,
        Relaxed,
    );
    let mut failures = TOOLTIP_FAILURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for reason in rows.iter().filter_map(|row| row.tooltip_failure) {
        let index = match failures.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                if reserve(&mut failures, 1).is_err() {
                    return;
                }
                failures.insert(index, (reason, 0));
                index
            }
        };
        failures[index].1 = failures[index].1.saturating_add(1);
    }
    MODIFIER_CAPTURED.fetch_add(
        rows.iter().filter(|row| row.modifier_recorded).count() as u64,
        Relaxed,
    );
    MODIFIER_REFUSED.fetch_add(
        rows.iter()
            .filter(|row| row.modifier_failure.is_some())
            .count() as u64,
        Relaxed,
    );
}

#[derive(Debug)]
pub struct Modifiers {
    spans: Vec<ModifierSpan>,
    bytes: Vec<u8>,
    len: usize,
    required_rows: usize,
    required_bytes: usize,
    pub shortfall: usize,
    failure: Option<&'static str>,
    complete: bool,
    held: HeldModifiers,
    roles: RoleObservation,
    role_counts: collector_observers::character_changes::Counts,
    charge: collector_primitives::raw_memory::Charge,
}

impl Modifiers {
    fn carried_whole(&self) -> bool {
        self.complete
            && self.shortfall == 0
            && self.required_rows <= self.spans.len()
            && self.spans[..self.required_rows].iter().all(|span| {
                span.certificate.is_some()
                    && span.recorded
                    && span.tooltip_recorded
                    && span.tooltip_failure.is_none()
            })
    }

    fn with_demand(demand: cadvisor::ModifierDemand) -> Self {
        let mut value = Self {
            spans: owned_vec(default_vec(collector_primitives::country_slots::rows(
                demand.rows,
                1,
            ))),
            bytes: owned_vec(default_vec(collector_primitives::country_slots::rows(
                demand.bytes.saturating_add(demand.tooltip_bytes),
                1,
            ))),
            len: 0,
            required_rows: 0,
            required_bytes: 0,
            shortfall: 0,
            failure: None,
            complete: false,
            held: HeldModifiers::default(),
            roles: RoleObservation::default(),
            role_counts: Default::default(),
            charge: Default::default(),
        };
        value.update_charge();
        value
    }

    fn update_charge(&mut self) {
        self.charge.set(collector_primitives::raw_memory::Capacity {
            payload_vectors: collector_primitives::raw_memory::vector_bytes(&self.spans)
                + collector_primitives::raw_memory::vector_bytes(&self.bytes)
                + collector_primitives::raw_memory::vector_bytes(&self.held.spans)
                + collector_primitives::raw_memory::vector_bytes(&self.held.bytes),
            ..Default::default()
        });
    }

    fn begin(&mut self) {
        self.held.rows = 0;
        let rows = self.required_rows.min(self.spans.len());
        if !self.complete
            || !self.spans[..rows]
                .iter()
                .any(|span| span.certificate.is_some())
        {
            return;
        }
        if (self.held.spans.len() < self.spans.len() || self.held.bytes.len() < self.bytes.len())
            && (engine_native_memory::walk::Buffer::Growing(&mut self.held.spans)
                    .ensure(self.spans.len())
                    .is_err()
                || engine_native_memory::walk::Buffer::Growing(&mut self.held.bytes)
                    .ensure(self.bytes.len())
                    .is_err())
        {
            return;
        }
        std::mem::swap(&mut self.spans, &mut self.held.spans);
        std::mem::swap(&mut self.bytes, &mut self.held.bytes);
        self.held.rows = rows;
    }

    fn held_span(
        &self,
        index: usize,
        certificate: Option<RoleCertificate>,
    ) -> Option<ModifierSpan> {
        let certificate = certificate?;
        let span = *self.held.spans[..self.held.rows].get(index)?;
        (span.certificate == Some(certificate) && span.recorded && span.tooltip_recorded)
            .then_some(span)
    }

    fn needs_growth(&self) -> bool {
        let live = live_modifier_demand();
        self.shortfall != 0
            || self.required_rows.max(live.rows) > self.spans.len()
            || self
                .required_bytes
                .max(live.bytes.saturating_add(live.tooltip_bytes))
                > self.bytes.len()
    }

    fn grow(&mut self) {
        let live = live_modifier_demand();
        let rows = self
            .spans
            .len()
            .max(collector_primitives::country_slots::rows(
                self.required_rows.max(live.rows),
                1,
            ));
        let bytes = self
            .bytes
            .len()
            .max(collector_primitives::country_slots::rows(
                self.required_bytes
                    .max(live.bytes.saturating_add(live.tooltip_bytes)),
                1,
            ));
        if resize(&mut self.spans, rows, ModifierSpan::default()).is_err()
            || resize(&mut self.bytes, bytes, 0).is_err()
        {
            return;
        }
        if !self.held.spans.is_empty()
            && (resize(&mut self.held.spans, rows, ModifierSpan::default()).is_err()
                || resize(&mut self.held.bytes, bytes, 0).is_err())
        {
            return;
        }
        self.update_charge();
    }

    #[cfg(test)]
    fn capture<'a>(
        &mut self,
        memory: &impl Memory,
        rows: impl Iterator<Item = &'a cadvisor::Row> + Clone,
    ) {
        self.capture_mode(memory, rows.map(|row| (None, row)))
    }

    fn capture_mode<'a>(
        &mut self,
        memory: &impl Memory,
        rows: impl Iterator<Item = (Option<usize>, &'a cadvisor::Row)> + Clone,
    ) {
        self.begin();
        self.len = 0;
        self.required_rows = 0;
        self.required_bytes = 0;
        self.shortfall = 0;
        self.failure = None;
        self.role_counts = Default::default();
        self.spans.fill(ModifierSpan::default());
        let result = (|| {
            for (character, row) in rows.clone() {
                let certificate = character.and_then(|character| {
                    let (epoch, sequence) = self.roles.changes.sequence(character)?;
                    Some(RoleCertificate {
                        role: row.pointer,
                        character,
                        epoch,
                        sequence,
                    })
                });
                let index = self.required_rows;
                let carried = self.held_span(index, certificate);
                let (array, len) = match carried {
                    Some(held) => (None, held.len),
                    None => {
                        let array = cadvisor::modifier_entries(memory, row.pointer)?;
                        let len = (array.count as usize)
                            .checked_mul(engine_native_parts::modifier_entry::STRIDE)
                            .ok_or("advisor modifier cache size overflow")?;
                        (Some(array), len)
                    }
                };
                self.required_bytes = self
                    .required_bytes
                    .checked_add(len)
                    .ok_or("advisor byte count overflow")?;
                self.required_rows = index.checked_add(1).ok_or("advisor row count overflow")?;
                engine_native_memory::walk::Buffer::Growing(&mut self.spans)
                    .ensure(self.required_rows)?;
                engine_native_memory::walk::Buffer::Growing(&mut self.bytes).ensure(
                    self.len
                        .checked_add(len)
                        .ok_or("advisor byte count overflow")?,
                )?;

                let mut span = ModifierSpan {
                    certificate,
                    carried: carried.is_some(),
                    ..ModifierSpan::default()
                };
                if index >= self.spans.len() || len > self.bytes.len() - self.len {
                    self.shortfall += 1;
                } else {
                    span.start = self.len;
                    span.len = len;
                    span.recorded = true;
                    let target = &mut self.bytes[self.len..self.len + len];
                    match (array, carried) {
                        (Some(array), _) => {
                            cadvisor::copy_modifier_entries(memory, array, target)?;
                        }
                        (None, Some(held)) => {
                            target.copy_from_slice(&self.held.bytes[held.start..held.start + len])
                        }
                        (None, None) => unreachable!("a role is either read or carried"),
                    }
                    self.len += len;
                }
                if carried.is_some() {
                    self.role_counts.reused += 1;
                } else {
                    self.role_counts.read += 1;
                }
                if let Some(target) = self.spans.get_mut(index) {
                    *target = span;
                }
            }
            for (index, (_, row)) in rows.enumerate() {
                let mut span = self.spans.get(index).copied().unwrap_or_default();
                let mut destination = engine_native_memory::walk::Buffer::GrowingFrom(&mut self.bytes, self.len);
                let read = if span.carried {
                    let held = self.held.spans[index];
                    let len = held.tooltip_len;
                    destination.ensure(len)?;
                    let recorded = len <= destination.len();
                    if recorded {
                        destination[..len].copy_from_slice(
                            &self.held.bytes[held.tooltip_start..held.tooltip_start + len],
                        );
                    }
                    Ok(cadvisor::TooltipRead {
                        len: if recorded { len } else { 0 },
                        required: len,
                        recorded,
                    })
                } else {
                    cadvisor::copy_tooltip_keys_buffer(memory, row.pointer, destination)
                };
                match read {
                    Ok(read) => {
                        let required = self.required_bytes.checked_add(read.required);
                        if let Some(required) = required {
                            self.required_bytes = required;
                        }
                        if required.is_none() {
                            span.tooltip_failure = Some("advisor tooltip byte count overflow");
                        } else if read.recorded && index < self.spans.len() {
                            span.tooltip_start = self.len;
                            span.tooltip_len = read.len;
                            span.tooltip_recorded = true;
                            self.len += read.len;
                        } else {
                            span.tooltip_failure = Some(cadvisor::TOOLTIP_CAPACITY_REASON);
                            if span.recorded {
                                self.shortfall += 1;
                            }
                        }
                    }
                    Err(reason) => span.tooltip_failure = Some(reason),
                }
                if let Some(target) = self.spans.get_mut(index) {
                    *target = span;
                }
            }
            Ok(())
        })();
        if let Err(reason) = result {
            self.failure.get_or_insert(reason);
        }
        self.complete = self.failure.is_none();
        collector_observers::character_changes::record(
            collector_observers::character_changes::Table::AdvisorRoles,
            &self.role_counts,
        );
    }
}

#[derive(Debug)]
pub struct Cache {
    pub sections: [Section; 2],
    pub modifiers: Modifiers,
    pub(super) result: ReadResult,
    pub(super) counts: Counts,
    quiet: Option<Quiet>,
    content: u64,
}

fn fresh_content() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quiet {
    scope: (usize, usize),
    advisors: (u64, u64),
    characters: (u64, u64),
    generation: u64,
}
static LIVE_DEFINITIONS: [[std::sync::atomic::AtomicUsize; 3]; 2] =
    [const { [const { std::sync::atomic::AtomicUsize::new(0) }; 3] }; 2];
#[cfg(any(test, feature = "test-support"))]
pub(super) fn forget_live() {
    for hint in [
        &LIVE_MODIFIER_ROWS,
        &LIVE_MODIFIER_BYTES,
        &LIVE_TOOLTIP_BYTES,
    ]
    .into_iter()
    .chain(LIVE_DEFINITIONS.iter().flatten())
    {
        hint.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}
pub(super) fn remember_definition_demand(demand: [cadvisor::DefinitionDemand; 2]) {
    for (storage, demand) in LIVE_DEFINITIONS.iter().zip(demand) {
        for (target, count) in storage
            .iter()
            .zip([demand.characters, demand.rows, demand.bytes])
        {
            target.fetch_max(count, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl Cache {
    pub fn new() -> Self {
        let mut value = Self::with_modifier_demand(live_modifier_demand());
        for (section, demand) in value.sections.iter_mut().zip(&LIVE_DEFINITIONS) {
            use std::sync::atomic::Ordering::Relaxed;
            for storage in [&mut section.current, &mut section.previous] {
                let characters = demand[0]
                    .load(Relaxed)
                    .saturating_sub(storage.entries.len());
                if engine_native_memory::walk::Buffer::Growing(&mut storage.rows)
                    .ensure(demand[1].load(Relaxed))
                    .is_err()
                    || engine_native_memory::walk::Buffer::Growing(&mut storage.bytes)
                        .ensure(demand[2].load(Relaxed))
                        .is_err()
                    || reserve(&mut storage.entries, characters).is_err()
                {
                    break;
                }
            }
            section.update_charge();
        }
        value
    }

    pub fn with_modifier_demand(demand: cadvisor::ModifierDemand) -> Self {
        Self {
            sections: [
                Section::with_capacity(0, 0, 0),
                Section::with_capacity(0, 0, 0),
            ],
            modifiers: Modifiers::with_demand(demand),
            result: ReadResult::rows(Outcome::Rows { len: 0 }),
            counts: Counts::default(),
            quiet: None,
            content: fresh_content(),
        }
    }

    fn carry_quiet(&mut self, observer: &dyn Changes, before: Option<(u64, u64)>) -> bool {
        if before != observer.quiet() {
            return false;
        }
        let rows = self.result.outcome.len() as u64;
        self.counts = Counts {
            reused: self
                .sections
                .iter()
                .map(|section| section.characters as u64)
                .sum(),
            roles: rows,
            ..Counts::default()
        };
        collector_observers::character_changes::record(
            collector_observers::character_changes::Table::AdvisorRoles,
            &collector_observers::character_changes::Counts {
                reused: rows,
                quiet: true,
                ..Default::default()
            },
        );
        true
    }
    #[cfg(test)]
    pub(super) fn observe_roles(
        &mut self,
        changes: &'static dyn collector_observers::character_changes::Changes,
    ) {
        self.modifiers.roles = RoleObservation::with(changes);
    }
    pub fn needs_growth(&self) -> bool {
        self.sections.iter().any(Section::needs_growth) || self.modifiers.needs_growth()
    }
    pub fn grow(&mut self) {
        for section in &mut self.sections {
            section.grow();
        }
        self.modifiers.grow();
    }
    pub fn prepare(&mut self) {
        for section in &mut self.sections {
            section.prepare();
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
        loaned: Option<census::Admitted<'_>>,
    ) {
        self.capture_mode(memory, context, observer, loaned)
    }

    fn capture_mode(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
        loaned: Option<census::Admitted<'_>>,
    ) {
        self.counts = Counts::default();
        let counted = Counted::new(memory);
        let before = observer.quiet();
        let characters = self.modifiers.roles.changes.quiet();
        let scope = (context.base, context.game_state);
        let quiet =
            loaned
                .zip(before)
                .zip(characters)
                .and_then(|((roles, advisors), characters)| {
                    (roles.generation != 0).then_some(Quiet {
                        scope,
                        advisors,
                        characters,
                        generation: roles.generation,
                    })
                });
        if quiet.is_some()
            && self.quiet == quiet
            && self.carry_quiet(observer, before)
        {
            return;
        }
        self.quiet = None;
        self.content = fresh_content();
        self.counts = Counts::default();
        self.result = ReadResult::rows(Outcome::Rows { len: 0 });
        #[cfg(feature = "advisor_profile")]
        let mut measurements = [[0; 11]; 2];
        #[cfg(feature = "advisor_profile")]
        let mut measurement_iter = measurements.iter_mut();
        let result = (|| {
            let sources = match loaned {
                Some(roles) => [
                    Source::Census {
                        roles,
                        range: 0..roles.historical,
                    },
                    Source::Census {
                        roles,
                        range: roles.historical..roles.len(),
                    },
                ],
                None => {
                    ccharacter_manager::Roster::read(&counted, context.base, context.game_state)?
                        .sections()
                        .map(|(array, roster)| Source::Roster {
                            roster,
                            geometry: (array.data, array.count),
                        })
                }
            };
            let mut len = 0;
            let mut unread = 0_u32;
            for (section, source) in self.sections.iter_mut().zip(sources) {
                #[cfg(feature = "advisor_profile")]
                let stamp = profile::Stamp::start();
                section.capture_section_mode(memory, context, observer, source, unread != 0);
                #[cfg(feature = "advisor_profile")]
                {
                    let elapsed = stamp.finish();
                    let sample = measurement_iter.next().unwrap();
                    sample[..2].copy_from_slice(&elapsed);
                    sample[2] = section.counts.read;
                    sample[3] = section.counts.reused;
                    sample[4] = section.counts.planned;
                    sample[5..10].copy_from_slice(&section.diagnostic);
                    sample[10] = section.characters as u64;
                }
                self.counts.planned += section.counts.planned;
                self.counts.reused += section.counts.reused;
                self.counts.read += section.counts.read;
                self.counts.roles += section.counts.roles;
                match section.result.outcome {
                    Outcome::Rows { len: n } => len += n,
                    Outcome::Truncated {
                        len: n,
                        unread_rows,
                    } => {
                        len += n;
                        unread = unread
                            .checked_add(unread_rows)
                            .ok_or("character missing row count overflow")?;
                    }
                    Outcome::Unavailable { reason } => return Err(reason),
                }
                self.result.bytes_len += section.result.bytes_len;
                self.result.required_bytes = self
                    .result
                    .required_bytes
                    .checked_add(section.result.required_bytes)
                    .ok_or("character byte count overflow")?;
            }
            self.result.outcome = if unread == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: unread,
                }
            };
            if before != observer.quiet() {
                return Err("advisor mutated during acquisition");
            }
            Ok(())
        })();
        #[cfg(feature = "advisor_profile")]
        profile::record(measurements);
        if let Err(reason) = result {
            self.result = ReadResult::rows(Outcome::Unavailable { reason });
            for section in &mut self.sections {
                section.valid = false;
                section.plan.invalidate();
            }
        } else {
            self.modifiers.capture_mode(
                &counted,
                self.sections.iter().flat_map(|section| {
                    let entries = &section.current.entries;
                    let mut entry = 0;
                    section.current.rows[..section.result.outcome.len()]
                        .iter()
                        .enumerate()
                        .map(move |(index, row)| {
                            while entries.get(entry).is_some_and(|e| e.rows.end <= index) {
                                entry += 1;
                            }
                            let character = entries
                                .get(entry)
                                .filter(|e| e.rows.contains(&index))
                                .map(|e| e.pointer);
                            (character, row)
                        })
                }),
            );
            if let Some(reason) = self.modifiers.failure {
                self.result = ReadResult::rows(Outcome::Unavailable { reason });
                for section in &mut self.sections {
                    section.valid = false;
                    section.plan.invalidate();
                }
            }
        }
        for section in &mut self.sections {
            section.update_charge();
        }
        self.modifiers.update_charge();

        self.quiet = quiet.filter(|_| {
            matches!(self.result.outcome, Outcome::Rows { .. })
                && self
                    .sections
                    .iter()
                    .all(|section| section.carried_whole(scope))
                && self.modifiers.carried_whole()
                && before == observer.quiet()
        });
    }
    pub fn decode(&self, captured: &mut Captured) {
        if let Some((_, result)) = captured
            .held
            .advisors
            .filter(|&(content, _)| content == self.content)
        {
            captured.advisors.primary(result);
            record_modifiers(&captured.advisors.rows.rows[..result.outcome.len()]);
            return;
        }
        captured.held.advisors = None;
        if let Outcome::Unavailable { .. } = self.result.outcome {
            captured.advisors.primary(self.result);
            return;
        }
        let Some(bytes_len) = self.result.bytes_len.checked_add(self.modifiers.len) else {
            captured
                .advisors
                .primary(ReadResult::rows(Outcome::Unavailable {
                    reason: "advisor byte count overflow",
                }));
            return;
        };
        let Some(required_bytes) = self
            .result
            .required_bytes
            .checked_add(self.modifiers.required_bytes)
        else {
            captured
                .advisors
                .primary(ReadResult::rows(Outcome::Unavailable {
                    reason: "advisor byte count overflow",
                }));
            return;
        };
        if let Err(reason) = captured.reserve_advisors(AdvisorDemand {
            rows: self.result.outcome.len(),
            bytes: bytes_len,
            overflowed: false,
        }) {
            captured
                .advisors
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
            return;
        }
        let mut row_start = 0;
        let mut byte_start = 0;
        let modifier_start = self.result.bytes_len;
        captured.advisors.bytes.first[modifier_start..modifier_start + self.modifiers.len]
            .copy_from_slice(&self.modifiers.bytes[..self.modifiers.len]);
        for section in &self.sections {
            let len = section.result.outcome.len();
            let bytes = section.result.bytes_len;
            captured.advisors.bytes.first[byte_start..byte_start + bytes]
                .copy_from_slice(&section.current.bytes[..bytes]);
            for (target, source) in captured.advisors.rows.rows[row_start..row_start + len]
                .iter_mut()
                .zip(&section.current.rows[..len])
            {
                *target = *source;
                target.key_start += byte_start;
                target.idea_start += byte_start;
                target.slot_start += byte_start;
                let modifier = self
                    .modifiers
                    .spans
                    .get(row_start)
                    .copied()
                    .unwrap_or_default();
                target.modifier_start = if modifier.recorded {
                    modifier_start + modifier.start
                } else {
                    0
                };
                target.tooltip_start = modifier_start + modifier.tooltip_start;
                target.tooltip_len = modifier.tooltip_len;
                target.tooltip_recorded = modifier.tooltip_recorded;
                target.tooltip_failure = modifier.tooltip_failure.or_else(|| {
                    (row_start >= self.modifiers.spans.len())
                        .then_some(cadvisor::TOOLTIP_CAPACITY_REASON)
                });
                target.modifier_len = modifier.len;
                target.modifier_recorded = modifier.recorded;
                target.modifier_failure =
                    (!modifier.recorded).then_some(cadvisor::MODIFIER_CAPACITY_REASON);
                row_start += 1;
            }
            byte_start += bytes;
        }
        let result = ReadResult {
            bytes_len,
            required_bytes,
            ..self.result
        };
        captured.advisors.primary(result);
        captured.held.advisors = Some((self.content, result));
        record_modifiers(&captured.advisors.rows.rows[..row_start]);
    }
}

struct Prefetched<'a, M> {
    memory: &'a M,
    geometry: (usize, u32),
}
impl<M: Memory> Memory for Prefetched<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.memory.copy(address, output);
        if ok
            && !output.is_empty()
            && output.len().is_multiple_of(8)
            && address.checked_sub(self.geometry.0).is_some_and(|offset| {
                offset.is_multiple_of(8)
                    && offset
                        .checked_add(output.len())
                        .is_some_and(|end| end <= self.geometry.1 as usize * 8)
            })
        {
            for pointer in output.chunks_exact(8) {
                let pointer = u64::from_le_bytes(pointer.try_into().unwrap()) as usize;
                for (offset, len) in [
                    (0, 16),
                    (ccharacter::COUNTRY, 4),
                    (ccharacter::ADVISOR_MAP_HEAD, 16),
                ] {
                    if let Some(address) = pointer.checked_add(offset) {
                        self.memory.prefetch(address, len);
                    }
                }
            }
        }
        ok
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.memory.copy_scattered(addresses, outputs);
        if N == 7
            && completed == N
            && outputs[0].len() == 16
            && outputs[1].len() == 4
            && outputs[2..6].iter().all(|span| span.is_empty())
            && outputs[6].len() == 16
            && addresses[0].checked_add(ccharacter::COUNTRY) == Some(addresses[1])
            && addresses[0].checked_add(ccharacter::ADVISOR_MAP_HEAD) == Some(addresses[6])
        {
            let head = u64::from_le_bytes(outputs[6][..8].try_into().unwrap()) as usize;
            self.memory.prefetch(head, 26);
        }
        completed
    }
}

struct Counted<'a, M> {
    memory: &'a M,
    #[cfg(feature = "advisor_profile")]
    plan: Cell<[u64; DIAGNOSTIC_SLOTS]>,
}
impl<'a, M> Counted<'a, M> {
    fn new(memory: &'a M) -> Self {
        Self {
            memory,
            #[cfg(feature = "advisor_profile")]
            plan: Cell::new([0; DIAGNOSTIC_SLOTS]),
        }
    }
}
impl<M: Memory> Memory for Counted<'_, M> {
    fn prefetch(&self, address: usize, len: usize) {
        self.memory.prefetch(address, len);
    }
    fn execute_read_plan(
        &self,
        plan: &engine_native_memory::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<engine_native_memory::read_plan::Outcome> {
        #[cfg(feature = "advisor_profile")]
        use engine_native_memory::read_plan::Outcome as PlanOutcome;
        #[cfg(feature = "advisor_profile")]
        let stamp = profile::Stamp::start();
        let result = self.memory.execute_read_plan(plan, destination);
        #[cfg(feature = "advisor_profile")]
        {
            let elapsed = stamp.finish();
            let (kind, read) = match result {
                Some(PlanOutcome::Complete) => (1, None),
                Some(PlanOutcome::ControlChanged { completed_reads }) => (
                    2,
                    completed_reads
                        .checked_sub(1)
                        .and_then(|i| plan.reads().get(i)),
                ),
                Some(PlanOutcome::ReadFailed { completed_reads }) => {
                    (3, plan.reads().get(completed_reads))
                }
                None => (4, None),
            };
            self.plan.set([
                elapsed[0],
                elapsed[1],
                kind,
                read.map_or(0, |r| r.address as u64),
                read.map_or(0, |r| r.len as u64),
            ]);
        }
        result
    }
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.memory.copy(address, output)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        self.memory.copy_scattered(addresses, outputs)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        self.memory.copy_scattered_prefetched(addresses, outputs)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.memory.copy_records(sources, layout, destination)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests;
