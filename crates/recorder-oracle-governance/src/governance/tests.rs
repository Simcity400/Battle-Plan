use super::*;

mod research_identity;

#[test]
fn gap_research_grants_oracle_compares_counter_unused_uses_and_restrictions() {
    use replay_data::rows::ResearchModifierRow as R;
    let (mut frame, _, country) = fixture();
    let pack = saved::parse(&content_reader_parser::parser::parse(br"countries={AAA={technology={next_bonus_id=8 limited_use_bonus={id=7 name=mod_bonus uses=3 claim=1 bonus=0.25 category=mod_category}}}}").unwrap().items).unwrap();
    let mut rows = vec![
        R {
            kind: R::GRANT_COUNTER,
            raw: 8,
            ..Default::default()
        },
        R {
            kind: R::SPEED_GRANT,
            source_id: 7,
            name: b"mod_bonus".to_vec(),
            raw: 25_000,
            ..Default::default()
        },
        R {
            kind: R::GRANT_USES,
            source_id: 7,
            raw: 3,
            auxiliary: 1,
            ..Default::default()
        },
        R {
            kind: R::GRANT_CATEGORY,
            source_id: 7,
            raw: 42,
            ..Default::default()
        },
    ];
    for (ordinal, row) in rows.iter_mut().enumerate() {
        row.country = country.id;
        row.index = engine_research::research_modifiers::grants::INVENTORY_INDEX;
        row.ordinal = ordinal as u32;
    }
    let links = BTreeMap::from([(country.id, &country)]);
    let names = BTreeMap::from([
        (42, b"mod_category".as_slice()),
        (43, b"other_category".as_slice()),
    ]);
    for changed in [None, Some((0, 9)), Some((2, 2)), Some((3, 43))] {
        let mut values = rows.clone();
        if let Some((at, raw)) = changed {
            values[at].raw = raw;
        }
        *frame.tables[TableId::ResearchModifiers.index()] =
            TableView::Complete(TableRows::ResearchModifiers(values));
        let mut report = refreshed_report();
        compare_research(&frame, &pack, &links, &names, &mut report).unwrap();
        assert_eq!(
            report.differences.is_empty(),
            changed.is_none(),
            "{:?}",
            report.differences
        );
    }
    frame.hour = 1;
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![
        replay_data::rows::SliceReadRow {
            table: TableId::ResearchModifiers.index() as u32,
            owner: country.id,
            sample_hour: 0,
        },
    ]));
    let mut report = refreshed_report();
    report.read_hours = Some(BTreeMap::new());
    compare_research(&frame, &pack, &links, &names, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .carried
            .keys()
            .any(|key| key.starts_with("ResearchModifiers"))
    );
}

fn refreshed_report() -> Report {
    let mut report = Report::default();
    report.read_now.insert(TableId::ResearchSlots);
    report
}
use replay_data::{
    rows::ActiveIdeaRow, rows::CompletedFocusRow, rows::CountryTacticUnlockRow,
    rows::DecisionCountsRow, rows::DecisionRow, rows::DynamicModifierRow, rows::LexicalTokenRow,
    rows::PreferredTacticRow, rows::ResearchSlotRow, rows::ResearchTechnologyRow, rows::TableId,
    rows::TableRows, rows::TechSharingGroupRow, rows::TechSharingMemberRow,
    rows::TechnologyDefinitionRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn compare_sharing_fixture(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let tokens = super::super::lexical_tokens(frame, report)?;
    compare_sharing_groups(frame, pack, links, &tokens, report)?;
    compare_sharing(frame, pack, links, &tokens, report)
}

pub(super) fn fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
    let country = CountryRow {
        border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
        id: 11,
        canonical_slot: 7,
        tag: b"AAA".to_vec(),
        political_power_raw: None,
        command_power_raw: 0,
        stability_raw: 0,
        war_support_raw: 0,
        convoys_raw: None,
        peace_conference_participant: 0,
        manpower_residual: 0,
    };
    let frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    let pack = saved::Pack {
        countries: BTreeMap::from([(b"AAA".to_vec(), saved::Country::default())]),
        ..Default::default()
    };
    (frame, pack, country)
}

fn active_ideas_report(actual: &[&[u8]], expected: &[&[u8]], tokens: &[&[u8]]) -> Report {
    let (mut frame, mut pack, country) = fixture();
    *frame.tables[TableId::ActiveIdeas.index()] = TableView::Complete(TableRows::ActiveIdeas(
        actual
            .iter()
            .enumerate()
            .map(|(ordinal, token)| ActiveIdeaRow {
                country: country.id,
                ordinal: ordinal as u32,
                token: token.to_vec(),
            })
            .collect(),
    ));
    pack.countries.get_mut(b"AAA".as_slice()).unwrap().politics = Some(saved::Politics {
        parties: BTreeMap::new(),
        leaders: BTreeMap::new(),
        names: BTreeMap::new(),
        ideas: expected.iter().map(|token| token.to_vec()).collect(),
        timed_ideas: Vec::new(),
        ruling_party: None,
        last_election: None,
        election_frequency: None,
        elections_allowed: None,
    });
    let tokens = tokens
        .iter()
        .enumerate()
        .map(|(id, &token)| (id as u32, token))
        .collect();
    let mut report = Report::default();
    let _ = compare_pack(
        &frame,
        &pack,
        &BTreeMap::from([(country.id, &country)]),
        &tokens,
        &mut report,
    );
    report
}

#[test]
fn active_ideas_use_recorded_canonical_spelling_across_mod_files() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "oracle_idea_case_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let roots = [root.join("base"), root.join("mod")];
    for (directory, file, input) in [
        (
            &roots[0],
            "common/ideas/first.txt",
            "ideas={country={MiXeD={}}}",
        ),
        (
            &roots[1],
            "common/ideas/second.txt",
            "ideas={country={mixed={} Undead={}}}",
        ),
    ] {
        let file = directory.join(file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, input).unwrap();
    }
    let layers =
        content_reader_layers::ResolvedLayers::from_folders(&[(&roots[0], &[]), (&roots[1], &[])])
            .unwrap();
    let (ideas, _, diagnostics) = content_reader_politics::ideas::read(&layers, &[]).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let (tokens, gap) = ideas.tokens();
    assert!(gap.is_none());
    let authored = tokens.map(str::as_bytes).collect::<Vec<_>>();
    assert_eq!(authored, [b"mixed".as_slice(), b"Undead"]);
    let report = active_ideas_report(&authored, &[b"MiXeD", b"undead"], &[b"MiXeD", b"undead"]);
    assert!(
        !report
            .failed_comparisons
            .contains_key("governance ActiveIdeas")
    );
    assert!(
        !report
            .differences
            .iter()
            .any(|difference| difference.contains("ideas.ordered_tokens")),
        "{:?}",
        report.differences
    );
    assert_eq!(report.compared_tables["ActiveIdeas"], 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn active_ideas_require_exact_writer_spelling_and_order() {
    for saved in [
        &[b"mixed".as_slice(), b"undead"][..],
        &[b"undead".as_slice(), b"MiXeD"][..],
    ] {
        let report = active_ideas_report(&[b"mixed", b"Undead"], saved, &[b"MiXeD", b"undead"]);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("ideas.ordered_tokens"))
        );
    }
    let report = active_ideas_report(&["É".as_bytes()], &["é".as_bytes()], &["É".as_bytes()]);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("ideas.ordered_tokens"))
    );
}

#[test]
fn active_ideas_preserve_identical_reverse_aliases_and_native_rejected_tokens() {
    for (actual, expected, tokens) in [
        (
            b"mixed".as_slice(),
            b"MiXeD".as_slice(),
            &[b"MiXeD".as_slice(), b"MiXeD"][..],
        ),
        (b"7mod".as_slice(), b"".as_slice(), &[b"".as_slice()][..]),
    ] {
        let report = active_ideas_report(&[actual], &[expected], tokens);
        assert!(
            !report
                .failed_comparisons
                .contains_key("governance ActiveIdeas")
        );
        assert!(
            !report
                .differences
                .iter()
                .any(|difference| difference.contains("ideas.ordered_tokens")),
            "{:?}",
            report.differences
        );
    }
}

#[test]
fn active_ideas_refuse_missing_and_ambiguous_recorded_spellings() {
    for tokens in [
        &[][..],
        &[b"other".as_slice()][..],
        &[b"MiXeD".as_slice(), b"mixed"][..],
    ] {
        let report = active_ideas_report(&[b"mixed"], &[b"MiXeD"], tokens);
        assert!(
            report
                .failed_comparisons
                .contains_key("governance ActiveIdeas"),
            "{:?}",
            report.failed_comparisons
        );
        assert!(!report.compared_tables.contains_key("ActiveIdeas"));
    }
}

#[test]
fn active_ideas_compare_lexer_case_identity_without_losing_order_or_other_bytes() {
    let (mut frame, _, country) = fixture();
    let parsed =
        content_reader_parser::parser::parse(b"countries={AAA={politics={ideas={MiXeD other}}}}").unwrap();
    let pack = saved::parse(&parsed.items).unwrap();
    let links = BTreeMap::from([(country.id, &country)]);
    let idea = |ordinal, token: &[u8]| ActiveIdeaRow {
        country: country.id,
        ordinal,
        token: token.to_vec(),
    };
    for (tokens, differences) in [
        ([b"mixed".as_slice(), b"other"], 0),
        ([b"OTHER".as_slice(), b"mixed"], 1),
        ([b"mix_d".as_slice(), b"other"], 1),
        ([b"mixed".as_slice(), b"oth\x80r"], 1),
    ] {
        *frame.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(vec![
                idea(0, tokens[0]),
                idea(1, tokens[1]),
            ]));
        let mut report = Report::default();
        let spellings = [b"MiXeD".as_slice(), b"other", b"mix_d", b"oth\x80r"]
            .into_iter()
            .enumerate()
            .map(|(id, token)| (id as u32, token))
            .collect();
        compare_ideas(&frame, &pack, &links, &spellings, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
}

#[test]
fn format99_rosters_compare_order_and_exact_values() {
    use replay_data::{rows::DecisionRandomItemRow, rows::TimedIdeaRow};
    let (mut frame, _, _) = fixture();
    let mut input = String::from("countries={");
    let mut countries = Vec::new();
    let mut ideas = Vec::new();
    let mut random = Vec::new();
    for id in 1..=3 {
        input.push_str(&format!("T{id}={{politics={{timed_idea={{idea=repeat days=-1}} timed_idea={{idea=repeat days=0}}}} decision_status={{random_item={{decision=inactive count=-2147483648 target=-1}} random_item={{decision=inactive count=0 target=7}}}}}}"));
        countries.push(CountryRow {
            id,
            tag: format!("T{id}").into_bytes(),
            ..Default::default()
        });
        for (ordinal, days) in [(0, -1), (1, 0)] {
            ideas.push(TimedIdeaRow {
                country: id,
                ordinal,
                token: b"repeat".to_vec(),
                days,
            });
        }
        for (ordinal, count, target) in [(0, i32::MIN, u32::MAX), (1, 0, 7)] {
            random.push(DecisionRandomItemRow {
                country: id,
                ordinal,
                token: b"inactive".to_vec(),
                count,
                target,
            });
        }
    }
    input.push('}');
    let pack = saved::parse(
        &content_reader_parser::parser::parse(input.as_bytes())
            .unwrap()
            .items,
    )
    .unwrap();
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    *frame.tables[TableId::TimedIdeas.index()] = TableView::Complete(TableRows::TimedIdeas(ideas));
    *frame.tables[TableId::DecisionRandomItems.index()] =
        TableView::Complete(TableRows::DecisionRandomItems(random));
    let mut report = Report::default();
    compare_timed_ideas(&frame, &pack, &links, &mut report).unwrap();
    compare_decision_random_items(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["TimedIdeas"], 6);
    assert_eq!(report.compared_tables["DecisionRandomItems"], 6);
    let TableView::Complete(TableRows::TimedIdeas(ideas)) =
        &mut *frame.tables[TableId::TimedIdeas.index()]
    else {
        unreachable!()
    };
    ideas[0].days = 0;
    let TableView::Complete(TableRows::DecisionRandomItems(random)) =
        &mut *frame.tables[TableId::DecisionRandomItems.index()]
    else {
        unreachable!()
    };
    random[0].target = 7;
    random[1].count = 1;
    let mut report = Report::default();
    compare_timed_ideas(&frame, &pack, &links, &mut report).unwrap();
    compare_decision_random_items(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    let TableView::Complete(TableRows::DecisionRandomItems(random)) =
        &mut *frame.tables[TableId::DecisionRandomItems.index()]
    else {
        unreachable!()
    };
    random[0].ordinal = 3;
    assert!(compare_decision_random_items(&frame, &pack, &links, &mut Report::default()).is_err());
}

#[test]
fn tactic_unlocks_cover_the_canonical_census() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let pack = |at_seven: saved::DoctrineStatus, stray: Vec<i32>| saved::Pack {
        doctrine: Some(
            (0..8)
                .map(|slot| match slot {
                    7 => at_seven.clone(),
                    3 => saved::DoctrineStatus {
                        country: Some(b"BBB".to_vec()),
                        enable_tactic: stray.clone(),
                        ..Default::default()
                    },
                    _ => saved::DoctrineStatus::default(),
                })
                .collect(),
        ),
        ..saved::Pack::default()
    };
    let aaa = |ids: Vec<i32>| saved::DoctrineStatus {
        country: Some(b"AAA".to_vec()),
        enable_tactic: ids,
        ..Default::default()
    };
    let mut compare = |rows: Vec<CountryTacticUnlockRow>, pack: &saved::Pack| {
        *frame.tables[TableId::CountryTacticUnlocks.index()] =
            TableView::Complete(TableRows::CountryTacticUnlocks(rows));
        let mut report = Report::default();
        let result = compare_tactic_unlocks(&frame, pack, &links, &mut report);
        (result.is_ok(), report)
    };
    let row = |ids: Vec<u32>| CountryTacticUnlockRow {
        country: 11,
        unlocked_tactic_ids: ids,
    };
    let (ok, report) = compare(vec![row(vec![4, 2])], &pack(aaa(vec![4, 2]), Vec::new()));
    assert!(ok);
    assert_eq!(report.compared_tables.get("CountryTacticUnlocks"), Some(&1));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("7 empty statuses of slots outside the census"))
    );
    let (_, report) = compare(vec![row(vec![2, 4])], &pack(aaa(vec![4, 2]), Vec::new()));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("AAA.enable_tactic"))
    );
    let (_, report) = compare(Vec::new(), &pack(aaa(Vec::new()), Vec::new()));
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("no captured row for a census country"));
    let (_, report) = compare(
        vec![row(Vec::new())],
        &pack(saved::DoctrineStatus::default(), Vec::new()),
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("AAA.doctrine status country"))
    );
    let (_, report) = compare(vec![row(vec![4, 2])], &pack(aaa(vec![4, 2]), vec![9]));
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("doctrine status 3.enable_tactic"));
    let (ok, _) = compare(vec![row(vec![4, 2])], &saved::Pack::default());
    assert!(!ok);
}

#[test]
fn unavailable_siblings_retain_independent_comparisons_and_aggregate_failure() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let document = content_reader_parser::parser::parse(
        br#"countries={AAA={
        politics={ideas={idea}} focus={completed=focus}
        preferred_tactic=7 technology={technologies={technology={research_points=0.00002}}}
    }}"#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    *frame.tables[TableId::ActiveIdeas.index()] =
        TableView::Complete(TableRows::ActiveIdeas(vec![ActiveIdeaRow {
            country: 11,
            ordinal: 0,
            token: b"different_idea".to_vec(),
        }]));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 11,
            ordinal: 0,
            token: b"focus".to_vec(),
        }]));
    *frame.tables[TableId::PreferredTactics.index()] =
        TableView::Complete(TableRows::PreferredTactics(vec![PreferredTacticRow {
            country: 11,
            tactic_id: 8,
            token: Vec::new(),
        }]));
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(vec![
            TechnologyDefinitionRow {
                database_index: 5,
                technology_handle: 5,
                id: b"technology".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                database_index: 5,
                country: 11,
                technology_handle: 5,
                progress_raw: 1,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![
        replay_data::rows::SliceReadRow {
            table: TableId::ResearchTechnologies.index() as u32,
            owner: 11,
            sample_hour: 0,
        },
    ]));
    for table in [
        TableId::PoliticalParties,
        TableId::Focus,
        TableId::ResearchSlots,
        TableId::DecisionCounts,
        TableId::DynamicModifiers,
        TableId::TechSharingMembers,
        TableId::PowerBalanceMembers,
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "injected sibling fault".into(),
        };
    }
    let mut report = refreshed_report();
    let tokens = BTreeMap::from([
        (1, b"different_idea".as_slice()),
        (5, b"technology".as_slice()),
    ]);
    assert!(compare_pack(&frame, &pack, &links, &tokens, &mut report).is_err());
    for table in [
        "ActiveIdeas",
        "CompletedFocuses",
        "PreferredTactics",
        "ResearchTechnologies",
    ] {
        assert_eq!(report.compared_tables.get(table), Some(&1), "{table}");
    }
    for table in [
        "Decisions",
        "TechSharingGroups",
        "PowerBalanceOverrides",
        "PowerBalanceRanges",
    ] {
        assert!(report.compared_tables.contains_key(table), "{table}");
    }
    for subject in [
        "ideas.ordered_tokens",
        "preferred_tactic.id",
        "technology.technology[0].research_points",
    ] {
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(subject)),
            "{subject}"
        );
    }
    assert!(
        report
            .failed_comparisons
            .contains_key("governance PoliticalParties")
    );
    assert!(
        report
            .failed_comparisons
            .contains_key("governance ResearchSlots")
    );
    assert!(
        report
            .failed_comparisons
            .contains_key("governance PowerBalanceMembers")
    );
}

#[test]
fn party_names_and_tags_compare_by_the_writer_default_rule() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let document = content_reader_parser::parser::parse(
        br#"countries={AAA={politics={parties={
        communism={default=no name="AAA_red" long_name="AAA_red_long" popularity=1}
        democratic={default=yes popularity=1}
        fascism={default=no name="AAA_black" long_name="" popularity=1}
    }}}}"#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    let party = |id, token: &[u8], tag, name_default, name: &[u8], long_name: &[u8]| {
        replay_data::rows::PoliticalPartyRow {
            country: 11,
            id,
            popularity: 100_000,
            token: token.to_vec(),
            tag,
            name_default,
            name: name.to_vec(),
            long_name: long_name.to_vec(),
            ..Default::default()
        }
    };
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![
            party(1, b"communism", 11, 0, b"AAA_red", b"AAA_red_long"),
            party(2, b"democratic", 11, 1, b"stale", b""),
            party(3, b"fascism", 7, 1, b"AAA_black", b"x"),
        ]));
    let mut report = Report::default();
    compare_parties(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.compared_tables.get("PoliticalParties"), Some(&3));
    let differences = report.differences.join(
        "
",
    );
    for subject in ["fascism.tag", "fascism.default", "fascism.long_name"] {
        assert!(differences.contains(subject), "{subject}: {differences}");
    }
    assert!(!differences.contains("communism"), "{differences}");
    assert!(!differences.contains("democratic"), "{differences}");
    assert!(!differences.contains("fascism.name"), "{differences}");
}

#[test]
fn idle_research_slot_is_not_equivalent_to_a_new_assignment() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let document = content_reader_parser::parser::parse(
        br#"countries={AAA={technology={
        slots={selected_technology={points_factor=1.35}}
    }}}"#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    let dictionary = BTreeMap::from([(5, (b"selected_technology".as_slice(), 1, 5))]);
    *frame.tables[TableId::ResearchSlots.index()] =
        TableView::Complete(TableRows::ResearchSlots(vec![ResearchSlotRow {
            database_index: replay_data::rows::Recorded::Value(
                (engine_research::cresearch_slot::IDLE_HANDLE) as i32,
            ),
            country: 11,
            index: 0,
            technology_handle: engine_research::cresearch_slot::IDLE_HANDLE,
            technology_present: 0,
            research_speed_raw: 100_000,
            banked_days_raw: 0,
            applied_days_raw: 0,
        }]));
    let mut report = Report::default();
    compare_research_slots(&frame, &pack, &links, &dictionary, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    for field in [
        "country: 11",
        "index: 0",
        "technology_handle: 10830",
        "technology_present: 0",
        "research_speed_raw: 100000",
        "banked_days_raw: 0",
        "applied_days_raw: 0",
        "points: None",
        "used_saved_points: None",
        "points_factor: Some(Decimal { raw: 135000",
    ] {
        assert!(report.differences[0].contains(field), "{field}");
    }
    let TableView::Complete(TableRows::ResearchSlots(rows)) =
        &mut *frame.tables[TableId::ResearchSlots.index()]
    else {
        unreachable!()
    };
    rows[0].technology_handle = 5;
    rows[0].database_index = replay_data::rows::Recorded::Value(5);
    rows[0].technology_present = 1;
    rows[0].research_speed_raw = 135_000;
    let mut report = Report::default();
    compare_research_slots(&frame, &pack, &links, &dictionary, &mut report).unwrap();
    assert!(report.differences.is_empty());
}

#[test]
fn shared_idle_handle_matches_saved_empty_without_a_technology_definition() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let document = content_reader_parser::parser::parse(
        br#"countries={AAA={technology={slots={empty={points=3}}}}}"#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    *frame.tables[TableId::ResearchSlots.index()] =
        TableView::Complete(TableRows::ResearchSlots(vec![ResearchSlotRow {
            database_index: replay_data::rows::Recorded::Value(
                (engine_research::cresearch_slot::IDLE_HANDLE) as i32,
            ),
            country: 11,
            technology_handle: engine_research::cresearch_slot::IDLE_HANDLE,
            research_speed_raw: engine_research::cresearch_slot::RESEARCH_SPEED_SCALE,
            banked_days_raw: 300_000,
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare_research_slots(&frame, &pack, &links, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables.get("ResearchSlots"), Some(&1));
}

fn sharing_fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
    let (mut frame, mut pack, country) = fixture();
    pack.tech_sharing = [b"first".as_slice(), b"second"]
        .into_iter()
        .enumerate()
        .map(|(index, token)| saved::TechSharing {
            token: token.to_vec(),
            countries: vec![b"AAA".to_vec()],
            bonuses: vec![
                content_reader_save::Decimal::parse(if index == 0 { b"0.00001" } else { b"0.00002" })
                    .unwrap(),
            ],
        })
        .collect();
    *frame.tables[TableId::TechSharingGroups.index()] =
        TableView::Complete(TableRows::TechSharingGroups(vec![
            TechSharingGroupRow {
                id: 0x10001,
                member_count: 1,
                base_bonus: -7,
            },
            TechSharingGroupRow {
                id: 0x20001,
                member_count: 1,
                base_bonus: 8,
            },
        ]));
    *frame.tables[TableId::TechSharingMembers.index()] =
        TableView::Complete(TableRows::TechSharingMembers(vec![
            TechSharingMemberRow {
                group: 0x10001,
                ordinal: 0,
                country: 11,
                bonus: 1,
            },
            TechSharingMemberRow {
                group: 0x20001,
                ordinal: 0,
                country: 11,
                bonus: 2,
            },
        ]));
    (frame, pack, country)
}

#[test]
fn sharing_complete_names_preserve_order_exact_bonuses_and_empty_groups() {
    let (mut frame, _, country) = fixture();
    let second = CountryRow {
        id: 12,
        canonical_slot: 8,
        tag: b"BBB".to_vec(),
        ..country
    };
    let links = BTreeMap::from([(11, &country), (12, &second)]);
    let document = content_reader_parser::parser::parse(
        br#"countries={AAA={} BBB={}}
        tech_sharing_group={id=first countries={AAA BBB} bonuses={-21474.83649 0}}
        tech_sharing_group={id=second}
    "#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    *frame.tables[TableId::TechSharingGroups.index()] =
        TableView::Complete(TableRows::TechSharingGroups(vec![
            TechSharingGroupRow {
                id: 0x10001,
                member_count: 2,
                base_bonus: -123,
            },
            TechSharingGroupRow {
                id: 0x20001,
                member_count: 0,
                base_bonus: 456,
            },
        ]));
    *frame.tables[TableId::TechSharingMembers.index()] =
        TableView::Complete(TableRows::TechSharingMembers(vec![
            TechSharingMemberRow {
                group: 0x10001,
                ordinal: 0,
                country: 11,
                bonus: -2_147_483_649,
            },
            TechSharingMemberRow {
                group: 0x10001,
                ordinal: 1,
                country: 12,
                bonus: 0,
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 0x10001,
                literal: b"first".to_vec(),
            },
            LexicalTokenRow {
                token: 0x20001,
                literal: b"second".to_vec(),
            },
        ]));
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables.get("TechSharingGroups"), Some(&2));
    assert_eq!(report.compared_tables.get("TechSharingMembers"), Some(&2));
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("group token is absent"))
    );

    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    rows[0].literal.extend_from_slice(b"\0unserialized_suffix");
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables.get("TechSharingGroups"), Some(&1));
    assert_eq!(report.compared_tables.get("TechSharingMembers"), Some(&0));
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("NUL") && reason.contains("unavailable mapping"))
    );
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("first NUL"))
    );

    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows[0].bonus += 1;
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("country_bonus_multiset"));
    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows[0].bonus -= 1;
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    rows[0].literal = b"first".to_vec();

    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows.swap(0, 1);
    rows[0].ordinal = 0;
    rows[1].ordinal = 1;
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(
        report
            .differences
            .iter()
            .filter(|difference| difference.contains("TechSharingMembers.country:"))
            .count(),
        2
    );
    assert_eq!(
        report
            .differences
            .iter()
            .filter(|difference| difference.contains("TechSharingMembers.bonus:"))
            .count(),
        2
    );
    assert!(
        !report
            .differences
            .iter()
            .any(|difference| difference.contains("country_bonus_multiset"))
    );

    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows.swap(0, 1);
    rows[0].ordinal = 0;
    rows[1].ordinal = 1;
    rows[0].bonus += 1;
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(
        report
            .differences
            .iter()
            .filter(|difference| difference.contains("TechSharingMembers.bonus:"))
            .count(),
        1
    );
}

#[test]
fn sharing_without_dictionary_checks_values_and_native_census() {
    let (mut frame, pack, country) = sharing_fixture();
    let links = BTreeMap::from([(11, &country)]);
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "LexicalTokensUnavailable: writer scheduling unproved".into(),
    };
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("group token is absent"))
    );

    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows[0].bonus += 1;
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("country_bonus_multiset"));

    let TableView::Complete(TableRows::TechSharingGroups(rows)) =
        &mut *frame.tables[TableId::TechSharingGroups.index()]
    else {
        unreachable!()
    };
    rows[0].member_count += 1;
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("captured_member_count"))
    );

    *frame.tables[TableId::TechSharingMembers.index()] = TableView::Unavailable {
        reason: "member read fault".into(),
    };
    assert!(compare_sharing_fixture(&frame, &pack, &links, &mut Report::default()).is_err());
}

#[test]
fn sharing_partial_names_do_not_fabricate_missing_groups_or_hide_known_group_differences() {
    let (mut frame, pack, country) = sharing_fixture();
    let links = BTreeMap::from([(11, &country)]);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 0x10001,
            literal: b"first".to_vec(),
        }]));
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables.get("TechSharingGroups"), Some(&1));
    assert_eq!(report.compared_tables.get("TechSharingMembers"), Some(&1));

    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows[0].bonus = 2;
    rows[1].bonus = 1;
    let mut report = Report::default();
    compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("TechSharingMembers.bonus"));
}

#[test]
fn sharing_unresolved_names_still_reject_orphans_and_gapped_ordinals() {
    let (mut frame, pack, country) = sharing_fixture();
    let links = BTreeMap::from([(11, &country)]);
    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows[0].ordinal = 1;
    assert!(compare_sharing_fixture(&frame, &pack, &links, &mut Report::default()).is_err());
    let TableView::Complete(TableRows::TechSharingMembers(rows)) =
        &mut *frame.tables[TableId::TechSharingMembers.index()]
    else {
        unreachable!()
    };
    rows[0].ordinal = 0;
    rows[0].group = 1;
    assert!(compare_sharing_fixture(&frame, &pack, &links, &mut Report::default()).is_err());
}

#[test]
fn decisions_compare_signed_target_bits_and_detect_missing_source_rows() {
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(11, &country), (12, &country)]);
    let mut containers: [Vec<saved::Decision>; 5] = Default::default();
    containers[3].push(saved::Decision {
        token: b"decision".to_vec(),
        days: -4,
        lifecycle: Some(3),
        target: Some(saved::Target::Numeric(0x8000_0000)),
        ignore: Some(true),
    });
    pack.countries.get_mut(b"AAA".as_slice()).unwrap().decisions = Some(saved::Decisions {
        taken: Vec::new(),
        containers,
        random_items: Vec::new(),
    });
    *frame.tables[TableId::DecisionCounts.index()] =
        TableView::Complete(TableRows::DecisionCounts(vec![DecisionCountsRow {
            country: 11,
            targeted: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::Decisions.index()] =
        TableView::Complete(TableRows::Decisions(vec![DecisionRow {
            country: 12,
            container: 3,
            ordinal: 0,
            token: b"decision".to_vec(),
            days: -4,
            lifecycle: 3,
            target_kind: 2,
            country_target: u32::MAX,
            numeric_target: 0x8000_0000,
            ignore: 1,
            description_seed: replay_data::rows::Recorded::Value(None),
        }]));
    let mut report = Report::default();
    compare_decisions(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let TableView::Complete(TableRows::Decisions(rows)) =
        &mut *frame.tables[TableId::Decisions.index()]
    else {
        unreachable!()
    };
    rows[0].numeric_target += 1;
    let mut report = Report::default();
    compare_decisions(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::Decisions.index()] =
        TableView::Complete(TableRows::Decisions(Vec::new()));
    let mut report = Report::default();
    compare_decisions(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::Decisions.index()] = TableView::Unavailable {
        reason: "fault".into(),
    };
    assert!(compare_decisions(&frame, &pack, &links, &mut Report::default()).is_err());
}

#[test]
fn decision_description_seeds_compare_native_first_match_and_known_absence() {
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(11, &country), (12, &country)]);
    for (country_target, numeric_target, saved_target, seed_target) in [
        (12, 0, saved::Target::Country(b"AAA".to_vec()), 7),
        (
            u32::MAX,
            i32::MIN as u32,
            saved::Target::Numeric(i32::MIN as u32),
            i32::MIN,
        ),
    ] {
        let mut containers: [Vec<saved::Decision>; 5] = Default::default();
        containers[3].push(saved::Decision {
            token: b"mission".to_vec(),
            days: 0,
            lifecycle: Some(0),
            target: Some(saved_target),
            ignore: Some(false),
        });
        pack.countries.get_mut(b"AAA".as_slice()).unwrap().decisions = Some(saved::Decisions {
            taken: Vec::new(),
            containers,
            random_items: vec![
                saved::DecisionRandomItem {
                    token: b"other".to_vec(),
                    count: 3,
                    target: seed_target,
                },
                saved::DecisionRandomItem {
                    token: b"MiSsIoN".to_vec(),
                    count: i32::MIN,
                    target: seed_target,
                },
                saved::DecisionRandomItem {
                    token: b"mission".to_vec(),
                    count: i32::MAX,
                    target: seed_target,
                },
            ],
        });
        let mut row = DecisionRow {
            country: 11,
            container: 3,
            token: b"mission".to_vec(),
            country_target,
            numeric_target,
            target_kind: if country_target as i32 > 0 { 1 } else { 2 },
            description_seed: replay_data::rows::Recorded::Value(Some(Some(i32::MIN))),
            ..Default::default()
        };
        let compare = |row: DecisionRow, pack: &saved::Pack, frame: &mut DecodedFrame| {
            *frame.tables[TableId::Decisions.index()] =
                TableView::Complete(TableRows::Decisions(vec![row]));
            let mut report = Report::default();
            compare_decisions(frame, pack, &links, &mut report).unwrap();
            report
        };
        assert!(
            compare(row.clone(), &pack, &mut frame)
                .differences
                .is_empty()
        );
        row.description_seed = replay_data::rows::Recorded::Value(Some(None));
        let report = compare(row.clone(), &pack, &mut frame);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("description_seed"));
        pack.countries
            .get_mut(b"AAA".as_slice())
            .unwrap()
            .decisions
            .as_mut()
            .unwrap()
            .random_items
            .clear();
        assert!(
            compare(row.clone(), &pack, &mut frame)
                .differences
                .is_empty()
        );
        row.description_seed = replay_data::rows::Recorded::Value(Some(Some(0)));
        assert_eq!(compare(row.clone(), &pack, &mut frame).differences.len(), 1);
        row.description_seed = replay_data::rows::Recorded::Value(None);
        assert!(compare(row, &pack, &mut frame).differences.is_empty());
    }
}

#[test]
fn modifier_omissions_check_predicates_without_fabricating_negative_defaults() {
    let (mut frame, mut pack, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let saved = saved::DynamicModifier {
        token: b"modifier".to_vec(),
        scope_country: None,
        scope_state: None,
        days: None,
        enabled: false,
        values: vec![content_reader_save::Decimal::parse(b"-21474.83649").unwrap()],
    };
    pack.countries
        .get_mut(b"AAA".as_slice())
        .unwrap()
        .dynamic_modifiers
        .push(saved);
    *frame.tables[TableId::DynamicModifiers.index()] =
        TableView::Complete(TableRows::DynamicModifiers(vec![DynamicModifierRow {
            country: 11,
            ordinal: 0,
            scope_country: u32::MAX,
            scope_state: -7,
            days_remaining: -2,
            enabled: 0,
            token: b"modifier".to_vec(),
            values: vec![-2_147_483_649],
        }]));
    let mut report = Report::default();
    compare_modifiers(&frame, &pack, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(!report.limitations.is_empty());
    let TableView::Complete(TableRows::DynamicModifiers(rows)) =
        &mut *frame.tables[TableId::DynamicModifiers.index()]
    else {
        unreachable!()
    };
    rows[0].days_remaining = 0;
    let mut report = Report::default();
    compare_modifiers(&frame, &pack, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn ordered_rows_cannot_skip_or_duplicate_native_positions() {
    assert!(check_ordinals([0, 1, 2]).is_ok());
    assert!(check_ordinals([0, 2]).is_err());
    assert!(check_ordinals([0, 0]).is_err());
    assert!(check_ordinals([1]).is_err());
}

#[test]
fn one_native_unit_and_literal_suffixes_have_explicit_outcomes() {
    let mut report = Report::default();
    equal(&mut report, "fixed", -2_147_483_649_i64, -2_147_483_648_i64);
    compare_literal(&mut report, "token", b"name\0native_tail", b"name");
    assert_eq!(report.differences.len(), 1);
    assert_eq!(report.limitations.len(), 1);
}

#[test]
fn scoped_research_technologies_need_a_full_refresh() {
    let (mut frame, pack, country) = fixture();
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(vec![
            TechnologyDefinitionRow {
                database_index: 5,
                technology_handle: 5,
                id: b"technology".to_vec(),
                ..Default::default()
            },
        ]));
    let links = BTreeMap::from([(11, &country)]);
    let scoped = |frame: &DecodedFrame, full: bool| {
        let mut report = refreshed_report();
        report.read_hours = Some(BTreeMap::new());
        report.forced_full = full;
        let _ = compare_research(
            frame,
            &pack,
            &links,
            &BTreeMap::from([(5, b"technology".as_slice())]),
            &mut report,
        );
        report
    };
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(Vec::new()));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![
        replay_data::rows::SliceReadRow {
            table: TableId::ResearchTechnologies.index() as u32,
            owner: 12,
            sample_hour: -3,
        },
    ]));
    let carried = scoped(&frame, false);
    assert!(!carried.compared_tables.contains_key("ResearchTechnologies"));
    assert!(carried.differences.is_empty(), "{:?}", carried.differences);
    assert_eq!(
        carried
            .carried
            .get("governance ResearchTechnologies (owners not read at this hour)"),
        Some(&Some(-3))
    );
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                database_index: 5,
                country: 11,
                technology_handle: 5,
                progress_raw: 999,
                ..Default::default()
            },
        ]));
    let full = scoped(&frame, true);
    assert!(full.carried.is_empty(), "{:?}", full.carried);
    assert!(
        full.differences
            .iter()
            .any(|line| line.starts_with("AAA.technology")),
        "{:?} failed {:?}",
        full.differences,
        full.failed_comparisons
    );
    let hidden = scoped(&frame, false);
    assert!(hidden.differences.is_empty(), "{:?}", hidden.differences);
}

#[test]
fn stale_or_unknown_research_reads_skip_the_research_family() {
    let (mut frame, pack, country) = fixture();
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                database_index: 5,
                country: 11,
                technology_handle: 5,
                ..Default::default()
            },
        ]));
    let read_at = |hour| {
        TableView::Complete(TableRows::SliceReads(vec![
            replay_data::rows::SliceReadRow {
                table: TableId::ResearchTechnologies.index() as u32,
                owner: 11,
                sample_hour: hour,
            },
        ]))
    };
    let skipped = |frame: &DecodedFrame, mut report: Report| {
        let _ = compare_research(
            frame,
            &pack,
            &BTreeMap::from([(11, &country)]),
            &BTreeMap::from([(5, b"technology".as_slice())]),
            &mut report,
        );
        report
            .limitations
            .keys()
            .any(|reason| reason.contains(replay_data::cadence::STAGGER_LIMITATION))
    };
    *frame.tables[TableId::SliceReads.index()] = read_at(0);
    assert!(!skipped(&frame, refreshed_report()));
    assert!(skipped(&frame, Report::default()));
    frame.hour = 1;
    assert!(skipped(&frame, refreshed_report()));
    frame.hour = 0;
    *frame.tables[TableId::SliceReads.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    assert!(skipped(&frame, refreshed_report()));
}

#[test]
fn technology_occurrence_dictionary_accepts_shared_tokens_and_refuses_lexer_aliases() {
    let (mut frame, pack, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(vec![
            TechnologyDefinitionRow {
                database_index: 1,
                technology_handle: 7,
                id: b"repeat".to_vec(),
                ..Default::default()
            },
            TechnologyDefinitionRow {
                database_index: 2,
                technology_handle: 7,
                id: b"REPEAT".to_vec(),
                ..Default::default()
            },
        ]));
    let names = BTreeMap::from([(7, b"RePeAt".as_slice())]);
    super::research::compare_research(&frame, &pack, &links, &names, &mut refreshed_report())
        .unwrap();
    let TableView::Complete(TableRows::TechnologyDefinitions(rows)) =
        &mut *frame.tables[TableId::TechnologyDefinitions.index()]
    else {
        unreachable!()
    };
    rows[1].technology_handle = 8;
    let mut report = refreshed_report();
    assert!(
        super::research::compare_research(&frame, &pack, &links, &names, &mut report)
            .unwrap_err()
            .to_string()
            .contains("handle 8")
    );
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn research_serializer_keeps_repeated_occurrences_order_and_exact_active_filter() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let pack = saved::parse(&content_reader_parser::parser::parse(b"countries={AAA={technology={technologies={repeat={level=1} repeat={research_points=0.5} repeat={ahead_reduction=0.00023}}}}}").unwrap().items).unwrap();
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(
            (1..=5)
                .map(|database_index| TechnologyDefinitionRow {
                    database_index,
                    technology_handle: 7,
                    id: if database_index == 2 {
                        b"REPEAT".to_vec()
                    } else {
                        b"repeat".to_vec()
                    },
                    max_level: 2,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(
            (1..=5)
                .map(|database_index| ResearchTechnologyRow {
                    country: 11,
                    database_index,
                    technology_handle: 7,
                    level: if database_index == 1 { 1 } else { 0 },
                    progress_raw: if database_index == 2 { 50_000 } else { 0 },
                    ahead_reduction_years_raw: if database_index >= 3 {
                        i64::from(database_index * 7 - 5)
                    } else {
                        0
                    },
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ResearchSlots.index()] =
        TableView::Complete(TableRows::ResearchSlots(vec![ResearchSlotRow {
            country: 11,
            technology_handle: 7,
            technology_present: 1,
            database_index: replay_data::rows::Recorded::Value(4),
            ..Default::default()
        }]));
    let dictionary =
        BTreeMap::from_iter((1..=5).map(|index| (index, (b"repeat".as_slice(), 2, 7))));
    let compare = |frame: &DecodedFrame, pack: &saved::Pack| {
        let mut report = Report::default();
        super::research::compare_research_technologies(
            frame,
            pack,
            &links,
            &dictionary,
            &mut report,
        )
        .unwrap();
        report
    };
    let report = compare(&frame, &pack);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables.get("ResearchTechnologies"), Some(&3));
    let mut at_limit = dictionary.clone();
    at_limit.get_mut(&4).unwrap().1 = 0;
    let mut report = Report::default();
    super::research::compare_research_technologies(&frame, &pack, &links, &at_limit, &mut report)
        .unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("serialized_occurrences"))
    );
    let mut reordered = pack.clone();
    reordered
        .countries
        .get_mut(b"AAA".as_slice())
        .unwrap()
        .research
        .as_mut()
        .unwrap()
        .technologies
        .swap(0, 1);
    assert!(!compare(&frame, &reordered).differences.is_empty());
    let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *frame.tables[TableId::ResearchTechnologies.index()]
    else {
        unreachable!()
    };
    rows[4].applied_bonus_count = 1;
    assert!(
        compare(&frame, &pack)
            .differences
            .iter()
            .any(|difference| difference.contains("serialized_occurrences"))
    );
    let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *frame.tables[TableId::ResearchTechnologies.index()]
    else {
        unreachable!()
    };
    rows[4].applied_bonus_count = 0;
    rows[0].technology_handle = 8;
    assert!(
        super::research::compare_research_technologies(
            &frame,
            &pack,
            &links,
            &dictionary,
            &mut Report::default()
        )
        .unwrap_err()
        .to_string()
        .contains("handle disagrees")
    );
    let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *frame.tables[TableId::ResearchTechnologies.index()]
    else {
        unreachable!()
    };
    rows[0].technology_handle = 7;
    *frame.tables[TableId::ResearchSlots.index()] = TableView::Unavailable {
        reason: "fixture active slot unavailable".into(),
    };
    let mut report = Report::default();
    assert!(
        super::research::compare_research_technologies(
            &frame,
            &pack,
            &links,
            &dictionary,
            &mut report
        )
        .is_err()
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("order, multiplicity"))
    );
    let mut differing = pack.clone();
    differing
        .countries
        .get_mut(b"AAA".as_slice())
        .unwrap()
        .research
        .as_mut()
        .unwrap()
        .technologies[0]
        .1
        .level = Some(99);
    let mut report = Report::default();
    assert!(
        super::research::compare_research_technologies(
            &frame,
            &differing,
            &links,
            &dictionary,
            &mut report,
        )
        .is_err()
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("serializer_required_occurrence[1]"))
    );
}

#[test]
fn has_elections_compares_both_saved_operands_and_skips_older_recordings() {
    let (mut frame, mut pack, country) = fixture();
    let differences = |frame: &DecodedFrame, pack: &saved::Pack| {
        let mut report = Report::default();
        compare_politics(
            frame,
            pack,
            &BTreeMap::from([(country.id, &country)]),
            &mut report,
        )
        .unwrap();
        report
            .differences
            .into_iter()
            .filter(|line| line.contains("has_elections"))
            .collect::<Vec<_>>()
    };
    let rows = |has_elections| {
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: country.id,
            has_elections,
            ..Default::default()
        }]))
    };
    for (allowed, frequency, recorded, differs) in [
        (true, 48, true, false),
        (true, 0, false, false),
        (false, 48, false, false),
        (true, 0, true, true),
        (false, 48, true, true),
    ] {
        pack.countries.get_mut(b"AAA".as_slice()).unwrap().politics = Some(saved::Politics {
            parties: BTreeMap::new(),
            leaders: BTreeMap::new(),
            names: BTreeMap::new(),
            ideas: Vec::new(),
            timed_ideas: Vec::new(),
            ruling_party: None,
            last_election: None,
            election_frequency: Some(frequency),
            elections_allowed: Some(allowed),
        });
        *frame.tables[TableId::Politics.index()] = rows(Recorded::Value(recorded));
        assert_eq!(
            !differences(&frame, &pack).is_empty(),
            differs,
            "{allowed} {frequency} {recorded}"
        );
        *frame.tables[TableId::Politics.index()] = rows(Recorded::NotRecorded);
        assert!(differences(&frame, &pack).is_empty());
    }
}
