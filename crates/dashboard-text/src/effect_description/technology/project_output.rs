use super::*;
use replay::rows::tables;
use std::collections::BTreeSet;

#[cfg(test)]
mod tests;

fn trailing(text: &mut String, count: usize) {
    if text.is_empty() {
        return;
    }
    while text.bytes().rev().take_while(|byte| *byte == b'\n').count() < count {
        text.push('\n');
    }
}

impl Renderer<'_, '_, '_> {
    // Template output only;
    // iteration_output belongs to execution. Native outer indent is 2 spaces.
    pub(in crate::effect_description::technology) fn complete_project_output(
        &mut self,
        item: &Item,
        scopes: Scopes,
        owner: u32,
        id: u32,
    ) -> Result<String, String> {
        let world = self.context.world()?;
        let row = world.project(owner, id)?;
        let token = world
            .token(row.definition_id)?
            .ok_or("LexicalTokens lacks project definition")?;
        let token = std::str::from_utf8(token).map_err(|_| "project definition is not UTF-8")?;
        let catalogue = self
            .character_content()?
            .special_projects
            .as_ref()
            .map_err(Clone::clone)?;
        if let Some(why) = &catalogue.unsettled {
            return Err(why.clone());
        }
        let definition = catalogue
            .projects
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(token))
            .map(|(_, definition)| definition)
            .ok_or("special-project content lacks selected definition")?;
        if let Some(why) = &definition.refusal {
            return Err(why.clone());
        }
        let output = definition.output.clone();
        let specialization = definition.specialization.clone();
        let country = world
            .slot(owner)
            .ok_or("CountryLinks lacks project owner")?;
        let project = self.evaluator()?.fresh_country_scope(Some(0), None)?;
        let project = self
            .evaluator()?
            .repoint_this(project, Scope::Project { owner, id })?;
        let mut text =
            self.char_native_text(b"SPECIAL_PROJECT_RESEARCH_OUTPUT", &[], scopes, false)?;
        trailing(&mut text, 1);

        // country, scientist, facility state, with two trailing
        // LFs per nonempty body. ROOT/PREV are THIS; FROM is the project.
        if !output.country_effects.is_empty() {
            let selected = self
                .evaluator()?
                .fresh_country_scope(Some(country), Some(project))?;
            self.project_effect_output(&mut text, &output.country_effects, selected)?;
        }
        if !output.scientist_effects.is_empty() {
            match self.project_output_scientist(item, scopes, specialization.as_deref()) {
                Ok(Some(character)) => {
                    let selected = self
                        .evaluator()?
                        .fresh_country_scope(Some(0), Some(project))?;
                    let selected = self
                        .evaluator()?
                        .repoint_this(selected, Scope::Character(character))?;
                    self.project_effect_output(&mut text, &output.scientist_effects, selected)?;
                }
                Ok(None) => {} // omits the body for a null scientist.
                Err(why) => self.project_output_refusal(why),
            }
        }
        if !output.facility_state_effects.is_empty() {
            match self.project_output_program(item, scopes, specialization.as_deref()) {
                Ok(Some((province, state))) => {
                    let selected = self
                        .evaluator()?
                        .fresh_country_scope(Some(0), Some(project))?;
                    let selected = self
                        .evaluator()?
                        .repoint_this(selected, Scope::State(state))?;
                    // stores the already-held province id in TLS,
                    // then describes the body.
                    self.evaluator()?.store_description_temp(
                        b"facility_province_id".to_vec(),
                        i64::from(province) * 100_000,
                    );
                    self.project_effect_output(
                        &mut text,
                        &output.facility_state_effects,
                        selected,
                    )?;
                }
                Ok(None) => {} // omits a null province.
                Err(why) => self.project_output_refusal(why),
            }
        }
        trailing(&mut text, 1);
        if !output.sub_unit_bonus.is_empty() {
            match self.units_bonus_description(&output.sub_unit_bonus, scopes) {
                Ok(bonus) => text.push_str(&bonus),
                Err(why) => self.project_output_refusal(why),
            }
        }
        trailing(&mut text, 1);
        if !output.equipment_bonus.is_empty() {
            let mut bonus = item.clone();
            bonus.key = Some(b"bonus".to_vec());
            bonus.value = installed_content::parser::Value::Block(output.equipment_bonus.clone());
            let mut adapter = item.clone();
            adapter.value = installed_content::parser::Value::Block(vec![bonus]);
            match self.units_equipment_bonus(&adapter, scopes) {
                Ok(bonus) => text.push_str(&bonus),
                Err(why) => self.project_output_refusal(why),
            }
        }
        trailing(&mut text, 1);
        for (kind, lists) in [
            (0, &output.enable_subunits),
            (1, &output.enable_equipments),
            (2, &output.enable_equipment_modules),
        ] {
            match self.project_output_unlocks(lists, kind, country, scopes) {
                Ok(names) if !names.is_empty() => {
                    text.push_str(&self.char_native_text(
                        match kind {
                            0 => b"SPECIAL_PROJECT_OUTPUT_SUBUNITS",
                            1 => b"SPECIAL_PROJECT_OUTPUT_EQUIPMENT",
                            _ => b"SPECIAL_PROJECT_OUTPUT_MODULE",
                        },
                        &[],
                        scopes,
                        false,
                    )?);
                    text.push('\n');
                    for name in names {
                        text.push_str(&name);
                        if kind == 1 {
                            trailing(&mut text, 1);
                        } else {
                            text.push('\n');
                        }
                    }
                }
                Ok(_) => {}
                Err(why) => self.project_output_refusal(why),
            }
        }
        Ok(indent_nonempty_lines(&text, 2))
    }

    fn project_output_refusal(&mut self, why: String) {
        self.refuse(
            "project completion",
            format!("key complete_special_project: {why}"),
        );
    }

    fn project_effect_output(
        &mut self,
        text: &mut String,
        items: &[Item],
        scopes: Scopes,
    ) -> Result<(), String> {
        self.evaluator()?.begin_description();
        let mut output = self.group(items, scopes, 0);
        self.evaluator()?.end_description(scopes)?;
        trailing(&mut output, 2);
        text.push_str(&output);
        Ok(())
    }

    // Resolve the scientist first; otherwise select the first
    // scientist-roster entry with specialization level >=1.
    fn project_output_scientist(
        &self,
        item: &Item,
        scopes: Scopes,
        specialization: Option<&str>,
    ) -> Result<Option<u32>, String> {
        if let Some(operand) = children(item)
            .iter()
            .rev()
            .find(|field| field.keyword_is("scientist"))
            .and_then(|field| field.value.scalar())
            && let Some(character) = self.effect_character_target(Some(operand), scopes)?
        {
            // The shared family helper proves the same character gate.
            return Ok(self
                .char_scientist(Scopes::of(Scope::Character(character.id)))?
                .map(|row| row.id));
        }
        let world = self.context.world()?;
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0,
        };
        let Some(specialization) = specialization else {
            return Ok(None);
        };
        let token = world
            .token_of(specialization.as_bytes())?
            .ok_or("LexicalTokens lacks project specialization")?;
        let scientists = self
            .context
            .text
            .frame()
            .complete::<tables::Scientists>()
            .map_err(|why| why.to_string())?;
        let skills = self
            .context
            .text
            .frame()
            .complete::<tables::ScientistSkills>()
            .map_err(|why| why.to_string())?;
        let eligible: BTreeSet<_> = skills
            .iter()
            .filter(|skill| skill.token == token && skill.level >= 1)
            .map(|skill| skill.character)
            .collect();
        Ok(scientists
            .iter()
            .filter(|row| world.slot(row.country) == Some(country))
            .filter(|row| eligible.contains(&row.character))
            .min_by_key(|row| row.ordinal)
            .map(|row| row.character))
    }

    // native program order, explicit state first, then matching
    // specialization. RaidFacilities holds that exact hourly roster/order.
    fn project_output_program(
        &self,
        item: &Item,
        scopes: Scopes,
        specialization: Option<&str>,
    ) -> Result<Option<(u32, u32)>, String> {
        let world = self.context.world()?;
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0,
        };
        let mut programs: Vec<_> = self
            .context
            .text
            .frame()
            .complete::<tables::RaidFacilities>()
            .map_err(|why| why.to_string())?
            .iter()
            .filter(|row| world.slot(row.country) == Some(country))
            .collect();
        programs.sort_by_key(|row| row.ordinal);
        let state = children(item)
            .iter()
            .rev()
            .find(|field| field.keyword_is("state"))
            .and_then(|field| field.value.scalar())
            .map(|operand| self.evaluator()?.scoped_state(operand, scopes))
            .transpose()?;
        if programs.is_empty() || (state.is_none() && specialization.is_none()) {
            return Ok(None);
        }
        let provinces: BTreeMap<_, _> = self
            .context
            .text
            .frame()
            .complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?
            .iter()
            .map(|row| (row.id, row.state))
            .collect();
        let province_state = |province| {
            provinces
                .get(&province)
                .copied()
                .ok_or_else(|| format!("Provinces lacks program province {province}"))
        };
        if let Some(state) = state {
            for program in &programs {
                let selected = province_state(program.province)?;
                if state == selected {
                    return Ok(Some((program.province, selected)));
                }
            }
        }
        let Some(specialization) = specialization else {
            return Ok(None);
        };
        let rows: BTreeMap<_, _> = self
            .context
            .text
            .frame()
            .complete::<tables::ProjectPrograms>()
            .map_err(|why| why.to_string())?
            .iter()
            .map(|row| (row.id, row))
            .collect();
        let buildings = self
            .context
            .content
            .buildings
            .ok_or_else(|| self.content_gap("buildings"))?;
        for program in programs {
            let row = rows
                .get(&program.id)
                .ok_or("ProjectPrograms lacks roster program configuration")?;
            let name = std::str::from_utf8(&row.building)
                .map_err(|_| "program building token is not UTF-8")?;
            let building = buildings
                .by_token(name)
                .map_err(|why| why.to_string())?
                .ok_or_else(|| format!("building catalogue lacks program configuration {name}"))?;
            if building
                .specialization
                .iter()
                .any(|name| name.eq_ignore_ascii_case(specialization))
            {
                return Ok(Some((program.province, province_state(program.province)?)));
            }
        }
        Ok(None)
    }

    fn project_output_unlocks(
        &self,
        lists: &[Item],
        kind: u8,
        country: u32,
        scopes: Scopes,
    ) -> Result<Vec<String>, String> {
        if lists.is_empty() {
            return Ok(Vec::new());
        }
        if kind == 0 {
            return self
                .technology_enabled_subunits(lists)?
                .into_iter()
                .map(|name| {
                    self.technology_country_name(
                        name.as_bytes(),
                        self.context.perspective,
                        false,
                        scopes,
                    )
                })
                .collect();
        }
        let mut names = Vec::new();
        for list in lists {
            // DLC-gated readers attach the final limit to the entire block.
            let fields = children(list);
            if let Some(limit) = fields.iter().rfind(|item| item.keyword_is("limit"))
                && !self.project_output_condition(children(limit), Scopes::empty())?
            {
                continue;
            }
            let entries = list
                .value
                .scalar()
                .map(|value| vec![value])
                .unwrap_or_else(|| {
                    fields
                        .iter()
                        .filter(|item| !item.keyword_is("limit"))
                        .filter_map(|item| item.value.scalar())
                        .collect()
                });
            for entry in entries {
                let token = installed_content::parser::c_string(entry);
                let name: &[u8] = match kind {
                    1 => {
                        let equipment = self
                            .context
                            .content
                            .equipment
                            .ok_or_else(|| self.content_gap("equipment"))?;
                        match equipment.get(token) {
                            Some(index) => &equipment.equipment[index].name,
                            None => continue,
                        }
                    }
                    _ => {
                        let equipment = self
                            .context
                            .content
                            .equipment
                            .ok_or_else(|| self.content_gap("equipment"))?;
                        let Some(name) = equipment.module(token) else {
                            continue;
                        };
                        if !self.technology_module_allowed(equipment, token, country)? {
                            continue;
                        }
                        name
                    }
                };
                // uses current viewing country,
                // not the project's owner; neither name undergoes expansion.
                names.push(self.technology_country_name(
                    name,
                    self.context.perspective,
                    false,
                    scopes,
                )?);
            }
        }
        Ok(names)
    }

    fn project_output_condition(&self, items: &[Item], scopes: Scopes) -> Result<bool, String> {
        match self.evaluator()?.evaluate(items, scopes) {
            crate::triggers::Eval::True => Ok(true),
            crate::triggers::Eval::False => Ok(false),
            crate::triggers::Eval::Unknown(why) => {
                Err(why.into_iter().collect::<Vec<_>>().join("; "))
            }
        }
    }
}
