use super::*;
use replay_data::rows::{PowerBalanceMemberRow, PowerBalanceRangeRow, PowerBalanceRow};

const BALANCE: u64 = tables::PowerBalances::ID.full_mask();
const BALANCE_ID: u64 = mask::<PowerBalanceRow>(&["balance"]);
const MEMBER: u64 = mask::<PowerBalanceMemberRow>(&["country"]);
const RANGE: u64 = mask::<PowerBalanceRangeRow>(&["identity"]);

#[cfg(test)]
mod tests;

fn text(fields: &[Item], name: &str) -> Vec<u8> {
    field(fields, name)
        .map(|item| {
            content_reader_parser::parser::c_string(item.value.scalar().unwrap_or_default()).to_vec()
        })
        .unwrap_or_default()
}

fn same(left: &[u8], right: &[u8]) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn absolute(value: i64) -> i64 {
    value.wrapping_abs()
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    fn balance_database(&self) -> Result<&content_reader_politics::power_balance::Database, String> {
        self.context
            .content
            .power_balances
            .ok_or_else(|| self.content_gap("power balances"))
    }

    fn balance_definition(
        &self,
        id: &[u8],
    ) -> Result<&content_reader_politics::power_balance::Definition, String> {
        self.balance_database()?.definition(id).ok_or_else(|| {
            format!(
                "balance {} is absent from content",
                String::from_utf8_lossy(id)
            )
        })
    }

    fn balance_current(
        &self,
        scopes: Scopes,
    ) -> Result<Option<Cow<'f, PowerBalanceRow>>, String> {
        let country = self.current_country(scopes)?;
        let members = self.complete::<tables::PowerBalanceMembers>(MEMBER)?;
        let mut record = None;
        for member in each(members) {
            let member = member?;
            let slot = self
                .context
                .text
                .recorded()
                .slot(member.country)
                .ok_or_else(|| format!("country links lacks balance member {}", member.country))?;
            if slot == country {
                record = Some(record.map_or(member.record, |old: u32| old.min(member.record)));
            }
        }
        let rows = self.complete::<tables::PowerBalances>(BALANCE)?;
        match record {
            Some(record) => keyed(&rows, u128::from(record))?
                .map(Some)
                .ok_or_else(|| format!("power balances lacks member record {record}")),
            None => Ok(None),
        }
    }

    fn balance_by_id(&self, id: &[u8]) -> Result<Option<Cow<'f, PowerBalanceRow>>, String> {
        let rows = self.complete::<tables::PowerBalances>(BALANCE)?;
        match find(rows.with_mask(BALANCE_ID), |row| same(&row.balance, id))? {
            Some(row) => keyed(&rows, row.key()),
            None => Ok(None),
        }
    }

    fn balance_name(&self, key: &[u8]) -> Result<String, String> {
        if key.is_empty() {
            return Ok(String::new());
        }
        self.localised(key, &[], Scopes::empty(), true)
    }

    fn balance_side(
        &self,
        definition: &content_reader_politics::power_balance::Definition,
        id: &[u8],
    ) -> Result<String, String> {
        match definition.sides.iter().find(|side| same(side, id)) {
            Some(side) => self.balance_name(side),
            None => Ok(String::new()),
        }
    }

    pub(in crate::effect_description) fn balance_modifier(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
    ) -> Result<String, String> {
        let id = text(children(item), "id");
        let definition = self.balance_definition(&id)?;
        let token = text_field(children(item), "modifier")?;
        let (name, modifiers) = self.static_modifier(token)?;
        let name = name
            .map(|name| self.localised(name.as_bytes(), &[], scopes, false))
            .transpose()?
            .unwrap_or_default();
        let balance = self.balance_name(&definition.id)?;
        let description = self.modifier_description_with(
            &modifiers,
            "",
            ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            },
        )?;
        self.char_native_text(
            if remove {
                b"EFFECT_REMOVE_POWER_BALANCE_MODIFIER"
            } else {
                b"EFFECT_ADD_POWER_BALANCE_MODIFIER"
            },
            &[
                ("MODIFIER", Argument::Text(&name)),
                ("POWER_BALANCE", Argument::Text(&balance)),
                ("DESC", Argument::Text(&description)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn balance_remove(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = text(children(item), "id");
        let Some(current) = self.balance_current(scopes)? else {
            return Ok(String::new());
        };
        if !same(&id, &current.balance) {
            return Ok(String::new());
        }
        let balance = self.balance_name(&self.balance_definition(&current.balance)?.id)?;
        self.char_native_line(
            b"EFFECT_REMOVE_POWER_BALANCE",
            &[("POWER_BALANCE", Argument::Text(&balance))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn balance_set(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let definition = self.balance_definition(&text(fields, "id"))?;
        let current = self.balance_current(scopes)?;
        let current = current.as_deref();
        let existing = self.balance_by_id(&definition.id)?;
        let existing = existing.as_deref();
        let chosen = |name, default: &[u8], recorded: Option<&Vec<u8>>| {
            let explicit = text(fields, name);
            if !explicit.is_empty() {
                explicit
            } else {
                recorded.map_or_else(|| default.to_vec(), Clone::clone)
            }
        };
        let left = chosen("left_side", &definition.left, existing.map(|row| &row.left));
        let right = chosen(
            "right_side",
            &definition.right,
            existing.map(|row| &row.right),
        );
        let changed = current.is_none_or(|row| !same(&row.balance, &definition.id));
        let explicit = field(fields, "set_value");
        let defaults = field(fields, "set_default")
            .and_then(|item| item.value.scalar())
            .and_then(content_reader_parser::parser::read_bool)
            .unwrap_or(false);
        let value = explicit
            .map(content_reader_script::modifiers::stored)
            .unwrap_or(if defaults {
                definition.initial_value
            } else {
                0
            });
        let trend = self.balance_side(definition, if value < 0 { &left } else { &right })?;
        let balance = self.balance_name(&definition.id)?;
        let mut out = String::new();
        if changed {
            if let Some(current) = current {
                let old = self.balance_name(&self.balance_definition(&current.balance)?.id)?;
                out.push_str(&self.char_native_line(
                    b"EFFECT_REMOVE_POWER_BALANCE",
                    &[("POWER_BALANCE", Argument::Text(&old))],
                    scopes,
                    true,
                )?);
            }
            let left = self.balance_side(definition, &left)?;
            let right = self.balance_side(definition, &right)?;
            out.push_str(&self.char_native_line(
                b"EFFECT_ADD_POWER_BALANCE",
                &[
                    ("POWER_BALANCE", Argument::Text(&balance)),
                    ("SIDE1", Argument::Text(&left)),
                    ("SIDE2", Argument::Text(&right)),
                    ("VALUE", Argument::Fixed5(absolute(value))),
                    ("TRENDING_SIDE", Argument::Text(&trend)),
                ],
                scopes,
                true,
            )?);
            return Ok(out);
        }
        let current = current.expect("unchanged definition has a record");
        for (old, new) in [(&current.left, &left), (&current.right, &right)] {
            if same(old, new) {
                continue;
            }
            if old.is_empty() && definition.sides.iter().any(Vec::is_empty) {
                return Err(
                    "power balances lacks the valid/null identity of an unnamed current side".into(),
                );
            }
            if !old.is_empty() {
                let old = self.balance_side(definition, old)?;
                out.push_str(&self.char_native_line(
                    b"EFFECT_REMOVE_POWER_BALANCE_SIDE",
                    &[
                        ("SIDE", Argument::Text(&old)),
                        ("POWER_BALANCE", Argument::Text(&balance)),
                    ],
                    scopes,
                    true,
                )?);
            }
            let new = self.balance_side(definition, new)?;
            out.push_str(&self.char_native_line(
                b"EFFECT_ADD_POWER_BALANCE_SIDE",
                &[
                    ("SIDE", Argument::Text(&new)),
                    ("POWER_BALANCE", Argument::Text(&balance)),
                ],
                scopes,
                true,
            )?);
        }
        if explicit.is_some() || defaults {
            out.push_str(&self.char_native_line(
                b"EFFECT_SET_POWER_BALANCE_VALUE",
                &[
                    ("POWER_BALANCE", Argument::Text(&balance)),
                    ("VALUE", Argument::Fixed5(absolute(value))),
                    ("TRENDING_SIDE", Argument::Text(&trend)),
                ],
                scopes,
                true,
            )?);
        }
        Ok(out)
    }

    pub(in crate::effect_description) fn balance_value(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let definition = self.balance_definition(&text(fields, "id"))?;
        let current = self.balance_by_id(&definition.id)?;
        let current = current.as_deref();
        let left = current.map_or(definition.left.as_slice(), |row| row.left.as_slice());
        let right = current.map_or(definition.right.as_slice(), |row| row.right.as_slice());
        let value = field(fields, "value").map_or(0, content_reader_script::modifiers::stored);
        let predicted = current
            .map_or(0, |row| row.value_raw)
            .wrapping_add(value)
            .clamp(-100_000, 100_000);
        let explicit = text(fields, "tooltip_side");
        let side = self.balance_side(
            definition,
            if !explicit.is_empty() {
                &explicit
            } else if value < 0 {
                left
            } else {
                right
            },
        )?;
        let predicted_side =
            self.balance_side(definition, if predicted < 0 { left } else { right })?;
        let trend = if value == 0 {
            String::new()
        } else {
            format!(
                "\u{13}BoP_{}_texticon ",
                if value < 0 { "left" } else { "right" }
            )
        };
        let balance = self.balance_name(&definition.id)?;
        let mut out = self.char_native_text(
            b"EFFECT_ADD_POWER_BALANCE_VALUE",
            &[
                ("POWER_BALANCE", Argument::Text(&balance)),
                ("TREND", Argument::Text(&trend)),
                ("PERCENT", Argument::Fixed5(absolute(value))),
                ("SIDE", Argument::Text(&side)),
                ("PREDICTED_VALUE", Argument::Fixed5(absolute(predicted))),
                ("PREDICTED_SIDE", Argument::Text(&predicted_side)),
            ],
            scopes,
            true,
        )?;
        let Some(current) = current else {
            return Ok(out);
        };
        let rows = self.complete::<tables::PowerBalanceRanges>(RANGE)?;
        let mut selected = None;
        let low = u128::from(current.ordinal) << 32;
        for row in each(rows.span(low, low | u128::from(u32::MAX))) {
            let row = row?;
            let Some(range) = self
                .balance_database()?
                .range(&definition.id, &row.identity)?
            else {
                return Err(
                    "power balance ranges has an unnamed runtime range; range identity is unrecorded"
                        .into(),
                );
            };
            if range.min < predicted && predicted < range.max {
                selected = Some(range);
            }
        }
        let Some(selected) = selected else {
            return Ok(out);
        };
        if same(&selected.id, &current.active_range) {
            return Ok(out);
        }
        let old = self
            .balance_database()?
            .range(&definition.id, &current.active_range)?;
        let range = self.localised(&selected.id, &[], scopes, true)?;
        while !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&self.char_native_text(
            b"EFFECT_POWER_BALANCE_VALUE_MOVED",
            &[
                ("POWER_BALANCE", Argument::Text(&balance)),
                ("TREND", Argument::Text(&trend)),
                ("RANGE", Argument::Text(&range)),
            ],
            scopes,
            true,
        )?);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let mut difference = Modifiers::default();
        for (source, scale) in old
            .into_iter()
            .map(|range| (&range.modifiers, -100_000))
            .chain(std::iter::once((&selected.modifiers, 100_000)))
        {
            if let Some(why) = source.unheld.cause() {
                return Err(why.to_owned());
            }
            for modifier in &source.values {
                let value = modifier.value.wrapping_mul(scale) / 100_000;
                if let Some(existing) = difference
                    .values
                    .iter_mut()
                    .find(|existing| existing.name == modifier.name)
                {
                    existing.value = existing.value.wrapping_add(value);
                } else {
                    let mut modifier = modifier.clone();
                    modifier.value = value;
                    difference.values.push(modifier);
                }
            }
        }
        out.push_str(&self.modifier_description_with(
            &difference,
            "",
            ModifierListOptions {
                category_mask: u32::MAX,
                force_plus: true,
                ..Default::default()
            },
        )?);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        Ok(out)
    }
}
