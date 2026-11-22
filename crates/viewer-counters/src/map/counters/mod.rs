
#[cfg(test)]
use viewer_foundation::map::stack_anchors::default_anchor;
use viewer_foundation::map::stack_anchors::{StackAnchors, seed_default_anchors};
use std::cell::RefCell;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_foundation::combat_selection::Hit;
use viewer_foundation::engagement_history::CombatFlags;
use viewer_foundation::engagement_history::History;
use viewer_foundation::engagement_history::LAND_ICON;
use viewer_foundation::engagement_history::NAVAL_ICON;
use viewer_foundation::engagement_history::kept_frames;
use viewer_foundation::map::building_facts::{self, SiteSlots, SlotAdmission};
use viewer_foundation::map::counter_anchors::stack_anchor;
use viewer_foundation::map::counter_hovers::land::{CounterControl, CounterHover};
use viewer_foundation::map::country_index::FlagTags;
use viewer_foundation::map::free_icons::{IconAnchor, IconFamily};
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::icon_windows::PoolView;
use viewer_foundation::map::lazy_icons::flag_set;
use viewer_foundation::map::memo::Recent;
pub use viewer_foundation::map::mode_visibility::{
    Priorities, PriorityIcon, PriorityTable, UnitKinds, Visibility,
};
use viewer_foundation::map::native_gui::Assets;
use viewer_foundation::map::native_gui::Sheets;
use viewer_foundation::map::native_gui::container_frame;
use viewer_foundation::map::native_resources::GuiResources;
use viewer_foundation::map::native_windows::NativeWindows as SharedNativeWindows;
use viewer_foundation::map::native_windows::WindowCache;
use viewer_foundation::map::province_map::HeightField;
use viewer_foundation::map::province_map::ProvinceMap;
use viewer_foundation::map::province_schedule::Shared;
use viewer_foundation::map::sprites::decode_sheet;
use viewer_foundation::map::sprites::icon_offset;
use viewer_foundation::map::terrain_content::TerrainContent;
use viewer_foundation::map::ui_scale::extent as ui_scale_extent;
use viewer_foundation::map::ui_scale::project;
use viewer_foundation::map::visibility::FogContent;
use viewer_foundation::map::visibility::Relations;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::movement_timing::Report;
use viewer_foundation::table_access::TokenKeys;
use viewer_foundation::table_access::rows as table_access_rows;
use viewer_foundation::unit_handle::Unit;

use content_reader_core::{
    ResolvedLayers, buildings::Slot, defines::Defines, flags, flags::FlagSet, gui::native,
    map::MapData, map::ProvinceKind, positions,
};
use content_reader_gfx::gfx::SpriteSet;
use content_reader_image::dds;
use content_reader_regions::regions;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use viewer_foundation::map::icon_windows::admitted;
use {
    crate::map::counter_groups, crate::map::counter_groups::Bounds,
    crate::map::counter_groups::Extents, crate::map::counter_groups::GroupDefines,
    crate::map::counter_groups::Level, crate::map::counter_portraits,
    viewer_foundation::map::allegiance, viewer_foundation::map::allegiance::Allegiance,
    viewer_foundation::map::camera::Camera, viewer_foundation::map::camera::MapSize,
    viewer_foundation::map::counter_action, viewer_foundation::map::counter_inputs,
    viewer_foundation::map::counter_inputs::NativeInputs, viewer_foundation::map::counter_move,
    viewer_foundation::map::counter_order, viewer_foundation::map::counter_status,
    viewer_foundation::map::lazy_icons::LazyFlags,
    viewer_foundation::map::lazy_icons::LazyTextures,
    viewer_foundation::map::perspective::SharingUnknown,
    viewer_foundation::map::perspective::Sight, viewer_foundation::map::perspective::Vision,
    viewer_foundation::map::province_admission::Admission,
    viewer_foundation::map::province_admission::province_boxes, viewer_foundation::map::sprites,
    viewer_foundation::map::sprites::Batch, viewer_foundation::map::sprites::SpriteSheet,
    viewer_foundation::map::sprites::layer, viewer_foundation::map::sprites::load_sheet,
    viewer_foundation::map::tables::Available, viewer_foundation::map::tables::core,
    viewer_foundation::map::tables::rows,
    viewer_foundation::map::task_force_visual::TaskForceMotions, viewer_foundation::map::templates,
    viewer_foundation::map::text,
};

mod air_grid;
mod air_missions;
mod base_windows;
mod layered;

pub fn layers_moved(was: [usize; 6], now: [usize; 6]) -> Vec<&'static str> {
    layered::Layered::moved(was, now)
}
mod preparation;
pub use layered::{GunPlaced, GunRoute, Kinds, Layered, LayeredGuns};
pub use base_windows::View as BaseView;
pub use preparation::Preparation;
mod air_volunteers;
mod bases;
pub use bases::{AirBaseCount, AirBaseFields, BaseFacts, BaseValues, RadarCircle, RadarInputs, RadarOutcome, RadarStates, RegistrationFacts, RegistrationFields, WingCensus, WingFields, WingInputs, base_count, controlled_of, merge_count, radar_notes_of, radar_of, site_bases_of, site_kind, site_states_of, wing_base_of, wing_country, wings_of};
mod shards;
pub use shards::{Sharded, Shards};
pub mod combat_markers;
pub mod faction_pings;
mod geography;
mod gun_members;
pub use gun_members::{Gun, GunFields, GunGround, GunJoins, GunMember, TodayGuns, gun_member, gun_name};
use gun_members::TodayGunJoins;
pub mod gui_windows;
mod initial_flags;
pub use initial_flags::warm_initial_flags;
use viewer_foundation::map::native_windows::NativeWindows;
mod hovers;
mod land;
pub use land::{DivisionStatus, StatusFields};
mod members;
pub use members::{DivisionFields, LandStacks, Member, MemberJoins, Orders, Strength, division_kind, division_member, member_orders, member_strength};
use members::*;
mod layouts;
use layouts::*;
pub mod land_windows;
mod naval;
mod naval_hovers;
mod naval_markers;
pub use naval_markers::{CombatValues, Decision, DetailFacts, FrameSource, HqForces, HqFrames, IconValues, NavalMarkerInputs, NavalMarkerValues, Omitted, ResultFacts, ResultFields, ResultTables, naval_marker_values, result_country_lists};
mod naval_members;
pub use naval_members::{FleetMarker, ForceFields, NavalJoins, NavalMember, NavalOperands, Port, PortSummary, Sea, SeaGround, TodayNaval, naval_member, port_force_unrecorded, port_undecided_reason};
mod naval_mine_badges;
mod naval_mission_inputs;
mod naval_missions;
mod naval_port_hovers;
mod naval_regional;
mod naval_repairs;
pub use replay_derive::{air_mission_counts, air_region_derive};
pub use air_missions::{AirPasses, AirValues, Ground, RegionAlerts, TodayAir, counts_withheld, fragment_withheld, ground_of, ground_withheld, interception_gate, interception_received_of, region_alerts, supplied, supply_refusal};

pub type MissionMemo = Recent<(replay_data::BranchId, replay_data::Hour, u32, bool), MissionIcons>;
pub mod carry;
pub mod naval_windows;
pub mod type_tint;

use bases::*;
#[cfg(test)]
pub(crate) use combat_markers::Gaps;
use combat_markers::*;
use faction_pings::*;
use land::*;
use naval::*;
use naval_markers::*;
use naval_missions::*;

use viewer_foundation::map::native_windows::MAPICONS_GUI;

pub const ZOOMED_IN_OFFSET: &str = "unit_counter_zoomed_in_offset";
pub const LAND_WINDOW: &str = "unit_counter";
use viewer_foundation::map::native_windows::NAVY_WINDOW;
pub const STACK_WINDOW: &str = "unit_mapicon";
use viewer_foundation::map::native_windows::AIR_BASE_WINDOW;
use viewer_foundation::map::native_windows::FACILITY_WINDOW;
use viewer_foundation::map::native_windows::NAVAL_BASE_WINDOW;
use viewer_foundation::map::native_windows::NAVAL_HEADQUARTER_WINDOW;
use viewer_foundation::map::native_windows::{
    GUN_EMPLACEMENT_WINDOW, RADAR_WINDOW, ROCKET_SITE_WINDOW, STRATEGIC_LOCATION_WINDOW,
};
pub const FACILITY_SPRITE_PREFIX: &str = "GFX_mapicon_facility_";
use viewer_foundation::map::native_windows::LAND_COMBAT_WINDOW;
use viewer_foundation::map::native_windows::NAVAL_COMBAT_WINDOW;
use viewer_foundation::map::native_windows::NAVAL_RESULT_WINDOW;
pub const GUN_WINDOW: &str = "unit_counter_railway_gun";
pub const ABILITY_WINDOW: &str = "active_ability_item";

#[cfg(test)]
use viewer_foundation::map::counter_anchors::STACK_KINDS;
use viewer_foundation::map::label_reads::VICTORY_POINT;
use viewer_foundation::map::unit_rules::DEFENDING;
use viewer_foundation::map::unit_rules::SHIP_IN_PORT;
use viewer_foundation::map::unit_rules::STANDSTILL;

pub const MEDIUM_FLAG_OVERLAY: &str = "gfx/interface/small_flag_overlay.tga";

pub const UNKNOWN_COUNT: &str = "?";

use viewer_foundation::map::templates::NO_INTEL_SPRITE;

fn invalid(message: impl Into<String>) -> io::Error {
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
    pub naval_counts: viewer_foundation::map::naval_count_approx::Defines,
    pub supply: counter_status::SupplyThresholds,
    pub tints: allegiance::Tints,
    pub land_combat: LandCombatDefines,
    type_colors: type_tint::Colors,
}

impl CounterDefines {
    pub fn read(defines: &content_reader_core::defines::Defines) -> io::Result<Self> {
        let number = |name| defines.number("NGraphics", name).map(|value| value as f32);
        let fixed = |name| {
            defines
                .fixed5("NGraphics", name)
                .map(viewer_foundation::map::camera::fixed5_camera_distance)
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
            naval_counts: viewer_foundation::map::naval_count_approx::Defines::read(defines)?,
            supply: counter_status::SupplyThresholds::read(defines)?,
            tints: allegiance::Tints::read(defines)?,
            land_combat: LandCombatDefines::read(defines)?,
            type_colors: type_tint::Colors::read(defines)?,
        })
    }

    pub fn coarse(&self, height: f32, strategic: bool) -> bool {
        height > self.coarse_distance[usize::from(strategic)]
    }

    #[cfg(test)]
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
    pub const VANILLA: Self = Self {
        movement_speeds: [12.0; 2],
        icons_cutoff: 900.0,
        units_cutoff: 120.0,
        air_base_cutoff: 900.0,
        naval_base_cutoff: 900.0,
        facility_cutoff: 900.0,
        naval_base_scale: 1.0,
        naval_base_highlighted_scale: 1.2,
        naval_mission_cutoff: 1500.0,
        land_combat_cutoff: 1500.0,
        naval_combat_cutoff: 1500.0,
        radar_cutoff: 1100.0,
        coarse_distance: [350.0, 350.0],
        priorities: Priorities::uniform([
            10, 11, 5, 3, 3, 3, 3, 20, 20, 19, 13, 13, 5, 3, 3, 10, 3, 10,
        ]),
        groups: GroupDefines::VANILLA,
        split_selected_limit: 12,
        naval_counts: viewer_foundation::map::naval_count_approx::Defines::VANILLA,
        supply: counter_status::SupplyThresholds {
            very_low: 20_000,
            low: 35_000,
            show: 50_000,
        },
        tints: allegiance::Tints::VANILLA,
        land_combat: LandCombatDefines::VANILLA,
        type_colors: type_tint::Colors::VANILLA,
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

use viewer_foundation::map::native_windows::TINY_UNIT_COUNTER_WINDOW;

pub const CONVOY_ADDITION_ICON: &str = "GFX_unit_transport_icon_medium_white";

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

fn blend_flag_overlay(pixels: &mut [u8], overlay: &[u8]) {
    for (pixel, over) in pixels.chunks_exact_mut(4).zip(overlay.chunks_exact(4)) {
        let factor = f32::from(over[3]) / 255.0;
        for channel in 0..3 {
            let dst = i32::from(pixel[channel]);
            let delta = ((i32::from(over[channel]) - dst) as f32 * factor) as i32;
            pixel[channel] = (dst + delta) as u8;
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ActionContent {
    sheet: Result<SpriteSheet, Cow<'static, str>>,
    short: BTreeMap<u32, Cow<'static, str>>,
}

impl ActionContent {
    pub(super) fn load(child: &Child, window: &str, sheet: Option<&SpriteSheet>) -> Self {
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

    pub(super) fn frame(
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

pub struct CounterAssets {
    air_sortie_defines: Result<viewer_foundation::map::aircraft_count::SortieDefines, String>,
    pub layouts: Layouts,
    pub windows: SharedNativeWindows,
    pub window_cache: Arc<WindowCache>,
    sheets: BTreeMap<String, SpriteSheet>,
    land_action: Option<ActionContent>,
    gun_action: Option<ActionContent>,
    pub gui: GuiResources,
    gui_assets: Assets,
    pub flags: Arc<LazyFlags>,
    pub medium_flags: Arc<LazyFlags>,
    pub large_flags: Arc<LazyFlags>,
    pub types: templates::TypeAtlas,
    portraits: counter_portraits::Portraits,
    recorded_sheets: BTreeMap<String, SpriteSheet>,
    pub ability_sprites: Arc<content_reader_abilities::abilities::AbilitySprites>,
    pub warnings: Vec<String>,
}

pub fn gui_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &content_reader_localisation::localisation::Localisation,
    tooltip_assets: Assets,
) -> Assets {
    let mut assets = gui_windows::window_assets(windows, registry, localisation);
    assets.extend(naval_missions::mission_assets(
        windows,
        registry,
        localisation,
    ));
    assets.extend(tooltip_assets);
    assets
}

impl CounterAssets {
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

    pub fn load(
        shared: &Arc<ResolvedLayers>,
        (layouts, windows): (Layouts, SharedNativeWindows),
        defines: &Defines,
        (sprites, registry): (&SpriteSet, &native::resources::Registry),
        (language, map_size, decode): (&str, [u32; 2], content_reader_image::native::Decode),
        images: &mut Vec<dds::Texture>,
        lazy: &Arc<LazyTextures>,
    ) -> io::Result<Self> {
        let layers: &ResolvedLayers = shared;
        let sheet_names: Vec<&str> = layouts.sprite_names();
        let decoded: Vec<io::Result<(dds::Texture, u32, [u32; 2])>> = std::thread::scope(|scope| {
            sheet_names
                .iter()
                .map(|name| job_priority::spawn_in(scope, || decode_sheet(layers, sprites, name, decode)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("sheet decoder panicked"))
                .collect()
        });
        let mut sheets = BTreeMap::new();
        let mut warnings: Vec<String> = layouts
            .missing
            .iter()
            .map(|child| format!("{MAPICONS_GUI} declares no {child}; it is not drawn."))
            .chain(
                layouts
                    .unsupported
                    .iter()
                    .map(|reason| format!("{MAPICONS_GUI} {reason}; it is not drawn.")),
            )
            .collect();
        for (name, sheet) in sheet_names.iter().zip(decoded) {
            let (texture, frames, size) = match sheet {
                Ok(sheet) => sheet,
                Err(error) => {
                    warnings.push(format!("{name} is not drawn on map icons: {error}"));
                    continue;
                }
            };
            let id = images.len();
            images.push(texture);
            sheets.insert(
                (*name).to_owned(),
                SpriteSheet {
                    name: (*name).to_owned(),
                    texture: id,
                    frames,
                    size,
                },
            );
        }
        let small_cell = FlagSet::Small.cell(defines)?;
        let medium_cell = FlagSet::Medium.cell(defines)?;
        let overlay_of = |stack: &ResolvedLayers| {
            stack
                .read(MEDIUM_FLAG_OVERLAY)
                .ok()
                .and_then(|bytes| flags::decode(&bytes, medium_cell))
        };
        let overlay = overlay_of(layers).or_else(|| {
            let overlay = overlay_of(&layers.without_overlays()?)?;
            layers.note_overlay_refusal(format!(
                "{MEDIUM_FLAG_OVERLAY} (not a {} x {} Targa)",
                medium_cell[0], medium_cell[1]
            ));
            Some(overlay)
        });
        let overlay_missing = overlay.is_none().then(|| {
            format!(
                "{MEDIUM_FLAG_OVERLAY} is missing or not a {} x {} Targa; medium counter flags are drawn without it.",
                medium_cell[0], medium_cell[1]
            )
        });
        let flags = flag_set(shared, FlagSet::Small, small_cell, |_| {}, lazy)?;
        let medium_flags = flag_set(
            shared,
            FlagSet::Medium,
            medium_cell,
            move |image| {
                if let Some(overlay) = &overlay {
                    blend_flag_overlay(&mut image.pixels, &overlay.pixels);
                }
            },
            lazy,
        )?;
        let large_flags = flag_set(
            shared,
            FlagSet::Normal,
            FlagSet::Normal.cell(defines)?,
            |_| {},
            lazy,
        )?;
        let (types, type_warnings) = templates::TypeAtlas::load(layers, sprites, decode, images)?;
        warnings.extend(type_warnings);
        warnings.extend(overlay_missing);
        let (portraits, portrait_warnings) =
            counter_portraits::load(shared, sprites, decode, lazy)?;
        warnings.extend(portrait_warnings);
        let mut recorded_sheets = BTreeMap::new();
        let format24 = layouts
            .land
            .iter()
            .flat_map(|land| [&land.group_marker, &land.leader_enroute])
            .chain(layouts.gun.iter().map(|gun| &gun.group_marker))
            .map(|child| child.sprite.as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        for name in [
            counter_order::UNKNOWN_LEADER_PORTRAIT,
            counter_order::ROCKET_SITE_ICON,
        ]
        .into_iter()
        .chain(format24)
        {
            if sheets.contains_key(name) || recorded_sheets.contains_key(name) {
                continue;
            }
            let loaded = if name == counter_order::UNKNOWN_LEADER_PORTRAIT {
                counter_portraits::load_sprite(layers, sprites, name, decode, images)
            } else {
                load_sheet(layers, sprites, name, decode, images)
            };
            match loaded {
                Ok(sheet) => {
                    recorded_sheets.insert(name.to_owned(), sheet);
                }
                Err(error) => {
                    warnings.push(format!("{name} is not drawn on recorded counters: {error}"))
                }
            }
        }
        let ability_sprites = if layouts.abilities.is_some() {
            content_reader_abilities::abilities::read(layers).unwrap_or_else(|error| {
                warnings.push(format!(
                    "common/abilities cannot be read ({error}); the counter ability grid is not drawn"
                ));
                Default::default()
            })
        } else {
            Default::default()
        };
        warnings.extend(
            ability_sprites
                .warnings
                .iter()
                .map(|(_, message)| message.clone()),
        );
        for name in ability_sprites.by_token.values().collect::<BTreeSet<_>>() {
            if sheets.contains_key(name) || recorded_sheets.contains_key(name) {
                continue;
            }
            match load_sheet(layers, sprites, name, decode, images) {
                Ok(sheet) => {
                    recorded_sheets.insert(name.to_owned(), sheet);
                }
                Err(error) => warnings.push(format!(
                    "ability sprite {name} is not drawn on counters: {error}"
                )),
            }
        }
        let gui = GuiResources::new(registry.clone(), language.as_bytes(), map_size, decode);
        let land_action = layouts.land.as_ref().ok().map(|layout| {
            ActionContent::load(
                &layout.action,
                LAND_WINDOW,
                sheets.get(&layout.action.sprite),
            )
        });
        let gun_action = layouts.gun.as_ref().ok().map(|layout| {
            ActionContent::load(
                &layout.action,
                GUN_WINDOW,
                sheets.get(&layout.action.sprite),
            )
        });
        Ok(Self {
            layouts,
            air_sortie_defines: viewer_foundation::map::aircraft_count::load_defines(
                defines,
            )
            .map_err(|e| e.to_string()),
            windows,
            sheets,
            land_action,
            gun_action,
            gui,
            gui_assets: Default::default(),
            flags,
            medium_flags,
            large_flags,
            window_cache: Arc::new(WindowCache::new(Arc::clone(shared))),
            types,
            portraits,
            recorded_sheets,
            ability_sprites: Arc::new(ability_sprites),
            warnings,
        })
    }

    pub fn load_gui(
        &mut self,
        layers: &ResolvedLayers,
        localisation: &content_reader_localisation::localisation::Localisation,
        tooltip_assets: Assets,
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<Vec<String>> {
        let assets = gui_assets(
            &self.windows,
            self.gui.registry(),
            localisation,
            tooltip_assets,
        );
        self.gui.load(layers, &assets, images)?;
        self.gui_assets = assets;
        Ok(self
            .gui
            .missing_textures()
            .into_iter()
            .map(|(texture, why)| {
                format!(
                    "{texture} does not load for the map-icon windows ({why}); what shows it \
                     draws nothing, as the game draws nothing."
                )
            })
            .chain(self.gui.uncovered_note("Map-icon windows"))
            .collect())
    }

    pub fn operative_portrait_texture(
        &self,
        name: &[u8],
    ) -> Option<viewer_foundation::map::native_gui::Texture> {
        let name = std::str::from_utf8(name).ok()?;
        let counter_portraits::Resolved::Sheet(sheet) = self.portraits.sprite(name) else {
            return None;
        };
        Some(viewer_foundation::map::native_gui::Texture {
            texture: sheet.texture,
            uv: [0.0, 0.0, 1.0 / sheet.frames.max(1) as f32, 1.0],
            size: [
                (sheet.size[0] / sheet.frames.max(1)) as i32,
                sheet.size[1] as i32,
            ],
        })
    }

    fn portrait(&self, portrait: &counter_order::Portrait) -> Option<SpriteSheet> {
        use counter_order::Portrait;
        use counter_portraits::Resolved;
        fn recorded(assets: &CounterAssets, portrait: &Portrait) -> Resolved {
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

    fn ability_sheet(&self, token: &[u8]) -> Option<&SpriteSheet> {
        self.sheet(self.ability_sprites.sprite(token)?)
    }

    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    pub fn census(&self, batch: &Batch, fonts: &text::Fonts, ui_scale: f32) -> serde_json::Value {
        use serde_json::json;
        use viewer_foundation::map::text::GlyphMetrics;

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
                .filter_map(|name| Sheets::sheet(&self.gui, name));
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
                .filter_map(|name| self.gui.font(name));
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

pub struct CounterContent {
    pub terrain: Arc<TerrainContent>,
    pub wing_types: Result<Arc<content_reader_core::unit_leader::SubUnits>, String>,
    mission_content: Result<naval_mission_inputs::Content, String>,
    pub recorded_labels: Option<Arc<viewer_foundation::naval_labels::RecordedLabels>>,
    pub port_mission_defines: viewer_foundation::map::port_mission_defines::Defines,
    pub anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    stack_scales: BTreeMap<(u32, u32), f32>,
    pub default_anchors: Arc<BTreeMap<u32, [f32; 3]>>,
    pub site_icons: Arc<[BTreeMap<u32, building_facts::SiteIcon>; building_facts::ICON_KINDS]>,
    pub air_base_first_provinces: Arc<building_facts::FirstProvinces>,
    ports: BTreeMap<u32, [f32; 3]>,
    pub building_icons: BuildingIconContent,
    pub kinds: Arc<BTreeMap<u32, ProvinceKind>>,
    pub bounds: Arc<BTreeMap<u32, Bounds>>,
    geography: geography::Memo,
    template_icons: TemplateIconMemo,
    pub routes: viewer_foundation::map::counter_move::RouteMemo,
    pub region_of: Arc<BTreeMap<u32, u32>>,
    region_provinces: BTreeMap<u32, Vec<u32>>,
    region_icons: BTreeMap<u32, [f32; 3]>,
    region_names: BTreeMap<u32, String>,
    ground_crew_defines: Result<(i64, i64), String>,
    air_log_timeout: Result<i32, String>,
    volunteer_air_defines: Result<air_volunteers::Defines, String>,
    pub missing_standstill: Vec<u32>,
    map: MapSize,
    pub aggression: Arc<Vec<i64>>,
    pub fog: Arc<Result<FogContent, String>>,
    pub defines: CounterDefines,
    pub supply_grace: Result<i32, String>,
    pub comms_max_distance: Result<i32, String>,
    pub dig_in_factor: Result<i64, String>,
    pub attrition: Result<Arc<viewer_foundation::map::counter_attrition::Content>, String>,
    pub ability_comms_scaling: Result<Arc<Vec<i64>>, String>,
    pub ability_no_hq_scaling: Result<i64, String>,
    pub subunit_tokens: Result<TokenKeys, String>,
    pub ideologies: content_reader_core::ideologies::Ideologies,
}

impl CounterContent {
    pub fn load(
        layers: &ResolvedLayers,
        artwork: &ResolvedLayers,
        map: &MapData,
        defines: &content_reader_core::defines::Defines,
        regions: &BTreeMap<u32, regions::StrategicRegion>,
    ) -> io::Result<Self> {
        Self::load_with_products(None, layers, artwork, map, defines, regions)
    }

    pub fn load_with_products(
        products: Option<&map_content::loader::Loader>,
        layers: &ResolvedLayers,
        artwork: &ResolvedLayers,
        map: &MapData,
        defines: &content_reader_core::defines::Defines,
        regions: &BTreeMap<u32, regions::StrategicRegion>,
    ) -> io::Result<Self> {
        let stacks = match products {
            Some(products) => products.anchors(layers)?,
            None => StackAnchors::read(layers)?,
        };
        let anchors = stacks.positions;
        let stack_scales = stacks.scales;
        let mut ports = BTreeMap::new();
        let definitions = content_reader_core::buildings::read(layers)?;
        let port_slots = port_slot_names(&definitions);
        let provinces = ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        };
        let building_slots = positions::read_buildings(layers)?;
        let (states, _) = content_reader_states::states::read(layers, defines, &map.definitions)?;
        let roster = artwork.descriptor_roster()?;
        let admit = |spawn: &content_reader_core::buildings::SpawnPoint,
                     slot: &content_reader_core::positions::BuildingPosition,
                     province: Option<u32>| {
            viewer_foundation::map::building_facts::admit_slot(
                spawn,
                slot,
                province,
                map,
                states.definitions(),
                &roster,
            )
        };
        let sites = SiteSlots::build(
            &building_slots,
            &definitions,
            |x, z| provinces.at(x, z),
            |spawn, slot, province| admit(spawn, slot, province) == SlotAdmission::Admitted,
        );
        let site_icons = Arc::new(
            [
                building_facts::AIR_BASE,
                building_facts::ROCKET_SITE,
                building_facts::GUN_EMPLACEMENT,
                building_facts::RADAR,
            ]
            .map(|kind| sites.site_icons(kind).clone()),
        );
        let air_base_first_provinces = Arc::new(sites.first_provinces().clone());
        let building_icons = BuildingIconContent::load(
            &definitions,
            &building_slots,
            &provinces,
            admit,
            BuildingIconContent::offsets(defines)?,
        );
        for building in building_slots {
            if let Some(Slot::Bucket(slot)) = definitions.slot(&building.kind)
                && port_slots.contains(slot.name.as_str())
                && let Some(province) = provinces.at(building.x, building.z)
            {
                ports
                    .entry(province)
                    .or_insert([building.x, building.y, building.z]);
            }
        }
        let mut region_of = BTreeMap::new();
        let mut region_provinces = BTreeMap::new();
        for (&id, region) in regions {
            let mut members: Vec<u32> = Vec::with_capacity(region.provinces.len());
            for &province in &region.provinces {
                region_of.insert(province, id);
                if map.definitions.contains_key(&province) && !members.contains(&province) {
                    members.push(province);
                }
            }
            region_provinces.insert(id, members);
        }
        let (terrain, default_map) = content_reader_core::terrain::read(layers)?;
        let wing_types = content_reader_core::unit_leader::sub_units(layers)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        let attrition = viewer_foundation::map::counter_attrition::Content::read(
            defines,
            &terrain,
            wing_types.clone(),
        )
        .map(Arc::new)
        .map_err(|error| error.to_string());
        let mission_content =
            naval_mission_inputs::Content::read(terrain.clone(), layers, regions, defines)
                .map_err(|error| error.to_string());
        Ok(Self {
            wing_types,
            mission_content,
            recorded_labels: None,
            port_mission_defines: viewer_foundation::map::port_mission_defines::Defines::read(
                defines,
            ),
            terrain: Arc::new(TerrainContent {
                combat_terrain: combat_terrains(&layers.read(&default_map.file("definitions")?)?)?,
                terrain_widths: terrain
                    .categories
                    .into_iter()
                    .map(|category| {
                        (
                            category.name,
                            [category.combat_width, category.combat_support_width],
                        )
                    })
                    .collect(),
            }),
            anchors: Arc::new(anchors),
            stack_scales,
            default_anchors: Arc::new(BTreeMap::new()),
            site_icons,
            air_base_first_provinces,
            ports,
            building_icons,
            kinds: Arc::new(
                map.definitions
                    .iter()
                    .map(|(&id, definition)| (id, definition.kind))
                    .collect(),
            ),
            bounds: Arc::new(province_boxes(&provinces)),
            geography: geography::Memo::default(),
            template_icons: TemplateIconMemo::default(),
            routes: Default::default(),
            region_of: Arc::new(region_of),
            region_provinces,
            region_icons: BTreeMap::new(),
            region_names: regions
                .iter()
                .map(|(&id, region)| (id, region.name_key.clone()))
                .collect(),
            volunteer_air_defines: air_volunteers::Defines::read(defines)
                .map_err(|e| e.to_string()),
            ground_crew_defines: (|| {
                Ok((
                    defines.fixed5("NAir", "AIR_MORE_GROUND_CREWS_COST")?,
                    defines.fixed5("NAir", "AIR_MORE_GROUND_CREWS_BOOST")?,
                ))
            })()
            .map_err(|error: io::Error| error.to_string()),
            air_log_timeout: defines
                .int32("NGame", "AIR_LOG_TIMEOUT_HOURS")
                .map_err(|error| error.to_string()),
            missing_standstill: Vec::new(),
            map: MapSize::of(map),
            aggression: Arc::new(defines.fixed5_array("NNavy", "AGGRESSION_SETTINGS_VALUES")?),
            defines: CounterDefines::read(defines)?,
            supply_grace: defines
                .int32("NMilitary", "SUPPLY_GRACE")
                .map_err(|error| error.to_string()),
            comms_max_distance: defines
                .int32("NMilitary", "COMMS_MAX_DISTANCE")
                .map_err(|error| error.to_string()),
            dig_in_factor: defines
                .fixed5("NMilitary", "DIG_IN_FACTOR")
                .map_err(|error| error.to_string()),
            attrition,
            ability_comms_scaling: defines
                .fixed5_array("NMilitary", "ABILITY_COMMS_SCALING")
                .map(Arc::new)
                .map_err(|error| error.to_string()),
            ability_no_hq_scaling: defines
                .fixed5("NMilitary", "ABILITY_NO_HQ_SCALING")
                .map_err(|error| error.to_string()),
            fog: match products {
                Some(products) => products.fog(layers, map, defines),
                None => Arc::new(
                    FogContent::load(layers, map, defines).map_err(|error| error.to_string()),
                ),
            },
            subunit_tokens: viewer_foundation::map::subunit_files::subunit_keys(layers)
                .map(|keys| TokenKeys::new(&keys))
                .map_err(|error| error.to_string()),
            ideologies: content_reader_core::ideologies::Ideologies::default(),
        })
    }

    pub fn fog(&self) -> Result<&FogContent, &'static str> {
        FogContent::loaded(&self.fog)
    }

    pub fn air_log_timeout(&self) -> Result<i32, &str> {
        self.air_log_timeout.as_ref().map(|&timeout| timeout).map_err(String::as_str)
    }

    pub fn ground_crew_defines(&self) -> Result<(i64, i64), &str> {
        self.ground_crew_defines.as_ref().copied().map_err(String::as_str)
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
        let (seeded, missing) =
            seed_default_anchors(provinces, &self.anchors, centroid, heights);
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

    fn site_anchor(&self, province: u32, icon: &building_facts::SiteIcon) -> Result<[f32; 3], String> {
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

    pub fn place_region_icons(
        &mut self,
        centroid: impl Fn(u32) -> Option<[f32; 2]>,
        width: u32,
        heights: &HeightField<'_>,
    ) -> Vec<u32> {
        let mut refused = Vec::new();
        for (&region, provinces) in &self.region_provinces {
            let centroids: Option<Vec<[i32; 2]>> = provinces
                .iter()
                .map(|&province| centroid(province).map(|[x, z]| [x as i32, z as i32]))
                .collect();
            let position = centroids.and_then(|centroids| {
                faction_pings::region_icon_position(&centroids, width as i32, |x, z| {
                    heights.native_height(x, z)
                })
            });
            match position {
                Some(position) => {
                    self.region_icons.insert(region, position);
                }
                None => refused.push(region),
            }
        }
        refused
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

fn seeded_only(anchors: &BTreeMap<(u32, u32), [f32; 3]>, province: u32, kind: u32) -> bool {
    !anchors.contains_key(&(province, kind))
        && (kind != VICTORY_POINT || !anchors.contains_key(&(province, DEFENDING)))
}

fn combat_terrains(bytes: &[u8]) -> io::Result<BTreeMap<u32, String>> {
    content_reader_core::map::province_terrains(bytes)?
        .into_iter()
        .map(|(id, terrain)| {
            String::from_utf8(terrain)
                .map(|terrain| (id, terrain))
                .map_err(|_| invalid(format!("province {id}'s terrain name is not UTF-8")))
        })
        .collect()
}

fn port_slot_names(definitions: &content_reader_core::buildings::Buildings) -> BTreeSet<&str> {
    definitions
        .types
        .values()
        .filter(|building| building.is_port)
        .flat_map(|building| [building.name.as_str(), building.slot_name()])
        .collect()
}

pub struct CounterInputs<'a> {
    pub content: &'a CounterContent,
    pub equipment: Result<&'a content_reader_core::equipment::Database, &'a str>,
    pub assets: &'a CounterAssets,
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
    pub movement_history: &'a viewer_foundation::map::counter_move::SharedMovement,
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

fn counter_plate_color(
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
    fn screen(&self, world: [f32; 3]) -> Option<[f32; 2]> {
        project(&self.camera, world, self.viewport, self.ui_scale)
    }

    fn navy_size(&self) -> Result<[f32; 2], String> {
        container_frame(
            self.windows.navy(),
            Some(gui_resources_viewport(self.viewport, self.ui_scale)),
        )
        .map(|frame| frame.size.map(|value| value as f32))
    }

    fn extent(&self) -> [f32; 2] {
        ui_scale_extent(self.viewport, self.ui_scale)
    }

    fn selected_join(&self, first: Unit, candidate: Unit, height: f32) -> bool {
        let split = self.selected.len() as i32 <= self.content.defines.split_selected_limit
            && height < self.content.defines.units_cutoff;
        crate::map::unit_selection::joins(self.selected, first, candidate, split)
    }

    pub(crate) fn priority(&self, icon: PriorityIcon) -> i8 {
        self.content
            .defines
            .priorities
            .get(self.visibility.priorities, icon)
    }

    pub(super) fn icon_height(&self, anchor: [f32; 3]) -> f32 {
        self.camera.icon_distance(anchor)
    }

    pub(super) fn shows(&self, cutoff: f32) -> bool {
        self.passes(|height| visible(height, cutoff))
    }

    pub(super) fn passes(&self, gate: impl Fn(f32) -> bool) -> bool {
        self.camera.free().is_some() || gate(self.camera.height)
    }

    pub(super) fn icon_anchor(
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
        viewer_foundation::map::gui_resources::contains(self.rect, point)
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
        viewer_foundation::map::gui_resources::contains(self.rect, point)
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
    pub hit: Option<viewer_foundation::map::native_gui::HitGeometry>,
    pub supported: bool,
    pub text: Option<Arc<viewer_foundation::tooltip_text::Tooltip>>,
}
impl NavalRegionalHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.hit.as_ref().map_or_else(
            || viewer_foundation::map::gui_resources::contains(self.rect, point),
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
    pub unit_hits: Vec<viewer_foundation::map::counter_hovers::unit_hits::Hit>,
    pub base_hits: Vec<viewer_foundation::map::counter_hovers::unit_hits::Blocker>,
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
    pub task_force_hovers: Vec<viewer_foundation::map::counter_hovers::task_force::Hover>,
    pub task_force_port_hovers: Vec<viewer_foundation::map::counter_hovers::task_force_port::Hover>,
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
    pub faction_pin_hovers: Vec<faction_pings::PinHover>,
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
    fn base_ordinal(&self) -> u32 {
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

    fn combat_ordinal(&self) -> u32 {
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

    pub(super) fn sharing_unknown(&mut self, what: &'static str, cause: SharingUnknown) {
        *self.unknown_sharing.entry((what, cause)).or_default() += 1;
    }

    fn absorb_later(&mut self, later: &mut Self) {
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

    pub(super) fn refuse_task_forces(
        &mut self,
        reason: &'static str,
        forces: impl IntoIterator<Item = u64>,
    ) {
        self.unplaced_task_forces
            .entry(reason)
            .or_default()
            .extend(forces);
    }

    fn assign_all_but_flags(&mut self, source: &Self) {
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

fn assign_union<'a>(
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

#[derive(Default)]
pub struct CounterFrame {
    pub pings: Batch,
    native_pings: PreparedPins,
    pub bases: Batch,
    native_bases: base_windows::PreparedBases,
    native_air: Option<Arc<air_missions::Prepared>>,
    pub counters: Batch,
    pub combats: Batch,
    native_land_combats: land_windows::PreparedCombats,
    naval: naval_windows::Plan,
    pub report: CounterReport,
}

pub fn visible(height: f32, cutoff: f32) -> bool {
    height < cutoff
}

pub fn zoom_lift(layouts: &ColumnLayout, units_cutoff: f32, height: f32) -> f32 {
    ((layouts.stack_size[1] + layouts.zoomed_in[1])
        * (1.0 - height.min(units_cutoff) / units_cutoff))
        .round()
}

fn units_shown(inputs: &CounterInputs<'_>, peace_conference: bool) -> bool {
    unit_counters_shown(
        &inputs.visibility,
        inputs.shows(inputs.content.defines.icons_cutoff),
        peace_conference,
    )
}

pub(super) fn unit_counters_shown(
    visibility: &Visibility,
    shown: bool,
    peace_conference: bool,
) -> bool {
    visibility.unit_counters && shown && !peace_conference
}

fn coarse(inputs: &CounterInputs<'_>) -> bool {
    inputs
        .content
        .defines
        .coarse(inputs.camera.height, inputs.visibility.strategic_grouping)
}

#[derive(Clone)]
pub struct HourTables {
    epoch: Arc<()>,
    base: Arc<HourBase>,
    land_combats: Arc<LandMarkers>,
    naval_markers: Arc<NavalMarkerTables>,
    engaged: Arc<CombatFlags>,
}

pub struct HourBase {
    frame: Arc<DecodedFrame>,
    engagement_content: EngagementContent,
    relations: Result<Relations, &'static str>,
    links: BTreeMap<u32, u32>,
    vision: Arc<Vision>,
    countries: Arc<Countries>,
    country_ids: BTreeMap<u32, u32>,
    geography: Arc<geography::Geometry>,
    native_relations: Arc<allegiance::NativeRelations>,
    peace_conference: bool,
    order: Option<counter_order::RecordedOrder>,
    native: NativeInputs,
    orders: counter_inputs::OrdersIndex,
    icons: Arc<TemplateIcons>,
    status: DivisionStatuses,
    movement: Movements,
    actions: counter_action::Actions,
    pings: HourPins,
    guns: GunInputs,
    abilities: AbilityInputs,
    hq_providers: BTreeSet<u64>,
    portraits: BTreeMap<u64, counter_order::Portrait>,
    icon_inputs: land::IconInputs,
    task_forces: TaskForceMotions,
    divisions: DivisionCounters,
    base_icons: BaseIcons,
    bases: BaseTables,
    naval: NavalTables,
    ports: PortTables,
    ideologies: BTreeMap<u32, Result<Option<i32>, String>>,
    air_missions: AirMissions,
    missions: Arc<MissionIcons>,
    type_tints: Arc<type_tint::Index>,
    left_out: Kinds,
    shown: Shown,
    lookups: Lookups,
}

impl std::ops::Deref for HourTables {
    type Target = HourBase;

    fn deref(&self) -> &HourBase {
        &self.base
    }
}

#[cfg(test)]
impl std::ops::DerefMut for HourTables {
    fn deref_mut(&mut self) -> &mut HourBase {
        Arc::get_mut(&mut self.base).expect("synthetic base must be uniquely held")
    }
}

impl HourBase {
    fn land(&self) -> CounterTables<'_> {
        CounterTables {
            type_tints: &self.type_tints,
            countries: &self.countries,
            icons: &self.icons,
            status: &self.status,
            movement: &self.movement,
            actions: &self.actions,
            vision: &self.vision,
            order: self.order.as_ref(),
            native: &self.native,
            orders: &self.orders,
            guns: &self.guns,
            abilities: &self.abilities,
            hq_providers: &self.hq_providers,
            portraits: &self.portraits,
            icon_inputs: &self.icon_inputs,
            task_forces: &self.task_forces,
        }
    }
}

struct EngagementContent {
    land: LandCombatDefines,
    kinds: Arc<BTreeMap<u32, ProvinceKind>>,
    anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    defaults: Arc<BTreeMap<u32, [f32; 3]>>,
}

impl EngagementContent {
    fn new(content: &CounterContent) -> Self {
        Self {
            land: content.defines.land_combat,
            kinds: Arc::clone(&content.kinds),
            anchors: Arc::clone(&content.anchors),
            defaults: Arc::clone(&content.default_anchors),
        }
    }
}

fn engagement_tables(
    frame: &DecodedFrame,
    vision: &Arc<Vision>,
    links: &BTreeMap<u32, u32>,
    relations: Result<&Relations, &'static str>,
    movement: &Movements,
    content: &EngagementContent,
    index: Option<&History>,
    left_out: Kinds,
    shown: Shown,
) -> io::Result<(Arc<LandMarkers>, Arc<NavalMarkerTables>, Arc<CombatFlags>)> {
    let engaged = CombatFlags::read(frame, vision.ours(), links, relations, index)?;
    let kept =
        |kind, flags| kept_frames(frame, kind, vision.ours(), flags, index).map(|(kept, _)| kept);
    let land_combats = if left_out.has(Kinds::LAND_MARKERS) || !shown.land_combats {
        Arc::default()
    } else {
        let land_kept = kept(LAND_ICON, &engaged.land)?;
        Arc::new(LandMarkers::read(
            frame,
            vision,
            links,
            &(engaged.land.clone(), engaged.land_provenance),
            &land_kept,
            movement,
            &content.land,
        )?)
    };
    let naval_markers = if left_out.has(Kinds::NAVAL_MARKERS) {
        Arc::new(NavalMarkerTables::empty())
    } else {
        let naval_kept = kept(NAVAL_ICON, &engaged.naval)?;
        Arc::new(NavalMarkerTables::read(
            frame,
            Viewer { vision, owner: &|raw| links.get(&raw).copied() },
            &engaged,
            &naval_kept,
            &|province| content.kinds.get(&province).copied(),
            &|province| stack_anchor(&content.anchors, &content.defaults, province, STANDSTILL),
        )?)
    };
    Ok((land_combats, naval_markers, Arc::new(engaged)))
}

impl HourTables {
    pub fn warm_flags(&self, assets: &CounterAssets) {
        self.countries.warm_flags(assets);
    }

    fn land(&self) -> CounterTables<'_> {
        self.base.land()
    }

    pub fn is_for_frame(&self, frame: &DecodedFrame) -> bool {
        self.frame.branch == frame.branch
            && self.frame.hour == frame.hour
            && self.frame.metrics == frame.metrics
            && self.frame.tables.iter().zip(&frame.tables).all(|(a, b)| {
                replay_frame::frame::SharedView::ptr_eq(a, b) && a.sampled_at() == b.sampled_at()
            })
    }

    pub fn refine_engagement(&self, index: Option<&History>) -> io::Result<Self> {
        let (land_combats, naval_markers, engaged) = engagement_tables(
            &self.frame,
            &self.vision,
            &self.links,
            self.relations.as_ref().map_err(|reason| *reason),
            &self.movement,
            &self.engagement_content,
            index,
            self.left_out,
            self.shown,
        )?;
        Ok(Self {
            epoch: Arc::new(()),
            base: Arc::clone(&self.base),
            land_combats,
            naval_markers,
            engaged,
        })
    }

    pub fn naval_dominance_sources(
        &self,
    ) -> Arc<viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap> {
        self.missions
            .naval_dominance_sources
            .clone()
            .unwrap_or_else(|| {
                Arc::new(Err("naval dominance source preparation unavailable".into()))
            })
    }
    pub fn vision(&self) -> &Arc<Vision> {
        &self.vision
    }

    pub fn left_out(&self) -> Kinds {
        self.left_out
    }

    pub fn lookups(&self) -> (u64, u64) {
        self.base.lookups.counts()
    }

    pub fn routes_by_key(&self) -> bool {
        self.base.movement.by_key()
    }

    pub fn fill_routes(&self, content: &CounterContent, movement: Option<&MovementContent>) {
        self.base.movement.fill(content, movement);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn land_combat_gaps(&self) -> (&Gaps, &Gaps) {
        (&self.land_combats.gaps, &self.land_combats.limits)
    }
}

fn peace_conference_hides_units(countries: &[replay_data::rows::CountryRow], ours: u32) -> bool {
    countries
        .iter()
        .any(|country| country.canonical_slot == ours && country.peace_conference_participant != 0)
}

#[allow(clippy::too_many_arguments)]
pub fn hour_tables(
    frame: &DecodedFrame,
    map: &MapData,
    vision: Arc<Vision>,
    content: &CounterContent,
    assets: &CounterAssets,
    equipment: Result<&content_reader_core::equipment::Database, &str>,
    localisation: &content_reader_localisation::localisation::Localisation,
    etas: &Etas,
    movement_content: Option<&MovementContent>,
    engaged: Option<&History>,
    air_passes: Option<&dyn AirPasses>,
    type_tints: &type_tint::Memo,
    mission_memo: &MissionMemo,
    reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &content_reader_core::province_graph::ProvinceGraph,
            &content_reader_core::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
) -> io::Result<HourTables> {
    hour_tables_profiled(
        frame,
        map,
        vision,
        content,
        assets,
        equipment,
        localisation,
        etas,
        movement_content,
        engaged,
        air_passes,
        type_tints,
        mission_memo,
        reads,
        mission_topology,
        Kinds::default(),
        Need::WHOLE,
        &Carry::default(),
        None,
        &mut viewer_foundation::profiling::BuilderSpans::new(false),
    )
}

#[derive(Default)]
pub struct Carry {
    countries: carry::Carried<CountryInputs, u32>,
    pins: carry::Carried<HourPins, u32>,
    abilities: carry::Carried<AbilityInputs, ()>,
    native_relations: carry::Carried<allegiance::NativeRelations, ()>,
}

struct CountryInputs {
    links: BTreeMap<u32, u32>,
    country_ids: BTreeMap<u32, u32>,
    relations: Result<Relations, &'static str>,
    countries: Arc<Countries>,
    peace_conference: bool,
}

fn country_inputs(frame: &DecodedFrame, map: &MapData, ours: u32) -> io::Result<CountryInputs> {
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let country_source = core::<tables::Countries>(frame)?;
    let country_rows: BTreeMap<_, _> = country_source
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect();
    let tags: BTreeMap<u32, String> = country_source
        .iter()
        .map(|country| (country.canonical_slot, String::from_utf8_lossy(&country.tag).into_owned()))
        .collect();
    let country_ids = country_source
        .iter()
        .map(|country| (country.canonical_slot, country.id))
        .collect();
    let appearances: BTreeMap<_, _> = rows::<tables::CountryAppearance>(frame)?
        .rows()
        .iter()
        .map(|row| (row.country, row))
        .collect();
    let relations = Relations::read(frame, &links);
    let countries = Countries {
        tags,
        flag_tags: initial_flags::flag_tags(&country_rows, &appearances),
        sides: match &relations {
            Ok(relations) => allegiance::counter_table(relations, &links, ours),
            Err(_) => BTreeMap::from([(ours, Allegiance::Ours)]),
        },
        groups: ideology_groups(frame, &country_rows)?,
        ui_colors: UiColors::read(map, frame, &links)?,
    };
    let peace_conference = peace_conference_hides_units(country_source, ours);
    Ok(CountryInputs { links, country_ids, relations, countries: Arc::new(countries), peace_conference })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub air_missions: bool,
    pub by_key: bool,
    pub shown: Shown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub units: bool,
    pub bases: bool,
    pub land_combats: bool,
    pub naval_missions: bool,
}

impl Shown {
    pub const EVERY: Self = Self { units: true, bases: true, land_combats: true, naval_missions: true };

    pub fn at(height: Option<f32>, target: Option<f32>, visibility: &Visibility, defines: &CounterDefines) -> Self {
        let Some(height) = height else { return Self::EVERY };
        let shows = |cutoff: f32| visible(height, cutoff) || target.is_some_and(|target| visible(target, cutoff));
        Self {
            units: visibility.unit_counters && shows(defines.icons_cutoff),
            bases: visibility.province_icons
                && (shows(defines.air_base_cutoff) || shows(defines.naval_base_cutoff) || shows(defines.radar_cutoff) || shows(defines.facility_cutoff)),
            land_combats: visibility.province_icons && shows(defines.land_combat_cutoff),
            naval_missions: shows(defines.naval_mission_cutoff),
        }
    }
}

#[derive(Clone)]
enum AirMissions {
    Made(Arc<air_missions::Prepared>),
    Deferred(Arc<DeferredAir>),
}

struct DeferredAir {
    frame: Arc<DecodedFrame>,
    vision: Arc<Vision>,
    passes: Option<Arc<dyn AirPasses + Send + Sync>>,
    made: std::sync::OnceLock<Result<Arc<air_missions::Prepared>, String>>,
}

impl AirMissions {
    fn get(&self, inputs: &CounterInputs<'_>) -> io::Result<Arc<air_missions::Prepared>> {
        let deferred = match self {
            Self::Made(made) => return Ok(Arc::clone(made)),
            Self::Deferred(deferred) => deferred,
        };
        if let Some(made) = deferred.made.get() {
            return made.clone().map_err(io::Error::other);
        }
        let hour = air_missions::HourInputs {
            content: inputs.content,
            assets: inputs.assets,
            localisation: inputs.localisation,
            passes: deferred.passes.as_deref().map(|passes| passes as &dyn AirPasses),
        };
        let made = air_missions::Prepared::read(&hour, &deferred.frame, &deferred.vision).map(Arc::new).map_err(|error| error.to_string());
        let _ = deferred.made.set(made.clone());
        made.map_err(io::Error::other)
    }

    fn made(&self) -> Option<Arc<air_missions::Prepared>> {
        match self {
            Self::Made(made) => Some(Arc::clone(made)),
            Self::Deferred(deferred) => deferred.made.get().and_then(|made| made.clone().ok()),
        }
    }
}

impl Need {
    pub const WHOLE: Self = Self { air_missions: true, by_key: false, shown: Shown::EVERY };

    pub fn routes_by_key(&self, left_out: Kinds) -> bool {
        self.by_key && left_out.has(Kinds::LAND) && left_out.has(Kinds::LAND_MARKERS)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Lookups(Arc<[std::sync::atomic::AtomicU64; 2]>);

impl Lookups {
    pub(super) fn made(&self) {
        self.0[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(super) fn kept(&self) {
        self.0[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn counts(&self) -> (u64, u64) {
        (self.0[0].load(std::sync::atomic::Ordering::Relaxed), self.0[1].load(std::sync::atomic::Ordering::Relaxed))
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments, reason = "the derivation's inputs, each from another owner")]
pub fn naval_dominance_sources(
    frame: &DecodedFrame,
    map: &MapData,
    content: &CounterContent,
    ours: u32,
    region: u32,
    reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &content_reader_core::province_graph::ProvinceGraph,
            &content_reader_core::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
) -> viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap {
    if frame.complete::<tables::NavalMissionLedgers>().is_ok_and(|rows| rows.is_empty()) {
        return Ok(BTreeMap::new());
    }
    let CountryInputs { links, country_ids, .. } = country_inputs(frame, map, ours).map_err(|error| error.to_string())?;
    let native = NativeInputs::read(frame).map_err(|error| error.to_string())?;
    let forces = table_access_rows::<tables::TaskForces>(frame)
        .map_err(|error| error.to_string())?
        .rows()
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let world = dashboard_text::triggers::World::new(frame, reads.trigger_content.facts()).map_err(|error| error.to_string());
    let cells = viewer_foundation::map::movement_reads::Cells::default();
    let moves = world
        .as_ref()
        .map(|world| viewer_foundation::map::movement_reads::Moves::new(world, reads, &cells))
        .map_err(Clone::clone);
    naval_mission_inputs::Hour::read(frame, content, &native, (&links, &country_ids), forces, (mission_topology, moves), ours)?.dominance_sources_in(Some(region))
}

pub fn hour_tables_profiled(
    frame: &DecodedFrame,
    map: &MapData,
    vision: Arc<Vision>,
    content: &CounterContent,
    assets: &CounterAssets,
    equipment: Result<&content_reader_core::equipment::Database, &str>,
    localisation: &content_reader_localisation::localisation::Localisation,
    etas: &Etas,
    movement_content: Option<&MovementContent>,
    engaged: Option<&History>,
    air_passes: Option<&dyn AirPasses>,
    type_tints: &type_tint::Memo,
    mission_memo: &MissionMemo,
    reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &content_reader_core::province_graph::ProvinceGraph,
            &content_reader_core::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
    left_out: Kinds,
    need: Need,
    carry: &Carry,
    deferred_passes: Option<Arc<dyn AirPasses + Send + Sync>>,
    spans: &mut viewer_foundation::profiling::BuilderSpans,
) -> io::Result<HourTables> {
    let carry = need.by_key.then_some(carry);
    let ours = vision.ours();
    let divisions = !left_out.has(Kinds::LAND) && need.shown.units;
    let shared = Arc::new(frame.clone());
    let lookups = Lookups::default();
    let CountryInputs { links, country_ids, relations, countries, peace_conference } =
        spans.measure("overlay.tables.counters.country_inputs", |_| match carry {
            Some(carry) => carry.countries.get_or_build(frame, &[], ours, || country_inputs(frame, map, ours)).map(|inputs| CountryInputs {
                links: inputs.links.clone(),
                country_ids: inputs.country_ids.clone(),
                relations: inputs.relations.clone(),
                countries: Arc::clone(&inputs.countries),
                peace_conference: inputs.peace_conference,
            }),
            None => country_inputs(frame, map, ours),
        })?;
    let geography = spans.measure("overlay.tables.counters.geography", |_| {
        geography::read(frame, content)
    })?;
    let (pings, abilities, guns, movement, actions) =
        spans.measure("overlay.tables.counters.movement_actions", |spans| {
            let pings = spans.measure("overlay.tables.counters.movement_actions.pins", |_| match carry {
                Some(carry) => carry.pins.get_or_build(frame, &[replay_data::rows::TableId::CountryLinks], ours, || HourPins::read(frame, &links, ours, content)).map(|pins| (*pins).clone()),
                None => HourPins::read(frame, &links, ours, content),
            })?;
            let abilities = spans.measure("overlay.tables.counters.movement_actions.abilities", |_| match carry {
                _ if !need.shown.units => Ok(AbilityInputs::default()),
                Some(carry) => carry.abilities.get_or_build(frame, &[replay_data::rows::TableId::CountryLinks], (), || AbilityInputs::read(frame, &links)).map(|abilities| (*abilities).clone()),
                None => AbilityInputs::read(frame, &links),
            })?;
            let hostility = spans.measure("overlay.tables.counters.movement_actions.hostility", |_| {
                counter_action::DestinationHostility::read(frame, &links)
            })?;
            let guns = spans.measure("overlay.tables.counters.movement_actions.guns", |_| {
                if left_out.has(Kinds::GUNS) || !need.shown.units {
                    GunInputs::hostility_only(frame, &hostility)
                } else {
                    GunInputs::read(frame, &hostility, content.subunit_tokens.as_ref().map_err(String::as_str))
                }
            })?;
            let movement = spans.measure("overlay.tables.counters.movement_actions.routes", |_| {
                if need.routes_by_key(left_out) || !(need.shown.units || need.shown.land_combats) {
                    Movements::read_by_key(&shared, lookups.clone())
                } else {
                    Movements::read(frame, etas, movement_content, content)
                }
            })?;
            let actions = if divisions {
                spans.measure("overlay.tables.counters.movement_actions.actions", |_| {
                    counter_action::Actions::read(frame, &hostility, Some(vision.ours()))
                })?
            } else {
                counter_action::Actions::default()
            };
            Ok::<_, io::Error>((pings, abilities, guns, movement, actions))
        })?;
    let (engagement_content, land_combats, naval_markers, engaged) =
        spans.measure("overlay.tables.counters.engagement", |_| {
            let engagement_content = EngagementContent::new(content);
            let (land_combats, naval_markers, engaged) = engagement_tables(
                frame,
                &vision,
                &links,
                relations.as_ref().map_err(|reason| *reason),
                &movement,
                &engagement_content,
                engaged,
                left_out,
                need.shown,
            )?;
            Ok::<_, io::Error>((engagement_content, land_combats, naval_markers, engaged))
        })?;
    let (native, order, base_icons) =
        spans.measure("overlay.tables.counters.native_base_icons", |spans| {
            let native = spans
                .measure("overlay.tables.counters.native_base_icons.native", |_| {
                    NativeInputs::read(frame)
                })?;
            let order = spans.measure("overlay.tables.counters.native_base_icons.order", |_| {
                counter_order::RecordedOrder::read(frame)
            })?;
            let base_icons = if !need.shown.bases {
                BaseIcons::default()
            } else {
                spans.measure("overlay.tables.counters.native_base_icons.base", |spans| {
                    BaseIcons::read_profiled(
                        frame,
                        &BaseContext {
                            content,
                            vision: &vision,
                            links: &links,
                            country_ids: &country_ids,
                            relations: relations.as_ref().map_err(|reason| *reason),
                            order: order.as_ref(),
                            carry_tokens: need.by_key,
                        },
                        spans,
                    )
                })?
            };
            Ok::<_, io::Error>((native, order, base_icons))
        })?;
    let icons_shown = need.shown.naval_missions;
    let missions = spans.measure("overlay.tables.counters.missions", |spans| {
        mission_memo.get_or_insert_without_waiting((frame.branch, frame.hour, vision.ours(), icons_shown), || {
            spans.measure("overlay.tables.counters.missions.build", |spans| {
                let (forces, mut icons) =
                    spans.measure("overlay.tables.counters.missions.build.read", |_| {
                        let forces = table_access_rows::<tables::TaskForces>(frame)?
                            .rows()
                            .iter()
                            .map(|row| (row.id, row))
                            .collect();
                        let icons = if icons_shown {
                            MissionIcons::read(
                                frame,
                                &vision,
                                &links,
                                &country_ids,
                                &native,
                                &forces,
                            )?
                        } else {
                            MissionIcons::default()
                        };
                        Ok::<_, io::Error>((forces, icons))
                    })?;
                icons.naval_dominance_sources = Some(Arc::new(Ok(BTreeMap::new())));
                if frame
                    .complete::<tables::NavalMissionLedgers>()
                    .map_or(true, |rows| !rows.is_empty())
                {
                    let world =
                        spans.measure("overlay.tables.counters.missions.build.world", |_| {
                            dashboard_text::triggers::World::new(
                                frame,
                                reads.trigger_content.facts(),
                            )
                            .map_err(|error| error.to_string())
                        });
                    let cells = viewer_foundation::map::movement_reads::Cells::default();
                    let moves = world
                        .as_ref()
                        .map(|world| {
                            viewer_foundation::map::movement_reads::Moves::new(world, reads, &cells)
                        })
                        .map_err(Clone::clone);
                    naval_mission_inputs::decorate_profiled(&mut icons, spans, || {
                        naval_mission_inputs::Hour::read(
                            frame,
                            content,
                            &native,
                            (&links, &country_ids),
                            forces,
                            (mission_topology, moves),
                            vision.ours(),
                        )
                    });
                }
                Ok::<_, io::Error>(Arc::new(icons))
            })
        })
    })?;
    let mut tables = spans.measure("overlay.tables.counters.remaining_hour_base", |spans| {
        let hq_providers = if need.shown.units {
            rows::<tables::OrdersGroups>(frame)?
                .rows()
                .iter()
                .filter(|group| group.leader != 0 && group.hq_provider != 0)
                .map(|group| group.hq_provider)
                .collect()
        } else {
            BTreeSet::new()
        };
        let type_tints = spans.measure("overlay.tables.counters.remaining_hour_base.type_tints", |_| {
            type_tint::Index::read(type_tints, frame, &links, vision.ours())
        });
        let tables = HourBase {
            frame: Arc::clone(&shared),
            engagement_content,
            divisions: BTreeMap::new(),
            ports: PortTables::default(),
            naval: spans.measure("overlay.tables.counters.remaining_hour_base.naval", |_| {
                NavalTables::read(frame, &vision)
            })?,
            ideologies: spans.measure("overlay.tables.counters.remaining_hour_base.ideologies", |_| {
                Ok::<_, io::Error>(
                    rows::<tables::TaskForces>(frame)?
                        .rows()
                        .iter()
                        .filter_map(|force| links.get(&force.logical_owner).copied())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .map(|slot| {
                            (
                                slot,
                                naval::ideology_frame(frame, &content.ideologies, &country_ids, slot),
                            )
                        })
                        .collect(),
                )
            })?,
            bases: spans.measure("overlay.tables.counters.remaining_hour_base.bases", |_| {
                match (need.shown.bases, left_out.has(Kinds::BASES)) {
                    (false, _) => Ok(BaseTables::none()),
                    (true, true) => BaseTables::rest(frame, &vision, content),
                    (true, false) => BaseTables::read(frame, &vision, content),
                }
            })?,
            air_missions: spans.measure(
                "overlay.tables.counters.remaining_hour_base.air_missions",
                |_| {
                    if left_out.has(Kinds::AIR_MISSIONS) {
                        return Ok(AirMissions::Made(Arc::default()));
                    }
                    if !need.air_missions {
                        return Ok(AirMissions::Deferred(Arc::new(DeferredAir {
                            frame: Arc::clone(&shared),
                            vision: Arc::clone(&vision),
                            passes: deferred_passes.clone(),
                            made: std::sync::OnceLock::new(),
                        })));
                    }
                    air_missions::Prepared::read(
                        &air_missions::HourInputs {
                            content,
                            assets,
                            localisation,
                            passes: air_passes,
                        },
                        frame,
                        &vision,
                    )
                    .map(|made| AirMissions::Made(Arc::new(made)))
                },
            )?,
            base_icons,
            missions,
            portraits: spans.measure(
                "overlay.tables.counters.remaining_hour_base.portraits",
                |_| if need.shown.units { hq_portraits(frame, &native) } else { Ok(BTreeMap::new()) },
            )?,
            hq_providers,
            icon_inputs: spans.measure("overlay.tables.counters.remaining_hour_base.icon_inputs", |_| {
                match (need.shown.units, need.by_key) {
                    (false, _) => Ok(land::IconInputs::default()),
                    (true, true) => land::IconInputs::read_by_key(&shared, lookups.clone()),
                    (true, false) => land::IconInputs::read(frame),
                }
            })?,
            native_relations: spans.measure(
                "overlay.tables.counters.remaining_hour_base.native_relations",
                |_| match carry {
                    Some(carry) => carry.native_relations.get_or_build(frame, &[replay_data::rows::TableId::CountryLinks], (), || allegiance::NativeRelations::read(frame, &links)),
                    None => allegiance::NativeRelations::read(frame, &links).map(Arc::new),
                },
            )?,
            links,
            vision,
            countries,
            country_ids,
            geography,
            peace_conference,
            order,
            native,
            orders: spans.measure("overlay.tables.counters.remaining_hour_base.orders", |_| {
                if need.shown.units { counter_inputs::OrdersIndex::read(frame) } else { Ok(counter_inputs::OrdersIndex::default()) }
            })?,
            icons: spans.measure("overlay.tables.counters.remaining_hour_base.template_icons", |_| {
                if need.shown.units { content.template_icons.read(frame) } else { Ok(Arc::default()) }
            })?,
            status: spans.measure("overlay.tables.counters.remaining_hour_base.status", |_| {
                match (need.shown.units, need.by_key) {
                    (false, _) => Ok(DivisionStatuses::default()),
                    (true, true) => DivisionStatuses::read_by_key(&shared, lookups.clone()),
                    (true, false) => DivisionStatuses::read(frame),
                }
            })?,
            movement,
            actions,
            relations,
            pings,
            guns,
            abilities,
            task_forces: spans.measure("overlay.tables.counters.remaining_hour_base.task_forces", |_| {
                TaskForceMotions::read(frame)
            })?,
            type_tints,
            left_out,
            shown: need.shown,
            lookups: lookups.clone(),
        };
        Ok::<_, io::Error>(tables)
    })?;
    if divisions {
        tables.divisions = spans.measure("overlay.tables.counters.divisions", |_| {
            division_counters(
                frame,
                &|raw| tables.links.get(&raw).copied(),
                &tables.land(),
            )
        })?;
    }
    if need.shown.bases {
    tables.ports = spans.measure("overlay.tables.counters.ports", |_| {
        let ideology = |slot: u32| tables.ideologies[&slot].clone();
        let sources = NavalSources {
            type_tints: &tables.type_tints,
            equipment,
            vision: &tables.vision,
            native: &tables.native,
            country_ids: &tables.country_ids,
            aggression: &content.aggression,
            ideology: &ideology,
        };
        PortTables::read(
            frame,
            content,
            map,
            movement_content,
            &tables.land(),
            &NavalContext::cached(frame, sources, &tables.naval)?,
            &|raw| tables.links.get(&raw).copied(),
        )
    })?;
    }
    Ok(HourTables {
        epoch: Arc::new(()),
        base: Arc::new(tables),
        land_combats,
        naval_markers,
        engaged,
    })
}

#[cfg(test)]
pub fn prepare_cached(inputs: &CounterInputs<'_>, tables: &HourTables) -> io::Result<PreparedCounters> {
    prepare_with(inputs, tables, None)
}

pub fn prepare_retained(
    inputs: &CounterInputs<'_>,
    tables: &HourTables,
    resources: u64,
    retained: &mut Preparation,
) -> io::Result<PreparedCounters> {
    retained.bind(inputs, tables, resources);
    prepare_with(inputs, tables, Some(retained))
}

fn prepare_with(
    inputs: &CounterInputs<'_>,
    tables: &HourTables,
    mut retained: Option<&mut Preparation>,
) -> io::Result<PreparedCounters> {
    let frame: &DecodedFrame = &tables.frame;
    let mut out = CounterFrame::default();
    let owner = |raw: u32| tables.links.get(&raw).copied();
    let layered = retained.as_deref().map(|retained| retained.layered().clone()).unwrap_or_default();

    out.report.grouping = inputs
        .camera
        .free()
        .is_none()
        .then(|| inputs.content.defines.groups.grouping(inputs.camera.height));
    out.report.type_tint_problem = tables.type_tints.problem().to_owned();
    let grouping_gaps = RefCell::new(BTreeMap::<String, usize>::new());
    let friendly =
        |first: u32, other: u32| match tables.native_relations.co_belligerent(other, first) {
            Ok(friendly) => friendly,
            Err(why) => {
                *grouping_gaps.borrow_mut().entry(why).or_default() += 1;
                false
            }
        };
    let geography = Geography {
        state_of: &tables.geography.state_of,
        extents: &tables.geography.extents,
    };

    let land = tables.land();

    faction_pings(inputs, &tables.pings, &mut out);
    let span = viewer_foundation::profiling::span("pc_bases");
    match retained.as_deref_mut() {
        Some(retained) => retained.bases(inputs, frame, tables, &mut out)?,
        None => bases(inputs, frame, tables, None, &mut out)?,
    }
    drop(span);
    let span = viewer_foundation::profiling::span("pc_air");
    if inputs.visibility.priorities == PriorityTable::StrategicAir {
        out.native_air = Some(match &layered.air {
            Some(values) => {
                let hour = air_missions::HourInputs { content: inputs.content, assets: inputs.assets, localisation: inputs.localisation, passes: None };
                let make = || air_missions::Prepared::of_values(values, &hour, frame, tables.vision.ours());
                match retained.as_deref_mut() {
                    Some(retained) => retained.air(values, make),
                    None => Arc::new(make()),
                }
            }
            None => tables.air_missions.get(inputs)?,
        });
    }
    drop(span);
    let ideology = |slot: u32| tables.ideologies[&slot].clone();
    let sources = NavalSources {
        type_tints: &tables.type_tints,
        equipment: inputs.equipment,
        vision: &tables.vision,
        native: &tables.native,
        country_ids: &tables.country_ids,
        aggression: &inputs.content.aggression,
        ideology: &ideology,
    };
    let span = viewer_foundation::profiling::span("pc_naval_context");
    let context = NavalContext::cached(frame, sources, &tables.naval)?;
    let naval = units_shown(inputs, tables.peace_conference).then_some(&context);
    drop(span);
    if let Some(naval) = &naval {
        out.naval.forces = Some(naval_windows::Forces {
            recorded: naval.forces.iter().map(|force| force.id).collect(),
            complete: naval.forces_complete,
        });
    }
    out.naval.countries = Some(Arc::clone(&tables.countries));
    let span = viewer_foundation::profiling::span("pc_land");
    let layered_divisions = layered.land.as_deref().map(land::division_counters_of);
    let prepared_land = match &naval {
        Some(naval) => Some(land_counters(
            inputs,
            layered_divisions.as_ref().unwrap_or(&tables.divisions),
            layered.land.as_deref(),
            layered.guns.as_deref(),
            &owner,
            &land,
            naval,
            &geography,
            &friendly,
            retained
                .as_deref_mut()
                .map(|retained| &mut retained.columns),
            &mut out,
        )?),
        None => None,
    };
    drop(span);
    if inputs.visibility.priorities == PriorityTable::StrategicAir
        && let (Some(land), Some(naval)) = (&prepared_land, &naval)
    {
        let census = air_grid::Census::new(frame);
        for flotilla in &land.flotillas {
            if flotilla.owner == tables.vision.ours() {
                out.naval.carrier_sea.insert(
                    flotilla.source,
                    census.as_ref().map_err(Clone::clone).and_then(|census| {
                        census.carrier(
                            inputs,
                            naval.ship_inputs.as_ref(),
                            &flotilla.members,
                            tables.vision.ours(),
                        )
                    }),
                );
            }
        }
    }
    out.report.grouping_gaps = grouping_gaps.into_inner();
    let span = viewer_foundation::profiling::span("pc_ports");
    if let Some(retained) = retained {
        retained.ports(
            inputs,
            frame,
            &owner,
            &land,
            naval,
            &tables.base_icons.naval,
            &tables.ports,
            &mut out,
        )?;
    } else {
        port_counters(
            inputs,
            frame,
            &owner,
            &land,
            naval,
            &tables.base_icons.naval,
            &tables.ports,
            &mut out,
        )?;
    }
    drop(span);
    land_combats(inputs, layered.land_markers.as_deref().unwrap_or(&tables.land_combats), &mut out)?;
    let layered_naval = layered.naval_markers.as_deref().map(|values| NavalMarkerTables::of_values(values, frame)).transpose()?;
    naval_markers(
        inputs,
        frame,
        &MarkerContext {
            vision: &tables.vision,
            owner: &owner,
            naval: Ok(&context),
            countries: &tables.countries,
            flags: &tables.engaged,
        },
        layered_naval.as_ref().unwrap_or(&tables.naval_markers),
        &mut out,
    )?;
    naval_missions(inputs, &tables.missions, &mut out)?;
    naval_regional::prepare(
        inputs,
        frame,
        &tables.vision,
        &owner,
        Ok(&context),
        &tables.countries,
        &mut out,
    )?;
    out.report.perspective = Arc::clone(&tables.vision);
    out.report.eta = tables.movement.eta.clone();
    Ok(PreparedCounters {
        fixed: out,
        land: prepared_land,
        drawn: DrawnCounters::default(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterPointer {
    pub(crate) unit: Unit,
    pub(crate) down: bool,
    pub(crate) since: f32,
}

fn hold_pointer(
    pointer: &mut Option<(Option<(Unit, bool)>, f32)>,
    left: &mut Vec<(Unit, f32)>,
    inputs: &CounterInputs<'_>,
    time: f32,
) -> (Option<CounterPointer>, Vec<CounterExit>) {
    let held = inputs
        .pressed
        .map(|unit| (unit, true))
        .or(inputs.plate_hovered.map(|unit| (unit, false)));
    if pointer.is_none_or(|(last, _)| last != held) {
        if let Some(Some((last, _))) = pointer.map(|(last, _)| last)
            && held.is_none_or(|(unit, _)| unit != last)
        {
            left.retain(|(gone, _)| *gone != last);
            left.push((last, time));
        }
        *pointer = Some((held, time));
    }
    if let Some((unit, _)) = held {
        left.retain(|(gone, _)| *gone != unit);
    }
    let at = pointer.map_or(time, |(_, at)| at);
    let current = held.map(|(unit, down)| CounterPointer {
        unit,
        down,
        since: time - at,
    });
    let exits = left
        .iter()
        .map(|&(unit, at)| CounterExit {
            unit,
            since: time - at,
        })
        .collect();
    (current, exits)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CounterExit {
    pub(crate) unit: Unit,
    pub(crate) since: f32,
}

pub(crate) struct PlateBinding<'a> {
    pub(crate) sprite: Option<&'a str>,
    pub(crate) frame: Option<i32>,
    pub(crate) tint: [f32; 4],
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_pointed_plate(
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
        let mut bindings = viewer_foundation::map::native_gui::Bindings::default();
        bindings.viewport = Some(viewer_foundation::map::gui_resources::viewport(
            inputs.viewport,
            inputs.ui_scale,
        ));
        bindings.time = time;
        let bound = bindings.bind(element);
        bound.color = Some(plate.tint);
        bound.frame = plate.frame;
        if let Some(sprite) = plate.sprite {
            bound.sprite = Some(sprite.as_bytes().to_vec());
            bound.setter = viewer_foundation::map::native_gui::SpriteSetter::Button;
        }
        let cause = viewer_foundation::map::gui_resources::refused_cause;
        let layout = viewer_foundation::map::native_gui::pointed_button(
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
        viewer_foundation::map::native_gui::draw(&layout, at, &resources, batch)
            .map_err(|refusal| cause(window, &refusal))?;
        Ok::<_, String>(layout.parts.iter().any(|part| part.shading.changing()))
    })();
    drawn.map_err(|why| format!("count_bg's pointer state is not drawn: {why}"))
}

pub struct PreparedCounters {
    pub fixed: CounterFrame,
    land: Option<PreparedLand>,
    drawn: DrawnCounters,
}

#[derive(Default)]
struct DrawnCounters {
    batch: Batch,
    combats: Batch,
    report: CounterReport,
    missing_owners: Vec<u32>,
    land_report: CounterReport,
    at_sea: Vec<land::AtSeaPlate>,
    columns: Vec<Arc<land::ColumnPlacement>>,
}

impl PreparedCounters {
    pub fn flag_source<'a>(
        &'a self,
        assets: &'a CounterAssets,
    ) -> Option<gui_windows::WindowFlags<'a>> {
        self.fixed
            .naval
            .countries
            .as_deref()
            .map(|countries| gui_windows::WindowFlags {
                countries,
                small: &assets.flags,
                large: &assets.large_flags,
            })
    }

    pub fn detached_flag_source<'a>(
        &self,
        assets: &'a CounterAssets,
    ) -> Option<impl viewer_foundation::map::native_flags::FlagSource + Send + Sync + 'a + use<'a>>
    {
        use viewer_foundation::map::native_flags::{FlagRecord, FlagSource};
        struct Flags<'a> {
            countries: Arc<Countries>,
            assets: &'a CounterAssets,
        }
        impl Flags<'_> {
            fn borrowed(&self) -> gui_windows::WindowFlags<'_> {
                gui_windows::WindowFlags {
                    countries: &self.countries,
                    small: &self.assets.flags,
                    large: &self.assets.large_flags,
                }
            }
        }
        impl FlagSource for Flags<'_> {
            fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String> {
                self.borrowed().tag_slot(tag)
            }
            fn first_tag_slot(&self) -> Result<u32, String> {
                self.borrowed().first_tag_slot()
            }
            fn atlas_len(&self, small: bool) -> Result<i32, String> {
                self.borrowed().atlas_len(small)
            }
            fn atlas_record(&self, small: bool, index: i32) -> Result<FlagRecord, String> {
                self.borrowed().atlas_record(small, index)
            }
        }
        self.fixed.naval.countries.as_ref().map(|countries| Flags {
            countries: Arc::clone(countries),
            assets,
        })
    }

    #[allow(clippy::too_many_arguments, reason = "separate mutable state for the host's composed icon views")]
    pub fn draw(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<()> {
        let Self { fixed, land, drawn } = self;
        let (counter_pointer, counter_exit) =
            hold_pointer(&mut naval.pointer_clock, &mut naval.pointer_left, inputs, time);
        let DrawnCounters {
            batch,
            combats,
            report,
            missing_owners,
            ..
        } = drawn;
        fixed.pings.clear();
        fixed.bases.clear();
        fixed.combats.clear();
        batch.clear();
        combats.clear();
        report.assign_all_but_flags(&fixed.report);
        missing_owners.clear();
        let span = viewer_foundation::profiling::span("dc_pings");
        fixed.native_pings.draw(
            inputs,
            pings,
            time,
            fixed.naval.countries.as_deref(),
            &mut fixed.pings,
            report,
        );
        drop(span);
        let span = viewer_foundation::profiling::span("dc_bases");
        fixed.native_bases.draw(
            inputs,
            bases,
            time,
            fixed.naval.countries.as_ref(),
            &mut fixed.bases,
            report,
        );
        drop(span);
        let mut at_sea = Vec::new();
        let span = viewer_foundation::profiling::span("dc_land_combats");
        fixed.native_land_combats.draw(
            inputs,
            land_combats,
            time,
            fixed.naval.countries.as_deref(),
            &mut fixed.combats,
            report,
        );
        drop(span);
        let span = viewer_foundation::profiling::span("dc_land");
        let columns = match land.as_mut() {
            Some(land) => {
                land.pointer = counter_pointer;
                land.exits = counter_exit;
                land.time = time;
                land.draw(inputs, batch, report, missing_owners, &mut at_sea)?
            }
            None => Vec::new(),
        };
        drop(span);
        {
            let _span = viewer_foundation::profiling::span("dc_naval");
            let plates: Vec<_> = at_sea
                .iter()
                .filter_map(|plate| {
                    let land = land.as_ref()?;
                    Some((
                        &land.flotillas[plate.member],
                        land.countries.as_ref(),
                        plate,
                    ))
                })
                .collect();
            naval_windows::present(
                inputs,
                &fixed.naval,
                &plates,
                &columns,
                time,
                counter_pointer,
                naval,
                &mut fixed.bases,
                batch,
                combats,
                report,
            )?;
            if land
                .as_ref()
                .is_some_and(|land| land.pointer_running.replace(false))
            {
                naval.keep_active();
            }
            let running = land
                .as_ref()
                .map(|land| land.exits_running.take())
                .unwrap_or_default();
            naval
                .pointer_left
                .retain(|(unit, _)| running.contains(unit));
        }
        batch.quads.extend_from_slice(&fixed.counters.quads);
        let span = viewer_foundation::profiling::span("dc_air");
        let empty_air = air_missions::Prepared::default();
        fixed.native_air.as_deref().unwrap_or(&empty_air).draw(
            inputs,
            time,
            &mut naval.air_missions,
            combats,
            report,
        );
        drop(span);
        missing_owners.sort_unstable();
        missing_owners.dedup();
        let tags = land.as_ref().map(|land| &land.countries.tags);
        assign_union(
            &mut report.missing_flags,
            &fixed.report.missing_flags,
            missing_owners
                .iter()
                .filter_map(|owner| tags.and_then(|tags| tags.get(owner))),
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments, reason = "separate mutable state for the host's composed icon views")]
    pub fn draw_branched(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        owners: viewer_foundation::profiling::FrameOwners,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<Option<viewer_foundation::profiling::OwnerSample>> {
        let Self { fixed, land, drawn } = self;
        let (counter_pointer, counter_exit) =
            hold_pointer(&mut naval.pointer_clock, &mut naval.pointer_left, inputs, time);
        if let Some(land) = land.as_mut() {
            land.pointer = counter_pointer;
            land.exits = counter_exit;
            land.time = time;
        }
        let DrawnCounters {
            batch,
            combats,
            report,
            missing_owners,
            land_report,
            at_sea: drawn_at_sea,
            columns: drawn_columns,
        } = drawn;
        fixed.pings.clear();
        fixed.bases.clear();
        fixed.combats.clear();
        batch.clear();
        combats.clear();
        report.assign_all_but_flags(&fixed.report);
        missing_owners.clear();
        let mut at_sea = Vec::new();
        let countries = fixed.naval.countries.as_deref();
        let held_countries = fixed.naval.countries.as_ref();
        let mut others = {
            let native_pings = &mut fixed.native_pings;
            let ping_batch = &mut fixed.pings;
            let native_bases = &mut fixed.native_bases;
            let base_batch = &mut fixed.bases;
            let native_land_combats = &mut fixed.native_land_combats;
            let combat_batch = &mut fixed.combats;
            let report = &mut *report;
            move || {
                owners.measure("counters_branch", || {
                    let span = viewer_foundation::profiling::span("dc_pings");
                    native_pings.draw(inputs, pings, time, countries, ping_batch, report);
                    drop(span);
                    let span = viewer_foundation::profiling::span("dc_bases");
                    native_bases.draw(inputs, bases, time, held_countries, base_batch, report);
                    drop(span);
                    let _span = viewer_foundation::profiling::span("dc_land_combats");
                    native_land_combats.draw(
                        inputs,
                        land_combats,
                        time,
                        countries,
                        combat_batch,
                        report,
                    );
                })
            }
        };
        let mut land_counters = {
            let land = &mut *land;
            let batch = &mut *batch;
            let land_report = &mut *land_report;
            let missing_owners = &mut *missing_owners;
            let at_sea = &mut at_sea;
            move || {
                let _span = viewer_foundation::profiling::span("dc_land");
                match land.as_mut() {
                    Some(land) => land.draw(inputs, batch, land_report, missing_owners, at_sea),
                    None => Ok(Vec::new()),
                }
            }
        };
        let ((_, sample), columns) = (others(), land_counters());
        report.absorb_later(land_report);
        let columns = columns?;
        drawn_columns.clone_from(&columns);
        {
            let _span = viewer_foundation::profiling::span("dc_naval");
            let plates: Vec<_> = at_sea
                .iter()
                .filter_map(|plate| {
                    let land = land.as_ref()?;
                    Some((
                        &land.flotillas[plate.member],
                        land.countries.as_ref(),
                        plate,
                    ))
                })
                .collect();
            naval_windows::present(
                inputs,
                &fixed.naval,
                &plates,
                &columns,
                time,
                counter_pointer,
                naval,
                &mut fixed.bases,
                batch,
                combats,
                report,
            )?;
            if land
                .as_ref()
                .is_some_and(|land| land.pointer_running.replace(false))
            {
                naval.keep_active();
            }
            let running = land
                .as_ref()
                .map(|land| land.exits_running.take())
                .unwrap_or_default();
            naval
                .pointer_left
                .retain(|(unit, _)| running.contains(unit));
        }
        *drawn_at_sea = at_sea;
        batch.quads.extend_from_slice(&fixed.counters.quads);
        let span = viewer_foundation::profiling::span("dc_air");
        let empty_air = air_missions::Prepared::default();
        fixed.native_air.as_deref().unwrap_or(&empty_air).draw(
            inputs,
            time,
            &mut naval.air_missions,
            combats,
            report,
        );
        drop(span);
        missing_owners.sort_unstable();
        missing_owners.dedup();
        let tags = land.as_ref().map(|land| &land.countries.tags);
        assign_union(
            &mut report.missing_flags,
            &fixed.report.missing_flags,
            missing_owners
                .iter()
                .filter_map(|owner| tags.and_then(|tags| tags.get(owner))),
        );
        Ok(sample)
    }

    #[allow(clippy::too_many_arguments, reason = "separate mutable state for the host's composed icon views")]
    pub fn draw_quiet(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        pools: viewer_foundation::map::icon_windows::Pools,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<()> {
        let Self { fixed, land, drawn } = self;
        let (mut scratch, mut report) = (Batch::default(), CounterReport::default());
        let countries = fixed.naval.countries.as_deref();
        if pools.pings {
            fixed.native_pings.draw(inputs, pings, time, countries, &mut scratch, &mut report);
        }
        if pools.bases {
            fixed.native_bases.draw(inputs, bases, time, fixed.naval.countries.as_ref(), &mut scratch, &mut report);
        }
        if pools.land_combats {
            fixed.native_land_combats.draw(inputs, land_combats, time, countries, &mut scratch, &mut report);
        }
        if !pools.naval {
            return Ok(());
        }
        let plates: Vec<_> = drawn
            .at_sea
            .iter()
            .filter_map(|plate| {
                let land = land.as_ref()?;
                Some((&land.flotillas[plate.member], land.countries.as_ref(), plate))
            })
            .collect();
        let pointer = land.as_ref().and_then(|land| land.pointer);
        let (mut bases_batch, mut counters_batch, mut combats) = (Batch::default(), Batch::default(), Batch::default());
        naval_windows::present(
            inputs,
            &fixed.naval,
            &plates,
            &drawn.columns,
            time,
            pointer,
            naval,
            &mut bases_batch,
            &mut counters_batch,
            &mut combats,
            &mut report,
        )?;
        let empty_air = air_missions::Prepared::default();
        fixed.native_air.as_deref().unwrap_or(&empty_air).draw(inputs, time, &mut naval.air_missions, &mut combats, &mut report);
        Ok(())
    }

    pub fn keep_drawn(&mut self, old: &mut PreparedCounters) {
        if let (Some(land), Some(old)) = (self.land.as_mut(), old.land.as_mut()) {
            land.keep_drawn(old);
        }
    }

    pub fn keep_drawn_by_value(&mut self, old: &mut PreparedCounters) {
        if let (Some(land), Some(old)) = (self.land.as_mut(), old.land.as_mut()) {
            land.keep_drawn_by_value(old);
        }
    }

    pub fn drawn(&self) -> (&Batch, &CounterReport) {
        (&self.drawn.batch, &self.drawn.report)
    }

    pub fn drawn_combats(&self) -> &Batch {
        &self.drawn.combats
    }
}

struct Geography<'a> {
    state_of: &'a BTreeMap<u32, u32>,
    extents: &'a Extents,
}

fn ideology_groups(
    frame: &DecodedFrame,
    countries: &BTreeMap<u32, &replay_data::rows::CountryRow>,
) -> io::Result<BTreeMap<u32, Result<Option<String>, String>>> {
    let groups = dashboard_text::flag_choices::ruling_groups(frame)?;
    Ok(countries
        .iter()
        .map(|(&slot, country)| {
            let group = groups.get(&country.id).cloned().unwrap_or_else(|| {
                Err(format!(
                    "country {} flag needs its ruling group",
                    country.id
                ))
            });
            (slot, group)
        })
        .collect())
}

fn draw_child(
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

pub(super) fn turned_rect(child: &Child, size: [f32; 2], origin: [f32; 2]) -> [f32; 4] {
    let (width, height) = (size[0] * child.scale, size[1] * child.scale);
    [
        origin[0] + child.at[0],
        origin[1] + child.at[1] + (height - width) * 0.5,
        height,
        width,
    ]
}

fn draw_bar(batch: &mut Batch, sheet: &SpriteSheet, child: &Child, origin: [f32; 2], fill: f32) {
    let rect = turned_rect(child, sheet.frame_size(), origin);
    batch.bar(
        sheet,
        [rect[0], rect[1]],
        [rect[2], rect[3]],
        counter_inputs::bar_pixels(fill, rect[2]),
        [1.0; 4],
    );
}

fn draw_text(
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
    let glyphs = text::layout(
        font,
        value,
        [origin[0] + child.at[0], origin[1] + child.at[1]],
        child.align,
        Some(child.max_width),
    );
    if !glyphs.is_empty() {
        batch.text(&glyphs, color);
    }
}

#[derive(Clone, Default)]
struct Countries {
    tags: BTreeMap<u32, String>,
    flag_tags: BTreeMap<u32, FlagTags>,
    sides: BTreeMap<u32, Allegiance>,
    groups: BTreeMap<u32, Result<Option<String>, String>>,
    ui_colors: UiColors,
}

impl Countries {
    pub(super) fn same_flags(&self, other: &Self) -> bool {
        self.tags == other.tags && self.flag_tags == other.flag_tags && self.groups == other.groups
    }
}

#[derive(Clone, Debug, Default)]
struct UiColors {
    colors: BTreeMap<u32, [f32; 4]>,
    inheritance: viewer_foundation::map::color_inheritance::Inheritance,
    gap: String,
    unavailable: Option<String>,
}

impl UiColors {
    fn read(map: &MapData, frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> io::Result<Self> {
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
                    [row.ui_red, row.ui_green, row.ui_blue, row.ui_alpha].map(f32::from_bits);
                Some((slot, rgba))
            })
            .collect();
        Ok(
            match viewer_foundation::map::color_inheritance::inheritance(map, frame, links) {
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
    fn ui_color(&self, slot: u32) -> Result<[f32; 4], String> {
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
        colors.colors.get(&source).copied().ok_or_else(|| {
            format!(
                "country-colour mode needs country slot {source}'s UI colour, \
                 but {}",
                colors.gap
            )
        })
    }

    fn warm_flags(&self, assets: &CounterAssets) {
        let keys: BTreeSet<String> = self
            .tags
            .keys()
            .filter_map(|&slot| self.flag_keys(slot))
            .flatten()
            .collect();
        for flags in [&assets.flags, &assets.medium_flags, &assets.large_flags] {
            flags.warm(keys.iter().map(String::as_str));
        }
    }

    fn flag_now(&self, flags: &LazyFlags, slot: u32) -> Option<(usize, [f32; 4])> {
        let keys = self.flag_keys(slot)?;
        flags.acquire(&keys.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn known_side(&self, slot: u32) -> Option<Allegiance> {
        self.sides.get(&slot).copied()
    }

    fn is_ours(&self, slot: u32) -> bool {
        self.known_side(slot) == Some(Allegiance::Ours)
    }

    fn same_allegiance_group(&self, a: u32, b: u32) -> bool {
        a == b
            || matches!(
                (self.known_side(a), self.known_side(b)),
                (Some(first), Some(other)) if first == other
            )
    }

    fn flag_keys(&self, slot: u32) -> Option<Vec<String>> {
        let (cosmetic, original) = self.flag_tags.get(&slot)?;
        let cosmetic = cosmetic.as_ref().ok()?;
        let group = self.groups.get(&slot)?.as_ref().ok()?;
        let keys = content_reader_country_names::country_names::flag_keys(
            original.as_deref().ok(),
            cosmetic,
            group.as_deref(),
        );
        Some(keys)
    }

    fn flag_refusal(&self, slot: u32) -> Option<String> {
        let Some((cosmetic, original)) = self.flag_tags.get(&slot) else {
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

fn unit_flag_owner(
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

const NAVAL_KIND: u32 = 1;

const COARSE_PLATE_FRAME: u32 = 2;

struct CoarseCounter<'a> {
    plate: &'a Child,
    large_flag: &'a Child,
    count_box: &'a TextChild,
    tint: [f32; 4],
    owner: u32,
    count: &'a str,
    keys: (u32, u32, u32),
    plate_drawn: bool,
}

fn draw_coarse_counter(
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

fn rank_children(batch: &mut Batch, first: usize, priority: i8, icon: u32) -> io::Result<()> {
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

fn draw_flag(
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

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
