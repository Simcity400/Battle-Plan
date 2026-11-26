
use super::*;

mod admission;

pub(super) fn format_variable(expression: &str, value: i64) -> String {
    let format = expression
        .split_once('|')
        .map(|(_, format)| format.as_bytes());
    content_reader_localisation::localisation::format_scripted_variable(value, format)
}

impl<'a> ScriptedText<'a> {
    pub(super) fn numeric_value(
        &self,
        expression: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        start: usize,
    ) -> Result<i64, String> {
        let operand = expression
            .split_once('|')
            .map_or(expression, |(operand, _)| operand);
        let normalized;
        let operand = if operand.contains('\u{12}') {
            normalized = operand.replacen('\u{12}', "@", 1);
            normalized.as_str()
        } else {
            operand
        };
        let within = |why| format!("[?{expression}]: {why}");
        if operand.ends_with(":days") || operand.ends_with(":days_left") {
            return Err(within(
                "flag-age variable localisation is not reproduced".into(),
            ));
        }
        admission::admit(self, operand, scopes, start).map_err(within)?;
        let evaluator = scopes
            .evaluator(self, crate::triggers::Temps::Stale)
            .map_err(within)?;
        let value = evaluator.scoped_localisation_value(
            operand.as_bytes(),
            entries::entry_scopes(scopes, start, false),
        );
        scopes.session.replace(Some(evaluator.variable_session()));
        value.map_err(|why| within(why.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::tests::script_game;
    use replay_data::{
        rows::ScriptVariableEntry, rows::ScriptVariablesRow, rows::TableId, rows::TableRows,
    };

    pub(super) fn fixture(
        scripted: &str,
        extra: &str,
    ) -> (
        crate::catalogue::Catalogue,
        crate::catalogue::tests::Folder,
        DecodedFrame,
    ) {
        let (catalogue, folder) = super::super::tests::catalogue(scripted, extra);
        let content = catalogue.scripted_content().unwrap();
        content.set_synthetic_environment(Default::default());
        (catalogue, folder, variable_frame())
    }

    fn variable_frame() -> DecodedFrame {
        let mut frame = crate::scripted_text::tests::complete_frame();
        *frame.tables[TableId::ScriptVariables.index()] =
            TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
                scope_kind: 1,
                scope_id: 100,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![
                    ScriptVariableEntry {
                        name: b"attack".to_vec(),
                        value_raw: 90_000,
                    },
                    ScriptVariableEntry {
                        name: b"radios_stored".to_vec(),
                        value_raw: 2_999_999,
                    },
                    ScriptVariableEntry {
                        name: b"targetger".to_vec(),
                        value_raw: 1_200_000,
                    },
                    ScriptVariableEntry {
                        name: b"total_unit".to_vec(),
                        value_raw: 4_000_000,
                    },
                ],
            }]));
        frame
    }

    #[test]
    fn reached_numeric_directives_share_fixed5_formatting() {
        script_game::enter();
        let (catalogue, _folder, frame) = fixture("", " TARGET:0 \"[?target@ROOT|0]\"\n");
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        assert_eq!(
            text.expand_in_session(
                "[?radios_stored|R0]/[?total_unit|R0]",
                &scopes,
                0,
                &evaluator
            )
            .unwrap(),
            "\u{11}R29\u{11}!/\u{11}R40\u{11}!"
        );
        let target = catalogue.localisation.localise(b"TARGET").unwrap();
        assert!(target.contains('\u{12}'));
        assert_eq!(
            text.expand_in_session(&target, &scopes, 0, &evaluator)
                .unwrap(),
            "12"
        );
        assert_eq!(
            text.expand("a[?.05|1+%=]\nb[?-.05999|1-%=]\n").unwrap(),
            "a\u{11}G+5.0%\u{11}!\nb\u{11}G-5.9%\u{11}!\n"
        );
        assert_eq!(
            text.expand("[?1.23999|2]/[?1.23999]/[?1.23999|]").unwrap(),
            "1.23/1.23999/1.23"
        );
        assert_eq!(
            text.expand("[?0|_]/[?0|R_]/[?1.25|1%%=]").unwrap(),
            "0/\u{11}R\u{11}!/+1.2%"
        );
        assert_eq!(
            text.expand("[?-5|=+.0]/[?.05|+3]/[?.05|§=+%]").unwrap(),
            "\u{11}R-5\u{11}!/\u{11}G0.050\u{11}!/\u{11}G+5.00%\u{11}!"
        );
    }

    #[test]
    fn script_entry_numeric_text_retains_temps_and_next_outer_trigger_resets_them() {
        script_game::enter();
        let (catalogue, _folder, frame) = fixture(
            r#"
            defined_text = { name = Attack text = { trigger = { set_temp_variable = { attack = .05 } } localization_key = ATTACK } }
            defined_text = { name = Reset text = { trigger = { check_variable = { temp_var:attack = 0 } } localization_key = RESET } }
            defined_text = { name = EmptyReset text = { trigger = {} localization_key = RESET } }
        "#,
            " ATTACK:0 \"Attack: [?attack|1+%=]\\n\"\n RESET:0 \"reset[?temp_var:attack|0]\"\n",
        );
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        assert_eq!(
            text.expand("[Attack][?attack|1+%=][Reset]").unwrap(),
            "Attack: \u{11}G+5.0%\u{11}!\n\u{11}G+5.0%\u{11}!reset0"
        );
        assert_eq!(
            text.expand("[Attack][EmptyReset]").unwrap(),
            "Attack: \u{11}G+5.0%\u{11}!\nreset0"
        );
    }

    #[test]
    fn random_selection_keeps_literal_numeric_text() {
        script_game::enter();
        let (catalogue, _folder, frame) = fixture(
            r#"
            defined_text = { name = Roll text = { random_list = {
                seed = 1
                1 = { localization_key = RANDOM_TEXT modifier = { factor = 1 set_temp_variable = { bonus = .05 } } }
                0 = { localization_key = WRONG }
            } } }
        "#,
            " RANDOM_TEXT:0 \"[?0.05|=+1%]/[?0.1|=+1%]\"\n WRONG:0 \"wrong\"\n",
        );
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        assert_eq!(
            text.expand("[Roll]").unwrap(),
            "\u{11}G+5.0%\u{11}!/\u{11}G+10.0%\u{11}!"
        );
    }

    #[test]
    fn explicit_random_seed_retains_tag_alias_trigger_temps() {
        script_game::enter();
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "localisation/mod_l_english.yml",
                "\u{feff}l_english:\n RANDOM_TEXT:0 \"[?attack|1+%=]\"\n WRONG:0 \"wrong\"\n",
            ),
            (
                "common/defines/00_defines.lua",
                "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 } }",
            ),
            (
                "common/scripted_localisation/mod.txt",
                "defined_text = { name = Roll text = { random_list = { seed = SEL.id 1 = { localization_key = RANDOM_TEXT } 0 = { localization_key = WRONG } } } }",
            ),
            (
                "common/country_tag_aliases/mod.txt",
                "SEL = { original_tag = GER set_temp_variable = { attack = .05 } }",
            ),
        ]);
        let content = catalogue.scripted_content().unwrap();
        assert!(
            content
                .environment()
                .unwrap()
                .inputs
                .set(Ok(entries::EnvironmentInputs::synthetic()
                    .with_aliases(std::sync::Arc::clone(&content.tag_aliases))))
                .is_ok()
        );
        let frame = variable_frame();
        let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
        assert_eq!(text.expand("[Roll]").unwrap(), "\u{11}G+5.0%\u{11}!");
        assert!(
            text.expand("[?SEL.id|0][?attack|1+%=]")
                .unwrap()
                .ends_with("\u{11}G+5.0%\u{11}!")
        );
    }

    #[test]
    fn missing_runtime_inputs_keep_their_named_refusals() {
        script_game::enter();
        let (catalogue, _folder, mut frame) = fixture("", "");
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        {
            let text = ScriptedText::new(
                &frame,
                &catalogue.localisation,
                catalogue.scripted_content().unwrap(),
                1,
            );
            assert!(
                text.expand("[?attack|0]")
                    .unwrap_err()
                    .contains("thread's temps")
            );
            assert!(
                text.expand("[?flag:days|0]")
                    .unwrap_err()
                    .contains("flag-age variable localisation")
            );
        }
        *frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
            reason: "variable capture failed".into(),
        };
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        assert!(
            text.expand_in_session("[?radios_stored|R0]", &scopes, 0, &evaluator)
                .unwrap_err()
                .contains("variable capture failed")
        );
    }
}
