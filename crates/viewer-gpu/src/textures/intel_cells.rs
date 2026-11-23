
use job_priority::SpawnCarried as _;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};

use super::{FOW_BLOCK, IntelMap, MapData, ProvinceKind, intel_image_size, invalid};
use std::collections::BTreeMap;

const CELL_PIXELS: usize = FOW_BLOCK as usize * FOW_BLOCK as usize;

struct CellProvince {
    land_index: u32,
    pixels: u8,
}

pub struct IntelCells(map_display_data::retained::Retained<IntelComposition>);
impl std::ops::Deref for IntelCells {
    type Target = IntelComposition;
    fn deref(&self) -> &IntelComposition {
        &self.0
    }
}
pub struct IntelComposition {
    offsets: Vec<usize>,
    entries: Vec<CellProvince>,
    land_provinces: Vec<u32>,
    province_cells: BTreeMap<u32, Vec<usize>>,
}

pub struct IntelCellsCache {
    map: Arc<MapData>,
    cells: OnceLock<Result<IntelCells, (io::ErrorKind, String)>>,
    building: Mutex<()>,
}

impl IntelCellsCache {
    pub fn new(map: Arc<MapData>) -> Self {
        Self {
            map,
            cells: OnceLock::new(),
            building: Mutex::new(()),
        }
    }

    pub fn prewarm(map: Arc<MapData>) -> Arc<Self> {
        let cache = Arc::new(Self::new(map));
        let pending = Arc::clone(&cache);
        let _ = std::thread::Builder::new()
            .name("map-intel-cells".into())
            .spawn_carried(move || {
                viewer_runtime::worker_priority::background_priority();
                let _ = pending.get_or_init();
            });
        cache
    }

    pub fn get_or_init(&self) -> io::Result<&IntelCells> {
        self.get_or_init_with(|| IntelCells::new(&self.map))
    }

    fn get_or_init_with(
        &self,
        build: impl FnOnce() -> io::Result<IntelCells>,
    ) -> io::Result<&IntelCells> {
        if self.cells.get().is_none() {
            let _building = self.building.lock().unwrap_or_else(|p| p.into_inner());
            if self.cells.get().is_none() {
                let result = match build() {
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            || engine_allocation::is_allocation_error(&error) =>
                    {
                        return Err(error);
                    }
                    result => result.map_err(|error| (error.kind(), error.to_string())),
                };
                let _ = self.cells.set(result);
            }
        }
        self.cells
            .get()
            .expect("completed cell table")
            .as_ref()
            .map_err(|(kind, error)| io::Error::new(*kind, error.clone()))
    }
}

impl IntelCells {
    pub fn new(map: &MapData) -> io::Result<Self> {
        let [width, height] = intel_image_size(map)?;
        let grid_width = (map.width / FOW_BLOCK) as usize;
        let grid_height = (map.height / FOW_BLOCK) as usize;
        let count = grid_width
            .checked_mul(grid_height)
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| invalid("the intel grid's cell count exceeds addressable storage"))?;
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(count)
            .map_err(|_| engine_allocation::oom())?;
        offsets.push(0);
        let land_count = map
            .definitions
            .values()
            .filter(|definition| definition.kind == ProvinceKind::Land)
            .count();
        let mut land_provinces = Vec::new();
        land_provinces
            .try_reserve_exact(land_count)
            .map_err(|_| engine_allocation::oom())?;
        let mut land_indices = HashMap::new();
        land_indices
            .try_reserve(land_count)
            .map_err(|_| engine_allocation::oom())?;
        for (&province, definition) in &map.definitions {
            if definition.kind == ProvinceKind::Land {
                let index = u32::try_from(land_provinces.len())
                    .map_err(|_| invalid("the intel land-province index exceeds u32"))?;
                land_indices.insert(province, index);
                land_provinces.push(province);
            }
        }
        let mut entries = Vec::new();
        let mut cell = [0u32; CELL_PIXELS];
        for cell_z in 0..grid_height {
            for cell_x in 0..grid_width {
                let mut used = 0;
                for step_z in 0..FOW_BLOCK as usize {
                    let row = height - 1 - (cell_z * FOW_BLOCK as usize + step_z);
                    let start = row * width + cell_x * FOW_BLOCK as usize;
                    for province in &map.province_ids[start..start + FOW_BLOCK as usize] {
                        if let Some(&index) = land_indices.get(province) {
                            cell[used] = index;
                            used += 1;
                        }
                    }
                }
                cell[..used].sort_unstable();
                entries
                    .try_reserve(used)
                    .map_err(|_| engine_allocation::oom())?;
                for run in cell[..used].chunk_by(|left, right| left == right) {
                    entries.push(CellProvince {
                        land_index: run[0],
                        pixels: run.len() as u8,
                    });
                }
                offsets.push(entries.len());
            }
        }
        let mut province_cells: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for cell in 0..offsets.len() - 1 {
            for entry in &entries[offsets[cell]..offsets[cell + 1]] {
                province_cells
                    .entry(land_provinces[entry.land_index as usize])
                    .or_default()
                    .push(cell);
            }
        }
        Ok(Self(map_display_data::retained::Retained::new(
            IntelComposition {
                province_cells,
                offsets,
                entries,
                land_provinces,
            },
        )))
    }

    pub fn cells(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn province_cells(&self, province: u32) -> &[usize] {
        self.province_cells.get(&province).map_or(&[], Vec::as_slice)
    }

    pub fn cell_byte(&self, cell: usize, byte: impl Fn(u32) -> u8) -> u8 {
        let mut sum = CELL_PIXELS as u32 * u32::from(u8::MAX);
        for entry in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
            sum -= u32::from(entry.pixels) * u32::from(u8::MAX - byte(self.land_provinces[entry.land_index as usize]));
        }
        !((sum / CELL_PIXELS as u32) as u8)
    }

    pub fn patch_values(
        &self,
        previous: &IntelMap,
        old: &viewer_foundation::perspective::values::Values<u32, u8>,
        visibility: &viewer_foundation::perspective::values::Values<u32, u8>,
    ) -> io::Result<IntelMap> {
        let mut dirty = map_display_data::dirty::Dirty::<64>::default();
        visibility.changed(old, |province| {
            for &cell in self.province_cells.get(&province).into_iter().flatten() {
                dirty.insert(cell);
            }
        });
        let mut fogged = previous.fogged_texels;
        let edits = dirty.iter().map(|cell| {
            let mut sum = CELL_PIXELS as u32 * u32::from(u8::MAX);
            for entry in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
                let province = self.land_provinces[entry.land_index as usize];
                let byte = visibility.get(&province).copied().unwrap_or(u8::MAX);
                sum -= u32::from(entry.pixels) * u32::from(u8::MAX - byte);
            }
            let byte = !((sum / CELL_PIXELS as u32) as u8);
            fogged -= usize::from(previous.texels[cell] > 0);
            fogged += usize::from(byte > 0);
            (cell, byte)
        });
        let texels = previous.texels.try_edited(edits)?;
        Ok(IntelMap {
            texels,
            fog: true,
            fogged_texels: fogged,
        })
    }
    pub fn intel_values(
        &self,
        visibility: &viewer_foundation::perspective::values::Values<u32, u8>,
    ) -> io::Result<IntelMap> {
        let mut fogged = 0;
        let texels = map_display_data::Paged::try_from_iter(
            self.offsets.len() - 1,
            (0..self.offsets.len() - 1).map(|cell| {
                let mut sum = CELL_PIXELS as u32 * u32::from(u8::MAX);
                for entry in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
                    let byte = visibility
                        .get(&self.land_provinces[entry.land_index as usize])
                        .copied()
                        .unwrap_or(u8::MAX);
                    sum -= u32::from(entry.pixels) * u32::from(u8::MAX - byte);
                }
                let byte = !((sum / CELL_PIXELS as u32) as u8);
                fogged += usize::from(byte > 0);
                byte
            }),
        )?;
        Ok(IntelMap {
            texels,
            fog: true,
            fogged_texels: fogged,
        })
    }
    #[cfg(test)]
    pub fn patch(
        &self,
        previous: &IntelMap,
        old: &BTreeMap<u32, u8>,
        visibility: &BTreeMap<u32, u8>,
    ) -> IntelMap {
        let mut dirty = std::collections::BTreeSet::new();
        for province in old.keys().chain(visibility.keys()) {
            if old.get(province).copied().unwrap_or(u8::MAX)
                != visibility.get(province).copied().unwrap_or(u8::MAX)
            {
                dirty.extend(
                    self.province_cells
                        .get(province)
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        let mut fogged = previous.fogged_texels;
        let edits = dirty
            .into_iter()
            .map(|cell| {
                let mut sum = CELL_PIXELS as u32 * u32::from(u8::MAX);
                for entry in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
                    let province = self.land_provinces[entry.land_index as usize];
                    let byte = visibility.get(&province).copied().unwrap_or(u8::MAX);
                    sum -= u32::from(entry.pixels) * u32::from(u8::MAX - byte);
                }
                let byte = !((sum / CELL_PIXELS as u32) as u8);
                fogged -= usize::from(previous.texels[cell] > 0);
                fogged += usize::from(byte > 0);
                (cell, byte)
            })
            .collect::<Vec<_>>();
        IntelMap {
            texels: previous.texels.edited(edits),
            fog: true,
            fogged_texels: fogged,
        }
    }

    pub fn try_intel_map(&self, visibility: Option<&BTreeMap<u32, u8>>) -> io::Result<IntelMap> {
        let mut fogged_texels = 0;
        let texels = map_display_data::Paged::try_from_iter(
            self.offsets.len() - 1,
            (0..self.offsets.len() - 1).map(|cell| {
                let Some(visibility) = visibility else {
                    return 0;
                };
                let mut sum = CELL_PIXELS as u32 * u32::from(u8::MAX);
                for entry in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
                    let byte = visibility
                        .get(&self.land_provinces[entry.land_index as usize])
                        .copied()
                        .unwrap_or(u8::MAX);
                    sum -= u32::from(entry.pixels) * u32::from(u8::MAX - byte);
                }
                let byte = !((sum / CELL_PIXELS as u32) as u8);
                fogged_texels += usize::from(byte > 0);
                byte
            }),
        )?;
        Ok(IntelMap {
            texels,
            fog: visibility.is_some(),
            fogged_texels,
        })
    }

    #[cfg(test)]
    pub fn intel_map(&self, visibility: Option<&BTreeMap<u32, u8>>) -> IntelMap {
        self.try_intel_map(visibility).unwrap()
    }
}

#[cfg(test)]
mod tests;
