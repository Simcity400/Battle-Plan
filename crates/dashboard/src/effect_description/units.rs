use super::*;

impl Renderer<'_, '_, '_> {
    pub(super) fn delete_template(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let value = item
            .value
            .scalar()
            .or_else(|| {
                field(fields, "division_template").and_then(|item| item.value.read_string())
            })
            .unwrap_or_default();
        let label = std::str::from_utf8(installed_content::parser::c_string(value))
            .map_err(|_| "template name is not UTF-8")?;
        let disband = installed_content::parser::read_bool_field(fields, "disband", false);
        self.line(
            if disband {
                b"EFFECT_DISBAND_UNIT_TEMPLATE_AND_UNITS"
            } else {
                b"EFFECT_DELETE_UNIT_TEMPLATE_AND_UNITS"
            },
            &[("NAME", Argument::Text(label))],
            scopes,
            true,
        ) // 14037EC90
    }
}
