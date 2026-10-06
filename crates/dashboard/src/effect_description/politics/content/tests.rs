use super::*;
use crate::effect_description::politics::tests::fixture;
use crate::scripted_text::Link;

fn caller() -> EventScopes {
    let mut scope = EventScope::of(EventObject::Country(1));
    scope.from = Link::Scope(1);
    scope.root = Link::Scope(2);
    scope.prev = Link::Scope(1);
    EventScopes::new(vec![
        scope,
        EventScope::of(EventObject::Country(2)),
        EventScope::of(EventObject::Country(3)),
    ])
}

macro_rules! ported {
    ($name:ident, $body:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let (fixture, _folder) = fixture();
            let content = fixture
                .catalogue
                .scripted_content()
                .unwrap()
                .effects()
                .unwrap();
            let output = fixture.render($body, &mut caller(), content.view(None));
            assert_eq!(output.text, $expected, "{:?}", output.refusals);
            assert!(output.refusals.is_empty(), "{:?}", output.refusals);
        }
    };
}

// Witness forms: BUL.txt:3985, baltic.txt:101, generic.txt:455/459,
// on_actions/07_nsb_on_actions.txt:763, afghanistan.txt:1926,
// operations/00_operations.txt:248/1965 and czechoslovakia_mu.txt:21424.
ported!(
    complete_national_focus,
    "complete_national_focus=SYN_FOCUS",
    "Complete Alpha:Focus AAA/AAA/AAA/AAA"
);
ported!(
    unlock_national_focus,
    "unlock_national_focus=SYN_FOCUS",
    "Unlock Alpha:Focus AAA/AAA/AAA/AAA"
);
ported!(
    uncomplete_national_focus,
    "uncomplete_national_focus={focus=SYN_FOCUS uncomplete_children=yes}",
    "Uncomplete Alpha:Focus AAA/AAA/AAA/AAA"
);
ported!(
    add_to_tech_sharing_group,
    "add_to_tech_sharing_group=SYN_GROUP",
    "Join AAA/Alpha:Group BBB [Root.GetTag]\n"
);
ported!(
    remove_from_tech_sharing_group,
    "remove_from_tech_sharing_group=SYN_GROUP",
    "Leave AAA/Alpha:Group BBB [Root.GetTag]\n"
);
ported!(
    upgrade_intelligence_agency,
    "upgrade_intelligence_agency=SYN_UPGRADE",
    "Agency Upgrade BBB\n"
);
ported!(
    add_operation_token,
    "add_operation_token={tag=FROM token=SYN_TOKEN}",
    "Receive \u{13}SYN_ICON Token CCC/\u{12}BBB Beta\n"
);
ported!(
    remove_operation_token,
    "remove_operation_token={tag=FROM token=SYN_TOKEN}",
    "Remove \u{13}SYN_ICON Token CCC/\u{12}BBB Beta\n"
);
ported!(
    unlock_tactic,
    "unlock_tactic=SYN_TACTIC",
    "Unlock tactic Tactic [Root.GetTag]"
);
ported!(
    set_popularities,
    "set_popularities={SYN_IDEOLOGY=SYN_SUPPORT}",
    "Set popularities CCC:\nIdeology [Root.GetTag]: 30.00%\nOther: 0.00%\n"
);
ported!(
    named_relation_rule_override,
    "add_relation_rule_override={can_send_volunteers=yes trigger=SYN_TRIGGER usage_desc=SYN_IGNORED}",
    "Relation Alpha->Eligible BBB\n:[yes]Volunteers"
);

ported!(
    focus_custom_tooltip,
    "complete_national_focus={focus=SYN_FOCUS tooltip=SYN_FOCUS_TOOLTIP}",
    "Complete for BBB/Alpha:Focus AAA/AAA/AAA/AAA"
);
ported!(
    party_first_match_and_database_order,
    "set_popularities={SYN_OTHER=70 SYN_IDEOLOGY=20 SYN_IDEOLOGY=90}",
    "Set popularities CCC:\nIdeology [Root.GetTag]: 20.00%\nOther: 70.00%\n"
);

#[test]
fn focus_uses_state_owner_and_fresh_seeded_country_scope() {
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let mut scopes = caller();
    scopes.scopes[0].this = EventObject::State(2);
    let output = fixture.render(
        "unlock_national_focus=SYN_FOCUS",
        &mut scopes,
        content.view(None),
    );
    assert_eq!(output.text, "Unlock Alpha:Focus AAA/AAA/AAA/AAA");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let added = scopes.scopes.last().unwrap();
    assert_eq!(added.root, Link::Itself);
    assert_eq!(added.from, Link::Itself);
    assert_eq!(added.prev, Link::Itself);
    assert_eq!(added.random_seed, 0xbf58_4897);
    assert_eq!(added.random_counter, 0xa827_4988);
}

#[test]
fn absent_catalogue_entries_keep_native_empty_gates_and_count_missing_names() {
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "add_to_tech_sharing_group=ABSENT upgrade_intelligence_agency=ABSENT",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output, Description::default());
    for body in [
        "unlock_tactic=syn_tactic",
        "complete_national_focus=syn_focus",
        "add_operation_token={tag=FROM token=ABSENT}",
    ] {
        let output = fixture.render(body, &mut caller(), content.view(None));
        assert_eq!(output.text, "");
        assert_eq!(output.refusals.values().sum::<usize>(), 1);
        assert!(
            output
                .refusals
                .keys()
                .all(|why| why.contains("absent from recorded content")),
            "{body}: {:?}",
            output.refusals
        );
    }
}
