
pub mod adjacency;
pub(crate) mod bitmap;
mod geometry;
pub(crate) mod records;
pub mod terrain;

use std::{
    collections::{BTreeMap, HashMap},
    io,
};

use self::{
    bitmap::NativeBitmap,
    geometry::{Crossing, Raster},
    records::{Kind, Provinces},
    terrain::TerrainDatabase,
};
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_parser::fields::invalid;

pub fn raster_neighbours(
    width: usize,
    height: usize,
    top_down: &[u32],
) -> io::Result<Vec<Vec<u32>>> {
    if width == 0 || width.checked_mul(height) != Some(top_down.len()) {
        return Err(invalid("the province raster does not match its dimensions"));
    }
    let bottom_up: Vec<u32> = top_down
        .chunks_exact(width)
        .rev()
        .flatten()
        .copied()
        .collect();
    let count = bottom_up.iter().max().map_or(1, |&id| id as usize + 1);
    adjacency::raster(width, height, &bottom_up, count)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProvinceTerrain {
    pub flags: u8,
    pub index: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovementDefines {
    pub embark_distance: i64,
    pub transport_scale: i64,
    pub invasion_speed: i64,
    pub outer_scale: i64,
    pub river_crossing_two: i64,
    pub river_crossing_three: i64,
    pub organisation: Result<(i64, i64), String>,
    pub noncore_supply: [Result<i64, String>; 2],
    pub combat_movement: Result<i64, String>,
    pub rail: Result<RailMovementDefines, String>,
    pub naval: Result<NavalMovementDefines, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalMovementDefines {
    pub range_scale: i64,
    pub use_home_base: Result<bool, String>,
}

impl NavalMovementDefines {
    fn read(defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            range_scale: defines.fixed5("NNavy", "NAVAL_RANGE_TO_INGAME_DISTANCE")?,
            use_home_base: defines
                .boolean("NCountry", "NAVY_USE_HOME_BASE_FOR_RANGE")
                .map_err(|error| error.to_string()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RailMovementDefines {
    pub speed: [i64; 4],
    pub maximum_level: i32,
}

impl RailMovementDefines {
    fn read(defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            speed: [
                defines.fixed5("NMilitary", "STRATEGIC_SPEED_INFRA_BASE")?,
                defines.fixed5("NMilitary", "STRATEGIC_SPEED_INFRA_MAX")?,
                defines.fixed5("NMilitary", "STRATEGIC_SPEED_RAIL_BASE")?,
                defines.fixed5("NMilitary", "STRATEGIC_SPEED_RAIL_MAX")?,
            ],
            maximum_level: defines.int32("NSupply", "MAX_RAILWAY_LEVEL")?,
        })
    }
}

impl MovementDefines {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            embark_distance: defines.fixed5("NMilitary", "AMPHIBIOUS_INVADE_MOVEMENT_COST")?,
            transport_scale: defines.fixed5("NNavy", "NAVAL_SPEED_MODIFIER")?,
            invasion_speed: defines.fixed5("NNavy", "AMPHIBIOUS_INVADE_SPEED_BASE")?,
            outer_scale: defines.fixed5("NMilitary", "LAND_SPEED_MODIFIER")?,
            river_crossing_two: defines.fixed5("NMilitary", "RIVER_CROSSING_SPEED_PENALTY")?,
            river_crossing_three: defines
                .fixed5("NMilitary", "RIVER_CROSSING_SPEED_PENALTY_LARGE")?,
            organisation: defines
                .fixed5("NMilitary", "LOW_ORG_FOR_ATTACK")
                .and_then(|threshold| {
                    Ok((
                        threshold,
                        defines.fixed5("NMilitary", "ZERO_ORG_MOVEMENT_MODIFIER")?,
                    ))
                })
                .map_err(|error| error.to_string()),
            noncore_supply: ["NON_CORE_SUPPLY_SPEED", "NON_CORE_SUPPLY_AIR_SPEED"].map(|name| {
                defines
                    .fixed5("NMilitary", name)
                    .map_err(|error| error.to_string())
            }),
            combat_movement: defines
                .fixed5("NMilitary", "COMBAT_MOVEMENT_SPEED")
                .map_err(|error| error.to_string()),
            rail: RailMovementDefines::read(defines).map_err(|error| error.to_string()),
            naval: NavalMovementDefines::read(defines).map_err(|error| error.to_string()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edge {
    distance: Option<i64>,
    kind: Option<u8>,
    sea_through: Option<i32>,
    crossing: Crossing,
}

#[derive(Clone, Debug, Default)]
pub struct MovementMap {
    terrain: Vec<Option<ProvinceTerrain>>,
    neighbours: Vec<Vec<u32>>,
    edges: Vec<Vec<Edge>>,
    unavailable: BTreeMap<(u32, u32), String>,
    unavailable_kinds: BTreeMap<(u32, u32), String>,
}

fn context(what: &str) -> impl FnOnce(io::Error) -> io::Error + '_ {
    move |error| io::Error::new(error.kind(), format!("{what}: {error}"))
}

impl MovementMap {
    pub fn read(layers: &ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        Self::read_map(layers, defines, None)
    }

    pub fn read_with_terrain(
        layers: &ResolvedLayers,
        defines: &Defines,
        terrain: &content_reader_key_names::terrain::TerrainDefinitions,
    ) -> io::Result<Self> {
        Self::read_map(layers, defines, Some(terrain))
    }

    fn read_map(
        layers: &ResolvedLayers,
        defines: &Defines,
        terrain: Option<&content_reader_key_names::terrain::TerrainDefinitions>,
    ) -> io::Result<Self> {
        let default_map = content_reader_key_names::terrain::read_default_map(layers)?;
        let file = |key: &str| -> io::Result<(String, content_reader_layers::content::Bytes)> {
            let path = default_map.file(key)?;
            let bytes = layers.read(&path).map_err(context(&path))?;
            Ok((path, bytes))
        };
        let (definitions_path, definitions) = file("definitions")?;
        let (provinces_path, provinces) = file("provinces")?;
        let provinces = records::read(&definitions, &provinces).map_err(context(&format!(
            "{definitions_path} with {provinces_path}"
        )))?;
        let database = match terrain {
            Some(definitions) => TerrainDatabase::from_definitions(definitions),
            None => TerrainDatabase::read(layers)?,
        };
        let (terrain_path, terrain) = file("terrain")?;
        let terrain = NativeBitmap::decode(&terrain).map_err(context(&terrain_path))?;
        let (rivers_path, rivers) = file("rivers")?;
        let rivers = Rivers::read(&rivers, defines, &provinces).map_err(context(&rivers_path))?;
        let authored = adjacency::read_authored(layers, &default_map)?;
        Self::derive(
            &provinces,
            &database,
            &terrain,
            &rivers,
            authored.as_deref().unwrap_or_default(),
        )
        .map_err(context("movement map"))
    }

    fn derive(
        provinces: &Provinces,
        database: &TerrainDatabase<'_>,
        terrain_bitmap: &NativeBitmap,
        rivers: &Rivers,
        authored: &[u8],
    ) -> io::Result<Self> {
        let (width, height) = (provinces.width, provinces.height);
        let mut lists = adjacency::raster(width, height, &provinces.ids, provinces.count)?;
        let terrain = province_terrain(provinces, &lists, database);
        let mut kinds = HashMap::new();
        adjacency::append_authored(&mut lists, &mut kinds, authored)?;
        let raster = Raster {
            width,
            height,
            ids: &provinces.ids,
        };
        let anchors = geometry::anchors(&raster, provinces.count);
        let land = |id: u32| provinces.kind(id) == Kind::Land;
        let crossings = geometry::crossings(&raster, &lists, &anchors, land)?;
        type PairInputs = (Result<i64, String>, Result<u8, String>);
        let mut pairs: HashMap<(u32, u32), PairInputs> = HashMap::new();
        for (source, targets) in lists.iter().enumerate() {
            for &target in targets {
                let pair = (target.min(source as u32), target.max(source as u32));
                if pairs.contains_key(&pair) {
                    continue;
                }
                let authored_kind = kinds.get(&pair).map_or(0, |&(kind, _)| kind);
                let points = geometry::route_points(
                    anchors[pair.0 as usize],
                    anchors[pair.1 as usize],
                    crossings[&pair].point,
                    width,
                    height,
                )
                .map_err(|error| error.to_string());
                let kind = if authored_kind == 0 && land(pair.0) && land(pair.1) {
                    points.as_ref().map_err(Clone::clone).and_then(|points| {
                        rivers.first_crossing(points).map(|kind| kind.unwrap_or(0))
                    })
                } else {
                    Ok(authored_kind)
                };
                let distance = points
                    .and_then(|points| native_distance(&points, width, terrain_bitmap, database))
                    .and_then(|distance| kind.as_ref().map(|_| distance).map_err(Clone::clone));
                pairs.insert(pair, (distance, kind));
            }
        }
        let unavailable = pairs
            .iter()
            .filter_map(|(&pair, (distance, _))| Some((pair, distance.as_ref().err()?.clone())))
            .collect();
        let unavailable_kinds = pairs
            .iter()
            .filter_map(|(&pair, (_, kind))| Some((pair, kind.as_ref().err()?.clone())))
            .collect();
        let edges = lists
            .iter()
            .enumerate()
            .map(|(source, targets)| {
                targets
                    .iter()
                    .map(|&target| {
                        let pair = (target.min(source as u32), target.max(source as u32));
                        let (distance, kind) = &pairs[&pair];
                        Edge {
                            distance: distance.as_ref().ok().copied(),
                            kind: kind.as_ref().ok().copied(),
                            sea_through: kinds.get(&pair).and_then(|&(kind, through)| {
                                (kind == crate::adjacencies::SEA_CROSSING).then_some(through)
                            }),
                            crossing: crossings[&pair],
                        }
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            terrain,
            neighbours: lists,
            edges,
            unavailable,
            unavailable_kinds,
        })
    }

    fn known(&self, province: u32) -> bool {
        self.neighbours(province).is_some()
    }

    fn edge(&self, from: u32, to: u32) -> Option<&Edge> {
        let at = self
            .neighbours
            .get(from as usize)?
            .iter()
            .position(|&target| target == to)?;
        self.edges.get(from as usize)?.get(at)
    }

    pub fn terrain(&self, province: u32) -> Option<ProvinceTerrain> {
        self.terrain.get(province as usize).copied().flatten()
    }

    pub fn neighbours(&self, province: u32) -> Option<&[u32]> {
        self.neighbours
            .get(province as usize)
            .filter(|_| province != 0)
            .map(Vec::as_slice)
    }

    pub fn distance(&self, from: u32, to: u32) -> Option<i64> {
        if !self.known(from) || !self.known(to) {
            return None;
        }
        self.edge(from, to)
            .map_or(Some(100_000), |edge| edge.distance)
    }

    pub fn crossing_kind(&self, from: u32, to: u32) -> Option<u8> {
        self.distance(from, to)?;
        self.edge(from, to).map_or(Some(0), |edge| edge.kind)
    }

    pub fn neighbour_kind(&self, from: u32, to: u32) -> Result<Option<u8>, String> {
        for province in [from, to] {
            if !self.known(province) {
                return Err(format!(
                    "province {province} has no native neighbour domain"
                ));
            }
        }
        let Some(edge) = self.edge(from, to) else {
            return Ok(None);
        };
        edge.kind.map(Some).ok_or_else(|| {
            let pair = (from.min(to), from.max(to));
            let reason = self
                .unavailable_kinds
                .get(&pair)
                .map_or("no kind refusal reason recorded", String::as_str);
            format!("province neighbour {from}->{to} crossing kind is unavailable: {reason}")
        })
    }

    pub fn front_border_target(&self, from: u32, to: u32) -> Option<u32> {
        let edge = self.edge(from, to)?;
        let Some(through) = edge.sea_through else {
            return Some(to);
        };
        let target = through as u32;
        self.edge(from, target).map(|_| target)
    }

    pub fn route_crossing(&self, from: u32, to: u32) -> Option<Option<[i32; 2]>> {
        let crossing = self.edge(from, to)?.crossing;
        Some((!crossing.no_midpoint && crossing.point[0] != -1).then_some(crossing.point))
    }

    pub fn neighbour_crossing(&self, from: u32, to: u32) -> Option<[i32; 2]> {
        Some(self.edge(from, to)?.crossing.point)
    }

    pub fn unavailable_edges(&self) -> &BTreeMap<(u32, u32), String> {
        &self.unavailable
    }

    pub fn records(&self) -> usize {
        self.neighbours.len()
    }

    pub fn neighbour_records(
        &self,
        province: u32,
    ) -> Option<impl ExactSizeIterator<Item = NeighbourRecord> + '_> {
        let index = province as usize;
        self.known(province).then(|| {
            self.neighbours[index]
                .iter()
                .zip(&self.edges[index])
                .map(|(&to, edge)| NeighbourRecord {
                    to,
                    distance: edge.distance,
                    kind: edge.kind,
                    no_midpoint: edge.crossing.no_midpoint,
                    crossing: edge.crossing.point,
                })
        })
    }

    #[cfg(feature = "test-support")]
    pub fn from_parts<T: Copy + Into<Option<ProvinceTerrain>>>(
        provinces: &[(u32, T, Vec<Neighbour>)],
    ) -> Self {
        let size = provinces
            .iter()
            .map(|(id, ..)| *id as usize + 1)
            .max()
            .unwrap_or(1);
        let mut map = Self {
            terrain: vec![None; size],
            neighbours: vec![Vec::new(); size],
            edges: vec![Vec::new(); size],
            unavailable: BTreeMap::new(),
            unavailable_kinds: BTreeMap::new(),
        };
        for (id, terrain, neighbours) in provinces {
            let id = *id as usize;
            map.terrain[id] = (*terrain).into();
            map.neighbours[id] = neighbours.iter().map(|n| n.to).collect();
            map.edges[id] = neighbours
                .iter()
                .map(|n| Edge {
                    distance: n.distance,
                    kind: Some(n.kind),
                    sea_through: (n.kind == crate::adjacencies::SEA_CROSSING).then_some(0),
                    crossing: Crossing {
                        point: n.midpoint.unwrap_or([-1, 0]),
                        no_midpoint: n.midpoint.is_none(),
                    },
                })
                .collect();
        }
        map
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeighbourRecord {
    pub to: u32,
    pub distance: Option<i64>,
    pub kind: Option<u8>,
    pub no_midpoint: bool,
    pub crossing: [i32; 2],
}

#[cfg(feature = "test-support")]
#[derive(Clone, Copy, Debug, Default)]
pub struct Neighbour {
    pub to: u32,
    pub distance: Option<i64>,
    pub kind: u8,
    pub midpoint: Option<[i32; 2]>,
}

fn province_terrain(
    provinces: &Provinces,
    raster_lists: &[Vec<u32>],
    database: &TerrainDatabase<'_>,
) -> Vec<Option<ProvinceTerrain>> {
    let mut terrain: Vec<Option<ProvinceTerrain>> = provinces
        .rows
        .iter()
        .map(|row| {
            let row = row.as_ref()?;
            let flags = match (row.kind, row.coastal) {
                (Kind::Land, _) => 1 | 4,
                (Kind::Lake, coastal) => 2 | if coastal { 8 } else { 0 },
                (Kind::Sea, coastal) => {
                    if coastal {
                        16
                    } else {
                        0
                    }
                }
            };
            Some(ProvinceTerrain {
                flags,
                index: database.ordinal(&row.terrain),
            })
        })
        .collect();
    let mut update = |id: u32, change: fn(&mut u8)| {
        if let Some(Some(entry)) = terrain.get_mut(id as usize) {
            change(&mut entry.flags);
        }
    };
    for (a, targets) in raster_lists.iter().enumerate() {
        let a = a as u32;
        for &b in targets.iter().filter(|&&b| a < b) {
            match (provinces.kind(a), provinces.kind(b)) {
                (Kind::Land, Kind::Land) => {
                    update(a, |flags| *flags &= !4);
                    update(b, |flags| *flags &= !4);
                }
                (Kind::Land, Kind::Sea) => update(a, |flags| *flags |= 8),
                (Kind::Sea, Kind::Land) => update(b, |flags| *flags |= 8),
                _ => {}
            }
        }
    }
    terrain
}

fn native_distance_scale(sum: i64) -> i64 {
    if (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) <= 0xffff_ffff_ffff_fffc {
        sum.wrapping_mul(100_000) / 300_000
    } else {
        0xffff_ffff
    }
}

fn native_distance(
    points: &[[i32; 2]],
    width: usize,
    bitmap: &NativeBitmap,
    database: &TerrainDatabase<'_>,
) -> Result<i64, String> {
    let width = width as i32;
    let mut sum = 0_i64;
    for &[x, y] in points {
        let x = x.wrapping_add(width) % width;
        if x < 0 || y < 0 {
            return Err("terrain pixel is outside the written bitmap".into());
        }
        let index = if x as usize >= bitmap.width || y as usize >= bitmap.height {
            0
        } else {
            bitmap
                .byte(x as usize, y as usize)
                .map_err(|error| error.to_string())?
        };
        sum = sum.wrapping_add(database.cost(index));
    }
    Ok(native_distance_scale(sum))
}

struct Rivers {
    bitmap: NativeBitmap,
    small_start: i32,
    small_stop: i32,
    large_stop: i32,
}

impl Rivers {
    fn read(bytes: &[u8], defines: &Defines, provinces: &Provinces) -> io::Result<Self> {
        let bitmap = NativeBitmap::decode(bytes)?;
        if bitmap.bits != 8 {
            return Err(invalid(format!(
                "a {}-bit river bitmap is unsupported; river kinds are 8-bit palette indices",
                bitmap.bits
            )));
        }
        if (bitmap.width, bitmap.height) != (provinces.width, provinces.height) {
            return Err(invalid(format!(
                "the {}x{} river bitmap does not match the {}x{} province bitmap",
                bitmap.width, bitmap.height, provinces.width, provinces.height
            )));
        }
        Ok(Self {
            bitmap,
            small_start: defines.int32("NMilitary", "RIVER_SMALL_START_INDEX")?,
            small_stop: defines.int32("NMilitary", "RIVER_SMALL_STOP_INDEX")?,
            large_stop: defines.int32("NMilitary", "RIVER_LARGE_STOP_INDEX")?,
        })
    }

    fn first_crossing(&self, points: &[[i32; 2]]) -> Result<Option<u8>, String> {
        let (width, height) = (self.bitmap.width as i32, self.bitmap.height as i32);
        for &[x, y] in points {
            let x = x.wrapping_add(width) % width;
            if !(0..height).contains(&y) || x < 0 {
                return Err("river movement point is outside the map".into());
            }
            let index = i32::from(
                self.bitmap
                    .byte(x as usize, y as usize)
                    .map_err(|error| error.to_string())?,
            );
            if (self.small_start..=self.large_stop).contains(&index) {
                return Ok(Some(2 + u8::from(index > self.small_stop)));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::bitmap::test_bmp;
    use super::*;

    #[test]
    fn refused_rail_defines_preserve_common_movement_content() {
        content_reader_script::test_game::enter();
        let defines = Defines::new().unwrap();
        defines.load("fixture.lua", b"NDefines = { NMilitary = { AMPHIBIOUS_INVADE_MOVEMENT_COST = 4, LAND_SPEED_MODIFIER = 1, RIVER_CROSSING_SPEED_PENALTY = -0.1, RIVER_CROSSING_SPEED_PENALTY_LARGE = -0.2 }, NNavy = { NAVAL_SPEED_MODIFIER = 1, AMPHIBIOUS_INVADE_SPEED_BASE = 2 } }").unwrap();
        let movement = MovementDefines::read(&defines).unwrap();
        assert_eq!(movement.embark_distance, 400_000);
        assert_eq!(movement.outer_scale, 100_000);
        assert!(
            movement
                .naval
                .unwrap_err()
                .contains("NAVAL_RANGE_TO_INGAME_DISTANCE")
        );
        assert!(
            movement
                .rail
                .unwrap_err()
                .contains("STRATEGIC_SPEED_INFRA_BASE")
        );
        assert!(
            movement
                .organisation
                .unwrap_err()
                .contains("LOW_ORG_FOR_ATTACK")
        );
        defines
            .load(
                "organisation.lua",
                b"NDefines.NMilitary.LOW_ORG_FOR_ATTACK = 0.35 NDefines.NMilitary.ZERO_ORG_MOVEMENT_MODIFIER = -0.7",
            )
            .unwrap();
        assert_eq!(
            MovementDefines::read(&defines).unwrap().organisation,
            Ok((35_000, -70_000))
        );
    }

    #[test]
    fn noncore_supply_and_combat_movement_defines_load_by_name() {
        content_reader_script::test_game::enter();
        let defines = Defines::new().unwrap();
        defines.load("fixture.lua", b"NDefines = { NMilitary = { AMPHIBIOUS_INVADE_MOVEMENT_COST = 4, LAND_SPEED_MODIFIER = 1, RIVER_CROSSING_SPEED_PENALTY = -0.1, RIVER_CROSSING_SPEED_PENALTY_LARGE = -0.2, NON_CORE_SUPPLY_SPEED = -0.45 }, NNavy = { NAVAL_SPEED_MODIFIER = 1, AMPHIBIOUS_INVADE_SPEED_BASE = 2 } }").unwrap();
        let movement = MovementDefines::read(&defines).unwrap();
        assert_eq!(movement.outer_scale, 100_000);
        assert_eq!(movement.noncore_supply[0], Ok(-45_000));
        assert!(
            movement.noncore_supply[1]
                .as_ref()
                .unwrap_err()
                .contains("NON_CORE_SUPPLY_AIR_SPEED")
        );
        assert!(
            movement
                .combat_movement
                .unwrap_err()
                .contains("COMBAT_MOVEMENT_SPEED")
        );
        defines
            .load(
                "noncore.lua",
                b"NDefines.NMilitary.NON_CORE_SUPPLY_AIR_SPEED = -0.35 NDefines.NMilitary.COMBAT_MOVEMENT_SPEED = 0.37",
            )
            .unwrap();
        let movement = MovementDefines::read(&defines).unwrap();
        assert_eq!(movement.noncore_supply, [Ok(-45_000), Ok(-35_000)]);
        assert_eq!(movement.combat_movement, Ok(37_000));
        let air_only = Defines::new().unwrap();
        air_only.load("air-only.lua", b"NDefines = { NMilitary = { AMPHIBIOUS_INVADE_MOVEMENT_COST = 4, LAND_SPEED_MODIFIER = 1, RIVER_CROSSING_SPEED_PENALTY = -0.1, RIVER_CROSSING_SPEED_PENALTY_LARGE = -0.2, NON_CORE_SUPPLY_AIR_SPEED = -0.35 }, NNavy = { NAVAL_SPEED_MODIFIER = 1, AMPHIBIOUS_INVADE_SPEED_BASE = 2 } }").unwrap();
        let movement = MovementDefines::read(&air_only).unwrap();
        assert!(
            movement.noncore_supply[0]
                .as_ref()
                .unwrap_err()
                .contains("NON_CORE_SUPPLY_SPEED")
        );
        assert_eq!(movement.noncore_supply[1], Ok(-35_000));
    }

    #[test]
    fn naval_range_defines_use_recorded_types_and_preserve_signed_scale() {
        content_reader_script::test_game::enter();
        let defines = Defines::new().unwrap();
        defines.load("fixture.lua", b"NDefines = { NNavy = { NAVAL_RANGE_TO_INGAME_DISTANCE = -2 }, NCountry = { NAVY_USE_HOME_BASE_FOR_RANGE = false } }").unwrap();
        assert_eq!(
            NavalMovementDefines::read(&defines).unwrap(),
            NavalMovementDefines {
                range_scale: -200_000,
                use_home_base: Ok(false),
            }
        );
        defines
            .load(
                "fixture.lua",
                b"NDefines.NCountry.NAVY_USE_HOME_BASE_FOR_RANGE = 1",
            )
            .unwrap();
        assert!(
            NavalMovementDefines::read(&defines)
                .unwrap()
                .use_home_base
                .unwrap_err()
                .contains("is not a boolean")
        );
    }

    fn edge(distance: Option<i64>, kind: u8, point: [i32; 2], no_midpoint: bool) -> Edge {
        Edge {
            distance,
            kind: Some(kind),
            sea_through: (kind == crate::adjacencies::SEA_CROSSING).then_some(0),
            crossing: Crossing { point, no_midpoint },
        }
    }

    #[test]
    fn a_missing_neighbour_uses_the_native_fallback_but_an_unknown_province_does_not() {
        let mut map = MovementMap {
            neighbours: vec![vec![], vec![2], vec![1], vec![]],
            edges: vec![
                vec![],
                vec![edge(Some(321_000), 3, [7, 8], false)],
                vec![edge(Some(321_000), 3, [7, 8], true)],
                vec![],
            ],
            ..Default::default()
        };
        assert_eq!(map.distance(1, 2), Some(321_000));
        assert_eq!(map.distance(1, 3), Some(100_000));
        assert_eq!(map.distance(3, 1), Some(100_000));
        assert_eq!(map.distance(4, 1), None);
        assert_eq!(map.distance(1, 4), None);
        assert_eq!(map.distance(0, 1), None);
        assert_eq!(map.crossing_kind(1, 2), Some(3));
        assert_eq!(map.crossing_kind(1, 3), Some(0));
        assert_eq!(map.crossing_kind(1, 4), None);
        assert_eq!(map.route_crossing(1, 2), Some(Some([7, 8])));
        assert_eq!(map.route_crossing(2, 1), Some(None));
        assert_eq!(map.route_crossing(1, 3), None);
        assert_eq!(map.neighbours(1), Some(&[2][..]));
        assert_eq!(map.neighbours(3), Some(&[][..]));
        assert_eq!(map.neighbours(4), None);
        map.edges[1][0].crossing.point[0] = -1;
        assert_eq!(map.route_crossing(1, 2), Some(None));
    }

    #[test]
    fn an_out_of_range_province_is_unknown_to_every_accessor() {
        let map = MovementMap {
            terrain: vec![None, Some(ProvinceTerrain::default())],
            neighbours: vec![vec![], vec![]],
            edges: vec![vec![], vec![]],
            ..Default::default()
        };
        for far in [2, 9, u32::MAX] {
            assert_eq!(map.terrain(far), None);
            assert_eq!(map.neighbours(far), None);
            assert_eq!(map.distance(far, 1), None);
            assert_eq!(map.distance(1, far), None);
            assert_eq!(map.crossing_kind(far, 1), None);
            assert_eq!(map.route_crossing(far, 1), None);
            assert_eq!(map.route_crossing(1, far), None);
            assert_eq!(map.neighbour_crossing(far, 1), None);
            assert_eq!(map.neighbour_crossing(1, far), None);
        }
        assert_eq!(map.neighbours(0), None);
        assert_eq!(map.route_crossing(0, 1), None);
    }

    #[test]
    fn an_unavailable_neighbour_is_not_the_no_adjacency_fallback() {
        let map = MovementMap {
            neighbours: vec![vec![], vec![2], vec![1], vec![]],
            edges: vec![
                vec![],
                vec![edge(None, 0, [11, 4], false)],
                vec![edge(None, 0, [11, 4], false)],
                vec![],
            ],
            unavailable: BTreeMap::from([((1, 2), "bad terrain cost".into())]),
            ..Default::default()
        };
        assert_eq!(map.distance(1, 2), None);
        assert_eq!(map.distance(1, 3), Some(100_000));
        assert_eq!(map.crossing_kind(1, 2), None);
        assert_eq!(map.route_crossing(1, 2), Some(Some([11, 4])));
        assert_eq!(map.unavailable_edges()[&(1, 2)], "bad terrain cost");
    }

    #[test]
    fn signed_scale_preserves_wrapping_and_the_positive_exceptional_result() {
        assert_eq!(native_distance_scale(700_000), 233_333);
        assert_eq!(native_distance_scale(-700_000), -233_333);
        assert_eq!(native_distance_scale(i64::MIN), 0xffff_ffff);
        assert_eq!(native_distance_scale(i64::MAX), 0xffff_ffff);
    }

    fn rivers(width: i32, rows: &[&[u8]]) -> Rivers {
        Rivers {
            bitmap: NativeBitmap::decode(&test_bmp(width, rows, 8)).unwrap(),
            small_start: 0,
            small_stop: 5,
            large_stop: 9,
        }
    }

    #[test]
    fn river_bytes_use_native_file_rows_and_the_first_qualifying_pixel_wins() {
        let rivers = rivers(4, &[&[3, 6, 9, 255], &[255; 4]]);
        assert_eq!(rivers.first_crossing(&[[0, 0], [1, 0]]), Ok(Some(2)));
        assert_eq!(rivers.first_crossing(&[[1, 0], [0, 0]]), Ok(Some(3)));
        assert_eq!(rivers.first_crossing(&[[0, 1], [1, 1]]), Ok(None));
        assert_eq!(rivers.first_crossing(&[[-3, 0]]), Ok(Some(3)));
        assert!(rivers.first_crossing(&[[0, 2]]).is_err());
    }

    #[test]
    fn padded_river_rows_keep_their_first_width_bytes() {
        let rivers = rivers(3, &[&[255, 255, 255, 3], &[255, 255, 9, 3]]);
        assert_eq!(rivers.first_crossing(&[[0, 1], [1, 1]]), Ok(None));
        assert_eq!(rivers.first_crossing(&[[0, 0], [1, 0], [2, 0]]), Ok(None));
        assert_eq!(rivers.first_crossing(&[[2, 1]]), Ok(Some(3)));
    }

    const TERRAIN: &str = "categories = { unknown = { movement_cost = 1 } plains = { movement_cost = 1 } \
        forest = { movement_cost = 1.5 } ocean = { } lakes = { } } \
        terrain = { p = { type = plains color = { 0 } } f = { type = forest color = { 1 } } }";

    fn synthetic(
        records: &[&str],
        rows: &[&[u32]],
        terrain_rows: &[&[u8]],
        authored: &[u8],
    ) -> MovementMap {
        let mut definitions = String::from(
            "0;0;0;0;land;false;unknown;0
",
        );
        for record in records {
            let (id, rest) = record.split_once(';').unwrap();
            definitions += &format!(
                "{id};{id};0;0;{rest}
"
            );
        }
        let width = rows[0].len();
        let rows: Vec<Vec<u8>> = rows
            .iter()
            .map(|row| {
                let mut bytes: Vec<u8> = row.iter().flat_map(|&id| [0, 0, id as u8]).collect();
                bytes.resize((width * 3).next_multiple_of(4), 0);
                bytes
            })
            .collect();
        let rows: Vec<&[u8]> = rows.iter().map(Vec::as_slice).collect();
        let provinces =
            records::read(definitions.as_bytes(), &test_bmp(width as i32, &rows, 24)).unwrap();
        let river_rows = vec![vec![255_u8; width.next_multiple_of(4)]; provinces.height];
        let river_rows: Vec<&[u8]> = river_rows.iter().map(Vec::as_slice).collect();
        MovementMap::derive(
            &provinces,
            &terrain::database(&[TERRAIN]),
            &NativeBitmap::decode(&test_bmp(4, terrain_rows, 8)).unwrap(),
            &rivers(width as i32, &river_rows),
            authored,
        )
        .unwrap()
    }

    #[test]
    fn distances_sum_terrain_costs_along_the_native_route() {
        content_reader_script::test_game::enter();
        let map = synthetic(
            &["1;land;false;plains", "2;land;false;Forest"],
            &[&[1, 1, 2, 2]],
            &[&[0, 0, 1, 1]],
            b"",
        );
        assert_eq!(map.neighbours(1), Some(&[2][..]));
        assert_eq!(
            map.distance(1, 2),
            Some(native_distance_scale(2 * 100_000 + 150_000))
        );
        assert_eq!(map.crossing_kind(1, 2), Some(0));
        assert_eq!(map.route_crossing(1, 2), Some(None));
        assert!(map.unavailable_edges().is_empty());
        assert_eq!(map.records(), 3);
        assert_eq!(
            map.neighbour_records(1).unwrap().collect::<Vec<_>>(),
            [NeighbourRecord {
                to: 2,
                distance: map.distance(1, 2),
                kind: Some(0),
                no_midpoint: true,
                crossing: map.neighbour_crossing(1, 2).unwrap(),
            }]
        );
        assert!(map.neighbour_records(0).is_none() && map.neighbour_records(3).is_none());
    }

    #[test]
    fn terrain_ordinals_resolve_case_insensitively_and_unknown_names_are_null() {
        content_reader_script::test_game::enter();
        let map = synthetic(
            &["1;land;false;PLAINS", "2;land;false;missing"],
            &[&[1, 2, 3]],
            &[&[0; 4]],
            b"",
        );
        assert_eq!(map.terrain(1).unwrap().index, 2);
        assert_eq!(map.terrain(2).unwrap().index, 0);
        assert_eq!(map.neighbours(3), Some(&[1, 2][..]));
        assert_eq!(map.terrain(3), None);
        assert_eq!(map.terrain(0), None);
    }

    #[test]
    fn kind_coast_and_hidden_bits_come_from_the_csv_and_raster_neighbours() {
        content_reader_script::test_game::enter();
        let map = synthetic(
            &[
                "1;land;false;plains",
                "2;sea;true;ocean",
                "3;lake;true;lakes",
                "4;land;true;plains",
            ],
            &[&[1, 2, 3, 4]],
            &[&[0; 4]],
            b"",
        );
        let flags = |id| map.terrain(id).unwrap().flags;
        assert_eq!(flags(1), 9);
        assert_eq!(flags(2), 16);
        assert_eq!(flags(3), 10);
        assert_eq!(flags(4), 1);

        let lake = synthetic(
            &["1;lake;false;lakes", "2;land;false;plains"],
            &[&[1, 1, 1], &[1, 2, 1], &[1, 1, 1]],
            &[&[0; 4]],
            b"h
2;2;sea;-1;-1;-1;-1;-1;;
",
        );
        assert_eq!(lake.terrain(2).unwrap().flags, 5);
    }

    #[test]
    fn a_four_province_crossing_repairs_the_diagonal_coast() {
        content_reader_script::test_game::enter();
        let map = synthetic(
            &[
                "1;land;false;plains",
                "2;land;false;plains",
                "3;lake;false;lakes",
                "4;sea;false;ocean",
            ],
            &[&[2, 3], &[1, 4]],
            &[&[0; 4]],
            b"",
        );
        assert_eq!(map.terrain(2).unwrap().flags, 9);
        assert!(map.neighbours(2).unwrap().contains(&4));
    }

    #[test]
    fn without_an_authored_file_only_raster_neighbours_remain() {
        content_reader_script::test_game::enter();
        let rows: &[&[u32]] = &[&[1, 2], &[3, 4]];
        let records = [
            "1;land;false;plains",
            "2;sea;false;ocean",
            "3;lake;false;lakes",
            "4;land;false;plains",
            "5;sea;false;ocean",
        ];
        let raster = synthetic(&records, rows, &[&[0; 4]], b"");
        assert_eq!(raster.neighbours(1), Some(&[2, 3, 4][..]));
        assert_eq!(raster.neighbours(5), Some(&[][..]));
        let authored = synthetic(
            &records,
            rows,
            &[&[0; 4]],
            b"h
1;5;sea;-1;-1;-1;-1;-1;;
",
        );
        assert_eq!(authored.neighbours(1), Some(&[2, 3, 4, 5][..]));
        assert_eq!(authored.crossing_kind(5, 1), Some(1));
        assert!(authored.distance(1, 5).is_some());
    }

    #[test]
    fn front_border_targets_use_signed_authored_through_and_last_row_updates() {
        content_reader_script::test_game::enter();
        let records = [
            "1;land;false;plains",
            "2;land;false;plains",
            "3;sea;false;ocean",
            "4;land;false;plains",
        ];
        for (rows, through, forward, reverse) in [
            ("1;2;sea;3;-1;-1;-1;-1;;\n", Some(3), Some(3), Some(3)),
            ("1;2;sea;-1;-1;-1;-1;-1;;\n", Some(-1), None, None),
            (
                "1;2;sea;-2147483648;-1;-1;-1;-1;;\n",
                Some(i32::MIN),
                None,
                None,
            ),
            (
                "1;2;sea;2147483647;-1;-1;-1;-1;;\n",
                Some(i32::MAX),
                None,
                None,
            ),
            ("1;2;sea;99;-1;-1;-1;-1;;\n", Some(99), None, None),
            (
                "1;2;sea;3;-1;-1;-1;-1;;\n1;2;sea;4;-1;-1;-1;-1;;\n",
                Some(4),
                Some(4),
                Some(4),
            ),
            (
                "1;2;sea;3;-1;-1;-1;-1;;\n1;2;river;-1;-1;-1;-1;-1;;\n",
                None,
                Some(2),
                Some(1),
            ),
        ] {
            let authored = format!("h\n{rows}");
            let map = synthetic(&records, &[&[1, 3, 2, 4]], &[&[0; 4]], authored.as_bytes());
            assert_eq!(map.edge(1, 2).unwrap().sea_through, through, "{rows}");
            assert_eq!(map.edge(2, 1).unwrap().sea_through, through, "{rows}");
            assert_eq!(map.front_border_target(1, 2), forward, "{rows}");
            assert_eq!(map.front_border_target(2, 1), reverse, "{rows}");
            assert_eq!(map.front_border_target(1, 4), Some(4));
            assert_eq!(map.front_border_target(3, 4), None);
            assert_eq!(map.front_border_target(0, 1), None);
        }
    }

    #[test]
    fn front_border_redirect_is_one_hop_and_does_not_demand_movement_distance() {
        content_reader_script::test_game::enter();
        let mut map = synthetic(
            &[
                "1;land;false;plains",
                "2;land;false;plains",
                "3;sea;false;ocean",
                "4;land;false;plains",
            ],
            &[&[1, 3, 2, 4]],
            &[&[0; 4]],
            b"h\n1;2;sea;3;-1;-1;-1;-1;;\n1;3;sea;4;-1;-1;-1;-1;;\n",
        );
        assert_eq!(map.front_border_target(1, 2), Some(3));
        assert_eq!(map.front_border_target(1, 3), Some(4));
        let at = map.neighbours[1].iter().position(|&to| to == 2).unwrap();
        map.edges[1][at].distance = None;
        assert_eq!(map.crossing_kind(1, 2), None);
        assert_eq!(map.front_border_target(1, 2), Some(3));
    }

    fn crossing_fixture(terrain: &NativeBitmap, rivers: &Rivers, authored: &[u8]) -> MovementMap {
        let definitions = b"0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;0\n2;2;0;0;land;false;plains;0\n3;3;0;0;land;false;plains;0\n";
        let provinces = records::read(
            definitions,
            &test_bmp(4, &[&[0, 0, 1, 0, 0, 1, 0, 0, 2, 0, 0, 2]], 24),
        )
        .unwrap();
        MovementMap::derive(
            &provinces,
            &terrain::database(&[TERRAIN]),
            terrain,
            rivers,
            authored,
        )
        .unwrap()
    }

    #[test]
    fn selected_river_kind_samples_the_existing_raster_and_first_qualifying_pixel() {
        content_reader_script::test_game::enter();
        let terrain = NativeBitmap::decode(&test_bmp(4, &[&[0; 4]], 8)).unwrap();
        for (pixels, kind) in [([255, 3, 7, 255], 2), ([255, 7, 3, 255], 3), ([255; 4], 0)] {
            let map = crossing_fixture(&terrain, &rivers(4, &[&pixels]), b"");
            assert_eq!(map.neighbour_kind(1, 2), Ok(Some(kind)));
            assert_eq!(map.neighbour_kind(2, 1), Ok(Some(kind)));
            assert_eq!(map.crossing_kind(1, 2), Some(kind));
            assert_eq!(
                map.neighbour_records(1).unwrap().next().unwrap().kind,
                Some(kind)
            );
        }
    }

    #[test]
    fn selected_authored_kind_suppresses_raster_river_sampling() {
        content_reader_script::test_game::enter();
        let terrain = NativeBitmap::decode(&test_bmp(4, &[&[0; 4]], 8)).unwrap();
        let river = rivers(4, &[&[3; 4]]);
        let map = crossing_fixture(&terrain, &river, b"h\n1;2;sea;-1;-1;-1;-1;-1;;\n");
        assert_eq!(map.neighbour_kind(1, 2), Ok(Some(1)));
        assert_eq!(map.crossing_kind(1, 2), Some(1));
    }

    #[test]
    fn selected_raster_kind_survives_unavailable_terrain_distance() {
        content_reader_script::test_game::enter();
        let terrain = NativeBitmap::decode(&test_bmp(4, &[&[]], 1)).unwrap();
        let map = crossing_fixture(&terrain, &rivers(4, &[&[255, 3, 255, 255]]), b"");
        assert_eq!(map.distance(1, 2), None);
        assert_eq!(map.crossing_kind(1, 2), None);
        assert_eq!(map.neighbour_kind(1, 2), Ok(Some(2)));
        assert_eq!(
            map.neighbour_records(1).unwrap().next().unwrap().kind,
            Some(2)
        );
        assert!(map.unavailable_edges()[&(1, 2)].contains("pixel reads outside"));
    }

    #[test]
    fn selected_unknown_river_kind_is_not_clear_and_distance_error_priority_is_preserved() {
        content_reader_script::test_game::enter();
        let terrain = NativeBitmap::decode(&test_bmp(4, &[&[]], 1)).unwrap();
        let mut river = rivers(4, &[&[255; 4]]);
        river.bitmap.height = 0;
        let map = crossing_fixture(&terrain, &river, b"");
        assert!(
            map.neighbour_kind(1, 2)
                .unwrap_err()
                .contains("river movement point")
        );
        assert_eq!(map.neighbour_records(1).unwrap().next().unwrap().kind, None);
        assert_eq!(map.distance(1, 2), None);
        assert!(map.unavailable_edges()[&(1, 2)].contains("pixel reads outside"));
        let authored = crossing_fixture(&terrain, &river, b"h\n1;2;sea;-1;-1;-1;-1;-1;;\n");
        assert_eq!(authored.neighbour_kind(1, 2), Ok(Some(1)));
    }

    #[test]
    fn selected_kind_distinguishes_known_absence_from_an_unknown_province_domain() {
        content_reader_script::test_game::enter();
        let terrain = NativeBitmap::decode(&test_bmp(4, &[&[0; 4]], 8)).unwrap();
        let map = crossing_fixture(&terrain, &rivers(4, &[&[255; 4]]), b"");
        assert_eq!(map.neighbour_kind(1, 3), Ok(None));
        assert_eq!(map.neighbour_kind(3, 1), Ok(None));
        for id in [0, 4, u32::MAX] {
            assert!(map.neighbour_kind(id, 1).is_err());
            assert!(map.neighbour_kind(1, id).is_err());
        }
    }

    #[test]
    fn selected_kind_uses_the_first_directional_neighbour_without_distance() {
        content_reader_script::test_game::enter();
        let terrain = NativeBitmap::decode(&test_bmp(4, &[&[0; 4]], 8)).unwrap();
        let mut map = crossing_fixture(&terrain, &rivers(4, &[&[255; 4]]), b"");
        map.edges[1][0].distance = None;
        map.edges[1][0].kind = Some(3);
        map.neighbours[1].push(2);
        map.edges[1].push(edge(Some(100_000), 2, [0, 0], false));
        assert_eq!(map.neighbour_kind(1, 2), Ok(Some(3)));
    }
}
