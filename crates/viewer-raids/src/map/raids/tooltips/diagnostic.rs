use super::super::rules::Lines;
use super::*;
use engine_native::{
    raid_tooltip_operands::ADDITIONAL_ORDER, raid_tooltip_operands::ESSENTIAL_ORDER,
    raid_tooltip_operands::definition_key,
};
use content_reader_parser::parser::Item;
use std::collections::BTreeMap;
use viewer_foundation::tooltip_text::country_localised;

fn add(lines: &mut Lines, key: &str, text: Result<String>) {
    lines.push((key.into(), text));
}

fn combine(result: &mut Eval, next: Eval) {
    *result = result.clone().and(|| next);
}

impl Context<'_, '_> {
    fn unit_requirements(&self, kind: &RaidType) -> Lines {
        super::super::requirements::build(
            self.loc,
            self.assets.equipment(),
            kind,
            |key| self.equipment_name(key),
            |key| {
                let token = self
                    .world
                    .token_of(key.as_bytes())
                    .map_err(|why| why.to_string())?
                    .unwrap_or(0);
                let rows = self
                    .world
                    .frame()
                    .complete::<tables::RaidTooltipOperands>()
                    .map_err(|e| e.to_string())?;
                let mut active = rows
                    .iter()
                    .filter(|r| {
                        r.raid == 0
                            && r.kind == engine_native::raid_tooltip_operands::SUBUNIT_ACTIVE
                    })
                    .collect::<Vec<_>>();
                active.sort_unstable_by_key(|r| r.ordinal);
                active
                    .iter()
                    .find(|r| r.key == token)
                    .or_else(|| active.iter().find(|r| r.key == 0))
                    .map(|r| r.raw != 0)
                    .ok_or_else(|| "global subunit definition witness absent".into())
            },
        )
    }
    fn source_description(&self, raid: &RaidRow) -> Result<String> {
        let source = &raid.source;
        let ship =
            (u64::from(source.ship_reference_id) << 32) | u64::from(source.ship_reference_type);
        let is_ship = source.kind_present != 0 && matches!(source.kind, Some(3 | 4));
        let location = if is_ship {
            super::super::names::Name::new(self.world.frame(), raid)?.source(
                &self.assets.tooltip,
                self.loc,
                self.recorded_language,
                ship,
            )?
        } else {
            let province = if source.kind_present != 0 && source.kind == Some(5) {
                unit_inputs::source_province(self.assets, self.world, raid, &mut None)?.0
            } else {
                source.province
            };
            let rows = self
                .world
                .frame()
                .complete::<tables::Provinces>()
                .map_err(|e| e.to_string())?;
            self.state_name(
                rows.iter()
                    .find(|r| r.id == province)
                    .ok_or("source province absent")?
                    .state,
            )?
        };
        let what = if source.kind_present == 0 {
            String::new()
        } else {
            match source.kind {
                Some(0 | 3 | 4) => {
                    match super::super::names::Name::new(self.world.frame(), raid)?
                        .source_air_base_kind()?
                    {
                        0 => self.plain("air_base")?,
                        1 => self.plain("rocket_site")?,
                        2 => self.plain("mega_gun_emplacement")?,
                        _ => String::new(),
                    }
                }
                Some(1) => self.plain("rocket_site")?,
                Some(2) => self.plain("naval_base")?,
                Some(5) => {
                    self.plain(&self.assets.building(self.world, source.building_type)?.name)?
                }
                _ => String::new(),
            }
        };
        localise(
            self.loc,
            "raid_source_desc",
            &[
                ("WHAT", Arg::Text(&what)),
                ("LOCATION", Arg::Text(&location)),
            ],
        )
    }
    fn plain(&self, key: &str) -> Result<String> {
        localise(self.loc, key, &[])
    }
    fn condition(&self, lines: &mut Lines, trigger: &[Item], scopes: Scopes, name: &str) -> Eval {
        let result = self.world.evaluate(trigger, scopes);
        match &result {
            Eval::False => {
                let title = match scopes.root {
                    Scope::Country(c) => self.countries.name(c),
                    _ => Err("raid trigger ROOT is not country".into()),
                };
                add(
                    lines,
                    "raid_tooltip_trigger_title",
                    title.and_then(|title| {
                        localise(
                            self.loc,
                            "raid_tooltip_trigger_title",
                            &[("TITLE", Arg::Text(&title))],
                        )
                    }),
                );
                lines.extend(self.trigger_text(trigger, scopes, name));
            }
            Eval::Unknown(reasons) => add(
                lines,
                name,
                Err(reasons.iter().cloned().collect::<Vec<_>>().join("; ")),
            ),
            Eval::True => {}
        }
        result
    }
    pub(super) fn conditions(
        &self,
        kind: &RaidType,
        scope: &RaidScope,
        scopes: Scopes,
        launch: bool,
        source: Option<Result<Option<u32>>>,
    ) -> (Eval, Lines) {
        let mut lines = Vec::new();
        let owner = match scopes.root {
            Scope::Country(c) => c,
            _ => self.ours,
        };
        let mut result = self.condition(
            &mut lines,
            &kind.visible,
            Scopes::of(Scope::Country(owner)),
            "raid visible",
        );
        combine(
            &mut result,
            offers::category_admits_unlock(self.assets, self.world, owner, kind),
        );
        let used = crate::map::raids::eligibility::used_up(self.assets, self.world, owner, kind);
        match &used {
            Eval::True => add(
                &mut lines,
                "tooltip_raid_already_launched",
                self.plain("tooltip_raid_already_launched"),
            ),
            Eval::Unknown(r) => add(
                &mut lines,
                "tooltip_raid_already_launched",
                Err(r.iter().cloned().collect::<Vec<_>>().join("; ")),
            ),
            _ => {}
        }
        combine(&mut result, used.negate());
        combine(
            &mut result,
            self.condition(&mut lines, &kind.show_target, scopes, "raid show_target"),
        );
        let cooldown = self.cooldown_days(owner, kind, scope);
        match cooldown {
            Ok(Some(days)) => {
                add(
                    &mut lines,
                    "tooltip_raid_on_cooldown",
                    localise(
                        self.loc,
                        "tooltip_raid_on_cooldown",
                        &[("DAYS", Arg::Int32(days))],
                    ),
                );
                combine(&mut result, Eval::False);
            }
            Ok(None) => {}
            Err(reason) => {
                add(&mut lines, "tooltip_raid_on_cooldown", Err(reason.clone()));
                combine(&mut result, Eval::unknown(reason));
            }
        }
        combine(
            &mut result,
            self.condition(&mut lines, &kind.available, scopes, "raid available"),
        );
        if launch {
            combine(
                &mut result,
                self.condition(&mut lines, &kind.launchable, scopes, "raid launchable"),
            );
        }
        if let Some(source) = source
            && self.world.has_children(&kind.launchable_from) != Ok(false)
        {
            match source {
                Ok(Some(country)) => combine(
                    &mut result,
                    self.condition(
                        &mut lines,
                        &kind.launchable_from,
                        Scopes {
                            root: Scope::Country(country),
                            this: Scope::Country(country),
                            ..scopes
                        },
                        "raid launchable_from",
                    ),
                ),
                other => {
                    let reason = other
                        .err()
                        .unwrap_or_else(|| "raid source country is null".into());
                    add(&mut lines, "raid launchable_from", Err(reason.clone()));
                    combine(&mut result, Eval::unknown(reason));
                }
            }
        }
        (result, lines)
    }
    fn cooldown_days(&self, owner: u32, kind: &RaidType, scope: &RaidScope) -> Result<Option<i32>> {
        let rows = self
            .world
            .frame()
            .complete::<tables::RaidCooldowns>()
            .map_err(|e| e.to_string())?;
        let Some(token) = self
            .world
            .token_of(kind.name.as_bytes())
            .map_err(|why| why.to_string())?
        else {
            return Ok(None);
        };
        Ok(rows
            .iter()
            .find(|row| {
                let s = &row.scope;
                row.owner_slot == owner
                    && row.raid_type == token
                    && s.building_type == scope.building_type
                    && (scope.building_type == 0
                        || (s.building_location == scope.building_location
                            && s.building_flags == scope.building_flags))
                    && s.province == scope.province
                    && s.state == scope.state
                    && s.leader_reference == scope.leader_reference
            })
            .map(|row| row.days))
    }
    fn equipment_order<'a>(
        &self,
        kind: &'a RaidType,
        additional: bool,
    ) -> Result<Vec<(&'a str, i32)>> {
        let required = if additional {
            &kind.additional_equipment
        } else {
            &kind.essential_equipment
        };
        let mut amounts: BTreeMap<&str, i32> =
            required.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        if amounts.is_empty() {
            return Ok(Vec::new());
        }
        let token = self
            .world
            .token_of(kind.name.as_bytes())
            .map_err(|why| why.to_string())?
            .ok_or("raid type token absent")?;
        let key = definition_key(token);
        let family = if additional {
            ADDITIONAL_ORDER
        } else {
            ESSENTIAL_ORDER
        };
        let rows = self
            .world
            .frame()
            .complete::<tables::RaidTooltipOperands>()
            .map_err(|e| e.to_string())?;
        let mut rows: Vec<_> = rows
            .iter()
            .filter(|r| r.raid == key && r.kind == family)
            .collect();
        rows.sort_unstable_by_key(|r| r.ordinal);
        let count = rows
            .first()
            .filter(|r| r.key == 0)
            .ok_or("raid equipment map order witness absent")?
            .raw;
        if usize::try_from(count).ok() != Some(amounts.len()) || rows.len() != amounts.len() + 1 {
            return Err("raid equipment map differs from content".into());
        }
        let mut out = Vec::new();
        for row in rows.into_iter().skip(1) {
            let key = self
                .world
                .token(row.key)
                .map_err(|why| why.to_string())?
                .ok_or("raid equipment token absent")?;
            let name = std::str::from_utf8(key).map_err(|e| e.to_string())?;
            let (name, amount) = amounts
                .remove_entry(name)
                .ok_or("raid equipment map token differs from content")?;
            out.push((name, amount));
        }
        Ok(out)
    }
    fn shortages(
        &self,
        kind: &RaidType,
        additional: bool,
        waiting: bool,
        header: bool,
        nukes: Result<i32>,
        have: impl Fn(&str) -> Result<i32>,
    ) -> (bool, Lines) {
        let mut lines = Vec::new();
        let mut met = true;
        let required = if additional {
            kind.additional_nukes
        } else {
            kind.nukes
        };
        let nuke_key = if kind.thermonuclear {
            engine_native::raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN
        } else {
            engine_native::raid_tooltip_operands::NUCLEAR_BOMB_TOKEN
        };
        let name = || {
            self.world
                .token(nuke_key)
                .map_err(|why| why.to_string())?
                .ok_or("nuke name token absent")
                .and_then(|b| std::str::from_utf8(b).map_err(|_| "nuke token is not UTF8"))
                .map_err(str::to_owned)
                .and_then(|key| self.plain(key))
        };
        let mut entry =
            |label: &str, name: Result<String>, required: i32, have: Result<i32>| match have {
                Ok(have) if have >= required => {}
                Ok(have) => {
                    met = false;
                    add(
                        &mut lines,
                        label,
                        name.and_then(|name| {
                            localise(
                                self.loc,
                                "tooltip_raid_missing_equipment_entry",
                                &[
                                    ("EQUIPMENT", Arg::Text(&name)),
                                    ("AMOUNT", Arg::Int32(required.wrapping_sub(have))),
                                ],
                            )
                        }),
                    );
                }
                Err(reason) => {
                    met = false;
                    add(&mut lines, label, Err(reason));
                }
            };
        entry(
            "tooltip_raid_missing_equipment_entry: nukes",
            name(),
            required,
            nukes,
        );
        match self.equipment_order(kind, additional) {
            Ok(order) => {
                for (name, required) in order {
                    entry(
                        &format!("tooltip_raid_missing_equipment_entry: {name}"),
                        self.equipment_name(name),
                        required,
                        have(name),
                    );
                }
            }
            Err(reason) => {
                met = false;
                add(
                    &mut lines,
                    "tooltip_raid_missing_equipment_entry",
                    Err(reason),
                );
            }
        }
        if !met && header {
            let key = if waiting {
                "tooltip_raid_waiting_for_equipment"
            } else {
                "tooltip_raid_missing_equipment_header"
            };
            lines.insert(0, (key.into(), self.plain(key)));
        }
        (met, lines)
    }
    fn equipment_name(&self, key: &str) -> Result<String> {
        let operands = self
            .countries
            .operands(self.ours)
            .ok_or("viewer country naming operands absent")?
            .as_ref()?;
        country_localised(self.loc, Ok(&operands.tag), key)
    }
    pub(super) fn offer_denial<'a, 'w>(
        &self,
        launcher: &launch::Launcher<'w, 'a>,
        local: &launch::Local<'_, 'a>,
        kind: &RaidType,
        scope: &RaidScope,
        scopes: Result<Scopes>,
    ) -> Lines {
        let mut lines = Vec::new();
        match known(launcher.command_power(kind)) {
            Ok(false) => add(
                &mut lines,
                "raid_insufficient_command_power",
                localise(
                    self.loc,
                    "raid_insufficient_command_power",
                    &[("COST", Arg::Fixed5(kind.command_power_raw))],
                ),
            ),
            Err(reason) => add(&mut lines, "raid_insufficient_command_power", Err(reason)),
            _ => {}
        }
        lines.extend(
            self.shortages(
                kind,
                false,
                false,
                true,
                launcher.nuke_count(kind),
                |name| launcher.stockpile(name),
            )
            .1,
        );
        match scopes {
            Ok(scopes) => lines.extend(self.conditions(kind, scope, scopes, false, None).1),
            Err(e) => add(&mut lines, "raid preparation triggers", Err(e)),
        }
        match known(launcher.valid_units(local, kind)) {
            Ok(false) => {
                add(
                    &mut lines,
                    "tooltip_raid_no_valid_unit",
                    self.plain("tooltip_raid_no_valid_unit"),
                );
                lines.extend(self.unit_requirements(kind));
            }
            Ok(true) => match known(launcher.reachable(local, kind, scope)) {
                Ok(false) => add(
                    &mut lines,
                    "tooltip_raid_cannot_reach",
                    self.plain("tooltip_raid_cannot_reach"),
                ),
                Err(e) => add(&mut lines, "tooltip_raid_cannot_reach", Err(e)),
                _ => {}
            },
            Err(e) => add(&mut lines, "tooltip_raid_no_valid_unit", Err(e)),
        }
        lines
    }
    pub(super) fn raid_diagnostics(
        &self,
        raid: &RaidRow,
        target: &RaidTargetRow,
        kind: &RaidType,
    ) -> (Result<bool>, Lines) {
        let mut lines = Vec::new();
        let source = unit_inputs::source_province(self.assets, self.world, raid, &mut None);
        match source
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|(province, _)| unit_inputs::unit_at_source(self.world, raid, *province))
        {
            Ok(false) => add(
                &mut lines,
                "raid_unit_not_yet_at_source",
                (|| {
                    let name = super::super::names::Name::new(self.world.frame(), raid)?
                        .description(&self.assets.tooltip, self.loc, self.recorded_language)?;
                    localise(
                        self.loc,
                        "raid_unit_not_yet_at_source",
                        &[
                            ("UNIT", Arg::Text(&name)),
                            ("SOURCE", Arg::Text(&self.source_description(raid)?)),
                        ],
                    )
                })(),
            ),
            Err(reason) => add(&mut lines, "raid_unit_not_yet_at_source", Err(reason)),
            _ => {}
        }
        match unit_inputs::raid_pool(self.world.frame(), raid) {
            Ok(pool) => {
                let have = |name: &str| -> Result<i32> {
                    let token = self
                        .world
                        .token_of(name.as_bytes())
                        .map_err(|why| why.to_string())?
                        .ok_or("raid pool equipment token absent")?;
                    let mut value = (pool
                        .archetype_totals
                        .iter()
                        .find(|(key, _)| *key == token)
                        .map(|(_, v)| *v)
                        .unwrap_or(0)
                        / 100_000) as i32;
                    let (equipment, _) = self.assets.equipment()?;
                    let definition = equipment
                        .named(name)
                        .ok_or_else(|| format!("raid equipment {name} is not in the content"))?;
                    if equipment.category(definition) & 1 != 0 {
                        value = value.max(-1);
                    }
                    Ok(value)
                };
                let (met, essential) =
                    self.shortages(kind, false, true, true, Ok(pool.nukes), have);
                lines.extend(essential);
                lines.extend(
                    self.shortages(kind, true, true, met, Ok(pool.nukes), have)
                        .1,
                );
            }
            Err(reason) => add(
                &mut lines,
                "tooltip_raid_waiting_for_equipment",
                Err(reason),
            ),
        }
        let scopes = match self.scopes(&target.scope) {
            Ok(s) => Scopes {
                root: Scope::Country(raid.parent_slot),
                this: Scope::Country(raid.parent_slot),
                ..s
            },
            Err(reason) => {
                add(&mut lines, "raid trigger scope", Err(reason.clone()));
                return (Err(reason), lines);
            }
        };
        let source = source.map(|(_, country)| country).and_then(|c| c);
        let (allowed, triggers) = self.conditions(
            kind,
            &target.scope,
            scopes,
            raid.phase == 3,
            (raid.phase == 3).then_some(source),
        );
        lines.extend(triggers);
        (known(allowed), lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::raids::test_support::{assets, positive_frame as base_frame, put};
    use replay_data::{
        rows::CountryAppearanceRow, rows::LexicalTokenRow, rows::PoliticsRow,
        rows::RaidCooldownRow, rows::RaidCountryRow, rows::RaidTooltipOperandRow,
    };

    fn frame() -> DecodedFrame {
        let mut frame = base_frame();
        put::<tables::CountryAppearance>(
            &mut frame,
            [10, 11]
                .into_iter()
                .map(|country| CountryAppearanceRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        );
        put::<tables::Politics>(
            &mut frame,
            [10, 11]
                .into_iter()
                .map(|country| PoliticsRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        );
        frame
    }

    fn parse(text: &str) -> Vec<Item> {
        content_reader_parser::parser::parse(text.as_bytes())
            .unwrap()
            .items
    }

    fn with_context(assets: &RaidAssets, frame: &DecodedFrame, run: impl FnOnce(&Context<'_, '_>)) {
        let loc: Localisation = [
            ("GER", "Viewer country"),
            ("ENG", "Source country"),
            ("GER_dam", "Local equipment"),
            ("nuclear_bomb", "Nuclear"),
            (
                "tooltip_raid_missing_equipment_entry",
                "$EQUIPMENT$: $AMOUNT$",
            ),
            ("tooltip_raid_on_cooldown", "cooldown $DAYS$"),
            ("raid_tooltip_trigger_title", "$TITLE$"),
            ("TRIGGER_FULLFILLED_PREFIX", "+"),
            ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let scripted_content = Err("no scripted content in this test".into());
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        run(&Context {
            assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        });
    }

    #[test]
    fn shortages_order_nukes_before_equipment_and_select_missing_or_waiting_header() {
        let assets = assets();
        let mut frame = frame();
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.push(LexicalTokenRow {
            token: engine_native::raid_tooltip_operands::NUCLEAR_BOMB_TOKEN,
            literal: b"nuclear_bomb".to_vec(),
        });
        tokens.sort_by_key(|r| r.token);
        put::<tables::LexicalTokens>(&mut frame, tokens);
        let mut kind = assets.catalog.types[0].clone();
        kind.nukes = 2;
        kind.essential_equipment = vec![("dam".into(), 4)];
        put::<tables::RaidTooltipOperands>(
            &mut frame,
            vec![
                RaidTooltipOperandRow {
                    raid: definition_key(100),
                    kind: ESSENTIAL_ORDER,
                    raw: 1,
                    ..Default::default()
                },
                RaidTooltipOperandRow {
                    raid: definition_key(100),
                    ordinal: 1,
                    kind: ESSENTIAL_ORDER,
                    key: 200,
                    ..Default::default()
                },
            ],
        );
        with_context(&assets, &frame, |context| {
            for (waiting, header) in [
                (false, "tooltip_raid_missing_equipment_header"),
                (true, "tooltip_raid_waiting_for_equipment"),
            ] {
                let (met, lines) =
                    context.shortages(&kind, false, waiting, true, Ok(1), |_| Ok(-1));
                assert!(!met);
                assert_eq!(
                    lines
                        .into_iter()
                        .map(|(_, s)| s.unwrap())
                        .collect::<Vec<_>>(),
                    [header, "Nuclear: 1", "Local equipment: 5"]
                );
            }
            assert_eq!(
                context.shortages(&kind, false, false, true, Ok(2), |_| Ok(4)),
                (true, vec![])
            );
            let additional = RaidType {
                additional_nukes: 3,
                ..Default::default()
            };
            let (_, lines) =
                context.shortages(&additional, true, true, false, Ok(1), |_| unreachable!());
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].1.as_deref(), Ok("Nuclear: 2"));
            let (_, lines) =
                context.shortages(&additional, true, true, true, Ok(1), |_| unreachable!());
            assert_eq!(lines[0].0, "tooltip_raid_waiting_for_equipment");
        });
    }

    #[test]
    fn failed_condition_chain_keeps_order_first_cooldown_and_source_country_title() {
        content_reader_script::test_game::enter();
        let assets = assets();
        let mut frame = frame();
        let mut kind = assets.catalog.types[0].clone();
        kind.fire_only_once = true;
        kind.visible = parse("always=no");
        kind.show_target = parse("always=no");
        kind.available = parse("always=no");
        kind.launchable = parse("always=no");
        kind.launchable_from = parse("always=no");
        put::<tables::RaidCountries>(
            &mut frame,
            vec![RaidCountryRow {
                canonical_slot: 2,
                country: 10,
                used_types: vec![100],
                ..Default::default()
            }],
        );
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        put::<tables::RaidCooldowns>(
            &mut frame,
            [7, -4]
                .into_iter()
                .enumerate()
                .map(|(ordinal, days)| RaidCooldownRow {
                    owner_slot: 2,
                    ordinal: ordinal as u32,
                    raid_type: 100,
                    scope: scope.clone(),
                    days,
                })
                .collect(),
        );
        with_context(&assets, &frame, |context| {
            let (result, lines) = context.conditions(
                &kind,
                &scope,
                Scopes::of(Scope::Country(2)),
                true,
                Some(Ok(Some(1))),
            );
            assert_eq!(result, Eval::False);
            assert_eq!(
                lines.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
                [
                    "raid_tooltip_trigger_title",
                    "raid visible: always",
                    "tooltip_raid_already_launched",
                    "raid_tooltip_trigger_title",
                    "raid show_target: always",
                    "tooltip_raid_on_cooldown",
                    "raid_tooltip_trigger_title",
                    "raid available: always",
                    "raid_tooltip_trigger_title",
                    "raid launchable: always",
                    "raid_tooltip_trigger_title",
                    "raid launchable_from: always",
                ]
            );
            assert_eq!(lines[5].1.as_deref(), Ok("cooldown 7"));
            assert_eq!(lines[10].1.as_deref(), Ok("Source country"));
            assert!(lines.iter().all(|(_, text)| text.is_ok()), "{lines:?}");
            let (_, preparation) =
                context.conditions(&kind, &scope, Scopes::of(Scope::Country(2)), false, None);
            assert_eq!(preparation.len(), 8);
            let (_, no_source) = context.conditions(
                &kind,
                &scope,
                Scopes::of(Scope::Country(2)),
                true,
                Some(Ok(None)),
            );
            assert_eq!(no_source.last().unwrap().0, "raid launchable_from");
            assert!(
                no_source
                    .last()
                    .unwrap()
                    .1
                    .as_ref()
                    .unwrap_err()
                    .contains("source country is null")
            );
        });
    }

    #[test]
    fn passing_conditions_emit_nothing_and_category_denial_has_no_text() {
        content_reader_script::test_game::enter();
        let mut assets = assets();
        let frame = frame();
        let mut kind = assets.catalog.types[0].clone();
        kind.show_target = Vec::new();
        with_context(&assets, &frame, |context| {
            assert_eq!(
                context.conditions(
                    &kind,
                    &RaidScope::default(),
                    Scopes::of(Scope::Country(2)),
                    true,
                    None
                ),
                (Eval::True, vec![])
            );
        });
        assets.catalog.categories.get_mut("air").unwrap().available = parse("always=no");
        with_context(&assets, &frame, |context| {
            assert_eq!(
                context.conditions(
                    &kind,
                    &RaidScope::default(),
                    Scopes::of(Scope::Country(2)),
                    true,
                    None
                ),
                (Eval::False, vec![])
            );
        });
    }

    #[test]
    fn scoped_launch_conditions_use_shared_traversal_without_changing_admission() {
        content_reader_script::test_game::enter();
        let mut assets = assets();
        assets
            .tooltip
            .state_names
            .insert(20, Ok("Target state".into()));
        let frame = frame();
        let loc: Localisation = [
            ("GER", "Viewer country"),
            ("ENG", "Source country"),
            ("raid_tooltip_trigger_title", "$TITLE$"),
            ("TRIGGER_FULLFILLED_PREFIX", "+"),
            ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
            ("TRIGGER_ALWAYS_FALSE", "never"),
            ("TRIGGER_ALWAYS_TRUE", "always"),
            ("TRIGGER_HAS_WAR", "at war"),
            ("TRIGGER_HAS_WAR_WITH", "at war with $TAG$ ($COUNTRY$)"),
            ("TRIGGER_OWNER", "Owner"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let scripted_content = Err("no scripted content in this test".into());
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let scopes = Scopes {
            from: Some(Scope::Country(1)),
            target_state: Some(20),
            ..Scopes::of(Scope::Country(2))
        };
        let mut kind = assets.catalog.types[0].clone();
        kind.show_target = Vec::new();
        kind.launchable = parse(
            "FROM = { has_war = yes } var:target_state = { owner = { always = no } } FROM = { always = no has_war_with = ROOT }",
        );
        let (result, lines) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            lines,
            vec![
                ("raid_tooltip_trigger_title".into(), Ok("Viewer country".into())),
                (
                    "raid launchable: FROM".into(),
                    Ok("-\u{11}YSource country\u{11}!: at war\n".into()),
                ),
                (
                    "raid launchable: var:target_state".into(),
                    Ok("-\u{11}YTarget state\u{11}!: Owner: never\n".into()),
                ),
                (
                    "raid launchable: FROM".into(),
                    Ok("-\u{11}YSource country\u{11}!:\n".into()),
                ),
                ("raid launchable: always".into(), Ok("   never\n".into())),
                (
                    "raid launchable: has_war_with".into(),
                    Ok("   at war with GER (Viewer country)\n".into()),
                ),
            ]
        );
        kind.launchable = parse("FROM = { always = yes }");
        assert_eq!(
            context.conditions(&kind, &RaidScope::default(), scopes, true, None),
            (Eval::True, vec![])
        );
        kind.launchable = parse("999 = { always = yes }");
        let (result, missing) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(missing.len(), 2);
        assert_eq!(missing[1].0, "raid launchable: 999");
        assert!(
            missing[1]
                .1
                .as_ref()
                .unwrap_err()
                .contains("resolves no target"),
            "{missing:?}"
        );
        kind.launchable = parse("FROM = { always = no synthetic_unported_trigger = yes }");
        let (result, partial) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            partial[1],
            (
                "raid launchable: FROM".into(),
                Ok("-\u{11}YSource country\u{11}!:\n".into()),
            )
        );
        assert_eq!(
            partial[2],
            ("raid launchable: always".into(), Ok("   never\n".into()))
        );
        assert_eq!(partial[3].0, "raid launchable: synthetic_unported_trigger");
        assert!(
            partial[3]
                .1
                .as_ref()
                .unwrap_err()
                .contains("synthetic_unported_trigger"),
            "{partial:?}"
        );
    }

    fn enter_trigger_test_game() {
        content_reader_script::test_game::with(|game| {
            use content_reader_script::trigger_keys::Reader;
            game.trigger("synthetic_unported_trigger", 0, Reader::Assign, 0)
                .trigger("set_temp_variable", 0, Reader::Body, 0)
                .trigger("add_to_temp_variable", 0, Reader::Body, 0)
                .trigger("check_variable", 0, Reader::Body, 0)
        });
    }

    #[test]
    fn shared_trigger_descriptions_handle_leaders_nested_refusals_hidden_temps_and_malformed_logic()
    {
        use replay_data::rows::UnitLeaderRow;
        let assets = assets();
        enter_trigger_test_game();
        let loc: Localisation = [
            ("GER", "Viewer country"),
            ("ENG", "Source country"),
            ("captured_general", "Captured General"),
            ("raid_tooltip_trigger_title", "$TITLE$"),
            ("TRIGGER_FULLFILLED_PREFIX", "+"),
            ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
            ("TRIGGER_ALWAYS_FALSE", "never"),
            ("TRIGGER_ALWAYS_TRUE", "always"),
            ("TRIGGER_HAS_WAR_WITH", "at war with $TAG$ ($COUNTRY$)"),
            ("TRIGGER_OR_STARTS", "either"),
            ("TRIGGER_AND_STARTS", "both"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let scripted_content = Err("no scripted content in this test".into());
        let reference = (5_u64 << 32) | 7;
        let scopes = Scopes {
            from: Some(Scope::Country(1)),
            target_leader: Some(reference),
            ..Scopes::of(Scope::Country(2))
        };
        let mut kind = assets.catalog.types[0].clone();
        kind.show_target = Vec::new();

        let mut named_frame = frame();
        put::<tables::UnitLeaders>(
            &mut named_frame,
            vec![UnitLeaderRow {
                id: 5,
                reference_type: 7,
                character_id: 5,
                owner: 10,
                inherited_text_a: b"captured_general".to_vec(),
                ..Default::default()
            }],
        );
        let named_world = World::new(&named_frame, assets.trigger_content.facts()).unwrap();
        let named_context = Context {
            assets: &assets,
            world: &named_world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(
                &named_frame,
                &loc,
            ),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        kind.launchable = parse("var:target_leader = { always = no }");
        let (result, leader_lines) =
            named_context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            leader_lines,
            vec![
                ("raid_tooltip_trigger_title".into(), Ok("Viewer country".into())),
                (
                    "raid launchable: var:target_leader".into(),
                    Ok("-\u{11}YCaptured General\u{11}!: never\n".into()),
                ),
            ]
        );

        let or_lines = named_context.trigger_text(
            &parse("OR = { always = no synthetic_unported_trigger = yes }"),
            scopes,
            "raid launchable",
        );
        assert_eq!(or_lines.len(), 3, "{or_lines:?}");
        assert_eq!(or_lines[0].0, "raid launchable: OR");
        assert!(
            or_lines[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("synthetic_unported_trigger"),
            "{or_lines:?}"
        );
        assert_eq!(
            or_lines[1],
            ("raid launchable: always".into(), Ok("   never\n".into()))
        );
        assert_eq!(or_lines[2].0, "raid launchable: synthetic_unported_trigger");
        assert!(
            or_lines[2]
                .1
                .as_ref()
                .unwrap_err()
                .contains("synthetic_unported_trigger"),
            "{or_lines:?}"
        );

        kind.launchable = parse(
            "hidden_trigger = { set_temp_variable = { x = 7 } } if = { limit = { check_variable = { temp_var:x = 7 } } always = no }",
        );
        let (result, hidden_lines) =
            named_context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            hidden_lines,
            vec![
                ("raid_tooltip_trigger_title".into(), Ok("Viewer country".into())),
                ("raid launchable: always".into(), Ok("-never\n".into())),
            ]
        );
        kind.launchable = parse(
            "hidden_trigger = { set_temp_variable = { x = 7 } } if = { limit = { check_variable = { temp_var:x = 8 } } always = no }",
        );
        assert_eq!(
            named_context.conditions(&kind, &RaidScope::default(), scopes, true, None),
            (Eval::True, vec![])
        );

        let malformed = named_context.trigger_text(
            &parse("always = no NOT = yes AND = yes OR = yes"),
            scopes,
            "raid launchable",
        );
        assert_eq!(malformed.len(), 4, "{malformed:?}");
        assert_eq!(
            malformed[0],
            ("raid launchable: always".into(), Ok("-never\n".into()))
        );
        for (entry, expected_key) in malformed[1..].iter().zip([
            "raid launchable: NOT",
            "raid launchable: AND",
            "raid launchable: OR",
        ]) {
            assert_eq!(entry.0, expected_key);
            assert!(
                entry
                    .1
                    .as_ref()
                    .unwrap_err()
                    .contains("logical trigger requires block"),
                "{malformed:?}"
            );
        }

        let mut unnamed_frame = frame();
        put::<tables::UnitLeaders>(
            &mut unnamed_frame,
            vec![UnitLeaderRow {
                id: 5,
                reference_type: 7,
                character_id: 5,
                owner: 10,
                ..Default::default()
            }],
        );
        let unnamed_world = World::new(&unnamed_frame, assets.trigger_content.facts()).unwrap();
        let unnamed_context = Context {
            assets: &assets,
            world: &unnamed_world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(
                &unnamed_frame,
                &loc,
            ),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        kind.launchable = parse("var:target_leader = { always = no }");
        let (result, unnamed_lines) =
            unnamed_context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(unnamed_lines.len(), 3, "{unnamed_lines:?}");
        assert_eq!(unnamed_lines[1].0, "raid launchable: var:target_leader");
        assert!(
            unnamed_lines[1]
                .1
                .as_ref()
                .unwrap_err()
                .contains("no recorded name"),
            "{unnamed_lines:?}"
        );
        assert_eq!(
            unnamed_lines[2],
            ("raid launchable: always".into(), Ok("   never\n".into()))
        );

        let mut missing_from_frame = base_frame();
        put::<tables::CountryAppearance>(
            &mut missing_from_frame,
            vec![CountryAppearanceRow {
                country: 10,
                ..Default::default()
            }],
        );
        put::<tables::Politics>(
            &mut missing_from_frame,
            vec![PoliticsRow {
                country: 10,
                ..Default::default()
            }],
        );
        let missing_world =
            World::new(&missing_from_frame, assets.trigger_content.facts()).unwrap();
        let missing_context = Context {
            assets: &assets,
            world: &missing_world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(
                &missing_from_frame,
                &loc,
            ),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        kind.launchable = parse("AND = { has_war_with = FROM always = no }");
        let (result, nested_lines) =
            missing_context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(nested_lines.len(), 3, "{nested_lines:?}");
        assert_eq!(nested_lines[1].0, "raid launchable: has_war_with");
        assert!(nested_lines[1].1.is_err(), "{nested_lines:?}");
        assert_eq!(
            nested_lines[2],
            ("raid launchable: always".into(), Ok("-never\n".into()))
        );
    }

    #[test]
    fn absent_prev_and_from_links_refuse_instead_of_entering_self() {
        content_reader_script::test_game::enter_viewer();
        let assets = assets();
        let frame = frame();
        let loc: Localisation = [
            ("GER", "Viewer country"),
            ("ENG", "Source country"),
            ("raid_tooltip_trigger_title", "$TITLE$"),
            ("TRIGGER_FULLFILLED_PREFIX", "+"),
            ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
            ("TRIGGER_ALWAYS_TRUE", "always"),
            ("TRIGGER_ALWAYS_FALSE", "never"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let scripted_content = Err("no scripted content in this test".into());
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let mut kind = assets.catalog.types[0].clone();
        kind.show_target = Vec::new();
        let bare = Scopes::of(Scope::Country(2));
        for (source, expected_key) in [
            ("PREV = { always = yes }", "raid launchable: PREV"),
            ("FROM = { always = yes }", "raid launchable: FROM"),
            ("ENG = { FROM = { always = yes } }", "raid launchable: FROM"),
        ] {
            kind.launchable = parse(source);
            let (result, lines) =
                context.conditions(&kind, &RaidScope::default(), bare, true, None);
            assert_eq!(result, Eval::False, "{source}");
            assert_eq!(lines.len(), 2, "{source}: {lines:?}");
            assert_eq!(lines[1].0, expected_key, "{source}: {lines:?}");
            assert!(
                lines[1]
                    .1
                    .as_ref()
                    .unwrap_err()
                    .contains("resolves no target"),
                "{source}: {lines:?}"
            );
        }
        let raid = Scopes {
            from: Some(Scope::Country(1)),
            ..bare
        };
        kind.launchable = parse("PREV = { always = yes }");
        let (result, lines) = context.conditions(&kind, &RaidScope::default(), raid, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1].0, "raid launchable: PREV");
        assert!(
            lines[1]
                .1
                .as_ref()
                .unwrap_err()
                .contains("resolves no target"),
            "{lines:?}"
        );
    }

    #[test]
    fn composite_prefix_evaluation_does_not_double_apply_hidden_temp_writes() {
        let assets = assets();
        enter_trigger_test_game();
        let mut frame = frame();
        *frame.tables[replay_data::rows::TableId::ScriptFlags.index()] =
            replay_frame::frame::TableView::Unavailable {
                reason: "flag read refused".into(),
            };
        let loc: Localisation = [
            ("GER", "Viewer country"),
            ("ENG", "Source country"),
            ("raid_tooltip_trigger_title", "$TITLE$"),
            ("TRIGGER_FULLFILLED_PREFIX", "+"),
            ("TRIGGER_UNFULLFILLED_PREFIX", "-"),
            ("TRIGGER_OR_STARTS", "either"),
            ("TRIGGER_ALWAYS_TRUE", "always"),
            ("TRIGGER_ALWAYS_FALSE", "never"),
            ("custom", "custom text"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let scripted_content = Err("no scripted content in this test".into());
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let context = Context {
            assets: &assets,
            world: &world,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &scripted_content,
            ours: 2,
            nato: false,
            countries: content_reader_country_names::country_names::CountryNames::read(&frame, &loc),
            flag_exists: &|_| true,
            scripted: Default::default(),
        };
        let scopes = Scopes {
            from: Some(Scope::Country(1)),
            ..Scopes::of(Scope::Country(2))
        };
        let mut kind = assets.catalog.types[0].clone();
        kind.show_target = Vec::new();
        kind.launchable = parse(
            "FROM = { hidden_trigger = { add_to_temp_variable = { x = 1 } } } if = { limit = { check_variable = { temp_var:x = 1 } } always = no }",
        );
        let (result, lines) = context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            lines,
            vec![
                ("raid_tooltip_trigger_title".into(), Ok("Viewer country".into())),
                ("raid launchable: always".into(), Ok("-never\n".into())),
            ]
        );
        kind.launchable = parse(
            "OR = { hidden_trigger = { add_to_temp_variable = { x = 1 } } always = no } if = { limit = { check_variable = { temp_var:x = 1 } } always = no }",
        );
        let (result, or_lines) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            or_lines.last(),
            Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
            "{or_lines:?}"
        );
        kind.launchable = parse(
            "FROM = { custom_trigger_tooltip = { tooltip = custom hidden_trigger = { add_to_temp_variable = { x = 1 } } always = yes } } if = { limit = { check_variable = { temp_var:x = 1 } } always = no }",
        );
        let (result, custom_lines) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            custom_lines.last(),
            Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
            "{custom_lines:?}"
        );
        for source in [
            "OR = { always = yes hidden_trigger = { add_to_temp_variable = { x = 1 } } } if = { limit = { check_variable = { temp_var:x = 0 } } always = no }",
            "OR = { FROM = { always = no hidden_trigger = { add_to_temp_variable = { x = 1 } } } always = yes } if = { limit = { check_variable = { temp_var:x = 0 } } always = no }",
            "OR = { NOT = { always = yes hidden_trigger = { add_to_temp_variable = { x = 1 } } } always = yes } if = { limit = { check_variable = { temp_var:x = 0 } } always = no }",
        ] {
            kind.launchable = parse(source);
            let (result, short_circuit_lines) =
                context.conditions(&kind, &RaidScope::default(), scopes, true, None);
            assert_eq!(result, Eval::False, "{source}");
            assert_eq!(
                short_circuit_lines.last(),
                Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
                "{source}: {short_circuit_lines:?}"
            );
        }
        for source in [
            "OR = { check_variable = { temp_var:x = 0 } hidden_trigger = { add_to_temp_variable = { x = 1 } } } if = { limit = { check_variable = { temp_var:x = 0 } } always = no }",
            "OR = { FROM = { check_variable = { temp_var:x = 1 } hidden_trigger = { add_to_temp_variable = { x = 1 } } } always = yes } if = { limit = { check_variable = { temp_var:x = 0 } } always = no }",
        ] {
            kind.launchable = parse(source);
            let (result, short_circuit_lines) =
                context.conditions(&kind, &RaidScope::default(), scopes, true, None);
            assert_eq!(result, Eval::False, "{source}");
            assert!(
                short_circuit_lines.iter().any(|(key, entry)| {
                    key == "raid launchable: check_variable" && entry.is_err()
                }),
                "{source}: {short_circuit_lines:?}"
            );
            assert_eq!(
                short_circuit_lines.last(),
                Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
                "{source}: {short_circuit_lines:?}"
            );
        }
        kind.launchable = parse(
            "hidden_trigger = { set_temp_variable = { x = 7 } } OR = { has_global_flag = oilfield always = yes } if = { limit = { check_variable = { temp_var:x = 7 } } always = no }",
        );
        let (result, flag_lines) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert!(
            flag_lines.iter().any(|(key, entry)| {
                key == "raid launchable: has_global_flag" && entry.is_err()
            }),
            "{flag_lines:?}"
        );
        assert_eq!(
            flag_lines.last(),
            Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
            "{flag_lines:?}"
        );
        kind.launchable = parse(
            "if = { limit = { has_global_flag = oilfield } always = yes } hidden_trigger = { set_temp_variable = { x = 7 } } if = { limit = { check_variable = { temp_var:x = 7 } } always = no }",
        );
        let (result, definite_lines) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert!(
            definite_lines
                .iter()
                .any(|(key, entry)| key == "raid launchable: if" && entry.is_err()),
            "{definite_lines:?}"
        );
        assert_eq!(
            definite_lines.last(),
            Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
            "{definite_lines:?}"
        );
        kind.launchable = parse(
            "OR = { has_global_flag = oilfield hidden_trigger = { set_temp_variable = { y = 1 } } } hidden_trigger = { set_temp_variable = { x = 7 } } if = { limit = { has_global_flag = oilfield } set_temp_variable = { x = 8 } } if = { limit = { check_variable = { temp_var:x = 7 } } always = no } always = no",
        );
        let (result, retainted_lines) =
            context.conditions(&kind, &RaidScope::default(), scopes, true, None);
        assert_eq!(result, Eval::False);
        assert_eq!(
            retainted_lines
                .iter()
                .filter(|(key, entry)| {
                    key == "raid launchable: always" && entry.as_deref() == Ok("-never\n")
                })
                .count(),
            1,
            "{retainted_lines:?}"
        );
        assert_eq!(
            retainted_lines.last(),
            Some(&("raid launchable: always".into(), Ok("-never\n".into()))),
            "{retainted_lines:?}"
        );
    }
}
