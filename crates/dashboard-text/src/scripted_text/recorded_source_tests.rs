use super::*;
use dashboard_core::test_fixtures::game;
use replay_frame::frame::DecodedSource;

#[test]
fn decoded_source_keeps_resolved_mod_names_scripted_branches_and_scope_promotions() {
    game::enter();
    let (_, base) = catalogue(SCRIPTED, "");
    let (_, overlay) = crate::catalogue::tests::folder_catalogue(&[
        (
            "localisation/replace/mod_l_english.yml",
            "\u{feff}l_english:\n GER:0 \"Mod country\"\n MOD_TITLE:0 \"[ROOT.GetName] / [ROOT.GetTag]\"\n DORMANT_TITLE:0 \"[ROOT.GetOldName]\"\n",
        ),
        (
            "common/scripted_localisation/mod.txt",
            "defined_text={name=GetModText text={trigger={always=no} localization_key=DORMANT_TITLE} text={trigger={always=yes} localization_key=MOD_TITLE}}",
        ),
    ]);
    let layers =
        content_reader_layers::ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&overlay.0, &[][..])])
            .unwrap();
    let catalogue = crate::catalogue::tests::layers_catalogue(layers);
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = with_party(frame());
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(Vec::new()));
    let source: &dyn DecodedSource = &frame;
    let text = ScriptedText::new(source, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[ROOT.GetModText]").as_deref(),
        Ok("Mod country / GER")
    );
    assert_eq!(
        text.expand("[ROOT.Capital.GetID] / [ROOT.Capital.Controller.GetTag]")
            .as_deref(),
        Ok("64 / SOV")
    );
    assert_eq!(
        text.expand("[ROOT.GER_erwin_rommel.GetName]").as_deref(),
        Ok("")
    );
    assert_eq!(text.expand("[ROOT.Nobody.GetTag]").as_deref(), Ok("GER"));
    assert_eq!(
        text.expand("[ROOT.GetOldName]").unwrap_err(),
        "[ROOT.GetOldName]: GetOldName: the old name was unreadable or longer than 256 bytes"
    );
}

#[test]
fn decoded_source_keeps_missing_character_refusal_bytes() {
    game::enter();
    let (catalogue, _folder) = catalogue(SCRIPTED, "");
    let content = catalogue.scripted_content().unwrap();
    let mut frame = frame();
    *frame.tables[TableId::Characters.index()] = TableView::Truncated {
        rows: TableRows::Characters(Vec::new()),
        missing: 3,
    };
    let source: &dyn DecodedSource = &frame;
    let text = ScriptedText::new(source, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[ROOT.GER_erwin_rommel.GetName]").unwrap_err(),
        "[ROOT.GER_erwin_rommel.GetName]: GER_erwin_rommel: whether it names a character of country slot 1 is unknown: characters: the table is truncated (3 rows missing)"
    );
}
