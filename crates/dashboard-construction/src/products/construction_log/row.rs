//! The row oracle: a branch's decoded frames as the hours the scan observes
//! ([`observed::Hour`]), each building table whole, and the scan of a branch from them, which
//! the columnar operator ([`super::facts`]) is checked against.

use super::{
    Scan, ScannedLog, SpendingGap,
    observed::{self, Building, BuildingLine, Buildings, Output, OutputLine, Queue, RailwayLine},
};
use dashboard_text::metadata::{
    branch_bounds, link_slot, mapped_slot, slot_read, validate_campaign,
};
use engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
use replay::container::Replay;
use replay_data::rows::{BuildingRow, Recorded, TableId, TableSchema, tables};
use replay_frame::frame::{DecodedFrame, FrameDecoder, SharedView, TableView};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A lexer id's literal in a complete dictionary; none when it is absent or not text.
fn literal(view: &SharedView, id: u32) -> Option<String> {
    let TableView::Complete(rows) = &**view else {
        return None;
    };
    let rows = rows.rows::<tables::LexicalTokens>()?;
    let index = rows.binary_search_by_key(&id, |row| row.token).ok()?;
    std::str::from_utf8(&rows[index].literal)
        .ok()
        .filter(|text| !text.contains('\0'))
        .map(str::to_owned)
}

/// The queue owner's tag at this hour, when the links resolve it.
fn owner_tag(frame: &DecodedFrame, raw: u32) -> Option<Arc<str>> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    let slot = link_slot(links, raw)?;
    countries
        .iter()
        .find(|country| {
            country.canonical_slot == slot && mapped_slot(country, links).ok() == Some(slot)
        })
        .map(|country| Arc::from(String::from_utf8_lossy(&country.tag).as_ref()))
}

/// A province's state from this hour's complete Provinces table.
fn province_state(frame: &DecodedFrame, province: u32) -> Option<u32> {
    let rows = frame.complete::<tables::Provinces>().ok()?;
    rows.binary_search_by_key(&province, |row| row.id)
        .ok()
        .map(|index| rows[index].state)
        .filter(|state| *state != 0)
}

/// A building table, whole while complete.
fn buildings<T: TableSchema<Row = BuildingRow>>(frame: &DecodedFrame) -> Buildings {
    match frame.complete::<T>() {
        Ok(rows) => Buildings::Whole(
            rows.iter()
                .map(|row| Building {
                    location: row.location,
                    kind: row.building_type,
                    level: row.level,
                    healthy: row.healthy_level,
                })
                .collect(),
        ),
        Err(_) => Buildings::Incomplete,
    }
}

/// What the oracle keeps from hour to hour: the dictionary it last took, whether it spelled
/// each type a line named, and how many distinct dictionaries it took.
#[derive(Default)]
pub struct Rows {
    dictionary: Option<SharedView>,
    spelled: BTreeMap<u32, bool>,
    pub dictionaries: usize,
}

impl Rows {
    /// `frame` as the scan observes it, its queues and output read where `reads` says.
    pub fn hour(&mut self, frame: &DecodedFrame, reads: [bool; 3]) -> observed::Hour {
        let mut types = Vec::new();
        let view = &frame.tables[TableId::LexicalTokens.index()];
        // An unchanged table is the same shared view hour after hour: taken once.
        if matches!(&**view, TableView::Complete(_))
            && !self
                .dictionary
                .as_ref()
                .is_some_and(|held| SharedView::ptr_eq(held, view))
        {
            self.dictionary = Some(view.clone());
            self.dictionaries += 1;
            for (&id, spelled) in &mut self.spelled {
                if !*spelled && let Some(token) = literal(view, id) {
                    *spelled = true;
                    types.push((id, Some(token)));
                }
            }
        }
        let queue = match (reads[0], frame.complete::<tables::BuildingConstruction>()) {
            (false, _) => Queue::NotRead,
            (true, Ok(rows)) => Queue::Read(
                rows.iter()
                    .map(|row| BuildingLine {
                        owner: owner_tag(frame, row.country),
                        row: row.clone(),
                    })
                    .collect(),
            ),
            (true, Err(_)) => Queue::Failed,
        };
        let railways = match (reads[1], frame.complete::<tables::RailwayConstruction>()) {
            (false, _) => Queue::NotRead,
            (true, Ok(rows)) => Queue::Read(
                rows.iter()
                    .map(|row| RailwayLine {
                        owner: owner_tag(frame, row.country),
                        from: row
                            .route
                            .first()
                            .and_then(|&province| province_state(frame, province)),
                        to: row
                            .route
                            .last()
                            .and_then(|&province| province_state(frame, province)),
                        row: row.clone(),
                    })
                    .collect(),
            ),
            (true, Err(_)) => Queue::Failed,
        };
        let output = match (reads[2], frame.complete::<tables::ConstructionLineOutput>()) {
            (false, _) => Output::NotRead,
            (true, Ok(rows)) => Output::Read(
                rows.iter()
                    .map(|row| {
                        let value = |field| match field {
                            Recorded::Value(value) => value,
                            Recorded::NotRecorded => None,
                        };
                        OutputLine {
                            owner: value(row.country).and_then(|country| owner_tag(frame, country)),
                            from: value(row.route_first)
                                .and_then(|province| province_state(frame, province)),
                            to: value(row.route_last)
                                .and_then(|province| province_state(frame, province)),
                            row: row.clone(),
                        }
                    })
                    .collect(),
            ),
            (true, Err(_)) => Output::Unavailable(
                match &*frame.tables[TableId::ConstructionLineOutput.index()] {
                    TableView::Unavailable { reason } if reason.contains("not recorded") => {
                        SpendingGap::NotRecorded
                    }
                    _ => SpendingGap::ReadFailed,
                },
            ),
        };
        let mut named: Vec<u32> = match &queue {
            Queue::Read(lines) => lines.iter().map(|line| line.row.building_type).collect(),
            Queue::NotRead | Queue::Failed => Vec::new(),
        };
        if matches!(&railways, Queue::Read(lines) if !lines.is_empty()) {
            named.push(RAILWAY_TEMPLATE_TOKEN);
        }
        if let Output::Read(lines) = &output {
            for line in lines {
                if line.row.railway {
                    named.push(RAILWAY_TEMPLATE_TOKEN);
                } else if let Recorded::Value(Some(kind)) = line.row.building_type {
                    named.push(kind);
                }
            }
        }
        for id in named {
            if let Entry::Vacant(entry) = self.spelled.entry(id) {
                let token = self.dictionary.as_ref().and_then(|view| literal(view, id));
                entry.insert(token.is_some());
                types.push((id, token));
            }
        }
        observed::Hour {
            branch: frame.branch,
            hour: frame.hour,
            buildings: [
                buildings::<tables::StateBuildings>(frame),
                buildings::<tables::ProvinceBuildings>(frame),
            ],
            queue,
            railways,
            output,
            types,
        }
    }
}

/// Scans one branch for every construction project.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedLog> {
    scan_with_reads(replay, branch, cancelled, |frame, table| {
        slot_read(replay.format, &frame.metrics, table, frame.hour)
    })
}

/// [`scan`], each queue and the output read where `read_queue` says.
pub fn scan_with_reads(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    read_queue: impl Fn(&DecodedFrame, TableId) -> bool,
) -> io::Result<ScannedLog> {
    let selected = super::TABLES;
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Construction request cancelled");
    let mut scan = Scan::default();
    let mut rows = Rows::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let hour = rows.hour(
                        decoded,
                        [
                            TableId::BuildingConstruction,
                            TableId::RailwayConstruction,
                            TableId::ConstructionLineOutput,
                        ]
                        .map(|table| read_queue(decoded, table)),
                    );
                    scan.observe_reads(&hour, true);
                }
            }
            Ok(())
        },
    )?;
    let last_hour = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start).1);
    scan.finish(last_hour);
    scan.log.dictionaries = rows.dictionaries;
    Ok(scan.log)
}
