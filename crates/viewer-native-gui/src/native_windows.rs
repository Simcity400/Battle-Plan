
use super::native_gui;
use viewer_runtime::errors::invalid;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::gui::native;
use std::sync::{Mutex, Weak};
use std::{io, sync::Arc};

pub const MAPICONS_GUI: &str = "interface/mapicons.gui";
pub const NAVY_WINDOW: &str = "unit_counter_navy";
pub const NAVAL_COMBAT_WINDOW: &str = "naval_combat_mapicon";
pub const TINY_UNIT_COUNTER_WINDOW: &str = "GFX_mapicon_tiny_unit_counter";
pub const NAVAL_BASE_WINDOW: &str = "naval_base_mapicon";
pub const NAVAL_RESULT_WINDOW: &str = "naval_combat_results_mapicon";
pub const AIR_BASE_WINDOW: &str = "air_base_mapicon";
pub const AIR_WING_WINDOW: &str = "air_wing_counter";
pub const AIR_MISSION_WINDOW: &str = "air_mission_mapicon";
pub const AIR_MISSION_ENTRY_WINDOW: &str =
    replay_derive::air_mission_counts::MISSION_ENTRY_TEMPLATE;
pub const ROCKET_SITE_WINDOW: &str = "rocket_site_mapicon";
pub const GUN_EMPLACEMENT_WINDOW: &str = "gun_emplacement_mapicon";
pub const RADAR_WINDOW: &str = "radar_mapicon";
pub const STRATEGIC_LOCATION_WINDOW: &str = "strategic_location_mapicon";
pub const NAVAL_HEADQUARTER_WINDOW: &str = "naval_headquarter_mapicon";
pub const FACILITY_WINDOW: &str = "special_facility_mapicon";
pub const LAND_COMBAT_WINDOW: &str = "land_combat_mapicon";
pub const VICTORY_POINT_WINDOW: &str = "victory_point_mapicon";
pub const CAPITAL_WINDOW: &str = "capital_mapicon";
pub const RAID_WINDOW: &str = "raid_map_icon";
pub const RAID_DETAILS_WINDOW: &str = "raid_map_target_details_window";
pub const RAID_DETAILS_ITEM_WINDOW: &str = "raid_target_entries_item";
pub const RAID_TYPE_ICON_WINDOW: &str = "raid_type_icon_entry";
pub const FACTION_PING_WINDOW: &str = "faction_ping_map_icon";
pub const INTEL_CONTAINER_WINDOW: &str = "intel_map_mode_map_icon";
pub const INTEL_SHIELD_WINDOW: &str = "counterintelligence_map_icon_entry";
pub const INTEL_OPERATIVE_WINDOW: &str = "operative_map_icon_entry";
pub const INTEL_OPERATION_WINDOW: &str = "operation_map_icon_entry";
pub const INTEL_MORE_WINDOW: &str = "intel_map_icon_more";
pub const ADJACENCY_RULE_WINDOW: &str = "adjacency_rule_mapicon";
pub const RESOURCE_WINDOW: &str = "resource_mapicon";
pub const RESOURCE_ITEM_WINDOW: &str = "resource_mapicon_item";
pub const OCCUPATION_WINDOW: &str = "resistance_compliance_map_icon";
pub const OCCUPATION_ITEM_WINDOW: &str = "resistance_compliance_map_icon_modifier_entry";

pub struct WindowCache {
    shared: Arc<ResolvedLayers>,
    layouts: Mutex<Vec<Weak<WindowLayout>>>,
}

impl WindowCache {
    pub fn new(shared: Arc<ResolvedLayers>) -> Self {
        Self {
            shared,
            layouts: Default::default(),
        }
    }

    pub fn at(&self, viewport: [u32; 2]) -> io::Result<NativeWindows> {
        let mut layouts = self.layouts.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        layouts.retain(|layout| layout.strong_count() > 0);
        let size = viewport.map(|value| value as i32);
        if let Some(windows) = layouts.iter().filter_map(Weak::upgrade).find(|windows| windows.guis.fits_viewport(size)) {
            return Ok(NativeWindows(windows));
        }
        let windows = NativeWindows::load_at(&self.shared, viewport)?;
        layouts.push(Arc::downgrade(&windows.0));
        Ok(windows)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeWindows(Arc<WindowLayout>);

impl std::ops::Deref for NativeWindows {
    type Target = WindowLayout;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, PartialEq)]
pub struct WindowLayout {
    pub guis: native::Guis,
    navy: (usize, usize),
    naval_combat: (usize, usize),
    tiny: Option<(usize, usize)>,
    naval_base: Option<(usize, usize)>,
    naval_result: Option<(usize, usize)>,
}

impl NativeWindows {
    pub fn load_at(layers: &ResolvedLayers, viewport: [u32; 2]) -> io::Result<Self> {
        Self::of(native::read_layers_at(
            layers,
            viewport.map(|value| value as i32),
        )?)
    }

    pub fn of(guis: native::Guis) -> io::Result<Self> {
        let index = |name: &str| {
            let found = guis
                .template(name.as_bytes())
                .filter(|window| window.kind == native::Kind::Container)?;
            guis.files.iter().enumerate().find_map(|(file, (_, gui))| {
                gui.templates()
                    .iter()
                    .position(|template| std::ptr::eq(template, found))
                    .map(|index| (file, index))
            })
        };
        let required = |name: &str| {
            index(name).ok_or_else(|| invalid(format!("no .gui file declares {name}")))
        };
        let navy = required(NAVY_WINDOW)?;
        let naval_combat = required(NAVAL_COMBAT_WINDOW)?;
        let tiny = index(TINY_UNIT_COUNTER_WINDOW);
        let naval_base = index(NAVAL_BASE_WINDOW);
        let naval_result = index(NAVAL_RESULT_WINDOW);
        Ok(Self(Arc::new(WindowLayout {
            guis,
            navy,
            naval_combat,
            tiny,
            naval_base,
            naval_result,
        })))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn read(bytes: &[u8], viewport: [i32; 2]) -> io::Result<Self> {
        Self::of(native::Guis::new(vec![(
            MAPICONS_GUI.to_owned(),
            native::read_at(bytes, viewport),
        )]))
    }

    fn at(&self, (file, index): (usize, usize)) -> &native::Element {
        &self.guis.files[file].1.templates()[index]
    }

    pub fn navy(&self) -> &native::Element {
        self.at(self.navy)
    }

    pub fn naval_combat(&self) -> &native::Element {
        self.at(self.naval_combat)
    }

    pub fn tiny(&self) -> Option<&native::Element> {
        self.tiny.map(|at| self.at(at))
    }

    pub fn naval_base(&self) -> Option<&native::Element> {
        self.naval_base.map(|at| self.at(at))
    }

    pub fn naval_result(&self) -> Option<&native::Element> {
        self.naval_result.map(|at| self.at(at))
    }

    pub fn gui_type(&self, name: &[u8]) -> Option<&native::Element> {
        self.guis.template(name)
    }

    pub fn reader_failures(&self) -> Vec<(&str, &native::Refusal)> {
        self.guis.refusals()
    }

    pub fn text_box_named(&self, name: &[u8]) -> bool {
        self.guis.text_box_named(name)
    }

    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn assets(
        &self,
        localisation: &content_reader_localisation::localisation::Localisation,
    ) -> native_gui::Assets {
        let mut all = native_gui::Assets::default();
        for window in [
            Some(self.navy()),
            Some(self.naval_combat()),
            self.tiny(),
            self.naval_base(),
            self.naval_result(),
        ]
        .into_iter()
        .flatten()
        {
            let gui_type = |name: &[u8]| self.gui_type(name);
            all.extend(native_gui::assets_in(window, &gui_type, localisation));
        }
        for name in [
            VICTORY_POINT_WINDOW.as_bytes(),
            CAPITAL_WINDOW.as_bytes(),
            FACTION_PING_WINDOW.as_bytes(),
            RAID_WINDOW.as_bytes(),
            RAID_DETAILS_WINDOW.as_bytes(),
            RAID_DETAILS_ITEM_WINDOW.as_bytes(),
            RAID_TYPE_ICON_WINDOW.as_bytes(),
            AIR_BASE_WINDOW.as_bytes(),
            AIR_WING_WINDOW.as_bytes(),
            AIR_MISSION_WINDOW.as_bytes(),
            AIR_MISSION_ENTRY_WINDOW.as_bytes(),
            ROCKET_SITE_WINDOW.as_bytes(),
            GUN_EMPLACEMENT_WINDOW.as_bytes(),
            RADAR_WINDOW.as_bytes(),
            STRATEGIC_LOCATION_WINDOW.as_bytes(),
            NAVAL_HEADQUARTER_WINDOW.as_bytes(),
            FACILITY_WINDOW.as_bytes(),
            LAND_COMBAT_WINDOW.as_bytes(),
            INTEL_CONTAINER_WINDOW.as_bytes(),
            INTEL_SHIELD_WINDOW.as_bytes(),
            INTEL_OPERATIVE_WINDOW.as_bytes(),
            INTEL_OPERATION_WINDOW.as_bytes(),
            INTEL_MORE_WINDOW.as_bytes(),
            ADJACENCY_RULE_WINDOW.as_bytes(),
            RESOURCE_WINDOW.as_bytes(),
            RESOURCE_ITEM_WINDOW.as_bytes(),
            OCCUPATION_WINDOW.as_bytes(),
            OCCUPATION_ITEM_WINDOW.as_bytes(),
            native_gui::text_sprites::TOOLTIP,
            b"defaultTextBox",
        ] {
            if let Some(window) = self.gui_type(name) {
                all.extend(native_gui::assets_in(
                    window,
                    &|name| self.gui_type(name),
                    localisation,
                ));
            }
        }
        all
    }

    pub fn naval_combat_frame(&self, viewport: [i32; 2]) -> Result<native_gui::Frame, String> {
        native_gui::container_frame(self.naval_combat(), Some(viewport))
    }

    pub fn naval_base_frame(
        &self,
        viewport: [i32; 2],
    ) -> Option<Result<native_gui::Frame, String>> {
        self.naval_base()
            .map(|base| native_gui::container_frame(base, Some(viewport)))
    }

    pub fn naval_result_frame(
        &self,
        viewport: [i32; 2],
    ) -> Option<Result<native_gui::Frame, String>> {
        self.naval_result()
            .map(|result| native_gui::container_frame(result, Some(viewport)))
    }
}

#[cfg(test)]
mod tests;
