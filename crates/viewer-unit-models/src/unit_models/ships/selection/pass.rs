use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key<'rows> {
    variant: u64,
    flags: u64,
    definition: Option<(u32, u32)>,
    valid: u8,
    fallback: &'rows [u8],
}

impl<'rows> Key<'rows> {
    fn new(row: &'rows replay_data::rows::ShipModelRow, definition: Option<(u32, u32)>) -> Self {
        Self {
            variant: row.variant,
            flags: row.formation_flags,
            definition,
            valid: row
                .counter
                .as_ref()
                .map_or(0, |counter| counter.definition_valid),
            fallback: &row.fallback_sprite,
        }
    }
}

#[derive(Default)]
struct Facts<'rows>(BTreeMap<Key<'rows>, Option<String>>);
impl<'rows> Facts<'rows> {
    fn resolve(
        &mut self,
        key: Key<'rows>,
        build: impl FnOnce() -> Result<Option<String>, String>,
    ) -> Result<Option<String>, String> {
        if let Some(value) = self.0.get(&key) {
            return Ok(value.clone());
        }
        let value = build()?;
        self.0.insert(key, value.clone());
        Ok(value)
    }
}

pub(in crate::unit_models::ships) struct Pass<'context, 'hour, 'rows> {
    assets: &'context UnitModelAssets,
    hour: Option<&'context Hour<'hour>>,
    _frame: &'rows DecodedFrame,
    facts: Facts<'rows>,
}

impl<'context, 'hour, 'rows> Pass<'context, 'hour, 'rows> {
    pub(in crate::unit_models::ships) fn new(
        assets: &'context UnitModelAssets,
        frame: &'rows DecodedFrame,
        hour: Option<&'context Hour<'hour>>,
    ) -> Self {
        Self {
            assets,
            hour,
            _frame: frame,
            facts: Facts::default(),
        }
    }
}

impl<'context, 'hour, 'rows> Names<'rows> for Pass<'context, 'hour, 'rows> {
    fn name<'tag>(
        &mut self,
        tag: impl FnOnce() -> Result<&'tag str, String>,
        row: &'rows replay_data::rows::ShipModelRow,
        graphics: &[u8],
        definition: Option<(u32, u32)>,
    ) -> Result<Option<String>, String> {
        let assets = self.assets;
        let hour = self.hour;
        name_with_generated(assets, tag, row, graphics, || {
            self.facts.resolve(Key::new(row, definition), || {
                generated(assets, row, definition, hour)
            })
        })
    }
}

#[cfg(test)]
mod tests;
