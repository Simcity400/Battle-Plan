use super::*;
use replay_data::rows::{
    BuildingRow, CountryAppearanceRow, RaidFacilityRow, TechnologyDefinitionRow,
};

const BUILDING_STATUS: u64 = mask::<BuildingRow>(&["ordinal", "status_flags", "active"]);
const FACILITY: u64 = mask::<RaidFacilityRow>(&["owner", "province", "active_project"]);
const TECHNOLOGY: u64 = mask::<TechnologyDefinitionRow>(&["technology_handle", "id"]);
const NAME_TAG: u64 = mask::<CountryAppearanceRow>(&["name_tag"]);

mod bound_effect;
mod project_output;
#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    fn technology_catalog(&self) -> Result<&content_reader_military::technology::Catalog, String> {
        self.context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?
            .technologies
            .as_ref()
            .ok_or("technology description catalogue unavailable".into())
    }

    pub(super) fn breakthrough(
        &self,
        item: &Item,
        scopes: Scopes,
        progress: bool,
    ) -> Result<String, String> {
        let fields = children(item);
        let mut tokens = Vec::new();
        let world = self.context.world()?;
        let catalog = self
            .character_content()?
            .special_projects
            .as_ref()
            .map_err(Clone::clone)?;
        for entry in fields
            .iter()
            .filter(|entry| entry.keyword_is("specialization"))
        {
            if entry.value_keyword().as_deref() == Some("all") {
                for name in catalog.specializations_in_order()? {
                    tokens.push(
                        world
                            .token_of(name.as_bytes())
                            .map_err(|why| why.to_string())?
                            .ok_or("lexical tokens lacks breakthrough specialization")?,
                    );
                }
                continue;
            }
            let operand = entry.value.scalar().unwrap_or_default();
            if catalog
                .specialization(&content_reader_parser::tokens::Token::c_str(operand))?
                .is_none()
            {
                continue;
            }
            let token = world
                .token_of(operand)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks breakthrough specialization")?;
            tokens.push(token);
        }
        if tokens.is_empty() {
            return Ok(String::new());
        }
        let raw_country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                world.raw_country(country).unwrap_or(0)
            }
            _ => 0,
        };
        let amount = field(fields, "value");
        let mut rows = String::new();
        for token in content_reader_military::special_projects::specialization_order(&tokens) {
            let operand = world
                .token(token)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks specialization text")?;
            let Some(name) = self.char_specialization_name(operand, raw_country, scopes)? else {
                continue;
            };
            let amount = if progress {
                Argument::Fixed5(amount.map(content_reader_script::modifiers::stored).unwrap_or(0))
            } else {
                Argument::Int32(
                    amount
                        .and_then(|field| field.value.scalar())
                        .map(integer)
                        .unwrap_or(0),
                )
            };
            let mut row = self.char_native_text(
                if progress {
                    b"ADD_BREAKTHROUGH_PROGRESS_EFFECT_SPECIALIZATION"
                } else {
                    b"ADD_BREAKTHROUGH_POINTS_EFFECT_SPECIALIZATION"
                },
                &[
                    ("SPECIALIZATION", Argument::Text(&name)),
                    ("AMOUNT", amount),
                ],
                scopes,
                false,
            )?;
            if !row.ends_with('\n') {
                row.push('\n');
            }
            rows.push_str(&row);
        }
        let mut header = self.char_native_text(
            if progress {
                b"ADD_BREAKTHROUGH_PROGRESS_EFFECT_DESC"
            } else {
                b"ADD_BREAKTHROUGH_POINTS_EFFECT_DESC"
            },
            &[],
            scopes,
            false,
        )?;
        if !header.ends_with('\n') {
            header.push('\n');
        }
        header.push_str(&indent_nonempty_lines(&rows, 2));
        if !header.ends_with('\n') {
            header.push('\n');
        }
        Ok(header)
    }


    pub(super) fn faction_power_projection(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let amount = self.scoped_value(scalar_operand(item)?, scopes)?;
        let faction = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0 => self
                .context
                .world()?
                .faction(country)
                .map_err(|why| why.to_string())?,
            _ => None,
        };
        let name = faction
            .map(|id| self.context.text.faction_name_by_id(id))
            .transpose()?;
        let text = match name.as_deref() {
            Some(name) => self.char_native_text(
                b"ADD_FACTION_POWER_PROJECTION_EFFECT",
                &[
                    ("VAL", Argument::Fixed5(amount)),
                    ("FACTION", Argument::Text(name)),
                ],
                scopes,
                false,
            )?,
            None => self.char_native_text(
                b"ADD_FACTION_POWER_PROJECTION_EFFECT_NO_FACTION",
                &[("VAL", Argument::Fixed5(amount))],
                scopes,
                false,
            )?,
        };
        self.expanded_with_faction_environment(&text, scopes, Some(faction))
    }


    pub(super) fn project_progress(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let amount = self.scoped_value(scalar_operand(item)?, scopes)?;
        let Scope::Project { owner, id } = scopes.this else {
            return Err("add_project_progress_ratio requires the PROJECT carrier".into());
        };
        let name = self.project_name(owner, id)?;
        self.char_native_text(
            b"EFFECT_ADD_SPECIAL_PROJECT_PROGRESS",
            &[
                ("PROJECT", Argument::Text(&name)),
                ("PROGRESS", Argument::Fixed5(amount)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn complete_project(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| {
                children(item)
                    .iter()
                    .rev()
                    .find(|field| field.keyword_is("project"))
                    .and_then(|field| field.value.read_string())
            })
            .unwrap_or_default();
        let selected = self
            .evaluator()?
            .scoped_scope(operand, scopes)
            .map_err(|why| why.to_string())?;
        let project = match selected {
            Some(Scope::Project { owner, id }) => Some((owner, id)),
            _ => None,
        };
        let name = match project {
            Some((owner, id)) => self.project_name(owner, id)?,
            None => std::str::from_utf8(content_reader_parser::parser::c_string(operand))
                .map_err(|_| "complete_special_project operand is not UTF-8")?
                .to_owned(),
        };
        let mut text = self.char_native_text(
            b"EFFECT_COMPLETE_SPECIAL_PROJECT",
            &[("PROJECT", Argument::Text(&name))],
            scopes,
            true,
        )?;
        if content_reader_parser::parser::read_bool_field(children(item), "show_modifiers", true)
            && let Some((owner, id)) = project
        {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            match self.complete_project_output(item, scopes, owner, id) {
                Ok(output) => text.push_str(&output),
                Err(why) => self.refuse(
                    "project completion",
                    format!("key complete_special_project: {why}"),
                ),
            }
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }


    pub(super) fn raid_project_progress(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(project) = self.raid_target_project(scopes)? else {
            return Ok(String::new());
        };
        let world = self.context.world()?;
        let token = world
            .token(project.definition_id)
            .map_err(|why| why.to_string())?
            .ok_or("lexical tokens lacks raid target project definition")?;
        let token = std::str::from_utf8(token)
            .map_err(|_| "raid target project definition is not UTF-8")?;
        let catalog = self
            .character_content()?
            .special_projects
            .as_ref()
            .map_err(Clone::clone)?;
        if let Some(why) = &catalog.unsettled {
            return Err(why.clone());
        }
        let definition = catalog
            .projects
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(token))
            .map(|(_, definition)| definition)
            .ok_or("raid target project definition is absent from content")?;
        if let Some(why) = &definition.refusal {
            return Err(why.clone());
        }
        let default = self
            .context
            .text
            .content()
            .environment()?
            .defines()
            .int32("NProject", "DEFAULT_COMPLEXITY")
            .map_err(|why| why.to_string())?;
        let constants = None;
        let (min, max) = definition.complexity(constants, default)?;
        let denominator = i64::from(min.wrapping_add(max)).wrapping_mul(10_000_000_000) / 200_000;
        let operand = scalar_operand(item)?;
        let amount = self.scoped_value(operand, scopes)?;
        let ratio = if denominator != 0
            && (amount as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
        {
            amount.wrapping_mul(100_000).wrapping_div(denominator)
        } else {
            0xffff_ffff
        };
        let days = crate::triggers::fixed_round(
            i64::from(definition.prototype_time(constants)?)
                .wrapping_mul(ratio)
                .wrapping_mul(100),
        ) as i32;
        let progress = self.scoped_value(operand, scopes)?;
        let name = self.project_name(project.owner, project.local_id)?;
        self.char_native_text(
            b"EFFECT_REDUCE_SPECIAL_PROJECT_PROGRESS",
            &[
                ("PROJECT", Argument::Text(&name)),
                ("PROGRESS", Argument::Fixed5(progress)),
                ("DAYS", Argument::Int32(days)),
            ],
            scopes,
            true,
        )
    }

    fn raid_target_project(
        &self,
        scopes: Scopes,
    ) -> Result<Option<std::borrow::Cow<'_, replay_data::rows::ProjectRow>>, String> {
        let Scope::State(state) = scopes.this else {
            return Err("raid_reduce_project_progress_ratio requires THIS state".into());
        };
        let Scope::Raid(reference) = scopes.root else {
            return Err("raid_reduce_project_progress_ratio requires ROOT RAID".into());
        };
        let world = self.context.world()?;
        let raid = world.raid(reference).map_err(|why| why.to_string())?;
        let provinces = world.state_order(state).map_err(|why| why.to_string())?;
        let buildings = self
            .context
            .content
            .buildings
            .ok_or_else(|| self.content_gap("buildings"))?;
        let rows = self.complete::<tables::ProvinceBuildings>(BUILDING_STATUS)?;
        for province in provinces {
            let province = world
                .provinces()
                .get(province)
                .ok_or("raid state province is not recorded")?;
            let owner = if province.owner == 0 {
                0
            } else {
                world
                    .slot(province.owner)
                    .ok_or("country links lacks raid victim province owner")?
            };
            let enemy = world
                .at_war(raid.parent_slot, owner)
                .map_err(|why| why.to_string())?;
            let mut facility = false;
            let low = u128::from(province.id) << 32;
            let mut active = Vec::new();
            for row in each(rows.span(low, low | u128::from(u32::MAX))) {
                let row = row?;
                if row.active {
                    active.push(row);
                }
            }
            active.sort_by_key(|row| row.ordinal);
            for row in &active {
                let token = world
                    .token(row.building_type)
                    .map_err(|why| why.to_string())?
                    .ok_or("lexical tokens lacks raid facility building type")?;
                let token = std::str::from_utf8(token)
                    .map_err(|_| "raid facility building type is not UTF-8")?;
                let definition = buildings
                    .by_token(token)
                    .map_err(|why| why.to_string())?
                    .ok_or("raid facility building type is absent from content")?;
                if let Some(why) = &definition.tooltip_refused {
                    return Err(why.clone());
                }
                if !definition.specialization.is_empty() {
                    facility = row.status_flags & 1 != 0;
                    break;
                }
            }
            if !enemy || !facility {
                continue;
            }
            let programs = self.complete::<tables::RaidFacilities>(FACILITY)?;
            let mut program: Option<Cow<'_, RaidFacilityRow>> = None;
            for row in programs.keyed(|key| world.slot((key >> 32) as u32) == Some(owner)) {
                let row = row.map_err(built)?;
                if row.province == province.id
                    && program.as_ref().is_none_or(|first| row.ordinal < first.ordinal)
                {
                    program = Some(row);
                }
            }
            let Some(project) =
                program.and_then(|row| row.active_project.map(|id| (row.owner, id)))
            else {
                return Ok(None);
            };
            return world
                .project(project.0, project.1)
                .map(Some)
                .map_err(|why| why.to_string());
        }
        Ok(None)
    }

    fn technology_template(
        &self,
        operand: &[u8],
        scopes: Scopes,
    ) -> Result<Option<&content_reader_military::technology::Definition>, String> {
        let catalog = self.technology_catalog()?;
        if let Some(template) = catalog.get(operand)? {
            return Ok(Some(template));
        }
        if !operand.starts_with(b"var:") {
            return Ok(None);
        }
        let handle = (self.scoped_value(operand, scopes)? / 100_000) as u32;
        let rows = self.complete::<tables::TechnologyDefinitions>(TECHNOLOGY)?;
        match find(rows, |row| row.technology_handle == handle)? {
            Some(row) => catalog.get(&row.id),
            None => Ok(None),
        }
    }

    fn technology_country_name(
        &self,
        key: &[u8],
        country: u32,
        original: bool,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut tag = self.tag(country)?.as_bytes().to_vec();
        if original && country > 0 {
            let world = self.context.world()?;
            let appearance = self
                .complete::<tables::CountryAppearance>(NAME_TAG)?
                .keyed(|key| world.slot(key as u32) == Some(country))
                .next()
                .transpose()
                .map_err(built)?
                .ok_or("country appearance lacks technology owner")?;
            if !appearance.name_tag.is_empty() {
                tag = appearance.name_tag.clone();
            }
        }
        let mut candidate = tag;
        candidate.push(b'_');
        candidate.extend_from_slice(key);
        let candidate_defined = std::str::from_utf8(&candidate)
            .ok()
            .is_some_and(|candidate| self.context.localisation.get(candidate).is_some());
        self.localised(
            if candidate_defined { &candidate } else { key },
            &[],
            scopes,
            false,
        )
    }

    fn technology_name(
        &self,
        template: &content_reader_military::technology::Definition,
        country: u32,
        scopes: Scopes,
        runtime: bool,
    ) -> Result<String, String> {
        let catalog = self.technology_catalog()?;
        let equipment = if runtime && !template.small_layout {
            self.technology_enabled_equipment(template)?
        } else {
            Vec::new()
        };
        let key = if equipment.len() == 1 {
            catalog.equipment.equipment[equipment[0]].name.as_slice()
        } else if !template.parent.is_empty() {
            &template.parent
        } else {
            &template.id
        };
        self.technology_country_name(key, country, runtime, scopes)
    }


    pub(super) fn set_technology(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let mut target = item.clone();
        target.value = content_reader_parser::parser::Value::Scalar(b"THIS".to_vec());
        let country = self.country_target(&target, scopes)?.unwrap_or(0);
        let mut out = String::new();
        for field in children(item)
            .iter()
            .filter(|field| !field.keyword_is("popup"))
        {
            let Some(template) =
                self.technology_template(field.key.as_deref().unwrap_or_default(), scopes)?
            else {
                continue;
            };
            let name = self.technology_name(template, country, scopes, false)?;
            out.push_str(&self.char_native_line(
                b"EFFECT_GIVE_TECHNOLOGY",
                &[("NAME", Argument::Text(&name))],
                scopes,
                false,
            )?);
        }
        Ok(out)
    }

    pub(super) fn technology_bonus(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let technology: Vec<_> = fields
            .iter()
            .filter(|field| field.keyword_is("technology"))
            .collect();
        let mut category = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for entry in fields.iter().filter(|field| field.keyword_is("category")) {
            let token =
                content_reader_parser::tokens::Token::c_str(entry.value.scalar().unwrap_or_default());
            if seen.insert(token) {
                category.push(entry);
            }
        }
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0,
        };
        let mut list = String::new();
        for (index, entry) in technology.iter().enumerate() {
            if let Some(template) =
                self.technology_template(entry.value.scalar().unwrap_or_default(), scopes)?
            {
                list.push_str(&self.technology_name(template, country, scopes, true)?);
                if index + 1 < technology.len() || !category.is_empty() {
                    list.push_str(", ");
                }
            }
        }
        for (index, entry) in category.iter().enumerate() {
            if let Some(name) = self
                .technology_catalog()?
                .category(entry.value.scalar().unwrap_or_default())?
            {
                list.push_str(&self.technology_country_name(name, country, false, scopes)?);
                if index + 1 < category.len() {
                    list.push_str(", ");
                }
            }
        }
        if let Some(index) = list.rfind(", ") {
            let and = self.char_native_text(b"AND", &[], scopes, false)?;
            list.replace_range(index..index + 2, &format!(" {and}"));
        }
        let ahead = field(fields, "ahead_reduction")
            .map(content_reader_script::modifiers::stored)
            .unwrap_or(0);
        let bonus = field(fields, "bonus")
            .map(content_reader_script::modifiers::stored)
            .unwrap_or(0);
        let uses = field(fields, "uses")
            .and_then(|field| field.value.scalar())
            .map(integer)
            .unwrap_or(1);
        if ahead == 0 && bonus == 0 {
            return Ok(String::new());
        }
        let uses_text = self.char_native_text(
            b"EFFECT_TECH_BONUS_USES",
            &[("USES", Argument::Int32(uses))],
            scopes,
            false,
        )?;
        if ahead == 0 {
            self.char_native_line(
                b"EFFECT_ADD_TECH_BONUS_BONUS_ONLY",
                &[
                    ("LIST", Argument::Text(&list)),
                    ("BONUS", Argument::Fixed5(bonus)),
                    ("USES", Argument::Text(&uses_text)),
                ],
                scopes,
                false,
            )
        } else if bonus == 0 {
            self.char_native_line(
                b"EFFECT_ADD_TECH_BONUS_AHEAD_ONLY",
                &[
                    ("LIST", Argument::Text(&list)),
                    ("AHEAD", Argument::Fixed5(ahead)),
                    ("USES", Argument::Text(&uses_text)),
                ],
                scopes,
                false,
            )
        } else {
            self.char_native_line(
                b"EFFECT_ADD_TECH_BONUS",
                &[
                    ("LIST", Argument::Text(&list)),
                    ("AHEAD", Argument::Fixed5(ahead)),
                    ("BONUS", Argument::Fixed5(bonus)),
                    ("USES", Argument::Int32(uses)),
                ],
                scopes,
                false,
            )
        }
    }

    fn doctrine_name(
        &self,
        kind: u8,
        operand: &[u8],
        scopes: Scopes,
    ) -> Result<Option<String>, String> {
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        database
            .name(kind, operand)?
            .map(|name| self.localised(&name.value, &[], scopes, false))
            .transpose()
    }


    pub(super) fn set_doctrine(
        &self,
        item: &Item,
        scopes: Scopes,
        sub: bool,
    ) -> Result<String, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| {
                field(children(item), "sub_doctrine").and_then(|field| field.value.scalar())
            })
            .unwrap_or_default();
        let Some(name) = self.doctrine_name(u8::from(sub), operand, scopes)? else {
            return Ok(String::new());
        };
        self.char_native_text(
            if sub {
                b"SET_SUB_DOCTRINE_EFFECT"
            } else {
                b"SET_GRAND_DOCTRINE_EFFECT"
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            false,
        )
    }


    pub(super) fn mastery_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
        daily: bool,
    ) -> Result<String, String> {
        let fields = children(item);
        let amount = field(fields, if daily { "amount" } else { "bonus" })
            .map(content_reader_script::modifiers::stored)
            .unwrap_or(0);
        let days = field(fields, "days")
            .and_then(|field| field.value.scalar())
            .map(integer)
            .unwrap_or(0);
        let inner = self.char_native_text(
            if daily {
                b"ADD_DAILY_MASTERY_EFFECT"
            } else {
                b"ADD_MASTERY_BONUS_EFFECT"
            },
            &[
                (
                    if daily { "AMOUNT" } else { "BONUS" },
                    Argument::Fixed5(amount),
                ),
                ("DAYS", Argument::Int32(days)),
            ],
            scopes,
            false,
        )?;
        let mut names = Vec::new();
        for (selector, kind, only, condition) in [
            (
                "folder",
                3,
                &b"TRACK_EFFECT_ONLY_FOLDER"[..],
                &b"TRACK_EFFECT_CONDITION_FOLDER"[..],
            ),
            (
                "grand_doctrine",
                0,
                b"TRACK_EFFECT_ONLY_GRAND_DOCTRINE",
                b"TRACK_EFFECT_CONDITION_GRAND_DOCTRINE",
            ),
            (
                "sub_doctrine",
                1,
                b"TRACK_EFFECT_ONLY_SUBDOCTRINE",
                b"TRACK_EFFECT_CONDITION_SUBDOCTRINE",
            ),
            (
                "track",
                2,
                b"TRACK_EFFECT_ONLY_TRACK",
                b"TRACK_EFFECT_CONDITION_TRACK_TYPE",
            ),
        ] {
            let Some(operand) = field(fields, selector).and_then(|field| field.value.scalar())
            else {
                continue;
            };
            let name = if kind == 3 {
                let database = self
                    .context
                    .content
                    .doctrines
                    .ok_or_else(|| self.content_gap("doctrines"))?;
                database
                    .folder_exact(operand)?
                    .map(|folder| self.localised(&folder.name, &[], scopes, false))
                    .transpose()?
            } else {
                self.doctrine_name(kind, operand, scopes)?
            };
            if let Some(name) = name {
                names.push((only, condition, name));
            }
        }
        let index = field(fields, "track_index")
            .and_then(|field| field.value.scalar())
            .map(integer)
            .unwrap_or(-1);
        if names.is_empty() && index < 0 {
            return self.char_native_text(
                b"TRACK_EFFECT_ALL_TRACKS",
                &[("TRACK_EFFECT_DESC", Argument::Text(&inner))],
                scopes,
                false,
            );
        }
        if names.len() == 1 && index < 0 {
            return self.char_native_text(
                names[0].0,
                &[
                    ("TRACK_EFFECT_DESC", Argument::Text(&inner)),
                    ("NAME", Argument::Text(&names[0].2)),
                ],
                scopes,
                false,
            );
        }
        let mut out = self.char_native_text(
            b"TRACK_EFFECT_COMPLEX_HEADER",
            &[("TRACK_EFFECT_DESC", Argument::Text(&inner))],
            scopes,
            false,
        )?;
        for (_, condition, name) in names {
            let row = self.char_native_text(
                condition,
                &[("NAME", Argument::Text(&name))],
                scopes,
                false,
            )?;
            append_description(&mut out, &row);
        }
        if index >= 0 {
            let row = self.char_native_text(
                b"TRACK_EFFECT_CONDITION_TRACK_INDEX",
                &[("INDEX", Argument::Int32(index))],
                scopes,
                false,
            )?;
            append_description(&mut out, &row);
        }
        Ok(out)
    }

    pub(super) fn research_slots(
        &self,
        item: &Item,
        scopes: Scopes,
        set: bool,
    ) -> Result<String, String> {
        let count = (self.scoped_value(scalar_operand(item)?, scopes)? / 100_000) as i32;
        if set {
            return self.char_native_text(
                b"EFFECT_SET_RESEARCH_SLOTS",
                &[("COUNT", Argument::Int32(count))],
                scopes,
                false,
            );
        }
        let (key, count) = if count > 0 {
            (&b"EFFECT_ADD_RESEARCH_SLOT"[..], count)
        } else {
            (&b"EFFECT_REMOVE_RESEARCH_SLOT"[..], count.wrapping_neg())
        };
        self.char_native_line(key, &[("COUNT", Argument::Int32(count))], scopes, false)
    }


    pub(super) fn inherit_technology(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some(country) = self
            .country_target(item, scopes)?
            .filter(|country| *country > 0)
        else {
            return Ok(String::new());
        };
        let name = self.country_name(country, false)?;
        self.char_native_text(
            b"EFFECT_INHERIT_TECHNOLOGY",
            &[("COUNTRY", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    pub(super) fn mio_research_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
        set: bool,
    ) -> Result<String, String> {
        let value = self.scoped_value(scalar_operand(item)?, scopes)?;
        self.char_native_line(
            if set {
                b"EFFECT_MIO_SET_RESEARCH_BONUS"
            } else {
                b"EFFECT_MIO_ADD_RESEARCH_BONUS"
            },
            &[("VAL", Argument::Fixed5(value))],
            scopes,
            true,
        )
    }


    pub(super) fn steal_technology_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let instant = field(children(item), "instant")
            .is_some_and(|value| value.value_keyword().as_deref() == Some("yes"));
        self.char_native_text(
            if instant {
                b"EFFECT_STEAL_RANDOM_TECH_INSTANT"
            } else {
                b"EFFECT_STEAL_RANDOM_TECH"
            },
            &[],
            scopes,
            true,
        )
    }


    pub(super) fn faction_research_unlocked(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = self.char_native_text(b"FACTION_ACCESS_TAB_REASEARCH", &[], scopes, false)?;
        self.char_native_text(
            if content_reader_parser::parser::c_string(scalar_operand(item)?) == b"yes" {
                b"FACTION_ACCESS_TAB_UNLOCK"
            } else {
                b"FACTION_ACCESS_TAB_LOCK"
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    fn folder_name(&self, value: &[u8]) -> Result<String, String> {
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database.folder(value)?.ok_or_else(|| {
            format!(
                "doctrine folder {} is absent",
                String::from_utf8_lossy(value)
            )
        })?;
        self.context
            .localisation
            .localise(&folder.name)
            .map_err(|why| why.to_string())
    }

    pub(super) fn mastery(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        for field in fields {
            if !matches!(
                key(field).to_ascii_lowercase().as_str(),
                "amount" | "folder"
            ) {
                return Err(format!(
                    "mastery filter {} is outside reached folder form",
                    key(field)
                ));
            }
        }
        let amount = field(fields, "amount")
            .map(content_reader_script::modifiers::stored)
            .unwrap_or(0);
        let folder = field(fields, "folder")
            .and_then(|item| item.value.scalar())
            .ok_or("mastery folder form absent")?;
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database
            .folder_exact(folder)?
            .ok_or("mastery folder is absent")?;
        let name = self
            .context
            .localisation
            .localise(&folder.name)
            .map_err(|why| why.to_string())?;
        let effect = self.localised(
            b"ADD_MASTERY_EFFECT",
            &[("AMOUNT", Argument::Fixed5(amount))],
            scopes,
            false,
        )?;
        self.localised(
            b"TRACK_EFFECT_ONLY_FOLDER",
            &[
                ("TRACK_EFFECT_DESC", Argument::Text(&effect)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            false,
        )
    }

    pub(super) fn doctrine_reduction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        if let Some(technology) = field(fields, "technology") {
            return Err(format!(
                "technology {} doctrine reduction description is unavailable",
                String::from_utf8_lossy(technology.value.scalar().unwrap_or_default())
            ));
        }
        let reduction = field(fields, "cost_reduction")
            .map(content_reader_script::modifiers::stored)
            .unwrap_or(0);
        let uses = field(fields, "uses")
            .and_then(|item| item.value.scalar())
            .map(content_reader_parser::fixed5::decimal_i32)
            .unwrap_or(1);
        let mut folders = Vec::new();
        for item in fields.iter().filter(|item| item.keyword_is("category")) {
            folders.push(self.folder_name(item.value.scalar().unwrap_or_default())?);
        }
        if folders.is_empty() {
            return Err("non-folder doctrine reduction is outside reached form".into());
        }
        let list = match folders.as_slice() {
            [one] => one.clone(),
            _ => {
                let last = folders.pop().unwrap();
                format!(
                    "{} {} {last}",
                    folders.join(", "),
                    self.context
                        .localisation
                        .localise(b"AND")
                        .map_err(|why| why.to_string())?
                )
            }
        };
        let uses = self.localised(
            b"EFFECT_TECH_BONUS_USES",
            &[("USES", Argument::Int32(uses))],
            scopes,
            false,
        )?;
        self.line(
            b"EFFECT_ADD_DOCTRINE_COST_REDUCTION",
            &[
                ("LIST", Argument::Text(&list)),
                ("REDUCTION", Argument::Fixed5(reduction)),
                ("USES", Argument::Text(&uses)),
            ],
            scopes,
            false,
        )
    }
}
