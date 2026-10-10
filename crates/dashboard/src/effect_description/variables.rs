use super::*;

mod scorers;
#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(super) fn save_event_target(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        self.evaluator()?
            .save_tooltip_target(item.value.read_string().unwrap_or_default(), scopes)?;
        Ok(String::new())
    }

    pub(super) fn save_global_event_target(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // cleared by the existing session exit.
        self.evaluator()?
            .save_global_tooltip_target(item.value.read_string().unwrap_or_default(), scopes)?;
        Ok(String::new())
    }

    pub(super) fn variable_effect(
        &self,
        item: &Item,
        scopes: Scopes,
        operation: &str,
        temporary: bool,
    ) -> Result<String, String> {
        // short-circuits before reading operands when tooltip is empty.
        // Temporary scalar writes execute even when the tooltip is empty.
        // Disable the trailing-LF check when copying the tooltip.
        let tooltip = children(item)
            .iter()
            .rev()
            .find(|field| field.keyword_is("tooltip"))
            .and_then(|field| field.value.read_string())
            .unwrap_or_default();
        if !temporary && tooltip.is_empty() {
            return Ok(String::new());
        }
        let values = self.evaluator()?.description_variable(
            operation,
            item,
            scopes,
            temporary,
            !tooltip.is_empty(),
        )?;
        let Some((left, right)) = values else {
            return Ok(String::new());
        };
        self.localised(
            tooltip,
            &[
                ("LEFT", Argument::Fixed5(left)),
                ("RIGHT", Argument::Fixed5(right)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn temp_variable_builder(
        &self,
        item: &Item,
        scopes: Scopes,
        operation: &str,
    ) -> Result<String, String> {
        let evaluator = self.evaluator()?;
        let result = match operation {
            "clamp_temp_variable" | "modulo_temp_variable" => evaluator
                .description_variable(operation, item, scopes, true, false)
                .map(|_| ()),
            // TLS RNG uses recorded variables.
            "set_temp_variable_to_random" => evaluator.description_random_temp(item, scopes),
            // initializes zero for an unknown type.
            "get_supply_vehicles_temp" => evaluator.description_supply_temp(item, scopes, || {
                self.context
                    .text
                    .content()
                    .environment()?
                    .defines()
                    .fixed5("NSupply", "MIN_TRAIN_REQUIREMENT")
                    .map_err(|why| why.to_string())
            }),
            _ => unreachable!(),
        };
        if let Err(why) = &result {
            evaluator.refuse_description_writer(format!("effect {operation}: {why}"));
        }
        result.map(|_| String::new())
    }
}

// Empty never executes setters, even temp arrays.
pub(super) fn empty() -> Result<String, String> {
    Ok(String::new())
}
