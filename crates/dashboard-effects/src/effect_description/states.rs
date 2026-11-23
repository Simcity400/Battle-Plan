use super::*;

mod buildings;
mod names;
mod occupation;
mod provinces;

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(super) fn start_border_war(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let states = self.complete::<tables::States>(0)?;
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
                .scoped_state_id(state.value.scalar().unwrap_or_default(), scopes)
                .map_err(|why| why.to_string())? as u32;
            if !states.contains(u128::from(state)) {
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
        )
    }
}
