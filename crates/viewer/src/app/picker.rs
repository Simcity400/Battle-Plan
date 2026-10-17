//! The map mode picker over a map view: which mode each slot holds, the clicks and keys that
//! choose and reassign them, and what the host is told.
//!
//! The view's map mode, slots and fog arrive in the host's map options and leave again when the
//! picker changes them ([`super::Viewer::take_view_events`]); until the host echoes a change,
//! the view keeps its own choice, so a layout message sent before the click cannot undo it.

use super::Viewer;
use crate::document::Document;
use eframe::egui;
use viewer_combat::picker::{self, Control, ModeButton, Open, Toggles};
use viewer_world::map::map_mode::{MapMode, Stock};

/// The picker's slots as the game ships them: the political, naval, air and operative large
/// buttons, then supply, terrain, resistance, compliance, resources, raids and factions.
pub const DEFAULT_SLOTS: [Stock; 11] = [
    Stock::Default,
    Stock::StrategicNavy,
    Stock::StrategicAir,
    Stock::Operatives,
    Stock::Supply,
    Stock::Terrain,
    Stock::Resistance,
    Stock::Compliance,
    Stock::Resources,
    Stock::Raids,
    Stock::Factions,
];

/// The slots the user can reassign: the content's `UI_CONFIGURABLE_SLOT_FROM` to `_TO`, within
/// the picker's slots.
pub fn configurable(document: &Document) -> std::ops::RangeInclusive<usize> {
    let [from, to] = document.map_modes.defines.configurable_slots;
    let last = DEFAULT_SLOTS.len() - 1;
    (from.max(0) as usize).min(last + 1)..=(to.max(0) as usize).min(last)
}

/// The runtime DLC bit of the feature that adds the fourth large button.
const FOURTH_BUTTON_DLC: u32 = 0x20;

/// What the picker sends the host.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewEvent {
    Options(dashboard::protocol::MapOptions),
    Toggle(dashboard::protocol::MapToggle),
}

/// The picker's state in one view.
#[derive(Default)]
pub struct PickerState {
    pub picker: picker::Picker,
    pub open: Open,
    /// Tokens of the configurable slots; empty holds the defaults.
    pub slots: Vec<String>,
    /// The options this view last chose, kept until the host's options match them.
    pub requested: Option<dashboard::protocol::MapOptions>,
    pub events: Vec<ViewEvent>,
    /// Hidden in the map-only pop-out; its shortcuts still work.
    pub hidden: bool,
}

fn localised(document: &Document, key: &str) -> String {
    document
        .combat
        .localisation
        .localise(key.as_bytes())
        .unwrap_or_else(|_| key.to_owned())
}

/// A mode's button and list entry.
pub fn mode_button(document: &Document, mode: &MapMode) -> ModeButton {
    match mode {
        MapMode::Stock(stock) => ModeButton {
            token: mode.token(),
            frame: Some(*stock as u32),
            sprite_name: None,
            unavailable: stock.unavailable().map(str::to_owned),
            name: localised(document, &format!("{}_NAME", stock.key())),
            tooltip: localised(document, stock.key()),
            description: localised(document, &format!("{}_DESCRIPTION", stock.key())),
        },
        MapMode::Scripted(name) => {
            let key = format!("MAPMODE_{}", name.to_ascii_uppercase());
            ModeButton {
                token: mode.token(),
                frame: None,
                sprite_name: Some(name.clone()),
                unavailable: None,
                name: localised(document, &format!("{key}_NAME")),
                tooltip: localised(document, &format!("{name}_tooltip")),
                description: localised(document, &format!("{name}_tooltip_delayed")),
            }
        }
    }
}

/// Every mode the lists offer: the picker's own from the content's first configurable mode
/// (`NMapMode.UI_CONFIGURABLE_SLOT_FROM`, which numbers the modes as it numbers the slots),
/// then the content's scripted ones.
pub fn all_modes(document: &Document, four_buttons: bool) -> Vec<MapMode> {
    Stock::ALL
        .into_iter()
        .skip(*configurable(document).start())
        // The operatives mode comes with the fourth button's feature.
        .filter(|stock| four_buttons || *stock != Stock::Operatives)
        .map(MapMode::Stock)
        .chain(
            document
                .map_modes
                .scripted
                .modes
                .iter()
                .map(|mode| MapMode::Scripted(mode.name.clone())),
        )
        .collect()
}

/// The modes the slots hold: the defaults with the view's configured slots over them.
pub fn slot_modes(document: &Document, configured: &[String]) -> Vec<MapMode> {
    DEFAULT_SLOTS
        .iter()
        .enumerate()
        .map(|(slot, stock)| {
            configurable(document)
                .contains(&slot)
                .then(|| slot - configurable(document).start())
                .and_then(|index| configured.get(index))
                .and_then(|token| MapMode::parse(token))
                .filter(|mode| document.map_modes.knows(mode))
                .unwrap_or(MapMode::Stock(*stock))
        })
        .collect()
}

/// What the province hover shows in `mode`: the location block only in the default and states
/// modes, each other mode's own lines otherwise.
pub fn mode_hover(
    mode: &MapMode,
    opinion_thresholds: [i32; 2],
    infrastructure_level: impl FnOnce() -> Result<i32, String>,
) -> viewer_counter_products::map::province_tooltips::ModeHover {
    use viewer_counter_products::map::province_tooltips::ModeHover;
    match mode {
        MapMode::Stock(stock) => match stock {
            Stock::Default => ModeHover::Location { category: false },
            Stock::States => ModeHover::Location { category: true },
            Stock::Manpower => ModeHover::Manpower,
            Stock::Diplomacy => {
                let [very_good, very_bad] = opinion_thresholds;
                ModeHover::Diplomacy {
                    very_good,
                    very_bad,
                }
            }
            Stock::Ideology => ModeHover::Ideology,
            Stock::Resistance | Stock::Compliance => ModeHover::Occupation,
            Stock::Factions
            | Stock::Players
            | Stock::StrategicAir
            | Stock::StrategicNavy
            | Stock::Raids => ModeHover::Nothing,
            // The app asks the terrain mode's own hover (viewer-combat's terrain_mode) instead.
            Stock::Terrain => ModeHover::Nothing,
            Stock::Resources => ModeHover::Resources,
            Stock::Operatives => ModeHover::Operatives,
            Stock::Infrastructure => ModeHover::Infrastructure(infrastructure_level()),
            Stock::Supply => ModeHover::Unbuilt(stock.unavailable().unwrap_or_default().to_owned()),
        },
        MapMode::Scripted(name) => ModeHover::Scripted(name.clone()),
    }
}

/// Whether the recording owns the feature of the fourth large button.
pub fn four_buttons(document: &Document) -> bool {
    document
        .frame()
        .ok()
        .and_then(|frame| {
            frame
                .complete::<replay::rows::tables::SimulationFeatures>()
                .ok()
                .and_then(|rows| rows.first().map(|row| row.dlc_mask))
        })
        .is_none_or(|mask| mask & FOURTH_BUTTON_DLC != 0)
}

/// An authored shortcut as the window's keys: modifiers and one key.
pub fn parse_shortcut(text: &str) -> Option<(egui::Modifiers, egui::Key)> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "," => key = Some(egui::Key::Comma),
            "." => key = Some(egui::Key::Period),
            other => {
                key = egui::Key::from_name(&other.to_ascii_uppercase())
                    .or_else(|| egui::Key::from_name(other))
            }
        }
    }
    key.map(|key| (modifiers, key))
}

/// Whether the host's options carry back the picker's choice: its map mode and slots. Fog and
/// the country are the host's own and never hold a choice back.
pub fn echoes(
    requested: &dashboard::protocol::MapOptions,
    options: &dashboard::protocol::MapOptions,
) -> bool {
    requested.map_mode == options.map_mode && requested.map_mode_slots == options.map_mode_slots
}

/// The map mode and slots the host's options give. A mode no recording can draw is never shown,
/// whoever names it.
pub fn host_choice(options: &dashboard::protocol::MapOptions) -> (MapMode, Vec<String>) {
    let mode = options
        .map_mode
        .as_deref()
        .and_then(MapMode::parse)
        .filter(|mode| !matches!(mode, MapMode::Stock(stock) if stock.unavailable().is_some()))
        .unwrap_or_default();
    (mode, options.map_mode_slots.clone())
}

/// The setting a picker toggle asks the host to flip, given each toggle's current state (`None`
/// while disabled) and whether a host owns fog.
pub fn toggle_request(
    name: &str,
    toggles: Toggles,
    hosted: bool,
) -> Option<dashboard::protocol::MapSetting> {
    use dashboard::protocol::MapSetting;
    Some(match name {
        "button_fog" if hosted => MapSetting::Fog,
        "day_night_toggle" => {
            toggles.day_night?;
            MapSetting::DayNight
        }
        "button_allied_plans" => {
            toggles.allied_plans?;
            MapSetting::AlliedPlans
        }
        _ => return None,
    })
}

/// What a picker click asks of the view.
#[derive(Clone, Debug, PartialEq)]
pub enum Click {
    Nothing,
    /// Choose this mode.
    Choose(MapMode),
    /// The configurable slots' new tokens; empty restores the defaults.
    Configure(Vec<String>),
    Toggle(&'static str),
}

/// A click on `control` (`secondary` is the right button), given the modes the slots hold and
/// the slots the user can reassign: the open list it leaves and what the view must do.
pub fn click(
    open: &mut Open,
    slots: &[MapMode],
    range: std::ops::RangeInclusive<usize>,
    control: &Control,
    secondary: bool,
) -> Click {
    match control {
        // Right-clicking a reassignable slot opens the list to reassign it.
        Control::Slot(slot) if secondary => {
            if range.contains(slot) {
                *open = Open::Assign(*slot);
            }
            Click::Nothing
        }
        Control::Slot(slot) => slots
            .get(*slot)
            .cloned()
            .map_or(Click::Nothing, Click::Choose),
        Control::All => {
            *open = if *open == Open::All {
                Open::None
            } else {
                Open::All
            };
            Click::Nothing
        }
        Control::Entry(token) => {
            let Some(mode) = MapMode::parse(token) else {
                return Click::Nothing;
            };
            match *open {
                Open::Assign(slot) => {
                    *open = Open::None;
                    let Some(index) = slot.checked_sub(*range.start()) else {
                        return Click::Nothing;
                    };
                    let mut configured: Vec<String> = slots
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| range.contains(index))
                        .map(|(_, mode)| mode.token())
                        .collect();
                    match configured.get_mut(index) {
                        Some(held) => *held = mode.token(),
                        None => return Click::Nothing,
                    }
                    Click::Configure(configured)
                }
                _ => {
                    if secondary {
                        *open = Open::None;
                    }
                    Click::Choose(mode)
                }
            }
        }
        Control::SlotEntry(slot) => {
            if range.contains(slot) {
                *open = Open::Assign(*slot);
            }
            Click::Nothing
        }
        Control::Default => {
            *open = Open::None;
            Click::Configure(Vec::new())
        }
        Control::Toggle(name) => Click::Toggle(name),
        Control::Window => Click::Nothing,
    }
}

impl Viewer {
    /// The picker's slots as configured, padded to the defaults' length.
    fn configured_slots(&self) -> Vec<String> {
        self.picker.slots.clone()
    }

    /// Chooses `mode` and tells the host.
    pub(crate) fn choose_map_mode(&mut self, mode: MapMode) {
        // A mode no recording can draw is offered disabled, and choosing it does nothing.
        if matches!(&mode, MapMode::Stock(stock) if stock.unavailable().is_some()) {
            return;
        }
        if self.map_mode != mode {
            self.map_mode = mode;
            self.report_options();
        }
    }

    /// Records the view's options as chosen here and queues them for the host.
    fn report_options(&mut self) {
        let options = dashboard::protocol::MapOptions {
            country: match &self.perspective.country {
                viewer_foundation::map::perspective::Country::Tag(tag) => Some(tag.clone()),
                _ => None,
            },
            fog: self.perspective.fog,
            map_mode: (self.map_mode != MapMode::default()).then(|| self.map_mode.token()),
            map_mode_slots: self.picker.slots.clone(),
            picker_hidden: self.picker.hidden,
        };
        self.picker.requested = Some(options.clone());
        self.picker.events.push(ViewEvent::Options(options));
    }

    /// The host's options for this view: its map mode and slots unless a choice made here is
    /// still on its way. The picker owns only these; fog and the country are the host's.
    pub(crate) fn apply_host_options(&mut self, options: &dashboard::protocol::MapOptions) {
        self.picker.hidden = options.picker_hidden;
        let echoed = self
            .picker
            .requested
            .as_ref()
            .is_some_and(|requested| echoes(requested, options));
        if echoed {
            self.picker.requested = None;
        }
        if self.picker.requested.is_none() {
            (self.map_mode, self.picker.slots) = host_choice(options);
        }
    }

    /// What the picker told the host since the last call.
    pub(crate) fn take_view_events(&mut self) -> Vec<ViewEvent> {
        std::mem::take(&mut self.picker.events)
    }

    /// The toggles' states, each the value its owner last applied: the host's for the settings it
    /// owns, which the picker never anticipates.
    pub(crate) fn picker_toggles(&self) -> Toggles {
        let host = self.dashboard_map;
        Toggles {
            day_night: host.then_some(self.graphics.day_night),
            player_counters: Some(self.player_counters),
            counter_colors: Some(
                self.counter_view.settings.color_mode
                    == viewer_counters::map::counters::CounterColorMode::Country,
            ),
            allied_plans: host.then_some(self.graphics.show_allied_plans),
            fog: Some(self.perspective.fog),
            // The radar overlay is not drawn by the viewer.
            radar: None,
        }
    }

    /// The countries whose counters draw: the widget's filter, and with the player-only toggle
    /// on, only the counters on the viewing country's roster (`ours`, its canonical slot), as the
    /// game keeps the local player's own. A counter's owner is its roster owner's slot.
    pub(crate) fn shown_counter_owners(
        &self,
        filtered: Option<std::sync::Arc<std::collections::BTreeSet<u32>>>,
        ours: u32,
    ) -> Option<std::sync::Arc<std::collections::BTreeSet<u32>>> {
        if !self.player_counters {
            return filtered;
        }
        let kept = filtered.is_none_or(|owners| owners.contains(&ours));
        Some(std::sync::Arc::new(
            kept.then_some(ours).into_iter().collect(),
        ))
    }

    /// Lays out the picker; `None` while hidden or without content.
    pub(crate) fn draw_picker(
        &mut self,
        document: &Document,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
    ) -> Option<Result<picker::PickerDrawn, String>> {
        if self.picker.hidden {
            return None;
        }
        let four = four_buttons(document);
        let slots: Vec<ModeButton> = slot_modes(document, &self.configured_slots())
            .iter()
            .map(|mode| mode_button(document, mode))
            .collect();
        let modes: Vec<ModeButton> = all_modes(document, four)
            .iter()
            .map(|mode| mode_button(document, mode))
            .collect();
        let selected = self.map_mode.token();
        let toggles = self.picker_toggles();
        let state = picker::State {
            four_buttons: four,
            slots: &slots,
            configurable_from: *configurable(document).start(),
            modes: &modes,
            selected: &selected,
            toggles,
            open: self.picker.open,
            click_info: localised(document, "MAP_MODE_CLICK_INFO"),
            click_info_assign: localised(document, "MAP_MODE_CLICK_INFO_WITH_ASSIGN"),
            all_tooltip: localised(document, "MAPMODE_ALL"),
            default_tooltip: localised(document, "CONFIG_MAPMODE_DEFAULT"),
        };
        let fingerprint = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            four.hash(&mut hasher);
            selected.hash(&mut hasher);
            self.picker.slots.hash(&mut hasher);
            format!("{:?}{toggles:?}", self.picker.open).hash(&mut hasher);
            hasher.finish()
        };
        Some(
            self.picker
                .picker
                .draw(
                    &document.combat,
                    &state,
                    fingerprint,
                    extent,
                    hovers,
                    time,
                    advance,
                )
                .map_err(|error| error.to_string()),
        )
    }

    /// A click on a picker control; `secondary` is the right button.
    pub(crate) fn picker_click(&mut self, document: &Document, control: &Control, secondary: bool) {
        let slots = slot_modes(document, &self.configured_slots());
        match click(
            &mut self.picker.open,
            &slots,
            configurable(document),
            control,
            secondary,
        ) {
            Click::Nothing => {}
            Click::Choose(mode) => self.choose_map_mode(mode),
            Click::Configure(configured) => {
                self.picker.slots = configured;
                self.report_options();
            }
            Click::Toggle(name) => self.picker_toggle(name),
        }
    }

    pub(super) fn picker_toggle(&mut self, name: &str) {
        // The host owns fog (the widget's or the campaign map's switch), day/night and allied
        // plans: the view asks it to flip the setting, and it flips its own value in order, so
        // quick clicks each count and the picker shows the host's value as it last applied it.
        // Without a host the view keeps fog.
        let hosted = self.embedded.is_some() || self.dashboard_map;
        if let Some(setting) = toggle_request(name, self.picker_toggles(), hosted) {
            self.picker
                .events
                .push(ViewEvent::Toggle(dashboard::protocol::MapToggle {
                    setting,
                }));
            return;
        }
        match name {
            "button_fog" if !hosted => self.perspective.fog = !self.perspective.fog,
            "player_counters_toggle" => {
                self.player_counters = !self.player_counters;
                // The counters' owners are chosen with the overlay's layout.
                self.overlay_at = None;
            }
            "button_counters_color_mode" => {
                use viewer_counters::map::counters::CounterColorMode;
                self.counter_view.settings.color_mode =
                    if self.counter_view.settings.color_mode == CounterColorMode::Country {
                        CounterColorMode::Allegiance
                    } else {
                        CounterColorMode::Country
                    };
            }
            _ => {}
        }
    }

    /// The picker's authored keys pressed this frame, as clicks.
    pub(crate) fn picker_keys(&mut self, ui: &egui::Ui, document: &Document, extent: [f32; 2]) {
        let four = four_buttons(document);
        let Ok(shortcuts) = picker::shortcuts(&document.combat, extent, four, DEFAULT_SLOTS.len())
        else {
            return;
        };
        let pressed: Vec<Control> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => shortcuts.iter().find_map(|(control, text)| {
                        let (wanted, wanted_key) = parse_shortcut(text)?;
                        (wanted_key == *key && modifiers.matches_exact(wanted))
                            .then(|| control.clone())
                    }),
                    _ => None,
                })
                .collect()
        });
        for control in pressed {
            self.picker_click(document, &control, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(fog: bool, mode: Option<&str>) -> dashboard::protocol::MapOptions {
        dashboard::protocol::MapOptions {
            country: None,
            fog,
            map_mode: mode.map(str::to_owned),
            map_mode_slots: Vec::new(),
            picker_hidden: false,
        }
    }

    #[test]
    fn a_fog_change_from_the_host_never_holds_back_the_pickers_choice() {
        // The picker chose air with fog on; the host carries air back with fog now off.
        assert!(echoes(
            &options(true, Some("strategic_air")),
            &options(false, Some("strategic_air"))
        ));
        assert!(!echoes(
            &options(true, Some("strategic_air")),
            &options(true, None)
        ));
    }

    #[test]
    fn the_host_restores_a_choice_but_never_an_unavailable_mode() {
        assert_eq!(
            host_choice(&options(true, Some("states"))).0,
            MapMode::Stock(Stock::States)
        );
        assert_eq!(
            host_choice(&options(true, Some("infrastructure"))).0,
            MapMode::Stock(Stock::Infrastructure)
        );
        assert_eq!(
            host_choice(&options(true, Some("supply"))).0,
            MapMode::default()
        );
        assert_eq!(host_choice(&options(true, None)).0, MapMode::default());
    }

    #[test]
    fn a_toggle_asks_the_host_to_flip_its_setting() {
        use dashboard::protocol::MapSetting;
        let toggles = Toggles {
            day_night: Some(true),
            allied_plans: Some(false),
            ..Toggles::default()
        };
        assert_eq!(
            toggle_request("day_night_toggle", toggles, true),
            Some(MapSetting::DayNight)
        );
        assert_eq!(
            toggle_request("button_allied_plans", toggles, true),
            Some(MapSetting::AlliedPlans)
        );
        assert_eq!(
            toggle_request("button_fog", toggles, true),
            Some(MapSetting::Fog)
        );
        // Without a host the view keeps fog; a disabled toggle asks for nothing.
        assert_eq!(toggle_request("button_fog", toggles, false), None);
        assert_eq!(
            toggle_request("day_night_toggle", Toggles::default(), true),
            None
        );
    }

    fn viewer() -> Viewer {
        Viewer::create(
            crate::Config::default(),
            crate::Options::default(),
            None,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
            None,
            None,
        )
    }

    fn toggles(viewer: &mut Viewer) -> Vec<dashboard::protocol::MapToggle> {
        viewer
            .take_view_events()
            .into_iter()
            .filter_map(|event| match event {
                ViewEvent::Toggle(toggle) => Some(toggle),
                ViewEvent::Options(_) => None,
            })
            .collect()
    }

    #[test]
    fn toggles_flip_on_the_host_and_show_what_it_last_applied() {
        use dashboard::protocol::{
            DashboardMapView, MapAction, MapOptions, MapSetting, MapToggle, ResolvedMapSettings,
            ViewBounds,
        };
        let mut viewer = viewer();
        viewer.dashboard_map = true;
        viewer.graphics.day_night = true;
        viewer.perspective.fog = true;
        let flip = |setting| MapToggle { setting };
        let fog = |viewer: &Viewer| viewer.picker_toggles().fog;
        let mut frame = eframe::Frame::_new_kittest();
        // The campaign map's host: each Configure action as the desktop applies it.
        let configure = |viewer: &mut Viewer, frame: &mut eframe::Frame, fog| {
            viewer.host_map_action(
                MapAction::Configure {
                    options: MapOptions {
                        fog,
                        ..Default::default()
                    },
                },
                &egui::Context::default(),
                frame,
            );
        };
        // A dashboard host's view snapshot, as one drawn frame of the widget map takes it.
        let dashboard = |viewer: &mut Viewer, frame: &eframe::Frame, fog| {
            let bounds = ViewBounds {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            };
            let target = DashboardMapView {
                options: MapOptions {
                    fog,
                    ..Default::default()
                },
                id: "widget".into(),
                source_id: "source".into(),
                path: "fixture".into(),
                revision: "revision".into(),
                branch_id: "1".into(),
                playing: false,
                hour: 0,
                fraction: 0.,
                sequence: 1,
                seek_sequence: None,
                bounds,
                clip: bounds,
                visible: true,
            };
            let mut output = egui::Context::default()
                .run_ui(Default::default(), |ui| viewer.map_ui(ui, frame, &target));
            output.textures_delta.clear();
        };

        // Each click asks for a flip; the picker keeps showing the host's value meanwhile.
        viewer.picker_toggle("button_fog");
        viewer.picker_toggle("button_fog");
        assert_eq!(fog(&viewer), Some(true));
        assert_eq!(
            toggles(&mut viewer),
            [flip(MapSetting::Fog), flip(MapSetting::Fog)]
        );
        // A coalesced dashboard snapshot after both flips (on, off, on) shows on.
        dashboard(&mut viewer, &frame, true);
        assert_eq!(fog(&viewer), Some(true));
        // An external change shows at once.
        dashboard(&mut viewer, &frame, false);
        assert_eq!(fog(&viewer), Some(false));
        // Replayed older Configure snapshots show each value the host sent, the last standing.
        configure(&mut viewer, &mut frame, true);
        configure(&mut viewer, &mut frame, false);
        assert_eq!(fog(&viewer), Some(false));
        viewer.picker_toggle("button_fog");
        assert_eq!(toggles(&mut viewer), [flip(MapSetting::Fog)]);

        // Day/night through the settings the host sends.
        viewer.picker_toggle("day_night_toggle");
        assert_eq!(viewer.picker_toggles().day_night, Some(true));
        let mut settings = ResolvedMapSettings::default();
        settings.graphics.day_night = false;
        viewer.set_embedded_graphics(&settings);
        assert_eq!(viewer.picker_toggles().day_night, Some(false));
        viewer.picker_toggle("day_night_toggle");
        assert_eq!(
            toggles(&mut viewer),
            [flip(MapSetting::DayNight), flip(MapSetting::DayNight)]
        );
        // The campaign map has no host for day/night: its toggle is disabled.
        viewer.dashboard_map = false;
        viewer.picker_toggle("day_night_toggle");
        assert!(toggles(&mut viewer).is_empty());
    }

    #[test]
    fn the_player_counters_toggle_keeps_the_viewing_countrys_own_counters() {
        let mut viewer = viewer();
        assert_eq!(viewer.picker_toggles().player_counters, Some(false));
        viewer.picker_toggle("player_counters_toggle");
        assert_eq!(viewer.picker_toggles().player_counters, Some(true));
        // The view's own toggle: nothing goes to the host.
        assert!(toggles(&mut viewer).is_empty());
        let filtered = |owners: Option<&[u32]>| {
            viewer
                .shown_counter_owners(
                    owners.map(|owners| std::sync::Arc::new(owners.iter().copied().collect())),
                    4,
                )
                .map(|owners| owners.iter().copied().collect::<Vec<_>>())
        };
        // Only the viewing country's roster counters draw, within the widget's own filter.
        assert_eq!(filtered(None), Some(vec![4]));
        assert_eq!(filtered(Some(&[2, 4])), Some(vec![4]));
        assert_eq!(filtered(Some(&[2])), Some(vec![]));
        viewer.picker_toggle("player_counters_toggle");
        assert_eq!(viewer.shown_counter_owners(None, 4), None);
    }

    #[test]
    fn a_restarted_campaign_map_shows_the_choice_its_host_sends_back() {
        let mut first = viewer();
        first.choose_map_mode(MapMode::Stock(Stock::Diplomacy));
        let Some(ViewEvent::Options(options)) = first.take_view_events().pop() else {
            panic!("the choice was not reported");
        };
        let mut restarted = viewer();
        let mut frame = eframe::Frame::_new_kittest();
        restarted.host_map_action(
            dashboard::protocol::MapAction::Configure { options },
            &egui::Context::default(),
            &mut frame,
        );
        assert_eq!(restarted.map_mode, MapMode::Stock(Stock::Diplomacy));
    }

    fn defaults() -> Vec<MapMode> {
        DEFAULT_SLOTS.iter().copied().map(MapMode::Stock).collect()
    }

    #[test]
    fn only_the_default_and_states_modes_hover_the_location_block() {
        use viewer_counter_products::map::province_tooltips::ModeHover;
        let hover = |stock| mode_hover(&MapMode::Stock(stock), [50, -50], || Ok(3));
        assert_eq!(
            hover(Stock::Default),
            ModeHover::Location { category: false }
        );
        assert_eq!(hover(Stock::States), ModeHover::Location { category: true });
        assert_eq!(hover(Stock::Manpower), ModeHover::Manpower);
        assert_eq!(
            hover(Stock::Infrastructure),
            ModeHover::Infrastructure(Ok(3))
        );
        assert_eq!(
            hover(Stock::Diplomacy),
            ModeHover::Diplomacy {
                very_good: 50,
                very_bad: -50
            }
        );
        assert_eq!(hover(Stock::Resistance), ModeHover::Occupation);
        assert_eq!(hover(Stock::Compliance), ModeHover::Occupation);
        assert_eq!(hover(Stock::Operatives), ModeHover::Operatives);
        assert_eq!(hover(Stock::Resources), ModeHover::Resources);
        for stock in [
            Stock::Factions,
            Stock::Players,
            Stock::StrategicAir,
            Stock::StrategicNavy,
            Stock::Raids,
        ] {
            assert_eq!(hover(stock), ModeHover::Nothing, "{stock:?}");
        }
        assert_eq!(
            mode_hover(&MapMode::Scripted("spheres".into()), [50, -50], || Ok(0)),
            ModeHover::Scripted("spheres".into())
        );
    }

    #[test]
    fn a_left_click_on_a_slot_chooses_its_mode() {
        let mut open = Open::None;
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &Control::Slot(2), false),
            Click::Choose(MapMode::Stock(Stock::StrategicAir))
        );
        assert_eq!(open, Open::None);
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &Control::Slot(11), false),
            Click::Nothing
        );
    }

    #[test]
    fn a_right_click_opens_reassignment_only_for_a_configurable_slot() {
        let mut open = Open::None;
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &Control::Slot(1), true),
            Click::Nothing
        );
        assert_eq!(open, Open::None);
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &Control::Slot(5), true),
            Click::Nothing
        );
        assert_eq!(open, Open::Assign(5));
    }

    #[test]
    fn an_entry_reassigns_the_open_slot_and_closes_the_list() {
        let mut open = Open::Assign(5);
        let entry = Control::Entry(MapMode::Stock(Stock::Manpower).token());
        let Click::Configure(configured) = click(&mut open, &defaults(), 4..=10, &entry, false)
        else {
            panic!("an entry in the reassignment list configures the slots");
        };
        assert_eq!(open, Open::None);
        // Slot 5 is the second configurable slot; the others keep their modes.
        let expected: Vec<String> = defaults()[4..=10]
            .iter()
            .enumerate()
            .map(|(index, mode)| {
                if index == 1 {
                    MapMode::Stock(Stock::Manpower).token()
                } else {
                    mode.token()
                }
            })
            .collect();
        assert_eq!(configured, expected);
        // A slot outside the range never indexes the configured list.
        let mut open = Open::Assign(2);
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &entry, false),
            Click::Nothing
        );
        assert_eq!(open, Open::None);
    }

    #[test]
    fn the_all_list_toggles_and_its_entries_choose() {
        let mut open = Open::None;
        click(&mut open, &defaults(), 4..=10, &Control::All, false);
        assert_eq!(open, Open::All);
        let entry = Control::Entry(MapMode::Stock(Stock::Ideology).token());
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &entry, false),
            Click::Choose(MapMode::Stock(Stock::Ideology))
        );
        assert_eq!(open, Open::All);
        // A right-click on an entry chooses it and closes the list.
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &entry, true),
            Click::Choose(MapMode::Stock(Stock::Ideology))
        );
        assert_eq!(open, Open::None);
        click(&mut open, &defaults(), 4..=10, &Control::All, false);
        click(&mut open, &defaults(), 4..=10, &Control::All, false);
        assert_eq!(open, Open::None);
    }

    #[test]
    fn the_default_button_restores_the_slots() {
        let mut open = Open::Assign(4);
        assert_eq!(
            click(&mut open, &defaults(), 4..=10, &Control::Default, false),
            Click::Configure(Vec::new())
        );
        assert_eq!(open, Open::None);
    }

    #[test]
    fn authored_shortcuts_parse_into_keys() {
        assert_eq!(
            parse_shortcut("F1"),
            Some((egui::Modifiers::NONE, egui::Key::F1))
        );
        assert_eq!(
            parse_shortcut("ctrl+F1"),
            Some((egui::Modifiers::CTRL, egui::Key::F1))
        );
        assert_eq!(
            parse_shortcut("n"),
            Some((egui::Modifiers::NONE, egui::Key::N))
        );
        assert_eq!(
            parse_shortcut(","),
            Some((egui::Modifiers::NONE, egui::Key::Comma))
        );
        assert_eq!(parse_shortcut(""), None);
    }

    #[test]
    fn the_shipped_slots_put_air_on_the_third_button() {
        // F3 selects the third slot, the air mode, as in the game.
        assert_eq!(DEFAULT_SLOTS[2], Stock::StrategicAir);
    }
}
