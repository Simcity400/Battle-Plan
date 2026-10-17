//! A newest window whose projects and output chains start inside its look-back.
use super::*;
use dashboard_base::prepared::{frame_changes::FrameChangeTracker, query::progressive};
use dashboard_text::metadata::{check_cancel, validate_campaign};
use replay::frame::FrameDecoder;

const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Provinces,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::ConstructionLineOutput,
    TableId::LexicalTokens,
];

struct Window {
    reply: ConstructionLogReply,
    type_ids: Vec<u32>,
    recorded: bool,
}

impl Window {
    fn new(mut reply: ConstructionLogReply, log: &ScannedLog) -> Self {
        let used: BTreeSet<_> = reply
            .projects
            .iter()
            .map(|project| project.building)
            .collect();
        let remap: BTreeMap<_, _> = used
            .into_iter()
            .enumerate()
            .map(|(index, old)| (old, index as u32))
            .collect();
        let type_ids = remap
            .keys()
            .map(|&index| log.types[index as usize].0)
            .collect();
        for project in &mut reply.projects {
            project.building = remap[&project.building];
        }
        Self {
            reply,
            type_ids,
            recorded: !log.reads.is_empty() || !log.railway_reads.is_empty(),
        }
    }
}

/// Only the newest numeric window is retained per branch/revision. A re-ask
/// remaps its types from the growing catalogue without repeating the look-back.
#[derive(Default)]
struct Cache {
    windows: BTreeMap<u64, Option<Window>>,
}

#[cfg(test)]
#[derive(Default)]
struct Reads(usize);

pub(super) fn query(
    source: &Source,
    branch: u64,
    types: &[BuildingInfo],
    cancelled: &AtomicBool,
) -> io::Result<Option<(ConstructionLogReply, bool)>> {
    let cache = source.state::<Cache>();
    let mut cache = match cache.try_lock() {
        Ok(cache) => cache,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        // Another request is proving this window. This one can keep its
        // immediate prefix and re-ask as preparation moves, without waiting.
        Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
    };
    if let std::collections::btree_map::Entry::Vacant(entry) = cache.windows.entry(branch) {
        progressive::decodes_allowed()?;
        let window = prepare(source, branch, cancelled)?;
        entry.insert(window);
    }
    let Some(window) = cache.windows.get(&branch).and_then(Option::as_ref) else {
        return Ok(None);
    };
    // Construction keeps the first resolved spelling of a building id. A
    // fresh scan cannot prove that spelling from a late dictionary alone.
    // Use only types already resolved by the forward catalogue; otherwise
    // leave the newest window pending until that catalogue reaches them.
    let own: Option<Vec<_>> = window
        .type_ids
        .iter()
        .map(|id| {
            types
                .iter()
                .find(|kind| kind.id == *id && kind.token.is_some())
                .cloned()
        })
        .collect();
    let Some(own) = own else {
        return Ok(None);
    };
    let mut reply = window.reply.clone();
    reply.types = Some(own);
    reply.exact_from = Some(reply.interval.first_hour);
    Ok(Some((reply, window.recorded)))
}

fn prepare(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Option<Window>> {
    let replay = source.replay_ref();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    let Some(last) = blocks.last() else {
        return Ok(None);
    };
    let target = last.first_hour;
    let mut start = blocks.len().saturating_sub(2);
    loop {
        let first = blocks[start].first_hour;
        let mut scan = Scan::default();
        let mut tracker = FrameChangeTracker::new();
        let mut output_since: BTreeMap<u64, i32> = BTreeMap::new();
        let read = |block: &replay::container::BlockIndex| {
            #[cfg(test)]
            {
                source.state::<Reads>().lock().unwrap().0 += 1;
            }
            replay.read_block_selected(block, TABLES)
        };
        let readable = dashboard_core::blocks::scan_blocks_by(
            blocks[start..].to_vec(),
            std::thread::available_parallelism()
                .map_or(1, |cores| cores.get() / 2)
                .max(1),
            cancelled,
            &read,
            |reads| {
                for (_, frames) in reads {
                    let Ok(frames) = frames else {
                        return Ok(false);
                    };
                    let mut decoder = FrameDecoder::for_tables(TABLES);
                    for frame in frames {
                        check_cancel(cancelled)?;
                        let Ok(decoded) = decoder.advance(&frame) else {
                            return Ok(false);
                        };
                        if validate_campaign(decoded, replay).is_err() {
                            return Ok(false);
                        }
                        let changes = tracker.observe(decoded, replay.format);
                        if changes.slot_read[TableId::ConstructionLineOutput.index()]
                            && let Ok(rows) = decoded.complete::<tables::ConstructionLineOutput>()
                        {
                            for row in rows {
                                output_since
                                    .entry(row.line)
                                    .and_modify(|first| *first = (*first).min(row.since_hour))
                                    .or_insert(row.since_hour);
                            }
                        }
                        scan.observe_changed(decoded, replay.format, &changes);
                    }
                    scan.seal();
                }
                Ok(true)
            },
        )?;
        if !readable {
            return Ok(None);
        }
        scan.sync_reads();
        let cut = Cut::new(Arc::new(scan.log), windows(replay, branch));
        let reply = cut.reply(blocks.len() - 1);
        if start == 0 {
            return Ok(Some(Window::new(reply, &cut.log)));
        }
        // Every project whose row or output the window carries must have its
        // real first queue read inside the look-back. A successful preceding
        // queue read and hourly building watches establish its opening stretch.
        // Each output chain must start here too: otherwise an earlier restart
        // could leave even its newest cumulative value unknown.
        let selected: BTreeSet<_> = reply
            .projects
            .iter()
            .map(|project| project.id.as_str())
            .chain(reply.outputs.iter().map(|output| output.id.as_str()))
            .collect();
        let mut before = target;
        let mut anchored = true;
        // A closed project can outlive its queue row in the output observer.
        // An unavailable table can change such an unseen chain's reason, and
        // an orphan row can belong to a project last listed before this scan.
        // Without an owner/supplying anchor, leave the window to preparation.
        if cut
            .log
            .output_reads
            .iter()
            .any(|(_, reason)| reason.is_some())
        {
            return Ok(None);
        }
        let known: BTreeSet<_> = cut
            .log
            .projects
            .iter()
            .map(|project| project.id.parse::<u64>().expect("native project handle"))
            .collect();
        for (&id, &since) in &output_since {
            if !known.contains(&id) {
                if since >= first {
                    return Ok(None);
                }
                anchored = false;
                before = before.min(since.saturating_sub(1));
            }
        }
        for project in cut
            .log
            .projects
            .iter()
            .filter(|project| selected.contains(project.id.as_str()))
        {
            let queue = if project.kind == ProjectKind::Railway {
                &cut.log.railway_reads
            } else {
                &cut.log.reads
            };
            let preceding = queue
                .first()
                .is_some_and(|hour| *hour < project.created_hour && *hour < project.first_read);
            let since = output_since
                .get(&project.id.parse::<u64>().expect("native project handle"))
                .copied();
            if !preceding || since.is_some_and(|hour| hour < first) {
                anchored = false;
                before = before.min(
                    project
                        .created_hour
                        .min(project.first_read)
                        .saturating_sub(1),
                );
                if let Some(since) = since {
                    before = before.min(since.saturating_sub(1));
                }
            }
        }
        // The output read held over the window's first hour is part of it.
        if cut
            .log
            .output_reads
            .first()
            .is_none_or(|(hour, _)| *hour >= target)
        {
            anchored = false;
            before = before.min(first.saturating_sub(1));
        }
        if anchored {
            return Ok(Some(Window::new(reply, &cut.log)));
        }
        // The recorded creation/supplying hours choose the bound. No project
        // duration or number of look-back blocks is capped by the application.
        start = blocks
            .partition_point(|block| block.first_hour <= before)
            .saturating_sub(1)
            .min(start - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::construction_log::tests::{
        CIVILIAN, MOD_REACTOR, building_at, building_line, construction_frame,
    };
    use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, records};
    use replay::rows::{ConstructionLineOutputRow, Recorded, TableRows};

    #[derive(Clone, Copy)]
    enum Extra {
        None,
        Orphan,
        Unread,
    }

    fn recording(blocks: usize, long: bool, restart: bool, extra: Extra) -> Replay {
        let last = 100 + (blocks as i32 - 1) * 8;
        let created = if long { 101 } else { last - 5 };
        let frames: Vec<_> = (100..last + 8)
            .map(|hour| {
                let mut lines = Vec::new();
                if hour == 100 {
                    let mut old = building_line(2, 6, 1, 0, 1);
                    old.building_type = MOD_REACTOR;
                    lines.push(old);
                }
                if hour > created && hour < last + 3 {
                    let mut line = building_line(1, 5, 1, 0, 2);
                    line.created_hour = created;
                    lines.push(line);
                }
                let mut frame = construction_frame(
                    hour,
                    true,
                    lines,
                    vec![],
                    Some(vec![building_at(5, CIVILIAN, (1, 1))]),
                    vec![],
                );
                frame.metrics.table_ns[TableId::ConstructionLineOutput.index()] = 1;
                *frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(
                    TableRows::ConstructionLineOutput(if hour > created && hour <= last + 3 {
                        vec![ConstructionLineOutputRow {
                            line: 1,
                            since_hour: if restart && hour >= created + 2 {
                                created + 1
                            } else {
                                created
                            },
                            construction_raw: i64::from(hour - created),
                            observed_since_creation: true,
                            exact: true,
                            ended: hour == last + 3,
                            completion: Recorded::Value(None),
                            ..Default::default()
                        }]
                    } else {
                        vec![]
                    }),
                );
                if hour == last + 1 {
                    match extra {
                        Extra::None => {}
                        Extra::Orphan => {
                            if let TableView::Complete(TableRows::ConstructionLineOutput(rows)) =
                                &mut *frame.tables[TableId::ConstructionLineOutput.index()]
                            {
                                rows.push(ConstructionLineOutputRow {
                                    line: 999,
                                    since_hour: last,
                                    exact: true,
                                    ..Default::default()
                                });
                            }
                        }
                        Extra::Unread => {
                            *frame.tables[TableId::ConstructionLineOutput.index()] =
                                TableView::Unavailable {
                                    reason: "synthetic unavailable output read".into(),
                                }
                        }
                    }
                }
                frame
            })
            .collect();
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..8].to_vec())]).unwrap();
        for chunk in frames[8..].chunks(8) {
            replay = replay
                .append(&[continuation_records(&replay, 1, chunk.to_vec())])
                .unwrap();
        }
        replay
    }

    #[test]
    fn newest_window_matches_the_complete_log_with_local_types_long_projects_and_output_restarts() {
        let flag = AtomicBool::new(false);
        for (long, restart) in [(false, false), (true, false), (true, true)] {
            let replay = recording(8, long, restart, Extra::None);
            let log = crate::products::construction_log::scan(&replay, 1, &flag).unwrap();
            let types: Vec<_> = log
                .types
                .iter()
                .map(|(id, token)| BuildingInfo {
                    id: *id,
                    token: token.clone(),
                    name: None,
                    icon: None,
                })
                .collect();
            let expected = Cut::new(Arc::new(log), windows(&replay, 1)).reply(7);
            let (source, root) = dashboard_base::test_fixtures::file_source(replay);
            let (mut actual, recorded) = query(&source, 1, &types, &flag)
                .unwrap()
                .expect("a proven newest window");
            assert!(recorded);
            let own = actual.types.take().unwrap();
            for project in &mut actual.projects {
                project.building = types
                    .iter()
                    .position(|kind| kind.id == own[project.building as usize].id)
                    .unwrap() as u32;
            }
            assert_eq!(actual.exact_from.take(), Some(expected.interval.first_hour));
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
            let count = source.state::<Reads>().lock().unwrap().0;
            assert!(query(&source, 1, &types, &flag).unwrap().is_some());
            assert_eq!(
                source.state::<Reads>().lock().unwrap().0,
                count,
                "a re-ask repeated the look-back"
            );
            drop(source);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn newest_reads_depend_on_the_anchor_and_never_on_earlier_block_count_or_unproven_type_names() {
        let flag = AtomicBool::new(false);
        for blocks in [2, 8] {
            let (source, root) = dashboard_base::test_fixtures::file_source(recording(
                blocks,
                false,
                false,
                Extra::None,
            ));
            assert!(
                query(&source, 1, &[], &flag).unwrap().is_none(),
                "a late dictionary cannot prove the first resolved type"
            );
            let types = [BuildingInfo {
                id: CIVILIAN,
                token: Some("synthetic_type".into()),
                name: None,
                icon: None,
            }];
            assert!(query(&source, 1, &types, &flag).unwrap().is_some());
            assert_eq!(source.state::<Reads>().lock().unwrap().0, 2);
            drop(source);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn unseen_output_owners_and_unavailable_reads_publish_no_unproven_window_or_repeat_work() {
        let flag = AtomicBool::new(false);
        for extra in [Extra::Orphan, Extra::Unread] {
            let (source, root) =
                dashboard_base::test_fixtures::file_source(recording(8, false, false, extra));
            let types = [BuildingInfo {
                id: CIVILIAN,
                token: Some("synthetic_type".into()),
                name: None,
                icon: None,
            }];
            assert!(query(&source, 1, &types, &flag).unwrap().is_none());
            let reads = source.state::<Reads>().lock().unwrap().0;
            assert_eq!(reads, 2);
            assert!(query(&source, 1, &types, &flag).unwrap().is_none());
            assert_eq!(source.state::<Reads>().lock().unwrap().0, reads);
            drop(source);
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
