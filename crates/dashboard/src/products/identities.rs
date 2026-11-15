//! `country-identities`: every tag's holders over a timeline, from the one identity table
//! (`TagHolders`) by the one rule ([`dashboard_query::identity`]). The dashboard's country index
//! is built from it; every per-country product reads the same holders.

use super::{BranchParams, LoadContext, Product};
use crate::Source;
use dashboard_query::{identity::Holder, series::MissingSpan};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct CountryIdentities;

/// A stretch of hours one country held a tag.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentityRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

/// One tag's holders, and the hours its countries were there but no link resolved them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TagIdentities {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub unresolved: Vec<IdentitySpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentitiesReply {
    pub branch_id: String,
    /// By tag.
    pub countries: Vec<TagIdentities>,
    /// Hours the identity table was not available: no tag's holder is known there.
    pub unknown: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryIdentitiesReply);

impl Product for CountryIdentities {
    const KEY: &'static str = "country-identities";
    const CONTENT_DEPENDENT: bool = false;
    type Params = BranchParams;
    type Reply = CountryIdentitiesReply;

    fn types() -> Vec<String> {
        vec![CountryIdentityRun::decl(), IdentitySpan::decl(), TagIdentities::decl(), MissingSpan::decl(), CountryIdentitiesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
        let query = crate::answer::query(source, branch)?;
        Ok(reply(&params.branch_id, &query.holders))
    }
}

/// The reply of `holders`.
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
