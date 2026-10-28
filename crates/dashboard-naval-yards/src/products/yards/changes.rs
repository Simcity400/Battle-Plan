//! A yard view's hour as its changes: only the ports, lines, ships and totals they reach are projected again.
use super::{management::TABLES, *};
use crate::products::{naval_refits, naval_repairs};
use dashboard_base::prepared::prepass::{Held, HeldBytes};
use dashboard_keyed::delta::{Delta, at_key, changed_rows, key_range, rows as view_rows};
use replay_data::rows::{NavalBaseRow, RowCodec, ShipRow, TableSchema};
use replay_frame::frame::{SharedView, TableView};
use std::collections::HashSet;

/// Tables that name owners, designs, classes and ports.
const NAMING: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::Provinces,
    TableId::States,
    TableId::ProvinceNames,
];
/// Tables only refits read.
const REFITS_ONLY: &[TableId] = &[
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::ProductionLines,
];
/// Tables only repairs read.
const REPAIRS_ONLY: &[TableId] = &[
    TableId::NavalRepairCountries,
    TableId::CountryFactories,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::Provinces,
    TableId::States,
    TableId::ProvinceNames,
];

fn reads(table: TableId, refits: bool) -> bool {
    !(if refits { REPAIRS_ONLY } else { REFITS_ONLY }).contains(&table)
}

fn at(table: TableId) -> usize {
    TABLES
        .iter()
        .position(|held| *held == table)
        .expect("management table")
}

/// Whether every table a view reads is complete in `frame`.
fn complete(frame: &DecodedFrame, refits: bool) -> bool {
    TABLES.iter().filter(|table| reads(**table, refits)).all(|&table| {
        matches!(&*frame.tables[table.index()], TableView::Complete(rows) if rows.table() == table)
    })
}

/// A value a name holds: a number, or a naming operand interned when published.
#[derive(Clone)]
pub(super) enum Value {
    Plain(i128),
    Label(Arc<Label>),
}

pub(super) struct Label {
    /// The operand's interning key.
    key: String,
    operand: Operand,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Plain(a), Self::Plain(b)) => a == b,
            (Self::Label(a), Self::Label(b)) => a.key == b.key,
            _ => false,
        }
    }
}

/// Names (or name suffixes) and their values, sorted, each once.
type Entries = Vec<(String, Value)>;

/// What a projection pushes for one port, line or country: values and refusals.
#[derive(Default)]
pub(super) struct Part {
    entries: Entries,
    refusals: Vec<(String, String, u64)>,
}

/// One ship's projection, shared by every port or line holding it.
struct Ship {
    /// Name suffixes after the holder's ship prefix.
    entries: Entries,
    refusals: Vec<(String, u64)>,
    /// The task force handles it reads: its own, a rejoin target and its children.
    tasks: Vec<u64>,
    /// The variant handles it reads VariantDefinitions at, ascending.
    variants: Vec<u64>,
}

impl State {
    /// Start projecting one part afresh: no tokens or refusal counts of another.
    fn begin_part(&mut self) {
        self.tokens.clear();
        self.index.clear();
        self.refusal_counts.clear();
        self.ship_refusals = None;
    }

    /// The part pushed since [`Self::begin_part`]: each name's last value, labels by operand.
    fn part(&self, values: &Values) -> Part {
        let mut entries = BTreeMap::new();
        let mut refusals = BTreeMap::new();
        for (name, value) in values.iter() {
            if let Some((owner, id)) = name.split_once("|refusal|") {
                let why = id
                    .parse::<usize>()
                    .ok()
                    .and_then(|id| match self.tokens.get(id) {
                        Some(Operand::Refusal(why)) => Some(why.clone()),
                        _ => None,
                    })
                    .expect("projected refusal token");
                refusals.insert((owner.to_owned(), why), value as u64);
            } else {
                entries.insert(name.to_owned(), self.value(name, value));
            }
        }
        Part {
            entries: entries.into_iter().collect(),
            refusals: refusals
                .into_iter()
                .map(|((owner, why), count)| (owner, why, count))
                .collect(),
        }
    }

    fn value(&self, name: &str, value: i128) -> Value {
        if is_label(name.rsplit('|').next().unwrap_or_default()) {
            let operand = self.tokens[value as usize].clone();
            Value::Label(Arc::new(Label {
                key: serde_json::to_string(&operand).expect("serializable naming operand"),
                operand,
            }))
        } else {
            Value::Plain(value)
        }
    }

    fn ship_item(&mut self, frame: &DecodedFrame, ship: u64) -> Ship {
        self.begin_part();
        self.ship_refusals = Some(Vec::new());
        self.collect_variants();
        let mut values = Values::default();
        self.project_ship(frame, &mut values, ship);
        let variants = self.collected_variants();
        let collected = self.ship_refusals.take().unwrap_or_default();
        let mut entries = BTreeMap::new();
        for (name, value) in values.iter() {
            entries.insert(name.to_owned(), self.value(name, value));
        }
        let mut refusals = BTreeMap::<String, u64>::new();
        for (id, count) in collected {
            if let Some(Operand::Refusal(why)) = self.tokens.get(id) {
                *refusals.entry(why.clone()).or_default() += count as u64;
            }
        }
        let row = find(frame.complete::<tables::Ships>().ok(), ship, |row| row.id);
        let mut tasks = Vec::new();
        if let Some(row) = row {
            tasks.push(row.task_force);
            if let Some(order) = find(
                frame.complete::<tables::NavalRepairTaskForces>().ok(),
                row.task_force,
                |row| row.task_force,
            ) {
                tasks.push(order.merge_after_repair);
            }
            if let Ok(children) = frame.complete::<tables::NavalRepairChildren>() {
                let first = u128::from(row.task_force) << 32;
                tasks.extend(
                    key_range(children, first..first + (1 << 32))
                        .iter()
                        .map(|child| child.child),
                );
            }
        }
        tasks.sort_unstable();
        tasks.dedup();
        Ship {
            entries: entries.into_iter().collect(),
            refusals: refusals.into_iter().collect(),
            tasks,
            variants,
        }
    }
}

/// The names and refusal totals an hour changed, with each total before it.
#[derive(Default)]
pub(super) struct Changes {
    names: BTreeMap<String, Option<Value>>,
    refusals: BTreeMap<(String, String), u64>,
}

impl Changes {
    /// The changes from `old` to `new` entries, each name `prefix` and its suffix.
    fn entries(&mut self, prefix: &str, old: &[(String, Value)], new: &[(String, Value)]) {
        let (mut a, mut b) = (0, 0);
        while a < old.len() || b < new.len() {
            match (old.get(a), new.get(b)) {
                (Some((x, before)), Some((y, now))) if x == y => {
                    if before != now {
                        self.names.insert(format!("{prefix}{y}"), Some(now.clone()));
                    }
                    a += 1;
                    b += 1;
                }
                (Some((x, _)), now) if now.is_none_or(|(y, _)| x < y) => {
                    self.names.insert(format!("{prefix}{x}"), None);
                    a += 1;
                }
                (_, Some((y, now))) => {
                    self.names.insert(format!("{prefix}{y}"), Some(now.clone()));
                    b += 1;
                }
                _ => unreachable!("entries merge"),
            }
        }
    }

    fn refusal(
        &mut self,
        totals: &mut HashMap<(String, String), u64>,
        owner: &str,
        why: &str,
        count: u64,
        add: bool,
    ) {
        if count == 0 {
            return;
        }
        let key = (owner.to_owned(), why.to_owned());
        let held = totals.get(&key).copied().unwrap_or(0);
        self.refusals.entry(key.clone()).or_insert(held);
        let now = if add {
            held + count
        } else {
            held.saturating_sub(count)
        };
        if now == 0 {
            totals.remove(&key);
        } else {
            totals.insert(key, now);
        }
    }

    fn part(
        &mut self,
        totals: &mut HashMap<(String, String), u64>,
        old: Option<&Part>,
        new: Option<&Part>,
    ) {
        self.entries(
            "",
            old.map_or(&[][..], |part| &part.entries),
            new.map_or(&[][..], |part| &part.entries),
        );
        for (owner, why, count) in old.into_iter().flat_map(|part| &part.refusals) {
            self.refusal(totals, owner, why, *count, false);
        }
        for (owner, why, count) in new.into_iter().flat_map(|part| &part.refusals) {
            self.refusal(totals, owner, why, *count, true);
        }
    }

    /// Refusals the hour holds that the hour before did not.
    pub(super) fn first_held<'a>(
        &'a self,
        totals: &'a HashMap<(String, String), u64>,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.refusals
            .iter()
            .filter(|(key, before)| **before == 0 && totals.contains_key(*key))
            .map(|((owner, why), _)| (owner.as_str(), why.as_str()))
    }

    /// Publish in whole-observation order, interning operands as it does.
    fn publish(
        self,
        state: &mut State,
        totals: &HashMap<(String, String), u64>,
        delta: &mut Delta,
    ) {
        enum Item {
            Name(String, Option<Value>),
            Refusal(String, String),
        }
        let mut ordered = BTreeMap::new();
        for (name, value) in self.names {
            ordered.insert(name.clone(), Item::Name(name, value));
        }
        // A whole observation sorts an owner's refusals between its ports or lines and its totals.
        for (owner, why) in self.refusals.into_keys() {
            ordered.insert(
                format!("{owner}|refusal|\u{0}{why}"),
                Item::Refusal(owner, why),
            );
        }
        for item in ordered.into_values() {
            match item {
                Item::Name(name, Some(Value::Plain(value))) => delta.set(name, value),
                Item::Name(name, Some(Value::Label(label))) => {
                    let id = state.intern_key(&label.key, || label.operand.clone());
                    delta.set(name, id as i128);
                }
                Item::Name(name, None) => delta.remove(name),
                Item::Refusal(owner, why) => {
                    let count = totals
                        .get(&(owner.clone(), why.clone()))
                        .copied()
                        .unwrap_or(0);
                    let id = state.intern(Operand::Refusal(why));
                    let name = format!("{owner}|refusal|{id}");
                    if count > 0 {
                        delta.set(name, i128::from(count));
                    } else {
                        delta.remove(name);
                    }
                }
            }
        }
    }
}

/// A repair port as its base projects it.
struct Port {
    owner: String,
    part: Part,
    /// The ships its queue holds, in order and with repeats.
    queued: Vec<u64>,
    /// Each distinct queued ship's prefix and handle.
    held: Vec<(String, u64)>,
}

#[derive(Default)]
struct Queued {
    entries: usize,
    ships: BTreeMap<u64, usize>,
}

#[derive(Default)]
struct Repairs {
    /// The raw references linked to each owner, ascending.
    raws: HashMap<String, Vec<u32>>,
    countries: HashMap<String, Entries>,
    factories: HashMap<String, Entries>,
    ports: HashMap<u32, Port>,
    /// Naval bases without an owner.
    unowned: usize,
    queued: HashMap<String, Queued>,
    totals: HashMap<String, Part>,
}

/// A refit link as the projection resolves it.
#[derive(Clone, PartialEq)]
struct Link {
    line: u64,
    /// Its ship's task force.
    task: Option<u64>,
    /// None for a stale link; then its owner, None where unresolved.
    owner: Option<Option<String>>,
}

#[derive(Default)]
struct Refits {
    links: HashMap<u64, Link>,
    by_task: HashMap<u64, HashSet<u64>>,
    by_line: HashMap<u64, HashSet<u64>>,
    /// Live links without an owner.
    unresolved: usize,
    /// Each owner's live lines and their ships.
    members: BTreeMap<(String, u64), BTreeSet<u64>>,
    /// The owners with ships on each line.
    line_owners: HashMap<u64, BTreeSet<String>>,
    /// Each live line's part and the ships it marks present.
    lines: HashMap<(String, u64), (Part, Vec<u64>)>,
    totals: HashMap<String, Part>,
}

/// One view's projection, kept so the next hour projects only what changed.
pub(crate) struct Engine {
    refits: bool,
    views: Vec<SharedView>,
    countries: Countries,
    owners: Vec<String>,
    /// Naming caches for the held naming tables.
    scratch: State,
    ships: HashMap<u64, Ship>,
    /// Each ship's holders: their prefixes, with their owners.
    holders: HashMap<u64, BTreeMap<String, String>>,
    /// The ships reading each task force handle.
    tasks: HashMap<u64, HashSet<u64>>,
    /// The ships reading each variant handle's definition.
    variants: HashMap<u64, HashSet<u64>>,
    /// Each owner's refusal totals by reason.
    totals: HashMap<(String, String), u64>,
    repairs: Repairs,
    refit: Refits,
}

/// Holders an hour removed and added: ship, prefix and owner.
#[derive(Default)]
struct Holding {
    removed: Vec<(u64, String, String)>,
    added: Vec<(u64, String, String)>,
}

impl Holding {
    fn diff(&mut self, owner_before: Option<&str>, before: &[(String, u64)], owner: Option<&str>, now: &[(String, u64)]) {
        let held: HashSet<&str> = now.iter().map(|(prefix, _)| prefix.as_str()).collect();
        let had: HashSet<&str> = before.iter().map(|(prefix, _)| prefix.as_str()).collect();
        if let Some(owner_before) = owner_before {
            for (prefix, ship) in before {
                if !held.contains(prefix.as_str()) {
                    self.removed.push((*ship, prefix.clone(), owner_before.to_owned()));
                }
            }
        }
        if let Some(owner) = owner {
            for (prefix, ship) in now {
                if !had.contains(prefix.as_str()) {
                    self.added.push((*ship, prefix.clone(), owner.to_owned()));
                }
            }
        }
    }
}

/// The keys of `T` whose rows `same` finds unequal; None when keys are out of order.
fn changed<T: TableSchema>(
    before: Option<&SharedView>,
    now: &SharedView,
    same: impl Fn(&T::Row, &T::Row) -> bool,
) -> Option<Vec<u128>> {
    if before.is_some_and(|before| SharedView::ptr_eq(before, now)) {
        return Some(Vec::new());
    }
    let new = view_rows::<T>(now)?;
    let old = match before {
        Some(before) => view_rows::<T>(before)?,
        None => &[],
    };
    let mut keys = Vec::new();
    changed_rows(old, new, same, |old, new| {
        keys.extend(old.or(new).map(RowCodec::key));
    })?;
    Some(keys)
}

/// Whether a naming table's rows read the same.
fn same_naming(table: TableId, before: &SharedView, now: &SharedView) -> bool {
    fn equal<T: TableSchema>(
        before: &SharedView,
        now: &SharedView,
        same: impl Fn(&T::Row, &T::Row) -> bool,
    ) -> bool {
        changed::<T>(Some(before), now, same).is_some_and(|keys| keys.is_empty())
    }
    if SharedView::ptr_eq(before, now) {
        return true;
    }
    match table {
        TableId::Countries => equal::<tables::Countries>(before, now, |a, b| {
            a.canonical_slot == b.canonical_slot && a.tag == b.tag
        }),
        TableId::CountryLinks => equal::<tables::CountryLinks>(before, now, PartialEq::eq),
        TableId::CountryAppearance => equal::<tables::CountryAppearance>(before, now, |a, b| {
            a.name_tag == b.name_tag
                && a.original_tag == b.original_tag
                && a.original_tag_reference == b.original_tag_reference
        }),
        TableId::ReservedCountryNaming => {
            equal::<tables::ReservedCountryNaming>(before, now, |a, b| {
                a.tag == b.tag
                    && a.original_tag == b.original_tag
                    && a.original_tag_reference == b.original_tag_reference
            })
        }
        TableId::LexicalTokens => equal::<tables::LexicalTokens>(before, now, PartialEq::eq),
        TableId::VariantDefinitions => {
            equal::<tables::VariantDefinitions>(before, now, PartialEq::eq)
        }
        TableId::Provinces => equal::<tables::Provinces>(before, now, |a, b| a.state == b.state),
        TableId::States => equal::<tables::States>(before, now, |a, b| {
            a.owner == b.owner && a.custom_name == b.custom_name && a.owner_name == b.owner_name
        }),
        TableId::ProvinceNames => equal::<tables::ProvinceNames>(before, now, PartialEq::eq),
        _ => false,
    }
}

/// The keys of a keyed management table whose read fields changed.
fn changed_keys(table: TableId, before: Option<&SharedView>, now: &SharedView) -> Option<Vec<u128>> {
    match table {
        TableId::NavalRepairCountries => {
            changed::<tables::NavalRepairCountries>(before, now, PartialEq::eq)
        }
        TableId::CountryFactories => changed::<tables::CountryFactories>(before, now, |a, b| {
            a.naval_total_raw == b.naval_total_raw
        }),
        TableId::NavalBases => {
            changed::<tables::NavalBases>(before, now, |a, b| a.country == b.country)
        }
        TableId::NavalRepairBases => changed::<tables::NavalRepairBases>(before, now, PartialEq::eq),
        TableId::NavalRepairQueue => changed::<tables::NavalRepairQueue>(before, now, PartialEq::eq),
        TableId::NavalRefitShips => changed::<tables::NavalRefitShips>(before, now, PartialEq::eq),
        TableId::NavalRefitProgress => {
            changed::<tables::NavalRefitProgress>(before, now, PartialEq::eq)
        }
        TableId::ProductionLines => changed::<tables::ProductionLines>(before, now, |a, b| {
            a.cost_raw == b.cost_raw && a.active_factories == b.active_factories && a.variant == b.variant
        }),
        TableId::Ships => changed::<tables::Ships>(before, now, |a, b| {
            a.task_force == b.task_force
                && a.strength == b.strength
                && a.class_id == b.class_id
                && a.name == b.name
                && a.naming == b.naming
        }),
        TableId::ShipModels => changed::<tables::ShipModels>(before, now, |a, b| {
            a.variant == b.variant
                && a.subunit_index == b.subunit_index
                && a.counter.as_ref().map(|c| c.maximum_strength)
                    == b.counter.as_ref().map(|c| c.maximum_strength)
        }),
        TableId::TaskForces => {
            changed::<tables::TaskForces>(before, now, |a, b| a.roster_owner == b.roster_owner)
        }
        TableId::TaskForceCounterInputs => {
            changed::<tables::TaskForceCounterInputs>(before, now, |a, b| a.name == b.name)
        }
        TableId::NavalRepairTaskForces => {
            changed::<tables::NavalRepairTaskForces>(before, now, PartialEq::eq)
        }
        TableId::NavalRepairChildren => {
            changed::<tables::NavalRepairChildren>(before, now, PartialEq::eq)
        }
        _ => None,
    }
}

impl Engine {
    /// A whole observation's projection of `frame`, kept for the next hour.
    pub(crate) fn build(frame: &DecodedFrame, refits: bool) -> Option<Box<Self>> {
        if !complete(frame, refits) {
            return None;
        }
        let countries = Countries::read(frame)?;
        let owners = owners(frame, &countries);
        // Changes are ordered by name, which an owner holding the separator breaks.
        if owners.iter().any(|owner| owner.contains('|')) {
            return None;
        }
        let mut repairs = Repairs::default();
        for link in frame.complete::<tables::CountryLinks>().ok()? {
            if let Some(owner) = countries.raw(link.id) {
                repairs.raws.entry(owner).or_default().push(link.id);
            }
        }
        let mut engine = Box::new(Self {
            refits,
            views: Vec::new(),
            countries,
            owners,
            scratch: State::default(),
            ships: HashMap::new(),
            holders: HashMap::new(),
            tasks: HashMap::new(),
            variants: HashMap::new(),
            totals: HashMap::new(),
            repairs,
            refit: Refits::default(),
        });
        engine.update(frame, true, &[])?;
        Some(engine)
    }

    /// The hour's changes, leaving the projection unusable where it returns None.
    pub(crate) fn changes(&mut self, frame: &DecodedFrame) -> Option<Changes> {
        if !complete(frame, self.refits) {
            return None;
        }
        for &table in NAMING {
            if table != TableId::VariantDefinitions
                && reads(table, self.refits)
                && !same_naming(table, &self.views[at(table)], &frame.tables[table.index()])
            {
                return None;
            }
        }
        // A definition names only the ships and lines reading it: those are projected again.
        let variants: Vec<u64> = changed::<tables::VariantDefinitions>(
            Some(&self.views[at(TableId::VariantDefinitions)]),
            &frame.tables[TableId::VariantDefinitions.index()],
            PartialEq::eq,
        )?
        .into_iter()
        .map(|key| key as u64)
        .collect();
        self.update(frame, false, &variants)
    }

    pub(crate) fn totals(&self) -> &HashMap<(String, String), u64> {
        &self.totals
    }

    pub(crate) fn publish(
        &self,
        changes: Changes,
        state: &mut State,
        delta: &mut Delta,
    ) {
        changes.publish(state, &self.totals, delta);
    }

    pub(crate) fn owners(&self) -> &[String] {
        &self.owners
    }

    fn update(&mut self, frame: &DecodedFrame, initial: bool, variants: &[u64]) -> Option<Changes> {
        let mut keys = vec![Vec::new(); TABLES.len()];
        for (index, &table) in TABLES.iter().enumerate() {
            if NAMING.contains(&table) || !reads(table, self.refits) {
                continue;
            }
            keys[index] = changed_keys(
                table,
                (!initial).then(|| &self.views[index]),
                &frame.tables[table.index()],
            )?;
        }
        if !variants.is_empty() {
            self.scratch.redesign(frame, variants);
        }
        let mut changes = Changes::default();
        let mut holding = Holding::default();
        let mut dirty = BTreeSet::new();
        if initial {
            dirty.extend(self.owners.iter().cloned());
        }
        if self.refits {
            self.refit_lines(frame, &keys, initial, variants, &mut changes, &mut holding, &mut dirty)?;
        } else {
            self.repair_ports(frame, &keys, initial, &mut changes, &mut holding, &mut dirty)?;
        }
        self.ship_changes(frame, &keys, variants, holding, &mut changes, &mut dirty);
        if self.refits {
            self.refit_totals(frame, &dirty, &mut changes)?;
        } else {
            self.repair_totals(frame, &dirty, &mut changes)?;
        }
        self.views = TABLES
            .iter()
            .map(|table| frame.tables[table.index()].clone())
            .collect();
        Some(changes)
    }

    fn repair_ports(
        &mut self,
        frame: &DecodedFrame,
        keys: &[Vec<u128>],
        initial: bool,
        changes: &mut Changes,
        holding: &mut Holding,
        dirty: &mut BTreeSet<String>,
    ) -> Option<()> {
        let Self {
            views,
            countries,
            scratch,
            totals,
            repairs,
            owners,
            ..
        } = self;
        for (table, held) in [
            (TableId::NavalRepairCountries, &mut repairs.countries),
            (TableId::CountryFactories, &mut repairs.factories),
        ] {
            let touched: BTreeSet<String> = keys[at(table)]
                .iter()
                .filter_map(|&key| countries.raw(key as u32))
                .collect();
            for owner in touched {
                let mut values = Values::default();
                for &raw in repairs.raws.get(&owner).into_iter().flatten() {
                    // The last of an owner's rows holds each of its names.
                    if table == TableId::NavalRepairCountries {
                        if let Some(row) = at_key(frame.complete::<tables::NavalRepairCountries>().ok()?, u128::from(raw)) {
                            values.clear();
                            naval_repairs::repair_country(&owner, row, &mut values);
                        }
                    } else if let Some(row) = at_key(frame.complete::<tables::CountryFactories>().ok()?, u128::from(raw)) {
                        values.clear();
                        naval_repairs::factories(&owner, row, &mut values);
                    }
                }
                let mut now: Entries = values
                    .iter()
                    .map(|(name, value)| (name.to_owned(), Value::Plain(value)))
                    .collect();
                now.sort_by(|a, b| a.0.cmp(&b.0));
                let old = held.remove(&owner).unwrap_or_default();
                changes.entries("", &old, &now);
                if !now.is_empty() {
                    held.insert(owner, now);
                }
            }
        }
        let bases = frame.complete::<tables::NavalBases>().ok()?;
        let repair_bases = frame.complete::<tables::NavalRepairBases>().ok()?;
        let queues = frame.complete::<tables::NavalRepairQueue>().ok()?;
        let complete_before = repairs.unowned == 0;
        let before = if initial {
            &[][..]
        } else {
            view_rows::<tables::NavalBases>(&views[at(TableId::NavalBases)])?
        };
        let mut provinces = BTreeSet::new();
        for &key in &keys[at(TableId::NavalBases)] {
            let unowned = |rows: &[NavalBaseRow]| {
                at_key(rows, key).is_some_and(|base| countries.raw(base.country).is_none())
            };
            repairs.unowned -= usize::from(unowned(before));
            repairs.unowned += usize::from(unowned(bases));
            provinces.insert(key as u32);
        }
        provinces.extend(keys[at(TableId::NavalRepairBases)].iter().map(|&key| key as u32));
        provinces.extend(keys[at(TableId::NavalRepairQueue)].iter().map(|&key| (key >> 32) as u32));
        for province in provinces {
            let now = at_key(bases, u128::from(province)).and_then(|base| {
                scratch.begin_part();
                let mut values = Values::default();
                let mut held = Vec::new();
                let (owner, queued) = naval_repairs::port(
                    scratch,
                    frame,
                    countries,
                    base,
                    Some(repair_bases),
                    Some(queues),
                    &mut values,
                    &mut |_: &mut State, _: &mut Values, _: &str, prefix: &str, ship: u64| {
                        held.push((prefix.to_owned(), ship))
                    },
                )?;
                Some(Port {
                    owner,
                    part: scratch.part(&values),
                    queued,
                    held,
                })
            });
            let old = repairs.ports.remove(&province);
            changes.part(
                totals,
                old.as_ref().map(|port| &port.part),
                now.as_ref().map(|port| &port.part),
            );
            holding.diff(
                old.as_ref().map(|port| port.owner.as_str()),
                old.as_ref().map_or(&[][..], |port| &port.held),
                now.as_ref().map(|port| port.owner.as_str()),
                now.as_ref().map_or(&[][..], |port| &port.held),
            );
            for (port, add) in [(&old, false), (&now, true)] {
                let Some(port) = port else { continue };
                let queued = repairs.queued.entry(port.owner.clone()).or_default();
                for &ship in &port.queued {
                    if add {
                        queued.entries += 1;
                        *queued.ships.entry(ship).or_default() += 1;
                    } else {
                        queued.entries -= 1;
                        let count = queued.ships.get_mut(&ship).expect("queued ship");
                        *count -= 1;
                        if *count == 0 {
                            queued.ships.remove(&ship);
                        }
                    }
                }
                if queued.entries == 0 {
                    repairs.queued.remove(&port.owner);
                }
                dirty.insert(port.owner.clone());
            }
            if let Some(now) = now {
                repairs.ports.insert(province, now);
            }
        }
        if complete_before != (repairs.unowned == 0) {
            dirty.extend(owners.iter().cloned());
        }
        Some(())
    }

    fn repair_totals(
        &mut self,
        frame: &DecodedFrame,
        dirty: &BTreeSet<String>,
        changes: &mut Changes,
    ) -> Option<()> {
        let ships = frame.complete::<tables::Ships>().ok()?;
        let Self {
            scratch,
            totals,
            repairs,
            owners,
            ..
        } = self;
        let complete = repairs.unowned == 0;
        for owner in dirty {
            if owners.binary_search(owner).is_err() {
                continue;
            }
            scratch.begin_part();
            let mut values = Values::default();
            let queued = repairs.queued.get(owner);
            naval_repairs::queue_totals(
                scratch,
                &mut values,
                owner,
                complete,
                queued.map(|queued| {
                    (
                        queued.entries,
                        queued.ships.len(),
                        Box::new(queued.ships.keys().copied())
                            as Box<dyn Iterator<Item = u64> + '_>,
                    )
                }),
                Some(ships),
            );
            let now = scratch.part(&values);
            let old = repairs.totals.insert(owner.clone(), now);
            changes.part(totals, old.as_ref(), repairs.totals.get(owner));
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn refit_lines(
        &mut self,
        frame: &DecodedFrame,
        keys: &[Vec<u128>],
        initial: bool,
        variants: &[u64],
        changes: &mut Changes,
        holding: &mut Holding,
        dirty: &mut BTreeSet<String>,
    ) -> Option<()> {
        let progress = frame.complete::<tables::NavalRefitProgress>().ok()?;
        let production = frame.complete::<tables::ProductionLines>().ok()?;
        let ships = frame.complete::<tables::Ships>().ok()?;
        let tasks = frame.complete::<tables::TaskForces>().ok()?;
        let links = frame.complete::<tables::NavalRefitShips>().ok()?;
        let Self {
            countries,
            scratch,
            totals,
            refit,
            owners,
            ..
        } = self;
        let mut touched = BTreeSet::new();
        if initial {
            touched.extend(links.iter().map(|link| link.ship));
        }
        touched.extend(keys[at(TableId::NavalRefitShips)].iter().map(|&key| key as u64));
        touched.extend(keys[at(TableId::Ships)].iter().map(|&key| key as u64));
        for &key in &keys[at(TableId::TaskForces)] {
            touched.extend(refit.by_task.get(&(key as u64)).into_iter().flatten());
        }
        let mut lines = BTreeSet::new();
        // A line names its production target's design.
        if !variants.is_empty() {
            for (owner, line) in refit.lines.keys() {
                if at_key(production, u128::from(*line))
                    .is_some_and(|row| variants.binary_search(&row.variant).is_ok())
                {
                    lines.insert((owner.clone(), *line));
                }
            }
        }
        for &key in keys[at(TableId::NavalRefitProgress)]
            .iter()
            .chain(&keys[at(TableId::ProductionLines)])
        {
            let line = key as u64;
            touched.extend(refit.by_line.get(&line).into_iter().flatten());
            for owner in refit.line_owners.get(&line).into_iter().flatten() {
                lines.insert((owner.clone(), line));
            }
        }
        let unresolved_before = refit.unresolved;
        for ship in touched {
            let now = at_key(links, u128::from(ship))
                .filter(|link| link.line != 0)
                .map(|link| {
                    let typed = dashboard_forces::products::naval_management::projection::refit(
                        link,
                        Some(progress),
                        Some(production),
                    );
                    Link {
                        line: link.line,
                        task: find(Some(ships), link.ship, |row| row.id).map(|row| row.task_force),
                        owner: (typed.live != Some(false)).then(|| {
                            naval_refits::link_owner(link, Some(ships), Some(tasks), countries)
                        }),
                    }
                });
            let old = refit.links.remove(&ship);
            if old == now {
                if let Some(old) = old {
                    refit.links.insert(ship, old);
                }
                continue;
            }
            for (link, add) in [(&old, false), (&now, true)] {
                let Some(link) = link else { continue };
                let by_line = refit.by_line.entry(link.line).or_default();
                if add {
                    by_line.insert(ship);
                } else {
                    by_line.remove(&ship);
                    if by_line.is_empty() {
                        refit.by_line.remove(&link.line);
                    }
                }
                if let Some(task) = link.task {
                    let by_task = refit.by_task.entry(task).or_default();
                    if add {
                        by_task.insert(ship);
                    } else {
                        by_task.remove(&ship);
                        if by_task.is_empty() {
                            refit.by_task.remove(&task);
                        }
                    }
                }
                match &link.owner {
                    Some(None) if add => refit.unresolved += 1,
                    Some(None) => refit.unresolved -= 1,
                    Some(Some(owner)) => {
                        let key = (owner.clone(), link.line);
                        let members = refit.members.entry(key.clone()).or_default();
                        if add {
                            members.insert(ship);
                            refit
                                .line_owners
                                .entry(link.line)
                                .or_default()
                                .insert(owner.clone());
                        } else {
                            members.remove(&ship);
                            if members.is_empty() {
                                refit.members.remove(&key);
                                let line_owners =
                                    refit.line_owners.get_mut(&link.line).expect("line owners");
                                line_owners.remove(owner);
                                if line_owners.is_empty() {
                                    refit.line_owners.remove(&link.line);
                                }
                            }
                        }
                        lines.insert(key);
                    }
                    None => {}
                }
            }
            if let Some(now) = now {
                refit.links.insert(ship, now);
            }
        }
        for (owner, line) in lines {
            let key = (owner, line);
            let members = refit.members.get(&key);
            let now = members.map(|members| {
                scratch.begin_part();
                let mut values = Values::default();
                let prefix = format!("{}|{line}", key.0);
                let amount = at_key(progress, u128::from(line));
                if let Some(amount) = amount {
                    naval_refits::line_progress(&prefix, amount, &mut values);
                    for ship in members {
                        values.push(format!("{prefix}|ship|{ship}|present"), 1);
                    }
                }
                naval_refits::line_metadata(
                    scratch,
                    frame,
                    &mut values,
                    &key.0,
                    &prefix,
                    at_key(production, u128::from(line)),
                    amount,
                );
                scratch.part(&values)
            });
            let old = refit.lines.remove(&key);
            changes.part(totals, old.as_ref().map(|(part, _)| part), now.as_ref());
            let prefix = |ship: u64| format!("{}|{line}|ship|{ship}", key.0);
            let before: Vec<(String, u64)> = old
                .iter()
                .flat_map(|(_, ships)| ships)
                .map(|&ship| (prefix(ship), ship))
                .collect();
            let ships: Vec<u64> = members.into_iter().flatten().copied().collect();
            let held: Vec<(String, u64)> = ships.iter().map(|&ship| (prefix(ship), ship)).collect();
            holding.diff(Some(&key.0), &before, Some(&key.0), &held);
            dirty.insert(key.0.clone());
            if let Some(now) = now {
                refit.lines.insert(key, (now, ships));
            }
        }
        if (unresolved_before == 0) != (refit.unresolved == 0) {
            dirty.extend(owners.iter().cloned());
        }
        Some(())
    }

    fn refit_totals(
        &mut self,
        frame: &DecodedFrame,
        dirty: &BTreeSet<String>,
        changes: &mut Changes,
    ) -> Option<()> {
        let progress = frame.complete::<tables::NavalRefitProgress>().ok()?;
        let production = frame.complete::<tables::ProductionLines>().ok()?;
        let Self {
            scratch,
            totals,
            refit,
            owners,
            ..
        } = self;
        let complete = refit.unresolved == 0;
        for owner in dirty {
            if owners.binary_search(owner).is_err() {
                continue;
            }
            let mut total = None::<naval_refits::RefitTotal>;
            for ((_, line), ships) in refit
                .members
                .range((owner.clone(), 0)..=(owner.clone(), u64::MAX))
            {
                total.get_or_insert_with(Default::default).add(
                    ships.len(),
                    *line,
                    Some(progress),
                    Some(production),
                );
            }
            scratch.begin_part();
            let mut values = Values::default();
            naval_refits::refit_totals(scratch, &mut values, owner, complete, total.as_ref());
            let now = scratch.part(&values);
            let old = refit.totals.insert(owner.clone(), now);
            changes.part(totals, old.as_ref(), refit.totals.get(owner));
        }
        Some(())
    }

    /// Project changed ships again and change the holders of changed ships and holdings.
    fn ship_changes(
        &mut self,
        frame: &DecodedFrame,
        keys: &[Vec<u128>],
        variants: &[u64],
        holding: Holding,
        changes: &mut Changes,
        dirty: &mut BTreeSet<String>,
    ) {
        let Self {
            scratch,
            ships,
            holders,
            tasks,
            variants: readers,
            totals,
            views,
            ..
        } = self;
        let mut touched = BTreeSet::new();
        for table in [TableId::Ships, TableId::ShipModels] {
            touched.extend(
                keys[at(table)]
                    .iter()
                    .map(|&key| key as u64)
                    .filter(|ship| ships.contains_key(ship)),
            );
        }
        for (table, shift) in [
            (TableId::TaskForces, 0),
            (TableId::TaskForceCounterInputs, 0),
            (TableId::NavalRepairTaskForces, 0),
            (TableId::NavalRepairChildren, 32),
        ] {
            for &key in &keys[at(table)] {
                touched.extend(tasks.get(&((key >> shift) as u64)).into_iter().flatten());
            }
        }
        for variant in variants {
            touched.extend(readers.get(variant).into_iter().flatten());
        }
        touched.extend(
            holding
                .added
                .iter()
                .map(|(ship, _, _)| *ship)
                .filter(|ship| !ships.contains_key(ship)),
        );
        let mut replaced = HashMap::new();
        for ship in touched {
            let now = scratch.ship_item(frame, ship);
            for &task in &now.tasks {
                tasks.entry(task).or_default().insert(ship);
            }
            for &variant in &now.variants {
                readers.entry(variant).or_default().insert(ship);
            }
            let old = ships.insert(ship, now);
            if let Some(old) = &old {
                let now = &ships[&ship];
                unindex(tasks, ship, &old.tasks, &now.tasks);
                unindex(readers, ship, &old.variants, &now.variants);
            }
            replaced.insert(ship, old);
        }
        let removed: HashSet<(u64, &str)> = holding
            .removed
            .iter()
            .map(|(ship, prefix, _)| (*ship, prefix.as_str()))
            .collect();
        for (&ship, old) in &replaced {
            let Some(old) = old else { continue };
            let now = &ships[&ship];
            for (prefix, owner) in holders.get(&ship).into_iter().flatten() {
                if removed.contains(&(ship, prefix.as_str())) {
                    continue;
                }
                changes.entries(prefix, &old.entries, &now.entries);
                for (why, count) in &old.refusals {
                    changes.refusal(totals, owner, why, *count, false);
                }
                for (why, count) in &now.refusals {
                    changes.refusal(totals, owner, why, *count, true);
                }
            }
        }
        for (ship, prefix, owner) in &holding.removed {
            let old = match replaced.get(ship) {
                Some(old) => old.as_ref(),
                None => ships.get(ship),
            };
            if let Some(old) = old {
                changes.entries(prefix, &old.entries, &[]);
                for (why, count) in &old.refusals {
                    changes.refusal(totals, owner, why, *count, false);
                }
            }
            if let Some(held) = holders.get_mut(ship) {
                held.remove(prefix);
                if held.is_empty() {
                    holders.remove(ship);
                }
            }
        }
        for (ship, prefix, owner) in &holding.added {
            let now = &ships[ship];
            changes.entries(prefix, &[], &now.entries);
            for (why, count) in &now.refusals {
                changes.refusal(totals, owner, why, *count, true);
            }
            holders
                .entry(*ship)
                .or_default()
                .insert(prefix.clone(), owner.clone());
        }
        // A ship no port or line holds is no longer projected.
        let unheld: Vec<u64> = replaced
            .keys()
            .chain(holding.removed.iter().map(|(ship, _, _)| ship))
            .copied()
            .filter(|ship| !holders.contains_key(ship))
            .collect();
        for ship in unheld {
            if let Some(old) = ships.remove(&ship) {
                unindex(tasks, ship, &old.tasks, &[]);
                unindex(readers, ship, &old.variants, &[]);
            }
        }
        // Owners whose queued ships appeared or went count them again.
        let before = views
            .get(at(TableId::Ships))
            .and_then(view_rows::<tables::Ships>);
        let now = frame.complete::<tables::Ships>().ok();
        for &key in &keys[at(TableId::Ships)] {
            let exists = |rows: Option<&[ShipRow]>| rows.and_then(|rows| at_key(rows, key)).is_some();
            if exists(before) != exists(now) {
                for owner in holders.get(&(key as u64)).into_iter().flat_map(BTreeMap::values) {
                    dirty.insert(owner.clone());
                }
            }
        }
    }

    pub(crate) fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(size_of::<Self>());
        bytes.add(self.views.capacity() * size_of::<SharedView>());
        for view in &self.views {
            bytes.view(view);
        }
        self.countries.held(bytes);
        self.scratch.held(bytes);
        bytes.heap(&self.owners);
        bytes.table::<u64, Ship>(self.ships.capacity());
        for ship in self.ships.values() {
            entries_held(&ship.entries, bytes);
            bytes.add(ship.refusals.capacity() * size_of::<(String, u64)>());
            for (why, _) in &ship.refusals {
                bytes.heap(why);
            }
            bytes.heap(&ship.tasks).heap(&ship.variants);
        }
        bytes.table::<u64, BTreeMap<String, String>>(self.holders.capacity());
        for held in self.holders.values() {
            bytes.heap(held);
        }
        for index in [&self.tasks, &self.variants] {
            bytes.table::<u64, HashSet<u64>>(index.capacity());
            for ships in index.values() {
                bytes.table::<u64, ()>(ships.capacity());
            }
        }
        bytes.table::<(String, String), u64>(self.totals.capacity());
        for (owner, why) in self.totals.keys() {
            bytes.heap(owner).heap(why);
        }
        let repairs = &self.repairs;
        bytes.table::<String, Vec<u32>>(repairs.raws.capacity());
        for (owner, raws) in &repairs.raws {
            bytes.heap(owner).heap(raws);
        }
        for held in [&repairs.countries, &repairs.factories] {
            bytes.table::<String, Entries>(held.capacity());
            for (owner, entries) in held {
                bytes.heap(owner);
                entries_held(entries, bytes);
            }
        }
        bytes.table::<u32, Port>(repairs.ports.capacity());
        for port in repairs.ports.values() {
            bytes.heap(&port.owner).heap(&port.queued);
            part_held(&port.part, bytes);
            bytes.add(port.held.capacity() * size_of::<(String, u64)>());
            for (prefix, _) in &port.held {
                bytes.heap(prefix);
            }
        }
        bytes.table::<String, Queued>(repairs.queued.capacity());
        for (owner, queued) in &repairs.queued {
            bytes.heap(owner).heap(&queued.ships);
        }
        let refit = &self.refit;
        bytes.table::<u64, Link>(refit.links.capacity());
        for link in refit.links.values() {
            if let Some(Some(owner)) = &link.owner {
                bytes.heap(owner);
            }
        }
        for index in [&refit.by_task, &refit.by_line] {
            bytes.table::<u64, HashSet<u64>>(index.capacity());
            for ships in index.values() {
                bytes.table::<u64, ()>(ships.capacity());
            }
        }
        bytes.tree::<(String, u64), BTreeSet<u64>>(refit.members.len());
        for ((owner, _), ships) in &refit.members {
            bytes.heap(owner).tree::<u64, ()>(ships.len());
        }
        bytes.table::<u64, BTreeSet<String>>(refit.line_owners.capacity());
        for owners in refit.line_owners.values() {
            bytes.heap(owners);
        }
        bytes.table::<(String, u64), (Part, Vec<u64>)>(refit.lines.capacity());
        for ((owner, _), (part, ships)) in &refit.lines {
            bytes.heap(owner).heap(ships);
            part_held(part, bytes);
        }

        for totals in [&repairs.totals, &refit.totals] {
            bytes.table::<String, Part>(totals.capacity());
            for (owner, part) in totals {
                bytes.heap(owner);
                part_held(part, bytes);
            }
        }
    }
}


fn unindex(tasks: &mut HashMap<u64, HashSet<u64>>, ship: u64, old: &[u64], now: &[u64]) {
    for task in old {
        if now.contains(task) {
            continue;
        }
        if let Some(ships) = tasks.get_mut(task) {
            ships.remove(&ship);
            if ships.is_empty() {
                tasks.remove(task);
            }
        }
    }
}

fn entries_held(entries: &Entries, bytes: &mut HeldBytes) {
    bytes.add(entries.capacity() * size_of::<(String, Value)>());
    for (name, value) in entries {
        bytes.heap(name);
        if let Value::Label(label) = value
            && bytes.once(Arc::as_ptr(label))
        {
            bytes.add(size_of::<Label>()).heap(&label.key);
            label.operand.held(bytes);
        }
    }
}

fn part_held(part: &Part, bytes: &mut HeldBytes) {
    entries_held(&part.entries, bytes);
    bytes.add(part.refusals.capacity() * size_of::<(String, String, u64)>());
    for (owner, why, _) in &part.refusals {
        bytes.heap(owner).heap(why);
    }
}
