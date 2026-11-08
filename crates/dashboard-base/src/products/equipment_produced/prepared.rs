//! Equipment produced from production's prepared history: the reply a scan of the
//! branch gives, through the prefix preparation has published. It reads prepared data
//! alone: before production or the country identities publish the branch, the reply is
//! pending (null), never a scan of the recording.

use super::{EquipmentProduced, MAX_REPLY_BYTES, ProducedParams, answer};
use crate::products::{
    ContentOptions, LoadContext, Product,
    built::{Built, Data},
    country_history,
};
use crate::{Source, prepared::prefix::Prefix};
use serde_json::Value;
use std::io;
use std::sync::atomic::AtomicBool;

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Data> {
    let params: ProducedParams = serde_json::from_value(params)?;
    EquipmentProduced::validate(&params).map_err(crate::invalid)?;
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid production branch"))?;
    let coverage = crate::prepared::prefix::read_or_empty(
        source,
        content,
        EquipmentProduced::KEY,
        branch,
        None,
        cancelled,
    )?
    .coverage;
    let prefix = Prefix::new(source, branch, coverage, None);
    let (store, key) = crate::prepared::files(source, content)?;
    let Some(base) = crate::prepared_production::history::base(source, &store, &key, branch, cancelled)?
    else {
        return Ok(Data::Tree(Value::Null));
    };
    let prepared = History {
        base: &base,
        ranges: crate::prepared_production::history::stored(&store, &key, branch),
        cancelled,
    };
    // Chosen countries take their recorded aliases from the prepared identities.
    let history = if params.top.is_none() && !params.tags.is_empty() {
        match country_history::prepared_branch_histories(source, branch, content, cancelled)? {
            Some(history) => Some(history),
            None => return Ok(Data::Tree(Value::Null)),
        }
    } else {
        None
    };
    let context = LoadContext {
        content: content.cloned(),
    };
    let reply = answer(
        source,
        &params,
        &context,
        &base.scanned,
        cancelled,
        || history.ok_or_else(|| io::Error::other("Chosen countries' identities were not read")),
        Some(&prepared),
    )?;
    let mut data = Built::within(&reply, cancelled, MAX_REPLY_BYTES, || {
        io::Error::other("These countries' production history exceeds one exact reply")
    })?;
    prefix.mark_revisable(source, branch, &mut data)?;
    Ok(Data::Built(data))
}

/// A branch's prepared history, as a reply reads it beyond its light scan.
struct History<'a> {
    base: &'a crate::prepared_production::history::Base,
    ranges: crate::prepared_production::history::Stored<'a>,
    cancelled: &'a AtomicBool,
}

impl super::Prepared for History<'_> {
    fn counted(
        &self,
        plan: &crate::prepared_production::history::CountPlan<'_>,
    ) -> io::Result<(
        std::collections::BTreeMap<String, crate::prepared_production::history::Counted>,
        Vec<String>,
    )> {
        self.base.counted(&self.ranges, plan, self.cancelled)
    }

    fn light(&self) -> io::Result<super::Histories> {
        self.base.light(&self.ranges, self.cancelled)
    }

    fn whole(&self, tags: &[String], light: Option<&super::Histories>) -> io::Result<super::Histories> {
        self.base.whole(&self.ranges, tags, light, self.cancelled)
    }
}
