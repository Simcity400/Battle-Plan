use super::super::{
    RaidAssets, TypeToken, launch, offers, paths,
    script::{Eval, Scope, Scopes, World},
    target_country, unit_inputs,
};
use super::{Key, Tooltip, chance, intel, rules, success};
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link, ScopeVariables};
use content_reader_localisation::localisation::Argument as Arg;
use content_reader_localisation::localisation::Localisation;
use content_reader_raids::raids::RaidType;
use replay_data::{rows::RaidRow, rows::RaidScope, rows::RaidTargetRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use viewer_native_gui::native_text::FLAG;
use viewer_foundation::tooltip_text::localise;
use viewer_foundation::tooltip_text::state_name as tooltip_text_state_name;

type Result<T> = std::result::Result<T, String>;
#[path = "diagnostic.rs"]
mod diagnostic;
#[path = "trigger.rs"]
mod trigger;

pub fn build(
    content: &super::Content,
    frame: &DecodedFrame,
    ours: u32,
    nato: bool,
    key: &Key,
) -> Tooltip {
    let flag_exists = |key: &str| content.flags.contains(key);
    build_from(Inputs {
        assets: &content.assets,
        frame,
        ours,
        nato,
        key,
        loc: &content.localisation,
        recorded_language: &content.recorded_language,
        scripted_content: &content.scripted,
        flag_exists: &flag_exists,
    })
}

pub fn details(
    content: &super::Content,
    frame: &DecodedFrame,
    ours: u32,
    targets: &[offers::TooltipTarget],
) -> super::Details {
    let flag_exists = |key: &str| content.flags.contains(key);
    let header = localise(&content.localisation, "MAP_ICON_RAID_TARGET_HEADER", &[]);
    let world = match World::new(frame, content.assets.trigger_content.facts()) {
        Ok(world) => world,
        Err(reason) => {
            return super::Details {
                header,
                entries: targets
                    .iter()
                    .map(|target| super::DetailEntry {
                        scope: target.scope.clone(),
                        name: Err(reason.to_string()),
                        icons: Err(reason.to_string()),
                    })
                    .collect(),
            };
        }
    };
    let context = Context {
        assets: &content.assets,
        world: &world,
        loc: &content.localisation,
        recorded_language: &content.recorded_language,
        scripted_content: &content.scripted,
        ours,
        nato: false,
        flag_exists: &flag_exists,
        scripted: Default::default(),
        countries: content_reader_country_names::country_names::CountryNames::read(frame, &content.localisation),
    };
    super::Details { header, entries: context.detail_entries(targets) }
}

pub struct Inputs<'a> {
    pub assets: &'a RaidAssets,
    pub frame: &'a DecodedFrame,
    pub ours: u32,
    pub nato: bool,
    pub key: &'a Key,
    pub loc: &'a Localisation,
    pub recorded_language: &'a str,
    pub scripted_content: &'a Result<dashboard_text::scripted_text::ScriptedContent>,
    pub flag_exists: &'a dyn Fn(&str) -> bool,
}

pub fn build_from(input: Inputs<'_>) -> Tooltip {
    let Inputs {
        assets,
        frame,
        ours,
        nato,
        key,
        loc,
        recorded_language,
        scripted_content,
        flag_exists,
    } = input;
    let world = match World::new(frame, assets.trigger_content.facts()) {
        Ok(world) => world,
        Err(reason) => {
            let mut out = Tooltip::default();
            out.refuse("raid tooltip", reason.to_string());
            return out;
        }
    };
    let context = Context {
        assets,
        world: &world,
        loc,
        recorded_language,
        scripted_content,
        ours,
        nato,
        flag_exists,
        scripted: Default::default(),
        countries: content_reader_country_names::country_names::CountryNames::read(frame, loc),
    };
    let result = match key {
        Key::Raid {
            id,
            reference_type,
            incoming,
        } => context.raid(*id, *reference_type, *incoming),
        Key::Offer(targets) => Ok(context.offers(targets, true)),
        Key::Type(target) => Ok(context.offers(std::slice::from_ref(&**target), false)),
    };
    result.unwrap_or_else(|reason| {
        let mut out = Tooltip::default();
        out.refuse("raid tooltip", reason.to_string());
        out
    })
}

fn detail_order(types: &[(usize, Result<bool>)]) -> (Result<Option<usize>>, Result<Vec<(usize, bool)>>) {
    let undecided: Vec<&str> = types.iter().filter_map(|(_, allowed)| allowed.as_ref().err().map(String::as_str)).collect();
    let launchable: Vec<usize> = types.iter().filter(|(_, allowed)| *allowed == Ok(true)).map(|&(at, _)| at).collect();
    let why = || undecided.join("; ");
    let named = match (launchable.as_slice(), undecided.is_empty()) {
        ([one], true) => Ok(Some(*one)),
        (_, true) => Ok(None),
        (many, false) if many.len() >= 2 => Ok(None),
        _ => Err(format!("raid details name: a type's launchability is undecided: {}", why())),
    };
    let buttons = if undecided.is_empty() {
        Ok(launchable
            .iter()
            .map(|&at| (at, true))
            .chain(types.iter().filter(|(_, allowed)| *allowed == Ok(false)).map(|&(at, _)| (at, false)))
            .collect())
    } else {
        Err(format!("raid details type buttons: a type's launchability is undecided: {}", why()))
    };
    (named, buttons)
}

struct Context<'a, 'w> {
    assets: &'a RaidAssets,
    world: &'w World<'a>,
    loc: &'a Localisation,
    recorded_language: &'a str,
    scripted_content: &'a Result<dashboard_text::scripted_text::ScriptedContent>,
    ours: u32,
    nato: bool,
    countries: content_reader_country_names::country_names::CountryNames<'a>,
    flag_exists: &'w dyn Fn(&str) -> bool,
    scripted: OnceCell<dashboard_text::scripted_text::ScriptedText<'a>>,
}

fn known(eval: Eval) -> Result<bool> {
    match eval {
        Eval::True => Ok(true),
        Eval::False => Ok(false),
        Eval::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
    }
}

fn event_scopes(scopes: Scopes) -> EventScopes {
    let target = |scope| match scope {
        Scope::Country(slot) => EventObject::Country(slot),
        Scope::State(id) => EventObject::State(id),
        Scope::Province(id) => EventObject::Province(id),
        Scope::Leader(reference) => EventObject::UnitLeader(reference),
        Scope::CountryLeader { country, leader } => {
            EventObject::CountryUnitLeader { country, leader }
        }
        Scope::Character(id) => EventObject::Character(id),
        Scope::IndustrialOrganisation(reference) => EventObject::IndustrialOrganisation(reference),
        Scope::Project { owner, id } => EventObject::Project { owner, id },
        Scope::Raid(reference) => EventObject::Raid(reference),
        Scope::Unit(reference) => EventObject::Unit(reference),
        Scope::Combat => unreachable!("a raid condition is never checked in a combat scope"),
    };
    let unknown = |scope| EventScope {
        random_counter: 0x5ea6_ba9e,
        random_seed_refusal: None,
        this: target(scope),
        root: Link::Unknown,
        from: Link::Unknown,
        prev: Link::Unknown,
        random_seed: 1,
        clock_seed: None,
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
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            this: target(scopes.this),
            root: if scopes.unknown.root {
                Link::Unknown
            } else {
                Link::Scope(1)
            },
            from,
            prev,
            random_seed: 1,
            clock_seed: None,
        };
        std::iter::once(current).chain(linked).collect()
    };
    let mut graph = EventScopes::new(list);
    graph.variables = ScopeVariables {
        target_state: scopes.target_state,
        target_province: scopes.target_province,
        target_leader: scopes.target_leader,
    };
    graph
}

impl<'a> Context<'a, '_> {
    fn scripted_text(&self) -> Result<&dashboard_text::scripted_text::ScriptedText<'a>> {
        let content = self.scripted_content.as_ref().map_err(Clone::clone)?;
        Ok(self.scripted.get_or_init(|| {
            dashboard_text::scripted_text::ScriptedText::new(
                self.world.frame(),
                self.loc,
                content,
                self.ours,
            )
            .viewing(self.ours)
            .with_nato_counters(self.nato)
        }))
    }

    fn scoped_text(&self, key: &str, scopes: Scopes) -> Result<String> {
        let text = localise(self.loc, key, &[])?;
        if !text.contains('[') {
            return Ok(text);
        }
        self.scripted_text()?.expand_in(&text, &event_scopes(scopes))
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
        tooltip_text_state_name(
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
    fn target_localisation(&self, key: &str) -> Result<Option<String>> {
        if self.loc.get(key).is_none() {
            return Ok(None);
        }
        localise(self.loc, key, &[]).map(Some)
    }
    fn victory_point(&self, province: u32) -> Result<Option<String>> {
        content_reader_raids::raids::target::victory_point(self.world.frame(), province, |key| {
            self.target_localisation(key)
        })
    }
    fn location(&self, scope: &RaidScope) -> Result<String> {
        content_reader_raids::raids::target::location(
            self.world.frame(),
            scope,
            |key| self.target_localisation(key),
            |state| self.state_name(state),
        )
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
                .trigger_content
                .terrain
                .as_ref()
                .ok_or("province movement content unavailable")?
                .map
                .terrain(province)
                .ok_or("target province terrain unavailable")?;
            let index = usize::try_from(terrain.index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .ok_or("target province has null terrain")?;
            let name = self
                .assets
                .tooltip
                .terrain_names
                .as_ref()?
                .get(index)
                .ok_or("target terrain ordinal absent")?;
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
                let (travelled, inputs) = unit_inputs::plane_inputs(self.world.frame(), raid)?;
                let branch = unit_inputs::SpeedBranch::of(kind, &|name| {
                    self.assets.equipment_category(name)
                })?;
                let speed = unit_inputs::raid_unit_speed(
                    branch,
                    &inputs,
                    kind.speed_multiplier_raw,
                    self.assets.tooltip.speed.as_ref()?,
                );
                let distance = unit_inputs::target_distance(
                    self.assets,
                    self.world.frame(),
                    raid,
                    target,
                    &mut Vec::new(),
                )?;
                (
                    viewer_gpu::unit_pose::elapsed_hours(travelled, speed),
                    viewer_gpu::unit_pose::remaining_hours(distance, travelled, speed),
                )
            }
            _ => (0, 0),
        };
        Ok((elapsed.wrapping_add(remaining) > 0)
            .then(|| viewer_gpu::unit_pose::phase_progress(elapsed, remaining)))
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
    fn facility(
        &self,
        scope: &RaidScope,
        current: &OnceCell<Result<i64>>,
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
        match program.and_then(|r| r.project_definition.map(|token| (r, token))) {
            None => Ok(rules::Facility::Idle),
            Some((program, token)) => {
                let key = self
                    .world
                    .token(token)
                    .map_err(|why| why.to_string())?
                    .ok_or("ongoing project definition token is absent")?;
                let key = std::str::from_utf8(key).map_err(|e| e.to_string())?;
                let owner = self
                    .world
                    .slot(program.owner)
                    .ok_or("ongoing project owner is absent")?;
                Ok(rules::Facility::Project(
                    self.project_tooltip_name(key, owner),
                ))
            }
        }
    }
    fn project_tooltip_name(&self, name: &str, country: u32) -> Result<String> {
        let catalog = self.assets.tooltip.projects.as_ref()?;
        if let Some(reason) = &catalog.unsettled {
            return Err(format!("special project catalog: {reason}"));
        }
        let (default, project) = catalog
            .projects
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("special project {name} has no content definition"))?;
        if let Some(reason) = &project.refusal {
            return Err(format!("special project {default}: {reason}"));
        }
        let key = project
            .name
            .as_deref()
            .filter(|key| !key.is_empty())
            .unwrap_or(default);
        let tag = std::str::from_utf8(
            self.world
                .tag(country)
                .ok_or("project country tag is absent")?,
        )
        .map_err(|e| e.to_string())?;
        let prefixed = format!("{tag}_{key}");
        let selected = if self.loc.get(&prefixed).is_some() {
            &prefixed
        } else {
            key
        };
        let scopes = Scopes::of(Scope::Country(country));
        let text = self.scoped_text(selected, scopes)?;
        if selected == default {
            Ok(format!("\u{11}Y{text}\u{11}!"))
        } else {
            Ok(format!(
                "\u{11}Y{text}\u{11}! ({})",
                self.scoped_text(default, scopes)?
            ))
        }
    }
    fn scopes(&self, scope: &RaidScope) -> Result<Scopes> {
        Ok(Scopes {
            cursor: None,
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
    fn detail_entries(&self, targets: &[offers::TooltipTarget]) -> Vec<super::DetailEntry> {
        let paths = paths::HourPaths::new(self.assets, self.world.frame());
        let launcher = launch::Launcher::new(self.assets, self.world, self.ours, &paths);
        let local = launcher.local();
        targets
            .iter()
            .map(|target| {
                let scope = &target.scope;
                let template = if scope.building_type == 0 {
                    Ok(None)
                } else {
                    self.assets.building(self.world, scope.building_type).map(|building| Some(building.name.as_str()))
                };
                let types: Vec<(usize, Result<bool>)> = target
                    .types
                    .iter()
                    .map(|&at| {
                        let allowed = match self.assets.catalog.types.get(at) {
                            Some(kind) => self
                                .scopes(scope)
                                .and_then(|s| known(launcher.launchable_in(&local, kind, scope, s))),
                            None => Err(format!("raid offer type: cached index {at} is outside content catalog")),
                        };
                        (at, allowed)
                    })
                    .collect();
                let (named, buttons) = detail_order(&types);
                let name = named.and_then(|kind| {
                    self.target_name(scope, kind.and_then(|at| self.assets.catalog.types.get(at)))
                });
                let icons = buttons.and_then(|buttons| {
                    let template = template.clone()?;
                    Ok(buttons
                        .into_iter()
                        .map(|(at, enabled)| (at, self.assets.glyph_key_of(at, template), enabled))
                        .collect())
                });
                super::DetailEntry { scope: scope.clone(), name, icons }
            })
            .collect()
    }

    fn offers(&self, targets: &[offers::TooltipTarget], bubble: bool) -> Tooltip {
        let paths = paths::HourPaths::new(self.assets, self.world.frame());
        let launcher = launch::Launcher::new(self.assets, self.world, self.ours, &paths);
        let local = launcher.local();
        let flags = OnceCell::new();
        let mut output = Vec::new();
        let mut quick = Ok(false);
        for target in targets {
            let scope = &target.scope;
            let mut types = Vec::new();
            let mut any = Eval::False;
            for &at in &target.types {
                let Some(kind) = self.assets.catalog.types.get(at) else {
                    return Tooltip {
                        refused: BTreeMap::from([(
                            format!(
                                "raid offer type: cached index {at} is outside content catalog"
                            ),
                            1,
                        )]),
                        ..Default::default()
                    };
                };
                let scopes = self.scopes(scope);
                let allowed = scopes
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|s| known(launcher.launchable_in(&local, kind, scope, *s)));
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
            if bubble && targets.len() == 1 && target.types.len() == 1 {
                quick = allowed.clone();
            }
            let kind = if target.types.len() == 1 {
                self.assets.catalog.types.get(target.types[0])
            } else {
                None
            };
            let current = OnceCell::new();
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
    fn flag(
        &self,
        country: u32,
        flags: &OnceCell<Result<Vec<dashboard_text::flag_choices::FlagChoice>>>,
    ) -> Result<String> {
        let choice = flags
            .get_or_init(|| {
                dashboard_text::flag_choices::choices(self.world.frame()).map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .find(|choice| self.world.slot(choice.raw) == Some(country))
            .ok_or("controller flag operands are absent")?;
        choice.key(self.flag_exists)?;
        if choice.tag.chars().count() != 3 {
            return Err("controller flag tag is not three codepoints".into());
        }
        Ok(format!("{}{}", FLAG, choice.tag))
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
                    super::names::Name::new(frame, raid).and_then(|name| {
                        name.assigned(&self.assets.tooltip, self.loc, self.recorded_language)
                    })
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
mod tests;
