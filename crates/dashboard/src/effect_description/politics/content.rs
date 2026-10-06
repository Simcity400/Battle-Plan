use super::*;
use crate::scripted_text::{EventObject, EventScope, random};
use installed_content::ResolvedLayers;

#[cfg(test)]
mod tests;

pub(in crate::effect_description) struct PoliticsContent {
    pub(in crate::effect_description) notes: Vec<String>,
    ideologies: Result<installed_content::ideologies::Ideologies, String>,
    focuses: Result<installed_content::focus::Focuses, String>,
    tactics: Result<installed_content::combat_tactics::Database, String>,
    upgrades: Result<installed_content::intelligence::AgencyUpgrades, String>,
    tokens: Result<installed_content::intelligence::OperationTokens, String>,
    sharing: Result<installed_content::technology_sharing::TechnologySharing, String>,
    pub(super) triggers: Result<installed_content::scripted_triggers::ScriptedTriggers, String>,
    pub(super) continents: Result<Vec<String>, String>,
}

impl PoliticsContent {
    pub(in crate::effect_description) fn read(layers: &ResolvedLayers) -> Self {
        let mut notes = Vec::new();
        let ideologies = installed_content::ideologies::read(layers)
            .map(|(db, diagnostics)| {
                notes.extend(diagnostics.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        let focuses = installed_content::focus::read_national(layers)
            .map(|(db, diagnostics)| {
                notes.extend(diagnostics.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        let tactics = installed_content::combat_tactics::read(layers)
            .inspect(|db| {
                notes.extend(db.logged.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let upgrades = installed_content::intelligence::read_agency_upgrades(layers)
            .inspect(|db| {
                notes.extend(db.warnings.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let tokens = installed_content::intelligence::read_operation_tokens(layers)
            .inspect(|db| {
                notes.extend(db.warnings.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let sharing = installed_content::technology_sharing::read(layers)
            .inspect(|db| {
                notes.extend(db.warnings.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let triggers = installed_content::scripted_triggers::read_descriptions(layers)
            .inspect(|db| {
                notes.extend(db.logged.iter().cloned());
            })
            .map_err(|why| why.to_string());
        Self {
            notes,
            ideologies,
            focuses,
            tactics,
            upgrades,
            tokens,
            sharing,
            triggers,
            continents: installed_content::terrain::read_default_map(layers)
                .and_then(|map| layers.read(&map.file("continent")?))
                .and_then(|bytes| installed_content::equipment_graphics::context::continent_names(&bytes))
                .map_err(|why| why.to_string()),
        }
    }
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn politics_content(
        &self,
    ) -> Result<&PoliticsContent, String> {
        Ok(&self.context.text.content().effects()?.politics)
    }

    // Own 1403929D0 -> 14039CAE0: resolve all selectors before evaluating
    // values. Country party roster follows ideology creation order (140B9F5E0).
    // First authored match wins; absent groups are zero. No divide-by-total.
    // EFFECT_SET_POPULARITIES@142750EC8; 142259590's times_hundred=false.
    pub(in crate::effect_description) fn politics_popularities(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let country = self.current_country(scopes)?;
        let content = self.politics_content()?;
        let (groups, gap) = content.ideologies.as_ref().map_err(Clone::clone)?.groups();
        if let Some(gap) = gap {
            return Err(gap.to_string());
        }
        let fields = children(item);
        let selectors = fields
            .iter()
            .map(|item| self.politics_ideology(item.key.as_deref(), scopes))
            .collect::<Result<Vec<_>, _>>()?;
        let parties = self
            .context
            .text
            .frame()
            .complete::<tables::PoliticalParties>()
            .map_err(|why| why.to_string())?;
        let mut text = self.line(b"EFFECT_SET_POPULARITIES", &[], scopes, true)?;
        for group in groups {
            // FindPartyByIdeology (140B980C0); a missing recorded row is a gap,
            // not permission to invent a party the runtime roster did not hold.
            if !parties.iter().any(|party| {
                self.context.text.recorded().slot(party.country) == Some(country)
                    && party.token.eq_ignore_ascii_case(group.token.as_bytes())
            }) {
                return Err(format!(
                    "PoliticalParties lacks ideology {} (14039CAE0)",
                    group.token
                ));
            }
            let value = selectors
                .iter()
                .position(|token| token.eq_ignore_ascii_case(group.token.as_bytes()))
                .map(|at| self.scoped_value(fields[at].value.scalar().unwrap_or_default(), scopes))
                .transpose()?
                .unwrap_or(0);
            let name = self.localised(group.token.as_bytes(), &[], scopes, false)?; // 140616BC0.
            text.push_str(&name);
            text.push_str(": "); // 142700678.
            text.push_str(&installed_content::fixed5::fixed5_percent(value, 2, false)); // 142259590.
            text.push('\n');
        }
        Ok(text)
    }

    // GetCountryTagOrStateOwner 14052B6A0, used by all three focus describers.
    fn politics_focus_country(&self, scopes: Scopes) -> Result<u32, String> {
        match scopes.this {
            Scope::Country(country) => Ok(country),
            Scope::State(state) => self
                .context
                .world()?
                .state_row(state)
                .and_then(|(owner, _)| self.context.text.recorded().slot(owner))
                .ok_or_else(|| {
                    format!("States/CountryLinks lacks focus-name owner for {state} (14052B6A0)")
                }),
            _ => Err("focus description scope has no country/state owner (14052B6A0)".into()),
        }
    }

    // CNationalFocus::GetLocalizedName 1402D2250: fresh country scope;
    // SeedEventScope 1402DADC0 adds Clock seed to byte-exact FNV1a(id) +0x58
    // (ReadToken 1402D76A0 -> 1402D70E0). Reuse the shared hash lanes.
    fn politics_focus_name(
        &self,
        id: &str,
        definition: &installed_content::focus::FocusDefinition,
        country: u32,
    ) -> Result<String, String> {
        let game_seed = self
            .context
            .text
            .frame()
            .complete::<tables::Clock>()
            .map_err(|why| why.to_string())?
            .first()
            .ok_or("Clock lacks focus-description seed (1402DADC0)")?
            .seed;
        let hash = id.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
        let seed = (game_seed as u32).wrapping_add(hash);
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            let mut scope = EventScope::of(EventObject::Country(country));
            scope.random_seed = random::scope_seed(seed);
            scope.random_counter = random::scope_counter(seed);
            graph.scopes.push(scope);
            at
        };
        self.localised(
            definition.localisation.as_bytes(),
            &[],
            self.evaluator()?.event_scopes(at)?,
            true,
        )
    }

    // Own 14037B7B0 / 14039A170 / 140398CF0. No appended LF. Lookup
    // 1402D1D60 compares exact ID bytes; continuous focuses cannot match.
    // UNLOCK_COMPLETE_EFFECT_FOR@14275A6F0; FOR@14275A6E8, FOCUS@14275A6E0.
    // UNLOCK_FOCUS_EFFECT@14275A5C8 / UNCOMPLETE_FOCUS_EFFECT@14275A820.
    pub(in crate::effect_description) fn politics_focus(
        &self,
        item: &Item,
        scopes: Scopes,
        action: u8,
    ) -> Result<String, String> {
        let id = item
            .value
            .scalar()
            .or_else(|| field(children(item), "focus").and_then(|item| item.value.scalar()))
            .unwrap_or_default();
        let id = std::str::from_utf8(installed_content::parser::c_string(id))
            .map_err(|_| "national-focus ID is not UTF-8 (1402D1D60)")?;
        let content = self.politics_content()?;
        let definition = content
            .focuses
            .as_ref()
            .map_err(Clone::clone)?
            .get(id)
            .ok_or_else(|| {
                format!("national focus {id} is absent from recorded content (1402D1D60)")
            })?;
        let country = self.politics_focus_country(scopes)?;
        let name = self.politics_focus_name(id, definition, country)?;
        let who = self.country_name(country, false)?; // 140BA5960.
        // 1403A9B50/1403B1040 read originator_name at +0x80; 14037B7B0 localises it.
        if action == 0
            && let Some(tooltip) = field(children(item), "originator_name")
                .and_then(|item| item.value.scalar())
                .filter(|value| !value.is_empty())
        {
            let tooltip = self.localised(tooltip, &[], scopes, true)?; // 1402F19A0.
            return self.localised(
                b"UNLOCK_COMPLETE_EFFECT_FOR",
                &[
                    ("WHO", Argument::Text(&tooltip)),
                    ("FOR", Argument::Text(&who)),
                    ("FOCUS", Argument::Text(&name)),
                ],
                scopes,
                true,
            );
        }
        let key = match action {
            0 => b"UNLOCK_COMPLETE_EFFECT".as_slice(), // 14275A6C8.
            1 => b"UNLOCK_FOCUS_EFFECT".as_slice(),
            _ => b"UNCOMPLETE_FOCUS_EFFECT".as_slice(),
        };
        self.localised(
            key,
            &[
                ("WHO", Argument::Text(&who)),
                ("LIST", Argument::Text(&name)),
            ],
            scopes,
            true,
        ) // 14275A5C0 / 14272B3D8.
    }

    // 1403214E0 / 140321A90: template name 1413BCF50 is scoped;
    // outer three-argument localisation is plain, then explicit LF.
    // EFFECT_ADD_COUNTRY_TO_TECH_SHARING_GROUP@14273C1D8;
    // EFFECT_REMOVE_COUNTRY_FROM_TECH_SHARING_GROUP@14273C0D8.
    pub(in crate::effect_description) fn politics_sharing(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let token = item.value.c_str().map_err(|why| why.to_string())?;
        let content = self.politics_content()?;
        let Some(group) = content
            .sharing
            .as_ref()
            .map_err(Clone::clone)?
            .group(token)?
        else {
            return Ok(String::new());
        };
        let name = self.localised(group.name.as_bytes(), &[], scopes, true)?;
        let country = self.current_country(scopes)?;
        let tag = self.tag(country)?;
        let country = self.country_name(country, false)?;
        self.line(
            if remove {
                b"EFFECT_REMOVE_COUNTRY_FROM_TECH_SHARING_GROUP"
            } else {
                b"EFFECT_ADD_COUNTRY_TO_TECH_SHARING_GROUP"
            },
            &[
                ("TAG", Argument::Text(tag)),
                ("COUNTRY", Argument::Text(&country)),
                ("GROUPNAME", Argument::Text(&name)),
            ],
            scopes,
            false,
        )
    }

    // 14039A4E0: nonnull, defined (+0x10 nonempty) upgrade then UPGRADE
    // @142703C38 in EFFECT_UPGRADE_INTELLIGENCE_AGENCY, expansion and LF.
    // EFFECT_UPGRADE_INTELLIGENCE_AGENCY@142763058.
    pub(in crate::effect_description) fn politics_upgrade_agency(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let token = item.value.c_str().map_err(|why| why.to_string())?;
        let content = self.politics_content()?;
        let Some(upgrade) = content
            .upgrades
            .as_ref()
            .map_err(Clone::clone)?
            .upgrade(token)?
        else {
            return Ok(String::new());
        };
        if upgrade.name.is_empty() {
            return Ok(String::new());
        }
        let name = self.localised(upgrade.name.as_bytes(), &[], scopes, false)?;
        self.line(
            b"EFFECT_UPGRADE_INTELLIGENCE_AGENCY",
            &[("UPGRADE", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // 140375370 / 140389F00: +0x50 name expands first, +0x70 text_icon
    // emits control 0x13. Marked-country helper 140BA59E0 is the shared
    // GetNameWithFlag scripted-text path (capitalized definite name).
    // OPERATION_TOKEN_RECEIVE@142761F78 / OPERATION_TOKEN_REMOVE@142762100.
    pub(in crate::effect_description) fn politics_operation_token(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let fields = children(item);
        let Some(country) = field(fields, "tag")
            .map(|item| {
                self.evaluator()?
                    .scoped_country(item.value.scalar().unwrap_or_default(), scopes)
            })
            .transpose()?
            .flatten()
        else {
            return Err("operation-token target is invalid (140536ED0)".into());
        };
        let token = field(fields, "token")
            .and_then(|item| item.value.scalar())
            .unwrap_or_default();
        let token = std::str::from_utf8(installed_content::parser::c_string(token))
            .map_err(|_| "operation-token ID is not UTF-8 (14039D9B0)")?;
        let content = self.politics_content()?;
        let token = content
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .token(token)?
            .ok_or_else(|| {
                format!("operation token {token} is absent from recorded content (14039D9B0)")
            })?;
        let name = self.localised(token.name.as_bytes(), &[], scopes, true)?;
        let name = format!("\u{13}{} {name}", token.icon);
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::Country(country)));
            at
        };
        let country = self.expanded(
            "[THIS.GetNameWithFlag]",
            self.evaluator()?.event_scopes(at)?,
        )?;
        self.line(
            if remove {
                b"OPERATION_TOKEN_REMOVE"
            } else {
                b"OPERATION_TOKEN_RECEIVE"
            },
            &[
                ("TOKEN", Argument::Text(&name)),
                ("COUNTRY", Argument::Text(&country)),
            ],
            scopes,
            true,
        )
    }

    // 14051E260 -> 1406B11F0: exact tactic bytes, NAME@142727E7C,
    // UNLOCK_TACTIC_EFFECT@1427BB730; plain localisation and no LF.
    pub(in crate::effect_description) fn politics_unlock_tactic(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let token = item.value.scalar().unwrap_or_default();
        let content = self.politics_content()?;
        let database = content.tactics.as_ref().map_err(Clone::clone)?;
        if !database.unsupported.is_empty() {
            return Err(database.unsupported.join("; "));
        }
        let tactic = database
            .tactics
            .iter()
            .rev()
            .find(|tactic| tactic.token == token)
            .ok_or_else(|| {
                format!(
                    "combat tactic {} is absent from recorded content (1406B11F0)",
                    String::from_utf8_lossy(token)
                )
            })?;
        if let Some(why) = &tactic.unproven {
            return Err(why.clone());
        }
        let name = self.localised(&tactic.token, &[], scopes, false)?;
        self.localised(
            b"UNLOCK_TACTIC_EFFECT",
            &[("NAME", Argument::Text(&name))],
            scopes,
            false,
        )
    }
}
