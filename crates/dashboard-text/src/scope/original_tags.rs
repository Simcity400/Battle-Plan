use crate::triggers::Failure;

use replay_data::{Hour, rows::TableSchema, rows::tables};
use replay_frame::{frame::SharedView, frame::TableView};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct OriginalTagLists<'a> {
    table: Lists<'a>,
}

#[derive(Clone, Default)]
struct Lists<'a>(Option<SharedView>, std::marker::PhantomData<&'a ()>);

impl std::fmt::Debug for OriginalTagLists<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalTagLists")
            .field("table", self.table())
            .finish()
    }
}

impl crate::triggers::inputs::Fingerprint for OriginalTagLists<'_> {
    fn fingerprint(&self, into: &mut std::hash::DefaultHasher) {
        self.table().as_deref().fingerprint(into);
    }
}

impl<'a> OriginalTagLists<'a> {
    pub fn read(frame: &dyn replay_frame::frame::DecodedSource) -> Self {
        Self {
            table: Lists(Some(frame.shared(tables::ReservedCountryNaming::ID.index())), std::marker::PhantomData),
        }
    }

    pub fn read_in(frame: replay_frame::frame::HourFrame<'a>) -> Self {
        match frame {
            replay_frame::frame::HourFrame::Decoded(frame) => Self::read(frame),
        }
    }

    fn table(&self) -> &Option<SharedView> {
        &self.table.0
    }

    fn row(&self) -> Result<&replay_data::rows::ReservedCountryNamingRow, Failure> {
        let table = self.table().as_deref().ok_or_else(|| {
            Failure::input(
                replay_data::rows::TableId::ReservedCountryNaming,
                "",
                None,
                "reserved country naming native lists are not recorded",
            )
        })?;
        let TableView::Complete(rows) = table else {
            return Err(Failure::input(
                replay_data::rows::TableId::ReservedCountryNaming,
                "",
                None,
                "reserved country naming native lists are unavailable this hour",
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
                "reserved country naming has no singleton row for native country lists",
            ));
        };
        Ok(row)
    }

    pub fn bucket(&self, canonical: u32) -> Result<&[u32], Failure> {
        let buckets = self
            .row()?
            .original_tag_buckets
            .require("original tag buckets of reserved country naming")
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
                    "original tag buckets of reserved country naming capture refused",
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

    pub(crate) fn original_reference(&self) -> Result<i32, Failure> {
        self.row()?
            .original_tag_reference
            .require("original tag reference of reserved country naming")
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
                    "original tag reference of reserved country naming capture refused",
                )
            })
            .map(|raw| raw.max(0))
    }

    pub(crate) fn reserved_owned_states(&self) -> Result<i32, Failure> {
        self.row()?
            .owned_state_count
            .require("owned state count of reserved country naming")
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
                    "owned state count of reserved country naming capture refused",
                )
            })
    }

    pub(crate) fn reserved_faction(&self) -> Result<Option<u64>, Failure> {
        self.row()?
            .faction
            .require("faction of reserved country naming")
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
                    "faction of reserved country naming capture refused",
                )
            })
            .map(|faction| (faction != 0).then_some(faction))
    }

    pub fn country_array(&self) -> Result<&[u32], Failure> {
        self.row()?
            .country_array
            .require("country array of reserved country naming")
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
                    "country array of reserved country naming capture refused",
                )
            })
    }

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
