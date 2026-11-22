use crate::{Coverage, Hours, Memory, Owner, PageInfo, Reservation};
use std::{io, mem::size_of, sync::Arc};

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

fn boundary_reduction(left: &[Coverage], right: &[Coverage]) -> usize {
    let (Some(prev), Some(next)) = (left.last(), right.first()) else {
        return 0;
    };
    if i64::from(next.hours.first) == i64::from(prev.hours.last) + 1 {
        usize::from(can_merge(prev, next))
    } else if next.hours.first <= prev.hours.last {
        if right.len() == 1 && next.hours.last <= prev.hours.last {
            1
        } else if next.hours.last > prev.hours.last {
            crate::reference::clip(
                next,
                Hours {
                    first: prev.hours.last + 1,
                    last: next.hours.last,
                },
            )
            .map_or(0, |clipped| usize::from(can_merge(prev, &clipped)))
        } else {
            0
        }
    } else {
        0
    }
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
    pub fn coverage_runs(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.coverage_runs)
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
