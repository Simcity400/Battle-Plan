//! Recorded joins for the three ordinary-map tooltip entry points.
use super::super::{
    RaidAssets, TypeToken, arrows, launch, offers, paths,
    script::{Eval, Scope, Scopes, World},
    target_country,
};
use super::{Key, Tooltip, chance, intel, rules, success};
use crate::combat::text::localise;
use dashboard::scripted_text::{EventObject, EventScope, EventScopes, Link, ScopeVariables};
use installed_content::localisation::Argument as Arg;
use installed_content::{localisation::Localisation, raids::RaidType};
use replay::{
    frame::DecodedFrame,
    rows::{RaidRow, RaidScope, RaidTargetRow, tables},
};

type Result<T> = std::result::Result<T, String>;
#[path = "diagnostic.rs"]
mod diagnostic;
#[path = "trigger.rs"]
mod trigger;

/// The tooltip of `key` for country `ours` in one recorded hour, from the
/// document's content.
pub fn build(
    content: &crate::document::Content,
    frame: &DecodedFrame,
    ours: u32,
    key: &Key,
) -> Tooltip {
    let (assets, combat) = (&content.icons.raids, &content.icons.combat);
    let loc = &content.names.localisation;
    let world = match World::new(frame, assets.facts()) {
        Ok(world) => world,
        Err(reason) => {
            let mut out = Tooltip::default();
            out.refuse("raid tooltip", reason);
            return out;
        }
    };
    let flag_exists = |key: &str| combat.has_flag(key);
    let context = Context {
        assets,
        world: &world,
        loc,
        recorded_language: &content.recorded_language,
        scripted_content: &content.scripted,
        ours,
        flag_exists: &flag_exists,
        scripted: Default::default(),
        countries: installed_content::country_names::CountryNames::read(frame, loc),
    };
    let result = match key {
        Key::Raid {
            id,
            reference_type,
            incoming,
        } => context.raid(*id, *reference_type, *incoming),
        Key::Offer(targets) => Ok(context.offers(targets)),
    };
    result.unwrap_or_else(|reason| {
        let mut out = Tooltip::default();
        out.refuse("raid tooltip", reason);
        out
    })
}

struct Context<'a, 'w> {
    assets: &'a RaidAssets,
    world: &'a World<'w>,
    loc: &'a Localisation,
    /// The recording client's language
    /// ([`dashboard::scripted_text::stored_text`]).
    recorded_language: &'a str,
    /// What custom trigger tooltips' scripted text reads.
    scripted_content: &'a Result<dashboard::scripted_text::ScriptedContent>,
    ours: u32,
    countries: installed_content::country_names::CountryNames<'a>,
    flag_exists: &'a dyn Fn(&str) -> bool,
    scripted: std::cell::OnceCell<dashboard::scripted_text::ScriptedText<'a>>,
}

fn known(eval: Eval) -> Result<bool> {
    match eval {
        Eval::True => Ok(true),
        Eval::False => Ok(false),
        Eval::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
    }
}

/// The CEventScopes a custom trigger tooltip's text is expanded in.
///
/// The raid condition chain starts
/// in a default-constructed scope (140528800) whose FROM, when set, is
/// another default-constructed scope of the target (140A8D3F0 ->
/// 141580600), so every link of both is known. Inside a scope change (a
/// PREV is set, or THIS is not ROOT) only the linked scopes' objects are
/// known, not their own links.
fn event_scopes(scopes: Scopes) -> EventScopes {
    let target = |scope| match scope {
        Scope::Country(slot) => EventObject::Country(slot),
        Scope::State(id) => EventObject::State(id),
        Scope::Province(id) => EventObject::Province(id),
        Scope::Leader(reference) => EventObject::UnitLeader(reference),
        Scope::Character(id) => EventObject::Character(id),
        Scope::Combat => unreachable!("a raid condition is never checked in a combat scope"),
    };
    let unknown = |scope| EventScope {
        this: target(scope),
        root: Link::Unknown,
        from: Link::Unknown,
        prev: Link::Unknown,
    };
    let top =
        scopes.prev.is_none() && scopes.this == scopes.root && scopes.unknown == Default::default();
    let list = if top {
        let own = EventScope::of(target(scopes.this));
        match scopes.from {
            Some(from) => vec![
                EventScope {
                    from: Link::Scope(1),
                    ..own
                },
                EventScope::of(target(from)),
            ],
            None => vec![own],
        }
    } else {
        let mut linked = vec![unknown(scopes.root)];
        let mut link = |scope: Option<Scope>, unset: bool| match scope {
            Some(scope) if !unset => {
                linked.push(unknown(scope));
                Link::Scope(linked.len())
            }
            _ => Link::Unknown,
        };
        let prev = link(scopes.prev, scopes.unknown.prev);
        let from = link(scopes.from, scopes.unknown.from);
        let current = EventScope {
            this: target(scopes.this),
            root: if scopes.unknown.root {
                Link::Unknown
            } else {
                Link::Scope(1)
            },
            from,
            prev,
        };
        std::iter::once(current).chain(linked).collect()
    };
    // The target variables stay set while the condition's text is built
    // (141580600 sets them for the raid scope's whole evaluation).
    EventScopes {
        scopes: list,
        variables: ScopeVariables {
            target_state: scopes.target_state,
            target_province: scopes.target_province,
            target_leader: scopes.target_leader,
        },
    }
}

impl Context<'_, '_> {
    fn scoped_text(&self, key: &str, scopes: Scopes) -> Result<String> {
        let text = localise(self.loc, key, &[])?;
        if !text.contains('[') {
            return Ok(text);
        }
        let content = self.scripted_content.as_ref().map_err(Clone::clone)?;
        let scripted = self.scripted.get_or_init(|| {
            dashboard::scripted_text::ScriptedText::new(
                self.world.frame(),
                self.loc,
                content,
                self.ours,
            )
        });
        scripted.expand_in(&text, &event_scopes(scopes))
    }

    fn type_of(&self, token: u32) -> Result<&RaidType> {
        match self.assets.raid_type(self.world, token)? {
            TypeToken::Type(at) => Ok(&self.assets.catalog.types[at]),
            TypeToken::Refused(at) => Err(format!(
                "raid type {} is refused",
                self.assets.catalog.uncertain[at].name
            )),
        }
    }
    fn type_name(&self, kind: &RaidType) -> Result<String> {
        // GetLocalizedName@140A8E3B0; prefix bytes at 14292A768.
        localise(self.loc, &format!("raid_type_{}", kind.name), &[])
    }
    fn state_name(&self, state: u32) -> Result<String> {
        let row = self
            .world
            .frame()
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.id == state)
            .ok_or("target state is absent")?;
        let (custom, owner) = (&row.custom_name, &row.owner_name);
        crate::combat::text::state_name(
            self.loc,
            self.recorded_language,
            state,
            custom,
            owner,
            || {
                let key = self
                    .assets
                    .tooltip
                    .state_names
                    .get(&state)
                    .ok_or("state has no content name")?
                    .as_ref()
                    .map_err(|reason| format!("state {state}'s name key: {reason}"))?;
                localise(self.loc, key, &[])
            },
        )
    }
    fn victory_point(&self, province: u32) -> Result<Option<String>> {
        // ResolveVictoryPointLocalization@140E68E30 is used directly here,
        // without the map label's province-override rung.
        let row = self
            .world
            .frame()
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.id == province)
            .ok_or("target province is absent")?;
        let owner = self
            .world
            .frame()
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|s| s.id == row.state)
            .and_then(|s| self.world.slot(s.owner));
        let tag = owner
            .map(|slot| {
                self.countries
                    .operands(slot)
                    .ok_or_else(|| format!("state owner {slot} has no naming operands"))?
                    .as_ref()
                    .map(|o| o.tag.as_str())
                    .map_err(Clone::clone)
            })
            .transpose()?;
        crate::map::labels::victory_point_keys(tag, province)
            .into_iter()
            .find(|key| self.loc.get(key).is_some())
            .map(|key| localise(self.loc, &key, &[]))
            .transpose()
            .map(|value| value.filter(|s| !s.is_empty()))
    }
    fn location(&self, scope: &RaidScope) -> Result<String> {
        let province = if scope.province != 0 {
            scope.province
        } else {
            scope.leader_province
        };
        if province != 0
            && let Some(name) = self.victory_point(province)?
        {
            return Ok(name);
        }
        match intel::target_state(self.world, scope)? {
            Some(state) => self.state_name(state),
            None => Ok("no_target_name".into()), // literal at 14296C780, 140FD1BB0.
        }
    }
    fn target_name(&self, scope: &RaidScope, kind: Option<&RaidType>) -> Result<String> {
        let location = self.location(scope)?;
        if let Some(kind) = kind {
            if let Some(reason) = &kind.tooltip.refused {
                return Err(reason.clone());
            }
            let custom = &kind.tooltip.target_loc_key;
            if !custom.is_empty() && self.loc.get(custom).is_some() {
                return localise(self.loc, custom, &[("LOCATION", Arg::Text(&location))]);
            }
        }
        if scope.building_type != 0 {
            let building = self.assets.building(self.world, scope.building_type)?;
            return localise(
                self.loc,
                "RAID_TARGET_NAME",
                &[
                    ("WHAT", Arg::Text(&localise(self.loc, &building.name, &[])?)),
                    ("LOCATION", Arg::Text(&location)),
                ],
            );
        }
        let province = if scope.province != 0 {
            scope.province
        } else {
            scope.leader_province
        };
        if province != 0 {
            if self.victory_point(province)?.is_some() {
                return localise(
                    self.loc,
                    "raid_target_name_victory_point",
                    &[("VICTORY_POINT", Arg::Text(&location))],
                );
            }
            let terrain = self
                .assets
                .terrain
                .as_ref()
                .ok_or("province movement content unavailable")?
                .map
                .terrain(province)
                .ok_or("target province terrain unavailable")?;
            let index = usize::from(terrain.index)
                .checked_sub(1)
                .ok_or("target province has null terrain")?;
            let name = self
                .assets
                .tooltip
                .terrain_names
                .as_ref()?
                .get(index)
                .ok_or("target terrain ordinal absent")?;
            // 140FD1D50 calls CGameItem::id_copy@140616BC0, which localises
            // the category token; NAME is the plain location.
            return localise(
                self.loc,
                "PROV_TOOLTIP_LOCATION_STATE",
                &[
                    ("TERRAIN", Arg::Text(&localise(self.loc, name, &[])?)),
                    ("NAME", Arg::Text(&location)),
                ],
            );
        }
        Ok(location)
    }
    fn description(&self, scope: &RaidScope, kind: &RaidType, ended: bool) -> Result<String> {
        let target = if ended {
            self.location(scope)?
        } else {
            self.target_name(scope, Some(kind))?
        };
        // String arguments TYPE@142734318 and TARGET@1427528E4.
        localise(
            self.loc,
            "raid_inst_item_desc",
            &[
                ("TYPE", Arg::Text(&self.type_name(kind)?)),
                ("TARGET", Arg::Text(&target)),
            ],
        )
    }
    fn progress(
        &self,
        raid: &RaidRow,
        target: &RaidTargetRow,
        kind: &RaidType,
    ) -> Result<Option<i64>> {
        let (elapsed, remaining) = match raid.phase {
            2 => {
                if let Some(reason) = &kind.tooltip.refused {
                    return Err(reason.clone());
                }
                let days = if kind.tooltip.days_to_prepare > 0 {
                    kind.tooltip.days_to_prepare
                } else {
                    *self.assets.tooltip.prepare_days.as_ref()?
                };
                (
                    raid.preparation_hours,
                    days.wrapping_mul(24).wrapping_sub(raid.preparation_hours),
                )
            }
            4 => {
                let (travelled, inputs) = arrows::plane_inputs(self.world.frame(), raid)?;
                let branch = arrows::SpeedBranch::of(kind, &|name| {
                    let equipment = &self.assets.equipment;
                    equipment.named(name).map(|kind| equipment.category(kind))
                })?;
                let speed = arrows::raid_unit_speed(
                    branch,
                    &inputs,
                    kind.speed_multiplier_raw,
                    self.assets.tooltip.speed.as_ref()?,
                );
                let distance =
                    arrows::target_distance(self.assets, self.world.frame(), raid, target)?;
                (
                    arrows::elapsed_hours(travelled, speed),
                    arrows::remaining_hours(distance, travelled, speed),
                )
            }
            _ => (0, 0),
        };
        Ok((elapsed.wrapping_add(remaining) > 0)
            .then(|| arrows::phase_progress(elapsed, remaining)))
    }
    fn current_for(&self, scope: &RaidScope, viewer: u32) -> Result<i64> {
        let lane = if scope.building_type == 0 {
            0
        } else {
            let building = self.assets.building(self.world, scope.building_type)?;
            if let Some(reason) = &building.tooltip_refused {
                return Err(reason.clone());
            }
            building.detecting_intel_type
        };
        intel::current(
            self.assets,
            self.world,
            scope,
            viewer,
            lane,
            self.assets.tooltip.intel.as_ref()?,
        )
    }
    fn current(&self, scope: &RaidScope) -> Result<i64> {
        self.current_for(scope, self.ours)
    }
    /// `current` is the target's CURRENT, computed at most once per target.
    fn facility(
        &self,
        scope: &RaidScope,
        current: &std::cell::OnceCell<Result<i64>>,
    ) -> Result<rules::Facility> {
        if scope.building_type == 0 {
            return Ok(rules::Facility::None);
        }
        let building = self.assets.building(self.world, scope.building_type)?;
        if let Some(reason) = &building.tooltip_refused {
            return Err(reason.clone());
        }
        if building.specializations.is_empty() {
            return Ok(rules::Facility::None);
        }
        let current = current.get_or_init(|| self.current(scope)).clone()?;
        let required = self.assets.tooltip.intel.as_ref()?.required;
        if current < required {
            return Ok(rules::Facility::Unknown { current, required });
        }
        let owner = target_country(self.world, scope)?.ok_or("facility has no controller")?;
        let province = self
            .assets
            .province(scope)
            .ok_or("facility has no province")?;
        let rows = self
            .world
            .frame()
            .complete::<tables::RaidFacilities>()
            .map_err(|e| e.to_string())?;
        let program = rows
            .iter()
            .filter(|r| self.world.slot(r.country) == Some(owner) && r.province == province)
            .min_by_key(|r| r.ordinal);
        match program.and_then(|r| r.project_definition) {
            None => Ok(rules::Facility::Idle),
            Some(token) => {
                let key = self
                    .world
                    .token(token)?
                    .ok_or("ongoing project definition token is absent")?;
                let key = std::str::from_utf8(key).map_err(|e| e.to_string())?;
                Ok(rules::Facility::Project(localise(self.loc, key, &[])))
            }
        }
    }
    fn scopes(&self, scope: &RaidScope) -> Result<Scopes> {
        Ok(Scopes {
            root: Scope::Country(self.ours),
            this: Scope::Country(self.ours),
            prev: None,
            from: target_country(self.world, scope)?.map(Scope::Country),
            target_state: intel::target_state(self.world, scope)?,
            target_province: self.assets.province(scope),
            target_leader: (scope.leader_reference != 0).then_some(scope.leader_reference),
            unknown: Default::default(),
            bare: false,
        })
    }
    fn offers(&self, targets: &[offers::TooltipTarget]) -> Tooltip {
        let paths = paths::HourPaths::new(self.assets, self.world.frame());
        let launcher = launch::Launcher::new(self.assets, self.world, self.ours, &paths);
        let local = launcher.local();
        let flags = std::cell::OnceCell::new();
        let mut output = Vec::new();
        let mut quick = Ok(false);
        for target in targets {
            let scope = &target.scope;
            let mut types = Vec::new();
            let mut any = Eval::False;
            for &at in &target.types {
                let Some(kind) = self.assets.catalog.types.get(at) else {
                    return Tooltip {
                        refused: std::collections::BTreeMap::from([(
                            format!(
                                "raid offer type: cached index {at} is outside content catalog"
                            ),
                            1,
                        )]),
                        ..Default::default()
                    };
                };
                let scopes = self.scopes(scope);
                let allowed = scopes.as_ref().map_err(Clone::clone).and_then(|s| {
                    known(launcher.launchable_in(&local, kind, scope, *s, paths::Asked::Certain))
                });
                any = any.or(|| match &allowed {
                    Ok(b) => Eval::from_bool(*b),
                    Err(r) => Eval::unknown(r.clone()),
                });
                let launch_condition = scopes.as_ref().map_err(Clone::clone).and_then(|s| {
                    let (allowed, lines) = self.conditions(kind, scope, *s, true, None);
                    known(allowed).map(|allowed| (!allowed).then_some(lines))
                });
                let desc = format!("raid_type_{}_desc", kind.name);
                types.push(rules::Type {
                    name: self.type_name(kind),
                    allowed: allowed.clone(),
                    captive: self.captive(scope),
                    description: self
                        .loc
                        .get(&desc)
                        .map(|_| localise(self.loc, &desc, &[]))
                        .transpose(),
                    denial: if allowed == Ok(false) {
                        self.offer_denial(&launcher, &local, kind, scope, scopes)
                    } else {
                        Vec::new()
                    },
                    launch_condition,
                    range: kind.range_factor_raw,
                    once: kind.fire_only_once,
                });
            }
            let allowed = known(any);
            if targets.len() == 1 && target.types.len() == 1 {
                quick = allowed.clone();
            }
            let kind = if target.types.len() == 1 {
                self.assets.catalog.types.get(target.types[0])
            } else {
                None
            };
            let current = std::cell::OnceCell::new();
            let detection = (|| {
                if scope.building_type == 0
                    || !self
                        .assets
                        .building(self.world, scope.building_type)?
                        .need_detection
                {
                    return Ok(None);
                }
                current
                    .get_or_init(|| self.current(scope))
                    .clone()
                    .map(Some)
            })();
            let flag = target_country(self.world, scope).and_then(|country| match country {
                None => Ok(String::new()),
                Some(country) => self.flag(country, &flags),
            });
            output.push(rules::Target {
                name: self.target_name(scope, kind),
                flag,
                allowed,
                facility: self.facility(scope, &current),
                detection,
                types,
            });
        }
        rules::offers(self.loc, output, quick)
    }
    /// `flags` holds every country's candidates, built once per tooltip.
    fn flag(
        &self,
        country: u32,
        flags: &std::cell::OnceCell<Result<Vec<dashboard::products::country_flags::FlagChoice>>>,
    ) -> Result<String> {
        let choice = flags
            .get_or_init(|| {
                dashboard::products::country_flags::choices(self.world.frame())
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .find(|choice| self.world.slot(choice.raw) == Some(country))
            .ok_or("controller flag operands are absent")?;
        let key = choice
            .candidates
            .iter()
            .find(|key| (self.flag_exists)(key))
            .ok_or("controller has no resolved flag asset")?;
        Ok(format!("{}{key}", crate::combat::INLINE_FLAG))
    }
    fn captive(&self, scope: &RaidScope) -> Result<Option<String>> {
        if scope.leader_reference == 0 {
            return Ok(None);
        }
        let leaders = self
            .world
            .frame()
            .complete::<tables::UnitLeaders>()
            .map_err(|e| e.to_string())?;
        let leader = leaders
            .iter()
            .find(|r| {
                (u64::from(r.id) << 32 | u64::from(r.reference_type)) == scope.leader_reference
            })
            .ok_or("captive leader is absent from UnitLeaders")?;
        // 141612FC0 reads +40. Its sole naming setter 140C16FB0 localises
        // the saved +20 input, recorded as inherited_text_a (not script ID).
        localise(
            self.loc,
            std::str::from_utf8(&leader.inherited_text_a).map_err(|e| e.to_string())?,
            &[],
        )
        .map(Some)
    }

    fn raid(&self, id: u32, reference_type: u32, incoming: bool) -> Result<Tooltip> {
        let frame = self.world.frame();
        let raid = frame
            .complete::<tables::Raids>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|r| r.id == id && r.reference_type == reference_type)
            .ok_or("hovered raid is absent")?;
        let target = frame
            .complete::<tables::RaidTargets>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|r| r.id == id && r.reference_type == reference_type)
            .ok_or("hovered raid target is absent")?;
        let kind = self.type_of(target.raid_type)?;
        let ended = raid.phase == 5;
        let chance = if incoming && !ended {
            let inputs = success::Inputs::new(self.assets, self.world, raid, &target.scope);
            chance::evaluate(
                self.loc,
                &kind.tooltip.success,
                self.assets.tooltip.thresholds.clone(),
                |m| match &inputs {
                    Ok(inputs) => {
                        inputs.input(m, || self.current_for(&target.scope, raid.parent_slot))
                    }
                    Err(reason) => chance::Input {
                        raw: Err(reason.clone()),
                        skip: Ok(false),
                        defender_can_affect: Err(reason.clone()),
                    },
                },
            )
        } else {
            chance::Evaluation {
                success: Ok(0),
                counterplay: Vec::new(),
            }
        };
        Ok(rules::raid(
            self.loc,
            rules::Raid {
                ended,
                phase: raid.phase,
                description: self.description(&target.scope, kind, ended),
                outcome: raid.outcome,
                progress: self.progress(raid, target, kind),
                gate: if !incoming && matches!(raid.phase, 1 | 3) {
                    self.raid_diagnostics(raid, target, kind)
                } else {
                    (Ok(true), Vec::new())
                },
                assigned_unit: if ended || incoming {
                    Ok(None)
                } else {
                    super::names::Name::new(frame, raid)
                        .and_then(|name| name.assigned(&self.assets.tooltip, self.loc))
                },
                owner: self.countries.name(raid.parent_slot),
                success: chance.success,
                counterplay: Ok(chance.counterplay),
            },
            incoming,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{assets, frame};
    use super::*;
    #[test]
    fn facility_threshold_is_strict_and_first_program_controls_project_text() {
        use super::super::super::tests::put;
        use replay::rows::RaidFacilityRow;
        for (required, first_project) in [(1, Some(100)), (0, None), (0, Some(100))] {
            let mut assets = assets();
            assets
                .buildings
                .types
                .get_mut("dam")
                .unwrap()
                .specializations
                .push("test".into());
            assets.tooltip.intel = Ok(intel::Defines {
                required,
                ..Default::default()
            });
            let mut frame = frame();
            put::<tables::RaidFacilities>(
                &mut frame,
                vec![
                    RaidFacilityRow {
                        country: 11,
                        ordinal: 0,
                        province: 3,
                        project_definition: first_project,
                        ..Default::default()
                    },
                    RaidFacilityRow {
                        country: 11,
                        ordinal: 1,
                        province: 3,
                        project_definition: Some(101),
                        ..Default::default()
                    },
                ],
            );
            let world = World::new(&frame, assets.facts()).unwrap();
            let loc: Localisation = [("oilfield".into(), "Viewer project".into())]
                .into_iter()
                .collect();
            let context = Context {
                assets: &assets,
                world: &world,
                loc: &loc,
                recorded_language: &loc.language,
                scripted_content: &Err("no scripted content in this test".into()),
                ours: 0,
                countries: installed_content::country_names::CountryNames::read(&frame, &loc),
                flag_exists: &|_| true,
                scripted: Default::default(),
            };
            let scope = RaidScope {
                building_type: 200,
                building_flags: 1,
                building_location: 3,
                ..Default::default()
            };
            // Country zero has CURRENT=0 in 140FD0EA0. Equality reveals the project.
            match (
                required,
                first_project,
                context.facility(&scope, &Default::default()).unwrap(),
            ) {
                (
                    1,
                    _,
                    rules::Facility::Unknown {
                        current: 0,
                        required: 1,
                    },
                )
                | (0, None, rules::Facility::Idle) => {}
                (0, Some(100), rules::Facility::Project(text)) => {
                    assert_eq!(text, Ok("Viewer project".into()))
                }
                _ => panic!("wrong facility threshold or program selection"),
            }
        }
    }

    #[test]
    fn captive_uses_original_leader_name_without_character_identity() {
        use super::super::super::tests::put;
        use replay::rows::UnitLeaderRow;
        let assets = assets();
        let mut frame = frame();
        put::<tables::UnitLeaders>(
            &mut frame,
            vec![UnitLeaderRow {
                id: 7,
                reference_type: 84,
                inherited_text_a: b"original_name".to_vec(),
                inherited_text_b: b"wrong client language".to_vec(),
                ..Default::default()
            }],
        );
        let world = World::new(&frame, assets.facts()).unwrap();
        let loc: Localisation = [("original_name".into(), "Viewer language name".into())]
            .into_iter()
            .collect();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &Err("no scripted content in this test".into()),
            ours: 0,
            flag_exists: &|_| true,
            scripted: Default::default(),
            countries: installed_content::country_names::CountryNames::read(&frame, &loc),
        };
        let scope = RaidScope {
            leader_reference: (7 << 32) | 84,
            ..Default::default()
        };
        assert_eq!(
            context.captive(&scope),
            Ok(Some("Viewer language name".into()))
        );
        assert_eq!(context.captive(&RaidScope::default()), Ok(None));
        // The game names every targeted leader; an unrecorded one is refused.
        assert_eq!(
            context.captive(&RaidScope {
                leader_reference: (8 << 32) | 84,
                ..Default::default()
            }),
            Err("captive leader is absent from UnitLeaders".into())
        );
    }
    /// Scripted text content with `MAX_SCRIPTED_LOC_RECURSION` alone.
    fn scripted_content() -> dashboard::scripted_text::ScriptedContent {
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_raid_scripted_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("common/defines/00_defines.lua");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        )
        .unwrap();
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let content = installed_content::content::ResolvedContent {
            layers,
            stats: Default::default(),
        };
        let scripted = dashboard::scripted_text::ScriptedContent::read(
            std::sync::Arc::new(content),
            String::new(),
        )
        .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        scripted
    }
    fn with_context(run: impl FnOnce(&Context<'_, '_>)) {
        let assets = assets();
        let scripted = Ok(scripted_content());
        let frame = frame();
        let world = World::new(&frame, assets.facts()).unwrap();
        let loc: Localisation = [
            ("TRIGGER_ALWAYS_FALSE", "never"),
            ("TRIGGER_ALWAYS_TRUE", "always"),
            ("TRIGGER_FULLFILLED_PREFIX", "+"),
            ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
            ("TRIGGER_OR_STARTS", "either"),
            ("TRIGGER_AND_STARTS", "both"),
            ("custom", "custom text"),
            (
                "scoped",
                "[ROOT.GetTag]/[FROM.GetID]/[THIS.GetTag]/[PREV.GetTag]",
            ),
            ("scoped_NOT", "negative [FROM.GetID]"),
            ("chain", "[FROM.ROOT.GetTag]/[ROOT.FROM.PREV.GetTag]"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted,
            ours: 0,
            countries: installed_content::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        run(&context);
    }
    fn triggers(text: &str) -> installed_content::triggers::Triggers {
        installed_content::parser::parse(text.as_bytes()).unwrap().items
    }
    #[test]
    fn trigger_inversion_indentation_custom_override_and_named_unsupported_leaf() {
        with_context(|context| {
            let scope = Scopes::of(Scope::Country(0));
            let text = context.trigger_text(
                &triggers("always=no NOT={always=no} OR={always=yes always=no}"),
                scope,
                "test",
            );
            assert_eq!(
                text.into_iter()
                    .map(|(_, s)| s.unwrap())
                    .collect::<String>(),
                "-never
+always
+either:
   always
   never
"
            );
            let text=context.trigger_text(&triggers("custom_trigger_tooltip={tooltip=custom always=no} hidden_trigger={always=no} unsupported=yes"),scope,"test");
            assert_eq!(text.len(), 2);
            assert_eq!(
                text[0].1.as_deref(),
                Ok("-custom text
")
            );
            assert!(text[1].1.as_ref().unwrap_err().contains("unsupported"));
        });
    }

    #[test]
    fn custom_tooltip_uses_shared_scope_expansion_and_native_negative_key() {
        with_context(|context| {
            let scopes = Scopes {
                root: Scope::Country(0),
                from: Some(Scope::State(10)),
                prev: Some(Scope::Country(0)),
                ..Scopes::of(Scope::Country(1))
            };
            let text = context.trigger_text(
                &triggers("custom_trigger_tooltip={tooltip=scoped always=yes}"),
                scopes,
                "test",
            );
            assert_eq!(text[0].1.as_deref(), Ok("+GER/10/ENG/GER\n"));
            let text = context.trigger_text(
                &triggers("NOT={custom_trigger_tooltip={tooltip=scoped always=yes}}"),
                scopes,
                "test",
            );
            assert_eq!(text[0].1.as_deref(), Ok("-negative 10\n"));
        });
    }

    /// The raid condition chain's scope (ROOT GER) with FROM a default scope
    /// of the target (ENG): chains follow the chosen scope's own links. A
    /// scope change leaves the linked scopes' links unknown, so a chain
    /// through them is refused rather than guessed.
    #[test]
    fn custom_tooltip_chains_follow_the_raid_scopes_links() {
        with_context(|context| {
            let raid = Scopes {
                from: Some(Scope::Country(1)),
                ..Scopes::of(Scope::Country(0))
            };
            let tooltip = triggers("custom_trigger_tooltip={tooltip=chain always=yes}");
            let text = context.trigger_text(&tooltip, raid, "test");
            assert_eq!(text[0].1.as_deref(), Ok("+ENG/ENG\n"));
            let nested = Scopes {
                prev: Some(Scope::Country(0)),
                ..raid
            };
            let text = context.trigger_text(&tooltip, nested, "test");
            let refused = text[0].1.as_ref().unwrap_err();
            assert!(refused.contains("Root link is not known"), "{refused}");
        });
    }

    #[test]
    fn event_scopes_keep_the_raid_links_leaders_and_target_variables() {
        let raid = Scopes {
            from: Some(Scope::Country(1)),
            target_state: Some(10),
            target_leader: Some(9 << 32 | 3),
            ..Scopes::of(Scope::Country(0))
        };
        let scopes = event_scopes(raid);
        assert_eq!(
            scopes.scopes,
            [
                EventScope {
                    from: Link::Scope(1),
                    ..EventScope::of(EventObject::Country(0))
                },
                EventScope::of(EventObject::Country(1)),
            ]
        );
        assert_eq!(scopes.variables.target_state, Some(10));
        assert_eq!(scopes.variables.target_leader, Some(9 << 32 | 3));
        let nested = event_scopes(Scopes {
            this: Scope::Leader(9 << 32 | 3),
            prev: Some(Scope::Country(0)),
            ..raid
        });
        assert_eq!(nested.scopes[0].this, EventObject::UnitLeader(9 << 32 | 3));
        assert_eq!(nested.scopes[0].root, Link::Scope(1));
        assert_eq!(nested.scopes[1].root, Link::Unknown);
        assert_eq!(nested.variables.target_state, Some(10));
    }

    #[test]
    fn state_name_uses_the_stored_owner_name_in_the_recording_language_only() {
        use super::super::super::tests::put;
        let assets = assets();
        let mut frame = frame();
        put::<tables::States>(
            &mut frame,
            vec![replay::rows::StateRow {
                id: 10,
                owner_name: b"owner cache $literal$".to_vec(),
                ..Default::default()
            }],
        );
        let world = World::new(&frame, assets.facts()).unwrap();
        let loc: Localisation = [("owner cache $literal$".into(), "must not localise".into())]
            .into_iter()
            .collect();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &Err("no scripted content in this test".into()),
            ours: 0,
            countries: installed_content::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        assert_eq!(
            context.state_name(10).as_deref(),
            Ok("owner cache $literal$")
        );
        let other = Context {
            recorded_language: "german",
            ..context
        };
        assert!(
            other
                .state_name(10)
                .unwrap_err()
                .contains("stored in german without its key")
        );
    }
    #[test]
    fn phase_progress_uses_preparation_override_and_refuses_missing_flying_inputs() {
        with_context(|context| {
            let kind = RaidType {
                tooltip: installed_content::raids::tooltips::Definition {
                    days_to_prepare: 2,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut raid = RaidRow {
                phase: 2,
                preparation_hours: 12,
                ..Default::default()
            };
            assert_eq!(
                context.progress(&raid, &RaidTargetRow::default(), &kind),
                Ok(Some(25_000))
            );
            raid.phase = 1;
            assert_eq!(
                context.progress(&raid, &RaidTargetRow::default(), &kind),
                Ok(None)
            );
            raid.phase = 4;
            assert!(
                context
                    .progress(&raid, &RaidTargetRow::default(), &kind)
                    .is_err()
            );
        });
    }
    #[test]
    #[ignore = "synthetic hover cost; release, affinity 0xF"]
    fn hover_build_cost() {
        use super::super::super::tests::put;
        use replay::rows::{CountryLinkRow, CountryRow};
        let countries: u32 = std::env::var("BATTLE_PLAN_RAID_SCORE_COUNTRIES").or_else(|_| std::env::var("HOI4_RAID_SCORE_COUNTRIES"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1500);
        let mut assets = assets();
        let mut frame = frame();
        assets
            .tooltip
            .state_names
            .insert(10, Ok("state_name".into()));
        put::<tables::Countries>(
            &mut frame,
            (0..countries)
                .map(|slot| CountryRow {
                    id: slot + 10,
                    canonical_slot: slot,
                    tag: format!("C{slot}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        );
        put::<tables::CountryLinks>(
            &mut frame,
            (0..countries)
                .map(|slot| CountryLinkRow {
                    id: slot + 10,
                    canonical_slot: slot,
                })
                .collect(),
        );
        put::<tables::Raids>(
            &mut frame,
            vec![RaidRow {
                id: 1,
                reference_type: 84,
                phase: 5,
                ..Default::default()
            }],
        );
        put::<tables::RaidTargets>(
            &mut frame,
            vec![RaidTargetRow {
                id: 1,
                reference_type: 84,
                raid_type: 100,
                scope: RaidScope {
                    state: 10,
                    ..Default::default()
                },
                ..Default::default()
            }],
        );
        let loc: Localisation = [
            ("raid_inst_item_desc".into(), "$TYPE$ $TARGET$".into()),
            (
                "tooltip_raid_result".into(),
                "$RAID_DESC$ $OUTCOME_TYPE$".into(),
            ),
        ]
        .into_iter()
        .collect();
        let mut samples = Vec::new();
        for _ in 0..100 {
            let start = std::time::Instant::now();
            let world = World::new(&frame, assets.facts()).unwrap();
            let context = Context {
                assets: &assets,
                world: &world,
                loc: &loc,
                recorded_language: &loc.language,
                scripted_content: &Err("no scripted content in this test".into()),
                ours: 0,
                flag_exists: &|_| true,
                scripted: Default::default(),
                countries: installed_content::country_names::CountryNames::read(&frame, &loc),
            };
            let tooltip = context.raid(1, 84, false).unwrap();
            assert!(tooltip.refused.is_empty(), "{:?}", tooltip.refused);
            std::hint::black_box(tooltip);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "raid hover ended countries={countries} median_ms={} max_ms={}",
            samples[50], samples[99]
        );
    }
}
