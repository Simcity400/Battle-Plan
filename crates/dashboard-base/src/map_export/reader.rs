use super::*;
use std::time::Instant;

#[derive(Debug)]
pub enum Restore {
    Hit(Arc<SnowMud>),
    Miss(String),
}

#[derive(Clone, Copy)]
pub struct Headroom {
    pub available: Option<(u64, u64)>,
    pub low: bool,
}
impl Headroom {
    fn maximum(self) -> io::Result<usize> {
        if self.low {
            return Err(invalid(
                "map export restore not admitted: OS memory pressure",
            ));
        }
        let (commit, physical) = self
            .available
            .ok_or_else(|| invalid("map export live memory headroom is unknown"))?;
        Ok(usize::try_from(commit.min(physical)).unwrap_or(usize::MAX))
    }
    fn unbounded() -> Self {
        Self {
            available: Some((u64::MAX, u64::MAX)),
            low: false,
        }
    }
}

#[derive(Debug)]
pub enum LayerProduct {
    Available(Vec<u8>),
    Unavailable(String),
    Unsupported(String),
    PartError { kind: String, display: String },
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
    pub fn bind(
        store: Arc<Store>,
        key: Key,
        limits: Limits,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
        let source = store
            .saved_key()?
            .ok_or_else(|| invalid("map export binding is missing"))?;
        if !Session::ready(&store, &source, &key)? {
            return Err(invalid("map export binding is stale"));
        }
        witness::verify(
            &key,
            |ordinal| store.read(&source, &key.section("witness"), ordinal),
            &mut check,
        )?;
        check()?;
        Ok(Self::new(store, key, limits))
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
                .ok_or_else(|| invalid("map export coverage has a dangling endpoint"))?;
            let header = codec::header(&range)?;
            self.validate_manifest_header(&header, branch)?;
            Ok(header.hour)
        };
        Ok(Some(Coverage {
            endpoints,
            first: read(0)?,
            last: read(endpoints - 1)?,
        }))
    }
    fn validate_manifest_header(&self, header: &codec::Header, branch: BranchId) -> io::Result<()> {
        if header.key != self.key.digest()
            || header.branch != branch
            || header.kind != 0
            || header.layer != 0
        {
            return Err(invalid(
                "map export index has wrong provenance/kind/layer/key",
            ));
        }
        Ok(())
    }
    fn manifest_in(
        &self,
        source: &SourceKey,
        branch: BranchId,
        hour: Hour,
        maximum: usize,
        peak: &mut usize,
    ) -> io::Result<Option<Manifest>> {
        self.manifest_bytes_in(source, branch, hour, maximum, peak)?
            .map(|body| {
                let manifest: Manifest = product_codec::decode(&body)?;
                records::validate_manifest(&self.key, &manifest)?;
                Ok(manifest)
            })
            .transpose()
    }
    fn manifest_bytes_in(
        &self,
        source: &SourceKey,
        branch: BranchId,
        hour: Hour,
        maximum: usize,
        peak: &mut usize,
    ) -> io::Result<Option<Vec<u8>>> {
        if !Session::ready(&self.store, source, &self.key)? {
            return Ok(None);
        }
        let section = self.key.records(branch);
        let mut low = 0;
        let mut high = self.store.range_count(source, &section)?.unwrap_or(0);
        while low < high {
            let mid = low + (high - low) / 2;
            let range = self
                .store
                .read(source, &section, mid)?
                .ok_or_else(|| invalid("map export endpoint index is dangling"))?;
            let header = codec::header(&range)?;
            self.validate_manifest_header(&header, branch)?;
            if header.hour < hour {
                low = mid + 1;
            } else if header.hour > hour {
                high = mid;
            } else {
                admit(&header, maximum)?;
                let body = codec::unpack_for_admitted(
                    range,
                    self.key.digest(),
                    Some((branch, hour)),
                    0,
                    0,
                    maximum,
                    maximum,
                    0,
                    peak,
                )?;
                return Ok(Some(body));
            }
        }
        Ok(None)
    }
    pub fn manifest(
        &self,
        branch: BranchId,
        hour: Hour,
        headroom: Headroom,
    ) -> io::Result<Option<Manifest>> {
        let maximum = headroom.maximum()?;
        let Some(source) = self.store.saved_key()? else {
            return Ok(None);
        };
        self.manifest_in(&source, branch, hour, maximum, &mut 0)
    }
    fn payload_in(
        &self,
        source: &SourceKey,
        id: u16,
        table: bool,
        payload: &Payload,
        maximum: usize,
        peak: &mut usize,
    ) -> io::Result<Vec<u8>> {
        let range = self
            .store
            .read(source, &payload_section(payload.section)?, payload.ordinal)?
            .ok_or_else(|| invalid("map export payload reference is dangling"))?;
        let header = codec::header(&range)?;
        admit(&header, maximum)?;
        read_payload(range, &self.key, id, table, payload, maximum, peak)
    }
    pub fn layer(
        &self,
        branch: BranchId,
        hour: Hour,
        layer: u16,
        headroom: Headroom,
    ) -> io::Result<Option<LayerProduct>> {
        let maximum = headroom.maximum()?;
        let Some(source) = self.store.saved_key()? else {
            return Ok(None);
        };
        let Some(manifest) = self.manifest_in(&source, branch, hour, maximum, &mut 0)? else {
            return Ok(None);
        };
        let Some(status) = manifest.layers.get(&layer) else {
            return Ok(None);
        };
        Ok(Some(match status {
            LayerStatus::Available { payload } => LayerProduct::Available(
                self.payload_in(&source, layer, false, payload, maximum, &mut 0)?,
            ),
            LayerStatus::Unavailable { reason } => LayerProduct::Unavailable(reason.clone()),
            LayerStatus::Unsupported { reason } => LayerProduct::Unsupported(reason.clone()),
            LayerStatus::PartError { kind, display } => LayerProduct::PartError {
                kind: kind.clone(),
                display: display.clone(),
            },
        }))
    }
    pub fn table(
        &self,
        branch: BranchId,
        hour: Hour,
        table: u16,
        headroom: Headroom,
    ) -> io::Result<Option<Vec<u8>>> {
        let maximum = headroom.maximum()?;
        let Some(source) = self.store.saved_key()? else {
            return Ok(None);
        };
        let Some(manifest) = self.manifest_in(&source, branch, hour, maximum, &mut 0)? else {
            return Ok(None);
        };
        manifest
            .tables
            .get(&table)
            .map(|payload| self.payload_in(&source, table, true, payload, maximum, &mut 0))
            .transpose()
    }
    fn lookup(
        &self,
        branch: BranchId,
        hour: Hour,
        headroom: Headroom,
        peak: &mut usize,
    ) -> io::Result<Option<Arc<SnowMud>>> {
        let maximum = headroom.maximum()?;
        let Some(source) = self.store.saved_key()? else {
            return Ok(None);
        };
        let Some(manifest) = self.manifest_bytes_in(&source, branch, hour, maximum, peak)? else {
            return Ok(None);
        };
        match records::snow_status(&manifest, &self.key)? {
            Some(records::SnowStatus::Available(payload)) => {
                if payload.raw > self.limits.maximum()? as u64 {
                    return Err(invalid("snow product exceeds runtime geometry admission"));
                }
                let raw = usize::try_from(payload.raw)
                    .map_err(|_| invalid("map export payload exceeds address space"))?;
                codec::admit_peak(
                    manifest.capacity(),
                    self.limits.decode_peak(raw)?,
                    maximum,
                    peak,
                )?;
                let range = self
                    .store
                    .read(&source, &payload_section(payload.section)?, payload.ordinal)?
                    .ok_or_else(|| invalid("map export payload reference is dangling"))?;
                let bytes = read_payload_admitted(
                    range,
                    &self.key,
                    SNOW,
                    false,
                    &payload,
                    maximum,
                    maximum,
                    manifest.capacity(),
                    peak,
                )?;
                codec::admit_peak(
                    manifest.capacity(),
                    self.limits.decode_peak(bytes.capacity())?,
                    maximum,
                    peak,
                )?;
                let snow = Arc::new(codec::decode_product(&bytes, self.limits)?);
                Ok(Some(snow))
            }
            Some(records::SnowStatus::Unavailable(reason)) => {
                Err(invalid(format!("snow unavailable: {reason}")))
            }
            Some(records::SnowStatus::Unsupported(reason)) => {
                Err(invalid(format!("snow unsupported: {reason}")))
            }
            Some(records::SnowStatus::PartError { kind, display }) => {
                Err(invalid(format!("snow {kind}: {display}")))
            }
            None => Ok(None),
        }
    }
    fn restore_measured(
        &self,
        branch: BranchId,
        hour: Hour,
        headroom: Headroom,
    ) -> (Restore, usize) {
        let mut peak = 0;
        let result = match self.lookup(branch, hour, headroom, &mut peak) {
            Ok(Some(snow)) => Restore::Hit(snow),
            Ok(None) => Restore::Miss("missing or stale endpoint".into()),
            Err(error) => Restore::Miss(error.to_string()),
        };
        (result, peak)
    }
    pub fn restore(&self, branch: BranchId, hour: Hour) -> Restore {
        self.restore_with_headroom(branch, hour, Headroom::unbounded())
    }
    pub fn restore_with_headroom(
        &self,
        branch: BranchId,
        hour: Hour,
        headroom: Headroom,
    ) -> Restore {
        self.restore_measured(branch, hour, headroom).0
    }
    pub fn build_or_restore(
        &self,
        branch: BranchId,
        hour: Hour,
        live: impl FnOnce() -> io::Result<Arc<SnowMud>>,
    ) -> io::Result<Arc<SnowMud>> {
        self.build_or_restore_admitted(branch, hour, Headroom::unbounded(), live)
    }
    pub fn build_or_restore_admitted(
        &self,
        branch: BranchId,
        hour: Hour,
        headroom: Headroom,
        live: impl FnOnce() -> io::Result<Arc<SnowMud>>,
    ) -> io::Result<Arc<SnowMud>> {
        let start = Instant::now();
        let (result, peak) = self.restore_measured(branch, hour, headroom);
        match result {
            Restore::Hit(snow) => {
                println!(
                    "PROFILE map_export_snow branch={branch} hour={hour} result=hit restore_us={} product_bytes={} restore_tracked_peak_bytes={peak}",
                    start.elapsed().as_micros(),
                    snow.owned_bytes() + 2 * size_of::<usize>()
                );
                Ok(snow)
            }
            Restore::Miss(reason) => {
                println!(
                    "PROFILE map_export_snow branch={branch} hour={hour} result=miss restore_us={} restore_tracked_peak_bytes={peak} reason={reason:?}",
                    start.elapsed().as_micros()
                );
                live()
            }
        }
    }
}
fn admit(header: &codec::Header, maximum: usize) -> io::Result<()> {
    if header
        .raw
        .checked_add(header.compressed)
        .is_none_or(|bytes| bytes > maximum)
    {
        return Err(invalid(
            "map export raw length not admitted by live headroom",
        ));
    }
    Ok(())
}
