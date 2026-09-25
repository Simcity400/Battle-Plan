//! One owned division census and narrowly gathered native core fields.
//! The census survives a private core-field failure so other consumers can
//! authenticate their own inputs without walking the country rosters again.

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
            seeds: Vec::with_capacity(capacity),
            outcome: Outcome::Unavailable {
                reason: "division census not captured",
            },
            roster_slots: Vec::with_capacity(capacity.saturating_mul(2)),
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
}
impl Default for Census {
    fn default() -> Self {
        Self {
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
        }
    }
}
impl Census {
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(8192),
            unit_inputs: Some(Arc::new(UnitInputs::new(8192))),
            rows: Vec::with_capacity(8192),
            sources: Vec::with_capacity(8192),
            ..Self::default()
        }
    }
    pub fn clear(&mut self) {
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
    }
    pub fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.entries)
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
    pub fn initialized(&self) -> bool {
        self.entries.capacity() != 0
    }
    pub(crate) fn source_capacity(&self) -> usize {
        self.sources.capacity()
    }
    pub(crate) fn unit_inputs(&self) -> Option<&Arc<UnitInputs>> {
        self.unit_inputs.as_ref()
    }
    pub fn grow(&mut self) {
        let target = self
            .required
            .max(self.entries.capacity().saturating_mul(2))
            .max(8192);
        fn reserve<T>(v: &mut Vec<T>, target: usize) {
            if v.capacity() < target {
                v.reserve_exact(target.saturating_sub(v.len()));
            }
        }
        reserve(&mut self.entries, target);
        if let Some(shared) = &mut self.unit_inputs {
            let inputs =
                Arc::get_mut(shared).expect("division input leases released before growth");
            reserve(&mut inputs.seeds, target);
            // The overflowed census kept counting every remaining roster, so
            // one step reaches the observed requirement.
            let slots = self
                .required_slots
                .max(inputs.roster_slots.capacity().saturating_mul(2))
                .max(16384);
            reserve(&mut inputs.roster_slots, slots);
        } else {
            self.unit_inputs = Some(Arc::new(UnitInputs::new(target)));
        }
        reserve(&mut self.rows, target);
        reserve(&mut self.sources, target);
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
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize, capacity: usize) {
        self.capture_inputs(memory, base, state);
        self.capture_core(memory, capacity);
    }
    /// Publish the immutable native input packet before acquiring private core
    /// dependencies. The owning callback may lease it to other jobs as soon as
    /// this method returns; those jobs must finish before `clear` or `grow`.
    pub(crate) fn capture_inputs(&mut self, memory: &impl Memory, base: usize, state: usize) {
        self.clear();
        let counted = crate::division_detail::CountedMemory::new(memory);
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
        let countries = Countries { countries, links };
        let mut found = false;
        // Set once the owned roster storage is exhausted: every later roster
        // is still visited, but only its native count is read, so the growth
        // demand covers the whole census in one step.
        let mut counting = false;
        let mut pointers = [0usize; BATCH];
        let mut pointer_bytes = [0u8; BATCH * 8];
        const POINTER: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        const POINTER_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&POINTER, 8).expect("valid division pointer layout");
        // Native country loops (CGameState::DailyUpdate 0x1401d4c85 and
        // 0x1401d5567, hourly 0x1401d81ab) dereference every slot without a
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
            found = true;
            let roster = self.array(
                memory,
                pointer,
                ccountry::DIVISIONS,
                8,
                "division roster unreadable or invalid",
            )?;
            self.required_slots = self.required_slots.saturating_add(roster.count as usize);
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
        if self.entries.len() > inputs.seeds.capacity() {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        }
        for entry in &self.entries {
            inputs.seeds.push(carmy::DivisionSeed {
                pointer: entry.pointer,
                ..Default::default()
            });
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
        let take = self.entries.len().min(capacity);
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
mod tests {
    use super::*;
    use crate::sampling::divisions_test_kit as kit;
    use std::cell::RefCell;

    struct Observed<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
        fault_at: Option<usize>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            if self.fault_at == Some(ordinal)
                || self.fault.is_some_and(|fault| {
                    address <= fault && address.saturating_add(output.len()) > fault
                })
            {
                return false;
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }

    /// Synthetic world of `countries` canonical countries, each with `per`
    /// division slots pointing at one shared army object. Slot 0 is the
    /// reserved sentinel; links are identity.
    mod many {
        use super::*;
        pub(super) const GAME_STATE: usize = 0x1000;
        pub(super) const CTAB: usize = 0x10000;
        pub(super) const LTAB: usize = 0x30000;
        pub(super) const COUNTRY0: usize = 0x40000;
        pub(super) const COUNTRY_STRIDE: usize = 0x300;
        pub(super) const ROSTERS: usize = 0x400000;
        pub(super) const ARMY: usize = 0x430000;

        fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
            bytes[at..at + value.len()].copy_from_slice(value);
        }
        fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
            put(bytes, at, &(data as u64).to_le_bytes());
            put(bytes, at + 8, &count.to_le_bytes());
            put(bytes, at + 12, &count.to_le_bytes());
        }
        pub(super) fn country(slot: u32) -> usize {
            COUNTRY0 + (slot as usize - 1) * COUNTRY_STRIDE
        }
        pub(super) fn roster(slot: u32, per: u32) -> usize {
            ROSTERS + (slot as usize - 1) * per as usize * 8
        }
        pub(super) fn world(countries: u32, per: u32) -> Vec<u8> {
            assert!(roster(countries + 1, per) <= ARMY);
            let mut bytes = vec![0; 0x480000];
            header(
                &mut bytes,
                GAME_STATE + cgame_state::COUNTRIES,
                CTAB,
                countries + 1,
            );
            header(
                &mut bytes,
                GAME_STATE + cgame_state::COUNTRY_LINKS,
                LTAB,
                countries + 1,
            );
            for slot in 1..=countries {
                let object = country(slot);
                put(
                    &mut bytes,
                    CTAB + slot as usize * 8,
                    &(object as u64).to_le_bytes(),
                );
                put(&mut bytes, LTAB + slot as usize * 4, &slot.to_le_bytes());
                put(
                    &mut bytes,
                    object,
                    &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
                );
                put(&mut bytes, object + ccountry::ID, &slot.to_le_bytes());
                header(
                    &mut bytes,
                    object + ccountry::DIVISIONS,
                    roster(slot, per),
                    per,
                );
                for index in 0..per as usize {
                    put(
                        &mut bytes,
                        roster(slot, per) + index * 8,
                        &(ARMY as u64).to_le_bytes(),
                    );
                }
            }
            put(&mut bytes, ARMY, &(carmy::VTABLE_RVA as u64).to_le_bytes());
            put(
                &mut bytes,
                ARMY + carmy::SECONDARY_BASE,
                &(carmy::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
            );
            bytes
        }
    }

    #[test]
    fn roster_overflow_counts_every_country_and_grows_in_one_step() {
        const COUNTRIES: u32 = 5000;
        const PER: u32 = 4;
        let total = (COUNTRIES * PER) as usize;
        let bytes = many::world(COUNTRIES, PER);
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: None,
            fault_at: None,
        };
        let mut census = Census::new();
        let roster_capacity = census.unit_inputs().unwrap().roster_slots.capacity();
        assert!(total > roster_capacity && total > census.entries.capacity());
        census.capture_inputs(&memory, 0, many::GAME_STATE);
        assert!(census.overflowed);
        assert_eq!(
            census.unit_inputs().unwrap().intel_sources().outcome,
            Outcome::Unavailable {
                reason: "division owned roster capacity exhausted"
            }
        );
        // Every roster count was read, including those after the miss...
        assert_eq!(census.required_slots, total);
        assert_eq!(census.required, total);
        // ...but no pointer page after the one that exhausted the storage.
        let last = many::roster(COUNTRIES, PER);
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| { address == many::country(COUNTRIES) + ccountry::DIVISIONS })
        );
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| (last..last + PER as usize * 8).contains(&address))
        );

        census.grow();
        census.clear();
        census.capture_inputs(&memory, 0, many::GAME_STATE);
        assert!(!census.overflowed);
        let inputs = census.unit_inputs().unwrap();
        assert_eq!(inputs.outcome(), Outcome::Rows { len: total });
        assert_eq!(inputs.seeds().len(), total);
        let intel = inputs.intel_sources();
        assert_eq!(intel.outcome, Outcome::Rows { len: total });
        for (index, entry) in intel.entries.iter().enumerate() {
            assert_eq!(entry.pointer, many::ARMY);
            assert_eq!(entry.canonical, index as u32 / PER + 1);
        }
    }

    #[test]
    fn entry_overflow_alone_also_grows_to_the_exact_count_in_one_step() {
        // 9,000 non-null entries exceed the 8,192 entry storage while the
        // 16,384 roster slots still fit.
        const COUNTRIES: u32 = 4500;
        const PER: u32 = 2;
        let total = (COUNTRIES * PER) as usize;
        let bytes = many::world(COUNTRIES, PER);
        let memory = kit::memory(&bytes);
        let mut census = Census::new();
        census.capture_inputs(&memory, 0, many::GAME_STATE);
        assert!(census.overflowed);
        assert_eq!((census.required, census.required_slots), (total, total));
        census.grow();
        census.clear();
        census.capture_inputs(&memory, 0, many::GAME_STATE);
        assert_eq!(
            census.unit_inputs().unwrap().outcome(),
            Outcome::Rows { len: total }
        );
    }

    #[test]
    fn census_reuses_the_callback_country_roster_and_matches_the_native_checks() {
        use engine::country_memory::{Cache, SeedStatus};
        const COUNTRIES: u32 = 5000;
        let bytes = many::world(COUNTRIES, 1);
        let plain = kit::memory(&bytes);
        let mut direct = Census::new();
        direct.capture_inputs(&plain, 0, many::GAME_STATE);
        let direct_entries = direct
            .unit_inputs()
            .unwrap()
            .intel_sources()
            .entries
            .to_vec();
        assert_eq!(direct_entries.len(), COUNTRIES as usize);

        let mut owner = Arc::new(Cache::with_capacity(1 << 17));
        let snapshot = Cache::prepare_shared(&mut owner, &plain, 0, many::GAME_STATE).unwrap();
        assert_eq!(snapshot.view(&plain).seed_status(), SeedStatus::Ready);
        let observed = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: None,
            fault_at: None,
        };
        let view = snapshot.roster_view(&observed);
        let mut reused = Census::new();
        reused.capture_inputs(&view, 0, many::GAME_STATE);
        assert_eq!(
            reused.unit_inputs().unwrap().intel_sources().entries,
            direct_entries
        );
        assert_eq!(
            reused.unit_inputs().unwrap().outcome(),
            direct.unit_inputs().unwrap().outcome()
        );
        // No country-table slot, link or country identity is read again:
        // only each roster header, its pointer page and the division seeds.
        let reads = observed.reads.borrow();
        assert!(!reads.iter().any(|&(address, _)| {
            (many::CTAB..many::LTAB + (COUNTRIES as usize + 1) * 4).contains(&address)
                || (address >= many::COUNTRY0
                    && address < many::country(COUNTRIES + 1)
                    && (address - many::COUNTRY0) % many::COUNTRY_STRIDE != ccountry::DIVISIONS)
        }));
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, _)| address >= many::COUNTRY0
                    && address < many::country(COUNTRIES + 1))
                .count(),
            COUNTRIES as usize
        );
    }

    #[test]
    fn null_slots_are_skipped_but_a_foreign_object_fails_the_census() {
        let mut bytes = many::world(4, 1);
        // A null slot has no native precedent, but skipping it is harmless.
        bytes[many::CTAB + 16..many::CTAB + 24].fill(0);
        let mut census = Census::new();
        census.capture_inputs(&kit::memory(&bytes), 0, many::GAME_STATE);
        assert_eq!(
            census.unit_inputs().unwrap().outcome(),
            Outcome::Rows { len: 3 }
        );
        // Every native country loop dereferences each slot as a CCountry.
        let foreign = many::country(3);
        bytes[foreign..foreign + 8].copy_from_slice(&1_u64.to_le_bytes());
        census.capture_inputs(&kit::memory(&bytes), 0, many::GAME_STATE);
        assert_eq!(
            census.unit_inputs().unwrap().outcome(),
            Outcome::Unavailable {
                reason: "country vtable mismatch"
            }
        );
    }

    #[test]
    fn input_stage_can_be_leased_before_core_without_repeating_the_census() {
        let bytes = kit::fixture();
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: None,
            fault_at: None,
        };
        let mut staged = Census::new();
        let allocated = staged.allocated_bytes();
        staged.capture_inputs(&memory, kit::BASE, kit::GAME_STATE);
        assert_eq!(
            staged.outcome(0),
            Outcome::Unavailable {
                reason: "division core not captured"
            }
        );
        assert_eq!(
            staged.outcome(64),
            Outcome::Unavailable {
                reason: "division core not captured"
            }
        );
        let lease = Arc::clone(staged.unit_inputs().unwrap());
        assert_eq!(lease.outcome(), Outcome::Rows { len: 3 });
        let boundary = memory.reads.borrow().len();
        assert!(!memory.reads.borrow().iter().any(|&(address, _)| {
            address == kit::GAME_STATE + cgame_state::PROVINCES
                || address == 0x40000
                || address == 0x41000
        }));
        let input_bytes = staged.payload_bytes();
        assert_eq!(
            input_bytes,
            memory
                .reads
                .borrow()
                .iter()
                .map(|&(_, len)| len as u64)
                .sum::<u64>()
        );

        staged.capture_core(&memory, 64);
        assert_eq!(staged.outcome(64), Outcome::Rows { len: 3 });
        assert!(Arc::ptr_eq(&lease, staged.unit_inputs().unwrap()));
        assert_eq!(staged.allocated_bytes(), allocated);
        assert!(staged.payload_bytes() > input_bytes);
        assert!(
            !memory.reads.borrow()[boundary..]
                .iter()
                .any(|&(address, _)| {
                    address == kit::GAME_STATE + cgame_state::COUNTRIES
                        || address == kit::GAME_STATE + cgame_state::COUNTRY_LINKS
                        || (0x23000..0x23010).contains(&address)
                        || address == 0x23100
                })
        );
        assert_eq!(
            staged.payload_bytes(),
            memory
                .reads
                .borrow()
                .iter()
                .map(|&(_, len)| len as u64)
                .sum::<u64>()
        );

        let mut combined = Census::new();
        combined.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(staged.rows(64), combined.rows(64));
        assert_eq!(staged.sources(64), combined.sources(64));
        assert_eq!(staged.payload_bytes(), combined.payload_bytes());
        let completed_reads = memory.reads.borrow().len();
        staged.capture_core(&memory, 64);
        assert_eq!(memory.reads.borrow().len(), completed_reads);
        drop(lease);
        staged.clear();
    }

    #[test]
    fn core_failure_is_not_retried_and_does_not_change_leased_intel_inputs() {
        let mut bytes = kit::fixture();
        // The full native roster retains counted nulls while typed unit seeds
        // omit them. UnitIntel must still observe the original null ordinal.
        bytes[0x23008..0x23010].fill(0);
        let mut census = Census::new();
        census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
        let lease = Arc::clone(census.unit_inputs().unwrap());
        assert_eq!(lease.outcome(), Outcome::Rows { len: 2 });
        assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
        assert_eq!(lease.intel_sources().entries[1].pointer, 0);
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: Some(kit::GAME_STATE + cgame_state::PROVINCES),
            fault_at: None,
        };
        census.capture_core(&memory, 0);
        assert_eq!(
            census.outcome(0),
            Outcome::Unavailable {
                reason: "province array unreadable or invalid"
            }
        );
        assert_eq!(memory.reads.borrow().len(), 1);
        census.capture_core(&memory, 64);
        assert_eq!(memory.reads.borrow().len(), 1);
        assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
        assert_eq!(
            census.intel_sources().entries,
            lease.intel_sources().entries
        );
        drop(lease);
        census.clear();
    }

    #[test]
    fn intel_pointer_outcome_survives_private_seed_capacity_failure() {
        let bytes = kit::fixture();
        let mut census = Census::new();
        Arc::get_mut(census.unit_inputs.as_mut().unwrap())
            .unwrap()
            .seeds = Vec::new();
        census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
        let lease = Arc::clone(census.unit_inputs().unwrap());
        assert_eq!(
            lease.outcome(),
            Outcome::Unavailable {
                reason: "division owned scalar capacity exhausted"
            }
        );
        assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
        assert_eq!(lease.intel_sources().entries.len(), 3);
        let bytes_before = census.payload_bytes();
        census.capture_core(&kit::memory(&bytes), 64);
        assert_eq!(census.payload_bytes(), bytes_before);
    }

    #[test]
    fn shared_seeds_keep_complete_census_beyond_core_capacity_and_private_faults() {
        let bytes = kit::fixture();
        let mut census = Census::new();
        census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 1);
        assert_eq!(
            census.rows(1).1,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        let inputs = Arc::clone(census.unit_inputs().unwrap());
        assert_eq!(inputs.outcome(), Outcome::Rows { len: 3 });
        assert_eq!(inputs.seeds().len(), 3);
        drop(inputs);

        struct Fault<'a>(&'a [u8]);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let fault = kit::A1 + carmy::STRENGTH;
                if address <= fault && address.saturating_add(output.len()) > fault {
                    return false;
                }
                kit::memory(self.0).copy(address, output)
            }
        }
        census.capture(&Fault(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(
            census.rows(64).1,
            Outcome::Unavailable {
                reason: "division hit points unreadable"
            }
        );
        let inputs = census.unit_inputs().unwrap();
        assert_eq!(inputs.outcome(), Outcome::Rows { len: 3 });
        assert_eq!(inputs.seeds()[0].strength, None);
        assert!(inputs.seeds()[0].organisation.is_some());
        assert!(inputs.seeds()[2].handle.is_some());
    }

    #[test]
    fn core_identity_reuse_keeps_serial_rows_and_first_source_failures() {
        let mut bytes = kit::fixture();
        // Ten entries cross two four-seed boundaries, retaining repeated child
        // pointers and the fixture's independent null location/template/group.
        let roster = kit::C0 + ccountry::DIVISIONS;
        for offset in [8, 12] {
            bytes[roster + offset..roster + offset + 4].copy_from_slice(&9u32.to_le_bytes());
        }
        for index in 0..9 {
            let pointer = [kit::A1, kit::A2][index % 2];
            bytes[0x23000 + index * 8..0x23008 + index * 8]
                .copy_from_slice(&(pointer as u64).to_le_bytes());
        }
        for capacity in [0, 1, 3, 4, 5, 8, 10] {
            // A failure at any original source must remain the first failure.
            // Successful repeated child reads are deliberately absent, so an
            // injected fault on only a removed occurrence is not a source fault.
            for fault in std::iter::once(None).chain(
                [
                    kit::GAME_STATE + cgame_state::PROVINCES,
                    kit::LTAB + 4,
                    kit::LTAB + 8,
                    kit::P1,
                    kit::P1 + cprovince::ID,
                    kit::PTAB + 8,
                    0x40000,
                    0x40000 + creferenced_division_template::TYPE,
                    0x40000 + creferenced_division_template::REVEAL_SKIP,
                    0x41000,
                ]
                .map(Some),
            ) {
                let mut census = Census::new();
                census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
                let allocated = census.allocated_bytes();
                let input_bytes = census.payload_bytes();
                let seeds = census.unit_inputs().unwrap().seeds().to_vec();
                assert_eq!(seeds.len(), 10);
                let roots = census.core_roots.unwrap();
                let make_memory = || Observed {
                    bytes: &bytes,
                    reads: RefCell::new(Vec::new()),
                    fault,
                    fault_at: None,
                };
                let serial = make_memory();
                let mut rows = Vec::new();
                // A fresh identity set per row forces the original scalar
                // child reads, including repeated addresses and their gates.
                let expected = (|| {
                    let provinces =
                        Array::read(&serial, kit::GAME_STATE + cgame_state::PROVINCES, 8)
                            .ok_or("province array unreadable or invalid")?;
                    for (index, seed) in seeds.iter().take(capacity).enumerate() {
                        let row = project(
                            &serial,
                            &roots.countries,
                            census.entries[index],
                            seed,
                            &mut CoreIdentities::new(roots.base, provinces),
                        )?;
                        rows.push(row);
                    }
                    Ok::<_, &'static str>(())
                })();

                let observed = make_memory();
                census.capture_core(&observed, capacity);
                let actual_reads = observed.reads.borrow();
                let serial_reads = serial.reads.borrow();
                let mut remaining = serial_reads.iter();
                for read in actual_reads.iter() {
                    assert!(remaining.any(|original| original == read));
                }
                if expected.is_err() {
                    assert_eq!(actual_reads.last(), serial_reads.last());
                } else if capacity > 0 {
                    // One root, one owner link per row, and exactly one complete
                    // authentication of each shared child (3 + 3 + 1 calls).
                    assert_eq!(actual_reads.len(), 1 + capacity + 7);
                }
                assert_eq!(census.rows, rows);
                assert_eq!(census.sources.len(), rows.len());
                assert_eq!(census.unit_inputs().unwrap().seeds(), seeds);
                assert_eq!(census.allocated_bytes(), allocated);
                assert_eq!(
                    census.payload_bytes() - input_bytes,
                    observed
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(address, len)| !fault.is_some_and(|fault| {
                            address <= fault && address.saturating_add(len) > fault
                        }))
                        .map(|&(_, len)| len as u64)
                        .sum::<u64>()
                );
                assert_eq!(
                    census.outcome(capacity),
                    match expected {
                        Ok(()) => complete(capacity, seeds.len() - capacity),
                        Err(reason) => Outcome::Unavailable { reason },
                    }
                );
            }
        }
    }

    #[test]
    fn core_identity_collisions_failures_and_next_walk_use_original_sources() {
        fn read_children(
            memory: &impl Memory,
            identities: &mut CoreIdentities,
            pointers: [usize; 3],
        ) -> Result<[u64; 3], &'static str> {
            Ok([
                identities.province(memory, pointers[0])? as u64,
                identities.template(memory, pointers[1])?.0,
                identities.group(memory, pointers[2])?,
            ])
        }
        let mut bytes = kit::fixture();
        let original = [kit::P1, 0x40000, 0x41000];
        let collision = |pointer, range: std::ops::Range<usize>| {
            range
                .step_by(16)
                .find(|&other| {
                    other != pointer && CoreIdentities::slot(other) == CoreIdentities::slot(pointer)
                })
                .unwrap()
        };
        let colliding = [
            collision(original[0], 0x60000..0x67000),
            collision(original[1], 0x56000..0x59000),
            collision(original[2], 0x5c000..0x5f000),
        ];
        let group_handle = (u32::MAX as u64) << 32 | engine::corders_group::REFERENCE_TYPE as u64;
        for (address, value) in [
            (colliding[0], cprovince::VTABLE_RVA as u64),
            (kit::PTAB + 16, colliding[0] as u64),
            (
                colliding[1],
                creferenced_division_template::VTABLE_RVA as u64,
            ),
            // The original core accepts any nonzero template handle. Do not
            // strengthen that contract by borrowing a stricter engine helper.
            (colliding[1] + creferenced_division_template::TYPE, u64::MAX),
            (colliding[2], engine::carmy_group::VTABLE_RVA as u64),
            (colliding[2] + 8, group_handle),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        let province_id = colliding[0] + cprovince::ID;
        bytes[province_id..province_id + 4].copy_from_slice(&2_u32.to_le_bytes());
        for offset in [8, 12] {
            let address = kit::GAME_STATE + cgame_state::PROVINCES + offset;
            bytes[address..address + 4].copy_from_slice(&3_u32.to_le_bytes());
        }
        let provinces = Array {
            data: kit::PTAB,
            count: 3,
        };
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: None,
            fault_at: None,
        };
        let mut identities = CoreIdentities::new(kit::BASE, provinces);
        assert_eq!(read_children(&memory, &mut identities, [0; 3]), Ok([0; 3]));
        assert!(memory.reads.borrow().is_empty());
        let expected_original = [
            1,
            0x3455_0000_0034,
            (57_u64 << 32) | engine::corders_group::REFERENCE_TYPE as u64,
        ];
        let expected_collision = [2, u64::MAX, group_handle];
        for (pointers, expected) in [
            (original, expected_original),
            (colliding, expected_collision),
            (original, expected_original),
        ] {
            assert_eq!(
                read_children(&memory, &mut identities, pointers),
                Ok(expected)
            );
        }
        assert_eq!(memory.reads.borrow().len(), 21);
        assert_eq!(
            read_children(&memory, &mut identities, original),
            Ok(expected_original)
        );
        assert_eq!(memory.reads.borrow().len(), 21);

        for fault in [
            colliding[0],
            province_id,
            kit::PTAB + 16,
            colliding[1],
            colliding[1] + 8,
            colliding[2],
        ] {
            let mut identities = CoreIdentities::new(kit::BASE, provinces);
            assert_eq!(
                read_children(&kit::memory(&bytes), &mut identities, original),
                Ok(expected_original)
            );
            let failing = Observed {
                bytes: &bytes,
                reads: RefCell::new(Vec::new()),
                fault: Some(fault),
                fault_at: None,
            };
            let expected = read_children(
                &failing,
                &mut CoreIdentities::new(kit::BASE, provinces),
                colliding,
            )
            .unwrap_err();
            let expected_reads = failing.reads.take();
            assert_eq!(
                read_children(&failing, &mut identities, colliding),
                Err(expected)
            );
            assert_eq!(*failing.reads.borrow(), expected_reads);
            assert_eq!(
                read_children(&kit::memory(&bytes), &mut identities, colliding),
                Ok(expected_collision)
            );
        }

        // Reusing Census storage begins a new core walk, even when all child
        // addresses survive. It must observe the new identities and backlink.
        let mut census = Census::new();
        census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 3);
        assert_eq!(census.rows[0].province, 1);
        assert_eq!(census.rows[0].template, expected_original[1]);
        assert_eq!(census.rows[0].orders_group, expected_original[2]);
        for (address, value) in [
            (kit::PTAB + 16, kit::P1 as u64),
            (original[1] + 8, u64::MAX),
            (original[2] + 8, group_handle),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[kit::P1 + cprovince::ID..kit::P1 + cprovince::ID + 4]
            .copy_from_slice(&2_u32.to_le_bytes());
        census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 3);
        assert_eq!(census.outcome(3), Outcome::Rows { len: 3 });
        assert_eq!(census.rows[0].province, expected_collision[0] as u32);
        assert_eq!(census.rows[0].template, expected_collision[1]);
        assert_eq!(census.rows[0].orders_group, expected_collision[2]);
    }
}
