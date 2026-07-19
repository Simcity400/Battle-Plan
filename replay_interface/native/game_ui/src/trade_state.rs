//! XUI15 country-resource trade admission and rendering.
//!
//! This module consumes only `replay_core::TradeView`. It never reconstructs
//! controller values from aggregate pools and never substitutes equipment-
//! market contracts for classic resource routes.

use crate::assets::AssetResolver;
use crate::compose::{ComposeOptions, ComposeOutput, Compositor};
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::gui_scene_util::{
    load_named_element, pair, prefix_element_names, promote_show_position, set_position_upper_left,
};
use crate::loc::Localisation;
use crate::paradox_text::Value;
use crate::Result;
use replay_core::{Leaf, ResourceTradeDealRow, TradeResourceRow, TradeView, ViewStatus};
use std::collections::{BTreeMap, BTreeSet};

pub const TRADE_RESOURCE_ORDER: [&str; 7] = [
    "oil",
    "aluminium",
    "rubber",
    "tungsten",
    "steel",
    "chromium",
    "coal",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeScreenUnavailable {
    DomainAbsent,
    DomainUnavailable,
    CountryAbsent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeDataIssue {
    NotRecorded,
    CapturedButUnobtained,
    IncompleteWitness,
    InvalidExactOrder,
    IncompleteRow,
    ArithmeticOverflow,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TradeData<T> {
    Available(T),
    Unavailable(TradeDataIssue),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeResourcePoolCard {
    pub resource: String,
    pub produced: f64,
    pub imported: f64,
    pub used: f64,
    pub to_export: f64,
    pub base_export: f64,
    pub exported: f64,
    pub available_derived: TradeData<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeResourceCard {
    pub resource: String,
    pub visible_order: i64,
    pub balance: f64,
    pub produced: f64,
    pub imported: f64,
    pub exported: f64,
    pub special_project: f64,
    pub production_line: f64,
    pub needed: f64,
    pub needed_current: Option<f64>,
    pub needed_max: Option<f64>,
    pub needed_type: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeDealCard {
    pub deal_id: i64,
    pub counterpart: String,
    pub direction: String,
    pub resource: String,
    pub requested: f64,
    pub delivered: f64,
    pub available_export: f64,
    pub civilian_factories: i64,
    pub factory_cost_per_resource: f64,
    pub convoys_assigned: i64,
    pub convoys_required: i64,
    pub no_convoys: bool,
    pub no_trade_influence: TradeData<bool>,
    pub no_resources: TradeData<bool>,
    pub route_kind: String,
    pub delivery_efficiency: f64,
    pub visible_order: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeFactoryHeader {
    pub total: i64,
    pub used: i64,
    pub available: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeResourceTotals {
    pub imported: f64,
    pub exported: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeDealTotals {
    pub requested: f64,
    pub delivered: f64,
    pub civilian_factories: i64,
    pub convoys_assigned: i64,
    pub convoys_required: i64,
    pub partners: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradePresentation {
    pub country_tag: String,
    pub resource_pools: TradeData<Vec<TradeResourcePoolCard>>,
    pub resource_cards: TradeData<Vec<TradeResourceCard>>,
    pub deals: TradeData<Vec<TradeDealCard>>,
    pub factories: TradeData<TradeFactoryHeader>,
    pub resource_totals: TradeData<TradeResourceTotals>,
    pub deal_totals: TradeData<TradeDealTotals>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AdmittedTradeScreen {
    Unavailable(TradeScreenUnavailable),
    Visible(TradePresentation),
}

pub fn admit_trade_screen(
    status: ViewStatus,
    country_tag: &str,
    view: Option<&TradeView>,
) -> AdmittedTradeScreen {
    match status {
        ViewStatus::DomainAbsent => {
            return AdmittedTradeScreen::Unavailable(TradeScreenUnavailable::DomainAbsent)
        }
        ViewStatus::DomainUnavailable => {
            return AdmittedTradeScreen::Unavailable(TradeScreenUnavailable::DomainUnavailable)
        }
        ViewStatus::Present => {}
    }
    let Some(view) = view else {
        return AdmittedTradeScreen::Unavailable(TradeScreenUnavailable::CountryAbsent);
    };

    let resource_pools = admit_resource_pools(view);
    let resource_cards = admit_resource_cards(view);
    let deals = admit_deals(view);
    let factories = admit_factories(view);
    let resource_totals = match &resource_cards {
        TradeData::Available(rows) => TradeData::Available(TradeResourceTotals {
            imported: rows.iter().map(|row| row.imported).sum(),
            exported: rows.iter().map(|row| row.exported).sum(),
        }),
        TradeData::Unavailable(issue) => TradeData::Unavailable(*issue),
    };
    let deal_totals = match &deals {
        TradeData::Available(rows) => compute_deal_totals(rows),
        TradeData::Unavailable(issue) => TradeData::Unavailable(*issue),
    };

    AdmittedTradeScreen::Visible(TradePresentation {
        country_tag: country_tag.to_string(),
        resource_pools,
        resource_cards,
        deals,
        factories,
        resource_totals,
        deal_totals,
    })
}

fn admit_resource_pools(view: &TradeView) -> TradeData<Vec<TradeResourcePoolCard>> {
    if let Some(issue) = completeness_issue(&view.resource_pools_complete) {
        return TradeData::Unavailable(issue);
    }
    let maps = [
        &view.produced,
        &view.imported,
        &view.used,
        &view.to_export,
        &view.base_export,
        &view.exported,
    ];
    let mut measured = Vec::with_capacity(maps.len());
    for map in maps {
        let Leaf::Measured(map) = map else {
            return TradeData::Unavailable(leaf_issue(map));
        };
        if map.len() != TRADE_RESOURCE_ORDER.len()
            || !TRADE_RESOURCE_ORDER
                .iter()
                .all(|resource| map.contains_key(*resource))
        {
            return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
        }
        measured.push(map);
    }
    let available = match &view.available_derived {
        Leaf::Measured(values) => Some(values),
        _ => None,
    };
    TradeData::Available(
        TRADE_RESOURCE_ORDER
            .iter()
            .map(|resource| TradeResourcePoolCard {
                resource: (*resource).to_string(),
                produced: measured[0][*resource],
                imported: measured[1][*resource],
                used: measured[2][*resource],
                to_export: measured[3][*resource],
                base_export: measured[4][*resource],
                exported: measured[5][*resource],
                available_derived: available
                    .and_then(|values| values.get(*resource).copied())
                    .map(TradeData::Available)
                    .unwrap_or_else(|| TradeData::Unavailable(leaf_issue(&view.available_derived))),
            })
            .collect(),
    )
}

fn admit_resource_cards(view: &TradeView) -> TradeData<Vec<TradeResourceCard>> {
    if let Some(issue) = completeness_issue(&view.resource_rows_complete) {
        return TradeData::Unavailable(issue);
    }
    let Leaf::Measured(rows) = &view.resource_rows else {
        return TradeData::Unavailable(leaf_issue(&view.resource_rows));
    };
    if rows.len() != TRADE_RESOURCE_ORDER.len() {
        return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
    }
    let mut cards = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let expected = TRADE_RESOURCE_ORDER[index];
        let Some(card) = measured_resource_card(row, expected, index) else {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        };
        cards.push(card);
    }
    TradeData::Available(cards)
}

fn measured_resource_card(
    row: &TradeResourceRow,
    expected_resource: &str,
    expected_order: usize,
) -> Option<TradeResourceCard> {
    let resource = measured(&row.resource)?.clone();
    let visible_order = *measured(&row.visible_order)?;
    if resource != expected_resource || visible_order != expected_order as i64 {
        return None;
    }
    let needed_current = row.needed_current.measured().copied();
    let needed_max = row.needed_max.measured().copied();
    if expected_resource == "oil" && (needed_current.is_none() || needed_max.is_none()) {
        return None;
    }
    Some(TradeResourceCard {
        resource,
        visible_order,
        balance: *measured(&row.balance)?,
        produced: *measured(&row.produced)?,
        imported: *measured(&row.imported)?,
        exported: *measured(&row.exported)?,
        special_project: *measured(&row.special_project)?,
        production_line: *measured(&row.production_line)?,
        needed: *measured(&row.needed)?,
        needed_current,
        needed_max,
        needed_type: measured(&row.needed_type)?.clone(),
    })
}

fn admit_deals(view: &TradeView) -> TradeData<Vec<TradeDealCard>> {
    if let Some(issue) = completeness_issue(&view.resource_deals_complete) {
        return TradeData::Unavailable(issue);
    }
    let Leaf::Measured(rows) = &view.resource_deals else {
        return TradeData::Unavailable(leaf_issue(&view.resource_deals));
    };
    let mut seen_ids = BTreeSet::new();
    let mut cards = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Some(card) = measured_deal_card(row, index) else {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        };
        if card.deal_id < 0 || !seen_ids.insert(card.deal_id) {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        }
        cards.push(card);
    }
    TradeData::Available(cards)
}

fn measured_deal_card(row: &ResourceTradeDealRow, expected_order: usize) -> Option<TradeDealCard> {
    let visible_order = *measured(&row.visible_order)?;
    if visible_order != expected_order as i64 {
        return None;
    }
    let convoys_assigned = *measured(&row.convoys_assigned)?;
    let convoys_required = *measured(&row.convoys_required)?;
    Some(TradeDealCard {
        deal_id: *measured(&row.deal_id)?,
        counterpart: measured(&row.counterpart)?.clone(),
        direction: measured(&row.direction)?.clone(),
        resource: measured(&row.resource)?.clone(),
        requested: *measured(&row.requested)?,
        delivered: *measured(&row.delivered)?,
        available_export: *measured(&row.available_export)?,
        civilian_factories: *measured(&row.civilian_factories)?,
        factory_cost_per_resource: *measured(&row.factory_cost_per_resource)?,
        convoys_assigned,
        convoys_required,
        no_convoys: convoys_assigned < convoys_required,
        no_trade_influence: optional_bool(&row.no_trade_influence),
        no_resources: optional_bool(&row.no_resources),
        route_kind: measured(&row.route_kind)?.clone(),
        delivery_efficiency: *measured(&row.delivery_efficiency)?,
        visible_order,
    })
}

fn admit_factories(view: &TradeView) -> TradeData<TradeFactoryHeader> {
    match (
        &view.factories_total,
        &view.factories_used,
        &view.factories_available,
    ) {
        (Leaf::Measured(total), Leaf::Measured(used), Leaf::Measured(available)) => {
            TradeData::Available(TradeFactoryHeader {
                total: *total,
                used: *used,
                available: *available,
            })
        }
        (Leaf::Absent, _, _) | (_, Leaf::Absent, _) | (_, _, Leaf::Absent) => {
            TradeData::Unavailable(TradeDataIssue::NotRecorded)
        }
        _ => TradeData::Unavailable(TradeDataIssue::CapturedButUnobtained),
    }
}

fn compute_deal_totals(rows: &[TradeDealCard]) -> TradeData<TradeDealTotals> {
    let mut requested = 0.0;
    let mut delivered = 0.0;
    let mut civilian_factories = 0i64;
    let mut convoys_assigned = 0i64;
    let mut convoys_required = 0i64;
    let mut seen = BTreeSet::new();
    let mut partners = Vec::new();
    for row in rows {
        requested += row.requested;
        delivered += row.delivered;
        let Some(next) = civilian_factories.checked_add(row.civilian_factories) else {
            return TradeData::Unavailable(TradeDataIssue::ArithmeticOverflow);
        };
        civilian_factories = next;
        let Some(next) = convoys_assigned.checked_add(row.convoys_assigned) else {
            return TradeData::Unavailable(TradeDataIssue::ArithmeticOverflow);
        };
        convoys_assigned = next;
        let Some(next) = convoys_required.checked_add(row.convoys_required) else {
            return TradeData::Unavailable(TradeDataIssue::ArithmeticOverflow);
        };
        convoys_required = next;
        if seen.insert(row.counterpart.clone()) {
            partners.push(row.counterpart.clone());
        }
    }
    TradeData::Available(TradeDealTotals {
        requested,
        delivered,
        civilian_factories,
        convoys_assigned,
        convoys_required,
        partners,
    })
}

fn completeness_issue(witness: &Leaf<bool>) -> Option<TradeDataIssue> {
    match witness {
        Leaf::Measured(true) => None,
        Leaf::Measured(false) => Some(TradeDataIssue::IncompleteWitness),
        Leaf::Absent => Some(TradeDataIssue::NotRecorded),
        Leaf::Unobtained => Some(TradeDataIssue::CapturedButUnobtained),
    }
}

fn leaf_issue<T>(leaf: &Leaf<T>) -> TradeDataIssue {
    match leaf {
        Leaf::Absent => TradeDataIssue::NotRecorded,
        Leaf::Unobtained => TradeDataIssue::CapturedButUnobtained,
        Leaf::Measured(_) => TradeDataIssue::IncompleteRow,
    }
}

fn measured<T>(leaf: &Leaf<T>) -> Option<&T> {
    leaf.measured()
}

fn optional_bool(leaf: &Leaf<bool>) -> TradeData<bool> {
    match leaf {
        Leaf::Measured(value) => TradeData::Available(*value),
        Leaf::Absent => TradeData::Unavailable(TradeDataIssue::NotRecorded),
        Leaf::Unobtained => TradeData::Unavailable(TradeDataIssue::CapturedButUnobtained),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeComposeScene {
    pub root: GuiElement,
    pub options: ComposeOptions,
}

pub fn build_trade_compose_scene(
    resolver: &AssetResolver,
    screen: &AdmittedTradeScreen,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<TradeComposeScene> {
    // The identical countrytradeview.gui document backs the root, every
    // resource card, and every deal row: parse it once per scene.
    let mut layout_cache: BTreeMap<String, LayoutSet> = BTreeMap::new();
    let mut country_trade = load_named_element(
        &mut layout_cache,
        resolver,
        "interface/countrytradeview.gui",
        "countrytradeview",
    )?;
    promote_show_position(&mut country_trade);
    let mut children = vec![country_trade];
    let localisation = Localisation::load_english(resolver)?;
    let mut options =
        ComposeOptions::new(viewport_width, viewport_height).with_localisation(&localisation);

    match screen {
        AdmittedTradeScreen::Unavailable(reason) => {
            children.push(unavailable_text(
                "xui15_screen_unavailable",
                116,
                205,
                screen_unavailable_text(*reason),
            ));
        }
        AdmittedTradeScreen::Visible(presentation) => {
            apply_trade_to_compose_options(presentation, &mut options);
            if let TradeData::Available(rows) = &presentation.resource_cards {
                for (index, _) in rows.iter().enumerate() {
                    let prefix = format!("xui15_resource_{index}_");
                    let mut element = load_named_element(
                        &mut layout_cache,
                        resolver,
                        "interface/countrytradeview.gui",
                        "resources_info_entry",
                    )?;
                    set_position_upper_left(&mut element, 106 + index as i32 * 79, 193);
                    prefix_element_names(&mut element, &prefix);
                    children.push(element);
                }
            } else {
                children.push(unavailable_text(
                    "xui15_resources_unavailable",
                    116,
                    205,
                    "RESOURCE DATA UNAVAILABLE",
                ));
            }
            if let TradeData::Available(rows) = &presentation.deals {
                for (index, _) in rows.iter().enumerate() {
                    let prefix = format!("xui15_deal_{index}_");
                    let mut element = load_named_element(
                        &mut layout_cache,
                        resolver,
                        "interface/countrytradeview.gui",
                        "country_trade_entry",
                    )?;
                    set_position_upper_left(&mut element, 9, 540 + index as i32 * 45);
                    prefix_element_names(&mut element, &prefix);
                    children.push(element);
                }
            } else {
                children.push(unavailable_text(
                    "xui15_deals_unavailable",
                    20,
                    552,
                    "CLASSIC RESOURCE DEALS UNAVAILABLE",
                ));
            }
        }
    }

    Ok(TradeComposeScene {
        root: wrapper_root(viewport_width, viewport_height, children),
        options,
    })
}

pub fn render_trade_screen(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    screen: &AdmittedTradeScreen,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<ComposeOutput> {
    let scene = build_trade_compose_scene(resolver, screen, viewport_width, viewport_height)?;
    let mut compositor = Compositor::new(resolver, registry);
    Ok(compositor.compose(&scene.root, &scene.options))
}

pub fn apply_trade_to_compose_options(
    presentation: &TradePresentation,
    options: &mut ComposeOptions,
) {
    match &presentation.factories {
        TradeData::Available(header) => {
            set_text(options, "factories_total", header.total.to_string());
            set_text(options, "factories_used", header.used.to_string());
            set_text(options, "factories_available", header.available.to_string());
        }
        TradeData::Unavailable(_) => {
            for name in ["factories_total", "factories_used", "factories_available"] {
                set_text(options, name, "--");
            }
        }
    }
    if let TradeData::Available(rows) = &presentation.resource_cards {
        for (index, row) in rows.iter().enumerate() {
            let prefix = format!("xui15_resource_{index}_");
            for (name, value) in [
                ("balance", format_trade_amount(row.balance)),
                ("produced", format_trade_amount(row.produced)),
                ("imported", format_trade_amount(row.imported)),
                ("exported", format_trade_amount(row.exported)),
                ("special_project", format_trade_amount(row.special_project)),
                ("production_line", format_trade_amount(row.production_line)),
                ("needed", format_needed(row)),
                ("neededType", row.needed_type.clone()),
            ] {
                set_text(options, &format!("{prefix}{name}"), value);
            }
            options
                .frame_overrides
                .insert(format!("{prefix}resources_icon"), index);
        }
    }
    if let TradeData::Available(rows) = &presentation.deals {
        for (index, row) in rows.iter().enumerate() {
            let prefix = format!("xui15_deal_{index}_");
            set_text(
                options,
                &format!("{prefix}country"),
                row.counterpart.clone(),
            );
            set_text(
                options,
                &format!("{prefix}exported"),
                format_trade_amount(row.delivered),
            );
            set_text(
                options,
                &format!("{prefix}available_export"),
                format_trade_amount(row.available_export),
            );
            set_text(
                options,
                &format!("{prefix}convoys"),
                format!("{}/{}", row.convoys_assigned, row.convoys_required),
            );
        }
    }
}

fn format_needed(row: &TradeResourceCard) -> String {
    if row.resource == "oil" {
        match (row.needed_current, row.needed_max) {
            (Some(current), Some(maximum)) => {
                format!(
                    "{}\n{}",
                    format_trade_amount(current),
                    format_trade_amount(maximum)
                )
            }
            _ => "--".to_string(),
        }
    } else {
        format_trade_amount(row.needed)
    }
}

pub fn format_trade_amount(value: f64) -> String {
    if value.fract() == 0.0 {
        return format!("{value:.0}");
    }
    let mut text = format!("{value:.5}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn set_text(options: &mut ComposeOptions, name: &str, value: impl Into<String>) {
    options
        .text_overrides
        .insert(name.to_string(), value.into());
}

fn wrapper_root(width: usize, height: usize, children: Vec<GuiElement>) -> GuiElement {
    GuiElement {
        element_type: "containerWindowType".to_string(),
        name: "xui15_trade_root".to_string(),
        position: None,
        size: Some(pair(width as i32, height as i32)),
        orientation: Some("UPPER_LEFT".to_string()),
        sprite_refs: Vec::new(),
        font: None,
        text: None,
        format: None,
        children,
        raw: Value::Object(Vec::new()),
    }
}

fn unavailable_text(name: &str, x: i32, y: i32, text: &str) -> GuiElement {
    GuiElement {
        element_type: "instantTextBoxType".to_string(),
        name: name.to_string(),
        position: Some(pair(x, y)),
        size: Some(pair(520, 24)),
        orientation: Some("UPPER_LEFT".to_string()),
        sprite_refs: Vec::new(),
        font: Some("hoi_20bs".to_string()),
        text: Some(text.to_string()),
        format: Some("center".to_string()),
        children: Vec::new(),
        raw: Value::Object(Vec::new()),
    }
}

fn screen_unavailable_text(reason: TradeScreenUnavailable) -> &'static str {
    match reason {
        TradeScreenUnavailable::DomainAbsent => "TRADE DATA NOT RECORDED",
        TradeScreenUnavailable::DomainUnavailable => "TRADE DATA UNAVAILABLE FOR THIS HOUR",
        TradeScreenUnavailable::CountryAbsent => "COUNTRY TRADE DATA UNAVAILABLE",
    }
}
