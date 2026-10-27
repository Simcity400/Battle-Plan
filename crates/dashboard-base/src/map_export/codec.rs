use super::*;
use replay_data::codec::Decoder;
use sha2::{Digest, Sha256};
use std::io::Read;

const MAGIC: &[u8; 8] = b"H4MAPE02";
pub(super) const HEADER: usize = 135;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub dimensions: [u32; 2],
    pub provinces: usize,
    pub factors: usize,
}
impl Limits {
    pub fn new(cells: &SnowCells, winter: &Winter) -> Self {
        Self {
            dimensions: cells.dimensions(),
            provinces: cells.provinces(),
            factors: winter.records.len(),
        }
    }
    fn raster(self) -> io::Result<usize> {
        let [w, h] = self.dimensions;
        if w == 0 || h == 0 {
            return Err(invalid("empty snow geometry"));
        }
        (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("snow raster size overflow"))
    }
    pub fn maximum(self) -> io::Result<usize> {
        self.raster()?
            .checked_add(
                self.provinces
                    .checked_mul(9)
                    .ok_or_else(|| invalid("snow province size overflow"))?,
            )
            .and_then(|n| n.checked_add(self.factors.checked_mul(5)?))
            .and_then(|n| n.checked_add(91))
            .ok_or_else(|| invalid("snow product size overflow"))
    }
    pub(super) fn decode_peak(self, encoded: usize) -> io::Result<usize> {
        use std::alloc::Layout;
        let overflow = || invalid("snow decode peak size overflow");
        let factors = Layout::array::<Option<f32>>(self.factors).map_err(|_| overflow())?;
        let factor_arc = Layout::new::<[usize; 2]>()
            .extend(factors)
            .map_err(|_| overflow())?
            .0
            .pad_to_align()
            .size();
        let snow_arc = Layout::new::<[usize; 2]>()
            .extend(Layout::new::<SnowMud>())
            .map_err(|_| overflow())?
            .0
            .pad_to_align()
            .size();
        encoded
            .checked_add(SnowMud::decode_storage_bytes(
                self.raster()?,
                self.provinces,
            )?)
            .and_then(|n| n.checked_add(factors.size()))
            .and_then(|n| n.checked_add(factor_arc))
            .and_then(|n| n.checked_add(snow_arc))
            .ok_or_else(overflow)
    }
}

pub(super) struct Header {
    pub key: [u8; 32],
    pub branch: BranchId,
    pub hour: Hour,
    pub kind: u8,
    pub layer: u16,
    pub raw: usize,
    pub compressed: usize,
    pub digest: [u8; 32],
    raw_body: bool,
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn usize64(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid("map export length exceeds address space"))
}
fn put64(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u64).to_le_bytes());
}

pub(super) fn read_exact_range(mut file: store::Range, length: usize) -> io::Result<Vec<u8>> {
    if file.len() != length as u64 {
        return Err(invalid("map export range length differs"));
    }
    let mut bytes = engine_allocation::vec(length)?;
    bytes.resize(length, 0);
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn header(range: &store::Range) -> io::Result<Header> {
    let mut range = range.clone();
    let mut bytes = [0; HEADER];
    range.read_exact(&mut bytes)?;
    if &bytes[..8] != MAGIC || digest(&bytes[..HEADER - 32]) != bytes[HEADER - 32..] {
        return Err(invalid("map export header checksum or contract mismatch"));
    }
    let mut d = Decoder::new(&bytes[8..HEADER - 32]);
    let key = d.take(32)?.try_into().unwrap();
    let branch = d.u64()?;
    let hour = d.i32()?;
    let tag = d.u8()?;
    let kind = tag & 0x7f;
    let layer = d.u16()?;
    let raw = usize64(d.u64()?)?;
    let compressed = usize64(d.u64()?)?;
    let digest = d.take(32)?.try_into().unwrap();
    d.finish()?;
    if range.len()
        != (HEADER as u64)
            .checked_add(compressed as u64)
            .ok_or_else(|| invalid("map export record size overflow"))?
    {
        return Err(invalid("map export stored length differs from range"));
    }
    if !matches!(kind, 0 | 3 | 4) || (kind == 0 && layer != 0) {
        return Err(invalid("map export record has unknown kind or layer"));
    }
    Ok(Header {
        key,
        branch,
        hour,
        kind,
        layer,
        raw,
        compressed,
        digest,
        raw_body: tag & 0x80 != 0,
    })
}

pub(super) fn pack_for(
    key: [u8; 32],
    branch: BranchId,
    hour: Hour,
    kind: u8,
    layer: u16,
    bytes: &[u8],
) -> io::Result<Vec<u8>> {
    if !matches!(kind, 0 | 3 | 4) || (kind == 0 && layer != 0) {
        return Err(invalid("map export record has unknown kind or layer"));
    }
    let compressed = zstd::bulk::compress(bytes, 3)?;
    let raw = compressed.len() >= bytes.len();
    let body = if raw { bytes } else { &compressed };
    let mut out = engine_allocation::vec(
        HEADER
            .checked_add(body.len())
            .ok_or_else(|| invalid("map export envelope length overflow"))?,
    )?;
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&key);
    out.extend_from_slice(&branch.to_le_bytes());
    out.extend_from_slice(&hour.to_le_bytes());
    out.push(kind | if raw { 0x80 } else { 0 });
    out.extend_from_slice(&layer.to_le_bytes());
    put64(&mut out, bytes.len());
    put64(&mut out, body.len());
    out.extend_from_slice(&digest(bytes));
    out.extend_from_slice(&digest(&out));
    out.extend_from_slice(body);
    Ok(out)
}

pub(super) fn unpack_for(
    range: store::Range,
    key: [u8; 32],
    target: Option<(BranchId, Hour)>,
    kind: u8,
    layer: u16,
    maximum: usize,
    buffer_peak: &mut usize,
) -> io::Result<Vec<u8>> {
    unpack_for_admitted(
        range,
        key,
        target,
        kind,
        layer,
        maximum,
        usize::MAX,
        0,
        buffer_peak,
    )
}

pub(super) fn unpack_for_admitted(
    mut range: store::Range,
    key: [u8; 32],
    target: Option<(BranchId, Hour)>,
    kind: u8,
    layer: u16,
    maximum: usize,
    headroom: usize,
    resident: usize,
    buffer_peak: &mut usize,
) -> io::Result<Vec<u8>> {
    let h = header(&range)?;
    if h.key != key
        || h.kind != kind
        || h.layer != layer
        || target.is_some_and(|t| t != (h.branch, h.hour))
    {
        return Err(invalid(
            "map export record has wrong provenance/kind/layer/key",
        ));
    }
    if h.raw > maximum || h.compressed > zstd::zstd_safe::compress_bound(h.raw) {
        return Err(invalid("map export record exceeds admitted raw length"));
    }
    let mut decoder = if h.raw_body {
        None
    } else {
        Some(zstd::zstd_safe::DCtx::try_create().ok_or_else(engine_allocation::oom)?)
    };
    let workspace = decoder.as_ref().map_or(0, |decoder| decoder.sizeof());
    let buffers = if h.raw_body {
        h.compressed
    } else {
        h.raw
            .checked_add(h.compressed)
            .ok_or_else(|| invalid("map export decode peak overflow"))?
    };
    admit_peak(
        resident,
        buffers
            .checked_add(workspace)
            .ok_or_else(|| invalid("map export decode peak overflow"))?,
        headroom,
        buffer_peak,
    )?;
    let mut prefix = [0; HEADER];
    range.read_exact(&mut prefix)?;
    let mut compressed = engine_allocation::vec(h.compressed)?;
    compressed.resize(h.compressed, 0);
    *buffer_peak = (*buffer_peak).max(compressed.capacity());
    range.read_exact(&mut compressed)?;
    let out = if h.raw_body {
        if h.raw != h.compressed {
            return Err(invalid("map export raw length differs"));
        }
        compressed
    } else {
        if zstd::zstd_safe::find_frame_compressed_size(&compressed).ok() != Some(compressed.len()) {
            return Err(invalid("map export trailing or invalid compressed bytes"));
        }
        if zstd::zstd_safe::get_frame_content_size(&compressed)
            .ok()
            .flatten()
            .is_some_and(|n| n != h.raw as u64)
        {
            return Err(invalid("map export frame decoded size differs"));
        }
        let mut out = engine_allocation::vec(h.raw)?;
        out.resize(h.raw, 0);
        *buffer_peak = (*buffer_peak).max(compressed.capacity().saturating_add(out.capacity()));
        let decoder = decoder
            .as_mut()
            .ok_or_else(|| invalid("map export decompressor is missing"))?;
        let log = (usize::BITS - h.raw.max(1).saturating_sub(1).leading_zeros()).max(10);
        decoder
            .set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(log))
            .map_err(|code| invalid(zstd::zstd_safe::get_error_name(code)))?;
        if decoder
            .decompress(&mut out[..], &compressed)
            .map_err(|code| {
                invalid(format!(
                    "map export compressed body is invalid: {}",
                    zstd::zstd_safe::get_error_name(code)
                ))
            })?
            != h.raw
        {
            return Err(invalid("map export decoded size differs"));
        }
        out
    };
    if digest(&out) != h.digest {
        return Err(invalid("map export body checksum differs"));
    }
    Ok(out)
}

pub(super) fn admit_peak(
    resident: usize,
    allocation: usize,
    maximum: usize,
    peak: &mut usize,
) -> io::Result<()> {
    let total = resident
        .checked_add(allocation)
        .ok_or_else(|| invalid("map export decode peak overflow"))?;
    if total > maximum {
        return Err(invalid(
            "map export decode peak not admitted by live headroom",
        ));
    }
    *peak = (*peak).max(total);
    Ok(())
}

#[cfg(test)]
pub(super) fn pack(
    key: &Key,
    branch: BranchId,
    hour: Hour,
    kind: u8,
    bytes: &[u8],
) -> io::Result<Vec<u8>> {
    pack_for(
        if kind == 3 {
            key.layer_digest(SNOW)?
        } else {
            key.digest()
        },
        branch,
        hour,
        kind,
        if kind == 3 { SNOW } else { 0 },
        bytes,
    )
}
pub(super) fn unpack(
    range: store::Range,
    key: &Key,
    target: Option<(BranchId, Hour)>,
    kind: u8,
    maximum: usize,
) -> io::Result<Vec<u8>> {
    unpack_for(
        range,
        if kind == 3 {
            key.layer_digest(SNOW)?
        } else {
            key.digest()
        },
        target,
        kind,
        if kind == 3 { SNOW } else { 0 },
        maximum,
        &mut 0,
    )
}

pub const STATUS_REFUSED: &str = "snow export: diagnostic exceeds runtime geometry admission";

pub(super) fn reason_length(reason: &str) -> io::Result<usize> {
    reason
        .len()
        .checked_add(8)
        .ok_or_else(|| invalid("snow status length overflow"))
}
pub(super) fn status_body(
    kind: u8,
    reason: &str,
    limits: Option<Limits>,
) -> io::Result<(u8, Vec<u8>)> {
    if let Some(limits) = limits {
        let maximum = limits.maximum()?;
        if reason_length(reason)? > maximum {
            if reason_length(STATUS_REFUSED)? > maximum {
                return Err(invalid("snow geometry cannot admit named status refusal"));
            }
            return Ok((2, encode_reason(STATUS_REFUSED)));
        }
    }
    Ok((kind, encode_reason(reason)))
}

pub(super) fn encode_reason(reason: &str) -> Vec<u8> {
    let mut out = Vec::new();
    put64(&mut out, reason.len());
    out.extend_from_slice(reason.as_bytes());
    out
}
pub(super) fn decode_reason(bytes: &[u8]) -> io::Result<String> {
    let mut d = Decoder::new(bytes);
    let n = usize64(d.u64()?)?;
    let text = std::str::from_utf8(d.take(n)?)
        .map_err(|_| invalid("snow status is not UTF-8"))?
        .to_owned();
    d.finish()?;
    Ok(text)
}

pub(super) fn encode_product(snow: &SnowMud, limits: Limits) -> io::Result<Vec<u8>> {
    validate_product(snow, limits)?;
    let bytes = product_codec::encode(snow)?;
    if bytes.len() > limits.maximum()? {
        return Err(invalid("snow encoded product exceeds geometry"));
    }
    Ok(bytes)
}

fn validate_product(snow: &SnowMud, limits: Limits) -> io::Result<()> {
    if [snow.width, snow.height] != limits.dimensions
        || snow.texels.len() != limits.raster()?
        || snow.province_snow.len() != limits.provinces
        || snow
            .post_factors
            .as_ref()
            .is_some_and(|v| v.len() != limits.factors)
    {
        return Err(invalid("snow product differs from runtime geometry"));
    }
    Ok(())
}

fn admit_product_lengths(bytes: &[u8], limits: Limits) -> io::Result<()> {
    use product_codec::Product;
    let mut input = Decoder::new(bytes);
    if [input.u32()?, input.u32()?] != limits.dimensions {
        return Err(invalid("snow decoded dimensions differ"));
    }
    let count = |input: &mut Decoder<'_>, expected| -> io::Result<usize> {
        let count = input.count64(1)?;
        if count != expected {
            return Err(invalid(
                "snow typed length differs from admitted geometry or remaining bytes",
            ));
        }
        Ok(count)
    };
    let raster = count(&mut input, limits.raster()?)?;
    input.take(raster)?;
    for _ in 0..count(&mut input, limits.provinces)? {
        Option::<i64>::decode(&mut input)?;
    }
    if input.bool()? {
        count(&mut input, limits.factors)?;
    }
    Ok(())
}

pub(super) fn decode_product(bytes: &[u8], limits: Limits) -> io::Result<SnowMud> {
    if bytes.len() > limits.maximum()? {
        return Err(invalid("snow decoded product exceeds geometry"));
    }
    // Admit the three geometry-sized containers before the generated decoder allocates them.
    admit_product_lengths(bytes, limits)?;
    #[cfg(test)]
    PRODUCT_DECODES.with(|decodes| decodes.set(decodes.get() + 1));
    let snow = product_codec::decode(bytes)?;
    validate_product(&snow, limits)?;
    Ok(snow)
}

#[cfg(test)]
thread_local! {
    pub(super) static PRODUCT_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
