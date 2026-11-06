//! The family's columnar preparation held to its row oracle ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every country's score, identity and continuity
//! at each hour, the multipliers, warnings, charts and navigation, across segment starts, a
//! gap, cuts and a resume; and the projection's work, which reads only the countries an hour's
//! changed rows reach.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    tests::{factories, parity_frame, prepared_naming, row_score_frame, rows, set},
    *,
};
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
    rows::{FactionMemberRow, RowCodec, TableRows, TableSchema},
    scope::CaptureMode,
};
use replay_frame::frame::TableView;
use serde_json::{Value, json};

fn content() -> ScoreContent {
    let mut hint_warnings = BTreeMap::new();
    let hints = score_hints(None, &mut hint_warnings);
    ScoreContent {
        naming: prepared_naming(),
        recorded_language: "english".into(),
        multipliers: content_multipliers(Err("Recorded content unavailable")),
        hints,
        hint_warnings,
    }
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>, reason: &str) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// Writes each table `frame` holds whole or refuses, as the native recorder writes it.
macro_rules! write_tables {
    ($walk:expr, $frame:expr; $($table:ident),* $(,)?) => {
        $(match &*$frame.tables[TableId::$table.index()] {
            TableView::Complete(TableRows::$table(rows)) => {
                write($walk, TableId::$table, Some(&rows[..]), "")
            }
            TableView::Unavailable { reason } => write::<
                <tables::$table as TableSchema>::Row,
            >($walk, TableId::$table, None, reason),
            _ => panic!("fixture table {:?} is neither complete nor refused", TableId::$table),
        })*
    };
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again), and the row frames of the same hours.
fn projected<'c>(
    content: &'c ScoreContent,
    frames: &[DecodedFrame],
    starts: &[i32],
) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut decoded = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(content, FormatVersion::CURRENT).unwrap();
    let needs = facts::needs().unwrap();
    let mut last: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let gap = last.is_some_and(|last| last + 1 != frame.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        last = Some(frame.hour);
        let walk = walk.as_mut().unwrap();
        write_tables!(walk, frame;
            Campaign, Countries, CountryLinks, CountryGameScoreInputs, AirWingGameScoreStats,
            GameScoreMultipliers, CountryFactories, AirBaseRegistrations, CountryAppearance,
            Politics, PoliticalParties, Autonomy, Factions, FactionMembers, LexicalTokens,
            ReservedCountryNaming,
        );
        let cold = starts.contains(&frame.hour);
        if cold {
            projector.end(&mut out).unwrap();
            projector = Projector::new(content, FormatVersion::CURRENT).unwrap();
        }
        let row = walk
            .end(|state, manifest, row, first| {
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
                            hour: row.hour,
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
                row.clone()
            })
            .unwrap();
        decoded.push(row);
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), decoded)
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

fn build(
    content: &ScoreContent,
    hours: Vec<facts::Hour>,
    cuts: &[i32],
    resume: Option<i32>,
) -> Built {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new(content);
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
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// The row oracle's store of `frames`, every hour read whole.
fn oracle(content: &ScoreContent, frames: &[DecodedFrame]) -> dashboard_base::prepared::series::tests::Fixture {
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut state = row::Prepared::default();
    let mut observer = content.observer();
    for frame in frames {
        state
            .observe(frame, FormatVersion::CURRENT, &mut observer, content)
            .unwrap();
    }
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    fixture
}

/// Every request the comparison asks over hours `first..=last`.
fn requests(first: i32, last: i32, tags: &[&str]) -> Vec<(&'static str, Value)> {
    let mut asked = Vec::new();
    for hour in first..=last {
        asked.push((
            "series-state",
            json!({"branchId": "1", "family": {"kind": "game-score"}, "reads": [{"hour": hour}]}),
        ));
        asked.push((
            "series-state",
            json!({"branchId": "1", "family": {"kind": "game-score"}, "tags": tags,
                "reads": [{"hour": hour}]}),
        ));
    }
    for measure in PARTS {
        let family = json!({"kind": "game-score", "measure": measure});
        for width in [4, last - first + 1] {
            asked.push((
                "series-summary",
                json!({"branchId": "1", "family": family, "tags": tags, "from": first,
                    "to": last, "width": width}),
            ));
        }
        for hour in [first, first + 7, last - 3, last] {
            for key in ["ArrowLeft", "ArrowRight"] {
                asked.push((
                    "series-next-change",
                    json!({"branchId": "1", "family": family, "key": key, "from": hour,
                        "firstHour": first, "lastHour": last}),
                ));
            }
        }
        for key in ["Home", "End"] {
            asked.push((
                "series-next-change",
                json!({"branchId": "1", "family": family, "tags": tags, "key": key,
                    "firstHour": first, "lastHour": last}),
            ));
        }
    }
    asked
}

/// Holds the publication `built` to the row oracle `fixture` on every request.
fn check(
    built: &Built,
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    (first, last): (i32, i32),
    label: &str,
) {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let flag = AtomicBool::new(false);
    for (product, params) in requests(first, last, &["BBB", "DDD", "FFF"]) {
        let expected =
            row::query(product, params.clone(), &fixture.store, &fixture.key, &flag).unwrap();
        let actual =
            query::series(product, &params, &mut snapshot, &directory, None, &flag).unwrap();
        assert_eq!(actual, expected, "{label}: {product} {params}");
    }
}

/// Hours 100 to 130 of six countries, without 118 and 119: an army grows at 103, a wing's
/// planes at 105, a member joins a faction at 107, a colour changes at 109, the multipliers at
/// 111, the inputs are refused at 113, a country changes its native id at 116, a subject's
/// cosmetic tag at 122, a country's factories at 125, a faction is renamed at 127, and a
/// country leaves at 128.
fn scenario() -> Vec<DecodedFrame> {
    (100..=130)
        .filter(|hour| !(118..=119).contains(hour))
        .map(|hour| {
            let mut frame = parity_frame();
            frame.hour = hour;
            if hour >= 103 {
                rows::<tables::CountryGameScoreInputs>(&mut frame)[2].army_count = Some(40);
            }
            if hour >= 105 {
                rows::<tables::AirWingGameScoreStats>(&mut frame)[1].planes = Some(9);
            }
            if hour >= 107 {
                let members = rows::<tables::FactionMembers>(&mut frame);
                members.push(FactionMemberRow {
                    country: 1005,
                    faction: 11,
                    ordinal: 1,
                });
                members.sort_by_key(|row| row.country);
            }
            if hour >= 109 {
                rows::<tables::CountryAppearance>(&mut frame)[1].red = 0.5_f32.to_bits();
            }
            if hour >= 111 {
                rows::<tables::GameScoreMultipliers>(&mut frame)[0].army_raw = Some(2 * Q);
            }
            if hour == 113 {
                *frame.tables[TableId::CountryGameScoreInputs.index()] = TableView::Unavailable {
                    reason: "synthetic score inputs unavailable".into(),
                };
            }
            if hour >= 116 {
                // Country DDD's native id moves; its slot's link moves with it.
                rows::<tables::Countries>(&mut frame)[3].id = 1104;
                rows::<tables::Countries>(&mut frame).sort_by_key(|row| row.id);
                let links = rows::<tables::CountryLinks>(&mut frame);
                links[3].id = 1104;
                links.sort_by_key(|row| row.id);
                for row in rows::<tables::CountryGameScoreInputs>(&mut frame) {
                    if row.country == 1004 {
                        row.country = 1104;
                    }
                }
                rows::<tables::CountryGameScoreInputs>(&mut frame).sort_by_key(|row| row.country);
                set(
                    &mut frame,
                    TableRows::CountryFactories(
                        [1001, 1002, 1003, 1005, 1006, 1104]
                            .into_iter()
                            .map(factories)
                            .collect(),
                    ),
                );
            }
            if hour >= 122 {
                rows::<tables::CountryAppearance>(&mut frame)[2].cosmetic_tag = b"COS".to_vec();
            }
            if hour >= 125 {
                rows::<tables::CountryFactories>(&mut frame)[0].military_total_raw = 900_001;
            }
            if hour >= 127 {
                rows::<tables::Factions>(&mut frame)[0].name = b"Achse".to_vec();
            }
            if hour >= 128 {
                // Country EEE leaves.
                rows::<tables::Countries>(&mut frame).retain(|row| row.id != 1005);
                rows::<tables::CountryLinks>(&mut frame).retain(|row| row.id != 1005);
            }
            frame
        })
        .collect()
}

#[test]
fn the_publication_answers_as_the_row_oracle_across_starts_cuts_a_gap_and_a_resume() {
    let content = content();
    let frames = scenario();
    for (label, starts, cuts, resume) in [
        ("one segment", &[][..], &[130][..], None),
        ("segment starts", &[104, 110, 117, 126][..], &[130][..], None),
        ("cuts", &[110][..], &[103, 110, 117, 125, 130][..], None),
        ("resume", &[113, 120][..], &[112, 120, 130][..], Some(112)),
        ("resume after the gap", &[121][..], &[117, 120, 130][..], Some(120)),
    ] {
        let (hours, decoded) = projected(&content, &frames, starts);
        let fixture = oracle(&content, &decoded);
        let built = build(&content, hours, cuts, resume);
        check(&built, &fixture, (100, 130), label);
    }
}

#[test]
fn the_projection_reads_the_whole_hour_only_where_it_opens() {
    let content = content();
    let frames = scenario();
    let (hours, _) = projected(&content, &frames, &[110]);
    for hour in &hours {
        let whole = matches!(hour.facts.as_deref(), Some(facts::Facts::Whole { .. }));
        // A segment's first hours read whole, and so do the hours a holder or link changes,
        // the multipliers change and the inputs are refused or come back.
        let expected = matches!(hour.hour, 100 | 110 | 111 | 113 | 114 | 116 | 120 | 128);
        assert_eq!(whole, expected, "hour {}", hour.hour);
    }
    // An hour changing nothing the score reads hands the emit nothing.
    for quiet in [101, 102, 104, 106, 108, 112, 115, 117, 121, 123, 124, 126, 129, 130] {
        let hour = hours.iter().find(|hour| hour.hour == quiet).unwrap();
        assert!(hour.facts.is_none(), "hour {quiet}");
    }
}

/// The countries an hour hands the emit, none when it hands nothing.
fn handed(hour: &facts::Hour) -> Option<Vec<String>> {
    hour.facts.as_deref().map(|facts| match facts {
        facts::Facts::Whole { countries, .. } | facts::Facts::Countries { countries, .. } => {
            countries.iter().map(|(tag, _, _)| tag.clone()).collect()
        }
    })
}

#[test]
fn of_fifteen_hundred_countries_an_hour_hands_on_only_the_countries_its_rows_reach() {
    let count = 1500;
    let content = content();
    let first = row_score_frame(count);
    let mut frames = vec![first.clone()];
    let mut next = |change: &dyn Fn(&mut DecodedFrame)| {
        let mut frame = frames.last().unwrap().clone();
        frame.hour += 1;
        change(&mut frame);
        frames.push(frame);
    };
    let index = |part: u32| (count / part) as usize;
    next(&|_| {});
    next(&|frame| {
        rows::<tables::CountryGameScoreInputs>(frame)[index(2)].army_count = Some(40);
    });
    next(&|frame| rows::<tables::AirWingGameScoreStats>(frame)[1].planes = Some(11));
    next(&|frame| {
        rows::<tables::CountryFactories>(frame)[index(3)].military_used += 1;
        rows::<tables::CountryGameScoreInputs>(frame)[index(3)].fleets[0].fleet += 1;
        rows::<tables::AirWingGameScoreStats>(frame)[0].mission_index = Some(1);
        rows::<tables::Politics>(frame)[index(3)].last_election += 1;
        rows::<tables::Factions>(frame)[0].color_red += 1;
    });
    next(&|frame| {
        rows::<tables::CountryAppearance>(frame)[index(4)].red = 0.5_f32.to_bits();
    });
    next(&|frame| {
        let hour = i64::from(frame.hour);
        for row in rows::<tables::Countries>(frame) {
            row.political_power_raw = Some(hour);
        }
    });
    next(&|frame| {
        let members = rows::<tables::FactionMembers>(frame);
        members.push(FactionMemberRow {
            country: 100_007,
            faction: 1,
            ordinal: 5,
        });
        members.sort_by_key(|row| row.country);
    });
    let (hours, _) = projected(&content, &frames, &[]);
    let tag = |at: usize| {
        let mut frame = first.clone();
        String::from_utf8(rows::<tables::Countries>(&mut frame)[at].tag.clone()).unwrap()
    };
    assert_eq!(handed(&hours[0]).map(|tags| tags.len()), Some(count as usize));
    assert_eq!(handed(&hours[1]), None, "an unchanged hour hands nothing");
    assert_eq!(handed(&hours[2]), Some(vec![tag(index(2))]), "an army change");
    // The second wing is flown from the base of the eleventh country's slot.
    assert_eq!(handed(&hours[3]), Some(vec![tag(10)]), "a wing's planes");
    assert_eq!(handed(&hours[4]), None, "fields no score part reads");
    assert_eq!(handed(&hours[5]), Some(vec![tag(index(4))]), "a colour change");
    assert_eq!(handed(&hours[6]), None, "every country's balance changing");
    assert_eq!(handed(&hours[7]), Some(vec![tag(7)]), "joining a faction");
}
