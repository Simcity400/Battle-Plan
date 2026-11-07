//! Exact hourly original-tag buckets and physical country-array order.
//! Changing an original tag can reorder a bucket without changing final
//! country operands; canonical country rows cannot reconstruct either list.
use crate::triggers::Failure;

use replay_data::{Hour, rows::TableSchema, rows::tables};
use replay_frame::{frame::ColumnView, frame::SharedView, frame::TableView};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Default)]
pub struct OriginalTagLists<'a> {
    table: Lists<'a>,
}

/// Where the lists are read: a decoded frame's shared view, or a walk's view, its one row
/// decoded only once a trigger reads the lists.
#[derive(Clone)]
enum Lists<'a> {
    Shared(Option<SharedView>),
    Walked(ColumnView<'a>, OnceLock<Option<SharedView>>),
}

impl Default for Lists<'_> {
    fn default() -> Self {
        Self::Shared(None)
    }
}

impl std::fmt::Debug for OriginalTagLists<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalTagLists")
            .field("table", self.table())
            .finish()
    }
}

impl<'a> OriginalTagLists<'a> {
    pub fn read(frame: &dyn replay_frame::frame::DecodedSource) -> Self {
        Self {
            table: Lists::Shared(Some(
                frame.shared(tables::ReservedCountryNaming::ID.index()),
            )),
        }
    }

    /// [`Self::read`] of any frame's tables: a walk's view is borrowed for the hour.
    pub fn read_in(frame: replay_frame::frame::HourFrame<'a>) -> Self {
        match frame {
            replay_frame::frame::HourFrame::Decoded(frame) => Self::read(frame),
            replay_frame::frame::HourFrame::Walked(columns) => Self {
                table: Lists::Walked(
                    columns.view(tables::ReservedCountryNaming::ID),
                    OnceLock::new(),
                ),
            },
        }
    }

    fn table(&self) -> &Option<SharedView> {
        match &self.table {
            Lists::Shared(table) => table,
            Lists::Walked(view, table) => table.get_or_init(|| {
                view.to_table_view(tables::ReservedCountryNaming::ID.full_mask())
                    .ok()
                    .map(SharedView::from)
            }),
        }
    }

    fn row(&self) -> Result<&replay_data::rows::ReservedCountryNamingRow, Failure> {
        let table = self.table().as_deref().ok_or_else(|| {
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
        replay: &replay::campaign::Replay,
        branch: replay_data::BranchId,
        hours: &std::collections::BTreeSet<Hour>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> std::io::Result<BTreeMap<Hour, OriginalTagLists<'static>>> {
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
            replay.visit_blocks(
                &[block],
                &selected,
                None,
                |_, _, hour_frame| {
                let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
                if hours.contains(&decoded.hour) {
                    out.insert(decoded.hour, OriginalTagLists::read(decoded));
                }
                    Ok(true)
                },
                || Ok(()),
            )?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
