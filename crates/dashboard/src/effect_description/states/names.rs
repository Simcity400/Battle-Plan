use super::*;

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn state_change(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        // 140378010 / 140398680 / 140378300 / 14038B980 / 14038BC70 / 140393560 / 140393850:
        // the invalid target still produces the localised line with STATE empty.
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
            return Err(
                "set_capital requires its native block reader (140534280/1403B4F40)".into(),
            );
        }
        // 1403B4F40 reads state as CScopedVariable@140539440, independently
        // of the effect selector. Constructor 140340860 defaults it to zero.
        let state = match field(children(item), "state") {
            Some(state) => {
                let operand = scalar_operand(state)?;
                self.evaluator()?.scoped_state_id(operand, scopes)? as u32
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
        // 14038FB90 does not read remember_old_capital.
    }

    pub(in crate::effect_description) fn state_name_effect(
        &self,
        item: &Item,
        scopes: Scopes,
        reset: bool,
    ) -> Result<String, String> {
        // 14030A700 stores the name token; 14030A5D0 always clears it.
        let name = if reset {
            &[][..]
        } else {
            installed_content::parser::c_string(item.value.scalar().ok_or_else(|| {
                "set_state_name requires one value (141385570/14030A700)".to_string()
            })?)
        };
        // 1403188E0 localises NAME before substituting it, without expansion.
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
        // 140318680 / 140318BA0 / 1403192B0: LF, without scoped expansion.
        let country = self.country_target(item, scopes)?.unwrap_or(0); // 140BA5960.
        let country = self.country_name(country, false)?;
        self.line(key, &[("COUNTRY", Argument::Text(&country))], scopes, false)
    }
}
