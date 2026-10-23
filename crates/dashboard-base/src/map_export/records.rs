use super::*;
use std::collections::BTreeMap;

pub const SNOW: u16 = 1;
pub const FORMAT: u32 = 2;
pub const TABLE_SECTION: u32 = 1 << 16;
pub const CONTRACT: &[u8] = b"map-export/endpoints40/multilayer/u64-counts/v2";
pub const CURSOR_CONTRACT: &[u8] = b"map-export/cursor/key-branch-frontier/sha256/v1";
pub const SNOW_CONTRACT: &[u8] =
    b"snow-mud/cells4/retained-edges/raw-winter-f32-bits/no-perspective-dependence/v1";

product_codec::product! {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Variant { pub scope: String, pub fields: BTreeMap<String, Vec<u8>> }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Producer {
        pub contract: Vec<u8>, pub version: u32, pub schema: [u8; 32], pub variant: Vec<u8>,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Payload {
        pub section: u32, pub ordinal: u64, pub digest: [u8; 32], pub raw: u64,
        pub branch: BranchId, pub hour: Hour,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum LayerStatus {
        0 => Available { payload: Payload },
        1 => Unavailable { reason: String },
        2 => Unsupported { reason: String },
        3 => PartError { kind: String, display: String },
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Manifest {
        pub endpoint: u8, pub layers: BTreeMap<u16, LayerStatus>, pub tables: BTreeMap<u16, Payload>,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Cursor { pub hour: Hour, pub block: Option<u64> }
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct BoundCursor { key: [u8; 32], branch: BranchId, cursor: Cursor }
}
impl Cursor {
    pub fn encode(&self, key: &Key, branch: BranchId) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::from(CURSOR_CONTRACT);
        bytes.extend_from_slice(&product_codec::encode(&BoundCursor {
            key: key.digest(),
            branch,
            cursor: self.clone(),
        })?);
        bytes.extend_from_slice(&codec::digest(&bytes));
        Ok(bytes)
    }

    pub fn read(range: store::Range, key: &Key, branch: BranchId) -> io::Result<Self> {
        let length = usize::try_from(range.len())
            .map_err(|_| invalid("map export cursor exceeds address space"))?;
        let bytes = codec::read_exact_range(range, length)?;
        if bytes.len() < CURSOR_CONTRACT.len() + 32 {
            return Err(invalid("short map export cursor"));
        }
        let end = bytes.len() - 32;
        if &bytes[..CURSOR_CONTRACT.len()] != CURSOR_CONTRACT
            || codec::digest(&bytes[..end]) != bytes[end..]
        {
            return Err(invalid("map export cursor checksum or contract differs"));
        }
        let bound: BoundCursor = product_codec::decode(&bytes[CURSOR_CONTRACT.len()..end])?;
        if bound.key != key.digest() || bound.branch != branch {
            return Err(invalid("map export cursor provenance differs"));
        }
        Ok(bound.cursor)
    }
}

impl Variant {
    pub fn snow() -> Self {
        Self {
            scope: "snow/perspective-independent".into(),
            fields: BTreeMap::new(),
        }
    }
}
impl Manifest {
    pub fn validate(&self) -> io::Result<()> {
        if self.endpoint == 0 || self.endpoint & !15 != 0 {
            return Err(invalid("map export manifest has unknown endpoint kind"));
        }
        for (&layer, status) in &self.layers {
            if layer == 0 {
                return Err(invalid("map export layer zero is reserved"));
            }
            if let LayerStatus::Available { payload } = status {
                payload_section(payload.section)?;
            }
        }
        for payload in self.tables.values() {
            payload_section(payload.section)?;
        }
        Ok(())
    }
}

pub fn schema_digest() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(CONTRACT);
    digest.update(product_codec::schema_digest::<Manifest>());
    digest.update(CURSOR_CONTRACT);
    digest.update(product_codec::schema_digest::<BoundCursor>());
    digest.update(product_codec::schema_digest::<binding::Binding>());
    digest.update(witness::WITNESS_CONTRACT);
    digest.update(product_codec::schema_digest::<witness::Entry>());
    digest.update(b"envelope135/raw-bit7/zstd-window-raw/index48/saturating-u32-length-hint");
    digest.finalize().into()
}

pub(super) fn validate_manifest(key: &Key, manifest: &Manifest) -> io::Result<()> {
    manifest.validate()?;
    if manifest
        .layers
        .keys()
        .any(|layer| !key.producers.contains_key(layer))
    {
        return Err(invalid("map export manifest names an unbound layer"));
    }
    Ok(())
}

pub fn payload_section(id: u32) -> io::Result<String> {
    if id < TABLE_SECTION && id != 0 {
        Ok(format!("map-export-v{FORMAT}/layer/{id}"))
    } else if (TABLE_SECTION..TABLE_SECTION * 2).contains(&id) {
        Ok(format!("map-export-v{FORMAT}/tables"))
    } else {
        Err(invalid("unknown map export payload section"))
    }
}

#[derive(Debug)]
pub(super) enum SnowStatus<'a> {
    Available(Payload),
    Unavailable(&'a str),
    Unsupported(&'a str),
    PartError { kind: &'a str, display: &'a str },
}

pub(super) fn snow_status<'a>(bytes: &'a [u8], key: &Key) -> io::Result<Option<SnowStatus<'a>>> {
    use product_codec::{Decoder, Product};
    fn text<'a>(input: &mut Decoder<'a>) -> io::Result<&'a str> {
        let count = input.count64(1)?;
        std::str::from_utf8(input.take(count)?).map_err(|_| invalid("invalid UTF-8 string"))
    }
    let mut input = Decoder::new(bytes);
    let endpoint = input.u8()?;
    if endpoint == 0 || endpoint & !15 != 0 {
        return Err(invalid("map export manifest has unknown endpoint kind"));
    }
    let count = input.count64(size_of::<u16>() + LayerStatus::MIN_BYTES)?;
    let mut previous = None;
    let mut snow = None;
    for _ in 0..count {
        let layer = input.u16()?;
        if previous.is_some_and(|previous| previous >= layer) {
            return Err(invalid("product map keys are not strictly ordered"));
        }
        if layer == 0 || !key.producers.contains_key(&layer) {
            return Err(invalid("map export manifest names an unbound layer"));
        }
        let status = match input.u8()? {
            0 => {
                let payload = Payload::decode(&mut input)?;
                validate_section(payload.section)?;
                SnowStatus::Available(payload)
            }
            1 => SnowStatus::Unavailable(text(&mut input)?),
            2 => SnowStatus::Unsupported(text(&mut input)?),
            3 => SnowStatus::PartError {
                kind: text(&mut input)?,
                display: text(&mut input)?,
            },
            _ => return Err(invalid("unknown product enum tag")),
        };
        if layer == SNOW {
            snow = Some(status);
        }
        previous = Some(layer);
    }
    let count = input.count64(size_of::<u16>() + Payload::MIN_BYTES)?;
    let mut previous = None;
    for _ in 0..count {
        let table = input.u16()?;
        if previous.is_some_and(|previous| previous >= table) {
            return Err(invalid("product map keys are not strictly ordered"));
        }
        validate_section(Payload::decode(&mut input)?.section)?;
        previous = Some(table);
    }
    input.finish()?;
    Ok(snow)
}

fn validate_section(id: u32) -> io::Result<()> {
    if id != 0 && id < TABLE_SECTION * 2 {
        Ok(())
    } else {
        Err(invalid("unknown map export payload section"))
    }
}
