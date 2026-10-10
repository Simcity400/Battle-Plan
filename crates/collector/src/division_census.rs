//! One owned division census and narrowly gathered native core fields.
//! The census survives a private core-field failure so other consumers can
//! authenticate their own inputs without walking the country rosters again.
//! DivisionCensus counts accumulate in the core projection, charged to Divisions.

use engine::{
    Memory, carmy, ccountry,
    cequipment_variant_pool::DivisionSource,
    cgame_state,
    cpdx_array::Array,
    cprovince, creferenced_division_template,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};

use std::sync::Arc;

const BATCH: usize = 64;
pub(crate) use engine::unit_viewer_entry::DivisionSource as Entry;

/// Immutable, callback-local native fields shared by independently bounded
/// consumers. The pointer census outcome excludes private core row failures.
#[derive(Debug)]
pub(crate) struct UnitInputs {
    seeds: Vec<carmy::DivisionSeed>,
    outcome: Outcome,
    roster_slots: Vec<Entry>,
    intel_outcome: Outcome,
}
impl UnitInputs {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            seeds: engine::allocation::owned_vec(engine::allocation::vec(capacity)),
            outcome: Outcome::Unavailable {
                reason: "division census not captured",
            },
            roster_slots: engine::allocation::owned_vec(engine::allocation::vec(
                capacity.saturating_mul(2),
            )),
            intel_outcome: Outcome::Unavailable {
                reason: "division census not captured",
            },
        }
    }
    pub(crate) fn seeds(&self) -> &[carmy::DivisionSeed] {
        &self.seeds
    }
    pub(crate) fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub(crate) fn intel_sources(&self) -> engine::unit_viewer_entry::DivisionSources<'_> {
        engine::unit_viewer_entry::DivisionSources {
            entries: &self.roster_slots,
            outcome: self.intel_outcome,
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(seeds: Vec<carmy::DivisionSeed>, outcome: Outcome) -> Self {
        let roster_slots = seeds
            .iter()
            .map(|seed| Entry {
                pointer: seed.pointer,
                canonical: seed.roster_owner.unwrap_or(0),
            })
            .collect();
        Self {
            seeds,
            outcome,
            roster_slots,
            intel_outcome: outcome,
        }
    }
    #[cfg(test)]
    pub(crate) fn empty_rows() -> Self {
        Self::fixture(Vec::new(), Outcome::Rows { len: 0 })
    }
}

#[derive(Debug)]
pub(crate) struct Census {
    counts: Vec<Option<replay::rows::DivisionCensusRow>>,
    required_counts: usize,
    counts_error: Option<&'static str>,
    entries: Vec<Entry>,
    unit_inputs: Option<Arc<UnitInputs>>,
    rows: Vec<carmy::Row>,
    sources: Vec<DivisionSource>,
    census_result: Outcome,
    row_error: Option<&'static str>,
    core_root_error: Option<&'static str>,
    /// Non-null division pointers, exact where read; after the roster storage
    /// is exhausted each remaining native roster count is an upper bound.
    required: usize,
    /// Every native roster slot of every canonical country, including nulls.
    required_slots: usize,
    pub overflowed: bool,
    bytes: u64,
    core_roots: Option<CoreRoots>,
    core_captured: bool,
    growing: bool,
}
impl Default for Census {
    fn default() -> Self {
        Self {
            counts: Vec::new(),
            required_counts: 0,
            counts_error: None,
            entries: Vec::new(),
            unit_inputs: None,
            rows: Vec::new(),
            sources: Vec::new(),
            census_result: Outcome::Unavailable {
                reason: "division census not captured",
            },
            row_error: None,
            core_root_error: None,
            required: 0,
            required_slots: 0,
            overflowed: false,
            bytes: 0,
            core_roots: None,
            core_captured: false,
            growing: false,
        }
    }
}
impl Census {
    pub(crate) fn new_worker() -> Self {
        Self {
            unit_inputs: Some(Arc::new(UnitInputs::new(0))),
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            counts: engine::allocation::owned_vec(engine::allocation::default_vec(
                crate::country_scale::census_rows(512),
            )),
            entries: engine::allocation::owned_vec(engine::allocation::vec(8192)),
            unit_inputs: Some(Arc::new(UnitInputs::new(8192))),
            rows: engine::allocation::owned_vec(engine::allocation::vec(8192)),
            sources: engine::allocation::owned_vec(engine::allocation::vec(8192)),
            ..Self::default()
        }
    }
    pub fn clear(&mut self) {
        self.counts.fill(None);
        self.required_counts = 0;
        self.counts_error = None;
        self.entries.clear();
        if let Some(shared) = &mut self.unit_inputs {
            let inputs =
                Arc::get_mut(shared).expect("prior division input leases released before reuse");
            inputs.seeds.clear();
            inputs.roster_slots.clear();
            inputs.outcome = Outcome::Unavailable {
                reason: "division census not captured",
            };
            inputs.intel_outcome = inputs.outcome;
        }
        self.rows.clear();
        self.sources.clear();
        self.census_result = Outcome::Unavailable {
            reason: "division census not captured",
        };
        self.row_error = None;
        self.core_root_error = None;
        self.required = 0;
        self.required_slots = 0;
        self.overflowed = false;
        self.bytes = 0;
        self.core_roots = None;
        self.core_captured = false;
        self.growing = false;
    }
    pub fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.counts)
            + crate::raw_memory::vector_bytes(&self.entries)
            + self.unit_inputs.as_ref().map_or(0, |inputs| {
                crate::raw_memory::vector_bytes(&inputs.seeds)
                    + crate::raw_memory::vector_bytes(&inputs.roster_slots)
            })
            + crate::raw_memory::vector_bytes(&self.rows)
            + crate::raw_memory::vector_bytes(&self.sources)
    }
    pub fn payload_bytes(&self) -> u64 {
        self.bytes
    }
    pub(crate) fn growing(&self) -> bool {
        self.growing
    }
    pub(crate) fn country_count(&self) -> usize {
        self.required_counts
    }
    pub(crate) fn counts_need_growth(&self) -> bool {
        self.required_counts > self.counts.len()
    }
    pub fn initialized(&self) -> bool {
        self.entries.capacity() != 0
    }
    pub(crate) fn source_requirement(&self) -> usize {
        self.required.max(self.sources.len())
    }
    pub(crate) fn unit_inputs(&self) -> Option<&Arc<UnitInputs>> {
        self.unit_inputs.as_ref()
    }
    pub fn grow(&mut self) {
        let counts = self
            .required_counts
            .max(crate::country_scale::census_rows(self.counts.len()));
        if !crate::table_capture::grow_vec(&mut self.counts, counts) {
            return;
        }
        let target = self.required;
        let additional = target.saturating_sub(self.entries.len());
        if engine::allocation::reserve_exact(&mut self.entries, additional).is_err() {
            return;
        }
        if let Some(shared) = &mut self.unit_inputs {
            let inputs =
                Arc::get_mut(shared).expect("division input leases released before growth");
            let additional = target.saturating_sub(inputs.seeds.len());
            if engine::allocation::reserve_exact(&mut inputs.seeds, additional).is_err() {
                return;
            }
            // The overflowed census kept counting every remaining roster, so
            // one step reaches the observed requirement.
            let slots = self.required_slots;
            let additional = slots.saturating_sub(inputs.roster_slots.len());
            if engine::allocation::reserve_exact(&mut inputs.roster_slots, additional).is_err() {
                return;
            }
        } else {
            self.unit_inputs = Some(Arc::new(UnitInputs::new(target)));
        }
        let additional = target.saturating_sub(self.rows.len());
        if engine::allocation::reserve_exact(&mut self.rows, additional).is_err() {
            return;
        }
        let additional = target.saturating_sub(self.sources.len());
        let _ = engine::allocation::reserve_exact(&mut self.sources, additional);
    }
    /// Grow retained storage only while the frame is off the callback.
    pub(crate) fn reserve_sources(&mut self, capacity: usize) {
        if self.entries.capacity() < capacity {
            self.required = self.required.max(capacity);
            self.grow();
        }
    }
    #[cfg(test)]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    #[cfg(test)]
    pub fn census_outcome(&self) -> Outcome {
        self.census_result
    }
    #[cfg(test)]
    pub fn intel_sources(&self) -> engine::unit_viewer_entry::DivisionSources<'_> {
        self.unit_inputs.as_ref().map_or(
            engine::unit_viewer_entry::DivisionSources {
                entries: &[],
                outcome: self.census_result,
            },
            |inputs| inputs.intel_sources(),
        )
    }
    pub fn sources(&self, capacity: usize) -> (&[DivisionSource], Outcome) {
        let outcome = self.outcome(capacity);
        (&self.sources[..outcome.len()], outcome)
    }
    #[cfg(test)]
    pub fn rows(&self, capacity: usize) -> (&[carmy::Row], Outcome) {
        let outcome = self.outcome(capacity);
        (&self.rows[..outcome.len()], outcome)
    }
    pub fn install_rows(&self, output: &mut [carmy::Row]) -> Outcome {
        let len = self.rows.len().min(output.len());
        output[..len].copy_from_slice(&self.rows[..len]);
        self.outcome(output.len())
    }
    pub fn outcome(&self, capacity: usize) -> Outcome {
        if self.overflowed {
            return Outcome::Unavailable {
                reason: "division owned census capacity exhausted",
            };
        }
        if let Outcome::Unavailable { reason } = self.census_result {
            return Outcome::Unavailable { reason };
        }
        if let Some(reason) = self.core_root_error {
            return Outcome::Unavailable { reason };
        }
        if !self.core_captured {
            return Outcome::Unavailable {
                reason: "division core not captured",
            };
        }
        let len = self.entries.len().min(capacity);
        if self.rows.len() < len {
            return Outcome::Unavailable {
                reason: self.row_error.unwrap_or("division owned row not captured"),
            };
        }
        complete(len, self.entries.len() - len)
    }
    #[cfg(test)]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize, capacity: usize) {
        self.capture_inputs(memory, base, state);
        self.capture_core(memory, capacity);
    }
    /// Publish the immutable native input packet before acquiring private core
    /// dependencies. The owning callback may lease it to other jobs as soon as
    /// this method returns; those jobs must finish before `clear` or `grow`.
    pub(crate) fn capture_inputs_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) {
        self.capture_inputs_mode(memory, base, state, true);
    }
    pub(crate) fn capture_inputs(&mut self, memory: &impl Memory, base: usize, state: usize) {
        self.capture_inputs_mode(memory, base, state, false);
    }
    fn capture_inputs_mode(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        growing: bool,
    ) {
        self.clear();
        let counted = crate::division_detail::CountedMemory::new(memory);
        self.growing = growing;
        if growing && self.unit_inputs.is_none() {
            self.unit_inputs = Some(Arc::new(UnitInputs::new(0)));
        }
        let result = self.capture_inputs_inner(&counted, base, state);
        self.bytes = counted.bytes();
        if let Err(reason) = result {
            if matches!(self.census_result, Outcome::Rows { .. }) {
                self.row_error = Some(reason);
            } else {
                self.census_result = Outcome::Unavailable { reason };
            }
            if let Some(inputs) = self.unit_inputs.as_mut().and_then(Arc::get_mut)
                && !matches!(inputs.outcome, Outcome::Rows { .. })
            {
                inputs.outcome = Outcome::Unavailable { reason };
            }
            if let Some(inputs) = self.unit_inputs.as_mut().and_then(Arc::get_mut)
                && !matches!(inputs.intel_outcome, Outcome::Rows { .. })
            {
                inputs.intel_outcome = Outcome::Unavailable { reason };
            }
        }
    }
    /// Consume the retained roots exactly once, without mutating the leased
    /// input packet or walking the country and division censuses again. A
    /// second call preserves the first result, including its failures.
    pub(crate) fn capture_core(&mut self, memory: &impl Memory, capacity: usize) {
        if self.core_captured {
            return;
        }
        let Some(roots) = self.core_roots.take() else {
            return;
        };
        let counted = crate::division_detail::CountedMemory::new(memory);
        let bytes_before = self.bytes;
        let result = self.capture_core_inner(&counted, roots, capacity);
        self.bytes = bytes_before.saturating_add(counted.bytes());
        if let Err(reason) = result {
            self.row_error = Some(reason);
        }
        self.core_captured = true;
    }
    fn capture_inputs_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        // The engine reader reuses this callback's complete country census
        // when the supplied memory carries it; otherwise it performs the same
        // native pointer, vtable, identity and link checks slot by slot.
        let reader = ccountry::Reader::new(memory, base, state)?;
        let [countries, links] = reader.arrays();
        self.required_counts = links.count as usize;
        // The hourly update uses the live country roster.
        // This worker keeps the original header even when a load outgrows hints.
        if self.growing {
            engine::walk::Buffer::Growing(&mut self.counts).ensure(self.required_counts)?;
        } else if self.counts.len() < self.required_counts {
            self.counts_error = Some("division coverage country capacity exhausted");
        }
        let countries = Countries { countries, links };
        let mut found = false;
        // Set once the owned roster storage is exhausted: every later roster
        // is still visited, but only its native count is read, so the growth
        // demand covers the whole census in one step.
        let mut counting = false;
        // Private scalar storage must not stop the independent native roster
        // used by UnitIntel. Finish that roster before refusing the seeds.
        let mut scalar_capacity_failed = false;
        let mut pointers = [0usize; BATCH];
        let mut pointer_bytes = [0u8; BATCH * 8];
        const POINTER: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        const POINTER_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&POINTER, 8).expect("valid division pointer layout");
        // The daily and hourly country loops dereference every slot without a
        // null test: a non-null slot with another vtable is never legitimate.
        for slot in 1..reader.count() {
            // Keep the census's established reason for an unreadable slot.
            let Some(country) = reader.at(slot).map_err(|reason| match reason {
                "country pointer unreadable or outside country table" => {
                    "country pointer unreadable"
                }
                other => other,
            })?
            else {
                continue;
            };
            let (pointer, canonical) = (country.pointer, country.canonical_slot);
            if let Some(count) = self.counts.get_mut(country.id as usize) {
                *count = Some(replay::rows::DivisionCensusRow {
                    country: country.id,
                    ..Default::default()
                });
            } else {
                self.counts_error = Some("division coverage country capacity exhausted");
            }
            found = true;
            let roster = self.array(
                memory,
                pointer,
                ccountry::DIVISIONS,
                8,
                "division roster unreadable or invalid",
            )?;
            self.required_slots = self.required_slots.saturating_add(roster.count as usize);
            // The division roster getter returns this original header.
            if self.growing {
                let inputs = self
                    .unit_inputs
                    .as_mut()
                    .and_then(Arc::get_mut)
                    .ok_or("division input packet still leased")?;
                engine::allocation::reserve(&mut inputs.roster_slots, roster.count as usize)
                    .map_err(|_| "division roster allocation failed")?;
                engine::allocation::reserve(&mut inputs.seeds, roster.count as usize)
                    .map_err(|_| "division seed allocation failed")?;
                engine::allocation::reserve(&mut self.entries, roster.count as usize)
                    .map_err(|_| "division census allocation failed")?;
            }
            if counting {
                self.required = self.required.saturating_add(roster.count as usize);
                continue;
            }
            'roster: for first in (0..roster.count).step_by(BATCH) {
                let count = ((roster.count - first) as usize).min(BATCH);
                for (i, p) in pointers.iter_mut().take(count).enumerate() {
                    *p = roster
                        .element(first + i as u32, 8)
                        .ok_or("division roster pointer unreadable")?;
                }
                let copied = memory.copy_records(
                    RecordSources::Pointers(&pointers[..count]),
                    &POINTER_LAYOUT,
                    &mut pointer_bytes,
                );
                if copied == 0 {
                    return Err("division roster pointer unreadable");
                }
                let Some(inputs) = self.unit_inputs.as_mut().and_then(Arc::get_mut) else {
                    self.overflowed = true;
                    return Err("division owned roster capacity exhausted");
                };
                for i in 0..count {
                    if i >= copied {
                        return Err("division roster pointer unreadable");
                    }
                    let pointer = usize::try_from(u64::from_le_bytes(
                        pointer_bytes[i * 8..i * 8 + 8].try_into().unwrap(),
                    ))
                    .map_err(|_| "division roster pointer unreadable")?;
                    if inputs.roster_slots.len() == inputs.roster_slots.capacity() {
                        // Count this and every later slot of the roster as
                        // an upper bound; no further pointer is copied.
                        let unread = roster.count as usize - (first as usize + i);
                        self.required = self.required.saturating_add(unread);
                        counting = true;
                        break 'roster;
                    }
                    inputs.roster_slots.push(Entry { pointer, canonical });
                    if pointer == 0 {
                        continue;
                    }
                    self.required += 1;
                    if self.entries.len() == self.entries.capacity() {
                        self.overflowed = true;
                    } else {
                        self.entries.push(Entry { pointer, canonical });
                        if inputs.seeds.len() == inputs.seeds.capacity() {
                            scalar_capacity_failed = true;
                        } else {
                            inputs.seeds.push(carmy::DivisionSeed {
                                pointer,
                                country_ordinal: Some(first + i as u32),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
        if counting {
            self.overflowed = true;
            return Err("division owned roster capacity exhausted");
        }
        if self.overflowed {
            return Err("division owned census capacity exhausted");
        }
        self.census_result = Outcome::Rows {
            len: self.entries.len(),
        };
        let Some(shared) = &mut self.unit_inputs else {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        };
        let inputs =
            Arc::get_mut(shared).expect("division inputs uniquely owned during acquisition");
        inputs.intel_outcome = Outcome::Rows {
            len: inputs.roster_slots.len(),
        };
        if scalar_capacity_failed || self.entries.len() > inputs.seeds.capacity() {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        }
        carmy::capture_division_seeds(memory, &mut inputs.seeds);
        for seed in &mut inputs.seeds {
            seed.division = seed.handle.unwrap_or(0);
        }
        inputs.outcome = self.census_result;
        self.core_roots = Some(CoreRoots {
            base,
            state,
            countries,
            found,
        });
        Ok(())
    }
    fn capture_core_inner(
        &mut self,
        memory: &impl Memory,
        roots: CoreRoots,
        capacity: usize,
    ) -> Result<(), &'static str> {
        let CoreRoots {
            base,
            state,
            countries,
            found,
        } = roots;
        if !found {
            self.core_root_error = Some("country table has no readable countries");
            return Ok(());
        }
        let provinces = match self.array(
            memory,
            state,
            cgame_state::PROVINCES,
            8,
            "province array unreadable or invalid",
        ) {
            Ok(provinces) if provinces.count >= 2 => provinces,
            Ok(_) => {
                self.core_root_error = Some("province table has no province slots");
                return Ok(());
            }
            Err(reason) => {
                self.core_root_error = Some(reason);
                return Ok(());
            }
        };
        let take = if self.growing {
            self.entries.len()
        } else {
            self.entries.len().min(capacity)
        };
        if self.growing {
            engine::allocation::reserve(&mut self.rows, take)
                .map_err(|_| "division row allocation failed")?;
            engine::allocation::reserve(&mut self.sources, take)
                .map_err(|_| "division source allocation failed")?;
        }
        if take > self.rows.capacity() || take > self.sources.capacity() {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        }
        let inputs = self.unit_inputs.as_ref().unwrap();
        let mut identities = CoreIdentities::new(base, provinces);
        for (index, (entry, seed)) in self
            .entries
            .iter()
            .zip(&inputs.seeds)
            .take(take)
            .enumerate()
        {
            // Only hint this pass's admitted seeds. Each four-row group is
            // hinted once before serial projection; no seed field is consumed
            // and no child value is copied ahead of its validation gates.
            const LOOKAHEAD: usize = 4;
            if index.is_multiple_of(LOOKAHEAD) {
                for pending in inputs
                    .seeds
                    .iter()
                    .skip(index)
                    .take((take - index).min(LOOKAHEAD))
                {
                    if let Some(location) = pending.location.filter(|&pointer| pointer != 0) {
                        engine::prefetch::span(location, 8);
                        if let Some(id) = location.checked_add(cprovince::ID) {
                            engine::prefetch::span(id, 4);
                        }
                    }
                    // Both identities occupy the exact vtable/handle pair.
                    for pointer in [pending.standard_template, pending.orders_group]
                        .into_iter()
                        .flatten()
                        .filter(|&pointer| pointer != 0)
                    {
                        engine::prefetch::span(pointer, 16);
                    }
                }
            }
            let row = project(memory, &countries, *entry, seed, &mut identities)?;
            Self::count_division(
                &mut self.counts,
                self.required_counts,
                &mut self.counts_error,
                &row,
            );
            self.sources.push(DivisionSource {
                pointer: entry.pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            });
            self.rows.push(row);
        }
        Ok(())
    }
    fn count_division(
        counts: &mut [Option<replay::rows::DivisionCensusRow>],
        country_count: usize,
        counts_error: &mut Option<&'static str>,
        row: &carmy::Row,
    ) {
        // Damage attribution chooses the expeditionary owner when signed-positive,
        // otherwise the roster owner. Count here, never walk twice.
        let owner = if (row.expeditionary_owner as i32) > 0 {
            row.expeditionary_owner
        } else {
            row.roster_owner
        };
        if owner as usize >= country_count {
            *counts_error = Some("division owner outside country links");
        } else if let Some(count) = counts.get_mut(owner as usize) {
            let count = count.get_or_insert_with(|| replay::rows::DivisionCensusRow {
                country: owner,
                ..Default::default()
            });
            let value = if row.orders_group == 0 {
                &mut count.ungrouped
            } else {
                &mut count.grouped
            };
            if let Some(next) = value.checked_add(1) {
                *value = next;
            } else {
                *counts_error = Some("division coverage count overflow");
            }
        } else {
            *counts_error = Some("division coverage country capacity exhausted");
        }
    }
    pub(crate) fn install_counts(&self, output: &mut [replay::rows::DivisionCensusRow]) -> Outcome {
        if let Some(reason) = self.counts_error {
            return Outcome::Unavailable { reason };
        }
        if !matches!(self.outcome(usize::MAX), Outcome::Rows { .. }) {
            return Outcome::Unavailable {
                reason: "division coverage census incomplete",
            };
        }
        let mut len = 0;
        let mut missing = 0;
        for count in self.counts.iter().flatten() {
            if let Some(row) = output.get_mut(len) {
                *row = count.clone();
                len += 1;
            } else {
                missing += 1;
            }
        }
        complete(len, missing)
    }
    fn array(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        offset: usize,
        stride: usize,
        reason: &'static str,
    ) -> Result<Array, &'static str> {
        let bytes = pointer
            .checked_add(offset)
            .and_then(|p| read::<[u8; 16]>(memory, p))
            .ok_or(reason)?;
        Array::from_header(bytes, stride).ok_or(reason)
    }
}

#[derive(Debug, Clone, Copy)]
struct CoreRoots {
    base: usize,
    state: usize,
    countries: Countries,
    found: bool,
}

#[derive(Debug, Clone, Copy)]
struct Countries {
    countries: Array,
    links: Array,
}
impl Countries {
    fn canonical(&self, memory: &impl Memory, raw: u32) -> Result<u32, &'static str> {
        if raw == 0 {
            return Err("country id is zero");
        }
        self.links
            .element(raw, 4)
            .and_then(|p| read::<u32>(memory, p))
            .filter(|&slot| slot > 0 && slot < self.countries.count)
            .ok_or("country link unreadable or outside country table")
    }
}

fn complete(len: usize, missing: usize) -> Outcome {
    if missing == 0 {
        Outcome::Rows { len }
    } else {
        match u32::try_from(missing) {
            Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
            Err(_) => Outcome::Unavailable {
                reason: "division unread row count overflow",
            },
        }
    }
}

const CORE_IDENTITY_SLOTS: usize = 256;

#[derive(Clone, Copy, Default)]
struct CoreIdentity {
    pointer: usize,
    value: u64,
}

/// Successful child identities from this core walk only. The owning callback
/// holds the game before its hourly update until every acquisition job joins;
/// these pointers and their province registry cannot change during this walk.
/// Null pointers are never inserted. A collision simply replaces a successful
/// entry after the unchanged native validation; no error or partial read is kept.
struct CoreIdentities {
    base: usize,
    provinces: Array,
    locations: [CoreIdentity; CORE_IDENTITY_SLOTS],
    templates: [CoreIdentity; CORE_IDENTITY_SLOTS],
    template_skips: [u8; CORE_IDENTITY_SLOTS],
    groups: [CoreIdentity; CORE_IDENTITY_SLOTS],
}

impl CoreIdentities {
    fn new(base: usize, provinces: Array) -> Self {
        Self {
            base,
            provinces,
            locations: [CoreIdentity::default(); CORE_IDENTITY_SLOTS],
            templates: [CoreIdentity::default(); CORE_IDENTITY_SLOTS],
            template_skips: [0; CORE_IDENTITY_SLOTS],
            groups: [CoreIdentity::default(); CORE_IDENTITY_SLOTS],
        }
    }

    fn slot(pointer: usize) -> usize {
        let hash = (pointer >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        (hash ^ (hash >> 32)) & (CORE_IDENTITY_SLOTS - 1)
    }

    fn province(&mut self, memory: &impl Memory, location: usize) -> Result<u32, &'static str> {
        if location == 0 {
            return Ok(0);
        }
        let slot = Self::slot(location);
        if self.locations[slot].pointer == location {
            return Ok(self.locations[slot].value as u32);
        }
        if read::<u64>(memory, location)
            != self
                .base
                .checked_add(cprovince::VTABLE_RVA)
                .map(|p| p as u64)
        {
            return Err("division province vtable mismatch");
        }
        let id = location
            .checked_add(cprovince::ID)
            .and_then(|p| read::<u32>(memory, p))
            .ok_or("division province id unreadable")?;
        if id == 0 {
            return Err("non-null division province has zero id");
        }
        let registered = self
            .provinces
            .element(id, 8)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("division province slot unreadable or out of range")?;
        if registered != location as u64 {
            return Err("division province differs from registered slot");
        }
        self.locations[slot] = CoreIdentity {
            pointer: location,
            value: id as u64,
        };
        Ok(id)
    }

    /// The template's handle and fog reveal-skip byte.
    fn template(
        &mut self,
        memory: &impl Memory,
        template_pointer: usize,
    ) -> Result<(u64, u8), &'static str> {
        if template_pointer == 0 {
            return Ok((0, 0));
        }
        let slot = Self::slot(template_pointer);
        if self.templates[slot].pointer == template_pointer {
            let value = self.templates[slot].value;
            return Ok((value, self.template_skips[slot]));
        }
        if read::<u64>(memory, template_pointer)
            != self
                .base
                .checked_add(creferenced_division_template::VTABLE_RVA)
                .map(|p| p as u64)
        {
            return Err("division template vtable mismatch");
        }
        let field = |offset| {
            template_pointer
                .checked_add(offset)
                .ok_or("division template address overflow")
        };
        let (mut handle, mut skip) = ([0_u8; 8], [0_u8; 1]);
        if memory.copy_scattered(
            [
                field(creferenced_division_template::TYPE)?,
                field(creferenced_division_template::REVEAL_SKIP)?,
            ],
            &mut [&mut handle, &mut skip],
        ) != 2
        {
            return Err("division template handle unreadable");
        }
        let handle = u64::from_le_bytes(handle);
        if handle == 0 {
            return Err("non-null division template has null handle");
        }
        self.templates[slot] = CoreIdentity {
            pointer: template_pointer,
            value: handle,
        };
        self.template_skips[slot] = skip[0];
        Ok((handle, skip[0]))
    }

    fn group(&mut self, memory: &impl Memory, group: usize) -> Result<u64, &'static str> {
        if group == 0 {
            return Ok(0);
        }
        let slot = Self::slot(group);
        if self.groups[slot].pointer == group {
            return Ok(self.groups[slot].value);
        }
        let identity =
            read::<[u8; 16]>(memory, group).ok_or("division orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = self
            .base
            .checked_add(engine::corders_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let army = self
            .base
            .checked_add(engine::carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if vtable != normal && vtable != army {
            return Err("division orders group vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != engine::corders_group::REFERENCE_TYPE {
            return Err("division orders group reference type mismatch");
        }
        self.groups[slot] = CoreIdentity {
            pointer: group,
            value: handle,
        };
        Ok(handle)
    }
}

fn project(
    memory: &impl Memory,
    countries: &Countries,
    entry: Entry,
    scalar: &carmy::DivisionSeed,
    identities: &mut CoreIdentities,
) -> Result<carmy::Row, &'static str> {
    let primary = identities
        .base
        .checked_add(carmy::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let secondary = identities
        .base
        .checked_add(carmy::SECONDARY_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if scalar
        .primary_vtable
        .ok_or("division primary vtable mismatch")?
        != primary
    {
        return Err("division primary vtable mismatch");
    }
    if scalar
        .secondary_vtable
        .ok_or("division secondary vtable mismatch")?
        != secondary
    {
        return Err("division secondary vtable mismatch");
    }
    let id = scalar.handle.ok_or("division handle unreadable")?;
    if id == 0 {
        return Err("non-null division has null handle");
    }
    let roster_owner = scalar
        .roster_owner
        .ok_or("division roster owner unreadable")?;
    if countries.canonical(memory, roster_owner)? != entry.canonical {
        return Err("division owner differs from roster country");
    }
    let logical_owner = scalar
        .logical_owner
        .ok_or("division logical owner unreadable")?;
    let expeditionary_owner = scalar
        .expeditionary_owner
        .ok_or("division expeditionary owner unreadable")?;
    let location = scalar
        .location
        .ok_or("division location pointer unreadable")?;
    let province = identities.province(memory, location)?;
    let template_pointer = scalar
        .standard_template
        .ok_or("division template pointer unreadable")?;
    let (template, reveal_skip) = identities.template(memory, template_pointer)?;
    let hit_points = scalar.strength.ok_or("division hit points unreadable")?;
    let organisation = scalar
        .organisation
        .ok_or("division organisation unreadable")?;
    let group = scalar
        .orders_group
        .ok_or("division orders group pointer unreadable")?;
    let orders_group = identities.group(memory, group)?;
    Ok(carmy::Row {
        id,
        roster_owner,
        logical_owner,
        expeditionary_owner,
        province,
        template,
        hit_points,
        organisation,
        orders_group,
        reveal_skip,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod coverage_tests;
