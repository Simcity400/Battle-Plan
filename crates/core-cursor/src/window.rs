use crate::{Coverage, Hour, Hours, Memory, Missing, Owner, PageInfo, Reservation};
use std::{io, mem::size_of, sync::Arc};

#[derive(Clone, Copy)]
struct RunRef<'a> {
    hours: Hours,
    missing: Option<&'a Missing>,
    accepted_before: u64,
}

impl<'a> RunRef<'a> {
    fn from_coverage(item: &'a Coverage) -> Self {
        Self {
            hours: item.hours,
            missing: item.missing.as_ref(),
            accepted_before: item.accepted_before,
        }
    }

    fn clip(item: &'a Coverage, span: Hours) -> Option<Self> {
        Self::from_coverage(item).clip_ref(span)
    }

    fn clip_ref(self, span: Hours) -> Option<Self> {
        let hours = self.hours.intersection(span)?;
        let accepted_before = if hours.first == self.hours.first {
            self.accepted_before
        } else {
            self.accepted_through(hours.first - 1)
        };
        Some(Self {
            hours,
            missing: self.missing,
            accepted_before,
        })
    }

    fn accepted_through(&self, hour: Hour) -> u64 {
        self.accepted_before
            + if self.missing.is_none() && hour >= self.hours.first {
                (i64::from(hour.min(self.hours.last)) - i64::from(self.hours.first) + 1) as u64
            } else {
                0
            }
    }

    fn can_merge(&self, next: &Self) -> bool {
        self.missing == next.missing
            && self.accepted_through(next.hours.first.saturating_sub(1)) == next.accepted_before
    }
}

#[derive(Clone, Copy)]
struct SpanSummary<'a> {
    runs: usize,
    first: RunRef<'a>,
    last: RunRef<'a>,
}

fn combine<'a>(
    left: Option<SpanSummary<'a>>,
    right: Option<SpanSummary<'a>>,
) -> Option<SpanSummary<'a>> {
    let Some(l) = left else {
        return right;
    };
    let Some(r) = right else {
        return Some(l);
    };
    let merged_end = if i64::from(r.first.hours.first) == i64::from(l.last.hours.last) + 1 {
        l.last.can_merge(&r.first).then_some(r.first.hours.last)
    } else if r.first.hours.first <= l.last.hours.last {
        if r.runs == 1 && r.last.hours.last <= l.last.hours.last {
            return Some(l);
        } else if r.first.hours.last > l.last.hours.last {
            r.first
                .clip_ref(Hours {
                    first: l.last.hours.last + 1,
                    last: r.first.hours.last,
                })
                .filter(|clipped| l.last.can_merge(clipped))
                .map(|clipped| clipped.hours.last)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(end) = merged_end {
        let merged = RunRef {
            hours: Hours {
                first: l.last.hours.first,
                last: end,
            },
            missing: l.last.missing,
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
    RunRef::from_coverage(previous).can_merge(&RunRef::from_coverage(next))
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

fn boundary_reduction(left: &[Coverage], right: &[Coverage]) -> usize {
    let (Some(prev), Some(next)) = (left.last(), right.first()) else {
        return 0;
    };
    let prev = RunRef::from_coverage(prev);
    if i64::from(next.hours.first) == i64::from(prev.hours.last) + 1 {
        usize::from(prev.can_merge(&RunRef::from_coverage(next)))
    } else if next.hours.first <= prev.hours.last {
        if right.len() == 1 && next.hours.last <= prev.hours.last {
            1
        } else if next.hours.last > prev.hours.last {
            RunRef::clip(
                next,
                Hours {
                    first: prev.hours.last + 1,
                    last: next.hours.last,
                },
            )
            .map_or(0, |clipped| usize::from(prev.can_merge(&clipped)))
        } else {
            0
        }
    } else {
        0
    }
}

fn summarize_entry<'a, T: Entry>(entry: &'a T, span: Hours) -> Option<SpanSummary<'a>> {
    let hours = entry.info().hours;
    if hours.last < span.first || hours.first > span.last {
        return None;
    }
    let items = entry.coverage();
    if items.is_empty() {
        return None;
    }
    if hours.first >= span.first && hours.last <= span.last {
        let mut first = RunRef::from_coverage(items.first()?);
        let mut last = RunRef::from_coverage(items.last()?);
        let runs = entry.coverage_runs();
        if runs == 1 {
            let whole = RunRef {
                hours: Hours {
                    first: first.hours.first,
                    last: last.hours.last,
                },
                missing: first.missing,
                accepted_before: first.accepted_before,
            };
            first = whole;
            last = whole;
        }
        return Some(SpanSummary { runs, first, last });
    }
    let start = items.partition_point(|item| item.hours.last < span.first);
    let end = items.partition_point(|item| item.hours.first <= span.last);
    let mut summary: Option<SpanSummary<'a>> = None;
    for item in &items[start..end] {
        if let Some(run) = RunRef::clip(item, span) {
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

fn summarize_node<'a, T: Entry>(link: &'a Link<T>, span: Hours) -> Option<SpanSummary<'a>> {
    let n = link.as_deref()?;
    if n.last.hours.last < span.first || n.first.hours.first > span.last {
        return None;
    }
    if n.first.hours.first >= span.first && n.last.hours.last <= span.last {
        let mut first = RunRef::from_coverage(n.first_entry.coverage().first()?);
        let mut last = RunRef::from_coverage(n.last_entry.coverage().last()?);
        if n.coverage_runs == 1 {
            let whole = RunRef {
                hours: Hours {
                    first: first.hours.first,
                    last: last.hours.last,
                },
                missing: first.missing,
                accepted_before: first.accepted_before,
            };
            first = whole;
            last = whole;
        }
        return Some(SpanSummary {
            runs: n.coverage_runs,
            first,
            last,
        });
    }
    let left = summarize_node(&n.left, span);
    let mid = summarize_entry(&*n.value, span);
    let right = summarize_node(&n.right, span);
    combine(combine(left, mid), right)
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
    first_entry: Arc<T>,
    last_entry: Arc<T>,
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
    let left_runs = left.as_ref().map_or(0, |n| n.coverage_runs);
    let right_runs = right.as_ref().map_or(0, |n| n.coverage_runs);
    let left_mid = left
        .as_ref()
        .map_or(0, |n| boundary_reduction(n.last_entry.coverage(), value.coverage()));
    let mid_right = right
        .as_ref()
        .map_or(0, |n| boundary_reduction(value.coverage(), n.first_entry.coverage()));
    let coverage_runs = (left_runs + value.coverage_runs() + right_runs)
        .saturating_sub(left_mid + mid_right);
    let first_entry = left
        .as_ref()
        .map_or_else(|| value.clone(), |n| n.first_entry.clone());
    let last_entry = right
        .as_ref()
        .map_or_else(|| value.clone(), |n| n.last_entry.clone());
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
        first_entry,
        last_entry,
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
