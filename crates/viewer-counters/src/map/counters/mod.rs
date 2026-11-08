//! On-map unit counters, base icons and combat icons.
//!
//! Resolved GUI windows supply hierarchy, resources and child layout. World anchors
//! come from the content's unit stacks and building slots; layout uses interface
//! pixels ([`super::ui_scale`]). Port children bypass the rounded zoom offset that
//! applies to ordinary unit counters when 3D units are enabled.
//!
//! [`CounterDefines`] reads strict camera cutoffs and signed priorities from content.
//! Groups use their own stack priority; ungrouped hovered stacks add five. Quads keep
//! the `(priority, icon, child)` layer key so page grouping preserves icon order.
//! Fixed-point ship and division values remain on their recorded scale until ratios
//! cancel it. Converting beforehand would lose bits.
//!
//! Recorded template icons, status, movement and leader inputs drive the corresponding
//! children. Missing tables retain their feature-specific fallback and named limitation.
//! At-sea naval bars average fully visible members; port bars use all members behind
//! the representative's visibility gate. Own counters bypass that gate. Low-intel
//! stacks show the no-intel icon and unknown count while fog is enabled.
//!
//! Railway guns share sources and columns with divisions but use separate plates.
//! At-sea task forces use naval plates; docked task forces use port items and active
//! regional missions use mission icons. Movement uses the settled route point.
//! Land-combat tint uses side survival estimates and a ten-percent comparison band;
//! missing operands retain unknown white, while no intel hides the wedge. Definition
//! terrain selects the authored plate suffix. HQ and addition markers need further
//! inputs. Orders-group markers copy recorded own or inherited colour directly.
//!
//! Coarse counters retain allegiance tint, show the representative's own-owner flag
//! from the medium atlas, preserve count or unknown count, and hide the other children.
//! Detailed counters may use an expeditionary owner; port flag selection is separate.
//! Controllers select neither flag. Grouping uses mean member anchors and vertical
//! columns ([`crate::map::counter_groups`]); source and pool history belongs to each view.

#[cfg(test)]
use map_products::loader::default_anchor;
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

use installed_content_core::{
    ResolvedLayers, buildings::Slot, defines::Defines, flags, flags::FlagSet, gui::native,
    map::MapData, map::ProvinceKind, positions,
};
use installed_content_gfx::gfx::SpriteSet;
use installed_content_image::dds;
use installed_content_regions::regions;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use viewer_foundation::map::icon_windows::on_screen;
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
mod preparation;
/// How many base icon families the counters draw, one pool view each.
pub use base_windows::View as BaseView;
pub use preparation::Preparation;
mod air_volunteers;
mod bases;
pub mod combat_markers;
pub mod faction_pings;
mod geography;
pub mod gui_windows;
mod initial_flags;
pub use initial_flags::warm_initial_flags;
use viewer_foundation::map::native_windows::NativeWindows;
mod hovers;
mod land;
mod layouts;
use layouts::*;
pub mod land_windows;
mod naval;
mod naval_hovers;
mod naval_markers;
mod naval_mine_badges;
mod naval_mission_inputs;
mod naval_missions;
mod naval_port_hovers;
mod naval_regional;
mod naval_repairs;
pub use air_missions::AirPasses;

pub type MissionMemo = Recent<(replay_data::BranchId, replay_data::Hour, u32), MissionIcons>;
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

/// The file every map-icon window lives in.
use viewer_foundation::map::native_windows::MAPICONS_GUI;

/// The content-defined y lift added when zoomed in.
pub const ZOOMED_IN_OFFSET: &str = "unit_counter_zoomed_in_offset";
/// kind 0.
pub const LAND_WINDOW: &str = "unit_counter";
/// The naval counter window.
use viewer_foundation::map::native_windows::NAVY_WINDOW;
pub const STACK_WINDOW: &str = "unit_mapicon";
use viewer_foundation::map::native_windows::AIR_BASE_WINDOW;
/// its `bg_btn` is replaced per facility.
use viewer_foundation::map::native_windows::FACILITY_WINDOW;
use viewer_foundation::map::native_windows::NAVAL_BASE_WINDOW;
use viewer_foundation::map::native_windows::NAVAL_HEADQUARTER_WINDOW;
use viewer_foundation::map::native_windows::{
    GUN_EMPLACEMENT_WINDOW, RADAR_WINDOW, ROCKET_SITE_WINDOW, STRATEGIC_LOCATION_WINDOW,
};
/// this prefix and the building's first specialization token.
pub const FACILITY_SPRITE_PREFIX: &str = "GFX_mapicon_facility_";
use viewer_foundation::map::native_windows::LAND_COMBAT_WINDOW;
/// The naval combat icon window.
use viewer_foundation::map::native_windows::NAVAL_COMBAT_WINDOW;
/// The retained naval result icon window.
use viewer_foundation::map::native_windows::NAVAL_RESULT_WINDOW;
/// kind 2.
pub const GUN_WINDOW: &str = "unit_counter_railway_gun";
/// The item window the land counter's `active_abilities` grid creates per ability.
pub const ABILITY_WINDOW: &str = "active_ability_item";

/// Unit-stack type 9, the attacker's side of the combat stand-off line.
/// The unit-stack kinds of a province's anchor table, `0` to `0x26`
///
#[cfg(test)]
use viewer_foundation::map::counter_anchors::STACK_KINDS;
/// Slot 38 (`0x26`), `Victory point`: the province label's anchor, which
/// inherits slot [`DEFENDING`] where the file leaves it unset ([`stack_anchor`]).
use viewer_foundation::map::label_reads::VICTORY_POINT;
/// Unit-stack type 10, the defender's side.
use viewer_foundation::map::unit_rules::DEFENDING;
/// Unit-stack type 19, the dock position of a coastal land province.
use viewer_foundation::map::unit_rules::SHIP_IN_PORT;
/// Unit-stack type 0, the idle counter anchor.
use viewer_foundation::map::unit_rules::STANDSTILL;

/// The cloth shading the game blends
/// into every medium cell; the small and full sets get none.
pub const MEDIUM_FLAG_OVERLAY: &str = "gfx/interface/small_flag_overlay.tga";

/// A stack the perspective cannot see shows `?` in its count box instead of the formatted number.
pub const UNKNOWN_COUNT: &str = "?";

/// The sprite that replaces the hidden counter's `type` child. Its one-based frame
/// is 1 with NATO counters off and 2 with NATO counters on. The former is the
/// zero-based [`templates::PICTORIAL_FRAME`].
use viewer_foundation::map::templates::NO_INTEL_SPRITE;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// One sprite child of a map-icon window, as `mapicons.gui` declares it.
#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    /// The `GFX_...` name, from `spriteType` or `quadTextureSprite`.
    pub sprite: String,
    /// `position`, in the window's own pixels.
    pub at: [f32; 2],
    /// `scale`; 1.0 when the child declares none.
    pub scale: f32,
    /// `frame` as the file writes it, **one-based**; 0 when it declares none.
    pub frame: u32,
    /// `rotation` in radians; 0 when the child declares none.
    pub rotation: f32,
    /// Icon centerPosition; unsupported action children are refused individually.
    pub centered: bool,
    /// Its place in its window's declaration order, which is the engine's draw order.
    pub order: u32,
}

impl Child {
    /// The zero-based strip frame the file's `frame` names, or `default`.
    pub fn frame_index(&self, default: u32) -> u32 {
        self.frame.checked_sub(1).unwrap_or(default)
    }
}

/// One text child of a map-icon window.
#[derive(Clone, Debug, PartialEq)]
pub struct TextChild {
    /// `font`; `None` when the box names none.
    pub font: Option<String>,
    /// See [`Child::order`].
    pub order: u32,
    pub at: [f32; 2],
    /// `maxWidth`, the box a run is aligned inside and cut at.
    pub max_width: f32,
    pub align: text::Align,
}

/// Every `NDefines` value the counter pass reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterDefines {
    /// `LAND_UNIT_MOVEMENT_SPEED` and `NAVAL_UNIT_MOVEMENT_SPEED`: the free arc's speeds, which
    /// a view's movement clock carries to its counters and figures.
    pub movement_speeds: [f32; 2],
    /// `NGraphics.UNITS_ICONS_DISTANCE_CUTOFF`, a fixed5 define : unit counters
    /// draw below it.
    pub icons_cutoff: f32,
    /// `NGraphics.UNITS_DISTANCE_CUTOFF`, the 3-D unit cutoff the zoom lift fades against.
    pub units_cutoff: f32,
    /// `NGraphics.AIRBASE_ICON_DISTANCE_CUTOFF`.
    pub air_base_cutoff: f32,
    /// `NGraphics.NAVALBASE_ICON_DISTANCE_CUTOFF`.
    pub naval_base_cutoff: f32,
    /// `NGraphics.FACILITY_DISTANCE_CUTOFF`: naval-HQ and facility icons draw strictly below it.
    pub facility_cutoff: f32,
    /// `NMapIcons.HOME_BASE_MAP_ICON_SCALE_DEFAULT`, the naval base button's scale.
    pub naval_base_scale: f32,
    /// Selected home bases use the content's highlighted button scale.
    pub naval_base_highlighted_scale: f32,
    /// `NGraphics.NAVAL_MISSION_ICONS_DISTANCE_CUTOFF` (strict).
    pub naval_mission_cutoff: f32,
    /// `NGraphics.LAND_COMBAT_DISTANCE_CUTOFF`.
    pub land_combat_cutoff: f32,
    /// `NGraphics.NAVAL_COMBAT_DISTANCE_CUTOFF`.
    pub naval_combat_cutoff: f32,
    /// `NGraphics.RADAR_ICON_DISTANCE_CUTOFF`: radar icons draw strictly below it.
    pub radar_cutoff: f32,
    /// Select coarse grouping strictly above the content distance threshold:
    /// `MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE`, and `..._STRATEGIC` in the map modes with
    /// strategic grouping ([`Visibility::strategic_grouping`]).
    pub coarse_distance: [f32; 2],
    /// Every map mode's `NMapIcons.<table>_PRIORITY_*`: unit stacks and their groups (port
    /// counters are the naval base's children), bases, facilities, combats and naval missions.
    pub priorities: Priorities,
    pub groups: GroupDefines,
    /// `NGraphics.MAP_ICONS_GROUP_SPLIT_SELECTED_LIMIT`.
    pub split_selected_limit: i32,
    /// `NNavy.INTEL_LEVEL_*_HALF_RANGE_*`, a naval mission item's count ranges.
    pub naval_counts: viewer_foundation::map::naval_count_approx::Defines,
    pub supply: counter_status::SupplyThresholds,
    pub tints: allegiance::Tints,
    pub land_combat: LandCombatDefines,
    type_colors: type_tint::Colors,
}

impl CounterDefines {
    pub fn read(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
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

    /// Coarse above `MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE`, strictly, or above its
    /// `_STRATEGIC` form when the map mode groups strategically (the air and navy modes).
    pub fn coarse(&self, height: f32, strategic: bool) -> bool {
        height > self.coarse_distance[usize::from(strategic)]
    }

    /// A unit-stack column's priority at a camera height in a map mode's priority table: its
    /// group icon's at every grouping level, its own stack icon's below them.
    #[cfg(test)]
    pub fn stack_priority(&self, height: f32, table: PriorityTable) -> i8 {
        self.level_priority(self.groups.grouping(height), table)
    }

    /// A unit-stack column's priority at its grouping level in a map mode's priority table.
    pub fn level_priority(&self, level: Level, table: PriorityTable) -> i8 {
        let icon = match level {
            Level::None => PriorityIcon::UnitsStack,
            _ => PriorityIcon::UnitsStackGroup,
        };
        self.priorities.get(table, icon)
    }

    /// The shipped `00_graphics.lua` values, for tests only.
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
        // The default table's: units, groups, victory points, air and naval bases, naval
        // headquarters, facilities, land and naval combats, naval results and missions, capitals,
        // rocket sites, gun emplacements, strategic locations, adjacency rules and resources.
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

/// The land counter's children from resolved content.
#[derive(Clone, Debug, PartialEq)]
pub struct LandCounter {
    /// `size`, 76 x 24.
    pub size: [f32; 2],
    pub plate: Child,
    pub selected: Child,
    pub overlay: Child,
    /// The `type` child's position from content; [`super::templates`] selects its
    /// drawn sprite.
    pub type_icon: Child,
    pub bar_org: Child,
    pub bar_str: Child,
    /// The status glyphs the counter hangs off its own plate.
    pub supply: Child,
    pub attrition: Child,
    pub digin: Child,
    pub training: Child,
    /// `onmap_action_icon`.
    pub action: Child,
    pub no_orders: Child,
    /// `defending_area`, the area-defence shield at `(-13, 6)`.
    pub defending_area: Child,
    /// The yellow `!` a member that is not the viewing country's takes instead.
    pub invalid_orders: Child,
    pub flag: Child,
    /// `large_flag`, the medium-atlas flag the coarse counter shows instead of
    /// the type icon, the bars and the small flag.
    pub large_flag: Child,
    pub count: TextChild,
    pub hq_badge: Child,
    pub hq_offset: [f32; 2],
    pub portrait: Child,
    /// `group_marker`, the own orders group's colour.
    pub group_marker: Child,
    /// `leader_enroute`, shown instead of `hq_badge` while the HQ group withdraws.
    pub leader_enroute: Child,
}

/// The railway-gun counter's children. Its own window has no organisation bar
/// and an action icon hanging off its right edge; the game binds them by
/// name like the land counter's.
#[derive(Clone, Debug, PartialEq)]
pub struct GunCounter {
    /// `size`, 58 x 24.
    pub size: [f32; 2],
    pub plate: Child,
    pub selected: Child,
    pub group_marker: Child,
    pub overlay: Child,
    /// `no_orders`, the rail location warning.
    pub no_orders: Child,
    /// `type`; like the land counter's, only its position is read.
    pub type_icon: Child,
    pub count: TextChild,
    pub bar_str: Child,
    pub flag: Child,
    pub large_flag: Child,
    pub supply: Child,
    /// `onmap_action_icon`, `frame update(action code + 1)`.
    pub action: Child,
}

/// The land counter's `active_abilities` grid and the `active_ability_item`
/// window it creates for each active ability.
#[derive(Clone, Debug, PartialEq)]
pub struct AbilityGrid {
    /// The grid's `position` inside `unit_counter`.
    pub at: [f32; 2],
    /// `slotsize`: one step right per ability.
    pub slot: [f32; 2],
    /// The grid's declaration index in `unit_counter`, its draw order.
    pub order: u32,
    /// The item window's `size`.
    pub item_size: [f32; 2],
    /// The item's `active_ability` sprite position, `(0, 0)` when undeclared.
    pub icon_at: [f32; 2],
    /// The item's `active_ability` scale.
    pub scale: f32,
    /// `orientation = center`: the sprite's position is measured from the item's centre
    /// rather than its upper-left corner. It does not centre the sprite itself.
    pub from_centre: bool,
    /// `centerPosition = yes` or `origo = center`: the sprite is centred on its position.
    pub centred: bool,
}

/// The window used for a side's convoy additions.
use viewer_foundation::map::native_windows::TINY_UNIT_COUNTER_WINDOW;

/// Convoy addition icon sprite, resolved through the content sprite registry.
pub const CONVOY_ADDITION_ICON: &str = "GFX_unit_transport_icon_medium_white";

/// The native stack root needed to place land, railway-gun and at-sea counters.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnLayout {
    pub stack_size: [f32; 2],
    pub zoomed_in: [f32; 2],
}

/// The counter and column templates read from the native GUI registry.
#[derive(Clone, Debug, PartialEq)]
pub struct Layouts {
    pub land: Result<LandCounter, String>,
    /// An error refuses railway-gun counters independently of land counters.
    pub gun: Result<GunCounter, String>,
    /// `None` when the grid, its item or its sprite is absent or laid out in a
    /// flow this pass does not reproduce: the ability grid is then not drawn.
    pub abilities: Option<AbilityGrid>,
    pub columns: Result<ColumnLayout, String>,
    /// Children `mapicons.gui` does not declare. The engine binds children by
    /// name and draws nothing for an absent one; so does this pass.
    pub missing: Vec<String>,
    /// Declared elements in a form this pass refuses to guess at, with why.
    pub unsupported: Vec<String>,
}

impl Layouts {
    /// Missing or unsupported templates refuse only the counter kinds that need them.
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

    /// Every `GFX_...` name the layouts refer to, in a stable order.
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
        // Swaps the provider's background by this literal.
        names.insert("GFX_onmap_unit_counter_empty");
        names.into_iter().collect()
    }
}

/// The flags of one [`FlagSet`], indexed through the content stack.
///
/// `GFX_mapicon_unit_large_flag_stripe` carries no `texturefile` at all â€” the
/// shipped `.gfx` says "Texture file is ste in code to proper flag" ( section
/// 6.5) â€” so the flag is not a sprite the loader can resolve. The engine builds
/// its flag atlases before drawing. Counter flags
/// acquire every candidate before their frame; combat retains lazy decoding.
/// The medium set also takes [`MEDIUM_FLAG_OVERLAY`].
/// The game per colour channel: `dst + trunc((overlay - dst) * (overlay_alpha / 255))`,
/// the delta truncated toward zero; the flag's own alpha is kept.
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

/// Static action-child support checked once when the content assets load.
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
        // Action codes zero through twelve retain their positive one-based frames.
        // The draw path does not clamp them.
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

/// The layouts, the sprite sheets they name and the flag atlas.
pub struct CounterAssets {
    air_sortie_defines: Result<viewer_foundation::map::aircraft_count::SortieDefines, String>,
    pub layouts: Layouts,
    /// The naval counter, the naval combat icon, its convoy addition, the port window and the
    /// other natively drawn map-icon windows, read by the shared native reader
    /// (`installed_content_core::gui::native`) over every `.gui` file of the content stack, templates found
    /// as the game finds them, independently of the legacy readers
    /// ([`Layouts`], the `.gfx` sprite map), and drawn through [`crate::map::native_gui`]
    /// ([`gui_windows`]).
    pub windows: SharedNativeWindows,
    /// Viewport-specific native templates shared with combat windows and raid icons.
    pub window_cache: Arc<WindowCache>,
    sheets: BTreeMap<String, SpriteSheet>,
    land_action: Option<ActionContent>,
    gun_action: Option<ActionContent>,
    /// The natively laid-out map-icon windows' sprites, textures, fonts and effects, loaded
    /// through the content stack's native registry as the game loads them
    /// ([`crate::map::native_resources::GuiResources`]): every sprite and font the templates name
    /// ([`crate::map::native_windows::NativeWindows::assets`]) and the sprites the game's code sets on them.
    pub gui: GuiResources,
    /// The names [`Self::gui`] was asked to load, which the census identifies drawn quads by.
    gui_assets: Assets,
    /// Small flag atlas used for detailed counter flags. Resolve cells from the
    /// recorded country appearance and content artwork.
    pub flags: Arc<LazyFlags>,
    /// The medium set `large_flag` draws from at the coarse grouping distance:
    /// the atlas the game selects with its third argument set.
    pub medium_flags: Arc<LazyFlags>,
    /// Medium flag atlas used by coarse counter flags; large masked shields
    /// retain their separate atlas selection.
    pub large_flags: Arc<LazyFlags>,
    /// Every counter unit-type frame the shipped `.gfx` declares, on one page.
    pub types: templates::TypeAtlas,
    portraits: counter_portraits::Portraits,
    /// Sheets only format-23 operands reach: the leader fallback portrait and the rocket-site
    /// carrier glyph. Loaded after every other counter texture, and absent ones are warned about.
    recorded_sheets: BTreeMap<String, SpriteSheet>,
    /// Content ability token to its map-counter sprite.
    pub ability_sprites: Arc<installed_content_abilities::abilities::AbilitySprites>,
    /// Presentation limitations found while loading, stated once.
    pub warnings: Vec<String>,
}

/// Combines map-window resources with the shared, already-harvested tooltip inventory.
pub fn gui_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &installed_content_localisation::localisation::Localisation,
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
    /// Takes the counter layouts and the natively read windows, loads exactly the sprites the
    /// layouts name, packs the flags; the windows' own resources load in [`Self::load_gui`].
    ///
    /// Flags and portraits are indexed here; flags are acquired before the counter frame,
    /// portraits decoded on first draw. `shared` keeps the layers for those reads.
    pub fn load(
        shared: &Arc<ResolvedLayers>,
        (layouts, windows): (Layouts, SharedNativeWindows),
        defines: &Defines,
        (sprites, registry): (&SpriteSet, &native::resources::Registry),
        (language, map_size, decode): (&str, [u32; 2], installed_content_image::native::Decode),
        images: &mut Vec<dds::Texture>,
        lazy: &Arc<LazyTextures>,
    ) -> io::Result<Self> {
        let layers: &ResolvedLayers = shared;
        // Sheet names in load order; decoded on scoped threads below and
        // appended in this order, so texture ids match the serial load.
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
            // The engine draws nothing for a sprite it cannot resolve; so does
            // this pass, per child.
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
        // `flag atlas loading` logs an unreadable overlay and
        // builds the medium atlas without it.
        let overlay_of = |stack: &ResolvedLayers| {
            stack
                .read(MEDIUM_FLAG_OVERLAY)
                .ok()
                .and_then(|bytes| flags::decode(&bytes, medium_cell))
        };
        // An overlay the visual mods break is the recording's own, named.
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
        // Sheets only recorded operands reach, loaded after every other counter texture.
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
        // Every sprite the content's abilities name, loaded once whatever the hour holds;
        // an ability whose sprite does not load draws no grid item, as in the game.
        let ability_sprites = if layouts.abilities.is_some() {
            installed_content_abilities::abilities::read(layers).unwrap_or_else(|error| {
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
        // The map-icon windows' resources, through the native registry only; loaded by
        // [`Self::load_gui`] once the localisation their texts are shown through is read.
        // The provinces bitmap's size is the global defines' map size every effect compiles
        // with; every texture decodes under the viewer's own `settings.txt`
        // (`force_pow2_textures`, also the effect globals' MAP_POW2/FOW_POW2).
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

    /// Loads the map-icon windows' resources through the native registry
    /// ([`gui_windows::window_assets`]): what their templates name, the sprites the game's code
    /// sets on them, and the inline icons their authored texts show through `localisation` (the
    /// names pass's, so it is read once), appending each texture to `images`. Returns what does
    /// not load, for the warning area.
    pub fn load_gui(
        &mut self,
        layers: &ResolvedLayers,
        localisation: &installed_content_localisation::localisation::Localisation,
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

    /// The recorded operative sprite's first texture frame, keeping the map icon's own mask.
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

    /// The sheet a headquarters portrait resolves to, or `None` when it names nothing loaded or a
    /// native input that is not recorded. A fallback descriptor that does not resolve draws
    /// `GFX_leader_unknown`, as the game does.
    ///
    /// A portrait not decoded yet is queued and draws nothing for that frame;
    /// it is not "unresolved", so it never falls back to the unknown leader.
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

    /// The sheet of a `GFX_...` name, when it loaded; an absent child draws nothing.
    pub fn sheet(&self, name: &str) -> Option<&SpriteSheet> {
        self.sheets
            .get(name)
            .or_else(|| self.recorded_sheets.get(name))
    }

    /// The sheet of an ability token's map-counter sprite, when the content
    /// names one and it loaded.
    fn ability_sheet(&self, token: &[u8]) -> Option<&SpriteSheet> {
        self.sheet(self.ability_sprites.sprite(token)?)
    }

    /// Sheets loaded, for reporting.
    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    /// Dump the submitted counter batch, rather than regenerating a second
    /// diagnostic layout. Frames are zero-based; rectangles are the batch's
    /// interface pixels, which are device pixels at a `ui_scale` of 1.0 and
    /// otherwise become device pixels through [`viewer_foundation::map::sprites::device_corners`].
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

/// The world anchors every icon of this pass hangs on.
pub struct CounterContent {
    /// Shared authored terrain and combat widths.
    pub terrain: Arc<TerrainContent>,
    /// Authored wing capacities, independent of terrain/attrition defines.
    pub wing_types: Result<Arc<installed_content_core::unit_leader::SubUnits>, String>,
    mission_content: Result<naval_mission_inputs::Content, String>,
    pub recorded_labels: Option<Arc<viewer_foundation::naval_labels::RecordedLabels>>,
    pub port_mission_defines: viewer_foundation::map::port_mission_defines::Defines,
    /// `(province, unit-stack type)` to world position. Every type is kept because
    /// the combat icon reads the attacker's own slot. The game checks that a unit's
    /// stack kind is at most 0x25.
    pub anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    /// Authored animator scale for each province and stack kind.
    stack_scales: BTreeMap<(u32, u32), f32>,
    /// Each province's anchor for every slot `map/unitstacks.txt` leaves out
    /// ([`CounterContent::seed_default_anchors`]).
    pub default_anchors: Arc<BTreeMap<u32, [f32; 3]>>,
    /// Each listed province's air-base, rocket-site and gun-emplacement icon,
    /// by base kind and province: every province with an admitted
    /// `map/buildings.txt` record whose bucket holds a type of the kind owns
    /// one ([`SiteSlots::site_icons`]).
    pub site_icons: Arc<[BTreeMap<u32, building_facts::SiteIcon>; building_facts::ICON_KINDS]>,
    pub air_base_first_provinces: Arc<building_facts::FirstProvinces>,
    /// First is_port building slot in province order.
    ports: BTreeMap<u32, [f32; 3]>,
    /// The naval-HQ and special-facility icons' templates and anchor records.
    pub building_icons: BuildingIconContent,
    /// Each defined province's kind, for the naval base's province-access test.
    pub kinds: Arc<BTreeMap<u32, ProvinceKind>>,
    /// Each province's bounding box in map units, for the grouping pass's two
    /// `HUGE` tests; see [`crate::map::counter_groups::Extents`].
    pub bounds: Arc<BTreeMap<u32, Bounds>>,
    /// Recent immutable grouping products, keyed by their ordered state projection and content.
    geography: geography::Memo,
    template_icons: TemplateIconMemo,
    /// Division routes, shared by the counter tables and the unit models.
    pub routes: viewer_foundation::map::counter_move::RouteMemo,
    /// Province to strategic region, from `map/strategicregions/*.txt`.
    pub region_of: Arc<BTreeMap<u32, u32>>,
    /// Each strategic region's provinces in file order. Repeated provinces are logged
    /// and skipped, as are province ids the map does not define.
    region_provinces: BTreeMap<u32, Vec<u32>>,
    /// Icon positions for every region [`CounterContent::place_region_icons`] could
    /// build; empty until it runs.
    region_icons: BTreeMap<u32, [f32; 3]>,
    region_names: BTreeMap<u32, String>,
    ground_crew_defines: Result<(i64, i64), String>,
    /// `NGame.AIR_LOG_TIMEOUT_HOURS`, the daily air pass's pruning window
    /// ([`replay_derive::air_region_derive`]).
    air_log_timeout: Result<i32, String>,
    volunteer_air_defines: Result<air_volunteers::Defines, String>,
    /// Provinces with no `Standstill` anchor, authored or default, stated
    /// once; filled by [`CounterContent::seed_default_anchors`].
    pub missing_standstill: Vec<u32>,
    map: MapSize,
    /// `NNavy.AGGRESSION_SETTINGS_VALUES` as the game holds it.
    pub aggression: Arc<Vec<i64>>,
    /// Province topology and intel defines for the fog rebuild, or why they
    /// failed to load; a failure leaves fog undecided, not the map.
    pub fog: Arc<Result<FogContent, String>>,
    pub defines: CounterDefines,
    /// Signed content threshold for the corresponding counter branch.
    pub supply_grace: Result<i32, String>,
    /// The signed integer threshold used by the game's HQ header.
    pub comms_max_distance: Result<i32, String>,
    /// Scale each division's fresh dig-in bonus by this content define.
    pub dig_in_factor: Result<i64, String>,
    /// The defines and terrain attrition the attrition hover's lines read
    /// (division attrition calculation).
    pub attrition: Result<Arc<viewer_foundation::map::counter_attrition::Content>, String>,
    /// The native active-ability headquarters communication scalings.
    pub ability_comms_scaling: Result<Arc<Vec<i64>>, String>,
    pub ability_no_hq_scaling: Result<i64, String>,
    /// The authored sub-unit keys by the lexer's token identity, which name
    /// a railway gun's stat definition, or why they failed to load.
    pub subunit_tokens: Result<TokenKeys, String>,
    /// The ideology groups whose icon frame a detailed at-sea naval counter shows on
    /// `counter_ideology` ([`naval::ideology_frame`]); set when the document is read.
    pub ideologies: installed_content_core::ideologies::Ideologies,
}

impl CounterContent {
    /// Reads `map/unitstacks.txt` and `map/buildings.txt` from the recorded `layers`. The
    /// building slots admit by the entities `artwork` registers, the stack the building layer
    /// draws from, so both agree on which slots stand.
    pub fn load(
        layers: &ResolvedLayers,
        artwork: &ResolvedLayers,
        map: &MapData,
        defines: &installed_content_core::defines::Defines,
        regions: &BTreeMap<u32, regions::StrategicRegion>,
    ) -> io::Result<Self> {
        Self::load_with_products(None, layers, artwork, map, defines, regions)
    }

    pub fn load_with_products(
        products: Option<&map_products::loader::Loader>,
        layers: &ResolvedLayers,
        artwork: &ResolvedLayers,
        map: &MapData,
        defines: &installed_content_core::defines::Defines,
        regions: &BTreeMap<u32, regions::StrategicRegion>,
    ) -> io::Result<Self> {
        let stacks = match products {
            Some(products) => products.anchors(layers)?,
            None => map_products::loader::StackAnchors::read(layers)?,
        };
        let anchors = stacks.positions;
        let stack_scales = stacks.scales;
        let mut ports = BTreeMap::new();
        let definitions = installed_content_core::buildings::read(layers)?;
        let port_slots = port_slot_names(&definitions);
        let provinces = ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        };
        let building_slots = positions::read_buildings(layers)?;
        // The icons anchor only at slots the loader admits, by the building layer's own rules;
        // document.rs reports the state id conflicts once.
        let (states, _) = installed_content_states::states::read(layers, defines, &map.definitions)?;
        let roster = artwork.descriptor_roster()?;
        let admit = |spawn: &installed_content_core::buildings::SpawnPoint,
                     slot: &installed_content_core::positions::BuildingPosition,
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
        // Every air-base province owns an icon, placed through the content's
        // building types rather than a slot name.
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
            // An ambiguous slot anchors nothing; MapBuildingAssets names it.
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
        let (terrain, default_map) = installed_content_core::terrain::read(layers)?;
        let wing_types = installed_content_core::unit_leader::sub_units(layers)
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
            ideologies: installed_content_core::ideologies::Ideologies::default(),
        })
    }

    /// The fog content, or the static reason the rebuild cannot run.
    pub fn fog(&self) -> Result<&FogContent, &'static str> {
        FogContent::loaded(&self.fog)
    }

    /// The world anchor of one unit-stack slot of one province, finalised by
    /// [`stack_anchor`]. Every consumer reads it here, including province labels.
    pub fn anchor(&self, province: u32, kind: u32) -> Option<[f32; 3]> {
        stack_anchor(&self.anchors, &self.default_anchors, province, kind)
    }

    /// Default unit-stack scale is one; standing slots follow the unit class.
    pub fn stack_scale(&self, province: u32, kind: u32) -> f32 {
        self.stack_scales
            .get(&(province, kind))
            .copied()
            .unwrap_or(1.0)
    }

    /// Seed every province's default stack table before applying authored entries.
    /// Defaults use province centres and sampled terrain height; authored slots
    /// replace their matching entries and retain content order.
    pub fn seed_default_anchors(
        &mut self,
        provinces: impl Iterator<Item = u32>,
        centroid: impl Fn(u32) -> Option<[f32; 2]>,
        heights: &HeightField<'_>,
    ) {
        let mut default_anchors = Arc::unwrap_or_clone(std::mem::take(&mut self.default_anchors));
        let (seeded, missing) =
            map_products::loader::seed_default_anchors(provinces, &self.anchors, centroid, heights);
        default_anchors.extend(seeded);
        self.missing_standstill.extend(missing);
        self.default_anchors = Arc::new(default_anchors);
    }

    /// Whether [`CounterContent::anchor`] answers with the province's seeded
    /// default: the file authors neither the slot nor what it inherits.
    pub fn seeded_anchor(&self, province: u32, kind: u32) -> bool {
        self.default_anchors.contains_key(&province) && seeded_only(&self.anchors, province, kind)
    }

    /// Every listed province's icon of base `kind`: its province, the state
    /// whose base of that kind it shows, and its world anchor, or why it has
    /// none. An icon no record places stands on the province's unit-stack
    /// slot 0.
    pub fn site_icons(
        &self,
        kind: usize,
    ) -> impl Iterator<Item = (u32, u32, Result<[f32; 3], String>)> + '_ {
        self.site_icons[kind].iter().map(|(&province, icon)| {
            let anchor = match &icon.anchor {
                Ok(building_facts::IconAnchor::Record(at)) => Ok(*at),
                Ok(building_facts::IconAnchor::Province) => self
                    .anchor(province, STANDSTILL)
                    .ok_or_else(|| "the province has no unit-stack slot 0".to_owned()),
                Err(reason) => Err(reason.clone()),
            };
            (province, icon.state, anchor)
        })
    }

    /// Naval counters in a land province use the first port-building visual record,
    /// falling back to unit-stack19.
    pub fn port(&self, province: u32) -> Option<[f32; 3]> {
        self.ports
            .get(&province)
            .copied()
            .or_else(|| self.anchor(province, SHIP_IN_PORT))
    }

    /// A defined province's kind.
    pub fn province_kind(&self, province: u32) -> Option<ProvinceKind> {
        self.kinds.get(&province).copied()
    }

    /// The province's strategic region, absent when no region claims it.
    pub fn region(&self, province: u32) -> Option<u32> {
        self.region_of.get(&province).copied()
    }

    /// A strategic region's provinces in authored file order.
    pub fn region_provinces(&self, region: u32) -> &[u32] {
        self.region_provinces
            .get(&region)
            .map_or(&[], Vec::as_slice)
    }

    /// A strategic region's icon position.
    pub fn region_icon(&self, region: u32) -> Option<[f32; 3]> {
        self.region_icons.get(&region).copied()
    }

    /// Builds every region's icon position once the province centroids and the height field
    /// exist ([`faction_pings::region_icon_position`]). Returns the regions that have none: no
    /// provinces, or a province with no pixel centroid.
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

    /// The two `HUGE` extents for this hour's province-to-state assignment.
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

/// Whether [`stack_anchor`] answers a province's `kind` with its seeded
/// default: no authored record, nor an inherited one.
fn seeded_only(anchors: &BTreeMap<(u32, u32), [f32; 3]>, province: u32, kind: u32) -> bool {
    !anchors.contains_key(&(province, kind))
        && (kind != VICTORY_POINT || !anchors.contains_key(&(province, DEFENDING)))
}

/// Each province's terrain name as the definition reader stores it
/// ([`installed_content_core::map::province_terrains`]); a combat icon's background sprite
/// is named after it.
fn combat_terrains(bytes: &[u8]) -> io::Result<BTreeMap<u32, String>> {
    installed_content_core::map::province_terrains(bytes)?
        .into_iter()
        .map(|(id, terrain)| {
            String::from_utf8(terrain)
                .map(|terrain| (id, terrain))
                .map_err(|_| invalid(format!("province {id}'s terrain name is not UTF-8")))
        })
        .collect()
}

/// Build eligible anchor buckets from port templates, following their
/// authored spawn-point redirects.
fn port_slot_names(definitions: &installed_content_core::buildings::Buildings) -> BTreeSet<&str> {
    definitions
        .types
        .values()
        .filter(|building| building.is_port)
        .flat_map(|building| [building.name.as_str(), building.slot_name()])
        .collect()
}

/// Everything one frame of icons needs besides the decoded hour.
pub struct CounterInputs<'a> {
    pub content: &'a CounterContent,
    /// Already loaded recorded equipment; demanded only by reached variant ties.
    pub equipment: Result<&'a installed_content_core::equipment::Database, &'a str>,
    pub assets: &'a CounterAssets,
    /// The windows as read in this view's GUI viewport ([`WindowCache::at`]).
    pub windows: &'a SharedNativeWindows,
    pub fonts: &'a text::Fonts,
    /// The content stack's localisation in the viewer's language, which a natively laid-out
    /// window's authored text key is shown through.
    pub localisation: &'a installed_content_localisation::localisation::Localisation,
    /// Language of cached names in this recording.
    pub recorded_language: &'a str,
    pub map: &'a MapData,
    pub camera: Camera,
    /// The frame in device pixels, which the projection uses.
    pub viewport: [u32; 2],
    /// Device pixels per interface pixel; see [`super::ui_scale`]. The icons
    /// are laid out in interface pixels.
    pub ui_scale: f32,
    /// The content movement map a route glides over; `None` stands movers still.
    pub movement: Option<&'a MovementContent>,
    /// The terrain height the route sampler reads.
    pub heights: HeightField<'a>,
    /// The user's counter options.
    pub settings: CounterSettings,
    /// Which counters this view draws; hidden ones keep their hovers and reports.
    pub display: &'a CounterDisplay,
    /// This view's local selection, never a captured game UI operand.
    pub selected: &'a BTreeSet<Unit>,
    /// Selected air wings, which gate the strategic base's numeric text.
    pub selected_air: &'a [u64],
    pub selected_air_bases: &'a [u64],
    /// Viewer-local fleet selection, distinct from selected task forces.
    pub selected_fleets: &'a [u64],
    /// The local pointer's active counter window, identified by one bound member.
    pub hovered: Option<Unit>,
    /// The host view's shared province schedule.
    pub schedule: &'a Shared,
    /// The host view's unit presentation history, which its figures share.
    pub movement_history: &'a viewer_foundation::map::counter_move::SharedMovement,
    /// What the view's map mode shows of the counters and icons.
    pub visibility: Visibility,
}

/// Which unit counters a view draws. A hidden counter is laid out, hit-tested, reported and
/// kept in its presentation windows as before; only its quads stay out of the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterDisplay {
    pub land: bool,
    pub railway_guns: bool,
    /// Task forces at sea.
    pub naval: bool,
    /// Ships in port on naval bases.
    pub ports: bool,
    /// Canonical country slots whose land, railway-gun, naval and port counters draw; `None` draws
    /// every country's.
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

    /// Whether a land plate draws: a railway gun's or a division's, by its owner's slot.
    pub fn land_plate(&self, gun: bool, owner: u32) -> bool {
        (if gun { self.railway_guns } else { self.land }) && self.owner(owner)
    }

    /// Whether a task force at sea draws.
    pub fn naval_plate(&self, owner: u32) -> bool {
        self.naval && self.owner(owner)
    }

    /// Whether a port item on a naval base draws, by its task force's owner where it has one.
    pub fn port_item(&self, owner: Option<u32>) -> bool {
        self.ports && owner.is_none_or(|owner| self.owner(owner))
    }
}

/// How counter plates are coloured, selected by `counter_color_mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CounterColorMode {
    /// 1: `NGraphics.COUNTER_MODE_ALLEGIANCE_*` by the counter's side against the viewer (a
    /// counter's type icon relation, [`type_tint::plate`]; a mission item's own allegiance).
    Allegiance,
    /// 0: the owner's current UI colour.
    Country,
}

/// Choose allegiance tint when the counter-colour setting requests it; otherwise
/// use the country's current UI colour and applicable subject-colour chain.
/// Counter and mission items retain their own allegiance decisions.
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

/// The counter options, switchable in the viewer as in the game's own options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CounterSettings {
    pub color_mode: CounterColorMode,
    /// `template_counter` equal to 1 selects the unit icons' NATO frame.
    pub nato: bool,
}

impl CounterColorMode {
    /// The map option's default ([`dashboard_core::map_display::option_defaults`]).
    pub const DEFAULT: Self =
        if dashboard_core::map_display::option_defaults::COUNTRY_COUNTER_COLOURS {
            Self::Country
        } else {
            Self::Allegiance
        };
}

impl Default for CounterSettings {
    /// The map option's colours and `settings.txt`'s unstored NATO frame.
    fn default() -> Self {
        Self {
            color_mode: CounterColorMode::DEFAULT,
            nato: installed_content_settings::settings::NATO_COUNTERS_DEFAULT,
        }
    }
}

impl CounterInputs<'_> {
    /// The route sampler's content, when the movement map loaded.
    pub fn route_world(&self) -> Option<counter_move::RouteWorld<'_>> {
        Some(counter_move::RouteWorld {
            map: &self.movement?.map,
            heights: self.heights,
            width: self.camera.map.x(),
        })
    }
}

impl CounterInputs<'_> {
    /// `world`'s screen position in interface pixels.
    fn screen(&self, world: [f32; 3]) -> Option<[f32; 2]> {
        project(&self.camera, world, self.viewport, self.ui_scale)
    }

    /// Resolve the root naval counter window in the view's GUI viewport.
    fn navy_size(&self) -> Result<[f32; 2], String> {
        container_frame(
            self.windows.navy(),
            Some(gui_resources_viewport(self.viewport, self.ui_scale)),
        )
        .map(|frame| frame.size.map(|value| value as f32))
    }

    /// The frame's size in interface pixels.
    fn extent(&self) -> [f32; 2] {
        ui_scale_extent(self.viewport, self.ui_scale)
    }

    /// the local selected-unit count
    /// does not exceed the content limit and the group stands nearer than the strict cutoff
    /// (`height`, its [`CounterInputs::icon_height`]).
    fn selected_join(&self, first: Unit, candidate: Unit, height: f32) -> bool {
        let split = self.selected.len() as i32 <= self.content.defines.split_selected_limit
            && height < self.content.defines.units_cutoff;
        crate::map::unit_selection::joins(self.selected, first, candidate, split)
    }

    /// An icon kind's priority in the map mode's table.
    pub(crate) fn priority(&self, icon: PriorityIcon) -> i8 {
        self.content
            .defines
            .priorities
            .get(self.visibility.priorities, icon)
    }

    /// How far an icon at world `anchor` stands from the camera, as the content's map-icon
    /// distances read it: the game's camera height, or a free eye's distance to the anchor.
    pub(super) fn icon_height(&self, anchor: [f32; 3]) -> f32 {
        self.camera.icon_distance(anchor)
    }

    /// Whether icons of a kind cut off at `cutoff` are drawn this frame: strictly below the
    /// cutoff height for the game's camera. A free eye keeps them all, and each fades by its own
    /// distance against the cutoff its quads carry ([`CounterInputs::icon_anchor`]).
    pub(super) fn shows(&self, cutoff: f32) -> bool {
        self.passes(|height| visible(height, cutoff))
    }

    /// Whether a pass gated on the camera height runs this frame: as `gate` decides for the game's
    /// camera; always for a free eye, whose icons each fade by their own distance.
    pub(super) fn passes(&self, gate: impl Fn(f32) -> bool) -> bool {
        self.camera.free().is_some() || gate(self.camera.height)
    }

    /// The tag of an icon of `family` standing at world `anchor` whose kind is cut off at
    /// `cutoff`.
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
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AirMissionHover {
    pub region: u32,
    pub mission: Option<usize>,
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

/// Regional naval controls retain their simulation identity for the hover product.
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

/// What one frame of icons drew, for reporting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CounterReport {
    /// Unit plates actually drawn; own plates support viewer-local selection.
    pub unit_hits: Vec<viewer_foundation::map::counter_hovers::unit_hits::Hit>,
    /// Actual drawn base buttons, at their content-defined native priorities.
    pub base_hits: Vec<viewer_foundation::map::counter_hovers::unit_hits::Blocker>,
    /// Radar base-hit index to province; the range manager follows the hovered icon.
    pub radar_hits: BTreeMap<usize, u32>,
    pub air_base_hits: BTreeMap<usize, (u32, i32)>,
    pub air_grid_hits: BTreeMap<usize, AirGridHover>,
    pub air_mission_hits: Vec<AirMissionHover>,
    pub naval_mission_hits: Vec<NavalMissionHit>,
    pub naval_region_hovers: Vec<NavalRegionalHover>,
    pub regional_naval_unit_hits: BTreeSet<usize>,
    pub combat_hits: Vec<Hit>,
    /// Naval tooltip observers, including progress icons without click callbacks.
    pub naval_marker_hovers: Vec<Hit>,
    /// Land counters' tooltip observers, in draw order.
    pub land_hovers: Vec<CounterHover>,
    /// At-sea task-force counters' root-inherited tooltip observers.
    pub task_force_hovers: Vec<viewer_foundation::map::counter_hovers::task_force::Hover>,
    /// Explicitly bound political port-item callback controls.
    pub task_force_port_hovers: Vec<viewer_foundation::map::counter_hovers::task_force_port::Hover>,
    /// This hour's arrival estimates and their refusals; see [`viewer_foundation::movement_timing::Etas`].
    pub eta: Report,
    pub land_counters: usize,
    /// Railway-gun plates among [`Self::land_counters`].
    pub railway_guns: usize,
    /// Guns not drawn this hour, because their table is incomplete or their stat
    /// definition's name is unrecorded.
    pub unavailable_railway_guns: usize,
    /// Reached gun location warnings lacking a current raw pair-count or cooldown.
    pub unavailable_railway_gun_locations: usize,
    /// Moving guns whose route the content map cannot place; they stand on their province.
    pub unplaced_railway_gun_routes: usize,
    /// Active abilities on drawn counters whose sprite did not resolve.
    pub unavailable_ability_icons: usize,
    pub naval_counters: usize,
    /// Naval counters, port items and combat tiles `unit_counter_navy` does not lay out or draw
    /// for through [`crate::map::native_gui`], by the module's reason or the missing operand;
    /// and the parts their updates leave unknown.
    pub navy_counter_gaps: BTreeMap<Cow<'static, str>, usize>,
    /// What the naval map-icon windows' lifecycle notes without refusing a window, by reason: a
    /// pool's growth, an icon kept by its province's presentation-dirty bit whose anchor does not
    /// project, a window that does not build or tick.
    pub naval_window_notes: BTreeMap<Cow<'static, str>, usize>,
    /// Land and railway-gun counters or children not drawn, by missing input or refused
    /// drawable: plate colour, hover metadata or an undeclared type sprite.
    pub land_counter_gaps: BTreeMap<Cow<'static, str>, usize>,
    /// Action children withheld on drawn own detailed division plates, by missing operand or
    /// unsupported sprite. The plate and its other children still draw.
    pub land_action_gaps: BTreeMap<Cow<'static, str>, usize>,
    /// Task forces the hour cannot place and does not draw, by reason, with their handles: no
    /// motion row or anchor, a port roster the native sort refuses, or a mission skip whose
    /// inputs are not recorded.
    pub unplaced_task_forces: BTreeMap<&'static str, BTreeSet<u64>>,
    /// Moving task forces whose route the content map cannot place, by reason, with their
    /// handles; they stand on their province.
    pub unplaced_task_force_routes: BTreeMap<&'static str, BTreeSet<u64>>,
    /// Mixed-owner port flags withheld because the Ships census is incomplete.
    pub unavailable_naval_flags: usize,
    /// Counters whose relation to the viewing country is unavailable: a detailed
    /// counter's type icon is withheld, and a plate not owned by the viewing
    /// country is drawn as other.
    pub unavailable_type_tints: usize,
    pub type_tint_problem: String,
    /// Counters drawn with the game's question mark because this perspective
    /// cannot see any of their members.
    pub no_intel_counters: usize,
    /// Combat icons whose progress box shows the game's question mark because
    /// no participant on either side is shown to this perspective.
    pub no_intel_combats: usize,
    pub air_bases: usize,
    /// All offered air-base, rocket-site and gun-emplacement icons the viewing country's
    /// fog of war hides, including off-screen anchors ([`Vision::sees_air_base`]).
    pub hidden_air_bases: usize,
    pub naval_bases: usize,
    pub naval_headquarters: usize,
    pub facilities: usize,
    pub rocket_sites: usize,
    pub gun_emplacements: usize,
    pub radars: usize,
    pub strategic_locations: usize,
    /// Naval base, naval-HQ and facility icons an unknown or refused input kept from being
    /// drawn, by reason.
    pub unknown_bases: BaseUnknowns,
    /// All offered facility icons whose sprite the content does not declare, including
    /// off-screen anchors, by sprite name.
    pub unresolved_facility_sprites: BTreeMap<String, usize>,
    /// Named, counted diagnostics of the native base, HQ, facility and land-combat windows.
    pub icon_gui_notes: BTreeMap<String, usize>,
    /// Faction-theatre pins drawn.
    pub faction_pings: usize,
    /// Native faction-pin buttons in paint order; hover highlights their region union.
    pub faction_pin_hovers: Vec<faction_pings::PinHover>,
    /// Named, counted native pin refusals and empty-draw diagnostics.
    pub pin_gui_refusals: BTreeMap<String, usize>,
    /// What kept this hour's pins from being decided or placed.
    pub faction_ping_limits: PinReport,
    pub land_combats: usize,
    /// Combat icons or parts of them the hour's inputs leave undecided, by
    /// kind ("Land", "Naval") and cause, with the combats each cause affects.
    pub combat_gaps: BTreeMap<(&'static str, String), BTreeSet<u64>>,
    /// Combats drawn with a value of limited provenance, or withheld for a
    /// value the recording does not hold, by kind and limitation, with the
    /// combats each limitation applies to.
    pub combat_limits: BTreeMap<(&'static str, String), BTreeSet<u64>>,
    /// Counter parts whose vision-sharing test this hour
    /// cannot decide, by what is affected and the missing input. A proven
    /// positive is never counted; an unknown negative is never drawn as one.
    /// Combat icons name theirs per combat in `combat_gaps`.
    pub unknown_sharing: BTreeMap<(&'static str, SharingUnknown), usize>,
    pub naval_combats: usize,
    /// Naval combat icons an unknown input kept from being placed or drawn; each one's cause and
    /// combat are in `combat_gaps` under "Naval".
    pub unavailable_naval_combats: usize,
    /// Drawn spill-over icons, which the game never rotates. The persistent
    /// icon pool supplies each slot's last frame.
    pub base_frame_naval_combats: usize,
    pub naval_result_icons: usize,
    /// Naval combat grids and convoy additions left undrawn, or drawn without a part the
    /// content does not define or declare, by reason: one count per combat side or addition,
    /// and one per large combat icon whose grid layout the content's form refuses.
    pub naval_grid_gaps: BTreeMap<Cow<'static, str>, usize>,
    /// Result icons not drawn: withheld by a decision or part the hour cannot make, or refused by
    /// the GUI module.
    pub unavailable_naval_results: usize,
    /// Those icons by reason, and the result pass's own gaps (a content with no result window).
    pub naval_result_gaps: BTreeMap<Cow<'static, str>, usize>,
    /// Result icon updates whose sprite the content does not declare under the name the
    /// executable selects, by that name: `sprite assignment` keeps the drawable.
    pub undeclared_result_sprites: BTreeMap<&'static str, usize>,
    /// Naval mission entries drawn.
    pub naval_mission_entries: usize,
    /// Drawn mission entries' children (counts, alerts, efficiencies) hidden until the P2/P3
    /// mission inputs are recorded, by child, kind and input.
    pub undecided_naval_mission_children: BTreeMap<String, usize>,
    /// Mission entries whose kind's executable sprite the content does not define, which then
    /// draw their button's authored sprite.
    pub authored_naval_mission_buttons: usize,
    /// The sprite names behind the count above.
    pub undefined_naval_mission_sprites: BTreeSet<String>,
    /// Mission icons whose region has no icon position.
    pub unanchored_naval_missions: usize,
    /// Mission entries not drawn, by the reason: a mission window the content does not declare
    /// or `crate::map::native_gui` does not lay out.
    pub naval_mission_gaps: BTreeMap<Cow<'static, str>, usize>,
    /// Why this hour's mission icons were left undecided.
    pub naval_mission_limits: BTreeSet<&'static str>,
    /// Stacks whose province has no `Standstill` anchor.
    pub unanchored: usize,
    /// Counters drawn between two provinces rather than on one.
    pub moving: usize,
    /// Owners whose flag is not in the small flag set.
    pub missing_flags: BTreeSet<String>,
    /// Reached counter flags whose recorded operand is unknown, by cause.
    pub counter_flag_gaps: BTreeMap<String, usize>,
    /// Tokens this hour's counters could not name, by reason: base building
    /// keys several tokens spell, and railway gun sub-units.
    pub refused_tokens: BTreeSet<String>,
    /// The grouping level for this camera; absent when no counter pass ran.
    pub grouping: Option<Level>,
    /// Native owner-cache comparisons the recording cannot answer. Those
    /// comparisons keep their groups separate; known grouping and plates remain available.
    pub grouping_gaps: BTreeMap<String, usize>,
    /// Groups the counters were gathered into.
    pub groups: usize,
    /// What the perspective resolved to, so the caller can word its limitation.
    pub perspective: Arc<Vision>,
}

impl CounterReport {
    /// A fresh icon ordinal among the base icons and port items, which share priorities.
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

    /// A fresh icon ordinal among the land combat, naval combat and naval result icons, which
    /// the content may give equal priorities (`naval_combat_priority`, `naval_result_priority`
    /// and the land combat's): past every such icon drawn so far, so no two icons of those passes
    /// share a layer key. A naval combat icon draws its window and grid items as one icon.
    fn combat_ordinal(&self) -> u32 {
        (self.land_combats + self.naval_combats + self.naval_result_icons) as u32
    }

    /// One line per cause a combat drew no icon, or drew one without an
    /// undecided part, then one per limitation of the values combats were
    /// drawn or withheld with, this frame, naming the combats: the windowed
    /// diagnostics, the dump and the counters export carry the same lines.
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

    /// What this hour's counters refused or cannot reproduce, one diagnostics line per reason:
    /// the refused tokens, the mixed-owner port flags withheld, the task forces the hour does
    /// not draw and the moving ones standing on their province, the naval-base, naval-HQ and
    /// facility icons not drawn, what the drawn naval combat icons cannot reproduce of the
    /// client's retained icons, the undecided naval result icons (the undecided naval combat
    /// icons are [`CounterReport::combat_gap_lines`], by cause and combat), the naval combat
    /// grids', convoy additions' and naval counters' gaps, and what the naval mission icons
    /// could not decide, place or draw ([`Self::naval_mission_warnings`]). The window's
    /// diagnostics, a capture, the dashboard's map details and `--dump` print the same lines.
    pub fn warnings(&self) -> impl Iterator<Item = String> + '_ {
        // Keep a combat icon until its pass is off, its combat is destroyed or a side's
        // count is zero (see `naval_markers`); this view keeps its own since it started.
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

    /// One line per kind and reason naval-base, naval-HQ and facility icons were not drawn,
    /// one per facility sprite the content does not declare, and native window diagnostics
    /// for bases, HQs, facilities and land combats.
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

    /// One line per reason task forces were not drawn or stand for an unplaced route, naming
    /// each by its saved `id` (the handle's high word).
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

    /// Counts one counter part whose vision sharing, or a census it reads, is unknown.
    pub(super) fn sharing_unknown(&mut self, what: &'static str, cause: SharingUnknown) {
        *self.unknown_sharing.entry((what, cause)).or_default() += 1;
    }

    /// Appends a later branch in serial order; debug-checks fields it must not write.
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
        } = later;
        self.unit_hits.append(unit_hits);
        self.base_hits.append(base_hits);
        untouched(&*radar_hits, "radar_hits");
        untouched(&*air_base_hits, "air_base_hits");
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
        // The branch report keeps its default perspective, which nothing reads.
        let _ = perspective;
    }

    /// `*self = source.clone()`, reusing this report's storage: a report kept across frames
    /// allocates only when a list outgrows its capacity or the missing-flag set changes.
    pub fn assign(&mut self, source: &Self) {
        self.assign_all_but_flags(source);
        if self.missing_flags != source.missing_flags {
            self.missing_flags.clone_from(&source.missing_flags);
        }
    }

    /// Records task forces the hour does not draw, and why.
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

    /// [`Self::assign`] of every field but `missing_flags`.
    fn assign_all_but_flags(&mut self, source: &Self) {
        let Self {
            unit_hits,
            base_hits,
            radar_hits,
            air_base_hits,
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
        } = source;
        self.unit_hits.clone_from(unit_hits);
        self.base_hits.clone_from(base_hits);
        self.radar_hits.clone_from(radar_hits);
        self.air_base_hits.clone_from(air_base_hits);
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
        // Refusals change only when a task force's reason does; compare first so unchanged
        // maps cost no allocation.
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
        // Both change only when an icon's reason or sprite does; compare first so unchanged
        // maps cost no allocation.
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
        // The gap map changes only when a combat's reason does; compare first so an
        // unchanged map costs no allocation.
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
    }
}

/// Sets `target` to `base` plus `extra`, leaving it untouched when it already holds exactly
/// that set, so an unchanged set costs no allocation.
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

/// One frame of icons, split by `DEFAULT_PRIORITY_*` so the caller can put the
/// victory-point labels between the base icons and the counters.
#[derive(Default)]
pub struct CounterFrame {
    /// Priority 0: faction-theatre pins.
    pub pings: Batch,
    native_pings: PreparedPins,
    /// Base, naval-HQ and facility glyphs, drawn before the victory-point labels.
    pub bases: Batch,
    native_bases: base_windows::PreparedBases,
    native_air: Option<Arc<air_missions::Prepared>>,
    /// Priority 3 port items; the columns of priority-10 counters are laid out per frame by
    /// [`PreparedCounters::draw`].
    pub counters: Batch,
    /// Priority 20: land combat icons.
    pub combats: Batch,
    native_land_combats: land_windows::PreparedCombats,
    /// What the naval map-icon windows ask of the presented frame, decided without any window
    /// ([`naval_windows::Plan`]).
    naval: naval_windows::Plan,
    pub report: CounterReport,
}

/// Whether an icon of this kind is drawn at a camera height: strictly below its cutoff.
pub fn visible(height: f32, cutoff: f32) -> bool {
    height < cutoff
}

/// while 3-D units are enabled, the root window height plus
/// unit_counter_zoomed_in_offset fades linearly to zero at UNITS_DISTANCE_CUTOFF.
/// The offset is rounded independently from the projected column origin.
pub fn zoom_lift(layouts: &ColumnLayout, units_cutoff: f32, height: f32) -> f32 {
    ((layouts.stack_size[1] + layouts.zoomed_in[1])
        * (1.0 - height.min(units_cutoff) / units_cutoff))
        .round()
}

/// When a pass is disabled it supplies no sources and clears its retained
/// source membership. Camera and recording gaps preserve their own refusals.
fn units_shown(inputs: &CounterInputs<'_>, peace_conference: bool) -> bool {
    unit_counters_shown(
        &inputs.visibility,
        inputs.shows(inputs.content.defines.icons_cutoff),
        peace_conference,
    )
}

/// Whether unit counters draw at all: their pass shown at this camera (`shown`), outside a peace
/// conference the viewing country takes part in, and not in the resources and raids map modes
/// ([`Visibility::unit_counters`]).
pub(super) fn unit_counters_shown(
    visibility: &Visibility,
    shown: bool,
    peace_conference: bool,
) -> bool {
    visibility.unit_counters && shown && !peace_conference
}

/// the coarse country grouping above its define.
fn coarse(inputs: &CounterInputs<'_>) -> bool {
    inputs
        .content
        .defines
        .coarse(inputs.camera.height, inputs.visibility.strategic_grouping)
}

/// Pushes one hour's counters and icons into a fresh [`CounterFrame`].
/// Camera-independent icon inputs, parsed once per (branch, hour, perspective)
/// and reused by the window's per-frame rebuilds. None of it moves with the
/// camera: the builders parse their own per job, since sharing one across
/// their threads would serialize them.
///
/// The tables every camera's layout reads whole are shared, not cloned, into
/// each [`PreparedCounters`]: a pan or zoom costs the layout, not a copy of
/// every country's row.
#[derive(Clone)]
pub struct HourTables {
    /// A refinement is a new preparation even when its independent base is shared.
    epoch: Arc<()>,
    base: Arc<HourBase>,
    land_combats: Arc<LandMarkers>,
    naval_markers: Arc<NavalMarkerTables>,
    engaged: Arc<CombatFlags>,
}

/// Immutable hour-worker products, independent of engagement-history publication.
/// Fields remain private; the shared base cannot be edited by a presented frame.
pub struct HourBase {
    frame: Arc<DecodedFrame>,
    engagement_content: EngagementContent,
    relations: Result<Relations, &'static str>,
    links: BTreeMap<u32, u32>,
    vision: Arc<Vision>,
    countries: Arc<Countries>,
    /// Canonical slot to `Countries.id`, the key of `CountryCounterInputs`.
    country_ids: BTreeMap<u32, u32>,
    geography: Arc<geography::Geometry>,
    /// Native directed co-belligerent cache, including each owner's independent refusals.
    native_relations: Arc<allegiance::NativeRelations>,
    /// The viewing country's peace-conference flag.
    peace_conference: bool,
    order: Option<counter_order::RecordedOrder>,
    native: NativeInputs,
    orders: counter_inputs::OrdersIndex,
    icons: Arc<TemplateIcons>,
    status: DivisionStatuses,
    movement: Movements,
    actions: counter_action::Actions,
    /// The faction-theatre pins this hour and perspective call for.
    pings: HourPins,
    guns: GunInputs,
    abilities: AbilityInputs,
    /// Divisions providing a led orders group's HQ.
    hq_providers: BTreeSet<u64>,
    /// [`hq_portraits`].
    portraits: BTreeMap<u64, counter_order::Portrait>,
    /// `ArmyIconInputs` by division.
    icon_inputs: BTreeMap<u64, replay_data::rows::ArmyIconInputRow>,
    task_forces: TaskForceMotions,
    divisions: DivisionCounters,
    base_icons: BaseIcons,
    bases: BaseTables,
    naval: NavalTables,
    ports: PortTables,
    ideologies: BTreeMap<u32, Result<Option<i32>, String>>,
    air_missions: Arc<air_missions::Prepared>,
    missions: Arc<MissionIcons>,
    /// Each owner's type-icon relation to the viewing country.
    type_tints: Arc<type_tint::Index>,
}

impl std::ops::Deref for HourTables {
    type Target = HourBase;

    fn deref(&self) -> &HourBase {
        &self.base
    }
}

// Synthetic fixtures edit a uniquely held base before presenting it. Production
// never obtains mutable access to a worker's shared preparation.
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

/// The immutable content operands of the history-dependent marker producers.
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
) -> io::Result<(Arc<LandMarkers>, Arc<NavalMarkerTables>, Arc<CombatFlags>)> {
    let engaged = CombatFlags::read(frame, vision.ours(), links, relations, index)?;
    // Each kind reads its own flags and retained-frame history, in original order.
    let kept =
        |kind, flags| kept_frames(frame, kind, vision.ours(), flags, index).map(|(kept, _)| kept);
    let land_kept = kept(LAND_ICON, &engaged.land)?;
    let naval_kept = kept(NAVAL_ICON, &engaged.naval)?;
    let land_combats = Arc::new(LandMarkers::read(
        frame,
        vision,
        links,
        &(engaged.land.clone(), engaged.land_provenance),
        &land_kept,
        movement,
        &content.land,
    )?);
    let naval_markers = Arc::new(NavalMarkerTables::read(
        frame,
        Viewer { vision, owner: &|raw| links.get(&raw).copied() },
        &engaged,
        &naval_kept,
        &|province| content.kinds.get(&province).copied(),
        &|province| stack_anchor(&content.anchors, &content.defaults, province, STANDSTILL),
    )?);
    Ok((land_combats, naval_markers, Arc::new(engaged)))
}

impl HourTables {
    /// Decides every counter-size flag candidate on the caller's hour worker.
    /// Drawing still acquires any candidate that has not been warmed.
    pub fn warm_flags(&self, assets: &CounterAssets) {
        self.countries.warm_flags(assets);
    }

    fn land(&self) -> CounterTables<'_> {
        self.base.land()
    }

    /// Every recorded input is still the same immutable allocation. Equal rows in a
    /// replacement frame do not authorize using another hour worker's preparation.
    pub fn is_for_frame(&self, frame: &DecodedFrame) -> bool {
        self.frame.branch == frame.branch
            && self.frame.hour == frame.hour
            && self.frame.metrics == frame.metrics
            && self.frame.tables.iter().zip(&frame.tables).all(|(a, b)| {
                replay_frame::frame::SharedView::ptr_eq(a, b) && a.sampled_at() == b.sampled_at()
            })
    }

    /// Rebuild only the products whose decisions read the engagement history, from
    /// this base's own recorded frame and content. No resource requests or GUI state.
    pub fn refine_engagement(&self, index: Option<&History>) -> io::Result<Self> {
        let (land_combats, naval_markers, engaged) = engagement_tables(
            &self.frame,
            &self.vision,
            &self.links,
            self.relations.as_ref().map_err(|reason| *reason),
            &self.movement,
            &self.engagement_content,
            index,
        )?;
        Ok(Self {
            epoch: Arc::new(()),
            base: Arc::clone(&self.base),
            land_combats,
            naval_markers,
            engaged,
        })
    }

    /// Camera-independent regional source products, with complete ledger coverage.
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

    /// The hour's land marker gaps and limitations, by cause, with the
    /// combats each affects.
    #[cfg(any(test, feature = "test-support"))]
    pub fn land_combat_gaps(&self) -> (&Gaps, &Gaps) {
        (&self.land_combats.gaps, &self.land_combats.limits)
    }
}

/// Fill no land or naval unit source while the viewing country's peace-conference
/// flag is set. Other countries' flags do not matter.
fn peace_conference_hides_units(countries: &[replay_data::rows::CountryRow], ours: u32) -> bool {
    countries
        .iter()
        .any(|country| country.canonical_slot == ours && country.peace_conference_participant != 0)
}

/// Parses everything the icon layout needs that does not move with the camera.
///
/// `vision` is the hour's viewing country, read once per hour and perspective
/// by the caller as the displayed hour's vision.
#[allow(clippy::too_many_arguments)]
pub fn hour_tables(
    frame: &DecodedFrame,
    map: &MapData,
    vision: Arc<Vision>,
    content: &CounterContent,
    assets: &CounterAssets,
    equipment: Result<&installed_content_core::equipment::Database, &str>,
    localisation: &installed_content_localisation::localisation::Localisation,
    etas: &Etas,
    movement_content: Option<&MovementContent>,
    engaged: Option<&History>,
    air_passes: Option<&dyn AirPasses>,
    type_tints: &type_tint::Memo,
    mission_memo: &MissionMemo,
    reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &installed_content_core::province_graph::ProvinceGraph,
            &installed_content_core::province_geometry::ProvinceGeometry,
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
        &mut viewer_foundation::profiling::BuilderSpans::new(false),
    )
}

/// The same hour-table construction, recording only explicit builder child scopes.
#[allow(clippy::too_many_arguments)]
pub fn hour_tables_profiled(
    frame: &DecodedFrame,
    map: &MapData,
    vision: Arc<Vision>,
    content: &CounterContent,
    assets: &CounterAssets,
    equipment: Result<&installed_content_core::equipment::Database, &str>,
    localisation: &installed_content_localisation::localisation::Localisation,
    etas: &Etas,
    movement_content: Option<&MovementContent>,
    engaged: Option<&History>,
    air_passes: Option<&dyn AirPasses>,
    type_tints: &type_tint::Memo,
    mission_memo: &MissionMemo,
    reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &installed_content_core::province_graph::ProvinceGraph,
            &installed_content_core::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
    spans: &mut viewer_foundation::profiling::BuilderSpans,
) -> io::Result<HourTables> {
    let (links, country_ids, relations, countries, peace_conference) =
        spans.measure("overlay.tables.counters.country_inputs", |_| {
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
                .map(|country| {
                    (
                        country.canonical_slot,
                        String::from_utf8_lossy(&country.tag).into_owned(),
                    )
                })
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
            // Plate allegiance colour and counter intel use separate tables, each
            // evaluated against the current viewing country.
            let relations = Relations::read(frame, &links);
            let countries = Countries {
                tags,
                flag_tags: initial_flags::flag_tags(&country_rows, &appearances),
                sides: match &relations {
                    Ok(relations) => allegiance::counter_table(relations, &links, vision.ours()),
                    Err(_) => BTreeMap::from([(vision.ours(), Allegiance::Ours)]),
                },
                groups: ideology_groups(frame, &country_rows)?,
                ui_colors: UiColors::read(map, frame, &links)?,
            };
            let peace_conference = peace_conference_hides_units(country_source, vision.ours());
            Ok::<_, io::Error>((links, country_ids, relations, countries, peace_conference))
        })?;
    let geography = spans.measure("overlay.tables.counters.geography", |_| {
        geography::read(frame, content)
    })?;
    let (pings, abilities, guns, movement, actions) =
        spans.measure("overlay.tables.counters.movement_actions", |spans| {
            let pings = spans.measure("overlay.tables.counters.movement_actions.pins", |_| {
                HourPins::read(frame, &links, vision.ours(), content)
            })?;
            let abilities = spans.measure("overlay.tables.counters.movement_actions.abilities", |_| {
                AbilityInputs::read(frame, &links)
            })?;
            let hostility = spans.measure("overlay.tables.counters.movement_actions.hostility", |_| {
                counter_action::DestinationHostility::read(frame, &links)
            })?;
            let guns = spans.measure("overlay.tables.counters.movement_actions.guns", |_| {
                GunInputs::read(
                    frame,
                    &hostility,
                    content.subunit_tokens.as_ref().map_err(String::as_str),
                )
            })?;
            let movement = spans.measure("overlay.tables.counters.movement_actions.routes", |_| {
                Movements::read(frame, etas, movement_content, content)
            })?;
            let actions = spans.measure("overlay.tables.counters.movement_actions.actions", |_| {
                counter_action::Actions::read(frame, &hostility, Some(vision.ours()))
            })?;
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
            let base_icons =
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
                        },
                        spans,
                    )
                })?;
            Ok::<_, io::Error>((native, order, base_icons))
        })?;
    let missions = spans.measure("overlay.tables.counters.missions", |spans| {
        mission_memo.get_or_insert_with((frame.branch, frame.hour, vision.ours()), || {
            spans.measure("overlay.tables.counters.missions.build", |spans| {
                let (forces, mut icons) =
                    spans.measure("overlay.tables.counters.missions.build.read", |_| {
                        let forces = table_access_rows::<tables::TaskForces>(frame)?
                            .rows()
                            .iter()
                            .map(|row| (row.id, row))
                            .collect();
                        let icons = MissionIcons::read(
                            frame,
                            &vision,
                            &links,
                            &country_ids,
                            &native,
                            &forces,
                        )?;
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
        // Keep HQ providers only for groups with both a leader and provider.
        let hq_providers = rows::<tables::OrdersGroups>(frame)?
            .rows()
            .iter()
            .filter(|group| group.leader != 0 && group.hq_provider != 0)
            .map(|group| group.hq_provider)
            .collect();
        let type_tints = spans.measure("overlay.tables.counters.remaining_hour_base.type_tints", |_| {
            type_tint::Index::read(type_tints, frame, &links, vision.ours())
        });
        let tables = HourBase {
            frame: Arc::new(frame.clone()),
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
                BaseTables::read(frame, &vision, content)
            })?,
            air_missions: spans.measure(
                "overlay.tables.counters.remaining_hour_base.air_missions",
                |_| {
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
                    .map(Arc::new)
                },
            )?,
            base_icons,
            missions,
            portraits: spans.measure(
                "overlay.tables.counters.remaining_hour_base.portraits",
                |_| hq_portraits(frame, &native),
            )?,
            hq_providers,
            icon_inputs: spans.measure("overlay.tables.counters.remaining_hour_base.icon_inputs", |_| {
                Ok::<_, io::Error>(
                    rows::<tables::ArmyIconInputs>(frame)?
                        .rows()
                        .iter()
                        .map(|row| (row.division, row.clone()))
                        .collect(),
                )
            })?,
            native_relations: spans.measure(
                "overlay.tables.counters.remaining_hour_base.native_relations",
                |_| allegiance::NativeRelations::read(frame, &links).map(Arc::new),
            )?,
            links,
            vision,
            countries: Arc::new(countries),
            country_ids,
            geography,
            peace_conference,
            order,
            native,
            orders: spans.measure("overlay.tables.counters.remaining_hour_base.orders", |_| {
                counter_inputs::OrdersIndex::read(frame)
            })?,
            icons: spans.measure("overlay.tables.counters.remaining_hour_base.template_icons", |_| {
                content.template_icons.read(frame)
            })?,
            status: spans.measure("overlay.tables.counters.remaining_hour_base.status", |_| {
                DivisionStatuses::read(frame)
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
        };
        Ok::<_, io::Error>(tables)
    })?;
    tables.divisions = spans.measure("overlay.tables.counters.divisions", |_| {
        division_counters(
            frame,
            &|raw| tables.links.get(&raw).copied(),
            &tables.land(),
        )
    })?;
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
    Ok(HourTables {
        epoch: Arc::new(()),
        base: Arc::new(tables),
        land_combats,
        naval_markers,
        engaged,
    })
}

#[cfg(test)]
pub fn prepare_cached(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    tables: &HourTables,
) -> io::Result<PreparedCounters> {
    prepare_with(inputs, frame, tables, None)
}

/// The retained view memo contains pure values only, never native window history.
pub fn prepare_retained(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    tables: &HourTables,
    resources: u64,
    retained: &mut Preparation,
) -> io::Result<PreparedCounters> {
    retained.bind(inputs, tables, resources);
    prepare_with(inputs, frame, tables, Some(retained))
}

fn prepare_with(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    tables: &HourTables,
    mut retained: Option<&mut Preparation>,
) -> io::Result<PreparedCounters> {
    let mut out = CounterFrame::default();
    let owner = |raw: u32| tables.links.get(&raw).copied();

    // The grouping pass needs each icon's level and the other owner's friendly
    // relation toward the group's first owner. The game's camera groups every icon at one level;
    // a free eye's icons each take their own.
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
        None => bases(inputs, frame, tables, &mut out)?,
    }
    drop(span);
    let span = viewer_foundation::profiling::span("pc_air");
    if inputs.visibility.priorities == PriorityTable::StrategicAir {
        out.native_air = Some(Arc::clone(&tables.air_missions));
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
    // The naval map-icon windows' plan names every task force the hour holds: a counter at sea
    // whose source is gone is destroyed by the presented frame, never here.
    if let Some(naval) = &naval {
        out.naval.forces = Some(naval_windows::Forces {
            recorded: naval.forces.iter().map(|force| force.id).collect(),
            complete: naval.forces_complete,
        });
    }
    out.naval.countries = Some(Arc::clone(&tables.countries));
    let span = viewer_foundation::profiling::span("pc_land");
    let prepared_land = match &naval {
        Some(naval) => Some(land_counters(
            inputs,
            &tables.divisions,
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
    land_combats(inputs, &tables.land_combats, &mut out)?;
    // Combat and result passes use their own presentation flags and do not read the
    // peace-conference flag, so they remain outside the unit-source gate.
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
        &tables.naval_markers,
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

/// Per-hour icon state. Only the columns' positions and their screen layout depend on the
/// fraction; bases, port items and combat icons retain their batches.
pub struct PreparedCounters {
    pub fixed: CounterFrame,
    land: Option<PreparedLand>,
    /// The last [`PreparedCounters::draw`]'s output, rebuilt in place each fraction.
    drawn: DrawnCounters,
}

/// One fraction's counters and report, kept so a redraw allocates nothing.
#[derive(Default)]
struct DrawnCounters {
    batch: Batch,
    /// The naval combat icons, which draw with the combat icons.
    combats: Batch,
    report: CounterReport,
    /// The owners whose flag a drawn land counter lacked, before their tags are looked up.
    missing_owners: Vec<u32>,
    /// The later land branch, absorbed after the preceding counters.
    land_report: CounterReport,
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

    /// A detached flag adapter lets another owner read the same countries while the
    /// counter batches and reports are mutably drawn. Atlas calls still follow draw order.
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

    /// Lays out the counters for this fraction, presented at `time` seconds on the view's own
    /// presentation clock (the one the view's other animated content runs on: the window's clock,
    /// a capture's `--animation-time` and `--animation-step`, a dump's `--presentation-time`),
    /// which the naval map-icon windows tick on ([`naval_windows::present`]);
    /// [`PreparedCounters::drawn`] reads the result.
    #[allow(clippy::too_many_arguments)] // Separate mutable state for the host's composed icon views.
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
            fixed.naval.countries.as_deref(),
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
            Some(land) => land.draw(inputs, batch, report, missing_owners, &mut at_sea)?,
            None => Vec::new(),
        };
        drop(span);
        // The presented frame runs the naval windows' lifecycle once, on this view's windows.
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
                naval,
                &mut fixed.bases,
                batch,
                combats,
                report,
            )?;
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

    /// Parallel draw with serial atlas decisions and report merge order.
    #[allow(clippy::too_many_arguments)] // Separate mutable state for the host's composed icon views.
    pub fn draw_branched(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        workers: &viewer_foundation::parallel::Workers,
        owners: viewer_foundation::profiling::FrameOwners,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<Option<viewer_foundation::profiling::OwnerSample>> {
        use viewer_foundation::map::request_order::{Preceding, following};
        let Self { fixed, land, drawn } = self;
        let DrawnCounters {
            batch,
            combats,
            report,
            missing_owners,
            land_report,
        } = drawn;
        fixed.pings.clear();
        fixed.bases.clear();
        fixed.combats.clear();
        batch.clear();
        combats.clear();
        report.assign_all_but_flags(&fixed.report);
        missing_owners.clear();
        let mut at_sea = Vec::new();
        let preceding = Arc::new(Preceding::default());
        let countries = fixed.naval.countries.as_deref();
        let others = {
            let preceding = Arc::clone(&preceding);
            let native_pings = &mut fixed.native_pings;
            let ping_batch = &mut fixed.pings;
            let native_bases = &mut fixed.native_bases;
            let base_batch = &mut fixed.bases;
            let native_land_combats = &mut fixed.native_land_combats;
            let combat_batch = &mut fixed.combats;
            let report = &mut *report;
            move || {
                // The land counters' undecided flag keys wait until this branch has finished.
                let _finished = preceding.finishing();
                owners.measure("counters_branch", || {
                    let span = viewer_foundation::profiling::span("dc_pings");
                    native_pings.draw(inputs, pings, time, countries, ping_batch, report);
                    drop(span);
                    let span = viewer_foundation::profiling::span("dc_bases");
                    native_bases.draw(inputs, bases, time, countries, base_batch, report);
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
        let land_counters = {
            let land = &mut *land;
            let batch = &mut *batch;
            let land_report = &mut *land_report;
            let missing_owners = &mut *missing_owners;
            let at_sea = &mut at_sea;
            move || {
                following(&preceding, || {
                    let _span = viewer_foundation::profiling::span("dc_land");
                    match land.as_mut() {
                        Some(land) => land.draw(inputs, batch, land_report, missing_owners, at_sea),
                        None => Ok(Vec::new()),
                    }
                })
            }
        };
        let ((_, sample), columns) = branches(workers, others, land_counters);
        // The land counters drew after the other three in the serial order, failing or not.
        report.absorb_later(land_report);
        let columns = columns?;
        // The presented frame runs the naval windows' lifecycle once, on this view's windows.
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
                naval,
                &mut fixed.bases,
                batch,
                combats,
                report,
            )?;
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
        Ok(sample)
    }

    /// The last [`PreparedCounters::draw`]'s batch and report.
    pub fn drawn(&self) -> (&Batch, &CounterReport) {
        (&self.drawn.batch, &self.drawn.report)
    }

    /// The last [`PreparedCounters::draw`]'s naval combat icons, drawn with the combat icons.
    pub fn drawn_combats(&self) -> &Batch {
        &self.drawn.combats
    }
}

/// Starts `first` before `second` can wait for it, even without a helper.
fn branches<A: Send, B: Send>(
    workers: &viewer_foundation::parallel::Workers,
    first: impl FnOnce() -> A + Send,
    second: impl FnOnce() -> B + Send,
) -> (A, B) {
    if workers.threads() == 1 {
        let first = first();
        return (first, second());
    }
    let first = std::sync::Mutex::new(Some(first));
    let second = std::sync::Mutex::new(Some(second));
    let firsts = std::sync::Mutex::new(None);
    let seconds = std::sync::Mutex::new(None);
    workers.run(2, &|at| match at {
        0 => {
            let first = first.lock().unwrap().take().expect("each task runs once");
            *firsts.lock().unwrap() = Some(first());
        }
        _ => {
            let second = second.lock().unwrap().take().expect("each task runs once");
            *seconds.lock().unwrap() = Some(second());
        }
    });
    (
        firsts.into_inner().unwrap().expect("the first task ran"),
        seconds.into_inner().unwrap().expect("the second task ran"),
    )
}

/// What the grouping pass needs about the map and the camera this hour.
struct Geography<'a> {
    /// Province to state; a sea or lake province is absent.
    state_of: &'a BTreeMap<u32, u32>,
    extents: &'a Extents,
}

/// The flag's ruling ideology group of every country slot, including refusals.
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

/// Draws one `.gui` child at its declared offset inside a window.
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

/// A quarter-turn swaps a bar's drawable dimensions. Preserve the authored
/// position and centre adjustment when forming its rectangle.
pub fn turned_rect(child: &Child, size: [f32; 2], origin: [f32; 2]) -> [f32; 4] {
    let (width, height) = (size[0] * child.scale, size[1] * child.scale);
    [
        origin[0] + child.at[0],
        origin[1] + child.at[1] + (height - width) * 0.5,
        height,
        width,
    ]
}

/// Draws one counter bar from its left edge, [`counter_inputs::bar_pixels`] long.
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

/// Lays one of a window's text boxes out at its declared offset.
fn draw_text(
    batch: &mut Batch,
    font: &text::Font,
    child: &TextChild,
    origin: [f32; 2],
    value: &str,
    color: [f32; 4],
) {
    // A text box `mapicons.gui` does not declare has no width and draws nothing.
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

/// The per-frame country tables every counter reads.
#[derive(Clone, Default)]
struct Countries {
    /// Canonical slot to tag.
    tags: BTreeMap<u32, String>,
    /// Canonical slot to its cosmetic tag and exact original-or-own flag tag.
    flag_tags: BTreeMap<u32, FlagTags>,
    /// Canonical slot to its side against the perspective.
    sides: BTreeMap<u32, Allegiance>,
    /// Canonical slot to its flag's ruling group token or field-local refusal.
    groups: BTreeMap<u32, Result<Option<String>, String>>,
    /// Each country's current UI colour and whose colours it takes.
    ui_colors: UiColors,
}

/// The current UI colours (`CountryAppearance.ui_*`) by canonical slot, and
/// the subject-colour chains the game follows.
#[derive(Clone, Debug, Default)]
struct UiColors {
    colors: BTreeMap<u32, [f32; 4]>,
    inheritance: viewer_foundation::map::color_inheritance::Inheritance,
    /// Why a country without a row has none: the table's state this hour.
    gap: String,
    /// Why the hour gives no colour at all, when its subject chains cannot be read.
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
    /// Current UI colour for the country slot, following the applicable subject chain.
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

    /// Decide the hour's candidates at every counter size, off the frame path.
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

    /// Decide every candidate before its first frame draws.
    fn flag_now(&self, flags: &LazyFlags, slot: u32) -> Option<(usize, [f32; 4])> {
        let keys = self.flag_keys(slot)?;
        flags.acquire(&keys.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// Which side a slot is on; an unrecorded country is [`Allegiance::Other`].
    fn side(&self, slot: u32) -> Allegiance {
        self.sides.get(&slot).copied().unwrap_or(Allegiance::Other)
    }

    /// The flag file keys of a slot's current flag, in the engine's order.
    fn flag_keys(&self, slot: u32) -> Option<Vec<String>> {
        let (cosmetic, original) = self.flag_tags.get(&slot)?;
        let cosmetic = cosmetic.as_ref().ok()?;
        let group = self.groups.get(&slot)?.as_ref().ok()?;
        let keys = installed_content_country_names::country_names::flag_keys(
            original.as_deref().ok(),
            cosmetic,
            group.as_deref(),
        );
        Some(keys)
    }

    /// Only reached after no known candidate supplies this drawn counter's flag.
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

/// Detailed flags use a positive expeditionary owner; otherwise use the roster
/// owner. Coarse flags always use the roster owner. Controllers select neither.
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

/// The grouping pass's kind for a naval counter.
const NAVAL_KIND: u32 = 1;

/// The one-based `count_bg` frame the game writes for a coarse counter,
/// as the zero-based strip index this pass draws.
const COARSE_PLATE_FRAME: u32 = 2;

/// One counter's coarse window: the three children the game leaves visible, the plate
/// tint the kind arm already set, and the icon's child keys.
struct CoarseCounter<'a> {
    plate: &'a Child,
    large_flag: &'a Child,
    count_box: &'a TextChild,
    tint: [f32; 4],
    owner: u32,
    count: &'a str,
    /// `(plate, flag, count)` child keys: the children's declaration indices,
    /// which the caller ranks into the icon's slots ([`rank_children`]).
    keys: (u32, u32, u32),
}

/// the coarse counter: `count_bg` keeps its allegiance tint and
/// takes frame 3, `large_flag` shows the owner's medium flag and `count_txt` the
/// member count (the literal `?` behind the hidden byte). Every other child the
/// land and naval arms wrote is hidden again; `low_supply` and `counter_ideology`
/// are the two the coarse pass does not touch.
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
    } = coarse;
    let assets = inputs.assets;
    batch.at_layer(u64::from(keys.0));
    if let Some(sheet) = assets.sheet(&plate.sprite) {
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

/// Ranks the declaration indices the quads from `first` on carry as their layers into the
/// icon's child slots, so the children draw in `mapicons.gui` order under `priority`.
///
/// Every child kind has a slot of its own; sharing one would let two children's quads
/// reorder, so an icon that would need more slots than the key holds is refused.
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

/// Draws one owner's flag cell at a counter's `flag` child.
///
/// The flag is the only child whose texture `interface/*.gfx` does not name;
/// see [`LazyFlags`]. A tag with no flag at all simply draws none.
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
