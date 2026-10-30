//! Country styles prepared on their own (`products::style_runs`): before preparation every hour
//! is left to a seek, and once prepared every hour of the branch answers exactly as its seek
//! reads it, names and colours alike, across a tag change, a new country, a subject, a changed
//! ruling party and colour, an incomplete table, a second block and a gap; and the branch's
//! facts, as runs, hold exactly those hours and styles.
use super::*;
use crate::{
    products::style_runs::{PREPARED_STYLES, country_facts},
    test_fixtures::{continuation_records, world_frame},
};
use dashboard_base::products::country_styles::{HourStyles, StyleRuns};
use installed_content_country_names::country_names::TABLES as STYLE_TABLES;
use installed_content_localisation::localisation::Localisation;
use replay::container::SeekResult;
use replay_data::rows::{
    AutonomyRow, CountryAppearanceRow, PoliticalPartyRow, PoliticsRow, ReservedCountryNamingRow,
};

/// GER (raw 9001, slot 7), ITA (raw 21, slot 8) and, from hour 24, D01 (raw 31, slot 9), which
/// becomes GER's puppet at 26; ITA is RSI from 28, GER's colour changes at 16 and its ruling
/// party at 18 and back at 34 (renaming its puppet through its adjective), the reserved names
/// built change at 36, and the appearance table is incomplete at 20.
fn styled(hour: i32) -> DecodedFrame {
    let mut frame = world_frame(hour, hour >= 24);
    if hour >= 28
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows.iter_mut().filter(|row| row.id == 21) {
            row.tag = b"RSI".to_vec();
        }
    }
    let appearance = |country, red: f32, overlord| CountryAppearanceRow {
        country,
        red: red.to_bits(),
        green: 0.5_f32.to_bits(),
        blue: 0.25_f32.to_bits(),
        leader_ideology: b"despotism".to_vec(),
        overlord,
        ..Default::default()
    };
    let mut appearances = vec![appearance(21, 0.1, 0)];
    if hour >= 24 {
        appearances.push(appearance(31, 0.3, if hour >= 26 { 9001 } else { 0 }));
    }
    appearances.push(appearance(9001, if hour < 16 { 0.2 } else { 0.9 }, 0));
    let appearances = TableRows::CountryAppearance(appearances);
    *frame.tables[TableId::CountryAppearance.index()] = if hour == 20 {
        TableView::Truncated {
            rows: appearances,
            missing: 1,
        }
    } else {
        TableView::Complete(appearances)
    };
    let politics = |country, party| PoliticsRow {
        country,
        ruling_party: Some(party),
        ..Default::default()
    };
    let mut politics_rows = vec![politics(21, 1)];
    if hour >= 24 {
        politics_rows.push(politics(31, 1));
    }
    politics_rows.push(politics(9001, if hour < 18 || hour >= 34 { 1 } else { 2 }));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(politics_rows));
    let party = |country, id, token: &[u8]| PoliticalPartyRow {
        country,
        id,
        token: token.to_vec(),
        ..Default::default()
    };
    let mut parties = vec![party(21, 1, b"fascism")];
    if hour >= 24 {
        parties.push(party(31, 1, b"neutrality"));
    }
    parties.extend([party(9001, 1, b"fascism"), party(9001, 2, b"democratic")]);
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(parties));
    let autonomy = if hour >= 26 {
        vec![AutonomyRow {
            subject: 31,
            score: 0,
            lower_score: 0,
            upper_score: 0,
            last_modified_bits: 0,
            state: b"autonomy_puppet".to_vec(),
            previous: None,
            next: None,
        }]
    } else {
        Vec::new()
    };
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(autonomy));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZZZ".to_vec(),
                built: if hour < 36 { 7 } else { 8 },
                ..Default::default()
            },
        ]));
    frame
}

#[test]
fn prepared_country_styles_answer_every_hour_as_its_seek_does() {
    assert!(independent::registered("country-styles"));
    assert!(!independent::hourly("country-styles"));
    super::super::install_gates();
    let first: Vec<_> = (10..=22).map(styled).collect();
    // A gap at 31 and 32 in the second block.
    let second: Vec<_> = (23..=40)
        .filter(|hour| !(31..=32).contains(hour))
        .map(styled)
        .collect();
    let opened = Replay::from_records(CAMPAIGN.into(), &[records(1, first.clone())]).unwrap();
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, first), continuation_records(&opened, 1, second)],
    )
    .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("country-styles");
    let flag = AtomicBool::new(false);
    let tags: Vec<String> = ["GER", "ITA", "RSI", "D01 mod", "ENG"]
        .map(str::to_owned)
        .into();
    // Each hour's inputs `PREPARED_STYLES` visits, or None when it leaves them to seeks.
    let read = |hours: &[i32], names: bool| {
        let mut visited = Vec::new();
        PREPARED_STYLES
            .inputs(
                &source,
                None,
                1,
                hours,
                &tags,
                names,
                &flag,
                &mut |hour, inputs| {
                    visited.push((hour, inputs));
                    Ok(())
                },
            )
            .unwrap()
            .then_some(visited)
    };
    assert_eq!(read(&[10], true), None, "nothing is prepared yet");
    run(&source, None, &flag, |_| {}).unwrap();
    let localisation: Localisation = [
        ("GER", "Germany"),
        ("GER_fascism", "German Reich"),
        ("GER_democratic", "German Republic"),
        ("GER_ADJ", "German"),
        ("GER_fascism_ADJ", "Reich"),
        ("GER_democratic_ADJ", "Federal"),
        ("ITA", "Italy"),
        ("RSI", "Italian Social Republic"),
        ("D01 mod", "Danubia"),
        (
            "D01 mod_GER_autonomy_puppet",
            "Danubia under $OVERLORDADJ$ rule",
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut prepared_hours = 0;
    for hour in 9..=41 {
        let seek =
            dashboard_base::prepared::query::frames::seek(&source, 1, hour, &STYLE_TABLES, &flag)
                .unwrap();
        for localisation in [Some(&localisation), None] {
            match (read(&[hour], localisation.is_some()), &seek) {
                (Some(mut prepared), SeekResult::Frame(frame)) => {
                    prepared_hours += 1;
                    let expected = HourStyles::read(frame, localisation).unwrap();
                    let (at, inputs) = prepared.pop().unwrap();
                    assert_eq!(at, hour);
                    let actual = HourStyles::build(inputs, localisation);
                    for tag in &tags {
                        assert_eq!(actual.style(tag), expected.style(tag), "{hour} {tag}");
                        assert_eq!(
                            actual.missing_reason(tag),
                            expected.missing_reason(tag),
                            "{hour} {tag}"
                        );
                    }
                }
                (None, SeekResult::Gap { .. }) => {}
                // Its truncated appearance table is named with its hour, so a seek says why.
                (None, SeekResult::Frame(_)) if hour == 20 => {}
                (prepared, seek) => panic!(
                    "hour {hour}: prepared {} where a seek gives {}",
                    prepared.is_some(),
                    matches!(seek, SeekResult::Frame(_))
                ),
            }
        }
    }
    // Every recorded hour but the truncated one, with names and without.
    assert_eq!(prepared_hours, 2 * (31 - 2 - 1));
    // Many hours at once, as `country-colours` asks and across read windows, are the hours read
    // alone.
    let hours: Vec<i32> = (21..=30).collect();
    let many = read(&hours, false).unwrap();
    assert_eq!(
        many.iter().map(|(hour, _)| *hour).collect::<Vec<_>>(),
        hours
    );
    for (hour, inputs) in many {
        assert_eq!(read(&[hour], false).unwrap(), [(hour, inputs)], "{hour}");
    }
    // A request reaching an hour preparation left to seeks, or a gap, is wholly a seek's.
    assert_eq!(read(&[19, 20, 21], false), None);
    assert_eq!(read(&[30, 31], false), None);
    // The whole branch's facts hold exactly the hours preparation answers, each tag's run in
    // force giving the style, and the reason, its hour gives.
    for localisation in [Some(&localisation), None] {
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let facts = country_facts(&source, (&store, &key), 1, [9, 41], localisation, &flag)
            .unwrap()
            .unwrap();
        let holds = |hour: i32| facts.known.iter().any(|&[first, last]| (first..=last).contains(&hour));
        for hour in 9..=41 {
            let Some(mut prepared) = read(&[hour], localisation.is_some()) else {
                assert!(!holds(hour), "{hour} is left to seeks");
                continue;
            };
            assert!(holds(hour), "{hour}");
            let expected = HourStyles::build(prepared.pop().unwrap().1, localisation);
            for tag in &tags {
                let Some(run) = facts
                    .countries
                    .iter()
                    .find(|country| &country.tag == tag)
                    .and_then(|country| country.runs.iter().rev().find(|run| run[0] <= hour))
                else {
                    // A tag no hour made a country has no runs: it is not one.
                    assert_eq!(expected.fact(tag).err().as_deref(), Some("not a country at this hour"), "{hour} {tag}");
                    continue;
                };
                let reason = (run[3] >= 0).then(|| facts.reasons[run[3] as usize].as_str());
                match expected.fact(tag) {
                    Ok((style, problem)) => {
                        assert_eq!(
                            (run[1] >= 0).then(|| facts.names[run[1] as usize].as_str()),
                            style.name.as_deref(),
                            "{hour} {tag}"
                        );
                        assert_eq!(
                            (run[2] >= 0).then(|| [(run[2] >> 16) as u8, (run[2] >> 8) as u8, run[2] as u8]),
                            style.color,
                            "{hour} {tag}"
                        );
                        assert_eq!(reason, problem, "{hour} {tag}");
                    }
                    Err(missing) => {
                        assert_eq!([run[1], run[2]], [-1, -1], "{hour} {tag}");
                        assert_eq!(reason, Some(missing.as_str()), "{hour} {tag}");
                    }
                }
            }
        }
        if localisation.is_some() {
            let d01 = facts.countries.iter().find(|country| country.tag == "D01 mod").unwrap();
            let named = |hour: i32| {
                let run = d01.runs.iter().rev().find(|run| run[0] <= hour).unwrap();
                facts.names[run[1] as usize].clone()
            };
            assert_ne!(named(33), named(35), "the overlord's new adjective renames its puppet");
        }
        // A run starts only where a tag's style changes.
        for country in &facts.countries {
            for pair in country.runs.windows(2) {
                assert_ne!(pair[0][1..], pair[1][1..], "{}", country.tag);
            }
        }
    }
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}
