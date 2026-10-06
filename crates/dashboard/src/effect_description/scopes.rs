use super::*;
use crate::triggers::{Eval, Scope};
use installed_content::scope::{self, Scope as Parsed};
use replay::rows::tables;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
enum Roster {
    Countries {
        other: bool,
        enemy: bool,
        possible: bool,
    },
    States,
    CountryStates {
        owned: bool,
        controlled: bool,
        core: bool,
    },
    Leaders {
        army: bool,
        navy: bool,
        operative: bool,
        global: bool,
    },
    Faction {
        allied: bool,
    },
    Subjects,
    Characters,
    Scientists {
        active: bool,
    },
    PartyLeaders,
    Neighbors,
    OriginalTags,
    IndustrialOrganisations,
    Missing(&'static str),
}

#[derive(Clone, Copy)]
struct IteratorKind {
    roster: Roster,
    random: bool,
}

// Vtables distinguish template classes with identical RTTI names. Selection
// virtuals feed 1402F4200 / 14036AA80 / 140312880 and 1402F4BA0.
fn iterator_kind(class: u64) -> Option<IteratorKind> {
    let (roster, random) = match class {
        0x142756730 => (
            Roster::Countries {
                other: false,
                enemy: false,
                possible: false,
            },
            false,
        ),
        0x142756620 => (
            Roster::Countries {
                other: false,
                enemy: false,
                possible: false,
            },
            true,
        ),
        0x142756A60 => (
            Roster::Countries {
                other: true,
                enemy: false,
                possible: false,
            },
            false,
        ),
        0x142756950 => (
            Roster::Countries {
                other: true,
                enemy: false,
                possible: false,
            },
            true,
        ),
        0x142758688 => (
            Roster::Countries {
                other: false,
                enemy: true,
                possible: false,
            },
            false,
        ),
        0x142758358 => (
            Roster::Countries {
                other: false,
                enemy: true,
                possible: false,
            },
            true,
        ),
        0x142756840 => (
            Roster::Countries {
                other: false,
                enemy: false,
                possible: true,
            },
            false,
        ),
        0x1427576E8 => (Roster::States, false),
        0x142757418 => (Roster::States, true),
        0x1427B2F58 => (Roster::Faction { allied: false }, false),
        0x142767790 => (Roster::Faction { allied: true }, false),
        0x1427678A0 => (Roster::Faction { allied: true }, true),
        0x142758578 => (Roster::Subjects, false),
        0x142758468 => (Roster::Subjects, true),
        0x14272AE58 => (Roster::Characters, false),
        0x14272AD68 => (Roster::Characters, true),
        0x14272B230 => (Roster::PartyLeaders, true),
        0x1427A9500 => (Roster::Scientists { active: false }, false),
        0x1427A9410 => (Roster::Scientists { active: false }, true),
        0x1427A96D0 => (Roster::Scientists { active: true }, true),
        0x142738278 => (Roster::Neighbors, false),
        0x1427575F8 => (Roster::Neighbors, true),
        0x142758AE8 => (Roster::OriginalTags, false),
        0x1427589B8 => (Roster::OriginalTags, true),
        0x142757D78 | 0x1427578C8 | 0x142757C88 => (
            Roster::CountryStates {
                owned: true,
                controlled: class == 0x142757C88,
                core: false,
            },
            class != 0x142757D78,
        ),
        0x142738188 | 0x142757AA8 => (
            Roster::CountryStates {
                owned: false,
                controlled: true,
                core: false,
            },
            class == 0x142757AA8,
        ),
        0x142757E68 | 0x142758048 => (
            Roster::CountryStates {
                owned: false,
                controlled: false,
                core: true,
            },
            class == 0x142758048,
        ),
        0x142758248 | 0x142758138 => (
            Roster::Missing(
                "CCountry ordered neighbor-country refs +0x1088/count +0x1094 are not recorded (14039D4E0)",
            ),
            class == 0x142758138,
        ),
        0x1427588A8 | 0x142758798 => (
            Roster::Missing(
                "CCountryOccupationStatus ordered occupied-country records +0x60/count +0x6C are not recorded (1413DE730)",
            ),
            class == 0x142758798,
        ),
        0x142753F50 | 0x142753C88 | 0x142753E68 | 0x142753D78 | 0x142753BA0 | 0x142753AB0
        | 0x142754210 | 0x142754128 | 0x142754038 => (
            Roster::Leaders {
                army: matches!(
                    class,
                    0x142753F50 | 0x142753C88 | 0x142753BA0 | 0x142753AB0 | 0x142754210
                ),
                navy: matches!(class, 0x142753E68 | 0x142753D78 | 0x142753BA0 | 0x142753AB0),
                operative: matches!(class, 0x142754128 | 0x142754038 | 0x142753BA0 | 0x142753AB0),
                global: class == 0x142754210,
            },
            matches!(class, 0x142753C88 | 0x142753D78 | 0x142753AB0 | 0x142754038),
        ),
        0x1427971A8 | 0x1427970B8 => (
            Roster::Missing(
                "CCountry ordered division pointers +0x290/count +0x29C are not recorded (14143D4A0); UNIT identity and ProvinceUnitOrder are available",
            ),
            class == 0x1427970B8,
        ),
        0x1427975D8 | 0x1427974E8 => (
            Roster::Missing(
                "state division iteration is not ported (14143D4A0); UNIT identity and ProvinceUnitOrder are recorded",
            ),
            class == 0x1427974E8,
        ),
        0x14272F680 | 0x14272F590 => (Roster::IndustrialOrganisations, class == 0x14272F590),
        _ => return None,
    };
    Some(IteratorKind { roster, random })
}

impl Renderer<'_, '_, '_> {
    fn collection_elements(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        // 1404C3640: first representative, caller header (140A0ED70), child indent +1.
        let Some(input) = field(children(item), "input") else {
            return Ok(String::new());
        };
        let collection = installed_content::collections::compile(input, self.context.content.collections)?;
        let Some(selected) = self
            .evaluator()?
            .collection_representative(&collection, scopes)?
        else {
            return Ok(String::new());
        };
        let header = match &collection.header {
            installed_content::collections::Header::Empty => String::new(),
            installed_content::collections::Header::Key(key) => self.localised(key, &[], scopes, true)?,
            installed_content::collections::Header::Bound(item) => {
                let Some(key) = item.value.scalar() else {
                    return Err("collection bound-name argument tree is not ported (1423912A0); corpus reach zero".into());
                };
                if key.is_empty() {
                    String::new()
                } else {
                    self.localised(key, &[], scopes, true)?
                }
            }
        };
        let body = self.effect_children(item);
        Ok("   ".repeat(indent) + &header + ":\n" + &self.group(&body, selected, indent + 1))
    }

    /// Fresh combined localisation environment; no caller links (1404A5070/1404A5160).
    #[allow(
        dead_code,
        reason = "staged core entry point for lead F's contested-owner leaves"
    )]
    pub(super) fn fresh_country_state_scope(
        &self,
        country: u32,
        state: u32,
    ) -> Result<Scopes, String> {
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            graph.scopes.push(crate::scripted_text::EventScope::of(
                crate::scripted_text::EventObject::CountryState { country, state },
            ));
            at
        };
        crate::triggers::event_scopes::project(&self.graph.borrow(), at)
    }

    pub(super) fn is_context(&self, item: &Item) -> bool {
        let name = key(item);
        !installed_content::effect_keys::REGISTERED_EFFECTS
            .iter()
            .any(|(_, native)| native.eq_ignore_ascii_case(&name))
            && item.value.items().is_some()
            && !matches!(
                scope::parse(&name, item.key_quoted, self.context.text.recorded()),
                Parsed::Unknown(_)
            )
    }

    pub(super) fn context_header(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<(Scopes, String)>, String> {
        let parsed = scope::parse(&key(item), item.key_quoted, self.context.text.recorded());
        let Some(selected) = self.evaluator()?.enter_scope(&parsed, scopes)? else {
            return Ok(None);
        };
        let header = match selected.this {
            Scope::Country(country) => {
                if country == self.context.perspective
                    && self.origin_country(scopes)? == Some(country)
                {
                    String::new()
                } else {
                    let tag = self.tag(country)?;
                    let name = self.country_name(country, false)?;
                    self.localised(
                        b"EFFECT_CONTEXT_COUNTRY",
                        &[
                            ("TAG", Argument::Text(tag)),
                            ("COUNTRY", Argument::Text(&name)),
                        ],
                        selected,
                        false,
                    )?
                }
            }
            Scope::State(state) => {
                let name = self.state_name(state)?;
                self.localised(
                    b"EFFECT_CONTEXT_STATE",
                    &[("COUNTRY", Argument::Text(&name))],
                    selected,
                    false,
                )?
            }
            Scope::Character(_) | Scope::Leader(_) => {
                let id = match selected.this {
                    Scope::Character(id) => id,
                    Scope::Leader(id) => {
                        self.context
                            .world()?
                            .unit_leader(id)?
                            .ok_or("UnitLeaders lacks context leader")?
                            .character_id
                    }
                    _ => unreachable!(),
                };
                let country = self
                    .context
                    .world()?
                    .character(id)?
                    .ok_or("Characters lacks context character")?
                    .country;
                let slot = self
                    .context
                    .world()?
                    .slot(country)
                    .ok_or("CountryLinks lacks context character country")?;
                let name = self.character_label(id)?;
                let flag = format!("\u{12}{}", self.tag(slot)?);
                self.localised(
                    b"EFFECT_CONTEXT_LEADER",
                    &[
                        ("LEADER", Argument::Text(&name)),
                        ("FLAG", Argument::Text(&flag)),
                    ],
                    selected,
                    false,
                )?
            }
            Scope::IndustrialOrganisation(reference) => {
                let row = self.context.world()?.industrial_organisation(reference)?;
                let country = self
                    .context
                    .world()?
                    .slot(row.owner)
                    .ok_or("CountryLinks lacks industrial organisation owner")?;
                let name = self.organisation_name(reference)?;
                let flag = format!("\u{12}{}", self.tag(country)?);
                self.localised(
                    b"EFFECT_CONTEXT_MIO",
                    &[
                        ("ORGANISATION", Argument::Text(&name)),
                        ("FLAG", Argument::Text(&flag)),
                    ],
                    selected,
                    false,
                )? // 141382590.
            }
            Scope::Project { owner, id } => {
                if self.context.localisation.get("EFFECT_CONTEXT_SPECIAL_PROJECT").is_none() {
                    return Err("native carrier localisation key EFFECT_CONTEXT_SPECIAL_PROJECT is not defined (141382590)".into());
                }
                let country = self.context.world()?.slot(owner)
                    .ok_or("CountryLinks lacks project owner")?;
                let name = self.project_name(owner, id)?;
                let flag = format!("\u{12}{}", self.tag(country)?);
                self.localised(
                    b"EFFECT_CONTEXT_SPECIAL_PROJECT",
                    &[("SPECIAL_PROJECT", Argument::Text(&name)), ("FLAG", Argument::Text(&flag))],
                    selected, false,
                )? // 141382590 -> 140FCDDF0.
            }
            // 141382590 has no raid/unit header arm; still walk the children.
            Scope::Raid(_) | Scope::Unit(_) => String::new(),
            _ => return Err("context header kind is not ported (141382590)".into()),
        };
        Ok(Some((selected, header)))
    }

    // 14052B240 reads ROOT's country or state owner, falling back to THIS only
    // when ROOT carries neither; 141382590 uses it for perspective suppression.
    fn origin_country(&self, scopes: Scopes) -> Result<Option<u32>, String> {
        match scopes.root {
            Scope::Country(country) => Ok((country > 0).then_some(country)),
            Scope::State(state) if state != 0 && state != crate::triggers::NULL_STATE => {
                let state = self
                    .context
                    .world()?
                    .states
                    .get(&state)
                    .ok_or("ROOT state owner is not recorded (14052B240)")?;
                self.context
                    .world()?
                    .recorded()
                    .slot(state.owner)
                    .map(Some)
                    .ok_or_else(|| format!("CountryLinks lacks ROOT state owner {}", state.owner))
            }
            _ => Ok(match scopes.this {
                Scope::Country(country) if country > 0 => Some(country),
                _ => None,
            }),
        }
    }

    pub(super) fn context_effect(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        let (selected, mut header) = match self.context_header(item, scopes) {
            Ok(Some(value)) => value,
            Ok(None) => return String::new(),
            Err(why) => {
                self.refuse(&key(item), why);
                return String::new();
            }
        };
        let body = self.attached(children(item), true);
        let child_indent = if header.is_empty() {
            0
        } else {
            let before = self.out.refusals.clone();
            let multiple = self.multiple(children(item), selected);
            let failed = self.out.refusals != before;
            if failed {
                return String::new(); // Preserve the probe's leaf causes once (140533D80).
            }
            self.out.refusals = before;
            if multiple {
                header.push_str(":\n");
                indent + 1
            } else {
                header.push(' ');
                0
            }
        };
        if body.first().is_some_and(|(child, _)| !self.indents(child)) {
            header.clear();
        }
        let mut text = String::new();
        for (child, branches) in body {
            let line = if child.keyword_is("if") {
                self.if_chain(child, &branches, selected, child_indent, false)
                    .unwrap_or_default()
            } else {
                self.builder(child, selected, child_indent)
            };
            text.push_str(&line); // 141381200 concatenates children directly.
        }
        if text.is_empty() {
            text
        } else {
            header + &text
        }
    }
}

impl Renderer<'_, '_, '_> {
    pub(super) fn scope_builder(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Option<String> {
        let class = self.effect_class(item)?;
        let result = if let Some(kind) = iterator_kind(class.vtable) {
            self.iterator(item, scopes, indent, kind)
        } else if class.vtable == 0x1427B6A68 {
            self.collection_elements(item, scopes, indent)
        } else if class.vtable == 0x142760010 {
            self.array_scope(item, scopes, indent)
        } else if class.vtable == 0x14272B088 {
            self.character_list(item, scopes, indent)
        } else if class.vtable == 0x142750FC8 {
            self.set_politics(item, scopes)
        } else {
            return None;
        };
        Some(match result {
            Ok(text) => text,
            Err(why) => {
                self.refuse(class.name, why);
                String::new()
            }
        })
    }

    pub(super) fn iterator_multiple(&self, item: &Item) -> bool {
        self.effect_class(item).is_some_and(|class| {
            iterator_kind(class.vtable).is_some() || class.vtable == 0x14272B088
        }) // vf11 ReturnTrue@1401802A0.
    }

    fn roster_order_gap(&self, source: Roster) -> Option<&'static str> {
        match source {
            Roster::CountryStates { owned: true, .. } => Some(
                "owned_state_order: two or more targets pass the limit; CCountry+0x478/count +0x484 is not recorded (14039D200/14039D340)",
            ),
            Roster::CountryStates {
                controlled: true, ..
            } => Some(
                "controlled_state_order: two or more targets pass the limit; CCountry+0x460/count +0x46C is not recorded (140319610)",
            ),
            Roster::CountryStates { core: true, .. } => Some(
                "core_state_order: two or more targets pass the limit; CCountry+0x4A8/count +0x4B4 is not recorded (14039D1A0)",
            ),
            Roster::Leaders {
                army: true,
                navy: true,
                ..
            } => Some(
                "unit_leader_order: two or more targets pass the limit; army/navy/operative rosters are not recorded (140338150/140338ED0/1403388C0)",
            ),
            Roster::Leaders { army: true, .. } => Some(
                "army_leader_order: two or more targets pass the limit; CCountryCharacters+0x70/count +0x7C is not recorded (140338150)",
            ),
            Roster::Leaders { navy: true, .. } => Some(
                "navy_leader_order: two or more targets pass the limit; CCountryCharacters+0x88/count +0x94 is not recorded (140338ED0)",
            ),
            Roster::Leaders {
                operative: true, ..
            } => Some(
                "operative_order: two or more targets pass the limit; CIntelligenceAgency+0xD8/count +0xE4 is not recorded (1403388C0/140368C10)",
            ),
            _ => None,
        }
    }

    fn roster(&self, source: Roster, item: &Item, scopes: Scopes) -> Result<Vec<Scope>, String> {
        let world = self.context.world()?;
        let frame = world.frame();
        let country = || self.current_country(scopes);
        let slot = |raw| {
            if raw == 0 {
                return Ok(0);
            }
            world
                .slot(raw)
                .ok_or_else(|| format!("CountryLinks lacks iterator country {raw}"))
        };
        match source {
            Roster::Missing(why) => {
                if self.effect_class(item).is_some_and(|class| {
                    matches!(
                        class.vtable,
                        0x142758248 | 0x142758138 | 0x1427588A8 | 0x142758798
                    )
                }) && !matches!(scopes.this, Scope::Country(_))
                {
                    return Ok(Vec::new());
                }
                Err(why.into())
            }
            Roster::CountryStates {
                owned,
                controlled,
                core,
            } => {
                let Scope::Country(caller) = scopes.this else {
                    return Ok(Vec::new());
                };
                let states = frame
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?;
                let owned_count = if owned {
                    Some(world.owned_state_count(caller)?)
                } else {
                    None
                };
                let mut out = Vec::new();
                let mut found_owned = 0;
                let cores = if core {
                    Some(
                        frame
                            .complete::<tables::StateCores>()
                            .map_err(|why| why.to_string())?,
                    )
                } else {
                    None
                };
                let nonempty = if owned || controlled {
                    Some(
                        frame
                            .complete::<tables::Provinces>()
                            .map_err(|why| why.to_string())?
                            .iter()
                            .map(|row| row.state)
                            .collect::<std::collections::BTreeSet<_>>(),
                    )
                } else {
                    None
                };
                for row in states {
                    // Country roster writers omit empty states (1406C52B0/1406C3890).
                    if nonempty
                        .as_ref()
                        .is_some_and(|states| !states.contains(&row.id))
                    {
                        continue;
                    }
                    let is_owned = owned && slot(row.owner)? == caller;
                    if is_owned {
                        found_owned += 1;
                    }
                    let is_core = match cores {
                        Some(cores) => cores
                            .iter()
                            .find(|core| core.state == row.id)
                            .ok_or("StateCores lacks state membership")?
                            .cores
                            .iter()
                            .map(|raw| slot(*raw))
                            .collect::<Result<Vec<_>, _>>()?
                            .contains(&caller),
                        None => true,
                    };
                    if (!owned || is_owned)
                        && (!controlled || slot(row.controller)? == caller)
                        && is_core
                    {
                        out.push(Scope::State(row.id));
                    }
                }
                if owned_count.is_some_and(|count| count != found_owned) {
                    return Err("States.owner membership differs from CountryCounterInputs.owned_state_count (CCountry+0x484)".into());
                }
                Ok(out)
            }
            Roster::Leaders {
                army,
                navy,
                operative,
                global,
            } => {
                let caller = match scopes.this {
                    Scope::Country(caller) => caller,
                    _ if !global => return Ok(Vec::new()),
                    _ => 0,
                };
                let mut out = Vec::new();
                let invisible = field(children(item), "include_invisible").is_some_and(|field| {
                    field.value.scalar().and_then(installed_content::parser::read_bool) == Some(true)
                });
                let members = if operative {
                    Some(
                        frame
                            .complete::<tables::OperativeMemberships>()
                            .map_err(|why| why.to_string())?,
                    )
                } else {
                    None
                };
                for row in frame
                    .complete::<tables::UnitLeaders>()
                    .map_err(|why| why.to_string())?
                {
                    let reference = u64::from(row.id) << 32 | u64::from(row.reference_type);
                    let member = members.is_some_and(|members| {
                        members.iter().any(|member| {
                            member.id == reference
                                && member.roster_kind == 0
                                && world.slot(member.roster_country) == Some(caller)
                        })
                    });
                    let military = (army && matches!(row.category, 0 | 1)
                        || navy && row.category == 2)
                        && (global || slot(row.owner)? == caller);
                    // CollectVisibleArmy/NavyLeaders@1406CE170/1406CE1D0, mask 0xF.
                    if military && !invisible && !self.evaluator()?.leader_visible(row).decided()? {
                        continue;
                    }
                    if military || operative && member {
                        out.push(Scope::Leader(reference));
                    }
                }
                Ok(out)
            }
            Roster::IndustrialOrganisations => {
                let Scope::Country(caller) = scopes.this else {
                    return Ok(Vec::new());
                };
                if caller == 0 {
                    return Ok(Vec::new());
                }
                let invisible = field(children(item), "include_invisible").is_some_and(|field| {
                    field.value.scalar().and_then(installed_content::parser::read_bool) == Some(true)
                });
                let mut rows = Vec::new();
                for row in frame
                    .complete::<tables::IndustrialOrganisations>()
                    .map_err(|why| why.to_string())?
                {
                    if slot(row.country)? == caller {
                        rows.push(row);
                    }
                }
                rows.sort_by_key(|row| row.ordinal); // CProductionStatus+0x130, 14139AB30.
                let mut out = Vec::new();
                for row in rows {
                    if invisible || self.organisation_visible(row)? {
                        out.push(Scope::IndustrialOrganisation(row.organisation));
                    }
                }
                Ok(out)
            }
            Roster::States => {
                frame
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?;
                // 14039D260 walks CGameState's state-id vector, not ownership order.
                Ok(world
                    .states
                    .keys()
                    .copied()
                    .filter(|id| *id > 0)
                    .map(Scope::State)
                    .collect())
            }
            Roster::Countries {
                other,
                enemy,
                possible,
            } => {
                let mut rows = frame
                    .complete::<tables::Countries>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.canonical_slot); // 1413DE860.
                let caller = if other || enemy {
                    Some(country()?)
                } else {
                    None
                };
                // WarRelations contains only matrix wars with retired +0x49 == 0
                // (engine cwar_relation::active), exactly 1406F3A00's predicate.
                let mut enemies = std::collections::BTreeSet::new();
                if enemy {
                    for row in frame
                        .complete::<tables::WarRelations>()
                        .map_err(|why| why.to_string())?
                    {
                        let first = slot(row.first_country)?;
                        let second = slot(row.second_country)?;
                        if Some(first) == caller {
                            enemies.insert(second);
                        }
                        if Some(second) == caller {
                            enemies.insert(first);
                        }
                    }
                }
                let mut out = Vec::new();
                for row in rows {
                    let target = row.canonical_slot;
                    if target == 0
                        || (!possible && !world.exists(target)?)
                        || (other && Some(target) == caller)
                    {
                        continue;
                    }
                    if enemy && !enemies.contains(&target) {
                        continue;
                    }
                    out.push(Scope::Country(target));
                }
                Ok(out)
            }
            Roster::Faction { allied } => {
                let caller = country()?;
                let Some(faction) = world.faction(caller)? else {
                    return Ok(if allied {
                        Vec::new()
                    } else {
                        vec![Scope::Country(caller)]
                    }); // 1404B0D70.
                };
                let mut rows = frame
                    .complete::<tables::FactionMembers>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .filter(|row| row.faction == faction)
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.ordinal); // CFaction+0x58, 14039D140.
                let mut out = Vec::new();
                for row in rows {
                    let target = slot(row.country)?;
                    if (!allied || target != caller) && world.exists(target)? {
                        out.push(Scope::Country(target));
                    }
                }
                Ok(out)
            }
            Roster::Subjects => {
                let caller = country()?;
                let mut out = Vec::new();
                for &raw in world
                    .appearance(caller)?
                    .subjects
                    .require("CountryAppearance.subjects")?
                {
                    // CDiplomacyStatus+0x170, 14039D560.
                    if raw <= 0 {
                        continue;
                    }
                    let target = slot(raw as u32)?;
                    if target != caller && world.exists(target)? {
                        out.push(Scope::Country(target));
                    }
                }
                Ok(out)
            }
            Roster::Characters => Ok(world
                .country_characters(country()?)?
                .into_iter()
                .map(|id| Scope::Character(id.unwrap_or(0)))
                .collect()), // 14138B830.
            Roster::Scientists { active } => {
                let caller = country()?;
                let mut rows = Vec::new();
                for row in frame
                    .complete::<tables::Scientists>()
                    .map_err(|why| why.to_string())?
                {
                    if slot(row.country)? == caller {
                        rows.push(row);
                    }
                }
                if active && !rows.is_empty() {
                    return Err("CScientist active byte +0x130 is not recorded (14048F1D0); Scientists is Hourly".into());
                }
                rows.sort_by_key(|row| row.ordinal); // 14048F3F0 -> 140490C00.
                Ok(rows
                    .into_iter()
                    .map(|row| Scope::Character(row.character))
                    .collect())
            }
            Roster::PartyLeaders => {
                let mut out = Vec::new();
                // 1402F7180 requires has_ideology and exactly one passing target.
                // Without that mandatory predicate the selector returns none.
                if !self
                    .limit(children(item))?
                    .iter()
                    .any(|item| item.keyword_is("has_ideology"))
                {
                    return Ok(out);
                }
                for party in world.party_leaders(country()?)? {
                    // CPoliticsStatus+0x20, 1402F3570.
                    if let Some(first) = party.leaders.iter().min_by_key(|leader| leader.ordinal) {
                        if first.ordinal != 0 {
                            return Err("PartyLeaders lacks ordinal zero (14118E5A0)".into());
                        }
                        if let Some(id) = first.character {
                            out.push(Scope::Character(id));
                        }
                    }
                }
                if !out.is_empty()
                    && self
                        .limit(children(item))?
                        .iter()
                        .any(|item| item.keyword_is("has_ideology"))
                {
                    return Err("CHasIdeology reads CCountryLeader current ideology pointer +0x140 (140440A10/140F8EC50); CountryLeaderRoles records the independent map key, not this operand (writer 1403611C0)".into());
                }
                Ok(out)
            }
            Roster::Neighbors => {
                let Scope::State(state) = scopes.this else {
                    return Ok(Vec::new());
                };
                let neighbors = world.neighbor_states.map_err(str::to_owned)?;
                Ok(neighbors
                    .get(&state)
                    .into_iter()
                    .flatten()
                    .copied()
                    .map(Scope::State)
                    .collect()) // 140319670 / 1413AB2C0.
            }
            Roster::OriginalTags => {
                let operand = field(children(item), "original_tag")
                    .and_then(|field| field.value.scalar())
                    .unwrap_or(b"THIS");
                let Some(target) = self.evaluator()?.scoped_country(operand, scopes)? else {
                    return Ok(Vec::new());
                };
                let recorded = world.recorded();
                if let Some(hour) = recorded.original_tags().pending() {
                    return Err(format!(
                        "_CountriesForOriginalTags is unknown since hour {hour} (1413AA900)"
                    ));
                }
                let original = recorded
                    .original_tags()
                    .name_tag(target)
                    .filter(|tag| !tag.is_empty())
                    .unwrap_or(self.tag(target)?.as_bytes());
                let name =
                    std::str::from_utf8(original).map_err(|_| "original tag is not UTF-8")?;
                let mut out = Vec::new();
                for group in recorded.original_tags().list(name) {
                    if group.members.len() > 1 {
                        return Err(format!(
                            "_CountriesForOriginalTags[{name}] order is ambiguous at {} (1413AA900)",
                            group.when
                        ));
                    }
                    for &slot in &group.members {
                        if world.exists(slot)? {
                            out.push(Scope::Country(slot));
                        }
                    }
                }
                let original = recorded.country(name);
                if let Some(original) = original
                    && world.exists(original)?
                {
                    out.push(Scope::Country(original));
                }
                Ok(out)
            }
        }
    }

    pub(super) fn draw(&self, scopes: Scopes) -> Result<u32, String> {
        let mut graph = self.graph.borrow_mut();
        let scope = graph
            .scopes
            .get_mut(
                scopes
                    .cursor
                    .ok_or("random description needs an event graph cursor")?,
            )
            .ok_or("random description cursor is absent")?;
        let draw = scope.random_draw()?;
        scope.random_seed = scope.random_seed.wrapping_add(1); // 1402F70C0, including singleton.
        Ok(draw)
    }

    fn targets(
        &mut self,
        roster: Vec<Scope>,
        items: &[Item],
        scopes: Scopes,
    ) -> Result<Vec<Scope>, String> {
        let mut out = Vec::new();
        for target in roster {
            let selected = self.evaluator()?.with_this(scopes, target)?;
            match self.condition(items, selected) {
                Eval::True => out.push(target),
                Eval::False => {}
                Eval::Unknown(why) => return Err(why.into_iter().collect::<Vec<_>>().join("; ")),
            }
        }
        Ok(out)
    }

    fn label(&self, scope: Scope, caller: Scopes, list: bool) -> Result<String, String> {
        let text = match scope {
            Scope::Country(0) => String::new(),
            Scope::Country(country) => {
                let name = self.country_name(country, false)?;
                let tag = self.tag(country)?;
                if list {
                    return self.localised(
                        b"EFFECT_LIST_COUNTRY",
                        &[
                            ("TAG", Argument::Text(tag)),
                            ("COUNTRY", Argument::Text(&name)),
                        ],
                        caller,
                        false,
                    );
                }
                format!("{name} \u{12}{tag}") // FormatMarkedName@140BA59E0.
            }
            Scope::State(0) => String::new(),
            Scope::State(id) => self.state_name(id)?,
            Scope::Character(0) => String::new(),
            Scope::Character(id) => self.character_label(id)?,
            Scope::IndustrialOrganisation(reference) => self.organisation_name(reference)?, // 14139AD50 -> 140DA44C0.
            Scope::Leader(id) => {
                let leader = self
                    .context
                    .world()?
                    .unit_leader(id)?
                    .ok_or("UnitLeaders lacks iterator target")?;
                String::from_utf8_lossy(&leader.inherited_text_a).into_owned() // 14138BC80 / 14138BE40, +0x40.
            }
            _ => return Err("iterator target label kind is unported".into()),
        };
        Ok(if list {
            format!("\u{11}Y{text}\u{11}!")
        } else {
            text
        })
    }

    // GetNameStringAt28@140612C90: this is the stored name, even when empty.
    fn character_label(&self, id: u32) -> Result<String, String> {
        if id == 0 {
            return Ok(String::new());
        }
        let row = self
            .context
            .world()?
            .character(id)?
            .ok_or("Characters lacks scope target")?;
        String::from_utf8(row.name.clone()).map_err(|_| "character name is not UTF-8".into())
    }

    // 140A3B740/140A3CFE0: default country/organisation pair; FROM is owner.
    fn organisation_scopes(
        &self,
        row: &replay::rows::IndustrialOrganisationRow,
    ) -> Result<Scopes, String> {
        use crate::scripted_text::{EventObject, EventScope, Link};
        let owner = self
            .context
            .world()?
            .slot(row.owner)
            .ok_or("CountryLinks lacks industrial organisation owner")?;
        let at = {
            let mut graph = self.graph.borrow_mut();
            let country = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::Country(owner)));
            let mut selected =
                EventScope::of(EventObject::IndustrialOrganisation(row.organisation));
            selected.from = Link::Scope(country);
            let at = graph.scopes.len();
            graph.scopes.push(selected);
            at
        };
        self.evaluator()?.event_scopes(at)
    }

    fn organisation_visible(
        &self,
        row: &replay::rows::IndustrialOrganisationRow,
    ) -> Result<bool, String> {
        let world = self.context.world()?;
        let token = world
            .token(row.token)?
            .ok_or("LexicalTokens lacks industrial organisation token")?;
        let database = self
            .context
            .content
            .industrial_organisations
            .ok_or_else(|| self.content_gap("industrial organisation visibility"))?;
        let template = database.template(token)?;
        let evaluator = self.evaluator()?;
        // Getter 140A44870 selects own +0xB0, else the direct inherited
        // pointer +0x360. Finalizer 140A45170/140A45AC0 installs that pointer
        // only for no children and no delete_included_values visible token.
        let visible = if !evaluator.has_children(&template.visible)? && !template.delete_visible {
            match &template.include {
                Some(parent) => database.template(parent)?.visible.as_slice(),
                None => &template.visible,
            }
        } else {
            &template.visible
        };
        if !evaluator.has_children(visible)? {
            return Ok(true);
        } // 141399450.
        evaluator
            .evaluate(visible, self.organisation_scopes(row)?)
            .decided()
    }

    pub(super) fn organisation_name(&self, reference: u64) -> Result<String, String> {
        let world = self.context.world()?;
        let row = world.industrial_organisation(reference)?;
        let name = if row.name.is_empty() {
            world
                .token(row.token)?
                .ok_or("LexicalTokens lacks industrial organisation name")?
        } else {
            row.name.as_slice()
        };
        let name =
            std::str::from_utf8(name).map_err(|_| "industrial organisation name is not UTF-8")?;
        let owner = world
            .slot(row.owner)
            .ok_or("CountryLinks lacks industrial organisation owner")?;
        let tagged = format!("{}_{name}", self.tag(owner)?);
        let key = if owner > 0 && self.context.localisation.get(&tagged).is_some() {
            tagged.as_bytes()
        } else {
            name.as_bytes()
        };
        self.localised(key, &[], self.organisation_scopes(row)?, true) // 140DA44C0/14146B1D0.
    }

    fn iterator(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
        kind: IteratorKind,
    ) -> Result<String, String> {
        let items = children(item);
        let roster = self.roster(kind.roster, item, scopes)?;
        let mut targets = Vec::new();
        // CRandomStateEffectBase vf25@1403374B0/140337D80: priority
        // membership preserves roster order; retry all only if none pass.
        if kind.random
            && matches!(kind.roster, Roster::States | Roster::Neighbors)
            && let Some(preferred) = field(items, "prioritize")
        {
            let ids = children(preferred)
                .iter()
                .filter_map(|item| item.value.scalar())
                .map(integer)
                .collect::<Vec<_>>();
            targets = self.targets(
                roster
                    .iter()
                    .copied()
                    .filter(
                        |scope| matches!(scope, Scope::State(id) if ids.contains(&(*id as i32))),
                    )
                    .collect(),
                items,
                scopes,
            )?;
        }
        if targets.is_empty() {
            targets = self.targets(roster, items, scopes)?;
        }
        if targets.len() >= 2
            && let Some(why) = self.roster_order_gap(kind.roster)
        {
            return Err(why.into());
        }
        let body = self.effect_children(item);
        if !kind.random
            && let Some(amount) =
                field(items, "random_select_amount").and_then(|field| field.value.scalar())
        {
            let amount = integer(amount);
            if amount < 0 {
                return Err(
                    "negative random_select_amount underflows native target removal (1402F76E0)"
                        .into(),
                );
            }
            while targets.len() > amount as usize {
                let at = self.draw(scopes)? as usize % targets.len();
                targets.swap_remove(at);
            } // 1402F76E0.
        }
        let visible = field(items, "visible_when_empty").is_some_and(|field| {
            field.value.scalar().and_then(installed_content::parser::read_bool) == Some(true)
        });
        if targets.is_empty() && !visible {
            return Ok(String::new());
        }
        let tooltip = field(items, "tooltip")
            .filter(|field| !field.value.scalar().is_some_and(|key| key.is_empty()));
        if kind.random {
            let target = if matches!(kind.roster, Roster::PartyLeaders) {
                // 1402F7180 requires a top-level has_ideology and exactly one
                // match. This selector does not draw from the caller seed.
                let mandatory = self
                    .limit(items)?
                    .iter()
                    .any(|item| item.keyword_is("has_ideology"));
                (mandatory && targets.len() == 1).then(|| targets[0])
            } else if targets.is_empty() {
                None
            } else {
                Some(targets[self.draw(scopes)? as usize % targets.len()])
            };
            if target.is_none() && !visible {
                return Ok(String::new());
            }
            let selected = self
                .evaluator()?
                .with_this(scopes, target.unwrap_or(scopes.this))?;
            let text = if let Some(tooltip) = tooltip {
                let mut text = self.custom_tooltip(tooltip, selected)?;
                text.push_str(":\n");
                text.push_str(&self.group(&body, selected, indent + 1));
                text
            } else {
                let label = target
                    .map(|target| self.label(target, scopes, false))
                    .transpose()?
                    .unwrap_or_default();
                let mut text = format!("(\u{11}Y{label}\u{11}!): "); // 1402F4BA0, 14270C620 / 14271BBBC.
                let before = self.out.refusals.clone();
                let probe = self.group(&body, selected, 0);
                if probe.bytes().filter(|byte| *byte == b'\n').count() < 2 {
                    text.push_str(&probe);
                } else {
                    self.out.refusals = before; // Probe advances RNG, but is not another occurrence.
                    text.push('\n');
                    text.push_str(&self.group(&body, selected, indent + 1));
                }
                text
            };
            return Ok(indent_nonempty_lines(&text, indent));
        }
        let individual = field(items, "display_individual_scopes").is_some_and(|field| {
            field.value.scalar().and_then(installed_content::parser::read_bool) == Some(true)
        });
        let mut text = String::new();
        let mut selected = self.evaluator()?.with_this(scopes, scopes.this)?;
        if let Some(tooltip) = tooltip {
            text.push_str(&self.custom_tooltip(tooltip, selected)?);
            text.push_str(":\n");
        } else if !individual && !targets.is_empty() {
            let mut labels = Vec::new();
            for &target in targets.iter().take(if targets.len() < 5 { 4 } else { 3 }) {
                labels.push(self.label(target, scopes, true)?);
            }
            let mut list = labels.join(", ");
            if targets.len() >= 5 {
                list = self.localised(
                    b"EFFECT_LIST_OTHERS",
                    &[
                        ("LIST", Argument::Text(&list)),
                        ("NUMBER", Argument::Int32((targets.len() - 3) as i32)),
                    ],
                    scopes,
                    false,
                )?;
            }
            text.push_str(&if targets.len() == 1 {
                format!("{list}: \n")
            } else {
                format!("({list}):\n")
            });
        }
        if targets.is_empty() {
            text.push_str(&self.group(&body, selected, indent + 1));
        } else if individual {
            for target in targets {
                selected = self.evaluator()?.repoint_this(selected, target)?;
                text.push_str(&format!("({}):\n", self.label(target, scopes, true)?));
                text.push_str(&self.group(&body, selected, indent + 1));
            }
        } else {
            selected = self.evaluator()?.repoint_this(selected, targets[0])?;
            text.push_str(&self.group(&body, selected, indent + 1));
        }
        Ok(indent_nonempty_lines(&text, indent)) // 1402F4200 / 14036AA80 / 140312880.
    }

    fn array_scope(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        let operand = field(children(item), "array")
            .and_then(|item| item.value.scalar())
            .unwrap_or_default();
        let roster = self
            .evaluator()?
            .scoped_array_scopes(operand, scopes)?
            .into_iter()
            .flatten()
            .collect();
        let targets = self.targets(roster, children(item), scopes)?;
        if targets.is_empty() {
            return Ok(String::new());
        }
        let target = targets[self.draw(scopes)? as usize % targets.len()]; // 1403A3930.
        let selected = self.evaluator()?.with_this(scopes, target)?;
        let label = self.label(target, scopes, false)?;
        let mut text = format!("\u{11}H{label}:\n\u{11}!"); // 14036B940.
        let body = self.effect_children(item);
        text.push_str(&self.group(&body, selected, indent + 1));
        Ok(text)
    }

    fn character_list(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        let roster = self.roster(Roster::Characters, item, scopes)?;
        let targets = self.targets(roster, children(item), scopes)?;
        let world = self.context.world()?;
        let frame = world.frame();
        let tables = crate::character_scope::Tables::read(frame);
        for &target in &targets {
            let id = match target {
                Scope::Character(id) => id,
                _ => continue,
            };
            if let Some(leader) = tables.unit_leader(id)?
                && leader.category() < 3
            {
                // 1402F5F60 reads these independent assignment references,
                // not the inverse formation/fleet leader binding.
                return Err("character_list_tooltip command-size inputs CArmyLeader +0x1048 / CNavyLeader +0xF60 are not recorded (1402F53E0/140C07780/140C077C0)".into());
            }
        }
        let roles = frame
            .complete::<tables::AdvisorRoles>()
            .map_err(|why| why.to_string())?;
        let traits = frame
            .complete::<tables::CharacterRoleTraits>()
            .map_err(|why| why.to_string())?;
        let hired = frame
            .complete::<tables::HiredAdvisors>()
            .map_err(|why| why.to_string())?;
        let country = self.current_country(scopes)?;
        let mut text = String::new();
        // Native slot strings, 1402F2830/1402F3060/1402F29F0/1402F3A50.
        for (slots, heading, sorted) in [
            (
                &[b"political_advisor".as_slice()][..],
                b"EFFECT_CHARACTER_LIST_POLITICAL_ADVISORS".as_slice(),
                false,
            ),
            (
                &[
                    b"high_command".as_slice(),
                    b"army_chief",
                    b"navy_chief",
                    b"air_chief",
                ][..],
                b"EFFECT_CHARACTER_LIST_MILITARY_ADVISORS".as_slice(),
                true,
            ),
            (
                &[b"theorist".as_slice()][..],
                b"EFFECT_CHARACTER_LIST_THEORISTS".as_slice(),
                false,
            ),
        ] {
            let mut members = Vec::new();
            for &target in &targets {
                let id = match target {
                    Scope::Character(id) => id,
                    _ => continue,
                };
                let mut names = Vec::new();
                for slot in slots {
                    if let Some(role) = roles
                        .iter()
                        .filter(|row| row.character == id && row.slot == *slot)
                        .min_by_key(|row| row.ordinal)
                        && let Some(row) = traits.iter().find(|row| {
                            row.character == id
                                && row.kind == replay::rows::CharacterRoleTraitRow::ADVISOR
                                && row.role == role.ordinal
                        })
                    {
                        for token in &row.tokens {
                            names.push(
                                world
                                    .token(*token)?
                                    .ok_or("LexicalTokens lacks advisor trait")?,
                            );
                        }
                    }
                }
                if !roles
                    .iter()
                    .any(|row| row.character == id && slots.contains(&row.slot.as_slice()))
                {
                    continue;
                }
                let mut score = 0;
                // Comparator 1402F6C40 -> 1402F3880 visits every advisor role,
                // including roles outside the displayed section.
                for row in traits.iter().filter(|row| {
                    row.character == id && row.kind == replay::rows::CharacterRoleTraitRow::ADVISOR
                }) {
                    for token in &row.tokens {
                        let name = world
                            .token(*token)?
                            .ok_or("LexicalTokens lacks advisor sort trait")?;
                        let suffix = name.rsplit(|byte| *byte == b'_').next().unwrap_or_default();
                        if !suffix.is_empty()
                            && suffix.iter().enumerate().all(|(i, byte)| {
                                byte.is_ascii_digit() || i == 0 && matches!(byte, b'+' | b'-')
                            })
                        {
                            score = score.max(integer(suffix));
                        }
                    }
                }
                members.push((id, names, score));
            }
            if sorted {
                members.sort_by_key(|member| std::cmp::Reverse(member.2));
            } // stable merge/insertion sort 1402F1B00.
            if members.is_empty() {
                continue;
            }
            text.push('\n');
            text.push_str(&self.localised(heading, &[], scopes, false)?);
            for (id, traits, _) in members {
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&format!("\u{11}Y{}\u{11}! (", self.character_label(id)?));
                let mut labels = traits
                    .iter()
                    .map(|name| {
                        String::from_utf8(name.to_vec())
                            .map_err(|_| "advisor trait name is not UTF-8")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                // 1410D7B80 searches the hired role, then compares its slot;
                // a hired role in this section must belong to this character.
                if hired.iter().any(|row| {
                    world.slot(row.country) == Some(country)
                        && row.character == id
                        && slots.contains(&row.slot.as_slice())
                }) {
                    labels.push(self.localised(
                        b"EFFECT_CHARACTER_LIST_IN_SERVICE",
                        &[],
                        scopes,
                        false,
                    )?);
                }
                text.push_str(&labels.join(", "));
                text.push(')');
            }
        }
        let mut text = indent_nonempty_lines(&text, indent);
        while text.bytes().rev().take_while(|byte| *byte == b'\n').count() < 2 {
            text.push('\n');
        } // EnsureTrailingNewlines@1424B6660, including empty lists.
        Ok(text)
    }

    pub(super) fn set_politics(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let items = children(item);
        let mut text = self.line(b"EFFECT_SET_POLITICS", &[], scopes, false)?; // 14036C090 -> 1403A0010.
        if let Some(ideology) = field(items, "ruling_party").and_then(|item| item.value.scalar()) {
            let country = self.current_country(scopes)?;
            let world = self.context.world()?;
            let parties = world
                .frame()
                .complete::<tables::PoliticalParties>()
                .map_err(|why| why.to_string())?;
            // ResolveIdeologySpecifier@1402EE8E0: cached token, numeric
            // database id, then decoded country's ruling ideology.
            let definition = if let Some(party) = parties
                .iter()
                .find(|row| row.token.eq_ignore_ascii_case(ideology))
            {
                party.id
            } else {
                let value = self.scoped_value(ideology, scopes)?;
                if let Some(party) = parties
                    .iter()
                    .find(|row| row.id == (value / 100_000) as u32)
                {
                    party.id
                } else if let Some(Scope::Country(target)) =
                    self.evaluator()?.scoped_scope(ideology, scopes)?
                {
                    parties
                        .iter()
                        .find(|row| world.slot(row.country) == Some(target) && row.is_ruling != 0)
                        .ok_or("PoliticalParties lacks scoped ruling ideology (14138B640)")?
                        .id
                } else {
                    return Err("ruling_party names no content ideology (1402EE8E0)".into());
                }
            };
            let party = parties
                .iter()
                .find(|row| world.slot(row.country) == Some(country) && row.id == definition)
                .ok_or("PoliticalParties lacks FindPartyByIdeology result (140B980C0)")?;
            let ideology = party.token.as_slice();
            let name = match field(items, "long_name")
                .and_then(|item| item.value.read_string())
                .filter(|key| !key.is_empty())
            {
                Some(key) => {
                    let name = self.localised(key, &[], scopes, false)?;
                    text.push_str(&self.line(
                        b"POLITICS_PARTY_NAME_CHANGE",
                        &[
                            (
                                "IDEOLOGY",
                                Argument::Text(&String::from_utf8_lossy(ideology)),
                            ),
                            ("NAME", Argument::Text(&name)),
                        ],
                        scopes,
                        false,
                    )?);
                    name
                }
                None => self
                    .context
                    .text
                    .party_name(party, self.tag(country)?, false)?, // GetLocalizedName@14118F860, not GetLongName.
            };
            text.push_str(&self.line(
                b"EFFECT_SET_POLITICS_RULING_PARTY",
                &[("PARTY", Argument::Text(&name))],
                scopes,
                false,
            )?);
        }
        match field(items, "elections_allowed")
            .and_then(|item| item.value.scalar().and_then(installed_content::parser::read_bool))
        {
            Some(false) => text.push_str(&self.line(
                b"EFFECT_SET_POLITICS_NO_ELECTIONS",
                &[],
                scopes,
                false,
            )?),
            Some(true) => {
                text.push_str(&self.line(b"EFFECT_SET_POLITICS_ELECTIONS", &[], scopes, false)?);
                if let Some(frequency) =
                    field(items, "election_frequency").and_then(|item| item.value.scalar())
                {
                    let frequency = integer(frequency);
                    if frequency >= 0 {
                        text.push_str(&self.line(
                            b"EFFECT_SET_POLITICS_ELECTION_FREQUENCY",
                            &[("MONTHS", Argument::Int32(frequency / 12))],
                            scopes,
                            false,
                        )?);
                    }
                }
            }
            None => {}
        }
        Ok(text) // Build@14036C090 aliases own-description and ignores indent.
    }
}
