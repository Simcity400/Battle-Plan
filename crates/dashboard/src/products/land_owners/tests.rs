use crate::test_fixtures::{CAMPAIGN, ownership_frame, records, scale_frame};
use replay::{container::Replay, spool::Record};
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::TableUpdate, frame::TableView};
use std::sync::atomic::AtomicBool;

#[test]
fn land_owners_shared_changes_skip_unchanged_rosters_and_bound_one_changed_row() {
    use super::Scan;
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let format = replay_data::format::FormatVersion::default();
    let first = scale_frame(1, 100, 24);
    let work = observer_tests::scaled_work(10, |hours| {
        let mut scan = Scan::default();
        let mut tracker = FrameChangeTracker::new();
        scan.observe(&first, None, Some(&tracker.observe(&first, format)))?;
        let configured = scan.work;
        let delta = observer_tests::unchanged_frames(
            configured,
            &first,
            hours as i32,
            format,
            |frame, changes| {
                scan.observe(frame, None, Some(changes))?;
                Ok(scan.work)
            },
        )?;
        assert_eq!(delta, Work::default());
        let mut oracle = Scan::default();
        oracle.observe(&first, None, None)?;
        observer_tests::assert_reply_parity(&scan.reply(), &oracle.reply())?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(work);

    let first = ownership_frame(1, 100, 11);
    let mut next = first.clone();
    next.hour += 1;
    if let TableView::Complete(TableRows::States(rows)) = &mut *next.tables[TableId::States.index()]
    {
        rows[0].owner = 44;
    }
    let mut tracker = FrameChangeTracker::new();
    let mut scan = Scan::default();
    scan.observe(&first, None, Some(&tracker.observe(&first, format)))
        .unwrap();
    let before = scan.work;
    scan.observe(&next, None, Some(&tracker.observe(&next, format)))
        .unwrap();
    assert_eq!(scan.work.emitted - before.emitted, 1);
    assert!(scan.work.row_visits - before.row_visits <= 5);
    let mut oracle = Scan::default();
    oracle.observe(&first, None, None).unwrap();
    oracle.observe(&next, None, None).unwrap();
    observer_tests::assert_reply_parity(&scan.reply(), &oracle.reply()).unwrap();

    // A restored recording-wide scan must revisit identical rows on a lower
    // branch, so its first witness and branch order remain the oracle's.
    scan = Scan::read(scan.saved_bytes().unwrap().as_slice()).unwrap();
    tracker.invalidate();
    next.branch = 0;
    next.hour = 90;
    scan.observe(&next, None, Some(&tracker.observe(&next, format)))
        .unwrap();
    oracle.observe(&next, None, None).unwrap();
    observer_tests::assert_reply_parity(&scan.reply(), &oracle.reply()).unwrap();
}

#[test]
fn land_owners_shared_driver_work_is_flat_over_retained_alias_history() {
    use super::Scan;
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::default();
    let work = observer_tests::scaled_work(10, |history| {
        let mut scan = Scan::default();
        let mut tracker = FrameChangeTracker::new();
        let mut frame = ownership_frame(1, 100, 11);
        for index in 0..history {
            frame.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut()
                    .find(|row| row.canonical_slot == 7)
                    .unwrap()
                    .tag = format!("recorded alias {index}").into_bytes();
            }
            scan.observe(&frame, None, Some(&tracker.observe(&frame, format)))?;
        }
        assert_eq!(scan.reply().countries.len(), history);
        let before = scan.reply();
        let delta =
            observer_tests::unchanged_frames(scan.work, &frame, 10, format, |frame, changes| {
                scan.observe(frame, None, Some(changes))?;
                Ok(scan.work)
            })?;
        observer_tests::assert_reply_parity(&scan.reply(), &before)?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(work);
}

#[test]
fn land_owners_changed_country_operands_skip_state_rows_and_keep_shortened_tag_order() {
    use super::Scan;
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::CURRENT;
    let work = observer_tests::scaled_work(10, |history| {
        let mut scan = Scan::default();
        let mut tracker = FrameChangeTracker::new();
        let mut frame = ownership_frame(1, 100, 11);
        for index in 0..history {
            frame.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut()
                    .find(|row| row.canonical_slot == 7)
                    .unwrap()
                    .tag = format!("{} {index}", "x".repeat(600)).into_bytes();
            }
            scan.observe(&frame, None, Some(&tracker.observe(&frame, format)))?;
        }
        let before = scan.work;
        frame.hour += 1;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(7);
        }
        scan.observe(&frame, None, Some(&tracker.observe(&frame, format)))?;
        assert_eq!(scan.work.row_visits - before.row_visits, 4);
        assert_eq!(
            scan.work.history_visits - before.history_visits,
            0,
            "balance-only changes skip the ownership join"
        );
        assert_eq!(scan.work.emitted - before.emitted, 0);
        let reply = scan.reply();
        assert!(
            reply
                .countries
                .windows(2)
                .all(|rows| rows[0].observed_hour < rows[1].observed_hour)
        );
        let restored = Scan::read(scan.saved_bytes()?.as_slice())?;
        observer_tests::assert_reply_parity(&reply, &restored.reply())?;
        Ok(observer_tests::Work {
            row_visits: scan.work.row_visits - before.row_visits,
            ..Default::default()
        })
    })
    .unwrap();
    observer_tests::assert_flat(work);
}

#[test]
fn land_owners_link_refresh_uses_retained_owners_and_country_keys_after_resume() {
    use super::Scan;
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut read = ownership_frame(1, 100, 11);
    let mut prepared = Scan::default();
    let mut oracle = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe(&read, None, Some(&tracker.observe(&read, format)))
        .unwrap();
    oracle.observe(&read, None, None).unwrap();
    let slots = std::sync::Arc::clone(&prepared.country_inputs.as_ref().unwrap().2);
    for hour in 101..=102 {
        read.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *read.tables[TableId::Countries.index()]
        {
            rows.iter_mut()
                .find(|row| row.canonical_slot == 7)
                .unwrap()
                .tag = format!("recorded alias {hour}").into_bytes();
        }
        let before = prepared.work;
        prepared
            .observe(&read, None, Some(&tracker.observe(&read, format)))
            .unwrap();
        oracle.observe(&read, None, None).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &slots,
            &prepared.country_inputs.as_ref().unwrap().2
        ));
        assert_eq!(prepared.work.row_visits - before.row_visits, 4);
        observer_tests::assert_reply_parity(&prepared.reply(), &oracle.reply()).unwrap();
    }
    read.hour = 103;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *read.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 11)
            .unwrap()
            .canonical_slot = 8;
    }
    let before = prepared.work;
    prepared
        .observe(&read, None, Some(&tracker.observe(&read, format)))
        .unwrap();
    oracle.observe(&read, None, None).unwrap();
    assert_eq!(
        prepared.work.row_visits - before.row_visits,
        read.complete::<replay_data::rows::tables::CountryLinks>()
            .unwrap()
            .len(),
        "only changed link rows are visited"
    );
    assert!(!std::sync::Arc::ptr_eq(
        &slots,
        &prepared.country_inputs.as_ref().unwrap().2
    ));
    observer_tests::assert_reply_parity(&prepared.reply(), &oracle.reply()).unwrap();
    prepared = Scan::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
    tracker.invalidate();
    read.branch = 0;
    read.hour = 90;
    prepared
        .observe(&read, None, Some(&tracker.observe(&read, format)))
        .unwrap();
    oracle.observe(&read, None, None).unwrap();
    observer_tests::assert_reply_parity(&prepared.reply(), &oracle.reply()).unwrap();
}

#[test]
fn land_owners_include_transient_and_other_branch_owners_but_not_controllers() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(
                1,
                vec![
                    ownership_frame(1, 100, 11),
                    ownership_frame(1, 101, 44),
                    ownership_frame(1, 1000, 11),
                ],
            ),
            records(2, vec![ownership_frame(2, 90, 66)]),
        ],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["FRA", "GER", "MOD"]
    );
    let france = result
        .countries
        .iter()
        .find(|country| country.tag == "FRA")
        .unwrap();
    assert_eq!((france.id.as_str(), france.observed_hour), ("8", 101));
    assert_eq!(
        result
            .countries
            .iter()
            .find(|country| country.tag == "MOD")
            .unwrap()
            .observed_branch_id,
        "2"
    );
}

#[test]
fn land_owners_name_countries_in_the_given_language_and_fall_back_to_tags() {
    let mut frame = ownership_frame(1, 100, 44);
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 44,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 44,
            ..Default::default()
        }]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let localisation: installed_content_localisation::localisation::Localisation =
        [("FRA".to_owned(), "France".to_owned())]
            .into_iter()
            .collect();
    let result = crate::land_owners(&replay, Some(&localisation), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].name, "France");
    let untranslated = installed_content_localisation::localisation::Localisation::default();
    let result = crate::land_owners(&replay, Some(&untranslated), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].name, "FRA");
    assert!(result.warnings.iter().any(|w| w.contains("show their tag")));
}

#[test]
fn land_owners_report_dangling_owners_and_missing_country_identity() {
    for missing in ["link", "row", "tag"] {
        let mut frame = ownership_frame(1, 101, 44);
        match missing {
            "link" => {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            "row" => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            _ => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.iter_mut()
                        .find(|row| row.id == 44)
                        .unwrap()
                        .tag
                        .clear();
                }
            }
        }
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![ownership_frame(1, 100, 11), frame])],
        )
        .unwrap();
        let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            result
                .countries
                .iter()
                .map(|country| country.tag.as_str())
                .collect::<Vec<_>>(),
            ["GER"],
            "{missing}"
        );
        assert!(!result.warnings.is_empty(), "{missing}");
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![ownership_frame(1, 100, 0)])],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert!(result.countries.is_empty());
    assert!(result.warnings.is_empty());
}

#[test]
fn land_owners_keep_partial_positive_evidence_and_do_not_reuse_lost_baselines() {
    let mut partial = ownership_frame(1, 102, 66);
    *partial.tables[TableId::States.index()] = TableView::Truncated {
        rows: TableRows::States(vec![replay_data::rows::StateRow {
            id: 1,
            owner: 66,
            controller: 55,
            movement_infrastructure: 0,
            ..Default::default()
        }]),
        missing: 1,
    };
    let mut input = records(
        1,
        vec![
            ownership_frame(1, 100, 11),
            ownership_frame(1, 101, 44),
            partial,
        ],
    );
    if let Record::Frame(frame) = &mut input[3] {
        frame.tables[TableId::States.index()] = TableUpdate::Unchanged { baseline: 99 };
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["GER", "MOD"]
    );
    assert!(!result.warnings.is_empty());
}

#[test]
fn land_owner_scan_honors_cancellation_and_block_budget() {
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![ownership_frame(1, 100, 11)])],
    )
    .unwrap();
    assert_eq!(
        crate::land_owners(&replay, None, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    // The container refuses an oversized block before decompressing it; the
    // product reports the gap instead of trusting a partial catalogue.
    replay.index[0].raw_length = replay::container::MAX_BLOCK_RAW_BYTES as u64 + 1;
    let owners = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert!(owners.countries.is_empty());
    assert!(!owners.warnings.is_empty());
}

#[test]
fn land_owners_list_every_country_once_across_branches() {
    const COUNTRIES: u32 = 24;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(2, vec![scale_frame(2, 90, COUNTRIES)]),
            records(
                1,
                vec![
                    scale_frame(1, 100, COUNTRIES),
                    scale_frame(1, 101, COUNTRIES),
                ],
            ),
            records(3, vec![scale_frame(3, 80, COUNTRIES)]),
        ],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries.len(), COUNTRIES as usize);
    assert!(result.warnings.is_empty());
    // Sorted by tag, each witnessed at its first hour on the lowest branch.
    assert!(
        result
            .countries
            .windows(2)
            .all(|pair| pair[0].tag < pair[1].tag)
    );
    assert!(
        result.countries.iter().all(|country| (
            country.observed_branch_id.as_str(),
            country.observed_hour
        ) == ("1", 100))
    );
}

#[test]
fn land_owners_prepared_recording_state_matches_after_two_commits_resume_append_and_tag_change() {
    use super::Scan;
    use crate::prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture};
    let mut fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut renamed = ownership_frame(1, 102, 11);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.canonical_slot == 7)
            .unwrap()
            .tag = b"renamed / realm".to_vec();
    }
    let mut missing = ownership_frame(3, 81, 66);
    *missing.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "synthetic missing ownership".into(),
    };
    let batches = [
        vec![ownership_frame(2, 90, 11), ownership_frame(2, 91, 44)],
        vec![
            ownership_frame(1, 100, 11),
            ownership_frame(1, 101, 44),
            renamed,
        ],
        vec![ownership_frame(3, 80, 66), missing],
    ];
    let mut inputs = Vec::new();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    for (commit, frames) in batches.into_iter().enumerate() {
        for frame in &frames {
            let changes = tracker.observe(frame, replay_data::format::FormatVersion::CURRENT);
            scan.observe(frame, None, Some(&changes)).unwrap();
        }
        inputs.push(records(frames[0].branch, frames));
        let replay = Replay::from_records(CAMPAIGN.into(), &inputs).unwrap();
        scan.complete = true;
        let mut writer = if commit == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        writer
            .write_at("land-owners", 0, scan.saved_bytes().unwrap().as_slice())
            .unwrap();
        writer.commit().unwrap();
        scan = Scan::read(
            fixture
                .store
                .read(&fixture.key, "land-owners", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        tracker.invalidate();
        assert!(scan.complete);
        assert_eq!(
            scan.reply(),
            crate::land_owners(&replay, None, &cancelled).unwrap(),
            "commit {commit}"
        );
    }
    assert_eq!(
        scan.reply()
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["FRA", "GER", "MOD", "renamed / realm"]
    );
    assert!(
        scan.reply()
            .countries
            .iter()
            .filter(|country| country.tag == "GER" || country.tag == "FRA")
            .all(|country| country.observed_branch_id == "1")
    );
}
