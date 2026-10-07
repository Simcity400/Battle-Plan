use super::*;
use replay::rows::tables;

impl Renderer<'_, '_, '_> {
    pub(super) fn purchase_contract(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        // 1403061D0: seller/buyer are scoped operands; factory count floors to
        // int32 text. Equipment uses the seller, but naming uses game-state country.
        let fields = children(item);
        // ReadEffect 140307C20 reuses these CScopedVariables. Their reader
        // 140539440/140539B50 retains prior flags; final-field parsing cannot.
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
                        "repeated purchase operand {name} needs retained CScopedVariable reader flags (140539440/140539B50)",
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
        };
        let seller_country = country("seller")?;
        let seller = self.context.text.country_marked_name(seller_country)?; // 140BA59E0.
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
            b"INTERNATIONAL_MARKET_EQUIPMENT_AMOUNT", // 142733BE0.
        )?;
        self.line(
            b"EFFECT_MARKET_CREATE_PURCHASE_CONTRACT",
            &[
                ("SELLER", Argument::Text(&seller)),
                ("BUYER", Argument::Text(&buyer)),
                ("CIVS", Argument::Text(&factories)), // 142733C14.
                ("EQUIPMENT", Argument::Text(&equipment)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn start_civil_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        // 140395FE0: ideology +0x3C0 is only the heading. Politics uses
        // ruling_party +0x70, falling back to the current ruling group.
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
            // 140395FE0 tests the result's valid byte +0x10, not operand presence.
            Ok(None) | Err(_) => {
                let frame = self.context.text.frame();
                let id = frame
                    .complete::<tables::Politics>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .find(|row| self.context.text.recorded().slot(row.country) == Some(country))
                    .and_then(|row| row.ruling_party)
                    .ok_or("Politics lacks civil-war fallback ruling ideology (140395FE0)")?;
                let token = frame
                    .complete::<tables::PoliticalParties>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .find(|row| {
                        self.context.text.recorded().slot(row.country) == Some(country)
                            && row.id == id
                    })
                    .ok_or(
                        "PoliticalParties lacks civil-war fallback ruling ideology (140395FE0)",
                    )?;
                self.politics_ideology(Some(&token.token), scopes)?
            }
        };
        // 1403A0010 receives elections=0, frequency=-1, empty long_name.
        // Unlike the heading, this appended helper text is not expanded.
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
        // 14037E9B0 / 1403799D0
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
        // 14037DF80: resolve target, validate the type, test the owned roster,
        // then localise COUNTRY/WARGOAL/TARGET, expand in caller scope, and LF.
        let target = self
            .country_target_field(item, "target")?
            .map(|target| self.country_target(target, scopes))
            .transpose()?
            .flatten()
            .unwrap_or(0);
        let fields = children(item);
        let Some(goal) = field(fields, "type").and_then(Item::value_keyword) else {
            return Ok(String::new()); // Missing/quoted/numeric token is not a type.
        };
        let cached = self.context.text.content().effects()?;
        let database = self
            .context
            .content
            .wargoals
            .or(cached.wargoals.as_ref())
            .ok_or_else(|| self.content_gap("Wargoal definitions"))?;
        let Some(goal) = database.find(goal.as_bytes())? else {
            return Ok(String::new()); // LookupWargoalTypeByToken@140A2C5D0 null object.
        };
        let source = self.current_country(scopes)?;
        if target == 0 || source == 0 {
            return Err("create_wargoal needs reserved country zero diplomacy/name inputs (14037DF80/140D28650)".into());
        }
        let recorded = self.context.text.recorded();
        let frame = self.context.text.frame();
        let roster = frame
            .complete::<tables::OwnedWargoals>()
            .map_err(|why| why.to_string())?;
        // 1403B1C60 -> 140A2C820: only the first generator is read. Other
        // generator modes leave this explicit +0x130/+0x13C vector empty;
        // BuildDescription does not execute their dynamic state selection.
        let generator = fields.iter().find(|field| field.keyword_is("generator"));
        for row in roster {
            let owner = recorded.slot(row.country).ok_or_else(|| {
                format!(
                    "CountryLinks lacks OwnedWargoals owner {} (140D28650)",
                    row.country
                )
            })?;
            if owner != source {
                continue;
            }
            // 140D28650 / FindInArray@1401B0F60: recipient is the target or
            // one of its stored direct subjects, with native country aliases.
            let recipient = if row.recipient == 0 {
                0
            } else {
                recorded.slot(row.recipient).ok_or_else(|| {
                    format!(
                        "CountryLinks lacks OwnedWargoals recipient {} (140D28650)",
                        row.recipient
                    )
                })?
            };
            if recipient != target {
                let appearance = frame
                    .complete::<tables::CountryAppearance>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .find(|row| recorded.slot(row.country) == Some(target))
                    .ok_or("CountryAppearance lacks target's ordered subjects (140D28650)")?;
                let subjects = appearance.subjects.require("CountryAppearance.subjects")?;
                let mut matches = false;
                for raw in subjects {
                    let subject = if *raw == 0 {
                        0
                    } else {
                        recorded.slot(*raw as u32).ok_or_else(|| {
                            format!("CountryLinks lacks ordered subject {raw} (140D28650)")
                        })?
                    };
                    if subject == recipient {
                        matches = true;
                        break;
                    }
                }
                if !matches {
                    continue;
                }
            }
            let token = self
                .context
                .world()?
                .token(row.goal_type as u32)?
                .ok_or_else(|| {
                    format!(
                        "LexicalTokens lacks owned wargoal type {} (140A2C5D0)",
                        row.goal_type
                    )
                })?;
            if installed_content::tokens::Token::c_str(token) != installed_content::tokens::Token::c_str(goal) {
                continue;
            }
            // 1403A35A0: an empty existing state set covers any generator;
            // otherwise every explicit claim must match a pointer (+0x48),
            // with invalid authored state IDs resolving to null/recorded zero.
            if row.states.is_empty() {
                return Ok(String::new());
            }
            let mut covered = true;
            for entry in generator
                .and_then(|field| field.value.items())
                .unwrap_or_default()
            {
                let id = installed_content::crt::scan_i32(entry.value.scalar().unwrap_or_default())
                    .stored()
                    .unwrap_or(0) as u32; // 1401F91B0 / 1424AB970.
                let state = self.valid_state(id)?.map_or(0, |state| state.id);
                if !row.states.contains(&state) {
                    covered = false;
                    break;
                }
            }
            if covered {
                return Ok(String::new());
            }
        }
        if self
            .context
            .localisation
            .get("EFFECT_CREATE_WARGOAL")
            .is_none()
        {
            return Err(
                "native localisation key EFFECT_CREATE_WARGOAL is not defined (14037DF80)".into(),
            );
        }
        let source = self.country_name(source, false)?;
        let target = self.country_name(target, false)?;
        let goal = self.localised(goal, &[], scopes, false)?; // 140616BC0.
        self.line(
            b"EFFECT_CREATE_WARGOAL",
            &[
                ("COUNTRY", Argument::Text(&source)),
                ("WARGOAL", Argument::Text(&goal)),
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
        // 1404AAEC0/1404AB530: faction lookup, scoped amount, plain country
        // name (140BA5960), then expansion in the copied scope + faction.
        let country = self.current_country(scopes)?;
        let faction = if country > 0 {
            self.context.world()?.faction(country)?
        } else {
            None
        };
        let value = if scope_selector(item).is_some() {
            0 // Generic selector reader retains the default amount.
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
        self.expanded_with_faction_environment(&text, scopes, Some(faction)) // No LF.
    }

    // 1404AC450: target first, then the stored ordered subject roster.
    pub(super) fn faction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
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
            .require("CountryAppearance.subjects")?
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
            let faction = self.context.text.faction_name_by_id(faction)?;
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
                Some(Some(self.context.text.faction_name_by_id(faction)?))
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
        // 14037F730 / reader 1403B2390; country-zero gate precedes the factory.
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
        // Factory 140D0A6D0 / static keywords 140082CB0; vf17 supplies the name.
        let relation = field(children(item), "relation").and_then(Item::value_keyword);
        let label: &[u8] = match relation.as_deref() {
            Some("naval_blockade") => b"DIPLOMACY_NAVAL_BLOCKADE_TITLE", // 14196E4F0
            Some("air_base_access" | "offer_air_base_access") => b"DIPLOMACY_AIR_BASE_ACCESS", // 14196DC60
            Some("guarantee") => b"DIPLOMACY_GUARANTEE_TITLE", // 140D0F790
            Some("military_access" | "offer_military_access") => b"DIPLOMACY_MILACC", // 140D0F820
            Some("improve_relation") => b"DIPLOMACY_IMPROVERELATION", // 140D0F7C0
            Some("non_aggression_pact") => b"DIPLOMACY_NONAGGRESSIONPACT", // 140D0F850
            Some("embargo") => b"DIPLOMACY_EMBARGO_TITLE", // 140D0F760
            Some("market_access_rights") => b"DIPLOMACY_INTERNATIONAL_MARKET_ACCESS_RIGHTS", // 14196EA30
            Some("war_relation") => b"DIPLOMACY_ATWAR", // 140D0F880
            Some("send_attache") => b"DIPLOMACY_SEND_ATTACHE", // 140D0F6B0
            Some("docking_rights" | "offer_docking_rights") => b"DIPLOMACY_DOCKING_RIGHTS", // 14196E0B0
            Some("licence_production_access") => b"DIPLOMACY_LICACC", // 140D0F7F0
            Some("puppet" | "lend_lease" | "equipment_purchase_contract_relation") => b"", // 140176EE0
            Some("boost_party_popularity") => return Err("temporary boost-party relation GetName@140D0F6E0 reads the constructor's null CIdeologyGroup+0x50 identifier; shared content does not expose that null-object identity".into()),
            _ => return Ok("BROKEN DIPLOMATIC RELATION EFFECT".into()), // 14275A998; no LF.
        };
        let name = self.localised(label, &[], scopes, false)?;
        let country = self.country_name(target, true)?;
        let active = installed_content::parser::read_bool_field(children(item), "active", true); // 14033D3F0.
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
        // 140379070 / 1403AF5F0: enemy precedes targeted_alliance; both are named
        // even at country zero. Repeated field reads append their selector chain.
        let mut names = Vec::new();
        for name in ["enemy", "targeted_alliance"] {
            let Some(operand) = self.country_target_field(item, name)? else {
                names.push(self.country_name(0, false)?);
                continue;
            };
            let value = installed_content::parser::c_string(scalar_operand(operand)?);
            if scope_selector(operand).is_some_and(|selector| {
                !matches!(
                    selector,
                    Selected::Keyword(Selector::Root | Selector::From | Selector::Prev)
                )
            }) {
                return Err(format!(
                    "{name} form {} is a country specifier, not a native ROOT/FROM/PREV selector (1403AD690); its CCountryTag reader is unavailable",
                    String::from_utf8_lossy(value)
                ));
            }
            let country = if value.contains(&b'.') || scope_selector(operand).is_some() {
                // 1403AD690 lowercases dotted links; 14036C350 follows each
                // link before projecting the last country's/state owner's tag.
                let mut terminal = Keyword::Root;
                for link in value.split(|byte| *byte == b'.') {
                    terminal = match Keyword::named(link) {
                        Some(link @ (Keyword::Root | Keyword::From | Keyword::Prev)) => link,
                        _ => {
                            return Err(format!(
                                "{name} has invalid scope-link component {} (1403AD690); native reader reports this token",
                                String::from_utf8_lossy(link)
                            ));
                        }
                    };
                }
                let text = std::str::from_utf8(value)
                    .map_err(|_| format!("{name} scope links are not UTF-8 (1403AD690)"))?;
                let parsed = installed_content::scope::parse(text, false, self.context.text.recorded());
                let selected = self.evaluator()?.enter_scope(&parsed, scopes)?;
                match selected.map(|selected| (selected.this, selected)) {
                    Some((Scope::Country(country) | Scope::CountryLeader { country, .. }, _))
                        if country > 0 => Some(country),
                    Some((Scope::State(state), selected)) if state != 0 && state != NULL_STATE => {
                        if self.valid_state(state)?.is_none() {
                            return Err(format!("invalid state {state}: native country/state-owner helper 14036C350 dereferences it"));
                        }
                        self.state_country_selector(Keyword::Owner, selected)?
                    }
                    _ if terminal == Keyword::Root => return Err("terminal ROOT without country/state needs shared origin_country fallback (14036C350 -> 14052B240); scopes.rs exposes no domain accessor".into()),
                    _ => None,
                }
            } else {
                let text = std::str::from_utf8(value)
                    .map_err(|_| format!("{name} country specifier is not UTF-8 (140BA53F0)"))?;
                if !installed_content::scope::Names::is_country(self.context.text.recorded(), text) {
                    return Err(format!(
                        "{name} country specifier {text} is not defined by the content stack (1403AD690/140BA53F0)"
                    ));
                }
                self.country_target(operand, scopes)?
            };
            let country = country.unwrap_or(0); // 140379070 -> 140BA5960.
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
        // 140317820: unchanged current state, no scoped expansion.
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
                .and_then(installed_content::parser::read_bool)
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
                    .scoped_state_id(scalar_operand(operand)?, scopes)?
                    as u32;
            }
        }
        Ok(result)
    }

    fn border_state_label(&self, state: u32) -> Result<String, String> {
        // 1403A0E30: state display name, '(', inline controller flag, ')'.
        let row = self.valid_state(state)?.ok_or_else(|| {
            format!("invalid state {state}: native marked-state helper 1403A0E30 dereferences it")
        })?;
        let controller = self
            .context
            .text
            .recorded()
            .slot(row.controller)
            .ok_or_else(|| {
                format!(
                    "CountryLinks lacks state {state} controller {} (1403A0E30, CState+0xCC)",
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
        // 14037A900 / 1403804C0; both evaluate state operands before flags.
        let mut states = self.border_states(item, scopes)?;
        if finalize {
            let attacker =
                installed_content::parser::read_bool_field(children(item), "attacker_win", false);
            let defender =
                installed_content::parser::read_bool_field(children(item), "defender_win", false);
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
        // 14038DFC0 / 1403A1330: combat-side context supplies both states.
        if scopes.this == Scope::Combat {
            return Err("combat-side CEventScope+0x60 target and attacker byte +0xB8 are not exposed by the description graph (1403A1330)".into());
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
            let enabled =
                installed_content::parser::read_bool_field(children(item), "change_state_after_war", false);
            let value = self.localised(if enabled { b"YES" } else { b"NO" }, &[], scopes, true)?;
            body.push_str(&self.line(
                b"EFFECT_UPDATE_BORDER_WAR_CHANGE_STATE",
                &[("VAL", Argument::Text(&value))],
                scopes,
                true,
            )?);
        }
        // Common max_units/modifier operands apply only with +0x60 combat context.
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

    // Feature 5@1401AE640 accepts any runtime DLC mask bit 1, 2 or 3.
    fn diplomacy_autonomy_feature(&self) -> Result<bool, String> {
        let [features] = self
            .context
            .text
            .frame()
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
        else {
            return Err(
                "SimulationFeatures lacks a unique autonomy feature mask (1401AE640 case 5)".into(),
            );
        };
        Ok(features.dlc_mask & 0xE != 0)
    }

    pub(super) fn diplomacy_autonomy_add(
        &self,
        item: &Item,
        scopes: Scopes,
        ratio: bool,
    ) -> Result<String, String> {
        // Ratio vf7@140371210; score vf7@140371620. Localization field +58
        // labels the recorded autonomy change, and is never read by either vf7.
        if !self.diplomacy_autonomy_feature()? {
            return Ok(String::new());
        }
        let current = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => Some(country).filter(|country| *country > 0),
            Scope::State(_) | Scope::Province(_) | Scope::Character(_) | Scope::Leader(_) => None,
            _ => return Err("autonomy description needs native CEventScope country +8; this projected scope does not expose it (140371210/140371620)".into()),
        };
        let value = field(children(item), "value");
        let value = if ratio {
            let Some(current) = current else {
                return Ok(String::new());
            };
            // Country +F88 -> diplomacy +350 is CCurrentAutonomyStatus;
            // a complete Autonomy table's absent row is a null pointer.
            let statuses = self
                .context
                .text
                .frame()
                .complete::<tables::Autonomy>()
                .map_err(|why| why.to_string())?;
            let Some(status) = statuses
                .iter()
                .find(|row| self.context.text.recorded().slot(row.subject) == Some(current))
            else {
                return Ok(String::new());
            };
            let value = match value {
                Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
                None => 0, // CScopedVariable constructor@14014AAC0.
            };
            // 1403A2CB0: two wrapping i64 operations, then signed /100000.
            status
                .upper_score
                .wrapping_sub(status.lower_score)
                .wrapping_mul(value)
                / 100_000
        } else {
            // ReadFixed5@1424ABB10/1424B0480 keeps +78 on failure; it never
            // drops the effect. Factory@14033B110 initializes +78 to zero.
            fixed_field(children(item), "value", 0)
        };
        if current.is_some() {
            self.line(
                b"ADD_AUTONOMY_SCORE_DESC",
                &[
                    ("VALUE", Argument::Fixed5(value)),
                    // Native quirk: type5 int32 literal1, NOT a country name.
                    // 1403712FE/140371305 and 1403716CD/1403716D4.
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
        // vf7@14037FFF0: capitalized definite name@140BA59B0, no LF.
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let name =
            installed_content::localisation::crt_case_text(&self.country_name(target, true)?, true, true);
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
        // vf7@140379CD0: target spec@140BA5D10 then vf15@1405336C0.
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
        // vf7@1402EC6F0 -> character resolver@1405294F0/GetUnitLeader@140F8A840.
        let frame = self.context.text.frame();
        let character = match scopes.this {
            Scope::Character(character) => character,
            Scope::Leader(reference) | Scope::CountryLeader { leader: reference, .. } => {
                let row = frame
                    .complete::<tables::UnitLeaders>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .find(|row| {
                        ((u64::from(row.id) << 32) | u64::from(row.reference_type)) == reference
                    })
                    .ok_or_else(|| format!("UnitLeaders lacks scope leader {reference:#x} (1405294F0)"))?;
                row.character_id
            }
            Scope::Country(_) | Scope::State(_) | Scope::Province(_) if scopes.bare => {
                return Ok(String::new());
            }
            _ => return Err("remove_exile_tag needs native CEventScope character +50; this projected scope does not expose it (1405294F0)".into()),
        };
        let tables = crate::character_scope::Tables::read(frame);
        let positive = match tables.unit_leader(character)? {
            Some(crate::character_scope::UnitLeader::Direct(row)) => {
                *row.exile_country.require("UnitLeaders.exile_country (CUnitLeader +0xED4)")? > 0
            }
            Some(crate::character_scope::UnitLeader::Referenced(_)) => return Err("referenced operative leader CUnitLeader.exile_country +0xED4 is not recorded by CharacterOperatives (1402EC6F0/140F8A840); no new input is captured".into()),
            None => false,
        };
        if positive {
            // The native performs neither scope expansion nor LF append.
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
        // release vf7@140387530 returns empty for null +128 before target.
        // set vf7@14038D870 checks target first; no freedom_level evaluation.
        let state = children(item).iter().rev().find(|field| {
            field.keyword_is("autonomy_state") || field.keyword_is("autonomous_state")
        }); // vf4 1403B4480/1403B4CC0: both spellings write the same field.
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
            .map(installed_content::parser::c_string)
            .unwrap_or_default();
        if release && id.is_empty() {
            return Ok(String::new());
        }
        let id = std::str::from_utf8(id)
            .map_err(|_| "autonomy state id is not UTF-8 (140669480)".to_owned())?;
        let states = self
            .context
            .content
            .autonomy_states
            .ok_or_else(|| self.content_gap("autonomy states"))?;
        let state = states.get(id); // 140669480 compares exact definition bytes.
        if state.is_none() && !id.is_empty() && (release || id != "autonomy_free") {
            return Err(format!(
                "autonomy state {id} is not defined by the recorded content stack (140669480)"
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
        // Native hardcoded free sentinel 1427CD270/global 14306F150. A defined
        // state with this id takes the ordinary state branch instead.
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
            None => String::new(), // Set's default empty name, 14038D870.
        }; // Release name 14066B140; set name 14062D120.
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
        // CMakePuppetEffect vf7@140384910; COUNTRY=current +10 name,
        // TARGET=selected country's +10 name (140BA56A0 then +10@140222400).
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
        // vf6 1404A8060 localises the raw value, unlike ParseScopeSelector readers.
        // vf7 1404AD3D0: no current country or empty stored name gives no text.
        if !matches!(scopes.this, Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0)
        {
            return Ok(String::new());
        }
        let name = self.localised(
            installed_content::parser::c_string(scalar_operand(item)?),
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
        // 1404AD630 / 1404B1410: a positive country is required before using the template.
        if !matches!(scopes.this, Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0)
        {
            return Ok(String::new());
        }
        Err("faction template content unavailable: native lookup 1404B1410 uses template database 143316B40 and 1404AD630 evaluates its declared NAME localisation expression at CFactionTemplate+0xD0; scalar and template/name/icon/color block forms require that cached content reader, including template existence (no recorded input is missing)".into())
    }

    fn current_faction(&self, scopes: Scopes) -> Result<Option<(u32, u64)>, String> {
        // GetScriptScopeFaction@141174150: no country gives no faction.
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
            .faction(source)?
            .map(|faction| (source, faction)))
    }

    pub(super) fn dismantle_faction(&self, scopes: Scopes) -> Result<String, String> {
        // 1404ADAD0: faction leader only; the effect's boolean never selects text.
        let Some((source, faction)) = self.current_faction(scopes)? else {
            return Ok(String::new());
        };
        let world = self.context.world()?;
        if world.recorded().faction_leader(source)? != Some(source) {
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
        // 1404ADE10: current country definite name, not the operand's selector.
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
        // 1404AE5F0: target country name; current country's faction and leader gate.
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let source = self.current_country(scopes)?;
        let world = self.context.world()?;
        let faction = world.faction(source)?;
        if faction.is_some() && world.recorded().faction_leader(source)? != Some(source) {
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
        // 1404AF850: name key expands once before OLDNAME/NEWNAME substitution,
        // and EFFECT_SET_FACTION_NAME expands again; vf6 14137DE50 clears selector names.
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
        // 1404B0390: only current country/faction are read; operand is ignored by vf7.
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
        // 1404AA830; category reader 140A142A0, label helper 140A142D0.
        // Category 4 is the constructor/default/invalid value (1404A8470).
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
            None => 0, // CScopedVariable constructor 14014AAC0.
        };
        let value = crate::triggers::fixed_round(value) as i32; // 1424D8760.
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
        // 1404ABBA0 / vf6 14137DCD0: selector flags leave constructor operand zero.
        // Native own text has no appended LF.
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
        // Add 1404AA450, remove 1404AE210, manifest 1404AEEE0; no faction -> empty.
        if self.current_faction(scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("faction goal content unavailable: vf6 1404A8310 resolves the declared goal in database 143316B10; GetLocalizedName 140A12C00 uses CFactionGoal NAME_LOCALIZATION +0x340 with selected-country and faction environment, which cached content and Renderer do not expose (no new recorded input is missing)".into())
    }

    pub(super) fn faction_upgrade_min(&self, scopes: Scopes) -> Result<String, String> {
        // 1404AF2C0: absent country/faction is empty before any display output.
        if self.current_faction(scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("faction upgrade content unavailable: 1404AF2C0 looks up upgrade token through 1404B10B0, its parent/type via vf13, its declared localised name via 140A1C740, and its fixed5 minimum at upgrade+0x68; no cached faction-upgrade reader exists (no new recorded input is missing)".into())
    }

    pub(super) fn faction_rule_content(&self, scopes: Scopes) -> Result<String, String> {
        // 1404AFEB0: absent faction is empty. Its +0x590 map is static content,
        // rebuilt from every rule/type by 1419826F0, not the active-rule state.
        if self.current_faction(scopes)?.is_none() {
            return Ok(String::new());
        }
        Err("faction rule content and expansion environment unavailable: 1404AFEB0 finds a declared rule in CFaction+0x590 (content-built by 1419826F0), localises its declared token with 140A1E260, then expands SET_FACTION_RULE_EFFECT with carried faction via 1410CF4A0; no new recorded input is needed".into())
    }

    pub(super) fn faction_military_access(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 1404AF650 has no country gate and no LF. vf6 1404A8100 -> 1424AFED0
        // uses exact strncmp("yes", raw scalar, 4), not ParseScopeSelector.
        let unlock = installed_content::parser::c_string(scalar_operand(item)?) == b"yes";
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
