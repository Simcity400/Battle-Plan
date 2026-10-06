use super::*;
use crate::triggers::scope_selector;
use installed_content::crt::{Scanned, scan_i32};
use replay::{frame::TableView, rows::TableId};

impl Renderer<'_, '_, '_> {
    // CIntEffect vf6/vf26, 14137DD70/141384870: selectors leave the integer
    // zero; numeric tokens scan %i, scoped values truncate toward zero.
    fn province_integer(&self, item: &Item, scopes: Scopes) -> Result<i32, String> {
        if scope_selector(item).is_some() {
            return Ok(0);
        }
        let operand = scalar_operand(item)?;
        Ok((self
            .evaluator()?
            .required(operand, item.quoted(), false, scopes)?
            / 100_000) as i32)
    }

    fn goto_state_id(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        let id = self.province_integer(item, scopes)?;
        if id != 0 {
            return Ok(id as u32);
        }
        // 1403A0CC0 -> vf16@140533BE0/14052FA10, with default state zero.
        if scope_selector(item).is_some() {
            self.state_target(item, scopes)
        } else {
            Ok(0)
        }
    }

    fn description_province(&self, id: u32) -> Result<Option<&replay::rows::ProvinceRow>, String> {
        Ok(self
            .context
            .text
            .frame()
            .complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.id == id))
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
                Err("goto_province has an invalid province; native 1403821C0 dereferences the missing target".into())
            } else {
                Ok(String::new()) // 140392F90 checks the province pointer.
            };
        };
        if province.state == 0 {
            return Err(
                "province has no state; native 1403821C0/140392F90 dereferences CProvince+0xC0"
                    .into(),
            );
        }
        let label = self.state_name(province.state)?;
        let args = [("STATE", Argument::Text(&label))];
        if goto {
            self.line(b"EFFECT_GO_TO_PROVINCE", &args, scopes, true) // 1403821C0.
        } else {
            self.localised(b"EFFECT_SET_PROVINCE_CONTROLLER", &args, scopes, true) // 140392F90: no LF.
        }
    }

    pub(in crate::effect_description) fn province_victory_points(
        &self,
        item: &Item,
        scopes: Scopes,
        add: bool,
    ) -> Result<String, String> {
        // 1403AE5F0 writes +0x58; 14039E9C0 evaluates and rounds that ID.
        let id = match field(children(item), "province") {
            Some(operand) => {
                let value = scalar_operand(operand)?;
                crate::triggers::fixed_round(self.scoped_value(value, scopes)?) as u32
            }
            None => 0,
        };
        let Some(province) = self.description_province(id)? else {
            return Ok(String::new()); // 14036E9A0/14036EBA0.
        };
        let mut label = self.context.text.province_display_name(id)?;
        if label.is_empty() {
            // Both VP descriptions copy the raw override after an empty display.
            // province_display_name already checked this sparse table's coverage.
            let raw = match &*self.context.text.frame().tables[TableId::ProvinceNames.index()] {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows
                    .rows::<tables::ProvinceNames>()
                    .and_then(|rows| rows.iter().find(|row| row.province == id)),
                TableView::Unavailable { reason } => {
                    return Err(format!(
                        "victory-point raw-name fallback: ProvinceNames unavailable: {reason}"
                    ));
                }
            };
            if let Some(row) = raw {
                if row.name.contains(&0) {
                    return Err("victory-point raw-name fallback with embedded NUL is not reproduced (CProvince+0x40; 14036E9A0/14036EBA0)".into());
                }
                label = std::str::from_utf8(&row.name)
                    .map_err(|_| "victory-point raw-name fallback is not UTF-8 (CProvince+0x40; 14036E9A0/14036EBA0)")?
                    .to_owned();
            }
            if label.is_empty() && province.state != 0 {
                label = self.state_name(province.state)?;
            }
            if label.is_empty() {
                return Ok(String::new());
            }
        }
        // Native reads the amount only after a usable name. Constructor
        // 14014AAC0 gives the missing field a literal zero; fixed_round1424D8760.
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
        ) // 1403273F0: expansion, no LF.
    }

    pub(in crate::effect_description) fn rename_province(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 1403B4C80 -> 1424AB970/1424B02C0: a failed %i scan keeps the
        // previous ID, including a repeated field; factory 140340170 starts at 0.
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
            // Native literal at 14275D618, returned without localisation/LF.
            return Ok("ERROR: Invalid location or missing victory point for rename".into());
        }
        let name = field(children(item), "name")
            .map(scalar_operand)
            .transpose()?
            .unwrap_or_default();
        let name = self.localised(name, &[], scopes, true)?; // NAME first at 14038C716.
        let old_name = self.context.text.province_display_name(id as u32)?; // 14038C728.
        self.line(
            b"EFFECT_SET_PROVINCE_NAME",
            &[
                ("OLDNAME", Argument::Text(&old_name)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            true,
        ) // 14038C5B0: one LF.
    }

    pub(in crate::effect_description) fn reset_province_name(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let id = self.province_integer(item, scopes)? as u32;
        if self.description_province(id)?.is_none() {
            return Ok(String::new()); // 14038CAF0.
        }
        let name = self.context.text.province_display_name(id)?;
        self.localised(
            b"EFFECT_RESET_PROVINCE_NAME",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        ) // 14038CAF0: no LF.
    }
}
