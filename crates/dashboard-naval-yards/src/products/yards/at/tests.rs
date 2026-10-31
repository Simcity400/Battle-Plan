//! What the yard readers take of a view's publication at every hour is what the recording gives
//! there, however the walk is cut into segments, published and resumed.

use super::{labels, width};
use crate::{
    products::yards::{
        emit::tests::{SHIP, build, identity, placements, project, scenario},
        layout,
        read::{self, Value},
        segment::{Refits, Repairs, View},
    },
    tests::rows::{Canon, Rows, whole},
};
use dashboard_series::{
    Budget, Cache, Knownness, PageRead, Snapshot,
    family::Directory,
    memory::Memory,
};

/// `value` in the oracle's form, a token cell by its spelling.
fn canon<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    value: &Value,
    token: bool,
) -> Canon {
    match value {
        Value::Null => Canon::Null,
        Value::Unsigned(number) if token => Canon::Token(
            read::token(snapshot, directory, *number)
                .unwrap()
                .expect("a numbered token"),
        ),
        Value::Integer(value) => Canon::Integer(*value),
        Value::Unsigned(value) => Canon::Unsigned(*value),
        Value::List(values) => Canon::List(
            values
                .iter()
                .map(|value| canon(snapshot, directory, value, false))
                .collect(),
        ),
        other => panic!("a yard cell holds {other:?}"),
    }
}

/// Every owner's rows the readers take at `hour`; none where the hour's owners do not resolve.
fn read_at<V: View>(snapshot: &mut Snapshot<'_, Memory>, directory: &Directory, hour: i32) -> Option<Rows> {
    let observed = read::evidence_at(
        snapshot,
        directory.key(layout::FAMILY, layout::OBSERVED, None),
        hour,
    )
    .unwrap();
    if !observed.is_some_and(|evidence| evidence.known == Knownness::Known) {
        return None;
    }
    let mut rows = Rows::new();
    for (actor, entry) in directory.actors.iter().enumerate() {
        let owner = String::from_utf8(entry.name.to_vec()).unwrap();
        for stream in [layout::HOLDERS, layout::SHIPS, layout::TOTALS, layout::REFUSALS] {
            let Some((_, held)) = read::rows_at(
                snapshot,
                directory.key(actor as u64, stream, None),
                width(stream, V::REFITS),
                hour,
            )
            .unwrap() else {
                continue;
            };
            let tokens = labels(stream, V::REFITS);
            for (key, cells) in held {
                // A refusal is keyed by its reason's token.
                let key = canon(snapshot, directory, &Value::Unsigned(key), stream == layout::REFUSALS);
                let cells = cells
                    .iter()
                    .enumerate()
                    .map(|(at, value)| canon(snapshot, directory, value, tokens.contains(&(at + 2))))
                    .collect();
                rows.insert((owner.clone(), stream, key), cells);
            }
        }
    }
    Some(rows)
}

fn reads_what_the_recording_gives<V: View>(label: &str) {
    let frames = scenario();
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    for placement in placements(frames.len()) {
        let mut cuts: Vec<i32> = hours
            .iter()
            .copied()
            .enumerate()
            .filter(|&(at, _)| placement.every != usize::MAX && at % placement.every == placement.every - 1)
            .map(|(_, hour)| hour)
            .collect();
        if cuts.last() != hours.last() {
            cuts.push(*hours.last().unwrap());
        }
        let resume = if placement.resume { cuts.clone() } else { Vec::new() };
        let (pages, root) = build(project::<V>(&frames, &placement.restarts), &cuts, &resume);
        let budget = Budget::new(1 << 30);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, layout::SCHEMA).unwrap();
        for frame in &frames {
            assert_eq!(
                read_at::<V>(&mut snapshot, &directory, frame.hour),
                whole::<V>(frame),
                "{label}, {}: hour {}",
                placement.label,
                frame.hour
            );
        }
    }
}

#[test]
fn the_readers_take_every_owner_s_rows_the_recording_gives_at_every_hour() {
    reads_what_the_recording_gives::<Repairs>("repairs");
    reads_what_the_recording_gives::<Refits>("refits");
}

/// A ship queued at two ports is shown with its details at both, though its row names the lower.
#[test]
fn a_ship_queued_at_two_ports_is_shown_at_both() {
    use replay_data::rows::{NavalBaseRow, NavalRepairQueueRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    const PORT: u32 = 12;
    let mut frames = scenario();
    for frame in &mut frames {
        if let TableView::Complete(TableRows::NavalBases(rows)) =
            &mut *frame.tables[TableId::NavalBases.index()]
        {
            rows.push(NavalBaseRow {
                province: PORT,
                country: 11,
                ..Default::default()
            });
        }
        if let TableView::Complete(TableRows::NavalRepairQueue(rows)) =
            &mut *frame.tables[TableId::NavalRepairQueue.index()]
        {
            rows.push(NavalRepairQueueRow {
                province: PORT,
                ordinal: 0,
                ship: SHIP,
            });
        }
    }
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let placement = &placements(frames.len())[0];
    let (pages, root) = build(
        project::<Repairs>(&frames, &placement.restarts),
        &[*hours.last().unwrap()],
        &[],
    );
    let budget = Budget::new(1 << 30);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, layout::SCHEMA).unwrap();
    let mut shown = 0;
    for frame in &frames {
        let params = super::YardsAtParams {
            branch_id: "1".into(),
            tag: "MOD".into(),
            hour: frame.hour,
        };
        let reply = super::query(&mut snapshot, &directory, false, &params, Ok(None))
            .unwrap()
            .unwrap();
        let ports: Vec<_> = reply
            .holders
            .iter()
            .filter(|port| port.queue.iter().any(|entry| entry.ship == SHIP.to_string()))
            .collect();
        for port in &ports {
            let ship = port.ships.iter().find(|ship| ship.id == SHIP.to_string());
            assert!(
                ship.is_some_and(|ship| ship.measures.contains_key("hull")),
                "port {} at hour {}",
                port.id,
                frame.hour
            );
        }
        if ports.len() == 2 {
            shown += 1;
        }
    }
    assert!(shown > 0, "the ship is queued at both ports");
}

/// A port's reply lists its queue as the recording orders it, repeats kept, at every hour, and
/// each ship it holds as present; an hour whose countries are not recorded still carries why the
/// content names are unavailable.
#[test]
fn a_port_s_reply_lists_its_queue_in_order() {
    let frames = scenario();
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let placement = &placements(frames.len())[0];
    let (pages, root) = build(
        project::<Repairs>(&frames, &placement.restarts),
        &[*hours.last().unwrap()],
        &[],
    );
    let budget = Budget::new(1 << 30);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, layout::SCHEMA).unwrap();
    let mut queued = 0;
    for frame in &frames {
        let params = super::YardsAtParams {
            branch_id: "1".into(),
            tag: "MOD".into(),
            hour: frame.hour,
        };
        let reply = super::query(&mut snapshot, &directory, false, &params, Ok(None))
            .unwrap()
            .unwrap();
        for holder in &reply.holders {
            let province: u32 = holder.id.parse().unwrap();
            let expected: Vec<(String, String)> = frame
                .complete::<replay_data::rows::tables::NavalRepairQueue>()
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.province == province)
                        .map(|row| (row.ordinal.to_string(), row.ship.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let listed: Vec<(String, String)> = holder
                .queue
                .iter()
                .map(|entry| (entry.ordinal.clone(), entry.ship.clone()))
                .collect();
            assert_eq!(listed, expected, "port {province} at hour {}", frame.hour);
            queued += listed.len();
            for ship in &holder.ships {
                assert_eq!(ship.measures.get("present").map(String::as_str), Some("1"));
            }
        }
        let unloaded = super::query(&mut snapshot, &directory, false, &params, Err("no game"))
            .unwrap()
            .unwrap();
        assert!(
            unloaded
                .warnings
                .iter()
                .any(|warning| warning == "Naval yard content names unavailable: no game"),
            "at hour {}: {:?}",
            frame.hour,
            unloaded.warnings
        );
    }
    assert!(queued > 0, "the scenario queues ships");
}
