
use super::Columns;
use viewer_layer_core::{layers::{List, Num, Number, Text}, owned::{Standing, Table}, playhead_host::mirror::{Cell, Mirror, Mirrors}};

pub(super) trait Source {
    fn live(&self, slot: u32) -> bool;
    fn key(&self, slot: u32) -> Option<u128>;
    fn number<T: Number>(&self, column: Num<T>, slot: u32) -> Cell<T>;
    fn list(&self, column: List, slot: u32) -> Cell<&[u8]>;
    fn text(&self, column: Text, slot: u32) -> Result<Cell<&[u8]>, ()>;
    fn format(&self) -> u16;
}

impl Source for Mirror {
    fn live(&self, slot: u32) -> bool {
        Mirror::live(self, slot)
    }

    fn key(&self, slot: u32) -> Option<u128> {
        Mirror::key(self, slot)
    }

    fn number<T: Number>(&self, column: Num<T>, slot: u32) -> Cell<T> {
        Mirror::number(self, column, slot)
    }

    fn list(&self, column: List, slot: u32) -> Cell<&[u8]> {
        Mirror::list(self, column, slot)
    }

    fn text(&self, column: Text, slot: u32) -> Result<Cell<&[u8]>, ()> {
        match Mirror::text(self, column, slot) {
            Cell::Value(id) => self.string(id).map(Cell::Value).ok_or(()),
            Cell::Missing => Ok(Cell::Missing),
            Cell::Unrecorded => Ok(Cell::Unrecorded),
        }
    }

    fn format(&self) -> u16 {
        Mirror::format(self)
    }
}

impl Source for Table {
    fn live(&self, slot: u32) -> bool {
        Table::live(self, slot)
    }

    fn key(&self, slot: u32) -> Option<u128> {
        Table::key(self, slot)
    }

    fn number<T: Number>(&self, column: Num<T>, slot: u32) -> Cell<T> {
        Table::number(self, column, slot)
    }

    fn list(&self, column: List, slot: u32) -> Cell<&[u8]> {
        Table::list(self, column, slot)
    }

    fn text(&self, column: Text, slot: u32) -> Result<Cell<&[u8]>, ()> {
        self.named_text(column, slot).map_err(drop)
    }

    fn format(&self) -> u16 {
        Table::format(self)
    }
}
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::collections::{BTreeSet, HashMap};

fn bytes<S: Source>(rows: &S, column: Text, slot: u32) -> Option<Vec<u8>> {
    match rows.text(column, slot) {
        Ok(Cell::Value(bytes)) => Some(bytes.to_vec()),
        Ok(_) => Some(Vec::new()),
        Err(()) => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Encoded {
    pub(super) cell: Cell<Vec<u8>>,
    pub(super) format: u16,
}

fn opaque<S: Source>(rows: &S, column: List, slot: u32) -> Encoded {
    let cell = match rows.list(column, slot) {
        Cell::Value(bytes) => Cell::Value(bytes.to_vec()),
        Cell::Missing => Cell::Missing,
        Cell::Unrecorded => Cell::Unrecorded,
    };
    Encoded { cell, format: rows.format() }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Node {
    pub(super) province: u32,
    pub(super) kind: u32,
    pub(super) flag: u8,
    pub(super) available: i64,
    pub(super) consumed: i64,
    pub(super) parent: i32,
    pub(super) route_from: u16,
    pub(super) route_to: u16,
    pub(super) break_after: u8,
    pub(super) path: Vec<u32>,
}

impl Node {
    pub(super) fn same_but_supply(&self, other: &Self) -> bool {
        Self { available: 0, consumed: 0, ..self.clone() } == Self { available: 0, consumed: 0, ..other.clone() }
    }
}

#[derive(Clone, Debug)]
pub(super) enum NodeCopy {
    Whole(Node),
    Census { province: u32, kind: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Country {
    pub(super) slot: Option<u32>,
    pub(super) tag: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Appearance {
    pub(super) reference: Cell<i32>,
    pub(super) original: Encoded,
    pub(super) culture: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Inventory {
    pub(super) node_count: u32,
    pub(super) allocations: Encoded,
    pub(super) access: Encoded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Reserved {
    pub(super) tag: Option<Vec<u8>>,
    pub(super) original: Encoded,
    pub(super) reference: Cell<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Meta {
    pub(super) held: bool,
    pub(super) status: Option<Status>,
    pub(super) refusal: Option<String>,
    pub(super) head: Option<Hour>,
}

impl Meta {
    fn of(mirror: Option<&Mirror>) -> Self {
        mirror.map_or_else(Self::default, |mirror| Self { held: true, status: mirror.status(), refusal: mirror.not_recorded(), head: mirror.head() })
    }

    pub(super) fn of_table(table: Option<&Table>) -> Self {
        table.map_or_else(Self::default, |table| Self { held: true, status: table.status(), refusal: table.not_recorded(), head: table.head() })
    }

    pub(super) fn of_standing(standing: Option<&Standing>) -> Self {
        standing.map_or_else(Self::default, |standing| Self { held: true, status: standing.status(), refusal: standing.not_recorded(), head: standing.head() })
    }

    pub(super) fn readable(&self) -> bool {
        self.held && matches!(self.status, Some(Status::Complete | Status::Truncated { .. }))
    }

    pub(super) fn complete(&self) -> bool {
        self.held && self.status == Some(Status::Complete)
    }
}

#[derive(Debug)]
pub(super) struct Rows<V> {
    pub(super) meta: Meta,
    by_key: HashMap<u128, (u32, V)>,
    by_slot: HashMap<u32, u128>,
}

impl<V> Default for Rows<V> {
    fn default() -> Self {
        Self { meta: Meta::default(), by_key: HashMap::new(), by_slot: HashMap::new() }
    }
}

impl<V: Clone + PartialEq> Rows<V> {
    pub(super) fn get(&self, key: u128) -> Option<&V> {
        self.by_key.get(&key).map(|(_, value)| value)
    }

    pub(super) fn row(&self, key: u128) -> Option<&V> {
        self.meta.readable().then(|| self.get(key)).flatten()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (u128, &V)> + '_ {
        self.by_key.iter().map(|(&key, (_, value))| (key, value))
    }

    fn take<C>(&mut self, copies: HashMap<u32, Option<(u128, C)>>, merge: impl Fn(Option<&V>, C) -> V) -> Vec<(u128, Option<V>)> {
        let mut before: HashMap<u128, Option<V>> = HashMap::new();
        for (slot, copy) in copies {
            let incoming = copy.as_ref().map(|&(key, _)| key);
            if let Some(&key) = self.by_slot.get(&slot)
                && Some(key) != incoming
            {
                self.by_slot.remove(&slot);
                if self.by_key.get(&key).is_some_and(|&(held, _)| held == slot) {
                    let (_, old) = self.by_key.remove(&key).expect("held above");
                    before.entry(key).or_insert(Some(old));
                }
            }
            let Some((key, copy)) = copy else { continue };
            let old = self.by_key.remove(&key);
            if let Some((held, _)) = &old
                && *held != slot
                && self.by_slot.get(held) == Some(&key)
            {
                self.by_slot.remove(held);
            }
            let value = merge(old.as_ref().map(|(_, value)| value), copy);
            before.entry(key).or_insert(old.map(|(_, value)| value));
            self.by_key.insert(key, (slot, value));
            self.by_slot.insert(slot, key);
        }
        before.into_iter().filter(|(key, old)| old.as_ref() != self.get(*key)).collect()
    }

    fn bytes(&self, each: usize) -> usize {
        self.by_key.capacity() * (each + 24) + self.by_slot.capacity() * 24
    }
}

#[derive(Debug, Default)]
pub(super) struct Store {
    pub(super) countries: Rows<Country>,
    pub(super) links: Rows<Option<u32>>,
    pub(super) appearance: Rows<Appearance>,
    pub(super) tokens: Rows<Option<Vec<u8>>>,
    pub(super) provinces: Rows<Option<u32>>,
    pub(super) reserved: Rows<Reserved>,
    pub(super) nodes: Rows<Node>,
    pub(super) inventory: Rows<Inventory>,
    pub(super) variants: Rows<(u32, u32)>,
    pub(super) edges: Rows<()>,
    pub(super) cooldowns: Rows<i32>,
    pub(super) supply: Meta,
    pub(super) naval: Meta,
    pub(super) copied: HashMap<u128, Hour>,
}

pub(super) const ROWS: [TableId; 11] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::Provinces,
    TableId::ReservedCountryNaming,
    TableId::SupplyNodes,
    TableId::TrainInventory,
    TableId::VariantDefinitions,
    TableId::RailEdges,
    TableId::RailCooldowns,
];

#[derive(Default)]
pub(super) struct Moved {
    pub(super) countries: Vec<(u128, Option<Country>)>,
    pub(super) links: Vec<(u128, Option<Option<u32>>)>,
    pub(super) appearance: Vec<(u128, Option<Appearance>)>,
    pub(super) tokens: Vec<(u128, Option<Option<Vec<u8>>>)>,
    pub(super) provinces: Vec<(u128, Option<Option<u32>>)>,
    pub(super) reserved: bool,
    pub(super) nodes: Vec<(u128, Option<Node>)>,
    pub(super) inventory: Vec<(u128, Option<Inventory>)>,
    pub(super) variants: Vec<(u128, Option<(u32, u32)>)>,
    pub(super) edges: Vec<(u128, Option<()>)>,
    pub(super) cooldowns: Vec<(u128, Option<i32>)>,
    pub(super) restated: Vec<TableId>,
}

pub(super) fn class(status: Option<Status>) -> Option<Status> {
    match status {
        Some(Status::Truncated { .. }) => Some(Status::Truncated { missing: 0 }),
        other => other,
    }
}

impl Store {
    pub(super) fn from_mirrors(mirrors: &Mirrors, columns: &Columns, whole: Option<u32>) -> Self {
        let mut batch = Batch::default();
        for table in ROWS {
            if let Some(mirror) = mirrors.get(table) {
                for slot in mirror.rows() {
                    batch.copy(table, slot, mirror, columns, whole);
                }
            }
        }
        batch.metas(mirrors);
        let mut store = Self::default();
        store.take(batch);
        store
    }

    fn meta_mut(&mut self, table: TableId) -> Option<&mut Meta> {
        Some(match table {
            TableId::Countries => &mut self.countries.meta,
            TableId::CountryLinks => &mut self.links.meta,
            TableId::CountryAppearance => &mut self.appearance.meta,
            TableId::LexicalTokens => &mut self.tokens.meta,
            TableId::Provinces => &mut self.provinces.meta,
            TableId::ReservedCountryNaming => &mut self.reserved.meta,
            TableId::SupplyNodes => &mut self.nodes.meta,
            TableId::TrainInventory => &mut self.inventory.meta,
            TableId::VariantDefinitions => &mut self.variants.meta,
            TableId::RailEdges => &mut self.edges.meta,
            TableId::RailCooldowns => &mut self.cooldowns.meta,
            TableId::ProvinceSupply => &mut self.supply,
            TableId::NavalBases => &mut self.naval,
            _ => return None,
        })
    }

    pub(super) fn meta(&self, table: TableId) -> Option<&Meta> {
        Some(match table {
            TableId::Countries => &self.countries.meta,
            TableId::CountryLinks => &self.links.meta,
            TableId::CountryAppearance => &self.appearance.meta,
            TableId::LexicalTokens => &self.tokens.meta,
            TableId::Provinces => &self.provinces.meta,
            TableId::ReservedCountryNaming => &self.reserved.meta,
            TableId::SupplyNodes => &self.nodes.meta,
            TableId::TrainInventory => &self.inventory.meta,
            TableId::VariantDefinitions => &self.variants.meta,
            TableId::RailEdges => &self.edges.meta,
            TableId::RailCooldowns => &self.cooldowns.meta,
            TableId::ProvinceSupply => &self.supply,
            TableId::NavalBases => &self.naval,
            _ => return None,
        })
    }

    pub(super) fn take(&mut self, batch: Batch) -> Moved {
        let mut moved = Moved::default();
        if let Some(at) = batch.at {
            for (key, _) in batch.nodes.values().flatten() {
                self.copied.insert(*key, at);
            }
        }
        for (table, meta) in batch.metas {
            if let Some(held) = self.meta_mut(table) {
                if (held.held, class(held.status)) != (meta.held, class(meta.status)) {
                    moved.restated.push(table);
                }
                *held = meta;
            }
        }
        fn replace<V>(_: Option<&V>, value: V) -> V {
            value
        }
        moved.countries = self.countries.take(batch.countries, replace);
        moved.links = self.links.take(batch.links, replace);
        moved.appearance = self.appearance.take(batch.appearance, replace);
        moved.tokens = self.tokens.take(batch.tokens, replace);
        moved.provinces = self.provinces.take(batch.provinces, replace);
        moved.reserved = !self.reserved.take(batch.reserved, replace).is_empty();
        moved.nodes = self.nodes.take(batch.nodes, |old, copy| match copy {
            NodeCopy::Whole(node) => node,
            NodeCopy::Census { province, kind } => Node { province, kind, ..old.cloned().unwrap_or_default() },
        });
        moved.inventory = self.inventory.take(batch.inventory, replace);
        moved.variants = self.variants.take(batch.variants, replace);
        moved.edges = self.edges.take(batch.edges, replace);
        moved.cooldowns = self.cooldowns.take(batch.cooldowns, replace);
        moved
    }

    pub(super) fn bytes(&self) -> usize {
        let paths: usize = self.nodes.iter().map(|(_, node)| node.path.capacity() * 4).sum();
        self.countries.bytes(48)
            + self.links.bytes(8)
            + self.appearance.bytes(96)
            + self.tokens.bytes(32)
            + self.provinces.bytes(8)
            + self.reserved.bytes(96)
            + self.nodes.bytes(72)
            + paths
            + self.inventory.bytes(72)
            + self.variants.bytes(8)
            + self.edges.bytes(0)
            + self.cooldowns.bytes(4)
    }
}

#[derive(Debug, Default)]
pub(super) struct Forget {
    pub(super) all: bool,
    pub(super) provinces: BTreeSet<u32>,
}

#[derive(Default)]
pub(super) struct Batch {
    metas: Vec<(TableId, Meta)>,
    countries: HashMap<u32, Option<(u128, Country)>>,
    links: HashMap<u32, Option<(u128, Option<u32>)>>,
    appearance: HashMap<u32, Option<(u128, Appearance)>>,
    tokens: HashMap<u32, Option<(u128, Option<Vec<u8>>)>>,
    provinces: HashMap<u32, Option<(u128, Option<u32>)>>,
    reserved: HashMap<u32, Option<(u128, Reserved)>>,
    nodes: HashMap<u32, Option<(u128, NodeCopy)>>,
    inventory: HashMap<u32, Option<(u128, Inventory)>>,
    variants: HashMap<u32, Option<(u128, (u32, u32))>>,
    edges: HashMap<u32, Option<(u128, ())>>,
    cooldowns: HashMap<u32, Option<(u128, i32)>>,
    pub(super) forget: Forget,
    pub(super) restated: bool,
    pub(super) at: Option<Hour>,
    pub(super) seen: [u64; 3],
}

impl Batch {
    pub(super) fn node_copies(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn rows(&self) -> usize {
        self.countries.len()
            + self.links.len()
            + self.appearance.len()
            + self.tokens.len()
            + self.provinces.len()
            + self.reserved.len()
            + self.nodes.len()
            + self.inventory.len()
            + self.variants.len()
            + self.edges.len()
            + self.cooldowns.len()
    }

    pub(super) fn drop_rows(&mut self) {
        *self = Self { forget: std::mem::take(&mut self.forget), ..Self::default() };
    }

    pub(super) fn metas(&mut self, mirrors: &Mirrors) {
        self.metas = ROWS.into_iter().chain([TableId::ProvinceSupply, TableId::NavalBases]).map(|table| (table, Meta::of(mirrors.get(table)))).collect();
    }

    pub(super) fn metas_of(&mut self, metas: Vec<(TableId, Meta)>) {
        self.metas = metas;
    }

    pub(super) fn copy<S: Source>(&mut self, table: TableId, slot: u32, mirror: &S, columns: &Columns, whole: Option<u32>) {
        let key = mirror.live(slot).then(|| mirror.key(slot)).flatten();
        macro_rules! put {
            ($rows:ident, $value:expr) => {{
                self.$rows.insert(slot, key.map(|key| (key, $value)));
            }};
        }
        match table {
            TableId::Countries => put!(countries, Country { slot: mirror.number(columns.country_slot, slot).value(), tag: bytes(mirror, columns.tag, slot) }),
            TableId::CountryLinks => put!(links, mirror.number(columns.link_slot, slot).value()),
            TableId::CountryAppearance => put!(appearance, Appearance { reference: mirror.number(columns.reference, slot), original: opaque(mirror, columns.original_tag, slot), culture: bytes(mirror, columns.culture, slot) }),
            TableId::LexicalTokens => put!(
                tokens,
                match mirror.text(columns.literal, slot) {
                    Ok(Cell::Value(bytes)) => Some(bytes.to_vec()),
                    _ => None,
                }
            ),
            TableId::Provinces => put!(provinces, mirror.number(columns.controller, slot).value()),
            TableId::ReservedCountryNaming => put!(reserved, Reserved { tag: bytes(mirror, columns.reserved_tag, slot), original: opaque(mirror, columns.reserved_original, slot), reference: mirror.number(columns.reserved_reference, slot) }),
            TableId::SupplyNodes => {
                let ours = key.is_some_and(|key| Some((key >> 32) as u32) == whole);
                let province = mirror.number(columns.node_province, slot).value().unwrap_or(0);
                let kind = mirror.number(columns.node_kind, slot).value().unwrap_or(0);
                put!(
                    nodes,
                    if ours {
                        NodeCopy::Whole(Node {
                            province,
                            kind,
                            flag: mirror.number(columns.node_flag, slot).value().unwrap_or(0),
                            available: mirror.number(columns.available, slot).value().unwrap_or(0),
                            consumed: mirror.number(columns.consumed, slot).value().unwrap_or(0),
                            parent: mirror.number(columns.parent, slot).value().unwrap_or(0),
                            route_from: mirror.number(columns.route_from, slot).value().unwrap_or(0),
                            route_to: mirror.number(columns.route_to, slot).value().unwrap_or(0),
                            break_after: mirror.number(columns.break_after, slot).value().unwrap_or(0),
                            path: match mirror.list(columns.path, slot) {
                                Cell::Value(bytes) => bytes.chunks_exact(4).map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]])).collect(),
                                _ => Vec::new(),
                            },
                        })
                    } else {
                        NodeCopy::Census { province, kind }
                    }
                )
            }
            TableId::TrainInventory => put!(inventory, Inventory { node_count: mirror.number(columns.node_count, slot).value().unwrap_or(0), allocations: opaque(mirror, columns.allocations, slot), access: opaque(mirror, columns.access, slot) }),
            TableId::VariantDefinitions => put!(variants, (mirror.number(columns.equipment_token, slot).value().unwrap_or(0), mirror.number(columns.creator_link, slot).value().unwrap_or(0))),
            TableId::RailEdges => put!(edges, ()),
            TableId::RailCooldowns => put!(cooldowns, mirror.number(columns.cooldown, slot).value().unwrap_or(0)),
            _ => {}
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.rows() * 64 + self.forget.provinces.len() * 24
    }
}
