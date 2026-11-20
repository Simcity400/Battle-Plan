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
    Drawn, HoverText, Values, Widget,
    binding::Bindings,
    gui::{self, Group, Retained, Row},
    gui_resources,
};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::map::native_gui::{self, lifecycle, pointer::Input};

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
    pub click_info: &'a str,
    pub click_info_assign: &'a str,
    pub all_tooltip: &'a str,
    pub default_tooltip: &'a str,
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
    /// `open_find_view_button`: opens or closes the find view.
    FindView,
    /// A control of the minimap window.
    Minimap(super::minimap::Control),
    /// A control of the find view.
    Find(super::find_view::Control),
    /// The window background: absorbs clicks without an action.
    Window,
}

/// A drawn control: its rectangle, and the alpha test its sprite asks for within it.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub rect: [f32; 4],
    pub control: Control,
    /// `transparencecheck`'s hit and the window origin its points are taken from.
    alpha: Option<std::sync::Arc<(native_gui::HitGeometry, [f32; 2])>>,
    /// Its button's path and click sound, which a right-click plays too.
    pub click: Option<(String, Vec<u8>)>,
    /// Disabled: it takes the pointer but acts on no click.
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

    /// `region` of the window at `origin`, testing its sprite's alpha where it asks to; `None`
    /// when its sprite passes the pointer through.
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

    /// Whether the control takes the pointer at `at`, as its native hit test does.
    pub fn contains(&self, at: [f32; 2]) -> bool {
        viewer_foundation::map::gui_resources::contains(self.rect, at)
            && self.alpha.as_ref().is_none_or(|alpha| {
                let (hit, origin) = alpha.as_ref();
                hit.contains([at[0] - origin[0], at[1] - origin[1]])
            })
    }
}

/// One frame of the picker: its quads and tooltips, and its controls in paint order.
#[derive(Clone)]
pub struct PickerDrawn {
    pub drawn: Drawn,
    pub controls: Vec<Placed>,
    /// The controls clicked this frame, with each click's order among the frame's button changes.
    pub clicked: Vec<(Control, usize)>,
}

impl PickerDrawn {
    /// This picture to draw again, its sounds and clicks spent ([`Drawn::again`]).
    fn again(&self) -> Self {
        Self {
            drawn: self.drawn.again(),
            controls: self.controls.clone(),
            clicked: Vec::new(),
        }
    }

    /// The topmost control under `point`.
    pub fn control_at(&self, point: [f32; 2]) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|placed| placed.contains(point))
            .map(|placed| &placed.control)
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
    root_generation: Option<u64>,
    values: Values,
    controls: BTreeMap<String, Control>,
    /// The open lists' window rectangles as last drawn.
    above: Vec<[f32; 4]>,
    /// Each window's controls as last drawn, bottom first.
    owners: Vec<(Part, Placed)>,
    /// What the last draw drew and what it drew it for: a frame asking for the same picture of a
    /// picker in which nothing animates takes this copy instead of laying it out again.
    held: Option<(Shown, PickerDrawn)>,
}

/// What a drawn picker was drawn for; the same again is the same picture.
#[derive(Clone, Copy, PartialEq)]
struct Shown {
    fingerprint: u64,
    viewport: [i32; 2],
    hovers: bool,
    advance: bool,
    /// Where the pointer stood, which decides what draws hovered.
    place: Option<native_gui::pointer::Place>,
}

/// One of the picker's windows, which share the primary button.
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
    /// Whether the primary button holds one of the picker's buttons.
    pub fn holding(&self) -> bool {
        [&self.list, &self.slots, &self.root]
            .into_iter()
            .flatten()
            .any(Retained::holding)
    }

    /// The control the last draw drew for the pointer: an open list's, drawn above, first.
    pub fn highlighted(&self) -> Option<&str> {
        [&self.list, &self.slots, &self.root]
            .into_iter()
            .flatten()
            .find_map(Retained::highlighted)
    }

    /// Whether a button the pointer holds still changes on the clock.
    pub fn pointing(&self) -> bool {
        [&self.root, &self.list, &self.slots]
            .into_iter()
            .flatten()
            .any(Retained::pointing)
    }

    /// Whether a window of the picker draws differently next frame without a call from the code.
    fn animating(&self) -> bool {
        [&self.root, &self.list, &self.slots]
            .into_iter()
            .flatten()
            .any(Retained::active)
    }

    fn refresh_values(&mut self, state: &State<'_>, root: &installed_content_core::gui::native::Element) {
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
                built: Some((viewport, (*windows).clone())),
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
        // A picker whose values did not change, in which nothing animates and whose pointer stood
        // where it stood with its button still, draws the picture it drew last frame.
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
        // A press one window holds, or lands on, keeps the pointer from the others until released.
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
        // The lists draw above the picker: the pointer over one last frame is not the root's.
        let above = std::mem::take(&mut self.above);
        let root_input = shared(Part::Root).map(|input| {
            input.filter(|at| {
                !above
                    .iter()
                    .any(|&rect| viewer_foundation::map::gui_resources::contains(rect, at))
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
    Ok(shortcuts_in(root, four_buttons, slots))
}

/// [`shortcuts`] of the picker's template.
fn shortcuts_in(
    root: &installed_content_core::gui::native::Element,
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
                // A toggle shows its second frame while on.
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
    input: Option<Input>,
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
                Placed::new([0.0, 0.0, 100.0, 50.0], Control::Window),
                Placed::new([10.0, 10.0, 20.0, 20.0], Control::Slot(1)),
            ],
            clicked: Vec::new(),
        };
        assert_eq!(drawn.control_at([15.0, 15.0]), Some(&Control::Slot(1)));
        assert_eq!(drawn.control_at([60.0, 15.0]), Some(&Control::Window));
        // A rectangle's far edges are outside it.
        assert_eq!(drawn.control_at([30.0, 15.0]), Some(&Control::Window));
        assert_eq!(drawn.control_at([100.0, 15.0]), None);
    }

    #[test]
    fn the_find_view_button_opens_the_find_view_by_click_and_shortcut() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let windows = viewer_foundation::map::native_windows::NativeWindows::read(
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
        // The button keeps its authored enabled state.
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
            click_info: "",
            click_info_assign: "",
            all_tooltip: "",
            default_tooltip: "",
        };
        let mut picker = Picker::default();
        picker.refresh_values(&state, root);
        let cached = &picker.values["day_night_toggle"] as *const Widget;
        picker.refresh_values(&state, root);
        assert_eq!(&picker.values["day_night_toggle"] as *const Widget, cached);
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
