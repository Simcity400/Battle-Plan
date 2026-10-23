use super::*;
use replay_data::codec::Decoder;

pub(super) struct Entry {
    pub digest: [u8; 32],
    pub section: u32,
    pub ordinal: u64,
    pub raw_hint: u32,
}
impl Entry {
    pub fn bytes(&self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        replay_data::codec::put_fixed(&mut bytes, &self.digest)?;
        replay_data::codec::put_fixed(&mut bytes, &self.section.to_le_bytes())?;
        replay_data::codec::put_fixed(&mut bytes, &self.ordinal.to_le_bytes())?;
        replay_data::codec::put_fixed(&mut bytes, &self.raw_hint.to_le_bytes())?;
        Ok(bytes)
    }
    pub fn read(range: store::Range) -> io::Result<Self> {
        let bytes = codec::read_exact_range(range, 48)?;
        let mut d = Decoder::new(&bytes);
        let entry = Self {
            digest: d.take(32)?.try_into().unwrap(),
            section: d.u32()?,
            ordinal: d.u64()?,
            raw_hint: d.u32()?,
        };
        d.finish()?;
        payload_section(entry.section)?;
        Ok(entry)
    }
}

#[derive(Default)]
pub(super) struct Scan {
    pub bucket: Option<u8>,
    pub entries: u64,
    pub peak_index_bytes: usize,
}

pub(super) fn intern(
    writer: &mut Writer,
    key: &Key,
    section: u32,
    branch: BranchId,
    hour: Hour,
    bytes: &[u8],
) -> io::Result<Payload> {
    intern_digest(
        writer,
        key,
        section,
        branch,
        hour,
        bytes,
        codec::digest(bytes),
        &mut Scan::default(),
    )
}

pub(super) fn intern_digest(
    writer: &mut Writer,
    key: &Key,
    section: u32,
    branch: BranchId,
    hour: Hour,
    bytes: &[u8],
    digest: [u8; 32],
    scan: &mut Scan,
) -> io::Result<Payload> {
    let payloads = payload_section(section)?;
    let table = section & TABLE_SECTION != 0;
    let layer = (section & 0xffff) as u16;
    let envelope_key = if table {
        key.table_digest()
    } else {
        key.layer_digest(layer)?
    };
    let kind = if table { 4 } else { 3 };
    let raw_hint = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let bucket = key.section(&format!("index/{:02x}", digest[0]));
    scan.bucket = Some(digest[0]);
    let mut ordinal = 0;
    while let Some(range) = writer.read_at(&bucket, ordinal)? {
        scan.entries += 1;
        scan.peak_index_bytes = scan.peak_index_bytes.max(48);
        ordinal += 1;
        let candidate = (|| {
            let entry = Entry::read(range)?;
            if entry.digest != digest || entry.raw_hint != raw_hint {
                return Ok(None);
            }
            let Some(range) = writer.read_at(&payload_section(entry.section)?, entry.ordinal)?
            else {
                return Ok(None);
            };
            let header = codec::header(&range)?;
            if header.raw != bytes.len() {
                return Ok(None);
            }
            let owner = (entry.section & 0xffff) as u16;
            let stored_table = entry.section & TABLE_SECTION != 0;
            let body = codec::unpack_for(
                range,
                if stored_table {
                    key.table_digest()
                } else {
                    key.layer_digest(owner)?
                },
                None,
                if stored_table { 4 } else { 3 },
                owner,
                bytes.len(),
                &mut 0,
            )?;
            if body != bytes {
                return Ok(None);
            }
            Ok(Some(Payload {
                section: entry.section,
                ordinal: entry.ordinal,
                digest: header.digest,
                raw: header.raw as u64,
                branch: header.branch,
                hour: header.hour,
            }))
        })();
        match candidate {
            Ok(Some(payload)) => return Ok(payload),
            Ok(None) => {}
            Err(error) if cache_validation(&error) => {}
            Err(error) => return Err(error),
        }
    }
    let packet = codec::pack_for(envelope_key, branch, hour, kind, layer, bytes)?;
    let ordinal = writer.write(&payloads, packet.as_slice())?;
    let entry = Entry {
        digest,
        section,
        ordinal,
        raw_hint,
    }
    .bytes()?;
    writer.write(&bucket, entry.as_slice())?;
    Ok(Payload {
        section,
        ordinal,
        digest: codec::digest(bytes),
        raw: bytes.len() as u64,
        branch,
        hour,
    })
}
