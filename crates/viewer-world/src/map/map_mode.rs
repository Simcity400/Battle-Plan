//! The game's map modes and what each draws in the political fill's two channels.
//!
//! The game keeps one gradient-border layer per kind (strategic regions, countries, states,
//! diplomacy, resistance and so on) and a map mode chooses which layer each of the two
//! channels shows, or none ([`Source`]). The country layer has variants (political, factions,
//! players, ideology), so one mode can recolour it while another keeps it under its own layer.
//!
//! | mode | channel 1 | channel 2 |
//! |---|---|---|
//! | political, navy | strategic regions | countries |
//! | air | strategic regions | none |
//! | factions, players, ideology | countries in that variant | none |
//! | resources, raids | countries | none |
//! | states, resistance, compliance, operatives | countries | the mode's layer |
//! | diplomacy | the mode's layer | none |
//! | infrastructure, manpower, terrain | the mode's layer | countries |
//! | supply | the supply-border layer, refused | the supply layer, refused |
//! | scripted | the bottom layer | the top layer |
//!
//! | mode | regions | colour | thickness, constant gradient, camera override / outline cutoff |
//! |---|---|---|---|
//! | political | controller | the subject-resolved recorded colour | area rule; country defines |
//! | air | strategic region, land and water | the viewing country's stored superiority weights, or regional active-mission counters at zero weights, through the content's strategic-air colours | strategic-region thickness and channel constants |
//! | navy | strategic region, sea only | the content's neutral strategic-navy tint, with the viewing country's naval control and gaining-side FX | strategic-region thickness and channel constants |
//! | factions | controller | the controller's faction colour at `FACTIONS_MEMBER_TRANSPARENCY`, else `FACTIONS_COLOR_NOT_MEMBER` | area rule; faction defines |
//! | players | controller | the subject-resolved colour, at `PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY` unless a player holds it | area rule; country defines |
//! | ideology | controller | the ruling party's ideology group colour at `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY`, else the subject-resolved colour | area rule; country defines |
//! | states | state | the state's live category's colour; a category without one: the fixed blue below | `THICKNESS_STATE`; state defines |
//! | diplomacy | controller | relation to the selected country, in the fixed colours below | `THICKNESS_DIPLOMACY`; diplomacy defines |
//! | resistance, compliance | the selected country's controlled states | the level through the content's colour list, or `OCCUPATION_RESISTANCE_NON_INITIALIZED_COLOR` without an occupier | `THICKNESS_RESISTANCE`, gradient 241; camera 0 / resistance cutoff |
//! | operatives | each state with positive strength in one of the selected country's networks, while the network's target controls it | the strength's share of 100% between the `MAP_MODE_INTEL_NETWORK_STRENGTH` low and high colours, or the quiet pair for a quiet subnetwork | `THICKNESS_STATE`, gradient 241; camera 0 / state cutoff |
//! | terrain | each province | its terrain category's colour ([`terrain`]) | zero, gradient 127; terrain camera / state cutoff |
//! | manpower | state | the population over `MAP_MODE_MANPOWER_RANGE_MAX`, blended between its two colours | `THICKNESS_STATE`, gradient 241; camera 0 / state cutoff |
//! | infrastructure | state | the infrastructure level over its maximum, blended between `MAP_MODE_INFRA_RANGE_COLOR_FROM` and `_TO`; no maximum: opaque black ([`infrastructure`]) | `THICKNESS_STATE`, gradient 241; camera 0 / state cutoff |
//!
//! In the players mode the selected country pulses and its fill factor is
//! `SELECTED_COUNTRY_HIGHLIGHT_THICKNESS_MULT`. The selected country is the viewing country:
//! the viewer has no separate diplomatic selection. Colours pack each component as the low
//! byte of `trunc(c * 255)`, alpha included; the alpha is the channel's opacity.
//!
//! A layer the viewer does not draw leaves its channel empty and is refused by name in
//! [`ModeFill::warnings`], as are the overlays a mode adds beyond its layers. What each mode
//! shows of the map's other layers (counters, icons, labels, fog, borders) is [`visibility`].
//! Air and Navy regional entries, their grids and hover text are viewer counter products;
//! selection, assignment highlights and ranges stay local to each view. Air-supply regional
//! summaries alone lack an exact acquisition within the recording cost rules and are refused
//! by name. The ordinary Air province hover contains detection, selected-wing assignment
//! feedback and the shared mine summary; the received-damage grids belong to the Air view.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

use installed_content_core::{
    defines::Defines, map::MapData, map::ProvinceKind, scripted_map_modes::GameLayer,
    scripted_map_modes::LayerType, scripted_map_modes::MapModes, scripted_map_modes::MapText,
};
use replay_data::{
    rows::{TableId, tables},
    scope::{self, CaptureMode},
};
use replay_frame::frame::DecodedFrame;
#[cfg(test)]
use viewer_foundation::map::color_inheritance::slot_of;
use viewer_foundation::map::{
    color_inheritance::active_subject,
    directed_wars,
    tables::{Available, core, rows},
};

#[cfg(test)]
use super::political::{color_source, inherited_colors};

#[cfg(test)]
mod colour_oracle;
mod colour_script;
mod country_fold;
mod membership;
pub mod resolved;
#[cfg(test)]
mod scripted_oracle;
mod state_fold;
#[cfg(test)]
mod stock_oracle;
mod style_fold;
pub use colour_script::ColourState;
pub mod infrastructure;
pub mod strategic;
pub mod terrain;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The picker's own modes, in the game's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stock {
    Default,
    StrategicAir,
    StrategicNavy,
    Operatives,
    States,
    Supply,
    Resistance,
    Compliance,
    Resources,
    Diplomacy,
    Factions,
    Players,
    Infrastructure,
    Manpower,
    Ideology,
    Terrain,
    Raids,
}

impl Stock {
    pub const ALL: [Self; 17] = [
        Self::Default,
        Self::StrategicAir,
        Self::StrategicNavy,
        Self::Operatives,
        Self::States,
        Self::Supply,
        Self::Resistance,
        Self::Compliance,
        Self::Resources,
        Self::Diplomacy,
        Self::Factions,
        Self::Players,
        Self::Infrastructure,
        Self::Manpower,
        Self::Ideology,
        Self::Terrain,
        Self::Raids,
    ];

    /// The protocol token.
    pub fn token(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::StrategicAir => "strategic_air",
            Self::StrategicNavy => "strategic_navy",
            Self::Operatives => "operatives",
            Self::States => "states",
            Self::Supply => "supply",
            Self::Resistance => "resistance",
            Self::Compliance => "compliance",
            Self::Resources => "resources",
            Self::Diplomacy => "diplomacy",
            Self::Factions => "factions",
            Self::Players => "players",
            Self::Infrastructure => "infrastructure",
            Self::Manpower => "manpower",
            Self::Ideology => "ideology",
            Self::Terrain => "terrain",
            Self::Raids => "raids",
        }
    }

    /// The localisation key of the mode's tooltip; `_NAME` and `_DESCRIPTION` follow it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Default => "MAPMODE_DEFAULT",
            Self::StrategicAir => "MAPMODE_STRATEGIC_AIR",
            Self::StrategicNavy => "MAPMODE_STRATEGIC_NAVY",
            Self::Operatives => "MAPMODE_OPERATIVES",
            Self::States => "MAPMODE_STATES",
            Self::Supply => "MAPMODE_SUPPLY_MAP_MODE",
            Self::Resistance => "MAPMODE_RESISTANCE",
            Self::Compliance => "MAPMODE_COMPLIANCE",
            Self::Resources => "MAPMODE_RESOURCES",
            Self::Diplomacy => "MAPMODE_DIPLOMACY",
            Self::Factions => "MAPMODE_FACTIONS",
            Self::Players => "MAPMODE_PLAYERS",
            Self::Infrastructure => "MAPMODE_INFRASTRUCTURE",
            Self::Manpower => "MAPMODE_MANPOWER",
            Self::Ideology => "MAPMODE_IDEOLOGY",
            Self::Terrain => "MAPMODE_TERRAIN",
            Self::Raids => "MAPMODE_RAIDS",
        }
    }

    /// Why a recording can never draw this mode, for modes the picker offers disabled.
    pub fn unavailable(self) -> Option<&'static str> {
        match self {
            Self::Supply => Some(SUPPLY_UNAVAILABLE),
            _ => None,
        }
    }
}

impl Stock {
    /// The recorded tables this mode's own layer and icons read beyond the political map's.
    pub fn tables(self) -> &'static [TableId] {
        use TableId::*;
        match self {
            Self::Default | Self::States | Self::Manpower | Self::Terrain | Self::Supply => &[],
            Self::Players => &[Players],
            Self::Ideology => &[Politics, PoliticalParties],
            Self::Factions => &[Factions, FactionMembers],
            Self::Diplomacy => &[Capitulation, Subjects, FactionMembers, RelationOpinions],
            Self::Resistance | Self::Compliance => &[StateResistance],
            Self::Operatives => &[IntelNetworks],
            Self::Infrastructure => &[StateBuildings, ResearchTechnologies, TechnologyDefinitions],
            Self::StrategicAir => &[RaidAirScores, CountryCounterInputs],
            Self::StrategicNavy => &[
                RaidAirScores,
                CountryCounterInputs,
                NavalSupremacy,
                RelationTuples,
            ],
            Self::Resources => &[StateResources],
            Self::Raids => &[RaidTargetStatuses],
        }
    }

    pub fn unavailable_in(self, mode: CaptureMode) -> Option<String> {
        if let Some(reason) = self.unavailable() {
            return Some(reason.to_owned());
        }
        let missing = self
            .tables()
            .iter()
            .find(|table| !scope::records(mode, **table))?;
        Some(format!(
            "Not in this recording: a {mode} recording leaves out {missing:?}, which this map mode reads."
        ))
    }
}

impl MapMode {
    pub fn unavailable_in(&self, mode: CaptureMode) -> Option<String> {
        match self {
            Self::Stock(stock) => stock.unavailable_in(mode),
            Self::Scripted(_) => (mode != CaptureMode::Full).then(|| {
                format!(
                    "Not in this recording: content map modes evaluate scripts over data a {mode} recording leaves out."
                )
            }),
        }
    }
}

/// Why the supply mode is unavailable: its province colours, supply borders, flows and
/// tooltips are computed from the supply network's per-province sources and per-hour node
/// amounts, which no recording holds.
pub const SUPPLY_UNAVAILABLE: &str = "Not available in replays: the game computes this map mode from the supply network's per-province sources and hourly hub amounts, which recordings do not hold.";

/// A view's map mode.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MapMode {
    Stock(Stock),
    /// A content mode by its declared name.
    Scripted(String),
}

impl Default for MapMode {
    fn default() -> Self {
        Self::Stock(Stock::Default)
    }
}

/// The token prefix of a scripted mode.
pub const SCRIPTED_PREFIX: &str = "scripted:";

impl MapMode {
    /// A protocol token: a stock token, or `scripted:` and a declared name.
    pub fn parse(token: &str) -> Option<Self> {
        if let Some(name) = token.strip_prefix(SCRIPTED_PREFIX) {
            return (!name.is_empty()).then(|| Self::Scripted(name.to_owned()));
        }
        Stock::ALL
            .into_iter()
            .find(|stock| stock.token() == token)
            .map(Self::Stock)
    }

    pub fn token(&self) -> String {
        match self {
            Self::Stock(stock) => stock.token().to_owned(),
            Self::Scripted(name) => format!("{SCRIPTED_PREFIX}{name}"),
        }
    }
}

/// One region's presentation in the country channel. Floats compare by their bits, so hours
/// share a fill only when it is bit-identical.
pub use map_display_data::landscape::{Draw, ModeFill, ModeLayer, Source, Style};
pub const POLITICAL: [Source; 2] = [Source::Strategic, Source::Country];

/// A colour define of four components, as f32.
fn color4(defines: &Defines, section: &str, name: &str) -> io::Result<[f32; 4]> {
    match defines.numbers(section, name)?.as_slice() {
        [r, g, b, a, ..] => Ok([*r as f32, *g as f32, *b as f32, *a as f32]),
        _ => Err(invalid(format!(
            "NDefines.{section}.{name} is not a four-component colour"
        ))),
    }
}

fn number(defines: &Defines, section: &str, name: &str) -> io::Result<f32> {
    Ok(defines.number(section, name)? as f32)
}

/// A content colour list: rows of a threshold and four components.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorList(pub Vec<f32>);

/// The difference below which two thresholds count as one.
const COLOR_LIST_EPSILON: f32 = 0.01;

impl ColorList {
    /// The colour of `value`: interpolated between the two rows around it, the alpha too.
    ///
    /// The first row at or above the value (from the second on, or the last) ends the
    /// interval; a gap under [`COLOR_LIST_EPSILON`] or a value below the interval takes its
    /// first row. A list that is not whole rows of five, or shorter than two rows, is white.
    pub fn color(&self, value: f32) -> [f32; 4] {
        let list = &self.0;
        let n = list.len();
        if n < 10 || !n.is_multiple_of(5) {
            return [1.0; 4];
        }
        let mut t = 0.0_f32;
        let mut first = 0;
        let last = n - 5;
        let mut at = 5;
        while at <= last {
            if list[at] >= value || at == last {
                let gap = list[at] - list[at - 5];
                first = at - 5;
                if gap >= COLOR_LIST_EPSILON {
                    let fraction = (value - list[at - 5]) / gap;
                    if fraction >= 0.0 {
                        t = fraction.min(1.0);
                    }
                }
                break;
            }
            at += 5;
        }
        let from = &list[first + 1..first + 5];
        let to = &list[first + 6..first + 10];
        std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
    }
}

/// The colour defines the modes read.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeDefines {
    factions_not_member: [f32; 4],
    factions_member_alpha: f32,
    players_other_alpha: f32,
    selected_fill: f32,
    ideology_alpha: f32,
    /// Camera override and outline cutoff per channel use.
    country_channel: [f32; 2],
    factions_channel: [f32; 2],
    state_channel: [f32; 2],
    diplomacy_channel: [f32; 2],
    resistance_cutoff: f32,
    state_thickness: f32,
    diplomacy_thickness: f32,
    resistance_thickness: f32,
    /// `GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH`: a scripted layer's width with `thickness = yes`.
    scripted_thickness: f32,
    very_good_opinion: i32,
    very_bad_opinion: i32,
    resistance_colors: ColorList,
    compliance_colors: ColorList,
    resistance_uninitialised: [f32; 4],
    /// The population that takes the manpower mode's last colour, and its two colours.
    manpower_max: f32,
    manpower_colors: [[f32; 4]; 2],
    /// The operatives mode's low and high network colours, and the quiet pair.
    network_colors: [[f32; 4]; 2],
    quiet_network_colors: [[f32; 4]; 2],
    /// The infrastructure mode's two colours.
    infrastructure_colors: [[f32; 4]; 2],
    /// `UI_CONFIGURABLE_SLOT_FROM` and `_TO`: the picker slots the user can reassign.
    pub configurable_slots: [i32; 2],
    pub terrain: terrain::Defines,
}

impl ModeDefines {
    /// `VERY_GOOD_OPINION` and `VERY_BAD_OPINION`, which the diplomacy mode's lines test.
    pub fn opinion_thresholds(&self) -> [i32; 2] {
        [self.very_good_opinion, self.very_bad_opinion]
    }

    pub fn load(defines: &Defines) -> io::Result<Self> {
        let graphics = |name: &str| number(defines, "NGraphics", name);
        let colors = |name: &str| -> io::Result<ColorList> {
            Ok(ColorList(
                defines
                    .numbers("NGraphics", name)?
                    .into_iter()
                    .map(|value| value as f32)
                    .collect(),
            ))
        };
        Ok(Self {
            factions_not_member: color4(defines, "NMapMode", "FACTIONS_COLOR_NOT_MEMBER")?,
            factions_member_alpha: number(defines, "NMapMode", "FACTIONS_MEMBER_TRANSPARENCY")?,
            players_other_alpha: number(
                defines,
                "NMapMode",
                "PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY",
            )?,
            selected_fill: number(
                defines,
                "NMapMode",
                "SELECTED_COUNTRY_HIGHLIGHT_THICKNESS_MULT",
            )?,
            ideology_alpha: number(defines, "NMapMode", "MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY")?,
            country_channel: [
                graphics("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_COUNTRY")?,
                graphics("GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY")?,
            ],
            factions_channel: [
                graphics("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_FACTIONS")?,
                graphics("GRADIENT_BORDERS_OUTLINE_CUTOFF_FACTIONS")?,
            ],
            state_channel: [
                graphics("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_STATE")?,
                graphics("GRADIENT_BORDERS_OUTLINE_CUTOFF_STATE")?,
            ],
            diplomacy_channel: [
                graphics("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_DIPLOMACY")?,
                graphics("GRADIENT_BORDERS_OUTLINE_CUTOFF_DIPLOMACY")?,
            ],
            resistance_cutoff: graphics("GRADIENT_BORDERS_OUTLINE_CUTOFF_RESISTANCE")?,
            state_thickness: graphics("GRADIENT_BORDERS_THICKNESS_STATE")?,
            diplomacy_thickness: graphics("GRADIENT_BORDERS_THICKNESS_DIPLOMACY")?,
            resistance_thickness: graphics("GRADIENT_BORDERS_THICKNESS_RESISTANCE")?,
            scripted_thickness: graphics("GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH")?,
            very_good_opinion: defines.int32("NDiplomacy", "VERY_GOOD_OPINION")?,
            very_bad_opinion: defines.int32("NDiplomacy", "VERY_BAD_OPINION")?,
            resistance_colors: colors("OCCUPATION_RESISTANCE_MAP_MODE_COLORS")?,
            compliance_colors: colors("OCCUPATION_COMPLIANCE_MAP_MODE_COLORS")?,
            resistance_uninitialised: color4(
                defines,
                "NGraphics",
                "OCCUPATION_RESISTANCE_NON_INITIALIZED_COLOR",
            )?,
            manpower_max: number(defines, "NMapMode", "MAP_MODE_MANPOWER_RANGE_MAX")?,
            manpower_colors: [
                color4(defines, "NMapMode", "MAP_MODE_MANPOWER_RANGE_COLOR_FROM")?,
                color4(defines, "NMapMode", "MAP_MODE_MANPOWER_RANGE_COLOR_TO")?,
            ],
            network_colors: [
                color4(
                    defines,
                    "NMapMode",
                    "MAP_MODE_INTEL_NETWORK_STRENGTH_COLOR_LOW",
                )?,
                color4(
                    defines,
                    "NMapMode",
                    "MAP_MODE_INTEL_NETWORK_STRENGTH_COLOR_HIGH",
                )?,
            ],
            quiet_network_colors: [
                color4(
                    defines,
                    "NMapMode",
                    "MAP_MODE_INTEL_NETWORK_STRENGTH_QUIET_COLOR_LOW",
                )?,
                color4(
                    defines,
                    "NMapMode",
                    "MAP_MODE_INTEL_NETWORK_STRENGTH_QUIET_COLOR_HIGH",
                )?,
            ],
            infrastructure_colors: [
                color4(defines, "NMapMode", "MAP_MODE_INFRA_RANGE_COLOR_FROM")?,
                color4(defines, "NMapMode", "MAP_MODE_INFRA_RANGE_COLOR_TO")?,
            ],
            configurable_slots: [
                defines.int32("NMapMode", "UI_CONFIGURABLE_SLOT_FROM")?,
                defines.int32("NMapMode", "UI_CONFIGURABLE_SLOT_TO")?,
            ],
            terrain: terrain::Defines {
                alpha: [
                    number(defines, "NMapMode", "MAP_MODE_TERRAIN_TRANSPARENCY")?,
                    number(defines, "NMapMode", "MAP_MODE_NAVAL_TERRAIN_TRANSPARENCY")?,
                ],
                channel: [
                    graphics("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_TERRAIN")?,
                    graphics("GRADIENT_BORDERS_OUTLINE_CUTOFF_STATE")?,
                ],
            },
        })
    }
}

/// The diplomacy mode's fixed colours: the selected country, its enemies, its faction,
/// countries it holds a war goal against (or against one of their subjects), and very good
/// and very bad opinions of it.
const DIPLOMACY_SELF: [f32; 4] = [0.1, 0.66, 0.1, 1.0];
const DIPLOMACY_WAR: [f32; 4] = [0.7, 0.0, 0.0, 1.0];
const DIPLOMACY_FACTION: [f32; 4] = [0.035, 0.426, 0.91, 1.0];
const DIPLOMACY_WAR_GOAL: [f32; 4] = [1.0, 0.35, 0.0, 1.0];
const DIPLOMACY_GOOD: [f32; 4] = [0.0, 0.4, 0.0, 1.0];
const DIPLOMACY_BAD: [f32; 4] = [1.0, 0.64, 0.0, 1.0];
/// The states mode's colour for a state whose category has no colour.
const STATE_WITHOUT_CATEGORY: [f32; 4] = [0.0, 0.0, 1.0, 0.25];
/// Resistance and compliance levels are fixed5 percentages.
const RESISTANCE_SCALE: f32 = 100_000.0;
/// The constant gradient the resistance, compliance and manpower layers write.
const LEVEL_GRADIENT: u8 = 241;

/// Each component as the low byte of `trunc(c * 255)`.
pub fn pack(color: [f32; 4]) -> [u8; 4] {
    color.map(|component| (component * 255.0) as i32 as u8)
}

/// Immutable inputs of every mode for one content stack.
pub struct Content {
    _credit: map_display_data::credit::Credit,
    id: u64,
    memberships: std::sync::Mutex<membership::Memberships>,
    styles: std::sync::Mutex<style_fold::Styles>,
    scripts: std::sync::Mutex<colour_script::Fold>,
    pub defines: ModeDefines,
    ideologies: Arc<installed_content_core::ideologies::Ideologies>,
    /// Each declared category's colour by folded name: `Ok(None)` for one without a colour.
    category_colors: BTreeMap<Vec<u8>, Result<Option<[f32; 4]>, String>>,
    pub scripted: MapModes,
    /// The content's faction templates, which name a faction without a literal name.
    faction_templates: Result<Arc<installed_content_core::faction_templates::Catalog>, String>,
    /// The recording's language, which a faction's stored literal name is written in.
    recorded_language: String,
    /// The terrain mode's layer, built once, or why the content cannot build it.
    terrain: Result<terrain::Built, String>,
    pub infrastructure: infrastructure::Content,
    pub strategic: strategic::Content,
}

impl Content {
    pub fn evict_memberships(&self) {
        self.strategic.evict();
        *self
            .scripts
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Default::default();
        *self
            .styles
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Default::default();
        *self
            .memberships
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Default::default();
    }
    pub fn bind_membership_map(&self, map: &Arc<MapData>) {
        self.strategic.bind(map);
        self.scripts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bind(map);
        self.styles
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bind(map);
        self.memberships
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bind(map);
    }
    pub fn projection_id(&self) -> u64 {
        self.id
    }
    #[allow(clippy::too_many_arguments)] // One argument per content source, read separately by the caller.
    pub fn new(
        defines: &Defines,
        ideologies: Arc<installed_content_core::ideologies::Ideologies>,
        categories: &installed_content_state_categories::state_categories::Database,
        scripted: MapModes,
        faction_templates: Result<Arc<installed_content_core::faction_templates::Catalog>, String>,
        recorded_language: String,
        terrain: Result<terrain::Sources, String>,
        infrastructure: infrastructure::Content,
        strategic: strategic::Content,
    ) -> io::Result<Self> {
        let credit = map_display_data::credit::display_products()
            .admit_item(size_of::<Self>() + 2 * size_of::<usize>() + align_of::<Self>())?;
        let defines = ModeDefines::load(defines)?;
        let terrain = match terrain {
            Ok(sources) => Ok(terrain::build(&sources, &defines.terrain)?),
            Err(reason) => Err(reason),
        };
        let category_colors = categories
            .names
            .iter()
            .map(|name| (name.to_ascii_lowercase(), categories.color(name)))
            .collect();
        Ok(Self {
            _credit: credit,
            id: super::next_projection_id(),
            memberships: Default::default(),
            styles: Default::default(),
            scripts: Default::default(),
            defines,
            ideologies,
            category_colors,
            scripted,
            faction_templates,
            recorded_language,
            terrain,
            infrastructure,
            strategic,
        })
    }

    /// Whose names the map writes in `mode`, far out and close in: the far names show at or
    /// above `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF` and the near ones below it, cross-fading. The
    /// game chooses per mode: faction names in the factions mode, players' names in the players
    /// mode, strategic region names in the air and navy modes (admitted by the local pointer
    /// and strategic-name preference in the name pass), country names otherwise, and none
    /// close in; a scripted mode by its `far_text` and `near_text`, each writing none when it is
    /// not declared.
    pub fn name_layers(&self, mode: &MapMode) -> NameLayers {
        let stock = |far| NameLayers {
            far,
            near: NameLayer::Nothing,
        };
        match mode {
            MapMode::Stock(Stock::Factions) => stock(NameLayer::Factions),
            MapMode::Stock(Stock::Players) => stock(NameLayer::Players),
            MapMode::Stock(Stock::StrategicAir | Stock::StrategicNavy) => {
                stock(NameLayer::StrategicRegions)
            }
            MapMode::Stock(_) => stock(NameLayer::Countries),
            MapMode::Scripted(name) => {
                let declared = self.scripted.get(name);
                NameLayers {
                    far: NameLayer::of(declared.and_then(|mode| mode.far_text)),
                    near: NameLayer::of(declared.and_then(|mode| mode.near_text)),
                }
            }
        }
    }

    /// Whether `mode` names a mode this content declares.
    pub fn knows(&self, mode: &MapMode) -> bool {
        match mode {
            MapMode::Stock(_) => true,
            MapMode::Scripted(name) => self.scripted.get(name).is_some(),
        }
    }
}

/// Whose names a mode writes far out and close in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameLayers {
    pub far: NameLayer,
    pub near: NameLayer,
}

/// Whose names the map writes in a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameLayer {
    /// Each country's own name.
    Countries,
    /// A faction's name across its members; countries outside a faction keep theirs.
    Factions,
    /// No names.
    Nothing,
    /// Each strategic region's name across its connected provinces.
    StrategicRegions,
    /// Each state's name across its provinces.
    States,
    /// Each player's name on the country they hold; other countries carry none.
    Players,
}

impl NameLayer {
    /// A scripted mode's declared `far_text` or `near_text`; undeclared writes none.
    fn of(text: Option<MapText>) -> Self {
        match text {
            Some(MapText::Country) => Self::Countries,
            Some(MapText::Faction) => Self::Factions,
            None | Some(MapText::None) => Self::Nothing,
            Some(MapText::State) => Self::States,
            Some(MapText::Player) => Self::Players,
        }
    }
}

/// Names written across groups of countries instead of each country's own: a faction's across
/// its members, or a player's on the country they hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupLabels {
    /// Each grouped country's canonical slot, with its group and that group's name or why it
    /// has none.
    pub groups: BTreeMap<u32, (u64, Result<String, String>)>,
    /// Whether countries outside every group keep their own names.
    pub others_named: bool,
}

/// The faction labels of one frame, named by the shared faction name rule in `localisation`.
pub fn faction_labels(
    content: &Content,
    frame: &DecodedFrame,
    localisation: &installed_content_localisation::localisation::Localisation,
) -> Result<GroupLabels, String> {
    use dashboard_text::faction_names;
    let factions = complete::<tables::Factions>(frame, "factions")?;
    let members = complete::<tables::FactionMembers>(frame, "factions")?;
    let countries = Countries::read(frame).map_err(|error| error.to_string())?;
    let lexical = faction_names::LexicalNames::new(
        frame.tables[replay_data::rows::TableId::LexicalTokens.index()].clone(),
    );
    let templates = content.faction_templates.as_deref().map_err(Clone::clone);
    let names: BTreeMap<u64, Result<String, String>> = factions
        .iter()
        .map(|row| {
            let template = faction_names::template_spelling(frame, row.template_token);
            let name = faction_names::name(
                &row.name,
                template
                    .as_ref()
                    .map(|value| value.as_deref())
                    .map_err(Clone::clone),
                templates.clone(),
                localisation,
                &content.recorded_language,
                |name| lexical.token_of(name),
            );
            (row.id, name)
        })
        .collect();
    Ok(GroupLabels {
        groups: members
            .iter()
            .filter_map(|row| {
                let slot = countries.slot(row.country)?;
                let name = names
                    .get(&row.faction)
                    .cloned()
                    .unwrap_or_else(|| Err(format!("faction {} has no Factions row", row.faction)));
                Some((slot, (row.faction, name)))
            })
            .collect(),
        others_named: true,
    })
}

/// The players mode's names: each player's name on the country they hold; countries no player
/// holds carry none.
pub fn player_labels(frame: &DecodedFrame) -> Result<GroupLabels, String> {
    let players = complete::<tables::Players>(frame, "players")?;
    let countries = Countries::read(frame).map_err(|error| error.to_string())?;
    let mut held: BTreeMap<u32, Vec<&replay_data::rows::PlayerRow>> = BTreeMap::new();
    for player in players {
        if let Some(slot) = countries.slot(player.country) {
            held.entry(slot).or_default().push(player);
        }
    }
    Ok(GroupLabels {
        groups: held
            .into_iter()
            .map(|(slot, players)| {
                let name = match players.as_slice() {
                    [player] => String::from_utf8(player.name.clone())
                        .map_err(|_| "its player's name is not UTF-8".to_owned()),
                    _ => Err(format!(
                        "{} players hold it, and which name the game writes is not known",
                        players.len()
                    )),
                };
                (slot, (u64::from(slot), name))
            })
            .collect(),
        others_named: false,
    })
}

/// The country tables of one frame, resolved to canonical slots.
struct Countries {
    links: BTreeMap<u32, u32>,
    /// Canonical slot to raw id.
    raw: BTreeMap<u32, u32>,
}

impl Countries {
    fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let links = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let raw = core::<tables::Countries>(frame)?
            .iter()
            .map(|country| (country.canonical_slot, country.id))
            .collect();
        Ok(Self { links, raw })
    }

    fn slot(&self, reference: u32) -> Option<u32> {
        self.links.get(&reference).copied()
    }
}

/// The controller region keys the political fill uses: the controller's slot plus one.
#[cfg(test)]
fn controller_regions(
    map: &MapData,
    frame: &DecodedFrame,
    countries: &Countries,
) -> io::Result<(Vec<u32>, BTreeSet<u32>)> {
    let provinces = core::<tables::Provinces>(frame)?;
    let len = region_table_len(map, provinces.iter().map(|row| row.id));
    let mut region_of = vec![0; len];
    let mut slots = BTreeSet::new();
    for province in provinces {
        let Some(definition) = map.definitions.get(&province.id) else {
            continue;
        };
        // Sea is never in the country layer; a land or lake province joins its controller's.
        if definition.kind == ProvinceKind::Sea || province.controller == 0 {
            continue;
        }
        let slot = slot_of(&countries.links, province.controller, "controller")?;
        region_of[province.id as usize] = slot + 1;
        slots.insert(slot);
    }
    Ok((region_of, slots))
}

fn region_table_len(map: &MapData, ids: impl Iterator<Item = u32>) -> usize {
    let largest = map
        .definitions
        .last_key_value()
        .map(|(&id, _)| id)
        .unwrap_or(0)
        .max(ids.max().unwrap_or(0));
    largest as usize + 1
}

/// The subject-resolved recorded colours, as f32, of the given slots.
#[cfg(test)]
fn subject_colors<'a>(
    map: &MapData,
    frame: &DecodedFrame,
    countries: &Countries,
    slots: impl IntoIterator<Item = &'a u32>,
    warnings: &mut Vec<String>,
) -> io::Result<BTreeMap<u32, [f32; 4]>> {
    let inherited = inherited_colors(map, frame, &countries.links, warnings)?;
    let appearance = rows::<tables::CountryAppearance>(frame)?;
    let recorded = appearance
        .rows()
        .iter()
        .map(|row| {
            (
                row.country,
                [row.red, row.green, row.blue, row.alpha].map(f32::from_bits),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut colors = BTreeMap::new();
    let mut missing = Vec::new();
    for &slot in slots {
        let Some(source) = color_source(slot, &inherited)? else {
            continue;
        };
        match countries.raw.get(&source).and_then(|raw| recorded.get(raw)) {
            Some(&color) => {
                colors.insert(slot, color);
            }
            None => missing.push(slot),
        }
    }
    if !missing.is_empty() {
        warnings.push(format!(
            "{} countries have no recorded colour this hour and take no fill in this map mode",
            missing.len()
        ));
    }
    Ok(colors)
}

fn style(color: [f32; 4]) -> Style {
    Style {
        color: pack(color),
        pulse: false,
        fill: 1.0,
    }
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] = alpha;
    color
}

/// The table a mode needs, or why the hour cannot draw it.
fn complete<'a, T: replay_data::rows::TableSchema>(
    frame: &'a DecodedFrame,
    mode: &str,
) -> Result<&'a [T::Row], String> {
    match rows::<T>(frame).map_err(|error| error.to_string())? {
        Available::Complete(rows) => Ok(rows),
        Available::Truncated { missing, .. } => Err(format!(
            "the {mode} map mode needs {:?}, truncated this hour ({missing} missing rows)",
            T::ID
        )),
        Available::Unavailable { reason } => Err(format!(
            "the {mode} map mode needs {:?}, unavailable this hour: {reason}",
            T::ID
        )),
        Available::NotRecorded { mode: capture } => Err(format!(
            "the {mode} map mode needs {:?}, not recorded in a {capture} recording",
            T::ID
        )),
    }
}

/// What the stock and scripted modes need besides the frame.
pub struct Inputs<'a, 'w> {
    pub content: &'a Content,
    pub map: &'a MapData,
    pub frame: &'a DecodedFrame,
    /// The viewing country's canonical slot, the modes' selected country.
    pub selected: Option<u32>,
    /// The hour's trigger world, which scripted modes' colour scripts run on.
    pub world: Option<&'a dashboard_text::triggers::World<'w>>,
    /// The scripted mode's borders as the previous hour kept them; `None` on a first hour.
    pub colour: Option<&'a ColourState>,
}

/// The country channel `mode` draws this hour.
pub fn fill(mode: &MapMode, inputs: &Inputs) -> io::Result<ModeFill> {
    match mode {
        MapMode::Stock(stock) => stock_fill(*stock, inputs),
        MapMode::Scripted(_) => {
            resolved::project(mode, inputs).and_then(|value| resolved::apply(&value, inputs.colour))
        }
    }
}

fn refusal(warnings: Vec<String>) -> ModeFill {
    ModeFill {
        draw: None,
        warnings,
        colour: None,
    }
}

/// A channel showing a layer the viewer built, or empty when the layer was refused.
fn built(layer: &Option<ModeLayer>, source: Source) -> Source {
    if layer.is_some() {
        source
    } else {
        Source::Empty
    }
}

fn stock_fill(stock: Stock, inputs: &Inputs) -> io::Result<ModeFill> {
    use Source::{Country, Empty, Mode, Strategic};
    let political = inputs.content.defines.country_channel;
    let mut warnings = Vec::new();
    let variant = |variant, warnings: &mut Vec<String>| -> io::Result<Draw> {
        let country = country_variant(variant, inputs, warnings)?;
        Ok(Draw {
            channels: [built(&country, Country), Empty],
            strategic: None,
            country,
            layer: None,
            second: None,
            political,
        })
    };
    let over_countries = |layer: Option<ModeLayer>| Draw {
        channels: [Country, built(&layer, Mode)],
        strategic: None,
        country: None,
        layer,
        second: None,
        political,
    };
    let draw = match stock {
        Stock::Default => return Ok(ModeFill::default()),
        // The resources mode draws no layer of its own: the countries, under its state
        // resource icons ([`visibility`]).
        Stock::Resources => Draw::of([Country, Empty], political),
        Stock::StrategicNavy | Stock::StrategicAir => {
            let mut draw = Draw::of(
                if stock == Stock::StrategicAir {
                    [Strategic, Empty]
                } else {
                    POLITICAL
                },
                political,
            );
            draw.strategic = Some(
                inputs
                    .content
                    .strategic
                    .layer(stock, inputs, &mut warnings)?,
            );
            if stock == Stock::StrategicAir {
                warnings.push(unbuilt(stock).to_owned());
            }
            draw
        }
        // The raid mode draws no layer of its own: the countries alone. Its raid icons and
        // what it hides are [`visibility`] rules; its source stripes belong to the raid creation
        // window, which a replay has no use for.
        Stock::Raids => Draw::of([Country, Empty], political),
        Stock::Factions => variant(Variant::Factions, &mut warnings)?,
        Stock::Players => variant(Variant::Players, &mut warnings)?,
        Stock::Ideology => variant(Variant::Ideology, &mut warnings)?,
        Stock::States => over_countries(states(inputs, &mut warnings)?),
        Stock::Resistance => over_countries(occupation(false, inputs, &mut warnings)?),
        Stock::Compliance => over_countries(occupation(true, inputs, &mut warnings)?),
        Stock::Operatives => over_countries(operatives(inputs, &mut warnings)?),
        Stock::Diplomacy => {
            let layer = diplomacy(inputs, &mut warnings)?;
            Draw {
                channels: [built(&layer, Mode), Empty],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Manpower => {
            let layer = manpower(inputs, &mut warnings)?;
            Draw {
                channels: [built(&layer, Mode), Country],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Terrain => {
            let layer = match &inputs.content.terrain {
                Ok(built) => {
                    warnings.extend(built.warnings.iter().cloned());
                    Some(built.layer.clone())
                }
                Err(why) => {
                    warnings.push(format!(
                        "Terrain map mode: its layer is not drawn and the countries show alone: {why}"
                    ));
                    None
                }
            };
            Draw {
                channels: [built(&layer, Mode), Country],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Infrastructure => {
            let layer = inputs.content.infrastructure.layer(
                &inputs.content.defines,
                inputs,
                &mut warnings,
            )?;
            Draw {
                channels: [built(&layer, Mode), Country],
                strategic: None,
                country: None,
                layer,
                second: None,
                political,
            }
        }
        Stock::Supply => {
            warnings.push(unbuilt(stock).to_owned());
            Draw::of([Empty, Empty], political)
        }
    };
    Ok(ModeFill {
        draw: Some(draw),
        warnings,
        colour: None,
    })
}

/// Why a picker mode keeps the political country channel.
fn unbuilt(stock: Stock) -> &'static str {
    match stock {
        Stock::Supply => {
            "Supply map mode: its province supply and supply-border layers need each province's distributed supply, which is not recorded; both channels stay empty and the supply hub icons, flow arrows and supply tooltips are not drawn"
        }
        Stock::StrategicAir => {
            "Air-supply regional summary operands are unavailable: exact consumer membership is rebuilt hourly; an extra full scan or synchronous per-consumer observation violates the recording cost rules"
        }
        _ => "This map mode's layer is not drawn",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Country,
    Factions,
    Players,
    Ideology,
}

/// The country layer's variants: the political regions in another colouring.
fn country_variant(
    variant: Variant,
    inputs: &Inputs,
    warnings: &mut Vec<String>,
) -> io::Result<Option<ModeLayer>> {
    inputs
        .content
        .styles
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .country(variant, inputs, warnings)
}

/// Drops provinces of regions without a style, so a refused region draws nothing.
fn finish(
    mut region_of: Vec<u32>,
    styles: BTreeMap<u32, Style>,
    thickness: Option<f32>,
    gradient: Option<u8>,
    channel: [f32; 2],
) -> io::Result<ModeLayer> {
    for key in &mut region_of {
        if *key != 0 && !styles.contains_key(key) {
            *key = 0;
        }
    }
    Ok(ModeLayer {
        region_of: region_of.into(),
        styles: map_display_data::value_index::ValueIndex::try_from_iter(styles)?,
        thickness,
        gradient,
        channel,
    })
}

/// Every state, coloured by its live category.
fn states(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let defines = &inputs.content.defines;
    let states = match complete::<tables::States>(inputs.frame, "states") {
        Ok(states) => states,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let Some(styles) = inputs
        .content
        .styles
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .states(inputs, states, warnings)?
    else {
        return Ok(None);
    };
    Ok(Some(ModeLayer {
        region_of: membership::plane(inputs, 1, &styles)?,
        styles,
        thickness: Some(defines.state_thickness),
        gradient: None,
        channel: defines.state_channel,
    }))
}

/// One country's relation to the selected country, as the diplomacy mode reads it.
#[derive(Clone, Copy, Debug, Default)]
struct Relation {
    selected: bool,
    at_war: bool,
    same_faction: bool,
    /// The selected country holds a war goal against it or one of its subjects; `None` when
    /// the recording cannot say.
    war_goal: Option<bool>,
    /// Its opinion of the selected country.
    opinion: i32,
}

impl Relation {
    /// The first relation that holds, in this order; `Ok(None)` leaves the country unfilled
    /// and `Err` means the war-goal test was reached without its operand.
    fn color(self, defines: &ModeDefines) -> Result<Option<[f32; 4]>, ()> {
        Ok(Some(if self.selected {
            DIPLOMACY_SELF
        } else if self.at_war {
            DIPLOMACY_WAR
        } else if self.same_faction {
            DIPLOMACY_FACTION
        } else if self.war_goal.ok_or(())? {
            DIPLOMACY_WAR_GOAL
        } else if self.opinion > defines.very_good_opinion {
            DIPLOMACY_GOOD
        } else if self.opinion < defines.very_bad_opinion {
            DIPLOMACY_BAD
        } else {
            return Ok(None);
        }))
    }
}

/// Why the diplomacy mode cannot colour a country below war and faction.
const WAR_GOALS_UNRECORDED: &str = "the selected country's owned war goals are not recorded, and the war-goal test comes before the opinions";

/// The slots the selected country's owned war goals mark: each goal's recipient and its
/// overlord, since a goal against a subject marks the overlord too. `Err` names why the
/// recording cannot say.
fn war_goal_slots(
    frame: &DecodedFrame,
    countries: &Countries,
    selected_raw: u32,
) -> Result<BTreeSet<u32>, String> {
    let capitulation = complete::<tables::Capitulation>(frame, "diplomacy")?;
    let row = capitulation
        .iter()
        .find(|row| row.country == selected_raw)
        .ok_or_else(|| "the selected country has no Capitulation row".to_owned())?;
    let recipients = match &row.war_goal_recipients {
        replay_data::rows::Recorded::Value(Some(recipients)) => recipients,
        replay_data::rows::Recorded::Value(None) => {
            return Err("a war goal of the selected country did not read".to_owned());
        }
        replay_data::rows::Recorded::NotRecorded => return Err(WAR_GOALS_UNRECORDED.to_owned()),
    };
    let subjects = complete::<tables::Subjects>(frame, "diplomacy")?;
    let mut marked = BTreeSet::new();
    for &recipient in recipients {
        let slot = countries
            .slot(recipient)
            .ok_or_else(|| format!("war goal recipient {recipient} has no CountryLinks row"))?;
        marked.insert(slot);
        for row in subjects
            .iter()
            .filter(|row| row.subject == recipient && active_subject(row, frame.hour))
        {
            let overlord = countries
                .slot(row.overlord)
                .ok_or_else(|| format!("overlord {} has no CountryLinks row", row.overlord))?;
            marked.insert(overlord);
        }
    }
    Ok(marked)
}

fn diplomacy(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let Inputs {
        content,
        frame,
        selected,
        ..
    } = inputs;
    let defines = &content.defines;
    let Some(selected) = *selected else {
        warnings.push("The diplomacy map mode needs a viewing country".to_owned());
        return Ok(None);
    };
    let countries = Countries::read(frame)?;
    let Some(&selected_raw) = countries.raw.get(&selected) else {
        warnings.push("The diplomacy map mode's viewing country has no Countries row".to_owned());
        return Ok(None);
    };
    let tables = (|| {
        Ok::<_, String>((
            complete::<tables::FactionMembers>(frame, "diplomacy")?,
            complete::<tables::RelationOpinions>(frame, "diplomacy")?,
        ))
    })();
    let (members, opinions) = match tables {
        Ok(tables) => tables,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let (_, slots) = membership::countries(inputs)?;
    let faction_of = members
        .iter()
        .filter_map(|row| Some((countries.slot(row.country)?, row.faction)))
        .collect::<HashMap<_, _>>();
    let wars = directed_wars::Owner::new(frame, selected_raw);
    let war_goals = war_goal_slots(frame, &countries, selected_raw);
    let mut styles = BTreeMap::new();
    let mut refused = BTreeMap::<String, usize>::new();
    for &slot in &slots {
        let Some(&raw) = countries.raw.get(&slot) else {
            continue;
        };
        let opinion = if slot == selected {
            0
        } else {
            opinions
                .iter()
                .find(|row| {
                    countries.slot(row.owner_country) == Some(slot)
                        && countries.slot(row.target_country) == Some(selected)
                })
                .map_or(0, |row| row.opinion)
        };
        let at_war = match &wars {
            Ok(owner) => owner.at_war(raw),
            Err(why) => Err(why.clone()),
        };
        let at_war = match at_war {
            Ok(value) => value,
            Err(why) => {
                *refused.entry(why).or_default() += 1;
                continue;
            }
        };
        let same_faction = faction_of
            .get(&slot)
            .is_some_and(|faction| faction_of.get(&selected) == Some(faction));
        let relation = Relation {
            selected: slot == selected,
            at_war,
            same_faction,
            war_goal: war_goals.as_ref().ok().map(|marked| marked.contains(&slot)),
            opinion,
        };
        match relation.color(defines) {
            Ok(Some(color)) => {
                styles.insert(slot + 1, style(color));
            }
            Ok(None) => {}
            Err(()) => {
                let why = war_goals.as_ref().err().cloned().unwrap_or_default();
                *refused.entry(why).or_default() += 1;
            }
        }
    }
    for (why, count) in refused {
        warnings.push(format!(
            "The diplomacy map mode leaves {count} countries unfilled: {why}"
        ));
    }
    let styles = map_display_data::value_index::ValueIndex::try_from_iter(styles)?;
    Ok(Some(ModeLayer {
        region_of: membership::plane(inputs, 0, &styles)?,
        styles,
        thickness: Some(defines.diplomacy_thickness),
        gradient: None,
        channel: defines.diplomacy_channel,
    }))
}

/// The manpower mode's colour for a state of `population`: the two colours blended by the
/// population's share of the range, clamped to it, in the game's single precision.
fn manpower_color(defines: &ModeDefines, population: i32) -> [f32; 4] {
    let t = (population as f32 / defines.manpower_max).clamp(0.0, 1.0);
    let [from, to] = defines.manpower_colors;
    std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
}

/// Every state, coloured by its population.
fn manpower(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let defines = &inputs.content.defines;
    let states = match complete::<tables::States>(inputs.frame, "manpower") {
        Ok(states) => states,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let Some(styles) = inputs
        .content
        .styles
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .manpower(inputs, states, warnings)?
    else {
        return Ok(None);
    };
    Ok(Some(ModeLayer {
        region_of: membership::plane(inputs, 1, &styles)?,
        styles,
        thickness: Some(defines.state_thickness),
        gradient: Some(LEVEL_GRADIENT),
        channel: [0.0, defines.state_channel[1]],
    }))
}

/// Network strengths are fixed5 percentages.
const NETWORK_STRENGTH_SCALE: f32 = 100_000.0;
const NETWORK_STRENGTH_FULL: f32 = 100.0;

/// The operatives mode's colour for a state's network strength: its share of 100%, clamped,
/// times the larger of one half and itself, blends the pair; quiet subnetworks take the quiet
/// pair. Single precision throughout, as the game computes it.
fn network_color(defines: &ModeDefines, strength: i64, quiet: bool) -> [f32; 4] {
    let share = (strength as f32 / NETWORK_STRENGTH_SCALE / NETWORK_STRENGTH_FULL).clamp(0.0, 1.0);
    let t = (share * share.max(0.5)).clamp(0.0, 1.0);
    let [from, to] = if quiet {
        defines.quiet_network_colors
    } else {
        defines.network_colors
    };
    std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
}

/// The selected country's intelligence networks: each state of positive strength whose
/// controller is the network's target, coloured by that strength.
fn operatives(inputs: &Inputs, warnings: &mut Vec<String>) -> io::Result<Option<ModeLayer>> {
    let defines = &inputs.content.defines;
    let Some(selected) = inputs.selected else {
        warnings.push("The operatives map mode needs a viewing country".to_owned());
        return Ok(None);
    };
    let Some(styles) = inputs
        .content
        .styles
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .operatives(inputs, selected, warnings)?
    else {
        return Ok(None);
    };
    Ok(Some(ModeLayer {
        region_of: membership::plane(inputs, 1, &styles)?,
        styles,
        thickness: Some(defines.state_thickness),
        gradient: Some(LEVEL_GRADIENT),
        channel: [0.0, defines.state_channel[1]],
    }))
}

fn occupation(
    compliance: bool,
    inputs: &Inputs,
    warnings: &mut Vec<String>,
) -> io::Result<Option<ModeLayer>> {
    let mode = if compliance {
        "compliance"
    } else {
        "resistance"
    };
    let defines = &inputs.content.defines;
    let Some(selected) = inputs.selected else {
        warnings.push(format!("The {mode} map mode needs a viewing country"));
        return Ok(None);
    };
    let Some(styles) = inputs
        .content
        .styles
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .occupation(inputs, selected, compliance, warnings)?
    else {
        return Ok(None);
    };
    Ok(Some(ModeLayer {
        region_of: membership::plane(inputs, 1, &styles)?,
        styles,
        thickness: Some(defines.resistance_thickness),
        gradient: Some(LEVEL_GRADIENT),
        channel: [0.0, defines.resistance_cutoff],
    }))
}

pub mod visibility;

#[cfg(test)]
mod tests;
