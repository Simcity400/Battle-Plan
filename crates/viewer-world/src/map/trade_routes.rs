//! The viewed country's trade-route lines and their convoys.
//!
//! The game draws its local player's lines in the navy, supply and resources map modes; the
//! viewer draws the viewed country's ([`Visibility::trade_routes`] gates them in the scene,
//! where they share the battle-plan arrow manager as `PlanKind::TradeRoute` draws).
//!
//! # Layers
//!
//! Each line belongs to one layer, drawn in layer order, from the recorded convoy clients:
//!
//! | layer | lines | route kinds drawn |
//! |---|---|---|
//! | 0 production | the viewed country's own resource origins (`ResourceOrigins` list 0) | naval |
//! | 1 supply transfers | its naval supply transfers | every kind |
//! | 2 exports | imports whose exporter it is | land and naval |
//! | 3 imports | imports whose importer it is | land and naval |
//! | 4 lend-lease given | lend-lease it gives | naval |
//! | 5 lend-lease received | lend-lease it receives | naval |
//! | 6 market bought | market contracts it buys, grouped by kind and endpoint states | land and naval |
//! | 7 market sold | market contracts it sells, grouped by kind and endpoint states | land and naval |
//!
//! A line takes its layer's `TRADE_ROUTE_*_COLOR`, or `TRADE_ROUTE_BAD_EFFICIENCY_ROUTE_COLOR`
//! when its recorded `bad` bit is set (a market line when any contract of its route is bad).
//! Within a bad naval line, each sea region where the viewed country's recorded delivery
//! efficiency (`RegionalConvoyEfficiency`, full without a row) is below
//! `TRADE_ROUTE_REGIONAL_BAD_EFFICIENCY_THRESHOLD` draws the arrow type's second texture variant
//! in `TRADE_ROUTE_BAD_EFFICIENCY_HOTSPOT_COLOR`, the line's secondary colour.
//!
//! An export, import or lend-lease line runs backwards when its client's direction country
//! (the importer or lend-lease receiver) links to the same country as the route's own country
//! (the exporter or giver). Lend-lease layer membership uses the giver separately. Every other
//! line runs from the route's source to its destination.
//!
//! # Geometry
//!
//! The `geometry` module searches ordered province neighbours within the recorded regions or
//! states, inserting raw crossing controls and using port anchors for port land provinces.
//! Shared province-use bytes displace controls before fixed5 smoothing/subdivision. Restricted
//! permissions without recorded proof are refused by name. Content supplies the ribbon style.
//!
//! # Convoys
//!
//! Layers 0 to 5 carry convoy models (the `convoys` module) when the view draws units and the
//! camera is below `UNITS_DISTANCE_CUTOFF`.

mod convoys;
mod geometry;
#[cfg(test)]
mod tests;

pub(crate) use convoys::presentation_counter;
pub use convoys::{
    CONVOY_ENTITY_SUFFIX, ConvoyDefines, ConvoyFleet, ConvoyLine, ConvoyPose,
    FALLBACK_CONVOY_ENTITY, convoy_count, convoy_entity, convoy_instances, convoys_shown,
};
pub use geometry::{RouteContent, Shaped, Stops, border_province, directed_stops, emit, overlap_offset, stops};

use std::{collections::BTreeMap, io};

use installed_content_core::defines::Defines;
use replay_data::{
    rows::ConvoyClientRow, rows::CountryLinkRow, rows::RegionalConvoyEfficiencyRow,
    rows::ResourceOriginRow, rows::TradeRouteRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::{
    errors::invalid,
    map::{
        mode_visibility::Visibility,
        province_map::HeightField,
        ribbon_content::{MAPARROWS, OrderAssets, OrderField, TRADE_ROUTE_TYPE_NAME},
        ribbon_fixed::Smoothing,
    },
    table_access::{self, Available},
};

/// `TradeRoutes.kind`: no route, land, naval.
pub const NO_ROUTE: u8 = 0;
pub const LAND_ROUTE: u8 = 1;
pub const NAVAL_ROUTE: u8 = 2;

/// `TradeRoutes.class` of a resource origin's route.
pub const ORIGIN_ROUTE_CLASS: u8 = 0;

/// `ResourceOrigins.list` of the controller's own origins.
pub const OWN_ORIGINS: u8 = 0;

/// `ConvoyClients.category`.
pub const SUPPLY_TRANSFER: u8 = 1;
pub const IMPORT: u8 = 2;
pub const LEND_LEASE: u8 = 4;
pub const MARKET: u8 = 6;

/// A full regional delivery efficiency, fixed5: what a region without a recorded row has.
pub const FULL_EFFICIENCY: i64 = 100_000;

/// A `TradeRoutes` key: (country, class, other).
pub type RouteKey = (u32, u8, u32);

macro_rules! fields {
    ($(#[$doc:meta])* $trait:ident for $row:ty { $($name:ident: $type:ty),+ $(,)? }) => {
        $(#[$doc])*
        pub trait $trait {
            $(fn $name(&self) -> $type;)+
        }

        impl $trait for $row {
            $(fn $name(&self) -> $type {
                self.$name
            })+
        }
    };
}

fields! {
    /// What the lines read of a `ResourceOrigins` row.
    OriginFields for ResourceOriginRow { country: u32, state: u32, list: u8, ordinal: u32, bad: bool, convoys_assigned: i32 }
}

fields! {
    /// What the lines read of a `ConvoyClients` row.
    ClientFields for ConvoyClientRow {
        handle: u32, category: u8, owner: u32, other: u32, route_class: u8, route_country: u32, route_other: u32,
        bad: bool, convoys_assigned: i32, owner_ordinal: Option<u32>, other_ordinal: Option<u32>,
    }
}

fields! {
    /// What the efficiencies read of a `RegionalConvoyEfficiency` row.
    RegionalFields for RegionalConvoyEfficiencyRow { country: u32, region: u32, efficiency: i64 }
}

/// What the lines and their geometry read of a `TradeRoutes` row.
pub trait RouteFields {
    fn country(&self) -> u32;
    fn class(&self) -> u8;
    fn other(&self) -> u32;
    fn kind(&self) -> u8;
    fn from(&self) -> u32;
    fn to(&self) -> u32;
    fn path(&self) -> &[u32];
    fn source_state(&self) -> Option<u32>;
    fn destination_state(&self) -> Option<u32>;
}

impl RouteFields for TradeRouteRow {
    fn country(&self) -> u32 {
        self.country
    }
    fn class(&self) -> u8 {
        self.class
    }
    fn other(&self) -> u32 {
        self.other
    }
    fn kind(&self) -> u8 {
        self.kind
    }
    fn from(&self) -> u32 {
        self.from
    }
    fn to(&self) -> u32 {
        self.to
    }
    fn path(&self) -> &[u32] {
        &self.path
    }
    fn source_state(&self) -> Option<u32> {
        self.source_state
    }
    fn destination_state(&self) -> Option<u32> {
        self.destination_state
    }
}

/// A `ResourceExchanges` row's place in its importer's and exporter's lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExchangeOrder {
    pub resource: u32,
    pub import_ordinal: u32,
    pub export_ordinal: u32,
}

/// Why a reference `CountryLinks` holds no row for is unresolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinksGap {
    Complete,
    Truncated { missing: u32 },
    Unavailable { reason: String },
    NotRecorded { mode: String },
}

/// A line's layer, in draw order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Production,
    SupplyTransfer,
    Export,
    Import,
    LendLeaseGiven,
    LendLeaseReceived,
    MarketBought,
    MarketSold,
}

impl Layer {
    pub const ALL: [Self; 8] = [
        Self::Production,
        Self::SupplyTransfer,
        Self::Export,
        Self::Import,
        Self::LendLeaseGiven,
        Self::LendLeaseReceived,
        Self::MarketBought,
        Self::MarketSold,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The `NGraphics` colour define of the layer's lines.
    pub fn color_define(self) -> &'static str {
        match self {
            Self::Production => "TRADE_ROUTE_PRODUCTION_TRANSFER_COLOR",
            Self::SupplyTransfer => "TRADE_ROUTE_SUPPLIES_TRANSFER_COLOR",
            Self::Export => "TRADE_ROUTE_RESOURCE_EXPORT_COLOR",
            Self::Import => "TRADE_ROUTE_RESOURCE_IMPORT_COLOR",
            Self::LendLeaseGiven => "TRADE_ROUTE_LEND_LEASE_EXPORT_COLOR",
            Self::LendLeaseReceived => "TRADE_ROUTE_LEND_LEASE_IMPORT_COLOR",
            Self::MarketBought | Self::MarketSold => "TRADE_ROUTE_INTERNATIONAL_MARKET_COLOR",
        }
    }

    /// Whether the layer draws a route of `kind`.
    pub fn admits(self, kind: u8) -> bool {
        match self {
            Self::Production | Self::LendLeaseGiven | Self::LendLeaseReceived => {
                kind == NAVAL_ROUTE
            }
            Self::SupplyTransfer => true,
            Self::Export | Self::Import | Self::MarketBought | Self::MarketSold => {
                kind == LAND_ROUTE || kind == NAVAL_ROUTE
            }
        }
    }

    /// Whether the layer's lines carry convoy models.
    pub fn carries_convoys(self) -> bool {
        !matches!(self, Self::MarketBought | Self::MarketSold)
    }

    /// Whether the layer draws one line per identical route of its market contracts.
    fn groups_contracts(self) -> bool {
        matches!(self, Self::MarketBought | Self::MarketSold)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::SupplyTransfer => "supply transfer",
            Self::Export => "export",
            Self::Import => "import",
            Self::LendLeaseGiven => "lend-lease given",
            Self::LendLeaseReceived => "lend-lease received",
            Self::MarketBought => "market bought",
            Self::MarketSold => "market sold",
        }
    }
}

/// The `NDefines.NGraphics` values the lines and convoys read.
#[derive(Clone, Debug, PartialEq)]
pub struct TradeRouteDefines {
    /// Each layer's colour, in [`Layer::ALL`] order.
    pub colors: [[f32; 4]; 8],
    /// `TRADE_ROUTE_BAD_EFFICIENCY_ROUTE_COLOR`.
    pub bad_color: [f32; 4],
    /// `TRADE_ROUTE_BAD_EFFICIENCY_HOTSPOT_COLOR`.
    pub hotspot_color: [f32; 4],
    /// `TRADE_ROUTE_REGIONAL_BAD_EFFICIENCY_THRESHOLD`, fixed5.
    pub regional_threshold: i64,
    /// `TRADEROUTE_SMOOTHNESS` and `TRADEROUTE_SMOOTHEN_PASSES`.
    pub smoothing: Smoothing,
    /// `TRADE_ROUTE_LINE_OFFSET`, world units.
    pub line_offset: f32,
    /// `TRADE_ROUTE_MAX_LINES`.
    pub max_lines: u32,
    pub convoys: ConvoyDefines,
    /// Stored-control hover radius multiplier.
    pub intersect_distance: f32,
}

const SECTION: &str = "NGraphics";

/// A finite `NGraphics` number.
pub(crate) fn finite(defines: &Defines, name: &str) -> io::Result<f32> {
    let value = defines.number(SECTION, name)? as f32;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid(format!("NDefines.{SECTION}.{name} is not finite")))
    }
}

/// A Lua colour: three HSV or four RGBA components, then the game's linear RGB conversion.
/// The colour reader consumes at most four values; alpha does not undergo conversion.
fn color(defines: &Defines, name: &str) -> io::Result<[f32; 4]> {
    let values = defines.numbers(SECTION, name)?;
    let mut color = match values[..values.len().min(4)] {
        [h, s, v] => {
            let [r, g, b] =
                viewer_foundation::map::gradient_math::hsv_to_rgb([h as f32, s as f32, v as f32]);
            [r, g, b, 1.0]
        }
        [r, g, b, a] => [r as f32, g as f32, b as f32, a as f32],
        _ => {
            return Err(invalid(format!(
                "NDefines.{SECTION}.{name} requires three HSV or four RGBA components"
            )));
        }
    };
    for channel in &mut color[..3] {
        *channel = channel.powf(2.2);
    }
    if !color.iter().all(|channel| channel.is_finite()) {
        return Err(invalid(format!(
            "NDefines.{SECTION}.{name} produces nonfinite RGBA"
        )));
    }
    Ok(color)
}

impl TradeRouteDefines {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        let mut colors = [[0.0; 4]; 8];
        for (value, layer) in colors.iter_mut().zip(Layer::ALL) {
            *value = color(defines, layer.color_define())?;
        }
        let passes = u32::try_from(defines.int32(SECTION, "TRADEROUTE_SMOOTHEN_PASSES")?)
            .map_err(|_| invalid("NDefines.NGraphics.TRADEROUTE_SMOOTHEN_PASSES is negative"))?;
        let max_lines = u32::try_from(defines.int32(SECTION, "TRADE_ROUTE_MAX_LINES")?)
            .ok()
            .filter(|&lines| lines > 0)
            .ok_or_else(|| invalid("NDefines.NGraphics.TRADE_ROUTE_MAX_LINES is not positive"))?;
        Ok(Self {
            colors,
            bad_color: color(defines, "TRADE_ROUTE_BAD_EFFICIENCY_ROUTE_COLOR")?,
            hotspot_color: color(defines, "TRADE_ROUTE_BAD_EFFICIENCY_HOTSPOT_COLOR")?,
            regional_threshold: defines
                .fixed5(SECTION, "TRADE_ROUTE_REGIONAL_BAD_EFFICIENCY_THRESHOLD")?,
            smoothing: Smoothing {
                smoothness: defines.fixed5(SECTION, "TRADEROUTE_SMOOTHNESS")?,
                passes,
            },
            line_offset: finite(defines, "TRADE_ROUTE_LINE_OFFSET")?,
            max_lines,
            convoys: ConvoyDefines::read(defines)?,
            intersect_distance: finite(defines, "TRADE_ROUTE_INTERSECT_DISTANCE_MULT")?,
        })
    }

    /// A line's primary colour.
    pub fn line_color(&self, layer: Layer, bad: bool) -> [f32; 4] {
        if bad {
            self.bad_color
        } else {
            self.colors[layer.index()]
        }
    }
}

/// Recorded country references as the viewed country's equality decides them: two references
/// are the same country when they link to the same canonical slot.
pub struct Countries<'a> {
    slots: std::borrow::Cow<'a, BTreeMap<u32, u32>>,
    gap: LinksGap,
    ours: u32,
    rows: std::marker::PhantomData<&'a ()>,
}

impl<'a> Countries<'a> {
    /// `ours` is the viewed country's canonical slot.
    pub fn new(links: Available<'a, tables::CountryLinks>, ours: u32) -> Self {
        let slots = links
            .rows()
            .iter()
            .map(|link: &CountryLinkRow| (link.id, link.canonical_slot))
            .collect();
        let gap = match links {
            Available::Complete(_) => LinksGap::Complete,
            Available::Truncated { missing, .. } => LinksGap::Truncated { missing },
            Available::Unavailable { reason } => LinksGap::Unavailable { reason: reason.to_owned() },
            Available::NotRecorded { mode } => LinksGap::NotRecorded { mode: mode.to_string() },
        };
        Self::of(slots, gap, ours)
    }

    /// Each reference's canonical slot, and why one it lacks is unresolved.
    pub fn of(slots: BTreeMap<u32, u32>, gap: LinksGap, ours: u32) -> Self {
        Self { slots: std::borrow::Cow::Owned(slots), gap, ours, rows: std::marker::PhantomData }
    }

    /// [`Self::of`] over slots held elsewhere.
    pub fn over(slots: &'a BTreeMap<u32, u32>, gap: LinksGap, ours: u32) -> Self {
        Self { slots: std::borrow::Cow::Borrowed(slots), gap, ours, rows: std::marker::PhantomData }
    }

    /// The canonical slot of a reference; `None` for 0, no country. The words are
    /// [`table_access::country_slot`]'s.
    pub fn slot(&self, country: u32) -> Result<Option<u32>, String> {
        if country == 0 {
            return Ok(None);
        }
        if let Some(&slot) = self.slots.get(&country) {
            return Ok(Some(slot));
        }
        Err(match &self.gap {
            LinksGap::Complete => format!("country {country} has no CountryLinks row"),
            LinksGap::Truncated { missing } => format!(
                "the CountryLinks table is truncated this hour ({missing} rows not reached) and holds no row for country {country}"
            ),
            LinksGap::Unavailable { reason } => format!(
                "the CountryLinks table is unavailable this hour ({reason}), so country {country} is unresolved"
            ),
            LinksGap::NotRecorded { mode } => format!(
                "the CountryLinks table is not recorded in a {mode} recording, so country {country} is unresolved"
            ),
        })
    }

    pub fn viewed(&self, country: u32) -> Result<bool, String> {
        Ok(self.slot(country)? == Some(self.ours))
    }

    pub fn same(&self, a: u32, b: u32) -> Result<bool, String> {
        let a = self.slot(a)?;
        Ok(a.is_some() && a == self.slot(b)?)
    }
}

/// The recorded tables the lines read: today's rows, or a layer's values.
pub struct Tables<'a, O = ResourceOriginRow, C = ConvoyClientRow, R = TradeRouteRow, G = RegionalConvoyEfficiencyRow> {
    pub origins: &'a [O],
    pub clients: &'a [C],
    pub routes: &'a [R],
    pub regional: &'a [G],
}

impl<O, C, R, G> Clone for Tables<'_, O, C, R, G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<O, C, R, G> Copy for Tables<'_, O, C, R, G> {}

/// One line of the viewed country.
#[derive(Clone, Debug, PartialEq)]
pub struct Line<'a, R = TradeRouteRow> {
    pub layer: Layer,
    pub route: &'a R,
    pub bad: bool,
    /// Whether the line runs from the route's destination to its source.
    pub reverse: bool,
    /// The convoys assigned to the line's client; none on market lines.
    pub convoys_assigned: i32,
    /// Which row of which table the line's convoys assigned is read from, so the hour's convoys
    /// are made again from the lines kept when that column alone moved ([`Sailing`]).
    pub assigned: Assigned,
    /// The client handle joining native exchange ordinals.
    pub client: u32,
}

/// Where a line's convoys assigned is read: a row of `ResourceOrigins`, a row of `ConvoyClients`,
/// or nowhere (a grouped market contract carries none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assigned {
    None,
    Origin(usize),
    Client(usize),
}

impl Assigned {
    /// The convoys assigned now, from each table's column in row order.
    pub fn of(self, origins: &[i32], clients: &[i32]) -> i32 {
        match self {
            Self::None => 0,
            Self::Origin(at) => origins.get(at).copied().unwrap_or_default(),
            Self::Client(at) => clients.get(at).copied().unwrap_or_default(),
        }
    }
}

/// One convoy-carrying line that drew, kept so the hour's convoys are made again without its
/// geometry: where its count is read and the controls its convoys sail.
#[derive(Clone, Debug)]
pub struct Sailing {
    pub assigned: Assigned,
    pub points: Vec<[f32; 2]>,
    pub map_width: f32,
}

/// What one hour's trade-route lines came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TradeRouteReport {
    /// The viewed country's lines per layer, before the layers' kind filters.
    pub lines: [usize; 8],
    pub drawn: usize,
    /// Lines whose route kind their layer does not draw.
    pub filtered: usize,
    /// Lines whose route draws nothing: no route, no path, or fewer than two placed points.
    pub empty: usize,
    /// Lines drawn with at least one hotspot region.
    pub hotspots: usize,
    /// Convoy lines and the convoys they carry.
    pub convoy_lines: usize,
    pub convoys: usize,
    /// What was refused rather than guessed, and how often.
    pub refused: BTreeMap<String, usize>,
    /// Why no line could be decided at all.
    pub unavailable: Option<String>,
}

impl TradeRouteReport {
    pub fn refuse(&mut self, reason: impl Into<String>) {
        *self.refused.entry(reason.into()).or_default() += 1;
    }

    /// The warning-area lines; empty when every line was decided.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(why) = &self.unavailable {
            out.push(format!("Trade-route lines are not drawn: {why}."));
        }
        for (what, count) in &self.refused {
            out.push(format!("Trade-route lines: {count} refused, {what}."));
        }
        out
    }
}

fn route_of(client: &impl ClientFields) -> RouteKey {
    (client.route_country(), client.route_class(), client.route_other())
}

/// The layers a client draws in for the viewed country.
fn client_layers(
    client: &impl ClientFields,
    countries: &Countries<'_>,
) -> Result<Vec<Layer>, String> {
    let (owner_side, other_side) = match client.category() {
        SUPPLY_TRANSFER => (Layer::SupplyTransfer, None),
        IMPORT => (Layer::Import, Some(Layer::Export)),
        LEND_LEASE => (Layer::LendLeaseGiven, Some(Layer::LendLeaseReceived)),
        MARKET => (Layer::MarketSold, Some(Layer::MarketBought)),
        other => return Err(format!("convoy client category {other} has no layer")),
    };
    let mut layers = Vec::new();
    if countries.viewed(client.owner())? {
        layers.push(owner_side);
    }
    if let Some(layer) = other_side
        && countries.viewed(client.other())?
    {
        layers.push(layer);
    }
    Ok(layers)
}

#[allow(clippy::too_many_arguments)]
fn push_line<'a, R: RouteFields>(
    routes: &BTreeMap<RouteKey, &'a R>,
    out: &mut Vec<Line<'a, R>>,
    report: &mut TradeRouteReport,
    layer: Layer,
    key: RouteKey,
    bad: bool,
    reverse: bool,
    convoys_assigned: i32,
    assigned: Assigned,
    client: u32,
) {
    report.lines[layer.index()] += 1;
    let Some(&route) = routes.get(&key) else {
        report.refuse(format!(
            "a {} line whose route has no TradeRoutes row",
            layer.name()
        ));
        return;
    };
    if !layer.admits(route.kind()) {
        report.filtered += 1;
        return;
    }
    out.push(Line {
        layer,
        route,
        bad,
        reverse,
        convoys_assigned,
        assigned,
        client,
    });
}

/// The viewed country's lines by layer and native directional list order. Exchanges
/// receive their existing resource/import/export order in the final displacement pass.
pub fn lines<'a, O: OriginFields, C: ClientFields, R: RouteFields, G>(
    tables: &Tables<'a, O, C, R, G>,
    countries: &Countries<'_>,
    report: &mut TradeRouteReport,
) -> Vec<Line<'a, R>> {
    let routes: BTreeMap<RouteKey, &'a R> = tables
        .routes
        .iter()
        .map(|route| ((route.country(), route.class(), route.other()), route))
        .collect();
    let mut out = Vec::new();
    let mut origins: Vec<(usize, &O)> = tables
        .origins
        .iter()
        .enumerate()
        .filter(|(_, row)| row.list() == OWN_ORIGINS)
        .collect();
    origins.sort_by_key(|(_, row)| row.ordinal());
    // Only the viewed list participates; other countries may have the same ordinals.
    origins.retain(|(_, row)| match countries.viewed(row.country()) {
        Ok(viewed) => viewed,
        Err(reason) => {
            report.refuse(format!("a resource origin's country: {reason}"));
            false
        }
    });
    if origins
        .windows(2)
        .any(|pair| pair[0].1.ordinal() == pair[1].1.ordinal())
    {
        report.refuse("production native origin list positions are duplicated");
        origins.clear();
    }
    for (at, origin) in origins {
        if origin.list() != OWN_ORIGINS {
            continue;
        }
        match countries.viewed(origin.country()) {
            Ok(true) => push_line(
                &routes,
                &mut out,
                report,
                Layer::Production,
                (origin.country(), ORIGIN_ROUTE_CLASS, origin.state()),
                origin.bad(),
                false,
                origin.convoys_assigned(),
                Assigned::Origin(at),
                0,
            ),
            Ok(false) => {}
            Err(reason) => report.refuse(format!("a resource origin's country: {reason}")),
        }
    }
    let mut by_layer: [Vec<(usize, &C)>; 8] = std::array::from_fn(|_| Vec::new());
    for (at, client) in tables.clients.iter().enumerate() {
        match client_layers(client, countries) {
            Ok(layers) => {
                for layer in layers {
                    by_layer[layer.index()].push((at, client));
                }
            }
            Err(reason) => report.refuse(format!("a convoy client: {reason}")),
        }
    }
    for layer in &Layer::ALL[1..] {
        let layer = *layer;
        let clients = &mut by_layer[layer.index()];
        if !matches!(layer, Layer::Import | Layer::Export) {
            let position = |row: &C| match layer {
                Layer::LendLeaseReceived | Layer::MarketBought => row.other_ordinal(),
                _ => row.owner_ordinal(),
            };
            clients.retain(|(_, row)| {
                if position(row).is_some() {
                    true
                } else {
                    report.refuse(format!(
                        "a {} line's native directional list position unavailable",
                        layer.name()
                    ));
                    false
                }
            });
            clients.sort_by_key(|(_, row)| position(row).unwrap());
            if clients
                .windows(2)
                .any(|pair| position(pair[0].1) == position(pair[1].1))
            {
                report.refuse(format!(
                    "{} native directional list positions are duplicated",
                    layer.name()
                ));
                continue;
            }
        }
        if layer.groups_contracts() {
            // Native equivalence ignores country keys, port provinces and path caches.
            // Keep the first native occurrence and combine every equivalent contract's colour.
            let mut index: BTreeMap<(u8, u32, u32), usize> = BTreeMap::new();
            let mut groups: Vec<(RouteKey, bool)> = Vec::new();
            for (_, client) in clients.iter() {
                let key = route_of(*client);
                let Some(&route) = routes.get(&key) else {
                    report.lines[layer.index()] += 1;
                    report.refuse(format!(
                        "a {} line whose route has no TradeRoutes row",
                        layer.name()
                    ));
                    continue;
                };
                if !layer.admits(route.kind()) {
                    report.lines[layer.index()] += 1;
                    report.filtered += 1;
                    continue;
                }
                let (Some(source), Some(destination)) =
                    (route.source_state(), route.destination_state())
                else {
                    report.lines[layer.index()] += 1;
                    report.refuse(format!(
                        "a {} line's native endpoint states unavailable",
                        layer.name()
                    ));
                    continue;
                };
                let identity = (route.kind(), source, destination);
                match index.get(&identity) {
                    Some(&at) => groups[at].1 |= client.bad(),
                    None => {
                        index.insert(identity, groups.len());
                        groups.push((key, client.bad()));
                    }
                }
            }
            for (key, bad) in groups {
                push_line(&routes, &mut out, report, layer, key, bad, false, 0, Assigned::None, 0);
            }
            continue;
        }
        for &(at, client) in clients.iter() {
            // Membership names the lend-lease giver, but its native convoy client names
            // the receiver for direction. Imports already name their importer in owner.
            let direction_country = match client.category() {
                LEND_LEASE => client.other(),
                _ => client.owner(),
            };
            let reverse = match client.category() {
                IMPORT | LEND_LEASE => {
                    match countries.same(direction_country, client.route_country()) {
                        Ok(reverse) => reverse,
                        Err(reason) => {
                            report.refuse(format!("a {} line's direction: {reason}", layer.name()));
                            continue;
                        }
                    }
                }
                _ => false,
            };
            push_line(
                &routes,
                &mut out,
                report,
                layer,
                route_of(client),
                client.bad(),
                reverse,
                client.convoys_assigned(),
                Assigned::Client(at),
                client.handle(),
            );
        }
    }
    out
}

/// The viewed country's recorded delivery efficiency in each sea region.
pub fn regional_efficiency(
    regional: &[impl RegionalFields],
    countries: &Countries<'_>,
    report: &mut TradeRouteReport,
) -> BTreeMap<u32, i64> {
    let mut out = BTreeMap::new();
    for row in regional {
        match countries.viewed(row.country()) {
            Ok(true) => {
                out.entry(row.region()).or_insert(row.efficiency());
            }
            Ok(false) => {}
            Err(reason) => {
                report.refuse(format!("a regional convoy efficiency's country: {reason}"))
            }
        }
    }
    out
}

/// Each province's texture variant is 1 (hotspot) for a sea region of a bad naval line where
/// the viewed country's efficiency is below the threshold, else 0. The crossing control
/// between two provinces takes their minimum variant, including at the destination.
pub fn control_variants<R: RouteFields>(
    line: &Line<'_, R>,
    stops: &Stops,
    efficiency: &BTreeMap<u32, i64>,
    threshold: i64,
) -> Vec<u8> {
    let provinces: Vec<u8> = stops
        .provinces
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let value = stops
                .regions
                .get(index)
                .and_then(|region| efficiency.get(region))
                .copied()
                .unwrap_or(FULL_EFFICIENCY);
            u8::from(line.bad && line.route.kind() == NAVAL_ROUTE && value < threshold)
        })
        .collect();
    let mut controls = Vec::with_capacity(provinces.len().saturating_mul(2).saturating_sub(1));
    for (index, &variant) in provinces.iter().enumerate() {
        if index > 0 {
            controls.push(provinces[index - 1].min(variant));
        }
        controls.push(variant);
    }
    controls
}

/// What [`build`] needs beyond the frame.
pub struct Inputs<'a> {
    pub orders: &'a OrderAssets,
    pub defines: Result<&'a TradeRouteDefines, &'a str>,
    pub content: &'a RouteContent<'a>,
    pub heights: &'a HeightField<'a>,
    /// The viewed country's canonical slot.
    pub ours: u32,
    /// Whether the content's `gfx` descriptors declare an entity.
    pub declared: &'a dyn Fn(&str) -> bool,
}

/// One hour's lines for the shared arrow manager ([`viewer_foundation::map::ribbon_mesh::append`])
/// and the convoy lines the unit models draw.
#[derive(Clone, Debug, Default)]
pub struct TradeRoutes {
    pub field: OrderField,
    pub convoys: Vec<ConvoyLine>,
    /// Every convoy-carrying line that drew, in the order the convoys are numbered in, whatever
    /// its count: what [`sailed`] makes the convoys again from.
    pub sailing: Vec<Sailing>,
    /// The entity every convoy of the hour draws, resolved from the viewed country's graphical
    /// culture while a line's count asked for it; none when none did or it was refused.
    pub convoy_entity: Option<String>,
    pub report: TradeRouteReport,
}

/// The convoys of lines already built, with the counts `origins` and `clients` (each table's
/// convoys assigned, in row order) give them now: the lines, their geometry and their order are
/// the hour's as long as nothing but those columns moved. None when a line that carries convoys
/// now drew none when it was built, so its entity was never resolved and the lines are built whole.
pub fn sailed(kept: &TradeRoutes, defines: &ConvoyDefines, origins: &[i32], clients: &[i32]) -> Option<Vec<ConvoyLine>> {
    let mut out: Vec<ConvoyLine> = Vec::new();
    for line in &kept.sailing {
        let count = convoy_count(line.assigned.of(origins, clients), defines);
        if count == 0 {
            continue;
        }
        out.push(ConvoyLine {
            entity: kept.convoy_entity.clone()?,
            seed: out.len() as u32,
            map_width: line.map_width,
            points: line.points.clone(),
            count,
        });
    }
    Some(out)
}

/// What a route's stops are searched from, kept from one hour's lines to the next's: a route's
/// stops are an A* over the content's province graph, and the rows deciding them move in few
/// hours, while the lines are built again in most ([`geometry::directed_stops`]).
#[derive(Default)]
pub struct KeptStops {
    before: BTreeMap<(RouteKey, bool), (RouteRead, Option<Stops>)>,
    now: BTreeMap<(RouteKey, bool), (RouteRead, Option<Stops>)>,
}

/// What a route's stops read of its row.
type RouteRead = (u8, u32, u32, Vec<u32>);

impl KeptStops {
    /// A build begins: what the build before kept is what this one may keep, and what it does not
    /// ask for is let go with it.
    pub fn begin(&mut self) {
        self.before = std::mem::take(&mut self.now);
    }

    /// `route`'s stops, searched only when its own row is not the one they were searched from. A
    /// route whose search is refused is searched again, so its refusal is reported every build.
    pub fn of<R: RouteFields>(&mut self, route: &R, reverse: bool, content: &RouteContent<'_>) -> Result<Option<Stops>, String> {
        let key = ((route.country(), route.class(), route.other()), reverse);
        let read: RouteRead = (route.kind(), route.from(), route.to(), route.path().to_vec());
        if let Some((held, stops)) = self.now.get(&key).or_else(|| self.before.get(&key))
            && *held == read
        {
            let stops = stops.clone();
            self.now.insert(key, (read, stops.clone()));
            viewer_foundation::view_record::add("trade routes", "stops kept", 1);
            return Ok(stops);
        }
        viewer_foundation::view_record::add("trade routes", "stops searched", 1);
        let stops = geometry::directed_stops(route, content, reverse)?;
        self.now.insert(key, (read, stops.clone()));
        Ok(stops)
    }
}

/// Whether a map mode shows the lines.
pub fn shown(visibility: &Visibility) -> bool {
    visibility.trade_routes
}

/// The viewed country's lines and convoys of the hour.
/// The viewed country's lines and convoys of the hour; `stops` keeps each route's stops from the
/// build before, where its caller has one to keep them in.
pub fn build(inputs: &Inputs<'_>, frame: &DecodedFrame, stops: Option<&mut KeptStops>) -> TradeRoutes {
    let mut out = TradeRoutes::default();
    if let Err(reason) = build_into(inputs, frame, stops, &mut out) {
        out.report.unavailable = Some(reason);
    }
    out
}

fn build_into(
    inputs: &Inputs<'_>,
    frame: &DecodedFrame,
    mut kept: Option<&mut KeptStops>,
    out: &mut TradeRoutes,
) -> Result<(), String> {
    inputs
        .defines
        .map_err(|reason| format!("the trade-route defines cannot be read: {reason}"))?;
    inputs
        .orders
        .types
        .get(TRADE_ROUTE_TYPE_NAME)
        .ok_or_else(|| format!("{MAPARROWS} declares no arrowType {TRADE_ROUTE_TYPE_NAME}"))?;
    let complete = |error: io::Error| error.to_string();
    let tables: Tables<'_> = Tables {
        origins: frame
            .complete::<tables::ResourceOrigins>()
            .map_err(complete)?,
        clients: frame
            .complete::<tables::ConvoyClients>()
            .map_err(complete)?,
        routes: frame.complete::<tables::TradeRoutes>().map_err(complete)?,
        regional: frame
            .complete::<tables::RegionalConvoyEfficiency>()
            .map_err(complete)?,
    };
    let countries = Countries::new(
        table_access::rows::<tables::CountryLinks>(frame).map_err(complete)?,
        inputs.ours,
    );
    let exchanges = frame.complete::<tables::ResourceExchanges>();
    let exchange_order: BTreeMap<u32, ExchangeOrder> = exchanges
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let order = ExchangeOrder { resource: row.resource, import_ordinal: row.import_ordinal, export_ordinal: row.export_ordinal };
                    ((row.id >> 32) as u32, order)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut stops = move |route: &TradeRouteRow, reverse| match kept.as_deref_mut() {
        Some(kept) => kept.of(route, reverse, inputs.content),
        None => geometry::directed_stops(route, inputs.content, reverse),
    };
    build_lines(
        inputs,
        &tables,
        &countries,
        &exchange_order,
        &|countries| convoys::viewed_culture(frame, countries),
        &mut stops,
        &mut geometry::emit,
        out,
    )
}

/// What draws one line's ribbon into the field: [`emit`], or a caller's that keeps unchanged ribbons.
pub type Emit<'a> = dyn FnMut(&mut OrderField, &viewer_foundation::map::ribbon_content::ArrowType, &Shaped, [f32; 4], [f32; 4], &HeightField<'_>) -> Option<Vec<[f32; 2]>> + 'a;

/// The viewed country's lines and convoys from the tables' values: `culture` gives the viewed
/// country's graphical culture, and `stops` a route's stops (the content's search; a caller may
/// keep them while the route is unchanged).
#[allow(clippy::too_many_arguments, reason = "the tables, the countries, the exchanges and the two content callbacks are each the caller's")]
pub fn build_lines<O: OriginFields, C: ClientFields, R: RouteFields, G: RegionalFields>(
    inputs: &Inputs<'_>,
    tables: &Tables<'_, O, C, R, G>,
    countries: &Countries<'_>,
    exchange_order: &BTreeMap<u32, ExchangeOrder>,
    culture: &dyn Fn(&Countries<'_>) -> Result<String, String>,
    stops: &mut dyn FnMut(&R, bool) -> Result<Option<Stops>, String>,
    emit: &mut Emit<'_>,
    out: &mut TradeRoutes,
) -> Result<(), String> {
    let defines = inputs
        .defines
        .map_err(|reason| format!("the trade-route defines cannot be read: {reason}"))?;
    let arrow = inputs
        .orders
        .types
        .get(TRADE_ROUTE_TYPE_NAME)
        .ok_or_else(|| format!("{MAPARROWS} declares no arrowType {TRADE_ROUTE_TYPE_NAME}"))?;
    let report = &mut out.report;
    let mut lines = lines(tables, countries, report);
    lines.retain(|line| {
        if matches!(line.layer, Layer::Import | Layer::Export)
            && !exchange_order.contains_key(&line.client)
        {
            report.refuse(format!(
                "a {} line's native exchange order: ResourceExchanges client {} unavailable",
                line.layer.name(),
                line.client
            ));
            false
        } else {
            true
        }
    });
    lines.sort_by_key(|line| {
        let rank = match line.layer {
            Layer::Production => 0,
            Layer::Import => 1,
            Layer::Export => 2,
            Layer::LendLeaseReceived => 3,
            Layer::LendLeaseGiven => 4,
            Layer::MarketBought => 5,
            Layer::MarketSold => 6,
            Layer::SupplyTransfer => 7,
        };
        let order = exchange_order
            .get(&line.client)
            .map_or((0, 0), |row| match line.layer {
                Layer::Import => (0, row.import_ordinal),
                Layer::Export => (row.resource, row.export_ordinal),
                _ => (0, 0),
            });
        (rank, order)
    });
    if lines.is_empty() {
        return Ok(());
    }
    let efficiency = regional_efficiency(tables.regional, countries, report);
    let mut entity: Option<Result<String, String>> = None;
    let mut overlaps: BTreeMap<u32, u8> = BTreeMap::new();
    let width = inputs.heights.width as f32;
    for line in &lines {
        let stops = match stops(line.route, line.reverse) {
            Ok(Some(stops)) => stops,
            Ok(None) => {
                report.empty += 1;
                continue;
            }
            Err(reason) => {
                report.refuse(format!("a {} line's route: {reason}", line.layer.name()));
                continue;
            }
        };
        let uses = geometry::province_uses(&stops, &mut overlaps);
        let points = match geometry::points(
            &stops,
            inputs.content,
            width,
            &uses,
            defines.max_lines,
            defines.line_offset,
        ) {
            Ok(points) => points,
            Err(reason) => {
                report.refuse(format!("a {} line's route: {reason}", line.layer.name()));
                continue;
            }
        };
        let variants = control_variants(line, &stops, &efficiency, defines.regional_threshold);
        let shaped = geometry::shape(&points, &variants, false, defines.smoothing);
        let color = defines.line_color(line.layer, line.bad);
        let Some(_) = emit(
            &mut out.field,
            arrow,
            &shaped,
            color,
            defines.hotspot_color,
            inputs.heights,
        ) else {
            report.empty += 1;
            continue;
        };
        report.drawn += 1;
        let scale = arrow.global_scale(shaped.controls.len());
        out.field
            .trade_route_hovers
            .extend(shaped.controls.iter().map(|&point| {
                let [x, z] = viewer_foundation::map::ribbon_fixed::to_world(point);
                [
                    x,
                    viewer_foundation::map::ribbon_mesh::ribbon_height(inputs.heights, x, z),
                    z,
                    arrow.body_width[0] * scale * arrow.body_uv_scale * defines.intersect_distance,
                ]
            }));
        if shaped.variants.iter().any(|&variant| variant != 0) {
            report.hotspots += 1;
        }
        if !line.layer.carries_convoys() {
            continue;
        }
        let count = convoy_count(line.convoys_assigned, &defines.convoys);
        let points: Vec<[f32; 2]> = shaped.controls.iter().copied().map(viewer_foundation::map::ribbon_fixed::to_world).collect();
        // The entity is resolved for the first line whose convoys ask for it, and never for an
        // hour that sails none; a line that drew none keeps its controls, so the hour the count
        // moves makes its convoys from them ([`sailed`]).
        if count == 0 {
            out.sailing.push(Sailing { assigned: line.assigned, points, map_width: width });
            continue;
        }
        let entity = entity.get_or_insert_with(|| {
            culture(countries).and_then(|culture| convoy_entity(&culture, inputs.declared))
        });
        match entity {
            Ok(entity) => {
                report.convoy_lines += 1;
                report.convoys += count as usize;
                out.convoys.push(ConvoyLine { entity: entity.clone(), seed: out.convoys.len() as u32, map_width: width, points: points.clone(), count });
                out.convoy_entity = Some(entity.clone());
                out.sailing.push(Sailing { assigned: line.assigned, points, map_width: width });
            }
            Err(reason) => {
                out.sailing.push(Sailing { assigned: line.assigned, points, map_width: width });
                report.refuse(format!("convoys of a {} line: {reason}", line.layer.name()))
            }
        }
    }
    viewer_foundation::map::ribbon_mesh::sort_draws(&mut out.field.draws);
    Ok(())
}
