//! Deferred native reserve credits, keyed by complete source division handle.
//! Only construction/growth allocates; per-source lists recycle on deletion.
use super::queue_index::FixedIndex;
const NONE: usize = usize::MAX;

#[derive(Clone, Copy, Default)]
pub(super) struct Net {
    pub variant: u64,
    pub country: u32,
    // Deferred deficit plus actual credit retains the exact source-time sum.
    // Merge uses only amount; no reserve credit is retained for that operation.
    pub amount: i128,
    pub credited: i128,
}
#[derive(Clone, Copy)]
struct Node {
    value: Net,
    next: usize,
}
#[derive(Clone, Copy)]
struct Head {
    army: usize,
    country: u32,
    first: usize,
    state: PoolState,
    revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PoolState {
    Bookkeeping,
    Live,
    Unsettled,
}
pub(super) struct Credits {
    heads: FixedIndex<u64, Head>,
    nodes: Vec<Node>,
    unused: usize,
    free: usize,
}
impl Credits {
    pub(super) fn new() -> Self {
        Self {
            heads: FixedIndex::new(),
            nodes: Vec::new(),
            unused: 0,
            free: NONE,
        }
    }
    pub(super) fn try_new(divisions: usize, entries: usize) -> std::io::Result<Self> {
        let heads = FixedIndex::try_new(divisions)?;
        let mut nodes = engine::allocation::vec(entries)?;
        nodes.resize(
            entries,
            Node {
                value: Net::default(),
                next: NONE,
            },
        );
        Ok(Self {
            heads,
            nodes,
            unused: 0,
            free: NONE,
        })
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.heads.allocated_bytes() + self.nodes.capacity() * std::mem::size_of::<Node>()
    }
    pub(super) fn clear(&mut self) -> Result<(), &'static str> {
        self.heads.clear()?;
        self.unused = 0;
        self.free = NONE;
        Ok(())
    }
    pub(super) fn copy_to(&self, target: &mut Self) -> Result<(), &'static str> {
        for (&handle, &head) in self.heads.iter() {
            target
                .heads
                .insert(
                    handle,
                    Head {
                        first: NONE,
                        ..head
                    },
                )
                .map_err(|_| "equipment return credit source capacity too short")?;
            self.visit(handle, head.army, head.country, |value| {
                target.push(handle, value)
            })?;
        }
        Ok(())
    }
    pub(super) fn begin(
        &mut self,
        handle: u64,
        army: usize,
        country: u32,
        revision: u64,
    ) -> Result<(), &'static str> {
        if revision == u64::MAX {
            return Err("equipment return continuity sequence exhausted");
        }
        self.remove(handle);
        self.heads
            .insert(
                handle,
                Head {
                    army,
                    country,
                    first: NONE,
                    state: PoolState::Bookkeeping,
                    revision,
                },
            )
            .map_err(|_| "equipment return credit source capacity too short")
    }
    pub(super) fn push(&mut self, handle: u64, value: Net) -> Result<(), &'static str> {
        let head = self
            .heads
            .get_mut(&handle)
            .ok_or("equipment return credit source absent")?;
        let node = if self.free != NONE {
            let node = self.free;
            self.free = self.nodes[node].next;
            node
        } else if self.unused < self.nodes.len() {
            let node = self.unused;
            self.unused += 1;
            node
        } else {
            return Err("equipment return credit entries capacity too short");
        };
        self.nodes[node] = Node {
            value,
            next: head.first,
        };
        head.first = node;
        Ok(())
    }
    pub(super) fn is_empty(&self) -> bool {
        self.heads.is_empty()
    }
    pub(super) fn has_unsettled(&self) -> bool {
        self.heads
            .iter()
            .any(|(_, head)| head.state == PoolState::Unsettled || !Self::continuous(head))
    }
    fn continuous(head: &Head) -> bool {
        head.revision != u64::MAX
            && head.revision == super::CREDIT_REVISION.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(super) fn skip(&mut self, handle: u64, army: usize) -> bool {
        let Some(head) = self.heads.get_mut(&handle).filter(|head| head.army == army) else {
            return false;
        };
        head.state = PoolState::Unsettled;
        head.revision = u64::MAX;
        true
    }
    pub(super) fn check_continuity(&self, handle: u64) -> Result<(), &'static str> {
        if self.heads.get(&handle).is_some_and(Self::continuous) {
            Ok(())
        } else {
            Err("equipment returned source skipped native mutation")
        }
    }
    pub(super) fn state(
        &self,
        handle: u64,
        army: usize,
        country: u32,
    ) -> Result<PoolState, &'static str> {
        let head = self
            .heads
            .get(&handle)
            .filter(|head| head.army == army)
            .ok_or("equipment returned source has no actual credit witness")?;
        if head.country != country {
            return Err("equipment returned source changed owner");
        }
        if !Self::continuous(head) {
            return Err("equipment returned source skipped native mutation");
        }
        if head.state == PoolState::Unsettled {
            return Err("equipment returned source lifetime incomplete");
        }
        Ok(head.state)
    }
    pub(super) fn invalidate(&mut self, handle: u64) {
        if let Some(head) = self.heads.get_mut(&handle) {
            head.state = PoolState::Unsettled;
        }
    }
    pub(super) fn set_state(
        &mut self,
        handle: u64,
        army: usize,
        country: u32,
        state: PoolState,
    ) -> Result<(), &'static str> {
        let head = self
            .heads
            .get_mut(&handle)
            .filter(|head| head.army == army)
            .ok_or("equipment returned source has no actual credit witness")?;
        if head.country != country {
            return Err("equipment returned source changed owner");
        }
        if !Self::continuous(head) {
            return Err("equipment returned source skipped native mutation");
        }
        head.state = state;
        Ok(())
    }
    pub(super) fn contains(&self, handle: u64) -> bool {
        self.heads.get(&handle).is_some()
    }
    pub(super) fn visit(
        &self,
        handle: u64,
        army: usize,
        country: u32,
        mut visit: impl FnMut(Net) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let head = self
            .heads
            .get(&handle)
            .filter(|head| head.army == army)
            .ok_or("equipment returned source has no actual credit witness")?;
        if head.country != country {
            return Err("equipment returned source changed owner");
        }
        let mut node = head.first;
        while node != NONE {
            visit(self.nodes[node].value)?;
            node = self.nodes[node].next;
        }
        Ok(())
    }
    pub(super) fn remove(&mut self, handle: u64) {
        if let Some(head) = self.heads.remove(&handle) {
            let mut node = head.first;
            while node != NONE {
                let next = self.nodes[node].next;
                self.nodes[node].next = self.free;
                self.free = node;
                node = next;
            }
        }
    }
}

pub(super) fn push(output: &mut [Net], len: &mut usize, value: Net) -> Result<(), &'static str> {
    *output
        .get_mut(*len)
        .ok_or("equipment conservation scratch too short")? = value;
    *len += 1;
    Ok(())
}
/// Wide sums preserve signed duplicate entries even when their partial sum
/// exceeds i64. Only the final serialized correction must fit the native width.
pub(super) fn compact(values: &mut [Net]) -> Result<usize, &'static str> {
    values.sort_unstable_by_key(|value| (value.country, value.variant));
    let mut len = 0;
    for index in 0..values.len() {
        let value = values[index];
        if len != 0
            && values[len - 1].variant == value.variant
            && values[len - 1].country == value.country
        {
            values[len - 1].amount = values[len - 1]
                .amount
                .checked_add(value.amount)
                .ok_or("equipment conservation wide sum overflow")?;
            values[len - 1].credited = values[len - 1]
                .credited
                .checked_add(value.credited)
                .ok_or("equipment conservation wide credit overflow")?;
        } else {
            values[len] = value;
            len += 1;
        }
    }
    Ok(len)
}
