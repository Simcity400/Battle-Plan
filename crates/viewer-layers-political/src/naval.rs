
mod reads;
#[cfg(test)]
mod tests;

use viewer_layer_core::{layers::{Carried, Settling, Dirty, Inputs, Layer, Upstream, View}, vision::Vision};
use viewer_layer_core::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use reads::Columns;
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::{
    cell::Cell as Used,
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use viewer_foundation::perspective::Choice;
use map_display_data::landscape::Attributes;
use viewer_map_modes::{map_mode::MapMode, naval_dominance::{self, Content, Fact, Relations, Sides}};

type Wars = HashMap<u32, Fact<Vec<u32>>>;

#[derive(Clone, Debug)]
struct Derived {
    links: Fact<BTreeMap<u32, u32>>,
    natives: Fact<HashMap<u32, Vec<u32>>>,
    factions: Fact<BTreeMap<u32, u64>>,
    subjects: Fact<BTreeSet<(u32, u32)>>,
    wars: Fact<Wars>,
    blockades: Fact<Vec<(u32, u32)>>,
}

impl Default for Derived {
    fn default() -> Self {
        fn unread<T>() -> Fact<T> {
            Err("not read".to_owned())
        }
        Self { links: unread(), natives: unread(), factions: unread(), subjects: unread(), wars: unread(), blockades: unread() }
    }
}

fn absent(mirrors: &Mirrors, table: TableId, hour: Hour) -> String {
    match mirrors.get(table).and_then(Mirror::not_recorded) {
        Some(why) => format!("hour {hour} {why}"),
        None => format!("hour {hour} {table:?} is not read"),
    }
}

fn whole(mirrors: &Mirrors, table: TableId) -> Option<&Mirror> {
    mirrors.get(table).filter(|mirror| mirror.complete())
}

impl Derived {
    fn read(&mut self, parts: u16, mirrors: &Mirrors, columns: &Columns, hour: Hour) -> u16 {
        fn put<T: PartialEq>(held: &mut T, new: T, part: u16, moved: &mut u16) {
            if *held != new {
                *held = new;
                *moved |= part;
            }
        }
        let mut moved = 0;
        if parts & reads::LINKS != 0 {
            let links = match whole(mirrors, TableId::CountryLinks) {
                Some(rows) => Ok(rows.rows().filter_map(|slot| reads::link(rows, columns, slot)).collect()),
                None => Err(absent(mirrors, TableId::CountryLinks, hour)),
            };
            put(&mut self.links, links, reads::LINKS, &mut moved);
        }
        let parts = if moved & reads::LINKS != 0 { parts | reads::WARS | reads::FACTIONS | reads::SUBJECTS } else { parts };
        let links = self.links.as_ref().ok();
        if parts & reads::COUNTRIES != 0 {
            let natives = match whole(mirrors, TableId::Countries) {
                Some(rows) => {
                    let mut natives: HashMap<u32, Vec<u32>> = HashMap::new();
                    for (id, canonical) in rows.rows().filter_map(|slot| reads::country(rows, columns, slot)) {
                        natives.entry(canonical).or_default().push(id);
                    }
                    natives.values_mut().for_each(|ids| ids.sort_unstable());
                    Ok(natives)
                }
                None => Err(absent(mirrors, TableId::Countries, hour)),
            };
            put(&mut self.natives, natives, reads::COUNTRIES, &mut moved);
        }
        if parts & reads::WARS != 0 {
            let wars = match (whole(mirrors, TableId::WarRelations), &self.links) {
                (None, _) => Err(absent(mirrors, TableId::WarRelations, hour)),
                (Some(_), Err(why)) => Err(why.clone()),
                (Some(rows), Ok(links)) => Ok(wars(rows, links)),
            };
            put(&mut self.wars, wars, reads::WARS, &mut moved);
        }
        if parts & reads::FACTIONS != 0 {
            let factions = (|| {
                let rows = whole(mirrors, TableId::FactionMembers).ok_or("FactionMembers")?;
                let links = links.ok_or("FactionMembers country")?;
                let mut factions = BTreeMap::new();
                for (raw, faction) in rows.rows().filter_map(|slot| reads::faction_member(rows, columns, slot)) {
                    let country = if raw == 0 { 0 } else { *links.get(&raw).ok_or("FactionMembers country")? };
                    factions.insert(country, faction);
                }
                Ok(factions)
            })()
            .map_err(|why: &str| why.to_owned());
            put(&mut self.factions, factions, reads::FACTIONS, &mut moved);
        }
        if parts & reads::SUBJECTS != 0 {
            let subjects = (|| {
                let rows = whole(mirrors, TableId::Subjects).ok_or("Subjects")?;
                let links = links.ok_or("Subjects country")?;
                let mut subjects = BTreeSet::new();
                for (overlord, subject) in rows.rows().filter_map(|slot| reads::subject(rows, slot)) {
                    let (a, b) = (*links.get(&overlord).ok_or("Subjects country")?, *links.get(&subject).ok_or("Subjects country")?);
                    subjects.extend([(a, b), (b, a)]);
                }
                Ok(subjects)
            })()
            .map_err(|why: &str| why.to_owned());
            put(&mut self.subjects, subjects, reads::SUBJECTS, &mut moved);
        }
        if parts & reads::TUPLES != 0 {
            let blockades = match whole(mirrors, TableId::RelationTuples) {
                Some(rows) => Ok(rows.rows().filter_map(|slot| reads::tuple(rows, columns, slot, naval_dominance::BLOCKADE)).collect()),
                None => Err(absent(mirrors, TableId::RelationTuples, hour)),
            };
            put(&mut self.blockades, blockades, reads::TUPLES, &mut moved);
        }
        moved
    }

    fn bytes(&self) -> usize {
        self.links.as_ref().map_or(0, BTreeMap::len) * 24
            + self.natives.as_ref().map_or(0, |natives| natives.len() * 48)
            + self.wars.as_ref().map_or(0, |wars| wars.values().map(|targets| 48 + targets.as_ref().map_or(0, Vec::capacity) * 4).sum())
            + self.factions.as_ref().map_or(0, BTreeMap::len) * 32 + self.subjects.as_ref().map_or(0, BTreeSet::len) * 24 + self.blockades.as_ref().map_or(0, Vec::capacity) * 8
    }
}

fn wars(rows: &Mirror, links: &BTreeMap<u32, u32>) -> Wars {
    let mut wars: Wars = HashMap::new();
    let refuse = |wars: &mut Wars, receiver: u32, why: String| {
        let held = wars.entry(receiver).or_insert_with(|| Ok(Vec::new()));
        if held.is_ok() {
            *held = Err(why);
        }
    };
    for (first, second) in rows.rows().filter_map(|slot| reads::war(rows, slot)) {
        let (a, b) = (links.get(&first).copied(), links.get(&second).copied());
        if let (Some(a), Some(b)) = (a, b)
            && a == b
        {
            if a != 0 {
                refuse(&mut wars, a, format!("WarRelations joins country slot {a} to itself"));
            }
            continue;
        }
        for (receiver, other, target) in [(a, second, b), (b, first, a)] {
            let Some(receiver) = receiver.filter(|&receiver| receiver != 0) else { continue };
            match target {
                None => refuse(&mut wars, receiver, format!("WarRelations names country {other}, which CountryLinks lacks")),
                Some(0) => refuse(&mut wars, receiver, format!("WarRelations links country {other} to the reserved slot")),
                Some(target) => {
                    if let Ok(targets) = wars.entry(receiver).or_insert_with(|| Ok(Vec::new())) {
                        targets.push(target);
                    }
                }
            }
        }
    }
    for targets in wars.values_mut().flatten() {
        targets.sort_unstable();
        targets.dedup();
    }
    wars
}

struct MirrorRelations<'a> {
    mirrors: &'a Mirrors,
    columns: &'a Columns,
    derived: &'a Derived,
    hour: Hour,
    used: Used<u16>,
}

impl MirrorRelations<'_> {
    fn uses(&self, part: u16) {
        self.used.set(self.used.get() | part);
    }

    fn absent(&self, table: TableId) -> String {
        self.uses(reads::HOUR);
        absent(self.mirrors, table, self.hour)
    }

    fn naming(&self, part: u16) -> Result<Option<(&Mirror, u32)>, String> {
        self.uses(part);
        let rows = whole(self.mirrors, TableId::ReservedCountryNaming).ok_or_else(|| self.absent(TableId::ReservedCountryNaming))?;
        let mut live = rows.rows();
        Ok(match (live.next(), live.next()) {
            (Some(slot), None) => Some((rows, slot)),
            _ => None,
        })
    }

    fn natives(&self, slot: u32) -> Result<&[u32], String> {
        self.uses(reads::COUNTRIES);
        let natives = self.derived.natives.as_ref().map_err(|why| {
            self.uses(reads::HOUR);
            why.clone()
        })?;
        Ok(natives.get(&slot).map_or(&[][..], Vec::as_slice))
    }

    fn links_used(&self) -> Result<&BTreeMap<u32, u32>, String> {
        self.uses(reads::LINKS);
        self.derived.links.as_ref().map_err(|why| {
            self.uses(reads::HOUR);
            why.clone()
        })
    }

    fn zero_subject(&self, links: &BTreeMap<u32, u32>, source: u32, viewer: u32) -> Fact<bool> {
        if source == 0 {
            let (rows, slot) = self.naming(reads::NAMING_REVERSE)?.ok_or("Country0 naval family naming singleton missing or duplicated")?;
            self.uses(reads::HOUR);
            let reverse = reads::naming_reverse(rows, self.columns, slot)?;
            let packet = reverse.require("ReservedCountryNaming.reverse_visibility")?.as_ref().ok_or("Country0 naval family reverse visibility refused")?;
            if !packet.sample_hour.fresh_at(self.hour) {
                return Err("Country0 naval family reverse visibility stale".into());
            }
            let cell = packet.cells.get(viewer as usize).and_then(Option::as_ref).ok_or("Country0 naval family target cell missing")?;
            let subject = cell.subject.as_ref().ok_or("Country0 naval family cached subject pointer not recorded at the reached branch")?;
            return naval_dominance::subject(subject, cell.target.ok_or_else(|| "Country0 naval family target refused".into()), links);
        }
        let country = *self.natives(source)?.first().ok_or("naval family source country missing")?;
        self.uses(reads::FORWARD);
        let rows = whole(self.mirrors, TableId::CountryCounterInputs).ok_or_else(|| self.absent(TableId::CountryCounterInputs))?;
        let slot = reads::counter_row(rows, country).ok_or("Country0 naval family forward source missing")?;
        let forward = reads::forward(rows, self.columns, slot)?;
        let packet = forward.require("CountryCounterInputs.forward_visibility")?.as_ref().ok_or("Country0 naval family forward visibility refused")?;
        let subject = packet.subject.as_ref().ok_or("naval family cached subject pointer not recorded at the reached branch")?;
        naval_dominance::subject(subject, packet.target.ok_or_else(|| "naval family target refused".into()), links)
    }

    fn derived_war(&self, slot: u32, target: u32) -> Fact<bool> {
        self.uses(reads::WARS | reads::LINKS);
        let wars = self.derived.wars.as_ref().map_err(|why| {
            self.uses(reads::HOUR);
            why.clone()
        })?;
        match wars.get(&slot) {
            None => Ok(false),
            Some(Ok(targets)) => Ok(targets.binary_search(&target).is_ok()),
            Some(Err(why)) => Err(why.clone()),
        }
    }
}

impl Relations for MirrorRelations<'_> {
    fn links(&self) -> Result<&BTreeMap<u32, u32>, String> {
        self.links_used()
    }

    fn family(&self, links: &BTreeMap<u32, u32>, source: u32, viewer: u32) -> Fact<bool> {
        if source == viewer && source != 0 {
            return Ok(true);
        }
        let zero_faction = if viewer == 0 {
            let (rows, slot) = self.naming(reads::NAMING_FACTION)?.ok_or("Country0 naval family naming singleton missing or duplicated")?;
            let faction = *reads::naming_faction(rows, self.columns, slot).require("ReservedCountryNaming.faction")?;
            Some(faction.ok_or("Country0 naval family faction capture refused")?)
        } else {
            None
        };
        let roster_member = if zero_faction == Some(0) {
            source == 0
        } else {
            self.uses(reads::FACTIONS | reads::LINKS);
            let factions = self.derived.factions.as_ref().map_err(Clone::clone)?;
            match zero_faction {
                Some(handle) => factions.get(&source) == Some(&handle),
                None => factions.get(&source).is_some_and(|faction| factions.get(&viewer) == Some(faction)),
            }
        };
        if roster_member {
            return Ok(true);
        }
        if source == 0 || viewer == 0 {
            return self.zero_subject(links, source, viewer);
        }
        self.uses(reads::SUBJECTS | reads::LINKS);
        let subjects = self.derived.subjects.as_ref().map_err(Clone::clone)?;
        Ok(source == viewer || subjects.contains(&(source, viewer)))
    }

    fn war(&self, source: u32, viewer: u32) -> Fact<bool> {
        let slot = if source == 0 {
            0
        } else {
            *self.links_used()?.get(&source).ok_or("directed-war raw country link missing")?
        };
        if slot == 0 {
            let (rows, at) = self.naming(reads::NAMING_REVERSE)?.ok_or("directed-war actual-zero naming singleton missing or duplicated")?;
            self.uses(reads::HOUR);
            let reverse = reads::naming_reverse(rows, self.columns, at)?;
            let packet = reverse.require("ReservedCountryNaming.reverse_visibility")?.as_ref().ok_or("ReservedCountryNaming.reverse_visibility capture refused")?;
            if !packet.sample_hour.fresh_at(self.hour) {
                return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch".into());
            }
            if packet.cells.len() > 65_536 {
                return Err("ReservedCountryNaming.reverse_visibility matrix domain invalid".into());
            }
            let cell = packet.cells.get(viewer as usize).and_then(Option::as_ref).ok_or("ReservedCountryNaming.reverse_visibility cell capture refused or outside observed matrix")?;
            return match cell.war.ok_or("ReservedCountryNaming.reverse_visibility war capture refused")? {
                [0, 0] => Ok(false),
                [1, retired] => Ok(retired == 0),
                _ => Err("ReservedCountryNaming.reverse_visibility war presence invalid".into()),
            };
        }
        let natives = self.natives(slot)?;
        let actual = match natives[..] {
            [] => return Err("directed-war receiver Countries row missing".into()),
            [id] => id,
            _ => return Err("directed-war receiver Countries row duplicated".into()),
        };
        self.uses(reads::DIRECTED);
        let packet = whole(self.mirrors, TableId::CountryCounterInputs)
            .and_then(|rows| Some((rows, reads::counter_row(rows, actual)?)))
            .and_then(|(rows, at)| reads::directed(rows, self.columns, at).ok())
            .and_then(|packet| packet.require("CountryCounterInputs.directed_wars").ok().cloned().flatten());
        match packet {
            Some(packet) => {
                self.uses(reads::HOUR);
                packet.admit(self.hour, actual as i32, slot)?.pointer_active(viewer)
            }
            None => self.derived_war(slot, viewer),
        }
    }

    fn blockades(&self) -> Fact<&[(u32, u32)]> {
        self.uses(reads::TUPLES);
        self.derived.blockades.as_deref().map_err(|why| {
            self.uses(reads::HOUR);
            why.clone()
        })
    }
}

const ALL_PARTS: u16 = u16::MAX;

const WHOLE: u16 = reads::LINKS | reads::TUPLES;

const DERIVED: u16 = reads::LINKS | reads::COUNTRIES | reads::FACTIONS | reads::SUBJECTS | reads::WARS | reads::TUPLES;

fn kind(status: Option<Status>) -> Option<Status> {
    status.map(|status| match status {
        Status::Truncated { .. } => Status::Truncated { missing: 0 },
        status => status,
    })
}

pub struct Naval {
    content: Arc<Content>,
    columns: Option<Columns>,
    choice: Choice,
    viewer: Option<u32>,
    hour: Option<Hour>,
    groups: HashMap<u32, Vec<(u32, u32)>>,
    attributes: Arc<BTreeMap<u32, Attributes>>,
    refusals: BTreeMap<u32, String>,
    regions: Dirty,
    sides: Sides,
    uses: u16,
    touched: u16,
    stale: u16,
    derived: Derived,
    epoch: Fact<()>,
    supremacy: Option<String>,
    statuses: HashMap<usize, Option<Status>>,
    gap: bool,
    handed: bool,
    frame: u64,
    marked: [u64; 3],
    clears: [u64; 3],
    moved: u16,
}

impl Naval {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            content,
            columns: None,
            choice: Choice::default(),
            viewer: None,
            hour: None,
            groups: HashMap::new(),
            attributes: Arc::default(),
            refusals: BTreeMap::new(),
            regions: Dirty::default(),
            sides: Sides::default(),
            uses: 0,
            touched: 0,
            stale: ALL_PARTS,
            derived: Derived::default(),
            epoch: Err("not composed yet".into()),
            supremacy: None,
            statuses: HashMap::new(),
            gap: false,
            handed: true,
            frame: 0,
            marked: [0; 3],
            clears: [0; 3],
            moved: 0,
        }
    }

    pub fn composed(&self, mirrors: &Mirrors) -> Self {
        let mut fresh = Self { columns: self.columns, choice: self.choice.clone(), ..Self::new(Arc::clone(&self.content)) };
        fresh.compose_at(mirrors, self.viewer);
        fresh
    }

    pub fn attributes(&self) -> &Arc<BTreeMap<u32, Attributes>> {
        &self.attributes
    }

    pub fn take_changed(&mut self) -> Option<Arc<BTreeMap<u32, Attributes>>> {
        std::mem::take(&mut self.handed).then(|| Arc::clone(&self.attributes))
    }

    pub fn warnings(&self) -> Vec<String> {
        let refused: Vec<String> = self.refusals.values().cloned().collect();
        naval_dominance::withheld(&refused).into_iter().collect()
    }

    pub fn viewer(&self) -> Option<u32> {
        self.viewer
    }

    pub fn drawn(&self) -> bool {
        self.columns.is_some()
    }

    pub fn choice(&self) -> &Choice {
        &self.choice
    }

    pub fn marked(&self) -> [u64; 3] {
        self.marked
    }

    pub fn clears(&self) -> ([u64; 3], u16) {
        (self.clears, self.moved)
    }

    fn mark_all(&mut self) {
        let ids: Vec<u32> = self.content.ids().collect();
        for id in ids {
            if self.regions.mark(id, self.frame) {
                self.marked[0] += 1;
            }
        }
    }

    fn mark(&mut self, region: u32) {
        if self.content.sea(region) && self.regions.mark(region, self.frame) {
            self.marked[0] += 1;
        }
    }

    fn compose_at(&mut self, mirrors: &Mirrors, viewer: Option<u32>) {
        let (marked, clears, moved) = (self.marked, self.clears, self.moved);
        *self = Self { columns: self.columns, choice: self.choice.clone(), ..Self::new(Arc::clone(&self.content)) };
        if self.columns.is_none() {
            (self.marked, self.clears, self.moved) = (marked, clears, moved);
            return;
        }
        self.statuses = mirrors.iter().map(|mirror| (mirror.table().index(), kind(mirror.status()))).collect();
        if let Some(rows) = mirrors.get(TableId::NavalSupremacy) {
            for slot in rows.rows() {
                if let Some(key) = rows.key(slot) {
                    let (region, country) = reads::supremacy_key(key);
                    self.groups.entry(region).or_default().push((country, slot));
                }
            }
            self.groups.values_mut().for_each(|rows| rows.sort_unstable());
        }
        self.mark_all();
        self.settle_at(mirrors, viewer);
        (self.marked, self.clears, self.moved) = (marked, clears, moved);
    }

    fn settle_at(&mut self, mirrors: &Mirrors, viewer: Option<u32>) {
        let Some(columns) = self.columns else { return };
        self.frame += 1;
        let viewer = if self.gap { None } else { viewer };
        let hour = mirrors.get(TableId::NavalSupremacy).and_then(Mirror::head).unwrap_or(0);
        if self.hour != Some(hour) {
            self.touched |= reads::HOUR;
            self.hour = Some(hour);
            if self.derived.links.is_err() {
                self.stale |= reads::LINKS;
            }
            if self.derived.blockades.is_err() {
                self.stale |= reads::TUPLES;
            }
            if self.derived.natives.is_err() {
                self.stale |= reads::COUNTRIES;
            }
            if self.derived.wars.is_err() {
                self.stale |= reads::WARS;
            }
        }
        let read = std::mem::take(&mut self.stale) & DERIVED;
        if read != 0 {
            let moved = self.derived.read(read, mirrors, &columns, hour);
            self.touched &= !(read & DERIVED & !moved);
        }
        let moved = std::mem::take(&mut self.touched) & self.uses;
        if viewer != self.viewer || moved & WHOLE != 0 {
            self.clears[usize::from(viewer == self.viewer)] += 1;
            self.moved |= moved;
            self.viewer = viewer;
            self.sides.clear();
            self.uses = 0;
            self.mark_all();
        } else if moved != 0
            && let Some(viewer) = viewer
        {
            self.clears[1] += 1;
            self.moved |= moved;
            let relations = MirrorRelations { mirrors, columns: &columns, derived: &self.derived, hour, used: Used::new(0) };
            let sources = self.sides.reread(&relations, viewer);
            self.uses = relations.used.get();
            self.marked[2] += self.sides.len() as u64;
            let regions: Vec<u32> = self.groups.iter().filter(|(_, rows)| sources.iter().any(|&source| rows.binary_search_by_key(&source, |row| row.0).is_ok())).map(|(&region, _)| region).collect();
            regions.into_iter().for_each(|region| self.mark(region));
        }
        let epoch = naval_dominance::cache_epoch(hour, mirrors.get(TableId::NavalSupremacy).and_then(Mirror::sampled));
        let supremacy = (!mirrors.get(TableId::NavalSupremacy).is_some_and(Mirror::complete)).then(|| absent(mirrors, TableId::NavalSupremacy, hour));
        if epoch != self.epoch || supremacy != self.supremacy {
            self.clears[2] += 1;
            (self.epoch, self.supremacy) = (epoch, supremacy);
            self.mark_all();
        }
        let Some(viewer) = self.viewer else {
            if !self.attributes.is_empty() || !self.refusals.is_empty() {
                (self.attributes, self.handed) = (Arc::default(), true);
                self.refusals.clear();
            }
            self.regions.clear();
            return;
        };
        if self.regions.since().is_none() {
            return;
        }
        let relations = MirrorRelations { mirrors, columns: &columns, derived: &self.derived, hour, used: Used::new(0) };
        let (content, groups, epoch, supremacy) = (&*self.content, &self.groups, &self.epoch, &self.supremacy);
        let (attributes, refusals, sides) = (Arc::make_mut(&mut self.attributes), &mut self.refusals, &mut self.sides);
        let (mut changed, mut settled, before) = (false, 0, sides.len());
        let mut values = Vec::new();
        let _ = self.regions.settle(|id| {
            settled += 1;
            values.clear();
            let mut failed = supremacy.clone();
            if let (None, Some(rows)) = (&failed, mirrors.get(TableId::NavalSupremacy)) {
                for &(country, slot) in groups.get(&id).map_or(&[][..], Vec::as_slice) {
                    match reads::supremacy(rows, &columns, slot, country) {
                        Some(value) => values.push(value),
                        None => {
                            failed = Some(format!("NavalSupremacy row of country {country} in region {id} is not recorded"));
                            break;
                        }
                    }
                }
            }
            let read = match failed {
                Some(why) => Err(why),
                None => Ok(&values[..]),
            };
            let (value, reason) = content.region(id, epoch, read, &relations, viewer, sides);
            let key = id + 1;
            changed |= attributes.insert(key, value) != Some(value);
            match reason {
                Some(reason) => {
                    changed |= refusals.insert(id, reason.clone()).as_ref() != Some(&reason);
                }
                None => changed |= refusals.remove(&id).is_some(),
            }
            Ok(())
        });
        self.marked[1] += settled;
        self.marked[2] += (self.sides.len() - before.min(self.sides.len())) as u64;
        self.uses |= relations.used.get();
        self.handed |= changed;
    }
}

impl Layer for Naval {
    fn take_naval_changed(&mut self) -> Option<Arc<BTreeMap<u32, Attributes>>> {
        self.take_changed()
    }

    fn name(&self) -> &'static str {
        "naval"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if view.map_mode != MapMode::default() {
            *self = Self { choice: view.perspective.clone(), ..Self::new(Arc::clone(&self.content)) };
            return Ok(());
        }
        self.choice = view.perspective.clone();
        inputs.after("vision");
        self.columns = Some(Columns::declare(inputs)?);
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_at(mirrors, None);
        Ok(())
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.compose_at(mirrors, upstream.get::<Vision>().and_then(Vision::ours));
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        if step.gap() {
            self.gap = true;
            return;
        }
        if std::mem::take(&mut self.gap) {
            self.touched = ALL_PARTS;
            self.stale = ALL_PARTS;
        }
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            if !reads::TABLES.contains(&table) {
                continue;
            }
            let Some(mirror) = mirrors.get(table) else { continue };
            let mut parts = 0;
            if let Some((status, _)) = changes.status()
                && self.statuses.insert(changes.table(), kind(Some(status))) != Some(kind(Some(status)))
            {
                parts |= columns.part(table, None);
            }
            let read = |key: Option<u128>| table != TableId::RelationTuples || key.is_some_and(|key| reads::tuple_kind(key) == naval_dominance::BLOCKADE);
            let keys = |keys: Option<&[u128]>, slots: &[u32]| match keys {
                Some(keys) => keys.iter().any(|&key| read(Some(key))),
                None => !slots.is_empty() && read(None),
            };
            if keys(changes.deleted().keys(), changes.deleted().slots()) || keys(changes.inserted().keys(), changes.inserted().slots()) {
                parts |= columns.part(table, None);
            }
            for written in changes.columns().iter().filter(|written| !written.is_empty()) {
                if written.slots().iter().any(|&slot| read(mirror.key(slot))) {
                    parts |= columns.part(table, Some(written.column()));
                }
            }
            if table == TableId::NavalSupremacy {
                for &key in changes.deleted().keys().unwrap_or_default() {
                    let (region, country) = reads::supremacy_key(key);
                    if let Some(rows) = self.groups.get_mut(&region) {
                        rows.retain(|&(held, _)| held != country);
                    }
                    self.mark(region);
                }
                for (at, &slot) in changes.inserted().slots().iter().enumerate() {
                    let Some(&key) = changes.inserted().keys().and_then(|keys| keys.get(at)) else { continue };
                    let (region, country) = reads::supremacy_key(key);
                    let rows = self.groups.entry(region).or_default();
                    rows.retain(|&(held, _)| held != country);
                    let at = rows.partition_point(|&(held, _)| held < country);
                    rows.insert(at, (country, slot));
                    self.mark(region);
                }
                for written in changes.columns().iter().filter(|written| columns.part(table, Some(written.column())) != 0) {
                    for &slot in written.slots() {
                        if let Some(key) = mirror.key(slot) {
                            self.mark(reads::supremacy_key(key).0);
                        }
                    }
                }
                continue;
            }
            self.touched |= parts;
            self.stale |= parts;
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_at(mirrors, self.viewer);
        Ok(())
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _settling: Settling) -> io::Result<()> {
        self.settle_at(mirrors, upstream.get::<Vision>().and_then(Vision::ours));
        Ok(())
    }


    fn carried(&self) -> Carried {
        Carried { entities: self.regions.len() }
    }

    fn held(&self) -> usize {
        self.groups.capacity() * 40
            + self.groups.values().map(|rows| rows.capacity() * 8).sum::<usize>()
            + self.attributes.len() * 32
            + self.refusals.values().map(|text| text.len() + 40).sum::<usize>()
            + self.regions.bytes()
            + self.sides.len() * 48
            + self.derived.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

