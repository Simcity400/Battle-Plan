
use dashboard_base::products::validate_country;
use dashboard_base::products::Country;
use super::{
    Content, DECIMALS, Domain, NEED_BOUND_RAW, StockType, StockVariantName, Types, UNRESOLVED_ARM, UNRESOLVED_MARK,
    history::{self, Logistics, Replay, Stock},
};
use dashboard_base::{Source, span::HourSpan};
use dashboard_base::products::{LoadContext, Product};
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun, ChartSummary, merge},
};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileHolder {
    pub country: Country,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub within: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileCatalogueParams {
    pub branch_id: String,
    pub holders: Vec<StockpileHolder>,
    #[doc = " The types a group counts; empty counts every type."]
    #[serde(default)]
    pub included: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StockpileMeasure {
    Stock,
    Share,
    Need,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileLine {
    #[doc = " The group's key: `[kind, key]` as JSON, a design's key its handle."]
    pub group: String,
    pub measure: StockpileMeasure,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileChartParams {
    pub branch_id: String,
    pub holders: Vec<StockpileHolder>,
    #[serde(default)]
    pub included: Vec<String>,
    pub lines: Vec<StockpileLine>,
    pub from: f64,
    pub to: f64,
    pub width: u32,
}

fn validate(branch_id: &str, holders: &[StockpileHolder]) -> Result<(), &'static str> {
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    if holders.is_empty() {
        return Err("Invalid stockpile selection");
    }
    let mut seen = BTreeSet::new();
    for holder in holders {
        validate_country(&holder.country)?;
        if !seen.insert(&holder.country) {
            return Err("Invalid stockpile selection");
        }
        if let Some(spans) = &holder.within
            && (spans.iter().any(|span| span.first_hour > span.last_hour) || spans.windows(2).any(|pair| pair[0].last_hour >= pair[1].first_hour))
        {
            return Err("Invalid stockpile membership");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileChoicePeak {
    pub raw: String,
    pub stock_drawable: bool,
    pub share_drawable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileCatalogueVariant {
    #[doc = " Its native handle, as a decimal string."]
    pub id: String,
    #[doc = " Its equipment token; empty where no read spelled it."]
    pub token: String,
    #[doc = " Its type's archetype key; null where the content does not resolve its type."]
    pub archetype: Option<String>,
    pub names: Vec<StockVariantName>,
    #[doc = " Every design recorded under its handle."]
    pub designs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileCatalogueLogistics {
    #[doc = " The archetype key, or the unresolved mark and the recorded token's spelling."]
    pub key: String,
    pub nonzero: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileCatalogueReply {
    pub branch_id: String,
    pub countries: Vec<Country>,
    pub campaign_key: String,
    pub decimals: u8,
    #[doc = " Every archetype a design or logistics type belongs to, in the equipment database's order."]
    pub types: Vec<StockType>,
    pub variants: Vec<StockpileCatalogueVariant>,
    pub logistics: Vec<StockpileCatalogueLogistics>,
    #[doc = " The hours the stockpile, the production lines and the logistics record were read for the"]
    #[doc = " holders within their hours, ascending."]
    pub reads: Vec<i32>,
    pub line_reads: Vec<i32>,
    pub logistics_reads: Vec<i32>,
    pub stock_hours: Vec<HourSpan>,
    pub line_hours: Vec<HourSpan>,
    pub balance_hours: Vec<HourSpan>,
    #[doc = " Hours the holder was AI-controlled, so the game computed no balance."]
    pub ai_hours: Vec<HourSpan>,
    pub need_bound_raw: String,
    #[doc = " Each group's peak and drawability, by its key (`[kind, key]` as JSON)."]
    pub choices: BTreeMap<String, StockpileChoicePeak>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileChartLine {
    pub group: String,
    pub measure: StockpileMeasure,
    pub summary: ChartSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileChartReply {
    pub branch_id: String,
    pub lines: Vec<StockpileChartLine>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StockpileCatalogueReply, StockpileChartReply);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Type,
    Domain,
    Variant,
}

pub(super) struct Group {
    pub wire: String,
    pub kind: Kind,
    pub key: String,
    pub variants: Vec<u64>,
    pub types: Vec<String>,
}

impl Group {
    fn resolved_arm(&self) -> bool {
        self.kind == Kind::Domain && self.key != UNRESOLVED_ARM
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Piece<T> {
    pub first: i32,
    pub last: i32,
    pub value: Option<T>,
    pub starts: bool,
}

fn push<T: PartialEq + Copy>(pieces: &mut Vec<Piece<T>>, (first, last): (i32, i32), value: Option<T>, starts: bool) {
    if let Some(previous) = pieces.last_mut()
        && !starts
        && previous.value == value
        && i64::from(previous.last) + 1 == i64::from(first)
    {
        previous.last = last;
        return;
    }
    pieces.push(Piece { first, last, value, starts });
}

pub(super) type Balance = (i128, Option<i128>);

type TypeTokens = BTreeMap<String, (Vec<u32>, bool)>;

#[derive(Default)]
pub(super) struct Lines {
    pub stock: Vec<Piece<i128>>,
    pub logistics: Vec<Piece<Balance>>,
}

pub(super) struct Member {
    pub token: String,
    pub archetype: Option<String>,
    pub domain: Option<Domain>,
    pub names: Vec<StockVariantName>,
}

pub(super) struct Model {
    replays: Vec<Arc<Replay>>,
    variants: BTreeMap<u64, Member>,
    logistics: TypeTokens,
    types: Vec<StockType>,
    groups: Vec<Group>,
    lines: Vec<Vec<Lines>>,
    first: i32,
    last: i32,
    warnings: Vec<String>,
}

impl Model {
    fn group(&self, wire: &str) -> Option<usize> {
        self.groups.iter().position(|group| group.wire == wire)
    }
}

fn domain_key(domain: Option<Domain>) -> &'static str {
    match domain {
        Some(Domain::Land) => "land",
        Some(Domain::Air) => "air",
        Some(Domain::Naval) => "naval",
        None => UNRESOLVED_ARM,
    }
}

fn wire(kind: Kind, key: &str) -> String {
    let kind = match kind {
        Kind::Type => "type",
        Kind::Domain => "domain",
        Kind::Variant => "variant",
    };
    serde_json::to_string(&(kind, key)).expect("a group key serializes")
}

pub(super) fn groups(variants: &BTreeMap<u64, Member>, logistics: &TypeTokens, domains: &BTreeMap<String, Domain>, included: &[String]) -> Vec<Group> {
    let shown = |key: &str| included.is_empty() || included.iter().any(|item| item == key);
    let type_key = |id: u64, member: &Member| {
        member.archetype.clone().unwrap_or_else(|| {
            let token = if member.token.is_empty() { id.to_string() } else { member.token.clone() };
            format!("{UNRESOLVED_MARK}{token}")
        })
    };
    fn group(grouped: &mut BTreeMap<String, Group>, kind: Kind, key: String) -> &mut Group {
        grouped.entry(key.clone()).or_insert_with(|| Group { wire: wire(kind, &key), kind, key, variants: Vec::new(), types: Vec::new() })
    }
    let mut result = Vec::new();
    for kind in [Kind::Type, Kind::Domain] {
        let mut grouped = BTreeMap::<String, Group>::new();
        for (&id, member) in variants {
            let key = type_key(id, member);
            if !shown(&key) {
                continue;
            }
            let key = match kind {
                Kind::Type => key,
                _ => domain_key(member.domain).to_owned(),
            };
            group(&mut grouped, kind, key).variants.push(id);
        }
        let listed: Vec<_> = logistics
            .iter()
            .filter(|(key, _)| shown(key))
            .map(|(key, &(_, nonzero))| {
                let row = match kind {
                    Kind::Type => key.clone(),
                    _ => domain_key(domains.get(key).copied()).to_owned(),
                };
                (key, row, nonzero)
            })
            .collect();
        for (_, row, nonzero) in &listed {
            if *nonzero {
                group(&mut grouped, kind, row.clone());
            }
        }
        for (key, row, _) in listed {
            if let Some(group) = grouped.get_mut(&row)
                && !group.types.contains(key)
            {
                group.types.push(key.clone());
            }
        }
        result.extend(grouped.into_values());
    }
    for (&id, member) in variants {
        if shown(&type_key(id, member)) {
            let key = id.to_string();
            result.push(Group { wire: wire(Kind::Variant, &key), kind: Kind::Variant, key, variants: vec![id], types: Vec::new() });
        }
    }
    result
}

struct Cursor<'a, T> {
    changes: &'a [(i32, T)],
    taken: usize,
}

impl<'a, T: Copy> Cursor<'a, T> {
    fn new(changes: &'a [(i32, T)]) -> Self {
        Self { changes, taken: 0 }
    }

    fn advance(&mut self, hour: i32) {
        while self.changes.get(self.taken).is_some_and(|(at, _)| *at <= hour) {
            self.taken += 1;
        }
    }

    fn value(&self) -> Option<T> {
        self.taken.checked_sub(1).map(|taken| self.changes[taken].1)
    }
}

pub(super) fn points(spans: &[HourSpan], mut hours: Vec<i32>) -> Vec<(i32, i32, bool)> {
    hours.sort_unstable();
    hours.dedup();
    let mut points = Vec::new();
    for span in spans {
        let start = hours.partition_point(|hour| *hour <= span.first_hour);
        let inside = hours[start..].iter().copied().take_while(|hour| *hour <= span.last_hour);
        let mut first = span.first_hour;
        let mut starts = true;
        for hour in inside {
            points.push((first, hour - 1, starts));
            (first, starts) = (hour, false);
        }
        points.push((first, span.last_hour, starts));
    }
    points
}

pub(super) fn stock_lines(replay: &Replay, groups: &[(usize, &Group)], untyped: &[u64], cancelled: &AtomicBool) -> io::Result<Vec<Vec<Piece<i128>>>> {
    let mut ids: Vec<u64> = groups.iter().flat_map(|(_, group)| group.variants.iter().copied()).chain(untyped.iter().copied()).collect();
    ids.sort_unstable();
    ids.dedup();
    let place: HashMap<u64, usize> = ids.iter().enumerate().map(|(at, id)| (*id, at)).collect();
    let mut cursors: Vec<_> = ids.iter().map(|id| Cursor::new(replay.variants.get(id).map_or(&[][..], Vec::as_slice))).collect();
    let members: Vec<Vec<usize>> = groups.iter().map(|(_, group)| group.variants.iter().map(|id| place[id]).collect()).collect();
    let untyped: Vec<usize> = untyped.iter().map(|id| place[id]).collect();
    let refused: Vec<bool> = groups.iter().map(|(_, group)| group.types.iter().any(|key| key.starts_with(UNRESOLVED_MARK))).collect();
    let hours = cursors.iter().flat_map(|cursor| cursor.changes.iter().map(|(hour, _)| *hour)).collect();
    let mut lines = vec![Vec::new(); groups.len()];
    for (count, (first, last, starts)) in points(&replay.stock, hours).into_iter().enumerate() {
        if count % 1024 == 0 {
            check_cancel(cancelled)?;
        }
        for cursor in &mut cursors {
            cursor.advance(first);
        }
        let stock = |at: usize| cursors[at].value().unwrap_or(Stock::Absent);
        let blocked = untyped.iter().any(|&at| stock(at).holds());
        for (index, members) in members.iter().enumerate() {
            let sum = if blocked || refused[index] { None } else { members.iter().try_fold(0_i128, |sum, &at| stock(at).raw().map(|raw| sum + raw)) };
            push(&mut lines[index], (first, last), sum, starts);
        }
    }
    Ok(lines)
}

pub(super) fn variant_line(replay: &Replay, id: u64) -> Vec<Piece<i128>> {
    let changes = replay.variants.get(&id).map_or(&[][..], Vec::as_slice);
    let mut cursor = Cursor::new(changes);
    let mut line = Vec::new();
    let hours = changes.iter().map(|(hour, _)| *hour).collect();
    for (first, last, starts) in points(&replay.stock, hours) {
        cursor.advance(first);
        push(&mut line, (first, last), cursor.value().unwrap_or(Stock::Absent).raw(), starts);
    }
    line
}

pub(super) struct Record<'a> {
    pub logistics: &'a TypeTokens,
    pub notes: BTreeMap<&'a str, Option<&'a str>>,
    pub unresolved: Vec<&'a str>,
}

const NEED_UNAVAILABLE: &str = "Need unavailable.";

pub(super) fn logistics_lines(replay: &Replay, groups: &[(usize, &Group)], record: &Record<'_>, cancelled: &AtomicBool) -> io::Result<Vec<Vec<Piece<Balance>>>> {
    let mut tokens: Vec<u32> = groups
        .iter()
        .flat_map(|(_, group)| group.types.iter().map(String::as_str))
        .chain(record.unresolved.iter().copied())
        .filter_map(|key| record.logistics.get(key))
        .flat_map(|(tokens, _)| tokens.iter().copied())
        .collect();
    tokens.sort_unstable();
    tokens.dedup();
    let place: HashMap<u32, usize> = tokens.iter().enumerate().map(|(at, token)| (*token, at)).collect();
    let mut cursors: Vec<_> = tokens.iter().map(|token| Cursor::new(replay.logistics.get(token).map_or(&[][..], Vec::as_slice))).collect();
    let places = |key: &str| -> Vec<usize> { record.logistics.get(key).map(|(tokens, _)| tokens.iter().map(|token| place[token]).collect()).unwrap_or_default() };
    let members: Vec<Vec<(Vec<usize>, Option<&str>)>> = groups
        .iter()
        .map(|(_, group)| group.types.iter().map(|key| (places(key), record.notes.get(key.as_str()).copied().unwrap_or(Some(NEED_UNAVAILABLE)))).collect())
        .collect();
    let unresolved: Vec<Vec<usize>> = record.unresolved.iter().map(|key| places(key)).collect();
    let hours = cursors.iter().flat_map(|cursor| cursor.changes.iter().map(|(hour, _)| *hour)).collect();
    let bound = NEED_BOUND_RAW.unsigned_abs();
    let mut lines = vec![Vec::new(); groups.len()];
    for (count, (first, last, starts)) in points(&replay.balance, hours).into_iter().enumerate() {
        if count % 1024 == 0 {
            check_cancel(cancelled)?;
        }
        for cursor in &mut cursors {
            cursor.advance(first);
        }
        let row = |tokens: &[usize]| tokens.iter().rev().find_map(|&at| cursors[at].value().flatten());
        let unattributed = unresolved.iter().any(|tokens| row(tokens).is_some_and(|row: Logistics| row.balance != Some(0) || row.stock != Some(0)));
        let listed = unresolved.iter().any(|tokens| row(tokens).is_some());
        for (index, (_, group)) in groups.iter().enumerate() {
            let types = &members[index];
            let value = if types.is_empty() || (group.resolved_arm() && unattributed) {
                None
            } else {
                (|| {
                    let (mut balance, mut stock_need) = (0_i128, Some(0_i128));
                    for (tokens, note) in types {
                        let held = row(tokens);
                        let (type_balance, type_stock) = match held {
                            Some(held) => (held.balance?, held.stock?),
                            None => (0, 0),
                        };
                        balance += i128::from(type_balance);
                        let available = held.and_then(|held| held.available);
                        let note = if available.is_some() { None } else { *note };
                        let native = available.unwrap_or(type_stock.max(0));
                        let need = native.wrapping_sub(type_balance);
                        stock_need = if note.is_some() || need.unsigned_abs() >= bound { None } else { stock_need.map(|sum| sum + i128::from(need)) };
                    }
                    if group.resolved_arm() && listed {
                        stock_need = None;
                    }
                    Some((balance, stock_need))
                })()
            };
            push(&mut lines[index], (first, last), value, starts);
        }
    }
    Ok(lines)
}

fn members(query: &Query<'_>, replays: &[Arc<Replay>], content: &Content, types: &mut Types, warnings: &mut Vec<String>) -> io::Result<(BTreeMap<u64, Member>, TypeTokens, BTreeMap<String, Domain>)> {
    let ids: BTreeSet<u64> = replays.iter().flat_map(|replay| replay.variants.keys().copied().chain(replay.produced.iter().copied())).collect();
    let tokens: BTreeSet<u32> = replays.iter().flat_map(|replay| replay.logistics.keys().copied()).collect();
    let last_known = replays.iter().flat_map(|replay| [&replay.stock, &replay.lines, &replay.balance, &replay.ai]).flatten().map(|span| span.last_hour).max();
    let mut named: BTreeMap<u64, super::Named> = BTreeMap::new();
    let mut spelled: BTreeMap<u32, String> = BTreeMap::new();
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(hour) = last_known {
        let mut asked: Vec<u64> = ids.iter().copied().collect();
        let mut hours = vec![hour];
        while let Some(hour) = hours.pop() {
            let found = super::name_at(query, hour, &asked, &tokens, content, &mut refused)?;
            spelled.extend(found.1);
            named.extend(found.0.into_iter().filter(|(_, name)| name.known));
            asked.retain(|id| !named.contains_key(id));
            let earlier: BTreeSet<i32> = asked
                .iter()
                .filter_map(|id| replays.iter().filter_map(|replay| replay.variants.get(id)?.iter().rev().find(|(_, stock)| *stock != Stock::Absent).map(|(at, _)| *at)).max())
                .filter(|at| *at < hour)
                .collect();
            hours.extend(earlier.last().copied());
        }
    }
    let mut variants = BTreeMap::new();
    let mut unnamed = 0;
    for id in ids {
        let held = named.remove(&id);
        if held.is_none() {
            unnamed += 1;
        }
        let token = held.as_ref().map(|name| name.token.clone()).unwrap_or_default();
        let archetype = types.variant(content, &token);
        let names = held
            .map(|name| vec![StockVariantName { first_hour: query.timeline.first, name: name.name, known: true, design: name.design }])
            .unwrap_or_default();
        variants.insert(id, Member { domain: archetype.as_deref().and_then(|archetype| types.domain(archetype)), token, archetype, names });
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} designs in stock or production have no recorded definition in this branch; their type is unknown."));
    }
    for (why, count) in refused {
        warnings.push(format!("{count} equipment designs with unknown text or design operands: {why}"));
    }
    let mut logistics = TypeTokens::new();
    let mut domains = BTreeMap::new();
    for token in tokens {
        let spelling = spelled.get(&token).cloned().unwrap_or_else(|| format!("token {token}"));
        let key = types.logistics(content, &spelling);
        if let Some(domain) = types.domain(&key) {
            domains.insert(key.clone(), domain);
        }
        let nonzero = replays.iter().any(|replay| {
            replay.logistics.get(&token).is_some_and(|changes| {
                changes.iter().any(|(_, row)| row.is_some_and(|row| row.balance != Some(0) || row.stock != Some(0) || row.available != Some(0)))
            })
        });
        let entry = logistics.entry(key).or_default();
        entry.0.push(token);
        entry.1 |= nonzero;
    }
    Ok((variants, logistics, domains))
}

fn build(query: &Query<'_>, replays: Vec<Arc<Replay>>, included: &[String], content: &Content, cancelled: &AtomicBool) -> io::Result<Model> {
    let mut warnings = content.warnings.clone();
    let mut types = Types::default();
    let (variants, logistics, domains) = members(query, &replays, content, &mut types, &mut warnings)?;
    let types = types.finish(content, &mut warnings);
    let groups = groups(&variants, &logistics, &domains, included);
    let untyped: Vec<u64> = variants.iter().filter(|(_, member)| member.archetype.is_none()).map(|(&id, _)| id).collect();
    let notes = logistics
        .keys()
        .map(|key| (key.as_str(), types.iter().find(|kind| kind.key == *key).map_or(Some(NEED_UNAVAILABLE), |kind| kind.need_note.as_deref())))
        .collect();
    let record = Record {
        logistics: &logistics,
        notes,
        unresolved: logistics.keys().filter(|key| key.starts_with(UNRESOLVED_MARK) && (included.is_empty() || included.iter().any(|item| item == *key))).map(String::as_str).collect(),
    };
    let summed: Vec<(usize, &Group)> = groups.iter().enumerate().filter(|(_, group)| group.kind != Kind::Variant).collect();
    let mut lines = Vec::with_capacity(replays.len());
    for replay in &replays {
        let stock = stock_lines(replay, &summed, &untyped, cancelled)?;
        let logistics = logistics_lines(replay, &summed, &record, cancelled)?;
        let mut holder: Vec<Lines> = groups
            .iter()
            .map(|group| Lines { stock: if group.kind == Kind::Variant { variant_line(replay, group.variants[0]) } else { Vec::new() }, logistics: Vec::new() })
            .collect();
        for (((index, _), stock), logistics) in summed.iter().zip(stock).zip(logistics) {
            holder[*index] = Lines { stock, logistics };
        }
        lines.push(holder);
    }
    drop(record);
    Ok(Model { replays, variants, logistics, types, groups, lines, first: query.timeline.first, last: query.timeline.last, warnings })
}

#[derive(Default)]
struct Kept {
    models: HashMap<String, Arc<Model>>,
}

fn model(source: &Source, query: &Query<'_>, tags: &[Country], included: &[String], load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<Model>> {
    let content = Content::load(source, load)?;
    let binding = content.catalogue.as_ref().map_or(0, |catalogue| Arc::as_ptr(catalogue) as usize);
    let key = serde_json::to_string(&(query.timeline.id, query.timeline.last, tags, included, binding)).map_err(io::Error::other)?;
    let state = source.state::<Kept>();
    if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).models.get(&key) {
        return Ok(Arc::clone(found));
    }
    let mut replays = Vec::with_capacity(tags.len());
    for tag in tags {
        check_cancel(cancelled)?;
        replays.push(history::kept(source, query, tag)?);
    }
    let made = Arc::new(build(query, replays, included, &content, cancelled)?);
    let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
    held.models.retain(|_, model| model.last == query.timeline.last);
    held.models.insert(key, Arc::clone(&made));
    Ok(made)
}

fn within(spans: &[HourSpan], within: Option<&[HourSpan]>) -> Vec<HourSpan> {
    let Some(within) = within else { return spans.to_vec() };
    let mut result = Vec::new();
    for span in spans {
        let start = within.partition_point(|member| member.last_hour < span.first_hour);
        for member in within[start..].iter().take_while(|member| member.first_hour <= span.last_hour) {
            result.push(HourSpan { first_hour: span.first_hour.max(member.first_hour), last_hour: span.last_hour.min(member.last_hour) });
        }
    }
    result
}

fn inside(hours: &[i32], spans: &[HourSpan]) -> impl Iterator<Item = i32> {
    hours.iter().copied().filter(|hour| spans.iter().any(|span| span.first_hour <= *hour && *hour <= span.last_hour)).collect::<Vec<_>>().into_iter()
}

fn overlaps(spans: Option<&[HourSpan]>, first: i32, last: i32) -> bool {
    spans.is_none_or(|spans| spans.get(spans.partition_point(|span| span.last_hour < first)).is_some_and(|span| span.first_hour <= last))
}

fn sorted(mut spans: Vec<HourSpan>) -> Vec<HourSpan> {
    spans.sort_by_key(|span| span.first_hour);
    spans
}

fn catalogue(source: &Source, query: &Query<'_>, params: &StockpileCatalogueParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<StockpileCatalogueReply> {
    let tags: Vec<Country> = params.holders.iter().map(|holder| holder.country.clone()).collect();
    let model = model(source, query, &tags, &params.included, load, cancelled)?;
    let mut reply = StockpileCatalogueReply {
        branch_id: params.branch_id.clone(),
        countries: tags.clone(),
        campaign_key: source.campaign_key().to_owned(),
        decimals: DECIMALS,
        types: model.types.clone(),
        variants: model
            .variants
            .iter()
            .map(|(id, member)| StockpileCatalogueVariant {
                id: id.to_string(),
                token: member.token.clone(),
                archetype: member.archetype.clone(),
                designs: member.names.iter().filter_map(|name| name.design.clone()).collect(),
                names: member.names.clone(),
            })
            .collect(),
        logistics: model.logistics.iter().map(|(key, &(_, nonzero))| StockpileCatalogueLogistics { key: key.clone(), nonzero }).collect(),
        reads: Vec::new(),
        line_reads: Vec::new(),
        logistics_reads: Vec::new(),
        stock_hours: Vec::new(),
        line_hours: Vec::new(),
        balance_hours: Vec::new(),
        ai_hours: Vec::new(),
        need_bound_raw: NEED_BOUND_RAW.to_string(),
        choices: BTreeMap::new(),
        warnings: model.warnings.clone(),
    };
    for (holder, replay) in params.holders.iter().zip(&model.replays) {
        let membership = holder.within.as_deref();
        let [stock, lines, balance, ai] = [&replay.stock, &replay.lines, &replay.balance, &replay.ai].map(|spans| within(spans, membership));
        reply.reads.extend(inside(&replay.reads, &stock));
        reply.line_reads.extend(inside(&replay.line_reads, &lines));
        reply.logistics_reads.extend(inside(&replay.logistics_reads, &balance));
        reply.stock_hours.extend(stock);
        reply.line_hours.extend(lines);
        reply.balance_hours.extend(balance);
        reply.ai_hours.extend(ai);
        if !replay.unresolved.is_empty() {
            reply.warnings.push(format!("{}: {} stretches of hours have no resolved country; nothing is shown for them.", query.holders.nearest_name(&holder.country, query.timeline.last), replay.unresolved.len()));
        }
    }
    for reads in [&mut reply.reads, &mut reply.line_reads, &mut reply.logistics_reads] {
        reads.sort_unstable();
        reads.dedup();
    }
    for spans in [&mut reply.stock_hours, &mut reply.line_hours, &mut reply.balance_hours, &mut reply.ai_hours] {
        *spans = sorted(std::mem::take(spans));
    }
    for (index, group) in model.groups.iter().enumerate() {
        check_cancel(cancelled)?;
        let mut choice = (0_i128, false, false);
        for (holder, params) in params.holders.iter().enumerate() {
            let membership = params.within.as_deref();
            let lines = &model.lines[holder][index];
            for piece in &lines.stock {
                if let Some(value) = piece.value
                    && overlaps(membership, piece.first, piece.last)
                {
                    choice.0 = choice.0.max(value);
                    choice.1 = true;
                }
            }
            for piece in &lines.logistics {
                if let Some((_, Some(need))) = piece.value
                    && overlaps(membership, piece.first, piece.last)
                {
                    choice.0 = choice.0.max(need);
                    choice.2 |= need > 0;
                }
            }
        }
        reply.choices.insert(group.wire.clone(), StockpileChoicePeak { raw: choice.0.to_string(), stock_drawable: choice.1, share_drawable: choice.2 });
    }
    if reply.balance_hours.is_empty() && reply.ai_hours.is_empty() {
        reply.warnings.push("The surplus or shortage is not in this recording: it has no logistics record (logistics equipment).".to_owned());
    }
    if reply.stock_hours.is_empty() {
        reply.warnings.push(format!("{}: no recorded stockpile in this branch", tags.iter().map(|country| query.holders.nearest_name(country, query.timeline.last)).collect::<Vec<_>>().join(", ")));
    }
    Ok(reply)
}

pub(super) struct Drawn {
    pub first: i32,
    pub last: i32,
    pub value: f64,
    pub absolute: u128,
    pub break_before: bool,
}

pub(super) fn drawn<T: Copy>(pieces: &[Piece<T>], measure: impl Fn(T) -> Option<(f64, u128)>, (from, to): (i32, i32), membership: Option<&[HourSpan]>, into: &mut Vec<Drawn>) {
    let measured = |piece: &Piece<T>| piece.value.and_then(&measure);
    let start = pieces.partition_point(|piece| piece.last < from);
    let end = pieces.partition_point(|piece| piece.first <= to);
    if start >= end {
        return;
    }
    let mut line: Vec<Drawn> = Vec::new();
    let mut joined = start > 0 && measured(&pieces[start - 1]).is_some();
    for piece in &pieces[start..end] {
        let Some((value, absolute)) = measured(piece) else {
            joined = false;
            continue;
        };
        line.push(Drawn { first: piece.first, last: piece.last, value, absolute, break_before: !joined || piece.starts });
        joined = true;
    }
    let Some(spans) = membership else {
        into.extend(line);
        return;
    };
    for drawn in line {
        let start = spans.partition_point(|span| span.last_hour < drawn.first);
        for span in spans[start..].iter().take_while(|span| span.first_hour <= drawn.last) {
            let first = drawn.first.max(span.first_hour);
            into.push(Drawn { first, last: drawn.last.min(span.last_hour), value: drawn.value, absolute: drawn.absolute, break_before: drawn.break_before || first != drawn.first });
        }
    }
}

pub(super) fn summary(mut lines: Vec<Drawn>, from: i32, to: i32, width: u32) -> ChartSummary {
    lines.sort_by_key(|line| line.first);
    let hours = i64::from(to) - i64::from(from) + 1;
    let hourly = hours <= i64::from(width);
    let bin_hours = if hourly { (hours + i64::from(width) - 1) / i64::from(width) } else { dashboard_query::series::grid(hours, width).1 };
    let first_bin = if hourly { i64::from(from) } else { i64::from(from).div_euclid(bin_hours) * bin_hours };
    let bins = ((i64::from(to) - first_bin) / bin_hours + 1).max(0) as usize;
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins];
    let mut runs = hourly.then(Vec::new);
    for line in &lines {
        let (start, end) = (i64::from(line.first).max(i64::from(from)), i64::from(line.last).min(i64::from(to)));
        let mut at = start;
        while at <= end {
            let slot = ((at - first_bin) / bin_hours) as usize;
            let bin_end = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(end);
            let (first, last) = (at as i32, bin_end as i32);
            let piece = ChartBucket {
                absolute_total: line.absolute * (bin_end - at + 1) as u128,
                first_hour: first,
                last_hour: last,
                first_value: line.value,
                last_value: line.value,
                last_value_hour: first,
                min_hour: first,
                min_value: line.value,
                max_hour: first,
                max_value: line.value,
                first_break: line.break_before && first == line.first,
                has_break: false,
            };
            buckets[slot] = match buckets[slot].take() {
                None => Some(piece),
                Some(held) => merge(Some(held), Some(piece)),
            };
            at = bin_end + 1;
        }
        if let Some(runs) = &mut runs
            && start <= end
        {
            runs.push(ChartRun { first_hour: start as i32, last_hour: end as i32, value: line.value, break_before: line.break_before && i64::from(line.first) >= i64::from(from) });
        }
    }
    for (slot, bucket) in buckets.iter_mut().enumerate() {
        if let Some(bucket) = bucket {
            let (first, last) = (first_bin + slot as i64 * bin_hours, first_bin + (slot as i64 + 1) * bin_hours - 1);
            bucket.has_break |= i64::from(bucket.first_hour) > first.max(i64::from(from)) || i64::from(bucket.last_hour) < last.min(i64::from(to));
        }
    }
    ChartSummary { from: f64::from(from), to: f64::from(to), width, buckets, runs }
}

fn chart(source: &Source, query: &Query<'_>, params: &StockpileChartParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<StockpileChartReply> {
    let tags: Vec<Country> = params.holders.iter().map(|holder| holder.country.clone()).collect();
    let model = model(source, query, &tags, &params.included, load, cancelled)?;
    let scale = 10_f64.powi(i32::from(DECIMALS));
    let (from, to) = ((params.from.floor() as i32).max(model.first), (params.to.floor() as i32).min(model.last));
    let mut reply = StockpileChartReply { branch_id: params.branch_id.clone(), lines: Vec::with_capacity(params.lines.len()), warnings: Vec::new() };
    for line in &params.lines {
        check_cancel(cancelled)?;
        let mut drawn_lines = Vec::new();
        match model.group(&line.group) {
            Some(index) => {
                for (holder, params) in params.holders.iter().enumerate() {
                    let lines = &model.lines[holder][index];
                    let membership = params.within.as_deref();
                    match line.measure {
                        StockpileMeasure::Stock => drawn(&lines.stock, |raw| Some((raw as f64 / scale, raw.unsigned_abs())), (from, to), membership, &mut drawn_lines),
                        StockpileMeasure::Share => drawn(
                            &lines.logistics,
                            |(balance, need)| need.filter(|need| *need > 0).map(|need| (balance as f64 / need as f64 * 100.0, 0)),
                            (from, to),
                            membership,
                            &mut drawn_lines,
                        ),
                        StockpileMeasure::Need => drawn(&lines.logistics, |(_, need)| need.map(|need| (need as f64 / scale, need.unsigned_abs())), (from, to), membership, &mut drawn_lines),
                    }
                }
            }
            None => reply.warnings.push(format!("The stockpile has no group {}.", line.group)),
        }
        let summary = if from <= to { summary(drawn_lines, from, to, params.width) } else { ChartSummary { from: params.from, to: params.to, width: params.width, buckets: Vec::new(), runs: None } };
        reply.lines.push(StockpileChartLine { group: line.group.clone(), measure: line.measure, summary });
    }
    Ok(reply)
}

pub struct EquipmentStockpileCatalogue;

impl Product for EquipmentStockpileCatalogue {
    const KEY: &'static str = "equipment-stockpile-catalogue";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StockpileCatalogueParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, included, "holders[].within[].firstHour": Start, "holders[].within[].lastHour": End, "holders[].country"];
    type Reply = StockpileCatalogueReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            StockpileHolder::decl(),
            StockpileCatalogueParams::decl(),
            StockpileChoicePeak::decl(),
            StockVariantName::decl(),
            StockpileCatalogueVariant::decl(),
            StockpileCatalogueLogistics::decl(),
            StockpileCatalogueReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(&params.branch_id, &params.holders)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        catalogue(source, &query, params, load, cancelled)
    }
}

pub struct EquipmentStockpileChart;

impl Product for EquipmentStockpileChart {
    const KEY: &'static str = "equipment-stockpile-chart";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StockpileChartParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, included, lines, from: Start, to: End, width, "holders[].within[].firstHour": Start, "holders[].within[].lastHour": End, "holders[].country"];
    type Reply = StockpileChartReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StockpileMeasure::decl(), StockpileLine::decl(), StockpileChartParams::decl(), StockpileChartLine::decl(), StockpileChartReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(&params.branch_id, &params.holders)?;
        if !params.from.is_finite() || !params.to.is_finite() || params.from > params.to || params.from < f64::from(i32::MIN) || params.to > f64::from(i32::MAX) || params.width == 0 {
            return Err("Invalid stockpile viewport");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        chart(source, &query, params, load, cancelled)
    }
}
