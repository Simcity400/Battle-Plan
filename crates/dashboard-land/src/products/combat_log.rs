
use dashboard_query::identity::Holders;
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::Names};
use super::{divisions::Content, land_battles::BattleCountryFilter};
use dashboard_base::{Source, products::at_hour::{self, Read}};
use dashboard_query::{Query, log};
use replay_data::{
    codec::Decoder,
    rows::{COMBAT_LOG_VARIANT_TYPE, CombatLogObserverKind, LandEquipmentLoss, LandSideLosses, LandTerminalObserver, TableId, tables},
    value::Value as _,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub struct CombatLog;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CombatLogParams {
    pub branch_id: String,
    pub hour: i32,
    pub countries: BattleCountryFilter,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogGroup {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogManpower {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
    pub reason: i32,
    pub date: i32,
    pub losses: [i32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogEquipment {
    pub tag: String,
    pub group: String,
    pub month: u32,
    pub ordinal: Option<u32>,
    pub reason: i32,
    pub date: i32,
    pub kind: u8,
    pub allow_zero: bool,
    pub name: Option<String>,
    pub variant: Option<String>,
    pub amount_raw: Option<String>,
    pub missing_source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogTemplate {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
    pub date: i32,
    pub template: String,
    pub name: Option<String>,
    pub wins: i32,
    pub total: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogBattle {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
    pub id: i32,
    pub attacker: bool,
    pub date: i32,
    pub province: u32,
    pub flags: [u8; 4],
    pub manpower: [i32; 2],
    pub references: i32,
    pub details: Vec<LogDetailTable>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogDetailTable {
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LogActiveSide {
    pub combat: String,
    pub attacker: bool,
    pub terminal: bool,
    pub progress_raw: String,
    pub damage_raw: String,
    pub activity_groups: usize,
    pub leaders: usize,
    pub loss_source: Option<String>,
    pub details: Vec<LogDetailTable>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CombatLogAvailability {
    pub groups: Option<String>,
    pub manpower: Option<String>,
    pub months: Option<String>,
    pub own_equipment: Option<String>,
    pub enemy_equipment: Option<String>,
    pub templates: Option<String>,
    pub battles: Option<String>,
    pub active: Option<String>,
    pub terminal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CombatLogReply {
    pub hour: i32,
    pub groups: Vec<LogGroup>,
    pub manpower: Vec<LogManpower>,
    pub equipment: Vec<LogEquipment>,
    pub templates: Vec<LogTemplate>,
    pub battles: Vec<LogBattle>,
    pub active: Vec<LogActiveSide>,
    pub availability: CombatLogAvailability,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CombatLogReply);

const STATE: [TableId; 16] = [
    TableId::CombatLogGroups,
    TableId::CombatLogMonths,
    TableId::CombatLogManpower,
    TableId::CombatLogTemplates,
    TableId::CombatLogReferences,
    TableId::CombatLogBattles,
    TableId::CombatLogEnemyEquipment,
    TableId::CombatLogBattleSides,
    TableId::CombatLogBattleEquipment,
    TableId::CombatLogBattleCountries,
    TableId::CombatLogObservers,
    TableId::CombatLogEquipment,
    TableId::LandCombatProgress,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
];

const MODIFIERS: &str = "combat-log-battle-modifiers";
const RESULTS: &str = "land-combat-results";
const RESULT_OBSERVERS: &str = "land-combat-result-observers";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

fn unavailable(frame: &DecodedFrame, sources: &[TableId]) -> Option<String> {
    let missing: Vec<String> = sources
        .iter()
        .filter_map(|id| dashboard_text::scope::unread(*id, &frame.tables[id.index()]))
        .collect();
    (!missing.is_empty()).then(|| missing.join("; "))
}

pub fn matches_country(filter: &BattleCountryFilter, holders: &Holders, tag: &str, hour: i32) -> bool {
    filter.countries.as_ref().is_none_or(|countries| countries.iter().any(|chosen| holders.goes_by(chosen, tag, hour)))
        && (filter.membership.is_none()
            || !filter.unobserved.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour)
                && filter.membership.as_ref().is_some_and(|runs| runs.iter().any(|run| run.tag == tag && run.first_hour <= hour && hour <= run.last_hour)))
}

pub fn matches_side(filter: &BattleCountryFilter, holders: &Holders, tags: impl IntoIterator<Item = Option<String>>, hour: i32) -> bool {
    let mut empty = true;
    for tag in tags {
        empty = false;
        if tag.is_none_or(|tag| matches_country(filter, holders, &tag, hour)) {
            return true;
        }
    }
    empty
}

fn detail(tables: &mut BTreeMap<&'static str, LogDetailTable>, title: &'static str, columns: &[&str], row: Vec<String>) {
    tables
        .entry(title)
        .or_insert_with(|| LogDetailTable { title: title.into(), columns: columns.iter().map(|column| (*column).into()).collect(), rows: Vec::new() })
        .rows
        .push(row);
}

pub fn fixed(raw: i64) -> String {
    let value = raw.unsigned_abs();
    let fraction = format!("{:05}", value % 100_000);
    let fraction = fraction.trim_end_matches('0');
    format!("{}{}{}{}", if raw < 0 { "-" } else { "" }, value / 100_000, if fraction.is_empty() { "" } else { "." }, fraction)
}

fn pool(kind: u64) -> String {
    match kind {
        0 => "Lost".into(),
        1 => "Captured by enemy".into(),
        2 => "Recovered".into(),
        _ => format!("Pool {kind}"),
    }
}

fn side(attacker: bool) -> &'static str {
    if attacker { "Attacker" } else { "Defender" }
}

struct Labels<'a> {
    country: &'a mut dyn FnMut(u32) -> String,
    equipment: &'a dyn Fn(u64) -> String,
    template: &'a mut dyn FnMut(u64) -> String,
}

fn observer_designs(facts: &[(u8, u64, [u64; 5])], losses: &[LandEquipmentLoss], out: &mut BTreeSet<u64>) {
    use CombatLogObserverKind as K;
    for &(kind, _, [a, b, ..]) in facts {
        if kind == K::Equipment as u8 || kind == K::ActivityEquipment as u8 && a != u64::MAX {
            out.insert(b);
        }
    }
    out.extend(losses.iter().map(|loss| loss.variant));
}

fn observer(combat: u64, attacker: bool, terminal: bool, facts: &[(u8, u64, [u64; 5])], losses: Result<(i32, &[LandEquipmentLoss]), String>, labels: &mut Labels<'_>) -> io::Result<LogActiveSide> {
    use CombatLogObserverKind as K;
    let mut result = LogActiveSide {
        combat: combat.to_string(),
        attacker,
        terminal,
        progress_raw: "0".into(),
        damage_raw: "0".into(),
        activity_groups: 0,
        leaders: 0,
        loss_source: None,
        details: Vec::new(),
    };
    let mut tables = BTreeMap::new();
    let (mut state, mut participants) = (false, false);
    for &(kind, item, [a, b, c, d, e]) in facts {
        let ordinal = item.to_string();
        match kind {
            k if k == K::State as u8 => {
                state = true;
                result.damage_raw = (a as i64).to_string();
                result.progress_raw = (b as i64).to_string();
                detail(&mut tables, "Observer state", &["Damage", "Progress", "Flags", "Snow", "Win"], vec![fixed(a as i64), fixed(b as i64), c.to_string(), (c & 1 != 0).to_string(), (c & 2 != 0).to_string()]);
            }
            k if k == K::Side as u8 => detail(&mut tables, "Side", &["Air factor", "Leader identity"], vec![fixed(b as i64), c.to_string()]),
            k if k == K::Pool as u8 => detail(&mut tables, "Equipment pool membership", &["Pool", "Present in save", "Allow zero entries"], vec![pool(item), (a != 0).to_string(), (b != 0).to_string()]),
            k if k == K::Equipment as u8 => detail(&mut tables, "Side equipment", &["Pool", "Order", "Design / identity", "Amount"], vec![pool(item), a.to_string(), (labels.equipment)(b), fixed(c as i64)]),
            k if k == K::Modifier as u8 => detail(&mut tables, "Modifier hours", &["Order", "Modifier index", "Hours"], vec![a.to_string(), (b as i32).to_string(), (c as i32).to_string()]),
            k if k == K::Country as u8 => {
                participants = true;
                detail(&mut tables, "Countries", &["Order", "Country / identity"], vec![a.to_string(), (labels.country)(b as u32)]);
            }
            k if k == K::Counters as u8 => {
                for (at, value) in [a, b, c, d, e].into_iter().enumerate() {
                    detail(&mut tables, "Stored counters", &["Index", "Hours"], vec![(item + at as u64).to_string(), (value as i32).to_string()]);
                }
            }
            k if k == K::Leader as u8 => {
                result.leaders += 1;
                detail(&mut tables, "Leader hours", &["Order", "Leader identity", "Hours"], vec![ordinal, a.to_string(), (b as i32).to_string()]);
            }
            k if k == K::Damage as u8 => detail(
                &mut tables,
                "Damage attribution",
                &["Order", "From", "Receiver", "To", "Damage"],
                vec![ordinal, (labels.country)(a as u32), (labels.country)(b as u32), (labels.country)(c as u32), fixed(d as i64)],
            ),
            k if k == K::Activity as u8 => {
                result.activity_groups += 1;
                detail(&mut tables, "Activity groups", &["Order", "Group identity", "Dealer", "Taker"], vec![ordinal, a.to_string(), (labels.country)(b as u32), (labels.country)(c as u32)]);
            }
            k if k == K::ActivityTemplate as u8 => detail(&mut tables, "Activity templates", &["Activity", "Order", "Template / identity"], vec![ordinal, a.to_string(), (labels.template)(b)]),
            k if k == K::ActivityDamage as u8 => detail(&mut tables, "Activity division damage", &["Activity", "Order", "Division identity", "Strength damage"], vec![ordinal, a.to_string(), b.to_string(), fixed(c as i64)]),
            k if k == K::ActivityEquipment as u8 => {
                if a == u64::MAX {
                    detail(&mut tables, "Activity equipment membership", &["Activity", "Present in save", "Allow zero entries"], vec![ordinal, (b != 0).to_string(), (c != 0).to_string()]);
                } else {
                    detail(&mut tables, "Activity damaged equipment", &["Activity", "Order", "Design / identity", "Amount"], vec![ordinal, a.to_string(), (labels.equipment)(b), fixed(c as i64)]);
                }
            }
            _ => return Err(io::Error::other(format!("Unsupported Combat Log observer kind {kind}"))),
        }
    }
    if !state {
        return Err(io::Error::other("Combat Log observer state missing"));
    }
    if !participants {
        detail(&mut tables, "Countries", &["Order", "Country / identity"], vec![String::new(), "Unknown participants".into()]);
    }
    match losses {
        Ok((manpower, entries)) => {
            detail(&mut tables, "Aggregate losses", &["Manpower"], vec![manpower.to_string()]);
            for (ordinal, loss) in entries.iter().enumerate() {
                detail(&mut tables, "Aggregate equipment losses", &["Order", "Design / identity", "Amount"], vec![ordinal.to_string(), (labels.equipment)(loss.variant), fixed(loss.amount_raw)]);
            }
        }
        Err(reason) => result.loss_source = Some(reason),
    }
    result.details = tables.into_values().collect();
    Ok(result)
}

fn logged(query: &Query<'_>, name: &str, filters: &[log::FacetFilter], from: i32, to: i32) -> io::Result<Vec<log::Row>> {
    let declared = dashboard_declare::log(name).ok_or_else(|| invalid(format!("No log is declared as {name}")))?;
    let reader = log::Reader::new(query, declared)?;
    let selection = reader.select(filters, from, to)?;
    let page = reader.page(&selection.patterns, selection.groups.len(), from, to, 0, u32::MAX)?;
    Ok(page.rows.into_iter().map(|paged| paged.row).collect())
}

#[derive(Debug, PartialEq, Eq)]
pub struct Finalized {
    pub combat: u64,
    pub observers: Vec<LandTerminalObserver>,
    pub losses: [LandSideLosses; 2],
}

pub fn finalized(row: &log::Row, columns: &[usize; 3]) -> Option<Finalized> {
    let list = |at: usize| match row.cells.get(columns[at]) {
        Some(log::Cell::List(bytes)) => Some(&bytes[..]),
        _ => None,
    };
    let combat = match row.cells.get(columns[0]) {
        Some(log::Cell::Number(value)) => *value as u64,
        _ => return None,
    };
    Some(Finalized {
        combat,
        observers: Vec::<LandTerminalObserver>::decode(&mut Decoder::new(list(1)?)).ok()?,
        losses: <[LandSideLosses; 2]>::decode(&mut Decoder::new(list(2)?)).ok()?,
    })
}

pub fn finalized_split(row: &log::Row, columns: &[usize; 2], observers: Vec<LandTerminalObserver>) -> Option<Finalized> {
    let combat = match row.cells.get(columns[0]) {
        Some(log::Cell::Number(value)) => *value as u64,
        _ => return None,
    };
    let losses = match row.cells.get(columns[1]) {
        Some(log::Cell::List(bytes)) => <[LandSideLosses; 2]>::decode(&mut Decoder::new(bytes)).ok()?,
        _ => return None,
    };
    Some(Finalized { combat, observers, losses })
}

pub fn result_observers(rows: &[log::Row], columns: &[usize; 11]) -> (BTreeMap<u64, Vec<LandTerminalObserver>>, BTreeSet<u64>) {
    let mut grouped: BTreeMap<u64, Vec<(u64, LandTerminalObserver)>> = BTreeMap::new();
    let mut incomplete = BTreeSet::new();
    for row in rows {
        let combat = (row.key >> 64) as u64;
        let ordinal = (row.key as u64) >> 1;
        let attacker = row.key & 1 != 0;
        let number = |at: usize| match row.cells.get(columns[at]) {
            Some(log::Cell::Number(value)) => Some(*value),
            _ => None,
        };
        let (
            Some(kind),
            Some(item),
            Some(index),
            Some(identity),
            Some(identity_2),
            Some(identity_3),
            Some(a),
            Some(b),
            Some(c),
            Some(d),
            Some(e),
        ) = (
            number(0),
            number(1),
            number(2),
            number(3),
            number(4),
            number(5),
            number(6),
            number(7),
            number(8),
            number(9),
            number(10),
        )
        else {
            incomplete.insert(combat);
            continue;
        };
        let obs = replay_data::rows::CombatLogObserverRow {
            combat,
            attacker,
            ordinal,
            kind: kind as u8,
            item: item as u64,
            index: index as u64,
            identity: identity as u64,
            identity_2: identity_2 as u64,
            identity_3: identity_3 as u64,
            a,
            b,
            c,
            d,
            e,
        };
        let (kind, item, values) = obs.fact();
        grouped.entry(combat).or_default().push((ordinal, LandTerminalObserver { attacker, kind, item, values }));
    }
    let out = grouped
        .into_iter()
        .filter(|(combat, _)| !incomplete.contains(combat))
        .map(|(combat, mut entries)| {
            entries.sort_unstable_by_key(|(ordinal, _)| *ordinal);
            (combat, entries.into_iter().map(|(_, entry)| entry).collect())
        })
        .collect();
    (out, incomplete)
}

pub fn modifiers_of(rows: &[log::Row], battles: &BTreeMap<u32, i32>, columns: &[usize; 2]) -> BTreeMap<u32, Vec<(u8, u32, i64, i64)>> {
    let mut newest: BTreeMap<u128, &log::Row> = BTreeMap::new();
    for row in rows {
        let battle = (row.key >> 40) as u32;
        if battles.get(&battle).is_some_and(|&date| row.hour >= date) {
            newest.insert(row.key, row);
        }
    }
    let mut out: BTreeMap<u32, Vec<(u8, u32, i64, i64)>> = BTreeMap::new();
    for (key, row) in newest {
        let number = |at: usize| match row.cells.get(columns[at]) {
            Some(log::Cell::Number(value)) => *value,
            _ => 0,
        };
        out.entry((key >> 40) as u32).or_default().push(((key >> 32) as u8, key as u32, number(0), number(1)));
    }
    out
}

impl Product for CombatLog {
    const KEY: &'static str = "combat-log-history";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = CombatLogParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, "countries.membership[].firstHour": Start, "countries.membership[].lastHour": End, "countries.unobserved[].firstHour": Start, "countries.unobserved[].lastHour": End, "countries.countries[]"];
    type Reply = CombatLogReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CombatLogParams::decl(),
            CombatLogReply::decl(),
            LogGroup::decl(),
            LogManpower::decl(),
            LogEquipment::decl(),
            LogTemplate::decl(),
            LogBattle::decl(),
            LogDetailTable::decl(),
            LogActiveSide::decl(),
            CombatLogAvailability::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)?;
        for run in params.countries.membership.iter().flatten() {
            dashboard_base::products::validate_tag(&run.tag)?;
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = params.hour;
        let run = query.timeline.at(hour).ok_or_else(|| dashboard_core::refused("This time is outside the recording"))?.run;
        let recording = query.replay.recording();
        let reads: Vec<Read> = STATE.into_iter().map(Read::all).collect();
        let frame = at_hour::frame(&query, hour, &reads, "Combat Log")?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        let names = Names::new(&query.holders);
        let links = dashboard_base::queries::links(source, run)?;
        let content = Content::load(source, context);
        let filter = &params.countries;
        let every = filter.countries.is_none() && filter.membership.is_none();
        let tag_of = |country: u32| names.tag_at(&links, country, hour).map(str::to_owned);
        let owner = |country: u32| tag_of(country).map_or(every, |tag| matches_country(filter, &query.holders, &tag, hour));
        let tag = |country: u32| tag_of(country).unwrap_or_else(|| format!("Country {country}"));
        let mut unresolved = BTreeSet::new();
        let mut country = |id: u32| {
            if tag_of(id).is_none() {
                unresolved.insert(id);
            }
            format!("{} ({id})", tag(id))
        };
        macro_rules! rows {
            ($table:ident) => {
                frame.complete::<tables::$table>().unwrap_or(&[])
            };
        }
        let available = |table: TableId| -> io::Result<Option<String>> {
            Ok(replay_lanes::lanes::available_reads(recording, run, table, hour, hour)?.is_empty().then(|| format!("{}: not recorded at this hour", table.words())))
        };
        let split_observers = recording.predates(TableId::LandCombatResultObservers.index()).is_none();
        let mut availability = CombatLogAvailability {
            groups: unavailable(&frame, &[TableId::CombatLogGroups]),
            manpower: unavailable(&frame, &[TableId::CombatLogManpower]),
            months: unavailable(&frame, &[TableId::CombatLogMonths]),
            own_equipment: unavailable(&frame, &[TableId::CombatLogEquipment]),
            enemy_equipment: unavailable(&frame, &[TableId::CombatLogEnemyEquipment]),
            templates: unavailable(&frame, &[TableId::CombatLogTemplates]),
            battles: unavailable(&frame, &[TableId::CombatLogReferences, TableId::CombatLogBattles, TableId::CombatLogBattleSides, TableId::CombatLogBattleEquipment, TableId::CombatLogBattleCountries]),
            active: unavailable(&frame, &[TableId::CombatLogObservers]),
            terminal: available(TableId::LandCombatResults)?,
        };
        if split_observers && availability.terminal.is_none() {
            availability.terminal = available(TableId::LandCombatResultObservers)?;
        }
        if availability.battles.is_none() {
            availability.battles = available(TableId::CombatLogBattleModifiers)?;
        }

        let battles: BTreeMap<u32, &replay_data::rows::CombatLogBattleRow> = rows!(CombatLogBattles).iter().map(|row| (row.id, row)).collect();
        let referenced: BTreeMap<u32, i32> = rows!(CombatLogReferences)
            .iter()
            .filter(|row| owner(row.country))
            .filter_map(|row| battles.get(&(row.battle as u32)).map(|battle| (battle.id, battle.date)))
            .collect();
        let modifiers = if availability.battles.is_none() && !referenced.is_empty() {
            let columns = [dashboard_declare::field(TableId::CombatLogBattleModifiers, "modifier")?, dashboard_declare::field(TableId::CombatLogBattleModifiers, "hours")?];
            let from = referenced.values().copied().min().unwrap_or(hour).max(query.timeline.first);
            let filters = [log::FacetFilter { name: "battle".into(), values: referenced.keys().map(|&id| i64::from(id)).collect(), ..Default::default() }];
            modifiers_of(&logged(&query, MODIFIERS, &filters, from, hour)?, &referenced, &columns)
        } else {
            BTreeMap::new()
        };
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut ended = Vec::new();
        let mut warnings = Vec::new();
        if availability.terminal.is_none() {
            if split_observers {
                let result_rows = logged(&query, RESULTS, &[], hour, hour)?;
                if !result_rows.is_empty() {
                    let result_columns = [
                        dashboard_declare::field(TableId::LandCombatResults, "combat")?,
                        dashboard_declare::field(TableId::LandCombatResults, "losses")?,
                    ];
                    let observer_columns = [
                        dashboard_declare::field(TableId::LandCombatResultObservers, "kind")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "item")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "index")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "identity")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "identity_2")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "identity_3")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "a")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "b")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "c")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "d")?,
                        dashboard_declare::field(TableId::LandCombatResultObservers, "e")?,
                    ];
                    let observer_rows = logged(&query, RESULT_OBSERVERS, &[], hour, hour)?;
                    let (mut by_combat, incomplete) = result_observers(&observer_rows, &observer_columns);
                    for row in result_rows {
                        let combat = match row.cells.get(result_columns[0]) {
                            Some(log::Cell::Number(value)) => Some(*value as u64),
                            _ => None,
                        };
                        let observers = combat.filter(|combat| !incomplete.contains(combat)).and_then(|combat| by_combat.remove(&combat));
                        match observers.and_then(|observers| finalized_split(&row, &result_columns, observers)) {
                            Some(result) => ended.push(result),
                            None => warnings.push("A finalized combat's observers are not recorded in full; it is not shown.".to_owned()),
                        }
                    }
                }
            } else {
                let columns = [
                    dashboard_declare::field(TableId::LandCombatResults, "combat")?,
                    dashboard_declare::field(TableId::LandCombatResults, "observers")?,
                    dashboard_declare::field(TableId::LandCombatResults, "losses")?,
                ];
                for row in logged(&query, RESULTS, &[], hour, hour)? {
                    match finalized(&row, &columns) {
                        Some(result) => ended.push(result),
                        None => warnings.push("A finalized combat's observers are not recorded in full; it is not shown.".to_owned()),
                    }
                }
            }
        }

        let mut designs = BTreeSet::new();
        designs.extend(rows!(CombatLogEnemyEquipment).iter().map(|row| row.variant));
        designs.extend(rows!(CombatLogEquipment).iter().map(|row| u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE)));
        designs.extend(rows!(CombatLogBattleEquipment).iter().map(|row| row.variant));
        let observers: Vec<&replay_data::rows::CombatLogObserverRow> = rows!(CombatLogObservers).iter().collect();
        let facts: Vec<(u8, u64, [u64; 5])> = observers.iter().map(|row| row.fact()).collect();
        observer_designs(&facts, &[], &mut designs);
        if let Ok(progress) = frame.complete::<tables::LandCombatProgress>() {
            for row in progress {
                for pool in [&row.attacker_equipment_losses, &row.defender_equipment_losses] {
                    if let replay_data::rows::Recorded::Value(pool) = pool {
                        designs.extend(pool.iter().map(|loss| loss.variant));
                    }
                }
            }
        }
        for result in &ended {
            let facts: Vec<(u8, u64, [u64; 5])> = result.observers.iter().map(|row| (row.kind, row.item, row.values)).collect();
            observer_designs(&facts, &result.losses[0].equipment, &mut designs);
            observer_designs(&[], &result.losses[1].equipment, &mut designs);
        }
        let designs: Vec<u64> = designs.into_iter().collect();
        let (named, design_warnings) = super::land_battles::design_names(&query, &content, run, hour, &designs)?;
        warnings.extend(design_warnings);
        let design_names: HashMap<u64, Option<String>> = designs.iter().copied().zip(named.into_iter().map(|design| design.name)).collect();
        let name_of = |variant: u64| design_names.get(&variant).cloned().flatten();
        let equipment = |variant: u64| name_of(variant).map_or_else(|| format!("Name unavailable ({variant})"), |name| format!("{name} ({variant})"));
        dashboard_text::metadata::check_cancel(cancelled)?;

        let mut reply = CombatLogReply {
            hour,
            groups: Vec::new(),
            manpower: Vec::new(),
            equipment: Vec::new(),
            templates: Vec::new(),
            battles: Vec::new(),
            active: Vec::new(),
            availability,
            warnings: Vec::new(),
        };
        for row in rows!(CombatLogGroups).iter().filter(|row| owner(row.country)) {
            reply.groups.push(LogGroup { tag: country(row.country), group: row.group.to_string(), ordinal: row.ordinal });
        }
        for row in rows!(CombatLogManpower).iter().filter(|row| owner(row.country)) {
            reply.manpower.push(LogManpower { tag: tag(row.country), group: row.group.to_string(), ordinal: row.ordinal, reason: row.reason, date: row.date, losses: [row.losses_0, row.losses_1, row.losses_2] });
        }
        let mut enemy_equipment = BTreeMap::<_, Vec<_>>::new();
        for row in rows!(CombatLogEnemyEquipment) {
            enemy_equipment.entry((row.country, row.group, row.month)).or_default().push(row);
        }
        let mut own_equipment = BTreeMap::<_, Vec<_>>::new();
        for row in rows!(CombatLogEquipment) {
            own_equipment.entry((row.country, row.group, row.kind, row.date)).or_default().push(row);
        }
        for month in rows!(CombatLogMonths).iter().filter(|row| owner(row.country)) {
            let mut pushed = 0usize;
            let missing_source = if month.kind == 5 { reply.availability.enemy_equipment.clone() } else { reply.availability.own_equipment.clone() };
            let mut add = |ordinal: Option<u32>, variant: Option<u64>, amount: Option<i64>| {
                reply.equipment.push(LogEquipment {
                    tag: tag(month.country),
                    group: month.group.to_string(),
                    month: month.ordinal,
                    ordinal,
                    reason: month.reason,
                    date: month.date,
                    kind: month.kind,
                    allow_zero: month.allow_zero,
                    name: variant.and_then(name_of),
                    variant: variant.map(|variant| variant.to_string()),
                    amount_raw: amount.map(|amount| amount.to_string()),
                    missing_source: missing_source.clone(),
                })
            };
            if month.kind == 5 {
                for row in enemy_equipment.get(&(month.country, month.group, month.ordinal)).into_iter().flatten() {
                    add(Some(row.ordinal), Some(row.variant), Some(row.amount_raw));
                    pushed += 1;
                }
            } else {
                for (ordinal, row) in own_equipment.get(&(month.country, month.group, month.kind, month.date)).into_iter().flatten().enumerate() {
                    add(Some(ordinal as u32), Some(u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE)), Some(row.amount_raw));
                    pushed += 1;
                }
            }
            if pushed == 0 {
                add(None, None, None);
            }
        }
        let mut template_names: BTreeMap<u64, Option<String>> = BTreeMap::new();
        let mut missing_templates = BTreeSet::new();
        let mut template_name = |id: u64| -> Option<String> {
            let name = template_names.entry(id).or_insert_with(|| content.template_name(&frame, id).ok()).clone();
            if name.is_none() {
                missing_templates.insert(id);
            }
            name
        };
        for row in rows!(CombatLogTemplates).iter().filter(|row| owner(row.country)) {
            reply.templates.push(LogTemplate {
                tag: tag(row.country),
                group: row.group.to_string(),
                ordinal: row.ordinal,
                date: row.date,
                template: row.template.to_string(),
                name: template_name(row.template),
                wins: row.wins,
                total: row.total,
            });
        }
        if reply.availability.battles.is_none() {
            let mut sides = BTreeMap::<_, Vec<_>>::new();
            for row in rows!(CombatLogBattleSides) {
                sides.entry(row.battle).or_default().push(row);
            }
            let mut battle_equipment = BTreeMap::<_, Vec<_>>::new();
            for row in rows!(CombatLogBattleEquipment) {
                battle_equipment.entry(row.battle).or_default().push(row);
            }
            let mut countries = BTreeMap::<_, Vec<_>>::new();
            for row in rows!(CombatLogBattleCountries) {
                countries.entry(row.battle).or_default().push(row);
            }
            for reference in rows!(CombatLogReferences).iter().filter(|row| owner(row.country)) {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let battle = *battles.get(&(reference.battle as u32)).ok_or_else(|| io::Error::other("combat log references: retained record missing"))?;
                let mut details = BTreeMap::new();
                let flags = battle.flags;
                detail(
                    &mut details,
                    "Flags",
                    &["Defender won", "Snow", "Overrun", "Player is attacker", "Stored bytes"],
                    vec![(flags[0] != 0).to_string(), (flags[1] != 0).to_string(), (flags[2] != 0).to_string(), (flags[3] != 0).to_string(), format!("{flags:?}")],
                );
                let mut manpower = [0; 2];
                for row in sides.get(&battle.id).into_iter().flatten() {
                    if row.side > 1 {
                        return Err(io::Error::other("combat log battle sides: invalid side"));
                    }
                    manpower[row.side as usize] = row.manpower;
                    detail(
                        &mut details,
                        "Sides",
                        &["Side", "Manpower lost", "Air factor", "Leader identity", "Pool flags"],
                        vec![side(row.side == 0).into(), row.manpower.to_string(), fixed(row.air_factor_raw), row.leader.to_string(), row.pool_flags.to_string()],
                    );
                    for kind in 0..3 {
                        let flags = (row.pool_flags >> (kind * 2)) & 3;
                        detail(
                            &mut details,
                            "Equipment pool membership",
                            &["Side", "Pool", "Present in save", "Allow zero entries"],
                            vec![side(row.side == 0).into(), pool(kind as u64), (flags & 1 != 0).to_string(), (flags & 2 != 0).to_string()],
                        );
                    }
                }
                for row in battle_equipment.get(&battle.id).into_iter().flatten() {
                    detail(
                        &mut details,
                        "Equipment",
                        &["Side", "Pool", "Order", "Design / identity", "Amount"],
                        vec![side(row.side == 0).into(), pool(row.kind as u64), row.ordinal.to_string(), equipment(row.variant), fixed(row.amount_raw)],
                    );
                }
                for &(row_side, ordinal, modifier, hours) in modifiers.get(&battle.id).into_iter().flatten() {
                    detail(&mut details, "Modifier hours", &["Side", "Order", "Modifier index", "Hours"], vec![side(row_side == 0).into(), ordinal.to_string(), (modifier as i32).to_string(), (hours as i32).to_string()]);
                }
                for row in countries.get(&battle.id).into_iter().flatten() {
                    detail(&mut details, "Countries", &["Side", "Order", "Country / identity"], vec![side(row.side == 0).into(), row.ordinal.to_string(), country(row.country)]);
                }
                reply.battles.push(LogBattle {
                    tag: tag(reference.country),
                    group: reference.group.to_string(),
                    ordinal: reference.ordinal,
                    id: reference.battle,
                    attacker: reference.attacker,
                    date: battle.date,
                    province: battle.province,
                    flags,
                    manpower,
                    references: battle.reference_count,
                    details: details.into_values().collect(),
                });
            }
        }
        let mut active: BTreeMap<(u64, bool), Vec<&replay_data::rows::CombatLogObserverRow>> = BTreeMap::new();
        for &row in &observers {
            active.entry((row.combat, row.attacker)).or_default().push(row);
        }
        let progress = frame.complete::<tables::LandCombatProgress>().map_err(|why| format!("land combat progress: {why}")).map(|rows| {
            let mut by_combat = BTreeMap::new();
            for row in rows {
                by_combat.entry(row.combat).or_insert(row);
            }
            by_combat
        });
        let mut template_label = |id: u64| template_name(id).map_or_else(|| format!("Name unavailable ({id})"), |name| format!("{name} ({id})"));
        let side_tags = |facts: &[(u8, u64, [u64; 5])]| -> Vec<Option<String>> {
            facts.iter().filter(|fact| fact.0 == CombatLogObserverKind::Country as u8).map(|fact| tag_of(fact.2[1] as u32)).collect()
        };
        for ((combat, attacker), mut operands) in active {
            dashboard_text::metadata::check_cancel(cancelled)?;
            operands.sort_unstable_by_key(|row| row.logical_order());
            let facts: Vec<(u8, u64, [u64; 5])> = operands.iter().map(|row| row.fact()).collect();
            let countries: Vec<Option<String>> = operands.iter().filter(|row| row.kind == CombatLogObserverKind::Country as u8).map(|row| tag_of(row.identity as u32)).collect();
            if !matches_side(filter, &query.holders, countries, hour) {
                continue;
            }
            let losses = progress.as_ref().map_err(Clone::clone).and_then(|rows| {
                let progress = rows.get(&combat).copied().ok_or_else(|| "land combat progress: observer combat missing".to_string())?;
                let (manpower, equipment) = if attacker {
                    (&progress.attacker_manpower_loss, &progress.attacker_equipment_losses)
                } else {
                    (&progress.defender_manpower_loss, &progress.defender_equipment_losses)
                };
                Ok((
                    *manpower.require("manpower").map_err(|why| format!("land combat progress: {why}"))?,
                    equipment.require("equipment").map_err(|why| format!("land combat progress: {why}"))?.as_slice(),
                ))
            });
            reply.active.push(observer(combat, attacker, false, &facts, losses, &mut Labels { country: &mut country, equipment: &equipment, template: &mut template_label })?);
        }
        for result in &ended {
            for attacker in [true, false] {
                let facts: Vec<(u8, u64, [u64; 5])> = result.observers.iter().filter(|row| row.attacker == attacker).map(|row| (row.kind, row.item, row.values)).collect();
                if facts.is_empty() || !matches_side(filter, &query.holders, side_tags(&facts), hour) {
                    continue;
                }
                let loss = &result.losses[usize::from(!attacker)];
                reply.active.push(observer(result.combat, attacker, true, &facts, Ok((loss.manpower, &loss.equipment)), &mut Labels { country: &mut country, equipment: &equipment, template: &mut template_label })?);
            }
        }
        if !unresolved.is_empty() {
            warnings.push(format!("{} Combat Log country references have no recorded country at this hour: {:?}", unresolved.len(), unresolved));
        }
        if !missing_templates.is_empty() {
            warnings.push(format!("{} retained or activity templates have no recorded name at this hour", missing_templates.len()));
        }
        reply.warnings = warnings;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
