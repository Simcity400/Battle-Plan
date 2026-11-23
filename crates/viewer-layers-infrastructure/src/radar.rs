
use viewer_layer_core::layers::{Carried, Settling, Dirty, Gpu, Inputs, Layer, Num, Upstream, View};
use viewer_layer_core::{vision::{Vision, complete}, playhead_host::mirror::{Mirror, Mirrors}};
use playhead::Step;
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_aircraft::radar_symbols::Group;

#[derive(Clone, Copy)]
struct Columns {
    link_slot: Num<u32>,
    faction: Num<u64>,
    state: Num<u32>,
    radius: Num<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    country: u32,
    ordinal: u32,
    state: Option<u32>,
    radius: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Families {
    factions: BTreeMap<u32, u64>,
    subjects: BTreeSet<(u32, u32)>,
}

impl Families {
    fn family(&self, a: u32, b: u32) -> bool {
        a == b || self.subjects.contains(&(a, b)) || self.factions.get(&a).is_some_and(|faction| self.factions.get(&b) == Some(faction))
    }
}

pub struct Radar {
    columns: Option<Columns>,
    groups: BTreeMap<u128, Entry>,
    links: BTreeMap<u32, u32>,
    families: Result<Families, &'static str>,
    refused: Option<String>,
    dirty: Dirty,
    keys: Vec<u128>,
    index: BTreeMap<u128, u32>,
    whole: bool,
    ours: Option<u32>,
    field: Option<Arc<viewer_aircraft::radar_symbols::Field>>,
    field_dirty: bool,
    statuses: std::collections::HashMap<usize, replay_columnar::manifest::Status>,
    frame: u64,
    marked: u64,
}

impl Radar {
    pub fn new() -> Self {
        Self {
            columns: None,
            groups: BTreeMap::new(),
            links: BTreeMap::new(),
            families: Err("not read"),
            refused: Some("not composed yet".into()),
            dirty: Dirty::default(),
            keys: Vec::new(),
            index: BTreeMap::new(),
            whole: true,
            ours: None,
            field: None,
            field_dirty: true,
            statuses: std::collections::HashMap::new(),
            frame: 0,
            marked: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new() }
    }

    pub fn marked(&self) -> u64 {
        self.marked
    }

    pub fn field(&self) -> Option<Arc<viewer_aircraft::radar_symbols::Field>> {
        self.field.clone()
    }

    pub fn ours(&self) -> Option<u32> {
        self.ours
    }

    pub fn groups(&self, ours: u32) -> Result<Vec<Group>, String> {
        if let Some(why) = &self.refused {
            return Err(why.clone());
        }
        Ok(self
            .groups
            .values()
            .map(|entry| {
                let country = self.links.get(&entry.country).copied();
                let admitted = match (country, &self.families) {
                    (Some(country), _) if country == ours => Ok(true),
                    (Some(country), Ok(families)) => Ok(families.family(country, ours)),
                    (_, Err(reason)) => Err((*reason).to_owned()),
                    (None, Ok(_)) => Ok(false),
                };
                Group { country, ordinal: entry.ordinal, admitted, state: entry.state, radius: entry.radius }
            })
            .collect())
    }

    fn mark(&mut self, key: u128) {
        let next = self.keys.len() as u32;
        let at = *self.index.entry(key).or_insert(next);
        if at == next {
            self.keys.push(key);
        }
        if self.dirty.mark(at, self.frame) {
            self.marked += 1;
        }
    }

    fn read_whole(&mut self, mirrors: &Mirrors) {
        let columns = self.columns.expect("declared");
        self.field_dirty = true;
        self.refused = None;
        for table in [TableId::CountryLinks, TableId::RadarCoverage] {
            if complete(mirrors, table).is_none() {
                self.refused = Some(mirrors.get(table).and_then(Mirror::not_recorded).unwrap_or_else(|| format!("{table:?} is not read")));
            }
        }
        self.links.clear();
        if let Some(links) = complete(mirrors, TableId::CountryLinks) {
            for slot in links.rows() {
                if let (Some(key), Some(canonical)) = (links.key(slot), links.number(columns.link_slot, slot).value()) {
                    self.links.insert(key as u32, canonical);
                }
            }
        }
        self.families = (|| {
            let mut families = Families::default();
            let factions = complete(mirrors, TableId::FactionMembers).ok_or("FactionMembers")?;
            for slot in factions.rows() {
                let Some(key) = factions.key(slot) else { continue };
                let raw = key as u32;
                let country = if raw == 0 { 0 } else { *self.links.get(&raw).ok_or("FactionMembers country")? };
                families.factions.insert(country, factions.number(columns.faction, slot).value().unwrap_or(0));
            }
            let subjects = complete(mirrors, TableId::Subjects).ok_or("Subjects")?;
            for key in subjects.rows().filter_map(|slot| subjects.key(slot)) {
                let a = *self.links.get(&((key >> 32) as u32)).ok_or("Subjects country")?;
                let b = *self.links.get(&(key as u32)).ok_or("Subjects country")?;
                families.subjects.extend([(a, b), (b, a)]);
            }
            Ok(families)
        })();
        self.groups.clear();
        if let Some(rows) = mirrors.get(TableId::RadarCoverage) {
            for key in rows.rows().filter_map(|slot| rows.key(slot)).collect::<Vec<_>>() {
                self.mark(key);
            }
        }
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        if self.whole {
            self.whole = false;
            self.read_whole(mirrors);
        }
        let columns = self.columns.expect("declared");
        let rows = complete(mirrors, TableId::RadarCoverage);
        let mut dirty = std::mem::take(&mut self.dirty);
        let _ = dirty.settle(|at| {
            let key = self.keys[at as usize];
            let entry = rows.and_then(|rows| rows.slot(key).map(|slot| Entry { country: (key >> 32) as u32, ordinal: key as u32, state: rows.number(columns.state, slot).value(), radius: rows.number(columns.radius, slot).value() }));
            let held = match entry {
                Some(entry) => self.groups.insert(key, entry),
                None => self.groups.remove(&key),
            };
            self.field_dirty |= held != entry;
            Ok(())
        });
        self.dirty = dirty;
    }
}

impl Layer for Radar {
    fn held_viewer(&self) -> Option<u32> {
        self.ours()
    }

    fn name(&self) -> &'static str {
        "radar"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.after("vision");
        for table in [TableId::CountryLinks, TableId::FactionMembers, TableId::Subjects, TableId::RadarCoverage] {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            faction: inputs.number(TableId::FactionMembers, "faction")?,
            state: inputs.number(TableId::RadarCoverage, "state")?,
            radius: inputs.number(TableId::RadarCoverage, "radius")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        *self = Self { columns: self.columns, ..Self::new() };
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        self.settle_dirty(mirrors);
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        if step.gap() {
            self.whole = true;
            return;
        }
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            let restated = changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status));
            let touched = restated
                || !changes.deleted().is_empty()
                || !changes.inserted().is_empty()
                || changes.columns().iter().any(|written| !written.is_empty());
            match table {
                TableId::CountryLinks | TableId::FactionMembers | TableId::Subjects if touched => self.whole = true,
                TableId::RadarCoverage => {
                    if restated {
                        self.whole = true;
                    }
                    let Some(mirror) = mirrors.get(table) else { continue };
                    let keys: Vec<u128> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).copied().collect();
                    keys.into_iter().for_each(|key| self.mark(key));
                    for written in changes.columns().iter().filter(|written| [columns.state.column, columns.radius.column].contains(&written.column())) {
                        let keys: Vec<u128> = written.slots().iter().filter_map(|&slot| mirror.key(slot)).collect();
                        keys.into_iter().for_each(|key| self.mark(key));
                    }
                }
                _ => {}
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        if self.uploaded() {
            return Ok(());
        }
        let assets = &gpu.content.assets;
        let heights = viewer_gpu::province_map::HeightField { width: assets.heightmap.width, height: assets.heightmap.height, values: assets.heightmap_texels() };
        let mut field = viewer_aircraft::radar_symbols::Field::default();
        match (self.ours, self.groups(self.ours.unwrap_or(u32::MAX))) {
            (Some(ours), Ok(groups)) => field.groups(groups, ours, heights, &gpu.content.radar_centers),
            (_, Err(why)) => *field.refused.entry(why).or_default() += 1,
            (None, Ok(_)) => {}
        }
        self.field = Some(Arc::new(field));
        self.field_dirty = false;
        Ok(())
    }

    fn uploaded(&self) -> bool {
        !self.field_dirty
    }

    fn carried(&self) -> Carried {
        Carried { entities: self.dirty.len() + usize::from(self.whole) }
    }

    fn held(&self) -> usize {
        self.groups.len() * 64 + self.links.len() * 24 + self.keys.capacity() * 16 + self.index.len() * 40 + self.dirty.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.compose(mirrors)?;
        self.ours = upstream.get::<Vision>().and_then(Vision::ours);
        Ok(())
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        let ours = upstream.get::<Vision>().and_then(Vision::ours);
        self.field_dirty |= ours != self.ours;
        self.ours = ours;
        self.settle(mirrors, settling)
    }
}
