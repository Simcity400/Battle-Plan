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

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

use installed_content::{
    defines::Defines,
    map::{MapData, ProvinceKind},
    scripted_map_modes::{GameLayer, LayerType, MapModes, MapText},
};
use replay::{frame::DecodedFrame, rows::tables};
use viewer_foundation::map::{
    color_inheritance::{active_subject, slot_of},
    directed_wars,
    tables::{Available, core, rows},
};

use super::political::{color_source, inherited_colors};

mod colour_script;
pub use colour_script::ColourState;
pub mod infrastructure;
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

/// Why the supply mode is unavailable: its province colours, supply borders, flows and
/// tooltips are computed from the supply network's per-province sources and per-hour node
/// amounts, which no recording holds.
pub const SUPPLY_UNAVAILABLE: &str = "Not available in replays: the game computes this map mode from the supply network's per-province sources and hourly hub amounts, which recordings do not hold.";

/// A view's map mode.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub color: [u8; 4],
    pub pulse: bool,
    pub fill: f32,
}

/// What one channel of the political fill shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Source {
    /// The strategic regions with their naval effects.
    #[default]
    Strategic,
    /// The country layer: the political fill, or [`Draw::country`]'s variant.
    Country,
    /// The mode's own layer, [`Draw::layer`].
    Mode,
    /// A scripted mode's second game layer, [`Draw::second`], when both its channels take
    /// layers of their own. It never shows beside the country layer.
    Second,
    /// Nothing.
    Empty,
}

/// The political channels' pair.
pub const POLITICAL: [Source; 2] = [Source::Strategic, Source::Country];

/// What a mode draws in the two channels.
#[derive(Clone, Debug)]
pub struct Draw {
    pub channels: [Source; 2],
    /// The country layer's variant colouring; `None` keeps the political fill.
    pub country: Option<ModeLayer>,
    /// The mode's own layer.
    pub layer: Option<ModeLayer>,
    /// A scripted mode's second game layer.
    pub second: Option<ModeLayer>,
    /// The political country layer's camera override and outline cutoff, which it takes into
    /// whichever channel shows it.
    pub political: [f32; 2],
}

impl PartialEq for Draw {
    fn eq(&self, other: &Self) -> bool {
        self.channels == other.channels
            && self.country == other.country
            && self.layer == other.layer
            && self.second == other.second
            && self.political.map(f32::to_bits) == other.political.map(f32::to_bits)
    }
}

impl Eq for Draw {}

impl Draw {
    fn of(channels: [Source; 2], political: [f32; 2]) -> Self {
        Self {
            channels,
            country: None,
            layer: None,
            second: None,
            political,
        }
    }

    /// Each channel's camera-distance override and outline cutoff: its layer's own. `None`
    /// keeps the content's defines for that channel's political layer (the strategic regions
    /// in channel 1, the countries in channel 2), and for an empty channel.
    pub fn channel_constants(&self) -> [Option<[f32; 2]>; 2] {
        self.channels.map(|source| match source {
            Source::Country => Some(
                self.country
                    .as_ref()
                    .map_or(self.political, |layer| layer.channel),
            ),
            Source::Mode => self.layer.as_ref().map(|layer| layer.channel),
            Source::Second => self.second.as_ref().map(|layer| layer.channel),
            Source::Strategic | Source::Empty => None,
        })
    }
}

/// One layer's regions and presentation.
#[derive(Clone, Debug)]
pub struct ModeLayer {
    /// Region key per province id; zero leaves the province out.
    pub region_of: Vec<u32>,
    pub styles: BTreeMap<u32, Style>,
    /// A fixed thickness for every region; `None` keeps the area rule.
    pub thickness: Option<f32>,
    /// A constant gradient byte for every region.
    pub gradient: Option<u8>,
    /// The channel's camera-distance override and outline cutoff.
    pub channel: [f32; 2],
}

impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        self.color == other.color
            && self.pulse == other.pulse
            && self.fill.to_bits() == other.fill.to_bits()
    }
}

impl Eq for Style {}

impl PartialEq for ModeLayer {
    fn eq(&self, other: &Self) -> bool {
        self.region_of == other.region_of
            && self.styles == other.styles
            && self.thickness.map(f32::to_bits) == other.thickness.map(f32::to_bits)
            && self.gradient == other.gradient
            && self.channel.map(f32::to_bits) == other.channel.map(f32::to_bits)
    }
}

impl Eq for ModeLayer {}

/// What a mode draws and what it cannot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeFill {
    /// `None` draws the political channels.
    pub draw: Option<Draw>,
    pub warnings: Vec<String>,
    /// A scripted mode's kept colour-script borders after this hour.
    pub colour: Option<Arc<ColourState>>,
}

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
    pub defines: ModeDefines,
    ideologies: Arc<installed_content::ideologies::Ideologies>,
    /// Each declared category's colour by folded name: `Ok(None)` for one without a colour.
    category_colors: BTreeMap<Vec<u8>, Result<Option<[f32; 4]>, String>>,
    pub scripted: MapModes,
    /// The content's faction templates, which name a faction without a literal name.
    faction_templates: Result<Arc<installed_content::faction_templates::Catalog>, String>,
    /// The recording's language, which a faction's stored literal name is written in.
    recorded_language: String,
    /// The terrain mode's layer, built once, or why the content cannot build it.
    terrain: Result<terrain::Built, String>,
    pub infrastructure: infrastructure::Content,
}

impl Content {
    pub fn new(
        defines: &Defines,
        ideologies: Arc<installed_content::ideologies::Ideologies>,
        categories: &installed_content::state_categories::Database,
        scripted: MapModes,
        faction_templates: Result<Arc<installed_content::faction_templates::Catalog>, String>,
        recorded_language: String,
        terrain: Result<terrain::Sources, String>,
        infrastructure: infrastructure::Content,
    ) -> io::Result<Self> {
        let defines = ModeDefines::load(defines)?;
        let terrain = terrain.map(|sources| terrain::build(&sources, &defines.terrain));
        let category_colors = categories
            .names
            .iter()
            .map(|name| (name.to_ascii_lowercase(), categories.color(name)))
            .collect();
        Ok(Self {
            defines,
            ideologies,
            category_colors,
            scripted,
            faction_templates,
            recorded_language,
            terrain,
            infrastructure,
        })
    }

    /// Whose names the map writes in `mode`, far out and close in: the far names show at or
    /// above `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF` and the near ones below it, cross-fading. The
    /// game chooses per mode: faction names in the factions mode, players' names in the players
    /// mode, strategic region names in the air and navy modes, country names otherwise, and none
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
    /// The strategic regions' names, which the viewer does not draw: the map carries none.
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
    localisation: &installed_content::localisation::Localisation,
) -> Result<GroupLabels, String> {
    use dashboard_text::faction_names;
    let factions = complete::<tables::Factions>(frame, "factions")?;
    let members = complete::<tables::FactionMembers>(frame, "factions")?;
    let countries = Countries::read(frame).map_err(|error| error.to_string())?;
    let lexical = faction_names::LexicalNames::new(
        frame.tables[replay::rows::TableId::LexicalTokens.index()].clone(),
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
    let mut held: BTreeMap<u32, Vec<&replay::rows::PlayerRow>> = BTreeMap::new();
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
fn subject_colors(
    map: &MapData,
    frame: &DecodedFrame,
    countries: &Countries,
    slots: &BTreeSet<u32>,
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
        let source = color_source(slot, &inherited)?;
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
fn complete<'a, T: replay::rows::TableSchema>(
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
        MapMode::Scripted(name) => scripted_fill(name, inputs),
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
            country,
            layer: None,
            second: None,
            political,
        })
    };
    let over_countries = |layer: Option<ModeLayer>| Draw {
        channels: [Country, built(&layer, Mode)],
        country: None,
        layer,
        second: None,
        political,
    };
    let draw = match stock {
        Stock::Default => return Ok(ModeFill::default()),
        Stock::StrategicNavy | Stock::StrategicAir | Stock::Resources => {
            warnings.push(unbuilt(stock).to_owned());
            match stock {
                Stock::StrategicNavy => return Ok(refusal(warnings)),
                Stock::StrategicAir => Draw::of([Strategic, Empty], political),
                _ => Draw::of([Country, Empty], political),
            }
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
        Stock::Operatives => {
            warnings.push(unbuilt(stock).to_owned());
            over_countries(operatives(inputs, &mut warnings)?)
        }
        Stock::Diplomacy => {
            let layer = diplomacy(inputs, &mut warnings)?;
            Draw {
                channels: [built(&layer, Mode), Empty],
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
            "Air map mode: the strategic regions show without their superiority colours, names, region statistics and range overlays, which are not drawn; the map writes no names"
        }
        Stock::StrategicNavy => {
            "Navy map mode: the political channels show without the naval supremacy colours, strategic region names, region statistics and range overlays, which are not drawn; the map writes no names"
        }
        Stock::Resources => {
            "Resources map mode: the countries show without the state resource icons, which are not drawn"
        }
        Stock::Operatives => {
            "Operatives map mode: the operative, operation and counterintelligence icons are not drawn; operative locations and missions are not recorded"
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
    let Inputs {
        content,
        map,
        frame,
        selected,
        ..
    } = inputs;
    let defines = &content.defines;
    let countries = Countries::read(frame)?;
    let (region_of, slots) = controller_regions(map, frame, &countries)?;
    let colors = subject_colors(map, frame, &countries, &slots, warnings)?;
    let mut styles = BTreeMap::new();
    let channel = if variant == Variant::Factions {
        defines.factions_channel
    } else {
        defines.country_channel
    };
    match variant {
        Variant::Country => {
            for (&slot, &color) in &colors {
                styles.insert(slot + 1, style(color));
            }
        }
        Variant::Factions => {
            let members = match complete::<tables::FactionMembers>(frame, "factions") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let factions = match complete::<tables::Factions>(frame, "factions") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let faction_colors = factions
                .iter()
                .map(|row| {
                    (
                        row.id,
                        [
                            row.color_red,
                            row.color_green,
                            row.color_blue,
                            row.color_alpha,
                        ]
                        .map(f32::from_bits),
                    )
                })
                .collect::<HashMap<_, _>>();
            let membership = members
                .iter()
                .filter_map(|row| Some((countries.slot(row.country)?, row.faction)))
                .collect::<HashMap<_, _>>();
            for &slot in &slots {
                let color = match membership
                    .get(&slot)
                    .and_then(|faction| faction_colors.get(faction))
                {
                    Some(&color) => with_alpha(color, defines.factions_member_alpha),
                    None => defines.factions_not_member,
                };
                styles.insert(slot + 1, style(color));
            }
        }
        Variant::Players => {
            let players = match complete::<tables::Players>(frame, "players") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let human = players
                .iter()
                .filter_map(|row| countries.slot(row.country))
                .collect::<BTreeSet<_>>();
            for (&slot, &color) in &colors {
                let color = if human.contains(&slot) {
                    color
                } else {
                    with_alpha(color, defines.players_other_alpha)
                };
                let mut region = style(color);
                if Some(slot) == *selected {
                    region.pulse = true;
                    region.fill = defines.selected_fill;
                }
                styles.insert(slot + 1, region);
            }
        }
        Variant::Ideology => {
            let politics = match complete::<tables::Politics>(frame, "ideology") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let parties = match complete::<tables::PoliticalParties>(frame, "ideology") {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            let ruling = politics
                .iter()
                .filter_map(|row| Some((countries.slot(row.country)?, row.ruling_party?)))
                .collect::<HashMap<_, _>>();
            let mut unknown = BTreeSet::new();
            for (&slot, &color) in &colors {
                let party = ruling.get(&slot).and_then(|&party| {
                    parties
                        .iter()
                        .find(|row| row.id == party && countries.slot(row.country) == Some(slot))
                });
                let color = match party {
                    None => Some(color),
                    Some(party) => {
                        let token = String::from_utf8_lossy(&party.token);
                        let group = match content.ideologies.get(&token) {
                            Ok(Some(group)) => Ok(Some(group)),
                            Ok(None) => content.ideologies.group_of(&token),
                            Err(gap) => Err(gap),
                        };
                        match group {
                            Ok(Some(group)) => match &group.color {
                                Ok(group_color) => {
                                    Some(with_alpha(*group_color, defines.ideology_alpha))
                                }
                                Err(_) => {
                                    unknown.insert(token.into_owned());
                                    None
                                }
                            },
                            _ => {
                                unknown.insert(token.into_owned());
                                None
                            }
                        }
                    }
                };
                if let Some(color) = color {
                    styles.insert(slot + 1, style(color));
                }
            }
            if !unknown.is_empty() {
                warnings.push(format!(
                    "The ideology map mode leaves countries unfilled whose ruling ideology has no readable content colour: {}",
                    unknown.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    Ok(Some(finish(region_of, styles, None, None, channel)))
}

/// Drops provinces of regions without a style, so a refused region draws nothing.
fn finish(
    mut region_of: Vec<u32>,
    styles: BTreeMap<u32, Style>,
    thickness: Option<f32>,
    gradient: Option<u8>,
    channel: [f32; 2],
) -> ModeLayer {
    for key in &mut region_of {
        if *key != 0 && !styles.contains_key(key) {
            *key = 0;
        }
    }
    ModeLayer {
        region_of,
        styles,
        thickness,
        gradient,
        channel,
    }
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
    let names = viewer_foundation::table_access::TokenNames::new(inputs.frame)?;
    let mut styles = BTreeMap::new();
    let (mut unread, mut unnamed, mut uncoloured) = (0, 0, 0);
    for state in states {
        let token = match state.category {
            replay::rows::Recorded::Value(Some(token)) => token,
            replay::rows::Recorded::Value(None) => {
                unread += 1;
                continue;
            }
            replay::rows::Recorded::NotRecorded => {
                warnings.push(
                    "The states map mode needs each state's live category, which this recording does not hold".to_owned(),
                );
                return Ok(None);
            }
        };
        let Ok(name) = names.name(token) else {
            unnamed += 1;
            continue;
        };
        match inputs
            .content
            .category_colors
            .get(name.to_ascii_lowercase().as_bytes())
        {
            Some(Ok(Some(color))) => {
                styles.insert(state.id + 1, style(*color));
            }
            Some(Ok(None)) => {
                styles.insert(state.id + 1, style(STATE_WITHOUT_CATEGORY));
            }
            Some(Err(_)) | None => uncoloured += 1,
        }
    }
    for (count, why) in [
        (unread, "whose category read was refused"),
        (
            unnamed,
            "whose category token the recorded dictionary does not name",
        ),
        (
            uncoloured,
            "whose category the content does not declare or colour",
        ),
    ] {
        if count != 0 {
            warnings.push(format!(
                "The states map mode leaves {count} states unfilled {why}"
            ));
        }
    }
    let provinces = core::<tables::Provinces>(inputs.frame)?;
    let mut region_of = vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
    for province in provinces {
        if province.state != 0 {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.state_thickness),
        None,
        defines.state_channel,
    )))
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
        replay::rows::Recorded::Value(Some(recipients)) => recipients,
        replay::rows::Recorded::Value(None) => {
            return Err("a war goal of the selected country did not read".to_owned());
        }
        replay::rows::Recorded::NotRecorded => return Err(WAR_GOALS_UNRECORDED.to_owned()),
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
        map,
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
    let (region_of, slots) = controller_regions(map, frame, &countries)?;
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
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.diplomacy_thickness),
        None,
        defines.diplomacy_channel,
    )))
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
    let mut styles = BTreeMap::new();
    for state in states {
        match state.population {
            replay::rows::Recorded::Value(population) => {
                styles.insert(state.id + 1, style(manpower_color(defines, population)));
            }
            replay::rows::Recorded::NotRecorded => {
                warnings.push(
                    "The manpower map mode needs each state's population, which this recording does not hold".to_owned(),
                );
                return Ok(None);
            }
        }
    }
    let provinces = core::<tables::Provinces>(inputs.frame)?;
    let mut region_of = vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
    for province in provinces {
        if province.state != 0 {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.state_thickness),
        Some(LEVEL_GRADIENT),
        [0.0, defines.state_channel[1]],
    )))
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
    let frame = inputs.frame;
    let networks = match complete::<tables::IntelNetworks>(frame, "operatives") {
        Ok(rows) => rows,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let countries = Countries::read(frame)?;
    let controllers = core::<tables::States>(frame)?
        .iter()
        .map(|state| (state.id, state.controller))
        .collect::<HashMap<_, _>>();
    let mut styles = BTreeMap::new();
    let mut unknown_states = 0;
    for row in networks {
        if countries.slot(row.owner_country) != Some(selected) || row.local_strength_raw <= 0 {
            continue;
        }
        let Ok(state) = u32::try_from(row.state_id) else {
            unknown_states += 1;
            continue;
        };
        let Some(&controller) = controllers.get(&state) else {
            unknown_states += 1;
            continue;
        };
        // The network covers the state only while its target controls it.
        if controller == 0
            || countries.slot(controller).is_none()
            || countries.slot(controller) != countries.slot(row.target_country)
        {
            continue;
        }
        styles.insert(
            state + 1,
            style(network_color(
                defines,
                row.local_strength_raw,
                row.local_quiet != 0,
            )),
        );
    }
    if unknown_states != 0 {
        warnings.push(format!(
            "The operatives map mode leaves {unknown_states} network states unfilled that are not recorded states"
        ));
    }
    let provinces = core::<tables::Provinces>(frame)?;
    let mut region_of = vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
    for province in provinces {
        if province.state != 0 {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.state_thickness),
        Some(LEVEL_GRADIENT),
        [0.0, defines.state_channel[1]],
    )))
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
    let frame = inputs.frame;
    let countries = Countries::read(frame)?;
    let resistance = match complete::<tables::StateResistance>(frame, mode) {
        Ok(rows) => rows,
        Err(why) => {
            warnings.push(why);
            return Ok(None);
        }
    };
    let by_state = resistance
        .iter()
        .map(|row| (row.state, row))
        .collect::<HashMap<_, _>>();
    // The selected country's controlled states, occupied ones included.
    let owned = core::<tables::States>(frame)?
        .iter()
        .filter(|state| countries.slot(state.controller) == Some(selected))
        .map(|state| state.id)
        .collect::<BTreeSet<_>>();
    let provinces = core::<tables::Provinces>(frame)?;
    let len = region_table_len(inputs.map, provinces.iter().map(|row| row.id));
    let mut region_of = vec![0; len];
    for province in provinces {
        if owned.contains(&province.state) {
            region_of[province.id as usize] = province.state + 1;
        }
    }
    let mut styles = BTreeMap::new();
    let mut missing = 0;
    for &state in &owned {
        let Some(row) = by_state.get(&state) else {
            missing += 1;
            continue;
        };
        let color = if row.country as i32 > 0 {
            let (list, level) = if compliance {
                (&defines.compliance_colors, row.compliance)
            } else {
                (&defines.resistance_colors, row.level)
            };
            list.color(level as f32 / RESISTANCE_SCALE)
        } else {
            defines.resistance_uninitialised
        };
        styles.insert(state + 1, style(color));
    }
    if missing != 0 {
        warnings.push(format!(
            "The {mode} map mode leaves {missing} controlled states unfilled without a StateResistance row"
        ));
    }
    warnings.push(format!(
        "The {mode} map mode shows the recording's sampled StateResistance values"
    ));
    Ok(Some(finish(
        region_of,
        styles,
        Some(defines.resistance_thickness),
        Some(LEVEL_GRADIENT),
        [0.0, defines.resistance_cutoff],
    )))
}

fn scripted_fill(name: &str, inputs: &Inputs) -> io::Result<ModeFill> {
    let Some(mode) = inputs.content.scripted.get(name) else {
        return Ok(refusal(vec![format!(
            "The scripted map mode {name} is not declared by this recording's content"
        )]));
    };
    let mut warnings = Vec::new();
    let previous = inputs.colour.filter(|state| state.mode == name);
    let refresh = colour_script::refreshes(previous, mode.update_daily, inputs.frame.hour);
    let mut kept: [colour_script::LayerState; 2] = Default::default();
    let mut coloured = false;
    let political = inputs.content.defines.country_channel;
    let mut draw = Draw::of([Source::Empty; 2], political);
    // The game layer each mode slot holds, so a layer both sides name shows in both.
    let mut held: Vec<(GameLayer, Source)> = Vec::new();
    // The bottom layer is channel 1 and the top layer channel 2. The country layer is one
    // object, so when both sides choose a variant of it, the top's variant shows in both.
    for (channel, (side, layer)) in [("bottom", &mode.bottom), ("top", &mode.top)]
        .into_iter()
        .enumerate()
    {
        draw.channels[channel] = match layer.kind {
            LayerType::None => Source::Empty,
            LayerType::Game(game) => {
                let variant = match game {
                    GameLayer::Country => Some(Variant::Country),
                    GameLayer::Players => Some(Variant::Players),
                    GameLayer::Factions => Some(Variant::Factions),
                    GameLayer::Ideology => Some(Variant::Ideology),
                    GameLayer::States | GameLayer::Diplomacy => None,
                };
                match variant {
                    Some(variant) => {
                        let country = country_variant(variant, inputs, &mut warnings)?;
                        let source = built(&country, Source::Country);
                        if channel == 1 && draw.channels[0] == Source::Country {
                            draw.channels[0] = source;
                        }
                        draw.country = country;
                        source
                    }
                    // The same game layer in both channels is one layer shown twice.
                    None if held.iter().any(|(layer, _)| *layer == game) => held
                        .iter()
                        .find_map(|&(layer, source)| (layer == game).then_some(source))
                        .unwrap_or(Source::Empty),
                    None => {
                        let layer = if game == GameLayer::States {
                            states(inputs, &mut warnings)?
                        } else {
                            diplomacy(inputs, &mut warnings)?
                        };
                        // A second distinct layer takes the country layer's place, which then
                        // shows in neither channel.
                        let (slot, source) = if draw.layer.is_none() {
                            (&mut draw.layer, Source::Mode)
                        } else {
                            (&mut draw.second, Source::Second)
                        };
                        let source = built(&layer, source);
                        *slot = layer;
                        held.push((game, source));
                        source
                    }
                }
            }
            LayerType::Country | LayerType::State | LayerType::StateController => {
                coloured = true;
                let borders = colour_script::borders(
                    name,
                    side,
                    layer,
                    inputs,
                    previous.map(|state| &state.layers[channel]),
                    refresh,
                    &mut warnings,
                );
                let built_layer = (!borders.refused)
                    .then(|| colour_script::layer(layer, &borders.borders, inputs));
                kept[channel] = borders;
                let (slot, source) = if draw.layer.is_none() {
                    (&mut draw.layer, Source::Mode)
                } else {
                    (&mut draw.second, Source::Second)
                };
                let source = built(&built_layer, source);
                *slot = built_layer;
                source
            }
        };
    }
    if coloured {
        // The game also refreshes when a script runs `force_update_map_mode`, which recordings
        // do not hold.
        warnings.push(format!(
            "Scripted map mode {name}: force_update_map_mode not recorded; its colour borders refresh on the first hour shown and each day's first hour only"
        ));
    }
    Ok(ModeFill {
        draw: Some(draw),
        warnings,
        colour: coloured.then(|| {
            Arc::new(ColourState {
                mode: name.to_owned(),
                layers: kept,
            })
        }),
    })
}

pub mod visibility;

#[cfg(test)]
mod tests;
