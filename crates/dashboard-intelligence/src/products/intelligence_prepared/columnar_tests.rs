//! The family's columnar preparation held to hourly preparation on the same hours, written as
//! the native recorder writes them: every product's replies from the pages the emit publishes
//! equal those of the observers told each decoded frame and its change facts block by block,
//! across segment starts, a gap, batches flushed partway through a block (one where a day ends)
//! and taken up by the espionage log's history, and fields no observer reads moving every hour.

use super::{
    Content, Observer, TABLES,
    emit::{Emit, Inputs, Pages},
    facts::{self, Projector},
};
use crate::products::{
    agency_comparison::{self, AgencyComparisonParams},
    decryption,
    espionage_log::{self, EspionageLogParams},
    operative_placements, operatives_roster,
};
use dashboard_base::{
    prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture},
    source::Source,
    test_fixtures::{continuation_records, file_source, replay, source},
};
use dashboard_core::config::LoadContext;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        AgencyUpgradeLevelRow, AutonomyRow, CharacterOperativeRow, CharacterPortraitRow,
        CountryAppearanceRow, CountryLinkRow, CountryRow, CryptologyRow, IntelligenceAgencyRow,
        LexicalTokenRow, OperationAssignmentRow, OperationCompletionRow, OperationRow,
        OperativeMembershipRow, PoliticalPartyRow, PoliticsRow, RaidIntelInputRow,
        ReservedCountryNamingRow, RowCodec, SimulationFeatureRow, TableId,
    },
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use serde_json::Value;
use std::{io, sync::atomic::AtomicBool};

/// The operation's definition token, and its spelling.
const OPERATION: u32 = 70;

thread_local! {
    /// Operations read every hour, its preparation ticking each hour, as the recorder reads it.
    static HOURLY_OPERATIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The hours, as offsets from the membership cadence's day start: 0 to 95 without 45 to 47.
fn offsets() -> Vec<i32> {
    (0..96).filter(|hour| !(45..=47).contains(hour)).collect()
}

fn base() -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::OperativeMemberships)
    else {
        panic!("daily membership");
    };
    24_000 + i32::try_from(slot).unwrap()
}

fn put<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: &[R], read: bool) {
    walk.table(
        table,
        Input::Snapshot {
            rows,
            partial: None,
            read,
            sample: Sample::FromReadBit,
        },
    )
    .unwrap();
}

/// One hour as the recorder writes it. A's agency watches B's ciphers from the start and C's
/// from hour 30, decrypting from 50; B is renamed at 40. A second operative serves from 20 to
/// 59. One operation, prepared daily, runs until 70 and completes at 71. Every country's
/// command power, every
/// raid input's defense, the agencies' upgrade progress and the debug byte move every hour.
fn write(walk: &mut Synthetic, hour: i32) {
    let at = base() + hour;
    let daily = hour % 24 == 0;
    let hourly = HOURLY_OPERATIONS.get();
    let b = if hour >= 40 { "BBX" } else { "BBB" };
    let countries: Vec<CountryRow> = [(7, 1, "AAA"), (9, 2, b), (11, 3, "CCC")]
        .into_iter()
        .map(|(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            command_power_raw: i64::from(hour),
            ..Default::default()
        })
        .collect();
    put(walk, TableId::Countries, &countries, false);
    let links: Vec<CountryLinkRow> = [(7, 1), (9, 2), (11, 3), (17, 1)]
        .into_iter()
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect();
    put(walk, TableId::CountryLinks, &links, false);
    let mut ciphers = vec![CryptologyRow {
        owner_country: 7,
        target_country: 9,
        amount_raw: i64::from(hour / 24) * 1_000,
        bonus_days: -1,
        bonus_active: 0,
        decrypting: u8::from(hour >= 50),
    }];
    if hour >= 30 {
        ciphers.push(CryptologyRow {
            owner_country: 7,
            target_country: 11,
            amount_raw: i64::from((hour - 30) / 24) * 500,
            bonus_days: -1,
            bonus_active: 0,
            decrypting: 0,
        });
    }
    put(walk, TableId::Cryptology, &ciphers, false);
    let raids: Vec<RaidIntelInputRow> = [7, 9, 11]
        .into_iter()
        .map(|country| RaidIntelInputRow {
            country,
            country_defense: i64::from(hour),
            crypto_strength: Some(150_000),
            ..Default::default()
        })
        .collect();
    put(walk, TableId::RaidIntelInputs, &raids, false);
    let features = [SimulationFeatureRow {
        dlc_mask: 1 << 5,
        human_ai: u8::from(hour % 2 == 0),
        ..Default::default()
    }];
    put(walk, TableId::SimulationFeatures, &features, false);
    let mut members = vec![OperativeMembershipRow {
        id: 1,
        roster_country: 7,
        owner_country: 7,
        assignment: 3,
        name: b"Literal operative".to_vec().into(),
        skill: Some(2),
        experience_raw: 123_456,
        captured_hour: engine_native::cgame_date::NULL_RAW_HOUR,
        ..Default::default()
    }];
    if (20..60).contains(&hour) {
        members.push(OperativeMembershipRow {
            id: 2,
            ..members[0].clone()
        });
    }
    put(walk, TableId::OperativeMemberships, &members, daily);
    put::<CharacterOperativeRow>(walk, TableId::CharacterOperatives, &[], false);
    put::<CharacterPortraitRow>(walk, TableId::CharacterPortraits, &[], false);
    let agencies: Vec<IntelligenceAgencyRow> = [7, 9]
        .into_iter()
        .map(|country| IntelligenceAgencyRow {
            country,
            upgrade_progress_raw: i64::from(hour),
            defense_raw: i64::from(hour / 24) * 10,
            active_count: 1,
            name: b"Agency".to_vec(),
            created: 1,
            ..Default::default()
        })
        .collect();
    put(walk, TableId::IntelligenceAgencies, &agencies, false);
    put::<AgencyUpgradeLevelRow>(walk, TableId::AgencyUpgradeLevels, &[], false);
    let tokens = [LexicalTokenRow {
        token: OPERATION,
        literal: b"operation_infiltrate".to_vec(),
    }];
    put(walk, TableId::LexicalTokens, &tokens, false);
    let operations: Vec<OperationRow> = (hour < 71)
        .then(|| OperationRow {
            id: 1,
            owner: 7,
            target: 9,
            definition_token: OPERATION,
            bonus: 0.into(),
            detected: 0.into(),
            effect_description: Vec::new().into(),
            preparation_current: if hourly {
                i64::from(hour)
            } else {
                i64::from(hour / 24) * 10
            },
            preparation_required: 100,
            ..Default::default()
        })
        .into_iter()
        .collect();
    put(walk, TableId::Operations, &operations, daily || hourly);
    put::<OperationAssignmentRow>(walk, TableId::OperationAssignments, &[], false);
    let completions: Vec<OperationCompletionRow> = (hour == 71)
        .then(|| OperationCompletionRow {
            owner: 7,
            sequence: 1,
            id: 1,
            target: 9,
            definition_token: OPERATION,
            completion_hour: at,
            ..Default::default()
        })
        .into_iter()
        .collect();
    put(walk, TableId::OperationCompletions, &completions, false);
    put::<CountryAppearanceRow>(walk, TableId::CountryAppearance, &[], false);
    put::<ReservedCountryNamingRow>(walk, TableId::ReservedCountryNaming, &[], false);
    put::<PoliticsRow>(walk, TableId::Politics, &[], false);
    put::<PoliticalPartyRow>(walk, TableId::PoliticalParties, &[], false);
    put::<AutonomyRow>(walk, TableId::Autonomy, &[], false);
}

/// The projected hours, a projector starting cold at each of `starts` and at each walk's first
/// hour (after the gap, the walk starts again), and the decoded frames of the same hours.
fn walked(starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new().unwrap();
    let needs = facts::needs().unwrap();
    for (ordinal, hour) in (0..).zip(offsets()) {
        let at = base() + hour;
        let gap = frames.last().is_some_and(|last| last.hour + 1 != at);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(TABLES, 1, at).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, hour);
        let cold = starts.contains(&hour);
        if cold {
            projector = Projector::new().unwrap();
        }
        let decoded = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cold;
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &needs,
                    (FormatVersion::CURRENT, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: decoded.hour,
                            run: 0,
                            ordinal,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                            world,
                        },
                        &mut out,
                    )
                    .unwrap();
                decoded.clone()
            })
            .unwrap();
        frames.push(decoded);
    }
    (out, frames)
}

/// The recording the hours are of, in replay blocks of `length` hours, and one ending at the gap.
fn recording(length: usize) -> (Source, std::path::PathBuf) {
    let hours: Vec<i32> = offsets().into_iter().map(|hour| base() + hour).collect();
    let mut blocks: Vec<Vec<i32>> = Vec::new();
    for hour in hours {
        match blocks.last_mut() {
            Some(block) if block.len() < length && block.last() == Some(&(hour - 1)) => {
                block.push(hour)
            }
            _ => blocks.push(vec![hour]),
        }
    }
    let frames = |block: &[i32]| block.iter().map(|&hour| source(1, hour, Some(1))).collect();
    let mut recorded = replay(frames(&blocks[0]));
    for block in &blocks[1..] {
        recorded = recorded
            .append(&[continuation_records(&recorded, 1, frames(block))])
            .unwrap();
    }
    file_source(recorded)
}

/// The family's store, as its stage's batches commit into it.
struct Staged {
    store: std::sync::Arc<store::Store>,
    key: store::SourceKey,
    writer: Option<store::Writer>,
    opened: bool,
}

impl Pages for Staged {
    fn writer(&mut self) -> io::Result<&mut store::Writer> {
        if self.writer.is_none() {
            let writer = if self.opened {
                self.store.extend(&self.key)?
            } else {
                self.store.replace(self.key.clone())?
            };
            self.opened = true;
            self.writer = Some(writer);
        }
        Ok(self.writer.as_mut().expect("an open writer"))
    }
}

impl Staged {
    fn commit(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.commit().unwrap();
        }
    }
}

/// Every product's reply over the whole branch and at each day.
fn replies(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    days: usize,
) -> Vec<(String, Value)> {
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let first = base();
    let spans = (0..days)
        .map(|day| {
            let from = first + i32::try_from(day).unwrap() * 24;
            (Some(from), Some(from + 23))
        })
        .chain([(None, None)]);
    let mut out = Vec::new();
    let mut keep = |label: String, value: Value| {
        assert!(!value.is_null(), "{label} answers");
        out.push((label, value));
    };
    for (first_hour, last_hour) in spans {
        let label = |product: &str| format!("{product} {first_hour:?}..{last_hour:?}");
        let reply = decryption::prepared::query(
            store,
            key,
            1,
            &decryption::prepared::Params {
                branch_id: "1".into(),
                pairs: Vec::new(),
                first_hour,
                last_hour,
            },
            &flag,
        )
        .unwrap();
        keep(label("decryption"), serde_json::to_value(reply).unwrap());
        let reply = operative_placements::prepared::query(
            store,
            key,
            1,
            &operative_placements::prepared::Params {
                branch_id: "1".into(),
                first_hour,
                last_hour,
            },
            &flag,
        )
        .unwrap();
        keep(label("placements"), serde_json::to_value(reply).unwrap());
        let reply = agency_comparison::prepared::query(
            source,
            &context,
            store,
            key,
            1,
            &agency_comparison::prepared::Params {
                selection: AgencyComparisonParams {
                    branch_id: "1".into(),
                    tags: Vec::new(),
                    include_uncreated: true,
                    top: None,
                },
                first_hour,
                last_hour,
            },
            &flag,
        )
        .unwrap();
        keep(label("agencies"), serde_json::to_value(reply).unwrap());
        let reply = operatives_roster::prepared::query(
            source,
            &context,
            store,
            key,
            1,
            &operatives_roster::prepared::Params {
                branch_id: "1".into(),
                tags: vec!["AAA".into()],
                first_hour,
                last_hour,
                include_departed: true,
            },
            &flag,
        )
        .unwrap();
        keep(label("roster"), serde_json::to_value(reply).unwrap());
    }
    for window in (0..days).map(Some).chain([None]) {
        let reply = espionage_log::prepared::query(
            source,
            &context,
            store,
            key,
            1,
            &EspionageLogParams {
                branch_id: "1".into(),
                window,
            },
            &flag,
        )
        .unwrap();
        keep(
            format!("espionage {window:?}"),
            serde_json::to_value(reply).unwrap(),
        );
    }
    out
}

#[test]
fn columnar_preparation_publishes_what_hourly_preparation_does() {
    held_to_hourly_preparation(10);
}

/// A batch flushed partway through a block publishes the espionage days its coverage reaches:
/// the block holding hours 0 to 44 is not finished at hour 23 or 35, and day 0 answers from it.
#[test]
fn a_batch_ending_partway_through_a_block_publishes_the_days_it_covers() {
    held_to_hourly_preparation(48);
}

/// Operations read every hour with its preparation ticking: the roster, which reads only each
/// operation's definition, is not observed again at those ticks and publishes the same replies.
#[test]
fn an_hourly_operations_read_of_unread_fields_publishes_what_hourly_preparation_does() {
    HOURLY_OPERATIONS.set(true);
    held_to_hourly_preparation(10);
}

fn held_to_hourly_preparation(length: usize) {
    let (source, root) = recording(length);
    let context = LoadContext { content: None };
    let content = Content::load(&source, &context).unwrap();
    let flag = AtomicBool::new(false);
    let (hours, frames) = walked(&[30, 77]);
    assert_eq!(hours.len(), frames.len());
    // Hourly preparation: every decoded frame told its change facts, each block finished.
    let reference = Fixture::new();
    let mut observer = Observer::default();
    let mut tracker = FrameChangeTracker::new();
    let mut writer = reference.store.replace(reference.key.clone()).unwrap();
    let mut frames = frames.iter().peekable();
    for (ordinal, window) in source.block_windows(1).iter().enumerate() {
        while let Some(frame) = frames.next_if(|frame| frame.hour <= window.last_hour) {
            let changes = tracker.observe(frame, FormatVersion::CURRENT);
            observer
                .observe(
                    &source,
                    &content,
                    frame,
                    (&changes, None),
                    &|_| false,
                    Some((&reference.store, &reference.key)),
                    &flag,
                )
                .unwrap();
        }
        observer
            .finish_block(&source, &mut writer, 1, ordinal, &flag)
            .unwrap();
    }
    assert!(frames.next().is_none());
    observer.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    // The emit, its batch flushed and committed partway through the block holding hour 35.
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    let mut pages = Staged {
        store,
        key,
        writer: None,
        opened: false,
    };
    let inputs = Inputs {
        source: &source,
        context: &context,
        content: &content,
    };
    let mut emit = Emit::new(1);
    let last = hours.last().unwrap().hour;
    for hour in hours {
        let at = hour.hour;
        emit.observe(inputs, hour, &mut pages).unwrap();
        // A batch ends where day 0 does, and partway through the next day.
        if at == base() + 23 || at == base() + 35 {
            emit.flush(inputs, pages.writer().unwrap(), None, &flag)
                .unwrap();
            assert!(emit.save().is_none(), "partway through a block");
            pages.commit();
            let day = espionage_log::prepared::query(
                &source,
                &context,
                &pages.store,
                &pages.key,
                1,
                &EspionageLogParams {
                    branch_id: "1".into(),
                    window: Some(0),
                },
                &flag,
            )
            .unwrap()
            .expect("day 0 is published with the batch covering it");
            assert_eq!(day.window, Some(0));
        }
    }
    emit.flush(inputs, pages.writer().unwrap(), Some(last), &flag)
        .unwrap();
    assert!(emit.save().is_some(), "every block finished");
    pages.commit();
    let days = 4;
    let expected = replies(&source, &reference.store, &reference.key, days);
    let actual = replies(&source, &pages.store, &pages.key, days);
    assert_eq!(actual.len(), expected.len());
    for ((label, actual), (_, expected)) in actual.iter().zip(&expected) {
        assert_eq!(actual, expected, "{label}");
    }
    let _ = std::fs::remove_dir_all(root);
}

/// Of hours that change nothing the observers read, the projector hands on no view, and tells
/// only of the agencies and the fields no observer reads.
#[test]
fn hours_that_change_only_unread_fields_hand_on_no_view() {
    let (hours, _) = walked(&[]);
    let position = |table: TableId| TABLES.iter().position(|&read| read == table).unwrap();
    let agencies = 1 << position(TableId::IntelligenceAgencies);
    // Hour 2 follows hour 1 with nothing the observers read changed.
    let quiet = &hours[2];
    assert!(!quiet.opens);
    assert!(quiet.views.is_empty(), "no view is handed on");
    assert_eq!(quiet.changed, agencies, "only the agencies' rebuild is told");
    for table in [
        TableId::Countries,
        TableId::RaidIntelInputs,
        TableId::SimulationFeatures,
        TableId::IntelligenceAgencies,
    ] {
        assert_ne!(quiet.rebuilt & (1 << position(table)), 0, "{table:?} rebuilt");
    }
    assert_eq!(quiet.reads, 0);
    // The day's first hour reads the memberships and operations, which changed.
    let day = hours.iter().find(|hour| hour.hour == base() + 24).unwrap();
    assert_eq!(day.reads, facts::MEMBERSHIPS_READ | facts::OPERATIONS_READ);
    assert!(
        day.views
            .iter()
            .any(|(table, _)| *table == TableId::Operations)
    );
    // The first hour after the gap opens: every table is told changed, and each whole table
    // the walk built again is handed on.
    let reopened = hours.iter().find(|hour| hour.hour == base() + 48).unwrap();
    assert!(reopened.opens);
    assert_eq!(reopened.changed, (1 << TABLES.len()) - 1);
    assert!(
        reopened
            .views
            .iter()
            .any(|(table, _)| *table == TableId::CountryLinks)
    );
}
