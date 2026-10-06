use super::*;

impl Renderer<'_, '_, '_> {
    pub(super) fn save_event_target(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        // 14038D670 -> 14052D4F0.
        self.evaluator()?
            .save_tooltip_target(item.value.read_string().unwrap_or_default(), scopes)?;
        Ok(String::new())
    }
}

// CClearVariableEffect describes nothing and does not execute (140176EE0).
pub(super) fn clear_variable() -> Result<String, String> {
    Ok(String::new())
}
