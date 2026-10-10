//! Prepared current-hour navy and theatre inputs for demanded moving owners.
//! The shared nearest-cache packet lives in country appearance; this helper
//! writes its loaned arrays directly and retains no duplicate cache payload.
//! Attach/live hints prepare the actual moving-owner closure; worker storage
//! grows from the original headers before the same sample is published.
use crate::table_capture::Arena;
use engine::{Memory, ccountry, country_naval_movement_inputs as native};
use replay::rows::{
    CountryNavalMovementInputs, NavalPathInputs, PhysicalNavyNearestCache, TheatreControllerAreas,
};

pub(crate) struct OwnerCensus<I> {
    pub owners: I,
    pub failure: Option<&'static str>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Capacities {
    pub owners: usize,
    pub owner_slots: usize,
    pub navies: usize,
    pub physical_slots: usize,
    pub foreign: usize,
    pub hubs: usize,
    pub provinces: usize,
    pub area_edges: usize,
    pub theatre_headers: usize,
    pub theatre_provinces: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SharedCapacities {
    pub nearest_rows: usize,
    pub nearest_bases: usize,
    pub reserved_provinces: usize,
    pub launch_bases: usize,
}

impl Capacities {
    /// Hint floors are observed for the actual moving-owner/physical closure,
    /// never a country-count times province-count payload estimate.
    pub(crate) fn raise(&mut self, required: Self) {
        macro_rules! raise { ($($field:ident),+) => { $(
            crate::country_scale::raise(&mut self.$field, required.$field);
        )+ }; }
        raise!(
            owners,
            owner_slots,
            navies,
            physical_slots,
            foreign,
            hubs,
            provinces,
            area_edges,
            theatre_headers,
            theatre_provinces
        );
    }
}

impl SharedCapacities {
    pub(crate) fn raise(&mut self, required: Self) {
        for (capacity, floor) in [
            (&mut self.nearest_rows, required.nearest_rows),
            (&mut self.nearest_bases, required.nearest_bases),
            (&mut self.reserved_provinces, required.reserved_provinces),
            (&mut self.launch_bases, required.launch_bases),
        ] {
            crate::country_scale::raise(capacity, floor);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Span {
    start: usize,
    len: usize,
}

#[derive(Default)]
struct OwnerRow {
    country: u32,
    navy_slot: u32,
    // Current-pass source only, cleared before capture returns.
    source: Option<ccountry::Country>,
    theatre: Option<Span>,
    null_controller_area: bool,
    theatre_reason: Option<&'static str>,
}

#[derive(Default)]
struct NavyRow {
    slot: u32,
    present: bool,
    demanded: bool,
    // Consume all controls and clear before the callback loan is returned.
    inputs: Option<native::NavyInputs>,
    identity_reason: Option<&'static str>,
    foreign: Option<Span>,
    hubs: Option<Span>,
    foreign_reason: Option<&'static str>,
    hubs_reason: Option<&'static str>,
}

#[derive(Default)]
struct NearestRow {
    slot: u32,
    present: bool,
    bases: Option<Span>,
    reason: Option<&'static str>,
}

/// The only holder of the reserved singleton's numerical payload. Capture
/// borrows this from country appearance after that pack's ordinary capture.
pub(crate) struct Shared {
    sample_hour: Option<i32>,
    nearest: Arena<NearestRow>,
    bases: Arena<Option<u32>>,
    reserved_provinces: Arena<u32>,
    nearest_reason: Option<&'static str>,
    reserved_reason: Option<&'static str>,
    reserved_demanded: bool,
    null_controller_area: bool,
    launch_bases: Arena<ccountry::launch_sources::LaunchNavalBase>,
    launch_scratch: Arena<ccountry::launch_sources::Occurrence>,
    launch_result: Result<ccountry::launch_sources::NavalProvider, &'static str>,
}

impl Shared {
    pub(crate) fn new(capacity: SharedCapacities) -> Self {
        Self {
            sample_hour: None,
            nearest: Arena::new(capacity.nearest_rows),
            bases: Arena::new(capacity.nearest_bases),
            reserved_provinces: Arena::new(capacity.reserved_provinces),
            nearest_reason: Some("naval movement sources not supplied"),
            reserved_reason: Some("naval movement sources not supplied"),
            reserved_demanded: false,
            null_controller_area: false,
            launch_bases: Arena::new(capacity.launch_bases),
            launch_scratch: Arena::new(capacity.launch_bases),
            launch_result: Err("reserved launch sources not supplied"),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.sample_hour = None;
        self.nearest.reset();
        self.bases.reset();
        self.reserved_provinces.reset();
        self.nearest_reason = Some("naval movement sources not supplied");
        self.reserved_reason = Some("naval movement sources not supplied");
        self.reserved_demanded = false;
        self.null_controller_area = false;
        self.launch_bases.reset();
        self.launch_scratch.reset();
        self.launch_result = Err("reserved launch sources not supplied");
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.nearest.allocated_bytes()
            + self.bases.allocated_bytes()
            + self.reserved_provinces.allocated_bytes()
            + self.launch_bases.allocated_bytes()
            + self.launch_scratch.allocated_bytes()
    }

    pub(crate) fn grow(&mut self, capacity: &mut SharedCapacities) {
        self.nearest.grow(&mut capacity.nearest_rows);
        self.bases.grow(&mut capacity.nearest_bases);
        self.reserved_provinces
            .grow(&mut capacity.reserved_provinces);
        self.launch_bases.grow(&mut capacity.launch_bases);
        self.launch_scratch.grow(&mut capacity.launch_bases);
    }

    pub(crate) fn required(&self) -> SharedCapacities {
        SharedCapacities {
            nearest_rows: self.nearest.required,
            nearest_bases: self.bases.required,
            reserved_provinces: self.reserved_provinces.required,
            launch_bases: self.launch_bases.required.max(self.launch_scratch.required),
        }
    }

    fn capacity(&self) -> SharedCapacities {
        SharedCapacities {
            nearest_rows: self.nearest.first.len(),
            nearest_bases: self.bases.first.len(),
            reserved_provinces: self.reserved_provinces.first.len(),
            launch_bases: self
                .launch_bases
                .first
                .len()
                .min(self.launch_scratch.first.len()),
        }
    }

    pub(crate) fn projection(&self) -> std::io::Result<Option<CountryNavalMovementInputs>> {
        let Some(sample_hour) = self.sample_hour else {
            return Ok(None);
        };
        let nearest_caches = if self.nearest_reason.is_none() {
            let mut rows = engine::allocation::vec(self.nearest.len)?;
            for row in &self.nearest.first[..self.nearest.len] {
                rows.push(PhysicalNavyNearestCache {
                    slot: row.slot,
                    present: row.present,
                    nearest_bases: row
                        .bases
                        .map(|span| copy_span(&self.bases, span))
                        .transpose()?,
                });
            }
            Some(rows)
        } else {
            None
        };
        let reserved_theatre = if self.reserved_reason.is_none() {
            Some(TheatreControllerAreas {
                provinces: engine::allocation::copy_vec(
                    &self.reserved_provinces.first[..self.reserved_provinces.len],
                )?,
                null_controller_area: self.null_controller_area,
            })
        } else {
            None
        };
        Ok(Some(CountryNavalMovementInputs {
            sample_hour,
            nearest_caches,
            reserved_theatre,
        }))
    }

    pub(crate) fn launch_sample_hour(&self) -> Option<i32> {
        self.sample_hour
    }

    pub(crate) fn projection_launch(
        &self,
        owned_states: Option<Vec<u32>>,
    ) -> std::io::Result<Option<replay::rows::ReservedLaunchSources>> {
        let Some(sample_hour) = self.sample_hour else {
            return Ok(None);
        };
        let naval_bases = match self.launch_result {
            Ok(ccountry::launch_sources::NavalProvider::NativeNull) => {
                replay::rows::LaunchNavalBases::NativeNull
            }
            Ok(ccountry::launch_sources::NavalProvider::Bases(len)) => {
                let values = self
                    .launch_bases
                    .first
                    .get(..self.launch_bases.len)
                    .and_then(|values| values.get(..len))
                    .ok_or_else(|| std::io::Error::other("reserved launch base arena invalid"))?;
                replay::rows::LaunchNavalBases::Bases(engine::allocation::collect_vec(
                    values.iter().map(|value| replay::rows::LaunchNavalBase {
                        base_province: value.base_province,
                        country: value.country,
                    }),
                )?)
            }
            Err(_) => replay::rows::LaunchNavalBases::Unavailable,
        };
        Ok(Some(replay::rows::ReservedLaunchSources {
            sample_hour,
            naval_bases,
            owned_states,
        }))
    }

    fn capture_launch(
        &mut self,
        memory: &impl Memory,
        base: usize,
        observed: &mut Result<native::NavyInputs, &'static str>,
    ) {
        let captured = match observed {
            Ok(inputs) => inputs.capture_launch_growing(
                memory,
                base,
                &mut self.launch_scratch.first,
                &mut self.launch_bases.first,
            ),
            Err(reason) => ccountry::launch_sources::NavalCapture {
                provider: Err(*reason),
                required: 0,
            },
        };
        self.launch_bases.required = captured.required;
        self.launch_scratch.required = captured.required;
        self.launch_result = captured.provider;
        self.launch_bases.len = match captured.provider {
            Ok(ccountry::launch_sources::NavalProvider::Bases(len)) => len,
            _ => 0,
        };
    }
}

pub(crate) struct Captured {
    sample_hour: Option<i32>,
    physical_count: Result<usize, &'static str>,
    known_access_modes: usize,
    owners: Arena<OwnerRow>,
    owner_flags: Arena<u8>,
    navies: Arena<NavyRow>,
    physical_slots: Arena<u32>,
    foreign: Arena<u32>,
    hubs: Arena<u32>,
    domain: Arena<native::ControllerAreaProvince>,
    marks: Arena<u8>,
    area_scratch: Arena<native::ControllerAreaProvince>,
    areas: Arena<native::AreaInput>,
    area_edges: Arena<u32>,
    area_reason: Option<&'static str>,
    headers: Arena<native::TheatreInput>,
    theatre_provinces: Arena<u32>,
    pub(crate) source_reason: Option<&'static str>,
    pub(crate) province_reason: Option<&'static str>,
}

impl Captured {
    pub(crate) fn new(capacity: Capacities) -> Self {
        Self {
            sample_hour: None,
            physical_count: Err("naval movement physical pass not begun"),
            known_access_modes: 0,
            owners: Arena::new(capacity.owners),
            owner_flags: Arena::new(capacity.owner_slots),
            navies: Arena::new(capacity.navies),
            physical_slots: Arena::new(capacity.physical_slots),
            foreign: Arena::new(capacity.foreign),
            hubs: Arena::new(capacity.hubs),
            domain: Arena::new(capacity.provinces),
            marks: Arena::new(capacity.provinces),
            area_scratch: Arena::new(capacity.provinces),
            areas: Arena::new(capacity.provinces),
            area_edges: Arena::new(capacity.area_edges),
            area_reason: Some("selected area sources not supplied"),
            headers: Arena::new(capacity.theatre_headers),
            theatre_provinces: Arena::new(capacity.theatre_provinces),
            source_reason: Some("naval movement sources not supplied"),
            province_reason: Some("naval movement sources not supplied"),
        }
    }

    fn clear_controls(&mut self) {
        for row in &mut self.owners.first {
            row.source = None;
        }
        for row in &mut self.navies.first {
            row.inputs = None;
        }
        for entry in &mut self.domain.first {
            entry.clear();
        }
        for header in &mut self.headers.first {
            header.clear();
        }
        for entry in &mut self.area_scratch.first {
            entry.clear();
        }
        self.owner_flags.first.fill(0);
        self.marks.first.fill(0);
    }

    pub(crate) fn reset(&mut self) {
        self.clear_controls();
        self.sample_hour = None;
        self.physical_count = Err("naval movement physical pass not begun");
        self.known_access_modes = 0;
        self.owners.reset();
        self.owner_flags.reset();
        self.navies.reset();
        self.physical_slots.reset();
        self.foreign.reset();
        self.hubs.reset();
        self.domain.reset();
        self.areas.reset();
        self.area_scratch.reset();
        self.area_edges.reset();
        self.area_reason = Some("selected area sources not supplied");
        self.marks.reset();
        self.headers.reset();
        self.theatre_provinces.reset();
        self.source_reason = Some("naval movement sources not supplied");
        self.province_reason = Some("naval movement sources not supplied");
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.owners.allocated_bytes()
            + self.owner_flags.allocated_bytes()
            + self.navies.allocated_bytes()
            + self.physical_slots.allocated_bytes()
            + self.foreign.allocated_bytes()
            + self.hubs.allocated_bytes()
            + self.domain.allocated_bytes()
            + self.areas.allocated_bytes()
            + self.area_scratch.allocated_bytes()
            + self.area_edges.allocated_bytes()
            + self.marks.allocated_bytes()
            + self.headers.allocated_bytes()
            + self.theatre_provinces.allocated_bytes()
    }

    pub(crate) fn grow(&mut self, capacity: &mut Capacities) {
        self.owners.grow(&mut capacity.owners);
        self.owner_flags.grow(&mut capacity.owner_slots);
        self.navies.grow(&mut capacity.navies);
        self.physical_slots.grow(&mut capacity.physical_slots);
        self.foreign.grow(&mut capacity.foreign);
        self.hubs.grow(&mut capacity.hubs);
        crate::table_capture::grow_capacity(
            &mut capacity.provinces,
            self.domain.required.max(self.marks.required),
        );
        self.domain.grow(&mut capacity.provinces);
        self.marks.grow(&mut capacity.provinces);
        self.areas.grow(&mut capacity.provinces);
        self.area_scratch.grow(&mut capacity.provinces);
        self.area_edges.grow(&mut capacity.area_edges);
        self.headers.grow(&mut capacity.theatre_headers);
        self.theatre_provinces.grow(&mut capacity.theatre_provinces);
    }

    pub(crate) fn required(&self) -> Capacities {
        Capacities {
            owners: self.owners.required,
            owner_slots: self.owner_flags.required,
            navies: self.navies.required,
            physical_slots: self.physical_slots.required,
            foreign: self.foreign.required,
            hubs: self.hubs.required,
            provinces: self.domain.required.max(self.marks.required),
            area_edges: self.area_edges.required,
            theatre_headers: self.headers.required,
            theatre_provinces: self.theatre_provinces.required,
        }
    }

    pub(crate) fn retain_required(&mut self, demand: Capacities) {
        self.owners.required = self.owners.required.max(demand.owners);
        self.owner_flags.required = self.owner_flags.required.max(demand.owner_slots);
        self.navies.required = self.navies.required.max(demand.navies);
        self.physical_slots.required = self.physical_slots.required.max(demand.physical_slots);
        self.foreign.required = self.foreign.required.max(demand.foreign);
        self.hubs.required = self.hubs.required.max(demand.hubs);
        self.domain.required = self.domain.required.max(demand.provinces);
        self.area_edges.required = self.area_edges.required.max(demand.area_edges);
        self.headers.required = self.headers.required.max(demand.theatre_headers);
        self.theatre_provinces.required = self
            .theatre_provinces
            .required
            .max(demand.theatre_provinces);
    }

    fn capacity(&self) -> Capacities {
        Capacities {
            owners: self.owners.first.len(),
            owner_slots: self.owner_flags.first.len(),
            navies: self.navies.first.len(),
            physical_slots: self.physical_slots.first.len(),
            foreign: self.foreign.first.len(),
            hubs: self.hubs.first.len(),
            provinces: self.domain.first.len().min(self.marks.first.len()),
            area_edges: self.area_edges.first.len(),
            theatre_headers: self.headers.first.len(),
            theatre_provinces: self.theatre_provinces.first.len(),
        }
    }

    /// One original holder acquires all physical manager slots before the
    /// mission pass, including aliases and native nulls. Controls survive until
    /// finish; no borrower may reacquire a failed slot.
    pub(crate) fn begin<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        state: usize,
        hour: Option<i32>,
        shared: &mut Shared,
    ) -> native::Reader<'a, M> {
        self.reset();
        shared.reset();
        self.sample_hour = hour;
        shared.sample_hour = hour;
        let reader = native::Reader::new(memory, base, state);
        // All current provinces, including when every unit is idle. Borrow the
        // SAME original observation before the moving-owner early returns.
        let domain = reader.capture_controller_area_domain_growing(&mut self.domain.first);
        self.domain.required = domain.required;
        self.domain.len = domain.len.unwrap_or(0);
        self.province_reason = domain.len.err();
        self.marks.required = domain.required;
        self.areas.required = domain.required;
        self.area_scratch.required = domain.required;
        let captured = reader.capture_selected_areas_growing(
            state,
            &domain,
            &self.domain.first[..self.domain.len],
            &mut self.area_scratch.first,
            &mut self.areas.first,
            &mut self.area_edges.first,
        );
        self.areas.len = captured.len.unwrap_or(0);
        self.area_edges.len = captured.edges_len;
        self.area_edges.required = captured.required_edges;
        self.area_reason = captured.len.err();
        if let Some(reason) = self.area_reason {
            area_refusal(reason);
        }
        for row in &self.areas.first[..self.areas.len] {
            if row.owner.is_none() {
                area_refusal("selected D8 observation unreadable");
            }
            if let Some(node) = row.node {
                if let Some(reason) = node.failure {
                    area_refusal(reason);
                }
                if node.first_land.is_none()
                    || node.nonsea_index.is_none()
                    || node.nonsea_count.is_none()
                    || node.land_count.is_none()
                {
                    area_refusal("selected area scalar or first-land identity unreadable");
                }
            }
        }

        self.physical_count = reader.bounds().navy_slots;
        if let Ok(count) = self.physical_count {
            self.navies.required = count;
            if let Err(reason) = engine::walk::Buffer::Growing(&mut self.navies.first).ensure(count)
            {
                self.physical_count = Err(reason);
                return reader;
            }
            for index in 0..count {
                let Ok(slot) = u32::try_from(index) else {
                    self.physical_count = Err("naval movement physical slot overflow");
                    break;
                };
                let mut observed = reader.observe_navy(slot);
                if let Ok(inputs) = &observed {
                    self.known_access_modes = self
                        .known_access_modes
                        .saturating_add(inputs.access_mode_count().unwrap_or(0));
                }
                if slot == 0 {
                    shared.capture_launch(memory, base, &mut observed);
                }
                if let Some(row) = self.navies.first.get_mut(index) {
                    *row = NavyRow {
                        slot,
                        ..Default::default()
                    };
                    match observed {
                        Ok(inputs) => {
                            row.present = inputs.present;
                            row.inputs = Some(inputs);
                        }
                        Err(reason) => row.identity_reason = Some(reason),
                    }
                    self.navies.len += 1;
                }
            }
            // Empty/null manager returns native null for any requested slot.
            if count == 0 {
                let mut zero = reader.observe_navy(0);
                shared.capture_launch(memory, base, &mut zero);
            }
        }
        reader
    }

    pub(crate) fn physical_count(&self) -> Result<usize, &'static str> {
        self.physical_count
    }

    pub(crate) fn known_access_modes(&self) -> usize {
        self.known_access_modes
    }

    pub(crate) fn borrow_navy(
        &mut self,
        slot: u32,
    ) -> Result<&mut native::NavyInputs, &'static str> {
        self.physical_count?;
        let row = self
            .navies
            .first
            .get_mut(slot as usize)
            .filter(|row| row.slot == slot && (slot as usize) < self.navies.len)
            .ok_or("naval movement original physical navy arena exhausted")?;
        if let Some(reason) = row.identity_reason {
            return Err(reason);
        }
        row.inputs
            .as_mut()
            .ok_or("naval movement original navy control retired")
    }

    /// Legacy callers use the same begin/finish operations. Ordinary borrows
    /// the controls for the original mission pass between these operations.
    #[cfg(test)]
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: Option<i32>,
        census: OwnerCensus<impl IntoIterator<Item = Result<ccountry::Country, &'static str>>>,
        shared: &mut Shared,
    ) {
        let mut reader = self.begin(memory, base, state, hour, shared);
        self.finish(memory, base, state, census, shared, &reader);
        reader.clear();
    }

    pub(crate) fn finish<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        census: OwnerCensus<impl IntoIterator<Item = Result<ccountry::Country, &'static str>>>,
        shared: &mut Shared,
        reader: &native::Reader<'_, M>,
    ) {
        let OwnerCensus {
            owners,
            failure: source_failure,
        } = census;
        self.source_reason = source_failure;
        shared.nearest_reason = source_failure;
        shared.reserved_reason = None;
        let result = self.capture_inner(memory, base, state, owners, shared, reader);
        if let Err(reason) = result {
            self.source_reason.get_or_insert(reason);
            shared.nearest_reason.get_or_insert(reason);
            shared.reserved_reason.get_or_insert(reason);
        }
        self.clear_controls();
    }

    fn capture_inner<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        owners: impl IntoIterator<Item = Result<ccountry::Country, &'static str>>,
        shared: &mut Shared,
        reader: &native::Reader<'_, M>,
    ) -> Result<(), &'static str> {
        if self.sample_hour.is_none() {
            return Err("naval movement sample hour unavailable");
        }
        let countries = ccountry::Reader::new(memory, base, state);
        self.owner_flags.required = countries
            .as_ref()
            .map_or(0, |reader| reader.count() as usize);
        engine::walk::Buffer::Growing(&mut self.owner_flags.first)
            .ensure(self.owner_flags.required)?;
        for owner in owners {
            let owner = match owner {
                Ok(owner) if owner.id != 0 && owner.canonical_slot != 0 && owner.pointer != 0 => {
                    owner
                }
                Ok(_) => {
                    self.source_reason
                        .get_or_insert("naval movement moving owner is not positive");
                    continue;
                }
                Err(reason) => {
                    self.source_reason.get_or_insert(reason);
                    continue;
                }
            };
            // The same callback-authenticated object can be demanded by many
            // task forces. Dedup by canonical physical slot, not native ID.
            let slot = owner.canonical_slot as usize;
            self.owner_flags.required = self.owner_flags.required.max(
                slot.checked_add(1)
                    .ok_or("naval movement owner-slot demand overflow")?,
            );
            engine::walk::Buffer::Growing(&mut self.owner_flags.first)
                .ensure(self.owner_flags.required)?;
            let flag = &mut self.owner_flags.first[slot];
            if *flag != 0 {
                continue;
            }
            *flag = 1;
            self.owners.required = self
                .owners
                .required
                .checked_add(1)
                .ok_or("naval movement owner demand overflow")?;
            engine::walk::Buffer::Growing(&mut self.owners.first).ensure(self.owners.required)?;
            self.owners.first[self.owners.len] = OwnerRow {
                country: owner.id,
                navy_slot: owner.canonical_slot,
                source: Some(owner),
                ..Default::default()
            };
            self.owners.len += 1;
        }
        self.owners.first[..self.owners.len].sort_unstable_by_key(|row| row.country);
        if self.owners.first[..self.owners.len]
            .windows(2)
            .any(|pair| pair[0].country == pair[1].country)
        {
            return Err("naval movement demanded native owner IDs are duplicated");
        }
        if self.owners.required == 0 && self.source_reason.is_none() {
            shared.reserved_reason = Some("naval movement reserved theatre not demanded");
            return Ok(());
        }
        shared.reserved_demanded = true;
        // Own slots are sorted first. Foreign cache slots are appended only
        // from actual successful own navy lists, preserving those lists' order.
        for row in &self.owners.first[..self.owners.len] {
            self.physical_slots.required = self
                .physical_slots
                .required
                .checked_add(1)
                .ok_or("naval movement physical slot demand overflow")?;
            engine::walk::Buffer::Growing(&mut self.physical_slots.first)
                .ensure(self.physical_slots.required)?;
            self.physical_slots.first[self.physical_slots.len] = row.navy_slot;
            self.physical_slots.len += 1;
        }
        self.physical_slots.first[..self.physical_slots.len].sort_unstable();
        self.physical_slots.len =
            unique_len(&mut self.physical_slots.first[..self.physical_slots.len]);
        let own_slots = self.physical_slots.len;
        for index in 0..own_slots {
            self.observe_navy(memory, self.physical_slots.first[index], true, shared)?;
        }
        for index in 0..self.foreign.len {
            self.physical_slots.required = self
                .physical_slots
                .required
                .checked_add(1)
                .ok_or("naval movement physical slot demand overflow")?;
            engine::walk::Buffer::Growing(&mut self.physical_slots.first)
                .ensure(self.physical_slots.required)?;
            self.physical_slots.first[self.physical_slots.len] = self.foreign.first[index];
            self.physical_slots.len += 1;
        }
        self.physical_slots.required = self.physical_slots.required.max(
            self.owners
                .required
                .checked_add(self.foreign.required)
                .ok_or("naval movement physical slot demand overflow")?,
        );
        // Missing foreign payload makes the needed physical closure unknown.
        for row in &self.navies.first[..self.navies.len] {
            if !row.demanded {
                continue;
            }
            if let Some(reason) = row.foreign_reason {
                shared.nearest_reason.get_or_insert(reason);
            }
        }
        self.physical_slots.first[..self.physical_slots.len].sort_unstable();
        self.physical_slots.len =
            unique_len(&mut self.physical_slots.first[..self.physical_slots.len]);
        self.navies.first[..self.navies.len].sort_unstable_by_key(|row| row.slot);
        for index in 0..self.physical_slots.len {
            let slot = self.physical_slots.first[index];
            // Own slots are handled only by the own pass, never the foreign pass.
            if self
                .owner_flags
                .first
                .get(slot as usize)
                .is_none_or(|&flag| flag == 0)
            {
                self.observe_navy(memory, slot, false, shared)?;
            }
        }
        let bounds = reader.bounds();
        let province_slots = bounds.provinces.and_then(|bound| {
            bound
                .checked_add(1)
                .ok_or("naval movement province bound overflow")
        });
        if let Err(reason) = province_slots {
            shared.nearest_reason.get_or_insert(reason);
        }
        shared.nearest.required = self.physical_slots.required;
        engine::walk::Buffer::Growing(&mut shared.nearest.first).ensure(shared.nearest.required)?;
        for row in &mut self.navies.first[..self.navies.len] {
            if !row.demanded {
                continue;
            }
            let Some(inputs) = row.inputs.as_mut() else {
                continue;
            };
            let demand = inputs.demand().nearest_bases;
            let result = match province_slots {
                Ok(province_slots) => inputs.capture_nearest_buffer(
                    memory,
                    base,
                    province_slots,
                    engine::walk::Buffer::Growing(&mut shared.bases.first).suffix(shared.bases.len),
                ),
                Err(reason) => engine::carmy::country_movement_inputs::DomainCapture {
                    len: Err(reason),
                    required: demand.required,
                    first_failure: None,
                },
            };
            shared.bases.required = shared
                .bases
                .required
                .checked_add(result.required)
                .ok_or("naval movement nearest demand overflow")?;
            let bases = result.len.ok().map(|len| {
                let span = Span {
                    start: shared.bases.len,
                    len,
                };
                shared.bases.len += len;
                span
            });
            shared.nearest.first[shared.nearest.len] = NearestRow {
                slot: row.slot,
                present: row.present,
                bases,
                reason: result.len.err(),
            };
            shared.nearest.len += 1;
        }
        if let Some(reason) = self.source_reason {
            shared.nearest_reason.get_or_insert(reason);
        }

        for index in 0..self.owners.len {
            let owner = self.owners.first[index]
                .source
                .ok_or("naval movement owner source cleared early")?;
            let captured = self.capture_theatre(
                reader,
                countries.as_ref().map_err(|&reason| reason),
                native::Owner::Positive(owner),
                false,
                shared,
            )?;
            self.owners.first[index].theatre = captured.0;
            self.owners.first[index].null_controller_area = captured.1;
            self.owners.first[index].theatre_reason = captured.2;
        }
        match reader.reserved_owner() {
            Ok(owner) => {
                let captured = self.capture_theatre(
                    reader,
                    countries.as_ref().map_err(|&reason| reason),
                    owner,
                    true,
                    shared,
                )?;
                shared.null_controller_area = captured.1;
                shared.reserved_reason = captured.2;
            }
            Err(reason) => {
                shared.reserved_reason = Some(reason);
            }
        }
        Ok(())
    }

    /// Exact current-row comparison borrows the owned occurrence arena. No
    /// graph vector is allocated just to decide whether projection changed.
    pub(crate) fn province_area_matches(
        &self,
        province: u32,
        previous: &replay::rows::Recorded<Option<replay::rows::ProvinceAreaInputs>>,
    ) -> bool {
        let replay::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let current = self
            .sample_hour
            .filter(|_| self.area_reason.is_none())
            .and_then(|hour| {
                let at = self.areas.first[..self.areas.len]
                    .binary_search_by_key(&province, |r| r.province)
                    .ok()?;
                Some((hour, &self.areas.first[at]))
            });
        let (hour, row, old) = match (current, previous) {
            (None, None) => return true,
            (Some((hour, row)), Some(old)) => (hour, row, old),
            _ => return false,
        };
        if hour != old.sample_hour || row.controller != old.controller || row.owner != old.owner {
            return false;
        }
        match (&row.node, &old.node) {
            (None, None) => true,
            (Some(node), Some(old)) => {
                node.first_land == old.first_land
                    && node.nonsea_index == old.nonsea_index
                    && node.nonsea_count == old.nonsea_count
                    && node.land_count == old.land_count
                    && node.neighbours.and_then(|(start, len)| {
                        self.area_edges.first[..self.area_edges.len]
                            .get(start..start.checked_add(len)?)
                    }) == old.neighbours.as_deref()
            }
            _ => false,
        }
    }

    pub(crate) fn province_area(&self, province: u32) -> Option<replay::rows::ProvinceAreaInputs> {
        let hour = self.sample_hour?;
        self.area_reason.map_or(Some(()), |_| None)?;
        let at = self.areas.first[..self.areas.len]
            .binary_search_by_key(&province, |r| r.province)
            .ok()?;
        let row = self.areas.first[at];
        Some(replay::rows::ProvinceAreaInputs {
            sample_hour: hour,
            controller: row.controller,
            owner: row.owner,
            node: row.node.map(|node| replay::rows::SelectedAreaNode {
                first_land: node.first_land,
                nonsea_index: node.nonsea_index,
                nonsea_count: node.nonsea_count,
                land_count: node.land_count,
                neighbours: node
                    .neighbours
                    .and_then(|(start, len)| {
                        self.area_edges.first[..self.area_edges.len]
                            .get(start..start.checked_add(len)?)
                    })
                    .map(|values| {
                        engine::allocation::owned_vec(engine::allocation::copy_vec(values))
                    }),
            }),
        })
    }

    fn observe_navy<M: Memory>(
        &mut self,
        memory: &M,
        slot: u32,
        own: bool,
        shared: &mut Shared,
    ) -> Result<(), &'static str> {
        if self.physical_count == Ok(0) {
            // Empty manager is native null for every requested slot. Keep its
            // numerical row without inventing an authenticated live control.
            if own {
                self.navies.required = self.navies.required.max(self.navies.len.saturating_add(1));
                engine::walk::Buffer::Growing(&mut self.navies.first)
                    .ensure(self.navies.required)?;
                self.navies.first[self.navies.len] = NavyRow {
                    slot,
                    present: false,
                    foreign: Some(Span {
                        start: self.foreign.len,
                        len: 0,
                    }),
                    hubs: Some(Span {
                        start: self.hubs.len,
                        len: 0,
                    }),
                    ..Default::default()
                };
                self.navies.len += 1;
            }
            engine::walk::Buffer::Growing(&mut shared.nearest.first).ensure(
                shared
                    .nearest
                    .len
                    .checked_add(1)
                    .ok_or("naval movement nearest-row demand overflow")?,
            )?;
            shared.nearest.first[shared.nearest.len] = NearestRow {
                slot,
                present: false,
                bases: Some(Span {
                    start: shared.bases.len,
                    len: 0,
                }),
                reason: None,
            };
            shared.nearest.len += 1;
            return Ok(());
        }
        let row = self
            .navies
            .first
            .get_mut(slot as usize)
            .filter(|row| row.slot == slot && (slot as usize) < self.navies.len)
            .ok_or("naval movement demanded original navy arena exhausted")?;
        row.demanded = true;
        match row.inputs.as_mut() {
            None => {
                shared.nearest_reason.get_or_insert(
                    row.identity_reason
                        .unwrap_or("naval movement original navy control retired"),
                );
            }
            Some(inputs) => {
                if own {
                    let foreign = inputs.capture_foreign_navy_slots_buffer(
                        memory,
                        engine::walk::Buffer::Growing(&mut self.foreign.first)
                            .suffix(self.foreign.len),
                    );
                    self.foreign.required = self
                        .foreign
                        .required
                        .checked_add(foreign.required)
                        .ok_or("naval movement foreign demand overflow")?;
                    row.foreign = foreign.len.ok().map(|len| {
                        let span = Span {
                            start: self.foreign.len,
                            len,
                        };
                        self.foreign.len += len;
                        span
                    });
                    row.foreign_reason = foreign.len.err();
                    let hubs = inputs.capture_supply_hubs_buffer(
                        memory,
                        engine::walk::Buffer::Growing(&mut self.hubs.first).suffix(self.hubs.len),
                    );
                    self.hubs.required = self
                        .hubs
                        .required
                        .checked_add(hubs.required)
                        .ok_or("naval movement hub demand overflow")?;
                    row.hubs = hubs.len.ok().map(|len| {
                        let span = Span {
                            start: self.hubs.len,
                            len,
                        };
                        self.hubs.len += len;
                        span
                    });
                    row.hubs_reason = hubs.len.err();
                }
            }
        }
        Ok(())
    }

    fn capture_theatre<M: Memory>(
        &mut self,
        reader: &native::Reader<'_, M>,
        countries: Result<&ccountry::Reader<'_, M>, &'static str>,
        owner: native::Owner,
        reserved: bool,
        shared: &mut Shared,
    ) -> Result<(Option<Span>, bool, Option<&'static str>), &'static str> {
        let headers = reader.capture_theatre_headers_optional_growing(
            countries.ok(),
            owner,
            &mut self.headers.first,
        );
        self.headers.required = self.headers.required.max(headers.required_headers);
        let header_len = match headers.len {
            Ok(len) => len,
            Err(reason) => {
                self.theatre_output_floor(reserved, shared)?;
                let reason = if reserved {
                    reason
                } else {
                    countries.err().unwrap_or(reason)
                };
                return Ok((None, false, Some(reason)));
            }
        };
        if let Some(reason) = self.province_reason {
            for header in &mut self.headers.first[..header_len] {
                header.clear();
            }
            self.theatre_output_floor(reserved, shared)?;
            return Ok((None, false, Some(reason)));
        }
        let output = if reserved {
            &mut shared.reserved_provinces
        } else {
            &mut self.theatre_provinces
        };
        let membership = reader.capture_theatre_membership_buffer(
            &mut self.headers.first[..header_len],
            &self.domain.first[..self.domain.len],
            engine::walk::Buffer::Growing(&mut self.marks.first),
            engine::walk::Buffer::Growing(&mut output.first).suffix(output.len),
        );
        output.required = output
            .required
            .checked_add(membership.provinces.required)
            .ok_or("naval movement theatre demand overflow")?;
        let span = membership.provinces.len.ok().map(|len| {
            let span = Span {
                start: output.len,
                len,
            };
            output.len += len;
            span
        });
        Ok((
            span,
            membership.null_controller_area,
            membership.provinces.len.err(),
        ))
    }

    fn theatre_output_floor(
        &mut self,
        reserved: bool,
        shared: &mut Shared,
    ) -> Result<(), &'static str> {
        let output = if reserved {
            &mut shared.reserved_provinces
        } else {
            &mut self.theatre_provinces
        };
        // Only the actual queried owners contribute this conservative bound;
        // it prevents a short source arena from erasing output pressure.
        output.required = output
            .required
            .checked_add(self.domain.required)
            .ok_or("naval movement theatre demand overflow")?;
        Ok(())
    }

    pub(crate) fn for_country(
        &self,
        country: u32,
    ) -> std::io::Result<(Option<NavalPathInputs>, Option<TheatreControllerAreas>)> {
        let Ok(index) =
            self.owners.first[..self.owners.len].binary_search_by_key(&country, |row| row.country)
        else {
            return Ok((None, None));
        };
        let owner = &self.owners.first[index];
        let naval = self.navies.first[..self.navies.len]
            .binary_search_by_key(&owner.navy_slot, |row| row.slot)
            .ok()
            .map(|index| &self.navies.first[index])
            .filter(|row| row.identity_reason.is_none())
            .map(|row| -> std::io::Result<_> {
                Ok(NavalPathInputs {
                    navy_slot: row.slot,
                    present: row.present,
                    foreign_navy_slots: row
                        .foreign
                        .map(|span| copy_span(&self.foreign, span))
                        .transpose()?,
                    supply_hubs: row
                        .hubs
                        .map(|span| copy_span(&self.hubs, span))
                        .transpose()?,
                })
            })
            .transpose()?;
        let theatre = owner
            .theatre
            .map(|span| -> std::io::Result<_> {
                Ok(TheatreControllerAreas {
                    provinces: copy_span(&self.theatre_provinces, span)?,
                    null_controller_area: owner.null_controller_area,
                })
            })
            .transpose()?;
        Ok((naval, theatre))
    }

    pub(crate) fn sample_hour(&self) -> Option<i32> {
        self.sample_hour
    }

    fn refusals(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.source_reason
            .into_iter()
            .chain(self.province_reason)
            .chain(
                self.owners.first[..self.owners.len]
                    .iter()
                    .filter_map(|row| row.theatre_reason),
            )
            .chain(self.navies.first[..self.navies.len].iter().flat_map(|row| {
                [row.identity_reason, row.foreign_reason, row.hubs_reason]
                    .into_iter()
                    .flatten()
            }))
    }
}

fn unique_len(values: &mut [u32]) -> usize {
    let mut len = 0;
    for index in 0..values.len() {
        if len == 0 || values[index] != values[len - 1] {
            values[len] = values[index];
            len += 1;
        }
    }
    len
}

fn copy_span<T: Copy>(arena: &Arena<T>, span: Span) -> std::io::Result<Vec<T>> {
    let values = span
        .start
        .checked_add(span.len)
        .and_then(|end| arena.first.get(..arena.len)?.get(span.start..end))
        .ok_or_else(|| std::io::Error::other("naval movement span outside acquired arena"))?;
    engine::allocation::copy_vec(values)
}

#[derive(Default)]
pub(crate) struct Work {
    sampled: u64,
    nearest_available: u64,
    nearest_refused: u64,
    reserved_available: u64,
    reserved_refused: u64,
    reserved_unreached: u64,
    launch_available: u64,
    launch_native_null: u64,
    launch_refused: u64,
    refusals: Vec<(&'static str, u64)>,
    required: Capacities,
    shared_required: SharedCapacities,
    capacity: Capacities,
    shared_capacity: SharedCapacities,
}

impl Work {
    pub(crate) fn observe(&mut self, capture: &Captured, shared: &Shared) -> std::io::Result<()> {
        if capture.sample_hour.is_none() {
            return Ok(());
        }
        self.sampled = self.sampled.saturating_add(1);
        self.required.raise(capture.required());
        self.shared_required.raise(shared.required());
        self.capacity = capture.capacity();
        self.shared_capacity = shared.capacity();
        if shared.launch_result.is_ok() {
            self.launch_available = self.launch_available.saturating_add(1);
            if matches!(
                shared.launch_result,
                Ok(ccountry::launch_sources::NavalProvider::NativeNull)
            ) {
                self.launch_native_null = self.launch_native_null.saturating_add(1);
            }
        } else {
            self.launch_refused = self.launch_refused.saturating_add(1);
        }
        if shared.nearest_reason.is_none() {
            self.nearest_available = self.nearest_available.saturating_add(1);
        } else {
            self.nearest_refused = self.nearest_refused.saturating_add(1);
        }
        if !shared.reserved_demanded {
            self.reserved_unreached = self.reserved_unreached.saturating_add(1);
        } else if shared.reserved_reason.is_none() {
            self.reserved_available = self.reserved_available.saturating_add(1);
        } else {
            self.reserved_refused = self.reserved_refused.saturating_add(1);
        }
        for reason in capture
            .refusals()
            .chain(shared.nearest_reason)
            .chain(shared.reserved_reason.filter(|_| shared.reserved_demanded))
            .chain(shared.launch_result.err())
            .chain(
                shared.nearest.first[..shared.nearest.len]
                    .iter()
                    .filter_map(|row| row.reason),
            )
        {
            let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
                Ok(index) => index,
                Err(index) => {
                    engine::allocation::reserve(&mut self.refusals, 1)?;
                    self.refusals.insert(index, (reason, 0));
                    index
                }
            };
            self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "sampled_hours": self.sampled,
            "nearest_available_hours": self.nearest_available,
            "nearest_refused_hours": self.nearest_refused,
            "reserved_theatre_available_hours": self.reserved_available,
            "reserved_theatre_refused_hours": self.reserved_refused,
            "reserved_theatre_unreached_hours": self.reserved_unreached,
            "reserved_launch_naval_available_hours": self.launch_available,
            "reserved_launch_naval_native_null_hours": self.launch_native_null,
            "reserved_launch_naval_refused_hours": self.launch_refused,
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "owners_required": self.required.owners,
            "physical_slots_required": self.required.physical_slots,
            "foreign_slots_required": self.required.foreign,
            "supply_hubs_required": self.required.hubs,
            "controller_area_domain_required": self.required.provinces,
            "theatre_headers_required": self.required.theatre_headers,
            "theatre_provinces_required": self.required.theatre_provinces,
            "nearest_rows_required": self.shared_required.nearest_rows,
            "nearest_bases_required": self.shared_required.nearest_bases,
            "reserved_theatre_provinces_required": self.shared_required.reserved_provinces,
            "reserved_launch_naval_bases_required": self.shared_required.launch_bases,
            "owners_capacity": self.capacity.owners,
            "owner_slots_capacity": self.capacity.owner_slots,
            "navies_capacity": self.capacity.navies,
            "physical_slots_capacity": self.capacity.physical_slots,
            "foreign_slots_capacity": self.capacity.foreign,
            "supply_hubs_capacity": self.capacity.hubs,
            "controller_area_domain_capacity": self.capacity.provinces,
            "theatre_headers_capacity": self.capacity.theatre_headers,
            "theatre_provinces_capacity": self.capacity.theatre_provinces,
            "nearest_rows_capacity": self.shared_capacity.nearest_rows,
            "nearest_bases_capacity": self.shared_capacity.nearest_bases,
            "reserved_theatre_provinces_capacity": self.shared_capacity.reserved_provinces,
            "reserved_launch_naval_bases_capacity": self.shared_capacity.launch_bases,
        })
    }
}

#[cfg(test)]
mod tests;

const AREA_REFUSAL_NAMES: [&str; 6] = [
    "selected area capacity refused",
    "selected area deferred or marked payload lifetime refused",
    "selected area orphan refused",
    "selected area controls or links refused",
    "selected area scalar or D8 refused",
    "selected area other acquisition refused",
];
static AREA_REFUSALS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];
fn area_refusal(reason: &'static str) {
    let index = if reason.contains("arena") {
        0
    } else if reason.contains("deferred") || reason.contains("marked") {
        1
    } else if reason.contains("orphan") {
        2
    } else if reason.contains("adjacency") {
        3
    } else if reason.contains("scalar") || reason.contains("D8") {
        4
    } else {
        5
    };
    AREA_REFUSALS[index].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}
pub(crate) fn area_data() -> impl serde::Serialize {
    crate::status::status!({"refusals":crate::status::map(AREA_REFUSAL_NAMES.into_iter()
        .zip(AREA_REFUSALS.iter()).map(|(name,count)|(name,count.load(std::sync::atomic::Ordering::Relaxed)))
        .filter(|(_,count)|*count!=0))})
}
