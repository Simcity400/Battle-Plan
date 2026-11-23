use engine_native_parts::cair_base_layout::{HOST_REFERENCE, VTABLE_RVA};

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
    walk::Outcome,
};
use crate::{ccountry_air_container, cstrategic_air_manager};
use engine_native_parts::cair_wing_pool;
use engine_native::{ccountry, cgame_state, cprovince, cstate};
use engine_sites::Rva;

mod carrier;
mod carrier_capture;
mod demand;
mod reuse;
mod state_summary;
use crate::air_fuel_inputs as fuel;
pub use carrier_capture::{
    CarrierAceSlot, CarrierBase, CarrierBuffers, CarrierCapture, CarrierCountryTerms,
    CarrierDemand, CarrierForecastSlot, CarrierIndexedCertificate, CarrierLengths, CarrierPool,
    CarrierPoolFuel, CarrierRegistration, ContainerFuel, CountryOccurrence,
};
pub use reuse::{HeldBase, RegistrationReuse, WalkedBase};
pub use state_summary::{
    state_country_order, state_country_order_demand, verify_country_occurrences,
};

#[allow(clippy::too_many_arguments)]
pub fn state_pool_inputs<M: Memory>(
    memory: &M,
    image: usize,
    registration: CarrierIndexedCertificate,
    pool: usize,
    members: Array,
    wings_start: usize,
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut [ResolvedLink],
) -> Result<CarrierPool, &'static str> {
    let metadata = carrier::pool_metadata(
        memory,
        image,
        carrier::PoolObservation {
            pointer: pool,
            registration,
            members,
            wings_start,
        },
        countries,
        &mut Resolver { cells: resolved },
        &mut None,
    )?;
    Ok(CarrierPool {
        type_index: metadata.definition.type_index,
        subunit_index: metadata.definition.subunit_index,
        wings_start: metadata.wings_start,
        wings_len: metadata.wings_len,
        ..Default::default()
    })
}

pub fn carrier_host_name_demand(
    memory: &impl Memory,
    image: usize,
    source: &Row,
) -> Result<usize, &'static str> {
    let mut resolver = engine_native_parts::creference_registry::ScopedResolver::new(image);
    carrier::host_name_demand(memory, image, source, &mut resolver)
}

pub const SIZE: usize = 0x90;
pub const INCOMING_WINGS: usize = 0x18;
pub const COUNTRY_CONTAINERS: usize = 0x30;
pub const COUNTRY_CONTAINERS_COUNT: usize = 0x3c;
pub const REGISTERED_CONTAINERS: usize = 0x48;
pub const STATE: usize = 0x68;
pub const PROVINCE: usize = 0x70;
pub const CAPACITY: usize = 0x78;
pub const KIND: usize = 0x7c;
pub const CREATE_COUNTRY_CONTAINER_RVA: Rva = Rva(0x00ca_8ca0);
pub const REGISTER_POOL_RVA: Rva = Rva(0x00ca_9250);
pub const ERASE_POOL_RVA: Rva = Rva(0x00cc_0700);
const BATCH: usize = 64;
const WING_RECORD_BYTES: usize = 40;
const WING_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 32,
    },
    RecordField {
        source_offset: engine_native_parts::cair_wing_layout::POOL,
        destination_offset: 32,
        len: 8,
    },
];
const WING_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&WING_FIELDS, WING_RECORD_BYTES).expect("valid record layout");
const POOL_RECORD_BYTES: usize = 24;
const POOL_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cair_wing_pool::WINGS,
        destination_offset: 8,
        len: 16,
    },
];
const POOL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POOL_FIELDS, POOL_RECORD_BYTES).expect("valid record layout");
const CONTAINER_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const CONTAINER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CONTAINER_FIELD, 8).expect("valid record layout");

pub const DENSE_BOUND: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum DenseRefusal {
    Header = 0,
    LongerThanSlots = 1,
    OverBound = 2,
    Entry = 3,
    Container = 4,
    Country = 5,
    SlotRange = 6,
    SharedSlot = 7,
    Stale = 8,
}

impl DenseRefusal {
    pub const COUNT: usize = 9;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Header,
        Self::LongerThanSlots,
        Self::OverBound,
        Self::Entry,
        Self::Container,
        Self::Country,
        Self::SlotRange,
        Self::SharedSlot,
        Self::Stale,
    ];

    pub const fn reason(self) -> &'static str {
        match self {
            Self::Header => "dense container list unreadable or invalid",
            Self::LongerThanSlots => "dense container list longer than the slot vector",
            Self::OverBound => "dense container list beyond the fast-path bound",
            Self::Entry => "dense container list entry null or listed twice",
            Self::Container => "listed container identity or span unreadable",
            Self::Country => "listed container country names no country slot",
            Self::SlotRange => "listed container slot beyond the slot vector",
            Self::SharedSlot => "two listed containers name one slot",
            Self::Stale => "listed container is not at the slot its country names",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DenseEntry {
    pub slot: u32,
    pub container: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SlotMode {
    pub dense: bool,
}

pub fn dense_plan(
    memory: &impl Memory,
    image: usize,
    base: usize,
    containers: Array,
    [countries, links]: [Array; 2],
    entries: &mut [DenseEntry; DENSE_BOUND],
) -> Result<usize, DenseRefusal> {
    use DenseRefusal as Refusal;
    let dense = base
        .checked_add(REGISTERED_CONTAINERS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or(Refusal::Header)?;
    if dense.count > containers.count {
        return Err(Refusal::LongerThanSlots);
    }
    let len = dense.count as usize;
    if len > DENSE_BOUND {
        return Err(Refusal::OverBound);
    }
    let mut bytes = [0_u8; DENSE_BOUND * 8];
    if len != 0 && !read_slice(memory, dense.data, &mut bytes[..len * 8]) {
        return Err(Refusal::Header);
    }
    let expected = engine_sites::at(image, ccountry_air_container::VTABLE_RVA)
        .ok_or(Refusal::Container)? as u64;
    for index in 0..len {
        let word = u64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap());
        let container = usize::try_from(word)
            .ok()
            .filter(|&pointer| pointer != 0)
            .ok_or(Refusal::Entry)?;
        if entries[..index]
            .iter()
            .any(|entry| entry.container == container)
        {
            return Err(Refusal::Entry);
        }
        if read::<u64>(memory, container) != Some(expected) {
            return Err(Refusal::Container);
        }
        let mut span = [0_u8; CONTAINER_SPAN];
        if !container
            .checked_add(ccountry_air_container::COUNTRY)
            .is_some_and(|at| read_slice(memory, at, &mut span))
        {
            return Err(Refusal::Container);
        }
        let country = u32::from_le_bytes(span[..4].try_into().unwrap());
        let slot = if country == 0 {
            0
        } else {
            if country >= links.count {
                return Err(Refusal::Country);
            }
            let entry = links
                .element(country, 4)
                .and_then(|at| read::<u32>(memory, at))
                .ok_or(Refusal::Country)?;
            if entry >= countries.count {
                return Err(Refusal::Country);
            }
            entry
        };
        if slot >= containers.count {
            return Err(Refusal::SlotRange);
        }
        if entries[..index].iter().any(|entry| entry.slot == slot) {
            return Err(Refusal::SharedSlot);
        }
        let at = containers.element(slot, 8).ok_or(Refusal::SlotRange)?;
        if read::<u64>(memory, at) != Some(container as u64) {
            return Err(Refusal::Stale);
        }
        entries[index] = DenseEntry { slot, container };
    }
    entries[..len].sort_unstable_by_key(|entry| entry.slot);
    Ok(len)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub state: Option<u32>,
    pub province: Option<u32>,
    pub host: u64,
    pub capacity: i32,
    pub kind: i32,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let reader = Reader { memory, base };
        let manager = reader.pointer(game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
        reader.vtable(manager, cstrategic_air_manager::VTABLE_RVA)?;
        let state_bases = reader.array(manager, cstrategic_air_manager::STATE_AIR_BASES)?;
        let rocket_bases = reader.array(manager, cstrategic_air_manager::STATE_ROCKET_BASES)?;
        let gun_bases = reader.array(manager, cstrategic_air_manager::STATE_GUN_BASES)?;
        let host_bases = reader.array(manager, cstrategic_air_manager::STATELESS_AIR_BASES)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for (array, indexed) in [
            (state_bases, true),
            (rocket_bases, true),
            (gun_bases, true),
            (host_bases, false),
        ] {
            for index in 0..array.count {
                let pointer = array
                    .element(index, 8)
                    .and_then(|at| read::<u64>(memory, at))
                    .and_then(|p| usize::try_from(p).ok())
                    .ok_or("air base roster pointer unreadable or address overflow")?;
                if pointer == 0 {
                    if indexed {
                        continue;
                    }
                    return Err("stateless air base roster contains null base");
                }
                output.ensure(len.checked_add(1).ok_or("air base row count overflow")?)?;
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("air base unread count overflow")?;
                    continue;
                }
                output[len] = reader.row(pointer, indexed.then_some(index))?;
                len += 1;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
}

#[derive(Clone, Copy)]
struct RegistrationBase {
    pointer: usize,
    handle: u64,
}

impl<M: Memory> Reader<'_, M> {
    fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|at| read::<u64>(self.memory, at))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("air base pointer unreadable or address overflow")
    }

    fn array(&self, pointer: usize, offset: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|at| Array::read(self.memory, at, 8))
            .ok_or("air base roster unreadable or invalid")
    }

    fn vtable(&self, pointer: usize, rva: Rva) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, rva).ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err("air base or linked object vtable mismatch")
        }
    }

    fn row(&self, pointer: usize, state_slot: Option<u32>) -> Result<Row, &'static str> {
        let mut identity = [0; 16];
        let mut data = [0; 32];
        let complete = self.memory.copy_scattered(
            [pointer, pointer.checked_add(HOST_REFERENCE).unwrap_or(0)],
            &mut [&mut identity, &mut data],
        );
        let expected = engine_sites::at(self.base, VTABLE_RVA).ok_or("module address overflow")?;
        if complete == 0 || u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
        {
            return Err("air base vtable mismatch or identity unreadable");
        }
        let id = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if id == 0 {
            return Err("nonnull air base has null handle");
        }
        if complete != 2 {
            return Err("air base fields unreadable or address overflow");
        }
        let host = u64::from_le_bytes(data[..8].try_into().unwrap());
        let state_pointer = usize::try_from(u64::from_le_bytes(data[8..16].try_into().unwrap()))
            .map_err(|_| "air base state address overflow")?;
        let province_pointer =
            usize::try_from(u64::from_le_bytes(data[16..24].try_into().unwrap()))
                .map_err(|_| "air base province address overflow")?;
        let state = if state_pointer == 0 {
            None
        } else {
            self.vtable(state_pointer, cstate::VTABLE_RVA)?;
            Some(
                state_pointer
                    .checked_add(cstate::ID)
                    .and_then(|at| read::<u32>(self.memory, at))
                    .ok_or("air base state id unreadable")?,
            )
        };
        if state != state_slot {
            return Err("air base state differs from roster slot");
        }
        if state.is_none() && host == 0 {
            return Err("stateless air base has no host reference");
        }
        let province = if province_pointer == 0 {
            None
        } else {
            self.vtable(province_pointer, cprovince::VTABLE_RVA)?;
            if state_pointer != 0
                && self.pointer(province_pointer, cprovince::STATE)? != state_pointer
            {
                return Err("air base province belongs to another state");
            }
            Some(
                province_pointer
                    .checked_add(cprovince::ID)
                    .and_then(|at| read::<u32>(self.memory, at))
                    .ok_or("air base province id unreadable")?,
            )
        };
        Ok(Row {
            id,
            state,
            province,
            host,
            capacity: i32::from_le_bytes(data[24..28].try_into().unwrap()),
            kind: i32::from_le_bytes(data[28..32].try_into().unwrap()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn registration<O: Memory>(
        &self,
        observed: RegistrationBase,
        countries: &ccountry::Reader<'_, M>,
        resolved: &mut Resolver<'_>,
        buffers: &mut RegistrationBuffers<'_>,
        state: &mut RegistrationState,
        carrier: &mut Option<&mut CarrierCapture<'_, O>>,
        slots: SlotMode,
        mut reuse: Option<&mut (dyn RegistrationReuse + '_)>,
    ) -> Result<(), &'static str> {
        let RegistrationBase {
            pointer: base,
            handle: base_handle,
        } = observed;
        let containers = self
            .array(base, COUNTRY_CONTAINERS)
            .map_err(|_| "air base container table unreadable or invalid")?;
        state.last_slots = containers.count;
        state.required_indexed = state.required_indexed.max(containers.count as usize);
        if let Some(carrier) = carrier.as_deref_mut() {
            carrier.indexed_header(containers.count);
        }
        let planned = slots
            .dense
            .then(|| {
                dense_plan(
                    self.memory,
                    self.base,
                    base,
                    containers,
                    countries.arrays(),
                    &mut state.scratch.plan,
                )
                .ok()
            })
            .flatten();
        if planned.is_some() {
            state.dense_bases += 1;
        }
        state.last_plan = planned;
        let expected_pool = engine_sites::at(self.base, cair_wing_pool::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let expected_primary = engine_sites::at(self.base, engine_native_parts::cair_wing_layout::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let expected_secondary = engine_sites::at(self.base, engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let RegistrationScratch {
            container_bytes,
            pool_pointers,
            pool_records,
            pool_pointer_bytes,
            wing_pointers,
            wing_records,
            wing_pointer_bytes,
            plan,
        } = &mut state.scratch;
        let mut next_planned = 0;
        let mut chunk_first = 0;
        while chunk_first < containers.count {
            let chunk = ((containers.count - chunk_first) as usize).min(BATCH);
            let copied = match planned {
                Some(len) => {
                    let end = chunk_first + chunk as u32;
                    let first = next_planned;
                    while plan[..len]
                        .get(next_planned)
                        .is_some_and(|entry| entry.slot < end)
                    {
                        next_planned += 1;
                    }
                    if first == next_planned {
                        chunk_first = end;
                        continue;
                    }
                    container_bytes[..chunk * 8].fill(0);
                    for entry in &plan[first..next_planned] {
                        let at = (entry.slot - chunk_first) as usize * 8;
                        container_bytes[at..at + 8]
                            .copy_from_slice(&(entry.container as u64).to_le_bytes());
                    }
                    chunk
                }
                None => match containers.element(chunk_first, 8) {
                    Some(base) => self.memory.copy_records(
                        RecordSources::Strided {
                            base,
                            stride: 8,
                            count: chunk,
                        },
                        &CONTAINER_LAYOUT,
                        &mut container_bytes[..chunk * 8],
                    ),
                    None => 0,
                },
            };
            for index in 0..chunk {
                let slot = chunk_first + index as u32;
                if index >= copied {
                    return Err("air base container pointer unreadable or address overflow");
                }
                let container = usize::try_from(u64::from_le_bytes(
                    container_bytes[index * 8..index * 8 + 8]
                        .try_into()
                        .unwrap(),
                ))
                .map_err(|_| "air base container pointer unreadable or address overflow")?;
                if container == 0 {
                    continue;
                }
                self.vtable(container, ccountry_air_container::VTABLE_RVA)?;
                let mut span = [0u8; CONTAINER_SPAN];
                if !container
                    .checked_add(ccountry_air_container::COUNTRY)
                    .is_some_and(|at| read_slice(self.memory, at, &mut span))
                {
                    return Err("air base container country unreadable");
                }
                let country = u32::from_le_bytes(span[..4].try_into().unwrap());
                let (capacity_raw, fuel) = container_span_words(&span);
                if slot == 0 {
                    if country != 0 && countries.link(country)?.is_some() {
                        return Err(
                            "air base container at the reserved slot is linked to a country slot",
                        );
                    }
                } else if resolved.resolve(countries, country)?.canonical_slot != slot {
                    return Err("air base container country differs from its slot");
                }
                if let Some(carrier) = carrier.as_deref_mut() {
                    carrier.indexed_container(country, slot, container);
                    carrier.begin_registration(fuel);
                }
                let pools = self
                    .array(container, ccountry_air_container::POOLS)
                    .map_err(|_| "air container pool vector unreadable or invalid")?;
                state.required_pools = state
                    .required_pools
                    .checked_add(pools.count as usize)
                    .ok_or("air registration pool demand overflow")?;
                if let Some(carrier) = carrier.as_deref_mut() {
                    carrier.registration_header(pools.count);
                }
                let start = state.wings_len;
                let mut wings_len = 0;
                let mut fits = state.len < buffers.registrations.len();
                let mut pool_first = 0;
                while pool_first < pools.count {
                    let chunk = ((pools.count - pool_first) as usize).min(BATCH);
                    let first_address = pools
                        .element(pool_first, 8)
                        .ok_or("air container pool pointer unreadable or null")?;
                    if !read_slice(
                        self.memory,
                        first_address,
                        &mut pool_pointer_bytes[..chunk * 8],
                    ) {
                        return Err("air container pool pointer unreadable or null");
                    }
                    for index in 0..chunk {
                        let bytes: [u8; 8] = pool_pointer_bytes[index * 8..index * 8 + 8]
                            .try_into()
                            .unwrap();
                        let pointer = usize::try_from(u64::from_le_bytes(bytes))
                            .map_err(|_| "air container pool pointer unreadable or null")?;
                        if pointer == 0 {
                            return Err("air container pool pointer unreadable or null");
                        }
                        pool_pointers[index] = pointer;
                    }
                    let copied = self.memory.copy_records(
                        RecordSources::Pointers(&pool_pointers[..chunk]),
                        &POOL_LAYOUT,
                        &mut pool_records[..chunk * POOL_RECORD_BYTES],
                    );
                    for index in 0..chunk {
                        let containing_pool = pool_pointers[index];
                        if copied < (index + 1) * POOL_FIELDS.len() {
                            if copied % POOL_FIELDS.len() == 0 {
                                return Err("air base or linked object vtable mismatch");
                            }
                            return Err("air pool wing vector unreadable or invalid");
                        }
                        let record = &pool_records
                            [index * POOL_RECORD_BYTES..(index + 1) * POOL_RECORD_BYTES];
                        if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected_pool {
                            return Err("air base or linked object vtable mismatch");
                        }
                        let header: [u8; 16] = record[8..24].try_into().unwrap();
                        let members = Array::from_header(header, 8)
                            .ok_or("air pool wing vector unreadable or invalid")?;
                        state.required_wings = state
                            .required_wings
                            .checked_add(members.count as usize)
                            .ok_or("air registration wing demand overflow")?;
                        if let Some(carrier) = carrier.as_deref_mut() {
                            carrier.begin_pool(containing_pool, members);
                        }
                        let mut wing_first = 0;
                        while wing_first < members.count {
                            let wing_chunk = ((members.count - wing_first) as usize).min(BATCH);
                            let wing_address = members
                                .element(wing_first, 8)
                                .ok_or("air pool wing pointer unreadable or null")?;
                            if !read_slice(
                                self.memory,
                                wing_address,
                                &mut wing_pointer_bytes[..wing_chunk * 8],
                            ) {
                                return Err("air pool wing pointer unreadable or null");
                            }
                            for wing_index in 0..wing_chunk {
                                let bytes: [u8; 8] = wing_pointer_bytes
                                    [wing_index * 8..wing_index * 8 + 8]
                                    .try_into()
                                    .unwrap();
                                let pointer = usize::try_from(u64::from_le_bytes(bytes))
                                    .map_err(|_| "air pool wing pointer unreadable or null")?;
                                if pointer == 0 {
                                    return Err("air pool wing pointer unreadable or null");
                                }
                                wing_pointers[wing_index] = pointer;
                            }
                            let wing_copied = self.memory.copy_records(
                                RecordSources::Pointers(&wing_pointers[..wing_chunk]),
                                &WING_LAYOUT,
                                &mut wing_records[..wing_chunk * WING_RECORD_BYTES],
                            );
                            for wing_index in 0..wing_chunk {
                                if wing_copied < (wing_index + 1) * WING_FIELDS.len() {
                                    return Err("air pool wing identity unreadable");
                                }
                                let identity = &wing_records[wing_index * WING_RECORD_BYTES
                                    ..(wing_index + 1) * WING_RECORD_BYTES];
                                if u64::from_le_bytes(identity[..8].try_into().unwrap())
                                    != expected_primary
                                    || u64::from_le_bytes(identity[16..24].try_into().unwrap())
                                        != expected_secondary
                                {
                                    return Err("air pool wing vtable mismatch");
                                }
                                let handle =
                                    u64::from_le_bytes(identity[24..32].try_into().unwrap());
                                if u64::from_le_bytes(identity[32..40].try_into().unwrap())
                                    != containing_pool as u64
                                {
                                    return Err(
                                        "registered air wing backlink differs from containing pool",
                                    );
                                }
                                if handle == 0 {
                                    return Err("registered air wing has a null reference handle");
                                }
                                if let Some(carrier) = carrier.as_deref_mut() {
                                    carrier.wing(wing_pointers[wing_index]);
                                }
                                match buffers.wings.get_mut(state.wings_len).filter(|_| fits) {
                                    Some(destination) => {
                                        *destination = handle;
                                        state.wings_len += 1;
                                        wings_len += 1;
                                    }
                                    None => fits = false,
                                }
                            }
                            wing_first += wing_chunk as u32;
                        }
                        if let Some(carrier) = carrier.as_deref_mut() {
                            carrier.finish_pool();
                        }
                    }
                    pool_first += chunk as u32;
                }
                if !fits {
                    state.wings_len = start;
                    state.unread_rows = state
                        .unread_rows
                        .checked_add(1)
                        .ok_or("air registration omitted count overflow")?;
                    if let Some(carrier) = carrier.as_deref_mut() {
                        carrier.finish_registration(None);
                    }
                    continue;
                }
                buffers.registrations[state.len] = RegistrationRow {
                    base: base_handle,
                    slot,
                    capacity_raw,
                    wings_start: start,
                    wings_len,
                };
                state.len += 1;
                if let Some(reuse) = reuse.as_deref_mut() {
                    reuse.container(country);
                }
                if let Some(carrier) = carrier.as_deref_mut() {
                    carrier.finish_registration(Some(wings_len));
                }
            }
            chunk_first += chunk as u32;
        }
        if let Some(carrier) = carrier.as_deref_mut() {
            carrier.finish_base(true);
        }
        Ok(())
    }
}

pub const CONTAINER_SPAN: usize =
    ccountry_air_container::FUEL_ALLOCATED_RAW + 8 - ccountry_air_container::COUNTRY;

pub fn container_span_words(span: &[u8; CONTAINER_SPAN]) -> (i64, ContainerFuel) {
    let word = |offset: usize| {
        let at = offset - ccountry_air_container::COUNTRY;
        i64::from_le_bytes(span[at..at + 8].try_into().unwrap())
    };
    (
        word(ccountry_air_container::CAPACITY_RAW),
        ContainerFuel {
            demand: Ok(word(ccountry_air_container::FUEL_DEMAND_RAW)),
            allocated: Ok(word(ccountry_air_container::FUEL_ALLOCATED_RAW)),
        },
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationRow {
    pub base: u64,
    pub slot: u32,
    pub capacity_raw: i64,
    pub wings_start: usize,
    pub wings_len: usize,
}

pub struct RegistrationBuffers<'a> {
    pub registrations: &'a mut [RegistrationRow],
    pub wings: &'a mut [u64],
    pub resolved: &'a mut [ResolvedLink],
}

pub type ResolvedLink = Option<engine_native_memory::country_identity::Country>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationWalk {
    pub outcome: Outcome,
    pub wings_len: usize,
    pub required_wings: usize,
    pub required_links: usize,
    pub required_bases: usize,
    pub required_indexed: usize,
    pub required_pools: usize,
    pub dense_bases: usize,
    pub reused_bases: usize,
}

#[derive(Default)]
struct RegistrationState {
    len: usize,
    unread_rows: u32,
    wings_len: usize,
    required_wings: usize,
    required_indexed: usize,
    required_pools: usize,
    dense_bases: usize,
    reused_bases: usize,
    last_slots: u32,
    last_plan: Option<usize>,
    scratch: RegistrationScratch,
}

struct RegistrationScratch {
    container_bytes: [u8; BATCH * 8],
    pool_pointers: [usize; BATCH],
    pool_records: [u8; BATCH * POOL_RECORD_BYTES],
    pool_pointer_bytes: [u8; BATCH * 8],
    wing_pointers: [usize; BATCH],
    wing_records: [u8; BATCH * WING_RECORD_BYTES],
    wing_pointer_bytes: [u8; BATCH * 8],
    plan: [DenseEntry; DENSE_BOUND],
}

impl Default for RegistrationScratch {
    fn default() -> Self {
        Self {
            container_bytes: [0; BATCH * 8],
            pool_pointers: [0; BATCH],
            pool_records: [0; BATCH * POOL_RECORD_BYTES],
            pool_pointer_bytes: [0; BATCH * 8],
            wing_pointers: [0; BATCH],
            wing_records: [0; BATCH * WING_RECORD_BYTES],
            wing_pointer_bytes: [0; BATCH * 8],
            plan: [DenseEntry::default(); DENSE_BOUND],
        }
    }
}

struct Resolver<'a> {
    cells: &'a mut [ResolvedLink],
}

impl Resolver<'_> {
    fn resolve(
        &mut self,
        countries: &ccountry::Reader<'_, impl Memory>,
        raw: u32,
    ) -> Result<engine_native_memory::country_identity::Country, &'static str> {
        let cell = raw as usize;
        if let Some(Some(country)) = self.cells.get(cell) {
            return Ok(*country);
        }
        let country = countries.resolve(raw)?;
        if let Some(cell) = self.cells.get_mut(cell) {
            *cell = Some(country);
        }
        Ok(country)
    }
}

pub fn walk_registrations<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
) -> RegistrationWalk {
    walk_registrations_planned(memory, base, game_state, buffers, SlotMode::default(), None)
}

pub fn walk_registrations_planned<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
    slots: SlotMode,
    reuse: Option<&mut (dyn RegistrationReuse + '_)>,
) -> RegistrationWalk {
    walk_registrations_inner::<M, M>(memory, base, game_state, buffers, &[], None, slots, reuse)
}

pub fn walk_registrations_with_carrier<M: Memory, O: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
    base_rows: &[Row],
    carrier: &mut CarrierCapture<'_, O>,
) -> RegistrationWalk {
    walk_registrations_with_carrier_planned(
        memory,
        base,
        game_state,
        buffers,
        base_rows,
        carrier,
        SlotMode::default(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_registrations_with_carrier_planned<M: Memory, O: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
    base_rows: &[Row],
    carrier: &mut CarrierCapture<'_, O>,
    slots: SlotMode,
    reuse: Option<&mut (dyn RegistrationReuse + '_)>,
) -> RegistrationWalk {
    walk_registrations_inner(
        memory,
        base,
        game_state,
        buffers,
        base_rows,
        Some(carrier),
        slots,
        reuse,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_registrations_inner<M: Memory, O: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    mut buffers: RegistrationBuffers<'_>,
    base_rows: &[Row],
    mut carrier: Option<&mut CarrierCapture<'_, O>>,
    slots: SlotMode,
    mut reuse: Option<&mut (dyn RegistrationReuse + '_)>,
) -> RegistrationWalk {
    let mut state = RegistrationState::default();
    let mut required_links = 0;
    let mut base_ordinal = 0_usize;
    let result = (|| {
        let reader = Reader { memory, base };
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let links = countries.arrays()[1].count as usize;
        required_links = links;
        let cells = std::mem::take(&mut buffers.resolved);
        let used = links.min(cells.len());
        cells[..used].fill(None);
        let mut resolved = Resolver {
            cells: &mut cells[..used],
        };
        let manager = reader.pointer(game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
        reader.vtable(manager, cstrategic_air_manager::VTABLE_RVA)?;
        for (offset, indexed) in [
            (cstrategic_air_manager::STATE_AIR_BASES, true),
            (cstrategic_air_manager::STATE_ROCKET_BASES, true),
            (cstrategic_air_manager::STATE_GUN_BASES, true),
            (cstrategic_air_manager::STATELESS_AIR_BASES, false),
        ] {
            let array = reader.array(manager, offset)?;
            for index in 0..array.count {
                let pointer = array
                    .element(index, 8)
                    .and_then(|at| read::<u64>(memory, at))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .ok_or("air base roster pointer unreadable or address overflow")?;
                if pointer == 0 {
                    if indexed {
                        continue;
                    }
                    return Err("stateless air base roster contains null base");
                }
                let identity =
                    read::<[u8; 16]>(memory, pointer).ok_or("air base identity unreadable")?;
                let expected =
                    engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
                if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
                    return Err("air base vtable mismatch");
                }
                let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
                if handle == 0 {
                    return Err("nonnull air base has null handle");
                }
                let source = base_rows.get(base_ordinal).copied();
                base_ordinal += 1;
                if let Some(carrier) = carrier.as_deref_mut() {
                    carrier.begin_base(pointer, handle, !indexed);
                    if let Some(source) = source.filter(|source| source.id == handle) {
                        carrier.authenticated_base(source);
                    }
                }
                let observed = RegistrationBase { handle, pointer };
                let Some(reuse) = reuse.as_deref_mut().filter(|_| indexed) else {
                    reader.registration(
                        observed,
                        &countries,
                        &mut resolved,
                        &mut buffers,
                        &mut state,
                        &mut carrier,
                        slots,
                        None,
                    )?;
                    continue;
                };
                if reader.reuse_registration(
                    observed,
                    &countries,
                    &mut resolved,
                    &mut buffers,
                    &mut state,
                    slots,
                    reuse,
                ) {
                    continue;
                }
                reuse.begin(handle, pointer);
                let before = (
                    state.len,
                    state.unread_rows,
                    state.wings_len,
                    state.required_pools,
                    state.required_wings,
                );
                reader.registration(
                    observed,
                    &countries,
                    &mut resolved,
                    &mut buffers,
                    &mut state,
                    &mut carrier,
                    slots,
                    Some(&mut *reuse),
                )?;
                if state.unread_rows == before.1 {
                    reuse.walked(WalkedBase {
                        slots: state.last_slots,
                        sought_plan: slots.dense,
                        plan: state.last_plan.map(|len| &state.scratch.plan[..len]),
                        rows: &buffers.registrations[before.0..state.len],
                        wings_start: before.2,
                        wings: &buffers.wings[before.2..state.wings_len],
                        pools: state.required_pools - before.3,
                        required_wings: state.required_wings - before.4,
                    });
                }
            }
        }
        Ok(if state.unread_rows == 0 {
            Outcome::Rows { len: state.len }
        } else {
            Outcome::Truncated {
                len: state.len,
                unread_rows: state.unread_rows,
            }
        })
    })();
    if let Some(carrier) = carrier {
        carrier.finish_walk();
    }
    RegistrationWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        wings_len: state.wings_len,
        required_wings: state.required_wings,
        required_links,
        required_bases: base_ordinal,
        required_indexed: state.required_indexed,
        required_pools: state.required_pools,
        dense_bases: state.dense_bases,
        reused_bases: state.reused_bases,
    }
}

pub fn carrier_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<CarrierDemand, &'static str> {
    engine_allocation::advisory(|| carrier_demand_inner(memory, base, state))
}

fn carrier_demand_inner(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<CarrierDemand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let walked = walk_registrations(
        memory,
        base,
        state,
        RegistrationBuffers {
            registrations: &mut [],
            wings: &mut [],
            resolved: &mut [],
        },
    );
    let registrations = match walked.outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("carrier registration sizing overflow")?,
        Outcome::Unavailable { reason } => return Err(reason),
    };
    let fuel_memos = walked
        .required_wings
        .checked_mul(2)
        .ok_or("carrier fuel memo demand overflow")?;
    let mut demand = CarrierDemand {
        bases: walked.required_bases,
        registrations,
        pools: walked.required_pools,
        indexed: walked.required_indexed,
        resolved: walked.required_links,
        country_terms: countries.arrays()[0].count as usize,
        aces: fuel_memos,
        forecasts: fuel_memos,
        ..Default::default()
    };
    let extras = (|| {
        let reader = Reader { memory, base };
        let manager = reader.pointer(state, cgame_state::STRATEGIC_AIR_MANAGER)?;
        reader.vtable(manager, cstrategic_air_manager::VTABLE_RVA)?;
        let hosts = reader.array(manager, cstrategic_air_manager::STATELESS_AIR_BASES)?;
        for slot in 0..hosts.count {
            let pointer = hosts
                .element(slot, 8)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|pointer| *pointer != 0)
                .ok_or("carrier sizing stateless root unreadable or null")?;
            reader.vtable(pointer, VTABLE_RVA)?;
            match carrier::dense_demand(memory, pointer).and_then(|count| {
                demand
                    .countries
                    .checked_add(count)
                    .ok_or("carrier dense country sizing overflow")
            }) {
                Ok(count) => demand.countries = count,
                Err(reason) => {
                    demand.failure.get_or_insert(reason);
                }
            }
            match reader
                .row(pointer, None)
                .and_then(|source| carrier_host_name_demand(memory, base, &source))
                .and_then(|bytes| {
                    demand
                        .name_bytes
                        .checked_add(bytes)
                        .ok_or("carrier name sizing overflow")
                }) {
                Ok(bytes) => demand.name_bytes = bytes,
                Err(reason) => {
                    demand.failure.get_or_insert(reason);
                }
            }
        }
        Ok::<(), &'static str>(())
    })();
    if let Err(reason) = extras {
        demand.failure.get_or_insert(reason);
    }
    if let Err(reason) = demand::raw_demand(
        memory,
        base,
        state,
        &countries,
        walked.required_wings,
        &mut demand,
    ) {
        demand.failure.get_or_insert(reason);
    }
    Ok(demand)
}

#[cfg(test)]
mod tests;
