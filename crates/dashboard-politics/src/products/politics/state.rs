
use dashboard_effects::ScriptedEffects;
use super::token;
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{Country, LoadContext, at_hour::{At, AtHour, Owner, Read, ReadsAt}}};
use crate::products::country_people::{self, Links};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::identity::Holder;
use dashboard_text::{catalogue::plain_text, scripted_text::{EventObject, EventScope, EventScopes, ScriptedText, probe_unselected_tables}};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet}, io, sync::Arc};
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
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub country: Option<Country>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceSide {
    pub token: String,
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceRange {
    pub token: String,
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub reason: Option<String>,
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ChangeShown {
    Daily,
    Weekly,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceChange {
    pub daily: String,
    pub weekly: String,
    pub weekly_change: String,
    pub shown: ChangeShown,
    pub rate: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceRecord {
    pub ordinal: u32,
    pub token: String,
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name_reason: Option<String>,
    pub left: BalanceSide,
    pub right: BalanceSide,
    #[doc = " The side it is moving toward, when the record names one."]
    pub trending: Option<BalanceSide>,
    pub range: Option<BalanceRange>,
    #[doc = " Every authored range in authored order; empty when content refuses."]
    pub ranges: Vec<BalanceRange>,
    pub members: Vec<BalanceMember>,
    pub change: Option<BalanceChange>,
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
    #[doc = " Null when the country has no identity at this hour; the exile is still witnessed."]
    pub tag: Option<String>,
    pub host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PoliticsStateReply {
    pub branch_id: String,
    pub view: PoliticsView,
    pub exiles: Vec<StateExile>,
    #[doc = " Why `exiles` may not hold every government in exile at the hour; null when every country's exile status was read."]
    pub exiles_missing: Option<String>,
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
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = PoliticsStateParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, view, country];
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
            BalanceChange::decl(),
            ChangeShown::decl(),
            BalanceSource::decl(),
            BalanceRecord::decl(),
            CurrentBalance::decl(),
            PoliticsUnavailable::decl(),
            StateExile::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid politics branch")?;
        if let Some(country) = &params.country {
            dashboard_base::products::validate_country(country)?;
        }
        Ok(())
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        match params.view {
            PoliticsView::Exiles => country_people::EXILE_TABLES.to_vec(),
            PoliticsView::Balances => BALANCE_TABLES.to_vec(),
        }
    }

    fn reads_in(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if step == 0 {
            return Self::tables(params).into_iter().map(Read::all).collect();
        }
        if step > MAX_SCRIPTED_STAGES || params.view != PoliticsView::Balances {
            return Vec::new();
        }
        let Some(Ok(catalogue)) = at.context.catalogue.as_ref().map(|held| held.as_deref()) else {
            return Vec::new();
        };
        let requested_slot = selected_slot(params.country.as_ref(), at.holders, at.hour);
        probe_balance_tables(held, catalogue, params.country.is_some(), requested_slot)
            .into_iter()
            .map(Read::all)
            .collect()
    }

    fn valid_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        let Some(Ok(catalogue)) = at.context.catalogue.as_ref().map(|held| held.as_deref()) else { return i32::MAX };
        if params.view != PoliticsView::Balances || !names_read_the_date(frame, catalogue) {
            return i32::MAX;
        }
        frame.hour.div_euclid(24).saturating_add(1).saturating_mul(24).saturating_sub(1)
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
        if let Some(reason) = params.country.as_ref().filter(|_| matches!(params.view, PoliticsView::Balances)).and_then(|country| at.holders.not_known(country, at.hour)) {
            return Err(dashboard_core::refused(reason));
        }
        let requested_slot = selected_slot(params.country.as_ref(), at.holders, at.hour);
        read_for(frame, params, catalogue, params.country.is_some(), requested_slot, at.live)
    }
}

const MAX_SCRIPTED_STAGES: usize = 8;

fn selected_slot(country: Option<&Country>, holders: &dashboard_query::identity::Holders, hour: i32) -> Option<u32> {
    let country = country?;
    match holders.at(country, hour).map(|run| run.holder) {
        Some(Holder::Found { slot, .. }) => Some(slot),
        _ => None,
    }
}

pub fn read(frame: &DecodedFrame, params: &PoliticsStateParams, catalogue: Option<Result<&Catalogue, &String>>, live: &Cells) -> io::Result<PoliticsStateReply> {
    read_for(frame, params, catalogue, false, None, live)
}

fn read_for(
    frame: &DecodedFrame,
    params: &PoliticsStateParams,
    catalogue: Option<Result<&Catalogue, &String>>,
    filter_country: bool,
    requested_slot: Option<u32>,
    live: &Cells,
) -> io::Result<PoliticsStateReply> {
    let mut reply = PoliticsStateReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        exiles: Vec::new(),
        exiles_missing: None,
        balances: Vec::new(),
        current: None,
        unavailable: Vec::new(),
        warnings: Vec::new(),
    };
    match params.view {
        PoliticsView::Exiles => exiles(frame, &mut reply, live),
        PoliticsView::Balances => balances(frame, catalogue, filter_country, requested_slot, &mut reply, live)?,
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
    if refusals[0] != 0 {
        reply.exiles_missing = Some(format!("{} countries' exile hosts are not recorded at this hour", refusals[0]));
    }
    let missing_tag = rows.iter().filter(|row| row.exile.tag.is_none()).count();
    if missing_tag != 0 {
        reply.refuse("exile identity", "the country has no identity at this hour", u32::try_from(missing_tag).unwrap_or(u32::MAX));
    }
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

pub(super) const NO_LOCALISATION: &str = "no localisation in the recorded content";
pub(super) const SHARED_RANGE_SCOPE_REFUSAL: &str = "the active range name needs a country event scope, which a shared balance without a chosen country cannot name";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NameScope {
    Empty,
    Country(Option<u32>),
}

pub(super) fn has_brackets(text: &str) -> bool {
    text.find('[').is_some_and(|open| text[open + 1..].contains(']'))
}

pub(super) fn raw_scripted_text(catalogue: &Catalogue, key: &str) -> Option<String> {
    if key.is_empty() || catalogue.localisation.get(key).is_none() {
        return None;
    }
    let raw = catalogue.localisation.localise(key.as_bytes()).ok()?;
    has_brackets(&raw).then_some(raw)
}

pub(super) fn expand_scoped(text: &ScriptedText<'_>, raw: &str, scope: NameScope) -> Result<String, String> {
    let expanded = match scope {
        NameScope::Empty => text.expand_empty(raw)?,
        NameScope::Country(Some(slot)) => {
            let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
            text.expand_in(raw, &scopes)?
        }
        NameScope::Country(None) => return Err(SHARED_RANGE_SCOPE_REFUSAL.to_owned()),
    };
    plain_text(&expanded)
}

pub(super) fn resolve_balance_name<'a>(
    catalogue: Option<&'a Catalogue>,
    catalogue_reason: Option<&str>,
    frame: Option<&'a DecodedFrame>,
    scripted: &mut Option<Result<ScriptedText<'a>, String>>,
    key: &str,
    scope: NameScope,
) -> (Option<String>, Option<String>) {
    if key.is_empty() {
        return (None, None);
    }
    let Some(catalogue) = catalogue else {
        return (None, catalogue_reason.map(str::to_owned));
    };
    if catalogue.localisation.get(key).is_none() {
        return (None, Some(NO_LOCALISATION.to_owned()));
    }
    let raw = match catalogue.localisation.localise(key.as_bytes()) {
        Ok(text) => text,
        Err(why) => return (None, Some(why.to_string())),
    };
    if !has_brackets(&raw) {
        return match plain_text(&raw) {
            Ok(text) => (Some(text), None),
            Err(why) => (None, Some(why)),
        };
    }
    let Some(frame) = frame else {
        return (None, Some("the name needs scripted localisation evaluated at a specific hour".to_owned()));
    };
    let text = scripted.get_or_insert_with(|| {
        catalogue.scripted_content().map(|content| ScriptedText::new(frame, &catalogue.localisation, content, 0))
    });
    match text {
        Ok(text) => match expand_scoped(text, &raw, scope) {
            Ok(name) => (Some(name), None),
            Err(why) => (None, Some(why)),
        },
        Err(why) => (None, Some(why.clone())),
    }
}

fn record_range_slot(
    listed: &[MemberRow],
    links: &Links<'_>,
    filter_country: bool,
    requested_slot: Option<u32>,
) -> Option<u32> {
    if filter_country {
        return requested_slot;
    }
    let mut found: Option<u32> = None;
    for member in listed {
        let slot = links.slot(member.country)?;
        match found {
            None => found = Some(slot),
            Some(existing) if existing == slot => {}
            Some(_) => return None,
        }
    }
    found
}

type MemberRow = replay_data::rows::PowerBalanceMemberRow;

fn by_record(members: &[MemberRow]) -> BTreeMap<u32, Vec<MemberRow>> {
    let mut out: BTreeMap<u32, Vec<MemberRow>> = BTreeMap::new();
    for member in members {
        out.entry(member.record).or_default().push(member.clone());
    }
    out
}

fn selected_records(members: &[MemberRow], links: &Links<'_>, filter_country: bool, requested_slot: Option<u32>) -> Option<BTreeSet<u32>> {
    filter_country.then(|| match requested_slot {
        Some(slot) => members.iter().filter(|member| links.slot(member.country) == Some(slot)).map(|member| member.record).collect(),
        None => BTreeSet::new(),
    })
}

pub(super) fn names_read_the_date(frame: &DecodedFrame, catalogue: &Catalogue) -> bool {
    let Ok(records) = frame.complete::<tables::PowerBalances>() else { return false };
    let effects = catalogue.scripted_content().ok().and_then(|content| content.effects().ok());
    let power_balances = effects.as_ref().and_then(|effects| effects.view(None).power_balances);
    records.iter().any(|record| {
        [&record.balance, &record.left, &record.right, &record.trending, &record.active_range].into_iter().any(|bytes| raw_scripted_text(catalogue, &token(bytes)).is_some())
            || power_balances.and_then(|database| database.ranges(&record.balance)).is_some_and(|ranges| ranges.iter().any(|range| raw_scripted_text(catalogue, &token(&range.id)).is_some()))
    })
}

pub(crate) fn change_of(members: &[MemberRow]) -> Option<BalanceChange> {
    if members.is_empty() {
        return None;
    }
    let (mut daily, mut weekly) = (0_i64, 0_i64);
    for member in members {
        let row = member_row(member);
        daily = daily.wrapping_add(row.daily_effective());
        weekly = weekly.wrapping_add(row.weekly_effective());
    }
    let drift = dashboard_text::scripted_text::balance_drift(daily, weekly);
    let (shown, rate) = match drift.shown {
        dashboard_text::scripted_text::ChangeRate::Daily(rate) => (ChangeShown::Daily, rate),
        dashboard_text::scripted_text::ChangeRate::Weekly(rate) => (ChangeShown::Weekly, rate),
    };
    Some(BalanceChange { daily: daily.to_string(), weekly: weekly.to_string(), weekly_change: drift.weekly_change.to_string(), shown, rate: rate.to_string() })
}

fn probe_balance_tables(
    frame: &DecodedFrame,
    catalogue: &Catalogue,
    filter_country: bool,
    requested_slot: Option<u32>,
) -> Vec<TableId> {
    let (Ok(records), Ok(members), Ok(links)) = (
        frame.complete::<tables::PowerBalances>(),
        frame.complete::<tables::PowerBalanceMembers>(),
        Links::new(frame),
    ) else {
        return Vec::new();
    };
    let effects = catalogue.scripted_content().ok().and_then(|content| content.effects().ok());
    let power_balances = effects.as_ref().and_then(|effects| effects.view(None).power_balances);
    let selected = selected_records(members, &links, filter_country, requested_slot);
    let grouped = by_record(members);
    let mut items: Vec<(String, NameScope)> = Vec::new();
    for record in records {
        if selected.as_ref().is_some_and(|selected| !selected.contains(&record.ordinal)) {
            continue;
        }
        let listed = grouped.get(&record.ordinal).map_or(&[][..], Vec::as_slice);
        let range_slot = record_range_slot(listed, &links, filter_country, requested_slot);
        for bytes in [&record.balance, &record.left, &record.right, &record.trending] {
            let key = token(bytes);
            if let Some(raw) = raw_scripted_text(catalogue, &key) {
                items.push((raw, NameScope::Empty));
            }
        }
        let active = token(&record.active_range);
        if let Some(raw) = raw_scripted_text(catalogue, &active) {
            items.push((raw, NameScope::Country(range_slot)));
        }
        if let Some(ranges) = power_balances.and_then(|database| database.ranges(&record.balance)) {
            for range in ranges {
                let key = token(&range.id);
                if let Some(raw) = raw_scripted_text(catalogue, &key) {
                    items.push((raw, NameScope::Country(range_slot)));
                }
            }
        }
    }
    if items.is_empty() {
        return Vec::new();
    }
    let Ok(content) = catalogue.scripted_content() else {
        return Vec::new();
    };
    let text = ScriptedText::new(frame, &catalogue.localisation, content, 0);
    probe_unselected_tables(frame, || {
        for (raw, scope) in &items {
            let _ = expand_scoped(&text, raw, *scope);
        }
    })
}

fn name<'a>(
    catalogue: Option<&'a Catalogue>,
    catalogue_reason: Option<&str>,
    frame: &'a DecodedFrame,
    scripted: &mut Option<Result<ScriptedText<'a>, String>>,
    key: &str,
    scope: NameScope,
    label: &str,
    reply: &mut PoliticsStateReply,
) -> (Option<String>, Option<String>) {
    let (resolved, reason) = resolve_balance_name(catalogue, catalogue_reason, Some(frame), scripted, key, scope);
    if resolved.is_none() && !key.is_empty() && catalogue.is_some() {
        if let Some(why) = &reason {
            reply.refuse(label, why.clone(), 1);
        }
    }
    (resolved, reason)
}

fn side<'a>(
    catalogue: Option<&'a Catalogue>,
    catalogue_reason: Option<&str>,
    frame: &'a DecodedFrame,
    scripted: &mut Option<Result<ScriptedText<'a>, String>>,
    bytes: &[u8],
    reply: &mut PoliticsStateReply,
) -> BalanceSide {
    let token = token(bytes);
    let (name, reason) = name(catalogue, catalogue_reason, frame, scripted, &token, NameScope::Empty, "balance side names", reply);
    BalanceSide { token, name, reason }
}

fn balances(
    frame: &DecodedFrame,
    catalogue: Option<Result<&Catalogue, &String>>,
    filter_country: bool,
    requested_slot: Option<u32>,
    reply: &mut PoliticsStateReply,
    live: &Cells,
) -> io::Result<()> {
    let value_column = column(TableId::PowerBalances, "value_raw")?;
    let links = Links::new(frame)?;
    let (records, members) = match (frame.complete::<tables::PowerBalances>(), frame.complete::<tables::PowerBalanceMembers>()) {
        (Ok(records), Ok(members)) => (records, members),
        (Err(error), _) | (_, Err(error)) => {
            reply.refuse("balances of power", error.to_string(), 1);
            return Ok(());
        }
    };
    let (catalogue, catalogue_reason) = match catalogue {
        Some(Ok(catalogue)) => (Some(catalogue), None),
        Some(Err(reason)) => {
            reply.refuse("balance names", reason.clone(), 1);
            (None, Some(reason.as_str()))
        }
        None => (None, None),
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
    let mut scripted = None;
    let selected = selected_records(members, &links, filter_country, requested_slot);
    let grouped = by_record(members);
    for record in records {
        if selected.as_ref().is_some_and(|selected| !selected.contains(&record.ordinal)) {
            continue;
        }
        let listed = grouped.get(&record.ordinal).map_or(&[][..], Vec::as_slice);
        let range_slot = record_range_slot(listed, &links, filter_country, requested_slot);
        let balance = token(&record.balance);
        let ranges: Vec<_> = content
            .and_then(|database| database.ranges(&record.balance))
            .map(|ranges| {
                ranges
                    .iter()
                    .map(|range| {
                        let token = token(&range.id);
                        let (name, reason) = name(catalogue, catalogue_reason, frame, &mut scripted, &token, NameScope::Country(range_slot), "balance range names", reply);
                        BalanceRange { token, name, reason, min: Some(range.min.to_string()), max: Some(range.max.to_string()) }
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
            let existing = ranges.iter().find(|range| range.token == active);
            let (resolved_name, resolved_reason) = match existing {
                Some(found) => (found.name.clone(), found.reason.clone()),
                None => name(catalogue, catalogue_reason, frame, &mut scripted, &active, NameScope::Country(range_slot), "balance range names", reply),
            };
            BalanceRange {
                name: resolved_name,
                reason: resolved_reason,
                token: active.clone(),
                min: bounds.as_ref().map(|(min, _)| min.clone()),
                max: bounds.map(|(_, max)| max),
            }
        });
        let trending = (!content_reader_parser::parser::c_string(&record.trending).is_empty()).then(|| side(catalogue, catalogue_reason, frame, &mut scripted, &record.trending, reply));
        let record_members = listed
            .iter()
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
        let (balance_name, balance_reason) = name(catalogue, catalogue_reason, frame, &mut scripted, &balance, NameScope::Empty, "balance names", reply);
        let left = side(catalogue, catalogue_reason, frame, &mut scripted, &record.left, reply);
        let right = side(catalogue, catalogue_reason, frame, &mut scripted, &record.right, reply);
        live.value(&record.ordinal.to_string(), "value", Cell { table: TableId::PowerBalances, column: value_column, key: record.ordinal.into() }, 1.0);
        reply.balances.push(BalanceRecord {
            ordinal: record.ordinal,
            name: balance_name,
            name_reason: balance_reason,
            token: balance,
            left,
            right,
            trending,
            range,
            ranges,
            members: record_members,
            change: change_of(listed),
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
            BalanceSource { name: catalogue.localised(&token, dashboard_text::catalogue::View::Plain), token, daily: daily.value.to_string(), weekly: weekly.value.to_string() }
        })
        .collect())
}
