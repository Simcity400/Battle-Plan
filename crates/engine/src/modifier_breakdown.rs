//! Cached modifier trees used for breakdown display.
//! Labels are stored display strings, distinct from original source keys.
use crate::{Memory, cpdx_array::Array, modifier_accumulator, msvc_string, read};

/// Preorder node; child occurrences retain their native order, including aliases.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Node {
    pub children: u32,
    pub value: i64,
    pub name: Vec<u8>,
}

/// Accumulator children header and stored display string.
const CHILDREN: usize = 0x18;
const DISPLAY: usize = 0x48;

fn text(memory: &impl Memory, address: usize) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    let mut used = 0;
    msvc_string::append_growing(memory, address, &mut bytes, &mut used)?;
    bytes.truncate(used);
    Ok(bytes)
}

/// No depth or node limit. Only ancestor cycles are invalid; repeated
/// siblings are separate occurrences. Values use passive effective lookup.
pub fn tree(
    memory: &impl Memory,
    base: usize,
    root: usize,
    key: u32,
) -> Result<Vec<Node>, &'static str> {
    walk(memory, base, root, key, Names::Every)
}

/// [`tree`]'s nodes, values and order for one key, reading only what a depth -1
/// breakdown prints: a node's display bytes are read only for a leaf whose
/// value is nonzero. Every other node keeps an empty name.
pub fn printed_nodes(
    memory: &impl Memory,
    base: usize,
    root: usize,
    key: u32,
) -> Result<Vec<Node>, &'static str> {
    walk(memory, base, root, key, Names::NonzeroLeaves)
}

/// Which nodes' display bytes a walk reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Names {
    Every,
    NonzeroLeaves,
}

/// A child entry points at its modifier object; the accumulator the tree
/// walks lies this far inside it.
const CHILD_ACCUMULATOR: usize = 0x10;

/// The preorder walk both readers share, the key's definition metadata read
/// once for the walk (a failed read is read again, as a fresh lookup would).
fn walk(
    memory: &impl Memory,
    base: usize,
    root: usize,
    key: u32,
    names: Names,
) -> Result<Vec<Node>, &'static str> {
    let definitions = crate::modifier_definition::DefinitionMemo::default();
    let mut nodes = Vec::new();
    // The current path: each node with its children and the next child.
    let mut stack: Vec<(usize, Array, u32)> = Vec::new();
    let mut address = root;
    loop {
        if address == 0 || stack.iter().any(|&(active, ..)| active == address) {
            return Err("modifier breakdown null node or ancestor cycle");
        }
        let children = address
            .checked_add(CHILDREN)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("modifier breakdown children unreadable or invalid")?;
        let raw = modifier_accumulator::lookup_at(memory, address, key)?;
        let value = if raw.present {
            crate::modifier_definition::effective_memo(memory, base, key, raw.raw, &definitions)?
        } else {
            0
        };
        let name = if names == Names::Every || (children.count == 0 && value != 0) {
            text(
                memory,
                address
                    .checked_add(DISPLAY)
                    .ok_or("modifier breakdown display address overflow")?,
            )?
        } else {
            Vec::new()
        };
        crate::allocation::reserve(&mut nodes, 1)
            .map_err(|_| "modifier breakdown allocation failed")?;
        crate::allocation::reserve(&mut stack, 1)
            .map_err(|_| "modifier breakdown allocation failed")?;
        nodes.push(Node {
            children: children.count,
            value,
            name,
        });
        stack.push((address, children, 0));
        loop {
            let Some((_, children, ordinal)) = stack.last_mut() else {
                return Ok(nodes);
            };
            if *ordinal == children.count {
                stack.pop();
                continue;
            }
            let pointer = children
                .element(*ordinal, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("modifier breakdown child pointer unreadable or null")?;
            *ordinal += 1;
            address = pointer
                .checked_add(CHILD_ACCUMULATOR)
                .ok_or("modifier breakdown child address overflow")?;
            break;
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Balance {
    /// Complete +0x50 static-modifier roster, including empty sources.
    pub sources: Vec<Vec<u8>>,
    pub daily: Vec<Node>,
    pub weekly: Vec<Node>,
}

/// The inputs of a record's cached +0xC8 tree: its complete +0x50
/// static-modifier roster (count +0x5C), every occurrence in array order, by
/// `CStaticModifier` script name (+0x1A8), empty sources included. Rebuilding
/// clears +0xC8 and passes each to filtered modifier addition in this order,
/// and source addition pushes a new source to the end and adds it the same way,
/// so playback derives both cached trees from these names and the content
/// (`replay::power_balance_derive`). No accumulator, child or getter is read.
pub fn balance_sources(memory: &impl Memory, record: usize) -> Result<Vec<Vec<u8>>, &'static str> {
    let sources = record
        .checked_add(0x50)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("power balance modifier sources unreadable or invalid")?;
    let mut names = Vec::new();
    crate::allocation::reserve_push_batch(&mut names, sources.count as usize)
        .map_err(|_| "power balance source allocation failed")?;
    for ordinal in 0..sources.count {
        let source = sources
            .element(ordinal, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("power balance static modifier unreadable or null")?;
        names.push(text(
            memory,
            source
                .checked_add(0x1a8)
                .ok_or("power balance modifier name address overflow")?,
        )?);
    }
    Ok(names)
}

/// Rebuilding appends one clone per nonempty static modifier, in +0x50/+0x5C
/// order. CStaticModifier's script identity is +0x1A8.
pub fn balance(memory: &impl Memory, base: usize, record: usize) -> Result<Balance, &'static str> {
    let sources = record
        .checked_add(0x50)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("power balance modifier sources unreadable or invalid")?;
    let mut output = Balance::default();
    let mut names = crate::allocation::vec(sources.count as usize)
        .map_err(|_| "power balance source allocation failed")?;
    crate::allocation::reserve_push_batch(&mut output.sources, sources.count as usize)
        .map_err(|_| "power balance source allocation failed")?;
    for ordinal in 0..sources.count {
        let source = sources
            .element(ordinal, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("power balance static modifier unreadable or null")?;
        let name = text(
            memory,
            source
                .checked_add(0x1a8)
                .ok_or("power balance modifier name address overflow")?,
        )?;
        // Children, custom tooltips or any eligible entry keep a clone.
        let accumulator = source
            .checked_add(0x10)
            .ok_or("power balance source accumulator address overflow")?;
        let entries = Array::read(memory, accumulator, 16)
            .ok_or("power balance source accumulator unreadable or invalid")?;
        let tooltips = read::<i32>(memory, accumulator + 0x78)
            .ok_or("power balance source tooltip count unreadable")?;
        let children = read::<i32>(memory, accumulator + 0x24)
            .ok_or("power balance source child count unreadable")?;
        let mut nonempty = tooltips > 0 || children != 0;
        if !nonempty {
            for i in 0..entries.count {
                let key = entries
                    .element(i, 16)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("power balance source modifier key unreadable")?;
                if crate::modifier_definition::enabled(memory, base, key)? {
                    nonempty = true;
                    break;
                }
            }
        }
        if nonempty {
            names.push(output.sources.len());
        }
        output.sources.push(name);
    }
    let root = record
        .checked_add(0xc8)
        .ok_or("power balance accumulator address overflow")?;
    output.daily = tree(
        memory,
        base,
        root,
        crate::modifier_keys::POWER_BALANCE_DAILY,
    )?;
    output.weekly = tree(
        memory,
        base,
        root,
        crate::modifier_keys::POWER_BALANCE_WEEKLY,
    )?;
    // Constructed nodes have depth one; cloned nodes have depth zero.
    for nodes in [&mut output.daily, &mut output.weekly] {
        if nodes[0].children as usize != names.len()
            || nodes.len() != names.len() + 1
            || nodes[1..].iter().any(|n| n.children != 0)
        {
            return Err("power balance cached modifier children do not match static source order");
        }
        nodes[0].name.clear();
        for (node, &source) in nodes[1..].iter_mut().zip(&names) {
            let name = &output.sources[source];
            let additional = name.len().saturating_sub(node.name.len());
            crate::allocation::reserve(&mut node.name, additional)
                .map_err(|_| "power balance name allocation failed")?;
            node.name.clear();
            node.name.extend_from_slice(name);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{memory::SliceMemory, modifier_definition as definition, modifier_keys as keys};

    struct Fixture(Vec<u8>);
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == definition::TABLE_RVA && output.len() == 8 {
                output.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
                return true;
            }
            if (0x3000_0000..0x3004_0000).contains(&address) {
                output.fill(0);
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: &self.0,
            }
            .copy(address, output)
        }
    }
    impl Fixture {
        fn new() -> Self {
            Self(vec![0; 0x2000])
        }
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0.resize(self.0.len().max(at + bytes.len()), 0);
            self.0[at..at + bytes.len()].copy_from_slice(bytes);
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn alloc(&mut self, size: usize) -> usize {
            let at = (self.0.len() + 7) & !7;
            self.0.resize(at + size, 0);
            at
        }
        fn string(&mut self, at: usize, name: &[u8]) {
            self.put(at, &[0; 32]);
            self.u64(at + 16, name.len() as u64);
            if name.len() < 16 {
                self.put(at, name);
                self.u64(at + 24, 15);
            } else {
                let data = self.alloc(name.len() + 1);
                self.put(data, name);
                self.u64(at, data as u64);
                self.u64(at + 24, name.len() as u64);
            }
        }
        fn array(&mut self, at: usize, values: &[u64]) {
            let data = self.alloc(values.len() * 8);
            for (i, value) in values.iter().enumerate() {
                self.u64(data + i * 8, *value);
            }
            self.u64(at, data as u64);
            self.u64(at + 8, (values.len() as u64) * 0x1_0000_0001);
        }
        fn node(&mut self, at: usize, label: &[u8], values: &[(u32, i64)], children: &[usize]) {
            self.put(at, &[0; 0xb0]);
            let entries = self.alloc(values.len() * 16);
            self.u64(at, entries as u64);
            self.u64(at + 8, (values.len() as u64) * 0x1_0000_0001);
            for (i, &(key, value)) in values.iter().enumerate() {
                self.put(entries + i * 16, &key.to_le_bytes());
                self.put(entries + i * 16 + 8, &value.to_le_bytes());
            }
            self.array(
                at + CHILDREN,
                &children
                    .iter()
                    .map(|p| (*p - 0x10) as u64)
                    .collect::<Vec<_>>(),
            );
            self.string(at + DISPLAY, label);
        }
    }

    #[test]
    fn country_and_selected_commander_trees_keep_order_labels_and_getter_values() {
        let mut f = Fixture::new();
        let country = f.alloc(0x1000);
        let leader = f.alloc(0x1000);
        let child = f.alloc(0x200);
        let first = f.alloc(0x200);
        let second = f.alloc(0x200);
        f.node(
            first,
            &[b'x'; 600],
            &[(keys::LAND_REINFORCE_RATE, -700)],
            &[],
        );
        f.node(second, b"zero", &[(keys::LAND_REINFORCE_RATE, 0)], &[]);
        f.node(
            child,
            b"nested",
            &[(keys::LAND_REINFORCE_RATE, 9)],
            &[first, second, first],
        );
        f.node(
            country + crate::ccountry::MODIFIER_ACCUMULATOR,
            b"",
            &[],
            &[child],
        );
        let nodes = tree(
            &f,
            0,
            country + crate::ccountry::MODIFIER_ACCUMULATOR,
            keys::LAND_REINFORCE_RATE,
        )
        .unwrap();
        assert_eq!(
            nodes
                .iter()
                .map(|n| (n.children, n.value))
                .collect::<Vec<_>>(),
            [(1, 0), (3, 9), (0, -700), (0, 0), (0, -700)]
        );
        assert_eq!(nodes[2].name, vec![b'x'; 600]);
        for offset in [
            crate::carmy_leader::MOVEMENT_MODIFIER,
            crate::carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER,
        ] {
            f.node(
                leader + offset + 0x10,
                b"commander",
                &[(keys::LAND_REINFORCE_RATE, 250)],
                &[first],
            );
            let nodes = tree(&f, 0, leader + offset + 0x10, keys::LAND_REINFORCE_RATE).unwrap();
            assert_eq!(
                (nodes[0].name.as_slice(), nodes[0].value, nodes[1].value),
                (b"commander".as_slice(), 250, -700)
            );
        }
        // The printed walk keeps the same nodes, values and order and reads
        // display bytes only for nonzero leaves.
        let root = country + crate::ccountry::MODIFIER_ACCUMULATOR;
        let full = tree(&f, 0, root, keys::LAND_REINFORCE_RATE).unwrap();
        let printed = printed_nodes(&f, 0, root, keys::LAND_REINFORCE_RATE).unwrap();
        assert_eq!(printed.len(), full.len());
        for (printed, full) in printed.iter().zip(&full) {
            assert_eq!(
                (printed.children, printed.value),
                (full.children, full.value)
            );
            let shown = full.children == 0 && full.value != 0;
            assert_eq!(
                printed.name,
                if shown { full.name.clone() } else { Vec::new() }
            );
        }
        f.array(first + CHILDREN, &[first as u64 - 0x10]);
        assert_eq!(
            tree(&f, 0, first, keys::LAND_REINFORCE_RATE),
            Err("modifier breakdown null node or ancestor cycle")
        );
        assert_eq!(
            printed_nodes(&f, 0, first, keys::LAND_REINFORCE_RATE),
            Err("modifier breakdown null node or ancestor cycle")
        );
        f.array(first + CHILDREN, &[0]);
        assert!(tree(&f, 0, first, keys::LAND_REINFORCE_RATE).is_err());
    }

    #[test]
    fn balance_sources_keep_duplicate_keys_and_cached_values_in_clone_order() {
        let mut f = Fixture::new();
        let record = f.alloc(0x200);
        let source = f.alloc(0x200);
        let empty = f.alloc(0x200);
        let tooltip = f.alloc(0x200);
        let children = [f.alloc(0x200), f.alloc(0x200), f.alloc(0x200)];
        f.node(
            source + 0x10,
            b"stored",
            &[(keys::POWER_BALANCE_DAILY, 999)],
            &[],
        );
        f.string(source + 0x1a8, b"static_key");
        f.node(empty + 0x10, b"", &[], &[]);
        f.string(empty + 0x1a8, b"empty_key");
        f.node(tooltip + 0x10, b"", &[], &[]);
        f.put(tooltip + 0x88, &1_i32.to_le_bytes());
        f.string(tooltip + 0x1a8, b"tooltip_key");
        for (i, child) in children.into_iter().enumerate() {
            f.node(
                child,
                b"cached label",
                &[
                    (keys::POWER_BALANCE_DAILY, i as i64 - 3),
                    (keys::POWER_BALANCE_WEEKLY, 20 + i as i64),
                ],
                &[],
            );
        }
        f.node(record + 0xc8, b"", &[], &children);
        f.array(
            record + 0x50,
            &[source as u64, empty as u64, source as u64, tooltip as u64],
        );
        let captured = balance(&f, 0, record).unwrap();
        assert_eq!(
            captured.sources,
            [
                b"static_key".to_vec(),
                b"empty_key".to_vec(),
                b"static_key".to_vec(),
                b"tooltip_key".to_vec()
            ]
        );
        assert_eq!(
            captured.daily[1..]
                .iter()
                .map(|n| (n.name.as_slice(), n.value))
                .collect::<Vec<_>>(),
            [
                (b"static_key".as_slice(), -3),
                (b"static_key".as_slice(), -2),
                (b"tooltip_key".as_slice(), -1)
            ]
        );
        assert_eq!(captured.weekly[3].value, 22);
        // The source roster alone reads no accumulator: an unreadable tree
        // leaves it intact.
        assert_eq!(balance_sources(&f, record).unwrap(), captured.sources);
        f.array(record + 0xc8 + CHILDREN, &[0]);
        assert!(balance(&f, 0, record).is_err());
        assert_eq!(balance_sources(&f, record).unwrap(), captured.sources);
    }

    #[test]
    fn five_thousand_countries_with_sources_have_no_population_limit() {
        let mut f = Fixture::new();
        let mut roots = Vec::new();
        for index in 0..5_000 {
            let country = f.alloc(0x1000);
            let child = f.alloc(0x200);
            f.node(child, b"source", &[(keys::LAND_REINFORCE_RATE, index)], &[]);
            let root = country + crate::ccountry::MODIFIER_ACCUMULATOR;
            f.node(root, b"", &[], &[child]);
            roots.push(root);
        }
        for (index, root) in roots.into_iter().enumerate() {
            let nodes = tree(&f, 0, root, keys::LAND_REINFORCE_RATE).unwrap();
            assert_eq!((nodes.len(), nodes[1].value), (2, index as i64));
        }
    }
}
