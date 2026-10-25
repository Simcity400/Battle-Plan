use super::*;
use crate::test_fixtures::{
    CAMPAIGN, PRODUCTION_HOUR as FIRST, continuation_records, production_frame, records,
};

fn recording() -> Replay {
    let frame = |hour| {
        let mut frame = production_frame(
            hour,
            true,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, i64::from(hour - FIRST), true, false)]),
        );
        if let replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::ProductionLines(rows),
        ) = &mut *frame.tables[TableId::ProductionLines.index()]
        {
            rows.retain(|row| row.country != 21);
        }
        if let replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::ProductionLineOutput(rows),
        ) = &mut *frame.tables[TableId::ProductionLineOutput.index()]
        {
            rows.retain(|row| row.country != 21);
        }
        frame
    };
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(FIRST)])]).unwrap();
    let second = first
        .append(&[continuation_records(&first, 1, vec![frame(FIRST + 1)])])
        .unwrap();
    second
        .append(&[continuation_records(&second, 1, vec![frame(FIRST + 2)])])
        .unwrap()
}

fn restore(pages: &mut fixtures::FixturePages, replay: &Replay) -> Prepared {
    restore_branch(pages, replay, 1)
}
fn restore_branch(pages: &mut fixtures::FixturePages, replay: &Replay, branch: u64) -> Prepared {
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    for ((section, range), bytes) in &pages.pages {
        writer.write_at(section, *range, bytes.as_slice()).unwrap();
    }
    writer.commit().unwrap();
    Prepared::resume(&fixture.store, &fixture.key, replay, branch, None)
        .unwrap()
        .expect("real resume binding")
}
fn known(prepared: &Prepared, pages: &mut fixtures::FixturePages) -> BTreeSet<String> {
    let mut keys: BTreeSet<_> = prepared.native.keys().map(native_key).collect();
    private::Store::new(pages, 1)
        .visit(prepared.directory.index, "native-", &mut |key, _| {
            keys.insert(key.into());
            Ok(())
        })
        .unwrap();
    keys
}

#[test]
fn empty_countries_and_unrequested_countries_publish_the_same_coverage() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut pages, &replay, block, None, &flag)
            .unwrap();
    }
    for tag in ["GER", "ITA"] {
        let (scan, meta) = saved_sections(1, tag);
        let saved: SavedScan = pages::read(&pages.read_at(&scan, 0).unwrap().unwrap()).unwrap();
        assert_eq!(saved.last_block, last_block(&replay, 1));
        assert!(pages.read_at(&meta, 0).unwrap().is_some());
        if tag == "ITA" {
            assert!(saved.scan.lines.is_empty());
            assert_eq!(saved.scan.reads, vec![FIRST, FIRST + 1, FIRST + 2]);
            assert_eq!(
                saved.scan.line_hours,
                vec![crate::products::country_groups::HourSpan {
                    first_hour: FIRST,
                    last_hour: FIRST + 2
                }]
            );
        }
    }
    assert!(prepared.directory.coverage.complete);
}

#[test]
fn invalid_block_cursors_cannot_publish_coverage_or_completed_scans() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    assert_eq!(
        prepared
            .advance_block(&mut pages, &replay, 1, None, &flag)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(
        prepared
            .publish(&mut pages, &replay, &replay.index[1], None, &flag)
            .is_err()
    );
    assert!(pages.pages.is_empty());
    assert_eq!(prepared.directory.last_block, None);
    assert!(!prepared.directory.coverage.complete);
    prepared
        .advance_block(&mut pages, &replay, 0, None, &flag)
        .unwrap();
    prepared
        .publish(&mut pages, &replay, &replay.index[0], None, &flag)
        .unwrap();
    let prefix = pages.pages.clone();
    let previous = prepared.processed.clone();
    prepared.processed.as_mut().unwrap().length += 1;
    assert!(prepared.next_block(&replay).is_err());
    assert!(
        prepared
            .advance_block(&mut pages, &replay, 1, None, &flag)
            .is_err()
    );
    assert!(
        prepared
            .publish(
                &mut pages,
                &replay,
                replay.index.last().unwrap(),
                None,
                &flag
            )
            .is_err()
    );
    prepared.processed = previous;
    for position in [0, 2, replay.index.len()] {
        assert!(
            prepared
                .advance_block(&mut pages, &replay, position, None, &flag)
                .is_err()
        );
    }
    assert!(
        prepared
            .publish(
                &mut pages,
                &replay,
                replay.index.last().unwrap(),
                None,
                &flag
            )
            .is_err()
    );
    assert_eq!(pages.pages, prefix);
    let coverage: crate::prepared::coverage::Coverage =
        serde_json::from_slice(&pages.read_at(&coverage_section(1), 0).unwrap().unwrap()).unwrap();
    assert_eq!(coverage.last_hour, FIRST);
    assert!(!coverage.complete);
    assert_eq!(
        prepared.directory.last_block,
        Some((FIRST, FIRST, replay.index[0].offset))
    );
    for section in [saved_sections(1, "GER").0, saved_sections(1, "GER").1] {
        assert!(pages.read_at(&section, 0).unwrap().is_none());
    }
    let scan = assemble(
        &prepared.directory,
        &tag_key("GER"),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| pages.read_at(section, range),
    )
    .unwrap();
    assert_eq!(scan.reads, vec![FIRST]);
    assert_eq!(scan.unreadable_stretches, 0);
}

#[test]
fn an_observer_failure_cannot_publish_or_checkpoint_a_partial_block() {
    struct Budget(usize);
    impl Drop for Budget {
        fn drop(&mut self) {
            crate::prepared::pages::TEST_BATCH_BYTES.set(self.0);
        }
    }
    struct FailedPages<'a>(&'a mut fixtures::FixturePages);
    impl Pages for FailedPages<'_> {
        fn write_at(&mut self, _: &str, _: u64, _: &[u8]) -> io::Result<()> {
            Err(io::Error::other("fixture observer write failed"))
        }
        fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
            Err(io::Error::other("fixture observer write failed"))
        }
        fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
            self.0.read_at(section, range)
        }
    }
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    prepared
        .advance_block(&mut pages, &replay, 0, None, &flag)
        .unwrap();
    prepared
        .publish(&mut pages, &replay, &replay.index[0], None, &flag)
        .unwrap();
    let prefix = pages.pages.clone();
    let _budget = Budget(crate::prepared::pages::TEST_BATCH_BYTES.replace(1));
    let error = prepared
        .advance_block(&mut FailedPages(&mut pages), &replay, 1, None, &flag)
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture observer write failed");
    assert!(prepared.block_failed);
    assert!(
        prepared
            .publish(&mut pages, &replay, &replay.index[1], None, &flag)
            .is_err()
    );
    assert!(prepared.save(&mut pages, &flag).is_err());
    assert!(
        prepared
            .advance_block(&mut pages, &replay, 2, None, &flag)
            .is_err()
    );
    assert_eq!(pages.pages, prefix);
    assert_eq!(prepared.directory.coverage.last_hour, FIRST);
    assert!(!prepared.directory.coverage.complete);
    for section in [saved_sections(1, "GER").0, saved_sections(1, "GER").1] {
        assert!(pages.read_at(&section, 0).unwrap().is_none());
    }
}

#[test]
fn an_interleaved_branch_start_and_appended_resume_read_every_block() {
    let frame = |branch, hour| {
        let mut frame = production_frame(
            hour,
            true,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, i64::from(hour - FIRST), true, false)]),
        );
        frame.branch = branch;
        frame
    };
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(1, vec![frame(1, FIRST)]),
            records(2, vec![frame(2, FIRST)]),
        ],
    )
    .unwrap();
    assert_eq!(first.index[1].branch, 2);
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&first, 2, ["GER".into(), "ITA".into()], None);
    let mut pages = fixtures::FixturePages::default();
    assert!(
        prepared
            .advance_block(&mut pages, &first, 0, None, &flag)
            .is_err()
    );
    prepared
        .advance_block(&mut pages, &first, 1, None, &flag)
        .unwrap();
    prepared
        .publish(&mut pages, &first, &first.index[1], None, &flag)
        .unwrap();
    assert!(prepared.directory.coverage.complete);
    let mut grown = first;
    for hour in FIRST + 1..=FIRST + 2 {
        grown = grown
            .append(&[continuation_records(&grown, 1, vec![frame(1, hour)])])
            .unwrap();
        grown = grown
            .append(&[continuation_records(&grown, 2, vec![frame(2, hour)])])
            .unwrap();
    }
    prepared = restore_branch(&mut pages, &grown, 2);
    assert!(!prepared.directory.coverage.complete);
    let next = prepared.next_block(&grown).unwrap();
    assert_eq!(next, 2);
    for (position, block) in grown
        .index
        .iter()
        .enumerate()
        .skip(next)
        .filter(|(_, block)| block.branch == 2)
    {
        prepared
            .advance_block(&mut pages, &grown, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut pages, &grown, block, None, &flag)
            .unwrap();
        prepared = restore_branch(&mut pages, &grown, 2);
    }
    let mut expected_pages = fixtures::FixturePages::default();
    for tag in ["GER", "ITA"] {
        let scan = super::super::reference::scan(
            &grown,
            &branch_blocks(&grown, 2),
            tag,
            None,
            None,
            &flag,
        )
        .unwrap();
        assert_eq!(scan.reads, vec![FIRST, FIRST + 1, FIRST + 2]);
        write_saved(
            &mut expected_pages,
            &SavedScan {
                branch: 2,
                tag: tag.into(),
                pictured: false,
                visual: None,
                executable: engine_executable::executable_tables::is_initialized(),
                last_block: last_block(&grown, 2),
                scan: SavedLines::save(&scan, &flag).unwrap(),
            },
        )
        .unwrap();
        for section in [saved_sections(2, tag).0, saved_sections(2, tag).1] {
            assert_eq!(
                pages.read_at(&section, 0).unwrap(),
                expected_pages.read_at(&section, 0).unwrap(),
                "{section}"
            );
        }
        let actual = assemble(
            &prepared.directory,
            &tag_key(tag),
            None,
            recorded_language(&grown),
            &flag,
            |section, range| pages.read_at(section, range),
        )
        .unwrap();
        assert_eq!(actual.reads, vec![FIRST, FIRST + 1, FIRST + 2]);
        assert_eq!(actual.unreadable_stretches, 0);
    }
    assert_eq!(prepared.directory.coverage.last_hour, FIRST + 2);
    assert!(prepared.directory.coverage.complete);
}

#[test]
fn internal_boundary_resume_preserves_assembled_and_public_compatibility_payloads() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut uninterrupted = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
    let mut resumed = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
    let mut left = fixtures::FixturePages::default();
    let mut right = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        uninterrupted
            .advance_block(&mut left, &replay, position, None, &flag)
            .unwrap();
        uninterrupted
            .publish(&mut left, &replay, block, None, &flag)
            .unwrap();
        resumed
            .advance_block(&mut right, &replay, position, None, &flag)
            .unwrap();
        resumed
            .publish(&mut right, &replay, block, None, &flag)
            .unwrap();
        assert_equivalent(&uninterrupted, &resumed, &replay, &mut left, &mut right);
        resumed = restore(&mut right, &replay);
        assert_eq!(resumed.next_block(&replay).unwrap(), position + 1);
    }
}

#[test]
fn a_prefix_stays_readable_and_missing_pages_are_refused() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    prepared
        .advance_block(&mut pages, &replay, 0, None, &flag)
        .unwrap();
    prepared
        .publish(&mut pages, &replay, &replay.index[0], None, &flag)
        .unwrap();
    assert!(!prepared.directory.coverage.complete);
    assert!(
        pages
            .read_at(&saved_sections(1, "GER").0, 0)
            .unwrap()
            .is_none()
    );
    let result = assemble(
        &prepared.directory,
        &tag_key("GER"),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| pages.read_at(section, range),
    )
    .unwrap();
    assert_eq!(result.reads, vec![FIRST]);
    assert_eq!(result.lines[0].output.last().unwrap().last_hour, FIRST);
    pages
        .pages
        .retain(|(section, _), _| section != &history_section(1));
    assert!(
        assemble(
            &prepared.directory,
            &tag_key("GER"),
            None,
            recorded_language(&replay),
            &flag,
            |section, range| pages.read_at(section, range)
        )
        .is_err()
    );
}

#[test]
fn native_instance_reuse_is_not_joined_and_aliases_point_to_the_observed_epoch() {
    use replay_data::rows::{CountryExperienceGainRow, TableRows};
    use replay_frame::frame::TableView;
    let replay = recording();
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (offset, instance) in [(0, 7), (1, 8)] {
        let mut frame = production_frame(
            FIRST + offset,
            true,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[(5, 70, 1, true, false)]),
        );
        *frame.tables[TableId::CountryExperienceGains.index()] =
            TableView::Complete(TableRows::CountryExperienceGains(vec![
                CountryExperienceGainRow {
                    country: 9001,
                    instance,
                    ..Default::default()
                },
            ]));
        prepared.observe(&mut pages, &replay, &frame, None).unwrap();
    }
    let countries: Vec<_> = known(&prepared, &mut pages)
        .into_iter()
        .filter(|key| key.starts_with("native-7-9001-"))
        .collect();
    assert_eq!(countries.len(), 2);
    assert_ne!(countries[0], countries[1]);
    assert_eq!(
        prepared
            .directory
            .aliases
            .iter()
            .filter(|alias| alias.tag == "GER")
            .count(),
        2
    );
}

#[test]
fn completed_runs_leave_only_active_tails_and_latest_line_indexes() {
    let replay = recording();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    let flag = AtomicBool::new(false);
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut pages, &replay, block, None, &flag)
            .unwrap();
        for country in prepared
            .native
            .values()
            .chain(prepared.aliases.values().filter_map(TagCountry::projection))
        {
            assert!(
                country
                    .state
                    .result
                    .lines
                    .iter()
                    .all(|line| line.runs.len() <= 1 && line.output.len() <= 1)
            );
            assert!(country.state.result.reads.is_empty());
        }
    }
    let result = assemble(
        &prepared.directory,
        &tag_key("GER"),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| pages.read_at(section, range),
    )
    .unwrap();
    assert_eq!(result.lines[0].output.len(), 3);
    assert_eq!(result.lines[0].runs.len(), 1);
    assert_eq!(result.lines[0].runs[0].last_hour, FIRST + 2);
}

#[test]
fn every_recorded_native_country_is_admitted_without_a_fixed_roster() {
    let replay = recording();
    let frame = crate::test_fixtures::scale_frame(1, FIRST, 1501);
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, [], None);
    prepared.observe(&mut pages, &replay, &frame, None).unwrap();
    assert_eq!(prepared.native.len(), 1501);
    assert_eq!(prepared.aliases.len(), 1501);
    assert!(
        prepared
            .aliases
            .values()
            .all(|alias| matches!(alias, TagCountry::Native(_)))
    );
}

#[test]
fn renamed_and_reused_aliases_resume_with_the_literal_tag_fold_bytes() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let frames: Vec<_> = [(9001, "GER"), (9001, "ALT"), (9002, "GER")]
        .into_iter()
        .enumerate()
        .map(|(offset, (id, tag))| {
            let mut frame = production_frame(
                FIRST + offset as i32,
                true,
                true,
                id,
                &[(5, 70, 8, 0)],
                Some(&[(5, 70, offset as i64, true, false)]),
            );
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            countries
                .iter_mut()
                .find(|country| country.id == id)
                .unwrap()
                .tag = tag.as_bytes().to_vec();
            frame
        })
        .collect();
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frames[0].clone()])]).unwrap();
    for frame in &frames[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, vec![frame.clone()])])
            .unwrap();
    }
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();

    for (position, _) in frames.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut pages, &replay, &replay.index[position], None, &flag)
            .unwrap();
        prepared = restore(&mut pages, &replay);
    }
    for (tag, hour, expected) in [
        ("GER", FIRST, Some(9001)),
        ("GER", FIRST + 1, None),
        ("ALT", FIRST + 1, Some(9001)),
        ("GER", FIRST + 2, Some(9002)),
    ] {
        assert_eq!(
            read_country_at(&prepared.directory, tag, hour, |section, range| pages
                .read_at(section, range))
            .unwrap()
            .map(|country| country.id),
            expected
        );
    }
    let literal = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "GER",
        None,
        None,
        &flag,
    )
    .unwrap();
    let saved: SavedScan = pages::read(
        &pages
            .read_at(&saved_sections(1, "GER").0, 0)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&saved.scan).unwrap(),
        serde_json::to_vec(&SavedLines::save(&literal, &flag).unwrap()).unwrap()
    );
}

#[test]
fn late_or_missing_instance_evidence_does_not_create_a_country_lifetime() {
    use replay_data::rows::{CountryExperienceGainRow, TableRows};
    use replay_frame::frame::TableView;
    let replay = recording();
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (offset, instance) in [None, Some(7), None, Some(7), Some(8)]
        .into_iter()
        .enumerate()
    {
        let mut frame = production_frame(
            FIRST + offset as i32,
            true,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[]),
        );
        if let Some(instance) = instance {
            *frame.tables[TableId::CountryExperienceGains.index()] =
                TableView::Complete(TableRows::CountryExperienceGains(vec![
                    CountryExperienceGainRow {
                        country: 9001,
                        instance,
                        ..Default::default()
                    },
                ]));
        }
        prepared.observe(&mut pages, &replay, &frame, None).unwrap();
        assert_eq!(
            known(&prepared, &mut pages)
                .iter()
                .filter(|key| key.starts_with("native-7-9001-"))
                .count(),
            if offset == 4 { 2 } else { 1 }
        );
    }
}

#[test]
fn a_recorded_native_instance_keeps_its_lifetime_when_it_returns() {
    use replay_data::rows::{CountryExperienceGainRow, TableRows};
    use replay_frame::frame::TableView;
    let replay = recording();
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for offset in 0..3 {
        let mut frame = production_frame(
            FIRST + offset,
            true,
            true,
            9001,
            &[(5, 70, 8, 0)],
            Some(&[]),
        );
        *frame.tables[TableId::CountryExperienceGains.index()] =
            TableView::Complete(TableRows::CountryExperienceGains(vec![
                CountryExperienceGainRow {
                    country: 9001,
                    instance: 7,
                    ..Default::default()
                },
            ]));
        if offset == 1 {
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            countries.retain(|country| country.id != 9001);
        }
        prepared.observe(&mut pages, &replay, &frame, None).unwrap();
    }
    assert_eq!(
        prepared
            .native
            .keys()
            .filter(|country| country.id == 9001)
            .count(),
        1
    );
    let country = prepared
        .native
        .iter()
        .find(|(country, _)| country.id == 9001)
        .unwrap()
        .1;
    assert_eq!(country.state.result.reads, vec![FIRST, FIRST + 2]);
}

#[test]
fn private_spills_preserve_completed_bytes_and_page_alias_history() {
    struct Budget(usize);
    impl Drop for Budget {
        fn drop(&mut self) {
            crate::prepared::pages::TEST_BATCH_BYTES.set(self.0);
        }
    }
    let replay = recording();
    let cancelled = AtomicBool::new(false);
    let prepare = |budget| {
        let _budget = Budget(crate::prepared::pages::TEST_BATCH_BYTES.replace(budget));
        let mut prepared = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
        let mut pages = fixtures::FixturePages::default();
        for (position, block) in replay.index.iter().enumerate() {
            prepared
                .advance_block(&mut pages, &replay, position, None, &cancelled)
                .unwrap();
            if position == 0 {
                assert!(pages.read_at(&coverage_section(1), 0).unwrap().is_none());
            }
            prepared
                .publish(&mut pages, &replay, block, None, &cancelled)
                .unwrap();
            assert!(prepared.directory.aliases.len() <= 2);
        }
        (prepared, pages)
    };
    let (ordinary, mut left) = prepare(usize::MAX);
    let (spilled, mut right) = prepare(1);
    assert!(spilled.directory.pages > ordinary.directory.pages);
    for tag in ["GER", "ITA"] {
        let (scan, meta) = saved_sections(1, tag);
        for section in [scan, meta] {
            assert_eq!(
                left.read_at(&section, 0).unwrap(),
                right.read_at(&section, 0).unwrap(),
                "{section}"
            );
        }
        for hour in FIRST..=FIRST + 2 {
            assert_eq!(
                read_country_at(&ordinary.directory, tag, hour, |section, range| left
                    .read_at(section, range))
                .unwrap(),
                read_country_at(&spilled.directory, tag, hour, |section, range| right
                    .read_at(section, range))
                .unwrap()
            );
        }
    }
}

#[test]
fn non_flushing_continuation_keeps_pending_records_before_observing_more() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut uninterrupted = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
    let mut resumed = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
    let mut left = fixtures::FixturePages::default();
    let mut right = fixtures::FixturePages::default();
    for position in 0..replay.index.len() {
        uninterrupted
            .advance_block(&mut left, &replay, position, None, &flag)
            .unwrap();
        resumed
            .advance_block(&mut right, &replay, position, None, &flag)
            .unwrap();
        if position + 1 < replay.index.len() {
            let block = &replay.index[position];
            resumed.directory.last_block = Some((block.first_hour, block.last_hour, block.offset));
            pages::write(&mut right, &directory_section(1), 0, &resumed.directory).unwrap();
            resumed.save(&mut right, &flag).unwrap();
            resumed = restore(&mut right, &replay);
            assert!(
                resumed
                    .native
                    .values()
                    .any(|country| country.state.pending_bytes != 0)
            );
        }
    }
    let block = replay.index.last().unwrap();
    uninterrupted
        .publish(&mut left, &replay, block, None, &flag)
        .unwrap();
    resumed
        .publish(&mut right, &replay, block, None, &flag)
        .unwrap();
    assert_equivalent(&uninterrupted, &resumed, &replay, &mut left, &mut right);
}

fn replay_frames(frames: Vec<DecodedFrame>) -> Replay {
    let mut frames = frames.into_iter();
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frames.next().unwrap()])]).unwrap();
    for frame in frames {
        replay = replay
            .append(&[continuation_records(&replay, 1, vec![frame])])
            .unwrap();
    }
    replay
}

fn authored(
    hour: i32,
    lines: &[(u64, u64, i32, i64)],
    output: &[(u64, u64, i64, bool, bool)],
) -> DecodedFrame {
    use replay_data::rows::{
        IndustrialOrganisationDetailsRow, IndustrialOrganisationRow, TableRows, VariantStatsRow,
    };
    use replay_frame::frame::TableView;
    let mut frame = production_frame(hour, true, true, 9001, lines, Some(output));
    *frame.tables[TableId::VariantStats.index()] = TableView::Complete(TableRows::VariantStats(
        (70..=72)
            .map(|variant| VariantStatsRow {
                variant,
                organisation: variant + 9,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(
            (79..=81)
                .map(|organisation| IndustrialOrganisationRow {
                    organisation,
                    country: 11,
                    owner: 11,
                    token: 602,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::IndustrialOrganisationDetails.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationDetails(
            (79..=81)
                .map(|organisation| IndustrialOrganisationDetailsRow {
                    organisation,
                    country: 11,
                    ..Default::default()
                })
                .collect(),
        ));
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    use replay_data::rows::{
        CountryProductionModifiersRow, LexicalTokenRow, ModifierBreakdownNode,
        ModifierDefinitionRow, ProductionLineModifiersRow, ProductionModifierTree,
    };
    let mut keys: Vec<_> = engine_executable::modifier_keys::all().collect();
    keys.sort_by_key(|key| key.index);
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            keys.iter()
                .map(|key| ModifierDefinitionRow {
                    index: key.index,
                    token: 1_000_000 + key.index,
                    ..Default::default()
                })
                .collect(),
        ));
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    tokens.extend(keys.iter().map(|key| LexicalTokenRow {
        token: 1_000_000 + key.index,
        literal: key.name.as_bytes().to_vec(),
    }));
    tokens.sort_by_key(|row| row.token);
    let trees = replay_derive::fixed_layout::PRODUCTION_MODIFIERS
        .keys
        .iter()
        .map(|key| ProductionModifierTree {
            key: *key,
            failure: vec![],
            nodes: vec![ModifierBreakdownNode {
                value: i64::from(hour - FIRST),
                name: b"Synthetic total".to_vec(),
                children: 0,
            }],
        })
        .collect();
    *frame.tables[TableId::CountryProductionModifiers.index()] =
        TableView::Complete(TableRows::CountryProductionModifiers(vec![
            CountryProductionModifiersRow {
                country: 9001,
                trees,
                ..Default::default()
            },
        ]));
    let TableView::Complete(TableRows::ProductionLines(rows)) =
        &mut *frame.tables[TableId::ProductionLines.index()]
    else {
        unreachable!()
    };
    rows.iter_mut()
        .filter(|row| row.country == 11)
        .for_each(|row| row.country = 9001);
    let modifiers = rows
        .iter()
        .filter(|row| row.country == 9001)
        .map(|row| ProductionLineModifiersRow {
            line: row.handle,
            country: row.country,
            variant: row.variant,
            organisation: row.mio,
            kind: row.kind,
            active_factories: row.active_factories,
            ..Default::default()
        })
        .collect();
    *frame.tables[TableId::ProductionLineModifiers.index()] =
        TableView::Complete(TableRows::ProductionLineModifiers(modifiers));
    frame.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 1;
    frame.metrics.table_ns[TableId::ProductionLineModifiers.index()] = 1;
    frame
}

fn public_payloads(source: &Source, origin: Option<i32>) -> Vec<serde_json::Value> {
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    vec![
        serde_json::to_value(
            ProductionLines::load(
                source,
                &ProductionLinesParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    exact: Some(true),
                },
                &context,
                &flag,
            )
            .unwrap(),
        )
        .unwrap(),
        serde_json::to_value(
            ProductionLineDetail::load(
                source,
                &ProductionDetailParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    window: FIRST.div_euclid(DETAIL_HOURS),
                },
                &context,
                &flag,
            )
            .unwrap(),
        )
        .unwrap(),
        serde_json::to_value(
            ProductionLineBreakdown::load(
                source,
                &ProductionBreakdownParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    key: "5#1".into(),
                    group: "output".into(),
                    origin_hour: origin,
                },
                &context,
                &flag,
            )
            .unwrap(),
        )
        .unwrap(),
    ]
}

#[test]
fn independent_v5_and_public_payloads_keep_same_hour_organisation_versions_delayed_output_and_reuse()
 {
    let lines = &[(5, 70, 8, 0), (6, 71, 2, 0), (7, 72, 1, 0)];
    let replay = replay_frames(vec![
        authored(FIRST, lines, &[(5, 70, 1, true, false)]),
        authored(FIRST + 1, &lines[1..], &[(5, 70, 2, true, true)]),
        authored(FIRST + 2, &lines[1..], &[(5, 70, 3, true, true)]),
        authored(FIRST + 3, lines, &[(5, 70, 1, true, false)]),
    ]);
    let flag = AtomicBool::new(false);
    let expected = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "GER",
        None,
        None,
        &flag,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &expected.modifiers["5#1"][0].country,
        &expected.modifiers["6#1"][0].country
    ));
    assert!(Arc::ptr_eq(
        &expected.modifiers["6#1"][0].country,
        &expected.modifiers["7#1"][0].country
    ));
    for id in 79..=81 {
        assert_eq!(expected.organisations[&id][0].first_hour, FIRST);
        assert_eq!(expected.organisations[&id][1].first_hour, FIRST);
        assert!(expected.organisations[&id][0].state.is_none());
        assert!(expected.organisations[&id][1].state.is_some());
    }
    let mut baseline = fixtures::FixturePages::default();
    write_saved(
        &mut baseline,
        &SavedScan {
            branch: 1,
            tag: "GER".into(),
            pictured: false,
            visual: None,
            executable: engine_executable::executable_tables::is_initialized(),
            last_block: last_block(&replay, 1),
            scan: SavedLines::save(&expected, &flag).unwrap(),
        },
    )
    .unwrap();
    for (spill, checkpoint) in [(false, false), (true, false), (false, true)] {
        let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
        let mut pages = fixtures::FixturePages::default();
        for (position, block) in replay.index.iter().enumerate() {
            prepared
                .advance_block(&mut pages, &replay, position, None, &flag)
                .unwrap();
            if spill {
                prepared.flush(&mut pages, &flag).unwrap();
            }
            if checkpoint && position + 1 < replay.index.len() {
                prepared.directory.last_block =
                    Some((block.first_hour, block.last_hour, block.offset));
                prepared.save(&mut pages, &flag).unwrap();
                prepared = restore(&mut pages, &replay);
            }
        }
        prepared
            .publish(
                &mut pages,
                &replay,
                replay.index.last().unwrap(),
                None,
                &flag,
            )
            .unwrap();
        for section in [saved_sections(1, "GER").0, saved_sections(1, "GER").1] {
            assert_eq!(
                pages.read_at(&section, 0).unwrap(),
                baseline.read_at(&section, 0).unwrap(),
                "{section}: spill={spill}, checkpoint={checkpoint}"
            );
        }
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "independent-production".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(Replay::open(replay.bytes().to_vec()).unwrap()),
            &flag,
        )
        .unwrap();
        *source.state::<fixtures::FixturePages>().lock().unwrap() = pages;
        let actual = public_payloads(&source, None);
        let reference = super::super::reference::with(&source, expected.clone(), || {
            public_payloads(&source, None)
        });
        assert_eq!(actual, reference);
    }
}

fn assert_reference_publication_modes(replay: &Replay, verify: impl Fn(&ScannedLines)) {
    let flag = AtomicBool::new(false);
    let expected =
        super::super::reference::scan(replay, &branch_blocks(replay, 1), "GER", None, None, &flag)
            .unwrap();
    verify(&expected);
    let mut baseline = fixtures::FixturePages::default();
    write_saved(
        &mut baseline,
        &SavedScan {
            branch: 1,
            tag: "GER".into(),
            pictured: false,
            visual: None,
            executable: engine_executable::executable_tables::is_initialized(),
            last_block: last_block(replay, 1),
            scan: SavedLines::save(&expected, &flag).unwrap(),
        },
    )
    .unwrap();
    for (spill, checkpoint) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut prepared = Prepared::new(replay, 1, ["GER".into()], None);
        let mut pages = fixtures::FixturePages::default();
        for (position, block) in replay.index.iter().enumerate() {
            prepared
                .advance_block(&mut pages, replay, position, None, &flag)
                .unwrap();
            if spill {
                prepared.flush(&mut pages, &flag).unwrap();
            }
            if checkpoint && position + 1 < replay.index.len() {
                assert!(
                    pages
                        .read_at(&saved_sections(1, "GER").0, 0)
                        .unwrap()
                        .is_none()
                );
                if !spill {
                    assert!(
                        prepared.native.values().any(|country| !country
                            .state
                            .result
                            .reads
                            .is_empty())
                    );
                }
                prepared.directory.last_block =
                    Some((block.first_hour, block.last_hour, block.offset));
                prepared.save(&mut pages, &flag).unwrap();
                prepared = restore(&mut pages, replay);
                assert_eq!(prepared.next_block(replay).unwrap(), position + 1);
            }
        }
        prepared
            .publish(
                &mut pages,
                replay,
                replay.index.last().unwrap(),
                None,
                &flag,
            )
            .unwrap();
        let actual = assemble(
            &prepared.directory,
            &tag_key("GER"),
            None,
            recorded_language(replay),
            &flag,
            |section, range| pages.read_at(section, range),
        )
        .unwrap();
        verify(&actual);
        for section in [saved_sections(1, "GER").0, saved_sections(1, "GER").1] {
            assert_eq!(
                pages.read_at(&section, 0).unwrap(),
                baseline.read_at(&section, 0).unwrap(),
                "{section}: spill={spill}, checkpoint={checkpoint}"
            );
        }
        let source = fixture_source(replay, pages);
        for origin in [
            None,
            Some(FIRST),
            Some(replay.index.last().unwrap().last_hour),
        ] {
            let actual = public_payloads(&source, origin);
            let reference = super::super::reference::with(&source, expected.clone(), || {
                public_payloads(&source, origin)
            });
            assert_eq!(
                actual, reference,
                "spill={spill}, checkpoint={checkpoint}, origin={origin:?}"
            );
        }
    }
}

fn replay_segments(first: Vec<DecodedFrame>, later: Vec<DecodedFrame>) -> Replay {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    replay
        .append(&[continuation_records(&replay, 1, later)])
        .unwrap()
}

#[test]
fn unchanged_organisations_reseed_after_naming_prerequisites_recover_across_nonflushing_resume() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    for prerequisite in [
        TableId::LexicalTokens,
        TableId::CountryLinks,
        TableId::Countries,
    ] {
        let mut frames: Vec<_> = (0..5)
            .map(|offset| {
                let mut frame = authored(FIRST + offset, &[(5, 70, 8, 0)], &[]);
                if (1..4).contains(&offset) {
                    frame.metrics.table_ns.fill(0);
                }
                if offset > 0 {
                    let TableView::Complete(TableRows::IndustrialOrganisationDetails(rows)) =
                        &mut *frame.tables[TableId::IndustrialOrganisationDetails.index()]
                    else {
                        unreachable!()
                    };
                    rows[0].state.funds_raw = 42;
                }
                if (1..=2).contains(&offset) {
                    *frame.tables[prerequisite.index()] = TableView::Unavailable {
                        reason: "synthetic naming prerequisite".into(),
                    };
                }
                if offset >= 3 {
                    let TableView::Complete(TableRows::LexicalTokens(rows)) =
                        &mut *frame.tables[TableId::LexicalTokens.index()]
                    else {
                        unreachable!()
                    };
                    rows.iter_mut()
                        .find(|row| row.token == 602)
                        .unwrap()
                        .literal = b"recovered_acme".to_vec();
                }
                frame
            })
            .collect();
        assert_eq!(
            frames[0]
                .complete::<tables::IndustrialOrganisations>()
                .unwrap(),
            frames[3]
                .complete::<tables::IndustrialOrganisations>()
                .unwrap()
        );
        let later = frames.split_off(2);
        let replay = replay_segments(frames, later);
        assert_reference_publication_modes(&replay, |scan| {
            let recovered = scan.organisations[&79].last().unwrap();
            assert_eq!(recovered.first_hour, FIRST + 4, "{prerequisite:?}");
            assert_eq!(recovered.token, "recovered_acme");
            assert_eq!(recovered.state.as_ref().unwrap().funds_raw, "42");
        });
    }
}

#[test]
fn country_owner_dependencies_follow_deleted_replaced_moved_and_aliased_rows_after_resume() {
    use replay_data::rows::{CountryLinkRow, TableRows};
    use replay_frame::frame::TableView;
    struct Names;
    impl Texts for Names {
        fn has(&self, key: &str) -> bool {
            ["ITA_acme", "SPA_acme", "acme"].contains(&key)
        }
        fn text(&self, key: &str) -> Option<String> {
            self.has(key).then(|| key.to_owned())
        }
    }
    for case in [
        "deleted",
        "replaced",
        "alias",
        "moved",
        "missing",
        "invalid-tag",
        "unordered-slots",
    ] {
        let frames: Vec<_> = (0..5)
            .map(|offset| {
                let mut frame = authored(FIRST + offset, &[(5, 70, 8, 0)], &[]);
                if (1..3).contains(&offset) {
                    frame.metrics.table_ns.fill(0);
                }
                let TableView::Complete(TableRows::IndustrialOrganisations(rows)) =
                    &mut *frame.tables[TableId::IndustrialOrganisations.index()]
                else {
                    unreachable!()
                };
                for row in rows {
                    row.owner = if case == "alias" { 333 } else { 21 };
                }
                let TableView::Complete(TableRows::LexicalTokens(rows)) =
                    &mut *frame.tables[TableId::LexicalTokens.index()]
                else {
                    unreachable!()
                };
                rows.iter_mut()
                    .find(|row| row.token == 602)
                    .unwrap()
                    .literal = b"acme".to_vec();
                if case == "alias" {
                    let TableView::Complete(TableRows::CountryLinks(rows)) =
                        &mut *frame.tables[TableId::CountryLinks.index()]
                    else {
                        unreachable!()
                    };
                    rows.push(CountryLinkRow {
                        id: 333,
                        canonical_slot: 0,
                    });
                    rows.sort_by_key(|row| row.id);
                }
                if case == "unordered-slots" {
                    let TableView::Complete(TableRows::CountryLinks(rows)) =
                        &mut *frame.tables[TableId::CountryLinks.index()]
                    else {
                        unreachable!()
                    };
                    rows.iter_mut()
                        .find(|row| row.id == 21)
                        .unwrap()
                        .canonical_slot = 12;
                }
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                else {
                    unreachable!()
                };
                if case == "unordered-slots" {
                    rows[0].canonical_slot = 12;
                }
                if case == "missing" && offset < 2 {
                    rows.retain(|row| row.id != 21);
                } else if (2..=3).contains(&offset) {
                    match case {
                        "deleted" => rows.retain(|row| row.id != 21),
                        "replaced" => {
                            rows[0].id = 31;
                            rows[0].tag = b"SPA".to_vec();
                        }
                        "alias" | "unordered-slots" => rows[0].tag = b"SPA".to_vec(),
                        "moved" => rows[0].canonical_slot = 1,
                        "invalid-tag" => rows[0].tag = vec![255],
                        "missing" => {}
                        _ => unreachable!(),
                    }
                }
                frame
            })
            .collect();
        let replay = replay_segments(vec![frames[0].clone()], frames[1..].to_vec());
        assert_reference_publication_modes(&replay, |scan| {
            for (hour, tag) in [
                (FIRST, if case == "missing" { None } else { Some("ITA") }),
                (
                    FIRST + 3,
                    match case {
                        "deleted" | "moved" | "invalid-tag" => None,
                        "replaced" | "alias" | "unordered-slots" => Some("SPA"),
                        _ => Some("ITA"),
                    },
                ),
                (FIRST + 4, Some("ITA")),
            ] {
                let raw = scan.organisations[&79]
                    .iter()
                    .rev()
                    .find(|raw| raw.first_hour <= hour)
                    .unwrap();
                assert_eq!(raw.owner_tag.as_deref(), tag, "{case} at {hour}");
                assert_eq!(raw.owner_unknown, tag.is_none(), "{case} at {hour}");
                let name = tag.map(|tag| format!("{tag}_acme"));
                assert_eq!(organisation_name(raw, Some(&Names)), name);
                let icon = organisation_icon_with_token(raw, true, |_| true);
                if let Some(tag) = tag {
                    assert_eq!(icon.unwrap(), Some(format!("GFX_{tag}_acme")));
                } else {
                    assert_eq!(
                        icon.unwrap_err(),
                        "the recorded owner identity is unavailable for fallback icon selection"
                    );
                    assert!(organisation_icon(raw, |_| true).is_none());
                    let explicit = RawOrganisation {
                        icon: "GFX_explicit".into(),
                        ..raw.clone()
                    };
                    assert_eq!(
                        organisation_icon(&explicit, |_| true).as_deref(),
                        Some("GFX_explicit")
                    );
                }
            }
        });
    }
}

#[test]
fn full_naming_snapshots_remove_disappeared_token_and_policy_operands() {
    use crate::prepared::frame_changes::FrameChanges;
    use replay_data::rows::{
        IndustrialOrganisationPoliciesRow, IndustrialOrganisationPolicy, TableRows,
    };
    use replay_frame::frame::TableView;
    let mut first = authored(FIRST, &[(5, 70, 8, 0)], &[]);
    let TableView::Complete(TableRows::IndustrialOrganisationDetails(rows)) =
        &mut *first.tables[TableId::IndustrialOrganisationDetails.index()]
    else {
        unreachable!()
    };
    rows[0].state.policy = 602;
    *first.tables[TableId::IndustrialOrganisationPolicies.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationPolicies(vec![
            IndustrialOrganisationPoliciesRow {
                country: 11,
                policies: vec![IndustrialOrganisationPolicy {
                    token: 602,
                    name: b"Synthetic policy".to_vec(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    let mut now = first.clone();
    now.hour += 1;
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *now.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.token != 602);
    *now.tables[TableId::IndustrialOrganisationPolicies.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationPolicies(vec![]));
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first.clone(), now.clone()])],
    )
    .unwrap();
    let expected = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "GER",
        None,
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut world = World::default();
    let input = world
        .extract(&replay, &first, None, &BTreeSet::new())
        .unwrap();
    assert_eq!(
        input.organisations[&79]
            .state
            .as_ref()
            .unwrap()
            .policy_name
            .as_deref(),
        Some("Synthetic policy")
    );
    let mut changes = FrameChanges::alone(&now, replay.format);
    changes.rows_changed.fill(false);
    for table in [
        TableId::LexicalTokens,
        TableId::IndustrialOrganisationPolicies,
    ] {
        changes.rows_changed[table.index()] = true;
    }
    let input = world
        .extract_cached(&replay, &now, None, &BTreeSet::new(), Some(input), &changes)
        .unwrap();
    let actual = &input.organisations[&79];
    let expected = expected.organisations[&79].last().unwrap();
    assert_eq!(actual.token, expected.token);
    assert_eq!(actual.state.as_ref().unwrap().policy_name, None);
    assert_eq!(
        actual.state.as_ref().unwrap().policy_name,
        expected.state.as_ref().unwrap().policy_name
    );
    assert_eq!(
        actual.state.as_ref().unwrap().policy_token,
        expected.state.as_ref().unwrap().policy_token
    );
    assert_reference_publication_modes(&replay, |_| {});
}

#[test]
fn unknown_line_kinds_do_not_demand_later_design_reads_with_spill_and_resume() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let mut frames: Vec<_> = (0..4)
        .map(|offset| {
            let mut frame = authored(FIRST + offset, &[(5, 70, 8, 0), (6, 71, 2, 0)], &[]);
            let TableView::Complete(TableRows::ProductionLines(rows)) =
                &mut *frame.tables[TableId::ProductionLines.index()]
            else {
                unreachable!()
            };
            rows.iter_mut().find(|row| row.handle == 6).unwrap().kind = 255;
            if offset > 0 {
                frame.metrics.table_ns.fill(0);
                frame.metrics.table_ns[TableId::VariantStats.index()] = 1;
                frame.tables[TableId::VariantStats.index()] = frame.tables
                    [TableId::VariantStats.index()]
                .clone()
                .with_sampled_at(Some(FIRST + offset));
                let TableView::Complete(TableRows::VariantStats(rows)) =
                    &mut *frame.tables[TableId::VariantStats.index()]
                else {
                    unreachable!()
                };
                rows.iter_mut()
                    .find(|row| row.variant == 71)
                    .unwrap()
                    .organisation = 81;
            }
            frame
        })
        .collect();
    let later = frames.split_off(1);
    let replay = replay_segments(frames, later);
    assert_reference_publication_modes(&replay, |scan| {
        assert_eq!(scan.unknown_kinds, 1);
        assert!(!scan.lines.iter().any(|line| line.handle == 6));
        assert!(!scan.designs.contains_key(&71));
        assert!(!scan.organisations.contains_key(&81));
        assert!(
            scan.designs[&70]
                .iter()
                .any(|read| read.read_hour == Some(FIRST + 3))
        );
    });
}

#[test]
fn an_unresolved_link_is_unknown_and_recovers_the_same_origin_lifetime_after_real_resume() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let mut middle = authored(FIRST + 1, &[(5, 70, 8, 0)], &[(5, 70, 2, true, false)]);
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *middle.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.retain(|row| row.id != 9001);
    let replay = replay_frames(vec![
        authored(FIRST, &[(5, 70, 8, 0)], &[]),
        middle,
        authored(FIRST + 2, &[(5, 70, 8, 0)], &[]),
    ]);
    let flag = AtomicBool::new(false);
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut pages, &replay, block, None, &flag)
            .unwrap();
        prepared = restore(&mut pages, &replay);
    }
    let before = read_country_at(&prepared.directory, "GER", FIRST, |section, range| {
        pages.read_at(section, range)
    })
    .unwrap()
    .unwrap();
    let after = read_country_at(&prepared.directory, "GER", FIRST + 2, |section, range| {
        pages.read_at(section, range)
    })
    .unwrap()
    .unwrap();
    assert_eq!(before, after);
    assert_eq!(before.epoch, 1);
    assert_eq!(before.instance, None);
    assert_eq!(
        known(&prepared, &mut pages)
            .iter()
            .filter(|key| key.starts_with("native-7-9001-"))
            .count(),
        1
    );
    let actual = assemble(
        &prepared.directory,
        &native_key(&before),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| pages.read_at(section, range),
    )
    .unwrap();
    assert_eq!(actual.unread_output_reads, 1);
    assert!(
        actual
            .unread_output
            .iter()
            .any(|span| span.first_hour <= FIRST + 1 && FIRST + 1 <= span.last_hour)
    );
    assert_eq!(actual.reads, vec![FIRST, FIRST + 2]);
    let expected = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "GER",
        None,
        Some((7, 9001)),
        &flag,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&SavedLines::save(&actual, &flag).unwrap()).unwrap(),
        serde_json::to_vec(&SavedLines::save(&expected, &flag).unwrap()).unwrap()
    );
    assert_eq!(
        actual.modifiers["5#1"]
            .iter()
            .map(|run| run.first_hour)
            .collect::<Vec<_>>(),
        vec![FIRST, FIRST + 2]
    );
    let source = fixture_source(&replay, pages);
    for origin in [FIRST, FIRST + 2] {
        let actual = public_payloads(&source, Some(origin));
        let reference = super::super::reference::with(&source, expected.clone(), || {
            public_payloads(&source, Some(origin))
        });
        assert_eq!(actual, reference, "origin={origin}");
        assert_eq!(actual[2]["reads"], serde_json::json!([FIRST, FIRST + 2]));
        assert!(
            actual[0]["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning.as_str().unwrap().contains("could not be read"))
        );
    }
}

#[test]
fn closed_handles_and_retired_lifetimes_are_only_private_pages_not_checkpoint_rosters() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, [], None);
    let mut pages = fixtures::FixturePages::default();
    let mut baseline = None;
    for offset in 0..80 {
        let mut frame = production_frame(
            FIRST + offset,
            true,
            true,
            if offset < 40 {
                9001
            } else {
                9001 + offset as u32
            },
            &[(100 + offset as u64, 70, 8, 0)],
            Some(&[]),
        );
        if offset > 40 {
            use replay_data::rows::TableRows;
            use replay_frame::frame::TableView;
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            countries.last_mut().unwrap().tag = format!("X{offset}").into_bytes();
        }
        prepared.observe(&mut pages, &replay, &frame, None).unwrap();
        prepared.flush(&mut pages, &flag).unwrap();
        prepared.save(&mut pages, &flag).unwrap();
        assert!(prepared.native.len() <= 2);
        assert!(prepared.aliases.len() <= 2);
        for country in prepared.native.values() {
            assert!(
                country.state.latest.len() <= country.state.open.len() + country.state.broken.len()
            );
            assert_eq!(country.state.appearances.len(), country.state.latest.len());
            assert_eq!(country.state.result.lines.len(), country.state.latest.len());
        }
        let snapshot = prepared
            .native
            .iter_mut()
            .find(|(key, _)| key.slot == 7)
            .unwrap()
            .1
            .snapshot(&flag)
            .unwrap();
        let size = serde_json::to_vec(&snapshot).unwrap().len();
        if offset == 10 {
            baseline = Some(size);
        }
        if offset >= 10 {
            assert!(
                size <= baseline.unwrap() + 512,
                "closed history leaked into the checkpoint: {size}"
            );
        }
    }
    assert!(known(&prepared, &mut pages).len() > prepared.native.len());
}

#[test]
fn a_boundary_without_quiet_hours_keeps_unchanged_identities_and_census_operands() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::{FrameEncoder, TableView};
    struct Budget(usize);
    impl Drop for Budget {
        fn drop(&mut self) {
            crate::prepared::pages::TEST_BATCH_BYTES.set(self.0);
        }
    }
    for boundary in ["flush", "save", "pressure"] {
        let replay = recording();
        let flag = AtomicBool::new(false);
        let mut prepared = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
        let mut pages = fixtures::FixturePages::default();
        let mut frame = production_frame(
            FIRST,
            true,
            true,
            9001,
            &[(5, 70, 8, 0), (6, 71, 3, 0)],
            Some(&[]),
        );
        let mut encoder = FrameEncoder::new();
        let updates = encoder.encode(frame.clone(), true).unwrap().into();
        let changes = prepared.world.recorded_changes(&replay, &frame, updates);
        let _budget = Budget(crate::prepared::pages::TEST_BATCH_BYTES.replace(
            if boundary == "pressure" { 1 } else { usize::MAX },
        ));
        prepared
            .observe_changed_into(&mut pages, &replay, &frame, None, &flag, &changes)
            .unwrap();
        assert!(prepared.quiet.is_none());
        match boundary {
            "flush" => prepared.flush(&mut pages, &flag).unwrap(),
            "save" => prepared.save(&mut pages, &flag).unwrap(),
            "pressure" => assert!(prepared.directory.pages > 0),
            _ => unreachable!(),
        }
        let input = prepared.last_input.as_ref().expect(boundary);
        assert_eq!(
            input.countries,
            Some(BTreeMap::from([
                ("GER".into(), Country::Found(7, 9001)),
                ("ITA".into(), Country::Found(0, 21)),
            ])),
            "{boundary}"
        );
        frame.hour += 1;
        frame.metrics.table_ns.fill(0);
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        rows[0].tag = b"SPA".to_vec();
        let updates = encoder.encode(frame.clone(), false).unwrap().into();
        let changes = prepared.world.recorded_changes(&replay, &frame, updates);
        assert_eq!(changes.rows(TableId::Countries).unwrap().0.len(), 1);
        assert!(changes.rows(TableId::ProductionLines).unwrap().0.is_empty());
        prepared
            .observe_changed_into(&mut pages, &replay, &frame, None, &flag, &changes)
            .unwrap();
        let input = prepared.last_input.as_ref().expect(boundary);
        assert_eq!(
            input.countries,
            Some(BTreeMap::from([
                ("GER".into(), Country::Found(7, 9001)),
                ("SPA".into(), Country::Found(0, 21)),
            ])),
            "{boundary}"
        );
        assert_eq!(
            input.lines[&7]
                .iter()
                .map(|row| (row.handle, row.variant, row.active_factories))
                .collect::<Vec<_>>(),
            vec![(5, 70, 8), (6, 71, 3)],
            "{boundary}"
        );
        assert_eq!(
            prepared.active.keys().copied().collect::<Vec<_>>(),
            vec![(0, 21), (7, 9001)],
            "{boundary}"
        );
        prepared.flush(&mut pages, &flag).unwrap();
        let scan = assemble(
            &prepared.directory,
            &tag_key("GER"),
            None,
            recorded_language(&replay),
            &flag,
            |section, range| pages.read_at(section, range),
        )
        .unwrap();
        assert_eq!(
            scan.lines
                .iter()
                .map(|line| (
                    line.handle,
                    line.ended_by,
                    line.runs
                        .iter()
                        .map(|run| (run.first_hour, run.last_hour, run.active_factories))
                        .collect::<Vec<_>>(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (5, None, vec![(FIRST, FIRST + 1, 8)]),
                (6, None, vec![(FIRST, FIRST + 1, 3)]),
            ],
            "{boundary}"
        );
        assert_eq!(scan.reads, vec![FIRST], "{boundary}");
        assert_eq!(scan.unreadable_stretches, 0, "{boundary}");
    }
}

#[test]
fn quiet_hours_and_read_evidence_do_not_rebuild_world_values_or_visit_active_lines() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, [], None);
    let mut pages = fixtures::FixturePages::default();
    let mut frame = scaled_production(1501, 4);
    prepared
        .observe_into(&mut pages, &replay, &frame, None, &flag)
        .unwrap();
    assert_eq!(
        prepared
            .native
            .values()
            .map(|country| country.state.open.len())
            .sum::<usize>(),
        6004
    );
    let before = crate::prepared::prepass::notes();
    for hour in FIRST + 1..FIRST + 1001 {
        frame.hour = hour;
        frame.metrics.table_ns.fill(0);
        prepared
            .observe_into(&mut pages, &replay, &frame, None, &flag)
            .unwrap();
    }
    for key in [
        "production-lines.country_fold",
        "production-lines.carried_line",
        "production-lines.definition_projection",
        "production-lines.design_projection",
        "production-lines.mio_candidate",
        "production-lines.mio_value_projection",
    ] {
        assert_eq!(
            crate::prepared::prepass::noted(key),
            before.get(key).copied().unwrap_or(0),
            "{key}"
        );
    }
    let carried = crate::prepared::prepass::noted("production-lines.carried_line");
    for hour in FIRST + 1001..FIRST + 1011 {
        frame.hour = hour;
        frame.tables[TableId::VariantStats.index()] = frame.tables[TableId::VariantStats.index()]
            .clone()
            .with_sampled_at(Some(hour));
        prepared
            .observe_into(&mut pages, &replay, &frame, None, &flag)
            .unwrap();
    }
    for key in [
        "production-lines.carried_line",
        "production-lines.definition_projection",
        "production-lines.design_projection",
        "production-lines.mio_value_projection",
    ] {
        assert_eq!(
            crate::prepared::prepass::noted(key),
            before.get(key).copied().unwrap_or(0),
            "read evidence: {key}"
        );
    }
    assert_eq!(
        crate::prepared::prepass::noted("production-lines.carried_line"),
        carried
    );
    prepared.flush(&mut pages, &flag).unwrap();
    assert_eq!(prepared.native.len(), 1501);
}

#[test]
fn one_changed_name_only_projects_its_organisation_not_the_world() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::{FrameEncoder, TableView};
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, [], None);
    let mut pages = fixtures::FixturePages::default();
    let mut frame = scaled_production(1501, 4);
    let mut encoder = FrameEncoder::new();
    encoder.encode(frame.clone(), true).unwrap();
    prepared
        .observe_into(&mut pages, &replay, &frame, None, &flag)
        .unwrap();
    frame.hour += 1;
    frame.metrics.table_ns.fill(0);
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    rows[501].literal = b"Changed synthetic organisation".to_vec();
    let updates = encoder.encode(frame.clone(), false).unwrap().into();
    let changes = prepared.world.recorded_changes(&replay, &frame, updates);
    let before = crate::prepared::prepass::notes();
    prepared
        .observe_changed_into(&mut pages, &replay, &frame, None, &flag, &changes)
        .unwrap();
    for key in [
        "production-lines.mio_candidate",
        "production-lines.mio_value_projection",
        "production-lines.organisation_projection",
    ] {
        assert_eq!(
            crate::prepared::prepass::noted(key) - before.get(key).copied().unwrap_or(0),
            1,
            "{key}"
        );
    }
    for key in [
        "production-lines.country_fold",
        "production-lines.carried_line",
        "production-lines.design_projection",
        "production-lines.definition_projection",
    ] {
        assert_eq!(
            crate::prepared::prepass::noted(key),
            before.get(key).copied().unwrap_or(0),
            "{key}"
        );
    }
    frame.hour += 1;
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    rows[501].command_power_raw += 1;
    let updates = encoder.encode(frame.clone(), false).unwrap().into();
    let changes = prepared.world.recorded_changes(&replay, &frame, updates);
    let before = crate::prepared::prepass::notes();
    prepared
        .observe_changed_into(&mut pages, &replay, &frame, None, &flag, &changes)
        .unwrap();
    for key in [
        "production-lines.country_fold",
        "production-lines.carried_line",
        "production-lines.mio_candidate",
        "production-lines.mio_value_projection",
    ] {
        assert_eq!(
            crate::prepared::prepass::noted(key),
            before.get(key).copied().unwrap_or(0),
            "irrelevant country delta: {key}"
        );
    }
}

fn scaled_production(countries: u32, per_country: u32) -> DecodedFrame {
    use replay_data::rows::{
        IndustrialOrganisationDetailsRow, IndustrialOrganisationRow, LexicalTokenRow,
        ProductionLineOutputRow, ProductionLineRow, TableRows, VariantStatsRow,
    };
    use replay_frame::frame::TableView;
    let mut frame = crate::test_fixtures::scale_frame(1, FIRST, countries);
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            (0..countries * per_country)
                .map(|id| ProductionLineRow {
                    handle: u64::from(id + 1),
                    country: 100_000 + id / per_country,
                    variant: 70,
                    mio: u64::from(1 + id / per_country),
                    kind: 0,
                    active_factories: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(Vec::<
            ProductionLineOutputRow,
        >::new()));
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(
            (0..countries)
                .map(|id| IndustrialOrganisationRow {
                    organisation: u64::from(id + 1),
                    owner: 100_000 + id,
                    country: 100_000 + id,
                    token: id + 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::IndustrialOrganisationDetails.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationDetails(
            (0..countries)
                .map(|id| IndustrialOrganisationDetailsRow {
                    organisation: u64::from(id + 1),
                    country: 100_000 + id,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        (0..countries)
            .map(|id| LexicalTokenRow {
                token: id + 1,
                literal: format!("Synthetic organisation {id}").into_bytes(),
            })
            .collect(),
    ));
    *frame.tables[TableId::VariantStats.index()] =
        TableView::Complete(TableRows::VariantStats(vec![VariantStatsRow {
            variant: 70,
            ..Default::default()
        }]));
    frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    frame
}

fn fixture_source(replay: &Replay, pages: fixtures::FixturePages) -> Source {
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "production-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::open(replay.bytes().to_vec()).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    *source.state::<fixtures::FixturePages>().lock().unwrap() = pages;
    source
}
fn assert_equivalent(
    a: &Prepared,
    b: &Prepared,
    replay: &Replay,
    left: &mut fixtures::FixturePages,
    right: &mut fixtures::FixturePages,
) {
    let flag = AtomicBool::new(false);
    for tag in ["GER", "ITA"] {
        let x = assemble(
            &a.directory,
            &tag_key(tag),
            None,
            recorded_language(replay),
            &flag,
            |section, range| left.read_at(section, range),
        )
        .unwrap();
        let y = assemble(
            &b.directory,
            &tag_key(tag),
            None,
            recorded_language(replay),
            &flag,
            |section, range| right.read_at(section, range),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&SavedLines::save(&x, &flag).unwrap()).unwrap(),
            serde_json::to_vec(&SavedLines::save(&y, &flag).unwrap()).unwrap()
        );
        for section in [saved_sections(1, tag).0, saved_sections(1, tag).1] {
            assert_eq!(
                left.read_at(&section, 0).unwrap(),
                right.read_at(&section, 0).unwrap(),
                "{section}"
            );
        }
    }
    if a.directory.coverage.complete {
        let left = fixture_source(
            replay,
            fixtures::FixturePages {
                pages: left.pages.clone(),
            },
        );
        let right = fixture_source(
            replay,
            fixtures::FixturePages {
                pages: right.pages.clone(),
            },
        );
        assert_eq!(public_payloads(&left, None), public_payloads(&right, None));
    }
}

#[test]
fn independent_full_v5_meta_and_all_public_routes_cover_gaps_refusals_and_native_tag_reuse() {
    let replay = super::super::tests::quiet_production_replay(32);
    let flag = AtomicBool::new(false);
    let blocks = branch_blocks(&replay, 1);
    let expected =
        super::super::reference::scan(&replay, &blocks, "GER", None, None, &flag).unwrap();
    let native =
        super::super::reference::scan(&replay, &blocks, "GER", None, Some((7, 9001)), &flag)
            .unwrap();
    let mut baseline = fixtures::FixturePages::default();
    write_saved(
        &mut baseline,
        &SavedScan {
            branch: 1,
            tag: "GER".into(),
            pictured: false,
            visual: None,
            executable: engine_executable::executable_tables::is_initialized(),
            last_block: last_block(&replay, 1),
            scan: SavedLines::save(&expected, &flag).unwrap(),
        },
    )
    .unwrap();
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut pages, &replay, block, None, &flag)
            .unwrap();
        prepared = restore(&mut pages, &replay);
    }
    for section in [saved_sections(1, "GER").0, saved_sections(1, "GER").1] {
        assert_eq!(
            pages.read_at(&section, 0).unwrap(),
            baseline.read_at(&section, 0).unwrap(),
            "{section}"
        );
    }
    let source = fixture_source(&replay, pages);
    for origin in [None, Some(FIRST)] {
        let actual = public_payloads(&source, origin);
        let reference = super::super::reference::with_native(
            &source,
            expected.clone(),
            BTreeMap::from([((7, 9001), native.clone())]),
            || public_payloads(&source, origin),
        );
        assert_eq!(actual, reference, "origin={origin:?}");
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct NamingPeak {
    live: usize,
    peak: usize,
    total_peak: usize,
    pages: usize,
    live_bytes: usize,
    peak_bytes: usize,
    total_bytes: usize,
}
thread_local! {
    static NAMING_PEAK: std::cell::Cell<NamingPeak> = const {
        std::cell::Cell::new(NamingPeak { live: 0, peak: 0, total_peak: 0, pages: 0, live_bytes: 0, peak_bytes: 0, total_bytes: 0 })
    };
}
pub(super) struct NamingOperands(usize, usize);
// Counts owned operand backing allocations, not tree nodes or backend fixture pages.
fn naming_bytes(versions: &BTreeMap<u64, Vec<equipment::VariantVersion>>) -> usize {
    versions
        .values()
        .map(|versions| {
            versions.capacity() * std::mem::size_of::<equipment::VariantVersion>()
                + versions
                    .iter()
                    .filter_map(|version| version.raw.as_ref())
                    .map(|raw| {
                        raw.token.capacity()
                            + raw.display_name.capacity()
                            + raw.suffix.capacity()
                            + raw.name_tag.as_ref().map_or(0, String::capacity)
                            + raw.type_tag.as_ref().map_or(0, String::capacity)
                            + raw.design.as_ref().map_or(0, String::capacity)
                            + raw.refusals.capacity() * std::mem::size_of::<(String, String)>()
                            + raw
                                .refusals
                                .iter()
                                .map(|(key, value)| key.capacity() + value.capacity())
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        })
        .sum()
}
impl NamingOperands {
    pub(super) fn hold(
        catalogue: &VariantCatalogue,
        retained: &BTreeMap<u64, Vec<equipment::VariantVersion>>,
    ) -> Self {
        let operands = catalogue.versions.values().map(Vec::len).sum();
        let bytes = naming_bytes(&catalogue.versions);
        let retained_operands: usize = retained.values().map(Vec::len).sum();
        let retained_bytes = naming_bytes(retained);
        NAMING_PEAK.with(|counter| {
            let mut count = counter.get();
            count.live += operands;
            count.peak = count.peak.max(count.live);
            count.total_peak = count.total_peak.max(count.live + retained_operands);
            count.pages += 1;
            count.live_bytes += bytes;
            count.peak_bytes = count.peak_bytes.max(count.live_bytes);
            count.total_bytes = count.total_bytes.max(count.live_bytes + retained_bytes);
            counter.set(count);
        });
        Self(operands, bytes)
    }
}
impl Drop for NamingOperands {
    fn drop(&mut self) {
        NAMING_PEAK.with(|counter| {
            let mut count = counter.get();
            count.live -= self.0;
            count.live_bytes -= self.1;
            counter.set(count);
        });
    }
}

#[test]
fn fixed_variant_naming_workspace_does_not_grow_with_historical_pages() {
    const VARIANTS: usize = 48;
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut baseline = None;
    for page_count in [4, 32, 128] {
        let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
        prepared.world.variants = VariantCatalogue::seeded(
            (0..VARIANTS as u64)
                .map(|id| {
                    (
                        id,
                        vec![equipment::VariantVersion {
                            first_hour: FIRST,
                            raw: Some(equipment::RawVariant {
                                id,
                                first_hour: FIRST,
                                token: format!("synthetic-{id}"),
                                display_name: "unchanged naming operand ".repeat(32),
                                ..Default::default()
                            }),
                        }],
                    )
                })
                .collect(),
        );
        let mut stored = fixtures::FixturePages::default();
        for hour in 0..page_count {
            prepared.world.variants.advance(FIRST + hour);
            prepared.flush(&mut stored, &flag).unwrap();
        }
        let expected = serde_json::to_value(&prepared.world.variants).unwrap();
        let directory = prepared.directory;
        drop(prepared.world);
        NAMING_PEAK.set(NamingPeak::default());
        let scan = assemble(
            &directory,
            &tag_key("GER"),
            None,
            recorded_language(&replay),
            &flag,
            |section, range| stored.read_at(section, range),
        )
        .unwrap();
        let peak = NAMING_PEAK.get();
        assert_eq!(peak.live, 0);
        assert_eq!(peak.live_bytes, 0);
        assert!(peak.peak_bytes > 32 * 768);
        let first = *baseline.get_or_insert((peak.peak_bytes, peak.total_bytes));
        assert_eq!(
            (peak.peak_bytes, peak.total_bytes),
            first,
            "operand allocations grew with pages"
        );
        assert_eq!(peak.pages, page_count as usize);
        assert_eq!(peak.peak, VARIANTS, "decoded page operands: {peak:?}");
        assert!(
            peak.total_peak <= 2 * VARIANTS,
            "page plus final operands: {peak:?}"
        );
        assert_eq!(serde_json::to_value(&scan.variants).unwrap(), expected);
        assert!(scan.lines.is_empty());
    }
}

#[test]
fn naming_pages_keep_independent_bytes_metadata_and_public_payloads_across_spill_and_resume() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let frames = (0..12)
        .map(|offset| {
            let mut frame = production_frame(
                FIRST + offset,
                true,
                true,
                9001,
                &[(5, 70, 8, 0)],
                Some(&[(5, 70, i64::from(offset), true, false)]),
            );
            let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                &mut *frame.tables[TableId::VariantDefinitions.index()]
            else {
                unreachable!()
            };
            let seed = rows[0].clone();
            for id in 100..132 {
                rows.push(VariantDefinitionRow { id, ..seed.clone() });
            }
            rows.sort_by_key(|row| row.id);
            frame
        })
        .collect();
    let replay = replay_frames(frames);
    assert_reference_publication_modes(&replay, |scan| {
        assert!(scan.variants.versions.len() >= 32);
        assert!(
            scan.variants
                .versions
                .values()
                .all(|versions| versions.len() == 1)
        );
    });
}

#[test]
fn ordered_page_logs_keep_old_checkpoints_and_shared_prefixes_immutable() {
    let mut stored = fixtures::FixturePages::default();
    let mut facts = private::Store::new(&mut stored, 1);
    let mut log = pages::PageLog::default();
    log.append(&mut facts, "synthetic-log", &"first").unwrap();
    let prefix = log;
    log.append(&mut facts, "synthetic-log", &"later").unwrap();
    let mut alternative = prefix;
    alternative
        .append(&mut facts, "synthetic-log", &"replacement")
        .unwrap();
    for (log, expected) in [
        (prefix, vec!["first"]),
        (log, vec!["first", "later"]),
        (alternative, vec!["first", "replacement"]),
    ] {
        let mut cursor = log.cursor(0).unwrap();
        for value in expected {
            assert_eq!(
                cursor
                    .next::<String>(&mut facts, "synthetic-log")
                    .unwrap()
                    .as_deref(),
                Some(value)
            );
        }
        assert!(
            cursor
                .next::<String>(&mut facts, "synthetic-log")
                .unwrap()
                .is_none()
        );
    }
    assert!(prefix.cursor(2).is_err());
}

fn unreadable_initial_recording(later: bool) -> Replay {
    use replay_data::rows::{CampaignRow, TableRows};
    use replay_frame::frame::{TableUpdate, TableView};
    let frame = production_frame(FIRST, true, true, 9001, &[(5, 70, 8, 0)], Some(&[]));
    let original = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let mut frames = original.read_block(&original.index[0]).unwrap();
    frames[0].tables[TableId::Campaign.index()] =
        TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
            id: b"wrong-initial-campaign".to_vec(),
        }]));
    let malformed = replay::container::test_work::with_block_frames(&original, &frames).unwrap();
    if !later {
        return malformed;
    }
    let mut frame = production_frame(
        FIRST + 1,
        true,
        true,
        9001,
        &[(5, 70, 8, 0)],
        Some(&[(5, 70, 1, true, false)]),
    );
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.tag == b"GER");
    malformed
        .append(&[continuation_records(&malformed, 1, vec![frame])])
        .unwrap()
}

#[test]
fn unreadable_initial_blocks_keep_the_seeded_publication_roster_after_committed_resume() {
    let flag = AtomicBool::new(false);
    for readable_later in [false, true] {
        let replay = unreadable_initial_recording(readable_later);
        let mut observed = 0;
        let failure = replay
            .read_block_recorded_alone(
                &replay.index[0],
                &Prepared::tables(false),
                |_, _| {
                    observed += 1;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap_err();
        assert_eq!(observed, 0);
        assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            replay::container::ReplayStream::failed_hour(&failure),
            Some(FIRST)
        );
        if readable_later {
            let mut hours = Vec::new();
            replay
                .read_block_recorded_alone(
                    &replay.index[1],
                    &Prepared::tables(false),
                    |_, frame| {
                        hours.push(frame.hour);
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(hours, vec![FIRST + 1]);
        }
        let mut expected = fixtures::FixturePages::default();
        for tag in ["GER", "ITA"] {
            let scan = super::super::reference::scan(
                &replay,
                &branch_blocks(&replay, 1),
                tag,
                None,
                None,
                &flag,
            )
            .unwrap();
            assert_eq!(scan.unreadable_stretches, 1);
            assert_eq!(
                scan.reads,
                if readable_later && tag == "GER" {
                    vec![FIRST + 1]
                } else {
                    vec![]
                }
            );
            assert_eq!(scan.unread_output[0].first_hour, FIRST);
            assert_eq!(scan.unread_output[0].last_hour, FIRST);
            if tag == "ITA" || !readable_later {
                assert!(scan.lines.is_empty());
            }
            write_saved(
                &mut expected,
                &SavedScan {
                    branch: 1,
                    tag: tag.into(),
                    pictured: false,
                    visual: None,
                    executable: engine_executable::executable_tables::is_initialized(),
                    last_block: last_block(&replay, 1),
                    scan: SavedLines::save(&scan, &flag).unwrap(),
                },
            )
            .unwrap();
        }
        for (resume, publish_prefix) in [(false, false), (true, false), (true, true)] {
            let mut prepared = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
            let mut stored = fixtures::FixturePages::default();
            for (position, block) in replay.index.iter().enumerate() {
                prepared
                    .advance_block(&mut stored, &replay, position, None, &flag)
                    .unwrap();
                if position == 0 && resume {
                    assert!(prepared.initial_tags.contains("ITA"));
                    if publish_prefix {
                        prepared
                            .publish(&mut stored, &replay, block, None, &flag)
                            .unwrap();
                    } else {
                        prepared.directory.last_block =
                            Some((block.first_hour, block.last_hour, block.offset));
                        prepared.save(&mut stored, &flag).unwrap();
                    }
                    prepared = restore(&mut stored, &replay);
                    assert_eq!(prepared.next_block(&replay).unwrap(), 1);
                }
            }
            prepared
                .publish(
                    &mut stored,
                    &replay,
                    replay.index.last().unwrap(),
                    None,
                    &flag,
                )
                .unwrap();
            assert!(prepared.directory.coverage.complete);
            let completed: BTreeMap<_, _> = stored
                .pages
                .iter()
                .filter(|((section, _), _)| section.starts_with("production-lines-scan-"))
                .map(|(key, bytes)| (key.clone(), bytes.clone()))
                .collect();
            assert_eq!(
                completed, expected.pages,
                "readable_later={readable_later}, resume={resume}, publish_prefix={publish_prefix}"
            );
        }
    }
}

#[test]
fn naming_boundary_tails_keep_latest_page_precedence_and_earlier_versions() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let first = equipment::VariantVersion {
        first_hour: FIRST,
        raw: Some(equipment::RawVariant {
            id: 70,
            first_hour: FIRST,
            display_name: "first".into(),
            ..Default::default()
        }),
    };
    let next = equipment::VariantVersion {
        first_hour: FIRST + 1,
        raw: Some(equipment::RawVariant {
            id: 70,
            first_hour: FIRST + 1,
            display_name: "later".into(),
            ..Default::default()
        }),
    };
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    prepared.world.variants = VariantCatalogue::seeded(BTreeMap::from([(70, vec![first.clone()])]));
    let mut stored = fixtures::FixturePages::default();
    prepared.flush(&mut stored, &flag).unwrap();
    prepared
        .world
        .variants
        .versions
        .get_mut(&70)
        .unwrap()
        .push(next);
    prepared.flush(&mut stored, &flag).unwrap();
    let tail = prepared
        .world
        .variants
        .versions
        .get_mut(&70)
        .unwrap()
        .last_mut()
        .unwrap();
    tail.raw.as_mut().unwrap().display_name = "corrected boundary".into();
    let expected = vec![first, tail.clone()];
    prepared.flush(&mut stored, &flag).unwrap();
    let actual = assemble(
        &prepared.directory,
        &tag_key("GER"),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| stored.read_at(section, range),
    )
    .unwrap();
    assert_eq!(actual.variants.versions[&70], expected);
}

#[test]
fn recording_growth_reconciles_new_seed_tags_without_rewalking_accepted_blocks() {
    use replay_data::rows::{CountryLinkRow, CountryRow, TableRows};
    use replay_frame::frame::TableView;
    let flag = AtomicBool::new(false);
    let completed = |stored: &fixtures::FixturePages| -> BTreeMap<_, _> {
        stored
            .pages
            .iter()
            .filter(|((section, _), _)| section.starts_with("production-lines-scan-"))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    for unreadable in [false, true] {
        let prefix = if unreadable {
            unreadable_initial_recording(false)
        } else {
            Replay::from_records(
                CAMPAIGN.into(),
                &[records(
                    1,
                    vec![production_frame(
                        FIRST,
                        true,
                        true,
                        9001,
                        &[(5, 70, 8, 0)],
                        Some(&[]),
                    )],
                )],
            )
            .unwrap()
        };
        for extend_old in [false, true] {
            let mut added =
                production_frame(FIRST + 2, true, true, 9001, &[(5, 70, 8, 0)], Some(&[]));
            added.branch = 2;
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *added.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            rows.push(CountryRow {
                id: 10000,
                canonical_slot: 8,
                tag: b"NEW".to_vec(),
                ..Default::default()
            });
            let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *added.tables[TableId::CountryLinks.index()]
            else {
                unreachable!()
            };
            rows.push(CountryLinkRow {
                id: 10000,
                canonical_slot: 8,
            });
            let mut grown = prefix.append(&[records(2, vec![added])]).unwrap();
            if extend_old {
                grown = grown
                    .append(&[continuation_records(
                        &grown,
                        1,
                        vec![production_frame(
                            FIRST + 1,
                            true,
                            true,
                            9001,
                            &[(5, 70, 8, 0)],
                            Some(&[(5, 70, 1, true, false)]),
                        )],
                    )])
                    .unwrap();
            }
            let mut expected = fixtures::FixturePages::default();
            let mut fresh = fixtures::FixturePages::default();
            for branch in [1, 2] {
                let mut prepared = Prepared::new(
                    &grown,
                    branch,
                    ["GER".into(), "ITA".into(), "NEW".into()],
                    None,
                );
                for (position, block) in grown
                    .index
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| block.branch == branch)
                {
                    prepared
                        .advance_block(&mut fresh, &grown, position, None, &flag)
                        .unwrap();
                    prepared
                        .publish(&mut fresh, &grown, block, None, &flag)
                        .unwrap();
                }
                for tag in ["GER", "ITA", "NEW"] {
                    let scan = super::super::reference::scan(
                        &grown,
                        &branch_blocks(&grown, branch),
                        tag,
                        None,
                        None,
                        &flag,
                    )
                    .unwrap();
                    if branch == 1 && tag == "NEW" {
                        assert!(scan.lines.is_empty());
                        assert_eq!(scan.unreadable_stretches, usize::from(unreadable));
                        assert!(scan.reads.is_empty());
                    }
                    write_saved(
                        &mut expected,
                        &SavedScan {
                            branch,
                            tag: tag.into(),
                            pictured: false,
                            visual: None,
                            executable: engine_executable::executable_tables::is_initialized(),
                            last_block: last_block(&grown, branch),
                            scan: SavedLines::save(&scan, &flag).unwrap(),
                        },
                    )
                    .unwrap();
                }
            }
            assert_eq!(completed(&fresh), expected.pages);
            for publish_prefix in [false, true] {
                let mut stored = fixtures::FixturePages::default();
                let mut prepared = Prepared::new(&prefix, 1, ["GER".into(), "ITA".into()], None);
                prepared
                    .advance_block(&mut stored, &prefix, 0, None, &flag)
                    .unwrap();
                if publish_prefix {
                    prepared
                        .publish(&mut stored, &prefix, &prefix.index[0], None, &flag)
                        .unwrap();
                } else {
                    prepared.directory.last_block = last_block(&prefix, 1);
                    prepared.save(&mut stored, &flag).unwrap();
                }
                let accepted = stored.pages.clone();
                prepared = restore(&mut stored, &grown);
                let next = prepared.next_block(&grown).unwrap();
                assert_eq!(next, 1);
                let frontier = (
                    prepared.directory.last_block,
                    prepared.directory.coverage.clone(),
                );
                let pages = prepared.directory.pages;
                prepared
                    .reconcile_tags(&mut stored, &grown, ["GER", "ITA", "NEW"], None, &flag)
                    .unwrap();
                if publish_prefix && !extend_old {
                    assert_eq!(prepared.directory.pages, pages);
                    assert_eq!(prepared.directory.last_block, frontier.0);
                    assert_eq!(
                        serde_json::to_value(&prepared.directory.coverage).unwrap(),
                        serde_json::to_value(frontier.1).unwrap()
                    );
                    for (key, bytes) in &accepted {
                        if key.0.starts_with("production-lines-history-")
                            || key.0.starts_with("production-lines-variants-")
                            || key.0.starts_with("production-lines-aliases-")
                            || key.0.starts_with("production-lines-scan-")
                            || key.0.starts_with("production-lines-state-")
                        {
                            assert_eq!(stored.pages.get(key), Some(bytes), "rewrote {key:?}");
                        }
                    }
                    let reconciled = stored.pages.clone();
                    prepared
                        .reconcile_tags(&mut stored, &grown, ["NEW", "ITA", "GER"], None, &flag)
                        .unwrap();
                    assert_eq!(
                        stored.pages, reconciled,
                        "reconciliation must be idempotent"
                    );
                }
                prepared = restore(&mut stored, &grown);
                assert_eq!(prepared.next_block(&grown).unwrap(), next);
                for (position, block) in grown
                    .index
                    .iter()
                    .enumerate()
                    .filter(|(position, block)| *position >= next && block.branch == 1)
                {
                    prepared
                        .advance_block(&mut stored, &grown, position, None, &flag)
                        .unwrap();
                    prepared
                        .publish(&mut stored, &grown, block, None, &flag)
                        .unwrap();
                }
                assert!(prepared.directory.coverage.complete);
                let mut second =
                    Prepared::new(&grown, 2, ["GER".into(), "ITA".into(), "NEW".into()], None);
                for (position, block) in grown
                    .index
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| block.branch == 2)
                {
                    second
                        .advance_block(&mut stored, &grown, position, None, &flag)
                        .unwrap();
                    second
                        .publish(&mut stored, &grown, block, None, &flag)
                        .unwrap();
                }
                assert_eq!(
                    completed(&stored),
                    expected.pages,
                    "unreadable={unreadable}, extend_old={extend_old}, publish_prefix={publish_prefix}"
                );
                assert_eq!(completed(&stored), completed(&fresh));
                let source = fixture_source(&grown, fixtures::FixturePages::default());
                let committed = crate::prepared::series::tests::Fixture::new();
                let mut writer = committed.writer();
                for ((section, range), bytes) in &stored.pages {
                    writer.write_at(section, *range, bytes.as_slice()).unwrap();
                }
                writer.commit().unwrap();
                let context = LoadContext { content: None };
                for branch in [1, 2] {
                    assert!(
                        current(&committed.store, &committed.key, &source, &context, branch)
                            .unwrap()
                    );
                    for tag in ["GER", "ITA", "NEW"] {
                        assert!(
                            saved_current(
                                &committed.store,
                                &committed.key,
                                &source,
                                &context,
                                branch,
                                tag,
                            )
                            .unwrap()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn failed_seed_publication_keeps_the_committed_roster_and_compatibility_sections_atomic() {
    struct FailedMeta<'a>(&'a mut store::Writer);
    impl Pages for FailedMeta<'_> {
        fn write_at(&mut self, section: &str, range: u64, bytes: &[u8]) -> io::Result<()> {
            if section == saved_sections(1, "NEW").1 {
                return Err(io::Error::other("fixture seed metadata write failed"));
            }
            Pages::write_at(self.0, section, range, bytes)
        }
        fn write(&mut self, section: &str, bytes: &[u8]) -> io::Result<u64> {
            Pages::write(self.0, section, bytes)
        }
        fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
            Pages::read_at(self.0, section, range)
        }
    }
    let replay = recording();
    let flag = AtomicBool::new(false);
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into(), "ITA".into()], None);
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut writer, &replay, position, None, &flag)
            .unwrap();
        prepared
            .publish(&mut writer, &replay, block, None, &flag)
            .unwrap();
    }
    writer.commit().unwrap();
    let accepted = fixture.payloads();
    let accepted_index = prepared.directory.index;
    let mut writer = fixture.writer();
    let error = prepared
        .reconcile_tags(
            &mut FailedMeta(&mut writer),
            &replay,
            ["GER", "ITA", "NEW"],
            None,
            &flag,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture seed metadata write failed");
    assert!(prepared.next_block(&replay).is_err());
    assert!(prepared.save(&mut writer, &flag).is_err());
    drop(writer);
    assert_eq!(fixture.payloads(), accepted);
    assert_eq!(
        directory(&fixture.store, &fixture.key, 1)
            .unwrap()
            .unwrap()
            .index,
        accepted_index
    );
    for section in [saved_sections(1, "NEW").0, saved_sections(1, "NEW").1] {
        assert!(
            fixture
                .store
                .read(&fixture.key, &section, 0)
                .unwrap()
                .is_none()
        );
    }
    let mut prepared = Prepared::resume(&fixture.store, &fixture.key, &replay, 1, None)
        .unwrap()
        .unwrap();
    let mut writer = fixture.writer();
    prepared
        .reconcile_tags(&mut writer, &replay, ["NEW"], None, &flag)
        .unwrap();
    writer.commit().unwrap();
    let scan = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "NEW",
        None,
        None,
        &flag,
    )
    .unwrap();
    let mut expected = fixtures::FixturePages::default();
    write_saved(
        &mut expected,
        &SavedScan {
            branch: 1,
            tag: "NEW".into(),
            pictured: false,
            visual: None,
            executable: engine_executable::executable_tables::is_initialized(),
            last_block: last_block(&replay, 1),
            scan: SavedLines::save(&scan, &flag).unwrap(),
        },
    )
    .unwrap();
    let actual = fixture.payloads();
    for (key, bytes) in expected.pages {
        assert_eq!(actual.get(&key), Some(&bytes));
    }
}
