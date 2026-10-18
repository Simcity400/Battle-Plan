//! The game's map mode picker (`MapmodesInterface_Ingame`) and its configuration windows,
//! hosted through the same retained native GUI walk as the combat windows.
//!
//! The caller decides what each button holds; this module binds the authored window to it,
//! lays it out bottom right as the game does and reports where each control landed.
//!
//! - `4_button_mapmode` shows when the recording owns the feature that adds the fourth large
//!   button, `3_button_mapmode` otherwise.
//! - Each picker slot is a button pair: `mapmode_button_<n>` while its mode is not shown and
//!   `mapmode_button_<n>_s` while it is. Large buttons take the slot's frame of the large sprite
//!   strips; small ones the mode's frame of the small strips, or a scripted mode's own
//!   `GFX_mapmode_buttons_(de)selected_small_<name>` sprite.
//! - The toggles bind their authored buttons; a toggle the caller cannot apply is disabled.
//! - `WindowWithAllMapModes_Standalone` lists every mode for `mapmodes_all`; right-clicking a
//!   configurable slot opens `WindowWithMapModeSlots` with `WindowWithAllMapModes` beside it.

use super::{
    Drawn, HoverText, TooltipAnchor, Values, Widget,
    binding::Bindings,
    gui::{self, Group, Retained, Row},
    gui_resources,
};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::map::native_gui::{self, lifecycle};

/// The picker's root window.
pub const PICKER_WINDOW: &str = "MapmodesInterface_Ingame";
/// The window listing every mode for `mapmodes_all`.
pub const ALL_MODES_WINDOW: &str = "WindowWithAllMapModes_Standalone";
/// The configurable slots and the list beside them while one is reassigned.
pub const SLOTS_WINDOW: &str = "WindowWithMapModeSlots";
pub const ASSIGN_WINDOW: &str = "WindowWithAllMapModes";
/// Every template the picker draws, for the GUI resource inventory.
pub const TEMPLATES: &[&[u8]] = &[
    b"MapmodesInterface_Ingame",
    b"ConfigurableMapModeSlot",
    b"ConfigurableMapMode",
    b"WindowWithMapModeSlots",
    b"WindowWithAllMapModes",
    b"WindowWithAllMapModes_Standalone",
];

/// One mode as a button or list entry shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeButton {
    /// The protocol token the caller selects with.
    pub token: String,
    /// The frame of the large and small button strips: the picker mode's number; `None` for
    /// a scripted mode, which has its own sprites.
    pub frame: Option<u32>,
    /// A scripted mode's sprite name suffix.
    pub sprite_name: Option<String>,
    /// Why the mode cannot be chosen: its button and entry show disabled with this reason.
    pub unavailable: Option<String>,
    /// The localised name the lists show.
    pub name: String,
    pub tooltip: String,
    pub description: String,
}

/// A toggle's state; `None` disables the button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toggles {
    pub day_night: Option<bool>,
    pub player_counters: Option<bool>,
    pub counter_colors: Option<bool>,
    pub allied_plans: Option<bool>,
    pub fog: Option<bool>,
    pub radar: Option<bool>,
}

/// The toggle buttons by authored name.
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

/// Which list window is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Open {
    #[default]
    None,
    /// The `mapmodes_all` list.
    All,
    /// Reassigning this configurable slot.
    Assign(usize),
}

/// What the picker shows this frame.
pub struct State<'a> {
    /// Whether the fourth large button exists.
    pub four_buttons: bool,
    /// The mode of each slot, in slot order.
    pub slots: &'a [ModeButton],
    /// The first slot the user can reassign; earlier ones are fixed.
    pub configurable_from: usize,
    /// Every mode the lists offer, in the game's order.
    pub modes: &'a [ModeButton],
    pub selected: &'a str,
    pub toggles: Toggles,
    pub open: Open,
    pub click_info: String,
    pub click_info_assign: String,
    pub all_tooltip: String,
    pub default_tooltip: String,
}

/// A clickable control and where it landed, in interface pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A picker slot's button.
    Slot(usize),
    All,
    Toggle(&'static str),
    /// An entry of the open list: its mode token.
    Entry(String),
    /// A slot entry of the configuration window.
    SlotEntry(usize),
    Default,
    /// The window background: absorbs clicks without an action.
    Window,
}

/// One frame of the picker: its quads and tooltips, and its controls in paint order.
pub struct PickerDrawn {
    pub drawn: Drawn,
    pub controls: Vec<([f32; 4], Control)>,
}

impl PickerDrawn {
    /// The topmost control under `point`.
    pub fn control_at(&self, point: [f32; 2]) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|(rect, _)| viewer_foundation::map::gui_resources::contains(*rect, point))
            .map(|(_, control)| control)
    }
}

/// The picker's retained windows; a new content or viewport rebuilds them.
#[derive(Default)]
pub struct Picker {
    built: Option<(
        [i32; 2],
        viewer_foundation::map::native_windows::NativeWindows,
    )>,
    root: Option<Retained>,
    list: Option<Retained>,
    slots: Option<Retained>,
    /// Bumps when what the windows show changes, so they bind again.
    generation: u64,
    last: Option<u64>,
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

/// The picker button a slot occupies: the first three or four slots are the large buttons,
/// the rest follow the small ones from the fifth button on.
pub fn button_of_slot(slot: usize, four_buttons: bool) -> Option<usize> {
    match slot {
        0..=2 => Some(slot),
        3 => four_buttons.then_some(3),
        _ => Some(slot),
    }
}

/// Authored buttons per layout.
const BUTTONS: usize = 12;

impl Picker {
    /// Lays out and draws the picker for `state`; `fingerprint` changes whenever `state` does.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        content: &super::content::Content,
        state: &State<'_>,
        fingerprint: u64,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
    ) -> io::Result<PickerDrawn> {
        let viewport = extent.map(|value| value as i32);
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        if self
            .built
            .as_ref()
            .is_none_or(|(size, built)| *size != viewport || !built.same(&windows))
        {
            *self = Self {
                built: Some((viewport, (*windows).clone())),
                ..Self::default()
            };
        }
        if self.last != Some(fingerprint) {
            self.last = Some(fingerprint);
            self.generation += 1;
        }
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
        };
        let root = gui::template(&windows, PICKER_WINDOW).map_err(super::invalid)?;
        let values = picker_values(state, root);
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
            &values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            generation,
            advance,
            hovers,
            true,
            &mut out.drawn.refused,
        ) else {
            return Ok(out);
        };
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
        let named: BTreeMap<_, _> = gui::elements(root)
            .into_iter()
            .map(|(path, element)| (element.path.clone(), path))
            .collect();
        for region in &prepared.layout.regions {
            let Some(rect) = gui::region_rect(region, origin) else {
                continue;
            };
            let Some(path) = named.get(&region.path) else {
                continue;
            };
            let leaf = path.rsplit('/').next().unwrap_or(path);
            if let Some(control) = picker_control(leaf, state) {
                out.controls.push((rect, control));
            }
        }
        if hovers {
            gui::append_hovers(&mut out.drawn.tooltips, prepared.hovers, origin);
        }
        // The lists draw above the picker.
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
                    &mut out,
                )?;
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
                    &mut out,
                )?;
            }
        }
        Ok(out)
    }
}

fn picker_control(leaf: &str, state: &State<'_>) -> Option<Control> {
    if leaf == "mapmodes_all" {
        return Some(Control::All);
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

/// A control's first authored shortcut.
fn authored_shortcut(root: &installed_content_core::gui::native::Element, path: &str) -> Option<String> {
    let shortcut = root.path_named(path)?.button()?.shortcuts.first()?;
    std::str::from_utf8(shortcut).ok().map(str::to_owned)
}

/// Every picker control's first authored shortcut, slots by their slot: what the picker's
/// keys select.
pub fn shortcuts(
    content: &super::content::Content,
    extent: [f32; 2],
    four_buttons: bool,
    slots: usize,
) -> io::Result<Vec<(Control, String)>> {
    let windows = content.window_cache.at(extent.map(|value| value as u32))?;
    let root = gui::template(&windows, PICKER_WINDOW).map_err(super::invalid)?;
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
    Ok(out)
}

/// Binds every slot button pair, the layout choice and the toggles.
fn picker_values(state: &State<'_>, root: &installed_content_core::gui::native::Element) -> Values {
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
            &state.click_info_assign
        } else {
            &state.click_info
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
                tooltip_anchor: TooltipAnchor::Pointer,
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
                // A toggle shows its second frame while on.
                frame: Some(u32::from(state == Some(true))),
                ..Default::default()
            },
        );
    }
    values.insert(
        "mapmodes_all".to_owned(),
        Widget {
            tooltip: hover(&state.all_tooltip),
            hover_owned: true,
            ..Default::default()
        },
    );
    // The find view has no replay counterpart.
    values.insert(
        "open_find_view_button".to_owned(),
        Widget {
            enabled: Some(false),
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

#[allow(clippy::too_many_arguments)]
fn draw_list(
    kept: &mut Option<Retained>,
    name: &str,
    state: &State<'_>,
    assigning: Option<usize>,
    windows: &viewer_foundation::map::native_windows::NativeWindows,
    resources: &gui_resources::GuiResources<'_>,
    viewport: [i32; 2],
    tick: lifecycle::Tick,
    generation: u64,
    advance: bool,
    hovers: bool,
    out: &mut PickerDrawn,
) -> io::Result<()> {
    // The list grows a row for each scripted mode, upward, as the game grows the window.
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
    // An entry names the shortcut of the slot that holds its mode.
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
        out,
        |key| Some(Control::Entry(key.to_owned())),
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_slots(
    kept: &mut Option<Retained>,
    state: &State<'_>,
    assigning: usize,
    windows: &viewer_foundation::map::native_windows::NativeWindows,
    resources: &gui_resources::GuiResources<'_>,
    viewport: [i32; 2],
    tick: lifecycle::Tick,
    generation: u64,
    advance: bool,
    hovers: bool,
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
            // The slot being reassigned shows pressed.
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
            tooltip: hover(&state.default_tooltip),
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
        out,
        |key| key.parse().ok().map(Control::SlotEntry),
    )
}

/// A mode list window grown by `rows` entries: its `slots` grid takes no scrollbar and grows
/// downward past its authored height, so the window gains `rows` slot heights and moves up by
/// the same, its bottom edge staying where it was.
fn grown(
    root: &installed_content_core::gui::native::Element,
    rows: usize,
) -> Result<installed_content_core::gui::native::Element, String> {
    use installed_content_core::gui::native::Fields;
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

#[allow(clippy::too_many_arguments)]
fn draw_window(
    retained: &mut Retained,
    root: &installed_content_core::gui::native::Element,
    values: &Values,
    groups: Vec<Group<'_>>,
    windows: &viewer_foundation::map::native_windows::NativeWindows,
    resources: &gui_resources::GuiResources<'_>,
    viewport: [i32; 2],
    tick: lifecycle::Tick,
    generation: u64,
    advance: bool,
    hovers: bool,
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
        &mut out.drawn.refused,
    ) else {
        return Ok(());
    };
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
    out.controls.push((
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
            out.controls.push((rect, Control::Default));
        }
    }
    for (_, key, region) in &prepared.entries {
        if let Some(rect) = gui::region_rect(region, origin)
            && let Some(control) = entry(key)
        {
            out.controls.push((rect, control));
        }
    }
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
        // Without the fourth large button, its slot has no button.
        assert_eq!(button_of_slot(3, false), None);
        assert_eq!(button_of_slot(4, false), Some(4));
        assert_eq!(button_of_slot(10, true), Some(10));
    }

    #[test]
    fn the_topmost_control_under_the_pointer_takes_the_click() {
        let drawn = PickerDrawn {
            drawn: Drawn::default(),
            // Paint order: the window, then a button over it.
            controls: vec![
                ([0.0, 0.0, 100.0, 50.0], Control::Window),
                ([10.0, 10.0, 20.0, 20.0], Control::Slot(1)),
            ],
        };
        assert_eq!(drawn.control_at([15.0, 15.0]), Some(&Control::Slot(1)));
        assert_eq!(drawn.control_at([60.0, 15.0]), Some(&Control::Window));
        // A rectangle's far edges are outside it.
        assert_eq!(drawn.control_at([30.0, 15.0]), Some(&Control::Window));
        assert_eq!(drawn.control_at([100.0, 15.0]), None);
    }

    #[test]
    fn a_mode_list_grows_upward_a_slot_per_scripted_mode() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let windows = viewer_foundation::map::test_support::gui::windows(
            br#"guiTypes = { containerWindowType = {
                name = "List" position = { x = -255 y = -322 } size = { width = 180 height = 296 }
                Orientation = lower_RIGHT
                gridboxtype = { name = "slots" position = { x = 0 y = 5 }
                    size = { width = 160 height = 264 } slotsize = { width = 156 height = 22 }
                    max_slots_horizontal = 1 }
            } }"#,
        );
        let root = gui::template(&windows, "List").unwrap();
        let viewport = Some([1920, 1080]);
        let authored = native_gui::container_frame(root, viewport).unwrap();
        assert_eq!(grown(root, 0).unwrap(), *root);
        let grown = native_gui::container_frame(&grown(root, 3).unwrap(), viewport).unwrap();
        // Three 22-pixel rows more, the bottom edge where it was.
        assert_eq!(grown.size[1], authored.size[1] + 66);
        assert_eq!(grown.at[1], authored.at[1] - 66);
        assert_eq!(
            grown.at[1] + grown.size[1],
            authored.at[1] + authored.size[1]
        );
        assert_eq!(grown.at[0], authored.at[0]);
    }

    #[test]
    fn buttons_show_their_modes_frame_whatever_the_slot() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let windows = viewer_foundation::map::native_windows::NativeWindows::read(
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
        // The game's default slots put navy (mode 2) before air (mode 1).
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
            click_info: String::new(),
            click_info_assign: String::new(),
            all_tooltip: String::new(),
            default_tooltip: String::new(),
        };
        let values = picker_values(&state, root);
        let frame = |button, selected| {
            values[&format!("{}/{}", layout_name(false), button_name(button, selected))].frame
        };
        assert_eq!(frame(0, true), Some(0));
        assert_eq!(frame(1, false), Some(2));
        assert_eq!(frame(2, false), Some(1));
        // Without a fourth large button the fourth slot is unused.
        assert!(values[&format!("{}/{}", layout_name(false), button_name(3, false))].hidden);
        // A mode no recording can draw shows disabled, its tooltip giving the reason.
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
