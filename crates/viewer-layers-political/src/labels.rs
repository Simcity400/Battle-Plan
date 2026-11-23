
use viewer_layer_core::layers::{Carried, Settling, Dirty, Inputs, Layer, List, Num, Text, Upstream, View};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use content_reader_localisation::localisation::Localisation;
use map_display_data::landscape::{LabelCandidate, LabelName, Labels as Display};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{
    Hour,
    country_flags::original_tag_of,
    rows::{Field, NativeValue, Opaque, Recorded, TableId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use viewer_foundation::allegiance::{Allegiance, ongoing};
use viewer_runtime::label_reads::{label_name, overrides_ladder, unreadable};

#[derive(Clone, Copy)]
struct Columns {
    state: Num<u32>,
    controller: Num<u32>,
    points: Num<i32>,
    link_slot: Num<u32>,
    country_slot: Num<u32>,
    tag: Text,
    state_owner: Num<u32>,
    state_controller: Num<u32>,
    capital: Num<i32>,
    original_tag: List,
    reference: Num<i32>,
    override_name: Text,
    war_start: Num<i32>,
    war_end: Num<i32>,
}

const CORE: [TableId; 3] = [TableId::Provinces, TableId::CountryLinks, TableId::Countries];
const OTHERS: [TableId; 5] = [TableId::States, TableId::Politics, TableId::CountryAppearance, TableId::ProvinceNames, TableId::WarRelations];

fn class(status: Status) -> u8 {
    match status {
        Status::Complete => 0,
        Status::Truncated { .. } => 1,
        _ => 2,
    }
}

fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn gap(mirror: Option<&Mirror>) -> String {
    match mirror.and_then(Mirror::status) {
        Some(Status::Complete) => "the complete table has no row for it".into(),
        Some(Status::Truncated { missing }) => format!("the table is truncated ({missing} rows missing)"),
        _ => mirror.and_then(Mirror::not_recorded).map_or_else(|| "the table is not read".into(), |why| format!("the table is unavailable: {why}")),
    }
}

pub struct Labels {
    localisation: Arc<Localisation>,
    columns: Option<Columns>,
    ours: Option<u32>,
    reads_vision: bool,
    shown: bool,
    perspective: Option<String>,
    refused: Option<String>,
    links: HashMap<u32, u32>,
    raws_of_slot: HashMap<u32, BTreeSet<u32>>,
    natives: BTreeSet<(u32, u32)>,
    native_slot: HashMap<u32, u32>,
    state_provinces: HashMap<u32, BTreeSet<u32>>,
    province_state: HashMap<u32, u32>,
    capital_of_slot: HashMap<u32, u32>,
    stars: HashMap<u32, u32>,
    withheld: BTreeMap<u32, &'static str>,
    unreadable: BTreeSet<u32>,
    renamed: BTreeSet<u32>,
    notes: Vec<String>,
    notes_stale: bool,
    capitals: BTreeSet<u32>,
    candidates: BTreeMap<u32, LabelCandidate>,
    controller_of: HashMap<u32, u32>,
    allegiances: BTreeMap<u32, u32>,
    next_war_end: Option<Hour>,
    original_tags: BTreeMap<u32, Result<String, String>>,
    link_raws: BTreeSet<u32>,
    country_natives: BTreeSet<u32>,
    tag_slots: BTreeSet<u32>,
    capital_slots: BTreeSet<u32>,
    star_states: BTreeSet<u32>,
    pointed: BTreeSet<u32>,
    provinces: Dirty,
    moved: Dirty,
    renamed_states: Dirty,
    every: bool,
    refusals_dirty: bool,
    wars_dirty: bool,
    statuses: HashMap<usize, Status>,
    hour: Hour,
    frame: u64,
    marked: [usize; 3],
    drawn: Arc<Display>,
    drawn_stale: bool,
    drawn_hours: Option<(Hour, Hour)>,
}

impl Labels {
    pub fn new(localisation: Arc<Localisation>, ours: Option<u32>) -> Self {
        Self {
            localisation,
            columns: None,
            ours,
            reads_vision: false,
            shown: true,
            perspective: None,
            refused: Some("not composed yet".into()),
            links: HashMap::new(),
            raws_of_slot: HashMap::new(),
            natives: BTreeSet::new(),
            native_slot: HashMap::new(),
            state_provinces: HashMap::new(),
            province_state: HashMap::new(),
            capital_of_slot: HashMap::new(),
            stars: HashMap::new(),
            withheld: BTreeMap::new(),
            unreadable: BTreeSet::new(),
            renamed: BTreeSet::new(),
            notes: Vec::new(),
            notes_stale: true,
            capitals: BTreeSet::new(),
            candidates: BTreeMap::new(),
            controller_of: HashMap::new(),
            allegiances: BTreeMap::new(),
            next_war_end: None,
            original_tags: BTreeMap::new(),
            link_raws: BTreeSet::new(),
            country_natives: BTreeSet::new(),
            tag_slots: BTreeSet::new(),
            capital_slots: BTreeSet::new(),
            star_states: BTreeSet::new(),
            pointed: BTreeSet::new(),
            provinces: Dirty::default(),
            moved: Dirty::default(),
            renamed_states: Dirty::default(),
            every: false,
            refusals_dirty: false,
            wars_dirty: false,
            statuses: HashMap::new(),
            hour: 0,
            frame: 0,
            marked: [0; 3],
            drawn: Arc::default(),
            drawn_stale: true,
            drawn_hours: None,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, shown: self.shown, reads_vision: self.reads_vision, ..Self::new(Arc::clone(&self.localisation), self.ours) }
    }

    pub fn drawn_hours(&self) -> Option<std::ops::RangeInclusive<Hour>> {
        self.drawn_hours.map(|(from, to)| from..=to)
    }

    pub fn reading_vision(self) -> Self {
        Self { reads_vision: true, ..self }
    }

    fn take_ours(&mut self, upstream: &Upstream<'_>) -> bool {
        let Some(vision) = upstream.get::<viewer_layer_core::vision::Vision>() else { return false };
        let ours = vision.ours();
        let changed = self.ours != ours;
        self.ours = ours;
        changed
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn ours(&self) -> Option<u32> {
        self.ours
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn marked(&self) -> [usize; 3] {
        self.marked
    }

    pub fn notes(&self) -> &[String] {
        if !self.shown || self.refused.is_some() { &[] } else { &self.notes }
    }

    fn read_name(&mut self, mirrors: &Mirrors, id: u32) {
        let columns = self.columns();
        let bytes = rows(mirrors.get(TableId::ProvinceNames)).and_then(|names| {
            let row = names.slot(u128::from(id))?;
            match names.text(columns.override_name, row) {
                Cell::Value(text) => names.string(text),
                _ => None,
            }
        });
        let changed = if bytes.is_some_and(unreadable) { self.unreadable.insert(id) } else { self.unreadable.remove(&id) };
        self.notes_stale |= changed;
    }

    fn make_notes(&mut self) {
        let unreadable: Vec<u32> = self.unreadable.iter().copied().collect();
        let mut out = viewer_foundation::tables::Limitations::default();
        map_products::note_unreadable_names(&mut out, &unreadable);
        viewer_map_icons::labels::note_withheld(&mut out, self.withheld.iter().map(|(&state, &why)| (state, why)));
        self.notes = out.into_lines();
        self.notes_stale = false;
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the labels layer is declared before it composes")
    }

    pub fn display(&self) -> Display {
        Display {
            perspective: self.perspective.clone(),
            candidates: Arc::new(self.candidates.values().cloned().collect()),
            allegiances: self.allegiances.clone(),
        }
    }

    fn by_slot(&self, slot: u32) -> Option<u32> {
        self.natives.range((slot, 0)..=(slot, u32::MAX)).next_back().map(|&(_, native)| native)
    }

    fn tag_bytes<'m>(&self, mirrors: &'m Mirrors, native: u32) -> Option<&'m [u8]> {
        let countries = mirrors.get(TableId::Countries)?;
        let row = countries.slot(u128::from(native))?;
        match countries.text(self.columns().tag, row) {
            Cell::Value(id) => countries.string(id),
            _ => Some(&[]),
        }
    }

    fn tag_of(&self, mirrors: &Mirrors, slot: u32) -> Option<String> {
        self.tag_bytes(mirrors, self.by_slot(slot)?).map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }

    fn core(mirrors: &Mirrors) -> Option<String> {
        for table in CORE {
            let Some(mirror) = mirrors.get(table) else { return Some(format!("{table:?} is not read yet")) };
            if mirror.status() != Some(Status::Complete) {
                let why = mirror.not_recorded().unwrap_or_else(|| format!("{table:?} is not wholly recorded at this hour"));
                return Some(format!("{why}; the core tables Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn"));
            }
        }
        None
    }

    fn restart(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.statuses = OTHERS.iter().filter_map(|&table| mirrors.get(table).and_then(Mirror::status).map(|status| (table.index(), status))).collect();
        for set in [&mut self.link_raws, &mut self.country_natives, &mut self.tag_slots, &mut self.capital_slots, &mut self.star_states, &mut self.pointed] {
            set.clear();
        }
        self.links.clear();
        self.raws_of_slot.clear();
        if let Some(links) = mirrors.get(TableId::CountryLinks) {
            for slot in links.rows() {
                if let (Some(key), Some(canonical)) = (links.key(slot), links.number(columns.link_slot, slot).value()) {
                    self.links.insert(key as u32, canonical);
                    self.raws_of_slot.entry(canonical).or_default().insert(key as u32);
                }
            }
        }
        self.natives.clear();
        self.native_slot.clear();
        if let Some(countries) = mirrors.get(TableId::Countries) {
            for slot in countries.rows() {
                if let (Some(key), Some(canonical)) = (countries.key(slot), countries.number(columns.country_slot, slot).value()) {
                    self.natives.insert((canonical, key as u32));
                    self.native_slot.insert(key as u32, canonical);
                }
            }
        }
        self.state_provinces.clear();
        self.province_state.clear();
        self.drawn_stale = true;
        self.candidates.clear();
        self.controller_of.clear();
        self.provinces.clear();
        self.moved.clear();
        self.renamed_states.clear();
        if let Some(provinces) = mirrors.get(TableId::Provinces) {
            for slot in provinces.rows() {
                let (Some(key), Some(state)) = (provinces.key(slot), provinces.number(columns.state, slot).value()) else { continue };
                let id = key as u32;
                self.province_state.insert(id, state);
                self.state_provinces.entry(state).or_default().insert(id);
                self.provinces.mark(id, self.frame);
            }
        }
        let slots: Vec<u32> = self.raws_of_slot.keys().copied().collect();
        self.original_tags.clear();
        self.capital_of_slot.clear();
        for &slot in &slots {
            if let Some(tag) = self.original_tag(mirrors, slot) {
                self.original_tags.insert(slot, tag);
            }
            if let Some(state) = self.capital_of(mirrors, slot) {
                self.capital_of_slot.insert(slot, state);
            }
        }
        self.original_tags.insert(0, Err("country slot zero's reserved original tag is not read by the labels layer".into()));
        self.stars.clear();
        self.withheld.clear();
        if let Some(states) = rows(mirrors.get(TableId::States)) {
            let ids: Vec<u32> = states.rows().filter_map(|row| states.key(row)).map(|key| key as u32).collect();
            for state in ids {
                match self.star_of(mirrors, state) {
                    Some(Ok(province)) => {
                        self.stars.insert(state, province);
                    }
                    Some(Err(why)) => {
                        self.withheld.insert(state, why);
                    }
                    None => {}
                }
            }
        }
        self.capitals = self.stars.values().copied().collect();
        self.unreadable.clear();
        self.renamed.clear();
        let named: Vec<u32> = rows(mirrors.get(TableId::ProvinceNames)).map(|names| names.rows().filter_map(|row| names.key(row)).map(|key| key as u32).collect()).unwrap_or_default();
        for id in named {
            self.read_name(mirrors, id);
        }
        self.notes_stale = true;
        self.read_wars(mirrors);
        self.every = false;
        self.refusals_dirty = false;
        self.wars_dirty = false;
    }

    fn original_tag(&self, mirrors: &Mirrors, slot: u32) -> Option<Result<String, String>> {
        let columns = self.columns();
        let native = self.by_slot(slot)?;
        let appearance = rows(mirrors.get(columns.original_tag.table))?;
        let row = self.raws_of_slot.get(&slot)?.iter().rev().find_map(|&raw| appearance.slot(u128::from(raw)))?;
        Some((|| {
            let reference = match appearance.number(columns.reference, row) {
                Cell::Value(reference) => Recorded::Value(Some(reference)),
                Cell::Missing => Recorded::Value(None),
                Cell::Unrecorded => Recorded::NotRecorded,
            };
            let original: Recorded<Option<Vec<u8>>> = match (&reference, appearance.list(columns.original_tag, row)) {
                (Recorded::Value(Some(reference)), Cell::Value(bytes)) if *reference > 0 => NativeValue::from_field(Field::Opaque(Opaque::Encoded(bytes, appearance.format()))).map_err(|error| error.to_string())?,
                _ => Recorded::NotRecorded,
            };
            let own = self.tag_bytes(mirrors, native).ok_or("the country's tag is not named at the head")?;
            original_tag_of(&reference, &original, own).map(str::to_owned)
        })())
    }

    fn capital_of(&self, mirrors: &Mirrors, slot: u32) -> Option<u32> {
        let politics = rows(mirrors.get(TableId::Politics))?;
        let column = self.columns().capital;
        self.raws_of_slot.get(&slot)?.iter().rev().find_map(|&raw| {
            let capital = politics.number(column, politics.slot(u128::from(raw))?).value().unwrap_or(0);
            (capital > 0).then_some(capital as u32)
        })
    }

    fn star_of(&self, mirrors: &Mirrors, state: u32) -> Option<Result<u32, &'static str>> {
        let columns = self.columns();
        let states = rows(mirrors.get(TableId::States))?;
        let controller = states.number(columns.state_controller, states.slot(u128::from(state))?).value().unwrap_or(0);
        if self.links.get(&controller).and_then(|slot| self.capital_of_slot.get(slot)) != Some(&state) {
            return None;
        }
        let provinces = mirrors.get(TableId::Provinces)?;
        let (mut most, mut holders) = (0, Vec::new());
        for &province in self.state_provinces.get(&state).into_iter().flatten() {
            let points = provinces.slot(u128::from(province)).and_then(|slot| provinces.number(columns.points, slot).value()).unwrap_or(0);
            if points > most {
                most = points;
                holders.clear();
            }
            if points > 0 && points == most {
                holders.push(province);
            }
        }
        Some(match holders[..] {
            [province] => Ok(province),
            [] => Err(viewer_map_icons::labels::NO_VICTORY_POINTS),
            _ => Err(viewer_map_icons::labels::TIED_VICTORY_POINTS),
        })
    }

    fn read_wars(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.hour = mirrors.get(TableId::Provinces).and_then(Mirror::head).unwrap_or(self.hour);
        self.drawn_stale = true;
        self.allegiances.clear();
        self.next_war_end = None;
        self.perspective = self.ours.and_then(|ours| self.tag_of(mirrors, ours));
        let Some(ours) = self.ours else { return };
        self.allegiances.insert(ours, Allegiance::Ours.index());
        let Some(wars) = rows(mirrors.get(TableId::WarRelations)) else { return };
        for row in wars.rows() {
            let (Some(key), Some(start), Some(end)) = (wars.key(row), wars.number(columns.war_start, row).value(), wars.number(columns.war_end, row).value()) else { continue };
            if end > start && end > self.hour {
                self.next_war_end = Some(self.next_war_end.map_or(end, |held| held.min(end)));
            }
            if !ongoing(start, end, self.hour) {
                continue;
            }
            let (first, second) = (self.links.get(&((key >> 32) as u32)).copied(), self.links.get(&(key as u32)).copied());
            let other = match (first, second) {
                (Some(a), Some(b)) if a == ours => Some(b),
                (Some(a), Some(b)) if b == ours => Some(a),
                _ => None,
            };
            if let Some(other) = other.filter(|&slot| slot != ours) {
                self.drawn_stale = true;
                self.allegiances.insert(other, Allegiance::Enemy.index());
            }
        }
    }

    fn owner(&self, mirrors: &Mirrors, state: u32) -> Option<u32> {
        let states = rows(mirrors.get(TableId::States))?;
        Some(states.number(self.columns().state_owner, states.slot(u128::from(state))?).value().unwrap_or(0))
    }

    fn victory_point_tag(&self, mirrors: &Mirrors, state: u32) -> Result<&str, String> {
        let slot = if state == 0 {
            0
        } else {
            let owner = self.owner(mirrors, state).ok_or_else(|| format!("the owner of state {state} (States.owner) is not recorded: {}", gap(mirrors.get(TableId::States))))?;
            if owner == 0 { 0 } else { *self.links.get(&owner).ok_or_else(|| format!("the owner link {owner} of state {state} resolves to no country"))? }
        };
        self.original_tags
            .get(&slot)
            .ok_or_else(|| format!("CountryAppearance.original_tag of slot {slot} is not recorded: {}", gap(mirrors.get(TableId::CountryAppearance))))?
            .as_deref()
            .map_err(Clone::clone)
    }

    fn settle_province(&mut self, mirrors: &Mirrors, id: u32) {
        let columns = self.columns();
        let provinces = mirrors.get(TableId::Provinces).expect("the core is read");
        let row = provinces.slot(u128::from(id));
        let points = row.and_then(|row| provinces.number(columns.points, row).value()).unwrap_or(0);
        let capital = self.capitals.contains(&id);
        let Some(row) = row.filter(|_| points > 0 || capital) else {
            self.drawn_stale = true;
            self.candidates.remove(&id);
            self.controller_of.remove(&id);
            return;
        };
        let state = provinces.number(columns.state, row).value().unwrap_or(0);
        let controller = provinces.number(columns.controller, row).value().unwrap_or(0);
        let recorded = rows(mirrors.get(TableId::ProvinceNames)).and_then(|names| {
            let row = names.slot(u128::from(id))?;
            Some(match names.text(columns.override_name, row) {
                Cell::Value(text) => names.string(text).unwrap_or(&[]),
                _ => &[],
            })
        });
        let tag = if recorded.is_some_and(overrides_ladder) { Ok(None) } else { self.victory_point_tag(mirrors, state).map(Some) };
        let name = tag.and_then(|tag| label_name(&self.localisation, recorded, tag, id).map_err(|refusal| refusal.to_string()));
        self.controller_of.insert(id, controller);
        self.drawn_stale = true;
        self.candidates.insert(
            id,
            LabelCandidate {
                province: id,
                victory_points: points,
                capital,
                controller: self.links.get(&controller).copied(),
                name: match name {
                    Ok(text) => LabelName::Value { text },
                    Err(reason) => LabelName::Refused { reason },
                },
            },
        );
    }

    fn settle_moved(&mut self, mirrors: &Mirrors, id: u32) {
        let provinces = mirrors.get(TableId::Provinces).expect("the core is read");
        let state = provinces.slot(u128::from(id)).and_then(|row| provinces.number(self.columns().state, row).value());
        let held = match state {
            Some(state) => self.province_state.insert(id, state),
            None => self.province_state.remove(&id),
        };
        if held == state {
            return;
        }
        if let Some(old) = held {
            if let Some(set) = self.state_provinces.get_mut(&old) {
                set.remove(&id);
            }
            self.star_states.insert(old);
        }
        if let Some(state) = state {
            self.state_provinces.entry(state).or_default().insert(id);
            self.star_states.insert(state);
        }
    }

    fn mark_reading(&mut self, mirrors: &Mirrors, raw: Option<u32>, slot: Option<u32>) {
        let mut marked = Vec::new();
        for (&id, &controller) in &self.controller_of {
            let state = self.province_state.get(&id).copied().unwrap_or(0);
            let owner = if state == 0 { Some(0) } else { self.owner(mirrors, state) };
            let reads_raw = raw.is_some_and(|raw| controller == raw || owner == Some(raw));
            let named_by = match owner {
                Some(0) => Some(0),
                Some(owner) => self.links.get(&owner).copied(),
                None => None,
            };
            if reads_raw || (slot.is_some() && named_by == slot) {
                marked.push(id);
            }
        }
        marked.into_iter().for_each(|id| self.mark_province(id));
    }

    fn settle_link(&mut self, mirrors: &Mirrors, raw: u32) {
        let new = mirrors.get(TableId::CountryLinks).and_then(|links| links.number(self.columns().link_slot, links.slot(u128::from(raw))?).value());
        let old = match new {
            Some(slot) => self.links.insert(raw, slot),
            None => self.links.remove(&raw),
        };
        if old == new {
            return;
        }
        for (slot, linked) in [(old, false), (new, true)] {
            let Some(slot) = slot else { continue };
            let raws = self.raws_of_slot.entry(slot).or_default();
            if linked {
                raws.insert(raw);
            } else {
                raws.remove(&raw);
                if raws.is_empty() {
                    self.raws_of_slot.remove(&slot);
                }
            }
            self.tag_slots.insert(slot);
            self.capital_slots.insert(slot);
            self.star_states.extend(self.capital_of_slot.get(&slot).copied());
        }
        self.wars_dirty = true;
        self.mark_reading(mirrors, Some(raw), None);
    }

    fn settle_country(&mut self, mirrors: &Mirrors, native: u32) {
        let new = mirrors.get(TableId::Countries).and_then(|countries| countries.number(self.columns().country_slot, countries.slot(u128::from(native))?).value());
        let old = match new {
            Some(slot) => self.native_slot.insert(native, slot),
            None => self.native_slot.remove(&native),
        };
        if let Some(old) = old {
            self.natives.remove(&(old, native));
            self.tag_slots.insert(old);
        }
        if let Some(new) = new {
            self.natives.insert((new, native));
            self.tag_slots.insert(new);
        }
        self.drawn_stale = true;
        self.perspective = self.ours.and_then(|ours| self.tag_of(mirrors, ours));
    }

    fn settle_tag_slot(&mut self, mirrors: &Mirrors, slot: u32) {
        if slot == 0 {
            return;
        }
        let tag = self.original_tag(mirrors, slot);
        if self.original_tags.get(&slot) == tag.as_ref() {
            return;
        }
        match tag {
            Some(tag) => self.original_tags.insert(slot, tag),
            None => self.original_tags.remove(&slot),
        };
        self.mark_reading(mirrors, None, Some(slot));
    }

    fn settle_capital_slot(&mut self, mirrors: &Mirrors, slot: u32) {
        let new = self.capital_of(mirrors, slot);
        let old = match new {
            Some(state) => self.capital_of_slot.insert(slot, state),
            None => self.capital_of_slot.remove(&slot),
        };
        if old != new {
            self.star_states.extend(old.into_iter().chain(new));
        }
    }

    fn settle_star(&mut self, mirrors: &Mirrors, state: u32) {
        let choice = self.star_of(mirrors, state);
        let why = choice.and_then(Result::err);
        let held = match why {
            Some(why) => self.withheld.insert(state, why),
            None => self.withheld.remove(&state),
        };
        self.notes_stale |= held != why;
        let new = choice.and_then(Result::ok);
        let old = match new {
            Some(province) => self.stars.insert(state, province),
            None => self.stars.remove(&state),
        };
        if old == new {
            return;
        }
        if let Some(old) = old {
            self.capitals.remove(&old);
            self.mark_province(old);
        }
        if let Some(new) = new {
            self.capitals.insert(new);
            self.mark_province(new);
        }
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.frame += 1;
        if !self.shown {
            if !self.candidates.is_empty() || !self.allegiances.is_empty() {
                self.drawn_stale = true;
                self.candidates.clear();
                self.controller_of.clear();
                self.allegiances.clear();
                self.perspective = None;
                self.provinces.clear();
            }
            return Ok(());
        }
        match Self::core(mirrors) {
            Some(why) => {
                if self.refused.as_ref() != Some(&why) {
                    self.drawn_stale = true;
                    self.candidates.clear();
                    self.controller_of.clear();
                    self.allegiances.clear();
                    self.perspective = None;
                    self.provinces.clear();
                    self.refused = Some(why);
                }
                return Ok(());
            }
            None => {
                if self.refused.take().is_some() || self.every {
                    self.restart(mirrors);
                }
            }
        }
        let mut dirty = std::mem::take(&mut self.moved);
        dirty.settle(|id| {
            self.settle_moved(mirrors, id);
            Ok(())
        })?;
        self.moved = dirty;
        for raw in std::mem::take(&mut self.link_raws) {
            self.settle_link(mirrors, raw);
        }
        for native in std::mem::take(&mut self.country_natives) {
            self.settle_country(mirrors, native);
        }
        for province in std::mem::take(&mut self.pointed) {
            if let Some(&state) = self.province_state.get(&province) {
                self.star_states.insert(state);
            }
        }
        for slot in std::mem::take(&mut self.tag_slots) {
            self.settle_tag_slot(mirrors, slot);
        }
        for slot in std::mem::take(&mut self.capital_slots) {
            self.settle_capital_slot(mirrors, slot);
        }
        for state in std::mem::take(&mut self.star_states) {
            self.settle_star(mirrors, state);
        }
        for id in std::mem::take(&mut self.renamed) {
            self.read_name(mirrors, id);
        }
        if self.notes_stale {
            self.make_notes();
        }
        if self.refusals_dirty {
            self.refusals_dirty = false;
            let refused: Vec<u32> = self.candidates.values().filter(|candidate| matches!(candidate.name, LabelName::Refused { .. })).map(|candidate| candidate.province).collect();
            refused.into_iter().for_each(|id| self.mark_province(id));
        }
        let head = mirrors.get(TableId::Provinces).and_then(Mirror::head).unwrap_or(self.hour);
        if self.wars_dirty || self.next_war_end.is_some_and(|end| head >= end) {
            self.wars_dirty = false;
            self.marked[2] += 1;
            self.read_wars(mirrors);
        }
        self.hour = head;
        let mut dirty = std::mem::take(&mut self.renamed_states);
        dirty.settle(|state| {
            let members: Vec<u32> = self.state_provinces.get(&state).into_iter().flatten().copied().filter(|id| self.candidates.contains_key(id)).collect();
            members.into_iter().for_each(|id| {
                self.provinces.mark(id, self.frame);
            });
            Ok(())
        })?;
        self.renamed_states = dirty;
        let mut dirty = std::mem::take(&mut self.provinces);
        dirty.settle(|id| {
            self.settle_province(mirrors, id);
            Ok(())
        })?;
        self.provinces = dirty;
        Ok(())
    }

    fn mark_province(&mut self, id: u32) {
        if self.provinces.mark(id, self.frame) {
            self.marked[0] += 1;
        }
    }

    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        let mut differing = 0;
        let mut first = None;
        let mut note = |what: String| {
            differing += 1;
            first.get_or_insert(what);
        };
        if self.refused != other.refused {
            note(format!("refused: {:?} against {:?}", self.refused, other.refused));
        }
        let (a, b) = (self.display(), other.display());
        if a.perspective != b.perspective || a.allegiances != b.allegiances {
            note(format!("perspective or sides: {:?} {:?} against {:?} {:?}", a.perspective, a.allegiances, b.perspective, b.allegiances));
        }
        for id in self.candidates.keys().chain(other.candidates.keys()).collect::<BTreeSet<_>>() {
            if self.candidates.get(id) != other.candidates.get(id) {
                note(format!("candidate {id}: {:?} against {:?}", self.candidates.get(id), other.candidates.get(id)));
            }
        }
        (differing, first)
    }

    pub fn candidates(&self) -> usize {
        self.candidates.len()
    }
}

impl Layer for Labels {
    fn held_viewer(&self) -> Option<u32> {
        self.ours().filter(|_| self.shown())
    }

    fn name(&self) -> &'static str {
        "labels"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.shown = view.overlay.labels;
        if !self.shown {
            self.refused = Some("the labels are hidden".into());
            return Ok(());
        }
        if self.reads_vision {
            inputs.after("vision");
        }
        for table in CORE.into_iter().chain(OTHERS) {
            inputs.table(table, true)?;
        }
        let appearance = TableId::CountryAppearance;
        self.columns = Some(Columns {
            state: inputs.number(TableId::Provinces, "state")?,
            controller: inputs.number(TableId::Provinces, "controller")?,
            points: inputs.number(TableId::Provinces, "victory_points")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            country_slot: inputs.number(TableId::Countries, "canonical_slot")?,
            tag: inputs.text(TableId::Countries, "tag")?,
            state_owner: inputs.number(TableId::States, "owner")?,
            state_controller: inputs.number(TableId::States, "controller")?,
            capital: inputs.number(TableId::Politics, "capital")?,
            original_tag: inputs.list(appearance, "original_tag")?,
            reference: inputs.number(appearance, "original_tag_reference")?,
            override_name: inputs.text(TableId::ProvinceNames, "name")?,
            war_start: inputs.number(TableId::WarRelations, "start_hour")?,
            war_end: inputs.number(TableId::WarRelations, "end_hour")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.refused = Some("composing".into());
        self.settle_dirty(mirrors)?;
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() || !self.shown {
            return;
        }
        let columns = self.columns();
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            let Some(mirror) = mirrors.get(table) else { continue };
            if OTHERS.contains(&table)
                && let Some((status, _)) = changes.status()
                && let held = self.statuses.insert(changes.table(), status)
                && held != Some(status)
            {
                if held.map(class) != Some(class(status)) {
                    self.every = true;
                } else if matches!(table, TableId::States | TableId::CountryAppearance) {
                    self.refusals_dirty = true;
                }
            }
            let lives: Vec<u128> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).copied().collect();
            let written = |wanted: &[usize]| -> Vec<u128> { changes.columns().iter().filter(|written| wanted.contains(&written.column())).flat_map(|written| written.slots().iter().filter_map(|&slot| mirror.key(slot))).collect() };
            match table {
                TableId::Provinces => {
                    let moved = written(&[columns.state.column]);
                    let points = written(&[columns.points.column]);
                    for &key in lives.iter().chain(&moved) {
                        self.moved.mark(key as u32, self.frame);
                    }
                    self.pointed.extend(lives.iter().chain(&points).map(|&key| key as u32));
                    for key in lives.into_iter().chain(moved).chain(points).chain(written(&[columns.controller.column])) {
                        self.mark_province(key as u32);
                    }
                }
                TableId::CountryLinks => self.link_raws.extend(lives.into_iter().chain(written(&[columns.link_slot.column])).map(|key| key as u32)),
                TableId::Countries => self.country_natives.extend(lives.into_iter().chain(written(&[columns.country_slot.column, columns.tag.column])).map(|key| key as u32)),
                TableId::States => {
                    self.star_states.extend(lives.iter().chain(&written(&[columns.state_controller.column])).map(|&key| key as u32));
                    for key in lives.into_iter().chain(written(&[columns.state_owner.column])) {
                        if self.renamed_states.mark(key as u32, self.frame) {
                            self.marked[1] += 1;
                        }
                    }
                }
                TableId::Politics => {
                    let slots: Vec<u32> = lives.into_iter().chain(written(&[columns.capital.column])).filter_map(|key| self.links.get(&(key as u32)).copied()).collect();
                    self.capital_slots.extend(slots);
                }
                TableId::CountryAppearance => {
                    let slots: Vec<u32> = lives.into_iter().chain(written(&[columns.original_tag.column, columns.reference.column])).filter_map(|key| self.links.get(&(key as u32)).copied()).collect();
                    self.tag_slots.extend(slots);
                }
                TableId::ProvinceNames => {
                    for key in lives.into_iter().chain(written(&[columns.override_name.column])) {
                        self.renamed.insert(key as u32);
                        self.mark_province(key as u32);
                    }
                }
                TableId::WarRelations => {
                    if !lives.is_empty() || !written(&[columns.war_start.column, columns.war_end.column]).is_empty() {
                        self.wars_dirty = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors)
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.take_ours(upstream);
        self.compose(mirrors)
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        if self.take_ours(upstream) {
            self.wars_dirty = true;
        }
        self.settle(mirrors, settling)
    }

    fn stage(&mut self) -> io::Result<()> {
        let (settled, hour) = (self.refused.is_none() && self.carried().entities == 0, self.hour);
        if self.drawn_stale {
            self.drawn = Arc::new(self.display());
            self.drawn_stale = false;
            self.drawn_hours = settled.then_some((hour, hour));
        } else if settled {
            self.drawn_hours = Some(self.drawn_hours.map_or((hour, hour), |(from, to)| (from.min(hour), to.max(hour))));
        }
        Ok(())
    }

    fn uploaded(&self) -> bool {
        !self.drawn_stale
    }

    fn carried(&self) -> Carried {
        if self.refused.is_some() {
            return Carried::default();
        }
        let flags = [self.every, self.refusals_dirty, self.wars_dirty].into_iter().filter(|&flag| flag).count();
        let sets = self.link_raws.len() + self.country_natives.len() + self.tag_slots.len() + self.capital_slots.len() + self.star_states.len() + self.pointed.len() + self.renamed.len();
        let entities = self.provinces.len() + self.moved.len() + self.renamed_states.len() + flags + sets;
        Carried { entities }
    }

    fn held(&self) -> usize {
        let names: usize = self
            .candidates
            .values()
            .map(|candidate| match &candidate.name {
                LabelName::Value { text } => text.as_ref().map_or(0, String::capacity),
                LabelName::Refused { reason } => reason.capacity(),
            })
            .sum();
        self.candidates.len() * 96
            + names
            + (self.links.capacity() + self.province_state.capacity() + self.native_slot.capacity() + self.capital_of_slot.capacity() + self.stars.capacity() + self.controller_of.capacity()) * 12
            + self.natives.len() * 16
            + self.raws_of_slot.values().map(|set| set.len() * 8 + 48).sum::<usize>()
            + self.state_provinces.values().map(|set| set.len() * 8 + 48).sum::<usize>()
            + self.capitals.len() * 8
            + self.allegiances.len() * 16
            + self.original_tags.values().map(|tag| 48 + tag.as_ref().map_or_else(String::capacity, String::capacity)).sum::<usize>()
            + self.provinces.bytes()
            + self.moved.bytes()
            + self.renamed_states.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Clone, Debug)]
pub struct Stamped {
    pub labels: Arc<Display>,
    pub ours: u32,
    pub branch: replay_data::BranchId,
    pub hours: std::ops::RangeInclusive<Hour>,
}

pub fn stamped(side: &viewer_layer_core::playhead_host::MapSide<impl viewer_layer_core::playhead_host::PlaybackProducts>) -> Option<Stamped> {
    let (labels, ours) = drawn(side.layer_parts())?;
    let layer = side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<Labels>())?;
    Some(Stamped { labels, ours, branch: side.branch()?, hours: layer.drawn_hours()? })
}

pub fn drawn(side: viewer_layer_core::playhead_host::LayerParts<'_>) -> Option<(Arc<Display>, u32)> {
    if !side.draws("labels") {
        return None;
    }
    let layer = side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<Labels>())?;
    Some((Arc::clone(&layer.drawn), layer.ours?))
}

#[cfg(test)]
mod tests;
