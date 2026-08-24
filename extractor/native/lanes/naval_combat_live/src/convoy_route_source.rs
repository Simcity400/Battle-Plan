use crate::convoy_terminal_attribution::{
    EntityHandle, ExactMemory, LiveRouteResolution, RouteEndpoint, RouteEndpointKind,
    RouteProducerKind, GAME_ENTITY_TYPE_TAG,
};
use sha2::{Digest, Sha256};

pub const A729_REFERENCE_RESOLVER_RVA: usize = 0x0220_A3D0;
pub const A729_REFERENCE_REGISTRY_LARGE_RVA: usize = 0x0343_8E50;
pub const A729_REFERENCE_REGISTRY_DEFAULT_RVA: usize = 0x0343_8E58;
pub const A729_REFERENCE_REGISTRY_SMALL_TABLE_RVA: usize = 0x0343_8E60;
pub const REFERENCE_SMALL_TABLE_COUNT: u32 = 100;
pub const REFERENCE_DEFAULT_LIMIT: u32 = 0x1269;
pub const REFERENCE_HASH_MULTIPLIER: u64 = 0x4963_1A45;
pub const A729_CNAVAL_UNIT_TRANSFER_VTABLE_RVA: usize = 0x0295_6CF8;
pub const A729_CLEND_LEASE_EXCHANGE_VTABLE_RVA: usize = 0x02A0_9DF8;

pub const CNAVAL_UNIT_TRANSFER_TARGET_PROVINCE_OFF: usize = 0x50;
pub const CNAVAL_UNIT_TRANSFER_SOURCE_PROVINCE_OFF: usize = 0x54;
pub const CLEND_LEASE_EXCHANGE_DESTINATION_STATE_PTR_OFF: usize = 0xC8;
pub const CLEND_LEASE_EXCHANGE_ORIGIN_STATE_PTR_OFF: usize = 0xD0;
pub const CSTATE_DATABASE_ID_OFF: usize = 0x58;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExactA729RouteRead {
    pub client: EntityHandle,
    pub resolution: LiveRouteResolution,
    pub reference_resolver: usize,
    pub reference_registry: usize,
    pub reference_slot: usize,
    pub route_object: usize,
    pub route_vtable: usize,
    pub source_endpoint_object: usize,
    pub destination_endpoint_object: usize,
}

impl ExactA729RouteRead {
    pub fn raw_observation_sha256(&self, game_hour_raw: i32) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4-a729-convoy-route-observation-v1\0");
        hasher.update(self.client.type_tag.to_le_bytes());
        hasher.update(self.client.id.to_le_bytes());
        for pointer in [
            self.reference_resolver,
            self.reference_registry,
            self.reference_slot,
            self.route_object,
            self.route_vtable,
            self.source_endpoint_object,
            self.destination_endpoint_object,
        ] {
            hasher.update((pointer as u64).to_le_bytes());
        }
        hasher.update(game_hour_raw.to_le_bytes());
        hasher.update([self.resolution.producer_kind as u8]);
        hasher.update([self.resolution.source.kind as u8]);
        hasher.update(self.resolution.source.id.to_le_bytes());
        hasher.update([self.resolution.destination.kind as u8]);
        hasher.update(self.resolution.destination.id.to_le_bytes());
        hasher.finalize().into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResolvedReference {
    object: usize,
    registry: usize,
    slot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteSourceError {
    NullModuleBase,
    AddressOverflow,
    InvalidClientHandle,
    ReferenceRegistryUnreadable,
    ReferenceRegistryImplausible,
    ReferenceResolutionFailed,
    ReferenceResolutionUnstable,
    ObjectVtableUnreadable,
    ObjectVtableUnstable,
    UnsupportedProducerVtable(u64),
    EndpointPointerUnreadable,
    EndpointPointerUnstable,
    EndpointUnreadable,
    EndpointUnstable,
    ZeroEndpoint,
}

/// The named leaves of "this convoy client has no route binding".
///
/// Before these existed the only evidence a capture carried about a refused
/// client was `h_convoy_resolver_error_hash_sample`, an FNV-1a of a message
/// that embeds the unbound MEMBER COUNT and the client handle's `Debug`. That
/// makes the hash a different value on every frame and unresolvable against
/// any closed candidate set -- captures/rc_vanilla published
/// 8018115730417974978 on the sampled frame and 8265184984225921010 as the
/// window fold, and an offline sweep of the payload-free half of that message
/// shape -- 1,792,000,000 candidates over `{count} x {client id} x {variant}`,
/// bounds recorded in `runs/rx_d90_witness/error_hash_sweep.out` -- matched
/// neither. The domain refused 22
/// of ~25 clients on essentially every frame of that capture and could not say
/// why. One counter per cause fixes that; index 15 is the authentication half
/// of the same call, which is not a `RouteSourceError` but is the same
/// question for a reader.
pub const CONVOY_ROUTE_REFUSAL_UNIT_NAMES: [&str; 17] = [
    "h_convoy_route_refused_null_module_base",
    "h_convoy_route_refused_address_overflow",
    "h_convoy_route_refused_invalid_client_handle",
    "h_convoy_route_refused_reference_registry_unreadable",
    "h_convoy_route_refused_reference_registry_implausible",
    "h_convoy_route_refused_reference_resolution_failed",
    "h_convoy_route_refused_reference_resolution_unstable",
    "h_convoy_route_refused_object_vtable_unreadable",
    "h_convoy_route_refused_object_vtable_unstable",
    // NOT `..._unsupported_producer_vtable`: `_vtable` is a RAW_WORD_SUFFIXES
    // entry, so that spelling would be folded as a last-value raw word instead
    // of summed as the member COUNT it is. The vtable itself travels beside it
    // as `h_convoy_route_refused_producer_vtable_sample`, where the raw-word
    // fold is correct.
    "h_convoy_route_refused_unsupported_producer",
    "h_convoy_route_refused_endpoint_pointer_unreadable",
    "h_convoy_route_refused_endpoint_pointer_unstable",
    "h_convoy_route_refused_endpoint_unreadable",
    "h_convoy_route_refused_endpoint_unstable",
    "h_convoy_route_refused_zero_endpoint",
    "h_convoy_route_refused_authentication",
    // Unreachable while `refusal_index()` stays exhaustive and the
    // authentication index stays in range; it exists so that an index the
    // accumulator cannot place lands in a NAMED slot instead of vanishing and
    // breaking the partition law silently.
    "h_convoy_route_refused_unclassified",
];

/// Index of the catch-all slot. Nothing in the tree writes it on purpose.
pub const CONVOY_ROUTE_REFUSAL_UNCLASSIFIED_INDEX: usize = 16;

/// Index of the authentication leaf, which no `RouteSourceError` produces.
pub const CONVOY_ROUTE_REFUSAL_AUTHENTICATION_INDEX: usize = 15;

impl RouteSourceError {
    /// Index of this variant's counter in `CONVOY_ROUTE_REFUSAL_UNIT_NAMES`.
    /// Exhaustive with no wildcard arm: a new variant fails to compile rather
    /// than being silently folded into an existing cause.
    pub fn refusal_index(&self) -> usize {
        match self {
            Self::NullModuleBase => 0,
            Self::AddressOverflow => 1,
            Self::InvalidClientHandle => 2,
            Self::ReferenceRegistryUnreadable => 3,
            Self::ReferenceRegistryImplausible => 4,
            Self::ReferenceResolutionFailed => 5,
            Self::ReferenceResolutionUnstable => 6,
            Self::ObjectVtableUnreadable => 7,
            Self::ObjectVtableUnstable => 8,
            Self::UnsupportedProducerVtable(_) => 9,
            Self::EndpointPointerUnreadable => 10,
            Self::EndpointPointerUnstable => 11,
            Self::EndpointUnreadable => 12,
            Self::EndpointUnstable => 13,
            Self::ZeroEndpoint => 14,
        }
    }

    /// The unmodelled producer vtable, when that is what refused. It is the
    /// one refusal whose cause is a VALUE rather than a state, and the value
    /// is what an RE follow-up needs.
    pub fn unsupported_producer_vtable(&self) -> Option<u64> {
        match self {
            Self::UnsupportedProducerVtable(vtable) => Some(*vtable),
            _ => None,
        }
    }
}

pub fn read_exact_a729_client_route<M: ExactMemory>(
    memory: &M,
    module_base: usize,
    client: EntityHandle,
) -> Result<ExactA729RouteRead, RouteSourceError> {
    if module_base == 0 {
        return Err(RouteSourceError::NullModuleBase);
    }
    if client.id == 0 || client.type_tag != GAME_ENTITY_TYPE_TAG {
        return Err(RouteSourceError::InvalidClientHandle);
    }
    let reference_resolver = module_base
        .checked_add(A729_REFERENCE_RESOLVER_RVA)
        .ok_or(RouteSourceError::AddressOverflow)?;
    let reference = resolve_reference(memory, module_base, client)?;
    if resolve_reference(memory, module_base, client)? != reference {
        return Err(RouteSourceError::ReferenceResolutionUnstable);
    }
    let route_object = reference.object;
    let route_vtable = memory
        .u64(route_object)
        .ok_or(RouteSourceError::ObjectVtableUnreadable)?;
    if memory.u64(route_object) != Some(route_vtable) {
        return Err(RouteSourceError::ObjectVtableUnstable);
    }

    let naval_vtable = module_base
        .checked_add(A729_CNAVAL_UNIT_TRANSFER_VTABLE_RVA)
        .ok_or(RouteSourceError::AddressOverflow)? as u64;
    let lend_lease_vtable = module_base
        .checked_add(A729_CLEND_LEASE_EXCHANGE_VTABLE_RVA)
        .ok_or(RouteSourceError::AddressOverflow)? as u64;
    let (resolution, source_endpoint_object, destination_endpoint_object) =
        if route_vtable == naval_vtable {
            let source = stable_u32(
                memory,
                route_object,
                CNAVAL_UNIT_TRANSFER_SOURCE_PROVINCE_OFF,
            )?;
            let destination = stable_u32(
                memory,
                route_object,
                CNAVAL_UNIT_TRANSFER_TARGET_PROVINCE_OFF,
            )?;
            let resolution = endpoints(
                client,
                RouteProducerKind::NavalTransport,
                RouteEndpointKind::Province,
                source,
                destination,
            )?;
            (resolution, route_object, route_object)
        } else if route_vtable == lend_lease_vtable {
            let source_state = stable_pointer(
                memory,
                route_object,
                CLEND_LEASE_EXCHANGE_ORIGIN_STATE_PTR_OFF,
            )?;
            let destination_state = stable_pointer(
                memory,
                route_object,
                CLEND_LEASE_EXCHANGE_DESTINATION_STATE_PTR_OFF,
            )?;
            let source = stable_u32(memory, source_state, CSTATE_DATABASE_ID_OFF)?;
            let destination = stable_u32(memory, destination_state, CSTATE_DATABASE_ID_OFF)?;
            let resolution = endpoints(
                client,
                RouteProducerKind::LendLease,
                RouteEndpointKind::State,
                source,
                destination,
            )?;
            (resolution, source_state, destination_state)
        } else {
            return Err(RouteSourceError::UnsupportedProducerVtable(route_vtable));
        };
    if memory.u64(route_object) != Some(route_vtable)
        || resolve_reference(memory, module_base, client)? != reference
    {
        return Err(RouteSourceError::ReferenceResolutionUnstable);
    }
    Ok(ExactA729RouteRead {
        client,
        resolution,
        reference_resolver,
        reference_registry: reference.registry,
        reference_slot: reference.slot,
        route_object,
        route_vtable: route_vtable as usize,
        source_endpoint_object,
        destination_endpoint_object,
    })
}

fn reference_words(client: EntityHandle) -> (u32, u32) {
    (client.type_tag, client.id)
}

pub(crate) fn reference_hash_key(client: EntityHandle) -> u64 {
    let (word0, word1) = reference_words(client);
    (u64::from(word0) << 32) | u64::from(word1)
}

fn resolve_reference<M: ExactMemory>(
    memory: &M,
    module_base: usize,
    client: EntityHandle,
) -> Result<ResolvedReference, RouteSourceError> {
    let (word0, word1) = reference_words(client);
    let registry_slot = if word0 < REFERENCE_SMALL_TABLE_COUNT {
        module_base
            .checked_add(A729_REFERENCE_REGISTRY_SMALL_TABLE_RVA)
            .and_then(|address| address.checked_add(word0 as usize * 8))
    } else if word0 < REFERENCE_DEFAULT_LIMIT {
        module_base.checked_add(A729_REFERENCE_REGISTRY_DEFAULT_RVA)
    } else {
        module_base.checked_add(A729_REFERENCE_REGISTRY_LARGE_RVA)
    }
    .ok_or(RouteSourceError::AddressOverflow)?;
    let registry = stable_raw_pointer(memory, registry_slot)
        .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
    let slots_address = registry
        .checked_add(8)
        .ok_or(RouteSourceError::AddressOverflow)?;
    let slots = stable_raw_pointer(memory, slots_address)
        .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
    let mask = stable_raw_u32(
        memory,
        registry
            .checked_add(0x14)
            .ok_or(RouteSourceError::AddressOverflow)?,
    )
    .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
    let max_probe = stable_raw_u8(
        memory,
        registry
            .checked_add(0x18)
            .ok_or(RouteSourceError::AddressOverflow)?,
    )
    .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
    if mask == 0 || mask > 0x3FFF_FFFF {
        return Err(RouteSourceError::ReferenceRegistryImplausible);
    }
    let key = reference_hash_key(client);
    let product = key.wrapping_mul(REFERENCE_HASH_MULTIPLIER);
    let bucket = ((product as u32) ^ ((product >> 32) as u32)) & mask;
    let mut slot = slots
        .checked_add(bucket as usize * 0x18)
        .ok_or(RouteSourceError::AddressOverflow)?;
    let mut probe = 1u16;
    loop {
        let occupied_distance = stable_raw_u8(
            memory,
            slot.checked_add(4)
                .ok_or(RouteSourceError::AddressOverflow)?,
        )
        .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
        if occupied_distance == 0 || probe > u16::from(occupied_distance) {
            return Err(RouteSourceError::ReferenceResolutionFailed);
        }
        let observed_word0 = stable_raw_u32(
            memory,
            slot.checked_add(8)
                .ok_or(RouteSourceError::AddressOverflow)?,
        )
        .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
        let observed_word1 = stable_raw_u32(
            memory,
            slot.checked_add(0x0C)
                .ok_or(RouteSourceError::AddressOverflow)?,
        )
        .map_err(|_| RouteSourceError::ReferenceRegistryUnreadable)?;
        if observed_word0 == word0 && observed_word1 == word1 {
            let object = stable_raw_pointer(
                memory,
                slot.checked_add(0x10)
                    .ok_or(RouteSourceError::AddressOverflow)?,
            )
            .map_err(|_| RouteSourceError::ReferenceResolutionFailed)?;
            return Ok(ResolvedReference {
                object,
                registry,
                slot,
            });
        }
        if probe > u16::from(max_probe).saturating_add(1) {
            return Err(RouteSourceError::ReferenceResolutionFailed);
        }
        probe = probe.saturating_add(1);
        slot = slot
            .checked_add(0x18)
            .ok_or(RouteSourceError::AddressOverflow)?;
    }
}

fn endpoints(
    route_key: EntityHandle,
    producer_kind: RouteProducerKind,
    kind: RouteEndpointKind,
    source: u32,
    destination: u32,
) -> Result<LiveRouteResolution, RouteSourceError> {
    if source == 0 || destination == 0 {
        return Err(RouteSourceError::ZeroEndpoint);
    }
    Ok(LiveRouteResolution {
        route_key,
        producer_kind,
        source: RouteEndpoint { kind, id: source },
        destination: RouteEndpoint {
            kind,
            id: destination,
        },
    })
}

fn stable_pointer<M: ExactMemory>(
    memory: &M,
    object: usize,
    offset: usize,
) -> Result<usize, RouteSourceError> {
    let address = object
        .checked_add(offset)
        .ok_or(RouteSourceError::AddressOverflow)?;
    let first = memory
        .u64(address)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or(RouteSourceError::EndpointPointerUnreadable)?;
    if memory
        .u64(address)
        .and_then(|value| usize::try_from(value).ok())
        != Some(first)
    {
        return Err(RouteSourceError::EndpointPointerUnstable);
    }
    Ok(first)
}

fn stable_raw_pointer<M: ExactMemory>(
    memory: &M,
    address: usize,
) -> Result<usize, RouteSourceError> {
    let first = memory
        .u64(address)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or(RouteSourceError::EndpointPointerUnreadable)?;
    if memory
        .u64(address)
        .and_then(|value| usize::try_from(value).ok())
        != Some(first)
    {
        return Err(RouteSourceError::EndpointPointerUnstable);
    }
    Ok(first)
}

fn stable_raw_u32<M: ExactMemory>(memory: &M, address: usize) -> Result<u32, RouteSourceError> {
    let first = memory
        .u32(address)
        .ok_or(RouteSourceError::EndpointUnreadable)?;
    if memory.u32(address) != Some(first) {
        return Err(RouteSourceError::EndpointUnstable);
    }
    Ok(first)
}

fn stable_raw_u8<M: ExactMemory>(memory: &M, address: usize) -> Result<u8, RouteSourceError> {
    let mut first = [0u8; 1];
    let mut second = [0u8; 1];
    if !memory.read_exact(address, &mut first) || !memory.read_exact(address, &mut second) {
        return Err(RouteSourceError::EndpointUnreadable);
    }
    if first != second {
        return Err(RouteSourceError::EndpointUnstable);
    }
    Ok(first[0])
}

fn stable_u32<M: ExactMemory>(
    memory: &M,
    object: usize,
    offset: usize,
) -> Result<u32, RouteSourceError> {
    let address = object
        .checked_add(offset)
        .ok_or(RouteSourceError::AddressOverflow)?;
    let first = memory
        .u32(address)
        .ok_or(RouteSourceError::EndpointUnreadable)?;
    if memory.u32(address) != Some(first) {
        return Err(RouteSourceError::EndpointUnstable);
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Memory {
        bytes: BTreeMap<usize, u8>,
        object: usize,
    }

    impl Memory {
        fn put_u8(&mut self, address: usize, value: u8) {
            self.bytes.insert(address, value);
        }

        fn put_u32(&mut self, address: usize, value: u32) {
            for (index, byte) in value.to_le_bytes().iter().enumerate() {
                self.bytes.insert(address + index, *byte);
            }
        }

        fn put_u64(&mut self, address: usize, value: u64) {
            for (index, byte) in value.to_le_bytes().iter().enumerate() {
                self.bytes.insert(address + index, *byte);
            }
        }
    }

    impl ExactMemory for Memory {
        fn read_exact(&self, address: usize, out: &mut [u8]) -> bool {
            for (index, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    fn client() -> EntityHandle {
        EntityHandle {
            id: 106_381,
            type_tag: GAME_ENTITY_TYPE_TAG,
        }
    }

    fn install_reference(memory: &mut Memory, base: usize, client: EntityHandle) {
        let registry = 0x9000usize;
        let slots = 0xA000usize;
        let mask = 7u32;
        let word0 = client.type_tag;
        let word1 = client.id;
        let key = (u64::from(word0) << 32) | u64::from(word1);
        let product = key.wrapping_mul(REFERENCE_HASH_MULTIPLIER);
        let bucket = ((product as u32) ^ ((product >> 32) as u32)) & mask;
        let slot = slots + bucket as usize * 0x18;
        memory.put_u64(base + A729_REFERENCE_REGISTRY_LARGE_RVA, registry as u64);
        memory.put_u64(registry + 8, slots as u64);
        memory.put_u32(registry + 0x14, mask);
        memory.put_u8(registry + 0x18, 1);
        memory.put_u8(slot + 4, 1);
        memory.put_u32(slot + 8, word0);
        memory.put_u32(slot + 0x0C, word1);
        memory.put_u64(slot + 0x10, memory.object as u64);
    }

    fn handle_from_raw(raw: u64) -> EntityHandle {
        EntityHandle {
            type_tag: raw as u32,
            id: (raw >> 32) as u32,
        }
    }

    #[test]
    fn reference_key_is_byte_identical_across_the_handle_rename() {
        let raw: u64 = 0x0001_9F8D_0000_1269;
        let word0 = raw as u32;
        let word1 = (raw >> 32) as u32;
        assert_eq!((word0, word1), (GAME_ENTITY_TYPE_TAG, 106_381));

        let client = handle_from_raw(raw);

        let after = (u64::from(client.type_tag) << 32) | u64::from(client.id);
        let before = (u64::from(raw as u32) << 32) | u64::from((raw >> 32) as u32);

        assert_eq!(before, 0x0000_1269_0001_9F8D, "the exact pre-rename key");
        assert_eq!(after, 0x0000_1269_0001_9F8D, "the exact post-rename key");
        assert_eq!(reference_hash_key(client), 0x0000_1269_0001_9F8D);
        assert_eq!(before, after);
        assert_eq!(reference_hash_key(client), before);

        assert_ne!(reference_hash_key(client), raw);

        let product = reference_hash_key(client).wrapping_mul(REFERENCE_HASH_MULTIPLIER);
        let bucket = ((product as u32) ^ ((product >> 32) as u32)) & 7u32;
        let flipped_product = raw.wrapping_mul(REFERENCE_HASH_MULTIPLIER);
        let flipped_bucket = ((flipped_product as u32) ^ ((flipped_product >> 32) as u32)) & 7u32;
        assert_ne!(
            bucket, flipped_bucket,
            "the reversed key must select a different bucket, or the lookup \
             tests could not detect a word-order regression"
        );
    }

    #[test]
    fn the_custody_digest_hashes_the_handle_as_its_raw_bytes() {
        let raw: u64 = 0x0001_9F8D_0000_1269;
        let client = handle_from_raw(raw);
        let read = ExactA729RouteRead {
            client,
            resolution: LiveRouteResolution {
                route_key: client,
                producer_kind: RouteProducerKind::LendLease,
                source: RouteEndpoint {
                    kind: RouteEndpointKind::State,
                    id: 282,
                },
                destination: RouteEndpoint {
                    kind: RouteEndpointKind::State,
                    id: 64,
                },
            },
            reference_resolver: 0x1111,
            reference_registry: 0x2222,
            reference_slot: 0x3333,
            route_object: 0x4444,
            route_vtable: 0x5555,
            source_endpoint_object: 0x6666,
            destination_endpoint_object: 0x7777,
        };

        let mut expected = Sha256::new();
        expected.update(b"hoi4-a729-convoy-route-observation-v1\0");
        expected.update(raw.to_le_bytes());
        for pointer in [
            read.reference_resolver,
            read.reference_registry,
            read.reference_slot,
            read.route_object,
            read.route_vtable,
            read.source_endpoint_object,
            read.destination_endpoint_object,
        ] {
            expected.update((pointer as u64).to_le_bytes());
        }
        expected.update(61_000_000i32.to_le_bytes());
        expected.update([read.resolution.producer_kind as u8]);
        expected.update([read.resolution.source.kind as u8]);
        expected.update(read.resolution.source.id.to_le_bytes());
        expected.update([read.resolution.destination.kind as u8]);
        expected.update(read.resolution.destination.id.to_le_bytes());

        assert_eq!(
            read.raw_observation_sha256(61_000_000),
            <[u8; 32]>::from(expected.finalize()),
            "the handle must enter the digest as the raw eight bytes it was \
             read as, in engine word order"
        );
    }

    #[test]
    fn registry_dispatch_reads_word0() {
        let base = 0x1400_0000_0usize;
        let object = 0x5000usize;
        let mut memory = Memory {
            bytes: BTreeMap::new(),
            object,
        };
        let client = handle_from_raw((106_381u64 << 32) | 7);
        assert_eq!(client.type_tag, 7);
        assert_eq!(client.id, 106_381);

        let registry = 0x9000usize;
        let slots = 0xA000usize;
        let mask = 7u32;
        let key = (7u64 << 32) | 106_381;
        let product = key.wrapping_mul(REFERENCE_HASH_MULTIPLIER);
        let bucket = ((product as u32) ^ ((product >> 32) as u32)) & mask;
        let slot = slots + bucket as usize * 0x18;
        memory.put_u64(
            base + A729_REFERENCE_REGISTRY_SMALL_TABLE_RVA + 7 * 8,
            registry as u64,
        );
        memory.put_u64(registry + 8, slots as u64);
        memory.put_u32(registry + 0x14, mask);
        memory.put_u8(registry + 0x18, 1);
        memory.put_u8(slot + 4, 1);
        memory.put_u32(slot + 8, 7);
        memory.put_u32(slot + 0x0C, 106_381);
        memory.put_u64(slot + 0x10, object as u64);

        let resolved = resolve_reference(&memory, base, client)
            .expect("word0 = 7 must dispatch to the small table at index 7");
        assert_eq!(resolved.object, object);
        assert_eq!(resolved.registry, registry);
    }

    #[test]
    fn the_t25_convoy_client_shape_resolves_instead_of_being_refused() {
        let base = 0x1400_0000_0usize;
        let object = 0x5000usize;
        let mut memory = Memory {
            bytes: BTreeMap::new(),
            object,
        };
        let client = handle_from_raw(0x0001_9F8D_0000_1269);
        install_reference(&mut memory, base, client);
        memory.put_u64(object, (base + A729_CNAVAL_UNIT_TRANSFER_VTABLE_RVA) as u64);
        memory.put_u32(object + CNAVAL_UNIT_TRANSFER_SOURCE_PROVINCE_OFF, 12_140);
        memory.put_u32(object + CNAVAL_UNIT_TRANSFER_TARGET_PROVINCE_OFF, 9_950);

        let read = read_exact_a729_client_route(&memory, base, client)
            .expect("the t25 convoy client handle must be admitted and resolved, not refused");
        assert_eq!(read.client, client);
        assert_eq!(read.resolution.source.id, 12_140);
        assert_eq!(read.resolution.destination.id, 9_950);

        let flipped = EntityHandle {
            id: 0x0001_9F8D_0000_1269u64 as u32,
            type_tag: (0x0001_9F8D_0000_1269u64 >> 32) as u32,
        };
        assert_eq!(
            read_exact_a729_client_route(&memory, base, flipped),
            Err(RouteSourceError::InvalidClientHandle),
        );
    }

    #[test]
    fn reads_lend_lease_serializer_endpoints() {
        let base = 0x1400_0000_0usize;
        let object = 0x5000usize;
        let origin = 0x7000usize;
        let destination = 0x8000usize;
        let mut memory = Memory {
            bytes: BTreeMap::new(),
            object,
        };
        install_reference(&mut memory, base, client());
        memory.put_u64(object, (base + A729_CLEND_LEASE_EXCHANGE_VTABLE_RVA) as u64);
        memory.put_u64(
            object + CLEND_LEASE_EXCHANGE_ORIGIN_STATE_PTR_OFF,
            origin as u64,
        );
        memory.put_u64(
            object + CLEND_LEASE_EXCHANGE_DESTINATION_STATE_PTR_OFF,
            destination as u64,
        );
        memory.put_u32(origin + CSTATE_DATABASE_ID_OFF, 282);
        memory.put_u32(destination + CSTATE_DATABASE_ID_OFF, 64);
        let read = read_exact_a729_client_route(&memory, base, client()).unwrap();
        assert_eq!(read.resolution.producer_kind, RouteProducerKind::LendLease);
        assert_eq!(read.resolution.source.id, 282);
        assert_eq!(read.resolution.destination.id, 64);
    }

    #[test]
    fn reads_naval_transfer_serializer_endpoints() {
        let base = 0x1400_0000_0usize;
        let object = 0x5000usize;
        let mut memory = Memory {
            bytes: BTreeMap::new(),
            object,
        };
        install_reference(&mut memory, base, client());
        memory.put_u64(object, (base + A729_CNAVAL_UNIT_TRANSFER_VTABLE_RVA) as u64);
        memory.put_u32(object + CNAVAL_UNIT_TRANSFER_SOURCE_PROVINCE_OFF, 12_140);
        memory.put_u32(object + CNAVAL_UNIT_TRANSFER_TARGET_PROVINCE_OFF, 9_950);
        let read = read_exact_a729_client_route(&memory, base, client()).unwrap();
        assert_eq!(
            read.resolution.producer_kind,
            RouteProducerKind::NavalTransport
        );
        assert_eq!(read.resolution.source.id, 12_140);
        assert_eq!(read.resolution.destination.id, 9_950);
    }

    #[test]
    fn rejects_unknown_concrete_producer() {
        let base = 0x1400_0000_0usize;
        let object = 0x5000usize;
        let mut memory = Memory {
            bytes: BTreeMap::new(),
            object,
        };
        install_reference(&mut memory, base, client());
        memory.put_u64(object, base as u64 + 0x1234);
        assert_eq!(
            read_exact_a729_client_route(&memory, base, client()),
            Err(RouteSourceError::UnsupportedProducerVtable(
                base as u64 + 0x1234
            ))
        );
    }
}
