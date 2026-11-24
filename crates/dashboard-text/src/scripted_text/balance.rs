use super::*;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{
    ModifierDefinitionRow, PowerBalanceMemberRow, PowerBalanceModifierBreakdownRow,
    PowerBalanceRow, SimulationFeatureRow,
};

const DEFINITION_TEXT: u64 = mask::<ModifierDefinitionRow>(&[
    "token",
    "name_key",
    "postfix_key",
    "argument",
    "flags",
    "category_mask",
    "feature_selector",
    "precision",
    "eligibility",
]);
const DEFINITION_VALUE: u64 = mask::<ModifierDefinitionRow>(&["flags", "precision", "postfix_key"]);
const DLC: u64 = mask::<SimulationFeatureRow>(&["dlc_mask"]);
const MEMBER: u64 = mask::<PowerBalanceMemberRow>(&[
    "country",
    "daily_raw",
    "weekly_raw",
    "daily_flags",
    "weekly_flags",
]);
const BALANCE: u64 = mask::<PowerBalanceRow>(&[
    "value_raw",
    "balance",
    "left",
    "right",
    "trending",
    "active_range",
]);
const SOURCES: u64 = mask::<PowerBalanceModifierBreakdownRow>(&["sources"]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BalanceProperty {
    Name,
    LeftSideName,
    RightSideName,
    ActiveSideName,
    TrendingSideName,
    ActiveRangeName,
    ChangeRateDescription,
    TrendIcon,
    ModifierDescription,
    ActiveRangeModifiers,
    ActiveRangeRules,
    ActivationEffects,
    DeactivationEffects,
}

impl BalanceProperty {
    fn parse(name: &str) -> Option<Self> {
        Some(if named(name, "GetPowerBalanceName") {
            Self::Name
        } else if named(name, "GetLeftSideName") {
            Self::LeftSideName
        } else if named(name, "GetRightSideName") {
            Self::RightSideName
        } else if named(name, "GetActiveSideName") {
            Self::ActiveSideName
        } else if named(name, "GetTrendingSideName") {
            Self::TrendingSideName
        } else if named(name, "GetActiveRangeName") {
            Self::ActiveRangeName
        } else if named(name, "GetChangeRateDesc") {
            Self::ChangeRateDescription
        } else if named(name, "GetBopTrendTextIcon") {
            Self::TrendIcon
        } else if named(name, "GetPowerBalanceModDesc") {
            Self::ModifierDescription
        } else if named(name, "GetActiveRangeModDesc") {
            Self::ActiveRangeModifiers
        } else if named(name, "GetActiveRangeRuleDesc") {
            Self::ActiveRangeRules
        } else if named(name, "GetActiveRangeActivationEffect") {
            Self::ActivationEffects
        } else if named(name, "GetActiveRangeDeactivationEffect") {
            Self::DeactivationEffects
        } else {
            return None;
        })
    }

    fn from_country_ordinal(ordinal: u32) -> Option<Self> {
        Some(match ordinal {
            46 => Self::Name,
            47 => Self::ModifierDescription,
            48 => Self::LeftSideName,
            49 => Self::RightSideName,
            50 => Self::ActiveSideName,
            51 => Self::TrendingSideName,
            52 => Self::ActiveRangeName,
            53 => Self::ActiveRangeModifiers,
            54 => Self::ActiveRangeRules,
            55 => Self::ActivationEffects,
            56 => Self::DeactivationEffects,
            57 => Self::ChangeRateDescription,
            58 => Self::TrendIcon,
            _ => return None,
        })
    }
}

pub(super) fn matches_numeric_handler(name: &str, ordinal: u32) -> bool {
    let handler = BalanceProperty::from_country_ordinal(ordinal);
    handler.is_some() && BalanceProperty::parse(name) == handler
}

impl<'a> ScriptedText<'a> {
    fn active_range_effects(
        &self,
        effects: &[content_reader_parser::parser::Item],
        refused: &mut Vec<String>,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
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
            let describer = self.content.effect_describer.as_deref().ok_or_else(|| {
                format!("{name}: power balance effect describer is unavailable")
            })?;
            let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
            describer.describe(self, name, effects, scopes, &mut graph, refused)
        })();
        scopes.fresh_depth.set(depth);
        result
    }

    fn active_range_description(
        &self,
        slot: u32,
        record: &replay_data::rows::PowerBalanceRow,
        name: &str,
        which: usize,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<String, String> {
        let definitions = self
            .frame
            .complete::<tables::ModifierDefinitions>(DEFINITION_TEXT)
            .map_err(|e| format!("{name}: {e}"))?;
        let tokens = self
            .frame
            .complete::<tables::LexicalTokens>(0)
            .map_err(|e| format!("{name}: {e}"))?;
        let mut registry = std::collections::BTreeMap::new();
        for definition in each(definitions) {
            let definition = definition.map_err(|e| format!("{name}: {e}"))?;
            if definition.token == 0 {
                continue;
            }
            let literal = spelled(&tokens, definition.token)
                .map_err(|e| format!("{name}: {e}"))?
                .ok_or_else(|| {
                    format!(
                        "{name}: modifier {} has no lexical token {}",
                        definition.index, definition.token
                    )
                })?;
            registry.insert(literal.to_ascii_lowercase(), definition);
        }
        let database = content_reader_politics::power_balance::read(&self.content.layers.layers, &|key| {
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
            let mut values: Vec<_> = range
                .modifiers
                .listed()
                .map(|m| {
                    registry
                        .get(m.name.as_bytes())
                        .map(|d| (m, d))
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
                let value =
                    if definition.flags & engine_native_parts::modifier_definition::CLAMP_MAXIMUM != 0 {
                        modifier.value.clamp(0, 100_000)
                    } else {
                        modifier.value
                    };
                let value = value.wrapping_mul(100_000) / 100_000;
                if value == 0 && definition.flags & 0x100 == 0 {
                    continue;
                }
                if definition.feature_selector != 0 {
                    let features = match self.frame.complete::<tables::SimulationFeatures>(DLC) {
                        Ok(rows) if rows.len() == 1 => each(rows).next().transpose()?,
                        _ => None,
                    };
                    let Some(features) = features else {
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
                    "{name}: {} feature-gated modifiers ({}) cannot be decided without this hour's dlc mask of simulation features",
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
            let mut start = 0;
            for (&value, &(_, key, default, fulfilled)) in range
                .rules
                .iter()
                .zip(content_reader_politics::power_balance::rules())
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
                    "{name}: {} unsupported effect descriptions ({}); their renderers are not implemented",
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

    fn balance_trees(
        &self,
        sources: &[Vec<u8>],
    ) -> Result<[Vec<replay_data::rows::ModifierBreakdownNode>; 2], String> {
        let registry = replay_derive::modifier_registry::Registry::from_tables(&self.frame, 0)?;
        if let Some(describer) = &self.content.effect_describer {
            describer.prepare_static_modifiers(self.content)?;
        }
        let statics = self.content.static_modifiers();
        let mut blocks = Vec::with_capacity(sources.len());
        for source in sources {
            let key =
                std::str::from_utf8(content_reader_parser::parser::c_string(source)).map_err(|_| {
                    format!(
                        "static modifier {} is not UTF-8",
                        String::from_utf8_lossy(source)
                    )
                })?;
            blocks.push(statics.get(&self.content.layers.layers, key)?.1);
        }
        let inputs: Vec<_> = sources
            .iter()
            .zip(&blocks)
            .map(
                |(source, block)| replay_derive::power_balance_derive::Source {
                    name: source,
                    auxiliary: !block.tooltip_table().is_empty() || !block.children.is_empty(),
                    values: block
                        .values
                        .iter()
                        .map(|value| (value.name.as_str(), value.value))
                        .collect(),
                },
            )
            .collect();
        Ok([
            replay_derive::power_balance_derive::tree(
                engine_executable::modifier_keys::POWER_BALANCE_DAILY,
                &inputs,
                &registry,
            )?,
            replay_derive::power_balance_derive::tree(
                engine_executable::modifier_keys::POWER_BALANCE_WEEKLY,
                &inputs,
                &registry,
            )?,
        ])
    }

    pub(super) fn balance(
        &self,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<Option<String>, String> {
        let Some(property) = BalanceProperty::parse(name) else {
            return Ok(None);
        };
        let which = property as usize;
        let members = self
            .frame
            .complete::<tables::PowerBalanceMembers>(MEMBER)
            .map_err(|e| format!("{name}: {e}"))?;
        let mut selected = None;
        for row in each(members) {
            let row = row?;
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
            .complete::<tables::PowerBalances>(BALANCE)
            .map_err(|e| format!("{name}: {e}"))?;
        let record = match selected {
            Some(id) => Some(
                keyed(&records, u128::from(id))?
                    .ok_or_else(|| format!("{name}: balance record {id} is missing"))?,
            ),
            None => None,
        };
        if which < 6 {
            let Some(record) = record.as_deref() else {
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
            if which == 5 {
                return self
                    .expand_fresh_country(&text, slot, name, scopes)
                    .map(Some);
            }
            if text.contains('[') && text.contains(']') {
                return self.expand_fresh_empty(&text, name, scopes).map(Some);
            }
            return Ok(Some(text));
        }
        if matches!(which, 9..=12) {
            return match record.as_deref() {
                Some(record) => self
                    .active_range_description(slot, record, name, which, scopes)
                    .map(Some),
                None => Ok(Some(String::new())),
            };
        }
        if which == 8 {
            let Some(record) = record.as_deref() else {
                return Ok(Some(String::new()));
            };
            let sources = self
                .frame
                .complete::<tables::PowerBalanceModifierBreakdowns>(SOURCES)
                .map_err(|e| format!("{name}: {e}"))?;
            let sources = keyed(&sources, u128::from(record.ordinal))?.ok_or_else(|| {
                format!(
                    "{name}: balance record {} has no modifier sources",
                    record.ordinal
                )
            })?;
            let definitions = self
                .frame
                .complete::<tables::ModifierDefinitions>(DEFINITION_VALUE)
                .map_err(|e| format!("{name}: {e}"))?;
            let trees = self
                .balance_trees(&sources.sources)
                .map_err(|e| format!("{name}: {e}"))?;
            let mut output = String::new();
            for (key, nodes) in [
                (
                    engine_executable::modifier_keys::POWER_BALANCE_DAILY,
                    &trees[0],
                ),
                (
                    engine_executable::modifier_keys::POWER_BALANCE_WEEKLY,
                    &trees[1],
                ),
            ] {
                let definition = keyed(&definitions, u128::from(key))?
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
                            &definition,
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
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&text);
            }
            return Ok(Some(output));
        }
        let (mut daily, mut weekly) = (0_i64, 0_i64);
        if let Some(record) = record {
            let low = u128::from(record.ordinal) << 32;
            for m in each(members.span(low, low | u128::from(u32::MAX))) {
                let m = m?;
                let native = engine_power_balance::power_balance_record::MemberRow {
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
        let drift = balance_drift(daily, weekly);
        if which == 7 {
            return Ok(Some(if drift.weekly_change == 0 {
                String::new()
            } else {
                format!(
                    "\u{13}BoP_{}_texticon ",
                    if drift.weekly_change < 0 { "left" } else { "right" }
                )
            }));
        }
        let (key, value) = match drift.shown {
            ChangeRate::Weekly(value) => (b"BOP_CHANGE_RATE_WEEKLY".as_slice(), value),
            ChangeRate::Daily(value) => (b"BOP_CHANGE_RATE_DAILY".as_slice(), value),
        };
        self.localisation
            .localise_with(key, &[("VALUE", Argument::Fixed5(value.wrapping_abs()))])
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeRate {
    Daily(i64),
    Weekly(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BalanceDrift {
    pub weekly_change: i64,
    pub shown: ChangeRate,
}

pub fn balance_drift(daily: i64, weekly: i64) -> BalanceDrift {
    let weekly_change = daily.wrapping_mul(7).wrapping_add(weekly);
    let converted = if !(i64::MIN + 2..=i64::MAX - 1).contains(&weekly) {
        0xffff_ffff
    } else {
        weekly.wrapping_mul(100_000) / 700_000
    };
    let change = daily.wrapping_add(converted);
    BalanceDrift {
        weekly_change,
        shown: if change == 0 { ChangeRate::Weekly(weekly_change) } else { ChangeRate::Daily(change) },
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests;
