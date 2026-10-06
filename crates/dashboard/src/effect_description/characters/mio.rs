use super::*;

impl Renderer<'_, '_, '_> {
    // CTraitTooltipEffect 1402FF8F0 / CUnlockTraitEffect 1402FFE10 call
    // 1413978B0. Its 141397DB0 -> 140529390 reads the independent MIO
    // carrier even when THIS is a country or character. IndustrialOrganisations
    // already records the handles; the graph cannot distinguish absent from
    // dropped +0x78/+0x7C, so neither the raw-token fallback nor a name is safe.
    pub(in crate::effect_description) fn char_mio_trait(&self) -> Result<String, String> {
        Err("CEventScope INDUSTRIAL_ORGANISATION_REFERENCE (+0x78/+0x7C) is not represented by the event graph (140529390); IndustrialOrganisations records the handles".into())
    }
}

#[cfg(test)]
mod tests;
