use super::*;

/// What the map files define for one province. None of it changes while a save
/// is loaded, so an entry read once serves every later hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticProvince {
    pub pointer: usize,
    pub definition: usize,
    pub flags: u8,
    pub terrain: usize,
    pub terrain_index: i32,
}

/// The adjacency between two provinces, or its absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticEdge {
    /// The native first-adjacency distance, including the 100000 fallback a
    /// pair with no adjacency takes.
    pub distance: i64,
    pub crossing: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StaticKey {
    base: usize,
    state: usize,
    session: i64,
    root: [u8; 16],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticCounters {
    pub hits: u64,
    pub misses: u64,
    pub cleared: u64,
}

/// Immutable province geometry kept across hours, owned by one acquisition
/// range. Adjacencies and the embark define are reloadable and are retained
/// only within a walk. Every entry comes from an authenticated live read; a new
/// module base, game state, play session or province census array clears the
/// whole memo. `begin` validates the key before immutable entries are reused.
#[derive(Default)]
pub struct StaticGeometry {
    key: Option<StaticKey>,
    root: Option<Array>,
    provinces: Vec<Option<StaticProvince>>,
    edges: std::collections::HashMap<
        u64,
        StaticEdge,
        std::hash::BuildHasherDefault<ProvincePairHasher>,
    >,
    embark: Option<i64>,
    counters: StaticCounters,
}

impl StaticGeometry {
    /// Re-key the memo for this hour, clearing it when the session or the
    /// province census it was built from changed. Reads the current province
    /// header and session identity once each.
    pub fn begin(&mut self, memory: &impl Memory, base: usize, state: usize) -> bool {
        // The counters describe this walk, not the memo's whole life.
        self.counters = StaticCounters::default();
        // LoadGameDefines (14073dda0), including the common/defines file-watch
        // callback 140174be0, rewrites this define without changing the session
        // or province census. Only immutable map geometry survives this walk.
        self.embark = None;
        // reloadfx straits (14027a250 -> 140a53290) updates existing crossing
        // kinds through 1413f51e0 and may append adjacencies. It does not replace
        // the province census or begin a new play session.
        self.edges.clear();
        let root = state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|address| crate::read::<[u8; 16]>(memory, address));
        let session = base
            .checked_add(cgame_state::PLAY_SESSION_STARTED_AT_RVA)
            .and_then(|address| crate::read::<i64>(memory, address));
        let key = root.zip(session).map(|(root, session)| StaticKey {
            base,
            state,
            session,
            root,
        });
        if key.is_none() || self.key != key {
            self.clear();
            self.counters.cleared = self.counters.cleared.saturating_add(1);
        }
        // An unreadable key leaves the memo unusable rather than denying the
        // walk: every lookup then reads live and nothing is retained.
        self.key = key;
        self.root = root.and_then(|root| Array::from_header(root, 8));
        self.root.is_some()
    }

    /// True when this hour's key is known, so entries may be served and kept.
    fn usable(&self) -> bool {
        self.key.is_some()
    }

    pub fn clear(&mut self) {
        self.key = None;
        self.root = None;
        self.provinces.clear();
        self.edges.clear();
        self.embark = None;
    }

    pub fn counters(&self) -> StaticCounters {
        self.counters
    }

    pub fn allocated_bytes(&self) -> usize {
        self.provinces.capacity() * std::mem::size_of::<Option<StaticProvince>>()
            + self.edges.capacity() * (std::mem::size_of::<(u64, StaticEdge)>() + 1)
    }

    fn stored(&self, id: u32) -> Option<StaticProvince> {
        *self.provinces.get(id as usize)?
    }

    /// One province's map data, from the memo or from an authenticated read.
    pub fn province(
        &mut self,
        memory: &impl Memory,
        base: usize,
        id: u32,
    ) -> Result<StaticProvince, &'static str> {
        if self.usable()
            && let Some(province) = self.stored(id)
        {
            self.counters.hits = self.counters.hits.saturating_add(1);
            return Ok(province);
        }
        self.counters.misses = self.counters.misses.saturating_add(1);
        let province = self.read_province(memory, base, id)?;
        if self.usable() {
            if self.provinces.len() <= id as usize {
                self.provinces.resize(id as usize + 1, None);
            }
            self.provinces[id as usize] = Some(province);
        }
        Ok(province)
    }

    /// The same authentication `GeometryPacket` performs: the census backlink,
    /// the province vtable and identity, and the definition's own identity.
    fn read_province(
        &self,
        memory: &impl Memory,
        base: usize,
        id: u32,
    ) -> Result<StaticProvince, &'static str> {
        let root = self.root.ok_or("static geometry province root invalid")?;
        let pointer = root
            .element(id, 8)
            .and_then(|address| crate::read::<u64>(memory, address))
            .map(|pointer| pointer as usize)
            .ok_or("static geometry province outside census or unreadable")?;
        if id == 0 || pointer == 0 {
            return Err("static geometry province identity mismatch");
        }
        // The three province fields are independent, as are the three
        // definition fields, so each group is one prefetched scattered copy.
        let (mut vtable, mut identity, mut definition) = ([0_u8; 8], [0_u8; 4], [0_u8; 8]);
        let fields = [
            pointer,
            pointer.checked_add(ID).ok_or("province address overflow")?,
            pointer
                .checked_add(DEFINITION)
                .ok_or("province address overflow")?,
        ];
        if memory
            .copy_scattered_prefetched(fields, &mut [&mut vtable, &mut identity, &mut definition])
            != fields.len()
        {
            return Err("static geometry province unreadable");
        }
        if u64::from_le_bytes(vtable)
            != base
                .checked_add(VTABLE_RVA)
                .ok_or("module address overflow")? as u64
            || u32::from_le_bytes(identity) != id
        {
            return Err("static geometry province identity mismatch");
        }
        let definition = u64::from_le_bytes(definition) as usize;
        let (mut definition_id, mut flags, mut terrain) = ([0_u8; 4], [0_u8; 1], [0_u8; 8]);
        let fields = [
            definition
                .checked_add(crate::province_definition::ID)
                .ok_or("province address overflow")?,
            definition
                .checked_add(crate::province_definition::TERRAIN_FLAGS)
                .ok_or("province address overflow")?,
            definition
                .checked_add(crate::province_definition::TERRAIN)
                .ok_or("province address overflow")?,
        ];
        if definition == 0
            || memory.copy_scattered_prefetched(
                fields,
                &mut [&mut definition_id, &mut flags, &mut terrain],
            ) != fields.len()
            || u32::from_le_bytes(definition_id) != id
        {
            return Err("static geometry province definition mismatch");
        }
        let terrain = u64::from_le_bytes(terrain) as usize;
        Ok(StaticProvince {
            pointer,
            definition,
            flags: flags[0],
            terrain,
            terrain_index: geometry_field(memory, terrain, crate::cterrain_type::DATABASE_INDEX)?,
        })
    }

    /// The native first adjacency between two provinces, from the memo or from
    /// a live scan of the source province's neighbour list.
    pub fn edge(
        &mut self,
        memory: &impl Memory,
        base: usize,
        from: u32,
        to: u32,
    ) -> Result<StaticEdge, &'static str> {
        let key = (u64::from(from) << 32) | u64::from(to);
        if self.usable()
            && let Some(edge) = self.edges.get(&key)
        {
            self.counters.hits = self.counters.hits.saturating_add(1);
            return Ok(*edge);
        }
        self.counters.misses = self.counters.misses.saturating_add(1);
        let definition = self.province(memory, base, from)?.definition;
        let header = geometry_field(memory, definition, crate::province_definition::NEIGHBOURS)?;
        let neighbours = Array::from_header(header, crate::province_neighbour::STRIDE)
            .ok_or("static geometry adjacency header invalid")?;
        // The native walk takes the first entry naming the destination; a pair
        // with none takes the 100000 fallback, which is memoized like any other.
        let mut edge = StaticEdge {
            distance: 100_000,
            crossing: None,
        };
        // The whole scanned prefix is one contiguous span; hint it once.
        if let Some(first) = neighbours.element(0, crate::province_neighbour::STRIDE) {
            crate::prefetch::span(
                first,
                (neighbours.count as usize)
                    .saturating_mul(crate::province_neighbour::STRIDE)
                    .min(4096),
            );
        }
        for slot in 0..neighbours.count {
            let address = neighbours
                .element(slot, crate::province_neighbour::STRIDE)
                .ok_or("static geometry adjacency overflow")?;
            if geometry_field::<u32>(memory, address, crate::province_neighbour::PROVINCE_ID)? == to
            {
                let (mut distance, mut crossing) = ([0_u8; 8], [0_u8; 1]);
                let fields = [
                    address
                        .checked_add(crate::province_neighbour::MOVEMENT_DISTANCE)
                        .ok_or("adjacency address overflow")?,
                    address
                        .checked_add(crate::province_neighbour::CROSSING_KIND)
                        .ok_or("adjacency address overflow")?,
                ];
                if memory.copy_scattered_prefetched(fields, &mut [&mut distance, &mut crossing])
                    != fields.len()
                {
                    return Err("static geometry adjacency unreadable");
                }
                edge = StaticEdge {
                    distance: i64::from_le_bytes(distance),
                    crossing: Some(crossing[0]),
                };
                break;
            }
        }
        if self.usable() {
            self.edges.insert(key, edge);
        }
        Ok(edge)
    }

    /// The reloadable native embark distance define, read once per walk.
    pub fn embark(&mut self, memory: &impl Memory, base: usize) -> Result<i64, &'static str> {
        if self.usable()
            && let Some(distance) = self.embark
        {
            self.counters.hits = self.counters.hits.saturating_add(1);
            return Ok(distance);
        }
        self.counters.misses = self.counters.misses.saturating_add(1);
        let distance = geometry_field(memory, base, crate::cunit::EMBARK_MOVEMENT_DISTANCE_RVA)?;
        if self.usable() {
            self.embark = Some(distance);
        }
        Ok(distance)
    }
}
