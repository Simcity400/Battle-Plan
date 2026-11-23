use std::collections::BTreeSet;

use content_reader_parser::{parser::Item, tokens::Token};

use super::{
    Eval, Failure, Scope, Scopes,
    eval::{Evaluator, key_text},
    operands::{compare, trigger_boolean},
};
use crate::state_resources::ONE;

impl Evaluator<'_, '_> {
    pub(super) fn has_scientist_level(
        &self,
        item: &Item,
        character: u32,
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let fields = item
            .value
            .items()
            .ok_or("has_scientist_level takes a block of level and specialization")?;
        let (mut specialization, mut level) = (None, None);
        for field in fields {
            if field.keyword_is("specialization") {
                specialization = Some(field);
            } else if field.keyword_is("level") {
                level = Some(field);
            } else {
                return Err(format!(
                    "has_scientist_level field {} is an unexpected key, whose load report is not reproduced",
                    key_text(field)
                )
                .into());
            }
        }
        let specialization = specialization
            .ok_or("has_scientist_level names no specialization; the unset specialization is not reproduced")?;
        let level = level.ok_or(
            "has_scientist_level names no level; the unset comparison is not reproduced",
        )?;
        let name = specialization
            .value
            .scalar()
            .map(content_reader_parser::parser::c_string)
            .ok_or("has_scientist_level specialization is a block, not a token")?;
        let catalog = self.world().special_projects.map_err(Failure::from)?;
        if catalog.specialization(&Token::c_str(name))?.is_none() || character == 0 {
            return Ok(Eval::False);
        }
        let token = self.world().token_of(name)?.ok_or_else(|| -> Failure {
            format!(
                "specialization {} is absent from lexical tokens",
                String::from_utf8_lossy(name)
            )
            .into()
        })?;
        let Some(actual) = self.world().scientist_level(character, token)? else {
            return Ok(Eval::False);
        };
        let operand = level
            .value
            .scalar()
            .ok_or("has_scientist_level level is a block, not a value")?;
        let required = self.required(operand, level.quoted(), false, scopes)?;
        Ok(Eval::from_bool(compare(
            i64::from(actual) * ONE,
            required,
            level.operator,
            false,
        )))
    }

    pub(super) fn is_active_scientist(&self, item: &Item, character: u32) -> Result<Eval, Failure> {
        let expected = trigger_boolean(item)?;
        if character == 0 {
            return Ok(Eval::False);
        }
        Ok(Eval::from_bool(
            self.world()
                .scientist_active(character)?
                .is_some_and(|active| active == u8::from(expected)),
        ))
    }

    pub(super) fn any_scientist(
        &self,
        block: &[Item],
        country: u32,
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let mut count: i64 = 1;
        let mut children = Vec::new();
        for item in block {
            if item.keyword_is("count") {
                count = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)? / ONE;
            } else if !item.keyword_is("tooltip") {
                children.push(item.clone());
            }
        }
        let (mut passed, mut open) = (0_i64, 0_i64);
        let mut reasons = BTreeSet::new();
        for character in self.world().country_scientists(country)? {
            match self.evaluate(&children, self.with_this(scopes, Scope::Character(character))?) {
                Eval::True => passed += 1,
                Eval::False => {}
                Eval::Unknown(more) => {
                    open += 1;
                    reasons.extend(more);
                }
            }
            if count - passed < 1 {
                return Ok(Eval::True);
            }
        }
        Ok(if open > 0 && count - passed - open < 1 {
            Eval::Unknown(reasons)
        } else {
            Eval::False
        })
    }
}
