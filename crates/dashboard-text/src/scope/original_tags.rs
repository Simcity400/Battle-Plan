//! Exact hourly original-tag buckets and physical country-array order.
//! Changing an original tag can reorder a bucket without changing final
//! country operands; canonical country rows cannot reconstruct either list.
use crate::triggers::Failure;

use replay_data::{Hour, rows::TableSchema, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::TableView};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct OriginalTagLists {
    table: Option<SharedView>,
}

impl OriginalTagLists {
    pub fn read(frame: &DecodedFrame) -> Self {
        Self {
            table: Some(frame.tables[tables::ReservedCountryNaming::ID.index()].clone()),
        }
    }

    fn row(&self) -> Result<&replay_data::rows::ReservedCountryNamingRow, Failure> {
        let table = self.table.as_deref().ok_or_else(|| {
            Failure::input(
                replay_data::rows::TableId::ReservedCountryNaming,
                "",
                None,
                "ReservedCountryNaming native lists are not recorded",
            )
        })?;
        let TableView::Complete(rows) = table else {
            return Err(Failure::input(
                replay_data::rows::TableId::ReservedCountryNaming,
                "",
                None,
                "ReservedCountryNaming native lists are unavailable this hour",
            ));
        };
        let [row] = rows
            .rows::<tables::ReservedCountryNaming>()
            .unwrap_or_default()
        else {
            return Err(Failure::input(
                replay_data::rows::TableId::ReservedCountryNaming,
                "",
                None,
                "ReservedCountryNaming has no singleton row for native country lists",
            ));
        };
        Ok(row)
    }

    /// Raw own-country IDs, in stored order, retaining every occurrence.
    pub fn bucket(&self, canonical: u32) -> Result<&[u32], Failure> {
        let buckets = self
            .row()?
            .original_tag_buckets
            .require("ReservedCountryNaming.original_tag_buckets")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "original_tag_buckets",
                    None,
                    why,
                )
            })?
            .as_ref()
            .ok_or_else(|| -> Failure {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "original_tag_buckets",
                    None,
                    "ReservedCountryNaming.original_tag_buckets capture refused",
                )
            })?;
        buckets
            .get(canonical as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| -> Failure {
                format!(
                    "original tag canonical index {canonical} is outside recorded native buckets"
                )
                .into()
            })
    }

    /// Reserved country's numeric original tag, independent of its name bytes.
    pub(crate) fn original_reference(&self) -> Result<i32, Failure> {
        self.row()?
            .original_tag_reference
            .require("ReservedCountryNaming.original_tag_reference")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "original_tag_reference",
                    None,
                    why,
                )
            })?
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "original_tag_reference",
                    None,
                    "ReservedCountryNaming.original_tag_reference capture refused",
                )
            })
            .map(|raw| raw.max(0))
    }

    /// Reserved country owned state count, demanded independently of the lists.
    pub(crate) fn reserved_owned_states(&self) -> Result<i32, Failure> {
        self.row()?
            .owned_state_count
            .require("ReservedCountryNaming.owned_state_count")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "owned_state_count",
                    None,
                    why,
                )
            })?
            .ok_or_else(|| -> Failure {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "owned_state_count",
                    None,
                    "ReservedCountryNaming.owned_state_count capture refused",
                )
            })
    }

    /// Actual reserved country faction: null is None, not a missing read.
    pub(crate) fn reserved_faction(&self) -> Result<Option<u64>, Failure> {
        self.row()?
            .faction
            .require("ReservedCountryNaming.faction")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "faction",
                    None,
                    why,
                )
            })?
            .ok_or_else(|| -> Failure {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "faction",
                    None,
                    "ReservedCountryNaming.faction capture refused",
                )
            })
            .map(|faction| (faction != 0).then_some(faction))
    }

    /// Physical country-array order, including aliases and zero.
    pub fn country_array(&self) -> Result<&[u32], Failure> {
        self.row()?
            .country_array
            .require("ReservedCountryNaming.country_array")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "country_array",
                    None,
                    why,
                )
            })?
            .as_deref()
            .ok_or_else(|| -> Failure {
                Failure::input(
                    replay_data::rows::TableId::ReservedCountryNaming,
                    "country_array",
                    None,
                    "ReservedCountryNaming.country_array capture refused",
                )
            })
    }

    /// Read requested snapshots independently; no earlier-hour reconstruction
    /// can establish the native cache's mutation order.
    pub fn derive(
        replay: &replay::container::Replay,
        branch: replay_data::BranchId,
        hours: &std::collections::BTreeSet<Hour>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> std::io::Result<BTreeMap<Hour, Self>> {
        let selected = [tables::ReservedCountryNaming::ID];
        let mut out = BTreeMap::new();
        for block in replay.index.iter().filter(|block| {
            block.branch == branch
                && hours
                    .range(block.first_hour..=block.last_hour)
                    .next()
                    .is_some()
        }) {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "request cancelled",
                ));
            }
            let mut decoder = replay_frame::frame::FrameDecoder::for_tables(&selected);
            for frame in replay.read_block_selected(block, &selected)? {
                let decoded = decoder.advance(&frame)?;
                if hours.contains(&decoded.hour) {
                    out.insert(decoded.hour, Self::read(decoded));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
