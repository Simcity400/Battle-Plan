//! The family's columnar preparation held to the walked scan ([`super::Scan`]) on the same
//! hours, written as the native recorder writes them: every part's runs, names and warnings
//! across segment starts, a gap, cuts and a resume, a read whose lines arrive after its hour
//! and a character renamed between them; and the replies, as the walked parts answer them.

use super::{
    Measure, Part, Scan,
    emit::{Emit, SCHEMA, part_actor},
    facts::{self, Projector},
};
use dashboard_base::{
    prepared::engine,
    source::{Source, SourceKey},
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
        CharacterRow, CountryLinkRow, CountryRow, CountryStabilityInputRow,
        CountryStabilityLineRow as Line, LexicalTokenRow, RowCodec, SliceReadRow, TableId,
    },
    scope::CaptureMode,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
};

const GER: u32 = 9001;
const ITA: u32 = 9002;
const FRA: u32 = 9003;
const REFUSAL: &str = "synthetic refusal";

/// The latest of `reads` at `hour`.
fn read_at(reads: &[i32], hour: i32) -> i32 {
    reads.iter().copied().filter(|&read| read <= hour).max().unwrap()
}

/// Hours 100 to 125 without 113 and 114. Germany reads at 100, 103, 107, 110, 116, 120 and
/// 124, its leader (character 7) and an idea; its lines are unavailable at 107 and arrive at
/// 108 still dated 107, the hour character 7 is renamed. Italy reads at 100, 105, 109, 118 and
/// 122 an unpaired line, its 109 read recorded no lines with it, and its 118 read's label is
/// refused. France reads at 100, 102, 104, 111, 117 and 123 the same idea and leader (character
/// 8) throughout, its inputs the same but from 104.
fn hours() -> Vec<i32> {
    (100..=125).filter(|hour| !(113..=114).contains(hour)).collect()
}

const GER_READS: [i32; 7] = [100, 103, 107, 110, 116, 120, 124];
const ITA_READS: [i32; 5] = [100, 105, 109, 118, 122];
const FRA_READS: [i32; 6] = [100, 102, 104, 111, 117, 123];

fn inputs(country: u32, read: i32) -> CountryStabilityInputRow {
    CountryStabilityInputRow {
        country,
        stability_raw: 50_000 + i64::from(read - 100) * 100,
        ..dashboard_base::test_fixtures::stability_inputs()
    }
}

/// France's inputs: the same at every read but from 104.
fn inputs_of_france(read: i32) -> CountryStabilityInputRow {
    CountryStabilityInputRow {
        country: FRA,
        stability_raw: if read >= 104 { 40_500 } else { 40_000 },
        ..dashboard_base::test_fixtures::stability_inputs()
    }
}

fn lines(hour: i32) -> Option<Vec<Line>> {
    if hour == 107 {
        return None;
    }
    let ger = read_at(&GER_READS, hour);
    let ita = read_at(&ITA_READS, hour);
    let mut lines = vec![
        Line {
            country: GER,
            line: 0,
            key: 0x8d,
            kind: Line::COUNTRY_LEADER,
            identity_id: 7,
            value: 30_000 + i64::from(ger - 100),
            ..Default::default()
        },
        Line {
            country: GER,
            line: 1,
            key: 0x8d,
            kind: Line::IDEA,
            identity: b"idea_a".to_vec(),
            value: 10_000,
            ..Default::default()
        },
    ];
    // Italy's 109 read recorded no lines: its lines stay those of 105.
    let ita_lines = if ita == 109 { 105 } else { ita };
    lines.push(Line {
        country: ITA,
        line: 0,
        key: 0x8d,
        kind: Line::UNPAIRED,
        value: 5_000 + i64::from(ita_lines - 100),
        text: if ita_lines >= 118 {
            b"caf\xc3".to_vec()
        } else {
            b"Bonus".to_vec()
        },
        ..Default::default()
    });
    lines.extend([
        Line {
            country: FRA,
            line: 0,
            key: 0x8d,
            kind: Line::COUNTRY_LEADER,
            identity_id: 8,
            value: 20_000,
            ..Default::default()
        },
        Line {
            country: FRA,
            line: 1,
            key: 0x8d,
            kind: Line::IDEA,
            identity: b"idea_f".to_vec(),
            value: 7_000,
            ..Default::default()
        },
    ]);
    lines.sort_unstable_by_key(|line| (line.country, line.line, line.key));
    Some(lines)
}

fn slice_reads(hour: i32) -> Vec<SliceReadRow> {
    let ger = read_at(&GER_READS, hour);
    let ita = read_at(&ITA_READS, hour);
    let ita_lines = if ita == 109 { 105 } else { ita };
    let read = |table: TableId, owner, sample_hour| SliceReadRow {
        table: table as u32,
        owner,
        sample_hour,
    };
    let mut rows = vec![
        read(TableId::CountryStabilityInputs, GER, ger),
        read(TableId::CountryStabilityInputs, ITA, ita),
        read(TableId::CountryStabilityLines, GER, ger),
        read(TableId::CountryStabilityLines, ITA, ita_lines),
        read(TableId::CountryStabilityInputs, FRA, read_at(&FRA_READS, hour)),
        read(TableId::CountryStabilityLines, FRA, read_at(&FRA_READS, hour)),
    ];
    rows.sort_unstable_by_key(|row| (row.table, row.owner));
    rows
}

fn characters(hour: i32) -> Vec<CharacterRow> {
    vec![
        CharacterRow {
            id: 7,
            country: GER,
            token: if hour < 108 { b"Old Name".to_vec() } else { b"New Name".to_vec() },
            name_token_id: -1,
            ..Default::default()
        },
        CharacterRow {
            id: 8,
            country: FRA,
            token: b"Fixed Name".to_vec(),
            name_token_id: -1,
            ..Default::default()
        },
    ]
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

/// What the walked scan keeps of a tag: each measure's parts with their runs.
#[derive(Default)]
struct Walked {
    measures: BTreeMap<String, [Measure; 2]>,
    warnings: BTreeMap<String, Vec<String>>,
}

/// The projected hours, a projector starting cold at each of `starts` and at each walk's
/// first hour (after a gap the walk starts again), and the walked scan of the same hours.
fn projected(starts: &[i32]) -> (Vec<facts::Hour>, Walked) {
    let format = FormatVersion::CURRENT;
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(format.version()).unwrap();
    let mut scan = Scan::default();
    let mut walked = Walked::default();
    let mut runs = 0;
    let mut last: Option<i32> = None;
    for (ordinal, hour) in (0..).zip(hours()) {
        let gap = last.is_some_and(|last| last + 1 != hour);
        last = Some(hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let countries = vec![
            CountryRow {
                id: GER,
                canonical_slot: 7,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: ITA,
                canonical_slot: 8,
                tag: b"ITA".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: FRA,
                canonical_slot: 9,
                tag: b"FRA".to_vec(),
                ..Default::default()
            },
        ];
        write(walk, TableId::Countries, Some(&countries[..]));
        let links = [
            CountryLinkRow {
                id: GER,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: ITA,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: FRA,
                canonical_slot: 9,
            },
        ];
        write(walk, TableId::CountryLinks, Some(&links[..]));
        let rows = [
            inputs(GER, read_at(&GER_READS, hour)),
            inputs(ITA, read_at(&ITA_READS, hour)),
            inputs_of_france(read_at(&FRA_READS, hour)),
        ];
        write(walk, TableId::CountryStabilityInputs, Some(&rows[..]));
        write(walk, TableId::CountryStabilityLines, lines(hour).as_deref());
        write(walk, TableId::SliceReads, Some(&slice_reads(hour)[..]));
        write(walk, TableId::Characters, Some(&characters(hour)[..]));
        write::<LexicalTokenRow>(walk, TableId::LexicalTokens, Some(&[]));
        write::<replay_data::rows::CountryAppearanceRow>(walk, TableId::CountryAppearance, None);
        write::<replay_data::rows::ReservedCountryNamingRow>(
            walk,
            TableId::ReservedCountryNaming,
            None,
        );
        write::<replay_data::rows::CountryCivilWarRow>(walk, TableId::CountryCivilWars, None);
        write::<replay_data::rows::CountryCounterInputRow>(
            walk,
            TableId::CountryCounterInputs,
            None,
        );
        write::<replay_data::rows::CountryWarRow>(walk, TableId::CountryWars, None);
        write::<replay_data::rows::WarRelationRow>(walk, TableId::WarRelations, None);
        let cold = starts.contains(&hour);
        if cold {
            projector = Projector::new(format.version()).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
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
                        hour,
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
            scan.step(decoded, None, None, |tag, _, _, joined, values, naming| {
                let measures = walked.measures.entry(tag.to_owned()).or_default();
                for (measure, values) in measures.iter_mut().zip(values) {
                    if let Some(values) = values {
                        measure.record(hour, joined, values, naming, &mut runs);
                    }
                }
                Ok(())
            })
            .unwrap();
        })
        .unwrap();
    }
    for tag in walked.measures.keys() {
        walked.warnings.insert(tag.clone(), scan.warnings(tag));
    }
    (out, walked)
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

struct Built {
    pages: Memory,
    root: u64,
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
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
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// A run as a key holds it: first and last hour, break, value.
type Raw = (i32, i32, bool, i128);

/// The runs a key holds of a walked part's runs: equal values going on without a break are
/// one.
fn expected(part: &Part) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    for run in &part.runs {
        let value: i128 = run.value_raw.parse().unwrap();
        let follows = out
            .last()
            .is_some_and(|last| last.1.checked_add(1) == Some(run.first_hour));
        let starts = run.starts_segment || !follows;
        match out.last_mut() {
            Some(last) if !starts && last.3 == value => last.1 = run.last_hour,
            _ => out.push((run.first_hour, run.last_hour, starts, value)),
        }
    }
    out
}

fn raw<P: dashboard_series::PageRead>(snapshot: &mut Snapshot<'_, P>, key: u64) -> Vec<Raw> {
    let accepted = snapshot.publication().accepted;
    engine::runs(snapshot, key, accepted.first, accepted.last)
        .unwrap()
        .into_iter()
        .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
        .collect()
}

/// A source the replies are asked of; they read nothing of its recording without content.
fn reply_source() -> Source {
    Source::from_replay(
        SourceKey {
            source_id: "parts columnar".into(),
            revision: "1".into(),
        },
        std::env::temp_dir().join("parts-columnar.battleplan"),
        Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![source(1, 100, Some(1))])])
                .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Holds the publication `built` to the walked scan.
fn check(built: &Built, walked: &Walked, label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let source = reply_source();
    let flag = AtomicBool::new(false);
    for (tag, measures) in &walked.measures {
        let catalogue = super::query::catalogue(&mut snapshot, &directory, tag)
            .unwrap()
            .unwrap_or_else(|| panic!("{label}: {tag} has no names"));
        assert_eq!(catalogue.warnings, walked.warnings[tag], "{label}: {tag}'s warnings");
        for (index, measure) in measures.iter().enumerate() {
            let published: BTreeSet<&String> = catalogue.parts[index].keys().collect();
            let held: BTreeSet<&String> = measure.parts.keys().collect();
            assert_eq!(published, held, "{label}: {tag}'s parts of measure {index}");
            for (id, part) in &measure.parts {
                let (named, _) = &catalogue.parts[index][id];
                assert_eq!(
                    *named,
                    Part {
                        runs: Vec::new(),
                        ..part.clone()
                    },
                    "{label}: {tag}'s part {id}"
                );
                let key = directory
                    .actor(part_actor(tag, id).unwrap().as_bytes())
                    .and_then(|actor| directory.key(actor, index as u64, None))
                    .unwrap_or_else(|| panic!("{label}: {tag}'s part {id} has no key"));
                assert_eq!(
                    raw(&mut snapshot, key),
                    expected(part),
                    "{label}: {tag}'s part {id} of measure {index}"
                );
            }
            let name = super::emit::MEASURES[index];
            for hour in hours() {
                let reply = super::query::query(
                    &source,
                    None,
                    "series-state",
                    json!({
                        "branchId": "1",
                        "family": {"kind": "stability-parts", "measure": name},
                        "reads": [{"tag": tag, "hour": hour}],
                    }),
                    &mut snapshot,
                    &directory,
                    &flag,
                )
                .unwrap();
                let ids: Vec<&str> = reply["parts"][tag.as_str()]["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|part| part["id"].as_str().unwrap())
                    .collect();
                assert_eq!(
                    ids,
                    measure.parts.keys().map(String::as_str).collect::<Vec<_>>(),
                    "{label}: {tag}'s reply names"
                );
                for (id, part) in &measure.parts {
                    let state = reply["states"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|state| state["tag"] == part_actor(tag, id).unwrap())
                        .unwrap();
                    let run = part
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour);
                    assert_eq!(
                        state["run"]["valueRaw"].as_str(),
                        run.map(|run| run.value_raw.as_str()),
                        "{label}: {tag}'s {id} at {hour}"
                    );
                }
                assert_eq!(reply["warnings"], json!(walked.warnings[tag]), "{label}");
            }
        }
    }
}

/// Every placement of segment starts, cuts and a resume publishes what the walked scan
/// keeps: a read met only after its hour, at a segment's first hour, names its character as
/// its own hour recorded it.
#[test]
fn the_publication_holds_what_the_walked_scan_keeps_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    for (starts, cuts, resume) in [
        (&[][..], &[125][..], None),
        (&[105, 108, 117][..], &[104, 107, 116, 125][..], Some(107)),
        (
            &[101, 102, 106, 108, 109, 111, 115, 119, 121][..],
            &[101, 105, 108, 112, 118, 125][..],
            Some(112),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}");
        let (hours, walked) = projected(starts);
        if starts.is_empty() {
            let handed = |same: bool| {
                hours.iter().flat_map(|hour| &hour.countries).any(|country| {
                    matches!(
                        (&country.parts, same),
                        (Some(super::facts::Parts::Same), true)
                            | (Some(super::facts::Parts::Named(_)), false)
                    )
                })
            };
            assert!(
                handed(true) && handed(false),
                "{label}: France's unchanged reads are handed on as the same parts"
            );
            assert!(
                walked.measures["FRA"][0]
                    .parts
                    .keys()
                    .any(|id| id.ends_with(":Fixed Name")),
                "{label}: France's leader is named"
            );
        }
        let names = &walked.measures["GER"][0].parts;
        assert!(
            names.keys().any(|id| id.ends_with(":Old Name"))
                && names.keys().any(|id| id.ends_with(":New Name")),
            "{label}: the rename between read 107 and its lines is named as read 107 held it"
        );
        assert_eq!(
            names
                .values()
                .filter(|part| part.character.as_ref().is_some_and(|c| c.1 == "Old Name"))
                .flat_map(|part| &part.runs)
                .map(|run| run.last_hour)
                .max(),
            Some(109),
            "{label}: read 107's leader is its own hour's"
        );
        assert!(
            walked.warnings["ITA"].iter().any(|warning| warning.contains("1 reads")),
            "{label}"
        );
        let built = build(hours, cuts, resume, &budget);
        check(&built, &walked, &label);
    }
}

/// A part's addends merge as the game's total does, leave zero lines out, and name a character
/// line by the character its read recorded.
#[test]
fn assembled_parts_merge_one_source_and_name_characters_as_recorded() {
    use super::{Addend, BreakdownLine, Name, assemble};
    let line = |kind: &str, id: u32, value: i64| BreakdownLine {
        line: 0,
        kind: kind.into(),
        identity: String::new(),
        identity_id: id,
        secondary_raw: "0".into(),
        value_raw: value.to_string(),
        text: None,
        label: None,
        group: None,
        group_kind: None,
    };
    let addend = |name: Name, line: Option<BreakdownLine>, raw: i64| Addend {
        name,
        term: None,
        line,
        raw,
    };
    let addends = [
        addend(
            Name::Character("line:countryLeader::7:0".into()),
            Some(line("countryLeader", 7, 3)),
            3,
        ),
        addend(
            Name::Character("line:countryLeader::7:0".into()),
            Some(line("countryLeader", 7, 4)),
            4,
        ),
        addend(Name::Id("remainder".into()), None, 0),
        addend(
            Name::Character("line:advisor::8:0".into()),
            Some(line("advisor", 8, 5)),
            5,
        ),
    ];
    let values = assemble(&addends, &|line| {
        (line.identity_id == 7).then(|| (Some("script".into()), "Saved".into(), Err("none".into())))
    });
    let ids: Vec<&str> = values.iter().map(|value| value.id.as_str()).collect();
    assert_eq!(
        ids,
        ["line:countryLeader::7:0:script:Saved", "line:advisor::8:0"]
    );
    assert_eq!(values[0].raw, 7);
    assert_eq!(values[0].line.as_ref().unwrap().value_raw, "3");
    assert!(values[1].character.is_none());
}

/// A read that recorded no lines is counted once, though a truncated read's hour, at a cut,
/// dates Germany past it before the hour after dates it there again.
#[test]
fn a_truncated_read_retires_no_read_its_recovery_dates_again() {
    use super::facts::{Characters, Country, Hour, Named, Parts, Reads};
    let offer = |read: i32, lined: bool| Country {
        index: 0,
        holder: (7, GER),
        read,
        lined,
        naming: lined.then(|| Arc::new(dashboard_text::identity::NamingCountry::default())),
        parts: lined.then(|| {
            Parts::Named(Box::new(Named {
                measures: [Some(Vec::new()), Some(Vec::new())],
                names: Vec::new(),
                refused: Default::default(),
            }))
        }),
    };
    // Italy dates its inputs at 11 throughout.
    let hour = |hour: i32, germany: Option<i32>, country: Country| Hour {
        branch: 1,
        hour,
        named: if hour == 9 {
            vec![(0, Arc::from("GER"))]
        } else {
            Vec::new()
        },
        ids: Vec::new(),
        valid: (hour == 9).then(|| vec![0]),
        reads: germany.map(|read| Reads::Every(vec![(GER, read), (ITA, 11)])),
        characters: Characters::Changed(Vec::new()),
        countries: vec![country],
    };
    let hours = vec![
        hour(9, Some(9), offer(9, true)),
        // Read 10 recorded no lines.
        hour(10, Some(10), offer(10, false)),
        // A truncated read dates Germany at 11, in its own hour only.
        hour(11, None, offer(11, true)),
        hour(12, Some(10), offer(10, false)),
        hour(13, Some(13), offer(13, true)),
    ];
    let budget = Budget::new(64 << 20);
    let built = build(hours, &[11, 12, 13], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let catalogue = super::query::catalogue(&mut snapshot, &directory, "GER")
        .unwrap()
        .unwrap();
    assert_eq!(
        catalogue.warnings,
        ["GER: 1 reads recorded no modifier lines with their inputs; their parts are not known."]
    );
}

/// A read that recorded no lines is counted once, though its country leaves the countries
/// with a holder (its link lost) over a cut, its inputs still dated there, and comes back to it.
#[test]
fn a_read_whose_country_left_the_holders_is_retired_by_its_recorded_date() {
    use super::facts::{Characters, Country, Hour, Named, Parts, Reads};
    let offer = |read: i32, lined: bool| Country {
        index: 0,
        holder: (7, GER),
        read,
        lined,
        naming: lined.then(|| Arc::new(dashboard_text::identity::NamingCountry::default())),
        parts: lined.then(|| {
            Parts::Named(Box::new(Named {
                measures: [Some(Vec::new()), Some(Vec::new())],
                names: Vec::new(),
                refused: Default::default(),
            }))
        }),
    };
    let hour = |hour: i32, germany: i32, valid: Option<Vec<u32>>, countries: Vec<Country>| Hour {
        branch: 1,
        hour,
        named: if hour == 9 {
            vec![(0, Arc::from("GER"))]
        } else {
            Vec::new()
        },
        ids: Vec::new(),
        valid,
        reads: Some(Reads::Every(vec![(GER, germany), (ITA, 11)])),
        characters: Characters::Changed(Vec::new()),
        countries,
    };
    let hours = vec![
        hour(9, 9, Some(vec![0]), vec![offer(9, true)]),
        // Read 10 recorded no lines.
        hour(10, 10, None, vec![offer(10, false)]),
        // Germany's link is lost: it holds no place, its read 10 still dated.
        hour(11, 10, Some(Vec::new()), Vec::new()),
        hour(12, 10, Some(vec![0]), vec![offer(10, false)]),
        hour(13, 13, None, vec![offer(13, true)]),
    ];
    let budget = Budget::new(64 << 20);
    let built = build(hours, &[11, 12, 13], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let catalogue = super::query::catalogue(&mut snapshot, &directory, "GER")
        .unwrap()
        .unwrap();
    assert_eq!(
        catalogue.warnings,
        ["GER: 1 reads recorded no modifier lines with their inputs; their parts are not known."]
    );
}
