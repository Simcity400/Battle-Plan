//! Advisor definitions retained only under uninterrupted mutation observation.
//! Double-buffered owned arenas let one changed character grow without moving
//! later cache keys or forcing the rest of the roster to be reacquired.
use super::*;
use engine::allocation::{default_vec, owned_vec, reserve, reserve_exact, resize, vec};
use std::cell::Cell;
#[cfg(feature = "advisor_profile")]
pub(crate) mod profile;

pub(crate) trait Changes {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn revision(&self, map: usize) -> Option<(u64, u64)>;
    fn verify(&self) -> bool;
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counts {
    pub planned: u64,
    pub reused: u64,
    pub read: u64,
    pub roles: u64,
    pub verified: u64,
    pub mismatches: u64,
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
        crate::raw_memory::vector_bytes(&self.rows)
            + crate::raw_memory::vector_bytes(&self.bytes)
            + crate::raw_memory::vector_bytes(&self.entries)
    }
}
#[derive(Debug)]
struct Section {
    plan: crate::snapshot_plan::SnapshotPlan,
    geometry: (usize, u32),
    /// Entries and plan come from a loaned census, whose plan traces only
    /// sentinels; roster passes trace the roster, views and sentinels.
    census: bool,
    #[cfg(feature = "advisor_profile")]
    diagnostic: [u64; 5],
    current: Owned,
    previous: Owned,
    valid: bool,
    scope: (usize, usize),
    observation: Option<(u64, u64)>,
    pub(super) result: ReadResult,
    pub(super) raw_bytes: u64,
    pub(super) counts: Counts,
    characters: usize,
    charge: crate::raw_memory::Charge,
}
impl Section {
    fn with_capacity(rows: usize, bytes: usize, characters: usize) -> Self {
        let mut cache = Self {
            plan: crate::snapshot_plan::SnapshotPlan::new(characters * 6, characters * 96),
            geometry: (0, 0),
            census: false,
            #[cfg(feature = "advisor_profile")]
            diagnostic: [0; 5],
            current: Owned::new(rows, bytes, characters),
            previous: Owned::new(rows, bytes, characters),
            valid: false,
            scope: (0, 0),
            observation: None,
            result: ReadResult::rows(Outcome::Rows { len: 0 }),
            raw_bytes: 0,
            counts: Counts::default(),
            characters: 0,
            charge: Default::default(),
        };
        cache.update_charge();
        cache
    }
    fn update_charge(&mut self) {
        self.charge.set(crate::raw_memory::Capacity {
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
    #[cfg(test)]
    pub fn decode(&self, captured: &mut Captured) {
        let rows = self.result.outcome.len();
        // Decode owns these allocations. Acquisition never resizes an arena.
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
            // Census passes read no country; certification applies the census's.
            self.plan.prepare();
            self.update_charge();
            return;
        }
        // Country is a result value, never a selecting pointer or a validation
        // predicate. Bind its exact read occurrence to the matching character;
        // the remaining observations retain equality controls.
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
                // A changed reader shape retains full comparisons. Never
                // infer a binding from an address alone or omit an unknown read.
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
    fn capture_section(
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
            // The census plan traces only sentinels: every occurrence, identity
            // and map header must repeat the ones these entries were built from.
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
            && !observer.verify()
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
            self.raw_bytes = counted.bytes.get();
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
                    // Every role in this occurrence has the same country.
                    // Avoid making unchanged row cache lines writable on
                    // each validation pass (decode may have read them on
                    // another core since the previous acquisition).
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
        let mut batch = if count_only {
            Batch::new(&mut [], &mut [])
        } else {
            Batch::new(&mut self.current.rows, &mut self.current.bytes)
        };
        self.characters = match &source {
            Source::Roster { roster, .. } => roster.count() as usize,
            Source::Census { range, .. } => range.len(),
        };
        let result = (|| {
            let mut index = 0;
            // One occurrence: fresh revision, owned reuse or a fresh walk.
            macro_rules! observe {
                ($character:expr, $map:expr) => {{
                    let character = $character;
                    let map = $map;
                    let previous = self.previous.entries.get(index);
                    // A quiet whole-domain revision avoids one hash-table lookup
                    // per unchanged character. Identity/controls remain fresh.
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
                    if hit && !observer.verify() {
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
                                // Role payloads cannot select another character's
                                // controls. Their future reuse is certified by the
                                // watched revision, never by these omitted reads.
                                let _payload = self.plan.untracked_reads();
                                cadvisor::walk_definitions_entry(
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
                        if hit {
                            self.counts.verified += 1;
                            let old = previous.unwrap();
                            let same = batch.failure.is_none()
                                && batch.unread == 0
                                && batch.len - row_start == old.rows.len()
                                && batch.bytes[byte_start..batch.bytes_len]
                                    == self.previous.bytes[old.bytes.clone()]
                                && batch.rows[row_start..batch.len]
                                    .iter()
                                    .zip(&self.previous.rows[old.rows.clone()])
                                    .all(|(new, old_row)| {
                                        let mut old_row = *old_row;
                                        old_row.country = character.country;
                                        old_row.key_start =
                                            old_row.key_start - old.bytes.start + byte_start;
                                        old_row.idea_start =
                                            old_row.idea_start - old.bytes.start + byte_start;
                                        old_row.slot_start =
                                            old_row.slot_start - old.bytes.start + byte_start;
                                        old_row.modifier_start =
                                            old_row.modifier_start - old.bytes.start + byte_start;
                                        *new == old_row
                                    });
                            if !same {
                                self.counts.mismatches += 1;
                            }
                        }
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
                        // Sentinels are gathered ahead of their maps. A header that
                        // fails before its sentinel read, or whose sentinel lies
                        // inside the character view, ends the batch.
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
                    .all(|entry| entry.revision.is_some())
                && self.counts.mismatches == 0
                && !observer.verify(),
        ) {
            self.result = ReadResult::rows(Outcome::Unavailable {
                reason: "advisor plan prefix differs",
            });
        }
        self.raw_bytes = counted.bytes.get();
        self.valid = matches!(self.result.outcome, Outcome::Rows { .. })
            && before.is_some()
            && self.current.entries.len() == self.characters
            && self.counts.mismatches == 0;
        self.observation = before;
    }
}

/// Where a section's occurrences come from.
enum Source<'a> {
    /// One roster vector, read with its character views.
    Roster {
        roster: ccharacter_manager::Roster,
        geometry: (usize, u32),
    },
    /// Occurrences of the roster job's census loaned in this callback.
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

pub(crate) fn note_modifier_demand(demand: cadvisor::ModifierDemand) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let rows = LIVE_MODIFIER_ROWS.fetch_max(demand.rows, Relaxed);
    let bytes = LIVE_MODIFIER_BYTES.fetch_max(demand.bytes, Relaxed);
    let tooltips = LIVE_TOOLTIP_BYTES.fetch_max(demand.tooltip_bytes, Relaxed);
    demand.rows > rows || demand.bytes > bytes || demand.tooltip_bytes > tooltips
}

pub(crate) fn live_modifier_demand() -> cadvisor::ModifierDemand {
    use std::sync::atomic::Ordering::Relaxed;
    cadvisor::ModifierDemand {
        rows: LIVE_MODIFIER_ROWS.load(Relaxed),
        bytes: LIVE_MODIFIER_BYTES.load(Relaxed),
        tooltip_bytes: LIVE_TOOLTIP_BYTES.load(Relaxed),
    }
}

pub(super) fn modifier_status() -> impl serde::Serialize {
    use std::sync::atomic::Ordering::Relaxed;
    let failures = owned_vec(engine::allocation::copy_vec(
        &TOOLTIP_FAILURES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    ));
    crate::status::status!({
        "tooltip_captured": TOOLTIP_CAPTURED.load(Relaxed),
        "tooltip_refused": TOOLTIP_REFUSED.load(Relaxed),
        "tooltip_refusals": crate::status::map(failures.into_iter()),
        "captured": MODIFIER_CAPTURED.load(Relaxed),
        "refused": MODIFIER_REFUSED.load(Relaxed),
        "refusals": {
            "advisor modifier cache exceeds admitted capacity": MODIFIER_REFUSED.load(Relaxed),
        },
        "scope": "cumulative hourly advisor modifier inputs; capacity refusals leave modifier_cache absent and retain AdvisorRoles and definition certificates",
    })
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

/// Fresh hourly inputs, outside the mutation-certified definition program.
/// These headers and entries are never supplied by a previous observation.
#[derive(Debug)]
struct Modifiers {
    spans: Vec<ModifierSpan>,
    bytes: Vec<u8>,
    len: usize,
    required_rows: usize,
    required_bytes: usize,
    shortfall: usize,
    failure: Option<&'static str>,
    charge: crate::raw_memory::Charge,
}

impl Modifiers {
    fn with_demand(demand: cadvisor::ModifierDemand) -> Self {
        let mut value = Self {
            spans: owned_vec(default_vec(crate::country_scale::rows(demand.rows, 1))),
            bytes: owned_vec(default_vec(crate::country_scale::rows(
                demand.bytes.saturating_add(demand.tooltip_bytes),
                1,
            ))),
            len: 0,
            required_rows: 0,
            required_bytes: 0,
            shortfall: 0,
            failure: None,
            charge: Default::default(),
        };
        value.update_charge();
        value
    }

    fn update_charge(&mut self) {
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.spans)
                + crate::raw_memory::vector_bytes(&self.bytes),
            ..Default::default()
        });
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
        let rows = self.spans.len().max(crate::country_scale::rows(
            self.required_rows.max(live.rows),
            1,
        ));
        let bytes = self.bytes.len().max(crate::country_scale::rows(
            self.required_bytes
                .max(live.bytes.saturating_add(live.tooltip_bytes)),
            1,
        ));
        if resize(&mut self.spans, rows, ModifierSpan::default()).is_err()
            || resize(&mut self.bytes, bytes, 0).is_err()
        {
            return;
        }
        self.update_charge();
    }

    fn capture<'a>(
        &mut self,
        memory: &impl Memory,
        rows: impl Iterator<Item = &'a cadvisor::Row> + Clone,
    ) {
        self.len = 0;
        self.required_rows = 0;
        self.required_bytes = 0;
        self.shortfall = 0;
        self.failure = None;
        self.spans.fill(ModifierSpan::default());
        let result = (|| {
            // Acquire every pre-existing numeric cache before appended keys
            // can use the remaining byte arena.
            for row in rows.clone() {
                let array = cadvisor::modifier_entries(memory, row.pointer)?;
                let len = (array.count as usize)
                    .checked_mul(engine::modifier_entry::STRIDE)
                    .ok_or("advisor modifier cache size overflow")?;
                self.required_bytes = self
                    .required_bytes
                    .checked_add(len)
                    .ok_or("advisor byte count overflow")?;
                let index = self.required_rows;
                self.required_rows = index.checked_add(1).ok_or("advisor row count overflow")?;
                let mut span = ModifierSpan::default();
                if index >= self.spans.len() || len > self.bytes.len() - self.len {
                    self.shortfall += 1;
                } else {
                    span.start = self.len;
                    span.len = len;
                    span.recorded = true;
                    cadvisor::copy_modifier_entries(
                        memory,
                        array,
                        &mut self.bytes[self.len..self.len + len],
                    )?;
                    self.len += len;
                }
                if let Some(target) = self.spans.get_mut(index) {
                    *target = span;
                }
            }
            for (index, row) in rows.enumerate() {
                let mut span = self.spans.get(index).copied().unwrap_or_default();
                let destination = if index < self.spans.len() {
                    &mut self.bytes[self.len..]
                } else {
                    &mut []
                };
                match cadvisor::copy_tooltip_keys(memory, row.pointer, destination) {
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
    }
}

/// Independent historical/generated certificates keep generated roster growth
/// from invalidating the large historical pass. Roots are acquired once first.
#[derive(Debug)]
pub(crate) struct Cache {
    sections: [Section; 2],
    modifiers: Modifiers,
    pub(super) result: ReadResult,
    pub(super) raw_bytes: u64,
    pub(super) counts: Counts,
}
impl Cache {
    pub fn new() -> Self {
        Self::with_modifier_demand(live_modifier_demand())
    }

    pub(super) fn with_modifier_demand(demand: cadvisor::ModifierDemand) -> Self {
        Self {
            sections: [
                Section::with_capacity(16384, 1 << 20, 16384),
                Section::with_capacity(4096, 1 << 18, 8192),
            ],
            modifiers: Modifiers::with_demand(demand),
            result: ReadResult::rows(Outcome::Rows { len: 0 }),
            raw_bytes: 0,
            counts: Counts::default(),
        }
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
    /// A loaned census supplies each section's occurrences, identities,
    /// countries and map headers; sentinels and role payloads stay fresh.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
        loaned: Option<census::Admitted<'_>>,
    ) {
        self.counts = Counts::default();
        self.raw_bytes = 0;
        for section in &mut self.sections {
            section.raw_bytes = 0;
        }
        self.result = ReadResult::rows(Outcome::Rows { len: 0 });
        let counted = Counted::new(memory);
        let before = observer.quiet();
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
                section.capture_section(memory, context, observer, source, unread != 0);
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
                self.counts.verified += section.counts.verified;
                self.counts.mismatches += section.counts.mismatches;
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
            self.modifiers.capture(
                &counted,
                self.sections.iter().flat_map(|section| {
                    section.current.rows[..section.result.outcome.len()].iter()
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
        self.raw_bytes = self
            .sections
            .iter()
            .map(|section| section.raw_bytes)
            .sum::<u64>()
            + counted.bytes.get();
    }
    pub fn decode(&self, captured: &mut Captured) {
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
        captured.advisors.primary(ReadResult {
            bytes_len,
            required_bytes,
            ..self.result
        });
        record_modifiers(&captured.advisors.rows.rows[..row_start]);
    }
}

/// Hints follow their selecting pointer read. The prepared executor repeats
/// them only after that pointer's exact control comparison succeeds. This
/// overlaps independent character cache misses without changing read order.
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
        // Recognize this reader's exact CCharacter::View field batch. Role
        // payload batches must not be interpreted as character map headers.
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
    bytes: Cell<u64>,
    #[cfg(feature = "advisor_profile")]
    plan: Cell<[u64; 5]>,
}
impl<'a, M> Counted<'a, M> {
    fn new(memory: &'a M) -> Self {
        Self {
            memory,
            bytes: Cell::new(0),
            #[cfg(feature = "advisor_profile")]
            plan: Cell::new([0; 5]),
        }
    }
}
impl<M: Memory> Memory for Counted<'_, M> {
    fn prefetch(&self, address: usize, len: usize) {
        self.memory.prefetch(address, len);
    }
    fn execute_read_plan(
        &self,
        plan: &engine::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<engine::read_plan::Outcome> {
        use engine::read_plan::Outcome as PlanOutcome;
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
        let bytes = match result {
            Some(PlanOutcome::Complete) => plan.destination_len(),
            Some(
                PlanOutcome::ControlChanged { completed_reads }
                | PlanOutcome::ReadFailed { completed_reads },
            ) => plan.reads()[..completed_reads]
                .iter()
                .map(|read| read.len)
                .sum(),
            None => 0,
        };
        self.bytes.set(self.bytes.get() + bytes as u64);
        result
    }
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.memory.copy(address, output);
        if ok {
            self.bytes.set(self.bytes.get() + output.len() as u64);
        }
        ok
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.memory.copy_scattered(addresses, outputs);
        self.bytes.set(
            self.bytes.get()
                + outputs[..copied]
                    .iter()
                    .map(|output| output.len() as u64)
                    .sum::<u64>(),
        );
        copied
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.memory.copy_scattered_prefetched(addresses, outputs);
        self.bytes.set(
            self.bytes.get()
                + outputs[..copied]
                    .iter()
                    .map(|output| output.len() as u64)
                    .sum::<u64>(),
        );
        copied
    }
    fn copy_records(
        &self,
        sources: engine::memory::RecordSources<'_>,
        layout: &engine::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let copied = self.memory.copy_records(sources, layout, destination);
        self.bytes
            .set(self.bytes.get() + layout.completed_bytes(copied) as u64);
        copied
    }
}

#[cfg(test)]
mod tests;
