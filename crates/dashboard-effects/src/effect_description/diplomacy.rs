use super::*;
use replay_data::rows::{
    AutonomyRow, CountryAppearanceRow, CountryWargoalRow, PoliticalPartyRow, PoliticsRow,
    SimulationFeatureRow, UnitLeaderRow, WargoalTargetRow, tables,
};

const RULING: u64 = mask::<PoliticsRow>(&["ruling_party"]);
const PARTY_TOKEN: u64 = mask::<PoliticalPartyRow>(&["token"]);
const GOAL: u64 = mask::<CountryWargoalRow>(&["handle", "recipient", "goal_type"]);
const TARGET_VALUE: u64 = mask::<WargoalTargetRow>(&["value"]);
const SUBJECTS: u64 = mask::<CountryAppearanceRow>(&["overlord", "subjects"]);
const FEATURES: u64 = mask::<SimulationFeatureRow>(&["dlc_mask"]);
const SCORES: u64 = mask::<AutonomyRow>(&["lower_score", "upper_score"]);
const LEADER_CHARACTER: u64 = mask::<UnitLeaderRow>(&["character_id"]);

pub(super) const CREATE_WARGOAL_UNRECORDED: &str =
    "create_wargoal requires recorded country wargoals and wargoal targets";

impl Renderer<'_, '_, '_> {
    pub(super) fn purchase_contract(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        for (fields, names) in std::iter::once((fields, ["seller", "buyer", "civilian_factories"]))
            .chain(
                fields
                    .iter()
                    .filter(|entry| entry.keyword_is("equipment"))
                    .map(|entry| (children(entry), ["amount", "producer", "variant_name"])),
            )
        {
            for name in names {
                if fields
                    .iter()
                    .filter(|field| field.keyword_is(name))
                    .nth(1)
                    .is_some()
                {
                    return Err(format!(
                        "repeated purchase operand {name} needs retained scoped variable reader flags",
                    ));
                }
            }
        }
        let country = |name| -> Result<u32, String> {
            field(fields, name)
                .map(|item| {
                    self.evaluator()?
                        .scoped_country(scalar_operand(item)?, scopes)
                })
                .transpose()
                .map(|country| country.flatten().unwrap_or(0))
                .map_err(|why| why.to_string())
        };
        let seller_country = country("seller")?;
        let seller = self.context.text.country_marked_name(seller_country)?;
        let buyer = self.context.text.country_marked_name(country("buyer")?)?;
        let factories = field(fields, "civilian_factories")
            .map(|item| self.scoped_value(scalar_operand(item)?, scopes))
            .transpose()?
            .unwrap_or(0);
        let factories = if factories % 100_000 == 0 {
            factories
        } else {
            (factories / 100_000)
                .wrapping_add(factories >> 63)
                .wrapping_mul(100_000)
        };
        let factories = ((factories / 100_000) as i32).to_string();
        let equipment = self.equipment_list_description(
            fields,
            seller_country,
            scopes,
            ", ",
            b"INTERNATIONAL_MARKET_EQUIPMENT_AMOUNT",
        )?;
        self.line(
            b"EFFECT_MARKET_CREATE_PURCHASE_CONTRACT",
            &[
                ("SELLER", Argument::Text(&seller)),
                ("BUYER", Argument::Text(&buyer)),
                ("CIVS", Argument::Text(&factories)),
                ("EQUIPMENT", Argument::Text(&equipment)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn start_civil_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let ideology = self.politics_ideology(
            field(fields, "ideology").map(scalar_operand).transpose()?,
            scopes,
        )?;
        let ideology = self.localised(&ideology, &[], scopes, false)?;
        let mut text = self.line(
            b"EFFECT_START_CIVIL_WAR",
            &[("IDEOLOGY", Argument::Text(&ideology))],
            scopes,
            true,
        )?;
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => 0,
        };
        let ruling = field(fields, "ruling_party")
            .map(|operand| self.politics_ideology(Some(scalar_operand(operand)?), scopes))
            .transpose();
        let ruling = match ruling {
            Ok(Some(ruling)) => ruling,
            Err(why) if why != super::politics::NULL_IDEOLOGY => return Err(why),
            Ok(None) | Err(_) => {
                let recorded = self.context.text.recorded();
                let id = self
                    .complete::<tables::Politics>(RULING)?
                    .keyed(|key| recorded.slot(key as u32) == Some(country))
                    .next()
                    .transpose()
                    .map_err(built)?
                    .and_then(|row| row.ruling_party)
                    .ok_or("Politics lacks civil-war fallback ruling ideology")?;
                let token = self
                    .complete::<tables::PoliticalParties>(PARTY_TOKEN)?
                    .keyed(|key| {
                        recorded.slot((key >> 32) as u32) == Some(country) && key as u32 == id
                    })
                    .next()
                    .transpose()
                    .map_err(built)?
                    .ok_or("political parties lacks civil-war fallback ruling ideology")?;
                self.politics_ideology(Some(&token.token), scopes)?
            }
        };
        text.push_str(&self.line(b"EFFECT_SET_POLITICS", &[], scopes, false)?);
        let party = if country > 0 {
            self.selected_party_name(country, &ruling)?
        } else {
            self.localised(&ruling, &[], scopes, false)?
        };
        text.push_str(&self.line(
            b"EFFECT_SET_POLITICS_RULING_PARTY",
            &[("PARTY", Argument::Text(&party))],
            scopes,
            false,
        )?);
        text.push_str(&self.line(b"EFFECT_SET_POLITICS_NO_ELECTIONS", &[], scopes, false)?);
        Ok(text)
    }

    pub(super) fn war_or_annex(
        &self,
        item: &Item,
        scopes: Scopes,
        annex: bool,
    ) -> Result<String, String> {
        self.diplomacy_pair(
            item,
            scopes,
            if annex {
                b"EFFECT_ANNEX_COUNTRY"
            } else {
                b"EFFECT_DECLARE_WAR"
            },
            if annex { "TARGET" } else { "COUNTRY" },
        )
    }

    pub(super) fn create_wargoal(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let target = self
            .country_target_field(item, "target")?
            .map(|target| self.country_target(target, scopes))
            .transpose()?
            .flatten()
            .unwrap_or(0);
        let fields = children(item);
        let Some(goal) = field(fields, "type").and_then(Item::value_keyword) else {
            return Ok(String::new());
        };
        let cached = self.context.text.content().effects()?;
        let database = self
            .context
            .content
            .wargoals
            .or(cached.wargoals.as_ref())
            .ok_or_else(|| self.content_gap("Wargoal definitions"))?;
        let Some(definition_name) = database.find(goal.as_bytes())? else {
            return Ok(String::new());
        };
        let source = self.current_country(scopes)?;
        if target == 0 || source == 0 {
            return Err("create_wargoal needs reserved country zero diplomacy/name inputs".into());
        }
        let world = self.context.world()?;
        let frame = self.tables();
        let goals = frame
            .complete::<tables::CountryWargoals>(GOAL)
            .map_err(|_| CREATE_WARGOAL_UNRECORDED.to_string())?;
        let targets = frame
            .complete::<tables::WargoalTargets>(TARGET_VALUE)
            .map_err(|_| CREATE_WARGOAL_UNRECORDED.to_string())?;
        let ambiguous_definition = database
            .names
            .iter()
            .filter(|name| {
                content_reader_parser::tokens::Token::c_str(name)
                    == content_reader_parser::tokens::Token::c_str(goal.as_bytes())
            })
            .count()
            > 1;
        let requested = field(fields, "generator")
            .map(children)
            .unwrap_or_default()
            .iter()
            .map(|state| scalar_operand(state).map(integer))
            .collect::<Result<Vec<_>, _>>()?;
        for member in targets.keyed(|key| {
            (key >> 64) as u8 == 3 && world.recorded().slot((key >> 72) as u32) == Some(source)
        }) {
            let member = member.map_err(built)?;
            if member.value == 0 {
                return Err("create_wargoal unused goal reference is null".into());
            }
            let low = u128::from(member.country) << 32;
            let owned = find(goals.span(low, low | u128::from(u32::MAX)), |row| {
                row.handle == member.value
            })?
            .ok_or("create_wargoal unused goal object is not recorded")?;
            let recipient = world
                .recorded()
                .slot(owned.recipient)
                .ok_or("create_wargoal goal recipient is unresolved")?;
            let matches_target = if recipient == target {
                true
            } else {
                let target_row = frame
                    .complete::<tables::CountryAppearance>(SUBJECTS)
                    .map_err(|why| why.to_string())?
                    .keyed(|key| world.recorded().slot(key as u32) == Some(target))
                    .next()
                    .transpose()
                    .map_err(built)?
                    .ok_or("create_wargoal target subjects are not recorded")?;
                let mut found = false;
                for &raw in target_row.subjects.require("subjects of country appearance")? {
                    if world
                        .recorded()
                        .slot(raw as u32)
                        .ok_or("create_wargoal target subject is unresolved")?
                        == recipient
                    {
                        found = true;
                        break;
                    }
                }
                found
            };
            if !matches_target {
                continue;
            }
            let spelling = world
                .token(owned.goal_type)
                .map_err(|why| why.to_string())?
                .ok_or("create_wargoal goal type spelling is not recorded")?;
            if content_reader_parser::tokens::Token::c_str(spelling)
                != content_reader_parser::tokens::Token::c_str(goal.as_bytes())
            {
                continue;
            }
            let low = u128::from(owned.country) << 72 | u128::from(owned.ordinal) << 32;
            let states = each(targets.span(low, low | u128::from(u32::MAX)))
                .collect::<Result<Vec<_>, _>>()?;
            let covers = if states.is_empty() || requested.is_empty() {
                true
            } else {
                let recorded_states = frame
                    .complete::<tables::States>(0)
                    .map_err(|why| why.to_string())?;
                requested.iter().all(|id| {
                    let resolved =
                        if *id > 0 && recorded_states.contains(u128::from(*id as u32)) {
                            *id as u64
                        } else {
                            0
                        };
                    states.iter().any(|row| row.value == resolved)
                })
            };
            if covers {
                if ambiguous_definition {
                    return Err("create_wargoal war-goal definition occurrence is ambiguous".into());
                }
                return Ok(String::new());
            }
        }
        let country = self.country_name(source, false)?;
        let target = self.country_name(target, false)?;
        let name = self.localised(definition_name, &[], scopes, false)?;
        self.line(
            b"EFFECT_CREATE_WARGOAL",
            &[
                ("COUNTRY", Argument::Text(&country)),
                ("WARGOAL", Argument::Text(&name)),
                ("TARGET", Argument::Text(&target)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn faction_influence(
        &self,
        item: &Item,
        scopes: Scopes,
        ratio: bool,
    ) -> Result<String, String> {
        let country = self.current_country(scopes)?;
        let faction = if country > 0 {
            self.context
                .world()?
                .faction(country)
                .map_err(|why| why.to_string())?
        } else {
            None
        };
        let value = if scope_selector(item).is_some() {
            0
        } else {
            self.scoped_value(scalar_operand(item)?, scopes)?
        };
        let amount = self.localised(
            if ratio {
                b"FACTION_INFLUENCE_RATIO"
            } else {
                b"FACTION_INFLUENCE_VALUE"
            },
            &[("VAL", Argument::Fixed5(value))],
            scopes,
            false,
        )?;
        let country = self.country_name(country, false)?;
        let text = if let Some(faction) = faction {
            let name = self.context.text.faction_name_by_id(faction)?;
            self.localised(
                b"ADD_FACTION_INFLUENCE_EFFECT",
                &[
                    ("VAL", Argument::Text(&amount)),
                    ("FACTION", Argument::Text(&name)),
                    ("COUNTRY", Argument::Text(&country)),
                ],
                scopes,
                false,
            )?
        } else {
            self.localised(
                b"ADD_FACTION_INFLUENCE_EFFECT_NO_FACTION",
                &[
                    ("VAL", Argument::Text(&amount)),
                    ("COUNTRY", Argument::Text(&country)),
                ],
                scopes,
                false,
            )?
        };
        self.expanded_with_faction_environment(&text, scopes, Some(faction))
    }

    pub(super) fn faction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let source = self.current_country(scopes)?;
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let world = self.context.world()?;
        let source_faction = world.faction(source).map_err(|why| why.to_string())?;
        let target_faction = world.faction(target).map_err(|why| why.to_string())?;
        let appearance = world
            .tables()
            .complete::<tables::CountryAppearance>(SUBJECTS)
            .map_err(|why| why.to_string())?
            .keyed(|key| world.recorded().slot(key as u32) == Some(target))
            .next()
            .transpose()
            .map_err(built)?
            .ok_or("country appearance lacks ordered target subjects")?;
        let subject = |raw: i32| {
            world
                .recorded()
                .slot(raw as u32)
                .ok_or_else(|| format!("country links lacks ordered subject {raw}"))
        };
        let mut body = String::new();
        if appearance.overlord > 0 {
            let master = world
                .recorded()
                .slot(appearance.overlord as u32)
                .ok_or("country links lacks target overlord")?;
            if source_faction.is_none()
                || world.faction(master).map_err(|why| why.to_string())? != source_faction
            {
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
        if let Some(faction) = target_faction.filter(|faction| Some(*faction) != source_faction) {
            let faction = self.context.text.faction_name_by_id(faction)?;
            let line = |country| -> Result<String, String> {
                let country = self.country_name(country, false)?;
                self.line(
                    b"EFFECT_REMOVE_FROM_FACTION_NAME",
                    &[
                        ("COUNTRY", Argument::Text(&country)),
                        ("FACTION", Argument::Text(&faction)),
                    ],
                    scopes,
                    true,
                )
            };
            body.push_str(&line(target)?);
            for &raw in appearance.subjects.require("subjects of country appearance")? {
                body.push_str(&line(subject(raw)?)?);
            }
        }
        let join = match source_faction {
            None => Some(None),
            Some(faction)
                if world
                    .recorded()
                    .faction_leader(source)
                    .map_err(|why| why.to_string())?
                    == Some(source) =>
            {
                Some(Some(self.context.text.faction_name_by_id(faction)?))
            }
            Some(_) => None,
        };
        if let Some(faction) = join {
            let line = |country| -> Result<String, String> {
                let country = self.country_name(country, false)?;
                match &faction {
                    None => self.line(
                        b"EFFECT_ADD_TO_FACTION",
                        &[("COUNTRY", Argument::Text(&country))],
                        scopes,
                        true,
                    ),
                    Some(faction) => self.line(
                        b"EFFECT_ADD_TO_FACTION_NAME",
                        &[
                            ("COUNTRY", Argument::Text(&country)),
                            ("FACTION", Argument::Text(faction)),
                        ],
                        scopes,
                        true,
                    ),
                }
            };
            body.push_str(&line(target)?);
            for &raw in appearance.subjects.require("subjects of country appearance")? {
                body.push_str(&line(subject(raw)?)?);
            }
        }
        Ok(body)
    }

    pub(super) fn diplomacy_country(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
        definite: bool,
    ) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let name = self.country_name(target, definite)?;
        self.line(key, &[("COUNTRY", Argument::Text(&name))], scopes, true)
    }

    pub(super) fn diplomacy_pair(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
        target_arg: &str,
    ) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let source = self.country_name(self.current_country(scopes)?, false)?;
        let target = self.country_name(target, false)?;
        self.line(
            key,
            &[
                ("NAME", Argument::Text(&source)),
                (target_arg, Argument::Text(&target)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn diplomatic_relation(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        if !matches!(scopes.this, Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0)
        {
            return Ok(String::new());
        }
        let Some(country) = self.country_target_field(item, "country")? else {
            return Ok(String::new());
        };
        let Some(target) = self.country_target(country, scopes)? else {
            return Ok(String::new());
        };
        let relation = field(children(item), "relation").and_then(Item::value_keyword);
        let label: &[u8] = match relation.as_deref() {
            Some("naval_blockade") => b"DIPLOMACY_NAVAL_BLOCKADE_TITLE",
            Some("air_base_access" | "offer_air_base_access") => b"DIPLOMACY_AIR_BASE_ACCESS",
            Some("guarantee") => b"DIPLOMACY_GUARANTEE_TITLE",
            Some("military_access" | "offer_military_access") => b"DIPLOMACY_MILACC",
            Some("improve_relation") => b"DIPLOMACY_IMPROVERELATION",
            Some("non_aggression_pact") => b"DIPLOMACY_NONAGGRESSIONPACT",
            Some("embargo") => b"DIPLOMACY_EMBARGO_TITLE",
            Some("market_access_rights") => b"DIPLOMACY_INTERNATIONAL_MARKET_ACCESS_RIGHTS",
            Some("war_relation") => b"DIPLOMACY_ATWAR",
            Some("send_attache") => b"DIPLOMACY_SEND_ATTACHE",
            Some("docking_rights" | "offer_docking_rights") => b"DIPLOMACY_DOCKING_RIGHTS",
            Some("licence_production_access") => b"DIPLOMACY_LICACC",
            Some("puppet" | "lend_lease" | "equipment_purchase_contract_relation") => b"",
            Some("boost_party_popularity") => return Err("temporary boost-party relation needs the null ideology-group identifier; shared content does not expose that null-object identity".into()),
            _ => return Ok("BROKEN DIPLOMATIC RELATION EFFECT".into()),
        };
        let name = self.localised(label, &[], scopes, false)?;
        let country = self.country_name(target, true)?;
        let active = content_reader_parser::parser::read_bool_field(children(item), "active", true);
        self.line(
            if active {
                b"EFFECT_ENACT_RELATION"
            } else {
                b"EFFECT_CANCEL_RELATION"
            },
            &[
                ("NAME", Argument::Text(&name)),
                ("WHO", Argument::Text(&country)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn add_to_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let mut names = Vec::new();
        for name in ["enemy", "targeted_alliance"] {
            let Some(operand) = self.country_target_field(item, name)? else {
                names.push(self.country_name(0, false)?);
                continue;
            };
            let value = content_reader_parser::parser::c_string(scalar_operand(operand)?);
            if scope_selector(operand).is_some_and(|selector| {
                !matches!(
                    selector,
                    Selected::Keyword(Selector::Root | Selector::From | Selector::Prev)
                )
            }) {
                return Err(format!(
                    "{name} form {} is a country specifier rather than a ROOT/FROM/PREV selector; its country-tag reader is unavailable",
                    String::from_utf8_lossy(value)
                ));
            }
            let country = if value.contains(&b'.') || scope_selector(operand).is_some() {
                let mut terminal = Keyword::Root;
                for link in value.split(|byte| *byte == b'.') {
                    terminal = match Keyword::named(link) {
                        Some(link @ (Keyword::Root | Keyword::From | Keyword::Prev)) => link,
                        _ => {
                            return Err(format!(
                                "{name} has invalid scope-link component {}; the reader reports this token",
                                String::from_utf8_lossy(link)
                            ));
                        }
                    };
                }
                let text = std::str::from_utf8(value)
                    .map_err(|_| format!("{name} scope links are not UTF-8"))?;
                let parsed =
                    content_reader_script::scope::parse(text, false, self.context.text.recorded());
                let selected = self
                    .evaluator()?
                    .enter_scope(&parsed, scopes)
                    .map_err(|why| why.to_string())?;
                match selected.map(|selected| (selected.this, selected)) {
                    Some((Scope::Country(country) | Scope::CountryLeader { country, .. }, _))
                        if country > 0 => Some(country),
                    Some((Scope::State(state), selected)) if state != 0 && state != NULL_STATE => {
                        if self.valid_state(state)?.is_none() {
                            return Err(format!("invalid state {state}: country/state-owner lookup requires an existing state"));
                        }
                        self.state_country_selector(Keyword::Owner, selected)?
                    }
                    _ if terminal == Keyword::Root => return Err("terminal ROOT without country/state needs shared origin_country fallback; scopes.rs exposes no domain accessor".into()),
                    _ => None,
                }
            } else {
                let text = std::str::from_utf8(value)
                    .map_err(|_| format!("{name} country specifier is not UTF-8"))?;
                if !content_reader_script::scope::Names::is_country(self.context.text.recorded(), text) {
                    return Err(format!(
                        "{name} country specifier {text} is not defined by the content stack"
                    ));
                }
                self.country_target(operand, scopes)?
            };
            let country = country.unwrap_or(0);
            names.push(self.country_name(country, false)?);
        }
        self.line(
            b"EFFECT_ADD_TO_WAR",
            &[
                ("COUNTRY1", Argument::Text(&names[0])),
                ("COUNTRY2", Argument::Text(&names[1])),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn set_border_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Scope::State(state) = scopes.this else {
            return Ok(String::new());
        };
        if self.valid_state(state)?.is_none() {
            return Ok(String::new());
        }
        let enabled = scope_selector(item).is_none()
            && item
                .value
                .scalar()
                .and_then(content_reader_parser::parser::read_bool)
                .unwrap_or(false);
        let name = self.state_name(state)?;
        self.line(
            if enabled {
                b"EFFECT_BORDER_WAR"
            } else {
                b"EFFECT_END_BORDER_WAR"
            },
            &[("STATE", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    fn border_states(&self, item: &Item, scopes: Scopes) -> Result<[u32; 2], String> {
        let mut result = [0; 2];
        for (index, name) in ["attacker", "defender"].into_iter().enumerate() {
            if let Some(operand) = field(children(item), name) {
                result[index] = self
                    .evaluator()?
                    .scoped_state_id(scalar_operand(operand)?, scopes)
                    .map_err(|why| why.to_string())? as u32;
            }
        }
        Ok(result)
    }

    fn border_state_label(&self, state: u32) -> Result<String, String> {
        let row = self.valid_state(state)?.ok_or_else(|| {
            format!("invalid state {state}: marked-state naming requires an existing state")
        })?;
        let controller = self
            .context
            .text
            .recorded()
            .slot(row.controller)
            .ok_or_else(|| {
                format!(
                    "country links lacks state {state} controller {}",
                    row.controller
                )
            })?;
        Ok(format!(
            "{}(\u{12}{})",
            self.state_name(state)?,
            self.tag(controller)?
        ))
    }

    pub(super) fn end_border_war(
        &self,
        item: &Item,
        scopes: Scopes,
        finalize: bool,
    ) -> Result<String, String> {
        let mut states = self.border_states(item, scopes)?;
        if finalize {
            let attacker =
                content_reader_parser::parser::read_bool_field(children(item), "attacker_win", false);
            let defender =
                content_reader_parser::parser::read_bool_field(children(item), "defender_win", false);
            if !attacker && !defender {
                return Ok(String::new());
            }
            if !attacker {
                states.swap(0, 1);
            }
        } else if self.valid_state(states[0])?.is_none() || self.valid_state(states[1])?.is_none() {
            return Ok(String::new());
        }
        let first = self.border_state_label(states[0])?;
        let second = self.border_state_label(states[1])?;
        self.line(
            if finalize {
                b"EFFECT_FINALIZE_BORDER_WAR"
            } else {
                b"EFFECT_CANCEL_BORDER_WAR"
            },
            &[
                ("STATE1", Argument::Text(&first)),
                ("STATE2", Argument::Text(&second)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn update_border_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if scopes.this == Scope::Combat {
            return Err(
                "combat-side target and attacker flag are not exposed by the description graph"
                    .into(),
            );
        }
        let states = self.border_states(item, scopes)?;
        if self.valid_state(states[0])?.is_none() || self.valid_state(states[1])?.is_none() {
            return Ok(String::new());
        }
        let first = self.state_name(states[0])?;
        let second = self.state_name(states[1])?;
        let mut body = self.line(
            b"EFFECT_UPDATE_BORDER_WAR",
            &[
                ("STATE1", Argument::Text(&first)),
                ("STATE2", Argument::Text(&second)),
            ],
            scopes,
            true,
        )?;
        if let Some(operand) = field(children(item), "combat_width") {
            let value = self.scoped_value(scalar_operand(operand)?, scopes)?;
            body.push_str(&self.line(
                b"EFFECT_UPDATE_BORDER_WAR_COMBAT_WIDTH",
                &[("NUM", Argument::Fixed5(value))],
                scopes,
                true,
            )?);
        }
        if field(children(item), "change_state_after_war").is_some() {
            let enabled = content_reader_parser::parser::read_bool_field(
                children(item),
                "change_state_after_war",
                false,
            );
            let value = self.localised(if enabled { b"YES" } else { b"NO" }, &[], scopes, true)?;
            body.push_str(&self.line(
                b"EFFECT_UPDATE_BORDER_WAR_CHANGE_STATE",
                &[("VAL", Argument::Text(&value))],
                scopes,
                true,
            )?);
        }
        for (name, max, modifier) in [
            (&first, "max_attackers", "attacker_modifier"),
            (&second, "max_defenders", "defender_modifier"),
        ] {
            let mut side = String::new();
            for (field_name, key) in [
                (max, b"EFFECT_UPDATE_BORDER_WAR_NUM_UNITS".as_slice()),
                (modifier, b"EFFECT_UPDATE_BORDER_WAR_MODIFIER"),
            ] {
                if let Some(operand) = field(children(item), field_name) {
                    let value = self.scoped_value(scalar_operand(operand)?, scopes)?;
                    side.push_str(&self.line(
                        key,
                        &[("NUM", Argument::Fixed5(value))],
                        scopes,
                        true,
                    )?);
                }
            }
            if !side.is_empty() {
                body.push_str(&format!("\t\u{11}H{name}\u{11}!:\n"));
                body.push_str(&side);
            }
        }
        Ok(body)
    }

    fn diplomacy_autonomy_feature(&self) -> Result<bool, String> {
        let features = self.complete::<tables::SimulationFeatures>(FEATURES)?;
        let (1, Some(features)) = (features.len(), each(features).next().transpose()?) else {
            return Err("simulation features lacks a unique autonomy feature mask".into());
        };
        Ok(features.dlc_mask & 0xE != 0)
    }

    pub(super) fn diplomacy_autonomy_add(
        &self,
        item: &Item,
        scopes: Scopes,
        ratio: bool,
    ) -> Result<String, String> {
        if !self.diplomacy_autonomy_feature()? {
            return Ok(String::new());
        }
        let current = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => Some(country).filter(|country| *country > 0),
            Scope::State(_) | Scope::Province(_) | Scope::Character(_) | Scope::Leader(_) => None,
            _ => return Err("autonomy description needs the carried country; this projected scope does not expose it".into()),
        };
        let value = field(children(item), "value");
        let value = if ratio {
            let Some(current) = current else {
                return Ok(String::new());
            };
            let statuses = self.complete::<tables::Autonomy>(SCORES)?;
            let recorded = self.context.text.recorded();
            let Some(status) = statuses
                .keyed(|key| recorded.slot(key as u32) == Some(current))
                .next()
                .transpose()
                .map_err(built)?
            else {
                return Ok(String::new());
            };
            let value = match value {
                Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
                None => 0,
            };
            status
                .upper_score
                .wrapping_sub(status.lower_score)
                .wrapping_mul(value)
                / 100_000
        } else {
            fixed_field(children(item), "value", 0)
        };
        if current.is_some() {
            self.line(
                b"ADD_AUTONOMY_SCORE_DESC",
                &[
                    ("VALUE", Argument::Fixed5(value)),
                    ("COUNTRY", Argument::Int32(1)),
                ],
                scopes,
                true,
            )
        } else {
            self.line(
                b"ADD_AUTONOMY_SCORE_DETAILED_DESC",
                &[("VALUE", Argument::Fixed5(value))],
                scopes,
                true,
            )
        }
    }

    pub(super) fn diplomacy_end_exile(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let name = content_reader_localisation::localisation::crt_case_text(
            &self.country_name(target, true)?,
            true,
            true,
        );
        self.localised(
            b"EFFECT_END_EXILE",
            &[("COUNTRY", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(super) fn diplomacy_become_exiled(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let name = self.country_name(target, true)?;
        self.line(
            b"EFFECT_BECOME_EXILED_IN",
            &[("HOSTNAMEDEF", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(super) fn diplomacy_remove_exile_tag(&self, scopes: Scopes) -> Result<String, String> {
        let frame = self.tables();
        let character = match scopes.this {
            Scope::Character(character) => character,
            Scope::Leader(reference) | Scope::CountryLeader { leader: reference, .. } => {
                let leaders = self.complete::<tables::UnitLeaders>(LEADER_CHARACTER)?;
                let row = keyed(&leaders, u128::from(reference))?
                    .ok_or_else(|| format!("unit leaders lacks scope leader {reference:#x}"))?;
                row.character_id
            }
            Scope::Country(_) | Scope::State(_) | Scope::Province(_) if scopes.bare => {
                return Ok(String::new());
            }
            _ => return Err("remove_exile_tag needs the carried character; this projected scope does not expose it".into()),
        };
        let tables = crate::character_scope::Tables::read_in(frame);
        let positive = match tables.unit_leader(character).map_err(|why| why.to_string())? {
            Some(crate::character_scope::UnitLeader::Direct(row)) => {
                *row.exile_country.require("exile country of unit leaders")? > 0
            }
            Some(crate::character_scope::UnitLeader::Referenced(_)) => return Err("referenced operative leader exile_country is not recorded by character operatives; no new input is captured".into()),
            None => false,
        };
        if positive {
            self.localised(b"EFFECT_REMOVE_EXILE_TAG", &[], scopes, false)
        } else {
            Ok(String::new())
        }
    }

    pub(super) fn diplomacy_autonomy_level(
        &self,
        item: &Item,
        scopes: Scopes,
        release: bool,
    ) -> Result<String, String> {
        let state = children(item).iter().rev().find(|field| {
            field.keyword_is("autonomy_state") || field.keyword_is("autonomous_state")
        });
        if release && state.is_none() {
            return Ok(String::new());
        }
        let target = if release {
            None
        } else {
            let Some(target) = self.country_target(item, scopes)? else {
                return Ok(String::new());
            };
            Some(target)
        };
        let id = state
            .map(scalar_operand)
            .transpose()?
            .map(content_reader_parser::parser::c_string)
            .unwrap_or_default();
        if release && id.is_empty() {
            return Ok(String::new());
        }
        let id =
            std::str::from_utf8(id).map_err(|_| "autonomy state id is not UTF-8".to_owned())?;
        let states = self
            .context
            .content
            .autonomy_states
            .ok_or_else(|| self.content_gap("autonomy states"))?;
        let state = states
            .get(id)
            .map_err(|gap| format!("autonomy state {id}: {gap}"))?;
        if state.is_none() && !id.is_empty() && (release || id != "autonomy_free") {
            return Err(format!(
                "autonomy state {id} is not defined by the recorded content stack"
            ));
        }
        let target = match target {
            Some(target) => target,
            None => {
                let Some(target) = self.country_target(item, scopes)? else {
                    return Ok(String::new());
                };
                target
            }
        };
        if !release && state.is_none() && id == "autonomy_free" {
            let country = self.country_name(target, true)?;
            return self.line(
                b"EFFECT_AUTONOMY_FREE",
                &[("COUNTRY", Argument::Text(&country))],
                scopes,
                true,
            );
        }
        let name = match state {
            Some(state) => self.localised(state.id.as_bytes(), &[], scopes, false)?,
            None => String::new(),
        };
        if release {
            let country = self.country_name(target, false)?;
            self.line(
                b"EFFECT_RELEASE_AUTONOMY",
                &[
                    ("COUNTRY", Argument::Text(&country)),
                    ("NAME", Argument::Text(&name)),
                ],
                scopes,
                true,
            )
        } else {
            let country = self.country_name(self.current_country(scopes)?, false)?;
            let target = self.country_name(target, false)?;
            self.line(
                b"EFFECT_SET_AUTONOMY",
                &[
                    ("COUNTRY", Argument::Text(&country)),
                    ("TARGET", Argument::Text(&target)),
                    ("AUTONOMY", Argument::Text(&name)),
                ],
                scopes,
                true,
            )
        }
    }

    pub(super) fn diplomacy_make_puppet(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let country = self.country_name(self.current_country(scopes)?, false)?;
        let target = self.country_name(target, false)?;
        self.line(
            b"EFFECT_MAKE_PUPPET",
            &[
                ("COUNTRY", Argument::Text(&country)),
                ("TARGET", Argument::Text(&target)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn create_faction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if !matches!(scopes.this, Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0)
        {
            return Ok(String::new());
        }
        let name = self.localised(
            content_reader_parser::parser::c_string(scalar_operand(item)?),
            &[],
            scopes,
            false,
        )?;
        if name.is_empty() {
            return Ok(String::new());
        }
        self.line(
            b"EFFECT_CREATE_FACTION",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(super) fn create_faction_template(&self, scopes: Scopes) -> Result<String, String> {
        if !matches!(scopes.this, Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0)
        {
            return Ok(String::new());
        }
        Err("faction template content unavailable: scalar and template/name/icon/color block forms need cached template lookup and declared NAME localisation, including template existence (no recorded input is missing)".into())
    }

    fn current_faction(&self, scopes: Scopes) -> Result<Option<(u32, u64)>, String> {
        let (Scope::Country(source)
        | Scope::CountryLeader {
            country: source, ..
        }) = scopes.this
        else {
            return Ok(None);
        };
        if source == 0 {
            return Ok(None);
        }
        Ok(self
            .context
            .world()?
            .faction(source)
            .map_err(|why| why.to_string())?
            .map(|faction| (source, faction)))
    }

    pub(super) fn dismantle_faction(&self, scopes: Scopes) -> Result<String, String> {
        let Some((source, faction)) = self.current_faction(scopes)? else {
            return Ok(String::new());
        };
        let world = self.context.world()?;
        if world
            .recorded()
            .faction_leader(source)
            .map_err(|why| why.to_string())?
            != Some(source)
        {
            return Ok(String::new());
        }
        let faction = self.context.text.faction_name_by_id(faction)?;
        self.line(
            b"EFFECT_DISMANTLE_FACTION",
            &[("NAME", Argument::Text(&faction))],
            scopes,
            true,
        )
    }

    pub(super) fn leave_faction(&self, scopes: Scopes) -> Result<String, String> {
        let Some((source, faction)) = self.current_faction(scopes)? else {
            return Ok(String::new());
        };
        let faction = self.context.text.faction_name_by_id(faction)?;
        let country = self.country_name(source, true)?;
        self.line(
            b"EFFECT_REMOVE_FROM_FACTION_NAME",
            &[
                ("COUNTRY", Argument::Text(&country)),
                ("FACTION", Argument::Text(&faction)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn remove_from_faction(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let source = self.current_country(scopes)?;
        let world = self.context.world()?;
        let faction = world.faction(source).map_err(|why| why.to_string())?;
        if faction.is_some()
            && world
                .recorded()
                .faction_leader(source)
                .map_err(|why| why.to_string())?
                != Some(source)
        {
            return Ok(String::new());
        }
        let country = self.country_name(target, false)?;
        match faction {
            Some(faction) => {
                let faction = self.context.text.faction_name_by_id(faction)?;
                self.line(
                    b"EFFECT_REMOVE_FROM_FACTION_NAME",
                    &[
                        ("COUNTRY", Argument::Text(&country)),
                        ("FACTION", Argument::Text(&faction)),
                    ],
                    scopes,
                    true,
                )
            }
            None => self.line(
                b"EFFECT_REMOVE_FROM_FACTION",
                &[("COUNTRY", Argument::Text(&country))],
                scopes,
                true,
            ),
        }
    }

    pub(super) fn set_faction_name(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some((_, faction)) = self.current_faction(scopes)? else {
            return Ok(String::new());
        };
        let name = if crate::triggers::scope_selector(item).is_some() {
            &b""[..]
        } else {
            item.value
                .read_string()
                .ok_or("set_faction_name requires the native scalar string")?
        };
        let new = self.localised(name, &[], scopes, true)?;
        let old = self.context.text.faction_name_by_id(faction)?;
        self.line(
            b"EFFECT_SET_FACTION_NAME",
            &[
                ("OLDNAME", Argument::Text(&old)),
                ("NEWNAME", Argument::Text(&new)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn set_faction_spymaster(&self, scopes: Scopes) -> Result<String, String> {
        let Some((_, faction)) = self.current_faction(scopes)? else {
            return Ok(String::new());
        };
        let name = self.context.text.faction_name_by_id(faction)?;
        self.line(
            b"EFFECT_SET_FACTION_SPYMASTER",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    pub(super) fn faction_goal_slot(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let category = match field(children(item), "category")
            .and_then(Item::value_keyword)
            .as_deref()
        {
            Some("short_term") => b"SHORT_TERM".as_slice(),
            Some("medium_term") => b"MEDIUM_TERM".as_slice(),
            Some("long_term") => b"LONG_TERM".as_slice(),
            _ => b"".as_slice(),
        };
        let category = self.localised(category, &[], scopes, false)?;
        let value = match field(children(item), "value") {
            Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
            None => 0,
        };
        let value = crate::triggers::fixed_round(value) as i32;
        let faction = self.current_faction(scopes)?;
        match faction {
            Some((_, faction)) => {
                let faction = self.context.text.faction_name_by_id(faction)?;
                self.line(
                    b"FACTION_ADD_EXTRA_GOAL_SLOT",
                    &[
                        ("VAL", Argument::Int32(value)),
                        ("CATEGORY", Argument::Text(&category)),
                        ("FACTION", Argument::Text(&faction)),
                    ],
                    scopes,
                    true,
                )
            }
            None => self.line(
                b"FACTION_ADD_EXTRA_GOAL_SLOT_NO_FACTION",
                &[
                    ("VAL", Argument::Int32(value)),
                    ("CATEGORY", Argument::Text(&category)),
                ],
                scopes,
                true,
            ),
        }
    }

    pub(super) fn faction_initiative(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let amount = if crate::triggers::scope_selector(item).is_some() {
            0
        } else {
            self.scoped_value(scalar_operand(item)?, scopes)?
        };
        let faction = self.current_faction(scopes)?;
        match faction {
            Some((_, faction)) => {
                let faction = self.context.text.faction_name_by_id(faction)?;
                self.localised(
                    b"ADD_FACTION_INITIATIVE_EFFECT",
                    &[
                        ("FACTION", Argument::Text(&faction)),
                        ("AMOUNT", Argument::Fixed5(amount)),
                    ],
                    scopes,
                    true,
                )
            }
            None => self.localised(
                b"ADD_FACTION_INITIATIVE_EFFECT_NO_FACTION",
                &[("AMOUNT", Argument::Fixed5(amount))],
                scopes,
                true,
            ),
        }
    }

    pub(super) fn faction_goal_content(&self, scopes: Scopes) -> Result<String, String> {
        if self.current_faction(scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("faction goal content unavailable: declared goal lookup and NAME_LOCALIZATION need the selected-country and faction environment, which cached content and Renderer do not expose (no new recorded input is missing)".into())
    }

    pub(super) fn faction_upgrade_min(&self, scopes: Scopes) -> Result<String, String> {
        if self.current_faction(scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("faction upgrade content unavailable: upgrade lookup needs its parent/type, declared localised name and fixed-point minimum; no cached faction-upgrade reader exists (no new recorded input is missing)".into())
    }

    pub(super) fn faction_rule_content(&self, scopes: Scopes) -> Result<String, String> {
        if self.current_faction(scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("faction rule content and expansion environment unavailable: declared rule lookup localises its token and expands SET_FACTION_RULE_EFFECT with the carried faction; no new recorded input is needed".into())
    }

    pub(super) fn faction_military_access(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let unlock = content_reader_parser::parser::c_string(scalar_operand(item)?) == b"yes";
        let name = self.localised(b"FACTION_ACCESS_TAB_MILITARY", &[], scopes, false)?;
        self.localised(
            if unlock {
                b"FACTION_ACCESS_TAB_UNLOCK"
            } else {
                b"FACTION_ACCESS_TAB_LOCK"
            },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }
}

#[cfg(test)]
mod tests;
