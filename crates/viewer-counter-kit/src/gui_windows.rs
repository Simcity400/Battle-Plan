
use content_reader_core::gui::native;
use std::io;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use super::{
    CONVOY_ADDITION_ICON, CounterColorMode, CounterDefines, CounterInputs, CounterSettings,
    Countries, EMBLEM_SUFFIX, FACILITY_SPRITE_PREFIX, STRATEGIC_LOCATION_SPRITE_PREFIX,
    UNKNOWN_COUNT, counter_plate_color, rank_children,
};
use viewer_foundation::templates;
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::templates::NO_INTEL_SPRITE;
use viewer_native_gui::lazy_icons::LazyFlags;
use viewer_native_gui::native_windows::{
    LAND_COMBAT_WINDOW, NAVAL_COMBAT_WINDOW, NAVY_WINDOW, NativeWindows,
};
use viewer_native_gui::sprites::Batch;
use viewer_unit_reads::{counter_inputs, counter_order, counter_status};
use viewer_native_gui::gui_resources::GuiAssets;
use viewer_native_gui::gui_resources::GuiFlags;
use viewer_native_gui::gui_resources::HitMode;
use viewer_native_gui::gui_resources::observer_hit_rectangle;
use viewer_native_gui::gui_resources::{HitRectangle, refused_cause};
use viewer_native_gui::native_resources::GuiResources;
use viewer_runtime::unit_handle::Unit;
use crate::flotilla::{Flotilla, NavalBars};

use viewer_native_gui::native_flags;
use viewer_native_gui::native_gui::{self, Layout, Texture, lifecycle::{Call, Instance}};

pub fn gui_assets<'a>(
    inputs: &CounterInputs<'a>,
    flags: Option<&'a WindowFlags<'a>>,
) -> GuiAssets<'a> {
    let [width, height] = inputs.viewport.map(i32::try_from);
    GuiAssets {
        gui: &inputs.assets.gui,
        windows: inputs.windows,
        localisation: inputs.localisation,
        flags: flags.map(|flags| GuiFlags {
            source: flags,
            small: flags.small,
            large: flags.large,
        }),
        portrait_textures: None,
        physical_viewport: width
            .ok()
            .zip(height.ok())
            .map(|(width, height)| [width, height]),
    }
}

pub fn window_flags<'a>(
    inputs: &CounterInputs<'a>,
    countries: Option<&'a Countries>,
) -> Option<WindowFlags<'a>> {
    countries.map(|countries| WindowFlags {
        countries,
        small: &inputs.assets.flags,
        large: &inputs.assets.large_flags,
    })
}

pub struct WindowFlags<'a> {
    pub countries: &'a Countries,
    pub small: &'a LazyFlags,
    pub large: &'a LazyFlags,
}

impl native_flags::FlagSource for WindowFlags<'_> {
    fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String> {
        let tag = std::str::from_utf8(tag).map_err(|_| {
            format!(
                "tag {:?} is not UTF-8, and the recording's tags are text",
                String::from_utf8_lossy(tag)
            )
        })?;
        self.countries
            .tags_map()
            .iter()
            .find(|(_, own)| own.as_str() == tag)
            .map(|(&slot, _)| Some(slot))
            .ok_or_else(|| {
                format!(
                    "tag {tag} has no recorded country: whether the country database \
                     holds it, and its slot, are not recorded"
                )
            })
    }

    fn first_tag_slot(&self) -> Result<u32, String> {
        Err("the country database's first tag is not recorded".to_owned())
    }

    fn atlas_len(&self, _: bool) -> Result<i32, String> {
        let len = self
            .countries
            .tags_map()
            .keys()
            .next_back()
            .map_or(0, |&slot| slot + 1);
        i32::try_from(len).map_err(|_| format!("{len} country slots exceed the atlas's count"))
    }

    fn atlas_record(&self, small: bool, index: i32) -> Result<native_flags::FlagRecord, String> {
        let atlas = if small { self.small } else { self.large };
        let slot = u32::try_from(index).map_err(|_| format!("flag record {index}"))?;
        if !self.countries.tags_map().contains_key(&slot) {
            return Err(format!(
                "country slot {slot}'s tag is not recorded, so its flag record is unknown"
            ));
        }
        Ok(match self.countries.flag_now(atlas, slot) {
            Some((texture, [u0, v0, u1, v1])) => native_flags::FlagRecord::Cell {
                coords: [u0, v0, u1 - u0, v1 - v0],
                page: Some(Texture {
                    texture,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    size: atlas.cell_size().map(|value| value as i32),
                }),
            },
            None => {
                if let Some(reason) = self.countries.flag_refusal(slot) {
                    return Err(reason);
                }
                native_flags::FlagRecord::Invalid
            }
        })
    }
}

pub fn draw_window(
    layout: &Layout,
    items: &[Vec<native_gui::Part>],
    origin: [i32; 2],
    resources: &dyn native_gui::Sheets,
    batch: &mut Batch,
    (priority, icon): (i8, u32),
) -> io::Result<Result<(), native_gui::Refusal>> {
    let first = batch.quads.len();
    batch.at_layer(0);
    if let Err(refusal) = native_gui::draw_composed(layout, items, origin, resources, batch) {
        return Ok(Err(refusal));
    }
    rank_children(batch, first, priority, icon).map(Ok)
}

pub fn draw_held(
    holder: &native_gui::Holder,
    parts: Vec<native_gui::Part>,
    origin: [i32; 2],
    resources: &dyn native_gui::Sheets,
    batch: &mut Batch,
    key: (i8, u32),
) -> io::Result<Result<(), native_gui::Refusal>> {
    let held = Layout {
        size: [0; 2],
        parts: Vec::new(),
        holders: vec![native_gui::Holder {
            before: 0,
            ..holder.clone()
        }],
        diagnostics: Vec::new(),
        pointed: None,
        hovered: None,
        target: None,
        boxes: Vec::new(),
        regions: Vec::new(),
    };
    draw_window(&held, &[parts], origin, resources, batch, key)
}

pub struct Writes<'w> {
    pub window: &'w native::Element,
    calls: Vec<(String, Call)>,
    controls: Vec<(String, native_gui::Bound)>,
    wrote: Vec<&'static str>,
    unknown: Vec<(&'static str, String)>,
    hidden: Vec<&'static str>,
    kept: Vec<&'static str>,
}

impl<'w> Writes<'w> {
    pub fn new(window: &'w native::Element) -> Self {
        Self {
            window,
            calls: Vec::new(),
            controls: Vec::new(),
            wrote: Vec::new(),
            unknown: Vec::new(),
            hidden: Vec::new(),
            kept: Vec::new(),
        }
    }

    pub fn hid(&mut self, part: &'static str) {
        self.hidden.push(part);
    }

    pub fn kept(&mut self, part: &'static str) {
        self.kept.push(part);
    }

    pub fn wrote(&mut self, part: &'static str) {
        if !self.wrote.contains(&part) {
            self.wrote.push(part);
        }
    }

    pub fn unknown(&mut self, part: &'static str, why: impl Into<String>) {
        self.unknown.push((part, why.into()));
    }

    fn call(&mut self, kind: native::Kind, name: &str, call: Call) -> bool {
        let Some(element) = self.window.find(kind, name) else {
            return false;
        };
        self.calls.push((element.path.clone(), call));
        true
    }

    pub fn show(&mut self, kind: native::Kind, name: &str) -> bool {
        self.call(kind, name, Call::Show)
    }

    pub fn hide(&mut self, kind: native::Kind, name: &str) -> bool {
        self.call(kind, name, Call::Hide)
    }

    pub fn hide_flag(&mut self, kind: native::Kind, name: &str, hide: bool) -> bool {
        self.call(kind, name, Call::SetHideFlag(hide))
    }

    pub fn hide_and_flag(&mut self, kind: native::Kind, name: &str) {
        self.hide(kind, name);
        self.hide_flag(kind, name, true);
    }

    pub fn unflag_and_show(&mut self, kind: native::Kind, name: &str) {
        self.hide_flag(kind, name, false);
        self.show(kind, name);
    }

    pub fn shown(&mut self, kind: native::Kind, name: &str, shown: bool) {
        if shown {
            self.show(kind, name);
        } else {
            self.hide(kind, name);
        }
    }

    pub fn frame(&mut self, kind: native::Kind, name: &str, frame: i32) -> bool {
        self.call(kind, name, Call::SetFrame(frame))
    }

    pub fn control(
        &mut self,
        kind: native::Kind,
        name: &str,
        set: impl FnOnce(&mut native_gui::Bound),
    ) -> bool {
        let Some(element) = self.window.find(kind, name) else {
            return false;
        };
        let mut written = native_gui::Bound::default();
        set(&mut written);
        if let Some(sprite) = written.sprite.take() {
            self.calls.push((
                element.path.clone(),
                Call::SetSprite(sprite, written.setter),
            ));
        }
        if let Some(text) = written.text.take() {
            self.calls.push((element.path.clone(), Call::SetText(text)));
        }
        let at = match self
            .controls
            .iter()
            .position(|(path, _)| *path == element.path)
        {
            Some(at) => at,
            None => {
                self.controls
                    .push((element.path.clone(), native_gui::Bound::default()));
                self.controls.len() - 1
            }
        };
        overlay(&mut self.controls[at].1, &written);
        true
    }

    pub fn finish(self) -> Vec<(String, Call)> {
        self.update().calls
    }

    pub fn update(self) -> Update {
        let mut calls = self.calls;
        calls.extend(
            self.controls
                .into_iter()
                .map(|(path, bound)| (path, Call::Control(Box::new(bound)))),
        );
        Update {
            calls,
            wrote: self.wrote,
            unknown: self.unknown,
            hidden: self.hidden,
            kept: self.kept,
            selectable: None,
            owner: None,
            port_tooltip: None,
            naval_convoy: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    pub naval_convoy: Option<u64>,
    pub selectable: Option<(bool, Arc<[u64]>)>,
    pub owner: Option<u32>,
    pub port_tooltip: Option<viewer_native_gui::counter_hovers::task_force_port::Metadata>,
    pub calls: Vec<(String, Call)>,
    pub wrote: Vec<&'static str>,
    pub unknown: Vec<(&'static str, String)>,
    pub hidden: Vec<&'static str>,
    pub kept: Vec<&'static str>,
}

impl Update {
    pub fn calls(calls: Vec<(String, Call)>) -> Self {
        Self {
            calls,
            ..Self::default()
        }
    }
}

pub const PLATE: &str = "count_bg's plate colour";

pub const BARS: &str = "the strength and organisation bars' move";

pub const SUPPLY: &str = "low_supply";

pub const PLATE_FRAME: &str = "count_bg's frame";

pub const CARRIER: &str = "air_base_icon";

pub const CARRIER_SPRITE: &str = "air_base_icon's sprite";

pub const CAPITALS: &str = "capital_count_txt";

pub const COUNT: &str = "count_txt";

pub const ORDERS: &str = "the order glyphs";

pub const PRIDE: &str = "pride_of_the_fleet_icon";

pub const MARKER: &str = "group_marker";

pub const FLAG: &str = "flag";

pub const IDEOLOGY: &str = "counter_ideology";

pub const MISSION_ITEM_KEPT: [&str; 3] = [SUPPLY, TYPE_SPRITE, CARRIER_SPRITE];

pub const TYPE_SPRITE: &str = "the type child's sprite";

pub fn apply(
    instance: &mut Instance,
    root: &native::Element,
    calls: &[(String, Call)],
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    let _profile = viewer_runtime::profiling::span("dc_nu_calls");
    if calls.is_empty() {
        return Ok(());
    }
    instance.apply_calls(
        root,
        calls.iter().filter_map(|(path, call)| {
            (!matches!(call, Call::Control(_))).then_some((path.as_str(), call))
        }),
        resources,
    )?;
    let _controls_profile = viewer_runtime::profiling::span("dc_nu_controls");
    for (path, call) in calls {
        let Call::Control(written) = call else {
            continue;
        };
        let element = instance.element_at_path(root, path)?;
        overlay(instance.control(root, element)?, written);
    }
    Ok(())
}

fn overlay(control: &mut native_gui::Bound, written: &native_gui::Bound) {
    macro_rules! copy {
        ($($field:ident),*) => {
            $(if written.$field.is_some() {
                control.$field = written.$field;
            })*
        };
    }
    copy!(
        color, progress, texture, stretch, offset, rotation, shield, checked, enabled
    );
}

pub fn native_hit_rectangle(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Option<HitRectangle> {
    observer_hit_rectangle(
        control,
        layout,
        origin,
        resources,
        notes,
        HitMode::NativeGeometry,
    )
}

pub fn find_path<'w>(
    window: &'w native::Element,
    path: &str,
) -> Option<&'w native::Element> {
    if window.path == path {
        return Some(window);
    }
    window
        .nested()
        .into_iter()
        .find_map(|nested| find_path(nested, path))
}



#[cfg(any(test, feature = "test-support"))]
pub fn background_footprint(
    window: &native::Element,
    layout: &native_gui::Layout,
) -> Option<[f32; 4]> {
    let background = window.find(native::Kind::Button, "bg_btn")?;
    layout
        .parts
        .iter()
        .filter(|part| part.path == background.path)
        .find_map(|part| match part.kind {
            native_gui::PartKind::Sprite { geometry, .. } => Some(footprint(&geometry)),
            _ => None,
        })
}


#[cfg(any(test, feature = "test-support"))]
fn footprint(geometry: &native_gui::Geometry) -> [f32; 4] {
    let corners = geometry.corners();
    let [mut x0, mut y0] = corners[0];
    let [mut x1, mut y1] = corners[0];
    for [x, y] in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    [x0, y0, x1 - x0, y1 - y0]
}



pub const NAVAL_RESULT_SPRITES: [&str; 4] = [
    "GFX_naval_combat_result_navalstrikes",
    "GFX_naval_combat_result_portstrikes",
    "GFX_naval_combat_result",
    "GFX_naval_combat_result_convoys",
];

pub const CONVOY_ICON_GAP: &str = "a convoy addition's GFX_unit_transport_icon_medium_white is not defined; drawn with the icon's authored sprite";
pub struct NavyState<'a> {
    pub flotilla: &'a Flotilla,
    pub countries: &'a Countries,
}

pub struct NavyContext<'a> {
    pub settings: CounterSettings,
    pub selected: &'a BTreeSet<Unit>,
    pub defines: &'a CounterDefines,
    pub art: &'a dyn NavyArt,
    pub gui: &'a dyn native_gui::Resources,
}

pub trait NavyArt {
    fn flag(&self, slot: u32, medium: bool) -> Option<Texture>;
}

pub struct CounterArt<'a> {
    pub assets: &'a crate::CounterArt,
    pub countries: &'a Countries,
}

impl NavyArt for CounterArt<'_> {
    fn flag(&self, slot: u32, medium: bool) -> Option<Texture> {
        let atlas = if medium {
            &self.assets.medium_flags
        } else {
            &self.assets.flags
        };
        let (texture, uv) = self.countries.flag_now(atlas, slot)?;
        Some(flag_texture(atlas, texture, uv))
    }
}

pub fn plate_color(
    ctx: &NavyContext<'_>,
    countries: &Countries,
    side: Allegiance,
    country: Option<u32>,
) -> Result<[f32; 4], String> {
    counter_plate_color(
        ctx.settings.color_mode,
        &ctx.defines.tints,
        countries,
        side,
        country,
    )
}

pub fn navy_calls(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    state: &NavyState<'_>,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
) -> Update {
    let flotilla = state.flotilla;
    let plate = match (ctx.settings.color_mode, &flotilla.side) {
        (CounterColorMode::Allegiance, Err(missing)) => Err(missing.clone()),
        (CounterColorMode::Country, _) => {
            plate_color(ctx, state.countries, Allegiance::Other, flotilla.logical)
        }
        (CounterColorMode::Allegiance, Ok(side)) => {
            plate_color(ctx, state.countries, *side, flotilla.logical)
        }
    };
    let mut writes = Writes::new(window);
    if flotilla.port {
        mission_item(ctx, window, state, plate, report, &mut writes);
    } else {
        at_sea(ctx, window, state, plate, report, &mut writes);
    }
    let mut update = writes.update();
    update.selectable = Some((
        state.countries.is_ours(flotilla.owner),
        Arc::clone(&flotilla.members),
    ));
    update.owner = Some(flotilla.owner);
    update.port_tooltip.clone_from(&flotilla.port_tooltip);
    update
}

fn plate_write(plate: &Result<[f32; 4], String>, writes: &mut Writes<'_>) {
    use native::Kind::Button;
    match plate {
        Ok(plate) => {
            writes.control(Button, "count_bg", |bound| bound.color = Some(*plate));
            writes.wrote(PLATE);
        }
        Err(why) => writes.unknown(PLATE, why.clone()),
    }
}

pub fn mission_item_setup(window: &native::Element) -> Vec<(String, Call)> {
    use native::Kind::{Icon, TextBox};
    let mut writes = Writes::new(window);
    for name in ["counter_ideology", "counter_overlay", "large_flag"] {
        writes.hide_and_flag(Icon, name);
    }
    for name in ["count_txt", "capital_count_txt"] {
        writes.control(TextBox, name, |bound| bound.text = Some(String::new()));
    }
    writes.finish()
}

fn bind_supply(
    ctx: &NavyContext<'_>,
    supply: counter_inputs::NavalSupply,
    flagged: bool,
    writes: &mut Writes<'_>,
) {
    use counter_inputs::NavalSupply;
    use native::Kind::Icon;
    let ratio = match supply {
        NavalSupply::Retain => return,
        NavalSupply::Unavailable => return writes.unknown(SUPPLY, SUPPLY_UNRECORDED),
        NavalSupply::Write(ratio) => ratio,
    };
    match counter_status::supply_frame(ratio, ctx.defines.supply) {
        None if flagged => writes.hide_and_flag(Icon, "low_supply"),
        None => {
            writes.hide(Icon, "low_supply");
        }
        Some(frame) => {
            if flagged {
                writes.unflag_and_show(Icon, "low_supply");
            } else {
                writes.show(Icon, "low_supply");
            }
            if let Ok(frame) = i32::try_from(frame + 1) {
                writes.frame(Icon, "low_supply", frame);
            }
        }
    }
    writes.wrote(SUPPLY);
}

pub const SUPPLY_UNRECORDED: &str = "low_supply is undecided: a member's received-supply \
     ratio (TaskForceCounterInputs) is not recorded";

fn bind_carrier(
    gui: &dyn native_gui::Resources,
    flotilla: &Flotilla,
    flagged: bool,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::Button;
    let undecided = |writes: &mut Writes<'_>, why: &str| {
        writes.unknown(CARRIER, why);
        writes.unknown(CARRIER_SPRITE, why);
    };
    let carrier = match flotilla.cells.as_ref() {
        None => return undecided(writes, SHIPS_UNRECORDED),
        Some(cells) if cells.carrier.len() != 1 => {
            return undecided(
                writes,
                "air_base_icon is undecided: the unrecorded carrier operands leave more than one \
                 state possible",
            );
        }
        Some(cells) => cells.carrier.first().copied().flatten(),
    };
    writes.wrote(CARRIER);
    match carrier {
        Some((sprite, frame)) => {
            if flagged {
                writes.unflag_and_show(Button, "air_base_icon");
            } else {
                writes.show(Button, "air_base_icon");
            }
            writes.control(Button, "air_base_icon", |bound| {
                bound.sprite = Some(sprite.as_bytes().to_vec());
                bound.setter = native_gui::SpriteSetter::ButtonStandard;
            });
            if gui.sprite(sprite.as_bytes()).is_some() {
                writes.wrote(CARRIER_SPRITE);
            } else {
                *report
                    .entry(format!("{sprite}{CARRIER_UNDEFINED}").into())
                    .or_default() += 1;
                writes.kept(CARRIER_SPRITE);
            }
            if let Ok(frame) = i32::try_from(frame + 1) {
                writes.frame(Button, "air_base_icon", frame);
            }
        }
        None => {
            if flagged {
                writes.hide_and_flag(Button, "air_base_icon");
            } else {
                writes.hide(Button, "air_base_icon");
            }
            writes.hid(CARRIER_SPRITE);
        }
    }
}

pub const CARRIER_UNDEFINED: &str = " is not defined; air_base_icon keeps the sprite it \
     showed because the selection does not replace it";

fn bar_writes(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    fills: (f32, f32),
    writes: &mut Writes<'_>,
) {
    use native::Kind::Icon;
    let (organisation, strength) = fills;
    let mut measured = true;
    for (name, fill) in [("bar_str", strength), ("bar_org", organisation)] {
        let length = match window.find(Icon, name) {
            Some(element) => {
                native_gui::drawable_extent(element, ctx.gui, &native_gui::Bindings::default())
                    .map_err(|refusals| {
                        refusals
                            .iter()
                            .map(|refusal| refused_cause(NAVY_WINDOW, refusal))
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
            }
            None => Ok(None),
        };
        let offset = match length {
            Ok(length) => {
                let height = length.map_or(0, |[_, height]| height);
                Some([
                    i32::try_from(counter_inputs::bar_step(fill, height as f32))
                        .unwrap_or(i32::MAX),
                    0,
                ])
            }
            Err(why) => {
                measured = false;
                writes.unknown(BARS, why);
                None
            }
        };
        writes.control(Icon, name, |bound| {
            bound.offset = offset;
            bound.stretch = Some(fill);
        });
    }
    if measured {
        writes.wrote(BARS);
    }
}

fn bind_orders(flotilla: &Flotilla, flagged: bool, writes: &mut Writes<'_>) {
    use native::Kind::Icon;
    let set = |writes: &mut Writes<'_>, name: &str, shown: bool| match (shown, flagged) {
        (true, true) => writes.unflag_and_show(Icon, name),
        (false, true) => writes.hide_and_flag(Icon, name),
        (shown, false) => writes.shown(Icon, name, shown),
    };
    let status = &flotilla.status;
    let (no_orders, invalid) = match status.no_orders {
        None => return writes.unknown(ORDERS, ORDERS_UNDECIDED),
        Some(true) => (true, Some(false)),
        Some(false) => (false, status.invalid_orders),
    };
    set(writes, "no_orders", no_orders);
    match invalid {
        Some(invalid) => {
            set(writes, "invalid_orders", invalid);
            writes.wrote(ORDERS);
        }
        None => writes.unknown(ORDERS, ORDERS_UNDECIDED),
    }
}

pub const ORDERS_UNDECIDED: &str = "the order glyphs are undecided: a member's order \
     operands (TaskForceCounterInputs, its fleet's regions) are not recorded";

pub const SHIP_CENSUS_INCOMPLETE: &str =
    "the ship count is undecided: the hour's Ships census is incomplete";

pub const SHIPS_UNRECORDED: &str =
    "a member's ship counter operands (ShipCounterInputs) are not recorded";

pub const PRIDE_UNDECIDED: &str = "pride_of_the_fleet_icon is undecided: a member's pride \
     of the fleet (CountryCounterInputs) or its ship is not recorded";

fn bind_pride(flotilla: &Flotilla, flagged: bool, writes: &mut Writes<'_>) {
    use native::Kind::Icon;
    let name = "pride_of_the_fleet_icon";
    let shown = match flotilla.shows_pride() {
        Some(shown) => shown,
        None => return writes.unknown(PRIDE, PRIDE_UNDECIDED),
    };
    match (shown, flagged) {
        (true, true) => writes.unflag_and_show(Icon, name),
        (false, true) => writes.hide_and_flag(Icon, name),
        (shown, false) => writes.shown(Icon, name, shown),
    }
    writes.wrote(PRIDE);
}

fn bind_marker(flotilla: &Flotilla, flagged: bool, writes: &mut Writes<'_>) {
    use native::Kind::Icon;
    match &flotilla.marker {
        Err(why) => return writes.unknown(MARKER, why.clone()),
        Ok(Some(color)) => {
            if flagged {
                writes.unflag_and_show(Icon, "group_marker");
            } else {
                writes.show(Icon, "group_marker");
            }
            let color = *color;
            writes.control(Icon, "group_marker", |bound| bound.color = Some(color));
        }
        Ok(None) if flagged => writes.hide_and_flag(Icon, "group_marker"),
        Ok(None) => {
            writes.hide(Icon, "group_marker");
        }
    }
    writes.wrote(MARKER);
}

enum Hull {
    Resolved(Vec<u8>),
    Invalid,
    Unknown(&'static str),
}

fn resolved_hull(flotilla: &Flotilla) -> Hull {
    use counter_order::HullIcon;
    match flotilla.cells.as_ref().map(|cells| &cells.hull) {
        Some(HullIcon::Resolved(Some(name))) => Hull::Resolved(name.clone()),
        Some(HullIcon::Resolved(None)) => Hull::Invalid,
        Some(HullIcon::Ambiguous) => Hull::Unknown(HULL_AMBIGUOUS),
        Some(HullIcon::Unrecorded) | None => Hull::Unknown(HULL_UNRECORDED),
    }
}

fn select_hull(
    gui: &dyn native_gui::Resources,
    hull: &Hull,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::Icon;
    if writes.window.find(Icon, "type").is_none() {
        return;
    }
    let name = match hull {
        Hull::Resolved(name) if !name.is_empty() => name,
        Hull::Resolved(_) | Hull::Invalid => return writes.kept(TYPE_SPRITE),
        Hull::Unknown(why) => return writes.unknown(TYPE_SPRITE, *why),
    };
    let sprite = [
        templates::UNIT_PREFIX.as_bytes(),
        name,
        templates::UNIT_SUFFIX.as_bytes(),
    ]
    .concat();
    if gui.sprite(&sprite).is_none() {
        *report
            .entry(format!("{}{HULL_UNDEFINED}", String::from_utf8_lossy(&sprite)).into())
            .or_default() += 1;
        return writes.kept(TYPE_SPRITE);
    }
    writes.control(Icon, "type", |bound| {
        bound.sprite = Some(sprite);
        bound.setter = native_gui::SpriteSetter::Button;
    });
    writes.wrote(TYPE_SPRITE);
    writes.frame(Icon, "type", 1);
}

fn no_intel_sprite(gui: &dyn native_gui::Resources, writes: &mut Writes<'_>) {
    let sprite = NO_INTEL_SPRITE.as_bytes();
    if writes.control(native::Kind::Icon, "type", |bound| {
        bound.sprite = Some(sprite.to_vec());
        bound.setter = native_gui::SpriteSetter::Icon;
    }) {
        if gui.sprite(sprite).is_some() {
            writes.wrote(TYPE_SPRITE);
        } else {
            writes.kept(TYPE_SPRITE);
        }
    }
}

fn no_intel_frame(ctx: &NavyContext<'_>) -> i32 {
    i32::try_from(templates::no_intel_icon(ctx.settings.nato).frame + 1).unwrap_or(1)
}

pub const HULL_UNDEFINED: &str =
    " is not defined; the type child keeps what it showed because the selection makes no write";

pub const HULL_AMBIGUOUS: &str = "a hull is not selected: two best ships tie on an \
     unrecorded variant and name different sprites";

pub const HULL_UNRECORDED: &str =
    "a hull is not selected: a member's ship counter operands are not recorded";

fn at_sea(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    state: &NavyState<'_>,
    plate: Result<[f32; 4], String>,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::{Button, Container, Icon, TextBox};
    let flotilla = state.flotilla;
    writes.hide(Icon, "counter_ideology");
    bind_supply(ctx, flotilla.supply, false, writes);
    bind_carrier(ctx.gui, flotilla, false, report, writes);
    let capitals = match &flotilla.cells {
        Some(cells) => cells.capitals.map(|sum| sum.to_string()),
        None => (flotilla.ships == Some(0) && !flotilla.hidden).then(|| "0".to_owned()),
    };
    writes.show(Icon, "capital_ship");
    writes.show(TextBox, "capital_count_txt");
    capitals_write(capitals, writes);
    bind_pride(flotilla, false, writes);
    let active = flotilla.status.active;
    let count = if flotilla.hidden {
        Some(UNKNOWN_COUNT.to_owned())
    } else {
        flotilla.ships.map(|ships| ships.to_string())
    };
    writes.hide(Container, "spotting_bar_container");
    let selected = flotilla
        .members
        .first()
        .is_some_and(|&id| ctx.selected.contains(&Unit::TaskForce(id)));
    if selected && (flotilla.coarse || flotilla.visible_member) {
        writes.show(Icon, "count_selected");
    } else {
        writes.hide(Icon, "count_selected");
    }
    if flotilla.coarse {
        plate_write(&plate, writes);
        writes.show(Button, "count_bg");
        writes.frame(Button, "count_bg", 3);
        writes.wrote(PLATE_FRAME);
        writes.show(TextBox, "count_txt");
        count_write(count, writes);
        match ctx.art.flag(flotilla.owner, true) {
            Some(texture) => {
                writes.show(Icon, "large_flag");
                writes.control(Icon, "large_flag", |bound| bound.texture = Some(texture));
            }
            None => {
                writes.hide(Icon, "large_flag");
            }
        }
        for name in [
            "bar_str",
            "general_portrait",
            "type",
            "flag",
            "bar_org",
            "no_orders",
            "counter_overlay",
            "group_marker",
        ] {
            writes.hide(Icon, name);
        }
        for part in [BARS, TYPE_SPRITE, MARKER, IDEOLOGY] {
            writes.hid(part);
        }
        return;
    }
    plate_write(&plate, writes);
    writes.show(Button, "count_bg");
    match active {
        Some(active) => {
            writes.frame(Button, "count_bg", if active { 2 } else { 1 });
            writes.wrote(PLATE_FRAME);
        }
        None => writes.unknown(PLATE_FRAME, ACTIVE_UNDECIDED),
    }
    writes.show(Icon, "counter_overlay");
    writes.hide(Icon, "large_flag");
    writes.show(TextBox, "count_txt");
    count_write(count, writes);
    match &flotilla.bars {
        NavalBars::Hidden => {
            writes.hide(Icon, "bar_str");
            writes.hide(Icon, "bar_org");
            writes.hid(BARS);
        }
        NavalBars::Fill(organisation, strength) => {
            writes.show(Icon, "bar_str");
            writes.show(Icon, "bar_org");
            bar_writes(ctx, window, (*organisation, *strength), writes);
        }
        NavalBars::Unknown(why) => {
            writes.show(Icon, "bar_str");
            writes.show(Icon, "bar_org");
            writes.unknown(BARS, why.clone());
        }
    }
    let ideology = if window.find(Icon, "flag").is_some() {
        match flotilla
            .flag_owner
            .and_then(|owner| ctx.art.flag(owner, false))
        {
            Some(texture) => {
                writes.show(Icon, "flag");
                writes.control(Icon, "flag", |bound| bound.texture = Some(texture));
            }
            None => {
                writes.hide(Icon, "flag");
            }
        }
        window
            .find(Icon, "counter_ideology")
            .map(|_| &flotilla.ideology)
    } else {
        None
    };
    match ideology {
        Some(Ok(Some(frame))) => {
            writes.frame(Icon, "counter_ideology", *frame);
            writes.unflag_and_show(Icon, "counter_ideology");
            writes.wrote(IDEOLOGY);
        }
        Some(Err(why)) => writes.unknown(IDEOLOGY, format!("counter_ideology is undecided: {why}")),
        Some(Ok(None)) | None => writes.hid(IDEOLOGY),
    }
    match flotilla.type_tint {
        Some(side) => {
            if flotilla.hidden {
                no_intel_sprite(ctx.gui, writes);
                writes.frame(Icon, "type", no_intel_frame(ctx));
            } else {
                select_hull(ctx.gui, &resolved_hull(flotilla), report, writes);
            }
            writes.show(Icon, "type");
            let tint = ctx.defines.type_colors.of(side);
            writes.control(Icon, "type", |bound| bound.color = Some(tint));
        }
        None => {
            writes.hide(Icon, "type");
            writes.hid(TYPE_SPRITE);
        }
    }
    bind_marker(flotilla, false, writes);
    bind_orders(flotilla, false, writes);
}

fn mission_item(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    state: &NavyState<'_>,
    plate: Result<[f32; 4], String>,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::{Container, Icon};
    let NavyState { flotilla, .. } = *state;
    match flotilla.flag_owner {
        None => writes.unknown(FLAG, PORT_FLAG_UNDECIDED),
        Some(owner) => {
            match ctx.art.flag(owner, false) {
                Some(texture) => {
                    writes.control(Icon, "flag", |bound| bound.texture = Some(texture));
                }
                None => {
                    writes.hide(Icon, "flag");
                }
            }
            writes.wrote(FLAG);
        }
    }
    plate_write(&plate, writes);
    writes.hide_and_flag(Icon, "bar_str");
    writes.hide_and_flag(Icon, "bar_org");
    let item_text = |total: Option<usize>, capital: bool| {
        let item = flotilla.item?;
        viewer_unit_reads::naval_count_approx::format_recorded_count(
            total.map(|total| total as i32),
            item.first,
            item.byte,
            capital,
            item.ships.map(|ships| ships as i32),
            &ctx.defines.naval_counts,
        )
    };
    let count = flotilla.item.and_then(|item| item_text(item.ships, false));
    let capitals = match (&flotilla.cells, flotilla.item) {
        (_, Some(item)) if item.ships == Some(0) => item_text(Some(0), true),
        (Some(cells), Some(_)) => item_text(cells.capitals, true),
        _ => None,
    };
    count_write(count, writes);
    capitals_write(capitals, writes);
    match &flotilla.bars {
        NavalBars::Hidden => writes.hid(BARS),
        NavalBars::Fill(organisation, strength) => {
            writes.unflag_and_show(Icon, "bar_str");
            writes.unflag_and_show(Icon, "bar_org");
            bar_writes(ctx, window, (*organisation, *strength), writes);
        }
        NavalBars::Unknown(why) => {
            writes.unflag_and_show(Icon, "bar_str");
            writes.unflag_and_show(Icon, "bar_org");
            writes.unknown(BARS, why.clone());
        }
    }
    bind_supply(ctx, flotilla.supply, true, writes);
    writes.frame(Icon, "type", no_intel_frame(ctx));
    if flotilla.no_intel_type() {
        no_intel_sprite(ctx.gui, writes);
    } else {
        select_hull(ctx.gui, &resolved_hull(flotilla), report, writes);
    }
    writes.hide_and_flag(Icon, "mission_type");
    if let Some(mission) = flotilla.mission {
        writes.unflag_and_show(Icon, "mission_type");
        if let Ok(frame) = i32::try_from(mission + 1) {
            writes.frame(Icon, "mission_type", frame);
        }
    }
    writes.hide_and_flag(Container, "spotting_bar_container");
    bind_carrier(ctx.gui, flotilla, true, report, writes);
    bind_orders(flotilla, true, writes);
    bind_pride(flotilla, true, writes);
    let selected = flotilla
        .members
        .first()
        .is_some_and(|&id| ctx.selected.contains(&Unit::TaskForce(id)));
    if selected {
        writes.unflag_and_show(Icon, "count_selected");
    } else {
        writes.hide_and_flag(Icon, "count_selected");
    }
    bind_marker(flotilla, true, writes);
}

fn count_write(count: Option<String>, writes: &mut Writes<'_>) {
    match count {
        Some(text) => {
            writes.control(native::Kind::TextBox, "count_txt", |bound| {
                bound.text = Some(text)
            });
            writes.wrote(COUNT);
        }
        None => writes.unknown(COUNT, SHIP_CENSUS_INCOMPLETE),
    }
}

fn capitals_write(capitals: Option<String>, writes: &mut Writes<'_>) {
    match capitals {
        Some(text) => {
            writes.control(native::Kind::TextBox, "capital_count_txt", |bound| {
                bound.text = Some(text)
            });
            writes.wrote(CAPITALS);
        }
        None => writes.unknown(CAPITALS, SHIPS_UNRECORDED),
    }
}

pub const ACTIVE_UNDECIDED: &str = "count_bg's frame is undecided: a member's mission \
     activity (TaskForceCounterInputs) is not recorded";

pub const PORT_FLAG_UNDECIDED: &str = "a mixed-owner port item's flag is undecided: the \
     Ships census its owners' strengths are summed over is incomplete";

fn flag_texture(atlas: &LazyFlags, texture: usize, uv: [f32; 4]) -> Texture {
    Texture {
        texture,
        uv,
        size: atlas.cell_size().map(|value| value as i32),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombatState {
    pub rotation: Option<Rotation>,
    pub hq: Result<bool, String>,
}

pub const PROGRESS: &str = "the combat progress bars";

pub const GRIDS: &str = "the combat grids";

pub const BG_FRAME: &str = "bg_btn's frame";

pub const RESULT_SPRITE: &str = "bg_btn's sprite";

pub fn result_calls(
    window: &native::Element,
    gui: &dyn native_gui::Resources,
    presentation: Result<(&'static str, u32), &str>,
    undeclared: &mut BTreeMap<&'static str, usize>,
) -> Update {
    use native::Kind::Button;
    let mut writes = Writes::new(window);
    match presentation {
        Ok((sprite, frame)) => {
            writes.control(Button, "bg_btn", |bound| {
                bound.sprite = Some(sprite.as_bytes().to_vec());
                bound.setter = native_gui::SpriteSetter::ButtonStandard;
            });
            if gui.sprite(sprite.as_bytes()).is_some() {
                writes.wrote(RESULT_SPRITE);
            } else {
                *undeclared.entry(sprite).or_default() += 1;
                writes.kept(RESULT_SPRITE);
            }
            writes.frame(
                Button,
                "bg_btn",
                i32::try_from(frame + 1).unwrap_or(i32::MAX),
            );
            writes.wrote(BG_FRAME);
        }
        Err(why) => {
            writes.unknown(RESULT_SPRITE, why);
            writes.unknown(BG_FRAME, why);
        }
    }
    writes.update()
}

pub const FRAME_BTN: &str = "frame_btn's naval-HQ frame";

#[derive(Clone, Debug, PartialEq)]
pub struct Rotation {
    pub engaged: Option<Engaged>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Engaged {
    pub variant: u32,
    pub progress: Result<CombatProgress, String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatProgress {
    pub small: bool,
    pub progress: i64,
    pub flip: bool,
}

impl CombatProgress {
    pub fn value(&self) -> i32 {
        let progress = if self.flip {
            100_000 - self.progress
        } else {
            self.progress
        };
        let fixed = progress.wrapping_mul(10_000_000) / 100_000;
        let rounded = if fixed < 0 {
            (fixed - 50_000) / 100_000
        } else {
            (fixed + 50_000) / 100_000
        };
        rounded as i32
    }
}

pub fn combat_calls(
    window: &native::Element,
    state: &CombatState,
    resources: &dyn native_gui::Resources,
) -> Update {
    use native::Kind::{Button, Icon};
    let mut writes = Writes::new(window);
    if let Some(rotation) = &state.rotation {
        let mut measured = true;
        writes.hide_and_flag(Icon, "progress");
        writes.hide_and_flag(Icon, "progress_small");
        if let Some(engaged) = &rotation.engaged {
            if let Ok(frame) = i32::try_from(engaged.variant + 1) {
                writes.frame(Button, "bg_btn", frame);
            }
            writes.wrote(BG_FRAME);
            if let Err(why) = &engaged.progress {
                measured = false;
                writes.unknown(
                    PROGRESS,
                    format!("icon omitted: its progress bar is undecided: {why}"),
                );
            }
            if let Ok(progress) = engaged.progress {
                let name = if progress.small {
                    "progress_small"
                } else {
                    "progress"
                };
                writes.unflag_and_show(Icon, name);
                let extent = match window.find(Icon, name) {
                    Some(element) if progress.flip => native_gui::drawable_extent(
                        element,
                        resources,
                        &native_gui::Bindings::default(),
                    )
                    .map_err(|refusals| {
                        refusals
                            .iter()
                            .map(|refusal| refused_cause(NAVAL_COMBAT_WINDOW, refusal))
                            .collect::<Vec<_>>()
                            .join("; ")
                    }),
                    _ => Ok(None),
                };
                let extent = match extent {
                    Ok(extent) => Some(extent),
                    Err(why) => {
                        measured = false;
                        writes.unknown(
                            PROGRESS,
                            format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not update: {why}"),
                        );
                        None
                    }
                };
                writes.control(Icon, name, |bar| {
                    if progress.flip {
                        bar.rotation = Some(std::f32::consts::PI);
                        if let Some(extent) = extent {
                            let [w, h] = extent.unwrap_or([0; 2]);
                            bar.offset = Some([w, h.wrapping_neg()]);
                        }
                    } else {
                        bar.rotation = Some(0.0);
                        bar.offset = Some([0; 2]);
                    }
                });
                writes.frame(Icon, name, progress.value());
            }
        }
        if measured {
            writes.wrote(PROGRESS);
        }
    }
    match &state.hq {
        Ok(true) => writes.unflag_and_show(Button, "frame_btn"),
        Ok(false) => writes.hide_and_flag(Button, "frame_btn"),
        Err(why) => writes.unknown(
            FRAME_BTN,
            format!("icon omitted: HQ frame undecided: {why}"),
        ),
    }
    if state.hq.is_ok() {
        writes.wrote(FRAME_BTN);
    }
    writes.update()
}

pub fn convoy_setup(tiny: &native::Element) -> Vec<(String, Call)> {
    let mut writes = Writes::new(tiny);
    writes.hide(native::Kind::Container, "spotting_bar_container");
    writes.finish()
}

pub fn convoy_calls(
    gui: &GuiResources,
    tiny: &native::Element,
    count: usize,
    tint: [f32; 4],
    gaps: &mut BTreeMap<Cow<'static, str>, usize>,
) -> Vec<(String, Call)> {
    use native::Kind::{Icon, TextBox};
    let mut writes = Writes::new(tiny);
    writes.control(Icon, "background", |bound| bound.color = Some(tint));
    writes.control(TextBox, "count", |bound| {
        bound.text = Some(count.to_string())
    });
    if tiny.find(Icon, "icon").is_some() {
        if gui
            .registry()
            .sprite(CONVOY_ADDITION_ICON.as_bytes())
            .is_some()
        {
            writes.control(Icon, "icon", |bound| {
                bound.sprite = Some(CONVOY_ADDITION_ICON.as_bytes().to_vec());
                bound.setter = native_gui::SpriteSetter::Icon;
            });
        } else {
            *gaps
                .entry(CONVOY_ICON_GAP.into())
                .or_default() += 1;
        }
    }
    writes.finish()
}

pub const CONVOY_WINDOW_ABSENT: &str =
    "a convoy addition's GFX_mapicon_tiny_unit_counter window is not declared; not drawn";

#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub fn window_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &content_reader_localisation::localisation::Localisation,
) -> native_gui::Assets {
    let mut assets = windows.assets(localisation);
    assets.sprites.extend(
        [
            CONVOY_ADDITION_ICON,
            counter_order::AIR_BASE_ICON,
            counter_order::ROCKET_SITE_ICON,
            NO_INTEL_SPRITE,
        ]
        .into_iter()
        .chain(NAVAL_RESULT_SPRITES)
        .filter(|name| registry.sprite(name.as_bytes()).is_some())
        .map(|name| name.as_bytes().to_vec()),
    );
    let (prefix, suffix) = (
        templates::UNIT_PREFIX.as_bytes(),
        templates::UNIT_SUFFIX.as_bytes(),
    );
    assets
        .sprites
        .extend(sprites_with_prefix(registry, prefix, suffix));
    assets.sprites.extend(sprites_with_prefix(
        registry,
        FACILITY_SPRITE_PREFIX.as_bytes(),
        b"",
    ));
    assets.sprites.extend(sprites_with_prefix(
        registry,
        STRATEGIC_LOCATION_SPRITE_PREFIX.as_bytes(),
        b"",
    ));
    if let Some(background) = windows
        .gui_type(LAND_COMBAT_WINDOW.as_bytes())
        .and_then(|root| root.find(native::Kind::Button, "bg_btn"))
        && let native::Fields::Button(button) = &background.fields
    {
        let mut prefix = button.sprite.clone();
        prefix.push(b'_');
        assets
            .sprites
            .extend(sprites_with_prefix(registry, &prefix, b""));
    }
    assets
        .sprites
        .extend(sprites_with_prefix(registry, b"", EMBLEM_SUFFIX.as_bytes()));
    assets
}

fn sprites_with_prefix<'a>(
    registry: &'a native::resources::Registry,
    prefix: &'a [u8],
    suffix: &'a [u8],
) -> impl Iterator<Item = Vec<u8>> + 'a {
    registry
        .files
        .iter()
        .flat_map(|(_, file)| &file.sprites)
        .filter(move |sprite| sprite.name.starts_with(prefix) && sprite.name.ends_with(suffix))
        .map(|sprite| sprite.name.clone())
}
