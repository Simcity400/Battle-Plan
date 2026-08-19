
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
use replay_core::{
    Leaf, MarketStockpileRow, ProductionLicenseAgreementRow, ProductionLicenseVariantRow,
    ResourceTradeDealRow, TradeContractRow, TradeResourceRow, TradeView, ViewStatus,
};
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

fn trade_resource_order(view: &TradeView) -> Option<Vec<String>> {
    let Leaf::Measured(rows) = &view.resource_rows else {
        return None;
    };
    rows.iter()
        .map(|row| row.resource.measured().cloned())
        .collect()
}

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
pub struct MarketStockpileCard {
    pub equipment_variant_id: u32,
    pub equipment_type: u32,
    pub amount_raw: i64,
    pub amount: f64,
    pub visible_order: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionLicenseVariantCard {
    pub variant_id: u32,
    pub visible_order: i64,
    pub creator_tag_db_id: u32,
    pub origin_tag_db_id: u32,
    pub source_tag_db_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgreementEndDate {
    Measured { raw: i64, date: String },
    Absent,
    Unavailable,
}

impl AgreementEndDate {
    pub fn display_text(&self) -> String {
        match self {
            AgreementEndDate::Measured { date, .. } => date.clone(),
            AgreementEndDate::Absent => String::new(),
            AgreementEndDate::Unavailable => "--".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionLicenseAgreementCard {
    pub agreement_id: String,
    pub visible_order: i64,
    pub owner: String,
    pub partner: String,
    pub start_date_raw: i64,
    pub start_date: String,
    pub end_date: AgreementEndDate,
    pub variants: Vec<ProductionLicenseVariantCard>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentPurchaseContractCard {
    pub contract_id: String,
    pub visible_order: i64,
    pub counterpart: String,
    pub direction: String,
    pub agreement_id: String,
    pub seller: String,
    pub buyer: String,
    pub start_date_raw: i64,
    pub start_date: String,
    pub end_date: AgreementEndDate,
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
    pub market_stockpile: TradeData<Vec<MarketStockpileCard>>,
    pub equipment_purchase_contracts: TradeData<Vec<EquipmentPurchaseContractCard>>,
    pub production_license_agreements: TradeData<Vec<ProductionLicenseAgreementCard>>,
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
    let market_stockpile = admit_market_stockpile(view);
    let equipment_purchase_contracts = admit_equipment_purchase_contracts(view);
    let production_license_agreements = admit_production_license_agreements(view);
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
        market_stockpile,
        equipment_purchase_contracts,
        production_license_agreements,
        factories,
        resource_totals,
        deal_totals,
    })
}

fn admit_resource_pools(view: &TradeView) -> TradeData<Vec<TradeResourcePoolCard>> {
    if let Some(issue) = completeness_issue(&view.resource_pools_complete) {
        return TradeData::Unavailable(issue);
    }
    let Some(order) = trade_resource_order(view) else {
        return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
    };
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
        if map.len() != order.len()
            || !order.iter().all(|resource| map.contains_key(resource))
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
        order
            .iter()
            .map(|resource| TradeResourcePoolCard {
                resource: resource.clone(),
                produced: measured[0][resource],
                imported: measured[1][resource],
                used: measured[2][resource],
                to_export: measured[3][resource],
                base_export: measured[4][resource],
                exported: measured[5][resource],
                available_derived: available
                    .and_then(|values| values.get(resource).copied())
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
    let mut cards = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Some(card) = measured_resource_card(row, index) else {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        };
        cards.push(card);
    }
    TradeData::Available(cards)
}

fn measured_resource_card(
    row: &TradeResourceRow,
    expected_order: usize,
) -> Option<TradeResourceCard> {
    let resource = measured(&row.resource)?.clone();
    let visible_order = *measured(&row.visible_order)?;
    if visible_order != expected_order as i64 {
        return None;
    }
    let balance = *measured(&row.balance)?;
    let produced = *measured(&row.produced)?;
    let imported = *measured(&row.imported)?;
    let exported = *measured(&row.exported)?;
    let special_project = *measured(&row.special_project)?;
    let production_line = *measured(&row.production_line)?;
    let needed = *measured(&row.needed)?;
    if ![
        balance,
        produced,
        imported,
        exported,
        special_project,
        production_line,
        needed,
    ]
    .into_iter()
    .all(exact_trade_i32_value)
    {
        return None;
    }
    let needed_current = row.needed_current.measured().copied();
    let needed_max = row.needed_max.measured().copied();
    let needed_type = measured(&row.needed_type)?.clone();
    let needed_semantics = if needed_type == "current_max" {
        needed_current.is_some_and(exact_trade_i32_value)
            && needed_max.is_some_and(exact_trade_i32_value)
            && needed_current == Some(needed)
    } else {
        matches!(&row.needed_current, Leaf::Absent)
            && matches!(&row.needed_max, Leaf::Absent)
            && needed == balance
            && ((balance.is_sign_negative() && needed_type == "deficit")
                || (!balance.is_sign_negative() && needed_type == "surplus"))
    };
    if !needed_semantics {
        return None;
    }
    Some(TradeResourceCard {
        resource,
        visible_order,
        balance,
        produced,
        imported,
        exported,
        special_project,
        production_line,
        needed,
        needed_current,
        needed_max,
        needed_type,
    })
}

fn exact_trade_i32_value(value: f64) -> bool {
    value.is_finite()
        && value.fract() == 0.0
        && value >= i32::MIN as f64
        && value <= i32::MAX as f64
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
        if !seen_ids.insert(card.deal_id) {
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
    let deal_id = *measured(&row.deal_id)?;
    let counterpart = measured(&row.counterpart)?.clone();
    let direction = measured(&row.direction)?.clone();
    let resource = measured(&row.resource)?.clone();
    let requested = *measured(&row.requested)?;
    let delivered = *measured(&row.delivered)?;
    let available_export = *measured(&row.available_export)?;
    let civilian_factories = *measured(&row.civilian_factories)?;
    let factory_cost_per_resource = *measured(&row.factory_cost_per_resource)?;
    let convoys_assigned = *measured(&row.convoys_assigned)?;
    let convoys_required = *measured(&row.convoys_required)?;
    let route_kind = measured(&row.route_kind)?.clone();
    let delivery_efficiency = *measured(&row.delivery_efficiency)?;
    if deal_id <= 0
        || counterpart.trim().is_empty()
        || !["import", "export"].contains(&direction.as_str())
        || resource.trim().is_empty()
        || !requested.is_finite()
        || requested.fract() != 0.0
        || !delivered.is_finite()
        || !available_export.is_finite()
        || available_export.fract() != 0.0
        || i32::try_from(civilian_factories).is_err()
        || !factory_cost_per_resource.is_finite()
        || i32::try_from(convoys_assigned).is_err()
        || i32::try_from(convoys_required).is_err()
        || !["land", "sea", "locked"].contains(&route_kind.as_str())
        || !delivery_efficiency.is_finite()
        || !(0.0..=1.0).contains(&delivery_efficiency)
        || !matches!(
            (&row.no_trade_influence, &row.no_resources),
            (Leaf::Measured(_), Leaf::Measured(_)) | (Leaf::Absent, Leaf::Absent)
        )
    {
        return None;
    }
    Some(TradeDealCard {
        deal_id,
        counterpart,
        direction,
        resource,
        requested,
        delivered,
        available_export,
        civilian_factories,
        factory_cost_per_resource,
        convoys_assigned,
        convoys_required,
        no_convoys: convoys_assigned < convoys_required,
        no_trade_influence: optional_bool(&row.no_trade_influence),
        no_resources: optional_bool(&row.no_resources),
        route_kind,
        delivery_efficiency,
        visible_order,
    })
}

fn admit_market_stockpile(view: &TradeView) -> TradeData<Vec<MarketStockpileCard>> {
    if let Some(issue) = completeness_issue(&view.market_stockpile_complete) {
        return TradeData::Unavailable(issue);
    }
    let Leaf::Measured(rows) = &view.market_stockpile else {
        return TradeData::Unavailable(leaf_issue(&view.market_stockpile));
    };
    let mut seen = BTreeSet::new();
    let mut cards = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Some(card) = measured_market_stockpile_card(row, index) else {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        };
        if !seen.insert((card.equipment_variant_id, card.equipment_type)) {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        }
        cards.push(card);
    }
    TradeData::Available(cards)
}

fn measured_market_stockpile_card(
    row: &MarketStockpileRow,
    expected_order: usize,
) -> Option<MarketStockpileCard> {
    let equipment_variant_id = *measured(&row.equipment_variant_id)?;
    let equipment_type = *measured(&row.equipment_type)?;
    let amount_raw = *measured(&row.amount_raw)?;
    let amount = *measured(&row.amount)?;
    let visible_order = *measured(&row.ordinal)?;
    if equipment_variant_id == 0
        || equipment_type != 70
        || amount_raw < 0
        || !amount.is_finite()
        || amount < 0.0
        || visible_order != expected_order as i64
    {
        return None;
    }
    Some(MarketStockpileCard {
        equipment_variant_id,
        equipment_type,
        amount_raw,
        amount,
        visible_order,
    })
}

fn admit_equipment_purchase_contracts(
    view: &TradeView,
) -> TradeData<Vec<EquipmentPurchaseContractCard>> {
    let Leaf::Measured(rows) = &view.equipment_purchase_contracts else {
        return TradeData::Unavailable(leaf_issue(&view.equipment_purchase_contracts));
    };
    let mut ids = BTreeSet::new();
    let mut previous_order = -1i64;
    let mut cards = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(card) = measured_equipment_purchase_contract(row) else {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        };
        if card.visible_order <= previous_order || !ids.insert(card.contract_id.clone()) {
            return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
        }
        previous_order = card.visible_order;
        cards.push(card);
    }
    TradeData::Available(cards)
}

fn measured_equipment_purchase_contract(
    row: &TradeContractRow,
) -> Option<EquipmentPurchaseContractCard> {
    if !matches!(
        &row.equipment_purchase_relation_complete,
        Leaf::Measured(true)
    ) {
        return None;
    }
    let contract_id = measured(&row.contract_id)?.clone();
    let visible_order = *measured(&row.visible_order)?;
    let counterpart = measured(&row.counterpart)?.clone();
    let direction = measured(&row.direction)?.clone();
    let agreement_id = measured(&row.relation_agreement_id)?.clone();
    let seller = measured(&row.seller)?.clone();
    let seller_id = *measured(&row.seller_tag_db_id)?;
    let buyer = measured(&row.buyer)?.clone();
    let buyer_id = *measured(&row.buyer_tag_db_id)?;
    let first_id = *measured(&row.relation_first_tag_db_id)?;
    let second_id = *measured(&row.relation_second_tag_db_id)?;
    let start_date_raw = *measured(&row.relation_start_date_raw)?;
    let start_date = measured(&row.relation_start_date)?.clone();
    if contract_id.is_empty()
        || visible_order < 0
        || seller_id == 0
        || buyer_id == 0
        || seller_id == buyer_id
        || !((first_id == seller_id && second_id == buyer_id)
            || (first_id == buyer_id && second_id == seller_id))
        || agreement_id
            != format!("equipment_purchase:{first_id}:{second_id}:{start_date_raw}")
        || !((direction == "outgoing" && counterpart == buyer)
            || (direction == "incoming" && counterpart == seller))
        || start_date_raw <= 0
        || start_date.is_empty()
        || !matches!(
            &row.relation_source,
            Leaf::Measured(source) if source == "D43_MTLL_plus_D53_CEquipmentPurchaseRelation"
        )
    {
        return None;
    }
    let end_date = agreement_end_date(
        &row.relation_end_date_raw,
        &row.relation_end_date,
        start_date_raw,
    )?;
    Some(EquipmentPurchaseContractCard {
        contract_id,
        visible_order,
        counterpart,
        direction,
        agreement_id,
        seller,
        buyer,
        start_date_raw,
        start_date,
        end_date,
    })
}

fn admit_production_license_agreements(
    view: &TradeView,
) -> TradeData<Vec<ProductionLicenseAgreementCard>> {
    if let Some(issue) = completeness_issue(&view.production_license_agreements_complete) {
        return TradeData::Unavailable(issue);
    }
    let Leaf::Measured(rows) = &view.production_license_agreements else {
        return TradeData::Unavailable(leaf_issue(&view.production_license_agreements));
    };
    let mut agreement_ids = BTreeSet::new();
    let mut variant_orders = BTreeSet::new();
    let mut previous_order = -1i64;
    let mut cards = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(card) = measured_production_license_agreement(row) else {
            return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
        };
        if card.visible_order <= previous_order || !agreement_ids.insert(card.agreement_id.clone()) {
            return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
        }
        previous_order = card.visible_order;
        for variant in &card.variants {
            if !variant_orders.insert(variant.visible_order) {
                return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
            }
        }
        cards.push(card);
    }
    if variant_orders
        .iter()
        .copied()
        .ne(0..variant_orders.len() as i64)
    {
        return TradeData::Unavailable(TradeDataIssue::InvalidExactOrder);
    }
    TradeData::Available(cards)
}

fn measured_production_license_agreement(
    row: &ProductionLicenseAgreementRow,
) -> Option<ProductionLicenseAgreementCard> {
    let agreement_id = measured(&row.agreement_id)?.clone();
    let visible_order = *measured(&row.visible_order)?;
    let owner = measured(&row.owner)?.clone();
    let owner_id = *measured(&row.owner_tag_db_id)?;
    let partner = measured(&row.partner)?.clone();
    let partner_id = *measured(&row.partner_tag_db_id)?;
    let first_id = *measured(&row.relation_first_tag_db_id)?;
    let second_id = *measured(&row.relation_second_tag_db_id)?;
    let start_date_raw = *measured(&row.start_date_raw)?;
    let start_date = measured(&row.start_date)?.clone();
    let source = measured(&row.source)?;
    let Leaf::Measured(variants) = &row.variants else {
        return None;
    };
    if agreement_id != format!("licence:{first_id}:{second_id}:{start_date_raw}")
        || owner_id == 0
        || partner_id == 0
        || owner_id == partner_id
        || !((first_id == owner_id && second_id == partner_id)
            || (first_id == partner_id && second_id == owner_id))
        || owner.is_empty()
        || partner.is_empty()
        || start_date_raw <= 0
        || start_date.is_empty()
        || source != "PLIC_v2_plus_D53_CLicenceProductionAccessRelation"
        || variants.is_empty()
    {
        return None;
    }
    let mut variant_cards = Vec::with_capacity(variants.len());
    for variant in variants {
        variant_cards.push(measured_production_license_variant(variant, partner_id)?);
    }
    let end_date = agreement_end_date(&row.end_date_raw, &row.end_date, start_date_raw)?;
    Some(ProductionLicenseAgreementCard {
        agreement_id,
        visible_order,
        owner,
        partner,
        start_date_raw,
        start_date,
        end_date,
        variants: variant_cards,
    })
}

fn agreement_end_date(
    raw_leaf: &Leaf<i64>,
    date_leaf: &Leaf<String>,
    start_date_raw: i64,
) -> Option<AgreementEndDate> {
    match (raw_leaf, date_leaf) {
        (Leaf::Measured(raw), Leaf::Measured(date))
            if *raw >= start_date_raw && !date.is_empty() =>
        {
            Some(AgreementEndDate::Measured {
                raw: *raw,
                date: date.clone(),
            })
        }
        (Leaf::Absent, Leaf::Absent) => Some(AgreementEndDate::Absent),
        (Leaf::Unobtained, Leaf::Unobtained) => Some(AgreementEndDate::Unavailable),
        _ => None,
    }
}

fn measured_production_license_variant(
    row: &ProductionLicenseVariantRow,
    partner_id: u32,
) -> Option<ProductionLicenseVariantCard> {
    let variant_id = *measured(&row.variant_id)?;
    let visible_order = *measured(&row.visible_order)?;
    let creator_tag_db_id = *measured(&row.creator_tag_db_id)?;
    let origin_tag_db_id = *measured(&row.origin_tag_db_id)?;
    let source_tag_db_id = *measured(&row.source_tag_db_id)?;
    let expected_source = if origin_tag_db_id == 0 {
        creator_tag_db_id
    } else {
        origin_tag_db_id
    };
    if variant_id == 0
        || variant_id >= (1 << 24)
        || visible_order < 0
        || creator_tag_db_id == 0
        || source_tag_db_id != expected_source
        || source_tag_db_id != partner_id
    {
        return None;
    }
    Some(ProductionLicenseVariantCard {
        variant_id,
        visible_order,
        creator_tag_db_id,
        origin_tag_db_id,
        source_tag_db_id,
    })
}

fn admit_factories(view: &TradeView) -> TradeData<TradeFactoryHeader> {
    match (
        &view.factories_total,
        &view.factories_used,
        &view.factories_available,
    ) {
        (Leaf::Measured(total), Leaf::Measured(used), Leaf::Measured(available)) => {
            if [*total, *used, *available]
                .into_iter()
                .any(|value| i32::try_from(value).is_err())
            {
                return TradeData::Unavailable(TradeDataIssue::IncompleteRow);
            }
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
        if !requested.is_finite() || !delivered.is_finite() {
            return TradeData::Unavailable(TradeDataIssue::ArithmeticOverflow);
        }
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
    if !localisation.decode_lossy_files.is_empty() {
        return Err(crate::GameUiError::parse(format!(
            "XUI15 trade localisation decode destroyed bytes in {:?}",
            localisation.decode_lossy_files
        )));
    }
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
            let deal_row_count = if let TradeData::Available(rows) = &presentation.deals {
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
                rows.len()
            } else {
                children.push(unavailable_text(
                    "xui15_deals_unavailable",
                    20,
                    552,
                    "CLASSIC RESOURCE DEALS UNAVAILABLE",
                ));
                1
            };

            let license_summary_y = checked_license_row_y(540, deal_row_count.max(1))?;
            let mut summary = load_named_element(
                &mut layout_cache,
                resolver,
                "interface/countrytradeview.gui",
                "country_trade_entry",
            )?;
            set_position_upper_left(&mut summary, 9, license_summary_y);
            rename_descendant_exact(&mut summary, "country", "count")?;
            prefix_element_names(&mut summary, "xui15_license_");
            options
                .hidden_elements
                .insert("xui15_license_country_flag".to_string());
            options
                .hidden_elements
                .insert("xui15_license_country_flag_frame".to_string());
            children.push(summary);

            match &presentation.production_license_agreements {
                TradeData::Available(rows) => {
                    for (index, _) in rows.iter().enumerate() {
                        let mut element = load_named_element(
                            &mut layout_cache,
                            resolver,
                            "interface/countrytradeview.gui",
                            "country_trade_entry",
                        )?;
                        let ordinal = index.checked_add(1).ok_or_else(|| {
                            crate::GameUiError::parse(
                                "XUI15 production-licence row ordinal overflow",
                            )
                        })?;
                        set_position_upper_left(
                            &mut element,
                            9,
                            checked_license_row_y(license_summary_y, ordinal)?,
                        );
                        for (authored_name, semantic_name) in [
                            ("country", "partner"),
                            ("available_export", "start_date"),
                            ("exported", "end_date"),
                            ("convoys", "variant_count"),
                        ] {
                            rename_descendant_exact(
                                &mut element,
                                authored_name,
                                semantic_name,
                            )?;
                        }
                        prefix_element_names(
                            &mut element,
                            &format!("xui15_license_{index}_"),
                        );
                        options
                            .hidden_elements
                            .insert(format!("xui15_license_{index}_country_flag"));
                        options
                            .hidden_elements
                            .insert(format!("xui15_license_{index}_country_flag_frame"));
                        children.push(element);
                    }
                }
                TradeData::Unavailable(_) => children.push(unavailable_text(
                    "xui15_licenses_unavailable",
                    20,
                    license_summary_y + 57,
                    "PRODUCTION LICENCES UNAVAILABLE",
                )),
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
    match &presentation.equipment_purchase_contracts {
        TradeData::Available(rows) => {
            set_text(
                options,
                "xui15_equipment_contract_count",
                rows.len().to_string(),
            );
            for (index, row) in rows.iter().enumerate() {
                let prefix = format!("xui15_equipment_contract_{index}_");
                set_text(options, &format!("{prefix}counterpart"), row.counterpart.clone());
                set_text(options, &format!("{prefix}direction"), row.direction.clone());
                set_text(options, &format!("{prefix}start_date"), row.start_date.clone());
                set_text(
                    options,
                    &format!("{prefix}end_date"),
                    row.end_date.display_text(),
                );
            }
        }
        TradeData::Unavailable(_) => {
            set_text(options, "xui15_equipment_contract_count", "--");
        }
    }
    match &presentation.production_license_agreements {
        TradeData::Available(rows) => {
            set_text(options, "xui15_license_count", rows.len().to_string());
            for (index, row) in rows.iter().enumerate() {
                let prefix = format!("xui15_license_{index}_");
                set_text(options, &format!("{prefix}partner"), row.partner.clone());
                set_text(options, &format!("{prefix}start_date"), row.start_date.clone());
                set_text(
                    options,
                    &format!("{prefix}end_date"),
                    row.end_date.display_text(),
                );
                set_text(
                    options,
                    &format!("{prefix}variant_count"),
                    row.variants.len().to_string(),
                );
            }
        }
        TradeData::Unavailable(_) => {
            set_text(options, "xui15_license_count", "--");
        }
    }
}

    pub(crate) fn format_needed(row: &TradeResourceCard) -> String {
        if row.needed_type == "current_max" {
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

fn rename_descendant_exact(
    element: &mut GuiElement,
    authored_name: &str,
    semantic_name: &str,
) -> Result<()> {
    let mut matches = 0usize;
    rename_descendants(element, authored_name, semantic_name, &mut matches);
    if matches != 1 {
        return Err(crate::GameUiError::missing(format!(
            "interface/countrytradeview.gui: country_trade_entry expected one {authored_name} child, found {matches}"
        )));
    }
    Ok(())
}

fn checked_license_row_y(base: i32, ordinal: usize) -> Result<i32> {
    let ordinal = i32::try_from(ordinal).map_err(|_| {
        crate::GameUiError::parse("XUI15 production-licence row count exceeds i32 geometry")
    })?;
    let offset = ordinal.checked_mul(45).ok_or_else(|| {
        crate::GameUiError::parse("XUI15 production-licence row offset overflow")
    })?;
    base.checked_add(offset).ok_or_else(|| {
        crate::GameUiError::parse("XUI15 production-licence row position overflow")
    })
}

fn rename_descendants(
    element: &mut GuiElement,
    authored_name: &str,
    semantic_name: &str,
    matches: &mut usize,
) {
    if element.name == authored_name {
        element.name = semantic_name.to_string();
        *matches += 1;
    }
    for child in &mut element.children {
        rename_descendants(child, authored_name, semantic_name, matches);
    }
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
