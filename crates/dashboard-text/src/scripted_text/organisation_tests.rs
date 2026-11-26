use super::*;
use crate::script_test_game as game;
use replay_data::{
    rows::IndustrialOrganisationRow, rows::LexicalTokenRow, rows::TableId, rows::TableRows,
};

#[test]
fn organisation_name_uses_event_read_override_owner_scope_and_exact_missing_key() {
    game::enter();
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
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "token read refused".into(),
    };
    let show = |frame: &DecodedFrame| {
        let text = ScriptedText::new(frame, &catalogue.localisation, content, 2);
        let (template, scopes) = text.organisation_template(reference)?;
        text.expand_in(&template, &scopes)
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
    let template = |frame: &DecodedFrame, root| {
        ScriptedText::new(frame, &catalogue.localisation, content, root)
            .organisation_template(reference)
            .map(|(template, _)| template)
    };
    assert_eq!(template(&frame, 2), Ok("Owner [From.GetTag]".into()));
    change_name(&mut frame, b"mod_missing");
    assert_eq!(show(&frame), Ok("mod_missing".into()));
    assert_eq!(template(&frame, 0), show(&frame));
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
        Err("country links lacks industrial organisation owner".into())
    );
}
