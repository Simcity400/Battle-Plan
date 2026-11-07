//! Names for province/hour and state/hour pairs, at each row's read hour.
//!
//! Province names try a nonempty recorded override through localisation
//! (even an empty result), then the state-owner original-or-own tag plus
//! `_VICTORY_POINTS_<id>`,
//! then `VICTORY_POINTS_<id>`. Only defined victory-point keys are used.
//! State names prefer the stored custom and owner-qualified strings, then
//! the content template. Custom strings are localised when assigned, and
//! owner-qualified strings only at an owner change or load. Those
//! strings are never reconstructed from today's owner or a different language.
//! A nonempty victory-point name is the battle name; otherwise use the state's
//! display name, without prefixing a province id or appending another name.
//! Provinces without a state use their strategic-region name;
//! region membership/names use installed_content_regions::regions.
//! Land requests use the approved primary name -> state -> unnamed ladder:
//! empty primary names try the state, and state-less provinces stay unnamed.
//! A known primary name needs no state-name lookup. Unresolved names remain null;
//! the caller shows Unnamed province and keeps the id only in its tooltip.
//! Provinces, States, Countries, CountryLinks, CountryAppearance and
//! ProvinceNames are hourly (older recordings read ProvinceNames daily and
//! carry it between reads). IDs and raw owner links are 32-bit; names retain
//! byte strings.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, recorded_rows};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals},
    metadata::{self, check_cancel, validate_campaign},
    scope,
    scripted_text::{self, StateNames},
    transport::json_len,
};
use installed_content_country_names::country_names;
use replay::campaign::Replay;
use replay_data::rows::{StateRow, TableId, tables};
use replay_frame::frame::{ColumnFrame, DecodedFrame, DecodedSource, TableView};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub mod served;

/// What a name of an hour the recording holds no frame of is missing for.
pub const NOT_RECORDED: &str = "Not in this recording: the row hour was not recorded";

/// The tables province and state names are read from.
pub const NAME_TABLES: [TableId; 8] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];
const STATE_TABLES: [TableId; 2] = [TableId::Campaign, TableId::States];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNamesParams {
    pub branch_id: String,
    /// Land's presentation ladder; other callers retain the native resolver.
    #[serde(default)]
    pub land: bool,
    pub places: Vec<PlaceNameRequest>,
    #[serde(default)]
    pub states: Vec<StateNameRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNameRequest {
    pub province_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateNameRequest {
    pub state_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct PlaceName {
    pub province_id: u32,
    pub hour: i32,
    /// Native province display name; null when no name resolves.
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct PlaceStateName {
    pub state_id: u32,
    pub hour: i32,
    /// Native state display name; null when no name resolves.
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct PlaceNamesReply {
    pub places: Vec<PlaceName>,
    pub states: Vec<PlaceStateName>,
    pub warnings: Vec<String>,
}

struct RegionNames {
    provinces: BTreeMap<u32, Result<String, String>>,
    warnings: Vec<String>,
}
/// Region names read for one catalogue.
type CachedRegionNames = (Arc<Catalogue>, Result<Arc<RegionNames>, String>);

#[derive(Default)]
struct PlaceState {
    content: Option<CachedRegionNames>,
}

fn region_names(source: &Source, cat: &Arc<Catalogue>) -> Result<Arc<RegionNames>, String> {
    let state = source.state::<PlaceState>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, names)) = &state.content
        && Arc::ptr_eq(cached, cat)
    {
        return names.clone();
    }
    let names = crate::content::strategic_regions(&cat.content.layers)
        .map(|regions| {
            let provinces = region_provinces(&regions, |region| {
                if !cat.has_presentation_key(&region.name_key) {
                    Err(format!(
                        "localisation {} is missing; region title is unavailable",
                        region.name_key
                    ))
                } else {
                    cat.localisation
                        .localise(region.name_key.as_bytes())
                        .map_err(|error| error.to_string())
                        .and_then(|name| catalogue::plain_text(&name))
                }
            });
            Arc::new(RegionNames {
                provinces,
                warnings: regions.notes.clone(),
            })
        })
        .map_err(|error| format!("Strategic-region names unavailable: {error}"));
    state.content = Some((Arc::clone(cat), names.clone()));
    names
}

/// Each province's strategic-region name (`name`), refused for a province two regions list. A
/// province one region lists more than once is in that region once, as the game builds it.
fn region_provinces(
    regions: &installed_content_regions::regions::StrategicRegions,
    name: impl Fn(&installed_content_regions::regions::StrategicRegion) -> Result<String, String>,
) -> BTreeMap<u32, Result<String, String>> {
    let mut provinces = BTreeMap::new();
    for region in regions.regions.values() {
        let named = name(region);
        for province in region.native_provinces(|_| true) {
            if provinces.insert(province, named.clone()).is_some() {
                provinces.insert(
                    province,
                    Err(format!(
                        "province {province}: multiple strategic-region memberships refused"
                    )),
                );
            }
        }
    }
    provinces
}

/// A state's recorded name inputs at an hour.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct StateInputs {
    pub custom: Vec<u8>,
    pub owner_name: Vec<u8>,
    /// Whose original-or-own tag its provinces' victory-point names are keyed by.
    pub owner: Owner,
}

/// A state owner's original-or-own tag, as victory-point names read it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Owner {
    /// The tag of the owner's canonical slot, slot zero's for no owner, or why it is unknown.
    Tag(Result<String, String>),
    /// The raw owner link resolves to no country.
    Unlinked(u32),
}

/// What place names are read from at an hour: the frame, or what preparation kept of it.
pub trait Inputs {
    fn hour(&self) -> i32;
    /// Province `id`'s state, zero for none; none without its Provinces row.
    fn province(&self, id: u32) -> Option<u32>;
    /// Province `id`'s ProvinceNames row's name.
    fn rename(&self, id: u32) -> Option<&[u8]>;
    /// Whether ProvinceNames holds every row, so a province without one has no rename.
    fn renames_complete(&self) -> bool;
    /// Why ProvinceNames may hold no row of a province.
    fn renames_gap(&self) -> &str;
    /// The capture mode that leaves States out of the recording.
    fn states_unrecorded(&self) -> Option<String>;
    /// State `id`'s inputs; none without its States row.
    fn state(&self, id: u32) -> Option<Cow<'_, StateInputs>>;
    /// Why States may hold no row of a state.
    fn states_gap(&self) -> &str;
    /// Slot zero's original-or-own tag, or why it is unknown.
    fn reserved_tag(&self) -> Result<&str, &str>;
}

/// [`Inputs`] read from a decoded frame, each table indexed at its first read.
pub struct FrameInputs<'f> {
    frame: &'f DecodedFrame,
    provinces: OnceCell<HashMap<u32, u32>>,
    renames: OnceCell<(HashMap<u32, &'f [u8]>, String)>,
    states: OnceCell<(HashMap<u32, &'f StateRow>, String)>,
    links: OnceCell<HashMap<u32, u32>>,
    tags: RefCell<HashMap<u32, Result<String, String>>>,
    reserved: OnceCell<Result<String, String>>,
}

impl<'f> FrameInputs<'f> {
    pub fn new(frame: &'f DecodedFrame) -> Self {
        Self {
            frame,
            provinces: OnceCell::new(),
            renames: OnceCell::new(),
            states: OnceCell::new(),
            links: OnceCell::new(),
            tags: RefCell::new(HashMap::new()),
            reserved: OnceCell::new(),
        }
    }

    fn renames(&self) -> &(HashMap<u32, &'f [u8]>, String) {
        self.renames.get_or_init(|| {
            let frame: &'f DecodedFrame = self.frame;
            let view: &'f TableView = &frame.tables[TableId::ProvinceNames.index()];
            let mut renames = HashMap::new();
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } = view {
                for row in rows.rows::<tables::ProvinceNames>().unwrap_or_default() {
                    renames.entry(row.province).or_insert(&row.name[..]);
                }
            }
            (renames, scope::gap(view))
        })
    }

    fn states(&self) -> &(HashMap<u32, &'f StateRow>, String) {
        self.states.get_or_init(|| {
            let frame: &'f DecodedFrame = self.frame;
            let view: &'f TableView = &frame.tables[TableId::States.index()];
            let mut states = HashMap::new();
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } = view {
                for row in rows.rows::<tables::States>().unwrap_or_default() {
                    states.entry(row.id).or_insert(row);
                }
            }
            (states, scope::gap(view))
        })
    }

    /// The canonical slot of each raw country link.
    fn links(&self) -> &HashMap<u32, u32> {
        self.links.get_or_init(|| {
            match &*self.frame.tables[TableId::CountryLinks.index()] {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows
                    .rows::<tables::CountryLinks>()
                    .unwrap_or_default()
                    .iter()
                    .map(|link| (link.id, link.canonical_slot))
                    .collect(),
                _ => HashMap::new(),
            }
        })
    }

    fn tag(&self, slot: u32) -> Result<String, String> {
        if let Some(tag) = self.tags.borrow().get(&slot) {
            return tag.clone();
        }
        let tag = country_names::original_or_own_tag(self.frame, slot).map(str::to_owned);
        self.tags.borrow_mut().insert(slot, tag.clone());
        tag
    }

    /// Every province's state, by id.
    pub fn provinces(&self) -> &HashMap<u32, u32> {
        self.provinces.get_or_init(|| {
            let mut provinces = HashMap::new();
            for row in recorded_rows::<tables::Provinces>(self.frame)
                .ok()
                .unwrap_or_default()
            {
                provinces.entry(row.id).or_insert(row.state);
            }
            provinces
        })
    }

    /// Every state's id.
    pub fn state_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.states().0.keys().copied()
    }
}

impl Inputs for FrameInputs<'_> {
    fn hour(&self) -> i32 {
        self.frame.hour
    }

    fn province(&self, id: u32) -> Option<u32> {
        self.provinces().get(&id).copied()
    }

    fn rename(&self, id: u32) -> Option<&[u8]> {
        self.renames().0.get(&id).copied()
    }

    fn renames_complete(&self) -> bool {
        let frame: &dyn DecodedSource = self.frame;
        frame.complete_status(TableId::ProvinceNames).is_ok()
    }

    fn renames_gap(&self) -> &str {
        &self.renames().1
    }

    fn states_unrecorded(&self) -> Option<String> {
        match &*self.frame.tables[TableId::States.index()] {
            TableView::NotRecorded { mode } => Some(mode.to_string()),
            _ => None,
        }
    }

    fn state(&self, id: u32) -> Option<Cow<'_, StateInputs>> {
        let row = self.states().0.get(&id)?;
        let owner = if row.owner == 0 {
            Owner::Tag(self.tag(0))
        } else {
            match self.links().get(&row.owner) {
                Some(&slot) => Owner::Tag(self.tag(slot)),
                None => Owner::Unlinked(row.owner),
            }
        };
        Some(Cow::Owned(StateInputs {
            custom: row.custom_name.clone(),
            owner_name: row.owner_name.clone(),
            owner,
        }))
    }

    fn states_gap(&self) -> &str {
        &self.states().1
    }

    fn reserved_tag(&self) -> Result<&str, &str> {
        self.reserved
            .get_or_init(|| self.tag(0))
            .as_deref()
            .map_err(String::as_str)
    }
}

/// [`Inputs`] read from a columnar walk's views of the name tables, as [`FrameInputs`] reads
/// a frame of them: each province and state read by key, only the fields names read, and each
/// original-or-own tag from the views of its country tables.
pub struct ColumnInputs<'w> {
    frame: ColumnFrame<'w>,
    tags: RefCell<HashMap<u32, Result<String, String>>>,
    reserved: OnceCell<Result<String, String>>,
    renames_gap: OnceCell<String>,
    states_gap: OnceCell<String>,
    /// The first row a read could not build; the hour refuses with it.
    failed: RefCell<Option<io::Error>>,
}

const PROVINCE_STATE: usize = replay_data::rows::column::<replay_data::rows::ProvinceRow>("state");
const RENAME: usize = replay_data::rows::column::<replay_data::rows::ProvinceNameRow>("name");
const LINK_SLOT: usize =
    replay_data::rows::column::<replay_data::rows::CountryLinkRow>("canonical_slot");
const STATE_FIELDS: u64 =
    replay_data::rows::mask::<StateRow>(&["owner", "custom_name", "owner_name"]);

impl<'w> ColumnInputs<'w> {
    /// The hour of a walk's views.
    pub fn new(frame: ColumnFrame<'w>) -> Self {
        Self {
            frame,
            tags: RefCell::new(HashMap::new()),
            reserved: OnceCell::new(),
            renames_gap: OnceCell::new(),
            states_gap: OnceCell::new(),
            failed: RefCell::new(None),
        }
    }

    /// The error a read met, if any: the hour refuses with it.
    pub fn failure(&self) -> Option<io::Error> {
        self.failed.borrow_mut().take()
    }

    fn fail<T>(&self, result: io::Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.failed.borrow_mut().get_or_insert(error);
                None
            }
        }
    }

    fn rows(&self, table: TableId) -> Option<replay_frame::frame::Columns<'w>> {
        self.frame.view(table).rows()
    }

    fn tag(&self, slot: u32) -> Result<String, String> {
        if let Some(tag) = self.tags.borrow().get(&slot) {
            return tag.clone();
        }
        // Slot zero's tag reads its reserved row alone; any other its country's rows.
        let tag = country_names::original_or_own_tag_in(&self.frame, slot).map(Cow::into_owned);
        self.tags.borrow_mut().insert(slot, tag.clone());
        tag
    }

    /// Every province's id, in key order.
    pub fn province_ids(&self) -> Vec<u32> {
        self.rows(TableId::Provinces)
            .map(|rows| rows.slots().iter().map(|&slot| rows.key(slot) as u32).collect())
            .unwrap_or_default()
    }

    /// Every state's id, in key order.
    pub fn state_ids(&self) -> Vec<u32> {
        self.rows(TableId::States)
            .map(|rows| rows.slots().iter().map(|&slot| rows.key(slot) as u32).collect())
            .unwrap_or_default()
    }
}

impl Inputs for ColumnInputs<'_> {
    fn hour(&self) -> i32 {
        self.frame.hour
    }

    fn province(&self, id: u32) -> Option<u32> {
        // As a frame reads it: no row while Provinces is not recorded.
        let rows = self.rows(TableId::Provinces)?;
        let slot = rows.find(u128::from(id))?;
        self.fail(rows.value::<u32>(slot, PROVINCE_STATE))
    }

    fn rename(&self, id: u32) -> Option<&[u8]> {
        let rows = self.rows(TableId::ProvinceNames)?;
        let slot = rows.find(u128::from(id))?;
        match self.fail(rows.bytes(slot, RENAME))? {
            Some(name) => Some(name),
            // An unrecorded name reads as the row decoder reads it.
            None => {
                self.fail(rows.value::<Vec<u8>>(slot, RENAME))?;
                Some(&[])
            }
        }
    }

    fn renames_complete(&self) -> bool {
        self.frame.view(TableId::ProvinceNames).is_complete()
    }

    fn renames_gap(&self) -> &str {
        self.renames_gap
            .get_or_init(|| self.frame.view(TableId::ProvinceNames).status().gap())
    }

    fn states_unrecorded(&self) -> Option<String> {
        match self.frame.view(TableId::States) {
            replay_frame::frame::ColumnView::NotRecorded { mode } => Some(mode.to_string()),
            _ => None,
        }
    }

    fn state(&self, id: u32) -> Option<Cow<'_, StateInputs>> {
        let rows = self.rows(TableId::States)?;
        let slot = rows.find(u128::from(id))?;
        let row = self.fail(rows.row::<StateRow>(slot, STATE_FIELDS))?;
        let owner = if row.owner == 0 {
            Owner::Tag(self.tag(0))
        } else {
            let slot = self.rows(TableId::CountryLinks).and_then(|links| {
                let at = links.find(u128::from(row.owner))?;
                self.fail(links.value::<u32>(at, LINK_SLOT))
            });
            match slot {
                Some(slot) => Owner::Tag(self.tag(slot)),
                None => Owner::Unlinked(row.owner),
            }
        };
        Some(Cow::Owned(StateInputs {
            custom: row.custom_name,
            owner_name: row.owner_name,
            owner,
        }))
    }

    fn states_gap(&self) -> &str {
        self.states_gap
            .get_or_init(|| self.frame.view(TableId::States).status().gap())
    }

    fn reserved_tag(&self) -> Result<&str, &str> {
        self.reserved
            .get_or_init(|| self.tag(0))
            .as_deref()
            .map_err(String::as_str)
    }
}

/// Province `id`'s rename as `GetName` reads it: unknown while ProvinceNames is incomplete.
fn rename(inputs: &impl Inputs, id: u32) -> Result<Option<&[u8]>, String> {
    match inputs.rename(id) {
        Some(name) => Ok(Some(name)),
        None if inputs.renames_complete() => Ok(None),
        None => Err(format!(
            "GetName of province {id}: ProvinceNames {}",
            inputs.renames_gap()
        )),
    }
}

/// The original-or-own tag province `id`'s victory-point name is keyed by: its state owner's,
/// not its controller's; a null state uses the country at slot zero.
fn victory_point_tag(inputs: &impl Inputs, id: u32, state: u32) -> Result<String, String> {
    if state == 0 {
        return inputs
            .reserved_tag()
            .map(str::to_owned)
            .map_err(str::to_owned);
    }
    let held = inputs.state(state).ok_or_else(|| {
        format!(
            "province {id}: state {state} has no States row: {}",
            inputs.states_gap()
        )
    })?;
    match &held.owner {
        Owner::Tag(tag) => tag.clone(),
        Owner::Unlinked(link) => Err(format!(
            "province {id}: state owner's link {link} resolves to no country"
        )),
    }
}

/// State `id`'s display name as places show it.
fn state_display(
    inputs: &impl Inputs,
    id: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    missing: impl FnOnce() -> String,
) -> Result<String, String> {
    if let Some(mode) = inputs.states_unrecorded() {
        return Err(metadata::not_recorded(inputs.hour(), TableId::States, &mode));
    }
    let held = inputs.state(id).ok_or_else(missing)?;
    let row = StateRow {
        id,
        custom_name: held.custom.clone(),
        owner_name: held.owner_name.clone(),
        ..Default::default()
    };
    scripted_text::state_display_name(&row, cat, names, scripted_text::StateNameUsage::Place)
}

/// Province `id`'s name, and why it is missing, by Land's ladder (`land`) or the native one.
pub fn place_name(
    inputs: &impl Inputs,
    id: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
    land: bool,
) -> (Option<String>, Option<String>) {
    let Some(state) = inputs.province(id) else {
        return (
            None,
            Some(format!("Not in this recording: province {id} at this hour")),
        );
    };
    let province_name = scripted_text::province_primary_name(&cat.localisation, rename(inputs, id), || {
        let tag = victory_point_tag(inputs, id, state)?;
        scripted_text::victory_point_name(&cat.localisation, id, &tag)
    })
    .and_then(|name| name.map(|name| catalogue::plain_text(&name)).transpose());
    if let Ok(Some(name)) = &province_name
        && (!land || !name.is_empty())
    {
        // Preserve successful empty localisation.
        return (Some(name.clone()), None);
    }
    if state == 0 {
        if land {
            let missing = format!("province {id}: no state name available");
            return (
                None,
                Some(match province_name {
                    Err(reason) => format!("{reason}; {missing}"),
                    Ok(_) => missing,
                }),
            );
        }
        let region = regions.map_err(str::to_owned).and_then(|regions| {
            regions
                .get(&id)
                .ok_or_else(|| format!("province {id}: no content-defined strategic region"))?
                .clone()
        });
        // Region identity is independent of an unread province override/owner.
        return match (province_name, region) {
            (Ok(_), Ok(region)) => (Some(region), None),
            (Err(reason), Ok(region)) => (Some(region), Some(reason)),
            (Ok(_), Err(reason)) => (None, Some(reason)),
            (Err(province), Err(region)) => (None, Some(format!("{province}; {region}"))),
        };
    }
    let state_name = state_display(inputs, state, cat, names, || {
        format!("state {state}: Not in this recording")
    });
    // Template text remains valid when empty; absence of
    // recorded/template inputs remains an error, distinct from that result.
    match (province_name, state_name) {
        (Ok(_), Ok(state)) => (Some(state), None),
        (Err(reason), Ok(state)) => (Some(state), Some(reason)),
        (Ok(_), Err(reason)) => (None, Some(reason)),
        (Err(province), Err(state)) => (None, Some(format!("{province}; {state}"))),
    }
}

/// State `id`'s name, and why it is missing.
pub fn state_name(
    inputs: &impl Inputs,
    id: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
) -> (Option<String>, Option<String>) {
    match state_display(inputs, id, cat, names, || {
        format!("Not in this recording: state {id} at this hour")
    }) {
        Ok(name) => (Some(name), None),
        Err(reason) => (None, Some(reason)),
    }
}

pub fn named_place(request: &PlaceNameRequest, name: (Option<String>, Option<String>)) -> PlaceName {
    let (name, missing) = name;
    PlaceName {
        province_id: request.province_id,
        hour: request.hour,
        name,
        missing,
    }
}

pub fn named_state(
    request: &StateNameRequest,
    name: (Option<String>, Option<String>),
) -> PlaceStateName {
    let (name, missing) = name;
    PlaceStateName {
        state_id: request.state_id,
        hour: request.hour,
        name,
        missing,
    }
}

enum RequestedName {
    Place(usize),
    State(usize),
}

impl RequestedName {
    fn missing<'a>(
        &self,
        places: &'a mut [PlaceName],
        states: &'a mut [PlaceStateName],
    ) -> &'a mut Option<String> {
        match self {
            Self::Place(at) => &mut places[*at].missing,
            Self::State(at) => &mut states[*at].missing,
        }
    }
}

fn read_names(
    replay: &Replay,
    branch: u64,
    params: &PlaceNamesParams,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<PlaceName>, Vec<PlaceStateName>)> {
    let mut places: Vec<_> = params
        .places
        .iter()
        .map(|request| {
            named_place(
                request,
                (
                    None,
                    Some(NOT_RECORDED.into()),
                ),
            )
        })
        .collect();
    let mut states: Vec<_> = params
        .states
        .iter()
        .map(|request| {
            named_state(
                request,
                (
                    None,
                    Some(NOT_RECORDED.into()),
                ),
            )
        })
        .collect();
    let mut blocks = BTreeMap::<usize, BTreeMap<i32, Vec<RequestedName>>>::new();
    for (hour, request) in params
        .places
        .iter()
        .enumerate()
        .map(|(at, request)| (request.hour, RequestedName::Place(at)))
        .chain(
            params
                .states
                .iter()
                .enumerate()
                .map(|(at, request)| (request.hour, RequestedName::State(at))),
        )
    {
        // An hour a recorded gap covers is unobserved, even where its frame survived.
        if replay.gaps.iter().any(|gap| {
            gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour
        }) {
            *request.missing(&mut places, &mut states) = Some(format!(
                "Not in this recording: {}",
                replay.gap_reason(branch, hour)?
            ));
            continue;
        }
        if let Some(block) = replay.index.iter().position(|block| {
            block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
        }) {
            blocks
                .entry(block)
                .or_default()
                .entry(hour)
                .or_default()
                .push(request);
        }
    }
    let tables: &[TableId] = if params.places.is_empty() {
        &STATE_TABLES
    } else {
        &NAME_TABLES
    };
    let keys: Vec<usize> = blocks.keys().copied().collect();
        let wanted_hours = blocks.into_values();
    dashboard_core::blocks::scan_blocks(
        replay,
        keys.iter().map(|&block| &replay.index[block]),
        tables,
        cancelled,
        |reads| {
            for ((_, read), mut hours) in reads.zip(wanted_hours) {
        check_cancel(cancelled)?;
        let frames = match read {
            Ok(frames) => frames,
            Err(error) => {
                for request in hours.values().flatten() {
                    *request.missing(&mut places, &mut states) = Some(format!(
                        "Not in this recording: unreadable names block: {error}"
                    ));
                }
                continue;
            }
        };
        for frame in frames {
            if hours.is_empty() {
                break;
            }
            check_cancel(cancelled)?;
            let decoded = match Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(&frame.decoded) {
                Ok(decoded) => decoded,
                Err(error) => {
                    for request in hours.values().flatten() {
                        *request.missing(&mut places, &mut states) = Some(format!(
                            "Not in this recording: invalid names baseline: {error}"
                        ));
                    }
                    break;
                }
            };
            validate_campaign(decoded, replay)?;
            if let Some(requested) = hours.remove(&frame.hour) {
                let inputs = FrameInputs::new(decoded);
                for request in requested {
                    match request {
                        RequestedName::Place(at) => {
                            places[at] = named_place(
                                &params.places[at],
                                place_name(
                                    &inputs,
                                    params.places[at].province_id,
                                    cat,
                                    names,
                                    regions,
                                    params.land,
                                ),
                            );
                        }
                        RequestedName::State(at) => {
                            states[at] = named_state(
                                &params.states[at],
                                state_name(&inputs, params.states[at].state_id, cat, names),
                            );
                        }
                    }
                }
            }
        }
            }
            Ok(())
        },
    )?;
    Ok((places, states))
}

/// The product's request and reply types.
pub fn types() -> Vec<String> {
    vec![
        PlaceNamesParams::decl(),
        PlaceNameRequest::decl(),
        StateNameRequest::decl(),
        PlaceName::decl(),
        PlaceStateName::decl(),
        PlaceNamesReply::decl(),
    ]
}

pub fn validate(params: &PlaceNamesParams) -> Result<(), &'static str> {
    if params.branch_id.parse::<u64>().is_err() {
        return Err("Invalid place names branch");
    }
    Ok(())
}

/// The reply read from the recording's row frames, for the live readers that still read them.
pub fn read_recorded(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<PlaceNamesReply> {
    validate(params).map_err(crate::invalid)?;
    check_cancel(cancelled)?;
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| crate::invalid("Invalid place names branch"))?;
    let replay = source.replay_ref();
    if !replay.branches.iter().any(|entry| entry.id == branch) {
        return Err(crate::invalid("Unknown place names branch"));
    }
    checked(resolve(
        source,
        params,
        context,
        true,
        |cat, names, regions| read_names(replay, branch, params, cat, names, regions, cancelled),
    )?)
}

/// The reply `read` resolves, with the catalogue's refusal lines when `reported`.
pub fn resolve(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    reported: bool,
    read: impl FnOnce(
        &Catalogue,
        Result<&StateNames, &str>,
        Result<&BTreeMap<u32, Result<String, String>>, &str>,
    ) -> io::Result<(Vec<PlaceName>, Vec<PlaceStateName>)>,
) -> io::Result<PlaceNamesReply> {
    let cat = context
        .content
        .as_ref()
        .ok_or_else(|| "Place names need the recorded content stack and language".to_owned())
        .and_then(|options| {
            catalogue::catalogue(source, options).map_err(|error| error.to_string())
        });
    let mut warnings = Vec::new();
    let (places, states) = match cat {
        Ok(cat) => {
            warnings.extend(cat.verification.iter().chain(&cat.warnings).cloned());
            let names = cat.cached_states().map(|states| &states.names);
            let regions = if params.places.is_empty() || params.land {
                Ok(Arc::new(RegionNames {
                    provinces: BTreeMap::new(),
                    warnings: Vec::new(),
                }))
            } else {
                region_names(source, &cat)
            };
            match &names {
                Ok(_) => {}
                Err(reason) => warnings.push(reason.clone()),
            }
            match &regions {
                Ok(regions) => warnings.extend(regions.warnings.iter().cloned()),
                Err(reason) => warnings.push(reason.clone()),
            }
            let resolved = read(
                &cat,
                names.as_ref().copied().map_err(String::as_str),
                regions
                    .as_ref()
                    .map(|regions| &regions.provinces)
                    .map_err(String::as_str),
            )?;
            if reported {
                warnings.extend(cat.refusal_lines());
            }
            resolved
        }
        Err(reason) => {
            warnings.push(reason.clone());
            (
                params
                    .places
                    .iter()
                    .map(|request| named_place(request, (None, Some(reason.clone()))))
                    .collect(),
                params
                    .states
                    .iter()
                    .map(|request| named_state(request, (None, Some(reason.clone()))))
                    .collect(),
            )
        }
    };
    Ok(PlaceNamesReply {
        places,
        states,
        warnings,
    })
}

/// Preparation passes the accepted decoded frame to the existing name ladder.
/// Refusal aggregation remains at the reply boundary so a drawn subset gets its own counts.
pub fn at_frame(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    frame: &DecodedFrame,
    flag: &AtomicBool,
) -> io::Result<PlaceNamesReply> {
    named_at_frame(source, params, context, &FrameInputs::new(frame), flag, true)
}

/// [`at_frame`] without the refusal lines, for a caller naming apart from its consumer scope:
/// of any frame's tables, a walk's views among them.
pub fn at_frame_unreported(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    frame: replay_frame::frame::HourFrame<'_>,
    flag: &AtomicBool,
) -> io::Result<PlaceNamesReply> {
    match frame {
        replay_frame::frame::HourFrame::Decoded(decoded) => named_at_frame(
            source,
            params,
            context,
            &FrameInputs::new(decoded.decoded_frame()),
            flag,
            false,
        ),
        replay_frame::frame::HourFrame::Walked(walked) => {
            let inputs = ColumnInputs::new(walked);
            let reply = named_at_frame(source, params, context, &inputs, flag, false)?;
            match inputs.failure() {
                Some(error) => Err(error),
                None => Ok(reply),
            }
        }
    }
}

fn named_at_frame(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    inputs: &impl Inputs,
    flag: &AtomicBool,
    reported: bool,
) -> io::Result<PlaceNamesReply> {
    check_cancel(flag)?;
    resolve(source, params, context, reported, |cat, names, regions| {
        let places = params
            .places
            .iter()
            .map(|request| {
                named_place(
                    request,
                    place_name(inputs, request.province_id, cat, names, regions, params.land),
                )
            })
            .collect();
        let states = params
            .states
            .iter()
            .map(|request| named_state(request, state_name(inputs, request.state_id, cat, names)))
            .collect();
        Ok((places, states))
    })
}

pub fn checked(reply: PlaceNamesReply) -> io::Result<PlaceNamesReply> {
    let PlaceNamesReply {
        places,
        states,
        mut warnings,
    } = reply;
    let mut refused = Refusals::default();
    for place in &places {
        if let Some(reason) = &place.missing {
            refused.note(
                reason,
                format!("province {} at {}", place.province_id, place.hour),
            );
        }
    }
    for state in &states {
        if let Some(reason) = &state.missing {
            refused.note(
                reason,
                format!("state {} at {}", state.state_id, state.hour),
            );
        }
    }
    warnings.extend(refused.lines("place names"));
    warnings.sort();
    warnings.dedup();
    let reply = PlaceNamesReply {
        places,
        states,
        warnings,
    };
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES)
        .ok_or_else(|| crate::invalid("The Place names reply exceeds its size budget"))?;
    Ok(reply)
}

#[cfg(test)]
mod tests;
