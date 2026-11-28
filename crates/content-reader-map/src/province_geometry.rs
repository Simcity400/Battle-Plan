
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

use content_reader_layers::ResolvedLayers;
use content_reader_parser::fields::invalid;
use crate::map::ProvinceKind;

pub mod land_areas;

pub type Bounds = [i32; 4];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceGeometry {
    pub width: i32,
    pub height: i32,
    pub centroids: BTreeMap<u32, [i32; 2]>,
    pub bounds: BTreeMap<u32, Bounds>,
    pub land: BTreeSet<u32>,
    pub lake: BTreeSet<u32>,
    pub nonsea_indices: BTreeMap<u32, i32>,
}

impl ProvinceGeometry {
    pub fn kind_flags(&self, id: u32) -> Option<u8> {
        self.centroids
            .contains_key(&id)
            .then(|| u8::from(self.land.contains(&id)) | (u8::from(self.lake.contains(&id)) << 1))
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let files = content_reader_key_names::terrain::read_default_map(layers)?;
        let map = engine_map(
            &layers.read(&files.file("definitions")?)?,
            &layers.read(&files.file("provinces")?)?,
        )?;
        Ok(Self::from_map(&map))
    }

    pub fn shared(layers: &ResolvedLayers) -> io::Result<Arc<Self>> {
        Self::keep(layers, || Self::read(layers))
    }

    pub(crate) fn keep(layers: &ResolvedLayers, load: impl FnOnce() -> io::Result<Self>) -> io::Result<Arc<Self>> {
        layers.product("province geometry", 1, &[], |_| load())
    }

    pub(crate) fn from_map(map: &EngineMap) -> Self {
        let mut sums: BTreeMap<u32, (i32, i32, u32)> =
            map.defined.iter().map(|&id| (id, (0, 0, 0))).collect();
        let mut bounds: BTreeMap<u32, Bounds> = BTreeMap::new();
        for_each_run(&map.ids, map.width, map.height, |id, y, start, length| {
            if let Some(entry) = sums.get_mut(&id) {
                for pixel in 0..length {
                    entry.0 = entry.0.wrapping_add(start.wrapping_add(pixel));
                }
                entry.1 = entry.1.wrapping_add(y.wrapping_mul(length));
                entry.2 = entry.2.wrapping_add(length as u32);
            }
            let end = start + length;
            bounds
                .entry(id)
                .and_modify(|[x, bottom, width, height]| {
                    if start < *x {
                        *width += *x - start;
                        *x = start;
                    }
                    if *width + *x < end {
                        *width = end - *x;
                    }
                    if y < *bottom {
                        *height += *bottom - y;
                        *bottom = y;
                    }
                    if *height + *bottom < y + 1 {
                        *height = y - *bottom + 1;
                    }
                })
                .or_insert([start, y, length, 1]);
        });
        let centroids = sums
            .into_iter()
            .map(|(id, (x, y, count))| {
                let point = if count == 0 {
                    [0, 0]
                } else {
                    [x.wrapping_div(count as i32), y.wrapping_div(count as i32)]
                };
                (id, point)
            })
            .collect();
        Self {
            width: map.width as i32,
            height: map.height as i32,
            centroids,
            bounds,
            land: map.land.clone(),
            lake: map.lake.clone(),
            nonsea_indices: map.nonsea_indices.clone(),
        }
    }

    pub fn state_centre(&self, provinces: &[u32]) -> Option<[i32; 2]> {
        let half = self.width / 2;
        let mut plain: Option<Bounds> = None;
        let mut wrapped: Option<Bounds> = None;
        let grow = |slot: &mut Option<Bounds>, [x, y, w, h]: Bounds| match slot {
            None => *slot = Some([x, y, w, h]),
            Some([sx, sy, sw, sh]) => {
                if x < *sx {
                    *sw += *sx - x;
                    *sx = x;
                }
                if *sx + *sw < w + x {
                    *sw = w - *sx + x;
                }
                if y < *sy {
                    *sh += *sy - y;
                    *sy = y;
                }
                if *sh + *sy < h + y {
                    *sh = h - *sy + y;
                }
            }
        };
        for province in provinces {
            let [x, y, w, h] = self.bounds.get(province).copied().unwrap_or_default();
            grow(&mut plain, [x, y, w, h]);
            grow(
                &mut wrapped,
                [if x < half { self.width + x } else { x }, y, w, h],
            );
        }
        let (plain, wrapped) = (plain?, wrapped?);
        let [x, y, w, h] = if wrapped[2] < plain[2] {
            wrapped
        } else {
            plain
        };
        let mut centre_x = w / 2 + x;
        if centre_x < 0 {
            centre_x += self.width;
        } else if centre_x >= self.width {
            centre_x -= self.width;
        }
        Some([centre_x, h / 2 + y])
    }

    pub fn nearest_to_centre(&self, provinces: &[u32]) -> Option<u32> {
        let centre = self.state_centre(provinces)?;
        let distance = |province: &u32| {
            let [x, y] = self.centroids.get(province).copied().unwrap_or_default();
            let (dx, dy) = (x.wrapping_sub(centre[0]), y.wrapping_sub(centre[1]));
            dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))
        };
        let mut best = *provinces.first()?;
        for province in &provinces[1..] {
            if distance(province) < distance(&best) {
                best = *province;
            }
        }
        Some(best)
    }

    pub fn wrapped_distance(&self, from: u32, to: u32) -> Option<i64> {
        if from == to {
            return Some(0);
        }
        let a = self.centroids.get(&from)?;
        let b = self.centroids.get(&to)?;
        let dx = a[0].wrapping_sub(b[0]);
        let dy = a[1].wrapping_sub(b[1]);
        let wrapped = if dx.wrapping_abs() <= self.width / 2 {
            dx
        } else {
            self.width.wrapping_sub(dx.wrapping_abs())
        };
        Some(native_fixed_sqrt(
            dy.wrapping_mul(dy)
                .wrapping_add(wrapped.wrapping_mul(wrapped)) as u32,
        ))
    }
}

pub fn native_fixed_sqrt(squared: u32) -> i64 {
    if (squared as i32) < 1 {
        return 0;
    }
    let shifted = squared & 0xffe0_0000 != 0;
    let input = if shifted {
        i64::from((squared as i32) >> 10) * 100_000
    } else {
        i64::from(squared as i32) * 100_000
    };
    let root = native_fixed5_sqrt(input);
    if shifted { root << 5 } else { root }
}

pub fn native_fixed5_sqrt(mut input: i64) -> i64 {
    if input < 10_001 {
        return sqrt_newton(input.wrapping_mul(100)).wrapping_mul(10_000) / 100_000;
    }
    let mut decades = 0;
    while input > 2_999_999_999 {
        input = input.wrapping_mul(1_000) / 100_000;
        decades += 1;
    }
    let mut root = sqrt_newton(input);
    for _ in 0..decades {
        root = root.wrapping_mul(10);
    }
    root
}

fn sqrt_newton(input: i64) -> i64 {
    if input < 2 {
        return 0;
    }
    let mut root = input.wrapping_mul(100_000) / 200_000;
    let tolerance = input.wrapping_mul(1_000) / 100_000;
    loop {
        let error = (root.wrapping_mul(root) / 100_000).wrapping_sub(input);
        let denominator = root.wrapping_mul(2);
        let step = if denominator == 0
            || (error as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
        {
            0xffff_ffff
        } else {
            error.wrapping_mul(100_000) / denominator
        };
        root = root.wrapping_sub(step);
        if (-tolerance..=tolerance).contains(&error) || step == 0 {
            return root;
        }
    }
}

pub struct EngineMap {
    pub ids: Vec<u32>,
    pub width: usize,
    pub height: usize,
    pub defined: BTreeSet<u32>,
    pub land: BTreeSet<u32>,
    pub lake: BTreeSet<u32>,
    pub nonsea_indices: BTreeMap<u32, i32>,
}

fn color_key(red: u8, green: u8, blue: u8) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

pub(crate) fn engine_map(definition_csv: &[u8], provinces_bmp: &[u8]) -> io::Result<EngineMap> {
    engine_map_checked(definition_csv, provinces_bmp, &mut || Ok(()))
}

pub fn engine_map_checked(
    definition_csv: &[u8],
    provinces_bmp: &[u8],
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<EngineMap> {
    check()?;
    let mut color_to_id = HashMap::new();
    let rows = crate::map::definition_rows(definition_csv)?;
    let mut count = u32::try_from(rows.len())
        .map_err(|_| invalid("definition.csv has more records than native u32 indexes"))?;
    let (mut land, mut lake) = (BTreeSet::new(), BTreeSet::new());
    for (ordinal, row) in rows.iter().enumerate() {
        if ordinal % 4096 == 0 {
            check()?;
        }
        if ordinal != 0 {
            match row.kind() {
                Some(ProvinceKind::Land) => {
                    land.insert(ordinal as u32);
                }
                Some(ProvinceKind::Lake) => {
                    lake.insert(ordinal as u32);
                }
                _ => {}
            }
        }
        let [red, green, blue] = row.rgb;
        if row.id != 0 {
            color_to_id.insert(color_key(red, green, blue), u32::from(row.id));
        }
    }
    let (colors, width, height) = colour_raster(provinces_bmp, check)?;
    check()?;
    let mut ids = vec![0_u32; colors.len()];
    let mut highest_id = 0;
    for engine_row in 0..height {
        check()?;
        let y = height - 1 - engine_row;
        for x in 0..width {
            if x % 4096 == 0 {
                check()?;
            }
            let at = y * width + x;
            ids[at] = match color_to_id.get(&colors[at]) {
                Some(&id) if id != 0 => id,
                _ => {
                    let invented = count as u16 as u32;
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("native invented province count exceeds u32"))?;
                    land.insert(invented);
                    color_to_id.insert(colors[at], invented);
                    invented
                }
            };
            highest_id = highest_id.max(ids[at]);
        }
    }
    if !ids.is_empty() && highest_id >= count {
        return Err(invalid(format!(
            "provinces.bmp carries province {highest_id} outside the {count} native records"
        )));
    }
    refuse_zero_length_runs(&ids, width, height, check)?;
    check()?;
    if count > i32::MAX as u32 {
        return Err(invalid(
            "province record count exceeds native signed map index domain",
        ));
    }
    let nonsea_indices = (0..count)
        .filter(|&ordinal| {
            rows.get(ordinal as usize)
                .is_none_or(|row| row.kind() != Some(ProvinceKind::Sea))
        })
        .enumerate()
        .map(|(index, province)| (province, index as i32))
        .collect();
    Ok(EngineMap {
        ids,
        width,
        height,
        defined: (1..count).collect(),
        land,
        lake,
        nonsea_indices,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RasterLayout {
    pub width: usize,
    pub height: usize,
    pub components: usize,
    pub pitch: usize,
    pub padded: usize,
    pub pixels: usize,
}

impl RasterLayout {
    pub fn written(&self) -> usize {
        (self.height - 1) * self.pitch + self.padded
    }

    pub fn file_end(&self) -> io::Result<usize> {
        self.padded
            .checked_mul(self.height)
            .and_then(|rows| rows.checked_add(self.pixels))
            .ok_or_else(|| invalid("BMP pixel extent overflows"))
    }

    pub fn colour(pixel: &[u8]) -> u32 {
        color_key(pixel[2], pixel[1], pixel[0])
    }
}

pub(crate) fn raster_layout(data: &[u8]) -> io::Result<RasterLayout> {
    let head = data
        .get(..54)
        .ok_or_else(|| invalid("provinces.bmp is shorter than the native 54-byte header"))?;
    if &head[..2] != b"BM" {
        return Err(invalid("provinces.bmp is not a BMP"));
    }
    let le32 = |at: usize| i32::from_le_bytes(head[at..at + 4].try_into().unwrap());
    let (width_raw, height_raw) = (le32(18), le32(22));
    let bpp = u16::from_le_bytes(head[28..30].try_into().unwrap());
    let components: i32 = match bpp {
        1 | 8 => 1,
        16 => 2,
        24 => 3,
        _ => {
            return Err(invalid(format!(
                "native province loader rejects {bpp}bpp BMP"
            )));
        }
    };
    if le32(30) != 0 {
        return Err(invalid("native province loader rejects a compressed BMP"));
    }
    if width_raw <= 0 || height_raw <= 0 {
        return Err(invalid("provinces.bmp has a nonpositive dimension"));
    }
    let pitch_raw = if bpp == 1 {
        components.wrapping_mul(width_raw) >> 3
    } else {
        components.wrapping_mul(width_raw)
    };
    let padded_raw = if bpp == 1 {
        components.wrapping_mul(width_raw).wrapping_add(3) >> 3
    } else {
        components.wrapping_mul(width_raw).wrapping_add(3) & !3
    };
    let advance_raw = if pitch_raw < 1 { padded_raw } else { pitch_raw };
    let exact_pitch = if bpp == 1 {
        (i128::from(components) * i128::from(width_raw)) >> 3
    } else {
        i128::from(components) * i128::from(width_raw)
    };
    let allocation = pitch_raw.wrapping_mul(height_raw).wrapping_add(7);
    let written_raw =
        (i128::from(height_raw) - 1) * i128::from(advance_raw) + i128::from(padded_raw);
    if i128::from(allocation) != exact_pitch * i128::from(height_raw) + 7
        || allocation < 1
        || i128::from(allocation) < written_raw
        || height_raw.wrapping_mul(width_raw) <= 0
    {
        return Err(invalid(format!(
            "native province BMP rows overflow for {width_raw}x{height_raw} at {bpp}bpp"
        )));
    }
    let pixels = if bpp == 24 {
        54
    } else {
        u32::from_le_bytes(head[10..14].try_into().unwrap()) as usize
    };
    Ok(RasterLayout {
        width: width_raw as usize,
        height: height_raw as usize,
        components: components as usize,
        pitch: advance_raw as usize,
        padded: padded_raw as usize,
        pixels,
    })
}

fn colour_raster(
    data: &[u8],
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<(Vec<u32>, usize, usize)> {
    check()?;
    let layout = raster_layout(data)?;
    let RasterLayout { width, height, components, pitch, padded, pixels } = layout;
    if layout.file_end()? > data.len() {
        return Err(invalid("provinces.bmp is short of its native decoded rows"));
    }
    let written = layout.written();
    check()?;
    let mut decoded = vec![0_u8; written];
    for row in 0..height {
        let (dst, src) = (row * pitch, pixels + row * padded);
        for (destination, source) in decoded[dst..dst + padded]
            .chunks_mut(4096)
            .zip(data[src..src + padded].chunks(4096))
        {
            check()?;
            destination.copy_from_slice(source);
        }
    }
    check()?;
    let mut colors = vec![0_u32; width * height];
    for y in 0..height {
        check()?;
        let row = (height - 1 - y) * pitch;
        for x in 0..width {
            if x % 4096 == 0 {
                check()?;
            }
            let at = row + x * components;
            let pixel = decoded
                .get(at..at + 3)
                .ok_or_else(|| invalid("native province colour reads unwritten BMP storage"))?;
            colors[y * width + x] = RasterLayout::colour(pixel);
        }
    }
    Ok((colors, width, height))
}

fn for_each_run(ids: &[u32], width: usize, height: usize, run: impl FnMut(u32, i32, i32, i32)) {
    for_each_run_checked(ids, width, height, &mut || Ok(()), run)
        .expect("uncancellable native raster runs");
}

fn for_each_run_checked(
    ids: &[u32],
    width: usize,
    height: usize,
    check: &mut impl FnMut() -> io::Result<()>,
    mut run: impl FnMut(u32, i32, i32, i32),
) -> io::Result<()> {
    for y in 0..height {
        check()?;
        let native_y = i32::from((height - 1 - y) as u16);
        let row = &ids[y * width..(y + 1) * width];
        let mut start = 0;
        while start < width {
            if start % 4096 == 0 {
                check()?;
            }
            let id = row[start];
            let mut end = start + 1;
            while end < width && row[end] == id {
                if end % 4096 == 0 {
                    check()?;
                }
                end += 1;
            }
            run(
                id,
                native_y,
                i32::from(start as u16),
                i32::from((end - start) as u16),
            );
            start = end;
        }
    }
    Ok(())
}

fn refuse_zero_length_runs(
    ids: &[u32],
    width: usize,
    height: usize,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let mut runs: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for_each_run_checked(ids, width, height, check, |id, _, _, length| {
        let entry = runs.entry(id).or_default();
        entry.0 += 1;
        entry.1 += length as usize;
    })?;
    if let Some((id, (seen, _))) = runs
        .iter()
        .find(|(_, (seen, kept))| *seen != 0 && *kept == 0)
    {
        return Err(invalid(format!(
            "all {seen} raster runs of province {id} narrow to zero length"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp_two_by_two(file_rows: [[[u8; 3]; 2]; 2]) -> Vec<u8> {
        let mut bmp = vec![0_u8; 70];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&70_u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&2_i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&2_i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
        for (y, row) in file_rows.iter().enumerate() {
            for (x, [r, g, b]) in row.iter().enumerate() {
                let at = 54 + y * 8 + x * 3;
                bmp[at..at + 3].copy_from_slice(&[*b, *g, *r]);
            }
        }
        bmp
    }

    fn map(engine_rows: &[&[u32]], count: u32) -> EngineMap {
        EngineMap {
            ids: engine_rows
                .iter()
                .rev()
                .flat_map(|row| row.iter().copied())
                .collect(),
            width: engine_rows[0].len(),
            height: engine_rows.len(),
            defined: (1..count).collect(),
            land: (1..count).collect(),
            lake: BTreeSet::new(),
            nonsea_indices: (0..count).map(|id| (id, id as i32)).collect(),
        }
    }

    #[test]
    fn native_raster_decode_cancels_between_bitmap_rows() {
        let bitmap = bmp_two_by_two([[[1, 2, 3]; 2]; 2]);
        let mut checks = 0;
        let error = colour_raster(&bitmap, &mut || {
            checks += 1;
            if checks == 4 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(checks, 4);
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn native_run_validation_cancels_inside_a_long_single_row() {
        let mut checks = 0;
        let error = refuse_zero_length_runs(&vec![1; 9000], 9000, 1, &mut || {
            checks += 1;
            if checks == 3 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(checks, 3);
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn native_record_resolution_cancels_after_a_raster_row() {
        let bitmap = bmp_two_by_two([[[1, 2, 3]; 2]; 2]);
        let mut checks = 0;
        let error = engine_map_checked(b"0;0;0;0;land\n1;1;2;3;land\n", &bitmap, &mut || {
            checks += 1;
            if checks == 15 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .err()
        .expect("cancelled record resolution");
        assert_eq!(checks, 15);
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn native_nonsea_indices_include_zero_lakes_unknown_and_invented_records() {
        let definitions =
            b"0;0;0;0;land\n1;255;0;0;sea\n2;0;255;0;lake\n3;1;2;3;unknown\n4;4;5;6;land\n";
        let map = engine_map(
            definitions,
            &bmp_two_by_two([[[255, 0, 0], [0, 0, 255]], [[0, 255, 0], [255, 0, 0]]]),
        )
        .unwrap();
        let geometry = ProvinceGeometry::from_map(&map);
        assert_eq!(
            geometry.nonsea_indices,
            BTreeMap::from([(0, 0), (2, 1), (3, 2), (4, 3), (5, 4)])
        );
        assert_eq!(geometry.land, BTreeSet::from([4, 5]));
        assert_eq!(geometry.lake, BTreeSet::from([2]));
    }

    #[test]
    fn file_order_invents_unclaimed_colours_and_keeps_unpainted_records() {
        const RED: [u8; 3] = [255, 0, 0];
        const GREEN: [u8; 3] = [0, 255, 0];
        const BLUE: [u8; 3] = [0, 0, 255];
        let definitions = b"0;0;0;0;land\n1;255;0;0;land\n2;0;255;0;land\n4;1;2;3;land\n";
        let map = engine_map(definitions, &bmp_two_by_two([[RED, BLUE], [RED, GREEN]])).unwrap();
        assert_eq!(map.ids, [1, 2, 1, 4]);
        let geometry = ProvinceGeometry::from_map(&map);
        assert_eq!(geometry.centroids[&3], [0, 0]);
        assert_eq!(geometry.centroids[&1], [0, 0]);
        assert_eq!(geometry.bounds[&1], [0, 0, 1, 2]);
        assert!(!geometry.bounds.contains_key(&3));
    }

    #[test]
    fn centroids_sum_runs_and_boxes_grow_to_every_run() {
        let geometry = ProvinceGeometry::from_map(&map(&[&[1, 1, 1, 2], &[1, 2, 2, 2]], 3));
        assert_eq!(geometry.bounds[&1], [0, 0, 3, 2]);
        assert_eq!(geometry.bounds[&2], [1, 0, 3, 2]);
        assert_eq!(geometry.centroids[&1], [0, 0]);
        assert_eq!(geometry.state_centre(&[1, 2]), Some([2, 1]));
        assert_eq!(geometry.nearest_to_centre(&[1, 2]), Some(2));
    }

    #[test]
    fn a_state_across_the_seam_takes_the_narrower_wrapped_box() {
        let row: Vec<u32> = (0..10)
            .map(|x| match x {
                0 | 1 => 1,
                8 | 9 => 3,
                _ => 2,
            })
            .collect();
        let geometry = ProvinceGeometry::from_map(&map(&[&row], 4));
        assert_eq!(geometry.state_centre(&[1, 3]), Some([0, 0]));
        assert_eq!(geometry.wrapped_distance(1, 3), Some(200_000));
    }

    #[test]
    fn fixed_square_root_keeps_integer_rounding() {
        assert_eq!(native_fixed_sqrt(0), 0);
        assert_eq!(native_fixed_sqrt(4), 200_000);
        assert_eq!(native_fixed_sqrt(2), 141_422);
        assert_eq!(native_fixed5_sqrt(-1), 0);
        assert_eq!(native_fixed5_sqrt(i64::MIN), 0);
        assert_eq!(native_fixed5_sqrt(i64::MAX), 0);
        assert_eq!(native_fixed5_sqrt(1), 317);
        assert_eq!(native_fixed5_sqrt(10_000), 31_622);
        assert_eq!(native_fixed5_sqrt(10_001), 31_625);
        assert_eq!(native_fixed5_sqrt(22_222), 47_141);
        assert_eq!(native_fixed5_sqrt(3_000_000_000), 17_320_530);
    }
}
