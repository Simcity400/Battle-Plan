use super::{consumer, invalid};
use core_spool::Allowances;
use std::io;

pub const REASONS: u64 = 65_536;
pub fn consumer_allowances(total: u64) -> io::Result<consumer::Allowances> {
    let metadata = consumer::Consumer::metadata_bytes()?;
    let shared = total
        .checked_sub(REASONS + metadata)
        .ok_or_else(|| invalid("consumer allowance cannot hold reasons and metadata"))?;
    let unit = shared / 32;
    Ok(consumer::Allowances {
        values: unit * 16,
        identities: unit * 8,
        index: unit,
        remap: unit * 2,
        reason_remap: unit,
        payloads: shared - unit * 28,
        reasons: REASONS,
        metadata,
    })
}
pub fn writer_allowances(pool: u64, readers: u64) -> io::Result<Allowances> {
    Ok(Allowances {
        dictionary: (pool / readers)
            .checked_sub(REASONS)
            .ok_or_else(|| invalid("writer dictionary share cannot hold reasons"))?,
        reasons: REASONS,
    })
}
