//! Reference registry used by deployment handles and order-group roots.
//! There is no generation word. Growth publishes mask before slots pointer,
//! requiring a quiescent boundary.

use engine_sites::Rva;
/// Low-type reference-table array RVA.
pub const LOW_ROOTS_RVA: Rva = Rva(0x034f_fc00);
/// Engine mid-type reference-table root RVA.
pub const MID_ROOT_RVA: Rva = Rva(0x034f_fbf8);
/// Engine high-type reference-table root RVA.
pub const HIGH_ROOT_RVA: Rva = Rva(0x034f_fbf0);
/// Engine low-type root count.
pub const LOW_ROOT_COUNT: u32 = 100;
/// Engine mid-type cutoff.
pub const MID_TYPE_LIMIT: u32 = 0x1269;
/// Engine static-empty bucket RVA.
pub const STATIC_EMPTY_RVA: Rva = Rva(0x0313_5f98);
/// Reference registry entries pointer.
pub const ENTRIES: usize = 0x08;
/// Reference registry u32 count.
pub const COUNT: usize = 0x10;
/// Reference registry u32 bucket mask.
pub const MASK: usize = 0x14;
/// Reference registry overflow-distance u8.
#[cfg(any(test, feature = "test-support"))]
pub const OVERFLOW_DISTANCE: usize = 0x18;
/// Reference registry factory allocation size.
pub const SIZE: usize = 0x20;
/// Reference registry bucket hash multiplier, wrapping u64.
pub const HASH_MULTIPLIER: u64 = 0x4963_1a45;
/// Reference registry minimum growth exponent.
pub const MINIMUM_EXPONENT: u32 = 3;
/// Reference registry maximum growth exponent.
pub const MAXIMUM_EXPONENT: u32 = 30;

/// Passive bounded reference lookup. Null root or absent key supplies
/// `None`; unreadable or invalid headers remain errors.
pub fn resolve(
    memory: &impl crate::Memory,
    base: usize,
    handle: u64,
) -> Result<Option<usize>, &'static str> {
    if handle == 0 {
        return Ok(None);
    }
    let root = root_address(base, handle)?;
    let Some(header) = read_header(memory, base, root)? else {
        return Ok(None);
    };
    probe(memory, header, handle)
}

/// Live entry count in the selected registry with resolution's header checks.
/// Low roots are type-specific; middle/high roots share several types.
/// Insertion increments the count and erasure decrements it, without a bucket walk.
pub fn entry_count(
    memory: &impl crate::Memory,
    base: usize,
    kind: u32,
) -> Result<usize, &'static str> {
    let root = root_address(base, u64::from(kind))?;
    Ok(read_header(memory, base, root)?.map_or(0, |header| header.count as usize))
}

#[derive(Clone, Copy, Debug)]
pub struct Header {
    entries: usize,
    count: u32,
    mask: u32,
    overflow: u32,
    terminal: u32,
}

/// One fixed-base quiescent pass. Unlike the occupant's small successful
/// header cache, this retains null and failed roots too, without eviction.
pub struct ScopedResolver {
    base: usize,
    headers: [Option<Result<Option<Header>, &'static str>>; LOW_ROOT_COUNT as usize + 2],
}

impl ScopedResolver {
    pub fn new(base: usize) -> Self {
        Self {
            base,
            headers: [None; LOW_ROOT_COUNT as usize + 2],
        }
    }

    pub fn resolve(
        &mut self,
        memory: &impl crate::Memory,
        handle: u64,
    ) -> Result<Option<usize>, &'static str> {
        if handle == 0 {
            return Ok(None);
        }
        let kind = handle as u32;
        let slot = if kind < LOW_ROOT_COUNT {
            kind as usize
        } else if kind < MID_TYPE_LIMIT {
            LOW_ROOT_COUNT as usize
        } else {
            LOW_ROOT_COUNT as usize + 1
        };
        let observed = *self.headers[slot].get_or_insert_with(|| {
            root_address(self.base, handle).and_then(|root| read_header(memory, self.base, root))
        });
        match observed? {
            Some(header) => probe(memory, header, handle),
            None => Ok(None),
        }
    }

    /// The first bucket a [`Self::resolve`] of `handle` reads, and the
    /// header it reads it under: `None` when the resolve reads no bucket (a
    /// zero handle, a null root or a probe that ends at once), so it misses.
    /// The header is read as `resolve` reads it, once a pass.
    pub fn first_cell(
        &mut self,
        memory: &impl crate::Memory,
        handle: u64,
    ) -> Result<Option<(Header, usize)>, &'static str> {
        if handle == 0 {
            return Ok(None);
        }
        let kind = handle as u32;
        let slot = if kind < LOW_ROOT_COUNT {
            kind as usize
        } else if kind < MID_TYPE_LIMIT {
            LOW_ROOT_COUNT as usize
        } else {
            LOW_ROOT_COUNT as usize + 1
        };
        let observed = *self.headers[slot].get_or_insert_with(|| {
            root_address(self.base, handle).and_then(|root| read_header(memory, self.base, root))
        });
        let Some(header) = observed? else {
            return Ok(None);
        };
        Ok(first_cell(header, handle)?.map(|cell| (header, cell)))
    }

    /// [`Self::resolve`] of `handle` from its [`Self::first_cell`]'s bytes,
    /// copied with other handles' first buckets; later buckets are read.
    pub fn resolve_gathered(
        &self,
        memory: &impl crate::Memory,
        header: Header,
        handle: u64,
        first: [u8; 24],
    ) -> Result<Option<usize>, &'static str> {
        probe_with(memory, header, handle, Some(first))
    }
}

/// Province occupant lookups share successful structural observations only
/// within one quiescent walk. Unit identities and buckets are never retained.
#[derive(Default)]
pub struct HeaderResolver {
    headers: [Option<(usize, Header)>; 4],
    next: usize,
}

impl HeaderResolver {
    /// Whether a [`Self::resolve`] of `handle` would take a header this
    /// resolver already read, without reading anything.
    pub fn holds(&self, base: usize, handle: u64) -> bool {
        handle != 0
            && root_address(base, handle)
                .is_ok_and(|root| self.headers.iter().flatten().any(|(key, _)| *key == root))
    }

    pub fn resolve(
        &mut self,
        memory: &impl crate::Memory,
        base: usize,
        handle: u64,
    ) -> Result<Option<usize>, &'static str> {
        if handle == 0 {
            return Ok(None);
        }
        let root = root_address(base, handle)?;
        let header = if let Some((_, header)) =
            self.headers.iter().flatten().find(|(key, _)| *key == root)
        {
            *header
        } else {
            let Some(header) = read_header(memory, base, root)? else {
                return Ok(None);
            };
            self.headers[self.next] = Some((root, header));
            self.next = (self.next + 1) % self.headers.len();
            header
        };
        probe(memory, header, handle)
    }
}

fn root_address(base: usize, handle: u64) -> Result<usize, &'static str> {
    let kind = handle as u32;
    let root = if kind < LOW_ROOT_COUNT {
        engine_sites::at(base, LOW_ROOTS_RVA).and_then(|roots| roots.checked_add(kind as usize * 8))
    } else if kind < MID_TYPE_LIMIT {
        engine_sites::at(base, MID_ROOT_RVA)
    } else {
        engine_sites::at(base, HIGH_ROOT_RVA)
    };
    root.ok_or("reference root overflow")
}

fn read_header(
    memory: &impl crate::Memory,
    base: usize,
    root: usize,
) -> Result<Option<Header>, &'static str> {
    use crate::{read, reference_bucket as bucket};
    let table = read::<u64>(memory, root).ok_or("reference root unreadable")? as usize;
    if table == 0 {
        return Ok(None);
    }
    let at = table
        .checked_add(ENTRIES)
        .ok_or("reference header overflow")?;
    let header = read::<[u8; 17]>(memory, at).ok_or("reference header unreadable")?;
    let entries = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let count = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let overflow = u32::from(header[16]);
    let capacity = mask.checked_add(1).ok_or("reference mask overflow")?;
    if entries == 0
        || !capacity.is_power_of_two()
        || capacity > 1 << MAXIMUM_EXPONENT
        || overflow == 255
    {
        return Err("reference header invalid");
    }
    let static_empty =
        engine_sites::at(base, STATIC_EMPTY_RVA).ok_or("reference static empty overflow")?;
    let exponent = capacity.trailing_zeros();
    if count > capacity
        || if capacity == 1 {
            entries != static_empty || count != 0 || overflow != 0
        } else {
            entries == static_empty
                || !(MINIMUM_EXPONENT..=MAXIMUM_EXPONENT).contains(&exponent)
                || overflow != exponent + 2
        }
    {
        return Err("reference header invalid");
    }
    let terminal = capacity
        .checked_add(overflow)
        .ok_or("reference terminal overflow")?;
    let cell = |index: u32| {
        entries
            .checked_add(index as usize * bucket::STRIDE)
            .ok_or("reference bucket overflow")
    };
    if read::<u8>(
        memory,
        cell(terminal)?
            .checked_add(bucket::DISTANCE)
            .ok_or("reference sentinel overflow")?,
    ) != Some(255)
        || count > terminal
    {
        return Err("reference sentinel or count invalid");
    }
    Ok(Some(Header {
        entries,
        count,
        mask,
        overflow,
        terminal,
    }))
}

fn probe(
    memory: &impl crate::Memory,
    header: Header,
    handle: u64,
) -> Result<Option<usize>, &'static str> {
    probe_with(memory, header, handle, None)
}

/// The first probe index of `handle` under `header`.
fn first_index(header: Header, handle: u64) -> u32 {
    // Engine hashes type-high/id-low, the reverse of the stored handle words.
    let hash = handle.rotate_left(32).wrapping_mul(HASH_MULTIPLIER);
    ((hash >> 32) as u32 ^ hash as u32) & header.mask
}

/// The bucket [`probe`] reads first, with its checks in their order; `None`
/// when the probe ends before reading one.
fn first_cell(header: Header, handle: u64) -> Result<Option<usize>, &'static str> {
    let index = first_index(header, handle);
    if index == header.terminal {
        return Ok(None);
    }
    if index > header.terminal {
        return Err("reference probe past sentinel");
    }
    header
        .entries
        .checked_add(index as usize * crate::reference_bucket::STRIDE)
        .map(Some)
        .ok_or("reference bucket overflow")
}

/// [`probe`], taking the first bucket's bytes when already copied.
fn probe_with(
    memory: &impl crate::Memory,
    header: Header,
    handle: u64,
    gathered: Option<[u8; 24]>,
) -> Result<Option<usize>, &'static str> {
    use crate::{read, reference_bucket as bucket};
    let Header {
        entries,
        overflow,
        terminal,
        ..
    } = header;
    let cell = |index: u32| {
        entries
            .checked_add(index as usize * bucket::STRIDE)
            .ok_or("reference bucket overflow")
    };
    let first = first_index(header, handle);
    let mut found = None;
    for distance in 1..=255_u32 {
        let index = first + distance - 1;
        if index == terminal {
            break;
        }
        if index > terminal {
            return Err("reference probe past sentinel");
        }
        let bytes = match gathered {
            Some(bytes) if distance == 1 => bytes,
            _ => read::<[u8; 24]>(memory, cell(index)?).ok_or("reference bucket unreadable")?,
        };
        let stored_distance = u32::from(bytes[bucket::DISTANCE]);
        if stored_distance == 255 {
            return Err("reference early sentinel");
        }
        if stored_distance > overflow + 1 {
            return Err("reference bucket distance invalid");
        }
        if stored_distance < distance {
            break;
        }
        if u64::from_le_bytes(bytes[8..16].try_into().unwrap()) == handle {
            let pointer = u64::from_le_bytes(bytes[16..].try_into().unwrap()) as usize;
            found = (pointer != 0).then_some(pointer);
            break;
        }
    }
    Ok(found)
}
/// Dynamic reference-type base added to the type counter.
#[cfg(any(test, feature = "test-support"))]
pub const DYNAMIC_TYPE_BASE: u32 = 0x1268;
/// Minted-ID counter, u32, incremented on minting and serialized unsigned
/// as `debug_current_ref_id`.
pub const ID_COUNTER_RVA: Rva = Rva(0x034f_ff20);
/// Default/invalid packed handle: low u32 type and high u32 ID. New reference
/// objects and fields initialize from it; absent front bindings compare both
/// words against it. It is not a counter and never advances.
pub const INVALID_REFERENCE_RVA: Rva = Rva(0x033e_b2a0);

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::reference_bucket as bucket;

    use super::test_support::*;

    #[test]
    fn entry_count_uses_the_native_root_and_validated_header_without_probing() {
        for kind in [0, 53, LOW_ROOT_COUNT, MID_TYPE_LIMIT, u32::MAX] {
            let mut memory = Fixture::new(kind);
            memory.put(TABLE + COUNT, &3_u32.to_le_bytes());
            memory.bytes.remove(&ENTRIES_ADDRESS);
            let census = crate::read_census::ReadCensus::new(&memory);
            assert_eq!(entry_count(&census, 0, kind), Ok(3));
            assert_eq!((census.calls(), census.bytes()), (3, 8 + 17 + 1));
            memory.put(Fixture::root(kind), &0_u64.to_le_bytes());
            assert_eq!(entry_count(&memory, 0, kind), Ok(0));
        }
        for address in [
            Fixture::root(53),
            TABLE + COUNT,
            ENTRIES_ADDRESS + 13 * bucket::STRIDE + bucket::DISTANCE,
        ] {
            let mut memory = Fixture::new(53);
            memory.bytes.remove(&address);
            assert!(entry_count(&memory, 0, 53).is_err());
        }
        let mut memory = Fixture::new(53);
        memory.put(TABLE + COUNT, &9_u32.to_le_bytes());
        assert!(entry_count(&memory, 0, 53).is_err());
        assert!(entry_count(&memory, usize::MAX, 53).is_err());
    }

    #[test]
    fn resolver_routes_full_handles_and_finds_displaced_entries() {
        for kind in [0, 53, 99, 100, MID_TYPE_LIMIT - 1, MID_TYPE_LIMIT, u32::MAX] {
            let handle = (u64::from(u32::MAX) << 32) | u64::from(kind);
            let mut memory = Fixture::new(kind);
            let index = Fixture::index(handle);
            memory.entry(index, 1, handle ^ (1 << 32), 0x9999);
            memory.entry(index + 1, 2, handle, 0x8888);
            memory.put(TABLE + COUNT, &2_u32.to_le_bytes());
            let census = crate::read_census::ReadCensus::new(&memory);
            assert_eq!(resolve(&census, 0, handle), Ok(Some(0x8888)));
            assert_eq!(census.calls(), 5);
            assert_eq!(census.bytes(), 8 + 17 + 1 + 2 * bucket::STRIDE);
            let mut retained = HeaderResolver::default();
            census.reset();
            assert_eq!(retained.resolve(&census, 0, handle), Ok(Some(0x8888)));
            assert_eq!((census.calls(), census.bytes()), (5, 74));
            census.reset();
            assert_eq!(retained.resolve(&census, 0, handle), Ok(Some(0x8888)));
            assert_eq!((census.calls(), census.bytes()), (2, 48));
            memory.entry(index + 1, 2, handle, 0);
            assert_eq!(resolve(&memory, 0, handle), Ok(None));
            assert_eq!(retained.resolve(&memory, 0, handle), Ok(None));
        }
    }

    #[test]
    fn retained_headers_distinguish_roots_share_routed_types_and_evict_without_loss() {
        let mut memory = Fixture::new(53);
        for kind in [54, 100, MID_TYPE_LIMIT, 55] {
            memory.put(Fixture::root(kind), &(TABLE as u64).to_le_bytes());
        }
        let mut retained = HeaderResolver::default();
        let census = crate::read_census::ReadCensus::new(&memory);
        for (kind, calls) in [
            (53, 4),
            (54, 4),
            (100, 4),
            (MID_TYPE_LIMIT, 4),
            (101, 1), // Same mid-type root, a different full handle.
            (53, 1),
            (55, 4), // Fifth distinct root replaces the oldest header.
            (53, 4),
        ] {
            census.reset();
            assert_eq!(
                retained.resolve(&census, 0, (42_u64 << 32) | u64::from(kind)),
                Ok(None)
            );
            assert_eq!(census.calls(), calls, "kind={kind}");
        }
    }

    #[test]
    fn scoped_headers_retain_nulls_failures_and_shared_roots_without_eviction() {
        let mut memory = Fixture::new(100);
        for kind in [53, 55, 56, 57, 58] {
            memory.put(Fixture::root(kind), &0_u64.to_le_bytes());
        }
        memory.put(Fixture::root(MID_TYPE_LIMIT), &(TABLE as u64).to_le_bytes());
        let mut scoped = ScopedResolver::new(0);
        let handle = |kind| (42_u64 << 32) | u64::from(kind);
        let census = crate::read_census::ReadCensus::new(&memory);
        for kind in [53, 55, 56, 57, 58] {
            census.reset();
            assert_eq!(scoped.resolve(&census, handle(kind)), Ok(None));
            assert_eq!((census.calls(), census.bytes()), (1, 8));
        }
        census.reset();
        assert_eq!(scoped.resolve(&census, handle(53)), Ok(None));
        assert_eq!(census.calls(), 0, "fifth root must not evict a null");
        assert_eq!(
            scoped.resolve(&census, handle(54)),
            Err("reference root unreadable")
        );
        census.reset();
        assert_eq!(
            scoped.resolve(&census, handle(54)),
            Err("reference root unreadable")
        );
        assert_eq!(census.calls(), 0, "failed roots must not retry");
        for (kind, calls) in [(100, 4), (101, 1), (MID_TYPE_LIMIT, 4), (u32::MAX, 1)] {
            census.reset();
            assert_eq!(scoped.resolve(&census, handle(kind)), Ok(None));
            assert_eq!(census.calls(), calls);
        }
        census.reset();
        assert_eq!(scoped.resolve(&census, 0), Ok(None));
        assert_eq!(census.calls(), 0);
        let mut overflow = ScopedResolver::new(usize::MAX);
        assert_eq!(
            overflow.resolve(&census, handle(53)),
            Err("reference root overflow")
        );
        assert_eq!(census.calls(), 0);
    }

    #[test]
    fn resolver_empty_roots_static_empty_and_robin_hood_stop_are_absence() {
        let empty = Fixture::default();
        let census = crate::read_census::ReadCensus::new(&empty);
        assert_eq!(resolve(&census, 0, 0), Ok(None));
        assert_eq!(census.calls(), 0);
        let handle = (0x7654_3210_u64 << 32) | 53;
        let mut memory = Fixture::new(53);
        assert_eq!(resolve(&memory, 0, handle), Ok(None));
        let index = Fixture::index(handle);
        memory.entry(index, 1, handle ^ (1 << 32), 0x9999);
        // The second cell's distance is below the probe distance. Its stale
        // key/pointer are not live, even when the bytes happen to match.
        memory.entry(index + 1, 1, handle, 0x8888);
        assert_eq!(resolve(&memory, 0, handle), Ok(None));
        memory.put(Fixture::root(53), &0_u64.to_le_bytes());
        let census = crate::read_census::ReadCensus::new(&memory);
        assert_eq!(resolve(&census, 0, handle), Ok(None));
        assert_eq!((census.calls(), census.bytes()), (1, 8));
        memory.put(Fixture::root(53), &(TABLE as u64).to_le_bytes());
        memory.put(TABLE + ENTRIES, &(STATIC_EMPTY_RVA.0 as u64).to_le_bytes());
        memory.put(TABLE + MASK, &0_u32.to_le_bytes());
        memory.put(TABLE + OVERFLOW_DISTANCE, &[0]);
        memory.put(STATIC_EMPTY_RVA.0, &[0; 2 * bucket::STRIDE]);
        memory.put(
            STATIC_EMPTY_RVA.0 + bucket::STRIDE + bucket::DISTANCE,
            &[255],
        );
        let census = crate::read_census::ReadCensus::new(&memory);
        assert_eq!(resolve(&census, 0, handle), Ok(None));
        assert_eq!(
            (census.calls(), census.bytes()),
            (4, 8 + 17 + 1 + bucket::STRIDE)
        );
    }

    #[test]
    fn resolver_rejects_malformed_headers_sentinels_and_bucket_faults() {
        let handle = (42_u64 << 32) | 53;
        for (address, bytes) in [
            (TABLE + COUNT, 9_u32.to_le_bytes().to_vec()),
            (TABLE + MASK, 6_u32.to_le_bytes().to_vec()),
            (TABLE + MASK, u32::MAX.to_le_bytes().to_vec()),
            (TABLE + OVERFLOW_DISTANCE, vec![4]),
            (TABLE + OVERFLOW_DISTANCE, vec![255]),
            (
                ENTRIES_ADDRESS + 13 * bucket::STRIDE + bucket::DISTANCE,
                vec![0],
            ),
            (
                ENTRIES_ADDRESS + Fixture::index(handle) * bucket::STRIDE + bucket::DISTANCE,
                vec![255],
            ),
            (
                ENTRIES_ADDRESS + Fixture::index(handle) * bucket::STRIDE + bucket::DISTANCE,
                vec![7],
            ),
        ] {
            let mut memory = Fixture::new(53);
            memory.put(address, &bytes);
            assert!(resolve(&memory, 0, handle).is_err(), "{address:x}");
        }
        let mut memory = Fixture::new(53);
        memory
            .bytes
            .remove(&(ENTRIES_ADDRESS + Fixture::index(handle) * bucket::STRIDE));
        assert!(resolve(&memory, 0, handle).is_err());
        assert!(resolve(&memory, usize::MAX, handle).is_err());
    }

    #[test]
    fn resolver_primary_read_failures_are_errors_and_stop_at_the_failed_source() {
        let handle = (42_u64 << 32) | 53;
        for (address, calls, reason) in [
            (Fixture::root(53), 1, "reference root unreadable"),
            (TABLE + ENTRIES, 2, "reference header unreadable"),
            (
                ENTRIES_ADDRESS + 13 * bucket::STRIDE + bucket::DISTANCE,
                3,
                "reference sentinel or count invalid",
            ),
            (
                ENTRIES_ADDRESS + Fixture::index(handle) * bucket::STRIDE,
                4,
                "reference bucket unreadable",
            ),
        ] {
            let mut memory = Fixture::new(53);
            memory.entry(Fixture::index(handle), 1, handle, 0x8888);
            memory.put(TABLE + COUNT, &1_u32.to_le_bytes());
            memory.bytes.remove(&address);
            let census = crate::read_census::ReadCensus::new(&memory);
            assert_eq!(resolve(&census, 0, handle), Err(reason));
            assert_eq!(census.calls(), calls);
            census.reset();
            assert_eq!(
                HeaderResolver::default().resolve(&census, 0, handle),
                Err(reason)
            );
            assert_eq!(census.calls(), calls);
        }
    }

    fn province_roster_fixture(kind: u32) -> (Fixture, [u64; 2]) {
        use crate::{carmy, cgame_state as gs, cprovince as p, cunit};
        let mut f = Fixture::new(kind);
        f.put(0x4000, &vec![0; 0x5000]);
        f.put(0x4000 + gs::PROVINCES, &0x5000u64.to_le_bytes());
        f.put(0x4000 + gs::PROVINCES + 8, &2u32.to_le_bytes());
        f.put(0x4000 + gs::PROVINCES + 12, &2u32.to_le_bytes());
        f.put(0x5008, &0x6000u64.to_le_bytes());
        f.put(0x6000, &(p::VTABLE_RVA.0 as u64).to_le_bytes());
        f.put(0x6000 + p::ID, &1u32.to_le_bytes());
        f.put(0x6000 + p::OCCUPANTS, &0x6500u64.to_le_bytes());
        f.put(0x6000 + p::OCCUPANTS + 8, &2u32.to_le_bytes());
        f.put(0x6000 + p::OCCUPANTS + 12, &2u32.to_le_bytes());
        let ids = [
            (4u64 << 32) | u64::from(kind),
            (2u64 << 32) | u64::from(kind),
        ];
        for (i, &id) in ids.iter().enumerate() {
            let unit = 0x7000 + i * 0x1000;
            f.put(0x6500 + i * 8, &(unit as u64).to_le_bytes());
            f.put(unit, &(carmy::VTABLE_RVA.0 as u64).to_le_bytes());
            f.put(unit + cunit::HANDLE, &id.to_le_bytes());
            f.put(unit + cunit::LOCATION, &0x6000u64.to_le_bytes());
            f.entry(Fixture::index(id), 1, id, (unit + 0x10) as u64);
        }
        f.put(TABLE + COUNT, &2u32.to_le_bytes());
        (f, ids)
    }

    #[test]
    fn province_roster_keeps_native_order_and_rejects_backlink_and_whole_row_capacity_failures() {
        use crate::{cprovince as p, cunit, walk::Outcome};
        let (mut f, ids) = province_roster_fixture(51);
        let mut rows = [p::UnitOrderRow::default(); 1];
        let mut refs = [0; 2];
        let result = p::walk_unit_order(&f, 0, 0x4000, &mut rows, &mut refs);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(refs, ids);
        let result = p::walk_unit_order(&f, 0, 0x4000, &mut rows, &mut refs[..1]);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(result.required, 2);
        f.put(0x7000 + cunit::LOCATION, &0u64.to_le_bytes());
        assert!(matches!(
            p::walk_unit_order(&f, 0, 0x4000, &mut rows, &mut refs).outcome,
            Outcome::Unavailable { .. }
        ));
    }
    /// Divisions may carry dynamic reference types, including 4713, rather
    /// than only type 51. Admission must retain the full dynamic reference.
    #[test]
    fn province_roster_accepts_dynamic_reference_type_divisions() {
        use crate::{cprovince as p, walk::Outcome};
        let (f, ids) = province_roster_fixture(4713);
        let mut rows = [p::UnitOrderRow::default(); 1];
        let mut refs = [0; 2];
        let result = p::walk_unit_order(&f, 0, 0x4000, &mut rows, &mut refs);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(refs, ids);
        assert_eq!(
            rows[0],
            p::UnitOrderRow {
                province: 1,
                mixed: [0, 2],
                naval: [2, 0],
            }
        );
    }
}
