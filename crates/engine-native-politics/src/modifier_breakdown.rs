use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, read};
use engine_native_parts::modifier_accumulator;

pub use game_model::modifier_breakdown::{Node, SourceValue, SourceNode};

pub fn source_nodes(
    memory: &impl Memory,
    root: usize,
    mut visit: impl FnMut(u32, SourceNode) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut stack: Vec<(usize, Array, u32)> = Vec::new();
    let mut address = root;
    let mut ordinal = 0u32;
    loop {
        if address == 0 || stack.iter().any(|&(active, ..)| active == address) {
            return Err("division modifier source null node or ancestor cycle");
        }
        let children = Array::read(
            memory,
            address
                .checked_add(CHILDREN)
                .ok_or("division modifier source address overflow")?,
            8,
        )
        .ok_or("division modifier source children unreadable")?;
        let mut values = Vec::new();
        let mut failure = false;
        modifier_accumulator::entries_at(memory, address, &mut |key, raw| {
            if engine_allocation::reserve(&mut values, 1).is_err() {
                failure = true;
            } else {
                values.push(SourceValue { key, raw });
            }
        })?;
        if failure {
            return Err("division modifier source values allocation failed");
        }
        values.sort_unstable_by_key(|value| value.key);
        let name = text(
            memory,
            address
                .checked_add(DISPLAY)
                .ok_or("division modifier source display overflow")?,
        )?;
        visit(
            ordinal,
            SourceNode {
                children: children.count,
                name,
                values,
            },
        )?;
        ordinal = ordinal
            .checked_add(1)
            .ok_or("division modifier source ordinal overflow")?;
        engine_allocation::reserve(&mut stack, 1)
            .map_err(|_| "division modifier source stack allocation failed")?;
        stack.push((address, children, 0));
        loop {
            let Some((_, children, next)) = stack.last_mut() else {
                return Ok(());
            };
            if *next == children.count {
                stack.pop();
                continue;
            }
            let pointer = children
                .element(*next, 8)
                .and_then(|slot| read::<u64>(memory, slot))
                .filter(|&pointer| pointer != 0)
                .ok_or("division modifier source child unreadable")?;
            *next += 1;
            address = (pointer as usize)
                .checked_add(CHILD_ACCUMULATOR)
                .ok_or("division modifier source child overflow")?;
            break;
        }
    }
}

const CHILDREN: usize = 0x18;
const DISPLAY: usize = 0x48;

fn text(memory: &impl Memory, address: usize) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    let mut used = 0;
    msvc_string::append(memory, address, &mut bytes, &mut used)?;
    bytes.truncate(used);
    Ok(bytes)
}

pub fn tree(
    memory: &impl Memory,
    base: usize,
    root: usize,
    key: u32,
) -> Result<Vec<Node>, &'static str> {
    walk(memory, base, root, key, Names::Every)
}

pub fn printed_nodes(
    memory: &impl Memory,
    base: usize,
    root: usize,
    key: u32,
) -> Result<Vec<Node>, &'static str> {
    walk(memory, base, root, key, Names::NonzeroLeaves)
}

pub type PrintedTree = Result<Vec<Node>, &'static str>;

pub fn printed_keys(
    memory: &impl Memory,
    base: usize,
    root: usize,
    keys: &[u32],
) -> Result<Vec<PrintedTree>, &'static str> {
    let definitions = engine_native_parts::modifier_definition::DefinitionMemo::default();
    let mut output =
        engine_allocation::vec(keys.len()).map_err(|_| "modifier breakdown allocation failed")?;
    output.resize_with(keys.len(), || Ok(Vec::new()));
    if keys.is_empty() {
        return Ok(output);
    }
    let mut values = engine_allocation::default_vec::<i64>(keys.len())
        .map_err(|_| "modifier breakdown allocation failed")?;
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
        for ((&key, value), nodes) in keys.iter().zip(&mut values).zip(&mut output) {
            *value = 0;
            if nodes.is_ok() {
                match modifier_accumulator::requested_effective_memo(
                    memory,
                    base,
                    address,
                    key,
                    &definitions,
                ) {
                    Ok(current) => *value = current,
                    Err(reason) => *nodes = Err(reason),
                }
            }
        }
        let name = if children.count == 0 && values.iter().any(|&v| v != 0) {
            text(
                memory,
                address
                    .checked_add(DISPLAY)
                    .ok_or("modifier breakdown display address overflow")?,
            )
        } else {
            Ok(Vec::new())
        };
        for (output, &value) in output.iter_mut().zip(&values) {
            if output.is_err() {
                continue;
            }
            let name = if children.count == 0 && value != 0 {
                match &name {
                    Ok(name) => engine_allocation::copy_vec(name)
                        .map_err(|_| "modifier breakdown allocation failed")?,
                    Err(reason) => {
                        *output = Err(*reason);
                        continue;
                    }
                }
            } else {
                Vec::new()
            };
            let nodes = output.as_mut().unwrap();
            engine_allocation::reserve(nodes, 1)
                .map_err(|_| "modifier breakdown allocation failed")?;
            nodes.push(Node {
                children: children.count,
                value,
                name,
            });
        }
        engine_allocation::reserve(&mut stack, 1)
            .map_err(|_| "modifier breakdown allocation failed")?;
        stack.push((address, children, 0));
        loop {
            let Some((_, children, ordinal)) = stack.last_mut() else {
                return Ok(output);
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Names {
    Every,
    NonzeroLeaves,
}

const CHILD_ACCUMULATOR: usize = 0x10;

fn walk(
    memory: &impl Memory,
    base: usize,
    root: usize,
    key: u32,
    names: Names,
) -> Result<Vec<Node>, &'static str> {
    let definitions = engine_native_parts::modifier_definition::DefinitionMemo::default();
    let mut nodes = Vec::new();
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
            engine_native_parts::modifier_definition::effective_memo(memory, base, key, raw.raw, &definitions)?
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
        engine_allocation::reserve(&mut nodes, 1)
            .map_err(|_| "modifier breakdown allocation failed")?;
        engine_allocation::reserve(&mut stack, 1)
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
    pub sources: Vec<Vec<u8>>,
    pub daily: Vec<Node>,
    pub weekly: Vec<Node>,
}

pub fn balance_sources(memory: &impl Memory, record: usize) -> Result<Vec<Vec<u8>>, &'static str> {
    let sources = record
        .checked_add(0x50)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("power balance modifier sources unreadable or invalid")?;
    let mut names = Vec::new();
    engine_allocation::reserve_push_batch(&mut names, sources.count as usize)
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

pub fn balance(memory: &impl Memory, base: usize, record: usize) -> Result<Balance, &'static str> {
    let sources = record
        .checked_add(0x50)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("power balance modifier sources unreadable or invalid")?;
    let mut output = Balance::default();
    let mut names = engine_allocation::vec(sources.count as usize)
        .map_err(|_| "power balance source allocation failed")?;
    engine_allocation::reserve_push_batch(&mut output.sources, sources.count as usize)
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
                if engine_native_parts::modifier_definition::enabled(memory, base, key)? {
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
        engine_executable::modifier_keys::POWER_BALANCE_DAILY,
    )?;
    output.weekly = tree(
        memory,
        base,
        root,
        engine_executable::modifier_keys::POWER_BALANCE_WEEKLY,
    )?;
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
            engine_allocation::reserve(&mut node.name, additional)
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
    use engine_native_memory::memory::SliceMemory;
    use engine_native_parts::modifier_definition as definition;
    use engine_executable::modifier_keys as keys;

    struct Fixture(Vec<u8>);
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == definition::TABLE_RVA.0 && output.len() == 8 {
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
    #[test]
    fn occupation_printed_keys_share_tree_headers_and_match_single_key_preorders() {
        use std::cell::Cell;
        let mut fixture = Fixture::new();
        let root = fixture.alloc(0x100);
        let leaf = fixture.alloc(0x100);
        let key = keys::LAND_REINFORCE_RATE;
        fixture.node(leaf, b"fixture source", &[(key, 321), (key + 1, -432)], &[]);
        fixture.node(root, b"", &[(key, 321), (key + 1, -432)], &[leaf, leaf]);
        struct Traced<'a>(&'a Fixture, Cell<usize>, usize, usize);
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if [self.2 + CHILDREN, self.3 + CHILDREN, self.3 + DISPLAY].contains(&address) {
                    self.1.set(self.1.get() + 1);
                }
                self.0.copy(address, out)
            }
        }
        let traced = Traced(&fixture, Cell::new(0), root, leaf);
        let actual = printed_keys(&traced, 0, root, &[key, key + 1])
            .unwrap()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let shared = traced.1.replace(0);
        let expected = vec![
            printed_nodes(&traced, 0, root, key).unwrap(),
            printed_nodes(&traced, 0, root, key + 1).unwrap(),
        ];
        assert_eq!(actual, expected);
        assert!(
            shared < traced.1.get(),
            "shared traversal must avoid repeated headers and labels"
        );
        assert_eq!(actual[0][1].name, b"fixture source");
        assert_eq!(actual[1][2].value, -432);
    }

    #[test]
    fn occupation_printed_keys_keep_a_healthy_tree_when_another_key_value_is_unreadable() {
        let mut fixture = Fixture::new();
        let root = fixture.alloc(0x100);
        let leaf = fixture.alloc(0x100);
        let key = keys::LAND_REINFORCE_RATE;
        fixture.node(leaf, b"fixture source", &[(key, 321), (key + 1, -432)], &[]);
        fixture.node(root, b"", &[(key, 321), (key + 1, -432)], &[leaf]);
        let entries = u64::from_le_bytes(fixture.0[leaf..leaf + 8].try_into().unwrap()) as usize;
        struct Fault<'a>(&'a Fixture, usize);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if address < self.1 + 8 && address + out.len() > self.1 {
                    return false;
                }
                self.0.copy(address, out)
            }
        }
        let output =
            printed_keys(&Fault(&fixture, entries + 24), 0, root, &[key, key + 1]).unwrap();
        assert_eq!(
            output[0].as_ref().unwrap(),
            &printed_nodes(&fixture, 0, root, key).unwrap()
        );
        assert!(output[1].is_err());
    }

    #[test]
    fn occupation_printed_keys_label_failure_does_not_refuse_a_zero_leaf_key() {
        let mut fixture = Fixture::new();
        let root = fixture.alloc(0x100);
        let leaf = fixture.alloc(0x100);
        let key = keys::LAND_REINFORCE_RATE;
        fixture.node(leaf, b"fixture source", &[(key, 0), (key + 1, 1)], &[]);
        fixture.node(root, b"", &[(key, 1), (key + 1, 1)], &[leaf]);
        struct Fault<'a>(&'a Fixture, usize);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if (self.1 + DISPLAY..self.1 + DISPLAY + 32).contains(&address) {
                    return false;
                }
                self.0.copy(address, out)
            }
        }
        let output = printed_keys(&Fault(&fixture, leaf), 0, root, &[key, key + 1]).unwrap();
        assert_eq!(output[0].as_ref().unwrap()[1].value, 0);
        assert!(output[1].is_err());
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
    fn shared_source_nodes_preserve_raw_keys_labels_and_repeated_children() {
        let mut fixture = Fixture::new();
        let root = fixture.alloc(0x200);
        let child = fixture.alloc(0x200);
        fixture.node(child, b"Source", &[(57, 19), (79, -31)], &[]);
        fixture.node(root, b"Root", &[(57, 38), (79, -62)], &[child, child]);
        let mut nodes = Vec::new();
        source_nodes(&fixture, root, |ordinal, node| {
            assert_eq!(ordinal as usize, nodes.len());
            nodes.push(node);
            Ok(())
        })
        .unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].children, 2);
        assert_eq!(nodes[1], nodes[2]);
        assert_eq!(nodes[1].name, b"Source");
        assert_eq!(
            nodes[1].values,
            [
                SourceValue { key: 57, raw: 19 },
                SourceValue { key: 79, raw: -31 }
            ]
        );
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
            country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            b"",
            &[],
            &[child],
        );
        let nodes = tree(
            &f,
            0,
            country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
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
            engine_native::carmy_leader::MOVEMENT_MODIFIER,
            engine_native::carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER,
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
        let root = country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
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
        assert_eq!(balance_sources(&f, record).unwrap(), captured.sources);
        f.array(record + 0xc8 + CHILDREN, &[0]);
        assert!(balance(&f, 0, record).is_err());
        assert_eq!(balance_sources(&f, record).unwrap(), captured.sources);
    }
}
