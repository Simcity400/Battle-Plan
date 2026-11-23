
use super::Viewer;
use crate::document::{Content, Document};
use eframe::egui;
use std::sync::{Arc, Weak};
use viewer_native_gui::native_gui::pointer::Input;
use viewer_combat::picker::{self, Control, ModeButton, Open, Toggles};
use viewer_map_modes::map_mode::{MapMode, Stock};

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

pub fn configurable(document: &Document) -> std::ops::RangeInclusive<usize> {
    let [from, to] = document.map_modes.defines.configurable_slots;
    let last = DEFAULT_SLOTS.len() - 1;
    (from.max(0) as usize).min(last + 1)..=(to.max(0) as usize).min(last)
}

const FOURTH_BUTTON_DLC: u32 = 0x20;

#[derive(Clone, Debug, PartialEq)]
pub enum ViewEvent {
    Options(dashboard_core::protocol::MapOptions),
    Toggle(dashboard_core::protocol::MapToggle),
    Modes(Vec<dashboard_core::protocol::MapModeChoice>),
    Observe(dashboard_core::protocol::ObservedCountry),
}

pub struct PickerState {
    pub radar: bool,
    pub picker: picker::Picker,
    pub open: Open,
    pub slots: Vec<String>,
    pub requested: Option<dashboard_core::protocol::MapOptions>,
    pub events: Vec<ViewEvent>,
    pub hidden: bool,
    pub country_pinned: bool,
    labels: Option<PickerLabels>,
    modes_for: Option<Weak<Content>>,
    pub minimap: super::minimap::MinimapState,
    pub find: super::find_view::FindState,
}

impl Default for PickerState {
    fn default() -> Self {
        Self {
            radar: dashboard_core::map_display::option_defaults::RADAR,
            picker: Default::default(),
            open: Default::default(),
            slots: Vec::new(),
            requested: None,
            events: Vec::new(),
            hidden: false,
            country_pinned: false,
            labels: None,
            modes_for: None,
            minimap: Default::default(),
            find: Default::default(),
        }
    }
}

impl PickerState {
    pub fn holding(&self) -> bool {
        self.picker.holding() || self.minimap.holding() || self.find.view.holding()
    }

    pub fn pointing(&self) -> bool {
        self.picker.pointing()
            || self.minimap.active()
            || self.find.view.pointing()
            || (self.find.open && self.find.field.blinking())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        document: &Document,
        mode: &MapMode,
        toggles: Toggles,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
        four: bool,
    ) -> Result<picker::PickerDrawn, String> {
        if !self
            .labels
            .as_ref()
            .is_some_and(|labels| labels.matches(document, &self.slots, four))
        {
            self.labels = Some(PickerLabels::new(document, &self.slots, four));
        }
        let selected = mode.token();
        let labels = self.labels.as_ref().unwrap();
        let state = picker::State {
            four_buttons: four,
            slots: &labels.slots,
            configurable_from: *configurable(document).start(),
            modes: &labels.modes,
            selected: &selected,
            toggles,
            open: self.open,
            click_info: &labels.click_info,
            click_info_assign: &labels.click_info_assign,
            all_tooltip: &labels.all_tooltip,
            default_tooltip: &labels.default_tooltip,
        };
        let fingerprint = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            four.hash(&mut hasher);
            selected.hash(&mut hasher);
            self.slots.hash(&mut hasher);
            self.open.hash(&mut hasher);
            toggles.hash(&mut hasher);
            hasher.finish()
        };
        self.picker
            .draw(
                &document.combat,
                &state,
                fingerprint,
                extent,
                hovers,
                time,
                advance,
                input,
            )
            .map_err(|error| error.to_string())
    }
}

fn dump_toggles(document: &Document, options: &crate::Options) -> Toggles {
    let settings = options.counter_settings(&document.game_options);
    Toggles {
        day_night: None,
        player_counters: Some(dashboard_core::map_display::option_defaults::PLAYER_COUNTERS),
        counter_colors: Some(
            settings.color_mode == viewer_counter_kit::CounterColorMode::Country,
        ),
        allied_plans: None,
        fog: viewer_foundation::perspective::fog_recorded(document.capture_mode())
            .then_some(options.perspective.fog),
        radar: Some(dashboard_core::map_display::option_defaults::RADAR),
    }
}

pub(crate) fn dump(
    document: &Document,
    options: &crate::Options,
) -> Result<picker::PickerDrawn, String> {
    PickerState::default().draw(
        document,
        &options.map_mode,
        dump_toggles(document, options),
        viewer_gpu::ui_scale::extent(options.size, options.ui_scale.unwrap_or(1.0)),
        false,
        options.presentation_time,
        true,
        None,
        four_buttons(document),
    )
}

struct PickerLabels {
    content: Weak<Content>,
    configured: Vec<String>,
    four_buttons: bool,
    slots: Vec<ModeButton>,
    modes: Vec<ModeButton>,
    click_info: String,
    click_info_assign: String,
    all_tooltip: String,
    default_tooltip: String,
}

impl PickerLabels {
    fn matches(&self, document: &Document, configured: &[String], four_buttons: bool) -> bool {
        std::ptr::eq(self.content.as_ptr(), Arc::as_ptr(&document.content))
            && self.configured == configured
            && self.four_buttons == four_buttons
    }

    fn new(document: &Document, configured: &[String], four_buttons: bool) -> Self {
        Self {
            content: Arc::downgrade(&document.content),
            configured: configured.to_vec(),
            four_buttons,
            slots: slot_modes(document, configured)
                .iter()
                .map(|mode| mode_button(document, mode))
                .collect(),
            modes: all_modes(document, four_buttons)
                .iter()
                .map(|mode| mode_button(document, mode))
                .collect(),
            click_info: localised(document, "MAP_MODE_CLICK_INFO"),
            click_info_assign: localised(document, "MAP_MODE_CLICK_INFO_WITH_ASSIGN"),
            all_tooltip: localised(document, "MAPMODE_ALL"),
            default_tooltip: localised(document, "CONFIG_MAPMODE_DEFAULT"),
        }
    }
}

fn localised(document: &Document, key: &str) -> String {
    document
        .combat
        .localisation
        .localise(key.as_bytes())
        .unwrap_or_else(|_| key.to_owned())
}

pub fn mode_button(document: &Document, mode: &MapMode) -> ModeButton {
    match mode {
        MapMode::Stock(stock) => ModeButton {
            token: mode.token(),
            frame: Some(*stock as u32),
            sprite_name: None,
            unavailable: stock.unavailable_in(document.capture_mode()),
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
                unavailable: mode.unavailable_in(document.capture_mode()),
                name: localised(document, &format!("{key}_NAME")),
                tooltip: localised(document, &format!("{name}_tooltip")),
                description: localised(document, &format!("{name}_tooltip_delayed")),
            }
        }
    }
}

pub fn all_modes(document: &Document, four_buttons: bool) -> Vec<MapMode> {
    Stock::ALL
        .into_iter()
        .skip(*configurable(document).start())
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

fn choosable_modes(document: &Document, four: bool) -> Vec<MapMode> {
    let mut modes: Vec<MapMode> = DEFAULT_SLOTS[..*configurable(document).start()]
        .iter()
        .filter(|stock| four || **stock != Stock::Operatives)
        .map(|stock| MapMode::Stock(*stock))
        .collect();
    for mode in all_modes(document, four) {
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes
}

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

pub fn mode_hover(
    mode: &MapMode,
    region: bool,
    opinion_thresholds: [i32; 2],
    infrastructure_level: impl FnOnce() -> Result<i32, String>,
) -> viewer_counter_products::map::province_tooltips::ModeHover {
    use viewer_counter_products::map::province_tooltips::ModeHover;
    let unprepared = || std::sync::Arc::new(Err("naval dominance sources not prepared".into()));
    if region && !matches!(mode, MapMode::Stock(Stock::StrategicAir | Stock::StrategicNavy)) {
        return match mode {
            MapMode::Stock(Stock::Default) => ModeHover::SeaRegion(unprepared()),
            MapMode::Stock(Stock::Terrain) => ModeHover::Terrain { region: true },
            _ => ModeHover::Region,
        };
    }
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
            Stock::Factions | Stock::Players | Stock::Raids => ModeHover::Nothing,
            Stock::Terrain => ModeHover::Terrain { region: false },
            Stock::StrategicAir => ModeHover::StrategicAir(Default::default()),
            Stock::StrategicNavy => ModeHover::StrategicNavy(unprepared()),
            Stock::Resources => ModeHover::Resources,
            Stock::Operatives => ModeHover::Operatives,
            Stock::Infrastructure => ModeHover::Infrastructure(infrastructure_level()),
            Stock::Supply => ModeHover::Unbuilt(stock.unavailable().unwrap_or_default().to_owned()),
        },
        MapMode::Scripted(name) => ModeHover::Scripted(name.clone()),
    }
}

pub fn picks_region(kind: Option<content_reader_core::map::ProvinceKind>, region: Option<u32>) -> bool {
    kind == Some(content_reader_core::map::ProvinceKind::Sea) && region.is_some()
}

pub fn four_buttons(document: &Document) -> bool {
    document.frame().map_or(true, four_buttons_at)
}

pub fn four_buttons_at(frame: &replay_frame::frame::DecodedFrame) -> bool {
    frame
        .complete::<replay_data::rows::tables::SimulationFeatures>()
        .ok()
        .and_then(|rows| rows.first().map(|row| row.dlc_mask))
        .is_none_or(|mask| mask & FOURTH_BUTTON_DLC != 0)
}

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
                let mut capitalised: String = other.get(..1).unwrap_or_default().to_ascii_uppercase();
                capitalised.push_str(other.get(1..).unwrap_or_default());
                key = egui::Key::from_name(&other.to_ascii_uppercase())
                    .or_else(|| egui::Key::from_name(other))
                    .or_else(|| egui::Key::from_name(&capitalised))
            }
        }
    }
    key.map(|key| (modifiers, key))
}

pub fn echoes(
    requested: &dashboard_core::protocol::MapOptions,
    options: &dashboard_core::protocol::MapOptions,
) -> bool {
    requested.map_mode == options.map_mode && requested.map_mode_slots == options.map_mode_slots
}

pub fn host_choice(options: &dashboard_core::protocol::MapOptions) -> (MapMode, Vec<String>) {
    let mode = options
        .map_mode
        .as_deref()
        .and_then(MapMode::parse)
        .filter(|mode| !matches!(mode, MapMode::Stock(stock) if stock.unavailable().is_some()))
        .unwrap_or_default();
    (mode, options.map_mode_slots.clone())
}

pub fn toggle_request(
    name: &str,
    toggles: Toggles,
    hosted: bool,
) -> Option<dashboard_core::protocol::MapSetting> {
    use dashboard_core::protocol::MapSetting;
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

#[derive(Clone, Debug, PartialEq)]
pub enum Click {
    Nothing,
    Choose(MapMode),
    Configure(Vec<String>),
    Toggle(&'static str),
}

pub fn click(
    open: &mut Open,
    slots: &[MapMode],
    range: std::ops::RangeInclusive<usize>,
    control: &Control,
    secondary: bool,
) -> Click {
    match control {
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
        Control::FindView | Control::Minimap(_) | Control::Find(_) | Control::Window => {
            Click::Nothing
        }
    }
}

impl Viewer {
    pub(crate) fn choose_map_mode(&mut self, mode: MapMode) {
        if matches!(&mode, MapMode::Stock(stock) if stock.unavailable().is_some())
            || self
                .document
                .as_ref()
                .is_some_and(|document| mode.unavailable_in(document.capture_mode()).is_some())
        {
            return;
        }
        if self.map_mode != mode {
            self.air_selection.clear_bases();
            self.map_mode = mode;
            self.report_options();
        }
    }

    fn report_options(&mut self) {
        use viewer_foundation::perspective::Country;
        let (country, country_slot) = match &self.perspective.country {
            Country::Recorded => (None, None),
            Country::Tag(tag) => (Some(tag.clone()), None),
            Country::Slot { slot, tag } => (Some(tag.clone()), Some(*slot)),
        };
        let options = dashboard_core::protocol::MapOptions {
            country,
            country_slot,
            country_pinned: self.picker.country_pinned,
            fog: self.perspective.fog,
            map_mode: (self.map_mode != MapMode::default()).then(|| self.map_mode.token()),
            map_mode_slots: self.picker.slots.clone(),
            picker_hidden: self.picker.hidden,
            observed: None,
        };
        self.picker.requested = Some(options.clone());
        self.picker.events.push(ViewEvent::Options(options));
    }

    pub(crate) fn apply_host_options(&mut self, options: &dashboard_core::protocol::MapOptions) {
        self.picker.hidden = options.picker_hidden;
        self.picker.country_pinned = options.country_pinned;
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
            if let Some(document) = &self.document
                && self
                    .map_mode
                    .unavailable_in(document.capture_mode())
                    .is_some()
            {
                self.map_mode = MapMode::default();
            }
        }
    }

    pub(crate) fn take_view_events(&mut self) -> Vec<ViewEvent> {
        if let Some(document) = &self.document
            && !self
                .picker
                .modes_for
                .as_ref()
                .is_some_and(|known| std::ptr::eq(known.as_ptr(), Arc::as_ptr(&document.content)))
        {
            self.picker.modes_for = Some(Arc::downgrade(&document.content));
            let four = self.head_rows.four_buttons(super::head_rows::head(super::live_of(&self.playhead_map)), document);
            let modes = choosable_modes(document, four)
                .iter()
                .map(|mode| {
                    let button = mode_button(document, mode);
                    dashboard_core::protocol::MapModeChoice {
                        token: button.token,
                        label: button.name,
                        unavailable: button.unavailable,
                    }
                })
                .collect();
            self.picker.events.push(ViewEvent::Modes(modes));
        }
        std::mem::take(&mut self.picker.events)
    }

    pub(crate) fn picker_toggles(&self) -> Toggles {
        let host = self.dashboard_map || self.host_graphics;
        Toggles {
            day_night: host.then_some(self.graphics.day_night),
            player_counters: Some(self.player_counters),
            counter_colors: Some(
                self.counter_view.settings.color_mode
                    == viewer_counter_kit::CounterColorMode::Country,
            ),
            allied_plans: host.then_some(self.graphics.show_allied_plans),
            fog: self
                .document
                .as_ref()
                .is_none_or(|document| {
                    viewer_foundation::perspective::fog_recorded(document.capture_mode())
                })
                .then_some(self.perspective.fog),
            radar: Some(self.picker.radar),
        }
    }

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

    pub(crate) fn draw_picker(
        &mut self,
        document: &Document,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
    ) -> Option<Result<picker::PickerDrawn, String>> {
        if self.picker.hidden {
            return None;
        }
        let toggles = if self.options.window_dump {
            dump_toggles(document, &self.options)
        } else {
            self.picker_toggles()
        };
        let four = self.head_rows.four_buttons(super::head_rows::head(super::live_of(&self.playhead_map)), document);
        self.refresh_find_view(document);
        let drawn = self.picker.draw(
            document,
            &self.map_mode,
            toggles,
            extent,
            hovers,
            time,
            advance,
            input.clone(),
            four,
        );
        Some(drawn.map(|mut drawn| {
            self.draw_map_windows(document, extent, hovers, time, advance, input, &mut drawn);
            drawn
        }))
    }

    pub(crate) fn picker_click(&mut self, document: &Document, control: &Control, secondary: bool) {
        match control {
            Control::FindView => self.picker.find.toggle(self.picker_now()),
            Control::Minimap(viewer_combat::minimap::Control::Handle) => self.toggle_minimap(document),
            Control::Minimap(viewer_combat::minimap::Control::OffensivePing) => self.picker.minimap.arm(true),
            Control::Minimap(viewer_combat::minimap::Control::DefensivePing) => self.picker.minimap.arm(false),
            Control::Find(find) => self.picker.find.click(*find),
            _ => {}
        }
        let slots = slot_modes(document, &self.picker.slots);
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
        let hosted = self.embedded.is_some() || self.dashboard_map;
        if let Some(setting) = toggle_request(name, self.picker_toggles(), hosted) {
            self.picker
                .events
                .push(ViewEvent::Toggle(dashboard_core::protocol::MapToggle {
                    setting,
                }));
            return;
        }
        match name {
            "button_radar" => self.picker.radar = !self.picker.radar,
            "button_fog" if !hosted => self.perspective.fog = !self.perspective.fog,
            "player_counters_toggle" => self.player_counters = !self.player_counters,
            "button_counters_color_mode" => {
                use viewer_counter_kit::CounterColorMode;
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

    pub(crate) fn picker_keys(&mut self, ui: &egui::Ui, document: &Document, extent: [f32; 2]) {
        if self.find_view_keys(ui, document, extent, self.picker_now()) {
            return;
        }
        let minimap = viewer_combat::minimap::handle_shortcut(&document.combat, extent)
            .ok()
            .flatten()
            .and_then(|text| parse_shortcut(&text));
        let toggles = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(event, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                    if minimap.is_some_and(|(wanted, wanted_key)| wanted_key == *key && modifiers.matches_exact(wanted)))
            })
        });
        if toggles {
            self.toggle_minimap(document);
        }
        let four = self.head_rows.four_buttons(super::head_rows::head(super::live_of(&self.playhead_map)), document);
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

    fn options(fog: bool, mode: Option<&str>) -> dashboard_core::protocol::MapOptions {
        dashboard_core::protocol::MapOptions {
            country: None,
            country_slot: None,
            country_pinned: false,
            fog,
            map_mode: mode.map(str::to_owned),
            map_mode_slots: Vec::new(),
            picker_hidden: false,
            observed: None,
        }
    }

    #[test]
    fn a_fog_change_from_the_host_never_holds_back_the_pickers_choice() {
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
    fn radar_toggle_controls_range_symbols_independently_of_station_icons() {
        let mut view = viewer();
        assert_eq!(view.picker_toggles().radar, Some(false));
        let graphics = view.graphics;
        view.picker_toggle("button_radar");
        assert_eq!(view.picker_toggles().radar, Some(true));
        assert_eq!(view.graphics, graphics);
        view.picker_toggle("button_radar");
        assert_eq!(view.picker_toggles().radar, Some(false));
    }

    #[test]
    fn a_toggle_asks_the_host_to_flip_its_setting() {
        use dashboard_core::protocol::MapSetting;
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

    #[test]
    fn a_loaded_recording_takes_settings_options_below_the_command_line() {
        use content_reader_settings::settings::MapOptions;
        use viewer_counter_kit::CounterColorMode;
        let game = MapOptions {
            day_night: false,
            allied_plans: true,
            country_counter_colours: true,
            nato_counters: true,
        };
        let mut window = viewer();
        window.take_game_options(&game);
        assert!(!window.graphics.day_night && window.graphics.show_allied_plans);
        assert_eq!(window.counter_view.settings.color_mode, CounterColorMode::Country);
        assert!(window.counter_view.settings.nato);
        let mut flagged = viewer();
        flagged.options.day_night = Some(true);
        flagged.options.allied_plans = Some(false);
        flagged.take_game_options(&game);
        assert!(flagged.graphics.day_night && !flagged.graphics.show_allied_plans);
        let mut hosted = viewer();
        hosted.host_graphics = true;
        hosted.graphics.day_night = true;
        hosted.take_game_options(&game);
        assert!(hosted.graphics.day_night && !hosted.graphics.show_allied_plans);
    }

    fn toggles(viewer: &mut Viewer) -> Vec<dashboard_core::protocol::MapToggle> {
        viewer
            .take_view_events()
            .into_iter()
            .filter_map(|event| match event {
                ViewEvent::Toggle(toggle) => Some(toggle),
                ViewEvent::Options(_) | ViewEvent::Modes(_) | ViewEvent::Observe(_) => None,
            })
            .collect()
    }

    #[test]
    fn toggles_flip_on_the_host_and_show_what_it_last_applied() {
        use dashboard_core::protocol::{DashboardMapView, MapAction, MapOptions, MapSetting, MapToggle, ResolvedMapSettings, ViewBounds};
        let mut viewer = viewer();
        viewer.dashboard_map = true;
        viewer.graphics.day_night = true;
        viewer.perspective.fog = true;
        let flip = |setting| MapToggle { setting };
        let fog = |viewer: &Viewer| viewer.picker_toggles().fog;
        let mut frame = eframe::Frame::_new_kittest();
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
                hours_per_second: 1.0,
                hour: 0,
                fraction: 0.,
                sequence: 1,
                seek_sequence: None,
                composed_at: 0.0,
                clock_id: "page-1".into(),
                bounds,
                clip: bounds,
                visible: true,
            };
            let mut output = egui::Context::default()
                .run_ui(Default::default(), |ui| viewer.map_ui(ui, frame, &target));
            output.textures_delta.clear();
        };

        viewer.picker_toggle("button_fog");
        viewer.picker_toggle("button_fog");
        assert_eq!(fog(&viewer), Some(true));
        assert_eq!(
            toggles(&mut viewer),
            [flip(MapSetting::Fog), flip(MapSetting::Fog)]
        );
        dashboard(&mut viewer, &frame, true);
        assert_eq!(fog(&viewer), Some(true));
        dashboard(&mut viewer, &frame, false);
        assert_eq!(fog(&viewer), Some(false));
        configure(&mut viewer, &mut frame, true);
        configure(&mut viewer, &mut frame, false);
        assert_eq!(fog(&viewer), Some(false));
        viewer.picker_toggle("button_fog");
        assert_eq!(toggles(&mut viewer), [flip(MapSetting::Fog)]);

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
    }

    #[test]
    fn campaign_picker_requests_day_night_and_allied_plans_without_changing_them_locally() {
        use dashboard_core::protocol::{MapSetting, MapToggle, ResolvedMapSettings};
        let mut viewer = viewer();
        assert!(!viewer.dashboard_map);
        let mut settings = ResolvedMapSettings::default();
        settings.graphics.day_night = true;
        settings.graphics.show_allied_plans = false;
        viewer.set_embedded_graphics(&settings);
        for _ in 0..2 {
            viewer.picker_toggle("day_night_toggle");
            viewer.picker_toggle("button_allied_plans");
        }
        assert_eq!(viewer.picker_toggles().day_night, Some(true));
        assert_eq!(viewer.picker_toggles().allied_plans, Some(false));
        assert_eq!(
            toggles(&mut viewer),
            [MapSetting::DayNight, MapSetting::AlliedPlans]
                .repeat(2)
                .into_iter()
                .map(|setting| MapToggle { setting })
                .collect::<Vec<_>>()
        );
        settings.graphics.day_night = false;
        settings.graphics.show_allied_plans = true;
        viewer.set_embedded_graphics(&settings);
        assert_eq!(viewer.picker_toggles().day_night, Some(false));
        assert_eq!(viewer.picker_toggles().allied_plans, Some(true));
    }

    #[test]
    fn a_viewer_without_a_host_offers_no_host_owned_toggles() {
        let mut viewer = viewer();
        let shown =
            |viewer: &Viewer| (viewer.graphics.day_night, viewer.graphics.show_allied_plans);
        let before = shown(&viewer);
        assert_eq!(viewer.picker_toggles().day_night, None);
        assert_eq!(viewer.picker_toggles().allied_plans, None);
        viewer.picker_toggle("day_night_toggle");
        viewer.picker_toggle("button_allied_plans");
        assert!(toggles(&mut viewer).is_empty());
        assert_eq!(shown(&viewer), before);
    }

    #[test]
    fn campaign_saved_settings_reach_the_frame_without_changing_the_hour() {
        use dashboard_core::{map_settings::Store, protocol::CampaignMapSettings};
        let root =
            std::env::temp_dir().join(format!("hoi4-campaign-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let previous = std::env::var_os("LOCALAPPDATA");
        unsafe { std::env::set_var("LOCALAPPDATA", &root) };
        let mut host = Store::open();
        let service = dashboard::Service::new(Arc::new(|_| true)).unwrap();
        let replay = crate::playback::tests::fixture();
        let campaign = dashboard_core::map_settings::campaign_key(&replay);
        let widget = dashboard_core::map_settings::widget_key(&campaign, "another-map");
        let mut viewer = viewer();
        assert!(!viewer.dashboard_map);
        assert!(!viewer.playing);
        let hour = 60_828_400;
        let speeds = Ok(viewer_gpu::frame::GameSpeeds::test());
        let initial = viewer.follow_map_settings(&service, campaign.clone());
        let light = viewer.frame_day_night(hour, &speeds);
        assert_eq!(light.strength, 1.0);
        let unchanged = viewer.follow_map_settings(&service, campaign.clone());
        assert!(Arc::ptr_eq(&initial, &unchanged));

        let mut settings = CampaignMapSettings::default();
        settings.graphics.day_night = Some(false);
        host.save("$global".into(), settings).unwrap();
        assert!(
            viewer
                .follow_map_settings(&service, campaign.clone())
                .graphics
                .day_night
        );
        let generation = service.map_settings_generation();
        service.reload_map_settings();
        assert_ne!(service.map_settings_generation(), generation);
        let off = viewer.follow_map_settings(&service, campaign.clone());
        assert!(!Arc::ptr_eq(&initial, &off));
        assert_eq!(viewer.picker_toggles().day_night, Some(false));
        let dark = viewer.frame_day_night(hour, &speeds);
        assert_eq!(dark.strength, 0.0);
        assert_eq!(dark.hour_fraction, light.hour_fraction);
        assert_eq!(dark.sun_direction, light.sun_direction);

        let mut settings = CampaignMapSettings::default();
        settings.graphics.day_night = Some(true);
        host.save(widget, settings).unwrap();
        service.reload_map_settings();
        viewer.follow_map_settings(&service, campaign.clone());
        assert_eq!(viewer.frame_day_night(hour, &speeds).strength, 0.0);

        viewer.picker_toggle("day_night_toggle");
        let [toggle]: [dashboard_core::protocol::MapToggle; 1] =
            toggles(&mut viewer).try_into().unwrap();
        host.flip(campaign.clone(), toggle.setting).unwrap();
        assert_eq!(viewer.frame_day_night(hour, &speeds).strength, 0.0);
        service.reload_map_settings();
        viewer.follow_map_settings(&service, campaign);
        assert_eq!(viewer.picker_toggles().day_night, Some(true));
        assert_eq!(viewer.frame_day_night(hour, &speeds).strength, 1.0);
        assert!(!viewer.playing);
        service.shutdown();
        unsafe {
            match previous {
                Some(value) => std::env::set_var("LOCALAPPDATA", value),
                None => std::env::remove_var("LOCALAPPDATA"),
            }
        }
        std::fs::remove_file(root.join("BattlePlan").join("map-settings.json")).unwrap();
        std::fs::remove_file(root.join("BattlePlan").join("map-settings.lock")).unwrap();
        std::fs::remove_dir(root.join("BattlePlan")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn the_player_counters_toggle_keeps_the_viewing_countrys_own_counters() {
        let mut viewer = viewer();
        assert_eq!(viewer.picker_toggles().player_counters, Some(false));
        viewer.picker_toggle("player_counters_toggle");
        assert_eq!(viewer.picker_toggles().player_counters, Some(true));
        assert!(toggles(&mut viewer).is_empty());
        let filtered = |owners: Option<&[u32]>| {
            viewer
                .shown_counter_owners(
                    owners.map(|owners| std::sync::Arc::new(owners.iter().copied().collect())),
                    4,
                )
                .map(|owners| owners.iter().copied().collect::<Vec<_>>())
        };
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
            dashboard_core::protocol::MapAction::Configure { options },
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
        let hover = |stock| mode_hover(&MapMode::Stock(stock), false, [50, -50], || Ok(3));
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
        assert_eq!(hover(Stock::Terrain), ModeHover::Terrain { region: false });
        assert_eq!(
            hover(Stock::StrategicAir),
            ModeHover::StrategicAir(Default::default())
        );
        assert_eq!(
            hover(Stock::StrategicNavy),
            ModeHover::StrategicNavy(std::sync::Arc::new(Err(
                "naval dominance sources not prepared".into()
            )))
        );
        for stock in [Stock::Factions, Stock::Players, Stock::Raids] {
            assert_eq!(hover(stock), ModeHover::Nothing, "{stock:?}");
        }
        assert_eq!(
            mode_hover(&MapMode::Scripted("spheres".into()), false, [50, -50], || Ok(0)),
            ModeHover::Scripted("spheres".into())
        );
    }

    #[test]
    fn a_picked_region_hovers_as_the_region_builder_answers_its_mode() {
        use content_reader_core::map::ProvinceKind::*;
        use viewer_counter_products::map::province_tooltips::ModeHover;
        assert!(picks_region(Some(Sea), Some(40)));
        assert!(!picks_region(Some(Sea), None));
        assert!(!picks_region(Some(Land), Some(40)));
        assert!(!picks_region(Some(Lake), Some(40)));
        assert!(!picks_region(None, Some(40)));
        let hover = |mode: MapMode| mode_hover(&mode, true, [50, -50], || Ok(3));
        assert_eq!(
            hover(MapMode::Stock(Stock::Default)),
            ModeHover::SeaRegion(std::sync::Arc::new(Err("naval dominance sources not prepared".into())))
        );
        assert_eq!(hover(MapMode::Stock(Stock::Terrain)), ModeHover::Terrain { region: true });
        assert_eq!(hover(MapMode::Stock(Stock::StrategicAir)), ModeHover::StrategicAir(Default::default()));
        assert!(matches!(hover(MapMode::Stock(Stock::StrategicNavy)), ModeHover::StrategicNavy(_)));
        for stock in Stock::ALL.into_iter().filter(|stock| {
            ![Stock::Default, Stock::Terrain, Stock::StrategicAir, Stock::StrategicNavy].contains(stock)
        }) {
            assert_eq!(hover(MapMode::Stock(stock)), ModeHover::Region, "{stock:?}");
        }
        assert_eq!(hover(MapMode::Scripted("spheres".into())), ModeHover::Region);
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
        assert_eq!(
            parse_shortcut("RETURN"),
            Some((egui::Modifiers::NONE, egui::Key::Enter))
        );
        assert_eq!(
            parse_shortcut("ESCAPE"),
            Some((egui::Modifiers::NONE, egui::Key::Escape))
        );
        assert_eq!(
            parse_shortcut("ctrl+m"),
            Some((egui::Modifiers::CTRL, egui::Key::M))
        );
        assert_eq!(parse_shortcut(""), None);
    }
}
