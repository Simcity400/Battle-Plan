//! The viewing country's relations, from the mirrors: each country's type-icon relation to the
//! viewer (`type_tint::Index`: directed wars, faction and subject families, the viewer's volunteer
//! hosts) and who is at war with whom. Nothing to draw or upload; the counters, the unit models and
//! the naval effects read it through `Inputs::after("relations")` ([`Relations::index`],
//! [`Facts::at_war`]) and take the countries it changed from `changed()`: kind `"country"`,
//! raw references.
//!
//! The viewer's slot is the vision layer's. Country slot zero's relation to the viewer is read from
//! the viewer's recorded `CountryCounterInputs` packets, as today; a viewer at slot zero reads
//! packets this layer does not decode and is refused by name.

use super::{Budget, Carried, Changed, Gpu, Inputs, Layer, List, Num, Upstream, View, vision};
use crate::playhead_host::mirror::{Cell, Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::{Recorded, ReverseZeroVisibilityInputs, TableId, ZeroSubjectInputs};
use viewer_foundation::map::{
    directed_wars::ReceiverWars,
    visibility::{Families, forward_zero_shared_of},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};
use viewer_counters::map::counters::type_tint::{Index, Receiver, RelationFacts, volunteer_hosts};

const SLOT_ZERO: &str = "a viewer at country slot zero reads recorded packets the relations layer does not decode";

#[derive(Clone, Copy)]
struct Columns {
    link_slot: Num<u32>,
    country_slot: Num<u32>,
    faction: Num<u64>,
    sent_division: Num<u64>,
    logical_owner: Num<u32>,
    reserved_faction: Num<u64>,
    zero_war: List,
    zero_subject: List,
    forward_visibility: List,
}

/// The tables whose change makes the index again.
const READ: [TableId; 9] = [
    TableId::CountryLinks,
    TableId::Countries,
    TableId::WarRelations,
    TableId::FactionMembers,
    TableId::Subjects,
    TableId::SentVolunteers,
    TableId::Divisions,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
];

fn complete(mirrors: &Mirrors, table: TableId) -> Result<&Mirror, String> {
    let mirror = mirrors.get(table).ok_or_else(|| format!("{table:?} is not read"))?;
    match mirror.not_recorded() {
        None => Ok(mirror),
        Some(why) => Err(why),
    }
}

/// The parts of [`Facts`], each read again from the mirrors only when its tables change.
const LINKS: u8 = 1;
const COUNTRIES: u8 = 2;
const WARS: u8 = 4;
const FAMILIES: u8 = 8;
const ZERO: u8 = 16;
const HOSTS: u8 = 32;
const EVERY_PART: u8 = 63;

/// One hour's relations as values, each part shared with the facts before it until its tables
/// change.
pub struct Facts {
    links: Arc<BTreeMap<u32, u32>>,
    /// Canonical slot to the raw id of its Countries row, and whether another row names it too.
    countries: Arc<Result<HashMap<u32, (u32, bool)>, String>>,
    /// Each war's two raw countries, or why WarRelations cannot say.
    wars: Arc<Result<Vec<(u32, u32)>, String>>,
    families: Arc<Result<Families, String>>,
    /// Whether the viewer (its slot) shares counters with country slot zero
    /// (`visibility::forward_zero_shared`).
    zero: (u32, Result<bool, String>),
}

/// One receiver country (`directed_wars::Owner`'s answers from the mirrors).
pub struct Country<'a> {
    facts: &'a Facts,
    actual: i32,
    slot: u32,
    wars: std::cell::OnceCell<Result<ReceiverWars, String>>,
}

impl Facts {
    /// Whether `receiver` (a raw reference) is at war with `target` (`directed_wars::Owner::at_war`).
    pub fn at_war(&self, receiver: u32, target: u32) -> Result<bool, WarUnknown> {
        self.receiver(receiver).map_err(WarUnknown::Receiver)?.at_war(target).map_err(WarUnknown::Operands)
    }

    fn canonical(&self, raw: u32) -> Result<u32, String> {
        if raw == 0 {
            return Ok(0);
        }
        self.links.get(&raw).copied().ok_or_else(|| "directed-war raw country link missing".to_owned())
    }
}

impl Country<'_> {
    fn pointer_active_slot(&self, target: u32) -> Result<bool, String> {
        if self.slot == 0 {
            return Err(SLOT_ZERO.into());
        }
        let wars = self.wars.get_or_init(|| {
            let wars = Result::as_ref(&*self.facts.wars).map_err(Clone::clone)?;
            let receiver = self.facts.links.iter().filter(|&(_, &slot)| slot == self.slot).map(|(&raw, _)| raw);
            ReceiverWars::from_pairs(wars.iter().copied(), receiver, |raw| self.facts.links.get(&raw).copied(), self.slot)
        });
        Ok(wars.as_ref().map_err(Clone::clone)?.pointer_active(target))
    }
}

impl Receiver for Country<'_> {
    fn same_country(&self, raw: u32) -> Result<bool, String> {
        if raw as i32 == self.actual {
            return Ok(true);
        }
        Ok(raw != 0 && self.actual != 0 && self.facts.canonical(raw)? == self.slot)
    }

    fn at_war(&self, raw: u32) -> Result<bool, String> {
        if self.same_country(raw)? {
            return Ok(false);
        }
        if self.actual <= 0 {
            return Ok(false);
        }
        self.pointer_active_slot(self.facts.canonical(raw)?)
    }
}

impl RelationFacts for Facts {
    type Receiver<'r>
        = Country<'r>
    where
        Self: 'r;

    fn receiver(&self, raw: u32) -> Result<Country<'_>, String> {
        let slot = self.canonical(raw)?;
        let actual = if slot == 0 {
            0
        } else {
            match Result::as_ref(&*self.countries).map_err(Clone::clone)?.get(&slot) {
                Some(&(id, false)) => id as i32,
                None => return Err("directed-war receiver Countries row missing".into()),
                Some(_) => return Err("directed-war receiver Countries row duplicated".into()),
            }
        };
        Ok(Country { facts: self, actual, slot, wars: std::cell::OnceCell::new() })
    }

    fn counter_shared(&self, viewer: &Country<'_>, raw: u32) -> Result<bool, String> {
        if viewer.same_country(raw)? {
            return Ok(true);
        }
        let target = if raw == 0 { 0 } else { self.links.get(&raw).copied().ok_or("visibility country link")? };
        if viewer.pointer_active_slot(target)? {
            return Ok(false);
        }
        if viewer.slot == 0 {
            return Err(SLOT_ZERO.into());
        }
        if target == 0 {
            let (slot, shared) = &self.zero;
            if *slot != viewer.slot {
                return Err("country slot zero's relation is read for the viewing country only".into());
            }
            return shared.clone();
        }
        Ok(Result::as_ref(&*self.families).map_err(Clone::clone)?.counter_shared(viewer.slot, target))
    }
}

/// Why [`Relations::at_war`] cannot answer: the receiver is not known, or its war operands are not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WarUnknown {
    Receiver(String),
    Operands(String),
}

/// The relations layer's index and what keeps it.
pub struct Relations {
    columns: Option<Columns>,
    index: Arc<Index>,
    /// Raw references (kind `"country"`) whose relation changed since the host last cleared it.
    changed: Changed,
    /// The index is to be made again at the next settle: the parts of its facts to read again,
    /// and the countries to decide again (`all`, or those `raws` names).
    dirty: bool,
    parts: u8,
    all: bool,
    raws: BTreeSet<u32>,
    /// The viewer's volunteer hosts as last read.
    hosts: Result<Vec<(u32, u32)>, String>,
    ours: Option<u32>,
    /// The divisions in the viewer's sent-volunteer list: their logical owners are hosts.
    sent: HashSet<u64>,
    statuses: HashMap<usize, Status>,
    /// Index recomputes since the layer was made.
    recomputes: u64,
    /// The values the index was last made from, which `at_war` asks.
    facts: Option<Arc<Facts>>,
    /// The viewer when no vision layer is read: the tests' world.
    #[cfg(test)]
    viewer: Option<u32>,
}

impl Default for Relations {
    fn default() -> Self {
        Self::new()
    }
}

impl Relations {
    pub fn new() -> Self {
        Self {
            columns: None,
            index: Arc::new(Index::unavailable_for("not composed yet".into())),
            changed: Changed::default(),
            dirty: true,
            parts: EVERY_PART,
            all: true,
            raws: BTreeSet::new(),
            hosts: Ok(Vec::new()),
            ours: None,
            sent: HashSet::new(),
            statuses: HashMap::new(),
            recomputes: 0,
            facts: None,
            #[cfg(test)]
            viewer: None,
        }
    }

    /// A layer of the same inputs, made from scratch.
    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new() }
    }

    /// Each country's type-icon relation to the viewing country.
    pub fn index(&self) -> &Arc<Index> {
        &self.index
    }

    /// Who is at war with whom ([`Facts::at_war`]), replaced whenever the index is made again;
    /// none until it is composed.
    pub fn wars(&self) -> Option<&Arc<Facts>> {
        self.facts.as_ref()
    }

    /// The viewer's slot when no vision layer is read: the tests' world.
    #[cfg(test)]
    pub fn viewed_as(mut self, ours: u32) -> Self {
        self.viewer = Some(ours);
        self
    }

    /// Recomputes since the layer was made.
    pub fn recomputes(&self) -> u64 {
        self.recomputes
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the relations layer is declared before it composes")
    }

    /// Everything to be read and decided again.
    fn everything(&mut self) {
        self.dirty = true;
        self.parts = EVERY_PART;
        self.all = true;
    }

    /// `part` of the facts to be read again, and the countries `raws` names decided again.
    fn mark(&mut self, part: u8, raws: impl IntoIterator<Item = u32>) {
        self.dirty = true;
        self.parts |= part;
        self.raws.extend(raws);
    }

    /// The raw references of canonical slot `slot` (country zero for zero), from the last facts.
    fn raws_of(&self, slot: u32) -> Vec<u32> {
        let mut raws: Vec<u32> = self.facts.iter().flat_map(|facts| facts.links.iter().filter(move |&(_, &held)| held == slot).map(|(&raw, _)| raw)).collect();
        if slot == 0 {
            raws.push(0);
        }
        raws
    }

    /// A relation between raw `a` and raw `b` changed (a war or a subject): only the viewer's
    /// relation to the other side can change, unless volunteer hosts read every country's wars.
    fn pair(&mut self, part: u8, a: u32, b: u32) {
        let slots = self.facts.as_ref().zip(self.ours).map(|(facts, ours)| {
            let slot = |raw: u32| if raw == 0 { Some(0) } else { facts.links.get(&raw).copied() };
            (slot(a), slot(b), ours)
        });
        let Some((Some(a), Some(b), ours)) = slots else { return self.everything() };
        if part == WARS && !matches!(&self.hosts, Ok(hosts) if hosts.is_empty()) {
            self.all = true;
        }
        let others: Vec<u32> = [(a, b), (b, a)].into_iter().filter(|&(side, _)| side == ours).flat_map(|(_, other)| self.raws_of(other)).collect();
        self.mark(part, others);
    }

    /// The index made again from the mirrors: the parts of its facts that changed read again,
    /// the countries a change names decided again; the countries it changes listed.
    fn recompute(&mut self, mirrors: &Mirrors, ours: u32) {
        self.recomputes += 1;
        let columns = self.columns();
        let old = self.facts.take();
        let (parts, all) = match &old {
            Some(_) if self.parts & LINKS == 0 => (self.parts, self.all),
            _ => (EVERY_PART, true),
        };
        let raws = std::mem::take(&mut self.raws);
        (self.parts, self.all) = (0, false);
        let links = match (&old, parts & LINKS) {
            (Some(old), 0) => Arc::clone(&old.links),
            _ => match complete(mirrors, TableId::CountryLinks) {
                Err(why) => {
                    let index = Index::unavailable_for(why);
                    self.publish(index, None);
                    return;
                }
                Ok(links) => Arc::new(links.rows().filter_map(|slot| Some((links.key(slot)? as u32, links.number(columns.link_slot, slot).value()?))).collect()),
            },
        };
        let part = |mask: u8| old.as_ref().filter(|_| parts & mask == 0);
        let countries = part(COUNTRIES).map_or_else(
            || {
                Arc::new(complete(mirrors, TableId::Countries).map(|countries| {
                    let mut by_slot: HashMap<u32, (u32, bool)> = HashMap::with_capacity(countries.rows().size_hint().0);
                    for slot in countries.rows() {
                        if let (Some(id), Some(canonical)) = (countries.key(slot), countries.number(columns.country_slot, slot).value()) {
                            by_slot.entry(canonical).and_modify(|(_, twice)| *twice = true).or_insert((id as u32, false));
                        }
                    }
                    by_slot
                }))
            },
            |old| Arc::clone(&old.countries),
        );
        let wars = part(WARS).map_or_else(
            || Arc::new(complete(mirrors, TableId::WarRelations).map(|wars| wars.rows().filter_map(|slot| wars.key(slot)).map(|key| ((key >> 32) as u32, key as u32)).collect())),
            |old| Arc::clone(&old.wars),
        );
        let families = part(FAMILIES).map_or_else(|| Arc::new(families(mirrors, &links, columns)), |old| Arc::clone(&old.families));
        let zero = match part(ZERO) {
            Some(old) if old.zero.0 == ours => old.zero.clone(),
            _ => (ours, zero_shared(mirrors, &links, ours, columns)),
        };
        if old.is_none() || parts & HOSTS != 0 {
            self.hosts = self.read_hosts(mirrors, &links, ours, columns);
        }
        let facts = Facts { links, countries, wars, families, zero };
        let hosts = self.hosts.as_deref().map_err(Clone::clone);
        if !all && raws.is_empty() && self.index.len() > 0 && hosts.as_ref().err() == self.index.volunteer_problem() {
            self.facts = Some(Arc::new(facts));
            return;
        }
        let index = if all || self.index.len() == 0 { Index::compute(&facts, &facts.links, ours, hosts) } else { self.index.remake(&facts, &facts.links, ours, hosts, &raws) };
        self.publish(index, Some(Arc::new(facts)));
    }

    /// `index` and the `facts` it was made from become the layer's; the countries whose relation
    /// changed are listed.
    fn publish(&mut self, index: Index, facts: Option<Arc<Facts>>) {
        for raw in index.changed_from(&self.index) {
            self.changed.mark("country", u64::from(raw));
        }
        self.index = Arc::new(index);
        self.facts = facts;
    }

    /// The viewer's volunteer hosts, and the divisions its list names.
    fn read_hosts(&mut self, mirrors: &Mirrors, links: &BTreeMap<u32, u32>, ours: u32, columns: Columns) -> Result<Vec<(u32, u32)>, String> {
        self.sent.clear();
        let sent = complete(mirrors, TableId::SentVolunteers).map_err(|why| format!("SentVolunteers: {why}"))?;
        let divisions = complete(mirrors, TableId::Divisions).map_err(|why| format!("Divisions: {why}"))?;
        let mut lists: BTreeMap<u32, Vec<(u32, u64)>> = BTreeMap::new();
        for slot in sent.rows() {
            let Some(key) = sent.key(slot) else { continue };
            let raw = (key >> 32) as u32;
            if links.get(&raw) == Some(&ours) {
                lists.entry(raw).or_default().push((key as u32, sent.number(columns.sent_division, slot).value().unwrap_or(0)));
            }
        }
        lists.values_mut().for_each(|list| list.sort_unstable());
        let list = replay_data::military_aid::volunteer_list_by(lists.values().map(Vec::as_slice), |&(ordinal, _)| ordinal)?;
        self.sent.extend(list.iter().map(|&(_, division)| division));
        let host_of = |division: u64| (division != 0).then(|| divisions.slot(u128::from(division))).flatten().and_then(|slot| divisions.number(columns.logical_owner, slot).value());
        volunteer_hosts(list.iter().copied(), host_of, links)
    }

    /// The same layer composed anew from `mirrors` for the viewer it last saw.
    pub fn composed(&self, mirrors: &Mirrors) -> Self {
        let mut fresh = self.fresh();
        fresh.ours = self.ours;
        if let Some(ours) = self.ours {
            fresh.recompute(mirrors, ours);
        }
        fresh
    }

    /// Countries whose relation differs from `other`'s, and the first described.
    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        self.index.differ(&other.index)
    }
}

/// Each country's faction and the subject pairs, in canonical slots.
fn families(mirrors: &Mirrors, links: &BTreeMap<u32, u32>, columns: Columns) -> Result<Families, String> {
    let link = |raw: u32, what: &str| links.get(&raw).copied().ok_or_else(|| what.to_owned());
    let members = complete(mirrors, TableId::FactionMembers)?;
    let mut factions = BTreeMap::new();
    for slot in members.rows() {
        let raw = members.key(slot).map_or(0, |key| key as u32);
        let country = if raw == 0 { 0 } else { link(raw, "FactionMembers country")? };
        factions.insert(country, members.number(columns.faction, slot).value().unwrap_or(0));
    }
    let subjects = complete(mirrors, TableId::Subjects)?;
    let pairs = subjects
        .rows()
        .filter_map(|slot| subjects.key(slot))
        .map(|key| Ok((link((key >> 32) as u32, "Subjects country")?, link(key as u32, "Subjects country")?)))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Families::from_parts(factions, pairs))
}

/// Whether the viewer at slot `ours` shares counters with country slot zero, from its
/// `CountryCounterInputs` packets (`visibility::forward_zero_shared`).
fn zero_shared(mirrors: &Mirrors, links: &BTreeMap<u32, u32>, ours: u32, columns: Columns) -> Result<bool, String> {
    if ours == 0 {
        return Err(SLOT_ZERO.into());
    }
    let rows = complete(mirrors, TableId::CountryCounterInputs).map_err(|_| "CountryCounterInputs")?;
    let mut matching = rows.rows().filter(|&slot| rows.key(slot).is_some_and(|key| links.get(&(key as u32)) == Some(&ours)));
    let slot = matching.next().ok_or("CountryCounterInputs visibility owner missing")?;
    if matching.next().is_some() {
        return Err("CountryCounterInputs visibility owner duplicated".into());
    }
    let forward = crate::layers::owned::opaque::<Option<ReverseZeroVisibilityInputs>>(rows, columns.forward_visibility, slot).unwrap_or(Recorded::Value(None));
    let zero_war = crate::layers::owned::opaque::<Option<[u8; 2]>>(rows, columns.zero_war, slot).unwrap_or(Recorded::Value(None));
    let zero_subject = crate::layers::owned::opaque::<Option<ZeroSubjectInputs>>(rows, columns.zero_subject, slot).unwrap_or(Recorded::Value(None));
    let faction = || -> Result<Option<u64>, &'static str> {
        let members = complete(mirrors, TableId::FactionMembers).map_err(|_| "FactionMembers")?;
        let mut faction = None;
        for slot in members.rows() {
            let raw = members.key(slot).map_or(0, |key| key as u32);
            let country = if raw == 0 { 0 } else { *links.get(&raw).ok_or("visibility country link")? };
            let held = members.number(columns.faction, slot).value().unwrap_or(0);
            if held == 0 {
                return Err("FactionMembers has null faction identity");
            }
            if country == ours {
                if faction.is_some_and(|previous| previous != held) {
                    return Err("FactionMembers has inconsistent faction identity");
                }
                faction = Some(held);
            }
        }
        Ok(faction)
    };
    let reserved = || -> Result<u64, &'static str> {
        let rows = complete(mirrors, TableId::ReservedCountryNaming).map_err(|_| "ReservedCountryNaming")?;
        let mut held = rows.rows();
        let (Some(slot), None) = (held.next(), held.next()) else { return Err("ReservedCountryNaming singleton") };
        match rows.number(columns.reserved_faction, slot) {
            Cell::Value(faction) => Ok(faction),
            Cell::Missing => Err("ReservedCountryNaming.faction capture refused"),
            Cell::Unrecorded => Err("ReservedCountryNaming.faction not recorded"),
        }
    };
    forward_zero_shared_of(links, &forward, &zero_war, &zero_subject, faction, reserved).map_err(str::to_owned)
}

impl Layer for Relations {
    fn name(&self) -> &'static str {
        "relations"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        #[cfg(test)]
        let reads_vision = self.viewer.is_none();
        #[cfg(not(test))]
        let reads_vision = true;
        if reads_vision {
            inputs.after("vision");
        }
        for table in READ {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            country_slot: inputs.number(TableId::Countries, "canonical_slot")?,
            faction: inputs.number(TableId::FactionMembers, "faction")?,
            sent_division: inputs.number(TableId::SentVolunteers, "division")?,
            logical_owner: inputs.number(TableId::Divisions, "logical_owner")?,
            reserved_faction: inputs.number(TableId::ReservedCountryNaming, "faction")?,
            zero_war: inputs.list(TableId::CountryCounterInputs, "zero_war")?,
            zero_subject: inputs.list(TableId::CountryCounterInputs, "zero_subject")?,
            forward_visibility: inputs.list(TableId::CountryCounterInputs, "forward_visibility")?,
        });
        self.everything();
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.everything();
        Ok(())
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        self.everything();
        self.settle_with(mirrors, upstream, &mut Budget::unlimited())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if self.columns.is_none() {
            return;
        }
        if step.gap() {
            self.everything();
            return;
        }
        let columns = self.columns();
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            if !READ.contains(&table) {
                continue;
            }
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.everything();
                continue;
            }
            let touched = !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty());
            if !touched {
                continue;
            }
            let rows_changed = !changes.deleted().is_empty() || !changes.inserted().is_empty();
            let written = |wanted: &[usize]| changes.columns().iter().any(|written| wanted.contains(&written.column()) && !written.is_empty());
            let read = rows_changed
                || match table {
                    TableId::CountryLinks => written(&[columns.link_slot.column]),
                    TableId::Countries => written(&[columns.country_slot.column]),
                    TableId::FactionMembers => written(&[columns.faction.column]),
                    TableId::SentVolunteers => written(&[columns.sent_division.column]),
                    TableId::ReservedCountryNaming => written(&[columns.reserved_faction.column]),
                    TableId::CountryCounterInputs => {
                        let viewer = |slot: u32| match (mirrors.get(table).and_then(|rows| rows.key(slot)), &self.facts, self.ours) {
                            (Some(key), Some(facts), Some(ours)) => facts.links.get(&(key as u32)) == Some(&ours),
                            _ => true,
                        };
                        let wanted = [columns.zero_war.column, columns.zero_subject.column, columns.forward_visibility.column];
                        changes.columns().iter().filter(|written| wanted.contains(&written.column())).flat_map(|written| written.slots()).any(|&slot| viewer(slot))
                    }
                    TableId::Divisions => true,
                    _ => false,
                };
            if !read {
                continue;
            }
            if table == TableId::Divisions {
                let Some(divisions) = mirrors.get(table) else { continue };
                let keys = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u64);
                let written = changes.columns().iter().filter(|written| written.column() == columns.logical_owner.column).flat_map(|written| written.slots()).filter_map(|&slot| divisions.key(slot)).map(|key| key as u64);
                if keys.chain(written).any(|division| self.sent.contains(&division)) {
                    self.mark(HOSTS, []);
                    self.all = true;
                }
                continue;
            }
            let keys: Vec<u128> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).copied().collect();
            match table {
                TableId::WarRelations | TableId::Subjects => {
                    let part = if table == TableId::WarRelations { WARS } else { FAMILIES };
                    keys.iter().for_each(|&key| self.pair(part, (key >> 32) as u32, key as u32));
                }
                TableId::FactionMembers => {
                    self.mark(FAMILIES | ZERO, [0]);
                    let mirror = mirrors.get(table);
                    let written = changes.columns().iter().flat_map(|written| written.slots()).filter_map(|&slot| mirror.and_then(|rows| rows.key(slot)));
                    let countries: Vec<u32> = keys.iter().copied().chain(written).map(|key| key as u32).collect();
                    for raw in countries {
                        match (self.facts.as_ref().and_then(|facts| if raw == 0 { Some(0) } else { facts.links.get(&raw).copied() }), self.ours) {
                            (Some(slot), Some(ours)) if slot != ours => {
                                let raws = self.raws_of(slot);
                                self.mark(FAMILIES, raws);
                            }
                            _ => self.all = true,
                        }
                    }
                }
                TableId::CountryCounterInputs | TableId::ReservedCountryNaming => self.mark(ZERO, [0]),
                TableId::Countries => {
                    self.mark(COUNTRIES, []);
                    self.all = true;
                }
                TableId::SentVolunteers => {
                    self.mark(HOSTS, []);
                    self.all = true;
                }
                _ => self.everything(),
            }
        }
    }

    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _budget: &mut Budget) -> io::Result<()> {
        if self.columns.is_none() {
            return Ok(());
        }
        let ours = upstream.get::<vision::Vision>().and_then(vision::Vision::ours);
        #[cfg(test)]
        let ours = ours.or(self.viewer);
        if ours != self.ours {
            self.ours = ours;
            self.everything();
        }
        if !self.dirty {
            return Ok(());
        }
        self.dirty = false;
        match ours {
            Some(ours) => self.recompute(mirrors, ours),
            None => {
                let index = Index::unavailable_for("the viewing country is not known: the vision layer draws nothing".into());
                for raw in index.changed_from(&self.index) {
                    self.changed.mark("country", u64::from(raw));
                }
                self.index = Arc::new(index);
            }
        }
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried { entities: usize::from(self.dirty), frames: 0 }
    }

    fn held(&self) -> usize {
        self.index.len() * 96 + self.changed.of("country").len() * 8 + self.sent.capacity() * 16
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn changed(&self) -> Option<&Changed> {
        Some(&self.changed)
    }

    fn clear_changed(&mut self) {
        self.changed.clear();
    }
}

pub mod proof;
#[cfg(test)]
mod tests;
