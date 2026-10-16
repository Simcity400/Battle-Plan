//! Each branch's published coverage page: the accepted extent every prepared
//! query reads before its own pages. The pipeline writes the page, with the
//! families' refusals beside the extent.

use crate::{Source, metadata::check_cancel, products::ContentOptions};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};

/// The prepared files' layout version: a checkpoint of another version is
/// prepared again, and coverage of another version is refused.
pub const SCHEMA: u32 = 22;

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
