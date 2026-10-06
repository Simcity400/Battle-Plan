use super::*;

#[cfg(test)]
mod tests;

// Native own text is empty (140176EE0 / 1402E9B40); setters never execute.
pub(super) fn describe() -> Result<String, String> {
    Ok(String::new())
}

impl Renderer<'_, '_, '_> {
    pub(super) fn empty_child_container(&self, item: &Item) -> Result<String, String> {
        self.require_no_effect_children(item)?;
        describe()
    }

    pub(super) fn require_no_effect_children(&self, item: &Item) -> Result<(), String> {
        // ReadEffect fallbacks (1403B1120 / 14031BA90) add native children.
        let token = item
            .keyword()
            .and_then(|key| installed_content::effect_keys::registered(&key.folded()));
        for child in children(item) {
            if token.is_some_and(|token| {
                child
                    .keyword()
                    .is_some_and(|key| installed_content::effect_keys::is_own_field(token, &key.folded()))
            }) {
                continue;
            }
            let adds_child = match self.context.content.key_names {
                Some(names) => installed_content::effect_keys::adds_child(child, names)?,
                None if child.keyword_is("else") || child.keyword_is("else_if") => false,
                None if self.effect_class(child).is_some() => true,
                None => {
                    return Err(format!(
                        "effect content names unavailable; cannot distinguish native child {} from ignored field (ReadEffect 1405345D0)",
                        key(child)
                    ));
                }
            };
            if adds_child {
                return Err("children-bearing form needs the shared 140532780 child walk / 140533D80 probe, which are not ported".into());
            }
        }
        Ok(())
    }
}
