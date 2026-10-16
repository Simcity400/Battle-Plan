//! Borrowed province and terrain images and shared pixel/world sampling contracts.

use crate::errors::invalid;
use installed_content::map::MapData;
use std::io;

/// A province id image in map pixel space: `provinces.bmp`, origin top-left, x right, y down.
///
/// The same shape as [`installed_content::map::MapData`](../../../installed-content/map/struct.MapData.html)'s
/// `width`, `height` and `province_ids`, borrowed so this module needs no loader dependency.
#[derive(Clone, Copy, Debug)]
pub struct ProvinceMap<'a> {
    pub width: u32,
    pub height: u32,
    /// Row-major province ids, `width * height` of them, starting at the top-left pixel.
    pub ids: &'a [u32],
}

impl ProvinceMap<'_> {
    /// Checks the dimensions are usable and the id slice covers exactly the declared pixels.
    pub fn validate(&self) -> io::Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(invalid("province map needs a nonzero width and height"));
        }
        let count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| invalid("province map pixel count overflows"))?;
        if self.ids.len() != count {
            return Err(invalid(format!(
                "province map holds {} ids for {} x {} pixels",
                self.ids.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }
}

/// `WATER_HEIGHT`: the vertex shader clamps a strip's `y` up to this so borders over water lie flat
/// on the sea plane.
pub const WATER_HEIGHT: f32 = 9.5;
/// World units per heightmap byte. Sea-level byte 95 is exactly [`WATER_HEIGHT`].
pub const HEIGHT_SCALE: f32 = 0.1;

/// Map pixel space to game world space for a map `map_height` pixels tall: `x` is the bitmap
/// column and `z` counts up from the bottom row, so `z = height - 1 - y`. The
/// height is the content's provinces bitmap. The inverse is [`pixel_from_world`].
pub fn world_from_pixel(map_height: u32, x: f32, y: f32) -> (f32, f32) {
    (x, map_height as f32 - 1.0 - y)
}

/// The inverse of [`world_from_pixel`].
pub fn pixel_from_world(map_height: u32, x: f32, z: f32) -> (f32, f32) {
    (x, map_height as f32 - 1.0 - z)
}

/// Terrain height samples in map pixel space, one byte per pixel, rows top-left first.
///
/// The caller feeds `map/heightmap.bmp` decoded by `installed_content::image::bmp::decode_indexed`; the
/// bytes are heights in [`HEIGHT_SCALE`] world units, the same scale `pdxmap.shader`'s vertex shader
/// uses.
#[derive(Clone, Copy, Debug)]
pub struct HeightField<'a> {
    pub width: u32,
    pub height: u32,
    pub values: &'a [u8],
}

impl HeightField<'_> {
    /// Each rail sample removes its half-pixel shift, interpolates four height bytes in X then Z,
    /// scales by .1 and clamps to sea level. Sample indices clamp rather than wrap.
    pub fn border_height_at_world(&self, x: f32, z: f32) -> f32 {
        self.native_height(x + 0.5, z + 0.5)
    }

    /// The height of the surface the terrain pass draws at world `(x, z)`, or the water plane where
    /// that is higher: one vertex per map pixel at its height byte, each cell split along the
    /// diagonal the terrain's index pattern chooses (from its south-west corner where the cell's
    /// column and row sum to an even number, else from its south-east corner). `x` wraps with the
    /// map and `z` clamps to it. A walker or a camera standing on the drawn ground reads this.
    pub fn surface_height(&self, x: f32, z: f32) -> f32 {
        if self.width == 0
            || self.height == 0
            || self.values.len() < (self.width * self.height) as usize
        {
            return WATER_HEIGHT;
        }
        let z = z.clamp(0.0, (self.height - 1) as f32);
        let (column, bottom) = (x.floor(), z.floor());
        let (fx, fz) = (x - column, z - bottom);
        let column = column as i64;
        let bottom = bottom as i64;
        let sample = |dx: i64, dz: i64| {
            let column = (column + dx).rem_euclid(i64::from(self.width)) as usize;
            let bottom = (bottom + dz).clamp(0, i64::from(self.height) - 1) as usize;
            let row = self.height as usize - 1 - bottom;
            f32::from(self.values[row * self.width as usize + column]) * HEIGHT_SCALE
        };
        let [a, b, c, d] = [sample(0, 0), sample(1, 0), sample(0, 1), sample(1, 1)];
        let height = if (column + bottom).rem_euclid(2) == 0 {
            if fx >= fz {
                a + (b - a) * fx + (d - b) * fz
            } else {
                a + (d - c) * fx + (c - a) * fz
            }
        } else if fx + fz <= 1.0 {
            a + (b - a) * fx + (c - a) * fz
        } else {
            d + (c - d) * (1.0 - fx) + (b - d) * (1.0 - fz)
        };
        height.max(WATER_HEIGHT)
    }

    /// Bilinear bottom-up height sampling with a half-pixel shift, clamped indices, .1 scaling and a
    /// sea floor. Movement routes supply spline x/z coordinates.
    pub fn native_height(&self, x: f32, z: f32) -> f32 {
        let x = x - 0.5;
        let z = z - 0.5;
        let column = (x as i64).clamp(0, i64::from(self.width.saturating_sub(2))) as usize;
        let bottom = (z as i64).clamp(0, i64::from(self.height.saturating_sub(2))) as usize;
        let sample = |column: usize, bottom: usize| {
            let row = self.height as usize - 1 - bottom.min(self.height as usize - 1);
            f32::from(self.values[row * self.width as usize + column.min(self.width as usize - 1)])
        };
        let tx = x - column as f32;
        let tz = z - bottom as f32;
        let a = sample(column, bottom);
        let b = sample(column + 1, bottom);
        let c = sample(column, bottom + 1);
        let d = sample(column + 1, bottom + 1);
        let low = (b - a) * tx + a;
        let high = (d - c) * tx + c;
        (((high - low) * tz + low) * HEIGHT_SCALE).max(WATER_HEIGHT)
    }

    /// The raid arrow height callbacks' terrain sample: floor and ceiling pixels of each fixed5
    /// coordinate, x wrapped once by `map_width` and every index clamped to the height bitmap. The four
    /// bytes are scaled by 0.1 and interpolated in x then z by the fixed5 fractions. There is no
    /// half-pixel shift or sea floor.
    pub fn fixed_height(&self, x: i64, z: i64, map_width: i64) -> f32 {
        const SCALE: i64 = 100_000;
        let floor = |value: i64| value.div_euclid(SCALE);
        let ceil = |value: i64| -((-value).div_euclid(SCALE));
        let (x0, x1, z0, z1) = (floor(x), ceil(x), floor(z), ceil(z));
        let wrap = |column: i64| {
            if column < 0 {
                column + map_width
            } else if column >= map_width {
                column - map_width
            } else {
                column
            }
        };
        let column = |value: i64| wrap(value).clamp(0, i64::from(self.width) - 1) as usize;
        let bottom = |value: i64| value.clamp(0, i64::from(self.height) - 1) as usize;
        let sample = |x: i64, z: i64| {
            let row = self.height as usize - 1 - bottom(z);
            f32::from(self.values[row * self.width as usize + column(x)]) * HEIGHT_SCALE
        };
        let tx = (x - x0 * SCALE) as f32 / SCALE as f32;
        let tz = (z - z0 * SCALE) as f32 / SCALE as f32;
        let low = (sample(x1, z0) - sample(x0, z0)) * tx + sample(x0, z0);
        let high = (sample(x1, z1) - sample(x0, z1)) * tx + sample(x0, z1);
        (high - low) * tz + low
    }

    /// Checks the sample count matches the declared dimensions.
    pub fn validate(&self) -> io::Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(invalid("height field needs a nonzero width and height"));
        }
        let count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| invalid("height field sample count overflows"))?;
        if self.values.len() != count {
            return Err(invalid(format!(
                "height field holds {} samples for {} x {} pixels",
                self.values.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }

    /// Nearest-sample lookup retained for the other map passes' existing placement rules.
    /// Border rails use [`Self::border_height_at_world`], the native bilinear/clamped call.
    /// x wraps with the world and the row clamps at the poles.
    pub fn height_at_world(&self, x: f32, z: f32) -> f32 {
        let (column, row) = pixel_from_world(self.height, x, z);
        let column = ((column + 0.5).floor() as i64).rem_euclid(i64::from(self.width)) as usize;
        let row = ((row + 0.5).floor() as i64).clamp(0, i64::from(self.height) - 1) as usize;
        let sample = self
            .values
            .get(row * (self.width as usize) + column)
            .copied()
            .unwrap_or(0);
        f32::from(sample) * HEIGHT_SCALE
    }
}

/// Truncate both coordinates to integers toward zero and
/// then bounds-checks them, so a sub-pixel negative position names pixel zero.
/// World z is `height - 1 - row` because `provinces.bmp` is stored bottom-up.
pub fn pixel(width: u32, height: u32, x: f32, z: f32) -> Option<usize> {
    if width == 0 || height == 0 || !x.is_finite() || !z.is_finite() {
        return None;
    }
    let column = x as i64;
    let row = i64::from(height) - 1 - z as i64;
    ((0..i64::from(width)).contains(&column) && (0..i64::from(height)).contains(&row))
        .then(|| row as usize * width as usize + column as usize)
}

/// The province under a world position of the map's own province bitmap.
pub fn province_at(map: &MapData, x: f32, z: f32) -> Option<u32> {
    ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    }
    .at(x, z)
}

impl ProvinceMap<'_> {
    /// The province under a world position, or `None` off the map or on an
    /// unclaimed pixel. There is no nearest-province fallback.
    pub fn at(&self, x: f32, z: f32) -> Option<u32> {
        pixel(self.width, self.height, x, z)
            .and_then(|index| self.ids.get(index).copied())
            .filter(|id| *id != 0)
    }
}

#[cfg(test)]
mod tests;

impl ProvinceMap<'_> {
    /// True when the id slice matches the declared size and neither dimension is zero.
    pub fn is_well_formed(&self) -> bool {
        let expected = (self.width as usize).checked_mul(self.height as usize);
        self.width > 0 && self.height > 0 && expected == Some(self.ids.len())
    }

    /// The id at a pixel, wrapping x around the date line and rejecting y outside the image.
    pub fn id_at(&self, x: i32, y: i32) -> Option<u32> {
        if y < 0 || y >= self.height as i32 {
            return None;
        }
        let x = x.rem_euclid(self.width as i32) as usize;
        self.ids.get(y as usize * self.width as usize + x).copied()
    }
}
