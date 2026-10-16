//! Drawn-row enrichment through full recorded references, never roster guesses.
use super::Kind;
#[cfg(any(test, feature = "test-support"))]
use super::{branch_id, census};
use dashboard_base::{
    catalogue,
    products::{Product, ReplyWarnings},
};
use dashboard_text::scripted_text;
use replay::{
    frame::DecodedFrame,
    rows::{Recorded, tables},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};
use ts_rs::TS;
#[cfg(any(test, feature = "test-support"))]
use {
    crate::products::{ships_sunk, sinkings},
    dashboard_base::source::Source,
    dashboard_core::config::LoadContext,
    dashboard_text::metadata::{check_cancel, validate_campaign},
    replay::{container::Replay, frame::FrameDecoder, rows::TableId},
    std::{
        collections::{BTreeMap, HashMap},
        sync::atomic::AtomicBool,
    },
};

pub struct NavalBattleDetails;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleDetailRequest {
    pub id: String,
    pub hour: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleDetailsParams {
    pub branch_id: String,
    pub battles: Vec<NavalBattleDetailRequest>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
pub struct NavalBattleFleetNames {
    pub names: Vec<String>,
    pub unknown: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleNamedShip {
    pub sink_hour: i32,
    pub known_at: i32,
    pub name: Option<String>,
    pub class_name: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleDetail {
    pub id: String,
    pub hour: i32,
    /// Fleets use the exact last observed combat roster, including ended rows.
    pub roster_hour: Option<i32>,
    pub fleets: [NavalBattleFleetNames; 2],
    pub named_ships: Vec<NavalBattleNamedShip>,
    pub ship_read: Option<i32>,
    pub unavailable: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
pub struct NavalBattleDetailsReply {
    pub rows: Vec<NavalBattleDetail>,
    pub warnings: Vec<String>,
}
impl ReplyWarnings for NavalBattleDetailsReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

#[cfg(any(test, feature = "test-support"))]
const ROSTER_TABLES: [TableId; 4] = [
    TableId::Campaign,
    TableId::NavalCombatDetails,
    TableId::TaskForces,
    TableId::Fleets,
];

/// Read only supplying frames requested by drawn rows; a gap never seeks nearby.
#[cfg(any(test, feature = "test-support"))]
fn roster_frames(
    replay: &Replay,
    branch: u64,
    hours: &BTreeSet<i32>,
    flag: &AtomicBool,
) -> io::Result<BTreeMap<i32, DecodedFrame>> {
    let mut out = BTreeMap::new();
    for block in replay.index.iter().filter(|block| {
        block.branch == branch
            && hours
                .range(block.first_hour..=block.last_hour)
                .next()
                .is_some()
    }) {
        check_cancel(flag)?;
        let frames = replay.read_block_selected(block, &ROSTER_TABLES)?;
        let mut decoder = FrameDecoder::for_tables(&ROSTER_TABLES);
        let last = *hours
            .range(block.first_hour..=block.last_hour)
            .next_back()
            .expect("wanted roster hour");
        for frame in frames {
            check_cancel(flag)?;
            if frame.hour > last {
                break;
            }
            let decoded = decoder.advance(&frame)?;
            if hours.contains(&decoded.hour) {
                validate_campaign(decoded, replay)?;
                out.insert(decoded.hour, decoded.clone());
            }
        }
    }
    Ok(out)
}

pub(super) fn fleet_names(
    frame: Option<&DecodedFrame>,
    combat: u64,
    province: u32,
    cat: Option<&catalogue::Catalogue>,
    refused: &mut catalogue::Refusals,
) -> [NavalBattleFleetNames; 2] {
    let mut out = std::array::from_fn(|_| NavalBattleFleetNames {
        names: vec![],
        unknown: None,
    });
    let Some((frame, cat)) = frame.zip(cat) else {
        for side in &mut out {
            side.unknown = Some("Fleet roster or content not recorded".into());
        }
        return out;
    };
    let Some((detail, tasks, fleets)) = frame
        .complete::<tables::NavalCombatDetails>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&combat, |row| row.combat)
                .ok()
                .map(|index| &rows[index])
                .filter(|row| row.province == province)
        })
        .zip(frame.complete::<tables::TaskForces>().ok())
        .zip(frame.complete::<tables::Fleets>().ok())
        .map(|((a, b), c)| (a, b, c))
    else {
        for side in &mut out {
            side.unknown = Some("Fleet identities not recorded".into());
        }
        return out;
    };
    for (side, input) in out.iter_mut().zip(&detail.sides) {
        let mut seen = BTreeSet::new();
        let mut missing = 0;
        // Combatant units are task-force handles. The recorded
        // TaskForce.fleet resolves the fleet reference.
        for reference in &input.units {
            let fleet = tasks
                .binary_search_by_key(reference, |row| row.id)
                .ok()
                .map(|index| &tasks[index])
                .and_then(|task| {
                    fleets
                        .binary_search_by_key(&task.fleet, |row| row.id)
                        .ok()
                        .map(|index| &fleets[index])
                });
            let Some(fleet) = fleet else {
                missing += 1;
                continue;
            };
            if !seen.insert(fleet.id) {
                continue;
            }
            match &fleet.name {
                Recorded::Value(Some(bytes)) => match scripted_text::stored_text(
                    &cat.localisation,
                    &cat.recorded_language,
                    bytes,
                    "fleet name",
                )
                .and_then(|text| catalogue::display_text(text.as_bytes()))
                {
                    Ok(name) if !name.is_empty() => side.names.push(name),
                    Ok(_) => missing += 1,
                    Err(reason) => {
                        refused.note(reason, format!("fleet {}", fleet.id));
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
    out
}

#[cfg(any(test, feature = "test-support"))]
fn matching_ships<'a>(
    ships: &'a [&'a sinkings::ShipEvent],
    event_through: i32,
    read_through: i32,
) -> impl Iterator<Item = &'a sinkings::ShipEvent> {
    ships.iter().copied().filter(move |ship| {
        visible(
            ship.entry.sink_hour,
            ship.read_hour,
            event_through,
            read_through,
        )
    })
}

pub(super) fn visible(
    sink_hour: i32,
    known_at: i32,
    event_through: i32,
    read_through: i32,
) -> bool {
    sink_hour <= event_through && known_at <= read_through
}
pub(super) fn selection(
    episode: &super::Episode,
    hour: i32,
) -> io::Result<(&super::Run, Option<(u64, i32)>)> {
    let at = episode.runs.partition_point(|run| run.first <= hour);
    let run = at
        .checked_sub(1)
        .map(|index| &episode.runs[index])
        .filter(|run| hour <= run.last)
        .ok_or_else(|| {
            dashboard_core::invalid("Naval battle has no supplying read at requested hour")
        })?;
    let roster = if episode.kind == Kind::Combat {
        Some((episode.reference, hour))
    } else {
        run.sample
            .observed
            .as_ref()
            .map(|observed| (observed.combat, observed.last))
    };
    Ok((run, roster))
}
pub(super) fn detail_row(
    request: &NavalBattleDetailRequest,
    roster: Option<(u64, i32)>,
    fleets: [NavalBattleFleetNames; 2],
    named_ships: Vec<NavalBattleNamedShip>,
    ship_read: Option<i32>,
) -> NavalBattleDetail {
    NavalBattleDetail {
        id: request.id.clone(),
        hour: request.hour,
        roster_hour: roster.map(|(_, hour)| hour),
        fleets,
        named_ships,
        ship_read,
        unavailable: roster
            .is_none()
            .then(|| "Named ships require the recorded combat identity".into()),
    }
}
impl Product for NavalBattleDetails {
    const KEY: &'static str = "naval-battle-details";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalBattleDetailsParams;
    type Reply = NavalBattleDetailsReply;
    fn types() -> Vec<String> {
        vec![
            NavalBattleDetailRequest::decl(),
            NavalBattleDetailsParams::decl(),
            NavalBattleFleetNames::decl(),
            NavalBattleNamedShip::decl(),
            NavalBattleDetail::decl(),
            NavalBattleDetailsReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid battle branch")?;
        let mut ids = BTreeSet::new();
        if params
            .battles
            .iter()
            .any(|row| row.id.is_empty() || !ids.insert((&row.id, row.hour)))
        {
            return Err("Invalid battle detail selection");
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        flag: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch_id(source, &params.branch_id)?;
        let census = census(source, branch, flag)?;
        let ships = sinkings::cached(source, branch, flag)?;
        let names = ships_sunk::cached_names(source, branch, &ships, context, flag)?;
        let cat = context
            .content()
            .and_then(|content| catalogue::catalogue(source, content));
        let mut reply = NavalBattleDetailsReply {
            rows: vec![],
            warnings: names.warnings().to_vec(),
        };
        if let Err(reason) = &cat {
            reply
                .warnings
                .push(format!("Fleet names unavailable: {reason}"));
        }
        let episodes: HashMap<_, _> = census
            .episodes
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let mut selected = vec![];
        let mut hours = BTreeSet::new();
        for request in &params.battles {
            check_cancel(flag)?;
            let episode = episodes
                .get(request.id.as_str())
                .ok_or_else(|| dashboard_core::invalid("Unknown naval battle"))?;
            let (run, roster) = selection(episode, request.hour)?;
            if let Some((_, hour)) = roster {
                hours.insert(hour);
            }
            selected.push((request, episode, run, roster));
        }
        let frames = roster_frames(source.replay_ref(), branch, &hours, flag)?;
        let by_combat = super::grouped(ships.ships.iter(), |ship| ship.entry.battle);
        let mut refused = catalogue::Refusals::default();
        for (request, episode, run, roster) in selected {
            check_cancel(flag)?;
            let fleets = roster
                .map(|(combat, hour)| {
                    fleet_names(
                        frames.get(&hour),
                        combat,
                        episode.province,
                        cat.as_ref().ok().map(|cat| cat.as_ref()),
                        &mut refused,
                    )
                })
                .unwrap_or_else(|| {
                    std::array::from_fn(|_| NavalBattleFleetNames {
                        names: vec![],
                        unknown: Some("Fleet identities not recorded".into()),
                    })
                });
            let named_ships = roster
                .map(|(combat, _)| {
                    matching_ships(
                        by_combat.get(&combat).map(Vec::as_slice).unwrap_or(&[]),
                        run.sample.end.unwrap_or(request.hour),
                        request.hour,
                    )
                    .map(|ship| {
                        let (name, class_name) = names.ship(ship);
                        NavalBattleNamedShip {
                            sink_hour: ship.entry.sink_hour,
                            known_at: ship.read_hour,
                            name,
                            class_name,
                        }
                    })
                    .collect()
                })
                .unwrap_or_default();
            let read = ships
                .ship_reads
                .partition_point(|hour| *hour <= request.hour);
            reply.rows.push(detail_row(
                request,
                roster,
                fleets,
                named_ships,
                read.checked_sub(1).map(|index| ships.ship_reads[index]),
            ));
        }
        reply
            .warnings
            .extend(refused.lines("Naval battle fleet names"));
        dashboard_base::products::check_reply_budget(&reply)
            .map_err(|_| io::Error::other("Naval battle details exceed their reply budget"))?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::source;
    use dashboard_text::catalogue::tests::folder_catalogue;
    use replay::{
        frame::TableView,
        rows::{FleetRow, NavalCombatDetailsRow, TableRows, TaskForceRow},
    };

    #[test]
    fn fleet_labels_follow_exact_handles_in_native_order_and_refuse_missing_names() {
        let combat = (1_u64 << 60) + 9;
        let task = (1_u64 << 61) + 7;
        let fleet = (1_u64 << 62) + 3;
        let mut frame = source(1, 8, Some(1));
        let mut detail = NavalCombatDetailsRow {
            combat,
            province: 15,
            ..Default::default()
        };
        detail.sides[0].units = vec![task, task + 1, task, 7];
        detail.sides[1].units = vec![task + 2];
        frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![detail])).into();
        frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![
                TaskForceRow {
                    id: task,
                    fleet,
                    ..Default::default()
                },
                TaskForceRow {
                    id: task + 1,
                    fleet: fleet + 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: task + 2,
                    fleet: fleet + 2,
                    ..Default::default()
                },
            ]))
            .into();
        frame.tables[TableId::Fleets.index()] = TableView::Complete(TableRows::Fleets(vec![
            FleetRow {
                id: 3,
                name: Recorded::Value(Some(b"Wrong low-word fleet".to_vec())),
                ..Default::default()
            },
            FleetRow {
                id: fleet,
                name: Recorded::Value(Some(b"\x11YLiteral mod fleet\x11!".to_vec())),
                ..Default::default()
            },
            FleetRow {
                id: fleet + 1,
                name: Recorded::Value(Some(b"Second fleet".to_vec())),
                ..Default::default()
            },
            FleetRow {
                id: fleet + 2,
                name: Recorded::NotRecorded,
                ..Default::default()
            },
        ]))
        .into();
        let (mut cat, _folder) = folder_catalogue(&[]);
        let mut refused = catalogue::Refusals::default();
        let names = fleet_names(Some(&frame), combat, 15, Some(&cat), &mut refused);
        assert_eq!(names[0].names, ["Literal mod fleet", "Second fleet"]);
        assert_eq!(
            names[0].unknown.as_deref(),
            Some("1 fleet names not recorded")
        );
        assert!(names[1].names.is_empty());
        assert!(names[1].unknown.is_some());
        assert!(
            fleet_names(Some(&frame), 9, 15, Some(&cat), &mut refused)[0]
                .names
                .is_empty()
        );
        cat.recorded_language = "different recorded language".into();
        assert!(
            fleet_names(Some(&frame), combat, 15, Some(&cat), &mut refused)[0]
                .names
                .is_empty()
        );
        assert!(!refused.lines("Fleet names").is_empty());
    }

    #[test]
    fn ship_join_uses_full_reference_and_both_recorded_dates() {
        let reference = 0x0500_0000_0000_0007;
        let mut scan = sinkings::Scanned::default();
        for (battle, sink_hour, read_hour) in [
            (reference, 9, 19),
            (7, 9, 19),
            (reference, 11, 19),
            (reference, 9, 43),
            (reference, 9, 19),
        ] {
            scan.ships.push(sinkings::ShipEvent {
                entry: replay::rows::SunkShipEntry {
                    battle,
                    sink_hour,
                    ..Default::default()
                },
                read_hour,
                victim: None,
                attacker: None,
                victim_as_of: None,
                attacker_as_of: None,
            });
        }
        let by_combat = super::super::grouped(scan.ships.iter(), |ship| ship.entry.battle);
        let ships = &by_combat[&reference];
        assert_eq!(matching_ships(ships, 10, 19).count(), 2);
        assert_eq!(matching_ships(ships, 10, 18).count(), 0);
        assert_eq!(matching_ships(ships, 11, 43).count(), 4);
    }
}
