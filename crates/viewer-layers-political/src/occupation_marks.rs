
#[cfg(test)]
mod tests;

use viewer_layer_core::layers::{Carried, Settling, Dirty, Inputs, Layer, List, Num, View};
use super::resource_icons::{complete, next_version};
use viewer_layer_core::playhead_host::{mirror::{Mirror, Mirrors}};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, NativeValue, Opaque, OccupationMap, Recorded, TableId};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};
use viewer_map_icons::occupation_icons::{self as icons, HourMarks, Mark, Marks};

const TABLES: [TableId; 4] = [TableId::States, TableId::CountryLinks, TableId::Countries, TableId::StateResistance];

pub trait Source: Send + Sync {
    fn occupation(&self) -> Option<&icons::Content>;
}


#[derive(Clone, Copy)]
struct Columns {
    controller: Num<u32>,
    link_slot: Num<u32>,
    country_slot: Num<u32>,
    country: Num<u32>,
    level: Num<i64>,
    speed: Num<i64>,
    compliance: Num<i64>,
    map: List,
}

pub struct OccupationMarks {
    content: Arc<dyn Source>,
    columns: Option<Columns>,
    marks: BTreeMap<u32, Mark>,
    viewers: BTreeMap<u32, u32>,
    whole: Result<(), String>,
    resistance: Option<String>,
    states: Dirty,
    everything: bool,
    reviewers: bool,
    statuses: HashMap<TableId, Option<Status>>,
    hidden: Option<&'static str>,
    frame: u64,
    marked: u64,
    version: u64,
}

const NAME: &str = "occupation_marks";
const NOT_DRAWN: &str = "no occupation mark is drawn at this view";
const NO_CONTENT: &str = "the content's occupation icons are not loaded";

impl OccupationMarks {
    pub fn new(content: Arc<dyn Source>) -> Self {
        Self { content, columns: None, marks: BTreeMap::new(), viewers: BTreeMap::new(), whole: Err(String::new()), resistance: None, states: Dirty::default(), everything: true, reviewers: true, statuses: HashMap::new(), hidden: None, frame: 0, marked: 0, version: 0 }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, hidden: self.hidden, ..Self::new(Arc::clone(&self.content)) }
    }

    pub fn hidden(&self) -> Option<&'static str> {
        self.hidden
    }

    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.content));
        let kept = std::mem::replace(self, empty);
        (self.hidden, self.frame, self.marked, self.version) = (kept.hidden, kept.frame, kept.marked, next_version());
    }

    pub fn marked(&self) -> u64 {
        self.marked
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn retained_marks(&self) -> (usize, bool) {
        (self.marks.len(), !self.viewers.is_empty())
    }

    pub fn marks(&self) -> Marks {
        if let Some(why) = self.hidden {
            return Err(why.to_owned());
        }
        self.whole.clone().map(|()| HourMarks { marks: self.marks.clone(), viewers: self.viewers.clone(), resistance: self.resistance.clone() })
    }

    fn mark(&mut self, state: u32) {
        if self.states.mark(state, self.frame) {
            self.marked += 1;
        }
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        let Some(columns) = self.columns else { return };
        let content = Arc::clone(&self.content);
        let Some(occupation) = content.occupation() else { return };
        if std::mem::take(&mut self.reviewers) {
            self.version = next_version();
            self.viewers.clear();
            if let Some(countries) = mirrors.get(TableId::Countries) {
                let mut rows: Vec<(u128, u32)> = countries.rows().filter_map(|slot| Some((countries.key(slot)?, slot))).collect();
                rows.sort_unstable();
                for (key, slot) in rows {
                    if let Some(canonical) = countries.number(columns.country_slot, slot).value() {
                        self.viewers.entry(canonical).or_insert(key as u32);
                    }
                }
            }
        }
        if std::mem::take(&mut self.everything) {
            self.version = next_version();
            self.whole = TABLES[..3].iter().try_for_each(|&table| complete(mirrors, table));
            self.resistance = complete(mirrors, TableId::StateResistance).err();
            let states: Vec<u32> = self.marks.keys().copied().chain(mirrors.get(TableId::States).into_iter().flat_map(|mirror| mirror.rows().filter_map(|slot| mirror.key(slot)).map(|key| key as u32).collect::<Vec<_>>())).collect();
            for state in states {
                self.mark(state);
            }
        }
        let mut dirty = std::mem::take(&mut self.states);
        let mut changed = false;
        let _ = dirty.settle(|state| {
            let (Some(states), Some(links), Some(resistance)) = (mirrors.get(TableId::States), mirrors.get(TableId::CountryLinks), mirrors.get(TableId::StateResistance)) else { return Ok(()) };
            let Some(slot) = states.slot(u128::from(state)) else {
                changed |= self.marks.remove(&state).is_some();
                return Ok(());
            };
            let controller = states.number(columns.controller, slot).value().unwrap_or(0);
            let link = links.slot(u128::from(controller)).and_then(|at| links.number(columns.link_slot, at).value());
            let occupied = if self.resistance.is_some() { None } else { resistance.slot(u128::from(state)).map(|at| occupation_of(occupation, resistance, columns, at)) };
            let mark = Mark { controller, slot: link, resistance: occupied };
            if self.marks.get(&state) != Some(&mark) {
                self.marks.insert(state, mark);
                changed = true;
            }
            Ok(())
        });
        self.states = dirty;
        if changed {
            self.version = next_version();
        }
    }
}

pub fn drawn<'a>(side: viewer_layer_core::playhead_host::LayerParts<'a>) -> Option<&'a OccupationMarks> {
    if !side.draws(NAME) {
        return None;
    }
    let layer = side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<OccupationMarks>())?;
    layer.hidden.is_none().then_some(layer)
}

fn occupation_of(content: &icons::Content, mirror: &Mirror, columns: Columns, slot: u32) -> Result<Option<icons::Occupation>, String> {
    let number = |column: Num<i64>| mirror.number(column, slot).value().unwrap_or(0);
    let map = match mirror.list(columns.map, slot).value() {
        Some(bytes) => <Recorded<Option<OccupationMap>> as NativeValue>::from_field(Field::Opaque(Opaque::Encoded(bytes, mirror.format()))).map_err(|why| why.to_string())?,
        None => Recorded::NotRecorded,
    };
    icons::occupation(content, mirror.number(columns.country, slot).value().unwrap_or(0), number(columns.level), number(columns.speed), number(columns.compliance), &map)
}

impl Layer for OccupationMarks {
    fn name(&self) -> &'static str {
        NAME
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let mode = matches!(view.map_mode, viewer_map_modes::map_mode::MapMode::Stock(viewer_map_modes::map_mode::Stock::Resistance | viewer_map_modes::map_mode::Stock::Compliance));
        let hidden = if !mode {
            Some(NOT_DRAWN)
        } else if self.content.occupation().is_none() {
            Some(NO_CONTENT)
        } else {
            None
        };
        if hidden.is_some() {
            self.hidden = hidden;
            self.release();
            return Ok(());
        }
        self.hidden = None;
        for table in TABLES {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            controller: inputs.number(TableId::States, "controller")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            country_slot: inputs.number(TableId::Countries, "canonical_slot")?,
            country: inputs.number(TableId::StateResistance, "country")?,
            level: inputs.number(TableId::StateResistance, "level")?,
            speed: inputs.number(TableId::StateResistance, "speed")?,
            compliance: inputs.number(TableId::StateResistance, "compliance")?,
            map: inputs.list(TableId::StateResistance, "map")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.hidden.is_some() {
            self.release();
            return Ok(());
        }
        let marked = self.marked;
        self.marks.clear();
        self.states.clear();
        (self.everything, self.reviewers) = (true, true);
        self.statuses = TABLES.into_iter().map(|table| (table, mirrors.get(table).and_then(Mirror::status))).collect();
        self.settle_dirty(mirrors);
        self.marked = marked;
        self.version = next_version();
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        if step.gap() {
            self.everything = true;
            return;
        }
        for table in TABLES {
            let Some(changes) = step.table(table.index()) else { continue };
            if let Some((status, _)) = changes.status() {
                let held = self.statuses.insert(table, Some(status)).flatten();
                if held.is_none_or(|held| std::mem::discriminant(&held) != std::mem::discriminant(&status)) {
                    self.everything = true;
                }
            }
            let read: &[usize] = match table {
                TableId::States => &[columns.controller.column],
                TableId::CountryLinks => &[columns.link_slot.column],
                TableId::Countries => &[columns.country_slot.column],
                _ => &[columns.country.column, columns.level.column, columns.speed.column, columns.compliance.column, columns.map.column],
            };
            let rows_changed = !changes.deleted().slots().is_empty() || !changes.inserted().slots().is_empty();
            let written: Vec<u32> = changes.columns().iter().filter(|written| read.contains(&written.column())).flat_map(|written| written.slots().iter().copied()).collect();
            if !rows_changed && written.is_empty() {
                continue;
            }
            match table {
                TableId::CountryLinks => self.everything = true,
                TableId::Countries => self.reviewers = true,
                _ => {
                    for &key in changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()) {
                        self.mark(key as u32);
                    }
                    if let Some(mirror) = mirrors.get(table) {
                        for slot in written {
                            if let Some(key) = mirror.key(slot) {
                                self.mark(key as u32);
                            }
                        }
                    }
                }
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }


    fn carried(&self) -> Carried {
        Carried { entities: self.states.len() }
    }

    fn held(&self) -> usize {
        self.marks.len() * 96 + self.viewers.len() * 16 + self.states.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
