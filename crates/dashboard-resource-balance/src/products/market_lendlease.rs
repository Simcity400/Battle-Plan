//! Selected-hour market and lend-lease facts shared by both widget consumers.
use dashboard_base::{
    Source,
    catalogue::catalogue,
    products::{LoadContext, Product, check_reply_budget, equipment},
};
use dashboard_text::metadata::{check_cancel, link_slot, validate_campaign};
use replay::container::SeekResult;
use replay_data::rows::{MarketPool, MarketPriceLevel, MarketSubsidy, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct MarketLendLease;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketLendLeaseParams {
    pub branch_id: String,
    pub hour: i32,
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketItem {
    pub variant: String,
    pub name: Option<String>,
    pub amount_raw: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketLevel {
    pub variant: String,
    pub name: Option<String>,
    pub level: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketGrant {
    pub cic_raw: String,
    pub archetype: Option<String>,
    pub targets: Vec<String>,
    pub trigger: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketOffer {
    pub country: u32,
    pub tag: Option<String>,
    pub inventory: Vec<MarketItem>,
    pub prices: Vec<MarketLevel>,
    pub subsidies: Vec<MarketGrant>,
    pub subsidy_total_raw: String,
    pub automation: [bool; 3],
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketTerms {
    pub id: String,
    pub seller: Option<String>,
    pub buyer: Option<String>,
    pub request: bool,
    pub speed: i32,
    pub equipment: Vec<MarketItem>,
    pub prices: Vec<MarketItem>,
    pub levels: Vec<MarketLevel>,
    pub subsidies: Vec<MarketGrant>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaseTerms {
    pub id: String,
    pub giver: Option<String>,
    pub receiver: Option<String>,
    pub revision: String,
    pub active_since: i32,
    pub last_delivery: i32,
    pub monthly: Vec<MarketItem>,
    pub percentage: Vec<MarketItem>,
    pub once: Vec<MarketItem>,
    pub delivered: Vec<MarketItem>,
    pub sunk: Vec<MarketItem>,
    pub fuel_daily_raw: String,
    pub fuel_percentage_raw: String,
    pub fuel_sent_raw: String,
    pub fuel_sunk_raw: String,
    pub last_fuel_delivered_raw: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketDelivery {
    pub id: String,
    pub occurrence: String,
    pub agreement: String,
    pub hour: i32,
    pub giver: Option<String>,
    pub receiver: Option<String>,
    pub kind: u8,
    pub item: MarketItem,
    pub lost_raw: String,
    pub total_raw: String,
    pub lost_total_raw: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketLendLeaseReply {
    pub branch_id: String,
    pub hour: i32,
    pub offers: Option<Vec<MarketOffer>>,
    pub contracts: Option<Vec<MarketTerms>>,
    pub agreements: Option<Vec<LeaseTerms>>,
    pub deliveries: Option<Vec<MarketDelivery>>,
    pub warnings: Vec<String>,
}
#[derive(Default)]
struct State {
    frame: Option<Arc<DecodedFrame>>,
}
impl Product for MarketLendLease {
    const KEY: &'static str = "market-lendlease";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = MarketLendLeaseParams;
    type Reply = MarketLendLeaseReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        for tag in &params.tags {
            dashboard_base::products::validate_tag(tag)?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            MarketLendLeaseParams::decl(),
            MarketLendLeaseReply::decl(),
            MarketItem::decl(),
            MarketLevel::decl(),
            MarketGrant::decl(),
            MarketOffer::decl(),
            MarketTerms::decl(),
            LeaseTerms::decl(),
            MarketDelivery::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(io::Error::other)?;
        let state = source.state::<State>();
        let frame = {
            let mut state = state
                .lock()
                .map_err(|_| io::Error::other("Market frame cache poisoned"))?;
            if let Some(frame) = &state.frame
                && frame.branch == branch
                && frame.hour == params.hour
            {
                Arc::clone(frame)
            } else {
                let mut selected = equipment::VariantCatalogue::TABLES.to_vec();
                selected.extend([
                    TableId::Campaign,
                    TableId::CountryMarket,
                    TableId::MarketContractTerms,
                    TableId::LendLeaseAgreements,
                    TableId::MarketEvents,
                ]);
                let frame = match source.replay().seek_selected_with(
                    branch,
                    params.hour,
                    Some(&selected),
                    || check_cancel(cancelled),
                )? {
                    SeekResult::Frame(frame) => Arc::new(*frame),
                    SeekResult::Gap { reason, .. } => return Err(io::Error::other(reason)),
                };
                validate_campaign(&frame, &source.replay())?;
                state.frame = Some(Arc::clone(&frame));
                frame
            }
        };
        let catalogue = catalogue(source, context.content()?)?;
        let mut reply = present(&frame, params, Some(&catalogue))?;
        reply.warnings.extend(catalogue.refusal_lines());
        check_cancel(cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
fn present(
    frame: &DecodedFrame,
    params: &MarketLendLeaseParams,
    catalogue: Option<&dashboard_base::catalogue::Catalogue>,
) -> io::Result<MarketLendLeaseReply> {
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(io::Error::other)?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(io::Error::other)?;
    let tags = equipment::Tags::new(
        countries,
        links,
        frame.complete::<tables::CountryAppearance>().ok(),
    );
    let mut warnings = Vec::new();
    let countries_by_slot: BTreeMap<_, _> = countries
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect();
    let wanted_tags: BTreeSet<_> = params.tags.iter().map(|tag| tag.as_bytes()).collect();
    let tag = |raw| {
        link_slot(links, raw)
            .and_then(|slot| countries_by_slot.get(&slot))
            .and_then(|country| String::from_utf8(country.tag.clone()).ok())
    };
    let chosen: BTreeSet<_> = countries
        .iter()
        .filter(|country| wanted_tags.is_empty() || wanted_tags.contains(country.tag.as_slice()))
        .map(|country| country.canonical_slot)
        .collect();
    let selected = |owner| {
        params.tags.is_empty() || link_slot(links, owner).is_some_and(|slot| chosen.contains(&slot))
    };
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    let mut needed = BTreeSet::new();
    let mut collect = |pool: &MarketPool| needed.extend(pool.items.iter().map(|row| row.variant));
    if let Ok(rows) = frame.complete::<tables::CountryMarket>() {
        for row in rows.iter().filter(|row| selected(row.country)) {
            collect(&row.inventory);
        }
    }
    if let Ok(rows) = frame.complete::<tables::MarketContractTerms>() {
        for row in rows
            .iter()
            .filter(|row| selected(row.country) || selected(row.buyer))
        {
            collect(&row.equipment);
            collect(&row.prices);
        }
    }
    if let Ok(rows) = frame.complete::<tables::LendLeaseAgreements>() {
        for row in rows
            .iter()
            .filter(|row| selected(row.country) || selected(row.receiver))
        {
            for pool in [
                &row.monthly,
                &row.percentage,
                &row.once,
                &row.delivered,
                &row.sunk,
            ] {
                collect(pool);
            }
        }
    }
    if let Ok(rows) = frame.complete::<tables::CountryMarket>() {
        for row in rows.iter().filter(|row| selected(row.country)) {
            needed.extend(row.prices.iter().map(|row| row.variant));
        }
    }
    if let Ok(rows) = frame.complete::<tables::MarketContractTerms>() {
        for row in rows
            .iter()
            .filter(|row| selected(row.country) || selected(row.buyer))
        {
            needed.extend(row.price_levels.iter().map(|row| row.variant));
        }
    }
    if let Ok(rows) = frame.complete::<tables::MarketEvents>() {
        needed.extend(
            rows.iter()
                .filter(|row| {
                    matches!(row.kind, 3 | 4 | 20)
                        && (selected(row.country) || selected(row.receiver))
                })
                .map(|row| row.variant),
        );
    }
    needed.remove(&0);
    let mut names = BTreeMap::new();
    if let (Ok(definitions), Some(lexical)) =
        (frame.complete::<tables::VariantDefinitions>(), lexical)
    {
        for definition in definitions
            .iter()
            .filter(|definition| needed.contains(&definition.id))
        {
            let raw = equipment::raw_variant(definition, frame.hour, &tags, lexical);
            match equipment::variant_name(
                &raw,
                catalogue.map(|catalogue| catalogue as &dyn dashboard_text::variant_names::Texts),
            ) {
                Ok(name) => {
                    names.insert(definition.id, name);
                }
                Err(reason) => warnings.push(reason),
            }
        }
    } else {
        warnings.push("Equipment names: recorded naming inputs unavailable".into());
    }
    let missing = needed
        .iter()
        .filter(|id| names.get(*id).is_none_or(Option::is_none))
        .count();
    if missing > 0 {
        warnings.push(format!(
            "Equipment names unavailable for {missing} demanded variants"
        ));
    }
    let item = |variant, amount_raw: i64| MarketItem {
        variant: format!("{variant:016x}"),
        name: names.get(&variant).cloned().flatten(),
        amount_raw: amount_raw.to_string(),
    };
    let pool = |pool: &MarketPool| {
        pool.items
            .iter()
            .map(|row| item(row.variant, row.amount_raw))
            .collect()
    };
    let levels = |rows: &[MarketPriceLevel]| {
        rows.iter()
            .map(|row| MarketLevel {
                variant: format!("{:016x}", row.variant),
                name: names.get(&row.variant).cloned().flatten(),
                level: row.level,
            })
            .collect()
    };
    let grants = |rows: &[MarketSubsidy]| {
        rows.iter()
            .map(|row| MarketGrant {
                cic_raw: row.cic_raw.to_string(),
                archetype: lexical
                    .and_then(|rows| {
                        rows.binary_search_by_key(&row.archetype, |token| token.token)
                            .ok()
                            .map(|index| &rows[index])
                    })
                    .and_then(|row| String::from_utf8(row.literal.clone()).ok())
                    .and_then(|token| {
                        catalogue
                            .and_then(|catalogue| equipment::localise_for(catalogue, None, &token))
                    }),
                targets: row
                    .targets
                    .iter()
                    .map(|raw| {
                        tag(*raw).unwrap_or_else(|| format!("Country {raw}: name unavailable"))
                    })
                    .collect(),
                trigger: (row.kind == 1)
                    .then(|| String::from_utf8_lossy(&row.trigger).into_owned()),
            })
            .collect()
    };
    let offers: Option<Vec<MarketOffer>> = match frame.complete::<tables::CountryMarket>() {
        Ok(rows) => Some(
            rows.iter()
                .filter(|row| selected(row.country))
                .map(|row| MarketOffer {
                    country: row.country,
                    tag: tag(row.country),
                    inventory: pool(&row.inventory),
                    prices: levels(&row.prices),
                    subsidies: grants(&row.subsidies),
                    subsidy_total_raw: row.subsidy_total_raw.to_string(),
                    automation: [
                        row.auto_accept_market_access,
                        row.auto_send_market_access,
                        row.auto_accept_purchase,
                    ],
                })
                .collect(),
        ),
        Err(reason) => {
            warnings.push(format!("Market offers: {reason}"));
            None
        }
    };
    let contracts: Option<Vec<MarketTerms>> = match frame.complete::<tables::MarketContractTerms>()
    {
        Ok(rows) => Some(
            rows.iter()
                .filter(|row| selected(row.country) || selected(row.buyer))
                .map(|row| MarketTerms {
                    id: format!("{:016x}", row.id),
                    seller: tag(row.country),
                    buyer: tag(row.buyer),
                    request: row.kind == 0,
                    speed: row.speed,
                    equipment: pool(&row.equipment),
                    prices: pool(&row.prices),
                    levels: levels(&row.price_levels),
                    subsidies: grants(&row.subsidies),
                })
                .collect(),
        ),
        Err(reason) => {
            warnings.push(format!("Market terms: {reason}"));
            None
        }
    };
    let agreements: Option<Vec<LeaseTerms>> = match frame.complete::<tables::LendLeaseAgreements>()
    {
        Ok(rows) => Some(
            rows.iter()
                .filter(|row| selected(row.country) || selected(row.receiver))
                .map(|row| LeaseTerms {
                    id: format!("{:016x}", row.id),
                    giver: tag(row.country),
                    receiver: tag(row.receiver),
                    revision: row.revision.to_string(),
                    active_since: row.active_since,
                    last_delivery: row.last_delivery,
                    monthly: pool(&row.monthly),
                    percentage: pool(&row.percentage),
                    once: pool(&row.once),
                    delivered: pool(&row.delivered),
                    sunk: pool(&row.sunk),
                    fuel_daily_raw: row.fuel_daily_raw.to_string(),
                    fuel_percentage_raw: row.fuel_percentage_raw.to_string(),
                    fuel_sent_raw: row.fuel_sent_raw.to_string(),
                    fuel_sunk_raw: row.fuel_sunk_raw.to_string(),
                    last_fuel_delivered_raw: row.last_fuel_delivered_raw.to_string(),
                })
                .collect(),
        ),
        Err(reason) => {
            warnings.push(format!("Lend-lease terms: {reason}"));
            None
        }
    };
    let deliveries: Option<Vec<MarketDelivery>> = match frame.complete::<tables::MarketEvents>() {
        Ok(rows) => {
            let mut missing = 0_usize;
            let deliveries = rows
                .iter()
                .filter(|row| {
                    matches!(row.kind, 3 | 4 | 5 | 6 | 20 | 21 | 34)
                        && (selected(row.country) || selected(row.receiver))
                })
                .map(|row| MarketDelivery {
                    id: row.id.to_string(),
                    occurrence: row.occurrence.to_string(),
                    agreement: format!("{:016x}", row.agreement),
                    hour: row.hour,
                    giver: tag(row.country),
                    receiver: tag(row.receiver),
                    kind: row.kind,
                    item: if let Some(reason) = row.delivery_unavailable_reason() {
                        missing += 1;
                        MarketItem {
                            variant: "0".into(),
                            name: Some(format!(
                                "{reason} ({} {:016x})",
                                if row.mode == 20 {
                                    "contract"
                                } else {
                                    "agreement"
                                },
                                row.agreement
                            )),
                            amount_raw: "0".into(),
                        }
                    } else {
                        item(row.variant, row.amount_raw)
                    },
                    lost_raw: row.auxiliary_raw.to_string(),
                    total_raw: row.total_raw.to_string(),
                    lost_total_raw: row.lost_total_raw.to_string(),
                })
                .collect();
            if missing > 0 {
                warnings.push(format!("Deliveries: {missing} amount streams unavailable; explicit holder refusals retained"));
            }
            Some(deliveries)
        }
        Err(reason) => {
            warnings.push(format!("Deliveries: {reason}"));
            None
        }
    };
    let missing_countries = offers
        .iter()
        .flatten()
        .filter(|row| row.tag.is_none())
        .count()
        + contracts
            .iter()
            .flatten()
            .filter(|row| row.seller.is_none() || row.buyer.is_none())
            .count()
        + agreements
            .iter()
            .flatten()
            .filter(|row| row.giver.is_none() || row.receiver.is_none())
            .count();
    if missing_countries > 0 {
        warnings.push(format!(
            "Country identities unavailable in {missing_countries} market or lend-lease rows"
        ));
    }
    warnings.sort();
    warnings.dedup();
    Ok(MarketLendLeaseReply {
        branch_id: params.branch_id.clone(),
        hour: frame.hour,
        offers,
        contracts,
        agreements,
        deliveries,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::{Metrics, SharedView, TableView};
    fn frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                SharedView::from(TableView::Complete(TableRows::empty(TableId::ALL[index])))
            }),
        };
        for rows in [
            TableRows::Countries(vec![
                CountryRow {
                    id: 5000,
                    canonical_slot: 2,
                    tag: b"AA".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 4999,
                    canonical_slot: 1,
                    tag: b"BB".to_vec(),
                    ..Default::default()
                },
            ]),
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 4999,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 5000,
                    canonical_slot: 2,
                },
            ]),
            TableRows::CountryMarket(vec![CountryMarketRow {
                country: 5000,
                inventory: MarketPool {
                    items: vec![MarketEquipment {
                        variant: u64::MAX,
                        amount_raw: 9_007_199_254_740_993,
                    }],
                    ..Default::default()
                }
                .into(),
                subsidy_total_raw: 17,
                ..Default::default()
            }]),
            TableRows::LendLeaseAgreements(vec![LendLeaseAgreementRow {
                country: 5000,
                id: 19,
                receiver: 4999,
                fuel_daily_raw: 32769,
                ..Default::default()
            }]),
            TableRows::MarketEvents(vec![
                MarketEventRow {
                    id: 21,
                    country: 5000,
                    receiver: 4999,
                    agreement: 123,
                    kind: 3,
                    amount_raw: 77,
                    ..Default::default()
                },
                MarketEventRow {
                    id: 22,
                    country: 5000,
                    receiver: 4999,
                    agreement: 123,
                    kind: 6,
                    ..Default::default()
                },
            ]),
        ] {
            let table = rows.table();
            frame.tables[table.index()] = TableView::Complete(rows).into();
        }
        frame
    }
    #[test]
    fn complete_delivery_ledger_counts_explicit_amount_refusals_and_keeps_other_events() {
        let mut frame = frame();
        frame.tables[TableId::MarketEvents.index()] =
            TableView::Complete(TableRows::MarketEvents(vec![
                MarketEventRow {
                    id: 1,
                    country: 5000,
                    receiver: 4999,
                    agreement: 17,
                    kind: 5,
                    amount_raw: 123,
                    ..Default::default()
                },
                MarketEventRow {
                    id: 2,
                    country: 5000,
                    receiver: 4999,
                    agreement: 18,
                    kind: 34,
                    mode: 20,
                    ordinal: 1,
                    ..Default::default()
                },
            ]))
            .into();
        assert!(frame.complete::<tables::MarketEvents>().is_ok());
        let reply = present(
            &frame,
            &MarketLendLeaseParams {
                branch_id: "1".into(),
                hour: 7,
                tags: vec!["AA".into()],
            },
            None,
        )
        .unwrap();
        assert!(
            reply
                .warnings
                .iter()
                .any(|warning| warning.contains("1 amount streams unavailable"))
        );
        let rows = reply.deliveries.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].item.amount_raw, "123");
        assert_eq!(rows[1].agreement, "0000000000000012");
        assert_eq!(rows[1].kind, 34);
        assert!(rows[1].item.name.as_ref().unwrap().contains("unreadable"));
    }
    #[test]
    fn market_lendlease_preserves_wide_values_missing_names_and_terminal_events() {
        let reply = present(
            &frame(),
            &MarketLendLeaseParams {
                branch_id: "1".into(),
                hour: 7,
                tags: vec!["AA".into()],
            },
            None,
        )
        .unwrap();
        assert_eq!(
            reply.offers.unwrap()[0].inventory[0].amount_raw,
            "9007199254740993"
        );
        assert_eq!(reply.agreements.unwrap()[0].fuel_daily_raw, "32769");
        let events = reply.deliveries.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].agreement, "000000000000007b");
        assert_eq!(events[1].kind, 6);
    }
    #[test]
    fn market_lendlease_unknown_tables_never_become_empty_and_buyer_selection_keeps_terms() {
        let mut frame = frame();
        frame.tables[TableId::CountryMarket.index()] = TableView::Unavailable {
            reason: "lost market writer".into(),
        }
        .into();
        frame.tables[TableId::MarketContractTerms.index()] =
            TableView::Complete(TableRows::MarketContractTerms(vec![
                MarketContractTermsRow {
                    country: 4999,
                    buyer: 5000,
                    id: u64::MAX,
                    kind: 1,
                    ..Default::default()
                },
            ]))
            .into();
        let reply = present(
            &frame,
            &MarketLendLeaseParams {
                branch_id: "1".into(),
                hour: 7,
                tags: vec!["AA".into()],
            },
            None,
        )
        .unwrap();
        assert!(reply.offers.is_none());
        assert_eq!(reply.contracts.unwrap()[0].seller.as_deref(), Some("BB"));
        assert!(
            reply
                .warnings
                .iter()
                .any(|warning| warning.contains("lost market writer"))
        );
    }
}
