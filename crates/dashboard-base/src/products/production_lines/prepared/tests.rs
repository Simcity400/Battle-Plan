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

pub(super) fn restore(pages: &mut fixtures::FixturePages, replay: &Replay) -> Prepared {
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
/// A whole scan in the old saved layout, to compare scans byte for byte.
fn saved_bytes(scan: &ScannedLines) -> Vec<u8> {
    serde_json::to_vec(&SavedLines::save(scan, &AtomicBool::new(false)).unwrap()).unwrap()
}

/// Pages read through a closure, as a query reads them.
struct ReadPages<F>(F);
impl<F: FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>> Pages for ReadPages<F> {
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        (self.0)(section, range)
    }
    fn write_at(&mut self, _: &str, _: u64, _: &[u8]) -> io::Result<()> {
        Err(crate::invalid("Production query cannot write pages"))
    }
    fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
        Err(crate::invalid("Production query cannot write pages"))
    }
}

fn tag_key(tag: &str) -> String {
    format!(
        "tag-{}",
        tag.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
fn native_key(country: &NativeCountry) -> String {
    format!("native-{}-{}-{}", country.slot, country.id, country.epoch)
}

/// The whole scan the index serves for `selected`: a tag's key or a native country's.
fn assemble(
    directory: &Directory,
    selected: &str,
    catalogue: Option<&Catalogue>,
    language: Arc<str>,
    cancelled: &AtomicBool,
    read: impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<ScannedLines> {
    let mut index = read::Index::open(Box::new(ReadPages(read)), directory.clone())?;
    let history = if let Some(hex) = selected.strip_prefix("tag-") {
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect();
        index.tag_history(std::str::from_utf8(&bytes).unwrap())
    } else {
        let parts: Vec<&str> = selected
            .strip_prefix("native-")
            .unwrap()
            .split('-')
            .collect();
        let country = NativeCountry {
            slot: parts[0].parse().unwrap(),
            id: parts[1].parse().unwrap(),
            epoch: parts[2].parse().unwrap(),
        };
        index
            .native_history(&country)
            .ok_or_else(|| crate::invalid("Production country identity is not prepared"))?
    };
    let localisation = catalogue
        .filter(|_| directory.localised)
        .map(|catalogue| Arc::clone(&catalogue.localisation));
    index.scanned(history, localisation, language, cancelled)
}

fn read_country_at(
    directory: &Directory,
    tag: &str,
    hour: i32,
    read: impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Option<NativeCountry>> {
    read::Index::open(Box::new(ReadPages(read)), directory.clone())?.country_at(tag, hour)
}

/// The tag's scan as a query assembles it from the prepared pages.
fn served(
    directory: &Directory,
    replay: &Replay,
    tag: &str,
    read: impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
) -> Vec<u8> {
    saved_bytes(
        &assemble(
            directory,
            &tag_key(tag),
            None,
            recorded_language(replay),
            &AtomicBool::new(false),
            read,
        )
        .unwrap(),
    )
}

fn known(prepared: &Prepared, _pages: &mut fixtures::FixturePages) -> BTreeSet<String> {
    prepared
        .native
        .keys()
        .chain(prepared.retired.keys())
        .map(native_key)
        .collect()
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
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
    }
    assert_eq!(prepared.directory.last_block, last_block(&replay, 1));
    for tag in ["GER", "ITA"] {
        let scan = assemble(
            &prepared.directory,
            &tag_key(tag),
            None,
            recorded_language(&replay),
            &flag,
            |section, range| pages.read_at(section, range),
        )
        .unwrap();
        if tag == "ITA" {
            assert!(scan.lines.is_empty());
            assert_eq!(scan.reads, vec![FIRST, FIRST + 1, FIRST + 2]);
            assert_eq!(
                scan.line_hours,
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
            .publish(&mut pages, &replay, &replay.index[1], &flag)
            .is_err()
    );
    assert!(pages.pages.is_empty());
    assert_eq!(prepared.directory.last_block, None);
    assert!(!prepared.directory.coverage.complete);
    prepared
        .advance_block(&mut pages, &replay, 0, None, &flag)
        .unwrap();
    prepared
        .publish(&mut pages, &replay, &replay.index[0], &flag)
        .unwrap();
    let prefix = pages.pages.clone();
    let previous = prepared.processed.clone();
    prepared.processed.as_mut().unwrap().last_hour += 1;
    assert!(prepared.next_block(&replay).is_err());
    assert!(
        prepared
            .advance_block(&mut pages, &replay, 1, None, &flag)
            .is_err()
    );
    assert!(
        prepared
            .publish(&mut pages, &replay, replay.index.last().unwrap(), &flag)
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
            .publish(&mut pages, &replay, replay.index.last().unwrap(), &flag)
            .is_err()
    );
    assert_eq!(pages.pages, prefix);
    let coverage: crate::prepared::coverage::Coverage =
        serde_json::from_slice(&pages.read_at(&coverage_section(1), 0).unwrap().unwrap()).unwrap();
    assert_eq!(coverage.last_hour, FIRST);
    assert!(!coverage.complete);
    assert_eq!(
        prepared.directory.last_block,
        Some((FIRST, FIRST, replay.index[0].key()))
    );
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
        .publish(&mut pages, &replay, &replay.index[0], &flag)
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
            .publish(&mut pages, &replay, &replay.index[1], &flag)
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
        .publish(&mut pages, &first, &first.index[1], &flag)
        .unwrap();
    assert!(prepared.directory.coverage.complete);
    let segments: Vec<_> = (FIRST + 1..=FIRST + 2)
        .flat_map(|hour| {
            [1, 2].map(|branch| continuation_records(&first, branch, vec![frame(branch, hour)]))
        })
        .collect();
    // Interleaving is readable recovery input, not a recording we can append again.
    let grown = first.append(&segments).unwrap();
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
        prepared.publish(&mut pages, &grown, block, &flag).unwrap();
        prepared = restore_branch(&mut pages, &grown, 2);
    }
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
        assert_eq!(
            served(&prepared.directory, &grown, tag, |section, range| pages
                .read_at(section, range)),
            saved_bytes(&scan),
            "{tag}"
        );
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
            .publish(&mut left, &replay, block, &flag)
            .unwrap();
        resumed
            .advance_block(&mut right, &replay, position, None, &flag)
            .unwrap();
        resumed.publish(&mut right, &replay, block, &flag).unwrap();
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
        .publish(&mut pages, &replay, &replay.index[0], &flag)
        .unwrap();
    assert!(!prepared.directory.coverage.complete);
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
        .retain(|(section, _), _| section != &index::index_section(1));
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
fn surrender_target_changes_keep_one_production_lifetime_and_alias() {
    use replay_data::rows::{CountryExperienceGainRow, TableRows};
    use replay_frame::frame::TableView;
    let replay = recording();
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (offset, instance) in [(0, 7), (1, 8), (2, 7)] {
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
    assert_eq!(countries.len(), 1);
    assert_eq!(
        prepared
            .aliases_spans
            .iter()
            .filter(|alias| alias.tag == "GER")
            .count(),
        1
    );
}

#[test]
fn a_new_published_country_link_cannot_reuse_the_previous_production_lifetime() {
    let replay = recording();
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (offset, id) in [(0, 9001), (1, 9002)] {
        let frame = production_frame(FIRST + offset, true, true, id, &[(5, 70, 8, 0)], Some(&[]));
        prepared.observe(&mut pages, &replay, &frame, None).unwrap();
    }
    let countries: Vec<_> = known(&prepared, &mut pages)
        .into_iter()
        .filter(|country| country.starts_with("native-7-"))
        .collect();
    assert_eq!(countries.len(), 2);
    assert!(
        countries
            .iter()
            .any(|country| country.starts_with("native-7-9001-"))
    );
    assert!(
        countries
            .iter()
            .any(|country| country.starts_with("native-7-9002-"))
    );
    let aliases: Vec<_> = prepared
        .aliases_spans
        .iter()
        .filter(|alias| alias.tag == "GER")
        .collect();
    assert_eq!(aliases.len(), 2);
    assert_ne!(aliases[0].country, aliases[1].country);
}

#[test]
fn each_publication_only_appends_and_keeps_only_carried_entries_pending() {
    let replay = recording();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    let flag = AtomicBool::new(false);
    for (position, block) in replay.index.iter().enumerate() {
        let before = pages.pages.clone();
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        for (key, bytes) in &before {
            if key.0 != directory_section(1) && key.0 != coverage_section(1) {
                assert_eq!(pages.pages.get(key), Some(bytes), "rewrote {key:?}");
            }
        }
        for history in prepared.histories() {
            let events = history.state.events.held();
            assert_eq!(events.detail.runs.len(), 0);
            assert!(events.detail.outputs.is_empty());
            assert!(events.overview.reads.is_empty());
            assert!(events.overview.lives.is_empty());
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
            .publish(&mut pages, &replay, &replay.index[position], &flag)
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
    assert_eq!(
        served(&prepared.directory, &replay, "GER", |section, range| pages
            .read_at(section, range)),
        saved_bytes(&literal)
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
            1
        );
    }
}

#[test]
fn a_published_country_link_keeps_its_lifetime_when_it_returns() {
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
        .0
        .clone();
    let flag = AtomicBool::new(false);
    prepared.flush(&mut pages, &flag).unwrap();
    prepared.save(&mut pages, &flag).unwrap();
    let scan = assemble(
        &prepared.directory,
        &native_key(&country),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| pages.read_at(section, range),
    )
    .unwrap();
    assert_eq!(scan.reads, vec![FIRST, FIRST + 2]);
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
                .publish(&mut pages, &replay, block, &cancelled)
                .unwrap();
            assert!(prepared.aliases_spans.len() <= 2);
        }
        (prepared, pages)
    };
    let (ordinary, mut left) = prepare(usize::MAX);
    let (spilled, mut right) = prepare(1);
    assert!(spilled.chunks >= ordinary.chunks);
    for tag in ["GER", "ITA"] {
        assert_eq!(
            served(&ordinary.directory, &replay, tag, |section, range| left
                .read_at(section, range)),
            served(&spilled.directory, &replay, tag, |section, range| right
                .read_at(section, range)),
            "{tag}"
        );
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
fn a_checkpoint_between_blocks_resumes_what_an_uninterrupted_preparation_publishes() {
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
            resumed.directory.last_block = Some((block.first_hour, block.last_hour, block.key()));
            resumed.save(&mut right, &flag).unwrap();
            resumed = restore(&mut right, &replay);
            assert!(
                resumed
                    .native
                    .values()
                    .any(|country| !country.state.open.is_empty())
            );
        }
    }
    let block = replay.index.last().unwrap();
    uninterrupted
        .publish(&mut left, &replay, block, &flag)
        .unwrap();
    resumed.publish(&mut right, &replay, block, &flag).unwrap();
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

pub(super) fn authored(
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
    let mut keys: Vec<_> = engine_executable::modifier_keys::all().unwrap().collect();
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
    let baseline = saved_bytes(&expected);
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
                    Some((block.first_hour, block.last_hour, block.key()));
                prepared.save(&mut pages, &flag).unwrap();
                prepared = restore(&mut pages, &replay);
            }
        }
        prepared
            .publish(&mut pages, &replay, replay.index.last().unwrap(), &flag)
            .unwrap();
        assert_eq!(
            served(&prepared.directory, &replay, "GER", |section, range| pages
                .read_at(section, range)),
            baseline,
            "spill={spill}, checkpoint={checkpoint}"
        );
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
    assert_reference_publication_tags(replay, &[], verify);
}

/// GER's scan and payloads, and each of `others`' scans, equal the independent reference in
/// every publication mode; `verify` checks GER's.
fn assert_reference_publication_tags(
    replay: &Replay,
    others: &[&str],
    verify: impl Fn(&ScannedLines),
) {
    let flag = AtomicBool::new(false);
    let reference = |tag: &str| {
        super::super::reference::scan(replay, &branch_blocks(replay, 1), tag, None, None, &flag)
            .unwrap()
    };
    let expected = reference("GER");
    verify(&expected);
    let baseline = saved_bytes(&expected);
    let other_baselines: Vec<_> = others
        .iter()
        .map(|tag| saved_bytes(&reference(tag)))
        .collect();
    for (spill, checkpoint) in [(false, false), (true, false), (false, true), (true, true)] {
        let tags = std::iter::once("GER").chain(others.iter().copied());
        let mut prepared = Prepared::new(replay, 1, tags.map(str::to_owned), None);
        let mut pages = fixtures::FixturePages::default();
        for (position, block) in replay.index.iter().enumerate() {
            prepared
                .advance_block(&mut pages, replay, position, None, &flag)
                .unwrap();
            if spill {
                prepared.flush(&mut pages, &flag).unwrap();
            }
            if checkpoint && position + 1 < replay.index.len() {
                prepared.directory.last_block =
                    Some((block.first_hour, block.last_hour, block.key()));
                prepared.save(&mut pages, &flag).unwrap();
                prepared = restore(&mut pages, replay);
                assert_eq!(prepared.next_block(replay).unwrap(), position + 1);
            }
        }
        prepared
            .publish(&mut pages, replay, replay.index.last().unwrap(), &flag)
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
        assert_eq!(
            saved_bytes(&actual),
            baseline,
            "spill={spill}, checkpoint={checkpoint}"
        );
        for (tag, baseline) in others.iter().zip(&other_baselines) {
            let actual = assemble(
                &prepared.directory,
                &tag_key(tag),
                None,
                recorded_language(replay),
                &flag,
                |section, range| pages.read_at(section, range),
            )
            .unwrap();
            assert_eq!(
                saved_bytes(&actual),
                *baseline,
                "{tag}: spill={spill}, checkpoint={checkpoint}"
            );
        }
        // The same pages opened from a store answer alike.
        let stored = crate::prepared::series::tests::Fixture::new();
        let mut writer = stored.writer();
        for ((section, range), bytes) in &pages.pages {
            writer.write_at(section, *range, bytes.as_slice()).unwrap();
        }
        writer.commit().unwrap();
        let mut opened = read::Index::open(
            Box::new(index::StoreRanges::new(&stored.store, stored.key.clone())),
            prepared.directory.clone(),
        )
        .unwrap();
        let history = opened.tag_history("GER");
        let actual = opened
            .scanned(history, None, recorded_language(replay), &flag)
            .unwrap();
        assert_eq!(
            saved_bytes(&actual),
            baseline,
            "GER from a store: spill={spill}, checkpoint={checkpoint}"
        );
        drop(opened);
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
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
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
    assert_eq!((before.slot, before.id), (7, 9001));
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
fn closed_handles_and_retired_lifetimes_keep_only_their_ledger_entries() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, [], None);
    let mut pages = fixtures::FixturePages::default();
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
            // A closed handle keeps its lifetime and appearance, never its reads.
            assert!(country.state.open.len() + country.state.broken.len() <= 1);
            assert!(country.state.keys.handles.len() <= offset as usize + 1);
            assert_eq!(country.state.events.held().detail.runs.len(), 0);
        }
        // A retired lifetime is put away whole at the flush after it retires.
        assert!(
            prepared
                .retired
                .values()
                .all(|history| history.blob.is_some())
        );
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
            if boundary == "pressure" {
                1
            } else {
                usize::MAX
            },
        ));
        prepared
            .observe_changed_into(&mut pages, &replay, &frame, None, &flag, &changes)
            .unwrap();
        assert!(prepared.quiet.is_none());
        match boundary {
            "flush" => prepared.flush(&mut pages, &flag).unwrap(),
            "save" => prepared.save(&mut pages, &flag).unwrap(),
            "pressure" => assert!(prepared.chunks > 0),
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
        prepared.save(&mut pages, &flag).unwrap();
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
fn unchanged_censuses_and_independent_read_dates_stitch_exactly_across_spill_and_resume() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let mut base = authored(FIRST, &[(5, 70, 8, 0), (6, 71, 3, 0)], &[]);
    *base.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(vec![]));
    *base.tables[TableId::CountryProductionModifiers.index()] =
        TableView::Complete(TableRows::CountryProductionModifiers(vec![]));
    *base.tables[TableId::ProductionLineModifiers.index()] =
        TableView::Complete(TableRows::ProductionLineModifiers(vec![]));
    let mut frames = Vec::new();
    for offset in 0..80 {
        let mut frame = base.clone();
        frame.hour = FIRST + offset;
        if offset > 0 {
            frame.metrics.table_ns.fill(0);
        }
        if offset % 7 == 0 {
            frame.metrics.table_ns[TableId::VariantStats.index()] = 1;
        }
        if offset % 11 == 0 {
            frame.metrics.table_ns[TableId::IndustrialOrganisationDetails.index()] = 1;
        }
        frame.tables[TableId::VariantStats.index()] = frame.tables[TableId::VariantStats.index()]
            .clone()
            .with_sampled_at(Some(FIRST + offset / 7 * 7));
        frame.tables[TableId::IndustrialOrganisationDetails.index()] = frame.tables
            [TableId::IndustrialOrganisationDetails.index()]
        .clone()
        .with_sampled_at(Some(FIRST + offset / 11 * 11));
        if offset == 79 {
            frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
            let TableView::Complete(TableRows::ProductionLines(rows)) =
                &mut *frame.tables[TableId::ProductionLines.index()]
            else {
                unreachable!()
            };
            rows.iter_mut()
                .find(|row| row.handle == 5)
                .unwrap()
                .active_factories = 12;
        }
        frames.push(frame);
    }
    let later = frames.split_off(39);
    let replay = replay_segments(frames, later);
    assert_reference_publication_modes(&replay, |scan| {
        // Organisation details share the lines' slot, so their timed reads are census reads.
        assert_eq!(
            scan.reads,
            [0, 11, 22, 24, 33, 44, 48, 55, 66, 72, 77, 79].map(|offset| FIRST + offset)
        );
        assert!(
            scan.designs[&70]
                .iter()
                .any(|read| read.read_hour == Some(FIRST + 77))
        );
        assert_eq!(
            scan.lines
                .iter()
                .find(|line| line.handle == 5)
                .unwrap()
                .runs
                .last()
                .unwrap()
                .active_factories,
            12
        );
    });
    struct Budget(usize);
    impl Drop for Budget {
        fn drop(&mut self) {
            crate::prepared::pages::TEST_BATCH_BYTES.set(self.0);
        }
    }
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
    let _budget = Budget(crate::prepared::pages::TEST_BATCH_BYTES.replace(1));
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        prepared = restore(&mut pages, &replay);
    }
    let actual = assemble(
        &prepared.directory,
        &tag_key("GER"),
        None,
        recorded_language(&replay),
        &flag,
        |section, range| pages.read_at(section, range),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&SavedLines::save(&actual, &flag).unwrap()).unwrap(),
        serde_json::to_vec(&SavedLines::save(&expected, &flag).unwrap()).unwrap()
    );
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

pub(super) fn scaled_production(countries: u32, per_country: u32) -> DecodedFrame {
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

pub(super) fn fixture_source(replay: &Replay, pages: fixtures::FixturePages) -> Source {
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
fn independent_full_scan_and_all_public_routes_cover_gaps_refusals_and_native_tag_reuse() {
    let replay = super::super::tests::quiet_production_replay(32);
    let flag = AtomicBool::new(false);
    let blocks = branch_blocks(&replay, 1);
    let expected =
        super::super::reference::scan(&replay, &blocks, "GER", None, None, &flag).unwrap();
    let native =
        super::super::reference::scan(&replay, &blocks, "GER", None, Some((7, 9001)), &flag)
            .unwrap();
    let baseline = saved_bytes(&expected);
    let mut pages = fixtures::FixturePages::default();
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        prepared = restore(&mut pages, &replay);
    }
    assert_eq!(
        served(&prepared.directory, &replay, "GER", |section, range| pages
            .read_at(section, range)),
        baseline
    );
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
fn commits_chain_every_chunk_in_order_across_spills_and_resumes() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.flush(&mut pages, &flag).unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        prepared = restore(&mut pages, &replay);
    }
    assert_eq!(prepared.directory.commits, replay.index.len() as u64);
    let mut opened = read::Index::open(
        Box::new(fixtures::FixturePages {
            pages: pages.pages.clone(),
        }),
        prepared.directory.clone(),
    )
    .unwrap();
    let chunks = opened.chunks().unwrap();
    assert_eq!(chunks.len() as u64, prepared.chunks);
    assert!(chunks.windows(2).all(|pair| {
        pair[0].range < pair[1].range && pair[0].last.checked_add(1) == Some(pair[1].first)
    }));
    assert_eq!(chunks.last().unwrap().last, FIRST + 2);
}

fn unreadable_initial_recording(later: bool) -> Replay {
    use replay_data::rows::{CampaignRow, TableRows};
    use replay_frame::frame::TableView;
    let mut frame = production_frame(FIRST, true, true, 9001, &[(5, 70, 8, 0)], Some(&[]));
    *frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"wrong-initial-campaign".to_vec(),
        }]));
    let malformed = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
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
            .read_block_alone(
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
        if readable_later {
            let mut hours = Vec::new();
            replay
                .read_block_alone(
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
        let mut expected = BTreeMap::new();
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
            expected.insert(tag, saved_bytes(&scan));
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
                            .publish(&mut stored, &replay, block, &flag)
                            .unwrap();
                    } else {
                        prepared.directory.last_block =
                            Some((block.first_hour, block.last_hour, block.key()));
                        prepared.save(&mut stored, &flag).unwrap();
                    }
                    prepared = restore(&mut stored, &replay);
                    assert_eq!(prepared.next_block(&replay).unwrap(), 1);
                }
            }
            prepared
                .publish(&mut stored, &replay, replay.index.last().unwrap(), &flag)
                .unwrap();
            assert!(prepared.directory.coverage.complete);
            for (tag, expected) in &expected {
                assert!(prepared.tag_names.contains(*tag));
                assert_eq!(
                    &served(&prepared.directory, &replay, tag, |section, range| stored
                        .read_at(section, range)),
                    expected,
                    "{tag}: readable_later={readable_later}, resume={resume}, publish_prefix={publish_prefix}"
                );
            }
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
    prepared.world.touch_variant(70);
    prepared.pending = true;
    prepared.flush(&mut stored, &flag).unwrap();
    prepared
        .world
        .variants
        .versions
        .get_mut(&70)
        .unwrap()
        .push(next);
    prepared.world.touch_variant(70);
    prepared.pending = true;
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
    prepared.world.touch_variant(70);
    prepared.pending = true;
    prepared.flush(&mut stored, &flag).unwrap();
    prepared.save(&mut stored, &flag).unwrap();
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
            let mut expected = BTreeMap::new();
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
                    prepared.publish(&mut fresh, &grown, block, &flag).unwrap();
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
                    assert_eq!(
                        served(&prepared.directory, &grown, tag, |section, range| fresh
                            .read_at(section, range)),
                        saved_bytes(&scan),
                        "{branch} {tag}"
                    );
                    expected.insert((branch, tag), saved_bytes(&scan));
                }
            }
            for publish_prefix in [false, true] {
                let mut stored = fixtures::FixturePages::default();
                let mut prepared = Prepared::new(&prefix, 1, ["GER".into(), "ITA".into()], None);
                prepared
                    .advance_block(&mut stored, &prefix, 0, None, &flag)
                    .unwrap();
                if publish_prefix {
                    prepared
                        .publish(&mut stored, &prefix, &prefix.index[0], &flag)
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
                let chunks = prepared.chunks;
                prepared
                    .reconcile_tags(&mut stored, &grown, ["GER", "ITA", "NEW"], &flag)
                    .unwrap();
                if publish_prefix && !extend_old {
                    assert_eq!(prepared.chunks, chunks);
                    assert_eq!(prepared.directory.last_block, frontier.0);
                    assert_eq!(
                        serde_json::to_value(&prepared.directory.coverage).unwrap(),
                        serde_json::to_value(frontier.1).unwrap()
                    );
                    for (key, bytes) in &accepted {
                        if key.0.starts_with("production-index-")
                            || key.0.starts_with("production-commits-")
                            || key.0.starts_with("production-ledger-")
                        {
                            assert_eq!(stored.pages.get(key), Some(bytes), "rewrote {key:?}");
                        }
                    }
                    let reconciled = stored.pages.clone();
                    prepared
                        .reconcile_tags(&mut stored, &grown, ["NEW", "ITA", "GER"], &flag)
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
                    prepared.publish(&mut stored, &grown, block, &flag).unwrap();
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
                    second.publish(&mut stored, &grown, block, &flag).unwrap();
                }
                for ((branch, tag), expected) in &expected {
                    let directory = if *branch == 1 {
                        &prepared.directory
                    } else {
                        &second.directory
                    };
                    assert_eq!(
                        &served(directory, &grown, tag, |section, range| stored
                            .read_at(section, range)),
                        expected,
                        "{branch} {tag}: unreadable={unreadable}, extend_old={extend_old}, publish_prefix={publish_prefix}"
                    );
                }
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
                }
            }
        }
    }
}

#[test]
fn failed_seed_reconciliation_keeps_the_committed_roster_atomic() {
    struct FailedDirectory<'a>(&'a mut store::Writer);
    impl Pages for FailedDirectory<'_> {
        fn write_at(&mut self, section: &str, range: u64, bytes: &[u8]) -> io::Result<()> {
            if section == directory_section(1) {
                return Err(io::Error::other("fixture directory write failed"));
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
            .publish(&mut writer, &replay, block, &flag)
            .unwrap();
    }
    writer.commit().unwrap();
    let accepted = fixture.payloads();
    let accepted_tags = prepared.directory.tags.clone();
    let mut writer = fixture.writer();
    let error = prepared
        .reconcile_tags(
            &mut FailedDirectory(&mut writer),
            &replay,
            ["GER", "ITA", "NEW"],
            &flag,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture directory write failed");
    assert!(prepared.next_block(&replay).is_err());
    assert!(prepared.save(&mut writer, &flag).is_err());
    drop(writer);
    assert_eq!(fixture.payloads(), accepted);
    assert_eq!(
        directory(&fixture.store, &fixture.key, 1)
            .unwrap()
            .unwrap()
            .tags,
        accepted_tags
    );
    let mut prepared = Prepared::resume(&fixture.store, &fixture.key, &replay, 1, None)
        .unwrap()
        .unwrap();
    let mut writer = fixture.writer();
    prepared
        .reconcile_tags(&mut writer, &replay, ["NEW"], &flag)
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
    let directory = directory(&fixture.store, &fixture.key, 1).unwrap().unwrap();
    assert_eq!(directory.tags, prepared.directory.tags);
    let actual = served(&directory, &replay, "NEW", |section, range| {
        use std::io::Read;
        fixture
            .store
            .read(&fixture.key, section, range)?
            .map(|mut file| {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            })
            .transpose()
    });
    assert_eq!(actual, saved_bytes(&scan));
}

#[test]
fn an_old_private_section_identity_forces_repreparation() {
    let replay = recording();
    let cancelled = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &cancelled)
            .unwrap();
        prepared
            .publish(&mut pages, &replay, block, &cancelled)
            .unwrap();
    }
    let commits: Vec<_> = pages
        .pages
        .keys()
        .filter(|(section, _)| *section == index::commits_section(1))
        .cloned()
        .collect();
    for key in commits {
        let bytes = pages.pages.remove(&key).unwrap();
        pages
            .pages
            .insert(("production-commits-v0-1".into(), key.1), bytes);
    }
    assert!(pages.pages.contains_key(&(directory_section(1), 0)));
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    for ((section, range), bytes) in &pages.pages {
        writer.write_at(section, *range, bytes.as_slice()).unwrap();
    }
    writer.commit().unwrap();
    assert!(!binary_current(&fixture.store, &fixture.key, 1).unwrap());
    assert!(
        Prepared::resume(&fixture.store, &fixture.key, &replay, 1, None)
            .unwrap()
            .is_none()
    );
}

/// One branch's frames in blocks of `hours` hours each.
pub(super) fn replay_blocks(frames: Vec<DecodedFrame>, hours: usize) -> Replay {
    let mut blocks = frames.chunks(hours).map(<[DecodedFrame]>::to_vec);
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, blocks.next().unwrap())]).unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, block)])
            .unwrap();
    }
    replay
}

/// The quiet hours a preparation of branch 1 keeps when it reads the icon rules' tables, as
/// one with icon content does.
fn pictured_quiet_hours(replay: &Replay) -> usize {
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    let before = crate::prepared::prepass::noted("production-lines.quiet_hour");
    for block in &replay.index {
        replay
            .read_block_alone(
                block,
                &Prepared::tables(true),
                |updates, frame| {
                    let changes =
                        prepared
                            .world
                            .recorded_changes(replay, frame, (*updates).clone());
                    prepared.observe_changed_into(&mut pages, replay, frame, None, &flag, &changes)
                },
                || Ok(()),
            )
            .unwrap();
    }
    crate::prepared::prepass::noted("production-lines.quiet_hour") - before
}

#[test]
fn variant_events_naming_nothing_and_state_edits_between_censuses_stay_quiet_and_exact() {
    use replay_data::rows::{StateRow, TableRows, VariantChangeRow};
    use replay_frame::frame::TableView;
    let mut base = authored(FIRST, &[(5, 70, 8, 0), (6, 71, 3, 0)], &[]);
    // A table the fixture leaves unavailable decodes to a new view every hour, which no hour
    // carries quietly; per-line modifier samples keep a census from carrying the hours after it.
    for table in Prepared::tables(true) {
        if matches!(
            &*base.tables[table.index()],
            TableView::Unavailable { .. } | TableView::NotRecorded { .. }
        ) {
            *base.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    *base.tables[TableId::ProductionLineModifiers.index()] =
        TableView::Complete(TableRows::ProductionLineModifiers(Vec::new()));
    // Two blocks of 24 hours with a census every 12 hours, the state edited at `edits`.
    let recording = |edits: &[i32]| {
        let mut frames = Vec::new();
        for offset in 0..48 {
            let mut frame = base.clone();
            frame.hour = FIRST + offset;
            if offset > 0 {
                frame.metrics.table_ns.fill(0);
            }
            // Every hour logs a variant event that names nothing new.
            *frame.tables[TableId::VariantChanges.index()] =
                TableView::Complete(TableRows::VariantChanges(vec![VariantChangeRow {
                    sequence: offset as u64 + 1,
                    hour: frame.hour,
                    variant: 70,
                    flags: Some([0; 3]),
                    ..Default::default()
                }]));
            *frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 7,
                    manpower_available: edits.iter().filter(|&&at| at <= offset).count() as _,
                    ..Default::default()
                }]));
            frame.metrics.table_ns[TableId::VariantChanges.index()] = 1;
            frame.metrics.table_ns[TableId::States.index()] = 1;
            if offset % 12 == 0 {
                frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
            }
            frames.push(frame);
        }
        replay_blocks(frames, 24)
    };
    let undisturbed = recording(&[]);
    // Edits between censuses, none at a census hour.
    let edited = recording(&[5, 30]);
    // Every publication mode publishes as the reference does, the hours between censuses quiet.
    let before = crate::prepared::prepass::noted("production-lines.quiet_hour");
    assert_reference_publication_modes(&edited, |_| {});
    assert!(crate::prepared::prepass::noted("production-lines.quiet_hour") - before >= 4 * 40);
    // Read with the icon rules' tables, the census after each edit chooses its icons again.
    assert_eq!(
        pictured_quiet_hours(&undisturbed) - pictured_quiet_hours(&edited),
        2
    );
}

/// Censuses a day apart. The hours between read nothing, so the countries follow them deferred:
/// design hours every five, output, organisations and modifiers each missing for a while, and a
/// lost lines read at 61 ending every carried line. GER holds no lines through the second day.
fn deferred_hours_replay(split: usize) -> Replay {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let held = authored(
        FIRST,
        &[(5, 70, 8, 0), (6, 71, 3, 0)],
        &[(5, 70, 1, true, false)],
    );
    let empty = authored(FIRST, &[], &[]);
    let mut frames: Vec<_> = (0..97)
        .map(|offset| {
            let mut frame = if (24..48).contains(&offset) {
                empty.clone()
            } else {
                held.clone()
            };
            frame.hour = FIRST + offset;
            if offset % 24 != 0 {
                frame.metrics.table_ns.fill(0);
            }
            if offset % 24 != 0 && offset % 5 == 0 {
                frame.metrics.table_ns[TableId::VariantStats.index()] = 1;
            }
            frame.tables[TableId::VariantStats.index()] = frame.tables
                [TableId::VariantStats.index()]
            .clone()
            .with_sampled_at(Some(FIRST + offset / 5 * 5));
            let missing = match offset {
                30 | 31 => Some(TableId::ProductionLineOutput),
                40..=42 => Some(TableId::IndustrialOrganisations),
                52 | 53 => Some(TableId::CountryProductionModifiers),
                61 => Some(TableId::ProductionLines),
                _ => None,
            };
            if let Some(table) = missing {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "synthetic availability change".into(),
                };
            }
            if let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
                &mut *frame.tables[TableId::ProductionLineOutput.index()]
            {
                rows.iter_mut()
                    .for_each(|row| row.units = i64::from(offset));
            }
            frame
        })
        .collect();
    let later = frames.split_off(split);
    replay_segments(frames, later)
}

#[test]
fn deferred_hours_follow_availability_designs_and_breaks_exactly_across_spill_and_resume() {
    let covered = |spans: &[crate::products::country_groups::HourSpan], offset: i32| {
        spans
            .iter()
            .any(|span| span.first_hour <= FIRST + offset && FIRST + offset <= span.last_hour)
    };
    // One resume inside a run of carried hours, one just after the lost lines read.
    for split in [37, 62] {
        let replay = deferred_hours_replay(split);
        assert_reference_publication_tags(&replay, &["ITA", "SPA"], |scan| {
            // Known to hold no lines is coverage, not a gap.
            assert!(covered(&scan.line_hours, 30));
            assert!(covered(&scan.output_hours, 29) && !covered(&scan.output_hours, 32));
            assert!(!covered(&scan.organisation_hours, 43));
            assert!(!covered(&scan.line_hours, 61) && !covered(&scan.line_hours, 70));
            assert!(covered(&scan.line_hours, 72));
            let designed = |offset| {
                scan.designs[&70]
                    .iter()
                    .any(|read| read.first_hour == FIRST + offset)
            };
            assert!(designed(5) && designed(50) && designed(75));
            assert!(!designed(25) && !designed(65));
        });
    }
}

#[test]
fn countries_follow_deferred_hours_once_per_run_and_design_hours_visit_only_their_carriers() {
    use replay_data::rows::{ProductionLineOutputRow, TableRows};
    use replay_frame::frame::TableView;
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, [], None);
    let mut pages = fixtures::FixturePages::default();
    let mut frame = scaled_production(1501, 1);
    // Three countries hold lines; every other one is known to hold none.
    let TableView::Complete(TableRows::ProductionLines(rows)) =
        &mut *frame.tables[TableId::ProductionLines.index()]
    else {
        unreachable!()
    };
    rows.truncate(3);
    // Output read at each census keeps design hours from being read evidence.
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(vec![
            ProductionLineOutputRow {
                line: 1,
                country: 100_000,
                variant: 70,
                units: 1,
                exact: true,
                ..Default::default()
            },
        ]));
    let output = frame.tables[TableId::ProductionLineOutput.index()].clone();
    prepared
        .observe_into(&mut pages, &replay, &frame, None, &flag)
        .unwrap();
    let followers = prepared.native.len()
        + prepared
            .aliases
            .values()
            .filter(|alias| alias.projection().is_some())
            .count()
        + 1;
    let count = |key| crate::prepared::prepass::noted(key);
    let (folds, carries, designs) = (
        count("production-lines.country_fold"),
        count("production-lines.country_carry"),
        count("production-lines.country_design"),
    );
    let days = 4;
    for offset in 1..=days * 24 {
        let hour = offset % 24;
        frame.hour = FIRST + offset;
        frame.metrics.table_ns.fill(0);
        if hour == 0 {
            frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
        }
        // Output goes missing for two hours a day: three runs of equal availability.
        if (11..13).contains(&hour) {
            *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable {
                reason: "synthetic availability change".into(),
            };
        } else {
            frame.tables[TableId::ProductionLineOutput.index()] = output.clone();
        }
        if hour != 0 && hour % 5 == 0 {
            frame.tables[TableId::VariantStats.index()] = frame.tables
                [TableId::VariantStats.index()]
            .clone()
            .with_sampled_at(Some(FIRST + offset));
        }
        prepared
            .observe_into(&mut pages, &replay, &frame, None, &flag)
            .unwrap();
        if hour == 23 {
            // Between censuses no country folds an hour, and only the three carrying lines
            // record the day's four design hours.
            assert_eq!(
                count("production-lines.country_fold"),
                folds + offset as usize / 24 * followers
            );
            assert_eq!(
                count("production-lines.country_design"),
                designs + (offset as usize / 24 + 1) * 4 * 3
            );
        }
    }
    assert_eq!(
        count("production-lines.country_fold") - folds,
        days as usize * followers
    );
    assert_eq!(
        count("production-lines.country_carry") - carries,
        days as usize * 3 * followers
    );
    assert_eq!(
        count("production-lines.country_design") - designs,
        days as usize * 4 * 3
    );
}

/// One line replaced every hour, each by a new handle; the first handle returns at the end.
fn churning_frames(days: i32) -> Vec<DecodedFrame> {
    let last = days * 24 - 1;
    (0..=last)
        .map(|offset| {
            let handle = if offset == last {
                1_000
            } else {
                1_000 + offset as u64
            };
            production_frame(
                FIRST + offset,
                true,
                true,
                9001,
                &[(handle, 70, 8, 0)],
                Some(&[]),
            )
        })
        .collect()
}

#[test]
fn held_keyed_values_stay_flat_however_many_handles_churn() {
    let flag = AtomicBool::new(false);
    let mut peaks = Vec::new();
    for days in [40, 160] {
        let replay = replay_blocks(churning_frames(days), 24);
        let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
        let mut pages = fixtures::FixturePages::default();
        let mut peak = 0;
        for (position, block) in replay.index.iter().enumerate() {
            prepared
                .advance_block(&mut pages, &replay, position, None, &flag)
                .unwrap();
            prepared.publish(&mut pages, &replay, block, &flag).unwrap();
            let held: usize = prepared
                .histories()
                .map(|history| history.state.keys.len())
                .sum();
            peak = peak.max(held);
            assert!(prepared.trees.cached() <= 2 * 4096);
        }
        peaks.push(peak);
        // The returning handle's next appearance came from its tree, numbered as the
        // reference numbers it.
        let expected = super::super::reference::scan(
            &replay,
            &branch_blocks(&replay, 1),
            "GER",
            None,
            None,
            &flag,
        )
        .unwrap();
        assert!(expected.lines.iter().any(|line| line.key == "1000#2"));
        assert_eq!(
            served(&prepared.directory, &replay, "GER", |section, range| pages
                .read_at(section, range)),
            saved_bytes(&expected),
            "{days} days"
        );
    }
    // At most two sweeps' worth of closed handles are held, however long the branch.
    assert!(peaks[1] <= peaks[0], "{peaks:?}");
    assert!(peaks[1] <= 2 * LEDGER_FULL_EVERY * 24 + 8, "{peaks:?}");
}

#[test]
fn ledger_records_stay_bounded_under_recurring_output_gaps() {
    let flag = AtomicBool::new(false);
    let frames = (0..24 * 120)
        .map(|offset| {
            // Every other hour's output goes unread.
            let output = [(5, 70, i64::from(offset), true, false)];
            production_frame(
                FIRST + offset,
                true,
                true,
                9001,
                &[(5, 70, 8, 0)],
                (offset % 2 == 0).then_some(&output[..]),
            )
        })
        .collect();
    let replay = replay_blocks(frames, 24);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    let mut sizes = Vec::new();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        let range = *prepared.ledger_ranges.last().unwrap();
        sizes.push(pages.pages[&(index::ledger_section(1), range)].len());
        assert!(prepared.histories().all(|history| {
            history
                .state
                .stream
                .as_ref()
                .is_none_or(|stream| stream.tail.len() < index::SPAN_PAGE)
        }));
    }
    let quarter = sizes.len() / 4;
    let early = *sizes[..quarter].iter().max().unwrap();
    let late = *sizes[sizes.len() - quarter..].iter().max().unwrap();
    // A record holds counts and cursors; the gaps so far stay in the chunks.
    assert!(late <= early + 64, "{sizes:?}");
    let expected = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "GER",
        None,
        None,
        &flag,
    )
    .unwrap();
    assert!(expected.unread_output.len() > 50);
    prepared = restore(&mut pages, &replay);
    assert_eq!(
        served(&prepared.directory, &replay, "GER", |section, range| pages
            .read_at(section, range)),
        saved_bytes(&expected)
    );
}

/// One line read every day with values that never change: its run stays open to the end.
fn steady_frames(days: i32) -> Vec<DecodedFrame> {
    (0..days * 24)
        .map(|offset| {
            let census = offset % 24 == 0;
            production_frame(
                FIRST + offset,
                census,
                census,
                9001,
                &[(5, 70, 8, 0)],
                Some(&[(5, 70, i64::from(offset), true, false)]),
            )
        })
        .collect()
}

#[test]
fn a_detail_window_decodes_as_many_parts_however_long_the_branch_runs_on() {
    let flag = AtomicBool::new(false);
    let mut firsts = Vec::new();
    for days in [30, 300] {
        let replay = replay_blocks(steady_frames(days), 12);
        let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
        let mut pages = fixtures::FixturePages::default();
        for (position, block) in replay.index.iter().enumerate() {
            prepared
                .advance_block(&mut pages, &replay, position, None, &flag)
                .unwrap();
            prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        }
        let last_hour = replay.index.last().unwrap().last_hour;
        let mut index = read::Index::open(Box::new(pages), prepared.directory.clone()).unwrap();
        let chunks = index.chunks().unwrap();
        let history = index.tag_history("GER");
        let windows = [
            FIRST.div_euclid(DETAIL_HOURS),
            (FIRST + days * 12).div_euclid(DETAIL_HOURS),
            last_hour.div_euclid(DETAIL_HOURS),
        ];
        for (at, window) in windows.into_iter().enumerate() {
            let (first, last) = detail_window(window).unwrap();
            let before = index.decoded;
            let read = index.window(history, first, last, &flag).unwrap();
            let decoded = index.decoded - before;
            let meeting = chunks
                .iter()
                .filter(|chunk| chunk.first <= last && chunk.last >= first)
                .count();
            // The window's own chunks, and a bisection's steps for what it holds open.
            assert!(
                decoded <= 2 * meeting + 48,
                "{days} days, window {window}: {decoded}"
            );
            let run = &read
                .lines
                .iter()
                .find(|line| line.key == "5#1")
                .unwrap()
                .runs[0];
            assert_eq!(run.last_hour, last_hour);
            if at == 0 {
                firsts.push(decoded);
            }
        }
    }
    // Three bisections of at most eleven steps over the longer branch's chunks, and the
    // chunk the last one names.
    assert!(firsts[1] <= firsts[0] + 34, "{firsts:?}");
}

#[test]
fn chunk_table_pages_fill_and_read_back_across_resumes() {
    struct Budget(usize);
    impl Drop for Budget {
        fn drop(&mut self) {
            crate::prepared::pages::TEST_BATCH_BYTES.set(self.0);
        }
    }
    let flag = AtomicBool::new(false);
    let frames = (0..index::CHUNK_PAGE as i32 + 80)
        .map(|offset| {
            production_frame(
                FIRST + offset,
                true,
                true,
                9001,
                &[(5, 70, 8, i64::from(offset % 3))],
                Some(&[(5, 70, i64::from(offset), true, false)]),
            )
        })
        .collect();
    let replay = replay_blocks(frames, 100);
    let expected = super::super::reference::scan(
        &replay,
        &branch_blocks(&replay, 1),
        "GER",
        None,
        None,
        &flag,
    )
    .unwrap();
    let _budget = Budget(crate::prepared::pages::TEST_BATCH_BYTES.replace(1));
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
        prepared = restore(&mut pages, &replay);
    }
    assert!(prepared.chunks > index::CHUNK_PAGE as u64);
    assert!(pages.pages.contains_key(&(index::chunks_section(1), 0)));
    let mut opened = read::Index::open(
        Box::new(fixtures::FixturePages {
            pages: pages.pages.clone(),
        }),
        prepared.directory.clone(),
    )
    .unwrap();
    let chunks = opened.chunks().unwrap();
    assert_eq!(chunks.len() as u64, prepared.chunks);
    assert!(chunks.windows(2).all(|pair| {
        pair[0].range < pair[1].range && pair[0].last.checked_add(1) == Some(pair[1].first)
    }));
    assert_eq!(
        served(&prepared.directory, &replay, "GER", |section, range| pages
            .read_at(section, range)),
        saved_bytes(&expected)
    );
}

#[test]
fn a_commit_whose_chunk_table_does_not_add_up_is_refused_as_invalid_data() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
    }
    let key = (index::commits_section(1), prepared.directory.commits - 1);
    let commit: Commit = index::unpack(&pages.pages[&key]).unwrap();
    let opened = |commit: &Commit| {
        let mut pages = fixtures::FixturePages {
            pages: pages.pages.clone(),
        };
        pages
            .pages
            .insert(key.clone(), index::pack(commit).unwrap());
        read::Index::open(Box::new(pages), prepared.directory.clone()).and_then(|mut index| {
            let total = index.len();
            (0..total).try_for_each(|at| index.chunk(at).map(drop))
        })
    };
    assert!(opened(&commit).is_ok());
    // More chunks than its table holds, and a page that was never written.
    for chunks in [commit.chunks + 1, commit.chunks + index::CHUNK_PAGE as u64] {
        let error = opened(&Commit {
            chunks,
            ..commit.clone()
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{chunks}");
    }
}

#[test]
fn dormant_countries_waking_after_many_gaps_hold_no_copied_spans() {
    use crate::test_fixtures::scale_frame;
    let replay = recording();
    let flag = AtomicBool::new(false);
    let countries = 300;
    let gap = |at: i32| FIRST + 3 + 2 * at;
    let mut held = Vec::new();
    for gaps in [200, 2_000] {
        let mut prepared = Prepared::new(&replay, 1, [], None);
        let mut pages = fixtures::FixturePages::default();
        prepared
            .observe(&mut pages, &replay, &scale_frame(1, FIRST, countries), None)
            .unwrap();
        // All but the first go dormant; the last stays so to the end.
        prepared
            .observe(&mut pages, &replay, &scale_frame(1, FIRST + 1, 1), None)
            .unwrap();
        prepared.flush(&mut pages, &flag).unwrap();
        for at in 0..gaps {
            prepared.lost(gap(at), gap(at)).unwrap();
            if at % 100 == 99 {
                prepared.flush(&mut pages, &flag).unwrap();
            }
        }
        prepared
            .observe(
                &mut pages,
                &replay,
                &scale_frame(1, gap(gaps), countries - 1),
                None,
            )
            .unwrap();
        // Before any flush: the woken countries name the gaps, holding none of them.
        held.push(
            prepared
                .histories()
                .map(|history| {
                    history.state.events.held().overview.unread_output.len()
                        + history
                            .state
                            .stream
                            .as_ref()
                            .map_or(0, |stream| stream.tail.len())
                })
                .sum::<usize>(),
        );
        let woken = prepared
            .native
            .values()
            .filter(|country| country.state.events.held().overview.wakes.len() == 1)
            .count();
        assert_eq!(woken, countries as usize - 2);
        prepared.save(&mut pages, &flag).unwrap();
        // A woken country's view and the one still dormant read every gap, as copied ones did.
        let mut index = read::Index::open(Box::new(pages), prepared.directory.clone()).unwrap();
        let expected: Vec<(i32, i32)> = std::iter::once((FIRST, FIRST))
            .chain((0..gaps).map(|at| (gap(at), gap(at))))
            .collect();
        for slot in [1, countries / 2, countries - 1] {
            let history = index
                .native_history(&NativeCountry {
                    slot,
                    id: 100_000 + slot,
                    epoch: 1,
                })
                .unwrap();
            let overview = index.overview(history, &flag).unwrap();
            let spans: Vec<(i32, i32)> = overview
                .unread_output
                .iter()
                .map(|span| (span.first_hour, span.last_hour))
                .collect();
            assert_eq!(spans, expected, "{gaps} gaps, slot {slot}");
            assert_eq!(overview.counters.unreadable_stretches, gaps as usize);
        }
    }
    // The absence's partial page and the spans since the last flush, however many gaps.
    assert!(held[1] <= held[0] + index::SPAN_PAGE, "{held:?}");
    assert!(held[1] < 100 + index::SPAN_PAGE, "{held:?}");
}

#[test]
fn a_malformed_absence_count_is_refused_as_invalid_data_on_resume_and_on_reading() {
    let replay = recording();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, ["GER".into()], None);
    let mut pages = fixtures::FixturePages::default();
    for (position, block) in replay.index.iter().enumerate() {
        prepared
            .advance_block(&mut pages, &replay, position, None, &flag)
            .unwrap();
        prepared.publish(&mut pages, &replay, block, &flag).unwrap();
    }
    let commit_key = (index::commits_section(1), prepared.directory.commits - 1);
    let commit: Commit = index::unpack(&pages.pages[&commit_key]).unwrap();
    let span = |at: i32| HourSpan {
        first_hour: at,
        last_hour: at,
    };
    let whole_page: Vec<HourSpan> = (0..index::SPAN_PAGE as i32).map(span).collect();
    // An absence that counts no span yet holds a whole page of them.
    let mut broken = fixtures::FixturePages {
        pages: pages.pages.clone(),
    };
    for &range in &commit.ledger {
        let key = (index::ledger_section(1), range);
        let mut record: LedgerRecord = index::unpack(&broken.pages[&key]).unwrap();
        let absent = record.absent;
        for (id, history) in &mut record.histories {
            if *id == absent
                && let StateLedger::Loaded { core, .. } = &mut history.state
            {
                core.stream = Some((0, whole_page.clone()));
            }
        }
        broken.pages.insert(key, index::pack(&record).unwrap());
    }
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    for ((section, range), bytes) in &broken.pages {
        writer.write_at(section, *range, bytes.as_slice()).unwrap();
    }
    writer.commit().unwrap();
    let error = Prepared::resume(&fixture.store, &fixture.key, &replay, 1, None)
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // A commit whose absence count its tail does not follow is refused when read.
    for (spans, span_tail) in [
        (0, whole_page.clone()),
        (5, vec![span(1), span(2), span(3)]),
    ] {
        let mut broken = fixtures::FixturePages {
            pages: pages.pages.clone(),
        };
        broken.pages.insert(
            commit_key.clone(),
            index::pack(&Commit {
                spans,
                span_tail,
                ..commit.clone()
            })
            .unwrap(),
        );
        let error = read::Index::open(Box::new(broken), prepared.directory.clone())
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{spans}");
    }
}
