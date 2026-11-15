use super::*;
use crate::scripted_text::{EventObject, EventScope, random};
use installed_content_core::ResolvedLayers;
use replay_data::rows::{PoliticalPartyRow, TechSharingGroupRow, TechSharingMemberRow};

const PARTY_TOKEN: u64 = mask::<PoliticalPartyRow>(&["token"]);
const SHARING_COUNT: u64 = mask::<TechSharingGroupRow>(&["member_count"]);
const SHARING_MEMBER: u64 = mask::<TechSharingMemberRow>(&["country"]);

#[cfg(test)]
mod tests;

pub(in crate::effect_description) struct PoliticsContent {
    pub(in crate::effect_description) notes: Vec<String>,
    focuses: Result<installed_content_focus::focus::Focuses, String>,
    tactics: Result<installed_content_core::combat_tactics::Database, String>,
    upgrades: Result<installed_content_core::intelligence::AgencyUpgrades, String>,
    tokens: Result<installed_content_core::intelligence::OperationTokens, String>,
    sharing: Result<installed_content_core::technology_sharing::TechnologySharing, String>,
    pub(super) triggers: Result<installed_content_core::scripted_triggers::ScriptedTriggers, String>,
    pub(super) continents: Result<Vec<String>, String>,
}

impl PoliticsContent {
    pub(in crate::effect_description) fn read(layers: &ResolvedLayers) -> Self {
        let mut notes = Vec::new();
        let focuses = installed_content_focus::focus::read_national(layers)
            .map(|(db, diagnostics)| {
                notes.extend(diagnostics.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        let tactics = installed_content_core::combat_tactics::read(layers)
            .inspect(|db| {
                notes.extend(db.logged.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let upgrades = installed_content_core::intelligence::read_agency_upgrades(layers)
            .inspect(|db| {
                notes.extend(db.warnings.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let tokens = installed_content_core::intelligence::read_operation_tokens(layers)
            .inspect(|db| {
                notes.extend(db.warnings.iter().cloned());
            })
            .map_err(|why| why.to_string());
        let sharing = installed_content_core::technology_sharing::read(layers)
            .inspect(|db| {
                notes.extend(db.warnings.iter().cloned());
            })
            .map_err(|why| why.to_string());
        // Relation titles need bindings stripped from the predicate copy.
        let triggers = installed_content_core::scripted_triggers::read_descriptions(layers)
            .inspect(|db| {
                notes.extend(db.logged.iter().cloned());
            })
            .map_err(|why| why.to_string());
        Self {
            notes,
            focuses,
            tactics,
            upgrades,
            tokens,
            sharing,
            triggers,
            // Continents retain authored name spelling; Environment's map facts have no such catalogue.
            continents: installed_content_core::terrain::read_default_map(layers)
                .and_then(|map| layers.read(&map.file("continent")?))
                .and_then(|bytes| {
                    installed_content_equipment_graphics::equipment_graphics::context::continent_names(
                        &bytes,
                    )
                })
                .map_err(|why| why.to_string()),
        }
    }
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn politics_ideologies(
        &self,
    ) -> Result<&installed_content_core::ideologies::Ideologies, String> {
        self.context.text.content().environment()?.ideologies()
    }

    pub(in crate::effect_description) fn politics_content(
        &self,
    ) -> Result<&PoliticsContent, String> {
        Ok(&self.context.text.content().effects()?.politics)
    }

    // resolve all selectors before evaluating
    // values. Country party roster follows ideology creation order.
    // First authored match wins; absent groups are zero. No divide-by-total.
    // Percent formatting does not multiply by one hundred.
    pub(in crate::effect_description) fn politics_popularities(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let country = self.current_country(scopes)?;
        let (groups, gap) = self.politics_ideologies()?.groups();
        if let Some(gap) = gap {
            return Err(gap.to_string());
        }
        let fields = children(item);
        let selectors = fields
            .iter()
            .map(|item| self.politics_ideology(item.key.as_deref(), scopes))
            .collect::<Result<Vec<_>, _>>()?;
        let parties = self.complete::<tables::PoliticalParties>(PARTY_TOKEN)?;
        let recorded = self.context.text.recorded();
        let ours = parties.keyed(|key| recorded.slot((key >> 32) as u32) == Some(country));
        let tokens = ours
            .map(|party| party.map(|party| party.token.clone()).map_err(built))
            .collect::<Result<Vec<_>, _>>()?;
        let mut text = self.line(b"EFFECT_SET_POPULARITIES", &[], scopes, true)?;
        for group in groups {
            // A missing recorded ideology-party row is a gap,
            // not permission to invent a party the runtime roster did not hold.
            if !tokens
                .iter()
                .any(|token| token.eq_ignore_ascii_case(group.token.as_bytes()))
            {
                return Err(format!("political parties lacks ideology {}", group.token));
            }
            let value = selectors
                .iter()
                .position(|token| token.eq_ignore_ascii_case(group.token.as_bytes()))
                .map(|at| self.scoped_value(fields[at].value.scalar().unwrap_or_default(), scopes))
                .transpose()?
                .unwrap_or(0);
            let name = self.localised(group.token.as_bytes(), &[], scopes, false)?;
            text.push_str(&name);
            text.push_str(": ");
            text.push_str(&installed_content_core::fixed5::fixed5_percent(value, 2, false));
            text.push('\n');
        }
        Ok(text)
    }

    // country or state-owner lookup, used by all three focus describers.
    fn politics_focus_country(&self, scopes: Scopes) -> Result<u32, String> {
        match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => Ok(country),
            Scope::State(state) => self
                .context
                .world()?
                .state_row(state)
                .and_then(|(owner, _)| self.context.text.recorded().slot(owner))
                .ok_or_else(|| format!("States/country links lacks focus-name owner for {state}")),
            _ => Err("focus description scope has no country/state owner".into()),
        }
    }

    // fresh country scope;
    // scope seeding adds Clock seed to byte-exact FNV1a(id)
    // while parsing the token. Reuse the shared hash lanes.
    fn politics_focus_name(
        &self,
        id: &str,
        definition: &installed_content_focus::focus::FocusDefinition,
        country: u32,
    ) -> Result<String, String> {
        let game_seed = self
            .clock_seed()?
            .ok_or("Clock lacks focus-description seed")?;
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
            self.evaluator()?
                .event_scopes(at)
                .map_err(|why| why.to_string())?,
            true,
        )
    }

    // No appended LF. Lookup
    // compares exact ID bytes; continuous focuses cannot match.

    pub(in crate::effect_description) fn politics_focus(
        &self,
        item: &Item,
        scopes: Scopes,
        action: u8,
    ) -> Result<String, String> {
        // Completion accepts scalar or focus; unlock
        // is scalar-only; uncomplete reads block focus.
        let focus = || field(children(item), "focus").and_then(|item| item.value.scalar());
        let id = match action {
            0 => item.value.scalar().or_else(focus),
            1 => item.value.scalar(),
            _ => focus(),
        }
        .unwrap_or_default();
        let id = std::str::from_utf8(installed_content_core::parser::c_string(id))
            .map_err(|_| "national-focus ID is not UTF-8")?;
        let content = self.politics_content()?;
        let definition = content
            .focuses
            .as_ref()
            .map_err(Clone::clone)?
            .get_in(installed_content_focus::focus::Database::National, id)
            .map_err(|gap| format!("national focus {id}: {gap}"))?
            .ok_or_else(|| format!("national focus {id} is absent from recorded content"))?;
        let country = self.politics_focus_country(scopes)?;
        let name = self.politics_focus_name(id, definition, country)?;
        let who = self.country_name(country, false)?;
        if action == 0
            && let Some(tooltip) = field(children(item), "originator_name")
        {
            // The originator name is bound localisation, not a plain string.
            // Scalar keys are supported; blocks need the core bound formatter.
            let tooltip = tooltip
                .value
                .scalar()
                .ok_or("originator_name bound localisation block requires core formatter")?;
            let tooltip = self.localised(tooltip, &[], scopes, true)?;
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
            0 => b"UNLOCK_COMPLETE_EFFECT".as_slice(),
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
        )
    }

    // template name is scoped;
    // outer three-argument localisation is plain, then explicit LF.

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

    // Parse the template token and match it against
    // the recorded group template token.
    // TechSharingGroups members retain the
    // recorded freshness. The group name expands in the caller.

    pub(in crate::effect_description) fn politics_sharing_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let (Scope::Country(country) | Scope::CountryLeader { country, .. }) = scopes.this else {
            return empty::describe();
        };
        if country == 0 {
            return empty::describe();
        }
        let fields = children(item);
        let Some(id) = field(fields, "id").and_then(|item| item.value.scalar()) else {
            return empty::describe();
        };
        let world = self.context.world()?;
        let Some(token) = world
            .token_of(installed_content_core::parser::c_string(id))
            .map_err(|why| why.to_string())?
        else {
            return empty::describe();
        };
        if token == 0x165 {
            // Native unset token.
            return empty::describe();
        }
        let groups = self.complete::<tables::TechSharingGroups>(SHARING_COUNT)?;
        let Some(group) = keyed(&groups, u128::from(token))? else {
            return empty::describe();
        };
        if group.member_count == 0 {
            return empty::describe();
        }
        let members = self.complete::<tables::TechSharingMembers>(SHARING_MEMBER)?;
        let low = u128::from(token) << 32;
        if find(members.span(low, low | u128::from(u32::MAX)), |member| {
            self.context.text.recorded().slot(member.country) == Some(country)
        })?
        .is_none()
        {
            return empty::describe();
        }
        let id = std::str::from_utf8(installed_content_core::parser::c_string(id))
            .map_err(|_| "technology-sharing ID is not UTF-8")?;
        let group = self
            .politics_content()?
            .sharing
            .as_ref()
            .map_err(Clone::clone)?
            .group(id)?
            .ok_or_else(|| {
                format!("technology-sharing group {id} is absent from recorded content")
            })?;
        let name = self.localised(group.name.as_bytes(), &[], scopes, true)?;
        let country = self.country_name(country, false)?;
        // fixed-point parsing, not a scoped variable.
        let bonus = field(fields, "bonus")
            .and_then(|item| item.value.scalar())
            .and_then(installed_content_core::fixed5::script_fixed5)
            .unwrap_or(0);
        self.line(
            b"EFFECT_MODIFY_COUNTRY_TECH_SHARING_BONUS",
            &[
                ("COUNTRY", Argument::Text(&country)),
                ("GROUPNAME", Argument::Text(&name)),
                ("BONUS", Argument::Fixed5(bonus)),
            ],
            scopes,
            false, // plain outer localisation, then explicit LF.
        )
    }

    // A defined upgrade with a nonempty identifier supplies UPGRADE
    // in EFFECT_UPGRADE_INTELLIGENCE_AGENCY, expansion and LF.

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

    // name expands first, text_icon
    // emits control 0x13. Marked-country helper is the shared
    // scripted-text path for a capitalized definite name.

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
            .transpose()
            .map_err(|why| why.to_string())?
            .flatten()
        else {
            return Err("operation-token target is invalid".into());
        };
        let token = field(fields, "token")
            .and_then(|item| item.value.scalar())
            .unwrap_or_default();
        let token = std::str::from_utf8(installed_content_core::parser::c_string(token))
            .map_err(|_| "operation-token ID is not UTF-8")?;
        let content = self.politics_content()?;
        let token = content
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .token(token)?
            .ok_or_else(|| format!("operation token {token} is absent from recorded content"))?;
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
            self.evaluator()?
                .event_scopes(at)
                .map_err(|why| why.to_string())?,
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

    // exact tactic bytes,
    // plain localisation and no LF.
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
                    "combat tactic {} is absent from recorded content",
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
