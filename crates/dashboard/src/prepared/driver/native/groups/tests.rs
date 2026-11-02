//! The native groups held to the row groups on the same hours, written as the native recorder
//! writes them: the reply its published scan answers and every page of its changes, across
//! segment starts the row blocks do not share, a gap, publications and resumes.

use super::{Branch, Hour, Projector, holds, scan_section};
use crate::{
    prepared::frame_changes::FrameChangeTracker,
    products::{
        LoadContext,
        country_groups::{Scan, prepared::TABLES},
    },
};
use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, file_source, records};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    container::Replay,
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CapitulationRow, CountryLinkRow, CountryRow, FactionMemberRow, FactionRow,
        HumanControllerRow, LexicalTokenRow, RowCodec, TableId, WarRelationRow,
    },
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

const TOKEN: u32 = 4242;
const BOUNDS: (i32, i32) = (100, 170);

/// One hour's group tables; none where a table is unavailable.
struct Tables {
    hour: i32,
    countries: Option<Vec<CountryRow>>,
    links: Vec<CountryLinkRow>,
    humans: Vec<HumanControllerRow>,
    factions: Option<Vec<FactionRow>>,
    members: Vec<FactionMemberRow>,
    wars: Vec<WarRelationRow>,
    capitulations: Vec<CapitulationRow>,
    /// The dictionary is read at this hour.
    lexical: bool,
}

/// Hours 100 to 170 without 130 and 131, of five countries. BBB's id changes at 150 and the
/// countries are unavailable at 145. AAA leads faction 77 until CCC, who joined at 115, takes
/// over at 140; BBB leaves at 125; 77 is renamed at 133. Faction 88, named by its template,
/// holds DDD and EEE from 105 to 160, and a member no link names at 120 to 122; the factions are
/// unavailable at 155 and 156. AAA and DDD are at war from 108; CCC and EEE from 112 until the
/// war's end at 135. EEE capitulates at 140. A human no link names plays at 118. The dictionary
/// is read at 100, 120 and 141.
fn scenario() -> Vec<Tables> {
    (BOUNDS.0..=BOUNDS.1)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let b = if hour >= 150 { 20 } else { 2 };
            let tags = [(1, 1, "AAA"), (b, 2, "BBB"), (3, 3, "CCC"), (4, 4, "DDD"), (5, 5, "EEE")];
            let mut countries: Vec<CountryRow> = tags
                .iter()
                .map(|&(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect();
            countries.sort_unstable_by_key(|row| row.id);
            let mut links: Vec<CountryLinkRow> = tags
                .iter()
                .map(|&(id, canonical_slot, _)| CountryLinkRow { id, canonical_slot })
                .chain([CountryLinkRow {
                    id: 11,
                    canonical_slot: 1,
                }])
                .collect();
            links.sort_unstable_by_key(|row| row.id);
            let mut humans = vec![HumanControllerRow { country: 1, count: 1 }];
            if hour >= 110 {
                humans.push(HumanControllerRow { country: b, count: 1 });
            }
            if hour == 118 {
                humans.push(HumanControllerRow { country: 99, count: 1 });
            }
            humans.sort_unstable_by_key(|row| row.country);
            let mut factions = vec![FactionRow {
                id: 77,
                name: if hour >= 133 { b"Allied Powers".to_vec() } else { b"Allies".to_vec() },
                ..Default::default()
            }];
            let templated = (105..=160).contains(&hour);
            if templated {
                factions.push(FactionRow {
                    id: 88,
                    template_token: Some(TOKEN),
                    ..Default::default()
                });
            }
            let member = |country, faction, ordinal| FactionMemberRow {
                country,
                faction,
                ordinal,
            };
            let mut members = vec![member(1, 77, u32::from(hour >= 140))];
            if hour < 125 {
                members.push(member(b, 77, 1));
            }
            if (115..140).contains(&hour) {
                members.push(member(3, 77, 2));
            } else if hour >= 140 {
                members.push(member(3, 77, 0));
            }
            if templated {
                members.push(member(4, 88, 0));
                members.push(member(5, 88, 1));
                if (120..=122).contains(&hour) {
                    members.push(member(99, 88, 2));
                }
            }
            members.sort_unstable_by_key(|row| row.country);
            let war = |first_country, second_country, start_hour, end_hour, instigator| {
                WarRelationRow {
                    first_country,
                    second_country,
                    start_hour,
                    end_hour,
                    first_casualties: i64::from(hour),
                    first_was_instigator: instigator,
                    ..Default::default()
                }
            };
            let mut wars = Vec::new();
            if hour >= 108 {
                wars.push(war(1, 4, 108, 0, 1));
            }
            if hour >= 112 {
                wars.push(war(3, 5, 112, 135, 0));
            }
            let capitulations = tags
                .iter()
                .map(|&(country, _, _)| {
                    let capitulated = country == 5 && hour >= 140;
                    CapitulationRow {
                        country,
                        capitulated: u8::from(capitulated),
                        capitulated_hour: if capitulated { 140 } else { 0 },
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>();
            let mut capitulations = capitulations;
            capitulations.sort_unstable_by_key(|row| row.country);
            Tables {
                hour,
                countries: (hour != 145).then_some(countries),
                links,
                humans,
                factions: (!(155..=156).contains(&hour)).then_some(factions),
                members,
                wars,
                capitulations,
                lexical: [100, 120, 141].contains(&hour),
            }
        })
        .collect()
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason: "synthetic refusal",
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// The projected hours of `hours`, a segment starting at each of `starts` and at each walk's
/// first hour (after a gap, the walk starts again), and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let needs = Needs {
        frame: holds().unwrap().to_vec(),
        ..Needs::default()
    };
    let mut projector = Projector {
        asks: holds().unwrap(),
        moved: Vec::new(),
    };
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last| last.hour + 1 != tables.hour);
        let fresh = walk.is_none() || gap;
        if fresh {
            walk = Some(Synthetic::new(TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, tables.countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&tables.links[..]));
        write(walk, TableId::HumanControllers, Some(&tables.humans[..]));
        write(walk, TableId::Factions, tables.factions.as_deref());
        write(walk, TableId::FactionMembers, Some(&tables.members[..]));
        write(walk, TableId::WarRelations, Some(&tables.wars[..]));
        write(walk, TableId::Capitulation, Some(&tables.capitulations[..]));
        let lexical = [LexicalTokenRow {
            token: TOKEN,
            literal: b"alliance_template".to_vec(),
        }];
        if fresh || tables.lexical {
            write(walk, TableId::LexicalTokens, Some(&lexical[..]));
        } else {
            let same: Input<'_, LexicalTokenRow> = Input::Same {
                read: false,
                sample: Sample::FromReadBit,
            };
            walk.table(TableId::LexicalTokens, same).unwrap();
        }
        let cut = starts.contains(&tables.hour);
        let frame = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cut;
                let world = World::at(
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
        frames.push(frame);
    }
    (out, frames)
}

/// The row frames' source, its blocks split at the gap.
fn source(frames: &[DecodedFrame]) -> (crate::Source, PathBuf) {
    let (first, second): (Vec<_>, Vec<_>) =
        frames.iter().cloned().partition(|frame| frame.hour < 130);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 1, second)])
        .unwrap();
    file_source(replay)
}

/// A store's writer of its own, cleared when dropped.
struct Pages {
    root: PathBuf,
    store: store::Store,
    writer: store::Writer,
}

/// What a published branch answers: its scan's reply, and every other page, by section.
type Answer = (Value, BTreeMap<String, Vec<Option<Vec<u8>>>>);

impl Pages {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hoi4-groups-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let recording = root.join("recording");
        std::fs::write(&recording, b"synthetic").unwrap();
        let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
        let writer = store
            .replace(store::SourceKey::capture(&recording, &[]).unwrap())
            .unwrap();
        Self {
            root,
            store,
            writer,
        }
    }

    fn read(&mut self, section: &str, range: u64) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        self.writer
            .read_at(section, range)
            .unwrap()?
            .read_to_end(&mut bytes)
            .unwrap();
        Some(bytes)
    }

    fn answered(&mut self, source: &crate::Source) -> Answer {
        let page = self.read(&scan_section(1), 0).unwrap();
        let reply = Scan::read(&page[..])
            .unwrap()
            .reply(source, "1", &LoadContext { content: None })
            .unwrap();
        let names: Vec<String> = self
            .writer
            .section_names()
            .filter(|name| *name != scan_section(1))
            .map(str::to_owned)
            .collect();
        let mut pages = BTreeMap::new();
        for name in names {
            let count = self.writer.range_count(&name).unwrap().unwrap_or(0);
            let ranges = (0..count).map(|range| self.read(&name, range)).collect();
            pages.insert(name, ranges);
        }
        (serde_json::to_value(reply).unwrap(), pages)
    }
}

impl Drop for Pages {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_dir_all(self.store.directory());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The row groups as a row preparation prepares them: each block read alone with change facts
/// of its own, published after each of `cuts`.
fn rows(source: &crate::Source, cuts: &[i32]) -> Answer {
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let mut pages = Pages::new();
    let mut branch = Branch::new(Scan::within(Some(BOUNDS)), Box::default(), BOUNDS);
    for index in replay.index.iter().filter(|block| block.branch == 1) {
        branch.scan.begin_block();
        let mut tracker = FrameChangeTracker::new();
        replay
            .read_block_alone(
                index,
                TABLES,
                |raw, frame| {
                    let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                    let Branch { scan, changes, .. } = &mut *branch;
                    changes.observe_changed(scan, frame, &facts, source, &context)?;
                    if cuts.contains(&frame.hour) {
                        branch.publish(&mut pages.writer, 1, &cancelled)?;
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        branch.scan.finish_within(Some(BOUNDS));
    }
    pages.answered(source)
}

/// The native groups keeping `hours`, published after each of `cuts`, and taken up again from
/// their published page and saved state after each of `resumes`.
fn native(hours: Vec<Hour>, cuts: &[i32], resumes: &[i32], source: &crate::Source) -> Answer {
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let mut pages = Pages::new();
    let mut branch = Branch::new(Scan::within(Some(BOUNDS)), Box::default(), BOUNDS);
    for hour in hours {
        let at = hour.hour;
        branch.observe(source, &context, 1, hour).unwrap();
        if !cuts.contains(&at) {
            continue;
        }
        branch.publish(&mut pages.writer, 1, &cancelled).unwrap();
        if resumes.contains(&at) {
            let saved = branch.saved().unwrap();
            let page = pages.read(&scan_section(1), 0).unwrap();
            branch = Branch::resumed(&page[..], &saved, BOUNDS).unwrap();
        }
    }
    pages.answered(source)
}

#[test]
fn native_groups_publish_as_the_row_groups() {
    let hours = scenario();
    // Each resume goes on with a segment of its own, as a later run does.
    for (starts, cuts, resumes) in [
        (&[][..], &[170][..], &[][..]),
        (&[110, 140][..], &[125, 170][..], &[][..]),
        (&[115, 127, 150][..], &[126, 150, 170][..], &[126][..]),
        (
            &[127, 147, 156][..],
            &[126, 146, 155, 170][..],
            &[126, 146][..],
        ),
    ] {
        let label = format!(
            "segments at {starts:?}, published after {cuts:?}, resumed after {resumes:?}"
        );
        let (projected, frames) = projected(&hours, starts);
        let (source, _root) = source(&frames);
        let (reply, pages) = rows(&source, cuts);
        let (native_reply, native_pages) = native(projected, cuts, resumes, &source);
        assert_eq!(native_reply, reply, "{label}: reply");
        assert!(!pages.is_empty(), "{label}");
        assert_eq!(
            native_pages.keys().collect::<Vec<_>>(),
            pages.keys().collect::<Vec<_>>(),
            "{label}: sections"
        );
        for (section, ranges) in &pages {
            assert!(native_pages[section] == *ranges, "{label}: {section}");
        }
    }
}

/// An hour that changes nothing the groups read hands its emit no table; a table not read
/// whole is handed on every hour, as the row decoder reads it anew.
#[test]
fn quiet_hours_hand_on_no_table() {
    let hours = scenario();
    let (projected, _) = projected(&hours, &[]);
    let quiet: Vec<i32> = projected
        .iter()
        .filter(|hour| hour.changes.is_empty())
        .map(|hour| hour.hour)
        .collect();
    // Casualties change every hour and no group reads them.
    for hour in [101, 102, 103, 104, 165, 170] {
        assert!(quiet.contains(&hour), "{hour}: {quiet:?}");
    }
    for hour in [100, 105, 108, 115, 132, 133, 145, 150, 155, 156] {
        assert!(!quiet.contains(&hour), "{hour}");
    }
    let handed: Vec<TableId> = projected
        .iter()
        .find(|hour| hour.hour == 156)
        .unwrap()
        .changes
        .iter()
        .map(|(table, _)| *table)
        .collect();
    assert_eq!(handed, [TableId::Factions]);
}
