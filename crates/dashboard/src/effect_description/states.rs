use super::*;
use replay::rows::tables;

impl Renderer<'_, '_, '_> {
    pub(super) fn state_change(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        // 140378010 / 140398680
        let states = self
            .context
            .text
            .frame()
            .complete::<tables::States>()
            .map_err(|why| why.to_string())?;
        let state = self.state_target(item, scopes)?;
        let label = if states.iter().any(|row| row.id == state) {
            self.state_name(state)?
        } else {
            String::new()
        };
        self.line(key, &[("STATE", Argument::Text(&label))], scopes, true)
    }

    pub(super) fn start_border_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let states = self
            .context
            .text
            .frame()
            .complete::<tables::States>()
            .map_err(|why| why.to_string())?;
        let mut labels = Vec::new();
        for side in ["attacker", "defender"] {
            let Some(side) = field(children(item), side) else {
                return Ok(String::new());
            };
            let Some(state) = field(children(side), "state") else {
                return Ok(String::new());
            };
            let state = self
                .evaluator()?
                .scoped_state_id(state.value.scalar().unwrap_or_default(), scopes)?
                as u32;
            if !states.iter().any(|row| row.id == state) {
                return Ok(String::new());
            }
            labels.push(self.state_name(state)?);
        }
        self.line(
            b"EFFECT_START_BORDER_WAR",
            &[
                ("STATE1", Argument::Text(&labels[0])),
                ("STATE2", Argument::Text(&labels[1])),
            ],
            scopes,
            true,
        ) // 140395AE0
    }
}
