use crate::{Coverage, Hour, Hours, Memory, Missing, Owner, PageInfo, Reservation};
use std::{io, mem::size_of, sync::Arc};

struct RunEdge<T> {
    entry: Arc<T>,
    index: usize,
    hours: Hours,
    accepted_before: u64,
}

impl<T: Entry> RunEdge<T> {
    fn as_ref(&self) -> RunRef<'_, T> {
        RunRef {
            entry: &self.entry,
            index: self.index,
            hours: self.hours,
            accepted_before: self.accepted_before,
        }
    }
}

struct RunRef<'a, T> {
    entry: &'a Arc<T>,
    index: usize,
    hours: Hours,
    accepted_before: u64,
}

impl<T> Clone for RunRef<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for RunRef<'_, T> {}

impl<'a, T: Entry> RunRef<'a, T> {
    fn from_entry(entry: &'a Arc<T>, index: usize) -> Self {
        let item = &entry.coverage()[index];
        Self {
            entry,
            index,
            hours: item.hours,
            accepted_before: item.accepted_before,
        }
    }

    fn clip(entry: &'a Arc<T>, index: usize, span: Hours) -> Option<Self> {
        let item = &entry.coverage()[index];
        let hours = item.hours.intersection(span)?;
        let accepted_before = if hours.first == item.hours.first {
            item.accepted_before
        } else {
            item.accepted_through(hours.first - 1)
        };
        Some(Self {
            entry,
            index,
            hours,
            accepted_before,
        })
    }

    fn missing(&self) -> Option<&'a Missing> {
        self.entry.coverage()[self.index].missing.as_ref()
    }

    fn accepted_through(&self, hour: Hour) -> u64 {
        self.accepted_before
            + if self.missing().is_none() && hour >= self.hours.first {
                (i64::from(hour.min(self.hours.last)) - i64::from(self.hours.first) + 1) as u64
            } else {
                0
            }
    }

    fn can_merge(&self, next: &Self) -> bool {
        self.missing() == next.missing()
            && self.accepted_through(next.hours.first.saturating_sub(1)) == next.accepted_before
    }

    fn to_edge(self) -> RunEdge<T> {
        RunEdge {
            entry: self.entry.clone(),
            index: self.index,
            hours: self.hours,
            accepted_before: self.accepted_before,
        }
    }
}

struct SpanSummary<'a, T> {
    runs: usize,
    first: RunRef<'a, T>,
    last: RunRef<'a, T>,
}

impl<T> Clone for SpanSummary<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for SpanSummary<'_, T> {}

fn remaining<T>(span: Hours, covered: Option<SpanSummary<'_, T>>) -> Option<Hours> {
    let Some(summary) = covered else {
        return Some(span);
    };
    if summary.last.hours.last >= span.last {
        return None;
    }
    Some(Hours {
        first: span.first.max(summary.last.hours.last + 1),
        last: span.last,
    })
}

fn combine<'a, T: Entry>(
    left: Option<SpanSummary<'a, T>>,
    right: Option<SpanSummary<'a, T>>,
) -> Option<SpanSummary<'a, T>> {
    let Some(l) = left else {
        return right;
    };
    let Some(r) = right else {
        return Some(l);
    };
    if i64::from(r.first.hours.first) == i64::from(l.last.hours.last) + 1
        && l.last.can_merge(&r.first)
    {
        let merged = RunRef {
            entry: l.last.entry,
            index: l.last.index,
            hours: Hours {
                first: l.last.hours.first,
                last: r.first.hours.last,
            },
            accepted_before: l.last.accepted_before,
        };
        Some(SpanSummary {
            runs: l.runs + r.runs - 1,
            first: if l.runs == 1 { merged } else { l.first },
            last: if r.runs == 1 { merged } else { r.last },
        })
    } else {
        Some(SpanSummary {
            runs: l.runs + r.runs,
            first: l.first,
            last: r.last,
        })
    }
}

pub(crate) fn can_merge(previous: &Coverage, next: &Coverage) -> bool {
    previous.missing == next.missing
        && previous.accepted_through(next.hours.first.saturating_sub(1)) == next.accepted_before
}

pub(crate) fn coverage_runs(coverage: &[Coverage]) -> usize {
    let mut runs = 0;
    let mut previous: Option<&Coverage> = None;
    for item in coverage {
        if previous.is_none_or(|prev| !can_merge(prev, item)) {
            runs += 1;
        }
        previous = Some(item);
    }
    runs
}

fn whole_node_summary<T: Entry>(n: &Node<T>) -> Option<SpanSummary<'_, T>> {
    Some(SpanSummary {
        runs: n.coverage_runs,
        first: n.coverage_first.as_ref()?.as_ref(),
        last: n.coverage_last.as_ref()?.as_ref(),
    })
}

fn summarize_entry<'a, T: Entry>(entry: &'a Arc<T>, span: Hours) -> Option<SpanSummary<'a, T>> {
    let hours = entry.info().hours;
    if hours.last < span.first || hours.first > span.last {
        return None;
    }
    let items = entry.coverage();
    if items.is_empty() {
        return None;
    }
    if hours.first >= span.first && hours.last <= span.last {
        let mut first = RunRef::from_entry(entry, 0);
        let mut last = RunRef::from_entry(entry, items.len() - 1);
        let runs = entry.coverage_runs();
        if runs == 1 {
            let whole = RunRef {
                entry,
                index: 0,
                hours: Hours {
                    first: first.hours.first,
                    last: last.hours.last,
                },
                accepted_before: first.accepted_before,
            };
            first = whole;
            last = whole;
        }
        return Some(SpanSummary { runs, first, last });
    }
    let start = items.partition_point(|item| item.hours.last < span.first);
    let end = items.partition_point(|item| item.hours.first <= span.last);
    let mut summary: Option<SpanSummary<'a, T>> = None;
    for index in start..end {
        if let Some(run) = RunRef::clip(entry, index, span) {
            summary = combine(
                summary,
                Some(SpanSummary {
                    runs: 1,
                    first: run,
                    last: run,
                }),
            );
        }
    }
    summary
}

fn summarize_node<'a, T: Entry>(
    link: &'a Link<T>,
    span: Hours,
) -> Option<SpanSummary<'a, T>> {
    let n = link.as_deref()?;
    if n.last.hours.last < span.first || n.first.hours.first > span.last {
        return None;
    }
    if n.first.hours.first >= span.first && n.last.hours.last <= span.last {
        return whole_node_summary(n);
    }
    let left = summarize_node(&n.left, span);
    let Some(mid_span) = remaining(span, left) else {
        return left;
    };
    let mid = summarize_entry(&n.value, mid_span);
    let left_mid = combine(left, mid);
    let Some(right_span) = remaining(span, left_mid) else {
        return left_mid;
    };
    let right = summarize_node(&n.right, right_span);
    combine(left_mid, right)
}

pub(crate) trait Entry {
    fn info(&self) -> PageInfo;
    fn bytes(&self) -> usize;
    fn coverage(&self) -> &[Coverage] {
        &[]
    }
    fn coverage_runs(&self) -> usize {
        0
    }
}

type Link<T> = Option<Arc<Node<T>>>;
struct Node<T> {
    value: Arc<T>,
    left: Link<T>,
    right: Link<T>,
    coverage_first: Option<RunEdge<T>>,
    coverage_last: Option<RunEdge<T>>,
    height: usize,
    count: usize,
    bytes: usize,
    directory_bytes: usize,
    coverage_runs: usize,
    first: PageInfo,
    last: PageInfo,
    _memory: Reservation,
}

pub(crate) struct Tree<T: Entry>(Link<T>);
impl<T: Entry> Clone for Tree<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: Entry> Default for Tree<T> {
    fn default() -> Self {
        Self(None)
    }
}
fn height<T: Entry>(link: &Link<T>) -> usize {
    link.as_ref().map_or(0, |n| n.height)
}
fn bytes<T: Entry>(link: &Link<T>) -> usize {
    link.as_ref().map_or(0, |n| n.bytes)
}
fn node<T: Entry>(
    value: Arc<T>,
    left: Link<T>,
    right: Link<T>,
    memory: &Memory,
) -> io::Result<Arc<Node<T>>> {
    let reservation = memory.reserve(
        Owner::Metadata,
        size_of::<Node<T>>() + 2 * size_of::<usize>(),
    )?;
    let (coverage_runs, coverage_first, coverage_last) = {
        let left_summary = left.as_deref().and_then(whole_node_summary);
        let value_hours = value.info().hours;
        let mid_summary =
            remaining(value_hours, left_summary).and_then(|span| summarize_entry(&value, span));
        let left_mid = combine(left_summary, mid_summary);
        let right_summary = right.as_deref().and_then(|r| {
            let r_span = Hours {
                first: r.first.hours.first,
                last: r.last.hours.last,
            };
            remaining(r_span, left_mid).and_then(|span| summarize_node(&right, span))
        });
        match combine(left_mid, right_summary) {
            Some(summary) => (
                summary.runs,
                Some(summary.first.to_edge()),
                Some(summary.last.to_edge()),
            ),
            None => (0, None, None),
        }
    };
    Ok(Arc::new(Node {
        height: 1 + height(&left).max(height(&right)),
        count: 1 + left.as_ref().map_or(0, |n| n.count) + right.as_ref().map_or(0, |n| n.count),
        bytes: value.bytes() + bytes(&left) + bytes(&right),
        directory_bytes: value.info().resident_bytes
            + left.as_ref().map_or(0, |n| n.directory_bytes)
            + right.as_ref().map_or(0, |n| n.directory_bytes),
        coverage_runs,
        first: left.as_ref().map_or(value.info(), |n| n.first),
        last: right.as_ref().map_or(value.info(), |n| n.last),
        coverage_first,
        coverage_last,
        value,
        left,
        right,
        _memory: reservation,
    }))
}
fn join<T: Entry>(
    value: Arc<T>,
    left: Link<T>,
    right: Link<T>,
    memory: &Memory,
) -> io::Result<Arc<Node<T>>> {
    if height(&left) > height(&right) + 1 {
        let l = left.as_ref().unwrap();
        let next = join(value, l.right.clone(), right, memory)?;
        return balance(l.value.clone(), l.left.clone(), Some(next), memory);
    }
    if height(&right) > height(&left) + 1 {
        let r = right.as_ref().unwrap();
        let next = join(value, left, r.left.clone(), memory)?;
        return balance(r.value.clone(), Some(next), r.right.clone(), memory);
    }
    node(value, left, right, memory)
}
fn balance<T: Entry>(
    value: Arc<T>,
    left: Link<T>,
    right: Link<T>,
    memory: &Memory,
) -> io::Result<Arc<Node<T>>> {
    if height(&left) > height(&right) + 1 {
        let l = left.as_ref().unwrap();
        if height(&l.left) >= height(&l.right) {
            let r = node(value, l.right.clone(), right, memory)?;
            return node(l.value.clone(), l.left.clone(), Some(r), memory);
        }
        let pivot = l.right.as_ref().unwrap();
        let a = node(l.value.clone(), l.left.clone(), pivot.left.clone(), memory)?;
        let b = node(value, pivot.right.clone(), right, memory)?;
        return node(pivot.value.clone(), Some(a), Some(b), memory);
    }
    if height(&right) > height(&left) + 1 {
        let r = right.as_ref().unwrap();
        if height(&r.right) >= height(&r.left) {
            let l = node(value, left, r.left.clone(), memory)?;
            return node(r.value.clone(), Some(l), r.right.clone(), memory);
        }
        let pivot = r.left.as_ref().unwrap();
        let a = node(value, left, pivot.left.clone(), memory)?;
        let b = node(
            r.value.clone(),
            pivot.right.clone(),
            r.right.clone(),
            memory,
        )?;
        return node(pivot.value.clone(), Some(a), Some(b), memory);
    }
    node(value, left, right, memory)
}
fn insert<T: Entry>(root: &Link<T>, value: Arc<T>, memory: &Memory) -> io::Result<Arc<Node<T>>> {
    let Some(n) = root else {
        return node(value, None, None, memory);
    };
    match value.info().id.cmp(&n.value.info().id) {
        std::cmp::Ordering::Less => join(
            n.value.clone(),
            Some(insert(&n.left, value, memory)?),
            n.right.clone(),
            memory,
        ),
        std::cmp::Ordering::Greater => join(
            n.value.clone(),
            n.left.clone(),
            Some(insert(&n.right, value, memory)?),
            memory,
        ),
        std::cmp::Ordering::Equal => node(value, n.left.clone(), n.right.clone(), memory),
    }
}
fn retain<T: Entry>(root: &Link<T>, hours: Hours, memory: &Memory) -> io::Result<Link<T>> {
    let Some(n) = root else {
        return Ok(None);
    };
    if n.last.hours.last < hours.first || n.first.hours.first > hours.last {
        return Ok(None);
    }
    if n.first.hours.last >= hours.first && n.last.hours.first <= hours.last {
        return Ok(root.clone());
    }
    if n.value.info().hours.last < hours.first {
        return retain(&n.right, hours, memory);
    }
    if n.value.info().hours.first > hours.last {
        return retain(&n.left, hours, memory);
    }
    Ok(Some(join(
        n.value.clone(),
        retain(&n.left, hours, memory)?,
        retain(&n.right, hours, memory)?,
        memory,
    )?))
}
impl<T: Entry> Tree<T> {
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.count)
    }
    pub fn filter(&mut self, keep: impl Fn(&T) -> bool, memory: &Memory) -> io::Result<()> {
        fn filter<T: Entry>(
            root: &Link<T>,
            keep: &impl Fn(&T) -> bool,
            memory: &Memory,
        ) -> io::Result<Link<T>> {
            let Some(n) = root else {
                return Ok(None);
            };
            let left = filter(&n.left, keep, memory)?;
            let right = filter(&n.right, keep, memory)?;
            if keep(&n.value) {
                let same = |a: &Link<T>, b: &Link<T>| match (a, b) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                };
                if same(&left, &n.left) && same(&right, &n.right) {
                    return Ok(root.clone());
                }
                return Ok(Some(join(n.value.clone(), left, right, memory)?));
            }
            fn merge<T: Entry>(
                left: Link<T>,
                right: Link<T>,
                memory: &Memory,
            ) -> io::Result<Link<T>> {
                let Some(r) = right else {
                    return Ok(left);
                };
                let next = merge(left, r.left.clone(), memory)?;
                Ok(Some(join(r.value.clone(), next, r.right.clone(), memory)?))
            }
            merge(left, right, memory)
        }
        self.0 = filter(&self.0, &keep, memory)?;
        Ok(())
    }
    pub fn insert(&mut self, value: Arc<T>, memory: &Memory) -> io::Result<()> {
        self.0 = Some(insert(&self.0, value, memory)?);
        Ok(())
    }
    pub fn retain(&mut self, hours: Hours, memory: &Memory) -> io::Result<()> {
        self.0 = retain(&self.0, hours, memory)?;
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        bytes(&self.0)
    }
    pub fn directory_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.directory_bytes)
    }
    pub fn coverage_runs_for(&self, span: Hours) -> usize {
        summarize_node(&self.0, span).map_or(0, |summary| summary.runs)
    }
    pub fn first(&self) -> Option<PageInfo> {
        self.0.as_ref().map(|n| n.first)
    }
    pub fn last(&self) -> Option<PageInfo> {
        self.0.as_ref().map(|n| n.last)
    }
    pub fn get(&self, id: u64) -> Option<&Arc<T>> {
        let mut node = self.0.as_deref();
        while let Some(n) = node {
            match id.cmp(&n.value.info().id) {
                std::cmp::Ordering::Less => node = n.left.as_deref(),
                std::cmp::Ordering::Greater => node = n.right.as_deref(),
                std::cmp::Ordering::Equal => return Some(&n.value),
            }
        }
        None
    }
    pub fn iter(&self, hours: Hours) -> Iter<'_, T> {
        Iter {
            tree: self,
            hours,
            front: None,
            back: None,
            done: false,
        }
    }
}
pub(crate) struct Iter<'a, T: Entry> {
    tree: &'a Tree<T>,
    hours: Hours,
    front: Option<u64>,
    back: Option<u64>,
    done: bool,
}
impl<'a, T: Entry> Iter<'a, T> {
    fn take(&mut self, reverse: bool) -> Option<&'a T> {
        if self.done {
            return None;
        }
        let mut node = self.tree.0.as_deref();
        let mut candidate = None;
        while let Some(n) = node {
            let info = n.value.info();
            if info.hours.last < self.hours.first || self.front.is_some_and(|id| info.id <= id) {
                node = n.right.as_deref();
            } else if info.hours.first > self.hours.last
                || self.back.is_some_and(|id| info.id >= id)
            {
                node = n.left.as_deref();
            } else {
                candidate = Some(&*n.value);
                node = if reverse {
                    n.right.as_deref()
                } else {
                    n.left.as_deref()
                };
            }
        }
        if let Some(value) = candidate {
            if reverse {
                self.back = Some(value.info().id);
            } else {
                self.front = Some(value.info().id);
            }
        } else {
            self.done = true;
        }
        candidate
    }
}
impl<'a, T: Entry> Iterator for Iter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        self.take(false)
    }
}
impl<T: Entry> DoubleEndedIterator for Iter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.take(true)
    }
}
