
use super::*;

type Box2 = ([i32; 2], [i32; 2]);

pub(super) enum PartMiss<'b> {
    NoParent,
    ParentKind(&'b Element),
    Neither(&'b Element),
}

pub(super) fn scrollbar_part<'b>(
    element: &'b Element,
    bar: &native::Scrollbar,
    name: &[u8],
    gui_type: &dyn Fn(&[u8]) -> Option<&'b Element>,
) -> Result<&'b Element, PartMiss<'b>> {
    let name = c_string(name);
    if !name.is_empty()
        && let Some(own) = element.child_named(name)
    {
        return Ok(own);
    }
    let parent = c_string(&bar.parent);
    let Some(template) = (!parent.is_empty()).then(|| gui_type(parent)).flatten() else {
        return Err(PartMiss::NoParent);
    };
    if !matches!(template.kind, Kind::Scrollbar | Kind::Window) {
        return Err(PartMiss::ParentKind(template));
    }
    template
        .child_named(name)
        .ok_or(PartMiss::Neither(template))
}

pub(super) fn scrollbar_part_names(
    bar: &native::Scrollbar,
) -> Vec<(&[u8], &'static str, &'static str)> {
    let mut names = vec![
        (
            bar.right_button.as_slice(),
            "rightButton",
            "button construction",
        ),
        (
            bar.left_button.as_slice(),
            "leftButton",
            "button construction",
        ),
        (bar.track.as_slice(), "track", "track construction"),
        (bar.slider.as_slice(), "slider", "slider construction"),
    ];
    for (name, what) in [
        (&bar.range_limit_min_icon, "rangeLimitMinIcon"),
        (&bar.range_limit_max_icon, "rangeLimitMaxIcon"),
    ] {
        if !c_string(name).is_empty() {
            names.push((name.as_slice(), what, "button construction"));
        }
    }
    names
}

use content_reader_parser::parser::c_string;

pub(super) fn fixed(value: f32) -> i64 {
    let bias = if value >= 0.0 { 1.0f32 } else { 0.0 };
    let scaled = bias + value * 100_000.0 - 0.5;
    if scaled.is_nan() || scaled >= i64::MAX as f32 || scaled < i64::MIN as f32 {
        i64::MIN
    } else {
        scaled as i64
    }
}

pub(super) fn scroll_pixels(value: i64) -> i32 {
    truncate_f32(value as f32 / 100_000.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Range {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub count: i32,
}

impl Range {
    pub fn new(min: i64, max: i64, step: i64) -> Self {
        let span = max.wrapping_sub(min);
        let quotient = if step == 0 || [i64::MAX, i64::MIN, i64::MIN + 1].contains(&span) {
            0xFFFF_FFFF
        } else {
            span.wrapping_mul(100_000).wrapping_div(step)
        };
        Self {
            min,
            max,
            step,
            count: ((quotient / 100_000) as i32).wrapping_add(1),
        }
    }

    pub fn snap(self, value: i64) -> i64 {
        if value >= self.max {
            return self.max;
        }
        if value <= self.min {
            return self.min;
        }
        if self.step == 100 {
            return value;
        }
        let mut best = self.min;
        let mut distance = value.wrapping_sub(self.min).wrapping_abs();
        let mut point = self.min;
        for _ in 0..self.count.max(0) {
            let own = value.wrapping_sub(point).wrapping_abs();
            if own < distance {
                best = point;
                distance = own;
            }
            point = point.wrapping_add(self.step);
        }
        best
    }

    pub fn clamp(self, value: i64) -> i64 {
        if value < self.min {
            self.min
        } else if value > self.max {
            self.max
        } else {
            value
        }
    }

    pub fn thumb(self, value: i64, travel: i32) -> i32 {
        let scaled = |value: i64| value as f32 / 100_000.0;
        let low = scaled(self.min);
        let span = scaled(self.max) - low;
        let t = if span != 0.0 {
            (scaled(value) - low) / span
        } else {
            0.0
        };
        let offset = travel as f32 * t;
        truncate_f32(if offset >= 0.0 {
            offset + 0.5
        } else {
            offset - 0.5
        })
    }
}

fn part(part: &Option<Box<Element>>) -> Option<(&Element, &native::Button)> {
    part.as_deref()
        .and_then(|element| Some((element, element.button()?)))
}

fn parse_gui_orientation(text: &[u8]) -> Orientation {
    [
        Orientation::UpperRight,
        Orientation::LowerLeft,
        Orientation::LowerRight,
        Orientation::Center,
        Orientation::CenterUp,
        Orientation::CenterDown,
    ]
    .into_iter()
    .find(|own| text.eq_ignore_ascii_case(own.name().as_bytes()))
    .unwrap_or_default()
}

fn list_box_anchor(orientation: Orientation, viewport: [i32; 2]) -> [i32; 2] {
    let [w, h] = viewport;
    match orientation.gui_value() {
        1 => [w, 0],
        2 => [0, h],
        3 => [w, h],
        4 => [w / 2, h / 2],
        5 => [w / 2, 0],
        6 => [w / 2, h],
        _ => [0, 0],
    }
}

fn float(value: i32) -> f32 {
    value as f32
}

#[derive(Clone, Copy)]
pub(super) enum Knob<'a> {
    Button(&'a Element, &'a native::Button),
    Icon(&'a Element, &'a native::Icon),
}

impl<'a> Knob<'a> {
    fn element(self) -> &'a Element {
        match self {
            Self::Button(element, _) | Self::Icon(element, _) => element,
        }
    }
}

type Drag<'a> = (&'a Element, &'a native::Button);

pub(super) struct Plain<'a> {
    element: &'a Element,
    bar: &'a native::Scrollbar,
    right: Knob<'a>,
    left: Knob<'a>,
    track: Drag<'a>,
    slider: Drag<'a>,
    icons: [Option<Knob<'a>>; 2],
    horizontal: bool,
    size: [i32; 2],
    origin: [f32; 2],
    right_at: [i32; 2],
    left_at: [i32; 2],
    track_at: [i32; 2],
    track_size: [i32; 2],
    offset: [f32; 2],
    pub(super) range: Range,
    pub(super) value: f32,
    limits: [i64; 2],
    limited: bool,
    extents: [[i32; 2]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PlainState {
    range: Range,
    value: f32,
    limits: [i64; 2],
}

impl<'a> Plain<'a> {
    pub(super) fn state(&self) -> PlainState {
        PlainState {
            range: self.range,
            value: self.value,
            limits: self.limits,
        }
    }

    pub(super) fn restore(&mut self, state: PlainState) {
        self.range = state.range;
        self.value = state.value;
        self.limits = state.limits;
        self.place_slider();
    }

    pub(super) fn element(&self) -> &'a Element {
        self.element
    }

    pub(super) fn parts(&self) -> [Option<&'a Element>; 6] {
        let (Knob::Button(right, _) | Knob::Icon(right, _)) = self.right;
        let (Knob::Button(left, _) | Knob::Icon(left, _)) = self.left;
        let icon = |index: usize| self.icons[index].map(Knob::element);
        [
            Some(right),
            Some(left),
            Some(self.track.0),
            Some(self.slider.0),
            icon(0),
            icon(1),
        ]
    }

    fn lay(&mut self, origin: [f32; 2]) {
        let [px, py] = origin;
        let [sx, sy] = self.size;
        let r = self.extents[0];
        if !self.horizontal {
            self.track_at = [truncate_f32(px), truncate_f32(float(r[1]) + py)];
            self.left_at = [
                truncate_f32(px),
                truncate_f32((float(sy) + py) - float(r[1])),
            ];
            self.right_at = [truncate_f32(px), truncate_f32(py)];
            self.track_size = [sx, sy.wrapping_sub(r[1].wrapping_mul(2))];
            self.track_at[0] = self.track_at[0].wrapping_add((r[0] / 2).wrapping_sub(sx / 2));
        } else {
            self.track_at = [truncate_f32(float(r[0]) + px), truncate_f32(py)];
            self.left_at = [truncate_f32(px), truncate_f32(py)];
            self.right_at = [
                truncate_f32((float(sx) + px) - float(r[0])),
                truncate_f32(py),
            ];
            self.track_size = [sx.wrapping_sub(r[0].wrapping_mul(2)), sy];
        }
    }

    fn slide(&self) -> Slide {
        Slide {
            vertical: !self.horizontal,
            travel: self.travel(),
            thumb: truncate_f32(self.slid()),
        }
    }

    fn travel(&self) -> i32 {
        let [r, l, s] = self.extents;
        if !self.horizontal {
            self.left_at[1]
                .wrapping_sub(self.right_at[1])
                .wrapping_sub(r[1])
                .wrapping_sub(s[1])
        } else {
            self.right_at[0]
                .wrapping_sub(s[0])
                .wrapping_sub(self.left_at[0])
                .wrapping_sub(l[0])
        }
    }

    fn slid(&self) -> f32 {
        let travel = self.travel();
        let low = self.range.min as f32 / 100_000.0;
        let high = self.range.max as f32 / 100_000.0;
        float(travel) * ((self.value - low) / (high - low))
    }

    pub(super) fn place_slider(&mut self) {
        let slid = self.slid();
        if !self.horizontal {
            self.offset[1] = slid;
        } else {
            self.offset[0] = slid;
        }
    }

    fn slider_at(&self) -> [i32; 2] {
        let [r, l, _] = self.extents;
        let shift = self.bar.slider_pos_shift();
        let [x, y] = if !self.horizontal {
            [
                truncate_f32(self.offset[0]),
                truncate_f32((float(self.right_at[1]) + self.offset[1]) + float(r[1])),
            ]
        } else {
            [
                truncate_f32((float(self.left_at[0]) + self.offset[0]) + float(l[0])),
                truncate_f32(self.offset[1]),
            ]
        };
        [shift[0].wrapping_add(x), shift[1].wrapping_add(y)]
    }

    pub(super) fn resize(&mut self, width: i32, height: i32) {
        let saved = self.origin;
        self.origin = [0.0; 2];
        let [own_width, own_height] = self.size;
        if (width == -1 || width == own_width) && (height == -1 || height == own_height) {
            return;
        }
        let mut width = if width == -1 { own_width } else { width };
        let mut height = if height == -1 { own_height } else { height };
        if height < 0 {
            height = 40;
        }
        if width < 0 {
            width = 40;
        }
        self.size = [width, height];
        self.lay([0.0; 2]);
        if !self.horizontal {
            self.offset[0] = 0.0;
        } else {
            self.offset[1] = 0.0;
        }
        self.origin = saved;
        self.place_slider();
    }

    pub(super) fn move_to(&mut self, at: [i32; 2]) {
        self.origin = at.map(float);
    }

    pub(super) fn node(&self, content: [i32; 2]) -> [i32; 2] {
        [
            content[0].wrapping_add(truncate_f32(self.origin[0])),
            content[1].wrapping_add(truncate_f32(self.origin[1])),
        ]
    }

    pub(super) fn set_range(&mut self, range: Range) {
        self.range = range;
        self.limits = [range.min, range.max];
        let low = range.min as f32 / 100_000.0;
        let high = range.max as f32 / 100_000.0;
        if self.value < low {
            self.value = low;
        } else if self.value > high {
            self.value = high;
        }
        self.place_slider();
    }

    pub(super) fn set_value(&mut self, value: i64) {
        let mut value = value;
        if self.limited {
            if value > self.limits[1] {
                value = self.limits[1];
            } else if value < self.limits[0] {
                value = self.limits[0];
            }
        }
        let value = fixed(value as f32 / 100_000.0);
        self.value = self.range.snap(value) as f32 / 100_000.0;
        self.place_slider();
    }

    pub(super) fn input(&mut self, direction: i32, wheel: bool) {
        if wheel && self.range.max <= 0 {
            return;
        }
        let value = if wheel {
            let step = self.range.step as f32 / 100_000.0 * self.bar.scroll_speed as f32;
            let next = self.value + step * direction as f32;
            fixed(
                next.max(self.range.min as f32 / 100_000.0)
                    .min(self.range.max as f32 / 100_000.0),
            )
        } else {
            let step = if self.range.step == 100 {
                1000
            } else {
                self.range.step
            };
            fixed(self.value).wrapping_add(step.wrapping_mul(i64::from(direction)))
        };
        self.set_value(value);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListControl {
    Step(i32),
    Track,
    Thumb(Slide),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListBoxRows {
    pub places: Vec<Option<[i32; 2]>>,
    pub scrollbar: Vec<Part>,
    pub range: i32,
    pub rect: [i32; 4],
    pub page: i32,
    pub controls: Vec<(ListControl, Region)>,
    clipped: bool,
    pub diagnostics: Vec<Diagnostic>,
}

impl ListBoxRows {
    pub(super) fn clipped(&self) -> bool {
        self.clipped
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn smooth_list_step(current: f32) -> f32 {
    (0.0 - current) * 0.2 + current
}

struct ListBoxOutcome {
    places: Vec<Option<[i32; 2]>>,
    range: i32,
    page: i32,
    shown: bool,
    smooth: bool,
}

fn finish_list_box_rows(
    element: &Element,
    list: &native::ListBox,
    at: [i32; 2],
    bar: Option<&Plain<'_>>,
    mut walk: Walk<'_>,
    outcome: ListBoxOutcome,
) -> Result<ListBoxRows, Refusals> {
    let mut scrollbar = Vec::new();
    let mut controls = Vec::new();
    if let (true, Some(bar)) = (outcome.shown, bar) {
        let node = bar.node(at);
        let first = walk.parts.len();
        let first_region = walk.regions.len();
        let outer = walk.opacity;
        walk.opacity = 1.0;
        let drawn = walk.plain_parts(bar, element, node, None, true);
        walk.opacity = outer;
        drawn.map_err(|refusal| vec![refusal])?;
        scrollbar = walk.parts.split_off(first);
        let [_, _, track, slider, ..] = bar.parts();
        let slide = bar.slide();
        controls = walk
            .regions
            .split_off(first_region)
            .into_iter()
            .filter_map(|region| {
                let control = match region.scroll {
                    Some((_, step)) => ListControl::Step(step),
                    None if slider.is_some_and(|slider| slider.path == region.path) => {
                        ListControl::Thumb(slide)
                    }
                    None if track.is_some_and(|track| track.path == region.path) => {
                        ListControl::Track
                    }
                    None => return None,
                };
                Some((control, region))
            })
            .collect();
    }
    let refused = walk.refused.take();
    walk.refusals.extend(refused);
    if !walk.refusals.is_empty() {
        return Err(walk.refusals);
    }
    Ok(ListBoxRows {
        places: outcome.places,
        scrollbar,
        range: outcome.range,
        rect: [at[0], at[1], list.size[0], list.size[1]],
        page: outcome.page,
        controls,
        clipped: !outcome.smooth,
        diagnostics: walk.diagnostics,
    })
}

pub fn list_box_uniform_rows(
    element: &Element,
    item_size: [i32; 2],
    count: usize,
    visible: std::ops::Range<usize>,
    current: Option<f32>,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<ListBoxRows, Refusals> {
    let smooth = element.kind == Kind::SmoothListBox;
    let path = &element.path;
    let Fields::ListBox(list) = &element.fields else {
        return Err(vec![Refusal::new(
            path,
            RefusalKind::Reader,
            "is not a list box",
        )]);
    };
    let at = list_box_place(element, list, smooth, bindings.viewport)
        .map_err(|refusal| vec![refusal])?;
    let walk = Walk::new(resources, bindings);
    let mut bar = match walk.list_scrollbar(element, list, smooth) {
        Ok(bar) => bar,
        Err(refusal) => return Err(vec![refusal]),
    };
    let border = list.border_size[0];
    let start = visible.start.min(count);
    let end = visible.end.min(count).max(start);
    let mut places = Vec::with_capacity(end - start);
    let mut shown = false;
    let mut range = 0;
    let page;
    if smooth {
        let horizontal = list.horizontal != 0;
        let extent = if horizontal { list.size[0] } else { list.size[1] };
        let step = if horizontal { item_size[0] } else { item_size[1] };
        page = extent;
        let cur = current.unwrap_or(bar.as_ref().map_or(0.0, |bar| bar.value));
        let along = |own: i32| truncate_f32((float(border) - cur) + float(own));
        let across = |own: i32| truncate_f32(float(border) + float(own));
        for index in start..end {
            let run = (index as i32).wrapping_mul(step);
            let target = if horizontal { [run, 0] } else { [0, run] };
            let place = if horizontal {
                [along(target[0]), across(target[1])]
            } else {
                [across(target[0]), along(target[1])]
            };
            places.push(Some([
                at[0].wrapping_add(place[0]),
                at[1].wrapping_add(place[1]),
            ]));
        }
        if let Some(bar) = &mut bar
            && count > 0
        {
            let over = (i64::from(step) * count as i64 - i64::from(extent))
                .clamp(0, i64::from(i32::MAX)) as i32;
            if over >= 1 {
                bar.set_range(Range::new(0, i64::from(over) * 100_000, bar.range.step));
                bar.value = cur.clamp(0.0, float(over));
                bar.place_slider();
                shown = true;
                range = over;
            }
        }
    } else {
        let fit = if count == 0 || item_size[1] <= 0 {
            0
        } else {
            truncate_f32(float(list.size[1]) / float(item_size[1]))
        };
        page = fit.max(1);
        let over = (count as i32).wrapping_sub(fit).max(0);
        let first = match current {
            Some(current) if bar.is_some() && over >= 1 => truncate_f32(current).clamp(0, over),
            _ => 0,
        };
        for index in start..end {
            let idx = index as f32;
            if idx < float(first) || float(first) + float(fit) <= idx {
                places.push(None);
            } else {
                let offset = (index as i32).wrapping_sub(first);
                let y = border.wrapping_add(offset.wrapping_mul(item_size[1]));
                places.push(Some([at[0].wrapping_add(border), at[1].wrapping_add(y)]));
            }
        }
        if let Some(bar) = &mut bar
            && count > 0
        {
            bar.set_range(Range::new(0, i64::from(over) * 100_000, 100_000));
            if bar.range.max >= 100_000 {
                bar.value = float(first);
                bar.place_slider();
                shown = true;
                range = over;
            }
        }
    }
    finish_list_box_rows(
        element,
        list,
        at,
        bar.as_ref(),
        walk,
        ListBoxOutcome {
            places,
            range,
            page,
            shown,
            smooth,
        },
    )
}

pub fn list_box_rows(
    element: &Element,
    rows: &[[i32; 2]],
    current: Option<f32>,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<ListBoxRows, Refusals> {
    let smooth = element.kind == Kind::SmoothListBox;
    list_box_rows_as(element, smooth, rows, current, resources, bindings)
}

pub fn list_box_rows_as(
    element: &Element,
    smooth: bool,
    rows: &[[i32; 2]],
    current: Option<f32>,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<ListBoxRows, Refusals> {
    let path = &element.path;
    let Fields::ListBox(list) = &element.fields else {
        return Err(vec![Refusal::new(
            path,
            RefusalKind::Reader,
            "is not a list box",
        )]);
    };
    let at = list_box_place(element, list, smooth, bindings.viewport)
        .map_err(|refusal| vec![refusal])?;
    let mut walk = Walk::new(resources, bindings);
    let mut bar = match walk.list_scrollbar(element, list, smooth) {
        Ok(bar) => bar,
        Err(refusal) => return Err(vec![refusal]),
    };
    let border = list.border_size[0];
    let mut places = Vec::with_capacity(rows.len());
    let mut shown = false;
    let mut range = 0;
    let page;
    if smooth {
        let horizontal = list.horizontal != 0;
        page = if horizontal { list.size[0] } else { list.size[1] };
        let cur = current.unwrap_or(bar.as_ref().map_or(0.0, |bar| bar.value));
        let mut run = 0i32;
        for row in rows {
            let target = if horizontal { [run, 0] } else { [0, run] };
            let along = |own: i32| truncate_f32((float(border) - cur) + float(own));
            let across = |own: i32| truncate_f32(float(border) + float(own));
            let place = if horizontal {
                [along(target[0]), across(target[1])]
            } else {
                [across(target[0]), along(target[1])]
            };
            places.push(Some([
                at[0].wrapping_add(place[0]),
                at[1].wrapping_add(place[1]),
            ]));
            run = run.wrapping_add(if horizontal { row[0] } else { row[1] });
        }
        if let Some(bar) = &mut bar
            && !rows.is_empty()
        {
            let extent = if horizontal {
                list.size[0]
            } else {
                list.size[1]
            };
            let over = run.wrapping_sub(extent);
            if over >= 1 {
                bar.set_range(Range::new(0, i64::from(over) * 100_000, bar.range.step));
                bar.value = cur.clamp(0.0, float(over));
                bar.place_slider();
                shown = true;
                range = over;
            }
        }
    } else {
        let fit = match rows.last() {
            None => 0,
            Some(last) => truncate_f32(float(list.size[1]) / float(last[1])),
        };
        page = fit.max(1);
        let over = (rows.len() as i32).wrapping_sub(fit).max(0);
        let first = match current {
            Some(current) if bar.is_some() && over >= 1 => truncate_f32(current).clamp(0, over),
            _ => 0,
        };
        let mut y = border;
        for (index, row) in rows.iter().enumerate() {
            let index = index as f32;
            if index < float(first) || float(first) + float(fit) <= index {
                places.push(None);
            } else {
                places.push(Some([at[0].wrapping_add(border), at[1].wrapping_add(y)]));
                y = y.wrapping_add(row[1]);
            }
        }
        if let Some(bar) = &mut bar
            && !rows.is_empty()
        {
            bar.set_range(Range::new(0, i64::from(over) * 100_000, 100_000));
            if bar.range.max >= 100_000 {
                bar.value = float(first);
                bar.place_slider();
                shown = true;
                range = over;
            }
        }
    }
    finish_list_box_rows(
        element,
        list,
        at,
        bar.as_ref(),
        walk,
        ListBoxOutcome {
            places,
            range,
            page,
            shown,
            smooth,
        },
    )
}

pub(super) fn list_box_place(
    element: &Element,
    list: &native::ListBox,
    smooth: bool,
    viewport: Option<[i32; 2]>,
) -> Result<[i32; 2], Refusal> {
    let orientation = parse_gui_orientation(&list.orientation);
    let anchor = if orientation.gui_value() == 0 {
        [0, 0]
    } else {
        let viewport = viewport.ok_or_else(|| {
            Refusal::new(
                &element.path,
                RefusalKind::NoParent,
                "anchors its orientation to the GUI viewport at construction, which the caller did not bind (Bindings::viewport)",
            )
        })?;
        list_box_anchor(orientation, viewport)
    };
    let times = if smooth { 2 } else { 1 };
    Ok([
        list.position[0].wrapping_add(anchor[0].wrapping_mul(times)),
        list.position[1].wrapping_add(anchor[1].wrapping_mul(times)),
    ])
}

pub(super) struct ContentScroll<'a> {
    bars: Vec<(bool, &'a Element, &'a native::ExtendedScrollbar)>,
    targets: [i64; 2],
}

impl<'a> ContentScroll<'a> {
    pub(super) fn named(
        &self,
    ) -> impl Iterator<Item = (bool, &'a Element, &'a native::ExtendedScrollbar)> + '_ {
        self.bars.iter().copied()
    }
}

pub(super) struct Bar<'a> {
    pub vertical: bool,
    pub template: &'a Element,
    pub bar: &'a native::ExtendedScrollbar,
    pub shown: bool,
    pub range: Range,
    pub value: i64,
    pub track_frame: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ContainerBar {
    pub shown: bool,
    pub range: Range,
    pub value: i64,
    pub track_frame: Option<i32>,
}

pub(super) fn track_frame(
    bar: &native::ExtendedScrollbar,
    range: Range,
    value: i64,
) -> Option<i32> {
    bar.set_track_frame_on_change.then(|| {
        let scaled = |value: i64| value as f32 / 100_000.0;
        let share = scaled(value) / (scaled(range.max) - scaled(range.min)) * 100.0;
        truncate_f32(if share >= 0.0 {
            share + 0.5
        } else {
            share - 0.5
        })
    })
}

impl<'a> Walk<'a> {
    pub(super) fn new(resources: &'a dyn Resources, bindings: &'a Bindings) -> Self {
        Walk {
            resources,
            bindings,
            parts: Vec::new(),
            holders: Vec::new(),
            regions: Vec::new(),
            diagnostics: Vec::new(),
            refusals: Vec::new(),
            top: None,
            background_index: None,
            opacity: 1.0,
            hits: Vec::new(),
            hovered: None,
            boxes: Vec::new(),
            pointed: None,
            target: None,
            text_effect: None,
            sized: None,
            via: None,
            region_owner: None,
            refused: Default::default(),
        }
    }

    pub(super) fn content_scroll(
        &mut self,
        window: &Element,
        container: &native::Container,
    ) -> ContentScroll<'a> {
        let path = &window.path;
        let mut bars = Vec::new();
        let mut targets = [0i64; 2];
        for (vertical, name) in [
            (true, &container.vertical_scrollbar),
            (false, &container.horizontal_scrollbar),
        ] {
            let name = c_string(name);
            if name.is_empty() {
                continue;
            }
            match self.resources.gui_type(name) {
                Some(template) => match &template.fields {
                    Fields::ExtendedScrollbar(bar) => {
                        let own = Range::new(bar.min_value, bar.max_value, bar.step_size);
                        targets[usize::from(vertical)] = own.snap(0);
                        bars.push((vertical, self.resolved(template), bar));
                    }
                    _ => self.diagnostics.push(Diagnostic {
                        path: path.clone(),
                        kind: DiagnosticKind::MissingScrollbar,
                        detail: format!(
                            "names scrollbar {:?}, a {}: the window takes only an extendedScrollbarType and has none",
                            lossy(name),
                            template.kind.keyword()
                        ),
                    }),
                },
                None => self.diagnostics.push(Diagnostic {
                    path: path.clone(),
                    kind: DiagnosticKind::MissingScrollbar,
                    detail: format!(
                        "names scrollbar {:?}, which no guiTypes template defines: the window has none",
                        lossy(name)
                    ),
                }),
            }
        }
        ContentScroll { bars, targets }
    }

    pub(super) fn scroll_offset(&self, window: &Element, scroll: &ContentScroll) -> [i32; 2] {
        let bound = self.bound(window).map_or([None; 2], |bound| bound.scroll);
        [0, 1].map(|axis| {
            bound[axis]
                .unwrap_or_else(|| scroll_pixels(scroll.targets[axis]))
                .wrapping_neg()
        })
    }

    fn part_extent(&self, path: &str, sprite: &[u8], scale: f32) -> Option<[i32; 2]> {
        [sprite, FALLBACK_SPRITE]
            .into_iter()
            .find_map(|name| self.info(path, name))
            .map(|info| info.scaled_extent(scale))
    }

    pub(super) fn sized_extent(
        &self,
        path: &str,
        sprite: &[u8],
        sized: [i32; 2],
    ) -> Option<[i32; 2]> {
        [sprite, FALLBACK_SPRITE]
            .into_iter()
            .find_map(|name| self.info(path, name))
            .map(|info| match info.kind {
                SpriteKind::CorneredTile { .. } => sized,
                _ => info.scaled_extent(1.0),
            })
    }

    pub(super) fn drag_part(
        &mut self,
        element: &Element,
        button: &native::Button,
        frame: i32,
        at: [i32; 2],
        sized: Option<[i32; 2]>,
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        if !self.shown(element) {
            return Ok(());
        }
        self.sized = sized;
        let drawn = self.sprite_part(
            element,
            &button.sprite,
            frame,
            1.0,
            0.0,
            at,
            Centre::None,
            clip,
        );
        self.sized = None;
        drawn
    }

    fn knob_extent(&self, knob: Knob) -> Result<[i32; 2], Refusal> {
        let (Knob::Button(element, _) | Knob::Icon(element, _)) = knob;
        self.drawable_extent(element)?.ok_or_else(|| {
            Refusal::new(
                &element.path,
                RefusalKind::NullRead,
                "has no drawable (neither its sprite nor the fallback is defined), which the drawable-size query reads through",
            )
        })
    }

    fn plain_part<'b>(
        &self,
        element: &'b Element,
        bar: &native::Scrollbar,
        name: &[u8],
        what: &str,
        site: &str,
    ) -> Result<&'b Element, Refusal>
    where
        'a: 'b,
    {
        let path = &element.path;
        let resources = self.resources;
        let miss = match scrollbar_part(element, bar, name, &|name| resources.gui_type(name)) {
            Ok(part) => return Ok(self.resolved(part)),
            Err(miss) => miss,
        };
        let name = c_string(name);
        let parent = c_string(&bar.parent);
        Err(match miss {
            PartMiss::NoParent => Refusal::new(
                path,
                RefusalKind::NullRead,
                format!(
                    "names its {what} {:?}, which it does not hold, and its parent {:?} is no template: child lookup reads the null parent",
                    lossy(name),
                    lossy(parent)
                ),
            ),
            PartMiss::ParentKind(template) => Refusal::new(
                path,
                RefusalKind::TypeConfusion,
                format!(
                    "names its {what} {:?}, which it does not hold, and its parent {} is a {}, whose fields are read as a children map (only scrollbarType and windowType provide one)",
                    lossy(name),
                    template.path,
                    template.kind.keyword()
                ),
            ),
            PartMiss::Neither(template) => Refusal::new(
                path,
                RefusalKind::NullRead,
                format!(
                    "names its {what} {:?}, which neither it nor its parent {} holds: the \
                     lookup returns null, which {site} reads through",
                    lossy(name),
                    template.path
                ),
            ),
        })
    }

    fn knob<'b>(part: &'b Element) -> Result<Knob<'b>, Refusal> {
        match &part.fields {
            Fields::Button(button) => Ok(Knob::Button(part, button)),
            Fields::Icon(icon) => Ok(Knob::Icon(part, icon)),
            _ => Err(Refusal::new(
                &part.path,
                RefusalKind::TypeConfusion,
                format!(
                    "is a scrollbar's button or range icon of kind {}, whose template provides no compatible object factory",
                    part.kind.keyword()
                ),
            )),
        }
    }

    fn drag<'b>(part: &'b Element) -> Result<Drag<'b>, Refusal> {
        match &part.fields {
            Fields::Button(button) => Ok((part, button)),
            _ => Err(Refusal::new(
                &part.path,
                RefusalKind::TypeConfusion,
                format!(
                    "is a scrollbar's track or slider of kind {}, which construction reads as a button template: a type-confused read",
                    part.kind.keyword()
                ),
            )),
        }
    }

    pub(super) fn plain<'b>(
        &self,
        element: &'b Element,
        bar: &'b native::Scrollbar,
    ) -> Result<Plain<'b>, Refusal>
    where
        'a: 'b,
    {
        let path = &element.path;
        let Some(min) = bar.min_value() else {
            return Err(Refusal::new(
                path,
                RefusalKind::UndefinedRead,
                "has no minValue, leaving the scrollbar's range uninitialised; the heap block is not zeroed, so its thumb lands where stale bytes put it",
            ));
        };
        let right = Self::knob(self.plain_part(
            element,
            bar,
            &bar.right_button,
            "rightButton",
            "button construction",
        )?)?;
        let left = Self::knob(self.plain_part(
            element,
            bar,
            &bar.left_button,
            "leftButton",
            "button construction",
        )?)?;
        let track = Self::drag(self.plain_part(
            element,
            bar,
            &bar.track,
            "track",
            "track construction",
        )?)?;
        let slider = Self::drag(self.plain_part(
            element,
            bar,
            &bar.slider,
            "slider",
            "slider construction",
        )?)?;
        let mut icons = [None, None];
        for (index, (name, what)) in [
            (&bar.range_limit_min_icon, "rangeLimitMinIcon"),
            (&bar.range_limit_max_icon, "rangeLimitMaxIcon"),
        ]
        .into_iter()
        .enumerate()
        {
            if !c_string(name).is_empty() {
                icons[index] = Some(Self::knob(self.plain_part(
                    element,
                    bar,
                    name,
                    what,
                    "button construction",
                )?)?);
            }
        }
        let extents = [
            self.knob_extent(right)?,
            self.knob_extent(left)?,
            self.part_extent(&slider.0.path, &slider.1.sprite, 1.0)
                .unwrap_or([0; 2]),
        ];
        let origin = bar.position.map(float);
        let mut plain = Plain {
            element,
            bar,
            right,
            left,
            track,
            slider,
            icons,
            horizontal: bar.horizontal != 0,
            size: bar.size,
            origin,
            right_at: [0; 2],
            left_at: [0; 2],
            track_at: [0; 2],
            track_size: [0; 2],
            offset: origin,
            range: Range::new(fixed(min), fixed(bar.max_value()), fixed(bar.step_size())),
            value: bar.start_value(),
            limits: [bar.range_limit_min, bar.range_limit_max],
            limited: bar.use_range_limit,
            extents,
        };
        plain.lay(origin);
        Ok(plain)
    }

    pub(super) fn icon_places(&self, plain: &Plain) -> Result<[Option<[i32; 2]>; 2], Refusal> {
        let mut places = [None, None];
        if !plain.limited {
            return Ok(places);
        }
        let s = plain.extents[2];
        let centre = [
            (s[0] / 2).wrapping_add(plain.track_at[0]),
            (s[1] / 2).wrapping_add(plain.track_at[1]),
        ];
        let track = self
            .sized_extent(&plain.track.0.path, &plain.track.1.sprite, plain.track_size)
            .unwrap_or([0; 2]);
        let room = [track[0].wrapping_sub(s[0]), track[1].wrapping_sub(s[1])];
        let span = plain.range.max.wrapping_sub(plain.range.min);
        for (index, (limit, end)) in [
            (plain.limits[0], plain.range.min),
            (plain.limits[1], plain.range.max),
        ]
        .into_iter()
        .enumerate()
        {
            if limit == end {
                continue;
            }
            let share = limit.wrapping_sub(plain.range.min);
            let scaled = share.wrapping_mul(100_000);
            let quotient = if span == 0 || [i64::MAX, i64::MIN, i64::MIN + 1].contains(&share) {
                0xFFFF_FFFF
            } else if scaled == i64::MIN && span == -1 {
                return Err(Refusal::new(
                    &plain.track.0.path,
                    RefusalKind::DivideByZero,
                    "places a range icon at a share whose division overflows (i64::MIN / -1), which faults and ends the game",
                ));
            } else {
                scaled / span
            };
            let fraction = quotient as f32 / 100_000.0;
            let mut place = plain.track_at;
            let axis = usize::from(!plain.horizontal);
            place[axis] = truncate_f32((float(room[axis]) * fraction + float(centre[axis])) + 0.5);
            places[index] = Some(place);
        }
        Ok(places)
    }

    fn knob_part(&mut self, knob: Knob, at: [i32; 2], clip: Option<Clip>) -> Result<(), Refusal> {
        if !self.shown(knob.element()) {
            return Ok(());
        }
        match knob {
            Knob::Button(element, button) => self.button(
                element,
                &native::Button {
                    position: [0, 0],
                    ..button.clone()
                },
                at,
                [0, 0],
                clip,
            ),
            Knob::Icon(element, icon) => self.icon(
                element,
                &native::Icon {
                    position: [0.0, 0.0],
                    ..icon.clone()
                },
                at,
                [0, 0],
                clip,
            ),
        }
    }

    pub(super) fn plain_parts(
        &mut self,
        plain: &Plain,
        owner: &Element,
        node: [i32; 2],
        clip: Option<Clip>,
        parts: bool,
    ) -> Result<(), Refusal> {
        let at = |place: [i32; 2]| {
            [
                node[0].wrapping_add(place[0]),
                node[1].wrapping_add(place[1]),
            ]
        };
        let icons = self.icon_places(plain)?;
        if parts {
            self.built(owner, Role::Part(0), |walk| {
                let first = walk.regions.len();
                walk.knob_part(plain.right, at(plain.right_at), clip)?;
                for region in &mut walk.regions[first..] {
                    region.scroll =
                        Some((!plain.horizontal, if plain.horizontal { 1 } else { -1 }));
                }
                Ok(())
            })?;
            self.built(owner, Role::Part(1), |walk| {
                let first = walk.regions.len();
                walk.knob_part(plain.left, at(plain.left_at), clip)?;
                for region in &mut walk.regions[first..] {
                    region.scroll =
                        Some((!plain.horizontal, if plain.horizontal { -1 } else { 1 }));
                }
                Ok(())
            })?;
            let (element, button) = plain.track;
            self.built(owner, Role::Part(2), |walk| {
                walk.drag_part(
                    element,
                    button,
                    -1,
                    at(plain.track_at),
                    Some(plain.track_size),
                    clip,
                )
            })?;
            let (element, button) = plain.slider;
            self.built(owner, Role::Part(3), |walk| {
                walk.drag_part(
                    element,
                    button,
                    button.frame,
                    at(plain.slider_at()),
                    None,
                    clip,
                )
            })?;
        }
        for (index, (icon, place)) in plain.icons.into_iter().zip(icons).enumerate() {
            if let (Some(icon), Some(place)) = (icon, place) {
                self.built(owner, Role::Part(4 + index as u8), |walk| {
                    walk.knob_part(icon, at(place), clip)
                })?;
            }
        }
        Ok(())
    }

    pub(super) fn scrollbar_child(
        &mut self,
        element: &Element,
        bar: &native::Scrollbar,
        origin: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let shown = self.shown(element);
        let hidden_by_code = self
            .bound(element)
            .and_then(|bound| bound.shown)
            .is_some_and(|shown| shown.hidden_by_code);
        if !shown && (!element.hide || hidden_by_code) {
            return Ok(());
        }
        let mut plain = self.plain(element, bar)?;
        if shown {
            plain.place_slider();
        }
        let outer = self.opacity;
        let drawn = self.plain_parts(&plain, element, origin, clip, shown);
        self.opacity = outer;
        drawn
    }

    pub(super) fn named_plain(
        &self,
        owner: &Element,
        name: &[u8],
    ) -> Result<Option<Plain<'a>>, Refusal> {
        let name = c_string(name);
        if name.is_empty() {
            return Ok(None);
        }
        let Some(template) = self.resources.gui_type(name) else {
            return Err(Refusal::new(
                &owner.path,
                RefusalKind::NullRead,
                format!(
                    "names scrollbarType {:?}, which no template defines: construction reads the null type",
                    lossy(name)
                ),
            ));
        };
        let Fields::Scrollbar(bar) = &template.fields else {
            return Err(Refusal::new(
                &owner.path,
                RefusalKind::TypeConfusion,
                format!(
                    "names scrollbarType {:?}, a {}, which construction reads as a scrollbar template",
                    lossy(name),
                    template.kind.keyword()
                ),
            ));
        };
        Ok(Some(self.plain(self.resolved(template), bar)?))
    }

    pub(super) fn list_scrollbar(
        &self,
        element: &Element,
        list: &native::ListBox,
        smooth: bool,
    ) -> Result<Option<Plain<'a>>, Refusal> {
        let Some(mut plain) = self.named_plain(element, &list.scrollbar_type)? else {
            return Ok(None);
        };
        let [ox, oy] = list.offset;
        let [w, h] = list.size;
        if smooth {
            let horizontal = list.horizontal != 0;
            if horizontal {
                plain.horizontal = true;
                plain.resize(w, -1);
            } else {
                plain.resize(-1, h);
            }
            plain.move_to(if horizontal {
                [ox, oy.wrapping_add(h)]
            } else {
                [ox.wrapping_add(w), oy]
            });
            plain.set_value(0);
            plain.set_range(Range::new(0, 0, plain.range.step));
        } else {
            plain.resize(-1, h);
            plain.move_to(if list.scrollbar_side != 0 {
                [w.wrapping_add(ox), oy]
            } else {
                [ox.wrapping_sub(plain.size[0]), oy]
            });
            plain.set_value(0);
            plain.set_range(Range::new(0, 0, 10_000));
        }
        Ok(Some(plain))
    }

    pub(super) fn list_box(
        &mut self,
        element: &Element,
        list: &native::ListBox,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let smooth = element.kind == Kind::SmoothListBox;
        self.list_box_as(element, list, smooth, origin, parent, clip)
    }

    pub(super) fn list_box_as(
        &mut self,
        element: &Element,
        list: &native::ListBox,
        smooth: bool,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let place = list_box_place(element, list, smooth, self.bindings.viewport)?;
        self.list_scrollbar(element, list, smooth)?;
        let at = [origin[0] + place[0], origin[1] + place[1]];
        let background = c_string(&list.background);
        if !background.is_empty() {
            let outer = self.opacity;
            self.opacity = 1.0;
            self.sized = Some(list.size);
            let drawn = self.sprite_part(element, background, -1, 1.0, 0.0, at, Centre::None, clip);
            self.sized = None;
            self.opacity = outer;
            drawn?;
        }
        let clip = if smooth && list.clipping {
            intersect(
                clip,
                [at[0], at[1], at[0] + list.size[0], at[1] + list.size[1]],
            )
        } else {
            clip
        };
        self.hold(element, origin, parent, clip, 1.0, Some(place))
    }

    pub(super) fn overflow_box(
        &self,
        child: &Element,
        content: [i32; 2],
    ) -> Result<Option<Box2>, Refusal> {
        let path = &child.path;
        let no_parent = |why: String| Refusal::new(path, RefusalKind::NoParent, why);
        let drawable = |sprite: &[u8], scale: f32| {
            self.part_extent(path, sprite, scale).ok_or_else(|| {
                Refusal::new(
                    path,
                    RefusalKind::NullRead,
                    "has no drawable (neither its sprite nor the fallback is defined), which the drawable-size query reads through",
                )
            })
        };
        let content = self
            .bound(child)
            .and_then(|bound| bound.layout_parent)
            .unwrap_or(content);
        Ok(Some(match &child.fields {
            Fields::Container(_) => {
                let bound = self.bound(child);
                let frame = bound
                    .and_then(|bound| bound.window_frame)
                    .map_or_else(|| container_frame(child, Some(content)), Ok)
                    .map_err(no_parent)?;
                (
                    bound.and_then(|bound| bound.position).unwrap_or(frame.at),
                    frame.size,
                )
            }
            Fields::Icon(icon) => {
                let anchor = control_anchor(icon.orientation, content);
                (
                    [
                        content_reader_parser::crt::truncate_f32(icon.position[0]).wrapping_add(anchor[0]),
                        content_reader_parser::crt::truncate_f32(icon.position[1]).wrapping_add(anchor[1]),
                    ],
                    drawable(&icon.sprite, icon.scale)?,
                )
            }
            Fields::Button(_) if child.kind == Kind::Checkbox => return Ok(None),
            Fields::Button(button) => (
                button.position,
                self.knob_extent(Knob::Button(child, button))?,
            ),
            Fields::Grid(grid) => {
                let (position, size, _) = grid_geometry(child, grid, content).map_err(no_parent)?;
                let bound = self.bound(child);
                let position = bound.and_then(|bound| bound.position).unwrap_or(position);
                match bound.and_then(|bound| bound.grid) {
                    Some(bounds) => (
                        [
                            position[0].wrapping_add(bounds.adjust[0]),
                            position[1].wrapping_add(bounds.adjust[1]),
                        ],
                        bounds.extent,
                    ),
                    None => (position, size),
                }
            }
            Fields::List(list) => (list.position, list.size),
            Fields::ExtendedScrollbar(bar) => {
                let (at, size) = extended_frame(child, bar, content).map_err(no_parent)?;
                (at, size)
            }
            Fields::EditBox(edit) => (edit.position(), edit.size()),
            Fields::PositionType(_) => return Ok(None),
            Fields::TextBox(text_box) => {
                let bytes = self
                    .resources
                    .localisation()
                    .localise_bytes(&text_box.text, &[])
                    .map_err(|error| Refusal::new(path, RefusalKind::Reader, error.to_string()))?;
                let text: Option<String> =
                    native_text::decode(&bytes).map(|text| text.into_iter().collect());
                (
                    [0, 0],
                    self.text_size(
                        child,
                        &text_box.font,
                        text.as_deref(),
                        TextObject::of(text_box),
                    )?,
                )
            }
            Fields::Scrollbar(bar) => (
                bar.position.map(|value| truncate_f32(float(value))),
                bar.size,
            ),
            Fields::ListBox(list) => {
                let place = if child.kind == Kind::SmoothListBox {
                    list_box_place(child, list, true, self.bindings.viewport)?
                } else {
                    list.position
                };
                (place, [0, 0])
            }
            Fields::DropDown(drop) => {
                let position =
                    evaluate_position(&drop.position, Some(content), path).map_err(no_parent)?;
                (position, self.drop_down_screen(child, drop)?)
            }
            Fields::Browser(browser) if child.kind == Kind::CurveGraph => {
                let info = [browser.sprite.as_slice(), FALLBACK_SPRITE]
                    .into_iter()
                    .find_map(|name| self.info(path, name))
                    .ok_or_else(|| {
                        Refusal::new(
                            path,
                            RefusalKind::NullRead,
                            "has no drawable (neither its sprite nor the fallback is defined), \
                             which CreateGraphTexture writes through",
                        )
                    })?;
                let size = browser.size();
                let texture = [0, 1].map(|axis| {
                    if size[axis] > 0 {
                        size[axis]
                    } else {
                        info.texture[axis]
                    }
                });
                ([0, 0], SpriteInfo { texture, ..info }.scaled_extent(1.0))
            }
            Fields::Browser(browser) => ([0, 0], drawable(&browser.sprite, 1.0)?),
            Fields::Window(_) | Fields::TextSprite(_) | Fields::Other => return Ok(None),
        }))
    }

    fn drop_down_screen(
        &self,
        element: &Element,
        drop: &native::DropDown,
    ) -> Result<[i32; 2], Refusal> {
        let viewport = self.drop_down_viewport(element)?;
        let frame = container_frame(&drop.inner, Some(viewport))
            .map_err(|why| Refusal::new(&element.path, RefusalKind::NoParent, why))?;
        let [w, h] = viewport;
        let x = float(frame.at[0].wrapping_add(w / 2)) + -float(w) * 0.5;
        let y = float(h) * 0.5 - float(frame.at[1].wrapping_add(h / 2));
        Ok([truncate_f32(x), truncate_f32(-y)])
    }

    fn drop_down_viewport(&self, element: &Element) -> Result<[i32; 2], Refusal> {
        self.bindings.viewport.ok_or_else(|| {
            Refusal::new(
                &element.path,
                RefusalKind::NoParent,
                "builds its header in a position object sized to the GUI viewport, which the caller did not bind (Bindings::viewport)",
            )
        })
    }

    pub(super) fn container_scrollbars(
        &mut self,
        window: &Element,
        container: &native::Container,
        scroll: &ContentScroll,
        origin: [i32; 2],
        frame: Frame,
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let view = frame.content_size;
        for Bar {
            vertical,
            template,
            bar,
            shown,
            range,
            value,
            track_frame,
        } in self.bars(window, container, scroll, frame)?
        {
            if !shown {
                continue;
            }
            if container.smooth_scrolling
                && part(&bar.slider).is_none()
                && value.wrapping_abs() > 10_000
            {
                return Err(Refusal::new(
                    &template.path,
                    RefusalKind::NullRead,
                    format!(
                        "smooths {}'s scroll to {value} (fixed point) with no slider: the first frame reads the null slider's visible bit",
                        window.path
                    ),
                ));
            }
            let no_parent = |why: String| Refusal::new(&template.path, RefusalKind::NoParent, why);
            let position = evaluate_position(
                &bar.position,
                Some(frame.size),
                format_args!("{}.position", template.path),
            )
            .map_err(no_parent)?;
            let evaluated = evaluate_size(
                &bar.size,
                Some(frame.size),
                position,
                format_args!("{}.size", template.path),
            )
            .map_err(no_parent)?;
            let size = if vertical {
                [evaluated[0], view[1]]
            } else {
                [view[0], evaluated[1]]
            };
            let anchor = compute_anchor(bar.orientation, bar.origo, frame.size, size);
            let margin = if vertical {
                [0, frame.content_at[1]]
            } else {
                [frame.content_at[0], 0]
            };
            let at = [
                origin[0] + anchor[0] + position[0] + margin[0],
                origin[1] + anchor[1] + position[1] + margin[1],
            ];
            self.built(window, Role::Bar(vertical), |walk| {
                walk.extended_parts(
                    template,
                    bar,
                    at,
                    size,
                    (range, value),
                    true,
                    track_frame,
                    clip,
                )
            })?;
        }
        Ok(())
    }

    pub(super) fn bars(
        &self,
        window: &Element,
        container: &native::Container,
        scroll: &ContentScroll<'a>,
        frame: Frame,
    ) -> Result<Vec<Bar<'a>>, Refusal> {
        self.bars_for(window, container, scroll, frame, None)
    }

    pub(super) fn bars_for(
        &self,
        window: &Element,
        container: &native::Container,
        scroll: &ContentScroll<'a>,
        frame: Frame,
        include_hidden: Option<bool>,
    ) -> Result<Vec<Bar<'a>>, Refusal> {
        let view = frame.content_size;
        let mut shown = [!container.autohide_scrollbars; 2];
        let mut low = [0i32; 2];
        let mut high = view;
        let has = |axis: usize| {
            scroll
                .bars
                .iter()
                .any(|(vertical, ..)| usize::from(*vertical) == axis)
        };
        if !scroll.bars.is_empty() && !container.backgrounds.is_empty() {
            for child in &window.children {
                if include_hidden == Some(false) && !self.shown(child) {
                    continue;
                }
                let Some((at, size)) = self.overflow_box(child, view)? else {
                    continue;
                };
                for axis in 0..2 {
                    let far = at[axis].wrapping_add(size[axis]);
                    if (view[axis] < far || at[axis] < 0) && has(axis) {
                        high[axis] = high[axis].max(far);
                        low[axis] = low[axis].min(at[axis]);
                        shown[axis] = true;
                    }
                }
            }
        }
        Ok(scroll
            .bars
            .iter()
            .map(|&(vertical, template, bar)| {
                let axis = usize::from(vertical);
                let step = fixed(if vertical {
                    container.vertical_scroll_step
                } else {
                    container.horizontal_scroll_step
                });
                let range = Range::new(
                    i64::from(low[axis]) * 100_000,
                    i64::from(high[axis].wrapping_sub(view[axis])) * 100_000,
                    step,
                );
                let state = include_hidden
                    .is_none()
                    .then(|| self.bound(window).and_then(|bound| bound.bars[axis]))
                    .flatten();
                let own = Range::new(bar.min_value, bar.max_value, bar.step_size);
                Bar {
                    vertical,
                    template,
                    bar,
                    shown: state.map_or(shown[axis], |state| state.shown),
                    range: state.map_or(range, |state| state.range),
                    value: state
                        .map_or_else(|| range.clamp(scroll.targets[axis]), |state| state.value),
                    track_frame: state.map_or_else(
                        || track_frame(bar, own, scroll.targets[axis]),
                        |state| state.track_frame,
                    ),
                }
            })
            .collect())
    }

    pub(super) fn extended_child(
        &mut self,
        element: &Element,
        bar: &native::ExtendedScrollbar,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let (at, size) = extended_frame(element, bar, parent)
            .map_err(|why| Refusal::new(&element.path, RefusalKind::NoParent, why))?;
        let range = Range::new(bar.min_value, bar.max_value, bar.step_size);
        self.extended_parts(
            element,
            bar,
            [origin[0] + at[0], origin[1] + at[1]],
            size,
            (range, bar.start_value),
            false,
            None,
            clip,
        )
    }

    #[expect(clippy::too_many_arguments, reason = "the element, the scrollbar, its placement and range, its track frame and clip")]
    fn extended_parts(
        &mut self,
        element: &Element,
        bar: &native::ExtendedScrollbar,
        at: [i32; 2],
        size: [i32; 2],
        (range, value): (Range, i64),
        relaid: bool,
        track_frame: Option<i32>,
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let [w, h] = size;
        let extent = |this: &Self, part: Option<(&Element, &native::Button)>, scale: bool| {
            part.and_then(|(element, button)| {
                this.part_extent(
                    &element.path,
                    &button.sprite,
                    if scale { button.scale } else { 1.0 },
                )
            })
            .unwrap_or([0; 2])
        };
        let increase = part(&bar.increase_button);
        let decrease = part(&bar.decrease_button);
        let track = part(&bar.track);
        let slider = part(&bar.slider);
        let decrease_at = decrease.map_or([0; 2], |(_, button)| button.position);
        let decrease_size = extent(self, decrease, true);
        let increase_at = increase.map_or([w, h], |(_, button)| {
            [button.position[0] + w, button.position[1] + h]
        });
        let increase_size = extent(self, increase, true);
        let own_track = track.map_or([0; 2], |(_, button)| button.position);
        let track_size_own = extent(self, track, false);
        let axis = usize::from(!bar.horizontal);
        let mut track_at = own_track;
        let mut track_size = track_size_own;
        track_at[axis] = decrease_at[axis] + decrease_size[axis];
        track_size[axis] = if relaid {
            increase_at[axis] - track_at[axis]
        } else {
            size[axis] - track_at[axis] - increase_size[axis]
        };
        let slider_size = extent(self, slider, false);
        let travel = if relaid {
            track_size[axis] - slider_size[axis]
        } else {
            track_size[axis]
        };
        let thumb = range.thumb(value, travel);
        if let Some((element, button)) = part(&bar.background) {
            self.drag_part(
                element,
                button,
                button.frame,
                [at[0] + button.position[0], at[1] + button.position[1]],
                Some(size),
                clip,
            )?;
        }
        if let Some((element, button)) = track {
            self.drag_part(
                element,
                button,
                track_frame.unwrap_or(-1),
                [at[0] + track_at[0], at[1] + track_at[1]],
                Some(track_size),
                clip,
            )?;
        }
        if let Some((element, button)) = increase
            && self.shown(element)
        {
            let first = self.regions.len();
            self.button(element, button, [at[0] + w, at[1] + h], size, clip)?;
            for region in &mut self.regions[first..] {
                region.scroll = Some((!bar.horizontal, 1));
            }
        }
        if let Some((element, button)) = decrease
            && self.shown(element)
        {
            let first = self.regions.len();
            self.button(element, button, at, size, clip)?;
            for region in &mut self.regions[first..] {
                region.scroll = Some((!bar.horizontal, -1));
            }
        }
        if let Some((element, button)) = slider {
            let mut place = [
                at[0] + button.position[0] + track_at[0],
                at[1] + button.position[1] + track_at[1],
            ];
            place[axis] = place[axis].wrapping_add(thumb);
            if self.shown(element) {
                let first = self.regions.len();
                self.drag_button(element, button, button.frame, place, clip)?;
                for region in &mut self.regions[first..] {
                    region.slide = Some(Slide {
                        vertical: !bar.horizontal,
                        travel,
                        thumb,
                    });
                }
            }
        }
        let _ = element;
        Ok(())
    }

    pub(super) fn edit_box(
        &mut self,
        element: &Element,
        edit: &native::EditBox,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let flags = edit.flags();
        let Some(text) = self.shown_text(element, &edit.text) else {
            return Ok(());
        };
        self.edit_caret(element, edit, &text, origin, parent, clip)?;
        if flags & 0x2 == 0 {
            return self.edit_text_sprite(element, edit, &text, origin, clip);
        }
        let orientation = match edit.orientation() {
            1 => Orientation::UpperRight,
            2 => Orientation::LowerLeft,
            3 => Orientation::LowerRight,
            4 => Orientation::Center,
            5 => Orientation::CenterUp,
            6 => Orientation::CenterDown,
            _ => Orientation::UpperLeft,
        };
        let position = edit.position();
        self.text(
            element,
            TextSpec {
                font: &edit.font,
                text: &text,
                position: [origin[0] + position[0], origin[1] + position[1]],
                size: edit
                    .size()
                    .map(|value| if value == 0 { 200 } else { value }),
                border: edit.border_size(),
                format: TextFormat::Left,
                vertical_alignment: VerticalAlignment::Top,
                orientation,
                extent: parent,
                fixed_size: false,
                truncate: false,
                multiline: edit.is_multiline(),
                special: flags & 0x4 != 0,
                color_code: 0,
                technique: None,
            },
            clip,
        )
    }

    fn edit_caret(
        &mut self,
        element: &Element,
        edit: &native::EditBox,
        text: &str,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let Some(caret) = self.bound(element).and_then(|bound| bound.caret) else {
            return Ok(());
        };
        let prefix: String = text.chars().take(caret).collect();
        let width = edit_measure(self.resources, &self.refused, element, edit, &prefix)?;
        let [x, y] = edit_text_origin(edit, origin, parent);
        self.sprite_part(element, EDIT_CARET, -1, 1.0, 0.0, [x + width, y], Centre::None, clip)?;
        Ok(())
    }

    pub(super) fn drop_down(
        &mut self,
        element: &Element,
        drop: &native::DropDown,
        origin: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let path = &element.path;
        let viewport = self.drop_down_viewport(element)?;
        let place = evaluate_position(&drop.position, Some(viewport), path)
            .map_err(|why| Refusal::new(path, RefusalKind::NoParent, why))?;
        let at = [origin[0] + place[0], origin[1] + place[1]];
        let header = drop.inner.as_ref();
        if header.container().is_none() {
            return Ok(());
        }
        let frame = container_frame(header, Some(viewport))
            .map_err(|why| Refusal::new(&header.path, RefusalKind::NoParent, why))?;
        let header_at = [at[0] + frame.at[0], at[1] + frame.at[1]];
        if self.shown(header) {
            self.window(header, header_at, frame, clip)?;
        }
        if let Some(button_element) = drop.expand_button.as_deref()
            && let Some(button) = button_element.button()
            && self.shown(button_element)
        {
            self.button(button_element, button, header_at, frame.size, clip)?;
        }
        Ok(())
    }

    pub(super) fn browser(&mut self, element: &Element) -> Result<(), Refusal> {
        Err(Refusal::new(
            &element.path,
            RefusalKind::MissingInput,
            "shows a web page through the browser texture on its first drawable show, which the recording does not hold",
        ))
    }

    pub(super) fn curve_graph(&mut self, element: &Element) -> Result<(), Refusal> {
        let shown = self
            .bound(element)
            .and_then(|bound| bound.shown)
            .is_some_and(|shown| shown.visible);
        if !shown {
            return Ok(());
        }
        Err(Refusal::new(
            &element.path,
            RefusalKind::MissingInput,
            "is shown, and draws the points its code supplies, which the recording does not hold",
        ))
    }
}

fn extended_frame(
    element: &Element,
    bar: &native::ExtendedScrollbar,
    parent: [i32; 2],
) -> Result<([i32; 2], [i32; 2]), String> {
    let path = &element.path;
    let position = evaluate_position(&bar.position, Some(parent), format_args!("{path}.position"))?;
    let size = evaluate_size(
        &bar.size,
        Some(parent),
        position,
        format_args!("{path}.size"),
    )?;
    let anchor = compute_anchor(bar.orientation, bar.origo, parent, size);
    Ok(([position[0] + anchor[0], position[1] + anchor[1]], size))
}

impl Walk<'_> {
    fn edit_text_sprite(
        &mut self,
        element: &Element,
        edit: &native::EditBox,
        text: &str,
        origin: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let path = &element.path;
        let position = edit.position();
        let at = [origin[0] + position[0], origin[1] + position[1]];
        let info = match self.info(path, b"") {
            Some(info) if matches!(info.kind, SpriteKind::Text { .. }) => info,
            Some(_) => {
                return self.sprite_part(element, b"", -1, 1.0, 0.0, at, Centre::None, clip);
            }
            None => SpriteInfo {
                texture: [0; 2],
                frames: 1,
                vertical: false,
                masked: false,
                animated: false,
                kind: SpriteKind::Text { textured: false },
                anchor: [0.0; 2],
                effect: self
                    .resources
                    .effect(super::super::native_resources::DEFAULT_SPRITE_EFFECT),
                hit: Hit::Frame,
                missing: None,
                refusals: Vec::new(),
            },
        };
        let texture = if edit.texture.is_empty() {
            text_sprites::BakeTexture::Type
        } else {
            match self.resources.texture_file(&edit.texture) {
                Some(TextureFile::Loaded(texture)) => {
                    text_sprites::BakeTexture::File(Some(texture))
                }
                Some(TextureFile::Failed(_)) => text_sprites::BakeTexture::File(None),
                Some(TextureFile::Refused(kind, why)) => {
                    return Err(Refusal::new(
                        path,
                        kind,
                        format!("bakes its text over the texture {why}"),
                    ));
                }
                None => {
                    return Err(Refusal::new(
                        path,
                        RefusalKind::MissingResource,
                        format!(
                            "bakes its text over the texture {:?}, which the caller has not \
                             loaded (Assets::textures)",
                            lossy(&edit.texture)
                        ),
                    ));
                }
            }
        };
        self.text_sprite(
            element,
            b"",
            &info,
            Some(text_sprites::TextSpriteDraw {
                text,
                font: &edit.font,
                texture,
                size: edit.size(),
                border: edit.border_size(),
                format: 0,
                fixed: false,
                special: edit.flags() & 0x4 != 0,
                multiline: true,
            }),
            -1,
            State::Up,
            at,
            [0, 0],
            0.0,
            clip,
        )?;
        Ok(())
    }
}

fn edit_text_origin(edit: &native::EditBox, origin: [i32; 2], parent: [i32; 2]) -> [i32; 2] {
    let [x, y] = [origin[0] + edit.position()[0], origin[1] + edit.position()[1]];
    let [w, h] = parent;
    let [bx, by] = edit.border_size().map(|side| side.max(0));
    let anchored = match edit.orientation() {
        1 => [x + w, y],
        2 => [x, y + h],
        3 => [x + w, y + h],
        4 => [x + w / 2, y + h / 2],
        5 => [x + w / 2, y],
        6 => [x + w / 2, y + h],
        _ => [x, y],
    };
    [anchored[0] + bx, anchored[1] + by]
}

fn edit_measure(
    resources: &dyn Resources,
    refused: &RefCell<Refusals>,
    element: &Element,
    edit: &native::EditBox,
    text: &str,
) -> Result<i32, Refusal> {
    let font = text_font(resources, element, &edit.font)?;
    let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
    let chars: Vec<char> = text.chars().collect();
    Ok(native_text::lay_out(
        font,
        &sprite,
        &chars,
        native_text::Limits {
            width: 0,
            height: i32::MAX,
            border: [0, 0],
            newlines: false,
            preserve_indent: false,
            ellipsis: false,
            multiline: false,
            special: edit.flags() & 0x4 != 0,
            icons_add_height: font.facts.icons_add_height,
        },
    )
    .map_err(|why| Refusal::new(&element.path, RefusalKind::DivideByZero, why))?
    .widths
    .first()
    .copied()
    .unwrap_or(0))
}

pub fn edit_caret_at(
    resources: &dyn Resources,
    element: &Element,
    text: &str,
    origin: [i32; 2],
    parent: [i32; 2],
    point: [i32; 2],
) -> Result<usize, Refusal> {
    let Fields::EditBox(edit) = &element.fields else {
        return Err(Refusal::new(
            &element.path,
            RefusalKind::Reader,
            "is not an edit box",
        ));
    };
    let refused = RefCell::new(Refusals::new());
    let start = edit_text_origin(edit, origin, parent);
    let x = point[0] - start[0];
    let chars: Vec<char> = text.chars().collect();
    let width = |count: usize| -> Result<i32, Refusal> {
        edit_measure(
            resources,
            &refused,
            element,
            edit,
            &chars[..count].iter().collect::<String>(),
        )
    };
    let mut caret = chars.len();
    for count in 1..=chars.len() {
        if x <= width(count)? {
            caret = count - 1;
            break;
        }
    }
    if caret < chars.len() && (width(caret + 1)? - x).abs() < (width(caret)? - x).abs() {
        caret += 1;
    }
    Ok(caret.min(chars.len()))
}
