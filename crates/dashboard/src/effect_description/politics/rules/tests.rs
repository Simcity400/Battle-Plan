use super::*;
use crate::catalogue::tests::folder_catalogue;
use crate::scripted_text::{EventObject, EventScope, Link};
use crate::triggers::ContentFacts;

fn fresh() -> EventScopes {
    EventScopes::new(vec![EventScope::of(EventObject::Country(1))])
}

#[test]
fn rules_use_registry_order_last_authored_value_and_fulfilled_field() {
    let (fixture, _folder) = super::super::tests::fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "set_rule={can_create_factions=yes can_send_volunteers=no can_send_volunteers=yes desc=SYN_DESC override=no can_not_declare_war=yes}",
        &mut fresh(),
        content.view(None),
    );
    assert_eq!(
        output.text,
        "Rules AAA:[no] CAN_NOT_DECLARE_WAR.\n[yes] Volunteers.\n[yes] Factions.\n"
    );
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn relation_uses_last_rule_and_explicit_target_before_trigger() {
    assert_eq!(relation("add_relation_rule_override={target=BBB trigger=SYN_TRIGGER usage_desc=SYN_DESC can_send_volunteers=yes can_create_factions=no}").unwrap(), "Relation Alpha->Beta:[no]Factions");
}

fn target(body: &str, graph: EventScopes) -> Result<String, String> {
    render(body, graph, None)
}

fn relation(body: &str) -> Result<String, String> {
    let template = Template {
        name: b"SYN_TRIGGER".to_vec(),
        body: Ok(installed_content::parser::parse(
            b"custom_trigger_tooltip={tooltip=SYN_OTHER_TIP always=yes}",
        )
        .unwrap()
        .items),
    };
    render(body, fresh(), Some(&template))
}

fn render(body: &str, graph: EventScopes, template: Option<&Template>) -> Result<String, String> {
    let (mut fixture, _fixture_folder) = super::super::tests::fixture();
    let (catalogue, _folder) = folder_catalogue(&[
        (
            "common/country_tags/test.txt",
            "AAA=\"countries/a.txt\" BBB=\"countries/b.txt\" CCC=\"countries/c.txt\"",
        ),
        (
            "map/default.map",
            "definitions=\"synthetic.csv\" provinces=\"synthetic.bmp\" terrain=\"synthetic.bmp\" rivers=\"synthetic.bmp\" heightmap=\"synthetic.bmp\" tree_definition=\"synthetic.bmp\" continent=\"synthetic_continents.txt\"",
        ),
        (
            "map/synthetic_continents.txt",
            "continents={SYN_CONT SYN_OTHER_CONT}",
        ),
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"gamma\"\n",
                " TRIGGER_FULLFILLED_PREFIX:0 \"[yes]\"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"[no]\"\n",
                " TRIGGER_AND_STARTS:0 \"All\"\n TRIGGER_OR_STARTS:0 \"Any\"\n",
                " EFFECT_ADD_RELATION_RULE_OVERRIDE:0 \"Relation $ORIGIN$->$TARGET$:\"\n CAN_SEND_VOLUNTEERS:0 \"Volunteers\"\n CAN_CREATE_FACTIONS:0 \"Factions\"\n",
                " TRIGGER_CAPITAL:0 \"Capital\"\n TRIGGER_IS_ON_CONTINENT:0 \"On $CONTINENT$ [This.GetTag]\"\n TRIGGER_IS_NOT_ON_CONTINENT:0 \"Off $CONTINENT$\"\n",
                " syn_cont:0 \"Continent\"\n AAA_syn_cont:0 \"Home [This.GetTag]\"\n syn_other_cont:0 \"Other continent\"\n",
                " TRIGGER_COUNTRY_FLAG_SET:0 \"$COUNTRY$ flag: \"\n TRIGGER_COUNTRY_FLAG_NOT_SET:0 \"$COUNTRY$ no flag: \"\n",
                " TRIGGER_FLAG_LIST_ENTRY:0 \"$FLAG$\"\n SYN_FLAG:0 \"Flag [This.GetTag]\"\n",
                " SYN_TIP:0 \"Tip [This.GetTag]\"\n SYN_TIP_NOT:0 \"Inverse [This.GetTag]\"\n SYN_OTHER_TIP:0 \"Other\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[replay::rows::TableId::Politics.index()] =
        replay::frame::TableView::Complete(replay::rows::TableRows::Politics(vec![
            replay::rows::PoliticsRow {
                country: 100,
                capital: 1,
                ..Default::default()
            },
            replay::rows::PoliticsRow {
                country: 200,
                capital: 2,
                ..Default::default()
            },
        ]));
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let text = ScriptedText::new(&fixture.frame, &fixture.catalogue.localisation, scripted, 1);
    let dlcs = Default::default();
    let triggers = Default::default();
    let aliases = Default::default();
    text.set_synthetic_world(World::read(
        &fixture.frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    ));
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic",
        scripted.effects()?.view(None),
    );
    let graph = Rc::new(RefCell::new(graph));
    let scopes = crate::triggers::event_scopes::project(&graph.borrow(), 0)?;
    let renderer = Renderer {
        context: &context,
        graph,
        evaluator: OnceCell::new(),
        out: Description::default(),
    };
    let items = installed_content::parser::parse(body.as_bytes()).unwrap().items;
    if template.is_some() {
        renderer.politics_relation_rule(&items[0], scopes, false, template)
    } else {
        renderer.politics_rule_target(&items, scopes)
    }
}

#[test]
fn empty_invalid_and_scope_targets_leave_native_template_precedence() {
    for target in ["\"\"", "SYN_MISSING", "ROOT", "event_target:SYN_TARGET"] {
        assert_eq!(relation(&format!("add_relation_rule_override={{target={target} trigger=SYN_TRIGGER can_send_volunteers=yes}}")).unwrap(), "Relation Alpha->Other\n:[yes]Volunteers");
    }
}

#[test]
fn reached_mod_target_capital_continent_uses_perspective_localisation() {
    // Workshop1521695605 MAF triggers (Mittelafrika).txt:54; three effect uses.
    let mut scope = EventScope::of(EventObject::Country(1));
    scope.from = Link::Scope(1);
    let graph = EventScopes::new(vec![scope, EventScope::of(EventObject::Country(2))]);
    assert_eq!(
        target("FROM={capital_scope={is_on_continent=SYN_CONT}}", graph).unwrap(),
        "\u{11}YBeta\u{11}!: Capital: On Home [This.GetTag] [This.GetTag]\n"
    );
    assert_eq!(
        target("is_on_continent=SYN_OTHER_CONT", fresh()).unwrap(),
        "On Other continent [This.GetTag]\n"
    );
    assert_eq!(
        target("is_on_continent=SYN_MISSING", fresh()).unwrap_err(),
        "CIsOnContinentTrigger continent SYN_MISSING is absent from recorded content (140AAE420)"
    );
}

#[test]
fn named_target_from_flag_uses_native_scope_name_and_marked_country() {
    // common/scripted_triggers/AST_scripted_triggers.txt:120, reached by AST.txt:646.
    let mut scope = EventScope::of(EventObject::Country(1));
    scope.from = Link::Scope(1);
    let graph = EventScopes::new(vec![scope, EventScope::of(EventObject::Country(2))]);
    assert_eq!(
        target("FROM={has_country_flag=SYN_FLAG}", graph).unwrap(),
        "\u{11}YBeta\u{11}!: \u{12}BBB Beta flag: Flag BBB\n"
    );
}

#[test]
fn mode_two_suppresses_status_but_not_resets_it_and_derives_negative_key() {
    assert_eq!(
        target(
            "custom_trigger_tooltip={tooltip=SYN_TIP always=no}",
            fresh()
        )
        .unwrap(),
        "Tip AAA\n"
    );
    assert_eq!(
        target(
            "NOT={custom_trigger_tooltip={tooltip=SYN_TIP always=no}}",
            fresh()
        )
        .unwrap(),
        "[yes]Inverse AAA\n"
    );
    assert_eq!(
        target(
            "NOT={custom_trigger_tooltip={tooltip=SYN_TIP not_tooltip=SYN_OTHER_TIP always=no}}",
            fresh()
        )
        .unwrap(),
        "[yes]Other\n"
    );
}

#[test]
fn target_context_and_or_keep_native_three_space_indentation() {
    assert_eq!(target("BBB={custom_trigger_tooltip={tooltip=SYN_TIP always=yes} custom_trigger_tooltip={tooltip=SYN_OTHER_TIP always=yes}}", fresh()).unwrap(), "\u{11}YBeta\u{11}!:\n   Tip BBB\n   Other\n");
    assert_eq!(target("OR={custom_trigger_tooltip={tooltip=SYN_TIP always=yes} custom_trigger_tooltip={tooltip=SYN_OTHER_TIP always=no}}", fresh()).unwrap(), "Any:\n   Tip AAA\n   Other\n");
    assert_eq!(target("BBB={AND={custom_trigger_tooltip={tooltip=SYN_TIP always=yes} custom_trigger_tooltip={tooltip=SYN_OTHER_TIP always=yes}} custom_trigger_tooltip={tooltip=SYN_OTHER_TIP always=yes}}", fresh()).unwrap(), "\u{11}YBeta\u{11}!:\n   All:\n      Tip BBB\n      Other\n   Other\n");
}

#[test]
fn unported_target_trigger_fields_are_named() {
    assert_eq!(
        target("has_country_flag={flag=SYN_FLAG days=2}", fresh()).unwrap_err(),
        "CCountryFlagTrigger field days description is not ported (141393260)"
    );
    assert_eq!(
        target("has_country_flag=SYN_FLAG@ROOT", fresh()).unwrap_err(),
        "CCountryFlagTrigger dynamic flag SYN_FLAG@ROOT description is not ported (141390890)"
    );
    assert_eq!(
        target("always=yes", fresh()).unwrap_err(),
        "target trigger always native description is not ported (CTrigger vf21/vf12 14053FCC0)"
    );
}
