//! Content-owned province multiplicities for the floor-sized fog grid.

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

/// Immutable pixel composition, built once from the content's province image.
/// Only defined land contributes dynamic bytes. Water and unknown IDs contribute
/// 255, and the north/east remainder outside the floor-sized grid is not read.
/// Storage follows the actual grid, land definitions and cell/province pairs.
pub struct IntelCells {
    offsets: Box<[usize]>,
    entries: Box<[CellProvince]>,
    land_provinces: Box<[u32]>,
}

/// One immutable map's cell table, shared by content loading and hour jobs.
/// Loading may prepare it early; a demand before it is ready uses the same
/// serialized construction. Completed tables and structural refusals are retained;
/// allocation/resource failures leave the cell empty so later demand can retry.
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

    /// Preparation never holds up content loading. If the thread cannot start,
    /// or an hour arrives first, the ordinary lazy construction remains exact.
    pub fn prewarm(map: Arc<MapData>) -> Arc<Self> {
        let cache = Arc::new(Self::new(map));
        let pending = Arc::clone(&cache);
        let _ = std::thread::Builder::new()
            .name("map-intel-cells".into())
            .spawn(move || {
                crate::worker_priority::background_priority();
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
            // Serialize preparation and demand without publishing transient resource
            // failures as immutable content facts. A later caller can retry them.
            let _building = self.building.lock().unwrap_or_else(|p| p.into_inner());
            if self.cells.get().is_none() {
                let result = match build() {
                    Err(error) if engine_native::allocation::is_allocation_error(&error) => {
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
            .map_err(|_| engine_native::allocation::oom())?;
        offsets.push(0);
        let land_count = map
            .definitions
            .values()
            .filter(|definition| definition.kind == ProvinceKind::Land)
            .count();
        let mut land_provinces = Vec::new();
        land_provinces
            .try_reserve_exact(land_count)
            .map_err(|_| engine_native::allocation::oom())?;
        let mut land_indices = HashMap::new();
        land_indices
            .try_reserve(land_count)
            .map_err(|_| engine_native::allocation::oom())?;
        for (&province, definition) in &map.definitions {
            if definition.kind == ProvinceKind::Land {
                // Unique u32 province keys admit at most u32::MAX + 1 entries.
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
                    .map_err(|_| engine_native::allocation::oom())?;
                for run in cell[..used].chunk_by(|left, right| left == right) {
                    entries.push(CellProvince {
                        land_index: run[0],
                        pixels: run.len() as u8,
                    });
                }
                offsets.push(entries.len());
            }
        }
        Ok(Self {
            offsets: offsets.into_boxed_slice(),
            entries: entries.into_boxed_slice(),
            land_provinces: land_provinces.into_boxed_slice(),
        })
    }

    /// Rebuild exactly the scalar raster's sum/divide/complement, without
    /// rereading static pixels. Missing visibility is not a recorded zero.
    pub fn intel_map(&self, visibility: Option<&BTreeMap<u32, u8>>) -> IntelMap {
        let mut texels = vec![0; self.offsets.len() - 1];
        let Some(visibility) = visibility else {
            return IntelMap {
                texels,
                fog: false,
                fogged_texels: 0,
            };
        };
        let byte_of: Vec<_> = self
            .land_provinces
            .iter()
            .map(|province| visibility.get(province).copied().unwrap_or(u8::MAX))
            .collect();
        let mut fogged_texels = 0;
        for (cell, out) in texels.iter_mut().enumerate() {
            let mut sum = CELL_PIXELS as u32 * u32::from(u8::MAX);
            for entry in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
                sum -= u32::from(entry.pixels)
                    * u32::from(u8::MAX - byte_of[entry.land_index as usize]);
            }
            *out = !((sum / CELL_PIXELS as u32) as u8);
            fogged_texels += usize::from(*out > 0);
        }
        IntelMap {
            texels,
            fog: true,
            fogged_texels,
        }
    }
}

#[cfg(test)]
mod tests;
