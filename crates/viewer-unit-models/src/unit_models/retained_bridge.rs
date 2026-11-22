use super::*;
use replay_data::rows::TableId;
use viewer_runtime::memo::Keyed;

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
        Self(Keyed::new(INPUTS))
    }
}

impl RetainedBridge {
    pub(super) fn read(
        &self,
        frame: &DecodedFrame,
        content: &model_selection::Content,
    ) -> Result<Arc<bridge::Retained>, String> {
        self.0.get_or_build(frame, (), |projected| {
            bridge::Retained::read(projected, content).map(Arc::new)
        })
    }
}

#[cfg(test)]
mod tests;
