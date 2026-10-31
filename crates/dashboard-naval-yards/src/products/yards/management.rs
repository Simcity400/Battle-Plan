//! The two yard views' tables, and what their naming operands hold.
use super::*;
use dashboard_base::prepared::prepass::{Held, HeldBytes};

pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::LexicalTokens,
    TableId::TaskForceCounterInputs,
    TableId::VariantDefinitions,
    TableId::NavalRepairCountries,
    TableId::CountryFactories,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::ProductionLines,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::NavalRepairTaskForces,
    TableId::NavalRepairChildren,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];

impl Held for Operand {
    fn held(&self, bytes: &mut HeldBytes) {
        match self {
            Operand::ShipCache { bytes: text, .. } | Operand::Task { bytes: text, .. } => {
                bytes.heap(text);
            }
            Operand::ShipRebuilt { naming, group, .. } => {
                bytes.heap(&naming.override_name).heap(group);
            }
            Operand::Design(raw) => {
                bytes
                    .heap(&raw.token)
                    .heap(&raw.display_name)
                    .heap(&raw.suffix)
                    .heap(&raw.name_tag)
                    .heap(&raw.type_tag)
                    .heap(&raw.design)
                    .heap(&raw.refusals);
            }
            Operand::Class { spelling, .. } => {
                bytes.heap(spelling);
            }
            Operand::Port(port) => {
                bytes.heap(&port.override_name).heap(&port.owner_tag);
                match &port.state {
                    Ok(state) => {
                        bytes.heap(&state.custom).heap(&state.owner);
                    }
                    Err(why) => {
                        bytes.heap(why);
                    }
                }
            }
            Operand::Refusal(why) => {
                bytes.heap(why);
            }
            Operand::Policy { .. } => {}
        }
    }
}
