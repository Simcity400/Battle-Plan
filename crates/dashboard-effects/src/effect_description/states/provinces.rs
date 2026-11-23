use super::*;
use crate::triggers::scope_selector;
use content_reader_parser::crt::{Scanned, scan_i32};
use replay_data::rows::TableId;

impl Renderer<'_, '_, '_> {
    fn province_integer(&self, item: &Item, scopes: Scopes) -> Result<i32, String> {
        if scope_selector(item).is_some() {
            return Ok(0);
        }
        let operand = scalar_operand(item)?;
        Ok((self
            .evaluator()?
            .required(operand, item.quoted(), false, scopes)
            .map_err(|why| why.to_string())?
            / 100_000) as i32)
    }

    fn goto_state_id(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        let id = self.province_integer(item, scopes)?;
        if id != 0 {
            return Ok(id as u32);
        }
        if scope_selector(item).is_some() {
            self.state_target(item, scopes)
        } else {
            Ok(0)
        }
    }

    fn description_province(
        &self,
        id: u32,
    ) -> Result<Option<replay_frame::frame::Province>, String> {
        const PROVINCE: u64 = mask::<replay_data::rows::ProvinceRow>(&[
            "state",
            "owner",
            "controller",
            "victory_points",
            "controller_area",
        ]);
        Ok(keyed(&self.complete::<tables::Provinces>(PROVINCE)?, u128::from(id))?
            .map(|row| replay_frame::frame::Province::from(&*row)))
    }

    pub(in crate::effect_description) fn go_to_state(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = self.goto_state_id(item, scopes)?;
        if self.valid_state(id)?.is_none() {
            return Ok(String::new());
        }
        let label = self.state_name(id)?;
        self.line(
            b"EFFECT_GO_TO_PROVINCE",
            &[("STATE", Argument::Text(&label))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn province_state_description(
        &self,
        item: &Item,
        scopes: Scopes,
        goto: bool,
    ) -> Result<String, String> {
        let id = self.province_integer(item, scopes)? as u32;
        let Some(province) = self.description_province(id)? else {
            return if goto {
                Err("goto_province has an invalid province; the description requires an existing target".into())
            } else {
                Ok(String::new())
            };
        };
        if province.state == 0 {
            return Err("province has no state; the description requires its state".into());
        }
        let label = self.state_name(province.state)?;
        let args = [("STATE", Argument::Text(&label))];
        if goto {
            self.line(b"EFFECT_GO_TO_PROVINCE", &args, scopes, true)
        } else {
            self.localised(b"EFFECT_SET_PROVINCE_CONTROLLER", &args, scopes, true)
        }
    }

    pub(in crate::effect_description) fn province_victory_points(
        &self,
        item: &Item,
        scopes: Scopes,
        add: bool,
    ) -> Result<String, String> {
        let id = match field(children(item), "province") {
            Some(operand) => {
                let value = scalar_operand(operand)?;
                crate::triggers::fixed_round(self.scoped_value(value, scopes)?) as u32
            }
            None => 0,
        };
        let Some(province) = self.description_province(id)? else {
            return Ok(String::new());
        };
        let mut label = self.context.text.province_display_name(id)?;
        if label.is_empty() {
            let frame = self.tables();
            let raw = match frame.status(TableId::ProvinceNames) {
                replay_frame::frame::OperandStatus::Unavailable(reason) => {
                    return Err(format!(
                        "victory-point raw-name fallback: province names unavailable: {reason}"
                    ));
                }
                replay_frame::frame::OperandStatus::NotRecorded(mode) => {
                    return Err(format!(
                        "victory-point raw-name fallback: province names is not recorded in a {mode} recording"
                    ));
                }
                _ => frame
                    .held::<tables::ProvinceNames>(0)
                    .text(
                        u128::from(id),
                        |row| &row.name,
                        replay_data::rows::column::<replay_data::rows::ProvinceNameRow>("name"),
                    )
                    .map_err(built)?,
            };
            if let Some(name) = raw {
                if name.contains(&0) {
                    return Err(
                        "victory-point raw-name fallback with embedded NUL is not reproduced"
                            .into(),
                    );
                }
                label = std::str::from_utf8(name)
                    .map_err(|_| "victory-point raw-name fallback is not UTF-8")?
                    .to_owned();
            }
            if label.is_empty() && province.state != 0 {
                label = self.state_name(province.state)?;
            }
            if label.is_empty() {
                return Ok(String::new());
            }
        }
        let value = match field(children(item), "value") {
            Some(operand) => {
                crate::triggers::fixed_round(self.scoped_value(scalar_operand(operand)?, scopes)?)
                    as i32
            }
            None => 0,
        };
        let key = if !add {
            b"EFFECT_SET_VP".as_slice()
        } else if value > 0 {
            b"EFFECT_ADD_VP".as_slice()
        } else {
            b"EFFECT_SUBTRACT_VP".as_slice()
        };
        self.localised(
            key,
            &[
                ("PROVINCE", Argument::Text(&label)),
                (
                    "VALUE",
                    Argument::Int32(if add && value <= 0 {
                        value.wrapping_neg()
                    } else {
                        value
                    }),
                ),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn rename_province(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut id = 0;
        for operand in children(item)
            .iter()
            .filter(|operand| operand.keyword_is("id"))
        {
            if let Some(operand) = operand.value.scalar()
                && let Scanned::Stored(value) = scan_i32(operand)
            {
                id = value;
            }
        }
        if id == 0
            || self
                .description_province(id as u32)?
                .is_none_or(|province| province.victory_points < 1)
        {
            return Ok("ERROR: Invalid location or missing victory point for rename".into());
        }
        let name = field(children(item), "name")
            .map(scalar_operand)
            .transpose()?
            .unwrap_or_default();
        let name = self.localised(name, &[], scopes, true)?;
        let old_name = self.context.text.province_display_name(id as u32)?;
        self.line(
            b"EFFECT_SET_PROVINCE_NAME",
            &[
                ("OLDNAME", Argument::Text(&old_name)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn reset_province_name(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = self.province_integer(item, scopes)? as u32;
        if self.description_province(id)?.is_none() {
            return Ok(String::new());
        }
        let name = self.context.text.province_display_name(id)?;
        self.localised(
            b"EFFECT_RESET_PROVINCE_NAME",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }
}
