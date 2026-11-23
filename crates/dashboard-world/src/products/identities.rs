
use dashboard_base::products::{BranchParams, LoadContext, Product};
use dashboard_base::Source;
use dashboard_query::{identity::Holder, series::MissingSpan};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct CountryIdentities;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentityRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdentitySpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TagIdentities {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub unresolved: Vec<IdentitySpan>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentitiesReply {
    pub branch_id: String,
    #[doc = " By tag."]
    pub countries: Vec<TagIdentities>,
    #[doc = " Hours the identity table was not available: no tag's holder is known there."]
    pub unknown: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryIdentitiesReply);

impl Product for CountryIdentities {
    const KEY: &'static str = "country-identities";
    const CONTENT_DEPENDENT: bool = false;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = CountryIdentitiesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryIdentityRun::decl(), IdentitySpan::decl(), TagIdentities::decl(), MissingSpan::decl(), CountryIdentitiesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        Ok(reply(&params.branch_id, &query.holders))
    }
}

pub fn reply(branch_id: &str, holders: &dashboard_query::identity::Holders) -> CountryIdentitiesReply {
    let countries = holders
        .tags
        .iter()
        .map(|(tag, runs)| {
            let mut identities = Vec::new();
            let mut unresolved = Vec::new();
            for run in runs {
                match run.holder {
                    Holder::Found { slot, id } => identities.push(CountryIdentityRun {
                        first_hour: run.first,
                        last_hour: run.last,
                        slot,
                        country_id: id,
                    }),
                    Holder::Unresolved => unresolved.push(IdentitySpan { first_hour: run.first, last_hour: run.last }),
                }
            }
            TagIdentities { tag: tag.clone(), identities, unresolved }
        })
        .collect();
    CountryIdentitiesReply {
        branch_id: branch_id.to_owned(),
        countries,
        unknown: holders
            .unknown
            .iter()
            .map(|(first, last, reason)| MissingSpan { first_hour: *first, last_hour: *last, reason: reason.clone() })
            .collect(),
        warnings: Vec::new(),
    }
}
