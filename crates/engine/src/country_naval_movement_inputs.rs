//! Current-hour simulation operands for class-1 counter path admission.
//! Select the roster owner's navy and owned theatres. Nearest-base and foreign
//! physical-manager slots retain stored order without resolution through newer
//! country links. Hub order remains stored order. These shared inputs refresh
//! through hourly dirty gates, staggered daily updates, setup and territory/base
//! changes; hubs also change on construction.
//! Compare every owned theatre's stored area pointers with the target actual
//! area, permitting null-to-null matches. Membership is simulation state;
//! volunteer filters, current-area province lists and UI caches do not substitute.
//! Controls are private and acquired/consumed once. Payload buffers are caller-
//! owned. Refused vectors expose no prefix but retain full known demand;
//! independent vectors and theatre domains preserve separate failures.
//! Drop/clear removes private pointers before the frozen pass ends.

use crate::{
    Memory,
    carmy::country_movement_inputs::DomainCapture,
    ccountry, cgame_state, cnaval_base,
    cpdx_array::{self, Array},
    cprovince, cstrategic_navy, cstrategic_navy_manager, ctheatre,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
};

/// Province-ID-indexed nearest-base pointer array.
pub const NEAREST_BASES: usize = 0x30;
/// Foreign physical navy slots retained in stored order.
pub const FOREIGN_NAVY_SLOTS: usize = 0x48;
/// Selecting manager stored during construction.
pub const MANAGER: usize = 0x60;
/// Stored hub u32 array used by range selection.
pub const SUPPLY_HUBS: usize = crate::naval_mission_inputs::HUBS;
/// Complete membership source/domain succeeded before this capacity refusal.
pub const THEATRE_PROVINCE_CAPACITY: &str = "naval movement theatre province arena exhausted";

mod areas;
pub use areas::{AreaCapture, AreaInput, AreaNodeInput};

const CONSUMED: &str = "naval movement input already consumed";
const BASE_BATCH: usize = 64;
const BASE_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cnaval_base::PROVINCE_ID,
        destination_offset: 8,
        len: 4,
    },
];
const BASE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BASE_FIELDS, 12).expect("valid nearest base layout");
const PROVINCE_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cprovince::ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: cprovince::CONTROLLER_AREA,
        destination_offset: 12,
        len: 8,
    },
];
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, 20).expect("valid province area layout");

struct ProvinceBatch {
    pointers: [usize; BASE_BATCH],
    slots: [u32; BASE_BATCH],
    len: usize,
}

impl ProvinceBatch {
    fn new() -> Self {
        Self {
            pointers: [0; BASE_BATCH],
            slots: [0; BASE_BATCH],
            len: 0,
        }
    }

    fn flush(
        &mut self,
        memory: &impl Memory,
        expected: u64,
        output: &mut [ControllerAreaProvince],
        written: &mut usize,
    ) -> Result<(), &'static str> {
        let count = self.len;
        self.len = 0;
        if count == 0 {
            return Ok(());
        }
        let mut bytes = [0; BASE_BATCH * 20];
        let copied = memory.copy_records(
            RecordSources::Pointers(&self.pointers[..count]),
            &PROVINCE_LAYOUT,
            &mut bytes[..count * 20],
        );
        let result = (|| {
            if copied != count * PROVINCE_FIELDS.len() {
                return Err("naval movement province-area record unreadable");
            }
            for (index, record) in bytes[..count * 20].chunks_exact(20).enumerate() {
                let id = u32::from_le_bytes(record[8..12].try_into().unwrap());
                // The selecting slot's pointer was already copied in this pass;
                // vtable + own ID authenticate it without rereading that slot.
                if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected
                    || id != self.slots[index]
                {
                    return Err("naval movement province identity mismatch");
                }
                let area = usize::try_from(u64::from_le_bytes(record[12..20].try_into().unwrap()))
                    .map_err(|_| "naval movement province area pointer overflow")?;
                let selected = self.pointers[index];
                let owner = selected
                    .checked_add(cprovince::OWNER_AREA)
                    .and_then(|address| read::<u64>(memory, address))
                    .map(|p| p as usize);
                output[*written] = ControllerAreaProvince {
                    province: id,
                    area,
                    owner,
                    pointer: selected,
                };
                *written += 1;
            }
            Ok(())
        })();
        self.pointers.fill(0);
        self.slots.fill(0);
        result
    }
}

/// Bounded sparse identity/ID copies, not one guarded operation per pointer.
struct BaseBatch {
    pointers: [usize; BASE_BATCH],
    indices: [usize; BASE_BATCH],
    len: usize,
}

impl BaseBatch {
    fn new() -> Self {
        Self {
            pointers: [0; BASE_BATCH],
            indices: [0; BASE_BATCH],
            len: 0,
        }
    }

    fn flush(
        &mut self,
        memory: &impl Memory,
        expected: u64,
        province_slots: usize,
        output: &mut [Option<u32>],
    ) -> Result<(), &'static str> {
        let count = self.len;
        self.len = 0;
        if count == 0 {
            return Ok(());
        }
        let mut bytes = [0; BASE_BATCH * 12];
        let copied = memory.copy_records(
            RecordSources::Pointers(&self.pointers[..count]),
            &BASE_LAYOUT,
            &mut bytes[..count * 12],
        );
        self.pointers.fill(0);
        if copied != count * BASE_FIELDS.len() {
            self.indices.fill(0);
            return Err("naval movement nearest-base identity or province unreadable");
        }
        for (index, record) in bytes[..count * 12].chunks_exact(12).enumerate() {
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected {
                self.indices.fill(0);
                return Err("naval movement nearest-base identity mismatch");
            }
            let id = u32::from_le_bytes(record[8..12].try_into().unwrap());
            if id as usize >= province_slots {
                self.indices.fill(0);
                return Err("naval movement nearest-base province outside map domain");
            }
            output[self.indices[index]] = Some(id);
        }
        self.indices.fill(0);
        Ok(())
    }
}

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("naval movement address overflow")
}

fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("naval movement pointer unreadable")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, stride)
        .ok_or("naval movement array unreadable or invalid")
}

fn vtable(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rva: usize,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(rva)
        .ok_or("naval movement module overflow")? as u64;
    if object == 0 || read::<u64>(memory, object) != Some(expected) {
        return Err("naval movement identity mismatch");
    }
    Ok(())
}

fn capture(required: usize, len: Result<usize, &'static str>) -> DomainCapture {
    DomainCapture {
        len,
        required,
        first_failure: None,
    }
}

/// Independent header floor; a refusal never erases earlier known demand.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HeaderDemand {
    pub required: usize,
    pub first_failure: Option<&'static str>,
}

impl HeaderDemand {
    fn observe(&mut self, count: Result<usize, &'static str>) {
        match count {
            Ok(count) => match self.required.checked_add(count) {
                Some(required) => self.required = required,
                None => {
                    self.first_failure
                        .get_or_insert("naval movement demand overflow");
                }
            },
            Err(reason) => {
                self.first_failure.get_or_insert(reason);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Whole physical navy manager geometry, including slot zero.
    pub navy_slots: Result<usize, &'static str>,
    /// Conservative positive province slot bound, including nullable slots.
    pub provinces: Result<usize, &'static str>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NavyDemand {
    pub nearest_bases: HeaderDemand,
    pub foreign_navy_slots: HeaderDemand,
    pub supply_hubs: HeaderDemand,
}

/// Off-thread conservative sizing over live headers, never payload guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveDemand {
    pub bounds: Bounds,
    pub navy: NavyDemand,
    pub theatres: HeaderDemand,
    pub theatre_areas: HeaderDemand,
}

#[derive(Debug, Clone, Copy)]
struct Manager {
    pointer: usize,
    navies: Array,
}

/// Authenticated owner context. Reserved zero never passes positive link APIs.
#[derive(Debug, Clone, Copy)]
pub enum Owner {
    Positive(ccountry::Country),
    Reserved(ReservedOwner),
}

/// Constructed only by [`Reader::reserved_owner`], from actual country slot0.
#[derive(Debug, Clone, Copy)]
pub struct ReservedOwner {
    pointer: usize,
}

impl Owner {
    /// Clear a borrowed reserved pointer before the acquisition lease ends.
    pub fn clear(&mut self) {
        *self = Self::Reserved(ReservedOwner { pointer: 0 });
    }

    fn pointer(self) -> Result<usize, &'static str> {
        match self {
            Self::Positive(country)
                if country.pointer != 0 && country.id != 0 && country.canonical_slot != 0 =>
            {
                Ok(country.pointer)
            }
            Self::Reserved(country) if country.pointer != 0 => Ok(country.pointer),
            _ => Err("naval movement owner is not authenticated positive country"),
        }
    }

    fn accepts<M: Memory>(
        self,
        countries: Option<&ccountry::Reader<'_, M>>,
        raw: u32,
    ) -> Result<(), &'static str> {
        match self {
            Self::Positive(country) => {
                let countries =
                    countries.ok_or("naval movement positive country root unavailable")?;
                if countries.resolve(raw)?.pointer != country.pointer {
                    return Err("naval movement theatre owner mismatch");
                }
            }
            Self::Reserved(_) if raw != 0 => {
                return Err("reserved naval movement theatre owner is not zero");
            }
            Self::Reserved(_) => {}
        }
        Ok(())
    }
}

/// One frozen pass's independently observed shared roots.
pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    manager: Result<Option<Manager>, &'static str>,
    provinces: Result<Array, &'static str>,
    reserved: Result<ReservedOwner, &'static str>,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, state: usize) -> Self {
        let manager = (|| {
            let manager = pointer(memory, at(state, cgame_state::STRATEGIC_NAVY_MANAGER)?)?;
            if manager == 0 {
                return Ok(None);
            }
            vtable(memory, base, manager, cstrategic_navy_manager::VTABLE_RVA)?;
            Ok(Some(Manager {
                pointer: manager,
                navies: array(memory, manager, cstrategic_navy_manager::NAVIES, 8)?,
            }))
        })();
        let provinces = array(memory, state, cgame_state::PROVINCES, 8).and_then(|array| {
            if array.count == 0 {
                Err("naval movement province array has no zero slot")
            } else {
                Ok(array)
            }
        });
        let reserved =
            ccountry::read_reserved_root(memory, base, state).map(|country| ReservedOwner {
                pointer: country.pointer,
            });
        Self {
            memory,
            base,
            manager,
            provinces,
            reserved,
        }
    }

    pub fn bounds(&self) -> Bounds {
        Bounds {
            navy_slots: self
                .manager
                .map(|manager| manager.map_or(0, |manager| manager.navies.count as usize)),
            provinces: self.provinces.map(|array| array.count as usize - 1),
        }
    }

    pub fn reserved_owner(&self) -> Result<Owner, &'static str> {
        self.reserved.map(Owner::Reserved)
    }

    pub fn clear(&mut self) {
        self.manager = Err("naval movement pass cleared");
        self.provinces = Err("naval movement pass cleared");
        self.reserved = Err("naval movement pass cleared");
    }

    /// Read the exact physical manager slot; foreign slots are not links.
    /// Construction fills physical slots 1..count and leaves zero null.
    pub fn observe_navy(&self, slot: u32) -> Result<NavyInputs, &'static str> {
        let Some(manager) = self.manager? else {
            return Ok(NavyInputs::absent(slot));
        };
        // The strategic-navy lookup returns null when the manager array is empty.
        if manager.navies.count == 0 {
            return Ok(NavyInputs::absent(slot));
        }
        let entry = manager
            .navies
            .element(slot, 8)
            .ok_or("naval movement physical navy slot outside manager")?;
        let navy = pointer(self.memory, entry)?;
        if navy == 0 {
            return Ok(NavyInputs::absent(slot));
        }
        let mut primary = [0; 8];
        let mut secondary = [0; 8];
        let mut backlink = [0; 8];
        if self.memory.copy_scattered(
            [
                navy,
                at(navy, cstrategic_navy::SECONDARY)?,
                at(navy, MANAGER)?,
            ],
            &mut [&mut primary, &mut secondary, &mut backlink],
        ) != 3
        {
            return Err("naval movement navy identity unreadable");
        }
        let expected = self
            .base
            .checked_add(cstrategic_navy::VTABLE_RVA)
            .ok_or("naval movement module overflow")? as u64;
        let secondary_expected = self
            .base
            .checked_add(cstrategic_navy::SECONDARY_VTABLE_RVA)
            .ok_or("naval movement module overflow")? as u64;
        if u64::from_le_bytes(primary) != expected
            || u64::from_le_bytes(secondary) != secondary_expected
            || u64::from_le_bytes(backlink) != manager.pointer as u64
        {
            return Err("naval movement navy identity or manager mismatch");
        }
        // Each header is independent. A failed header is retained, not retried.
        Ok(NavyInputs {
            slot,
            present: true,
            launch_navy: Some(navy),
            ledger_navy: Some(navy),
            original_owner: None,
            ledgers: Some(array(
                self.memory,
                navy,
                cstrategic_navy::REGION_MISSIONS,
                cstrategic_navy::REGION_MISSIONS_STRIDE,
            )),
            access: Some(array(self.memory, navy, cstrategic_navy::ACCESS_MODES, 1)),
            nearest: Some(array(self.memory, navy, NEAREST_BASES, 8)),
            foreign: Some(array(self.memory, navy, FOREIGN_NAVY_SLOTS, 4)),
            hubs: Some(array(self.memory, navy, SUPPLY_HUBS, 4)),
        })
    }

    /// Shared actual controller-area census. O(P) reads, then O(P log P)
    /// in-place sort. Opaque area pointers are equality inputs, not dereferenced.
    pub fn capture_controller_area_domain(
        &self,
        output: &mut [ControllerAreaProvince],
    ) -> DomainCapture {
        self.capture_controller_area_domain_buffer(crate::walk::Buffer::Fixed(output))
    }

    pub fn capture_controller_area_domain_growing(
        &self,
        output: &mut Vec<ControllerAreaProvince>,
    ) -> DomainCapture {
        self.capture_controller_area_domain_buffer(crate::walk::Buffer::Growing(output))
    }

    pub fn capture_controller_area_domain_buffer(
        &self,
        mut output: crate::walk::Buffer<'_, ControllerAreaProvince>,
    ) -> DomainCapture {
        let provinces = match self.provinces {
            Ok(array) => array,
            Err(reason) => return capture(0, Err(reason)),
        };
        let required = provinces.count as usize - 1;
        if let Err(reason) = output.ensure(required) {
            return capture(required, Err(reason));
        }
        if output.len() < required {
            return capture(
                required,
                Err("naval movement province-area arena exhausted"),
            );
        }
        let mut len = 0;
        output[..required].fill(ControllerAreaProvince::default());
        let mut batch = ProvinceBatch::new();
        let result = (|| {
            let expected = self
                .base
                .checked_add(cprovince::VTABLE_RVA)
                .ok_or("naval movement module overflow")? as u64;
            let positive = provinces.slice(1, provinces.count - 1, 8)?;
            cpdx_array::pointers(
                self.memory,
                positive,
                "naval movement province pointer unreadable",
                |slot, province| {
                    if province == 0 {
                        return Ok(());
                    }
                    batch.pointers[batch.len] = province;
                    batch.slots[batch.len] = slot + 1;
                    batch.len += 1;
                    if batch.len == BASE_BATCH {
                        batch.flush(self.memory, expected, &mut output, &mut len)?;
                    }
                    Ok(())
                },
            )?;
            batch.flush(self.memory, expected, &mut output, &mut len)?;
            output[..len].sort_unstable_by_key(|entry| (entry.area, entry.province));
            Ok(len)
        })();
        batch.pointers.fill(0);
        batch.slots.fill(0);
        if result.is_err() {
            output[..required].fill(ControllerAreaProvince::default());
        }
        capture(required, result)
    }

    /// Capture ALL owned theatre headers, with no volunteer/status filter.
    /// A short header arena still visits live headers for conservative demand,
    /// but refuses the entire projection and clears every acquired control.
    pub fn capture_theatre_headers(
        &self,
        countries: &ccountry::Reader<'_, M>,
        owner: Owner,
        output: &mut [TheatreInput],
    ) -> TheatreHeaderCapture {
        self.capture_theatre_headers_optional(Some(countries), owner, output)
    }

    /// Reserved raw-zero ownership requires no positive country/link census.
    /// Positive ownership still resolves its raw reference through that root.
    pub fn capture_theatre_headers_optional(
        &self,
        countries: Option<&ccountry::Reader<'_, M>>,
        owner: Owner,
        output: &mut [TheatreInput],
    ) -> TheatreHeaderCapture {
        self.capture_theatre_headers_optional_buffer(
            countries,
            owner,
            crate::walk::Buffer::Fixed(output),
        )
    }

    pub fn capture_theatre_headers_optional_growing(
        &self,
        countries: Option<&ccountry::Reader<'_, M>>,
        owner: Owner,
        output: &mut Vec<TheatreInput>,
    ) -> TheatreHeaderCapture {
        self.capture_theatre_headers_optional_buffer(
            countries,
            owner,
            crate::walk::Buffer::Growing(output),
        )
    }

    pub fn capture_theatre_headers_optional_buffer(
        &self,
        countries: Option<&ccountry::Reader<'_, M>>,
        owner: Owner,
        mut output: crate::walk::Buffer<'_, TheatreInput>,
    ) -> TheatreHeaderCapture {
        for input in output.iter_mut() {
            input.clear();
        }
        let theatres = match owner
            .pointer()
            .and_then(|country| array(self.memory, country, ccountry::THEATRES, 8))
        {
            Ok(array) => array,
            Err(reason) => {
                return TheatreHeaderCapture {
                    len: Err(reason),
                    required_headers: 0,
                    areas: HeaderDemand {
                        required: 0,
                        first_failure: Some(reason),
                    },
                };
            }
        };
        let required_headers = theatres.count as usize;
        // Theatre-membership lookup reaches every original owned header.
        if let Err(reason) = output.ensure(required_headers) {
            return TheatreHeaderCapture {
                len: Err(reason),
                required_headers,
                areas: HeaderDemand {
                    required: 0,
                    first_failure: Some(reason),
                },
            };
        }
        let mut areas = HeaderDemand::default();
        let mut written = 0;
        let result = cpdx_array::pointers(
            self.memory,
            theatres,
            "naval movement theatre pointer unreadable",
            |index, theatre| {
                let input = self.theatre_header(countries, owner, theatre)?;
                areas.observe(Ok(input
                    .areas
                    .as_ref()
                    .and_then(|array| array.as_ref().ok())
                    .map_or(0, |array| array.count as usize)));
                if let Some(Err(reason)) = input.areas.as_ref() {
                    areas.first_failure.get_or_insert(*reason);
                }
                if let Some(destination) = output.get_mut(index as usize) {
                    *destination = input;
                    written = index as usize + 1;
                }
                Ok(())
            },
        )
        .and({
            if output.len() < required_headers {
                Err("naval movement theatre header arena exhausted")
            } else {
                Ok(required_headers)
            }
        });
        if let Err(reason) = result {
            areas.first_failure.get_or_insert(reason);
            for input in &mut output[..written] {
                input.clear();
            }
        }
        TheatreHeaderCapture {
            len: result,
            required_headers,
            areas,
        }
    }

    fn theatre_header(
        &self,
        countries: Option<&ccountry::Reader<'_, M>>,
        owner: Owner,
        theatre: usize,
    ) -> Result<TheatreInput, &'static str> {
        if theatre == 0 {
            return Err("naval movement theatre is null");
        }
        let mut identity = [0; 16];
        let mut raw_owner = [0; 4];
        if self.memory.copy_scattered(
            [theatre, at(theatre, ctheatre::OWNER)?],
            &mut [&mut identity, &mut raw_owner],
        ) != 2
        {
            return Err("naval movement theatre identity unreadable");
        }
        let expected = self
            .base
            .checked_add(ctheatre::VTABLE_RVA)
            .ok_or("naval movement module overflow")? as u64;
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected
            || handle as u32 != ctheatre::REFERENCE_TYPE
            || (handle >> 32) == 0
        {
            return Err("naval movement theatre identity mismatch");
        }
        owner.accepts(countries, u32::from_le_bytes(raw_owner))?;
        Ok(TheatreInput {
            reference: handle,
            areas: Some(array(
                self.memory,
                theatre,
                ctheatre::CONTROLLER_AREA_DATA,
                8,
            )),
        })
    }

    /// Project equality against the complete domain, including null area
    /// entries. Scratch/output is reused per owner: O(owner*P) final scans.
    pub fn capture_theatre_membership(
        &self,
        headers: &mut [TheatreInput],
        domain: &[ControllerAreaProvince],
        marks: &mut [u8],
        output: &mut [u32],
    ) -> TheatreMembershipCapture {
        self.capture_theatre_membership_buffer(
            headers,
            domain,
            crate::walk::Buffer::Fixed(marks),
            crate::walk::Buffer::Fixed(output),
        )
    }

    pub fn capture_theatre_membership_growing(
        &self,
        headers: &mut [TheatreInput],
        domain: &[ControllerAreaProvince],
        marks: &mut Vec<u8>,
        output: &mut Vec<u32>,
    ) -> TheatreMembershipCapture {
        self.capture_theatre_membership_buffer(
            headers,
            domain,
            crate::walk::Buffer::Growing(marks),
            crate::walk::Buffer::Growing(output),
        )
    }

    pub fn capture_theatre_membership_buffer(
        &self,
        headers: &mut [TheatreInput],
        domain: &[ControllerAreaProvince],
        mut marks: crate::walk::Buffer<'_, u8>,
        mut output: crate::walk::Buffer<'_, u32>,
    ) -> TheatreMembershipCapture {
        let mut null_controller_area = false;
        let mut written = 0;
        let mut required = domain.len();
        let result = (|| {
            marks.ensure(domain.len())?;
            if marks.len() < domain.len() {
                return Err("naval movement theatre mark arena exhausted");
            }
            if domain
                .windows(2)
                .any(|pair| (pair[0].area, pair[0].province) >= (pair[1].area, pair[1].province))
                || domain.iter().any(|entry| entry.province == 0)
            {
                return Err("naval movement province-area domain invalid");
            }
            marks[..domain.len()].fill(0);
            for input in headers.iter_mut() {
                let areas = input.areas.take().ok_or(CONSUMED)??;
                cpdx_array::pointers(
                    self.memory,
                    areas,
                    "naval movement theatre area pointer unreadable",
                    |_, area| {
                        null_controller_area |= area == 0;
                        let first = domain.partition_point(|entry| entry.area < area);
                        let last = domain.partition_point(|entry| entry.area <= area);
                        marks[first..last].fill(1);
                        Ok(())
                    },
                )?;
            }
            required = marks[..domain.len()]
                .iter()
                .filter(|&&mark| mark != 0)
                .count();
            output.ensure(required)?;
            if output.len() < required {
                return Err(THEATRE_PROVINCE_CAPACITY);
            }
            for (entry, &mark) in domain.iter().zip(&marks[..domain.len()]) {
                if mark != 0 {
                    output[written] = entry.province;
                    written += 1;
                }
            }
            output[..written].sort_unstable();
            Ok(written)
        })();
        for input in headers {
            input.clear();
        }
        marks.fill(0);
        if result.is_err() {
            output[..written].fill(0);
            null_controller_area = false;
        }
        TheatreMembershipCapture {
            provinces: capture(required, result),
            null_controller_area,
        }
    }
}

impl<M: Memory> Drop for Reader<'_, M> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Borrowed current-pass equality domain, never a serialized native pointer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControllerAreaProvince {
    pub province: u32,
    area: usize,
    owner: Option<usize>,
    pointer: usize,
}

impl ControllerAreaProvince {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Default)]
pub struct TheatreInput {
    reference: u64,
    areas: Option<Result<Array, &'static str>>,
}

impl TheatreInput {
    /// Authenticated ordered roster member; projection does not reconstruct
    /// this identity. Zero denotes cleared/unobserved input, not a theatre.
    pub fn reference(&self) -> u64 {
        self.reference
    }

    pub fn clear(&mut self) {
        self.reference = 0;
        self.areas = None;
    }
}

impl Drop for TheatreInput {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TheatreHeaderCapture {
    pub len: Result<usize, &'static str>,
    pub required_headers: usize,
    /// Full known area-count floor, including a capacity refusal.
    pub areas: HeaderDemand,
}

#[derive(Debug, Clone, Copy)]
pub struct TheatreMembershipCapture {
    pub provinces: DomainCapture,
    /// Valid only when `provinces.len` succeeds.
    pub null_controller_area: bool,
}

impl TheatreMembershipCapture {
    /// Exact count even with an intentionally empty destination. Other
    /// refusals expose only a conservative bound and cannot supply a count.
    pub fn exact_province_count(&self) -> Result<usize, &'static str> {
        match self.provinces.len {
            Ok(len) => Ok(len),
            Err(THEATRE_PROVINCE_CAPACITY) => Ok(self.provinces.required),
            Err(reason) => Err(reason),
        }
    }
}

/// Independent native arrays behind one authenticated physical navy slot.
/// Do not reconstruct foreign slots by resolving them as country references.
pub struct NavyInputs {
    pub slot: u32,
    pub present: bool,
    // Already authenticated manager selection, never serialized or reacquired.
    launch_navy: Option<usize>,
    ledger_navy: Option<usize>,
    original_owner: Option<Result<u32, &'static str>>,
    ledgers: Option<Result<Array, &'static str>>,
    access: Option<Result<Array, &'static str>>,
    nearest: Option<Result<Array, &'static str>>,
    foreign: Option<Result<Array, &'static str>>,
    hubs: Option<Result<Array, &'static str>>,
}

impl NavyInputs {
    fn absent(slot: u32) -> Self {
        let empty = Ok(Array::default());
        Self {
            slot,
            present: false,
            launch_navy: Some(0),
            ledger_navy: Some(0),
            original_owner: None,
            ledgers: Some(empty),
            access: Some(empty),
            nearest: Some(empty),
            foreign: Some(empty),
            hubs: Some(empty),
        }
    }

    pub fn demand(&self) -> NavyDemand {
        let floor = |source: &Option<Result<Array, &'static str>>| {
            let mut demand = HeaderDemand::default();
            demand.observe(source.as_ref().ok_or(CONSUMED).and_then(|source| {
                source
                    .as_ref()
                    .map(|array| array.count as usize)
                    .map_err(|&error| error)
            }));
            demand
        };
        NavyDemand {
            nearest_bases: floor(&self.nearest),
            foreign_navy_slots: floor(&self.foreign),
            supply_hubs: floor(&self.hubs),
        }
    }

    pub fn clear(&mut self) {
        self.launch_navy = None;
        self.ledger_navy = None;
        self.original_owner = None;
        self.ledgers = None;
        self.access = None;
        self.nearest = None;
        self.foreign = None;
        self.hubs = None;
    }

    /// Same authenticated physical navy, borrowed only inside its frozen pass.
    /// Mission ledgers and presence share this control; launch consumption does
    /// not retire it. The source owner clears it before returning the callback.
    pub fn ledger_navy(&self) -> Result<usize, &'static str> {
        self.ledger_navy.ok_or(CONSUMED)
    }

    /// Strict icon owner authentication on this SAME physical control. The
    /// transport null/alias domain does not require or weaken this observation.
    pub fn observe_owner(&mut self, memory: &impl Memory) -> Result<u32, &'static str> {
        if let Some(original) = self.original_owner {
            return original;
        }
        let original = self.ledger_navy().and_then(|navy| {
            if navy == 0 {
                return Err("country navy absent or unreadable");
            }
            let address = navy
                .checked_add(cstrategic_navy::OWNER)
                .ok_or("navy owner overflow")?;
            read(memory, address).ok_or("navy owner unreadable")
        });
        self.original_owner = Some(original);
        original
    }

    /// Consume the original outer header once; absence is the native empty list.
    pub fn take_ledger_regions(&mut self) -> Result<Array, &'static str> {
        self.ledgers.take().ok_or(CONSUMED)?
    }

    /// The same original physical navy's current +B0/+BC observation is indexed
    /// and searched for modes 1/2. A failed initial header stays failed and is
    /// never reacquired.
    pub fn take_access_modes(&mut self) -> Result<Array, &'static str> {
        self.access.take().ok_or(CONSUMED)?
    }

    /// Demand from the held original header, including a control that cannot
    /// fit its owner's arena. This does not consume or reacquire the header.
    pub fn access_mode_count(&self) -> Option<usize> {
        self.access
            .as_ref()?
            .as_ref()
            .ok()
            .map(|array| array.count as usize)
    }

    /// Consume the independently reached launch provider. Native null is an
    /// explicit observation; an empty list exits before the owner read.
    /// Movement header/province faults are not prerequisites.
    pub fn capture_launch(
        &mut self,
        memory: &impl Memory,
        base: usize,
        scratch: &mut [ccountry::launch_sources::Occurrence],
        output: &mut [ccountry::launch_sources::LaunchNavalBase],
    ) -> ccountry::launch_sources::NavalCapture {
        self.capture_launch_buffer(
            memory,
            base,
            crate::walk::Buffer::Fixed(scratch),
            crate::walk::Buffer::Fixed(output),
        )
    }

    pub fn capture_launch_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        scratch: &mut Vec<ccountry::launch_sources::Occurrence>,
        output: &mut Vec<ccountry::launch_sources::LaunchNavalBase>,
    ) -> ccountry::launch_sources::NavalCapture {
        self.capture_launch_buffer(
            memory,
            base,
            crate::walk::Buffer::Growing(scratch),
            crate::walk::Buffer::Growing(output),
        )
    }

    pub fn capture_launch_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        scratch: crate::walk::Buffer<'_, ccountry::launch_sources::Occurrence>,
        output: crate::walk::Buffer<'_, ccountry::launch_sources::LaunchNavalBase>,
    ) -> ccountry::launch_sources::NavalCapture {
        match self.launch_navy.take() {
            Some(navy) => {
                ccountry::launch_sources::capture_naval_buffer(memory, base, navy, scratch, output)
            }
            None => ccountry::launch_sources::NavalCapture {
                required: 0,
                provider: Err("reserved launch navy control consumed"),
            },
        }
    }

    /// Preserve native nonnull base-ID0 and stale cache entries. Identity is
    /// vtable/+10 only: no current map, holder, level or owned-base filter.
    pub fn capture_nearest(
        &mut self,
        memory: &impl Memory,
        base: usize,
        province_slots: usize,
        output: &mut [Option<u32>],
    ) -> DomainCapture {
        self.capture_nearest_buffer(
            memory,
            base,
            province_slots,
            crate::walk::Buffer::Fixed(output),
        )
    }

    pub fn capture_nearest_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        province_slots: usize,
        output: &mut Vec<Option<u32>>,
    ) -> DomainCapture {
        self.capture_nearest_buffer(
            memory,
            base,
            province_slots,
            crate::walk::Buffer::Growing(output),
        )
    }

    pub fn capture_nearest_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        province_slots: usize,
        mut output: crate::walk::Buffer<'_, Option<u32>>,
    ) -> DomainCapture {
        let source = match self.nearest.take().ok_or(CONSUMED).and_then(|array| array) {
            Ok(source) => source,
            Err(reason) => return capture(0, Err(reason)),
        };
        let required = source.count as usize;
        if let Err(reason) = output.ensure(required) {
            return capture(required, Err(reason));
        }
        if output.len() < required {
            return capture(required, Err("naval movement nearest-base arena exhausted"));
        }
        output[..required].fill(None);
        let mut batch = BaseBatch::new();
        let result = (|| {
            let expected = base
                .checked_add(cnaval_base::VTABLE_RVA)
                .ok_or("naval movement module overflow")? as u64;
            cpdx_array::pointers(
                memory,
                source,
                "naval movement nearest-base pointer unreadable",
                |index, navy_base| {
                    if navy_base == 0 {
                        return Ok(());
                    }
                    batch.pointers[batch.len] = navy_base;
                    batch.indices[batch.len] = index as usize;
                    batch.len += 1;
                    if batch.len == BASE_BATCH {
                        batch.flush(memory, expected, province_slots, &mut output)?;
                    }
                    Ok(())
                },
            )?;
            batch.flush(memory, expected, province_slots, &mut output)?;
            Ok(required)
        })();
        batch.pointers.fill(0);
        batch.indices.fill(0);
        if result.is_err() {
            output[..required].fill(None);
        }
        capture(required, result)
    }

    fn copy_list(
        source: &mut Option<Result<Array, &'static str>>,
        memory: &impl Memory,
        mut output: crate::walk::Buffer<'_, u32>,
        capacity: &'static str,
        unreadable: &'static str,
    ) -> DomainCapture {
        let source = match source.take().ok_or(CONSUMED).and_then(|array| array) {
            Ok(source) => source,
            Err(reason) => return capture(0, Err(reason)),
        };
        let required = source.count as usize;
        if let Err(reason) = output.ensure(required) {
            return capture(required, Err(reason));
        }
        if output.len() < required {
            return capture(required, Err(capacity));
        }
        let result = cpdx_array::copy_u32s(memory, source, &mut output[..required], unreadable)
            .map(|()| required);
        if result.is_err() {
            output[..required].fill(0);
        }
        capture(required, result)
    }

    pub fn capture_foreign_navy_slots(
        &mut self,
        memory: &impl Memory,
        output: &mut [u32],
    ) -> DomainCapture {
        self.capture_foreign_navy_slots_buffer(memory, crate::walk::Buffer::Fixed(output))
    }

    pub fn capture_foreign_navy_slots_growing(
        &mut self,
        memory: &impl Memory,
        output: &mut Vec<u32>,
    ) -> DomainCapture {
        self.capture_foreign_navy_slots_buffer(memory, crate::walk::Buffer::Growing(output))
    }

    pub fn capture_foreign_navy_slots_buffer(
        &mut self,
        memory: &impl Memory,
        output: crate::walk::Buffer<'_, u32>,
    ) -> DomainCapture {
        Self::copy_list(
            &mut self.foreign,
            memory,
            output,
            "naval movement foreign-slot arena exhausted",
            "naval movement foreign-slot payload unreadable",
        )
    }

    pub fn capture_supply_hubs(
        &mut self,
        memory: &impl Memory,
        output: &mut [u32],
    ) -> DomainCapture {
        self.capture_supply_hubs_buffer(memory, crate::walk::Buffer::Fixed(output))
    }

    pub fn capture_supply_hubs_growing(
        &mut self,
        memory: &impl Memory,
        output: &mut Vec<u32>,
    ) -> DomainCapture {
        self.capture_supply_hubs_buffer(memory, crate::walk::Buffer::Growing(output))
    }

    pub fn capture_supply_hubs_buffer(
        &mut self,
        memory: &impl Memory,
        output: crate::walk::Buffer<'_, u32>,
    ) -> DomainCapture {
        Self::copy_list(
            &mut self.hubs,
            memory,
            output,
            "naval movement hub arena exhausted",
            "naval movement hub payload unreadable",
        )
    }
}

impl Drop for NavyInputs {
    fn drop(&mut self) {
        self.clear();
    }
}

mod demand;
pub use demand::{DemandSlot, DemandedDemand};
#[cfg(test)]
mod tests;
