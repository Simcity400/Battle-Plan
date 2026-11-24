
use dashboard_effects::ScriptedEffects;
use super::token;
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{LoadContext, at_hour::{At, AtHour}}};
use crate::products::country_people::{self, Links};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_text::catalogue::View;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PoliticsView {
    Exiles,
    Balances,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsStateParams {
    pub branch_id: String,
    pub view: PoliticsView,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceSide {
    pub token: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceRange {
    pub token: String,
    pub name: Option<String>,
    #[doc = " Fixed5 bounds; null where the recorded content does not establish them."]
    pub min: Option<String>,
    pub max: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceMember {
    pub tag: Option<String>,
    pub daily: Option<String>,
    pub weekly: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceSource {
    pub token: String,
    pub name: Option<String>,
    pub daily: String,
    pub weekly: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceRecord {
    pub ordinal: u32,
    pub token: String,
    pub name: Option<String>,
    pub left: BalanceSide,
    pub right: BalanceSide,
    #[doc = " The side it is moving toward, when the record names one."]
    pub trending: Option<BalanceSide>,
    pub range: Option<BalanceRange>,
    #[doc = " Every authored range in authored order; empty when content refuses."]
    pub ranges: Vec<BalanceRange>,
    pub members: Vec<BalanceMember>,
    #[doc = " The drift's sources; null when they cannot be derived."]
    pub sources: Option<Vec<BalanceSource>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CurrentBalance {
    pub tag: String,
    pub record: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PoliticsUnavailable {
    pub label: String,
    pub reason: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StateExile {
    pub id: String,
    pub tag: String,
    pub host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PoliticsStateReply {
    pub branch_id: String,
    pub view: PoliticsView,
    pub exiles: Vec<StateExile>,
    pub balances: Vec<BalanceRecord>,
    #[doc = " Null when the membership cannot say which record each country shows."]
    pub current: Option<Vec<CurrentBalance>>,
    pub unavailable: Vec<PoliticsUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PoliticsStateReply);

impl PoliticsStateReply {
    fn refuse(&mut self, label: &str, reason: impl Into<String>, count: u32) {
        let reason = reason.into();
        if let Some(entry) = self.unavailable.iter_mut().find(|entry| entry.label == label && entry.reason == reason) {
            entry.count = entry.count.saturating_add(count);
        } else {
            self.unavailable.push(PoliticsUnavailable { label: label.into(), reason, count });
        }
    }
}

pub const BALANCE_TABLES: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::PowerBalanceModifierBreakdowns,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
];

pub struct PoliticsState;

pub struct PoliticsContext {
    catalogue: Option<Result<Arc<Catalogue>, String>>,
}

impl AtHour for PoliticsState {
    const KEY: &'static str = "politics-state";
    const TITLE: &'static str = "Politics";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PoliticsStateParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, view];
    type Reply = PoliticsStateReply;
    type Context = PoliticsContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            PoliticsStateParams::decl(),
            PoliticsStateReply::decl(),
            PoliticsView::decl(),
            BalanceSide::decl(),
            BalanceRange::decl(),
            BalanceMember::decl(),
            BalanceSource::decl(),
            BalanceRecord::decl(),
            CurrentBalance::decl(),
            PoliticsUnavailable::decl(),
            StateExile::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid politics branch")
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        match params.view {
            PoliticsView::Exiles => country_people::EXILE_TABLES.to_vec(),
            PoliticsView::Balances => BALANCE_TABLES.to_vec(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let catalogue = load.content().and_then(|content| catalogue::catalogue(source, content)).map_err(|error| error.to_string());
        Ok(PoliticsContext { catalogue: Some(catalogue) })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let catalogue = match params.view {
            PoliticsView::Balances => at.context.catalogue.as_ref().map(|held| held.as_deref()),
            PoliticsView::Exiles => None,
        };
        read(frame, params, catalogue, at.live)
    }
}

pub fn read(frame: &DecodedFrame, params: &PoliticsStateParams, catalogue: Option<Result<&Catalogue, &String>>, live: &Cells) -> io::Result<PoliticsStateReply> {
    let mut reply = PoliticsStateReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        exiles: Vec::new(),
        balances: Vec::new(),
        current: None,
        unavailable: Vec::new(),
        warnings: Vec::new(),
    };
    match params.view {
        PoliticsView::Exiles => exiles(frame, &mut reply, live),
        PoliticsView::Balances => balances(frame, catalogue, &mut reply, live)?,
    }
    for entry in &reply.unavailable {
        reply.warnings.push(format!("{} refused {}: {}", entry.count, entry.label, entry.reason));
    }
    Ok(reply)
}

fn column(table: TableId, name: &str) -> io::Result<usize> {
    dashboard_declare::field(table, name)
}

fn exiles(frame: &DecodedFrame, reply: &mut PoliticsStateReply, live: &Cells) {
    if let Err(error) = frame.complete::<tables::CountryStabilityInputs>() {
        reply.refuse("exile status", error.to_string(), 1);
        return;
    }
    let (rows, refusals) = country_people::exile_rows(frame, |_, _| true);
    for (count, field) in refusals.into_iter().zip(["exile host roster", "exile legitimacy", "exile manpower", "exile manpower cap", "exile leader resources"]) {
        if count != 0 {
            reply.refuse(field, "operands unavailable", count);
        }
    }
    let missing_air = rows.iter().filter(|row| row.exile.deployed_air.is_none()).count();
    if missing_air != 0 {
        reply.refuse("exile deployed air crews", "wing attribution or manpower unavailable", u32::try_from(missing_air).unwrap_or(u32::MAX));
    }
    let missing_host = rows.iter().filter(|row| row.exile.host.is_none()).count();
    if missing_host != 0 {
        reply.refuse("exile host identity", "the host has no identity at this hour", u32::try_from(missing_host).unwrap_or(u32::MAX));
    }
    let inputs = TableId::CountryStabilityInputs;
    let found = (|| -> io::Result<_> {
        let numbers = [column(inputs, "exile_legitimacy")?, column(inputs, "exile_manpower")?, column(inputs, "exile_manpower_cap")?, column(inputs, "exile_leaders")?];
        Ok((numbers, column(TableId::CountryDeployedManpower, "exile_air")?, column(TableId::SliceReads, "sample_hour")?))
    })();
    let Ok(([legitimacy, manpower, cap, leaders], air, read)) = found else {
        reply.refuse("exile numbers", "this build's schema names no column they are read from", u32::try_from(rows.len()).unwrap_or(u32::MAX));
        return;
    };
    for row in rows {
        let id = row.country.to_string();
        let at = |column| Cell { table: inputs, column, key: u128::from(row.country) };
        live.value(&id, "legitimacy", at(legitimacy), 1e-5);
        live.value(&id, "manpower", at(manpower), 1.0);
        live.value(&id, "cap", at(cap), 1.0);
        live.value(&id, "leaders", at(leaders), 1.0);
        live.value(&id, "readHour", Cell { table: TableId::SliceReads, column: read, key: (inputs.index() as u128) << 32 | u128::from(row.country) }, 1.0);
        if let Some(slot) = row.slot {
            live.value(&id, "deployedAir", Cell { table: TableId::CountryDeployedManpower, column: air, key: slot.into() }, 1.0);
        }
        reply.exiles.push(StateExile { id, tag: row.exile.tag, host: row.exile.host });
    }
}

fn name(catalogue: Option<&Catalogue>, key: &str, label: &str, reply: &mut PoliticsStateReply) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let catalogue = catalogue?;
    match catalogue.localised(key, View::Plain) {
        Some(text) if text.contains('[') && text.contains(']') => {
            reply.refuse(label, "the name needs scripted localisation this view cannot evaluate", 1);
            None
        }
        Some(text) => Some(text),
        None => {
            reply.refuse(label, "no localisation in the recorded content", 1);
            None
        }
    }
}

fn side(catalogue: Option<&Catalogue>, bytes: &[u8], reply: &mut PoliticsStateReply) -> BalanceSide {
    let token = token(bytes);
    BalanceSide { name: name(catalogue, &token, "balance side names", reply), token }
}

fn balances(frame: &DecodedFrame, catalogue: Option<Result<&Catalogue, &String>>, reply: &mut PoliticsStateReply, live: &Cells) -> io::Result<()> {
    let value_column = column(TableId::PowerBalances, "value_raw")?;
    let links = Links::new(frame)?;
    let (records, members) = match (frame.complete::<tables::PowerBalances>(), frame.complete::<tables::PowerBalanceMembers>()) {
        (Ok(records), Ok(members)) => (records, members),
        (Err(error), _) | (_, Err(error)) => {
            reply.refuse("balances of power", error.to_string(), 1);
            return Ok(());
        }
    };
    let catalogue = match catalogue {
        Some(Ok(catalogue)) => Some(catalogue),
        Some(Err(reason)) => {
            reply.refuse("balance names", reason.clone(), 1);
            None
        }
        None => None,
    };
    let effects = catalogue.map(|catalogue| catalogue.scripted_content().and_then(|content| content.effects()).map(|effects| (catalogue, effects)));
    let (content, statics) = match &effects {
        Some(Ok((catalogue, effects))) => {
            let view = effects.view(None);
            (view.power_balances, view.static_modifiers.map(|statics| (*catalogue, statics)))
        }
        Some(Err(reason)) => {
            reply.refuse("balance ranges", reason.clone(), 1);
            (None, None)
        }
        None => (None, None),
    };
    if catalogue.is_some() && content.is_none() && matches!(effects, Some(Ok(_))) {
        reply.refuse("balance ranges", "the recorded content declares no balances of power", 1);
    }
    let breakdowns = frame.complete::<tables::PowerBalanceModifierBreakdowns>().ok();
    let registry = replay_derive::modifier_registry::Registry::from_frame(frame);
    let mut unresolved = 0_u32;
    let mut current = BTreeMap::<u32, u32>::new();
    for member in members {
        match links.slot(member.country) {
            Some(slot) => {
                current.entry(slot).and_modify(|record| *record = (*record).min(member.record)).or_insert(member.record);
            }
            None => unresolved += 1,
        }
    }
    if unresolved == 0 {
        reply.current = Some(current.into_iter().filter_map(|(slot, record)| Some(CurrentBalance { tag: links.slot_tag(slot)?, record })).collect());
    } else {
        reply.refuse("balance members", "a member has no identity at this hour, so no country's shown balance is known", unresolved);
    }
    for record in records {
        let balance = token(&record.balance);
        let ranges: Vec<_> = content
            .and_then(|database| database.ranges(&record.balance))
            .map(|ranges| {
                ranges
                    .iter()
                    .map(|range| {
                        let token = token(&range.id);
                        BalanceRange { name: name(catalogue, &token, "balance range names", reply), token, min: Some(range.min.to_string()), max: Some(range.max.to_string()) }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if content.is_some() && ranges.is_empty() {
            reply.refuse("balance ranges", format!("balance {balance} is not declared by the recorded content"), 1);
        }
        let active = token(&record.active_range);
        let authored = content.map(|database| database.range(&record.balance, &record.active_range));
        if let Some(Err(reason)) = &authored {
            reply.refuse("balance ranges", reason.clone(), 1);
        }
        let range = (!active.is_empty()).then(|| {
            let bounds = match authored {
                Some(Ok(Some(range))) => Some((range.min.to_string(), range.max.to_string())),
                Some(Ok(None)) => {
                    reply.refuse("balance range bounds", format!("range {active} is not among its balance's authored ranges"), 1);
                    None
                }
                Some(Err(_)) | None => None,
            };
            BalanceRange {
                name: ranges.iter().find(|range| range.token == active).and_then(|range| range.name.clone()),
                token: active.clone(),
                min: bounds.as_ref().map(|(min, _)| min.clone()),
                max: bounds.map(|(_, max)| max),
            }
        });
        let trending = (!content_reader_parser::parser::c_string(&record.trending).is_empty()).then(|| side(catalogue, &record.trending, reply));
        let record_members = members
            .iter()
            .filter(|member| member.record == record.ordinal)
            .map(|member| {
                let row = member_row(member);
                BalanceMember {
                    tag: links.tag(member.country),
                    daily: member.daily_present.then(|| row.daily_effective().to_string()),
                    weekly: member.weekly_present.then(|| row.weekly_effective().to_string()),
                }
            })
            .collect();
        let sources = match (&statics, &registry, breakdowns) {
            (Some((catalogue, statics)), Ok(registry), Some(breakdowns)) => match breakdowns.iter().find(|row| row.record == record.ordinal) {
                Some(row) => match drift_sources(catalogue, statics, registry, &row.sources) {
                    Ok(sources) => Some(sources),
                    Err(reason) => {
                        reply.refuse("balance drift sources", reason, 1);
                        None
                    }
                },
                None => {
                    reply.refuse("balance drift sources", "the record's static-modifier roster is not recorded", 1);
                    None
                }
            },
            (_, Err(reason), _) => {
                reply.refuse("balance drift sources", reason.clone(), 1);
                None
            }
            (_, _, None) => {
                reply.refuse("balance drift sources", "the static-modifier rosters are not recorded", 1);
                None
            }
            (None, _, _) => None,
        };
        let balance_name = name(catalogue, &balance, "balance names", reply);
        let left = side(catalogue, &record.left, reply);
        let right = side(catalogue, &record.right, reply);
        live.value(&record.ordinal.to_string(), "value", Cell { table: TableId::PowerBalances, column: value_column, key: record.ordinal.into() }, 1.0);
        reply.balances.push(BalanceRecord {
            ordinal: record.ordinal,
            name: balance_name,
            token: balance,
            left,
            right,
            trending,
            range,
            ranges,
            members: record_members,
            sources,
        });
    }
    Ok(())
}

fn member_row(row: &replay_data::rows::PowerBalanceMemberRow) -> engine_power_balance::power_balance_record::MemberRow {
    engine_power_balance::power_balance_record::MemberRow {
        record: row.record,
        ordinal: row.ordinal,
        country: row.country,
        daily_raw: row.daily_raw,
        weekly_raw: row.weekly_raw,
        daily_present: row.daily_present,
        weekly_present: row.weekly_present,
        daily_flags: row.daily_flags,
        weekly_flags: row.weekly_flags,
    }
}

fn drift_sources(
    catalogue: &Catalogue,
    statics: &content_reader_politics::static_modifiers::Descriptions,
    registry: &replay_derive::modifier_registry::Registry<'_>,
    sources: &[Vec<u8>],
) -> Result<Vec<BalanceSource>, String> {
    let layers = &catalogue.content.layers;
    let mut blocks = Vec::with_capacity(sources.len());
    for source in sources {
        let key = std::str::from_utf8(content_reader_parser::parser::c_string(source)).map_err(|_| format!("static modifier {} is not UTF-8", String::from_utf8_lossy(source)))?;
        blocks.push(statics.get(layers, key)?.1);
    }
    let inputs: Vec<_> = sources
        .iter()
        .zip(&blocks)
        .map(|(source, block)| replay_derive::power_balance_derive::Source {
            name: source,
            auxiliary: !block.tooltip_table().is_empty() || !block.children.is_empty(),
            values: block.values.iter().map(|value| (value.name.as_str(), value.value)).collect(),
        })
        .collect();
    let daily = replay_derive::power_balance_derive::tree(engine_executable::modifier_keys::POWER_BALANCE_DAILY, &inputs, registry)?;
    let weekly = replay_derive::power_balance_derive::tree(engine_executable::modifier_keys::POWER_BALANCE_WEEKLY, &inputs, registry)?;
    if daily.len() != weekly.len() {
        return Err("the daily and weekly source lists differ".into());
    }
    Ok(daily
        .into_iter()
        .zip(weekly)
        .skip(1)
        .map(|(daily, weekly)| {
            let token = token(&daily.name);
            BalanceSource { name: catalogue.localised(&token, View::Plain), token, daily: daily.value.to_string(), weekly: weekly.value.to_string() }
        })
        .collect())
}
