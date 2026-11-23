use super::*;
use crate::triggers::{Eval, Scope};
use content_reader_script::scope::{self, Scope as Parsed};
use replay_data::rows::{
    CountryCounterInputRow, CountryRow, DivisionRow, DivisionStatusRow, FactionMemberRow,
    IndustrialOrganisationRow, OccupationSettingsRow, OperativeMembershipRow,
    ProvinceUnitOrderRow, ScientistRow, tables,
};

const COUNTRY_SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const MEMBERSHIP: u64 = mask::<OperativeMembershipRow>(&["ordinal"]);
const ORGANISATION: u64 = mask::<IndustrialOrganisationRow>(&["country", "ordinal", "token"]);
const FACTION_MEMBER: u64 = mask::<FactionMemberRow>(&["faction", "ordinal"]);
const SCIENTIST: u64 = mask::<ScientistRow>(&["country", "ordinal"]);
const SCIENTIST_ACTIVE: u64 = mask::<ScientistRow>(&["active"]);
const NEIGHBORS: u64 = mask::<CountryCounterInputRow>(&["neighbors"]);
const OCCUPIED: u64 = mask::<OccupationSettingsRow>(&["occupied"]);
const COUNTRY_ORDINAL: u64 = mask::<DivisionStatusRow>(&["country_ordinal"]);
const ROSTER_OWNER: u64 = mask::<DivisionRow>(&["roster_owner"]);
const MIXED: usize = replay_data::rows::column::<ProvinceUnitOrderRow>("mixed");

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
    CountryNeighbors,
    OccupiedCountries,
    CountryDivisions,
    StateDivisions,
    IndustrialOrganisations,
}

#[derive(Clone, Copy)]
struct IteratorKind {
    roster: Roster,
    random: bool,
}

fn iterator_kind(class: u64) -> Option<IteratorKind> {
    let (roster, random) = match class {
        0x1427EA260 => (
            Roster::Countries {
                other: false,
                enemy: false,
                possible: false,
            },
            false,
        ),
        0x1427EA150 => (
            Roster::Countries {
                other: false,
                enemy: false,
                possible: false,
            },
            true,
        ),
        0x1427EA590 => (
            Roster::Countries {
                other: true,
                enemy: false,
                possible: false,
            },
            false,
        ),
        0x1427EA480 => (
            Roster::Countries {
                other: true,
                enemy: false,
                possible: false,
            },
            true,
        ),
        0x1427EC1B8 => (
            Roster::Countries {
                other: false,
                enemy: true,
                possible: false,
            },
            false,
        ),
        0x1427EBE88 => (
            Roster::Countries {
                other: false,
                enemy: true,
                possible: false,
            },
            true,
        ),
        0x1427EA370 => (
            Roster::Countries {
                other: false,
                enemy: false,
                possible: true,
            },
            false,
        ),
        0x1427EB218 => (Roster::States, false),
        0x1427EAF48 => (Roster::States, true),
        0x1428468b8 => (Roster::Faction { allied: false }, false),
        0x1427FB360 => (Roster::Faction { allied: true }, false),
        0x1427FB470 => (Roster::Faction { allied: true }, true),
        0x1427EC0A8 => (Roster::Subjects, false),
        0x1427EBF98 => (Roster::Subjects, true),
        0x1427BEBE0 => (Roster::Characters, false),
        0x1427BEAF0 => (Roster::Characters, true),
        0x1427bef60 => (Roster::PartyLeaders, true),
        0x14283D0C0 => (Roster::Scientists { active: false }, false),
        0x14283CFD0 => (Roster::Scientists { active: false }, true),
        0x14283D290 => (Roster::Scientists { active: true }, true),
        0x14283D1A8 => (Roster::Scientists { active: true }, false),
        0x1427CBD90 => (Roster::Neighbors, false),
        0x1427EB128 => (Roster::Neighbors, true),
        0x1427EC618 => (Roster::OriginalTags, false),
        0x1427EC4E8 => (Roster::OriginalTags, true),
        0x1427EB8A8 | 0x1427EB3F8 | 0x1427EB7B8 => (
            Roster::CountryStates {
                owned: true,
                controlled: class == 0x1427EB7B8,
                core: false,
            },
            class != 0x1427EB8A8,
        ),
        0x1427CBCA0 | 0x1427EB5D8 => (
            Roster::CountryStates {
                owned: false,
                controlled: true,
                core: false,
            },
            class == 0x1427EB5D8,
        ),
        0x1427EB998 | 0x1427EBB78 => (
            Roster::CountryStates {
                owned: false,
                controlled: false,
                core: true,
            },
            class == 0x1427EBB78,
        ),
        0x1427EBD78 | 0x1427EBC68 => (Roster::CountryNeighbors, class == 0x1427EBC68),
        0x1427EC3D8 | 0x1427EC2C8 => (Roster::OccupiedCountries, class == 0x1427EC2C8),
        0x1427E7A80 | 0x1427E77B8 | 0x1427E7998 | 0x1427E78A8 | 0x1427E76D0 | 0x1427E75E0
        | 0x1427E7D40 | 0x1427E7C58 | 0x1427E7B68 => (
            Roster::Leaders {
                army: matches!(
                    class,
                    0x1427E7A80 | 0x1427E77B8 | 0x1427E76D0 | 0x1427E75E0 | 0x1427E7D40
                ),
                navy: matches!(class, 0x1427E7998 | 0x1427E78A8 | 0x1427E76D0 | 0x1427E75E0),
                operative: matches!(class, 0x1427E7C58 | 0x1427E7B68),
                global: class == 0x1427E7D40,
            },
            matches!(class, 0x1427E77B8 | 0x1427E78A8 | 0x1427E75E0 | 0x1427E7B68),
        ),
        0x14282ACE0 | 0x14282ABF0 => (Roster::CountryDivisions, class == 0x14282ABF0),
        0x14282B0C0 | 0x14282AFD0 => (Roster::StateDivisions, class == 0x14282AFD0),
        0x1427c34f8 | 0x1427c3408 => (Roster::IndustrialOrganisations, class == 0x1427c3408),
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
        let Some(input) = field(children(item), "input") else {
            return Ok(String::new());
        };
        let collection =
            content_reader_politics::collections::compile(input, self.context.content.collections)?;
        let Some(selected) = self
            .evaluator()?
            .collection_representative(&collection, scopes)
            .map_err(|why| why.to_string())?
        else {
            return Ok(String::new());
        };
        let header = match &collection.header {
            content_reader_politics::collections::Header::Empty => String::new(),
            content_reader_politics::collections::Header::Key(key) => {
                self.localised(key, &[], scopes, true)?
            }
            content_reader_politics::collections::Header::Bound(item) => {
                let Some(key) = item.value.scalar() else {
                    return Err("collection bound-name argument tree is unsupported".into());
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
            .map_err(|why| why.to_string())
    }

    pub(super) fn is_context(&self, item: &Item) -> bool {
        let name = key(item);
        !content_reader_script::effect_keys::REGISTERED_EFFECTS
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
        let Some(selected) = self
            .evaluator()?
            .enter_scope(&parsed, scopes)
            .map_err(|why| why.to_string())?
        else {
            return Ok(None);
        };
        let header = match selected.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => {
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
                            .unit_leader(id)
                            .map_err(|why| why.to_string())?
                            .ok_or("unit leaders lacks context leader")?
                            .character_id
                    }
                    _ => unreachable!(),
                };
                let country = self
                    .context
                    .world()?
                    .character(id)
                    .map_err(|why| why.to_string())?
                    .ok_or("Characters lacks context character")?
                    .country;
                let slot = self
                    .context
                    .world()?
                    .slot(country)
                    .ok_or("country links lacks context character country")?;
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
                let row = self
                    .context
                    .world()?
                    .industrial_organisation(reference)
                    .map_err(|why| why.to_string())?;
                let country = self
                    .context
                    .world()?
                    .slot(row.owner)
                    .ok_or("country links lacks industrial organisation owner")?;
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
                )?
            }
            Scope::Project { owner, id } => {
                if self
                    .context
                    .localisation
                    .get("EFFECT_CONTEXT_SPECIAL_PROJECT")
                    .is_none()
                {
                    return Err(
                        "carrier localisation key EFFECT_CONTEXT_SPECIAL_PROJECT is not defined"
                            .into(),
                    );
                }
                let country = self
                    .context
                    .world()?
                    .slot(owner)
                    .ok_or("country links lacks project owner")?;
                let name = self.project_name(owner, id)?;
                let flag = format!("\u{12}{}", self.tag(country)?);
                self.localised(
                    b"EFFECT_CONTEXT_SPECIAL_PROJECT",
                    &[
                        ("SPECIAL_PROJECT", Argument::Text(&name)),
                        ("FLAG", Argument::Text(&flag)),
                    ],
                    selected,
                    false,
                )?
            }
            Scope::Raid(_) | Scope::Unit(_) => String::new(),
            _ => return Err("context header kind is unsupported".into()),
        };
        Ok(Some((selected, header)))
    }

    fn origin_country(&self, scopes: Scopes) -> Result<Option<u32>, String> {
        match scopes.root {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                Ok((country > 0).then_some(country))
            }
            Scope::State(state) if state != 0 && state != crate::triggers::NULL_STATE => {
                let state = self
                    .context
                    .world()?
                    .states()
                    .get(&state)
                    .ok_or("ROOT state owner is not recorded")?;
                self.context
                    .world()?
                    .recorded()
                    .slot(state.owner)
                    .map(Some)
                    .ok_or_else(|| format!("country links lacks ROOT state owner {}", state.owner))
            }
            _ => Ok(match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0 => {
                    Some(country)
                }
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
                return String::new();
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
            text.push_str(&line);
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
        } else if class.vtable == 0x14284A2E0 {
            self.collection_elements(item, scopes, indent)
        } else if class.vtable == 0x1427f3b40 {
            self.array_scope(item, scopes, indent)
        } else if class.vtable == 0x1427BEDB8 {
            self.character_list(item, scopes, indent)
        } else if class.vtable == 0x1427E4AF8 {
            self.set_politics(item, scopes)
        } else {
            return None;
        };
        Some(match result {
            Ok(text) => text,
            Err(why) => {
                self.refuse(&key(item), why);
                String::new()
            }
        })
    }

    pub(super) fn iterator_multiple(&self, item: &Item) -> bool {
        self.effect_class(item).is_some_and(|class| {
            iterator_kind(class.vtable).is_some() || class.vtable == 0x1427BEDB8
        })
    }

    fn roster(&self, source: Roster, item: &Item, scopes: Scopes) -> Result<Vec<Scope>, String> {
        let world = self.context.world()?;
        let frame = world.tables();
        let country = || self.current_country(scopes);
        let slot = |raw| {
            if raw == 0 {
                return Ok(0);
            }
            world
                .slot(raw)
                .ok_or_else(|| format!("country links lacks iterator country {raw}"))
        };
        match source {
            Roster::CountryStates {
                owned,
                controlled,
                core,
            } => {
                let (Scope::Country(caller)
                | Scope::CountryLeader {
                    country: caller, ..
                }) = scopes.this
                else {
                    return Ok(Vec::new());
                };
                let inputs = world
                    .country_counter_inputs(caller)
                    .map_err(|why| why.to_string())?;
                let (recorded, name) = if owned {
                    (&inputs.owned_states, "owned states of country counter inputs")
                } else if controlled {
                    (
                        &inputs.controlled_states,
                        "controlled states of country counter inputs",
                    )
                } else {
                    debug_assert!(core);
                    (&inputs.core_states, "core states of country counter inputs")
                };
                let states = recorded
                    .require(name)?
                    .as_deref()
                    .ok_or_else(|| format!("{name} capture unavailable"))?;
                let mut out = Vec::new();
                for &id in states {
                    if owned && controlled {
                        let row = world.states().get(&id).ok_or_else(|| {
                            format!("owned-controlled iterator state {id} is not recorded")
                        })?;
                        if slot(row.controller)? != caller {
                            continue;
                        }
                    }
                    out.push(Scope::State(id));
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
                    Scope::Country(caller)
                    | Scope::CountryLeader {
                        country: caller, ..
                    } => caller,
                    _ if !global => return Ok(Vec::new()),
                    _ => 0,
                };
                let mut out = Vec::new();
                let invisible = field(children(item), "include_invisible").is_some_and(|field| {
                    field
                        .value
                        .scalar()
                        .and_then(content_reader_parser::parser::read_bool)
                        == Some(true)
                });
                if !operative {
                    let repeats = if global {
                        let mut count = 0;
                        for row in each(
                            frame
                                .complete::<tables::Countries>(COUNTRY_SLOT)
                                .map_err(|why| why.to_string())?,
                        ) {
                            let row = row?;
                            if row.id as i32 > 0
                                && row.canonical_slot != caller
                                && world
                                    .exists(row.canonical_slot)
                                    .map_err(|why| why.to_string())?
                            {
                                count += 1;
                            }
                        }
                        count
                    } else {
                        1
                    };
                    if repeats == 0 {
                        return Ok(out);
                    }
                    let inputs = world
                        .country_counter_inputs(caller)
                        .map_err(|why| why.to_string())?;
                    for _ in 0..repeats {
                        for (enabled, recorded, name) in [
                            (
                                army,
                                &inputs.army_leaders,
                                "army leaders of country counter inputs",
                            ),
                            (
                                navy,
                                &inputs.navy_leaders,
                                "navy leaders of country counter inputs",
                            ),
                        ] {
                            if !enabled {
                                continue;
                            }
                            let leaders = recorded
                                .require(name)?
                                .as_deref()
                                .ok_or_else(|| format!("{name} capture unavailable"))?;
                            for &reference in leaders {
                                let row = world
                                    .unit_leader(reference)
                                    .map_err(|why| why.to_string())?
                                    .ok_or_else(|| {
                                        format!(
                                            "iterator unit leader {reference:#x} is not recorded"
                                        )
                                    })?;
                                if invisible || self.evaluator()?.leader_visible(&*row).decided()? {
                                    out.push(Scope::Leader(reference));
                                }
                            }
                        }
                    }
                    return Ok(out);
                }
                let mut members = Vec::new();
                for member in frame
                    .complete::<tables::OperativeMemberships>(MEMBERSHIP)
                    .map_err(|why| why.to_string())?
                    .keyed(|key| key as u8 == 0 && world.slot((key >> 8) as u32) == Some(caller))
                {
                    members.push(member.map_err(built)?);
                }
                members.sort_by_key(|member| {
                    member
                        .ordinal
                        .require("ordinal of operative memberships")
                        .ok()
                        .copied()
                });
                for member in members {
                    world
                        .unit_leader(member.id)
                        .map_err(|why| why.to_string())?
                        .ok_or_else(|| {
                            format!("iterator operative {:#x} is not recorded", member.id)
                        })?;
                    out.push(Scope::Leader(member.id));
                }
                Ok(out)
            }
            Roster::IndustrialOrganisations => {
                let (Scope::Country(caller)
                | Scope::CountryLeader {
                    country: caller, ..
                }) = scopes.this
                else {
                    return Ok(Vec::new());
                };
                if caller == 0 {
                    return Ok(Vec::new());
                }
                let invisible = field(children(item), "include_invisible").is_some_and(|field| {
                    field
                        .value
                        .scalar()
                        .and_then(content_reader_parser::parser::read_bool)
                        == Some(true)
                });
                let mut rows = Vec::new();
                for row in each(
                    frame
                        .complete::<tables::IndustrialOrganisations>(ORGANISATION)
                        .map_err(|why| why.to_string())?,
                ) {
                    let row = row?;
                    if slot(row.country)? == caller {
                        rows.push(row);
                    }
                }
                rows.sort_by_key(|row| row.ordinal);
                let mut out = Vec::new();
                for row in rows {
                    if invisible || self.organisation_visible(&row)? {
                        out.push(Scope::IndustrialOrganisation(row.organisation));
                    }
                }
                Ok(out)
            }
            Roster::States => {
                frame
                    .complete::<tables::States>(0)
                    .map_err(|why| why.to_string())?;
                Ok(world
                    .states()
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
                let slots = world.country_slots().map_err(|why| why.to_string())?;
                let caller = if other || enemy {
                    Some(country()?)
                } else {
                    None
                };
                let mut enemies = std::collections::BTreeSet::new();
                if enemy {
                    for key in frame
                        .complete::<tables::WarRelations>(0)
                        .map_err(|why| why.to_string())?
                        .keys()
                    {
                        let first = slot((key >> 32) as u32)?;
                        let second = slot(key as u32)?;
                        if Some(first) == caller {
                            enemies.insert(second);
                        }
                        if Some(second) == caller {
                            enemies.insert(first);
                        }
                    }
                }
                let mut out = Vec::new();
                for target in slots {
                    if target == 0
                        || (!possible && !world.exists(target).map_err(|why| why.to_string())?)
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
                let Some(faction) = world.faction(caller).map_err(|why| why.to_string())? else {
                    return Ok(if allied {
                        Vec::new()
                    } else {
                        vec![Scope::Country(caller)]
                    });
                };
                let mut rows = Vec::new();
                for row in each(
                    frame
                        .complete::<tables::FactionMembers>(FACTION_MEMBER)
                        .map_err(|why| why.to_string())?,
                ) {
                    let row = row?;
                    if row.faction == faction {
                        rows.push(row);
                    }
                }
                rows.sort_by_key(|row| row.ordinal);
                let mut out = Vec::new();
                for row in rows {
                    let target = slot(row.country)?;
                    if (!allied || target != caller)
                        && world.exists(target).map_err(|why| why.to_string())?
                    {
                        out.push(Scope::Country(target));
                    }
                }
                Ok(out)
            }
            Roster::Subjects => {
                let caller = country()?;
                let mut out = Vec::new();
                for &raw in world
                    .appearance(caller)
                    .map_err(|why| why.to_string())?
                    .subjects
                    .require("subjects of country appearance")?
                {
                    if raw <= 0 {
                        continue;
                    }
                    let target = slot(raw as u32)?;
                    if world.exists(target).map_err(|why| why.to_string())? && target != caller {
                        out.push(Scope::Country(target));
                    }
                }
                Ok(out)
            }
            Roster::Characters => Ok(world
                .country_characters(country()?)
                .map_err(|why| why.to_string())?
                .into_iter()
                .map(|id| Scope::Character(id.unwrap_or(0)))
                .collect()),
            Roster::Scientists { .. } => {
                let caller = country()?;
                let mut rows = Vec::new();
                for row in each(
                    frame
                        .complete::<tables::Scientists>(SCIENTIST)
                        .map_err(|why| why.to_string())?,
                ) {
                    let row = row?;
                    if slot(row.country)? == caller {
                        rows.push(row);
                    }
                }
                rows.sort_by_key(|row| row.ordinal);
                Ok(rows
                    .into_iter()
                    .map(|row| Scope::Character(row.character))
                    .collect())
            }
            Roster::PartyLeaders => {
                let mut out = Vec::new();
                if !self
                    .limit(children(item))?
                    .iter()
                    .any(|item| item.keyword_is("has_ideology"))
                {
                    return Ok(out);
                }
                for party in world
                    .party_leaders(country()?)
                    .map_err(|why| why.to_string())?
                {
                    if let Some(first) = party.leaders.iter().min_by_key(|leader| leader.ordinal) {
                        if first.ordinal != 0 {
                            return Err("party leaders lacks ordinal zero".into());
                        }
                        if let Some(id) = first.character {
                            out.push(Scope::Character(id));
                        }
                    }
                }
                Ok(out)
            }
            Roster::CountryNeighbors | Roster::OccupiedCountries => {
                let (Scope::Country(caller)
                | Scope::CountryLeader {
                    country: caller, ..
                }) = scopes.this
                else {
                    return Ok(Vec::new());
                };
                let ours = |key: u128| world.slot(key as u32) == Some(caller);
                let neighbors;
                let occupied;
                let refs = if matches!(source, Roster::CountryNeighbors) {
                    neighbors = frame
                        .complete::<tables::CountryCounterInputs>(NEIGHBORS)
                        .map_err(|why| why.to_string())?
                        .keyed(ours)
                        .next()
                        .transpose()
                        .map_err(built)?
                        .ok_or("country counter inputs lacks neighbor roster")?;
                    neighbors
                        .neighbors
                        .require("neighbors of country counter inputs")?
                        .as_ref()
                        .ok_or("neighbors of country counter inputs capture refused")?
                } else {
                    occupied = frame
                        .complete::<tables::OccupationSettings>(OCCUPIED)
                        .map_err(|why| why.to_string())?
                        .keyed(ours)
                        .next()
                        .transpose()
                        .map_err(built)?
                        .ok_or("occupation settings lacks occupied roster")?;
                    occupied
                        .occupied
                        .require("occupied of occupation settings")?
                        .as_ref()
                        .ok_or("occupied of occupation settings capture refused")?
                };
                refs.iter()
                    .map(|raw| slot(*raw).map(Scope::Country))
                    .collect()
            }
            Roster::CountryDivisions => {
                let (Scope::Country(caller)
                | Scope::CountryLeader {
                    country: caller, ..
                }) = scopes.this
                else {
                    return Ok(Vec::new());
                };
                if caller == 0 {
                    return Ok(Vec::new());
                }
                let status = frame
                    .complete::<tables::DivisionStatus>(COUNTRY_ORDINAL)
                    .map_err(|why| why.to_string())?;
                let mut rows = Vec::new();
                for row in each(
                    frame
                        .complete::<tables::Divisions>(ROSTER_OWNER)
                        .map_err(|why| why.to_string())?,
                ) {
                    let row = row?;
                    if slot(row.roster_owner)? != caller {
                        continue;
                    }
                    let ordinal = keyed(&status, u128::from(row.id))?
                        .ok_or("division status lacks country ordinal")?
                        .country_ordinal
                        .require("country ordinal of division status")?
                        .ok_or("country ordinal of division status capture refused")?;
                    rows.push((ordinal, row.id));
                }
                rows.sort_by_key(|row| row.0);
                if rows
                    .iter()
                    .enumerate()
                    .any(|(index, row)| row.0 as usize != index)
                {
                    return Err(
                        "country ordinal of division status is not a complete country roster".into(),
                    );
                }
                Ok(rows
                    .into_iter()
                    .map(|(_, unit)| Scope::Unit(unit))
                    .collect())
            }
            Roster::StateDivisions => {
                let Scope::State(state) = scopes.this else {
                    return Ok(Vec::new());
                };
                if state == 0 {
                    return Ok(Vec::new());
                }
                frame
                    .complete::<tables::States>(0)
                    .map_err(|why| why.to_string())?;
                if !world.states().contains_key(&state) {
                    return Ok(Vec::new());
                }
                let order = frame
                    .complete::<tables::ProvinceUnitOrder>(0)
                    .map_err(|why| why.to_string())?;
                let armies = frame
                    .complete::<tables::Divisions>(0)
                    .map_err(|why| why.to_string())?;
                let mut out = Vec::new();
                for &province in world.state_order(state).map_err(|why| why.to_string())? {
                    let mixed = order
                        .u64s(u128::from(province), |row| &row.mixed, MIXED)
                        .map_err(built)?
                        .ok_or_else(|| {
                            format!("province unit order lacks state {state} province {province}")
                        })?;
                    for &unit in mixed {
                        if armies.contains(u128::from(unit)) {
                            out.push(Scope::Unit(unit));
                        } else if frame
                            .complete::<tables::TaskForces>(0)
                            .ok()
                            .is_some_and(|rows| rows.contains(u128::from(unit)))
                            || frame
                                .complete::<tables::RailwayGunCounterInputs>(0)
                                .ok()
                                .is_some_and(|rows| rows.contains(u128::from(unit)))
                        {
                        } else {
                            return Err(format!(
                                "state iterator unit {unit:#x} has no recorded class census"
                            ));
                        }
                    }
                }
                Ok(out)
            }
            Roster::Neighbors => {
                let Scope::State(state) = scopes.this else {
                    return Ok(Vec::new());
                };
                let neighbors = world.neighbor_states().map_err(str::to_owned)?;
                Ok(neighbors
                    .get(&state)
                    .into_iter()
                    .flatten()
                    .copied()
                    .map(Scope::State)
                    .collect())
            }
            Roster::OriginalTags => {
                let operand = field(children(item), "original_tag")
                    .and_then(|field| field.value.scalar())
                    .unwrap_or(b"THIS");
                let Some(target) = self
                    .evaluator()?
                    .scoped_country(operand, scopes)
                    .map_err(|why| why.to_string())?
                else {
                    return Ok(Vec::new());
                };
                let selected = world
                    .original_reference(target)
                    .map_err(|why| why.to_string())?;
                if selected <= 0 {
                    return Ok(Vec::new());
                }
                let recorded = world.recorded();
                let mut out = Vec::new();
                for raw in recorded
                    .original_bucket(selected)
                    .map_err(|why| why.to_string())?
                    .iter()
                    .copied()
                    .chain(std::iter::once(selected as u32))
                {
                    if (raw as i32) <= 0 {
                        continue;
                    }
                    let slot = recorded.raw_slot(raw).map_err(|why| why.to_string())?;
                    if world.exists(slot).map_err(|why| why.to_string())? {
                        out.push(Scope::Country(slot));
                    }
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
        scope.random_seed = scope.random_seed.wrapping_add(1);
        Ok(draw)
    }

    fn targets(
        &mut self,
        roster: Vec<Scope>,
        items: &[Item],
        scopes: Scopes,
        active_scientist: bool,
    ) -> Result<Vec<Scope>, String> {
        let mut out = Vec::new();
        for target in roster {
            let selected = self
                .evaluator()?
                .with_this(scopes, target)
                .map_err(|why| why.to_string())?;
            match self.condition(items, selected) {
                Eval::True => {
                    if active_scientist {
                        let Scope::Character(character) = target else {
                            return Err("active scientist target is not a character".into());
                        };
                        let row = keyed(
                            &self.complete::<tables::Scientists>(SCIENTIST_ACTIVE)?,
                            u128::from(character),
                        )?
                        .ok_or("Scientists lacks active iterator target")?;
                        if row
                            .active
                            .require("active of scientists")?
                            .ok_or("active of scientists capture refused")?
                            == 0
                        {
                            continue;
                        }
                    }
                    out.push(target);
                }
                Eval::False => {}
                Eval::Unknown(why) => return Err(why.into_iter().collect::<Vec<_>>().join("; ")),
            }
        }
        Ok(out)
    }

    fn label(&self, scope: Scope, caller: Scopes, list: bool) -> Result<String, String> {
        let text = match scope {
            Scope::Country(0) | Scope::CountryLeader { country: 0, .. } => String::new(),
            Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                if list {
                    let name = self.country_name(country, false)?;
                    let tag = self.tag(country)?;
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
                self.context.text.country_marked_name(country)?
            }
            Scope::State(0) => String::new(),
            Scope::State(id) => self.state_name(id)?,
            Scope::Character(0) => String::new(),
            Scope::Character(id) => self.character_label(id)?,
            Scope::IndustrialOrganisation(reference) => self.organisation_name(reference)?,
            Scope::Leader(id) => {
                let leader = self
                    .context
                    .world()?
                    .unit_leader(id)
                    .map_err(|why| why.to_string())?
                    .ok_or("unit leaders lacks iterator target")?;
                String::from_utf8_lossy(&leader.inherited_text_a).into_owned()
            }
            _ => return Err("iterator target label kind is unported".into()),
        };
        Ok(if list {
            format!("\u{11}Y{text}\u{11}!")
        } else {
            text
        })
    }

    fn character_label(&self, id: u32) -> Result<String, String> {
        if id == 0 {
            return Ok(String::new());
        }
        let row = self
            .context
            .world()?
            .character(id)
            .map_err(|why| why.to_string())?
            .ok_or("Characters lacks scope target")?;
        self.effect_character_name(&row)
    }

    fn organisation_scopes(
        &self,
        row: &replay_data::rows::IndustrialOrganisationRow,
    ) -> Result<Scopes, String> {
        use crate::scripted_text::{EventObject, EventScope, Link};
        let owner = self
            .context
            .world()?
            .slot(row.owner)
            .ok_or("country links lacks industrial organisation owner")?;
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
        self.evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())
    }

    fn organisation_visible(
        &self,
        row: &replay_data::rows::IndustrialOrganisationRow,
    ) -> Result<bool, String> {
        let world = self.context.world()?;
        let token = world
            .token(row.token)
            .map_err(|why| why.to_string())?
            .ok_or("lexical tokens lacks industrial organisation token")?;
        let database = self
            .context
            .content
            .industrial_organisations
            .ok_or_else(|| self.content_gap("industrial organisation visibility"))?;
        let template = database.template(token)?;
        let evaluator = self.evaluator()?;
        let visible = if !evaluator
            .has_children(&template.visible)
            .map_err(|why| why.to_string())?
            && !template.delete_visible
        {
            match &template.include {
                Some(parent) => database.template(parent)?.visible.as_slice(),
                None => &template.visible,
            }
        } else {
            &template.visible
        };
        if !evaluator
            .has_children(visible)
            .map_err(|why| why.to_string())?
        {
            return Ok(true);
        }
        evaluator
            .evaluate(visible, self.organisation_scopes(row)?)
            .decided()
    }

    pub(super) fn organisation_name(&self, reference: u64) -> Result<String, String> {
        let world = self.context.world()?;
        let row = world
            .industrial_organisation(reference)
            .map_err(|why| why.to_string())?;
        let name = if row.name.is_empty() {
            world
                .token(row.token)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks industrial organisation name")?
        } else {
            row.name.as_slice()
        };
        let name =
            std::str::from_utf8(name).map_err(|_| "industrial organisation name is not UTF-8")?;
        let owner = world
            .slot(row.owner)
            .ok_or("country links lacks industrial organisation owner")?;
        let tagged = format!("{}_{name}", self.tag(owner)?);
        let key = if owner > 0 && self.context.localisation.get(&tagged).is_some() {
            tagged.as_bytes()
        } else {
            name.as_bytes()
        };
        self.localised(key, &[], self.organisation_scopes(&row)?, true)
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
        let mut unordered_operatives = Vec::new();
        if roster.len() >= 2
            && matches!(
                kind.roster,
                Roster::Leaders {
                    operative: true,
                    ..
                }
            )
        {
            let world = self.context.world()?;
            let caller = self.current_country(scopes)?;
            for member in world
                .tables()
                .complete::<tables::OperativeMemberships>(MEMBERSHIP)
                .map_err(|why| why.to_string())?
                .keyed(|key| key as u8 == 0 && world.slot((key >> 8) as u32) == Some(caller))
            {
                let member = member.map_err(built)?;
                if member.ordinal.require("ordinal of operative memberships").is_err() {
                    unordered_operatives.push(Scope::Leader(member.id));
                }
            }
        }
        let order_checkpoint = if unordered_operatives.is_empty() {
            None
        } else {
            let evaluator = self.evaluator()?;
            Some((evaluator.variable_effects(), evaluator.variable_session()))
        };
        let mut targets = Vec::new();
        if kind.random
            && matches!(
                kind.roster,
                Roster::States | Roster::Neighbors | Roster::CountryStates { .. }
            )
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
                false,
            )?;
        }
        if targets.is_empty() {
            let selected = self.targets(
                roster,
                items,
                scopes,
                matches!(kind.roster, Roster::Scientists { active: true }),
            );
            if selected.is_err()
                && let Some((_, session)) = &order_checkpoint
            {
                self.evaluator()?.restore_variable_session(session);
            }
            targets = selected?;
        }
        if let Some((effects, session)) = order_checkpoint {
            let evaluator = self.evaluator()?;
            if effects != evaluator.variable_effects()
                || targets.len() >= 2
                    && targets
                        .iter()
                        .any(|target| unordered_operatives.contains(target))
            {
                evaluator.restore_variable_session(&session);
                return Err(
                    "ordinal of operative memberships not recorded for limit/selection order".into(),
                );
            }
        }
        let body = self.effect_children(item);
        if !kind.random
            && let Some(amount) =
                field(items, "random_select_amount").and_then(|field| field.value.scalar())
        {
            let amount = integer(amount);
            if amount < 0 {
                return Err("negative random_select_amount underflows target removal".into());
            }
            while targets.len() > amount as usize {
                let at = self.draw(scopes)? as usize % targets.len();
                targets.swap_remove(at);
            }
        }
        let visible = field(items, "visible_when_empty").is_some_and(|field| {
            field
                .value
                .scalar()
                .and_then(content_reader_parser::parser::read_bool)
                == Some(true)
        });
        if targets.is_empty() && !visible {
            return Ok(String::new());
        }
        let tooltip = field(items, "tooltip")
            .filter(|field| !field.value.scalar().is_some_and(|key| key.is_empty()));
        if kind.random {
            let target = if matches!(kind.roster, Roster::PartyLeaders) {
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
                .with_this(scopes, target.unwrap_or(scopes.this))
                .map_err(|why| why.to_string())?;
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
                let mut text = format!("(\u{11}Y{label}\u{11}!): ");
                let before = self.out.refusals.clone();
                let probe = self.group(&body, selected, 0);
                if probe.bytes().filter(|byte| *byte == b'\n').count() < 2 {
                    text.push_str(&probe);
                } else {
                    self.out.refusals = before;
                    text.push('\n');
                    text.push_str(&self.group(&body, selected, indent + 1));
                }
                text
            };
            return Ok(indent_nonempty_lines(&text, indent));
        }
        let individual = field(items, "display_individual_scopes").is_some_and(|field| {
            field
                .value
                .scalar()
                .and_then(content_reader_parser::parser::read_bool)
                == Some(true)
        });
        let mut text = String::new();
        let mut selected = self
            .evaluator()?
            .with_this(scopes, scopes.this)
            .map_err(|why| why.to_string())?;
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
                selected = self
                    .evaluator()?
                    .repoint_this(selected, target)
                    .map_err(|why| why.to_string())?;
                text.push_str(&format!("({}):\n", self.label(target, scopes, true)?));
                text.push_str(&self.group(&body, selected, indent + 1));
            }
        } else {
            selected = self
                .evaluator()?
                .repoint_this(selected, targets[0])
                .map_err(|why| why.to_string())?;
            text.push_str(&self.group(&body, selected, indent + 1));
        }
        Ok(indent_nonempty_lines(&text, indent))
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
            .scoped_array_scopes(operand, scopes)
            .map_err(|why| why.to_string())?
            .into_iter()
            .flatten()
            .collect();
        let targets = self.targets(roster, children(item), scopes, false)?;
        if targets.is_empty() {
            return Ok(String::new());
        }
        let target = targets[self.draw(scopes)? as usize % targets.len()];
        let selected = self
            .evaluator()?
            .with_this(scopes, target)
            .map_err(|why| why.to_string())?;
        let label = self.label(target, scopes, false)?;
        let mut text = format!("\u{11}H{label}:\n\u{11}!");
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
        let targets = self.targets(roster, children(item), scopes, false)?;
        let world = self.context.world()?;
        let tables = crate::character_scope::Tables::read_in(world.tables());
        for &target in &targets {
            let id = match target {
                Scope::Character(id) => id,
                _ => continue,
            };
            if let Some(leader) = tables.unit_leader(id).map_err(|why| why.to_string())?
                && leader.category() < 3
            {
                return Err(
                    "character_list_tooltip army/navy leader command-size inputs are not recorded"
                        .into(),
                );
            }
        }
        fn whole<T: replay_data::rows::TableSchema>(
            frame: &impl replay_frame::frame::Tables,
        ) -> Result<Vec<T::Row>, String>
        where
            T::Row: Clone,
        {
            frame
                .complete_rows::<T>(T::ID.full_mask())
                .map_err(|why| why.to_string())?
                .iter()
                .map(|row| row.map(std::borrow::Cow::into_owned).map_err(|why| why.to_string()))
                .collect()
        }
        let frame = world.tables();
        let roles = whole::<tables::AdvisorRoles>(&frame)?;
        let traits = whole::<tables::CharacterRoleTraits>(&frame)?;
        let hired = whole::<tables::HiredAdvisors>(&frame)?;
        let country = self.current_country(scopes)?;
        let mut text = String::new();
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
                                && row.kind == replay_data::rows::CharacterRoleTraitRow::ADVISOR
                                && row.role == role.ordinal
                        })
                    {
                        for token in &row.tokens {
                            names.push(
                                world
                                    .token(*token)
                                    .map_err(|why| why.to_string())?
                                    .ok_or("lexical tokens lacks advisor trait")?,
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
                for row in traits.iter().filter(|row| {
                    row.character == id
                        && row.kind == replay_data::rows::CharacterRoleTraitRow::ADVISOR
                }) {
                    for token in &row.tokens {
                        let name = world
                            .token(*token)
                            .map_err(|why| why.to_string())?
                            .ok_or("lexical tokens lacks advisor sort trait")?;
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
            }
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
        }
        Ok(text)
    }

    pub(super) fn set_politics(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let items = children(item);
        let mut text = self.line(b"EFFECT_SET_POLITICS", &[], scopes, false)?;
        if let Some(ideology) = field(items, "ruling_party").and_then(|item| item.value.scalar()) {
            let country = self.current_country(scopes)?;
            let ideology = self.politics_ideology(Some(ideology), scopes)?;
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
                                Argument::Text(&String::from_utf8_lossy(&ideology)),
                            ),
                            ("NAME", Argument::Text(&name)),
                        ],
                        scopes,
                        false,
                    )?);
                    name
                }
                None => self.selected_party_name(country, &ideology)?,
            };
            text.push_str(&self.line(
                b"EFFECT_SET_POLITICS_RULING_PARTY",
                &[("PARTY", Argument::Text(&name))],
                scopes,
                false,
            )?);
        }
        match field(items, "elections_allowed").and_then(|item| {
            item.value
                .scalar()
                .and_then(content_reader_parser::parser::read_bool)
        }) {
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
        Ok(text)
    }
}
