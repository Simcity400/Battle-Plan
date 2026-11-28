
use std::{io, sync::atomic::AtomicBool};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::Source;

pub use crate::config::{ContentOptions, LoadContext};
pub use crate::protocol::validate_tag;
pub use dashboard_query::identity::Country;
pub use dashboard_query::fields::{Fields, HourRole};
pub use crate::air_wing_names as air_wings;
pub use crate::identity;

pub const MAX_PRODUCT_REPLY_BYTES: usize = 24 * 1024 * 1024;

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<'a, T: Serialize> {
    pub source_id: &'a str,
    pub revision: &'a str,
    pub product: &'static str,
    pub data: T,
}

#[derive(Debug)]
struct OverBudget(String);
impl std::fmt::Display for OverBudget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for OverBudget {}

pub fn over_budget(message: impl Into<String>) -> io::Error {
    io::Error::other(OverBudget(message.into()))
}

pub fn is_over_budget(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<OverBudget>())
}

pub fn check_reply_budget<T: Serialize>(reply: &T) -> io::Result<()> {
    if crate::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(over_budget(format!(
            "This product reply exceeds the transport reply budget of {MAX_PRODUCT_REPLY_BYTES} bytes"
        )));
    }
    Ok(())
}

pub trait ReplyWarnings {
    fn warnings_mut(&mut self) -> &mut Vec<String>;
}

#[macro_export]
macro_rules! fields {
    ($($name:tt $(: $role:ident)?),* $(,)?) => {
        &[$(($crate::field_name!($name), $crate::field_role!($($role)?))),*]
    };
}

#[macro_export]
macro_rules! field_name {
    ($name:ident) => { stringify!($name) };
    ($name:literal) => { $name };
}

#[macro_export]
macro_rules! field_role {
    () => { $crate::products::HourRole::Undated };
    ($role:ident) => { $crate::products::HourRole::$role };
}

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
    const CONTENT_DEPENDENT: bool;
    const INTERACTIVE: bool = false;
    const REPLAYS: bool = false;
    const PREPARE: Option<super::read_ahead::Prepare<Self::Params>> = None;
    const FIELDS: Fields;
    type Params: DeserializeOwned + 'static;
    type Reply: Serialize + ReplyWarnings + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    #[cfg(feature = "export-types")]
    fn types() -> Vec<String>
    where
        Self::Params: TS,
        Self::Reply: TS,
    {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply>;
}

pub fn validate_country(country: &Country) -> Result<(), &'static str> {
    country.tag().map_or(Ok(()), validate_tag)
}

pub fn country_in_frame<'a>(
    countries: &'a [replay_data::rows::CountryRow],
    links: &[replay_data::rows::CountryLinkRow],
    country: &Country,
) -> crate::metadata::TaggedCountry<'a> {
    match country {
        Country::Tag(tag) => crate::metadata::tagged_country(countries, links, tag),
        Country::Slot(slot) => crate::metadata::slotted_country(countries, links, *slot),
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryParams {
    pub branch_id: String,
    pub country: Country,
}

impl CountryParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&self.country)
    }

    pub fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
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

pub use crate::metadata::{RecordedRows, recorded_rows};
