//! CCountry's balance-of-power properties (141A7B350 cases 46..58).
use super::*;
use installed_content::localisation::Argument;

const PROPERTIES: [&str; 13] = [
    "GetPowerBalanceName",
    "GetLeftSideName",
    "GetRightSideName",
    "GetActiveSideName",
    "GetTrendingSideName",
    "GetActiveRangeName",
    "GetChangeRateDesc",
    "GetBopTrendTextIcon",
    "GetPowerBalanceModDesc",
    "GetActiveRangeModDesc",
    "GetActiveRangeRuleDesc",
    "GetActiveRangeActivationEffect",
    "GetActiveRangeDeactivationEffect",
];

impl ScriptedText<'_> {
    fn active_range_effects(
        &self,
        effects: &[installed_content::parser::Item],
        refused: &mut Vec<String>,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes,
    ) -> Result<String, String> {
        if effects.is_empty() {
            return Ok(String::new());
        }
        if scopes.needs_large_stack() {
            return self.on_large_stack(scopes, |scopes| {
                self.active_range_effects(effects, refused, slot, name, scopes)
            });
        }
        let depth = scopes.fresh_depth.get();
        if scopes.active.borrow().len() + depth >= MAX_SCRIPTED_NESTING {
            return Err(nesting_refusal(name));
        }
        scopes.fresh_depth.set(depth + 1);
        let result = (|| {
            let content = self.content.effects()?;
            let mut context = crate::effect_description::Context::new(
                self,
                self.localisation,
                self.root, // Viewing country, native CContextLocalizationText +0x520/+0x524.
                name,
                content.view(None),
            )
            .with_expansion_scopes(scopes);
            // Nested BuildDescription preserves active TLS (1405354A0/140544190).
            let session = scopes.session.borrow().clone();
            if let Some(session) = &session {
                context = context.with_variable_session(session);
            }
            let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
            let description = crate::effect_description::describe(effects, &context, &mut graph);
            if session.is_some() {
                scopes.session.replace(context.variable_session());
            }
            for (reason, count) in description.refusals {
                refused.extend(std::iter::repeat_n(reason, count));
            }
            Ok(description.text)
        })();
        scopes.fresh_depth.set(depth);
        result
    }

    fn active_range_description(
        &self,
        slot: u32,
        record: &replay::rows::PowerBalanceRow,
        name: &str,
        which: usize,
        scopes: &ExpansionScopes,
    ) -> Result<String, String> {
        let definitions = self
            .frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|e| format!("{name}: {e}"))?;
        let tokens = self
            .frame
            .complete::<tables::LexicalTokens>()
            .map_err(|e| format!("{name}: {e}"))?;
        let tokens: std::collections::BTreeMap<_, _> =
            tokens.iter().map(|r| (r.token, &r.literal)).collect();
        let mut registry = std::collections::BTreeMap::new();
        for definition in definitions.iter().filter(|r| r.token != 0) {
            let literal = tokens.get(&definition.token).ok_or_else(|| {
                format!(
                    "{name}: modifier {} has no lexical token {}",
                    definition.index, definition.token
                )
            })?;
            registry.insert(literal.to_ascii_lowercase(), definition);
        }
        let database = installed_content::power_balance::read(&self.content.layers.layers, &|key| {
            Some(registry.contains_key(&key.to_ascii_lowercase()))
        })
        .map_err(|e| format!("{name}: {e}"))?;
        let Some(range) = database
            .range(&record.balance, &record.active_range)
            .map_err(|e| format!("{name}: {e}"))?
        else {
            return Ok(String::new());
        };
        let mut text = String::new();
        let key = if which == 9 {
            // 14054E050 lists registered keys by index, then custom tooltip slots.
            let mut values: Vec<_> = range
                .modifiers
                .listed()
                .map(|m| {
                    registry
                        .get(m.name.as_bytes())
                        .map(|d| (m, *d))
                        .ok_or_else(|| {
                            format!("{name}: modifier {} has no recorded definition", m.key)
                        })
                })
                .collect::<Result<_, _>>()?;
            values.sort_by_key(|(_, d)| d.index);
            let mut missing = Vec::new();
            for (modifier, definition) in values {
                if definition.category_mask == 0
                    || (definition.feature_selector != 0 && definition.eligibility == 0)
                {
                    continue;
                }
                // 140551820 clamps first; 14054E050 wraps even this unit scale.
                let value = if definition.flags & engine::modifier_definition::CLAMP_MAXIMUM != 0 {
                    modifier.value.clamp(0, 100_000)
                } else {
                    modifier.value
                };
                let value = value.wrapping_mul(100_000) / 100_000;
                if value == 0 && definition.flags & 0x100 == 0 {
                    continue;
                }
                if definition.feature_selector != 0 {
                    // Register 14054A990 writes param 9 to definition +0x68.
                    // 14054E050's getter 140551820 and formatter 14054F6D0
                    // require any selector bit, not has_feature 1401AE640.
                    // 143317CD0 -> 1401AE190 returns the recorded 143316E78.
                    let Ok([features]) = self
                        .frame
                        .complete::<tables::SimulationFeatures>()
                        .map(<[_]>::as_ref)
                    else {
                        missing.push(modifier.key.as_str());
                        continue;
                    };
                    if definition.feature_selector & features.dlc_mask == 0 {
                        continue;
                    }
                }
                let argument = self.stored_text(
                    &definition.argument,
                    &format!("{name}: modifier {} argument", modifier.key),
                )?;
                let arguments = [("ARG", Argument::Text(&argument))];
                let label = self
                    .localisation
                    .localise_with(
                        &definition.name_key,
                        if argument.is_empty() { &[] } else { &arguments },
                    )
                    .map_err(|e| e.to_string())?;
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&label);
                text.push_str(": ");
                text.push_str(&crate::modifier_text::value_text(
                    definition,
                    value,
                    2,
                    false,
                    false,
                    |key, _| self.localisation.localise(key).map_err(|e| e.to_string()),
                )?);
            }
            if !missing.is_empty() {
                return Err(format!(
                    "{name}: {} feature-gated modifiers ({}) cannot be decided without this hour's SimulationFeatures.dlc_mask",
                    missing.len(),
                    missing.join(", ")
                ));
            }
            for tooltip in range
                .modifiers
                .tooltips()
                .iter()
                .filter(|key| !key.is_empty())
            {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&self.localise(tooltip)?);
            }
            b"PROGRESS_SECTION_MODIFIER".as_slice()
        } else if which == 10 {
            // CRule::BuildChangedRulesDescription 14062CE10, registry order.
            let mut start = 0;
            for (&value, &(_, key, default, fulfilled)) in
                range.rules.iter().zip(installed_content::power_balance::RULES)
            {
                if value == default {
                    continue;
                }
                if text.len() != start {
                    text.push('\n');
                }
                start = text.len();
                text.push_str(&self.localise(if value == fulfilled {
                    "TRIGGER_FULLFILLED_PREFIX"
                } else {
                    "TRIGGER_UNFULLFILLED_PREFIX"
                })?);
                text.push_str(&self.localise(key)?);
            }
            b"PROGRESS_SECTION_RULE".as_slice()
        } else {
            let mut refused = Vec::new();
            text = self.active_range_effects(
                if which == 11 {
                    &range.activation
                } else {
                    &range.deactivation
                },
                &mut refused,
                slot,
                name,
                scopes,
            )?;
            if !refused.is_empty() {
                return Err(format!(
                    "{name}: {} unsupported effect descriptions ({}); their renderers are not ported (CEffect::BuildDescription 140532780)",
                    refused.len(),
                    refused.join(", ")
                ));
            }
            if which == 11 {
                b"PROGRESS_SECTION_ACTIVATION_EFFECT".as_slice()
            } else {
                b"PROGRESS_SECTION_DEACTIVATION_EFFECT".as_slice()
            }
        };
        if text.is_empty() {
            return Ok(text);
        }
        // 140A04DA0/140A05130/140A04500/140A047C0: text DESC, country event.
        let text = self
            .localisation
            .localise_with(key, &[("DESC", Argument::Text(&text))])
            .map_err(|e| e.to_string())?;
        let mut text = self.expand_fresh_country(&text, slot, name, scopes)?;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    pub(super) fn balance(
        &self,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes,
    ) -> Result<Option<String>, String> {
        let Some(which) = PROPERTIES.iter().position(|p| named(p, name)) else {
            return Ok(None);
        };
        // find_by_country 140E424F0: first manager record containing the country.
        // Whole membership is required, including records preceding a found one.
        let members = self
            .frame
            .complete::<tables::PowerBalanceMembers>()
            .map_err(|e| format!("{name}: {e}"))?;
        let mut selected = None;
        for row in members {
            let owner = self.scopes.slot(row.country).ok_or_else(|| {
                format!(
                    "{name}: balance member country {} is unresolved",
                    row.country
                )
            })?;
            if owner == slot {
                selected = Some(selected.map_or(row.record, |old: u32| old.min(row.record)));
            }
        }
        let records = self
            .frame
            .complete::<tables::PowerBalances>()
            .map_err(|e| format!("{name}: {e}"))?;
        let record = match selected {
            Some(id) => Some(
                records
                    .iter()
                    .find(|r| r.ordinal == id)
                    .ok_or_else(|| format!("{name}: balance record {id} is missing"))?,
            ),
            None => None,
        };
        if which < 6 {
            let Some(record) = record else {
                // Null identities are empty: 140A7C300, 140A7C610, 140A7C800.
                return Ok(Some(String::new()));
            };
            let key = match which {
                0 => &record.balance,
                1 => &record.left,
                2 => &record.right,
                3 if record.value_raw < 0 => &record.left,
                3 => &record.right,
                4 => &record.trending,
                _ => &record.active_range,
            };
            let text = self.localisation.localise(key).map_err(|e| e.to_string())?;
            // 140A7E7B0 starts a fresh empty event; the range vf9 instead
            // receives a fresh country event (140A7E780).
            if which == 5 {
                return self
                    .expand_fresh_country(&text, slot, name, scopes)
                    .map(Some);
            }
            if text.contains('[') && text.contains(']') {
                return Err(format!(
                    "{name}: the definition name requires scripted localisation in an empty event scope (140A7E7B0), which this country view cannot reproduce"
                ));
            }
            return Ok(Some(text));
        }
        if matches!(which, 9..=12) {
            return match record {
                Some(record) => self
                    .active_range_description(slot, record, name, which, scopes)
                    .map(Some),
                None => Ok(Some(String::new())),
            };
        }
        if which == 8 {
            let Some(record) = record else {
                return Ok(Some(String::new()));
            };
            let sources = self
                .frame
                .complete::<tables::PowerBalanceModifierBreakdowns>()
                .map_err(|e| format!("{name}: {e}"))?;
            let sources = sources
                .iter()
                .find(|r| r.record == record.ordinal)
                .ok_or_else(|| {
                    format!(
                        "{name}: balance record {} has no modifier breakdown",
                        record.ordinal
                    )
                })?;
            let definitions = self
                .frame
                .complete::<tables::ModifierDefinitions>()
                .map_err(|e| format!("{name}: {e}"))?;
            let mut output = String::new();
            for (key, nodes) in [
                (engine::modifier_keys::POWER_BALANCE_DAILY, &sources.daily),
                (engine::modifier_keys::POWER_BALANCE_WEEKLY, &sources.weekly),
            ] {
                let definition = definitions
                    .iter()
                    .find(|r| r.index == key)
                    .ok_or_else(|| format!("{name}: modifier definition {key} is missing"))?;
                let text = crate::modifier_text::breakdown(
                    nodes,
                    crate::modifier_text::Breakdown {
                        depth: -1,
                        prefix: "",
                        suffix: "",
                    },
                    |node| {
                        let mut text = if node.name.is_empty() {
                            String::new()
                        } else {
                            self.localisation
                                .localise(&node.name)
                                .map_err(|e| e.to_string())?
                        };
                        if !text.is_empty() {
                            text.push_str(": ");
                        }
                        text.push_str(&crate::modifier_text::value_text(
                            definition,
                            node.value,
                            2,
                            false,
                            false,
                            |key, required| {
                                if required
                                    && self
                                        .localisation
                                        .get(std::str::from_utf8(key).map_err(|_| {
                                            "modifier value key is not UTF-8".to_owned()
                                        })?)
                                        .is_none()
                                {
                                    return Err(format!(
                                        "{name}: modifier value localisation {} is missing",
                                        String::from_utf8_lossy(key)
                                    ));
                                }
                                self.localisation.localise(key).map_err(|e| e.to_string())
                            },
                        )?);
                        Ok(text)
                    },
                )?;
                // 140E422E0 adds a separator after nonempty daily text only
                // when the breakdown has not already ended with a newline.
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&text);
            }
            return Ok(Some(output));
        }
        let (mut daily, mut weekly) = (0_i64, 0_i64);
        if let Some(record) = record {
            for m in members.iter().filter(|m| m.record == record.ordinal) {
                let native = engine::power_balance_record::MemberRow {
                    daily_raw: m.daily_raw,
                    weekly_raw: m.weekly_raw,
                    daily_flags: m.daily_flags,
                    weekly_flags: m.weekly_flags,
                    ..Default::default()
                };
                daily = daily.wrapping_add(native.daily_effective());
                weekly = weekly.wrapping_add(native.weekly_effective());
            }
        }
        // 140E41430: signed 64-bit seven-times-daily plus weekly.
        let weekly_change = daily.wrapping_mul(7);
        let weekly_change = weekly_change.wrapping_add(weekly);
        if which == 7 {
            // Executable literals and trailing space at 142700920.
            return Ok(Some(if weekly_change == 0 {
                String::new()
            } else {
                format!(
                    "\u{13}BoP_{}_texticon ",
                    if weekly_change < 0 { "left" } else { "right" }
                )
            }));
        }
        // 140E420B0 preserves its sentinel path and wrapping fixed5 product.
        let converted = if !(i64::MIN + 2..=i64::MAX - 1).contains(&weekly) {
            0xffff_ffff
        } else {
            weekly.wrapping_mul(100_000) / 700_000
        };
        let change = daily.wrapping_add(converted);
        let (key, value) = if change == 0 {
            (b"BOP_CHANGE_RATE_WEEKLY".as_slice(), weekly_change)
        } else {
            (b"BOP_CHANGE_RATE_DAILY".as_slice(), change)
        };
        self.localisation
            .localise_with(key, &[("VALUE", Argument::Fixed5(value.wrapping_abs()))])
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
