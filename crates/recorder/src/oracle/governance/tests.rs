use super::*;

/// A report whose frame refreshed ResearchSlots, as a full-refresh hour does.
fn refreshed_report() -> Report {
    let mut report = Report::default();
    report
        .observed_refresh
        .push(replay::cadence::cadence(TableId::ResearchSlots));
    report
}
use replay::{
    frame::{Metrics, TableView},
    rows::{
        ActiveIdeaRow, CompletedFocusRow, CountryTacticUnlockRow, DecisionCountsRow, DecisionRow,
        DynamicModifierRow, LexicalTokenRow, PreferredTacticRow, ResearchSlotRow,
        ResearchTechnologyRow, TableId, TableRows, TechSharingGroupRow, TechSharingMemberRow,
        TechnologyDefinitionRow,
    },
};

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

fn fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
    let country = CountryRow {
        border_land_doctrine_level: replay::rows::Recorded::Value(None),
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

/// Saved status N is canonical slot N's (140D6C880, 140D6A7B0). Every
/// census country needs its captured row, empty or not; only a status
/// whose slot is outside the census may be missing from the recording,
/// and then only while it holds no unlock.
#[test]
fn tactic_unlocks_cover_the_canonical_census() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    // The fixture's AAA is canonical slot 7.
    let pack = |at_seven: saved::DoctrineStatus, stray: Vec<i32>| saved::Pack {
        doctrine: Some(
            (0..8)
                .map(|slot| match slot {
                    7 => at_seven.clone(),
                    3 => saved::DoctrineStatus {
                        country: Some(b"BBB".to_vec()),
                        enable_tactic: stray.clone(),
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
    // Order is native vector order, not a set.
    let (_, report) = compare(vec![row(vec![2, 4])], &pack(aaa(vec![4, 2]), Vec::new()));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("AAA.enable_tactic"))
    );
    // A census country's missing row is a difference even when its
    // saved status is empty: the collector promises explicit rows.
    let (_, report) = compare(Vec::new(), &pack(aaa(Vec::new()), Vec::new()));
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("no captured row for a census country"));
    // A status at the census slot must name that country.
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
    // Unlocks on a slot outside the census are a difference.
    let (_, report) = compare(vec![row(vec![4, 2])], &pack(aaa(vec![4, 2]), vec![9]));
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("doctrine status 3.enable_tactic"));
    // A save without its doctrine block is a failed comparison.
    let (ok, _) = compare(vec![row(vec![4, 2])], &saved::Pack::default());
    assert!(!ok);
}

#[test]
fn unavailable_siblings_retain_independent_comparisons_and_aggregate_failure() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let document = installed_content::parser::parse(
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
                technology_handle: 5,
                id: b"technology".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                country: 11,
                technology_handle: 5,
                progress_raw: 1,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![replay::rows::SliceReadRow {
            table: TableId::ResearchTechnologies.index() as u32,
            owner: 11,
            sample_hour: 0,
        }]));
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
    assert!(compare_pack(&frame, &pack, &links, &BTreeMap::new(), &mut report).is_err());
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
        "technology.technology.research_points",
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
    let document = installed_content::parser::parse(
        br#"countries={AAA={politics={parties={
        communism={default=no name="AAA_red" long_name="AAA_red_long" popularity=1}
        democratic={default=yes popularity=1}
        fascism={default=no name="AAA_black" long_name="" popularity=1}
    }}}}"#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    let party = |id, token: &[u8], tag, name_default, name: &[u8], long_name: &[u8]| {
        replay::rows::PoliticalPartyRow {
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
            // A default-named party's stored keys are not saved.
            party(2, b"democratic", 11, 1, b"stale", b""),
            // The canonical slot (7) is not the tag index.
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
    // Keys are compared only where the save wrote them.
    assert!(!differences.contains("communism"), "{differences}");
    assert!(!differences.contains("democratic"), "{differences}");
    assert!(!differences.contains("fascism.name"), "{differences}");
}

#[test]
fn idle_research_slot_is_not_equivalent_to_a_new_assignment() {
    let (mut frame, _, country) = fixture();
    let links = BTreeMap::from([(11, &country)]);
    let document = installed_content::parser::parse(
        br#"countries={AAA={technology={
        slots={selected_technology={points_factor=1.35}}
    }}}"#,
    )
    .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    let dictionary = BTreeMap::from([(5, (b"selected_technology".as_slice(), 1))]);
    *frame.tables[TableId::ResearchSlots.index()] =
        TableView::Complete(TableRows::ResearchSlots(vec![ResearchSlotRow {
            country: 11,
            index: 0,
            technology_handle: engine::cresearch_slot::IDLE_HANDLE,
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
    let document =
        installed_content::parser::parse(br#"countries={AAA={technology={slots={empty={points=3}}}}}"#)
            .unwrap();
    let pack = saved::parse(&document.items).unwrap();
    *frame.tables[TableId::ResearchSlots.index()] =
        TableView::Complete(TableRows::ResearchSlots(vec![ResearchSlotRow {
            country: 11,
            technology_handle: engine::cresearch_slot::IDLE_HANDLE,
            research_speed_raw: engine::cresearch_slot::RESEARCH_SPEED_SCALE,
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
                installed_content::save::Decimal::parse(if index == 0 { b"0.00001" } else { b"0.00002" })
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
        ..country.clone()
    };
    let links = BTreeMap::from([(11, &country), (12, &second)]);
    let document = installed_content::parser::parse(
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

    // The complete multiset is unchanged, but the saved pair order differs.
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

    // Preserve the aggregate pairs while moving them between native groups.
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
            description_seed: replay::rows::Recorded::Value(None),
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
            description_seed: replay::rows::Recorded::Value(Some(Some(i32::MIN))),
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
        row.description_seed = replay::rows::Recorded::Value(Some(None));
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
        row.description_seed = replay::rows::Recorded::Value(Some(Some(0)));
        assert_eq!(compare(row.clone(), &pack, &mut frame).differences.len(), 1);
        row.description_seed = replay::rows::Recorded::Value(None);
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
        values: vec![installed_content::save::Decimal::parse(b"-21474.83649").unwrap()],
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
        let _ = compare_research(frame, &pack, &links, &mut report);
        report
    };
    // Country 11 has no captured rows: a rowless owner, whose slice was
    // not read at this hour (only owner 12's read is recorded). Rows
    // alone would call it current; the scoped oracle carries it.
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(Vec::new()));
    *frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![replay::rows::SliceReadRow {
            table: TableId::ResearchTechnologies.index() as u32,
            owner: 12,
            sample_hour: -3,
        }]));
    let carried = scoped(&frame, false);
    assert!(!carried.compared_tables.contains_key("ResearchTechnologies"));
    assert!(carried.differences.is_empty(), "{:?}", carried.differences);
    assert_eq!(
        carried
            .carried
            .get("governance ResearchTechnologies (owners not read at this hour)"),
        Some(&Some(-3))
    );
    // At a full refresh the technologies are compared, and a real
    // difference (a captured row unlike the save) is kept.
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
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
    // The same differing row at a non-full hour is carried, not compared.
    let hidden = scoped(&frame, false);
    assert!(hidden.differences.is_empty(), "{:?}", hidden.differences);
}

#[test]
fn stale_or_unknown_research_reads_skip_the_research_family() {
    let (mut frame, pack, country) = fixture();
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(vec![
            ResearchTechnologyRow {
                country: 11,
                technology_handle: 5,
                ..Default::default()
            },
        ]));
    let read_at = |hour| {
        TableView::Complete(TableRows::SliceReads(vec![replay::rows::SliceReadRow {
            table: TableId::ResearchTechnologies.index() as u32,
            owner: 11,
            sample_hour: hour,
        }]))
    };
    let skipped = |frame: &DecodedFrame, mut report: Report| {
        let _ = compare_research(frame, &pack, &BTreeMap::from([(11, &country)]), &mut report);
        report
            .limitations
            .keys()
            .any(|reason| reason.contains(replay::cadence::STAGGER_LIMITATION))
    };
    // Country 11's roster was read at hour 0, the oracle hour.
    *frame.tables[TableId::SliceReads.index()] = read_at(0);
    assert!(!skipped(&frame, refreshed_report()));
    // ResearchSlots was not refreshed at this hour.
    assert!(skipped(&frame, Report::default()));
    // The roster was read before the oracle hour.
    frame.hour = 1;
    assert!(skipped(&frame, refreshed_report()));
    // The read hours are unknown.
    frame.hour = 0;
    *frame.tables[TableId::SliceReads.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    assert!(skipped(&frame, refreshed_report()));
}
