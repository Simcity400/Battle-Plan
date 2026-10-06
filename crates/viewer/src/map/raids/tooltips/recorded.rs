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
    let flag_exists = |key: &str| content.icons.counters.flags.contains(key);
    build_from(Inputs {
        assets: &content.icons.raids,
        frame,
        ours,
        key,
        loc: &content.names.localisation,
        recorded_language: &content.recorded_language,
        scripted_content: &content.scripted,
        flag_exists: &flag_exists,
    })
}

pub(crate) struct Inputs<'a> {
    pub assets: &'a RaidAssets,
    pub frame: &'a DecodedFrame,
    pub ours: u32,
    pub key: &'a Key,
    pub loc: &'a Localisation,
    pub recorded_language: &'a str,
    pub scripted_content: &'a Result<dashboard::scripted_text::ScriptedContent>,
    pub flag_exists: &'a dyn Fn(&str) -> bool,
}

pub(crate) fn build_from(input: Inputs<'_>) -> Tooltip {
    let Inputs {
        assets,
        frame,
        ours,
        key,
        loc,
        recorded_language,
        scripted_content,
        flag_exists,
    } = input;
    let world = match World::new(frame, assets.facts()) {
        Ok(world) => world,
        Err(reason) => {
            let mut out = Tooltip::default();
            out.refuse("raid tooltip", reason);
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
        flag_exists,
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
    // 140A8D3F0 -> 141580600 builds raids from default scopes (140528800).
    let target = |scope| match scope {
        Scope::Country(slot) => EventObject::Country(slot),
        Scope::State(id) => EventObject::State(id),
        Scope::Province(id) => EventObject::Province(id),
        Scope::Leader(reference) => EventObject::UnitLeader(reference),
        Scope::Character(id) => EventObject::Character(id),
        Scope::Combat => unreachable!("a raid condition is never checked in a combat scope"),
    };
    let unknown = |scope| EventScope {
        random_counter: 0x5ea6_ba9e,
        this: target(scope),
        root: Link::Unknown,
        from: Link::Unknown,
        prev: Link::Unknown,
        random_seed: 1,
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
            this: target(scopes.this),
            root: if scopes.unknown.root {
                Link::Unknown
            } else {
                Link::Scope(1)
            },
            from,
            prev,
            random_seed: 1,
        };
        std::iter::once(current).chain(linked).collect()
    };
    // The target variables stay set while the condition's text is built
    // (141580600 sets them for the raid scope's whole evaluation).
    let mut graph = EventScopes::new(list);
    graph.variables = ScopeVariables {
        target_state: scopes.target_state,
        target_province: scopes.target_province,
        target_leader: scopes.target_leader,
    };
    graph
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
                let branch =
                    arrows::SpeedBranch::of(kind, &|name| self.assets.equipment_category(name))?;
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
        match program.and_then(|r| r.project_definition.map(|token| (r, token))) {
            None => Ok(rules::Facility::Idle),
            Some((program, token)) => {
                let key = self
                    .world
                    .token(token)?
                    .ok_or("ongoing project definition token is absent")?;
                let key = std::str::from_utf8(key).map_err(|e| e.to_string())?;
                // The reader authenticates the selected project's +1C against
                // this program owner; no daily project-history join is needed.
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
        // 140FCDDF0 -> 140A81BB0/14146B580: nonempty narrative name,
        // otherwise template+238 (140A852A0); then TAG_key. 140FCADC0
        // stores template+18 at project+20: name record is template+1F0.
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
        // 1416151F0 takes record +40: 140A81510 appends the default
        // in parentheses when keys differ; 140A81880 only wraps yellow.
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
        choice
            .candidates
            .iter()
            .find(|key| (self.flag_exists)(key))
            .ok_or("controller has no resolved flag asset")?;
        // DrawInlineFlag@1402EEFF0 resolves the tag through this hour's native small atlas.
        if choice.tag.chars().count() != 3 {
        }
        Ok(format!("{}{}", crate::map::native_text::FLAG, choice.tag))
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
