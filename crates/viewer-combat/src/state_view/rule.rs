use content_reader_politics::faction_templates::FactionTemplateProducts as _;
use super::{loc, paths_named};
use crate::binding::Bindings;
use crate::content::Content;
use crate::tooltips::{hover_text, set_hover};
use content_reader_map::adjacency_rules::{AdjacencyRule, Verdict};
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::localisation::Argument;
use content_reader_gui::gui::native;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_unit_reads::movement_reads::RuleReason;

type Loc<'a> = dyn Fn(&str, &[(&str, Argument<'_>)]) -> Result<String, String> + 'a;

const SUFFIXES: [&str; 4] = ["_CONTESTED", "_ENEMY", "_FRIEND", "_NEUTRAL"];

fn allowed(loc: &Loc<'_>, set: bool) -> Result<String, String> {
    let (key, colour) = if set { ("YES", 'G') } else { ("NO", 'R') };
    Ok(format!("\u{11}{colour}{}\u{11}!", loc(key, &[])?))
}

fn permission_list(loc: &Loc<'_>, bits: u8) -> Result<String, String> {
    let [army, navy, submarines, trade] = [1, 2, 4, 8].map(|bit| allowed(loc, bits & bit != 0));
    loc(
        "PERMISSION_DESC",
        &[
            ("ARMY", Argument::Text(&army?)),
            ("NAVY", Argument::Text(&navy?)),
            ("SUBMARINES", Argument::Text(&submarines?)),
            ("TRADE", Argument::Text(&trade?)),
        ],
    )
}

fn permission_text(loc: &Loc<'_>, permissions: [u8; 4], relation: Option<usize>, reason: &str, current: bool) -> Result<String, String> {
    let mut text = String::new();
    if relation.is_none() && current {
        text.push_str(reason);
    }
    for (index, suffix) in SUFFIXES.iter().enumerate() {
        let this = relation == Some(index);
        if this != current {
            continue;
        }
        let name = loc(&format!("PERMISSION{suffix}"), &[])?;
        let heading = if this { "PERMISSION_CURRENT_HEADER" } else { "PERMISSION_HEADER" };
        text.push_str(&loc(heading, &[("PERMISSION", Argument::Text(&name))])?);
        text.push('\n');
        if this {
            text.push_str(reason);
            text.push('\n');
        }
        text.push_str(&permission_list(loc, permissions[index])?);
        text.push('\n');
    }
    Ok(text)
}

fn side(content: &Content, frame: &DecodedFrame, names: &CountryNames<'_>, slot: u32) -> Result<String, String> {
    let links = frame.complete::<tables::CountryLinks>().map_err(|e| e.to_string())?;
    let members = frame.complete::<tables::FactionMembers>().map_err(|e| e.to_string())?;
    let slot_of = |raw: u32| links.binary_search_by_key(&raw, |row| row.id).ok().map(|at| links[at].canonical_slot);
    let Some(member) = members.iter().find(|row| slot_of(row.country) == Some(slot)) else {
        return names.name(slot);
    };
    let factions = frame.complete::<tables::Factions>().map_err(|e| e.to_string())?;
    let faction = factions
        .iter()
        .find(|row| row.id == member.faction)
        .ok_or_else(|| format!("faction {} has no Factions row", member.faction))?;
    let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
    let templates = scripted.layers.layers.faction_templates();
    let lexical = dashboard_text::faction_names::LexicalNames::new(frame.tables[replay_data::rows::TableId::LexicalTokens.index()].clone());
    let template = dashboard_text::faction_names::template_spelling(frame, faction.template_token);
    dashboard_text::faction_names::name(
        &faction.name,
        template.as_ref().map(|value| value.as_deref()).map_err(Clone::clone),
        templates.as_deref().map_err(Clone::clone),
        &content.localisation,
        &content.recorded_language,
        |name| lexical.token_of(name),
    )
}

fn disabled(content: &Content, frame: &DecodedFrame, rule: &AdjacencyRule, viewer: u32) -> Result<String, String> {
    let key = rule.disabled_reason.as_ref().map_err(Clone::clone)?;
    if key.is_empty() {
        return loc(content, "PERMISSION_BLOCKED", &[]);
    }
    let text = content.localisation.localise_with(key, &[]).map_err(|why| why.to_string())?;
    let text = if text.contains('[') {
        use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, ScriptedText};
        let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
        let scripted = ScriptedText::new(frame, &content.localisation, scripted, viewer).viewing(viewer);
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(viewer))]);
        scripted.expand_in_without_random_seed(&text, &scopes, "the disabled rule's reason")?
    } else {
        text
    };
    if text.is_empty() {
        return loc(content, "PERMISSION_BLOCKED", &[]);
    }
    let text = viewer_foundation::tooltip_text::displayable(content.assets.as_ref(), text)?;
    loc(content, "PERMISSION_BLOCKED_DUE_TO", &[("REASON", Argument::Text(&text))])
}

fn keys(verdict: Verdict) -> Result<(&'static str, &'static str), String> {
    match verdict {
        Verdict::Enemy => Ok(("PERMISSION_ENEMY_UNNAMED_DESC", "PERMISSION_NOT_ENEMY_DESC")),
        Verdict::Friend => Ok(("PERMISSION_FRIEND_UNNAMED_DESC", "PERMISSION_NOT_FRIENDLY_DESC")),
        Verdict::Neutral => Ok(("PERMISSION_NEUTRAL_UNNAMED_DESC", "PERMISSION_NOT_NEUTRAL_DESC")),
        Verdict::Contested => Err("the contested relation has no trigger of its own".into()),
    }
}

fn trigger_reason(loc: &Loc<'_>, describe: &dyn Fn(Verdict, usize) -> Result<String, String>, reason: RuleReason) -> Result<String, String> {
    use Verdict::{Enemy, Friend, Neutral};
    let (shown, failed): (Verdict, &[Verdict]) = match reason {
        RuleReason::Held(verdict) => return Ok(format!("{}:
{}", loc(keys(verdict)?.0, &[])?, describe(verdict, 1)?)),
        RuleReason::Excluded(Neutral) => (Neutral, &[Enemy, Friend]),
        RuleReason::Excluded(Friend) => (Friend, &[Enemy, Neutral]),
        RuleReason::Excluded(Enemy) => (Enemy, &[Friend, Neutral]),
        RuleReason::NotEnemy => (Neutral, &[Enemy]),
        RuleReason::NotFriendly => (Neutral, &[Friend]),
        RuleReason::NotNeutral => (Enemy, &[Neutral]),
        other => return Err(format!("{other:?} does not describe the rule's triggers")),
    };
    let mut text = format!("{}:
  ", loc(keys(shown)?.0, &[])?);
    for &verdict in failed {
        text.push_str(&loc(keys(verdict)?.1, &[])?);
        text.push_str(":
");
        text.push_str(&describe(verdict, 2)?);
    }
    Ok(text)
}

fn triggers(content: &Content, frame: &DecodedFrame, rule: &AdjacencyRule, viewer: u32, verdict: Verdict, indent: usize) -> Result<String, String> {
    use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, ScriptedText};
    let items = match verdict {
        Verdict::Enemy => &rule.is_enemy,
        Verdict::Friend => &rule.is_friend,
        Verdict::Neutral => &rule.is_neutral,
        Verdict::Contested => return Err("the contested relation has no trigger of its own".into()),
    };
    let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
    let text = ScriptedText::new(frame, &content.localisation, scripted, viewer).viewing(viewer);
    let world = dashboard_text::triggers::World::new(frame, content.trigger_content.facts()).map_err(|why| why.to_string())?;
    let graph = std::rc::Rc::new(std::cell::RefCell::new(EventScopes::new(vec![EventScope::of(EventObject::Country(viewer))])));
    let evaluator = dashboard_text::triggers::Evaluator::new(&world, None).with_event_scopes(graph);
    let scopes = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
    let mut refused = std::collections::BTreeMap::new();
    let no_tactic = |_: &[u8]| -> Result<Vec<u8>, String> { Err("an adjacency rule trigger names no tactic".into()) };
    let described = dashboard_text::triggers::description::describe_at(items, &evaluator, &text, &content.localisation, scopes, &[], &no_tactic, indent, 0, &mut refused)?;
    if let Some(why) = refused.keys().next() {
        return Err(format!("its triggers: {why}"));
    }
    viewer_foundation::tooltip_text::displayable(content.assets.as_ref(), described)
}

fn reason(content: &Content, frame: &DecodedFrame, rule: &AdjacencyRule, viewer: u32, reason: RuleReason) -> Result<String, String> {
    let names = CountryNames::read(frame, &content.localisation);
    match reason {
        RuleReason::Disabled => disabled(content, frame, rule, viewer),
        RuleReason::Contested(first, second) => {
            let (first, second) = (side(content, frame, &names, first)?, side(content, frame, &names, second)?);
            loc(content, "PERMISSION_CONTESTED_DESC", &[("SIDE1", Argument::Text(&first)), ("SIDE2", Argument::Text(&second))])
        }
        RuleReason::Enemy(country) => loc(content, "PERMISSION_ENEMY_DESC", &[("ENEMY", Argument::Text(&names.name(country)?))]),
        RuleReason::Friend(country) => loc(content, "PERMISSION_FRIEND_DESC", &[("FRIEND", Argument::Text(&names.name(country)?))]),
        RuleReason::Neutral => Ok(loc(content, "PERMISSION_NEUTRAL_UNNAMED_DESC", &[])? + "."),
        RuleReason::Held(_) | RuleReason::Excluded(_) | RuleReason::NotEnemy | RuleReason::NotFriendly | RuleReason::NotNeutral => trigger_reason(
            &|key: &str, arguments: &[(&str, Argument<'_>)]| loc(content, key, arguments),
            &|verdict: Verdict, indent: usize| triggers(content, frame, rule, viewer, verdict, indent),
            reason,
        ),
    }
}

fn hover(content: &Content, frame: &DecodedFrame, rule: &AdjacencyRule, viewer: u32, relations: &crate::RuleRelations) -> Result<(String, String), String> {
    let relation = relations(frame, rule, viewer)?;
    let why = reason(content, frame, rule, viewer, relation)?;
    let at = relation.verdict().map(|verdict| verdict as usize);
    let text = |key: &str, arguments: &[(&str, Argument<'_>)]| loc(content, key, arguments);
    let name = loc(content, &rule.name, &[])?;
    let mut main = loc(content, "HEADER", &[("KEY", Argument::Text(&name))])?;
    main.push_str(&permission_text(&text, rule.permissions, at, &why, true)?);
    let delayed = permission_text(&text, rule.permissions, at, &why, false)?;
    Ok((main, delayed))
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, province: u32, viewer: u32, relations: Option<&crate::RuleRelations>) {
    let rule = match &content.assets.required_rules {
        Ok(rules) => match rules.of(province) {
            Some(rule) => rule,
            None => return,
        },
        Err(_) => return,
    };
    let text = relations
        .ok_or_else(|| "the adjacency rule relations were not given to the window".to_owned())
        .and_then(|relations| hover(content, frame, rule, viewer, relations))
        .map_err(|why| format!("state view required rule hover: {why}"));
    for path in paths_named(root, "province_required_rule") {
        match &text {
            Ok((main, delayed)) => set_hover(&mut out.values, path, hover_text(main), hover_text(delayed)),
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(why.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(key: &str, arguments: &[(&str, Argument<'_>)]) -> Result<String, String> {
        let named: Vec<String> = arguments
            .iter()
            .map(|(name, value)| match value {
                Argument::Text(text) => format!("{name}={text}"),
                _ => unreachable!(),
            })
            .collect();
        Ok(if named.is_empty() { key.to_owned() } else { format!("{key}[{}]", named.join(",")) })
    }

    #[test]
    fn the_current_relation_heads_the_main_text_and_the_others_are_delayed() {
        let permissions = [0x0f, 0x00, 0x05, 0x0f];
        let main = permission_text(&fake, permissions, Some(1), "WHY", true).unwrap();
        assert_eq!(
            main,
            "PERMISSION_CURRENT_HEADER[PERMISSION=PERMISSION_ENEMY]\nWHY\n\
             PERMISSION_DESC[ARMY=\u{11}RNO\u{11}!,NAVY=\u{11}RNO\u{11}!,SUBMARINES=\u{11}RNO\u{11}!,TRADE=\u{11}RNO\u{11}!]\n"
        );
        let delayed = permission_text(&fake, permissions, Some(1), "WHY", false).unwrap();
        let headings: Vec<&str> = delayed.lines().filter(|line| line.starts_with("PERMISSION_HEADER")).collect();
        assert_eq!(
            headings,
            [
                "PERMISSION_HEADER[PERMISSION=PERMISSION_CONTESTED]",
                "PERMISSION_HEADER[PERMISSION=PERMISSION_FRIEND]",
                "PERMISSION_HEADER[PERMISSION=PERMISSION_NEUTRAL]"
            ]
        );
        assert!(!delayed.contains("WHY"));
        assert!(delayed.contains("ARMY=\u{11}GYES\u{11}!,NAVY=\u{11}RNO\u{11}!,SUBMARINES=\u{11}GYES\u{11}!,TRADE=\u{11}RNO\u{11}!"));
    }

    #[test]
    fn a_trigger_reason_describes_the_held_triggers_at_one_and_the_failed_at_two() {
        let describe = |verdict: Verdict, indent: usize| Ok(format!("{verdict:?}@{indent}
"));
        assert_eq!(
            trigger_reason(&fake, &describe, RuleReason::Held(Verdict::Friend)).unwrap(),
            "PERMISSION_FRIEND_UNNAMED_DESC:
Friend@1
"
        );
        assert_eq!(
            trigger_reason(&fake, &describe, RuleReason::Excluded(Verdict::Neutral)).unwrap(),
            "PERMISSION_NEUTRAL_UNNAMED_DESC:
  PERMISSION_NOT_ENEMY_DESC:
Enemy@2
PERMISSION_NOT_FRIENDLY_DESC:
Friend@2
"
        );
        assert_eq!(
            trigger_reason(&fake, &describe, RuleReason::NotNeutral).unwrap(),
            "PERMISSION_ENEMY_UNNAMED_DESC:
  PERMISSION_NOT_NEUTRAL_DESC:
Neutral@2
"
        );
        assert_eq!(
            trigger_reason(&fake, &describe, RuleReason::NotFriendly).unwrap(),
            "PERMISSION_NEUTRAL_UNNAMED_DESC:
  PERMISSION_NOT_FRIENDLY_DESC:
Friend@2
"
        );
    }

    #[test]
    fn a_disabled_rule_gives_its_reason_alone_and_every_relation_delayed() {
        assert_eq!(permission_text(&fake, [0; 4], None, "BLOCKED", true).unwrap(), "BLOCKED");
        let delayed = permission_text(&fake, [0; 4], None, "BLOCKED", false).unwrap();
        assert_eq!(delayed.lines().filter(|line| line.starts_with("PERMISSION_HEADER")).count(), 4);
        assert!(!delayed.contains("BLOCKED"));
    }
}
