//! Drawn-row naval titles. Loss groups use the full combat reference and
//! province; headers resolve the province with no result, including after
//! the combat ends.
use super::super::sinkings;
use dashboard_base::{
    catalogue,
    products::{Product, ReplyWarnings, check_reply_budget, place_names},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::{Catalogue, Refusals, View},
    metadata::{check_cancel, recorded_rows, validate_campaign},
    naval_battle_name::{Picker, battle_name_candidates},
};
use installed_content_core::province_geometry::ProvinceGeometry;
use installed_content_localisation::localisation::Argument;
use installed_content_regions::regions::StrategicRegion;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct ShipsSunkBattleTitles;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShipsSunkBattleTitlesParams {
    pub branch_id: String,
    /// Numeric indices of the immutable ShipsSunk reply, only the drawn rows.
    pub rows: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipBattleTitle {
    pub index: u32,
    /// Title geography and names use the entry's supplying read.
    pub hour: i32,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkBattleTitlesReply {
    pub rows: Vec<ShipBattleTitle>,
    pub warnings: Vec<String>,
}
impl ReplyWarnings for ShipsSunkBattleTitlesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

struct Geography {
    geometry: ProvinceGeometry,
    regions: BTreeMap<u32, StrategicRegion>,
    province_regions: BTreeMap<u32, Result<u32, String>>,
    candidates: BTreeMap<u32, Vec<u32>>,
    factor: i64,
    cutoff: i64,
    warnings: Vec<String>,
}
impl Geography {
    fn read(cat: &Catalogue) -> Result<Self, String> {
        let regions = dashboard_base::content::strategic_regions(&cat.content.layers)
            .map_err(|error| error.to_string());
        Self::from_regions(cat, regions.as_deref())
    }
    fn from_regions(
        cat: &Catalogue,
        regions: Result<&installed_content_regions::regions::StrategicRegions, &String>,
    ) -> Result<Self, String> {
        let geometry =
            ProvinceGeometry::read(&cat.content.layers).map_err(|error| error.to_string())?;
        let regions = regions.map_err(Clone::clone)?;
        let states = cat.cached_states()?;
        let candidates =
            battle_name_candidates(&regions.regions, states.states.definitions(), &|province| {
                geometry.centroids.contains_key(&province)
            });
        let mut province_regions = BTreeMap::new();
        for (&id, region) in &regions.regions {
            for &province in &region.provinces {
                if province_regions
                    .insert(province, Ok(id))
                    .is_some_and(|previous| previous != Ok(id))
                {
                    province_regions.insert(
                        province,
                        Err(format!(
                            "province {province} belongs to multiple content strategic regions"
                        )),
                    );
                }
            }
        }
        let warnings = regions
            .notes
            .iter()
            .cloned()
            .chain(states.warnings.iter().map(|note| note.message.clone()))
            .collect();
        Ok(Self {
            geometry,
            regions: regions.regions.clone(),
            province_regions,
            candidates,
            factor: states
                .defines
                .fixed5("NNavy", "BATTLE_NAME_VP_FACTOR")
                .map_err(|error| error.to_string())?,
            cutoff: states
                .defines
                .fixed5("NNavy", "BATTLE_NAME_VP_CUTOFF")
                .map_err(|error| error.to_string())?,
            warnings,
        })
    }
}

type CachedGeography = (Arc<Catalogue>, Result<Arc<Geography>, String>);

/// The geography of the last catalogue read, kept apart for the recording's own catalogue
/// (which preparation configures) and the shown one (which requests read).
#[derive(Default)]
struct Cache {
    content: Option<CachedGeography>,
}
fn geography(source: &Source, cat: &Arc<Catalogue>) -> Result<Arc<Geography>, String> {
    let state = source.state::<catalogue::ByCatalogue<Cache>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let cache = state.of(cat);
    if let Some((from, geo)) = &cache.content
        && Arc::ptr_eq(from, cat)
    {
        return geo.clone();
    }
    let geo = Geography::read(cat).map(Arc::new);
    cache.content = Some((Arc::clone(cat), geo.clone()));
    geo
}

pub fn configure_geography(source: &Source, cat: &Arc<Catalogue>, content: &super::NameContent) {
    let state = source.state::<catalogue::ByCatalogue<Cache>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let cache = state.of(cat);
    if cache
        .content
        .as_ref()
        .is_some_and(|(from, _)| Arc::ptr_eq(from, cat))
    {
        return;
    }
    let geo = Geography::from_regions(cat, content.regions.as_deref()).map(Arc::new);
    cache.content = Some((Arc::clone(cat), geo));
}

// Battle title arguments receive text rather than a localisation key.
fn title(cat: &Catalogue, name: &str) -> Result<String, String> {
    let key = "NAVAL_BATTLE_TITLE";
    // Return plain localisation without an emptiness gate;
    // a missing key is copied, a defined empty value survives.
    catalogue::localised_with(
        &cat.localisation,
        key,
        &[("NAME", Argument::Text(name))],
        View::Plain,
    )
}

/// None is the native region fallback; Some is the exact scored name province.
fn pick(
    geo: &Geography,
    points: &BTreeMap<u32, i32>,
    province: u32,
) -> Result<(u32, Option<u32>), String> {
    pick_from(geo, &|id| points.get(&id).copied(), province)
}

/// [`pick`] through a lookup of each province's recorded victory points.
fn pick_from(
    geo: &Geography,
    points: &dyn Fn(u32) -> Option<i32>,
    province: u32,
) -> Result<(u32, Option<u32>), String> {
    if !geo.geometry.centroids.contains_key(&province) {
        return Err(format!("province {province} has no content map definition"));
    }
    let region = *geo
        .province_regions
        .get(&province)
        .ok_or_else(|| format!("province {province} has no content strategic region"))?
        .as_ref()
        .map_err(Clone::clone)?;
    let position = |id| geo.geometry.centroids.get(&id).copied().unwrap_or([0, 0]);
    let victory_points =
        |id| points(id).ok_or_else(|| format!("province {id} has no recorded victory points"));
    let picker = Picker {
        width: geo.geometry.width,
        factor: geo.factor,
        cutoff: geo.cutoff,
        position: &position,
        victory_points: &victory_points,
    };
    picker
        .pick(
            province,
            geo.candidates.get(&region).map_or(&[], Vec::as_slice),
        )
        .map(|picked| (region, picked))
}

pub fn check_content(cat: &Catalogue) -> Result<Vec<String>, String> {
    let geo = Geography::read(cat)?;
    // State and region reader notes belong to their corpus checks.
    let mut warnings = Vec::new();
    let mut refused = Refusals::default();
    for region in geo.regions.values() {
        let name = catalogue::localised(&cat.localisation, &region.name_key, View::Plain);
        match name.and_then(|name| title(cat, &name)) {
            Ok(_) => (),
            Err(reason) => refused.note(reason, format!("strategic region {}", region.id)),
        }
    }
    warnings.extend(refused.lines("Ships sunk battle titles"));
    Ok(warnings)
}

fn initial(index: u32, ship: &sinkings::ShipEvent) -> ShipBattleTitle {
    let missing = (ship.entry.battle != 0 && ship.entry.cause_record == 0).then(|| {
        if ship.entry.province_present == 0 {
            "Battle province not recorded"
        } else {
            "Battle title hour not recorded"
        }
        .to_owned()
    });
    ShipBattleTitle {
        index,
        hour: ship.read_hour,
        name: None,
        missing,
    }
}
fn title_value<'a>(
    geo: &Geography,
    cat: &Catalogue,
    (region, picked): (u32, Option<u32>),
    place: impl FnOnce(u32) -> Option<&'a place_names::PlaceName>,
) -> Result<String, String> {
    let name = if let Some(province) = picked {
        let name = place(province).ok_or_else(|| "Battle-name province not resolved".to_owned())?;
        if let Some(reason) = &name.missing {
            return Err(reason.clone());
        }
        name.name
            .clone()
            .ok_or_else(|| "Battle-name province not recorded".to_owned())?
    } else {
        let key = &geo.regions[&region].name_key;
        cat.localised(key, View::Plain)
            .ok_or_else(|| format!("Region title {key} is unavailable in the recorded language"))?
    };
    title(cat, &name)
}

fn checked(mut reply: ShipsSunkBattleTitlesReply) -> io::Result<ShipsSunkBattleTitlesReply> {
    let mut refused = Refusals::default();
    for row in &reply.rows {
        if let Some(reason) = &row.missing {
            refused.note(reason, format!("ship row {}", row.index));
        }
    }
    reply
        .warnings
        .extend(refused.lines("Ships sunk battle titles"));
    check_reply_budget(&reply)?;
    Ok(reply)
}

impl Product for ShipsSunkBattleTitles {
    const KEY: &'static str = "ships-sunk-battle-titles";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&super::served::PLAN);
    type Params = ShipsSunkBattleTitlesParams;
    type Reply = ShipsSunkBattleTitlesReply;

    fn types() -> Vec<String> {
        vec![
            ShipsSunkBattleTitlesParams::decl(),
            ShipBattleTitle::decl(),
            ShipsSunkBattleTitlesReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid ships sunk battle-title branch");
        }
        if params.rows.iter().collect::<BTreeSet<_>>().len() != params.rows.len() {
            return Err("Duplicate ships sunk title rows");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        flag: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(params).map_err(dashboard_core::invalid)?;
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let scan = sinkings::cached(source, branch, flag)?;
        let mut reply = ShipsSunkBattleTitlesReply {
            rows: vec![],
            warnings: vec![],
        };
        let mut wanted = BTreeMap::<i32, BTreeMap<u32, Vec<usize>>>::new();
        for &index in &params.rows {
            check_cancel(flag)?;
            let ship = scan
                .ships
                .get(index as usize)
                .ok_or_else(|| dashboard_core::invalid("Unknown ships sunk title row"))?;
            let at = reply.rows.len();
            let row = initial(index, ship);
            let needs = row.missing.is_some();
            reply.rows.push(row);
            if needs && ship.entry.province_present != 0 {
                wanted
                    .entry(ship.read_hour)
                    .or_default()
                    .entry(ship.entry.province)
                    .or_default()
                    .push(at);
            }
        }
        if wanted.is_empty() {
            return checked(reply);
        }
        let cat = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(cat) => cat,
            Err(error) => {
                for at in wanted
                    .values()
                    .flat_map(|provinces| provinces.values())
                    .flatten()
                {
                    reply.rows[*at].missing = Some(error.to_string());
                }
                return checked(reply);
            }
        };
        let geo = match geography(source, &cat) {
            Ok(geo) => geo,
            Err(reason) => {
                for at in wanted
                    .values()
                    .flat_map(|provinces| provinces.values())
                    .flatten()
                {
                    reply.rows[*at].missing = Some(reason.clone());
                }
                return checked(reply);
            }
        };
        reply.warnings.extend(geo.warnings.iter().cloned());
        let tables = [TableId::Campaign, TableId::Provinces];
        let replay = source.replay_ref();
        let mut selected = BTreeMap::new();
        let mut failed = BTreeMap::new();
        let mut blocks = BTreeMap::<usize, BTreeSet<i32>>::new();
        for &hour in wanted.keys() {
            if let Some(block) = replay.index.iter().position(|block| {
                block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
            }) {
                blocks.entry(block).or_default().insert(hour);
            }
        }
        for (block, mut hours) in blocks {
            check_cancel(flag)?;
            let frames = match replay.read_block_selected(&replay.index[block], &tables) {
                Ok(frames) => frames,
                Err(error) => {
                    for hour in hours {
                        for &province in wanted[&hour].keys() {
                            failed.insert(
                                (hour, province),
                                format!("Battle title block unreadable: {error}"),
                            );
                        }
                    }
                    continue;
                }
            };
            let mut decoder = FrameDecoder::for_tables(&tables);
            for frame in frames {
                if hours.is_empty() {
                    break;
                }
                check_cancel(flag)?;
                let decoded = decoder.advance(&frame)?;
                validate_campaign(decoded, replay)?;
                if hours.remove(&frame.hour) {
                    let points = recorded_rows::<tables::Provinces>(decoded)?
                        .iter()
                        .map(|row| (row.id, row.victory_points))
                        .collect();
                    for &province in wanted[&frame.hour].keys() {
                        match pick(&geo, &points, province) {
                            Ok(value) => {
                                selected.insert((frame.hour, province), value);
                            }
                            Err(reason) => {
                                failed.insert((frame.hour, province), reason);
                            }
                        }
                    }
                }
            }
        }
        let requests: BTreeSet<_> = selected
            .iter()
            .filter_map(|(&(hour, _), &(_, province))| province.map(|province| (province, hour)))
            .collect();
        let names = if requests.is_empty() {
            BTreeMap::new()
        } else {
            let places = requests
                .into_iter()
                .map(|(province_id, hour)| place_names::PlaceNameRequest { province_id, hour })
                .collect();
            let names = place_names::read_recorded(
                source,
                &place_names::PlaceNamesParams {
                    land: false,
                    branch_id: params.branch_id.clone(),
                    places,
                    states: vec![],
                },
                context,
                flag,
            )?;
            reply.warnings.extend(names.warnings);
            names
                .places
                .into_iter()
                .map(|name| ((name.province_id, name.hour), name))
                .collect()
        };
        for (&hour, provinces) in &wanted {
            for (&province, rows) in provinces {
                check_cancel(flag)?;
                let value = selected
                    .get(&(hour, province))
                    .ok_or_else(|| {
                        failed
                            .get(&(hour, province))
                            .cloned()
                            .unwrap_or_else(|| "Battle title hour not recorded".into())
                    })
                    .and_then(|selection| {
                        title_value(&geo, &cat, *selection, |picked| names.get(&(picked, hour)))
                    });
                for &at in rows {
                    match &value {
                        Ok(name) => {
                            reply.rows[at].name = Some(name.clone());
                            reply.rows[at].missing = None;
                        }
                        Err(reason) => reply.rows[at].missing = Some(reason.clone()),
                    }
                }
            }
        }
        checked(reply)
    }
}

#[derive(Serialize, Deserialize)]
struct PreparedTitle {
    row: ShipBattleTitle,
    place: Option<place_names::PlaceName>,
    error: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    rows: dashboard_base::prepared::series::Accumulator,
    metadata: dashboard_base::prepared::series::Accumulator,
    published: usize,
}
#[derive(Serialize, Deserialize)]
struct TitleWarnings {
    geography: Vec<String>,
    places: Vec<String>,
}
impl Prepared {
    pub fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        ships: &[sinkings::ShipEvent],
        indices: &[u32],
        flag: &AtomicBool,
    ) -> io::Result<()> {
        self.observe_read(source, context, frame, ships, indices, flag, true)
    }

    /// [`Self::observe`] of a frame that holds the tables a place name's
    /// scripted text reads only when `places`. Without them no ship may be
    /// new there: a frame read so is one whose hour no sinking can first be
    /// read at, and a new ship stops preparation rather than be titled from
    /// a frame without the tables its place name reads.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_read(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        ships: &[sinkings::ShipEvent],
        indices: &[u32],
        flag: &AtomicBool,
        places: bool,
    ) -> io::Result<()> {
        let cat = context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
            .map_err(|error| error.to_string());
        self.observe_titled(
            source,
            cat,
            frame.hour,
            ships,
            indices,
            flag,
            |new| {
                if !places && !new.is_empty() {
                    return Err(io::Error::other(format!(
                        "A sunk ship was first read at hour {}, which was read without the tables its place name reads",
                        frame.hour
                    )));
                }
                Ok(())
            },
            |geo, wanted| {
                let points = recorded_rows::<tables::Provinces>(frame)
                    .map(|rows| {
                        rows.iter()
                            .map(|row| (row.id, row.victory_points))
                            .collect::<BTreeMap<_, _>>()
                    })
                    .map_err(|error| error.to_string());
                let lookup =
                    |id: u32| points.as_ref().ok().and_then(|points| points.get(&id).copied());
                let sighting = sight(
                    geo,
                    wanted,
                    points
                        .as_ref()
                        .map(|_| &lookup as &dyn Fn(u32) -> Option<i32>)
                        .map_err(Clone::clone),
                    |requests| {
                        place_names::at_frame_unreported(
                            source,
                            &requested(frame.branch, frame.hour, requests),
                            context,
                            frame,
                            flag,
                        )
                    },
                );
                Ok(sighting)
            },
        )
    }

    /// [`Self::observe_read`] of an hour whose title operands were read apart ([`sighting`]),
    /// naming with `cat`, the content its scan was configured with.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_sighted(
        &mut self,
        source: &Source,
        cat: Result<Arc<Catalogue>, String>,
        hour: i32,
        ships: &[sinkings::ShipEvent],
        indices: &[u32],
        flag: &AtomicBool,
        sighted: Option<(Sighting, catalogue::OwnRefusals)>,
    ) -> io::Result<()> {
        let mut sighted = sighted;
        self.observe_titled(
            source,
            cat,
            hour,
            ships,
            indices,
            flag,
            |_| Ok(()),
            |_, _| {
                let (sighting, noted) = sighted.take().ok_or_else(|| {
                    io::Error::other(format!(
                        "A sunk ship first read at hour {hour} was not titled at its read"
                    ))
                })?;
                catalogue::credit(&noted);
                Ok(sighting)
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_titled(
        &mut self,
        source: &Source,
        cat: Result<Arc<Catalogue>, String>,
        hour: i32,
        ships: &[sinkings::ShipEvent],
        indices: &[u32],
        flag: &AtomicBool,
        admit: impl FnOnce(&[(u32, &sinkings::ShipEvent)]) -> io::Result<()>,
        sight: impl FnOnce(&Geography, &[&sinkings::ShipEvent]) -> io::Result<Sighting>,
    ) -> io::Result<()> {
        check_cancel(flag)?;
        // Global immutable scan indices; the shared scan alone discovers occurrences.
        let first = indices.partition_point(|index| (*index as usize) < self.published);
        let new: Vec<_> = indices[first..]
            .iter()
            .copied()
            .zip(&ships[first..])
            .collect();
        admit(new.as_slice())?;
        let mut rows: Vec<_> = new
            .iter()
            .map(|(index, ship)| PreparedTitle {
                row: initial(*index, ship),
                place: None,
                error: None,
            })
            .collect();
        let mut notes = TitleWarnings {
            geography: Vec::new(),
            places: Vec::new(),
        };
        let wanted: Vec<_> = (0..new.len())
            .filter(|&at| rows[at].row.missing.is_some() && new[at].1.entry.province_present != 0)
            .map(|at| (at, new[at].1))
            .collect();
        if !wanted.is_empty() {
            let content = cat;
            let geo = content
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|cat| geography(source, cat));
            match (content, geo) {
                (Ok(cat), Ok(geo)) => {
                    notes.geography = geo.warnings.clone();
                    let ships: Vec<_> = wanted.iter().map(|(_, ship)| *ship).collect();
                    let sighting = sight(&*geo, ships.as_slice())?;
                    match sighting.picks {
                        Ok(picks) => {
                            let selections: BTreeMap<_, _> =
                                wanted.iter().map(|(at, _)| *at).zip(picks).collect();
                            match sighting.places {
                                Ok(mut places) => {
                                    if !sighting.requested.is_empty() {
                                        places.warnings.extend(cat.refusal_lines());
                                    }
                                    notes.places = places.warnings;
                                    for (at, selection) in selections {
                                        rows[at].place = selection
                                            .as_ref()
                                            .ok()
                                            .and_then(|(_, picked)| {
                                                picked.and_then(|province| {
                                                    places
                                                        .places
                                                        .iter()
                                                        .find(|name| name.province_id == province)
                                                })
                                            })
                                            .cloned();
                                        let value = selection.and_then(|selection| {
                                            title_value(&geo, &cat, selection, |_| {
                                                rows[at].place.as_ref()
                                            })
                                        });
                                        match value {
                                            Ok(name) => {
                                                rows[at].row.name = Some(name);
                                                rows[at].row.missing = None;
                                            }
                                            Err(reason) => rows[at].row.missing = Some(reason),
                                        }
                                    }
                                }
                                Err(error) => {
                                    for (at, selection) in selections {
                                        match selection {
                                            Ok((region, None)) => match title_value(
                                                &geo,
                                                &cat,
                                                (region, None),
                                                |_| None,
                                            ) {
                                                Ok(name) => {
                                                    rows[at].row.name = Some(name);
                                                    rows[at].row.missing = None;
                                                }
                                                Err(reason) => rows[at].row.missing = Some(reason),
                                            },
                                            Ok((_, Some(_))) => {
                                                rows[at].error = Some(error.clone())
                                            }
                                            Err(reason) => rows[at].row.missing = Some(reason),
                                        }
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            for (at, _) in &wanted {
                                rows[*at].error = Some(error.clone());
                            }
                        }
                    }
                }
                (Err(error), _) => {
                    for (at, _) in &wanted {
                        rows[*at].row.missing = Some(error.to_string());
                    }
                }
                (_, Err(reason)) => {
                    for (at, _) in &wanted {
                        rows[*at].row.missing = Some(reason.clone());
                    }
                }
            }
        }
        if !rows.is_empty() {
            self.metadata.event(0, hour, &notes)?;
        }
        for row in rows {
            self.rows.event(0, hour, &row)?;
        }
        self.rows.through(hour);
        self.metadata.through(hour);
        if let Some(index) = indices.last() {
            self.published = self.published.max(*index as usize + 1);
        }
        Ok(())
    }
    pub fn flush<W: dashboard_base::prepared::pages::Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        branch: u64,
        flag: &AtomicBool,
    ) -> io::Result<()> {
        self.rows
            .flush(writer, &format!("sinking-titles-{branch}"), 0, flag)?;
        self.metadata
            .flush(writer, &format!("sinking-title-warnings-{branch}"), 0, flag)
    }
}
/// What an hour's new ships' titles read: their picks and the picked places' names.
pub(crate) struct Sighting {
    picks: Result<Vec<Result<(u32, Option<u32>), String>>, String>,
    requested: BTreeSet<u32>,
    places: Result<place_names::PlaceNamesReply, String>,
}

/// The place names of `requests` at `hour` of `branch`, as a sunk ship's title asks them.
fn requested(branch: u64, hour: i32, requests: &BTreeSet<u32>) -> place_names::PlaceNamesParams {
    place_names::PlaceNamesParams {
        branch_id: branch.to_string(),
        land: false,
        places: requests
            .iter()
            .map(|&province_id| place_names::PlaceNameRequest { province_id, hour })
            .collect(),
        states: vec![],
    }
}

fn sight(
    geo: &Geography,
    wanted: &[&sinkings::ShipEvent],
    points: Result<&dyn Fn(u32) -> Option<i32>, String>,
    name: impl FnOnce(&BTreeSet<u32>) -> io::Result<place_names::PlaceNamesReply>,
) -> Sighting {
    let points = match points {
        Ok(points) => points,
        Err(error) => {
            return Sighting {
                picks: Err(error),
                requested: BTreeSet::new(),
                places: Ok(place_names::PlaceNamesReply {
                    places: vec![],
                    states: vec![],
                    warnings: vec![],
                }),
            };
        }
    };
    let picks: Vec<_> = wanted
        .iter()
        .map(|ship| pick_from(geo, points, ship.entry.province))
        .collect();
    let requested: BTreeSet<_> = picks
        .iter()
        .filter_map(|value| value.as_ref().ok().and_then(|(_, province)| *province))
        .collect();
    let places = if requested.is_empty() {
        Ok(place_names::PlaceNamesReply {
            places: vec![],
            states: vec![],
            warnings: vec![],
        })
    } else {
        name(&requested).map_err(|error| error.to_string())
    };
    Sighting {
        picks: Ok(picks),
        requested,
        places,
    }
}

impl Sighting {
    /// The bytes it holds on the heap, about.
    pub(crate) fn heap(&self) -> usize {
        let picks = match &self.picks {
            Ok(picks) => {
                picks.capacity() * size_of::<Result<(u32, Option<u32>), String>>()
                    + picks
                        .iter()
                        .filter_map(|pick| pick.as_ref().err())
                        .map(String::capacity)
                        .sum::<usize>()
            }
            Err(error) => error.capacity(),
        };
        let places = match &self.places {
            Ok(reply) => {
                reply.places.capacity() * size_of::<place_names::PlaceName>()
                    + reply.states.capacity() * size_of::<place_names::PlaceStateName>()
                    + reply.warnings.capacity() * size_of::<String>()
                    + reply
                        .places
                        .iter()
                        .flat_map(|place| place.name.iter().chain(&place.missing))
                        .chain(&reply.warnings)
                        .map(String::capacity)
                        .sum::<usize>()
            }
            Err(error) => error.capacity(),
        };
        picks + self.requested.len() * size_of::<u32>() + places
    }
}

/// The title operands of `wanted`, read apart; `frame` is built only when a pick names a province.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sighting(
    source: &Source,
    context: &LoadContext,
    (branch, hour): (u64, i32),
    wanted: &[&sinkings::ShipEvent],
    points: Result<&dyn Fn(u32) -> Option<i32>, String>,
    frame: impl FnOnce() -> io::Result<DecodedFrame>,
    flag: &AtomicBool,
) -> io::Result<Option<(Sighting, catalogue::OwnRefusals)>> {
    let (sighted, noted) = catalogue::capture_isolated(|| {
        let Ok(cat) = context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        else {
            return Ok(None);
        };
        let Ok(geo) = geography(source, &cat) else {
            return Ok(None);
        };
        check_cancel(flag)?;
        // A frame that cannot be built stops preparation, never titles a ship.
        let mut failed = None;
        let sighting = sight(&geo, wanted, points, |requests| {
            let frame = frame().inspect_err(|error| {
                failed = Some(io::Error::new(error.kind(), error.to_string()));
            })?;
            place_names::at_frame_unreported(
                source,
                &requested(branch, hour, requests),
                context,
                &frame,
                flag,
            )
        });
        match failed {
            Some(error) => Err(error),
            None => Ok(Some(sighting)),
        }
    });
    Ok(sighted?.map(|sighting| (sighting, noted)))
}

/// Whether a sunk ship first found needs its title's operands read at its hour.
pub(crate) fn wanted(ship: &sinkings::ShipEvent) -> bool {
    initial(0, ship).missing.is_some() && ship.entry.province_present != 0
}

pub fn prepared_query(
    params: &ShipsSunkBattleTitlesParams,
    store: &store::Store,
    key: &store::SourceKey,
    flag: &AtomicBool,
) -> io::Result<ShipsSunkBattleTitlesReply> {
    ShipsSunkBattleTitles::validate(params).map_err(dashboard_core::invalid)?;
    let branch: u64 = params.branch_id.parse().expect("validated branch");
    let mut rows = dashboard_base::prepared::series::Reader::new(
        store,
        key,
        format!("sinking-titles-{branch}"),
        flag,
    )?;
    let mut notes = dashboard_base::prepared::series::Reader::new(
        store,
        key,
        format!("sinking-title-warnings-{branch}"),
        flag,
    )?;
    let mut reply = ShipsSunkBattleTitlesReply {
        rows: vec![],
        warnings: vec![],
    };
    let mut place_names = BTreeMap::new();
    let mut geography_reads = BTreeSet::new();
    let mut place_warnings = Vec::new();
    let mut geography_written = false;
    let mut place_reads = BTreeSet::new();
    for &index in &params.rows {
        check_cancel(flag)?;
        let value = rows
            .events(&[0], i32::MIN, i32::MAX, u64::from(index), 1)?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| dashboard_core::invalid("Unknown ships sunk title row"))?;
        let title: PreparedTitle = serde_json::from_slice(&value.row)?;
        if let Some(error) = title.error {
            return Err(dashboard_core::invalid(error));
        }
        // No battle/cause needs neither geography nor place warnings.
        if title.row.name.is_some()
            || (title
                .row
                .missing
                .as_deref()
                .is_some_and(|reason| reason != "Battle province not recorded"))
        {
            geography_reads.insert(title.row.hour);
        }
        if let Some(place) = title.place {
            place_reads.insert(title.row.hour);
            place_names.insert((place.province_id, place.hour), place);
        }
        reply.rows.push(title.row);
    }
    for hour in geography_reads {
        if let Some((_, value)) = notes.latest_event(0, hour)? {
            let notes: TitleWarnings = serde_json::from_slice(&value)?;
            // The old builder reports geography once per request, across read hours.
            if !geography_written {
                reply.warnings.extend(notes.geography);
                geography_written = true;
            }
            if place_reads.contains(&hour) {
                place_warnings.extend(notes.places);
            }
        }
    }
    if !place_names.is_empty() {
        let named = place_names::checked(place_names::PlaceNamesReply {
            places: place_names.into_values().collect(),
            states: vec![],
            warnings: place_warnings,
        })?;
        reply.warnings.extend(named.warnings);
    }
    checked(reply)
}
#[cfg(test)]
mod tests;
