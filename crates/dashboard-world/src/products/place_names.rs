
use dashboard_base::products::{LoadContext, at_hour::{self, Read}};
use dashboard_base::{Source, catalogue::{self, Catalogue}};
use dashboard_text::{
    metadata::not_recorded,
    scope,
    scripted_text,
};
use content_reader_country_names::country_names;
use replay_data::rows::{StateRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, DecodedSource, TableView};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

#[cfg(test)]
mod tests;

pub const TABLES: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];

pub fn reads(provinces: &[u32], states: &[u32], step: usize, held: &DecodedFrame) -> Vec<Read> {
    match step {
        0 => {
            let mut reads: Vec<Read> = TABLES[..4].iter().map(|&table| Read::all(table)).collect();
            if !provinces.is_empty() {
                reads.push(Read::these(TableId::Provinces, provinces.iter().map(|&id| u128::from(id))));
                reads.push(Read::these(TableId::ProvinceNames, provinces.iter().map(|&id| u128::from(id))));
            }
            reads
        }
        1 => {
            let mut wanted: BTreeSet<u32> = states.iter().copied().collect();
            if let Ok(rows) = held.complete::<tables::Provinces>() {
                wanted.extend(rows.iter().map(|row| row.state).filter(|&state| state != 0));
            }
            if wanted.is_empty() {
                return Vec::new();
            }
            vec![Read::these(TableId::States, wanted.into_iter().map(u128::from))]
        }
        _ => Vec::new(),
    }
}

pub struct Content {
    pub cat: Arc<Catalogue>,
    regions: Result<Regions, String>,
}

struct Regions {
    by_region: BTreeMap<u32, Result<String, String>>,
    by_province: BTreeMap<u32, Result<String, String>>,
    notes: Vec<String>,
}

#[derive(Default)]
struct Cached {
    held: Option<(Arc<Catalogue>, Arc<Content>)>,
}

impl Content {
    pub fn read(source: &Source, load: &LoadContext) -> io::Result<Arc<Self>> {
        let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused("Place names need the recorded content stack and language"))?;
        let cat = catalogue::catalogue(source, options)?;
        let state = source.state::<Cached>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((held, content)) = &state.held
            && Arc::ptr_eq(held, &cat)
        {
            return Ok(Arc::clone(content));
        }
        let regions = dashboard_base::content::strategic_regions(&cat.content.layers)
            .map(|regions| {
                let mut by_region = BTreeMap::new();
                let mut by_province = BTreeMap::new();
                for (&id, region) in &regions.regions {
                    let name = crate::products::region_names::region_name(&cat, &regions, id);
                    for province in region.native_provinces(|_| true) {
                        if by_province.insert(province, name.clone()).is_some() {
                            by_province.insert(province, Err(format!("province {province}: multiple strategic-region memberships refused")));
                        }
                    }
                    by_region.insert(id, name);
                }
                Regions { by_region, by_province, notes: regions.notes.clone() }
            })
            .map_err(|error| format!("Strategic-region names unavailable: {error}"));
        let content = Arc::new(Self { cat: Arc::clone(&cat), regions });
        state.held = Some((cat, Arc::clone(&content)));
        Ok(content)
    }

    pub fn region(&self, id: u32) -> Result<String, String> {
        let regions = self.regions.as_ref().map_err(Clone::clone)?;
        regions.by_region.get(&id).cloned().unwrap_or_else(|| Err(format!("region {id}: no content-defined strategic region")))
    }

    fn province_region(&self, id: u32) -> Result<String, String> {
        let regions = self.regions.as_ref().map_err(Clone::clone)?;
        regions.by_province.get(&id).cloned().unwrap_or_else(|| Err(format!("province {id}: no content-defined strategic region")))
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = self.cat.verification.iter().chain(&self.cat.warnings).cloned().collect();
        match &self.regions {
            Ok(regions) => out.extend(regions.notes.iter().cloned()),
            Err(reason) => out.push(reason.clone()),
        }
        if let Err(reason) = self.cat.cached_states() {
            out.push(reason);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub name: Option<String>,
    pub missing: Option<String>,
}

impl Name {
    fn found(name: String) -> Self {
        Self { name: Some(name), missing: None }
    }

    fn missing(reason: impl Into<String>) -> Self {
        Self { name: None, missing: Some(reason.into()) }
    }

    fn of(name: Result<String, String>) -> Self {
        match name {
            Ok(name) => Self::found(name),
            Err(reason) => Self::missing(reason),
        }
    }
}

pub struct Places<'f> {
    frame: &'f DecodedFrame,
    provinces: OnceCell<HashMap<u32, u32>>,
    renames: OnceCell<HashMap<u32, &'f [u8]>>,
    states: OnceCell<HashMap<u32, &'f StateRow>>,
    links: OnceCell<HashMap<u32, u32>>,
    tags: RefCell<HashMap<u32, Result<String, String>>>,
}

impl<'f> Places<'f> {
    pub fn new(frame: &'f DecodedFrame) -> Self {
        Self {
            frame,
            provinces: OnceCell::new(),
            renames: OnceCell::new(),
            states: OnceCell::new(),
            links: OnceCell::new(),
            tags: RefCell::new(HashMap::new()),
        }
    }

    fn view(&self, table: TableId) -> &'f TableView {
        let frame: &'f DecodedFrame = self.frame;
        &frame.tables[table.index()]
    }

    fn provinces(&self) -> &HashMap<u32, u32> {
        self.provinces.get_or_init(|| match self.view(TableId::Provinces) {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.rows::<tables::Provinces>().unwrap_or_default().iter().map(|row| (row.id, row.state)).collect(),
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => HashMap::new(),
        })
    }

    fn renames(&self) -> &HashMap<u32, &'f [u8]> {
        self.renames.get_or_init(|| {
            let mut out = HashMap::new();
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } = self.view(TableId::ProvinceNames) {
                for row in rows.rows::<tables::ProvinceNames>().unwrap_or_default() {
                    out.entry(row.province).or_insert(&row.name[..]);
                }
            }
            out
        })
    }

    fn states(&self) -> &HashMap<u32, &'f StateRow> {
        self.states.get_or_init(|| {
            let mut out = HashMap::new();
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } = self.view(TableId::States) {
                for row in rows.rows::<tables::States>().unwrap_or_default() {
                    out.entry(row.id).or_insert(row);
                }
            }
            out
        })
    }

    fn links(&self) -> &HashMap<u32, u32> {
        self.links.get_or_init(|| self.frame.complete::<tables::CountryLinks>().map(|rows| rows.iter().map(|link| (link.id, link.canonical_slot)).collect()).unwrap_or_default())
    }

    fn tag(&self, slot: u32) -> Result<String, String> {
        if let Some(tag) = self.tags.borrow().get(&slot) {
            return tag.clone();
        }
        let tag = country_names::original_or_own_tag(self.frame, slot).map(str::to_owned);
        self.tags.borrow_mut().insert(slot, tag.clone());
        tag
    }

    pub fn province_state(&self, id: u32) -> Option<u32> {
        self.provinces().get(&id).copied()
    }

    fn rename(&self, id: u32) -> Result<Option<&'f [u8]>, String> {
        match self.renames().get(&id) {
            Some(name) => Ok(Some(name)),
            None if (self.frame as &dyn DecodedSource).complete_status(TableId::ProvinceNames).is_ok() => Ok(None),
            None => Err(format!("GetName of province {id}: province names {}", scope::gap(self.view(TableId::ProvinceNames)))),
        }
    }

    fn victory_point_tag(&self, id: u32, state: u32) -> Result<String, String> {
        if state == 0 {
            return self.tag(0);
        }
        let row = self.states().get(&state).ok_or_else(|| format!("province {id}: state {state} has no States row: {}", scope::gap(self.view(TableId::States))))?;
        if row.owner == 0 {
            return self.tag(0);
        }
        match self.links().get(&row.owner) {
            Some(&slot) => self.tag(slot),
            None => Err(format!("province {id}: state owner's link {} resolves to no country", row.owner)),
        }
    }

    fn state_display(&self, content: &Content, id: u32, absent: impl FnOnce() -> String) -> Result<String, String> {
        if let TableView::NotRecorded { mode } = self.view(TableId::States) {
            return Err(not_recorded(self.frame.hour, TableId::States, mode));
        }
        let row = self.states().get(&id).ok_or_else(absent)?;
        let states = content.cat.cached_states();
        let names = states.as_ref().map(|states| &states.names).map_err(String::as_str);
        scripted_text::state_display_name(row, &content.cat, names, scripted_text::StateNameUsage::Place)
    }

    pub fn state(&self, content: &Content, id: u32) -> Name {
        Name::of(self.state_display(content, id, || format!("Not in this recording: state {id} at this hour")))
    }

    pub fn province(&self, content: &Content, id: u32, land: bool) -> Name {
        let Some(state) = self.province_state(id) else {
            return Name::missing(format!("Not in this recording: province {id} at this hour"));
        };
        let loc = &content.cat.localisation;
        let own = scripted_text::province_primary_name(loc, self.rename(id), || scripted_text::victory_point_name(loc, id, &self.victory_point_tag(id, state)?))
            .and_then(|name| name.map(|name| catalogue::plain_text(&name)).transpose());
        if let Ok(Some(name)) = &own
            && (!land || !name.is_empty())
        {
            return Name::found(name.clone());
        }
        let fallback = if state != 0 {
            self.state_display(content, state, || format!("state {state}: Not in this recording"))
        } else if land {
            Err(format!("province {id}: no state name available"))
        } else {
            content.province_region(id)
        };
        match (own, fallback) {
            (Ok(_), Ok(name)) => Name::found(name),
            (Err(reason), Ok(name)) => Name { name: Some(name), missing: Some(reason) },
            (Ok(_), Err(reason)) => Name::missing(reason),
            (Err(own), Err(fallback)) => Name::missing(format!("{own}; {fallback}")),
        }
    }
}

pub fn read_hour(query: &dashboard_query::Query<'_>, frames: &mut at_hour::Frames<'_>, hour: i32, provinces: &[u32], states: &[u32]) -> Result<DecodedFrame, String> {
    if query.timeline.at(hour).is_none() {
        let reason = query.timeline.unheld.iter().find(|unheld| unheld.first <= hour && hour <= unheld.last).map_or("the hour was not recorded", |unheld| unheld.reason.as_str());
        return Err(format!("Not in this recording: {reason}"));
    }
    let mut frame = DecodedFrame { branch: query.timeline.id, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Default::default() };
    for step in 0.. {
        let reads = reads(provinces, states, step, &frame);
        if reads.is_empty() {
            break;
        }
        let read = frames.frame(hour, &reads, "Place names").map_err(|error| format!("Not in this recording: {error}"))?;
        for table in reads.iter().map(|read| read.table.index()) {
            frame.tables[table] = read.tables[table].clone();
        }
    }
    Ok(frame)
}
