
use viewer_layer_core::{layers::{Carried, Settling, Changed, Inputs, Keys, Layer, List, Num, Upstream, View}, vision};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::{Recorded, ReverseZeroVisibilityInputs, TableId, ZeroSubjectInputs};
use replay_derive::directed_war_derive::ReceiverWars;
use viewer_foundation::visibility::{Families, forward_zero_shared_of};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::Arc,
};
use viewer_counter_kit::type_tint::{Index, Receiver, RelationFacts, volunteer_hosts};

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

const LINKS: u8 = 1;
const COUNTRIES: u8 = 2;
const WARS: u8 = 4;
const FAMILIES: u8 = 8;
const ZERO: u8 = 16;
const HOSTS: u8 = 32;
const EVERY_PART: u8 = 63;

pub struct Facts {
    links: Arc<BTreeMap<u32, u32>>,
    countries: Arc<Result<HashMap<u32, (u32, bool)>, String>>,
    wars: Arc<Result<Vec<(u32, u32)>, String>>,
    families: Arc<Result<Families, String>>,
    zero: (u32, Result<bool, String>),
}

pub struct Country<'a> {
    facts: &'a Facts,
    actual: i32,
    slot: u32,
    wars: std::cell::OnceCell<Result<ReceiverWars, String>>,
}

impl Facts {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WarUnknown {
    Receiver(String),
    Operands(String),
}

pub struct Relations {
    columns: Option<Columns>,
    index: Arc<Index>,
    changed: Changed,
    dirty: bool,
    parts: u8,
    all: bool,
    owed: Keys,
    hosts_new: bool,
    hosts: Result<Vec<(u32, u32)>, String>,
    ours: Option<u32>,
    sent: HashSet<u64>,
    statuses: HashMap<usize, Status>,
    recomputes: u64,
    decided: u64,
    facts: Option<Arc<Facts>>,
    #[cfg(any(test, feature = "test-support"))]
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
            owed: Keys::default(),
            hosts_new: false,
            hosts: Ok(Vec::new()),
            ours: None,
            sent: HashSet::new(),
            statuses: HashMap::new(),
            recomputes: 0,
            decided: 0,
            facts: None,
            #[cfg(any(test, feature = "test-support"))]
            viewer: None,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new() }
    }

    pub fn index(&self) -> &Arc<Index> {
        &self.index
    }

    pub fn wars(&self) -> Option<&Arc<Facts>> {
        self.facts.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn viewed_as(mut self, ours: u32) -> Self {
        self.viewer = Some(ours);
        self
    }

    pub fn recomputes(&self) -> u64 {
        self.recomputes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn decided(&self) -> u64 {
        self.decided
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the relations layer is declared before it composes")
    }

    fn everything(&mut self) {
        self.dirty = true;
        self.parts = EVERY_PART;
        self.all = true;
    }

    fn mark_all(&mut self, links: &BTreeMap<u32, u32>) {
        self.all = false;
        let frame = self.recomputes;
        let raws: Vec<u32> = links.keys().copied().chain([0]).chain(self.index.countries()).collect();
        for raw in raws {
            self.owed.mark(u64::from(raw), frame);
        }
    }

    fn mark(&mut self, part: u8, raws: impl IntoIterator<Item = u32>) {
        self.dirty = true;
        self.parts |= part;
        let frame = self.recomputes;
        for raw in raws {
            self.owed.mark(u64::from(raw), frame);
        }
    }

    fn raws_of(&self, slot: u32) -> Vec<u32> {
        let mut raws: Vec<u32> = self.facts.iter().flat_map(|facts| facts.links.iter().filter(move |&(_, &held)| held == slot).map(|(&raw, _)| raw)).collect();
        if slot == 0 {
            raws.push(0);
        }
        raws
    }

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

    fn recompute(&mut self, mirrors: &Mirrors, ours: u32) {
        self.recomputes += 1;
        let columns = self.columns();
        let old = self.facts.take();
        let (parts, all) = match &old {
            Some(_) if self.parts & LINKS == 0 => (self.parts, self.all),
            _ => (EVERY_PART, true),
        };
        self.all = all;
        self.parts = 0;
        let links = match (&old, parts & LINKS) {
            (Some(old), 0) => Arc::clone(&old.links),
            _ => match complete(mirrors, TableId::CountryLinks) {
                Err(why) => {
                    let index = Index::unavailable_for(why);
                    for raw in index.changed_from(&self.index) {
                        self.changed.mark("country", u64::from(raw));
                    }
                    (self.index, self.facts) = (Arc::new(index), None);
                    self.owed.clear();
                    self.all = false;
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
        if old.is_none() || parts & HOSTS != 0 || std::mem::take(&mut self.hosts_new) {
            let read = self.read_hosts(mirrors, &links, ours, columns);
            self.all |= std::mem::replace(&mut self.hosts, read) != self.hosts;
        }
        let facts = Facts { links, countries, wars, families, zero };
        if self.all {
            self.mark_all(&facts.links);
        }
        let hosts = self.hosts.as_deref().map_err(Clone::clone);
        if self.owed.len() == 0 && self.index.len() > 0 && hosts.as_ref().err() == self.index.volunteer_problem() {
            self.facts = Some(Arc::new(facts));
            return;
        }
        let decided = self.owed.take();
        self.decided += decided.len() as u64;
        let mut index = self.index.clone_shared();
        let mut changed = std::mem::take(&mut self.changed);
        index.remake_in_place(&facts, &facts.links, ours, hosts, decided.into_iter().map(|raw| raw as u32), &mut |raw| changed.mark("country", u64::from(raw)));
        (self.changed, self.index, self.facts) = (changed, Arc::new(index), Some(Arc::new(facts)));
    }

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

    pub fn composed(&self, mirrors: &Mirrors) -> Self {
        let mut fresh = self.fresh();
        fresh.ours = self.ours;
        if let Some(ours) = self.ours {
            fresh.recompute(mirrors, ours);
        }
        fresh
    }

    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        self.index.differ(&other.index)
    }
}

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
    let forward = viewer_layer_core::owned::opaque::<Option<ReverseZeroVisibilityInputs>>(rows, columns.forward_visibility, slot).unwrap_or(Recorded::Value(None));
    let zero_war = viewer_layer_core::owned::opaque::<Option<[u8; 2]>>(rows, columns.zero_war, slot).unwrap_or(Recorded::Value(None));
    let zero_subject = viewer_layer_core::owned::opaque::<Option<ZeroSubjectInputs>>(rows, columns.zero_subject, slot).unwrap_or(Recorded::Value(None));
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

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
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
        self.settle_with(mirrors, upstream, Settling::Whole)
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
                    let mirror = mirrors.get(table);
                    let written = changes.columns().iter().flat_map(|written| written.slots()).filter_map(|&slot| mirror.and_then(|rows| rows.key(slot)));
                    let countries: Vec<u32> = keys.iter().copied().chain(written).map(|key| key as u32).collect();
                    self.mark(COUNTRIES, []);
                    for raw in countries {
                        match (self.facts.as_ref().and_then(|facts| if raw == 0 { Some(0) } else { facts.links.get(&raw).copied() }), self.ours) {
                            (Some(slot), Some(our)) if slot != our => {
                                let mut raws = self.raws_of(slot);
                                raws.push(raw);
                                self.mark(COUNTRIES, raws);
                            }
                            _ => self.all = true,
                        }
                    }
                }
                TableId::SentVolunteers => self.mark(HOSTS, []),
                _ => self.everything(),
            }
        }
    }

    fn settle(&mut self, _mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        Ok(())
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _settling: Settling) -> io::Result<()> {
        if self.columns.is_none() {
            return Ok(());
        }
        let ours = upstream.get::<vision::Vision>().and_then(vision::Vision::ours);
        #[cfg(any(test, feature = "test-support"))]
        let ours = ours.or(self.viewer);
        if ours != self.ours {
            self.ours = ours;
            self.everything();
        }
        if !self.dirty && self.owed.len() == 0 {
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
                self.owed.clear();
            }
        }
        Ok(())
    }


    fn carried(&self) -> Carried {
        let marked = if self.all { self.index.len().max(1) } else { 0 };
        Carried { entities: self.owed.len() + marked + usize::from(self.dirty) }
    }

    fn held(&self) -> usize {
        self.index.len() * 96 + self.changed.bytes() + self.sent.capacity() * 16 + self.owed.bytes()
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

#[cfg(test)]
mod tests;
