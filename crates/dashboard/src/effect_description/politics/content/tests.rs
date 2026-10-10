use super::*;
use crate::effect_description::politics::tests::fixture;
use crate::scripted_text::Link;
use replay::{
    frame::TableView,
    rows::{TableId, TableRows, TechSharingGroupRow},
};

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

macro_rules! description_case {
    ($name:ident, $body:expr, $expected:expr) => {
        #[test]
        fn $name() {
            engine::executable_tables::initialize_for_tests().expect(
                "BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests",
            );
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

description_case!(
    complete_national_focus,
    "complete_national_focus=SYN_FOCUS",
    "Complete Alpha:Focus AAA/AAA/AAA/AAA"
);
description_case!(
    unlock_national_focus,
    "unlock_national_focus=SYN_FOCUS",
    "Unlock Alpha:Focus AAA/AAA/AAA/AAA"
);
description_case!(
    uncomplete_national_focus,
    "uncomplete_national_focus={uncomplete_children=yes focus=SYN_FOCUS}",
    "Uncomplete Alpha:Focus AAA/AAA/AAA/AAA"
);
description_case!(
    add_to_tech_sharing_group,
    "add_to_tech_sharing_group=SYN_GROUP",
    "Join AAA/Alpha:Group BBB [Root.GetTag]\n"
);
description_case!(
    remove_from_tech_sharing_group,
    "remove_from_tech_sharing_group=SYN_GROUP",
    "Leave AAA/Alpha:Group BBB [Root.GetTag]\n"
);

description_case!(
    modify_tech_sharing_bonus,
    "modify_tech_sharing_bonus={bonus=0.07 id=SYN_GROUP}",
    "Sharing Alpha/Group BBB:7.00% [Root.GetTag]\n"
);
description_case!(
    sharing_bonus_uses_token_fold_and_fixed5_reader,
    "modify_tech_sharing_bonus={bonus=-0.070009 id=syn_group}",
    "Sharing Alpha/Group BBB:-7.00% [Root.GetTag]\n"
);

#[test]
fn sharing_bonus_keeps_empty_gates() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for body in [
        "modify_tech_sharing_bonus={bonus=0.03}",
        "modify_tech_sharing_bonus={id=ABSENT bonus=0.03}",
    ] {
        assert_eq!(
            fixture.render(body, &mut caller(), content.view(None)),
            Description::default()
        );
    }
    let body = "modify_tech_sharing_bonus={bonus=0.07 id=SYN_GROUP}";
    for object in [
        EventObject::Country(0),
        EventObject::Country(2),
        EventObject::State(2),
    ] {
        let mut scopes = caller();
        scopes.scopes[0].this = object;
        assert_eq!(
            fixture.render(body, &mut scopes, content.view(None)),
            Description::default()
        );
    }
    *fixture.frame.tables[TableId::TechSharingMembers.index()] = TableView::Unavailable {
        reason: "no group needs no members".into(),
    };
    for groups in [
        vec![],
        vec![TechSharingGroupRow {
            id: 0x10010,
            ..Default::default()
        }],
    ] {
        *fixture.frame.tables[TableId::TechSharingGroups.index()] =
            TableView::Complete(TableRows::TechSharingGroups(groups));
        assert_eq!(
            fixture.render(body, &mut caller(), content.view(None)),
            Description::default()
        );
    }
}

#[test]
fn sharing_bonus_counts_missing_recorded_membership() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::TechSharingMembers.index()] = TableView::Unavailable {
        reason: "missing sharing members".into(),
    };
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "modify_tech_sharing_bonus={bonus=0.07 id=SYN_GROUP}",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(
        output.refusals.keys().all(|why| {
            why.contains("modify_tech_sharing_bonus") && why.contains("missing sharing members")
        }),
        "{:?}",
        output.refusals
    );
}

description_case!(
    upgrade_intelligence_agency,
    "upgrade_intelligence_agency=SYN_UPGRADE",
    "Agency Upgrade BBB\n"
);
description_case!(
    add_operation_token,
    "add_operation_token={tag=FROM token=SYN_TOKEN}",
    "Receive \u{13}SYN_ICON Token CCC/\u{12}BBB Beta\n"
);
description_case!(
    remove_operation_token,
    "remove_operation_token={tag=FROM token=SYN_TOKEN}",
    "Remove \u{13}SYN_ICON Token CCC/\u{12}BBB Beta\n"
);
description_case!(
    unlock_tactic,
    "unlock_tactic=SYN_TACTIC",
    "Unlock tactic Tactic [Root.GetTag]"
);
description_case!(
    set_popularities,
    "set_popularities={SYN_IDEOLOGY=SYN_SUPPORT}",
    "Set popularities CCC:\nIdeology [Root.GetTag]: 30.00%\nOther: 0.00%\n"
);
description_case!(
    named_relation_rule_override,
    "add_relation_rule_override={can_send_volunteers=yes trigger=SYN_TRIGGER usage_desc=SYN_IGNORED}",
    "Relation Alpha->Eligible BBB\n:[yes]Volunteers"
);

description_case!(
    focus_custom_tooltip,
    "complete_national_focus={focus=SYN_FOCUS originator_name=SYN_FOCUS_TOOLTIP}",
    "Complete for BBB/Alpha:Focus AAA/AAA/AAA/AAA"
);
#[test]
fn focus_unknown_tooltip_key_uses_core_child_projection() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "complete_national_focus={focus=SYN_FOCUS tooltip=SYN_FOCUS_TOOLTIP}",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output.text, "Complete Alpha:Focus AAA/AAA/AAA/AAA");
    assert_eq!(
        output.refusals,
        BTreeMap::from([(
            "synthetic effect tooltip: key tooltip: unknown effect key tooltip".to_owned(),
            1,
        )])
    );
}
// Scalar empty still constructs one bound node.
description_case!(
    focus_empty_originator_keeps_bound_node,
    "complete_national_focus={focus=SYN_FOCUS originator_name=\"\"}",
    "Complete /Alpha:Focus AAA/AAA/AAA/AAA"
);

#[test]
fn focus_bound_originator_is_named_and_counted() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "complete_national_focus={focus=SYN_FOCUS originator_name={localization_key=SYN_FOCUS_TOOLTIP}}",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(output.refusals.keys().all(|why| {
        why.contains("complete_national_focus")
            && why.contains("originator_name bound localisation block requires core formatter")
    }));
}

#[test]
fn focus_operands_keep_each_reader_shape() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for body in [
        "unlock_national_focus={focus=SYN_FOCUS}",
        "uncomplete_national_focus=SYN_FOCUS",
    ] {
        let output = fixture.render(body, &mut caller(), content.view(None));
        assert_eq!(output.text, "");
        assert_eq!(output.refusals.values().sum::<usize>(), 1);
        assert!(
            output
                .refusals
                .keys()
                .all(|why| { why.contains("national focus  is absent from recorded content") })
        );
    }
}
description_case!(
    party_first_match_and_database_order,
    "set_popularities={SYN_OTHER=61 SYN_IDEOLOGY=23 SYN_IDEOLOGY=84}",
    "Set popularities CCC:\nIdeology [Root.GetTag]: 23.00%\nOther: 61.00%\n"
);

#[test]
fn focus_uses_state_owner_and_fresh_seeded_country_scope() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
fn absent_catalogue_entries_keep_empty_gates_and_count_missing_names() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
