//! Drawn-row naval titles. Loss groups use the full combat reference and
//! province; headers resolve the province with no result, including after
//! the combat ends.
use super::super::{
    LoadContext, Product, ReplyWarnings, check_reply_budget, place_names, recorded_rows, sinkings,
};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals, View},
    metadata::{check_cancel, validate_campaign},
    naval_battle_name::{Picker, battle_name_candidates},
};
use installed_content::{
    localisation::Argument, province_geometry::ProvinceGeometry, regions::StrategicRegion,
};
use replay::{
    frame::FrameDecoder,
    rows::{TableId, tables},
};
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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipBattleTitle {
    pub index: u32,
    /// Title geography and names use the entry's supplying read.
    pub hour: i32,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
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
        let geometry =
            ProvinceGeometry::read(&cat.content.layers).map_err(|error| error.to_string())?;
        let regions =
            installed_content::regions::read(&cat.content.layers).map_err(|error| error.to_string())?;
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
            .into_iter()
            .chain(states.warnings.iter().map(|note| note.message.clone()))
            .collect();
        Ok(Self {
            geometry,
            regions: regions.regions,
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

#[derive(Default)]
struct Cache {
    content: Option<CachedGeography>,
}
fn geography(source: &Source, cat: &Arc<Catalogue>) -> Result<Arc<Geography>, String> {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((from, geo)) = &cache.content
        && Arc::ptr_eq(from, cat)
    {
        return geo.clone();
    }
    let geo = Geography::read(cat).map(Arc::new);
    cache.content = Some((Arc::clone(cat), geo.clone()));
    geo
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
    let victory_points = |id| {
        points
            .get(&id)
            .copied()
            .ok_or_else(|| format!("province {id} has no recorded victory points"))
    };
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

pub(crate) fn check_content(cat: &Catalogue) -> Result<Vec<String>, String> {
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
        Self::validate(params).map_err(crate::invalid)?;
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
                .ok_or_else(|| crate::invalid("Unknown ships sunk title row"))?;
            let at = reply.rows.len();
            reply.rows.push(ShipBattleTitle {
                index,
                hour: ship.read_hour,
                name: None,
                missing: None,
            });
            // The full reference identifies the group; never use a low word
            // or guess a result by place/date. Causes have no battle.
            if ship.entry.battle != 0 && ship.entry.cause_record == 0 {
                reply.rows[at].missing = Some(
                    if ship.entry.province_present == 0 {
                        "Battle province not recorded"
                    } else {
                        "Battle title hour not recorded"
                    }
                    .into(),
                );
                if ship.entry.province_present != 0 {
                    wanted
                        .entry(ship.read_hour)
                        .or_default()
                        .entry(ship.entry.province)
                        .or_default()
                        .push(at);
                }
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
            let names = place_names::PlaceNames::load(
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
                    .and_then(|(region, picked)| {
                        let name = if let Some(province) = picked {
                            let name = names
                                .get(&(*province, hour))
                                .ok_or_else(|| "Battle-name province not resolved".to_owned())?;
                            if let Some(reason) = &name.missing {
                                return Err(reason.clone());
                            }
                            name.name
                                .clone()
                                .ok_or_else(|| "Battle-name province not recorded".to_owned())?
                        } else {
                            let key = &geo.regions[region].name_key;
                            // Copy the template's name text, including its empty default.
                            cat.localised(key, View::Plain).ok_or_else(|| {
                                format!(
                                    "Region title {key} is unavailable in the recorded language"
                                )
                            })?
                        };
                        title(&cat, &name)
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

#[cfg(test)]
mod tests;
