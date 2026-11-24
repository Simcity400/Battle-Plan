
mod factory_groups;
mod factory_mix;
mod factory_parts;
mod game_score;
pub mod nuclear_arsenal;
pub mod world_tension;
pub mod world_tension_stack;

pub use factory_groups::{FactoryGroups, FactoryGroupsParams, FactoryGroupsReply};
pub use factory_mix::{FactoryMix, FactoryMixParams, FactoryMixPart, FactoryMixReply, FactoryMixRow, factory_channel_value};
pub use factory_parts::{FactoryParts, FactoryPartsParams, FactoryPartsReply};
pub use game_score::{GameScore, GameScoreParams, GameScoreReply};
pub use nuclear_arsenal::{NuclearArsenal, NuclearArsenalParams, NuclearArsenalReply};
pub use world_tension::{WorldTension, WorldTensionParams, WorldTensionReply};
pub use world_tension_stack::{WorldTensionStack, WorldTensionStackParams, WorldTensionStackReply};

pub(crate) fn forget(source: &dashboard_base::Source) {
    world_tension_stack::forget(source);
}

use dashboard_query::identity::{Holder, Holders};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

fn branch(text: &str) -> Result<(), &'static str> {
    text.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
}

pub(crate) struct Named {
    ids: HashMap<u32, String>,
    slots: HashMap<u32, Vec<(i32, i32, String)>>,
}

impl Named {
    pub(crate) fn new(holders: &Holders, hour: i32) -> Self {
        let mut ids = HashMap::new();
        let mut slots: HashMap<u32, Vec<(i32, i32, String)>> = HashMap::new();
        for (tag, runs) in &holders.tags {
            for run in runs {
                let Holder::Found { slot, id } = run.holder else { continue };
                if run.first <= hour && hour <= run.last {
                    ids.entry(id).or_insert_with(|| tag.clone());
                }
                slots.entry(slot).or_default().push((run.first, run.last, tag.clone()));
            }
        }
        for runs in slots.values_mut() {
            runs.sort_unstable_by_key(|run| run.0);
        }
        Self { ids, slots }
    }

    pub(crate) fn id(&self, id: u32) -> Option<&str> {
        self.ids.get(&id).map(String::as_str)
    }

    pub(crate) fn slot(&self, slot: u32, hour: i32) -> Option<(&str, bool)> {
        let runs = self.slots.get(&slot)?;
        if let Some(run) = runs.iter().find(|run| run.0 <= hour && hour <= run.1) {
            return Some((&run.2, false));
        }
        runs.iter().rev().find(|run| run.1 < hour).map(|run| (run.2.as_str(), true))
    }

    pub(crate) fn known(&self, slot: u32) -> bool {
        self.slots.contains_key(&slot)
    }
}

fn rows<'f, T: replay_data::rows::TableSchema>(frame: &'f replay_frame::frame::DecodedFrame) -> Result<&'f [T::Row], String> {
    frame.complete::<T>().map_err(|error| error.to_string())
}
