use super::*;
use replay_data::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow, Recorded,
    TableId, TableRows,
};
use replay_frame::frame::{DecodedFrame, Metrics, TableView};

/// The tables [`choices`] reads, its ruling groups included.
const TABLES: [TableId; 5] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
];

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
    let flags = choices(source).unwrap();
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
    for flag in choices(&frame).unwrap() {
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
        let choices = choices(&flag_frame(captured, b"COS")).unwrap();
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
        let choices = choices(&flag_frame(captured, b"COS")).unwrap();
        assert_eq!(choices[0].candidates, ["COS_group", "COS"]);
        assert_eq!(choices[0].key(|key| key == "COS"), Ok("COS"));
        assert!(
            choices[0]
                .key(|key| key == "OWN" || key == "GEN")
                .unwrap_err()
                .contains(reason)
        );
    }
    let invalid = choices(&flag_frame(Recorded::Value(Some(b"ORI".to_vec())), &[0xff])).unwrap();
    assert!(invalid[0].candidates.is_empty());
    assert!(
        invalid[0]
            .key(|key| key == "ORI")
            .unwrap_err()
            .contains("cosmetic tag is not UTF-8")
    );
    // A nonpositive raw reference chooses own independently of the
    // unused optional original bytes, including a failed byte capture.
    for reference in [0, -7] {
        let mut frame = flag_frame(Recorded::Value(None), b"COS");
        let appearance = match &mut *frame.tables[TableId::CountryAppearance.index()] {
            TableView::Complete(TableRows::CountryAppearance(rows)) => &mut rows[0],
            _ => unreachable!(),
        };
        appearance.original_tag_reference = Recorded::Value(Some(reference));
        let choice = choices(&frame).unwrap().remove(0);
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
        let choice = choices(&frame).unwrap().remove(0);
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
        let choice = choices(&frame).unwrap().remove(0);
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
        choices(&frame).unwrap()[0].candidates,
        ["COS_group", "COS", "ORI_group", "ORI"]
    );
    // A reader reconstructing only these tables holds everything choices reads.
    let mut selected = frame.clone();
    for table in TableId::ALL {
        if !TABLES.contains(&table) {
            *selected.tables[table.index()] = TableView::Unavailable {
                reason: "not selected".into(),
            };
        }
    }
    assert_eq!(
        choices(&selected).unwrap()[0].candidates,
        ["COS_group", "COS", "ORI_group", "ORI"]
    );
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 10,
            id: 4,
            token: Vec::new(),
            ..Default::default()
        }]));
    assert_eq!(ruling_groups(&frame).unwrap()[&10], Ok(Some(String::new())));
    // The ordinary group's capture identity proves an active flag even with
    // an empty token, so the qualified candidates keep the underscore.
    assert_eq!(
        choices(&frame).unwrap()[0].candidates,
        ["COS_", "COS", "ORI_", "ORI"]
    );
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 10,
            ruling_party: None,
            ..Default::default()
        }]));
    assert!(
        choices(&frame).unwrap()[0]
            .key(|key| key == "COS")
            .unwrap_err()
            .contains("null ruling-party pointer")
    );
    *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    assert!(
        choices(&frame).unwrap()[0]
            .key(|key| key == "COS")
            .unwrap_err()
            .contains("needs its Politics row")
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
    let choices = choices(&frame).unwrap();
    assert_eq!(
        choices.iter().map(|choice| choice.raw).collect::<Vec<_>>(),
        [10, 11]
    );
    assert_eq!(choices[0].candidates, choices[1].candidates);
    assert!(choices.iter().all(|choice| choice.refusal.is_none()));
    // The resolved object's own link need not occur in a partial alias
    // list for its already captured politics and appearance to be usable.
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 2,
        }]));
    let alias_only = super::choices(&frame).unwrap();
    assert_eq!(alias_only[0].candidates, choices[0].candidates);
    assert!(alias_only[0].refusal.is_none());
}
