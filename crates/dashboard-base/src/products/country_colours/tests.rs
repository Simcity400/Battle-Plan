use super::{CountryColourParams, CountryColourReply, CountryColours, OVER_BUDGET, load_within};
use crate::prepared::query::frames;
use crate::products::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_styles::{CountryStyleParams, CountryStyles, HourStyles, STYLE_TABLES},
};
use crate::test_fixtures::{CAMPAIGN, continuation_records, file_source, records, source};
use replay::container::{Replay, Section, SeekResult};
use replay_data::rows::{CountryAppearanceRow, CountryRow, PoliticsRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// GER is recorded; ITA is a country with no appearance row; XXX is no country.
const TAGS: [&str; 3] = ["GER", "ITA", "XXX"];

/// An hour where GER has `colour` (as the engine's floats), or no appearance table.
fn frame(hour: i32, colour: Option<[f32; 3]>) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"ITA".to_vec(),
            ..Default::default()
        });
    }
    *frame.tables[TableId::CountryAppearance.index()] = match colour {
        Some([red, green, blue]) => {
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 9001,
                red: red.to_bits(),
                green: green.to_bits(),
                blue: blue.to_bits(),
                ..Default::default()
            }]))
        }
        // Its reason reaches the reply, so the budget is measured with escaped text.
        None => TableView::Unavailable {
            reason: "fixture \"held\" back\\slash".into(),
        },
    };
    // A naming table country-styles reads, so its sections exist to damage.
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 9001,
            last_election: hour,
            ..Default::default()
        }]));
    frame
}

const RED: Option<[f32; 3]> = Some([0.2, 0.5, 1.5]);

fn replay(frames: Vec<DecodedFrame>) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

fn memory(replay: Replay) -> crate::Source {
    crate::Source::from_replay(
        crate::SourceKey {
            source_id: "colours".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn params(hours: &[i32]) -> CountryColourParams {
    CountryColourParams {
        branch_id: "1".into(),
        hours: hours.to_vec(),
        tags: TAGS.iter().map(|tag| (*tag).to_owned()).collect(),
    }
}

fn colours(source: &crate::Source, hours: &[i32]) -> CountryColourReply {
    let context = LoadContext { content: None };
    CountryColours::load(source, &params(hours), &context, &AtomicBool::new(false)).unwrap()
}

/// A fresh copy of `replay`, so an oracle reads nothing another request cached.
fn fresh(replay: &Replay) -> Replay {
    let mut copy = Replay::open(replay.bytes().to_vec()).unwrap();
    copy.campaign_id.clone_from(&replay.campaign_id);
    copy
}

/// Every hour reads as a direct seek of a fresh replay reads it (the original
/// oracle): unread with the seek's own reason where it fails, else the colour,
/// or the reason for none, that hour's styles give.
fn assert_seek_parity(replay: &Replay, hours: &[i32]) -> CountryColourReply {
    let reply = colours(&memory(fresh(replay)), hours);
    let oracle = fresh(replay);
    for &hour in hours {
        let unread = reply.unread.iter().find(|unread| unread.hour == hour);
        let reason = match oracle.seek_selected(1, hour, Some(STYLE_TABLES)) {
            Ok(SeekResult::Frame(frame)) => {
                assert!(
                    unread.is_none(),
                    "hour {hour} seeks but is unread: {unread:?}"
                );
                let styles = HourStyles::read(&frame, None).unwrap();
                for tag in TAGS {
                    let run = reply
                        .runs
                        .iter()
                        .rfind(|run| run.tag == tag && run.hour <= hour)
                        .unwrap();
                    let colour = styles.colour(tag);
                    assert_eq!(run.color, colour.clone().ok(), "{tag} at {hour}");
                    assert_eq!(run.missing, colour.err(), "{tag} at {hour}");
                }
                continue;
            }
            Ok(SeekResult::Gap { reason, .. }) => reason,
            Err(error) => error.to_string(),
        };
        assert_eq!(
            unread.map(|unread| unread.reason.as_str()),
            Some(reason.as_str()),
            "hour {hour}"
        );
    }
    reply
}

/// Every hour reads as `country-styles` reads it: unread exactly where that fails,
/// else the same colour, and for none the reason it warns of.
fn assert_parity(source: &crate::Source, hours: &[i32]) -> CountryColourReply {
    let reply = colours(source, hours);
    let context = LoadContext { content: None };
    for &hour in hours {
        let unread = reply.unread.iter().any(|unread| unread.hour == hour);
        let styles = CountryStyles::load(
            source,
            &CountryStyleParams {
                branch_id: "1".into(),
                hour,
                tags: TAGS.iter().map(|tag| (*tag).to_owned()).collect(),
            },
            &context,
            &AtomicBool::new(false),
        );
        let Ok(styles) = styles else {
            assert!(unread, "hour {hour} is read here but not by country-styles");
            continue;
        };
        assert!(
            !unread,
            "hour {hour} is read by country-styles but not here"
        );
        for style in &styles.styles {
            let run = reply
                .runs
                .iter()
                .rfind(|run| run.tag == style.tag && run.hour <= hour)
                .unwrap();
            assert_eq!(run.color, style.color, "{} at {hour}", style.tag);
            match &run.missing {
                Some(why) => assert!(
                    styles.warnings.contains(&format!("{}: {why}", style.tag)),
                    "{} at {hour}: {why} against {:?}",
                    style.tag,
                    styles.warnings
                ),
                None => assert!(style.color.is_some()),
            }
        }
    }
    reply
}

#[test]
fn colours_come_as_changes_with_black_apart_from_missing() {
    let source = memory(replay(vec![
        frame(100, RED),
        frame(101, None),
        frame(102, Some([0.0, 0.0, 0.0])),
        frame(103, RED),
    ]));
    let reply = assert_parity(&source, &[103, 100, 102, 101, 100, 50, 500]);
    let at = |tag: &str, hour: i32| {
        reply
            .runs
            .iter()
            .rfind(|run| run.tag == tag && run.hour <= hour)
            .unwrap()
            .clone()
    };
    assert_eq!(at("GER", 100).color, Some([51, 127, 255]));
    // Black is a recorded colour; a country without an appearance row has none, and says why.
    assert_eq!(
        (at("GER", 102).color, at("GER", 102).missing),
        (Some([0, 0, 0]), None)
    );
    assert_eq!(at("ITA", 102).color, None);
    assert_eq!(
        at("ITA", 102).missing.as_deref(),
        Some("no recorded map colour")
    );
    assert!(
        at("GER", 101)
            .missing
            .unwrap()
            .contains("appearance table is incomplete")
    );
    assert_eq!(
        at("XXX", 100).missing.as_deref(),
        Some("not a country at this hour")
    );
    // An unchanged colour starts no run.
    assert!(
        !reply
            .runs
            .iter()
            .any(|run| run.hour == 103 && run.tag == "ITA")
    );
    let mut unread: Vec<_> = reply.unread.iter().map(|unread| unread.hour).collect();
    unread.sort_unstable();
    assert_eq!(unread, [50, 500]);
}

#[test]
fn an_hour_missing_inside_a_block_is_unread_as_country_styles_finds_it() {
    let source = memory(replay(vec![
        frame(100, RED),
        frame(101, RED),
        frame(103, RED),
    ]));
    let reply = assert_parity(&source, &[100, 101, 102, 103]);
    assert_eq!(
        reply
            .unread
            .iter()
            .map(|unread| unread.hour)
            .collect::<Vec<_>>(),
        [102]
    );
}

#[test]
fn hours_across_blocks_and_outside_the_recording() {
    let first = replay(vec![frame(100, RED), frame(101, RED)]);
    let joined = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(200, Some([0.4, 0.5, 1.5])), frame(201, None)],
        )])
        .unwrap();
    assert!(joined.index.len() >= 2);
    let source = memory(joined);
    let reply = assert_parity(&source, &[50, 100, 101, 150, 200, 201, 300]);
    let mut unread: Vec<_> = reply.unread.iter().map(|unread| unread.hour).collect();
    unread.sort_unstable();
    assert_eq!(unread, [50, 150, 300]);
}

#[test]
fn blocks_read_ahead_answer_as_each_hour_seeks_in_hour_order() {
    let mut joined = replay(vec![frame(100, RED), frame(101, None)]);
    for (start, colour) in [
        (200, Some([0.4, 0.5, 1.5])),
        (300, RED),
        (400, None),
        (500, Some([0.9, 0.1, 0.2])),
    ] {
        joined = joined
            .append(&[continuation_records(
                &joined,
                1,
                vec![frame(start, colour), frame(start + 1, RED)],
            )])
            .unwrap();
    }
    assert!(joined.index.len() >= 5);
    let hours = [100, 101, 150, 200, 201, 300, 301, 400, 401, 500, 501, 600];
    let reply = assert_seek_parity(&joined, &hours);
    assert_parity(&memory(fresh(&joined)), &hours);
    assert!(
        reply
            .runs
            .windows(2)
            .all(|pair| pair[0].hour <= pair[1].hour)
    );
}

/// `replay` with the last byte of `section` of its first block flipped.
fn damaged_section(replay: &Replay, section: Section) -> Replay {
    let block = &replay.index[0];
    let sections = replay
        .block_sections(block)
        .unwrap()
        .expect("a grouped synthetic block");
    let stored: u64 = sections.iter().map(|size| size.compressed).sum();
    let mut offset = block.offset + block.length - stored;
    for size in sections {
        if size.section == section {
            let mut bytes = replay.bytes().to_vec();
            bytes[usize::try_from(offset + size.compressed - 1).unwrap()] ^= 1;
            return Replay::open(bytes).unwrap();
        }
        offset += size.compressed;
    }
    panic!("the synthetic block has no {section:?} section");
}

#[test]
fn a_damaged_naming_table_is_unread_as_country_styles_reads_it() {
    let whole = replay(vec![frame(100, RED), frame(101, RED), frame(102, RED)]);
    // Politics names countries; colours alone would not need it, but country-styles reads it.
    let source = memory(damaged_section(
        &whole,
        Section::Keyframe(TableId::Politics),
    ));
    let reply = assert_parity(&source, &[100, 101, 102]);
    assert_eq!(reply.unread.len(), 3);
    assert!(reply.runs.is_empty());
    assert_seek_parity(
        &damaged_section(&whole, Section::Keyframe(TableId::Politics)),
        &[100, 101, 102],
    );
}

#[test]
fn damage_after_the_first_hour_reads_what_a_direct_seek_reads() {
    let whole = replay(vec![frame(100, RED), frame(101, RED), frame(102, RED)]);
    // Only hours after the keyframe need the later Politics updates.
    let damaged = damaged_section(&whole, Section::Later(TableId::Politics));
    assert_seek_parity(&damaged, &[100, 101, 102]);
    assert_parity(&memory(fresh(&damaged)), &[100, 101, 102]);
}

#[test]
fn hours_at_and_past_the_branch_end_read_what_a_direct_seek_reads() {
    let whole = replay(vec![frame(100, RED), frame(101, None), frame(102, RED)]);
    let reply = assert_seek_parity(&whole, &[101, 102, 103, 104]);
    let mut unread: Vec<_> = reply.unread.iter().map(|unread| unread.hour).collect();
    unread.sort_unstable();
    assert_eq!(unread, [103, 104]);
}

#[test]
fn every_hour_after_a_campaign_mismatch_says_so_as_a_seek_does() {
    let mut mismatched = replay(vec![frame(100, RED), frame(101, RED), frame(102, RED)]);
    mismatched.campaign_id = b"another campaign".to_vec();
    let reply = assert_seek_parity(&mismatched, &[100, 101, 102]);
    // Not "Hour not recorded" for the hours after the first: each names the mismatch.
    assert_eq!(
        reply
            .unread
            .iter()
            .map(|unread| (unread.hour, unread.reason.as_str()))
            .collect::<Vec<_>>(),
        [100, 101, 102].map(|hour| (hour, "frame campaign identity differs from replay header"))
    );
    assert!(reply.runs.is_empty());
    assert_parity(&memory(fresh(&mismatched)), &[100, 101, 102]);
}

#[test]
fn a_malformed_block_tail_reads_what_a_seek_reads() {
    let whole = replay(vec![frame(100, RED), frame(101, None), frame(102, RED)]);
    let block = &whole.index[0];
    let mut bytes = whole.bytes().to_vec();
    bytes[usize::try_from(block.offset + block.length - 1).unwrap()] ^= 1;
    let damaged = Replay::open(bytes).unwrap();
    assert_seek_parity(&damaged, &[100, 101, 102]);
    assert_parity(&memory(damaged), &[100, 101, 102]);
}

#[test]
fn one_block_is_read_once_and_shared_with_country_styles() {
    let source = memory(replay(vec![
        frame(100, RED),
        frame(101, RED),
        frame(102, None),
        frame(103, RED),
    ]));
    colours(&source, &[100, 101, 102, 103]);
    // One block read, no hour sought on its own.
    assert_eq!(frames::reads(&source), (1, 0));
    // The same hours again, and country-styles at one of them, reuse what was read.
    colours(&source, &[100, 101, 102, 103]);
    let context = LoadContext { content: None };
    CountryStyles::load(
        &source,
        &CountryStyleParams {
            branch_id: "1".into(),
            hour: 101,
            tags: vec!["GER".into()],
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(frames::reads(&source), (1, 0));
}

#[test]
fn a_sparse_recording_names_its_gaps_from_one_pass_over_its_file() {
    // Every sixth hour recorded, an hour asked of each day: most of them are gaps.
    let sparse = replay((0..60).map(|step| frame(100 + step * 6, RED)).collect());
    assert_eq!(sparse.index.len(), 1);
    let hours: Vec<i32> = (0..16).map(|day| 101 + day * 23).collect();
    let (source, root) = file_source(fresh(&sparse));
    let reply = colours(&source, &hours);
    assert!(reply.unread.len() > hours.len() / 2);
    // One read of the block; no hour, gap or not, read or rebuilt on its own.
    assert_eq!(frames::reads(&source), (1, 0));
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
    // Each gap is named as a direct seek of its hour names it.
    assert_seek_parity(&sparse, &hours);
}

#[test]
fn a_reply_fits_a_budget_of_exactly_its_size_and_not_a_byte_less() {
    let source = memory(replay(vec![
        frame(100, RED),
        frame(101, None),
        frame(102, RED),
    ]));
    // Runs and unread hours both, with escaped text in the reasons.
    let hours = [100, 101, 102, 500, 600];
    let whole = load_within(
        &source,
        &params(&hours),
        &AtomicBool::new(false),
        MAX_PRODUCT_REPLY_BYTES,
    )
    .unwrap();
    assert!(!whole.runs.is_empty() && whole.unread.len() == 2);
    let json = serde_json::to_string(&whole).unwrap();
    assert!(json.contains("\\\"held\\\""), "{json}");
    let fitted = load_within(
        &source,
        &params(&hours),
        &AtomicBool::new(false),
        json.len(),
    )
    .unwrap();
    assert_eq!(serde_json::to_string(&fitted).unwrap(), json);
    let refused = load_within(
        &source,
        &params(&hours),
        &AtomicBool::new(false),
        json.len() - 1,
    )
    .unwrap_err();
    assert!(refused.to_string().starts_with(OVER_BUDGET), "{refused}");
}

#[test]
fn cancelling_while_a_block_is_read_stops_the_reply() {
    let source = memory(replay(vec![frame(100, RED), frame(101, RED)]));
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    frames::ON_RELEASE.set(Some(Box::new(move || flag.store(true, Ordering::Relaxed))));
    let result = load_within(
        &source,
        &params(&[100, 101]),
        &cancelled,
        MAX_PRODUCT_REPLY_BYTES,
    );
    frames::ON_RELEASE.set(None);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
}

#[test]
fn a_selection_needs_hours_and_countries() {
    let valid = params(&[100]);
    assert!(CountryColours::validate(&valid).is_ok());
    for invalid in [
        CountryColourParams {
            hours: vec![],
            ..valid.clone()
        },
        CountryColourParams {
            tags: vec![],
            ..valid.clone()
        },
        CountryColourParams {
            branch_id: "x".into(),
            ..valid.clone()
        },
    ] {
        assert!(CountryColours::validate(&invalid).is_err());
    }
}
