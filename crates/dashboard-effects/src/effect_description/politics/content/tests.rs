use super::*;
use crate::effect_description::politics::tests::fixture;
use crate::scripted_text::Link;
use replay_data::{rows::TableId, rows::TableRows, rows::TechSharingGroupRow};
use replay_frame::frame::TableView;
use crate::fixtures::game;

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

#[test]
fn content_description_cases() {
    game::enter();
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for (case, body, expected) in [
        (
            "fixture_finish_focus",
            "fixture_finish_focus=SYN_FOCUS",
            "Complete Alpha:Focus AAA/AAA/AAA/AAA",
        ),
        (
            "fixture_allow_focus",
            "fixture_allow_focus=SYN_FOCUS",
            "Unlock Alpha:Focus AAA/AAA/AAA/AAA",
        ),
        (
            "fixture_reset_focus",
            "fixture_reset_focus={uncomplete_children=yes focus=SYN_FOCUS}",
            "Uncomplete Alpha:Focus AAA/AAA/AAA/AAA",
        ),
        (
            "fixture_join_research_group",
            "fixture_join_research_group=SYN_GROUP",
            "Join AAA/Alpha:Group BBB [Root.GetTag]\n",
        ),
        (
            "fixture_leave_research_group",
            "fixture_leave_research_group=SYN_GROUP",
            "Leave AAA/Alpha:Group BBB [Root.GetTag]\n",
        ),
        (
            "fixture_research_group_bonus",
            "fixture_research_group_bonus={bonus=0.07 id=SYN_GROUP}",
            "Sharing Alpha/Group BBB:7.00% [Root.GetTag]\n",
        ),
        (
            "sharing_bonus_uses_token_fold_and_fixed5_reader",
            "fixture_research_group_bonus={bonus=-0.070009 id=syn_group}",
            "Sharing Alpha/Group BBB:-7.00% [Root.GetTag]\n",
        ),
        (
            "fixture_agency_upgrade",
            "fixture_agency_upgrade=SYN_UPGRADE",
            "Agency Upgrade BBB\n",
        ),
        (
            "fixture_grant_token",
            "fixture_grant_token={tag=FROM token=SYN_TOKEN}",
            "Receive \u{13}SYN_ICON Token CCC/\u{12}BBB Beta\n",
        ),
        (
            "fixture_revoke_token",
            "fixture_revoke_token={tag=FROM token=SYN_TOKEN}",
            "Remove \u{13}SYN_ICON Token CCC/\u{12}BBB Beta\n",
        ),
        (
            "fixture_allow_tactic",
            "fixture_allow_tactic=SYN_TACTIC",
            "Unlock tactic Tactic [Root.GetTag]",
        ),
        (
            "fixture_party_support",
            "fixture_party_support={SYN_IDEOLOGY=SYN_SUPPORT}",
            "Set popularities CCC:\nIdeology [Root.GetTag]: 30.00%\nOther: 0.00%\n",
        ),
        (
            "named_relation_rule_override",
            "fixture_grant_override={fixture_volunteers=yes trigger=SYN_TRIGGER usage_desc=SYN_IGNORED}",
            "Relation Alpha->Eligible BBB\n:[yes]Volunteers",
        ),
        (
            "focus_custom_tooltip",
            "fixture_finish_focus={focus=SYN_FOCUS originator_name=SYN_FOCUS_TOOLTIP}",
            "Complete for BBB/Alpha:Focus AAA/AAA/AAA/AAA",
        ),
        (
            "focus_empty_originator_keeps_bound_node",
            "fixture_finish_focus={focus=SYN_FOCUS originator_name=\"\"}",
            "Complete /Alpha:Focus AAA/AAA/AAA/AAA",
        ),
        (
            "party_first_match_and_database_order",
            "fixture_party_support={SYN_OTHER=61 SYN_IDEOLOGY=23 SYN_IDEOLOGY=84}",
            "Set popularities CCC:\nIdeology [Root.GetTag]: 23.00%\nOther: 61.00%\n",
        ),
    ] {
        let output = fixture.render(body, &mut caller(), content.view(None));
        assert_eq!(output.text, expected, "{case}: {:?}", output.refusals);
        assert!(output.refusals.is_empty(), "{case}: {:?}", output.refusals);
    }
}

#[test]
fn national_focus_effects_ignore_a_refused_continuous_file() {
    game::enter();
    let (fixture, _folder) = crate::effect_description::politics::tests::fixture_with(&[(
        "common/continuous_focus/zz_refused.txt",
        "continuous_focus_palette={focus={text=SYN_WRONG_NAME}}",
    )]);
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for (body, expected) in [
        (
            "fixture_finish_focus=SYN_FOCUS",
            "Complete Alpha:Focus AAA/AAA/AAA/AAA",
        ),
        (
            "fixture_allow_focus=SYN_FOCUS",
            "Unlock Alpha:Focus AAA/AAA/AAA/AAA",
        ),
    ] {
        let output = fixture.render(body, &mut caller(), content.view(None));
        assert_eq!(output.text, expected, "{body}: {:?}", output.refusals);
        assert!(output.refusals.is_empty(), "{body}: {:?}", output.refusals);
    }
}

#[test]
fn sharing_bonus_keeps_empty_gates() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for body in [
        "fixture_research_group_bonus={bonus=0.03}",
        "fixture_research_group_bonus={id=ABSENT bonus=0.03}",
    ] {
        assert_eq!(
            fixture.render(body, &mut caller(), content.view(None)),
            Description::default()
        );
    }
    let body = "fixture_research_group_bonus={bonus=0.07 id=SYN_GROUP}";
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
    game::enter();
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
        "fixture_research_group_bonus={bonus=0.07 id=SYN_GROUP}",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(
        output.refusals.keys().all(|why| {
            why.contains("fixture_research_group_bonus") && why.contains("missing sharing members")
        }),
        "{:?}",
        output.refusals
    );
}

#[test]
fn focus_unknown_tooltip_key_uses_core_child_projection() {
    game::enter();
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "fixture_finish_focus={focus=SYN_FOCUS tooltip=SYN_FOCUS_TOOLTIP}",
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

#[test]
fn focus_bound_originator_is_named_and_counted() {
    game::enter();
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "fixture_finish_focus={focus=SYN_FOCUS originator_name={localization_key=SYN_FOCUS_TOOLTIP}}",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output.text, "");
    assert_eq!(output.refusals.values().sum::<usize>(), 1);
    assert!(output.refusals.keys().all(|why| {
        why.contains("fixture_finish_focus")
            && why.contains("originator_name bound localisation block requires core formatter")
    }));
}

#[test]
fn focus_operands_keep_each_reader_shape() {
    game::enter();
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    for body in [
        "fixture_allow_focus={focus=SYN_FOCUS}",
        "fixture_reset_focus=SYN_FOCUS",
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

#[test]
fn focus_uses_state_owner_and_fresh_seeded_country_scope() {
    game::enter();
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
        "fixture_allow_focus=SYN_FOCUS",
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
    game::enter();
    let (fixture, _folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    let output = fixture.render(
        "fixture_join_research_group=ABSENT fixture_agency_upgrade=ABSENT",
        &mut caller(),
        content.view(None),
    );
    assert_eq!(output, Description::default());
    for body in [
        "fixture_allow_tactic=syn_tactic",
        "fixture_finish_focus=syn_focus",
        "fixture_grant_token={tag=FROM token=ABSENT}",
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
