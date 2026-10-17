//! Each branch's published coverage page: the accepted extent every prepared
//! query reads before its own pages. The pipeline writes the page, with the
//! families' refusals beside the extent.
//!
//! A family prepared on its own publishes its own coverage and refusals,
//! which only the pipeline's registry names. The dashboard installs its
//! [`Gates`] once, and readers here ask them through [`product_coverage`] and
//! [`refusal`]; with none installed every family is prepared hour by hour,
//! so a product waits for the branch's coverage and no family refuses it.

use crate::{Source, metadata::check_cancel, products::ContentOptions};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::{OnceLock, atomic::AtomicBool},
};

/// What the pipeline answers for the families prepared on their own.
pub trait Gates: Sync {
    /// The coverage `product`'s queries wait for on `branch`: that of the
    /// family prepared on its own gating the product, or the branch's.
    fn product_coverage(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        product: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Coverage>>;
    /// The refusal `query` answers with on `branch`, if a family it reads
    /// refused the branch.
    fn refusal(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        query: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<()>;
}

static GATES: OnceLock<&'static dyn Gates> = OnceLock::new();

/// Installs the pipeline's gates; later installs keep the first.
pub fn install_gates(gates: &'static dyn Gates) {
    let _ = GATES.set(gates);
}

/// The coverage `product`'s queries wait for on `branch` ([`Gates`]).
pub fn product_coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    product: &str,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    match GATES.get() {
        Some(gates) => gates.product_coverage(source, content, branch, product, cancelled),
        None => coverage(source, content, branch, cancelled),
    }
}

/// `query`'s refusal on `branch` ([`Gates`]).
pub fn refusal(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    query: &str,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    match GATES.get() {
        Some(gates) => gates.refusal(source, content, branch, query, cancelled),
        None => check_cancel(cancelled),
    }
}

/// The prepared files' version, raised when their layout or the values
/// preparation publishes change: a checkpoint of another version is
/// prepared again, and coverage of another version is refused.
pub const SCHEMA: u32 = 25;

/// Accepted extent, distinct from a complete source's timeline and recorded gaps.
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedCoverage")]
pub struct Coverage {
    pub schema: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub complete: bool,
}

pub fn coverage_section(branch: u64) -> String {
    format!("preparation-coverage-{branch}")
}

/// The extent a published coverage page carries; the refusals beside it are
/// read by the pipeline.
#[derive(Deserialize)]
struct PublishedCoverage {
    #[serde(flatten)]
    coverage: Coverage,
}

/// Publishes accepted coverage for family tests that write their own prepared files.
#[cfg(any(test, feature = "test-support"))]
pub fn publish_coverage(
    writer: &mut store::Writer,
    branch: u64,
    first_hour: i32,
    last_hour: i32,
    complete: bool,
) -> io::Result<()> {
    let page = Coverage {
        schema: SCHEMA,
        first_hour,
        last_hour,
        complete,
    };
    writer.write_at(
        &coverage_section(branch),
        0,
        serde_json::to_vec(&page)?.as_slice(),
    )
}

pub fn coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    check_cancel(cancelled)?;
    let (store, key) = super::files(source, content)?;
    let Some(file) = store.read(&key, &coverage_section(branch), 0)? else {
        return Ok(None);
    };
    let range = serde_json::from_reader::<_, PublishedCoverage>(io::BufReader::new(file))?.coverage;
    if range.schema != SCHEMA || range.first_hour > range.last_hour {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported prepared coverage schema",
        ));
    }
    Ok(Some(range))
}
