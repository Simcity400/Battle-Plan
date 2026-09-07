use super::country_roster_readset::{Budget, OpenedRead};
use compact_contract::country_roster::Cause;
use std::alloc::{alloc, Layout};

const COUNTRY_LIMIT: usize = 65_536;
const COUNTRY_SLOT_LIMIT: usize = 10_000;
const SLOT_LIMIT: usize = 20_000;
const READ_LIMIT: usize = 5_297_152;
const TRACE_BLOCK: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    HeaderPointer, HeaderCapacity, HeaderCount, VectorSlot, PrimaryVtable, Class,
    ReferenceVtable, Reference, Owner, RegistryRoot, RegistrySlots, RegistryMask,
    RegistryExtra, ProbeDistance, ProbeId, ProbeType, Payload,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    NotRead, AddressOverflow, NullPointer(Field), Unreadable(Field), Changed(Field),
    NegativeCapacity, NegativeCount, CountExceedsCapacity, CountryBound, SlotBound,
    ReadBound, RetainedBound, AllocationFailed, PrimaryVtable, Class, ReferenceVtable,
    OwnerMismatch, UninitializedReference, NullRegistry, MissingKey, Sentinel,
    NullPayload, DifferentPayload, ProbeBound, DuplicatePointer, DuplicateReference,
}
impl Refusal {
    fn wire_cause(self) -> Cause {
        match self {
            Self::NotRead => Cause::RailwayNotRead,
            Self::AddressOverflow => Cause::RailwayAddressOverflow,
            Self::NegativeCapacity => Cause::RailwayNegativeCapacity,
            Self::NegativeCount => Cause::RailwayNegativeCount,
            Self::CountExceedsCapacity => Cause::RailwayCountExceedsCapacity,
            Self::CountryBound => Cause::RailwayCountryBound,
            Self::SlotBound => Cause::RailwaySlotBound,
            Self::ReadBound => Cause::RailwayReadBound,
            Self::RetainedBound => Cause::RailwayRetainedBound,
            Self::AllocationFailed => Cause::RailwayAllocationFailed,
            Self::PrimaryVtable => Cause::RailwayPrimaryVtable,
            Self::Class => Cause::RailwayClass,
            Self::ReferenceVtable => Cause::RailwayReferenceVtable,
            Self::OwnerMismatch => Cause::RailwayOwnerMismatch,
            Self::UninitializedReference => Cause::RailwayUninitializedReference,
            Self::NullRegistry => Cause::RailwayNullRegistry,
            Self::MissingKey => Cause::RailwayMissingKey,
            Self::Sentinel => Cause::RailwaySentinel,
            Self::NullPayload => Cause::RailwayNullPayload,
            Self::DifferentPayload => Cause::RailwayDifferentPayload,
            Self::ProbeBound => Cause::RailwayProbeBound,
            Self::DuplicatePointer => Cause::RailwayDuplicatePointer,
            Self::DuplicateReference => Cause::RailwayDuplicateReference,
            Self::NullPointer(Field::HeaderPointer) => Cause::RailwayNullPointerHeaderPointer,
            Self::NullPointer(Field::HeaderCapacity) => Cause::RailwayNullPointerHeaderCapacity,
            Self::NullPointer(Field::HeaderCount) => Cause::RailwayNullPointerHeaderCount,
            Self::NullPointer(Field::VectorSlot) => Cause::RailwayNullPointerVectorSlot,
            Self::NullPointer(Field::PrimaryVtable) => Cause::RailwayNullPointerPrimaryVtable,
            Self::NullPointer(Field::Class) => Cause::RailwayNullPointerClass,
            Self::NullPointer(Field::ReferenceVtable) => Cause::RailwayNullPointerReferenceVtable,
            Self::NullPointer(Field::Reference) => Cause::RailwayNullPointerReference,
            Self::NullPointer(Field::Owner) => Cause::RailwayNullPointerOwner,
            Self::NullPointer(Field::RegistryRoot) => Cause::RailwayNullPointerRegistryRoot,
            Self::NullPointer(Field::RegistrySlots) => Cause::RailwayNullPointerRegistrySlots,
            Self::NullPointer(Field::RegistryMask) => Cause::RailwayNullPointerRegistryMask,
            Self::NullPointer(Field::RegistryExtra) => Cause::RailwayNullPointerRegistryExtra,
            Self::NullPointer(Field::ProbeDistance) => Cause::RailwayNullPointerProbeDistance,
            Self::NullPointer(Field::ProbeId) => Cause::RailwayNullPointerProbeId,
            Self::NullPointer(Field::ProbeType) => Cause::RailwayNullPointerProbeType,
            Self::NullPointer(Field::Payload) => Cause::RailwayNullPointerPayload,
            Self::Unreadable(Field::HeaderPointer) => Cause::RailwayUnreadableHeaderPointer,
            Self::Unreadable(Field::HeaderCapacity) => Cause::RailwayUnreadableHeaderCapacity,
            Self::Unreadable(Field::HeaderCount) => Cause::RailwayUnreadableHeaderCount,
            Self::Unreadable(Field::VectorSlot) => Cause::RailwayUnreadableVectorSlot,
            Self::Unreadable(Field::PrimaryVtable) => Cause::RailwayUnreadablePrimaryVtable,
            Self::Unreadable(Field::Class) => Cause::RailwayUnreadableClass,
            Self::Unreadable(Field::ReferenceVtable) => Cause::RailwayUnreadableReferenceVtable,
            Self::Unreadable(Field::Reference) => Cause::RailwayUnreadableReference,
            Self::Unreadable(Field::Owner) => Cause::RailwayUnreadableOwner,
            Self::Unreadable(Field::RegistryRoot) => Cause::RailwayUnreadableRegistryRoot,
            Self::Unreadable(Field::RegistrySlots) => Cause::RailwayUnreadableRegistrySlots,
            Self::Unreadable(Field::RegistryMask) => Cause::RailwayUnreadableRegistryMask,
            Self::Unreadable(Field::RegistryExtra) => Cause::RailwayUnreadableRegistryExtra,
            Self::Unreadable(Field::ProbeDistance) => Cause::RailwayUnreadableProbeDistance,
            Self::Unreadable(Field::ProbeId) => Cause::RailwayUnreadableProbeId,
            Self::Unreadable(Field::ProbeType) => Cause::RailwayUnreadableProbeType,
            Self::Unreadable(Field::Payload) => Cause::RailwayUnreadablePayload,
            Self::Changed(Field::HeaderPointer) => Cause::RailwayChangedHeaderPointer,
            Self::Changed(Field::HeaderCapacity) => Cause::RailwayChangedHeaderCapacity,
            Self::Changed(Field::HeaderCount) => Cause::RailwayChangedHeaderCount,
            Self::Changed(Field::VectorSlot) => Cause::RailwayChangedVectorSlot,
            Self::Changed(Field::PrimaryVtable) => Cause::RailwayChangedPrimaryVtable,
            Self::Changed(Field::Class) => Cause::RailwayChangedClass,
            Self::Changed(Field::ReferenceVtable) => Cause::RailwayChangedReferenceVtable,
            Self::Changed(Field::Reference) => Cause::RailwayChangedReference,
            Self::Changed(Field::Owner) => Cause::RailwayChangedOwner,
            Self::Changed(Field::RegistryRoot) => Cause::RailwayChangedRegistryRoot,
            Self::Changed(Field::RegistrySlots) => Cause::RailwayChangedRegistrySlots,
            Self::Changed(Field::RegistryMask) => Cause::RailwayChangedRegistryMask,
            Self::Changed(Field::RegistryExtra) => Cause::RailwayChangedRegistryExtra,
            Self::Changed(Field::ProbeDistance) => Cause::RailwayChangedProbeDistance,
            Self::Changed(Field::ProbeId) => Cause::RailwayChangedProbeId,
            Self::Changed(Field::ProbeType) => Cause::RailwayChangedProbeType,
            Self::Changed(Field::Payload) => Cause::RailwayChangedPayload,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Reference { pub native_type: u32, pub id: u32 }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Accounting {
    pub reserved_bytes: usize,
    pub opening_attempted_bytes: usize,
    pub closing_attempted_bytes: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Header {
    pub pointer: Option<usize>,
    pub capacity: Option<i32>,
    pub count: Option<i32>,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    pub ordinal: u32,
    pub complete_object: Option<usize>,
    pub reference: Option<Reference>,
    pub read: Result<(), Refusal>,
    pub seal: Result<(), Refusal>,
    pub duplicate_pointer: bool,
    pub duplicate_reference: bool,
    registry: Option<usize>,
}
impl Row {
    fn unread(ordinal: u32) -> Self {
        Self { ordinal, complete_object: None, reference: None, read: Err(Refusal::NotRead),
            seal: Ok(()), duplicate_pointer: false, duplicate_reference: false, registry: None }
    }
}
// CAP must still seal the original country/class/table/tick bracket before wire admission.
pub(super) struct CountryObservation {
    pub owner_tag_db_id: u32,
    pub header: Header,
    pub read: Result<(), Refusal>,
    pub seal: Result<(), Refusal>,
    pub rows: Box<[Row]>,
    pub accounting: Accounting,
}
impl CountryObservation {
    pub(super) fn into_roster_country(
        self, physical_slot: u32, acquisition: compact_contract::country_roster::Acquisition,
        budget: &Budget,
    ) -> compact_contract::country_roster::Country {
        use compact_contract::country_roster::{Country, MemberRef, Slot};
        let mut country = Country {
            physical_slot, owner_tag_db_id: Some(self.owner_tag_db_id), acquisition,
            observed_count: self.header.count.and_then(|count| u32::try_from(count).ok()),
            failure: self.read.and(self.seal).err().map(Refusal::wire_cause), slots: Vec::new(),
        };
        let retained = self.rows.len().checked_mul(std::mem::size_of::<Slot>())
            .ok_or(Cause::RailwayRetainedBound).and_then(|bytes| budget.retain(bytes));
        if retained.is_err() {
            country.failure = Some(Cause::RailwayRetainedBound);
            return country;
        }
        if country.slots.try_reserve_exact(self.rows.len()).is_err() {
            country.failure = Some(Cause::RailwayAllocationFailed);
            return country;
        }
        for ordinal in 0..self.rows.len() {
            country.slots.push(Slot {
                ordinal: ordinal as u32,
                reference: self.locally_sealed_reference(ordinal)
                    .map(|reference| MemberRef { native_type: reference.native_type, id: reference.id })
                    .map_err(Refusal::wire_cause),
            });
        }
        country
    }

    pub(super) fn locally_sealed_reference(&self, ordinal: usize) -> Result<Reference, Refusal> {
        self.read?;
        self.seal?;
        let row = self.rows.get(ordinal).ok_or(Refusal::NotRead)?;
        row.seal?;
        if row.duplicate_pointer { return Err(Refusal::DuplicatePointer); }
        if row.duplicate_reference { return Err(Refusal::DuplicateReference); }
        row.read?;
        row.reference.ok_or(Refusal::NotRead)
    }
}

pub(super) fn refuse_cross_country_duplicates(
    observation: &mut compact_contract::country_roster::Observation, budget: &Budget,
) {
    use compact_contract::country_roster::MemberRef;
    let refuse = |observation: &mut compact_contract::country_roster::Observation, cause| {
        observation.failure = Some(cause);
        for country in &mut observation.countries {
            for slot in &mut country.slots {
                if slot.reference.is_ok() { slot.reference = Err(cause); }
            }
        }
    };
    let count = observation.countries.iter().try_fold(0usize, |count, country|
        count.checked_add(country.slots.len()));
    let Some(count) = count.filter(|count| *count <= SLOT_LIMIT) else {
        refuse(observation, Cause::RailwaySlotBound);
        return;
    };
    let Some(bytes) = count.checked_mul(std::mem::size_of::<(MemberRef, usize, usize)>()) else {
        refuse(observation, Cause::RailwayRetainedBound);
        return;
    };
    if budget.retain(bytes).is_err() {
        refuse(observation, Cause::RailwayRetainedBound);
        return;
    }
    let mut index = Vec::new();
    if index.try_reserve_exact(count).is_err() {
        refuse(observation, Cause::RailwayAllocationFailed);
        return;
    }
    for (country_index, country) in observation.countries.iter().enumerate() {
        for (slot_index, slot) in country.slots.iter().enumerate() {
            if let Ok(reference) = slot.reference { index.push((reference, country_index, slot_index)); }
        }
    }
    index.sort_unstable();
    let mut start = 0;
    while start < index.len() {
        let mut end = start + 1;
        while end < index.len() && index[end].0 == index[start].0 { end += 1; }
        if end - start > 1 {
            for &(_, country, slot) in &index[start..end] {
                observation.countries[country].slots[slot].reference = Err(Cause::RailwayDuplicateReference);
            }
        }
        start = end;
    }
}

// One instance per owning sweep; country/class/table/tick custody stays with that caller.
pub(super) struct RailwayReadset<'a> {
    budget: &'a Budget,
    countries: usize,
    slots: usize,
    reserved: usize,
    opening_stopped: Option<Refusal>,
}
impl<'a> RailwayReadset<'a> {
    pub(super) fn new(budget: &'a Budget) -> Self {
        Self { budget, countries: 0, slots: 0, reserved: 0, opening_stopped: None }
    }
    pub(super) fn read_country(
        &mut self, module_base: usize, country: usize, owner_tag_db_id: u32,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> CountryObservation {
        let mut out = CountryObservation { owner_tag_db_id, header: Header::default(),
            read: Ok(()), seal: Ok(()), rows: Box::default(), accounting: Accounting::default() };
        if self.countries >= COUNTRY_LIMIT {
            out.read = Err(Refusal::CountryBound);
            return out;
        }
        self.countries += 1;
        let mut trace = Trace::default();
        out.read = self.open_country(module_base, country, &mut out, &mut trace, read);
        trace.close(&mut out, read);
        out
    }
    fn field<const N: usize>(
        &mut self, address: usize, field: Field, owner: TraceOwner, trace: &mut Trace,
        accounting: &mut Accounting, read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; N], Refusal> {
        address.checked_add(N).ok_or(Refusal::AddressOverflow)?;
        if let Some(failure) = self.opening_stopped { return Err(failure); }
        let Some(reserved) = self.reserved.checked_add(N * 2).filter(|n| *n <= READ_LIMIT) else {
            self.opening_stopped = Some(Refusal::ReadBound);
            return Err(Refusal::ReadBound);
        };
        if let Err(failure) = trace.prepare(self.budget) {
            self.opening_stopped = Some(failure);
            return Err(failure);
        }
        let reservation = match self.budget.reserve_read(address, N) {
            Ok(reservation) => reservation,
            Err(_) => {
                self.opening_stopped = Some(Refusal::ReadBound);
                return Err(Refusal::ReadBound);
            }
        };
        self.reserved = reserved;
        accounting.reserved_bytes += N * 2;
        accounting.opening_attempted_bytes += N;
        let opening = reservation.open(read).map_err(|_| Refusal::Unreadable(field))?;
        let mut bytes = [0; N];
        bytes.copy_from_slice(opening.bytes());
        trace.push(TraceEntry { opening, field, owner });
        Ok(bytes)
    }
    fn open_country(
        &mut self, module_base: usize, country: usize, out: &mut CountryObservation,
        trace: &mut Trace, read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), Refusal> {
        if country == 0 { return Err(Refusal::NullPointer(Field::HeaderPointer)); }
        let pointer = u64::from_le_bytes(self.field::<8>(offset(country, 0x2a8)?,
            Field::HeaderPointer, TraceOwner::Country, trace, &mut out.accounting, read)?);
        let pointer = usize::try_from(pointer).map_err(|_| Refusal::AddressOverflow)?;
        out.header.pointer = Some(pointer);
        let capacity = i32::from_le_bytes(self.field::<4>(offset(country, 0x2b0)?,
            Field::HeaderCapacity, TraceOwner::Country, trace, &mut out.accounting, read)?);
        out.header.capacity = Some(capacity);
        let count = i32::from_le_bytes(self.field::<4>(offset(country, 0x2b4)?,
            Field::HeaderCount, TraceOwner::Country, trace, &mut out.accounting, read)?);
        out.header.count = Some(count);
        if capacity < 0 { return Err(Refusal::NegativeCapacity); }
        if count < 0 { return Err(Refusal::NegativeCount); }
        if count > capacity { return Err(Refusal::CountExceedsCapacity); }
        let count = count as usize;
        if count > COUNTRY_SLOT_LIMIT || self.slots.checked_add(count).filter(|n| *n <= SLOT_LIMIT).is_none() {
            return Err(Refusal::SlotBound);
        }
        self.slots += count;
        if count == 0 { return Ok(()); }
        out.rows = charged_rows(self.budget, count)?;
        let mut indices = charged_indices(self.budget, count)?;
        if pointer == 0 { return Err(Refusal::NullPointer(Field::VectorSlot)); }
        pointer.checked_add(count.checked_mul(8).ok_or(Refusal::AddressOverflow)?)
            .ok_or(Refusal::AddressOverflow)?;
        let mut cache = charged_box(self.budget, [None; 102])?;
        for ordinal in 0..count {
            let row = &mut out.rows[ordinal];
            row.read = self.open_row(module_base, pointer + ordinal * 8, out.owner_tag_db_id,
                row, &mut cache, trace, &mut out.accounting, read);
        }
        mark_duplicates(&mut out.rows, &mut indices);
        Ok(())
    }
    fn open_row(
        &mut self, module_base: usize, slot: usize, country_owner: u32, row: &mut Row,
        cache: &mut [Option<Result<Registry, Refusal>>; 102], trace: &mut Trace,
        accounting: &mut Accounting, read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), Refusal> {
        let owner = TraceOwner::Row(row.ordinal as usize);
        let body = u64::from_le_bytes(self.field::<8>(slot, Field::VectorSlot, owner, trace, accounting, read)?);
        let body = usize::try_from(body).map_err(|_| Refusal::AddressOverflow)?;
        row.complete_object = Some(body);
        if body == 0 { return Err(Refusal::NullPointer(Field::VectorSlot)); }
        let primary = u64::from_le_bytes(self.field::<8>(body, Field::PrimaryVtable, owner, trace, accounting, read)?);
        if primary != offset(module_base, 0x0295b600)? as u64 { return Err(Refusal::PrimaryVtable); }
        let class = u32::from_le_bytes(self.field::<4>(offset(body, 8)?, Field::Class, owner, trace, accounting, read)?);
        if class != 13 { return Err(Refusal::Class); }
        let secondary = u64::from_le_bytes(self.field::<8>(offset(body, 0x10)?, Field::ReferenceVtable, owner, trace, accounting, read)?);
        if secondary != offset(module_base, 0x0295b7d8)? as u64 { return Err(Refusal::ReferenceVtable); }
        let pair = self.field::<8>(offset(body, 0x18)?, Field::Reference, owner, trace, accounting, read)?;
        let reference = Reference { native_type: u32::from_le_bytes(pair[..4].try_into().unwrap()),
            id: u32::from_le_bytes(pair[4..].try_into().unwrap()) };
        row.reference = Some(reference);
        let observed_owner = u32::from_le_bytes(self.field::<4>(offset(body, 0x1d8)?, Field::Owner, owner, trace, accounting, read)?);
        if observed_owner != country_owner { return Err(Refusal::OwnerMismatch); }
        if reference.native_type == 0 && reference.id == 0 { return Err(Refusal::UninitializedReference); }
        let (index, root) = root_slot(module_base, reference.native_type)?;
        row.registry = Some(index);
        let registry = match cache[index] {
            Some(value) => value,
            None => {
                let value = self.open_registry(root, index, trace, accounting, read);
                cache[index] = Some(value);
                value
            }
        }?;
        self.lookup(registry, reference, offset(body, 0x10)?, owner, trace, accounting, read)
    }
    fn open_registry(
        &mut self, root: usize, index: usize, trace: &mut Trace, accounting: &mut Accounting,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<Registry, Refusal> {
        let owner = TraceOwner::Registry(index);
        let registry = u64::from_le_bytes(self.field::<8>(root, Field::RegistryRoot, owner, trace, accounting, read)?);
        let registry = usize::try_from(registry).map_err(|_| Refusal::AddressOverflow)?;
        if registry == 0 { return Err(Refusal::NullRegistry); }
        let slots = u64::from_le_bytes(self.field::<8>(offset(registry, 8)?, Field::RegistrySlots, owner, trace, accounting, read)?);
        let slots = usize::try_from(slots).map_err(|_| Refusal::AddressOverflow)?;
        let mask = u32::from_le_bytes(self.field::<4>(offset(registry, 0x14)?, Field::RegistryMask, owner, trace, accounting, read)?);
        let extra = self.field::<1>(offset(registry, 0x18)?, Field::RegistryExtra, owner, trace, accounting, read)?[0];
        if slots == 0 { return Err(Refusal::NullPointer(Field::RegistrySlots)); }
        Ok(Registry { slots, mask, extra })
    }
    fn lookup(
        &mut self, registry: Registry, reference: Reference, expected: usize, owner: TraceOwner,
        trace: &mut Trace, accounting: &mut Accounting,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), Refusal> {
        let hash = ((u64::from(reference.native_type) << 32) | u64::from(reference.id)).wrapping_mul(0x49631a45);
        let bucket = ((hash as u32 ^ (hash >> 32) as u32) & registry.mask) as i32;
        let mut slot = signed_slot(registry.slots, bucket)?;
        let sentinel = signed_slot(registry.slots, registry.mask.wrapping_add(u32::from(registry.extra) + 1) as i32)?;
        for probe in 1..=255u16 {
            let distance = self.field::<1>(offset(slot, 4)?, Field::ProbeDistance, owner, trace, accounting, read)?[0];
            if u16::from(distance) < probe { return Err(Refusal::MissingKey); }
            let id = u32::from_le_bytes(self.field::<4>(offset(slot, 0xc)?, Field::ProbeId, owner, trace, accounting, read)?);
            if id == reference.id {
                let native_type = u32::from_le_bytes(self.field::<4>(offset(slot, 8)?, Field::ProbeType, owner, trace, accounting, read)?);
                if native_type == reference.native_type {
                    if slot == sentinel { return Err(Refusal::Sentinel); }
                    let payload = u64::from_le_bytes(self.field::<8>(offset(slot, 0x10)?, Field::Payload, owner, trace, accounting, read)?);
                    if payload == 0 { return Err(Refusal::NullPayload); }
                    if payload != expected as u64 { return Err(Refusal::DifferentPayload); }
                    return Ok(());
                }
            }
            if probe != 255 { slot = offset(slot, 24)?; }
        }
        Err(Refusal::ProbeBound)
    }
}
#[derive(Clone, Copy)]
struct Registry { slots: usize, mask: u32, extra: u8 }
fn offset(base: usize, offset: usize) -> Result<usize, Refusal> {
    base.checked_add(offset).ok_or(Refusal::AddressOverflow)
}
fn signed_slot(base: usize, index: i32) -> Result<usize, Refusal> {
    base.checked_add_signed((i64::from(index) * 24).try_into().map_err(|_| Refusal::AddressOverflow)?)
        .ok_or(Refusal::AddressOverflow)
}
fn root_slot(base: usize, native_type: u32) -> Result<(usize, usize), Refusal> {
    let (index, rva) = if native_type < 100 {
        (native_type as usize, 0x03438e60 + native_type as usize * 8)
    } else if native_type < 0x1269 { (100, 0x03438e58) } else { (101, 0x03438e50) };
    Ok((index, offset(base, rva)?))
}

#[derive(Clone, Copy)]
enum TraceOwner { Country, Row(usize), Registry(usize) }
struct TraceEntry { opening: OpenedRead, field: Field, owner: TraceOwner }
struct TraceBlock { entries: [Option<TraceEntry>; TRACE_BLOCK], len: usize, next: Option<Box<TraceBlock>> }
const _: () = assert!(std::mem::size_of::<TraceBlock>() <= 8192);
#[derive(Default)]
struct Trace { head: Option<Box<TraceBlock>> }
impl Trace {
    fn prepare(&mut self, budget: &Budget) -> Result<(), Refusal> {
        if self.head.as_ref().is_some_and(|block| block.len < TRACE_BLOCK) { return Ok(()); }
        let mut next = charged_box(budget, TraceBlock {
            entries: std::array::from_fn(|_| None), len: 0, next: None })?;
        next.next = self.head.take();
        self.head = Some(next);
        Ok(())
    }
    fn push(&mut self, entry: TraceEntry) {
        let block = self.head.as_mut().expect("trace capacity reserved before opening read");
        block.entries[block.len] = Some(entry);
        block.len += 1;
    }
    fn close(mut self, out: &mut CountryObservation, read: &mut impl FnMut(usize, &mut [u8]) -> bool) {
        // Compare only original addresses. Capabilities are consumed even when another seal fails.
        while let Some(mut block) = self.head.take() {
            self.head = block.next.take();
            for entry in block.entries.iter_mut().take(block.len).filter_map(Option::take) {
                out.accounting.closing_attempted_bytes += entry.opening.width();
                let failure = match entry.opening.close(read) {
                    Ok(()) => continue,
                    Err(Cause::VectorChanged) => Refusal::Changed(entry.field),
                    Err(_) => Refusal::Unreadable(entry.field),
                };
                match entry.owner {
                    TraceOwner::Country => { if out.seal.is_ok() { out.seal = Err(failure); } }
                    TraceOwner::Row(index) => { if out.rows[index].seal.is_ok() { out.rows[index].seal = Err(failure); } }
                    TraceOwner::Registry(index) => {
                        for row in out.rows.iter_mut().filter(|row| row.registry == Some(index)) {
                            if row.seal.is_ok() { row.seal = Err(failure); }
                        }
                    }
                }
            }
        }
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        while let Some(mut block) = self.head.take() { self.head = block.next.take(); }
    }
}
fn charged_box<T>(budget: &Budget, value: T) -> Result<Box<T>, Refusal> {
    let layout = Layout::new::<T>();
    if layout.size() == 0 { return Err(Refusal::RetainedBound); }
    budget.retain(layout.size()).map_err(|_| Refusal::RetainedBound)?;
    // The charged layout includes padding/alignment; Box takes exactly this initialized allocation.
    unsafe {
        let pointer = alloc(layout).cast::<T>();
        if pointer.is_null() { return Err(Refusal::AllocationFailed); }
        pointer.write(value);
        Ok(Box::from_raw(pointer))
    }
}
fn charged_slice<T: Copy>(budget: &Budget, count: usize, init: impl Fn(usize) -> T) -> Result<Box<[T]>, Refusal> {
    if count == 0 { return Ok(Box::default()); }
    let layout = Layout::array::<T>(count).map_err(|_| Refusal::RetainedBound)?;
    if layout.size() == 0 { return Err(Refusal::RetainedBound); }
    budget.retain(layout.size()).map_err(|_| Refusal::RetainedBound)?;
    // Nonzero, checked layout; each element is initialized before ownership passes to Box<[T]>.
    unsafe {
        let pointer = alloc(layout).cast::<T>();
        if pointer.is_null() { return Err(Refusal::AllocationFailed); }
        for index in 0..count { pointer.add(index).write(init(index)); }
        Ok(Box::from_raw(std::ptr::slice_from_raw_parts_mut(pointer, count)))
    }
}
fn charged_rows(budget: &Budget, count: usize) -> Result<Box<[Row]>, Refusal> {
    charged_slice(budget, count, |ordinal| Row::unread(ordinal as u32))
}
fn charged_indices(budget: &Budget, count: usize) -> Result<Box<[usize]>, Refusal> {
    charged_slice(budget, count, |index| index)
}
fn mark_duplicates(rows: &mut [Row], indices: &mut [usize]) {
    indices.sort_unstable_by_key(|index| rows[*index].complete_object.filter(|pointer| *pointer != 0));
    for pair in indices.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        if rows[a].complete_object.is_some_and(|pointer| pointer != 0) && rows[a].complete_object == rows[b].complete_object {
            rows[a].duplicate_pointer = true;
            rows[b].duplicate_pointer = true;
        }
    }
    indices.sort_unstable_by_key(|index| rows[*index].reference);
    for pair in indices.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        if rows[a].reference.is_some() && rows[a].reference == rows[b].reference {
            rows[a].duplicate_reference = true;
            rows[b].duplicate_reference = true;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    const BASE: usize = 0x140000000;
    const COUNTRY: usize = 0x20000;
    const VECTOR: usize = 0x30000;
    const BODY: usize = 0x40000;
    const REGISTRY: usize = 0x60000;
    const SLOTS: usize = 0x70000;
    const OWNER: u32 = 7;

    #[derive(Default)]
    struct Memory {
        bytes: BTreeMap<usize, u8>,
        calls: Vec<(usize, usize)>,
        visits: BTreeMap<(usize, usize), usize>,
        fail: BTreeSet<(usize, usize)>,
        change_on_close: BTreeMap<(usize, usize), Vec<u8>>,
        fail_on_close: BTreeSet<(usize, usize)>,
    }
    impl Memory {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() { self.bytes.insert(address + index, *byte); }
        }
        fn read(&mut self, address: usize, out: &mut [u8]) -> bool {
            let key = (address, out.len());
            self.calls.push(key);
            let visit = self.visits.entry(key).or_default();
            *visit += 1;
            let closing = *visit == 2;
            if self.fail.contains(&key) || (closing && self.fail_on_close.contains(&key)) { return false; }
            if closing {
                if let Some(bytes) = self.change_on_close.get(&key).cloned() { self.put(address, &bytes); }
            }
            for (index, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(address + index)) else { return false; };
                *byte = *value;
            }
            true
        }
        fn header(&mut self, count: i32, capacity: i32, pointer: usize) {
            self.put(COUNTRY + 0x2a8, &(pointer as u64).to_le_bytes());
            self.put(COUNTRY + 0x2b0, &capacity.to_le_bytes());
            self.put(COUNTRY + 0x2b4, &count.to_le_bytes());
        }
        fn body(&mut self, ordinal: usize, body: usize, reference: Reference, owner: u32) {
            self.put(VECTOR + ordinal * 8, &(body as u64).to_le_bytes());
            self.put(body, &((BASE + 0x0295b600) as u64).to_le_bytes());
            self.put(body + 8, &13u32.to_le_bytes());
            self.put(body + 0x10, &((BASE + 0x0295b7d8) as u64).to_le_bytes());
            self.put(body + 0x18, &reference.native_type.to_le_bytes());
            self.put(body + 0x1c, &reference.id.to_le_bytes());
            self.put(body + 0x1d8, &owner.to_le_bytes());
        }
        fn registry(&mut self, reference: Reference, slots: usize, mask: u32, extra: u8) {
            let root = match reference.native_type {
                0..=99 => BASE + 0x03438e60 + 8 * reference.native_type as usize,
                100..=0x1268 => BASE + 0x03438e58,
                _ => BASE + 0x03438e50,
            };
            self.put(root, &(REGISTRY as u64).to_le_bytes());
            self.put(REGISTRY + 8, &(slots as u64).to_le_bytes());
            self.put(REGISTRY + 0x14, &mask.to_le_bytes());
            self.put(REGISTRY + 0x18, &[extra]);
        }
        fn entry(&mut self, slot: usize, distance: u8, reference: Reference, payload: usize) {
            self.put(slot + 4, &[distance]);
            self.put(slot + 8, &reference.native_type.to_le_bytes());
            self.put(slot + 0xc, &reference.id.to_le_bytes());
            self.put(slot + 0x10, &(payload as u64).to_le_bytes());
        }
        fn positive(reference: Reference) -> Self {
            let mut memory = Self::default();
            memory.header(1, 1, VECTOR);
            memory.body(0, BODY, reference, OWNER);
            memory.registry(reference, SLOTS, 0, 0);
            memory.entry(SLOTS, 1, reference, BODY + 0x10);
            memory
        }
        fn observe(&mut self) -> CountryObservation {
            RailwayReadset::new(&Budget::default()).read_country(BASE, COUNTRY, OWNER, &mut |address, out| self.read(address, out))
        }
        fn attempted_bytes(&self) -> usize { self.calls.iter().map(|(_, bytes)| bytes).sum() }
    }
    fn reference() -> Reference { Reference { native_type: 51, id: 19 } }
    fn assert_accounting(out: &CountryObservation, memory: &Memory) {
        assert_eq!(out.accounting.opening_attempted_bytes + out.accounting.closing_attempted_bytes, memory.attempted_bytes());
        assert!(out.accounting.reserved_bytes >= memory.attempted_bytes());
    }

    #[test]
    fn complete_object_plus_reference_base_is_required_and_accounted() {
        for (payload, expected) in [(BODY + 0x10, Ok(reference())), (BODY, Err(Refusal::DifferentPayload)),
            (BODY + 0x1010, Err(Refusal::DifferentPayload)), (0, Err(Refusal::NullPayload))] {
            let mut memory = Memory::positive(reference());
            memory.put(SLOTS + 0x10, &(payload as u64).to_le_bytes());
            let out = memory.observe();
            assert_eq!(out.locally_sealed_reference(0), expected);
            assert_eq!(out.accounting, Accounting { reserved_bytes: 188, opening_attempted_bytes: 94, closing_attempted_bytes: 94 });
            assert_accounting(&out, &memory);
            assert_eq!(out.rows[0].complete_object, Some(BODY));
            assert_eq!(out.rows[0].reference, Some(reference()));
        }
    }
    #[test]
    fn all_registry_tiers_accept_raw_types_without_a_railway_whitelist() {
        for (native_type, root) in [(0, BASE + 0x03438e60), (99, BASE + 0x03438e60 + 99 * 8),
            (100, BASE + 0x03438e58), (0x1268, BASE + 0x03438e58),
            (0x1269, BASE + 0x03438e50), (u32::MAX, BASE + 0x03438e50)] {
            let key = Reference { native_type, id: 19 };
            let mut memory = Memory::positive(key);
            let out = memory.observe();
            assert_eq!(out.locally_sealed_reference(0), Ok(key));
            assert_eq!(memory.calls.iter().filter(|call| **call == (root, 8)).count(), 2);
        }
    }
    #[test]
    fn signed_negative_bucket_and_wrapping_sentinel_use_native_i32_arithmetic() {
        let key = Reference { native_type: 0, id: 2 };
        let mut memory = Memory::positive(key);
        // 2 * 0x49631a45 = 0x92c6348a; mask selects INT_MIN, or -0xc00000000 bytes.
        memory.registry(key, 0xd00000000, 0x80000000, 0);
        memory.entry(0x100000000, 1, key, BODY + 0x10);
        let out = memory.observe();
        assert_eq!(out.locally_sealed_reference(0), Ok(key));
        assert!(memory.calls.contains(&(0x100000004, 1)));
        assert!(!memory.calls.iter().any(|(address, _)| *address >= 0xd00000000));
        assert_eq!(signed_slot(0xd00000000, 0x800000ffu32 as i32), Ok(0x1000017e8));
        assert_eq!(signed_slot(8, -1), Err(Refusal::AddressOverflow));
    }
    #[test]
    fn matching_sentinel_never_reads_payload() {
        let key = reference();
        let mut memory = Memory::positive(key);
        memory.entry(SLOTS, 1, Reference { id: 20, ..key }, 0);
        memory.entry(SLOTS + 24, 2, key, BODY + 0x10);
        let out = memory.observe();
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::Sentinel));
        assert!(!memory.calls.contains(&(SLOTS + 8, 4)));
        assert!(!memory.calls.contains(&(SLOTS + 24 + 0x10, 8)));
        assert_accounting(&out, &memory);
    }
    #[test]
    fn distance_and_id_branches_skip_unconsumed_fields() {
        for distance in [0, 1] {
            let mut memory = Memory::positive(reference());
            memory.entry(SLOTS, distance, Reference { id: 20, ..reference() }, 0);
            memory.put(SLOTS + 24 + 4, &[0]);
            let out = memory.observe();
            assert_eq!(out.locally_sealed_reference(0), Err(Refusal::MissingKey));
            assert!(!memory.calls.contains(&(SLOTS + 8, 4)));
            assert!(!memory.calls.contains(&(SLOTS + 0x10, 8)));
            assert_eq!(memory.calls.contains(&(SLOTS + 0xc, 4)), distance == 1);
            assert!(!memory.calls.contains(&(SLOTS + 24 + 0xc, 4)));
            assert_accounting(&out, &memory);
        }
    }
    #[test]
    fn type_mismatch_advances_without_reading_payload() {
        let mut memory = Memory::positive(reference());
        memory.entry(SLOTS, 1, Reference { native_type: 50, ..reference() }, 0);
        memory.put(SLOTS + 24 + 4, &[1]);
        let out = memory.observe();
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::MissingKey));
        assert!(memory.calls.contains(&(SLOTS + 8, 4)));
        assert!(!memory.calls.contains(&(SLOTS + 0x10, 8)));
        assert!(!memory.calls.contains(&(SLOTS + 24 + 0xc, 4)));
    }
    #[test]
    fn null_unreadable_missing_and_payload_outcomes_remain_distinct() {
        let root = BASE + 0x03438e60 + 51 * 8;
        let mut memory = Memory::positive(reference());
        memory.put(root, &0u64.to_le_bytes());
        let out = memory.observe();
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::NullRegistry));
        assert!(!memory.calls.contains(&(REGISTRY + 8, 8)));
        let mut memory = Memory::positive(reference());
        memory.fail.insert((root, 8));
        let out = memory.observe();
        assert_eq!(out.rows[0].read, Err(Refusal::Unreadable(Field::RegistryRoot)));
        assert_eq!(out.rows[0].seal, Ok(()));
        assert_eq!(out.accounting.reserved_bytes - memory.attempted_bytes(), 8);
        assert_eq!(memory.calls.iter().filter(|call| **call == (root, 8)).count(), 1);
        assert_accounting(&out, &memory);
    }
    #[test]
    fn every_consulted_field_mutation_is_caught_at_its_original_address() {
        let cases = [
            (COUNTRY + 0x2a8, 8, Field::HeaderPointer), (COUNTRY + 0x2b0, 4, Field::HeaderCapacity),
            (COUNTRY + 0x2b4, 4, Field::HeaderCount), (VECTOR, 8, Field::VectorSlot),
            (BODY, 8, Field::PrimaryVtable), (BODY + 8, 4, Field::Class),
            (BODY + 0x10, 8, Field::ReferenceVtable), (BODY + 0x18, 8, Field::Reference),
            (BODY + 0x1d8, 4, Field::Owner), (BASE + 0x03438e60 + 51 * 8, 8, Field::RegistryRoot),
            (REGISTRY + 8, 8, Field::RegistrySlots), (REGISTRY + 0x14, 4, Field::RegistryMask),
            (REGISTRY + 0x18, 1, Field::RegistryExtra), (SLOTS + 4, 1, Field::ProbeDistance),
            (SLOTS + 0xc, 4, Field::ProbeId), (SLOTS + 8, 4, Field::ProbeType),
            (SLOTS + 0x10, 8, Field::Payload),
        ];
        for (address, width, field) in cases {
            let mut memory = Memory::positive(reference());
            let replacement = vec![0xab; width];
            memory.change_on_close.insert((address, width), replacement);
            let out = memory.observe();
            assert_eq!(out.rows[0].read, Ok(()), "{field:?}");
            assert_eq!(out.locally_sealed_reference(0), Err(Refusal::Changed(field)), "{field:?}");
            assert!(!memory.calls.iter().any(|(address, _)| *address >= 0xababababab));
            assert_accounting(&out, &memory);
            assert_eq!(out.accounting.reserved_bytes, memory.attempted_bytes());
        }
    }
    #[test]
    fn closing_read_failure_is_separate_from_successful_opening() {
        let mut memory = Memory::positive(reference());
        memory.fail_on_close.insert((BODY + 0x18, 8));
        let out = memory.observe();
        assert_eq!(out.rows[0].read, Ok(()));
        assert_eq!(out.rows[0].seal, Err(Refusal::Unreadable(Field::Reference)));
        assert_accounting(&out, &memory);
        assert_eq!(out.accounting.reserved_bytes, memory.attempted_bytes());
    }
    #[test]
    fn empty_negative_and_invalid_headers_are_not_interchangeable() {
        for (count, capacity, expected) in [(0, 0, Ok(())), (-1, 0, Err(Refusal::NegativeCount)),
            (0, -1, Err(Refusal::NegativeCapacity)), (2, 1, Err(Refusal::CountExceedsCapacity)),
            (10_001, 10_001, Err(Refusal::SlotBound))] {
            let mut memory = Memory::default();
            memory.header(count, capacity, 0);
            let out = memory.observe();
            assert_eq!(out.read, expected);
            assert_eq!(out.header.count, Some(count));
            assert_eq!(out.header.capacity, Some(capacity));
            assert!(out.rows.is_empty());
            assert_eq!(memory.calls.len(), 6);
            assert_eq!(out.accounting, Accounting { reserved_bytes: 32, opening_attempted_bytes: 16, closing_attempted_bytes: 16 });
        }
        let mut memory = Memory::default();
        memory.header(2, 2, 0);
        let out = memory.observe();
        assert_eq!(out.read, Err(Refusal::NullPointer(Field::VectorSlot)));
        assert_eq!(out.rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::NullPointer(Field::VectorSlot)));
    }
    #[test]
    fn uninitialized_pair_and_wrong_owner_do_not_invoke_registry() {
        for (key, owner, expected) in [(Reference { native_type: 0, id: 0 }, OWNER, Refusal::UninitializedReference),
            (reference(), OWNER + 1, Refusal::OwnerMismatch)] {
            let mut memory = Memory::positive(key);
            memory.put(BODY + 0x1d8, &owner.to_le_bytes());
            let out = memory.observe();
            assert_eq!(out.locally_sealed_reference(0), Err(expected));
            assert_eq!(out.rows[0].reference, Some(key));
            assert!(!memory.calls.iter().any(|(address, _)| *address >= BASE));
        }
    }
    #[test]
    fn all_duplicate_ordinals_survive_and_all_are_ambiguous() {
        for same_pointer in [false, true] {
            let mut memory = Memory::positive(reference());
            memory.header(3, 3, VECTOR);
            for ordinal in 1..3 {
                memory.body(ordinal, if same_pointer { BODY } else { BODY + ordinal * 0x1000 }, reference(), OWNER);
            }
            let out = memory.observe();
            assert_eq!(out.rows.len(), 3);
            for (index, row) in out.rows.iter().enumerate() {
                assert_eq!(row.ordinal, index as u32);
                assert!(row.duplicate_reference);
                assert_eq!(row.duplicate_pointer, same_pointer);
                assert_eq!(out.locally_sealed_reference(index), Err(if same_pointer { Refusal::DuplicatePointer } else { Refusal::DuplicateReference }));
            }
            let root = BASE + 0x03438e60 + 51 * 8;
            assert_eq!(memory.calls.iter().filter(|call| **call == (root, 8)).count(), 2);
        }
    }
    #[test]
    fn last_row_mutation_preserves_other_row_and_shared_root_mutation_refuses_both() {
        for root_changes in [false, true] {
            let mut memory = Memory::positive(reference());
            let second = Reference { id: 20, ..reference() };
            memory.header(2, 2, VECTOR);
            memory.body(1, BODY + 0x1000, second, OWNER);
            // Extra=2 places sentinel after the occupied second slot.
            memory.registry(reference(), SLOTS, 0, 2);
            memory.entry(SLOTS + 24, 2, second, BODY + 0x1010);
            let (address, width, field) = if root_changes {
                (BASE + 0x03438e60 + 51 * 8, 8, Field::RegistryRoot)
            } else { (VECTOR + 8, 8, Field::VectorSlot) };
            memory.change_on_close.insert((address, width), vec![0xcd; width]);
            let out = memory.observe();
            assert_eq!(out.rows[0].read, Ok(()));
            assert_eq!(out.rows[1].read, Ok(()));
            assert_eq!(out.locally_sealed_reference(1), Err(Refusal::Changed(field)));
            assert_eq!(out.locally_sealed_reference(0), if root_changes { Err(Refusal::Changed(field)) } else { Ok(reference()) });
        }
    }
    #[test]
    fn lookup_cap_stops_before_cl_wrap_and_accounts_exact_skipped_type_reads() {
        let mut memory = Memory::positive(reference());
        for probe in 0..255 {
            memory.entry(SLOTS + probe * 24, 255, Reference { id: 20, ..reference() }, 0);
        }
        let out = memory.observe();
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::ProbeBound));
        assert_eq!(out.accounting.reserved_bytes, 32 + 80 + 42 + 255 * (2 + 8));
        assert_accounting(&out, &memory);
        assert!(!memory.calls.contains(&(SLOTS + 255 * 24 + 4, 1)));
        for probe in 0..255 {
            assert!(!memory.calls.contains(&(SLOTS + probe * 24 + 8, 4)));
            assert!(!memory.calls.contains(&(SLOTS + probe * 24 + 0x10, 8)));
        }
    }
    #[test]
    fn reservations_seal_earlier_reads_after_shared_budget_exhaustion() {
        let budget = Budget::test_limits(40, 1_000_000);
        let mut memory = Memory::positive(reference());
        let out = RailwayReadset::new(&budget).read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::ReadBound));
        assert_eq!(out.accounting, Accounting { reserved_bytes: 32, opening_attempted_bytes: 16, closing_attempted_bytes: 16 });
        assert_eq!(out.seal, Ok(()));
        assert_eq!(budget.verification_bytes(), 41);
        assert!(!memory.calls.contains(&(VECTOR, 8)));
        assert_accounting(&out, &memory);
    }
    #[test]
    fn failed_opening_consumes_reservation_and_cannot_fund_another_read() {
        let budget = Budget::test_limits(16, 1_000_000);
        let mut sampler = RailwayReadset::new(&budget);
        let mut memory = Memory::positive(reference());
        memory.fail.insert((COUNTRY + 0x2a8, 8));
        let first = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(first.read, Err(Refusal::Unreadable(Field::HeaderPointer)));
        assert_eq!(first.accounting, Accounting { reserved_bytes: 16, opening_attempted_bytes: 8, closing_attempted_bytes: 0 });
        memory.fail.clear();
        let second = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(second.read, Err(Refusal::ReadBound));
        assert_eq!(second.accounting, Accounting::default());
        assert_eq!(memory.calls, [(COUNTRY + 0x2a8, 8)]);
    }
    #[test]
    fn railway_limit_and_retention_are_charged_before_any_unfunded_read() {
        let budget = Budget::default();
        let mut sampler = RailwayReadset::new(&budget);
        sampler.reserved = READ_LIMIT - 16;
        let mut memory = Memory::positive(reference());
        let first = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(first.read, Err(Refusal::ReadBound));
        assert_eq!(first.accounting, Accounting { reserved_bytes: 16, opening_attempted_bytes: 8, closing_attempted_bytes: 8 });
        let second = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(second.read, Err(Refusal::ReadBound));
        assert_eq!(memory.calls.len(), 2);
        let budget = Budget::test_limits(1000, std::mem::size_of::<TraceBlock>() - 1);
        let mut memory = Memory::positive(reference());
        let out = RailwayReadset::new(&budget).read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(out.read, Err(Refusal::RetainedBound));
        assert!(memory.calls.is_empty());
    }
    #[test]
    fn row_storage_is_charged_including_layout_before_allocation() {
        let limit = std::mem::size_of::<TraceBlock>() + std::mem::size_of::<Row>() - 1;
        let budget = Budget::test_limits(1000, limit);
        let mut memory = Memory::positive(reference());
        let out = RailwayReadset::new(&budget).read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(out.read, Err(Refusal::RetainedBound));
        assert_eq!(out.header.count, Some(1));
        assert!(out.rows.is_empty());
        assert_eq!(memory.calls.len(), 6);
        assert_eq!(out.seal, Ok(()));
    }
    #[test]
    fn sweep_count_bounds_do_not_turn_into_empty_and_no_lookup_survives_next_country() {
        let budget = Budget::default();
        let mut sampler = RailwayReadset::new(&budget);
        sampler.slots = SLOT_LIMIT;
        let mut memory = Memory::positive(reference());
        let out = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(out.read, Err(Refusal::SlotBound));
        assert_eq!(out.header.count, Some(1));
        sampler.countries = COUNTRY_LIMIT;
        let calls = memory.calls.len();
        let out = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(out.read, Err(Refusal::CountryBound));
        assert_eq!(memory.calls.len(), calls);
        let mut sampler = RailwayReadset::new(&budget);
        let first = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(first.locally_sealed_reference(0), Ok(reference()));
        memory.put(BASE + 0x03438e60 + 51 * 8, &0u64.to_le_bytes());
        let second = sampler.read_country(BASE, COUNTRY, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(second.locally_sealed_reference(0), Err(Refusal::NullRegistry));
    }
    #[test]
    fn unreadable_physical_slot_keeps_its_ordinal_without_blanking_a_good_row() {
        let mut memory = Memory::positive(reference());
        memory.header(2, 2, VECTOR);
        memory.fail.insert((VECTOR + 8, 8));
        let out = memory.observe();
        assert_eq!(out.read, Ok(()));
        assert_eq!(out.rows.len(), 2);
        assert_eq!(out.locally_sealed_reference(0), Ok(reference()));
        assert_eq!(out.rows[1].ordinal, 1);
        assert_eq!(out.rows[1].complete_object, None);
        assert_eq!(out.locally_sealed_reference(1), Err(Refusal::Unreadable(Field::VectorSlot)));
        assert_eq!(out.accounting.reserved_bytes - memory.attempted_bytes(), 8);
        assert_accounting(&out, &memory);
    }
    #[test]
    fn unreadable_matching_payload_is_not_a_witnessed_null_payload() {
        let mut memory = Memory::positive(reference());
        memory.fail.insert((SLOTS + 0x10, 8));
        let out = memory.observe();
        assert_eq!(out.rows[0].reference, Some(reference()));
        assert_eq!(out.locally_sealed_reference(0), Err(Refusal::Unreadable(Field::Payload)));
        assert_eq!(out.accounting, Accounting { reserved_bytes: 188, opening_attempted_bytes: 94, closing_attempted_bytes: 86 });
        assert_accounting(&out, &memory);
    }

    #[test]
    fn bad_body_class_and_address_overflow_skip_unproven_object_fields() {
        for (address, bytes, expected) in [(BODY, 8, Refusal::PrimaryVtable), (BODY + 8, 4, Refusal::Class),
            (BODY + 0x10, 8, Refusal::ReferenceVtable)] {
            let mut memory = Memory::positive(reference());
            memory.put(address, &vec![0; bytes]);
            let out = memory.observe();
            assert_eq!(out.locally_sealed_reference(0), Err(expected));
            assert!(!memory.calls.contains(&(BODY + 0x18, 8)));
        }
        let mut memory = Memory::default();
        let out = RailwayReadset::new(&Budget::default()).read_country(BASE, usize::MAX - 4, OWNER, &mut |a, b| memory.read(a, b));
        assert_eq!(out.read, Err(Refusal::AddressOverflow));
        assert!(memory.calls.is_empty());
    }
    #[test]
    fn converted_slots_and_cross_country_duplicate_index_are_separately_charged() {
        use compact_contract::country_roster::{Acquisition, Kind, Observation};
        let mut memory=Memory::positive(reference());let local=memory.observe();
        let refused=local.into_roster_country(0,Acquisition::Bracketed,&Budget::test_limits(1000,0));
        assert_eq!(refused.failure,Some(Cause::RailwayRetainedBound));assert!(refused.slots.is_empty());
        let mut memory=Memory::positive(reference());let local=memory.observe();
        let row=local.into_roster_country(0,Acquisition::Bracketed,&Budget::default());assert!(row.complete());
        let mut second=row.clone();second.physical_slot=1;second.owner_tag_db_id=Some(OWNER+1);
        let observed=Observation {kind:Kind::Railway,country_count:Some(2),failure:None,countries:vec![row,second]};
        let mut no_index=observed.clone();refuse_cross_country_duplicates(&mut no_index,&Budget::test_limits(1000,0));
        assert_eq!(no_index.failure,Some(Cause::RailwayRetainedBound));
        assert!(no_index.countries.iter().all(|country|country.slots.iter().all(|slot|
            slot.reference==Err(Cause::RailwayRetainedBound))));
        assert_eq!(Observation::parse(&no_index.encode().unwrap(),Kind::Railway).unwrap(),no_index);
        let mut indexed=observed;refuse_cross_country_duplicates(&mut indexed,&Budget::default());
        for country in &indexed.countries { assert_eq!(country.slots[0].reference,Err(Cause::RailwayDuplicateReference)); }
        assert_eq!(Observation::parse(&indexed.encode().unwrap(),Kind::Railway).unwrap(),indexed);
    }

}
