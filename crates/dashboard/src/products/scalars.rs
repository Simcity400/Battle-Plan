//! Country scalars at an hour and over a campaign: game scores, nuclear stockpiles, factory parts,
//! world tension and its threat events. Their lines, values at the head and ranks are declared
//! series (`dashboard_declare`: `country-game-scores`, `country-nukes`, `country-factories`,
//! `world-tension`, `threat-contributions`); these products add what a declared series cannot:
//! every country's row at an hour with its reasons, a sum over a country's parts, a breakdown and
//! the threat sources joined into events.

mod factory_groups;
mod factory_parts;
mod game_score;
pub(crate) mod nuclear_arsenal;
mod threat_events;
pub(crate) mod world_tension;

pub use factory_groups::{FactoryGroups, FactoryGroupsParams, FactoryGroupsReply};
pub use factory_parts::{FactoryParts, FactoryPartsParams, FactoryPartsReply};
pub use game_score::{GameScore, GameScoreParams, GameScoreReply};
pub use nuclear_arsenal::{NuclearArsenal, NuclearArsenalParams, NuclearArsenalReply};
pub use threat_events::{ThreatEvents, ThreatEventsParams, ThreatEventsReply};
pub use world_tension::{WorldTension, WorldTensionParams, WorldTensionReply};

use dashboard_query::identity::{Holder, Holders};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

fn branch(text: &str) -> Result<(), &'static str> {
    text.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
}

/// Who held each raw country id and each canonical slot at an hour, and each slot's holders over
/// the timeline, so a slot no tag holds now is named by the tag that last held it.
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

    /// The tag holding raw country `id` at the hour.
    pub(crate) fn id(&self, id: u32) -> Option<&str> {
        self.ids.get(&id).map(String::as_str)
    }

    /// The tag holding `slot` at `hour`, or the one that last held it before (`true`).
    pub(crate) fn slot(&self, slot: u32, hour: i32) -> Option<(&str, bool)> {
        let runs = self.slots.get(&slot)?;
        if let Some(run) = runs.iter().find(|run| run.0 <= hour && hour <= run.1) {
            return Some((&run.2, false));
        }
        runs.iter().rev().find(|run| run.1 < hour).map(|run| (run.2.as_str(), true))
    }

    /// Whether any tag ever held `slot`.
    pub(crate) fn known(&self, slot: u32) -> bool {
        self.slots.contains_key(&slot)
    }
}

/// A table's rows at an hour, or why they are not known.
fn rows<'f, T: replay_data::rows::TableSchema>(frame: &'f replay_frame::frame::DecodedFrame) -> Result<&'f [T::Row], String> {
    frame.complete::<T>().map_err(|error| error.to_string())
}
