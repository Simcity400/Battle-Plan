use super::{NAVAL_PACK_N, NavalJob, OOB_MASK};
use collector_raw::raw::{RawLog, Recorded};
use engine_native_memory::Memory;
use engine_native::corders_group;

pub struct OrderGroupSource<'a> {
    log: &'a RawLog<NAVAL_PACK_N>,
    recorded: Recorded<'a>,
}

impl<'a> OrderGroupSource<'a> {
    pub fn new(job: &'a NavalJob) -> Result<Self, &'static str> {
        Ok(Self {
            log: &job.log,
            recorded: Recorded::build(&job.log)?,
        })
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
