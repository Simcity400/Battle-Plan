//! CAirBase is shared by state and host bases.
//! Country-indexed containers differ from the dense registration list.

use crate::{
    Memory, cair_wing, cair_wing_pool, ccountry, ccountry_air_container, cgame_state,
    cpdx_array::Array,
    cprovince, cstate, cstrategic_air_manager,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
    walk::Outcome,
};

mod carrier;
mod carrier_capture;
mod demand;
mod reuse;
use crate::cair_wing_pool::fuel;
pub use carrier_capture::{
    CarrierAceSlot, CarrierBase, CarrierBuffers, CarrierCapture, CarrierCountryTerms,
    CarrierDemand, CarrierForecastSlot, CarrierIndexedCertificate, CarrierLengths, CarrierPool,
    CarrierPoolFuel, CarrierRegistration, ContainerFuel, CountryOccurrence, GrowingCarrierBuffers,
};
pub use reuse::{HeldBase, RegistrationReuse, WalkedBase};

/// Off-thread host-name storage hint for one independently authenticated base
/// row. A fresh reference resolver is scoped to this hint only; capture must
/// read/authenticate its own source again and never substitute these bytes.
pub fn carrier_host_name_demand(
    memory: &impl Memory,
    image: usize,
    source: &Row,
) -> Result<usize, &'static str> {
    let mut resolver = crate::creference_registry::ScopedResolver::new(image);
    carrier::host_name_demand(memory, image, source, &mut resolver)
}

/// Air-base vtable.
pub const VTABLE_RVA: usize = 0x0294_1fa0;
/// Air-base allocation size.
pub const SIZE: usize = 0x90;
/// Reference type value checked against the low handle word.
pub const TYPE: u32 = 65;
/// Full handle containing u32 type and id.
pub const HANDLE: usize = 0x08;
/// Country-ordinal container-pointer vector; a nonnull slot grants access.
pub const COUNTRY_CONTAINERS: usize = 0x30;
/// Country-container capacity, u32.
pub const COUNTRY_CONTAINERS_CAPACITY: usize = 0x38;
/// Country-container count, u32, bounding resolved ordinals.
pub const COUNTRY_CONTAINERS_COUNT: usize = 0x3c;
/// Dense registered-container vector data, appended on creation.
pub const REGISTERED_CONTAINERS: usize = 0x48;
/// CAirBase host paired-u32 reference, including carrier CShip handle; AddCarrierAirBase writes it per RE_NON_STATE_AIR_BASE.
pub const HOST_REFERENCE: usize = 0x60;
/// CState pointer, null for host bases.
pub const STATE: usize = 0x68;
/// Province pointer or null; do not invent host locations.
pub const PROVINCE: usize = 0x70;
/// Capacity, i32, used by signed division.
pub const CAPACITY: usize = 0x78;
/// Signed base kind, i32, set by construction; dispatch branches on zero, one and two.
pub const KIND: usize = 0x7c;
/// Per-country fuel sum following registered containers and the state veto.
pub const FUEL_SUM_RVA: usize = 0x00c3_d2d0;
/// State-base constructor initializing country-container vectors.
pub const STATE_CONSTRUCTOR_RVA: usize = 0x00c3_8200;
/// Stateless host-base constructor initializing country-container vectors.
pub const HOST_CONSTRUCTOR_RVA: usize = 0x00c3_83c0;
/// Container allocation and indexing path.
pub const CREATE_COUNTRY_CONTAINER_RVA: usize = 0x00c3_c2b0;
/// Pool registration resolving the owner ordinal and appending to its container.
pub const REGISTER_POOL_RVA: usize = 0x00c3_c860;
/// Indexed country pool-vector getter.
pub const COUNTRY_POOLS_RVA: usize = 0x00c4_3f70;
/// Access query testing the country ordinal, count and slot.
pub const HAS_ACCESS_RVA: usize = 0x00c4_7cc0;
/// Registered-container repair pass resolving each country handle.
pub const REPAIR_CONTAINERS_RVA: usize = 0x00c4_9c40;
/// Pool erasure scanning the country's registered vector.
pub const ERASE_POOL_RVA: usize = 0x00c5_4220;
/// Pool batch width for the stored wing vector.
const BATCH: usize = 64;
/// Wing identity span.
const WING_RECORD_BYTES: usize = 40;
/// Exactly the identity span the registration rows need from one wing.
const WING_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 32,
    },
    RecordField {
        source_offset: crate::cair_wing::POOL,
        destination_offset: 32,
        len: 8,
    },
];
const WING_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&WING_FIELDS, WING_RECORD_BYTES).expect("valid record layout");
/// Pool spans containing the primary vtable and wing-vector controls.
const POOL_RECORD_BYTES: usize = 24;
/// Exactly the vtable and wing-vector header the registration walk needs from one pool.
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
/// One country-container slot pointer. State and host constructors size
/// this vector to the full country-slot count; read it in strided chunks.
const CONTAINER_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const CONTAINER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CONTAINER_FIELD, 8).expect("valid record layout");

/// Containers a base's dense list may name for the walk to read only their
/// slot words ([`dense_plan`]); a longer list takes the full slot scan. A
/// bound on this shortcut only, never on what a walk records.
pub const DENSE_BOUND: usize = 64;

/// Why a base takes the full slot scan instead of its dense list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum DenseRefusal {
    /// The dense header (`REGISTERED_CONTAINERS`) or its entries did not read or are invalid.
    Header = 0,
    /// The list names more containers than the slot vector has slots.
    LongerThanSlots = 1,
    /// More than [`DENSE_BOUND`] listed containers.
    OverBound = 2,
    /// A null entry, or a container listed twice.
    Entry = 3,
    /// A listed container's identity or span did not read or authenticate.
    Container = 4,
    /// A listed container's country names no country slot.
    Country = 5,
    /// A listed container's slot lies beyond the slot vector.
    SlotRange = 6,
    /// Two listed containers name one slot.
    SharedSlot = 7,
    /// A listed container is not at the slot its country names.
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

/// One listed container and the `COUNTRY_CONTAINERS` slot it occupies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DenseEntry {
    pub slot: u32,
    pub container: usize,
}

/// How the registration walk finds each base's occupied container slots.
#[derive(Clone, Copy, Debug, Default)]
pub struct SlotMode<'a> {
    /// Read only the slot words a base's dense list names ([`dense_plan`]);
    /// a base whose list does not prove its slots reads every chunk.
    pub dense: bool,
    /// Bases (by handle) that read every chunk regardless.
    pub full: &'a [u64],
}

/// The occupied slots of one base's country-container vector from its dense registered-container
/// list, sorted by slot, when the list proves them; every other slot is then null.
///
/// At run time only container creation (which stores the new container at its country's canonical
/// slot and appends it to `REGISTERED_CONTAINERS`) and country-access removal (which nulls the slot
/// and erases every occurrence from `REGISTERED_CONTAINERS`) write the slots; the save reader
/// rebuilds both. So the listed containers are the occupied slots, each at the slot its own country
/// reference names in the country link table. Each listed container's identity and span are the
/// exact reads the slot loop makes, its slot comes from that link entry, and its slot word must
/// hold it. Any other result refuses, and the base takes the full scan, whose outcome and reasons
/// are the original walk's.
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
    let expected = image
        .checked_add(ccountry_air_container::VTABLE_RVA)
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
        // Container creation files a zero id at slot zero and any other id at
        // its link entry, which is zero for an id not yet linked.
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
    /// Full native reference, without pointer-derived generations.
    pub id: u64,
    pub state: Option<u32>,
    pub province: Option<u32>,
    /// Native reference bytes, including the shared null reference.
    pub host: u64,
    pub capacity: i32,
    /// Zero is the ordinary aircraft base. Preserve other native kinds.
    pub kind: i32,
}

/// Reads all three state-indexed kinds and stateless base membership once. State slots may be
/// null; the stateless vector is dense and its nulls are failures. Occupancy is
/// derived from the captured wing roster, and building levels from Buildings,
/// so neither source is walked again here. Duplicate handles are checked after
/// capture. Capacity omits whole rows and counts every remaining nonnull slot.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
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
                // Carrier-base creation inserts into the stateless vector.
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

    fn vtable(&self, pointer: usize, rva: usize) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err("air base or linked object vtable mismatch")
        }
    }

    fn row(&self, pointer: usize, state_slot: Option<u32>) -> Result<Row, &'static str> {
        let mut identity = [0; 16];
        // Exact consecutive fields: host/state/province/capacity/kind.
        let mut data = [0; 32];
        let complete = self.memory.copy_scattered(
            [pointer, pointer.checked_add(HOST_REFERENCE).unwrap_or(0)],
            &mut [&mut identity, &mut data],
        );
        let expected = self
            .base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
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
        slots: SlotMode<'_>,
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
        // A base whose dense list proves its occupied slots reads none of its
        // slot vector beyond the plan's slot words: the listed containers are
        // the occupied slots, every other one is null ([`dense_plan`]).
        let planned = (slots.dense && !slots.full.contains(&base_handle))
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
        // Pool and wing pointers with their fixed fields are gathered in
        // 64-row batches instead of one guarded read per object; every pool
        // and wing is still validated and staged in native pool then wing
        // order with its own copied prefix, so rows and single-fault reasons
        // keep their positions.
        let expected_pool = self
            .base
            .checked_add(cair_wing_pool::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let expected_primary = self
            .base
            .checked_add(cair_wing::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let expected_secondary = self
            .base
            .checked_add(cair_wing::SECONDARY_VTABLE_RVA)
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
        // The country-slot vector is copied in strided 64-slot chunks, one
        // guarded round each, matching the raw acquisition's chunk takes; a
        // planned base's chunk is laid out from its plan instead. Each
        // container is still completed before the next copied slot is
        // examined, and the first uncopied slot fails at its own position, so
        // a later pointer fault cannot hide an earlier container failure or
        // its staged demand.
        let mut chunk_first = 0;
        while chunk_first < containers.count {
            let chunk = ((containers.count - chunk_first) as usize).min(BATCH);
            let copied = match planned {
                Some(len) => {
                    // A chunk naming no listed container is null throughout.
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
                // A null slot means no access. An absent container is measured absence
                // rather than a torn read.
                if container == 0 {
                    continue;
                }
                self.vtable(container, ccountry_air_container::VTABLE_RVA)?;
                // One span holds the country reference, the capacity word the country-pool
                // registration gate tests and the fuel scalars the carrier observer records.
                let mut span = [0u8; CONTAINER_SPAN];
                if !container
                    .checked_add(ccountry_air_container::COUNTRY)
                    .is_some_and(|at| read_slice(self.memory, at, &mut span))
                {
                    return Err("air base container country unreadable");
                }
                let country = u32::from_le_bytes(span[..4].try_into().unwrap());
                let (capacity_raw, fuel) = container_span_words(&span);
                // Container creation installs the container at the ordinal its
                // own country reference resolves to, so the two must still
                // agree. At reserved slot zero the callers short-circuit an id
                // of zero to that ordinal without consulting the link table, so
                // the reference there must be zero or not yet linked; one that
                // does resolve names a country whose own slot is positive and
                // cannot be filed here. The raw reference is recorded as stored
                // on every accepted path.
                if slot == 0 {
                    // Only a readable zero link entry is an unlinked id; an
                    // unreadable or out-of-range entry stays a fault rather than
                    // being folded into an accept here.
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

/// Bytes from the container's country reference to the end of its allocated fuel word, the end of
/// the 0xF8-byte container, read as one span: the country, the capacity word, and the fuel demand
/// and allocation the carrier observer records.
pub const CONTAINER_SPAN: usize =
    ccountry_air_container::FUEL_ALLOCATED_RAW + 8 - ccountry_air_container::COUNTRY;

/// The capacity word and both fuel scalars of one container span.
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

/// One country's registered pool membership at one base, in native order.
///
/// `slot` is the indexed-container ordinal used by creation and access
/// checks. Authenticate the container's country reference against it; do
/// not retain that reference here. Flatten wings in pool order without
/// recording pool boundaries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationRow {
    /// Full native CAirBase handle (`HANDLE`).
    pub base: u64,
    pub slot: u32,
    /// Container capacity word; admission requires a positive value.
    pub capacity_raw: i64,
    pub wings_start: usize,
    pub wings_len: usize,
}

/// Caller-owned storage for the registration walk. Allocation is off-thread.
pub struct RegistrationBuffers<'a> {
    pub registrations: &'a mut [RegistrationRow],
    pub wings: &'a mut [u64],
    /// Dense per-walk resolution cache indexed by raw country link id. Size it
    /// to [`RegistrationWalk::required_links`] off the game thread; ids beyond
    /// a shorter slice (including an empty one) resolve uncached with the same
    /// reads and results, so a small cache never refuses or changes a row.
    pub resolved: &'a mut [ResolvedLink],
}

/// One dense resolver cell. `None` is an id not yet resolved in this walk.
pub type ResolvedLink = Option<ccountry::Country>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationWalk {
    pub outcome: Outcome,
    pub wings_len: usize,
    /// Full native wing demand, including whole rows omitted for space.
    pub required_wings: usize,
    /// Live `CGameState` country-link count: the resolver length at which
    /// every raw id this walk can resolve is cached. Zero before the country
    /// headers were read.
    pub required_links: usize,
    /// Off-hour optional carrier storage floors from this same complete walk.
    pub required_bases: usize,
    pub required_indexed: usize,
    pub required_pools: usize,
    /// Bases whose dense list proved their occupied slots ([`SlotMode`]).
    pub dense_bases: usize,
    /// State bases whose earlier output was appended ([`RegistrationReuse`]).
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
    /// The slot count and plan length of the base last begun.
    last_slots: u32,
    last_plan: Option<usize>,
    scratch: RegistrationScratch,
}

/// Reused only within this walk. Each pointer block is fully copied before
/// parsing, and record prefixes are checked before decoding. Shorter batches
/// never consume old tails, so these buffers need initialization only once.
struct RegistrationScratch {
    container_bytes: [u8; BATCH * 8],
    pool_pointers: [usize; BATCH],
    pool_records: [u8; BATCH * POOL_RECORD_BYTES],
    pool_pointer_bytes: [u8; BATCH * 8],
    wing_pointers: [usize; BATCH],
    wing_records: [u8; BATCH * WING_RECORD_BYTES],
    wing_pointer_bytes: [u8; BATCH * 8],
    /// The current base's listed containers ([`dense_plan`]).
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

/// A dense cache of raw-reference resolutions, local to one walk.
///
/// Every air container names one of the same country targets, so the walk would
/// otherwise re-resolve each reference once per container. A successful resolve
/// implies `raw < links.count`, so a cache of the live link count holds every
/// resolvable id without collisions. Its cells are cleared when the walk starts,
/// so it reuses successful authentication only while the hourly world is
/// stable; it cannot detect a later mutation or a newly unreadable field within
/// this walk. Ids outside the caller's slice run the original validator.
struct Resolver<'a> {
    cells: crate::walk::Buffer<'a, ResolvedLink>,
}

impl Resolver<'_> {
    fn resolve(
        &mut self,
        countries: &ccountry::Reader<'_, impl Memory>,
        raw: u32,
    ) -> Result<ccountry::Country, &'static str> {
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

/// Read every air base's per-country registered pool membership.
///
/// The four AirBases roots let registration rows join by handle. Displayed
/// wings use indexed registration; assigning a pool's base can precede a
/// registration refusal. An AirWings base reference cannot replace membership.
///
/// A null container means no access and emits no row. Null counted pools or
/// wings, bad vtables, zero wing handles and invalid vector headers refuse
/// the table. Capacity omits whole container rows, releases staged wings and
/// reports full remaining demand.
pub fn walk_registrations<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
) -> RegistrationWalk {
    walk_registrations_planned(memory, base, game_state, buffers, SlotMode::default(), None)
}

/// [`walk_registrations`] finding each base's occupied slots as `slots`
/// says. A base its dense list proves reads only its listed containers' slot
/// words; the rows, wings and outcome are those of the full scan. With
/// `reuse`, a state base whose subtree it says this memory holds as an earlier
/// walk read it appends that walk's output instead of walking it again
/// ([`RegistrationReuse`]), and each state base walked is reported to it.
pub fn walk_registrations_planned<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
    slots: SlotMode<'_>,
    reuse: Option<&mut (dyn RegistrationReuse + '_)>,
) -> RegistrationWalk {
    walk_registrations_inner::<M, M>(memory, base, game_state, buffers, &[], None, slots, reuse)
}

/// Feeds the fixed optional observer at the existing read/authentication
/// points. `base_rows` is the accepted AirBases prefix in original traversal
/// order from this exact sample, never carried rows or a sorted copy. Only the
/// O(1) matching row join permits host-name reads. All optional source reads
/// use the observer's distinct Memory, which is owned-only on a decoder.
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

/// [`walk_registrations_with_carrier`] finding each base's occupied slots as
/// `slots` says, appending earlier outputs `reuse` offers
/// ([`walk_registrations_planned`]). The observer sees the same containers in
/// the same order; it observes no state base.
#[allow(clippy::too_many_arguments)]
pub fn walk_registrations_with_carrier_planned<M: Memory, O: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    buffers: RegistrationBuffers<'_>,
    base_rows: &[Row],
    carrier: &mut CarrierCapture<'_, O>,
    slots: SlotMode<'_>,
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
    slots: SlotMode<'_>,
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
            cells: crate::walk::Buffer::Fixed(&mut cells[..used]),
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
                let expected = base
                    .checked_add(VTABLE_RVA)
                    .ok_or("module address overflow")?;
                if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
                    return Err("air base vtable mismatch");
                }
                let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
                if handle == 0 {
                    return Err("nonnull air base has null handle");
                }
                let source = base_rows.get(base_ordinal).copied();
                // Four u32-count roots cannot overflow the pinned 64-bit
                // address space. This adds no failure to the original walk.
                base_ordinal += 1;
                if let Some(carrier) = carrier.as_deref_mut() {
                    carrier.begin_base(pointer, handle, !indexed);
                    if let Some(source) = source.filter(|source| source.id == handle) {
                        carrier.authenticated_base(source);
                    }
                }
                let observed = RegistrationBase { handle, pointer };
                // A state base's output depends on its subtree and the
                // country tables alone; the carrier observes none of it.
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
                        sought_plan: slots.dense && !slots.full.contains(&handle),
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

/// Presize optional fuel storage before mid-game attach's first callback.
/// The original registration walk authenticates every member even with empty
/// output. Existing passive fuel readers also measure optional raw
/// requests; the hint retains no payload and calls no game getter.
pub fn carrier_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<CarrierDemand, &'static str> {
    crate::allocation::advisory(|| carrier_demand_inner(memory, base, state))
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
    // The complete original membership authenticated these stateless roots.
    // Dense counts retain repeated occurrences; retained names use the same
    // cached-name helper as capture. These hints copy no country/name payload.
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
