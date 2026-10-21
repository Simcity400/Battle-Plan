//! A lazy successful conventional census, keyed only by held input views.
use super::*;
use replay_data::rows::TableId;
use viewer_foundation::map::memo::Keyed;

const INPUTS: &[TableId] = &[
    TableId::VariantDefinitions,
    TableId::VariantUpgrades,
    TableId::AvailableEquipmentVariants,
    TableId::CountryLinks,
    TableId::SimulationFeatures,
    TableId::LexicalTokens,
];

pub(super) struct RetainedBridge(Keyed<bridge::Retained>);

impl Default for RetainedBridge {
    fn default() -> Self {
        // Recency bounds storage, never which country or variant is admitted.
        Self(Keyed::new(INPUTS, 8))
    }
}

impl RetainedBridge {
    pub(super) fn read<'a>(
        &self,
        frame: &'a DecodedFrame,
        content: &'a model_selection::Content,
    ) -> Result<bridge::Prepared<'a>, String> {
        self.0
            .get_or_build(frame, (), |projected| {
                bridge::Retained::read(projected, content).map(Arc::new)
            })
            .map(|retained| retained.prepared())
    }
}

#[cfg(test)]
mod tests;
