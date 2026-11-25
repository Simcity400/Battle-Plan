
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    State,
    StatePool,
    OriginState,
    Origin,
    CountryResources,
    StrategicAir,
    CountryTag,
    EveryCountry,
    PoolCapacity,
    Province,
    Route,
    OriginRoute,
    RouteSender,
    EveryRoute,
}

pub const MUTATIONS: [(MutationSite, Receiver); 35] = [
    (
        MutationSite {
            name: "refresh_resource_modifier_gates",
            rva: Rva(0xa2_aff0),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "add_building_dynamic_modifier",
            rva: Rva(0xa1_c5a0),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "remove_building_dynamic_modifier",
            rva: Rva(0xa2_8210),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "add_resource",
            rva: Rva(0xa1_bee0),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "expire_timed_resource",
            rva: Rva(0xc2_0fd0),
        },
        Receiver::StatePool,
    ),
    (
        MutationSite {
            name: "destroy_state",
            rva: Rva(0xa1_b040),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "add_core",
            rva: Rva(0xa1_c3f0),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "remove_core",
            rva: Rva(0xa2_8180),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "construct_resource_origin",
            rva: Rva(0xd0_4290),
        },
        Receiver::OriginState,
    ),
    (
        MutationSite {
            name: "destroy_resource_origin",
            rva: Rva(0xd0_4cf0),
        },
        Receiver::Origin,
    ),
    (
        MutationSite {
            name: "add_building_resource_modifier",
            rva: Rva(0xd0_5640),
        },
        Receiver::CountryResources,
    ),
    (
        MutationSite {
            name: "clear_country_resources",
            rva: Rva(0xd0_a270),
        },
        Receiver::CountryResources,
    ),
    (
        MutationSite {
            name: "construct_strategic_air",
            rva: Rva(0xca_4660),
        },
        Receiver::CountryTag,
    ),
    (
        MutationSite {
            name: "append_air_site",
            rva: Rva(0xca_8330),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "append_missile_site",
            rva: Rva(0xca_9110),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "refresh_air_base_access",
            rva: Rva(0xcc_1a80),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "populate_air_sites",
            rva: Rva(0xcb_7c00),
        },
        Receiver::EveryCountry,
    ),
    (
        MutationSite {
            name: "repair_air_base_containers",
            rva: Rva(0xcb_61a0),
        },
        Receiver::EveryCountry,
    ),
    (
        MutationSite {
            name: "withdraw_air_site",
            rva: Rva(0xcb_fb00),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "withdraw_missile_site",
            rva: Rva(0xcc_0610),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "transfer_air_base",
            rva: Rva(0xcc_11a0),
        },
        Receiver::CountryTag,
    ),
    (
        MutationSite {
            name: "clear_air_sites",
            rva: Rva(0xca_c4c0),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "clear_missile_sites",
            rva: Rva(0xca_c720),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "destroy_strategic_air",
            rva: Rva(0xca_57d0),
        },
        Receiver::StrategicAir,
    ),
    (
        MutationSite {
            name: "set_country_pool_capacity",
            rva: Rva(0xcc_0d40),
        },
        Receiver::PoolCapacity,
    ),
    (
        MutationSite {
            name: "add_claim",
            rva: Rva(0xa1_c2a0),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "remove_claim",
            rva: Rva(0xa2_7fe0),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "add_state_strategic_location",
            rva: Rva(0xa1_c640),
        },
        Receiver::State,
    ),
    (
        MutationSite {
            name: "add_province_strategic_location",
            rva: Rva(0xee_b010),
        },
        Receiver::Province,
    ),
    (
        MutationSite {
            name: "update_trade_route",
            rva: Rva(0xd1_c280),
        },
        Receiver::Route,
    ),
    (
        MutationSite {
            name: "construct_trade_route",
            rva: Rva(0xd0_40d0),
        },
        Receiver::RouteSender,
    ),
    (
        MutationSite {
            name: "construct_country_resources",
            rva: Rva(0xd0_3a70),
        },
        Receiver::RouteSender,
    ),
    (
        MutationSite {
            name: "load_trade_route",
            rva: Rva(0xd1_7c20),
        },
        Receiver::EveryRoute,
    ),
    (
        MutationSite {
            name: "update_supply_trade_route",
            rva: Rva(0xd1_c7c0),
        },
        Receiver::Route,
    ),
    (
        MutationSite {
            name: "refresh_origin_trade_route",
            rva: Rva(0xd0_93a0),
        },
        Receiver::OriginRoute,
    ),
];

pub const FIRST_ROUTE_SITE: usize = 29;
pub const ORIGIN_ROUTE_SITE: usize = 34;
pub const ORIGIN_OWNER: usize = 0x18;

pub fn origin_route_kind(memory: &impl engine_native_memory::Memory, origin: usize) -> Option<u8> {
    engine_native_memory::read(
        memory,
        origin
            .checked_add(crate::cresource_origin::DELIVERY_ROUTE)?
            .checked_add(crate::resource_map::ROUTE_KIND)?,
    )
}

pub fn route_country(
    memory: &impl engine_native_memory::Memory,
    receiver: Receiver,
    first: usize,
    second: usize,
) -> Result<Option<u32>, Unreadable> {
    let link = match receiver {
        Receiver::Route => first.checked_add(crate::resource_map::ROUTE_LINKS[0]),
        Receiver::OriginRoute => first.checked_add(ORIGIN_OWNER),
        Receiver::RouteSender => Some(second),
        _ => None,
    };
    link.and_then(|address| engine_native_memory::read::<u32>(memory, address))
        .map(|raw| (raw != 0).then_some(raw))
        .ok_or(Unreadable)
}

pub const DAILY_CAPACITIES: MutationSite = MutationSite {
    name: "daily_refresh_base_capacities",
    rva: Rva(0xca_9400),
};

const DAILY_BASES: usize = 0x10;
const CAPACITY_PAIR_VECTOR_STRIDE: usize = 0x18;
const CAPACITY_PAIR_COUNT: usize = 0xc;
const CAPACITY_PAIR_VALUE: usize = 8;

pub fn daily_unchanged(
    memory: &impl engine_native_memory::Memory,
    module: usize,
    air: usize,
    pairs: usize,
) -> Option<u64> {
    use engine_native_memory::{
        cpdx_array::Array,
        memory::{RecordField, RecordLayout, RecordSources},
        read, read_slice,
    };
    const BATCH: usize = 64;
    const PENDING: usize = 64;
    const LINK_CACHE: usize = 512;
    const BASE_FIELDS: [RecordField; 2] = [
        RecordField {
            source_offset: engine_native_air::cair_base::COUNTRY_CONTAINERS,
            destination_offset: 0,
            len: 16,
        },
        RecordField {
            source_offset: engine_native_air::cair_base::STATE,
            destination_offset: 16,
            len: 8,
        },
    ];
    const BASE_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&BASE_FIELDS, 24).expect("valid air base layout");
    const PAIR_FIELDS: [RecordField; 1] = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 16,
    }];
    const PAIR_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&PAIR_FIELDS, 16).expect("valid pair layout");
    const PTR_FIELDS: [RecordField; 1] = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }];
    const PTR_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&PTR_FIELDS, 8).expect("valid pointer layout");
    const CAP_FIELDS: [RecordField; 1] = [RecordField {
        source_offset: engine_native_air::ccountry_air_container::CAPACITY_RAW,
        destination_offset: 0,
        len: 8,
    }];
    const CAP_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&CAP_FIELDS, 8).expect("valid capacity layout");

    let singleton_addr = engine_sites::at(module, engine_native::cgame_state::SINGLETON_RVA)?;
    let country_addr = air.checked_add(engine_native_air::cstrategic_air::COUNTRY)?;
    let bases_addr = air.checked_add(DAILY_BASES)?;
    let mut game_bytes = [0_u8; 8];
    let mut own_bytes = [0_u8; 4];
    let mut air_bases = [0_u8; 16];
    let mut vectors_bytes = [0_u8; 8];
    if memory.copy_scattered(
        [singleton_addr, country_addr, bases_addr, pairs],
        &mut [
            &mut game_bytes[..],
            &mut own_bytes[..],
            &mut air_bases[..],
            &mut vectors_bytes[..],
        ],
    ) < 4
    {
        return None;
    }
    let count = i32::from_le_bytes(air_bases[12..16].try_into().unwrap()).max(0) as usize;
    if count == 0 {
        return Some(0);
    }
    let game = u64::from_le_bytes(game_bytes) as usize;
    let own = u32::from_le_bytes(own_bytes);
    let bases = u64::from_le_bytes(air_bases[..8].try_into().unwrap()) as usize;
    let vectors = u64::from_le_bytes(vectors_bytes) as usize;

    let links = Array::read(
        memory,
        game.checked_add(engine_native::cgame_state::COUNTRY_LINKS)?,
        4,
    )?;
    let cached_links = (links.count as usize).min(LINK_CACHE);
    let mut link_bytes = [0_u8; LINK_CACHE * 4];
    if cached_links > 0 && !read_slice(memory, links.data, &mut link_bytes[..cached_links * 4]) {
        return None;
    }
    let resolve_slot = |raw: u32| -> Option<u32> {
        if raw == 0 {
            Some(0)
        } else if (raw as usize) < cached_links {
            let at = (raw as usize) * 4;
            Some(u32::from_le_bytes(link_bytes[at..at + 4].try_into().unwrap()))
        } else {
            read::<u32>(memory, links.element(raw, 4)?)
        }
    };
    let own_slot = resolve_slot(own);

    let mut base_raw = [0_u8; BATCH * 8];
    let mut base_ptrs = [0_usize; BATCH];
    let mut vec_raw = [0_u8; BATCH * CAPACITY_PAIR_VECTOR_STRIDE];
    let mut base_records = [0_u8; BATCH * 24];

    let mut pair_ptrs = [0_usize; PENDING];
    let mut pair_containers = [Array { data: 0, count: 0 }; PENDING];
    let mut pair_bytes = [0_u8; PENDING * 16];
    let mut pair_len = 0_usize;

    let mut elem_ptrs = [0_usize; PENDING];
    let mut expected = [0_i64; PENDING];
    let mut is_own = [false; PENDING];
    let mut container_bytes = [0_u8; PENDING * 8];
    let mut container_ptrs = [0_usize; PENDING];
    let mut cap_bytes = [0_u8; PENDING * 8];
    let mut check_len = 0_usize;

    let flush_checks = |check_len: &mut usize,
                        elem_ptrs: &[usize; PENDING],
                        expected: &[i64; PENDING],
                        is_own: &[bool; PENDING],
                        container_bytes: &mut [u8; PENDING * 8],
                        container_ptrs: &mut [usize; PENDING],
                        cap_bytes: &mut [u8; PENDING * 8]|
     -> bool {
        let total = *check_len;
        if total == 0 {
            return true;
        }
        *check_len = 0;
        if memory.copy_records(
            RecordSources::Pointers(&elem_ptrs[..total]),
            &PTR_LAYOUT,
            &mut container_bytes[..total * 8],
        ) < total
        {
            return false;
        }
        for (j, chunk) in container_bytes[..total * 8].chunks_exact(8).enumerate() {
            let ptr = u64::from_le_bytes(chunk.try_into().unwrap()) as usize;
            if ptr == 0 {
                return false;
            }
            container_ptrs[j] = ptr;
        }
        if memory.copy_records(
            RecordSources::Pointers(&container_ptrs[..total]),
            &CAP_LAYOUT,
            &mut cap_bytes[..total * 8],
        ) < total
        {
            return false;
        }
        for j in 0..total {
            let actual = i64::from_le_bytes(cap_bytes[j * 8..(j + 1) * 8].try_into().unwrap());
            if is_own[j] {
                if actual < 1 {
                    return false;
                }
            } else if actual != expected[j] {
                return false;
            }
        }
        true
    };

    let flush_pairs = |pair_len: &mut usize,
                       pair_ptrs: &[usize; PENDING],
                       pair_containers: &[Array; PENDING],
                       pair_bytes: &mut [u8; PENDING * 16],
                       check_len: &mut usize,
                       elem_ptrs: &mut [usize; PENDING],
                       expected: &mut [i64; PENDING],
                       is_own: &mut [bool; PENDING],
                       container_bytes: &mut [u8; PENDING * 8],
                       container_ptrs: &mut [usize; PENDING],
                       cap_bytes: &mut [u8; PENDING * 8]|
     -> bool {
        let total = *pair_len;
        if total == 0 {
            return true;
        }
        *pair_len = 0;
        if memory.copy_records(
            RecordSources::Pointers(&pair_ptrs[..total]),
            &PAIR_LAYOUT,
            &mut pair_bytes[..total * 16],
        ) < total
        {
            return false;
        }
        for (j, chunk) in pair_bytes[..total * 16].chunks_exact(16).enumerate() {
            let raw = u32::from_le_bytes(chunk[..4].try_into().unwrap());
            let want = i64::from_le_bytes(chunk[CAPACITY_PAIR_VALUE..].try_into().unwrap());
            let Some(slot) = resolve_slot(raw) else {
                return false;
            };
            let Some(elem) = pair_containers[j].element(slot, 8) else {
                return false;
            };
            elem_ptrs[*check_len] = elem;
            expected[*check_len] = want;
            is_own[*check_len] = false;
            *check_len += 1;
            if *check_len == PENDING
                && !flush_checks(
                    check_len,
                    elem_ptrs,
                    expected,
                    is_own,
                    container_bytes,
                    container_ptrs,
                    cap_bytes,
                )
            {
                return false;
            }
        }
        true
    };

    let mut stores = 0_u64;
    let mut base_first = 0;
    while base_first < count {
        let batch = (count - base_first).min(BATCH);
        let base_addr = bases.checked_add(base_first.checked_mul(8)?)?;
        let vec_addr = vectors.checked_add(base_first.checked_mul(CAPACITY_PAIR_VECTOR_STRIDE)?)?;
        if memory.copy_scattered(
            [base_addr, vec_addr],
            &mut [
                &mut base_raw[..batch * 8],
                &mut vec_raw[..batch * CAPACITY_PAIR_VECTOR_STRIDE],
            ],
        ) < 2
        {
            return None;
        }
        for (dst, chunk) in base_ptrs[..batch]
            .iter_mut()
            .zip(base_raw[..batch * 8].chunks_exact(8))
        {
            *dst = u64::from_le_bytes(chunk.try_into().unwrap()) as usize;
        }
        if memory.copy_records(
            RecordSources::Pointers(&base_ptrs[..batch]),
            &BASE_LAYOUT,
            &mut base_records[..batch * 24],
        ) < batch * BASE_LAYOUT.field_count()
        {
            return None;
        }
        for i in 0..batch {
            let rec = &base_records[i * 24..(i + 1) * 24];
            let containers = Array::from_header(rec[..16].try_into().unwrap(), 8)?;
            let state = u64::from_le_bytes(rec[16..24].try_into().unwrap());
            let vec_rec =
                &vec_raw[i * CAPACITY_PAIR_VECTOR_STRIDE..(i + 1) * CAPACITY_PAIR_VECTOR_STRIDE];
            let data = u64::from_le_bytes(vec_rec[..8].try_into().unwrap()) as usize;
            let entries = usize::try_from(i32::from_le_bytes(
                vec_rec[CAPACITY_PAIR_COUNT..CAPACITY_PAIR_COUNT + 4]
                    .try_into()
                    .unwrap(),
            ))
            .ok()?;
            for entry in 0..entries {
                pair_ptrs[pair_len] = data.checked_add(entry.checked_mul(16)?)?;
                pair_containers[pair_len] = containers;
                pair_len += 1;
                if pair_len == PENDING
                    && !flush_pairs(
                        &mut pair_len,
                        &pair_ptrs,
                        &pair_containers,
                        &mut pair_bytes,
                        &mut check_len,
                        &mut elem_ptrs,
                        &mut expected,
                        &mut is_own,
                        &mut container_bytes,
                        &mut container_ptrs,
                        &mut cap_bytes,
                    )
                {
                    return None;
                }
            }
            stores += entries as u64;
            if state != 0 {
                elem_ptrs[check_len] = containers.element(own_slot?, 8)?;
                expected[check_len] = 0;
                is_own[check_len] = true;
                check_len += 1;
                if check_len == PENDING
                    && !flush_checks(
                        &mut check_len,
                        &elem_ptrs,
                        &expected,
                        &is_own,
                        &mut container_bytes,
                        &mut container_ptrs,
                        &mut cap_bytes,
                    )
                {
                    return None;
                }
            }
        }
        base_first += batch;
    }
    if !flush_pairs(
        &mut pair_len,
        &pair_ptrs,
        &pair_containers,
        &mut pair_bytes,
        &mut check_len,
        &mut elem_ptrs,
        &mut expected,
        &mut is_own,
        &mut container_bytes,
        &mut container_ptrs,
        &mut cap_bytes,
    ) || !flush_checks(
        &mut check_len,
        &elem_ptrs,
        &expected,
        &is_own,
        &mut container_bytes,
        &mut container_ptrs,
        &mut cap_bytes,
    ) {
        return None;
    }
    Some(stores)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreadable;

pub fn state(
    memory: &impl engine_native_memory::Memory,
    receiver: Receiver,
    first: usize,
    third: usize,
) -> Result<Option<u32>, Unreadable> {
    let id = |state: usize| {
        state
            .checked_add(engine_native::cstate::ID)
            .and_then(|address| engine_native_memory::read::<u32>(memory, address))
            .filter(|&id| id != 0)
            .map(Some)
            .ok_or(Unreadable)
    };
    match receiver {
        Receiver::State => id(first),
        Receiver::StatePool => id(first
            .checked_sub(engine_native::cstate::RESOURCE_POOL)
            .ok_or(Unreadable)?),
        Receiver::OriginState if third == 0 => Ok(None),
        Receiver::OriginState => id(third),
        Receiver::Origin => {
            let state = first
                .checked_add(crate::cresource_origin::STATE)
                .and_then(|address| engine_native_memory::read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or(Unreadable)?;
            if state == 0 { Ok(None) } else { id(state) }
        }
        Receiver::Province => {
            let state = first
                .checked_add(engine_native::cprovince::STATE)
                .and_then(|address| engine_native_memory::read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or(Unreadable)?;
            if state == 0 { Ok(None) } else { id(state) }
        }
        _ => Err(Unreadable),
    }
}

pub fn country(memory: &impl engine_native_memory::Memory, resources: usize) -> Option<u32> {
    resources
        .checked_add(crate::ccountry_resources::OWNER)
        .and_then(|address| engine_native_memory::read::<u32>(memory, address))
        .filter(|&raw| raw != 0)
}

#[cfg(test)]
pub const FIRST_SITE: usize = 12;
#[cfg(test)]
pub const FIRST_CLAIM_SITE: usize = 25;
#[cfg(test)]
pub const FIRST_STRATEGIC_LOCATION_SITE: usize = 27;

pub fn site_country(
    memory: &impl engine_native_memory::Memory,
    receiver: Receiver,
    first: usize,
    second: usize,
    third: usize,
) -> Result<Option<u32>, Unreadable> {
    let link = match receiver {
        Receiver::StrategicAir => first.checked_add(engine_native_air::cstrategic_air::COUNTRY),
        Receiver::CountryTag => Some(second),
        Receiver::PoolCapacity => Some(third),
        _ => None,
    };
    link.and_then(|address| engine_native_memory::read::<u32>(memory, address))
        .map(|raw| (raw != 0).then_some(raw))
        .ok_or(Unreadable)
}

pub fn pool_capacity(
    memory: &impl engine_native_memory::Memory,
    module: usize,
    base: usize,
    tag: usize,
) -> Option<i64> {
    let containers_addr = base.checked_add(engine_native_air::cair_base::COUNTRY_CONTAINERS)?;
    let singleton_addr = engine_sites::at(module, engine_native::cgame_state::SINGLETON_RVA)?;
    let mut raw_bytes = [0_u8; 4];
    let mut containers_bytes = [0_u8; 16];
    let mut game_bytes = [0_u8; 8];
    let copied = memory.copy_scattered(
        [tag, containers_addr, singleton_addr],
        &mut [
            &mut raw_bytes[..],
            &mut containers_bytes[..],
            &mut game_bytes[..],
        ],
    );
    if copied < 2 {
        return None;
    }
    let raw = u32::from_le_bytes(raw_bytes);
    let containers = engine_native_memory::cpdx_array::Array::from_header(containers_bytes, 8)?;
    let slot = if raw == 0 {
        0
    } else {
        if copied < 3 {
            return None;
        }
        let game = u64::from_le_bytes(game_bytes);
        let links = engine_native_memory::cpdx_array::Array::read(
            memory,
            usize::try_from(game)
                .ok()?
                .checked_add(engine_native::cgame_state::COUNTRY_LINKS)?,
            4,
        )?;
        engine_native_memory::read::<u32>(memory, links.element(raw, 4)?)?
    };
    let container = engine_native_memory::read::<u64>(memory, containers.element(slot, 8)?)?;
    if container == 0 {
        return None;
    }
    engine_native_memory::read::<i64>(
        memory,
        usize::try_from(container)
            .ok()?
            .checked_add(engine_native_air::ccountry_air_container::CAPACITY_RAW)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    struct Daily(Vec<u8>);
    impl Daily {
        const AT: usize = 0x1000_0000;
        const GAME: usize = Self::AT + 0x100;
        const LINKS: usize = Self::AT + 0x800;
        const AIR: usize = Self::AT + 0x1000;
        const BASES: usize = Self::AT + 0x1100;
        const PAIRS: usize = Self::AT + 0x5000;
        const VECTORS: usize = Self::AT + 0x5100;
        fn base(index: usize) -> usize {
            Self::AT + 0x2000 + index * 0x1000
        }
        fn container(base: usize, slot: usize) -> usize {
            Self::AT + 0x8000 + base * 0x400 + slot * 0x100
        }
        fn entries(base: usize) -> usize {
            Self::AT + 0x6000 + base * 0x100
        }
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.0[address - Self::AT..][..bytes.len()].copy_from_slice(bytes);
        }
        fn word(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.word(address, data as u64);
            self.put(address + 8, &count.to_le_bytes());
            self.put(address + 12, &count.to_le_bytes());
        }
        fn capacity(&mut self, base: usize, slot: usize, value: i64) {
            self.put(
                Self::container(base, slot) + engine_native_air::ccountry_air_container::CAPACITY_RAW,
                &value.to_le_bytes(),
            );
        }
        fn new() -> Self {
            let mut f = Self(vec![0; 0xa000]);
            f.word(Self::AT, Self::GAME as u64);
            f.array(
                Self::GAME + engine_native::cgame_state::COUNTRY_LINKS,
                Self::LINKS,
                4,
            );
            for raw in 1..4_u32 {
                f.put(Self::LINKS + raw as usize * 4, &raw.to_le_bytes());
            }
            f.array(Self::AIR + 0x10, Self::BASES, 2);
            f.put(
                Self::AIR + engine_native_air::cstrategic_air::COUNTRY,
                &1_u32.to_le_bytes(),
            );
            f.word(Self::PAIRS, Self::VECTORS as u64);
            for index in 0..2 {
                let base = Self::base(index);
                f.word(Self::BASES + index * 8, base as u64);
                let containers = base + 0x100;
                f.array(base + engine_native_air::cair_base::COUNTRY_CONTAINERS, containers, 4);
                for slot in 1..4 {
                    f.word(containers + slot * 8, Self::container(index, slot) as u64);
                    f.capacity(index, slot, 100_000);
                }
                f.array(Self::VECTORS + index * 0x18, Self::entries(index), 2);
                for (entry, raw) in [2_u32, 3].into_iter().enumerate() {
                    let at = Self::entries(index) + entry * 16;
                    f.put(at, &raw.to_le_bytes());
                    f.put(at + 8, &100_000_i64.to_le_bytes());
                }
            }
            f.word(Self::base(1) + engine_native_air::cair_base::STATE, 0x77);
            f
        }
        fn unchanged(&self) -> Option<u64> {
            let memory = SliceMemory {
                base: Self::AT,
                bytes: &self.0,
            };
            daily_unchanged(
                &memory,
                Self::AT - engine_native::cgame_state::SINGLETON_RVA.0,
                Self::AIR,
                Self::PAIRS,
            )
        }
    }

    #[test]
    fn a_daily_pass_is_certified_only_when_every_store_is_unchanged() {
        assert_eq!(Daily::new().unchanged(), Some(4));
        let mut changed = Daily::new();
        changed.capacity(1, 3, 99_999);
        assert_eq!(changed.unchanged(), None);
        let mut missing = Daily::new();
        missing.word(Daily::base(0) + 0x100 + 2 * 8, 0);
        assert_eq!(missing.unchanged(), None);
        let mut withdrawn = Daily::new();
        withdrawn.capacity(1, 1, 0);
        assert_eq!(withdrawn.unchanged(), None);
        let mut host = Daily::new();
        host.capacity(0, 1, 0);
        assert_eq!(host.unchanged(), Some(4));
        let mut torn = Daily::new();
        torn.word(Daily::VECTORS + 0x18, 0x7000_0000);
        assert_eq!(torn.unchanged(), None);

        let mut empty = Daily::new();
        empty.array(Daily::AIR + 0x10, Daily::BASES, 0);
        assert_eq!(empty.unchanged(), Some(0));

        let mut empty_entries_with_state = Daily::new();
        empty_entries_with_state.array(Daily::VECTORS + 0x18, Daily::entries(1), 0);
        assert_eq!(empty_entries_with_state.unchanged(), Some(2));
        empty_entries_with_state.capacity(1, 1, 0);
        assert_eq!(empty_entries_with_state.unchanged(), None);
    }

    #[test]
    fn route_writers_name_the_country_holding_the_route() {
        let base = 0x1000;
        let mut bytes = vec![0_u8; 0x800];
        let route = 0x100;
        bytes[route + crate::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&9_u32.to_le_bytes());
        bytes[route + crate::resource_map::ROUTE_LINKS[1]..][..4]
            .copy_from_slice(&5_u32.to_le_bytes());
        let link = 0x300;
        bytes[link..][..4].copy_from_slice(&7_u32.to_le_bytes());
        let origin = 0x400;
        bytes[origin + ORIGIN_OWNER..][..4].copy_from_slice(&11_u32.to_le_bytes());
        bytes[origin
            + crate::cresource_origin::DELIVERY_ROUTE
            + crate::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&9_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        assert_eq!(
            route_country(&memory, Receiver::Route, base + route, 0),
            Ok(Some(9))
        );
        assert_eq!(
            route_country(&memory, Receiver::RouteSender, 0, base + link),
            Ok(Some(7))
        );
        assert_eq!(
            route_country(&memory, Receiver::RouteSender, 0, base + 0x380),
            Ok(None)
        );
        assert_eq!(
            route_country(&memory, Receiver::Route, 0x10, 0),
            Err(Unreadable)
        );
        assert_eq!(
            route_country(&memory, Receiver::EveryRoute, base + route, base + link),
            Err(Unreadable)
        );
        assert_eq!(
            route_country(&memory, Receiver::OriginRoute, base + origin, 0),
            Ok(Some(11))
        );
        assert_eq!(origin_route_kind(&memory, base + origin), Some(0));
        assert_eq!(origin_route_kind(&memory, usize::MAX), None);
        for (site, receiver) in &MUTATIONS[FIRST_ROUTE_SITE..] {
            assert!(
                matches!(
                    receiver,
                    Receiver::Route
                        | Receiver::RouteSender
                        | Receiver::OriginRoute
                        | Receiver::EveryRoute
                ),
                "{}",
                site.name
            );
        }
    }

    #[test]
    fn receivers_name_their_state_or_country() {
        let base = 0x1000;
        let mut bytes = vec![0_u8; 0x2000];
        let state = 0x400;
        bytes[state + engine_native::cstate::ID..][..4].copy_from_slice(&42_u32.to_le_bytes());
        let origin = 0x100;
        bytes[origin + crate::cresource_origin::STATE..][..8]
            .copy_from_slice(&((base + state) as u64).to_le_bytes());
        let resources = 0x1800;
        bytes[resources + crate::ccountry_resources::OWNER..][..4]
            .copy_from_slice(&7_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        let at = |offset: usize| base + offset;
        assert_eq!(
            super::state(&memory, Receiver::State, at(state), 0),
            Ok(Some(42))
        );
        assert_eq!(
            super::state(
                &memory,
                Receiver::StatePool,
                at(state) + engine_native::cstate::RESOURCE_POOL,
                0
            ),
            Ok(Some(42))
        );
        assert_eq!(
            super::state(&memory, Receiver::OriginState, 0, at(state)),
            Ok(Some(42))
        );
        assert_eq!(super::state(&memory, Receiver::OriginState, 0, 0), Ok(None));
        assert_eq!(
            super::state(&memory, Receiver::Origin, at(origin), 0),
            Ok(Some(42))
        );
        assert_eq!(
            super::state(&memory, Receiver::Origin, at(0x200), 0),
            Ok(None)
        );
        assert_eq!(
            super::state(&memory, Receiver::Origin, 0x10, 0),
            Err(Unreadable)
        );
        assert_eq!(
            super::state(&memory, Receiver::State, at(0x600), 0),
            Err(Unreadable)
        );
        assert_eq!(super::country(&memory, at(resources)), Some(7));
        assert_eq!(super::country(&memory, at(0x600)), None);
    }

    #[test]
    fn site_list_receivers_name_their_country() {
        let base = 0x1000;
        let mut bytes = vec![0_u8; 0x400];
        bytes[0x100 + engine_native_air::cstrategic_air::COUNTRY..][..4].copy_from_slice(&9_u32.to_le_bytes());
        bytes[0x300..0x304].copy_from_slice(&5_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        let at = |offset: usize| base + offset;
        let name = |receiver, first, second, third| {
            super::site_country(&memory, receiver, first, second, third)
        };
        assert_eq!(name(Receiver::StrategicAir, at(0x100), 0, 0), Ok(Some(9)));
        assert_eq!(name(Receiver::CountryTag, 0, at(0x300), 0), Ok(Some(5)));
        assert_eq!(name(Receiver::PoolCapacity, 0, 0, at(0x300)), Ok(Some(5)));
        assert_eq!(name(Receiver::StrategicAir, at(0x200), 0, 0), Ok(None));
        assert_eq!(name(Receiver::CountryTag, 0, 0x10, 0), Err(Unreadable));
        assert!(MUTATIONS[..FIRST_SITE].iter().all(|(_, receiver)| matches!(
            receiver,
            Receiver::State
                | Receiver::StatePool
                | Receiver::OriginState
                | Receiver::Origin
                | Receiver::CountryResources
        )));
        assert!(
            MUTATIONS[FIRST_SITE..FIRST_CLAIM_SITE]
                .iter()
                .all(|(_, receiver)| matches!(
                    receiver,
                    Receiver::StrategicAir
                        | Receiver::CountryTag
                        | Receiver::EveryCountry
                        | Receiver::PoolCapacity
                ))
        );
        assert!(
            MUTATIONS[FIRST_CLAIM_SITE..FIRST_STRATEGIC_LOCATION_SITE]
                .iter()
                .all(|(_, receiver)| *receiver == Receiver::State)
        );
        assert_eq!(
            MUTATIONS[FIRST_STRATEGIC_LOCATION_SITE..FIRST_ROUTE_SITE]
                .iter()
                .map(|(_, receiver)| *receiver)
                .collect::<Vec<_>>(),
            [Receiver::State, Receiver::Province]
        );
    }

    #[test]
    fn the_pool_capacity_is_the_tag_slots_container_word() {
        let module = 0x1000;
        let base = module + engine_native::cgame_state::SINGLETON_RVA.0;
        let mut bytes = vec![0_u8; 0x2000];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        let array = |data: usize, count: u32| {
            let mut header = [0_u8; 16];
            header[..8].copy_from_slice(&((base + data) as u64).to_le_bytes());
            header[8..12].copy_from_slice(&count.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
            header
        };
        put(0, &((base + 0x100) as u64).to_le_bytes());
        put(0x100 + engine_native::cgame_state::COUNTRY_LINKS, &array(0x800, 8));
        put(0x800 + 7 * 4, &2_u32.to_le_bytes());
        put(
            0x1000 + engine_native_air::cair_base::COUNTRY_CONTAINERS,
            &array(0x1100, 4),
        );
        put(0x1100 + 2 * 8, &((base + 0x1200) as u64).to_le_bytes());
        put(
            0x1200 + engine_native_air::ccountry_air_container::CAPACITY_RAW,
            &50_000_i64.to_le_bytes(),
        );
        put(0x1800, &7_u32.to_le_bytes());
        put(0x1804, &3_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        let capacity = |tag| super::pool_capacity(&memory, module, base + 0x1000, tag);
        assert_eq!(capacity(base + 0x1800), Some(50_000));
        assert_eq!(capacity(base + 0x1804), None);
        assert_eq!(capacity(0x10), None);
    }
}
