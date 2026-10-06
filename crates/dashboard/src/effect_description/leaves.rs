use super::*;
use crate::{
    scripted_text::{EventObject, EventScope, Link},
    triggers::{NULL_STATE, Scope},
};
use replay::rows::tables;

impl Renderer<'_, '_, '_> {
    pub(super) fn tag(&self, country: u32) -> Result<&str, String> {
        self.context
            .text
            .recorded()
            .tag(country)
            .ok_or_else(|| format!("Countries lacks tag for country {country}"))
    }

    pub(super) fn country_name(&self, country: u32, definite: bool) -> Result<String, String> {
        self.context.text.country_name(country, definite)
    }

    fn current_country(&self, scopes: Scopes) -> Result<u32, String> {
        match scopes.this {
            Scope::Country(country) => Ok(country),
            _ => Err("current scope has no country (1405336C0)".into()),
        }
    }

    fn country_target(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| field(children(item), "target").and_then(|field| field.value.scalar()));
        match operand {
            Some(value) => self.evaluator()?.scoped_country(value, scopes),
            None => Ok(match scopes.this {
                Scope::Country(country) => Some(country),
                _ => None,
            }),
        }
    }

    fn state_target(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        let state = match item.value.scalar() {
            Some(value) => self.evaluator()?.scoped_state_id(value, scopes)? as u32,
            None => 0,
        };
        if state == 0 {
            Ok(match scopes.this {
                Scope::State(state) => state,
                _ => NULL_STATE,
            })
        } else {
            Ok(state)
        }
    }

    pub(super) fn line(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let mut text = self.localised(key, args, scopes, expand)?;
        text.push('\n');
        Ok(text)
    }

    pub(super) fn leaf(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let name = key(item).to_ascii_lowercase();
        let numeric: Option<&[u8]> = match name.as_str() {
            "add_political_power" => Some(b"POLITICS_ADD_POLITICAL_POWER"), // 140376050
            "army_experience" => Some(b"EFFECT_ADD_ARMY_EXPERIENCE"),       // 140370FF0
            "add_stability" => Some(b"EFFECT_ADD_STABILITY"),               // 140377DF0
            "add_war_support" => Some(b"EFFECT_ADD_WAR_SUPPORT"),           // 1403797B0
            _ => None,
        };
        if let Some(key) = numeric {
            let value = self
                .evaluator()?
                .scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
            return self.line(key, &[("VAL", Argument::Fixed5(value))], scopes, true);
        }
        match name.as_str() {
            // All use the empty description virtual 140176EE0; setters do not run.
            "clear_variable" | "set_country_flag" | "clr_country_flag" | "clr_state_flag"
            | "effect_tooltip" | "hidden_effect" | "if" | "else" | "else_if" | "every_state" => {
                Ok(String::new())
            }
            "save_event_target_as" => {
                // 14038D670 -> 14052D4F0
                self.evaluator()?
                    .save_tooltip_target(item.value.read_string().unwrap_or_default(), scopes)?;
                Ok(String::new())
            }
            "custom_effect_tooltip" => {
                // 1413832B0
                let key = item.value.read_string().ok_or(
                    "custom_effect_tooltip bound localisation block is not ported (1413832B0)",
                )?;
                if key.contains(&b'|') {
                    return Err(format!(
                        "custom_effect_tooltip {} bound formatter is not ported (1413832B0)",
                        String::from_utf8_lossy(key)
                    ));
                }
                self.localised(key, &[], scopes, true)
            }
            "add_state_claim" | "transfer_state" => {
                // 140378010 / 140398680
                let states = self
                    .context
                    .text
                    .frame()
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?;
                let state = self.state_target(item, scopes)?;
                let label = if states.iter().any(|row| row.id == state) {
                    self.context.text.state_name(state)?
                } else {
                    String::new()
                };
                self.line(
                    if name == "add_state_claim" {
                        b"EFFECT_ADD_STATE_CLAIM"
                    } else {
                        b"EFFECT_TRANSFER_STATE"
                    },
                    &[("STATE", Argument::Text(&label))],
                    scopes,
                    true,
                )
            }
            "declare_war_on" | "annex_country" => {
                // 14037E9B0 / 1403799D0
                let Some(target) = self.country_target(item, scopes)? else {
                    return Ok(String::new());
                };
                let source = self.country_name(self.current_country(scopes)?, false)?;
                let target = self.country_name(target, false)?;
                self.line(
                    if name == "annex_country" {
                        b"EFFECT_ANNEX_COUNTRY"
                    } else {
                        b"EFFECT_DECLARE_WAR"
                    },
                    &[
                        ("NAME", Argument::Text(&source)),
                        (
                            if name == "annex_country" {
                                "TARGET"
                            } else {
                                "COUNTRY"
                            },
                            Argument::Text(&target),
                        ),
                    ],
                    scopes,
                    true,
                )
            }
            "start_border_war" => {
                let states = self
                    .context
                    .text
                    .frame()
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?;
                let mut labels = Vec::new();
                for side in ["attacker", "defender"] {
                    let Some(side) = field(children(item), side) else {
                        return Ok(String::new());
                    };
                    let Some(state) = field(children(side), "state") else {
                        return Ok(String::new());
                    };
                    let state = self
                        .evaluator()?
                        .scoped_state_id(state.value.scalar().unwrap_or_default(), scopes)?
                        as u32;
                    if !states.iter().any(|row| row.id == state) {
                        return Ok(String::new());
                    }
                    labels.push(self.context.text.state_name(state)?);
                }
                self.line(
                    b"EFFECT_START_BORDER_WAR",
                    &[
                        ("STATE1", Argument::Text(&labels[0])),
                        ("STATE2", Argument::Text(&labels[1])),
                    ],
                    scopes,
                    true,
                ) // 140395AE0
            }
            "add_to_faction" => self.faction(item, scopes),
            "add_mastery" => self.mastery(item, scopes),
            "add_doctrine_cost_reduction" => self.doctrine_reduction(item, scopes),
            "add_timed_idea" => self.timed_idea(item, scopes),
            "remove_targeted_decision" => self.removed_decision(item, scopes),
            "country_event" => self.country_event(item, scopes),
            "delete_unit_template_and_units" => {
                let fields = children(item);
                let value = item
                    .value
                    .scalar()
                    .or_else(|| {
                        field(fields, "division_template").and_then(|item| item.value.read_string())
                    })
                    .unwrap_or_default();
                let label = std::str::from_utf8(installed_content::parser::c_string(value))
                    .map_err(|_| "template name is not UTF-8")?;
                let disband = installed_content::parser::read_bool_field(fields, "disband", false);
                self.line(
                    if disband {
                        b"EFFECT_DISBAND_UNIT_TEMPLATE_AND_UNITS"
                    } else {
                        b"EFFECT_DELETE_UNIT_TEMPLATE_AND_UNITS"
                    },
                    &[("NAME", Argument::Text(label))],
                    scopes,
                    true,
                ) // 14037EC90
            }
            _ if self.is_context(item) => self
                .context_header(item, scopes)
                .map(|selected| selected.map(|(_, header)| header).unwrap_or_default()),
            _ => Err(format!(
                "{name} description is outside the vanilla-reached subset (CEffect::BuildDescription 140532780)"
            )),
        }
    }

    fn faction_name(&self, faction: u64) -> Result<String, String> {
        let row = self
            .context
            .world()?
            .frame()
            .complete::<tables::Factions>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.id == faction)
            .ok_or_else(|| format!("Factions lacks {faction}"))?;
        let name = crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            &row.name,
            "faction name (140D77360)",
        )?;
        if !name.is_empty() {
            return Ok(name);
        }
        let token = row
            .template_token
            .ok_or("Factions has no name or template_token")?;
        let literal = self
            .context
            .world()?
            .frame()
            .complete::<tables::LexicalTokens>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.token == token)
            .ok_or_else(|| format!("LexicalTokens lacks faction template {token}"))?;
        self.context
            .localisation
            .localise(&literal.literal)
            .map_err(|why| why.to_string())
    }

    // 1404AC450: target first, then the stored ordered subject roster.
    fn faction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let source = self.current_country(scopes)?;
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let world = self.context.world()?;
        let source_faction = world.faction(source)?;
        let target_faction = world.faction(target)?;
        let appearance = world
            .frame()
            .complete::<tables::CountryAppearance>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(target))
            .ok_or("CountryAppearance lacks ordered target subjects")?;
        let subjects = appearance
            .subjects
            .iter()
            .map(|raw| {
                world
                    .recorded()
                    .slot(*raw as u32)
                    .ok_or_else(|| format!("CountryLinks lacks ordered subject {raw}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut body = String::new();
        if appearance.overlord > 0 {
            let master = world
                .recorded()
                .slot(appearance.overlord as u32)
                .ok_or("CountryLinks lacks target overlord")?;
            if source_faction.is_none() || world.faction(master)? != source_faction {
                let puppet = self.country_name(target, false)?;
                let master = self.country_name(master, true)?;
                body.push_str(&self.line(
                    b"EFFECT_REMOVE_FROM_FACTION_END_PUPPET",
                    &[
                        ("PUPPET", Argument::Text(&puppet)),
                        ("MASTER", Argument::Text(&master)),
                    ],
                    scopes,
                    true,
                )?);
            }
        }
        let countries: Vec<_> = std::iter::once(target).chain(subjects).collect();
        if let Some(faction) = target_faction.filter(|faction| Some(*faction) != source_faction) {
            let faction = self.faction_name(faction)?;
            for &country in &countries {
                let country = self.country_name(country, false)?;
                body.push_str(&self.line(
                    b"EFFECT_REMOVE_FROM_FACTION_NAME",
                    &[
                        ("COUNTRY", Argument::Text(&country)),
                        ("FACTION", Argument::Text(&faction)),
                    ],
                    scopes,
                    true,
                )?);
            }
        }
        let join = match source_faction {
            None => Some(None),
            Some(faction) if world.recorded().faction_leader(source)? == Some(source) => {
                Some(Some(self.faction_name(faction)?))
            }
            Some(_) => None,
        };
        if let Some(faction) = join {
            for country in countries {
                let country = self.country_name(country, false)?;
                body.push_str(&match &faction {
                    None => self.line(
                        b"EFFECT_ADD_TO_FACTION",
                        &[("COUNTRY", Argument::Text(&country))],
                        scopes,
                        true,
                    )?,
                    Some(faction) => self.line(
                        b"EFFECT_ADD_TO_FACTION_NAME",
                        &[
                            ("COUNTRY", Argument::Text(&country)),
                            ("FACTION", Argument::Text(faction)),
                        ],
                        scopes,
                        true,
                    )?,
                });
            }
        }
        Ok(body)
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

    // 14051D460 -> 14051E8B0; only the reached folder selector.
    fn mastery(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        for field in fields {
            if !matches!(
                key(field).to_ascii_lowercase().as_str(),
                "amount" | "folder"
            ) {
                return Err(format!(
                    "mastery filter {} is outside reached folder form (14051E8B0)",
                    key(field)
                ));
            }
        }
        let amount = field(fields, "amount")
            .map(installed_content::modifiers::stored)
            .unwrap_or(0);
        let folder = field(fields, "folder")
            .and_then(|item| item.value.scalar())
            .ok_or("mastery folder form absent (14051E8B0)")?;
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database
            .folder_exact(folder)?
            .ok_or("mastery folder is absent (14051D460)")?;
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

    // 14031FCD0: folder display, authored reduction, formatted use count.
    fn doctrine_reduction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        // Native +0x58 technology and +0x70 category lists precede +0x88
        // folders (14031FCD0); never silently omit a technology operand.
        if let Some(technology) = field(fields, "technology") {
            return Err(format!(
                "technology {} doctrine reduction description is not ported (14031FCD0 +0x58)",
                String::from_utf8_lossy(technology.value.scalar().unwrap_or_default())
            ));
        }
        let reduction = field(fields, "cost_reduction")
            .map(installed_content::modifiers::stored)
            .unwrap_or(0);
        let uses = field(fields, "uses")
            .and_then(|item| item.value.scalar())
            .map(installed_content::fixed5::decimal_i32)
            .unwrap_or(1);
        let mut folders = Vec::new();
        for item in fields.iter().filter(|item| item.keyword_is("category")) {
            folders.push(self.folder_name(item.value.scalar().unwrap_or_default())?);
        }
        if folders.is_empty() {
            return Err("non-folder doctrine reduction is outside reached form (14031FCD0)".into());
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
                        .localise(b"AND") // Native literal at 14273BBDC, used by 14225CBD0.
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

    // 14037BDC0: fresh seeded title scope; FROM is the caller's ROOT.
    fn country_event(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
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
            .ok_or("country_event has no id (14037BDC0)")?;
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
            parent.random_seed = parent.random_seed.wrapping_add(1); // 14037BDC0 advances +0x0C.
            title_scope.random_seed = crate::scripted_text::random::scope_seed(seed);
            title_scope.random_counter = crate::scripted_text::random::scope_counter(seed);
            let at = graph.scopes.len();
            graph.scopes.push(title_scope);
            at
        };
        let selected = self.evaluator()?.event_scopes(at)?;
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
}
