//! The family's columnar preparation held to the row recording's scan
//! ([`dashboard_base::products::idea_history::Scan`]) on the same hours, written as the native
//! recorder writes them: every country's holdings, unobserved hours, identity starts with their
//! naming, advisor templates and warnings, and the event feed's additions, refusals and
//! definitions, across segment starts, a gap, cuts and a resume.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    query,
};
use dashboard_base::{
    products::idea_history::{Scan, ScannedIdeas, row_feed},
    test_fixtures::{CAMPAIGN, records, source},
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    container::Replay,
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        ActiveIdeaRow, CharacterRow, CountryAppearanceRow, CountryLinkRow, CountryRow,
        HiredAdvisorRow, LexicalTokenRow, ModifierDefinitionRow, RowCodec, TableId,
    },
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeSet, sync::atomic::AtomicBool};

const REFUSAL: &str = "synthetic refusal";

/// The countries, their links and every table holdings are read from at one hour; none where
/// a table is unavailable.
struct Tables {
    hour: i32,
    countries: Option<Vec<CountryRow>>,
    links: Vec<CountryLinkRow>,
    ideas: Option<Vec<ActiveIdeaRow>>,
    advisors: Vec<HiredAdvisorRow>,
    characters: Vec<CharacterRow>,
    tokens: Vec<LexicalTokenRow>,
    appearance: Vec<CountryAppearanceRow>,
    definitions: Vec<ModifierDefinitionRow>,
}

fn idea(country: u32, ordinal: u32, token: &[u8]) -> ActiveIdeaRow {
    ActiveIdeaRow {
        country,
        ordinal,
        token: token.to_vec(),
    }
}

fn advisor(country: u32, ordinal: u32, character: u32, idea: &[u8], slot: &[u8]) -> HiredAdvisorRow {
    HiredAdvisorRow {
        country,
        ordinal,
        character,
        idea: idea.to_vec(),
        slot: slot.to_vec(),
    }
}

/// Hours 100 to 150 of three countries, without 120 and 121.
///
/// AAA holds a spirit throughout, one law until 105 and another from 106 (a law change), and,
/// through a second raw id of its slot, an alias idea from 115 to 118. It appoints character 7
/// at 103 and character 8, who has no Characters row (an hour-dated template), at 104, and an
/// appointment with no idea token from 108; it holds a token that is not UTF-8 at 127 and takes
/// a cosmetic tag at 135. BBB's advisor 10 is renamed at 115 in the same seat (no new
/// appointment); BBB's country changes id at 130 (a new identity) and records a seat that is not
/// UTF-8 at 128. CCC, a dynamic country, is unresolved from 110 to 112 and takes up an idea
/// meanwhile (an addition after an unobserved stretch). A row no link resolves is held from 125
/// to 126; ActiveIdeas is unavailable at 132 and 133; the countries are unavailable at 140; the
/// modifier definitions change at 130.
fn scenario() -> Vec<Tables> {
    (100..=150)
        .filter(|hour| !(120..=121).contains(hour))
        .map(|hour| {
            let b = if hour >= 130 { 20 } else { 2 };
            let mut countries = vec![
                CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    tag: b"AAA".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: b,
                    canonical_slot: 2,
                    tag: b"BBB".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 3,
                    canonical_slot: 3,
                    tag: b"CCC".to_vec(),
                    ..Default::default()
                },
            ];
            countries.sort_unstable_by_key(|row| row.id);
            let mut links = vec![(1, 1), (b, 2), (11, 1)];
            if !(110..=112).contains(&hour) {
                links.push((3, 3));
            }
            let mut links: Vec<_> = links
                .into_iter()
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect();
            links.sort_unstable_by_key(|row| row.id);
            let mut ideas = vec![
                idea(1, 0, b"a_spirit"),
                idea(1, 1, if hour <= 105 { b"a_law_one" } else { b"a_law_two" }),
                idea(b, 0, b"b_idea"),
                idea(3, 0, b"c_idea"),
            ];
            if (115..=118).contains(&hour) {
                ideas.push(idea(11, 0, b"a_alias_idea"));
            }
            if hour >= 111 {
                ideas.push(idea(3, 1, b"c_new"));
            }
            if (125..=126).contains(&hour) {
                ideas.push(idea(99, 0, b"nobody"));
            }
            if hour == 127 {
                ideas.push(idea(1, 2, b"\xff"));
            }
            ideas.sort_unstable_by_key(|row| (row.country, row.ordinal));
            let mut advisors = vec![advisor(b, 0, 10, b"adv_ten", b"political_advisor")];
            if hour >= 103 {
                advisors.push(advisor(1, 0, 7, b"adv_seven", b"political_advisor"));
            }
            if hour >= 104 {
                advisors.push(advisor(1, 1, 8, b"adv_eight", b"theorist"));
            }
            if hour >= 108 {
                advisors.push(advisor(1, 2, 9, b"", b"army_chief"));
            }
            if hour == 128 {
                advisors.push(advisor(b, 1, 11, b"adv_eleven", b"\xfe"));
            }
            advisors.sort_unstable_by_key(|row| (row.country, row.ordinal));
            let characters = vec![
                CharacterRow {
                    id: 7,
                    name_token_id: 501,
                    token: b"seven".to_vec(),
                    ..Default::default()
                },
                CharacterRow {
                    id: 9,
                    token: b"nine".to_vec(),
                    ..Default::default()
                },
                CharacterRow {
                    id: 10,
                    token: if hour >= 115 { b"ten renamed".to_vec() } else { b"ten".to_vec() },
                    ..Default::default()
                },
                CharacterRow {
                    id: 11,
                    token: b"eleven".to_vec(),
                    ..Default::default()
                },
            ];
            let tokens = vec![
                LexicalTokenRow {
                    token: 501,
                    literal: b"AAA_character_seven".to_vec(),
                },
                LexicalTokenRow {
                    token: 900,
                    literal: b"Political_Power_Gain".to_vec(),
                },
            ];
            let appearance = vec![
                CountryAppearanceRow {
                    country: 1,
                    cosmetic_tag: if hour >= 135 { b"COS".to_vec() } else { Vec::new() },
                    ..Default::default()
                },
                CountryAppearanceRow {
                    country: 3,
                    name_tag: b"DYN".to_vec(),
                    ..Default::default()
                },
            ];
            let definitions = vec![ModifierDefinitionRow {
                index: 0,
                token: 900,
                name_key: b"MODIFIER_POLITICAL_POWER_GAIN".to_vec(),
                flags: if hour >= 130 { 2 } else { 0 },
                precision: 2,
                ..Default::default()
            }];
            Tables {
                hour,
                countries: (hour != 140).then_some(countries),
                links,
                ideas: (!(132..=133).contains(&hour)).then_some(ideas),
                advisors,
                characters,
                tokens,
                appearance,
                definitions,
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
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// The projected hours of `hours`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again), and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new().unwrap();
    let needs = facts::needs().unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, tables.countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&tables.links[..]));
        write(walk, TableId::ActiveIdeas, tables.ideas.as_deref());
        write(walk, TableId::HiredAdvisors, Some(&tables.advisors[..]));
        write(walk, TableId::Characters, Some(&tables.characters[..]));
        write(walk, TableId::LexicalTokens, Some(&tables.tokens[..]));
        write(walk, TableId::CountryAppearance, Some(&tables.appearance[..]));
        write(walk, TableId::ModifierDefinitions, Some(&tables.definitions[..]));
        let cold = starts.contains(&tables.hour);
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
        // The row recording's frame of the hour, with the campaign it is recorded under.
        let mut frame = decoded;
        frame.tables[TableId::Campaign.index()] =
            source(1, tables.hour, None).tables[TableId::Campaign.index()].clone();
        frames.push(frame);
    }
    (out, frames)
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

fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
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
            &cache,
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
            writer = Writer::resume(&mut pages, &cache, budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// What a reply reads of a country's history: all but the feed's own naming and the scan's
/// gap starts.
fn compared(mut scanned: ScannedIdeas) -> serde_json::Value {
    scanned.span_naming.clear();
    scanned.gap_starts.clear();
    serde_json::to_value(scanned).unwrap()
}

/// The fields of each recorded definition a modifier's display reads, by name.
type Displayed = Vec<(String, u32, Vec<u8>, Vec<u8>, Vec<u8>, u32, i32, u32)>;

fn displayed(
    definitions: Option<&std::collections::BTreeMap<String, ModifierDefinitionRow>>,
) -> Option<Displayed> {
    definitions.map(|definitions| {
        definitions
            .iter()
            .map(|(name, row)| {
                (
                    name.clone(),
                    row.index,
                    row.name_key.clone(),
                    row.postfix_key.clone(),
                    row.argument.clone(),
                    row.flags,
                    row.precision,
                    row.feature_selector,
                )
            })
            .collect()
    })
}

/// Holds the publication `built` to the row scan of `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == 1)
        .map(|start| dashboard_text::metadata::branch_bounds(&replay, start));
    let mut scan = Scan::new(&replay, 1);
    for frame in frames {
        scan.observe(frame).unwrap();
    }
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let tags: BTreeSet<String> = directory
        .actors
        .iter()
        .map(|actor| String::from_utf8(actor.name.to_vec()).unwrap())
        .chain(["AAA", "BBB", "CCC", "ZZZ"].map(str::to_owned))
        .collect();
    for tag in &tags {
        assert_eq!(
            compared(query::country(&mut snapshot, &directory, tag, bounds).unwrap()),
            compared(scan.country(&replay, 1, tag).unwrap()),
            "{label}: {tag}'s history"
        );
    }
    let flag = AtomicBool::new(false);
    let expected = row_feed(&replay, 1, &flag).unwrap();
    let actual = query::feed(&mut snapshot, &directory).unwrap();
    assert_eq!(actual.additions, expected.additions, "{label}: additions");
    assert_eq!(actual.observed, expected.observed, "{label}: observed");
    assert_eq!(actual.refusals, expected.refusals, "{label}: refusals");
    assert_eq!(
        actual.gap_additions, expected.gap_additions,
        "{label}: additions after an unobserved stretch"
    );
    assert_eq!(
        displayed(actual.definitions.as_ref()),
        displayed(expected.definitions.as_ref()),
        "{label}: definitions"
    );
    for (tag, templates) in &actual.templates {
        assert_eq!(
            Some(templates),
            expected.templates.get(tag),
            "{label}: {tag}'s templates"
        );
    }
}

/// Every placement of segment starts, cuts and a resume publishes what the row scan reads.
#[test]
fn the_publication_holds_what_the_row_scan_reads_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let every: Vec<i32> = (100..=150)
        .filter(|hour| !(120..=121).contains(hour))
        .collect();
    for (starts, cuts, resume) in [
        (&[][..], &[150][..], None),
        (&[106, 125, 141][..], &[105, 119, 139, 150][..], Some(119)),
        (&[111, 116, 130, 134][..], &[112, 129, 135, 150][..], Some(129)),
        (&every[1..], &every[..], Some(127)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume, &budget);
        check(&built, &frames, &label);
    }
}

/// The scenario reads as the game holds it: the law change and the appointments are the feed's
/// additions, the renamed advisor and the holdings after a new identity are none, the idea taken
/// up while unresolved is counted rather than invented, and the missing character's template is
/// dated by the last hour the inputs were complete.
#[test]
fn the_scenario_reads_as_the_game_holds_it() {
    let budget = Budget::new(256 << 20);
    let (projected, frames) = projected(&scenario(), &[]);
    let built = build(projected, &[150], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let feed = query::feed(&mut snapshot, &directory).unwrap();
    let additions: Vec<_> = feed
        .additions
        .iter()
        .map(|addition| (addition.tag.as_str(), addition.span.token.as_str(), addition.span.first_hour))
        .collect();
    assert!(additions.contains(&("AAA", "adv_seven", 103)));
    assert!(additions.contains(&("AAA", "adv_eight", 104)));
    assert!(additions.contains(&("AAA", "a_law_two", 106)));
    assert!(additions.contains(&("AAA", "a_alias_idea", 115)));
    assert!(!additions.iter().any(|&(tag, token, _)| tag == "BBB" && token == "adv_ten"));
    assert!(!additions.iter().any(|&(tag, token, _)| tag == "CCC" && token == "c_new"));
    assert_eq!(feed.gap_additions, 1);
    assert!(feed.refusals.iter().any(|(reason, count)| reason
        == dashboard_base::products::idea_history::refusal::TOKENLESS
        && *count == 1));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == 1)
        .map(|start| dashboard_text::metadata::branch_bounds(&replay, start));
    let aaa = query::country(&mut snapshot, &directory, "AAA", bounds).unwrap();
    assert_eq!(
        aaa.templates[&8].as_ref().unwrap_err(),
        "character 8 has no Characters row at hour 150"
    );
    let bbb = query::country(&mut snapshot, &directory, "BBB", bounds).unwrap();
    assert!(bbb.identity_starts.contains(&130));
    assert!(
        bbb.spans
            .iter()
            .any(|span| span.token == "adv_ten" && span.first_hour == 115)
    );
}
