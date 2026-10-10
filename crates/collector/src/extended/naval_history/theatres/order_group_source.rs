//! Exact same-pass OOB operands loaned to optional child-front capture.

use super::{NAVAL_PACK_N, NavalJob, OOB_MASK};
use crate::raw::{RawLog, Recorded};
use engine::{Memory, corders_group};

pub(crate) struct OrderGroupSource<'a> {
    log: &'a RawLog<NAVAL_PACK_N>,
    recorded: Recorded<'a>,
}

impl<'a> OrderGroupSource<'a> {
    pub(crate) fn new(job: &'a NavalJob) -> Self {
        Self {
            log: &job.log,
            recorded: Recorded::build(&job.log),
        }
    }
}

impl Memory for OrderGroupSource<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        match destination.len() {
            0 => true,
            1 => {
                let Some(anchor_address) = address
                    .checked_sub(corders_group::IS_ARMY_GROUP - corders_group::MOTORIZATION_LEVEL)
                else {
                    return false;
                };
                let Some((anchor, Some(flag))) =
                    self.recorded.unique_observation(anchor_address, 1)
                else {
                    return false;
                };
                // copy_oob_root_spans emits the own flag then its anchor,
                // including the failed field before continuing its suffix.
                // Parent flag copies never emit this unique anchor. Its
                // own success is irrelevant: its position selects the flag.
                if anchor.mask != OOB_MASK
                    || anchor.bulk
                    || flag.addr != address
                    || flag.len != 1
                    || flag.mask != OOB_MASK
                    || flag.bulk
                    || !flag.ok
                {
                    return false;
                }
                let Some(&byte) = self.log.bytes.get(flag.start as usize) else {
                    return false;
                };
                destination[0] = byte;
                true
            }
            16 => {
                // nspan logs each root's full child header exactly once.
                // Refuse missing/repeated headers and retain exact failures;
                // neither a bulk span nor a newly requested 4B read may heal it.
                let Some((header, _)) = self.recorded.unique_observation(address, 16) else {
                    return false;
                };
                header.mask == OOB_MASK && !header.bulk && self.recorded.copy(address, destination)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
