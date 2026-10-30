//! Roster derivation uses certified retained state and the shared interpreter.
//! A boundary's unproved mutable operand is refused by its predicate, never
//! replaced by the later hour sample.
use super::*;
use crate::{
    scripted_text::{EventObject, EventScope, EventScopes, Link, ScriptedText, random},
    triggers::{Evaluator, Scope, Scopes},
};
use replay_data::event_feed_rows::{
    EventFeedCountry, EventFeedEvent,
    decisions::{
        CONTAINERS, DecisionState, IGNORED, REENABLE, REMOVE_REQUESTED, TAKEN, TARGETED,
        TARGETED_TIMED, TIMED,
    },
};
use std::{cell::RefCell, rc::Rc};

/// Decision tokens are written as JSON text when they are UTF-8, as their bytes otherwise.
mod token {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(token: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        match std::str::from_utf8(token) {
            Ok(text) => serializer.serialize_str(text),
            Err(_) => serializer.collect_seq(token),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        Ok(match Read::deserialize(deserializer)? {
            Read::Text(text) => text.into_bytes(),
            Read::Bytes(bytes) => bytes,
        })
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Read {
        Text(String),
        Bytes(Vec<u8>),
    }

    /// A token in a list.
    pub(super) struct Text<'a>(pub &'a [u8]);
    impl Serialize for Text<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serialize(self.0, serializer)
        }
    }
    pub(super) struct Owned(pub Vec<u8>);
    impl<'de> Deserialize<'de> for Owned {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserialize(deserializer).map(Self)
        }
    }

    /// Ignored decisions' tokens.
    pub(super) mod list {
        use super::*;
        pub(in super::super) fn serialize<S: Serializer>(
            tokens: &[Vec<u8>],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(tokens.iter().map(|token| Text(token)))
        }
        pub(in super::super) fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<Vec<u8>>, D::Error> {
            Ok(Vec::<Owned>::deserialize(deserializer)?
                .into_iter()
                .map(|token| token.0)
                .collect())
        }
    }

    /// Description counts: each decision's token, target word and count.
    pub(super) mod counts {
        use super::*;
        pub(in super::super) fn serialize<S: Serializer>(
            counts: &[(Vec<u8>, i32, i32)],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(
                counts
                    .iter()
                    .map(|(token, target, count)| (Text(token), target, count)),
            )
        }
        pub(in super::super) fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<(Vec<u8>, i32, i32)>, D::Error> {
            Ok(Vec::<(Owned, i32, i32)>::deserialize(deserializer)?
                .into_iter()
                .map(|(token, target, count)| (token.0, target, count))
                .collect())
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Member {
    container: u8,
    #[serde(with = "token")]
    token: Vec<u8>,
    state: i32,
    days: i32,
    flags: u8,
    country: Option<u32>,
    numeric: Option<i32>,
}
/// A final roster rebuild's owner as its reduction left it there: what its owner holds, as
/// kept, with the members' days and target countries of that moment, for another thread to
/// make the rebuild's [`Inputs`] of.
pub(super) struct Captured {
    /// The owner's native id.
    pub(super) owner: u32,
    sequence: u64,
    complete: bool,
    seed: Option<i32>,
    political_power: Option<i64>,
    override_admission: Option<bool>,
    kept: std::sync::Arc<super::native::retained::Kept>,
    days: Vec<i32>,
    /// The owner's target countries, with their slots once resolved.
    pub(super) countries: Vec<([u32; 3], Option<u32>)>,
}
impl Captured {
    /// The bytes it owns, itself and its kept copy included.
    pub(super) fn heap(&self) -> usize {
        size_of::<Self>()
            + self.kept.heap()
            + self.days.capacity() * size_of::<i32>()
            + self.countries.capacity() * size_of::<([u32; 3], Option<u32>)>()
    }
    /// `state` at the rebuild `event` of a read `covered` whole, with `kept` its copy.
    pub(super) fn of(
        state: &DecisionState,
        event: &EventFeedEvent,
        covered: bool,
        kept: std::sync::Arc<super::native::retained::Kept>,
    ) -> Self {
        Self {
            owner: event.actor.id,
            sequence: event.sequence,
            complete: covered && state.complete(),
            seed: event.script.and_then(|script| script.days),
            political_power: event.script.and_then(|script| match script.change {
                Some(replay_data::event_feed_rows::EventFeedScriptChange::Decision(change)) => {
                    change.political_power
                }
                _ => None,
            }),
            override_admission: event
                .script
                .and_then(|script| script.lifecycle)
                .map(|value| value != 0),
            kept,
            days: (0..CONTAINERS as u8)
                .filter_map(|container| state.members(container).ok())
                .flatten()
                .map(|member| member.days)
                .collect(),
            countries: state
                .target_countries
                .values()
                .map(|country| ([country.reference, country.id, country.instance], None))
                .collect(),
        }
    }
    /// Its inputs, as [`Inputs::captured`] takes them at the rebuild.
    pub(super) fn inputs(&self) -> Inputs {
        Inputs::of(self)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Inputs {
    sequence: u64,
    complete: bool,
    seed: Option<i32>,
    political_power: Option<i64>,
    override_admission: Option<bool>,
    members: Vec<Member>,
    #[serde(with = "token::list")]
    ignored: Vec<Vec<u8>>,
    #[serde(with = "token::counts")]
    random: Vec<(Vec<u8>, i32, i32)>,
    countries: Vec<([u32; 3], Option<u32>)>,
}
impl Inputs {
    /// The bytes it owns on the heap.
    pub(super) fn heap(&self) -> usize {
        self.members.capacity() * size_of::<Member>()
            + self
                .members
                .iter()
                .map(|member| member.token.capacity())
                .sum::<usize>()
            + self.ignored.capacity() * size_of::<Vec<u8>>()
            + self.ignored.iter().map(Vec::capacity).sum::<usize>()
            + self.random.capacity() * size_of::<(Vec<u8>, i32, i32)>()
            + self
                .random
                .iter()
                .map(|(token, _, _)| token.capacity())
                .sum::<usize>()
            + self.countries.capacity() * size_of::<([u32; 3], Option<u32>)>()
    }
    pub fn captured(state: &DecisionState, event: &EventFeedEvent, covered: bool) -> Self {
        let mut members = Vec::new();
        for container in 0..CONTAINERS as u8 {
            if let Ok(rows) = state.members(container) {
                members.extend(rows.iter().map(|row| Member {
                    container,
                    token: row.token.clone(),
                    state: row.lifecycle,
                    days: row.days,
                    flags: row.flags,
                    country: row.country_target,
                    numeric: row.numeric_target,
                }));
            }
        }
        Self {
            sequence: event.sequence,
            complete: covered && state.complete(),
            seed: event.script.and_then(|script| script.days),
            political_power: event.script.and_then(|script| match script.change {
                Some(replay_data::event_feed_rows::EventFeedScriptChange::Decision(change)) => {
                    change.political_power
                }
                _ => None,
            }),
            override_admission: event
                .script
                .and_then(|script| script.lifecycle)
                .map(|value| value != 0),
            members,
            ignored: state.ignored.iter().cloned().collect(),
            random: state
                .random_counts
                .iter()
                .map(|value| (value.token.clone(), value.target, value.count))
                .collect(),
            countries: state
                .target_countries
                .values()
                .map(|country| ([country.reference, country.id, country.instance], None))
                .collect(),
        }
    }
    /// The inputs of the final rebuild `captured` holds, as [`Self::captured`] takes them.
    fn of(captured: &Captured) -> Self {
        let decided = captured.kept.decided();
        let members = decided
            .into_iter()
            .flat_map(|decided| decided.members.iter().enumerate())
            .flat_map(|(container, rows)| rows.iter().map(move |row| (container as u8, row)))
            .zip(&captured.days)
            .map(|((container, row), &days)| Member {
                container,
                token: row.token.clone(),
                state: row.lifecycle,
                days,
                flags: row.flags,
                country: row.country_target,
                numeric: row.numeric_target,
            })
            .collect();
        Self {
            sequence: captured.sequence,
            complete: captured.complete,
            seed: captured.seed,
            political_power: captured.political_power,
            override_admission: captured.override_admission,
            members,
            ignored: decided.map_or_else(Vec::new, |decided| {
                decided.ignored.iter().cloned().collect()
            }),
            random: decided.map_or_else(Vec::new, |decided| {
                decided
                    .random_counts
                    .iter()
                    .map(|value| (value.token.clone(), value.target, value.count))
                    .collect()
            }),
            countries: captured.countries.clone(),
        }
    }
    pub fn resolve(&mut self, slot: &mut impl FnMut(EventFeedCountry) -> Option<u32>) {
        for (identity, resolved) in &mut self.countries {
            let [reference, id, instance] = *identity;
            *resolved = slot(EventFeedCountry {
                reference,
                id,
                instance,
            });
        }
    }
    fn slot(&self, reference: u32) -> Result<u32, String> {
        if reference as i32 <= 0 {
            return Ok(0);
        }
        self.countries
            .iter()
            .find(|(identity, _)| identity[0] == reference)
            .and_then(|(_, slot)| *slot)
            .ok_or_else(|| {
                format!("target country reference {reference} has no exact rebuild identity/slot")
            })
    }
    fn has(&self, container: u8, token: &[u8]) -> bool {
        self.members
            .iter()
            .any(|row| row.container == container && row.token.eq_ignore_ascii_case(token))
    }
    fn graph(
        &self,
        owner: u32,
        definition: &installed_content_core::decisions::Decision,
        target: Option<(bool, u32)>,
    ) -> Result<(Rc<RefCell<EventScopes>>, i32), String> {
        let mut own = EventScope::of(EventObject::Country(owner));
        if target.is_none() {
            let count = self
                .random
                .iter()
                .find(|(token, target, _)| {
                    token.eq_ignore_ascii_case(&definition.id) && *target == 0
                })
                .map_or(0, |(_, _, count)| *count);
            return Ok((Rc::new(RefCell::new(EventScopes::new(vec![own]))), count));
        }
        let target = target
            .map(|(state, target)| {
                if state {
                    Ok(EventObject::State(target))
                } else {
                    self.slot(target).map(EventObject::Country)
                }
            })
            .transpose()?;
        let target_word = match target {
            Some(EventObject::Country(slot)) => slot as i32,
            Some(EventObject::State(state)) => state as i32,
            _ => 0,
        };
        let count = if definition.fixed_random_seed {
            0
        } else {
            self.random
                .iter()
                .find(|(token, target, _)| {
                    token.eq_ignore_ascii_case(&definition.id) && *target == target_word
                })
                .map_or(0, |(_, _, count)| *count)
        };
        let mut from = EventScope::of(target.unwrap_or(EventObject::Country(owner)));
        if let Some(seed) = self.seed {
            let seed = seed
                .wrapping_add(definition.seed_component)
                .wrapping_add(count)
                .wrapping_add(target_word) as u32;
            own.random_seed = random::scope_seed(seed);
            own.random_counter = random::scope_counter(seed);
            from.random_seed = random::scope_seed(own.random_seed);
            from.random_counter = random::scope_counter(own.random_seed);
        } else {
            own.random_seed_refusal = Some("decision rebuild campaign seed was not recorded");
            from.random_seed_refusal = own.random_seed_refusal;
        }
        own.from = Link::Scope(1);
        from.from = Link::Scope(0);
        Ok((
            Rc::new(RefCell::new(EventScopes::new(vec![own, from]))),
            count,
        ))
    }
}
fn conjunction<'a>(
    evaluator: &Evaluator<'_, '_>,
    scopes: Scopes,
    bodies: impl IntoIterator<Item = (&'a str, &'a [installed_content_core::parser::Item])>,
) -> Result<bool, String> {
    for (name, body) in bodies {
        if !evaluator
            .evaluate(body, scopes)
            .decided()
            .map_err(|why| format!("{name}: {why}"))?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn availability(
    evaluator: &Evaluator<'_, '_>,
    scopes: Scopes,
    definition: &installed_content_core::decisions::Decision,
    category: Option<&installed_content_core::decisions::CategoryConditions>,
    inputs: &Inputs,
    targeted: bool,
    member: Option<&Member>,
) -> Result<bool, String> {
    if let Some(member) = member
        && (member.state != 0 || member.flags & REMOVE_REQUESTED != 0)
    {
        return Ok(false);
    }
    if inputs
        .override_admission
        .ok_or("decision admission override was not recorded")?
    {
        return Ok(true);
    }
    if inputs.has(TAKEN, &definition.id) || !targeted && inputs.has(REENABLE, &definition.id) {
        return Ok(false);
    }
    if definition.power_balance {
        return Err("category available: PowerBalanceMembers at this rebuild boundary has no writer certificate".into());
    }
    if !conjunction(
        evaluator,
        scopes,
        [
            (
                "category available",
                category.map_or(&[][..], |category| category.available.as_slice()),
            ),
            ("available", &definition.available),
        ],
    )? {
        return Ok(false);
    }
    if !definition.custom_cost_trigger.is_empty() {
        return evaluator
            .evaluate(&definition.custom_cost_trigger, scopes)
            .decided()
            .map_err(|why| format!("custom_cost_trigger: {why}"));
    }
    let cost = if definition.mission && !definition.selectable_mission || definition.cost.is_empty()
    {
        0
    } else {
        evaluator
            .scoped_value(&definition.cost, scopes)
            .map_err(|why| format!("cost: {why}"))?
    };
    if cost <= 0 {
        Ok(true)
    } else {
        inputs.political_power.map(|power| power >= cost).ok_or_else(|| "cost: Countries.political_power_raw at this native rebuild boundary has no writer certificate".into())
    }
}
pub(super) fn present(
    builder: &mut Builder,
    catalogue: &Catalogue,
    item: &mut EventFeedItem,
    inputs: Option<&script_effects::Inputs>,
) -> io::Result<()> {
    let Some((inputs, roster)) =
        inputs.and_then(|inputs| inputs.roster.as_ref().map(|roster| (inputs, roster)))
    else {
        builder.unavailable(
            "decisionRosterBoundary",
            "Decision roster rebuild",
            "rebuild retained-state operands absent",
            Some(1),
        );
        return Ok(());
    };
    if !roster.complete {
        builder.unavailable(
            "decisionRosterBoundary",
            "Decision roster rebuild",
            format!(
                "Rebuild {}: owner baseline, writer or pass certificate is incomplete",
                roster.sequence
            ),
            Some(1),
        );
        return Ok(());
    }
    let result = (|| -> Result<String, String> {
        let owner = inputs
            .actor_slot
            .ok_or("decision rebuild owner has no exact canonical slot")?;
        let scripted = catalogue.scripted_content()?;
        let effects = scripted.effects()?;
        let decisions = effects
            .view(None)
            .decisions
            .ok_or("decision database unavailable from recorded content")?;
        let frame = replay_frame::frame::DecodedFrame {
            branch: 0,
            hour: inputs.hour,
            metrics: Default::default(),
            tables: std::array::from_fn(|_| {
                replay_frame::frame::TableView::Unavailable {
                    reason:
                        "mutable input at the decision rebuild boundary has no writer certificate"
                            .into(),
                }
                .into()
            }),
        };
        let text = ScriptedText::new(&frame, &catalogue.localisation, scripted, owner);
        let world = text.trigger_world()?;
        let mut lines = vec![format!(
            "Decision roster at hour {}, observation {}",
            inputs.hour, roster.sequence
        )];
        for definition in decisions.roster()? {
            let token = String::from_utf8_lossy(&definition.id);
            let category = decisions.category_conditions.get(&definition.category);
            let targets: Vec<_> = if definition.targeted {
                roster
                    .members
                    .iter()
                    .filter(|member| {
                        matches!(member.container, TARGETED | TARGETED_TIMED)
                            && member.state != 2
                            && member.token.eq_ignore_ascii_case(&definition.id)
                    })
                    .map(|member| {
                        (
                            Some((
                                member.country.unwrap_or(0) as i32 <= 0,
                                if member.country.unwrap_or(0) as i32 > 0 {
                                    member.country.unwrap()
                                } else {
                                    member.numeric.unwrap_or(0) as u32
                                },
                            )),
                            Some(member),
                        )
                    })
                    .collect()
            } else if definition.mission {
                roster
                    .members
                    .iter()
                    .filter(|member| {
                        member.container == TIMED
                            && member.token.eq_ignore_ascii_case(&definition.id)
                            && member.state != 4
                    })
                    .map(|member| (None, Some(member)))
                    .collect()
            } else {
                vec![(None, None)]
            };
            for (target, member) in targets {
                let display = target.map_or_else(
                    || token.to_string(),
                    |(state, id)| {
                        format!("{token} ({} {id})", if state { "state" } else { "country" })
                    },
                );
                let evaluate = (|| -> Result<Option<String>, String> {
                    if let Some(reason) = &definition.refused {
                        return Err(reason.clone());
                    }
                    let base = Evaluator::new(world, None);
                    let country = Scopes::of(Scope::Country(owner));
                    if !conjunction(
                        &base,
                        country,
                        [
                            (
                                "category allowed",
                                category.map_or(&[][..], |category| category.allowed.as_slice()),
                            ),
                            ("allowed", &definition.allowed),
                        ],
                    )? {
                        return Ok(None);
                    }
                    let (graph, count) = roster.graph(owner, definition, target)?;
                    let evaluator = Evaluator::new(world, None).with_event_scopes(graph);
                    let scopes = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
                    if !conjunction(
                        &evaluator,
                        scopes,
                        [
                            (
                                "category visible",
                                category.map_or(&[][..], |category| category.visible.as_slice()),
                            ),
                            ("visible", &definition.visible),
                        ],
                    )? {
                        return Ok(None);
                    }
                    let ignored = member.map_or_else(
                        || {
                            roster
                                .ignored
                                .iter()
                                .any(|token| token.eq_ignore_ascii_case(&definition.id))
                        },
                        |member| member.flags & IGNORED != 0,
                    );
                    let state = member.map_or_else(
                        || {
                            if roster.has(TAKEN, &definition.id) {
                                "taken".to_owned()
                            } else if roster.has(REENABLE, &definition.id) {
                                "cooling down".to_owned()
                            } else if definition.mission && !roster.has(TIMED, &definition.id) {
                                "inactive mission".to_owned()
                            } else {
                                "idle".to_owned()
                            }
                        },
                        |member| format!("status {}, {} days", member.state, member.days),
                    );
                    let available = availability(
                        &evaluator,
                        scopes,
                        definition,
                        category,
                        roster,
                        definition.targeted,
                        member,
                    );
                    let available = match available {
                        Ok(available) => available.to_string(),
                        Err(reason) => {
                            builder.unavailable(
                                "decisionRosterOperand",
                                "Decision availability",
                                format!(
                                    "{display} available at observation {}: {reason}",
                                    roster.sequence
                                ),
                                Some(1),
                            );
                            "unavailable operand".into()
                        }
                    };
                    Ok(Some(format!(
                        "{display}: visible; available {available}; {state}; ignored {ignored}; description count {count}"
                    )))
                })();
                match evaluate {
                    Ok(Some(line)) => lines.push(line),
                    Ok(None) => {}
                    Err(reason) => builder.unavailable(
                        "decisionRosterOperand",
                        "Decision visibility",
                        format!(
                            "{display} visible at observation {}: {reason}",
                            roster.sequence
                        ),
                        Some(1),
                    ),
                }
            }
        }
        Ok(lines.join("\n"))
    })();
    match result {
        Ok(description) => {
            item.name = Some("Decision roster rebuilt".into());
            item.description = Some(description);
        }
        Err(reason) => builder.unavailable(
            "decisionRosterOperand",
            "Decision roster rebuild",
            reason,
            Some(1),
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> Inputs {
        Inputs {
            sequence: 9,
            complete: true,
            seed: Some(7),
            political_power: Some(100 * 100_000),
            override_admission: Some(false),
            members: Vec::new(),
            ignored: Vec::new(),
            random: Vec::new(),
            countries: Vec::new(),
        }
    }
    #[test]
    fn retained_description_counts_seed_both_scopes_and_signed_target_words() {
        let mut inputs = inputs();
        inputs.random.push((b"mod_target".to_vec(), 42, 5));
        let definition = installed_content_core::decisions::Decision {
            id: b"mod_target".to_vec(),
            seed_component: 11,
            ..Default::default()
        };
        let (graph, count) = inputs.graph(3, &definition, Some((true, 42))).unwrap();
        assert_eq!(count, 5);
        let graph = graph.borrow();
        let seed = 7_u32 + 11 + 5 + 42;
        assert_eq!(
            (graph.scopes[0].random_seed, graph.scopes[0].random_counter),
            (random::scope_seed(seed), random::scope_counter(seed))
        );
        assert_eq!(
            graph.scopes[1].random_seed,
            random::scope_seed(graph.scopes[0].random_seed)
        );
        assert_eq!(graph.scopes[1].this, EventObject::State(42));
        assert!(
            inputs
                .graph(3, &definition, Some((false, 17)))
                .unwrap_err()
                .contains("country reference 17")
        );
    }
    #[test]
    fn roster_uses_shared_triggers_and_names_only_the_missing_operand() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "common/decisions/categories/synthetic.txt",
                "category={allowed={always=yes}}",
            ),
            (
                "common/decisions/synthetic.txt",
                "category={shown={visible={always=yes} available={always=yes} cost=0} paid={cost=10} expensive={cost=101} hidden={visible={always=no}} cooling={cost=0} missing={visible={has_country_flag=unrecorded_flag}}}",
            ),
        ]);
        let event = EventFeedEvent {
            sequence: 9,
            generation: 1,
            hour: 7,
            source: replay_data::event_feed_rows::EventFeedSource::ScriptState,
            actor: EventFeedCountry {
                reference: 1,
                id: 1,
                instance: 1,
            },
            target: Default::default(),
            cause: replay_data::rows::Recorded::Value(Default::default()),
            script: Some(Default::default()),
        };
        let mut captured = script_effects::Inputs::captured(&Default::default(), &event);
        captured.actor_slot = Some(1);
        let mut roster = inputs();
        roster.members.push(Member {
            container: REENABLE,
            token: b"cooling".to_vec(),
            state: 0,
            days: 5,
            flags: 0,
            country: None,
            numeric: None,
        });
        captured.roster = Some(roster.clone());
        let mut builder = Builder::new(0, 10);
        let mut item = plain_item(None, None, None, None);
        present(&mut builder, &catalogue, &mut item, Some(&captured)).unwrap();
        let description = item.description.unwrap();
        assert!(description.contains("shown: visible; available true"));
        assert!(description.contains("cooling: visible; available false"));
        assert!(description.contains("paid: visible; available true"));
        assert!(description.contains("expensive: visible; available false"));
        assert!(!description.contains("hidden:"));
        for (power, expected) in [
            (Some(5 * 100_000), "available false"),
            (Some(10 * 100_000), "available true"),
            (None, "availability unavailable"),
        ] {
            roster.political_power = power;
            captured.roster = Some(roster.clone());
            let mut check = Builder::new(0, 10);
            let mut item = plain_item(None, None, None, None);
            present(&mut check, &catalogue, &mut item, Some(&captured)).unwrap();
            let description = item.description.unwrap();
            if power.is_some() {
                assert!(
                    description.contains(&format!("paid: visible; {expected}")),
                    "{description}"
                );
            } else {
                assert!(
                    check
                        .feed
                        .unavailable
                        .iter()
                        .any(|row| row.reason.contains("paid")
                            && row.reason.contains("political_power_raw"))
                );
            }
            assert!(description.contains("shown: visible; available true"));
        }

        assert!(
            builder
                .feed
                .unavailable
                .iter()
                .any(|row| row.reason.contains("missing visible")
                    && row.reason.contains("has_country_flag"))
        );
        assert!(
            !builder
                .feed
                .unavailable
                .iter()
                .any(|row| row.kind == "decisionRosterBoundary")
        );
    }
}
