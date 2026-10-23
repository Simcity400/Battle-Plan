use super::*;
use std::time::Instant;

#[derive(Debug)]
pub enum Restore {
    Hit(Arc<SnowMud>),
    Miss(String),
}

#[derive(Clone)]
pub struct Reader {
    store: Arc<Store>,
    key: Key,
    limits: Limits,
}
impl Reader {
    pub fn new(store: Arc<Store>, key: Key, limits: Limits) -> Self {
        Self { store, key, limits }
    }
    pub fn coverage(&self, branch: BranchId) -> io::Result<Option<Coverage>> {
        let Some(source) = self.store.saved_key()? else {
            return Ok(None);
        };
        if !Session::ready(&self.store, &source, &self.key)? {
            return Ok(None);
        }
        let section = self.key.records(branch);
        let endpoints = self.store.range_count(&source, &section)?.unwrap_or(0);
        if endpoints == 0 {
            return Ok(None);
        }
        let read = |index| -> io::Result<Hour> {
            let range = self
                .store
                .read(&source, &section, index)?
                .ok_or_else(|| invalid("snow coverage has a dangling endpoint"))?;
            let header = codec::header(&range)?;
            if header.key != self.key.digest() || header.branch != branch {
                return Err(invalid("snow coverage has wrong provenance"));
            }
            Ok(header.hour)
        };
        Ok(Some(Coverage {
            endpoints,
            first: read(0)?,
            last: read(endpoints - 1)?,
        }))
    }
    fn lookup(
        &self,
        branch: BranchId,
        hour: Hour,
        buffer_peak: &mut usize,
    ) -> io::Result<Option<Arc<SnowMud>>> {
        let Some(source) = self.store.saved_key()? else {
            return Ok(None);
        };
        if !Session::ready(&self.store, &source, &self.key)? {
            return Ok(None);
        }
        let section = self.key.records(branch);
        let mut low = 0;
        let mut high = self.store.range_count(&source, &section)?.unwrap_or(0);
        while low < high {
            let mid = low + (high - low) / 2;
            let file = self
                .store
                .read(&source, &section, mid)?
                .ok_or_else(|| invalid("snow endpoint index is dangling"))?;
            let header = codec::header(&file)?;
            if header.key != self.key.digest() || header.branch != branch {
                return Err(invalid("snow index has wrong provenance/key"));
            }
            if header.hour < hour {
                low = mid + 1;
            } else if header.hour > hour {
                high = mid;
            } else {
                if header.kind != 0 {
                    if !matches!(header.kind, 1 | 2) {
                        return Err(invalid("snow endpoint has invalid status"));
                    }
                    let body = codec::unpack_measured(
                        file,
                        &self.key,
                        Some((branch, hour)),
                        header.kind,
                        self.limits.maximum()?,
                        buffer_peak,
                    )?;
                    let reason = codec::decode_reason(&body)?;
                    return Err(invalid(format!(
                        "snow {}: {reason}",
                        if header.kind == 1 {
                            "unavailable"
                        } else {
                            "unsupported"
                        }
                    )));
                }
                let body = codec::unpack_measured(
                    file,
                    &self.key,
                    Some((branch, hour)),
                    0,
                    40,
                    buffer_peak,
                )?;
                if body.len() != 40 {
                    return Err(invalid("snow manifest has invalid typed length"));
                }
                let ordinal = u64::from_le_bytes(body[..8].try_into().unwrap());
                let payload = self
                    .store
                    .read(&source, &self.key.section("payloads"), ordinal)?
                    .ok_or_else(|| invalid("snow payload reference is dangling"))?;
                let mut payload_peak = 0;
                let bytes = codec::unpack_measured(
                    payload,
                    &self.key,
                    None,
                    3,
                    self.limits.maximum()?,
                    &mut payload_peak,
                );
                *buffer_peak = (*buffer_peak).max(body.capacity() + payload_peak);
                let bytes = bytes?;
                if codec::digest(&bytes) != body[8..] {
                    return Err(invalid("snow manifest payload digest differs"));
                }
                let snow = Arc::new(codec::decode_product(&bytes, self.limits)?);
                *buffer_peak = (*buffer_peak).max(
                    body.capacity()
                        + bytes.capacity()
                        + snow.owned_bytes()
                        + 2 * std::mem::size_of::<usize>(),
                );
                return Ok(Some(snow));
            }
        }
        Ok(None)
    }
    fn restore_measured(&self, branch: BranchId, hour: Hour) -> (Restore, usize) {
        let mut buffer_peak = 0;
        let result = match self.lookup(branch, hour, &mut buffer_peak) {
            Ok(Some(snow)) => Restore::Hit(snow),
            Ok(None) => Restore::Miss("missing or stale endpoint".into()),
            Err(error) => Restore::Miss(error.to_string()),
        };
        (result, buffer_peak)
    }
    pub fn restore(&self, branch: BranchId, hour: Hour) -> Restore {
        self.restore_measured(branch, hour).0
    }
    pub fn build_or_restore(
        &self,
        branch: BranchId,
        hour: Hour,
        live: impl FnOnce() -> io::Result<Arc<SnowMud>>,
    ) -> io::Result<Arc<SnowMud>> {
        let start = Instant::now();
        let (result, buffer_peak) = self.restore_measured(branch, hour);
        match result {
            Restore::Hit(snow) => {
                println!(
                    "PROFILE map_export_snow branch={branch} hour={hour} result=hit restore_us={} product_bytes={} restore_tracked_peak_bytes={buffer_peak}",
                    start.elapsed().as_micros(),
                    snow.owned_bytes() + 2 * std::mem::size_of::<usize>()
                );
                Ok(snow)
            }
            Restore::Miss(reason) => {
                println!(
                    "PROFILE map_export_snow branch={branch} hour={hour} result=miss restore_us={} restore_tracked_peak_bytes={buffer_peak} reason={reason:?}",
                    start.elapsed().as_micros()
                );
                live()
            }
        }
    }
}
