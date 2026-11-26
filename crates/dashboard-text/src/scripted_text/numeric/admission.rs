
use super::*;
use engine_executable::{
    executable_tables::numeric::NumericIdentity, executable_tables::numeric::NumericProperty,
    executable_tables::numeric::NumericScope, executable_tables::numeric::NumericTables,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PropertyHandler {
    Country,
    State,
    UnitLeader,
    Character,
    Balance,
    Callback,
}

fn property_handler(property: &NumericProperty) -> Result<PropertyHandler, String> {
    Ok(match property.identity {
        NumericIdentity::Scope {
            scope: NumericScope::Country,
            ordinal: ordinal @ 46..=58,
            ..
        } => {
            if !balance::matches_numeric_handler(property.name, ordinal) {
                return Err(format!(
                    "{}: numeric balance property has no matching implemented handler",
                    property.name
                ));
            }
            PropertyHandler::Balance
        }
        NumericIdentity::Scope {
            scope: NumericScope::Country,
            ..
        } => PropertyHandler::Country,
        NumericIdentity::Scope {
            scope: NumericScope::State,
            ..
        } => PropertyHandler::State,
        NumericIdentity::Scope {
            scope: NumericScope::UnitLeader,
            ..
        } => PropertyHandler::UnitLeader,
        NumericIdentity::Scope {
            scope: NumericScope::Character,
            ..
        } => PropertyHandler::Character,
        NumericIdentity::Callback { .. } => PropertyHandler::Callback,
    })
}

fn registered_property(tables: &NumericTables, name: &str) -> Result<bool, String> {
    tables
        .exact(name)
        .map(property_handler)
        .transpose()
        .map(|handler| handler.is_some())
}

pub(super) fn admit<'a>(
    text: &ScriptedText<'a>,
    operand: &str,
    scopes: &ExpansionScopes<'_, 'a>,
    start: usize,
) -> Result<(), String> {
    if let Some((_, suffix)) = operand.rsplit_once('.') {
        let numeric =
            engine_executable::executable_tables::numeric().map_err(|error| error.to_string())?;
        if registered_property(numeric, suffix)?
            || text.content.names.entries.contains_key(suffix.as_bytes())
        {
            return Err(format!(
                "{suffix}: numeric localisation's property dispatch is not reproduced"
            ));
        }
        if !text.content.names.unknown.is_empty() {
            return Err(format!(
                "{suffix}: whether numeric localisation selects a scripted property is unknown: {}",
                text.content.names.unknown.join("; ")
            ));
        }
    }
    let evaluator = scopes.evaluator(text, crate::triggers::Temps::Stale)?;
    let parsed = evaluator
        .operand(operand.as_bytes())
        .map_err(|why| why.to_string())?;
    if parsed.literal.is_some()
        || parsed.token.is_some()
        || parsed.mtth.is_some()
        || parsed.mio.is_some()
        || parsed.special_project.is_some()
        || parsed.unset.is_some()
    {
        return Ok(());
    }
    let original = operand
        .strip_prefix("var:")
        .or_else(|| operand.strip_prefix("temp_var:"))
        .unwrap_or(operand);
    let mut remainder = original;
    for delimiter in ['?', '^'] {
        if let Some(at) = remainder.find(delimiter).filter(|at| *at > 0) {
            remainder = &remainder[..at];
        }
    }
    if let Some(at) = remainder.find('@') {
        remainder = &remainder[..at];
    }
    if let Some((_, rest)) = remainder.split_once(':') {
        remainder = rest;
    }
    let mut parts = remainder.split('.');
    let candidate = parts.next().unwrap_or_default();
    let second = parts.next();
    if parts.next().is_some()
        || candidate.is_empty()
        || (second.is_none() && (parsed.random || parsed.ai_random || parsed.num))
        || (second.is_some()
            && (parsed.country.is_some() || parsed.state.is_some() || parsed.global))
    {
        return Ok(());
    }
    evaluator
        .admit_localisation_parse_character(candidate, entries::entry_scopes(scopes, start, false))
        .map_err(|why| why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::tests::script_game;
    use replay_data::{
        rows::CharacterRow, rows::CountryCharacterRow, rows::LexicalTokenRow, rows::TableId,
        rows::TableRows,
    };

    #[test]
    fn property_admission_uses_runtime_names_and_handler_identity() {
        let tables = NumericTables {
            properties: vec![NumericProperty {
                name: "SyntheticProperty",
                identity: NumericIdentity::Scope {
                    scope: NumericScope::State,
                    ordinal: 1,
                    handler_rva: 1,
                },
            }],
        };
        assert_eq!(registered_property(&tables, "SyntheticProperty"), Ok(true));
        assert_eq!(registered_property(&tables, "syntheticproperty"), Ok(false));
        assert_eq!(registered_property(&tables, "GetName"), Ok(false));
        let wrong_balance = NumericProperty {
            name: "SyntheticProperty",
            identity: NumericIdentity::Scope {
                scope: NumericScope::Country,
                ordinal: 46,
                handler_rva: 1,
            },
        };
        assert!(
            property_handler(&wrong_balance)
                .unwrap_err()
                .contains("matching implemented handler")
        );
    }

    fn fixture() -> (
        crate::catalogue::Catalogue,
        crate::catalogue::tests::Folder,
        DecodedFrame,
    ) {
        let (catalogue, folder, mut frame) = super::super::tests::fixture("", "");
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 500,
                ..CharacterRow::default()
            }]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 500,
                literal: b"attack".to_vec(),
            }]));
        *frame.tables[TableId::CountryCharacters.index()] =
            TableView::Complete(TableRows::CountryCharacters(vec![CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            }]));
        (catalogue, folder, frame)
    }

    #[test]
    fn live_character_grammar_uses_country_rosters_before_temps() {
        script_game::enter();
        let (catalogue, _folder, mut frame) = fixture();
        let event = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        {
            let text = ScriptedText::new(
                &frame,
                &catalogue.localisation,
                catalogue.scripted_content().unwrap(),
                1,
            );
            let scopes = ExpansionScopes::new(&event);
            assert!(
                admit(&text, "temp_var:AtTaCk", &scopes, 0)
                    .unwrap_err()
                    .contains("live character")
            );
            assert!(
                admit(&text, "attack.value", &scopes, 0)
                    .unwrap_err()
                    .contains("live character")
            );
            assert!(admit(&text, "3.5", &scopes, 0).is_ok());
            assert!(admit(&text, "ROOT.attack", &scopes, 0).is_ok());
        }
        *frame.tables[TableId::CountryCharacters.index()] =
            TableView::Complete(TableRows::CountryCharacters(vec![CountryCharacterRow {
                country: 200,
                ordinal: 0,
                character: Some(7),
            }]));
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        assert!(admit(&text, "attack", &ExpansionScopes::new(&event), 0).is_ok());
    }

    #[test]
    fn exact_properties_and_missing_character_census_remain_named() {
        script_game::enter();
        let (catalogue, _folder, mut frame) = fixture();
        let event = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        *frame.tables[TableId::Characters.index()] = TableView::Unavailable {
            reason: "character census failed".into(),
        };
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let scopes = ExpansionScopes::new(&event);
        assert!(
            admit(&text, "ROOT.GetName", &scopes, 0)
                .unwrap_err()
                .contains("property dispatch")
        );
        assert!(
            admit(&text, "foo.GetDateString", &scopes, 0)
                .unwrap_err()
                .contains("property dispatch")
        );
        assert!(
            admit(&text, "attack", &scopes, 0)
                .unwrap_err()
                .contains("character census failed")
        );
        assert!(admit(&text, "3.5", &scopes, 0).is_ok());
    }

    #[test]
    fn original_character_fallback_runs_only_after_temp_admission() {
        script_game::enter();
        let (catalogue, _folder, mut frame) = fixture();
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 500,
                literal: b"target_province?5".to_vec(),
            }]));
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        let mut scopes = crate::triggers::Scopes::of(crate::triggers::Scope::Country(1));
        scopes.target_province = Some(9);
        assert_eq!(
            evaluator.scoped_localisation_value(b"target_province?5", scopes),
            Ok(900_000)
        );
        scopes.target_province = None;
        assert!(
            evaluator
                .scoped_localisation_value(b"target_province?5", scopes)
                .unwrap_err()
                .contains("before stored variables")
        );
        assert_eq!(
            evaluator.scoped_value(b"target_province?5", scopes),
            Ok(500_000)
        );
        drop(evaluator);
        drop(text);
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 500,
                literal: b"ROOT.attack".to_vec(),
            }]));
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        assert!(
            evaluator
                .scoped_localisation_value(b"ROOT.attack", scopes)
                .unwrap_err()
                .contains("before stored variables")
        );
        assert_eq!(evaluator.scoped_value(b"ROOT.attack", scopes), Ok(90_000));
    }

    #[test]
    fn known_empty_character_registry_needs_no_token_dictionary() {
        script_game::enter();
        let (catalogue, _folder, mut frame) = fixture();
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(Vec::new()));
        *frame.tables[TableId::CountryCharacters.index()] =
            TableView::Complete(TableRows::CountryCharacters(Vec::new()));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "dictionary unavailable".into(),
        };
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let event = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        assert!(admit(&text, "attack", &ExpansionScopes::new(&event), 0).is_ok());
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        assert_eq!(
            evaluator.scoped_localisation_value(
                b"ROOT.attack",
                crate::triggers::Scopes::of(crate::triggers::Scope::Country(1))
            ),
            Ok(90_000)
        );
    }

    #[test]
    fn known_country_roster_and_temp_ignore_unrelated_manager_gaps() {
        let (catalogue, _folder, mut frame) = fixture();
        *frame.tables[TableId::Characters.index()] = TableView::Truncated {
            rows: TableRows::Characters(vec![CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 500,
                ..CharacterRow::default()
            }]),
            missing: 10,
        };
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Truncated {
            rows: TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 500,
                literal: b"other_name".to_vec(),
            }]),
            missing: 10,
        };
        let text = ScriptedText::new(
            &frame,
            &catalogue.localisation,
            catalogue.scripted_content().unwrap(),
            1,
        );
        let mut event = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        event.variables.target_province = Some(9);
        let evaluator = crate::triggers::Evaluator::new(text.world().unwrap(), None);
        assert_eq!(
            text.expand_in_session("[?target_province|0]", &event, 0, &evaluator),
            Ok("9".into())
        );
    }
}
