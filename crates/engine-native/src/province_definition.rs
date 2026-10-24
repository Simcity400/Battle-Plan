//! Province definition reached through a province.

/// Province-definition allocation size.
pub const SIZE: usize = 0xd8;
/// Run-length pixel-span array.
pub const PIXEL_SPANS: usize = 0x58;
/// Province-neighbour array.
pub const NEIGHBOURS: usize = 0x70;
/// Signed i32 neighbour count.
pub const NEIGHBOUR_COUNT: usize = 0x7c;
/// Province-definition u32 ID.
pub const ID: usize = 0xc4;
/// Province-definition terrain flag byte.
pub const TERRAIN_FLAGS: usize = 0xd2;
/// The [`TERRAIN_FLAGS`] bit set for a land province; clear, the province is
/// sea, and the land movement speed moves a leg to or from it at the owner's
/// transport speed.
pub const LAND_FLAG: u8 = 1;
/// The lake bit; invasion selection requires both land and lake bits clear.
pub const LAKE_FLAG: u8 = 2;
/// Signed i32 port sea-zone ID identifying the sea province reached by a
/// land port. Construction clears it. Authored port slots store their seventh
/// field even below one; path-edge classification compares it with the sea neighbour.
pub const PORT_SEA: usize = 0xc8;
/// Terrain-type pointer reached through the province definition.
pub const TERRAIN: usize = 0xa8;
/// ID-taking constructor, clearing centroid fields.
pub const CONSTRUCTOR_RVA: usize = 0x0140_84f0;

/// Province IDs in the raster are u16, bounding the oracle census to their
/// representable range. Refuse a larger count before sizing allocation.
pub const MAX_CENSUS_RECORDS: u32 = 1 << 16;

/// The movement operands one definition stores, for the one-shot load oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementDefinition {
    /// Raw `TERRAIN_FLAGS` byte.
    pub terrain_flags: u8,
    /// One-based terrain ID in the definition's selected terrain object.
    /// The null terrain object holds zero.
    pub terrain_index: i32,
    /// The `NEIGHBOURS` entries in native order.
    pub neighbours: Vec<crate::province_neighbour::Record>,
}

/// The province census's definitions, by province ID. Record zero is the
/// native null record and is not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementCensus {
    /// The census count, record zero included.
    pub records: u32,
    /// `provinces[id - 1]`: the definition, or why that province could not
    /// be read. One failure never hides the others.
    pub provinces: Vec<Result<MovementDefinition, &'static str>>,
}

/// Reads every province definition's movement operands for the recorder's
/// one-shot load oracle. It allocates and reads the whole map, so it never
/// runs in the capture path. Each province is authenticated as the capture
/// readers do: census backlink, `CProvince` vtable and ID, the definition's own
/// ID, and a `CTerrainType` or null-terrain vtable. Counts read from the game
/// are bounded before they size an allocation.
pub fn read_movement_census(
    memory: &impl crate::Memory,
    base: usize,
) -> Result<MovementCensus, &'static str> {
    use crate::{cgame_state, cprovince, cterrain_type, province_neighbour as neighbour, read};
    let module = |rva: usize| base.checked_add(rva).ok_or("module address overflow");
    let game_state = read::<u64>(memory, module(cgame_state::SINGLETON_RVA)?)
        .filter(|&pointer| pointer != 0)
        .ok_or("game state unreadable or null")? as usize;
    let census = game_state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
        .ok_or("province census header unreadable or invalid")?;
    if census.count < 2 {
        return Err("no province census: load a save first");
    }
    if census.count > MAX_CENSUS_RECORDS {
        return Err("province census exceeds the native u16 province IDs");
    }
    let province_vtable = module(cprovince::VTABLE_RVA)? as u64;
    let terrain_vtables = [
        module(cterrain_type::VTABLE_RVA)? as u64,
        module(cterrain_type::NULL_VTABLE_RVA)? as u64,
    ];
    let field =
        |pointer: usize, offset: usize| pointer.checked_add(offset).ok_or("address overflow");
    let province = |id: u32| -> Result<MovementDefinition, &'static str> {
        let pointer = census
            .element(id, 8)
            .and_then(|address| read::<u64>(memory, address))
            .ok_or("census slot unreadable")? as usize;
        if pointer == 0 {
            return Err("census slot null");
        }
        if read::<u64>(memory, pointer) != Some(province_vtable)
            || read::<u32>(memory, field(pointer, cprovince::ID)?) != Some(id)
        {
            return Err("province identity mismatch");
        }
        let definition = read::<u64>(memory, field(pointer, cprovince::DEFINITION)?)
            .filter(|&definition| definition != 0)
            .ok_or("definition pointer unreadable or null")? as usize;
        if read::<u32>(memory, field(definition, ID)?) != Some(id) {
            return Err("definition identity mismatch");
        }
        let terrain_flags = read::<u8>(memory, field(definition, TERRAIN_FLAGS)?)
            .ok_or("terrain flags unreadable")?;
        let terrain = read::<u64>(memory, field(definition, TERRAIN)?)
            .filter(|&terrain| terrain != 0)
            .ok_or("terrain pointer unreadable or null")? as usize;
        if !read::<u64>(memory, terrain).is_some_and(|vtable| terrain_vtables.contains(&vtable)) {
            return Err("terrain vtable mismatch");
        }
        let terrain_index = read::<i32>(memory, field(terrain, cterrain_type::DATABASE_INDEX)?)
            .ok_or("terrain index unreadable")?;
        let entries = crate::cpdx_array::Array::read(
            memory,
            field(definition, NEIGHBOURS)?,
            neighbour::STRIDE,
        )
        .ok_or("neighbour header unreadable or invalid")?;
        // Appending skips an existing target, so the neighbour list holds distinct census records.
        if entries.count > census.count {
            return Err("neighbour count exceeds the province census");
        }
        let mut bytes = vec![0; entries.count as usize * neighbour::STRIDE];
        if !crate::read_slice(memory, entries.data, &mut bytes) {
            return Err("neighbour entries unreadable");
        }
        let neighbours = bytes
            .chunks_exact(neighbour::STRIDE)
            .map(|entry| neighbour::Record::decode(entry.try_into().unwrap()))
            .collect();
        Ok(MovementDefinition {
            terrain_flags,
            terrain_index,
            neighbours,
        })
    };
    Ok(MovementCensus {
        records: census.count,
        provinces: (1..census.count).map(province).collect(),
    })
}

/// Independently demanded definition fields retained by one owned geometry
/// acquisition. A failed field is remembered without invalidating another
/// branch that never consumes it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeometryFields {
    pub id: Option<Result<u32, &'static str>>,
    pub flags: Option<Result<u8, &'static str>>,
    pub terrain: Option<Result<usize, &'static str>>,
    pub neighbours: Option<Result<[u8; 16], &'static str>>,
    pub prefix_start: Option<usize>,
    pub prefix_len: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cgame_state, cprovince, cterrain_type, memory::SliceMemory, province_neighbour as neighbour,
    };

    const MODULE: usize = 0x1_4000_0000;
    const START: usize = MODULE + cgame_state::SINGLETON_RVA;
    const GAME_STATE: usize = START + 0x100;
    const CENSUS: usize = START + 0x1000;
    const TERRAIN_A: usize = START + 0x2000;
    const NULL_TERRAIN: usize = START + 0x2100;

    struct Image(Vec<u8>);
    impl Image {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0[at - START..at - START + bytes.len()].copy_from_slice(bytes);
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
        fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: START,
                bytes: &self.0,
            }
        }
    }

    fn province(id: u32) -> usize {
        START + 0x10_000 * id as usize
    }
    fn definition(id: u32) -> usize {
        province(id) + 0x1000
    }
    fn entries(id: u32) -> usize {
        province(id) + 0x2000
    }

    /// Province 1 has a real terrain and two neighbours; province 2 the null
    /// terrain and one neighbour.
    fn fixture() -> Image {
        let mut image = Image(vec![0; 0x40_000]);
        image.u64(START, GAME_STATE as u64);
        image.array(GAME_STATE + cgame_state::PROVINCES, CENSUS, 3);
        image.u64(TERRAIN_A, (MODULE + cterrain_type::VTABLE_RVA) as u64);
        image.u32(TERRAIN_A + cterrain_type::DATABASE_INDEX, 7);
        image.u64(
            NULL_TERRAIN,
            (MODULE + cterrain_type::NULL_VTABLE_RVA) as u64,
        );
        let rows: [(u32, usize, u8, &[neighbour::Record]); 2] = [
            (
                1,
                TERRAIN_A,
                9,
                &[
                    neighbour::Record {
                        id: 2,
                        crossing_kind: 3,
                        flag: 1,
                        distance: -233_333,
                        crossing: [-1, 0],
                    },
                    neighbour::Record {
                        id: 5,
                        crossing_kind: 0,
                        flag: 0,
                        distance: 0xffff_ffff,
                        crossing: [120, 4000],
                    },
                ],
            ),
            (
                2,
                NULL_TERRAIN,
                16,
                &[neighbour::Record {
                    id: 1,
                    crossing_kind: 3,
                    flag: 1,
                    distance: -233_333,
                    crossing: [-1, 0],
                }],
            ),
        ];
        for (id, terrain, flags, records) in rows {
            image.u64(CENSUS + 8 * id as usize, province(id) as u64);
            image.u64(province(id), (MODULE + cprovince::VTABLE_RVA) as u64);
            image.u32(province(id) + cprovince::ID, id);
            image.u64(province(id) + cprovince::DEFINITION, definition(id) as u64);
            image.u32(definition(id) + ID, id);
            image.put(definition(id) + TERRAIN_FLAGS, &[flags]);
            image.u64(definition(id) + TERRAIN, terrain as u64);
            image.array(
                definition(id) + NEIGHBOURS,
                entries(id),
                records.len() as u32,
            );
            for (index, record) in records.iter().enumerate() {
                let at = entries(id) + index * neighbour::STRIDE;
                image.u32(at + neighbour::PROVINCE_ID, record.id);
                image.put(
                    at + neighbour::CROSSING_KIND,
                    &[record.crossing_kind, record.flag],
                );
                image.u64(at + neighbour::MOVEMENT_DISTANCE, record.distance as u64);
                image.u32(at + neighbour::CROSSING_POINT, record.crossing[0] as u32);
                image.u32(
                    at + neighbour::CROSSING_POINT + 4,
                    record.crossing[1] as u32,
                );
                // Padding the reader must ignore.
                image.put(at, &[0xAA; 8]);
            }
        }
        image
    }

    #[test]
    fn every_definition_reads_its_raw_entries_flags_and_terrain_ordinal() {
        let image = fixture();
        let census = read_movement_census(&image.memory(), MODULE).unwrap();
        assert_eq!(census.records, 3);
        let first = census.provinces[0].as_ref().unwrap();
        assert_eq!((first.terrain_flags, first.terrain_index), (9, 7));
        assert_eq!(
            first.neighbours,
            [
                neighbour::Record {
                    id: 2,
                    crossing_kind: 3,
                    flag: 1,
                    distance: -233_333,
                    crossing: [-1, 0],
                },
                neighbour::Record {
                    id: 5,
                    crossing_kind: 0,
                    flag: 0,
                    distance: 0xffff_ffff,
                    crossing: [120, 4000],
                },
            ]
        );
        let second = census.provinces[1].as_ref().unwrap();
        assert_eq!((second.terrain_flags, second.terrain_index), (16, 0));
        assert_eq!(second.neighbours.len(), 1);
    }

    #[test]
    fn one_bad_province_is_reported_without_hiding_the_others() {
        for (at, value, reason) in [
            (CENSUS + 16, 0, "census slot null"),
            (province(2), 0, "province identity mismatch"),
            (province(2) + cprovince::ID, 1, "province identity mismatch"),
            (definition(2) + ID, 3, "definition identity mismatch"),
            (NULL_TERRAIN, 0, "terrain vtable mismatch"),
            (
                definition(2) + NEIGHBOURS + 8,
                0,
                "neighbour header unreadable or invalid",
            ),
            (
                definition(2) + NEIGHBOURS,
                (START + 0x3f_fff) as u64,
                "neighbour entries unreadable",
            ),
        ] {
            let mut image = fixture();
            if at == definition(2) + NEIGHBOURS + 8 {
                // Capacity below the count.
                image.u32(at, value as u32);
            } else if at == province(2) + cprovince::ID || at == definition(2) + ID {
                image.u32(at, value as u32);
            } else {
                image.u64(at, value);
            }
            let census = read_movement_census(&image.memory(), MODULE).unwrap();
            assert!(census.provinces[0].is_ok(), "{reason}");
            assert_eq!(census.provinces[1], Err(reason));
        }
    }

    #[test]
    fn counts_beyond_the_census_or_u16_ids_are_refused_before_allocating() {
        // A neighbour header claiming i32::MAX entries over readable data.
        let mut image = fixture();
        image.array(definition(2) + NEIGHBOURS, entries(2), i32::MAX as u32);
        let census = read_movement_census(&image.memory(), MODULE).unwrap();
        assert!(census.provinces[0].is_ok());
        assert_eq!(
            census.provinces[1],
            Err("neighbour count exceeds the province census")
        );
        // One more entry than the census has records.
        image.array(definition(2) + NEIGHBOURS, entries(2), 4);
        assert_eq!(
            read_movement_census(&image.memory(), MODULE)
                .unwrap()
                .provinces[1],
            Err("neighbour count exceeds the province census")
        );

        let mut image = fixture();
        image.array(GAME_STATE + cgame_state::PROVINCES, CENSUS, i32::MAX as u32);
        assert_eq!(
            read_movement_census(&image.memory(), MODULE),
            Err("province census exceeds the native u16 province IDs")
        );
        image.array(
            GAME_STATE + cgame_state::PROVINCES,
            CENSUS,
            MAX_CENSUS_RECORDS + 1,
        );
        assert_eq!(
            read_movement_census(&image.memory(), MODULE),
            Err("province census exceeds the native u16 province IDs")
        );
    }

    #[test]
    fn an_unloaded_or_unreadable_census_is_refused() {
        let mut image = fixture();
        image.array(GAME_STATE + cgame_state::PROVINCES, 0, 0);
        assert_eq!(
            read_movement_census(&image.memory(), MODULE),
            Err("no province census: load a save first")
        );
        image.u64(START, 0);
        assert_eq!(
            read_movement_census(&image.memory(), MODULE),
            Err("game state unreadable or null")
        );
    }
}
