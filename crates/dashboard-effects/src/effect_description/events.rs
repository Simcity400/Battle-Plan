
use super::*;
use crate::{
    scripted_text::{EventObject, EventScope, Link, random},
    triggers::{NULL_STATE, Scope},
};
use content_reader_politics::decisions::Decision;
use replay_data::rows::{
    ClockRow, CountryAppearanceRow, DecisionRandomItemRow, DecisionRow, PoliticalPartyRow,
    PoliticsRow, tables,
};

const DECISION: u64 = mask::<DecisionRow>(&[
    "token",
    "country_target",
    "numeric_target",
    "description_seed",
]);
const RANDOM_ITEM: u64 = mask::<DecisionRandomItemRow>(&["token", "target", "count"]);
const SEED: u64 = mask::<ClockRow>(&["seed"]);
const COSMETIC: u64 = mask::<CountryAppearanceRow>(&["cosmetic_tag"]);
const RULING: u64 = mask::<PoliticsRow>(&["ruling_party"]);
const PARTY_TOKEN: u64 = mask::<PoliticalPartyRow>(&["token"]);

const DECISION_RANDOM_COUNT_GAP: &str =
    "decision random items is not complete for this decision-description seed";

mod descriptions;
#[cfg(test)]
mod tests;

fn matching<'a>(
    rows: Rowset<'a, DecisionRow>,
    definition: &[u8],
    owner: u32,
    mission: bool,
    target_country: Option<u32>,
    target_state: u32,
    slot: impl Fn(u32) -> Option<u32>,
) -> Result<Option<Cow<'a, DecisionRow>>, String> {
    let container: u8 = if mission { 4 } else { 3 };
    let mut candidates = Vec::new();
    for row in rows.keyed(|key| (key >> 32) as u8 == container) {
        let row = row.map_err(built)?;
        if row.token.eq_ignore_ascii_case(definition) {
            candidates.push(row);
        }
    }
    candidates.sort_by_key(|row| row.ordinal);
    for row in candidates {
        let row_owner = slot(row.country)
            .ok_or_else(|| format!("country links lacks Decisions owner {}", row.country))?;
        if row_owner != owner || row.numeric_target != target_state {
            continue;
        }
        let country = if row.country_target == 0 {
            None
        } else {
            Some(slot(row.country_target).ok_or_else(|| {
                format!("country links lacks Decisions target {}", row.country_target)
            })?)
        };
        if country == target_country {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn decision_seed(
    game_seed: i32,
    definition: &Decision,
    additive: Option<Option<i32>>,
    target: i32,
) -> Result<u32, String> {
    let additive = if definition.fixed_random_seed {
        0
    } else {
        additive
            .ok_or("Decisions description_seed is unread")?
            .unwrap_or(0)
    };
    Ok(game_seed
        .wrapping_add(definition.seed_component)
        .wrapping_add(additive)
        .wrapping_add(target) as u32)
}

fn name_key(
    base: &str,
    cosmetic: &str,
    tag: &str,
    ideology: &str,
    exists: impl Fn(&str) -> bool,
) -> String {
    let suffix = format!("_{ideology}");
    let mut candidates = Vec::new();
    if !cosmetic.is_empty() {
        candidates.extend([
            format!("{cosmetic}_{base}{suffix}"),
            format!("{cosmetic}_{base}"),
        ]);
    }
    candidates.extend([
        format!("{tag}_{base}{suffix}"),
        format!("{tag}_{base}"),
        format!("{base}{suffix}"),
        base.to_owned(),
    ]);
    candidates
        .into_iter()
        .find(|key| exists(key))
        .unwrap_or_else(|| base.to_owned())
}

pub fn decision_name_key_at<'f>(
    frame: impl Into<HourFrame<'f>>,
    recorded: &crate::scope::RecordedScopes<'_>,
    localisation: &Localisation,
    definition: &Decision,
    country: u32,
    tag: &str,
) -> Result<String, String> {
    let frame = frame.into();
    let ours = |key: u128| recorded.slot(key as u32) == Some(country);
    let appearance = frame
        .complete::<tables::CountryAppearance>(COSMETIC)
        .map_err(|why| why.to_string())?
        .keyed(ours)
        .next()
        .transpose()
        .map_err(built)?
        .ok_or("country appearance lacks decision-name cosmetic tag")?;
    let politics = frame
        .complete::<tables::Politics>(RULING)
        .map_err(|why| why.to_string())?
        .keyed(ours)
        .next()
        .transpose()
        .map_err(built)?
        .ok_or("Politics lacks decision-name ruling party")?;
    let ruling = politics
        .ruling_party
        .ok_or("decision-name ruling party is absent")?;
    let party = frame
        .complete::<tables::PoliticalParties>(PARTY_TOKEN)
        .map_err(|why| why.to_string())?
        .keyed(|key| recorded.slot((key >> 32) as u32) == Some(country) && key as u32 == ruling)
        .next()
        .transpose()
        .map_err(built)?
        .ok_or("political parties lacks decision-name ruling ideology")?;
    let utf8 = |bytes: &[u8]| {
        std::str::from_utf8(content_reader_parser::parser::c_string(bytes))
            .map(str::to_owned)
            .map_err(|_| "decision-name key is not UTF-8".to_owned())
    };
    Ok(name_key(
        &utf8(&definition.name)?,
        &utf8(&appearance.cosmetic_tag)?,
        tag,
        &utf8(&party.token)?,
        |key| localisation.get(key).is_some(),
    ))
}

impl Renderer<'_, '_, '_> {
    pub(super) fn decision_remove_days(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "decision")
            .and_then(|item| item.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or("decision content is unread")?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let name = self.decision_title(definition, scopes)?;
        let days = fields
            .iter()
            .filter(|item| item.keyword_is("days"))
            .filter_map(|item| {
                item.value
                    .scalar()
                    .and_then(|value| content_reader_parser::crt::scan_i32(value).stored())
            })
            .next_back()
            .unwrap_or(0);
        self.line(
            if days < 0 {
                b"REMOVE_DAYS_TO_DECISION"
            } else {
                b"ADD_DAYS_TO_DECISION"
            },
            &[
                ("DAYS", Argument::Int32(days.wrapping_neg())),
                ("DECISION", Argument::Text(&name)),
            ],
            scopes,
            true,
        )
    }

    fn decision_title(&self, definition: &Decision, scopes: Scopes) -> Result<String, String> {
        let label_key = self.decision_name_key(definition, self.current_country(scopes)?)?;
        self.localised(label_key.as_bytes(), &[], scopes, true)
    }

    pub(super) fn custom_tooltip(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        self.bound_localisation(
            &content_reader_script::localization_binding::BoundLocalization::from_value(&item.value),
            scopes,
            true,
        )
    }

    pub(super) fn country_event(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if let Some(desc) = field(children(item), "desc")
            .and_then(|item| item.value.read_string())
            .filter(|desc| !desc.is_empty())
        {
            return self.line(desc, &[], scopes, true);
        }
        let id = item
            .value
            .scalar()
            .or_else(|| field(children(item), "id").and_then(|item| item.value.scalar()))
            .ok_or("country_event has no id")?;
        let event = (self
            .context
            .content
            .events
            .ok_or("referenced event lookup unavailable")?)(id)?;
        if let Some(why) = &event.refused {
            return Err(why.clone());
        }
        let mut title_scope = EventScope::of(EventObject::Country(self.current_country(scopes)?));
        let cursor = scopes.cursor.unwrap_or(0);
        let at = {
            let mut graph = self.graph.borrow_mut();
            title_scope.from = graph.scopes[cursor].root;
            if title_scope.from == Link::Itself {
                title_scope.from = Link::Scope(cursor);
            }
            let parent = &mut graph.scopes[cursor];
            let seed = crate::scripted_text::random::scoped_draw(
                parent.random_seed,
                parent.random_counter,
            ) as u32;
            parent.random_seed = parent.random_seed.wrapping_add(1);
            title_scope.random_seed = crate::scripted_text::random::scope_seed(seed);
            title_scope.random_counter = crate::scripted_text::random::scope_counter(seed);
            title_scope.random_seed_refusal = parent.random_seed_refusal;
            let at = graph.scopes.len();
            graph.scopes.push(title_scope);
            at
        };
        let selected = self
            .evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())?;
        let title = match event.sole_title_key()? {
            Some(key) => self.localised(key, &[], selected, true)?,
            None => String::new(),
        };
        self.localised(
            b"EFFECT_COUNTRY_EVENT",
            &[("EVENT", Argument::Text(&title))],
            scopes,
            true,
        )
    }

    pub(super) fn removed_decision(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "decision")
            .and_then(|value| value.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or("decision content is unread")?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let owner = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => return Err("remove_targeted_decision scope has no country".into()),
        };
        let (country, state) = self.decision_target(field(fields, "target"), scopes)?;
        if country.is_none() && state == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let rows = world
            .tables()
            .complete::<tables::Decisions>(DECISION)
            .map_err(|why| why.to_string())?;
        let Some(row) = matching(
            rows,
            &definition.id,
            owner,
            definition.mission,
            country,
            state,
            |raw| world.recorded().slot(raw),
        )?
        else {
            return Ok(String::new());
        };
        if let Some(why) = &definition.refused {
            return Err(why.clone());
        }
        let title_scope = self.decision_title_scope(definition, &row, owner)?;
        let name = self.decision_title(definition, title_scope)?;
        let mut text = self.localised(
            b"EFFECT_REMOVE_TARGETED_DECISION",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )?;
        text.push('\n');
        Ok(text)
    }

    fn decision_target(
        &self,
        target: Option<&Item>,
        scopes: Scopes,
    ) -> Result<(Option<u32>, u32), String> {
        let evaluator = self.evaluator()?;
        if let Some(item) = target {
            let value = item.value.scalar().unwrap_or_default();
            let text = std::str::from_utf8(content_reader_parser::parser::c_string(value))
                .map_err(|_| "decision target is not UTF-8")?;
            let parsed =
                content_reader_script::scope::parse(text, item.quoted(), self.context.world()?.recorded());
            if matches!(
                parsed,
                content_reader_script::scope::Scope::Keyword(_)
                    | content_reader_script::scope::Scope::EventTarget { .. }
            ) {
                return Ok(
                    match evaluator
                        .enter_scope(&parsed, scopes)
                        .map_err(|why| why.to_string())?
                        .map(|selected| selected.this)
                    {
                        Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => {
                            (Some(country), 0)
                        }
                        Some(Scope::State(state)) if state != NULL_STATE => (None, state),
                        _ => (None, 0),
                    },
                );
            }
            if let Some(country) = evaluator
                .scoped_country(value, scopes)
                .map_err(|why| why.to_string())?
            {
                return Ok((Some(country), 0));
            }
            let state = evaluator
                .scoped_state_id(value, scopes)
                .map_err(|why| why.to_string())? as u32;
            if state != 0 {
                return Ok((None, state));
            }
        }
        Ok(match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => (Some(country), 0),
            Scope::State(state) if state != NULL_STATE => (None, state),
            _ => (None, 0),
        })
    }

    fn decision_name_key(&self, definition: &Decision, country: u32) -> Result<String, String> {
        let world = self.context.world()?;
        decision_name_key_at(
            world.tables(),
            world.recorded(),
            self.context.localisation,
            definition,
            country,
            self.tag(country)?,
        )
    }

    fn decision_title_scope(
        &self,
        definition: &Decision,
        row: &DecisionRow,
        owner: u32,
    ) -> Result<Scopes, String> {
        let country_target = if row.country_target == 0 {
            None
        } else {
            Some(
                self.context
                    .world()?
                    .recorded()
                    .slot(row.country_target)
                    .ok_or("country links lacks decision title target")?,
            )
        };
        let target = country_target.unwrap_or(row.numeric_target) as i32;
        let seed = self.decision_description_seed(definition, Some(row), owner, target);
        let (at, from) = self.seeded_decision_graph(
            owner,
            Some(match country_target {
                Some(country) => EventObject::Country(country),
                None => EventObject::State(row.numeric_target),
            }),
            seed,
        );
        let scopes = self
            .evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())?;
        let highlighted = definition.has_state_targets
            || self
                .evaluator()?
                .has_children(&definition.highlight_conditions)
                .map_err(|why| why.to_string())?;
        if highlighted && let Some(state) = self.highlight_state(definition, scopes)? {
            let mut graph = self.graph.borrow_mut();
            let index = graph.scopes.len();
            graph.scopes.push(EventScope::of(EventObject::State(state)));
            graph.scopes[from].from = Link::Scope(index);
        }
        self.evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())
    }

    fn decision_description_seed(
        &self,
        definition: &Decision,
        row: Option<&DecisionRow>,
        owner: u32,
        target: i32,
    ) -> Result<u32, &'static str> {
        let game_seed = self
            .context
            .world()
            .map_err(|_| "Clock decision-description seed is unavailable")?
            .tables()
            .complete::<tables::Clock>(SEED)
            .map_err(|_| "Clock decision-description seed is unavailable")?
            .iter()
            .next()
            .transpose()
            .map_err(|_| "Clock decision-description seed is unavailable")?
            .map(|row| row.seed)
            .ok_or("Clock lacks decision-description seed")?;
        let additive = if definition.fixed_random_seed {
            None
        } else if let Some(row) = row {
            *row.description_seed
                .require("description seed of decisions")
                .map_err(|_| "description seed of decisions not recorded")?
        } else {
            let world = self
                .context
                .world()
                .map_err(|_| DECISION_RANDOM_COUNT_GAP)?;
            let rows = world
                .tables()
                .complete::<tables::DecisionRandomItems>(RANDOM_ITEM)
                .map_err(|_| DECISION_RANDOM_COUNT_GAP)?;
            let mut first: Option<Cow<'_, DecisionRandomItemRow>> = None;
            for row in rows.keyed(|key| world.slot((key >> 32) as u32) == Some(owner)) {
                let row = row.map_err(|_| DECISION_RANDOM_COUNT_GAP)?;
                if row.token.eq_ignore_ascii_case(&definition.id)
                    && row.target == target as u32
                    && first.as_ref().is_none_or(|first| row.ordinal < first.ordinal)
                {
                    first = Some(row);
                }
            }
            Some(first.map(|row| row.count))
        };
        decision_seed(game_seed, definition, additive, target)
            .map_err(|_| "Decisions description_seed is unread")
    }

    fn seeded_decision_graph(
        &self,
        owner: u32,
        target: Option<EventObject>,
        seed: Result<u32, &'static str>,
    ) -> (usize, usize) {
        let mut graph = self.graph.borrow_mut();
        let at = graph.scopes.len();
        let mut own = EventScope::of(EventObject::Country(owner));
        let mut from = EventScope::of(target.unwrap_or(EventObject::Country(owner)));
        match seed {
            Ok(seed) => {
                own.random_seed = random::scope_seed(seed);
                own.random_counter = random::scope_counter(seed);
                from.random_seed = random::scope_seed(own.random_seed);
                from.random_counter = random::scope_counter(own.random_seed);
            }
            Err(why) => {
                own.random_seed_refusal = Some(why);
                from.random_seed_refusal = Some(why);
            }
        }
        if target.is_some() {
            own.from = Link::Scope(at + 1);
            graph.scopes.extend([own, from]);
            (at, at + 1)
        } else {
            graph.scopes.push(from);
            (at, at)
        }
    }

    fn fresh_country_scope(&self, owner: u32, target: Option<u32>) -> Result<Scopes, String> {
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            let mut scope = EventScope::of(EventObject::Country(owner));
            scope.random_seed = random::scope_seed(0);
            scope.random_counter = random::scope_counter(0);
            if let Some(state) = target {
                scope.from = Link::Scope(at + 1);
                graph.scopes.push(scope);
                graph.scopes.push(EventScope::of(EventObject::State(state)));
            } else {
                graph.scopes.push(scope);
            }
            at
        };
        self.evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())
    }

    fn highlight_state(
        &self,
        definition: &Decision,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        if let Scope::State(state) = scopes.this
            && state != NULL_STATE
            && state != 0
        {
            return Ok(Some(state));
        }
        if let Some(Scope::State(state)) = scopes.from
            && state != NULL_STATE
            && state != 0
        {
            return Ok(Some(state));
        }
        let evaluator = self.evaluator()?;
        self.context
            .world()?
            .tables()
            .complete::<tables::States>(0)
            .map_err(|why| why.to_string())?;
        let owner = match scopes.this {
            Scope::Country(owner) | Scope::CountryLeader { country: owner, .. } => owner,
            _ => return Ok(None),
        };
        let mut candidates = std::collections::BTreeSet::new();
        let root = self.fresh_country_scope(owner, None)?;
        if evaluator
            .evaluate(&definition.target_root_trigger, root)
            .decided()?
        {
            for array in &definition.target_arrays {
                for object in evaluator
                    .scoped_array_scopes(array, scopes)
                    .map_err(|why| why.to_string())?
                {
                    if let Some(Scope::State(state)) = object
                        && state > 0
                        && state != NULL_STATE
                        && self.context.world()?.states().contains_key(&state)
                        && evaluator
                            .evaluate(
                                &definition.target_trigger,
                                self.fresh_country_scope(owner, Some(state))?,
                            )
                            .decided()?
                    {
                        candidates.insert(state);
                    }
                }
            }
            for operand in &definition.target_states {
                let state = evaluator
                    .scoped_state_id(operand, scopes)
                    .map_err(|why| why.to_string())? as u32;
                if state > 0
                    && state != NULL_STATE
                    && self.context.world()?.states().contains_key(&state)
                    && evaluator
                        .evaluate(
                            &definition.target_trigger,
                            self.fresh_country_scope(owner, Some(state))?,
                        )
                        .decided()?
                {
                    candidates.insert(state);
                }
            }
        }
        if candidates.is_empty()
            && evaluator
                .has_children(&definition.highlight_conditions)
                .map_err(|why| why.to_string())?
        {
            for &state in self
                .context
                .world()?
                .states()
                .keys()
                .filter(|state| **state > 0)
            {
                let selected = evaluator
                    .with_this(scopes, Scope::State(state))
                    .map_err(|why| why.to_string())?;
                if evaluator
                    .evaluate(&definition.highlight_conditions, selected)
                    .decided()?
                {
                    candidates.insert(state);
                }
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let graph = self.graph.borrow();
        let scope = &graph.scopes[scopes
            .cursor
            .ok_or("decision highlight lacks scope cursor")?];
        let index = scope.random_draw()? as usize % candidates.len();
        Ok(candidates.into_iter().nth(index))
    }
}

#[cfg(test)]
mod politics_days {
    use super::*;
    use crate::fixtures::game;
    use crate::{fixtures::folder_catalogue, effect_description::tests::Fixture};
    use replay_data::{rows::PoliticalPartyRow, rows::PoliticsRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;

    #[test]
    fn remove_days_uses_caller_title_scope_and_negates_each_branch_without_instance_reads() {
        game::enter();
        let mut fixture = Fixture::new();
        let (catalogue, _folder) = folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
            ),
            (
                "localisation/test_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                    " ADD_DAYS_TO_DECISION:0 \"Add $DAYS|H$ to $DECISION|H$ in [Root.GetTag]\"\n",
                    " REMOVE_DAYS_TO_DECISION:0 \"Remove $DAYS|H$ from $DECISION|H$ in [Root.GetTag]\"\n",
                    " SYN_DECISION:0 \"Wrong base\"\n",
                    " SYN_TITLE:0 \"Wrong fallback\"\n",
                    " AAA_SYN_TITLE_group:0 \"Mission [From.GetTag]/[Prev.GetTag]/[Root.GetTag]\"\n"
                ),
            ),
        ]);
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        fixture.catalogue = catalogue;
        *fixture.frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 100,
                ruling_party: Some(7),
                ..Default::default()
            }]));
        *fixture.frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 100,
                id: 7,
                token: b"group".to_vec(),
                ..Default::default()
            }]));
        for table in [TableId::Clock, TableId::Decisions] {
            *fixture.frame.tables[table.index()] = TableView::Unavailable {
                reason: "unused decision execution data".into(),
            };
        }
        let mut database = content_reader_politics::decisions::Database::default();
        database.entries.insert(
            b"syn_decision".to_vec(),
            Decision {
                id: b"SYN_DECISION".to_vec(),
                name: b"SYN_TITLE".to_vec(),
                ..Default::default()
            },
        );
        let mut caller = EventScope::of(EventObject::Country(1));
        caller.root = Link::Scope(1);
        caller.from = Link::Scope(2);
        caller.random_seed = 123;
        caller.random_counter = 456;
        let mut scopes = EventScopes::new(vec![
            caller,
            EventScope::of(EventObject::Country(3)),
            EventScope::of(EventObject::Country(2)),
        ]);
        for (days, expected) in [
            (
                "-43",
                "Remove \u{11}H43\u{11}! from \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "43",
                "Add \u{11}H-43\u{11}! to \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "0",
                "Add \u{11}H0\u{11}! to \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "-2147483648",
                "Remove \u{11}H-2147483648\u{11}! from \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "-053 days = invalid",
                "Remove \u{11}H43\u{11}! from \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
        ] {
            let output = fixture.render(
                &format!("fixture_removal_clock = {{ days = {days} decision = SYN_DECISION }}"),
                &mut scopes,
                Content {
                    decisions: Some(&database),
                    ..Default::default()
                },
            );
            assert_eq!(output.text, expected, "days={days}");
            assert!(output.refusals.is_empty(), "{:?}", output.refusals);
            assert_eq!(scopes.scopes[0], caller);
            assert_eq!(scopes.scopes.len(), 3);
        }
        let absent = fixture.render(
            "fixture_removal_clock = { days = -43 decision = ABSENT }",
            &mut scopes,
            Content {
                decisions: Some(&database),
                ..Default::default()
            },
        );
        assert_eq!(absent.text, "");
        assert!(absent.refusals.is_empty());
    }
}
