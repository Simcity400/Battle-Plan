//! War names use content-defined types and overrides;
//! the capture supplies the native default prefix and ordered first goal.
use super::*;
use crate::scripted_text::ScriptedText;
use installed_content::{country_names::CountryNames, localisation::Argument};
use std::cell::OnceCell;

pub(super) fn unknown(reason: impl Into<String>) -> WarName {
    WarName {
        text: "Not recorded".into(),
        known: false,
        reason: Some(reason.into()),
        key: None,
    }
}
/// One frame's naming operands, used with one catalogue. Rows ascend by key,
/// so lookups search them; the first war named builds the shared contexts.
pub(super) struct Inputs<'a> {
    pub goals: Result<&'a [rows::WarRelationGoalRow], String>,
    pub tokens: Result<&'a [rows::LexicalTokenRow], String>,
    pub prefix: Result<&'a str, String>,
    names: OnceCell<CountryNames<'a>>,
    /// Plain state names read the frame and content, never the context's root.
    states: OnceCell<ScriptedText<'a>>,
}
impl<'a> Inputs<'a> {
    pub fn read(frame: &'a DecodedFrame) -> Self {
        Self {
            goals: frame
                .complete::<tables::WarRelationGoals>()
                .map_err(|e| e.to_string()),
            tokens: frame
                .complete::<tables::LexicalTokens>()
                .map_err(|e| e.to_string()),
            prefix: frame
                .complete::<tables::WarNameRules>()
                .map_err(|e| e.to_string())
                .and_then(|rows| match rows {
                    [row] if row.index == 0 => std::str::from_utf8(&row.prefix)
                        .map_err(|_| "native war naming prefix is not UTF-8".into()),
                    _ => Err("native war naming prefix is missing".into()),
                }),
            names: OnceCell::new(),
            states: OnceCell::new(),
        }
    }
}
pub(super) fn name<'a>(
    frame: &'a DecodedFrame,
    hour: &Hour,
    p: Pair,
    cat: Option<&'a Catalogue>,
    inputs: &Inputs<'a>,
) -> WarName {
    let built = (|| -> Result<(String, String), String> {
        let cat = cat.ok_or("Not recorded: the recorded content stack is not resolved")?;
        let localisation = &cat.localisation;
        let names = inputs
            .names
            .get_or_init(|| CountryNames::read(frame, localisation));
        let relation = &hour.world.relations[&p];
        let (instigator, defender, side) = if relation.first_was_instigator != 0 {
            (relation.first_country, relation.second_country, 0)
        } else {
            (relation.second_country, relation.first_country, 1)
        };
        let owned = hour
            .goal_owned
            .get(&p)
            .ok_or("Not recorded: representative's owned goal list")?;
        let count = if side == 0 {
            owned.first_goal_count
        } else {
            owned.second_goal_count
        };
        let goal = if count == 0 {
            None
        } else {
            let goals = inputs.goals.as_ref().map_err(Clone::clone)?;
            let first = goals
                .binary_search_by_key(&(owned.owner, owned.ordinal, side, 0), |r| {
                    (r.owner, r.relation_ordinal, r.side, r.ordinal)
                })
                .map_err(|_| "Not recorded: instigator's first war goal")?;
            Some(&goals[first])
        };
        let goal = goal.filter(|r| r.resolved != 0);
        let mut prefix = inputs.prefix.clone()?;
        if let Some(goal) = goal {
            let tokens = inputs.tokens.as_ref().map_err(Clone::clone)?;
            let token = tokens
                .binary_search_by_key(&goal.type_token, |r| r.token)
                .map_err(|_| "Not recorded: war goal type token")?;
            let token = std::str::from_utf8(&tokens[token].literal)
                .map_err(|_| "war goal type is not UTF-8")?;
            let override_key = cat.war_names()?.prefix(token)?;
            if !override_key.is_empty() {
                prefix = override_key;
            }
        }
        let first_tag = &hour.tags[&instigator];
        let second_tag = &hour.tags[&defender];
        let pair_key = format!("{prefix}_{first_tag}_{second_tag}");
        let allied = |target: u32| -> Result<bool, String> {
            // Check self, the war veto,
            // nonnull faction equality, then either cached subject endpoint.
            if hour.view == target {
                return Ok(true);
            }
            if hour.world.active(hour.view, target) {
                return Ok(false);
            }
            if hour
                .world
                .factions
                .get(&hour.view)
                .is_some_and(|f| *f != 0 && hour.world.factions.get(&target) == Some(f))
            {
                return Ok(true);
            }
            if hour.recorded_view != Some(hour.view) {
                return Err("Not recorded: cached subject naming predicate for the selected viewpoint (only the recording client's is captured)".into());
            }
            hour.world
                .scores
                .get(&target)
                .filter(|s| s.score_available == 1)
                .map(|s| s.view_subject != 0)
                .ok_or_else(|| "Not recorded: cached subject naming predicate".into())
        };
        let suffix = if allied(instigator)? {
            true
        } else {
            allied(defender)?
        };
        // Both alliance branches append the defender tag.
        let extra = format!("{pair_key}_{second_tag}");
        let key = if suffix && localisation.get(&extra).is_some() {
            extra
        } else if localisation.get(&pair_key).is_some() {
            pair_key
        } else {
            prefix.to_owned()
        };
        let first = names.name_with(instigator, "_ADJ")?;
        let second = names.name_with(defender, "_ADJ")?;
        let first_country = names.name(instigator)?;
        let second_country = names.name(defender)?;
        let mut state = String::new();
        let mut country = None;
        if let Some(goal) = goal {
            if goal.state_count != 0 {
                let scripted = cat.scripted_content()?;
                state = inputs
                    .states
                    .get_or_init(|| ScriptedText::new(frame, localisation, scripted, hour.view))
                    .plain_state_name(goal.first_state)?;
            }
            if goal.country_count != 0 {
                // Reference zero selects the reserved country without a link.
                let slot = if goal.first_country == 0 {
                    0
                } else {
                    *hour
                        .raw_to_slot
                        .get(&goal.first_country)
                        .ok_or("Not recorded: first goal's country naming link")?
                };
                country = Some((names.name(slot)?, names.name_with(slot, "_ADJ")?));
            }
        }
        let mut arguments = vec![
            ("FIRST", Argument::Text(&first)),
            ("SECOND", Argument::Text(&second)),
            ("FIRST_COUNTRY", Argument::Text(&first_country)),
            ("SECOND_COUNTRY", Argument::Text(&second_country)),
            ("ORDER", Argument::Text("")),
        ];
        // The recorded naming rule supplies no ordinal suffix.
        if goal.is_some() {
            arguments.push(("STATE", Argument::Text(&state)));
        }
        if let Some((name, adj)) = &country {
            arguments.push(("COUNTRY", Argument::Text(name)));
            arguments.push(("COUNTRY_ADJ", Argument::Text(adj)));
        }
        let text =
            catalogue::localised_with(localisation, &key, &arguments, catalogue::View::Plain)?;
        Ok((text, key))
    })();
    match built {
        Ok((text, key)) => WarName {
            text,
            known: true,
            reason: None,
            key: Some(key),
        },
        Err(reason) => unknown(reason),
    }
}
