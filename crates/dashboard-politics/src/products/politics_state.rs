//! Hour-specific political details for every country at once, read from the
//! displayed hour alone: every government in exile with its deployed air
//! crews, and every balance of power the game stores with its drift sources.
//! Their histories are prepared ([`history`]); these reads supply only what
//! the history does not keep. Nothing here reads live memory.
//!
//! Exiles are the exile cards' own projection ([`country_people::exiles`])
//! for the whole world.
//! Balances keep the game's stored record order: a country's current balance
//! is the first record listing it, which needs the whole membership resolved.
//! Names and range bounds resolve through the recording's content stack;
//! a name that cannot resolve leaves its numbers exact. The weekly-change
//! sources derive from each record's recorded static-modifier roster through
//! [`replay_derive::power_balance_derive`], kept apart from the members'
//! recorded contributions without forcing the two to reconcile.
//!
//! The prepared balance history is [`history`].

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    invalid,
    products::{
        LoadContext, Product, check_reply_budget,
        country_people::{self, PeopleExile},
    },
};
use dashboard_text::catalogue::View;
use replay::container::SeekResult;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub mod civil_wars;
pub mod history;
#[cfg(test)]
pub(crate) mod tests;

pub struct PoliticsState;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PoliticsView {
    Exiles,
    Balances,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsStateParams {
    pub branch_id: String,
    pub hour: i32,
    pub view: PoliticsView,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSide {
    pub token: String,
    pub name: Option<String>,
}

/// An authored range, its inclusive fixed5 bounds as decimal strings.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRange {
    pub token: String,
    pub name: Option<String>,
    /// Fixed5 bounds; null where the recorded content does not establish them.
    pub min: Option<String>,
    pub max: Option<String>,
}

/// A record member's recorded contribution: the effective daily and weekly
/// values (fixed5 decimal strings), null where the record holds none.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceMember {
    pub tag: Option<String>,
    pub daily: Option<String>,
    pub weekly: Option<String>,
}

/// One static-modifier source of a record's drift, in roster order, with
/// its effective daily and weekly values (fixed5 decimal strings).
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSource {
    pub token: String,
    pub name: Option<String>,
    pub daily: String,
    pub weekly: String,
}

/// One stored balance record at the hour, in stored order.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRecord {
    pub ordinal: u32,
    pub token: String,
    pub name: Option<String>,
    /// The stored fixed5 value as a decimal string, from -100000 to 100000.
    pub value: String,
    pub left: BalanceSide,
    pub right: BalanceSide,
    /// The side it is moving toward, when the record names one.
    pub trending: Option<BalanceSide>,
    pub range: Option<BalanceRange>,
    /// Every authored range in authored order; empty when content refuses.
    pub ranges: Vec<BalanceRange>,
    pub members: Vec<BalanceMember>,
    /// The drift's sources; null when they cannot be derived.
    pub sources: Option<Vec<BalanceSource>>,
}

/// The record the game shows for a country: the first record listing it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CurrentBalance {
    pub tag: String,
    pub record: u32,
}

/// What a view cannot show, named and counted.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsUnavailable {
    pub label: String,
    pub reason: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsStateReply {
    pub branch_id: String,
    pub hour: i32,
    pub view: PoliticsView,
    pub exiles: Vec<PeopleExile>,
    pub balances: Vec<BalanceRecord>,
    /// Null when the membership cannot say which record each country shows.
    pub current: Option<Vec<CurrentBalance>>,
    pub unavailable: Vec<PoliticsUnavailable>,
    pub warnings: Vec<String>,
}

impl PoliticsStateReply {
    fn refuse(&mut self, label: &str, reason: impl Into<String>, count: u32) {
        let reason = reason.into();
        if let Some(entry) = self
            .unavailable
            .iter_mut()
            .find(|entry| entry.label == label && entry.reason == reason)
        {
            entry.count = entry.count.saturating_add(count);
        } else {
            self.unavailable.push(PoliticsUnavailable {
                label: label.into(),
                reason,
                count,
            });
        }
    }
}

impl Product for PoliticsState {
    const KEY: &'static str = "politics-state";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = PoliticsStateParams;
    type Reply = PoliticsStateReply;
    fn types() -> Vec<String> {
        vec![
            PoliticsStateParams::decl(),
            PoliticsView::decl(),
            BalanceSide::decl(),
            BalanceRange::decl(),
            BalanceMember::decl(),
            BalanceSource::decl(),
            BalanceRecord::decl(),
            CurrentBalance::decl(),
            PoliticsUnavailable::decl(),
            PoliticsStateReply::decl(),
        ]
    }
    fn validate(p: &PoliticsStateParams) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid politics branch")
    }
    fn load(
        source: &Source,
        p: &PoliticsStateParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<PoliticsStateReply> {
        if cancelled.load(Ordering::Acquire) {
            return Err(invalid("Politics request cancelled"));
        }
        let frame = match source.replay().seek_selected(
            p.branch_id.parse().unwrap(),
            p.hour,
            Some(tables_for(p.view)),
        )? {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { reason, .. } => {
                return Err(invalid(format!("Politics unavailable: {reason}")));
            }
        };
        let catalogue = match p.view {
            PoliticsView::Balances => Some(
                context
                    .content()
                    .and_then(|content| catalogue::catalogue(source, content))
                    .map_err(|error| error.to_string()),
            ),
            _ => None,
        };
        let reply = read(&frame, p, catalogue.as_ref().map(|c| c.as_deref()))?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

const EXILE_TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::AirWings,
];
pub const BALANCE_TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::PowerBalanceModifierBreakdowns,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
];

fn tables_for(view: PoliticsView) -> &'static [TableId] {
    match view {
        PoliticsView::Exiles => EXILE_TABLES,
        PoliticsView::Balances => BALANCE_TABLES,
    }
}

/// Raw country references to their tag at the hour, through the links.
pub(crate) struct Tags<'a> {
    links: BTreeMap<u32, u32>,
    slots: BTreeMap<u32, &'a [u8]>,
}
impl<'a> Tags<'a> {
    pub(crate) fn new(frame: &'a DecodedFrame) -> io::Result<Self> {
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        Ok(Self {
            links: links
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect(),
            slots: countries
                .iter()
                .map(|row| (row.canonical_slot, row.tag.as_slice()))
                .collect(),
        })
    }
    /// Every holder slot read at the hour.
    pub(crate) fn holders(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.keys().copied()
    }
    pub(crate) fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }
    pub(crate) fn tag(&self, raw: u32) -> Option<String> {
        self.slot_tag(self.slot(raw)?)
    }
    pub(crate) fn slot_tag(&self, slot: u32) -> Option<String> {
        self.slots
            .get(&slot)
            .and_then(|tag| std::str::from_utf8(tag).ok())
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
    }
}

pub fn read(
    frame: &DecodedFrame,
    p: &PoliticsStateParams,
    catalogue: Option<Result<&Catalogue, &String>>,
) -> io::Result<PoliticsStateReply> {
    let mut reply = PoliticsStateReply {
        branch_id: p.branch_id.clone(),
        hour: frame.hour,
        view: p.view,
        exiles: Vec::new(),
        balances: Vec::new(),
        current: None,
        unavailable: Vec::new(),
        warnings: Vec::new(),
    };
    match p.view {
        PoliticsView::Exiles => exiles(frame, &mut reply),
        PoliticsView::Balances => balances(frame, catalogue, &mut reply)?,
    }
    for entry in &reply.unavailable {
        reply
            .warnings
            .push(format!("{} refused {}: {}", entry.count, entry.label, entry.reason));
    }
    Ok(reply)
}

fn exiles(frame: &DecodedFrame, reply: &mut PoliticsStateReply) {
    if let Err(error) = frame.complete::<tables::CountryStabilityInputs>() {
        reply.refuse("exile status", error.to_string(), 1);
        return;
    }
    let (rows, refusals) = country_people::exiles(frame, |_, _| true);
    for (count, field) in refusals.into_iter().zip([
        "exile host roster",
        "exile legitimacy",
        "exile manpower",
        "exile manpower cap",
        "exile leader resources",
    ]) {
        if count != 0 {
            reply.refuse(field, "operands unavailable", count);
        }
    }
    let missing_air = rows.iter().filter(|row| row.deployed_air.is_none()).count();
    if missing_air != 0 {
        reply.refuse(
            "exile deployed air crews",
            "wing attribution or manpower unavailable",
            u32::try_from(missing_air).unwrap_or(u32::MAX),
        );
    }
    let missing_host = rows.iter().filter(|row| row.host.is_none()).count();
    if missing_host != 0 {
        reply.refuse(
            "exile host identity",
            "the host has no identity at this hour",
            u32::try_from(missing_host).unwrap_or(u32::MAX),
        );
    }
    reply.exiles = rows;
}

fn token(bytes: &[u8]) -> String {
    String::from_utf8_lossy(installed_content_core::parser::c_string(bytes)).into_owned()
}

/// A content name localised plainly. Text that needs scripted localisation
/// in a scope this view cannot reproduce is refused, not shown raw.
fn name(
    catalogue: Option<&Catalogue>,
    key: &str,
    label: &str,
    reply: &mut PoliticsStateReply,
) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let catalogue = catalogue?;
    match catalogue.localised(key, View::Plain) {
        Some(text) if text.contains('[') && text.contains(']') => {
            reply.refuse(
                label,
                "the name needs scripted localisation this view cannot evaluate",
                1,
            );
            None
        }
        Some(text) => Some(text),
        None => {
            reply.refuse(label, "no localisation in the recorded content", 1);
            None
        }
    }
}

fn side(
    catalogue: Option<&Catalogue>,
    bytes: &[u8],
    reply: &mut PoliticsStateReply,
) -> BalanceSide {
    let token = token(bytes);
    BalanceSide {
        name: name(catalogue, &token, "balance side names", reply),
        token,
    }
}

fn balances(
    frame: &DecodedFrame,
    catalogue: Option<Result<&Catalogue, &String>>,
    reply: &mut PoliticsStateReply,
) -> io::Result<()> {
    let tags = Tags::new(frame)?;
    let (records, members) = match (
        frame.complete::<tables::PowerBalances>(),
        frame.complete::<tables::PowerBalanceMembers>(),
    ) {
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
    let effects = catalogue.map(|catalogue| {
        catalogue
            .scripted_content()
            .and_then(|content| content.effects())
            .map(|effects| (catalogue, effects))
    });
    let (content, statics) = match &effects {
        Some(Ok((catalogue, effects))) => {
            let view = effects.view(None);
            (
                view.power_balances,
                view.static_modifiers.map(|statics| (*catalogue, statics)),
            )
        }
        Some(Err(reason)) => {
            reply.refuse("balance ranges", reason.clone(), 1);
            (None, None)
        }
        None => (None, None),
    };
    if catalogue.is_some() && content.is_none() && matches!(effects, Some(Ok(_))) {
        reply.refuse(
            "balance ranges",
            "the recorded content declares no balances of power",
            1,
        );
    }
    let breakdowns = frame
        .complete::<tables::PowerBalanceModifierBreakdowns>()
        .ok();
    let registry = replay_derive::modifier_registry::Registry::from_frame(frame);
    // The game's current balance needs the whole membership resolved.
    let mut unresolved = 0_u32;
    let mut current = BTreeMap::<u32, u32>::new();
    for member in members {
        match tags.slot(member.country) {
            Some(slot) => {
                current
                    .entry(slot)
                    .and_modify(|record| *record = (*record).min(member.record))
                    .or_insert(member.record);
            }
            None => unresolved += 1,
        }
    }
    if unresolved == 0 {
        reply.current = Some(
            current
                .into_iter()
                .filter_map(|(slot, record)| {
                    Some(CurrentBalance {
                        tag: tags.slot_tag(slot)?,
                        record,
                    })
                })
                .collect(),
        );
    } else {
        reply.refuse(
            "balance members",
            "a member has no identity at this hour, so no country's shown balance is known",
            unresolved,
        );
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
                        BalanceRange {
                            name: name(catalogue, &token, "balance range names", reply),
                            token,
                            min: Some(range.min.to_string()),
                            max: Some(range.max.to_string()),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if content.is_some() && ranges.is_empty() {
            reply.refuse(
                "balance ranges",
                format!("balance {balance} is not declared by the recorded content"),
                1,
            );
        }
        let active = token(&record.active_range);
        // The authored range the recorded id names, where it names exactly one;
        // otherwise its bounds stay unknown and the record's other facts stand.
        let authored = content.map(|database| database.range(&record.balance, &record.active_range));
        if let Some(Err(reason)) = &authored {
            reply.refuse("balance ranges", reason.clone(), 1);
        }
        let range = (!active.is_empty()).then(|| {
            let bounds = match authored {
                Some(Ok(Some(range))) => Some((range.min.to_string(), range.max.to_string())),
                Some(Ok(None)) => {
                    reply.refuse(
                        "balance range bounds",
                        format!("range {active} is not among its balance's authored ranges"),
                        1,
                    );
                    None
                }
                // Refused above when ambiguous; unknown without content.
                Some(Err(_)) | None => None,
            };
            BalanceRange {
                // Named as the authored list names it, so a refusal counts once.
                name: ranges
                    .iter()
                    .find(|range| range.token == active)
                    .and_then(|range| range.name.clone()),
                token: active.clone(),
                min: bounds.as_ref().map(|(min, _)| min.clone()),
                max: bounds.map(|(_, max)| max),
            }
        });
        let trending = (!record.trending.is_empty()
            && !installed_content_core::parser::c_string(&record.trending).is_empty())
        .then(|| side(catalogue, &record.trending, reply));
        let record_members = members
            .iter()
            .filter(|member| member.record == record.ordinal)
            .map(|member| {
                let row = engine_power_balance_member(member);
                BalanceMember {
                    tag: tags.tag(member.country),
                    daily: member
                        .daily_present
                        .then(|| row.daily_effective().to_string()),
                    weekly: member
                        .weekly_present
                        .then(|| row.weekly_effective().to_string()),
                }
            })
            .collect();
        let sources = match (&statics, &registry, breakdowns) {
            (Some((catalogue, statics)), Ok(registry), Some(breakdowns)) => {
                match breakdowns.iter().find(|row| row.record == record.ordinal) {
                    Some(row) => {
                        match drift_sources(catalogue, statics, registry, &row.sources) {
                            Ok(sources) => Some(sources),
                            Err(reason) => {
                                reply.refuse("balance drift sources", reason, 1);
                                None
                            }
                        }
                    }
                    None => {
                        reply.refuse(
                            "balance drift sources",
                            "the record's static-modifier roster is not recorded",
                            1,
                        );
                        None
                    }
                }
            }
            (_, Err(reason), _) => {
                reply.refuse("balance drift sources", reason.clone(), 1);
                None
            }
            (_, _, None) => {
                reply.refuse(
                    "balance drift sources",
                    "the static-modifier rosters are not recorded",
                    1,
                );
                None
            }
            (None, _, _) => None,
        };
        let balance_name = name(catalogue, &balance, "balance names", reply);
        let left = side(catalogue, &record.left, reply);
        let right = side(catalogue, &record.right, reply);
        reply.balances.push(BalanceRecord {
            ordinal: record.ordinal,
            name: balance_name,
            token: balance,
            value: record.value_raw.to_string(),
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

fn engine_power_balance_member(
    row: &replay_data::rows::PowerBalanceMemberRow,
) -> engine_power_balance::power_balance_record::MemberRow {
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

/// The record's drift sources as the game's breakdown lists them: one leaf
/// per nonempty source in roster order, its effective daily and weekly
/// values. A source the content stack does not declare, or an entry the
/// recorded registry cannot name, refuses the whole list by name.
fn drift_sources(
    catalogue: &Catalogue,
    statics: &installed_content_core::static_modifiers::Descriptions,
    registry: &replay_derive::modifier_registry::Registry<'_>,
    sources: &[Vec<u8>],
) -> Result<Vec<BalanceSource>, String> {
    let layers = &catalogue.content.layers;
    let mut blocks = Vec::with_capacity(sources.len());
    for source in sources {
        let key = std::str::from_utf8(installed_content_core::parser::c_string(source)).map_err(|_| {
            format!(
                "static modifier {} is not UTF-8",
                String::from_utf8_lossy(source)
            )
        })?;
        blocks.push(statics.get(layers, key)?.1);
    }
    let inputs: Vec<_> = sources
        .iter()
        .zip(&blocks)
        .map(|(source, block)| replay_derive::power_balance_derive::Source {
            name: source,
            auxiliary: !block.tooltip_table().is_empty() || !block.children.is_empty(),
            values: block
                .values
                .iter()
                .map(|value| (value.name.as_str(), value.value))
                .collect(),
        })
        .collect();
    let daily = replay_derive::power_balance_derive::tree(
        engine_executable::modifier_keys::POWER_BALANCE_DAILY,
        &inputs,
        registry,
    )?;
    let weekly = replay_derive::power_balance_derive::tree(
        engine_executable::modifier_keys::POWER_BALANCE_WEEKLY,
        &inputs,
        registry,
    )?;
    if daily.len() != weekly.len() {
        return Err("the daily and weekly source lists differ".into());
    }
    Ok(daily
        .into_iter()
        .zip(weekly)
        .skip(1)
        .map(|(daily, weekly)| {
            let token = token(&daily.name);
            BalanceSource {
                name: catalogue.localised(&token, View::Plain),
                token,
                daily: daily.value.to_string(),
                weekly: weekly.value.to_string(),
            }
        })
        .collect())
}
