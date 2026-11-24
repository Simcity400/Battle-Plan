use super::*;
use crate::effect_description::{EntityKind, IterationScopes, OutputSection};
use replay_data::rows::{
    ProjectProgramRow, ProvinceRow, RaidFacilityRow, ScientistRow, ScientistSkillRow, tables,
};
use std::collections::BTreeSet;

const SCIENTIST: u64 = mask::<ScientistRow>(&["country", "ordinal"]);
const SKILL: u64 = mask::<ScientistSkillRow>(&["token", "level"]);
const PROGRAM: u64 = mask::<RaidFacilityRow>(&["id", "province"]);
const PROVINCE_STATE: u64 = mask::<ProvinceRow>(&["state"]);
const PROGRAM_BUILDING: u64 = mask::<ProjectProgramRow>(&["building"]);

#[cfg(test)]
mod tests;

fn push_section(sections: &mut Vec<OutputSection>, heading: Option<String>, text: &str) {
    let text = text.trim_end_matches('\n');
    if !text.is_empty() {
        sections.push(OutputSection {
            heading,
            text: text.to_owned(),
        });
    }
}

fn trailing(text: &mut String, count: usize) {
    if text.is_empty() {
        return;
    }
    while text.bytes().rev().take_while(|byte| *byte == b'\n').count() < count {
        text.push('\n');
    }
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description::technology) fn complete_project_output(
        &mut self,
        item: &Item,
        scopes: Scopes,
        owner: u32,
        id: u32,
    ) -> Result<String, String> {
        let world = self.context.world()?;
        let row = world.project(owner, id).map_err(|why| why.to_string())?;
        let token = world
            .token(row.definition_id)
            .map_err(|why| why.to_string())?
            .ok_or("lexical tokens lacks project definition")?;
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
        let scientist = self.project_output_scientist(item, scopes, specialization.as_deref());
        let facility = self.project_output_program(item, scopes, specialization.as_deref());
        self.project_output_body(&output, item, scopes, owner, id, scientist, facility)
    }

    pub(in crate::effect_description) fn project_output_body(
        &mut self,
        output: &content_reader_military::special_projects::Output,
        item: &Item,
        scopes: Scopes,
        owner: u32,
        id: u32,
        scientist: Result<Option<u32>, String>,
        facility: Result<Option<(u32, u32)>, String>,
    ) -> Result<String, String> {
        let world = self.context.world()?;
        let country = world
            .slot(owner)
            .ok_or("country links lacks project owner")?;
        let project = self
            .evaluator()?
            .fresh_country_scope(Some(0), None)
            .map_err(|why| why.to_string())?;
        let project = self
            .evaluator()?
            .repoint_this(project, Scope::Project { owner, id })
            .map_err(|why| why.to_string())?;
        let mut text =
            self.char_native_text(b"SPECIAL_PROJECT_RESEARCH_OUTPUT", &[], scopes, false)?;
        trailing(&mut text, 1);

        if !output.country_effects.is_empty() {
            let selected = self
                .evaluator()?
                .fresh_country_scope(Some(country), Some(project))
                .map_err(|why| why.to_string())?;
            self.project_effect_output(&mut text, &output.country_effects, selected)?;
        }
        if !output.scientist_effects.is_empty() {
            match scientist {
                Ok(Some(character)) => {
                    let selected = self
                        .evaluator()?
                        .fresh_country_scope(Some(0), Some(project))
                        .map_err(|why| why.to_string())?;
                    let selected = self
                        .evaluator()?
                        .repoint_this(selected, Scope::Character(character))
                        .map_err(|why| why.to_string())?;
                    self.project_effect_output(&mut text, &output.scientist_effects, selected)?;
                }
                Ok(None) => {}
                Err(why) => self.project_output_refusal(why),
            }
        }
        if !output.facility_state_effects.is_empty() {
            match facility {
                Ok(Some((province, state))) => {
                    let selected = self
                        .evaluator()?
                        .fresh_country_scope(Some(0), Some(project))
                        .map_err(|why| why.to_string())?;
                    let selected = self
                        .evaluator()?
                        .repoint_this(selected, Scope::State(state))
                        .map_err(|why| why.to_string())?;
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
                Ok(None) => {}
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
            match self.project_equipment_bonus(item, &output.equipment_bonus, scopes) {
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

    pub(in crate::effect_description) fn project_iteration_body(
        &mut self,
        output: &content_reader_military::special_projects::Output,
        scopes: Scopes,
        owner: u32,
        id: u32,
        iteration: IterationScopes,
    ) -> Result<(Option<String>, Vec<OutputSection>), String> {
        let country = self
            .context
            .world()?
            .slot(owner)
            .ok_or("country links lacks project owner")?;
        let project = self
            .evaluator()?
            .fresh_country_scope(Some(0), None)
            .map_err(|why| why.to_string())?;
        let project = self
            .evaluator()?
            .repoint_this(project, Scope::Project { owner, id })
            .map_err(|why| why.to_string())?;
        let mut sections = Vec::new();
        if !output.country_effects.is_empty() {
            let selected = self
                .evaluator()?
                .fresh_country_scope(Some(country), Some(project))
                .map_err(|why| why.to_string())?;
            self.iteration_effects(
                &mut sections,
                &output.country_effects,
                selected,
                iteration.country_seed,
                "country effects",
            );
        }
        if !output.scientist_effects.is_empty() {
            match iteration.scientist {
                Ok(Some(character)) => {
                    let selected = self
                        .evaluator()?
                        .fresh_country_scope(Some(0), Some(project))
                        .map_err(|why| why.to_string())?;
                    let selected = self
                        .evaluator()?
                        .repoint_this(selected, Scope::Character(character))
                        .map_err(|why| why.to_string())?;
                    self.iteration_effects(
                        &mut sections,
                        &output.scientist_effects,
                        selected,
                        iteration.scientist_seed,
                        "scientist effects",
                    );
                }
                Ok(None) => {}
                Err(why) => self.refuse("scientist effects", why),
            }
        }
        if !output.facility_state_effects.is_empty() {
            match iteration.facility {
                Ok(Some((province, state))) => {
                    let selected = self
                        .evaluator()?
                        .fresh_country_scope(Some(0), Some(project))
                        .map_err(|why| why.to_string())?;
                    let selected = self
                        .evaluator()?
                        .repoint_this(selected, Scope::State(state))
                        .map_err(|why| why.to_string())?;
                    self.evaluator()?.store_description_temp(
                        b"facility_province_id".to_vec(),
                        i64::from(province) * 100_000,
                    );
                    self.iteration_effects(
                        &mut sections,
                        &output.facility_state_effects,
                        selected,
                        iteration.state_seed,
                        "state effects",
                    );
                }
                Ok(None) => {}
                Err(why) => self.refuse("state effects", why),
            }
        }
        if !output.sub_unit_bonus.is_empty() {
            match self.units_bonus_description(&output.sub_unit_bonus, scopes) {
                Ok(text) => self.iteration_section(
                    &mut sections,
                    Some(b"SPECIAL_PROJECT_REWARD_UNIT_BONUS"),
                    &text,
                    scopes,
                    "unit bonus",
                ),
                Err(why) => self.refuse("unit bonus", why),
            }
        }
        if !output.equipment_bonus.is_empty() {
            let blank = Item {
                key: None,
                key_quoted: false,
                operator: Default::default(),
                value: content_reader_parser::parser::Value::Block(Vec::new()),
                kind: content_reader_parser::parser::Kind::Open,
            };
            match self.project_equipment_bonus(&blank, &output.equipment_bonus, scopes) {
                Ok(text) => {
                    self.iteration_section(&mut sections, None, &text, scopes, "equipment bonus")
                }
                Err(why) => self.refuse("equipment bonus", why),
            }
        }
        for (kind, lists, heading, what) in [
            (
                0,
                &output.enable_subunits,
                b"SPECIAL_PROJECT_REWARD_SUBUNITS".as_slice(),
                "sub-unit unlocks",
            ),
            (
                2,
                &output.enable_equipment_modules,
                b"SPECIAL_PROJECT_REWARD_EQUIPMENT_MODULE".as_slice(),
                "module unlocks",
            ),
            (
                1,
                &output.enable_equipments,
                b"SPECIAL_PROJECT_REWARD_EQUIPMENT".as_slice(),
                "equipment unlocks",
            ),
        ] {
            match self.project_output_unlocks(lists, kind, country, scopes) {
                Ok(names) if !names.is_empty() => {
                    self.iteration_section(&mut sections, Some(heading), &names.join("\n"), scopes, what)
                }
                Ok(_) => {}
                Err(why) => self.refuse(what, why),
            }
        }
        let heading = if sections.is_empty() {
            None
        } else {
            Some(self.char_native_text(b"SPECIAL_PROJECT_REWARDS_HEADER_TEXT", &[], scopes, false)?)
        };
        Ok((heading, sections))
    }

    fn iteration_effects(
        &mut self,
        sections: &mut Vec<OutputSection>,
        items: &[Item],
        selected: Scopes,
        seed: Option<u32>,
        what: &str,
    ) {
        if let Err(why) = self.seed_iteration_scope(selected, seed) {
            self.refuse(what, why);
            return;
        }
        let mut text = String::new();
        match self.project_effect_output(&mut text, items, selected) {
            Ok(()) => push_section(sections, None, &text),
            Err(why) => self.refuse(what, why),
        }
    }

    fn seed_iteration_scope(&self, selected: Scopes, seed: Option<u32>) -> Result<(), String> {
        let Some(seed) = seed else {
            return Ok(());
        };
        let at = selected
            .cursor
            .ok_or("iteration effect scope has no scope graph")?;
        let mut graph = self.graph.borrow_mut();
        let scope = graph
            .scopes
            .get_mut(at)
            .ok_or("iteration effect scope is absent from its graph")?;
        scope.random_seed = crate::scripted_text::random::scope_seed(seed);
        scope.random_counter = crate::scripted_text::random::scope_counter(seed);
        Ok(())
    }

    fn iteration_section(
        &mut self,
        sections: &mut Vec<OutputSection>,
        heading: Option<&[u8]>,
        text: &str,
        scopes: Scopes,
        what: &str,
    ) {
        let heading = match heading
            .map(|key| self.char_native_text(key, &[], scopes, false))
            .transpose()
        {
            Ok(heading) => heading,
            Err(why) => {
                self.refuse(what, why);
                return;
            }
        };
        push_section(sections, heading, text);
    }

    fn project_equipment_bonus(
        &self,
        item: &Item,
        equipment_bonus: &[Item],
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut bonus = item.clone();
        bonus.key = Some(b"bonus".to_vec());
        bonus.value = content_reader_parser::parser::Value::Block(equipment_bonus.to_vec());
        let mut adapter = item.clone();
        adapter.value = content_reader_parser::parser::Value::Block(vec![bonus]);
        self.units_equipment_bonus(&adapter, scopes)
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
        self.evaluator()?
            .end_description(scopes)
            .map_err(|why| why.to_string())?;
        trailing(&mut output, 2);
        text.push_str(&output);
        Ok(())
    }

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
            .token_of(specialization.as_bytes())
            .map_err(|why| why.to_string())?
            .ok_or("lexical tokens lacks project specialization")?;
        let scientists = self.complete::<tables::Scientists>(SCIENTIST)?;
        let skills = self.complete::<tables::ScientistSkills>(SKILL)?;
        let mut eligible = BTreeSet::new();
        for skill in each(skills) {
            let skill = skill?;
            if skill.token == token && skill.level >= 1 {
                eligible.insert(skill.character);
            }
        }
        let mut selected: Option<Cow<'_, ScientistRow>> = None;
        for row in each(scientists) {
            let row = row?;
            if world.slot(row.country) == Some(country)
                && eligible.contains(&row.character)
                && selected.as_ref().is_none_or(|first| row.ordinal < first.ordinal)
            {
                selected = Some(row);
            }
        }
        Ok(selected.map(|row| row.character))
    }

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
        let mut programs = Vec::new();
        for row in self
            .complete::<tables::RaidFacilities>(PROGRAM)?
            .keyed(|key| world.slot((key >> 32) as u32) == Some(country))
        {
            programs.push(row.map_err(built)?);
        }
        programs.sort_by_key(|row| row.ordinal);
        let state = children(item)
            .iter()
            .rev()
            .find(|field| field.keyword_is("state"))
            .and_then(|field| field.value.scalar())
            .map(|operand| self.evaluator()?.scoped_state(operand, scopes))
            .transpose()
            .map_err(|why| why.to_string())?;
        if programs.is_empty() || (state.is_none() && specialization.is_none()) {
            return Ok(None);
        }
        let provinces = self.complete::<tables::Provinces>(PROVINCE_STATE)?;
        let province_state = |province: u32| -> Result<u32, String> {
            keyed(&provinces, u128::from(province))?
                .map(|row| row.state)
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
        let rows = self.complete::<tables::ProjectPrograms>(PROGRAM_BUILDING)?;
        let buildings = self
            .context
            .content
            .buildings
            .ok_or_else(|| self.content_gap("buildings"))?;
        for program in programs {
            let row = keyed(&rows, u128::from(program.id))?
                .ok_or("project programs lacks roster program configuration")?;
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
                    .and_then(|shown| self.technology_entity(EntityKind::SubUnit, name.as_bytes(), shown))
                })
                .collect();
        }
        let mut names = Vec::new();
        for list in lists {
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
                let token = content_reader_parser::parser::c_string(entry);
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
                let shown =
                    self.technology_country_name(name, self.context.perspective, false, scopes)?;
                let entity = if kind == 1 {
                    EntityKind::Equipment
                } else {
                    EntityKind::Module
                };
                names.push(self.technology_entity(entity, token, shown)?);
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
