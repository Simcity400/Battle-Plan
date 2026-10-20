//! Current authored checks and reward descriptions, independent of held caches.
use super::*;
use crate::{
    scripted_text::{EventObject, EventScope, EventScopes},
    triggers::{Eval, Scope, Scopes},
};
fn roots(frame: &DecodedFrame, faction: u64) -> (Result<u32, String>, Result<u32, String>) {
    let link = |country| {
        crate::metadata::link_slot(
            frame
                .complete::<tables::CountryLinks>()
                .map_err(|why| why.to_string())?,
            country,
        )
        .ok_or_else(|| "Goal country link unavailable".to_owned())
    };
    let leader = (|| {
        let country = frame
            .complete::<tables::FactionMembers>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|member| member.faction == faction && member.ordinal == 0)
            .ok_or("Goal checks require the faction's first member")?
            .country;
        link(country)
    })();
    let selected = (|| {
        let country = frame
            .complete::<tables::LocalPlayer>()
            .map_err(|why| why.to_string())?
            .first()
            .and_then(|row| row.viewing_country())
            .ok_or("Goal UI checks require the selected country")?;
        link(country)
    })();
    (leader, selected)
}
// The command creates a fresh leader scope, then binds the acting country as FROM.
fn command_scopes(leader: u32, actor: u32) -> Scopes {
    Scopes {
        from: Some(Scope::Country(actor)),
        bare: true,
        ..Scopes::of(Scope::Country(leader))
    }
}
pub(super) fn attach(
    frame: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
    definition: &installed_content_core::faction_goals::Definition,
    reply: &mut FactionGoalDefinition,
) {
    let roots = roots(frame, faction);
    // Keep authored operands distinct from the held scalar/section cache.
    for input in &definition.progress_inputs {
        let kind = String::from_utf8_lossy(input.key.as_deref().unwrap_or_default()).into_owned();
        let mut value = FactionRewardDetail {
            kind: kind.clone(),
            text: String::new(),
            reasons: Vec::new(),
        };
        if matches!(
            kind.as_str(),
            "min" | "max" | "total_amount" | "completed_amount"
        ) {
            if let Some(scalar) = input.value.scalar() {
                if std::str::from_utf8(scalar)
                    .ok()
                    .is_some_and(|text| text.parse::<f64>().is_ok_and(f64::is_finite))
                {
                    value.text = format!("Authored constant: {}", String::from_utf8_lossy(scalar));
                } else {
                    value.reasons.push(format!("goal.ratio_progress.{kind}: current scripted operand evaluation is unproved; held progress remains available"));
                }
            }
        } else {
            value.reasons.push(format!(
                "goal.ratio_progress.{kind}: this reached input has no exact playback evaluator"
            ));
        }
        reply.progress.push(value);
    }
    for (index, section) in definition.progress_sections.iter().enumerate() {
        let mut value = FactionRewardDetail {
            kind: format!("Section {index}"),
            text: String::new(),
            reasons: Vec::new(),
        };
        for field in section {
            let kind =
                String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()).into_owned();
            if matches!(kind.as_str(), "min" | "max") {
                if let Some(scalar) = field.value.scalar() {
                    value
                        .text
                        .push_str(&format!("{kind}: {}\n", String::from_utf8_lossy(scalar)));
                }
            } else if matches!(kind.as_str(), "on_activate" | "on_deactivate") {
                // Effect rendering below uses the same leader scope as goal effects.
                let mut temporary = installed_content_core::faction_goals::Definition::default();
                temporary.effects.insert(
                    kind.clone(),
                    field.value.items().unwrap_or_default().to_vec(),
                );
                let mut rendered = FactionGoalDefinition {
                    icon: None,
                    progress: Vec::new(),
                    sections: Vec::new(),
                    conditions: Vec::new(),
                    rewards: Vec::new(),
                    flags: Vec::new(),
                    token: None,
                    name: None,
                    description: None,
                    category: None,
                    category_name: None,
                    reason: None,
                };
                attach(frame, faction, cat, &temporary, &mut rendered);
                for effect in rendered.rewards {
                    value.text.push_str(&format!("{kind}: {}\n", effect.text));
                    value.reasons.extend(effect.reasons);
                }
            } else if matches!(kind.as_str(), "modifier" | "rule") {
                value.text.push_str(&format!("Authored {kind}: {} fields; applied sources follow the held active-section cache.\n",field.value.items().unwrap_or_default().len()));
            } else {
                value.reasons.push(format!("goal.progress_sections[{index}].{kind}: held modifier/rule cache ownership is unproved"));
            }
        }
        reply.sections.push(value);
    }
    for kind in ["allowed", "visible", "available", "completed", "cancel"] {
        let body = definition
            .conditions
            .get(kind)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut detail = FactionConditionDetail {
            kind: kind.into(),
            result: None,
            reasons: Vec::new(),
        };
        if body.is_empty() {
            detail.result = Some(!matches!(kind, "completed" | "cancel"));
        } else {
            let result = (|| -> Result<Eval, String> {
                let leader = roots.0.clone()?;
                let text =
                    ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, leader);
                Ok(text.trigger_world()?.evaluate(
                    body,
                    if kind == "available" || kind == "visible" {
                        command_scopes(leader, roots.1.clone()?)
                    } else {
                        let mut scopes = Scopes::of(Scope::Country(leader));
                        scopes.bare = true;
                        scopes
                    },
                ))
            })();
            match result {
                Ok(Eval::True) => detail.result = Some(true),
                Ok(Eval::False) => detail.result = Some(false),
                Ok(Eval::Unknown(why)) => detail
                    .reasons
                    .extend(why.into_iter().map(|why| why.to_string())),
                Err(why) => detail.reasons.push(why),
            }
        }
        reply.conditions.push(detail);
    }
    command_gates(frame, faction, cat, definition, reply);
    for kind in ["locked_goal", "is_manifest", "auto_complete"] {
        let (result, reasons) = match definition.flags.get(kind) {
            None => (Some(false), Vec::new()),
            Some(Ok(value)) => (Some(*value), Vec::new()),
            Some(Err(why)) => (None, vec![why.clone()]),
        };
        reply.flags.push(FactionConditionDetail {
            kind: kind.into(),
            result,
            reasons,
        });
    }
    for (kind, body) in &definition.effects {
        let mut detail = FactionRewardDetail {
            kind: kind.clone(),
            text: String::new(),
            reasons: Vec::new(),
        };
        if !body.is_empty() {
            let result = (|| -> Result<crate::effect_description::Description, String> {
                let leader = roots.0.clone()?;
                let text =
                    ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, leader);
                let content = cat.scripted_content()?;
                let cached = content.effects()?;
                let context = crate::effect_description::Context::new(
                    &text,
                    &cat.localisation,
                    leader,
                    "faction goal",
                    cached.view(None),
                );
                let mut scopes =
                    EventScopes::new(vec![EventScope::of(EventObject::Country(leader))]);
                Ok(crate::effect_description::describe(
                    body,
                    &context,
                    &mut scopes,
                ))
            })();
            match result {
                Ok(value) => {
                    detail.text = value.text;
                    detail.reasons.extend(
                        value
                            .refusals
                            .into_iter()
                            .map(|(why, count)| format!("{why} ({count})")),
                    );
                }
                Err(why) => detail.reasons.push(why),
            }
        }
        reply.rewards.push(detail);
    }
}

fn unused_candidate(name: &str, held: &[String]) -> bool {
    !held.iter().any(|existing| existing == name)
}
fn replacement_cost(status: i32, unfinished_cost: i64) -> i64 {
    if status == 0 { unfinished_cost } else { 0 }
}
fn command_gates(
    frame: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
    definition: &installed_content_core::faction_goals::Definition,
    reply: &mut FactionGoalDefinition,
) {
    let gates = (|| -> Result<(Result<bool, String>, bool), String> {
        let (leader, actor) = roots(frame, faction);
        let leader = leader?;
        let actor = actor?;
        let features = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
            .first()
            .ok_or("goal command runtime DLC feature mask missing")?;
        if features.dlc_mask & engine_dlc::dlc_feature_state::DEEPER_FACTIONS_MASK == 0 {
            return Ok((Ok(false), false));
        }
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|why| why.to_string())?;
        let members = frame
            .complete::<tables::FactionMembers>()
            .map_err(|why| why.to_string())?;
        let member = members.iter().find(|row| {
            row.faction == faction && crate::metadata::link_slot(links, row.country) == Some(actor)
        });
        let Some(_member) = member else {
            return Ok((Ok(false), false));
        };
        let entries = frame
            .complete::<tables::FactionGoalEntries>()
            .map_err(|why| why.to_string())?;
        let entries = companions::selected(entries, faction, |row| row.faction);
        let held = entries.iter().find(|row| {
            row.kind == 0
                && row
                    .token
                    .and_then(|token| {
                        faction_names::template_spelling(frame, Some(token))
                            .ok()
                            .flatten()
                    })
                    .as_deref()
                    == reply.token.as_deref()
        });
        let check = |kind: &str| -> Result<bool, String> {
            let value = reply
                .conditions
                .iter()
                .find(|value| value.kind == kind)
                .ok_or_else(|| format!("goal.{kind}: evaluation missing"))?;
            value.result.ok_or_else(|| value.reasons.join("; "))
        };
        // The native command checks available then allowed; visible is presentation only.
        let selection_checks = if held.is_some() {
            Ok(false)
        } else {
            check("available").and_then(|available| {
                if available {
                    check("allowed")
                } else {
                    Ok(false)
                }
            })
        };
        let Some(category) = definition.category.as_deref() else {
            return Err("goal.category missing".into());
        };
        let term = [b"short_term".as_slice(), b"medium_term", b"long_term"]
            .iter()
            .position(|value| *value == category)
            .ok_or("goal.category selects no native term")?;
        let defines =
            installed_content_core::defines::read(&cat.content.layers).map_err(|why| why.to_string())?;
        let capacity = defines
            .int32(
                "NFactions",
                [
                    "MAX_NUM_SHORT_TERM_GOALS",
                    "MAX_NUM_MEDIUM_TERM_GOALS",
                    "MAX_NUM_LONG_TERM_GOALS",
                ][term],
            )
            .map_err(|why| why.to_string())?;
        let extra = entries
            .iter()
            .find(|row| row.kind == 3 && row.ordinal as usize == term)
            .ok_or_else(|| format!("goal.extra_slots[{term}] missing"))?
            .slots;
        let catalog = installed_content_core::faction_goals::cached(&cat.content.layers)?;
        let mut used = 0i32;
        let mut held_names = Vec::new();
        for row in entries.iter().filter(|row| row.kind == 0) {
            let token = row.token.ok_or("runtime goal definition missing")?;
            let spelling = faction_names::template_spelling(frame, Some(token))?
                .ok_or("runtime goal spelling missing")?;
            let other = catalog
                .get(&spelling)?
                .ok_or_else(|| format!("goal {spelling} content missing"))?;
            held_names.push(spelling);
            if other.category.as_deref() == Some(category) {
                used = used.wrapping_add(1);
            }
        }
        let select = selection_checks
            .map(|eligible| eligible && capacity.wrapping_add(extra).wrapping_sub(used) > 0);
        let locked = definition
            .flags
            .get("locked_goal")
            .cloned()
            .transpose()?
            .unwrap_or(false);
        let removable = if let Some(held) = held {
            if locked {
                false
            } else {
                let cost = if held.status != 0 {
                    0
                } else {
                    replacement_cost(
                        held.status,
                        defines
                            .fixed5("NFactions", "REPLACING_UNFINISHED_FACTION_GOAL_COST")
                            .map_err(|why| why.to_string())?,
                    )
                };
                let affordable = if cost <= 0 {
                    true
                } else {
                    let rows = frame
                        .complete::<tables::FactionAccounts>()
                        .map_err(|why| why.to_string())?;
                    let account = rows
                        .iter()
                        .find(|row| {
                            row.kind == 0
                                && crate::metadata::link_slot(links, row.country) == Some(actor)
                        })
                        .ok_or("goal replacement actor initiative missing")?;
                    account.initiative() >= cost
                };
                if !affordable {
                    false
                } else {
                    let text = ScriptedText::new(
                        frame,
                        &cat.localisation,
                        cat.scripted_content()?,
                        leader,
                    );
                    let world = text.trigger_world()?;
                    let mut eligible = false;
                    let mut missing = Vec::new();
                    for (token, candidate) in catalog.definitions() {
                        if candidate.category.as_deref() != Some(category) {
                            continue;
                        }
                        let installed_content_core::tokens::Token::Name(name) = token else {
                            missing.push("replacement candidate has rejected identity".into());
                            continue;
                        };
                        let name = std::str::from_utf8(name)
                            .map_err(|_| "replacement candidate identity is not UTF-8")?;
                        if !unused_candidate(name, &held_names) {
                            continue;
                        }
                        if let Err(why) = catalog.get(name) {
                            missing.push(why);
                            continue;
                        }
                        let mut accepts = true;
                        for kind in ["available", "allowed"] {
                            let body = candidate
                                .conditions
                                .get(kind)
                                .map(Vec::as_slice)
                                .unwrap_or_default();
                            let scopes = if kind == "available" {
                                command_scopes(leader, actor)
                            } else {
                                let mut value = Scopes::of(Scope::Country(leader));
                                value.bare = true;
                                value
                            };
                            match world.evaluate(body, scopes) {
                                Eval::True => (),
                                Eval::False => {
                                    accepts = false;
                                    break;
                                }
                                Eval::Unknown(why) => {
                                    missing.extend(why.into_iter().map(|why| why.to_string()));
                                    accepts = false;
                                    break;
                                }
                            }
                        }
                        if accepts {
                            eligible = true;
                            break;
                        }
                    }
                    if !eligible && !missing.is_empty() {
                        return Err(format!(
                            "goal replacement candidate operands: {}",
                            missing.join("; ")
                        ));
                    }
                    eligible
                }
            }
        } else {
            false
        };
        Ok((select, removable))
    })();
    for (kind, select) in [
        ("select permission", true),
        ("remove permission (replacement)", false),
    ] {
        let (result, reasons) = match &gates {
            Ok((add, remove)) => {
                if select {
                    match add {
                        Ok(add) => (Some(*add), Vec::new()),
                        Err(why) => (None, vec![why.clone()]),
                    }
                } else {
                    (Some(*remove), Vec::new())
                }
            }
            Err(why) => (None, vec![why.clone()]),
        };
        reply.conditions.push(FactionConditionDetail {
            kind: kind.into(),
            result,
            reasons,
        });
    }
    // The add/change command has no independent clock deadline: term dates
    // anchor authored checks. Never invent a fixed-day cooldown from those dates.
    reply.conditions.push(FactionConditionDetail {
        kind: "cooldown".into(),
        result: Some(true),
        reasons: Vec::new(),
    });
    let dates = frame
        .complete::<tables::FactionGoalEntries>()
        .ok()
        .into_iter()
        .flatten()
        .filter(|row| row.faction == faction && row.kind == 4)
        .map(|row| format!("term {} date {}", row.ordinal, row.date))
        .collect::<Vec<_>>()
        .join("; ");
    reply.rewards.push(FactionRewardDetail{kind:"Command timing".into(),text:format!("No independent clock cooldown in the add/change command. Authored availability and allowed conditions apply at hour {}. {dates}",frame.hour),reasons:Vec::new()});
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{CountryLinkRow, FactionMemberRow, TableRows};
    use replay_frame::frame::TableView;
    #[test]
    fn confirmation_excludes_every_held_goal_for_selection_and_replacement() {
        let held = vec!["A".into(), "B".into()];
        assert!(
            !["A", "B"]
                .into_iter()
                .any(|candidate| unused_candidate(candidate, &held))
        );
        assert!(!unused_candidate("A", &held));
        assert!(unused_candidate("C", &held));
    }
    #[test]
    fn unlocked_cancelled_goal_with_unused_replacement_needs_no_initiative() {
        let held = vec!["A".into(), "B".into()];
        for status in [1, 2, -1, i32::MAX] {
            assert!(unused_candidate("C", &held) && 0 >= replacement_cost(status, 100_000));
        }
        assert!(0 < replacement_cost(0, 100_000));
    }
    #[test]
    fn command_scope_evaluates_root_and_from_as_different_countries() {
        use installed_content_core::{tag_aliases::TagAliases, trigger_keys::TriggerNames};
        use std::{collections::BTreeSet, sync::Arc};
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![
                replay_data::rows::CountryRow {
                    id: 7,
                    canonical_slot: 7,
                    tag: b"AAA".to_vec(),
                    ..Default::default()
                },
                replay_data::rows::CountryRow {
                    id: 19,
                    canonical_slot: 19,
                    tag: b"BBB".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 19,
                    canonical_slot: 19,
                },
            ]));
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                [7, 19]
                    .map(|country| replay_data::rows::CountryCounterInputRow {
                        country,
                        owned_state_count: 1,
                        ..Default::default()
                    })
                    .to_vec(),
            ));
        let dlcs = BTreeSet::new();
        let names = TriggerNames::default();
        let aliases = Arc::new(TagAliases::default());
        let world = crate::triggers::World::read(
            &frame,
            crate::triggers::ContentFacts::content_only(&dlcs, &names, &aliases),
        );
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let keys = installed_content_core::key_names::KeyNames {
            countries: [b"AAA".to_vec(), b"BBB".to_vec()].into(),
            ..Default::default()
        };
        let body =
            installed_content_core::triggers::read(b"ROOT = { tag = AAA } FROM = { tag = BBB }", &keys)
                .items;
        assert_eq!(world.evaluate(&body, command_scopes(7, 19)), Eval::True);
        assert_eq!(world.evaluate(&body, command_scopes(19, 19)), Eval::False);
    }
    #[test]
    fn command_scope_keeps_leader_root_and_actor_from_distinct() {
        let scope = command_scopes(7, 19);
        assert_eq!(scope.root, Scope::Country(7));
        assert_eq!(scope.this, Scope::Country(7));
        assert_eq!(scope.from, Some(Scope::Country(19)));
        assert_eq!(scope.prev, None);
    }
    #[test]
    fn faction_goal_leader_scope_does_not_depend_on_unavailable_selected_country() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 12,
                canonical_slot: 7,
            }]));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                faction: 9,
                ordinal: 0,
                country: 12,
            }]));
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Unavailable {
            reason: "selected country unavailable".into(),
        };
        let (leader, selected) = roots(&frame, 9);
        assert_eq!(leader, Ok(7));
        assert!(
            selected
                .unwrap_err()
                .contains("selected country unavailable")
        );
    }
}
