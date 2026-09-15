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
}
impl UnitInputs {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            seeds: Vec::with_capacity(capacity),
            outcome: Outcome::Unavailable {
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
    #[cfg(test)]
    pub(crate) fn fixture(seeds: Vec<carmy::DivisionSeed>, outcome: Outcome) -> Self {
        Self { seeds, outcome }
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
    core_masks: Vec<u64>,
    union_masks: Vec<u64>,
    changed_masks: Vec<u64>,
    rows: Vec<carmy::Row>,
    group_kinds: Vec<u64>,
    sources: Vec<DivisionSource>,
    headers: Vec<[u8; 16]>,
    country_roots: Vec<(u32, usize, u32, u32)>,
    roster_slots: Vec<Entry>,
    census_result: Outcome,
    row_error: Option<&'static str>,
    core_root_error: Option<&'static str>,
    required: usize,
    pub overflowed: bool,
    bytes: u64,
    roster_headers: usize,
}
impl Default for Census {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            unit_inputs: None,
            core_masks: Vec::new(),
            union_masks: Vec::new(),
            changed_masks: Vec::new(),
            rows: Vec::new(),
            group_kinds: Vec::new(),
            sources: Vec::new(),
            headers: Vec::new(),
            country_roots: Vec::new(),
            roster_slots: Vec::new(),
            census_result: Outcome::Unavailable {
                reason: "division census not captured",
            },
            row_error: None,
            core_root_error: None,
            required: 0,
            overflowed: false,
            bytes: 0,
            roster_headers: 0,
        }
    }
}
impl Census {
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(8192),
            unit_inputs: Some(Arc::new(UnitInputs::new(8192))),
            core_masks: Vec::with_capacity(8192),
            union_masks: Vec::with_capacity(8192),
            changed_masks: Vec::with_capacity(8192),
            rows: Vec::with_capacity(8192),
            group_kinds: Vec::with_capacity(8192),
            sources: Vec::with_capacity(8192),
            headers: Vec::with_capacity(1024),
            country_roots: Vec::with_capacity(1024),
            roster_slots: Vec::with_capacity(16384),
            ..Self::default()
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        if let Some(shared) = &mut self.unit_inputs {
            let inputs =
                Arc::get_mut(shared).expect("prior division input leases released before reuse");
            inputs.seeds.clear();
            inputs.outcome = Outcome::Unavailable {
                reason: "division census not captured",
            };
        }
        self.core_masks.clear();
        self.union_masks.clear();
        self.changed_masks.clear();
        self.rows.clear();
        self.group_kinds.clear();
        self.sources.clear();
        self.headers.clear();
        self.country_roots.clear();
        self.roster_slots.clear();
        self.census_result = Outcome::Unavailable {
            reason: "division census not captured",
        };
        self.row_error = None;
        self.core_root_error = None;
        self.required = 0;
        self.overflowed = false;
        self.bytes = 0;
        self.roster_headers = 0;
    }
    pub fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.entries)
            + self
                .unit_inputs
                .as_ref()
                .map_or(0, |inputs| crate::raw_memory::vector_bytes(&inputs.seeds))
            + crate::raw_memory::vector_bytes(&self.core_masks)
            + crate::raw_memory::vector_bytes(&self.union_masks)
            + crate::raw_memory::vector_bytes(&self.changed_masks)
            + crate::raw_memory::vector_bytes(&self.rows)
            + crate::raw_memory::vector_bytes(&self.sources)
            + crate::raw_memory::vector_bytes(&self.group_kinds)
            + crate::raw_memory::vector_bytes(&self.headers)
            + crate::raw_memory::vector_bytes(&self.country_roots)
            + crate::raw_memory::vector_bytes(&self.roster_slots)
    }
    pub fn payload_bytes(&self) -> u64 {
        self.bytes
    }
    pub fn initialized(&self) -> bool {
        self.entries.capacity() != 0
    }
    pub(crate) fn unit_inputs(&self) -> Option<&Arc<UnitInputs>> {
        self.unit_inputs.as_ref()
    }
    pub(crate) fn unit_inputs_equal(&self, other: &Self) -> bool {
        self.intel_equal(other)
    }
    /// The callback invokes this once after every shared-input consumer joins.
    /// Only fields demanded by those consumers are freshly checked. Core rows
    /// also reject changes in their overlapping fields, without adding a full
    /// hot core double read for fields no companion consumed.
    pub(crate) fn verify_unit_inputs(
        &mut self,
        memory: &impl Memory,
        consumers: &[&[u64]],
    ) -> &[u64] {
        self.union_masks.fill(0);
        self.changed_masks.fill(0);
        for masks in consumers {
            for (union, mask) in self.union_masks.iter_mut().zip(*masks) {
                *union |= mask;
            }
        }
        if let Some(inputs) = &self.unit_inputs {
            let counted = crate::division_detail::CountedMemory::new(memory);
            carmy::verify_division_seed_fields(
                &counted,
                inputs.seeds(),
                &self.union_masks,
                &mut self.changed_masks,
            );
            self.bytes = self.bytes.saturating_add(counted.bytes());
        }
        &self.changed_masks
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
            reserve(
                &mut Arc::get_mut(shared)
                    .expect("division input leases released before growth")
                    .seeds,
                target,
            );
        } else {
            self.unit_inputs = Some(Arc::new(UnitInputs::new(target)));
        }
        reserve(&mut self.core_masks, target);
        reserve(&mut self.union_masks, target);
        reserve(&mut self.changed_masks, target);
        reserve(&mut self.rows, target);
        reserve(&mut self.sources, target);
        reserve(&mut self.group_kinds, target);
        let headers = self.headers.capacity().saturating_mul(2).max(1024);
        let countries = self.country_roots.capacity().saturating_mul(2).max(1024);
        let slots = self.roster_slots.capacity().saturating_mul(2).max(16384);
        reserve(&mut self.headers, headers);
        reserve(&mut self.country_roots, countries);
        reserve(&mut self.roster_slots, slots);
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
    pub fn intel_sources(&self) -> engine::unit_viewer_entry::DivisionSources<'_> {
        let outcome = match self.census_result {
            Outcome::Rows { .. } => Outcome::Rows {
                len: self.roster_slots.len(),
            },
            other => other,
        };
        engine::unit_viewer_entry::DivisionSources {
            entries: &self.roster_slots,
            outcome,
        }
    }
    pub fn intel_equal(&self, other: &Self) -> bool {
        matches!(
            (self.census_result, other.census_result),
            (Outcome::Rows { .. }, Outcome::Rows { .. })
        ) && self.roster_slots == other.roster_slots
            && self.country_roots == other.country_roots
            && self.headers.get(..self.roster_headers) == other.headers.get(..other.roster_headers)
    }
    pub fn sources(&self, capacity: usize) -> (&[DivisionSource], Outcome) {
        let outcome = self.outcome(capacity);
        (&self.sources[..outcome.len()], outcome)
    }
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
        let len = self.entries.len().min(capacity);
        if self
            .changed_masks
            .iter()
            .zip(&self.core_masks)
            .take(len)
            .any(|(changed, consumed)| changed & consumed != 0)
        {
            return Outcome::Unavailable {
                reason: "division shared fields changed or unreadable",
            };
        }
        if self.rows.len() < len {
            return Outcome::Unavailable {
                reason: self.row_error.unwrap_or("division owned row not captured"),
            };
        }
        complete(len, self.entries.len() - len)
    }
    pub fn equal(&self, other: &Self, capacity: usize) -> bool {
        matches!(self.outcome(capacity), Outcome::Rows { .. }) && self.same(other, capacity)
    }
    pub fn same(&self, other: &Self, capacity: usize) -> bool {
        let (a, ao) = self.rows(capacity);
        let (b, bo) = other.rows(capacity);
        !matches!(ao, Outcome::Unavailable { .. })
            && ao == bo
            && a == b
            && self.entries == other.entries
            && self.headers == other.headers
            && self.country_roots == other.country_roots
            && self.roster_slots == other.roster_slots
            && self
                .unit_inputs
                .as_ref()
                .zip(other.unit_inputs.as_ref())
                .is_some_and(|(first, second)| {
                    first
                        .seeds
                        .iter()
                        .zip(&second.seeds)
                        .take(a.len())
                        .all(|(a, b)| carmy::division_seed_fields_equal(a, b, (1 << 11) - 1))
                })
            && self.group_kinds.get(..a.len()) == other.group_kinds.get(..b.len())
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize, capacity: usize) {
        self.clear();
        let counted = crate::division_detail::CountedMemory::new(memory);
        let result = self.capture_inner(&counted, base, state, capacity);
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
        }
    }
    fn capture_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        capacity: usize,
    ) -> Result<(), &'static str> {
        let countries = Countries {
            countries: self.array(
                memory,
                state,
                cgame_state::COUNTRIES,
                8,
                "country array unreadable or invalid",
            )?,
            links: self.array(
                memory,
                state,
                cgame_state::COUNTRY_LINKS,
                4,
                "country links unreadable or invalid",
            )?,
        };
        if countries.countries.count < 2 {
            return Err("country table has no country slots");
        }
        if countries.links.count == 0 {
            return Err("country links have no sentinel slot");
        }
        let mut found = false;
        let mut pointers = [0usize; BATCH];
        let mut pointer_bytes = [0u8; BATCH * 8];
        const POINTER: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        const POINTER_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&POINTER, 8).expect("valid division pointer layout");
        for slot in 1..countries.countries.count {
            let pointer = countries.pointer(memory, slot)?;
            if pointer == 0 {
                self.root((slot, 0, 0, 0))?;
                continue;
            }
            if read::<u64>(memory, pointer)
                != base.checked_add(ccountry::VTABLE_RVA).map(|p| p as u64)
            {
                return Err("country vtable mismatch");
            }
            let id = pointer
                .checked_add(ccountry::ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("country id unreadable")?;
            let canonical = countries.canonical(memory, id)?;
            if countries.canonical(memory, slot)? != canonical {
                return Err("country slot and object resolve to different countries");
            }
            if countries.pointer(memory, canonical)? != pointer {
                return Err("canonical country slot points to a different object");
            }
            self.root((slot, pointer, id, canonical))?;
            if slot != canonical {
                continue;
            }
            found = true;
            let roster = self.array(
                memory,
                pointer,
                ccountry::DIVISIONS,
                8,
                "division roster unreadable or invalid",
            )?;
            for first in (0..roster.count).step_by(BATCH) {
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
                self.bytes += (copied * 8) as u64;
                for i in 0..count {
                    if i >= copied {
                        return Err("division roster pointer unreadable");
                    }
                    let pointer = usize::try_from(u64::from_le_bytes(
                        pointer_bytes[i * 8..i * 8 + 8].try_into().unwrap(),
                    ))
                    .map_err(|_| "division roster pointer unreadable")?;
                    if self.roster_slots.len() == self.roster_slots.capacity() {
                        self.overflowed = true;
                        return Err("division owned roster capacity exhausted");
                    }
                    self.roster_slots.push(Entry { pointer, canonical });
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
        if self.overflowed {
            return Err("division owned census capacity exhausted");
        }
        self.census_result = Outcome::Rows {
            len: self.entries.len(),
        };
        self.roster_headers = self.headers.len();
        let Some(shared) = &mut self.unit_inputs else {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        };
        let inputs =
            Arc::get_mut(shared).expect("division inputs uniquely owned during acquisition");
        if self.entries.len() > inputs.seeds.capacity()
            || self.entries.len() > self.core_masks.capacity()
            || self.entries.len() > self.union_masks.capacity()
            || self.entries.len() > self.changed_masks.capacity()
        {
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
        self.core_masks.resize(self.entries.len(), 0);
        self.union_masks.resize(self.entries.len(), 0);
        self.changed_masks.resize(self.entries.len(), 0);
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
        if take > self.rows.capacity()
            || take > self.sources.capacity()
            || take > self.group_kinds.capacity()
        {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        }
        let inputs = self.unit_inputs.as_ref().unwrap();
        for (index, (entry, seed)) in self
            .entries
            .iter()
            .zip(&inputs.seeds)
            .take(take)
            .enumerate()
        {
            let (row, group_kind) = project(
                memory,
                base,
                &countries,
                provinces,
                *entry,
                seed,
                &mut self.core_masks[index],
            )?;
            self.sources.push(DivisionSource {
                pointer: entry.pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            });
            self.rows.push(row);
            self.group_kinds.push(group_kind);
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
        if self.headers.len() == self.headers.capacity() {
            self.overflowed = true;
            return Err("division owned header capacity exhausted");
        }
        self.headers.push(bytes);
        self.bytes += 16;
        Array::from_header(bytes, stride).ok_or(reason)
    }
    fn root(&mut self, root: (u32, usize, u32, u32)) -> Result<(), &'static str> {
        if self.country_roots.len() == self.country_roots.capacity() {
            self.overflowed = true;
            return Err("division owned country capacity exhausted");
        }
        self.country_roots.push(root);
        Ok(())
    }
}

struct Countries {
    countries: Array,
    links: Array,
}
impl Countries {
    fn pointer(&self, memory: &impl Memory, slot: u32) -> Result<usize, &'static str> {
        self.countries
            .element(slot, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("country pointer unreadable")
    }
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

fn project(
    memory: &impl Memory,
    base: usize,
    countries: &Countries,
    provinces: Array,
    entry: Entry,
    scalar: &carmy::DivisionSeed,
    consumed: &mut u64,
) -> Result<(carmy::Row, u64), &'static str> {
    let primary = base
        .checked_add(carmy::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let secondary = base
        .checked_add(carmy::SECONDARY_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if consume(
        scalar.primary_vtable,
        consumed,
        carmy::DivisionSeed::PRIMARY_VTABLE,
        "division primary vtable mismatch",
    )? != primary
    {
        return Err("division primary vtable mismatch");
    }
    if consume(
        scalar.secondary_vtable,
        consumed,
        carmy::DivisionSeed::SECONDARY_VTABLE,
        "division secondary vtable mismatch",
    )? != secondary
    {
        return Err("division secondary vtable mismatch");
    }
    let id = consume(
        scalar.handle,
        consumed,
        carmy::DivisionSeed::HANDLE,
        "division handle unreadable",
    )?;
    if id == 0 {
        return Err("non-null division has null handle");
    }
    let roster_owner = consume(
        scalar.roster_owner,
        consumed,
        carmy::DivisionSeed::ROSTER_OWNER,
        "division roster owner unreadable",
    )?;
    if countries.canonical(memory, roster_owner)? != entry.canonical {
        return Err("division owner differs from roster country");
    }
    let logical_owner = consume(
        scalar.logical_owner,
        consumed,
        carmy::DivisionSeed::LOGICAL_OWNER,
        "division logical owner unreadable",
    )?;
    let expeditionary_owner = consume(
        scalar.expeditionary_owner,
        consumed,
        carmy::DivisionSeed::EXPEDITIONARY_OWNER,
        "division expeditionary owner unreadable",
    )?;
    let location = consume(
        scalar.location,
        consumed,
        carmy::DivisionSeed::LOCATION,
        "division location pointer unreadable",
    )?;
    let province = if location == 0 {
        0
    } else {
        if read::<u64>(memory, location)
            != base.checked_add(cprovince::VTABLE_RVA).map(|p| p as u64)
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
        let registered = provinces
            .element(id, 8)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("division province slot unreadable or out of range")?;
        if registered != location as u64 {
            return Err("division province differs from registered slot");
        }
        id
    };
    let template_pointer = consume(
        scalar.standard_template,
        consumed,
        carmy::DivisionSeed::STANDARD_TEMPLATE,
        "division template pointer unreadable",
    )?;
    let template = if template_pointer == 0 {
        0
    } else {
        if read::<u64>(memory, template_pointer)
            != base
                .checked_add(creferenced_division_template::VTABLE_RVA)
                .map(|p| p as u64)
        {
            return Err("division template vtable mismatch");
        }
        let handle = template_pointer
            .checked_add(creferenced_division_template::TYPE)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("division template handle unreadable")?;
        if handle == 0 {
            return Err("non-null division template has null handle");
        }
        handle
    };
    let hit_points = consume(
        scalar.strength,
        consumed,
        carmy::DivisionSeed::STRENGTH,
        "division hit points unreadable",
    )?;
    let organisation = consume(
        scalar.organisation,
        consumed,
        carmy::DivisionSeed::ORGANISATION,
        "division organisation unreadable",
    )?;
    let group = consume(
        scalar.orders_group,
        consumed,
        carmy::DivisionSeed::ORDERS_GROUP,
        "division orders group pointer unreadable",
    )?;
    let (orders_group, group_kind) = if group == 0 {
        (0, 0)
    } else {
        let identity =
            read::<[u8; 16]>(memory, group).ok_or("division orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = base
            .checked_add(engine::corders_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let army = base
            .checked_add(engine::carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if vtable != normal && vtable != army {
            return Err("division orders group vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != engine::corders_group::REFERENCE_TYPE {
            return Err("division orders group reference type mismatch");
        }
        (handle, vtable)
    };
    Ok((
        carmy::Row {
            id,
            roster_owner,
            logical_owner,
            expeditionary_owner,
            province,
            template,
            hit_points,
            organisation,
            orders_group,
        },
        group_kind,
    ))
}

fn consume<T>(
    field: Option<T>,
    consumed: &mut u64,
    mask: u64,
    reason: &'static str,
) -> Result<T, &'static str> {
    *consumed |= mask;
    field.ok_or(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::divisions_test_kit as kit;

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
    fn shared_verification_is_field_and_consumer_capacity_scoped() {
        let bytes = kit::fixture();
        let mut changed = bytes.clone();
        let mut census = Census::new();
        census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        changed[kit::A1 + engine::cunit::CLASS..kit::A1 + engine::cunit::CLASS + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        let masks = [carmy::DivisionSeed::CLASS, 0, 0];
        let differences = census.verify_unit_inputs(&kit::memory(&changed), &[&masks]);
        assert_eq!(differences[0], carmy::DivisionSeed::CLASS);
        assert_eq!(census.rows(64).1, Outcome::Rows { len: 3 });

        // A later source's shared location fault affects consumers that used
        // that source, while an independently bounded core prefix stays valid.
        let third = census.unit_inputs().unwrap().seeds()[2].pointer;
        changed[third + engine::cunit::LOCATION..third + engine::cunit::LOCATION + 8]
            .copy_from_slice(&0u64.to_le_bytes());
        let masks = [0, 0, carmy::DivisionSeed::LOCATION];
        let differences = census.verify_unit_inputs(&kit::memory(&changed), &[&masks]);
        assert_eq!(differences[2], carmy::DivisionSeed::LOCATION);
        assert_eq!(
            census.rows(1).1,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        assert_eq!(
            census.rows(64).1,
            Outcome::Unavailable {
                reason: "division shared fields changed or unreadable"
            }
        );
    }

    #[test]
    fn unrelated_companion_fields_do_not_change_core_cold_witness() {
        let bytes = kit::fixture();
        let mut changed = bytes.clone();
        changed[kit::A1 + engine::cunit::CLASS..kit::A1 + engine::cunit::CLASS + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        let mut first = Census::new();
        let mut second = Census::new();
        first.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        second.capture(&kit::memory(&changed), kit::BASE, kit::GAME_STATE, 64);
        assert!(first.equal(&second, 64));
        assert!(first.unit_inputs_equal(&second));
        assert!(!carmy::division_seed_fields_equal(
            &first.unit_inputs().unwrap().seeds()[0],
            &second.unit_inputs().unwrap().seeds()[0],
            carmy::DivisionSeed::CLASS,
        ));
    }
}
