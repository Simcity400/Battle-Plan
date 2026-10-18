use super::*;
use crate::triggers::scope_selector;
use installed_content_core::fixed5::fixed5_percent;
use replay_data::rows::ModifierDefinitionRow;

impl Renderer<'_, '_, '_> {
    fn occupation_operand(&self, item: &Item, scopes: Scopes) -> Result<i64, String> {
        // Selector flags do not change generic description's scope.
        // Its numeric own text reads the unchanged literal-zero operand.
        if scope_selector(item).is_some() {
            Ok(0)
        } else {
            self.scoped_value(scalar_operand(item)?, scopes)
        }
    }

    fn occupation_state(&self, scopes: Scopes) -> Result<Option<u32>, String> {
        let Scope::State(state) = scopes.this else {
            return Ok(None);
        };
        Ok(self.valid_state(state)?.map(|row| row.id))
    }

    fn occupation_value(
        &self,
        value: i64,
        positive_good: bool,
        scopes: Scopes,
    ) -> Result<String, String> {
        // The zero branch keeps '+' and yellow controls.
        if value == 0 {
            return Ok("\u{11}Y+0.0%\u{11}!".to_owned());
        }
        crate::modifier_text::value_text(
            &ModifierDefinitionRow {
                flags: crate::modifier_text::PERCENT
                    | if positive_good {
                        crate::modifier_text::POSITIVE_IS_GOOD
                    } else {
                        0
                    },
                precision: 1,
                ..Default::default()
            },
            value,
            1,
            true,
            false,
            |key, _| self.localised(key, &[], scopes, false),
        )
    }

    pub(in crate::effect_description) fn occupation_percent(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
        positive_good: Option<bool>,
    ) -> Result<String, String> {
        // no scope expansion.
        let value = self.occupation_operand(item, scopes)?;
        let value = match positive_good {
            Some(good) => self.occupation_value(value, good, scopes)?,
            None => fixed5_percent(value, 1, false),
        };
        self.line(key, &[("VALUE", Argument::Text(&value))], scopes, false)
    }

    pub(in crate::effect_description) fn demilitarized_zone(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // An invalid current state gives empty text.
        if self.occupation_state(scopes)?.is_none() {
            return Ok(String::new());
        }
        let enabled = scope_selector(item).is_none()
            && installed_content_core::parser::read_bool(scalar_operand(item)?).unwrap_or(false);
        self.line(
            if enabled {
                b"EFFECT_DEMILITARIZED_ZONE"
            } else {
                b"EFFECT_NOT_DEMILITARIZED_ZONE"
            },
            &[],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn extra_shared_slots(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // rounded zero takes REMOVE; fixed-point rounding.
        let value = crate::triggers::fixed_round(self.occupation_operand(item, scopes)?) as i32;
        self.line(
            if value > 0 {
                b"EFFECT_ADD_EXTRA_STATE_SHARED_BUILDING_SLOTS"
            } else {
                b"EFFECT_REMOVE_EXTRA_STATE_SHARED_BUILDING_SLOTS"
            },
            &[(
                "COUNT",
                Argument::Int32(if value > 0 {
                    value
                } else {
                    value.wrapping_neg()
                }),
            )],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn cancel_resistance(
        &self,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.localised(b"EFFECT_CANCEL_RESISTANCE", &[], scopes, false) // no LF.
    }

    pub(in crate::effect_description) fn force_enable_resistance(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // operands name no countries in the own text; no LF.
        let Some(state) = self.occupation_state(scopes)? else {
            return Ok(String::new());
        };
        let clear = installed_content_core::parser::read_bool_field(children(item), "clear", false);
        let state = self.state_name(state)?;
        self.localised(
            if clear {
                b"EFFECT_FORCE_ADD_RESISTANCE_CLEAR"
            } else {
                b"EFFECT_FORCE_ADD_RESISTANCE"
            },
            &[("STATE", Argument::Text(&state))],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn set_state_category(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let category = installed_content_core::parser::c_string(scalar_operand(item)?);
        let database = self
            .context
            .content
            .state_categories
            .ok_or_else(|| self.content_gap("state categories"))?;
        // Reject the undefined database null object.
        if database.find(category)?.is_none() {
            return Err(format!(
                "invalid state category {}",
                String::from_utf8_lossy(category)
            ));
        }
        // localises the operand's spelling; no expansion, then LF.
        let category = self.localised(category, &[], scopes, false)?;
        self.line(
            b"EFFECT_SET_STATE_CATEGORY",
            &[("CATEGORY", Argument::Text(&category))],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn start_resistance(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // country operand; no LF.
        let target = self.country_target(item, scopes)?;
        if let Some(target) = target.filter(|target| *target > 0) {
            let country = self.country_name(target, false)?;
            self.localised(
                b"EFFECT_INIT_RESISTANCE_BY_COUNTRY",
                &[("COUNTRY", Argument::Text(&country))],
                scopes,
                false,
            )
        } else {
            self.localised(b"EFFECT_INIT_RESISTANCE", &[], scopes, false)
        }
    }

    pub(in crate::effect_description) fn supply_units(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let value = self.occupation_operand(item, scopes)?;
        self.localised(
            b"EFFECT_SUPPLY_UNITS",
            &[("NUM", Argument::Fixed5(value))],
            scopes,
            false,
        ) // no LF.
    }

    pub(in crate::effect_description) fn resistance_target(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // defaults amount=-1; scalar writes amount.
        let fields = children(item);
        // effect parsing reuses operands whose reader retains prior flags.
        for name in ["amount", "days", "occupied", "occupier"] {
            if fields
                .iter()
                .filter(|field| field.keyword_is(name))
                .nth(1)
                .is_some()
            {
                return Err(format!(
                    "repeated resistance-target operand {name} needs retained scoped variable reader flags",
                ));
            }
        }
        let amount = match item.value.scalar() {
            Some(amount) => Some(amount),
            None => field(fields, "amount").map(scalar_operand).transpose()?,
        };
        let amount = match amount {
            Some(amount) => self.scoped_value(amount, scopes)?,
            None => -100_000,
        };
        if amount == 0 {
            return Ok(String::new());
        }
        // evaluates days and both countries only after nonzero amount.
        let days = field(fields, "days")
            .map(|item| self.scoped_value(scalar_operand(item)?, scopes))
            .transpose()?
            .unwrap_or(0);
        let days = crate::triggers::fixed_round(days) as i32;
        let country = |name| -> Result<u32, String> {
            field(fields, name)
                .map(|item| {
                    self.evaluator()?
                        .scoped_country(scalar_operand(item)?, scopes)
                })
                .transpose()
                .map(|country| country.flatten().unwrap_or(0))
        };
        let occupied = country("occupied")?;
        let occupier = country("occupier")?;
        let key: &[u8] = match ((occupied as i32) > 0, (occupier as i32) > 0) {
            (false, false) => b"EFFECT_ADD_RESISTANCE_TARGET",
            (true, false) => b"EFFECT_ADD_RESISTANCE_TARGET_WITH_FROM",
            (false, true) => b"EFFECT_ADD_RESISTANCE_TARGET_WITH_TARGET",
            (true, true) => b"EFFECT_ADD_RESISTANCE_TARGET_WITH_FROM_AND_TARGET",
        };
        // uses wrapping fixed5 multiplication even at scale 1;
        // positive is bad, precision 1, '%' suffix, no ratio scaling.
        let amount = amount.wrapping_mul(100_000) / 100_000;
        let value = if amount == 0 {
            "\u{11}Y+0.0%\u{11}!".to_owned() // Zero also gets '+'.
        } else {
            self.occupation_value(amount, false, scopes)?
        };
        // constructs both marked-country arguments in every branch.
        let occupied = self.context.text.country_marked_name(occupied)?;
        let occupier = self.context.text.country_marked_name(occupier)?;
        let mut text = self.localised(
            key,
            &[
                ("VALUE", Argument::Text(&value)),
                ("OCCUPIER", Argument::Text(&occupier)),
                ("OCCUPIED", Argument::Text(&occupied)),
                ("NUM_DAYS", Argument::Int32(days)),
            ],
            scopes,
            false,
        )?;
        if days > 0 {
            text.push_str(" (");
            text.push_str(&self.localised(
                b"DURATION_DAYS",
                &[("NUM", Argument::Int32(days))],
                scopes,
                false,
            )?);
            text.push(')');
        }
        text.push('\n'); // No expansion.
        Ok(text)
    }
}
