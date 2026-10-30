use crate::{
    budget::{Budget, Slab},
    invalid,
};
use std::{cmp::Ordering, io};

#[derive(Clone, Copy)]
pub struct Actor<'a> {
    pub identity: u64,
    pub tag: &'a str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Known(i128),
    Unknown,
    Excluded,
}
#[derive(Clone, Copy)]
struct Node {
    left: Option<usize>,
    right: Option<usize>,
    height: i16,
    size: usize,
    amount: Amount,
    published: Option<usize>,
}
pub struct RankBuilder<'a> {
    budget: &'a Budget,
    actors: &'a [Actor<'a>],
    nodes: Slab<'a, Node>,
    forward: Slab<'a, Option<usize>>,
    root: Option<usize>,
    descending: bool,
    include_unknown: bool,
    first: bool,
    hour: Option<i32>,
    failed: bool,
    visited: u64,
}
impl<'a> RankBuilder<'a> {
    pub fn new(
        budget: &'a Budget,
        actors: &'a [Actor<'a>],
        descending: bool,
        include_unknown: bool,
    ) -> io::Result<Self> {
        let mut nodes = Slab::new(budget, actors.len())?;
        let mut forward = Slab::new(budget, actors.len())?;
        for _ in actors {
            nodes.push(Node {
                left: None,
                right: None,
                height: 1,
                size: 1,
                amount: Amount::Unknown,
                published: None,
            })?;
            forward.push(None)?;
        }
        let mut value = Self {
            budget,
            actors,
            nodes,
            forward,
            root: None,
            descending,
            include_unknown,
            first: true,
            hour: None,
            failed: false,
            visited: 0,
        };
        if include_unknown {
            for i in 0..actors.len() {
                value.root = Some(value.insert(value.root, i));
            }
        }
        Ok(value)
    }
    fn included(&self, amount: Amount) -> bool {
        matches!(amount, Amount::Known(_)) || (self.include_unknown && amount == Amount::Unknown)
    }
    fn compare(&self, a: usize, b: usize) -> Ordering {
        let value = match (self.nodes[a].amount, self.nodes[b].amount) {
            (Amount::Known(a), Amount::Known(b)) => {
                if self.descending {
                    b.cmp(&a)
                } else {
                    a.cmp(&b)
                }
            }
            (Amount::Known(_), _) => Ordering::Less,
            (_, Amount::Known(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        };
        value
            .then_with(|| {
                self.actors[a]
                    .tag
                    .as_bytes()
                    .cmp(self.actors[b].tag.as_bytes())
            })
            .then_with(|| a.cmp(&b))
    }
    fn size(&self, node: Option<usize>) -> usize {
        node.map_or(0, |i| self.nodes[i].size)
    }
    fn height(&self, node: Option<usize>) -> i16 {
        node.map_or(0, |i| self.nodes[i].height)
    }
    fn refresh(&mut self, i: usize) {
        self.nodes[i].size = 1 + self.size(self.nodes[i].left) + self.size(self.nodes[i].right);
        self.nodes[i].height = 1 + self
            .height(self.nodes[i].left)
            .max(self.height(self.nodes[i].right));
    }
    fn left(&mut self, i: usize) -> usize {
        let right = self.nodes[i].right.expect("AVL right child");
        self.nodes[i].right = self.nodes[right].left;
        self.nodes[right].left = Some(i);
        self.refresh(i);
        self.refresh(right);
        right
    }
    fn right(&mut self, i: usize) -> usize {
        let left = self.nodes[i].left.expect("AVL left child");
        self.nodes[i].left = self.nodes[left].right;
        self.nodes[left].right = Some(i);
        self.refresh(i);
        self.refresh(left);
        left
    }
    fn balance(&mut self, i: usize) -> usize {
        self.refresh(i);
        let delta = self.height(self.nodes[i].left) - self.height(self.nodes[i].right);
        if delta > 1 {
            let left = self.nodes[i].left.expect("AVL left child");
            if self.height(self.nodes[left].left) < self.height(self.nodes[left].right) {
                let rotated = self.left(left);
                self.nodes[i].left = Some(rotated);
            }
            return self.right(i);
        }
        if delta < -1 {
            let right = self.nodes[i].right.expect("AVL right child");
            if self.height(self.nodes[right].right) < self.height(self.nodes[right].left) {
                let rotated = self.right(right);
                self.nodes[i].right = Some(rotated);
            }
            return self.left(i);
        }
        i
    }
    fn insert(&mut self, root: Option<usize>, i: usize) -> usize {
        let Some(root) = root else {
            self.nodes[i].left = None;
            self.nodes[i].right = None;
            self.nodes[i].size = 1;
            self.nodes[i].height = 1;
            return i;
        };
        if self.compare(i, root) == Ordering::Less {
            let left = self.insert(self.nodes[root].left, i);
            self.nodes[root].left = Some(left);
        } else {
            let right = self.insert(self.nodes[root].right, i);
            self.nodes[root].right = Some(right);
        }
        self.balance(root)
    }
    fn remove(&mut self, root: Option<usize>, i: usize) -> Option<usize> {
        let root = root?;
        match self.compare(i, root) {
            Ordering::Less => {
                let child = self.remove(self.nodes[root].left, i);
                self.nodes[root].left = child;
            }
            Ordering::Greater => {
                let child = self.remove(self.nodes[root].right, i);
                self.nodes[root].right = child;
            }
            Ordering::Equal => {
                let (left, right) = (self.nodes[root].left, self.nodes[root].right);
                if left.is_none() {
                    return right;
                }
                if right.is_none() {
                    return left;
                }
                let mut next = right.expect("AVL successor");
                while let Some(left) = self.nodes[next].left {
                    next = left;
                }
                let right = self.remove(right, next);
                self.nodes[next].left = left;
                self.nodes[next].right = right;
                return Some(self.balance(next));
            }
        }
        Some(self.balance(root))
    }
    pub fn position(&self, handle: usize) -> io::Result<Option<usize>> {
        if handle >= self.nodes.len() {
            return Err(invalid("unknown rank actor handle"));
        }
        if !self.included(self.nodes[handle].amount) {
            return Ok(None);
        }
        let mut at = self.root;
        let mut base = 0;
        while let Some(i) = at {
            match self.compare(handle, i) {
                Ordering::Less => at = self.nodes[i].left,
                Ordering::Equal => return Ok(Some(base + self.size(self.nodes[i].left))),
                Ordering::Greater => {
                    base += self.size(self.nodes[i].left) + 1;
                    at = self.nodes[i].right;
                }
            }
        }
        Err(invalid("rank tree lost an included actor"))
    }
    pub fn changes(
        &mut self,
        hour: i32,
        changes: &[(usize, Amount)],
        mut position: impl FnMut(usize, Option<u64>) -> io::Result<()>,
        mut actor: impl FnMut(u64, Option<usize>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.visited = 0;
        if self.failed {
            return Err(invalid("discard rank builder after a failed output sink"));
        }
        if self.hour.is_some_and(|previous| hour <= previous) {
            return Err(invalid("rank batches must have distinct ascending hours"));
        }
        let capacity = changes
            .len()
            .checked_mul(3)
            .and_then(|n| n.checked_add(2))
            .ok_or_else(engine_allocation::oom)?;
        let mut handles = Slab::new(self.budget, changes.len())?;
        let mut old = Slab::new(self.budget, changes.len())?;
        let mut intervals = Slab::new(self.budget, capacity)?;
        let mut before = Slab::new(self.budget, changes.len())?;
        let mut after = Slab::new(self.budget, changes.len())?;
        for &(handle, _) in changes {
            if handle >= self.nodes.len() {
                return Err(invalid("unknown rank actor handle"));
            }
            handles.push(handle)?;
            old.push(self.position(handle)?)?;
        }
        handles.sort_unstable();
        if handles.windows(2).any(|w| w[0] == w[1]) {
            return Err(invalid("an actor changed twice in one rank batch"));
        }
        let old_size = self.size(self.root);
        self.failed = true;
        for &(handle, amount) in changes {
            if self.included(self.nodes[handle].amount) {
                self.root = self.remove(self.root, handle);
            }
            self.nodes[handle].amount = amount;
            if self.included(amount) {
                self.root = Some(self.insert(self.root, handle));
            }
        }
        let new_size = self.size(self.root);
        if self.first {
            if old_size.max(new_size) > 0 {
                intervals.push((0, old_size.max(new_size) - 1))?;
            }
            self.first = false;
        }
        for (index, &(handle, _)) in changes.iter().enumerate() {
            let next = self.position(handle)?;
            if let Some(rank) = old[index] {
                before.push(rank)?;
            }
            if let Some(rank) = next {
                after.push(rank)?;
                intervals.push((rank, rank))?;
            }
            if next.is_none() && self.nodes[handle].published.is_some() {
                actor(self.actors[handle].identity, None)?;
                self.nodes[handle].published = None;
            }
        }
        before.sort_unstable();
        after.sort_unstable();
        let unchanged = old_size - before.len();
        if unchanged != new_size - after.len() {
            return Err(invalid("rank batch changed the unmodified roster"));
        }
        let (mut old_at, mut new_at, mut ordinal) = (0, 0, 0);
        while ordinal < unchanged {
            while old_at < before.len() && before[old_at] - old_at <= ordinal {
                old_at += 1;
            }
            while new_at < after.len() && after[new_at] - new_at <= ordinal {
                new_at += 1;
            }
            let end = before
                .get(old_at)
                .map_or(unchanged, |rank| *rank - old_at)
                .min(after.get(new_at).map_or(unchanged, |rank| *rank - new_at))
                .min(unchanged);
            if end <= ordinal {
                return Err(invalid("rank displacement sweep did not advance"));
            }
            if old_at != new_at {
                intervals.push((ordinal + new_at, end - 1 + new_at))?;
            }
            ordinal = end;
        }
        if old_size > new_size {
            intervals.push((new_size, old_size - 1))?;
        }
        intervals.sort_unstable();
        let mut at = 0;
        while at < intervals.len() {
            let (first, mut last) = intervals[at];
            at += 1;
            while at < intervals.len() && intervals[at].0 <= last.saturating_add(1) {
                last = last.max(intervals[at].1);
                at += 1;
            }
            self.emit_range(self.root, 0, first, last, &mut position, &mut actor)?;
            for rank in new_size.max(first)..=last {
                if rank < self.forward.len() && self.forward[rank].is_some() {
                    position(rank, None)?;
                    self.forward[rank] = None;
                }
            }
        }
        self.hour = Some(hour);
        self.failed = false;
        Ok(())
    }
    fn emit_range(
        &mut self,
        node: Option<usize>,
        base: usize,
        first: usize,
        last: usize,
        position: &mut impl FnMut(usize, Option<u64>) -> io::Result<()>,
        actor: &mut impl FnMut(u64, Option<usize>) -> io::Result<()>,
    ) -> io::Result<()> {
        let Some(i) = node else {
            return Ok(());
        };
        self.visited = crate::add(self.visited, 1)?;
        if base > last || base + self.nodes[i].size <= first {
            return Ok(());
        }
        let rank = base + self.size(self.nodes[i].left);
        self.emit_range(self.nodes[i].left, base, first, last, position, actor)?;
        if first <= rank && rank <= last {
            if self.forward[rank] != Some(i) {
                position(rank, Some(self.actors[i].identity))?;
                self.forward[rank] = Some(i);
            }
            if self.nodes[i].published != Some(rank) {
                actor(self.actors[i].identity, Some(rank))?;
                self.nodes[i].published = Some(rank);
            }
        }
        self.emit_range(self.nodes[i].right, rank + 1, first, last, position, actor)
    }
    pub fn visited(&self) -> u64 {
        self.visited
    }
}
