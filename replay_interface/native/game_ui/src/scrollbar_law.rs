
use crate::compose::Rect;
use crate::{ErrorKind, GameUiError, Result};

const FIXED_SCALE_F32: f32 = 100_000.0;
const SMOOTH_SNAP_DISTANCE: i64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarCarrierAuthority {
    ExactA729Runtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtendedScrollbarCarrier {
    pub authority: ScrollbarCarrierAuthority,
    pub axis: ScrollbarAxis,
    pub min_fixed: i64,
    pub max_fixed: i64,
    pub value_fixed: i64,
    pub base_pixel: (i32, i32),
    pub travel_pixels: (i32, i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtendedScrollbarInverseCarrier {
    pub authority: ScrollbarCarrierAuthority,
    pub axis: ScrollbarAxis,
    pub min_fixed: i64,
    pub max_fixed: i64,
    pub base_pixel: (i32, i32),
    pub travel_pixels: (i32, i32),
    pub desired_pixel: (i32, i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegacyScrollbarCarrier {
    pub authority: ScrollbarCarrierAuthority,
    pub axis: ScrollbarAxis,
    pub min_fixed: i64,
    pub max_fixed: i64,
    pub value_f32: f32,
    pub base_pixel_f32: (f32, f32),
    pub leading_axis_position: i32,
    pub trailing_axis_position: i32,
    pub leading_axis_size: i32,
    pub fixed_thumb_size: (i32, i32),
}

pub fn extended_thumb_pixel(input: ExtendedScrollbarCarrier) -> Result<(i32, i32)> {
    validate_ordered_value(input.min_fixed, input.max_fixed, input.value_fixed)?;

    let min_f = input.min_fixed as f32 / FIXED_SCALE_F32;
    let range_f = input.max_fixed as f32 / FIXED_SCALE_F32 - min_f;
    let mut t = 0.0_f32;
    if range_f != 0.0 || range_f.is_nan() {
        t = (input.value_fixed as f32 / FIXED_SCALE_F32 - min_f) / range_f;
    }
    if !t.is_finite() {
        return Err(fail_closed(
            "extended scrollbar mapper produced a non-finite fraction",
        ));
    }

    match input.axis {
        ScrollbarAxis::Horizontal => {
            let delta = checked_half_away((input.travel_pixels.0 as f32) * t)?;
            Ok((
                checked_add_i32(input.base_pixel.0, delta)?,
                input.base_pixel.1,
            ))
        }
        ScrollbarAxis::Vertical => {
            let delta = checked_half_away((input.travel_pixels.1 as f32) * t)?;
            Ok((
                input.base_pixel.0,
                checked_add_i32(input.base_pixel.1, delta)?,
            ))
        }
    }
}

pub fn extended_value_for_pixel(input: ExtendedScrollbarInverseCarrier) -> Result<i64> {
    if input.max_fixed < input.min_fixed {
        return Err(fail_closed("extended scrollbar inverse range is reversed"));
    }
    let (base, travel, desired) = match input.axis {
        ScrollbarAxis::Horizontal => (
            input.base_pixel.0,
            input.travel_pixels.0,
            input.desired_pixel.0,
        ),
        ScrollbarAxis::Vertical => (
            input.base_pixel.1,
            input.travel_pixels.1,
            input.desired_pixel.1,
        ),
    };
    if input.max_fixed == input.min_fixed || travel == 0 {
        return Ok(input.min_fixed);
    }
    let travel_end = checked_add_i32(base, travel)?;
    let desired = desired.clamp(base.min(travel_end), base.max(travel_end));
    let mapped = |value_fixed: i64| -> Result<i32> {
        let pixel = extended_thumb_pixel(ExtendedScrollbarCarrier {
            authority: input.authority,
            axis: input.axis,
            min_fixed: input.min_fixed,
            max_fixed: input.max_fixed,
            value_fixed,
            base_pixel: input.base_pixel,
            travel_pixels: input.travel_pixels,
        })?;
        Ok(match input.axis {
            ScrollbarAxis::Horizontal => pixel.0,
            ScrollbarAxis::Vertical => pixel.1,
        })
    };
    let mut low = input.min_fixed;
    let mut high = input.max_fixed;
    let ordered = |pixel: i32| {
        if travel > 0 {
            i64::from(pixel)
        } else {
            -i64::from(pixel)
        }
    };
    let desired_ordered = ordered(desired);
    while low < high {
        let mid = low + (high - low) / 2;
        if ordered(mapped(mid)?) < desired_ordered {
            low = mid
                .checked_add(1)
                .ok_or_else(|| fail_closed("extended scrollbar inverse overflow"))?;
        } else {
            high = mid;
        }
    }
    let upper = low;
    let lower = upper.saturating_sub(1).max(input.min_fixed);
    let upper_distance = i64::from(mapped(upper)?).abs_diff(i64::from(desired));
    let lower_distance = i64::from(mapped(lower)?).abs_diff(i64::from(desired));
    Ok(if lower_distance <= upper_distance {
        lower
    } else {
        upper
    })
}

pub fn legacy_thumb_rect(input: LegacyScrollbarCarrier) -> Result<Rect> {
    if input.max_fixed <= input.min_fixed {
        return Err(fail_closed(
            "legacy scrollbar requires caller-established nonzero ordered range",
        ));
    }
    if !input.value_f32.is_finite()
        || !input.base_pixel_f32.0.is_finite()
        || !input.base_pixel_f32.1.is_finite()
    {
        return Err(fail_closed(
            "legacy scrollbar carrier contains non-finite f32 state",
        ));
    }
    let min_f = input.min_fixed as f32 / FIXED_SCALE_F32;
    let max_f = input.max_fixed as f32 / FIXED_SCALE_F32;
    if input.value_f32 < min_f || input.value_f32 > max_f {
        return Err(fail_closed(
            "legacy scrollbar current value was not caller-clamped",
        ));
    }
    if input.leading_axis_size < 0 || input.fixed_thumb_size.0 <= 0 || input.fixed_thumb_size.1 <= 0
    {
        return Err(fail_closed(
            "legacy scrollbar resolved child sizes are unavailable",
        ));
    }

    let thumb_axis_size = match input.axis {
        ScrollbarAxis::Horizontal => input.fixed_thumb_size.0,
        ScrollbarAxis::Vertical => input.fixed_thumb_size.1,
    };
    let travel = input
        .trailing_axis_position
        .checked_sub(input.leading_axis_position)
        .and_then(|v| v.checked_sub(input.leading_axis_size))
        .and_then(|v| v.checked_sub(thumb_axis_size))
        .ok_or_else(|| fail_closed("legacy scrollbar travel overflow"))?;

    let relative = (travel as f32) * ((input.value_f32 - min_f) / (max_f - min_f));
    let (x, y) = match input.axis {
        ScrollbarAxis::Horizontal => (input.base_pixel_f32.0 + relative, input.base_pixel_f32.1),
        ScrollbarAxis::Vertical => (input.base_pixel_f32.0, input.base_pixel_f32.1 + relative),
    };
    Ok(Rect::new(
        checked_truncate(x)?,
        checked_truncate(y)?,
        input.fixed_thumb_size.0,
        input.fixed_thumb_size.1,
    ))
}

pub fn extended_displayed_value(
    displayed: i64,
    target: i64,
    smooth_factor_fixed: i64,
    slider_animation_enabled: bool,
) -> Result<i64> {
    let delta = target
        .checked_sub(displayed)
        .ok_or_else(|| fail_closed("extended scrollbar smooth delta overflow"))?;
    let distance = delta
        .checked_abs()
        .ok_or_else(|| fail_closed("extended scrollbar smooth distance overflow"))?;
    if !slider_animation_enabled || distance <= SMOOTH_SNAP_DISTANCE {
        return Ok(target);
    }
    let step = delta
        .checked_mul(smooth_factor_fixed)
        .ok_or_else(|| fail_closed("extended scrollbar smooth multiply overflow"))?
        / 100_000;
    displayed
        .checked_add(step)
        .ok_or_else(|| fail_closed("extended scrollbar smooth accumulation overflow"))
}

fn validate_ordered_value(minimum: i64, maximum: i64, value: i64) -> Result<()> {
    if maximum < minimum {
        return Err(fail_closed("scrollbar range endpoints are reversed"));
    }
    if value < minimum || value > maximum {
        return Err(fail_closed("scrollbar value is not setter-clamped"));
    }
    Ok(())
}

fn checked_half_away(value: f32) -> Result<i32> {
    let rounded = value.round();
    if !rounded.is_finite() || rounded < -2_147_483_648.0 || rounded >= 2_147_483_648.0 {
        return Err(fail_closed("scrollbar rounded pixel offset is outside i32"));
    }
    Ok(rounded as i32)
}

fn checked_truncate(value: f32) -> Result<i32> {
    let truncated = value.trunc();
    if !truncated.is_finite() || truncated < -2_147_483_648.0 || truncated >= 2_147_483_648.0 {
        return Err(fail_closed(
            "legacy scrollbar truncated coordinate is outside i32",
        ));
    }
    Ok(truncated as i32)
}

fn checked_add_i32(lhs: i32, rhs: i32) -> Result<i32> {
    lhs.checked_add(rhs)
        .ok_or_else(|| fail_closed("scrollbar pixel coordinate overflow"))
}

fn fail_closed(message: &str) -> GameUiError {
    GameUiError::new(ErrorKind::Other, format!("UI06_FAIL_CLOSED: {message}"))
}
