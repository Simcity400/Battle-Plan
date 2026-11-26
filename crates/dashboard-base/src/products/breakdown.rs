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
    #[doc = " None when nothing is known."]
    pub amount: Option<BreakdownAmount>,
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
    #[doc = " None when not recorded."]
    pub amount: Option<BreakdownAmount>,
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
    pub total: BreakdownTotal,
    #[doc = " In display order: the largest known part first, then parts at zero or below, then other,"]
    #[doc = " then parts not recorded."]
    pub parts: Vec<BreakdownPart>,
    #[doc = " Hours of the period that were not recorded."]
    pub unrecorded_hours: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownAmount {
    #[doc = " Reduced, over a positive denominator."]
    pub numerator: String,
    pub denominator: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exact {
    numerator: i128,
    denominator: i128,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Exact {
    pub const ZERO: Self = Self { numerator: 0, denominator: 1 };

    pub fn new(numerator: i128, denominator: i128) -> Option<Self> {
        if denominator <= 0 {
            return None;
        }
        let divisor = i128::try_from(gcd(numerator.unsigned_abs(), denominator.unsigned_abs())).ok()?;
        Some(Self { numerator: numerator / divisor, denominator: denominator / divisor })
    }

    pub fn whole(value: i128) -> Self {
        Self { numerator: value, denominator: 1 }
    }

    pub fn scaled(raw: i128, decimals: u8) -> Option<Self> {
        Self::new(raw, 10_i128.checked_pow(u32::from(decimals))?)
    }

    pub fn numerator(self) -> i128 {
        self.numerator
    }

    pub fn denominator(self) -> i128 {
        self.denominator
    }

    pub fn plus(self, other: Self) -> Option<Self> {
        self.checked_add(other)
    }

    pub fn minus(self, other: Self) -> Option<Self> {
        self.checked_add(Self { numerator: other.numerator.checked_neg()?, denominator: other.denominator })
    }

    fn checked_add(self, other: Self) -> Option<Self> {
        let divisor = i128::try_from(gcd(self.denominator.unsigned_abs(), other.denominator.unsigned_abs())).ok()?;
        let denominator = self.denominator.checked_mul(other.denominator / divisor)?;
        let numerator = self.numerator.checked_mul(other.denominator / divisor)?.checked_add(other.numerator.checked_mul(self.denominator / divisor)?)?;
        Self::new(numerator, denominator)
    }

    fn value(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    fn of(self, whole: Self) -> f64 {
        match (self.numerator.checked_mul(whole.denominator), whole.numerator.checked_mul(self.denominator)) {
            (Some(part), Some(whole)) => part as f64 / whole as f64,
            _ => self.value() / whole.value(),
        }
    }

    fn compare(self, other: Self) -> std::cmp::Ordering {
        let sign = self.numerator.signum().cmp(&other.numerator.signum());
        if sign != std::cmp::Ordering::Equal {
            return sign;
        }
        let left = wide_mul(self.numerator.unsigned_abs(), other.denominator.unsigned_abs());
        let right = wide_mul(other.numerator.unsigned_abs(), self.denominator.unsigned_abs());
        if self.numerator < 0 { right.cmp(&left) } else { left.cmp(&right) }
    }

    pub fn wire(self) -> BreakdownAmount {
        BreakdownAmount { numerator: self.numerator.to_string(), denominator: self.denominator.to_string() }
    }
}

fn wide_mul(a: u128, b: u128) -> (u128, u128) {
    const LOW: u128 = u64::MAX as u128;
    let (a_lo, a_hi) = (a & LOW, a >> 64);
    let (b_lo, b_hi) = (b & LOW, b >> 64);
    let ll = a_lo * b_lo;
    let lh = a_lo * b_hi;
    let hl = a_hi * b_lo;
    let hh = a_hi * b_hi;
    let mid = (ll >> 64) + (lh & LOW) + (hl & LOW);
    let lo = (ll & LOW) | (mid << 64);
    let hi = hh + (lh >> 64) + (hl >> 64) + (mid >> 64);
    (hi, lo)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Amount {
    Known(Exact),
    Partial(Exact, String),
    NotRecorded(String),
    NotRecordedSigned(String),
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
const INEXACT: &str = "Too many fractions to add exactly";

#[derive(Clone, Debug)]
struct Sum {
    known: Option<Exact>,
    measured: u32,
    partial: Vec<String>,
    missing: Vec<String>,
    signed: bool,
    count: Option<u64>,
    last: Option<i32>,
}

impl Default for Sum {
    fn default() -> Self {
        Self { known: Some(Exact::ZERO), measured: 0, partial: Vec::new(), missing: Vec::new(), signed: false, count: None, last: None }
    }
}

impl Sum {
    fn plus(&mut self, value: Option<Exact>) {
        self.known = self.known.zip(value).and_then(|(held, value)| held.checked_add(value));
    }

    fn known(&self) -> Exact {
        self.known.unwrap_or(Exact::ZERO)
    }

    fn add(&mut self, amount: &Amount, count: Option<u64>, last: Option<i32>) {
        self.date(last);
        match amount {
            Amount::Known(value) => {
                self.plus(Some(*value));
                self.measured += 1;
                self.signed |= value.numerator() < 0;
            }
            Amount::Partial(value, reason) => {
                self.plus(Some(*value));
                self.measured += 1;
                self.signed |= value.numerator() < 0;
                remember(&mut self.partial, reason);
            }
            Amount::NotRecorded(reason) => remember(&mut self.missing, reason),
            Amount::NotRecordedSigned(reason) => {
                self.signed = true;
                remember(&mut self.missing, reason);
            }
        }
        self.add_count(count);
    }

    fn merge(&mut self, other: &Sum) {
        self.plus(other.known);
        self.measured += other.measured;
        self.signed |= other.signed;
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

    fn unbounded(&self) -> bool {
        self.signed && !self.missing.is_empty()
    }

    fn is_empty(&self) -> bool {
        self.measured == 0 && self.missing.is_empty()
    }

    fn state(&self) -> BreakdownState {
        if self.known.is_none() {
            return BreakdownState::NotRecorded;
        }
        match (self.measured, self.missing.is_empty(), self.partial.is_empty()) {
            (0, false, _) => BreakdownState::NotRecorded,
            (_, false, _) if self.signed => BreakdownState::NotRecorded,
            (_, false, _) => BreakdownState::AtLeast,
            (_, true, false) => BreakdownState::Partial,
            _ => BreakdownState::Known,
        }
    }

    fn reason(&self) -> Option<String> {
        let inexact = self.known.is_none().then_some(INEXACT);
        let reasons: Vec<&str> = inexact.into_iter().chain(self.missing.iter().chain(&self.partial).map(String::as_str)).collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }

    fn amount(&self) -> Option<BreakdownAmount> {
        self.known.filter(|_| (self.measured > 0 || self.missing.is_empty()) && !self.unbounded()).map(Exact::wire)
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
        _ if branch.sum.known().numerator() > 0 => 0,
        _ => 1,
    }
}

fn exact_share_of(part: &Sum, denominator: Option<Exact>) -> Option<f64> {
    if part.state() != BreakdownState::Known {
        return None;
    }
    let (Some(whole), Some(amount)) = (denominator, part.known) else {
        return None;
    };
    if whole.numerator() <= 0 || amount.numerator() <= 0 || amount.compare(whole).is_gt() {
        return None;
    }
    Some(amount.of(whole))
}

fn parts(children: Vec<Branch>, whole: Option<Exact>, parent: Option<Exact>) -> Vec<BreakdownPart> {
    let mut children = children;
    let has_negative = children.iter().any(|child| child.sum.known.is_some_and(|amount| amount.numerator() < 0));
    let whole = whole.filter(|_| !has_negative);
    let parent = parent.filter(|_| !has_negative);
    children.sort_by(|a, b| {
        class(a)
            .cmp(&class(b))
            .then_with(|| if class(a) < 2 { b.sum.known().compare(a.sum.known()) } else { std::cmp::Ordering::Equal })
            .then_with(|| a.node.ordinal.cmp(&b.node.ordinal))
    });
    children
        .into_iter()
        .map(|branch| {
            let own_whole = branch.sum.known.filter(|_| branch.sum.state() == BreakdownState::Known);
            let children = parts(branch.children, whole, own_whole);
            BreakdownPart {
                share: exact_share_of(&branch.sum, whole),
                parent_share: exact_share_of(&branch.sum, parent),
                key: branch.node.key,
                name: branch.node.name,
                name_missing: branch.node.name_missing,
                ordinal: branch.node.ordinal,
                other: branch.node.other,
                amount: branch.sum.amount(),
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

fn assemble(mut root: Branch, unrecorded: &Unrecorded, qualified: &[String], unwitnessed: bool, recorded: Option<&Amount>) -> Breakdown {
    root.split_rest();
    if let Some(total) = recorded {
        let (amount, state, reason, whole) = match total {
            Amount::Known(value) => (Some(value.wire()), BreakdownState::Known, None, Some(*value)),
            Amount::Partial(value, reason) => (Some(value.wire()), BreakdownState::Partial, Some(reason.clone()), None),
            Amount::NotRecorded(reason) | Amount::NotRecordedSigned(reason) => (None, BreakdownState::NotRecorded, Some(reason.clone()), None),
        };
        return Breakdown { total: BreakdownTotal { amount, state, reason }, parts: parts(root.children, whole, whole), unrecorded_hours: unrecorded.hours };
    }
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
    let whole = root.sum.known.filter(|_| state == BreakdownState::Known);
    Breakdown {
        total: BreakdownTotal { amount: if unwitnessed { None } else { root.sum.amount() }, state, reason },
        parts: parts(root.children, whole, whole),
        unrecorded_hours: unrecorded.hours,
    }
}

pub fn breakdown(leaves: &[Leaf], unrecorded: &Unrecorded) -> Breakdown {
    let mut root = Branch::new(Node::new("", "", 0));
    for leaf in leaves {
        let mut sum = Sum::default();
        sum.add(&leaf.amount, leaf.count, leaf.last);
        root.add(&leaf.path, &sum);
    }
    assemble(root, unrecorded, &[], false, None)
}

pub fn breakdown_qualified(leaves: &[Leaf], qualified: &[String]) -> Breakdown {
    let mut root = Branch::new(Node::new("", "", 0));
    for leaf in leaves {
        let mut sum = Sum::default();
        sum.add(&leaf.amount, leaf.count, leaf.last);
        root.add(&leaf.path, &sum);
    }
    assemble(root, &Unrecorded::default(), qualified, false, None)
}

pub fn breakdown_of(leaves: &[Leaf], total: &Amount) -> Breakdown {
    let mut root = Branch::new(Node::new("", "", 0));
    for leaf in leaves {
        let mut sum = Sum::default();
        sum.add(&leaf.amount, leaf.count, leaf.last);
        root.add(&leaf.path, &sum);
    }
    assemble(root, &Unrecorded::default(), &[], false, Some(total))
}
pub struct History {
    from: i32,
    to: i32,
    paths: Vec<Vec<Node>>,
    steps: Vec<Vec<(i32, Sum)>>,
    qualified: Vec<(i32, String)>,
    gaps: Vec<(i32, i32, String)>,
    changes: Vec<i32>,
}

pub struct HistoryBuilder {
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
    pub fn new(from: i32, to: i32) -> Self {
        Self { from, to, paths: Vec::new(), index: HashMap::new(), events: Vec::new(), qualified: Vec::new(), gaps: Vec::new(), changes: Vec::new(), dated: false }
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
        History { from: self.from, to: self.to, paths: self.paths, steps, qualified: self.qualified, gaps: self.gaps, changes: self.changes }
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
        (assemble(root, &unrecorded, &qualified, unwitnessed, None), if inside { hour } else { self.through(hour) })
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
