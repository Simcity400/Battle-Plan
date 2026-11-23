
use dashboard_base::products::{LoadContext, Product, at_hour::{self, Read}};
use dashboard_base::{Source, catalogue::{self, Catalogue, Refusals}};
use dashboard_text::{
    metadata::{check_cancel, not_recorded},
    scope,
    scripted_text,
};
use content_reader_country_names::country_names;
use replay_data::rows::{StateRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, DecodedSource, TableView};
use serde::{Deserialize, Serialize};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

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
                let named = |region: &content_reader_regions::regions::StrategicRegion| {
                    if !cat.has_presentation_key(&region.name_key) {
                        return Err(format!("localisation {} is missing; region title is unavailable", region.name_key));
                    }
                    cat.localisation.localise(region.name_key.as_bytes()).map_err(|error| error.to_string()).and_then(|name| catalogue::plain_text(&name))
                };
                let mut by_region = BTreeMap::new();
                let mut by_province = BTreeMap::new();
                for (&id, region) in &regions.regions {
                    let name = named(region);
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
        self.provinces.get_or_init(|| self.frame.complete::<tables::Provinces>().map(|rows| rows.iter().map(|row| (row.id, row.state)).collect()).unwrap_or_default())
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

pub struct PlaceNames;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNamesParams {
    pub branch_id: String,
    #[doc = " Land's ladder: a province in no state stays unnamed rather than taking its region's name."]
    #[serde(default)]
    pub land: bool,
    pub places: Vec<PlaceNameRequest>,
    #[serde(default)]
    pub states: Vec<StateNameRequest>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNameRequest {
    pub province_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateNameRequest {
    pub state_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaceName {
    pub province_id: u32,
    pub hour: i32,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaceStateName {
    pub state_id: u32,
    pub hour: i32,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaceNamesReply {
    pub places: Vec<PlaceName>,
    pub states: Vec<PlaceStateName>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PlaceNamesReply);

fn read_hour(query: &dashboard_query::Query<'_>, frames: &mut at_hour::Frames<'_>, hour: i32, provinces: &[u32], states: &[u32]) -> Result<DecodedFrame, String> {
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

pub fn resolve_provinces(
    query: &dashboard_query::Query<'_>,
    content: &Content,
    asks: &[(u32, i32)],
    land: bool,
    cancelled: &AtomicBool,
) -> io::Result<HashMap<(u32, i32), Name>> {
    let mut hours: BTreeMap<i32, Vec<u32>> = BTreeMap::new();
    for &(province_id, hour) in asks {
        hours.entry(hour).or_default().push(province_id);
    }
    let mut places: HashMap<(u32, i32), Name> = HashMap::new();
    let mut frames = at_hour::Frames::new(query)?;
    for (&hour, provinces) in &mut hours {
        check_cancel(cancelled)?;
        provinces.sort_unstable();
        provinces.dedup();
        match read_hour(query, &mut frames, hour, provinces, &[]) {
            Ok(frame) => {
                let named = Places::new(&frame);
                for &id in provinces.iter() {
                    places.insert((id, hour), named.province(content, id, land));
                }
            }
            Err(reason) => {
                for &id in provinces.iter() {
                    places.insert((id, hour), Name::missing(reason.clone()));
                }
            }
        }
    }
    Ok(places)
}

impl Product for PlaceNames {
    const KEY: &'static str = "place-names";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = PlaceNamesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, land, "places[].hour": Read, "states[].hour": Read, "places[].provinceId", "states[].stateId"];
    type Reply = PlaceNamesReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid place names branch");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![PlaceNamesParams::decl(), PlaceNameRequest::decl(), StateNameRequest::decl(), PlaceName::decl(), PlaceStateName::decl(), PlaceNamesReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid place names branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let content = Content::read(source, load)?;
        let mut hours: BTreeMap<i32, (Vec<u32>, Vec<u32>)> = BTreeMap::new();
        for place in &params.places {
            hours.entry(place.hour).or_default().0.push(place.province_id);
        }
        for state in &params.states {
            hours.entry(state.hour).or_default().1.push(state.state_id);
        }
        let mut places: HashMap<(u32, i32), Name> = HashMap::new();
        let mut states: HashMap<(u32, i32), Name> = HashMap::new();
        let mut frames = at_hour::Frames::new(&query)?;
        for (&hour, (provinces, wanted)) in &mut hours {
            check_cancel(cancelled)?;
            provinces.sort_unstable();
            provinces.dedup();
            wanted.sort_unstable();
            wanted.dedup();
            match read_hour(&query, &mut frames, hour, provinces, wanted) {
                Ok(frame) => {
                    let named = Places::new(&frame);
                    for &id in provinces.iter() {
                        places.insert((id, hour), named.province(&content, id, params.land));
                    }
                    for &id in wanted.iter() {
                        states.insert((id, hour), named.state(&content, id));
                    }
                }
                Err(reason) => {
                    for &id in provinces.iter() {
                        places.insert((id, hour), Name::missing(reason.clone()));
                    }
                    for &id in wanted.iter() {
                        states.insert((id, hour), Name::missing(reason.clone()));
                    }
                }
            }
        }
        let mut refused = Refusals::default();
        let places: Vec<PlaceName> = params
            .places
            .iter()
            .map(|asked| {
                let name = places.get(&(asked.province_id, asked.hour)).cloned().unwrap_or_else(|| Name::missing("Not in this recording"));
                if let Some(reason) = &name.missing {
                    refused.note(reason, format!("province {} at {}", asked.province_id, asked.hour));
                }
                PlaceName { province_id: asked.province_id, hour: asked.hour, name: name.name, missing: name.missing }
            })
            .collect();
        let states: Vec<PlaceStateName> = params
            .states
            .iter()
            .map(|asked| {
                let name = states.get(&(asked.state_id, asked.hour)).cloned().unwrap_or_else(|| Name::missing("Not in this recording"));
                if let Some(reason) = &name.missing {
                    refused.note(reason, format!("state {} at {}", asked.state_id, asked.hour));
                }
                PlaceStateName { state_id: asked.state_id, hour: asked.hour, name: name.name, missing: name.missing }
            })
            .collect();
        let mut warnings = content.warnings();
        warnings.extend(content.cat.refusal_lines());
        warnings.extend(refused.lines("place names"));
        warnings.sort();
        warnings.dedup();
        let reply = PlaceNamesReply { places, states, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
