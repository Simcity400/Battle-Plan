use super::*;
use replay_data::rows::*;

pub fn fixture(
    bop: Option<&str>,
) -> (
    crate::catalogue::Catalogue,
    crate::catalogue::tests::Folder,
    DecodedFrame,
) {
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/country_tags/synthetic.txt",
            "GER=\"countries/synthetic_ger.txt\" SOV=\"countries/synthetic_sov.txt\"",
        ),
        (
            "common/defines/test.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        ("common/units/synthetic.txt", "sub_units={syn_unit={}}"),
        (
            "common/bop/test.txt",
            bop.unwrap_or(
                r#"balance = {
                side = { id = side range = { id = active
                    modifier = { second = 1 first = 0.1 hidden_modifier = { hidden = 1 }
                        added_modifier = { first = 9 } custom_modifier_tooltip = TIP }
                    modifier = { first = 0.25 }
                    rule = { can_create_factions = yes can_join_factions = no
                        can_send_volunteers = no desc = IGNORED override = { 12 = IGNORED } }
                    on_activate = { custom_effect_tooltip = ONE custom_effect_tooltip = ACTIVATION
                        hidden_effect = { custom_effect_tooltip = ACTIVATION }
                        effect_tooltip = { custom_effect_tooltip = NEXT } }
                    on_deactivate = { custom_effect_tooltip = DEACTIVATION }
                } }
            }"#,
            ),
        ),
        (
            "common/scripted_localisation/synthetic.txt",
            "entry={name=BalanceEntry text={localization_key=ENTRY_BODY}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " GER:0 \"Country\"\n FIRST:0 \"First\"\n SECOND:0 \"Second\"\n",
                " TIP:0 \"[ROOT.GetTag] tip\"\n PROGRESS_SECTION_MODIFIER:0 \"Modifiers:\\n$DESC$\"\n",
                " PROGRESS_SECTION_RULE:0 \"Rules:\\n$DESC$\"\n",
                " TRIGGER_FULLFILLED_PREFIX:0 \"Yes: \"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"No: \"\n",
                " CAN_CREATE_FACTIONS:0 \"Create factions\"\n CAN_JOIN_FACTIONS:0 \"Join factions\"\n",
                " ACTIVATION:0 \"[ROOT.GetTag] activated\"\n NEXT:0 \"Next effect\"\n",
                " ONE:0 \"x\"\n",
                " syn_unit:0 \"Unit\"\n SUBUNIT_BONUS_HEADER:0 \"$ICON$$NAME$\"\n",
                " LOOP:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n ENTRY_BODY:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n ENTRY_TOOLTIP:0 \"[BalanceEntry]\"\n",
                " SOV:0 \"Other\"\n EFFECT_CONTEXT_COUNTRY:0 \"@$TAG$$COUNTRY$\"\n",
                " POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n",
                " EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
                " DEACTIVATION:0 \"[ROOT.GetTag] deactivated\"\n",
                " PROGRESS_SECTION_ACTIVATION_EFFECT:0 \"Activate:\\n$DESC$\"\n",
                " PROGRESS_SECTION_DEACTIVATION_EFFECT:0 \"Deactivate:\\n$DESC$\"\n"
            ),
        ),
    ]);
    let mut frame = crate::scope::tests::positive_frame();
    *frame.tables[TableId::PowerBalances.index()] =
        TableView::Complete(TableRows::PowerBalances(vec![PowerBalanceRow {
            balance: b"balance".to_vec(),
            active_range: b"active".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::PowerBalanceMembers(vec![
            PowerBalanceMemberRow {
                country: 100,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        ["first", "second", "hidden"]
            .into_iter()
            .enumerate()
            .map(|(at, key)| LexicalTokenRow {
                token: at as u32 + 1,
                literal: key.as_bytes().to_vec(),
            })
            .collect(),
    ));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            ["FIRST", "SECOND", "HIDDEN"]
                .into_iter()
                .enumerate()
                .map(|(at, key)| ModifierDefinitionRow {
                    index: at as u32 + 1,
                    token: at as u32 + 1,
                    name_key: key.as_bytes().to_vec(),
                    category_mask: u32::MAX,
                    precision: 2,
                    flags: 3,
                    ..Default::default()
                })
                .collect(),
        ));
    (catalogue, folder, frame)
}

pub fn description(
    catalogue: &crate::catalogue::Catalogue,
    frame: &DecodedFrame,
    property: &str,
) -> Result<String, String> {
    let text = ScriptedText::new(
        frame,
        &catalogue.localisation,
        catalogue.scripted_content()?,
        1,
    );
    crate::catalogue::localised_in(
        &catalogue.localisation,
        &format!("[ROOT.{property}]"),
        &[],
        &text,
    )
}

#[cfg(test)]
#[test]
fn active_range_mod_description() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(None);
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeModDesc").unwrap(),
        "Modifiers:\nFirst: +25.00%\nSecond: +100.00%\nGER tip\n"
    );
}

#[cfg(test)]
#[test]
fn active_range_modifier_feature_gate() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, mut frame) = fixture(None);
    for (mask, eligibility, shown) in [
        (0b1010, 1, true),
        (0b0010, 1, true),
        (0b1000, 1, true),
        (0b0001, 1, false),
        (0, 1, false),
        (0b1010, 0, false),
    ] {
        if let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        {
            rows[0].feature_selector = 0b1010;
            rows[0].eligibility = eligibility;
        }
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: mask,
                ..Default::default()
            }]));
        assert_eq!(
            description(&catalogue, &frame, "GetActiveRangeModDesc").unwrap(),
            if shown {
                "Modifiers:\nFirst: +25.00%\nSecond: +100.00%\nGER tip\n"
            } else {
                "Modifiers:\nSecond: +100.00%\nGER tip\n"
            },
            "mask {mask:#x}, eligibility {eligibility}"
        );
    }
}

#[cfg(test)]
#[test]
fn active_range_rule_description() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(None);
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeRuleDesc").unwrap(),
        "Rules:\nYes: Create factions\nNo: Join factions\n"
    );
}

pub const MAX_SCRIPTED_NESTING: usize = super::super::MAX_SCRIPTED_NESTING;

pub struct BalanceEffectScopes<'a>(ExpansionScopes<'a, 'a>);

impl<'a> BalanceEffectScopes<'a> {
    pub fn new(graph: &EventScopes, depth: usize) -> Self {
        let mut scopes = ExpansionScopes::new(graph);
        scopes.on_large_stack = true;
        scopes.fresh_depth.set(depth);
        Self(scopes)
    }

    pub fn set_depth(&self, depth: usize) {
        self.0.fresh_depth.set(depth);
    }

    pub fn depth(&self) -> usize {
        self.0.fresh_depth.get()
    }

    pub fn check_appended(&self, graph: &EventScopes, depth: usize) {
        let scopes = &self.0;
        {
            let appended = scopes.append_description(graph).unwrap();
            assert_eq!(appended.offset, 1);
            assert_eq!(scopes.scope(0), graph.scopes[0]);
            assert_eq!(scopes.scope(1), graph.scopes[0]);
            assert_eq!(scopes.fresh_depth.get(), depth);
            assert!(scopes.on_large_stack);
        }
        assert_eq!(scopes.scopes.borrow().len(), 1);
    }

    pub fn describe(
        &self,
        text: &ScriptedText<'a>,
        effects: &[content_reader_parser::parser::Item],
        refused: &mut Vec<String>,
        slot: u32,
        name: &str,
    ) -> Result<String, String> {
        text.active_range_effects(effects, refused, slot, name, &self.0)
    }
}
