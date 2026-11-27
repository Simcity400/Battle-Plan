use super::*;
use replay_data::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow, Recorded,
    TableId, TableRows,
};
use replay_frame::frame::{DecodedFrame, Metrics, TableView};

const TABLES: [TableId; 5] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
];

fn available_choices(frame: &dyn DecodedSource) -> Vec<FlagChoice> {
    match choices(frame).unwrap() {
        Choices::Available { choices, .. } => choices,
        Choices::Unavailable(why) => panic!("expected available choices, got {why}"),
    }
}

fn available_groups(frame: &dyn DecodedSource) -> BTreeMap<u32, Result<Option<String>, String>> {
    match ruling_groups(frame).unwrap() {
        RulingGroups::Available { groups, .. } => groups,
        RulingGroups::Unavailable(why) => panic!("expected available ruling groups, got {why}"),
    }
}

#[test]
fn decoded_source_keeps_mod_art_candidates_aliases_and_unknown_tail() {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 17,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [100, 150]
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: 1,
            })
            .into(),
    ));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 1,
            tag: b"D01".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 100,
            cosmetic_tag: b"MOD_COS".to_vec(),
            original_tag_reference: Recorded::Value(Some(200)),
            original_tag: Recorded::Value(Some(b"ORG".to_vec())),
            ..Default::default()
        }]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 100,
            ruling_party: Some(7),
            ..Default::default()
        }]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 7,
            token: b"mod_group".to_vec(),
            ..Default::default()
        }]));
    let source: &dyn DecodedSource = &frame;
    let flags = available_choices(source);
    assert_eq!(
        flags.iter().map(|flag| flag.raw).collect::<Vec<_>>(),
        [100, 150]
    );
    for flag in &flags {
        assert_eq!(flag.tag, "D01");
        assert_eq!(
            flag.candidates,
            ["MOD_COS_mod_group", "MOD_COS", "ORG_mod_group", "ORG"]
        );
        assert_eq!(flag.key(|key| key == "ORG_mod_group"), Ok("ORG_mod_group"));
        assert_eq!(flag.refusal, None);
    }
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(None);
    }
    for flag in available_choices(&frame) {
        assert_eq!(flag.candidates, ["MOD_COS_mod_group", "MOD_COS"]);
        assert_eq!(flag.key(|key| key == "MOD_COS"), Ok("MOD_COS"));
        assert_eq!(
            flag.key(|_| false).unwrap_err(),
            "original tag reference of country appearance capture refused"
        );
    }
}

fn flag_frame(original: Recorded<Option<Vec<u8>>>, cosmetic: &[u8]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 10,
            canonical_slot: 2,
        }]));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 10,
            canonical_slot: 2,
            tag: b"OWN".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 10,
            name_tag: b"GEN".to_vec(),
            cosmetic_tag: cosmetic.to_vec(),
            original_tag: original,
            original_tag_reference: Recorded::Value(Some(1)),
            ..Default::default()
        }]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 10,
            ruling_party: Some(4),
            ..Default::default()
        }]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 10,
            id: 4,
            token: b"group".to_vec(),
            ..Default::default()
        }]));
    frame
}

#[test]
fn flag_choices_use_exact_original_and_keep_known_cosmetic_before_suffix_refusals() {
    for (captured, tail) in [
        (Recorded::Value(Some(b"ORI".to_vec())), "ORI"),
        (Recorded::Value(Some(Vec::new())), ""),
    ] {
        let choices = available_choices(&flag_frame(captured, b"COS"));
        assert_eq!(
            choices[0].candidates,
            [
                "COS_group".to_owned(),
                "COS".to_owned(),
                format!("{tail}_group"),
                tail.to_owned()
            ]
        );
        assert_eq!(choices[0].key(|key| key == tail), Ok(tail));
        assert!(choices[0].refusal.is_none());
    }
    for (captured, reason) in [
        (
            Recorded::Value(None),
            "original tag of country appearance capture refused",
        ),
        (
            Recorded::NotRecorded,
            "original tag of country appearance not recorded",
        ),
        (Recorded::Value(Some(vec![0xff])), "not UTF-8"),
    ] {
        let choices = available_choices(&flag_frame(captured, b"COS"));
        assert_eq!(choices[0].candidates, ["COS_group", "COS"]);
        assert_eq!(choices[0].key(|key| key == "COS"), Ok("COS"));
        assert!(
            choices[0]
                .key(|key| key == "OWN" || key == "GEN")
                .unwrap_err()
                .contains(reason)
        );
    }
    let invalid = available_choices(&flag_frame(Recorded::Value(Some(b"ORI".to_vec())), &[0xff]));
    assert!(invalid[0].candidates.is_empty());
    assert!(
        invalid[0]
            .key(|key| key == "ORI")
            .unwrap_err()
            .contains("cosmetic tag is not UTF-8")
    );
    for reference in [0, -7] {
        let mut frame = flag_frame(Recorded::Value(None), b"COS");
        let appearance = match &mut *frame.tables[TableId::CountryAppearance.index()] {
            TableView::Complete(TableRows::CountryAppearance(rows)) => &mut rows[0],
            _ => unreachable!(),
        };
        appearance.original_tag_reference = Recorded::Value(Some(reference));
        let choice = available_choices(&frame).remove(0);
        assert_eq!(choice.candidates, ["COS_group", "COS", "OWN_group", "OWN"]);
        assert!(choice.refusal.is_none());
    }
    for (reference, reason) in [
        (
            Recorded::Value(None),
            "original tag reference of country appearance capture refused",
        ),
        (
            Recorded::NotRecorded,
            "original tag reference of country appearance not recorded",
        ),
    ] {
        let mut frame = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
        let appearance = match &mut *frame.tables[TableId::CountryAppearance.index()] {
            TableView::Complete(TableRows::CountryAppearance(rows)) => &mut rows[0],
            _ => unreachable!(),
        };
        appearance.original_tag_reference = reference;
        let choice = available_choices(&frame).remove(0);
        assert_eq!(choice.candidates, ["COS_group", "COS"]);
        assert_eq!(choice.key(|key| key == "COS"), Ok("COS"));
        assert!(choice.key(|key| key == "ORI").unwrap_err().contains(reason));
    }
}

#[test]
fn flag_choices_preserve_unknown_ruling_groups_before_plain_candidates() {
    let mut frame = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 10,
            ruling_party: Some(4),
            ..Default::default()
        }]));
    for (party, reason) in [
        (None, "needs political parties ruling group 4"),
        (Some(vec![0xff]), "ruling group is not UTF-8"),
    ] {
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(
                party
                    .map(|token| PoliticalPartyRow {
                        country: 10,
                        id: 4,
                        token,
                        ..Default::default()
                    })
                    .into_iter()
                    .collect(),
            ));
        let choice = available_choices(&frame).remove(0);
        assert!(choice.candidates.is_empty());
        assert!(
            choice
                .key(|key| key == "COS" || key == "ORI")
                .unwrap_err()
                .contains(reason)
        );
    }
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 10,
            id: 4,
            token: b"group".to_vec(),
            ..Default::default()
        }]));
    assert_eq!(
        available_choices(&frame)[0].candidates,
        ["COS_group", "COS", "ORI_group", "ORI"]
    );
    let mut selected = frame.clone();
    for table in TableId::ALL {
        if !TABLES.contains(&table) {
            *selected.tables[table.index()] = TableView::Unavailable {
                reason: "not selected".into(),
            };
        }
    }
    assert_eq!(
        available_choices(&selected)[0].candidates,
        ["COS_group", "COS", "ORI_group", "ORI"]
    );
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 10,
            id: 4,
            token: Vec::new(),
            ..Default::default()
        }]));
    assert_eq!(available_groups(&frame)[&10], Ok(Some(String::new())));
    assert_eq!(
        available_choices(&frame)[0].candidates,
        ["COS_", "COS", "ORI_", "ORI"]
    );
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 10,
            ruling_party: None,
            ..Default::default()
        }]));
    assert!(
        available_choices(&frame)[0]
            .key(|key| key == "COS")
            .unwrap_err()
            .contains("null ruling-party pointer")
    );
    *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    let error = available_choices(&frame)[0]
        .key(|key| key == "COS")
        .unwrap_err();
    assert!(error.contains("read failed"), "{error}");
    assert!(
        available_groups(&frame)[&10]
            .as_ref()
            .unwrap_err()
            .contains("read failed")
    );
}

#[test]
fn unavailable_tables_carry_their_reason_through_flag_choices_and_ruling_groups() {
    let base = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
    for table in [TableId::CountryLinks, TableId::Countries] {
        let mut frame = base.clone();
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "damaged block".into(),
        };
        let Choices::Unavailable(why) = choices(&frame).unwrap() else {
            panic!("{table:?} unavailable must not return empty choices");
        };
        assert!(why.contains("damaged block"), "{why}");
    }
    let mut no_countries = base.clone();
    *no_countries.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "older format".into(),
    };
    let RulingGroups::Unavailable(why) = ruling_groups(&no_countries).unwrap() else {
        panic!("Countries unavailable must not return empty ruling groups");
    };
    assert!(why.contains("older format"), "{why}");

    let mut no_appearance = base.clone();
    *no_appearance.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "appearance unread".into(),
    };
    let error = available_choices(&no_appearance)[0]
        .key(|_| true)
        .unwrap_err();
    assert!(error.contains("appearance unread"), "{error}");

    let mut no_parties = base;
    *no_parties.tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
        reason: "parties unread".into(),
    };
    let error = available_choices(&no_parties)[0]
        .key(|_| true)
        .unwrap_err();
    assert!(error.contains("parties unread"), "{error}");
    assert!(
        available_groups(&no_parties)[&10]
            .as_ref()
            .unwrap_err()
            .contains("parties unread")
    );
}

#[test]
fn flag_aliases_use_the_resolved_country_objects_operands() {
    let mut frame = flag_frame(Recorded::Value(Some(b"ORI".to_vec())), b"COS");
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 10,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 11,
                canonical_slot: 2,
            },
        ]));
    let choices = available_choices(&frame);
    assert_eq!(
        choices.iter().map(|choice| choice.raw).collect::<Vec<_>>(),
        [10, 11]
    );
    assert_eq!(choices[0].candidates, choices[1].candidates);
    assert!(choices.iter().all(|choice| choice.refusal.is_none()));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 2,
        }]));
    let alias_only = available_choices(&frame);
    assert_eq!(alias_only[0].candidates, choices[0].candidates);
    assert!(alias_only[0].refusal.is_none());
}
