use super::*;

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn state_change(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let label = self.state_label(self.state_target(item, scopes)?)?;
        self.line(key, &[("STATE", Argument::Text(&label))], scopes, true)
    }

    fn state_label(&self, state: u32) -> Result<String, String> {
        if self.valid_state(state)?.is_some() {
            self.state_name(state)
        } else {
            Ok(String::new())
        }
    }

    pub(in crate::effect_description) fn set_capital(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        if item.value.scalar().is_some() {
            return Err("set_capital requires its native block reader".into());
        }
        let state = match field(children(item), "state") {
            Some(state) => {
                let operand = scalar_operand(state)?;
                self.evaluator()?
                    .scoped_state_id(operand, scopes)
                    .map_err(|why| why.to_string())? as u32
            }
            None => 0,
        };
        let label = self.state_label(state)?;
        self.line(
            b"EFFECT_SET_CAPITAL",
            &[("STATE", Argument::Text(&label))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn state_name_effect(
        &self,
        item: &Item,
        scopes: Scopes,
        reset: bool,
    ) -> Result<String, String> {
        let name = if reset {
            &[][..]
        } else {
            content_reader_parser::parser::c_string(
                item.value
                    .scalar()
                    .ok_or_else(|| "set_state_name requires one value".to_string())?,
            )
        };
        if name.is_empty() {
            self.line(b"EFFECT_RESET_STATE_NAME", &[], scopes, false)
        } else {
            let label = self.localised(name, &[], scopes, false)?;
            self.line(
                b"EFFECT_SET_STATE_NAME",
                &[("NAME", Argument::Text(&label))],
                scopes,
                false,
            )
        }
    }

    pub(in crate::effect_description) fn state_country_to(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let country = self.country_target(item, scopes)?.unwrap_or(0);
        let country = self.country_name(country, false)?;
        self.line(key, &[("COUNTRY", Argument::Text(&country))], scopes, false)
    }
}
