
use super::{
    Drawn, HoverText, Values, Widget,
    binding::Bindings,
    gui::{self, Group, Retained, Row},
    gui_resources,
};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_native_gui::native_gui::{self, lifecycle, pointer::Input};

pub const PICKER_WINDOW: &str = "MapmodesInterface_Ingame";
pub const ALL_MODES_WINDOW: &str = "WindowWithAllMapModes_Standalone";
pub const SLOTS_WINDOW: &str = "WindowWithMapModeSlots";
pub const ASSIGN_WINDOW: &str = "WindowWithAllMapModes";
pub const TEMPLATES: &[&[u8]] = &[
    b"MapmodesInterface_Ingame",
    b"ConfigurableMapModeSlot",
    b"ConfigurableMapMode",
    b"WindowWithMapModeSlots",
    b"WindowWithAllMapModes",
    b"WindowWithAllMapModes_Standalone",
];

#[derive(Clone, Debug, PartialEq)]
pub struct ModeButton {
    pub token: String,
    pub frame: Option<u32>,
    pub sprite_name: Option<String>,
    pub unavailable: Option<String>,
    pub name: String,
    pub tooltip: String,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Toggles {
    pub day_night: Option<bool>,
    pub player_counters: Option<bool>,
    pub counter_colors: Option<bool>,
    pub allied_plans: Option<bool>,
    pub fog: Option<bool>,
    pub radar: Option<bool>,
}

pub const TOGGLES: [&str; 6] = [
    "day_night_toggle",
    "player_counters_toggle",
    "button_counters_color_mode",
    "button_allied_plans",
    "button_fog",
    "button_radar",
];

impl Toggles {
    fn get(&self, name: &str) -> Option<bool> {
        match name {
            "day_night_toggle" => self.day_night,
            "player_counters_toggle" => self.player_counters,
            "button_counters_color_mode" => self.counter_colors,
            "button_allied_plans" => self.allied_plans,
            "button_fog" => self.fog,
            "button_radar" => self.radar,
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Open {
    #[default]
    None,
    All,
    Assign(usize),
}

pub struct State<'a> {
    pub four_buttons: bool,
    pub slots: &'a [ModeButton],
    pub configurable_from: usize,
    pub modes: &'a [ModeButton],
    pub selected: &'a str,
    pub toggles: Toggles,
    pub open: Open,
    pub click_info: &'a str,
    pub click_info_assign: &'a str,
    pub all_tooltip: &'a str,
    pub default_tooltip: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Slot(usize),
    All,
    Toggle(&'static str),
    Entry(String),
    SlotEntry(usize),
    Default,
    FindView,
    Minimap(super::minimap::Control),
    Find(super::find_view::Control),
    Window,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub rect: [f32; 4],
    pub control: Control,
    alpha: Option<std::sync::Arc<(native_gui::HitGeometry, [f32; 2])>>,
    pub click: Option<(String, Vec<u8>)>,
    pub disabled: bool,
}

impl Placed {
    pub(crate) fn new(rect: [f32; 4], control: Control) -> Self {
        Self {
            rect,
            control,
            alpha: None,
            click: None,
            disabled: false,
        }
    }

    pub(crate) fn region(
        rect: [f32; 4],
        control: Control,
        region: &native_gui::Region,
        origin: [i32; 2],
        click: Option<(String, Vec<u8>)>,
    ) -> Option<Self> {
        let form = region.hit.as_ref().map(|hit| hit.form);
        if form == Some(native_gui::Hit::Through) {
            return None;
        }
        let alpha = region
            .hit
            .as_ref()
            .filter(|hit| hit.form == native_gui::Hit::Alpha)
            .map(|hit| std::sync::Arc::new((hit.clone(), origin.map(|value| value as f32))));
        Some(Self {
            rect,
            control,
            alpha,
            click,
            disabled: region.disabled,
        })
    }

    pub fn contains(&self, at: [f32; 2]) -> bool {
        viewer_runtime::geometry::contains(self.rect, at)
            && self.alpha.as_ref().is_none_or(|alpha| {
                let (hit, origin) = alpha.as_ref();
                hit.contains([at[0] - origin[0], at[1] - origin[1]])
            })
    }
}

#[derive(Clone)]
pub struct PickerDrawn {
    pub drawn: Drawn,
    pub controls: Vec<Placed>,
    pub clicked: Vec<(Control, usize)>,
}

impl PickerDrawn {
    fn again(&self) -> Self {
        Self {
            drawn: self.drawn.again(),
            controls: self.controls.clone(),
            clicked: Vec::new(),
        }
    }

    pub fn control_at(&self, point: [f32; 2]) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|placed| placed.contains(point))
            .map(|placed| &placed.control)
    }
}

#[derive(Default)]
pub struct Picker {
    built: Option<(
        [i32; 2],
        viewer_native_gui::native_windows::NativeWindows,
    )>,
    root: Option<Retained>,
    list: Option<Retained>,
    slots: Option<Retained>,
    generation: u64,
    last: Option<u64>,
    root_generation: Option<u64>,
    values: Values,
    controls: BTreeMap<String, Control>,
    above: Vec<[f32; 4]>,
    owners: Vec<(Part, Placed)>,
    held: Option<(Shown, PickerDrawn)>,
}

#[derive(Clone, Copy, PartialEq)]
struct Shown {
    fingerprint: u64,
    viewport: [i32; 2],
    hovers: bool,
    advance: bool,
    place: Option<native_gui::pointer::Place>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Root,
    Slots,
    List,
}

fn hover(text: &str) -> Option<HoverText> {
    (!text.is_empty()).then(|| Arc::from(text))
}

fn button_name(button: usize, selected: bool) -> String {
    format!(
        "mapmode_button_{}{}",
        button + 1,
        if selected { "_s" } else { "" }
    )
}

pub fn button_of_slot(slot: usize, four_buttons: bool) -> Option<usize> {
    match slot {
        0..=2 => Some(slot),
        3 => four_buttons.then_some(3),
        _ => Some(slot),
    }
}

const BUTTONS: usize = 12;

impl Picker {
    pub fn holding(&self) -> bool {
        [&self.list, &self.slots, &self.root]
            .into_iter()
            .flatten()
            .any(Retained::holding)
    }

    pub fn highlighted(&self) -> Option<&str> {
        [&self.list, &self.slots, &self.root]
            .into_iter()
            .flatten()
            .find_map(Retained::highlighted)
    }

    pub fn pointing(&self) -> bool {
        [&self.root, &self.list, &self.slots]
            .into_iter()
            .flatten()
            .any(Retained::pointing)
    }

    fn animating(&self) -> bool {
        [&self.root, &self.list, &self.slots]
            .into_iter()
            .flatten()
            .any(Retained::active)
    }

    fn refresh_values(&mut self, state: &State<'_>, root: &content_reader_gui::gui::native::Element) {
        if self.root_generation == Some(self.generation) {
            return;
        }
        self.values = picker_values(state, root);
        self.controls = gui::elements(root)
            .into_iter()
            .filter_map(|(path, element)| {
                let leaf = path.rsplit('/').next().unwrap_or(&path);
                picker_control(leaf, state).map(|control| (element.path.clone(), control))
            })
            .collect();
        self.root_generation = Some(self.generation);
    }

    #[expect(clippy::too_many_arguments, reason = "the content and state, the window's extent, and the frame's clock and input")]
    pub fn draw(
        &mut self,
        content: &super::content::Content,
        state: &State<'_>,
        fingerprint: u64,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
    ) -> io::Result<PickerDrawn> {
        let viewport = extent.map(|value| value as i32);
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        if self
            .built
            .as_ref()
            .is_none_or(|(size, built)| *size != viewport || !built.same(&windows))
        {
            *self = Self {
                built: Some((viewport, windows.clone())),
                ..Self::default()
            };
        }
        if self.last != Some(fingerprint) {
            self.last = Some(fingerprint);
            self.generation += 1;
        }
        let shown = Shown {
            fingerprint,
            viewport,
            hovers,
            advance,
            place: input.as_ref().and_then(|input| input.place),
        };
        let still = input
            .as_ref()
            .is_none_or(|input| input.buttons.is_empty() && !input.held);
        if still
            && !self.animating()
            && let Some((held, drawn)) = &self.held
            && *held == shown
        {
            return Ok(drawn.again());
        }
        self.held = None;
        let assets = content.assets.as_ref();
        let inventory = gui_resources::GuiAssets::new(assets, &Bindings::default());
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &content.localisation,
            &content.small_flags,
            &content.large_flags,
            &inventory,
        );
        let tick = lifecycle::Tick { time, dt: 0.0 };
        let mut out = PickerDrawn {
            drawn: Drawn::default(),
            controls: Vec::new(),
            clicked: Vec::new(),
        };
        let root = gui::template(&windows, PICKER_WINDOW).map_err(super::invalid)?;
        self.refresh_values(state, root);
        let holder = [
            (Part::Root, &self.root),
            (Part::Slots, &self.slots),
            (Part::List, &self.list),
        ]
        .into_iter()
        .find(|(_, window)| window.as_ref().is_some_and(Retained::holding))
        .map(|(part, _)| part);
        let owners = std::mem::take(&mut self.owners);
        let owner = |at: [f32; 2]| {
            owners
                .iter()
                .rev()
                .find(|(_, placed)| placed.contains(at))
                .map(|(part, _)| *part)
        };
        let shared = |me: Part| {
            input
                .as_ref()
                .map(|input| input.shared(holder, owner, me))
        };
        let above = std::mem::take(&mut self.above);
        let root_input = shared(Part::Root).map(|input| {
            input.filter(|at| {
                !above
                    .iter()
                    .any(|&rect| viewer_runtime::geometry::contains(rect, at))
            })
        });
        let values = &self.values;
        let generation = self.generation;
        let retained = gui::ensure(&mut self.root, PICKER_WINDOW, root, viewport, &resources)
            .map_err(super::invalid)?;
        let frame = native_gui::container_frame(root, Some(viewport)).map_err(super::invalid)?;
        retained
            .instance
            .set_position(root, root, frame.at)
            .map_err(super::invalid)?;
        let Some(prepared) = retained.prepare(
            root,
            values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            generation,
            advance,
            hovers,
            true,
            root_input
                .as_ref()
                .map(|input| input.local(frame.at.map(|value| value as f32))),
            &mut out.drawn.refused,
        ) else {
            return Ok(out);
        };
        out.drawn.sounds.extend(prepared.sounds.iter().cloned());
        out.clicked.extend(
            prepared
                .clicks
                .iter()
                .filter(|(key, _, _)| key.is_none())
                .filter_map(|(_, path, order)| Some((self.controls.get(path)?.clone(), *order))),
        );
        let origin = frame.at;
        if !gui::draw_prepared(
            &prepared.layout,
            &prepared.items,
            origin,
            &resources,
            &mut out.drawn,
        ) {
            return Ok(out);
        }
        for region in &prepared.layout.regions {
            let Some(rect) = gui::region_rect(region, origin) else {
                continue;
            };
            if let Some(control) = self.controls.get(&region.path) {
                let click = gui::click_sound(root, &region.path, &resources);
                out.controls
                    .extend(Placed::region(rect, control.clone(), region, origin, click));
            }
        }
        if hovers {
            gui::append_hovers(&mut out.drawn.tooltips, prepared.hovers, origin);
        }
        let root_controls = out.controls.len();
        let mut slot_controls = root_controls;
        match state.open {
            Open::None => {
                self.list = None;
                self.slots = None;
            }
            Open::All => {
                self.slots = None;
                draw_list(
                    &mut self.list,
                    ALL_MODES_WINDOW,
                    state,
                    None,
                    &windows,
                    &resources,
                    viewport,
                    tick,
                    generation,
                    advance,
                    hovers,
                    shared(Part::List),
                    &mut out,
                )?;
            }
            Open::Assign(slot) => {
                draw_slots(
                    &mut self.slots,
                    state,
                    slot,
                    &windows,
                    &resources,
                    viewport,
                    tick,
                    generation,
                    advance,
                    hovers,
                    shared(Part::Slots),
                    &mut out,
                )?;
                slot_controls = out.controls.len();
                draw_list(
                    &mut self.list,
                    ASSIGN_WINDOW,
                    state,
                    Some(slot),
                    &windows,
                    &resources,
                    viewport,
                    tick,
                    generation,
                    advance,
                    hovers,
                    shared(Part::List),
                    &mut out,
                )?;
            }
        }
        self.owners = out
            .controls
            .iter()
            .enumerate()
            .map(|(at, placed)| {
                let part = if at < root_controls {
                    Part::Root
                } else if at < slot_controls {
                    Part::Slots
                } else {
                    Part::List
                };
                (part, placed.clone())
            })
            .collect();
        self.above = out
            .controls
            .iter()
            .filter(|placed| placed.control == Control::Window)
            .map(|placed| placed.rect)
            .collect();
        self.held = Some((shown, out.again()));
        Ok(out)
    }
}

fn picker_control(leaf: &str, state: &State<'_>) -> Option<Control> {
    if leaf == "mapmodes_all" {
        return Some(Control::All);
    }
    if leaf == "open_find_view_button" {
        return Some(Control::FindView);
    }
    if let Some(toggle) = TOGGLES.iter().find(|name| **name == leaf) {
        return Some(Control::Toggle(toggle));
    }
    let number = leaf
        .strip_prefix("mapmode_button_")?
        .trim_end_matches("_s")
        .parse::<usize>()
        .ok()?;
    let button = number.checked_sub(1)?;
    (0..state.slots.len())
        .find(|&slot| button_of_slot(slot, state.four_buttons) == Some(button))
        .map(Control::Slot)
}

fn layout_name(four_buttons: bool) -> &'static str {
    if four_buttons {
        "4_button_mapmode"
    } else {
        "3_button_mapmode"
    }
}

fn authored_shortcut(root: &content_reader_gui::gui::native::Element, path: &str) -> Option<String> {
    let shortcut = root.path_named(path)?.button()?.shortcuts.first()?;
    std::str::from_utf8(shortcut).ok().map(str::to_owned)
}

pub fn shortcuts(
    content: &super::content::Content,
    extent: [f32; 2],
    four_buttons: bool,
    slots: usize,
) -> io::Result<Vec<(Control, String)>> {
    let windows = content.window_cache.at(extent.map(|value| value as u32))?;
    let root = gui::template(&windows, PICKER_WINDOW).map_err(super::invalid)?;
    Ok(shortcuts_in(root, four_buttons, slots))
}

fn shortcuts_in(
    root: &content_reader_gui::gui::native::Element,
    four_buttons: bool,
    slots: usize,
) -> Vec<(Control, String)> {
    let layout = layout_name(four_buttons);
    let mut out = Vec::new();
    for slot in 0..slots {
        if let Some(button) = button_of_slot(slot, four_buttons)
            && let Some(shortcut) =
                authored_shortcut(root, &format!("{layout}/{}", button_name(button, false)))
        {
            out.push((Control::Slot(slot), shortcut));
        }
    }
    for name in TOGGLES {
        if let Some(shortcut) = authored_shortcut(root, name) {
            out.push((Control::Toggle(name), shortcut));
        }
    }
    if let Some(shortcut) = authored_shortcut(root, "mapmodes_all") {
        out.push((Control::All, shortcut));
    }
    if let Some(shortcut) = authored_shortcut(root, "open_find_view_button") {
        out.push((Control::FindView, shortcut));
    }
    out
}

fn picker_values(state: &State<'_>, root: &content_reader_gui::gui::native::Element) -> Values {
    let mut values = Values::new();
    let layout = layout_name(state.four_buttons);
    let other = layout_name(!state.four_buttons);
    values.insert(
        other.to_owned(),
        Widget {
            hidden: true,
            ..Default::default()
        },
    );
    for hidden in [
        "social_view_interface_window",
        "tutorial_highlight_default_mapmode",
        "logistics_tutorial_glow",
        "logistics_tutorial_glow_1",
        "logistics_tutorial_glow_2",
    ] {
        let path = if hidden.starts_with("social") {
            hidden.to_owned()
        } else {
            format!("{layout}/{hidden}")
        };
        values.insert(
            path,
            Widget {
                hidden: true,
                ..Default::default()
            },
        );
    }
    let mut used = [false; BUTTONS];
    for (slot, mode) in state.slots.iter().enumerate() {
        let Some(button) = button_of_slot(slot, state.four_buttons) else {
            continue;
        };
        if button >= BUTTONS {
            continue;
        }
        used[button] = true;
        let selected = mode.token == state.selected;
        let large = button < if state.four_buttons { 4 } else { 3 };
        let tooltip = if slot >= state.configurable_from {
            state.click_info_assign
        } else {
            state.click_info
        };
        for shown in [false, true] {
            let path = format!("{layout}/{}", button_name(button, shown));
            let shortcut = authored_shortcut(root, &path)
                .map(|shortcut| super::tooltips::shortcut_keycaps(&shortcut))
                .unwrap_or_default();
            let mut widget = Widget {
                hidden: shown != selected,
                enabled: mode.unavailable.is_some().then_some(false),
                tooltip: hover(&format!(
                    "{}\n{}",
                    mode.tooltip,
                    mode.unavailable.as_deref().unwrap_or(tooltip)
                )),
                tooltip_description: hover(&mode.description),
                tooltip_shortcut: hover(&shortcut),
                hover_owned: true,
                ..Default::default()
            };
            if let (true, Some(frame)) = (large, mode.frame) {
                widget.frame = Some(frame);
            } else if let Some(frame) = mode.frame {
                widget.frame = Some(frame);
                widget.sprite = Some(
                    if shown {
                        "GFX_mapmode_buttons_selected_small"
                    } else {
                        "GFX_mapmode_buttons_deselected_small"
                    }
                    .to_owned(),
                );
            } else if let Some(name) = &mode.sprite_name {
                widget.sprite = Some(format!(
                    "GFX_mapmode_buttons_{}_small_{name}",
                    if shown { "selected" } else { "deselected" }
                ));
            }
            values.insert(path, widget);
        }
    }
    for (button, used) in used.into_iter().enumerate() {
        if !used {
            for shown in [false, true] {
                values.insert(
                    format!("{layout}/{}", button_name(button, shown)),
                    Widget {
                        hidden: true,
                        ..Default::default()
                    },
                );
            }
        }
    }
    for name in TOGGLES {
        let state = state.toggles.get(name);
        values.insert(
            name.to_owned(),
            Widget {
                enabled: Some(state.is_some()),
                frame: Some(u32::from(state == Some(true))),
                ..Default::default()
            },
        );
    }
    values.insert(
        "mapmodes_all".to_owned(),
        Widget {
            tooltip: hover(state.all_tooltip),
            hover_owned: true,
            ..Default::default()
        },
    );
    values
}

fn entry_values(mode: &ModeButton, selected: bool, shortcut: Option<&str>) -> Values {
    let mut values = Values::new();
    values.insert(
        "name".to_owned(),
        Widget {
            text: Some(mode.name.clone()),
            ..Default::default()
        },
    );
    values.insert(
        "bg".to_owned(),
        Widget {
            enabled: mode.unavailable.is_some().then_some(false),
            tooltip: hover(&match &mode.unavailable {
                Some(why) => format!("{}\n{why}", mode.tooltip),
                None => mode.tooltip.clone(),
            }),
            tooltip_description: hover(&mode.description),
            hover_owned: true,
            ..Default::default()
        },
    );
    values.insert(
        "shortcut".to_owned(),
        Widget {
            text: Some(shortcut.unwrap_or_default().to_owned()),
            ..Default::default()
        },
    );
    for (name, shown) in [("icon_selected", true), ("icon_deselected", false)] {
        let mut widget = Widget {
            hidden: shown != selected,
            ..Default::default()
        };
        match (&mode.frame, &mode.sprite_name) {
            (Some(frame), _) => widget.frame = Some(*frame),
            (None, Some(name)) => {
                widget.sprite = Some(format!(
                    "GFX_mapmode_buttons_{}_small_{name}",
                    if shown { "selected" } else { "deselected" }
                ))
            }
            (None, None) => {}
        }
        values.insert(name.to_owned(), widget);
    }
    values
}

#[expect(clippy::too_many_arguments, reason = "the retained list, its state and assignment, the window's resources, and the frame's clock and input")]
fn draw_list(
    kept: &mut Option<Retained>,
    name: &str,
    state: &State<'_>,
    assigning: Option<usize>,
    windows: &viewer_native_gui::native_windows::NativeWindows,
    resources: &gui_resources::GuiResources<'_>,
    viewport: [i32; 2],
    tick: lifecycle::Tick,
    generation: u64,
    advance: bool,
    hovers: bool,
    input: Option<Input>,
    out: &mut PickerDrawn,
) -> io::Result<()> {
    let scripted = state
        .modes
        .iter()
        .filter(|mode| mode.frame.is_none())
        .count();
    let root = &grown(
        gui::template(windows, name).map_err(super::invalid)?,
        scripted,
    )
    .map_err(super::invalid)?;
    let retained = gui::ensure(kept, name, root, viewport, resources).map_err(super::invalid)?;
    let selected = |mode: &ModeButton| match assigning {
        Some(slot) => state
            .slots
            .get(slot)
            .is_some_and(|held| held.token == mode.token),
        None => mode.token == state.selected,
    };
    let picker = gui::template(windows, PICKER_WINDOW).map_err(super::invalid)?;
    let layout = layout_name(state.four_buttons);
    let slot_shortcut = |mode: &ModeButton| {
        let slot = state
            .slots
            .iter()
            .position(|held| held.token == mode.token)?;
        let button = button_of_slot(slot, state.four_buttons)?;
        authored_shortcut(picker, &format!("{layout}/{}", button_name(button, false)))
    };
    let shortcuts: Vec<Option<String>> = state.modes.iter().map(slot_shortcut).collect();
    let values: Vec<Values> = state
        .modes
        .iter()
        .zip(&shortcuts)
        .map(|(mode, shortcut)| entry_values(mode, selected(mode), shortcut.as_deref()))
        .collect();
    let groups = vec![Group {
        path: "slots".into(),
        rows: state
            .modes
            .iter()
            .zip(&values)
            .map(|(mode, values)| Row {
                naval_layout: None,
                key: mode.token.clone(),
                template: "ConfigurableMapMode",
                values,
                explicit: None,
            })
            .collect(),
    }];
    let window_values = Values::new();
    draw_window(
        retained,
        root,
        &window_values,
        groups,
        windows,
        resources,
        viewport,
        tick,
        generation,
        advance,
        hovers,
        input,
        out,
        |key| Some(Control::Entry(key.to_owned())),
    )
}

#[expect(clippy::too_many_arguments, reason = "the retained slots, their state and assignment, the window's resources, and the frame's clock and input")]
fn draw_slots(
    kept: &mut Option<Retained>,
    state: &State<'_>,
    assigning: usize,
    windows: &viewer_native_gui::native_windows::NativeWindows,
    resources: &gui_resources::GuiResources<'_>,
    viewport: [i32; 2],
    tick: lifecycle::Tick,
    generation: u64,
    advance: bool,
    hovers: bool,
    input: Option<Input>,
    out: &mut PickerDrawn,
) -> io::Result<()> {
    let root = gui::template(windows, SLOTS_WINDOW).map_err(super::invalid)?;
    let retained =
        gui::ensure(kept, SLOTS_WINDOW, root, viewport, resources).map_err(super::invalid)?;
    let configurable: Vec<_> = state
        .slots
        .iter()
        .enumerate()
        .skip(state.configurable_from)
        .collect();
    let values: Vec<Values> = configurable
        .iter()
        .map(|(slot, mode)| {
            let mut values = Values::new();
            values.insert(
                "name".to_owned(),
                Widget {
                    text: Some(mode.name.clone()),
                    ..Default::default()
                },
            );
            values.insert(
                "bg".to_owned(),
                Widget {
                    frame: Some(u32::from(*slot == assigning)),
                    ..Default::default()
                },
            );
            values
        })
        .collect();
    let groups = vec![Group {
        path: "slots".into(),
        rows: configurable
            .iter()
            .zip(&values)
            .map(|((slot, _), values)| Row {
                naval_layout: None,
                key: slot.to_string(),
                template: "ConfigurableMapModeSlot",
                values,
                explicit: None,
            })
            .collect(),
    }];
    let mut window_values = Values::new();
    window_values.insert(
        "default".to_owned(),
        Widget {
            tooltip: hover(state.default_tooltip),
            hover_owned: true,
            ..Default::default()
        },
    );
    draw_window(
        retained,
        root,
        &window_values,
        groups,
        windows,
        resources,
        viewport,
        tick,
        generation,
        advance,
        hovers,
        input,
        out,
        |key| key.parse().ok().map(Control::SlotEntry),
    )
}

fn grown(
    root: &content_reader_gui::gui::native::Element,
    rows: usize,
) -> Result<content_reader_gui::gui::native::Element, String> {
    use content_reader_gui::gui::native::Fields;
    if rows == 0 {
        return Ok(root.clone());
    }
    let slot = root
        .path_named("slots")
        .and_then(|slots| slots.grid())
        .ok_or_else(|| format!("{} has no slots grid", root.path))?
        .slot_size
        .size[1];
    if slot.percent || slot.from_position || slot.auto {
        return Err(format!(
            "{}'s slot height is relative, which the list's growth does not measure",
            root.path
        ));
    }
    let extra = i32::try_from(rows)
        .ok()
        .and_then(|rows| rows.checked_mul(slot.value))
        .ok_or_else(|| format!("{} cannot grow by {rows} rows", root.path))?;
    let mut root = root.clone();
    let Fields::Container(container) = &mut root.fields else {
        return Err(format!("{} is not a containerWindowType", root.path));
    };
    container.size.size[1].value += extra;
    for position in [
        &mut container.position,
        &mut container.show_position,
        &mut container.hide_position,
    ] {
        position.y.value -= extra;
    }
    Ok(root)
}

#[expect(clippy::too_many_arguments, reason = "the retained window, its root, values and groups, its resources, and the frame's clock and input")]
fn draw_window(
    retained: &mut Retained,
    root: &content_reader_gui::gui::native::Element,
    values: &Values,
    groups: Vec<Group<'_>>,
    windows: &viewer_native_gui::native_windows::NativeWindows,
    resources: &gui_resources::GuiResources<'_>,
    viewport: [i32; 2],
    tick: lifecycle::Tick,
    generation: u64,
    advance: bool,
    hovers: bool,
    input: Option<Input>,
    out: &mut PickerDrawn,
    entry: impl Fn(&str) -> Option<Control>,
) -> io::Result<()> {
    let frame = native_gui::container_frame(root, Some(viewport)).map_err(super::invalid)?;
    retained
        .instance
        .set_position(root, root, frame.at)
        .map_err(super::invalid)?;
    let Some(prepared) = retained.prepare(
        root,
        values,
        groups,
        windows,
        resources,
        tick,
        generation,
        advance,
        hovers,
        true,
        input
            .as_ref()
            .map(|input| input.local(frame.at.map(|value| value as f32))),
        &mut out.drawn.refused,
    ) else {
        return Ok(());
    };
    out.drawn.sounds.extend(prepared.sounds.iter().cloned());
    let origin = frame.at;
    if !gui::draw_prepared(
        &prepared.layout,
        &prepared.items,
        origin,
        resources,
        &mut out.drawn,
    ) {
        return Ok(());
    }
    out.controls.push(Placed::new(
        [
            origin[0] as f32,
            origin[1] as f32,
            prepared.layout.size[0] as f32,
            prepared.layout.size[1] as f32,
        ],
        Control::Window,
    ));
    let named: BTreeMap<_, _> = gui::elements(root)
        .into_iter()
        .map(|(path, element)| (element.path.clone(), path))
        .collect();
    for region in &prepared.layout.regions {
        if named.get(&region.path).map(String::as_str) == Some("default")
            && let Some(rect) = gui::region_rect(region, origin)
        {
            let click = gui::click_sound(root, &region.path, resources);
            out.controls
                .extend(Placed::region(rect, Control::Default, region, origin, click));
        }
    }
    for (_, key, region) in &prepared.entries {
        if let Some(rect) = gui::region_rect(region, origin)
            && let Some(control) = entry(key)
        {
            out.controls
                .extend(Placed::region(rect, control, region, origin, None));
        }
    }
    out.clicked
        .extend(prepared.clicks.iter().filter_map(|(key, path, order)| {
            let control = match key {
                Some(key) => entry(key),
                None => (named.get(path).map(String::as_str) == Some("default"))
                    .then_some(Control::Default),
            };
            Some((control?, *order))
        }));
    if hovers {
        gui::append_hovers(&mut out.drawn.tooltips, prepared.hovers, origin);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_take_the_large_buttons_then_the_small_ones() {
        assert_eq!(button_of_slot(0, false), Some(0));
        assert_eq!(button_of_slot(3, true), Some(3));
        assert_eq!(button_of_slot(3, false), None);
        assert_eq!(button_of_slot(4, false), Some(4));
        assert_eq!(button_of_slot(10, true), Some(10));
    }

    #[test]
    fn the_topmost_control_under_the_pointer_takes_the_click() {
        let drawn = PickerDrawn {
            drawn: Drawn::default(),
            controls: vec![
                Placed::new([0.0, 0.0, 100.0, 50.0], Control::Window),
                Placed::new([10.0, 10.0, 20.0, 20.0], Control::Slot(1)),
            ],
            clicked: Vec::new(),
        };
        assert_eq!(drawn.control_at([15.0, 15.0]), Some(&Control::Slot(1)));
        assert_eq!(drawn.control_at([60.0, 15.0]), Some(&Control::Window));
        assert_eq!(drawn.control_at([30.0, 15.0]), Some(&Control::Window));
        assert_eq!(drawn.control_at([100.0, 15.0]), None);
    }

    #[test]
    fn the_find_view_button_opens_the_find_view_by_click_and_shortcut() {
        content_reader_script::test_game::enter();
        let windows = viewer_native_gui::native_windows::NativeWindows::read(
            b"guiTypes = { containerWindowType = { name = unit_counter_navy }               containerWindowType = { name = naval_combat_mapicon }               containerWindowType = { name = MapmodesInterface_Ingame                 buttonType = { name = open_find_view_button shortcut = \"ctrl+f\" } } }",
            [1280, 720],
        )
        .unwrap();
        let root = gui::template(&windows, PICKER_WINDOW).unwrap();
        let state = State {
            four_buttons: false,
            slots: &[],
            configurable_from: 4,
            modes: &[],
            selected: "default",
            toggles: Toggles::default(),
            open: Open::None,
            click_info: "",
            click_info_assign: "",
            all_tooltip: "",
            default_tooltip: "",
        };
        assert!(
            picker_values(&state, root)
                .get("open_find_view_button")
                .is_none_or(|widget| widget.enabled != Some(false))
        );
        assert_eq!(
            picker_control("open_find_view_button", &state),
            Some(Control::FindView)
        );
        assert!(
            shortcuts_in(root, false, 0).contains(&(Control::FindView, "ctrl+f".to_owned()))
        );
    }

    #[test]
    fn buttons_show_their_modes_frame_whatever_the_slot() {
        content_reader_script::test_game::enter();
        let windows = viewer_native_gui::native_windows::NativeWindows::read(
            b"guiTypes = { containerWindowType = { name = unit_counter_navy } \
              containerWindowType = { name = naval_combat_mapicon } \
              containerWindowType = { name = MapmodesInterface_Ingame } }",
            [1280, 720],
        )
        .unwrap();
        let root = gui::template(&windows, PICKER_WINDOW).unwrap();
        let mode = |token: &str, frame| ModeButton {
            token: token.to_owned(),
            frame: Some(frame),
            sprite_name: None,
            unavailable: None,
            name: String::new(),
            tooltip: String::new(),
            description: String::new(),
        };
        let slots = [
            mode("default", 0),
            mode("navy", 2),
            mode("air", 1),
            mode("supply", 5),
        ];
        let state = State {
            four_buttons: false,
            slots: &slots,
            configurable_from: 4,
            modes: &[],
            selected: "default",
            toggles: Toggles::default(),
            open: Open::None,
            click_info: "",
            click_info_assign: "",
            all_tooltip: "",
            default_tooltip: "",
        };
        let mut picker = Picker::default();
        picker.refresh_values(&state, root);
        let cached = &raw const picker.values["day_night_toggle"];
        picker.refresh_values(&state, root);
        assert_eq!(&raw const picker.values["day_night_toggle"], cached);
        let snapshot = |values: &Values| {
            values
                .iter()
                .map(|(path, widget)| {
                    (
                        path.clone(),
                        widget.frame,
                        widget.hidden,
                        widget.enabled,
                        widget.text.clone(),
                        widget.sprite.clone(),
                        widget.tooltip.clone(),
                        widget.tooltip_description.clone(),
                        widget.tooltip_shortcut.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            snapshot(&picker.values),
            snapshot(&picker_values(&state, root))
        );
        picker.generation += 1;
        let changed = State {
            toggles: Toggles {
                day_night: Some(true),
                ..Toggles::default()
            },
            ..state
        };
        picker.refresh_values(&changed, root);
        assert_eq!(
            snapshot(&picker.values),
            snapshot(&picker_values(&changed, root))
        );
        let state = changed;
        let values = picker_values(&state, root);
        let frame = |button, selected| {
            values[&format!("{}/{}", layout_name(false), button_name(button, selected))].frame
        };
        assert_eq!(frame(0, true), Some(0));
        assert_eq!(frame(1, false), Some(2));
        assert_eq!(frame(2, false), Some(1));
        assert!(values[&format!("{}/{}", layout_name(false), button_name(3, false))].hidden);
        assert_eq!(frame(0, false), Some(0));
        let mut unavailable = slots.clone();
        unavailable[1].unavailable = Some("Not available".to_owned());
        let values = picker_values(
            &State {
                slots: &unavailable,
                ..state
            },
            root,
        );
        let navy = &values[&format!("{}/{}", layout_name(false), button_name(1, false))];
        assert_eq!(navy.enabled, Some(false));
        assert!(navy.tooltip.as_deref().unwrap().ends_with(
            "
Not available"
        ));
        let entry = &entry_values(&unavailable[1], false, None)["bg"];
        assert_eq!(entry.enabled, Some(false));
    }
}
