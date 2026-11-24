
use dashboard_base::products::Country;
use super::{
    Content, Domain, NEED_BOUND_RAW, StockType, Types, UNRESOLVED_ARM, UNRESOLVED_MARK,
    history::{self, Logistics, Replay, Stock},
};
use dashboard_base::{Source, span::HourSpan};
use dashboard_base::products::LoadContext;
use dashboard_query::Query;
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StockpileMeasure {
    Stock,
    Share,
    Need,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Type,
    Domain,
    Variant,
}

pub(super) struct Group {
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
}

pub(super) struct Model {
    pub replays: Vec<Arc<Replay>>,
    pub types: Vec<StockType>,
    pub groups: Vec<Group>,
    pub lines: Vec<Vec<Lines>>,
    pub last: i32,
    pub warnings: Vec<String>,
}

fn domain_key(domain: Option<Domain>) -> &'static str {
    match domain {
        Some(Domain::Land) => "land",
        Some(Domain::Air) => "air",
        Some(Domain::Naval) => "naval",
        None => UNRESOLVED_ARM,
    }
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
        grouped.entry(key.clone()).or_insert_with(|| Group { kind, key, variants: Vec::new(), types: Vec::new() })
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
            result.push(Group { kind: Kind::Variant, key, variants: vec![id], types: Vec::new() });
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
        variants.insert(id, Member { domain: archetype.as_deref().and_then(|archetype| types.domain(archetype)), token, archetype });
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
    Ok(Model { replays, types, groups, lines, last: query.timeline.last, warnings })
}

#[derive(Default)]
struct Kept {
    models: HashMap<String, Arc<Model>>,
}

pub(super) fn model(source: &Source, query: &Query<'_>, tags: &[Country], included: &[String], load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<Model>> {
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
