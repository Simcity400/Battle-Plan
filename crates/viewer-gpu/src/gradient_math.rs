
use std::io;

use viewer_runtime::errors::invalid;

pub const DIVISOR: u32 = 2;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldDefines {
    pub center: f32,
    pub country_thickness: [f32; 2],
    pub country_area: [f32; 2],
    pub strategic_thickness: f32,
    pub optimization_range: f32,
    pub navy_neutral: [u8; 3],
    pub mode_thickness_max: f32,
}

impl FieldDefines {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let number = |key: &str| -> io::Result<f32> {
            let value = defines.number("NGraphics", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.NGraphics.{key} is outside f32")))
            }
        };
        let navy = defines.numbers("NGraphics", "STRATEGIC_NAVY_COLOR_NEUTRAL")?;
        let navy_neutral = match navy.as_slice() {
            [r, g, b, ..] => [*r, *g, *b].map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8),
            _ => {
                return Err(invalid(
                    "NDefines.NGraphics.STRATEGIC_NAVY_COLOR_NEUTRAL is not a colour",
                ));
            }
        };
        Ok(Self {
            center: number("GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS")?,
            country_thickness: [
                number("GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW")?,
                number("GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH")?,
            ],
            country_area: [
                number("GRADIENT_BORDERS_FIELD_COUNTRY_LOW")?,
                number("GRADIENT_BORDERS_FIELD_COUNTRY_HIGH")?,
            ],
            strategic_thickness: number("GRADIENT_BORDERS_THICKNESS_STRATEGIC_REGIONS")?,
            optimization_range: number("GRADIENT_BORDERS_OPTIMIZATION_RANGE")?,
            navy_neutral,
            mode_thickness_max: number("GRADIENT_BORDERS_THICKNESS_STATE")?
                .max(number("GRADIENT_BORDERS_THICKNESS_DIPLOMACY")?)
                .max(number("GRADIENT_BORDERS_THICKNESS_RESISTANCE")?),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const TEST: Self = Self {
        center: 2.5,
        country_thickness: [5.0, 25.0],
        country_area: [300.0, 9000.0],
        strategic_thickness: 140.0,
        optimization_range: 29.0,
        navy_neutral: [51, 63, 89],
        mode_thickness_max: 14.0,
    };
}

pub fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
    if saturation == 0.0 {
        return [value; 3];
    }
    let sixth = hue * 6.0;
    let sector = sixth.floor().min(5.0);
    let fraction = sixth - sector;
    let p = (1.0 - saturation) * value;
    let q = (1.0 - fraction * saturation) * value;
    let t = (1.0 - (1.0 - fraction) * saturation) * value;
    match (0..5).find(|&case| sector == case as f32) {
        Some(0) => [value, t, p],
        Some(1) => [q, value, p],
        Some(2) => [p, value, t],
        Some(3) => [p, q, value],
        Some(4) => [t, p, value],
        _ => [value, p, q],
    }
}
