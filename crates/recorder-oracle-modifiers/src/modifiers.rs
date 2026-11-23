
use super::{Report, TokenNames, complete, invalid, required_country_tag};
use content_reader_parser::parser::c_string;
use content_reader_save::characters as saved;
use replay_data::{rows::CountryRow, rows::HiredAdvisorRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub fn compare(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: io::Result<&TokenNames<'_>>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report.attempt_table(TableId::ModifierDefinitions, |report| {
        definitions(frame, tokens?, report)
    });
    complete_comparison &= report.attempt_table(TableId::HiredAdvisors, |report| {
        hired_advisors(frame, pack, links, report)
    });
    if complete_comparison {
        Ok(())
    } else {
        Err(invalid(
            "modifier definition or hired advisor comparison incomplete; see per-table errors",
        ))
    }
}

fn definitions(
    frame: &DecodedFrame,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::ModifierDefinitions>(frame)?;
    if rows.iter().any(|row| row.token == 0) {
        return Err(invalid(
            "ModifierDefinitions has a row with token 0, which Register never writes",
        ));
    }
    let unnamed = rows
        .iter()
        .filter(|row| !tokens.contains_key(&row.token))
        .count();
    if unnamed != 0 {
        report.limitation(
            "modifier script-name tokens absent from the recorded lexical dictionary cannot be named",
            format!("ModifierDefinitions: {unnamed} rows"),
        );
    }
    if rows
        .windows(2)
        .any(|pair| pair[0].eligibility != pair[1].eligibility)
    {
        return Err(invalid(
            "ModifierDefinitions: the eligibility callback state differs between rows",
        ));
    }
    report.compared("ModifierDefinitions", 0);
    report.limitation(
        "modifier definitions are engine registration state that no save writer serializes, so this table has no save comparison; the recorded rows decode but are not certified against a save",
        format!("ModifierDefinitions: {} decoded rows, zero save comparisons", rows.len()),
    );
    Ok(())
}

fn character_handle(id: u32) -> u64 {
    (u64::from(id) << 32) | 73
}

fn hired_advisors(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    if !report.refresh_is_current(TableId::HiredAdvisors, frame.hour) {
        report.limitation(
            &format!("HiredAdvisors {}", replay_data::cadence::STAGGER_LIMITATION),
            format!(
                "HiredAdvisors refreshes on {} (oracle hour {})",
                replay_data::cadence::refresh_text(TableId::HiredAdvisors),
                frame.hour
            ),
        );
        return Ok(());
    }
    let mut recorded = BTreeMap::<&[u8], (u32, Vec<&HiredAdvisorRow>)>::new();
    for row in complete::<tables::HiredAdvisors>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        let (raw, list) = recorded.entry(tag).or_insert((row.country, Vec::new()));
        if *raw != row.country {
            return Err(invalid(format!(
                "HiredAdvisors records two country references for {}",
                String::from_utf8_lossy(tag)
            )));
        }
        if row.ordinal as usize != list.len() {
            return Err(invalid(
                "HiredAdvisors ordinals are not contiguous from zero within a country",
            ));
        }
        list.push(row);
    }
    let roles = pack.manager.as_ref();
    let mut compared = 0;
    let mut unjoined = Vec::new();
    let mut truncated_text = Vec::new();
    for (tag, expected) in &pack.appointed_advisors {
        let actual = recorded
            .remove(tag.as_slice())
            .map(|(_, list)| list)
            .unwrap_or_default();
        let label = format!("HiredAdvisors {}", String::from_utf8_lossy(tag));
        if actual.len() != expected.len() {
            report.difference(format!("{label} count"), actual.len(), expected.len());
        }
        for (ordinal, (row, saved)) in actual.iter().zip(expected).enumerate() {
            compared += 1;
            let label = format!("{label}[{ordinal}]");
            if character_handle(row.character) != saved.character {
                report.difference(
                    format!("{label}.character"),
                    row.character,
                    saved.character >> 32,
                );
            }
            if c_string(&row.slot) != saved.slot.as_slice() {
                report.difference(
                    format!("{label}.slot"),
                    String::from_utf8_lossy(c_string(&row.slot)),
                    String::from_utf8_lossy(&saved.slot),
                );
            }
            if c_string(&row.slot).len() != row.slot.len() {
                truncated_text.push(format!("{label}.slot"));
            }
            let role = roles
                .and_then(|manager| manager.characters.get(&saved.character))
                .map(|character| {
                    character
                        .advisors
                        .iter()
                        .filter(|role| role.slot.as_deref().unwrap_or_default() == saved.slot)
                        .collect::<Vec<_>>()
                });
            match role.as_deref() {
                Some([role]) => {
                    let written = c_string(&row.idea);
                    let matches = match role.idea.as_deref() {
                        None => written.is_empty(),
                        Some(saved) => !written.is_empty() && written == saved,
                    };
                    if !matches {
                        report.difference(
                            format!("{label}.idea"),
                            (!written.is_empty())
                                .then(|| String::from_utf8_lossy(written).into_owned()),
                            role.idea
                                .as_deref()
                                .map(|saved| String::from_utf8_lossy(saved).into_owned()),
                        );
                    }
                    if c_string(&row.idea).len() != row.idea.len() {
                        truncated_text.push(format!("{label}.idea"));
                    }
                }
                _ => unjoined.push(format!("{label}.idea")),
            }
        }
        for (ordinal, extra) in actual.iter().enumerate().skip(expected.len()) {
            report.difference(format!("{label}[{ordinal}]"), extra.character, "absent");
        }
        for (ordinal, missing) in expected.iter().enumerate().skip(actual.len()) {
            report.difference(
                format!("{label}[{ordinal}]"),
                "absent",
                missing.character >> 32,
            );
        }
    }
    for (tag, (_, list)) in recorded {
        report.difference(
            format!("HiredAdvisors {}", String::from_utf8_lossy(tag)),
            list.len(),
            "country absent from save",
        );
    }
    if !unjoined.is_empty() {
        report.limitation(
            "the save writes no idea token for an appointed advisor, and no single saved advisor role of that character has the appointed slot, so its idea token is unverified",
            unjoined.join(", "),
        );
    }
    if !truncated_text.is_empty() {
        report.limitation(
            "text serialization stops at the first NUL; captured suffix bytes are not comparable",
            truncated_text.join(", "),
        );
    }
    report.limitation(
        "the idea token of an appointed advisor is compared with the saved role of the same character and slot, not with a saved field of the appointed entry",
        "HiredAdvisors.idea",
    );
    report.compared("HiredAdvisors", compared);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::ModifierDefinitionRow, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    fn pack() -> saved::Pack {
        saved::parse(
            &content_reader_parser::parser::parse(
                br#"character_manager={ historical={
                    character={ id={ id=1482 type=73 } token="a" country="GER"
                        advisors={ advisor={ slot="political_advisor" idea_token="GER_a" }
                                   advisor={ slot="theorist" idea_token="GER_a_theorist" } } }
                    character={ id={ id=1451 type=73 } token="b" country="GER"
                        advisors={ advisor={ slot="high_command" idea_token="GER_b" } } }
                }} countries={
                    GER={ characters={ appointed_advisors={
                        { slot="political_advisor" character={ id=1482 type=73 } }
                        { slot="high_command" character={ id=1451 type=73 } }
                    } } }
                    ENG={ }
                }"#,
            )
            .unwrap()
            .items,
        )
        .unwrap()
    }

    fn row(
        country: u32,
        ordinal: u32,
        character: u32,
        idea: &[u8],
        slot: &[u8],
    ) -> HiredAdvisorRow {
        HiredAdvisorRow {
            country,
            ordinal,
            character,
            idea: idea.to_vec(),
            slot: slot.to_vec(),
        }
    }

    fn frame(rows: Vec<HiredAdvisorRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 21,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::HiredAdvisors.index()] =
            TableView::Complete(TableRows::HiredAdvisors(rows));
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                ModifierDefinitionRow {
                    index: 0x8d,
                    token: 5,
                    ..Default::default()
                },
            ]));
        frame
    }

    fn run(rows: Vec<HiredAdvisorRow>) -> Report {
        let germany = CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(102, &germany)]);
        let tokens = BTreeMap::from([(5, b"stability_factor".as_slice())]);
        let mut report = Report::default();
        let _ = compare(&frame(rows), &pack(), &links, Ok(&tokens), &mut report);
        report
    }

    #[test]
    fn matching_appointments_compare_order_character_slot_and_joined_idea() {
        let report = run(vec![
            row(102, 0, 1482, b"GER_a", b"political_advisor"),
            row(102, 1, 1451, b"GER_b", b"high_command"),
        ]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.compared_tables["HiredAdvisors"], 2);
        assert_eq!(report.compared_tables["ModifierDefinitions"], 0);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no save writer serializes"))
        );
    }

    #[test]
    fn appointments_compare_without_the_lexical_dictionary() {
        let germany = CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(102, &germany)]);
        let rows = frame(vec![row(102, 0, 1451, b"GER_b", b"high_command")]);
        let mut report = Report::default();
        let missing = Err(io::Error::other("LexicalTokens truncated"));
        let _ = compare(&rows, &pack(), &links, missing, &mut report);
        assert!(
            report
                .failed_comparisons
                .contains_key("ModifierDefinitions")
        );
        assert_eq!(report.compared_tables["HiredAdvisors"], 1);
        assert!(!report.differences.is_empty());
    }

    #[test]
    fn order_identity_slot_idea_and_membership_differences_are_reported() {
        let report = run(vec![
            row(102, 0, 1451, b"GER_b", b"high_command"),
            row(102, 1, 1482, b"GER_a", b"political_advisor"),
        ]);
        assert_eq!(report.differences.len(), 6, "{:?}", report.differences);
        let report = run(vec![row(102, 0, 1482, b"GER_x", b"political_advisor")]);
        assert!(report.differences.iter().any(|d| d.contains(".idea")));
        assert!(report.differences.iter().any(|d| d.contains("count")));
        assert!(report.differences.iter().any(|d| d.contains("[1]")));
        let report = run(vec![
            row(102, 0, 1482, b"", b"political_advisor"),
            row(102, 1, 1451, b"GER_b", b"high_command"),
        ]);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("[0].idea"));
        let report = run(vec![row(102, 1, 1451, b"GER_b", b"high_command")]);
        assert!(report.failed_comparisons.contains_key("HiredAdvisors"));
    }

    #[test]
    fn appointments_are_compared_at_every_hour() {
        assert_eq!(
            replay_data::cadence::cadence(TableId::HiredAdvisors),
            replay_data::cadence::Cadence::Hourly
        );
        let mut rows = frame(Vec::new());
        rows.hour += 1;
        let germany = CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(102, &germany)]);
        let mut report = Report::default();
        compare(&rows, &pack(), &links, Ok(&BTreeMap::new()), &mut report).unwrap();
        assert!(report.compared_tables.contains_key("HiredAdvisors"));
        assert!(!report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("absent from the recorded lexical dictionary"))
        );
    }
}
