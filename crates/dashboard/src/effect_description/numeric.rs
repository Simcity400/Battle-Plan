use super::*;

impl Renderer<'_, '_, '_> {
    pub(super) fn numeric_value(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let value = self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?;
        self.line(key, &[("VAL", Argument::Fixed5(value))], scopes, true)
    }
}
