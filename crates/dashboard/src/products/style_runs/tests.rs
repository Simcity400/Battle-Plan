use super::{Configured, emit, facts, facts_reply, styles_at};
use crate::{flag_choices::Chain, products::country_flags::FlagFolders};
use dashboard_base::products::country_styles::{
    CountryFactRuns, CountryFlagChoice, HourStyles, StyleInputs,
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use facts::{Country, Fact, Hour, Naming};
use installed_content_country_names::country_names::Operands;
use installed_content_localisation::localisation::Localisation;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        AutonomyRow, CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow,
        PoliticsRow, Recorded, ReservedCountryNamingRow, RowCodec, TableId,
    },
    scope::CaptureMode,
};
use std::sync::{Arc, atomic::AtomicBool};

/// Why an atlas draws nothing, when nothing more is known.
const UNAVAILABLE: &str = "flag artwork unavailable in the recorded content";

/// A one-pixel flag the decoder reads at a one-pixel cell.
fn targa() -> Vec<u8> {
    vec![
        0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x28, 30, 20, 10, 255,
    ]
}

/// Country styles choosing flags among `files`, each a one-pixel flag at one-pixel cells, with
/// the content folders it reads, removed when dropped.
fn configured(files: &[&str]) -> (Configured, [crate::catalogue::tests::Folder; 2]) {
    let textures: Vec<(&str, Vec<u8>)> = files.iter().map(|&path| (path, targa())).collect();
    let (catalogue, folders) = crate::catalogue::texture_tests::texture_catalogue(&[], &textures);
    let flags = FlagFolders::of(catalogue.content.layers.clone(), [[1, 1]; 3]);
    (Configured::with(Ok(flags)), folders)
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: emit::SCHEMA,
    }
}

/// A publication built hour by hour from facts, cut after each of `cuts`' hours.
struct Built {
    pages: Memory,
    root: u64,
}

/// [`Built`] by one emit, saved at each cut of `restarts` and loaded again from the publication
/// so far, as a restart goes on: the hour after holds a segment's first projection.
fn build(
    configured: &Configured,
    hours: Vec<Hour>,
    cuts: &[i32],
    restarts: &[i32],
    budget: &Budget,
) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = configured.emit();
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
        if restarts.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
            emit = emit::Emit::load(configured, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn hour(at: i32, known: bool, whole: bool, named: &[(u32, &str)], countries: Vec<Country>) -> Hour {
    Hour {
        branch: 1,
        hour: at,
        known,
        whole,
        named: named
            .iter()
            .map(|&(index, tag)| (index, Arc::from(tag)))
            .collect(),
        countries,
    }
}

fn chain(candidates: &[&str]) -> Arc<Chain> {
    Arc::new(Chain {
        candidates: candidates.iter().map(|&key| key.to_owned()).collect(),
        refusal: None,
    })
}

/// A tag held by `holder`, named for `tag` under `group`.
fn fact(
    holder: (u32, u32),
    colour: Option<[u8; 3]>,
    (tag, group): (&str, Option<&str>),
    flag: Arc<Chain>,
) -> Option<Fact> {
    Some(Fact {
        holder,
        colour,
        naming: Arc::new(Naming {
            chain: vec![(holder.1, Ok(Operands::for_tag(tag, group)))],
            built: None,
        }),
        flag,
    })
}

fn country(index: u32, fact: Option<Fact>) -> Country {
    Country { index, fact }
}

fn rgb([red, green, blue]: [u8; 3]) -> i32 {
    i32::from(red) << 16 | i32::from(green) << 8 | i32::from(blue)
}

fn choice(keys: [Option<&str>; 3], reasons: [Option<&str>; 3]) -> CountryFlagChoice {
    CountryFlagChoice {
        keys: keys.map(|key| key.map(str::to_owned)),
        reasons: reasons.map(|reason| reason.map(str::to_owned)),
    }
}

/// GER's flag stays one choice through a cosmetic tag no file draws, and becomes another with
/// its ideology; ZZZ's mod has no file for any candidate. An unknown hour leaves a gap in the
/// known spans, a colour change after it starts a run, and a tag no country holds names why. A
/// restart at the cut before the gap goes on from the publication alone.
#[test]
fn the_facts_reply_gives_each_tags_runs_and_flag_choices() {
    let (configured, _folders) = configured(&[
        "gfx/flags/GER_fascism.tga",
        "gfx/flags/GER.tga",
        "gfx/flags/medium/GER.tga",
        "gfx/flags/small/GER.tga",
    ]);
    let ger = |colour, candidates: &[&str]| {
        fact(
            (1, 1),
            Some(colour),
            ("GER", Some("fascism")),
            chain(candidates),
        )
    };
    let zzz = || {
        fact(
            (5, 5),
            None,
            ("ZZZ", None),
            chain(&["ORI_neutrality", "ORI"]),
        )
    };
    let fascist = ["GER_fascism", "GER"];
    let hours = vec![
        hour(
            100,
            true,
            true,
            &[(0, "GER"), (1, "ZZZ")],
            vec![country(0, ger([10, 20, 30], &fascist)), country(1, zzz())],
        ),
        hour(101, true, false, &[], vec![]),
        hour(
            102,
            true,
            false,
            &[],
            vec![country(
                0,
                ger(
                    [10, 20, 30],
                    &["GER_COS_fascism", "GER_COS", "GER_fascism", "GER"],
                ),
            )],
        ),
        hour(
            103,
            true,
            false,
            &[],
            vec![country(0, ger([10, 20, 30], &["GER_communism", "GER"]))],
        ),
        hour(104, false, false, &[], vec![]),
        // A segment starts: its projection names the tags anew and hands every one.
        hour(
            105,
            true,
            true,
            &[(0, "ZZZ"), (1, "GER")],
            vec![
                country(0, zzz()),
                country(1, ger([11, 20, 30], &["GER_communism", "GER"])),
            ],
        ),
        hour(106, true, false, &[], vec![country(0, None)]),
    ];
    let localisation: Localisation = [("GER".to_owned(), "German Reich".to_owned())]
        .into_iter()
        .collect();
    let budget = Budget::new(64 << 20);
    for restarts in [&[][..], &[103][..]] {
        let built = build(&configured, hours.clone(), &[103, 106], restarts, &budget);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
        let cancelled = AtomicBool::new(false);
        let reply = facts_reply(
            &mut snapshot,
            &directory,
            1,
            [100, 106],
            Some(&localisation),
            &cancelled,
        )
        .unwrap();
        assert_eq!(reply.branch_id, "1");
        assert_eq!(reply.known, [[100, 103], [105, 106]]);
        assert_eq!(reply.names, ["German Reich"]);
        assert_eq!(
            reply.reasons,
            [
                "no localisation key names ZZZ; no recorded map colour",
                "not a country at this hour"
            ]
        );
        assert_eq!(
            reply.flags,
            [
                choice([Some("GER_fascism"), Some("GER"), Some("GER")], [None; 3]),
                choice([Some("GER"); 3], [None; 3]),
                choice([None; 3], [Some(UNAVAILABLE); 3]),
            ]
        );
        assert_eq!(
            reply.countries,
            [
                CountryFactRuns {
                    tag: "GER".into(),
                    runs: vec![
                        [100, 0, rgb([10, 20, 30]), -1, 0],
                        [103, 0, rgb([10, 20, 30]), -1, 1],
                        [105, 0, rgb([11, 20, 30]), -1, 1],
                    ],
                },
                CountryFactRuns {
                    tag: "ZZZ".into(),
                    runs: vec![[100, -1, -1, 0, 2], [106, -1, -1, 1, -1]],
                },
            ],
            "restarts: {restarts:?}"
        );
        // Without a language, names are missing and say nothing of it.
        let unnamed =
            facts_reply(&mut snapshot, &directory, 1, [100, 106], None, &cancelled).unwrap();
        assert!(unnamed.names.is_empty());
        assert_eq!(
            unnamed.countries[0].runs[0],
            [100, -1, rgb([10, 20, 30]), -1, 0]
        );
        // A span inside the publication gives its own first hour.
        let inside =
            facts_reply(&mut snapshot, &directory, 1, [102, 102], None, &cancelled).unwrap();
        assert_eq!(inside.known, [[102, 102]]);
        assert_eq!(
            inside.countries[0].runs,
            [[102, -1, rgb([10, 20, 30]), -1, 0]]
        );
        assert!(
            facts_reply(&mut snapshot, &directory, 1, [107, 106], None, &cancelled)
                .unwrap()
                .countries
                .is_empty()
        );

        // The hours preparation knows answer the style products; an unknown one leaves them
        // all to a seek.
        let mut visited: Vec<(i32, StyleInputs)> = Vec::new();
        let tags = ["GER", "ZZZ", "AAA"].map(str::to_owned);
        assert!(
            styles_at(
                &mut snapshot,
                &directory,
                &[100, 102, 105],
                &tags,
                true,
                &cancelled,
                &mut |hour, inputs| {
                    visited.push((hour, inputs));
                    Ok(())
                },
            )
            .unwrap()
        );
        assert_eq!(
            visited.iter().map(|(hour, _)| *hour).collect::<Vec<_>>(),
            [100, 102, 105]
        );
        let (_, at_100) = &visited[0];
        assert_eq!(
            at_100.tags.iter().collect::<Vec<_>>(),
            [(&"GER".to_owned(), &(1, 1)), (&"ZZZ".to_owned(), &(5, 5))]
        );
        assert_eq!(at_100.colors.get(&1), Some(&[10, 20, 30]));
        assert_eq!(at_100.colors.get(&5), None);
        let styles = HourStyles::build(at_100.clone(), Some(&localisation));
        assert_eq!(
            styles.get("GER").unwrap().name.as_deref(),
            Some("German Reich")
        );
        assert_eq!(visited[2].1.colors.get(&1), Some(&[11, 20, 30]));
        let mut seen = 0;
        assert!(
            !styles_at(
                &mut snapshot,
                &directory,
                &[103, 104],
                &tags,
                false,
                &cancelled,
                &mut |_, _| {
                    seen += 1;
                    Ok(())
                },
            )
            .unwrap()
        );
        assert_eq!(seen, 0);
    }
}

/// Content that cannot be read refuses every flag by its reason; names and colours stand.
#[test]
fn refused_content_names_its_refusal_in_every_flag() {
    let refusal = "flags cannot be chosen: Game content is not configured";
    let configured = Configured::with(Err(refusal.into()));
    let hours = vec![hour(
        0,
        true,
        true,
        &[(0, "GER")],
        vec![country(
            0,
            fact((1, 1), Some([1, 2, 3]), ("GER", None), chain(&["GER"])),
        )],
    )];
    let budget = Budget::new(16 << 20);
    let built = build(&configured, hours, &[0], &[], &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    let reply = facts_reply(
        &mut snapshot,
        &directory,
        1,
        [0, 0],
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.flags, [choice([None; 3], [Some(refusal); 3])]);
    assert_eq!(reply.countries[0].runs, [[0, -1, rgb([1, 2, 3]), -1, 0]]);
}

/// 1,500 countries over 120 hours, cut at uneven hours and restarted at one: every tag's runs
/// are its own values' changes, each flag choice numbered once whichever chain chose it.
#[test]
fn fifteen_hundred_countries_keep_their_runs_through_cuts() {
    const TAGS: u32 = 1_500;
    const HOURS: i32 = 120;
    let (configured, _folders) = configured(&["gfx/flags/A.tga", "gfx/flags/B.tga"]);
    // "X, A" chooses what "A" does; "C" chooses nothing.
    let chains = [
        chain(&["A"]),
        chain(&["B"]),
        chain(&["C"]),
        chain(&["X", "A"]),
    ];
    let drawn = [Some("A"), Some("B"), None, Some("A")];
    let tag = |index: u32| format!("T{index:04}");
    // Each tag's colour moves every few hours and its flag chain every few more; one in a
    // hundred is no country for twenty hours.
    let colour = |index: u32, at: i32| {
        let step = (at / (3 + (index % 5) as i32)) as u8;
        [step, (index % 251) as u8, (index / 251) as u8]
    };
    let flag = |index: u32, at: i32| ((index as i32 + at / (7 + (index % 3) as i32)) % 4) as usize;
    let value = |index: u32, at: i32| {
        let gone = index % 100 == 7 && (40..60).contains(&at);
        (!gone).then(|| (colour(index, at), flag(index, at)))
    };
    let tags: Vec<String> = (0..TAGS).map(tag).collect();
    let mut hours = Vec::new();
    for at in 0..HOURS {
        // A segment starts after the restart at 60.
        let whole = at == 0 || at == 61;
        let named: Vec<(u32, &str)> = if whole {
            (0..TAGS)
                .zip(&tags)
                .map(|(index, tag)| (index, tag.as_str()))
                .collect()
        } else {
            Vec::new()
        };
        let countries = (0..TAGS)
            .filter(|&index| whole || value(index, at) != value(index, at - 1))
            .map(|index| {
                let fact = value(index, at).and_then(|(colour, flag)| {
                    fact(
                        (index + 1, index + 1),
                        Some(colour),
                        (&tags[index as usize], None),
                        Arc::clone(&chains[flag]),
                    )
                });
                country(index, fact)
            })
            .collect();
        hours.push(hour(at, true, whole, &named, countries));
    }
    let budget = Budget::new(256 << 20);
    let built = build(&configured, hours, &[17, 60, 98, HOURS - 1], &[60], &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    let cancelled = AtomicBool::new(false);
    let reply = facts_reply(
        &mut snapshot,
        &directory,
        1,
        [0, HOURS - 1],
        None,
        &cancelled,
    )
    .unwrap();
    assert_eq!(reply.known, [[0, HOURS - 1]]);
    assert_eq!(reply.flags.len(), 3);
    assert_eq!(reply.countries.len(), TAGS as usize);
    for (index, runs) in (0..TAGS).zip(&reply.countries) {
        assert_eq!(runs.tag, tags[index as usize]);
        // Each change of colour, drawn key or holding, as the reply gives it.
        let mut model = Vec::new();
        let mut shown = None;
        for at in 0..HOURS {
            let next = value(index, at).map(|(colour, flag)| (rgb(colour), drawn[flag]));
            if shown != Some(next) {
                shown = Some(next);
                model.push((at, next));
            }
        }
        let given: Vec<(i32, Option<(i32, Option<&str>)>)> = runs
            .runs
            .iter()
            .map(|run| {
                let held = (run[4] >= 0).then(|| {
                    let keys = &reply.flags[run[4] as usize].keys;
                    (run[2], keys[0].as_deref())
                });
                (run[0], held)
            })
            .collect();
        assert_eq!(given, model, "{}", runs.tag);
    }
    // Every tag at three hours, as the style products ask them.
    let mut visited = 0;
    assert!(
        styles_at(
            &mut snapshot,
            &directory,
            &[0, 50, HOURS - 1],
            &tags,
            false,
            &cancelled,
            &mut |at, inputs| {
                for index in 0..TAGS {
                    let held = inputs.tags.get(&tags[index as usize]).copied();
                    match value(index, at) {
                        None => assert_eq!(held, None, "{at} {index}"),
                        Some((colour, _)) => {
                            assert_eq!(held, Some((index + 1, index + 1)));
                            assert_eq!(inputs.colors.get(&(index + 1)), Some(&colour));
                        }
                    }
                }
                visited += 1;
                Ok(())
            },
        )
        .unwrap()
    );
    assert_eq!(visited, 3);
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

/// One hour's rows of the style tables; a table of None is unavailable.
#[derive(Clone)]
struct Rows {
    countries: Vec<CountryRow>,
    links: Vec<CountryLinkRow>,
    appearances: Option<Vec<CountryAppearanceRow>>,
    politics: Option<Vec<PoliticsRow>>,
    parties: Vec<PoliticalPartyRow>,
}

/// One hour's projection, with the row decoder's flag choices of the hour, by tag.
type Projected = (Hour, Vec<(String, Vec<String>, Option<String>)>);

/// Walks `script` hour by hour from hour 100, a new segment starting at ordinal `segment`: each
/// hour's projection, and the full rebuilds of every segment's projection by cause.
fn project(script: &[Rows], segment: usize) -> (Vec<Projected>, [(&'static str, u64); 4]) {
    let configured = Configured::with(Err("unused".into()));
    let needs = facts::needs().unwrap();
    // The row decoder's frame of the style tables, Campaign aside.
    let mut walk = Synthetic::new(&facts::TABLES[1..], 1, 100).unwrap();
    let mut held: Option<dashboard_world::World> = None;
    let rebuilds = Arc::default();
    let mut projector = configured.projector(&rebuilds).unwrap();
    let mut projected = Vec::new();
    for (ordinal, rows) in script.iter().enumerate() {
        write(&mut walk, TableId::Countries, Some(&rows.countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&rows.links[..]));
        write(
            &mut walk,
            TableId::CountryAppearance,
            rows.appearances.as_deref(),
        );
        write(&mut walk, TableId::Politics, rows.politics.as_deref());
        write(
            &mut walk,
            TableId::PoliticalParties,
            Some(&rows.parties[..]),
        );
        write::<AutonomyRow>(&mut walk, TableId::Autonomy, Some(&[]));
        write::<ReservedCountryNamingRow>(&mut walk, TableId::ReservedCountryNaming, Some(&[]));
        let restarts = ordinal == segment;
        if restarts {
            projector = configured.projector(&rebuilds).unwrap();
            held = None;
        }
        let mut out: Vec<Hour> = Vec::new();
        let decoded = walk
            .end(|state, manifest, frame, starts| {
                let starts = starts || restarts;
                let world = dashboard_world::World::at(
                    &mut held,
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
                            hour: frame.hour,
                            run: 0,
                            ordinal: ordinal as u32,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                            world,
                        },
                        &mut out,
                    )
                    .unwrap();
                crate::flag_choices::choices(frame)
                    .unwrap()
                    .into_iter()
                    .map(|choice| (choice.tag, choice.candidates, choice.refusal))
                    .collect::<Vec<_>>()
            })
            .unwrap();
        projected.push((out.pop().unwrap(), decoded));
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(projected).unwrap(), rebuilds.counts())
}

/// The projector on a synthetic walk: the first hour hands every tag; a cosmetic tag or a
/// ruling ideology changes only its country's chain, which the row decoder's frame of the hour
/// chooses alike; an original tag joins its country's chain; an hour without appearances is
/// unknown, and the known hour after hands only what changed; an incomplete Politics names
/// its refusal in names and flags; a segment's first hour hands every tag again.
#[test]
fn the_projector_hands_only_what_changed_and_chooses_as_the_frame_does() {
    let countries = [("GER", 1_u32), ("ZZZ", 5)].map(|(tag, id)| CountryRow {
        id,
        canonical_slot: id,
        tag: tag.as_bytes().to_vec(),
        ..Default::default()
    });
    let links = [1_u32, 5].map(|id| CountryLinkRow {
        id,
        canonical_slot: id,
    });
    let appearance =
        |country: u32, cosmetic: &[u8], original: Option<&[u8]>| CountryAppearanceRow {
            country,
            red: 1.0_f32.to_bits(),
            cosmetic_tag: cosmetic.to_vec(),
            original_tag_reference: Recorded::Value(Some(if original.is_some() { 9 } else { 0 })),
            original_tag: Recorded::Value(original.map(<[u8]>::to_vec)),
            ..Default::default()
        };
    let ruling = |ger: u32| {
        vec![
            PoliticsRow {
                country: 1,
                ruling_party: Some(ger),
                ..Default::default()
            },
            PoliticsRow {
                country: 5,
                ruling_party: Some(2),
                ..Default::default()
            },
        ]
    };
    let party = |country: u32, id: u32, token: &[u8]| PoliticalPartyRow {
        country,
        id,
        token: token.to_vec(),
        ..Default::default()
    };
    let base = Rows {
        countries: countries.to_vec(),
        links: links.to_vec(),
        appearances: Some(vec![
            appearance(1, b"", None),
            appearance(5, b"", Some(b"ORI")),
        ]),
        politics: Some(ruling(4)),
        parties: vec![
            party(1, 4, b"fascism"),
            party(1, 6, b"communism"),
            party(5, 2, b"neutrality"),
        ],
    };
    let cosmetic = Rows {
        appearances: Some(vec![
            appearance(1, b"GER_COS", None),
            appearance(5, b"", Some(b"ORI")),
        ]),
        ..base.clone()
    };
    let communist = Rows {
        politics: Some(ruling(6)),
        ..cosmetic.clone()
    };
    let script = [
        base,
        cosmetic,
        communist.clone(),
        Rows {
            appearances: None,
            ..communist.clone()
        },
        communist.clone(),
        Rows {
            politics: None,
            ..communist.clone()
        },
        communist,
    ];
    let (projected, rebuilds) = project(&script, 6);
    // Two segments start; the hour after an unknown one and an incomplete Politics rebuild once.
    assert_eq!(rebuilds.map(|(_, count)| count), [2, 1, 1, 0]);
    let handed = |at: usize| -> Vec<(u32, Option<(Vec<String>, Option<String>)>)> {
        projected[at]
            .0
            .countries
            .iter()
            .map(|country| {
                (
                    country.index,
                    country
                        .fact
                        .as_ref()
                        .map(|fact| (fact.flag.candidates.clone(), fact.flag.refusal.clone())),
                )
            })
            .collect()
    };
    let keys = |keys: &[&str]| keys.iter().map(|&key| key.to_owned()).collect::<Vec<_>>();
    let shape: Vec<(bool, bool)> = projected
        .iter()
        .map(|(hour, _)| (hour.known, hour.whole))
        .collect();
    assert_eq!(
        shape,
        [
            (true, true),
            (true, false),
            (true, false),
            (false, false),
            (true, false),
            (true, false),
            (true, true),
        ]
    );
    let ori = (keys(&["ORI_neutrality", "ORI"]), None);
    let red = keys(&["GER_COS_communism", "GER_COS", "GER_communism", "GER"]);
    assert_eq!(
        handed(0),
        [
            (0, Some((keys(&["GER_fascism", "GER"]), None))),
            (1, Some(ori.clone())),
        ]
    );
    assert_eq!(
        projected[0]
            .0
            .named
            .iter()
            .map(|(index, tag)| (*index, tag.to_string()))
            .collect::<Vec<_>>(),
        [(0, "GER".to_owned()), (1, "ZZZ".to_owned())]
    );
    assert_eq!(
        handed(1),
        [(
            0,
            Some((
                keys(&["GER_COS_fascism", "GER_COS", "GER_fascism", "GER"]),
                None
            ))
        )]
    );
    assert_eq!(handed(2), [(0, Some((red.clone(), None)))]);
    assert!(handed(3).is_empty());
    assert!(handed(4).is_empty());
    let politics = |id: u32| Some(format!("country {id} flag needs its Politics row"));
    assert_eq!(
        handed(5),
        [
            (0, Some((Vec::new(), politics(1)))),
            (1, Some((Vec::new(), politics(5))))
        ]
    );
    let refused = projected[5].0.countries[0].fact.as_ref().unwrap();
    assert_eq!(
        refused.naming.chain,
        [(1, Err("Politics is incomplete this hour".to_owned()))]
    );
    assert_eq!(handed(6), [(0, Some((red, None))), (1, Some(ori))]);
    // Each known hour's chains are the row decoder's choices of the same hour.
    for (hour, decoded) in &projected {
        if !hour.known {
            continue;
        }
        let ger = projected
            .iter()
            .take_while(|(earlier, _)| earlier.hour <= hour.hour)
            .flat_map(|(earlier, _)| &earlier.countries)
            .filter(|country| country.index == 0)
            .last()
            .and_then(|country| country.fact.as_ref())
            .unwrap();
        let (_, candidates, refusal) = decoded.iter().find(|(tag, ..)| tag == "GER").unwrap();
        assert_eq!(&ger.flag.candidates, candidates, "{}", hour.hour);
        assert_eq!(&ger.flag.refusal, refusal, "{}", hour.hour);
        assert_eq!(ger.holder, (1, 1));
        assert_eq!(ger.colour, Some([255, 0, 0]));
    }
}

/// 1,500 countries on a synthetic walk: the first hour hands every one, and one country's new
/// cosmetic tag hands that country alone.
#[test]
fn fifteen_hundred_countries_hand_only_the_country_that_changed() {
    const COUNT: u32 = 1_500;
    let tag = |id: u32| format!("C{id:04}").into_bytes();
    let countries: Vec<CountryRow> = (1..=COUNT)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: tag(id),
            ..Default::default()
        })
        .collect();
    let links: Vec<CountryLinkRow> = (1..=COUNT)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        })
        .collect();
    let appearances = |changed: Option<u32>| -> Vec<CountryAppearanceRow> {
        (1..=COUNT)
            .map(|id| CountryAppearanceRow {
                country: id,
                cosmetic_tag: if changed == Some(id) {
                    b"COS".to_vec()
                } else {
                    Vec::new()
                },
                original_tag_reference: Recorded::Value(Some(0)),
                original_tag: Recorded::Value(None),
                ..Default::default()
            })
            .collect()
    };
    let politics: Vec<PoliticsRow> = (1..=COUNT)
        .map(|id| PoliticsRow {
            country: id,
            ruling_party: Some(1),
            ..Default::default()
        })
        .collect();
    let parties: Vec<PoliticalPartyRow> = (1..=COUNT)
        .map(|id| PoliticalPartyRow {
            country: id,
            id: 1,
            token: b"neutrality".to_vec(),
            ..Default::default()
        })
        .collect();
    let rows = |changed| Rows {
        countries: countries.clone(),
        links: links.clone(),
        appearances: Some(appearances(changed)),
        politics: Some(politics.clone()),
        parties: parties.clone(),
    };
    let (projected, rebuilds) = project(&[rows(None), rows(None), rows(Some(777))], usize::MAX);
    assert_eq!(rebuilds.map(|(_, count)| count), [1, 0, 0, 0]);
    assert_eq!(projected[0].0.countries.len(), COUNT as usize);
    assert!(projected[0].0.whole);
    assert!(projected[1].0.countries.is_empty());
    let changed = &projected[2].0.countries;
    assert_eq!(changed.len(), 1);
    let flag = &changed[0].fact.as_ref().unwrap().flag;
    assert_eq!(
        flag.candidates,
        ["COS_neutrality", "COS", "C0777_neutrality", "C0777"]
    );
}
