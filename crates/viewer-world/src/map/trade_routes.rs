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
pub use geometry::{RouteContent, Shaped, Stops, border_province, overlap_offset, stops};

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
    links: Available<'a, tables::CountryLinks>,
    slots: BTreeMap<u32, u32>,
    ours: u32,
}

impl<'a> Countries<'a> {
    /// `ours` is the viewed country's canonical slot.
    pub fn new(links: Available<'a, tables::CountryLinks>, ours: u32) -> Self {
        let slots = links
            .rows()
            .iter()
            .map(|link: &CountryLinkRow| (link.id, link.canonical_slot))
            .collect();
        Self { links, slots, ours }
    }

    /// The canonical slot of a reference; `None` for 0, no country.
    pub fn slot(&self, country: u32) -> Result<Option<u32>, String> {
        if country == 0 {
            return Ok(None);
        }
        match self.slots.get(&country) {
            Some(&slot) => Ok(Some(slot)),
            None => table_access::country_slot(&self.links, country),
        }
    }

    pub fn viewed(&self, country: u32) -> Result<bool, String> {
        Ok(self.slot(country)? == Some(self.ours))
    }

    pub fn same(&self, a: u32, b: u32) -> Result<bool, String> {
        let a = self.slot(a)?;
        Ok(a.is_some() && a == self.slot(b)?)
    }
}

/// The recorded tables the lines read.
#[derive(Clone, Copy)]
pub struct Tables<'a> {
    pub origins: &'a [ResourceOriginRow],
    pub clients: &'a [ConvoyClientRow],
    pub routes: &'a [TradeRouteRow],
    pub regional: &'a [RegionalConvoyEfficiencyRow],
}

/// One line of the viewed country.
#[derive(Clone, Debug, PartialEq)]
pub struct Line<'a> {
    pub layer: Layer,
    pub route: &'a TradeRouteRow,
    pub bad: bool,
    /// Whether the line runs from the route's destination to its source.
    pub reverse: bool,
    /// The convoys assigned to the line's client; none on market lines.
    pub convoys_assigned: i32,
    /// The client handle joining native exchange ordinals.
    pub client: u32,
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

fn route_of(client: &ConvoyClientRow) -> RouteKey {
    (client.route_country, client.route_class, client.route_other)
}

/// The layers a client draws in for the viewed country.
fn client_layers(
    client: &ConvoyClientRow,
    countries: &Countries<'_>,
) -> Result<Vec<Layer>, String> {
    let (owner_side, other_side) = match client.category {
        SUPPLY_TRANSFER => (Layer::SupplyTransfer, None),
        IMPORT => (Layer::Import, Some(Layer::Export)),
        LEND_LEASE => (Layer::LendLeaseGiven, Some(Layer::LendLeaseReceived)),
        MARKET => (Layer::MarketSold, Some(Layer::MarketBought)),
        other => return Err(format!("convoy client category {other} has no layer")),
    };
    let mut layers = Vec::new();
    if countries.viewed(client.owner)? {
        layers.push(owner_side);
    }
    if let Some(layer) = other_side
        && countries.viewed(client.other)?
    {
        layers.push(layer);
    }
    Ok(layers)
}

#[allow(clippy::too_many_arguments)]
fn push_line<'a>(
    routes: &BTreeMap<RouteKey, &'a TradeRouteRow>,
    out: &mut Vec<Line<'a>>,
    report: &mut TradeRouteReport,
    layer: Layer,
    key: RouteKey,
    bad: bool,
    reverse: bool,
    convoys_assigned: i32,
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
    if !layer.admits(route.kind) {
        report.filtered += 1;
        return;
    }
    out.push(Line {
        layer,
        route,
        bad,
        reverse,
        convoys_assigned,
        client,
    });
}

/// The viewed country's lines by layer and native directional list order. Exchanges
/// receive their existing resource/import/export order in the final displacement pass.
pub fn lines<'a>(
    tables: &Tables<'a>,
    countries: &Countries<'_>,
    report: &mut TradeRouteReport,
) -> Vec<Line<'a>> {
    let routes: BTreeMap<RouteKey, &'a TradeRouteRow> = tables
        .routes
        .iter()
        .map(|route| ((route.country, route.class, route.other), route))
        .collect();
    let mut out = Vec::new();
    let mut origins: Vec<_> = tables
        .origins
        .iter()
        .filter(|row| row.list == OWN_ORIGINS)
        .collect();
    origins.sort_by_key(|row| row.ordinal);
    // Only the viewed list participates; other countries may have the same ordinals.
    origins.retain(|row| match countries.viewed(row.country) {
        Ok(viewed) => viewed,
        Err(reason) => {
            report.refuse(format!("a resource origin's country: {reason}"));
            false
        }
    });
    if origins
        .windows(2)
        .any(|pair| pair[0].ordinal == pair[1].ordinal)
    {
        report.refuse("production native origin list positions are duplicated");
        origins.clear();
    }
    for origin in origins {
        if origin.list != OWN_ORIGINS {
            continue;
        }
        match countries.viewed(origin.country) {
            Ok(true) => push_line(
                &routes,
                &mut out,
                report,
                Layer::Production,
                (origin.country, ORIGIN_ROUTE_CLASS, origin.state),
                origin.bad,
                false,
                origin.convoys_assigned,
                0,
            ),
            Ok(false) => {}
            Err(reason) => report.refuse(format!("a resource origin's country: {reason}")),
        }
    }
    let mut by_layer: [Vec<&ConvoyClientRow>; 8] = Default::default();
    for client in tables.clients {
        match client_layers(client, countries) {
            Ok(layers) => {
                for layer in layers {
                    by_layer[layer.index()].push(client);
                }
            }
            Err(reason) => report.refuse(format!("a convoy client: {reason}")),
        }
    }
    for layer in &Layer::ALL[1..] {
        let layer = *layer;
        let clients = &mut by_layer[layer.index()];
        if !matches!(layer, Layer::Import | Layer::Export) {
            let position = |row: &ConvoyClientRow| match layer {
                Layer::LendLeaseReceived | Layer::MarketBought => row.other_ordinal,
                _ => row.owner_ordinal,
            };
            clients.retain(|row| {
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
            clients.sort_by_key(|row| position(row).unwrap());
            if clients
                .windows(2)
                .any(|pair| position(pair[0]) == position(pair[1]))
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
            for client in clients.iter() {
                let key = route_of(client);
                let Some(&route) = routes.get(&key) else {
                    report.lines[layer.index()] += 1;
                    report.refuse(format!(
                        "a {} line whose route has no TradeRoutes row",
                        layer.name()
                    ));
                    continue;
                };
                if !layer.admits(route.kind) {
                    report.lines[layer.index()] += 1;
                    report.filtered += 1;
                    continue;
                }
                let (Some(source), Some(destination)) =
                    (route.source_state, route.destination_state)
                else {
                    report.lines[layer.index()] += 1;
                    report.refuse(format!(
                        "a {} line's native endpoint states unavailable",
                        layer.name()
                    ));
                    continue;
                };
                let identity = (route.kind, source, destination);
                match index.get(&identity) {
                    Some(&at) => groups[at].1 |= client.bad,
                    None => {
                        index.insert(identity, groups.len());
                        groups.push((key, client.bad));
                    }
                }
            }
            for (key, bad) in groups {
                push_line(&routes, &mut out, report, layer, key, bad, false, 0, 0);
            }
            continue;
        }
        for client in clients.iter() {
            // Membership names the lend-lease giver, but its native convoy client names
            // the receiver for direction. Imports already name their importer in owner.
            let direction_country = match client.category {
                LEND_LEASE => client.other,
                _ => client.owner,
            };
            let reverse = match client.category {
                IMPORT | LEND_LEASE => {
                    match countries.same(direction_country, client.route_country) {
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
                client.bad,
                reverse,
                client.convoys_assigned,
                client.handle,
            );
        }
    }
    out
}

/// The viewed country's recorded delivery efficiency in each sea region.
pub fn regional_efficiency(
    regional: &[RegionalConvoyEfficiencyRow],
    countries: &Countries<'_>,
    report: &mut TradeRouteReport,
) -> BTreeMap<u32, i64> {
    let mut out = BTreeMap::new();
    for row in regional {
        match countries.viewed(row.country) {
            Ok(true) => {
                out.entry(row.region).or_insert(row.efficiency);
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
pub fn control_variants(
    line: &Line<'_>,
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
            u8::from(line.bad && line.route.kind == NAVAL_ROUTE && value < threshold)
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
    pub report: TradeRouteReport,
}

/// Whether a map mode shows the lines.
pub fn shown(visibility: &Visibility) -> bool {
    visibility.trade_routes
}

/// The viewed country's lines and convoys of the hour.
pub fn build(inputs: &Inputs<'_>, frame: &DecodedFrame) -> TradeRoutes {
    let mut out = TradeRoutes::default();
    if let Err(reason) = build_into(inputs, frame, &mut out) {
        out.report.unavailable = Some(reason);
    }
    out
}

fn build_into(
    inputs: &Inputs<'_>,
    frame: &DecodedFrame,
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
    let complete = |error: io::Error| error.to_string();
    let tables = Tables {
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
    let report = &mut out.report;
    let mut lines = lines(&tables, &countries, report);
    let exchanges = frame.complete::<tables::ResourceExchanges>();
    let exchange_order: BTreeMap<u32, _> = exchanges
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|row| ((row.id >> 32) as u32, row))
                .collect()
        })
        .unwrap_or_default();
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
    let efficiency = regional_efficiency(tables.regional, &countries, report);
    let mut entity: Option<Result<String, String>> = None;
    let mut overlaps: BTreeMap<u32, u8> = BTreeMap::new();
    let width = inputs.heights.width as f32;
    for line in &lines {
        let stops = match geometry::directed_stops(line.route, inputs.content, line.reverse) {
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
        let Some(_) = geometry::emit(
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
        let count = if line.layer.carries_convoys() {
            convoy_count(line.convoys_assigned, &defines.convoys)
        } else {
            0
        };
        if count == 0 {
            continue;
        }
        let entity = entity.get_or_insert_with(|| {
            convoys::viewed_culture(frame, &countries)
                .and_then(|culture| convoy_entity(&culture, inputs.declared))
        });
        match entity {
            Ok(entity) => {
                report.convoy_lines += 1;
                report.convoys += count as usize;
                out.convoys.push(ConvoyLine {
                    entity: entity.clone(),
                    seed: out.convoys.len() as u32,
                    map_width: width,
                    points: shaped
                        .controls
                        .iter()
                        .copied()
                        .map(viewer_foundation::map::ribbon_fixed::to_world)
                        .collect(),
                    count,
                });
            }
            Err(reason) => {
                report.refuse(format!("convoys of a {} line: {reason}", line.layer.name()))
            }
        }
    }
    viewer_foundation::map::ribbon_mesh::sort_draws(&mut out.field.draws);
    Ok(())
}
