//! Authored bodies share scoped text and effect formatting. Occurrence inputs
//! prove scopes; sampled end-of-hour state never stands in for execution state.
use super::*;
use crate::{
    effect_description,
    scripted_text::{EventObject, EventScope, EventScopes, Link, ScopeName, ScriptedText},
};
use replay_data::event_feed_rows::{EventFeedEvent, EventFeedHistoryRow};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Inputs {
    pub(super) hour: i32,
    option: Option<i32>,
    actor: u32,
    identities: [[u32; 3]; 2],
    pub(super) actor_slot: Option<u32>,
    targeted: bool,
    target: Option<DecisionTarget>,
    missing: bool,
    scopes: Vec<ScopeInput>,
}
#[derive(Clone, Serialize, Deserialize)]
struct DecisionTarget {
    country: Option<u32>,
    numeric: Option<i32>,
    slot: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize)]
struct ScopeInput {
    reference: u32,
    /// The collector tried to record `id` and `names`.
    attempted: bool,
    /// The native ID `reference` resolved to at the occurrence, when recorded.
    id: Option<u32>,
    slot: Option<u32>,
    state: i32,
    bindings: [u32; 3],
    random: [u32; 2],
    unsupported: bool,
    /// The objects, region or combat the scope carries, which text may read instead of its country.
    opaque: Option<String>,
    /// The country's name, definite name and adjective at the occurrence, when recorded.
    names: Option<[Vec<u8>; 3]>,
}
impl Inputs {
    pub(super) fn captured(pack: &EventFeedHistoryRow, event: &EventFeedEvent) -> Self {
        let script = event.script.expect("script occurrence");
        Self {
            hour: event.hour,
            option: script.option,
            actor: event.actor.reference,
            actor_slot: None,
            identities: [event.actor, event.target]
                .map(|country| [country.reference, country.id, country.instance]),
            targeted: script.country_target.is_some() || script.numeric_target.is_some(),
            target: (script.country_target.is_some() || script.numeric_target.is_some()).then_some(
                DecisionTarget {
                    country: script.country_target,
                    numeric: script.numeric_target,
                    slot: None,
                },
            ),
            missing: script.scope_unrecorded,
            scopes: pack.script_scopes[script.scope_start as usize
                ..script.scope_start as usize + script.scope_len as usize]
                .iter()
                .map(|scope| {
                    let opaque = OBJECTS
                        .iter()
                        .zip(scope.references)
                        .filter(|(_, reference)| *reference != 0)
                        .map(|(kind, _)| *kind)
                        .chain(scope.strategic_region.map(|_| "a strategic region"))
                        .chain(scope.combat_context_unrecorded.then_some("a combat"))
                        .collect::<Vec<_>>();
                    ScopeInput {
                        reference: scope.country,
                        attempted: scope.country_attempted,
                        id: scope.country_id,
                        slot: None,
                        state: scope.state,
                        bindings: scope.bindings,
                        random: scope.random,
                        unsupported: !opaque.is_empty()
                            || scope.target_storage_unrecorded
                            || scope.tooltip_storage_unrecorded,
                        opaque: (!opaque.is_empty()).then(|| opaque.join(" and ")),
                        names: scope
                            .country_names
                            .map(|texts| texts.map(|text| text.as_bytes().to_vec())),
                    }
                })
                .collect(),
        }
    }
    pub(super) fn resolve(
        &mut self,
        mut slot: impl FnMut(replay_data::event_feed_rows::EventFeedCountry) -> Option<u32>,
    ) {
        let identities = self.identities;
        let resolve = |reference: u32,
                       recorded: Option<u32>,
                       slot: &mut dyn FnMut(
            replay_data::event_feed_rows::EventFeedCountry,
        ) -> Option<u32>| {
            if reference as i32 <= 0 {
                return Some(0);
            }
            // A later alias may give a raw reference to another country.
            let [reference, id, instance] = recorded
                .map(|id| [reference, id, 0])
                .or_else(|| {
                    identities
                        .iter()
                        .copied()
                        .find(|country| country[0] == reference)
                })?;
            slot(replay_data::event_feed_rows::EventFeedCountry {
                reference,
                id,
                instance,
            })
        };
        self.actor_slot = resolve(self.actor, None, &mut slot);
        if let Some(target) = &mut self.target
            && let Some(country) = target.country.filter(|country| *country as i32 > 0)
        {
            target.slot = resolve(country, None, &mut slot);
        }
        for scope in &mut self.scopes {
            scope.slot = resolve(scope.reference, scope.id, &mut slot);
        }
    }
    fn graph(&self, decision: bool) -> Result<EventScopes, String> {
        if decision {
            let slot = self
                .actor_slot
                .ok_or("decision actor has no exact occurrence-hour canonical slot")?;
            let mut scope = EventScope::of(EventObject::Country(slot));
            scope.random_seed_refusal = Some("decision execution random seed was not recorded");
            if self.targeted {
                let target = self
                    .target
                    .as_ref()
                    .ok_or("targeted decision selector was not recorded")?;
                let object = if target.country.is_some_and(|country| country as i32 > 0) {
                    EventObject::Country(
                        target
                            .slot
                            .ok_or("targeted decision country has no exact occurrence-hour slot")?,
                    )
                } else {
                    EventObject::State(
                        target
                            .numeric
                            .ok_or("targeted decision numeric selector was not recorded")?
                            as u32,
                    )
                };
                let mut from = EventScope::of(object);
                from.random_seed_refusal = scope.random_seed_refusal;
                scope.from = Link::Scope(1);
                from.from = Link::Scope(0);
                return Ok(EventScopes::new(vec![scope, from]));
            }
            return Ok(EventScopes::new(vec![scope]));
        }
        if self.missing || self.scopes.is_empty() {
            return Err("event execution scope graph was not recorded".into());
        }
        for (id, scope) in self.scopes.iter().enumerate() {
            if scope.unsupported {
                return Err(format!(
                    "scope {id} carries unsupported object, region, combat or saved/tooltip-target operands"
                ));
            }
            if scope.slot.is_none() {
                return Err(format!(
                    "scope {id} country reference {} has no exact occurrence-hour canonical slot",
                    scope.reference
                ));
            }
        }
        Ok(self.scope_graph())
    }

    /// The recorded scopes, an unproven country as none: callers decide what may be read.
    fn scope_graph(&self) -> EventScopes {
        let mut scopes = Vec::with_capacity(self.scopes.len());
        for scope in &self.scopes {
            let country = scope.slot.unwrap_or(0);
            let object = if scope.state > 0 {
                if country == 0 {
                    EventObject::State(scope.state as u32)
                } else {
                    EventObject::CountryState {
                        country,
                        state: scope.state as u32,
                    }
                }
            } else {
                EventObject::Country(country)
            };
            let [root, from, prev] = scope.bindings.map(|index| Link::Scope(index as usize));
            let mut resolved = EventScope::of(object);
            resolved.root = root;
            resolved.from = from;
            resolved.prev = prev;
            // Native scope acquisition preserves the two independent words.
            resolved.random_seed = scope.random[0];
            resolved.random_counter = scope.random[1];
            scopes.push(resolved);
        }
        EventScopes::new(scopes)
    }

    /// The scopes a title reads and where each one's country names come from.
    fn title_graph(&self) -> Result<(EventScopes, Vec<ScopeName>), String> {
        if !self.scopes.iter().any(|scope| scope.attempted) {
            return Ok((self.graph(false)?, Vec::new()));
        }
        let names = self
            .scopes
            .iter()
            .map(|scope| match (&scope.opaque, scope.slot, &scope.names) {
                (Some(kinds), ..) => ScopeName::Unproven(format!("it carries {kinds}")),
                (None, None, _) => ScopeName::Unproven(format!(
                    "its country reference {} has no exact occurrence-hour canonical slot",
                    scope.reference
                )),
                (None, Some(_), Some(names)) => ScopeName::Recorded(names.clone()),
                (None, Some(_), None) if scope.reference as i32 <= 0 => {
                    ScopeName::Unproven("it holds no country".into())
                }
                (None, Some(_), None) => ScopeName::Unproven(
                    "its country's names were not recorded: unreadable, too long or possibly mid-rename"
                        .into(),
                ),
            })
            .collect();
        Ok((self.scope_graph(), names))
    }
}


/// The objects a scope's references name, in native field order.
const OBJECTS: [&str; 9] = [
    "a character",
    "an operation",
    "an ace",
    "a unit",
    "an industrial organisation",
    "a purchase contract",
    "a raid",
    "a project",
    "a faction",
];

/// An event text key expanded in the occurrence's recorded scopes, country names only.
pub(super) fn scoped_text(
    catalogue: &Catalogue,
    inputs: &Inputs,
    frame: replay_frame::frame::HourFrame<'_>,
    key: &str,
) -> Result<String, String> {
    let (scopes, names) = inputs.title_graph()?;
    let scripted = catalogue.scripted_content()?;
    let text = catalogue
        .localisation
        .localise(key.as_bytes())
        .map_err(|why| why.to_string())?;
    let expander = ScriptedText::new_in(
        frame,
        &catalogue.localisation,
        scripted,
        inputs.actor_slot.unwrap_or(0),
    );
    catalogue::plain_text(&expander.expand_names_in(&text, &scopes, &names)?)
}

/// A decision's title key and text for its owner, and whether the recorded title scope can expand it.
pub(super) fn decision_name(
    catalogue: &Catalogue,
    inputs: &Inputs,
    frame: replay_frame::frame::HourFrame<'_>,
    token: &str,
) -> Result<(String, String, bool), String> {
    let content = catalogue.scripted_content()?;
    let effects = content.effects()?;
    let decisions = effects
        .view(None)
        .decisions
        .ok_or("decision database is unavailable from the recorded content stack")?;
    let definition = decisions
        .decision(token.as_bytes())?
        .ok_or_else(|| format!("decision {token} is absent from recorded content"))?;
    if let Some(reason) = &definition.refused {
        return Err(reason.clone());
    }
    let owner = inputs
        .actor_slot
        .ok_or("decision owner has no exact occurrence-hour canonical slot")?;
    let text = ScriptedText::new_in(frame, &catalogue.localisation, content, owner);
    let key = crate::effect_description::decision_name_key_at(
        frame,
        text.recorded(),
        &catalogue.localisation,
        definition,
        owner,
        text.country_tag(owner)?,
    )?;
    let name = catalogue
        .localised(&key, catalogue::View::Plain)
        .ok_or_else(|| format!("decision title {key} is absent from recorded content"))?;
    // A highlighted state would join the title scope; the recording does not hold it.
    let scoped = !definition.has_state_targets && definition.highlight_conditions.is_empty();
    Ok((key, name, scoped))
}

/// A decision title key expanded in the game's title scope: the owner, its FROM the recorded target.
pub(super) fn decision_title_text(
    catalogue: &Catalogue,
    inputs: &Inputs,
    frame: replay_frame::frame::HourFrame<'_>,
    key: &str,
) -> Result<String, String> {
    let mut scopes = inputs.graph(true)?;
    for scope in &mut scopes.scopes {
        scope.random_seed_refusal = Some("decision title random seed was not recorded");
    }
    // The title's target scope links to itself, unlike the execution scope.
    if let Some(target) = scopes.scopes.get_mut(1) {
        target.from = Link::Itself;
    }
    let text = catalogue
        .localisation
        .localise(key.as_bytes())
        .map_err(|why| why.to_string())?;
    let owner = inputs.actor_slot.unwrap_or(0);
    let expander = ScriptedText::new_in(
        frame,
        &catalogue.localisation,
        catalogue.scripted_content()?,
        owner,
    );
    catalogue::plain_text(&expander.expand_names_in(&text, &scopes, &[])?)
}

fn render(
    catalogue: &Catalogue,
    inputs: &Inputs,
    decision: bool,
    source: &str,
    items: &[installed_content_core::parser::Item],
) -> Result<effect_description::Description, String> {
    let scripted = catalogue.scripted_content()?;
    let content = scripted.effects()?;
    // Values sampled at an hour boundary do not prove the state immediately
    // before/after an effect. Content-only/scoped descriptions still work;
    // the shared evaluator names every runtime input that cannot be supplied.
    let frame = replay_frame::frame::DecodedFrame {
        branch: 0,
        hour: inputs.hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| {
            replay_frame::frame::TableView::Unavailable {
                reason: "exact effect-execution state between samples was not recorded".into(),
            }
            .into()
        }),
    };
    let text = ScriptedText::new(
        &frame,
        &catalogue.localisation,
        scripted,
        inputs.actor_slot.unwrap_or(0),
    );
    let events = script_events(catalogue);
    let lookup = |id: &[u8]| events.get(id);
    let context = effect_description::Context::new(
        &text,
        &catalogue.localisation,
        inputs.actor_slot.unwrap_or(0),
        source,
        content.view(Some(&lookup)),
    );
    Ok(effect_description::describe(
        items,
        &context,
        &mut inputs.graph(decision)?,
    ))
}
fn body(
    builder: &mut Builder,
    catalogue: &Catalogue,
    item: &mut EventFeedItem,
    inputs: Option<&Inputs>,
    decision: bool,
    label: &str,
    items: &[installed_content_core::parser::Item],
) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let token = item.token.as_deref().unwrap_or("unidentified content");
    let source = format!("{token} {label}");
    let rendered = inputs
        .ok_or_else(|| "occurrence scope operands were not retained".to_owned())
        .and_then(|inputs| render(catalogue, inputs, decision, &source, items));
    let rendered = match rendered {
        Ok(rendered) => rendered,
        Err(reason) => {
            // Count every top-level authored effect whose body could not be
            // described; retain its exact key and owning event/decision.
            for effect in items {
                let key = effect
                    .key
                    .as_deref()
                    .map(String::from_utf8_lossy)
                    .unwrap_or_default();
                builder.unavailable(
                    "scriptEffectDescription",
                    "Authored effects",
                    format!("{source} effect {key}: {reason}"),
                    Some(1),
                );
            }
            return Ok(());
        }
    };
    for (reason, count) in rendered.refusals {
        builder.unavailable(
            "scriptEffectDescription",
            "Authored effects",
            format!("{source}: {reason}"),
            Some(
                u32::try_from(count)
                    .map_err(|_| io::Error::other("Effect refusal count exceeds u32"))?,
            ),
        );
    }
    if !rendered.text.is_empty() {
        let description = item.description.get_or_insert_default();
        if !description.is_empty() {
            description.push_str("\n\n");
        }
        description.push_str(&format!("Authored {label}:\n{}", rendered.text));
    }
    Ok(())
}
pub(super) fn present(
    builder: &mut Builder,
    catalogue: &Catalogue,
    kind: EventFeedKind,
    item: &mut EventFeedItem,
    inputs: Option<&Inputs>,
) -> io::Result<()> {
    use EventFeedKind::*;
    let Some(token) = item.token.clone() else {
        return Ok(());
    };
    if matches!(
        kind,
        ScriptEventFired | NewsEventFired | EventOptionSelected
    ) {
        let events = script_events(catalogue);
        match events.get(token.as_bytes()) {
            Ok(event) => {
                if let Some(reason) = &event.refused {
                    builder.unavailable(
                        "scriptEffectDescription",
                        "Authored effects",
                        format!("Event {token}: {reason}"),
                        Some(1),
                    );
                } else if kind != EventOptionSelected {
                    body(
                        builder,
                        catalogue,
                        item,
                        inputs,
                        false,
                        "immediate effects",
                        &event.immediate,
                    )?;
                } else {
                    match inputs.and_then(|input| input.option) {
                        Some(ordinal) if ordinal < 0 => body(builder, catalogue, item, inputs, false, "immediate option effects", &event.immediate)?,
                        Some(ordinal) => match event.options.get(ordinal as usize) {
                            Some(effects) => body(builder, catalogue, item, inputs, false, &format!("option {ordinal} effects"), effects)?,
                            None => builder.unavailable("scriptEffectDescription", "Authored effects", format!("Event {token} option {ordinal}: body absent from recorded content"), Some(1)),
                        },
                        None => builder.unavailable("scriptEffectDescription", "Authored effects", format!("Event {token}: selected option ordinal was not recorded"), Some(1)),
                    }
                }
            }
            Err(reason) => builder.unavailable(
                "scriptEffectDescription",
                "Authored effects",
                format!("Event {token}: {reason}"),
                Some(1),
            ),
        }
    } else {
        let field = match kind {
            DecisionSelected | DecisionCompleted | DecisionCompletionEffect => "complete_effect",
            DecisionRemovalEffect => "remove_effect",
            DecisionCancellationEffect => "cancel_effect",
            DecisionTimedOut | DecisionTimeoutEffect => "timeout_effect",
            _ => return Ok(()),
        };
        let result = (|| {
            let content = catalogue.scripted_content()?.effects()?;
            let decisions = content
                .view(None)
                .decisions
                .ok_or("decision effect database is unavailable from the recorded content stack")?;
            decisions
                .decision(token.as_bytes())
                .map(|decision| decision.cloned())?
                .ok_or_else(|| format!("decision {token} is absent from recorded content"))
        })();
        match result {
            Ok(decision) => {
                if let Some(reason) = decision.refused {
                    builder.unavailable(
                        "scriptEffectDescription",
                        "Authored effects",
                        format!("Decision {token} {field}: {reason}"),
                        Some(1),
                    );
                } else {
                    let effects = match field {
                        "complete_effect" => &decision.complete_effect,
                        "remove_effect" => &decision.remove_effect,
                        "cancel_effect" => &decision.cancel_effect,
                        _ => &decision.timeout_effect,
                    };
                    body(builder, catalogue, item, inputs, true, field, effects)?;
                }
            }
            Err(reason) => builder.unavailable(
                "scriptEffectDescription",
                "Authored effects",
                format!("Decision {token} {field}: {reason}"),
                Some(1),
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_graph_preserves_alias_resolution_links_and_both_random_words() {
        let mut input = Inputs {
            hour: 10,
            option: Some(2),
            actor: 90,
            identities: [[90, 100, 8], [91, 101, 9]],
            actor_slot: None,
            targeted: false,
            target: None,
            missing: false,
            scopes: vec![
                ScopeInput {
                    reference: 90,
                    attempted: false,
                    id: None,
                    slot: None,
                    state: 0,
                    bindings: [0, 1, 0],
                    random: [17, 23],
                    unsupported: false,
                    opaque: None,
                    names: None,
                },
                ScopeInput {
                    reference: 91,
                    attempted: false,
                    id: None,
                    slot: None,
                    state: 4,
                    bindings: [1, 0, 1],
                    random: [31, 37],
                    unsupported: false,
                    opaque: None,
                    names: None,
                },
            ],
        };
        input.resolve(|country| match (country.reference, country.id) {
            (90, 100) => Some(2),
            (91, 101) => Some(3),
            (92, 102) => Some(4),
            _ => None,
        });
        let graph = input.graph(false).unwrap();
        assert_eq!(graph.scopes[0].this, EventObject::Country(2));
        assert_eq!(
            graph.scopes[1].this,
            EventObject::CountryState {
                country: 3,
                state: 4
            }
        );
        assert_eq!(graph.scopes[0].from, Link::Scope(1));
        assert_eq!(
            (graph.scopes[0].random_seed, graph.scopes[0].random_counter),
            (17, 23)
        );
        input.scopes[1].unsupported = true;
        assert!(input.graph(false).unwrap_err().contains("scope 1"));
        input.targeted = true;
        assert!(input.graph(true).unwrap_err().contains("targeted decision"));
    }

    #[test]
    fn titles_name_scopes_by_their_occurrence_names_and_old_recordings_keep_the_hours() {
        let scope = |reference, id: Option<u32>, name: Option<&str>, bindings| ScopeInput {
            reference,
            attempted: id.is_some() || name.is_some(),
            id,
            slot: None,
            state: 0,
            bindings,
            random: [1, 2],
            unsupported: false,
            opaque: None,
            names: name.map(|name| [name; 3].map(|text| text.as_bytes().to_vec())),
        };
        let slots = |country: replay_data::event_feed_rows::EventFeedCountry| {
            match (country.reference, country.id) {
                (90, 100) => Some(2),
                (92, 102) => Some(4),
                _ => None,
            }
        };
        let mut input = Inputs {
            hour: 10,
            option: None,
            actor: 90,
            identities: [[90, 100, 8], [0; 3]],
            actor_slot: None,
            targeted: false,
            target: None,
            missing: false,
            scopes: vec![
                scope(90, Some(100), Some("Reich"), [0, 1, 0]),
                // FROM: neither actor nor target; its own recorded ID proves it.
                scope(92, Some(102), Some("Italy"), [0, 2, 1]),
                // A later alias of the same reference to another ID proves nothing.
                scope(92, Some(103), Some("Other"), [0, 0, 2]),
            ],
        };
        input.scopes[2].opaque = Some("a character".into());
        input.scopes[2].unsupported = true;
        input.resolve(slots);
        let (graph, names) = input.title_graph().unwrap();
        assert_eq!(graph.scopes[1].this, EventObject::Country(4));
        assert_eq!(graph.scopes[1].from, Link::Scope(2));
        assert!(matches!(&names[1], ScopeName::Recorded(texts) if texts[0] == b"Italy"));
        assert!(matches!(&names[2], ScopeName::Unproven(why) if why.contains("a character")));
        // Effects may read any scope, so they still refuse the whole graph.
        assert!(input.graph(false).unwrap_err().contains("scope 2"));
        input.scopes[2].opaque = None;
        input.scopes[2].unsupported = false;
        let (_, names) = input.title_graph().unwrap();
        assert!(matches!(&names[2], ScopeName::Unproven(why) if why.contains("reference 92")));
        // A proven country whose names were not recorded is not named from the hour.
        input.scopes[1].names = None;
        let (_, names) = input.title_graph().unwrap();
        assert!(matches!(&names[1], ScopeName::Unproven(why) if why.contains("not recorded")));
        // A failed attempt is still not an old recording: no scope falls back to the hour.
        for scope in &mut input.scopes {
            scope.id = None;
            scope.names = None;
        }
        input.resolve(slots);
        let (_, names) = input.title_graph().unwrap();
        assert!(matches!(&names[0], ScopeName::Unproven(why) if why.contains("not recorded")));
        // Older recordings, as before: the hour's names where every scope is proven.
        input.scopes.truncate(1);
        input.scopes[0] = scope(90, None, None, [0; 3]);
        input.resolve(slots);
        assert!(input.title_graph().unwrap().1.is_empty());
        input.scopes.push(scope(92, None, None, [0, 0, 1]));
        input.resolve(slots);
        assert!(input.title_graph().unwrap_err().contains("scope 1"));
    }

    #[test]
    fn selected_authored_effects_are_formatted_and_unsupported_children_are_named_counted() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "events/synthetic.txt",
                "add_namespace=synthetic country_event={id=synthetic.1 title=TITLE desc=BODY option={name=CHOICE custom_effect_tooltip=KNOWN_EFFECT add_political_power=var:missing}}",
            ),
            ("common/decisions/categories/synthetic.txt", "category={}"),
            (
                "common/decisions/synthetic.txt",
                "category={synthetic_decision={cancel_effect={custom_effect_tooltip=KNOWN_EFFECT} timeout_effect={add_stability=var:missing}}}",
            ),
            (
                "localisation/english/synthetic_l_english.yml",
                "\u{feff}l_english:\n KNOWN_EFFECT:0 \"Synthetic authored result\"\n TITLE:0 \"Title\"\n BODY:0 \"Body\"\n CHOICE:0 \"Choice\"\n",
            ),
        ]);
        let input = Inputs {
            hour: 10,
            option: Some(0),
            actor: 1,
            identities: [[1, 1, 1], [0; 3]],
            actor_slot: Some(1),
            targeted: false,
            target: None,
            missing: false,
            scopes: vec![ScopeInput {
                reference: 1,
                attempted: true,
                id: Some(1),
                slot: Some(1),
                state: 0,
                bindings: [0; 3],
                random: [1, 2],
                unsupported: false,
                opaque: None,
                names: None,
            }],
        };
        let mut builder = Builder::new(10, 10);
        let mut item = plain_item(Some("synthetic.1".into()), None, None, None);
        present(
            &mut builder,
            &catalogue,
            EventFeedKind::EventOptionSelected,
            &mut item,
            Some(&input),
        )
        .unwrap();
        assert!(
            item.description
                .as_ref()
                .unwrap()
                .contains("Synthetic authored result")
        );
        let refusal = builder
            .feed
            .unavailable
            .iter()
            .find(|entry| entry.reason.contains("add_political_power"))
            .unwrap();
        assert!(refusal.reason.contains("synthetic.1 option 0"));
        assert_eq!(refusal.count, Some(1));
        let mut item = plain_item(Some("synthetic_decision".into()), None, None, None);
        present(
            &mut builder,
            &catalogue,
            EventFeedKind::DecisionCancellationEffect,
            &mut item,
            Some(&input),
        )
        .unwrap();
        assert!(
            item.description
                .as_ref()
                .unwrap()
                .contains("Synthetic authored result")
        );
        present(
            &mut builder,
            &catalogue,
            EventFeedKind::DecisionTimeoutEffect,
            &mut item,
            Some(&input),
        )
        .unwrap();
        assert!(builder.feed.unavailable.iter().any(|entry| {
            entry.reason.contains("synthetic_decision timeout_effect")
                && entry.reason.contains("add_stability")
        }));
    }
}
