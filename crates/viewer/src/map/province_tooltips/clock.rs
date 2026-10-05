//! Province local clock/night, 140EFFE50 and 140F0BAC0.

use std::io;

use crate::map::frame::{seasonal_direction_raw, seasonal_sine};

const SCALE: i64 = 100_000;

#[derive(Clone, Copy, Debug)]
pub struct Defines {
    gmt_offset: i32,
    poles: [i64; 2],
    feather: i64,
}

impl Defines {
    pub fn load(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            gmt_offset: defines.int32("NGraphics", "GMT_OFFSET")?,
            poles: [
                defines.fixed5("NGraphics", "SOUTH_POLE_OFFSET")?,
                defines.fixed5("NGraphics", "NORTH_POLE_OFFSET")?,
            ],
            feather: defines.fixed5("NGraphics", "DAY_NIGHT_FEATHER")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalClock {
    pub hour: i32,
    /// Signed i64 fixed5 night factor, clamped 0..100000.
    pub night: i64,
    /// PROV_TOOLTIP_LOCAL_TIME's FRAME (1424D8760 rounds, then +1).
    pub icon_frame: i32,
}

pub fn derive(
    hour: i32,
    centroid: [i32; 2],
    map_size: [u32; 2],
    defines: &Defines,
    suspend: Option<[u8; 2]>,
) -> Result<LocalClock, String> {
    match suspend {
        Some([0, 0]) => {}
        Some(flags) => {
            return Err(format!(
                "Province local clock/night refused: weather manager is suspended ({}, {})",
                flags[0], flags[1]
            ));
        }
        None => {
            return Err(
                "Province local clock/night refused: weather-manager suspension flags unavailable"
                    .into(),
            );
        }
    }
    if map_size
        .iter()
        .any(|&dimension| dimension == 0 || dimension > i32::MAX as u32)
    {
        return Err(
            "Province local clock/night refused: map dimensions outside native i32 range".into(),
        );
    }
    // 140F03670 subtracts epoch 43_800_000, then uses signed remainder.
    // A negative result would index before
    // the native hourly-vector array and is not a valid simulation date.
    let utc = hour.wrapping_sub(43_800_000) % 24;
    if utc < 0 {
        return Err("Province local clock/night refused: negative native hour index".into());
    }
    let phase = longitude_phase(centroid[0], map_size[0], defines.gmt_offset);
    let vector = hour_vector(centroid[1], map_size[1], phase, utc, defines.poles);
    let season = seasonal_direction_raw(hour);
    let dot = season.into_iter().zip(vector).fold(0i64, |sum, (a, b)| {
        sum.wrapping_add(a.wrapping_mul(b) / SCALE)
    });
    // 140F03670 stores +/-feather/2 and fixed reciprocal of their difference.
    let upper = defines.feather.wrapping_mul(50_000) / SCALE;
    let lower = upper.wrapping_neg();
    let width = upper.wrapping_sub(lower);
    let factor = if width == 0 {
        0xffff_ffff
    } else {
        10_000_000_000 / width
    };
    let night = dot
        .wrapping_sub(lower)
        .wrapping_mul(factor)
        .wrapping_div(SCALE)
        .clamp(0, SCALE);
    let hour_delta = i64::from(utc) * 10_000_000_000 / 2_400_000;
    let local = (phase.wrapping_add(hour_delta) % SCALE).wrapping_mul(2_400_000);
    Ok(LocalClock {
        hour: (local / 10_000_000_000) as i32 + 1,
        night,
        icon_frame: ((night + 50_000) / SCALE + 1) as i32,
    })
}

fn longitude_phase(x: i32, width: u32, gmt_offset: i32) -> i64 {
    // 140EFFE50: signed i32 subtraction before widening, then fixed division.
    (i64::from(x.wrapping_sub(gmt_offset)).wrapping_mul(10_000_000_000)
        / (i64::from(width) * SCALE))
        .wrapping_add(SCALE)
}

fn hour_vector(y: i32, height: u32, phase: i64, hour: i32, poles: [i64; 2]) -> [i64; 3] {
    // 140EFFE50 constructs the 24 vectors from integer province centroid.
    let latitude = i64::from(y).wrapping_mul(10_000_000_000) / (i64::from(height) * SCALE);
    let latitude = poles[1].wrapping_sub(poles[0]).wrapping_mul(latitude) / SCALE;
    let latitude = latitude.wrapping_add(poles[0]).wrapping_mul(314_159) / SCALE;
    let vertical = cosine(latitude).wrapping_neg();
    let horizontal = SCALE.wrapping_sub(vertical.wrapping_abs());
    let hour_delta = i64::from(hour) * 10_000_000_000 / 2_400_000;
    let angle = (phase.wrapping_add(hour_delta) % SCALE).wrapping_mul(628_318) / SCALE;
    normalize([
        horizontal.wrapping_mul(seasonal_sine(angle)) / SCALE,
        vertical,
        cosine(angle).wrapping_mul(horizontal) / SCALE,
    ])
}

fn cosine(angle: i64) -> i64 {
    seasonal_sine(angle.wrapping_add(157_080))
}

fn normalize(vector: [i64; 3]) -> [i64; 3] {
    if vector == [0; 3] {
        return vector;
    }
    let square = vector.into_iter().fold(0i64, |sum, value| {
        sum.wrapping_add(value.wrapping_mul(value))
    });
    let mut root = square;
    if square > 1 {
        let mut next = square / 2;
        loop {
            root = next;
            next = (square / root + root) / 2;
            if next >= root {
                break;
            }
        }
    }
    let divisor = i64::from(root as i32);
    if divisor == 0 {
        return [0xffff_ffff; 3];
    }
    vector.map(|value| {
        if value == i64::MIN || value == i64::MIN + 1 || value == i64::MAX {
            0xffff_ffff
        } else {
            value.wrapping_mul(SCALE).wrapping_div(divisor)
        }
    })
}

#[cfg(test)]
mod tests;
