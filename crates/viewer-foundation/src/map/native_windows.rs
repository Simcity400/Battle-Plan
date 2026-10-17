//! Shared native GUI registry, window handles and template asset demand.

use super::native_gui;
use crate::errors::invalid;
use installed_content::{ResolvedLayers, gui::native};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::{io, sync::Arc};

pub const MAPICONS_GUI: &str = "interface/mapicons.gui";
pub const NAVY_WINDOW: &str = "unit_counter_navy";
pub const NAVAL_COMBAT_WINDOW: &str = "naval_combat_mapicon";
pub const TINY_UNIT_COUNTER_WINDOW: &str = "GFX_mapicon_tiny_unit_counter";
pub const NAVAL_BASE_WINDOW: &str = "naval_base_mapicon";
pub const NAVAL_RESULT_WINDOW: &str = "naval_combat_results_mapicon";
pub const AIR_BASE_WINDOW: &str = "air_base_mapicon";
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
pub const FACTION_PING_WINDOW: &str = "faction_ping_map_icon";
/// The Operatives map mode's per-state intel container and its entry windows.
pub const INTEL_CONTAINER_WINDOW: &str = "intel_map_mode_map_icon";
pub const INTEL_SHIELD_WINDOW: &str = "counterintelligence_map_icon_entry";
pub const INTEL_OPERATIVE_WINDOW: &str = "operative_map_icon_entry";
pub const INTEL_OPERATION_WINDOW: &str = "operation_map_icon_entry";
pub const INTEL_MORE_WINDOW: &str = "intel_map_icon_more";
pub const ADJACENCY_RULE_WINDOW: &str = "adjacency_rule_mapicon";
/// A state's resource icon and the window of each of its rows.
pub const RESOURCE_WINDOW: &str = "resource_mapicon";
pub const RESOURCE_ITEM_WINDOW: &str = "resource_mapicon_item";

/// Native templates read once per GUI viewport and retained for every window consumer.
pub struct WindowCache {
    shared: Arc<ResolvedLayers>,
    by_viewport: Mutex<BTreeMap<[u32; 2], Arc<NativeWindows>>>,
}

impl WindowCache {
    pub fn new(shared: Arc<ResolvedLayers>) -> Self {
        Self {
            shared,
            by_viewport: Default::default(),
        }
    }

    /// The windows as a client whose GUI viewport is `viewport` reads them
    /// (`read_layers_at`: a `windowType`'s `max_resolution` holds that viewport), read once per
    /// viewport and kept: what a view lays out, constructs and draws in its own viewport.
    pub fn at(&self, viewport: [u32; 2]) -> io::Result<Arc<NativeWindows>> {
        let mut read = self
            .by_viewport
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(windows) = read.get(&viewport) {
            return Ok(Arc::clone(windows));
        }
        let windows = Arc::new(NativeWindows::load_at(&self.shared, viewport)?);
        read.insert(viewport, Arc::clone(&windows));
        Ok(windows)
    }
}

/// The map-icon windows drawn through the module. Every `.gui` file joins one registry
/// in content-stack load order ([`native::read_layers`]); the last registration wins.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeWindows {
    pub guis: Arc<native::Guis>,
    navy: (usize, usize),
    naval_combat: (usize, usize),
    /// `None` when the content declares no convoy addition window.
    tiny: Option<(usize, usize)>,
    /// `None` when the content declares no naval base window: no port item is placed.
    naval_base: Option<(usize, usize)>,
    /// `None` when the content declares no naval result window: no retained result icon is
    /// placed, counted.
    naval_result: Option<(usize, usize)>,
}

impl NativeWindows {
    /// Templates read in `viewport`, the GUI root extent at load. A `windowType` with
    /// `max_resolution` keeps that extent less its authored value. Naval counter and combat
    /// windows are required.
    pub fn load_at(layers: &ResolvedLayers, viewport: [u32; 2]) -> io::Result<Self> {
        Self::of(native::read_layers_at(
            layers,
            viewport.map(|value| value as i32),
        )?)
    }

    /// The windows of `guis`.
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
        Ok(Self {
            guis: Arc::new(guis),
            navy,
            naval_combat,
            tiny,
            naval_base,
            naval_result,
        })
    }

    /// The windows of one file's `bytes` read in the GUI viewport `viewport`, for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn read(bytes: &[u8], viewport: [i32; 2]) -> io::Result<Self> {
        Self::of(native::Guis {
            files: vec![(MAPICONS_GUI.to_owned(), native::read_at(bytes, viewport))],
        })
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

    /// Finds a GUI type in the shared registry.
    pub fn gui_type(&self, name: &[u8]) -> Option<&native::Element> {
        self.guis.template(name)
    }

    pub fn reader_failures(&self) -> Vec<(&str, &native::Refusal)> {
        self.guis.refusals()
    }

    /// Whether a registered `textBoxType` has `name`. Construction registers a session
    /// text sprite under that name when no sprite already has it.
    pub fn text_box_named(&self, name: &[u8]) -> bool {
        self.guis.text_box_named(name)
    }

    /// Whether `other` holds the same registry: the windows' instances are built from it.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.guis, &other.guis)
    }

    /// Every resource the templates draw, through construction's own lookups
    /// ([`native_gui::assets_in`]: the templates they name, scrollbar parts through their
    /// `parent`, edit boxes' texture files, and their authored texts' inline icons through the
    /// localisation and decode drawing uses), the windows' sets merged whole
    /// ([`native_gui::Assets::extend`]).
    pub fn assets(
        &self,
        localisation: &installed_content::localisation::Localisation,
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
        // Assets for capital and victory-point label updates.
        for name in [
            VICTORY_POINT_WINDOW.as_bytes(),
            CAPITAL_WINDOW.as_bytes(),
            FACTION_PING_WINDOW.as_bytes(),
            RAID_WINDOW.as_bytes(),
            AIR_BASE_WINDOW.as_bytes(),
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

    /// Resolves the parentless naval combat window's frame in `viewport` GUI pixels,
    /// or reports why no naval combat icon can be drawn.
    pub fn naval_combat_frame(&self, viewport: [i32; 2]) -> Result<native_gui::Frame, String> {
        native_gui::container_frame(self.naval_combat(), Some(viewport))
    }

    /// The naval base window's frame, as [`Self::naval_combat_frame`].
    pub fn naval_base_frame(
        &self,
        viewport: [i32; 2],
    ) -> Option<Result<native_gui::Frame, String>> {
        self.naval_base()
            .map(|base| native_gui::container_frame(base, Some(viewport)))
    }

    /// The naval result window's frame, as [`Self::naval_combat_frame`].
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
