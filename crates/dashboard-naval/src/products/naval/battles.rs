use super::{sinking_details::BattleIndex, sunk};
use dashboard_base::{Source, catalogue::Refusals, products::{LoadContext, Product}};
use dashboard_query::read;
use replay_lanes::{at::Seeker, lanes::{self, Keys}};
use replay_data::rows::{Recorded, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub mod tail;

pub fn battle_id(combat: u64, first_hour: i32) -> String {
    format!("combat:{combat}:{first_hour}")
}

pub fn parse_id(id: &str) -> Option<(u64, i32)> {
    let mut parts = id.strip_prefix("combat:")?.split(':');
    let combat = parts.next()?.parse().ok()?;
    let first = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((combat, first))
}

pub fn parse_result_id(id: &str) -> Option<u64> {
    id.strip_prefix("result:")?.parse().ok()
}

fn branch(branch_id: &str) -> io::Result<u64> {
    branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

fn valid_branch(branch_id: &str) -> Result<(), &'static str> {
    branch_id.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
}

pub struct NavalBattleDetails;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleDetailRequest {
    pub id: String,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleDetailsParams {
    pub branch_id: String,
    pub battles: Vec<NavalBattleDetailRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct NavalBattleFleetNames {
    pub names: Vec<String>,
    pub unknown: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleNamedShip {
    pub sink_hour: i32,
    pub known_at: i32,
    pub name: Option<String>,
    pub class_name: Option<String>,
    pub sinking_cause: super::sinking_details::ShipSinkingAttribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleDetail {
    pub id: String,
    pub hour: i32,
    #[doc = " The hour its fleets are read at: the hour asked."]
    pub roster_hour: Option<i32>,
    pub fleets: [NavalBattleFleetNames; 2],
    pub named_ships: Vec<NavalBattleNamedShip>,
    pub ship_read: Option<i32>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct NavalBattleDetailsReply {
    pub rows: Vec<NavalBattleDetail>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalBattleDetailsReply);

fn unknown_fleets(reason: &str) -> [NavalBattleFleetNames; 2] {
    std::array::from_fn(|_| NavalBattleFleetNames { names: Vec::new(), unknown: Some(reason.into()) })
}

fn fleets_at(seeker: &mut Seeker<'_>, run: u32, hour: i32, combats: &[u64], content: &sunk::Content, refused: &mut Refusals) -> io::Result<Vec<[NavalBattleFleetNames; 2]>> {
    let unknown = |reason: &str| combats.iter().map(|_| unknown_fleets(reason)).collect::<Vec<_>>();
    let Some(cat) = content.catalogue() else { return Ok(unknown("Fleet roster or content not recorded")) };
    let keys: Vec<u128> = combats.iter().map(|&combat| u128::from(combat)).collect::<BTreeSet<_>>().into_iter().collect();
    let mut frame_of = |table: TableId, keys: &[u128]| dashboard_query::cost::storage(|| seeker.frame(run, hour, &[read::Read { table, columns: None, keys: Keys::These(keys) }]));
    let frame = frame_of(TableId::NavalCombatDetails, &keys)?;
    let Ok(details) = frame.complete::<tables::NavalCombatDetails>() else { return Ok(unknown("Fleet identities not recorded")) };
    let detail_of: HashMap<u64, _> = details.iter().map(|row| (row.combat, row)).collect();
    let tasks: Vec<u128> = details.iter().flat_map(|row| row.sides.iter().flat_map(|side| side.units.iter().map(|&unit| u128::from(unit)))).collect::<BTreeSet<_>>().into_iter().collect();
    let frame = frame_of(TableId::TaskForces, &tasks)?;
    let Ok(forces) = frame.complete::<tables::TaskForces>() else { return Ok(unknown("Fleet identities not recorded")) };
    let fleet_of: HashMap<u64, u64> = forces.iter().map(|row| (row.id, row.fleet)).collect();
    let fleet_keys: Vec<u128> = fleet_of.values().map(|&fleet| u128::from(fleet)).collect::<BTreeSet<_>>().into_iter().collect();
    let frame = frame_of(TableId::Fleets, &fleet_keys)?;
    let Ok(fleets) = frame.complete::<tables::Fleets>() else { return Ok(unknown("Fleet identities not recorded")) };
    let names: HashMap<u64, &Recorded<Option<Vec<u8>>>> = fleets.iter().map(|row| (row.id, &row.name)).collect();
    let mut all = Vec::with_capacity(combats.len());
    for combat in combats {
        let Some(detail) = detail_of.get(combat) else {
            all.push(unknown_fleets("Fleet identities not recorded"));
            continue;
        };
        let mut out: [NavalBattleFleetNames; 2] = std::array::from_fn(|_| NavalBattleFleetNames { names: Vec::new(), unknown: None });
        for (side, input) in out.iter_mut().zip(&detail.sides) {
            let (mut seen, mut missing) = (BTreeSet::new(), 0);
            for unit in &input.units {
                let Some((fleet, name)) = fleet_of.get(unit).and_then(|fleet| Some((*fleet, *names.get(fleet)?))) else {
                    missing += 1;
                    continue;
                };
                if !seen.insert(fleet) {
                    continue;
                }
                match name {
                    Recorded::Value(Some(bytes)) => match dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, "fleet name")
                        .and_then(|text| content_reader_localisation::localisation::display_text(text.as_bytes()))
                    {
                        Ok(name) if !name.is_empty() => side.names.push(name),
                        Ok(_) => missing += 1,
                        Err(reason) => {
                            refused.note(reason, format!("fleet {fleet}"));
                            missing += 1;
                        }
                    },
                    _ => missing += 1,
                }
            }
            if missing != 0 {
                side.unknown = Some(format!("{missing} fleet names not recorded"));
            }
        }
        all.push(out);
    }
    Ok(all)
}

impl Product for NavalBattleDetails {
    const KEY: &'static str = "naval-battle-details";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalBattleDetailsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, "battles[].hour": Read];
    type Reply = NavalBattleDetailsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalBattleDetailsParams::decl(), NavalBattleDetailsReply::decl(), NavalBattleDetailRequest::decl(), NavalBattleFleetNames::decl(), NavalBattleNamedShip::decl(), NavalBattleDetail::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        valid_branch(&params.branch_id)?;
        if params.battles.iter().any(|battle| parse_id(&battle.id).is_none() && parse_result_id(&battle.id).is_none()) {
            return Err("Invalid naval battle");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        #[derive(Clone, Copy)]
        enum Target {
            Live { combat: u64, first_hour: i32 },
            Result { id: u64 },
        }

        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let content = sunk::Content::load(source, context);
        let mut refused = Refusals::default();
        let mut reply = NavalBattleDetailsReply { rows: Vec::with_capacity(params.battles.len()), warnings: content.warnings() };
        let mut at: BTreeMap<(u32, i32, i32), Vec<(usize, Target)>> = BTreeMap::new();
        for (index, asked) in params.battles.iter().enumerate() {
            let mut detail = NavalBattleDetail { id: asked.id.clone(), hour: asked.hour, roster_hour: None, fleets: unknown_fleets("Fleet roster not recorded"), named_ships: Vec::new(), ship_read: None, unavailable: None };
            match (parse_id(&asked.id), parse_result_id(&asked.id), query.timeline.at(asked.hour)) {
                (Some((combat, first_hour)), _, Some(stretch)) => {
                    detail.roster_hour = Some(asked.hour);
                    at.entry((stretch.run, stretch.first, asked.hour)).or_default().push((index, Target::Live { combat, first_hour }));
                }
                (_, Some(id), Some(stretch)) => {
                    at.entry((stretch.run, stretch.first, asked.hour)).or_default().push((index, Target::Result { id }));
                }
                _ => detail.unavailable = Some("Details are kept for battles at a recorded hour".into()),
            }
            reply.rows.push(detail);
        }
        let mut last: HashMap<(u32, i32), i32> = HashMap::new();
        for &(run, first, hour) in at.keys() {
            last.insert((run, first), hour);
        }
        let mut stretch: Option<(u32, i32)> = None;
        let (mut reads, mut lists) = (Vec::new(), HashMap::new());
        let mut seeker = Seeker::new(recording)?;
        for (&(run, first, hour), battles) in &at {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if stretch != Some((run, first)) {
                stretch = Some((run, first));
                reads = lanes::reads(recording, run, TableId::SunkShips, first, last[&(run, first)])?;
                lists = HashMap::new();
            }
            let mut needed_combats = BTreeSet::new();
            let mut needed_results = BTreeSet::new();
            let mut live_indices = Vec::new();
            let mut live_combats = Vec::new();
            for &(index, target) in battles {
                match target {
                    Target::Live { combat, .. } => {
                        needed_combats.insert(combat);
                        live_indices.push(index);
                        live_combats.push(combat);
                    }
                    Target::Result { id } => {
                        needed_results.insert(id);
                    }
                }
            }
            if !live_combats.is_empty() {
                for (index, fleets) in live_indices.into_iter().zip(fleets_at(&mut seeker, run, hour, &live_combats, &content, &mut refused)?) {
                    reply.rows[index].fleets = fleets;
                }
            }
            let battle_index = BattleIndex::build(source, &query, hour, &needed_combats, &needed_results)?;
            let mut active = Vec::new();
            for &(index, target) in battles {
                match target {
                    Target::Result { id } if battle_index.result_hour(id).is_none() => {
                        reply.rows[index].unavailable = Some("Naval battle result was not recorded at this hour".into());
                    }
                    _ => active.push((index, target)),
                }
            }
            if active.is_empty() {
                continue;
            }
            let through: Vec<(i32, i32)> = reads.iter().take_while(|read| read.0 <= hour).map(|&(start, end)| (start, end.min(hour))).collect();
            let Some(&(_, read)) = through.last() else { continue };
            if let std::collections::hash_map::Entry::Vacant(slot) = lists.entry(read) {
                slot.insert(sunk::sinkings_seen(&mut seeker, run, read)?);
            }
            let Some((read_at, entries)) = &lists[&read] else { continue };
            let matches_any = |entry: &replay_data::rows::SunkShipEntry| {
                active.iter().any(|&(_, target)| match target {
                    Target::Live { combat, first_hour } => battle_index.matches_live(combat, first_hour, entry),
                    Target::Result { id } => battle_index.matches_result(id, entry),
                })
            };
            let matched: Vec<_> = entries.iter().filter(|entry| entry.sink_hour <= hour && matches_any(entry)).cloned().collect();
            let details = sunk::ship_details(&mut seeker, run, hour, &matched, &content, &mut refused)?;
            for (index, target) in active {
                let row = &mut reply.rows[index];
                row.ship_read = Some(*read_at);
                row.named_ships = matched
                    .iter()
                    .zip(&details)
                    .filter(|(entry, _)| match target {
                        Target::Live { combat, first_hour } => battle_index.matches_live(combat, first_hour, entry),
                        Target::Result { id } => battle_index.matches_result(id, entry),
                    })
                    .map(|(entry, (name, class_name, sinking_cause))| NavalBattleNamedShip {
                        sink_hour: entry.sink_hour,
                        known_at: sunk::known_at(&through, entry.sink_hour).unwrap_or(*read_at),
                        name: name.clone(),
                        class_name: class_name.clone(),
                        sinking_cause: sinking_cause.clone(),
                    })
                    .collect();
            }
        }
        reply.warnings.extend(refused.lines("Naval battle details"));
        Ok(reply)
    }
}
