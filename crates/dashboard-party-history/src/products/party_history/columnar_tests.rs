//! The family's columnar preparation held to the row scan ([`super::scan`]) on the same hours,
//! written as the native recorder writes them: each tag's popularity, ruling, name and leader
//! runs, elections, marks and refusals, across segment starts, a gap, cuts and a resume; and the
//! replies, as the row reply answers them.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    *,
};
use dashboard_base::{
    prepared::coverage::Coverage,
    test_fixtures::{CAMPAIGN, continuation_records, file_source, records, source},
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
        CharacterPortrait, CharacterPortraitRow, CharacterRow, CountryAppearanceRow,
        CountryLinkRow, CountryRow, LexicalTokenRow, PoliticalPartyRow, PoliticsRow, Recorded,
        ReservedCountryNamingRow, RowCodec, TableId, TableRows,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const REFUSAL: &str = "synthetic refusal";

/// One hour as the recorder writes it; no parties where the table is unavailable.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Vec<CountryRow>,
    links: Vec<CountryLinkRow>,
    parties: Option<Vec<PoliticalPartyRow>>,
    appearances: Vec<CountryAppearanceRow>,
    politics: Vec<PoliticsRow>,
    characters: Vec<CharacterRow>,
    portraits: Vec<CharacterPortraitRow>,
    tokens: Vec<LexicalTokenRow>,
}

/// Hours 100 to 150 without 120 and 121. GER (9001, slot 7, alias 11) has two parties whose
/// popularity moves every four hours and an alias's third; democrats rule 105 to 109, fascists
/// otherwise, renamed by `set_party_name` from 140; its leader changes at 112; its last election
/// moves at 108 and, across the gap, at 122. ENG changes id at 135, has no leader and an
/// unreadable token. FRA holds no party. The parties are unavailable at 125 and 126.
fn scenario() -> Vec<Tables> {
    (100..=150)
        .filter(|hour| !(120..=121).contains(hour))
        .map(|hour| {
            let eng = if hour >= 135 { 51 } else { 50 };
            let country = |id, canonical_slot, tag: &str| CountryRow {
                id,
                canonical_slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            };
            let mut countries = vec![country(9001, 7, "GER"), country(eng, 8, "ENG"), country(60, 9, "FRA")];
            countries.sort_by_key(|row| row.id);
            let mut links: Vec<_> = [(11, 7), (9001, 7), (eng, 8), (60, 9)]
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .to_vec();
            links.sort_by_key(|row| row.id);
            let democratic = 3_000_000 + i64::from(hour / 4) * 1_000;
            let ruling = if (105..110).contains(&hour) {
                "democratic"
            } else {
                "fascism"
            };
            let party = |country, id, token: &str, popularity, rules: bool| PoliticalPartyRow {
                country,
                id,
                popularity,
                is_ruling: u8::from(rules),
                token: token.as_bytes().to_vec(),
                tag: country,
                name_default: 1,
                ..Default::default()
            };
            let mut fascism = party(9001, 2, "fascism", 10_000_000 - democratic, ruling == "fascism");
            if hour >= 140 {
                fascism.name_default = 0;
                fascism.name = b"GER_custom_party".to_vec();
            }
            let mut parties = vec![
                party(9001, 1, "democratic", democratic, ruling == "democratic"),
                fascism,
                party(11, 3, "neutrality", 500_000, false),
                party(eng, 1, "democratic", 6_000_000, true),
                party(eng, 2, "", 1, false),
            ];
            parties.sort_by_key(|row| (row.country, row.id));
            let appearance = |country, leader: Option<u32>, ideology: &str| CountryAppearanceRow {
                country,
                leader_character: leader,
                leader_ideology: ideology.as_bytes().to_vec(),
                subjects: Recorded::Value(Vec::new()),
                original_tag: Recorded::Value(Some(Vec::new())),
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            };
            let (leader, ideology) = if hour < 112 {
                (7, "nazism")
            } else {
                (8, "conservatism")
            };
            let mut appearances = vec![appearance(9001, Some(leader), ideology), appearance(eng, None, "")];
            appearances.sort_by_key(|row| row.country);
            let election = match hour {
                ..=107 => 10,
                108..=121 => 20,
                _ => 30,
            };
            let mut politics = vec![
                PoliticsRow {
                    country: 9001,
                    last_election: election,
                    ..Default::default()
                },
                PoliticsRow {
                    country: eng,
                    last_election: 5,
                    ..Default::default()
                },
            ];
            politics.sort_by_key(|row| row.country);
            let character = |id, name_token_id, token: &str| CharacterRow {
                id,
                name_token_id,
                token: token.as_bytes().to_vec(),
                ..Default::default()
            };
            Tables {
                hour,
                countries,
                links,
                parties: (!(125..=126).contains(&hour)).then_some(parties),
                appearances,
                politics,
                characters: vec![character(7, 70, "GER_leader_a"), character(8, 80, "GER_leader_b")],
                portraits: vec![CharacterPortraitRow {
                    character: 7,
                    portraits: vec![CharacterPortrait {
                        category: replay_data::rows::PORTRAIT_CIVILIAN,
                        size: replay_data::rows::PORTRAIT_LARGE,
                        is_set: 1,
                        kind: 0,
                        identity: b"GFX_leader_a".to_vec(),
                    }],
                }],
                tokens: vec![
                    LexicalTokenRow {
                        token: 70,
                        literal: b"leader_a".to_vec(),
                    },
                    LexicalTokenRow {
                        token: 80,
                        literal: b"leader_b".to_vec(),
                    },
                ],
            }
        })
        .collect()
}

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = source(1, tables.hour, Some(1));
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    put(
        TableId::Countries,
        TableView::Complete(TableRows::Countries(tables.countries.clone())),
    );
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(tables.links.clone())),
    );
    put(
        TableId::PoliticalParties,
        match &tables.parties {
            Some(rows) => TableView::Complete(TableRows::PoliticalParties(rows.clone())),
            None => TableView::Unavailable {
                reason: REFUSAL.into(),
            },
        },
    );
    put(
        TableId::CountryAppearance,
        TableView::Complete(TableRows::CountryAppearance(tables.appearances.clone())),
    );
    put(
        TableId::Politics,
        TableView::Complete(TableRows::Politics(tables.politics.clone())),
    );
    put(
        TableId::Characters,
        TableView::Complete(TableRows::Characters(tables.characters.clone())),
    );
    put(
        TableId::CharacterPortraits,
        TableView::Complete(TableRows::CharacterPortraits(tables.portraits.clone())),
    );
    put(
        TableId::LexicalTokens,
        TableView::Complete(TableRows::LexicalTokens(tables.tokens.clone())),
    );
    put(
        TableId::ReservedCountryNaming,
        TableView::Unavailable {
            reason: REFUSAL.into(),
        },
    );
    frame
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
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, Some(&tables.countries[..]));
        write(walk, TableId::CountryLinks, Some(&tables.links[..]));
        write(walk, TableId::PoliticalParties, tables.parties.as_deref());
        write(walk, TableId::CountryAppearance, Some(&tables.appearances[..]));
        write(walk, TableId::Politics, Some(&tables.politics[..]));
        write(walk, TableId::Characters, Some(&tables.characters[..]));
        write(walk, TableId::CharacterPortraits, Some(&tables.portraits[..]));
        write(walk, TableId::LexicalTokens, Some(&tables.tokens[..]));
        write::<ReservedCountryNamingRow>(walk, TableId::ReservedCountryNaming, None);
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
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
        })
        .unwrap();
        frames.push(frame(tables));
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

/// The row recording of `frames`, split at the gap as the recorder splits it.
fn recording(frames: Vec<DecodedFrame>) -> Replay {
    let (first, second): (Vec<_>, Vec<_>) = frames.into_iter().partition(|frame| frame.hour < 122);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    replay
        .append(&[continuation_records(&replay, 1, second)])
        .unwrap()
}

/// `params`' reply from `built`, its coverage the whole branch.
fn ask(source: &Source, built: &Built, budget: &Budget, params: Value) -> Value {
    let cache = Cache::new(budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let accepted = Coverage {
        schema: dashboard_base::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 150,
        complete: true,
    };
    query::query(
        source,
        serde_json::from_value(params).unwrap(),
        &LoadContext { content: None },
        &mut snapshot,
        &directory,
        Some(accepted),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// The row scan's reply for `tag`.
fn expected(source: &Source, tag: &str) -> (ScannedParties, Value) {
    let cancelled = AtomicBool::new(false);
    let scanned = scan(&source.replay(), 1, tag, &cancelled).unwrap();
    let params = CountryParams {
        branch_id: "1".into(),
        tag: tag.into(),
    };
    let reply = reply(
        source,
        &params,
        &LoadContext { content: None },
        &scanned,
        None,
        &cancelled,
    )
    .unwrap();
    (scanned, serde_json::to_value(reply).unwrap())
}

#[test]
fn the_publication_holds_the_row_history_across_starts_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let (_, frames) = projected(&hours, &[]);
    let (source, root) = file_source(recording(frames));
    for (starts, cuts, resume) in [
        (&[][..], &[150][..], None),
        (&[113][..], &[112, 150][..], Some(112)),
        (&[106, 136][..], &[105, 130, 150][..], Some(130)),
        (&[110, 140][..], &[109, 139, 150][..], Some(139)),
    ] {
        let (projected, _) = projected(&hours, starts);
        let built = build(projected, cuts, resume, &budget);
        for tag in ["GER", "ENG", "FRA"] {
            let label = format!("{tag} at starts {starts:?}, cuts {cuts:?}");
            let history = ask(
                &source,
                &built,
                &budget,
                json!({"branchId": "1", "tag": tag, "mode": "history"}),
            );
            let (_, wanted) = expected(&source, tag);
            assert_eq!(history["reply"], wanted, "{label}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn states_marks_and_navigation_read_the_row_runs() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let (projected, frames) = projected(&hours, &[113]);
    let built = build(projected, &[112, 150], Some(112), &budget);
    let (source, root) = file_source(recording(frames));
    let (scanned, _) = expected(&source, "GER");
    let holding = |first: i32, last: i32, hour: i32| first <= hour && hour <= last;
    for hour in [100, 104, 105, 112, 119, 122, 125, 140, 150] {
        let state = ask(
            &source,
            &built,
            &budget,
            json!({"branchId": "1", "tag": "GER", "mode": "state", "hours": [hour]}),
        );
        let reply = &state["reply"];
        for party in reply["parties"].as_array().unwrap() {
            let token = party["token"].as_str().unwrap();
            let wanted: Vec<_> = scanned.parties[token]
                .runs
                .iter()
                .filter(|run| holding(run.first_hour, run.last_hour, hour))
                .collect();
            assert_eq!(party["runs"], json!(wanted), "{token} at {hour}");
        }
        let ruling: Vec<_> = scanned
            .ruling
            .iter()
            .filter(|run| holding(run.first_hour, run.last_hour, hour))
            .collect();
        assert_eq!(reply["ruling"], json!(ruling), "ruling at {hour}");
        let leaders: Vec<_> = scanned
            .leaders
            .iter()
            .filter(|run| holding(run.first_hour, run.last_hour, hour))
            .collect();
        assert_eq!(reply["leaders"], json!(leaders), "leader at {hour}");
        let names: Vec<_> = scanned
            .party_name_runs
            .iter()
            .filter(|run| holding(run.first_hour, run.last_hour, hour))
            .collect();
        assert_eq!(reply["partyNameRuns"], json!(names), "name at {hour}");
    }
    let events = |field: &str| {
        ask(
            &source,
            &built,
            &budget,
            json!({"branchId": "1", "tag": "GER", "mode": "events", "field": field,
                "from": 100, "to": 150}),
        )
    };
    // Changes of government at 105 and 110, of leader at 112; elections at 108 and, across the
    // gap, at 122.
    assert_eq!(events("ruling")["total"], "2");
    assert_eq!(events("leader")["total"], "1");
    let elections = events("election");
    assert_eq!(elections["total"], "2");
    assert_eq!(elections["reply"]["elections"], json!(scanned.elections));
    assert_eq!(scanned.elections, [108, 122]);
    let all = events("all");
    assert_eq!(all["total"], "5");
    let marks: Vec<i32> = all["reply"]["ruling"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|run| run["startsSegment"] == false)
        .map(|run| run["firstHour"].as_i64().unwrap() as i32)
        .collect();
    assert_eq!(marks, [105, 110]);
    let next = |fields: Value, values: bool, from: i32| {
        ask(
            &source,
            &built,
            &budget,
            json!({"branchId": "1", "tag": "GER", "mode": "next", "fields": fields,
                "values": values, "navigation": {"key": "ArrowRight", "from": from,
                "firstHour": 100, "lastHour": 150}}),
        )["hour"]
            .clone()
    };
    assert_eq!(next(json!(["ruling"]), false, 104), json!(105.0));
    assert_eq!(next(json!(["ruling"]), false, 105), json!(110.0));
    assert_eq!(next(json!(["election"]), false, 100), json!(108.0));
    assert_eq!(next(json!(["election"]), false, 108), json!(122.0));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_unchanged_hour_hands_the_emit_nothing_and_a_popularity_move_only_its_tag() {
    let hours = scenario();
    let (projected, _) = projected(&hours, &[]);
    for hour in &projected {
        let Some(facts) = &hour.facts else {
            continue;
        };
        if facts.whole {
            continue;
        }
        // Between whole hours only GER's popularity, ruling, name, leader and election move.
        for (tag, _) in &facts.changed {
            assert_eq!(&**tag, "GER", "hour {}", hour.hour);
        }
    }
    let quiet = projected
        .iter()
        .filter(|hour| (101..=103).contains(&hour.hour))
        .all(|hour| hour.facts.is_none());
    assert!(quiet, "popularity holds from 101 to 103");
    let moved = projected.iter().find(|hour| hour.hour == 104).unwrap();
    assert!(moved.facts.as_ref().is_some_and(|facts| facts.changed.len() == 1));
}
