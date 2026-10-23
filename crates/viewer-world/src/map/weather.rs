//! `SnowMudData`: the hour's snow and mud grid the terrain, water and river
//! passes sample, built from the display bytes the game stores.
//!
//! Snow and mud display bytes start at zero after load and change only when a weather pass updates
//! them. Recorded WeatherProvinces display bytes supply each hour, including seeks. Four-by-four
//! pixel blocks average into grid cells with fixed division by sixteen; permanent-snow terrain
//! contributes 255. Retain recorded boundary cells. RGBA packs previous mud, current snow, previous
//! snow and current mud. Presentation keeps both planes and advances the linear graphics-time fade
//! independently of replay hours.
//!

use std::{io, time::Duration, time::Instant};

use installed_content_core::map::MapData;
use replay_data::{rows::Recorded, rows::TableId, rows::WeatherProvinceRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use viewer_foundation::map::{province_map::ProvinceMap, tables::rows, textures::SNOW_MUD_BLOCK};

mod presentation;
pub use presentation::Presentation;
mod render_delta;
pub use render_delta::RenderDelta;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// One hour's `SnowMudData` payload.
pub struct SnowMud {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBA8 texels, row 0 at world `z = 0`.
    pub texels: Vec<u8>,
    /// Recorded `WeatherProvinces.snow` by province id, `None` where the hour
    /// holds no row: the raw word the winter grading reads
    /// (`map::post::Volumes::set_snow`).
    pub province_snow: Vec<Option<i64>>,
    /// Winter factors from this document's immutable post-volume tables, when
    /// prepared by the hour builders. Independent exports can compute them at install.
    pub post_factors: Option<super::post::SnowFactors>,
    /// Wall time of the rasterisation, for reporting.
    pub build: Duration,
    pub summary: Summary,
    pub refresh: Option<Refresh>,
}

impl SnowMud {
    /// Prepares winter grading independently of the view's snow/mud refresh clock.
    pub fn prepare_post(&mut self, volumes: &super::post::Volumes) {
        self.post_factors = Some(volumes.snow_factors(&self.province_snow));
    }
}

/// Build time is report-only; retained winter factors must compare by bits.
impl PartialEq for SnowMud {
    fn eq(&self, other: &Self) -> bool {
        let bits = |factors: &super::post::SnowFactors| {
            factors
                .iter()
                .map(|factor| factor.map(f32::to_bits))
                .collect::<Vec<_>>()
        };
        self.width == other.width
            && self.height == other.height
            && self.refresh == other.refresh
            && self.summary == other.summary
            && self.province_snow == other.province_snow
            && self.post_factors.as_ref().map(bits) == other.post_factors.as_ref().map(bits)
            && self.texels == other.texels
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refresh {
    pub interval: i64,
    pub identity: u64,
}

/// What the hour's weather table held, for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Recorded `WeatherProvinces` rows.
    pub provinces: usize,
    /// Whether the table is whole; missing provinces draw bare ground.
    pub complete: bool,
    /// Rows whose native snow word is above zero.
    pub with_snow: usize,
    /// Rows with a nonzero snow display byte.
    pub shown_snow: usize,
    /// Rows with a nonzero mud display byte.
    pub shown_mud: usize,
    /// Retained native edge cells unavailable in this sample.
    pub missing_edges: usize,
}

impl Summary {
    /// One line for the status bar and the headless dump.
    pub fn line(&self) -> String {
        format!(
            "{} provinces{}, {} with snow, {} drawn snowed, {} drawn muddy{}",
            self.provinces,
            if self.complete { "" } else { " (incomplete)" },
            self.with_snow,
            self.shown_snow,
            self.shown_mud,
            if self.missing_edges == 0 {
                String::new()
            } else {
                format!(
                    ", {} retained snow/mud edge cells unavailable",
                    self.missing_edges
                )
            }
        )
    }
}

/// `MAP_SIZE / 4`: the grid fills for a province image.
pub fn grid_size(width: u32, height: u32) -> [u32; 2] {
    [width / SNOW_MUD_BLOCK, height / SNOW_MUD_BLOCK]
}

/// Every table `grid` reads; the window keeps a grid while the hours share
/// them (`viewer_foundation::map::memo`).
pub const TABLES: &[TableId] = &[TableId::WeatherProvinces];

/// The hour's grid from its recorded display bytes, pixel by pixel: the
/// reference [`grid_from_cells`] is tested against. A truncated or unavailable
/// table draws its missing provinces bare and says so.
#[cfg(test)]
pub fn grid(map: &MapData, perm_snow: &[bool], frame: &DecodedFrame) -> io::Result<SnowMud> {
    let image = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    grid_with(largest_province(map), frame, |bytes| {
        rasterise(&image, perm_snow, bytes)
    })
}

/// Includes defined provinces absent from the province image.
fn largest_province(map: &MapData) -> u32 {
    let drawn = map.province_ids.iter().copied().max().unwrap_or(0);
    map.definitions
        .last_key_value()
        .map_or(drawn, |(&defined, _)| drawn.max(defined))
}

/// The hour's grid from its recorded display bytes, through the map's
/// precomputed cell composition, which reads each cell's few provinces instead
/// of its sixteen pixels. A truncated or unavailable table draws its missing
/// provinces bare and says so.
pub fn grid_from_cells(cells: &SnowCells, frame: &DecodedFrame) -> io::Result<SnowMud> {
    grid_with(cells.largest, frame, |bytes| Ok(cells.average(bytes)))
}

/// Reads the hour's bytes and turns `average`'s cells into the texture.
/// `largest` includes undrawn definitions; a recorded ID beyond it refuses the hour.
fn grid_with(
    largest: u32,
    frame: &DecodedFrame,
    average: impl FnOnce(&[[u8; 2]]) -> io::Result<(u32, u32, Vec<u8>)>,
) -> io::Result<SnowMud> {
    let view = rows::<tables::WeatherProvinces>(frame)?;
    let recorded = view.rows();
    if let Some(row) = recorded.iter().find(|row| row.id > largest) {
        return Err(invalid(format!(
            "Snow/mud presentation: 1 refused weather province {} beyond the map's largest \
             province {largest}",
            row.id
        )));
    }
    let largest = largest as usize;
    let mut bytes = vec![[0_u8; 2]; largest + 1];
    let mut province_snow = vec![None; largest + 1];
    let mut summary = Summary {
        provinces: recorded.len(),
        complete: view.is_complete(),
        ..Summary::default()
    };
    for &WeatherProvinceRow {
        id,
        snow,
        display_snow,
        display_mud,
        ..
    } in recorded
    {
        bytes[id as usize] = [display_snow, display_mud];
        province_snow[id as usize] = Some(snow);
        summary.with_snow += usize::from(snow > 0);
        summary.shown_snow += usize::from(display_snow != 0);
        summary.shown_mud += usize::from(display_mud != 0);
    }
    let start = Instant::now();
    let (width, height, mut cells) = average(&bytes)?;
    let edge_cells = (width as usize)
        .checked_add(height as usize)
        .and_then(|count| count.checked_sub(1))
        .filter(|_| width > 0 && height > 0)
        .ok_or_else(|| invalid("Snow/mud presentation: 1 refused empty grid"))?;
    let display = recorded
        .iter()
        .find(|row| row.id == 0)
        .and_then(|row| match &row.display {
            Recorded::Value(Some(display)) => Some(display),
            _ => None,
        });
    let refresh = if let Some(display) = display {
        if [display.width, display.height] != [width, height]
            || display.edges.len() != edge_cells * 2
        {
            return Err(invalid(
                "Snow/mud presentation: 1 refused retained boundary size mismatch",
            ));
        }
        Some(Refresh {
            interval: display.refresh_interval,
            identity: display.refresh_identity,
        })
    } else {
        summary.missing_edges = edge_cells;
        None
    };
    // Boundary cells retain recorded edges. Missing history is unknown, rather than evidence that
    // an edge follows this hour's per-province simulation.
    let boundary = (0..height - 1)
        .map(|z| z * width + width - 1)
        .chain((0..width).map(|x| (height - 1) * width + x));
    for (edge, cell) in boundary.enumerate() {
        cells[cell as usize * 2..cell as usize * 2 + 2].copy_from_slice(
            &display.map_or([0; 2], |d| [d.edges[edge], d.edges[edge_cells + edge]]),
        );
    }
    let texels = cells
        .chunks_exact(2)
        .flat_map(|cell| [cell[1], cell[0], cell[0], cell[1]])
        .collect();
    Ok(SnowMud {
        width,
        height,
        texels,
        province_snow,
        post_factors: None,
        build: start.elapsed(),
        summary,
        refresh,
    })
}

/// Which provinces each grid cell's sixteen pixels read, so an hour's grid
/// costs one pass over the cells rather than over every pixel.
///
/// A cell's sum is the same integer `rasterise` adds pixel by pixel: every
/// `perm_snow` pixel counts 255 snow, every other pixel its province's snow
/// byte, and every pixel its province's mud byte.
pub struct SnowCells {
    /// The map's largest province id ([`largest_province`]).
    largest: u32,
    width: u32,
    height: u32,
    /// Each cell's entries: `offsets[cell]..offsets[cell + 1]`.
    offsets: Vec<u32>,
    /// A province and how many of the cell's pixels read its snow and mud.
    entries: Vec<(u32, u8, u8)>,
    /// How many of the cell's pixels are `perm_snow`.
    perm: Vec<u8>,
}

impl SnowCells {
    pub fn new(map: &MapData, perm_snow: &[bool]) -> io::Result<Self> {
        let image = ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        };
        image.validate()?;
        if perm_snow.len() != image.ids.len() {
            return Err(invalid(format!(
                "the perm_snow mask holds {} flags for {} province pixels",
                perm_snow.len(),
                image.ids.len()
            )));
        }
        let [width, height] = grid_size(image.width, image.height);
        let count = width as usize * height as usize;
        let mut offsets = Vec::with_capacity(count + 1);
        let mut entries: Vec<(u32, u8, u8)> = Vec::with_capacity(count * 2);
        let mut perm = Vec::with_capacity(count);
        offsets.push(0);
        for cell_z in 0..height {
            for cell_x in 0..width {
                let start = entries.len();
                let mut perm_pixels = 0u8;
                for step_z in 0..SNOW_MUD_BLOCK {
                    let world_z = cell_z * SNOW_MUD_BLOCK + step_z;
                    for step_x in 0..SNOW_MUD_BLOCK {
                        let column = cell_x * SNOW_MUD_BLOCK + step_x;
                        let row = image.height - 1 - world_z;
                        let at = (row * image.width + column) as usize;
                        let province = image.ids[at];
                        let snow = u8::from(!perm_snow[at]);
                        perm_pixels += u8::from(perm_snow[at]);
                        match entries[start..]
                            .iter_mut()
                            .find(|(id, _, _)| *id == province)
                        {
                            Some(entry) => {
                                entry.1 += snow;
                                entry.2 += 1;
                            }
                            None => entries.push((province, snow, 1)),
                        }
                    }
                }
                perm.push(perm_pixels);
                offsets.push(entries.len() as u32);
            }
        }
        Ok(Self {
            largest: largest_province(map),
            width,
            height,
            offsets,
            entries,
            perm,
        })
    }

    /// `rasterise`'s cells for these per-province bytes.
    fn average(&self, bytes: &[[u8; 2]]) -> (u32, u32, Vec<u8>) {
        let area = SNOW_MUD_BLOCK * SNOW_MUD_BLOCK;
        let mut cells = vec![0u8; self.perm.len() * 2];
        for (cell, out) in cells.chunks_exact_mut(2).enumerate() {
            let mut snow = u32::from(self.perm[cell]) * u32::from(u8::MAX);
            let mut mud = 0u32;
            let range = self.offsets[cell] as usize..self.offsets[cell + 1] as usize;
            for &(province, snow_pixels, pixels) in &self.entries[range] {
                let [province_snow, province_mud] =
                    bytes.get(province as usize).copied().unwrap_or([0; 2]);
                snow += u32::from(snow_pixels) * u32::from(province_snow);
                mud += u32::from(pixels) * u32::from(province_mud);
            }
            out.copy_from_slice(&[(snow / area) as u8, (mud / area) as u8]);
        }
        (self.width, self.height, cells)
    }
}

/// Average sixteen per-province display bytes per cell with integer division. Pixels outside the
/// image use province zero while still contributing to the count. Output row zero is world z zero,
/// the image's last row. Each cell stores snow and mud.
#[cfg(test)]
fn rasterise(
    image: &ProvinceMap<'_>,
    perm_snow: &[bool],
    bytes: &[[u8; 2]],
) -> io::Result<(u32, u32, Vec<u8>)> {
    image.validate()?;
    if perm_snow.len() != image.ids.len() {
        return Err(invalid(format!(
            "the perm_snow mask holds {} flags for {} province pixels",
            perm_snow.len(),
            image.ids.len()
        )));
    }
    let [grid_width, grid_height] = grid_size(image.width, image.height);
    let area = SNOW_MUD_BLOCK * SNOW_MUD_BLOCK;
    let mut cells = vec![0u8; grid_width as usize * grid_height as usize * 2];
    for cell_z in 0..grid_height {
        for cell_x in 0..grid_width {
            let (mut snow, mut mud) = (0u32, 0u32);
            for step_z in 0..SNOW_MUD_BLOCK {
                let world_z = cell_z * SNOW_MUD_BLOCK + step_z;
                for step_x in 0..SNOW_MUD_BLOCK {
                    let column = cell_x * SNOW_MUD_BLOCK + step_x;
                    let row = image.height - 1 - world_z;
                    let at = (row * image.width + column) as usize;
                    let [province_snow, province_mud] =
                        bytes.get(image.ids[at] as usize).copied().unwrap_or([0; 2]);
                    snow += u32::from(if perm_snow[at] {
                        u8::MAX
                    } else {
                        province_snow
                    });
                    mud += u32::from(province_mud);
                }
            }
            let at = (cell_z as usize * grid_width as usize + cell_x as usize) * 2;
            cells[at..at + 2].copy_from_slice(&[(snow / area) as u8, (mud / area) as u8]);
        }
    }
    Ok((grid_width, grid_height, cells))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    #[test]
    fn snow_snapshots_are_equal_in_every_read_field_but_not_their_build_time() {
        let snapshot = || SnowMud {
            width: 2,
            height: 1,
            texels: vec![1, 2, 3, 4, 5, 6, 7, 8],
            province_snow: vec![None, Some(7)],
            post_factors: Some(std::sync::Arc::from([Some(0.0f32), None])),
            build: Duration::from_millis(3),
            summary: Summary::default(),
            refresh: Some(Refresh {
                interval: 25_000,
                identity: 4,
            }),
        };
        let base = snapshot();
        assert!(base == snapshot());
        assert!(
            base == SnowMud {
                build: Duration::from_millis(90),
                ..snapshot()
            },
            "the build time is a report"
        );
        let changes: [fn(&mut SnowMud); 8] = [
            |snow| snow.width = 8,
            |snow| snow.height = 4,
            |snow| snow.texels[7] = 9,
            |snow| snow.province_snow[1] = Some(8),
            |snow| snow.post_factors = None,
            // Bitwise, so negative zero is a different factor.
            |snow| snow.post_factors = Some(std::sync::Arc::from([Some(-0.0f32), None])),
            |snow| snow.summary.with_snow = 1,
            |snow| snow.refresh = None,
        ];
        for (index, change) in changes.into_iter().enumerate() {
            let mut changed = snapshot();
            change(&mut changed);
            assert!(base != changed, "change {index}");
        }
        let mut left = snapshot();
        let mut right = snapshot();
        left.post_factors = Some(std::sync::Arc::from([
            Some(f32::from_bits(0x7fc0_1234)),
            None,
        ]));
        right.post_factors = Some(std::sync::Arc::from([
            Some(f32::from_bits(0x7fc0_1234)),
            None,
        ]));
        assert!(left == right, "equal NaN payloads settle together");
        right.post_factors = Some(std::sync::Arc::from([
            Some(f32::from_bits(0x7fc0_4321)),
            None,
        ]));
        assert!(left != right, "different NaN payloads must not share");
    }

    /// A 12 x 12 province image of four-pixel bands: province 30 north, 20
    /// and 21 (the right third) in the middle, 10 south.
    fn image() -> Vec<u32> {
        let mut ids = Vec::new();
        for row in 0..12u32 {
            let band = [30, 20, 10][(row / 4) as usize];
            for column in 0..12u32 {
                ids.push(if band == 20 && column >= 8 { 21 } else { band });
            }
        }
        ids
    }

    fn map(ids: Vec<u32>) -> MapData {
        MapData {
            width: 12,
            height: 12,
            province_ids: ids,
            definitions: Default::default(),
            country_colors: Default::default(),
        }
    }

    fn frame(view: TableView) -> DecodedFrame {
        let mut tables: [TableView; TableId::ALL.len()] =
            std::array::from_fn(|_| TableView::Unavailable {
                reason: "not needed".into(),
            });
        tables[TableId::WeatherProvinces.index()] = view;
        DecodedFrame {
            branch: 1,
            hour: 1,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    fn row(id: u32, snow: i64, display_snow: u8, display_mud: u8) -> WeatherProvinceRow {
        WeatherProvinceRow {
            id,
            snow,
            display_snow,
            display_mud,
            ..Default::default()
        }
    }

    fn display_row(width: u32, height: u32, edges: Vec<u8>, identity: u64) -> WeatherProvinceRow {
        WeatherProvinceRow {
            display: Recorded::Value(Some(replay_data::rows::WeatherDisplay {
                width,
                height,
                refresh_interval: 25_000,
                refresh_identity: identity,
                edges,
            })),
            ..Default::default()
        }
    }

    fn texel(snow: &SnowMud, x: u32, z: u32) -> [u8; 4] {
        let at = ((z * snow.width + x) * 4) as usize;
        snow.texels[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn cells_average_the_recorded_bytes_south_first_with_both_planes_settled() {
        let rows = vec![
            display_row(3, 3, vec![200, 240, 0, 0, 0, 0, 0, 255, 255, 255], 1),
            row(10, 5, 200, 0),
            // Recorded snow without a written byte draws bare, as in the game.
            row(20, 7, 0, 0),
            row(21, 0, 240, 0),
            row(30, 0, 0, 255),
        ];
        let snow = grid(
            &map(image()),
            &[false; 144],
            &frame(TableView::Complete(TableRows::WeatherProvinces(rows))),
        )
        .unwrap();
        assert_eq!([snow.width, snow.height], [3, 3]);
        // (previous mud, current snow, previous snow, current mud).
        assert_eq!(texel(&snow, 0, 0), [0, 200, 200, 0]);
        assert_eq!(texel(&snow, 1, 1), [0, 0, 0, 0]);
        assert_eq!(texel(&snow, 2, 1), [0, 240, 240, 0]);
        assert_eq!(texel(&snow, 0, 2), [255, 0, 0, 255]);
        assert_eq!(snow.province_snow[20], Some(7));
        assert_eq!(snow.province_snow[11], None);
        assert_eq!(
            snow.summary,
            Summary {
                provinces: 5,
                complete: true,
                with_snow: 2,
                shown_snow: 2,
                shown_mud: 1,
                missing_edges: 0,
            }
        );
    }

    #[test]
    fn perm_snow_split_cells_and_missing_rows() {
        let mut ids = image();
        // Four pixels of cell (0, 1) move to province 21.
        for row in 4..8 {
            ids[row * 12 + 3] = 21;
        }
        let mut perm = [false; 144];
        perm[4..8].fill(true);
        let rows = vec![
            display_row(3, 3, vec![0, 240, 0, 63, 0, 0, 0, 0, 0, 0], 1),
            row(20, 0, 40, 0),
            row(21, 0, 240, 0),
        ];
        let snow = grid(
            &map(ids),
            &perm,
            &frame(TableView::Truncated {
                rows: TableRows::WeatherProvinces(rows),
                missing: 2,
            }),
        )
        .unwrap();
        // Twelve pixels of 40 and four of 240 average to 90.
        assert_eq!(texel(&snow, 0, 1)[1], 90);
        // Four perm_snow pixels of sixteen: 63, and no mud.
        assert_eq!(texel(&snow, 1, 2), [0, 63, 63, 0]);
        assert!(!snow.summary.complete);
        assert!(
            grid(
                &map(image()),
                &[false; 3],
                &frame(TableView::Complete(TableRows::WeatherProvinces(vec![])))
            )
            .is_err()
        );
    }

    #[test]
    fn the_cell_composition_averages_exactly_as_the_pixels_do() {
        // A 16 x 8 image with a province boundary through most cells and
        // perm_snow on a diagonal, against every byte pattern below.
        let ids: Vec<u32> = (0..128u32).map(|at| 1 + (at * 7 + at / 16) % 5).collect();
        let perm: Vec<bool> = (0..128).map(|at| at % 9 == 0).collect();
        let map = MapData {
            width: 16,
            height: 8,
            province_ids: ids,
            definitions: Default::default(),
            country_colors: Default::default(),
        };
        let cells = SnowCells::new(&map, &perm).unwrap();
        for seed in 0..20u32 {
            let rows = (1..=5u32)
                .map(|id| {
                    let byte = |salt: u32| ((id * 97 + seed * 31 + salt) % 256) as u8;
                    row(id, i64::from(id), byte(1), byte(2))
                })
                .collect();
            let frame = frame(TableView::Complete(TableRows::WeatherProvinces(rows)));
            let pixels = grid(&map, &perm, &frame).unwrap();
            let composed = grid_from_cells(&cells, &frame).unwrap();
            assert_eq!(pixels.texels, composed.texels, "seed {seed}");
            assert_eq!(pixels.province_snow, composed.province_snow);
            assert_eq!(pixels.summary, composed.summary);
        }
    }

    #[test]
    fn odd_sizes_leave_the_same_edge_pixels_out_of_both_paths() {
        // 18 x 10 pixels: a 4 x 2 grid, and the last two columns and rows
        // belong to no cell in either path.
        let ids: Vec<u32> = (0..180u32).map(|at| 1 + (at * 11 + at / 18) % 7).collect();
        let perm: Vec<bool> = (0..180).map(|at| at % 13 == 0).collect();
        let map = MapData {
            width: 18,
            height: 10,
            province_ids: ids,
            definitions: Default::default(),
            country_colors: Default::default(),
        };
        let cells = SnowCells::new(&map, &perm).unwrap();
        let rows = (1..=7u32)
            .map(|id| row(id, 0, (id * 37 % 256) as u8, (id * 91 % 256) as u8))
            .collect();
        let frame = frame(TableView::Complete(TableRows::WeatherProvinces(rows)));
        let pixels = grid(&map, &perm, &frame).unwrap();
        let composed = grid_from_cells(&cells, &frame).unwrap();
        assert_eq!([composed.width, composed.height], [4, 2]);
        assert_eq!(pixels.texels, composed.texels);
    }

    #[test]
    fn a_weather_row_beyond_the_maps_provinces_refuses_the_hour() {
        let map = map(image());
        let cells = SnowCells::new(&map, &[false; 144]).unwrap();
        let rows = vec![row(10, 5, 200, 0), row(u32::MAX, 0, 0, 0)];
        let frame = frame(TableView::Complete(TableRows::WeatherProvinces(rows)));
        let Err(refusal) = grid_from_cells(&cells, &frame) else {
            panic!("a foreign province draws");
        };
        assert!(
            refusal.to_string().contains("weather province 4294967295"),
            "{refusal}"
        );
    }

    #[test]
    fn the_grid_follows_the_province_image_size() {
        assert_eq!(grid_size(5632, 2048), [1408, 512]);
        assert_eq!(grid_size(4099, 2050), [1024, 512]);
    }

    #[test]
    fn snow_mud_retains_live_edges_across_dirty_updates_and_rebuilds() {
        let map = map(vec![1; 144]);
        let cells = SnowCells::new(&map, &[false; 144]).unwrap();
        let snapshot = |snow, mud, edge_snow, edge_mud, identity| {
            let edges = [vec![edge_snow; 5], vec![edge_mud; 5]].concat();
            grid_from_cells(
                &cells,
                &frame(TableView::Complete(TableRows::WeatherProvinces(vec![
                    display_row(3, 3, edges, identity),
                    row(1, 0, snow, mud),
                ]))),
            )
            .unwrap()
        };
        // Attachment occurs after a rebuild the recorder never saw. Its live
        // edge already differs from simulation; no previous frame is needed.
        let attached = snapshot(10, 20, 30, 40, 7);
        let dirty = snapshot(100, 200, 30, 40, 8);
        let rebuilt = snapshot(100, 200, 70, 80, 9);
        for (x, z) in [(2, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
            assert_eq!(texel(&attached, x, z), [40, 30, 30, 40]);
            assert_eq!(texel(&dirty, x, z), [40, 30, 30, 40]);
            assert_eq!(texel(&rebuilt, x, z), [80, 70, 70, 80]);
        }
        assert_eq!(texel(&dirty, 0, 0), [200, 100, 100, 200]);
        assert_eq!(dirty.refresh.unwrap().identity, 8);
        // Historical rows retain available interior data, never invent edges.
        let old = grid_from_cells(
            &cells,
            &frame(TableView::Complete(TableRows::WeatherProvinces(vec![
                WeatherProvinceRow {
                    display: Recorded::NotRecorded,
                    ..Default::default()
                },
                row(1, 0, 100, 200),
            ]))),
        )
        .unwrap();
        assert_eq!(old.summary.missing_edges, 5);
        assert_eq!(texel(&old, 2, 2), [0; 4]);
        assert_eq!(texel(&old, 0, 0), [200, 100, 100, 200]);
        assert!(old.refresh.is_none());
    }

    #[test]
    fn snow_mud_refuses_mismatched_live_boundary_dimensions_and_length() {
        let map = map(vec![1; 144]);
        for packet in [
            display_row(2, 3, vec![1; 8], 1),
            display_row(3, 3, vec![1; 9], 1),
        ] {
            let error = grid(
                &map,
                &[false; 144],
                &frame(TableView::Complete(TableRows::WeatherProvinces(vec![
                    packet,
                ]))),
            )
            .err()
            .unwrap();
            assert!(
                error
                    .to_string()
                    .contains("1 refused retained boundary size mismatch")
            );
        }
    }
}
