use viewer_native_gui::native_gui;
use viewer_runtime::geometry::contains;
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod combat_markers;
pub mod counter_groups;
pub mod counter_portraits;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod flotilla;
pub mod gui_windows;
pub mod land_windows;
pub mod layouts;
pub mod naval_combat_grid;
pub mod naval_combat_occurrences;
pub mod shards;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod type_tint;

use crate::combat_markers::LandCombatDefines;
use crate::combat_markers::LandMarkers;
use crate::counter_groups::Bounds;
use crate::counter_groups::Extents;
use crate::counter_groups::GroupDefines;
use crate::counter_groups::Level;
use crate::layouts::ability_grid;
use crate::layouts::column_layout;
use crate::layouts::gun_counter;
use crate::layouts::land_counter;
use crate::layouts::window;
use content_reader_gui::gui::native;
use content_reader_map::map::MapData;
use content_reader_map::map::ProvinceKind;
use dashboard_text::table_access::TokenKeys;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use viewer_engagements::combat_selection::Hit;
use viewer_engagements::engagement_history::CombatFlags;
use viewer_foundation::allegiance;
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::building_facts;
use viewer_foundation::country_index::CountryIndex;
use viewer_foundation::country_index::FlagTags;
use viewer_foundation::mode_visibility::Priorities;
use viewer_foundation::mode_visibility::PriorityIcon;
use viewer_foundation::mode_visibility::PriorityTable;
use viewer_foundation::mode_visibility::Visibility;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::movement_timing::Report;
use viewer_foundation::perspective::SharingUnknown;
use viewer_foundation::perspective::Vision;
use viewer_foundation::tables::Available;
use viewer_foundation::tables::rows;
use viewer_foundation::templates;
use viewer_foundation::unit_rules::DEFENDING;
use viewer_foundation::unit_rules::SHIP_IN_PORT;
use viewer_foundation::unit_rules::STANDSTILL;
use viewer_foundation::visibility::FogContent;
use viewer_gpu::camera::Camera;
use viewer_gpu::camera::MapSize;
use viewer_gpu::icon_tracks::IconAnchor;
use viewer_gpu::icon_tracks::IconFamily;
use viewer_gpu::province_map::HeightField;
use viewer_gpu::province_schedule::Shared;
use viewer_gpu::stack_anchors::seed_default_anchors;
use viewer_gpu::terrain_content::TerrainContent;
use viewer_gpu::ui_scale::extent as ui_scale_extent;
use viewer_gpu::ui_scale::project;
use viewer_native_gui::counter_hovers::land::CounterHover;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::lazy_icons::LazyFlags;
use viewer_native_gui::native_gui::Assets;
use viewer_native_gui::native_gui::container_frame;
use viewer_native_gui::native_resources::GuiResources;
use viewer_native_gui::native_windows::NAVY_WINDOW;
use viewer_native_gui::native_windows::NativeWindows as SharedNativeWindows;
use viewer_native_gui::native_windows::WindowCache;
use viewer_native_gui::sprites;
use viewer_native_gui::sprites::Batch;
use viewer_native_gui::sprites::SpriteSheet;
use viewer_native_gui::sprites::icon_offset;
use viewer_native_gui::sprites::layer;
use viewer_native_gui::text;
use viewer_runtime::label_reads::VICTORY_POINT;
use viewer_runtime::unit_handle::Unit;
use viewer_unit_reads::counter_action;
use viewer_unit_reads::counter_anchors::stack_anchor;
use viewer_unit_reads::counter_inputs;
use viewer_unit_reads::counter_move;
use viewer_unit_reads::counter_order;
use viewer_unit_reads::counter_status;

pub const ZOOMED_IN_OFFSET: &str = "unit_counter_zoomed_in_offset";
pub const LAND_WINDOW: &str = "unit_counter";
pub const STACK_WINDOW: &str = "unit_mapicon";
pub const FACILITY_SPRITE_PREFIX: &str = "GFX_mapicon_facility_";
pub const GUN_WINDOW: &str = "unit_counter_railway_gun";
pub const ABILITY_WINDOW: &str = "active_ability_item";

pub const MEDIUM_FLAG_OVERLAY: &str = "gfx/interface/small_flag_overlay.tga";

pub const UNKNOWN_COUNT: &str = "?";

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    pub sprite: String,
    pub at: [f32; 2],
    pub scale: f32,
    pub frame: u32,
    pub rotation: f32,
    pub centered: bool,
    pub order: u32,
}

impl Child {
    pub fn frame_index(&self, default: u32) -> u32 {
        self.frame.checked_sub(1).unwrap_or(default)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextChild {
    pub font: Option<String>,
    pub order: u32,
    pub at: [f32; 2],
    pub max_width: f32,
    pub align: text::Align,
    pub truncate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterDefines {
    pub movement_speeds: [f32; 2],
    pub icons_cutoff: f32,
    pub units_cutoff: f32,
    pub air_base_cutoff: f32,
    pub naval_base_cutoff: f32,
    pub facility_cutoff: f32,
    pub naval_base_scale: f32,
    pub naval_base_highlighted_scale: f32,
    pub naval_mission_cutoff: f32,
    pub land_combat_cutoff: f32,
    pub naval_combat_cutoff: f32,
    pub radar_cutoff: f32,
    pub coarse_distance: [f32; 2],
    pub priorities: Priorities,
    pub groups: GroupDefines,
    pub split_selected_limit: i32,
    pub naval_counts: viewer_unit_reads::naval_count_approx::Defines,
    pub supply: counter_status::SupplyThresholds,
    pub tints: allegiance::Tints,
    pub land_combat: LandCombatDefines,
    pub type_colors: type_tint::Colors,
}

impl CounterDefines {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let number = |name| defines.number("NGraphics", name).map(|value| value as f32);
        let fixed = |name| {
            defines
                .fixed5("NGraphics", name)
                .map(viewer_gpu::camera::fixed5_camera_distance)
        };
        let int = |name| defines.int32("NGraphics", name).map(|value| value as f32);
        Ok(Self {
            movement_speeds: [
                number("LAND_UNIT_MOVEMENT_SPEED")?,
                number("NAVAL_UNIT_MOVEMENT_SPEED")?,
            ],
            icons_cutoff: fixed("UNITS_ICONS_DISTANCE_CUTOFF")?,
            units_cutoff: number("UNITS_DISTANCE_CUTOFF")?,
            air_base_cutoff: number("AIRBASE_ICON_DISTANCE_CUTOFF")?,
            naval_base_cutoff: number("NAVALBASE_ICON_DISTANCE_CUTOFF")?,
            facility_cutoff: number("FACILITY_DISTANCE_CUTOFF")?,
            naval_base_scale: defines.number("NMapIcons", "HOME_BASE_MAP_ICON_SCALE_DEFAULT")?
                as f32,
            naval_base_highlighted_scale: defines
                .number("NMapIcons", "HOME_BASE_MAP_ICON_SCALE_HIGHLIGHTED")?
                as f32,
            land_combat_cutoff: number("LAND_COMBAT_DISTANCE_CUTOFF")?,
            naval_combat_cutoff: number("NAVAL_COMBAT_DISTANCE_CUTOFF")?,
            radar_cutoff: number("RADAR_ICON_DISTANCE_CUTOFF")?,
            coarse_distance: [
                int("MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE")?,
                int("MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE_STRATEGIC")?,
            ],
            priorities: Priorities::load(defines)?,
            split_selected_limit: defines
                .int32("NGraphics", "MAP_ICONS_GROUP_SPLIT_SELECTED_LIMIT")?,
            naval_mission_cutoff: number("NAVAL_MISSION_ICONS_DISTANCE_CUTOFF")?,
            groups: GroupDefines::read(defines)?,
            naval_counts: viewer_unit_reads::naval_count_approx::Defines::read(defines)?,
            supply: counter_status::SupplyThresholds::read(defines)?,
            tints: allegiance::Tints::read(defines)?,
            land_combat: LandCombatDefines::read(defines)?,
            type_colors: type_tint::Colors::read(defines)?,
        })
    }

    pub fn coarse(&self, height: f32, strategic: bool) -> bool {
        height > self.coarse_distance[usize::from(strategic)]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn stack_priority(&self, height: f32, table: PriorityTable) -> i8 {
        self.level_priority(self.groups.grouping(height), table)
    }

    pub fn level_priority(&self, level: Level, table: PriorityTable) -> i8 {
        let icon = match level {
            Level::None => PriorityIcon::UnitsStack,
            _ => PriorityIcon::UnitsStackGroup,
        };
        self.priorities.get(table, icon)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const FIXTURE: Self = Self {
        movement_speeds: [11.0, 9.0],
        icons_cutoff: 840.0,
        units_cutoff: 135.0,
        air_base_cutoff: 840.0,
        naval_base_cutoff: 840.0,
        facility_cutoff: 840.0,
        naval_base_scale: 1.15,
        naval_base_highlighted_scale: 1.35,
        naval_mission_cutoff: 1420.0,
        land_combat_cutoff: 1420.0,
        naval_combat_cutoff: 1420.0,
        radar_cutoff: 1040.0,
        coarse_distance: [325.0, 325.0],
        priorities: Priorities::uniform([
            8, 9, 4, 2, 2, 2, 2, 18, 18, 17, 12, 12, 4, 2, 2, 8, 2, 8,
        ]),
        groups: GroupDefines::FIXTURE,
        split_selected_limit: 9,
        naval_counts: viewer_unit_reads::naval_count_approx::Defines::FIXTURE,
        supply: counter_status::SupplyThresholds::FIXTURE,
        tints: allegiance::Tints::FIXTURE,
        land_combat: LandCombatDefines::FIXTURE,
        type_colors: type_tint::Colors::FIXTURE,
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct LandCounter {
    pub size: [f32; 2],
    pub plate: Child,
    pub selected: Child,
    pub overlay: Child,
    pub type_icon: Child,
    pub bar_org: Child,
    pub bar_str: Child,
    pub supply: Child,
    pub attrition: Child,
    pub digin: Child,
    pub training: Child,
    pub action: Child,
    pub no_orders: Child,
    pub defending_area: Child,
    pub invalid_orders: Child,
    pub flag: Child,
    pub large_flag: Child,
    pub count: TextChild,
    pub hq_badge: Child,
    pub hq_offset: [f32; 2],
    pub portrait: Child,
    pub group_marker: Child,
    pub leader_enroute: Child,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GunCounter {
    pub size: [f32; 2],
    pub plate: Child,
    pub selected: Child,
    pub group_marker: Child,
    pub overlay: Child,
    pub no_orders: Child,
    pub type_icon: Child,
    pub count: TextChild,
    pub bar_str: Child,
    pub flag: Child,
    pub large_flag: Child,
    pub supply: Child,
    pub action: Child,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AbilityGrid {
    pub at: [f32; 2],
    pub slot: [f32; 2],
    pub order: u32,
    pub item_size: [f32; 2],
    pub icon_at: [f32; 2],
    pub scale: f32,
    pub from_centre: bool,
    pub centred: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColumnLayout {
    pub stack_size: [f32; 2],
    pub zoomed_in: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layouts {
    pub land: Result<LandCounter, String>,
    pub gun: Result<GunCounter, String>,
    pub abilities: Option<AbilityGrid>,
    pub columns: Result<ColumnLayout, String>,
    pub missing: Vec<String>,
    pub unsupported: Vec<String>,
}

impl Layouts {
    pub fn read(windows: &SharedNativeWindows) -> Self {
        let guis = &windows.guis;
        let mut missing = Vec::new();
        let mut unsupported = Vec::new();
        let land = land_counter(guis, &mut missing).map_err(|why| why.to_string());
        let gun = gun_counter(guis, &mut missing)
            .and_then(|gun| {
                gun.ok_or_else(|| invalid(format!("no .gui file declares {GUN_WINDOW}")))
            })
            .map_err(|why| why.to_string());
        let abilities = window(guis, LAND_WINDOW).ok().and_then(|land| {
            match ability_grid(guis, land, &mut missing, &mut unsupported) {
                Ok(grid) => grid,
                Err(why) => {
                    unsupported.push(why.to_string());
                    None
                }
            }
        });
        Self {
            land,
            gun,
            abilities,
            columns: column_layout(guis).map_err(|why| why.to_string()),
            missing,
            unsupported,
        }
    }

    pub fn sprite_names(&self) -> Vec<&str> {
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for child in self
            .land
            .iter()
            .flat_map(|land| {
                [
                    &land.plate,
                    &land.selected,
                    &land.overlay,
                    &land.bar_org,
                    &land.bar_str,
                    &land.supply,
                    &land.attrition,
                    &land.digin,
                    &land.training,
                    &land.action,
                    &land.no_orders,
                    &land.defending_area,
                    &land.invalid_orders,
                    &land.hq_badge,
                ]
            })
            .chain(self.gun.iter().flat_map(|gun| {
                [
                    &gun.plate,
                    &gun.selected,
                    &gun.overlay,
                    &gun.no_orders,
                    &gun.bar_str,
                    &gun.supply,
                    &gun.action,
                ]
            }))
        {
            if !child.sprite.is_empty() {
                names.insert(child.sprite.as_str());
            }
        }
        names.insert("GFX_onmap_unit_counter_empty");
        names.into_iter().collect()
    }
}

#[derive(Clone, Debug)]
pub struct ActionContent {
    pub sheet: Result<SpriteSheet, Cow<'static, str>>,
    pub short: BTreeMap<u32, Cow<'static, str>>,
}

impl ActionContent {
    pub fn load(child: &Child, window: &str, sheet: Option<&SpriteSheet>) -> Self {
        let reason = if child.sprite.is_empty() {
            Some("missing")
        } else if child.centered {
            Some("centerPosition unsupported")
        } else if child.rotation != 0.0 {
            Some("rotation unsupported")
        } else if sheet.is_none() {
            Some("unavailable")
        } else {
            None
        };
        let sheet = match reason {
            Some(reason) => Err(format!(
                "{window}.onmap_action_icon sprite {} {reason}",
                child.sprite
            )
            .into()),
            None => Ok(sheet.unwrap().clone()),
        };
        let short = sheet
            .as_ref()
            .ok()
            .map(|sheet| {
                (sheet.frames..=12)
                    .map(|frame| {
                        (
                            frame,
                            format!(
                                "{window}.onmap_action_icon frame {} exceeds {}'s {} frames",
                                frame + 1,
                                child.sprite,
                                sheet.frames
                            )
                            .into(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { sheet, short }
    }

    pub fn frame(
        &self,
        action: counter_action::Action,
        own: bool,
        coarse: bool,
    ) -> Result<Option<u32>, &str> {
        if !own || coarse {
            return Ok(None);
        }
        let Some(frame) = counter_action::action_frame(action?, true) else {
            return Ok(None);
        };
        self.sheet.as_ref().map_err(|reason| reason.as_ref())?;
        if let Some(reason) = self.short.get(&frame) {
            return Err(reason.as_ref());
        }
        Ok(Some(frame))
    }
}

pub fn seeded_only(anchors: &BTreeMap<(u32, u32), [f32; 3]>, province: u32, kind: u32) -> bool {
    !anchors.contains_key(&(province, kind))
        && (kind != VICTORY_POINT || !anchors.contains_key(&(province, DEFENDING)))
}

pub struct CounterInputs<'a> {
    pub content: &'a CounterMap,
    pub equipment: Result<&'a content_reader_key_names::equipment::Database, &'a str>,
    pub assets: &'a CounterArt,
    pub windows: &'a SharedNativeWindows,
    pub fonts: &'a text::Fonts,
    pub localisation: &'a content_reader_localisation::localisation::Localisation,
    pub recorded_language: &'a str,
    pub map: &'a MapData,
    pub hour: i32,
    pub camera: Camera,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
    pub moved: &'a counter_move::MovedHours,
    pub movement: Option<&'a MovementContent>,
    pub heights: HeightField<'a>,
    pub settings: CounterSettings,
    pub display: &'a CounterDisplay,
    pub selected: &'a BTreeSet<Unit>,
    pub selected_air: &'a [u64],
    pub selected_air_bases: &'a [u64],
    pub selected_fleets: &'a [u64],
    pub hovered: Option<Unit>,
    pub pressed: Option<Unit>,
    pub plate_hovered: Option<Unit>,
    pub schedule: &'a Shared,
    pub movement_history: &'a viewer_unit_reads::counter_move::SharedMovement,
    pub visibility: Visibility,
    pub art: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterDisplay {
    pub land: bool,
    pub railway_guns: bool,
    pub naval: bool,
    pub ports: bool,
    pub owners: Option<std::sync::Arc<BTreeSet<u32>>>,
}

impl Default for CounterDisplay {
    fn default() -> Self {
        Self {
            land: true,
            railway_guns: true,
            naval: true,
            ports: true,
            owners: None,
        }
    }
}

impl CounterDisplay {
    pub fn owner(&self, owner: u32) -> bool {
        self.owners
            .as_ref()
            .is_none_or(|owners| owners.contains(&owner))
    }

    pub fn land_plate(&self, gun: bool, owner: u32) -> bool {
        (if gun { self.railway_guns } else { self.land }) && self.owner(owner)
    }

    pub fn naval_plate(&self, owner: u32) -> bool {
        self.naval && self.owner(owner)
    }

    pub fn port_item(&self, owner: Option<u32>) -> bool {
        self.ports && owner.is_none_or(|owner| self.owner(owner))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CounterColorMode {
    Allegiance,
    Country,
}

pub fn counter_plate_color(
    mode: CounterColorMode,
    tints: &allegiance::Tints,
    countries: &Countries,
    side: Allegiance,
    country: Option<u32>,
) -> Result<[f32; 4], String> {
    match mode {
        CounterColorMode::Allegiance => Ok(tints.of(side)),
        CounterColorMode::Country => {
            let country = country.ok_or_else(|| {
                "a counter's logical owner is not a linked country, so its plate colour \
                 in country-colour mode is unknown"
                    .to_owned()
            })?;
            countries.ui_color(country)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CounterSettings {
    pub color_mode: CounterColorMode,
    pub nato: bool,
}

impl CounterColorMode {
    pub const DEFAULT: Self =
        Self::of(dashboard_core::map_display::option_defaults::COUNTRY_COUNTER_COLOURS);

    pub const fn of(country_colours: bool) -> Self {
        if country_colours {
            Self::Country
        } else {
            Self::Allegiance
        }
    }
}

impl Default for CounterSettings {
    fn default() -> Self {
        Self {
            color_mode: CounterColorMode::DEFAULT,
            nato: dashboard_core::map_display::option_defaults::NATO_COUNTERS,
        }
    }
}

impl CounterInputs<'_> {
    pub fn route_world(&self) -> Option<counter_move::RouteWorld<'_>> {
        Some(counter_move::RouteWorld {
            map: &self.movement?.map,
            heights: self.heights,
            width: self.camera.map.x(),
        })
    }
}

impl CounterInputs<'_> {
    pub fn screen(&self, world: [f32; 3]) -> Option<[f32; 2]> {
        project(&self.camera, world, self.viewport, self.ui_scale)
    }

    pub fn navy_size(&self) -> Result<[f32; 2], String> {
        container_frame(
            self.windows.navy(),
            Some(gui_resources_viewport(self.viewport, self.ui_scale)),
        )
        .map(|frame| frame.size.map(|value| value as f32))
    }

    pub fn extent(&self) -> [f32; 2] {
        ui_scale_extent(self.viewport, self.ui_scale)
    }

    pub fn selected_join(&self, first: Unit, candidate: Unit, height: f32) -> bool {
        let split = self.selected.len() as i32 <= self.content.defines.split_selected_limit
            && height < self.content.defines.units_cutoff;
        selection_joins(self.selected, first, candidate, split)
    }

    pub fn priority(&self, icon: PriorityIcon) -> i8 {
        self.content
            .defines
            .priorities
            .get(self.visibility.priorities, icon)
    }

    pub fn icon_height(&self, anchor: [f32; 3]) -> f32 {
        self.camera.icon_distance(anchor)
    }

    pub fn shows(&self, cutoff: f32) -> bool {
        self.passes(|height| visible(height, cutoff))
    }

    pub fn passes(&self, gate: impl Fn(f32) -> bool) -> bool {
        self.camera.free().is_some() || gate(self.camera.height)
    }

    pub fn icon_anchor(
        &self,
        family: IconFamily,
        anchor: [f32; 3],
        cutoff: f32,
    ) -> Option<IconAnchor> {
        Some(IconAnchor {
            world: anchor,
            cutoff,
            family,
            moves: None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirMissionPart {
    Mission(usize),
    GroundCrews,
    Supply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirMissionClick {
    SelectMission(usize),
    OpenRegion,
    Nothing,
}

impl AirMissionPart {
    pub fn click(self) -> AirMissionClick {
        match self {
            Self::Mission(mission) => AirMissionClick::SelectMission(mission),
            Self::GroundCrews => AirMissionClick::Nothing,
            Self::Supply => AirMissionClick::OpenRegion,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AirMissionHover {
    pub region: u32,
    pub part: AirMissionPart,
    pub rect: [f32; 4],
    pub layer: u64,
    pub text: Arc<viewer_foundation::tooltip_text::Tooltip>,
}
impl AirMissionHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        viewer_runtime::geometry::contains(self.rect, point)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavalMissionHit {
    pub region: u32,
    pub mission: u32,
    pub rect: [f32; 4],
    pub layer: u64,
    pub text: Option<Arc<viewer_foundation::tooltip_text::Tooltip>>,
}

impl NavalMissionHit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        viewer_runtime::geometry::contains(self.rect, point)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavalRegionalControl {
    Combat(usize),
    Results(usize),
    Mines,
    Accidents,
    Convoy(u64),
}
#[derive(Clone, Debug, PartialEq)]
pub struct NavalRegionalHover {
    pub region: u32,
    pub control: NavalRegionalControl,
    pub rect: [f32; 4],
    pub layer: u64,
    pub hit: Option<viewer_native_gui::native_gui::HitGeometry>,
    pub supported: bool,
    pub text: Option<Arc<viewer_foundation::tooltip_text::Tooltip>>,
}
impl NavalRegionalHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.hit.as_ref().map_or_else(
            || viewer_runtime::geometry::contains(self.rect, point),
            |hit| hit.contains(point),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AirGridHover {
    pub family: IconFamily,
    pub members: Option<Arc<Vec<u64>>>,
    pub text: Arc<viewer_foundation::tooltip_text::Tooltip>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CounterReport {
    pub unit_hits: Vec<viewer_native_gui::counter_hovers::unit_hits::Hit>,
    pub base_hits: Vec<viewer_native_gui::counter_hovers::unit_hits::Blocker>,
    pub radar_hits: BTreeMap<usize, u32>,
    pub air_base_hits: BTreeMap<usize, (u32, i32)>,
    pub naval_base_hits: BTreeMap<usize, u32>,
    pub air_grid_hits: BTreeMap<usize, AirGridHover>,
    pub air_mission_hits: Vec<AirMissionHover>,
    pub naval_mission_hits: Vec<NavalMissionHit>,
    pub naval_region_hovers: Vec<NavalRegionalHover>,
    pub regional_naval_unit_hits: BTreeSet<usize>,
    pub combat_hits: Vec<Hit>,
    pub naval_marker_hovers: Vec<Hit>,
    pub land_hovers: Vec<CounterHover>,
    pub task_force_hovers: Vec<viewer_native_gui::counter_hovers::task_force::Hover>,
    pub task_force_port_hovers: Vec<viewer_native_gui::counter_hovers::task_force_port::Hover>,
    pub eta: Report,
    pub land_counters: usize,
    pub railway_guns: usize,
    pub unavailable_railway_guns: usize,
    pub unavailable_railway_gun_locations: usize,
    pub unplaced_railway_gun_routes: usize,
    pub unavailable_ability_icons: usize,
    pub naval_counters: usize,
    pub navy_counter_gaps: BTreeMap<Cow<'static, str>, usize>,
    pub naval_window_notes: BTreeMap<Cow<'static, str>, usize>,
    pub land_counter_gaps: BTreeMap<Cow<'static, str>, usize>,
    pub land_action_gaps: BTreeMap<Cow<'static, str>, usize>,
    pub unplaced_task_forces: BTreeMap<&'static str, BTreeSet<u64>>,
    pub unplaced_task_force_routes: BTreeMap<&'static str, BTreeSet<u64>>,
    pub unavailable_naval_flags: usize,
    pub unavailable_type_tints: usize,
    pub type_tint_problem: String,
    pub no_intel_counters: usize,
    pub no_intel_combats: usize,
    pub air_bases: usize,
    pub hidden_air_bases: usize,
    pub naval_bases: usize,
    pub naval_headquarters: usize,
    pub facilities: usize,
    pub rocket_sites: usize,
    pub gun_emplacements: usize,
    pub radars: usize,
    pub strategic_locations: usize,
    pub unknown_bases: BaseUnknowns,
    pub unresolved_facility_sprites: BTreeMap<String, usize>,
    pub icon_gui_notes: BTreeMap<String, usize>,
    pub faction_pings: usize,
    pub faction_pin_hovers: Vec<PinHover>,
    pub pin_gui_refusals: BTreeMap<String, usize>,
    pub faction_ping_limits: PinReport,
    pub land_combats: usize,
    pub combat_gaps: BTreeMap<(&'static str, String), BTreeSet<u64>>,
    pub combat_limits: BTreeMap<(&'static str, String), BTreeSet<u64>>,
    pub unknown_sharing: BTreeMap<(&'static str, SharingUnknown), usize>,
    pub naval_combats: usize,
    pub unavailable_naval_combats: usize,
    pub base_frame_naval_combats: usize,
    pub naval_result_icons: usize,
    pub naval_grid_gaps: BTreeMap<Cow<'static, str>, usize>,
    pub unavailable_naval_results: usize,
    pub naval_result_gaps: BTreeMap<Cow<'static, str>, usize>,
    pub undeclared_result_sprites: BTreeMap<&'static str, usize>,
    pub naval_mission_entries: usize,
    pub undecided_naval_mission_children: BTreeMap<String, usize>,
    pub authored_naval_mission_buttons: usize,
    pub undefined_naval_mission_sprites: BTreeSet<String>,
    pub unanchored_naval_missions: usize,
    pub naval_mission_gaps: BTreeMap<Cow<'static, str>, usize>,
    pub naval_mission_limits: BTreeSet<&'static str>,
    pub unanchored: usize,
    pub moving: usize,
    pub missing_flags: BTreeSet<String>,
    pub counter_flag_gaps: BTreeMap<String, usize>,
    pub refused_tokens: BTreeSet<String>,
    pub grouping: Option<Level>,
    pub grouping_gaps: BTreeMap<String, usize>,
    pub groups: usize,
    pub perspective: Arc<Vision>,
    pub pointed: Option<Unit>,
}

impl CounterReport {
    pub fn base_ordinal(&self) -> u32 {
        (self.air_bases
            + self.naval_bases
            + self.naval_headquarters
            + self.facilities
            + self.rocket_sites
            + self.gun_emplacements
            + self.radars
            + self.strategic_locations
            + self.naval_counters) as u32
    }

    pub fn combat_ordinal(&self) -> u32 {
        (self.land_combats + self.naval_combats + self.naval_result_icons) as u32
    }

    pub fn combat_gap_lines(&self) -> impl Iterator<Item = String> + '_ {
        fn ids(combats: &BTreeSet<u64>) -> String {
            let ids: Vec<String> = combats.iter().map(u64::to_string).collect();
            ids.join(", ")
        }
        self.combat_gaps
            .iter()
            .map(|((kind, cause), combats)| {
                format!(
                    "{kind} combat icons: {} undecided: {cause}; combats {}",
                    combats.len(),
                    ids(combats)
                )
            })
            .chain(self.combat_limits.iter().map(|((kind, limit), combats)| {
                format!(
                    "{kind} combats: {} {limit}; combats {}",
                    combats.len(),
                    ids(combats)
                )
            }))
    }

    pub fn warnings(&self) -> impl Iterator<Item = String> + '_ {
        let retained = (self.naval_combats > 0).then(|| {
            format!(
                "Naval combat icons: {} drawn from this view's own windows, kept since the view \
                 started; the recording client's windows from before that are in no recording.",
                self.naval_combats
            )
        });
        let base_frames = (self.base_frame_naval_combats > 0).then(|| {
            format!(
                "Naval combat icons: {} keep the frame their pool slot last had in this view (unengaged and spill-over icons retain that frame); the recording client's pool before the view started is in no recording.",
                self.base_frame_naval_combats
            )
        });
        self.refused_tokens
            .iter()
            .map(|reason| format!("Counters: refused token: {reason}."))
            .chain(self.grouping_gaps.iter().map(|(why, count)| {
                format!("Counter grouping: {count} comparisons unknown; groups kept separate: {why}.")
            }))
            .chain((self.unavailable_type_tints != 0).then(|| {
                format!(
                    "Counters: {} counters lack a relation input ({}); type icons withheld, \
                     plates not our own drawn as other.",
                    self.unavailable_type_tints, self.type_tint_problem
                )
            }))
            .chain(
                self.pin_gui_refusals
                    .iter()
                    .map(|(why, count)| format!("Faction pins: {count} {why}")),
            )
            .chain((self.unavailable_naval_flags != 0).then(|| {
                format!(
                    "Counters: {} mixed-owner port flags unavailable: incomplete Ships census.",
                    self.unavailable_naval_flags
                )
            }))
            .chain((self.unavailable_railway_gun_locations != 0).then(|| {
                format!(
                    "Counters: {} railway-gun location warnings undecided: current raw railway pair count or cooldown unavailable.",
                    self.unavailable_railway_gun_locations
                )
            }))
            .chain(self.unknown_sharing.iter().map(|((what, cause), count)| {
                format!("Counters: {count} {what}: {}.", cause.clause())
            }))
            .chain(self.task_force_lines())
            .chain(self.base_icon_lines())
            .chain(retained)
            .chain(base_frames)
            .chain(
                self.naval_grid_gaps
                    .iter()
                    .map(|(gap, count)| format!("Naval combat grids: {count} x {gap}.")),
            )
            .chain(
                self.navy_counter_gaps
                    .iter()
                    .map(|(gap, count)| format!("Naval counters: {count} not drawn: {gap}.")),
            )
            .chain(
                self.naval_window_notes
                    .iter()
                    .map(|(note, count)| format!("Naval map-icon windows: {count} x {note}.")),
            )
            .chain(
                self.land_counter_gaps
                    .iter()
                    .map(|(gap, count)| format!("Land counters: {count} not drawn: {gap}.")),
            )
            .chain(
                self.land_action_gaps
                    .iter()
                    .map(|(gap, count)| format!("Land counter actions: {count} not drawn: {gap}.")),
            )
            .chain(self.counter_flag_gaps.iter().map(|(gap, count)| {
                format!("Counter flags: {count} not drawn: {gap}.")
            }))
            .chain(self.naval_result_gaps.iter().map(|(gap, count)| {
                format!("Naval result icons: {count} x {gap}.")
            }))
            .chain(
                self.undeclared_result_sprites
                    .iter()
                    .map(|(sprite, count)| {
                        format!(
                            "Naval result icons: {count} keep their drawable: the content declares no sprite {sprite}."
                        )
                    }),
            )
            .chain(self.naval_mission_warnings())
    }

    pub fn base_icon_lines(&self) -> impl Iterator<Item = String> + '_ {
        let facilities = self.unresolved_facility_sprites.iter().map(|(sprite, count)| {
            format!(
                "Counters: {count} special-facility icons keep their drawable: the content declares no sprite {sprite}."
            )
        });
        self.unknown_bases.lines().chain(facilities).chain(
            self.icon_gui_notes
                .iter()
                .map(|(why, count)| format!("Map icons: {count} x {why}.")),
        )
    }

    pub fn task_force_lines(&self) -> impl Iterator<Item = String> + '_ {
        let ids = |forces: &BTreeSet<u64>| {
            forces
                .iter()
                .map(|handle| (handle >> 32).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let absent = self
            .unplaced_task_forces
            .iter()
            .map(move |(reason, forces)| {
                format!(
                    "Counters: {} task forces not drawn: {reason}; task forces {}.",
                    forces.len(),
                    ids(forces)
                )
            });
        let standing = self
            .unplaced_task_force_routes
            .iter()
            .map(move |(reason, forces)| {
                format!(
                    "Counters: {} moving task forces stand on their province: {reason}; task forces {}.",
                    forces.len(),
                    ids(forces)
                )
            });
        absent.chain(standing)
    }

    pub fn sharing_unknown(&mut self, what: &'static str, cause: SharingUnknown) {
        *self.unknown_sharing.entry((what, cause)).or_default() += 1;
    }

    pub fn absorb_later(&mut self, later: &mut Self) {
        fn add_counts<K: Ord>(into: &mut BTreeMap<K, usize>, from: &mut BTreeMap<K, usize>) {
            for (key, count) in std::mem::take(from) {
                *into.entry(key).or_default() += count;
            }
        }
        fn unite_sets<K: Ord, T: Ord>(
            into: &mut BTreeMap<K, BTreeSet<T>>,
            from: &mut BTreeMap<K, BTreeSet<T>>,
        ) {
            for (key, mut set) in std::mem::take(from) {
                into.entry(key).or_default().append(&mut set);
            }
        }
        fn untouched<T: Default + PartialEq>(field: &T, name: &str) {
            debug_assert!(
                *field == T::default(),
                "a later counters step wrote {name}, which absorbing it would lose"
            );
        }
        let Self {
            unit_hits,
            base_hits,
            radar_hits,
            air_base_hits,
            naval_base_hits,
            air_grid_hits,
            air_mission_hits,
            naval_mission_hits,
            naval_region_hovers,
            regional_naval_unit_hits,
            combat_hits,
            naval_marker_hovers,
            land_hovers,
            task_force_hovers,
            task_force_port_hovers,
            eta,
            land_counters,
            railway_guns,
            unavailable_railway_guns,
            unavailable_railway_gun_locations,
            unplaced_railway_gun_routes,
            unavailable_ability_icons,
            naval_counters,
            navy_counter_gaps,
            naval_window_notes,
            land_counter_gaps,
            land_action_gaps,
            unplaced_task_forces,
            unplaced_task_force_routes,
            unavailable_naval_flags,
            unavailable_type_tints,
            type_tint_problem,
            no_intel_counters,
            no_intel_combats,
            air_bases,
            hidden_air_bases,
            naval_bases,
            naval_headquarters,
            facilities,
            rocket_sites,
            gun_emplacements,
            radars,
            strategic_locations,
            unknown_bases,
            unresolved_facility_sprites,
            icon_gui_notes,
            faction_pings,
            faction_pin_hovers,
            pin_gui_refusals,
            faction_ping_limits,
            land_combats,
            combat_gaps,
            combat_limits,
            unknown_sharing,
            naval_combats,
            unavailable_naval_combats,
            base_frame_naval_combats,
            naval_result_icons,
            naval_grid_gaps,
            unavailable_naval_results,
            naval_result_gaps,
            undeclared_result_sprites,
            naval_mission_entries,
            undecided_naval_mission_children,
            authored_naval_mission_buttons,
            undefined_naval_mission_sprites,
            unanchored_naval_missions,
            naval_mission_gaps,
            naval_mission_limits,
            unanchored,
            moving,
            missing_flags,
            counter_flag_gaps,
            refused_tokens,
            grouping,
            grouping_gaps,
            groups,
            perspective,
            pointed,
        } = later;
        self.unit_hits.append(unit_hits);
        self.base_hits.append(base_hits);
        untouched(&*radar_hits, "radar_hits");
        untouched(&*air_base_hits, "air_base_hits");
        untouched(&*naval_base_hits, "naval_base_hits");
        untouched(&*air_grid_hits, "air_grid_hits");
        self.air_mission_hits.append(air_mission_hits);
        self.naval_mission_hits.append(naval_mission_hits);
        self.naval_region_hovers.append(naval_region_hovers);
        untouched(&*regional_naval_unit_hits, "regional_naval_unit_hits");
        self.combat_hits.append(combat_hits);
        self.naval_marker_hovers.append(naval_marker_hovers);
        self.land_hovers.append(land_hovers);
        self.task_force_hovers.append(task_force_hovers);
        self.task_force_port_hovers.append(task_force_port_hovers);
        untouched(&*eta, "eta");
        self.land_counters += std::mem::take(land_counters);
        self.railway_guns += std::mem::take(railway_guns);
        self.unavailable_railway_guns += std::mem::take(unavailable_railway_guns);
        self.unavailable_railway_gun_locations += std::mem::take(unavailable_railway_gun_locations);
        self.unplaced_railway_gun_routes += std::mem::take(unplaced_railway_gun_routes);
        self.unavailable_ability_icons += std::mem::take(unavailable_ability_icons);
        self.naval_counters += std::mem::take(naval_counters);
        add_counts(&mut self.navy_counter_gaps, navy_counter_gaps);
        add_counts(&mut self.naval_window_notes, naval_window_notes);
        add_counts(&mut self.land_counter_gaps, land_counter_gaps);
        add_counts(&mut self.land_action_gaps, land_action_gaps);
        unite_sets(&mut self.unplaced_task_forces, unplaced_task_forces);
        unite_sets(
            &mut self.unplaced_task_force_routes,
            unplaced_task_force_routes,
        );
        self.unavailable_naval_flags += std::mem::take(unavailable_naval_flags);
        self.unavailable_type_tints += std::mem::take(unavailable_type_tints);
        untouched(&*type_tint_problem, "type_tint_problem");
        self.no_intel_counters += std::mem::take(no_intel_counters);
        self.no_intel_combats += std::mem::take(no_intel_combats);
        self.air_bases += std::mem::take(air_bases);
        self.hidden_air_bases += std::mem::take(hidden_air_bases);
        self.naval_bases += std::mem::take(naval_bases);
        self.naval_headquarters += std::mem::take(naval_headquarters);
        self.facilities += std::mem::take(facilities);
        self.rocket_sites += std::mem::take(rocket_sites);
        self.gun_emplacements += std::mem::take(gun_emplacements);
        self.radars += std::mem::take(radars);
        self.strategic_locations += std::mem::take(strategic_locations);
        untouched(&*unknown_bases, "unknown_bases");
        add_counts(
            &mut self.unresolved_facility_sprites,
            unresolved_facility_sprites,
        );
        add_counts(&mut self.icon_gui_notes, icon_gui_notes);
        self.faction_pings += std::mem::take(faction_pings);
        self.faction_pin_hovers.append(faction_pin_hovers);
        add_counts(&mut self.pin_gui_refusals, pin_gui_refusals);
        untouched(&*faction_ping_limits, "faction_ping_limits");
        self.land_combats += std::mem::take(land_combats);
        unite_sets(&mut self.combat_gaps, combat_gaps);
        unite_sets(&mut self.combat_limits, combat_limits);
        add_counts(&mut self.unknown_sharing, unknown_sharing);
        self.naval_combats += std::mem::take(naval_combats);
        self.unavailable_naval_combats += std::mem::take(unavailable_naval_combats);
        self.base_frame_naval_combats += std::mem::take(base_frame_naval_combats);
        self.naval_result_icons += std::mem::take(naval_result_icons);
        add_counts(&mut self.naval_grid_gaps, naval_grid_gaps);
        self.unavailable_naval_results += std::mem::take(unavailable_naval_results);
        add_counts(&mut self.naval_result_gaps, naval_result_gaps);
        add_counts(
            &mut self.undeclared_result_sprites,
            undeclared_result_sprites,
        );
        self.naval_mission_entries += std::mem::take(naval_mission_entries);
        add_counts(
            &mut self.undecided_naval_mission_children,
            undecided_naval_mission_children,
        );
        self.authored_naval_mission_buttons += std::mem::take(authored_naval_mission_buttons);
        self.undefined_naval_mission_sprites
            .append(undefined_naval_mission_sprites);
        self.unanchored_naval_missions += std::mem::take(unanchored_naval_missions);
        add_counts(&mut self.naval_mission_gaps, naval_mission_gaps);
        self.naval_mission_limits.append(naval_mission_limits);
        self.unanchored += std::mem::take(unanchored);
        self.moving += std::mem::take(moving);
        self.missing_flags.append(missing_flags);
        add_counts(&mut self.counter_flag_gaps, counter_flag_gaps);
        self.refused_tokens.append(refused_tokens);
        untouched(&*grouping, "grouping");
        add_counts(&mut self.grouping_gaps, grouping_gaps);
        self.groups += std::mem::take(groups);
        let _ = perspective;
        if let Some(pointed) = pointed.take() {
            self.pointed = Some(pointed);
        }
    }

    pub fn assign(&mut self, source: &Self) {
        self.assign_all_but_flags(source);
        if self.missing_flags != source.missing_flags {
            self.missing_flags.clone_from(&source.missing_flags);
        }
    }

    pub fn refuse_task_forces(
        &mut self,
        reason: &'static str,
        forces: impl IntoIterator<Item = u64>,
    ) {
        self.unplaced_task_forces
            .entry(reason)
            .or_default()
            .extend(forces);
    }

    pub fn assign_all_but_flags(&mut self, source: &Self) {
        let Self {
            unit_hits,
            base_hits,
            radar_hits,
            air_base_hits,
            naval_base_hits,
            air_grid_hits,
            air_mission_hits,
            naval_mission_hits,
            naval_region_hovers,
            regional_naval_unit_hits,
            combat_hits,
            naval_marker_hovers,
            land_hovers,
            task_force_hovers,
            task_force_port_hovers,
            eta,
            land_counters,
            railway_guns,
            unavailable_railway_guns,
            unavailable_railway_gun_locations,
            unplaced_railway_gun_routes,
            unplaced_task_forces,
            unplaced_task_force_routes,
            unavailable_ability_icons,
            naval_counters,
            unavailable_naval_flags,
            unavailable_type_tints,
            type_tint_problem,
            no_intel_counters,
            no_intel_combats,
            air_bases,
            hidden_air_bases,
            naval_bases,
            naval_headquarters,
            facilities,
            rocket_sites,
            gun_emplacements,
            radars,
            strategic_locations,
            unknown_bases,
            unresolved_facility_sprites,
            icon_gui_notes,
            faction_pings,
            faction_pin_hovers,
            pin_gui_refusals,
            faction_ping_limits,
            land_combats,
            combat_gaps,
            combat_limits,
            unknown_sharing,
            naval_combats,
            unavailable_naval_combats,
            base_frame_naval_combats,
            naval_result_icons,
            unavailable_naval_results,
            naval_grid_gaps,
            navy_counter_gaps,
            naval_window_notes,
            land_counter_gaps,
            land_action_gaps,
            naval_result_gaps,
            undeclared_result_sprites,
            naval_mission_entries,
            undecided_naval_mission_children,
            authored_naval_mission_buttons,
            undefined_naval_mission_sprites,
            unanchored_naval_missions,
            naval_mission_gaps,
            naval_mission_limits,
            unanchored,
            moving,
            missing_flags: _,
            counter_flag_gaps,
            refused_tokens,
            grouping,
            grouping_gaps,
            groups,
            perspective,
            pointed,
        } = source;
        self.unit_hits.clone_from(unit_hits);
        self.base_hits.clone_from(base_hits);
        self.radar_hits.clone_from(radar_hits);
        self.air_base_hits.clone_from(air_base_hits);
        self.naval_base_hits.clone_from(naval_base_hits);
        self.air_grid_hits.clone_from(air_grid_hits);
        self.air_mission_hits.clone_from(air_mission_hits);
        self.naval_mission_hits.clone_from(naval_mission_hits);
        self.naval_region_hovers.clone_from(naval_region_hovers);
        self.regional_naval_unit_hits
            .clone_from(regional_naval_unit_hits);
        self.combat_hits.clone_from(combat_hits);
        if self.counter_flag_gaps != *counter_flag_gaps {
            self.counter_flag_gaps.clone_from(counter_flag_gaps);
        }
        self.naval_marker_hovers.clone_from(naval_marker_hovers);
        self.land_hovers.clone_from(land_hovers);
        self.task_force_hovers.clone_from(task_force_hovers);
        self.task_force_port_hovers
            .clone_from(task_force_port_hovers);
        self.faction_pin_hovers.clone_from(faction_pin_hovers);
        if self.eta != *eta {
            self.eta.clone_from(eta);
        }
        self.land_counters = *land_counters;
        self.railway_guns = *railway_guns;
        self.unavailable_railway_guns = *unavailable_railway_guns;
        self.unavailable_railway_gun_locations = *unavailable_railway_gun_locations;
        self.unplaced_railway_gun_routes = *unplaced_railway_gun_routes;
        if self.unplaced_task_forces != *unplaced_task_forces {
            self.unplaced_task_forces.clone_from(unplaced_task_forces);
        }
        if self.unplaced_task_force_routes != *unplaced_task_force_routes {
            self.unplaced_task_force_routes
                .clone_from(unplaced_task_force_routes);
        }
        self.unavailable_ability_icons = *unavailable_ability_icons;
        self.naval_counters = *naval_counters;
        self.unavailable_naval_flags = *unavailable_naval_flags;
        self.unavailable_type_tints = *unavailable_type_tints;
        self.type_tint_problem.clone_from(type_tint_problem);
        self.no_intel_counters = *no_intel_counters;
        self.no_intel_combats = *no_intel_combats;
        self.air_bases = *air_bases;
        self.hidden_air_bases = *hidden_air_bases;
        self.naval_bases = *naval_bases;
        self.naval_headquarters = *naval_headquarters;
        self.facilities = *facilities;
        self.rocket_sites = *rocket_sites;
        self.gun_emplacements = *gun_emplacements;
        self.radars = *radars;
        self.strategic_locations = *strategic_locations;
        if self.unknown_bases != *unknown_bases {
            self.unknown_bases.clone_from(unknown_bases);
        }
        if self.unresolved_facility_sprites != *unresolved_facility_sprites {
            self.unresolved_facility_sprites
                .clone_from(unresolved_facility_sprites);
        }
        self.icon_gui_notes.clone_from(icon_gui_notes);
        self.faction_pings = *faction_pings;
        self.pin_gui_refusals.clone_from(pin_gui_refusals);
        self.faction_ping_limits.assign(faction_ping_limits);
        self.land_combats = *land_combats;
        if self.combat_gaps != *combat_gaps {
            self.combat_gaps.clone_from(combat_gaps);
        }
        if self.combat_limits != *combat_limits {
            self.combat_limits.clone_from(combat_limits);
        }
        if self.unknown_sharing != *unknown_sharing {
            self.unknown_sharing.clone_from(unknown_sharing);
        }
        self.naval_combats = *naval_combats;
        self.unavailable_naval_combats = *unavailable_naval_combats;
        self.base_frame_naval_combats = *base_frame_naval_combats;
        self.naval_result_icons = *naval_result_icons;
        self.unavailable_naval_results = *unavailable_naval_results;
        if self.navy_counter_gaps != *navy_counter_gaps {
            self.navy_counter_gaps.clone_from(navy_counter_gaps);
        }
        if self.naval_window_notes != *naval_window_notes {
            self.naval_window_notes.clone_from(naval_window_notes);
        }
        if self.land_counter_gaps != *land_counter_gaps {
            self.land_counter_gaps.clone_from(land_counter_gaps);
        }
        if self.land_action_gaps != *land_action_gaps {
            self.land_action_gaps.clone_from(land_action_gaps);
        }
        if self.naval_grid_gaps != *naval_grid_gaps {
            self.naval_grid_gaps.clone_from(naval_grid_gaps);
        }
        if self.naval_result_gaps != *naval_result_gaps {
            self.naval_result_gaps.clone_from(naval_result_gaps);
        }
        if self.undeclared_result_sprites != *undeclared_result_sprites {
            self.undeclared_result_sprites
                .clone_from(undeclared_result_sprites);
        }
        self.naval_mission_entries = *naval_mission_entries;
        if self.undecided_naval_mission_children != *undecided_naval_mission_children {
            self.undecided_naval_mission_children
                .clone_from(undecided_naval_mission_children);
        }
        self.authored_naval_mission_buttons = *authored_naval_mission_buttons;
        if self.undefined_naval_mission_sprites != *undefined_naval_mission_sprites {
            self.undefined_naval_mission_sprites
                .clone_from(undefined_naval_mission_sprites);
        }
        self.unanchored_naval_missions = *unanchored_naval_missions;
        if self.naval_mission_gaps != *naval_mission_gaps {
            self.naval_mission_gaps.clone_from(naval_mission_gaps);
        }
        if self.naval_mission_limits != *naval_mission_limits {
            self.naval_mission_limits.clone_from(naval_mission_limits);
        }
        self.unanchored = *unanchored;
        self.moving = *moving;
        if self.refused_tokens != *refused_tokens {
            self.refused_tokens.clone_from(refused_tokens);
        }
        self.grouping = *grouping;
        self.grouping_gaps.clone_from(grouping_gaps);
        self.groups = *groups;
        self.perspective.clone_from(perspective);
        self.pointed = *pointed;
    }
}

pub fn assign_union<'a>(
    target: &mut BTreeSet<String>,
    base: &'a BTreeSet<String>,
    extra: impl Iterator<Item = &'a String> + Clone,
) {
    let same = base
        .iter()
        .chain(extra.clone())
        .all(|tag| target.contains(tag))
        && target
            .iter()
            .all(|tag| base.contains(tag) || extra.clone().any(|other| other == tag));
    if !same {
        target.clone_from(base);
        target.extend(extra.cloned());
    }
}

pub fn visible(height: f32, cutoff: f32) -> bool {
    height < cutoff
}

pub fn zoom_lift(layouts: &ColumnLayout, units_cutoff: f32, height: f32) -> f32 {
    ((layouts.stack_size[1] + layouts.zoomed_in[1])
        * (1.0 - height.min(units_cutoff) / units_cutoff))
        .round()
}

pub fn units_shown(inputs: &CounterInputs<'_>, peace_conference: bool) -> bool {
    unit_counters_shown(
        &inputs.visibility,
        inputs.shows(inputs.content.defines.icons_cutoff),
        peace_conference,
    )
}

pub fn unit_counters_shown(visibility: &Visibility, shown: bool, peace_conference: bool) -> bool {
    visibility.unit_counters && shown && !peace_conference
}

pub fn coarse(inputs: &CounterInputs<'_>) -> bool {
    inputs
        .content
        .defines
        .coarse(inputs.camera.height, inputs.visibility.strategic_grouping)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterPointer {
    pub unit: Unit,
    pub down: bool,
    pub since: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterExit {
    pub unit: Unit,
    pub since: f32,
}

pub struct PlateBinding<'a> {
    pub sprite: Option<&'a str>,
    pub frame: Option<i32>,
    pub tint: [f32; 4],
}

#[expect(
    clippy::too_many_arguments,
    reason = "the plate, pointer state and timing bind to the same native control"
)]
pub fn draw_pointed_plate(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    window: &str,
    origin: [f32; 2],
    plate: PlateBinding<'_>,
    down: Option<bool>,
    since: f32,
    time: f32,
) -> Result<bool, String> {
    let drawn = (|| {
        let root = inputs
            .windows
            .guis
            .template(window)
            .ok_or_else(|| format!("{window} is not declared"))?;
        let element = root
            .find(native::Kind::Button, "count_bg")
            .ok_or_else(|| format!("{window} has no count_bg button"))?;
        let resources = gui_windows::gui_assets(inputs, None);
        let mut bindings = viewer_native_gui::native_gui::Bindings::default();
        bindings.viewport = Some(viewer_native_gui::gui_resources::viewport(
            inputs.viewport,
            inputs.ui_scale,
        ));
        bindings.time = time;
        let bound = bindings.bind(element);
        bound.color = Some(plate.tint);
        bound.frame = plate.frame;
        if let Some(sprite) = plate.sprite {
            bound.sprite = Some(sprite.as_bytes().to_vec());
            bound.setter = viewer_native_gui::native_gui::SpriteSetter::Button;
        }
        let cause = viewer_native_gui::gui_resources::refused_cause;
        let layout = viewer_native_gui::native_gui::pointed_button(
            root, element, &resources, bindings, down, since,
        )
        .map_err(|refusals| {
            refusals
                .iter()
                .map(|refusal| cause(window, refusal))
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        let at = origin.map(|value| value as i32);
        if at.map(|value| value as f32) != origin {
            return Err(format!("{window} is placed between whole interface pixels"));
        }
        viewer_native_gui::native_gui::draw(&layout, at, &resources, batch)
            .map_err(|refusal| cause(window, &refusal))?;
        Ok::<_, String>(layout.parts.iter().any(|part| part.shading.changing()))
    })();
    drawn.map_err(|why| format!("count_bg's pointer state is not drawn: {why}"))
}

pub struct Geography<'a> {
    pub state_of: &'a BTreeMap<u32, u32>,
    pub extents: &'a Extents,
}

pub fn draw_child(
    batch: &mut Batch,
    sheet: &SpriteSheet,
    child: &Child,
    origin: [f32; 2],
    frame: u32,
    color: [f32; 4],
) {
    let size = sheet.frame_size();
    batch.scaled(
        sheet,
        frame,
        [origin[0] + child.at[0], origin[1] + child.at[1]],
        [size[0] * child.scale, size[1] * child.scale],
        color,
    );
}

pub fn bar_placement(
    at: [f32; 2],
    frame: [f32; 2],
    scale: f32,
    fill: f32,
) -> ([f32; 2], [f32; 2], f32) {
    let size = [frame[0] * scale, frame[1] * scale];
    let vertical = frame[1] > frame[0];
    let (offset_x, length) = if vertical {
        (
            counter_inputs::bar_step(fill, size[1].trunc()) as f32,
            size[1],
        )
    } else {
        (0.0, size[0])
    };
    ([at[0] + offset_x, at[1]], size, length)
}

pub fn draw_bar(
    batch: &mut Batch,
    sheet: &SpriteSheet,
    child: &Child,
    origin: [f32; 2],
    fill: f32,
) {
    let frame = sheet.frame_size();
    let (at, size, length) = bar_placement(
        [origin[0] + child.at[0], origin[1] + child.at[1]],
        frame,
        child.scale,
        fill,
    );
    batch.bar(
        sheet,
        at,
        size,
        counter_inputs::bar_pixels(fill, length),
        child.rotation,
        [1.0; 4],
    );
}

pub fn draw_text(
    batch: &mut Batch,
    font: &text::Font,
    child: &TextChild,
    origin: [f32; 2],
    value: &str,
    color: [f32; 4],
) {
    if child.max_width <= 0.0 {
        return;
    }
    let glyphs = text::layout_truncated(
        font,
        value,
        [origin[0] + child.at[0], origin[1] + child.at[1]],
        child.align,
        Some(child.max_width),
        child.truncate,
    );
    if !glyphs.is_empty() {
        batch.text(&glyphs, color);
    }
}

#[derive(Clone, Default)]
pub struct Countries {
    pub index: Option<Arc<CountryIndex>>,
    pub tags: BTreeMap<u32, String>,
    pub flag_tags: BTreeMap<u32, FlagTags>,
    pub sides: BTreeMap<u32, Allegiance>,
    pub groups: BTreeMap<u32, Result<Option<String>, String>>,
    pub ui_colors: UiColors,
}

impl Countries {
    pub fn tags_map(&self) -> &BTreeMap<u32, String> {
        self.index.as_deref().map_or(&self.tags, CountryIndex::tags)
    }

    pub fn flag_tags_map(&self) -> &BTreeMap<u32, FlagTags> {
        self.index
            .as_deref()
            .map_or(&self.flag_tags, CountryIndex::flag_tags)
    }

    pub fn same_flags(&self, other: &Self) -> bool {
        self.tags_map() == other.tags_map()
            && self.flag_tags_map() == other.flag_tags_map()
            && self.groups == other.groups
    }
}

#[derive(Clone, Debug, Default)]
pub struct UiColors {
    pub colors: BTreeMap<u32, Option<[f32; 4]>>,
    pub inheritance: viewer_foundation::color_inheritance::Inheritance,
    pub gap: String,
    pub unavailable: Option<String>,
}

impl UiColors {
    pub fn read(
        map: &MapData,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
    ) -> io::Result<Self> {
        let appearance = rows::<tables::CountryAppearance>(frame)?;
        let gap = match &appearance {
            Available::Complete(_) => "CountryAppearance has no row for it".to_owned(),
            Available::Truncated { missing, .. } => {
                format!("CountryAppearance is truncated ({missing} missing rows)")
            }
            Available::Unavailable { reason } => {
                format!("CountryAppearance is unavailable: {reason}")
            }
            Available::NotRecorded { mode } => {
                format!("CountryAppearance is not recorded in a {mode} recording")
            }
        };
        let colors = appearance
            .rows()
            .iter()
            .filter_map(|row| {
                let slot = *links.get(&row.country)?;
                let rgba =
                    counter_inputs::color(row.ui_red, row.ui_green, row.ui_blue, row.ui_alpha);
                Some((slot, rgba))
            })
            .collect();
        Ok(
            match viewer_foundation::color_inheritance::inheritance(map, frame, links) {
                Ok(inheritance) => Self {
                    colors,
                    inheritance,
                    gap,
                    unavailable: None,
                },
                Err(error) => Self {
                    unavailable: Some(error.to_string()),
                    ..Self::default()
                },
            },
        )
    }
}

impl Countries {
    pub fn ui_color(&self, slot: u32) -> Result<[f32; 4], String> {
        let colors = &self.ui_colors;
        if let Some(reason) = &colors.unavailable {
            return Err(format!(
                "country-colour mode needs the countries' UI colours, which are unknown: {reason}"
            ));
        }
        let source = colors
            .inheritance
            .source(slot)
            .map_err(|error| format!("country-colour mode: {error}"))?;
        match colors.colors.get(&source) {
            Some(Some(rgba)) => Ok(*rgba),
            Some(None) => Err(format!(
                "country-colour mode needs country slot {source}'s UI colour, \
                 but its CountryAppearance row has non-finite colour components"
            )),
            None => Err(format!(
                "country-colour mode needs country slot {source}'s UI colour, \
                 but {}",
                colors.gap
            )),
        }
    }

    pub fn warm_flags(&self, assets: &CounterArt) {
        let keys: BTreeSet<String> = self
            .tags_map()
            .keys()
            .filter_map(|&slot| self.flag_keys(slot))
            .flatten()
            .collect();
        for flags in [&assets.flags, &assets.medium_flags, &assets.large_flags] {
            flags.warm(keys.iter().map(String::as_str));
        }
    }

    pub fn flag_now(&self, flags: &LazyFlags, slot: u32) -> Option<(usize, [f32; 4])> {
        let keys = self.flag_keys(slot)?;
        flags.acquire(&keys.iter().map(String::as_str).collect::<Vec<_>>())
    }

    pub fn known_side(&self, slot: u32) -> Option<Allegiance> {
        self.sides.get(&slot).copied()
    }

    pub fn is_ours(&self, slot: u32) -> bool {
        self.known_side(slot) == Some(Allegiance::Ours)
    }

    pub fn same_allegiance_group(&self, a: u32, b: u32) -> bool {
        a == b
            || matches!(
                (self.known_side(a), self.known_side(b)),
                (Some(first), Some(other)) if first == other
            )
    }

    pub fn flag_keys(&self, slot: u32) -> Option<Vec<String>> {
        let (cosmetic, original) = self.flag_tags_map().get(&slot)?;
        let cosmetic = cosmetic.as_ref().ok()?;
        let group = self.groups.get(&slot)?.as_ref().ok()?;
        let keys = content_reader_country_names::country_names::flag_keys(
            original.as_deref().ok(),
            cosmetic,
            group.as_deref(),
        );
        Some(keys)
    }

    pub fn flag_refusal(&self, slot: u32) -> Option<String> {
        let Some((cosmetic, original)) = self.flag_tags_map().get(&slot) else {
            return Some(format!(
                "country slot {slot} flag needs its CountryAppearance row"
            ));
        };
        if let Err(reason) = cosmetic {
            return Some(format!("country slot {slot}: {reason}"));
        }
        match self.groups.get(&slot) {
            Some(Ok(_)) => {}
            Some(Err(reason)) => return Some(format!("country slot {slot}: {reason}")),
            None => return Some(format!("country slot {slot} flag needs its ruling group")),
        }
        original
            .as_ref()
            .err()
            .map(|reason| format!("country slot {slot}: {reason}"))
    }
}

pub fn unit_flag_owner(
    owner: u32,
    expeditionary_owner: u32,
    coarse: bool,
    resolve: &impl Fn(u32) -> Option<u32>,
) -> u32 {
    if !coarse && (expeditionary_owner as i32) > 0 {
        resolve(expeditionary_owner).unwrap_or(owner)
    } else {
        owner
    }
}

pub const NAVAL_KIND: u32 = 1;

pub const COARSE_PLATE_FRAME: u32 = 2;

pub struct CoarseCounter<'a> {
    pub plate: &'a Child,
    pub large_flag: &'a Child,
    pub count_box: &'a TextChild,
    pub tint: [f32; 4],
    pub owner: u32,
    pub count: &'a str,
    pub keys: (u32, u32, u32),
    pub plate_drawn: bool,
}

pub fn draw_coarse_counter(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    origin: [f32; 2],
    countries: &Countries,
    coarse: CoarseCounter<'_>,
) -> io::Result<()> {
    let CoarseCounter {
        plate,
        large_flag,
        count_box,
        tint,
        owner,
        count,
        keys,
        plate_drawn,
    } = coarse;
    let assets = inputs.assets;
    batch.at_layer(u64::from(keys.0));
    if !plate_drawn && let Some(sheet) = assets.sheet(&plate.sprite) {
        draw_child(batch, sheet, plate, origin, COARSE_PLATE_FRAME, tint);
    }
    batch.at_layer(u64::from(keys.1));
    draw_flag(
        &assets.medium_flags,
        batch,
        large_flag,
        origin,
        countries,
        owner,
    );
    batch.at_layer(u64::from(keys.2));
    if let Some(font) = &inputs.fonts.counter {
        draw_text(batch, font, count_box, origin, count, font.color());
    }
    Ok(())
}

pub fn rank_children(batch: &mut Batch, first: usize, priority: i8, icon: u32) -> io::Result<()> {
    let mut orders: Vec<u64> = batch.quads[first..].iter().map(|quad| quad.layer).collect();
    orders.sort_unstable();
    orders.dedup();
    let slots = sprites::CHILD_SLOTS;
    if orders.len() > slots as usize {
        return Err(invalid(format!(
            "a counter drew {} distinct children, more than the {slots} child slots of the \
             icon layer",
            orders.len()
        )));
    }
    for quad in &mut batch.quads[first..] {
        let rank = orders.binary_search(&quad.layer).unwrap_or(0) as u32;
        quad.layer = layer(priority, icon, rank);
    }
    Ok(())
}

pub fn draw_flag(
    flags: &LazyFlags,
    batch: &mut Batch,
    child: &Child,
    origin: [f32; 2],
    countries: &Countries,
    owner: u32,
) {
    let Some((texture, uv)) = countries.flag_now(flags, owner) else {
        return;
    };
    batch.cell(
        texture,
        [origin[0] + child.at[0], origin[1] + child.at[1]],
        [
            flags.cell_size()[0] as f32 * child.scale,
            flags.cell_size()[1] as f32 * child.scale,
        ],
        uv,
        [1.0; 4],
    );
}

pub fn selection_joins(
    selected: &BTreeSet<Unit>,
    first: Unit,
    candidate: Unit,
    split: bool,
) -> bool {
    let first = selected.contains(&first);
    let candidate = selected.contains(&candidate);
    first == candidate && !(split && first)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinReport {
    pub undecided: Option<String>,
    pub ambiguous: usize,
    pub unplaced: usize,
    pub unresolved_hidden: usize,
}

impl PinReport {
    pub fn assign(&mut self, source: &Self) {
        let Self {
            undecided,
            ambiguous,
            unplaced,
            unresolved_hidden,
        } = source;
        self.undecided.clone_from(undecided);
        self.ambiguous = *ambiguous;
        self.unplaced = *unplaced;
        self.unresolved_hidden = *unresolved_hidden;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PinHover {
    pub rect: [f32; 4],
    pub theatre: (u64, u32),
    pub regions: Arc<[u32]>,
    pub layer: u64,
}

impl PinHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseUnknowns {
    pub naval_bases: BTreeMap<String, usize>,
    pub naval_headquarters: BTreeMap<String, usize>,
    pub facilities: BTreeMap<String, usize>,
}

impl BaseUnknowns {
    pub fn totals(&self) -> [usize; 3] {
        [
            &self.naval_bases,
            &self.naval_headquarters,
            &self.facilities,
        ]
        .map(|reasons| reasons.values().sum())
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        [
            ("naval base", &self.naval_bases),
            ("naval HQ", &self.naval_headquarters),
            ("special-facility", &self.facilities),
        ]
        .into_iter()
        .flat_map(|(kind, reasons)| {
            reasons.iter().map(move |(reason, count)| {
                format!("Counters: {count} {kind} icons not drawn: {reason}.")
            })
        })
    }
}

pub type ColumnIdentity = (Level, u32, u32, u32, u32);

#[derive(Clone, Debug)]
pub struct ColumnPlacement {
    pub hovered: bool,
    pub priority: i8,
    pub anchor: Option<IconAnchor>,
    pub identity: ColumnIdentity,
    pub count: usize,
    pub screen: [i32; 2],
    pub holder: native_gui::Holder,
    pub grid: [i32; 2],
}

#[derive(Debug)]
pub struct AtSeaPlate {
    pub member: usize,
    pub ordinal: u32,
    pub column: Arc<ColumnPlacement>,
    pub local: [i32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct JoinRules {
    pub coarse: bool,
    pub color_mode: CounterColorMode,
    pub nato: bool,
}

impl JoinRules {
    pub fn owners_join(self, countries: &Countries, rep: u32, candidate: u32) -> bool {
        if rep == candidate {
            return true;
        }
        match self.color_mode {
            CounterColorMode::Country => false,
            CounterColorMode::Allegiance => countries.same_allegiance_group(rep, candidate),
        }
    }
}

pub type Movement = counter_move::DivisionRoute;

#[derive(Clone, Default)]
pub struct Movements {
    pub by_division: Arc<BTreeMap<u64, Movement>>,
    pub eta: Report,
    pub by_key: Option<Arc<KeyedRoutes>>,
}

pub struct KeyedRoutes {
    frame: Arc<DecodedFrame>,
    slots: Vec<std::sync::OnceLock<Option<Box<Movement>>>>,
    lookups: Lookups,
}

impl Movements {
    pub fn read_by_key(frame: &Arc<DecodedFrame>, lookups: Lookups) -> io::Result<Self> {
        let count = rows::<tables::DivisionMovement>(frame)?.rows().len();
        rows::<tables::Divisions>(frame)?;
        let mut eta = Report::default();
        eta.refused.insert(Arc::from(ROUTES_BY_KEY), 1);
        Ok(Self {
            by_division: Arc::default(),
            eta,
            by_key: Some(Arc::new(KeyedRoutes {
                frame: Arc::clone(frame),
                slots: (0..count).map(|_| std::sync::OnceLock::new()).collect(),
                lookups,
            })),
        })
    }

    pub fn prepare(
        &self,
        divisions: impl IntoIterator<Item = u64>,
        content: &CounterMap,
        movement_content: Option<&MovementContent>,
    ) {
        let Some(keyed) = &self.by_key else { return };
        let on_frame = !viewer_runtime::redraws::apart_now();
        let frame = &*keyed.frame;
        let Ok(movement) = rows::<tables::DivisionMovement>(frame) else {
            return;
        };
        let movement = movement.rows();
        let census = rows::<tables::Divisions>(frame).map_or(&[][..], |rows| rows.rows());
        let map = movement_content.map(|movement| &movement.map);
        let anchor = |province| content.anchor(province, STANDSTILL);
        let mut terms = None;
        for division in divisions {
            let Ok(at) = movement.binary_search_by_key(&division, |row| row.division) else {
                continue;
            };
            if keyed.slots[at].get().is_some() {
                keyed.lookups.kept();
                continue;
            }
            keyed.lookups.made();
            if on_frame {
                viewer_runtime::view_record::add("overlay", "routes_made_on_frame", 1);
            }
            let row = &movement[at];
            let census_row = census
                .binary_search_by_key(&division, |row| row.id)
                .ok()
                .map(|at| &census[at]);
            let terms = terms
                .get_or_insert_with(|| viewer_foundation::movement_timing::HourTerms::new(frame));
            let eta = viewer_foundation::movement_timing::estimate_one(
                row,
                census_row,
                terms,
                movement_content,
            );
            let route = counter_move::DivisionRoutes::route_of(
                row,
                census_row.map_or(0, |row| row.province),
                eta,
                map,
                &anchor,
                &content.routes,
            );
            let _ = keyed.slots[at].set(Some(Box::new(route)));
        }
        content.routes.sweep(movement.len(), |division| {
            movement
                .binary_search_by_key(division, |row| row.division)
                .is_ok()
        });
    }

    pub fn fill(&self, content: &CounterMap, movement_content: Option<&MovementContent>) {
        let Some(keyed) = &self.by_key else { return };
        let Ok(movement) = rows::<tables::DivisionMovement>(&keyed.frame) else {
            return;
        };
        self.prepare(
            movement.rows().iter().map(|row| row.division),
            content,
            movement_content,
        );
    }

    pub fn by_key(&self) -> bool {
        self.by_key.is_some()
    }
    pub fn read(
        frame: &DecodedFrame,
        etas: &Etas,
        movement: Option<&MovementContent>,
        content: &CounterMap,
    ) -> io::Result<Self> {
        let map = movement.map(|movement| &movement.map);
        let routes = counter_move::DivisionRoutes::read_kept(
            frame,
            etas,
            map,
            &|id| content.anchor(id, STANDSTILL),
            &content.routes,
        )?;
        let mut eta = etas.report().clone();
        eta.routes_unresolved = routes.unresolved;
        eta.route_hops_skipped = routes.skipped_hops;
        Ok(Self {
            by_division: Arc::clone(&routes.by_division),
            eta,
            by_key: None,
        })
    }

    pub fn get(&self, division: u64) -> Option<&Movement> {
        let Some(keyed) = &self.by_key else {
            return self.by_division.get(&division);
        };
        let movement = rows::<tables::DivisionMovement>(&keyed.frame).ok()?.rows();
        let at = movement
            .binary_search_by_key(&division, |row| row.division)
            .ok()?;
        keyed.slots[at].get()?.as_deref()
    }
}

pub fn is_land(inputs: &CounterInputs<'_>, province: u32) -> bool {
    province_is_land(inputs.map, inputs.movement, province)
}

pub fn province_is_land(map: &MapData, movement: Option<&MovementContent>, province: u32) -> bool {
    movement
        .and_then(|movement| movement.map.terrain(province))
        .map_or_else(
            || {
                map.definitions
                    .get(&province)
                    .is_some_and(|definition| matches!(definition.kind, ProvinceKind::Land))
            },
            |terrain| terrain.flags & 1 != 0,
        )
}

pub struct CounterArt {
    pub air_sortie_defines: Result<replay_derive::aircraft_count::SortieDefines, String>,
    pub layouts: Layouts,
    pub windows: SharedNativeWindows,
    pub window_cache: Arc<WindowCache>,
    pub sheets: BTreeMap<String, SpriteSheet>,
    pub land_action: Option<ActionContent>,
    pub gun_action: Option<ActionContent>,
    pub gui: GuiResources,
    pub gui_assets: Assets,
    pub flags: Arc<LazyFlags>,
    pub medium_flags: Arc<LazyFlags>,
    pub large_flags: Arc<LazyFlags>,
    pub types: templates::TypeAtlas,
    pub portraits: counter_portraits::Portraits,
    pub recorded_sheets: BTreeMap<String, SpriteSheet>,
    pub ability_sprites: Arc<content_reader_abilities::abilities::AbilitySprites>,
    pub warnings: Vec<String>,
}
impl CounterArt {
    pub fn plate_sounds(&self, unit: Unit) -> Result<native::ButtonSounds, String> {
        let window = match unit {
            Unit::Division(_) => LAND_WINDOW,
            Unit::RailwayGun(_) => GUN_WINDOW,
            Unit::TaskForce(_) => NAVY_WINDOW,
        };
        let button = self
            .windows
            .guis
            .template(window)
            .ok_or_else(|| format!("{window} is not declared"))?
            .find(native::Kind::Button, "count_bg")
            .and_then(native::Element::button)
            .ok_or_else(|| format!("{window} has no count_bg button"))?;
        let sprite_click = self
            .gui
            .registry()
            .sprite(&button.sprite)
            .map_or(&[][..], |facts| facts.kind.click_sound());
        Ok(button.sounds(sprite_click, &self.windows.guis.default_sounds()))
    }

    pub fn operative_portrait_texture(
        &self,
        name: &[u8],
    ) -> Option<viewer_native_gui::native_gui::Texture> {
        let name = std::str::from_utf8(name).ok()?;
        let counter_portraits::Resolved::Sheet(sheet) = self.portraits.sprite(name) else {
            return None;
        };
        Some(viewer_native_gui::native_gui::Texture {
            texture: sheet.texture,
            uv: [0.0, 0.0, 1.0 / sheet.frames.max(1) as f32, 1.0],
            size: [
                (sheet.size[0] / sheet.frames.max(1)) as i32,
                sheet.size[1] as i32,
            ],
        })
    }

    pub fn portrait(&self, portrait: &counter_order::Portrait) -> Option<SpriteSheet> {
        use counter_order::Portrait;
        use counter_portraits::Resolved;
        fn recorded(assets: &CounterArt, portrait: &Portrait) -> Resolved {
            let unknown = || {
                assets
                    .sheet(counter_order::UNKNOWN_LEADER_PORTRAIT)
                    .cloned()
                    .map_or(Resolved::Unresolved, Resolved::Sheet)
            };
            match portrait {
                Portrait::Sprite(name) | Portrait::File(name) => {
                    let Ok(name) = std::str::from_utf8(name) else {
                        return Resolved::Unresolved;
                    };
                    if name == counter_order::UNKNOWN_LEADER_PORTRAIT {
                        return unknown();
                    }
                    assets
                        .portraits
                        .resolve(&counter_portraits::identity_key(name))
                }
                Portrait::FallbackIfUnresolved(inner)
                    if matches!(**inner, Portrait::Unrecorded(_)) =>
                {
                    Resolved::Unresolved
                }
                Portrait::FallbackIfUnresolved(inner) => match recorded(assets, inner) {
                    Resolved::Unresolved => unknown(),
                    resolved => resolved,
                },
                Portrait::Unrecorded(_) => Resolved::Unresolved,
            }
        }
        match recorded(self, portrait) {
            Resolved::Sheet(sheet) => Some(sheet),
            Resolved::Pending | Resolved::Unresolved => None,
        }
    }

    pub fn sheet(&self, name: &str) -> Option<&SpriteSheet> {
        self.sheets
            .get(name)
            .or_else(|| self.recorded_sheets.get(name))
    }

    pub fn ability_sheet(&self, token: &[u8]) -> Option<&SpriteSheet> {
        self.sheet(self.ability_sprites.sprite(token)?)
    }

    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    pub fn census(&self, batch: &Batch, fonts: &text::Fonts, ui_scale: f32) -> serde_json::Value {
        use serde_json::json;
        use viewer_native_gui::text::GlyphMetrics;

        let mut plates: BTreeMap<u64, Vec<serde_json::Value>> = BTreeMap::new();
        for quad in &batch.quads {
            let ordinal = quad.layer / icon_offset(1);
            let mut sprite = None;
            let mut frame = None;
            let mut flag = None;
            let mut character = None;
            let portrait = self.portraits.sheet_of_texture(quad.texture);
            let gui_sheets = self
                .gui_assets
                .sprites
                .iter()
                .filter_map(|name| self.gui.loaded_sheet(name));
            for sheet in self
                .sheets
                .values()
                .chain(portrait.iter())
                .chain(self.recorded_sheets.values())
                .chain(gui_sheets)
            {
                if sheet.texture == quad.texture {
                    sprite = Some(sheet.name.as_str());
                    frame = Some(
                        (quad.uv[0] * sheet.size[0] as f32 / sheet.frame_size()[0]).round() as u32,
                    );
                    break;
                }
            }
            if quad.texture == self.types.texture {
                for ((name, index), cell) in self.types.entries() {
                    if self.types.uv(*cell) == quad.uv {
                        sprite = Some(name.as_str());
                        frame = Some(*index);
                        break;
                    }
                }
            }
            if self.flags.owns(quad.texture) {
                sprite = Some("GFX_mapicon_unit_flag_stripe");
                flag = self.flags.key_at(quad.texture, quad.uv);
            }
            let window_fonts = self
                .gui_assets
                .fonts
                .iter()
                .filter_map(|name| self.gui.loaded_font(name));
            for font in window_fonts {
                for ch in
                    "0123456789?ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz /-+.%".chars()
                {
                    if let Some(glyph) = font.quad(ch)
                        && glyph.page.texture == quad.texture
                        && glyph.uv == quad.uv
                    {
                        character = Some(ch.to_string());
                        sprite = std::str::from_utf8(&font.facts.name).ok();
                        break;
                    }
                }
            }
            for font in fonts.counter.iter() {
                for ch in
                    "0123456789?ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz /-+.%".chars()
                {
                    if font.page_of(ch) == Some(quad.texture)
                        && font.glyph(ch).is_some_and(|glyph| glyph.uv == quad.uv)
                    {
                        character = Some(ch.to_string());
                        sprite = Some(font.name.as_str());
                        break;
                    }
                }
            }
            plates.entry(ordinal).or_default().push(json!({
                "sprite": sprite, "frame": frame, "flag": flag, "character": character,
                "texture": quad.texture, "rect": quad.rect, "uv": quad.uv,
                "color": quad.color, "rotation": quad.rotation, "layer": quad.layer,
            }));
        }
        let plates: Vec<_> = plates
            .into_iter()
            .filter(|(_, quads)| {
                quads.iter().any(|q| {
                    matches!(
                        q["sprite"].as_str(),
                        Some(
                            "GFX_onmap_unit_counter"
                                | "GFX_onmap_unit_counter_empty"
                                | "GFX_onmap_naval_unit_counter"
                        )
                    )
                })
            })
            .map(|(ordinal, quads)| {
                let origin = quads.first().map(|q| json!([q["rect"][0], q["rect"][1]]));
                let text: String = quads
                    .iter()
                    .filter_map(|q| q["character"].as_str())
                    .collect();
                json!({"ordinal": ordinal, "origin": origin, "text": text, "quads": quads})
            })
            .collect();
        let coordinate_space = if ui_scale == 1.0 {
            "device pixels"
        } else {
            "interface pixels"
        };
        json!({
            "schema": 1, "coordinate_space": coordinate_space, "ui_scale": ui_scale,
            "frames": "zero based", "plates": plates
        })
    }
}
pub struct CounterMap {
    pub naval_mission_allegiance_group_cutoff: Result<f32, String>,
    pub terrain: Arc<TerrainContent>,
    pub wing_types: Result<Arc<content_reader_script::sub_units::SubUnits>, String>,
    pub recorded_labels: Option<Arc<viewer_runtime::recorded_labels::RecordedLabels>>,
    pub port_mission_defines: viewer_unit_reads::port_mission_defines::Defines,
    pub anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    pub stack_scales: BTreeMap<(u32, u32), f32>,
    pub default_anchors: Arc<BTreeMap<u32, [f32; 3]>>,
    pub site_icons: Arc<[BTreeMap<u32, building_facts::SiteIcon>; building_facts::ICON_KINDS]>,
    pub air_base_first_provinces: Arc<building_facts::FirstProvinces>,
    pub ports: BTreeMap<u32, [f32; 3]>,
    pub kinds: Arc<BTreeMap<u32, ProvinceKind>>,
    pub bounds: Arc<BTreeMap<u32, Bounds>>,
    pub routes: viewer_unit_reads::counter_move::RouteMemo,
    pub region_of: Arc<BTreeMap<u32, u32>>,
    pub region_provinces: BTreeMap<u32, Vec<u32>>,
    pub region_icons: BTreeMap<u32, [f32; 3]>,
    pub region_names: BTreeMap<u32, String>,
    pub ground_crew_defines: Result<(i64, i64), String>,
    pub air_log_timeout: Result<i32, String>,
    pub missing_standstill: Vec<u32>,
    pub map: MapSize,
    pub aggression: Arc<Vec<i64>>,
    pub fog: Arc<Result<FogContent, String>>,
    pub defines: CounterDefines,
    pub supply_grace: Result<i32, String>,
    pub comms_max_distance: Result<i32, String>,
    pub dig_in_factor: Result<i64, String>,
    pub attrition: Result<Arc<viewer_unit_reads::counter_attrition::Content>, String>,
    pub ability_comms_scaling: Result<Arc<Vec<i64>>, String>,
    pub ability_no_hq_scaling: Result<i64, String>,
    pub subunit_tokens: Result<TokenKeys, String>,
    pub ideologies: content_reader_key_names::ideologies::Ideologies,
}

impl CounterMap {
    pub fn fog(&self) -> Result<&FogContent, &'static str> {
        FogContent::loaded(&self.fog)
    }

    pub fn air_log_timeout(&self) -> Result<i32, &str> {
        self.air_log_timeout
            .as_ref()
            .map(|&timeout| timeout)
            .map_err(String::as_str)
    }

    pub fn ground_crew_defines(&self) -> Result<(i64, i64), &str> {
        self.ground_crew_defines
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }

    pub fn anchor(&self, province: u32, kind: u32) -> Option<[f32; 3]> {
        stack_anchor(&self.anchors, &self.default_anchors, province, kind)
    }

    pub fn stack_scale(&self, province: u32, kind: u32) -> f32 {
        self.stack_scales
            .get(&(province, kind))
            .copied()
            .unwrap_or(1.0)
    }

    pub fn seed_default_anchors(
        &mut self,
        provinces: impl Iterator<Item = u32>,
        centroid: impl Fn(u32) -> Option<[f32; 2]>,
        heights: &HeightField<'_>,
    ) {
        let mut default_anchors = Arc::unwrap_or_clone(std::mem::take(&mut self.default_anchors));
        let (seeded, missing) = seed_default_anchors(provinces, &self.anchors, centroid, heights);
        default_anchors.extend(seeded);
        self.missing_standstill.extend(missing);
        self.default_anchors = Arc::new(default_anchors);
    }

    pub fn seeded_anchor(&self, province: u32, kind: u32) -> bool {
        self.default_anchors.contains_key(&province) && seeded_only(&self.anchors, province, kind)
    }

    pub fn site_icons(
        &self,
        kind: usize,
    ) -> impl Iterator<Item = (u32, u32, Result<[f32; 3], String>)> + '_ {
        self.site_icons[kind]
            .iter()
            .map(|(&province, icon)| (province, icon.state, self.site_anchor(province, icon)))
    }

    pub fn site_icon(&self, kind: usize, province: u32) -> Option<(u32, Result<[f32; 3], String>)> {
        let icon = self.site_icons[kind].get(&province)?;
        Some((icon.state, self.site_anchor(province, icon)))
    }

    fn site_anchor(
        &self,
        province: u32,
        icon: &building_facts::SiteIcon,
    ) -> Result<[f32; 3], String> {
        match &icon.anchor {
            Ok(building_facts::IconAnchor::Record(at)) => Ok(*at),
            Ok(building_facts::IconAnchor::Province) => self
                .anchor(province, STANDSTILL)
                .ok_or_else(|| "the province has no unit-stack slot 0".to_owned()),
            Err(reason) => Err(reason.clone()),
        }
    }

    pub fn port(&self, province: u32) -> Option<[f32; 3]> {
        self.ports
            .get(&province)
            .copied()
            .or_else(|| self.anchor(province, SHIP_IN_PORT))
    }

    pub fn province_kind(&self, province: u32) -> Option<ProvinceKind> {
        self.kinds.get(&province).copied()
    }

    pub fn region(&self, province: u32) -> Option<u32> {
        self.region_of.get(&province).copied()
    }

    pub fn region_provinces(&self, region: u32) -> &[u32] {
        self.region_provinces
            .get(&region)
            .map_or(&[], Vec::as_slice)
    }

    pub fn region_icon(&self, region: u32) -> Option<[f32; 3]> {
        self.region_icons.get(&region).copied()
    }

    pub fn extents(&self, state_of: &BTreeMap<u32, u32>) -> Extents {
        Extents::build(
            &self.bounds,
            state_of,
            &self.region_of,
            &self.defines.groups,
            self.map.x(),
        )
    }
}

#[derive(Default)]
pub struct CounterFrame<P = (), B = (), A = (), N = ()> {
    pub pings: Batch,
    pub native_pings: P,
    pub bases: Batch,
    pub native_bases: B,
    pub native_air: A,
    pub counters: Batch,
    pub combats: Batch,
    pub native_land_combats: land_windows::PreparedCombats,
    pub naval: N,
    pub report: CounterReport,
}

pub struct HourTables<B, N> {
    pub epoch: Arc<()>,
    pub base: Arc<B>,
    pub land_combats: Arc<LandMarkers>,
    pub naval_markers: Arc<N>,
    pub engaged: Arc<CombatFlags>,
}
impl<B, N> Clone for HourTables<B, N> {
    fn clone(&self) -> Self {
        Self {
            epoch: Arc::clone(&self.epoch),
            base: Arc::clone(&self.base),
            land_combats: Arc::clone(&self.land_combats),
            naval_markers: Arc::clone(&self.naval_markers),
            engaged: Arc::clone(&self.engaged),
        }
    }
}

pub mod plan_probe {
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Copy, Debug, Default)]
    pub struct Counts {
        pub semantic_plans: u64,
        pub semantic_plan_hits: u64,
        pub ordered_groups: u64,
        pub ordered_group_members_equal: u64,
        pub base_plan_hits: u64,
        pub column_plan_hits: u64,
        pub site_icons: u64,
        pub plates_drawn: u64,
        pub pools_presented: u64,
        pub pools_kept: u64,
    }

    static ACTIVE: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        static CURRENT: RefCell<Option<Counts>> = const { RefCell::new(None) };
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn collect<T>(work: impl FnOnce() -> T) -> (T, Counts) {
        struct Restore(Option<Counts>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|current| *current.borrow_mut() = self.0.take());
                ACTIVE.fetch_sub(1, Ordering::Relaxed);
            }
        }
        let previous = CURRENT.with(|current| current.replace(Some(Counts::default())));
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        let _restore = Restore(previous);
        let result = work();
        let counts = CURRENT.with(|current| current.borrow().unwrap_or_default());
        (result, counts)
    }

    #[inline]
    pub fn record(change: impl FnOnce(&mut Counts)) {
        if ACTIVE.load(Ordering::Relaxed) != 0 {
            record_active(change);
        }
    }

    #[cold]
    fn record_active(change: impl FnOnce(&mut Counts)) {
        CURRENT.with(|current| {
            if let Some(counts) = current.borrow_mut().as_mut() {
                change(counts);
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Kinds(u8);

impl Kinds {
    pub const LAND: Self = Self(1);
    pub const LAND_MARKERS: Self = Self(1 << 1);
    pub const NAVAL_MARKERS: Self = Self(1 << 2);
    pub const BASES: Self = Self(1 << 3);
    pub const GUNS: Self = Self(1 << 4);
    pub const AIR_MISSIONS: Self = Self(1 << 5);
    pub const ALL: [Self; 6] = [
        Self::LAND,
        Self::LAND_MARKERS,
        Self::NAVAL_MARKERS,
        Self::BASES,
        Self::GUNS,
        Self::AIR_MISSIONS,
    ];

    pub fn has(self, kind: Self) -> bool {
        self.0 & kind.0 == kind.0 && kind.0 != 0
    }

    #[must_use]
    pub fn with(self, kind: Self) -> Self {
        Self(self.0 | kind.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

pub fn blend_flag_overlay(pixels: &mut [u8], overlay: &[u8]) {
    for (pixel, over) in pixels.chunks_exact_mut(4).zip(overlay.chunks_exact(4)) {
        let factor = f32::from(over[3]) / 255.0;
        for channel in 0..3 {
            let dst = i32::from(pixel[channel]);
            let delta = ((i32::from(over[channel]) - dst) as f32 * factor) as i32;
            pixel[channel] = (dst + delta) as u8;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Lookups(Arc<[std::sync::atomic::AtomicU64; 2]>);

impl Lookups {
    pub fn made(&self) {
        self.0[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn kept(&self) {
        self.0[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn counts(&self) -> (u64, u64) {
        (
            self.0[0].load(std::sync::atomic::Ordering::Relaxed),
            self.0[1].load(std::sync::atomic::Ordering::Relaxed),
        )
    }
}
pub fn unwrap(reference: i32, x: i32, width: i32) -> i32 {
    if width <= 0 {
        return x;
    }
    let (reference, width) = (i64::from(reference), i64::from(width));
    let mut x = i64::from(x);
    while (reference - x).abs() > width / 2 {
        x += if reference - x >= 0 { width } else { -width };
    }
    x as i32
}

pub fn region_icon_position(
    centroids: &[[i32; 2]],
    width: i32,
    height: impl Fn(f32, f32) -> f32,
) -> Option<[f32; 3]> {
    let reference = centroids.first()?[0];
    let (mut x, mut z) = (0.0f32, 0.0f32);
    for &[cx, cz] in centroids {
        x += unwrap(reference, cx, width) as f32;
        z += cz as f32;
    }
    let count = centroids.len() as f32;
    let (mut x, z) = (x / count, z / count);
    let span = width as f32;
    if x < 0.0 {
        x += span;
    } else if x >= span {
        x -= span;
    }
    Some([x, height(x, z), z])
}

pub fn theatre_anchor(positions: &[[f32; 3]]) -> Option<[f32; 3]> {
    if positions.is_empty() {
        return None;
    }
    let inverse = 1.0f32 / positions.len() as f32;
    let mut sum = [0.0f32; 3];
    for position in positions {
        for (total, value) in sum.iter_mut().zip(position) {
            *total += value;
        }
    }
    Some(sum.map(|total| total * inverse))
}

impl CounterReport {
    pub fn naval_mission_warnings(&self) -> Vec<String> {
        let mut warnings: Vec<String> = self
            .naval_mission_limits
            .iter()
            .map(|reason| format!("Naval mission icons are not drawn this hour: {reason}."))
            .collect();
        warnings.extend(
            self.undecided_naval_mission_children
                .iter()
                .map(|(child, count)| format!("{count} x a naval mission entry hides {child}.")),
        );
        warnings.extend(
            self.naval_mission_gaps
                .iter()
                .map(|(reason, count)| format!("{count} x {reason}.")),
        );
        if self.unanchored_naval_missions != 0 {
            warnings.push(format!(
                "{} naval mission icons are not drawn: their strategic region has no icon position.",
                self.unanchored_naval_missions
            ));
        }
        if self.authored_naval_mission_buttons != 0 {
            warnings.push(format!(
                "{} naval mission entries draw their button's authored sprite: the content defines no sprite {}.",
                self.authored_naval_mission_buttons,
                self.undefined_naval_mission_sprites
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        warnings
    }
}

pub const CONVOY_ADDITION_ICON: &str = "GFX_unit_transport_icon_medium_white";

pub const EMBLEM_SUFFIX: &str = "_miniature";

const ROUTES_BY_KEY: &str = "not recorded: the routes and their estimates are made by key for the divisions the layout admits";

pub const STRATEGIC_LOCATION_SPRITE_PREFIX: &str = "GFX_strategic_location_";
