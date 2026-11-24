
pub mod ledger;

use dashboard_base::products::Country;
use dashboard_base::{Source, products::{LoadContext, check_reply_budget, at_hour::{At, AtHour, Owner, Read}}, queries::country_ids::Names};
use dashboard_world::products::equipment_names::{EquipmentIcon, VariantName, self as names, Named};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::identity::{Holder, Holders};
use dashboard_text::{
    metadata::link_slot,
    variant_names::{Texts, localise_for},
};
use replay_data::rows::{CountryLinkRow, LendLeaseAgreementRow, LexicalTokenRow, MarketPool, MarketPriceLevel, MarketSubsidy, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MarketSection {
    Offers,
    Contracts,
    Agreements,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketLendLeaseParams {
    pub branch_id: String,
    #[doc = " Rows these countries take part in (as seller or buyer, giver or receiver); every row when"]
    #[doc = " empty."]
    pub countries: Vec<Country>,
    #[doc = " The one section read; every section when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub only: Option<MarketSection>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketItem {
    pub variant: String,
    pub name: Option<String>,
    pub amount_raw: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketLevel {
    pub variant: String,
    pub name: Option<String>,
    #[doc = " 0 low, 1 normal, 2 high."]
    pub level: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketGrant {
    pub cic_raw: String,
    pub archetype: Option<String>,
    pub targets: Vec<String>,
    pub trigger: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketOffer {
    #[doc = " The seller's raw country id."]
    pub country: u32,
    pub tag: Option<String>,
    pub inventory: Vec<MarketItem>,
    pub prices: Vec<MarketLevel>,
    pub subsidies: Vec<MarketGrant>,
    pub subsidy_total_raw: String,
    pub auto_send_market_access: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketTerms {
    pub key: String,
    #[doc = " The handle, hexadecimal."]
    pub id: String,
    pub seller: Option<String>,
    pub buyer: Option<String>,
    pub request: bool,
    pub speed: i32,
    pub equipment: Vec<MarketItem>,
    #[doc = " Agreed prices, CIC."]
    pub prices: Vec<MarketItem>,
    pub levels: Vec<MarketLevel>,
    pub subsidies: Vec<MarketGrant>,
    #[doc = " Whether a purchase contract stands for it: its progress is in the live cells."]
    pub purchase: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaseTerms {
    pub key: String,
    #[doc = " The handle, hexadecimal."]
    pub id: String,
    pub giver: Option<String>,
    pub receiver: Option<String>,
    pub revision: String,
    pub active_since: i32,
    pub last_delivery: i32,
    pub monthly: Vec<MarketItem>,
    #[doc = " Production shares, /1000 percent."]
    pub percentage: Vec<MarketItem>,
    pub once: Vec<MarketItem>,
    pub delivered: Vec<MarketItem>,
    pub sunk: Vec<MarketItem>,
    pub fuel_daily_raw: String,
    pub fuel_percentage_raw: String,
    pub last_fuel_delivered_raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketLendLeaseReply {
    pub branch_id: String,
    #[doc = " Null where the section was not asked or not recorded at the hour (`warnings` says which)."]
    pub offers: Option<Vec<MarketOffer>>,
    pub contracts: Option<Vec<MarketTerms>>,
    pub agreements: Option<Vec<LeaseTerms>>,
    #[doc = " Why purchase progress is not known at the hour, when contracts are shown and it is not."]
    pub purchases_missing: Option<String>,
    #[doc = " Rows whose country in the chosen side has no tag at the hour: a chosen country may be theirs."]
    pub unattributed: u32,
    pub variants: Vec<VariantName>,
    pub icons: Vec<EquipmentIcon>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MarketLendLeaseReply);

pub(crate) struct Parties<'a> {
    links: &'a [CountryLinkRow],
    names: Names,
    hour: i32,
}

impl<'a> Parties<'a> {
    pub(crate) fn new(links: &'a [CountryLinkRow], holders: &Holders, hour: i32) -> Self {
        Self { links, names: Names::new(holders), hour }
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        link_slot(self.links, raw)
    }

    pub(crate) fn tag(&self, raw: u32) -> Option<String> {
        self.names.slot_tag(self.slot(raw)?, self.hour).map(str::to_owned)
    }
}

pub(crate) fn pool_variants(pool: &MarketPool) -> impl Iterator<Item = u64> + '_ {
    pool.items.iter().map(|item| item.variant).filter(|&variant| variant != 0)
}

pub(crate) fn lease_variants(row: &LendLeaseAgreementRow) -> impl Iterator<Item = u64> + '_ {
    [&row.monthly, &row.percentage, &row.once, &row.delivered, &row.sunk].into_iter().flat_map(|pool| pool_variants(pool))
}

fn named_variant(named: &Named, variant: u64) -> Option<String> {
    named.variants.get(&variant).and_then(|held| held.name.clone())
}

pub(crate) fn pool(named: &Named, pool: &MarketPool) -> Vec<MarketItem> {
    pool.items.iter().map(|item| MarketItem { variant: item.variant.to_string(), name: named_variant(named, item.variant), amount_raw: item.amount_raw.to_string() }).collect()
}

fn levels<'r>(named: &Named, rows: impl IntoIterator<Item = &'r MarketPriceLevel>) -> Vec<MarketLevel> {
    rows.into_iter().map(|row| MarketLevel { variant: row.variant.to_string(), name: named_variant(named, row.variant), level: row.level }).collect()
}

fn grants<'r>(rows: impl IntoIterator<Item = &'r MarketSubsidy>, parties: &Parties<'_>, lexical: Option<&[LexicalTokenRow]>, texts: Option<&dyn Texts>) -> Vec<MarketGrant> {
    rows.into_iter()
        .map(|row| MarketGrant {
            cic_raw: row.cic_raw.to_string(),
            archetype: lexical
                .and_then(|lexical| lexical.binary_search_by_key(&row.archetype, |token| token.token).ok().map(|at| &lexical[at]))
                .and_then(|token| std::str::from_utf8(&token.literal).ok())
                .and_then(|key| texts.and_then(|texts| localise_for(texts, None, key))),
            targets: row.targets.iter().map(|&raw| parties.tag(raw).unwrap_or_else(|| format!("Country {raw}: name unavailable"))).collect(),
            trigger: (row.kind == 1).then(|| String::from_utf8_lossy(&row.trigger).into_owned()),
        })
        .collect()
}

pub(crate) fn lease_terms(row: &LendLeaseAgreementRow, tag: impl Fn(u32) -> Option<String>, named: &Named) -> LeaseTerms {
    LeaseTerms {
        key: lease_key(row).to_string(),
        id: format!("{:016x}", row.id),
        giver: tag(row.country),
        receiver: tag(row.receiver),
        revision: row.revision.to_string(),
        active_since: row.active_since,
        last_delivery: row.last_delivery,
        monthly: pool(named, &row.monthly),
        percentage: pool(named, &row.percentage),
        once: pool(named, &row.once),
        delivered: pool(named, &row.delivered),
        sunk: pool(named, &row.sunk),
        fuel_daily_raw: row.fuel_daily_raw.to_string(),
        fuel_percentage_raw: row.fuel_percentage_raw.to_string(),
        last_fuel_delivered_raw: row.last_fuel_delivered_raw.to_string(),
    }
}

pub(crate) fn lease_key(row: &LendLeaseAgreementRow) -> u128 {
    u128::from(row.country) << 64 | u128::from(row.id)
}

fn sections(params: &MarketLendLeaseParams) -> [bool; 3] {
    match params.only {
        None => [true; 3],
        Some(MarketSection::Offers) => [true, false, false],
        Some(MarketSection::Contracts) => [false, true, false],
        Some(MarketSection::Agreements) => [false, false, true],
    }
}

fn section<'f, R>(shown: bool, rows: io::Result<&'f [R]>, label: &str, warnings: &mut Vec<String>) -> Option<&'f [R]> {
    if !shown {
        return None;
    }
    rows.map_err(|error| warnings.push(format!("{label} are not recorded at this hour: {error}"))).ok()
}

fn chosen(holders: &Holders, tags: &[Country], hour: i32, warnings: &mut Vec<String>) -> Option<BTreeSet<u32>> {
    if tags.is_empty() {
        return None;
    }
    let mut slots = BTreeSet::new();
    for tag in tags {
        match holders.at(tag, hour).map(|run| run.holder) {
            Some(Holder::Found { slot, .. }) => {
                slots.insert(slot);
            }
            _ => warnings.push(holders.say(tag, hour, |name| format!("{name} is not held by any country at this hour"))),
        }
    }
    Some(slots)
}

pub struct MarketLendLease;

impl AtHour for MarketLendLease {
    const KEY: &'static str = "market-lendlease";
    const TITLE: &'static str = "Market and lend-lease";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MarketLendLeaseParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, only, "countries[]"];
    type Reply = MarketLendLeaseReply;
    type Context = names::Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().try_for_each(dashboard_base::products::validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            MarketLendLeaseParams::decl(),
            MarketLendLeaseReply::decl(),
            MarketSection::decl(),
            MarketItem::decl(),
            MarketLevel::decl(),
            MarketGrant::decl(),
            MarketOffer::decl(),
            MarketTerms::decl(),
            LeaseTerms::decl(),
        ]
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let [offers, contracts, agreements] = sections(params);
        let mut out = names::TABLES.to_vec();
        out.extend(offers.then_some(TableId::CountryMarket));
        out.extend(contracts.then_some(TableId::MarketContractTerms));
        out.extend(contracts.then_some(TableId::PurchaseContracts));
        out.extend(agreements.then_some(TableId::LendLeaseAgreements));
        out
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let [offers, contracts, agreements] = sections(params);
        match step {
            0 => {
                let mut reads: Vec<Read> = names::WHOLE.iter().map(|&table| Read::all(table)).collect();
                reads.extend(offers.then(|| Read::all(TableId::CountryMarket)));
                reads.extend(contracts.then(|| Read::all(TableId::MarketContractTerms)));
                reads.extend(agreements.then(|| Read::all(TableId::LendLeaseAgreements)));
                reads
            }
            1 => {
                let mut variants: BTreeSet<u64> = BTreeSet::new();
                let mut purchases: BTreeSet<u64> = BTreeSet::new();
                if let Ok(rows) = held.complete::<tables::CountryMarket>() {
                    for row in rows {
                        variants.extend(pool_variants(&row.inventory));
                        variants.extend(row.prices.iter().map(|level| level.variant));
                    }
                }
                if let Ok(rows) = held.complete::<tables::MarketContractTerms>() {
                    for row in rows {
                        variants.extend(pool_variants(&row.equipment).chain(pool_variants(&row.prices)));
                        variants.extend(row.price_levels.iter().map(|level| level.variant));
                        if row.kind == 1 {
                            purchases.insert(row.id);
                        }
                    }
                }
                if let Ok(rows) = held.complete::<tables::LendLeaseAgreements>() {
                    for row in rows {
                        variants.extend(lease_variants(row));
                    }
                }
                let mut reads = vec![
                    Read::these(TableId::VariantDefinitions, variants.into_iter().map(u128::from)),
                    names::capital_state_read(held),
                ];
                if contracts {
                    reads.push(Read::these(TableId::PurchaseContracts, purchases.into_iter().map(u128::from)));
                }
                reads
            }
            2 => {
                let mut tokens: BTreeSet<u32> = names::variant_tokens(held).collect();
                if let Ok(rows) = held.complete::<tables::CountryMarket>() {
                    tokens.extend(rows.iter().flat_map(|row| row.subsidies.iter().map(|grant| grant.archetype)));
                }
                if let Ok(rows) = held.complete::<tables::MarketContractTerms>() {
                    tokens.extend(rows.iter().flat_map(|row| row.subsidies.iter().map(|grant| grant.archetype)));
                }
                names::spelling_reads(held, tokens, true)
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        names::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let reply = market_at(frame, params, at.holders, at.context, at.live)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub(crate) fn market_at(frame: &DecodedFrame, params: &MarketLendLeaseParams, holders: &Holders, context: &names::Context, live: &Cells) -> io::Result<MarketLendLeaseReply> {
    let [show_offers, show_contracts, show_agreements] = sections(params);
    let hour = frame.hour;
    let mut reply = MarketLendLeaseReply {
        branch_id: params.branch_id.clone(),
        offers: None,
        contracts: None,
        agreements: None,
        purchases_missing: None,
        unattributed: 0,
        variants: Vec::new(),
        icons: Vec::new(),
        warnings: Vec::new(),
    };
    let links = match frame.complete::<tables::CountryLinks>() {
        Ok(links) => links,
        Err(error) => {
            reply.warnings.push(format!("Countries are not recorded at this hour, so no market row can be named or chosen: {error}"));
            return Ok(reply);
        }
    };
    let parties = Parties::new(links, holders, hour);
    let wanted = chosen(holders, &params.countries, hour, &mut reply.warnings);
    let selected = |raw: u32| wanted.as_ref().is_none_or(|slots| parties.slot(raw).is_some_and(|slot| slots.contains(&slot)));
    let offers = section(show_offers, frame.complete::<tables::CountryMarket>(), "Market offers", &mut reply.warnings);
    let terms = section(show_contracts, frame.complete::<tables::MarketContractTerms>(), "Market requests and contracts", &mut reply.warnings);
    let leases = section(show_agreements, frame.complete::<tables::LendLeaseAgreements>(), "Lend-lease agreements", &mut reply.warnings);
    let offers: Option<Vec<_>> = offers.map(|rows| rows.iter().filter(|row| selected(row.country)).collect());
    let terms: Option<Vec<_>> = terms.map(|rows| rows.iter().filter(|row| selected(row.country) || selected(row.buyer)).collect());
    let leases: Option<Vec<_>> = leases.map(|rows| rows.iter().filter(|row| selected(row.country) || selected(row.receiver)).collect());

    let mut variants: BTreeSet<u64> = BTreeSet::new();
    for row in offers.iter().flatten() {
        variants.extend(pool_variants(&row.inventory));
        variants.extend(row.prices.iter().map(|level| level.variant));
    }
    for row in terms.iter().flatten() {
        variants.extend(pool_variants(&row.equipment).chain(pool_variants(&row.prices)));
        variants.extend(row.price_levels.iter().map(|level| level.variant));
    }
    for row in leases.iter().flatten() {
        variants.extend(lease_variants(row));
    }
    variants.remove(&0);
    let mut named = Named::default();
    let variants: Vec<u64> = variants.into_iter().collect();
    names::name(frame, &variants, &[], context, &mut named);
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    let texts: Option<&dyn Texts> = context.as_ref().ok().map(|content| content.catalogue.as_ref() as &dyn Texts);

    reply.offers = offers.map(|rows| {
        rows.into_iter()
            .map(|row| MarketOffer {
                country: row.country,
                tag: parties.tag(row.country),
                inventory: pool(&named, &row.inventory),
                prices: levels(&named, row.prices.iter()),
                subsidies: grants(row.subsidies.iter(), &parties, lexical, texts),
                subsidy_total_raw: row.subsidy_total_raw.to_string(),
                auto_send_market_access: row.auto_send_market_access,
            })
            .collect()
    });
    if let Some(rows) = terms {
        let purchases: HashMap<u64, (u32, u32)> = match frame.complete::<tables::PurchaseContracts>() {
            Ok(rows) => rows.iter().map(|purchase| (purchase.id, (purchase.seller, purchase.buyer))).collect(),
            Err(error) => {
                reply.purchases_missing = Some(format!("Purchase progress is not recorded at this hour: {error}"));
                HashMap::new()
            }
        };
        let column = |name| dashboard_declare::field(TableId::PurchaseContracts, name);
        let progress = [("assigned", column("convoys_assigned")?), ("convoys", column("convoys_total")?), ("collected", column("collected_raw")?), ("deliveries", column("completed_deliveries")?)];
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let key = (u128::from(row.country) << 72 | u128::from(row.id) << 8 | u128::from(row.kind)).to_string();
            let purchase = row.kind == 1 && purchases.get(&row.id) == Some(&(row.country, row.buyer));
            if purchase {
                for (name, column) in progress {
                    live.value(&key, name, Cell { table: TableId::PurchaseContracts, column, key: u128::from(row.id) }, 1.0);
                }
            }
            let (seller, buyer) = (parties.tag(row.country), parties.tag(row.buyer));
            if wanted.is_some() && (seller.is_none() || buyer.is_none()) {
                reply.unattributed += 1;
            }
            out.push(MarketTerms {
                key,
                id: format!("{:016x}", row.id),
                seller,
                buyer,
                request: row.kind == 0,
                speed: row.speed,
                equipment: pool(&named, &row.equipment),
                prices: pool(&named, &row.prices),
                levels: levels(&named, row.price_levels.iter()),
                subsidies: grants(row.subsidies.iter(), &parties, lexical, texts),
                purchase,
            });
        }
        reply.contracts = Some(out);
    }
    if let Some(rows) = leases {
        let sent = dashboard_declare::field(TableId::LendLeaseAgreements, "fuel_sent_raw")?;
        let lost = dashboard_declare::field(TableId::LendLeaseAgreements, "fuel_sunk_raw")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let terms = lease_terms(row, |raw| parties.tag(raw), &named);
            let key = lease_key(row);
            live.value(&terms.key, "fuelSent", Cell { table: TableId::LendLeaseAgreements, column: sent, key }, 1.0);
            live.value(&terms.key, "fuelLost", Cell { table: TableId::LendLeaseAgreements, column: lost, key }, 1.0);
            if wanted.is_some() && (terms.giver.is_none() || terms.receiver.is_none()) {
                reply.unattributed += 1;
            }
            out.push(terms);
        }
        reply.agreements = Some(out);
    }
    if reply.unattributed > 0 {
        reply.warnings.push(format!("{} market or lend-lease rows name a country with no tag at this hour; a chosen country may take part in them", reply.unattributed));
    }
    reply.warnings.extend(std::mem::take(&mut named.warnings));
    reply.icons = named.icons();
    reply.variants = named.variants.into_values().map(|variant| VariantName { hour: None, ..variant }).collect();
    Ok(reply)
}
