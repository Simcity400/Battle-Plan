use super::*;

impl Renderer<'_, '_, '_> {
    // ReadScript 1403A9C20 takes the held scalar; 1403A9EC0 additionally reads
    // mission, and 1403A92B0 additionally reads desc. No other own key is used.
    // Activate/remove decision keys@14275DFB0/14275E0A0;
    // activate/remove mission keys@14275DD78/14275E288.
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
        // 14036F940 appends the authored description to NAME, including LF.
        if item.keyword_is("activate_mission_tooltip")
            && let Some(desc) = field(children(item), "desc")
                .and_then(|field| field.value.read_string())
                .filter(|desc| !desc.is_empty())
        {
            name.push_str(&self.line(desc, &[], scopes, true)?);
        }
        self.char_native_line(
            match key(item).to_ascii_lowercase().as_str() {
                "activate_decision" => b"EFFECT_ACTIVATE_DECISION", // 14036F380.
                "remove_decision" => b"EFFECT_REMOVE_DECISION",     // 140388CA0.
                "remove_mission" => b"EFFECT_REMOVE_MISSION",       // 140389C20.
                _ => b"EFFECT_ACTIVATES_MISSION",                   // 14036F660 / 14036F940.
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // 140372A90 reads mission at +0x58 and evaluates days at +0x78; its reader
    // 1403AECE0 consumes exactly mission/days. Zero rounds to native-empty.
    // ADD_DAYS_TO_MISSION/REMOVE_DAYS_TO_MISSION@14275E7D8/14275E7C0.
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
        let days = crate::triggers::fixed_round(amount) as i32; // 1424D8760, shared native arithmetic.
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

    // 140399060 -> 140720D10: category names use their authored identifier
    // (140716B20 +0x18), without 140721730's country/ideology ladder.
    // EFFECT_UNLOCK_DECISION_CATEGORY@14275DE88.
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

    // 140399360; reader 1403ACCD0 owns only decision, show_effect_tooltip,
    // show_modifiers. Independent detail failures retain the native NAME line.
    // Unlock/BOP keys@14275DC20/14275DC00; select/remove headers@14275DB68/14275DBB8.
    // Allocator 140342280 (store 1403422CA) clears +0x78/+0x79: both flags default no.
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
            // 140399360 still emits the wrapper when +0x118 is the null sentinel.
            return self.char_native_line(
                b"EFFECT_UNLOCK_DECISION",
                &[("NAME", Argument::Text("")), ("EFFECT", Argument::Text(""))],
                scopes,
                true,
            );
        };
        let name = self.decision_title(definition, scopes)?;
        let mut effects = String::new();
        if installed_content::parser::read_bool_field(fields, "show_effect_tooltip", false) {
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
                                        // 140399360 appends this after completion text.
                                        effects.push_str(&self.char_native_line(
                                            b"EFFECT_UNLOCK_DECISION_TIMED_HEADER",
                                            &[("DAYS", Argument::Int32(days))],
                                            scopes,
                                            true,
                                        )?);
                                    }
                                    Ok(_) => {}
                                    Err(why) => self.refuse(
                                        "CUnlockDecisionTooltipEffect",
                                        format!("key unlock_decision_tooltip: {why}"),
                                    ),
                                }
                            }
                        }
                    }
                    Err(why) => self.refuse(
                        "CUnlockDecisionTooltipEffect",
                        format!("key unlock_decision_tooltip: {why}"),
                    ),
                }
            }
            if !effects.is_empty() {
                effects.insert(0, '\n');
            }
        }
        if installed_content::parser::read_bool_field(fields, "show_modifiers", false)
            && (!definition.modifier.is_empty() || !definition.targeted_modifier.is_empty())
        {
            match self.decision_modifier_details(definition, scopes) {
                Ok(modifiers) if !modifiers.is_empty() => {
                    // 140399C27/140399C49 adds LF when empty or already LF-terminated.
                    if effects.is_empty() || effects.ends_with('\n') {
                        effects.push('\n');
                    }
                    effects.push_str(&self.localised(
                        b"DECISION_WHILE_ACTIVE",
                        &[],
                        scopes,
                        true,
                    )?); // 14275DBE0.
                    effects.push('\n');
                    effects.push_str(&modifiers);
                }
                Ok(_) => {}
                Err(why) => self.refuse(
                    "CUnlockDecisionTooltipEffect",
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

    // 140399821/140399BC9 call raw FormatDescription@14054E050 with all
    // categories and scale 100000. Only the final NAME/EFFECT wrapper expands.
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
            &installed_content::modifiers::values(&definition.modifier),
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
            // 14039988A/1403998A2 resolves the target in the caller's scope.
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
            // The shared GetNameWithFlag callback reproduces 140BA59E0:
            // flag control, tag, space, capitalized definite country name.
            let name = self.expanded("[THIS.GetNameWithFlag]", selected)?;
            text.push_str("\u{11}H"); // 140399962.
            text.push_str(&name);
            text.push_str("\u{11}!\n");
            let modifiers = installed_content::modifiers::targeted(fields).modifiers;
            text.push_str(&self.modifier_description_with(&modifiers, "  ", options)?); // Prefix at 1403997CE.
        }
        Ok(text)
    }

    // 140370080 uses 14036C6E0's target, then 1407290D0's seed and highlight
    // scope. It does not require an active targeted decision (unlike removal).
    // EFFECT_ACTIVATE_TARGETED_DECISION@14275E490.
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
        // STargetedDecisionTarget::seed_target 140721230 uses canonical_slot
        // 140BA6240 for a country, otherwise its stored numeric state bits.
        let target = country.unwrap_or(state) as i32;
        let seed = if definition.fixed_random_seed {
            self.decision_description_seed(definition, None, owner, target)
        } else {
            let row = world
                .frame()
                .complete::<tables::Decisions>()
                .ok()
                .and_then(|rows| {
                    rows.iter()
                        .filter(|row| {
                            world.recorded().slot(row.country) == Some(owner)
                                && row.token.eq_ignore_ascii_case(&definition.id)
                                && row.numeric_target == state
                                && (if row.country_target == 0 {
                                    None
                                } else {
                                    world.recorded().slot(row.country_target)
                                }) == country
                        })
                        .min_by_key(|row| row.ordinal)
                });
            self.decision_description_seed(definition, row, owner, target)
        };
        let target = country
            .map(EventObject::Country)
            .or_else(|| (state != 0).then_some(EventObject::State(state)));
        let (at, from) = self.seeded_decision_graph(owner, target, seed);
        let selected = self.evaluator()?.event_scopes(at)?;
        if highlight
            && (definition.has_state_targets
                || self
                    .evaluator()?
                    .has_children(&definition.highlight_conditions)?)
            && let Some(highlighted) = self.highlight_state(definition, selected)?
        {
            let mut graph = self.graph.borrow_mut();
            let index = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::State(highlighted)));
            graph.scopes[from].from = Link::Scope(index);
        }
        self.evaluator()?.event_scopes(at)
    }

    // 140388F80 resolves its authored decision pointer (+0x148), then the
    // fresh country/title path 1407209E0, rather than the caller title scope.
    // EFFECT_REMOVE_DECISION@14275E0A0.
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

    // 140318EA0 follows 14037BDC0's draw, but 14137D8C0 sets a state object.
    // Reader 1413859E0 accepts id/desc/hours/days/random/random_hours/
    // random_days/major/trigger_for; id, desc and trigger_for affect own text.
    // EFFECT_COUNTRY_EVENT@142729928, EVENT@142729920.
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
            .ok_or("state_event has no id (140318EA0)")?;
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
                // 14137D8C0 resolves the +50 character's unit leader first,
                // then SetUnitLeader(1), SetCountry(leader+120, 0). Both
                // operands survive, even when the caller carries a country.
                EventObject::UnitLeader(leader) | EventObject::CountryUnitLeader { leader, .. } => {
                    let world = self.context.world()?;
                    let original = world.unit_leader(leader)?
                        .ok_or("state_event scope leader is not recorded (1405294F0)")?;
                    match crate::character_scope::Tables::read(world.frame())
                        .unit_leader(original.character_id)? {
                        Some(crate::character_scope::UnitLeader::Direct(row)) => {
                            let country = if row.owner == 0 { 0 } else {
                                world.slot(row.owner).ok_or("state_event current leader country lacks CountryLinks")?
                            };
                            let leader = u64::from(row.id) << 32 | u64::from(row.reference_type);
                            EventObject::CountryUnitLeader { country, leader }
                        }
                        Some(crate::character_scope::UnitLeader::Referenced(_)) => {
                            return Err("state_event current operative role's CUnitLeader+F48 character reference is not recorded by CharacterOperatives (14137D8C0/1405294F0)".into());
                        }
                        None => match object {
                            EventObject::CountryUnitLeader { country, .. } => EventObject::Country(country),
                            _ => return Err("state_event bare leader with no current role leaves a native null country; that title scope is not projected (14137D8C0)".into()),
                        },
                    }
                }
                EventObject::Character(_) => return Err("state_event current character role and its resulting title scope are not projected (14137D8C0/1405294F0)".into()),
                EventObject::Country(country) | EventObject::CountryState { country, .. } => {
                    EventObject::Country(country)
                }
                _ => EventObject::Country(0),
            }
        } else if let Some(target) = field(children(item), "trigger_for") {
            // 14137D8C0 accepts these links, else FindCountryIndexByTag;
            // it does not evaluate scoped variables or THIS.
            let operand = target.value.read_string().unwrap_or_default();
            let lower = operand.to_ascii_lowercase();
            let link = matches!(
                lower.as_slice(),
                b"owner" | b"controller" | b"occupied" | b"from" | b"root" | b"prev"
            );
            let tag = self
                .context
                .world()?
                .frame()
                .complete::<tables::Countries>()
                .map_err(|why| why.to_string())?
                .iter()
                .any(|row| row.tag.eq_ignore_ascii_case(operand));
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
            title.random_seed_refusal = parent.random_seed_refusal; // 14137D8C0 derives lanes.
            title.from = match parent.root {
                Link::Itself => Link::Scope(cursor),
                link => link,
            };
            let at = graph.scopes.len();
            graph.scopes.push(title);
            at
        };
        let selected = self.evaluator()?.event_scopes(at)?;
        let title = match event.deterministic_title_key(|triggers| {
            self.evaluator()?.evaluate(triggers, selected).decided()
        })? {
            Some(key) => self.localised(key, &[], selected, true)?,
            None => String::new(),
        };
        // LocalizeWithMovedStringArgument 1402E30F0 does not expand the wrapper.
        self.char_native_text(
            b"EFFECT_COUNTRY_EVENT",
            &[("EVENT", Argument::Text(&title))],
            scopes,
            false,
        )
    }

    // 1402FEFE0: ReadValue 141385570/1405345D0 stores one operand. A
    // template pointer wins, then the caller country's first matching instance.
    // MIO-name/initial-trait keys@142731CD8/142731D00.
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
        } else if let Some(Scope::IndustrialOrganisation(reference)) =
            self.evaluator()?.scoped_scope(operand, scopes)?
        {
            let row = self.context.world()?.industrial_organisation(reference)?;
            self.context
                .world()?
                .token(row.token)?
                .ok_or("LexicalTokens lacks MIO template token")?
                .to_vec()
        } else {
            operand.to_vec()
        };
        let country = self.current_country(scopes)?;
        let world = self.context.world()?;
        let mut instance = None;
        if let Some(id) = world.token_of(&token)? {
            instance = world
                .frame()
                .complete::<tables::IndustrialOrganisations>()
                .map_err(|why| why.to_string())?
                .iter()
                .filter(|row| world.slot(row.country) == Some(country) && row.token == id)
                .min_by_key(|row| row.ordinal);
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
            let key = self.mio_content_name_key(name, country)?; // 140A44730 -> 141399DA0.
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
                [token.as_slice(), b"_initial_trait"].concat() // 140A405B0 -> 141399FE0, token 0x3FE6.
            } else {
                initial
            };
            let key = self.mio_content_name_key(&base, trait_country)?;
            let name = self.localised(&key, &[], scopes, false)?;
            if !name.is_empty() {
                out.push(' '); // 1402FEFE0 adds one space, with no LF.
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

    // ReadValue 141385570 -> 1402F90F0 stores one scoped operand and its
    // spelling at +0x108; ReadEffect 1405345D0 owns no MIO/name block fields.
    // 1402FF9D0 uses 140537750's scoped object, else copies +0x108 literally.
    // A resolved object uses the shared 140DA44C0 name/owner environment.
    // EFFECT_MIO_UNLOCK_MIO@142731698.
    pub(in crate::effect_description) fn unlock_mio(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = item.value.scalar().unwrap_or_default();
        let name = if let Some(Scope::IndustrialOrganisation(reference)) =
            self.evaluator()?.scoped_scope(operand, scopes)?
        {
            self.organisation_name(reference)?
        } else {
            String::from_utf8(installed_content::parser::c_string(operand).to_vec())
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
        Ok(base.to_vec()) // 141399DA0 -> 14146B1D0.
    }

    // 1402FFD40 -> 1413970E0; reader thunk 140301210 -> 141398350 owns
    // policy/show_modifiers. Country 0 takes the raw-key localisation fallback.
    // EFFECT_MIO_UNLOCK_POLICY@142731780.
    // Allocator 1402F9DB0 sets show_modifiers (+0xA8) yes; the zero-country
    // fallback never emits the modifier block (1413970E0).
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
                .slot(world.industrial_organisation(reference)?.owner)
                .ok_or("CountryLinks lacks policy MIO owner (141396FF0)")?
        } else {
            self.current_country(scopes)?
        };
        let definition = if country > 0 {
            let row = world
                .frame()
                .complete::<tables::CountryCounterInputs>()
                .map_err(|why| why.to_string())?
                .iter()
                .find(|row| world.slot(row.country) == Some(country))
                .ok_or("CountryCounterInputs lacks MIO policy roster (141396FF0)")?;
            let policies = row
                .mio_policies
                .require("CountryCounterInputs.mio_policies")?
                .as_ref()
                .ok_or("CountryCounterInputs.mio_policies capture refused (140E487D0)")?;
            if let Some(token) = world.token_of(operand)?
                && policies.contains(&token)
            {
                let spelling = world
                    .token(token)?
                    .ok_or("LexicalTokens lacks MIO policy spelling")?;
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
        // 1413970E0 uses the authored raw-key fallback when the stored cache
        // has no matching policy; it emits no modifier block on this path.
        let name = if let Some(policy) = &definition {
            let base = if policy.name.is_empty() {
                &policy.token
            } else {
                &policy.name
            };
            let key = self.mio_content_name_key(base, country)?; // 140A3C9B0 -> 141399DA0.
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
        if installed_content::parser::read_bool_field(children(item), "show_modifiers", true)
            && let Some(policy) = definition
        {
            let body = self.char_mio_policy_modifiers(&policy, scopes);
            text.push_str(" ("); // 14270C624, appended after the wrapper's LF.
            text.push_str(&body);
            text.push(')'); // 14270BE94; even when no modifiers are visible.
        }
        Ok(text)
    }

    // 14071E7D0 evaluates in a fresh owner scope and narrows signed /100000;
    // instant_decisions 143317263 changes only a positive delay to one.
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
        let [features] = self
            .context
            .text
            .frame()
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
        else {
            return Err("SimulationFeatures lacks instant_decisions (14071E7D0)".into());
        };
        let instant = features
            .instant_decisions
            .require("SimulationFeatures.instant_decisions")?
            .ok_or("SimulationFeatures.instant_decisions capture refused (14071E7D0)")?;
        Ok(if instant != 0 { 1 } else { days })
    }

    // ReadValue@141385570/1405345D0 stores only the option name at +0x58.
    // 1413832F0 searches manager +0/+0x0C, then option +0x178/+0x188, by
    // exact bytes; state events live in the other manager array (140A01920).
    pub(in crate::effect_description) fn event_option(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = installed_content::parser::c_string(item.value.scalar().unwrap_or_default());
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
                // Copy@1405285F0/1405296C0 preserves object/seeds/PREV and both
                // target arrays. 14138332E/1413833B1 then swap ROOT and FROM.
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
            let selected = self.evaluator()?.event_scopes(at)?;
            // 141527890 invokes the option's BuildDescription at indent 0 in
            // the existing description session, then Trim@1424B7400.
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
                // VisitRecursively@140535120 visits self and native children;
                // lambda@141387A00 tests the diagnostic token, not the operand.
                if item
                    .keyword()
                    .and_then(|key| installed_content::effect_keys::registered(&key.folded()))
                    == Some(0x3FF4)
                {
                    return Ok(true);
                }
                pending.push(self.effect_children(item));
                if class.vtable == 0x1427BE590 {
                    // CIfEffect@141386F50 also visits all attached alternatives.
                    for branch in branches {
                        pending.push(self.effect_children(branch));
                    }
                } else if class.vtable == 0x1429A0260 {
                    // CRandomListEffect@141386FF0 visits each embedded member
                    // group, without evaluating weights or modifier triggers.
                    for member in installed_content::effect_keys::random_list_members(item)? {
                        pending.push(installed_content::effect_keys::random_member_fields(&member));
                    }
                }
                // CScriptedEffect inherits 140535120: the template at +0x60
                // is expanded only by BuildDescription@141382080, not this scan.
            }
        }
        Ok(false)
    }
}
