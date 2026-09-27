//! Data products: one module per widget data set, registered once below.
//!
//! A product turns a source plus typed parameters into one compact reply that
//! covers a whole branch, so playback in the frontend is a local lookup. The
//! service, transport, host and frontend never name a product; they route by
//! its key.

pub mod artwork;
pub mod country_flags;
pub mod country_groups;
pub mod country_measure;
pub mod country_stability;
pub mod country_styles;
pub mod idea_history;
pub mod land_owners;
pub mod national_focus;
pub mod politics_content;
pub mod research;

use std::{io, sync::atomic::AtomicBool};

use installed_content::ResolveOptions;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::{Source, transport::bounded_json};

/// Game and user folders chosen in Settings, needed by products that read
/// artwork or localisation.
#[derive(Clone)]
pub struct ContentOptions {
    pub options: ResolveOptions,
    pub allow_unverified: bool,
}

pub struct LoadContext {
    pub content: Option<ContentOptions>,
}

impl LoadContext {
    pub fn content(&self) -> io::Result<&ContentOptions> {
        self.content
            .as_ref()
            .ok_or_else(|| crate::invalid("Choose the game and user folders in Settings first"))
    }
}

pub trait Product: 'static {
    const KEY: &'static str;
    type Params: DeserializeOwned + TS + 'static;
    type Reply: Serialize + TS + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    /// TypeScript declarations this product introduces, parameters and reply first.
    fn types() -> Vec<String> {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply>;
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

/// The one check for a requested country tag, shared by products and the map.
/// Tags are lookup keys compared with recorded tags, never paths. The engine's
/// tag loader (`CCountryDatabase` load_tags, `14070e900`) takes any non-NUL
/// bytes, so no character set is assumed; only emptiness, NUL and a generous
/// length bound are refused.
pub fn validate_tag(tag: &str) -> Result<(), &'static str> {
    if tag.is_empty() || tag.len() > 128 || tag.contains('\0') {
        return Err("Invalid country tag");
    }
    Ok(())
}

/// The rows a table holds this hour, complete or truncated; an unavailable table
/// holds none. For products that use whatever rows were recorded.
pub(crate) fn recorded_rows<T: replay::rows::TableSchema>(
    frame: &replay::frame::DecodedFrame,
) -> io::Result<&[T::Row]> {
    use replay::frame::TableView;
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<T>().ok_or_else(|| {
                crate::invalid(format!(
                    "hour {} {:?} contains another table's rows",
                    frame.hour,
                    T::ID
                ))
            })
        }
        TableView::Unavailable { .. } => Ok(&[]),
    }
}

/// Serialised with the lease identity the frontend used, so a reply can never
/// be attributed to another recording.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a, T: Serialize> {
    source_id: &'a str,
    revision: &'a str,
    product: &'static str,
    data: T,
}

/// A product erased to its key and JSON entry points.
pub struct Registered {
    pub key: &'static str,
    pub load:
        fn(&Source, &str, serde_json::Value, &LoadContext, &AtomicBool) -> Result<String, String>,
    pub types: fn() -> Vec<String>,
}

const fn register<P: Product>() -> Registered {
    Registered {
        key: P::KEY,
        load: load_json::<P>,
        types: types::<P>,
    }
}

fn load_json<P: Product>(
    source: &Source,
    lease_id: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let params: P::Params = serde_json::from_value(params)
        .map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    let data = P::load(source, &params, context, cancelled).map_err(|error| error.to_string())?;
    bounded_json(
        &Envelope {
            source_id: lease_id,
            revision: &source.key().revision,
            product: P::KEY,
            data,
        },
        cancelled,
    )
}

fn types<P: Product>() -> Vec<String> {
    P::types()
}

/// Every product the service answers. Add a product here and in the frontend
/// widget registry; nothing else needs to change.
pub static PRODUCTS: &[Registered] = &[
    register::<national_focus::NationalFocus>(),
    register::<research::Research>(),
    register::<country_flags::CountryFlags>(),
    register::<land_owners::LandOwnersProduct>(),
    register::<country_styles::CountryStyles>(),
    register::<country_groups::CountryGroups>(),
    register::<country_stability::CountryStability>(),
    register::<country_measure::CountryMeasure>(),
    register::<idea_history::IdeaHistory>(),
];

pub fn find(key: &str) -> Option<&'static Registered> {
    PRODUCTS.iter().find(|product| product.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_keys_are_unique_and_exported_types_are_named_once() {
        let mut keys: Vec<_> = PRODUCTS.iter().map(|product| product.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRODUCTS.len());
        let contract = crate::typescript_contract();
        let names: Vec<_> = contract
            .lines()
            .filter_map(|line| line.strip_prefix("export type "))
            .map(|rest| rest.split_whitespace().next().unwrap_or_default())
            .collect();
        for name in [
            "CountryParams",
            "CountryFlagSize",
            "CountryFlagImage",
            "LandOwners",
            "FocusReply",
            "ResearchReply",
            "CountryStyleReply",
            "CountryGroupReply",
            "CountryStabilityReply",
            "FixedRun",
            "MeasureReply",
            "IdeaHistoryReply",
        ] {
            assert_eq!(
                names.iter().filter(|n| **n == name).count(),
                1,
                "{name} must be declared exactly once"
            );
        }
    }
}
