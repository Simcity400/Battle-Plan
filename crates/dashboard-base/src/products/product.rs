//! The product contract, its shared parameters and the reply checks every
//! product uses. The registry in `products/mod.rs` erases products to their keys.

use std::{io, sync::atomic::AtomicBool};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::Source;

pub use crate::config::{ContentOptions, LoadContext};
pub use crate::protocol::validate_tag;
// Products name the air wing reader at this path.
pub use crate::air_wing_names as air_wings;
// The public path; products also name the identity helpers through it.
pub use crate::identity;

/// Leave room for the lease envelope and escaped product JSON inside the
/// transport's complete-response bound. History products share this reserve.
pub const MAX_PRODUCT_REPLY_BYTES: usize = 24 * 1024 * 1024;

/// Serialised with the lease identity the frontend used, so a reply can never
/// be attributed to another recording.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<'a, T: Serialize> {
    pub source_id: &'a str,
    pub revision: &'a str,
    pub product: &'static str,
    pub data: T,
}

/// Refuse a complete reply instead of dropping recorded entries to fit it.
pub fn check_reply_budget<T: Serialize>(reply: &T) -> io::Result<()> {
    if crate::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(format!(
            "This product reply exceeds the transport reply budget of {MAX_PRODUCT_REPLY_BYTES} bytes"
        )));
    }
    Ok(())
}

/// The fixed64 finite domain excludes the three
/// infinity/undefined sentinels while preserving every signed finite value.
pub fn finite_fixed64(raw: i64) -> bool {
    (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// The engine's saturated fixed5 value, which also marks a fixed5 cache it never computed.
pub const FIXED5_MAX: i64 = 0x53e2_d61f_d320;

/// A goal's progress cache as a percentage; the game leaves [`FIXED5_MAX`] in goals without ratio progress.
pub fn faction_goal_progress_percent(raw: i64) -> Option<f64> {
    (finite_fixed64(raw) && raw != FIXED5_MAX).then_some(raw as f64 / 1000.0)
}

/// All product replies carry content diagnostics at the shared serialization boundary.
pub trait ReplyWarnings {
    fn warnings_mut(&mut self) -> &mut Vec<String>;
}

/// Implements [`ReplyWarnings`] for replies whose warnings are their `warnings` field.
#[macro_export]
macro_rules! reply_warnings {
    ($($reply:ty),+ $(,)?) => {$(
        impl $crate::products::ReplyWarnings for $reply {
            fn warnings_mut(&mut self) -> &mut Vec<String> {
                &mut self.warnings
            }
        }
    )+};
}

pub trait Product: 'static {
    const KEY: &'static str;
    /// Whether any reply field reads localisation, artwork or content definitions.
    const CONTENT_DEPENDENT: bool;
    /// Whether it answers a hover or label at one hour (one block at most): served on its
    /// own threads, so a whole-recording scan never delays it.
    const INTERACTIVE: bool = false;
    /// Whether a reply depends on this machine (its displays, settings outside the content, its
    /// GPU), which no kept reply's identity captures: such a reply is never kept on disk.
    const MACHINE_DEPENDENT: bool = false;
    /// Whether a reply reads a prepared family's publication, which grows while the family
    /// prepares: such a reply is never kept on disk.
    const PUBLISHED: bool = false;
    /// How its served replies are checked between two preparations of a
    /// recording; none checks them by the generic plan.
    const SERVED: Option<&'static crate::prepared::served::Plan> = None;
    type Params: DeserializeOwned + TS + 'static;
    type Reply: Serialize + TS + ReplyWarnings + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    /// Every table it or its prepared family may read at an hour through the native hour reader
    /// ([`crate::prepared::query::native`]): a recording's change-run index holds exactly the
    /// union of these over the registry, and the map's tables.
    fn native_tables() -> Vec<replay_data::rows::TableId> {
        Vec::new()
    }

    /// TypeScript declarations this product introduces, parameters and reply first.
    fn types() -> Vec<String> {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    /// The whole-recording reply. A product served only by prepared queries
    /// keeps its loader as its tests' oracle (`#[cfg(test)]`) and refuses here.
    fn load(
        _source: &Source,
        _params: &Self::Params,
        _context: &LoadContext,
        _cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            prepared_only(Self::KEY),
        ))
    }
}

pub fn prepared_only(key: &str) -> String {
    format!("{key} is served by prepared queries")
}

/// Parameters shared by the per-country products.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryParams {
    pub branch_id: String,
    pub tag: String,
}

impl CountryParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_tag(&self.tag)
    }

    pub fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

/// Parameters of a product about the whole world of one branch.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchParams {
    pub branch_id: String,
}

impl BranchParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }

    pub fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

/// Selection checks shared by products drawing several countries.
pub fn validate_countries(branch_id: &str, tags: &[String]) -> Result<(), &'static str> {
    if tags.is_empty() {
        return Err("Invalid country selection");
    }
    for tag in tags {
        validate_tag(tag)?;
    }
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    Ok(())
}

/// A census API has no warnings field, so an unrepresentable key refuses its map.
pub fn require_displayable_tags(
    refused: &std::collections::BTreeMap<Vec<u8>, usize>,
) -> io::Result<()> {
    if refused.is_empty() {
        return Ok(());
    }
    Err(dashboard_core::refused(format!(
        "Country tags are not UTF-8 (bytes: observation count): {refused:?}; the census cannot be represented"
    )))
}

pub use crate::metadata::recorded_rows;

/// A table-state reason that can be shared across dated history runs. Read
/// dates and changing truncation counts belong to coverage, not reason text.
pub fn table_missing_reason(
    frame: &replay_frame::frame::DecodedFrame,
    table: replay_data::rows::TableId,
) -> Option<String> {
    use replay_frame::frame::TableView;
    match &*frame.tables[table.index()] {
        TableView::Complete(rows) if rows.table() == table => None,
        TableView::Complete(_) => Some(format!("{table:?} contains another table's rows")),
        TableView::Truncated { .. } => Some(format!("{table:?} is truncated")),
        TableView::Unavailable { reason } => {
            if reason.starts_with("missing table baseline") {
                return Some(format!("{table:?} baseline missing"));
            }
            // FrameEncoder withholds partial rows at keyframes, retaining the
            // truncation count in its named unavailable marker.
            let truncated = reason
                .strip_prefix("truncated at keyframe: ")
                .and_then(|count| count.strip_suffix(" rows not read"))
                .is_some_and(|count| count.parse::<u32>().is_ok_and(|count| count != 0));
            Some(if truncated {
                format!("{table:?} is truncated")
            } else {
                reason.clone()
            })
        }
        TableView::NotRecorded { mode } => {
            Some(format!("{table:?} is not recorded in a {mode} recording"))
        }
    }
}
