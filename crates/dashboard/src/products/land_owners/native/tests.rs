//! The native catalogue held to the row scan on the same hours, written as the native recorder
//! writes them: first witnesses across a gap, segment starts with and without the walk's world
//! starting again, tag changes, a slot two countries share, land lost and regained, two branches
//! read in either order, a resumed page, each way ownership goes unknown, and where each branch
//! stands: pending, done or refused.

use super::{Branch, Hour, Page, Projector, Readiness, catalogue, needs, readiness};
use crate::{QueryError, products::land_owners::Scan};
use dashboard_base::prepared::roots::{Cursor, End, Frontier, Held, Interval};
use dashboard_world::World;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, CountryRow, RowCodec, StateRow, TableId},
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;

const FORMAT: FormatVersion = FormatVersion::CURRENT;

fn frontier(complete: bool, last_hour: i32) -> Frontier {
    Frontier {
        version: 1,
        next: Cursor { run: 0, ordinal: 1 },
        segments: 1,
        first_hour: 0,
        last_hour,
        prepared: 1,
        proof: [0; 32],
        ledger_pages: 0,
        first_hole: None,
        state: None,
        carry: None,
        end: None,
        complete,
    }
}

#[test]
fn readiness_tracks_completion_losses_and_refusals() {
    assert!(matches!(readiness(4, None, None, 10), Readiness::Pending));
    assert!(matches!(
        readiness(4, None, Some(&frontier(false, 10)), 10),
        Readiness::Pending
    ));
    assert!(matches!(
        readiness(4, None, Some(&frontier(true, 9)), 10),
        Readiness::Pending
    ));
    assert!(matches!(
        readiness(4, None, Some(&frontier(true, 10)), 10),
        Readiness::Done { incomplete: false }
    ));
    let mut hole = frontier(true, 10);
    hole.first_hole = Some(Interval {
        first_hour: 5,
        last_hour: 5,
        held: Held::Lost,
        reason: String::new(),
    });
    assert!(matches!(
        readiness(4, None, Some(&hole), 10),
        Readiness::Done { incomplete: true }
    ));
    let mut lost = frontier(false, 9);
    lost.end = Some(End {
        hour: 9,
        held: Held::Lost,
        reason: String::new().into(),
        from: None,
    });
    assert!(matches!(
        readiness(4, None, Some(&lost), 10),
        Readiness::Done { incomplete: true }
    ));
    let pending = frontier(false, 10);
    let complete = frontier(true, 10);
    for frontier in [None, Some(&pending), Some(&complete)] {
        match readiness(4, Some((8, "bad data".into())), frontier, 10) {
            Readiness::Refused(message) => assert_eq!(
                message,
                QueryError::Failed {
                    message: "Land owners were refused on branch 4 from hour 8: bad data".into(),
                }
            ),
            _ => panic!("refusal must win"),
        }
    }
    // A publication ending refused refuses with its reason, its page read or not.
    let mut refused = frontier(false, 9);
    refused.end = Some(End {
        hour: 7,
        held: Held::Refused,
        reason: QueryError::Refused {
            text: "bad data".into(),
        },
        from: None,
    });
    match readiness(4, None, Some(&refused), 10) {
        Readiness::Refused(message) => assert_eq!(
            message,
            QueryError::Refused {
                text: "bad data".into(),
            }
        ),
        _ => panic!("a refused end refuses"),
    }
}

/// One hour's ownership tables; Countries is unavailable where none.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Option<Vec<CountryRow>>,
    links: Vec<CountryLinkRow>,
    states: Vec<StateRow>,
}

fn country(id: u32, canonical_slot: u32, tag: &str) -> CountryRow {
    CountryRow {
        id,
        canonical_slot,
        tag: tag.as_bytes().to_vec(),
        ..CountryRow::default()
    }
}

fn link(id: u32, canonical_slot: u32) -> CountryLinkRow {
    CountryLinkRow { id, canonical_slot }
}

fn state(id: u32, owner: u32) -> StateRow {
    StateRow {
        id,
        owner,
        controller: owner,
        ..StateRow::default()
    }
}

/// Branch 1's hours 100 to 125 without 113 and 114: AAA (slot 1, alias link 11), BBB (slot 2),
/// CCC (slot 3, CCX from 108) own a state each and AAA's alias another; DDD (slot 4) owns BBB's
/// state at 104 and 105; BBX shares slot 2 from 110 and claims it once BBB's row goes at 120;
/// EEE (slot 5) appears at 117 and takes CCX's state at 118.
fn ownership(hour: i32) -> Tables {
    let mut countries = vec![
        country(1, 1, "AAA"),
        country(3, 3, if hour >= 108 { "CCX" } else { "CCC" }),
        country(4, 4, "DDD"),
    ];
    let mut links = vec![link(1, 1), link(2, 2), link(3, 3), link(4, 4), link(11, 1)];
    if hour < 120 {
        countries.push(country(2, 2, "BBB"));
    }
    if hour >= 110 {
        countries.push(country(7, 2, "BBX"));
        links.push(link(7, 2));
    }
    if hour >= 117 {
        countries.push(country(5, 5, "EEE"));
        links.push(link(5, 5));
    }
    countries.sort_unstable_by_key(|row| row.id);
    links.sort_unstable_by_key(|row| row.id);
    let states = vec![
        state(1, 1),
        state(2, if (104..=105).contains(&hour) { 4 } else { 2 }),
        state(3, if hour >= 118 { 5 } else { 3 }),
        state(4, 11),
    ];
    Tables {
        hour,
        countries: Some(countries),
        links,
        states,
    }
}

fn hours() -> Vec<i32> {
    (100..=125)
        .filter(|hour| !(113..=114).contains(hour))
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

/// `branch`'s projected hours of `hours` and their row frames. A segment starts at each of
/// `starts`, the walk's world with it, and at each walk's first hour (after a gap, the walk starts
/// again); at each of `cold` the projector alone starts again, its world going on.
fn projected(
    branch: u64,
    hours: &[Tables],
    starts: &[i32],
    cold: &[i32],
) -> (Vec<Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let needs = needs(false).unwrap();
    let fresh = || Projector::new(FORMAT, None).unwrap();
    let mut projector = fresh();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(super::tables(false), branch, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, tables.countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&tables.links[..]));
        write(walk, TableId::States, Some(&tables.states[..]));
        let cut = starts.contains(&tables.hour);
        if cut || cold.contains(&tables.hour) {
            projector = fresh();
        }
        let frame = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cut;
                let world = World::at(
                    &mut world,
                    starts,
                    &needs,
                    (FORMAT, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch,
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

/// The page a branch keeps of `hours`.
fn page(hours: Vec<Hour>) -> Page {
    let mut branch = Branch::new();
    for hour in hours {
        branch.observe(hour);
    }
    branch.page().clone()
}

/// The row scan's reply to `frames`, read in their order.
fn scanned(frames: &[DecodedFrame]) -> crate::LandOwners {
    let mut scan = Scan::default();
    for frame in frames {
        scan.observe(frame, None, None).unwrap();
    }
    scan.reply()
}

/// Each run of consecutive hours of `frames`, as the recording's blocks.
fn blocks(frames: &[DecodedFrame]) -> Vec<(u64, i32, i32)> {
    let mut blocks: Vec<(u64, i32, i32)> = Vec::new();
    for frame in frames {
        match blocks.last_mut() {
            Some((branch, _, last)) if *branch == frame.branch && *last + 1 == frame.hour => {
                *last = frame.hour;
            }
            _ => blocks.push((frame.branch, frame.hour, frame.hour)),
        }
    }
    blocks
}

#[test]
fn the_native_catalogue_answers_as_the_row_scan_on_one_branch() {
    let hours: Vec<Tables> = hours().into_iter().map(ownership).collect();
    let (projected, frames) = projected(1, &hours, &[107], &[111, 121]);
    // A quiet hour hands nothing on.
    assert!(
        projected
            .iter()
            .any(|hour| hour.hour == 103 && hour.found.is_empty())
    );
    let found: Vec<(i32, u32, &[u8])> = projected
        .iter()
        .flat_map(|hour| {
            hour.found
                .iter()
                .map(move |found| (hour.hour, found.slot, &*found.tag))
        })
        .filter(|(hour, _, _)| ![100, 107, 111, 115, 121].contains(hour))
        .collect();
    assert_eq!(
        found,
        [
            (104, 4, &b"DDD"[..]),
            (106, 2, b"BBB"),
            (108, 3, b"CCX"),
            (118, 5, b"EEE"),
            (120, 2, b"BBX"),
        ]
    );
    let page = page(projected);
    assert!(!page.incomplete);
    let native = catalogue(blocks(&frames), &[(1, page.clone())], false, None);
    assert_eq!(native, scanned(&frames));
    let tags: Vec<&str> = native
        .countries
        .iter()
        .map(|country| &*country.tag)
        .collect();
    assert_eq!(tags, ["AAA", "BBB", "BBX", "CCC", "CCX", "DDD", "EEE"]);
    // A page read back is the page written.
    assert_eq!(Page::read(&page.bytes().unwrap()).unwrap(), page);
}

#[test]
fn the_native_catalogue_keeps_each_lowest_branch_witness_in_block_order() {
    let first: Vec<Tables> = hours().into_iter().map(ownership).collect();
    // Branch 2 renames CCC at 101, before branch 1 does.
    let second: Vec<Tables> = (100..=103)
        .map(|hour| {
            let mut tables = ownership(hour);
            if hour >= 101 {
                for row in tables.countries.iter_mut().flatten() {
                    if row.id == 3 {
                        row.tag = b"CCX".to_vec();
                    }
                }
            }
            tables
        })
        .collect();
    let (one, one_frames) = projected(1, &first, &[], &[]);
    let (two, two_frames) = projected(2, &second, &[], &[]);
    let pages = [(1, page(one)), (2, page(two))];
    for frames in [
        [&one_frames[..], &two_frames[..]].concat(),
        [&two_frames[..], &one_frames[..]].concat(),
    ] {
        let native = catalogue(blocks(&frames), &pages, false, None);
        assert_eq!(native, scanned(&frames));
        let renamed = native
            .countries
            .iter()
            .find(|country| country.tag == "CCX")
            .unwrap();
        assert_eq!(
            (&*renamed.observed_branch_id, renamed.observed_hour),
            ("1", 108)
        );
    }
}

#[test]
fn a_resumed_branch_goes_on_from_its_page() {
    let hours: Vec<Tables> = hours().into_iter().map(ownership).collect();
    let (projected, frames) = projected(1, &hours, &[], &[]);
    let at = projected.iter().position(|hour| hour.hour == 110).unwrap();
    let mut rest = projected;
    let tail = rest.split_off(at);
    let mut kept = Branch::new();
    for hour in rest {
        kept.observe(hour);
    }
    let mut pages = PageMemory::default();
    kept.publish(&mut pages, 1).unwrap();
    let mut resumed = Branch::resume(&mut pages, 1).unwrap();
    for hour in tail {
        resumed.observe(hour);
    }
    let native = catalogue(blocks(&frames), &[(1, resumed.page().clone())], false, None);
    assert_eq!(native, scanned(&frames));
}

/// Two tags of one slot whose shortened display tags are equal keep the row scan's order: the
/// slot's first-seen tag first, though its raw bytes sort after.
#[test]
fn tags_shortened_alike_keep_their_first_seen_order() {
    let long = "x".repeat(600);
    let hours: Vec<Tables> = (100..=102)
        .map(|hour| {
            let tag = if hour == 100 { format!("{long}z") } else { format!("{long}a") };
            Tables {
                hour,
                countries: Some(vec![country(1, 7, &tag)]),
                links: vec![link(1, 7)],
                states: vec![state(1, 1)],
            }
        })
        .collect();
    let (projected, frames) = projected(1, &hours, &[], &[]);
    let native = catalogue(blocks(&frames), &[(1, page(projected))], false, None);
    let observed: Vec<i32> = native
        .countries
        .iter()
        .map(|country| country.observed_hour)
        .collect();
    assert_eq!(observed, [100, 101]);
    assert_eq!(native, scanned(&frames));
}

/// Ownership goes unknown where the row scan finds it unknown: an owner no link names, a country
/// the links do not resolve, an owning slot no country claims and Countries unavailable; owner 0
/// linked to nothing is no country's. What the pages cannot know, the reply adds: the recording's
/// gaps and lost hours, and why the content names countries by their tags.
#[test]
fn the_native_catalogue_finds_ownership_unknown_where_the_row_scan_does() {
    type Fault = fn(&mut Tables);
    let faults: [(&str, Fault, bool); 5] = [
        (
            "unlinked owner",
            |tables| tables.states.push(state(9, 99)),
            true,
        ),
        (
            "unowned state",
            |tables| tables.states.push(state(9, 0)),
            false,
        ),
        (
            "unresolved country",
            |tables| {
                let countries = tables.countries.as_mut().unwrap();
                countries.push(country(8, 8, "HHH"));
                countries.sort_unstable_by_key(|row| row.id);
            },
            true,
        ),
        (
            "unclaimed slot",
            |tables| {
                tables.links.push(link(12, 12));
                tables.states.push(state(9, 12));
            },
            true,
        ),
        (
            "countries unavailable",
            |tables| tables.countries = None,
            true,
        ),
    ];
    for (name, fault, unknown) in faults {
        let hours: Vec<Tables> = hours()
            .into_iter()
            .map(|hour| {
                let mut tables = ownership(hour);
                if (109..=110).contains(&hour) {
                    fault(&mut tables);
                }
                tables
            })
            .collect();
        let (projected, frames) = projected(1, &hours, &[107], &[]);
        let page = page(projected);
        assert_eq!(page.incomplete, unknown, "{name}");
        let native = catalogue(blocks(&frames), &[(1, page)], false, None);
        assert_eq!(native, scanned(&frames), "{name}");
    }
    let hours: Vec<Tables> = hours().into_iter().map(ownership).collect();
    let (projected, frames) = projected(1, &hours, &[], &[]);
    let reason = "Country names show tags until the game folders are chosen.";
    let native = catalogue(
        blocks(&frames),
        &[(1, page(projected))],
        true,
        Some(reason.to_owned()),
    );
    let mut scan = Scan {
        incomplete: true,
        warnings: vec![reason.to_owned()],
        ..Scan::default()
    };
    for frame in &frames {
        scan.observe(frame, None, None).unwrap();
    }
    assert_eq!(native, scan.reply());
}

/// Pages kept in memory, as a writer keeps them.
#[derive(Default)]
struct PageMemory(std::collections::HashMap<(String, u64), Vec<u8>>);

impl dashboard_base::prepared::pages::Pages for PageMemory {
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> std::io::Result<()> {
        self.0.insert((section.to_owned(), range), data.to_vec());
        Ok(())
    }

    fn write(&mut self, section: &str, data: &[u8]) -> std::io::Result<u64> {
        let range = self.0.keys().filter(|(held, _)| held == section).count() as u64;
        self.write_at(section, range, data)?;
        Ok(range)
    }

    fn read_at(&mut self, section: &str, range: u64) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.0.get(&(section.to_owned(), range)).cloned())
    }
}
