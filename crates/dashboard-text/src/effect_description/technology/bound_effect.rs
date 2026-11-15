//! tech_effect's ordinary tooltip body; the callback deliberately omits timing,
//! requirements and the technology's prose.
use super::*;
use installed_content_core::{parser::c_string, tokens::Token};

const TECHNOLOGY_ID: u64 = mask::<replay_data::rows::TechnologyDefinitionRow>(&["id"]);

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn bound_technology_effect(
        &mut self,
        token: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        let catalog = self.technology_catalog()?;
        let Some(template) = catalog.get(token.as_bytes())?.cloned() else {
            return Ok(String::new()); // missing technology gives empty text.
        };
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0, // The formatter reads the current country without state-owner fallback.
        };
        if country == 0 {
            // constructs an empty status roster; calls
            // fresh-init only for country indices starting at one.
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let definition = find(
            self.complete::<tables::TechnologyDefinitions>(TECHNOLOGY_ID)?,
            |row| c_string(&row.id).eq_ignore_ascii_case(c_string(&template.id)),
        )?
        .ok_or("technology definitions lacks bound technology identity")?;
        // The key is the country, then the database index.
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

        // sub unit stat bonus is the same helper used by add_unit_bonus and
        // special-project output. Its icon preference comes from the viewer.
        let mut out = match self.units_bonus_description(&template.unit_bonus, scopes) {
            Ok(text) => text,
            Err(why) => {
                self.refuse("technology unit bonus", why);
                String::new()
            }
        };
        self.technology_unlock_description(&mut out, &template, country, scopes);

        // constructs a fresh scope with this technology's country
        // and tests before describing. Caller links do not leak in.
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
        // uses precision2, include mask -1, scale100000,
        // no expansion; a body of at most one byte is omitted.
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

    // technology constructor builds the ordered immutable roster,
    // skipping unknown types and the DLC-locked floating-harbor category.
    pub(super) fn technology_enabled_equipment(
        &self,
        template: &installed_content_core::technology::Definition,
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
        template: &installed_content_core::technology::Definition,
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
                    } // equipment type.
                    let name = self.technology_country_name(
                        &definition.name,
                        self.context.perspective,
                        false,
                        scopes,
                    )?;
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
                for (ordinal, name) in modules.iter().enumerate() {
                    if !self.technology_module_allowed(&catalog.equipment, name, country)? {
                        continue;
                    }
                    // Pass an explicit zero country tag to equipment naming.
                    let name = self.technology_country_name(name, 0, false, scopes)?;
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
                        // Use the localised singular equipment name.
                        let name = self.localised(building.name.as_bytes(), &[], scopes, false)?;
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
                for (ordinal, name) in sub_units.iter().enumerate() {
                    let name = self.technology_country_name(
                        name.as_bytes(),
                        self.context.perspective,
                        false,
                        scopes,
                    )?;
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
        template: &installed_content_core::technology::Definition,
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
            self.technology_join(&mut names, &name, Some(tactic.id) == last, scopes)?;
        }
        Ok(names)
    }

    // Drop repeated definition pointers before the reader
    // attaches its combined leading limit; admits that first roster.
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
                // Look up the registered token spelling directly, rather than the
                // token lookup which coalesces rejected names to token zero.
                let operand = std::str::from_utf8(c_string(operand))
                    .map_err(|_| "sub-unit unlock name is not UTF-8")?;
                let index = match database.get(operand) {
                    installed_content_core::unit_leader::SubUnit::Index(index) => index,
                    installed_content_core::unit_leader::SubUnit::Unknown => continue,
                    installed_content_core::unit_leader::SubUnit::Uncertain(why) => return Err(why.into()),
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

    // is shared by technology and special-project module unlocks.
    pub(super) fn technology_module_allowed(
        &self,
        equipment: &installed_content_core::equipment::Database,
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

    fn technology_join(
        &self,
        out: &mut String,
        name: &str,
        last: bool,
        scopes: Scopes,
    ) -> Result<(), String> {
        self.technology_join_formatted(out, &format!("\u{11}H{name}\u{11}!"), last, scopes)
    }

    // tests the final roster pointer, even when a later duplicate
    // equipment's display line is skipped; AND includes its authored suffix.
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
        out.push_str(&self.char_native_text(
            key,
            &[(argument, Argument::Text(names))],
            scopes,
            false,
        )?);
        out.push('\n');
        Ok(())
    }
}
