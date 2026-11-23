
use crate::{Memory, read};

pub const LEFT: usize = 0;
pub const PARENT: usize = 8;
pub const RIGHT: usize = 0x10;
pub const COLOR: usize = 0x18;
pub const NIL: usize = 0x19;
pub const PAYLOAD: usize = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetError {
    NullLink,
    UnreadableNode,
    InvalidHead,
    ForeignSentinel,
    ExcessNodes,
    MissingNodes,
    Cycle,
    BudgetOverflow,
}

pub fn walk_set<E>(
    memory: &impl Memory,
    head: usize,
    size: u64,
    error: impl Fn(SetError) -> E,
    mut visit: impl FnMut(u64) -> Result<(), E>,
) -> Result<(), E> {
    const BYTES: usize = PAYLOAD + 8;
    let read_node = |address| {
        if address == 0 {
            return Err(error(SetError::NullLink));
        }
        read::<[u8; BYTES]>(memory, address).ok_or_else(|| error(SetError::UnreadableNode))
    };
    let link = |bytes: &[u8; BYTES], offset| {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize
    };
    let sentinel = read_node(head)?;
    if sentinel[NIL] == 0 {
        return Err(error(SetError::InvalidHead));
    }
    let mut budget = size
        .checked_mul(8)
        .and_then(|n| n.checked_add(16))
        .ok_or_else(|| error(SetError::BudgetOverflow))?;
    let mut step = |address| {
        budget = budget
            .checked_sub(1)
            .ok_or_else(|| error(SetError::Cycle))?;
        read_node(address)
    };
    let mut visited = 0;
    let mut current = link(&sentinel, LEFT);
    while current != head {
        if visited == size {
            return Err(error(SetError::ExcessNodes));
        }
        let bytes = step(current)?;
        if bytes[NIL] != 0 {
            return Err(error(SetError::ForeignSentinel));
        }
        visit(u64::from_le_bytes(bytes[PAYLOAD..].try_into().unwrap()))?;
        visited += 1;
        let right = link(&bytes, RIGHT);
        let mut next = step(right)?;
        if next[NIL] == 0 {
            current = right;
            loop {
                let left = link(&next, LEFT);
                let candidate = step(left)?;
                if candidate[NIL] != 0 {
                    break;
                }
                current = left;
                next = candidate;
            }
        } else {
            let mut child = current;
            let mut parent = link(&bytes, PARENT);
            loop {
                let above = step(parent)?;
                if above[NIL] != 0 || link(&above, RIGHT) != child {
                    break;
                }
                child = parent;
                parent = link(&above, PARENT);
            }
            current = parent;
        }
    }
    if visited != size {
        return Err(error(SetError::MissingNodes));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub struct Cursor {
    head: usize,
    next: usize,
    remaining: u32,
    count: u32,
}

impl Cursor {
    pub fn read(
        memory: &impl Memory,
        owner: usize,
        offset: usize,
    ) -> Result<Self, &'static str> {
        let words = read::<[u8; 16]>(memory, at(owner, offset)?)
            .ok_or("character role map header unreadable")?;
        let (head, _) = Self::header(words)?;
        let sentinel =
            read::<[u8; 26]>(memory, head).ok_or("character role map sentinel unreadable")?;
        Self::from_parts(words, sentinel)
    }

    pub fn header(words: [u8; 16]) -> Result<(usize, u32), &'static str> {
        let head = usize::try_from(u64::from_le_bytes(words[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("character role map head invalid")?;
        let count = u32::try_from(u64::from_le_bytes(words[8..].try_into().unwrap()))
            .map_err(|_| "character role map count overflow")?;
        Ok((head, count))
    }

    pub fn from_parts(words: [u8; 16], sentinel: [u8; 26]) -> Result<Self, &'static str> {
        use crate::msvc_tree_node as node;
        let (head, count) = Self::header(words)?;
        if sentinel[node::NIL] != 1 {
            return Err("character role map sentinel invalid");
        }
        let link = |offset| {
            usize::try_from(u64::from_le_bytes(
                sentinel[offset..offset + 8].try_into().unwrap(),
            ))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("character role map link unreadable or null")
        };
        let next = link(node::LEFT)?;
        if (count == 0) != (next == head) {
            return Err("character role map size disagrees with sentinel");
        }
        if count == 0 && (link(node::PARENT)? != head || link(node::RIGHT)? != head) {
            return Err("empty character role map links invalid");
        }
        Ok(Self {
            head,
            next,
            remaining: count,
            count,
        })
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn next(&mut self, memory: &impl Memory) -> Result<Option<usize>, &'static str> {
        use crate::msvc_tree_node as node;
        if self.remaining == 0 {
            if self.next != self.head {
                return Err("character role map contains excess nodes");
            }
            return Ok(None);
        }
        let current = self.next;
        if current == self.head || read::<u8>(memory, at(current, node::NIL)?) != Some(0) {
            return Err("character role map ended before declared count");
        }
        let right = Self::link(memory, current, node::RIGHT)?;
        let mut next = right;
        let mut steps = 0_u64;
        if right != self.head {
            loop {
                steps += 1;
                if steps > u64::from(self.count) {
                    return Err("character role map child cycle");
                }
                if read::<u8>(memory, at(next, node::NIL)?) != Some(0) {
                    return Err("character role map child sentinel invalid");
                }
                let left = Self::link(memory, next, node::LEFT)?;
                if left == self.head {
                    break;
                }
                if Self::link(memory, left, node::PARENT)? != next {
                    return Err("character role map child parent mismatch");
                }
                next = left;
            }
            if Self::link(memory, right, node::PARENT)? != current {
                return Err("character role map right parent mismatch");
            }
        } else {
            let mut child = current;
            next = Self::link(memory, child, node::PARENT)?;
            while next != self.head {
                steps += 1;
                if steps > u64::from(self.count) {
                    return Err("character role map parent cycle");
                }
                if Self::link(memory, next, node::LEFT)? == child {
                    break;
                }
                if Self::link(memory, next, node::RIGHT)? != child {
                    return Err("character role map parent does not own child");
                }
                child = next;
                next = Self::link(memory, child, node::PARENT)?;
            }
        }
        self.next = next;
        self.remaining -= 1;
        Ok(Some(current))
    }

    fn link(memory: &impl Memory, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        read::<u64>(memory, at(pointer, offset)?)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("character role map link unreadable or null")
    }
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("character address overflow")
}
