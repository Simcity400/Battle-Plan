use crate::{
    naval_tree_changes::{self, Changes, Counts, Held, Store},
    table_conversion::unavailable,
};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{cfex_air, cfex_member, cnaval_combat};
use engine_native_naval::naval_ship_modifier_inputs;
use engine_naval_history::{
    naval_modifier_trees as trees, naval_modifier_trees::Definitions, naval_modifier_trees::KEYS,
    naval_modifier_trees::Kind, naval_modifier_trees::RawNode, naval_modifier_trees::Request,
};
use replay_data::{rows, rows::TableRows};
use replay_frame::frame::TableView;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Root {
    accumulator: usize,
    nodes: [usize; 2],
}

pub struct Trees {
    requests: Vec<Request>,
    root_of: Vec<usize>,
    roots: Vec<Root>,
    nodes: Vec<RawNode>,
    text: Vec<u8>,
    stack: Vec<trees::Frame>,
    fresh_roots: Vec<Root>,
    fresh_nodes: Vec<RawNode>,
    fresh_text: Vec<u8>,
    outcome: Outcome,
    oracle: bool,
}

impl Default for Trees {
    fn default() -> Self {
        Self {
            requests: Vec::new(),
            root_of: Vec::new(),
            roots: Vec::new(),
            nodes: Vec::new(),
            text: Vec::new(),
            stack: Vec::new(),
            fresh_roots: Vec::new(),
            fresh_nodes: Vec::new(),
            fresh_text: Vec::new(),
            outcome: Outcome::Unavailable {
                reason: "naval modifier trees not captured this hour",
            },
            oracle: naval_tree_changes::ORACLE,
        }
    }
}

fn grow<T: Clone>(vector: &mut Vec<T>, value: T) -> Result<(), &'static str> {
    engine_allocation::reserve(vector, 1).map_err(|_| "collector allocation failed")?;
    vector.push(value);
    Ok(())
}

impl Trees {
    pub fn reset(&mut self) {
        self.requests.clear();
        self.root_of.clear();
        self.roots.clear();
        self.nodes.clear();
        self.text.clear();
        self.stack.clear();
        self.fresh_roots.clear();
        self.fresh_nodes.clear();
        self.fresh_text.clear();
        self.outcome = Outcome::Unavailable {
            reason: "naval modifier trees not captured this hour",
        };
    }

    pub fn allocated_bytes(&self) -> usize {
        self.requests.capacity() * size_of::<Request>()
            + self.root_of.capacity() * size_of::<usize>()
            + (self.roots.capacity() + self.fresh_roots.capacity()) * size_of::<Root>()
            + (self.nodes.capacity() + self.fresh_nodes.capacity()) * size_of::<RawNode>()
            + self.text.capacity()
            + self.fresh_text.capacity()
            + self.stack.capacity() * size_of::<trees::Frame>()
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn samples_outcome(&self) -> Outcome {
        if self.oracle {
            self.outcome
        } else {
            Outcome::Rows { len: 0 }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the census slices are separately owned arenas of the naval pack"
    )]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        combats: Option<&[engine_native_naval::naval_combat_details::DetailsRow]>,
        members: Option<&[engine_native_naval::naval_member_details::DetailsRow]>,
        planes: &[engine_native_naval::naval_air_details::Row],
        sources: &[naval_ship_modifier_inputs::SourceRow],
    ) {
        self.reset();
        let (Some(combats), Some(members)) = (combats, members) else {
            self.outcome = Outcome::Unavailable {
                reason: "naval modifier tree sources incomplete",
            };
            return;
        };
        let mut store = naval_tree_changes::take_store();
        let mut counts = Counts::default();
        match self.read(
            memory,
            base,
            state,
            combats,
            members,
            planes,
            sources,
            &mut store,
            &mut counts,
            &naval_tree_changes::Observer,
        ) {
            Ok(()) => {
                naval_tree_changes::record(&counts);
                naval_tree_changes::return_store(store);
                self.outcome = Outcome::Rows {
                    len: self.requests.len(),
                };
            }
            Err(reason) => {
                naval_tree_changes::failed();
                self.outcome = Outcome::Unavailable { reason };
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the census slices, the store and the observer are separate owners"
    )]
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        combats: &[engine_native_naval::naval_combat_details::DetailsRow],
        members: &[engine_native_naval::naval_member_details::DetailsRow],
        planes: &[engine_native_naval::naval_air_details::Row],
        sources: &[naval_ship_modifier_inputs::SourceRow],
        store: &mut Box<Store>,
        counts: &mut Counts,
        changes: &impl Changes,
    ) -> Result<(), &'static str> {
        let requests = &mut self.requests;
        trees::enumerate(
            memory,
            base,
            state,
            combats,
            members,
            planes,
            sources,
            &mut |request| grow(requests, request),
        )?;
        if self.requests.is_empty() {
            store.held.clear();
            return Ok(());
        }
        let definitions = Definitions::read(memory, base)?;
        self.requests
            .sort_unstable_by_key(|request| (request.accumulator, request.kind, request.owner));
        let mut held = std::mem::take(&mut store.held);
        let mut at = 0;
        while at < self.requests.len() {
            let accumulator = self.requests[at].accumulator;
            let mut end = at + 1;
            while self
                .requests
                .get(end)
                .is_some_and(|request| request.accumulator == accumulator)
            {
                end += 1;
            }
            let carried = self.requests[at..end]
                .iter()
                .find(|request| Kind::from_u8(request.kind).is_some_and(Kind::carried))
                .copied();
            if self.requests[at..end].iter().any(|request| {
                Kind::from_u8(request.kind).is_some_and(Kind::carried)
                    && carried.is_some_and(|first| first.receiver != request.receiver)
            }) {
                return Err("naval tree accumulator names two receivers");
            }
            let root = self.roots.len();
            let first = self.nodes.len();
            match carried {
                Some(request) => {
                    counts.receivers += 1;
                    let certificate = changes.sequence(request.receiver);
                    let previous = held
                        .remove(&request.receiver)
                        .filter(|previous| previous.certified(certificate, accumulator));
                    match previous {
                        Some(previous) => {
                            self.copy_held(&previous)?;
                            counts.reused += 1;
                            store.held.insert(request.receiver, previous);
                        }
                        None => {
                            trees::capture_tree(
                                memory,
                                accumulator,
                                &definitions,
                                &mut self.nodes,
                                &mut self.text,
                                &mut self.stack,
                            )?;
                            counts.read += 1;
                            if let Some(fresh) = self.held_copy(first, accumulator, certificate)? {
                                store.held.insert(request.receiver, fresh);
                            }
                        }
                    }
                }
                None => {
                    trees::capture_tree(
                        memory,
                        accumulator,
                        &definitions,
                        &mut self.nodes,
                        &mut self.text,
                        &mut self.stack,
                    )?;
                    counts.uncarried += 1;
                }
            }
            grow(
                &mut self.roots,
                Root {
                    accumulator,
                    nodes: [first, self.nodes.len() - first],
                },
            )?;
            if self.oracle {
                let fresh_first = self.fresh_nodes.len();
                trees::capture_tree(
                    memory,
                    accumulator,
                    &definitions,
                    &mut self.fresh_nodes,
                    &mut self.fresh_text,
                    &mut self.stack,
                )?;
                grow(
                    &mut self.fresh_roots,
                    Root {
                        accumulator,
                        nodes: [fresh_first, self.fresh_nodes.len() - fresh_first],
                    },
                )?;
            }
            for _ in at..end {
                grow(&mut self.root_of, root)?;
            }
            at = end;
        }
        drop(held);
        Ok(())
    }

    fn copy_held(&mut self, held: &Held) -> Result<(), &'static str> {
        let offset = self.text.len();
        engine_allocation::reserve(&mut self.text, held.text.len())
            .map_err(|_| "collector allocation failed")?;
        self.text.extend_from_slice(&held.text);
        engine_allocation::reserve(&mut self.nodes, held.nodes.len())
            .map_err(|_| "collector allocation failed")?;
        for node in &held.nodes {
            let mut node = *node;
            node.name[0] += offset;
            self.nodes.push(node);
        }
        Ok(())
    }

    fn held_copy(
        &self,
        first: usize,
        accumulator: usize,
        certificate: Option<(u64, u64)>,
    ) -> Result<Option<Held>, &'static str> {
        let Some(certificate) = certificate else {
            return Ok(None);
        };
        let nodes = &self.nodes[first..];
        let mut held = Held {
            certificate,
            accumulator,
            nodes: engine_allocation::vec(nodes.len())
                .map_err(|_| "collector allocation failed")?,
            text: Vec::new(),
        };
        for node in nodes {
            let mut copy = *node;
            let span = self
                .text
                .get(node.name[0]..node.name[0] + node.name[1])
                .ok_or("naval tree display span invalid")?;
            copy.name[0] = held.text.len();
            engine_allocation::reserve(&mut held.text, span.len())
                .map_err(|_| "collector allocation failed")?;
            held.text.extend_from_slice(span);
            held.nodes.push(copy);
        }
        Ok(Some(held))
    }

    pub fn tables(&self) -> [TableView; 2] {
        let samples = if self.oracle {
            match self.outcome {
                Outcome::Unavailable { reason } => unavailable(reason),
                _ => match self.project_samples() {
                    Ok(rows) => TableView::Complete(TableRows::NavalModifierTreeSamples(rows)),
                    Err(reason) => unavailable(reason),
                },
            }
        } else {
            TableView::Complete(TableRows::NavalModifierTreeSamples(Vec::new()))
        };
        let trees = match self.outcome {
            Outcome::Unavailable { reason } => unavailable(reason),
            _ => match self.project() {
                Ok(rows) => TableView::Complete(TableRows::NavalModifierTrees(rows)),
                Err(reason) => unavailable(reason),
            },
        };
        [trees, samples]
    }

    fn project(&self) -> Result<Vec<rows::NavalModifierTreeRow>, &'static str> {
        let mut owners: BTreeMap<(u8, u64, u32), usize> = BTreeMap::new();
        for (request, &root) in self.requests.iter().zip(&self.root_of) {
            let owner = (request.kind, request.owner, request.sub);
            if *owners.entry(owner).or_insert(root) != root {
                return Err("naval tree owner names two accumulators");
            }
        }
        let mut rows = Vec::new();
        engine_allocation::reserve(&mut rows, owners.len())
            .map_err(|_| "collector allocation failed")?;
        for ((kind, owner, sub), root) in owners {
            let root = self.roots[root];
            let trees = project(range(&self.nodes, root.nodes), &self.text)?;
            if !trees.is_empty() || kind == Kind::SideWeather as u8 {
                rows.push(rows::NavalModifierTreeRow {
                    kind,
                    owner,
                    sub,
                    trees,
                });
            }
        }
        Ok(rows)
    }

    fn project_samples(&self) -> Result<Vec<rows::NavalModifierTreeSampleRow>, &'static str> {
        let mut rows = Vec::new();
        engine_allocation::reserve(&mut rows, self.requests.len())
            .map_err(|_| "collector allocation failed")?;
        for (request, &root) in self.requests.iter().zip(&self.root_of) {
            let root = self
                .fresh_roots
                .get(root)
                .ok_or("naval tree sample root missing")?;
            rows.push(rows::NavalModifierTreeSampleRow {
                combat: request.combat,
                side: request.side,
                battle_slot: request.battle_slot,
                ordinal: request.ordinal,
                scope: request.scope,
                kind: request.kind,
                owner: request.owner,
                sub: request.sub,
                trees: project(range(&self.fresh_nodes, root.nodes), &self.fresh_text)?,
            });
        }
        rows.sort_unstable_by_key(|row| row.key());
        if rows.windows(2).any(|pair| pair[0].key() == pair[1].key()) {
            return Err("naval tree sample occurrence repeated");
        }
        Ok(rows)
    }
}

fn range<T>(items: &[T], span: [usize; 2]) -> &[T] {
    &items[span[0]..span[0] + span[1]]
}

pub fn project(
    nodes: &[RawNode],
    text: &[u8],
) -> Result<Vec<rows::NavalModifierKeyTree>, &'static str> {
    project_keys(nodes, text, &KEYS)
}

pub fn project_keys<const N: usize>(
    nodes: &[RawNode<N>],
    text: &[u8],
    keys: &[u32; N],
) -> Result<Vec<rows::NavalModifierKeyTree>, &'static str> {
    let Some(root) = nodes.first() else {
        return Err("naval modifier root has no nodes");
    };
    let mut trees = Vec::new();
    for (lane, &key) in keys.iter().enumerate() {
        let mut projected = Vec::new();
        let mut cursor = 0;
        copy_visible(nodes, text, &mut cursor, lane, true, &mut projected)?;
        if cursor != nodes.len() {
            return Err("naval modifier tree has extra nodes");
        }
        if root.children != 0 && projected[0].children == 0 {
            projected[0].children = 1;
            projected.push(rows::ModifierBreakdownNode::default());
        }
        let prints = projected
            .iter()
            .any(|node| node.children == 0 && node.value != 0);
        if projected[0].value == 0 && !prints {
            continue;
        }
        engine_allocation::reserve(&mut trees, 1).map_err(|_| "collector allocation failed")?;
        trees.push(rows::NavalModifierKeyTree {
            key,
            nodes: projected,
        });
    }
    Ok(trees)
}

fn copy_visible<const N: usize>(
    nodes: &[RawNode<N>],
    text: &[u8],
    cursor: &mut usize,
    lane: usize,
    root: bool,
    output: &mut Vec<rows::ModifierBreakdownNode>,
) -> Result<bool, &'static str> {
    let node = *nodes
        .get(*cursor)
        .ok_or("naval modifier preorder is incomplete")?;
    *cursor += 1;
    let start = output.len();
    engine_allocation::reserve(output, 1).map_err(|_| "collector allocation failed")?;
    output.push(rows::ModifierBreakdownNode::default());
    let mut kept = 0_u32;
    for _ in 0..node.children {
        if copy_visible(nodes, text, cursor, lane, false, output)? {
            kept += 1;
        }
    }
    let value = node.values[lane];
    if !root && (node.children != 0 || value == 0) && kept == 0 {
        output.truncate(start);
        return Ok(false);
    }
    let name = if node.children == 0 && value != 0 {
        let span = text
            .get(node.name[0]..node.name[0] + node.name[1])
            .ok_or("naval modifier display span invalid")?;
        engine_allocation::copy_vec(span).map_err(|_| "collector allocation failed")?
    } else {
        Vec::new()
    };
    output[start] = rows::ModifierBreakdownNode {
        children: kept,
        value,
        name,
    };
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_projection_keeps_only_printable_escort_sources() {
        let key = engine_executable::modifier_keys::CONVOY_ESCORT_EFFICIENCY;
        let nodes = [
            RawNode::<1> {
                children: 2,
                values: [25_000],
                ..Default::default()
            },
            RawNode {
                name: [0, 4],
                values: [25_000],
                ..Default::default()
            },
            RawNode {
                name: [usize::MAX, usize::MAX],
                ..Default::default()
            },
        ];
        let projected = project_keys(&nodes, b"Idea", &[key]).unwrap();
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].key, key);
        assert_eq!(projected[0].nodes.len(), 2);
        assert_eq!(projected[0].nodes[0].children, 1);
        assert_eq!(projected[0].nodes[1].name, b"Idea");
    }

    fn lane(key: u32) -> usize {
        KEYS.iter().position(|&candidate| candidate == key).unwrap()
    }

    fn node(children: u32, name: [usize; 2], values: &[(u32, i64)]) -> RawNode {
        let mut out = RawNode {
            children,
            name,
            values: [0; KEYS.len()],
        };
        for &(key, value) in values {
            out.values[lane(key)] = value;
        }
        out
    }

    #[test]
    fn projection_prunes_per_key_and_keeps_interior_roots_interior() {
        let text = b"zerokept";
        let nodes = [
            node(2, [0, 0], &[(0x1b2, 900), (0x5b, 77)]),
            node(0, [0, 4], &[(0x5b, 77)]),
            node(0, [4, 4], &[(0x1b2, 900)]),
        ];
        let trees = project(&nodes, text).unwrap();
        assert_eq!(
            trees.iter().map(|tree| tree.key).collect::<Vec<_>>(),
            [0x5b, 0x1b2]
        );
        let positioning = &trees[1].nodes;
        assert_eq!(positioning.len(), 2);
        assert_eq!(positioning[0].children, 1);
        assert_eq!(positioning[0].value, 900);
        assert!(positioning[0].name.is_empty());
        assert_eq!(positioning[1].name, b"kept");
        assert_eq!(positioning[1].value, 900);
        let nodes = [node(1, [0, 0], &[(0x24, 123)]), node(0, [0, 0], &[])];
        let trees = project(&nodes, text).unwrap();
        assert_eq!(trees.len(), 1);
        assert_eq!(trees[0].nodes[0].children, 1);
        assert_eq!(trees[0].nodes[0].value, 123);
        assert_eq!(trees[0].nodes[1], rows::ModifierBreakdownNode::default());
        let trees = project(&[node(0, [4, 4], &[(0x5a, -5)])], text).unwrap();
        assert_eq!(trees[0].key, 0x5a);
        assert_eq!(trees[0].nodes[0].name, b"kept");
    }

    struct Fixed(Option<(u64, u64)>);
    impl Changes for Fixed {
        fn sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
            self.0
        }
    }

    #[test]
    fn a_certified_country_tree_is_copied_and_a_moved_one_is_read() {
        let mut trees = Trees::default();
        trees.requests.push(Request {
            accumulator: 0x15b8,
            receiver: 0x1000,
            kind: Kind::Country as u8,
            owner: 7,
            ..Default::default()
        });
        let mut store = Box::new(Store::default());
        store.held.insert(
            0x1000,
            Held {
                certificate: (1, 4),
                accumulator: 0x15b8,
                nodes: vec![node(0, [0, 5], &[(0x1b2, 5_000)])],
                text: b"Ideas".to_vec(),
            },
        );
        let roots = trees.copy_certified(&mut store, &Fixed(Some((1, 4))));
        assert_eq!(roots, Some(1));
        let projected = project(range(&trees.nodes, trees.roots[0].nodes), &trees.text).unwrap();
        assert_eq!(projected[0].nodes[0].name, b"Ideas");
        assert!(store.held.contains_key(&0x1000));
        let mut trees = Trees::default();
        trees.requests.push(Request {
            accumulator: 0x15b8,
            receiver: 0x1000,
            kind: Kind::Country as u8,
            owner: 7,
            ..Default::default()
        });
        assert_eq!(trees.copy_certified(&mut store, &Fixed(Some((1, 6)))), None);
    }

    impl Trees {
        fn copy_certified(&mut self, store: &mut Store, changes: &impl Changes) -> Option<usize> {
            let requests = self.requests.clone();
            for request in requests {
                let certificate = changes.sequence(request.receiver)?;
                let held = store.held.remove(&request.receiver)?;
                if held.certificate != certificate || held.accumulator != request.accumulator {
                    store.held.insert(request.receiver, held);
                    return None;
                }
                let first = self.nodes.len();
                self.copy_held(&held).ok()?;
                self.roots.push(Root {
                    accumulator: request.accumulator,
                    nodes: [first, self.nodes.len() - first],
                });
                self.root_of.push(self.roots.len() - 1);
                store.held.insert(request.receiver, held);
            }
            Some(self.roots.len())
        }
    }

    #[test]
    fn the_placeholder_owns_no_storage_and_records_no_samples_without_the_oracle() {
        let trees = Trees::default();
        assert_eq!(trees.allocated_bytes(), 0);
        let [refused, samples] = trees.tables();
        assert!(matches!(refused, TableView::Unavailable { .. }));
        if !naval_tree_changes::ORACLE {
            assert_eq!(
                samples,
                TableView::Complete(TableRows::NavalModifierTreeSamples(Vec::new()))
            );
            assert_eq!(trees.samples_outcome(), Outcome::Rows { len: 0 });
        }
    }
}
