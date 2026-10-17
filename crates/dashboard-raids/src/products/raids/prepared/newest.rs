//! A recent scan is exact when none of its raid keys needs earlier state.
use super::*;
use dashboard_base::prepared::query::{earlier, progressive};
use replay::container::TableFields;
use std::cell::RefCell;

pub(super) fn scan(
    source: &Source,
    branch: u64,
    content: Option<&Content>,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<(ScannedRaids, HourSpan)>> {
    earlier::cached(
        source,
        "raids:window",
        branch,
        source.block_windows(branch).len().saturating_sub(1),
        key,
        cancelled,
        || scan_uncached(source, branch, content, cancelled),
    )
}

fn scan_uncached(
    source: &Source,
    branch: u64,
    content: Option<&Content>,
    cancelled: &AtomicBool,
) -> io::Result<Option<(ScannedRaids, HourSpan)>> {
    let blocks: Vec<_> = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    let Some(target) = blocks.last() else {
        return Ok(None);
    };
    let start = blocks.len().saturating_sub(2);
    let scan = RefCell::new(Scan::new());
    let seen = RefCell::new(BTreeSet::new());
    let country_anchor = RefCell::new(start == 0);
    progressive::state_tail(
        source,
        branch,
        None,
        FAMILY.tables,
        &[],
        &[
            blocks[start].first_hour,
            target.first_hour,
            target.last_hour,
        ],
        &|frame, _| {
            if frame.hour >= blocks[start].first_hour && frame.hour <= target.last_hour {
                if frame.hour <= target.first_hour
                    && frame.complete::<tables::Countries>().is_ok()
                    && frame.complete::<tables::CountryLinks>().is_ok()
                {
                    *country_anchor.borrow_mut() = true;
                }
                if let Ok(rows) = frame.complete::<tables::Raids>() {
                    seen.borrow_mut().extend(
                        rows.iter()
                            .map(|row| (u64::from(row.id) << 32) | u64::from(row.reference_type)),
                    );
                }
                let mut scan = scan.borrow_mut();
                scan.format = source.replay_ref().format;
                scan.observe_changed(frame, content, None)?;
            }
            Ok(Default::default())
        },
        cancelled,
    )?;
    // Launch witnesses and the first ended victim persist by native key,
    // even across removal. A repeated key therefore needs forward history.
    if !country_anchor.into_inner() {
        return Ok(None);
    }
    let seen = seen.into_inner();
    if !seen.is_empty() {
        let prior = earlier::keys(
            source,
            "raids:keys",
            &blocks[..start],
            &[TableFields::keys(TableId::Raids)],
            |frame, facts| {
                if let Ok(rows) = frame.complete::<tables::Raids>() {
                    facts.values.extend(
                        rows.iter()
                            .map(|row| (u64::from(row.id) << 32) | u64::from(row.reference_type)),
                    );
                }
            },
            cancelled,
        )?;
        if seen.iter().any(|key| prior.values.contains(key)) {
            return Ok(None);
        }
    }
    let mut scan = scan.into_inner().finish();
    scan.episodes.retain(|episode| {
        episode
            .runs
            .iter()
            .any(|run| run.last_hour >= target.first_hour)
    });
    scan.read_hours.retain(|hour| *hour >= target.first_hour);
    Ok(Some((
        scan,
        HourSpan {
            first_hour: target.first_hour,
            last_hour: target.last_hour,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::raids::tests::{frame as recorded_frame, raid};

    fn frame(hour: i32, rows: Vec<RaidRow>) -> DecodedFrame {
        let mut frame = recorded_frame(hour, rows);
        frame.metrics.table_ns[TableId::Raids.index()] = 1;
        frame
    }
    use dashboard_base::test_fixtures::{continuation_records, file_source, replay};

    #[test]
    fn an_empty_local_raid_window_needs_no_earlier_keys() {
        let mut recorded = replay(vec![frame(100, Vec::new()), frame(101, Vec::new())]);
        for hour in [102, 104, 106] {
            recorded = recorded
                .append(&[continuation_records(
                    &recorded,
                    1,
                    vec![frame(hour, Vec::new()), frame(hour + 1, Vec::new())],
                )])
                .unwrap();
        }
        let (source, root) = file_source(recorded);
        let (_, key) = dashboard_base::prepared::files(&source, None).unwrap();
        let (actual, span) = scan(&source, 1, None, &key, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert!(actual.episodes.is_empty());
        assert_eq!((span.first_hour, span.last_hour), (106, 107));
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(&source, "raids:keys").2,
            0
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newest_raid_rows_equal_the_hourly_scan_and_only_two_full_blocks_are_read() {
        for preceding in [0, 8, 24] {
            let mut recorded = replay(vec![frame(100, Vec::new()), frame(101, Vec::new())]);
            for block in 0..preceding {
                let hour = 102 + block * 2;
                recorded = recorded
                    .append(&[continuation_records(
                        &recorded,
                        1,
                        vec![frame(hour, Vec::new()), frame(hour + 1, Vec::new())],
                    )])
                    .unwrap();
            }
            let hour = 102 + preceding * 2;
            recorded = recorded
                .append(&[continuation_records(
                    &recorded,
                    1,
                    vec![
                        frame(
                            hour,
                            vec![raid(7, native::PHASE_PREPARING, native::OUTCOME_NONE)],
                        ),
                        frame(
                            hour + 1,
                            vec![raid(7, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE)],
                        ),
                    ],
                )])
                .unwrap();
            let expected =
                crate::products::raids::scan(&recorded, 1, None, &AtomicBool::new(false)).unwrap();
            let (source, root) = file_source(recorded);
            let (_, key) = dashboard_base::prepared::files(&source, None).unwrap();
            let (actual, _) = scan(&source, 1, None, &key, &AtomicBool::new(false))
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(actual.episodes).unwrap(),
                serde_json::to_value(expected.episodes).unwrap()
            );
            let before = dashboard_base::prepared::query::frames::reads(&source);
            assert_eq!(
                before.0, 2,
                "full look-back is independent of {preceding} earlier blocks"
            );
            let (_, refused) = progressive::without_block_decodes(|| {
                scan(&source, 1, None, &key, &AtomicBool::new(false)).unwrap()
            });
            assert!(!refused);
            assert_eq!(
                dashboard_base::prepared::query::frames::reads(&source),
                before
            );
            let context = LoadContext { content: None };
            let reply = super::super::query(
                &source,
                serde_json::json!({"branchId":"1","view":"tally","tags":[],"top":null}),
                &context,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(reply.is_object());
            assert!(!reply["pending"].as_array().unwrap().is_empty());
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
