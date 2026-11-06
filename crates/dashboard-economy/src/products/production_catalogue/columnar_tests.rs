//! The family's columnar preparation held to its row census ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every view's reply, for every country and for
//! each alone, in every language, across segment starts, a gap, cuts and a resume. The row
//! census observes the hours as the walk does, by their changes within each day's block; the
//! countries' hourly balances make it read every domain every hour. The hours are written read
//! every hour, and with the daily tables read only at their daily slots and carried between.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    row::{
        self, Prepared,
        equivalence::{churn, languages, same_inputs},
    },
    *,
};
use crate::products::test_support::fixture;
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use dashboard_series::{
    Blob, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{TableId, TableSchema, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, FrameDecoder, FrameEncoder, TableView};
use serde_json::Value;
use std::sync::{Arc, atomic::AtomicBool};

/// The hour whose rows `table` holds at `hour` of a walk from `first`, and whether it is read
/// then: every hour, or with `carried`, a daily table's last read at its slot (the walk's first
/// hour before its first slot), carried between reads.
fn source(table: TableId, hour: i32, first: i32, carried: bool) -> (i32, bool) {
    let slot = match table {
        TableId::ProductionLicenseRecords
        | TableId::IndustrialOrganisations
        | TableId::IndustrialOrganisationDetails
        | TableId::IndustrialOrganisationPolicies => 8,
        TableId::VariantDefinitions | TableId::VariantModules | TableId::VariantUpgrades => 22,
        _ => return (hour, true),
    };
    if !carried {
        return (hour, true);
    }
    let read = hour - (hour - slot).rem_euclid(24);
    if read >= first {
        (read, read == hour)
    } else {
        (first, hour == first)
    }
}

fn write<T: TableSchema>(walk: &mut Synthetic, hour: i32, first: i32, carried: bool) {
    let (at, read) = source(T::ID, hour, first, carried);
    let input = churn(at);
    let sample = Sample::FromReadBit;
    let input = match &*input.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

/// Every table the family reads at `hour` of a walk from `first`.
fn write_hour(walk: &mut Synthetic, hour: i32, first: i32, carried: bool) {
    write::<tables::Campaign>(walk, hour, first, carried);
    write::<tables::Countries>(walk, hour, first, carried);
    write::<tables::CountryLinks>(walk, hour, first, carried);
    write::<tables::CountryAppearance>(walk, hour, first, carried);
    write::<tables::LexicalTokens>(walk, hour, first, carried);
    write::<tables::ProductionLicenseRecords>(walk, hour, first, carried);
    write::<tables::IndustrialOrganisations>(walk, hour, first, carried);
    write::<tables::IndustrialOrganisationDetails>(walk, hour, first, carried);
    write::<tables::IndustrialOrganisationPolicies>(walk, hour, first, carried);
    write::<tables::VariantDefinitions>(walk, hour, first, carried);
    write::<tables::VariantModules>(walk, hour, first, carried);
    write::<tables::VariantUpgrades>(walk, hour, first, carried);
    write::<tables::VariantChanges>(walk, hour, first, carried);
}

/// The projected hours of `hours`, each run of consecutive hours a walk of its own, a projector
/// starting cold at each of `starts`; and the row frames the walk decoded of the same hours.
fn projected(hours: &[i32], starts: &[i32], carried: bool) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let format = FormatVersion::CURRENT;
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<(Synthetic, i32)> = None;
    let mut world = None;
    let mut projector = Projector::new(format, CaptureMode::Full).unwrap();
    for (ordinal, &hour) in (0..).zip(hours) {
        let gap = frames.last().is_some_and(|last| last.hour + 1 != hour);
        if walk.is_none() || gap {
            walk = Some((Synthetic::new(facts::TABLES, 1, hour).unwrap(), hour));
        }
        let (walk, first) = walk.as_mut().unwrap();
        write_hour(walk, hour, *first, carried);
        let cold = starts.contains(&hour);
        if cold {
            projector = Projector::new(format, CaptureMode::Full).unwrap();
        }
        let frame = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cold;
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &facts::needs(),
                    (format, CaptureMode::Full),
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
        frames.push(frame);
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), frames)
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>) -> Built {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new();
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// `reason` without the hours its reads named.
fn stable(reason: &str) -> String {
    let mut out = String::new();
    let mut rest = reason;
    while let Some(at) = rest.find("hour ") {
        let (before, after) = rest.split_at(at);
        let digits = after["hour ".len()..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        let tail = &after["hour ".len() + digits..];
        out.push_str(before);
        match tail.strip_prefix(' ') {
            Some(tail) if digits > 0 => rest = tail,
            _ => {
                out.push_str("hour ");
                rest = &after["hour ".len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A reply with its refusals' reasons named without their hours, their hours summed.
fn normalised(mut reply: Value) -> Value {
    let lines: Vec<String> = reply["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect();
    let mut kept = Vec::new();
    let mut hours = BTreeMap::<String, u64>::new();
    for line in lines {
        let refusal = line
            .strip_prefix("Production catalogue: ")
            .and_then(|rest| rest.strip_suffix(" observed hours)"))
            .and_then(|rest| rest.rsplit_once(" ("));
        match refusal {
            Some((reason, count)) => {
                *hours.entry(stable(reason)).or_default() += count.parse::<u64>().unwrap();
            }
            None => kept.push(Value::String(line)),
        }
    }
    kept.extend(hours.into_iter().map(|(reason, count)| {
        Value::String(format!("Production catalogue: {reason} ({count} observed hours)"))
    }));
    reply["warnings"] = Value::Array(kept);
    reply
}

/// Every view's reply through `through`, for every country and each of `tags` alone, in every
/// language, as `read` answers it.
fn replies(
    tags: &[&str],
    through: i32,
    mut read: impl FnMut(&ProductionCatalogueParams, &Naming<'_>) -> ProductionCatalogueReply,
) -> Vec<(String, Value)> {
    let mut replies = Vec::new();
    for (language, texts) in languages().iter().enumerate() {
        let naming = Naming {
            texts: texts.as_ref().map(|texts| texts as &dyn Texts),
            units: None,
            icon: &|raw: &RawOrganisation| {
                (!raw.token.is_empty()).then(|| {
                    let key = format!("GFX_{}", raw.token);
                    let image = if raw.icon.is_empty() {
                        Err("no stored sprite".to_owned())
                    } else {
                        Ok(raw.icon.clone())
                    };
                    (key, image)
                })
            },
        };
        for view in [
            CatalogueView::Licences,
            CatalogueView::Organisations,
            CatalogueView::Designs,
        ] {
            for chosen in std::iter::once(&[][..]).chain(tags.iter().map(std::slice::from_ref)) {
                let params = ProductionCatalogueParams {
                    branch_id: "1".into(),
                    view,
                    tags: chosen.iter().map(|tag| tag.to_string()).collect(),
                };
                let reply = serde_json::to_value(read(&params, &naming)).unwrap();
                replies.push((
                    format!("language {language}, {view:?} of {chosen:?} through {through}"),
                    normalised(reply),
                ));
            }
        }
    }
    replies
}

const TAGS: [&str; 6] = ["LIC", "GIV", "OTH", "NEW", "SEV", "FOU"];

/// The row census's replies to every request of `frames` through each of `throughs`, the
/// frames observed by their changes in day-long blocks, as the walk reads a row recording.
fn row_replies(frames: &[DecodedFrame], throughs: &[i32]) -> Vec<(String, Value)> {
    let format = FormatVersion::CURRENT;
    let mut state = Prepared::default();
    let mut block: Option<(i32, FrameEncoder, FrameDecoder, FrameChangeTracker)> = None;
    let mut last: Option<i32> = None;
    for frame in frames {
        let day = frame.hour.div_euclid(24);
        let fresh = block.as_ref().is_none_or(|(held, ..)| *held != day)
            || last.is_some_and(|last| last + 1 != frame.hour);
        if fresh {
            block = Some((
                day,
                FrameEncoder::new(),
                FrameDecoder::new(),
                FrameChangeTracker::new(),
            ));
        }
        let (_, encoder, decoder, tracker) = block.as_mut().unwrap();
        let wire = encoder.encode(frame.clone(), fresh).unwrap();
        let decoded = decoder.advance(&wire).unwrap().clone();
        let changes = tracker.observe_updates(&decoded, Arc::new(wire), format);
        state.observe_changes(format, &decoded, &changes).unwrap();
        last = Some(frame.hour);
    }
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    throughs
        .iter()
        .flat_map(|&through| {
            replies(&TAGS, through, |params, naming| {
                row::query(
                    &fixture.store,
                    &fixture.key,
                    params,
                    Some(through),
                    naming,
                    Vec::new(),
                    &flag,
                )
                .unwrap()
            })
        })
        .collect()
}

/// The publication's replies to every request through each of `throughs`.
fn published_replies(built: &Built, throughs: &[i32]) -> Vec<(String, Value)> {
    let flag = AtomicBool::new(false);
    throughs
        .iter()
        .flat_map(|&through| {
            let mut snapshot =
                Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            replies(&TAGS, through, |params, naming| {
                super::query(
                    &mut snapshot,
                    &directory,
                    params,
                    through,
                    naming,
                    Vec::new(),
                    &flag,
                )
                .unwrap()
            })
        })
        .collect()
}

/// Every hour of the scenario: 0 to 95 without 80 and 81.
fn hours() -> Vec<i32> {
    (0..96).filter(|hour| !(80..82).contains(hour)).collect()
}

#[test]
fn the_publication_answers_every_request_as_the_row_census_does() {
    let hours = hours();
    let throughs = [50, 95];
    // One segment; segment starts before, at and after a refused identity, a refused table, a
    // renamed spelling and the gap; and cuts with a resume at a segment start.
    let placements: [(&[i32], &[i32], Option<i32>); 3] = [
        (&[], &[95], None),
        (&[10, 33, 34, 36, 40, 44, 62, 63, 66, 82, 90], &[95], None),
        (&[24, 48, 72], &[23, 47, 71, 95], Some(47)),
    ];
    for carried in [false, true] {
        let mut expected = None;
        for (starts, cuts, resume) in placements {
            let label = format!(
                "carried {carried}, starts {starts:?}, cuts {cuts:?}, resume {resume:?}"
            );
            let (projected, frames) = projected(&hours, starts, carried);
            let expected = expected.get_or_insert_with(|| row_replies(&frames, &throughs));
            let built = build(projected, cuts, resume);
            let published = published_replies(&built, &throughs);
            assert_eq!(published.len(), expected.len(), "{label}");
            for ((name, published), (_, expected)) in published.iter().zip(expected.iter()) {
                assert_eq!(published, expected, "{label}: {name}");
            }
        }
    }
}

#[test]
fn an_hour_whose_inputs_did_not_change_hands_the_emit_nothing() {
    let hours = hours();
    let (projected, _) = projected(&hours, &[], false);
    for (at, hour) in projected.iter().enumerate().skip(1) {
        let before = projected[at - 1].hour;
        if before + 1 != hour.hour || !same_inputs(&churn(before), &churn(hour.hour)) {
            continue;
        }
        assert!(hour.facts.is_none(), "hour {} hands on facts", hour.hour);
        assert!(hour.built.is_empty(), "hour {} names variants anew", hour.hour);
    }
}
