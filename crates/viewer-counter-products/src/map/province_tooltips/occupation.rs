use super::{
    Context,
    mode_lines::{FIXED5 as ONE, OccupationDefines, mul, percent},
};
use content_reader_localisation::localisation::Argument as Arg;
use replay_data::{rows, rows::Recorded, rows::tables};

fn add(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}
fn gap(lines: &mut Vec<String>) {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(String::new());
}
fn points(a: i64, b: i64) -> i64 {
    i64::from((a.wrapping_mul(b) / 10_000_000_000) as i32).wrapping_mul(ONE)
}
fn bounded(value: i64, minimum: i64, maximum: i64) -> i64 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}
fn ratio(legitimacy: i64, maximum: i64) -> i64 {
    if maximum == 0 || [i64::MIN, i64::MIN + 1, i64::MAX].contains(&legitimacy) {
        i64::from(u32::MAX)
    } else {
        legitimacy.wrapping_mul(ONE).wrapping_div(maximum)
    }
}

fn resistance_rate_section(level: i64, target: i64, reenable: i64) -> Option<u8> {
    if (level == 0 && target < reenable) || level == target {
        None
    } else {
        Some(if level < target { 1 } else { 2 })
    }
}

fn resistance_decay_low(d: &OccupationDefines, level: i64) -> Option<i64> {
    d.decay_factors
        .iter()
        .find(|&&(threshold, _)| level < threshold)
        .map(|&(_, factor)| percent(factor))
        .filter(|&factor| factor != 0)
}

fn resistance_rate(
    d: &OccupationDefines,
    level: i64,
    target: i64,
    claim: bool,
    modifier: i64,
) -> (i64, i64, Option<i64>) {
    if level < target {
        let raw = mul(add(ONE, modifier), d.growth_base);
        (
            bounded(raw, d.growth_min, d.growth_max).min(target.wrapping_sub(level)),
            raw,
            None,
        )
    } else {
        let low = resistance_decay_low(d, level);
        let factor = add(
            add(
                add(ONE, if claim { percent(d.decay_claim) } else { 0 }),
                low.unwrap_or(0),
            ),
            modifier,
        );
        let raw = mul(factor, d.decay_base);
        (
            bounded(raw, d.decay_min, d.decay_max).min(level.wrapping_sub(target)),
            raw,
            low,
        )
    }
}

pub(super) fn base_value(value: i64, colour: char) -> String {
    format!(
        "\u{11}{colour}{}%\u{11}!",
        content_reader_parser::fixed5::fixed5_to_string(value, 3)
    )
}

fn source_precision(section: u8, source: u8, precision: i32) -> i32 {
    if section == 4 && source == 1 {
        2
    } else {
        precision
    }
}

fn source_total_value(source: &rows::OccupationModifierSource) -> Result<i64, String> {
    source.total.ok_or_else(|| {
        format!(
            "occupation modifier total unavailable: section {}, source {}, key {}: {}",
            source.section,
            source.source,
            source.key,
            String::from_utf8_lossy(&source.failure),
        )
    })
}

fn source_total(sources: &[rows::OccupationModifierSource], section: u8) -> Result<i64, String> {
    let (key, occupied_key) = engine_occupation::cresistance::hover::KEYS[section as usize];
    if sources.iter().filter(|row| row.section == section).count()
        != if occupied_key == 0 { 3 } else { 4 }
    {
        return Err(format!(
            "occupation modifier section {section} has unsupported sources"
        ));
    }
    let mut total = 0_i64;
    for source in 0..if occupied_key == 0 { 3 } else { 4 } {
        let key = if source == 3 { occupied_key } else { key };
        let mut matching = sources
            .iter()
            .filter(|row| row.section == section && row.source == source && row.key == key);
        let row = matching
            .next()
            .ok_or("occupation modifier source is missing")?;
        if matching.next().is_some() {
            return Err("occupation modifier source is duplicated".into());
        }
        total = add(total, source_total_value(row)?);
    }
    Ok(total)
}

fn validate_sources(
    sources: &[rows::OccupationModifierSource],
    resistance_only: bool,
) -> Result<(), String> {
    let count = if resistance_only { 12 } else { 22 };
    let needed = sources
        .iter()
        .filter(|source| !resistance_only || source.section < 3);
    if needed.clone().count() != count
        || needed
            .clone()
            .any(|source| source.section >= 6 || source.source >= 4)
    {
        return Err("occupation modifier source identities are incomplete or unsupported".into());
    }
    Ok(())
}

impl Context<'_> {
    pub(super) fn occupation_complete(
        &self,
        row: &rows::StateResistanceRow,
        hover: &rows::OccupationHover,
        sources: &[rows::OccupationModifierSource],
        refused: &mut Vec<(&'static str, String)>,
    ) -> Result<String, String> {
        self.occupation_sections(row, hover, sources, refused, false)
    }

    pub(super) fn occupation_marker(
        &self,
        row: &rows::StateResistanceRow,
        hover: &rows::OccupationHover,
        sources: &[rows::OccupationModifierSource],
        refused: &mut Vec<(&'static str, String)>,
    ) -> Result<String, String> {
        self.occupation_sections(row, hover, sources, refused, true)
    }

    fn occupation_sections(
        &self,
        row: &rows::StateResistanceRow,
        hover: &rows::OccupationHover,
        sources: &[rows::OccupationModifierSource],
        refused: &mut Vec<(&'static str, String)>,
        resistance_only: bool,
    ) -> Result<String, String> {
        validate_sources(sources, resistance_only)?;
        let d = self
            .content
            .province_hover
            .occupation
            .as_ref()
            .map_err(Clone::clone)?;
        let state = self.state_row()?;
        let claim = self.occupation_claim()?;
        let stability = self.occupation_stability()?;
        let peace = stability.at_war == 0;
        let stability = replay_data::country_stability::stability(stability)
            .value
            .ok_or("controller stability was refused")?;
        let Recorded::Value(population) = state.population else {
            return Err("state population is not recorded".into());
        };
        let legitimacy = ratio(hover.legitimacy, d.max_legitimacy);
        let mut target = d.target_base;
        let mut terms = vec![self.value("MODIFIER_BASE_VALUE_PERCENTAGE", percent(d.target_base))?];
        for (key, value) in [
            (
                "RESISTANCE_TARGET_MODIFIER_HAS_CLAIM",
                if claim { d.claim } else { 0 },
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_POP_LOW",
                d.population(population),
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_IS_AT_PEACE",
                if peace { d.peace } else { 0 },
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_PER_STABILITY_LOSS",
                d.stability_loss(stability),
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_OCCUPIED_CAPITULATED",
                if self.capitulated(row.country)? {
                    d.occupied_capitulated
                } else {
                    0
                },
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_OCCUPIED_IS_EXILE",
                if hover.exile_host > 0 {
                    add(
                        d.exile_min,
                        mul(d.exile_max.wrapping_sub(d.exile_min), legitimacy),
                    )
                } else {
                    0
                },
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_STATE_VP",
                d.state_vp(self.state_victory_points()?),
            ),
        ] {
            target = add(target, value);
            if value != 0 {
                terms.push(self.value(key, percent(value))?);
            }
        }
        let mut target =
            source_total(sources, 0).map(|modifier| add(target, modifier.wrapping_mul(100)));
        self.source_lines(sources, 0, 2, &mut terms, refused);
        for added in &hover.added {
            if self.occupation_selector(added.controller, state.controller)?
                && self.occupation_selector(added.occupied, row.country)?
            {
                target = target.map(|target| add(target, added.value));
                if !added.localisation.is_empty() {
                    let key = std::str::from_utf8(
                        added.localisation.split(|byte| *byte == 0).next().unwrap(),
                    )
                    .map_err(|_| "added resistance target localisation key is not UTF-8")?;
                    terms.push(self.value(key, percent(added.value))?);
                }
            }
        }
        if hover.flow > 0 {
            target = target.map(|target| add(target, hover.flow));
            let state = if hover.flow_from == 0 {
                String::new()
            } else {
                self.state_name_of(hover.flow_from)?
            };
            terms.push(self.loc(
                "RESISTANCE_TARGET_FLOW",
                &[
                    ("FLOW", Arg::Fixed5(percent(hover.flow))),
                    ("STATE", Arg::Text(&state)),
                ],
            )?);
        }
        if let Ok(current) = &mut target
            && *current < d.target_floor
            && d.target_floor > 0
        {
            terms.push(self.loc(
                "RESISTANCE_TARGET_CAP_FOR_NON_COMPLIANCE",
                &[
                    ("CUR_TARGET", Arg::Fixed5(percent(*current))),
                    ("CAP", Arg::Fixed5(percent(d.target_floor))),
                ],
            )?);
            *current = d.target_floor;
        }
        let compliance_term = points(row.compliance, d.per_compliance);
        if compliance_term != 0 {
            target = target.map(|target| add(target, compliance_term));
            terms.push(self.value(
                "RESISTANCE_TARGET_MODIFIER_PER_COMPLIANCE",
                percent(compliance_term),
            )?);
        }
        let mut lines = if resistance_only {
            Vec::new()
        } else {
            vec![
                format!("\u{11}H{}\u{11}!", self.state_name()?),
                String::new(),
            ]
        };
        lines.extend([
            self.loc(
                "RESISTANCE_STRENGTH",
                &[("TOT", Arg::Fixed5(percent(row.level)))],
            )?,
            String::new(),
        ]);
        let target = match target {
            Ok(uncapped) => {
                let target = bounded(uncapped, 0, engine_occupation::cresistance::MAX_LEVEL);
                lines.push(self.capped(
                    "RESISTANCE_TARGET_VALUE",
                    "RESISTANCE_TARGET_VALUE_CAPPED",
                    percent(target),
                    percent(uncapped),
                )?);
                Some(target)
            }
            Err(why) => {
                refused.push((
                    "RESISTANCE_TARGET_VALUE, RESISTANCE_TARGET_VALUE_CAPPED",
                    why.clone(),
                ));
                if d.target_floor > 0 {
                    refused.push(("RESISTANCE_TARGET_CAP_FOR_NON_COMPLIANCE", why.clone()));
                }
                refused.push((
                    "RESISTANCE_GROWTH_VALUE, RESISTANCE_DECAY_VALUE, RESISTANCE_DISABLED_INFO",
                    why,
                ));
                None
            }
        };
        lines.extend(terms);
        gap(&mut lines);
        if let Some(target) = target {
            if row.level == 0 && target < d.reenable {
                lines.push(self.loc(
                    "RESISTANCE_DISABLED_INFO",
                    &[("LIMIT", Arg::Fixed5(percent(d.reenable)))],
                )?);
            } else if let Some(section) = resistance_rate_section(row.level, target, d.reenable) {
                let growing = section == 1;
                let rate = source_total(sources, section)
                    .map(|modifier| resistance_rate(d, row.level, target, claim, modifier));
                let (key, capped) = if growing {
                    ("RESISTANCE_GROWTH_VALUE", "RESISTANCE_GROWTH_VALUE_CAPPED")
                } else {
                    ("RESISTANCE_DECAY_VALUE", "RESISTANCE_DECAY_VALUE_CAPPED")
                };
                let low = match rate {
                    Ok((value, raw, low)) => {
                        lines.push(self.capped(key, capped, value, raw)?);
                        low
                    }
                    Err(why) => {
                        refused.push((key, why));
                        if growing {
                            None
                        } else {
                            resistance_decay_low(d, row.level)
                        }
                    }
                };
                lines.push(self.base(
                    if growing { d.growth_base } else { d.decay_base },
                    if growing { 'R' } else { 'G' },
                )?);
                if !growing {
                    if claim && d.decay_claim != 0 {
                        lines.push(self.value(
                            "RESISTANCE_DECAY_MODIFIER_HAS_CLAIM",
                            percent(d.decay_claim),
                        )?);
                    }
                    if let Some(low) = low {
                        lines.push(self.value("RESISTANCE_DECAY_MODIFIER_LOW_RESISTANCE", low)?);
                    }
                }
                self.source_lines(sources, section, 2, &mut lines, refused);
            }
        }
        gap(&mut lines);
        self.occupation_effects(
            "RESISTANCE_EFFECTS",
            &hover.resistance_effects,
            &hover.resistance_extra,
            &mut lines,
            refused,
        );
        if resistance_only {
            return Ok(lines.join("\n").trim_end_matches('\n').to_owned());
        }
        gap(&mut lines);
        lines.push(self.loc(
            "COMPLIANCE_STRENGTH",
            &[("TOT", Arg::Fixed5(percent(row.compliance)))],
        )?);
        gap(&mut lines);
        let mut growth = vec![self.base(d.compliance_growth_base, 'G')?];
        let mut factor = ONE;
        for (key, value) in [
            (
                "COMPLIANCE_GROWTH_HAS_CLAIM",
                if claim {
                    percent(d.compliance_claim)
                } else {
                    0
                },
            ),
            (
                "COMPLIANCE_GROWTH_IS_AT_PEACE",
                if peace {
                    percent(d.compliance_peace)
                } else {
                    0
                },
            ),
        ] {
            factor = add(factor, value);
            if value != 0 {
                growth.push(self.value(key, value)?);
            }
        }
        let factor = source_total(sources, 3).map(|modifier| add(factor, modifier));
        self.source_lines(sources, 3, 2, &mut growth, refused);
        let decay = mul(percent(row.compliance), d.decay_at_max_compliance);
        let exile_decay = mul(legitimacy, d.exile_decay);
        let gain = source_total(sources, 4);
        let mut extra = Vec::new();
        if decay != 0 {
            extra.push(self.value("COMPLIANCE_DECAY_DUE_TO_COMPLIANCE", decay)?);
        }
        if exile_decay != 0 {
            extra.push(self.value("COMPLIANCE_DECAY_PER_EXILE_LEGITIMACY", exile_decay)?);
        }
        self.source_lines(sources, 4, 3, &mut extra, refused);
        if !extra.is_empty() {
            growth.push(self.loc("COMPLIANCE_DECAY_INFO", &[])?);
            growth.extend(extra);
        }
        let mut raw = factor.and_then(|factor| {
            gain.map(|gain| {
                add(
                    add(
                        add(mul(d.compliance_growth_base, factor), decay),
                        exile_decay,
                    ),
                    gain,
                )
            })
        });
        match source_total(sources, 5) {
            Ok(no_gain) if no_gain > d.no_gain_limit => {
                raw = raw.map(|raw| raw.min(0));
                growth.push(self.loc("COMPLIANCE_GAIN_IS_DISABLED", &[])?);
            }
            Ok(no_gain) if no_gain > 0 => growth.push(self.loc(
                "COMPLIANCE_GAIN_IS_SCALED",
                &[("RATIO", Arg::Fixed5(hover.gain_ratio))],
            )?),
            Ok(_) => {}
            Err(why) => {
                raw = Err(why.clone());
                refused.push((
                    "COMPLIANCE_GAIN_IS_DISABLED, COMPLIANCE_GAIN_IS_SCALED",
                    why,
                ));
            }
        }
        match raw {
            Ok(raw) => {
                let value = bounded(raw, d.compliance_min, d.compliance_max);
                let (key, capped) = if value >= 0 {
                    ("COMPLIANCE_GROWTH_VALUE", "COMPLIANCE_GROWTH_VALUE_CAPPED")
                } else {
                    ("COMPLIANCE_DECAY_VALUE", "COMPLIANCE_DECAY_VALUE_CAPPED")
                };
                lines.push(self.capped(key, capped, value, raw)?);
            }
            Err(why) => refused.push(("COMPLIANCE_GROWTH_VALUE, COMPLIANCE_DECAY_VALUE", why)),
        }
        lines.extend(growth);
        gap(&mut lines);
        self.occupation_effects(
            "COMPLIANCE_EFFECTS",
            &hover.compliance_effects,
            &hover.compliance_extra,
            &mut lines,
            refused,
        );
        Ok(lines.join("\n").trim_end_matches('\n').to_owned())
    }

    fn value(&self, key: &str, value: i64) -> Result<String, String> {
        self.loc(key, &[("VALUE", Arg::Fixed5(value))])
    }
    fn capped(&self, key: &str, capped: &str, value: i64, raw: i64) -> Result<String, String> {
        self.loc(
            if value == raw { key } else { capped },
            &[
                ("VALUE", Arg::Fixed5(value)),
                ("VALUE_NON_CAPPED", Arg::Fixed5(raw)),
            ],
        )
    }
    fn base(&self, value: i64, colour: char) -> Result<String, String> {
        let value = base_value(value, colour);
        self.loc("MODIFIER_BASE_VALUE_STR", &[("VALUE", Arg::Text(&value))])
    }
    fn occupation_selector(&self, selector: i32, country: u32) -> Result<bool, String> {
        Ok(selector < 1
            || selector as u32 == country
            || (country != 0 && self.slot(selector as u32)? == self.slot(country)?))
    }
    fn modifier_definition(&self, key: u32) -> Result<&rows::ModifierDefinitionRow, String> {
        self.frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.index == key)
            .ok_or_else(|| format!("modifier definition {key} is absent"))
    }
    fn modifier_value(
        &self,
        definition: &rows::ModifierDefinitionRow,
        value: i64,
        precision: i32,
        invert: bool,
    ) -> Result<String, String> {
        dashboard_text::modifier_text::value_text(
            definition,
            value,
            precision,
            false,
            invert,
            |key, executable| {
                if executable {
                    let key = std::str::from_utf8(key)
                        .map_err(|_| "modifier value key is not UTF-8".to_owned())?;
                    viewer_foundation::tooltip_text::builtin(
                        self.content.text_icons.as_ref(),
                        &self.content.localisation,
                        key,
                        &[],
                    )
                } else {
                    viewer_foundation::tooltip_text::token(
                        self.content.text_icons.as_ref(),
                        &self.content.localisation,
                        key,
                    )
                }
            },
        )
    }
    fn source_lines(
        &self,
        sources: &[rows::OccupationModifierSource],
        section: u8,
        precision: i32,
        lines: &mut Vec<String>,
        refused: &mut Vec<(&'static str, String)>,
    ) {
        for source in sources
            .iter()
            .filter(|row| row.section == section && row.total != Some(0))
        {
            let text = (|| {
                source_total_value(source)?;
                if !source.failure.is_empty() {
                    return Err(String::from_utf8_lossy(&source.failure).into_owned());
                }
                let definition = self.modifier_definition(source.key)?;
                dashboard_text::modifier_text::breakdown(
                    &source.nodes,
                    dashboard_text::modifier_text::Breakdown {
                        depth: -1,
                        prefix: "",
                        suffix: "",
                    },
                    |node| {
                        let mut text = self.content.province_hover.labels.text(
                            &self.content.localisation,
                            &node.name,
                            "occupation modifier source",
                        )?;
                        if !text.is_empty() {
                            text.push_str(": ");
                        }
                        text.push_str(&self.modifier_value(
                            definition,
                            node.value,
                            source_precision(section, source.source, precision),
                            source.source == 3,
                        )?);
                        Ok(text)
                    },
                )
            })();
            match text {
                Ok(text) if !text.is_empty() => lines.push(text.trim_end_matches('\n').to_owned()),
                Ok(_) => {}
                Err(why) => refused.push((
                    "occupation modifier source",
                    format!(
                        "section {section}, source {}, key {}: {why}",
                        source.source, source.key
                    ),
                )),
            }
        }
    }
    fn occupation_effects(
        &self,
        key: &'static str,
        effects: &[rows::OccupationEffect],
        extra: &rows::OccupationEffectExtra,
        lines: &mut Vec<String>,
        refused: &mut Vec<(&'static str, String)>,
    ) {
        append_effect_block(
            key,
            effects,
            extra,
            || self.loc(key, &[]),
            |effect| {
                let definition = self.modifier_definition(effect.key)?;
                if effect.value == 0 && definition.flags & (1 << 8) == 0 {
                    return Ok(String::new());
                }
                let argument = std::str::from_utf8(&definition.argument)
                    .map_err(|_| "modifier argument is not UTF-8".to_owned())?;
                let name = viewer_foundation::tooltip_text::token_with(
                    self.content.text_icons.as_ref(),
                    &self.content.localisation,
                    &definition.name_key,
                    &[("ARG", Arg::Text(argument))],
                )?;
                Ok(format!(
                    "{name}: {}",
                    self.modifier_value(definition, effect.value, 2, false)?
                ))
            },
            (lines, refused),
        );
    }
}

fn append_effect_block(
    key: &'static str,
    effects: &[rows::OccupationEffect],
    extra: &rows::OccupationEffectExtra,
    header: impl FnOnce() -> Result<String, String>,
    mut render: impl FnMut(&rows::OccupationEffect) -> Result<String, String>,
    (lines, refused): (&mut Vec<String>, &mut Vec<(&'static str, String)>),
) {
    if effects.is_empty() && extra.child_sources == 0 && extra.custom_tooltips == 0 {
        return;
    }
    if extra.custom_tooltips != 0 {
        refused.push((
            key,
            format!(
                "{} custom modifier tooltip keys are not recorded",
                extra.custom_tooltips
            ),
        ));
    }
    match header() {
        Ok(text) => lines.push(text),
        Err(why) => refused.push((key, why)),
    }
    for effect in effects {
        match render(effect) {
            Ok(text) if !text.is_empty() => lines.push(text),
            Ok(_) => {}
            Err(why) => refused.push((key, format!("modifier {}: {why}", effect.key))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occupation_marker_resistance_section_needs_no_compliance_source_packet() {
        let mut sources: Vec<_> = (0..3)
            .flat_map(|section| {
                (0..4).map(move |source| rows::OccupationModifierSource {
                    section,
                    source,
                    total: Some(0),
                    ..Default::default()
                })
            })
            .collect();
        assert!(validate_sources(&sources, true).is_ok());
        assert!(validate_sources(&sources, false).is_err());
        sources.push(rows::OccupationModifierSource {
            section: 3,
            source: 0,
            total: None,
            failure: b"compliance-only source refused".to_vec(),
            ..Default::default()
        });
        assert!(validate_sources(&sources, true).is_ok());
        sources[0].source = 4;
        assert!(validate_sources(&sources, true).is_err());
    }
    #[test]
    fn occupation_effect_children_use_aggregate_rows_without_refusal_or_duplication() {
        let effects = [
            rows::OccupationEffect {
                key: 7,
                value: 5_000,
            },
            rows::OccupationEffect {
                key: 8,
                value: 1_620,
            },
        ];
        let mut expected = None;
        for children in [0, 1, 5_000] {
            let mut lines = vec!["previous section".into(), String::new()];
            let mut refused = Vec::new();
            let mut visited = Vec::new();
            append_effect_block(
                "COMPLIANCE_EFFECTS",
                &effects,
                &rows::OccupationEffectExtra {
                    child_sources: children,
                    custom_tooltips: 0,
                },
                || Ok("Effects".into()),
                |effect| {
                    visited.push(effect.key);
                    Ok(format!("{}: {}", effect.key, effect.value))
                },
                (&mut lines, &mut refused),
            );
            assert_eq!(visited, [7, 8]);
            assert_eq!(
                lines,
                ["previous section", "", "Effects", "7: 5000", "8: 1620"]
            );
            assert!(refused.is_empty());
            assert_eq!(expected.get_or_insert_with(|| lines.clone()), &lines);
        }
        let mut lines = Vec::new();
        let mut refused = Vec::new();
        append_effect_block(
            "RESISTANCE_EFFECTS",
            &[],
            &rows::OccupationEffectExtra {
                child_sources: 1,
                custom_tooltips: 0,
            },
            || Ok("Effects".into()),
            |_| unreachable!(),
            (&mut lines, &mut refused),
        );
        assert_eq!(lines, ["Effects"]);
        assert!(refused.is_empty());
    }

    #[test]
    fn occupation_unprinted_decay_root_total_does_not_refuse_growth_or_other_sections() {
        let mut sources = engine_occupation::cresistance::hover::KEYS
            .iter()
            .enumerate()
            .flat_map(|(section, &(key, occupied))| {
                (0..if occupied == 0 { 3 } else { 4 }).map(move |source| {
                    rows::OccupationModifierSource {
                        section: section as u8,
                        source,
                        key: if source == 3 { occupied } else { key },
                        total: Some(0),
                        ..Default::default()
                    }
                })
            })
            .collect::<Vec<_>>();
        let decay = sources
            .iter_mut()
            .find(|source| source.section == 2 && source.source == 0)
            .unwrap();
        decay.total = None;
        decay.failure = b"fixture root value unreadable".to_vec();
        let selected = resistance_rate_section(ONE, 2 * ONE, ONE).unwrap();
        assert_eq!(selected, 1);
        assert_eq!(source_total(&sources, selected).unwrap(), 0);
        for section in [0, 3, 4, 5] {
            assert_eq!(source_total(&sources, section).unwrap(), 0);
        }
        let why = source_total(&sources, 2).unwrap_err();
        assert!(why.contains("section 2, source 0"));
        assert!(why.contains("fixture root value unreadable"));
        sources
            .iter_mut()
            .find(|source| source.section == 2 && source.source == 0)
            .unwrap()
            .failure
            .clear();
        assert!(source_total(&sources, 2).is_err());
        sources
            .iter_mut()
            .find(|source| source.section == 2 && source.source == 0)
            .unwrap()
            .total = Some(0);
        assert_eq!(source_total(&sources, 2).unwrap(), 0);
    }

    #[test]
    fn occupation_rate_tree_selection_uses_current_target_even_with_stale_or_zero_speeds() {
        let mut row = rows::StateResistanceRow {
            level: ONE,
            speed: -1_000,
            active: 1,
            ..Default::default()
        };
        let target = 2 * ONE;
        assert!(row.speed < 0);
        assert_eq!(resistance_rate_section(row.level, target, ONE), Some(1));
        let d = OccupationDefines::default();
        assert_eq!(resistance_rate(&d, row.level, target, false, 0).0, 0);
        row.speed = 1_000;
        assert!(row.speed > 0);
        assert_eq!(resistance_rate_section(row.level, 0, ONE), Some(2));
        assert_eq!(resistance_rate_section(row.level, row.level, ONE), None);
        row.level = 0;
        row.speed = 0;
        assert_eq!(row.speed, 0);
        assert_eq!(resistance_rate_section(row.level, ONE - 1, ONE), None);
        assert_eq!(resistance_rate_section(row.level, ONE, ONE), Some(1));
    }

    #[test]
    fn occupation_rate_bases_keep_percentage_inside_the_colour_span() {
        for colour in ['R', 'G'] {
            assert_eq!(
                base_value(7_500, colour),
                format!("\u{11}{colour}0.075%\u{11}!")
            );
        }
    }

    #[test]
    fn occupation_additional_compliance_state_precision_is_two_country_and_relation_three() {
        let definition = rows::ModifierDefinitionRow {
            flags: dashboard_text::modifier_text::PERCENT,
            precision: 3,
            ..Default::default()
        };
        for (source, expected) in [(0, "-0.012%"), (1, "-0.01%"), (2, "-0.012%")] {
            let text = dashboard_text::modifier_text::value_text(
                &definition,
                -1_234,
                source_precision(4, source, 3),
                false,
                false,
                |_, _| Err("unexpected localisation".into()),
            )
            .unwrap();
            assert!(text.contains(expected), "source {source}: {text:?}");
        }
        assert_eq!(source_precision(0, 1, 2), 2);
        assert_eq!(source_precision(3, 1, 2), 2);
    }

    #[test]
    fn occupation_rate_clamps_after_factors_and_then_to_target_distance() {
        let d = OccupationDefines {
            growth_base: 20_000,
            growth_min: 1_000,
            growth_max: 200_000,
            decay_base: 10_000,
            decay_min: 1_000,
            decay_max: 200_000,
            decay_claim: 2_500_000,
            decay_factors: vec![(2_000_000, 5_000_000)],
            ..Default::default()
        };
        assert_eq!(
            resistance_rate(&d, ONE, ONE + 17, false, 50_000),
            (17, 30_000, None)
        );
        assert_eq!(
            resistance_rate(&d, ONE, ONE - 3, true, 0),
            (3, 17_500, Some(50_000))
        );
        assert_eq!(
            resistance_rate(&d, 2_000_000, 0, true, 0),
            (12_500, 12_500, None)
        );
        assert_eq!(
            resistance_rate(&d, ONE, ONE + 30_000, false, -200_000),
            (1_000, -20_000, None)
        );
    }
    #[test]
    fn occupation_target_compliance_truncates_before_the_floor_and_legitimacy_keeps_sentinels() {
        assert_eq!(points(3_999_999, -50_000), -19 * ONE);
        assert_eq!(points(4_000_000, -50_000), -20 * ONE);
        assert_eq!(ratio(7_500_000, 10_000_000), 75_000);
        assert_eq!(ratio(1, 0), i64::from(u32::MAX));
    }
}
