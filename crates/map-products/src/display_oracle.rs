use content_reader_map::map::MapData;
use map_display_data::{Refresh, SnowMud, Summary};
use replay_data::rows::{Recorded, TableId, TableRows, WeatherProvinceRow};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{io, time::Instant};
const SNOW_MUD_BLOCK: u32 = 4;
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
fn grid_size(width: u32, height: u32) -> [u32; 2] {
    [width / SNOW_MUD_BLOCK, height / SNOW_MUD_BLOCK]
}
fn largest_province(map: &MapData) -> u32 {
    let drawn = map.province_ids.iter().copied().max().unwrap_or(0);
    map.definitions
        .last_key_value()
        .map_or(drawn, |(&defined, _)| drawn.max(defined))
}
fn grid_with(
    largest: u32,
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
    let boundary = (0..height - 1)
        .map(|z| z * width + width - 1)
        .chain((0..width).map(|x| (height - 1) * width + x));
    for (edge, cell) in boundary.enumerate() {
        cells[cell as usize * 2..cell as usize * 2 + 2].copy_from_slice(
            &display.map_or([0; 2], |d| [d.edges[edge], d.edges[edge_cells + edge]]),
        );
        unknown[cell as usize] = 0;
    }
    let texels: Vec<u8> = cells
        .chunks_exact(2)
        .flat_map(|cell| [cell[1], cell[0], cell[0], cell[1]])
        .collect();
    Ok(SnowMud {
        width,
        height,
        texels: texels.into(),
        unknown: unknown.into(),
        province_snow: province_snow.into(),
        post_factors: None,
        build: start.elapsed(),
        summary,
        refresh,
    })
}
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
pub fn snow(
    map: &MapData,
    perm: &[bool],
    frame: &DecodedFrame,
    winter: &map_snow::winter::Winter,
) -> io::Result<SnowMud> {
    let image = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let mut out = grid_with(largest_province(map), frame, |bytes, known| {
        rasterise(&image, perm, bytes, known)
    })?;
    fn band(value: f32, low: f32, high: f32) -> f32 {
        let width = if high - low <= 0.001 {
            0.001
        } else {
            high - low
        };
        let value = (value - low) / width;
        if 0.0 <= value { value.min(1.0) } else { 0.0 }
    }
    let factors = winter
        .records
        .iter()
        .map(|records| {
            let mut sum = 0.0f32;
            for record in records {
                let raw = out
                    .province_snow
                    .get(record.province as usize)
                    .copied()
                    .flatten()?;
                let province = band(
                    raw as f32 / 100_000.0,
                    winter.defines.per_province[0],
                    winter.defines.per_province[1],
                );
                sum += province * record.weight;
            }
            Some(band(sum, winter.defines.total[0], winter.defines.total[1]))
        })
        .collect::<Vec<_>>();
    out.post_factors = Some(factors.into());
    Ok(out)
}
