use super::*;
use replay_data::rows::{
    CountryCounterInputRow, CountryRow, IndustrialOrganisationRow, SimulationFeatureRow,
};

const COUNTRY_TAG: usize = replay_data::rows::column::<CountryRow>("tag");
const ORGANISATION: u64 = mask::<IndustrialOrganisationRow>(&["country", "ordinal", "token"]);
const MIO_POLICIES: u64 = mask::<CountryCounterInputRow>(&["mio_policies"]);
const INSTANT: u64 = mask::<SimulationFeatureRow>(&["instant_decisions"]);

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn decision_effect(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mission = item.keyword_is("activate_mission")
            || item.keyword_is("remove_mission")
            || item.keyword_is("activate_mission_tooltip");
        let id = item
            .value
            .scalar()
            .or_else(|| {
                mission
                    .then(|| {
                        field(children(item), "mission").and_then(|field| field.value.read_string())
                    })
                    .flatten()
            })
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or_else(|| self.content_gap("decisions"))?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let mut name = self.decision_title(definition, scopes)?;
        if item.keyword_is("activate_mission_tooltip")
            && let Some(desc) = field(children(item), "desc")
                .and_then(|field| field.value.read_string())
                .filter(|desc| !desc.is_empty())
        {
            name.push_str(&self.line(desc, &[], scopes, true)?);
        }
        self.char_native_line(
            match key(item).to_ascii_lowercase().as_str() {
                "activate_decision" => b"EFFECT_ACTIVATE_DECISION",
                "remove_decision" => b"EFFECT_REMOVE_DECISION",
                "remove_mission" => b"EFFECT_REMOVE_MISSION",
                _ => b"EFFECT_ACTIVATES_MISSION",
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }


    pub(in crate::effect_description) fn mission_days(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "mission")
            .and_then(|field| field.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or_else(|| self.content_gap("decisions"))?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let amount = field(fields, "days")
            .and_then(|field| field.value.scalar())
            .map(|operand| self.scoped_value(operand, scopes))
            .transpose()?
            .unwrap_or(0);
        let days = crate::triggers::fixed_round(amount) as i32;
        if days == 0 {
            return Ok(String::new());
        }
        let name = self.decision_title(definition, scopes)?;
        self.char_native_line(
            if days < 0 {
                b"REMOVE_DAYS_TO_MISSION"
            } else {
                b"ADD_DAYS_TO_MISSION"
            },
            &[
                (
                    "DAYS",
                    Argument::Int32(if days < 0 { days.wrapping_neg() } else { days }),
                ),
                ("MISSION", Argument::Text(&name)),
            ],
            scopes,
            true,
        )
    }


    pub(in crate::effect_description) fn decision_category(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let database = self
            .context
            .content
            .decisions
            .ok_or_else(|| self.content_gap("decisions"))?;
        let Some(name) = database.category(item.value.scalar().unwrap_or_default())? else {
            return Ok(String::new());
        };
        let name = self.localised(name, &[], scopes, true)?;
        self.char_native_line(
            b"EFFECT_UNLOCK_DECISION_CATEGORY",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn unlock_decision(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = item
            .value
            .scalar()
            .or_else(|| field(fields, "decision").and_then(|field| field.value.read_string()))
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or_else(|| self.content_gap("decisions"))?;
        let Some(definition) = database.decision(id)? else {
            return self.char_native_line(
                b"EFFECT_UNLOCK_DECISION",
                &[("NAME", Argument::Text("")), ("EFFECT", Argument::Text(""))],
                scopes,
                true,
            );
        };
        let name = self.decision_title(definition, scopes)?;
        let mut effects = String::new();
        if content_reader_parser::parser::read_bool_field(fields, "show_effect_tooltip", false) {
            for (body, header) in [
                (
                    &definition.complete_effect,
                    b"EFFECT_UNLOCK_DECISION_SELECT_HEADER".as_slice(),
                ),
                (
                    &definition.remove_effect,
                    b"EFFECT_UNLOCK_DECISION_REMOVE_HEADER".as_slice(),
                ),
            ] {
                if body.is_empty() {
                    continue;
                }
                let selected = self.decision_seeded_scope(
                    definition,
                    self.current_country(scopes)?,
                    None,
                    0,
                    false,
                );
                match selected {
                    Ok(selected) => {
                        let text = self.group(body, selected, 0);
                        if !text.is_empty() {
                            effects.push_str(&self.char_native_line(header, &[], scopes, true)?);
                            effects.push_str(&text);
                            if header == b"EFFECT_UNLOCK_DECISION_SELECT_HEADER" {
                                match self.decision_removal_delay(definition, scopes) {
                                    Ok(days) if days > 0 => {
                                        effects.push_str(&self.char_native_line(
                                            b"EFFECT_UNLOCK_DECISION_TIMED_HEADER",
                                            &[("DAYS", Argument::Int32(days))],
                                            scopes,
                                            true,
                                        )?);
                                    }
                                    Ok(_) => {}
                                    Err(why) => self.refuse(
                                        "decision tooltip",
                                        format!("key unlock_decision_tooltip: {why}"),
                                    ),
                                }
                            }
                        }
                    }
                    Err(why) => self.refuse(
                        "decision tooltip",
                        format!("key unlock_decision_tooltip: {why}"),
                    ),
                }
            }
            if !effects.is_empty() {
                effects.insert(0, '\n');
            }
        }
        if content_reader_parser::parser::read_bool_field(fields, "show_modifiers", false)
            && (!definition.modifier.is_empty() || !definition.targeted_modifier.is_empty())
        {
            match self.decision_modifier_details(definition, scopes) {
                Ok(modifiers) if !modifiers.is_empty() => {
                    if effects.is_empty() || effects.ends_with('\n') {
                        effects.push('\n');
                    }
                    effects.push_str(&self.localised(
                        b"DECISION_WHILE_ACTIVE",
                        &[],
                        scopes,
                        true,
                    )?);
                    effects.push('\n');
                    effects.push_str(&modifiers);
                }
                Ok(_) => {}
                Err(why) => self.refuse(
                    "decision tooltip",
                    format!("key unlock_decision_tooltip: {why}"),
                ),
            }
        }
        self.char_native_line(
            if definition.power_balance {
                b"EFFECT_UNLOCK_DECISION_BOP"
            } else {
                b"EFFECT_UNLOCK_DECISION"
            },
            &[
                ("NAME", Argument::Text(&name)),
                ("EFFECT", Argument::Text(&effects)),
            ],
            scopes,
            true,
        )
    }

    fn decision_modifier_details(
        &self,
        definition: &Decision,
        scopes: Scopes,
    ) -> Result<String, String> {
        let options = ModifierListOptions {
            category_mask: u32::MAX,
            ..Default::default()
        };
        let mut text = self.modifier_description_with(
            &content_reader_script::modifiers::values(&definition.modifier),
            "",
            options,
        )?;
        for fields in &definition.targeted_modifier {
            let Some(target) = fields
                .iter()
                .rev()
                .find(|field| field.keyword_is("tag") || field.keyword_is("key"))
            else {
                continue;
            };
            let Some(country) = self
                .country_target(target, scopes)?
                .filter(|country| *country > 0)
            else {
                continue;
            };
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            let selected = self.fresh_country_scope(country, None)?;
            let name = self.expanded("[THIS.GetNameWithFlag]", selected)?;
            text.push_str("\u{11}H");
            text.push_str(&name);
            text.push_str("\u{11}!\n");
            let modifiers = content_reader_script::modifiers::targeted(fields).modifiers;
            text.push_str(&self.modifier_description_with(&modifiers, "  ", options)?);
        }
        Ok(text)
    }


    pub(in crate::effect_description) fn activate_targeted_decision(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "decision")
            .and_then(|field| field.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or_else(|| self.content_gap("decisions"))?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let (country, state) = self.decision_target(field(fields, "target"), scopes)?;
        if country.is_none() && state == 0 {
            return Ok(String::new());
        }
        let owner = self.current_country(scopes)?;
        let selected = self.decision_seeded_scope(definition, owner, country, state, true)?;
        let name = self.decision_title(definition, selected)?;
        self.char_native_line(
            b"EFFECT_ACTIVATE_TARGETED_DECISION",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(super) fn decision_seeded_scope(
        &self,
        definition: &Decision,
        owner: u32,
        country: Option<u32>,
        state: u32,
        highlight: bool,
    ) -> Result<Scopes, String> {
        let world = self.context.world()?;
        let target = country.unwrap_or(state) as i32;
        let seed = if definition.fixed_random_seed {
            self.decision_description_seed(definition, None, owner, target)
        } else {
            let row = match world.tables().complete::<tables::Decisions>(DECISION) {
                Err(_) => None,
                Ok(rows) => {
                    let mut first: Option<Cow<'_, DecisionRow>> = None;
                    for row in
                        rows.keyed(|key| world.recorded().slot((key >> 40) as u32) == Some(owner))
                    {
                        let row = row.map_err(built)?;
                        if row.token.eq_ignore_ascii_case(&definition.id)
                            && row.numeric_target == state
                            && (if row.country_target == 0 {
                                None
                            } else {
                                world.recorded().slot(row.country_target)
                            }) == country
                            && first.as_ref().is_none_or(|first| row.ordinal < first.ordinal)
                        {
                            first = Some(row);
                        }
                    }
                    first
                }
            };
            self.decision_description_seed(definition, row.as_deref(), owner, target)
        };
        let target = country
            .map(EventObject::Country)
            .or_else(|| (state != 0).then_some(EventObject::State(state)));
        let (at, from) = self.seeded_decision_graph(owner, target, seed);
        let selected = self
            .evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())?;
        if highlight
            && (definition.has_state_targets
                || self
                    .evaluator()?
                    .has_children(&definition.highlight_conditions)
                    .map_err(|why| why.to_string())?)
            && let Some(highlighted) = self.highlight_state(definition, selected)?
        {
            let mut graph = self.graph.borrow_mut();
            let index = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::State(highlighted)));
            graph.scopes[from].from = Link::Scope(index);
        }
        self.evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())
    }


    pub(in crate::effect_description) fn decision_cooldown(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let database = self
            .context
            .content
            .decisions
            .ok_or_else(|| self.content_gap("decisions"))?;
        let Some(definition) = database.decision(item.value.scalar().unwrap_or_default())? else {
            return Ok(String::new());
        };
        let owner = self.current_country(scopes)?;
        let selected = self.decision_seeded_scope(definition, owner, None, 0, true)?;
        let name = self.decision_title(definition, selected)?;
        self.char_native_line(
            b"EFFECT_REMOVE_DECISION",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }


    pub(in crate::effect_description) fn state_event(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        if let Some(desc) = field(children(item), "desc")
            .and_then(|field| field.value.read_string())
            .filter(|desc| !desc.is_empty())
        {
            return self.line(desc, &[], scopes, true);
        }
        let id = item
            .value
            .scalar()
            .or_else(|| field(children(item), "id").and_then(|field| field.value.scalar()))
            .ok_or("state_event has no id")?;
        let event = (self
            .context
            .content
            .events
            .ok_or("referenced event lookup unavailable")?)(id)?;
        if let Some(why) = &event.refused {
            return Err(why.clone());
        }
        let cursor = scopes
            .cursor
            .ok_or("state_event lacks caller scope graph")?;
        let object = self.graph.borrow().scopes[cursor].this;
        let state = match object {
            EventObject::State(state) | EventObject::CountryState { state, .. } => state,
            _ => 0,
        };
        let object = if state == 0 {
            match object {
                EventObject::UnitLeader(leader) | EventObject::CountryUnitLeader { leader, .. } => {
                    let world = self.context.world()?;
                    let original = world.unit_leader(leader).map_err(|why| why.to_string())?
                        .ok_or("state_event scope leader is not recorded")?;
                    match crate::character_scope::Tables::read_in(world.tables())
                        .unit_leader(original.character_id).map_err(|why| why.to_string())? {
                        Some(crate::character_scope::UnitLeader::Direct(row)) => {
                            let country = if row.owner == 0 { 0 } else {
                                world.slot(row.owner).ok_or("state_event current leader country lacks country links")?
                            };
                            let leader = u64::from(row.id) << 32 | u64::from(row.reference_type);
                            EventObject::CountryUnitLeader { country, leader }
                        }
                        Some(crate::character_scope::UnitLeader::Referenced(_)) => {
                            return Err("state_event current operative role's leader character reference is not recorded by character operatives".into());
                        }
                        None => match object {
                            EventObject::CountryUnitLeader { country, .. } => EventObject::Country(country),
                            _ => return Err("state_event bare leader with no current role leaves a native null country; that title scope is not projected".into()),
                        },
                    }
                }
                EventObject::Character(_) => return Err("state_event current character role and its resulting title scope are not projected".into()),
                EventObject::Country(country) | EventObject::CountryState { country, .. } => {
                    EventObject::Country(country)
                }
                _ => EventObject::Country(0),
            }
        } else if let Some(target) = field(children(item), "trigger_for") {
            let operand = target.value.read_string().unwrap_or_default();
            let lower = operand.to_ascii_lowercase();
            let link = matches!(
                lower.as_slice(),
                b"owner" | b"controller" | b"occupied" | b"from" | b"root" | b"prev"
            );
            let countries = self
                .context
                .world()?
                .tables()
                .complete::<tables::Countries>(0)
                .map_err(|why| why.to_string())?;
            let mut tag = false;
            for text in countries.texts(|row| &row.tag, COUNTRY_TAG) {
                if text.map_err(built)?.eq_ignore_ascii_case(operand) {
                    tag = true;
                    break;
                }
            }
            let country = if link || tag {
                self.country_target(target, scopes)?.unwrap_or(0)
            } else {
                0
            };
            if country > 0 {
                EventObject::CountryState { country, state }
            } else {
                EventObject::State(state)
            }
        } else {
            EventObject::State(state)
        };
        let at = {
            let mut graph = self.graph.borrow_mut();
            let parent = &mut graph.scopes[cursor];
            let seed = random::scoped_draw(parent.random_seed, parent.random_counter) as u32;
            parent.random_seed = parent.random_seed.wrapping_add(1);
            let mut title = EventScope::of(object);
            title.random_seed = random::scope_seed(seed);
            title.random_counter = random::scope_counter(seed);
            title.random_seed_refusal = parent.random_seed_refusal;
            title.from = match parent.root {
                Link::Itself => Link::Scope(cursor),
                link => link,
            };
            let at = graph.scopes.len();
            graph.scopes.push(title);
            at
        };
        let selected = self
            .evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())?;
        let title = self.event_title(
            event.deterministic_title_key(|triggers| {
                self.evaluator()?.evaluate(triggers, selected).decided()
            })?,
            selected,
        )?;
        self.char_native_text(
            b"EFFECT_COUNTRY_EVENT",
            &[("EVENT", Argument::Text(&title))],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn show_mio(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = item.value.scalar().unwrap_or_default();
        let database = self
            .context
            .content
            .industrial_organisations
            .ok_or_else(|| self.content_gap("industrial organisation visibility"))?;
        let token = if database
            .templates
            .contains_key(&operand.to_ascii_lowercase())
        {
            database.template(operand)?.token.clone()
        } else if let Some(Scope::IndustrialOrganisation(reference)) = self
            .evaluator()?
            .scoped_scope(operand, scopes)
            .map_err(|why| why.to_string())?
        {
            let row = self
                .context
                .world()?
                .industrial_organisation(reference)
                .map_err(|why| why.to_string())?;
            self.context
                .world()?
                .token(row.token)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks MIO template token")?
                .to_vec()
        } else {
            operand.to_vec()
        };
        let country = self.current_country(scopes)?;
        let world = self.context.world()?;
        let mut instance = None;
        if let Some(id) = world.token_of(&token).map_err(|why| why.to_string())? {
            let organisations = world
                .tables()
                .complete::<tables::IndustrialOrganisations>(ORGANISATION)
                .map_err(|why| why.to_string())?;
            for row in each(organisations) {
                let row = row?;
                if world.slot(row.country) == Some(country)
                    && row.token == id
                    && instance
                        .as_ref()
                        .is_none_or(|first: &Cow<'_, IndustrialOrganisationRow>| {
                            row.ordinal < first.ordinal
                        })
                {
                    instance = Some(row);
                }
            }
        }
        let (name, initial, trait_country) = if let Some(row) = instance {
            let (_, initial) = database.display_fields(&token)?;
            (self.organisation_name(row.organisation)?, initial, country)
        } else if database.templates.contains_key(&token.to_ascii_lowercase()) {
            let (name, initial) = database.display_fields(&token)?;
            let name = if name.is_empty() {
                token.as_slice()
            } else {
                name.as_slice()
            };
            let key = self.mio_content_name_key(name, country)?;
            (self.localised(&key, &[], scopes, false)?, initial, 0)
        } else {
            (
                String::from_utf8(operand.to_vec()).map_err(|_| "MIO operand is not UTF-8")?,
                None,
                0,
            )
        };
        let mut out = self.char_native_text(
            b"EFFECT_SHOW_MIO_TOOLTIP_MIO_NAME",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )?;
        if let Some(initial) = initial {
            let base = if initial.is_empty() {
                [token.as_slice(), b"_initial_trait"].concat()
            } else {
                initial
            };
            let key = self.mio_content_name_key(&base, trait_country)?;
            let name = self.localised(&key, &[], scopes, false)?;
            if !name.is_empty() {
                out.push(' ');
                out.push_str(&self.char_native_text(
                    b"EFFECT_SHOW_MIO_TOOLTIP_INITIAL_TRAIT_NAME",
                    &[("NAME", Argument::Text(&name))],
                    scopes,
                    true,
                )?);
            }
        }
        Ok(out)
    }


    pub(in crate::effect_description) fn unlock_mio(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = item.value.scalar().unwrap_or_default();
        let name = if let Some(Scope::IndustrialOrganisation(reference)) = self
            .evaluator()?
            .scoped_scope(operand, scopes)
            .map_err(|why| why.to_string())?
        {
            self.organisation_name(reference)?
        } else {
            String::from_utf8(content_reader_parser::parser::c_string(operand).to_vec())
                .map_err(|_| "MIO operand is not UTF-8")?
        };
        self.char_native_line(
            b"EFFECT_MIO_UNLOCK_MIO",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    fn mio_content_name_key(&self, base: &[u8], country: u32) -> Result<Vec<u8>, String> {
        if country > 0 {
            let tagged = [self.tag(country)?.as_bytes(), b"_", base].concat();
            if self
                .context
                .localisation
                .get(std::str::from_utf8(&tagged).map_err(|_| "MIO name key is not UTF-8")?)
                .is_some()
            {
                return Ok(tagged);
            }
        }
        Ok(base.to_vec())
    }


    pub(in crate::effect_description) fn unlock_mio_policy(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| field(children(item), "policy").and_then(|field| field.value.read_string()))
            .unwrap_or_default();
        let world = self.context.world()?;
        let country = if let Scope::IndustrialOrganisation(reference) = scopes.this {
            world
                .slot(
                    world
                        .industrial_organisation(reference)
                        .map_err(|why| why.to_string())?
                        .owner,
                )
                .ok_or("country links lacks policy MIO owner")?
        } else {
            self.current_country(scopes)?
        };
        let definition = if country > 0 {
            let row = world
                .tables()
                .complete::<tables::CountryCounterInputs>(MIO_POLICIES)
                .map_err(|why| why.to_string())?
                .keyed(|key| world.slot(key as u32) == Some(country))
                .next()
                .transpose()
                .map_err(built)?
                .ok_or("country counter inputs lacks MIO policy roster")?;
            let policies = row
                .mio_policies
                .require("mio policies of country counter inputs")?
                .as_ref()
                .ok_or("mio policies of country counter inputs capture refused")?;
            if let Some(token) = world.token_of(operand).map_err(|why| why.to_string())?
                && policies.contains(&token)
            {
                let spelling = world
                    .token(token)
                    .map_err(|why| why.to_string())?
                    .ok_or("lexical tokens lacks MIO policy spelling")?;
                Some(
                    self.context
                        .content
                        .industrial_organisations
                        .ok_or_else(|| self.content_gap("industrial organisations"))?
                        .policies
                        .policy(spelling)?
                        .clone(),
                )
            } else {
                None
            }
        } else {
            None
        };
        let name = if let Some(policy) = &definition {
            let base = if policy.name.is_empty() {
                &policy.token
            } else {
                &policy.name
            };
            let key = self.mio_content_name_key(base, country)?;
            self.localised(&key, &[], self.fresh_country_scope(country, None)?, true)?
        } else {
            self.localised(operand, &[], scopes, false)?
        };
        let mut text = self.char_native_line(
            b"EFFECT_MIO_UNLOCK_POLICY",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )?;
        if content_reader_parser::parser::read_bool_field(children(item), "show_modifiers", true)
            && let Some(policy) = definition
        {
            let body = self.char_mio_policy_modifiers(&policy, scopes);
            text.push_str(" (");
            text.push_str(&body);
            text.push(')');
        }
        Ok(text)
    }

    fn decision_removal_delay(&self, definition: &Decision, scopes: Scopes) -> Result<i32, String> {
        if definition.days_remove.is_empty() {
            return Ok(0);
        }
        let days = (self.scoped_value(
            &definition.days_remove,
            self.fresh_country_scope(self.current_country(scopes)?, None)?,
        )? / 100_000) as i32;
        if days <= 0 {
            return Ok(days);
        }
        let features = self.complete::<tables::SimulationFeatures>(INSTANT)?;
        let (1, Some(features)) = (features.len(), each(features).next().transpose()?) else {
            return Err("simulation features lacks instant_decisions".into());
        };
        let instant = features
            .instant_decisions
            .require("instant decisions of simulation features")?
            .ok_or("instant decisions of simulation features capture refused")?;
        Ok(if instant != 0 { 1 } else { days })
    }

    pub(in crate::effect_description) fn event_option(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = content_reader_parser::parser::c_string(item.value.scalar().unwrap_or_default());
        for (event, option) in self.context.text.content().event_index().options(name)? {
            if let Some(why) = &event.refused {
                return Err(why.clone());
            }
            let body = &event.options[option];
            if self.option_has_dependent_tooltip(body)? {
                continue;
            }
            let cursor = scopes
                .cursor
                .ok_or("event_option_tooltip lacks caller scope graph")?;
            let at = {
                let mut graph = self.graph.borrow_mut();
                let caller = *graph
                    .scopes
                    .get(cursor)
                    .ok_or("event_option_tooltip caller scope is missing")?;
                let link = |link| match link {
                    Link::Itself => Link::Scope(cursor),
                    link => link,
                };
                let mut copied = caller;
                copied.root = link(caller.from);
                copied.from = link(caller.root);
                copied.prev = link(caller.prev);
                let at = graph.scopes.len();
                graph.scopes.push(copied);
                if let Some(targets) = graph.saved_targets.get(&cursor).cloned() {
                    graph.saved_targets.insert(at, targets);
                }
                if let Some(targets) = graph.tooltip_targets.get(&cursor).cloned() {
                    graph.tooltip_targets.insert(at, targets);
                }
                at
            };
            let selected = self
                .evaluator()?
                .event_scopes(at)
                .map_err(|why| why.to_string())?;
            return Ok(trim_description(&self.group(body, selected, 0)).to_owned());
        }
        Ok(String::new())
    }

    fn option_has_dependent_tooltip(&mut self, body: &[Item]) -> Result<bool, String> {
        let mut pending = vec![body.to_vec()];
        while let Some(body) = pending.pop() {
            for (item, branches) in self.attached(&body, false) {
                let Some(class) = self.effect_class(item) else {
                    continue;
                };
                if item
                    .keyword()
                    .and_then(|key| content_reader_script::effect_keys::registered(&key.folded()))
                    == Some(DEPENDENT_OPTION_TOOLTIP_TOKEN)
                {
                    return Ok(true);
                }
                pending.push(self.effect_children(item));
                if class.vtable == effect_identity::CONDITIONAL {
                    for branch in branches {
                        pending.push(self.effect_children(branch));
                    }
                } else if class.vtable == effect_identity::WEIGHTED_CHOICES {
                    for member in content_reader_script::effect_keys::random_list_members(item)? {
                        pending.push(content_reader_script::effect_keys::random_member_fields(&member));
                    }
                }
            }
        }
        Ok(false)
    }
}
