#[cfg(test)]
use std::time::Duration;
use std::{io, time::Instant};

use content_reader_map::map::MapData;
use replay_data::{rows::Recorded, rows::TableId, rows::WeatherProvinceRow};
use replay_frame::frame::DecodedFrame;

use replay_data::rows::TableRows;
use replay_frame::frame::TableView;

pub mod fold;
pub mod winter;
pub const SNOW_MUD_BLOCK: u32 = 4;

#[derive(Clone, Copy)]
struct ProvinceMap<'a> {
    width: u32,
    height: u32,
    ids: &'a [u32],
}
impl ProvinceMap<'_> {
    fn validate(&self) -> io::Result<()> {
        let count = (self.width as usize).checked_mul(self.height as usize);
        if self.width == 0 || self.height == 0 || count != Some(self.ids.len()) {
            return Err(invalid("province image dimensions differ from its pixels"));
        }
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub use map_display_data::{Refresh, SnowMud, Summary};

pub fn grid_size(width: u32, height: u32) -> [u32; 2] {
    [width / SNOW_MUD_BLOCK, height / SNOW_MUD_BLOCK]
}

pub const TABLES: &[TableId] = &[TableId::WeatherProvinces];

#[cfg(test)]
pub fn grid(map: &MapData, perm_snow: &[bool], frame: &DecodedFrame) -> io::Result<SnowMud> {
    let image = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    grid_with(
        largest_province(map),
        grid_size(map.width, map.height),
        frame,
        |bytes, known| rasterise(&image, perm_snow, bytes, known),
    )
}

fn largest_province(map: &MapData) -> u32 {
    let drawn = map.province_ids.iter().copied().max().unwrap_or(0);
    map.definitions
        .last_key_value()
        .map_or(drawn, |(&defined, _)| drawn.max(defined))
}

pub fn grid_from_cells(cells: &SnowCells, frame: &DecodedFrame) -> io::Result<SnowMud> {
    grid_with(cells.largest, cells.dimensions(), frame, |bytes, known| {
        let (width, height, averaged) = cells.average(bytes);
        Ok((width, height, averaged, cells.unknown(known)))
    })
}

fn grid_with(
    largest: u32,
    dimensions: [u32; 2],
    frame: &DecodedFrame,
    average: impl FnOnce(&[[u8; 2]], &[bool]) -> io::Result<(u32, u32, Vec<u8>, Vec<u8>)>,
) -> io::Result<SnowMud> {
    let (recorded, complete): (&[WeatherProvinceRow], bool) =
        match &*frame.tables[TableId::WeatherProvinces.index()] {
            TableView::Complete(TableRows::WeatherProvinces(rows)) => (rows, true),
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(rows),
                ..
            } => (rows, false),
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => (&[], false),
            _ => {
                return Err(invalid(format!(
                    "hour {} WeatherProvinces contains another table's rows",
                    frame.hour
                )));
            }
        };
    if let Some(row) = recorded.iter().find(|row| row.id > largest) {
        return Err(invalid(format!(
            "Snow/mud presentation: 1 refused weather province {} beyond the map's largest \
             province {largest}",
            row.id
        )));
    }
    let largest =
        usize::try_from(largest).map_err(|_| invalid("province id does not fit memory"))?;
    let provinces = largest
        .checked_add(1)
        .ok_or_else(|| invalid("province count overflow"))?;
    let cells = (dimensions[0] as usize)
        .checked_mul(dimensions[1] as usize)
        .ok_or_else(|| invalid("snow raster byte count overflow"))?;
    let texel_len = cells
        .checked_mul(4)
        .ok_or_else(|| invalid("snow raster byte count overflow"))?;
    let mut bytes = vec![[0_u8; 2]; provinces];
    let mut known = vec![complete; provinces];
    let mut province_snow = vec![None; provinces];
    let mut summary = Summary {
        provinces: recorded.len(),
        complete,
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
        known[id as usize] = true;
        province_snow[id as usize] = Some(snow);
        summary.with_snow += usize::from(snow > 0);
        summary.shown_snow += usize::from(display_snow != 0);
        summary.shown_mud += usize::from(display_mud != 0);
    }
    let start = Instant::now();
    let (width, height, mut cells, mut unknown) = average(&bytes, &known)?;
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
    for (edge, cell) in boundary(width, height).enumerate() {
        cells[cell as usize * 2..cell as usize * 2 + 2].copy_from_slice(
            &display.map_or([0; 2], |d| [d.edges[edge], d.edges[edge_cells + edge]]),
        );
        unknown[cell as usize] = 0;
    }
    let mut texels = Vec::new();
    texels
        .try_reserve_exact(texel_len)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::OutOfMemory))?;
    texels.extend(
        cells
            .chunks_exact(2)
            .flat_map(|cell| [cell[1], cell[0], cell[0], cell[1]]),
    );
    Ok(SnowMud {
        width,
        height,
        texels: map_display_data::Paged::try_from_vec(texels)?,
        unknown: map_display_data::Paged::try_from_vec(unknown)?,
        province_snow: map_display_data::Paged::try_from_vec(province_snow)?,
        post_factors: None,
        build: start.elapsed(),
        summary,
        refresh,
    })
}

pub fn boundary(width: u32, height: u32) -> impl Iterator<Item = u32> {
    (0..height - 1)
        .map(move |z| z * width + width - 1)
        .chain((0..width).map(move |x| (height - 1) * width + x))
}

#[derive(Clone)]
pub struct SnowCells(map_display_data::retained::Retained<SnowComposition>);
impl std::ops::Deref for SnowCells {
    type Target = SnowComposition;
    fn deref(&self) -> &SnowComposition {
        &self.0
    }
}
pub struct SnowComposition {
    largest: u32,
    width: u32,
    height: u32,
    offsets: Vec<usize>,
    entries: Vec<(u32, u8, u8)>,
    perm: Vec<u8>,
    province_cells: std::collections::BTreeMap<u32, Vec<usize>>,
}

impl SnowCells {
    pub fn write_identity(&self, out: &mut impl std::io::Write) -> io::Result<()> {
        for value in [self.largest, self.width, self.height] {
            out.write_all(&value.to_le_bytes())?;
        }
        for value in &self.offsets {
            out.write_all(&(*value as u64).to_le_bytes())?;
        }
        for &(province, snow, mud) in &self.entries {
            out.write_all(&province.to_le_bytes())?;
            out.write_all(&[snow, mud])?;
        }
        out.write_all(&self.perm)
    }
    pub fn dimensions(&self) -> [u32; 2] {
        [self.width, self.height]
    }
    pub fn provinces(&self) -> usize {
        self.largest as usize + 1
    }

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
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| invalid("snow grid count overflow"))?;
        let mut offsets = Vec::with_capacity(count + 1);
        let mut entries: Vec<(u32, u8, u8)> = Vec::new();
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
                        let at = row as usize * image.width as usize + column as usize;
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
                offsets.push(entries.len());
            }
        }
        let mut province_cells: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
        for cell in 0..count {
            for &(province, _, _) in &entries[offsets[cell]..offsets[cell + 1]] {
                province_cells.entry(province).or_default().push(cell);
            }
        }
        Ok(Self(map_display_data::retained::Retained::new(
            SnowComposition {
                province_cells,
                largest: largest_province(map),
                width,
                height,
                offsets,
                entries,
                perm,
            },
        )))
    }

    pub fn largest(&self) -> u32 {
        self.largest
    }

    pub fn cells(&self) -> usize {
        self.perm.len()
    }

    pub fn province_cells(&self, province: u32) -> &[usize] {
        self.province_cells.get(&province).map_or(&[], Vec::as_slice)
    }

    pub fn cell_bytes(&self, cell: usize, bytes: impl Fn(u32) -> [u8; 2]) -> [u8; 2] {
        let area = SNOW_MUD_BLOCK * SNOW_MUD_BLOCK;
        let mut snow = u32::from(self.perm[cell]) * u32::from(u8::MAX);
        let mut mud = 0u32;
        for &(province, snow_pixels, pixels) in &self.entries[self.offsets[cell]..self.offsets[cell + 1]] {
            let [province_snow, province_mud] = bytes(province);
            snow += u32::from(snow_pixels) * u32::from(province_snow);
            mud += u32::from(pixels) * u32::from(province_mud);
        }
        [(snow / area) as u8, (mud / area) as u8]
    }

    fn unknown(&self, known: &[bool]) -> Vec<u8> {
        let known = |province: u32| known.get(province as usize) == Some(&true);
        (0..self.perm.len())
            .map(|cell| self.cell_unknown(cell, known))
            .collect()
    }

    pub fn cell_unknown(&self, cell: usize, known: impl Fn(u32) -> bool) -> u8 {
        u8::from(
            self.entries[self.offsets[cell]..self.offsets[cell + 1]]
                .iter()
                .any(|&(province, _, _)| !known(province)),
        )
    }

    fn average(&self, bytes: &[[u8; 2]]) -> (u32, u32, Vec<u8>) {
        let mut cells = vec![0u8; self.perm.len() * 2];
        for (cell, out) in cells.chunks_exact_mut(2).enumerate() {
            out.copy_from_slice(&self.cell_bytes(cell, |province| bytes.get(province as usize).copied().unwrap_or([0; 2])));
        }
        (self.width, self.height, cells)
    }
}

#[cfg(test)]
fn rasterise(
    image: &ProvinceMap<'_>,
    perm_snow: &[bool],
    bytes: &[[u8; 2]],
    known: &[bool],
) -> io::Result<(u32, u32, Vec<u8>, Vec<u8>)> {
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
    let mut unknown = vec![0u8; grid_width as usize * grid_height as usize];
    for cell_z in 0..grid_height {
        for cell_x in 0..grid_width {
            let (mut snow, mut mud) = (0u32, 0u32);
            let mut unobserved = false;
            for step_z in 0..SNOW_MUD_BLOCK {
                let world_z = cell_z * SNOW_MUD_BLOCK + step_z;
                for step_x in 0..SNOW_MUD_BLOCK {
                    let column = cell_x * SNOW_MUD_BLOCK + step_x;
                    let row = image.height - 1 - world_z;
                    let at = row as usize * image.width as usize + column as usize;
                    let [province_snow, province_mud] =
                        bytes.get(image.ids[at] as usize).copied().unwrap_or([0; 2]);
                    unobserved |= known.get(image.ids[at] as usize) != Some(&true);
                    snow += u32::from(if perm_snow[at] {
                        u8::MAX
                    } else {
                        province_snow
                    });
                    mud += u32::from(province_mud);
                }
            }
            let cell = cell_z as usize * grid_width as usize + cell_x as usize;
            cells[cell * 2..cell * 2 + 2]
                .copy_from_slice(&[(snow / area) as u8, (mud / area) as u8]);
            unknown[cell] = u8::from(unobserved);
        }
    }
    Ok((grid_width, grid_height, cells, unknown))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

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
        assert_eq!(texel(&snow, 0, 1)[1], 90);
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

    fn unknown_cells(snow: &SnowMud) -> Vec<(u32, u32)> {
        (0..snow.height)
            .flat_map(|z| (0..snow.width).map(move |x| (x, z)))
            .filter(|&(x, z)| snow.unknown[(z * snow.width + x) as usize] != 0)
            .collect()
    }

    fn both(map: &MapData, perm: &[bool], view: TableView) -> SnowMud {
        let frame = frame(view);
        let pixels = grid(map, perm, &frame).unwrap();
        let composed = grid_from_cells(&SnowCells::new(map, perm).unwrap(), &frame).unwrap();
        assert_eq!(pixels, composed);
        composed
    }

    #[test]
    fn missing_truncated_and_unrecorded_rows_leave_their_cells_unknown() {
        let map = map(image());
        let perm = [false; 144];
        let edges = || display_row(3, 3, vec![1; 10], 1);
        let rows = |ids: &[u32]| {
            std::iter::once(edges())
                .chain(ids.iter().map(|&id| row(id, 0, id as u8, 0)))
                .collect::<Vec<_>>()
        };
        let whole = both(
            &map,
            &perm,
            TableView::Complete(TableRows::WeatherProvinces(rows(&[10, 20, 21, 30]))),
        );
        assert!(unknown_cells(&whole).is_empty());
        let omitted = both(
            &map,
            &perm,
            TableView::Complete(TableRows::WeatherProvinces(rows(&[10, 21, 30]))),
        );
        assert!(unknown_cells(&omitted).is_empty());
        let truncated = both(
            &map,
            &perm,
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(rows(&[10, 21, 30])),
                missing: 1,
            },
        );
        assert_eq!(unknown_cells(&truncated), [(0, 1), (1, 1)]);
        assert_eq!(truncated.texels, omitted.texels);
        assert_eq!(texel(&truncated, 0, 0), texel(&whole, 0, 0));
        for view in [
            TableView::Unavailable {
                reason: "synthetic read failure".into(),
            },
            TableView::NotRecorded {
                mode: replay_data::scope::CaptureMode::Full,
            },
        ] {
            let absent = both(&map, &perm, view);
            assert_eq!(unknown_cells(&absent), [(0, 0), (1, 0), (0, 1), (1, 1)]);
            assert!(absent.texels.iter().all(|&byte| byte == 0));
        }
        let unbounded = both(
            &map,
            &perm,
            TableView::Complete(TableRows::WeatherProvinces(
                [10, 20, 21, 30].map(|id| row(id, 0, 0, 0)).into(),
            )),
        );
        assert!(unknown_cells(&unbounded).is_empty());
        assert_eq!(unbounded.summary.missing_edges, 5);
        assert!(boundary(3, 3).all(|cell| texel(&unbounded, cell % 3, cell / 3) == [0; 4]));
    }

    #[test]
    fn a_mixed_cell_with_any_unobserved_province_is_unknown_even_under_permanent_snow() {
        let mut ids = image();
        let mut perm = [false; 144];
        for row in 4..8 {
            ids[row * 12 + 3] = 22;
            perm[row * 12 + 3] = true;
        }
        let map = map(ids);
        let rows = vec![
            display_row(3, 3, vec![0; 10], 1),
            row(10, 0, 0, 0),
            row(20, 0, 40, 8),
            row(21, 0, 240, 0),
        ];
        let snow = both(
            &map,
            &perm,
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(rows.clone()),
                missing: 1,
            },
        );
        assert_eq!(unknown_cells(&snow), [(0, 1)]);
        let mut observed = rows;
        observed.push(row(22, 0, 0, 0));
        let snow = both(
            &map,
            &perm,
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(observed),
                missing: 1,
            },
        );
        assert!(unknown_cells(&snow).is_empty());
    }
}

pub fn build(
    cells: &SnowCells,
    frame: &DecodedFrame,
    winter: &impl winter::Factors,
) -> io::Result<SnowMud> {
    let mut snow = grid_from_cells(cells, frame)?;
    snow.post_factors = Some(winter.snow_factors_paged(&snow.province_snow)?);
    Ok(snow)
}

pub fn permanent_mask(
    definitions: &content_reader_key_names::terrain::TerrainDefinitions,
    terrain: &content_reader_image::IndexedImage,
) -> Vec<bool> {
    terrain
        .indices
        .iter()
        .map(|index| {
            definitions
                .palette
                .get(index)
                .is_some_and(|kind| kind.perm_snow)
        })
        .collect()
}

#[cfg(test)]
mod equality_tests {
    use super::*;

    fn snow() -> SnowMud {
        SnowMud {
            width: 1,
            height: 1,
            texels: vec![0; 4].into(),
            unknown: vec![0].into(),
            province_snow: vec![None].into(),
            post_factors: Some(vec![Some(f32::from_bits(0x7fc0_0123)), None, Some(0.0)].into()),
            build: Duration::ZERO,
            summary: Summary::default(),
            refresh: Some(Refresh {
                interval: 1,
                identity: 2,
            }),
        }
    }
    #[test]
    fn equality_includes_all_semantics_and_float_bits_but_not_measured_duration() {
        let original = snow();
        let mut changed = original.clone();
        changed.build = Duration::from_secs(100);
        assert_eq!(original, changed);
        let change: [fn(&mut SnowMud); 10] = [
            |s| s.width += 1,
            |s| s.height += 1,
            |s| s.texels[0] = 1,
            |s| s.unknown[0] = 1,
            |s| s.province_snow[0] = Some(0),
            |s| s.summary.complete = true,
            |s| s.refresh.as_mut().unwrap().identity += 1,
            |s| s.refresh.as_mut().unwrap().interval += 1,
            |s| s.post_factors = None,
            |s| {
                s.post_factors =
                    Some(vec![Some(f32::from_bits(0x7fc0_0124)), None, Some(0.0)].into())
            },
        ];
        for mutate in change {
            let mut changed = original.clone();
            mutate(&mut changed);
            assert_ne!(original, changed);
        }
        let mut changed = original.clone();
        changed.post_factors =
            Some(vec![Some(f32::from_bits(0x7fc0_0123)), None, Some(-0.0)].into());
        assert_ne!(original, changed);
        let mut changed = original.clone();
        changed.post_factors =
            Some(vec![Some(f32::from_bits(0x7fc0_0123)), Some(0.0), Some(0.0)].into());
        assert_ne!(original, changed);
    }
}

#[cfg(test)]
mod table_contract_tests {
    use super::*;
    #[test]
    fn table_availability_matches_the_old_typed_rows_adapter_including_not_recorded() {
        let map = MapData {
            width: 12,
            height: 12,
            province_ids: vec![1; 144],
            definitions: Default::default(),
            country_colors: Default::default(),
        };
        let cells = SnowCells::new(&map, &[false; 144]).unwrap();
        let rows = vec![WeatherProvinceRow {
            id: 1,
            snow: 123,
            display_snow: 91,
            display_mud: 7,
            ..Default::default()
        }];
        for table in [
            TableView::Complete(TableRows::WeatherProvinces(rows.clone())),
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(rows),
                missing: 2,
            },
            TableView::Unavailable {
                reason: "sample missing".into(),
            },
            TableView::NotRecorded {
                mode: replay_data::scope::CaptureMode::Full,
            },
            TableView::Complete(TableRows::Countries(Vec::new())),
        ] {
            let old_reference = match &table {
                TableView::Complete(TableRows::WeatherProvinces(rows)) => {
                    Some((rows.as_slice(), true))
                }
                TableView::Truncated {
                    rows: TableRows::WeatherProvinces(rows),
                    ..
                } => Some((rows.as_slice(), false)),
                TableView::Unavailable { .. } | TableView::NotRecorded { .. } => {
                    Some((&[][..], false))
                }
                _ => None,
            };
            let frame = DecodedFrame {
                branch: 1,
                hour: 23,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "unused".into(),
                    }
                    .into()
                }),
            };
            let mut frame = frame;
            frame.tables[TableId::WeatherProvinces.index()] = table.clone().into();
            let result = grid_from_cells(&cells, &frame);
            if let Some((rows, complete)) = old_reference {
                let snow = result.unwrap();
                assert_eq!(snow.summary.complete, complete, "{table:?}");
                assert_eq!(snow.summary.provinces, rows.len());
                assert_eq!(snow.province_snow[1], rows.first().map(|r| r.snow));
                assert_eq!(
                    &snow.texels[..4],
                    if rows.is_empty() {
                        &[0, 0, 0, 0]
                    } else {
                        &[7, 91, 91, 7]
                    }
                );
                assert!(snow.refresh.is_none());
                assert_eq!(snow.summary.missing_edges, 5);
            } else {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "hour 23 WeatherProvinces contains another table's rows"
                );
            }
        }
    }
}

pub fn display_fields() -> u64 {
    let table = TableId::WeatherProvinces;
    (0..table.schema().len())
        .filter(|&field| {
            table.field_name(field).is_some_and(|name| {
                ["snow", "display_snow", "display_mud", "display"].contains(&name)
            })
        })
        .fold(0, |mask, field| mask | (1u64 << field))
}
