use super::*;

mod scorers;
#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(super) fn save_event_target(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        // 14038D670 -> 14052D4F0.
        self.evaluator()?
            .save_tooltip_target(item.value.read_string().unwrap_or_default(), scopes)?;
        Ok(String::new())
    }

    pub(super) fn save_global_event_target(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        // 14038D6B0 -> 1401ED290, cleared by the existing 140529520 session exit.
        self.evaluator()?.save_global_tooltip_target(
            item.value.read_string().unwrap_or_default(), scopes,
        )?;
        Ok(String::new())
    }

    pub(super) fn variable_effect(
        &self,
        item: &Item,
        scopes: Scopes,
        operation: &str,
        temporary: bool,
    ) -> Result<String, String> {
        // 14039A7D0 short-circuits before reading operands when tooltip is empty.
        // 140379020 first executes vf13 for temporary scalars, even then.
        // 1403B691F passes zero to ReadStringInto's LF check (1424ABB50).
        let tooltip = children(item).iter().rev().find(|field| field.keyword_is("tooltip"))
            .and_then(|field| field.value.read_string()).unwrap_or_default();
        if !temporary && tooltip.is_empty() {
            return Ok(String::new());
        }
        let values = self.evaluator()?.description_variable(
            operation, item, scopes, temporary, !tooltip.is_empty(),
        )?;
        let Some((left, right)) = values else { return Ok(String::new()); };
        self.localised(tooltip, &[
            ("LEFT", Argument::Fixed5(left)), ("RIGHT", Argument::Fixed5(right)),
        ], scopes, true)
    }

    pub(super) fn temp_variable_builder(
        &self,
        item: &Item,
        scopes: Scopes,
        operation: &str,
    ) -> Result<String, String> {
        let evaluator = self.evaluator()?;
        let result = match operation {
            // 14037B520 -> 14034EDD0; 14047BF30 -> 14047B280.
            "clamp_temp_variable" | "modulo_temp_variable" => evaluator
                .description_variable(operation, item, scopes, true, false).map(|_| ()),
            // 14047BF80 -> 14047B530. The pre-RNG invalid-range branch is known.
            "set_temp_variable_to_random" => evaluator.description_random_temp(item, scopes),
            // 14047BEE0 -> 14047B0B0 initializes zero for an unknown type.
            "get_supply_vehicles_temp" => evaluator.description_supply_temp(item, scopes),
            _ => unreachable!(),
        };
        if let Err(why) = &result {
            evaluator.refuse_description_writer(format!("effect {operation}: {why}"));
        }
        result.map(|_| String::new())
    }
}

// Empty vf7 (140176EE0/1402E9B40) never executes setters, even temp arrays.
pub(super) fn empty() -> Result<String, String> {
    Ok(String::new())
}
