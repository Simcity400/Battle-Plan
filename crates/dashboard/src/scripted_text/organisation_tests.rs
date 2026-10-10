use super::*;
use replay::rows::{IndustrialOrganisationRow, LexicalTokenRow, TableId, TableRows};

#[test]
fn organisation_name_uses_event_read_override_owner_scope_and_exact_missing_key() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = tests::catalogue(
        "",
        " GER_mod_override:0 \"Owner [From.GetTag]\"\n mod_token:0 \"Token\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    let mut frame = crate::scope::tests::positive_frame();
    let reference = 9 | 17u64 << 32;
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: reference,
                owner: 100,
                token: 800,
                name: b"mod_override".to_vec(),
                ..Default::default()
            },
        ]));
    // A stored override is independent of the lexer read.
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "token read refused".into(),
    };
    let show = |frame: &DecodedFrame| {
        ScriptedText::new(frame, &catalogue.localisation, content, 2).organisation_name(reference)
    };
    assert_eq!(show(&frame), Ok("Owner GER".into()));
    let change_name = |frame: &mut DecodedFrame, name: &[u8]| {
        let TableView::Complete(TableRows::IndustrialOrganisations(rows)) =
            &mut *frame.tables[TableId::IndustrialOrganisations.index()]
        else {
            panic!("organisations");
        };
        rows[0].name = name.to_vec();
    };
    change_name(&mut frame, b"mod_missing");
    // Localisation copies a missing key; it does not select a later name.
    assert_eq!(show(&frame), Ok("mod_missing".into()));
    change_name(&mut frame, b"");
    assert!(show(&frame).unwrap_err().contains("token read refused"));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 800,
            literal: b"mod_token".to_vec(),
        }]));
    assert_eq!(show(&frame), Ok("Token".into()));
    change_name(&mut frame, &[0xff]);
    assert_eq!(
        show(&frame),
        Err("industrial organisation name is not UTF-8".into())
    );
    change_name(&mut frame, b"mod_override");
    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "owner read refused".into(),
    };
    assert_eq!(
        show(&frame),
        Err("CountryLinks lacks industrial organisation owner".into())
    );
}
