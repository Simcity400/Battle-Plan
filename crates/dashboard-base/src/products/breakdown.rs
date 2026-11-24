use crate::Source;
use crate::products::artwork::sprites::SpriteFrame;
use content_reader_layers::content::ResolvedContent;
use dashboard_core::config::LoadContext;
use serde::Serialize;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, PoisonError, Weak},
};
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum BreakdownState {
    Known,
    Partial,
    AtLeast,
    NotRecorded,
}

#[derive(Clone, Debug, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownTotal {
    #[doc = " Scaled by `decimals`; none when nothing is known."]
    pub raw: Option<String>,
    pub state: BreakdownState,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownPart {
    #[doc = " Unique among its siblings."]
    pub key: String,
    pub name: String,
    #[doc = " Why `name` is only an identifier: the recording names it nowhere."]
    pub name_missing: Option<String>,
    #[doc = " Its place in the recording's own order of the parts at its level: colours follow it, so a"]
    #[doc = " filtered or reordered breakdown keeps them."]
    pub ordinal: u32,
    #[doc = " Unattributed or other: drawn in the reserved grey."]
    pub other: bool,
    #[doc = " Scaled by `decimals`; none when not recorded."]
    pub raw: Option<String>,
    pub state: BreakdownState,
    pub reason: Option<String>,
    #[doc = " Of the whole; none where the whole is only a lower bound or the part is not positive."]
    pub share: Option<f64>,
    #[doc = " Of the part above it (of the whole for a first-level part); none where that is only a lower"]
    #[doc = " bound or either is not positive."]
    pub parent_share: Option<f64>,
    pub count: Option<u64>,
    #[doc = " The latest hour one of its amounts was read; none where the product does not date them."]
    pub last: Option<i32>,
    #[doc = " Its picture, where the product names one."]
    pub art: Option<SpriteFrame>,
    #[doc = " In display order. Their known amounts add up to this part's: an amount given to this part"]
    #[doc = " itself and to none of them is the last known child, `other`."]
    pub children: Vec<BreakdownPart>,
}

#[derive(Clone, Debug, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub decimals: u8,
    pub total: BreakdownTotal,
    #[doc = " In display order: the largest known part first, then parts at zero or below, then other,"]
    #[doc = " then parts not recorded."]
    pub parts: Vec<BreakdownPart>,
    #[doc = " Hours of the period that were not recorded."]
    pub unrecorded_hours: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Amount {
    Known(i128),
    Partial(i128, String),
    NotRecorded(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub key: String,
    pub name: String,
    pub name_missing: Option<String>,
    pub ordinal: u32,
    pub other: bool,
    pub art: Option<SpriteFrame>,
}

impl Node {
    pub fn new(key: impl Into<String>, name: impl Into<String>, ordinal: u32) -> Self {
        Self { key: key.into(), name: name.into(), name_missing: None, ordinal, other: false, art: None }
    }

    pub fn name_missing(mut self, reason: Option<String>) -> Self {
        self.name_missing = reason;
        self
    }

    pub fn other(mut self) -> Self {
        self.other = true;
        self
    }

    pub fn with_art(mut self, art: Option<SpriteFrame>) -> Self {
        self.art = art;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub path: Vec<Node>,
    pub amount: Amount,
    pub count: Option<u64>,
    pub last: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unrecorded {
    pub hours: u32,
    pub reason: Option<String>,
}

const REST_KEY: &str = "~rest";
const REST_NAME: &str = "Other";

#[derive(Clone, Debug, Default)]
struct Sum {
    known: i128,
    measured: u32,
    partial: Vec<String>,
    missing: Vec<String>,
    count: Option<u64>,
    last: Option<i32>,
}

impl Sum {
    fn add(&mut self, amount: &Amount, count: Option<u64>, last: Option<i32>) {
        self.date(last);
        match amount {
            Amount::Known(value) => {
                self.known = self.known.saturating_add(*value);
                self.measured += 1;
            }
            Amount::Partial(value, reason) => {
                self.known = self.known.saturating_add(*value);
                self.measured += 1;
                remember(&mut self.partial, reason);
            }
            Amount::NotRecorded(reason) => remember(&mut self.missing, reason),
        }
        self.add_count(count);
    }

    fn merge(&mut self, other: &Sum) {
        self.known = self.known.saturating_add(other.known);
        self.measured += other.measured;
        for reason in &other.partial {
            remember(&mut self.partial, reason);
        }
        for reason in &other.missing {
            remember(&mut self.missing, reason);
        }
        self.add_count(other.count);
        self.date(other.last);
    }

    fn date(&mut self, last: Option<i32>) {
        if let Some(hour) = last {
            self.last = Some(self.last.map_or(hour, |held| held.max(hour)));
        }
    }

    fn add_count(&mut self, count: Option<u64>) {
        if let Some(count) = count {
            self.count = Some(self.count.unwrap_or(0).saturating_add(count));
        }
    }

    fn is_empty(&self) -> bool {
        self.measured == 0 && self.missing.is_empty()
    }

    fn state(&self) -> BreakdownState {
        match (self.measured, self.missing.is_empty(), self.partial.is_empty()) {
            (0, false, _) => BreakdownState::NotRecorded,
            (_, false, _) => BreakdownState::AtLeast,
            (_, true, false) => BreakdownState::Partial,
            _ => BreakdownState::Known,
        }
    }

    fn reason(&self) -> Option<String> {
        let reasons: Vec<&str> = self.missing.iter().chain(&self.partial).map(String::as_str).collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }

    fn raw(&self) -> Option<String> {
        (self.measured > 0 || self.missing.is_empty()).then(|| self.known.to_string())
    }

    fn whole(&self) -> Option<i128> {
        matches!(self.state(), BreakdownState::Known | BreakdownState::Partial).then_some(self.known)
    }
}

fn remember(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|held| held == reason) {
        reasons.push(reason.to_owned());
    }
}

struct Branch {
    node: Node,
    sum: Sum,
    own: Sum,
    children: Vec<Branch>,
    index: HashMap<String, usize>,
}

impl Branch {
    fn new(node: Node) -> Self {
        Self { node, sum: Sum::default(), own: Sum::default(), children: Vec::new(), index: HashMap::new() }
    }

    fn child(&mut self, node: &Node) -> &mut Branch {
        let at = match self.index.get(&node.key) {
            Some(&at) => at,
            None => {
                self.children.push(Branch::new(node.clone()));
                self.index.insert(node.key.clone(), self.children.len() - 1);
                self.children.len() - 1
            }
        };
        &mut self.children[at]
    }

    fn add(&mut self, path: &[Node], sum: &Sum) {
        self.sum.merge(sum);
        let mut at = self;
        for node in path {
            at = at.child(node);
            at.sum.merge(sum);
        }
        at.own.merge(sum);
    }

    fn split_rest(&mut self) {
        if !self.children.is_empty() && !self.own.is_empty() {
            let ordinal = self.children.iter().map(|child| child.node.ordinal.saturating_add(1)).max().unwrap_or(0);
            let mut rest = Branch::new(Node::new(REST_KEY, REST_NAME, ordinal).other());
            rest.sum = std::mem::take(&mut self.own);
            self.children.push(rest);
        }
        for child in &mut self.children {
            child.split_rest();
        }
    }
}

fn class(branch: &Branch) -> u8 {
    match branch.sum.state() {
        BreakdownState::NotRecorded => 3,
        _ if branch.node.other => 2,
        _ if branch.sum.known > 0 => 0,
        _ => 1,
    }
}

fn share_of(part: &Sum, whole: Option<i128>) -> Option<f64> {
    match (whole, part.state()) {
        (Some(whole), BreakdownState::Known | BreakdownState::Partial) if whole > 0 && part.known > 0 => Some(part.known as f64 / whole as f64),
        _ => None,
    }
}

fn exact_share_of(part: &Sum, parent: Option<i128>) -> Option<f64> {
    (part.state() == BreakdownState::Known).then(|| share_of(part, parent)).flatten()
}

fn parts(children: Vec<Branch>, whole: Option<i128>, parent: Option<i128>) -> Vec<BreakdownPart> {
    let mut children = children;
    children.sort_by(|a, b| {
        class(a)
            .cmp(&class(b))
            .then_with(|| if class(a) < 2 { b.sum.known.cmp(&a.sum.known) } else { std::cmp::Ordering::Equal })
            .then_with(|| a.node.ordinal.cmp(&b.node.ordinal))
    });
    children
        .into_iter()
        .map(|branch| {
            let own_whole = branch.sum.whole().filter(|_| whole.is_some() && branch.sum.state() == BreakdownState::Known);
            let children = parts(branch.children, whole, own_whole);
            BreakdownPart {
                share: share_of(&branch.sum, whole),
                parent_share: exact_share_of(&branch.sum, parent),
                key: branch.node.key,
                name: branch.node.name,
                name_missing: branch.node.name_missing,
                ordinal: branch.node.ordinal,
                other: branch.node.other,
                raw: branch.sum.raw(),
                state: branch.sum.state(),
                reason: branch.sum.reason(),
                count: branch.sum.count,
                last: branch.sum.last,
                art: branch.node.art,
                children,
            }
        })
        .collect()
}

fn assemble(decimals: u8, mut root: Branch, unrecorded: &Unrecorded, qualified: &[String], unwitnessed: bool) -> Breakdown {
    root.split_rest();
    let unwitnessed = unwitnessed && root.sum.measured == 0;
    let mut state = if unwitnessed { BreakdownState::NotRecorded } else { root.sum.state() };
    let mut reason = root.sum.reason();
    let gap = (unrecorded.hours > 0).then_some(unrecorded.reason.as_deref()).into_iter().flatten();
    for extra in gap.chain(qualified.iter().map(String::as_str)) {
        reason = Some(match reason {
            Some(held) if held.contains(extra) => held,
            Some(held) => format!("{extra}; {held}"),
            None => extra.to_owned(),
        });
    }
    if state == BreakdownState::Known && (unrecorded.hours > 0 || !qualified.is_empty()) {
        state = BreakdownState::Partial;
    }
    let whole = matches!(state, BreakdownState::Known | BreakdownState::Partial).then_some(root.sum.known);
    Breakdown {
        decimals,
        total: BreakdownTotal { raw: if unwitnessed { None } else { root.sum.raw() }, state, reason },
        parts: parts(root.children, whole, whole.filter(|_| state == BreakdownState::Known)),
        unrecorded_hours: unrecorded.hours,
    }
}

pub fn breakdown(decimals: u8, leaves: &[Leaf], unrecorded: &Unrecorded) -> Breakdown {
    let mut root = Branch::new(Node::new("", "", 0));
    for leaf in leaves {
        let mut sum = Sum::default();
        sum.add(&leaf.amount, leaf.count, leaf.last);
        root.add(&leaf.path, &sum);
    }
    assemble(decimals, root, unrecorded, &[], false)
}
pub struct History {
    decimals: u8,
    from: i32,
    to: i32,
    paths: Vec<Vec<Node>>,
    steps: Vec<Vec<(i32, Sum)>>,
    qualified: Vec<(i32, String)>,
    gaps: Vec<(i32, i32, String)>,
    changes: Vec<i32>,
}

pub struct HistoryBuilder {
    decimals: u8,
    from: i32,
    to: i32,
    paths: Vec<Vec<Node>>,
    index: HashMap<Vec<String>, usize>,
    events: Vec<(i32, usize, Amount, Option<u64>)>,
    qualified: Vec<(i32, String)>,
    gaps: Vec<(i32, i32, String)>,
    changes: Vec<i32>,
    dated: bool,
}

impl HistoryBuilder {
    pub fn new(decimals: u8, from: i32, to: i32) -> Self {
        Self { decimals, from, to, paths: Vec::new(), index: HashMap::new(), events: Vec::new(), qualified: Vec::new(), gaps: Vec::new(), changes: Vec::new(), dated: false }
    }

    pub fn dated(mut self) -> Self {
        self.dated = true;
        self
    }
    pub fn leaf(&mut self, path: Vec<Node>) -> usize {
        let keys: Vec<String> = path.iter().map(|node| node.key.clone()).collect();
        if let Some(&at) = self.index.get(&keys) {
            return at;
        }
        self.paths.push(path);
        self.index.insert(keys, self.paths.len() - 1);
        self.paths.len() - 1
    }
    pub fn add(&mut self, hour: i32, leaf: usize, amount: Amount, count: Option<u64>) {
        self.events.push((hour, leaf, amount, count));
    }
    pub fn qualify(&mut self, hour: i32, reason: impl Into<String>) {
        self.qualified.push((hour, reason.into()));
    }
    pub fn unrecorded(&mut self, first: i32, last: i32, reason: impl Into<String>) {
        if first <= last {
            self.gaps.push((first, last, reason.into()));
        }
    }
    pub fn change(&mut self, hour: i32) {
        self.changes.push(hour);
    }

    pub fn build(mut self) -> History {
        self.events.sort_by_key(|event| (event.0, event.1));
        let mut steps: Vec<Vec<(i32, Sum)>> = vec![Vec::new(); self.paths.len()];
        for (hour, leaf, amount, count) in self.events {
            self.changes.push(hour);
            let held = &mut steps[leaf];
            let mut sum = held.last().map(|(_, sum)| sum.clone()).unwrap_or_default();
            sum.add(&amount, count, self.dated.then_some(hour));
            match held.last_mut() {
                Some(last) if last.0 == hour => last.1 = sum,
                _ => held.push((hour, sum)),
            }
        }
        self.qualified.sort_by_key(|(hour, _)| *hour);
        let mut seen = std::collections::HashSet::new();
        self.qualified.retain(|(_, reason)| seen.insert(reason.clone()));
        self.changes.extend(self.qualified.iter().map(|(hour, _)| *hour));
        self.gaps.sort_by_key(|gap| gap.0);
        for &(first, last, _) in &self.gaps {
            self.changes.push(first);
            self.changes.push(last.saturating_add(1));
        }
        self.changes.sort_unstable();
        self.changes.dedup();
        History { decimals: self.decimals, from: self.from, to: self.to, paths: self.paths, steps, qualified: self.qualified, gaps: self.gaps, changes: self.changes }
    }
}

impl History {
    pub fn at(&self, hour: i32) -> (Breakdown, i32) {
        let mut root = Branch::new(Node::new("", "", 0));
        for (path, steps) in self.paths.iter().zip(&self.steps) {
            let at = steps.partition_point(|(from, _)| *from <= hour);
            if let Some((_, sum)) = at.checked_sub(1).map(|at| &steps[at]) {
                root.add(path, sum);
            }
        }
        let qualified: Vec<String> = self.qualified[..self.qualified.partition_point(|(from, _)| *from <= hour)].iter().map(|(_, reason)| reason.clone()).collect();
        let mut unrecorded = Unrecorded::default();
        let mut reasons: Vec<String> = Vec::new();
        let (mut inside, mut counted) = (false, i64::from(self.from) - 1);
        for (first, last, reason) in &self.gaps[..self.gaps.partition_point(|gap| gap.0 <= hour)] {
            let (first, last) = (i64::from((*first).max(self.from)), i64::from((*last).min(hour)));
            if first > last {
                continue;
            }
            inside |= last == i64::from(hour);
            let start = first.max(counted + 1);
            if start <= last {
                unrecorded.hours = unrecorded.hours.saturating_add(u32::try_from(last - start + 1).unwrap_or(u32::MAX));
            }
            counted = counted.max(last);
            remember(&mut reasons, reason);
        }
        unrecorded.reason = (!reasons.is_empty()).then(|| reasons.join("; "));
        let period = i64::from(hour) - i64::from(self.from) + 1;
        let unwitnessed = period > 0 && i64::from(unrecorded.hours) >= period;
        (assemble(self.decimals, root, &unrecorded, &qualified, unwitnessed), if inside { hour } else { self.through(hour) })
    }
    pub fn through(&self, hour: i32) -> i32 {
        let next = self.changes[self.changes.partition_point(|&at| at <= hour)..].first().copied();
        next.map_or(self.to, |next| next - 1).min(self.to).max(hour)
    }
}
pub struct Dated<T>(Vec<(i32, T)>);

impl<T> Dated<T> {
    pub fn new(mut values: Vec<(i32, T)>) -> Self {
        values.sort_by_key(|(hour, _)| *hour);
        let mut out: Vec<(i32, T)> = Vec::with_capacity(values.len());
        for (hour, value) in values {
            match out.last_mut() {
                Some(last) if last.0 == hour => last.1 = value,
                _ => out.push((hour, value)),
            }
        }
        Self(out)
    }

    pub fn at(&self, hour: i32) -> Option<&T> {
        self.0[..self.0.partition_point(|(from, _)| *from <= hour)].last().map(|(_, value)| value)
    }

    pub fn hours(&self) -> impl Iterator<Item = i32> + '_ {
        self.0.iter().map(|(hour, _)| *hour)
    }
}
pub fn counted(sorted: &[i32], hour: i32) -> u32 {
    u32::try_from(sorted.partition_point(|&at| at <= hour)).unwrap_or(u32::MAX)
}

const HELD: usize = 16;

type ContentMark = Option<Weak<ResolvedContent>>;

pub struct Held<K, V>(Vec<(K, ContentMark, Arc<V>)>);

impl<K, V> Default for Held<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

fn same_content(a: &ContentMark, b: &ContentMark) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Weak::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}
pub fn held<K, V>(source: &Source, context: &LoadContext, key: K, make: impl FnOnce() -> io::Result<V>) -> io::Result<Arc<V>>
where
    K: PartialEq + Send + 'static,
    V: Send + Sync + 'static,
{
    let content: ContentMark = context.content.as_ref().and_then(|options| source.content(&options.options).ok()).map(|content| Arc::downgrade(&content));
    let state = source.state::<Held<K, V>>();
    {
        let mut entries = state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = entries.0.iter().position(|(held, mark, _)| *held == key && same_content(mark, &content)) {
            let entry = entries.0.remove(at);
            let found = Arc::clone(&entry.2);
            entries.0.push(entry);
            return Ok(found);
        }
    }
    let made = Arc::new(make()?);
    let mut entries = state.lock().unwrap_or_else(PoisonError::into_inner);
    entries.0.retain(|(held, _, _)| *held != key);
    if entries.0.len() >= HELD {
        entries.0.remove(0);
    }
    entries.0.push((key, content, Arc::clone(&made)));
    Ok(made)
}
