use super::*;
use crate::effect_description::entity_links::{EntityKind, mark_at, unmark};
use content_reader_parser::{parser::c_string, tokens::Token};

const TECHNOLOGY_ID: u64 = mask::<replay_data::rows::TechnologyDefinitionRow>(&["id"]);

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn bound_technology_effect(
        &mut self,
        token: &str,
        database_index: Option<i32>,
        scopes: Scopes,
    ) -> Result<String, String> {
        let catalog = self.technology_catalog()?;
        let template = match database_index {
            Some(index) => Some(catalog.template(index, token.as_bytes())?.ok_or_else(|| format!("technology {token}: recorded template {index} is absent from the content"))?),
            None => catalog.get(token.as_bytes())?,
        };
        let Some(template) = template.cloned() else {
            return Ok(String::new());
        };
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0,
        };
        if country == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let definition = find(
            self.complete::<tables::TechnologyDefinitions>(TECHNOLOGY_ID)?,
            |row| database_index.is_none_or(|index| row.database_index == index) && c_string(&row.id).eq_ignore_ascii_case(c_string(&template.id)),
        )?
        .ok_or("technology definitions lacks bound technology identity")?;
        let status = self
            .complete::<tables::ResearchTechnologies>(0)?
            .keys()
            .any(|key| {
                key as u32 as i32 == definition.database_index
                    && world.slot((key >> 32) as u32) == Some(country)
            });
        if !status {
            return Err("research technologies lacks bound country's technology status".into());
        }
        if let Some(why) = &catalog.effect_refusal {
            return Err(why.clone());
        }

        let mut out = match self.units_bonus_description(&template.unit_bonus, scopes) {
            Ok(text) => text,
            Err(why) => {
                self.refuse("technology unit bonus", why);
                String::new()
            }
        };
        self.technology_unlock_description(&mut out, &template, country, scopes);

        if !template.on_research_complete.is_empty() {
            let selected = (|| {
                let selected = self
                    .evaluator()?
                    .fresh_country_scope(Some(country), None)
                    .map_err(|why| why.to_string())?;
                self.technology_condition(&template.on_research_complete_limit, selected)
                    .map(|allowed| allowed.then_some(selected))
            })();
            match selected {
                Ok(Some(selected)) => {
                    out.push_str(&self.group(&template.on_research_complete, selected, 0));
                    out.push('\n');
                }
                Ok(None) => {}
                Err(why) => self.refuse("technology research completion", why),
            }
        }
        match self.technology_tactic_names(&template, scopes) {
            Ok(tactics) if !tactics.is_empty() => {
                if let Err(why) = self.technology_unlock_header(
                    &mut out,
                    b"RESEARCH_UNLOCKS_TACTICS_HEADER",
                    "TACTICS",
                    &tactics,
                    scopes,
                ) {
                    self.refuse("technology tactics", why);
                }
            }
            Ok(_) => {}
            Err(why) => self.refuse("technology tactics", why),
        }
        if template.modifiers.listed().next().is_some()
            || !template.modifiers.tooltips().is_empty()
            || template.modifiers.unheld.cause().is_some()
        {
            let modifiers = self.modifier_description_with(
                &template.modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    ..Default::default()
                },
            );
            match modifiers {
                Ok(modifiers) if modifiers.len() > 1 => {
                    out.push_str(&modifiers);
                    out.push('\n');
                }
                Ok(_) => {}
                Err(why) => self.refuse("technology modifiers", why),
            }
        }
        Ok(out)
    }

    fn technology_condition(&self, body: &[Item], scopes: Scopes) -> Result<bool, String> {
        self.evaluator()?.evaluate(body, scopes).decided()
    }

    pub(super) fn technology_enabled_equipment(
        &self,
        template: &content_reader_military::technology::Definition,
    ) -> Result<Vec<usize>, String> {
        let catalog = self.technology_catalog()?;
        let mut equipment = Vec::new();
        for entry in &template.equipment {
            if entry.keyword_is("limit") {
                if self.technology_condition(children(entry), Scopes::empty())? {
                    continue;
                }
                break;
            }
            let name = entry
                .value
                .scalar()
                .ok_or("technology equipment reader is unsettled")?;
            let Some(index) = catalog.equipment.get(name) else {
                continue;
            };
            let definition = &catalog.equipment.equipment[index];
            if catalog.equipment.category(definition) & 0x200 != 0 {
                let features = each(self.complete::<tables::SimulationFeatures>(DLC_MASK)?)
                    .next()
                    .transpose()?
                    .ok_or("simulation features absent")?;
                if features.dlc_mask & 0x80 == 0 {
                    continue;
                }
            }
            if !equipment.contains(&index) {
                equipment.push(index);
            }
        }
        Ok(equipment)
    }

    fn technology_unlock_description(
        &mut self,
        out: &mut String,
        template: &content_reader_military::technology::Definition,
        country: u32,
        scopes: Scopes,
    ) {
        let equipment: Result<(), String> = (|| {
            let catalog = self.technology_catalog()?;
            let equipment = self.technology_enabled_equipment(template)?;
            if !equipment.is_empty() {
                let mut names = String::new();
                for (ordinal, &index) in equipment.iter().enumerate() {
                    let definition = &catalog.equipment.equipment[index];
                    if definition.duplicate {
                        continue;
                    }
                    let name = self.technology_country_name(
                        &definition.name,
                        self.context.perspective,
                        false,
                        scopes,
                    )?;
                    let name = self.technology_entity(EntityKind::Equipment, &definition.name, name)?;
                    self.technology_join(
                        &mut names,
                        &name,
                        ordinal + 1 == equipment.len(),
                        scopes,
                    )?;
                }
                self.technology_unlock_header(
                    out,
                    b"RESEARCH_UNLOCKS_EQUIPMENT_HEADER",
                    "EQUIPMENTS",
                    &names,
                    scopes,
                )?;
            }
            Ok(())
        })();
        if let Err(why) = equipment {
            self.refuse("technology equipment unlocks", why);
        }
        let modules: Result<(), String> = (|| {
            let catalog = self.technology_catalog()?;
            let mut modules = Vec::new();
            for entry in &template.modules {
                let operand = entry
                    .value
                    .scalar()
                    .ok_or("technology module unlock is not scalar")?;
                if let Some(name) = catalog.equipment.module(operand)
                    && !modules
                        .iter()
                        .any(|old: &&[u8]| Token::c_str(old) == Token::c_str(name))
                {
                    modules.push(name);
                }
            }
            if !modules.is_empty() {
                let mut names = String::new();
                for (ordinal, token) in modules.iter().enumerate() {
                    if !self.technology_module_allowed(&catalog.equipment, token, country)? {
                        continue;
                    }
                    let name = self.technology_country_name(token, 0, false, scopes)?;
                    let name = self.technology_entity(EntityKind::Module, token, name)?;
                    self.technology_join(&mut names, &name, ordinal + 1 == modules.len(), scopes)?;
                }
                if !names.is_empty() {
                    self.technology_unlock_header(
                        out,
                        b"RESEARCH_UNLOCKS_EQUIPMENT_MODULES_HEADER",
                        "MODULES",
                        &names,
                        scopes,
                    )?;
                }
            }
            Ok(())
        })();
        if let Err(why) = modules {
            self.refuse("technology module unlocks", why);
        }
        let buildings: Result<(), String> = (|| {
            if !template.buildings.is_empty() {
                let database = self
                    .context
                    .content
                    .buildings
                    .ok_or_else(|| self.content_gap("buildings"))?;
                let mut buildings = Vec::new();
                for (token, level) in &template.buildings {
                    let token = std::str::from_utf8(c_string(token))
                        .map_err(|_| "technology building token is not UTF-8")?;
                    if let Some(building) =
                        database.by_token(token).map_err(|why| why.to_string())?
                    {
                        let name = self.localised(building.name.as_bytes(), &[], scopes, false)?;
                        let name =
                            self.technology_entity(EntityKind::Building, building.name.as_bytes(), name)?;
                        let level = self.char_native_text(
                            b"BUILDING_LEVEL",
                            &[("LEVEL", Argument::Int32(*level))],
                            scopes,
                            false,
                        )?;
                        buildings.push((building, format!("\u{11}H{name}\u{11}! ({level})")));
                    }
                }
                if !buildings.is_empty() {
                    let mut names = String::new();
                    let last = buildings.last().unwrap().0;
                    for (building, name) in &buildings {
                        self.technology_join_formatted(
                            &mut names,
                            name,
                            std::ptr::eq(*building, last),
                            scopes,
                        )?;
                    }
                    self.technology_unlock_header(
                        out,
                        b"RESEARCH_UNLOCKS_BUILDINGS_HEADER",
                        "BUILDINGS",
                        &names,
                        scopes,
                    )?;
                }
            }
            Ok(())
        })();
        if let Err(why) = buildings {
            self.refuse("technology building unlocks", why);
        }
        let sub_units: Result<(), String> = (|| {
            let sub_units = self.technology_enabled_subunits(&template.sub_units)?;
            if !sub_units.is_empty() {
                let mut names = String::new();
                for (ordinal, token) in sub_units.iter().enumerate() {
                    let name = self.technology_country_name(
                        token.as_bytes(),
                        self.context.perspective,
                        false,
                        scopes,
                    )?;
                    let name = self.technology_entity(EntityKind::SubUnit, token.as_bytes(), name)?;
                    self.technology_join(
                        &mut names,
                        &name,
                        ordinal + 1 == sub_units.len(),
                        scopes,
                    )?;
                }
                self.technology_unlock_header(
                    out,
                    b"RESEARCH_UNLOCKS_SUBUNITS_HEADER",
                    "SUBUNITS",
                    &names,
                    scopes,
                )?;
            }
            Ok(())
        })();
        if let Err(why) = sub_units {
            self.refuse("technology sub-unit unlocks", why);
        }
    }

    fn technology_tactic_names(
        &self,
        template: &content_reader_military::technology::Definition,
        scopes: Scopes,
    ) -> Result<String, String> {
        if template.tactics.is_empty() {
            return Ok(String::new());
        }
        let database = self
            .technology_catalog()?
            .tactics
            .as_ref()
            .ok_or("technology combat-tactic catalogue unavailable")?
            .as_ref()
            .map_err(Clone::clone)?;
        let mut tactics = Vec::new();
        for entry in &template.tactics {
            let operand = entry.value.read_string().unwrap_or_default();
            match database
                .tactics
                .iter()
                .find(|tactic| tactic.token == operand)
            {
                Some(tactic) => {
                    if let Some(why) = &tactic.unproven {
                        return Err(why.clone());
                    }
                    tactics.push(tactic);
                }
                None => {
                    if let Some(why) = database.unsupported.first() {
                        return Err(why.clone());
                    }
                }
            }
        }
        let mut names = String::new();
        let last = tactics.last().map(|tactic| tactic.id);
        for tactic in &tactics {
            let name = self.technology_country_name(
                &tactic.token,
                self.context.perspective,
                false,
                scopes,
            )?;
            let name = self.technology_entity(EntityKind::Tactic, &tactic.token, name)?;
            self.technology_join(&mut names, &name, Some(tactic.id) == last, scopes)?;
        }
        Ok(names)
    }

    pub(super) fn technology_enabled_subunits(&self, lists: &[Item]) -> Result<Vec<&str>, String> {
        if lists.is_empty() {
            return Ok(Vec::new());
        }
        let database = self
            .context
            .content
            .key_names
            .and_then(|names| names.sub_units.as_ref())
            .ok_or("sub-unit catalogue unavailable")?;
        let mut roster = Vec::new();
        for list in lists {
            let operands: Vec<_> = match list.value.scalar() {
                Some(operand) => vec![operand],
                None => children(list)
                    .iter()
                    .filter(|entry| !entry.keyword_is("limit"))
                    .filter_map(|entry| entry.value.scalar())
                    .collect(),
            };
            for operand in operands {
                let operand = std::str::from_utf8(c_string(operand))
                    .map_err(|_| "sub-unit unlock name is not UTF-8")?;
                let index = match database.get(operand) {
                    content_reader_script::sub_units::SubUnit::Index(index) => index,
                    content_reader_script::sub_units::SubUnit::Unknown => continue,
                    content_reader_script::sub_units::SubUnit::Uncertain(why) => return Err(why.into()),
                };
                let name = database
                    .name(index)
                    .ok_or("sub-unit index has no registered name")??;
                let name = std::str::from_utf8(c_string(name.as_bytes()))
                    .map_err(|_| "sub-unit unlock name is not UTF-8")?;
                if !roster.iter().any(|(old, _): &(&str, &Item)| *old == name) {
                    roster.push((name, list));
                }
            }
        }
        let mut admitted = Vec::new();
        for (name, list) in roster {
            let limit: Vec<_> = children(list)
                .iter()
                .filter(|entry| entry.keyword_is("limit"))
                .flat_map(|entry| children(entry).iter().cloned())
                .collect();
            if limit.is_empty() || self.technology_condition(&limit, Scopes::empty())? {
                admitted.push(name);
            }
        }
        Ok(admitted)
    }

    pub(super) fn technology_module_allowed(
        &self,
        equipment: &content_reader_key_names::equipment::Database,
        token: &[u8],
        country: u32,
    ) -> Result<bool, String> {
        let conditions = equipment
            .module_conditions(token)
            .ok_or("module admission projection is unavailable")?;
        let selected = self
            .evaluator()?
            .fresh_country_scope(Some(country), None)
            .map_err(|why| why.to_string())?;
        if let Some(limit) = &conditions.limit
            && !self.technology_condition(limit, selected)?
        {
            return Ok(false);
        }
        self.technology_condition(&conditions.allowed, selected)
    }

    pub(super) fn technology_entity(
        &self,
        kind: EntityKind,
        token: &[u8],
        name: String,
    ) -> Result<String, String> {
        self.technology_entity_at(kind, token, None, name)
    }

    pub(super) fn technology_entity_at(
        &self,
        kind: EntityKind,
        token: &[u8],
        index: Option<i32>,
        name: String,
    ) -> Result<String, String> {
        if !self.context.entity_links {
            return Ok(name);
        }
        let token = std::str::from_utf8(c_string(token)).map_err(|_| {
            format!(
                "the {kind:?} link token {} is not UTF-8",
                String::from_utf8_lossy(token)
            )
        })?;
        Ok(mark_at(kind, token, index, &name))
    }

    fn technology_join(
        &self,
        out: &mut String,
        name: &str,
        last: bool,
        scopes: Scopes,
    ) -> Result<(), String> {
        self.technology_join_formatted(out, &format!("\u{11}H{name}\u{11}!"), last, scopes)
    }

    fn technology_join_formatted(
        &self,
        out: &mut String,
        name: &str,
        last: bool,
        scopes: Scopes,
    ) -> Result<(), String> {
        if !out.is_empty() {
            out.push_str(if last { " " } else { ", " });
            if last {
                out.push_str(&self.localised(b"AND", &[], scopes, false)?);
            }
        }
        out.push_str(name);
        Ok(())
    }

    fn technology_unlock_header(
        &self,
        out: &mut String,
        key: &[u8],
        argument: &str,
        names: &str,
        scopes: Scopes,
    ) -> Result<(), String> {
        out.push_str(&self.linked_text(names, |names| {
            self.char_native_text(key, &[(argument, Argument::Text(names))], scopes, false)
        })?);
        out.push('\n');
        Ok(())
    }

    pub(super) fn linked_text(
        &self,
        names: &str,
        render: impl Fn(&str) -> Result<String, String>,
    ) -> Result<String, String> {
        let text = render(names)?;
        if !self.context.entity_links {
            return Ok(text);
        }
        if unmark(&text) != render(&unmark(names))? {
            return Err(
                "the game's formatting changes the linked names in this line, so their links are not reproduced"
                    .into(),
            );
        }
        Ok(text)
    }
}
