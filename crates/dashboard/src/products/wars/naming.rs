//! CWarRelation::BuildWarName@140D0BCA0. Content owns types and overrides;
//! the capture supplies the native default prefix and ordered first goal.
use super::*;
use installed_content::{country_names::CountryNames, localisation::Argument};

pub(super) fn unknown(reason: impl Into<String>) -> WarName {
    WarName {
        text: "Not recorded".into(),
        known: false,
        reason: Some(reason.into()),
        key: None,
    }
}
pub(super) struct Inputs<'a> {
    pub goals: Result<HashMap<(u32, u32, u32), &'a rows::WarRelationGoalRow>, String>,
    pub tokens: Result<HashMap<u32, &'a [u8]>, String>,
    pub prefix: Result<&'a str, String>,
    pub names: Option<CountryNames<'a>>,
}
impl<'a> Inputs<'a> {
    pub fn read(frame: &'a DecodedFrame, cat: Option<&'a Catalogue>) -> Self {
        Self {
            goals: frame
                .complete::<tables::WarRelationGoals>()
                .map(|rows| {
                    rows.iter()
                        .filter(|r| r.ordinal == 0)
                        .map(|r| ((r.owner, r.relation_ordinal, r.side), r))
                        .collect()
                })
                .map_err(|e| e.to_string()),
            tokens: frame
                .complete::<tables::LexicalTokens>()
                .map(|rows| {
                    rows.iter()
                        .map(|r| (r.token, r.literal.as_slice()))
                        .collect()
                })
                .map_err(|e| e.to_string()),
            prefix: frame
                .complete::<tables::WarNameRules>()
                .map_err(|e| e.to_string())
                .and_then(|rows| match rows {
                    [row] if row.index == 0 => std::str::from_utf8(&row.prefix)
                        .map_err(|_| "native war naming prefix is not UTF-8".into()),
                    _ => Err("native war naming prefix is missing".into()),
                }),
            names: cat.map(|c| CountryNames::read(frame, &c.localisation)),
        }
    }
}
pub(super) fn name(
    frame: &DecodedFrame,
    hour: &Hour,
    p: Pair,
    cat: Option<&Catalogue>,
    inputs: &Inputs<'_>,
) -> WarName {
    let built = (|| -> Result<(String, String), String> {
        let cat = cat.ok_or("Not recorded: the recorded content stack is not resolved")?;
        let localisation = &cat.localisation;
        let names = inputs
            .names
            .as_ref()
            .ok_or("Not recorded: country naming operands")?;
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
            Some(
                *inputs
                    .goals
                    .as_ref()
                    .map_err(Clone::clone)?
                    .get(&(owned.owner, owned.ordinal, side))
                    .ok_or("Not recorded: instigator's first war goal")?,
            )
        };
        let goal = goal.filter(|r| r.resolved != 0);
        let mut prefix = inputs.prefix.clone()?;
        if let Some(goal) = goal {
            let token = std::str::from_utf8(
                inputs
                    .tokens
                    .as_ref()
                    .map_err(Clone::clone)?
                    .get(&goal.type_token)
                    .ok_or("Not recorded: war goal type token")?,
            )
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
            // IsAlliedTo@1406F3A90(local,target,0,0): self, war veto,
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
        // Both native branches append the DEFENDER tag (140D0C065..C150).
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
                state = crate::scripted_text::ScriptedText::new(
                    frame,
                    localisation,
                    scripted,
                    hour.view,
                )
                .plain_state_name(goal.first_state)?;
            }
            if goal.country_count != 0 {
                // 140D0BCA0 calls 140BA5960/140BA5560; 140BA56A0 resolves
                // reference zero to the reserved CCountry[0], without a link.
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
        // ReturnZero@14011CD10 makes the collision loop stop on pass one.
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
