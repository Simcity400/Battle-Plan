use crate::compose::{
    measure_to_pixels_for_parent, resolve_element_rect, ComposeOptions, Orientation, Rect,
};
use crate::gui::{parse_measure_pair, GuiElement, MeasurePair};
use crate::paradox_text::{first_scalar_any, scalar_f32, scalar_i32, Value};
use crate::scrollbar_law::{
    extended_thumb_pixel, legacy_thumb_rect, ExtendedScrollbarCarrier, LegacyScrollbarCarrier,
};
use crate::{GameUiError, Result};

pub use crate::scrollbar_law::ScrollbarAxis;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListFlow {
    Vertical,
    Horizontal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListBoxLayout {
    pub rect: Rect,
    pub orientation: Orientation,
    pub flow: ListFlow,
    pub spacing: i32,
    pub scrollbar_type: Option<String>,
    pub scrollbar_offset: (i32, i32),
    pub scrollbar_side: Option<ScrollbarSide>,
}

impl ListBoxLayout {
    pub fn declared_row_rect(&self, index: usize, row_size: (i32, i32)) -> Result<Rect> {
        if row_size.0 <= 0 || row_size.1 <= 0 {
            return Err(untraced(
                "listbox row size is runtime/controller-provided; pass a positive file-derived template size",
            ));
        }
        let step = match self.flow {
            ListFlow::Vertical => (0, row_size.1 + self.spacing),
            ListFlow::Horizontal => (row_size.0 + self.spacing, 0),
        };
        let index = i32::try_from(index)
            .map_err(|_| untraced("listbox row index overflows pixel space"))?;
        let x = step
            .0
            .checked_mul(index)
            .and_then(|v| self.rect.x().checked_add(v))
            .ok_or_else(|| untraced("listbox row index overflows pixel space"))?;
        let y = step
            .1
            .checked_mul(index)
            .and_then(|v| self.rect.y().checked_add(v))
            .ok_or_else(|| untraced("listbox row index overflows pixel space"))?;
        Ok(Rect::new(x, y, row_size.0, row_size.1))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridBoxLayout {
    pub rect: Rect,
    pub format: GridBoxFormat,
    pub slot_size: (i32, i32),
    pub max_slots_x: Option<usize>,
    pub max_slots_y: Option<usize>,
    pub add_horizontal: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridBoxFormat {
    Up,
    Down,
    Left,
    Right,
    UpperLeft,
    LowerLeft,
    UpperRight,
}

impl GridBoxFormat {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw
            .map(str::trim)
            .unwrap_or("")
            .to_ascii_uppercase()
            .as_str()
        {
            "DOWN" => Self::Down,
            "LEFT" => Self::Left,
            "RIGHT" => Self::Right,
            "UPPER_LEFT" => Self::UpperLeft,
            "LOWER_LEFT" => Self::LowerLeft,
            "UPPER_RIGHT" => Self::UpperRight,
            _ => Self::Up,
        }
    }
}

impl GridBoxLayout {
    pub fn slot_rect(&self, index: usize) -> Result<Rect> {
        if self.format != GridBoxFormat::UpperLeft {
            return Err(untraced(
                "gridbox slot anchoring is only ported for format=UPPER_LEFT; other formats are parsed but not laid out",
            ));
        }
        let columns = self.max_slots_x.ok_or_else(|| {
            untraced("gridbox slot_rect requires max_slots.x or max_slots_horizontal")
        })?;
        if columns == 0 {
            return Err(untraced("gridbox max_slots.x is zero"));
        }
        if let Some(rows) = self.max_slots_y {
            if index >= columns * rows {
                return Err(untraced("gridbox slot index exceeds declared max_slots"));
            }
        }
        let col = i32::try_from(index % columns)
            .map_err(|_| untraced("gridbox slot index overflows pixel space"))?;
        let row = i32::try_from(index / columns)
            .map_err(|_| untraced("gridbox slot index overflows pixel space"))?;
        let x = self
            .slot_size
            .0
            .checked_mul(col)
            .and_then(|v| self.rect.x().checked_add(v))
            .ok_or_else(|| untraced("gridbox slot index overflows pixel space"))?;
        let y = self
            .slot_size
            .1
            .checked_mul(row)
            .and_then(|v| self.rect.y().checked_add(v))
            .ok_or_else(|| untraced("gridbox slot index overflows pixel space"))?;
        Ok(Rect::new(x, y, self.slot_size.0, self.slot_size.1))
    }

    pub fn production_variable_height_slot_rects(
        &self,
        child_sizes: &[(i32, i32)],
    ) -> Result<Vec<Rect>> {
        let mut logical_y = 0i32;
        let mut rects = Vec::with_capacity(child_sizes.len());
        for &(width, height) in child_sizes {
            if width <= 0 || height <= 0 {
                return Err(untraced(
                    "production variable-height grid requires positive file-derived child sizes",
                ));
            }
            let (x, y) = self.explicit_coordinate_origin((0, logical_y))?;
            rects.push(Rect::new(x, y, width, height));
            logical_y = logical_y
                .checked_add(height)
                .ok_or_else(|| untraced("production variable-height grid overflows pixel space"))?;
        }
        Ok(rects)
    }

    pub fn explicit_coordinate_origin(&self, pair: (i32, i32)) -> Result<(i32, i32)> {
        let (slot_w, slot_h) = self.slot_size;
        let (grid_w, grid_h) = (self.rect.w(), self.rect.h());
        let mul = |left: i32, right: i32| {
            left.checked_mul(right)
                .ok_or_else(|| untraced("gridbox explicit coordinate overflows pixel space"))
        };
        let add = |left: i32, right: i32| {
            left.checked_add(right)
                .ok_or_else(|| untraced("gridbox explicit coordinate overflows pixel space"))
        };
        let sub = |left: i32, right: i32| {
            left.checked_sub(right)
                .ok_or_else(|| untraced("gridbox explicit coordinate overflows pixel space"))
        };
        let next_x = pair
            .0
            .checked_add(1)
            .ok_or_else(|| untraced("gridbox explicit coordinate overflows pixel space"))?;
        let next_y = pair
            .1
            .checked_add(1)
            .ok_or_else(|| untraced("gridbox explicit coordinate overflows pixel space"))?;
        let (relative_x, relative_y) = match self.format {
            GridBoxFormat::Up => (
                add(sub(grid_w, slot_w)? / 2, mul(slot_w, pair.0)?)?,
                mul(slot_h, pair.1)?,
            ),
            GridBoxFormat::Down => (
                add(sub(grid_w, slot_w)? / 2, mul(slot_w, pair.0)?)?,
                sub(grid_h, mul(slot_h, next_y)?)?,
            ),
            GridBoxFormat::Left => (
                mul(slot_w, pair.1)?,
                add(sub(grid_h, slot_h)? / 2, mul(slot_h, pair.0)?)?,
            ),
            GridBoxFormat::Right => (
                sub(grid_w, mul(slot_w, next_y)?)?,
                add(sub(grid_h, slot_h)? / 2, mul(slot_h, pair.0)?)?,
            ),
            GridBoxFormat::UpperLeft => (mul(slot_w, pair.0)?, mul(slot_h, pair.1)?),
            GridBoxFormat::LowerLeft => (
                mul(slot_w, pair.0)?,
                sub(grid_h, mul(slot_h, next_y)?)?,
            ),
            GridBoxFormat::UpperRight => (
                sub(grid_w, mul(slot_w, next_x)?)?,
                mul(slot_h, pair.1)?,
            ),
        };
        Ok((
            add(self.rect.x(), relative_x)?,
            add(self.rect.y(), relative_y)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarFamily {
    Legacy,
    Extended,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollbarLayout {
    pub rect: Rect,
    pub family: ScrollbarFamily,
    pub axis: ScrollbarAxis,
    pub border_size: Option<(i32, i32)>,
    pub min_value: Option<f32>,
    pub max_value: Option<f32>,
    pub step_size: Option<f32>,
    pub start_value: Option<f32>,
    pub slider: Option<ScrollbarPart>,
    pub track: Option<ScrollbarPart>,
    pub left_button: Option<ScrollbarPart>,
    pub right_button: Option<ScrollbarPart>,
}

impl ScrollbarLayout {
    pub fn thumb_rect_for_value(&self, _value: f32) -> Result<Rect> {
        Err(untraced(
            "scrollbar thumb travel/value mapping is not pinned by the captured CScrollbarType evidence",
        ))
    }

    pub fn thumb_rect_from_legacy_carrier(&self, carrier: LegacyScrollbarCarrier) -> Result<Rect> {
        if self.family != ScrollbarFamily::Legacy || self.axis != carrier.axis {
            return Err(untraced(
                "legacy scrollbar carrier does not match authored family/axis",
            ));
        }
        legacy_thumb_rect(carrier)
    }

    pub fn thumb_pixel_from_extended_carrier(
        &self,
        carrier: ExtendedScrollbarCarrier,
    ) -> Result<(i32, i32)> {
        if self.family != ScrollbarFamily::Extended || self.axis != carrier.axis {
            return Err(untraced(
                "extended scrollbar carrier does not match authored family/axis",
            ));
        }
        extended_thumb_pixel(carrier)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrollbarPart {
    pub name: String,
    pub rect: Rect,
}

pub fn listbox_layout(
    element: &GuiElement,
    parent: Rect,
    options: &ComposeOptions,
) -> Result<ListBoxLayout> {
    if !is_listbox(&element.element_type) {
        return Err(GameUiError::parse(format!(
            "{} is not a listbox type",
            element.element_type
        )));
    }
    let rect = resolve_element_rect(element, parent, (0, 0), options);
    let orientation = Orientation::parse(element.orientation.as_deref());
    let flow = if boolish_i32(&element.raw, "horizontal").unwrap_or(false) {
        ListFlow::Horizontal
    } else {
        ListFlow::Vertical
    };
    let spacing = scalar_i32_ci(&element.raw, "spacing").unwrap_or(0);
    let scrollbar_type =
        first_scalar_any(&element.raw, &["scrollbartype", "scrollbarType"]).map(str::to_string);
    let scrollbar_offset = element
        .raw
        .first_value_ci("offset")
        .and_then(parse_measure_pair)
        .map(|pair| pair_to_pixels(&pair, rect))
        .unwrap_or((0, 0));
    let scrollbar_side = first_scalar_any(&element.raw, &["scrollbar_side", "scrollbarSide"])
        .and_then(parse_scrollbar_side);

    Ok(ListBoxLayout {
        rect,
        orientation,
        flow,
        spacing,
        scrollbar_type,
        scrollbar_offset,
        scrollbar_side,
    })
}

pub fn gridbox_layout(
    element: &GuiElement,
    parent: Rect,
    options: &ComposeOptions,
) -> Result<GridBoxLayout> {
    if !element.element_type.eq_ignore_ascii_case("gridBoxType") {
        return Err(GameUiError::parse(format!(
            "{} is not a gridBoxType",
            element.element_type
        )));
    }
    let rect = resolve_element_rect(element, parent, (0, 0), options);
    let slot_size = element
        .raw
        .first_value_ci("slotsize")
        .and_then(parse_measure_pair)
        .map(|pair| pair_to_pixels(&pair, rect))
        .ok_or_else(|| untraced("gridbox slotsize missing"))?;
    if slot_size.0 <= 0 || slot_size.1 <= 0 {
        return Err(untraced("gridbox slotsize resolves to a non-positive size"));
    }

    let max_pair = element.raw.first_value_ci("max_slots");
    let max_slots_x = scalar_usize_ci(&element.raw, "max_slots_horizontal")
        .or_else(|| pair_scalar_usize(max_pair, &["x", "width"]));
    let max_slots_y = scalar_usize_ci(&element.raw, "max_slots_vertical")
        .or_else(|| pair_scalar_usize(max_pair, &["y", "height"]));
    let format = GridBoxFormat::parse(element.raw.first_scalar_ci("format"));
    let add_horizontal = boolish_i32(&element.raw, "add_horizontal");

    Ok(GridBoxLayout {
        rect,
        format,
        slot_size,
        max_slots_x,
        max_slots_y,
        add_horizontal,
    })
}

pub fn scrollbar_layout(
    element: &GuiElement,
    parent: Rect,
    options: &ComposeOptions,
) -> Result<ScrollbarLayout> {
    if !is_scrollbar(&element.element_type) {
        return Err(GameUiError::parse(format!(
            "{} is not a scrollbar type",
            element.element_type
        )));
    }
    let rect = resolve_element_rect(element, parent, (0, 0), options);
    let axis = if boolish_i32(&element.raw, "horizontal").unwrap_or(false) {
        ScrollbarAxis::Horizontal
    } else {
        ScrollbarAxis::Vertical
    };
    let border_size = element
        .raw
        .first_value_ci("borderSize")
        .and_then(parse_measure_pair)
        .map(|pair| pair_to_pixels(&pair, rect));

    let slider_name = first_scalar_any(&element.raw, &["slider"]).map(str::to_string);
    let track_name = first_scalar_any(&element.raw, &["track"]).map(str::to_string);
    let left_name =
        first_scalar_any(&element.raw, &["leftbutton", "leftButton"]).map(str::to_string);
    let right_name =
        first_scalar_any(&element.raw, &["rightbutton", "rightButton"]).map(str::to_string);

    Ok(ScrollbarLayout {
        rect,
        family: if element
            .element_type
            .eq_ignore_ascii_case("extendedScrollbarType")
        {
            ScrollbarFamily::Extended
        } else {
            ScrollbarFamily::Legacy
        },
        axis,
        border_size,
        min_value: scalar_f32_ci(&element.raw, "minValue"),
        max_value: scalar_f32_ci(&element.raw, "maxValue"),
        step_size: scalar_f32_ci(&element.raw, "stepSize"),
        start_value: scalar_f32_ci(&element.raw, "startValue"),
        slider: part(element, rect, options, slider_name.as_deref()),
        track: part(element, rect, options, track_name.as_deref()),
        left_button: part(element, rect, options, left_name.as_deref()),
        right_button: part(element, rect, options, right_name.as_deref()),
    })
}

fn part(
    element: &GuiElement,
    parent: Rect,
    options: &ComposeOptions,
    wanted_name: Option<&str>,
) -> Option<ScrollbarPart> {
    let wanted_name = wanted_name?;
    let child = element
        .children
        .iter()
        .find(|child| child.name.eq_ignore_ascii_case(wanted_name))?;
    let rect = resolve_element_rect(child, parent, (0, 0), options);
    Some(ScrollbarPart {
        name: child.name.clone(),
        rect,
    })
}

fn pair_to_pixels(pair: &MeasurePair, parent: Rect) -> (i32, i32) {
    (
        measure_to_pixels_for_parent(&pair.x, parent.w()),
        measure_to_pixels_for_parent(&pair.y, parent.h()),
    )
}

fn is_listbox(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "listboxtype" | "smoothlistboxtype"
    )
}

fn is_scrollbar(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "scrollbartype" | "extendedscrollbartype"
    )
}

fn boolish_i32(value: &Value, key: &str) -> Option<bool> {
    let raw = value.first_scalar_ci(key)?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => raw.parse::<i32>().ok().map(|v| v != 0),
    }
}

fn parse_scrollbar_side(raw: &str) -> Option<ScrollbarSide> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "left" => Some(ScrollbarSide::Left),
        "right" => Some(ScrollbarSide::Right),
        _ => None,
    }
}

fn scalar_i32_ci(value: &Value, key: &str) -> Option<i32> {
    value.first_value_ci(key).and_then(scalar_i32)
}

fn scalar_f32_ci(value: &Value, key: &str) -> Option<f32> {
    value.first_value_ci(key).and_then(scalar_f32)
}

fn scalar_usize_ci(value: &Value, key: &str) -> Option<usize> {
    scalar_i32_ci(value, key).and_then(|v| usize::try_from(v).ok())
}

fn pair_scalar_usize(value: Option<&Value>, keys: &[&str]) -> Option<usize> {
    let value = value?;
    first_scalar_any(value, keys)?.parse::<usize>().ok()
}

fn untraced(message: &str) -> GameUiError {
    GameUiError::new(crate::ErrorKind::Other, format!("UNTRACED: {message}"))
}
