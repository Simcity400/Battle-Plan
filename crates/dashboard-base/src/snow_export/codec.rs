use super::*;
use replay_data::codec::Decoder;
use sha2::{Digest, Sha256};
use std::io::Read;

const MAGIC: &[u8; 8] = b"H4SNOW01";
const HEADER: usize = 133;

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
}

pub(super) struct Header {
    pub key: [u8; 32],
    pub branch: BranchId,
    pub hour: Hour,
    pub kind: u8,
    pub raw: usize,
    compressed: usize,
    pub digest: [u8; 32],
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn usize64(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid("snow length exceeds address space"))
}
fn put64(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u64).to_le_bytes());
}

pub(super) fn read_exact_range(mut file: store::Range, length: usize) -> io::Result<Vec<u8>> {
    if file.len() != length as u64 {
        return Err(invalid("snow range length differs"));
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
        return Err(invalid("snow header checksum or contract mismatch"));
    }
    let mut d = Decoder::new(&bytes[8..HEADER - 32]);
    let key = d.take(32)?.try_into().unwrap();
    let branch = d.u64()?;
    let hour = d.i32()?;
    let kind = d.u8()?;
    let raw = usize64(d.u64()?)?;
    let compressed = usize64(d.u64()?)?;
    let digest = d.take(32)?.try_into().unwrap();
    d.finish()?;
    if range.len()
        != (HEADER as u64)
            .checked_add(compressed as u64)
            .ok_or_else(|| invalid("snow record size overflow"))?
    {
        return Err(invalid("snow compressed length differs from range"));
    }
    Ok(Header {
        key,
        branch,
        hour,
        kind,
        raw,
        compressed,
        digest,
    })
}

pub(super) fn pack(
    key: &Key,
    branch: BranchId,
    hour: Hour,
    kind: u8,
    bytes: &[u8],
) -> io::Result<Vec<u8>> {
    let compressed = zstd::bulk::compress(bytes, 3)?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&key.digest());
    out.extend_from_slice(&branch.to_le_bytes());
    out.extend_from_slice(&hour.to_le_bytes());
    out.push(kind);
    put64(&mut out, bytes.len());
    put64(&mut out, compressed.len());
    out.extend_from_slice(&digest(bytes));
    let checksum = digest(&out);
    out.extend_from_slice(&checksum);
    out.extend_from_slice(&compressed);
    Ok(out)
}

pub(super) fn unpack(
    range: store::Range,
    key: &Key,
    target: Option<(BranchId, Hour)>,
    kind: u8,
    maximum: usize,
) -> io::Result<Vec<u8>> {
    unpack_measured(range, key, target, kind, maximum, &mut 0)
}

pub(super) fn unpack_measured(
    mut range: store::Range,
    key: &Key,
    target: Option<(BranchId, Hour)>,
    kind: u8,
    maximum: usize,
    buffer_peak: &mut usize,
) -> io::Result<Vec<u8>> {
    let header = header(&range)?;
    if header.key != key.digest()
        || header.kind != kind
        || target.is_some_and(|t| t != (header.branch, header.hour))
    {
        return Err(invalid("snow endpoint has wrong provenance/status/key"));
    }
    if header.raw > maximum || header.compressed > zstd::zstd_safe::compress_bound(maximum) {
        return Err(invalid("snow record exceeds runtime geometry admission"));
    }
    let mut prefix = [0; HEADER];
    range.read_exact(&mut prefix)?;
    let mut compressed = engine_allocation::vec(header.compressed)?;
    compressed.resize(header.compressed, 0);
    *buffer_peak = (*buffer_peak).max(compressed.capacity());
    range.read_exact(&mut compressed)?;
    if zstd::zstd_safe::find_frame_compressed_size(&compressed).ok() != Some(compressed.len()) {
        return Err(invalid(
            "snow record has trailing or invalid compressed bytes",
        ));
    }
    if zstd::zstd_safe::get_frame_content_size(&compressed)
        .ok()
        .flatten()
        .is_some_and(|n| n != header.raw as u64)
    {
        return Err(invalid("snow frame decoded size differs"));
    }
    let mut out = engine_allocation::vec(header.raw)?;
    out.resize(header.raw, 0);
    *buffer_peak = (*buffer_peak).max(compressed.capacity() + out.capacity());
    let mut decoder = zstd::bulk::Decompressor::new()?;
    let log = (usize::BITS - maximum.max(1).saturating_sub(1).leading_zeros()).max(10);
    decoder.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(log))?;
    if decoder
        .decompress_to_buffer(&compressed, &mut out[..])
        .map_err(|error| invalid(format!("snow compressed body is invalid: {error}")))?
        != header.raw
        || digest(&out) != header.digest
    {
        return Err(invalid("snow record checksum or decoded size differs"));
    }
    Ok(out)
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
    let mut out = Vec::new();
    out.extend_from_slice(&snow.width.to_le_bytes());
    out.extend_from_slice(&snow.height.to_le_bytes());
    put64(&mut out, snow.texels.len());
    out.extend_from_slice(&snow.texels);
    put64(&mut out, snow.province_snow.len());
    for value in &snow.province_snow {
        out.push(u8::from(value.is_some()));
        if let Some(value) = value {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.push(u8::from(snow.post_factors.is_some()));
    if let Some(values) = &snow.post_factors {
        put64(&mut out, values.len());
        for value in values.iter() {
            out.push(u8::from(value.is_some()));
            if let Some(value) = value {
                out.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    out.push(u8::from(snow.refresh.is_some()));
    if let Some(refresh) = snow.refresh {
        out.extend_from_slice(&refresh.interval.to_le_bytes());
        out.extend_from_slice(&refresh.identity.to_le_bytes());
    }
    let s = snow.summary;
    for count in [
        s.provinces,
        s.with_snow,
        s.shown_snow,
        s.shown_mud,
        s.missing_edges,
    ] {
        put64(&mut out, count);
    }
    out.push(u8::from(s.complete));
    if out.len() > limits.maximum()? {
        return Err(invalid("snow encoded product exceeds geometry"));
    }
    Ok(out)
}

fn count(d: &mut Decoder<'_>, expected: usize, minimum: usize) -> io::Result<usize> {
    let n = usize64(d.u64()?)?;
    if n != expected || n > d.remaining() / minimum {
        return Err(invalid(
            "snow typed length differs from admitted geometry or remaining bytes",
        ));
    }
    Ok(n)
}

pub(super) fn decode_product(bytes: &[u8], limits: Limits) -> io::Result<SnowMud> {
    if bytes.len() > limits.maximum()? {
        return Err(invalid("snow decoded product exceeds geometry"));
    }
    let mut d = Decoder::new(bytes);
    let (width, height) = (d.u32()?, d.u32()?);
    if [width, height] != limits.dimensions {
        return Err(invalid("snow decoded dimensions differ"));
    }
    let n = count(&mut d, limits.raster()?, 1)?;
    let texels = engine_allocation::copy_vec(d.take(n)?)?;
    let n = count(&mut d, limits.provinces, 1)?;
    let mut province_snow = engine_allocation::vec(n)?;
    for _ in 0..n {
        province_snow.push(if d.bool()? { Some(d.i64()?) } else { None });
    }
    let post_factors = if d.bool()? {
        let n = count(&mut d, limits.factors, 1)?;
        let mut values = engine_allocation::vec(n)?;
        for _ in 0..n {
            values.push(if d.bool()? {
                Some(f32::from_bits(d.u32()?))
            } else {
                None
            });
        }
        Some(Arc::from(values))
    } else {
        None
    };
    let refresh = if d.bool()? {
        Some(map_snow::Refresh {
            interval: d.i64()?,
            identity: d.u64()?,
        })
    } else {
        None
    };
    let summary = map_snow::Summary {
        provinces: usize64(d.u64()?)?,
        with_snow: usize64(d.u64()?)?,
        shown_snow: usize64(d.u64()?)?,
        shown_mud: usize64(d.u64()?)?,
        missing_edges: usize64(d.u64()?)?,
        complete: d.bool()?,
    };
    d.finish()?;
    Ok(SnowMud {
        width,
        height,
        texels,
        province_snow,
        post_factors,
        refresh,
        summary,
        build: Default::default(),
    })
}
