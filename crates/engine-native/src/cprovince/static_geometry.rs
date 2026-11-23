use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticProvince {
    pub pointer: usize,
    pub definition: usize,
    pub flags: u8,
    pub terrain: usize,
    pub terrain_index: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticEdge {
    pub distance: i64,
    pub crossing: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticGeometryKey {
    base: usize,
    state: usize,
    session: i64,
    root: [u8; 16],
}

fn observe_static_key(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> (Option<StaticGeometryKey>, Option<Array>) {
    let root = state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|address| engine_native_memory::read::<[u8; 16]>(memory, address));
    let session = engine_sites::at(base, cgame_state::PLAY_SESSION_STARTED_AT_RVA)
        .and_then(|address| engine_native_memory::read::<i64>(memory, address));
    let key = root.zip(session).map(|(root, session)| StaticGeometryKey {
        base,
        state,
        session,
        root,
    });
    (key, root.and_then(|root| Array::from_header(root, 8)))
}

pub fn static_geometry_demand_key(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<StaticGeometryKey, &'static str> {
    let (key, root) = observe_static_key(memory, base, state);
    root.ok_or("static geometry province root invalid")?;
    key.ok_or("static geometry session key unavailable")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticCounters {
    pub hits: u64,
    pub misses: u64,
    pub cleared: u64,
}

#[derive(Default)]
pub struct StaticGeometry {
    key: Option<StaticGeometryKey>,
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
    pub fn begin(&mut self, memory: &impl Memory, base: usize, state: usize) -> bool {
        self.counters = StaticCounters::default();
        self.embark = None;
        self.edges.clear();
        let (key, root) = observe_static_key(memory, base, state);
        if key.is_none() || self.key != key {
            self.clear();
            self.counters.cleared = self.counters.cleared.saturating_add(1);
        }
        self.key = key;
        self.root = root;
        self.root.is_some()
    }

    fn usable(&self) -> bool {
        self.key.is_some()
    }

    pub fn province_count(&self) -> Result<usize, &'static str> {
        self.demand_key()?;
        self.root
            .map(|root| root.count as usize)
            .ok_or("static geometry province root invalid")
    }

    pub fn demand_key(&self) -> Result<StaticGeometryKey, &'static str> {
        self.root.ok_or("static geometry province root invalid")?;
        self.key.ok_or("static geometry session key unavailable")
    }

    pub fn reserve_slots(&mut self, count: usize) -> std::io::Result<()> {
        if self.provinces.len() < count {
            engine_allocation::resize(&mut self.provinces, count, None)?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.key = None;
        self.root = None;
        self.provinces.fill(None);
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
                engine_allocation::resize(&mut self.provinces, id as usize + 1, None)
                    .map_err(|_| "static geometry allocation failed")?;
            }
            self.provinces[id as usize] = Some(province);
        }
        Ok(province)
    }

    fn read_province(
        &self,
        memory: &impl Memory,
        base: usize,
        id: u32,
    ) -> Result<StaticProvince, &'static str> {
        let root = self.root.ok_or("static geometry province root invalid")?;
        let pointer = root
            .element(id, 8)
            .and_then(|address| engine_native_memory::read::<u64>(memory, address))
            .map(|pointer| pointer as usize)
            .ok_or("static geometry province outside census or unreadable")?;
        if id == 0 || pointer == 0 {
            return Err("static geometry province identity mismatch");
        }
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
            != engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64
            || u32::from_le_bytes(identity) != id
        {
            return Err("static geometry province identity mismatch");
        }
        let definition = u64::from_le_bytes(definition) as usize;
        let (mut definition_id, mut flags, mut terrain) = ([0_u8; 4], [0_u8; 1], [0_u8; 8]);
        let fields = [
            definition
                .checked_add(crate::cprovince::definition::ID)
                .ok_or("province address overflow")?,
            definition
                .checked_add(crate::cprovince::definition::TERRAIN_FLAGS)
                .ok_or("province address overflow")?,
            definition
                .checked_add(crate::cprovince::definition::TERRAIN)
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
            terrain_index: geometry_field(memory, terrain, engine_native_parts::cterrain_type::DATABASE_INDEX)?,
        })
    }

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
        let header = geometry_field(memory, definition, crate::cprovince::definition::NEIGHBOURS)?;
        let neighbours = Array::from_header(header, engine_native_parts::province_neighbour::STRIDE)
            .ok_or("static geometry adjacency header invalid")?;
        let mut edge = StaticEdge {
            distance: 100_000,
            crossing: None,
        };
        if let Some(first) = neighbours.element(0, engine_native_parts::province_neighbour::STRIDE) {
            engine_native_memory::prefetch::span(
                first,
                (neighbours.count as usize)
                    .saturating_mul(engine_native_parts::province_neighbour::STRIDE)
                    .min(4096),
            );
        }
        for slot in 0..neighbours.count {
            let address = neighbours
                .element(slot, engine_native_parts::province_neighbour::STRIDE)
                .ok_or("static geometry adjacency overflow")?;
            if geometry_field::<u32>(memory, address, engine_native_parts::province_neighbour::PROVINCE_ID)? == to
            {
                let (mut distance, mut crossing) = ([0_u8; 8], [0_u8; 1]);
                let fields = [
                    address
                        .checked_add(engine_native_parts::province_neighbour::MOVEMENT_DISTANCE)
                        .ok_or("adjacency address overflow")?,
                    address
                        .checked_add(engine_native_parts::province_neighbour::CROSSING_KIND)
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
            engine_allocation::reserve_map(&mut self.edges, 1)
                .map_err(|_| "static geometry allocation failed")?;
            self.edges.insert(key, edge);
        }
        Ok(edge)
    }

    pub fn embark(&mut self, memory: &impl Memory, base: usize) -> Result<i64, &'static str> {
        if self.usable()
            && let Some(distance) = self.embark
        {
            self.counters.hits = self.counters.hits.saturating_add(1);
            return Ok(distance);
        }
        self.counters.misses = self.counters.misses.saturating_add(1);
        let distance = engine_sites::at(base, crate::cunit::EMBARK_MOVEMENT_DISTANCE_RVA)
            .and_then(|address| engine_native_memory::read(memory, address))
            .ok_or("unit geometry field unreadable")?;
        if self.usable() {
            self.embark = Some(distance);
        }
        Ok(distance)
    }
}
